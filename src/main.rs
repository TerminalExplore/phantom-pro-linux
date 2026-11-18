use mouse_config::config::{self, Config, Patch};
use mouse_config::device::{Link, Mouse};
use mouse_config::macros;
use mouse_config::ops::{apply as apply_patches, config_dir, hex, load_backup, read_image, save_backup, stamp, Opts};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
usage: mouse-config [--base ADDR] [--force] <command>

  status                      battery and whether the mouse answers through the dongle
  show                        decoded settings
  dump START END              raw bytes (hex addresses)
  backup [FILE]               save the settings area to a file
  restore FILE                write a backup file back
  write-noop                  write the first settings bytes back unchanged (protocol test)
  set rate HZ                 125 | 250 | 500 | 1000
  set lod N                   1 | 2
  set stages N                number of active DPI stages (1..8)
  set current N               current DPI stage (1..8)
  set dpi STAGE VALUE         e.g. set dpi 2 1600 (multiple of 50, up to 12800)
  set color STAGE RRGGBB      stage light colour
  set debounce MS             button debounce (1..30)
  set key KEY CLASS VALUE     raw key assignment (class 1 mouse button, 2 DPI, 0 off ...), e.g. set key 6 2 1
  macro list                  keys that hold a macro
  macro show KEY              actions of the macro on KEY (1..16)
  macro set KEY NAME SCRIPT [REPEAT]
                              store a macro and point the key at it; script words: tap:K down:K up:K
                              click:left|right|middle|back|forward wait:MS (K: a-z 0-9 f1-f12 enter esc tab space ...)
  macro store KEY NAME SCRIPT store a macro in the key's slot WITHOUT changing what the key does
  macro clear KEY             switch the key off (the mouse refuses to erase a stored macro, it can only be replaced)
  set motion-sync|angle-snap|ripple|leds-off-moving on|off
  set light-mode N            body light: 0 off, 1 rainbow, 2 one colour, 3 fixed rainbow, 4 neon, 5 breathing rainbow, 6 colourful
  set light-color RRGGBB      body light colour (used by the one-colour mode)
  set light-speed N           0..9
  set light-brightness N      0..9
  set light on|off            body light enable
  set idle-off N              lights out after idle: 0 10s, 1 30s, 2 1m, 3 5m, 4 10m, 5 15m, 6 20m, 7 25m, 8 30m, 9 35m, 10 40m

--base selects the memory copy the settings are read from / written to (default 0x0000).";

fn parse_hex(s: &str) -> Result<u16, String> {
    u16::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| format!("bad hex address {s}"))
}

fn need<'a>(args: &'a [String], i: usize, what: &str) -> Result<&'a str, String> {
    args.get(i).map(String::as_str).ok_or_else(|| format!("missing {what}"))
}

fn num<T: std::str::FromStr>(args: &[String], i: usize, what: &str) -> Result<T, String> {
    need(args, i, what)?.parse().map_err(|_| format!("bad {what}"))
}

fn parse_color(s: &str) -> Result<[u8; 3], String> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return Err("colour must be RRGGBB".into());
    }
    let v = u32::from_str_radix(s, 16).map_err(|_| "colour must be RRGGBB".to_string())?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn apply(mouse: &mut Mouse, o: &Opts, patches: Vec<Patch>) -> Result<(), String> {
    for line in apply_patches(mouse, o, patches)? {
        println!("{line}");
    }
    Ok(())
}

fn bar(mouse: &mut Mouse, o: &Opts) -> Result<config::Bar, String> {
    Ok(Config::parse(&read_image(mouse, o.base)?).bar)
}

