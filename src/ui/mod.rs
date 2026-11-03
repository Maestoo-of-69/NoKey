//! Windows and drawing. Native controls (edit boxes, list boxes) are used
//! for input; buttons and text are owner-drawn with the current theme.
//! Every visible string comes from `i18n`.

pub mod editor;
pub mod host;
pub mod main_window;
pub mod settings;
pub mod theme;

use crate::i18n::is_rtl;
use crate::win::{px, wide};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;
use theme::palette;
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, IsWindowEnabled};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub struct Fonts {
    pub title: HFONT,
    pub normal: HFONT,
    pub small: HFONT,
    pub bold: HFONT,
    pub big: HFONT,
}

// GDI handles are process-wide; storing them as usize keeps OnceLock happy.
struct FontHandles([usize; 5]);
static FONTS: OnceLock<FontHandles> = OnceLock::new();

fn font(size: i32, weight: i32) -> HFONT {
    let face = wide("Segoe UI");
    unsafe {
        CreateFontW(
            -px(size),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        )
    }
}

pub fn fonts() -> Fonts {
    let h = FONTS.get_or_init(|| {
        FontHandles([
            font(22, FW_SEMIBOLD as i32) as usize,
            font(14, FW_NORMAL as i32) as usize,
            font(12, FW_NORMAL as i32) as usize,
            font(14, FW_SEMIBOLD as i32) as usize,
            font(22, FW_BOLD as i32) as usize,
        ])
    });
    let p = |i: usize| h.0[i] as *mut core::ffi::c_void;
    Fonts { title: p(0), normal: p(1), small: p(2), bold: p(3), big: p(4) }
}

// ---------- window helpers ----------

pub fn hinstance() -> windows_sys::Win32::Foundation::HINSTANCE {
    unsafe { GetModuleHandleW(null()) }
}

pub fn register_class(name: &str, proc: WNDPROC, icon: HICON) {
    let class = wide(name);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance(),
        hIcon: icon,
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: null_mut(), // windows paint their own background
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
        hIconSm: icon,
    };
    unsafe { RegisterClassExW(&wc) };
}

/// Creates a fixed-size top-level window whose client area is `w` x `h`
/// (96-DPI units), centered on the work area. Hebrew mirrors the layout.
pub fn create_window(class: &str, title: &str, w: i32, h: i32, owner: HWND) -> HWND {
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    let ex = if is_rtl() { WS_EX_LAYOUTRTL } else { 0 };
    let mut rc = RECT { left: 0, top: 0, right: px(w), bottom: px(h) };
    unsafe { AdjustWindowRectEx(&mut rc, style, 0, ex) };
    let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
    let mut work = RECT { left: 0, top: 0, right: 1280, bottom: 800 };
    unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut _, 0) };
    let x = work.left + ((work.right - work.left) - ww) / 2;
    let y = work.top + ((work.bottom - work.top) - wh).max(0) / 2;
    let c = wide(class);
    let t = wide(title);
    let hwnd = unsafe {
        CreateWindowExW(ex, c.as_ptr(), t.as_ptr(), style, x, y, ww, wh, owner, null_mut(), hinstance(), null())
    };
    apply_frame(hwnd);
    hwnd
}

/// Title bar colors for the current theme (Windows 10 2004+ / 11).
pub fn apply_frame(hwnd: HWND) {
    let p = palette();
    let dark: i32 = p.dark as i32;
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark as *const i32 as _, 4);
        DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR as u32, &p.bg as *const u32 as _, 4);
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, &p.line as *const u32 as _, 4);
        DwmSetWindowAttribute(hwnd, DWMWA_TEXT_COLOR as u32, &p.text as *const u32 as _, 4);
    }
}

/// Repaints a window and all its controls (after a theme change).
pub fn repaint(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    apply_frame(hwnd);
    unsafe { RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME) };
}

pub fn place(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    unsafe { MoveWindow(hwnd, px(x), px(y), px(w), px(h), 1) };
}

pub fn show(hwnd: HWND, visible: bool) {
    unsafe { ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE }) };
}

pub fn enable(hwnd: HWND, on: bool) {
    unsafe { EnableWindow(hwnd, on as i32) };
}

pub fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

pub fn is_visible(hwnd: HWND) -> bool {
    !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } != 0
}

// ---------- controls ----------

