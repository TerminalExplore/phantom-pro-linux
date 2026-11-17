//! Decoding and patching of the mouse's configuration image (bytes 0x0000..0x00c0).
//!
//! Single values are stored as pairs `(v, 0x55 - v)`, three-byte values (DPI, RGB, keys) as groups
//! `(a, b, c, 0x55 - (a+b+c))`. Field order follows the vendor's DriverLib.FlashDataMap / MouseConfig.

pub const IMAGE_LEN: usize = 0xc0;

const RATE: usize = 0x00;
const STAGES: usize = 0x02;
const CURRENT: usize = 0x04;
const LOD: usize = 0x0a;
const DPI: usize = 0x0c;
const COLOR: usize = 0x2c;
const LED: usize = 0x4c;
const KEYS: usize = 0x60;
/// Body light: `mode, R, G, B, speed, brightness, chk` as one block, then the enable pair.
const BAR: usize = 0xa0;
const BAR_ENABLE: usize = 0xa7;
const DEBOUNCE: usize = 0xa9;
const MOTION_SYNC: usize = 0xab;
const LEDS_OFF_AFTER: usize = 0xad;
const ANGLE_SNAP: usize = 0xaf;
const RIPPLE: usize = 0xb1;
const LEDS_OFF_MOVING: usize = 0xb3;
const SLEEP_CUSTOM: usize = 0xb5;
const SLEEP_TIME: usize = 0xb7;
const POWER_SAVING: usize = 0xb9;

pub const STAGE_COUNT: usize = 8;
pub const KEY_COUNT: usize = 16;

/// DPI codes above this need the DPIex byte, whose bit layout is not known yet.
pub const MAX_DPI: u32 = 12800;

pub struct Patch {
    pub addr: usize,
    pub bytes: Vec<u8>,
}

/// The body light ("LedBar" in the vendor's structures).
#[derive(Clone, Copy)]
pub struct Bar {
    pub mode: u8,
    pub color: [u8; 3],
    pub speed: u8,
    pub brightness: u8,
    pub enable: u8,
}

pub const BAR_MODES: [&str; 7] =
    ["Выключить", "Радуга", "Один цвет", "Радуга фиксированная", "Неон", "Радуга дышит", "Красочный"];
pub const IDLE_TIMES: [&str; 11] =
    ["10 сек.", "30 сек.", "1 мин", "5 мин", "10 мин", "15 мин", "20 мин", "25 мин", "30 мин", "35 мин", "40 мин"];
/// Speed and brightness of the body light run 0..=9.
pub const BAR_LEVELS: u8 = 9;

pub struct Config {
    pub rate: u8,
    pub stages: u8,
    pub current: u8,
    pub lod: u8,
    pub dpi: [(u16, u16, u8); STAGE_COUNT],
    pub colors: [[u8; 3]; STAGE_COUNT],
    pub led: [u8; 4],
    pub keys: [[u8; 3]; KEY_COUNT],
    pub bar: Bar,
    pub debounce: u8,
    pub motion_sync: u8,
    pub leds_off_after: u8,
    pub angle_snap: u8,
    pub ripple: u8,
    pub leds_off_moving: u8,
    pub sleep_custom: u8,
    pub sleep_time: u8,
    pub power_saving: u8,
    pub problems: Vec<String>,
}

pub fn pair(v: u8) -> [u8; 2] {
    [v, 0x55u8.wrapping_sub(v)]
}

pub fn group(a: u8, b: u8, c: u8) -> [u8; 4] {
    [a, b, c, 0x55u8.wrapping_sub(a).wrapping_sub(b).wrapping_sub(c)]
}

fn unpair(img: &[u8], at: usize, name: &str, problems: &mut Vec<String>) -> u8 {
    if img[at].wrapping_add(img[at + 1]) != 0x55 {
        problems.push(format!("{name} @ {at:#04x}: {:02x} {:02x} is not a valid pair", img[at], img[at + 1]));
    }
    img[at]
}

fn ungroup(img: &[u8], at: usize, name: &str, problems: &mut Vec<String>) -> [u8; 3] {
    let g = &img[at..at + 4];
    if g[0].wrapping_add(g[1]).wrapping_add(g[2]).wrapping_add(g[3]) != 0x55 {
        problems.push(format!("{name} @ {at:#04x}: {g:02x?} has a bad checksum"));
    }
    [g[0], g[1], g[2]]
}

/// DPI of a sensor code: PAW3395 uses `(code + 1) * 50`.
pub fn code_to_dpi(code: u8) -> u32 {
    (code as u32 + 1) * 50
}

pub fn dpi_to_code(dpi: u32) -> Result<u8, String> {
    if !dpi.is_multiple_of(50) || !(50..=MAX_DPI).contains(&dpi) {
        return Err(format!("DPI must be a multiple of 50 between 50 and {MAX_DPI}"));
    }
    Ok((dpi / 50 - 1) as u8)
}