fn set(mouse: &mut Mouse, o: &Opts, a: &[String]) -> Result<(), String> {
    let patch = match need(a, 0, "setting")? {
        "light-mode" => config::bar_patch(&config::Bar { mode: num(a, 1, "mode")?, ..bar(mouse, o)? })?,
        "light-color" => {
            config::bar_patch(&config::Bar { color: parse_color(need(a, 1, "colour")?)?, ..bar(mouse, o)? })?
        }
        "light-speed" => config::bar_patch(&config::Bar { speed: num(a, 1, "speed")?, ..bar(mouse, o)? })?,
        "light-brightness" => {
            config::bar_patch(&config::Bar { brightness: num(a, 1, "brightness")?, ..bar(mouse, o)? })?
        }
        "light" => match need(a, 1, "on|off")? {
            "on" => config::set_bar_enable(true),
            "off" => config::set_bar_enable(false),
            _ => return Err("expected on or off".into()),
        },
        "idle-off" => config::set_leds_off_after(num(a, 1, "index")?)?,
        "rate" => config::set_rate(num(a, 1, "rate")?)?,
        "lod" => config::set_lod(num(a, 1, "lod")?)?,
        "stages" => config::set_stage_count(num(a, 1, "count")?)?,
        "current" => config::set_current(num(a, 1, "stage")?)?,
        "dpi" => config::set_dpi(num(a, 1, "stage")?, num(a, 2, "dpi")?)?,
        "color" => config::set_color(num(a, 1, "stage")?, parse_color(need(a, 2, "colour")?)?)?,
        "debounce" => config::set_debounce(num(a, 1, "ms")?)?,
        "key" => {
            let key: usize = num(a, 1, "key")?;
            config::set_key(key.checked_sub(1).ok_or("key must be 1..16")?, num(a, 2, "class")?, num(a, 3, "value")?)?
        }
        name @ ("motion-sync" | "angle-snap" | "ripple" | "leds-off-moving") => {
            let on = match need(a, 1, "on|off")? {
                "on" => true,
                "off" => false,
                _ => return Err("expected on or off".into()),
            };
            config::set_flag(name, on)?
        }
        other => return Err(format!("unknown setting {other}")),
    };
    apply(mouse, o, vec![patch])
}

fn macro_key(a: &[String], i: usize) -> Result<usize, String> {
    let k: usize = num(a, i, "key")?;
    if !(1..=config::KEY_COUNT).contains(&k) {
        return Err(format!("key must be 1..{}", config::KEY_COUNT));
    }
    Ok(k - 1)
}

fn read_slot(mouse: &mut Mouse, key: usize) -> Result<Vec<u8>, String> {
    mouse.read(macros::slot_addr(key) as u16, macros::SLOT_LEN).map_err(|e| e.to_string())
}

