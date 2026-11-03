//! Configuration data model (schema version 2).

use crate::discord::mentions::is_snowflake;
use crate::shortcut::Shortcut;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;
/// Where official NoKey releases are published.
pub const DEFAULT_UPDATE_REPO: &str = "Maestoo-of-69/NoKey";
/// Each binding owns 4 hotkey ids (one per shortcut); ids from 0xB000 up are reserved.
pub const MAX_BINDING_ID: u32 = 0x2C00;
pub const MAX_SHORTCUTS: usize = 3;
pub const MAX_ACTIONS: usize = 10;
/// Profile id 0 on a binding means "active in every profile".
pub const ALL_PROFILES: u32 = 0;
pub const DEFAULT_PROFILE: u32 = 1;

// ---------- actions ----------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    DiscordPing {
        guild_id: String,
        #[serde(default)]
        guild_name: String,
        channel_id: String,
        #[serde(default)]
        channel_name: String,
        user_id: String,
        #[serde(default)]
        user_name: String,
    },
    OpenApp {
        path: String,
        #[serde(default)]
        args: String,
    },
    OpenFile {
        path: String,
    },
    OpenFolder {
        path: String,
    },
    /// Opens the URL in the default Windows browser.
    OpenUrl {
        url: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    DiscordPing,
    OpenApp,
    OpenFile,
    OpenFolder,
    OpenUrl,
}

impl ActionKind {
    pub const ALL: [ActionKind; 5] =
        [ActionKind::DiscordPing, ActionKind::OpenApp, ActionKind::OpenFile, ActionKind::OpenFolder, ActionKind::OpenUrl];

    /// Localization key of the action name.
    pub fn key(self) -> &'static str {
        match self {
            ActionKind::DiscordPing => "action.discord_ping",
            ActionKind::OpenApp => "action.open_app",
            ActionKind::OpenFile => "action.open_file",
            ActionKind::OpenFolder => "action.open_folder",
            ActionKind::OpenUrl => "action.open_url",
        }
    }

    pub fn empty(self) -> Action {
        match self {
            ActionKind::DiscordPing => Action::DiscordPing {
                guild_id: String::new(),
                guild_name: String::new(),
                channel_id: String::new(),
                channel_name: String::new(),
                user_id: String::new(),
                user_name: String::new(),
            },
            ActionKind::OpenApp => Action::OpenApp { path: String::new(), args: String::new() },
            ActionKind::OpenFile => Action::OpenFile { path: String::new() },
            ActionKind::OpenFolder => Action::OpenFolder { path: String::new() },
            ActionKind::OpenUrl => Action::OpenUrl { url: String::new() },
        }
    }
}

impl Action {
    pub fn kind(&self) -> ActionKind {
        match self {
            Action::DiscordPing { .. } => ActionKind::DiscordPing,
            Action::OpenApp { .. } => ActionKind::OpenApp,
            Action::OpenFile { .. } => ActionKind::OpenFile,
            Action::OpenFolder { .. } => ActionKind::OpenFolder,
            Action::OpenUrl { .. } => ActionKind::OpenUrl,
        }
    }

    /// Short target text, e.g. "@friend · #general" or the URL.
    pub fn target(&self) -> String {
        match self {
            Action::DiscordPing { user_name, user_id, channel_name, .. } => {
                let who = if user_name.is_empty() { user_id.clone() } else { format!("@{user_name}") };
                if channel_name.is_empty() { who } else { format!("{who} · #{channel_name}") }
            }
            Action::OpenApp { path, .. } | Action::OpenFile { path } | Action::OpenFolder { path } => file_name(path),
            Action::OpenUrl { url } => url.clone(),
        }
    }

