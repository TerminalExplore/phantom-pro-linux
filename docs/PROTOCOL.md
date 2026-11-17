# Protocol notes

What is known about the configuration protocol of the Compx-based mice (tested on the Ardor Gaming Phantom
Pro V2, ARDW-PH3395-BK). Everything here was worked out for interoperability, from the device's behaviour
and from reading how the vendor's Windows tool talks to it. Items marked **(verified)** were confirmed on
real hardware; the rest are inferred and may be wrong.

## Transport

| Link | USB id | Notes |
|---|---|---|
| 2.4G dongle | `3554:f52d` | the mouse talks to the dongle over a proprietary 2.4 GHz radio protocol, not Bluetooth |
| mouse on its USB cable | `3554:f52e` | direct; the dongle then reports the mouse as "not reachable" |

Both expose three HID interfaces (keyboard, vendor channel, mouse). The configuration channel is
**interface 1**: input and output report id `0x08`, 16 data bytes (17 with the id). On Linux that is one
`/dev/hidraw*` node; `write()` sends a frame, `read()` returns the reply. Access needs a udev rule, for example

```
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="3554", ATTRS{idProduct}=="f52d|f52e", ENV{ID_USB_INTERFACE_NUM}=="01", TAG+="uaccess"
```

## Frame

```
08 <cmd> 00 <addr_hi> <addr_lo> <len> <data...> <chk>        17 bytes
chk = 0x55 - (sum of the first 16 bytes) mod 256
```

The reply carries the same command; for reads and writes it echoes address and length, data starts at byte 6.
Two processes talking at once steal each other's replies, so serialise access (the tool uses `flock`).

| cmd | name | notes |
|---|---|---|
| `01` | encryption / info | not used |
| `03` | online | reply byte 6 = 1 when the mouse itself answers (**verified**) |
| `04` | battery | byte 6 = percent, byte 7 = on cable, bytes 8–9 = millivolts big endian (**verified**) |
| `07` | write flash | address, length ≤ 10, data (**verified**) |
| `08` | read flash | address, length ≤ 10 (**verified**) |
| `09` | clear settings | **factory reset, never sent by this tool** |
| `0a` | status changed | event pushed by the mouse |
| `0d` | enter USB update mode | **firmware update, never sent by this tool** |
| `0e` / `0f` | get / set current profile | not used yet |
| `11` | enter MTK mode | **never sent** |

Unlisted ids exist (pairing, dongle RGB, long-range mode); they are not used.

## Memory

Reads work for the whole range (`0x0000`–`0x1fff` readable). Writes to the settings area at `0x0000` take
effect immediately and **survive switching the mouse off** (**verified**). A second copy at `0x1b00` is not
updated by writes and differs from the live one; it is not where settings are stored.

Single values are stored as a pair `(v, 0x55 - v)`, three-byte values as a group `(a, b, c, 0x55 - a - b - c)`.

| Address | Content |
|---|---|
| `0x00` | pairs: report rate mask, DPI stage count, current stage (0-based), x spin-down, y spin-down, lift-off |
| `0x0c` | 8 DPI stages, 4-byte groups `(x, y, ex, chk)`; `x = y = DPI / 50 - 1` for the PAW3395 up to 12800 DPI; `ex` is not decoded (needed above that) |
| `0x2c` | 8 stage colours, groups `(R, G, B, chk)` |
| `0x4c` | pairs: DPI light mode (0 off, 1 steady, 2 breathing), brightness, speed, enable |
| `0x60` | 16 key assignments, groups `(class, value, extra, chk)` |
| `0xa0` | body light block: `mode, R, G, B, speed, brightness, chk` where `chk = 0x55 - sum(6 bytes)` |
| `0xa7` | pair: body light enable |
| `0xa9` … `0xb9` | pairs: key debounce (ms), motion sync, lights-off idle time index, angle snapping, ripple control, lights off while moving, custom sensor sleep, sensor sleep time, sensor power saving |
| `0x0300 + 0x180 * key` | macro slot of key `key` (0-based), see below |

Report rate mask: `1` = 1000 Hz, `2` = 500, `4` = 250, `8` = 125 (`0x10` 2000, `0x20` 4000 need the 4K dongle).
Body light modes `0`…`6`: off, rainbow, single colour, fixed rainbow, neon, breathing rainbow, colourful;
speed and brightness `0`…`9`. Idle time index into 10 s, 30 s, 1, 5, 10, 15, 20, 25, 30, 35, 40 min.

Key classes: `0` off, `1` mouse button (value is the HID button mask: `1` left, `2` right, `4` middle, `8` back,
`16` forward), `2` DPI function, `3` wheel/axis, `4` fire key, `5` shortcut, `6` macro, `7` report rate,
`8` lamp, `9` profile, `10` DPI lock. Only keys 1–6 exist physically (left, right, middle, back, forward, DPI).
Slots 7–16 are firmware slots with no button; 9–12 carry factory assignments of unknown purpose.

### Macros

One slot of 0x180 bytes per key: `[0]` name length 1–30, `[1..=30]` name, `[0x1f]` action count 2–70, then 5
bytes per action from `0x20`, then `chk = 0x55 - sum(bytes 0x1f .. 0x20 + 5n)`.
An action is `[state<<6 | kind, value0, value1, delay_hi, delay_lo]`; state `0b10` pressed, `0b01` released.
Kinds: `0` modifier (mask `1` ctrl, `2` shift, `4` alt, `8` win — **assumed**), `1` keyboard key with the HID
usage in `value0` (**verified**), `2` media, `3` power, `4` mouse button (**assumed**), `5` pointer move.
The key points at its macro with `(class 6, key index, repeat)`; repeat `0` means "while the key is held".

The macro area only accepts blocks that form a valid macro: writes of all `0xff` or all `0x00` are silently
ignored. A macro can therefore be replaced but not erased (**verified**); disabling the key is enough.

## Not known

Profile switching (`0f`), the meaning of the lift-off values, the bit layout of `ex` above 12800 DPI,
pairing, calibration, and the factory assignments of key slots 9–12.
