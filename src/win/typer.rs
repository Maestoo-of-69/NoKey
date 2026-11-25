//! The real keyboard for "Type text" (`actions::typing::Keyboard`):
//! SendInput with KEYEVENTF_UNICODE, polled modifier / Esc state and the
//! foreground window. No hook: Esc is read with GetAsyncKeyState.

use crate::actions::typing::plan::Stroke;
use crate::actions::typing::{Keyboard, TypeFail};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, HANDLE, HWND};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenElevation, TokenIntegrityLevel,
    TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// `dwExtraInfo` of every key NoKey types, so its own keyboard hook can
/// ignore them (a typed Enter must never trigger a binding).
pub const TYPED_TAG: usize = 0x4E4B_5459; // "NKTY"

const MODIFIERS: [VIRTUAL_KEY; 5] = [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN];

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

/// Esc is down now, or was pressed since the last check (a quick tap
/// between two polls still counts).
fn escape_pressed() -> bool {
    let state = unsafe { GetAsyncKeyState(VK_ESCAPE as i32) };
    state < 0 || state & 1 != 0
}

fn input(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: TYPED_TAG } },
    }
}

/// Key-down events then key-up events for one stroke (ups mirror downs).
fn inputs(stroke: Stroke) -> Vec<INPUT> {
    match stroke {
        Stroke::Enter => vec![input(VK_RETURN, 0, 0), input(VK_RETURN, 0, KEYEVENTF_KEYUP)],
        Stroke::Tab => vec![input(VK_TAB, 0, 0), input(VK_TAB, 0, KEYEVENTF_KEYUP)],
        Stroke::Backspace => vec![input(VK_BACK, 0, 0), input(VK_BACK, 0, KEYEVENTF_KEYUP)],
        Stroke::Unicode { units, len } => {
            let down = units[..len].iter().map(|&u| input(0, u, KEYEVENTF_UNICODE));
            let up = units[..len].iter().map(|&u| input(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
            down.chain(up).collect()
        }
    }
}

/// After only `sent` of `list` went in: the key-ups still owed for keys
/// that went down, so nothing is left stuck (Enter/Tab especially).
fn owed_key_ups(list: &[INPUT], sent: usize) -> Vec<INPUT> {
    let half = list.len() / 2;
    let downs_in = sent.min(half);
    let ups_in = sent.saturating_sub(half);
    (ups_in..downs_in).map(|i| list[half + i]).collect()
}

/// `owed_key_ups` for a batch: finds the stroke the send stopped in
/// (earlier strokes went in completely, later ones not at all).
fn owed_in_batch(strokes: &[Vec<INPUT>], mut sent: usize) -> Vec<INPUT> {
    for list in strokes {
        if sent < list.len() {
            return owed_key_ups(list, sent);
        }
        sent -= list.len();
    }
    Vec::new()
}

/// Identifies the window in front (0 when there is none).
pub fn foreground_id() -> isize {
    unsafe { GetForegroundWindow() as isize }
}

// ---------- UIPI check ----------

/// What opening the foreground process (and its token) gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Opened { integrity: Option<u32>, elevated: Option<bool> },
    /// OpenProcess or OpenProcessToken said ERROR_ACCESS_DENIED.
    AccessDenied,
    /// Any other failure (e.g. the process just exited).
    Failed,
}

/// True when Windows (UIPI) would drop our input to the target. Fails
/// closed: a non-elevated NoKey that may not even look at the target
/// assumes it is elevated. The SendInput count check stays as a backstop.
pub fn blocked_by_uipi(our_elevated: bool, our_integrity: Option<u32>, target: Probe) -> bool {
    match target {
        Probe::AccessDenied => !our_elevated,
        Probe::Failed => false,
        Probe::Opened { integrity: Some(theirs), .. } if our_integrity.is_some() => Some(theirs) > our_integrity,
        Probe::Opened { elevated, .. } => !our_elevated && elevated == Some(true),
    }
}

fn token_info(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Option<Vec<u64>> {
    let mut len = 0u32;
    unsafe { GetTokenInformation(token, class, null_mut(), 0, &mut len) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)]; // 8-byte aligned
    let ok = unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast(), len, &mut len) };
    (ok != 0).then_some(buf)
}

