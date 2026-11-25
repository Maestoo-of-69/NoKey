//! Owner-drawn slider (docs/DESIGN.md, Components): a 4px track, the part
//! up to the value in `accent`, and a round thumb. Its own window class, so
//! it gets focus, keys and mouse capture like a native control.
//!
//! - Mapping is logarithmic (`pos_of` / `value_at`): equal travel = equal
//!   ratio, which suits 30..30000. Values snap to 5 below 300, 10 below
//!   3000 and 100 from there up.
//! - Mouse: press = instant press look, the thumb jumps to the pointer and
//!   then tracks it 1:1 under capture; hover fades a halo in (120 ms),
//!   release fades the press out (100 ms). Keyboard and wheel snap.
//! - Hebrew: the child inherits the parent's mirrored layout, so drawing
//!   and mouse coordinates are both mirrored (the value grows to the left)
//!   and only ←/→ are swapped.
//!
//! Changes are sent to the parent as WM_COMMAND with `SLIDER_CHANGED`.

use super::super::anim::{self, lerp_color};
use super::super::theme::palette;
use super::super::{fill, hinstance, paint, register_class};
use super::{HOVER_MS, RELEASE_MS, SLOT_HOVER, SLOT_PRESS};
use crate::win::{px, wide};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// WM_COMMAND notification code (high word) of a value change. Far from
/// the EN_* / BN_* codes the editor handles itself.
pub const SLIDER_CHANGED: u32 = 0x0F01;
const CLASS: &str = "NoKeySlider";
/// Space at both ends so the thumb and its focus ring fit (96-DPI units).
pub const END_PAD: i32 = 13;
const THUMB_R: i32 = 7;
/// The thumb grows by this much while hovered or dragged.
const THUMB_GROW: i32 = 2;
const HALO_R: i32 = 13;
/// How far the halo tints `bg` toward `accent` at full hover.
const HALO_TINT: f32 = 0.32;
const PRESS_DARKEN: f32 = 0.15;
/// Keyboard / wheel steps as a share of the track.
const STEP_ARROW: f32 = 0.01;
const STEP_WHEEL: f32 = 0.02;
const STEP_PAGE: f32 = 0.10;

// ---------- mapping (pure) ----------

/// Snapping unit at `v`: 5 below 300, 10 below 3000, 100 from 3000 up.
fn unit(v: u32) -> u32 {
    match v {
        0..300 => 5,
        300..3000 => 10,
        _ => 100,
    }
}

fn snap(raw: f64, lo: u32, hi: u32) -> u32 {
    let u = unit(raw.round() as u32) as f64;
    ((raw / u).round() * u).clamp(lo as f64, hi as f64) as u32
}

/// Track position (0..1) of `value`, on a log scale.
pub fn pos_of(value: u32, lo: u32, hi: u32) -> f32 {
    let (lo, hi) = (lo.max(1) as f64, hi.max(lo + 1) as f64);
    (((value as f64).clamp(lo, hi) / lo).ln() / (hi / lo).ln()) as f32
}

/// Snapped value at track position `pos` (0..1).
pub fn value_at(pos: f32, lo: u32, hi: u32) -> u32 {
    let (l, h) = (lo.max(1) as f64, hi.max(lo + 1) as f64);
    snap(l * (h / l).powf(pos.clamp(0.0, 1.0) as f64), lo, hi)
}

/// `value` moved by `delta` of the track; at least one snapping unit.
pub fn step(value: u32, lo: u32, hi: u32, delta: f32) -> u32 {
    let next = value_at(pos_of(value, lo, hi) + delta, lo, hi);
    if next != value || delta == 0.0 {
        return next;
    }
    let u = unit(value);
    if delta > 0.0 { (value + u).min(hi) } else { value.saturating_sub(u).max(lo) }
}

// ---------- window ----------

#[derive(Clone, Copy)]
struct State {
    lo: u32,
    hi: u32,
    value: u32,
    hover: bool,
    dragging: bool,
}

thread_local! {
    static SLIDERS: RefCell<HashMap<usize, State>> = RefCell::new(HashMap::new());
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

fn state(hwnd: HWND) -> Option<State> {
    SLIDERS.with(|m| m.borrow().get(&(hwnd as usize)).copied())
}

fn update(hwnd: HWND, f: impl FnOnce(&mut State)) {
    SLIDERS.with(|m| {
        if let Some(s) = m.borrow_mut().get_mut(&(hwnd as usize)) {
            f(s);
        }
    });
}

/// Creates a slider (place it with `ui::place`; ~28 tall works well).
pub fn slider(parent: HWND, id: i32, (lo, hi): (u32, u32), value: u32) -> HWND {
    if !REGISTERED.with(Cell::get) {
        register_class(CLASS, Some(slider_proc), null_mut());
        REGISTERED.with(|r| r.set(true));
    }
    let class = wide(CLASS);
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            0,
            0,
            10,
            10,
            parent,
            id as isize as HMENU,
            hinstance(),
            null(),
        )
    };
    let value = value.clamp(lo, hi);
    SLIDERS.with(|m| m.borrow_mut().insert(hwnd as usize, State { lo, hi, value, hover: false, dragging: false }));
    hwnd
}