pub fn rate_hz(mask: u8) -> Option<u32> {
    Some(match mask {
        0x01 => 1000,
        0x02 => 500,
        0x04 => 250,
        0x08 => 125,
        0x10 => 2000,
        0x20 => 4000,
        _ => return None,
    })
}

pub fn hz_to_rate(hz: u32) -> Result<u8, String> {
    match hz {
        1000 => Ok(0x01),
        500 => Ok(0x02),
        250 => Ok(0x04),
        125 => Ok(0x08),
        _ => Err("report rate must be 125, 250, 500 or 1000 (2000/4000 Hz need the 4K dongle)".into()),
    }
}

pub fn key_class(class: u8) -> &'static str {
    match class {
        0 => "off",
        1 => "mouse button",
        2 => "DPI",
        3 => "wheel/axis",
        4 => "fire",
        5 => "shortcut",
        6 => "macro",
        7 => "report rate",
        8 => "lamp",
        9 => "profile",
        10 => "DPI lock",
        _ => "?",
    }
}

impl Config {
    pub fn parse(img: &[u8]) -> Config {
        assert!(img.len() >= IMAGE_LEN);
        let mut p = Vec::new();
        let mut dpi = [(0, 0, 0); STAGE_COUNT];
        let mut colors = [[0; 3]; STAGE_COUNT];
        for i in 0..STAGE_COUNT {
            let g = ungroup(img, DPI + i * 4, &format!("dpi[{i}]"), &mut p);
            dpi[i] = (g[0] as u16, g[1] as u16, g[2]);
            colors[i] = ungroup(img, COLOR + i * 4, &format!("color[{i}]"), &mut p);
        }
        let mut keys = [[0; 3]; KEY_COUNT];
        for (i, k) in keys.iter_mut().enumerate() {
            *k = ungroup(img, KEYS + i * 4, &format!("key[{i}]"), &mut p);
        }
        let mut led = [0; 4];
        for (i, l) in led.iter_mut().enumerate() {
            *l = unpair(img, LED + i * 2, "dpi led", &mut p);
        }
        let blk = &img[BAR..BAR + 7];
        if blk[..7].iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0x55 {
            p.push(format!("body light @ {BAR:#04x}: {blk:02x?} has a bad checksum"));
        }
        let mut u = |at, name| unpair(img, at, name, &mut p);
        let bar_enable = u(BAR_ENABLE, "body light enable");
        Config {
            rate: u(RATE, "report rate"),
            stages: u(STAGES, "stage count"),
            current: u(CURRENT, "current stage"),
            lod: u(LOD, "lod"),
            dpi,
            colors,
            led,
            keys,
            bar: Bar {
                mode: blk[0],
                color: [blk[1], blk[2], blk[3]],
                speed: blk[4],
                brightness: blk[5],
                enable: bar_enable,
            },
            debounce: u(DEBOUNCE, "debounce"),
            motion_sync: u(MOTION_SYNC, "motion sync"),
            leds_off_after: u(LEDS_OFF_AFTER, "leds off after"),
            angle_snap: u(ANGLE_SNAP, "angle snapping"),
            ripple: u(RIPPLE, "ripple"),
            leds_off_moving: u(LEDS_OFF_MOVING, "leds off when moving"),
            sleep_custom: u(SLEEP_CUSTOM, "custom sleep"),
            sleep_time: u(SLEEP_TIME, "sleep time"),
            power_saving: u(POWER_SAVING, "power saving"),
            problems: p,
        }
    }

