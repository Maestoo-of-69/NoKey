//! Binding editor with four tabs: General, Trigger, Actions, Conditions.
//! The Trigger tab lives in `editor/trigger.rs`, the Actions tab in
//! `editor/steps.rs`, recording in `editor/record.rs`, shared measuring and
//! drawing in `editor/layout.rs`. The tab underline slides after a click.

mod cancel;
mod layout;
mod leave;
mod record;
mod steps;
mod trigger;
mod type_text;

pub use leave::try_close;
pub use record::on_record;
use record::{add_shortcut, describe, start_recording, stop_recording};

use super::*;
use crate::app::{self, profile_name};
use crate::config::{
    self, timing, Binding, Conditions, Trigger, TriggerMode, ALL_PROFILES, COOLDOWN_RANGE_MS, MAX_BINDING_ID,
    MAX_PRIORITY, MAX_SHORTCUTS, REPEAT_INTERVAL_RANGE, REPEAT_MAX_RANGE,
};
use crate::discord::api::ApiError;
use crate::discord::models::{Channel, Guild, User};
use crate::engine::recorder::{RecordMode, RecordUpdate, Recorder};
use crate::i18n::{self, t, tf};
use crate::shortcut::Shortcut;
use crate::win::{get_text, hook, set_text};
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;

const CLASS: &str = "NoKeyEditor";
fn view_w() -> i32 {
    page::width()
}
fn view_h() -> i32 {
    page::height()
}
// Layout in 96-DPI units (docs/DESIGN.md): header 52, tabs 36, action bar 56,
// field frames 36, captions 24 above their field. Rows are frame tops.
const PAD: i32 = 16;
fn content_w() -> i32 {
    view_w() - 2 * PAD
}
const TOP: i32 = 108;
const CAP: i32 = 24;
const FRAME_H: i32 = 36;
const Y_NAME: i32 = 132;
const Y_DESC: i32 = 216;
const Y_ICON: i32 = 300;
const ICON_W: i32 = 80;
const PROFILE_X: i32 = PAD + ICON_W + 16;
const Y_TOGGLES: i32 = 360; // three 28-tall checkboxes, 32 apart
const Y_PRIORITY: i32 = 500;
const PRIORITY_W: i32 = 56;
const Y_PROCESS: i32 = 204;
const Y_TITLE: i32 = 328;
const Y_FULLSCREEN: i32 = 428;

pub(super) const TAB_GENERAL: usize = 0;
pub(super) const TAB_TRIGGER: usize = 1;
pub(super) const TAB_ACTIONS: usize = 2;
const TAB_KEYS: [&str; 4] = ["editor.tab_general", "editor.tab_trigger", "editor.tab_actions", "editor.tab_conditions"];

const ID_TAB: i32 = 300; // 300..304
const ID_PROFILE: i32 = 313;
const ID_ENABLED: i32 = 314;
const ID_SOUND: i32 = 315;
const ID_NOTIFY: i32 = 316;
const ID_PRIORITY: i32 = 317;
const ID_REMOVE_SC: i32 = 320; // 320..323
const ID_REC_MODE: i32 = 324;
const ID_RECORD: i32 = 325;
const ID_TRIGGER: i32 = 326;
const ID_REPEAT_INTERVAL: i32 = 327;
const ID_REPEAT_MAX: i32 = 328;
const ID_INHERIT: i32 = 329;
const ID_COOLDOWN: i32 = 330;
const ID_CANCEL_REC: i32 = 332;
const ID_CANCEL_CLEAR: i32 = 333;
const ID_NAME: i32 = 310;
const ID_DESCRIPTION: i32 = 311;
const ID_ICON: i32 = 312;
const ID_PROCESS: i32 = 340;
const ID_TITLE: i32 = 341;
const ID_FULLSCREEN: i32 = 342;
const ID_SAVE: i32 = 350;
const ID_CANCEL: i32 = 351;
const IDCANCEL_KEY: i32 = 2; // Esc through IsDialogMessage
/// Recording swallows keys, so it ends on its own if left running.
const TIMER_RECORD_LIMIT: usize = 1;
const RECORD_LIMIT_MS: u32 = 30_000;

