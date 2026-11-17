//! Macros as the mouse stores them: one 0x180-byte slot per key at 0x300 + 0x180 * key.
//!
//! Slot layout (from the vendor's HIDUsb.dll parser): `[0]` name length (1..=30), `[1..=30]` name,
//! `[0x1f]` action count (2..=70), then 5 bytes per action starting at `0x20`, then one checksum byte
//! `0x55 - sum(bytes 0x1f .. 0x20 + 5 * count)`. An action is
//! `[state<<6 | kind, value0, value1, delay_hi, delay_lo]` where state `10` = pressed, `01` = released.
//! The key itself points at its macro with `(class 6, key index, repeat count)`.

use crate::config::{set_key_ex, Patch};

pub const SLOT_BASE: usize = 0x300;
pub const SLOT_LEN: usize = 0x180;
pub const MAX_NAME: usize = 30;
pub const MAX_ACTIONS: usize = 70;
pub const DEFAULT_DELAY_MS: u16 = 10;

/// Kinds of action (the vendor's HID_CODE_TYPE).
pub const KIND_MODIFIER: u8 = 0;
pub const KIND_KEY: u8 = 1;
pub const KIND_MEDIA: u8 = 2;
pub const KIND_MOUSE: u8 = 4;
pub const KIND_MOVE: u8 = 5;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Down,
    Up,
    None,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Action {
    pub state: State,
    pub kind: u8,
    pub value: [u8; 2],
    pub delay: u16,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Macro {
    pub name: String,
    pub actions: Vec<Action>,
}

pub fn slot_addr(key: usize) -> usize {
    SLOT_BASE + SLOT_LEN * key
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0x55u8, |a, b| a.wrapping_sub(*b))
}

impl Macro {
    /// Bytes to store in the slot (shorter than the slot: only what the parser reads).
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let name = self.name.as_bytes();
        if name.is_empty() || name.len() > MAX_NAME {
            return Err(format!("macro name must be 1..{MAX_NAME} bytes"));
        }
        if !(2..=MAX_ACTIONS).contains(&self.actions.len()) {
            return Err(format!("a macro needs 2..{MAX_ACTIONS} actions, this one has {}", self.actions.len()));
        }
        let mut out = vec![0u8; 0x20];
        out[0] = name.len() as u8;
        out[1..1 + name.len()].copy_from_slice(name);
        out[0x1f] = self.actions.len() as u8;
        for a in &self.actions {
            let state = match a.state {
                State::Down => 0x80,
                State::Up => 0x40,
                State::None => 0x00,
            };
            out.extend_from_slice(&[
                state | (a.kind & 0x0f),
                a.value[0],
                a.value[1],
                (a.delay >> 8) as u8,
                a.delay as u8,
            ]);
        }
        out.push(checksum(&out[0x1f..]));
        Ok(out)
    }

    /// Parse a slot read from the mouse; `None` when it holds no valid macro.
    pub fn decode(slot: &[u8]) -> Option<Macro> {
        if slot.len() < 0x20 {
            return None;
        }
        let name_len = slot[0] as usize;
        let count = slot[0x1f] as usize;
        if !(1..=MAX_NAME).contains(&name_len) || !(2..=MAX_ACTIONS).contains(&count) {
            return None;
        }
        let end = 0x20 + 5 * count;
        if slot.len() <= end || slot[end] != checksum(&slot[0x1f..end]) {
            return None;
        }
        let actions = slot[0x20..end]
            .chunks(5)
            .map(|c| Action {
                state: match c[0] & 0xc0 {
                    0x80 => State::Down,
                    0x40 => State::Up,
                    _ => State::None,
                },
                kind: c[0] & 0x0f,
                value: [c[1], c[2]],
                delay: u16::from_be_bytes([c[3], c[4]]),
            })
            .collect();
        Some(Macro { name: String::from_utf8_lossy(&slot[1..1 + name_len]).into_owned(), actions })
    }

    /// Patches that store the macro in the slot of `key` and point the key at it.
    pub fn patches(&self, key: usize, repeat: u8) -> Result<Vec<Patch>, String> {
        let bytes = self.encode()?;
        Ok(vec![Patch { addr: slot_addr(key), bytes }, set_key_ex(key, 6, key as u8, repeat)?])
    }
}