    pub fn describe(&self) -> String {
        let mut s = String::new();
        let on = |v: u8| if v != 0 { "on" } else { "off" };
        let rate = rate_hz(self.rate).map_or(format!("unknown ({:#04x})", self.rate), |h| format!("{h} Hz"));
        s += &format!("report rate        {rate}\n");
        s += &format!("lift-off (LOD)     {} (raw)\n", self.lod);
        s += &format!("key debounce       {} ms\n", self.debounce);
        s += &format!("motion sync        {}\n", on(self.motion_sync));
        s += &format!("angle snapping     {}\n", on(self.angle_snap));
        s += &format!("ripple control     {}\n", on(self.ripple));
        s += &format!("leds off when moving {}, after idle index {}\n", on(self.leds_off_moving), self.leds_off_after);
        s += &format!(
            "sensor sleep       custom {}, time index {}, power saving {}\n",
            on(self.sleep_custom),
            self.sleep_time,
            on(self.power_saving)
        );
        let b = &self.bar;
        let mode = BAR_MODES.get(b.mode as usize).copied().unwrap_or("?");
        s += &format!(
            "body light         {mode}, colour #{:02x}{:02x}{:02x}, speed {}, brightness {}, {}\n",
            b.color[0],
            b.color[1],
            b.color[2],
            b.speed,
            b.brightness,
            on(b.enable)
        );
        s += &format!(
            "dpi light          mode {}, brightness {}, speed {}, enabled {}\n",
            self.led[0],
            self.led[1],
            self.led[2],
            on(self.led[3])
        );
        s += &format!("DPI stages         {} active, current #{}\n", self.stages, self.current + 1);
        for i in 0..STAGE_COUNT {
            let (x, y, ex) = self.dpi[i];
            let c = self.colors[i];
            let mark = if i == self.current as usize {
                '*'
            } else if i < self.stages as usize {
                ' '
            } else {
                '-'
            };
            let ex = if ex != 0 { format!("  (ex {ex:#04x})") } else { String::new() };
            s += &format!(
                "  {mark} #{}  x {:>5}  y {:>5}  color #{:02x}{:02x}{:02x}{ex}\n",
                i + 1,
                code_to_dpi(x as u8),
                code_to_dpi(y as u8),
                c[0],
                c[1],
                c[2]
            );
        }
        s += "buttons\n";
        for (i, k) in self.keys.iter().enumerate() {
            s += &format!("  {:>2}  {:<13} value {:#04x} extra {:#04x}\n", i + 1, key_class(k[0]), k[1], k[2]);
        }
        for p in &self.problems {
            s += &format!("warning: {p}\n");
        }
        s
    }
}

fn single(at: usize, v: u8) -> Patch {
    Patch { addr: at, bytes: pair(v).to_vec() }
}

pub fn set_rate(hz: u32) -> Result<Patch, String> {
    Ok(single(RATE, hz_to_rate(hz)?))
}

pub fn set_lod(raw: u8) -> Result<Patch, String> {
    if !(1..=2).contains(&raw) {
        return Err("lod must be 1 or 2".into());
    }
    Ok(single(LOD, raw))
}

pub fn set_stage_count(n: usize) -> Result<Patch, String> {
    if !(1..=STAGE_COUNT).contains(&n) {
        return Err(format!("stage count must be 1..{STAGE_COUNT}"));
    }
    Ok(single(STAGES, n as u8))
}

pub fn set_current(stage: usize) -> Result<Patch, String> {
    if !(1..=STAGE_COUNT).contains(&stage) {
        return Err(format!("stage must be 1..{STAGE_COUNT}"));
    }
    Ok(single(CURRENT, (stage - 1) as u8))
}

pub fn set_dpi(stage: usize, dpi: u32) -> Result<Patch, String> {
    if !(1..=STAGE_COUNT).contains(&stage) {
        return Err(format!("stage must be 1..{STAGE_COUNT}"));
    }
    let code = dpi_to_code(dpi)?;
    Ok(Patch { addr: DPI + (stage - 1) * 4, bytes: group(code, code, 0).to_vec() })
}

pub fn set_color(stage: usize, rgb: [u8; 3]) -> Result<Patch, String> {
    if !(1..=STAGE_COUNT).contains(&stage) {
        return Err(format!("stage must be 1..{STAGE_COUNT}"));
    }
    Ok(Patch { addr: COLOR + (stage - 1) * 4, bytes: group(rgb[0], rgb[1], rgb[2]).to_vec() })
}

pub fn set_flag(name: &str, on: bool) -> Result<Patch, String> {
    let at = match name {
        "motion-sync" => MOTION_SYNC,
        "angle-snap" => ANGLE_SNAP,
        "ripple" => RIPPLE,
        "leds-off-moving" => LEDS_OFF_MOVING,
        _ => return Err(format!("unknown switch {name}")),
    };
    Ok(single(at, on as u8))
}

/// Assign a key: `class` is the vendor's KEY_CLASS (1 = mouse button, 2 = DPI, 0 = off), `value` its argument.
pub fn set_key(index: usize, class: u8, value: u8) -> Result<Patch, String> {
    if index >= KEY_COUNT {
        return Err(format!("key must be 1..{KEY_COUNT}"));
    }
    Ok(Patch { addr: KEYS + index * 4, bytes: group(class, value, 0).to_vec() })
}

/// The whole six-byte light block has to be rewritten together because one checksum covers it.
pub fn bar_patch(b: &Bar) -> Result<Patch, String> {
    if b.mode as usize >= BAR_MODES.len() {
        return Err(format!("light mode must be 0..{}", BAR_MODES.len() - 1));
    }
    if b.speed > BAR_LEVELS || b.brightness > BAR_LEVELS {
        return Err(format!("speed and brightness must be 0..{BAR_LEVELS}"));
    }
    let data = [b.mode, b.color[0], b.color[1], b.color[2], b.speed, b.brightness];
    let chk = data.iter().fold(0x55u8, |a, x| a.wrapping_sub(*x));
    let mut bytes = data.to_vec();
    bytes.push(chk);
    Ok(Patch { addr: BAR, bytes })
}

