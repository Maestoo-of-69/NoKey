//! Hover and press tracking for owner-drawn buttons and list boxes (comctl32
//! subclasses). These mouse handlers are where the micro-interactions start
//! (docs/DESIGN.md "Motion"); keyboard and programmatic changes never pass
//! through here, so they snap. The subclasses also drive the frame timer of
//! their control (`anim::on_timer`) and forget it on destroy.

use super::super::{anim, is_checked, BTN_LOGO, BTN_SWITCH, BTN_TAB, BTN_TOGGLE, KIND_MASK};
use super::{row_slot, HOVER_MS, RELEASE_MS, ROW_HOVER, ROW_HOVER_MS, ROW_SELECT, SELECT_FLASH_MS, SLOT_HOVER, SLOT_ON, SLOT_PRESS};
use super::{RIPPLE_MS, SLOT_PULSE, SLOT_RIPPLE, SLOT_TINT, TOGGLE_MS};
use std::cell::RefCell;
use std::collections::HashMap;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{FillRect, InvalidateRect, HBRUSH, HDC};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Button userdata bit set while the mouse is over the button.
pub const BTN_HOVER: isize = 0x200;
/// Button userdata: show the run-result tint (`set_tint`).
pub const BTN_TINT: isize = 0x400;
/// Button userdata: the tint is the failure color (kept while fading out).
pub const BTN_TINT_ERR: isize = 0x800;
/// Button userdata: the background pulses (recording).
pub const BTN_PULSE: isize = 0x1000;
/// Userdata bits owned by the kit (kept by `ui::set_kind`).
pub const BTN_KIT_BITS: isize = BTN_HOVER | BTN_TINT | BTN_TINT_ERR | BTN_PULSE;
const BUTTON_SUBCLASS: usize = 0x4E4B_0001;
const LIST_SUBCLASS: usize = 0x4E4B_0002;

thread_local! {
    /// Hovered item index per list box.
    static LIST_HOVER: RefCell<HashMap<usize, i32>> = RefCell::new(HashMap::new());
    /// Item under a held mouse button per list box.
    static LIST_PRESS: RefCell<HashMap<usize, i32>> = RefCell::new(HashMap::new());
    /// Where each button was last clicked (client device pixels).
    static RIPPLES: RefCell<HashMap<usize, (i32, i32)>> = RefCell::new(HashMap::new());
}

/// Centre of the click ripple on `btn`.
pub fn ripple_origin(btn: HWND) -> Option<(i32, i32)> {
    RIPPLES.with(|r| r.borrow().get(&(btn as usize)).copied())
}

fn start_ripple(btn: HWND, lparam: LPARAM) {
    let at = ((lparam & 0xFFFF) as i16 as i32, ((lparam >> 16) & 0xFFFF) as i16 as i32);
    RIPPLES.with(|r| r.borrow_mut().insert(btn as usize, at));
    anim::start(btn, SLOT_RIPPLE, 0.0, 1.0, RIPPLE_MS);
}

fn track_leave(hwnd: HWND) {
    let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
    unsafe { TrackMouseEvent(&mut tme) };
}

fn userdata(hwnd: HWND) -> isize {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) }
}

/// Sets or clears userdata `bits`; repaints when something changed.
fn set_bits(hwnd: HWND, bits: isize, on: bool) -> bool {
    let v = userdata(hwnd);
    let nv = if on { v | bits } else { v & !bits };
    if nv != v {
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, nv);
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }
    }
    nv != v
}

fn set_hover(hwnd: HWND, on: bool) {
    if set_bits(hwnd, BTN_HOVER, on) {
        anim::animate(hwnd, SLOT_HOVER, !on as u8 as f32, on as u8 as f32, HOVER_MS);
    }
}

/// (pushed, shown on): what a button draws, read around mouse messages.
fn visual(hwnd: HWND) -> (bool, bool) {
    let pushed = unsafe { SendMessageW(hwnd, BM_GETSTATE, 0, 0) } as u32 & BST_PUSHED != 0;
    (pushed, is_checked(hwnd) != pushed)
}

