//! Main window: the "disable all" switch, profile, search and sort, the
//! binding list, details of the selected binding, the run button and status.

use super::theme::palette;
use super::*;
use crate::app::{self, actions_summary, profile_name, App, Conn};
use crate::config::{ActionKind, Binding, TriggerMode};
use crate::i18n::{t, tf};
use std::cell::RefCell;
use std::time::Duration;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{Ellipse, HDC};

const CLASS: &str = "NoKeyMain";
const W: i32 = 560;
const H: i32 = 666;
const ROW_H: i32 = 44;
const HEADER_H: i32 = 52;

const ID_LIST: i32 = 100;
const ID_NEW: i32 = 101;
const ID_EDIT: i32 = 102;
const ID_MORE: i32 = 103;
const ID_RUN: i32 = 105;
const ID_SETTINGS: i32 = 106;
const ID_DISABLE_ALL: i32 = 107;
const ID_PROFILE: i32 = 108;
const ID_SEARCH: i32 = 109;
const ID_SORT: i32 = 110;

const TIMER_FLASH: usize = 1;
const TIMER_COOLDOWN: usize = 2;
const FLASH_MS: u32 = 1500;
/// Cooldowns shorter than this don't lock the run button.
const MIN_LOCK_MS: u32 = 250;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SortKey {
    #[default]
    Name,
    Shortcut,
    Action,
    Profile,
    Enabled,
}

const SORT_KEYS: [(SortKey, &str); 5] = [
    (SortKey::Name, "sort.name"),
    (SortKey::Shortcut, "sort.shortcut"),
    (SortKey::Action, "sort.action"),
    (SortKey::Profile, "sort.profile"),
    (SortKey::Enabled, "sort.enabled"),
];

#[derive(Default, Clone)]
struct Row {
    id: u32,
    icon: String,
    name: String,
    summary: String,
    chip: String,
    enabled: bool,
    paused: bool,
    problem: bool,
}

#[derive(Default, Clone)]
struct View {
    rows: Vec<Row>,
    total: usize,
    connected: bool,
    conn_short: String,
    all_enabled: bool,
    emergency_error: Option<String>,
    profile: String,
    last_ping: String,
    last_result: Option<(bool, String)>,
    detail: [String; 2],
    run_label: Option<String>,
    selected_enabled: bool,
    selected_paused: bool,
}

struct State {
    hwnd: HWND,
    list: HWND,
    search: HWND,
    btn_disable_all: HWND,
    btn_profile: HWND,
    btn_sort: HWND,
    btn_edit: HWND,
    btn_more: HWND,
    btn_run: HWND,
    selected: Option<u32>,
    view: View,
    flash: Option<(u32, bool)>,
    cooling: bool,
    sort: SortKey,
    show_all: bool,
    told_about_tray: bool,
}

thread_local! {
    static ST: RefCell<Option<State>> = const { RefCell::new(None) };
    /// View options that survive a rebuild (language change).
    static KEEP: RefCell<(SortKey, bool, Option<u32>)> = const { RefCell::new((SortKey::Name, false, None)) };
}

fn st<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    ST.with(|s| s.try_borrow_mut().ok()?.as_mut().map(f))
}

pub fn register(icon: HICON) {
    register_class(CLASS, Some(wndproc), icon);
}

pub fn create() {
    create_window(CLASS, "NoKey", W, H, null_mut());
    refresh();
}

pub fn hwnd() -> HWND {
    st(|s| s.hwnd).unwrap_or(null_mut())
}

pub fn is_foreground() -> bool {
    let h = hwnd();
    !h.is_null() && unsafe { GetForegroundWindow() } == h
}

pub fn show_main() {
    let h = hwnd();
    if h.is_null() {
        return;
    }
    unsafe {
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        } else {
            ShowWindow(h, SW_SHOW);
        }
        SetForegroundWindow(h);
    }
}

pub fn show_minimized() {
    let h = hwnd();
    if !h.is_null() {
        unsafe { ShowWindow(h, SW_SHOWMINNOACTIVE) };
    }
}

