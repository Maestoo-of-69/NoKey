//! Binding editor with four tabs: General, Trigger, Actions, Conditions.
//! The Actions tab lives in `editor/steps.rs`.

mod steps;

use super::theme::palette;
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
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};

const CLASS: &str = "NoKeyEditor";
const W: i32 = 540;
const H: i32 = 620;

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
    owner: HWND,
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
    // Actions (see steps.rs)
    pub(super) steps: steps::Steps,
    // Conditions
    process: HWND,
    title: HWND,
    t_fullscreen: HWND,
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

pub fn open(owner: HWND, binding: Option<Binding>) {
    if let Some(h) = ed(|e| e.hwnd) {
        unsafe { SetForegroundWindow(h) };
        return;
    }
    let title = t(if binding.is_some() { "editor.title_edit" } else { "editor.title_new" });
    let hwnd = create_window(CLASS, &title, W, H, owner);
    if hwnd.is_null() {
        return;
    }
    ed(|e| e.owner = owner);
    load(binding);
    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOW);
    }
    if let Some(name) = ed(|e| e.name) {
        unsafe { SetFocus(name) };
    }
}

fn comma() -> bool {
    i18n::language().decimal_comma()
}

fn on_create(hwnd: HWND) {
    let b = |id, text: &str, kind, x, y, w, h| {
        let btn = button(hwnd, id, text, kind);
        place(btn, x, y, w, h);
        btn
    };
    let e = |id, style, x, y, w| {
        let ed = edit(hwnd, id, style);
        place(ed, x, y, w, 20);
        ed
    };
    let tab_w = (W - 48) / 4;
    let tabs: [HWND; 4] =
        std::array::from_fn(|i| b(ID_TAB + i as i32, &t(TAB_KEYS[i]), BTN_TAB, 24 + tab_w * i as i32, 8, tab_w, 40));

    // General
    let name = e(ID_NAME, 0, 34, 86, W - 68);
    set_cue(name, &t("editor.name_cue"));
    let description = e(ID_DESCRIPTION, 0, 34, 152, W - 68);
    set_cue(description, &t("editor.description_cue"));
    let icon = e(ID_ICON, 0, 34, 218, 80);
    set_cue(icon, "🔔");
    let profile_btn = b(ID_PROFILE, "", BTN_DROPDOWN, 150, 210, 240, 32);
    let t_enabled = b(ID_ENABLED, &t("editor.enabled"), BTN_TOGGLE, 24, 262, W - 48, 26);
    let t_sound = b(ID_SOUND, &t("editor.sound"), BTN_TOGGLE, 24, 292, W - 48, 26);
    let t_notify = b(ID_NOTIFY, &t("editor.notify"), BTN_TOGGLE, 24, 322, W - 48, 26);
    let priority = e(ID_PRIORITY, ES_NUMBER as u32, 34, 390, 50);
    let general = vec![name, description, icon, profile_btn, t_enabled, t_sound, t_notify, priority];

    // Trigger
    let remove_btns: [HWND; 3] = std::array::from_fn(|i| {
        b(ID_REMOVE_SC + i as i32, &t("editor.remove"), BTN_SECONDARY, W - 24 - 110, 84 + 32 * i as i32, 110, 28)
    });
    let rec_mode_btn = b(ID_REC_MODE, "", BTN_DROPDOWN, 24, 186, 190, 32);
    let record_btn = b(ID_RECORD, &t("editor.record"), BTN_PRIMARY, 222, 186, 130, 32);
    let trigger_btn = b(ID_TRIGGER, "", BTN_DROPDOWN, 24, 296, 240, 32);
    let repeat_interval = e(ID_REPEAT_INTERVAL, 0, 34, 394, 140);
    let repeat_max = e(ID_REPEAT_MAX, ES_NUMBER as u32, 280, 394, 100);
    let t_inherit = b(ID_INHERIT, "", BTN_TOGGLE, 24, 456, 330, 26);
    let cooldown = e(ID_COOLDOWN, 0, 390, 460, 110);
    let mut trigger = vec![rec_mode_btn, record_btn, trigger_btn, repeat_interval, repeat_max, t_inherit, cooldown];
    trigger.extend(remove_btns);

    // Actions
    let (steps, actions_group) = steps::create(hwnd);

    // Conditions
    let process = e(ID_PROCESS, 0, 34, 140, W - 68);
    set_cue(process, "chrome.exe");
    let title = e(ID_TITLE, 0, 34, 226, W - 68);
    set_cue(title, &t("editor.title_cue"));
    let t_fullscreen = b(ID_FULLSCREEN, &t("editor.skip_fullscreen"), BTN_TOGGLE, 24, 290, W - 48, 26);
    let conditions = vec![process, title, t_fullscreen];

    b(ID_SAVE, &t("common.save"), BTN_PRIMARY, 24, 568, 140, 36);
    b(ID_CANCEL, &t("common.cancel"), BTN_SECONDARY, 172, 568, 120, 36);

    add_tooltip(hwnd, priority, &t("tip.priority"));
    add_tooltip(hwnd, trigger_btn, &t("tip.trigger"));
    add_tooltip(hwnd, repeat_interval, &t("tip.repeat_interval"));
    add_tooltip(hwnd, repeat_max, &t("tip.repeat_max"));
    add_tooltip(hwnd, t_inherit, &t("tip.cooldown"));
    add_tooltip(hwnd, cooldown, &t("tip.cooldown"));
    add_tooltip(hwnd, process, &t("tip.process"));
    add_tooltip(hwnd, title, &t("tip.window_title"));
    add_tooltip(hwnd, t_fullscreen, &t("tip.fullscreen"));
    add_tooltip(hwnd, icon, &t("tip.icon"));

    let mut edits = vec![name, description, icon, priority, repeat_interval, repeat_max, cooldown, process, title];
    edits.extend(steps.edits());
    ED.with(|cell| {
        *cell.borrow_mut() = Some(Ed {
            hwnd,
            owner: null_mut(),
            editing: None,
            tab: TAB_GENERAL,
            tabs,
            groups: [general, trigger, actions_group, conditions],
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
            steps,
            process,
            title,
            t_fullscreen,
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
            (e.rec_mode_btn, e.mode, e.record_btn, e.recording),
            (e.trigger_btn, e.trigger, e.profile_btn, e.profile),
            (e.t_inherit, e.cooldown, [e.repeat_interval, e.repeat_max]),
        )
    }) else {
        return;
    };
    let (hwnd, tab, tabs, groups, n_sc, removes, rec, trig, cd) = s;
    let (rec_mode_btn, mode, record_btn, recording) = rec;
    let (trigger_btn, trigger, profile_btn, profile) = trig;
    let (inherit, cooldown, rep) = cd;
    for (i, group) in groups.iter().enumerate() {
        set_checked(tabs[i], i == tab);
        for &c in group {
            show(c, i == tab);
        }
    }
    if tab == TAB_TRIGGER {
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
    enable(record_btn, recording || n_sc < MAX_SHORTCUTS);
    set_text(trigger_btn, &t(trigger.key()));
    let pname = app::with(|a| profile_name(&a.config, profile));
    set_text(profile_btn, &pname);
    enable(cooldown, !is_checked(inherit));
    invalidate(hwnd);
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

fn paint_trigger(hdc: HDC, shortcuts: &[Shortcut], live: &str, info: &str, recording: bool, trigger: TriggerMode) {
    let p = palette();
    let f = fonts();
    caption(hdc, &t("editor.shortcuts"), 24, 60, 300);
    if shortcuts.is_empty() {
        text(hdc, &t("editor.no_shortcuts"), &r(24, 84, W - 160, 28), f.normal, p.muted, DT_SINGLELINE | DT_VCENTER);
    }
    for (i, sc) in shortcuts.iter().enumerate() {
        let row = r(24, 84 + 32 * i as i32, W - 160, 28);
        round_rect(hdc, &row, p.border, Some(p.surface), 8);
        let inner = RECT { left: row.left + px(10), ..row };
        let label = format!("{}  ·  {}", sc.label(), t(kind_key(sc)));
        text(hdc, &label, &inner, f.bold, p.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }
    let shown = match (recording, live.is_empty()) {
        (true, true) => t("editor.press_keys"),
        (true, false) => live.to_string(),
        _ => String::new(),
    };
    text(hdc, &shown, &r(24, 224, W - 48, 22), f.bold, p.text, DT_SINGLELINE | DT_END_ELLIPSIS);
    text(hdc, info, &r(24, 246, W - 48, 30), f.small, p.muted, DT_WORDBREAK | DT_END_ELLIPSIS);
    caption(hdc, &t("editor.trigger"), 24, 276, 300);
    text(hdc, &trigger_hint(trigger), &r(24, 334, W - 48, 34), f.small, p.muted, DT_WORDBREAK | DT_END_ELLIPSIS);
    if trigger.repeats() {
        caption(hdc, &t("editor.repeat_interval"), 24, 370, 240);
        caption(hdc, &t("editor.repeat_max"), 270, 370, 220);
    }
    caption(hdc, &t("editor.cooldown"), 24, 434, 300);
    text(hdc, &t("editor.cooldown_hint"), &r(24, 496, W - 48, 34), f.small, p.muted, DT_WORDBREAK);
}

fn on_paint(hwnd: HWND) {
    let Some((tab, edits, shortcuts, live, info, recording, trigger)) =
        ed(|e| (e.tab, e.edits.clone(), e.shortcuts.clone(), e.live.clone(), e.info.clone(), e.recording, e.trigger))
    else {
        return;
    };
    paint(hwnd, |hdc, _| {
        let p = palette();
        let f = fonts();
        header_band(hdc, W, 52);
        edit_frames(hwnd, hdc, &edits);
        match tab {
            TAB_GENERAL => {
                caption(hdc, &t("editor.name"), 24, 60, 300);
                caption(hdc, &t("editor.description"), 24, 126, 300);
                caption(hdc, &t("editor.icon"), 24, 192, 110);
                caption(hdc, &t("editor.profile"), 150, 192, 240);
                caption(hdc, &t("editor.priority"), 24, 364, 300);
                text(hdc, &t("editor.priority_hint"), &r(104, 386, W - 128, 30), f.small, p.muted, DT_WORDBREAK);
            }
            TAB_TRIGGER => paint_trigger(hdc, &shortcuts, &live, &info, recording, trigger),
            TAB_ACTIONS => steps::paint(hdc),
            _ => {
                text(hdc, &t("editor.conditions_hint"), &r(24, 60, W - 48, 40), f.small, p.muted, DT_WORDBREAK);
                caption(hdc, &t("editor.process"), 24, 112, 300);
                let hint = r(24, 172, W - 48, 18);
                text(hdc, &t("editor.process_hint"), &hint, f.small, p.muted, DT_SINGLELINE | DT_END_ELLIPSIS);
                caption(hdc, &t("editor.window_title"), 24, 198, 300);
                let hint = r(24, 258, W - 48, 18);
                text(hdc, &t("editor.window_title_hint"), &hint, f.small, p.muted, DT_SINGLELINE | DT_END_ELLIPSIS);
            }
        }
    });
}

// ---------- recording ----------

fn start_recording() {
    let Some(mode) = ed(|e| e.mode) else { return };
    match hook::start_recording(Recorder::new(mode)) {
        Ok(()) => {
            let prompt = t(match mode {
                RecordMode::Standard => "editor.prompt_standard",
                RecordMode::Combo => "editor.prompt_combo",
                RecordMode::Sequence => "editor.prompt_sequence",
            });
            let hwnd = ed(|e| {
                e.recording = true;
                e.live = String::new();
                e.info = prompt;
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
            ed(|e| e.info = err);
        }
    }
    layout();
}

fn stop_recording() -> Option<RecordUpdate> {
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
fn describe(sc: &Shortcut, self_id: u32, profile: u32) -> String {
    let conflicts = app::with(|a| a.config.conflicts_with(sc, Some(self_id), profile));
    if !conflicts.is_empty() {
        tf("editor.conflict", &[("details", &conflicts.join("; "))])
    } else if sc.needs_hook() {
        t("editor.uses_hook")
    } else {
        t("editor.works_everywhere")
    }
}

fn add_shortcut(sc: Shortcut) {
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
    let Some((mode, recording)) = ed(|e| (e.mode, e.recording)) else {
        hook::stop_recording();
        return;
    };
    if !recording {
        return; // a late update from a recording that already ended
    }
    match update {
        RecordUpdate::Progress(s) => {
            ed(|e| e.live = s);
            layout();
        }
        RecordUpdate::Done(sc) => {
            stop_recording();
            add_shortcut(sc);
        }
        RecordUpdate::Cancelled => {
            stop_recording();
            ed(|e| e.info = t("editor.record_cancelled"));
            layout();
        }
        RecordUpdate::Rejected(msg) => {
            if mode == RecordMode::Sequence {
                stop_recording();
            }
            ed(|e| {
                e.info = msg;
                e.live = String::new();
            });
            layout();
        }
    }
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
            (e.editing, e.profile, e.shortcuts.clone(), e.trigger),
            [e.name, e.description, e.icon, e.priority, e.repeat_interval, e.repeat_max, e.cooldown, e.process, e.title],
            [e.t_enabled, e.t_sound, e.t_notify, e.t_inherit, e.t_fullscreen],
            e.steps.actions.clone(),
        )
    }) else {
        return Err((String::new(), TAB_GENERAL));
    };
    let ((editing, profile, shortcuts, mode), edits, toggles, actions) = e;
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
    };
    if let Err((key, step)) = binding.validate() {
        let tab = match key {
            "err.name" => TAB_GENERAL,
            "err.repeat_discord" | "err.too_many_shortcuts" => TAB_TRIGGER,
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
    let Some((hwnd, owner)) = ed(|e| (e.hwnd, e.owner)) else { return };
    ED.with(|e| *e.borrow_mut() = None);
    unsafe {
        EnableWindow(owner, 1); // re-enable before destroying so focus returns to it
        DestroyWindow(hwnd);
    }
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

fn menu_choice<T: Copy + PartialEq>(hwnd: HWND, anchor: HWND, options: &[(T, String)], current: T) -> Option<T> {
    let items: Vec<String> = options.iter().map(|(_, s)| s.clone()).collect();
    let checked: Vec<usize> = options.iter().position(|(v, _)| *v == current).into_iter().collect();
    popup_menu(hwnd, anchor, &items, &checked).map(|i| options[i].0)
}

fn on_command(id: i32, code: u32) {
    let Some(hwnd) = ed(|e| e.hwnd) else { return };
    match id {
        _ if (ID_TAB..ID_TAB + 4).contains(&id) => set_tab((id - ID_TAB) as usize),
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
                if ed(|e| e.recording).unwrap_or(false) {
                    stop_recording();
                    start_recording();
                }
                layout();
            }
        }
        ID_RECORD => {
            if ed(|e| e.recording).unwrap_or(false) {
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
                start_recording();
            }
        }
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
        _ if code == EN_SETFOCUS || code == EN_KILLFOCUS || code == EN_CHANGE => {
            if !steps::on_edit_change(id, code) {
                invalidate(hwnd);
            }
        }
        _ => steps::on_command(id, code),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
            mis.itemHeight = px(30) as u32;
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
        WM_TIMER if wparam == TIMER_RECORD_LIMIT => {
            if stop_recording().is_some() {
                ed(|e| e.info = t("editor.record_timeout"));
                layout();
            }
            0
        }
        WM_CLOSE => {
            close();
            0
        }
        _ => def(hwnd, msg, wparam, lparam),
    }
}