    /// Checks the fields that can be checked offline. Errors are
    /// localization keys (see `i18n`).
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Action::DiscordPing { guild_id, channel_id, user_id, .. } => {
                if !guild_id.is_empty() && !is_snowflake(guild_id) {
                    return Err("err.guild_id");
                }
                if !is_snowflake(channel_id) {
                    return Err("err.channel_id");
                }
                if !is_snowflake(user_id) {
                    return Err("err.user_id");
                }
                Ok(())
            }
            Action::OpenApp { path, .. } => {
                let lower = normalized_path(path)?;
                if lower.is_empty() {
                    return Err("err.choose_app");
                }
                if !(lower.ends_with(".exe") || lower.ends_with(".lnk")) {
                    return Err("err.app_type");
                }
                if BLOCKED_PROGRAMS.contains(&file_name(&lower).as_str()) {
                    return Err("err.app_blocked");
                }
                Ok(())
            }
            Action::OpenFile { path } => {
                let lower = normalized_path(path)?;
                if lower.is_empty() {
                    return Err("err.choose_file");
                }
                if BLOCKED_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
                    return Err("err.file_blocked");
                }
                Ok(())
            }
            Action::OpenFolder { path } => {
                if normalized_path(path)?.is_empty() { Err("err.choose_folder") } else { Ok(()) }
            }
            Action::OpenUrl { url } => {
                if is_web_url(url) { Ok(()) } else { Err("err.url") }
            }
        }
    }
}

/// `http://` or `https://`, a host, and no spaces, quotes or control characters.
pub fn is_web_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"));
    match rest {
        Some(r) => {
            let host = r.split(['/', '?', '#']).next().unwrap_or("");
            !host.is_empty() && !r.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"')
        }
        None => false,
    }
}

/// Extensions that would run code (or follow a link to code) if "opened".
/// Keeps Open File from becoming a way to execute scripts.
const BLOCKED_EXTENSIONS: [&str; 34] = [
    ".exe", ".com", ".bat", ".cmd", ".ps1", ".psm1", ".vbs", ".vbe", ".vb", ".js", ".jse", ".wsf", ".wsh", ".ws",
    ".msi", ".msp", ".scr", ".pif", ".hta", ".cpl", ".lnk", ".url", ".py", ".pyw", ".jar", ".reg", ".msc",
    ".appref-ms", ".application", ".scf", ".sct", ".chm", ".inf", ".gadget",
];

/// Programs that run arbitrary commands or scripts from their arguments.
const BLOCKED_PROGRAMS: [&str; 22] = [
    "cmd.exe", "powershell.exe", "powershell_ise.exe", "pwsh.exe", "wscript.exe", "cscript.exe", "mshta.exe",
    "rundll32.exe", "regsvr32.exe", "msiexec.exe", "bash.exe", "wsl.exe", "conhost.exe", "forfiles.exe",
    "python.exe", "pythonw.exe", "py.exe", "pyw.exe", "node.exe", "java.exe", "javaw.exe", "msbuild.exe",
];

/// Lower-cases a path and removes what Windows ignores at the end (dots and
/// spaces), so "run.bat." can't slip past an extension check. Rejects NTFS
/// alternate data streams ("file.txt:hidden").
fn normalized_path(path: &str) -> Result<String, &'static str> {
    let p = path.trim().trim_end_matches(['.', ' ']).to_ascii_lowercase();
    let after_drive = p.get(2..).unwrap_or("");
    if after_drive.contains(':') || p.contains(char::is_control) {
        return Err("err.path_chars");
    }
    Ok(p)
}

pub fn file_name(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['\\', '/']);
    trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed).to_string()
}

// ---------- triggers ----------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    /// Runs when the shortcut goes down.
    #[default]
    Press,
    /// Runs when the shortcut is released.
    Release,
    /// Runs on the second press within the double-tap window.
    DoubleTap,
    /// Runs once the shortcut has been held for the long-press time.
    LongPress,
    /// Runs repeatedly while the shortcut is held.
    Repeat,
    /// First press starts repeating, the next press stops it.
    Toggle,
}

impl TriggerMode {
    pub const ALL: [TriggerMode; 6] = [
        TriggerMode::Press,
        TriggerMode::Release,
        TriggerMode::DoubleTap,
        TriggerMode::LongPress,
        TriggerMode::Repeat,
        TriggerMode::Toggle,
    ];