pub fn slider_value(hwnd: HWND) -> u32 {
    state(hwnd).map_or(0, |s| s.value)
}

/// Sets the value without notifying (loading a step); snaps.
pub fn set_slider_value(hwnd: HWND, value: u32) {
    update(hwnd, |s| s.value = value.clamp(s.lo, s.hi));
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

/// Stores a new value; repaints and tells the parent when it changed.
fn change(hwnd: HWND, value: u32) {
    let Some(old) = state(hwnd) else { return };
    let value = value.clamp(old.lo, old.hi);
    if value == old.value {
        return;
    }
    update(hwnd, |s| s.value = value);
    unsafe {
        InvalidateRect(hwnd, null(), 0);
        let id = GetDlgCtrlID(hwnd) as usize & 0xFFFF;
        SendMessageW(GetParent(hwnd), WM_COMMAND, (SLIDER_CHANGED as usize) << 16 | id, hwnd as LPARAM);
    }
}

/// Track ends (device pixels, client coordinates).
fn track(hwnd: HWND) -> (i32, i32, i32) {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetClientRect(hwnd, &mut rc) };
    (rc.left + px(END_PAD), (rc.right - px(END_PAD)).max(rc.left + px(END_PAD) + 1), (rc.top + rc.bottom) / 2)
}

fn value_at_x(hwnd: HWND, x: i32) -> Option<u32> {
    let s = state(hwnd)?;
    let (left, right, _) = track(hwnd);
    Some(value_at((x - left) as f32 / (right - left) as f32, s.lo, s.hi))
}

fn circle(hdc: HDC, cx: i32, cy: i32, r: i32, outline: COLORREF, fill_color: COLORREF) {
    unsafe {
        let pen = CreatePen(PS_SOLID, px(1).max(1), outline);
        let brush = CreateSolidBrush(fill_color);
        let (op, ob) = (SelectObject(hdc, pen as _), SelectObject(hdc, brush as _));
        Ellipse(hdc, cx - r, cy - r, cx + r + 1, cy + r + 1);
        SelectObject(hdc, op);
        SelectObject(hdc, ob);
        DeleteObject(pen as _);
        DeleteObject(brush as _);
    }
}

fn draw(hwnd: HWND, hdc: HDC) {
    let Some(s) = state(hwnd) else { return };
    let p = palette();
    let enabled = unsafe { IsWindowEnabled(hwnd) } != 0;
    let focused = unsafe { GetFocus() } == hwnd;
    let (left, right, cy) = track(hwnd);
    let x = left + ((right - left) as f32 * pos_of(s.value, s.lo, s.hi)).round() as i32;
    let half = px(2).max(2); // a 4px rail (2 above and below the center)
    let rail = RECT { left, top: cy - half, right, bottom: cy + half };
    fill(hdc, &rail, if enabled { p.border } else { p.line });
    if enabled {
        fill(hdc, &RECT { right: x, ..rail }, p.accent);
    }
    let hover = anim::value(hwnd, SLOT_HOVER, (s.hover || s.dragging) as u8 as f32);
    let press = anim::value(hwnd, SLOT_PRESS, s.dragging as u8 as f32);
    if enabled && hover > 0.01 {
        let halo = lerp_color(p.bg, p.accent, HALO_TINT * hover);
        circle(hdc, x, cy, px(HALO_R), halo, halo);
    }
    let grow = if enabled { (hover.max(press) * px(THUMB_GROW) as f32).round() as i32 } else { 0 };
    let thumb = px(THUMB_R) + grow;
    if focused && enabled {
        circle(hdc, x, cy, thumb + px(3), p.accent, if hover > 0.01 { lerp_color(p.bg, p.accent, HALO_TINT * hover) } else { p.bg });
    }
    let knob = if enabled { lerp_color(p.accent_text, p.key_edge, PRESS_DARKEN * press) } else { p.surface };
    circle(hdc, x, cy, thumb, if enabled { p.accent } else { p.line }, knob);
}

