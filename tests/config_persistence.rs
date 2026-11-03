//! Settings persistence: save/load round trip, migration from v1, corrupted
//! and invalid files.

use nokey::config::{self, Action, Binding, Config, Language, Theme, TriggerMode, ALL_PROFILES, SCHEMA_VERSION};
use nokey::shortcut::{Shortcut, Stroke, MOD_CTRL};
use std::fs;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nokey-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A config written by NoKey 0.1 (schema v1; IDs are made up).
const V1_FILE: &str = r#"{
  "bindings": [
    { "id": 1, "name": "Discord Ping", "enabled": true,
      "shortcut": { "type": "standard", "mods": 0, "vk": 119 },
      "action": { "kind": "discord_ping", "guild_id": "111111111111111111", "guild_name": "My Server",
                  "channel_id": "222222222222222222", "channel_name": "general",
                  "user_id": "333333333333333333", "user_name": "friend" } },
    { "id": 2, "name": "Open Browser", "enabled": true,
      "shortcut": { "type": "standard", "mods": 7, "vk": 81 },
      "action": { "kind": "open_url", "url": "https://example.com" } },
    { "id": 3, "name": "Both", "enabled": false,
      "shortcut": { "type": "sequence", "steps": [ {"mods":0,"vk":123}, {"mods":0,"vk":122} ] },
      "action": { "kind": "multi", "binding_ids": [2, 1] } }
  ],
  "cooldown_secs": 5,
  "minimize_to_tray": true,
  "pause_hotkey": true
}"#;

#[test]
fn v1_file_is_migrated_with_backup() {
    let dir = temp_dir("migrate");
    fs::write(config::config_path(&dir), V1_FILE).unwrap();
    let loaded = config::load(&dir, Language::Ru);
    assert!(loaded.warnings.is_empty(), "warnings: {:?}", loaded.warnings);
    let c = &loaded.config;
    assert_eq!(c.schema, SCHEMA_VERSION);
    assert_eq!(c.bindings.len(), 3, "no binding may be lost");
    assert_eq!(c.cooldown_ms, 5000);
    assert_eq!(c.language, Language::Ru);
    assert_eq!(c.bindings[2].actions.len(), 2, "multi became two inline actions");

    let backup = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("config.v1-backup-"))
        .expect("the original v1 file must be backed up");
    assert_eq!(fs::read_to_string(backup.path()).unwrap(), V1_FILE, "backup is the untouched original");

    // The upgraded file was saved: loading again gives the same config without a new backup.
    let again = config::load(&dir, Language::En);
    assert_eq!(&again.config, c);
    let backups =
        fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains("backup")).count();
    assert_eq!(backups, 1);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn save_then_load_round_trip() {
    let dir = temp_dir("roundtrip");
    let mut ping = Binding {
        name: "Ping".into(),
        description: "Wake up a friend".into(),
        icon: "🔔".into(),
        shortcuts: vec![
            Shortcut::Standard { mods: MOD_CTRL, vk: 0x51 },
            Shortcut::Sequence { steps: vec![Stroke { mods: 0, vk: 0x51 }, Stroke { mods: 0, vk: 0x45 }] },
        ],
        actions: vec![
            Action::OpenUrl { url: "https://example.com".into() },
            Action::DiscordPing {
                guild_id: "111111111111111111".into(),
                guild_name: "Server".into(),
                channel_id: "222222222222222222".into(),
                channel_name: "general".into(),
                user_id: "333333333333333333".into(),
                user_name: "friend".into(),
            },
        ],
        step_delay_ms: 250,
        cooldown_ms: Some(10),
        priority: 7,
        sound: true,
        ..Binding::new(1, 2)
    };
    ping.trigger.mode = TriggerMode::DoubleTap;
    ping.conditions.process = "chrome.exe".into();
    let config = Config {
        cooldown_ms: 10,
        theme: Theme::Jewish,
        language: Language::He,
        bindings_enabled: false,
        active_profile: 3,
        profile_switch: Some(Shortcut::Standard { mods: MOD_CTRL, vk: 0x7B }),
        bindings: vec![ping],
        ..Config::default()
    };
    config::save(&dir, &config).unwrap();
    let loaded = config::load(&dir, Language::En);
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.config, config);
    assert!(!dir.join("config.json.tmp").exists(), "temp file must be renamed away");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn missing_file_gives_defaults_in_the_system_language() {
    let dir = temp_dir("missing");
    let loaded = config::load(&dir, Language::He);
    assert!(loaded.warnings.is_empty());
    assert_eq!(loaded.config.language, Language::He);
    assert_eq!(loaded.config.profiles.len(), 4, "Default, Gaming, Work, Discord");
    assert!(loaded.config.bindings_enabled);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn corrupted_file_is_kept_aside_and_defaults_used() {
    let dir = temp_dir("corrupt");
    fs::write(config::config_path(&dir), "{ this is not json").unwrap();
    let loaded = config::load(&dir, Language::En);
    assert_eq!(loaded.config, Config::default());
    assert_eq!(loaded.warnings.len(), 1);
    assert!(loaded.warnings[0].contains("corrupted"));
    let kept = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("config.corrupt-"));
    assert!(kept, "the broken file must be preserved for the user");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn invalid_values_are_repaired_with_warnings() {
    let dir = temp_dir("invalid");
    let text = r#"{ "schema": 2, "cooldown_ms": 1, "sequence_timeout_ms": 1, "update_repo": "not a repo",
      "bindings": [ { "id": 1, "name": "Bad", "enabled": true, "profile": 99,
        "shortcuts": [ { "type": "standard", "mods": 0, "vk": 65 },
                       { "type": "standard", "mods": 3, "vk": 19 } ],
        "trigger": { "mode": "repeat" },
        "actions": [ { "kind": "discord_ping", "guild_id": "", "channel_id": "222222222222222222",
                       "user_id": "333333333333333333" } ] } ] }"#;
    fs::write(config::config_path(&dir), text).unwrap();
    let loaded = config::load(&dir, Language::En);
    let c = &loaded.config;
    assert_eq!(c.cooldown_ms, 10, "minimum 0.01 s");
    assert_eq!(c.sequence_timeout_ms, 300);
    assert!(c.update_repo.is_empty());
    let b = &c.bindings[0];
    assert!(b.shortcuts.is_empty(), "plain 'A' and the reserved Ctrl+Alt+Pause are both removed");
    assert_eq!(b.profile, ALL_PROFILES, "unknown profile");
    assert_eq!(b.trigger.mode, TriggerMode::Press, "Discord pings may not repeat");
    assert_eq!(loaded.warnings.len(), 4, "{:?}", loaded.warnings);
    let _ = fs::remove_dir_all(dir);
}