const MODES: [(RecordMode, &str); 3] = [
    (RecordMode::Standard, "editor.mode_standard"),
    (RecordMode::Combo, "editor.mode_combo"),
    (RecordMode::Sequence, "editor.mode_sequence"),
];

pub(super) struct Ed {
    hwnd: HWND,
    /// What was loaded (see leave.rs).
    loaded: String,
    editing: Option<u32>,
    tab: usize,
    tabs: [HWND; 4],
    groups: [Vec<HWND>; 4],
    edits: Vec<HWND>,
    // General
    name: HWND,
    description: HWND,
    icon: HWND,
    profile_btn: HWND,
    profile: u32,
    t_enabled: HWND,
    t_sound: HWND,
    t_notify: HWND,
    priority: HWND,
    // Trigger
    shortcuts: Vec<Shortcut>,
    remove_btns: [HWND; 3],
    rec_mode_btn: HWND,
    record_btn: HWND,
    mode: RecordMode,
    recording: bool,
    live: String,
    info: String,
    trigger_btn: HWND,
    trigger: TriggerMode,
    repeat_interval: HWND,
    repeat_max: HWND,
    t_inherit: HWND,
    cooldown: HWND,
    // Stop shortcut (see cancel.rs)
    cancel_shortcut: Option<Shortcut>,
    cancel_btns: [HWND; 2],
    /// The running recording is for the stop shortcut.
    rec_cancel: bool,
    /// Error or result under the stop row; empty shows the hint.
    cancel_info: String,
    // Actions (see steps.rs)
    pub(super) steps: steps::Steps,
    // Conditions
    process: HWND,
    title: HWND,
    t_fullscreen: HWND,
    geo: layout::Geo,
    /// Shortcut rows the Trigger tab's lower rows were placed for (0 = not yet).
    placed_rows: usize,
}

thread_local! {
    static ED: RefCell<Option<Ed>> = const { RefCell::new(None) };
}

pub(super) fn ed<R>(f: impl FnOnce(&mut Ed) -> R) -> Option<R> {
    ED.with(|e| e.try_borrow_mut().ok()?.as_mut().map(f))
}

pub fn register(icon: HICON) {
    register_class(CLASS, Some(wndproc), icon);
}

pub fn hwnd() -> HWND {
    ed(|e| e.hwnd).unwrap_or(null_mut())
}

pub fn open(main: HWND, binding: Option<Binding>) {
    if ed(|e| e.hwnd).is_some() {
        main_window::show_main();
        return;
    }
    if !page::go_home() {
        return;
    }
    main_window::show_main();
    let hwnd = page::create(CLASS, main);
    if hwnd.is_null() {
        return;
    }
    load(binding);
    leave::remember();
    page::show_page(hwnd);
    if let Some(name) = ed(|e| e.name) {
        unsafe { SetFocus(name) };
    }
}

fn comma() -> bool {
    i18n::language().decimal_comma()
}

/// Places a control (96-DPI units) and returns it.
fn placed(ctrl: HWND, x: i32, y: i32, w: i32, h: i32) -> HWND {
    place(ctrl, x, y, w, h);
    ctrl
}

fn field_at(hwnd: HWND, id: i32, style: u32, x: i32, y: i32, w: i32) -> HWND {
    let e = edit(hwnd, id, style);
    layout::place_field(e, x, y, w);
    e
}

fn create_general(hwnd: HWND) -> [HWND; 8] {
    let check = |id, key: &str, y| placed(button(hwnd, id, &t(key), BTN_TOGGLE), PAD, y, content_w(), 28);
    let name = field_at(hwnd, ID_NAME, 0, PAD, Y_NAME, content_w());
    set_cue(name, &t("editor.name_cue"));
    let description = field_at(hwnd, ID_DESCRIPTION, 0, PAD, Y_DESC, content_w());
    set_cue(description, &t("editor.description_cue"));
    let icon = field_at(hwnd, ID_ICON, 0, PAD, Y_ICON, ICON_W);
    set_cue(icon, "🔔");
    let profile = placed(button(hwnd, ID_PROFILE, "", BTN_DROPDOWN), PROFILE_X, Y_ICON + 2, 240, 32);
    let enabled = check(ID_ENABLED, "editor.enabled", Y_TOGGLES);
    let sound = check(ID_SOUND, "editor.sound", Y_TOGGLES + 32);
    let notify = check(ID_NOTIFY, "editor.notify", Y_TOGGLES + 64);
    let priority = field_at(hwnd, ID_PRIORITY, ES_NUMBER as u32, PAD, Y_PRIORITY, PRIORITY_W);
    add_tooltip(hwnd, priority, &t("tip.priority"));
    add_tooltip(hwnd, icon, &t("tip.icon"));
    [name, description, icon, profile, enabled, sound, notify, priority]
}

