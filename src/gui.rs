//! GPUI front end for mouse-config, laid out like the vendor's "Mouse Drive Beta" window.
//!
//! The scene is the vendor's 1020x700 canvas; every coordinate below is taken from the original
//! form definitions and scaled to the window. The artwork is loaded at run time from
//! ~/.local/share/mouse-config/assets (see scripts/install-assets.sh); without it the app still works.

use gpui::{
    div, img, linear_color_stop, linear_gradient, prelude::*, px, rgb, size, AnyElement, App, Application, Bounds,
    Context, Div, FocusHandle, KeyDownEvent, KeyUpEvent, Modifiers, ModifiersChangedEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, SharedString, Task, TitlebarOptions, Window, WindowBounds,
    WindowOptions,
};
use mouse_config::config::{self, Config, Patch};
use mouse_config::device::{Battery, Link, Mouse};
use mouse_config::macros::{self, Action, Macro, State};
use mouse_config::ops::{self, Opts};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const SCENE_W: f32 = 1020.0;
const SCENE_H: f32 = 700.0;
/// Origin of the tab pages on the scene: tab control (136, 98) plus the page offset (4, 26).
const PAGE: (f32, f32) = (140.0, 124.0);

/// Vertical gradient of the vendor background (y on the 700 px scene, colour), sampled from bg.png.
const BG_STOPS: [(f32, u32); 8] = [
    (0.0, 0x1c1c1c),
    (50.0, 0x1b1b1b),
    (100.0, 0x171717),
    (200.0, 0x141414),
    (350.0, 0x101010),
    (500.0, 0x0b0b0b),
    (650.0, 0x080808),
    (699.0, 0x070707),
];

const RED: u32 = 0xff1a1a;
const TEXT: u32 = 0xe6e6e6;
const MUTED: u32 = 0x8a8a8a;
const LINE: u32 = 0x555555;
const OK: u32 = 0x39d353;
const WARN: u32 = 0xffb02e;

/// Stage light colours offered by the vendor software.
const PALETTE: [[u8; 3]; 8] = [
    [255, 0, 0],
    [0, 0, 255],
    [0, 255, 0],
    [255, 255, 0],
    [0, 255, 255],
    [255, 0, 255],
    [255, 255, 255],
    [255, 128, 0],
];

/// Positions of the 16 key buttons on the mouse photo (relative to the 432x356 picture).
const KEY_SPOTS: [(f32, f32); 16] = [
    (94.0, 212.0),
    (53.0, 149.0),
    (144.0, 95.0),
    (203.0, 193.0),
    (263.0, 159.0),
    (229.0, 86.0),
    (384.0, 9.0),
    (384.0, 51.0),
    (384.0, 95.0),
    (384.0, 139.0),
    (384.0, 183.0),
    (384.0, 227.0),
    (383.0, 271.0),
    (339.0, 271.0),
    (295.0, 271.0),
    (251.0, 271.0),
];

/// Functions this build can assign to a key: (class, value, label).
const KEY_CHOICES: [(u8, u8, &str); 7] = [
    (1, 1, "Левая кнопка"),
    (1, 2, "Правая кнопка"),
    (1, 4, "Средняя кнопка"),
    (1, 8, "Назад"),
    (1, 16, "Вперёд"),
    (2, 1, "DPI"),
    (0, 0, "Отключена"),
];

/// The mouse has six physical buttons: keys 1..6 (left, right, middle, back, forward, DPI).
const PHYSICAL_KEYS: usize = 6;

const TABS: [&str; 5] = ["Кнопки", "Сенсор", "Макросы", "Подсветка", "Настройки"];

/// One conversation with the device at a time: the poll and the writes share the dongle's reply stream.
static DEVICE: Mutex<()> = Mutex::new(());

fn assets_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    base.join("mouse-config/assets")
}

/// Pixel size of a PNG, read from its header and cached.
fn png_size(path: &Path) -> Option<(f32, f32)> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (f32, f32)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(s) = cache.lock().ok()?.get(path) {
        return Some(*s);
    }
    use std::io::Read;
    let mut head = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[1..4] != b"PNG" {
        return None;
    }
    let size = (
        u32::from_be_bytes(head[16..20].try_into().ok()?) as f32,
        u32::from_be_bytes(head[20..24].try_into().ok()?) as f32,
    );
    cache.lock().ok()?.insert(path.to_path_buf(), size);
    Some(size)
}

struct Snapshot {
    img: Vec<u8>,
    link: Link,
    node: String,
    online: bool,
    battery: Option<Battery>,
}

fn load() -> Result<Snapshot, String> {
    let _guard = DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let mut mouse = Mouse::open().map_err(|e| e.to_string())?;
    let online = mouse.online().unwrap_or(false);
    let battery = if online { mouse.battery().ok() } else { None };
    let img = ops::read_image(&mut mouse, 0)?;
    Ok(Snapshot { img, link: mouse.link, node: mouse.node.display().to_string(), online, battery })
}

fn read_slot(slot: usize) -> Result<Option<Macro>, String> {
    let _guard = DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let mut mouse = Mouse::open().map_err(|e| e.to_string())?;
    let bytes = mouse.read(macros::slot_addr(slot) as u16, macros::SLOT_LEN).map_err(|e| e.to_string())?;
    Ok(Macro::decode(&bytes))
}

fn write(patch: Patch, backup: bool) -> Result<(), String> {
    let _guard = DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let mut mouse = Mouse::open().map_err(|e| e.to_string())?;
    ops::apply(&mut mouse, &Opts { base: 0, force: false, backup }, vec![patch])?;
    Ok(())
}

/// Scene-to-window scaling: the scene is centred in the window.
#[derive(Clone, Copy)]
struct Canvas {
    k: f32,
    ox: f32,
    oy: f32,
}

impl Canvas {
    fn px(&self, v: f32) -> Pixels {
        px(v * self.k)
    }

    /// A box at scene coordinates.
    fn at(&self, x: f32, y: f32, w: f32, h: f32) -> Div {
        div().absolute().left(self.px(x)).top(self.px(y)).w(self.px(w)).h(self.px(h))
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&self, s: impl Into<String>, x: f32, y: f32, w: f32, h: f32, size_px: f32, color: u32) -> Div {
        self.at(x, y, w, h)
            .flex()
            .items_center()
            .text_size(self.px(size_px))
            .text_color(rgb(color))
            .child(SharedString::from(s.into()))
    }

    /// A vendor image, or nothing when the artwork is not installed.
    fn pic(&self, name: &str, x: f32, y: f32, w: f32, h: f32) -> AnyElement {
        let path = assets_dir().join(name);
        if !path.exists() {
            return div().into_any_element();
        }
        self.at(x, y, w, h).child(img(path).w(self.px(w)).h(self.px(h)).object_fit(ObjectFit::Fill)).into_any_element()
    }

    /// A vendor image at its own size, centred in the box. The vendor's state images (normal, selected,
    /// hover) differ in size on purpose, so they must not be stretched to one rectangle.
    fn pic_nat(&self, name: &str, x: f32, y: f32, w: f32, h: f32) -> AnyElement {
        let path = assets_dir().join(name);
        let Some((nw, nh)) = png_size(&path) else { return div().into_any_element() };
        self.pic(name, x + (w - nw) / 2.0, y + (h - nh) / 2.0, nw, nh)
    }

    fn scene_x(&self, window_x: Pixels) -> f32 {
        (f32::from(window_x) - self.ox) / self.k
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Dpi,
    Bright,
    Speed,
    Hue,
}

#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Mode,
    Idle,
}

