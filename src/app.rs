//! Application state and the glue between the hotkey engine, the action
//! engine, profiles and the windows.
//!
//! Everything runs on the UI thread except network and shell calls, which
//! run on short-lived worker threads and report back with `WM_APP_TASK`.
//! Hotkeys, the tray icon and timers belong to a hidden host window, so the
//! visible windows can be re-created (language change) without losing them.
//! Rule: never call into `ui` while `with()` holds the state borrowed —
//! Win32 may re-enter a window procedure synchronously.

use crate::actions::{self, RunReport, StepError};
use crate::config::{self, timing, *};
use crate::discord::api::{wait_duration, ApiError, Client};
use crate::discord::models::{Channel, Guild, User};
use crate::engine::matcher::Matcher;
use crate::engine::trigger::{Effect, TriggerEngine};
use crate::i18n::{self, t, tf};
use crate::security::dpapi;
use crate::services::cooldown::Cooldowns;
use crate::shortcut::{Shortcut, MOD_ALT, MOD_CTRL};
use crate::ui;
use crate::update::{self, ReleaseInfo};
use crate::win::shell::ShellFail;
use crate::win::{self, context, hook, hotkey, startup, tick, tray, PostTarget, WM_APP_SHOW, WM_APP_TASK};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, HWND, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows_sys::Win32::UI::Controls::{InitCommonControlsEx, ICC_STANDARD_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Ctrl+Alt+Pause. With Ctrl held, the Pause key reports VK_CANCEL on most
/// keyboards, so both codes are registered. Always active.
pub const EMERGENCY_IDS: [i32; 2] = [0xB000, 0xB001];
pub const PROFILE_SWITCH_HOTKEY: i32 = 0xB002;
/// Matcher id of the profile switch shortcut when it needs the hook.
pub const PROFILE_SWITCH_ID: u32 = 0xFFFF_0000;
const VK_PAUSE: u16 = 0x13;
const VK_CANCEL: u16 = 0x03;
pub const TIMER_REPEAT: usize = 0x10_0000;
pub const TIMER_HOLD: usize = 0x20_0000;
pub const TIMER_PAUSE: usize = 0x30_0000;
pub const TIMER_ID_MASK: usize = 0x0F_FFFF;
pub const HOST_CLASS: &str = "NoKeyHost";
/// Default profiles (Default, Gaming, Work, Discord) can be renamed, not deleted.
const BUILTIN_PROFILES: u32 = 4;

/// RegisterHotKey id of a binding's n-th shortcut (4 per binding).
pub fn hotkey_id(binding: u32, index: usize) -> i32 {
    (binding * 4 + index as u32) as i32
}

pub fn binding_of_hotkey(id: i32) -> u32 {
    id as u32 / 4
}

#[derive(Clone, Debug, PartialEq)]
pub enum Conn {
    NoToken,
    Checking,
    Connected(String),
    Rejected,
    Offline(String),
}

impl Conn {
    pub fn line(&self) -> String {
        match self {
            Conn::NoToken => t("discord.no_token"),
            Conn::Checking => t("discord.checking"),
            Conn::Connected(name) => tf("discord.connected_as", &[("name", name)]),
            Conn::Rejected => t("discord.invalid_token"),
            Conn::Offline(e) => tf("discord.offline", &[("details", e)]),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Hotkey,
    Repeat,
    Button,
}

pub enum TaskResult {
    /// `generation` identifies the check; results of superseded checks are ignored.
    Validated { generation: u64, new_token: Option<String>, result: Result<User, ApiError> },
    Ran { binding: u32, report: RunReport },
    Guilds(Result<Vec<Guild>, ApiError>),
    Channels(Result<Vec<Channel>, ApiError>),
    UserInfo(Result<User, ApiError>),
    UpdateChecked(Result<Option<ReleaseInfo>, String>),
    UpdateDownloaded(Result<update::Downloaded, String>),
}

/// A result to show as a tray notification and/or a sound.
struct Notice {
    ok: bool,
    text: String,
    sound: bool,
    notify: bool,
}

pub struct App {
    pub dir: PathBuf,
    pub config: Config,
    pub token: Option<String>,
    pub conn: Conn,
    conn_before_check: Conn,
    validation_generation: u64,
    cooldowns: Cooldowns,
    in_flight: HashSet<u32>,
    discord_until: Option<Instant>,
    pub hotkey_errors: HashMap<u32, String>,
    pub emergency_error: Option<String>,
    registered: Vec<i32>,
    trigger: TriggerEngine,
    /// Running Repeat/Toggle bindings: (runs so far, limit).
    repeats: HashMap<u32, (u32, u32)>,
    /// Temporarily paused bindings; `None` = until NoKey restarts.
    paused: HashMap<u32, Option<Instant>>,
    pub last_ping: String,
    pub last_result: Option<(bool, String)>,
    pub host: HWND,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Results from worker threads. The window message is only a wake-up call
/// and carries no pointer, so another process can't forge a payload.
static TASKS: Mutex<VecDeque<TaskResult>> = Mutex::new(VecDeque::new());

fn lock_tasks() -> std::sync::MutexGuard<'static, VecDeque<TaskResult>> {
    TASKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs `f` with the application state. Do not call `ui` functions inside.
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(a.borrow_mut().as_mut().expect("app not initialized")))
}

fn local_time() -> String {
    let mut t = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

// ---------- error texts ----------

pub fn api_error_text(e: &ApiError) -> String {
    match e {
        ApiError::InvalidToken => t("discord.invalid_token"),
        ApiError::NoAccess => t("discord.no_access"),
        ApiError::MissingPermissions => t("discord.no_permission"),
        ApiError::UnknownChannel => t("discord.unknown_channel"),
        ApiError::UnknownUser => t("discord.unknown_user"),
        ApiError::UnknownGuild => t("discord.unknown_guild"),
        ApiError::MembersIntent => t("discord.members_intent"),
        ApiError::RateLimited(s) => tf("discord.rate_limited", &[("secs", &format!("{s:.1}"))]),
        ApiError::Network(m) => tf("discord.network", &[("details", m)]),
        ApiError::Http(code, m) => tf("discord.http", &[("code", &code.to_string()), ("details", m)]),
    }
}

pub fn shell_error_text(f: &ShellFail) -> String {
    match f {
        ShellFail::FileNotFound => t("err.shell_file_not_found"),
        ShellFail::FolderNotFound => t("err.shell_folder_not_found"),
        ShellFail::AccessDenied => t("err.shell_access_denied"),
        ShellFail::InUse => t("err.shell_in_use"),
        ShellFail::NoAssociation => t("err.shell_no_association"),
        ShellFail::Other(code) => tf("err.shell_other", &[("code", &code.to_string())]),
    }
}

pub fn step_error_text(e: &StepError) -> String {
    match e {
        StepError::Invalid(key) => t(key),
        StepError::NoToken => t("discord.no_token"),
        StepError::Discord(e) => api_error_text(e),
        StepError::Shell(f) => shell_error_text(f),
    }
}

/// Short description of a binding's actions for lists.
pub fn actions_summary(b: &Binding) -> String {
    let Some(first) = b.actions.first() else { return t("main.no_actions") };
    // Left-to-right marks (U+200E, invisible) around the target ("@friend ·
    // #general", a URL) keep "@" and "#" in place inside Hebrew text. GDI
    // supports LRM but draws the newer isolate characters as boxes.
    let mut s = format!("{} · \u{200E}{}\u{200E}", t(first.kind().key()), first.target());
    if b.actions.len() > 1 {
        s.push_str(&tf("main.more_actions", &[("count", &(b.actions.len() - 1).to_string())]));
    }
    s
}

pub fn profile_name(config: &Config, id: u32) -> String {
    if id == ALL_PROFILES {
        return t("profile.all");
    }
    config.profile(id).map(|p| p.name.clone()).unwrap_or_else(|| t("profile.all"))
}

impl App {
    /// Runs `task` on a short-lived worker thread. Returns false if Windows
    /// could not create the thread.
    fn spawn(&self, task: impl FnOnce() -> TaskResult + Send + 'static) -> bool {
        let target = PostTarget::new(self.host);
        std::thread::Builder::new()
            .spawn(move || {
                let result = task();
                lock_tasks().push_back(result);
                target.post(WM_APP_TASK, 0);
            })
            .is_ok()
    }

    fn set_result(&mut self, ok: bool, text: String) {
        self.last_result = Some((ok, text));
    }

    fn notice(&mut self, ok: bool, text: String, sound: bool, notify: bool) -> Notice {
        self.set_result(ok, text.clone());
        Notice { ok, text, sound, notify }
    }

    pub fn is_paused(&mut self, id: u32) -> bool {
        match self.paused.get(&id) {
            Some(None) => true,
            Some(Some(until)) if *until > Instant::now() => true,
            Some(Some(_)) => {
                self.paused.remove(&id);
                false
            }
            None => false,
        }
    }

    pub fn paused_ids(&self) -> Vec<u32> {
        let now = Instant::now();
        self.paused.iter().filter(|(_, u)| u.is_none_or(|t| t > now)).map(|(id, _)| *id).collect()
    }

    fn stop_timers(&mut self, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::StopRepeat(id) => self.stop_repeat(id),
                Effect::CancelHold(id) => unsafe {
                    KillTimer(self.host, TIMER_HOLD + id as usize);
                },
                _ => {}
            }
        }
    }

    fn stop_repeat(&mut self, id: u32) {
        unsafe { KillTimer(self.host, TIMER_REPEAT + id as usize) };
        self.repeats.remove(&id);
        self.trigger.repeat_finished(id);
    }

    /// (Re)registers every shortcut of the active profile. Called after any change.
    pub fn apply_hotkeys(&mut self) {
        for id in self.registered.drain(..) {
            hotkey::unregister(self.host, id);
        }
        let stops = self.trigger.reset();
        self.stop_timers(stops);
        for id in self.repeats.keys().copied().collect::<Vec<_>>() {
            self.stop_repeat(id);
        }
        self.hotkey_errors.clear();
        self.emergency_error = None;
        self.trigger.double_tap_ms = self.config.double_tap_ms as u64;

        // The emergency switch is always registered, even when everything is off.
        let mut last_err = None;
        for (id, vk) in EMERGENCY_IDS.into_iter().zip([VK_PAUSE, VK_CANCEL]) {
            match hotkey::register(self.host, id, MOD_CTRL | MOD_ALT, vk) {
                Ok(()) => self.registered.push(id),
                Err(e) => last_err = Some(e),
            }
        }
        if !self.registered.iter().any(|id| EMERGENCY_IDS.contains(id)) {
            self.emergency_error = last_err.map(|e| format!("Ctrl + Alt + Pause {e}"));
        }

        let mut matcher = Matcher::new(self.config.combo_window_ms as u64, self.config.sequence_timeout_ms as u64);
        let mut hook_users = Vec::new();
        if self.config.bindings_enabled {
            let paused = self.paused_ids();
            let active: Vec<Binding> = self
                .config
                .bindings
                .iter()
                .filter(|b| b.enabled && self.config.in_active_profile(b) && !paused.contains(&b.id))
                .cloned()
                .collect();
            for b in &active {
                for (i, sc) in b.shortcuts.iter().enumerate() {
                    match sc {
                        Shortcut::Standard { mods, vk } if !b.trigger.mode.needs_key_up() => {
                            match hotkey::register(self.host, hotkey_id(b.id, i), *mods, *vk) {
                                Ok(()) => self.registered.push(hotkey_id(b.id, i)),
                                Err(e) => {
                                    self.hotkey_errors.insert(b.id, format!("{} {e}", sc.label()));
                                }
                            }
                        }
                        other => {
                            matcher.add(b.id, b.priority, other);
                            hook_users.push(b.id);
                        }
                    }
                }
            }
            if let Some(ps) = self.config.profile_switch.clone() {
                match ps {
                    Shortcut::Standard { mods, vk } => {
                        if hotkey::register(self.host, PROFILE_SWITCH_HOTKEY, mods, vk).is_ok() {
                            self.registered.push(PROFILE_SWITCH_HOTKEY);
                        }
                    }
                    other => matcher.add(PROFILE_SWITCH_ID, MAX_PRIORITY, &other),
                }
            }
        }
        if let Err(e) = hook::set_matcher(Some(matcher)) {
            for id in hook_users {
                self.hotkey_errors.insert(id, e.clone());
            }
        }
    }

    fn start(&mut self, id: u32, source: Source) -> Option<Notice> {
        let b = self.config.binding(id)?.clone();
        if source != Source::Button {
            let off = !self.config.bindings_enabled || !b.enabled || !self.config.in_active_profile(&b);
            if off || self.is_paused(id) {
                return None;
            }
            if !b.conditions.is_empty() && !b.conditions.allows(&context::foreground()) {
                if source == Source::Hotkey {
                    self.set_result(false, tf("status.skipped_conditions", &[("name", &b.name)]));
                }
                return None;
            }
        }
        if let Err((key, _)) = b.validate() {
            let text = tf("status.failed", &[("name", &b.name), ("details", &t(key))]);
            return Some(self.notice(false, text, b.sound, b.notify));
        }
        let length = Duration::from_millis(self.config.cooldown_for(&b) as u64);
        if let Err(left) = self.cooldowns.try_start(id, length, Instant::now()) {
            if source != Source::Repeat {
                let secs = timing::format_seconds(left.as_millis() as u32, i18n::language().decimal_comma());
                self.set_result(false, tf("status.cooldown", &[("name", &b.name), ("secs", &secs)]));
            }
            return None; // a blocked double-press is not worth a notification
        }
        if self.in_flight.contains(&id) {
            if source != Source::Repeat {
                self.set_result(false, tf("status.busy", &[("name", &b.name)]));
            }
            return None;
        }
        if b.has_discord()
            && let Some(until) = self.discord_until.filter(|u| *u > Instant::now())
        {
            let text = api_error_text(&ApiError::RateLimited((until - Instant::now()).as_secs_f64()));
            return Some(self.notice(false, text, b.sound, b.notify));
        }
        self.in_flight.insert(id);
        self.set_result(true, tf("status.running", &[("name", &b.name)]));
        let token = if b.has_discord() { self.token.clone() } else { None };
        let (steps, delay, stop) = (b.actions.clone(), b.step_delay_ms, b.stop_on_failure);
        let started = self.spawn(move || TaskResult::Ran {
            binding: id,
            report: actions::run(&steps, delay, stop, token.as_deref()),
        });
        if !started {
            self.in_flight.remove(&id);
            return Some(self.notice(false, t("err.thread"), b.sound, b.notify));
        }
        None
    }

    fn on_ran(&mut self, id: u32, report: RunReport) -> Option<Notice> {
        self.in_flight.remove(&id);
        let b = self.config.binding(id)?.clone();
        if let Some(w) = report.discord_wait {
            let until = Instant::now() + wait_duration(w);
            self.discord_until = Some(until);
            self.cooldowns.extend_to(id, until);
        }
        for o in &report.outcomes {
            match (&o.result, o.kind) {
                (Err(StepError::Discord(ApiError::InvalidToken)), _) => self.conn = Conn::Rejected,
                (Ok(()), ActionKind::DiscordPing) => {
                    self.last_ping = format!("{} · {}", local_time(), o.target);
                    if !matches!(self.conn, Conn::Connected(_)) {
                        self.conn = Conn::Connected("bot".into());
                    }
                }
                _ => {}
            }
        }
        let ok = report.ok();
        let repeating = self.repeats.contains_key(&id);
        let text = if ok {
            match report.outcomes.as_slice() {
                [only] if only.kind == ActionKind::DiscordPing => tf("status.mention_sent", &[("target", &only.target)]),
                _ => tf("status.done", &[("name", &b.name)]),
            }
        } else {
            let detail =
                report.first_error().and_then(|o| o.result.as_ref().err()).map(step_error_text).unwrap_or_default();
            let mut text = tf("status.failed", &[("name", &b.name), ("details", &detail)]);
            if report.skipped > 0 {
                text.push(' ');
                text.push_str(&tf("status.skipped_steps", &[("count", &report.skipped.to_string())]));
            }
            text
        };
        if !ok && repeating {
            self.stop_repeat(id); // don't keep failing on a timer
        }
        // While a binding repeats, only failures are announced.
        let announce = !repeating || !ok;
        Some(self.notice(ok, text, b.sound && announce, b.notify && announce))
    }

    fn on_validated(&mut self, new_token: Option<String>, result: Result<User, ApiError>) -> Option<Notice> {
        match (new_token, result) {
            (Some(token), Ok(user)) => match dpapi::save_token(&self.dir, &token) {
                Ok(()) => {
                    self.token = Some(token);
                    self.conn = Conn::Connected(user.display_name().to_string());
                    let text = tf("discord.connected_as", &[("name", user.display_name())]);
                    Some(self.notice(true, text, false, false))
                }
                Err(e) => {
                    self.conn = self.conn_before_check.clone();
                    Some(self.notice(false, e, false, false))
                }
            },
            (Some(_), Err(e)) => {
                self.conn = self.conn_before_check.clone();
                let text = tf("discord.token_not_saved", &[("details", &api_error_text(&e))]);
                Some(self.notice(false, text, false, false))
            }
            (None, Ok(user)) => {
                self.conn = Conn::Connected(user.display_name().to_string());
                None
            }
            (None, Err(ApiError::InvalidToken)) => {
                self.conn = Conn::Rejected;
                Some(self.notice(false, t("discord.invalid_token"), false, true))
            }
            (None, Err(e)) => {
                self.conn = Conn::Offline(api_error_text(&e));
                None
            }
        }
    }

    fn validate(&mut self, new_token: Option<String>) {
        let Some(token) = new_token.clone().or_else(|| self.token.clone()) else { return };
        if self.conn != Conn::Checking {
            self.conn_before_check = self.conn.clone();
        }
        self.conn = Conn::Checking;
        self.validation_generation += 1;
        let generation = self.validation_generation;
        let started = self.spawn(move || TaskResult::Validated {
            generation,
            new_token,
            result: Client::new(&token).current_user(),
        });
        if !started {
            self.conn = self.conn_before_check.clone();
        }
    }

    /// Applies a config change atomically: validate, save, then switch.
    fn commit(&mut self, mut next: Config) -> Result<(), String> {
        next.sanitize();
        config::save(&self.dir, &next)?;
        self.config = next;
        self.apply_hotkeys();
        Ok(())
    }
}

// ---------- entry points used by windows ----------

fn show_notice(notice: Option<Notice>) {
    let Some(n) = notice else { return };
    if n.sound {
        unsafe { MessageBeep(if n.ok { MB_OK } else { MB_ICONHAND }) };
    }
    let (host, wanted) = with(|a| (a.host, a.config.notifications));
    if n.notify && wanted && !ui::main_window::is_foreground() {
        let title = if n.ok { "NoKey".to_string() } else { t("notify.failed_title") };
        tray::notify(host, &title, &n.text, !n.ok);
    }
}

fn apply_effects(effects: Vec<Effect>) {
    for e in effects {
        match e {
            Effect::Run(id) => {
                let n = with(|a| a.start(id, Source::Hotkey));
                show_notice(n);
                ui::main_window::after_run(id);
            }
            Effect::StartRepeat(id) => with(|a| {
                let Some(b) = a.config.binding(id) else { return };
                let (max, interval) = (b.trigger.repeat_max, b.trigger.repeat_interval_ms);
                if max <= 1 {
                    a.trigger.repeat_finished(id);
                    return;
                }
                a.repeats.insert(id, (1, max));
                unsafe { SetTimer(a.host, TIMER_REPEAT + id as usize, interval, None) };
            }),
            Effect::StopRepeat(id) => with(|a| a.stop_repeat(id)),
            Effect::StartHold(id) => with(|a| unsafe {
                SetTimer(a.host, TIMER_HOLD + id as usize, a.config.long_press_ms, None);
            }),
            Effect::CancelHold(id) => with(|a| unsafe {
                KillTimer(a.host, TIMER_HOLD + id as usize);
            }),
        }
    }
}

/// A shortcut went down (`Some(true)`), up (`Some(false)`) or was pressed
/// as a whole (`None`, from RegisterHotKey).
pub fn on_key(id: u32, down: Option<bool>) {
    if id == PROFILE_SWITCH_ID {
        if down != Some(false) {
            next_profile();
        }
        return;
    }
    let effects = with(|a| {
        let mode = a.config.binding(id)?.trigger.mode;
        Some(match down {
            None => a.trigger.tap(id, mode, tick()),
            Some(true) => a.trigger.down(id, mode, tick()),
            Some(false) => a.trigger.up(id, mode),
        })
    });
    apply_effects(effects.unwrap_or_default());
}

pub fn on_hotkey(id: i32) {
    if EMERGENCY_IDS.contains(&id) {
        toggle_all();
    } else if id == PROFILE_SWITCH_HOTKEY {
        next_profile();
    } else {
        on_key(binding_of_hotkey(id), None);
    }
}

pub fn on_timer(timer: usize) {
    let id = (timer & TIMER_ID_MASK) as u32;
    let kind = timer & !TIMER_ID_MASK;
    match kind {
        TIMER_REPEAT => {
            let notice = with(|a| {
                let (count, max) = *a.repeats.get(&id)?;
                if count >= max {
                    a.stop_repeat(id);
                    return None;
                }
                a.repeats.insert(id, (count + 1, max));
                let n = a.start(id, Source::Repeat);
                if count + 1 >= max {
                    a.stop_repeat(id);
                }
                n
            });
            show_notice(notice);
            ui::main_window::refresh();
        }
        TIMER_HOLD => {
            let effects = with(|a| {
                unsafe { KillTimer(a.host, timer) };
                a.trigger.hold_elapsed(id)
            });
            apply_effects(effects);
        }
        TIMER_PAUSE => {
            with(|a| {
                unsafe { KillTimer(a.host, timer) };
                a.paused.remove(&id);
                a.apply_hotkeys();
            });
            ui::main_window::refresh();
        }
        _ => {}
    }
}

/// Runs a binding from the window (big button or tray). Profile, pause and
/// conditions don't apply to a direct click.
pub fn run_binding(id: u32) {
    let notice = with(|a| a.start(id, Source::Button));
    show_notice(notice);
    ui::main_window::after_run(id);
}

/// "Disable all binds" / "Enable all binds" (also Ctrl+Alt+Pause).
pub fn toggle_all() {
    let notice = with(|a| {
        let mut next = a.config.clone();
        next.bindings_enabled = !next.bindings_enabled;
        let enabled = next.bindings_enabled;
        if let Err(e) = a.commit(next) {
            return a.notice(false, e, false, true);
        }
        let key = if enabled { "status.all_enabled" } else { "status.all_disabled" };
        a.notice(true, t(key), false, true)
    });
    show_notice(Some(notice));
    ui::main_window::refresh();
    tray_tip();
}

pub fn tray_tip() {
    let (host, enabled, profile) =
        with(|a| (a.host, a.config.bindings_enabled, profile_name(&a.config, a.config.active_profile)));
    let state = if enabled { profile } else { t("main.status_disabled_short") };
    tray::set_tip(host, &format!("NoKey — {state}"));
}

pub fn drain_tasks() {
    while let Some(task) = lock_tasks().pop_front() {
        on_task(task);
    }
}

fn on_task(result: TaskResult) {
    match result {
        TaskResult::Ran { binding, report } => {
            let ok = report.ok();
            let notice = with(|a| a.on_ran(binding, report));
            show_notice(notice);
            ui::main_window::flash(binding, ok);
        }
        TaskResult::Validated { generation, new_token, result } => {
            let notice = with(|a| {
                // A newer check, Connect or Disconnect happened since: this result is stale.
                if generation != a.validation_generation {
                    return None;
                }
                a.on_validated(new_token, result)
            });
            show_notice(notice);
            ui::main_window::refresh();
            ui::settings::refresh();
        }
        TaskResult::Guilds(r) => ui::editor::on_guilds(r),
        TaskResult::Channels(r) => ui::editor::on_channels(r),
        TaskResult::UserInfo(r) => ui::editor::on_user(r),
        TaskResult::UpdateChecked(r) => ui::settings::on_update_checked(r),
        TaskResult::UpdateDownloaded(r) => ui::settings::on_update_downloaded(r),
    }
}

pub fn connect(token: String) {
    with(|a| a.validate(Some(token)));
    ui::settings::refresh();
    ui::main_window::refresh();
}

pub fn disconnect() -> Result<(), String> {
    with(|a| {
        dpapi::delete_token(&a.dir)?;
        a.token = None;
        a.conn = Conn::NoToken;
        a.validation_generation += 1; // ignore a check that is still running
        Ok::<(), String>(())
    })?;
    ui::settings::refresh();
    ui::main_window::refresh();
    Ok(())
}

/// Runs a Discord lookup for the binding editor on a worker thread.
pub fn discord_lookup(task: impl FnOnce(&Client) -> TaskResult + Send + 'static) -> Result<(), String> {
    with(|a| {
        let token = a.token.clone().ok_or_else(|| t("discord.no_token"))?;
        if a.spawn(move || task(&Client::new(&token))) { Ok(()) } else { Err(t("err.thread")) }
    })
}

pub fn spawn_task(task: impl FnOnce() -> TaskResult + Send + 'static) {
    with(|a| a.spawn(task));
}

fn after_change() {
    ui::main_window::refresh();
    tray_tip();
}

/// Adds or replaces a binding. Returns the hotkey registration problem, if any.
pub fn save_binding(b: Binding) -> Result<Option<String>, String> {
    let result = with(|a| {
        let mut next = a.config.clone();
        match next.bindings.iter_mut().find(|x| x.id == b.id) {
            Some(slot) => *slot = b.clone(),
            None => next.bindings.push(b.clone()),
        }
        a.commit(next)?;
        Ok(a.hotkey_errors.get(&b.id).cloned())
    });
    after_change();
    result
}

pub fn delete_binding(id: u32) -> Result<(), String> {
    let result = with(|a| {
        let mut next = a.config.clone();
        next.bindings.retain(|b| b.id != id);
        a.paused.remove(&id);
        a.commit(next)
    });
    after_change();
    result
}

/// Copies a binding (without its shortcuts, which would conflict). Returns the new id.
pub fn duplicate_binding(id: u32) -> Result<u32, String> {
    let result = with(|a| {
        let mut next = a.config.clone();
        let src = next.binding(id).cloned().ok_or("binding not found")?;
        let new_id = next.next_id();
        if new_id >= MAX_BINDING_ID {
            return Err(t("err.too_many_bindings"));
        }
        let copy =
            Binding { id: new_id, name: tf("main.copy_name", &[("name", &src.name)]), shortcuts: Vec::new(), ..src };
        next.bindings.push(copy);
        a.commit(next)?;
        Ok(new_id)
    });
    after_change();
    result
}

pub fn set_binding_enabled(id: u32, enabled: bool) -> Result<Option<String>, String> {
    let result = with(|a| {
        let mut next = a.config.clone();
        if let Some(b) = next.bindings.iter_mut().find(|b| b.id == id) {
            b.enabled = enabled;
        }
        a.commit(next)?;
        Ok(a.hotkey_errors.get(&id).cloned())
    });
    after_change();
    result
}

/// Pauses one binding for a while (`None` = until NoKey restarts).
pub fn pause_binding(id: u32, length: Option<Duration>) {
    with(|a| {
        a.paused.insert(id, length.map(|l| Instant::now() + l));
        unsafe { KillTimer(a.host, TIMER_PAUSE + id as usize) };
        if let Some(l) = length {
            let ms = l.as_millis().min(u32::MAX as u128) as u32;
            unsafe { SetTimer(a.host, TIMER_PAUSE + id as usize, ms, None) };
        }
        a.apply_hotkeys();
    });
    ui::main_window::refresh();
}

pub fn resume_binding(id: u32) {
    with(|a| {
        a.paused.remove(&id);
        unsafe { KillTimer(a.host, TIMER_PAUSE + id as usize) };
        a.apply_hotkeys();
    });
    ui::main_window::refresh();
}

// ---------- profiles ----------

pub fn set_active_profile(id: u32) {
    let notice = with(|a| {
        let mut next = a.config.clone();
        next.active_profile = id;
        if let Err(e) = a.commit(next) {
            return a.notice(false, e, false, true);
        }
        let name = profile_name(&a.config, id);
        a.notice(true, tf("status.profile_switched", &[("name", &name)]), false, true)
    });
    show_notice(Some(notice));
    after_change();
    ui::settings::refresh();
}

pub fn next_profile() {
    let next = with(|a| {
        let ids: Vec<u32> = a.config.profiles.iter().map(|p| p.id).collect();
        let pos = ids.iter().position(|&id| id == a.config.active_profile).unwrap_or(0);
        ids[(pos + 1) % ids.len()]
    });
    set_active_profile(next);
}

pub fn is_builtin_profile(id: u32) -> bool {
    (1..=BUILTIN_PROFILES).contains(&id)
}

fn edit_profiles(f: impl FnOnce(&mut Config) -> Result<u32, String>) -> Result<u32, String> {
    let result = with(|a| {
        let mut next = a.config.clone();
        let id = f(&mut next)?;
        a.commit(next)?;
        Ok(id)
    });
    after_change();
    ui::settings::refresh();
    result
}

pub fn add_profile(name: &str) -> Result<u32, String> {
    let name = name.trim().to_string();
    edit_profiles(|c| {
        if name.is_empty() {
            return Err(t("err.profile_name"));
        }
        let id = c.next_profile_id();
        c.profiles.push(Profile { id, name });
        Ok(id)
    })
}

pub fn rename_profile(id: u32, name: &str) -> Result<u32, String> {
    let name = name.trim().to_string();
    edit_profiles(|c| {
        if name.is_empty() {
            return Err(t("err.profile_name"));
        }
        let p = c.profiles.iter_mut().find(|p| p.id == id).ok_or("profile not found")?;
        p.name = name;
        Ok(id)
    })
}

/// Copies a profile and all of its bindings. Returns the new profile id.
pub fn duplicate_profile(id: u32) -> Result<u32, String> {
    edit_profiles(|c| {
        let src = c.profile(id).cloned().ok_or("profile not found")?;
        let new_id = c.next_profile_id();
        c.profiles.push(Profile { id: new_id, name: tf("main.copy_name", &[("name", &src.name)]) });
        let copies: Vec<Binding> = c.bindings.iter().filter(|b| b.profile == id).cloned().collect();
        for b in copies {
            let bid = c.next_id();
            if bid >= MAX_BINDING_ID {
                return Err(t("err.too_many_bindings"));
            }
            c.bindings.push(Binding { id: bid, profile: new_id, ..b });
        }
        Ok(new_id)
    })
}

/// Deletes a custom profile; its bindings move to the Default profile.
pub fn delete_profile(id: u32) -> Result<u32, String> {
    edit_profiles(|c| {
        if is_builtin_profile(id) {
            return Err(t("err.profile_builtin"));
        }
        c.profiles.retain(|p| p.id != id);
        for b in c.bindings.iter_mut().filter(|b| b.profile == id) {
            b.profile = DEFAULT_PROFILE;
        }
        if c.active_profile == id {
            c.active_profile = DEFAULT_PROFILE;
        }
        Ok(DEFAULT_PROFILE)
    })
}

pub fn set_profile_switch(shortcut: Option<Shortcut>) -> Result<u32, String> {
    edit_profiles(|c| {
        if let Some(sc) = &shortcut {
            config::check_shortcut(sc)?;
        }
        c.profile_switch = shortcut;
        Ok(0)
    })
}

// ---------- settings ----------

pub fn save_settings(next: Config) -> Result<(), String> {
    let autostart = next.start_with_windows;
    with(|a| a.commit(next))?;
    startup::set_enabled(autostart)?;
    after_change();
    Ok(())
}

pub fn set_theme(theme: Theme) -> Result<(), String> {
    with(|a| {
        let mut next = a.config.clone();
        next.theme = theme;
        a.commit(next)
    })?;
    let had_star = ui::theme::palette().star.is_some();
    ui::theme::set_theme(theme);
    if ui::theme::palette().star.is_some() != had_star {
        // The main window header changes height: lay the windows out again.
        ui::rebuild_all();
    } else {
        ui::repaint_all();
    }
    Ok(())
}

/// Re-reads the Windows light/dark setting (WM_SETTINGCHANGE).
pub fn system_theme_changed() {
    let theme = with(|a| a.config.theme);
    if theme == Theme::System && ui::theme::set_theme(theme) {
        ui::repaint_all();
    }
}

pub fn set_language(language: Language) -> Result<(), String> {
    with(|a| {
        let mut next = a.config.clone();
        next.language = language;
        a.commit(next)
    })?;
    i18n::set_language(language);
    ui::rebuild_all();
    tray_tip();
    Ok(())
}

pub fn host_hwnd() -> HWND {
    with(|a| a.host)
}

fn release_resources() -> HWND {
    let host = with(|a| {
        for id in a.registered.drain(..) {
            hotkey::unregister(a.host, id);
        }
        let stops = a.trigger.reset();
        a.stop_timers(stops);
        a.host
    });
    hook::shutdown();
    tray::remove(host);
    host
}

/// Full shutdown: hotkeys, hook and tray icon are released before the windows go.
pub fn exit() {
    ui::editor::close();
    ui::settings::close();
    ui::main_window::destroy();
    let host = release_resources();
    unsafe { DestroyWindow(host) };
}

pub fn install_update_and_exit(downloaded: &update::Downloaded) -> Result<(), String> {
    let dir = with(|a| a.dir.clone());
    update::apply_and_restart(downloaded, &dir)?;
    exit();
    Ok(())
}

fn instance_mutex_name(dir: &Path) -> String {
    if std::env::var_os("NOKEY_DATA_DIR").is_none() {
        return "Local\\NoKey.Instance".into();
    }
    // A separate data dir (portable copy or tests) gets its own instance.
    let hash = dir.to_string_lossy().bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("Local\\NoKey.Instance.{hash:016x}")
}

/// Ensures one running copy. A second launch brings the first to front.
fn single_instance(dir: &Path, after_update: bool) -> Option<HANDLE> {
    let name = win::wide(&instance_mutex_name(dir));
    let handle = unsafe { CreateMutexW(null(), 1, name.as_ptr()) };
    if handle.is_null() {
        return None;
    }
    if win::last_error() != ERROR_ALREADY_EXISTS {
        return Some(handle);
    }
    if after_update {
        // The old version is shutting down; wait for it to release the mutex.
        let r = unsafe { WaitForSingleObject(handle, 15_000) };
        if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
            return Some(handle);
        }
    } else {
        let class = win::wide(HOST_CLASS);
        let existing = unsafe { FindWindowW(class.as_ptr(), null()) };
        if !existing.is_null() {
            PostTarget::new(existing).post(WM_APP_SHOW, 0);
        }
    }
    unsafe { CloseHandle(handle) };
    None
}