const MODIFIERS: [(&str, u8); 4] = [("ctrl", 0x01), ("shift", 0x02), ("alt", 0x04), ("win", 0x08)];
const MOUSE: [(&str, u8); 5] = [("left", 0x01), ("right", 0x02), ("middle", 0x04), ("back", 0x08), ("forward", 0x10)];
const NAMED: [(&str, u8); 12] = [
    ("enter", 40),
    ("esc", 41),
    ("backspace", 42),
    ("tab", 43),
    ("space", 44),
    ("right", 79),
    ("left", 80),
    ("down", 81),
    ("up", 82),
    ("delete", 76),
    ("home", 74),
    ("end", 77),
];

/// `(kind, value)` for a key name: a letter or digit, F1..F12, a named key, a modifier or a mouse button.
fn lookup(name: &str, mouse: bool) -> Result<(u8, u8), String> {
    let n = name.to_lowercase();
    if mouse {
        return MOUSE
            .iter()
            .find(|(k, _)| *k == n)
            .map(|(_, v)| (KIND_MOUSE, *v))
            .ok_or(format!("unknown mouse button {name}"));
    }
    if let Some((_, v)) = MODIFIERS.iter().find(|(k, _)| *k == n) {
        return Ok((KIND_MODIFIER, *v));
    }
    if let Some((_, v)) = NAMED.iter().find(|(k, _)| *k == n) {
        return Ok((KIND_KEY, *v));
    }
    let b = n.as_bytes();
    match b {
        [c @ b'a'..=b'z'] => Ok((KIND_KEY, 4 + (c - b'a'))),
        [c @ b'1'..=b'9'] => Ok((KIND_KEY, 30 + (c - b'1'))),
        [b'0'] => Ok((KIND_KEY, 39)),
        [b'f', rest @ ..] => match std::str::from_utf8(rest).ok().and_then(|s| s.parse::<u8>().ok()) {
            Some(f @ 1..=12) => Ok((KIND_KEY, 57 + f)),
            _ => Err(format!("unknown key {name}")),
        },
        _ => Err(format!("unknown key {name}")),
    }
}

/// `(kind, value)` for a key as named by the GUI (GPUI's layout-independent key names).
pub fn key_value(name: &str) -> Result<(u8, u8), String> {
    lookup(if name == "escape" { "esc" } else { name }, false)
}

pub fn make_action(state: State, kind: u8, value: u8, delay: u16) -> Action {
    Action { state, kind, value: [value, 0], delay }
}

fn action(state: State, kind: u8, value: u8) -> Action {
    Action { state, kind, value: [value, 0], delay: DEFAULT_DELAY_MS }
}

/// Parse a script such as `tap:h tap:i down:shift tap:1 up:shift click:left wait:50`.
/// `tap:K` presses and releases, `down:`/`up:` act alone, `click:B` taps a mouse button,
/// `wait:MS` sets the pause after the previous action (default 10 ms).
pub fn parse_script(name: &str, script: &str) -> Result<Macro, String> {
    let mut actions: Vec<Action> = Vec::new();
    for tok in script.split_whitespace() {
        let (verb, arg) = tok.split_once(':').ok_or(format!("expected verb:argument, got {tok}"))?;
        match verb {
            "tap" => {
                let (k, v) = lookup(arg, false)?;
                actions.push(action(State::Down, k, v));
                actions.push(action(State::Up, k, v));
            }
            "down" | "up" => {
                let (k, v) = lookup(arg, false)?;
                actions.push(action(if verb == "down" { State::Down } else { State::Up }, k, v));
            }
            "click" => {
                let (k, v) = lookup(arg, true)?;
                actions.push(action(State::Down, k, v));
                actions.push(action(State::Up, k, v));
            }
            "wait" => {
                let ms: u16 = arg.parse().map_err(|_| format!("bad wait {arg}"))?;
                actions.last_mut().ok_or("wait: needs an action before it")?.delay = ms;
            }
            other => return Err(format!("unknown verb {other}")),
        }
    }
    Ok(Macro { name: name.to_string(), actions })
}