fn macro_cmd(mouse: &mut Mouse, o: &Opts, a: &[String]) -> Result<(), String> {
    match need(a, 0, "macro command")? {
        "list" => {
            let cfg = Config::parse(&read_image(mouse, o.base)?);
            for (i, k) in cfg.keys.iter().enumerate().filter(|(_, k)| k[0] == 6) {
                let m = macros::Macro::decode(&read_slot(mouse, k[1] as usize)?);
                println!(
                    "key {:>2}: slot {} repeat {} {}",
                    i + 1,
                    k[1],
                    k[2],
                    m.map_or("(no valid macro in the slot)".into(), |m| format!(
                        "\"{}\", {} actions",
                        m.name,
                        m.actions.len()
                    ))
                );
            }
        }
        "show" => {
            let key = macro_key(a, 1)?;
            match macros::Macro::decode(&read_slot(mouse, key)?) {
                Some(m) => {
                    println!("{:?}, {} actions", m.name, m.actions.len());
                    for (i, act) in m.actions.iter().enumerate() {
                        println!("  {:>2}  {}", i + 1, macros::describe_action(act));
                    }
                }
                None => println!("no valid macro in the slot of key {}", key + 1),
            }
        }
        "set" => {
            let key = macro_key(a, 1)?;
            let m = macros::parse_script(need(a, 2, "name")?, need(a, 3, "script")?)?;
            let repeat: u8 = if a.len() > 4 { num(a, 4, "repeat")? } else { 1 };
            let patches = m.patches(key, repeat)?;
            let old = read_slot(mouse, key)?;
            let path = config_dir().join(format!("macro-slot{}-{}.txt", key + 1, stamp()));
            save_backup(&path, macros::slot_addr(key) as u16, &old).map_err(|e| e.to_string())?;
            println!("old slot saved to {}", path.display());
            apply(mouse, o, patches)?;
        }
        "store" => {
            let key = macro_key(a, 1)?;
            let m = macros::parse_script(need(a, 2, "name")?, need(a, 3, "script")?)?;
            let old = read_slot(mouse, key)?;
            let path = config_dir().join(format!("macro-slot{}-{}.txt", key + 1, stamp()));
            save_backup(&path, macros::slot_addr(key) as u16, &old).map_err(|e| e.to_string())?;
            println!("old slot saved to {}", path.display());
            apply(mouse, o, vec![Patch { addr: macros::slot_addr(key), bytes: m.encode()? }])?;
        }
        "clear" => {
            let key = macro_key(a, 1)?;
            apply(mouse, o, vec![config::set_key(key, 0, 0)?])?;
        }
        other => return Err(format!("unknown macro command {other}")),
    }
    Ok(())
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut o = Opts { base: 0x0000, force: false, backup: true };
    let mut rest = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base" => o.base = parse_hex(&it.next().ok_or("--base needs an address")?)?,
            "--force" => o.force = true,
            "-h" | "--help" | "help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => rest.push(a),
        }
    }
    let cmd = rest.first().map(String::as_str).ok_or(USAGE)?;
    let mut mouse = Mouse::open().map_err(|e| e.to_string())?;
    match cmd {
        "status" => {
            println!(
                "node     {} ({})",
                mouse.node.display(),
                if mouse.link == Link::Wired { "mouse on cable" } else { "2.4G dongle" }
            );
            let online = mouse.online().map_err(|e| e.to_string())?;
            println!("mouse    {}", if online { "answers" } else { "not reachable (off or asleep)" });
            if online {
                let b = mouse.battery().map_err(|e| e.to_string())?;
                println!("battery  {}% {} mV{}", b.percent, b.millivolts, if b.on_cable { ", on cable" } else { "" });
            }
        }
        "show" => print!("{}", Config::parse(&read_image(&mut mouse, o.base)?).describe()),
        "dump" => {
            let start = parse_hex(need(&rest, 1, "start")?)?;
            let end = parse_hex(need(&rest, 2, "end")?)?;
            let data = mouse.read(start, (end.saturating_sub(start)) as usize).map_err(|e| e.to_string())?;
            for (i, c) in data.chunks(10).enumerate() {
                println!("{:04x}: {}", start as usize + i * 10, hex(c));
            }
        }
        "backup" => {
            let path =
                rest.get(1).map(PathBuf::from).unwrap_or_else(|| config_dir().join(format!("backup-{}.txt", stamp())));
            save_backup(&path, o.base, &read_image(&mut mouse, o.base)?).map_err(|e| e.to_string())?;
            println!("saved {}", path.display());
        }
        "restore" => {
            let image = load_backup(Path::new(need(&rest, 1, "file")?))?;
            let patch = Patch { addr: 0, bytes: image };
            apply(&mut mouse, &o, vec![patch])?;
        }
        "write-noop" => {
            let image = read_image(&mut mouse, o.base)?;
            let patch = Patch { addr: 0, bytes: image[..12].to_vec() };
            apply(&mut mouse, &o, vec![patch])?;
        }
        "macro" => macro_cmd(&mut mouse, &o, &rest[1..])?,
        "set" => set(&mut mouse, &o, &rest[1..])?,
        other => return Err(format!("unknown command {other}\n{USAGE}")),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mouse-config: {e}");
            ExitCode::FAILURE
        }
    }
}