pub const BTN_SECONDARY: isize = 0;
pub const BTN_PRIMARY: isize = 1;
pub const BTN_BIG: isize = 2;
pub const BTN_TOGGLE: isize = 3;
pub const BTN_DROPDOWN: isize = 4;
/// A tab header; "checked" = the selected tab.
pub const BTN_TAB: isize = 5;
/// Warning style (the "Enable all binds" state).
pub const BTN_WARN: isize = 6;
const BTN_CHECKED: isize = 0x100;
const KIND_MASK: isize = 0xFF;

fn child(parent: HWND, class: &str, text: &str, style: u32, id: i32) -> HWND {
    let c = wide(class);
    let t = wide(text);
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            c.as_ptr(),
            t.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
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
    let f = if class == "BUTTON" { fonts().bold } else { fonts().normal };
    unsafe { SendMessageW(hwnd, WM_SETFONT, f as WPARAM, 0) };
    hwnd
}

pub fn button(parent: HWND, id: i32, text: &str, kind: isize) -> HWND {
    let hwnd = child(parent, "BUTTON", text, WS_TABSTOP | BS_OWNERDRAW as u32, id);
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, kind) };
    hwnd
}

pub fn set_kind(btn: HWND, kind: isize) {
    unsafe {
        let checked = GetWindowLongPtrW(btn, GWLP_USERDATA) & BTN_CHECKED;
        SetWindowLongPtrW(btn, GWLP_USERDATA, kind | checked);
        InvalidateRect(btn, null(), 0);
    }
}

pub fn edit(parent: HWND, id: i32, extra_style: u32) -> HWND {
    child(parent, "EDIT", "", WS_TABSTOP | ES_AUTOHSCROLL as u32 | extra_style, id)
}

pub fn set_cue(edit: HWND, text: &str) {
    let t = wide(text);
    unsafe { SendMessageW(edit, EM_SETCUEBANNER, 1, t.as_ptr() as LPARAM) };
}

pub fn listbox(parent: HWND, id: i32, extra_style: u32) -> HWND {
    let style = WS_TABSTOP | WS_VSCROLL | (LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_NOINTEGRALHEIGHT) as u32 | extra_style;
    let hwnd = child(parent, "LISTBOX", "", style, id);
    let theme_name = wide(if palette().dark { "DarkMode_Explorer" } else { "Explorer" });
    unsafe { SetWindowTheme(hwnd, theme_name.as_ptr(), null()) };
    hwnd
}

pub fn is_checked(btn: HWND) -> bool {
    unsafe { GetWindowLongPtrW(btn, GWLP_USERDATA) & BTN_CHECKED != 0 }
}

pub fn set_checked(btn: HWND, on: bool) {
    unsafe {
        let v = GetWindowLongPtrW(btn, GWLP_USERDATA) & !BTN_CHECKED;
        SetWindowLongPtrW(btn, GWLP_USERDATA, if on { v | BTN_CHECKED } else { v });
        InvalidateRect(btn, null(), 0);
    }
}

pub fn toggle(btn: HWND) {
    set_checked(btn, !is_checked(btn));
}

thread_local! {
    static TOOLTIPS: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
}

/// Adds a hover tooltip to a control (one tooltip window per top-level window).
pub fn add_tooltip(parent: HWND, ctrl: HWND, text: &str) {
    let tip = TOOLTIPS.with(|t| t.borrow().get(&(parent as usize)).copied());
    let tip = match tip {
        Some(t) if unsafe { IsWindow(t as HWND) } != 0 => t as HWND,
        _ => {
            let class = wide("tooltips_class32");
            let ex = if is_rtl() { WS_EX_LAYOUTRTL } else { 0 };
            let t = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | ex,
                    class.as_ptr(),
                    null(),
                    WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                    0,
                    0,
                    0,
                    0,
                    parent,
                    null_mut(),
                    hinstance(),
                    null(),
                )
            };
            unsafe {
                SendMessageW(t, TTM_SETMAXTIPWIDTH, 0, px(320) as LPARAM);
                SendMessageW(t, TTM_SETDELAYTIME, TTDT_AUTOPOP as WPARAM, 15_000);
            }
            TOOLTIPS.with(|m| m.borrow_mut().insert(parent as usize, t as usize));
            t
        }
    };
    let mut text_w = wide(text);
    let mut ti: TTTOOLINFOW = unsafe { std::mem::zeroed() };
    ti.cbSize = std::mem::size_of::<TTTOOLINFOW>() as u32;
    ti.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
    ti.hwnd = parent;
    ti.uId = ctrl as usize;
    ti.lpszText = text_w.as_mut_ptr();
    unsafe { SendMessageW(tip, TTM_ADDTOOLW, 0, &ti as *const TTTOOLINFOW as LPARAM) };
}