/// What the action presses or does, e.g. "x", "shift", "mouse left".
pub fn action_name(a: &Action) -> String {
    match a.kind {
        KIND_MODIFIER => MODIFIERS
            .iter()
            .find(|(_, v)| *v == a.value[0])
            .map_or(format!("modifier {:#04x}", a.value[0]), |(n, _)| n.to_string()),
        KIND_KEY => match a.value[0] {
            v @ 4..=29 => ((b'a' + (v - 4)) as char).to_string(),
            v @ 30..=38 => ((b'1' + (v - 30)) as char).to_string(),
            39 => "0".into(),
            v @ 58..=69 => format!("f{}", v - 57),
            v => NAMED.iter().find(|(_, c)| *c == v).map_or(format!("key {v:#04x}"), |(n, _)| n.to_string()),
        },
        KIND_MOUSE => MOUSE
            .iter()
            .find(|(_, v)| *v == a.value[0])
            .map_or(format!("button {:#04x}", a.value[0]), |(n, _)| format!("mouse {n}")),
        KIND_MEDIA => format!("media {:02x}{:02x}", a.value[0], a.value[1]),
        KIND_MOVE => format!("move {} {}", a.value[0] as i8, a.value[1] as i8),
        k => format!("type {k} {:02x}{:02x}", a.value[0], a.value[1]),
    }
}

pub fn describe_action(a: &Action) -> String {
    let what = action_name(a);
    let state = match a.state {
        State::Down => "down",
        State::Up => "up",
        State::None => "do",
    };
    format!("{state:<4} {what:<14} wait {} ms", a.delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let m = parse_script("hello", "tap:h tap:i wait:50 click:left").unwrap();
        assert_eq!(m.actions.len(), 6);
        let bytes = m.encode().unwrap();
        let mut slot = bytes.clone();
        slot.resize(SLOT_LEN, 0xff);
        assert_eq!(Macro::decode(&slot), Some(m));
    }

    #[test]
    fn layout_matches_the_vendor_parser() {
        let m = parse_script("ab", "down:a up:a").unwrap();
        let b = m.encode().unwrap();
        assert_eq!(b[0], 2);
        assert_eq!(&b[1..3], b"ab");
        assert_eq!(b[0x1f], 2);
        // first action: pressed (0b10), kind 1 (normal key), HID usage of "a" = 4, delay 10 ms
        assert_eq!(&b[0x20..0x25], &[0x81, 4, 0, 0, 10]);
        assert_eq!(&b[0x25..0x2a], &[0x41, 4, 0, 0, 10]);
        assert_eq!(b.len(), 0x20 + 10 + 1);
        let sum: u8 = b[0x1f..].iter().fold(0u8, |a, x| a.wrapping_add(*x));
        assert_eq!(sum, 0x55);
    }

    #[test]
    fn rejects_bad_macros_and_slots() {
        assert!(parse_script("x", "tap:a").unwrap().encode().is_ok());
        assert!(parse_script("x", "down:a").unwrap().encode().is_err()); // one action only
        assert!(parse_script("", "tap:a").unwrap().encode().is_err());
        assert!(parse_script("x", "tap:nope").is_err());
        assert_eq!(Macro::decode(&[0xff; SLOT_LEN]), None);
        let mut slot = parse_script("x", "tap:a").unwrap().encode().unwrap();
        slot[0x22] ^= 1; // corrupt a value byte
        assert_eq!(Macro::decode(&slot), None);
    }
}