fn create_conditions(hwnd: HWND) -> [HWND; 3] {
    let process = field_at(hwnd, ID_PROCESS, 0, PAD, Y_PROCESS, content_w());
    set_cue(process, "chrome.exe");
    let title = field_at(hwnd, ID_TITLE, 0, PAD, Y_TITLE, content_w());
    set_cue(title, &t("editor.title_cue"));
    let skip = button(hwnd, ID_FULLSCREEN, &t("editor.skip_fullscreen"), BTN_TOGGLE);
    place(skip, PAD, Y_FULLSCREEN, content_w(), 28);
    add_tooltip(hwnd, process, &t("tip.process"));
    add_tooltip(hwnd, title, &t("tip.window_title"));
    add_tooltip(hwnd, skip, &t("tip.fullscreen"));
    [process, title, skip]
}

fn on_create(hwnd: HWND) {
    let g = layout::Geo::measure(hwnd);
    page::home_button(hwnd); // first: it is also first in the Tab order
    let tabs: [HWND; 4] = std::array::from_fn(|i| {
        let (x, w) = g.tabs[i];
        // 34 tall: the 2px strip below is the parent-drawn underline.
        placed(button(hwnd, ID_TAB + i as i32, &t(TAB_KEYS[i]), BTN_TAB), x, 52, w, 34)
    });
    let general = create_general(hwnd);
    let (trigger, remove_btns) = trigger::create(hwnd, &g);
    let cancel_btns = cancel::create(hwnd);
    let (steps, actions_group) = steps::create(hwnd);
    let conditions = create_conditions(hwnd);
    // Action bar: Cancel (ghost) then Save (primary), right-aligned.
    placed(button(hwnd, ID_CANCEL, &t("common.cancel"), BTN_SECONDARY), g.cancel.0, view_h() - 44, g.cancel.1, 32);
    placed(button(hwnd, ID_SAVE, &t("common.save"), BTN_PRIMARY), g.save.0, view_h() - 44, g.save.1, 32);
    let [name, description, icon, profile_btn, t_enabled, t_sound, t_notify, priority] = general;
    let [rec_mode_btn, record_btn, trigger_btn, repeat_interval, repeat_max, t_inherit, cooldown] = trigger;
    let [process, title, t_fullscreen] = conditions;
    let mut edits = vec![name, description, icon, priority, repeat_interval, repeat_max, cooldown, process, title];
    edits.extend(steps.edits());
    let mut trigger_group = trigger.to_vec();
    trigger_group.extend(remove_btns);
    trigger_group.extend(cancel_btns);
    ED.with(|cell| {
        *cell.borrow_mut() = Some(Ed {
            hwnd,
            loaded: String::new(),
            editing: None,
            tab: TAB_GENERAL,
            tabs,
            groups: [general.to_vec(), trigger_group, actions_group, conditions.to_vec()],
            edits,
            name,
            description,
            icon,
            profile_btn,
            profile: ALL_PROFILES,
            t_enabled,
            t_sound,
            t_notify,
            priority,
            shortcuts: Vec::new(),
            remove_btns,
            rec_mode_btn,
            record_btn,
            mode: RecordMode::Standard,
            recording: false,
            live: String::new(),
            info: t("editor.record_hint"),
            trigger_btn,
            trigger: TriggerMode::Press,
            repeat_interval,
            repeat_max,
            t_inherit,
            cooldown,
            cancel_shortcut: None,
            cancel_btns,
            rec_cancel: false,
            cancel_info: String::new(),
            steps,
            process,
            title,
            t_fullscreen,
            geo: g,
            placed_rows: 0,
        })
    });
}

