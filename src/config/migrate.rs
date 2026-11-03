//! Converts older config files to the current schema.
//!
//! v1 (NoKey 0.1): one shortcut and one action per binding, "multi" actions
//! that referenced other bindings, cooldown in whole seconds.

use super::model::*;
use crate::shortcut::Shortcut;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum V1Action {
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
    OpenUrl {
        url: String,
    },
    Multi {
        binding_ids: Vec<u32>,
    },
}

#[derive(Deserialize)]
struct V1Binding {
    id: u32,
    name: String,
    enabled: bool,
    shortcut: Option<Shortcut>,
    action: V1Action,
}

#[derive(Deserialize)]
#[serde(default)]
struct V1Config {
    bindings: Vec<V1Binding>,
    cooldown_secs: u32,
    combo_window_ms: u32,
    sequence_timeout_ms: u32,
    start_with_windows: bool,
    minimize_to_tray: bool,
    launch_minimized: bool,
    notifications: bool,
    update_repo: String,
}

impl Default for V1Config {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
            cooldown_secs: 3,
            combo_window_ms: 350,
            sequence_timeout_ms: 1000,
            start_with_windows: false,
            minimize_to_tray: true,
            launch_minimized: false,
            notifications: true,
            update_repo: String::new(),
        }
    }
}

fn simple_action(a: &V1Action) -> Option<Action> {
    Some(match a {
        V1Action::DiscordPing { guild_id, guild_name, channel_id, channel_name, user_id, user_name } => {
            Action::DiscordPing {
                guild_id: guild_id.clone(),
                guild_name: guild_name.clone(),
                channel_id: channel_id.clone(),
                channel_name: channel_name.clone(),
                user_id: user_id.clone(),
                user_name: user_name.clone(),
            }
        }
        V1Action::OpenApp { path, args } => Action::OpenApp { path: path.clone(), args: args.clone() },
        V1Action::OpenFile { path } => Action::OpenFile { path: path.clone() },
        V1Action::OpenFolder { path } => Action::OpenFolder { path: path.clone() },
        V1Action::OpenUrl { url } => Action::OpenUrl { url: url.clone() },
        V1Action::Multi { .. } => return None,
    })
}

/// The schema version of a parsed config file (files without one are v1).
pub fn schema_of(value: &Value) -> u32 {
    value.get("schema").and_then(Value::as_u64).map(|v| v as u32).unwrap_or(1)
}

/// Converts a v1 file. The caller backs up the original first.
pub fn from_v1(value: Value, language: Language) -> Result<Config, String> {
    let old: V1Config = serde_json::from_value(value).map_err(|e| e.to_string())?;
    let bindings = old
        .bindings
        .iter()
        .map(|b| {
            let actions = match &b.action {
                // A "multi" action becomes the referenced bindings' actions, in order.
                V1Action::Multi { binding_ids } => binding_ids
                    .iter()
                    .filter_map(|id| old.bindings.iter().find(|x| x.id == *id))
                    .filter_map(|x| simple_action(&x.action))
                    .collect(),
                other => simple_action(other).into_iter().collect(),
            };
            Binding {
                name: b.name.clone(),
                enabled: b.enabled,
                shortcuts: b.shortcut.clone().into_iter().collect(),
                actions,
                ..Binding::new(b.id, ALL_PROFILES)
            }
        })
        .collect();
    Ok(Config {
        bindings,
        cooldown_ms: old.cooldown_secs.saturating_mul(1000),
        combo_window_ms: old.combo_window_ms,
        sequence_timeout_ms: old.sequence_timeout_ms,
        start_with_windows: old.start_with_windows,
        minimize_to_tray: old.minimize_to_tray,
        launch_minimized: old.launch_minimized,
        notifications: old.notifications,
        update_repo: old.update_repo,
        language,
        ..Config::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = r#"{
      "bindings": [
        { "id": 1, "name": "Ping", "enabled": true,
          "shortcut": { "type": "standard", "mods": 0, "vk": 119 },
          "action": { "kind": "discord_ping", "guild_id": "111111111111111111",
                      "channel_id": "222222222222222222", "user_id": "333333333333333333", "user_name": "friend" } },
        { "id": 2, "name": "Web", "enabled": true, "shortcut": null,
          "action": { "kind": "open_url", "url": "https://example.com" } },
        { "id": 3, "name": "Both", "enabled": false,
          "shortcut": { "type": "combo", "keys": [65, 83] },
          "action": { "kind": "multi", "binding_ids": [2, 1, 99] } }
      ],
      "cooldown_secs": 5, "pause_hotkey": false, "minimize_to_tray": false
    }"#;

    #[test]
    fn detects_schema() {
        assert_eq!(schema_of(&serde_json::from_str(V1).unwrap()), 1);
        assert_eq!(schema_of(&serde_json::json!({"schema": 2})), 2);
    }

    #[test]
    fn converts_v1_without_losing_bindings() {
        let c = from_v1(serde_json::from_str(V1).unwrap(), Language::Ru).unwrap();
        assert_eq!(c.schema, SCHEMA_VERSION);
        assert_eq!(c.bindings.len(), 3);
        assert_eq!(c.cooldown_ms, 5000);
        assert!(!c.minimize_to_tray);
        assert_eq!(c.language, Language::Ru);
        assert_eq!(c.bindings[0].shortcuts.len(), 1);
        assert!(c.bindings[1].shortcuts.is_empty());
        let multi = &c.bindings[2];
        assert_eq!(multi.actions.len(), 2, "multi expands to the referenced actions; missing ids are dropped");
        assert_eq!(multi.actions[0].kind(), ActionKind::OpenUrl);
        assert_eq!(multi.actions[1].kind(), ActionKind::DiscordPing);
        assert!(!multi.enabled);
        assert!(c.bindings.iter().all(|b| b.profile == ALL_PROFILES));
        assert_eq!(c.profiles.len(), 4);
    }
}
