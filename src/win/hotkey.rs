//! Global hotkeys through RegisterHotKey: Windows delivers WM_HOTKEY to our
//! window from any foreground app, with no polling.

use crate::shortcut::{key_name, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT as W_ALT, MOD_CONTROL as W_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT as W_SHIFT, MOD_WIN as W_WIN,
};

const ERROR_HOTKEY_ALREADY_REGISTERED: u32 = 1409;

pub fn to_win_mods(mods: u8) -> HOT_KEY_MODIFIERS {
    let mut m = MOD_NOREPEAT; // holding the key fires once, not repeatedly
    if mods & MOD_CTRL != 0 {
        m |= W_CONTROL;
    }
    if mods & MOD_ALT != 0 {
        m |= W_ALT;
    }
    if mods & MOD_SHIFT != 0 {
        m |= W_SHIFT;
    }
    if mods & MOD_WIN != 0 {
        m |= W_WIN;
    }
    m
}

/// Registers a global hotkey. The error text is translated.
pub fn register(hwnd: HWND, id: i32, mods: u8, vk: u16) -> Result<(), String> {
    let ok = unsafe { RegisterHotKey(hwnd, id, to_win_mods(mods), vk as u32) };
    if ok != 0 {
        return Ok(());
    }
    let code = super::last_error();
    if code == ERROR_HOTKEY_ALREADY_REGISTERED {
        Err(crate::i18n::t("err.hotkey_in_use"))
    } else {
        Err(crate::i18n::tf("err.hotkey_failed", &[("code", &code.to_string()), ("key", &key_name(vk))]))
    }
}

pub fn unregister(hwnd: HWND, id: i32) {
    unsafe { UnregisterHotKey(hwnd, id) };
}
