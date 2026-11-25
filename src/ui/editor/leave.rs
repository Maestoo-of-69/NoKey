//! Leaving the editor: what was loaded is remembered, so the "NoKey" button
//! (or opening another page) asks before dropping edits. Cancel and Esc
//! close without asking, as before.

use super::super::*;
use super::{close, ed, steps, stop_recording};
use crate::i18n::t;
use crate::win::get_text;

/// Everything Save would read, as one comparable string.
fn snapshot() -> String {
    steps::commit();
    let options = steps::read_options().ok();
    let Some((edits, toggles, rest)) = ed(|e| {
        let edits =
            [e.name, e.description, e.icon, e.priority, e.repeat_interval, e.repeat_max, e.cooldown, e.process, e.title];
        let toggles = [e.t_enabled, e.t_sound, e.t_notify, e.t_inherit, e.t_fullscreen];
        let rest = format!("{:?}", (&e.shortcuts, &e.cancel_shortcut, e.trigger, e.profile, &e.steps.actions));
        (edits, toggles, rest)
    }) else {
        return String::new();
    };
    format!("{:?}{:?}{options:?}{rest}", edits.map(get_text), toggles.map(is_checked))
}

pub(super) fn remember() {
    let loaded = snapshot();
    ed(|e| e.loaded = loaded);
}

/// Closes the editor unless the user wants to keep unsaved changes.
/// True when no editor is open afterwards.
pub fn try_close() -> bool {
    let Some(hwnd) = ed(|e| e.hwnd) else { return true };
    stop_recording(); // its hook would swallow the dialog's keys
    let now = snapshot();
    if ed(|e| e.loaded != now).unwrap_or(false) {
        super::super::main_window::show_main();
        if !confirm(hwnd, &t("editor.confirm_discard")) {
            return false;
        }
    }
    close();
    true
}