fn load(binding: Option<Binding>) {
    let (active_profile, global_cd) = app::with(|a| (a.config.active_profile, a.config.cooldown_ms));
    let b = binding.clone().unwrap_or_else(|| {
        let mut nb = Binding::new(0, active_profile);
        nb.actions = vec![crate::config::ActionKind::DiscordPing.empty()];
        nb
    });
    let Some((texts, toggles)) = ed(|e| {
        e.editing = binding.as_ref().map(|x| x.id);
        e.profile = b.profile;
        e.shortcuts = b.shortcuts.clone();
        e.cancel_shortcut = b.cancel_shortcut.clone();
        e.cancel_info = String::new();
        e.trigger = b.trigger.mode;
        e.mode = match b.shortcuts.first() {
            Some(Shortcut::Combo { .. }) => RecordMode::Combo,
            Some(Shortcut::Sequence { .. }) => RecordMode::Sequence,
            _ => RecordMode::Standard,
        };
        (
            [e.name, e.description, e.icon, e.priority, e.repeat_interval, e.repeat_max, e.cooldown, e.process, e.title, e.t_inherit],
            [e.t_enabled, e.t_sound, e.t_notify, e.t_inherit, e.t_fullscreen],
        )
    }) else {
        return;
    };
    let values = [
        b.name.clone(),
        b.description.clone(),
        b.icon.clone(),
        b.priority.to_string(),
        timing::format_seconds(b.trigger.repeat_interval_ms, comma()),
        b.trigger.repeat_max.to_string(),
        timing::format_seconds(b.cooldown_ms.unwrap_or(global_cd), comma()),
        b.conditions.process.clone(),
        b.conditions.window_title.clone(),
        tf("editor.inherit_cooldown", &[("secs", &timing::format_seconds(global_cd, comma()))]),
    ];
    for (h, v) in texts.iter().zip(values) {
        set_text(*h, &v);
    }
    ed(|e| e.placed_rows = 0); // the inherit label changed: re-place the cooldown row
    let flags = [b.enabled, b.sound, b.notify, b.cooldown_ms.is_none(), b.conditions.skip_fullscreen];
    for (h, on) in toggles.iter().zip(flags) {
        set_checked(*h, on);
    }
    steps::load(&b);
    if let Some(sc) = b.shortcuts.first() {
        let info = describe(sc, b.id, b.profile);
        ed(|e| e.info = info);
    }
    layout();
}

/// The window changed size: right-aligned controls move, fields stretch, the
/// action bar follows the bottom edge.
pub fn resize(by: page::Resize) {
    let Some((hwnd, mut right, mut wide)) = ed(|e| {
        let mut right = vec![e.rec_mode_btn, e.record_btn];
        right.extend(e.remove_btns);
        let wide = vec![e.name, e.description, e.t_enabled, e.t_sound, e.t_notify, e.process, e.title, e.t_fullscreen];
        (e.hwnd, right, wide)
    }) else {
        return;
    };
    let bar = [ID_CANCEL, ID_SAVE].map(|id| unsafe { GetDlgItem(hwnd, id) });
    right.extend(bar);
    wide.extend(ed(|e| [e.steps.list(), e.steps.stop_toggle()]).into_iter().flatten());
    page::anchor(hwnd, by, &right, &wide, &bar);
    // Measured positions and the rows placed from them follow the new width.
    let geo = layout::Geo::measure(hwnd);
    ed(|e| {
        e.geo = geo;
        e.placed_rows = 0;
    });
    layout();
}

