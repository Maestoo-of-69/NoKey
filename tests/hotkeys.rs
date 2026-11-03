//! Global hotkeys and the keyboard hook, tested with real Windows input.
//!
//! Keystrokes are injected with SendInput using F22–F24 (with Ctrl+Alt+Shift
//! for the standard shortcut). These keys have no default meaning, so the
//! tests do not disturb other apps. The receiving window is a hidden
//! message-only window that never has focus — the same situation as NoKey
//! running in the tray while another app is active.

use nokey::engine::matcher::Matcher;
use nokey::shortcut::{Shortcut, MOD_ALT, MOD_CTRL, MOD_SHIFT};
use nokey::win::{hook, hotkey, wide, WM_APP_FIRE};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const VK_F22: u16 = 0x85;
const VK_F23: u16 = 0x86;
const VK_F24: u16 = 0x87;
const MODS: u8 = MOD_CTRL | MOD_ALT | MOD_SHIFT;

/// Input injection is global, so the tests must not overlap.
static SERIAL: Mutex<()> = Mutex::new(());

fn message_window() -> HWND {
    let class = wide("STATIC");
    let hwnd = unsafe {
        CreateWindowExW(0, class.as_ptr(), null(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), null_mut(), null())
    };
    assert!(!hwnd.is_null(), "message window");
    hwnd
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(keys: &[(u16, bool)]) {
    let inputs: Vec<INPUT> = keys.iter().map(|&(vk, up)| key(vk, up)).collect();
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    assert_eq!(sent as usize, inputs.len(), "SendInput was blocked");
}

/// Pumps messages until `want` arrives or the timeout passes.
fn wait_for(msg_id: u32, wparam: usize, timeout: Duration) -> bool {
    let end = Instant::now() + timeout;
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while Instant::now() < end {
        while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == msg_id && msg.wParam == wparam {
                return true;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn register_detects_conflicts_and_unregister_frees_the_key() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let a = message_window();
    let b = message_window();
    hotkey::register(a, 1, MODS, VK_F22).expect("first registration");
    let err = hotkey::register(b, 1, MODS, VK_F22).expect_err("second registration must fail");
    assert!(err.contains("already used"), "{err}");
    hotkey::unregister(a, 1);
    hotkey::register(b, 1, MODS, VK_F22).expect("free after unregister");
    hotkey::unregister(b, 1);
    unsafe {
        DestroyWindow(a);
        DestroyWindow(b);
    }
}

#[test]
fn hotkey_fires_without_focus_in_hidden_window() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let hwnd = message_window();
    hotkey::register(hwnd, 7, MODS, VK_F24).expect("register");
    send(&[
        (VK_CONTROL, false),
        (VK_MENU, false),
        (VK_SHIFT, false),
        (VK_F24, false),
        (VK_F24, true),
        (VK_SHIFT, true),
        (VK_MENU, true),
        (VK_CONTROL, true),
    ]);
    let fired = wait_for(WM_HOTKEY, 7, Duration::from_secs(2));
    hotkey::unregister(hwnd, 7);
    unsafe { DestroyWindow(hwnd) };
    assert!(fired, "WM_HOTKEY not received");
}

#[test]
fn keyboard_hook_matches_combo_and_is_removed() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let hwnd = message_window();
    hook::set_target(hwnd);
    let mut m = Matcher::new(400, 1000);
    m.add(42, 0, &Shortcut::Combo { keys: vec![VK_F23, VK_F24] });
    hook::set_matcher(Some(m)).expect("install hook");
    assert!(hook::is_installed());

    send(&[(VK_F23, false), (VK_F24, false), (VK_F24, true), (VK_F23, true)]);
    let fired = wait_for(WM_APP_FIRE, 42, Duration::from_secs(2));

    hook::set_matcher(None).unwrap();
    let removed = !hook::is_installed();
    unsafe { DestroyWindow(hwnd) };
    assert!(fired, "combo F23 + F24 not detected by the hook");
    assert!(removed, "hook must be removed when no combos remain");
}

#[test]
fn keyboard_hook_matches_sequence() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let hwnd = message_window();
    hook::set_target(hwnd);
    let mut m = Matcher::new(400, 1500);
    let steps = [VK_F23, VK_F23, VK_F24].map(|vk| nokey::shortcut::Stroke { mods: 0, vk }).to_vec();
    m.add(9, 0, &Shortcut::Sequence { steps });
    hook::set_matcher(Some(m)).expect("install hook");
    for vk in [VK_F23, VK_F23, VK_F24] {
        send(&[(vk, false), (vk, true)]);
        std::thread::sleep(Duration::from_millis(40));
    }
    let fired = wait_for(WM_APP_FIRE, 9, Duration::from_secs(2));
    hook::shutdown();
    unsafe { DestroyWindow(hwnd) };
    assert!(fired, "sequence F23 → F23 → F24 not detected");
}