pub fn destroy() {
    let h = hwnd();
    if let Some(keep) = st(|s| (s.sort, s.show_all, s.selected)) {
        KEEP.with(|k| *k.borrow_mut() = keep);
    }
    ST.with(|s| *s.borrow_mut() = None);
    if !h.is_null() {
        unsafe { DestroyWindow(h) };
    }
}

/// Re-creates the window (new language / layout direction).
pub fn rebuild() {
    let was_visible = is_visible(hwnd());
    destroy();
    create();
    if was_visible {
        show_main();
    }
}

pub fn select(id: u32) {
    st(|s| {
        s.selected = Some(id);
        s.cooling = false;
    });
    refresh();
}

fn chip(b: &Binding) -> String {
    match b.shortcuts.as_slice() {
        [] => t("main.no_shortcut"),
        [one] => one.label(),
        [first, rest @ ..] => format!("{} +{}", first.label(), rest.len()),
    }
}

fn matches_query(a: &App, b: &Binding, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {} {}",
        b.name,
        b.description,
        actions_summary(b),
        b.shortcuts.iter().map(|s| s.label()).collect::<Vec<_>>().join(" "),
        profile_name(&a.config, b.profile)
    )
    .to_lowercase();
    q.split_whitespace().all(|w| hay.contains(w))
}

fn build_view(a: &mut App, selected: Option<u32>, sort: SortKey, show_all: bool, query: &str) -> View {
    let paused = a.paused_ids();
    let q = query.trim().to_lowercase();
    let mut list: Vec<&Binding> = a
        .config
        .bindings
        .iter()
        .filter(|b| show_all || a.config.in_active_profile(b))
        .filter(|b| matches_query(a, b, &q))
        .collect();
    match sort {
        SortKey::Name => list.sort_by_key(|b| b.name.to_lowercase()),
        SortKey::Shortcut => list.sort_by_key(|b| (b.shortcuts.is_empty(), chip(b).to_lowercase())),
        SortKey::Action => list.sort_by_key(|b| (t(b.actions.first().map_or("", |x| x.kind().key())), b.name.to_lowercase())),
        SortKey::Profile => list.sort_by_key(|b| (profile_name(&a.config, b.profile), b.name.to_lowercase())),
        SortKey::Enabled => list.sort_by_key(|b| (!b.enabled, b.name.to_lowercase())),
    }
    let rows = list
        .iter()
        .map(|b| {
            let mut summary = actions_summary(b);
            if b.trigger.mode != TriggerMode::Press {
                summary = format!("{} · {summary}", t(b.trigger.mode.key()));
            }
            if show_all {
                summary = format!("{} · {summary}", profile_name(&a.config, b.profile));
            }
            let paused_now = paused.contains(&b.id);
            Row {
                id: b.id,
                icon: if b.icon.is_empty() { b.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default() } else { b.icon.clone() },
                name: b.name.clone(),
                summary,
                chip: if !b.enabled {
                    t("main.off")
                } else if paused_now {
                    t("main.paused")
                } else {
                    chip(b)
                },
                enabled: b.enabled && !paused_now,
                paused: paused_now,
                problem: a.hotkey_errors.contains_key(&b.id),
            }
        })
        .collect();
    let mut v = View {
        rows,
        total: a.config.bindings.iter().filter(|b| b.enabled && a.config.in_active_profile(b)).count(),
        connected: matches!(a.conn, Conn::Connected(_)),
        conn_short: match a.conn {
            Conn::Connected(_) => t("main.connected"),
            Conn::Checking => t("main.checking"),
            _ => t("main.disconnected"),
        },
        all_enabled: a.config.bindings_enabled,
        emergency_error: a.emergency_error.clone(),
        profile: profile_name(&a.config, a.config.active_profile),
        last_ping: if a.last_ping.is_empty() { "—".into() } else { a.last_ping.clone() },
        last_result: a.last_result.clone(),
        ..View::default()
    };
    if let Some(b) = selected.and_then(|id| a.config.binding(id)) {
        v.selected_enabled = b.enabled;
        v.selected_paused = paused.contains(&b.id);
        v.detail[0] = actions_summary(b);
        let keys = if b.shortcuts.is_empty() {
            t("main.no_shortcut")
        } else {
            b.shortcuts.iter().map(|s| s.label()).collect::<Vec<_>>().join("  |  ")
        };
        let problem = a.hotkey_errors.get(&b.id).map(|e| format!("  ⚠ {e}")).unwrap_or_default();
        v.detail[1] = format!("{keys} · {} · {}{problem}", t(b.trigger.mode.key()), profile_name(&a.config, b.profile));
        let all_discord = b.actions.iter().all(|x| x.kind() == ActionKind::DiscordPing);
        v.run_label = Some(t(if all_discord { "main.run_ping" } else { "main.run" }));
    }
    v
}

