//! Settings with four tabs: General (theme, language, startup, timing),
//! Discord, Profiles and Updates. Layout follows docs/DESIGN.md: header bar,
//! tabs, sentence-case captions, switch rows and a bottom action bar. All
//! coordinates are 96-DPI units from the left (Hebrew windows mirror).
//! Motion: the tab underline slides after a click, switches slide, the
//! profile-switch Record button pulses while recording.

use super::theme::palette;
use super::*;
use crate::app::{self, is_builtin_profile, Conn};
#[cfg(not(feature = "store"))]
use crate::app::TaskResult;
use crate::config::{
    timing, Language, Theme, COMBO_WINDOW_RANGE, COOLDOWN_RANGE_MS, DOUBLE_TAP_RANGE, LONG_PRESS_RANGE,
    SEQUENCE_TIMEOUT_RANGE,
};
use crate::engine::recorder::{RecordMode, RecordUpdate, Recorder};
use crate::i18n::{self, t, tf};
use crate::security::dpapi::mask;
use crate::update::CURRENT_VERSION;
#[cfg(not(feature = "store"))]
use crate::update::{self, ReleaseInfo};
use crate::win::{get_text, hook, set_text};
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;

mod view;

const CLASS: &str = "NoKeySettings";
fn view_w() -> i32 {
    page::width()
}
// Fits a 1080p screen at 150 % (client 924 px + frame) with room to spare.
fn view_h() -> i32 {
    page::height()
}
const PAD: i32 = 16;
fn content_w() -> i32 {
    view_w() - 2 * PAD
}
const HEADER_H: i32 = 52;
const TAB_H: i32 = 36;
/// First caption of every tab: 16 below the tab strip.
const TOP: i32 = HEADER_H + TAB_H + 16;
const BAR_H: i32 = 56;
fn bar_y() -> i32 {
    view_h() - BAR_H
}
const BTN_H: i32 = 32;
/// Field frame height (edit 20 + 8 padding above and below).
const FIELD_H: i32 = 36;
const SWITCH_Y: i32 = 168;
const SWITCH_ROW: i32 = 30;
/// Switch button width; its pill ends 2px inside, i.e. on the right padding.
const SWITCH_BTN_W: i32 = 44;
const TIMING_Y: i32 = 312;
const NUM_GAP: i32 = 16;
fn num_w() -> i32 {
    (content_w() - 2 * NUM_GAP) / 3
}
/// Vertical stride of the timing grid: two-line label + gap + field + gap.
const NUM_ROW: i32 = 84;
const NUM_LABEL_H: i32 = 32;
const LIST_ROW: i32 = 40;
/// Animation slot (settings window): the pulsing Discord status dot.
const SLOT_DOT: u32 = 1;

pub const TAB_GENERAL: usize = 0;
pub const TAB_DISCORD: usize = 1;
pub const TAB_PROFILES: usize = 2;
pub const TAB_UPDATES: usize = 3;
const TAB_KEYS: [&str; 4] = ["settings.tab_general", "settings.tab_discord", "settings.tab_profiles", "settings.tab_updates"];

const ID_TAB: i32 = 400; // 400..404
const ID_THEME: i32 = 410;
const ID_LANGUAGE: i32 = 411;
const ID_TOGGLE: i32 = 412; // 412..415
/// Store build: opens Windows Settings > Apps > Startup (replaces the first switch).
#[cfg(feature = "store")]
const ID_STARTUP_APPS: i32 = 416;
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
#[cfg(not(feature = "store"))]
const ID_REPO: i32 = 450;
#[cfg(not(feature = "store"))]
const ID_CHECK: i32 = 451;
const ID_SAVE: i32 = 460;
const ID_CLOSE: i32 = 461;
const IDCANCEL_KEY: i32 = 2;