/// Push buttons (not switches, checkboxes or tabs) have the press look.
fn has_press_look(hwnd: HWND) -> bool {
    !matches!(userdata(hwnd) & KIND_MASK, BTN_SWITCH | BTN_TOGGLE | BTN_TAB)
}

/// Starts the press-release and switch/checkbox transitions for a visual
/// change caused by a mouse message. `released` = the release fade was
/// already started before the message was handled.
fn after_mouse(hwnd: HWND, before: (bool, bool), released: bool) {
    if unsafe { IsWindow(hwnd) } == 0 {
        return; // the click closed the window
    }
    let after = visual(hwnd);
    let kind = userdata(hwnd) & KIND_MASK;
    if matches!(kind, BTN_SWITCH | BTN_TOGGLE) {
        if before.1 != after.1 {
            anim::spring(hwnd, SLOT_ON, before.1 as u8 as f32, after.1 as u8 as f32, TOGGLE_MS);
        }
        return;
    }
    if kind == BTN_TAB {
        return; // tabs have no press look
    }
    match (before.0, after.0) {
        (true, false) if !released => anim::animate(hwnd, SLOT_PRESS, 1.0, 0.0, RELEASE_MS),
        (false, true) => anim::cancel(hwnd, SLOT_PRESS),
        _ => {}
    }
}

unsafe extern "system" fn button_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    let call = || unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    match msg {
        WM_TIMER if anim::on_timer(hwnd, wparam) => return 0,
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_LBUTTONUP => {
            let before = visual(hwnd);
            if msg != WM_LBUTTONUP && has_press_look(hwnd) {
                start_ripple(hwnd, lparam);
            }
            // The click may open a modal (menu, message box) inside call():
            // start the release fade first so the button never waits pressed.
            let released = msg == WM_LBUTTONUP && before.0 && has_press_look(hwnd);
            if released {
                anim::animate(hwnd, SLOT_PRESS, 1.0, 0.0, RELEASE_MS);
            }
            let r = anim::pointer_scope(call);
            after_mouse(hwnd, before, released);
            return r;
        }
        // draw_button paints the whole item (double-buffered): no erase flash.
        WM_ERASEBKGND => return 1,
        WM_MOUSEMOVE => {
            // Re-arm every time (idempotent): a leave can be lost while a popup menu is open.
            track_leave(hwnd);
            set_hover(hwnd, true);
            let before = visual(hwnd);
            let r = call();
            after_mouse(hwnd, before, false); // dragging out of / back onto a held button
            return r;
        }
        WM_CAPTURECHANGED => {
            // Releasing the mouse after a click keeps the hover (no fade-out).
            set_hover(hwnd, is_hovered(hwnd) && cursor_over(hwnd));
            let before = visual(hwnd);
            let r = call();
            after_mouse(hwnd, before, false);
            return r;
        }
        // A button disabled or hidden under the cursor never gets WM_MOUSELEAVE.
        WM_MOUSELEAVE | WM_ENABLE | WM_SHOWWINDOW => set_hover(hwnd, false),
        WM_SETCURSOR if userdata(hwnd) & KIND_MASK == BTN_LOGO => {
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND)) };
            return 1;
        }
        WM_NCDESTROY => {
            RIPPLES.with(|r| r.borrow_mut().remove(&(hwnd as usize)));
            anim::forget(hwnd)
        }
        _ => {}
    }
    call()
}

/// Makes an owner-drawn button repaint on hover (sets `BTN_HOVER`) and
/// animate hover, press and toggle changes made with the mouse.
pub fn track_button_hover(btn: HWND) {
    unsafe { SetWindowSubclass(btn, Some(button_proc), BUTTON_SUBCLASS, 0) };
}