/// Rebuilds the view from the app state and updates every control.
pub fn refresh() {
    let Some((hwnd, selected, sort, show_all, search)) = st(|s| (s.hwnd, s.selected, s.sort, s.show_all, s.search)) else {
        return;
    };
    let query = crate::win::get_text(search);
    let (view, selected) = app::with(|a| {
        let visible = build_view(a, None, sort, show_all, &query);
        let sel = selected.filter(|id| visible.rows.iter().any(|r| r.id == *id)).or_else(|| visible.rows.first().map(|r| r.id));
        (build_view(a, sel, sort, show_all, &query), sel)
    });
    let Some((list, edit, more, run, disable_all, profile, sort_btn, label)) = st(|s| {
        s.selected = selected;
        let label = match s.flash {
            _ if view.run_label.is_none() => t("main.select_binding"),
            Some((id, ok)) if Some(id) == selected => t(if ok { "main.done" } else { "main.failed" }),
            _ => view.run_label.clone().unwrap_or_default(),
        };
        s.view = view.clone();
        (s.list, s.btn_edit, s.btn_more, s.btn_run, s.btn_disable_all, s.btn_profile, s.btn_sort, label)
    }) else {
        return;
    };

    unsafe {
        SendMessageW(list, WM_SETREDRAW, 0, 0);
        SendMessageW(list, LB_RESETCONTENT, 0, 0);
        let mut sel_index = -1isize;
        for (i, row) in view.rows.iter().enumerate() {
            SendMessageW(list, LB_ADDSTRING, 0, row.id as LPARAM);
            if Some(row.id) == selected {
                sel_index = i as isize;
            }
        }
        SendMessageW(list, LB_SETCURSEL, sel_index as WPARAM, 0);
        SendMessageW(list, WM_SETREDRAW, 1, 0);
        InvalidateRect(list, null(), 1);
    }
    let has_sel = selected.is_some();
    enable(edit, has_sel);
    enable(more, has_sel);
    crate::win::set_text(run, &label);
    let cooling = st(|s| s.cooling).unwrap_or(false);
    enable(run, has_sel && !cooling);
    crate::win::set_text(disable_all, &t(if view.all_enabled { "main.disable_all" } else { "main.enable_all" }));
    set_kind(disable_all, if view.all_enabled { BTN_BIG } else { BTN_WARN });
    crate::win::set_text(profile, &view.profile);
    let sort_label = SORT_KEYS.iter().find(|(k, _)| *k == sort).map(|(_, key)| t(key)).unwrap_or_default();
    crate::win::set_text(sort_btn, &sort_label);
    invalidate(hwnd);
}

/// Called right after a binding was started from the window: locks the run
/// button for the cooldown.
pub fn after_run(id: u32) {
    let cooldown_ms = app::with(|a| a.config.binding(id).map(|b| a.config.cooldown_for(b)).unwrap_or(0));
    st(|s| {
        if s.selected == Some(id) && cooldown_ms >= MIN_LOCK_MS {
            s.cooling = true;
            unsafe { SetTimer(s.hwnd, TIMER_COOLDOWN, cooldown_ms, None) };
        }
    });
    refresh();
}

/// Briefly shows success or failure on the run button.
pub fn flash(id: u32, ok: bool) {
    st(|s| {
        s.flash = Some((id, ok));
        unsafe { SetTimer(s.hwnd, TIMER_FLASH, FLASH_MS, None) };
    });
    refresh();
}

