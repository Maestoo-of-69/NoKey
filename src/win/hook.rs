//! Low-level keyboard hook (WH_KEYBOARD_LL) for combos and sequences, and
//! for recording shortcuts. Installed only while it is needed.
//!
//! The hook runs on the UI thread (it has a message loop) and must return
//! quickly, so it only updates in-memory state and posts a message.
//! Keystrokes are never stored or logged.

use super::{tick, PostTarget, WM_APP_FIRE, WM_APP_RECORD};
use crate::engine::matcher::{Event, Matcher};
use crate::engine::recorder::{RecordUpdate, Recorder};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

struct HookState {
    hhook: HHOOK,
    target: HWND,
    matcher: Option<Matcher>,
    recorder: Option<Recorder>,
    /// Keys pressed during recording. Only their key-ups are swallowed, so a
    /// key held before recording started is still released normally.
    recorded_down: Vec<u16>,
    /// Recorder results waiting for the window (WM_APP_RECORD is only a wake-up).
    updates: VecDeque<RecordUpdate>,
}

thread_local! {
    static STATE: RefCell<HookState> = const {
        RefCell::new(HookState {
            hhook: null_mut(),
            target: null_mut(),
            matcher: None,
            recorder: None,
            recorded_down: Vec::new(),
            updates: VecDeque::new(),
        })
    };
}

fn is_held(vk: u16) -> bool {
    let state = unsafe { GetAsyncKeyState(vk as i32) };
    state < 0 // high bit set = key is down
}

/// Returns true when the event must not reach other apps.
fn handle(st: &mut HookState, vk: u16, is_down: bool) -> bool {
    let target = PostTarget::new(st.target);
    if let Some(rec) = st.recorder.as_mut() {
        if is_down {
            if !st.recorded_down.contains(&vk) {
                st.recorded_down.push(vk);
            }
        } else if let Some(i) = st.recorded_down.iter().position(|&k| k == vk) {
            st.recorded_down.swap_remove(i);
        } else {
            return false; // key-up of a key pressed before recording
        }
        if let Some(update) = rec.key_event(vk, is_down) {
            st.updates.push_back(update);
            target.post(WM_APP_RECORD, 0);
        }
        return true;
    }
    if let Some(m) = st.matcher.as_mut() {
        let mut events = if is_down { m.forget_released(is_held) } else { Vec::new() };
        let hit = m.key_event(vk, is_down, tick());
        events.extend(hit.events);
        for e in events {
            // lparam: 1 = shortcut went down, 0 = released
            match e {
                Event::Down(id) => target.post_with(WM_APP_FIRE, id as WPARAM, 1),
                Event::Up(id) => target.post_with(WM_APP_FIRE, id as WPARAM, 0),
            }
        }
        return hit.swallow;
    }
    false
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let msg = wparam as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        if (is_down || is_up) && info.vkCode < 0xFF {
            // try_borrow: never panic if a message is pumped while state is borrowed
            let swallow =
                STATE.with(|s| s.try_borrow_mut().map(|mut st| handle(&mut st, info.vkCode as u16, is_down)).unwrap_or(false));
            if swallow {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

fn refresh(st: &mut HookState) -> Result<(), String> {
    let needed = st.recorder.is_some() || st.matcher.as_ref().is_some_and(|m| !m.is_empty());
    if needed && st.hhook.is_null() {
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        let h = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0) };
        if h.is_null() {
            return Err(format!("Windows refused the keyboard hook (error {}).", super::last_error()));
        }
        st.hhook = h;
    } else if !needed && !st.hhook.is_null() {
        unsafe { UnhookWindowsHookEx(st.hhook) };
        st.hhook = null_mut();
    }
    Ok(())
}

pub fn set_target(hwnd: HWND) {
    STATE.with(|s| s.borrow_mut().target = hwnd);
}

/// Replaces the matcher. `None` (or an empty matcher) removes the hook.
pub fn set_matcher(matcher: Option<Matcher>) -> Result<(), String> {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.matcher = matcher;
        refresh(&mut st)
    })
}

pub fn start_recording(recorder: Recorder) -> Result<(), String> {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.recorder = Some(recorder);
        st.recorded_down.clear();
        st.updates.clear();
        refresh(&mut st)
    })
}

/// Stops recording. Returns the final result for sequences.
pub fn stop_recording() -> Option<RecordUpdate> {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let update = st.recorder.take().map(|mut r| r.finish());
        st.recorded_down.clear();
        st.updates.clear();
        let _ = refresh(&mut st);
        update
    })
}

/// Recorder results posted since the last call (on WM_APP_RECORD).
pub fn take_record_updates() -> Vec<RecordUpdate> {
    STATE.with(|s| s.borrow_mut().updates.drain(..).collect())
}

pub fn is_installed() -> bool {
    STATE.with(|s| !s.borrow().hhook.is_null())
}

/// Removes the hook on exit.
pub fn shutdown() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.matcher = None;
        st.recorder = None;
        let _ = refresh(&mut st);
    });
}
