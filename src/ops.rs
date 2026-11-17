//! Shared by the CLI and the GUI: backups and the guarded write path.

use crate::config::{Config, Patch, IMAGE_LEN};
use crate::device::Mouse;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Opts {
    pub base: u16,
    pub force: bool,
    /// Save the current settings to ~/.config/mouse-config/auto-<time>.txt before writing.
    pub backup: bool,
}

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/mouse-config")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

pub fn stamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn save_backup(path: &Path, base: u16, image: &[u8]) -> std::io::Result<()> {
    let mut text = String::new();
    for (i, chunk) in image.chunks(10).enumerate() {
        text += &format!("{:04x}: {}\n", base as usize + i * 10, hex(chunk));
    }
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(path, text)
}

pub fn load_backup(path: &Path) -> Result<Vec<u8>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut image = vec![0xffu8; IMAGE_LEN];
    for line in text.lines() {
        let Some((addr, rest)) = line.split_once(':') else { continue };
        let Ok(addr) = usize::from_str_radix(addr.trim(), 16) else { continue };
        let bytes: Vec<u8> = rest
            .split_whitespace()
            .take_while(|w| w.len() == 2 && w.chars().all(|c| c.is_ascii_hexdigit()))
            .map(|w| u8::from_str_radix(w, 16).unwrap())
            .collect();
        for (i, b) in bytes.iter().enumerate() {
            if addr + i < IMAGE_LEN {
                image[addr + i] = *b;
            }
        }
    }
    Ok(image)
}

pub fn read_image(mouse: &mut Mouse, base: u16) -> Result<Vec<u8>, String> {
    mouse.read(base, IMAGE_LEN).map_err(|e| e.to_string())
}

/// Read the current settings, keep a backup of them, then write `patches` and verify each one.
/// Returns log lines describing what was done.
pub fn apply(mouse: &mut Mouse, o: &Opts, patches: Vec<Patch>) -> Result<Vec<String>, String> {
    let before = read_image(mouse, o.base)?;
    let cfg = Config::parse(&before);
    if !cfg.problems.is_empty() && !o.force {
        return Err(format!(
            "settings at {:#06x} look wrong, refusing to write (--force to override):\n  {}",
            o.base,
            cfg.problems.join("\n  ")
        ));
    }
    let mut log = Vec::new();
    if o.backup {
        let backup = config_dir().join(format!("auto-{}.txt", stamp()));
        save_backup(&backup, o.base, &before).map_err(|e| e.to_string())?;
        log.push(format!("backup: {}", backup.display()));
    }
    for p in patches {
        let addr = o.base as usize + p.addr;
        let old = before.get(p.addr..p.addr + p.bytes.len()).map_or("(outside the settings area)".to_string(), hex);
        log.push(format!("write {addr:#06x}: {old} -> {}", hex(&p.bytes)));
        let reply = mouse.write(addr as u16, &p.bytes).map_err(|e| e.to_string())?;
        log.push(format!("  reply {}", hex(&reply)));
    }
    log.push("verified by read-back".into());
    Ok(log)
}