fn on_create(hwnd: HWND) {
    let b = |id, text: &str, kind, x, y, w, h| {
        let btn = button(hwnd, id, text, kind);
        place(btn, x, y, w, h);
        btn
    };
    let btn_settings = b(ID_SETTINGS, &t("main.settings"), BTN_SECONDARY, W - 24 - 120, 12, 120, 30);
    let btn_disable_all = b(ID_DISABLE_ALL, &t("main.disable_all"), BTN_BIG, 24, 60, W - 48, 48);
    let btn_profile = b(ID_PROFILE, "", BTN_DROPDOWN, 24, 136, 170, 32);
    let search = edit(hwnd, ID_SEARCH, 0);
    place(search, 212, 142, 196, 20);
    set_cue(search, &t("main.search"));
    let btn_sort = b(ID_SORT, "", BTN_DROPDOWN, 426, 136, 110, 32);
    let list = listbox(hwnd, ID_LIST, 0);
    place(list, 24, 202, W - 48, 4 * ROW_H);
    b(ID_NEW, &t("main.new"), BTN_PRIMARY, 24, 388, 160, 32);
    let btn_edit = b(ID_EDIT, &t("main.edit"), BTN_SECONDARY, 192, 388, 110, 32);
    let btn_more = b(ID_MORE, &t("main.more"), BTN_DROPDOWN, 310, 388, 140, 32);
    let btn_run = b(ID_RUN, &t("main.select_binding"), BTN_PRIMARY, 24, 508, W - 48, 50);

    add_tooltip(hwnd, btn_disable_all, &t("tip.disable_all"));
    add_tooltip(hwnd, btn_profile, &t("tip.profile"));
    add_tooltip(hwnd, search, &t("tip.search"));
    add_tooltip(hwnd, btn_sort, &t("tip.sort"));
    add_tooltip(hwnd, btn_more, &t("tip.more"));
    add_tooltip(hwnd, btn_settings, &t("tip.settings"));

    let (sort, show_all, selected) = KEEP.with(|k| *k.borrow());
    ST.with(|s| {
        *s.borrow_mut() = Some(State {
            hwnd,
            list,
            search,
            btn_disable_all,
            btn_profile,
            btn_sort,
            btn_edit,
            btn_more,
            btn_run,
            selected,
            view: View::default(),
            flash: None,
            cooling: false,
            sort,
            show_all,
            told_about_tray: false,
        })
    });
}

fn dot(hdc: HDC, x: i32, y: i32, filled: bool) {
    let p = palette();
    let rad = px(4);
    let rc = RECT { left: x - rad, top: y - rad, right: x + rad, bottom: y + rad };
    if filled {
        unsafe {
            let b = CreateSolidBrush(p.accent);
            let pen = CreatePen(PS_SOLID, 1, p.accent);
            let ob = SelectObject(hdc, b as _);
            let op = SelectObject(hdc, pen as _);
            Ellipse(hdc, rc.left, rc.top, rc.right, rc.bottom);
            SelectObject(hdc, ob);
            SelectObject(hdc, op);
            DeleteObject(b as _);
            DeleteObject(pen as _);
        }
    } else {
        round_rect(hdc, &rc, p.muted, None, 8);
    }
}

