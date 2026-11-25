//! Painting of the main window's own surface: header, banner, field frame,
//! caption row, empty state, detail line and the action bar hairline.
//! Controls (buttons, list rows) paint themselves.

use super::view::{detail_line, View};
use super::{caption_y, header_h, height, list_bottom, list_top};
use super::{view_w, BANNER_H, BAR_H, DETAIL_H, DOT_SPACE, GAP, HEADER_GAP, PAD};
use crate::i18n::{t, tf};
use crate::ui::theme::palette;
use crate::ui::{caption, edit_frames, fonts, hline, kit, r, star_of_david, text, text_width, units};
use crate::win::px;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::*;

/// The banner as drawn this frame: current (animated) height and its text.
pub(super) struct Banner {
    pub height: i32,
    pub text: String,
}

/// Text of the banner the view asks for, if any.
pub(super) fn banner_text(v: &View) -> Option<String> {
    match (&v.emergency_error, v.all_enabled) {
        (Some(e), _) => Some(format!("⚠ {e}")),
        (None, false) => Some(t("main.status_disabled")),
        (None, true) => None,
    }
}

/// The banner slides down from under the header: drawn at full height,
/// clipped to its current height.
fn banner(hdc: HDC, b: &Banner) {
    if b.height <= 0 {
        return;
    }
    let top = header_h();
    unsafe {
        let saved = SaveDC(hdc);
        IntersectClipRect(hdc, 0, px(top), px(view_w()), px(top + b.height));
        kit::banner(hdc, &r(0, top + b.height - BANNER_H, view_w(), BANNER_H), &b.text);
        RestoreDC(hdc, saved);
    }
}

fn header(hdc: HDC, v: &View, status: (i32, i32), dot_pulse: f32) {
    let p = palette();
    let f = fonts();
    let hh = header_h();
    kit::header_bar(hdc, view_w(), hh);
    text(hdc, "NoKey", &r(PAD, 0, view_w() / 2, hh), f.title, p.text, DT_SINGLELINE | DT_VCENTER);
    if let Some(star) = p.star {
        let size = px(hh - 18);
        let cx = px(PAD + HEADER_GAP) + text_width(hdc, "NoKey", f.title) + size / 2;
        star_of_david(hdc, cx, px(hh / 2), size, star, p.bg);
    }
    // Discord status: dot + short text, right-aligned before the profile dropdown.
    let (left, right) = (px(status.0), px(status.1));
    let tw = text_width(hdc, &v.conn_short, f.small);
    let text_left = (right - tw).max(left + px(DOT_SPACE));
    if text_left < right {
        let cy = px(hh / 2);
        // Checking: a filled dot that pulses; connected: solid; else outlined.
        kit::status_dot_ex(hdc, text_left - px(DOT_SPACE) + px(4), cy, v.connected || v.checking, dot_pulse);
        let rc = RECT { left: text_left, top: cy - px(10), right, bottom: cy + px(10) };
        text(hdc, &v.conn_short, &rc, f.small, p.muted, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }
}

fn empty_state(hdc: HDC, banner_h: i32, searching: bool) {
    let p = palette();
    let f = fonts();
    let msg = t(if searching { "main.nothing_found" } else { "main.empty" });
    let (headline, hint) = msg.split_once('\n').unwrap_or((&msg, ""));
    let top = list_top(banner_h);
    let mid = top + (list_bottom() - top) / 2;
    let flags = DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_END_ELLIPSIS;
    let headline_y = if hint.is_empty() { mid - 12 } else { mid - 24 };
    text(hdc, headline, &r(PAD, headline_y, view_w() - 2 * PAD, 24), f.big, p.text, flags);
    text(hdc, hint, &r(PAD, mid, view_w() - 2 * PAD, 20), f.small, p.muted, flags);
}

/// Draws everything that is not a control into `hdc` (the window's memory DC).
#[allow(clippy::too_many_arguments)] // one call site; the inputs are a frame's snapshot
pub(super) fn window(
    hwnd: HWND,
    hdc: HDC,
    v: &View,
    search: HWND,
    flash: Option<(u32, bool)>,
    status: (i32, i32),
    b: &Banner,
    dot_pulse: f32,
) {
    let p = palette();
    let f = fonts();
    header(hdc, v, status, dot_pulse);
    banner(hdc, b);

    edit_frames(hwnd, hdc, &[search]);
    let cap_y = caption_y(b.height);
    let count = tf("main.active_count", &[("count", &v.total.to_string())]);
    let count_w = units(text_width(hdc, &count, f.small));
    caption(hdc, &t("main.bindings"), PAD, cap_y, view_w() - 2 * PAD - count_w - GAP);
    text(hdc, &count, &r(view_w() - PAD - count_w, cap_y, count_w, 18), f.small, p.muted, DT_SINGLELINE | DT_RIGHT);
    if v.rows.is_empty() {
        empty_state(hdc, b.height, !crate::win::get_text(search).trim().is_empty());
    }

    let (line, ok) = detail_line(v, flash);
    let color = if ok { p.muted } else { p.text };
    let detail = r(PAD, height() - BAR_H - DETAIL_H, view_w() - 2 * PAD, DETAIL_H);
    text(hdc, &line, &detail, f.small, color, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    hline(hdc, 0, height() - BAR_H, view_w());
}
