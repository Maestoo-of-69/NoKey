//! Facts about the foreground window, read only when a binding fires
//! (no background monitoring).

use crate::config::Foreground;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
};

fn process_name(hwnd: HWND) -> String {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return String::new(); // e.g. an elevated process
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        crate::config::file_name(&path)
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// True when the window covers its whole monitor (games, videos,
/// presentations). The desktop itself does not count.
fn is_fullscreen(hwnd: HWND) -> bool {
    if matches!(class_name(hwnd).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
        return false;
    }
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    unsafe {
        if GetWindowRect(hwnd, &mut rc) == 0 {
            return false;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut mi) == 0 {
            return false;
        }
    }
    let m = mi.rcMonitor;
    rc.left <= m.left && rc.top <= m.top && rc.right >= m.right && rc.bottom >= m.bottom
}

pub fn foreground() -> Foreground {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return Foreground::default();
    }
    Foreground { process: process_name(hwnd), title: super::get_text(hwnd), fullscreen: is_fullscreen(hwnd) }
}
