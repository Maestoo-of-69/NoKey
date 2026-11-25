//! Fields of a "Type text" step on the Actions tab: a multi-line text box,
//! the "erase the shortcut" checkbox beside the character counter, the speed
//! slider with its live value, and the "type like a human" checkbox with a
//! hint. Shown instead of the generic single-line fields when the selected
//! step is TypeText (see steps.rs).

use super::super::theme::palette;
use super::super::*;
use super::layout::{muted, muted_centered, Measure, TOGGLE_EXTRA};
use super::steps::{FIELDS_Y, LABEL_W};
use super::{content_w, view_h, view_w, FRAME_H, PAD};
use crate::config::type_text::{char_count, normalize_newlines};
use crate::config::{Action, DEFAULT_CPM, MAX_TYPE_CHARS, TYPE_CPM_RANGE};
use crate::i18n::{t, tf};
use crate::win::{get_text, set_text};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};

pub(super) const ID_TEXT: i32 = 392;
pub(super) const ID_SPEED: i32 = 393;
pub(super) const ID_HUMAN: i32 = 394;
pub(super) const ID_ERASE: i32 = 395;
/// Room kept for the counter at the right end of the erase checkbox's row.
const COUNTER_W: i32 = 96;
const CUE_SUBCLASS: usize = 0x4E4B_0003;
/// Content must end above the action bar (it starts at 564).
fn bottom() -> i32 {
    view_h() - 64
}
const ROW_H: i32 = 28;
const LINE: i32 = 16;
/// The checkbox's label starts this far right of its box.
const TOGGLE_INDENT: i32 = 26;
/// Paste guard in UTF-16 units; the real limit (10 000 characters) is
/// checked on save and shown by the counter.
const EDIT_LIMIT: usize = 2 * MAX_TYPE_CHARS;

#[derive(Clone, Copy)]
pub(in crate::ui) struct TypeCtl {
    pub text: HWND,
    pub erase: HWND,
    pub speed: HWND,
    pub human: HWND,
}

impl TypeCtl {
    pub fn all(&self) -> [HWND; 4] {
        [self.text, self.erase, self.speed, self.human]
    }
}

/// Vertical layout (96-DPI units): frame tops and heights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rows {
    pub text_h: i32,
    pub counter: i32,
    pub speed: i32,
    pub human: i32,
    pub hint: i32,
    pub hint_h: i32,
}

/// Rows for a hint `hint_h` tall: the text box takes what is left above
/// the action bar.
pub(super) fn rows(hint_h: i32) -> Rows {
    let below = 4 + ROW_H + ROW_H + 2 + ROW_H + hint_h;
    let text_h = bottom() - FIELDS_Y - below;
    let counter = FIELDS_Y + text_h + 4;
    let speed = counter + ROW_H;
    let human = speed + ROW_H + 2;
    Rows { text_h, counter, speed, human, hint: human + ROW_H, hint_h }
}

/// Everything measured: label column, value label width, rows.
#[derive(Clone, Copy)]
struct Geo {
    label_w: i32,
    value_w: i32,
    toggle_w: i32,
    erase_w: i32,
    rows: Rows,
}

fn speed_label(cpm: u32) -> String {
    tf("field.cpm_value", &[("cpm", &cpm.to_string())])
}

fn geometry() -> Geo {
    let m = Measure::window(super::hwnd());
    let small = fonts().small;
    let names = [t("field.text"), t("field.speed")];
    let label_w = (m.widest(&names, small) + 4).clamp(LABEL_W.0, LABEL_W.1);
    let value_w = m.widest(&[speed_label(TYPE_CPM_RANGE.1), speed_label(TYPE_CPM_RANGE.0)], small) + 4;
    let toggle_w = (m.width(&t("field.human"), fonts().normal) + TOGGLE_EXTRA).min(content_w());
    let erase_w = (m.width(&t("field.erase_trigger"), fonts().normal) + TOGGLE_EXTRA).min(content_w() - COUNTER_W);
    // The hint gets one or two lines; the text box shrinks for the second.
    let hint_h = m.height(&t("hint.type_text"), small, content_w() - TOGGLE_INDENT).clamp(LINE, 2 * LINE);
    Geo { label_w, value_w, toggle_w, erase_w, rows: rows(hint_h) }
}

// ---------- cue text (EM_SETCUEBANNER refuses multi-line edits) ----------

fn draw_cue(edit: HWND) {
    if unsafe { GetWindowTextLengthW(edit) } > 0 {
        return;
    }
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe {
        SendMessageW(edit, EM_GETRECT, 0, &mut rc as *mut RECT as LPARAM);
        let hdc = GetDC(edit);
        text(hdc, &t("hint.type_text_cue"), &rc, fonts().normal, palette().muted, DT_WORDBREAK);
        ReleaseDC(edit, hdc);
    }
}

unsafe extern "system" fn cue_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    let r = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    match msg {
        WM_PAINT => draw_cue(hwnd),
        WM_SETFOCUS | WM_KILLFOCUS => unsafe {
            InvalidateRect(hwnd, null(), 1);
        },
        _ => {}
    }
    r
}

// ---------- creation, loading, reading ----------