    pub fn key(self) -> &'static str {
        match self {
            TriggerMode::Press => "trigger.press",
            TriggerMode::Release => "trigger.release",
            TriggerMode::DoubleTap => "trigger.double_tap",
            TriggerMode::LongPress => "trigger.long_press",
            TriggerMode::Repeat => "trigger.repeat",
            TriggerMode::Toggle => "trigger.toggle",
        }
    }

    pub fn repeats(self) -> bool {
        matches!(self, TriggerMode::Repeat | TriggerMode::Toggle)
    }

    /// Modes that need key-up events, which only the keyboard hook sees.
    pub fn needs_key_up(self) -> bool {
        matches!(self, TriggerMode::Release | TriggerMode::LongPress | TriggerMode::Repeat)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Trigger {
    pub mode: TriggerMode,
    pub repeat_interval_ms: u32,
    /// Hard limit on runs per Repeat/Toggle activation.
    pub repeat_max: u32,
}

impl Default for Trigger {
    fn default() -> Self {
        Self { mode: TriggerMode::Press, repeat_interval_ms: 1000, repeat_max: 10 }
    }
}

pub const REPEAT_INTERVAL_RANGE: (u32, u32) = (100, 60_000);
pub const REPEAT_MAX_RANGE: (u32, u32) = (1, 1000);

// ---------- conditions ----------

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conditions {
    /// Run only when this program is in the foreground, e.g. "chrome.exe".
    pub process: String,
    /// Run only when the foreground window title contains this text.
    pub window_title: String,
    /// Do not run while a fullscreen app (game, video) is in front.
    pub skip_fullscreen: bool,
}

/// Foreground window facts used to check conditions.
#[derive(Clone, Debug, Default)]
pub struct Foreground {
    pub process: String,
    pub title: String,
    pub fullscreen: bool,
}

impl Conditions {
    pub fn is_empty(&self) -> bool {
        self.process.trim().is_empty() && self.window_title.trim().is_empty() && !self.skip_fullscreen
    }

    /// Case-insensitive; a process name matches with or without ".exe".
    pub fn allows(&self, fg: &Foreground) -> bool {
        let want = self.process.trim().to_lowercase();
        if !want.is_empty() {
            let have = fg.process.to_lowercase();
            let want_exe = if want.ends_with(".exe") { want.clone() } else { format!("{want}.exe") };
            if have != want && have != want_exe {
                return false;
            }
        }
        let title = self.window_title.trim().to_lowercase();
        if !title.is_empty() && !fg.title.to_lowercase().contains(&title) {
            return false;
        }
        !(self.skip_fullscreen && fg.fullscreen)
    }
}

// ---------- bindings, profiles, settings ----------

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// One emoji or character shown in the list.
    #[serde(default)]
    pub icon: String,
    pub enabled: bool,
    /// Profile id, or `ALL_PROFILES`.
    #[serde(default)]
    pub profile: u32,
    #[serde(default)]
    pub shortcuts: Vec<Shortcut>,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Pause between actions of a multi-action binding.
    #[serde(default)]
    pub step_delay_ms: u32,
    #[serde(default = "yes")]
    pub stop_on_failure: bool,
    /// `None` = use the global cooldown.
    #[serde(default)]
    pub cooldown_ms: Option<u32>,
    /// Higher wins when one key event matches several bindings (0–9).
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub sound: bool,
    #[serde(default = "yes")]
    pub notify: bool,
    #[serde(default)]
    pub conditions: Conditions,
}

pub const STEP_DELAY_RANGE: (u32, u32) = (0, 60_000);
pub const MAX_PRIORITY: u8 = 9;

impl Binding {
    pub fn new(id: u32, profile: u32) -> Self {
        Self {
            id,
            name: String::new(),
            description: String::new(),
            icon: String::new(),
            enabled: true,
            profile,
            shortcuts: Vec::new(),
            trigger: Trigger::default(),
            actions: Vec::new(),
            step_delay_ms: 0,
            stop_on_failure: true,
            cooldown_ms: None,
            priority: 0,
            sound: false,
            notify: true,
            conditions: Conditions::default(),
        }
    }

    pub fn has_discord(&self) -> bool {
        self.actions.iter().any(|a| a.kind() == ActionKind::DiscordPing)
    }

