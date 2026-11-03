//! End-to-end smoke test of NoKey.exe: startup, main window, tray icon,
//! hotkey registration while minimized to the tray, clean shutdown.
//!
//! Runs the real executable with its own data folder (NOKEY_DATA_DIR), so a
//! NoKey copy you already have running is not affected.

use std::fs;
use std::process::Command;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_SHIFT};
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_TIP, NIM_MODIFY, NOTIFYICONDATAW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const VK_F21: u32 = 0x84;
/// `ui::host::EXIT_COMMAND`: the tray menu's Exit, sent to the hidden host window.
const EXIT_COMMAND: usize = 4;
const VK_PAUSE: u32 = 0x13;

struct Found {
    pid: u32,
    class: &'static str,
    hwnd: HWND,
}

unsafe extern "system" fn find_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    let found = unsafe { &mut *(lparam as *mut Found) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    let mut class = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), 64) };
    if pid == found.pid && String::from_utf16_lossy(&class[..n as usize]) == found.class {
        found.hwnd = hwnd;
        return 0;
    }
    1
}

fn window_of(pid: u32, class: &'static str) -> HWND {
    let mut found = Found { pid, class, hwnd: null_mut() };
    unsafe { EnumWindows(Some(find_window), &mut found as *mut Found as LPARAM) };
    found.hwnd
}

fn can_register(mods: u32, vk: u32) -> bool {
    let ok = unsafe { RegisterHotKey(null_mut(), 0x7778, mods, vk) } != 0;
    if ok {
        unsafe { UnregisterHotKey(null_mut(), 0x7778) };
    }
    ok
}

fn tray_icon_exists(hwnd: HWND) -> bool {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_TIP;
    for (i, c) in "NoKey".encode_utf16().enumerate() {
        nid.szTip[i] = c;
    }
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) != 0 }
}

fn can_register_test_hotkey() -> bool {
    let ok = unsafe { RegisterHotKey(null_mut(), 0x7777, MOD_CONTROL | MOD_ALT | MOD_SHIFT, VK_F21) } != 0;
    if ok {
        unsafe { UnregisterHotKey(null_mut(), 0x7777) };
    }
    ok
}

#[test]
fn starts_minimized_to_tray_registers_hotkey_and_exits_cleanly() {
    let dir = std::env::temp_dir().join(format!("nokey-test-app-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // One binding on Ctrl+Alt+Shift+F21 (mods 7 = Ctrl|Alt|Shift, vk 132).
    let config = r#"{ "bindings": [ { "id": 1, "name": "Test", "enabled": true,
        "shortcut": { "type": "standard", "mods": 7, "vk": 132 },
        "action": { "kind": "open_url", "url": "https://example.com" } } ],
        "minimize_to_tray": true, "launch_minimized": true, "notifications": false }"#;
    fs::write(dir.join("config.json"), config).unwrap();
    assert!(can_register_test_hotkey(), "Ctrl+Alt+Shift+F21 is taken by another app; test can't run");
    // A NoKey you already run (or another app) may hold Ctrl+Alt+Pause; then
    // the emergency-switch checks can't tell whose registration they see.
    let pause_free = can_register(MOD_CONTROL | MOD_ALT, VK_PAUSE);
    if !pause_free {
        eprintln!("Ctrl+Alt+Pause is held by another process: emergency switch checks skipped");
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_NoKey")).env("NOKEY_DATA_DIR", &dir).spawn().expect("start NoKey.exe");
    let pid = child.id();

    // 1. Startup and window creation
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut hwnd: HWND = null_mut();
    while hwnd.is_null() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        hwnd = window_of(pid, "NoKeyMain");
    }
    assert!(!hwnd.is_null(), "main window was not created");
    std::thread::sleep(Duration::from_millis(500));

    // 2. Launched minimized: window hidden, tray icon present
    assert_eq!(unsafe { IsWindowVisible(hwnd) }, 0, "launch minimized should start in the tray");
    let host = window_of(pid, "NoKeyHost");
    assert!(!host.is_null(), "hidden host window missing");
    assert!(tray_icon_exists(host), "tray icon missing");

    // 3. The binding's global hotkey and the emergency switch are registered
    //    while the app sits in the tray (config v1 was migrated to v2 on start)
    assert!(!can_register_test_hotkey(), "NoKey did not register Ctrl+Alt+Shift+F21");
    if pause_free {
        assert!(!can_register(MOD_CONTROL | MOD_ALT, VK_PAUSE), "Ctrl+Alt+Pause emergency switch not registered");
    }
    assert!(fs::read_to_string(dir.join("config.json")).unwrap().contains("\"schema\": 2"), "config not migrated");

    // 4. Clean shutdown through the tray menu's Exit command
    unsafe { PostMessageW(host, WM_COMMAND, EXIT_COMMAND, 0) };
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break Some(s);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let status = status.expect("NoKey did not exit within 5 s");
    assert!(status.success(), "exit code {status:?}");

    // 5. Resources released
    assert!(can_register_test_hotkey(), "hotkey still registered after exit");
    if pause_free {
        assert!(can_register(MOD_CONTROL | MOD_ALT, VK_PAUSE), "emergency hotkey still registered after exit");
    }
    assert!(!tray_icon_exists(host), "tray icon left behind after exit");
    let _ = fs::remove_dir_all(dir);
}