/// Shows the controls of the current tab and refreshes labels.
pub(super) fn layout() {
    let Some(s) = ed(|e| {
        (
            e.hwnd,
            e.tab,
            e.tabs,
            e.groups.clone(),
            e.shortcuts.len(),
            e.remove_btns,
            (e.rec_mode_btn, e.mode, e.record_btn, e.recording && !e.rec_cancel),
            (e.cancel_btns, e.recording && e.rec_cancel, e.cancel_shortcut.is_some()),
            (e.trigger_btn, e.trigger, e.profile_btn, e.profile),
            (e.t_inherit, e.cooldown, [e.repeat_interval, e.repeat_max]),
        )
    }) else {
        return;
    };
    let (hwnd, tab, tabs, groups, n_sc, removes, rec, stop, trig, cd) = s;
    let (rec_mode_btn, mode, record_btn, recording) = rec;
    let ([stop_record, stop_clear], stop_recording_now, has_stop) = stop;
    let (trigger_btn, trigger, profile_btn, profile) = trig;
    let (inherit, cooldown, rep) = cd;
    for (i, group) in groups.iter().enumerate() {
        set_checked(tabs[i], i == tab);
        for &c in group {
            show(c, i == tab);
        }
    }
    if tab == TAB_TRIGGER {
        trigger::place_rows(n_sc);
        for (i, &b) in removes.iter().enumerate() {
            show(b, i < n_sc);
        }
        for c in rep {
            show(c, trigger.repeats());
        }
    }
    if tab == TAB_ACTIONS {
        steps::layout();
    }
    let mode_key = MODES.iter().find(|(m, _)| *m == mode).map(|(_, k)| *k).unwrap_or("editor.mode_standard");
    set_text(rec_mode_btn, &t(mode_key));
    set_text(record_btn, &t(if recording { "editor.stop" } else { "editor.record" }));
    // One primary per window (Save): Record is ghost, warn-coloured while recording.
    set_kind(record_btn, if recording { BTN_WARN } else { BTN_SECONDARY });
    kit::set_pulsing(record_btn, recording);
    enable(record_btn, recording || n_sc < MAX_SHORTCUTS);
    set_text(stop_record, &t(if stop_recording_now { "editor.stop" } else { "editor.record" }));
    set_kind(stop_record, if stop_recording_now { BTN_WARN } else { BTN_SECONDARY });
    kit::set_pulsing(stop_record, stop_recording_now);
    enable(stop_clear, has_stop || stop_recording_now);
    set_text(trigger_btn, &t(trigger.key()));
    let pname = app::with(|a| profile_name(&a.config, profile));
    set_text(profile_btn, &pname);
    enable(cooldown, !is_checked(inherit));
    invalidate(hwnd);
}

fn paint_general(hdc: HDC) {
    caption(hdc, &t("editor.name"), PAD, Y_NAME - CAP, content_w());
    caption(hdc, &t("editor.description"), PAD, Y_DESC - CAP, content_w());
    caption(hdc, &t("editor.icon"), PAD, Y_ICON - CAP, ICON_W);
    caption(hdc, &t("editor.profile"), PROFILE_X, Y_ICON - CAP, 240);
    caption(hdc, &t("editor.priority"), PAD, Y_PRIORITY - CAP, content_w());
    let x = PAD + PRIORITY_W + 12;
    layout::muted_centered(hdc, &t("editor.priority_hint"), &r(x, Y_PRIORITY - 6, view_w() - PAD - x, 48));
}

fn paint_conditions(hdc: HDC) {
    layout::muted(hdc, &t("editor.conditions_hint"), &r(PAD, TOP, content_w(), 48));
    caption(hdc, &t("editor.process"), PAD, Y_PROCESS - CAP, content_w());
    layout::muted(hdc, &t("editor.process_hint"), &r(PAD, Y_PROCESS + FRAME_H + 4, content_w(), 32));
    caption(hdc, &t("editor.window_title"), PAD, Y_TITLE - CAP, content_w());
    layout::muted(hdc, &t("editor.window_title_hint"), &r(PAD, Y_TITLE + FRAME_H + 4, content_w(), 32));
}

fn on_paint(hwnd: HWND) {
    let Some((tab, tabs, edits, editing)) = ed(|e| (e.tab, e.tabs, e.edits.clone(), e.editing.is_some())) else {
        return;
    };
    let title = t(if editing { "editor.title_edit" } else { "editor.title_new" });
    paint(hwnd, |hdc, _| {
        layout::paint_chrome(hdc, hwnd, &title);
        kit::tab_underline(hdc, hwnd, tabs[tab]);
        edit_frames(hwnd, hdc, &edits);
        match tab {
            TAB_GENERAL => paint_general(hdc),
            TAB_TRIGGER => trigger::paint(hdc),
            TAB_ACTIONS => steps::paint(hdc),
            _ => paint_conditions(hdc),
        }
    });
}