    /// Checks everything that can be checked offline. Returns
    /// (localization key, step index) on error.
    pub fn validate(&self) -> Result<(), (&'static str, Option<usize>)> {
        if self.name.trim().is_empty() {
            return Err(("err.name", None));
        }
        if self.actions.is_empty() {
            return Err(("err.no_actions", None));
        }
        if self.actions.len() > MAX_ACTIONS {
            return Err(("err.too_many_actions", None));
        }
        if self.shortcuts.len() > MAX_SHORTCUTS {
            return Err(("err.too_many_shortcuts", None));
        }
        for (i, a) in self.actions.iter().enumerate() {
            a.validate().map_err(|k| (k, Some(i)))?;
        }
        // The original spec forbids automatic repeated mentions.
        if self.trigger.mode.repeats() && self.has_discord() {
            return Err(("err.repeat_discord", None));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Contrast,
    System,
    Jewish,
}

impl Theme {
    pub const ALL: [Theme; 5] = [Theme::Dark, Theme::Light, Theme::Contrast, Theme::System, Theme::Jewish];

    pub fn key(self) -> &'static str {
        match self {
            Theme::Dark => "theme.dark",
            Theme::Light => "theme.light",
            Theme::Contrast => "theme.contrast",
            Theme::System => "theme.system",
            Theme::Jewish => "theme.jewish",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Ru,
    #[default]
    En,
    He,
    Pain,
}

impl Language {
    pub const ALL: [Language; 4] = [Language::Ru, Language::En, Language::He, Language::Pain];

    /// The language's own name, as shown in Settings.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::Ru => "Русский",
            Language::En => "English",
            Language::He => "עברית",
            Language::Pain => "Язык боли",
        }
    }

    pub fn is_rtl(self) -> bool {
        self == Language::He
    }

    /// Russian-style languages use a comma as the decimal separator.
    pub fn decimal_comma(self) -> bool {
        matches!(self, Language::Ru | Language::Pain)
    }
}

pub fn default_profiles() -> Vec<Profile> {
    ["Default", "Gaming", "Work", "Discord"]
        .iter()
        .enumerate()
        .map(|(i, n)| Profile { id: i as u32 + 1, name: n.to_string() })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema: u32,
    pub profiles: Vec<Profile>,
    pub active_profile: u32,
    pub bindings: Vec<Binding>,
    /// Global cooldown in milliseconds (10 ms – 1 hour).
    pub cooldown_ms: u32,
    pub combo_window_ms: u32,
    pub sequence_timeout_ms: u32,
    pub double_tap_ms: u32,
    pub long_press_ms: u32,
    /// The "Disable all binds" switch. Ctrl+Alt+Pause flips it.
    pub bindings_enabled: bool,
    pub theme: Theme,
    pub language: Language,
    /// Global shortcut that switches to the next profile.
    pub profile_switch: Option<Shortcut>,
    pub start_with_windows: bool,
    pub minimize_to_tray: bool,
    pub launch_minimized: bool,
    pub notifications: bool,
    /// GitHub repository with releases, "owner/name". Empty disables updates.
    pub update_repo: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            profiles: default_profiles(),
            active_profile: DEFAULT_PROFILE,
            bindings: Vec::new(),
            cooldown_ms: 3000,
            combo_window_ms: 350,
            sequence_timeout_ms: 1000,
            double_tap_ms: 300,
            long_press_ms: 600,
            bindings_enabled: true,
            theme: Theme::Dark,
            language: Language::En,
            profile_switch: None,
            start_with_windows: false,
            minimize_to_tray: true,
            launch_minimized: false,
            notifications: true,
            update_repo: DEFAULT_UPDATE_REPO.to_string(),
        }
    }
}

pub const COOLDOWN_RANGE_MS: (u32, u32) = (10, 3_600_000);
pub const COMBO_WINDOW_RANGE: (u32, u32) = (100, 2000);
pub const SEQUENCE_TIMEOUT_RANGE: (u32, u32) = (300, 5000);
pub const DOUBLE_TAP_RANGE: (u32, u32) = (100, 1000);
pub const LONG_PRESS_RANGE: (u32, u32) = (200, 5000);

impl Config {
    pub fn binding(&self, id: u32) -> Option<&Binding> {
        self.bindings.iter().find(|b| b.id == id)
    }

    pub fn next_id(&self) -> u32 {
        self.bindings.iter().map(|b| b.id).max().unwrap_or(0) + 1
    }

    pub fn profile(&self, id: u32) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn next_profile_id(&self) -> u32 {
        self.profiles.iter().map(|p| p.id).max().unwrap_or(0) + 1
    }

    /// True when the binding belongs to the active profile (or to all).
    pub fn in_active_profile(&self, b: &Binding) -> bool {
        b.profile == ALL_PROFILES || b.profile == self.active_profile
    }

    pub fn cooldown_for(&self, b: &Binding) -> u32 {
        b.cooldown_ms.unwrap_or(self.cooldown_ms)
    }