fn token_elevated(token: HANDLE) -> Option<bool> {
    let buf = token_info(token, TokenElevation)?;
    Some(unsafe { (*(buf.as_ptr() as *const TOKEN_ELEVATION)).TokenIsElevated } != 0)
}

/// Mandatory integrity level (the last sub-authority of the label SID).
fn token_integrity(token: HANDLE) -> Option<u32> {
    let buf = token_info(token, TokenIntegrityLevel)?;
    unsafe {
        let sid = (*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid;
        let count = GetSidSubAuthorityCount(sid);
        if sid.is_null() || count.is_null() || *count == 0 {
            return None;
        }
        Some(*GetSidSubAuthority(sid, *count as u32 - 1))
    }
}

fn denied_or_failed() -> Probe {
    if unsafe { GetLastError() } == ERROR_ACCESS_DENIED { Probe::AccessDenied } else { Probe::Failed }
}

/// Opens a process token and reads it; the handle is closed here.
fn probe_process(process: HANDLE) -> Probe {
    let mut token: HANDLE = null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return denied_or_failed();
    }
    let probe = Probe::Opened { integrity: token_integrity(token), elevated: token_elevated(token) };
    unsafe { CloseHandle(token) };
    probe
}

fn probe_pid(pid: u32) -> Probe {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return denied_or_failed();
    }
    let probe = probe_process(handle);
    unsafe { CloseHandle(handle) };
    probe
}

fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

// ---------- the keyboard ----------

pub struct WinKeyboard<'a> {
    cancel: &'a AtomicBool,
    target: HWND,
    start: Instant,
}

impl<'a> WinKeyboard<'a> {
    pub fn new(cancel: &'a AtomicBool) -> Self {
        Self { cancel, target: null_mut(), start: Instant::now() }
    }
}