pub fn is_hovered(btn: HWND) -> bool {
    userdata(btn) & BTN_HOVER != 0
}

fn cursor_over(hwnd: HWND) -> bool {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) != 0 && WindowFromPoint(pt) == hwnd }
}

/// Re-checks a hovered button after a modal popup menu, which can swallow
/// its WM_MOUSELEAVE.
pub fn sync_hover(btn: HWND) {
    if is_hovered(btn) && !cursor_over(btn) {
        set_hover(btn, false);
    }
}

/// Shows (Some(ok)) or hides the run-result tint of a button. The tint
/// appears at once; `fade_tint` crossfades it for a mouse-started run.
pub fn set_tint(btn: HWND, result: Option<bool>) {
    set_bits(btn, BTN_TINT, result.is_some());
    if let Some(ok) = result {
        set_bits(btn, BTN_TINT_ERR, !ok);
    }
}

/// Crossfades the tint in (`on`) or out over ~200 ms. Only while the tint
/// is shown (`set_tint`), so a tint that already vanished never flashes back.
pub fn fade_tint(btn: HWND, on: bool) {
    if userdata(btn) & BTN_TINT == 0 {
        return;
    }
    anim::animate(btn, SLOT_TINT, !on as u8 as f32, on as u8 as f32, super::TINT_MS);
}

/// Starts or stops the recording pulse of a button.
pub fn set_pulsing(btn: HWND, on: bool) {
    if set_bits(btn, BTN_PULSE, on) && !on {
        anim::cancel_pulse(btn, SLOT_PULSE);
    }
}

/// Toggles a switch or checkbox with its slide (a mouse click on its label).
pub fn toggle_animated(btn: HWND) {
    let was = is_checked(btn);
    super::super::set_checked(btn, !was);
    anim::spring(btn, SLOT_ON, was as u8 as f32, !was as u8 as f32, TOGGLE_MS);
}

// ---------- list boxes ----------

fn item_rect(list: HWND, item: i32) -> Option<RECT> {
    if item < 0 {
        return None;
    }
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe { SendMessageW(list, LB_GETITEMRECT, item as WPARAM, &mut rc as *mut RECT as LPARAM) } != LB_ERR as isize;
    ok.then_some(rc)
}

/// Item data (the row key) of `item`, if it exists.
fn item_key(list: HWND, item: i32) -> Option<u32> {
    if item < 0 {
        return None;
    }
    let data = unsafe { SendMessageW(list, LB_GETITEMDATA, item as WPARAM, 0) };
    (data != LB_ERR as isize).then_some(data as u32)
}

fn invalidate_item(list: HWND, item: i32) {
    if let Some(rc) = item_rect(list, item) {
        unsafe { InvalidateRect(list, &rc, 0) };
    }
}

fn set_list_hover(list: HWND, item: i32) {
    let old = LIST_HOVER.with(|m| m.borrow_mut().insert(list as usize, item)).unwrap_or(-1);
    if old == item {
        return;
    }
    // The old row glides out while the new one glides in.
    for (i, to) in [(old, 0.0), (item, 1.0)] {
        if let Some(key) = item_key(list, i) {
            anim::animate(list, row_slot(ROW_HOVER, key), 1.0 - to, to, ROW_HOVER_MS);
        }
        invalidate_item(list, i);
    }
}

fn set_list_press(list: HWND, item: i32) {
    let old = LIST_PRESS.with(|m| m.borrow_mut().insert(list as usize, item)).unwrap_or(-1);
    if old != item {
        invalidate_item(list, old);
        invalidate_item(list, item);
    }
}

/// Row under the client point in `lparam`, or -1. LB_ITEMFROMPOINT returns
/// the nearest row for a point below the last one; that is not a hit.
fn hit_item(list: HWND, lparam: LPARAM) -> i32 {
    let hit = unsafe { SendMessageW(list, LB_ITEMFROMPOINT, 0, lparam) } as u32;
    if hit >> 16 != 0 {
        return -1;
    }
    let item = (hit & 0xFFFF) as i32;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
    match item_rect(list, item) {
        Some(rc) if y >= rc.top && y < rc.bottom => item,
        _ => -1,
    }
}

