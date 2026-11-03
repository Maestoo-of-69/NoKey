//! Local configuration (`config.json`). The bot token is NOT stored here —
//! see `security::dpapi`.

pub mod migrate;
mod model;
pub mod timing;

pub use model::*;

use crate::shortcut::{Shortcut, MOD_ALT, MOD_CTRL};
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "config.json";
const MAX_ICON_CHARS: usize = 8;
const VK_PAUSE: u16 = 0x13;
const VK_CANCEL: u16 = 0x03;

/// Ctrl+Alt+Pause is reserved for the emergency "disable all" switch.
pub fn is_reserved(sc: &Shortcut) -> bool {
    matches!(sc, Shortcut::Standard { mods, vk } if *mods == MOD_CTRL | MOD_ALT && (*vk == VK_PAUSE || *vk == VK_CANCEL))
}

fn clamp(v: &mut u32, (lo, hi): (u32, u32)) {
    *v = (*v).clamp(lo, hi);
}

/// Checks a shortcut before it is stored.
pub fn check_shortcut(sc: &Shortcut) -> Result<(), String> {
    sc.validate()?;
    if is_reserved(sc) {
        return Err("Ctrl + Alt + Pause is reserved for turning all bindings off and on.".into());
    }
    Ok(())
}

impl Config {
    /// Fixes values a hand-edited or older file could contain. Returns
    /// notes for the user about anything that was changed.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        self.schema = SCHEMA_VERSION;
        clamp(&mut self.cooldown_ms, COOLDOWN_RANGE_MS);
        clamp(&mut self.combo_window_ms, COMBO_WINDOW_RANGE);
        clamp(&mut self.sequence_timeout_ms, SEQUENCE_TIMEOUT_RANGE);
        clamp(&mut self.double_tap_ms, DOUBLE_TAP_RANGE);
        clamp(&mut self.long_press_ms, LONG_PRESS_RANGE);

        // Profiles: unique ids, the Default profile always exists.
        let mut seen = Vec::new();
        self.profiles.retain(|p| {
            let ok = p.id != ALL_PROFILES && !seen.contains(&p.id);
            seen.push(p.id);
            ok
        });
        if self.profile(DEFAULT_PROFILE).is_none() {
            self.profiles.insert(0, Profile { id: DEFAULT_PROFILE, name: "Default".into() });
        }
        for p in &mut self.profiles {
            if p.name.trim().is_empty() {
                p.name = format!("Profile {}", p.id);
            }
        }
        if self.profile(self.active_profile).is_none() {
            self.active_profile = DEFAULT_PROFILE;
        }

        let mut seen = Vec::new();
        self.bindings.retain(|b| {
            let ok = b.id > 0 && b.id < MAX_BINDING_ID && !seen.contains(&b.id);
            seen.push(b.id);
            if !ok {
                notes.push(format!("Skipped binding \"{}\" with an invalid id.", b.name));
            }
            ok
        });
        let profile_ids: Vec<u32> = self.profiles.iter().map(|p| p.id).collect();
        for b in &mut self.bindings {
            b.shortcuts.retain(|sc| match check_shortcut(sc) {
                Ok(()) => true,
                Err(e) => {
                    notes.push(format!("\"{}\": {e} The shortcut was removed.", b.name));
                    false
                }
            });
            b.shortcuts.truncate(MAX_SHORTCUTS);
            b.actions.truncate(MAX_ACTIONS);
            if b.profile != ALL_PROFILES && !profile_ids.contains(&b.profile) {
                b.profile = ALL_PROFILES;
            }
            clamp(&mut b.trigger.repeat_interval_ms, REPEAT_INTERVAL_RANGE);
            clamp(&mut b.trigger.repeat_max, REPEAT_MAX_RANGE);
            clamp(&mut b.step_delay_ms, STEP_DELAY_RANGE);
            if let Some(c) = &mut b.cooldown_ms {
                clamp(c, COOLDOWN_RANGE_MS);
            }
            b.priority = b.priority.min(MAX_PRIORITY);
            if b.icon.chars().count() > MAX_ICON_CHARS {
                b.icon = b.icon.chars().take(MAX_ICON_CHARS).collect();
            }
            if b.trigger.mode.repeats() && b.has_discord() {
                notes.push(format!("\"{}\": repeating Discord pings are not allowed; set to Press.", b.name));
                b.trigger.mode = TriggerMode::Press;
            }
        }
        if let Some(sc) = &self.profile_switch
            && let Err(e) = check_shortcut(sc)
        {
            notes.push(format!("Profile switch shortcut: {e} It was removed."));
            self.profile_switch = None;
        }
        if !self.update_repo.is_empty() && !crate::update::is_valid_repo(&self.update_repo) {
            notes.push("The update repository name was invalid and has been cleared.".into());
            self.update_repo.clear();
        }
        notes
    }
}

pub struct Loaded {
    pub config: Config,
    /// Shown to the user when the file was missing parts or corrupted.
    pub warnings: Vec<String>,
}

