//! Measuring and drawing helpers shared by the editor's tabs: text
//! measured in 96-DPI units, the measured control geometry (`Geo`) used by
//! both placing and painting, field frames, the header and action bar.

use super::super::theme::{self, palette};
use super::super::*;
use super::{content_w, FRAME_H, view_h, MODES, PAD, TAB_KEYS, view_w};
use crate::config::TriggerMode;
use crate::i18n::t;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

/// Extra width around a measured label: buttons (bold), dropdowns (normal
/// text + arrow), checkboxes (normal text + box).
pub(super) const BTN_EXTRA: i32 = 24;
pub(super) const DROP_EXTRA: i32 = 40;
pub(super) const TOGGLE_EXTRA: i32 = 30;

fn wrapped_height(hdc: HDC, s: &str, font: HFONT, width: i32) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut rc = RECT { left: 0, top: 0, right: width, bottom: 0 };
    unsafe {
        SelectObject(hdc, font as _);
        DrawTextW(hdc, w.as_ptr(), w.len() as i32, &mut rc, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
    }
    rc.bottom - rc.top
}

/// Measures many strings in 96-DPI units on one window DC (released on
/// drop). For a single width, `measure_text` is enough.
pub(super) struct Measure {
    hwnd: HWND,
    hdc: HDC,
}

impl Measure {
    pub fn window(hwnd: HWND) -> Self {
        Self { hwnd, hdc: unsafe { GetDC(hwnd) } }
    }

    pub fn width(&self, s: &str, font: HFONT) -> i32 {
        units(text_width(self.hdc, s, font))
    }

    pub fn widest(&self, labels: &[String], font: HFONT) -> i32 {
        labels.iter().map(|s| self.width(s, font)).max().unwrap_or(0)
    }

    /// Height of `s` word-wrapped in a box `w` units wide.
    pub fn height(&self, s: &str, font: HFONT, w: i32) -> i32 {
        units(wrapped_height(self.hdc, s, font, px(w)))
    }
}

impl Drop for Measure {
    fn drop(&mut self) {
        unsafe { ReleaseDC(self.hwnd, self.hdc) };
    }
}

pub(super) fn labels(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|k| t(k)).collect()
}

/// Measured x positions and widths (96-DPI units) of the editor controls
/// whose labels vary by language. Computed once per window from every
/// label a control can show, so placing and painting agree.
#[derive(Clone, Copy)]
pub(super) struct Geo {
    pub tabs: [(i32, i32); 4],
    pub mode: (i32, i32),
    pub record: (i32, i32),
    pub remove: (i32, i32),
    pub trigger_w: i32,
    pub repeat: [(i32, i32); 2],
    /// Stop-shortcut row: label width, then Record and Clear at the right.
    pub stop_label_w: i32,
    pub stop_record: (i32, i32),
    pub stop_clear: (i32, i32),
    pub cancel: (i32, i32),
    pub save: (i32, i32),
}

impl Geo {
    pub fn measure(hwnd: HWND) -> Geo {
        let m = Measure::window(hwnd);
        let f = fonts();
        let button = |keys: &[&str], min: i32| (m.widest(&labels(keys), f.bold) + BTN_EXTRA).max(min);
        let caption_w = |key: &str, min: i32| (m.width(&sentence_case(&t(key)), f.small) + 8).max(min);
        let modes: Vec<String> = MODES.iter().map(|(_, k)| t(k)).collect();
        let triggers: Vec<String> = TriggerMode::ALL.iter().map(|mode| t(mode.key())).collect();
        // Recording switches the button to the warn style, which uses the big font.
        let record_w = (m.widest(&labels(&["editor.record", "editor.stop"]), f.big) + BTN_EXTRA).max(88);
        let mode_w = (m.widest(&modes, f.normal) + DROP_EXTRA).max(120);
        let remove_w = button(&["editor.remove"], 80);
        let (save_w, cancel_w) = (button(&["common.save"], 96), button(&["common.cancel"], 80));
        let trigger_w = (m.widest(&triggers, f.normal) + DROP_EXTRA).max(160);
        let rep_x = PAD + trigger_w + 16;
        let rep_w = caption_w("editor.repeat_interval", 96);
        let max_x = rep_x + rep_w + 16;
        let max_w = caption_w("editor.repeat_max", 80).min(view_w() - PAD - max_x);
        let record = (view_w() - PAD - record_w, record_w);
        let clear_w = button(&["editor.clear"], 72);
        let stop_clear = (view_w() - PAD - clear_w, clear_w);
        // The label gives way first: a third of the row at most.
        let stop_label_w = caption_w("editor.cancel_shortcut", 0).min(content_w() / 3);
        let save = (view_w() - PAD - save_w, save_w);
        Geo {
            tabs: tab_slots(&m),
            mode: (record.0 - 8 - mode_w, mode_w),
            record,
            remove: (view_w() - PAD - remove_w, remove_w),
            trigger_w,
            repeat: [(rep_x, rep_w), (max_x, max_w)],
            stop_label_w,
            stop_record: (stop_clear.0 - 8 - record_w, record_w),
            stop_clear,
            cancel: (save.0 - 8 - cancel_w, cancel_w),
            save,
        }
    }
}

/// Tabs sized to their labels from the left edge; equal widths if they
/// would not fit.
fn tab_slots(m: &Measure) -> [(i32, i32); 4] {
    let widths = TAB_KEYS.map(|k| (m.width(&t(k), fonts().bold) + BTN_EXTRA).max(64));
    let fits = widths.iter().sum::<i32>() + 4 * 3 <= content_w();
    let mut x = PAD;
    widths.map(|w| {
        let w = if fits { w } else { content_w() / 4 };
        let slot = (x, w);
        x += w + if fits { 4 } else { 0 };
        slot
    })
}

/// Places an edit so that its frame (see `edit_frames`) covers x, y, w.
pub(super) fn place_field(edit: HWND, x: i32, y: i32, w: i32) {
    place(edit, x + 10, y + 8, w - 20, FRAME_H - 16);
}

/// Disabled edits send WM_CTLCOLORSTATIC; paint them on the field's own
/// `surface` with muted text so a disabled field reads as one box.
pub(super) fn disabled_edit_color(wparam: WPARAM, lparam: LPARAM, edits: &[HWND]) -> Option<LRESULT> {
    if !edits.contains(&(lparam as HWND)) {
        return None;
    }
    let (hdc, p) = (wparam as HDC, palette());
    unsafe {
        SetTextColor(hdc, p.muted);
        SetBkColor(hdc, p.surface);
    }
    Some(theme::surface_brush() as LRESULT)
}

/// Header bar with the window title, and the action bar's top hairline.
pub(super) fn paint_chrome(hdc: HDC, hwnd: HWND, title: &str) {
    page::paint_header(hdc, hwnd, title);
    hline(hdc, 0, view_h() - 56, view_w());
}

/// Small muted hint text, word-wrapped inside `rc`.
pub(super) fn muted(hdc: HDC, s: &str, rc: &RECT) {
    text(hdc, s, rc, fonts().small, palette().muted, DT_WORDBREAK | DT_END_ELLIPSIS);
}

/// Small muted text, word-wrapped and vertically centered in `rc`.
pub(super) fn muted_centered(hdc: HDC, s: &str, rc: &RECT) {
    let font = fonts().small;
    let h = wrapped_height(hdc, s, font, rc.right - rc.left).min(rc.bottom - rc.top);
    let top = rc.top + (rc.bottom - rc.top - h) / 2;
    muted(hdc, s, &RECT { top, bottom: top + h, ..*rc });
}