pub fn set_bar_enable(on: bool) -> Patch {
    single(BAR_ENABLE, on as u8)
}

/// Index into IDLE_TIMES: the time without movement after which the lights go out.
pub fn set_leds_off_after(index: u8) -> Result<Patch, String> {
    if index as usize >= IDLE_TIMES.len() {
        return Err(format!("idle time index must be 0..{}", IDLE_TIMES.len() - 1));
    }
    Ok(single(LEDS_OFF_AFTER, index))
}

/// Like `set_key` with the key's second parameter (repeat count for macro keys).
pub fn set_key_ex(index: usize, class: u8, value: u8, extra: u8) -> Result<Patch, String> {
    if index >= KEY_COUNT {
        return Err(format!("key must be 1..{KEY_COUNT}"));
    }
    Ok(Patch { addr: KEYS + index * 4, bytes: group(class, value, extra).to_vec() })
}

pub fn set_debounce(ms: u8) -> Result<Patch, String> {
    if !(1..=30).contains(&ms) {
        return Err("debounce must be 1..30 ms".into());
    }
    Ok(single(DEBOUNCE, ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    // bytes read from a real mouse on 2026-11-17
    const LIVE: &str = "01 54 06 4f 02 53 00 55 00 55 01 54 07 07 00 47 0f 0f 00 37 1f 1f 00 17 2f 2f 00 f7 3f 3f 00 d7 7f 7f 00 57 7f 7f 00 57 7f 7f 00 57 ff 00 00 56 00 00 ff 56 00 ff 00 56 ff ff 00 57 00 ff ff 57 ff 00 ff 57 ff 00 ff 57 ff 00 ff 57 02 53 80 d5 03 52 01 54 ff 00 ff 57 00 55 80 d5 03 52 00 55 01 01 00 53 01 02 00 52 01 04 00 50 01 08 00 4c 01 10 00 44 02 01 00 52 00 00 00 55 00 00 00 55 07 00 00 4e 02 02 00 51 02 03 00 50 04 0a 03 44 00 00 00 55 00 00 00 55 00 00 00 55 00 00 00 55 01 ff 00 ff 07 09 46 01 54 04 51 01 54 06 4f 00 55 00 55 01 54 00 55 06 4f 01 54 0a 4b";

    fn live() -> Vec<u8> {
        let mut img: Vec<u8> = LIVE.split(' ').map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
        img.resize(IMAGE_LEN, 0xff);
        img
    }

    #[test]
    fn parses_live_image_without_problems() {
        let c = Config::parse(&live());
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        assert_eq!(c.rate, 1);
        assert_eq!((c.stages, c.current, c.debounce), (6, 2, 4));
        let dpis: Vec<u32> = c.dpi.iter().map(|d| code_to_dpi(d.0 as u8)).collect();
        assert_eq!(dpis[..6], [400, 800, 1600, 2400, 3200, 6400]);
        assert_eq!(c.colors[0], [255, 0, 0]);
        assert_eq!(c.keys[0], [1, 1, 0]);
        let b = c.bar;
        assert_eq!((b.mode, b.color, b.speed, b.brightness, b.enable), (1, [255, 0, 255], 7, 9, 1));
    }

    #[test]
    fn light_block_matches_the_stored_bytes() {
        let img = live();
        let c = Config::parse(&img);
        let p = bar_patch(&c.bar).unwrap();
        assert_eq!(p.bytes, img[p.addr..p.addr + 7]);
        assert_eq!(set_bar_enable(true).bytes, img[0xa7..0xa9]);
        assert_eq!(set_leds_off_after(6).unwrap().bytes, img[0xad..0xaf]);
        assert!(bar_patch(&Bar { speed: 10, ..c.bar }).is_err());
    }

    #[test]
    fn patches_match_the_stored_encoding() {
        let img = live();
        let p = set_dpi(3, 1600).unwrap();
        assert_eq!(p.bytes, img[p.addr..p.addr + 4]);
        let p = set_color(1, [255, 0, 0]).unwrap();
        assert_eq!(p.bytes, img[p.addr..p.addr + 4]);
        let p = set_rate(1000).unwrap();
        assert_eq!(p.bytes, img[p.addr..p.addr + 2]);
        let p = set_debounce(4).unwrap();
        assert_eq!(p.bytes, img[p.addr..p.addr + 2]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(set_dpi(1, 425).is_err());
        assert!(set_dpi(9, 800).is_err());
        assert!(set_dpi(1, 26000).is_err());
        assert!(set_rate(750).is_err());
    }
}
