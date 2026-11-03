//! Hidden host window. It owns the global hotkeys, the tray icon, the
//! timers and receives messages from the keyboard hook and worker threads,
//! so the visible windows can be destroyed and re-created freely.

use super::*;
use crate::app::{self, HOST_CLASS};
use crate::i18n::t;
use crate::win::{tray, WM_APP_FIRE, WM_APP_RECORD, WM_APP_SHOW, WM_APP_TASK, WM_APP_TRAY};
use std::cell::Cell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};

const ID_OPEN: usize = 1;
const ID_TOGGLE_ALL: usize = 2;
const ID_SETTINGS: usize = 3;
const ID_EXIT: usize = 4;
const ID_PROFILE_BASE: usize = 100;

/// Also used by tests to close the app cleanly (WM_COMMAND with this id).
pub const EXIT_COMMAND: usize = ID_EXIT;

thread_local! {
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static TRAY_ICON: Cell<usize> = const { Cell::new(0) };
}

pub fn register() {
    register_class(HOST_CLASS, Some(wndproc), null_mut());
}

/// Creates the hidden window (a real top-level window, not message-only,
/// so it receives broadcasts like TaskbarCreated and WM_SETTINGCHANGE).
pub fn create(tray_icon: HICON) -> HWND {
    TRAY_ICON.with(|c| c.set(tray_icon as usize));
    let msg = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
    TASKBAR_CREATED.with(|c| c.set(msg));
    let class = wide(HOST_CLASS);
    let title = wide("NoKey");
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            hinstance(),
            null(),
        )
    }
}

fn tray_menu(hwnd: HWND) {
    let (enabled, profiles, active) =
        app::with(|a| (a.config.bindings_enabled, a.config.profiles.clone(), a.config.active_profile));
    unsafe {
        let menu = CreatePopupMenu();
        let add = |m: HMENU, id: usize, label: &str, flags: MENU_ITEM_FLAGS| {
            let w = wide(label);
            AppendMenuW(m, MF_STRING | flags, id, w.as_ptr());
        };
        add(menu, ID_OPEN, &t("tray.open"), 0);
        let sub = CreatePopupMenu();
        for p in &profiles {
            add(sub, ID_PROFILE_BASE + p.id as usize, &p.name, if p.id == active { MF_CHECKED } else { 0 });
        }
        let label = wide(&t("tray.profile"));
        AppendMenuW(menu, MF_POPUP, sub as usize, label.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        add(menu, ID_TOGGLE_ALL, &t(if enabled { "main.disable_all" } else { "main.enable_all" }), 0);
        add(menu, ID_SETTINGS, &t("tray.settings"), 0);
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        add(menu, ID_EXIT, &t("tray.exit"), 0);
        SetMenuDefaultItem(menu, ID_OPEN as u32, 0);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd); // required so the menu closes when clicking elsewhere
        let rtl = if crate::i18n::is_rtl() { TPM_LAYOUTRTL } else { 0 };
        let cmd =
            TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | rtl, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        on_command(cmd as usize);
    }
}

fn on_command(id: usize) {
    match id {
        ID_OPEN => main_window::show_main(),
        ID_TOGGLE_ALL => app::toggle_all(),
        ID_SETTINGS => settings::open(main_window::hwnd()),
        ID_EXIT => app::exit(),
        p if p >= ID_PROFILE_BASE => app::set_active_profile((p - ID_PROFILE_BASE) as u32),
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let taskbar_created = TASKBAR_CREATED.with(|c| c.get());
    if msg == taskbar_created && msg != 0 {
        // Explorer restarted: put the tray icon back.
        let icon = TRAY_ICON.with(|c| c.get()) as HICON;
        tray::add(hwnd, icon, "NoKey");
        app::tray_tip();
        return 0;
    }
    match msg {
        WM_HOTKEY => {
            app::on_hotkey(wparam as i32);
            0
        }
        WM_APP_FIRE => {
            app::on_key(wparam as u32, Some(lparam != 0));
            0
        }
        WM_APP_TASK => {
            app::drain_tasks();
            0
        }
        WM_APP_RECORD => {
            for update in crate::win::hook::take_record_updates() {
                if settings::is_recording() {
                    settings::on_record(update);
                } else {
                    editor::on_record(update);
                }
            }
            0
        }
        WM_APP_SHOW => {
            main_window::show_main();
            0
        }
        WM_APP_TRAY => {
            match (lparam & 0xFFFF) as u32 {
                WM_LBUTTONDBLCLK => main_window::show_main(),
                WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            on_command(wparam & 0xFFFF);
            0
        }
        WM_TIMER => {
            app::on_timer(wparam);
            0
        }
        WM_SETTINGCHANGE => {
            app::system_theme_changed();
            0
        }
        WM_ENDSESSION if wparam != 0 => {
            app::exit();
            0
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => def(hwnd, msg, wparam, lparam),
    }
}
