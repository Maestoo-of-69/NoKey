//! Trigger tab of the binding editor: recorded shortcuts as keycaps, the
//! recording status, the trigger mode with repeat fields, and the cooldown.
//! The rows below the shortcuts follow the number of shortcut rows, so an
//! empty list leaves no gap.

use super::super::theme::palette;
use super::super::*;
use super::layout::{muted, place_field, Geo, TOGGLE_EXTRA};
use super::{cancel, ed, CAP, content_w, FRAME_H, PAD, TOP};
use super::{ID_COOLDOWN, ID_INHERIT, ID_REC_MODE, ID_RECORD, ID_REMOVE_SC, ID_REPEAT_INTERVAL, ID_REPEAT_MAX, ID_TRIGGER};
use crate::app;
use crate::config::TriggerMode;
use crate::i18n::{t, tf};
use crate::shortcut::Shortcut;
use crate::win::get_text;
use windows_sys::Win32::Foundation::HWND;

/// First shortcut row; rows are 32 tall, 36 apart (at most three).
const Y_SHORTCUTS: i32 = 140;
const ROW_STEP: i32 = 36;
/// Live keys (24) + two lines of info while recording; three lines otherwise.
const STATUS_H: i32 = 56;
const HINT_H: i32 = 32;
/// The stop-shortcut row (`editor/cancel.rs`) and its one-line hint.
pub(super) const CANCEL_H: i32 = 32;
pub(super) const CANCEL_HINT_H: i32 = 16;
const COOLDOWN_W: i32 = 90;
/// Animation slot (editor window): fade-in of the newest live key.
pub(super) const SLOT_LIVE_KEY: u32 = 1;

/// Tops (96-DPI units) of the rows under the shortcuts: the status block,
/// the trigger-mode row and the cooldown row (field-frame tops), and the
/// stop-shortcut row.
struct Rows {
    status: i32,
    status_h: i32,
    trigger: i32,
    cooldown: i32,
    cancel: i32,
}

/// The gaps shrink as shortcut rows are added, so everything ends above the
/// action bar (564). With three shortcuts nothing more can be recorded, so
/// the status block only has to hold two lines of info.
fn rows(shortcuts: usize) -> Rows {
    let n = shortcuts.max(1) as i32; // the "no shortcut" line takes one row
    let (status_h, trigger_gap, cooldown_gap) = match n {
        1 => (STATUS_H, 16, 24),
        2 => (STATUS_H, 16, 16),
        _ => (32, 16, 8),
    };
    let status = Y_SHORTCUTS + n * ROW_STEP - 4 + 8;
    let trigger = status + status_h + trigger_gap + CAP;
    let cooldown = trigger + FRAME_H + 4 + HINT_H + cooldown_gap + CAP;
    let cancel = cooldown + FRAME_H + 4 + HINT_H + 8;
    Rows { status, status_h, trigger, cooldown, cancel }
}

/// Trigger tab controls, created in visual (= Tab key) order. Mode and
/// Record share the "Shortcuts" caption row; the rows below are placed by
/// `place_rows`.
pub(super) fn create(hwnd: HWND, g: &Geo) -> ([HWND; 7], [HWND; 3]) {
    let top = TOP - 8;
    let rec_mode_btn = button(hwnd, ID_REC_MODE, "", BTN_DROPDOWN);
    place(rec_mode_btn, g.mode.0, top, g.mode.1, 32);
    let record_btn = button(hwnd, ID_RECORD, &t("editor.record"), BTN_SECONDARY);
    place(record_btn, g.record.0, top, g.record.1, 32);
    let remove_btns: [HWND; 3] = std::array::from_fn(|i| {
        let btn = button(hwnd, ID_REMOVE_SC + i as i32, &t("editor.remove"), BTN_SECONDARY);
        place(btn, g.remove.0, Y_SHORTCUTS + ROW_STEP * i as i32, g.remove.1, 32);
        btn
    });
    let trigger_btn = button(hwnd, ID_TRIGGER, "", BTN_DROPDOWN);
    let repeat_interval = edit(hwnd, ID_REPEAT_INTERVAL, 0);
    let repeat_max = edit(hwnd, ID_REPEAT_MAX, ES_NUMBER as u32);
    let t_inherit = button(hwnd, ID_INHERIT, "", BTN_TOGGLE);
    let cooldown = edit(hwnd, ID_COOLDOWN, 0);
    let tips = [
        (trigger_btn, "tip.trigger"),
        (repeat_interval, "tip.repeat_interval"),
        (repeat_max, "tip.repeat_max"),
        (t_inherit, "tip.cooldown"),
        (cooldown, "tip.cooldown"),
    ];
    for (ctrl, key) in tips {
        add_tooltip(hwnd, ctrl, &t(key));
    }
    ([rec_mode_btn, record_btn, trigger_btn, repeat_interval, repeat_max, t_inherit, cooldown], remove_btns)
}