pub fn config_path(dir: &Path) -> PathBuf {
    dir.join(CONFIG_FILE)
}

fn unix_time() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn corrupted(dir: &Path, path: &Path, why: &str, language: Language) -> Loaded {
    let backup = dir.join(format!("config.corrupt-{}.json", unix_time()));
    let moved = fs::rename(path, &backup).is_ok();
    let mut msg = format!("Settings file is corrupted ({why}). Default settings are used.");
    if moved {
        msg.push_str(&format!(" The old file was kept as {}.", backup.display()));
    }
    Loaded { config: Config { language, ..Config::default() }, warnings: vec![msg] }
}

/// Loads the settings. Older files are upgraded: the original is copied to
/// `config.v1-backup-<time>.json` first, the result is validated, then saved.
pub fn load(dir: &Path, default_language: Language) -> Loaded {
    let path = config_path(dir);
    let fresh = || Config { language: default_language, ..Config::default() };
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded { config: fresh(), warnings: Vec::new() };
        }
        Err(e) => {
            return Loaded {
                config: fresh(),
                warnings: vec![format!("Could not read {}: {e}. Using default settings.", path.display())],
            };
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return corrupted(dir, &path, &e.to_string(), default_language),
    };
    let schema = migrate::schema_of(&value);
    let mut warnings = Vec::new();
    let parsed = match schema {
        1 => {
            let backup = dir.join(format!("config.v1-backup-{}.json", unix_time()));
            if let Err(e) = fs::copy(&path, &backup) {
                return Loaded {
                    config: fresh(),
                    warnings: vec![format!("Could not back up the old settings ({e}); they were not changed.")],
                };
            }
            migrate::from_v1(value, default_language)
        }
        _ => {
            if schema > SCHEMA_VERSION {
                warnings.push("The settings file was written by a newer NoKey; unknown options are ignored.".into());
            }
            serde_json::from_value::<Config>(value).map_err(|e| e.to_string())
        }
    };
    let mut config = match parsed {
        Ok(c) => c,
        Err(e) => return corrupted(dir, &path, &e, default_language),
    };
    warnings.extend(config.sanitize());
    if schema == 1
        && let Err(e) = save(dir, &config)
    {
        warnings.push(format!("Upgraded settings could not be saved: {e}"));
    }
    Loaded { config, warnings }
}

/// Writes atomically: a crash mid-write never leaves a half-written file.
pub fn save(dir: &Path, config: &Config) -> Result<(), String> {
    use std::io::Write;
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp = dir.join("config.json.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("Cannot write settings: {e}"))?;
        f.write_all(json.as_bytes()).map_err(|e| format!("Cannot write settings: {e}"))?;
        f.sync_all().map_err(|e| format!("Cannot write settings: {e}"))?;
    }
    fs::rename(&tmp, config_path(dir)).map_err(|e| format!("Cannot save settings: {e}"))
}

/// `%APPDATA%\NoKey`, or `NOKEY_DATA_DIR` when set (portable use and tests).
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NOKEY_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("NoKey")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url_binding(id: u32) -> Binding {
        Binding {
            name: "x".into(),
            shortcuts: vec![Shortcut::Standard { mods: 0, vk: 0x41 }],
            actions: vec![Action::OpenUrl { url: "https://a.b".into() }],
            ..Binding::new(id, ALL_PROFILES)
        }
    }

    #[test]
    fn sanitize_clamps_and_dedupes() {
        let mut c = Config { cooldown_ms: 0, combo_window_ms: 99_999, active_profile: 77, ..Config::default() };
        c.bindings = vec![url_binding(1), url_binding(1)];
        let notes = c.sanitize();
        assert_eq!(c.cooldown_ms, 10);
        assert_eq!(c.combo_window_ms, 2000);
        assert_eq!(c.active_profile, DEFAULT_PROFILE);
        assert_eq!(c.bindings.len(), 1);
        assert!(c.bindings[0].shortcuts.is_empty(), "plain letter shortcut must be dropped");
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn emergency_shortcut_is_reserved() {
        let reserved = Shortcut::Standard { mods: MOD_CTRL | MOD_ALT, vk: VK_PAUSE };
        assert!(check_shortcut(&reserved).is_err());
        assert!(check_shortcut(&Shortcut::Standard { mods: MOD_CTRL | MOD_ALT, vk: VK_CANCEL }).is_err());
        assert!(check_shortcut(&Shortcut::Standard { mods: MOD_CTRL, vk: VK_PAUSE }).is_ok());
    }

    #[test]
    fn default_profile_is_restored() {
        let mut c = Config { profiles: vec![Profile { id: 5, name: String::new() }], ..Config::default() };
        let mut b = url_binding(1);
        b.profile = 9;
        c.bindings = vec![b];
        c.sanitize();
        assert!(c.profile(DEFAULT_PROFILE).is_some());
        assert_eq!(c.profile(5).unwrap().name, "Profile 5");
        assert_eq!(c.bindings[0].profile, ALL_PROFILES, "binding of a missing profile goes to all profiles");
    }
}