// ---------- Discord lookups (forwarded to the Actions tab) ----------

pub fn on_guilds(result: Result<Vec<Guild>, ApiError>) {
    steps::on_guilds(result);
}

pub fn on_channels(result: Result<Vec<Channel>, ApiError>) {
    steps::on_channels(result);
}

pub fn on_user(result: Result<User, ApiError>) {
    steps::on_user(result);
}

// ---------- save ----------

fn number(text: &str, range: (u32, u32), label: &str) -> Result<u32, String> {
    timing::parse_ms_in(text, range).map_err(|k| format!("{label}: {}", t(k)))
}

pub(super) fn seconds(text: &str, range: (u32, u32), label: &str) -> Result<u32, String> {
    let shown = |ms: u32| timing::format_seconds(ms, comma());
    timing::parse_seconds_in(text, range)
        .map_err(|k| format!("{label}: {} ({}–{})", t(k), shown(range.0), shown(range.1)))
}

/// Reads every tab into a Binding. Returns the tab to show on error.
fn build() -> Result<Binding, (String, usize)> {
    steps::commit();
    let Some(e) = ed(|e| {
        (
            (e.editing, e.profile, e.shortcuts.clone(), e.cancel_shortcut.clone(), e.trigger),
            [e.name, e.description, e.icon, e.priority, e.repeat_interval, e.repeat_max, e.cooldown, e.process, e.title],
            [e.t_enabled, e.t_sound, e.t_notify, e.t_inherit, e.t_fullscreen],
            e.steps.actions.clone(),
        )
    }) else {
        return Err((String::new(), TAB_GENERAL));
    };
    let ((editing, profile, shortcuts, cancel_shortcut, mode), edits, toggles, actions) = e;
    let [name, description, icon, priority, rep_int, rep_max, cooldown, process, title] = edits.map(get_text);
    let [enabled, sound, notify, inherit, fullscreen] = toggles.map(is_checked);
    let (step_delay_ms, stop_on_failure) = steps::read_options().map_err(|m| (m, TAB_ACTIONS))?;

    let priority =
        number(&priority, (0, MAX_PRIORITY as u32), &t("editor.priority")).map_err(|m| (m, TAB_GENERAL))? as u8;
    let mut trigger = Trigger { mode, ..Trigger::default() };
    if mode.repeats() {
        trigger.repeat_interval_ms =
            seconds(&rep_int, REPEAT_INTERVAL_RANGE, &t("editor.repeat_interval")).map_err(|m| (m, TAB_TRIGGER))?;
        trigger.repeat_max =
            number(&rep_max, REPEAT_MAX_RANGE, &t("editor.repeat_max")).map_err(|m| (m, TAB_TRIGGER))?;
    }
    let cooldown_ms = if inherit {
        None
    } else {
        Some(seconds(&cooldown, COOLDOWN_RANGE_MS, &t("editor.cooldown")).map_err(|m| (m, TAB_TRIGGER))?)
    };
    let id = editing.unwrap_or_else(|| app::with(|a| a.config.next_id()));
    if id >= MAX_BINDING_ID {
        return Err((t("err.too_many_bindings"), TAB_GENERAL));
    }
    let binding = Binding {
        id,
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        icon: icon.trim().chars().take(8).collect(),
        enabled,
        profile,
        shortcuts,
        trigger,
        actions,
        step_delay_ms,
        stop_on_failure,
        cooldown_ms,
        priority,
        sound,
        notify,
        conditions: Conditions {
            process: process.trim().to_string(),
            window_title: title.trim().to_string(),
            skip_fullscreen: fullscreen,
        },
        cancel_shortcut,
    };
    if let Err((key, step)) = binding.validate() {
        let tab = match key {
            "err.name" => TAB_GENERAL,
            "err.repeat_discord" | "err.repeat_typing" | "err.too_many_shortcuts" => TAB_TRIGGER,
            "err.cancel_standard" | "err.cancel_invalid" | "err.cancel_reserved" | "err.cancel_same" => TAB_TRIGGER,
            _ => TAB_ACTIONS,
        };
        let msg = match step {
            Some(i) => {
                steps::select(i);
                tf("editor.step_error", &[("n", &(i + 1).to_string()), ("details", &t(key))])
            }
            None => t(key),
        };
        return Err((msg, tab));
    }
    Ok(binding)
}

