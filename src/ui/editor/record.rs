//! Shortcut recording in the binding editor: start/stop, conflict info and
//! the recorder's progress updates (the newest key fades in, Motion #16).

use super::*;

/// Records a shortcut of the type chosen in the dropdown.
pub(super) fn start_recording() {
    let Some(mode) = ed(|e| e.mode) else { return };
    begin(mode, false);
}

/// Records the stop shortcut (always a standard one).
pub(super) fn start_recording_cancel() {
    begin(RecordMode::Standard, true);
}

fn begin(mode: RecordMode, rec_cancel: bool) {
    match hook::start_recording(Recorder::new(mode)) {
        Ok(()) => {
            let prompt = t(match mode {
                RecordMode::Standard => "editor.prompt_standard",
                RecordMode::Combo => "editor.prompt_combo",
                RecordMode::Sequence => "editor.prompt_sequence",
            });
            let hwnd = ed(|e| {
                e.recording = true;
                e.rec_cancel = rec_cancel;
                e.live = String::new();
                if rec_cancel {
                    e.cancel_info = String::new(); // the row shows its own prompt
                } else {
                    e.info = prompt;
                }
                e.hwnd
            });
            if let Some(h) = hwnd {
                unsafe {
                    SetFocus(h); // outside the borrow: SetFocus sends EN_KILLFOCUS synchronously
                    SetTimer(h, TIMER_RECORD_LIMIT, RECORD_LIMIT_MS, None);
                }
            }
        }
        Err(err) => {
            ed(|e| if rec_cancel { e.cancel_info = err } else { e.info = err });
        }
    }
    layout();
}

pub(super) fn stop_recording() -> Option<RecordUpdate> {
    let (was, hwnd) = ed(|e| (std::mem::replace(&mut e.recording, false), e.hwnd)).unwrap_or((false, null_mut()));
    if !was {
        return None;
    }
    unsafe { KillTimer(hwnd, TIMER_RECORD_LIMIT) };
    let result = hook::stop_recording();
    layout();
    result
}

/// Explains how the shortcut behaves, or what it conflicts with.
pub(super) fn describe(sc: &Shortcut, self_id: u32, profile: u32) -> String {
    let conflicts = app::with(|a| a.config.conflicts_with(sc, Some(self_id), profile));
    if !conflicts.is_empty() {
        tf("editor.conflict", &[("details", &conflicts.join("; "))])
    } else if sc.needs_hook() {
        t("editor.uses_hook")
    } else {
        t("editor.works_everywhere")
    }
}

pub(super) fn add_shortcut(sc: Shortcut) {
    let Some((id, profile, existing)) = ed(|e| (e.editing.unwrap_or(0), e.profile, e.shortcuts.clone())) else { return };
    if let Err(err) = config::check_shortcut(&sc) {
        ed(|e| e.info = err);
        layout();
        return;
    }
    if existing.contains(&sc) {
        ed(|e| e.info = t("editor.already_added"));
        layout();
        return;
    }
    let info = describe(&sc, id, profile);
    ed(|e| {
        if e.shortcuts.len() < MAX_SHORTCUTS {
            e.shortcuts.push(sc);
        }
        e.info = info;
    });
    layout();
}

pub fn on_record(update: RecordUpdate) {
    let Some((mode, recording, rec_cancel)) = ed(|e| (e.mode, e.recording, e.rec_cancel)) else {
        hook::stop_recording();
        return;
    };
    if !recording {
        return; // a late update from a recording that already ended
    }
    // The stop row shows its own result lines.
    let report = |msg: String| {
        ed(|e| if rec_cancel { e.cancel_info = msg } else { e.info = msg });
    };
    match update {
        RecordUpdate::Progress(s) => {
            let added = trigger::live_key_count(&s);
            if let Some((hwnd, before)) = ed(|e| (e.hwnd, trigger::live_key_count(&std::mem::replace(&mut e.live, s))))
                && added > before
            {
                anim::start(hwnd, trigger::SLOT_LIVE_KEY, 0.0, 1.0, kit::KEY_FADE_MS);
            }
            layout();
        }
        RecordUpdate::Done(sc) => {
            stop_recording();
            if rec_cancel { cancel::set(sc) } else { add_shortcut(sc) }
        }
        RecordUpdate::Cancelled => {
            stop_recording();
            report(t("editor.record_cancelled"));
            layout();
        }
        RecordUpdate::Rejected(msg) => {
            if mode == RecordMode::Sequence && !rec_cancel {
                stop_recording();
            }
            report(msg);
            ed(|e| e.live = String::new());
            layout();
        }
    }
}