/// Handles WM_CTLCOLOR* so edit boxes and list boxes use the theme.
pub fn ctl_color(msg: u32, wparam: WPARAM) -> Option<LRESULT> {
    let hdc = wparam as HDC;
    let p = palette();
    match msg {
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => unsafe {
            SetTextColor(hdc, p.text);
            SetBkColor(hdc, p.surface);
            Some(theme::surface_brush() as LRESULT)
        },
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => unsafe {
            SetTextColor(hdc, p.text);
            SetBkColor(hdc, p.bg);
            Some(theme::bg_brush() as LRESULT)
        },
        _ => None,
    }
}

// ---------- drawing ----------

pub fn r(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT { left: px(x), top: px(y), right: px(x + w), bottom: px(y + h) }
}

pub fn fill(hdc: HDC, rc: &RECT, color: COLORREF) {
    unsafe {
        let b = CreateSolidBrush(color);
        FillRect(hdc, rc, b);
        DeleteObject(b as _);
    }
}

pub fn text(hdc: HDC, s: &str, rc: &RECT, font: HFONT, color: COLORREF, flags: DRAW_TEXT_FORMAT) {
    // An empty Vec has a dangling (non-null) pointer, and DrawTextW with
    // DT_END_ELLIPSIS reads the buffer even when the length is 0.
    if s.is_empty() {
        return;
    }
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut rc = *rc;
    // No DT_RTLREADING: in Hebrew every NoKey window (and its memory DC) is
    // mirrored, which already gives right-to-left reading; the flag would
    // flip it back.
    unsafe {
        SelectObject(hdc, font as _);
        SetTextColor(hdc, color);
        SetBkMode(hdc, TRANSPARENT as i32);
        DrawTextW(hdc, w.as_ptr(), w.len() as i32, &mut rc, flags | DT_NOPREFIX);
    }
}

pub fn text_width(hdc: HDC, s: &str, font: HFONT) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut rc = RECT { left: 0, top: 0, right: 10_000, bottom: 100 };
    unsafe {
        SelectObject(hdc, font as _);
        DrawTextW(hdc, w.as_ptr(), w.len() as i32, &mut rc, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    }
    rc.right - rc.left
}

/// Rounded rectangle with a 1px outline and optional fill.
pub fn round_rect(hdc: HDC, rc: &RECT, outline: COLORREF, fill_color: Option<COLORREF>, radius: i32) {
    unsafe {
        let pen = CreatePen(PS_SOLID, px(1).max(1), outline);
        let brush = match fill_color {
            Some(c) => CreateSolidBrush(c),
            None => GetStockObject(NULL_BRUSH) as HBRUSH,
        };
        let old_pen = SelectObject(hdc, pen as _);
        let old_brush = SelectObject(hdc, brush as _);
        RoundRect(hdc, rc.left, rc.top, rc.right, rc.bottom, px(radius), px(radius));
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        DeleteObject(pen as _);
        if fill_color.is_some() {
            DeleteObject(brush as _);
        }
    }
}

pub fn hline(hdc: HDC, x: i32, y: i32, w: i32) {
    fill(hdc, &r(x, y, w, 1), palette().line);
}

/// Small upper-case section label.
pub fn caption(hdc: HDC, s: &str, x: i32, y: i32, w: i32) {
    text(hdc, &s.to_uppercase(), &r(x, y, w, 18), fonts().small, palette().muted, DT_SINGLELINE | DT_LEFT);
}

