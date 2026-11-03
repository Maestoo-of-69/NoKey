//! Settings with four tabs: General (theme, language, startup, timing),
//! Discord, Profiles and Updates.

use super::theme::palette;
use super::*;
use crate::app::{self, is_builtin_profile, TaskResult};
use crate::config::{
    timing, Language, Theme, COMBO_WINDOW_RANGE, COOLDOWN_RANGE_MS, DOUBLE_TAP_RANGE, LONG_PRESS_RANGE,
    SEQUENCE_TIMEOUT_RANGE,
};
use crate::engine::recorder::{RecordMode, RecordUpdate, Recorder};
use crate::i18n::{self, t, tf};
use crate::security::dpapi::mask;
use crate::update::{self, ReleaseInfo, CURRENT_VERSION};
use crate::win::{get_text, hook, set_text};
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;

const CLASS: &str = "NoKeySettings";
const W: i32 = 540;
const H: i32 = 600;

pub const TAB_GENERAL: usize = 0;
pub const TAB_DISCORD: usize = 1;
pub const TAB_PROFILES: usize = 2;
pub const TAB_UPDATES: usize = 3;
const TAB_KEYS: [&str; 4] = ["settings.tab_general", "settings.tab_discord", "settings.tab_profiles", "settings.tab_updates"];

const ID_TAB: i32 = 400; // 400..404
const ID_THEME: i32 = 410;
const ID_LANGUAGE: i32 = 411;
const ID_TOGGLE: i32 = 412; // 412..416
const ID_NUM: i32 = 420; // 420..425
const ID_TOKEN: i32 = 430;
const ID_CONNECT: i32 = 431;
const ID_DISCONNECT: i32 = 432;
const ID_PROFILES: i32 = 440;
const ID_PROFILE_NAME: i32 = 441;
const ID_P_ADD: i32 = 442;
const ID_P_RENAME: i32 = 443;
const ID_P_DUPLICATE: i32 = 444;
const ID_P_DELETE: i32 = 445;
const ID_P_ACTIVATE: i32 = 446;
const ID_SWITCH_RECORD: i32 = 447;
const ID_SWITCH_CLEAR: i32 = 448;
const ID_REPO: i32 = 450;
const ID_CHECK: i32 = 451;
const ID_SAVE: i32 = 460;
const ID_CLOSE: i32 = 461;
const IDCANCEL_KEY: i32 = 2;

const TOGGLES: [&str; 4] = ["settings.start_with_windows", "settings.minimize_to_tray", "settings.launch_minimized", "settings.notifications"];
/// (label key, x, y) of the timing fields: cooldown is in seconds, the rest in ms.
const NUMS: [(&str, i32, i32); 5] = [
    ("settings.cooldown", 24, 286),
    ("settings.combo_window", 190, 286),
    ("settings.sequence_timeout", 356, 286),
    ("settings.double_tap", 24, 358),
    ("settings.long_press", 190, 358),
];

struct Se {
    hwnd: HWND,
    tab: usize,
    tabs: [HWND; 4],
    groups: [Vec<HWND>; 4],
    edits: Vec<HWND>,
    theme_btn: HWND,
    language_btn: HWND,
    toggles: [HWND; 4],
    nums: [HWND; 5],
    token: HWND,
    btn_disconnect: HWND,
    profiles: HWND,
    profile_ids: Vec<u32>,
    profile_name: HWND,
    switch_record: HWND,
    recording: bool,
    switch_info: String,
    repo: HWND,
    btn_check: HWND,
    update_status: String,
}

thread_local! {
    static SE: RefCell<Option<Se>> = const { RefCell::new(None) };
}

fn se<R>(f: impl FnOnce(&mut Se) -> R) -> Option<R> {
    SE.with(|s| s.try_borrow_mut().ok()?.as_mut().map(f))
}

pub fn register(icon: HICON) {
    register_class(CLASS, Some(wndproc), icon);
}

pub fn hwnd() -> HWND {
    se(|s| s.hwnd).unwrap_or(null_mut())
}

pub fn current_tab() -> Option<usize> {
    se(|s| s.tab)
}