/// The vendor's 14 default colours for the body light.
const PRESETS: [[u8; 3]; 14] = [
    [255, 0, 0],
    [0, 255, 255],
    [0, 255, 0],
    [255, 0, 255],
    [255, 255, 0],
    [255, 80, 0],
    [0, 0, 255],
    [227, 0, 127],
    [46, 166, 223],
    [96, 25, 133],
    [255, 255, 255],
    [139, 178, 36],
    [255, 155, 51],
    [208, 113, 173],
];
/// Where the presets sit relative to the colour panel (411, 44) on the light page.
const PRESET_SPOTS: [(f32, f32); 14] = [
    (8.0, 315.0),
    (54.0, 315.0),
    (101.0, 315.0),
    (149.0, 315.0),
    (195.0, 315.0),
    (242.0, 315.0),
    (292.0, 315.0),
    (8.0, 371.0),
    (54.0, 371.0),
    (101.0, 371.0),
    (149.0, 371.0),
    (195.0, 371.0),
    (242.0, 371.0),
    (292.0, 371.0),
];

fn hue_to_rgb(frac: f32) -> [u8; 3] {
    let h = (frac.clamp(0.0, 1.0) * 285.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        _ => (x, 0.0, 1.0),
    };
    [(r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8]
}

struct Gui {
    snap: Option<Snapshot>,
    note: Option<(String, bool)>,
    queue: VecDeque<Patch>,
    writing: bool,
    backed_up: bool,
    load_failures: u8,
    tab: usize,
    home: bool,
    key: usize,
    stage: usize,
    drag: Option<Drag>,
    menu: Option<Menu>,
    /// Macros read from the mouse, by slot (= key index); `None` when the slot holds no valid macro.
    stored: HashMap<usize, Option<Macro>>,
    macro_key: usize,
    /// Show the key slots 7..16, which have no physical button on this mouse.
    show_extra: bool,
    draft: Option<Vec<Action>>,
    sel_action: Option<usize>,
    recording: bool,
    real_pauses: bool,
    repeat: u8,
    mods: Modifiers,
    last_event: Option<Instant>,
    focus: FocusHandle,
    canvas: Canvas,
    _poll: Task<()>,
}

impl Gui {
    fn new(cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| loop {
            let result = cx.background_executor().spawn(async { load() }).await;
            if this.update(cx, |this, cx| this.loaded(result, cx)).is_err() {
                break;
            }
            cx.background_executor().timer(Duration::from_secs(5)).await;
        });
        Gui {
            snap: None,
            note: None,
            queue: VecDeque::new(),
            writing: false,
            backed_up: false,
            load_failures: 0,
            tab: 1,
            home: false,
            key: 0,
            stage: 0,
            drag: None,
            menu: None,
            stored: HashMap::new(),
            macro_key: 0,
            show_extra: false,
            draft: None,
            sel_action: None,
            recording: false,
            real_pauses: false,
            repeat: 1,
            mods: Modifiers::default(),
            last_event: None,
            focus: cx.focus_handle(),
            canvas: Canvas { k: 1.0, ox: 0.0, oy: 0.0 },
            _poll: poll,
        }
    }

    fn loaded(&mut self, result: Result<Snapshot, String>, cx: &mut Context<Self>) {
        if self.writing || !self.queue.is_empty() || self.drag.is_some() {
            return; // keep the optimistic state until our writes have landed
        }
        match result {
            Ok(snap) => {
                if matches!(&self.note, Some((_, true))) {
                    self.note = None;
                }
                self.load_failures = 0;
                self.snap = Some(snap);
            }
            Err(e) => {
                // a single failed read must not blank the window: keep the last settings for a while
                self.load_failures = self.load_failures.saturating_add(1);
                if self.load_failures >= 3 {
                    self.snap = None;
                }
                self.note = Some((e, true));
            }
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async { load() }).await;
            this.update(cx, |this, cx| this.loaded(result, cx)).ok();
        })
        .detach();
    }

    /// Show `patch` immediately in the local copy of the settings.
    fn preview(&mut self, patch: &Patch) {
        if let Some(snap) = self.snap.as_mut() {
            // macro slots live outside the settings image
            if let Some(dst) = snap.img.get_mut(patch.addr..patch.addr + patch.bytes.len()) {
                dst.copy_from_slice(&patch.bytes);
            }
        }
    }

    /// Show `patch` immediately, then write it to the mouse; writes go out strictly in click order.
    fn mutate(&mut self, patch: Result<Patch, String>, cx: &mut Context<Self>) {
        let patch = match patch {
            Ok(p) => p,
            Err(e) => {
                self.note = Some((e, true));
                cx.notify();
                return;
            }
        };
        if self.snap.is_none() {
            return;
        }
        self.preview(&patch);
        self.queue.push_back(patch);
        cx.notify();
        if self.writing {
            return;
        }
        self.writing = true;
        cx.spawn(async move |this, cx| loop {
            let Ok((next, backup)) = this.update(cx, |this, _| {
                let backup = !this.backed_up;
                (this.queue.pop_front(), backup)
            }) else {
                break;
            };
            let Some(patch) = next else {
                this.update(cx, |this, cx| {
                    this.writing = false;
                    this.reload(cx);
                })
                .ok();
                break;
            };
            let result = cx.background_executor().spawn(async move { write(patch, backup) }).await;
            this.update(cx, |this, cx| {
                this.backed_up = true;
                this.note = Some(match result {
                    Ok(()) => ("Записано в мышь и проверено чтением".to_string(), false),
                    Err(e) => (e, true),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Read the macros the keys point at (a slot is 0x180 bytes, so only on demand).
    fn load_slots(&mut self, cx: &mut Context<Self>) {
        let Some(snap) = &self.snap else { return };
        let cfg = Config::parse(&snap.img);
        for k in cfg.keys.iter().filter(|k| k[0] == 6) {
            let slot = k[1] as usize;
            if slot >= config::KEY_COUNT || self.stored.contains_key(&slot) {
                continue;
            }
            cx.spawn(async move |this, cx| {
                let result = cx.background_executor().spawn(async move { read_slot(slot) }).await;
                this.update(cx, |this, cx| {
                    match result {
                        Ok(m) => {
                            this.stored.insert(slot, m);
                        }
                        Err(e) => this.note = Some((e, true)),
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Actions shown for the selected key: the draft being edited, else what the mouse holds.
    fn shown_actions(&self) -> Vec<Action> {
        if let Some(d) = &self.draft {
            return d.clone();
        }
        self.snap
            .as_ref()
            .map(|s| Config::parse(&s.img).keys[self.macro_key])
            .filter(|k| k[0] == 6)
            .and_then(|k| self.stored.get(&(k[1] as usize)).cloned().flatten())
            .map(|m| m.actions)
            .unwrap_or_default()
    }

    fn record(&mut self, state: State, kind: u8, value: u8, cx: &mut Context<Self>) {
        let now = Instant::now();
        let real = self.real_pauses;
        let draft = self.draft.get_or_insert_with(Vec::new);
        if draft.len() >= macros::MAX_ACTIONS {
            self.note = Some((format!("В макросе не больше {} действий", macros::MAX_ACTIONS), true));
        } else {
            if real {
                if let (Some(last), Some(prev)) = (self.last_event, draft.last_mut()) {
                    prev.delay = now.duration_since(last).as_millis().clamp(1, 65535) as u16;
                }
            }
            draft.push(macros::make_action(state, kind, value, macros::DEFAULT_DELAY_MS));
            self.last_event = Some(now);
        }
        cx.notify();
    }

    fn on_key(&mut self, key: &str, down: bool, cx: &mut Context<Self>) {
        if !self.recording {
            return;
        }
        match macros::key_value(key) {
            Ok((kind, value)) => self.record(if down { State::Down } else { State::Up }, kind, value, cx),
            Err(_) if down => {
                self.note = Some((format!("Клавиша «{key}» не поддерживается в макросах"), true));
                cx.notify();
            }
            Err(_) => {}
        }
    }

    fn on_modifiers(&mut self, now: Modifiers, cx: &mut Context<Self>) {
        let before = std::mem::replace(&mut self.mods, now);
        if !self.recording {
            return;
        }
        for (was, is, name) in [
            (before.control, now.control, "ctrl"),
            (before.shift, now.shift, "shift"),
            (before.alt, now.alt, "alt"),
            (before.platform, now.platform, "win"),
        ] {
            if was != is {
                if let Ok((kind, value)) = macros::key_value(name) {
                    self.record(if is { State::Down } else { State::Up }, kind, value, cx);
                }
            }
        }
    }

    fn save_macro(&mut self, cx: &mut Context<Self>) {
        let key = self.macro_key;
        let Some(actions) = self.draft.clone() else {
            self.note = Some(("Сначала запишите или откройте макрос".into(), true));
            cx.notify();
            return;
        };
        let m = Macro { name: format!("Macro {}", key + 1), actions };
        match m.patches(key, self.repeat) {
            Ok(patches) => {
                self.recording = false;
                self.stored.insert(key, Some(m));
                self.draft = None;
                for p in patches {
                    self.mutate(Ok(p), cx);
                }
            }
            Err(e) => {
                self.note = Some((e, true));
                cx.notify();
            }
        }
    }

    /// Track of a slider in scene coordinates: (x, width).
    fn track(d: Drag) -> (f32, f32) {
        match d {
            Drag::Dpi => (PAGE.0 + 84.0, 620.0),
            Drag::Bright | Drag::Speed => (PAGE.0 + 43.0, 267.0),
            Drag::Hue => (PAGE.0 + 419.0, 308.0),
        }
    }

    fn frac_at(&self, d: Drag, window_x: Pixels) -> f32 {
        let (x, w) = Self::track(d);
        ((self.canvas.scene_x(window_x) - x) / w).clamp(0.0, 1.0)
    }

    fn bar(&self) -> Option<config::Bar> {
        self.snap.as_ref().map(|s| Config::parse(&s.img).bar)
    }

    /// The patch a drag at `frac` of its track stands for.
    fn drag_patch(&self, d: Drag, frac: f32) -> Result<Patch, String> {
        let bar = self.bar().ok_or("нет связи с мышью")?;
        match d {
            Drag::Dpi => {
                let v = (100.0 + frac * (config::MAX_DPI as f32 - 100.0)) / 50.0;
                config::set_dpi(self.stage + 1, (v.round() as u32 * 50).clamp(100, config::MAX_DPI))
            }
            Drag::Bright => {
                config::bar_patch(&config::Bar { brightness: (frac * config::BAR_LEVELS as f32).round() as u8, ..bar })
            }
            Drag::Speed => {
                config::bar_patch(&config::Bar { speed: (frac * config::BAR_LEVELS as f32).round() as u8, ..bar })
            }
            Drag::Hue => config::bar_patch(&config::Bar { color: hue_to_rgb(frac), ..bar }),
        }
    }

    fn drag_to(&mut self, d: Drag, window_x: Pixels, cx: &mut Context<Self>) {
        if let Ok(p) = self.drag_patch(d, self.frac_at(d, window_x)) {
            self.preview(&p);
            cx.notify();
        }
    }

    /// Send what the drag left in the local copy to the mouse.
    fn commit_drag(&mut self, d: Drag, cx: &mut Context<Self>) {
        let Some(snap) = &self.snap else { return };
        let cfg = Config::parse(&snap.img);
        let patch = match d {
            Drag::Dpi => {
                let stage = self.stage.min(config::STAGE_COUNT - 1);
                config::set_dpi(stage + 1, config::code_to_dpi(cfg.dpi[stage].0 as u8))
            }
            Drag::Bright | Drag::Speed | Drag::Hue => config::bar_patch(&cfg.bar),
        };
        self.mutate(patch, cx);
    }
}

fn key_name(k: [u8; 3]) -> String {
    match (k[0], k[1]) {
        (0, _) => "Отключена".into(),
        (1, 1) => "Левая кнопка".into(),
        (1, 2) => "Правая кнопка".into(),
        (1, 4) => "Средняя кнопка".into(),
        (1, 8) => "Назад".into(),
        (1, 16) => "Вперёд".into(),
        (1, v) => format!("Кнопка мыши {v:#04x}"),
        (2, v) => format!("DPI ({v})"),
        (c, v) => format!("{} {v:#04x}", config::key_class(c)),
    }
}

fn rgb_of(c: [u8; 3]) -> u32 {
    ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
}

/// The vendor's flat button: a bordered box, red when active.
#[allow(clippy::too_many_arguments)]
fn flat(
    c: &Canvas,
    id: impl Into<gpui::ElementId>,
    label: String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    active: bool,
) -> gpui::Stateful<Div> {
    c.at(x, y, w, h)
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .rounded(c.px(5.0))
        .border_1()
        .border_color(rgb(if active { RED } else { LINE }))
        .bg(rgb(if active { RED } else { 0x1d1d1d }))
        .text_size(c.px(13.0))
        .text_color(rgb(if active { 0xffffff } else { TEXT }))
        .cursor_pointer()
        .hover(|s| s.border_color(rgb(RED)))
        .child(label)
}

impl Gui {
    fn chrome(&self, c: &Canvas, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = vec![c.pic("bg.png", 0.0, 0.0, SCENE_W, SCENE_H)];
        out.push(c.text("Phantom Pro V2", 221.0, 21.0, 300.0, 28.0, 18.0, TEXT).into_any_element());

        // navigation strip: the default sprite with the selected segment overlaid
        out.push(c.pic("nav.png", 218.0, 64.0, 382.0, 26.0));
        let seg = 382.0 / 5.0;
        let nav_on = assets_dir().join("nav_on.png");
        for (i, tab_label) in TABS.iter().enumerate() {
            let selected = !self.home && self.tab == i;
            if selected && nav_on.exists() {
                out.push(
                    c.at(218.0 + seg * i as f32, 64.0, seg, 26.0)
                        .overflow_hidden()
                        .child(
                            img(nav_on.clone())
                                .absolute()
                                .left(c.px(-seg * i as f32))
                                .w(c.px(382.0))
                                .h(c.px(26.0))
                                .object_fit(ObjectFit::Fill),
                        )
                        .into_any_element(),
                );
            }
            out.push(
                c.at(218.0 + seg * i as f32, 64.0, seg, 26.0)
                    .id(("tab", i))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = i;
                        this.home = false;
                        if i == 2 {
                            this.load_slots(cx);
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            if selected {
                out.push(
                    c.text(*tab_label, 218.0 + seg * i as f32, 92.0, seg, 18.0, 12.0, RED)
                        .justify_center()
                        .into_any_element(),
                );
            }
        }

        // home and "system mouse" buttons on the left edge
        out.push(
            flat(c, "home", "⌂".into(), 38.0, 130.0, 34.0, 30.0, self.home)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.home = true;
                    cx.notify();
                }))
                .into_any_element(),
        );

        // battery
        if let Some(snap) = &self.snap {
            if let Some(b) = &snap.battery {
                out.push(
                    c.text(format!("{}%", b.percent), 892.0, 62.0, 56.0, 24.0, 14.0, TEXT)
                        .justify_end()
                        .into_any_element(),
                );
                out.push(c.pic("battery_empty.png", 950.0, 64.0, 42.0, 20.0));
                let color = if b.on_cable {
                    WARN
                } else if b.percent <= 20 {
                    RED
                } else {
                    OK
                };
                let fill = 34.0 * (b.percent.min(100) as f32 / 100.0);
                out.push(c.at(953.0, 67.0, fill.max(1.0), 14.0).bg(rgb(color)).into_any_element());
                if b.on_cable {
                    out.push(c.text("⚡", 950.0, 64.0, 38.0, 20.0, 13.0, 0x000000).justify_center().into_any_element());
                }
            } else {
                out.push(c.text("мышь спит", 880.0, 62.0, 112.0, 24.0, 13.0, MUTED).justify_end().into_any_element());
            }
            let (link, color) = match snap.link {
                Link::Wired => ("USB", OK),
                Link::Dongle => ("2.4G", WARN),
            };
            out.push(c.text(link, 780.0, 12.0, 60.0, 24.0, 13.0, color).justify_end().into_any_element());
        }
        out
    }

    /// Switch that shows or hides the service key slots 7..16.
    fn extra_toggle(&self, c: &Canvas, x: f32, y: f32, w: f32, cx: &mut Context<Self>) -> AnyElement {
        let label = if self.show_extra {
            "Скрыть служебные ячейки 7–16"
        } else {
            "Показать служебные ячейки 7–16"
        };
        flat(c, "extra-toggle", label.to_string(), x, y, w, 26.0, self.show_extra)
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_extra = !this.show_extra;
                if !this.show_extra {
                    if this.key >= PHYSICAL_KEYS {
                        this.key = 0;
                    }
                    if this.macro_key >= PHYSICAL_KEYS {
                        this.macro_key = 0;
                        this.draft = None;
                        this.sel_action = None;
                        this.recording = false;
                    }
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn page_buttons(&self, c: &Canvas, cfg: &Config, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (px0, py0) = PAGE;
        let mut out: Vec<AnyElement> = Vec::new();
        out.push(c.text("Профиль 1", px0 + 44.0, py0 + 24.0, 124.0, 21.0, 13.0, TEXT).into_any_element());
        // the mouse photo and its 16 key buttons
        let (mx, my) = (px0 + 182.0, py0 + 6.0);
        out.push(c.pic("mouse.png", mx, my, 432.0, 356.0));
        let visible = if self.show_extra { config::KEY_COUNT } else { PHYSICAL_KEYS };
        for (i, (x, y)) in KEY_SPOTS.iter().enumerate().take(visible) {
            let selected = i == self.key;
            let extra = i >= PHYSICAL_KEYS;
            let name = if selected { "key_down.png" } else { "key.png" };
            // the slots without a physical button are drawn dimmed
            out.push(
                c.at(mx + x, my + y, 38.0, 38.0)
                    .opacity(if extra && !selected { 0.45 } else { 1.0 })
                    .child(c.pic_nat(name, 0.0, 0.0, 38.0, 38.0))
                    .into_any_element(),
            );
            out.push(
                c.at(mx + x, my + y, 38.0, 38.0)
                    .id(("key", i))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(c.px(12.0))
                    .text_color(rgb(0xffffff))
                    .cursor_pointer()
                    .child((i + 1).to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.key = i;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        out.push(self.extra_toggle(c, px0 + 560.0, py0 + 326.0, 268.0, cx));
        if self.show_extra {
            out.push(
                c.text(
                    "У мыши 6 кнопок. Ячейки 7–16 служебные, 9–12 лучше не менять.",
                    px0 + 40.0,
                    py0 + 326.0,
                    510.0,
                    28.0,
                    11.0,
                    MUTED,
                )
                .items_start()
                .into_any_element(),
            );
        }
        // function panel
        let (fx, fy) = (px0 + 2.0, py0 + 360.0);
        out.push(c.pic("panel.png", fx, fy, 862.0, 216.0));
        let current = cfg.keys[self.key];
        out.push(
            c.text(format!("Кнопка {}", self.key + 1), fx + 24.0, fy + 14.0, 300.0, 24.0, 15.0, TEXT)
                .into_any_element(),
        );
        out.push(
            c.text(format!("Сейчас: {}", key_name(current)), fx + 24.0, fy + 40.0, 500.0, 22.0, 13.0, MUTED)
                .into_any_element(),
        );
        for (n, (class, value, label)) in KEY_CHOICES.iter().enumerate() {
            let active = current[0] == *class && current[1] == *value;
            let (class, value, key) = (*class, *value, self.key);
            let (col, row) = (n % 4, n / 4);
            out.push(
                flat(
                    c,
                    ("choice", n),
                    label.to_string(),
                    fx + 24.0 + col as f32 * 205.0,
                    fy + 78.0 + row as f32 * 44.0,
                    190.0,
                    34.0,
                    active,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.mutate(config::set_key(key, class, value), cx)))
                .into_any_element(),
            );
        }
        out.push(
            c.text(
                "Макросы, горячие клавиши и автоповтор появятся позже",
                fx + 24.0,
                fy + 172.0,
                600.0,
                22.0,
                12.0,
                MUTED,
            )
            .into_any_element(),
        );
        out
    }

    fn dpi_row(&self, c: &Canvas, cfg: &Config, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (px0, py0) = PAGE;
        let mut out: Vec<AnyElement> = Vec::new();
        let stages = cfg.stages as usize;

        // stage buttons
        for i in 0..config::STAGE_COUNT {
            let (x, y) = (px0 + 80.0 + 100.0 * i as f32, py0 + 108.0);
            let active = i < stages;
            let selected = i == self.stage;
            let color = cfg.colors[i];
            let dpi = config::code_to_dpi(cfg.dpi[i].0 as u8);
            out.push(c.pic_nat(if selected { "dpi_on.png" } else { "dpi_off.png" }, x, y, 68.0, 70.0));
            out.push(
                c.at(x, y, 68.0, 70.0)
                    .id(("stage", i))
                    .opacity(if active { 1.0 } else { 0.35 })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.stage = i;
                        cx.notify();
                    }))
                    // the frame is 54x56 when not selected and 68x70 when selected; keep the number inside it
                    .child(c.text(
                        (i + 1).to_string(),
                        if selected { 6.0 } else { 13.0 },
                        if selected { 4.0 } else { 11.0 },
                        20.0,
                        16.0,
                        11.0,
                        MUTED,
                    ))
                    .child(c.text(dpi.to_string(), 0.0, 20.0, 68.0, 26.0, 15.0, TEXT).justify_center())
                    .into_any_element(),
            );
            let next =
                PALETTE.iter().position(|p| *p == color).map_or(PALETTE[0], |p| PALETTE[(p + 1) % PALETTE.len()]);
            out.push(
                c.at(x + 14.0, y + 50.0, 40.0, 8.0)
                    .id(("stage-color", i))
                    .rounded(c.px(4.0))
                    .bg(rgb(rgb_of(color)))
                    .opacity(if active { 1.0 } else { 0.35 })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.mutate(config::set_color(i + 1, next), cx)))
                    .into_any_element(),
            );
            // marker of the current stage
            let is_current = i == cfg.current as usize;
            out.push(
                c.at(x + 22.0, py0 + 184.0, 24.0, 20.0)
                    .id(("current", i))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.mutate(config::set_current(i + 1), cx)))
                    .child(c.pic_nat(
                        if is_current { "dpi_cur_hover.png" } else { "dpi_cur.png" },
                        0.0,
                        0.0,
                        24.0,
                        20.0,
                    ))
                    .opacity(if is_current { 1.0 } else { 0.25 })
                    .into_any_element(),
            );
        }

        // stage count
        out.push(c.text("Количество ступеней", px0 + 80.0, py0 + 20.0, 170.0, 22.0, 13.0, TEXT).into_any_element());
        out.push(
            flat(c, "stages-", "−".into(), px0 + 250.0, py0 + 20.0, 24.0, 22.0, false)
                .on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.mutate(config::set_stage_count(stages.saturating_sub(1)), cx)
                    }),
                )
                .into_any_element(),
        );
        out.push(
            c.text(stages.to_string(), px0 + 276.0, py0 + 20.0, 28.0, 22.0, 14.0, TEXT)
                .justify_center()
                .into_any_element(),
        );
        out.push(
            flat(c, "stages+", "+".into(), px0 + 306.0, py0 + 20.0, 24.0, 22.0, false)
                .on_click(cx.listener(move |this, _, _, cx| this.mutate(config::set_stage_count(stages + 1), cx)))
                .into_any_element(),
        );

        // DPI slider for the selected stage
        let sel = self.stage.min(config::STAGE_COUNT - 1);
        let dpi = config::code_to_dpi(cfg.dpi[sel].0 as u8);
        let frac = ((dpi as f32 - 100.0) / (config::MAX_DPI as f32 - 100.0)).clamp(0.0, 1.0);
        let (tx, ty, tw) = (px0 + 84.0, py0 + 62.0, 620.0);
        out.push(
            c.text(format!("Ступень {}", sel + 1), px0 + 84.0, py0 + 46.0, 200.0, 14.0, 11.0, MUTED).into_any_element(),
        );
        out.push(c.at(tx, ty + 9.0, tw, 5.0).rounded(c.px(3.0)).bg(rgb(LINE)).into_any_element());
        out.push(c.at(tx, ty + 9.0, tw * frac, 5.0).rounded(c.px(3.0)).bg(rgb(RED)).into_any_element());
        out.push(c.at(tx + tw * frac - 8.0, ty + 3.0, 16.0, 16.0).rounded_full().bg(rgb(0xffffff)).into_any_element());
        out.push(
            c.at(tx - 8.0, ty - 6.0, tw + 16.0, 34.0)
                .id("dpi-slider")
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.drag = Some(Drag::Dpi);
                        this.drag_to(Drag::Dpi, ev.position.x, cx);
                    }),
                )
                .into_any_element(),
        );
        out.push(
            flat(c, "dpi-", "−".into(), px0 + 712.0, py0 + 62.0, 22.0, 22.0, false)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mutate(config::set_dpi(sel + 1, dpi.saturating_sub(50).max(50)), cx)
                }))
                .into_any_element(),
        );
        out.push(
            c.text(dpi.to_string(), px0 + 738.0, py0 + 62.0, 62.0, 22.0, 14.0, TEXT)
                .justify_center()
                .into_any_element(),
        );
        out.push(
            flat(c, "dpi+", "+".into(), px0 + 802.0, py0 + 62.0, 22.0, 22.0, false)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mutate(config::set_dpi(sel + 1, (dpi + 50).min(config::MAX_DPI)), cx)
                }))
                .into_any_element(),
        );
        out
    }

    fn page_sensor(&self, c: &Canvas, cfg: &Config, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (px0, py0) = PAGE;
        let mut out = self.dpi_row(c, cfg, cx);

        // report rate
        out.push(c.text("Частота опроса", px0 + 68.0, py0 + 264.0, 200.0, 24.0, 14.0, TEXT).into_any_element());
        for (n, hz) in [125u32, 250, 500, 1000].into_iter().enumerate() {
            let active = config::rate_hz(cfg.rate) == Some(hz);
            out.push(
                flat(
                    c,
                    ("rate", n),
                    format!("{hz} Гц"),
                    px0 + 72.0 + 120.0 * n as f32,
                    py0 + 293.0,
                    74.0,
                    26.0,
                    active,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.mutate(config::set_rate(hz), cx)))
                .into_any_element(),
            );
        }

        // DPI light: shown, not editable yet
        out.push(c.text("Подсветка ступеней DPI", px0 + 68.0, py0 + 369.0, 260.0, 24.0, 14.0, TEXT).into_any_element());
        let effect = match cfg.led[0] {
            0 => "выкл",
            1 => "постоянная",
            2 => "мерцание",
            _ => "?",
        };
        out.push(
            c.text(
                format!("Режим: {effect}   Скорость: {}   Яркость: {}", cfg.led[2], cfg.led[1]),
                px0 + 74.0,
                py0 + 400.0,
                460.0,
                24.0,
                13.0,
                MUTED,
            )
            .into_any_element(),
        );

        // sensor switches
        let x0 = px0 + 540.0;
        out.push(c.text("Сенсор", x0, py0 + 264.0, 200.0, 24.0, 14.0, TEXT).into_any_element());
        let lod = cfg.lod;
        out.push(c.text("Высота отрыва (LOD)", x0, py0 + 296.0, 190.0, 26.0, 13.0, TEXT).into_any_element());
        for (n, v) in [1u8, 2].into_iter().enumerate() {
            out.push(
                flat(c, ("lod", n), format!("{v} мм"), x0 + 200.0 + n as f32 * 66.0, py0 + 296.0, 58.0, 26.0, lod == v)
                    .on_click(cx.listener(move |this, _, _, cx| this.mutate(config::set_lod(v), cx)))
                    .into_any_element(),
            );
        }
        let debounce = cfg.debounce;
        out.push(c.text("Задержка кнопок, мс", x0, py0 + 334.0, 190.0, 26.0, 13.0, TEXT).into_any_element());
        out.push(
            flat(c, "deb-", "−".into(), x0 + 200.0, py0 + 336.0, 24.0, 22.0, false)
                .on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.mutate(config::set_debounce(debounce.saturating_sub(1)), cx)
                    }),
                )
                .into_any_element(),
        );
        out.push(
            c.text(debounce.to_string(), x0 + 226.0, py0 + 334.0, 30.0, 26.0, 14.0, TEXT)
                .justify_center()
                .into_any_element(),
        );
        out.push(
            flat(c, "deb+", "+".into(), x0 + 258.0, py0 + 336.0, 24.0, 22.0, false)
                .on_click(cx.listener(move |this, _, _, cx| this.mutate(config::set_debounce(debounce + 1), cx)))
                .into_any_element(),
        );
        let items: [(&'static str, &'static str, bool); 4] = [
            ("motion-sync", "Motion Sync", cfg.motion_sync != 0),
            ("angle-snap", "Angle Snapping", cfg.angle_snap != 0),
            ("ripple", "Ripple Control", cfg.ripple != 0),
            ("leds-off-moving", "Гасить свет при движении", cfg.leds_off_moving != 0),
        ];
        for (n, (name, label, on)) in items.into_iter().enumerate() {
            let y = py0 + 372.0 + n as f32 * 34.0;
            out.push(c.text(label, x0, y, 200.0, 26.0, 13.0, TEXT).into_any_element());
            out.push(
                flat(c, ("sw", n), if on { "ВКЛ".into() } else { "ВЫКЛ".into() }, x0 + 200.0, y, 66.0, 26.0, on)
                    .on_click(cx.listener(move |this, _, _, cx| this.mutate(config::set_flag(name, !on), cx)))
                    .into_any_element(),
            );
        }
        out
    }

    fn slider(
        &self,
        c: &Canvas,
        id: &'static str,
        d: Drag,
        y: f32,
        frac: f32,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (tx, tw) = Self::track(d);
        vec![
            c.at(tx, y + 9.0, tw, 5.0).rounded(c.px(3.0)).bg(rgb(LINE)).into_any_element(),
            c.at(tx, y + 9.0, tw * frac, 5.0).rounded(c.px(3.0)).bg(rgb(RED)).into_any_element(),
            c.at(tx + tw * frac - 8.0, y + 3.0, 16.0, 16.0).rounded_full().bg(rgb(0xffffff)).into_any_element(),
            c.at(tx - 8.0, y - 6.0, tw + 16.0, 34.0)
                .id(id)
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.drag = Some(d);
                        this.drag_to(d, ev.position.x, cx);
                    }),
                )
                .into_any_element(),
        ]
    }

    fn page_light(&self, c: &Canvas, cfg: &Config, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (px0, py0) = PAGE;
        let bar = cfg.bar;
        let mut out: Vec<AnyElement> = Vec::new();
        let mode = config::BAR_MODES.get(bar.mode as usize).copied().unwrap_or("?");

        out.push(c.text("Режим подсветки", px0 + 39.0, py0 + 66.0, 200.0, 24.0, 14.0, TEXT).into_any_element());
        out.push(
            flat(c, "light-mode", format!("{mode}   ▾"), px0 + 43.0, py0 + 105.0, 304.0, 35.0, false)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.menu = Some(Menu::Mode);
                    cx.notify();
                }))
                .into_any_element(),
        );

        out.push(c.text("Яркость", px0 + 39.0, py0 + 170.0, 120.0, 24.0, 14.0, TEXT).into_any_element());
        out.extend(self.slider(
            c,
            "light-bright",
            Drag::Bright,
            py0 + 207.0,
            bar.brightness as f32 / config::BAR_LEVELS as f32,
            cx,
        ));
        out.push(
            c.text(bar.brightness.to_string(), px0 + 330.0, py0 + 205.0, 30.0, 24.0, 14.0, TEXT).into_any_element(),
        );

        out.push(c.text("Скорость", px0 + 39.0, py0 + 262.0, 120.0, 24.0, 14.0, TEXT).into_any_element());
        out.extend(self.slider(
            c,
            "light-speed",
            Drag::Speed,
            py0 + 297.0,
            bar.speed as f32 / config::BAR_LEVELS as f32,
            cx,
        ));
        out.push(c.text(bar.speed.to_string(), px0 + 330.0, py0 + 295.0, 30.0, 24.0, 14.0, TEXT).into_any_element());

        let moving_off = cfg.leds_off_moving != 0;
        out.push(
            c.text("Отключение при движении", px0 + 39.0, py0 + 359.0, 170.0, 24.0, 13.0, TEXT).into_any_element(),
        );
        out.push(
            flat(
                c,
                "light-moving",
                if moving_off { "ВКЛ".into() } else { "ВЫКЛ".into() },
                px0 + 244.0,
                py0 + 357.0,
                66.0,
                26.0,
                moving_off,
            )
            .on_click(
                cx.listener(move |this, _, _, cx| this.mutate(config::set_flag("leds-off-moving", !moving_off), cx)),
            )
            .into_any_element(),
        );
        let idle = config::IDLE_TIMES.get(cfg.leds_off_after as usize).copied().unwrap_or("?");
        out.push(
            c.text("Отключение при бездействии", px0 + 39.0, py0 + 412.0, 200.0, 31.0, 12.0, TEXT).into_any_element(),
        );
        out.push(
            flat(c, "light-idle", format!("{idle}  ▾"), px0 + 244.0, py0 + 412.0, 133.0, 31.0, false)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.menu = Some(Menu::Idle);
                    cx.notify();
                }))
                .into_any_element(),
        );

        // colour panel
        let (cx0, cy0) = (px0 + 411.0, py0 + 44.0);
        out.push(c.text("Цвет подсветки", cx0 + 3.0, cy0 + 22.0, 200.0, 24.0, 14.0, TEXT).into_any_element());
        out.push(c.pic("color_bar.png", cx0 + 8.0, cy0 + 62.0, 308.0, 34.0));
        out.push(
            c.at(cx0 + 8.0, cy0 + 56.0, 308.0, 46.0)
                .id("light-hue")
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.drag = Some(Drag::Hue);
                        this.drag_to(Drag::Hue, ev.position.x, cx);
                    }),
                )
                .into_any_element(),
        );
        out.push(
            c.text("Предварительный просмотр цвета", cx0 + 8.0, cy0 + 126.0, 300.0, 24.0, 13.0, MUTED)
                .into_any_element(),
        );
        out.push(
            c.at(cx0 + 8.0, cy0 + 179.0, 60.0, 60.0)
                .rounded(c.px(6.0))
                .bg(rgb(rgb_of(bar.color)))
                .border_1()
                .border_color(rgb(LINE))
                .into_any_element(),
        );
        for (n, (label, ch)) in [("R:", 0usize), ("G:", 1), ("B:", 2)].into_iter().enumerate() {
            let y = cy0 + 166.0 + n as f32 * 30.0;
            let v = bar.color[ch];
            let set = move |delta: i32| {
                let mut color = bar.color;
                color[ch] = (v as i32 + delta).clamp(0, 255) as u8;
                config::bar_patch(&config::Bar { color, ..bar })
            };
            out.push(c.text(label, cx0 + 101.0, y, 24.0, 28.0, 13.0, TEXT).into_any_element());
            out.push(
                flat(c, ("rgb-", n), "−".into(), cx0 + 127.0, y + 3.0, 24.0, 22.0, false)
                    .on_click(cx.listener(move |this, _, _, cx| this.mutate(set(-5), cx)))
                    .into_any_element(),
            );
            out.push(c.text(v.to_string(), cx0 + 153.0, y, 40.0, 28.0, 13.0, TEXT).justify_center().into_any_element());
            out.push(
                flat(c, ("rgb+", n), "+".into(), cx0 + 195.0, y + 3.0, 24.0, 22.0, false)
                    .on_click(cx.listener(move |this, _, _, cx| this.mutate(set(5), cx)))
                    .into_any_element(),
            );
        }
        out.push(c.text("Цвета по умолчанию", cx0 + 6.0, cy0 + 278.0, 300.0, 24.0, 13.0, MUTED).into_any_element());
        for (n, ((x, y), color)) in PRESET_SPOTS.iter().zip(PRESETS).enumerate() {
            let active = bar.color == color;
            out.push(
                c.at(cx0 + x, cy0 + y, 28.0, 28.0)
                    .id(("preset", n))
                    .rounded(c.px(3.0))
                    .bg(rgb(rgb_of(color)))
                    .border_2()
                    .border_color(rgb(if active { 0xffffff } else { 0x777777 }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(b) = this.bar() {
                            this.mutate(config::bar_patch(&config::Bar { color, ..b }), cx);
                        }
                    }))
                    .into_any_element(),
            );
        }
        out.push(
            c.text("Цвет используется в режиме «Один цвет».", cx0 + 8.0, cy0 + 412.0, 360.0, 20.0, 12.0, MUTED)
                .into_any_element(),
        );
        out
    }

    /// The open drop-down list, with a catcher behind it that closes it.
    fn menu_layer(&self, c: &Canvas, cfg: &Config, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(menu) = self.menu else { return Vec::new() };
        let (px0, py0) = PAGE;
        let (items, current, x, y, w, row_h): (Vec<&str>, usize, f32, f32, f32, f32) = match menu {
            Menu::Mode => (config::BAR_MODES.to_vec(), cfg.bar.mode as usize, px0 + 43.0, py0 + 140.0, 304.0, 30.0),
            Menu::Idle => {
                let h = config::IDLE_TIMES.len() as f32 * 26.0;
                (config::IDLE_TIMES.to_vec(), cfg.leds_off_after as usize, px0 + 244.0, py0 + 412.0 - h, 133.0, 26.0)
            }
        };
        let mut out: Vec<AnyElement> = vec![c
            .at(0.0, 0.0, SCENE_W, SCENE_H)
            .id("menu-catcher")
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| {
                this.menu = None;
                cx.notify();
            }))
            .into_any_element()];
        for (i, label) in items.into_iter().enumerate() {
            out.push(
                flat(c, ("menu-item", i), label.to_string(), x, y + i as f32 * row_h, w, row_h, i == current)
                    .occlude()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        let patch = match (menu, this.bar()) {
                            (Menu::Mode, Some(b)) => config::bar_patch(&config::Bar { mode: i as u8, ..b }),
                            (Menu::Idle, _) => config::set_leds_off_after(i as u8),
                            _ => Err("нет связи с мышью".into()),
                        };
                        this.mutate(patch, cx);
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    fn page_macros(&self, c: &Canvas, cfg: &Config, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (px0, py0) = PAGE;
        let mut out: Vec<AnyElement> = Vec::new();
        out.push(c.text("Кнопки", px0 + 66.0, py0 + 46.0, 200.0, 21.0, 14.0, TEXT).into_any_element());
        out.push(c.text("Действия", px0 + 338.0, py0 + 46.0, 200.0, 21.0, 14.0, TEXT).into_any_element());

        // keys
        let visible = if self.show_extra { config::KEY_COUNT } else { PHYSICAL_KEYS };
        for i in 0..visible {
            let k = cfg.keys[i];
            let label = if k[0] == 6 {
                match self.stored.get(&(k[1] as usize)) {
                    Some(Some(m)) => format!("{}  ·  макрос «{}»", i + 1, m.name),
                    Some(None) => format!("{}  ·  макрос (нет данных)", i + 1),
                    None => format!("{}  ·  макрос…", i + 1),
                }
            } else {
                format!("{}  ·  {}", i + 1, key_name(k))
            };
            out.push(
                flat(c, ("mkey", i), label, px0 + 70.0, py0 + 70.0 + i as f32 * 26.0, 250.0, 24.0, i == self.macro_key)
                    .justify_start()
                    .px(c.px(8.0))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.macro_key = i;
                        this.draft = None;
                        this.sel_action = None;
                        this.recording = false;
                        if let Some(snap) = &this.snap {
                            let k = Config::parse(&snap.img).keys[i];
                            this.repeat = if k[0] == 6 { k[2].min(3) } else { 1 };
                        }
                        this.load_slots(cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        out.push(self.extra_toggle(c, px0 + 70.0, py0 + 70.0 + visible as f32 * 26.0 + 8.0, 250.0, cx));

        // actions of the selected key
        let actions = self.shown_actions();
        let mut list = c
            .at(px0 + 342.0, py0 + 70.0, 250.0, 430.0)
            .id("actions")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgb(LINE))
            .rounded(c.px(4.0));
        if actions.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_size(c.px(12.0))
                    .text_color(rgb(MUTED))
                    .child("Пусто. Нажмите «Запись» и нажимайте клавиши."),
            );
        }
        for (i, a) in actions.iter().enumerate() {
            let sel = self.sel_action == Some(i);
            list = list.child(
                div()
                    .id(("action", i))
                    .flex_none()
                    .h(c.px(22.0))
                    .px(c.px(8.0))
                    .flex()
                    .items_center()
                    .text_size(c.px(12.0))
                    .text_color(rgb(if sel { 0xffffff } else { TEXT }))
                    .bg(rgb(if sel { RED } else { 0x161616 }))
                    .cursor_pointer()
                    .child(format!(
                        "{:>2}   {}  {}   ·  {} мс",
                        i + 1,
                        match a.state {
                            State::Down => "↓",
                            State::Up => "↑",
                            State::None => "•",
                        },
                        macros::action_name(a),
                        a.delay
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sel_action = Some(i);
                        cx.notify();
                    })),
            );
        }
        out.push(list.into_any_element());

        // recorder and options
        let x = px0 + 632.0;
        out.push(
            flat(
                c,
                "record",
                if self.recording { "■  Стоп".into() } else { "●  Запись".into() },
                x,
                py0 + 35.0,
                190.0,
                32.0,
                self.recording,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.recording = !this.recording;
                if this.recording {
                    this.draft.get_or_insert_with(Vec::new);
                    this.mods = Modifiers::default();
                    this.last_event = None;
                    window.focus(&this.focus);
                }
                cx.notify();
            }))
            .into_any_element(),
        );
        out.push(
            c.text(
                if self.recording {
                    "Идёт запись: нажимайте клавиши в этом окне"
                } else {
                    "Запись — клавиши в этом окне"
                },
                x,
                py0 + 72.0,
                230.0,
                34.0,
                11.0,
                if self.recording { WARN } else { MUTED },
            )
            .items_start()
            .into_any_element(),
        );
        out.push(c.text("Паузы между действиями", x, py0 + 118.0, 220.0, 22.0, 13.0, TEXT).into_any_element());
        for (n, (label, real)) in [("10 мс", false), ("как записаны", true)].into_iter().enumerate() {
            out.push(
                flat(
                    c,
                    ("pauses", n),
                    label.to_string(),
                    x + n as f32 * 96.0,
                    py0 + 144.0,
                    90.0,
                    26.0,
                    self.real_pauses == real,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.real_pauses = real;
                    cx.notify();
                }))
                .into_any_element(),
            );
        }
        out.push(c.text("Повторы", x, py0 + 196.0, 120.0, 22.0, 13.0, TEXT).into_any_element());
        let repeat = self.repeat;
        out.push(
            flat(c, "rep-", "−".into(), x, py0 + 222.0, 26.0, 24.0, false)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.repeat = repeat.saturating_sub(1);
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(
            c.text(repeat.to_string(), x + 30.0, py0 + 222.0, 30.0, 24.0, 14.0, TEXT)
                .justify_center()
                .into_any_element(),
        );
        out.push(
            flat(c, "rep+", "+".into(), x + 64.0, py0 + 222.0, 26.0, 24.0, false)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.repeat = (repeat + 1).min(3);
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(
            c.text("0 — пока кнопка удержана", x + 100.0, py0 + 222.0, 140.0, 24.0, 11.0, MUTED).into_any_element(),
        );
        out.push(
            flat(c, "save-macro", "Записать в мышь".into(), x, py0 + 470.0, 190.0, 30.0, false)
                .on_click(cx.listener(|this, _, _, cx| this.save_macro(cx)))
                .into_any_element(),
        );

        // bottom row, as in the vendor window
        let key = self.macro_key;
        let is_macro = cfg.keys[key][0] == 6;
        out.push(
            flat(c, "m-new", "Новый".into(), px0 + 73.0, py0 + 525.0, 116.0, 30.0, false)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.draft = Some(Vec::new());
                    this.sel_action = None;
                    this.recording = false;
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(
            flat(c, "m-edit", "Изменить".into(), px0 + 204.0, py0 + 525.0, 116.0, 30.0, false)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.draft.is_none() {
                        this.draft = Some(this.shown_actions());
                    }
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(
            flat(c, "m-del-action", "Удалить действие".into(), px0 + 342.0, py0 + 525.0, 116.0, 30.0, false)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.draft.is_none() {
                        this.draft = Some(this.shown_actions());
                    }
                    if let (Some(d), Some(i)) = (this.draft.as_mut(), this.sel_action) {
                        if i < d.len() {
                            d.remove(i);
                        }
                    }
                    this.sel_action = None;
                    cx.notify();
                }))
                .into_any_element(),
        );
        out.push(
            flat(
                c,
                "m-off",
                if is_macro {
                    "Отключить кнопку".into()
                } else {
                    "Кнопка не макрос".into()
                },
                px0 + 476.0,
                py0 + 525.0,
                116.0,
                30.0,
                false,
            )
            .opacity(if is_macro { 1.0 } else { 0.4 })
            .on_click(cx.listener(move |this, _, _, cx| {
                if is_macro {
                    this.draft = None;
                    this.mutate(config::set_key(key, 0, 0), cx);
                }
            }))
            .into_any_element(),
        );
        out
    }

    fn page_soon(&self, c: &Canvas, title: &str, text: &str) -> Vec<AnyElement> {
        let (px0, py0) = PAGE;
        vec![
            c.text(title, px0 + 60.0, py0 + 60.0, 600.0, 30.0, 20.0, TEXT).into_any_element(),
            c.text(text, px0 + 60.0, py0 + 100.0, 760.0, 80.0, 14.0, MUTED).items_start().into_any_element(),
        ]
    }

    fn page_settings(&self, c: &Canvas) -> Vec<AnyElement> {
        let (px0, py0) = PAGE;
        let mut lines = vec![format!("mouse-config {}", env!("CARGO_PKG_VERSION"))];
        if let Some(s) = &self.snap {
            lines.push(format!("Узел устройства: {}", s.node));
            lines.push(format!(
                "Подключение: {}",
                if s.link == Link::Wired { "кабель USB (3554:f52e)" } else { "донгл 2.4G (3554:f52d)" }
            ));
        }
        lines.push("Резервные копии настроек: ~/.config/mouse-config/".into());
        lines.push("Перед первой записью за сеанс текущие настройки сохраняются автоматически.".into());
        let mut out = vec![c.text("Настройки", px0 + 60.0, py0 + 40.0, 600.0, 30.0, 20.0, TEXT).into_any_element()];
        for (i, l) in lines.into_iter().enumerate() {
            out.push(c.text(l, px0 + 60.0, py0 + 90.0 + i as f32 * 30.0, 800.0, 26.0, 14.0, MUTED).into_any_element());
        }
        out
    }

    fn page_home(&self, c: &Canvas, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let online = self.snap.as_ref().is_some_and(|s| s.online || s.link == Link::Wired);
        let (x, y) = (410.0, 220.0);
        out.push(c.pic(if online { "home_on.png" } else { "home_off.png" }, x, y, 200.0, 253.0));
        out.push(
            c.at(x, y, 200.0, 253.0)
                .id("home-card")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.home = false;
                    this.tab = 0;
                    cx.notify();
                }))
                .into_any_element(),
        );
        if let Some(s) = &self.snap {
            out.push(c.pic_nat(
                if s.link == Link::Wired { "wired.png" } else { "wireless.png" },
                x + 8.0,
                y + 8.0,
                34.0,
                40.0,
            ));
        }
        out.push(c.text("Phantom Pro V2", x, y + 262.0, 200.0, 24.0, 15.0, TEXT).justify_center().into_any_element());
        out.push(
            c.text("нажмите, чтобы настроить", x, y + 286.0, 200.0, 20.0, 12.0, MUTED)
                .justify_center()
                .into_any_element(),
        );
        out
    }

    fn status(&self, c: &Canvas) -> AnyElement {
        let (text, color) = match (&self.note, self.writing || !self.queue.is_empty()) {
            (_, true) => ("Запись…".to_string(), WARN),
            (Some((t, true)), _) => (t.lines().next().unwrap_or("").to_string(), RED),
            (Some((t, false)), _) => (t.clone(), OK),
            (None, _) if self.snap.is_none() => {
                ("Мышь не найдена: подключите донгл 2.4G или кабель USB".to_string(), MUTED)
            }
            (None, _) => (String::new(), MUTED),
        };
        c.text(text, 221.0, 46.0, 560.0, 18.0, 12.0, color).into_any_element()
    }
}

