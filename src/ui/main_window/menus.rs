//! Popup menus of the main window: More (also the list's context menu),
//! profile and sort.

use super::view::SORT_KEYS;
use super::{refresh, select, selected_binding, st};
use crate::app;
use crate::i18n::{t, tf};
use crate::ui::{alert, confirm, popup_menu, settings};
use std::time::Duration;
use windows_sys::Win32::Foundation::HWND;

pub(super) fn more_menu(hwnd: HWND, anchor: HWND) {
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

pub(super) fn profile_menu(hwnd: HWND, anchor: HWND) {
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

pub(super) fn sort_menu(hwnd: HWND, anchor: HWND) {
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