/// Moves the trigger and cooldown rows under the shortcut rows. Runs only
/// when the row count changed (or after `load` reset it), so nothing moves
/// while typing or recording.
pub(super) fn place_rows(shortcuts: usize) {
    let n = shortcuts.max(1);
    let Some((hwnd, g, ctrls, stop_btns)) = ed(|e| {
        if e.placed_rows == n {
            return None;
        }
        e.placed_rows = n;
        let ctrls = [e.trigger_btn, e.repeat_interval, e.repeat_max, e.t_inherit, e.cooldown];
        Some((e.hwnd, e.geo, ctrls, e.cancel_btns))
    })
    .flatten() else {
        return;
    };
    let [trigger_btn, repeat_interval, repeat_max, inherit, cooldown] = ctrls;
    let y = rows(n);
    cancel::place_buttons(stop_btns, &g, y.cancel);
    place(trigger_btn, PAD, y.trigger + 2, g.trigger_w, 32);
    let [(rx, rw), (mx, mw)] = g.repeat;
    place_field(repeat_interval, rx, y.trigger, rw);
    place_field(repeat_max, mx, y.trigger, mw);
    // The checkbox label holds the global cooldown; the field follows it.
    let label_w = measure_text(hwnd, &get_text(inherit), fonts().normal);
    let w = (label_w + TOGGLE_EXTRA).min(content_w() - COOLDOWN_W - 16);
    place(inherit, PAD, y.cooldown + 4, w, 28);
    place_field(cooldown, PAD + w + 16, y.cooldown, COOLDOWN_W);
}

fn trigger_hint(mode: TriggerMode) -> String {
    let (double_tap, long_press) = app::with(|a| (a.config.double_tap_ms, a.config.long_press_ms));
    match mode {
        TriggerMode::Press => t("editor.hint_press"),
        TriggerMode::Release => t("editor.hint_release"),
        TriggerMode::DoubleTap => tf("editor.hint_double_tap", &[("ms", &double_tap.to_string())]),
        TriggerMode::LongPress => tf("editor.hint_long_press", &[("ms", &long_press.to_string())]),
        TriggerMode::Repeat => t("editor.hint_repeat"),
        TriggerMode::Toggle => t("editor.hint_toggle"),
    }
}

fn kind_key(sc: &Shortcut) -> &'static str {
    match sc {
        Shortcut::Standard { .. } => "editor.mode_standard",
        Shortcut::Combo { .. } => "editor.mode_combo",
        Shortcut::Sequence { .. } => "editor.mode_sequence",
    }
}

/// One recorded shortcut: keycaps, then its kind in muted text. Falls back
/// to plain text when the keycaps would not fit.
fn shortcut_row(hdc: HDC, sc: &Shortcut, row: &RECT) {
    let p = palette();
    let f = fonts();
    let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
    round_rect(hdc, row, p.border, Some(p.surface), 8);
    let inner = RECT { left: row.left + px(12), right: row.right - px(12), ..*row };
    let label = sc.label();
    let kind = t(kind_key(sc));
    if kit::keycaps_width(hdc, &label) > inner.right - inner.left {
        text(hdc, &format!("{label}  ·  {kind}"), &inner, f.bold, p.text, flags);
        return;
    }
    let right = kit::keycaps(hdc, &label, inner.left, (row.top + row.bottom) / 2);
    text(hdc, &kind, &RECT { left: right + px(12), ..inner }, f.small, p.muted, flags);
}

/// Splits the recorder's live text into its keys and a "more to come" flag.
fn split_live(live: &str) -> (&str, bool) {
    match live.strip_suffix(" + …").or_else(|| live.strip_suffix(" → …")) {
        Some(k) => (k, true),
        None => (live, false),
    }
}

