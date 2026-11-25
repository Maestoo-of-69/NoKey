//! What the main window shows, built from the app state: list rows, the
//! header status, the detail line and the Run button label. No drawing here.

use crate::app::{actions_summary, profile_name, App, Conn};
use crate::config::{ActionKind, Binding, TriggerMode};
use crate::i18n::t;
use crate::ui::sentence_case;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SortKey {
    #[default]
    Name,
    Shortcut,
    Action,
    Profile,
    Enabled,
}

pub(super) const SORT_KEYS: [(SortKey, &str); 5] = [
    (SortKey::Name, "sort.name"),
    (SortKey::Shortcut, "sort.shortcut"),
    (SortKey::Action, "sort.action"),
    (SortKey::Profile, "sort.profile"),
    (SortKey::Enabled, "sort.enabled"),
];

/// Sort label for the button: "Sort: name" -> "name" (the menu keeps the full
/// label). Every locale writes "<prefix>: <value>"; without ": " the full label is used.
pub(super) fn sort_short(key: &str) -> String {
    let full = t(key);
    match full.split_once(": ") {
        Some((_, value)) if !value.trim().is_empty() => sentence_case(value.trim()),
        _ => full,
    }
}

#[derive(Default, Clone)]
pub(super) struct Row {
    pub id: u32,
    pub icon: String,
    pub name: String,
    pub summary: String,
    /// First shortcut label ("" = none).
    pub keys: String,
    /// Number of further shortcuts.
    pub extra: usize,
    /// "Off" / "Paused" chip shown instead of keycaps.
    pub state_chip: Option<String>,
    pub enabled: bool,
    pub problem: bool,
}

#[derive(Default, Clone)]
pub(super) struct View {
    pub rows: Vec<Row>,
    pub total: usize,
    pub connected: bool,
    /// The bot token is being checked (the status dot pulses).
    pub checking: bool,
    pub conn_short: String,
    pub all_enabled: bool,
    pub emergency_error: Option<String>,
    pub profile: String,
    pub last_result: Option<(bool, String)>,
    /// Selected binding: actions · trigger · profile (· problem).
    pub detail: String,
    pub run_label: Option<String>,
    pub selected_paused: bool,
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

fn make_row(a: &App, b: &Binding, paused_now: bool, show_all: bool) -> Row {
    let mut summary = actions_summary(b);
    if b.trigger.mode != TriggerMode::Press {
        summary = format!("{} · {summary}", t(b.trigger.mode.key()));
    }
    if show_all {
        summary = format!("{} · {summary}", profile_name(&a.config, b.profile));
    }
    let state_chip = if !b.enabled {
        Some(t("main.off"))
    } else if paused_now {
        Some(sentence_case(&t("main.paused")))
    } else {
        None
    };
    Row {
        id: b.id,
        icon: if b.icon.is_empty() { b.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default() } else { b.icon.clone() },
        name: b.name.clone(),
        summary,
        keys: b.shortcuts.first().map(|s| s.label()).unwrap_or_default(),
        extra: b.shortcuts.len().saturating_sub(1),
        state_chip,
        enabled: b.enabled && !paused_now,
        problem: a.hotkey_errors.contains_key(&b.id),
    }
}

pub(super) fn build_view(a: &mut App, selected: Option<u32>, sort: SortKey, show_all: bool, query: &str) -> View {
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
    let rows = list.iter().map(|b| make_row(a, b, paused.contains(&b.id), show_all)).collect();
    let mut v = View {
        rows,
        total: a.config.bindings.iter().filter(|b| b.enabled && a.config.in_active_profile(b)).count(),
        connected: matches!(a.conn, Conn::Connected(_)),
        checking: matches!(a.conn, Conn::Checking),
        conn_short: match a.conn {
            Conn::Connected(_) => t("main.connected"),
            Conn::Checking => t("main.checking"),
            _ => t("main.disconnected"),
        },
        all_enabled: a.config.bindings_enabled,
        emergency_error: a.emergency_error.clone(),
        profile: profile_name(&a.config, a.config.active_profile),
        last_result: a.last_result.clone(),
        ..View::default()
    };
    if let Some(b) = selected.and_then(|id| a.config.binding(id)) {
        v.selected_paused = paused.contains(&b.id);
        let problem = a.hotkey_errors.get(&b.id).map(|e| format!(" · ⚠ {e}")).unwrap_or_default();
        v.detail = format!("{} · {} · {}{problem}", actions_summary(b), t(b.trigger.mode.key()), profile_name(&a.config, b.profile));
        let all_discord = b.actions.iter().all(|x| x.kind() == ActionKind::DiscordPing);
        v.run_label = Some(t(if all_discord { "main.run_ping" } else { "main.run" }));
    }
    v
}

pub(super) fn run_text(label: &str) -> String {
    format!("▶ {label}")
}

/// Every label the Run button can show; its width fits the widest so it never jumps.
pub(super) fn run_labels() -> [String; 4] {
    [run_text(&t("main.run")), run_text(&t("main.run_ping")), t("main.done"), t("main.failed")]
}

pub(super) fn run_label(view: &View, flash: Option<(u32, bool)>, selected: Option<u32>) -> String {
    match (&view.run_label, flash) {
        (None, _) => run_text(&t("main.run")),
        (Some(_), Some((id, ok))) if Some(id) == selected => t(if ok { "main.done" } else { "main.failed" }),
        (Some(label), _) => run_text(label),
    }
}

/// The line above the action bar: a fresh run result, else the selection.
/// The flag is false for a failure (drawn in the text color, not muted).
pub(super) fn detail_line(v: &View, flash: Option<(u32, bool)>) -> (String, bool) {
    let result = |ok: bool, msg: &str| (format!("{} {msg}", if ok { "✓" } else { "✕" }), ok);
    if let Some((_, flash_ok)) = flash {
        return match &v.last_result {
            Some((ok, msg)) => result(*ok, msg),
            None => result(flash_ok, &t(if flash_ok { "main.done" } else { "main.failed" })),
        };
    }
    if !v.detail.is_empty() {
        return (v.detail.clone(), true);
    }
    match &v.last_result {
        Some((ok, msg)) => result(*ok, msg),
        None => (t("main.nothing_selected"), true),
    }
}
