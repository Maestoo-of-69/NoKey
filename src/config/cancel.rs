//! Rules for a binding's cancel shortcut (`Binding::cancel_shortcut`): a
//! standard shortcut (RegisterHotKey) that stops the binding's running job.
//! Pure and language-neutral; errors are localization keys.

use super::{is_reserved, Binding};
use crate::shortcut::{conflict, Shortcut};

/// Checks the binding's cancel shortcut, if it has one.
pub(super) fn validate(b: &Binding) -> Result<(), &'static str> {
    b.cancel_shortcut.as_ref().map_or(Ok(()), |sc| check(sc, &b.shortcuts))
}

/// Checks `sc` as the cancel shortcut of a binding fired by `own`. Conflicts
/// with other bindings are allowed: it is registered only while this one runs.
pub fn check(sc: &Shortcut, own: &[Shortcut]) -> Result<(), &'static str> {
    if !matches!(sc, Shortcut::Standard { .. }) {
        return Err("err.cancel_standard");
    }
    if sc.validate().is_err() {
        return Err("err.cancel_invalid");
    }
    if is_reserved(sc) {
        return Err("err.cancel_reserved");
    }
    if own.iter().any(|o| conflict(sc, o).is_some()) {
        return Err("err.cancel_same");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{Action, ALL_PROFILES};
    use super::*;
    use crate::shortcut::{Stroke, MOD_ALT, MOD_CTRL, MOD_SHIFT};

    fn binding(cancel: Option<Shortcut>) -> Binding {
        let mut b = Binding::new(1, ALL_PROFILES);
        b.name = "x".into();
        b.actions = vec![Action::OpenUrl { url: "https://example.com".into() }];
        b.shortcuts = vec![Shortcut::Standard { mods: MOD_CTRL, vk: 0x51 }];
        b.cancel_shortcut = cancel;
        b
    }

    fn std(mods: u8, vk: u16) -> Option<Shortcut> {
        Some(Shortcut::Standard { mods, vk })
    }

    #[test]
    fn accepts_none_and_a_distinct_standard_shortcut() {
        assert_eq!(validate(&binding(None)), Ok(()));
        assert_eq!(validate(&binding(std(MOD_CTRL | MOD_SHIFT, 0x51))), Ok(()));
        assert_eq!(validate(&binding(std(0, 0x7B))), Ok(()), "F12 alone");
        assert_eq!(binding(std(0, 0x1B)).validate(), Ok(()), "Esc alone is allowed");
    }

    #[test]
    fn refuses_bad_cancel_shortcuts() {
        let combo = Some(Shortcut::Combo { keys: vec![0x41, 0x53] });
        assert_eq!(validate(&binding(combo)), Err("err.cancel_standard"));
        let seq = Some(Shortcut::Sequence { steps: vec![Stroke { mods: 0, vk: 0x41 }, Stroke { mods: 0, vk: 0x42 }] });
        assert_eq!(validate(&binding(seq)), Err("err.cancel_standard"));
        assert_eq!(validate(&binding(std(0, 0x41))), Err("err.cancel_invalid"), "a plain letter blocks typing");
        assert_eq!(validate(&binding(std(MOD_CTRL, 0x11))), Err("err.cancel_invalid"), "modifier only");
        assert_eq!(validate(&binding(std(MOD_CTRL | MOD_ALT, 0x13))), Err("err.cancel_reserved"));
        assert_eq!(validate(&binding(std(MOD_CTRL | MOD_ALT, 0x03))), Err("err.cancel_reserved"));
        assert_eq!(validate(&binding(std(MOD_CTRL, 0x51))), Err("err.cancel_same"), "the bind's own shortcut");
        assert_eq!(binding(std(MOD_CTRL, 0x51)).validate(), Err(("err.cancel_same", None)), "Binding::validate checks it");
    }

    #[test]
    fn a_step_of_the_binding_sequence_is_refused() {
        let mut b = binding(std(MOD_ALT, 0x77));
        b.shortcuts = vec![Shortcut::Sequence { steps: vec![Stroke { mods: MOD_ALT, vk: 0x77 }, Stroke { mods: 0, vk: 0x78 }] }];
        assert_eq!(validate(&b), Err("err.cancel_same"));
    }

    #[test]
    fn serde_default_and_round_trip() {
        let old = r#"{"id":1,"name":"x","enabled":true}"#;
        let b: Binding = serde_json::from_str(old).unwrap();
        assert_eq!(b.cancel_shortcut, None, "older files have no cancel shortcut");
        let with = binding(std(MOD_CTRL | MOD_SHIFT, 0x51));
        let json = serde_json::to_string(&with).unwrap();
        assert!(json.contains("\"cancel_shortcut\":{\"type\":\"standard\""), "{json}");
        assert_eq!(serde_json::from_str::<Binding>(&json).unwrap(), with);
    }
}