fn on_paint(hwnd: HWND) {
    let Some((v, search)) = st(|s| (s.view.clone(), s.search)) else { return };
    paint(hwnd, |hdc, _| {
        let p = palette();
        let f = fonts();
        header_band(hdc, W, HEADER_H);
        text(hdc, "NoKey", &r(24, 10, 200, 34), f.title, p.text, DT_SINGLELINE | DT_VCENTER);

        // Discord connection, right-aligned before the Settings button
        let right = px(W - 24 - 120 - 14);
        let tw = text_width(hdc, &v.conn_short, f.small);
        let tr = RECT { left: right - tw, top: px(12), right, bottom: px(42) };
        let color = if v.connected { p.text } else { p.muted };
        text(hdc, &v.conn_short, &tr, f.small, color, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
        dot(hdc, tr.left - px(12), px(27), v.connected);

        let status = match (&v.emergency_error, v.all_enabled) {
            (Some(e), _) => format!("⚠ {e}"),
            (None, true) => t("main.status_enabled"),
            (None, false) => t("main.status_disabled"),
        };
        let status_color = if v.all_enabled { p.muted } else { p.text };
        text(hdc, &status, &r(24, 112, W - 48, 18), f.small, status_color, DT_SINGLELINE | DT_CENTER | DT_END_ELLIPSIS);

        edit_frames(hwnd, hdc, &[search]);
        caption(hdc, &t("main.bindings"), 24, 182, 260);
        let count = tf("main.active_count", &[("count", &v.total.to_string())]);
        text(hdc, &count, &r(W - 224, 182, 200, 18), f.small, p.muted, DT_SINGLELINE | DT_RIGHT);
        if v.rows.is_empty() {
            let area = r(24, 202, W - 48, 4 * ROW_H);
            round_rect(hdc, &area, p.line, None, 10);
            let empty = if crate::win::get_text(search).trim().is_empty() { t("main.empty") } else { t("main.nothing_found") };
            text(hdc, &empty, &area, f.normal, p.muted, DT_CENTER | DT_VCENTER | DT_WORDBREAK);
        }

        hline(hdc, 24, 432, W - 48);
        caption(hdc, &t("main.selected"), 24, 442, 300);
        if v.run_label.is_none() {
            text(hdc, &t("main.nothing_selected"), &r(24, 462, W - 48, 20), f.normal, p.muted, DT_SINGLELINE);
        }
        for (i, line) in v.detail.iter().enumerate() {
            let font = if i == 0 { f.normal } else { f.small };
            let color = if i == 0 { p.text } else { p.muted };
            text(hdc, line, &r(24, 462 + 20 * i as i32, W - 48, 20), font, color, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
        }

        hline(hdc, 24, 572, W - 48);
        caption(hdc, &t("main.status"), 24, 582, 300);
        // The value column starts after the longest label (labels differ a lot between languages).
        let (ping_label, result_label) = (t("main.last_ping"), t("main.result"));
        let label_w = text_width(hdc, &ping_label, f.small).max(text_width(hdc, &result_label, f.small));
        let vx = (px(24) + label_w + px(16)).max(px(134));
        let value = |y: i32, h: i32| RECT { left: vx, top: px(y), right: px(W - 24), bottom: px(y + h) };
        text(hdc, &ping_label, &r(24, 602, W - 48, 20), f.small, p.muted, DT_SINGLELINE | DT_VCENTER);
        text(hdc, &v.last_ping, &value(602, 20), f.normal, p.text, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
        let result = match &v.last_result {
            Some((ok, msg)) => format!("{} {msg}", if *ok { "✓" } else { "✕" }),
            None => t("main.ready"),
        };
        text(hdc, &result_label, &r(24, 622, W - 48, 20), f.small, p.muted, DT_SINGLELINE | DT_VCENTER);
        text(hdc, &result, &value(622, 40), f.normal, p.text, DT_WORDBREAK | DT_END_ELLIPSIS);
    });
}

fn draw_row(dis: &DRAWITEMSTRUCT) {
    if dis.itemID == u32::MAX {
        return;
    }
    let Some(row) = st(|s| s.view.rows.get(dis.itemID as usize).cloned()).flatten() else { return };
    let hdc = dis.hDC;
    let rc = dis.rcItem;
    let p = palette();
    let f = fonts();
    let selected = dis.itemState & ODS_SELECTED != 0;
    fill(hdc, &rc, if selected { p.selected } else { p.surface });
    if selected {
        fill(hdc, &RECT { right: rc.left + px(3), ..rc }, p.accent);
    }
    let main_color = if row.enabled { p.text } else { p.muted };

    let icon = RECT { left: rc.left + px(10), top: rc.top + px(8), right: rc.left + px(38), bottom: rc.bottom - px(8) };
    round_rect(hdc, &icon, p.border, Some(p.bg), 8);
    text(hdc, &row.icon, &icon, f.bold, main_color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    let chip_w = text_width(hdc, &row.chip, f.small) + px(20);
    let chip = RECT { left: rc.right - px(12) - chip_w, top: rc.top + px(11), right: rc.right - px(12), bottom: rc.bottom - px(11) };
    let outline = if row.problem || row.paused { p.accent } else { p.border };
    round_rect(hdc, &chip, outline, Some(p.bg), 8);
    let chip_text = if row.problem { format!("⚠ {}", row.chip) } else { row.chip.clone() };
    text(hdc, &chip_text, &chip, f.small, main_color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    let text_right = chip.left - px(10);
    let left = icon.right + px(10);
    let name_rc = RECT { left, top: rc.top + px(4), right: text_right, bottom: rc.top + px(24) };
    text(hdc, &row.name, &name_rc, f.bold, main_color, DT_SINGLELINE | DT_END_ELLIPSIS);
    let sum_rc = RECT { left, top: rc.top + px(23), right: text_right, bottom: rc.bottom - px(3) };
    text(hdc, &row.summary, &sum_rc, f.small, p.muted, DT_SINGLELINE | DT_END_ELLIPSIS);
    fill(hdc, &RECT { top: rc.bottom - 1, ..rc }, p.line);
}

fn selected_binding() -> Option<Binding> {
    let id = st(|s| s.selected).flatten()?;
    app::with(|a| a.config.binding(id).cloned())
}

fn more_menu(hwnd: HWND, anchor: HWND) {
    let Some(b) = selected_binding() else { return };
    let paused = st(|s| s.view.selected_paused).unwrap_or(false);
    let mut items = vec![
        t("main.duplicate"),
        t("main.delete"),
        t(if b.enabled { "main.disable" } else { "main.enable" }),
        "-".to_string(),
        t("main.pause_15m"),
        t("main.pause_1h"),
        t("main.pause_restart"),
    ];
    if paused {
        items.push(t("main.resume"));
    }
    match popup_menu(hwnd, anchor, &items, &[]) {
        Some(0) => match app::duplicate_binding(b.id) {
            Ok(id) => select(id),
            Err(e) => alert(hwnd, &e),
        },
        Some(1) => {
            if confirm(hwnd, &tf("main.confirm_delete", &[("name", &b.name)]))
                && let Err(e) = app::delete_binding(b.id)
            {
                alert(hwnd, &e);
            }
        }
        Some(2) => match app::set_binding_enabled(b.id, !b.enabled) {
            Ok(Some(problem)) => alert(hwnd, &tf("err.hotkey_taken", &[("details", &problem)])),
            Ok(None) => {}
            Err(e) => alert(hwnd, &e),
        },
        Some(4) => app::pause_binding(b.id, Some(Duration::from_secs(15 * 60))),
        Some(5) => app::pause_binding(b.id, Some(Duration::from_secs(60 * 60))),
        Some(6) => app::pause_binding(b.id, None),
        Some(7) => app::resume_binding(b.id),
        _ => {}
    }
}

fn profile_menu(hwnd: HWND, anchor: HWND) {
    let (profiles, active) = app::with(|a| (a.config.profiles.clone(), a.config.active_profile));
    let mut items: Vec<String> = profiles.iter().map(|p| p.name.clone()).collect();
    let checked: Vec<usize> = profiles.iter().position(|p| p.id == active).into_iter().collect();
    items.push("-".into());
    items.push(t("main.manage_profiles"));
    match popup_menu(hwnd, anchor, &items, &checked) {
        Some(i) if i < profiles.len() => app::set_active_profile(profiles[i].id),
        Some(i) if i == items.len() - 1 => settings::open_tab(hwnd, settings::TAB_PROFILES),
        _ => {}
    }
}

fn sort_menu(hwnd: HWND, anchor: HWND) {
    let Some((sort, show_all)) = st(|s| (s.sort, s.show_all)) else { return };
    let mut items: Vec<String> = SORT_KEYS.iter().map(|(_, k)| t(k)).collect();
    let mut checked: Vec<usize> = SORT_KEYS.iter().position(|(k, _)| *k == sort).into_iter().collect();
    items.push("-".into());
    items.push(t("main.show_all_profiles"));
    if show_all {
        checked.push(items.len() - 1);
    }
    match popup_menu(hwnd, anchor, &items, &checked) {
        Some(i) if i < SORT_KEYS.len() => {
            st(|s| s.sort = SORT_KEYS[i].0);
        }
        Some(i) if i == items.len() - 1 => {
            st(|s| s.show_all = !s.show_all);
        }
        _ => return,
    }
    refresh();
}

fn hide_to_tray(hwnd: HWND) {
    unsafe { ShowWindow(hwnd, SW_HIDE) };
    let first = st(|s| !std::mem::replace(&mut s.told_about_tray, true)).unwrap_or(false);
    let (notify, host) = app::with(|a| (a.config.notifications, a.host));
    if first && notify {
        crate::win::tray::notify(host, "NoKey", &t("notify.still_running"), false);
    }
}

fn select_at_cursor(list: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut pt);
        ScreenToClient(list, &mut pt);
    }
    let packed = ((pt.y as u32 & 0xFFFF) << 16 | (pt.x as u32 & 0xFFFF)) as LPARAM;
    let hit = unsafe { SendMessageW(list, LB_ITEMFROMPOINT, 0, packed) } as u32;
    if hit >> 16 == 0 {
        let id = unsafe { SendMessageW(list, LB_GETITEMDATA, (hit & 0xFFFF) as WPARAM, 0) } as u32;
        select(id);
    }
}

fn on_command(hwnd: HWND, id: i32, code: u32) {
    let ctl = |f: fn(&State) -> HWND| st(|s| f(s)).unwrap_or(null_mut());
    match id {
        ID_LIST if code == LBN_SELCHANGE => {
            let list = ctl(|s| s.list);
            let idx = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
            if idx >= 0 {
                let data = unsafe { SendMessageW(list, LB_GETITEMDATA, idx as WPARAM, 0) } as u32;
                st(|s| {
                    s.selected = Some(data);
                    s.cooling = false;
                    unsafe { KillTimer(s.hwnd, TIMER_COOLDOWN) };
                });
                refresh();
            }
        }
        ID_LIST if code == LBN_DBLCLK => on_command(hwnd, ID_EDIT, 0),
        ID_SEARCH if code == EN_CHANGE => refresh(),
        ID_SEARCH if code == EN_SETFOCUS || code == EN_KILLFOCUS => invalidate(hwnd),
        ID_NEW => editor::open(hwnd, None),
        ID_EDIT => {
            if let Some(b) = selected_binding() {
                editor::open(hwnd, Some(b));
            }
        }
        ID_MORE => more_menu(hwnd, ctl(|s| s.btn_more)),
        ID_RUN => {
            if let Some(b) = selected_binding() {
                app::run_binding(b.id);
            }
        }
        ID_SETTINGS => settings::open(hwnd),
        ID_DISABLE_ALL => app::toggle_all(),
        ID_PROFILE => profile_menu(hwnd, ctl(|s| s.btn_profile)),
        ID_SORT => sort_menu(hwnd, ctl(|s| s.btn_sort)),
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
            mis.itemHeight = px(ROW_H) as u32;
            1
        }
        WM_DRAWITEM => {
            let dis = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            if dis.CtlType == ODT_LISTBOX {
                draw_row(dis);
            } else {
                draw_button(dis);
            }
            1
        }
        WM_COMMAND => {
            let (id, code) = command_parts(wparam);
            on_command(hwnd, id, code);
            0
        }
        WM_CONTEXTMENU => {
            let list = st(|s| s.list).unwrap_or(null_mut());
            if wparam as HWND == list {
                select_at_cursor(list);
                more_menu(hwnd, list);
            }
            0
        }
        WM_TIMER => {
            unsafe { KillTimer(hwnd, wparam) };
            st(|s| match wparam {
                TIMER_FLASH => s.flash = None,
                TIMER_COOLDOWN => s.cooling = false,
                _ => {}
            });
            refresh();
            0
        }
        WM_SYSCOMMAND if (wparam & 0xFFF0) as u32 == SC_MINIMIZE => {
            if app::with(|a| a.config.minimize_to_tray) {
                hide_to_tray(hwnd);
                0
            } else {
                def(hwnd, msg, wparam, lparam)
            }
        }
        WM_CLOSE => {
            if app::with(|a| a.config.minimize_to_tray) {
                hide_to_tray(hwnd);
            } else {
                app::exit();
            }
            0
        }
        _ => def(hwnd, msg, wparam, lparam),
    }
}