impl Render for Gui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let k = (vw / SCENE_W).min(vh / SCENE_H).max(0.2);
        let c = Canvas { k, ox: (vw - SCENE_W * k) / 2.0, oy: 0.0 };
        self.canvas = c;

        let mut scene = div().absolute().left(px(c.ox)).top(px(c.oy)).w(c.px(SCENE_W)).h(c.px(SCENE_H));
        for e in self.chrome(&c, cx) {
            scene = scene.child(e);
        }
        let page: Vec<AnyElement> = if self.home {
            self.page_home(&c, cx)
        } else if let Some(snap) = &self.snap {
            let cfg = Config::parse(&snap.img);
            match self.tab {
                0 => self.page_buttons(&c, &cfg, cx),
                1 => self.page_sensor(&c, &cfg, cx),
                2 => self.page_macros(&c, &cfg, cx),
                3 => self.page_light(&c, &cfg, cx),
                _ => self.page_settings(&c),
            }
        } else {
            self.page_soon(&c, "Мышь не найдена", "Подключите донгл 2.4G или кабель USB (3554:f52d / 3554:f52e).")
        };
        for e in page {
            scene = scene.child(e);
        }
        if let Some(snap) = &self.snap {
            if self.tab == 3 && !self.home {
                for e in self.menu_layer(&c, &Config::parse(&snap.img), cx) {
                    scene = scene.child(e);
                }
            }
        }
        scene = scene.child(self.status(&c));

        div()
            .size_full()
            .bg(rgb(0x070707))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if !ev.is_held {
                    this.on_key(&ev.keystroke.key, true, cx);
                }
            }))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| this.on_key(&ev.keystroke.key, false, cx)))
            .on_modifiers_changed(
                cx.listener(|this, ev: &ModifiersChangedEvent, _, cx| this.on_modifiers(ev.modifiers, cx)),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some(d) = this.drag {
                    if ev.pressed_button == Some(MouseButton::Left) {
                        this.drag_to(d, ev.position.x, cx);
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if let Some(d) = this.drag.take() {
                        this.commit_drag(d, cx);
                    }
                }),
            )
            // the vendor background continues over the whole window: its gradient (sampled from bg.png), then its bottom edge
            .children(BG_STOPS.windows(2).map(|w| {
                let ((y0, c0), (y1, c1)) = (w[0], w[1]);
                div().absolute().left(px(0.0)).top(c.px(y0)).w_full().h(c.px(y1 - y0 + 0.5)).bg(linear_gradient(
                    180.,
                    linear_color_stop(rgb(c0), 0.0),
                    linear_color_stop(rgb(c1), 1.0),
                ))
            }))
            .child(div().absolute().left(px(0.0)).top(c.px(SCENE_H - 1.0)).w_full().h(px(1.0)).bg(rgb(0x2c2c2c)))
            .child(scene)
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(SCENE_W), px(SCENE_H)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("mouse-config".into()),
                titlebar: Some(TitlebarOptions { title: Some("Phantom Pro V2".into()), ..Default::default() }),
                ..Default::default()
            },
            |_, cx| cx.new(Gui::new),
        )
        .unwrap();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