fn save() {
    stop_recording();
    let Some(hwnd) = ed(|e| e.hwnd) else { return };
    let binding = match build() {
        Ok(b) => b,
        Err((msg, tab)) => {
            set_tab(tab);
            if !msg.is_empty() {
                alert(hwnd, &msg);
            }
            return;
        }
    };
    let conflicts: Vec<String> = binding
        .shortcuts
        .iter()
        .flat_map(|sc| app::with(|a| a.config.conflicts_with(sc, Some(binding.id), binding.profile)))
        .collect();
    if !conflicts.is_empty() && !confirm(hwnd, &tf("editor.confirm_conflicts", &[("details", &conflicts.join("\n"))])) {
        return;
    }
    let id = binding.id;
    match app::save_binding(binding) {
        Ok(problem) => {
            close();
            main_window::select(id);
            if let Some(p) = problem {
                alert(main_window::hwnd(), &tf("err.hotkey_taken", &[("details", &p)]));
            }
        }
        Err(e) => alert(hwnd, &e),
    }
}

pub fn close() {
    stop_recording();
    let Some(hwnd) = ed(|e| e.hwnd) else { return };
    ED.with(|e| *e.borrow_mut() = None);
    unsafe { DestroyWindow(hwnd) };
    page::closed();
}

/// Lets Tab/Shift+Tab move between controls.
pub fn dialog_message(msg: &MSG) -> bool {
    match ed(|e| e.hwnd) {
        Some(h) => unsafe { IsDialogMessageW(h, msg) != 0 },
        None => false,
    }
}

pub(super) fn set_tab(tab: usize) {
    if tab != TAB_ACTIONS {
        steps::commit();
    }
    ed(|e| e.tab = tab);
    layout();
}

/// A tab button was clicked: the underline slides when the mouse did it.
fn click_tab(hwnd: HWND, new: usize) {
    if anim::by_pointer()
        && let Some((old, tabs)) = ed(|e| (e.tab, e.tabs))
    {
        kit::slide_tab(hwnd, tabs[old], tabs[new]);
    }
    set_tab(new);
}

fn menu_choice<T: Copy + PartialEq>(hwnd: HWND, anchor: HWND, options: &[(T, String)], current: T) -> Option<T> {
    let items: Vec<String> = options.iter().map(|(_, s)| s.clone()).collect();
    let checked: Vec<usize> = options.iter().position(|(v, _)| *v == current).into_iter().collect();
    popup_menu(hwnd, anchor, &items, &checked).map(|i| options[i].0)
}

