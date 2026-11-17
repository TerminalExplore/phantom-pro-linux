//! Transport for the Compx dongle's vendor HID channel (USB interface 1, report 0x08, 17 bytes).
//!
//! Frame: `08 <cmd> 00 <addr_hi> <addr_lo> <len> <data..> <chk>`, `chk = 0x55 - sum(bytes 0..16)`.
//! Layout taken from the vendor driver (HIDUsb.dll / DriverLib.UsbCommandID).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const FRAME: usize = 17;
const REPORT_ID: u8 = 0x08;
const DONGLE_ID: &str = "HID_ID=0003:00003554:0000F52D";
const WIRED_ID: &str = "HID_ID=0003:00003554:0000F52E";

pub const CMD_STATUS: u8 = 0x03;
pub const CMD_POWER: u8 = 0x04;
pub const CMD_WRITE: u8 = 0x07;
pub const CMD_READ: u8 = 0x08;

/// Commands this tool may send. Deliberately absent: 0x09 ClearSetting (factory reset),
/// 0x0d EnterUsbUpdateMode and 0x11 EnterMTKMode (firmware), pairing and descriptor changes.
const ALLOWED: [u8; 4] = [CMD_STATUS, CMD_POWER, CMD_WRITE, CMD_READ];
/// Payload bytes per read/write frame.
pub const CHUNK: usize = 10;

pub type Frame = [u8; FRAME];

pub fn checksum(frame: &[u8]) -> u8 {
    0x55u8.wrapping_sub(frame[..FRAME - 1].iter().fold(0u8, |a, b| a.wrapping_add(*b)))
}

pub struct Battery {
    pub percent: u8,
    pub on_cable: bool,
    pub millivolts: u16,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Link {
    Wired,
    Dongle,
}

pub struct Mouse {
    file: File,
    pub node: PathBuf,
    pub link: Link,
}

/// Vendor interface (interface 1) of the mouse on its cable, or else of the 2.4G dongle.
fn find_node() -> io::Result<(PathBuf, Link)> {
    let mut dongle = None;
    for entry in fs::read_dir("/sys/class/hidraw")? {
        let dir = entry?.path();
        let Ok(uevent) = fs::read_to_string(dir.join("device/uevent")) else { continue };
        let link = if uevent.contains(WIRED_ID) {
            Link::Wired
        } else if uevent.contains(DONGLE_ID) {
            Link::Dongle
        } else {
            continue;
        };
        let real = fs::canonicalize(dir.join("device"))?;
        if !real.to_string_lossy().contains(":1.1/") {
            continue;
        }
        let node = PathBuf::from("/dev").join(dir.file_name().unwrap());
        match link {
            Link::Wired => return Ok((node, link)),
            Link::Dongle => dongle = Some((node, link)),
        }
    }
    dongle.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "Compx mouse (3554:f52e) or dongle (3554:f52d) not found")
    })
}

fn timeout_err(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("no reply to {what}"))
}

impl Mouse {
    pub fn open() -> io::Result<Self> {
        let (node, link) = find_node()?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&node)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", node.display())))?;
        Ok(Mouse { file, node, link })
    }

    fn wait_readable(&self, timeout: Duration) -> bool {
        let mut p = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        unsafe { libc::poll(&mut p, 1, ms) > 0 }
    }

    fn drain(&mut self) {
        let mut buf = [0u8; 64];
        while self.wait_readable(Duration::ZERO) {
            if self.file.read(&mut buf).unwrap_or(0) == 0 {
                break;
            }
        }
    }

    /// Send one frame and wait for the reply carrying the same command and a valid checksum.
    /// An advisory lock keeps two processes (say the GUI and the CLI) from stealing each other's replies.
    fn exchange(&mut self, cmd: u8, frame: &Frame) -> io::Result<Frame> {
        assert!(ALLOWED.contains(&cmd), "command {cmd:#04x} is not allowed");
        assert_eq!(frame[1], cmd);
        let fd = self.file.as_raw_fd();
        unsafe { libc::flock(fd, libc::LOCK_EX) };
        let result = self.exchange_locked(cmd, frame);
        unsafe { libc::flock(fd, libc::LOCK_UN) };
        result
    }

    fn exchange_locked(&mut self, cmd: u8, frame: &Frame) -> io::Result<Frame> {
        self.drain();
        self.file.write_all(frame)?;
        let end = Instant::now() + Duration::from_millis(1000);
        let mut buf = [0u8; 64];
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if !self.wait_readable(left) {
                break;
            }
            let n = self.file.read(&mut buf)?;
            if n == FRAME && buf[0] == REPORT_ID && buf[1] == cmd && checksum(&buf[..n]) == buf[FRAME - 1] {
                let mut out = [0u8; FRAME];
                out.copy_from_slice(&buf[..FRAME]);
                return Ok(out);
            }
        }
        Err(timeout_err(&format!("command {cmd:#04x}")))
    }

    fn frame(cmd: u8, addr: u16, len: u8, data: &[u8]) -> Frame {
        let mut f = [0u8; FRAME];
        f[0] = REPORT_ID;
        f[1] = cmd;
        f[3] = (addr >> 8) as u8;
        f[4] = addr as u8;
        f[5] = len;
        f[6..6 + data.len()].copy_from_slice(data);
        f[FRAME - 1] = checksum(&f);
        f
    }

    /// Whether the mouse itself answers through the dongle (it does not while switched off or asleep).
    pub fn online(&mut self) -> io::Result<bool> {
        let r = self.exchange(CMD_STATUS, &Self::frame(CMD_STATUS, 0, 0, &[]))?;
        Ok(r[6] == 1)
    }

    pub fn battery(&mut self) -> io::Result<Battery> {
        let r = self.exchange(CMD_POWER, &Self::frame(CMD_POWER, 0, 0, &[]))?;
        Ok(Battery { percent: r[6], on_cable: r[7] == 1, millivolts: u16::from_be_bytes([r[8], r[9]]) })
    }

    pub fn read(&mut self, addr: u16, len: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        let mut done = 0;
        while done < len {
            let n = (len - done).min(CHUNK);
            let a = addr + done as u16;
            let r = self.exchange(CMD_READ, &Self::frame(CMD_READ, a, n as u8, &[]))?;
            if u16::from_be_bytes([r[3], r[4]]) != a || r[5] as usize != n {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("bad read reply at {a:#06x}")));
            }
            out.extend_from_slice(&r[6..6 + n]);
            done += n;
        }
        Ok(out)
    }

    /// Write `data` at `addr` and verify it by reading it back. Returns the raw reply of the first frame.
    pub fn write(&mut self, addr: u16, data: &[u8]) -> io::Result<Frame> {
        let mut first = None;
        for (i, chunk) in data.chunks(CHUNK).enumerate() {
            let a = addr + (i * CHUNK) as u16;
            let r = self.exchange(CMD_WRITE, &Self::frame(CMD_WRITE, a, chunk.len() as u8, chunk))?;
            first.get_or_insert(r);
        }
        let back = self.read(addr, data.len())?;
        if back != data {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("verify failed at {addr:#06x}: wrote {data:02x?}, read back {back:02x?}"),
            ));
        }
        first.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "nothing to write"))
    }
}