/// Window header background. The Jewish theme adds a light lattice of
/// interlocking triangles and blue stripes; other themes are plain.
pub fn header_band(hdc: HDC, width: i32, height: i32) {
    let p = palette();
    let Some(pattern) = p.pattern else { return };
    unsafe {
        let saved = SaveDC(hdc);
        IntersectClipRect(hdc, 0, 0, px(width), px(height));
        let pen = CreatePen(PS_SOLID, 1, pattern);
        let old = SelectObject(hdc, pen as _);
        let step = px(16);
        let h = px(height);
        let w = px(width);
        // horizontal lines + two diagonal families = triangular lattice
        let mut y = step / 2;
        while y < h {
            MoveToEx(hdc, 0, y, null_mut());
            LineTo(hdc, w, y);
            y += step;
        }
        let run = (h as f32 / 1.732) as i32; // horizontal run of a 60° line
        let mut x = -run;
        while x < w + run {
            MoveToEx(hdc, x, 0, null_mut());
            LineTo(hdc, x + run, h);
            MoveToEx(hdc, x, 0, null_mut());
            LineTo(hdc, x - run, h);
            x += step;
        }
        SelectObject(hdc, old);
        DeleteObject(pen as _);
        RestoreDC(hdc, saved);
    }
    fill(hdc, &RECT { left: 0, top: 0, right: px(width), bottom: px(4) }, p.accent);
    fill(hdc, &RECT { left: 0, top: px(height) - px(2), right: px(width), bottom: px(height) }, p.accent);
}

/// Draws the frame around each visible edit box (accent when focused).
pub fn edit_frames(parent: HWND, hdc: HDC, edits: &[HWND]) {
    let p = palette();
    let focus = unsafe { GetFocus() };
    for &e in edits {
        if unsafe { IsWindowVisible(e) } == 0 {
            continue;
        }
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe {
            GetWindowRect(e, &mut rc);
            MapWindowPoints(null_mut(), parent, &mut rc as *mut RECT as *mut POINT, 2);
        }
        let (l, rr) = (rc.left.min(rc.right), rc.left.max(rc.right));
        let frame = RECT { left: l - px(10), top: rc.top - px(8), right: rr + px(10), bottom: rc.bottom + px(8) };
        let enabled = unsafe { IsWindowEnabled(e) } != 0;
        let outline = if focus == e { p.accent } else if enabled { p.border } else { p.line };
        round_rect(hdc, &frame, outline, Some(p.surface), 8);
    }
}