const TOGGLES: [&str; 4] = ["settings.start_with_windows", "settings.minimize_to_tray", "settings.launch_minimized", "settings.notifications"];
/// (label key, tooltip key, column, row) of the timing fields: cooldown is in
/// seconds, the rest in ms.
const NUMS: [(&str, &str, i32, i32); 5] = [
    ("settings.cooldown", "tip.global_cooldown", 0, 0),
    ("settings.combo_window", "tip.combo_window", 1, 0),
    ("settings.sequence_timeout", "tip.sequence_timeout", 2, 0),
    ("settings.double_tap", "tip.double_tap", 0, 1),
    ("settings.long_press", "tip.long_press", 1, 1),
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
    /// Room left of Record/Clear for the shortcut keycaps (96-DPI units).
    shortcut_w: i32,
    recording: bool,
    switch_info: String,
    #[cfg(not(feature = "store"))]
    repo: HWND,
    #[cfg(not(feature = "store"))]
    btn_check: HWND,
    #[cfg(not(feature = "store"))]
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

pub fn open_tab(main: HWND, tab: usize) {
    let open = se(|s| s.hwnd).is_some();
    if !open && !page::go_home() {
        return;
    }
    main_window::show_main();
    if !open {
        if page::create(CLASS, main).is_null() {
            return;
        }
        load();
    }
    se(|s| s.tab = tab);
    layout();
    if !open {
        page::show_page(hwnd());
    }
}

fn comma() -> bool {
    i18n::language().decimal_comma()
}

fn load() {
    let cfg = app::with(|a| a.config.clone());
    let Some((toggles, nums)) = se(|s| (s.toggles, s.nums)) else { return };
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
    #[cfg(not(feature = "store"))]
    if let Some(repo) = se(|s| s.repo) {
        set_text(repo, &cfg.update_repo);
    }
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
    set_text(theme_btn, &t(theme.effective().key()));
    set_text(language_btn, language.native_name());
    let selected = selected_profile();
    unsafe {
        super::kit::reset_list_hover(list);
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
        kit::set_pulsing(btn, recording);
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

#[cfg(not(feature = "store"))]
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

#[cfg(not(feature = "store"))]
fn set_status(text: &str) {
    if let Some(h) = se(|s| {
        s.update_status = text.to_string();
        s.hwnd
    }) {
        invalidate(h);
    }
}

#[cfg(not(feature = "store"))]
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

#[cfg(not(feature = "store"))]
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

#[cfg(not(feature = "store"))]
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
    let Some((hwnd, toggles, nums)) = se(|s| (s.hwnd, s.toggles, s.nums)) else { return };
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
    let mut next = app::with(|a| a.config.clone());
    #[cfg(not(feature = "store"))]
    {
        let repo = se(|s| get_text(s.repo)).unwrap_or_default().trim().to_string();
        if !repo.is_empty() && !update::is_valid_repo(&repo) {
            alert(hwnd, &t("err.repo"));
            return;
        }
        next.update_repo = repo;
    }
    // Store build: autostart lives in Windows Settings, keep the stored value.
    #[cfg(not(feature = "store"))]
    {
        next.start_with_windows = is_checked(toggles[0]);
    }
    next.minimize_to_tray = is_checked(toggles[1]);
    next.launch_minimized = is_checked(toggles[2]);
    next.notifications = is_checked(toggles[3]);
    [next.cooldown_ms, next.combo_window_ms, next.sequence_timeout_ms, next.double_tap_ms, next.long_press_ms] = values;
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
    page::closed();
}

/// The window changed size.
pub fn resize(by: page::Resize) {
    let Some((hwnd, toggles, columns, wide)) = se(|s| {
        s.shortcut_w += view_w() - by.old_width;
        let columns = (s.theme_btn, s.language_btn, s.nums);
        #[cfg(not(feature = "store"))]
        let wide = vec![s.token, s.profiles, s.profile_name, s.repo];
        #[cfg(feature = "store")]
        let wide = vec![s.token, s.profiles, s.profile_name];
        (s.hwnd, s.toggles, columns, wide)
    }) else {
        return;
    };
    let item = |id| unsafe { GetDlgItem(hwnd, id) };
    let bar = [item(ID_CLOSE), item(ID_SAVE)];
    let mut right = toggles.to_vec();
    right.extend(
        [ID_CONNECT, ID_DISCONNECT, ID_P_ADD, ID_P_RENAME, ID_P_DUPLICATE, ID_P_DELETE, ID_SWITCH_RECORD, ID_SWITCH_CLEAR]
            .into_iter()
            .map(item),
    );
    #[cfg(not(feature = "store"))]
    right.push(item(ID_CHECK));
    right.extend(bar);
    page::anchor(hwnd, by, &right, &wide, &bar);
    view::place_columns(columns.0, columns.1, columns.2);
}

pub fn dialog_message(msg: &MSG) -> bool {
    match se(|s| s.hwnd) {
        Some(h) => unsafe { IsDialogMessageW(h, msg) != 0 },
        None => false,
    }
}

/// A tab button was clicked: the underline slides when the mouse did it.
fn click_tab(hwnd: HWND, new: usize) {
    if let Some((old, tabs)) = se(|s| (std::mem::replace(&mut s.tab, new), s.tabs))
        && anim::by_pointer()
    {
        kit::slide_tab(hwnd, tabs[old], tabs[new]);
    }
    layout();
}

fn on_command(id: i32, code: u32) {
    let Some(hwnd) = se(|s| s.hwnd) else { return };
    match id {
        _ if (ID_TAB..ID_TAB + 4).contains(&id) => click_tab(hwnd, (id - ID_TAB) as usize),
        ID_THEME => {
            let Some(btn) = se(|s| s.theme_btn) else { return };
            let current = app::with(|a| a.config.theme).effective();
            let items: Vec<String> = Theme::SELECTABLE.iter().map(|th| t(th.key())).collect();
            let checked: Vec<usize> = Theme::SELECTABLE.iter().position(|th| *th == current).into_iter().collect();
            if let Some(i) = popup_menu(hwnd, btn, &items, &checked) {
                if let Err(e) = app::set_theme(Theme::SELECTABLE[i]) {
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
        #[cfg(not(feature = "store"))]
        ID_CHECK => check_updates(),
        #[cfg(feature = "store")]
        ID_STARTUP_APPS => open_startup_settings(),
        ID_SAVE => save(),
        ID_CLOSE | IDCANCEL_KEY | page::ID_HOME => close(),
        _ if code == EN_SETFOCUS || code == EN_KILLFOCUS => invalidate(hwnd),
        _ => {}
    }
}

/// Store build: the Run key is not usable inside an MSIX package, so the
/// autostart switch is replaced by a shortcut to Windows' own startup list.
/// ShellExecute runs on a worker thread (it may block); it cannot fail for
/// this built-in URI on supported Windows versions, so the result is ignored.
#[cfg(feature = "store")]
fn open_startup_settings() {
    std::thread::spawn(|| {
        let _ = crate::win::shell::open("ms-settings:startupapps", "");
    });
}

/// A click on a switch row's label toggles its switch (bigger hit area).
/// Client coordinates are already mirrored in Hebrew, so left-based rows work.
fn on_click(lparam: LPARAM) {
    let (x, y) = ((lparam & 0xFFFF) as i16 as i32, ((lparam >> 16) & 0xFFFF) as i16 as i32);
    let Some((tab, toggles)) = se(|s| (s.tab, s.toggles)) else { return };
    if tab != TAB_GENERAL || x < px(PAD) || x >= px(view_w() - PAD - SWITCH_BTN_W) || y < px(SWITCH_Y) {
        return;
    }
    let row = ((y - px(SWITCH_Y)) / px(SWITCH_ROW)) as usize;
    // Store build: row 0 holds the Windows Settings button, not a switch.
    if cfg!(feature = "store") && row == 0 {
        return;
    }
    if let Some(&btn) = toggles.get(row) {
        kit::toggle_animated(btn);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // The profile list sits on the window background (rows draw their own fills).
    if msg == WM_CTLCOLORLISTBOX && unsafe { GetDlgCtrlID(lparam as HWND) } == ID_PROFILES {
        let p = palette();
        unsafe {
            SetTextColor(wparam as HDC, p.text);
            SetBkColor(wparam as HDC, p.bg);
        }
        return super::theme::bg_brush() as LRESULT;
    }
    if let Some(r) = ctl_color(msg, wparam) {
        return r;
    }
    match msg {
        WM_CREATE => {
            view::on_create(hwnd);
            0
        }
        WM_PAINT => {
            view::on_paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MEASUREITEM => {
            let mis = unsafe { &mut *(lparam as *mut MEASUREITEMSTRUCT) };
            mis.itemHeight = px(LIST_ROW) as u32;
            1
        }
        WM_LBUTTONDOWN => {
            on_click(lparam);
            0
        }
        WM_DRAWITEM => {
            let dis = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            if dis.CtlType == ODT_LISTBOX {
                view::draw_profile_row(dis);
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