impl Keyboard for WinKeyboard<'_> {
    fn send(&mut self, strokes: &[Stroke]) -> bool {
        let each: Vec<Vec<INPUT>> = strokes.iter().map(|&s| inputs(s)).collect();
        let list: Vec<INPUT> = each.iter().flatten().copied().collect();
        let size = std::mem::size_of::<INPUT>() as i32;
        let sent = unsafe { SendInput(list.len() as u32, list.as_ptr(), size) } as usize;
        if sent < list.len() {
            let ups = owed_in_batch(&each, sent);
            if !ups.is_empty() {
                unsafe { SendInput(ups.len() as u32, ups.as_ptr(), size) };
            }
        }
        sent == list.len()
    }

    fn stop_requested(&mut self) -> bool {
        self.cancel.load(Ordering::Acquire) || escape_pressed()
    }

    fn modifiers_held(&mut self) -> bool {
        MODIFIERS.iter().any(|&vk| held(vk))
    }

    fn begin(&mut self) -> Result<(), TypeFail> {
        let fg = unsafe { GetForegroundWindow() };
        if fg.is_null() {
            return Err(TypeFail::NoWindow); // e.g. a lock screen or a window switch in progress
        }
        let pid = window_pid(fg);
        if pid == unsafe { GetCurrentProcessId() } {
            return Err(TypeFail::OwnWindow);
        }
        // UIPI silently drops input to a higher-integrity window, so check first.
        let (ours, our_integrity) = match probe_process(unsafe { GetCurrentProcess() }) {
            Probe::Opened { integrity, elevated } => (elevated.unwrap_or(false), integrity),
            _ => (false, None),
        };
        if blocked_by_uipi(ours, our_integrity, probe_pid(pid)) {
            return Err(TypeFail::Elevated);
        }
        self.target = fg;
        escape_pressed(); // drop a stale "pressed since last call" bit
        Ok(())
    }

    fn same_window(&mut self) -> bool {
        unsafe { GetForegroundWindow() == self.target }
    }

    fn target_window(&self) -> isize {
        self.target as isize
    }

    fn now_ms(&mut self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn sleep(&mut self, ms: u32) {
        std::thread::sleep(Duration::from_millis(ms as u64));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEDIUM: u32 = 0x2000;
    const HIGH: u32 = 0x3000;

    fn ki(list: &[INPUT]) -> Vec<KEYBDINPUT> {
        list.iter().map(|i| unsafe { i.Anonymous.ki }).collect()
    }

    #[test]
    fn strokes_become_tagged_down_up_pairs() {
        let emoji = ki(&inputs(Stroke::Unicode { units: [0xD83D, 0xDC4B], len: 2 }));
        assert_eq!(emoji.len(), 4, "two units down, two units up");
        assert_eq!((emoji[0].wScan, emoji[1].wScan), (0xD83D, 0xDC4B));
        assert_eq!(emoji[0].dwFlags, KEYEVENTF_UNICODE);
        assert_eq!(emoji[3].dwFlags, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);
        assert!(emoji.iter().all(|k| k.dwExtraInfo == TYPED_TAG && k.wVk == 0));
        let enter = ki(&inputs(Stroke::Enter));
        assert_eq!((enter[0].wVk, enter[1].dwFlags), (VK_RETURN, KEYEVENTF_KEYUP));
    }

    #[test]
    fn a_short_send_releases_what_went_down() {
        let enter = inputs(Stroke::Enter);
        let owed = ki(&owed_key_ups(&enter, 1));
        assert_eq!(owed.len(), 1);
        assert_eq!((owed[0].wVk, owed[0].dwFlags), (VK_RETURN, KEYEVENTF_KEYUP));
        assert!(owed_key_ups(&enter, 0).is_empty(), "nothing went down");
        assert!(owed_key_ups(&enter, 2).is_empty(), "complete");
        let pair = inputs(Stroke::Unicode { units: [0xD83D, 0xDC4B], len: 2 });
        let owed = ki(&owed_key_ups(&pair, 3));
        assert_eq!(owed.len(), 1, "one of two ups already went in");
        assert_eq!(owed[0].wScan, 0xDC4B);
    }

    #[test]
    fn a_short_batch_releases_only_the_cut_stroke() {
        let batch = [inputs(Stroke::Backspace), inputs(Stroke::Enter), inputs(Stroke::Tab)];
        let owed = ki(&owed_in_batch(&batch, 3));
        assert_eq!(owed.len(), 1);
        assert_eq!((owed[0].wVk, owed[0].dwFlags), (VK_RETURN, KEYEVENTF_KEYUP));
        assert!(owed_in_batch(&batch, 0).is_empty());
        assert!(owed_in_batch(&batch, 4).is_empty(), "stopped between strokes");
        assert!(owed_in_batch(&batch, 6).is_empty(), "complete");
        assert_eq!(ki(&batch[0])[0].wVk, VK_BACK);
        assert!(ki(&batch[0]).iter().all(|k| k.dwExtraInfo == TYPED_TAG));
    }

    #[test]
    fn uipi_check_fails_closed() {
        let opened = |integrity, elevated| Probe::Opened { integrity, elevated };
        // Not elevated and not allowed to look: assume the target is elevated.
        assert!(blocked_by_uipi(false, Some(MEDIUM), Probe::AccessDenied));
        assert!(!blocked_by_uipi(true, Some(HIGH), Probe::AccessDenied));
        // Integrity levels decide when both are known.
        assert!(blocked_by_uipi(false, Some(MEDIUM), opened(Some(HIGH), Some(true))));
        assert!(!blocked_by_uipi(false, Some(MEDIUM), opened(Some(MEDIUM), Some(false))));
        assert!(!blocked_by_uipi(true, Some(HIGH), opened(Some(HIGH), Some(true))));
        assert!(blocked_by_uipi(true, Some(HIGH), opened(Some(0x4000), None)), "system integrity");
        // Fallback on the elevation flag.
        assert!(blocked_by_uipi(false, None, opened(None, Some(true))));
        assert!(!blocked_by_uipi(false, None, opened(None, Some(false))));
        // Unknown failures don't block: the SendInput count still catches UIPI.
        assert!(!blocked_by_uipi(false, Some(MEDIUM), Probe::Failed));
    }
}
