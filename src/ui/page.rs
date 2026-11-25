//! One window: the binding editor and Settings are child "pages" laid over
//! the main window. One page at a time; the main view's own controls are
//! hidden meanwhile. Each page header has the "NoKey" button that leads back.
//! A page opened with the mouse slides in.

use super::theme::palette;
use super::*;
use crate::i18n::t;
use std::cell::Cell;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;

/// Smallest client size of the window, and its size when it opens (96-DPI
/// units). It can be dragged larger or maximized; every view follows.
pub const MIN_W: i32 = 520;
pub const MIN_H: i32 = 620;
pub const HEADER_H: i32 = 52;
/// WM_COMMAND id of the "NoKey" header button.
pub const ID_HOME: i32 = 900;
const HOME_X: i32 = 8;
const HOME_PAD: i32 = 8;
/// How far to the side a page starts (96-DPI units), and for how long it slides.
const SLIDE_FROM: f32 = 64.0;
const SLIDE_MS: u32 = 280;
const SLOT_SLIDE: u32 = 0x9A6E_0001;

thread_local! {
    static OPEN: Cell<bool> = const { Cell::new(false) };
    /// Current client size (96-DPI units) and the same in device pixels.
    static SIZE: Cell<(i32, i32)> = const { Cell::new((MIN_W, MIN_H)) };
    static DEVICE: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    /// Main-view controls hidden while a page is open.
    static HIDDEN: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

pub fn is_open() -> bool {
    OPEN.with(Cell::get)
}

/// Current width of the window's client area (96-DPI units).
pub fn width() -> i32 {
    SIZE.with(Cell::get).0
}

/// Current height of the window's client area (96-DPI units).
pub fn height() -> i32 {
    SIZE.with(Cell::get).1
}

/// A window is about to be created at its default size.
pub fn reset_size() {
    SIZE.with(|s| s.set((MIN_W, MIN_H)));
    DEVICE.with(|d| d.set((px(MIN_W), px(MIN_H))));
}

/// How far a resize moved the right and bottom edges, in device pixels, and
/// the width before it (96-DPI units).
#[derive(Clone, Copy)]
pub struct Resize {
    pub dx: i32,
    pub dy: i32,
    pub old_width: i32,
}

/// The main window's client area is now `w` x `h` device pixels: every view
/// follows.
pub fn resized(w: i32, h: i32) {
    let (old_w, old_h) = DEVICE.with(|d| d.replace((w, h)));
    if (old_w, old_h) == (w, h) {
        return;
    }
    let by = Resize { dx: w - old_w, dy: h - old_h, old_width: width() };
    SIZE.with(|s| s.set((units(w), units(h))));
    main_window::relayout();
    for page in [editor::hwnd(), settings::hwnd()] {
        if !page.is_null() {
            // Keeps the position: a page may still be sliding in.
            unsafe { SetWindowPos(page, null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE) };
        }
    }
    editor::resize(by);
    settings::resize(by);
}

/// Follows a resize for controls that were placed once: `right` ones move
/// with the right edge, `wide` ones stretch to it, `low` ones move with the
/// bottom edge.
pub fn anchor(parent: HWND, by: Resize, right: &[HWND], wide: &[HWND], low: &[HWND]) {
    let mut all: Vec<HWND> = Vec::new();
    for &c in right.iter().chain(wide).chain(low) {
        if !c.is_null() && !all.contains(&c) {
            all.push(c);
        }
    }
    for c in all {
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe {
            GetWindowRect(c, &mut rc);
            // Two points, so a mirrored (Hebrew) parent keeps left < right.
            MapWindowPoints(null_mut(), parent, &mut rc as *mut RECT as *mut POINT, 2);
        }
        let x = rc.left + if right.contains(&c) { by.dx } else { 0 };
        let y = rc.top + if low.contains(&c) { by.dy } else { 0 };
        let w = rc.right - rc.left + if wide.contains(&c) { by.dx } else { 0 };
        unsafe { MoveWindow(c, x, y, w, rc.bottom - rc.top, 1) };
    }
    unsafe { InvalidateRect(parent, null(), 0) };
}

/// A hidden page window covering the main window's client area.
pub fn create(class: &str, main: HWND) -> HWND {
    let class = wide(class);
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe {
        GetClientRect(main, &mut rc);
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            null(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            rc.right,
            rc.bottom,
            main,
            null_mut(),
            hinstance(),
            null(),
        )
    }
}

/// Shows a loaded page in place of the main view.
pub fn show_page(page: HWND) {
    hide_main(page);
    if anim::by_pointer() {
        anim::start(page, SLOT_SLIDE, SLIDE_FROM, 0.0, SLIDE_MS);
        on_frame(page);
    }
    unsafe {
        SetWindowPos(page, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        // The clicked main-view control is hidden now: without focus inside
        // the page, Tab and Esc would do nothing.
        SetFocus(page);
    }
}

/// Follows the slide; pages call it after each animation frame.
pub fn on_frame(page: HWND) {
    let x = anim::value(page, SLOT_SLIDE, 0.0).round() as i32;
    unsafe { SetWindowPos(page, null_mut(), px(x), 0, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
}

fn hide_main(page: HWND) {
    OPEN.with(|o| o.set(true));
    let mut child = unsafe { GetWindow(GetParent(page), GW_CHILD) };
    while !child.is_null() {
        // The style bit, not IsWindowVisible: the window may be in the tray.
        let shown = unsafe { GetWindowLongW(child, GWL_STYLE) } as u32 & WS_VISIBLE != 0;
        if child != page && shown {
            HIDDEN.with(|h| h.borrow_mut().push(child as usize));
            show(child, false);
        }
        child = unsafe { GetWindow(child, GW_HWNDNEXT) };
    }
}

/// A page was destroyed: the main view comes back.
pub fn closed() {
    if !OPEN.with(|o| o.replace(false)) {
        return;
    }
    for child in HIDDEN.with(|h| std::mem::take(&mut *h.borrow_mut())) {
        show(child as HWND, true);
    }
    main_window::refresh();
    let list = main_window::list();
    if !list.is_null() {
        unsafe { SetFocus(list) };
    }
}

/// Closes whatever page is open. False when the user keeps an edited
/// binding, or a dialog is still waiting for an answer.
pub fn go_home() -> bool {
    let main = main_window::hwnd();
    if !main.is_null() && unsafe { IsWindowEnabled(main) } == 0 {
        return false;
    }
    if !editor::try_close() {
        return false;
    }
    settings::close();
    true
}

fn home_width(page: HWND) -> i32 {
    measure_text(page, "NoKey", fonts().title) + 2 * HOME_PAD
}

/// The "NoKey" button at the start of a page header.
pub fn home_button(page: HWND) -> HWND {
    let btn = button(page, ID_HOME, "NoKey", BTN_LOGO);
    place(btn, HOME_X, 10, home_width(page), 32);
    add_tooltip(page, btn, &t("tip.home"));
    btn
}

/// Header bar of a page: room for the "NoKey" button, "/", then the title.
pub fn paint_header(hdc: HDC, page: HWND, title: &str) {
    kit::header_bar(hdc, width(), HEADER_H);
    let (p, f) = (palette(), fonts());
    let x = HOME_X + home_width(page) + 2;
    let line = DT_SINGLELINE | DT_VCENTER;
    text(hdc, "/", &r(x, 0, 14, HEADER_H), f.title, p.muted, line | DT_CENTER);
    let tx = x + 22;
    text(hdc, title, &r(tx, 0, width() - 16 - tx, HEADER_H), f.title, p.text, line | DT_END_ELLIPSIS);
}
