//! System tray icon (Shell_NotifyIcon) and balloon notifications.

use super::WM_APP_TRAY;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::HICON;

const TRAY_ID: u32 = 1;

fn copy_into(dst: &mut [u16], text: &str) {
    let mut i = 0;
    for c in text.encode_utf16() {
        if i + 1 >= dst.len() {
            break;
        }
        dst[i] = c;
        i += 1;
    }
    dst[i] = 0;
}

fn base(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid
}

pub fn add(hwnd: HWND, icon: HICON, tip: &str) -> bool {
    let mut nid = base(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_APP_TRAY;
    nid.hIcon = icon;
    copy_into(&mut nid.szTip, tip);
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) != 0 }
}

pub fn set_tip(hwnd: HWND, tip: &str) {
    let mut nid = base(hwnd);
    nid.uFlags = NIF_TIP;
    copy_into(&mut nid.szTip, tip);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

pub fn notify(hwnd: HWND, title: &str, text: &str, is_error: bool) {
    let mut nid = base(hwnd);
    nid.uFlags = NIF_INFO;
    copy_into(&mut nid.szInfoTitle, title);
    copy_into(&mut nid.szInfo, text);
    nid.dwInfoFlags = NIIF_NOSOUND | if is_error { NIIF_WARNING } else { NIIF_INFO };
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

pub fn remove(hwnd: HWND) {
    let nid = base(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
}