pub(super) fn create(hwnd: HWND) -> TypeCtl {
    let text = edit_multiline(hwnd, ID_TEXT);
    unsafe {
        SendMessageW(text, EM_SETLIMITTEXT, EDIT_LIMIT, 0);
        SetWindowSubclass(text, Some(cue_proc), CUE_SUBCLASS, 0);
    }
    // Created in visual (= Tab key) order.
    let erase = button(hwnd, ID_ERASE, &t("field.erase_trigger"), BTN_TOGGLE);
    let speed = kit::slider(hwnd, ID_SPEED, TYPE_CPM_RANGE, DEFAULT_CPM);
    let human = button(hwnd, ID_HUMAN, &t("field.human"), BTN_TOGGLE);
    add_tooltip(hwnd, erase, &t("tip.type_erase"));
    add_tooltip(hwnd, speed, &t("tip.type_speed"));
    add_tooltip(hwnd, human, &t("tip.type_human"));
    TypeCtl { text, erase, speed, human }
}

/// Fills the controls from a TypeText step (caller marks "loading").
pub(super) fn load(ctl: &TypeCtl, action: &Action) {
    if let Action::TypeText { text, cpm, human, erase_trigger } = action {
        set_text(ctl.text, &normalize_newlines(text).replace('\n', "\r\n"));
        set_checked(ctl.erase, *erase_trigger);
        kit::set_slider_value(ctl.speed, *cpm);
        set_checked(ctl.human, *human);
    }
}

/// The step as the controls show it. The text is not trimmed: spaces and
/// line breaks are typed as they are.
pub(super) fn read(ctl: &TypeCtl) -> Action {
    Action::TypeText {
        text: normalize_newlines(&get_text(ctl.text)),
        cpm: kit::slider_value(ctl.speed),
        human: is_checked(ctl.human),
        erase_trigger: is_checked(ctl.erase),
    }
}

/// Shows and places the controls (or hides them for other kinds).
pub(super) fn place_controls(ctl: &TypeCtl, visible: bool) {
    for c in ctl.all() {
        show(c, visible);
    }
    if !visible {
        return;
    }
    let g = geometry();
    let fx = PAD + g.label_w + 12;
    place(ctl.text, fx + 10, FIELDS_Y + 8, view_w() - PAD - fx - 20, g.rows.text_h - 16);
    place(ctl.erase, PAD, g.rows.counter, g.erase_w, ROW_H);
    let speed_x = fx - kit::END_PAD; // the rail starts under the text box
    place(ctl.speed, speed_x, g.rows.speed, view_w() - PAD - g.value_w - 8 - speed_x, ROW_H);
    place(ctl.human, PAD, g.rows.human, g.toggle_w, ROW_H);
}

/// The text changed: repaint the cue when the box became (or stopped
/// being) empty, since the edit draws typed text without WM_PAINT.
pub(super) fn on_text_change(ctl: &TypeCtl) {
    if unsafe { GetWindowTextLengthW(ctl.text) } <= 1 {
        unsafe { InvalidateRect(ctl.text, null(), 1) };
    }
}

pub(super) fn paint(hdc: HDC, ctl: &TypeCtl) {
    let (g, p) = (geometry(), palette());
    muted_centered(hdc, &t("field.text"), &r(PAD, FIELDS_Y, g.label_w, FRAME_H));
    let count = char_count(&get_text(ctl.text));
    let color = if count > MAX_TYPE_CHARS { p.err } else { p.muted };
    let counter = format!("{count} / {MAX_TYPE_CHARS}");
    let cr = r(view_w() - PAD - COUNTER_W, g.rows.counter, COUNTER_W, ROW_H);
    text(hdc, &counter, &cr, fonts().small, color, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
    muted_centered(hdc, &t("field.speed"), &r(PAD, g.rows.speed, g.label_w, ROW_H));
    let value = speed_label(kit::slider_value(ctl.speed));
    let focus = unsafe { GetFocus() } == ctl.speed;
    let value_color = if focus { p.text } else { p.muted };
    let vr = r(view_w() - PAD - g.value_w, g.rows.speed, g.value_w, ROW_H);
    text(hdc, &value, &vr, fonts().small, value_color, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
    let hint_rc = r(PAD + TOGGLE_INDENT, g.rows.hint, content_w() - TOGGLE_INDENT, g.rows.hint_h);
    muted(hdc, &t("hint.type_text"), &hint_rc);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_fit_above_the_action_bar() {
        for hint_h in [LINE, 2 * LINE] {
            let rows = rows(hint_h);
            assert_eq!(rows.hint + rows.hint_h, bottom(), "hint {hint_h}: ends exactly at the budget");
            assert!(rows.counter >= FIELDS_Y + rows.text_h);
            assert!(rows.speed >= rows.counter + ROW_H && rows.human >= rows.speed + ROW_H);
        }
        // About four lines of 14pt text (19 px each) inside the 8px frame padding.
        assert!(rows(2 * LINE).text_h - 16 >= 3 * 19, "still three lines with a two-line hint");
        assert!(rows(LINE).text_h - 16 >= 4 * 19);
    }
}
