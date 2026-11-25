//! The "stop shortcut" row at the bottom of the Trigger tab: a label, the
//! recorded keycaps (or "Not set"), Record and Clear, and a hint line that
//! also shows the recording prompt and errors.

use super::super::theme::palette;
use super::super::*;
use super::layout::{muted, Geo};
use super::record::{start_recording_cancel, stop_recording};
use super::trigger::{self, CANCEL_H, CANCEL_HINT_H};
use super::{ed, layout, content_w, ID_CANCEL_CLEAR, ID_CANCEL_REC, PAD};
use crate::config;
use crate::i18n::t;
use crate::shortcut::Shortcut;
use windows_sys::Win32::Foundation::HWND;

/// Record and Clear, created last on the tab; `place_buttons` moves them.
pub(super) fn create(hwnd: HWND) -> [HWND; 2] {
    let record = button(hwnd, ID_CANCEL_REC, &t("editor.record"), BTN_SECONDARY);
    let clear = button(hwnd, ID_CANCEL_CLEAR, &t("editor.clear"), BTN_SECONDARY);
    add_tooltip(hwnd, record, &t("tip.cancel_shortcut"));
    [record, clear]
}

/// Puts the buttons on the row whose top is `y`.
pub(super) fn place_buttons(buttons: [HWND; 2], g: &Geo, y: i32) {
    for (btn, (x, w)) in buttons.into_iter().zip([g.stop_record, g.stop_clear]) {
        place(btn, x, y, w, CANCEL_H);
    }
}

/// Record / Stop and Clear.
pub(super) fn on_command(id: i32) {
    let recording = ed(|e| e.recording && e.rec_cancel).unwrap_or(false);
    if id == ID_CANCEL_CLEAR {
        if recording {
            stop_recording();
        }
        ed(|e| {
            e.cancel_shortcut = None;
            e.cancel_info = String::new();
        });
    } else if recording {
        stop_recording();
        ed(|e| e.cancel_info = t("editor.record_stopped"));
    } else {
        stop_recording(); // a shortcut recording may be running
        start_recording_cancel();
    }
    layout();
}

/// A recorded key: kept if it can be this binding's stop shortcut.
pub(super) fn set(sc: Shortcut) {
    ed(|e| match config::check_cancel_shortcut(&sc, &e.shortcuts) {
        Ok(()) => {
            e.cancel_shortcut = Some(sc);
            e.cancel_info = String::new();
        }
        Err(key) => e.cancel_info = t(key),
    });
    layout();
}

/// Label, then the live keys / keycaps / "Not set" up to the Record button.
pub(super) fn paint(hdc: HDC, y: i32, newest_alpha: f32) {
    let Some((g, current, live, info, recording)) =
        ed(|e| (e.geo, e.cancel_shortcut.clone(), e.live.clone(), e.cancel_info.clone(), e.recording && e.rec_cancel))
    else {
        return;
    };
    let (p, f) = (palette(), fonts());
    let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
    let label = sentence_case(&t("editor.cancel_shortcut"));
    text(hdc, &label, &r(PAD, y, g.stop_label_w, CANCEL_H), f.small, p.muted, flags);
    let x = PAD + g.stop_label_w + 12;
    let value = r(x, y, g.stop_record.0 - 8 - x, CANCEL_H);
    match &current {
        _ if recording && live.is_empty() => text(hdc, &t("editor.press_keys"), &value, f.bold, p.text, flags),
        _ if recording => trigger::live_keys(hdc, &live, &value, newest_alpha),
        Some(sc) if kit::keycaps_width(hdc, &sc.label()) <= value.right - value.left => {
            kit::keycaps(hdc, &sc.label(), value.left, (value.top + value.bottom) / 2);
        }
        Some(sc) => text(hdc, &sc.label(), &value, f.bold, p.text, flags),
        None => text(hdc, &t("editor.not_set"), &value, f.normal, p.muted, flags),
    }
    let hint = match info {
        info if !info.is_empty() => info,
        _ if recording => t("editor.prompt_cancel"),
        _ => t("editor.cancel_hint"),
    };
    muted(hdc, &hint, &r(PAD, y + CANCEL_H, content_w(), CANCEL_HINT_H));
}