/// Paints an owner-drawn button.
pub fn draw_button(dis: &DRAWITEMSTRUCT) {
    let hdc = dis.hDC;
    let rc = dis.rcItem;
    let raw = unsafe { GetWindowLongPtrW(dis.hwndItem, GWLP_USERDATA) };
    let checked = raw & BTN_CHECKED != 0;
    let kind = raw & KIND_MASK;
    let pressed = dis.itemState & ODS_SELECTED != 0;
    let disabled = dis.itemState & ODS_DISABLED != 0;
    let focused = dis.itemState & ODS_FOCUS != 0;
    let p = palette();
    let f = fonts();
    let label = crate::win::get_text(dis.hwndItem);

    fill(hdc, &rc, p.bg);
    if kind == BTN_TOGGLE {
        let size = px(18);
        let top = rc.top + (rc.bottom - rc.top - size) / 2;
        let bx = RECT { left: rc.left, top, right: rc.left + size, bottom: top + size };
        let on = checked != pressed;
        let outline = if disabled { p.line } else if on || focused { p.accent } else { p.border };
        round_rect(hdc, &bx, outline, Some(if on { p.accent } else { p.bg }), 5);
        if on {
            text(hdc, "✓", &bx, f.bold, p.accent_text, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        let tr = RECT { left: bx.right + px(10), ..rc };
        let color = if disabled { p.muted } else { p.text };
        text(hdc, &label, &tr, f.normal, color, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        return;
    }
    if kind == BTN_TAB {
        let color = if checked { p.text } else { p.muted };
        text(hdc, &label, &rc, f.bold, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        if checked {
            fill(hdc, &RECT { top: rc.bottom - px(3), ..rc }, p.accent);
        } else {
            fill(hdc, &RECT { top: rc.bottom - px(1), ..rc }, p.line);
        }
        return;
    }

    let (bg, fg, outline) = match (kind, pressed, disabled) {
        (_, _, true) => (p.surface, p.muted, p.surface),
        (BTN_PRIMARY | BTN_BIG, false, _) => (p.accent, p.accent_text, p.accent),
        (BTN_PRIMARY | BTN_BIG, true, _) => (p.selected, p.text, p.accent),
        (BTN_WARN, false, _) => (p.warn, p.warn_text, p.warn),
        (BTN_WARN, true, _) => (p.selected, p.text, p.warn),
        (_, false, _) => (p.bg, p.text, if focused { p.accent } else { p.border }),
        (_, true, _) => (p.accent, p.accent_text, p.accent),
    };
    let radius = if kind == BTN_BIG || kind == BTN_WARN { 14 } else { 8 };
    let inner = RECT { left: rc.left, top: rc.top, right: rc.right - 1, bottom: rc.bottom - 1 };
    round_rect(hdc, &inner, outline, Some(bg), radius);
    if kind == BTN_DROPDOWN {
        let tr = RECT { left: rc.left + px(12), right: rc.right - px(26), ..rc };
        text(hdc, &label, &tr, f.normal, fg, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        let ar = RECT { left: rc.right - px(26), ..rc };
        text(hdc, "▾", &ar, f.normal, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    } else {
        let font = if kind == BTN_BIG || kind == BTN_WARN { f.big } else { f.bold };
        let tr = RECT { left: rc.left + px(6), right: rc.right - px(6), ..rc };
        text(hdc, &label, &tr, font, fg, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    }
    if focused && (kind == BTN_BIG || kind == BTN_WARN) {
        let ring = RECT { left: rc.left + px(4), top: rc.top + px(4), right: rc.right - px(5), bottom: rc.bottom - px(5) };
        round_rect(hdc, &ring, fg, None, 10);
    }
}

/// Double-buffered painting: draw into a memory DC, then copy at once
/// (prevents flicker). In RTL windows the memory DC is mirrored too.
pub fn paint(hwnd: HWND, draw: impl FnOnce(HDC, &RECT)) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut rc);
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
        let old = SelectObject(mem, bmp as _);
        if is_rtl() {
            SetLayout(mem, LAYOUT_RTL);
        }
        fill(mem, &rc, palette().bg);
        draw(mem, &rc);
        BitBlt(hdc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old);
        DeleteObject(bmp as _);
        DeleteDC(mem);
        EndPaint(hwnd, &ps);
    }
}

/// Shows a popup menu under `anchor` and returns the chosen index.
/// Items equal to "-" are separators; `checked` marks items.
pub fn popup_menu(owner: HWND, anchor: HWND, items: &[String], checked: &[usize]) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    unsafe {
        let menu = CreatePopupMenu();
        for (i, item) in items.iter().enumerate() {
            if item == "-" {
                AppendMenuW(menu, MF_SEPARATOR, 0, null());
                continue;
            }
            let w = wide(item);
            let flags = if checked.contains(&i) { MF_STRING | MF_CHECKED } else { MF_STRING };
            AppendMenuW(menu, flags, i + 1, w.as_ptr());
        }
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(anchor, &mut rc);
        let (x, align) = if is_rtl() { (rc.right, TPM_RIGHTALIGN | TPM_LAYOUTRTL) } else { (rc.left, TPM_LEFTALIGN) };
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | align | TPM_TOPALIGN, x, rc.bottom + 2, 0, owner, null());
        DestroyMenu(menu);
        (cmd > 0).then(|| cmd as usize - 1)
    }
}

/// Low word of WPARAM in WM_COMMAND is the control id, high word the notification.
pub fn command_parts(wparam: WPARAM) -> (i32, u32) {
    ((wparam & 0xFFFF) as i32, ((wparam >> 16) & 0xFFFF) as u32)
}

pub fn def(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub fn rtl_box() -> MESSAGEBOX_STYLE {
    if is_rtl() { MB_RTLREADING | MB_RIGHT } else { 0 }
}

/// Shows a message box with an OK button.
pub fn alert(owner: HWND, text: &str) {
    crate::win::message_box(owner, text, MB_OK | MB_ICONWARNING | rtl_box());
}

/// Shows a Yes/No question.
pub fn confirm(owner: HWND, text: &str) -> bool {
    crate::win::message_box(owner, text, MB_YESNO | MB_ICONQUESTION | rtl_box()) == IDYES
}

/// Repaints every NoKey window after a theme change.
pub fn repaint_all() {
    for hwnd in [main_window::hwnd(), settings::hwnd(), editor::hwnd()] {
        repaint(hwnd);
    }
}

/// Re-creates the windows after a language change (Hebrew needs a mirrored
/// layout, which Windows only applies when a window is created).
pub fn rebuild_all() {
    editor::close();
    let settings_tab = settings::current_tab();
    settings::close();
    main_window::rebuild();
    if let Some(tab) = settings_tab {
        settings::open_tab(main_window::hwnd(), tab);
    }
}