/// Number of keys pressed so far in the recorder's live text.
pub(super) fn live_key_count(live: &str) -> usize {
    kit::key_count(split_live(live).0)
}

/// Keys pressed so far while recording ("Ctrl + …" draws Ctrl and a muted …).
/// The newest key fades in while `SLOT_LIVE_KEY` runs.
pub(super) fn live_keys(hdc: HDC, live: &str, rc: &RECT, newest_alpha: f32) {
    let p = palette();
    let f = fonts();
    let flags = DT_SINGLELINE | DT_VCENTER;
    let (keys, more) = split_live(live);
    if kit::keycaps_width(hdc, keys) + px(24) > rc.right - rc.left {
        text(hdc, live, rc, f.bold, p.text, flags | DT_END_ELLIPSIS);
        return;
    }
    let style = kit::KeyStyle { last_alpha: Some(newest_alpha), ..kit::KeyStyle::default() };
    let right = kit::keycaps_ex(hdc, keys, rc.left, (rc.top + rc.bottom) / 2, &style);
    if more {
        text(hdc, "…", &RECT { left: right + px(6), ..*rc }, f.bold, p.muted, flags);
    }
}

/// Live keys while recording, then the recording / conflict info.
fn paint_status(hdc: HDC, (y, h): (i32, i32), live: &str, info: &str, recording: bool, newest_alpha: f32) {
    let mut top = y;
    if recording {
        let rc = r(PAD, top, content_w(), 24);
        if live.is_empty() {
            let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
            text(hdc, &t("editor.press_keys"), &rc, fonts().bold, palette().text, flags);
        } else {
            live_keys(hdc, live, &rc, newest_alpha);
        }
        top += 24;
    }
    muted(hdc, info, &r(PAD, top, content_w(), y + h - top));
}

pub(super) fn paint(hdc: HDC) {
    let Some((hwnd, g, shortcuts, live, info, recording, trigger)) =
        ed(|e| (e.hwnd, e.geo, e.shortcuts.clone(), e.live.clone(), e.info.clone(), e.recording && !e.rec_cancel, e.trigger))
    else {
        return;
    };
    let newest_alpha = anim::value(hwnd, SLOT_LIVE_KEY, 1.0);
    let y = rows(shortcuts.len());
    caption(hdc, &t("editor.shortcuts"), PAD, TOP, g.mode.0 - PAD - 12);
    if shortcuts.is_empty() {
        let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
        text(hdc, &t("editor.no_shortcuts"), &r(PAD, Y_SHORTCUTS, content_w(), 32), fonts().normal, palette().muted, flags);
    }
    for (i, sc) in shortcuts.iter().enumerate() {
        shortcut_row(hdc, sc, &r(PAD, Y_SHORTCUTS + ROW_STEP * i as i32, g.remove.0 - 8 - PAD, 32));
    }
    paint_status(hdc, (y.status, y.status_h), &live, &info, recording, newest_alpha);
    caption(hdc, &t("editor.trigger"), PAD, y.trigger - CAP, g.trigger_w);
    if trigger.repeats() {
        for ((x, w), key) in g.repeat.into_iter().zip(["editor.repeat_interval", "editor.repeat_max"]) {
            caption(hdc, &t(key), x, y.trigger - CAP, w);
        }
    }
    muted(hdc, &trigger_hint(trigger), &r(PAD, y.trigger + FRAME_H + 4, content_w(), HINT_H));
    caption(hdc, &t("editor.cooldown"), PAD, y.cooldown - CAP, content_w());
    muted(hdc, &t("editor.cooldown_hint"), &r(PAD, y.cooldown + FRAME_H + 4, content_w(), HINT_H));
    cancel::paint(hdc, y.cancel, newest_alpha);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_fit_above_the_action_bar() {
        for shortcuts in 0..=3 {
            let y = rows(shortcuts);
            assert!(y.cancel + CANCEL_H + CANCEL_HINT_H <= 556, "{shortcuts} shortcuts");
            assert!(y.trigger - CAP >= y.status + y.status_h, "{shortcuts}: status above the trigger caption");
            assert!(y.cooldown - CAP >= y.trigger + FRAME_H + 4 + HINT_H);
            assert!(y.cancel >= y.cooldown + FRAME_H + 4 + HINT_H);
        }
    }
}
