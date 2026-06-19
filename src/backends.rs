//! Cursor backends + Win32 input helpers.
//!
//! Protocol to the firmware (matches firmware/mouse_hid.ino), 7-byte packets:
//!   [0xAA][cmd][d0][d1][d2][d3][checksum]   checksum = cmd^d0^d1^d2^d3
use std::io::{Read, Write};
use std::time::Duration;

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEINPUT, MOUSE_EVENT_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
};

#[derive(Clone, Copy)]
pub enum MouseButton {
    Left = 0,
    Right = 1,
    Middle = 2,
}

pub trait MouseBackend {
    fn move_relative(&mut self, dx: i32, dy: i32);
    fn click(&mut self, b: MouseButton);
    fn ok(&self) -> bool;
}

// ---- Win32 input helpers ----------------------------------------------------
fn send_mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

pub fn get_cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

pub fn key_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

pub fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

// ---- Windows SendInput backend ---------------------------------------------
pub struct WindowsMouse;

impl MouseBackend for WindowsMouse {
    fn move_relative(&mut self, dx: i32, dy: i32) {
        if dx != 0 || dy != 0 {
            send_mouse(MOUSEEVENTF_MOVE, dx, dy);
        }
    }
    fn click(&mut self, b: MouseButton) {
        let (down, up) = match b {
            MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        };
        send_mouse(down, 0, 0);
        send_mouse(up, 0, 0);
    }
    fn ok(&self) -> bool {
        true
    }
}

// ---- RP2040 / RP2350 serial backend ----------------------------------------
fn packet(cmd: u8, d0: u8, d1: u8, d2: u8, d3: u8) -> [u8; 7] {
    let chk = cmd ^ d0 ^ d1 ^ d2 ^ d3;
    [0xAA, cmd, d0, d1, d2, d3, chk]
}

pub struct SerialMouse {
    port: Option<Box<dyn serialport::SerialPort>>,
    pub verified: bool,
}

impl SerialMouse {
    pub fn new() -> Self {
        Self {
            port: None,
            verified: false,
        }
    }

    pub fn list_ports() -> Vec<String> {
        serialport::available_ports()
            .map(|ps| ps.into_iter().map(|p| p.port_name).collect())
            .unwrap_or_default()
    }

    /// Returns true if the port opened (verified == firmware answered the ping).
    pub fn connect(&mut self, name: &str) -> bool {
        self.verified = false;
        match serialport::new(name, 115200)
            .timeout(Duration::from_millis(60))
            .open()
        {
            Ok(mut p) => {
                let _ = p.write_all(&packet(b'P', 0, 0, 0, 0));
                let mut buf = [0u8; 1];
                if p.read_exact(&mut buf).is_ok() && buf[0] == b'K' {
                    self.verified = true;
                }
                self.port = Some(p);
                true
            }
            Err(_) => {
                self.port = None;
                false
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.port = None;
        self.verified = false;
    }

    fn write(&mut self, pkt: &[u8; 7]) {
        if let Some(p) = self.port.as_mut() {
            let _ = p.write_all(pkt);
        }
    }
}

impl MouseBackend for SerialMouse {
    fn move_relative(&mut self, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        let x = (dx.clamp(-32768, 32767) as i16) as u16;
        let y = (dy.clamp(-32768, 32767) as i16) as u16;
        let pkt = packet(
            b'M',
            (x & 0xFF) as u8,
            (x >> 8) as u8,
            (y & 0xFF) as u8,
            (y >> 8) as u8,
        );
        self.write(&pkt);
    }
    fn click(&mut self, b: MouseButton) {
        let pkt = packet(b'C', b as u8, 0, 0, 0);
        self.write(&pkt);
    }
    fn ok(&self) -> bool {
        self.port.is_some()
    }
}