fn on_command(id: i32, code: u32) {
    let Some(hwnd) = ed(|e| e.hwnd) else { return };
    match id {
        _ if (ID_TAB..ID_TAB + 4).contains(&id) => click_tab(hwnd, (id - ID_TAB) as usize),
        ID_PROFILE => {
            let Some((btn, current)) = ed(|e| (e.profile_btn, e.profile)) else { return };
            let mut options = vec![(ALL_PROFILES, t("profile.all"))];
            options.extend(app::with(|a| a.config.profiles.iter().map(|p| (p.id, p.name.clone())).collect::<Vec<_>>()));
            if let Some(p) = menu_choice(hwnd, btn, &options, current) {
                ed(|e| e.profile = p);
                layout();
            }
        }
        ID_ENABLED | ID_SOUND | ID_NOTIFY | ID_FULLSCREEN | ID_INHERIT => {
            if let Some(btn) = ed(|e| match id {
                ID_ENABLED => e.t_enabled,
                ID_SOUND => e.t_sound,
                ID_NOTIFY => e.t_notify,
                ID_INHERIT => e.t_inherit,
                _ => e.t_fullscreen,
            }) {
                toggle(btn);
                layout();
            }
        }
        _ if (ID_REMOVE_SC..ID_REMOVE_SC + 3).contains(&id) => {
            let i = (id - ID_REMOVE_SC) as usize;
            ed(|e| {
                if i < e.shortcuts.len() {
                    e.shortcuts.remove(i);
                }
                e.info = t("editor.record_hint");
            });
            layout();
        }
        ID_REC_MODE => {
            let Some((btn, mode)) = ed(|e| (e.rec_mode_btn, e.mode)) else { return };
            let options: Vec<(RecordMode, String)> = MODES.iter().map(|(m, k)| (*m, t(k))).collect();
            if let Some(m) = menu_choice(hwnd, btn, &options, mode) {
                ed(|e| e.mode = m);
                // read after the menu closes: recording may have ended meanwhile
                if ed(|e| e.recording && !e.rec_cancel).unwrap_or(false) {
                    stop_recording();
                    start_recording();
                }
                layout();
            }
        }
        ID_RECORD => {
            if ed(|e| e.recording && !e.rec_cancel).unwrap_or(false) {
                match stop_recording() {
                    Some(RecordUpdate::Done(sc)) => add_shortcut(sc),
                    Some(RecordUpdate::Rejected(m)) => {
                        ed(|e| e.info = m);
                    }
                    _ => {
                        ed(|e| e.info = t("editor.record_stopped"));
                    }
                }
                layout();
            } else {
                stop_recording(); // the stop shortcut may be recording
                start_recording();
            }
        }
        ID_CANCEL_REC | ID_CANCEL_CLEAR => cancel::on_command(id),
        ID_TRIGGER => {
            let Some((btn, mode)) = ed(|e| (e.trigger_btn, e.trigger)) else { return };
            let options: Vec<(TriggerMode, String)> = TriggerMode::ALL.iter().map(|m| (*m, t(m.key()))).collect();
            if let Some(m) = menu_choice(hwnd, btn, &options, mode) {
                ed(|e| e.trigger = m);
                layout();
            }
        }
        ID_SAVE => save(),
        ID_CANCEL | IDCANCEL_KEY => close(),
        page::ID_HOME => {
            page::go_home();
        }
        _ if code == EN_SETFOCUS || code == EN_KILLFOCUS || code == EN_CHANGE => {
            if !steps::on_edit_change(id, code) {
                invalidate(hwnd);
            }
        }
        _ => steps::on_command(id, code),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CTLCOLORSTATIC {
        let edits = ed(|e| e.edits.clone()).unwrap_or_default();
        if let Some(r) = layout::disabled_edit_color(wparam, lparam, &edits) {
            return r;
        }
    }
    if let Some(r) = ctl_color(msg, wparam) {
        return r;
    }
    match msg {
        WM_CREATE => {
            on_create(hwnd);
            0
        }
        WM_PAINT => {
            on_paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MEASUREITEM => {
            let mis = unsafe { &mut *(lparam as *mut MEASUREITEMSTRUCT) };
            mis.itemHeight = px(steps::ROW_H) as u32;
            1
        }
        WM_DRAWITEM => {
            let dis = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            if dis.CtlType == ODT_LISTBOX {
                steps::draw_row(dis);
            } else {
                draw_button(dis);
            }
            1
        }
        WM_COMMAND => {
            let (id, code) = command_parts(wparam);
            on_command(id, code);
            0
        }
        WM_TIMER if anim::on_timer(hwnd, wparam) => {
            page::on_frame(hwnd);
            0
        }
        WM_TIMER if wparam == TIMER_RECORD_LIMIT => {
            if stop_recording().is_some() {
                ed(|e| {
                    let msg = t("editor.record_timeout");
                    if e.rec_cancel { e.cancel_info = msg } else { e.info = msg }
                });
                layout();
            }
            0
        }
        WM_CLOSE => {
            close();
            0
        }
        WM_NCDESTROY => {
            anim::forget(hwnd);
            def(hwnd, msg, wparam, lparam)
        }
        _ => def(hwnd, msg, wparam, lparam),
    }
}