fn set_hover(hwnd: HWND, on: bool) {
    let Some(s) = state(hwnd) else { return };
    if s.hover == on {
        return;
    }
    update(hwnd, |s| s.hover = on);
    anim::animate(hwnd, SLOT_HOVER, !on as u8 as f32, on as u8 as f32, HOVER_MS);
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

fn on_key(hwnd: HWND, vk: u16) -> bool {
    let Some(s) = state(hwnd) else { return false };
    // In a mirrored (Hebrew) window "visually right" means a smaller value.
    let right = if crate::i18n::is_rtl() { -1.0 } else { 1.0 };
    let next = match vk {
        VK_RIGHT => step(s.value, s.lo, s.hi, right * STEP_ARROW),
        VK_LEFT => step(s.value, s.lo, s.hi, -right * STEP_ARROW),
        VK_UP => step(s.value, s.lo, s.hi, STEP_ARROW),
        VK_DOWN => step(s.value, s.lo, s.hi, -STEP_ARROW),
        VK_PRIOR => step(s.value, s.lo, s.hi, STEP_PAGE),
        VK_NEXT => step(s.value, s.lo, s.hi, -STEP_PAGE),
        VK_HOME => s.lo,
        VK_END => s.hi,
        _ => return false,
    };
    change(hwnd, next);
    true
}

fn mouse_x(lparam: LPARAM) -> i32 {
    (lparam & 0xFFFF) as i16 as i32
}

fn on_button_down(hwnd: HWND, lparam: LPARAM) {
    unsafe {
        SetFocus(hwnd);
        SetCapture(hwnd);
    }
    update(hwnd, |s| s.dragging = true);
    anim::cancel(hwnd, SLOT_PRESS); // press is instant
    if let Some(v) = value_at_x(hwnd, mouse_x(lparam)) {
        change(hwnd, v);
    }
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

fn end_drag(hwnd: HWND) {
    if state(hwnd).is_some_and(|s| s.dragging) {
        update(hwnd, |s| s.dragging = false);
        anim::animate(hwnd, SLOT_PRESS, 1.0, 0.0, RELEASE_MS);
        unsafe { InvalidateRect(hwnd, null(), 0) };
    }
}

fn track_leave(hwnd: HWND) {
    let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
    unsafe { TrackMouseEvent(&mut tme) };
}

unsafe extern "system" fn slider_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => paint(hwnd, |hdc, _| draw(hwnd, hdc)),
        WM_ERASEBKGND => return 1,
        WM_GETDLGCODE => return DLGC_WANTARROWS as LRESULT,
        WM_KEYDOWN if on_key(hwnd, wparam as u16) => {}
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as i16;
            if let Some(s) = state(hwnd) {
                change(hwnd, step(s.value, s.lo, s.hi, STEP_WHEEL * delta.signum() as f32));
            }
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => on_button_down(hwnd, lparam),
        WM_MOUSEMOVE => {
            track_leave(hwnd);
            set_hover(hwnd, true);
            if state(hwnd).is_some_and(|s| s.dragging)
                && let Some(v) = value_at_x(hwnd, mouse_x(lparam))
            {
                change(hwnd, v);
            }
        }
        WM_LBUTTONUP => unsafe {
            ReleaseCapture(); // WM_CAPTURECHANGED ends the drag
        },
        WM_CAPTURECHANGED => end_drag(hwnd),
        WM_MOUSELEAVE => set_hover(hwnd, false),
        WM_SETFOCUS | WM_KILLFOCUS | WM_ENABLE => unsafe {
            InvalidateRect(hwnd, null(), 0);
        },
        WM_TIMER if anim::on_timer(hwnd, wparam) => {}
        WM_NCDESTROY => {
            anim::forget(hwnd);
            SLIDERS.with(|m| m.borrow_mut().remove(&(hwnd as usize)));
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: (u32, u32) = (30, 30_000);

    #[test]
    fn log_mapping_hits_the_ends_and_the_middle() {
        assert_eq!(value_at(0.0, R.0, R.1), 30);
        assert_eq!(value_at(1.0, R.0, R.1), 30_000);
        assert_eq!(value_at(0.5, R.0, R.1), 950, "geometric middle of 30..30000");
        assert!((pos_of(300, R.0, R.1) - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(pos_of(10, R.0, R.1), 0.0, "clamped below");
        assert_eq!(value_at(-1.0, R.0, R.1), 30);
        assert_eq!(value_at(2.0, R.0, R.1), 30_000);
    }

    #[test]
    fn snapped_values_round_trip_through_the_track() {
        let mut v = R.0;
        while v <= R.1 {
            assert_eq!(value_at(pos_of(v, R.0, R.1), R.0, R.1), v, "value {v}");
            v += unit(v);
        }
    }

    #[test]
    fn values_snap_to_5_then_10_then_100() {
        assert_eq!((unit(295), unit(300), unit(2990), unit(3000)), (5, 10, 10, 100));
        for i in 0..=1000 {
            let v = value_at(i as f32 / 1000.0, R.0, R.1);
            assert_eq!(v % unit(v), 0, "{v}");
        }
    }

    #[test]
    fn steps_always_move_and_stop_at_the_ends() {
        assert_eq!(step(30, R.0, R.1, STEP_ARROW), 35, "at least one unit");
        assert_eq!(step(30, R.0, R.1, -STEP_ARROW), 30);
        assert_eq!(step(30_000, R.0, R.1, STEP_PAGE), 30_000);
        assert_eq!(step(10_000, R.0, R.1, STEP_ARROW) % 100, 0);
        assert!(step(300, R.0, R.1, STEP_ARROW) > 300);
        assert!(step(300, R.0, R.1, -STEP_PAGE) < 300);
        let up = step(1000, R.0, R.1, STEP_ARROW);
        assert_eq!(up % 10, 0);
        assert_eq!(step(1000, R.0, R.1, 0.0), 1000);
    }
}