    /// Bindings whose shortcuts interfere with `shortcut` (excluding `self_id`)
    /// and can be active at the same time as a binding in `profile`.
    pub fn conflicts_with(&self, shortcut: &Shortcut, self_id: Option<u32>, profile: u32) -> Vec<String> {
        let mut out = Vec::new();
        for b in self.bindings.iter().filter(|b| Some(b.id) != self_id && b.enabled) {
            let overlap = profile == ALL_PROFILES || b.profile == ALL_PROFILES || b.profile == profile;
            if !overlap {
                continue;
            }
            for other in &b.shortcuts {
                if let Some(why) = crate::shortcut::conflict(shortcut, other) {
                    out.push(format!("\"{}\" {why}", b.name));
                }
            }
        }
        if let Some(ps) = &self.profile_switch
            && crate::shortcut::conflict(shortcut, ps).is_some()
        {
            out.push("profile switch shortcut".into());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_validation() {
        assert!(is_web_url("https://example.com"));
        assert!(is_web_url("http://github.com/x?y=1"));
        assert!(!is_web_url("file:///c:/x"));
        assert!(!is_web_url("https://"));
        assert!(!is_web_url("https://a.b/\"x"));
        assert!(!is_web_url("javascript:alert(1)"));
    }

    #[test]
    fn open_file_blocks_scripts() {
        let file = |p: &str| Action::OpenFile { path: p.into() }.validate();
        let app = |p: &str| Action::OpenApp { path: p.into(), args: String::new() }.validate();
        assert!(file(r"C:\a\run.BAT").is_err());
        assert!(file(r"C:\a\run.bat. . ").is_err(), "trailing dots/spaces must not bypass the check");
        assert!(file(r"C:\a\x.lnk").is_err());
        assert!(file(r"C:\a\notes.txt:evil.bat").is_err(), "alternate data streams are rejected");
        assert!(file(r"C:\a\notes.txt").is_ok());
        assert!(app(r"C:\a\tool.exe").is_ok());
        assert!(app(r"C:\a\tool.bat").is_err());
        assert!(app(r"C:\Windows\System32\cmd.exe").is_err());
        assert!(app(r"C:\Windows\System32\WindowsPowerShell\v1.0\POWERSHELL.EXE").is_err());
    }

    #[test]
    fn repeat_modes_are_refused_for_discord() {
        let mut b = Binding::new(1, ALL_PROFILES);
        b.name = "x".into();
        b.actions = vec![Action::DiscordPing {
            guild_id: String::new(),
            guild_name: String::new(),
            channel_id: "222222222222222222".into(),
            channel_name: String::new(),
            user_id: "333333333333333333".into(),
            user_name: String::new(),
        }];
        assert!(b.validate().is_ok());
        b.trigger.mode = TriggerMode::Repeat;
        assert_eq!(b.validate(), Err(("err.repeat_discord", None)));
        b.trigger.mode = TriggerMode::Toggle;
        assert!(b.validate().is_err());
        b.trigger.mode = TriggerMode::DoubleTap;
        assert!(b.validate().is_ok());
    }

    #[test]
    fn conditions_match_foreground() {
        let fg = Foreground { process: "Chrome.exe".into(), title: "GitHub - Google Chrome".into(), fullscreen: false };
        let c = |p: &str, t: &str, f: bool| Conditions { process: p.into(), window_title: t.into(), skip_fullscreen: f };
        assert!(c("", "", false).allows(&fg));
        assert!(c("chrome", "", false).allows(&fg));
        assert!(c("chrome.exe", "github", false).allows(&fg));
        assert!(!c("firefox", "", false).allows(&fg));
        assert!(!c("", "gitlab", false).allows(&fg));
        let full = Foreground { fullscreen: true, ..fg };
        assert!(!c("", "", true).allows(&full));
    }

    #[test]
    fn profile_scope_and_cooldown() {
        let mut c = Config::default();
        let mut b = Binding::new(1, 2);
        assert!(!c.in_active_profile(&b));
        c.active_profile = 2;
        assert!(c.in_active_profile(&b));
        b.profile = ALL_PROFILES;
        c.active_profile = 3;
        assert!(c.in_active_profile(&b));
        assert_eq!(c.cooldown_for(&b), 3000);
        b.cooldown_ms = Some(10);
        assert_eq!(c.cooldown_for(&b), 10);
    }
}