pub fn is_recording() -> bool {
    se(|s| s.recording).unwrap_or(false)
}

pub fn open(owner: HWND) {
    open_tab(owner, TAB_GENERAL);
}

pub fn open_tab(owner: HWND, tab: usize) {
    if se(|s| s.hwnd).is_none() {
        let hwnd = create_window(CLASS, &t("settings.title"), W, H, owner);
        if hwnd.is_null() {
            return;
        }
        load();
    }
    se(|s| s.tab = tab);
    layout();
    let h = hwnd();
    unsafe {
        ShowWindow(h, SW_SHOW);
        SetForegroundWindow(h);
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
    let theme_btn = b(ID_THEME, "", BTN_DROPDOWN, 24, 82, 230, 32);
    let language_btn = b(ID_LANGUAGE, "", BTN_DROPDOWN, 286, 82, 230, 32);
    let toggles: [HWND; 4] =
        std::array::from_fn(|i| b(ID_TOGGLE + i as i32, &t(TOGGLES[i]), BTN_TOGGLE, 24, 130 + 30 * i as i32, W - 48, 26));
    let nums: [HWND; 5] = std::array::from_fn(|i| {
        let (_, x, y) = NUMS[i];
        e(ID_NUM + i as i32, 0, x + 10, y + 28, 130)
    });
    let mut general = vec![theme_btn, language_btn];
    general.extend(toggles);
    general.extend(nums);

    // Discord
    let token = e(ID_TOKEN, ES_PASSWORD as u32, 34, 90, W - 68);
    set_cue(token, &t("settings.token_cue"));
    let btn_connect = b(ID_CONNECT, &t("settings.connect"), BTN_PRIMARY, 24, 130, 140, 32);
    let btn_disconnect = b(ID_DISCONNECT, &t("settings.disconnect"), BTN_SECONDARY, 172, 130, 140, 32);
    let discord = vec![token, btn_connect, btn_disconnect];

    // Profiles
    let profiles = listbox(hwnd, ID_PROFILES, 0);
    place(profiles, 24, 82, W - 48, 150);
    let profile_name = e(ID_PROFILE_NAME, 0, 34, 248, W - 68);
    set_cue(profile_name, &t("settings.profile_name_cue"));
    let p_buttons = [
        b(ID_P_ADD, &t("settings.profile_add"), BTN_SECONDARY, 24, 288, 110, 30),
        b(ID_P_RENAME, &t("settings.profile_rename"), BTN_SECONDARY, 142, 288, 120, 30),
        b(ID_P_DUPLICATE, &t("settings.profile_duplicate"), BTN_SECONDARY, 270, 288, 120, 30),
        b(ID_P_DELETE, &t("settings.profile_delete"), BTN_SECONDARY, 398, 288, 118, 30),
        b(ID_P_ACTIVATE, &t("settings.profile_activate"), BTN_PRIMARY, 24, 328, 200, 30),
    ];
    let switch_record = b(ID_SWITCH_RECORD, &t("editor.record"), BTN_SECONDARY, 24, 428, 130, 30);
    let switch_clear = b(ID_SWITCH_CLEAR, &t("settings.clear"), BTN_SECONDARY, 162, 428, 110, 30);
    let mut profiles_group = vec![profiles, profile_name, switch_record, switch_clear];
    profiles_group.extend(p_buttons);

    // Updates
    let repo = e(ID_REPO, 0, 34, 134, 320);
    set_cue(repo, "owner/repository");
    let btn_check = b(ID_CHECK, &t("settings.check_updates"), BTN_SECONDARY, 376, 126, 140, 36);
    let updates = vec![repo, btn_check];

    b(ID_SAVE, &t("common.save"), BTN_PRIMARY, 24, 548, 140, 36);
    b(ID_CLOSE, &t("common.close"), BTN_SECONDARY, 172, 548, 120, 36);

    add_tooltip(hwnd, nums[0], &t("tip.global_cooldown"));
    add_tooltip(hwnd, nums[1], &t("tip.combo_window"));
    add_tooltip(hwnd, nums[2], &t("tip.sequence_timeout"));
    add_tooltip(hwnd, nums[3], &t("tip.double_tap"));
    add_tooltip(hwnd, nums[4], &t("tip.long_press"));
    add_tooltip(hwnd, theme_btn, &t("tip.theme"));
    add_tooltip(hwnd, switch_record, &t("tip.profile_switch"));

    let mut edits = nums.to_vec();
    edits.extend([token, profile_name, repo]);
    SE.with(|s| {
        *s.borrow_mut() = Some(Se {
            hwnd,
            tab: TAB_GENERAL,
            tabs,
            groups: [general, discord, profiles_group, updates],
            edits,
            theme_btn,
            language_btn,
            toggles,
            nums,
            token,
            btn_disconnect,
            profiles,
            profile_ids: Vec::new(),
            profile_name,
            switch_record,
            recording: false,
            switch_info: String::new(),
            repo,
            btn_check,
            update_status: String::new(),
        })
    });
}

fn load() {
    let cfg = app::with(|a| a.config.clone());
    let Some((toggles, nums, repo)) = se(|s| (s.toggles, s.nums, s.repo)) else { return };
    let flags = [cfg.start_with_windows, cfg.minimize_to_tray, cfg.launch_minimized, cfg.notifications];
    for (b, on) in toggles.iter().zip(flags) {
        set_checked(*b, on);
    }
    let values = [
        timing::format_seconds(cfg.cooldown_ms, comma()),
        cfg.combo_window_ms.to_string(),
        cfg.sequence_timeout_ms.to_string(),
        cfg.double_tap_ms.to_string(),
        cfg.long_press_ms.to_string(),
    ];
    for (e, v) in nums.iter().zip(values) {
        set_text(*e, &v);
    }
    set_text(repo, &cfg.update_repo);
    refresh();
}

fn layout() {
    let Some((hwnd, tab, tabs, groups)) = se(|s| (s.hwnd, s.tab, s.tabs, s.groups.clone())) else { return };
    for (i, group) in groups.iter().enumerate() {
        set_checked(tabs[i], i == tab);
        for &c in group {
            show(c, i == tab);
        }
    }
    invalidate(hwnd);
}

/// Refreshes everything that depends on app state (connection, profiles).
pub fn refresh() {
    let Some((hwnd, disconnect, theme_btn, language_btn, list)) =
        se(|s| (s.hwnd, s.btn_disconnect, s.theme_btn, s.language_btn, s.profiles))
    else {
        return;
    };
    let (has_token, theme, language, profiles) =
        app::with(|a| (a.token.is_some(), a.config.theme, a.config.language, a.config.profiles.clone()));
    enable(disconnect, has_token);
    set_text(theme_btn, &t(theme.key()));
    set_text(language_btn, language.native_name());
    let selected = selected_profile();
    unsafe {
        SendMessageW(list, LB_RESETCONTENT, 0, 0);
        for p in &profiles {
            SendMessageW(list, LB_ADDSTRING, 0, p.id as LPARAM);
        }
        let idx = selected.and_then(|id| profiles.iter().position(|p| p.id == id)).map_or(usize::MAX, |i| i);
        SendMessageW(list, LB_SETCURSEL, idx as WPARAM, 0);
    }
    se(|s| s.profile_ids = profiles.iter().map(|p| p.id).collect());
    invalidate(hwnd);
}

fn selected_profile() -> Option<u32> {
    let (list, ids) = se(|s| (s.profiles, s.profile_ids.clone()))?;
    let idx = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
    if idx < 0 {
        return None;
    }
    ids.get(idx as usize).copied()
}

fn paint_general(hdc: HDC) {
    let p = palette();
    let f = fonts();
    caption(hdc, &t("settings.theme"), 24, 60, 230);
    caption(hdc, &t("settings.language"), 286, 60, 230);
    caption(hdc, &t("settings.timing"), 24, 262, 300);
    for (key, x, y) in NUMS {
        text(hdc, &t(key), &r(x, y, 160, 18), f.small, p.muted, DT_SINGLELINE | DT_END_ELLIPSIS);
    }
    text(hdc, &t("settings.timing_hint"), &r(24, 426, W - 48, 40), f.small, p.muted, DT_WORDBREAK);
}

fn paint_discord(hdc: HDC) {
    let p = palette();
    let f = fonts();
    let (conn_line, masked) = app::with(|a| (a.conn.line(), a.token.as_deref().map(mask)));
    caption(hdc, &t("settings.discord_bot"), 24, 60, 300);
    text(hdc, &conn_line, &r(24, 176, W - 48, 20), f.bold, p.text, DT_SINGLELINE | DT_END_ELLIPSIS);
    let note = match masked {
        Some(m) => tf("settings.token_saved", &[("mask", &m)]),
        None => t("settings.token_note"),
    };
    text(hdc, &note, &r(24, 198, W - 48, 18), f.small, p.muted, DT_SINGLELINE | DT_END_ELLIPSIS);
    text(hdc, &t("settings.discord_help"), &r(24, 232, W - 48, 60), f.small, p.muted, DT_WORDBREAK);
}

fn paint_profiles(hdc: HDC, switch_info: &str, recording: bool) {
    let p = palette();
    let f = fonts();
    let switch = app::with(|a| a.config.profile_switch.as_ref().map(|s| s.label()));
    caption(hdc, &t("settings.profiles"), 24, 60, 300);
    caption(hdc, &t("settings.profile_switch"), 24, 378, 400);
    let current = if recording {
        t("editor.press_keys")
    } else {
        switch.unwrap_or_else(|| t("settings.not_set"))
    };
    text(hdc, &current, &r(24, 398, W - 48, 24), f.bold, p.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    text(hdc, switch_info, &r(24, 466, W - 48, 34), f.small, p.muted, DT_WORDBREAK | DT_END_ELLIPSIS);
}

fn paint_updates(hdc: HDC, status: &str) {
    let p = palette();
    let f = fonts();
    caption(hdc, &t("settings.updates"), 24, 60, 300);
    let ver = tf("settings.installed_version", &[("version", CURRENT_VERSION)]);
    text(hdc, &ver, &r(24, 82, W - 48, 18), f.normal, p.text, DT_SINGLELINE);
    caption(hdc, &t("settings.repo"), 24, 108, 300);
    text(hdc, status, &r(24, 176, W - 48, 40), f.small, p.text, DT_WORDBREAK | DT_END_ELLIPSIS);
    text(hdc, &t("settings.update_help"), &r(24, 230, W - 48, 60), f.small, p.muted, DT_WORDBREAK);
}

fn on_paint(hwnd: HWND) {
    let Some((tab, edits, status, switch_info, recording)) =
        se(|s| (s.tab, s.edits.clone(), s.update_status.clone(), s.switch_info.clone(), s.recording))
    else {
        return;
    };
    paint(hwnd, |hdc, _| {
        header_band(hdc, W, 52);
        edit_frames(hwnd, hdc, &edits);
        match tab {
            TAB_GENERAL => paint_general(hdc),
            TAB_DISCORD => paint_discord(hdc),
            TAB_PROFILES => paint_profiles(hdc, &switch_info, recording),
            _ => paint_updates(hdc, &status),
        }
    });
}

fn draw_profile_row(dis: &DRAWITEMSTRUCT) {
    if dis.itemID == u32::MAX {
        return;
    }
    let Some(id) = se(|s| s.profile_ids.get(dis.itemID as usize).copied()).flatten() else { return };
    let (name, active, count) = app::with(|a| {
        let name = a.config.profile(id).map(|p| p.name.clone()).unwrap_or_default();
        let count = a.config.bindings.iter().filter(|b| b.profile == id).count();
        (name, a.config.active_profile == id, count)
    });
    let p = palette();
    let f = fonts();
    let rc = dis.rcItem;
    let selected = dis.itemState & ODS_SELECTED != 0;
    fill(dis.hDC, &rc, if selected { p.selected } else { p.surface });
    if selected {
        fill(dis.hDC, &RECT { right: rc.left + px(3), ..rc }, p.accent);
    }
    let mark = if active { "✓ " } else { "" };
    let left = RECT { left: rc.left + px(12), right: rc.right - px(180), ..rc };
    text(dis.hDC, &format!("{mark}{name}"), &left, f.bold, p.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    let mut note = tf("settings.profile_bindings", &[("count", &count.to_string())]);
    if is_builtin_profile(id) {
        note = format!("{} · {note}", t("settings.profile_builtin"));
    }
    let right = RECT { left: rc.right - px(180), right: rc.right - px(10), ..rc };
    text(dis.hDC, &note, &right, f.small, p.muted, DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_END_ELLIPSIS);
}

// ---------- Discord ----------

fn connect() {
    let Some((hwnd, token_edit)) = se(|s| (s.hwnd, s.token)) else { return };
    let token = get_text(token_edit).trim().to_string();
    if token.is_empty() {
        alert(hwnd, &t("err.token_empty"));
        return;
    }
    if token.chars().any(char::is_whitespace) {
        alert(hwnd, &t("err.token_spaces"));
        return;
    }
    set_text(token_edit, "");
    app::connect(token);
}

fn disconnect() {
    let Some(hwnd) = se(|s| s.hwnd) else { return };
    if confirm(hwnd, &t("settings.confirm_disconnect"))
        && let Err(e) = app::disconnect()
    {
        alert(hwnd, &e);
    }
}

// ---------- profiles ----------

fn profile_action(id: i32) {
    let Some((hwnd, name_edit)) = se(|s| (s.hwnd, s.profile_name)) else { return };
    let name = get_text(name_edit);
    let selected = selected_profile();
    let result = match (id, selected) {
        (ID_P_ADD, _) => app::add_profile(&name),
        (ID_P_RENAME, Some(p)) => app::rename_profile(p, &name),
        (ID_P_DUPLICATE, Some(p)) => app::duplicate_profile(p),
        (ID_P_DELETE, Some(p)) => {
            if !confirm(hwnd, &t("settings.confirm_delete_profile")) {
                return;
            }
            app::delete_profile(p)
        }
        (ID_P_ACTIVATE, Some(p)) => {
            app::set_active_profile(p);
            Ok(p)
        }
        _ => Err(t("err.select_profile")),
    };
    match result {
        Ok(_) => {
            if id == ID_P_ADD || id == ID_P_RENAME {
                set_text(name_edit, "");
            }
            refresh();
        }
        Err(e) => alert(hwnd, &e),
    }
}

fn toggle_switch_recording() {
    if se(|s| s.recording).unwrap_or(false) {
        hook::stop_recording();
        se(|s| {
            s.recording = false;
            s.switch_info = t("editor.record_stopped");
        });
    } else {
        match hook::start_recording(Recorder::new(RecordMode::Standard)) {
            Ok(()) => {
                let hwnd = se(|s| {
                    s.recording = true;
                    s.switch_info = t("editor.prompt_standard");
                    s.hwnd
                });
                if let Some(h) = hwnd {
                    unsafe { SetFocus(h) };
                }
            }
            Err(e) => {
                se(|s| s.switch_info = e);
            }
        }
    }
    update_switch_button();
}

fn update_switch_button() {
    if let Some((btn, recording, hwnd)) = se(|s| (s.switch_record, s.recording, s.hwnd)) {
        set_text(btn, &t(if recording { "editor.stop" } else { "editor.record" }));
        invalidate(hwnd);
    }
}

/// Recorder results while the profile switch shortcut is being recorded.
pub fn on_record(update: RecordUpdate) {
    match update {
        RecordUpdate::Progress(_) => {}
        RecordUpdate::Done(sc) => {
            hook::stop_recording();
            se(|s| s.recording = false);
            let info = match app::set_profile_switch(Some(sc)) {
                Ok(_) => t("settings.switch_saved"),
                Err(e) => e,
            };
            se(|s| s.switch_info = info);
        }
        RecordUpdate::Cancelled => {
            hook::stop_recording();
            se(|s| {
                s.recording = false;
                s.switch_info = t("editor.record_cancelled");
            });
        }
        RecordUpdate::Rejected(msg) => {
            se(|s| s.switch_info = msg);
        }
    }
    update_switch_button();
}

// ---------- updates ----------

fn check_updates() {
    let Some((hwnd, repo_edit, btn)) = se(|s| (s.hwnd, s.repo, s.btn_check)) else { return };
    let repo = get_text(repo_edit).trim().to_string();
    if !update::is_valid_repo(&repo) {
        alert(hwnd, &t("err.repo"));
        return;
    }
    enable(btn, false);
    set_status(&t("settings.checking_updates"));
    app::spawn_task(move || TaskResult::UpdateChecked(update::check(&repo)));
}

fn set_status(text: &str) {
    if let Some(h) = se(|s| {
        s.update_status = text.to_string();
        s.hwnd
    }) {
        invalidate(h);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

pub fn on_update_checked(result: Result<Option<ReleaseInfo>, String>) {
    let Some((hwnd, btn)) = se(|s| (s.hwnd, s.btn_check)) else { return };
    enable(btn, true);
    match result {
        Ok(None) => set_status(&tf("settings.up_to_date", &[("version", CURRENT_VERSION)])),
        Ok(Some(info)) => {
            set_status(&tf("settings.update_available", &[("version", &info.version)]));
            let notes = if info.notes.trim().is_empty() { t("settings.no_notes") } else { truncate(info.notes.trim(), 600) };
            let q = tf(
                "settings.confirm_download",
                &[("version", &info.version), ("current", CURRENT_VERSION), ("notes", &notes)],
            );
            if confirm(hwnd, &q) {
                enable(btn, false);
                set_status(&t("settings.downloading"));
                app::spawn_task(move || TaskResult::UpdateDownloaded(update::download(&info)));
            }
        }
        Err(e) => set_status(&e),
    }
}

pub fn on_update_downloaded(result: Result<update::Downloaded, String>) {
    let Some((hwnd, btn)) = se(|s| (s.hwnd, s.btn_check)) else { return };
    enable(btn, true);
    match result {
        Ok(downloaded) => {
            set_status(&t("settings.update_verified"));
            if confirm(hwnd, &t("settings.confirm_install"))
                && let Err(e) = app::install_update_and_exit(&downloaded)
            {
                alert(hwnd, &e);
            }
        }
        Err(e) => set_status(&e),
    }
}

// ---------- save ----------

fn save() {
    let Some((hwnd, toggles, nums, repo_edit)) = se(|s| (s.hwnd, s.toggles, s.nums, s.repo)) else { return };
    let label = |i: usize| t(NUMS[i].0);
    let ms = |i: usize, range: (u32, u32)| {
        timing::parse_ms_in(&get_text(nums[i]), range)
            .map_err(|k| format!("{}: {} ({}–{})", label(i), t(k), range.0, range.1))
    };
    let parsed = (|| -> Result<[u32; 5], String> {
        let cooldown = timing::parse_seconds_in(&get_text(nums[0]), COOLDOWN_RANGE_MS).map_err(|k| {
            let show = |v| timing::format_seconds(v, comma());
            format!("{}: {} ({}–{})", label(0), t(k), show(COOLDOWN_RANGE_MS.0), show(COOLDOWN_RANGE_MS.1))
        })?;
        Ok([
            cooldown,
            ms(1, COMBO_WINDOW_RANGE)?,
            ms(2, SEQUENCE_TIMEOUT_RANGE)?,
            ms(3, DOUBLE_TAP_RANGE)?,
            ms(4, LONG_PRESS_RANGE)?,
        ])
    })();
    let values = match parsed {
        Ok(v) => v,
        Err(e) => {
            se(|s| s.tab = TAB_GENERAL);
            layout();
            alert(hwnd, &e);
            return;
        }
    };
    let repo = get_text(repo_edit).trim().to_string();
    if !repo.is_empty() && !update::is_valid_repo(&repo) {
        alert(hwnd, &t("err.repo"));
        return;
    }
    let mut next = app::with(|a| a.config.clone());
    next.start_with_windows = is_checked(toggles[0]);
    next.minimize_to_tray = is_checked(toggles[1]);
    next.launch_minimized = is_checked(toggles[2]);
    next.notifications = is_checked(toggles[3]);
    [next.cooldown_ms, next.combo_window_ms, next.sequence_timeout_ms, next.double_tap_ms, next.long_press_ms] = values;
    next.update_repo = repo;
    match app::save_settings(next) {
        Ok(()) => close(),
        Err(e) => alert(hwnd, &e),
    }
}

pub fn close() {
    if se(|s| s.recording).unwrap_or(false) {
        hook::stop_recording();
    }
    let Some(hwnd) = se(|s| s.hwnd) else { return };
    SE.with(|s| *s.borrow_mut() = None);
    unsafe { DestroyWindow(hwnd) };
}

pub fn dialog_message(msg: &MSG) -> bool {
    match se(|s| s.hwnd) {
        Some(h) => unsafe { IsDialogMessageW(h, msg) != 0 },
        None => false,
    }
}

fn on_command(id: i32, code: u32) {
    let Some(hwnd) = se(|s| s.hwnd) else { return };
    match id {
        _ if (ID_TAB..ID_TAB + 4).contains(&id) => {
            se(|s| s.tab = (id - ID_TAB) as usize);
            layout();
        }
        ID_THEME => {
            let Some(btn) = se(|s| s.theme_btn) else { return };
            let current = app::with(|a| a.config.theme);
            let items: Vec<String> = Theme::ALL.iter().map(|th| t(th.key())).collect();
            let checked: Vec<usize> = Theme::ALL.iter().position(|th| *th == current).into_iter().collect();
            if let Some(i) = popup_menu(hwnd, btn, &items, &checked) {
                if let Err(e) = app::set_theme(Theme::ALL[i]) {
                    alert(hwnd, &e);
                }
                refresh();
            }
        }
        ID_LANGUAGE => {
            let Some(btn) = se(|s| s.language_btn) else { return };
            let current = app::with(|a| a.config.language);
            let items: Vec<String> = Language::ALL.iter().map(|l| l.native_name().to_string()).collect();
            let checked: Vec<usize> = Language::ALL.iter().position(|l| *l == current).into_iter().collect();
            if let Some(i) = popup_menu(hwnd, btn, &items, &checked)
                && Language::ALL[i] != current
                && let Err(e) = app::set_language(Language::ALL[i])
            {
                alert(hwnd, &e);
            }
        }
        _ if (ID_TOGGLE..ID_TOGGLE + 4).contains(&id) => {
            if let Some(btn) = se(|s| s.toggles[(id - ID_TOGGLE) as usize]) {
                toggle(btn);
            }
        }
        ID_CONNECT => connect(),
        ID_DISCONNECT => disconnect(),
        ID_PROFILES if code == LBN_SELCHANGE => {
            if let Some(p) = selected_profile() {
                let name = app::with(|a| a.config.profile(p).map(|x| x.name.clone()).unwrap_or_default());
                if let Some(e) = se(|s| s.profile_name) {
                    set_text(e, &name);
                }
            }
        }
        ID_PROFILES if code == LBN_DBLCLK => profile_action(ID_P_ACTIVATE),
        ID_P_ADD | ID_P_RENAME | ID_P_DUPLICATE | ID_P_DELETE | ID_P_ACTIVATE => profile_action(id),
        ID_SWITCH_RECORD => toggle_switch_recording(),
        ID_SWITCH_CLEAR => {
            let info = match app::set_profile_switch(None) {
                Ok(_) => t("settings.switch_cleared"),
                Err(e) => e,
            };
            se(|s| s.switch_info = info);
            invalidate(hwnd);
        }
        ID_CHECK => check_updates(),
        ID_SAVE => save(),
        ID_CLOSE | IDCANCEL_KEY => close(),
        _ if code == EN_SETFOCUS || code == EN_KILLFOCUS => invalidate(hwnd),
        _ => {}
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
                draw_profile_row(dis);
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
        WM_CLOSE => {
            close();
            0
        }
        _ => def(hwnd, msg, wparam, lparam),
    }
}