/// Program entry: returns the process exit code.
pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let start_hidden_arg = has("--minimized") || has(startup::AUTOSTART_ARG);

    unsafe {
        CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES | ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&icc);
    }
    let dir = config::data_dir();
    let Some(_instance) = single_instance(&dir, has("--updated")) else { return 0 };
    update::cleanup_old();

    let loaded = config::load(&dir, i18n::system_language());
    let mut warnings = loaded.warnings;
    let config = loaded.config;
    i18n::set_language(config.language);
    ui::theme::set_theme(config.theme);
    let token = dpapi::load_token(&dir).unwrap_or_else(|e| {
        warnings.push(e);
        None
    });
    let conn = if token.is_some() { Conn::Checking } else { Conn::NoToken };
    let (launch_minimized, to_tray, autostart) = (config.launch_minimized, config.minimize_to_tray, config.start_with_windows);
    let double_tap = config.double_tap_ms as u64;

    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            dir,
            config,
            token,
            conn: conn.clone(),
            conn_before_check: Conn::NoToken,
            validation_generation: 0,
            cooldowns: Cooldowns::default(),
            in_flight: HashSet::new(),
            discord_until: None,
            hotkey_errors: HashMap::new(),
            emergency_error: None,
            registered: Vec::new(),
            trigger: TriggerEngine::new(double_tap),
            repeats: HashMap::new(),
            paused: HashMap::new(),
            last_ping: String::new(),
            last_result: None,
            host: null_mut(),
        })
    });

    let big_icon = win::icon::create(unsafe { GetSystemMetrics(SM_CXICON) } as u32);
    let small_icon = win::icon::create(unsafe { GetSystemMetrics(SM_CXSMICON) } as u32);
    ui::host::register();
    ui::main_window::register(big_icon);
    ui::editor::register(big_icon);
    ui::settings::register(big_icon);
    let host = ui::host::create(small_icon);
    if host.is_null() {
        win::error_box(null_mut(), "NoKey could not create its window.");
        return 1;
    }
    with(|a| a.host = host);
    hook::set_target(host);
    if !tray::add(host, small_icon, "NoKey") {
        warnings.push(t("err.tray"));
    }
    with(|a| {
        a.apply_hotkeys();
        if conn == Conn::Checking {
            a.validate(None);
        }
    });
    tray_tip();
    if autostart && let Err(e) = startup::set_enabled(true) {
        warnings.push(e);
    }
    ui::main_window::create();
    if start_hidden_arg || launch_minimized {
        if !to_tray {
            ui::main_window::show_minimized();
        }
    } else {
        ui::main_window::show_main();
    }
    if !warnings.is_empty() {
        ui::alert(ui::main_window::hwnd(), &warnings.join("\n\n"));
    }

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        if ui::editor::dialog_message(&msg) || ui::settings::dialog_message(&msg) {
            continue;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    0
}