/// Rows paint themselves (double-buffered); only the empty area below the
/// last row is filled here, with the brush the parent gives the list.
fn erase_list(list: HWND, hdc: HDC) {
    let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(list, &mut client) };
    let count = unsafe { SendMessageW(list, LB_GETCOUNT, 0, 0) };
    let top = if count > 0 { item_rect(list, count as i32 - 1).map_or(client.top, |rc| rc.bottom.max(client.top)) } else { client.top };
    if top >= client.bottom {
        return;
    }
    let brush = unsafe { SendMessageW(GetParent(list), WM_CTLCOLORLISTBOX, hdc as WPARAM, list as LPARAM) } as HBRUSH;
    let brush = if brush.is_null() { super::super::theme::bg_brush() } else { brush };
    unsafe { FillRect(hdc, &RECT { top, ..client }, brush) };
}

/// Mouse-down on a row: press its keycaps, and flash the row when the click
/// changed the selection (arrow keys select too, but never get here).
fn list_button_down(list: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let before = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
    set_list_press(list, hit_item(list, lparam));
    let r = anim::pointer_scope(|| unsafe { DefSubclassProc(list, msg, wparam, lparam) });
    if unsafe { IsWindow(list) } == 0 {
        return r;
    }
    let after = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
    if after >= 0
        && after != before
        && let Some(key) = item_key(list, after as i32)
    {
        anim::start(list, row_slot(ROW_SELECT, key), 1.0, 0.0, SELECT_FLASH_MS);
    }
    r
}

unsafe extern "system" fn list_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    match msg {
        WM_TIMER if anim::on_timer(hwnd, wparam) => return 0,
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => return list_button_down(hwnd, msg, wparam, lparam),
        WM_ERASEBKGND => {
            erase_list(hwnd, wparam as HDC);
            return 1;
        }
        WM_MOUSEMOVE => {
            track_leave(hwnd);
            set_list_hover(hwnd, hit_item(hwnd, lparam));
        }
        WM_MOUSELEAVE => {
            set_list_hover(hwnd, -1);
            set_list_press(hwnd, -1);
        }
        WM_LBUTTONUP | WM_CAPTURECHANGED => set_list_press(hwnd, -1),
        WM_NCDESTROY => {
            LIST_HOVER.with(|m| m.borrow_mut().remove(&(hwnd as usize)));
            LIST_PRESS.with(|m| m.borrow_mut().remove(&(hwnd as usize)));
            anim::forget(hwnd);
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Tracks the row under the mouse in an owner-drawn list box.
pub fn track_list_hover(list: HWND) {
    unsafe { SetWindowSubclass(list, Some(list_proc), LIST_SUBCLASS, 0) };
}

/// Forgets the hovered row (call before refilling a list: rows move under a
/// still cursor). Hover glides and the held-row look stop; other row
/// effects keep running.
pub fn reset_list_hover(list: HWND) {
    LIST_HOVER.with(|m| m.borrow_mut().insert(list as usize, -1));
    LIST_PRESS.with(|m| m.borrow_mut().insert(list as usize, -1));
    anim::cancel_where(list, |slot| slot >> 28 == ROW_HOVER);
}

/// Index of the hovered row in `list`, if any.
pub fn hovered_item(list: HWND) -> Option<u32> {
    LIST_HOVER.with(|m| m.borrow().get(&(list as usize)).copied()).filter(|&i| i >= 0).map(|i| i as u32)
}

/// Index of the row under a held mouse button in `list`, if any.
pub fn pressed_item(list: HWND) -> Option<u32> {
    LIST_PRESS.with(|m| m.borrow().get(&(list as usize)).copied()).filter(|&i| i >= 0).map(|i| i as u32)
}
