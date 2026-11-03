//! Thin helpers around Win32 and OS integration (hotkeys, hook, tray,
//! autostart, shell). Window classes and drawing live in `ui`.

pub mod context;
pub mod hook;
pub mod hotkey;
pub mod icon;
pub mod shell;
pub mod startup;
pub mod tray;

use std::cell::Cell;
use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Hook -> host window: binding `wparam` went down (`lparam` 1) or up (0).
pub const WM_APP_FIRE: u32 = WM_APP + 1;
/// Worker thread -> main window: results are waiting in `app`'s task queue.
pub const WM_APP_TASK: u32 = WM_APP + 2;
/// Hook -> main window: recorder updates are waiting in the hook's queue.
pub const WM_APP_RECORD: u32 = WM_APP + 3;
/// Tray icon callback.
pub const WM_APP_TRAY: u32 = WM_APP + 4;
/// Second instance -> first instance: show the window.
pub const WM_APP_SHOW: u32 = WM_APP + 5;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

pub fn tick() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}

pub fn get_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

pub fn set_text(hwnd: HWND, text: &str) {
    let w = wide(text);
    unsafe { SetWindowTextW(hwnd, w.as_ptr()) };
}

pub fn message_box(owner: HWND, text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let t = wide(text);
    let c = wide("NoKey");
    unsafe { MessageBoxW(owner, t.as_ptr(), c.as_ptr(), flags) }
}

pub fn error_box(owner: HWND, text: &str) {
    message_box(owner, text, MB_OK | MB_ICONWARNING);
}

/// A window handle that can be moved to a worker thread. Only used to
/// post messages back, which is thread-safe in Win32. Messages never carry
/// pointers: another process could post a forged one.
#[derive(Clone, Copy)]
pub struct PostTarget(usize);

impl PostTarget {
    pub fn new(hwnd: HWND) -> Self {
        Self(hwnd as usize)
    }

    pub fn post(self, msg: u32, wparam: WPARAM) {
        self.post_with(msg, wparam, 0);
    }

    pub fn post_with(self, msg: u32, wparam: WPARAM, lparam: LPARAM) {
        unsafe { PostMessageW(self.0 as HWND, msg, wparam, lparam) };
    }
}

thread_local! {
    static DPI: Cell<u32> = const { Cell::new(0) };
}

/// Scales a 96-DPI layout value to the system DPI.
pub fn px(v: i32) -> i32 {
    let dpi = DPI.with(|d| {
        if d.get() == 0 {
            let sys = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForSystem() };
            d.set(if sys == 0 { 96 } else { sys });
        }
        d.get()
    });
    (v as i64 * dpi as i64 / 96) as i32
}
