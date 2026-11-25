//! Limits and checks for the "Type text" action (`Action::TypeText`).
//! Kept apart from `model.rs` so the model stays small; everything here is
//! pure and language-neutral (it also runs on worker threads, where the
//! UI's thread-local translations are not loaded).

use super::{Binding, TriggerMode};
use crate::shortcut::Shortcut;

/// Typing speed range in characters ("letters") per minute.
pub const TYPE_CPM_RANGE: (u32, u32) = (30, 30_000);
pub const DEFAULT_CPM: u32 = 300;
/// Longest text one action may type, in characters (Unicode scalar values).
pub const MAX_TYPE_CHARS: usize = 10_000;
/// Characters of the text shown in lists ("«first words…»").
const EXCERPT_CHARS: usize = 24;

pub(super) fn default_cpm() -> u32 {
    DEFAULT_CPM
}

/// Number of characters the text has once line breaks are normalized
/// ("\r\n" counts as one), the same count the editor's counter shows.
pub fn char_count(text: &str) -> usize {
    let crlf = text.matches("\r\n").count();
    text.chars().count() - crlf
}

/// "\r\n" and lone "\r" become "\n" (what the multi-line edit box returns
/// vs. what is stored in config.json).
pub fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Checks the text and speed. Errors are localization keys.
pub fn validate(text: &str, cpm: u32) -> Result<(), &'static str> {
    // Only spaces, line breaks or control characters: nothing visible to type.
    if !text.chars().any(|c| !c.is_whitespace() && !c.is_control()) {
        return Err("err.type_text_empty");
    }
    if char_count(text) > MAX_TYPE_CHARS {
        return Err("err.type_text_long");
    }
    if !(TYPE_CPM_RANGE.0..=TYPE_CPM_RANGE.1).contains(&cpm) {
        return Err("err.typing_speed");
    }
    Ok(())
}

/// Backspaces needed to erase what the binding's `fired`-th shortcut typed
/// into the focused app before the run started. A double tap typed it twice.
pub fn erase_count(b: &Binding, fired: usize) -> usize {
    let per_press = b.shortcuts.get(fired).map_or(0, Shortcut::typed_chars);
    if b.trigger.mode == TriggerMode::DoubleTap { per_press * 2 } else { per_press }
}

/// The first words of the text on one line, with "…" when cut. No quotes:
/// the UI adds language-appropriate ones.
pub fn excerpt(text: &str) -> String {
    // Lazy: reads only the first words, however long the text is.
    let mut out = String::new();
    let mut count = 0;
    for word in text.split_whitespace() {
        if count > 0 {
            out.push(' ');
            count += 1;
        }
        for c in word.chars() {
            if count >= EXCERPT_CHARS {
                return format!("{}…", out.trim_end());
            }
            out.push(c);
            count += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{Action, ActionKind, ALL_PROFILES};
    use super::*;

    fn typing(text: &str, cpm: u32) -> Action {
        Action::TypeText { text: text.into(), cpm, human: true, erase_trigger: true }
    }

    #[test]
    fn validates_text_and_speed() {
        assert_eq!(typing("", 300).validate(), Err("err.type_text_empty"));
        assert_eq!(typing("  \n\t ", 300).validate(), Err("err.type_text_empty"));
        assert_eq!(typing("\u{7}\u{1b}\r\n", 300).validate(), Err("err.type_text_empty"), "control-only text");
        assert!(typing("hi", 300).validate().is_ok());
        assert_eq!(typing("hi", 29).validate(), Err("err.typing_speed"));
        assert!(typing("hi", 30).validate().is_ok());
        assert!(typing("hi", 3000).validate().is_ok());
        assert!(typing("hi", 30_000).validate().is_ok());
        assert_eq!(typing("hi", 30_001).validate(), Err("err.typing_speed"));
    }

    #[test]
    fn length_limit_counts_characters_not_bytes() {
        let max = "я".repeat(MAX_TYPE_CHARS);
        assert!(typing(&max, 300).validate().is_ok(), "10 000 two-byte letters are fine");
        assert_eq!(typing(&format!("{max}a"), 300).validate(), Err("err.type_text_long"));
        let crlf = "a\r\n".repeat(MAX_TYPE_CHARS / 2);
        assert_eq!(char_count(&crlf), MAX_TYPE_CHARS, "CRLF counts as one line break");
        assert!(typing(&crlf, 300).validate().is_ok());
    }

    #[test]
    fn serde_round_trip_and_defaults() {
        let a = Action::TypeText { text: "Привет, мир!\n👋".into(), cpm: 450, human: false, erase_trigger: false };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"kind\":\"type_text\""), "{json}");
        assert_eq!(serde_json::from_str::<Action>(&json).unwrap(), a);
        let short: Action = serde_json::from_str(r#"{"kind":"type_text","text":"x"}"#).unwrap();
        let defaults = Action::TypeText { text: "x".into(), cpm: DEFAULT_CPM, human: true, erase_trigger: true };
        assert_eq!(short, defaults, "older files: human typing and erasing are on");
        let empty = Action::TypeText { text: String::new(), cpm: 300, human: true, erase_trigger: true };
        assert_eq!(ActionKind::TypeText.empty(), empty);
        assert!(ActionKind::ALL.contains(&ActionKind::TypeText));
        assert_eq!(a.kind(), ActionKind::TypeText);
    }

    #[test]
    fn repeat_modes_are_refused_for_typing() {
        let mut b = Binding::new(1, ALL_PROFILES);
        b.name = "x".into();
        b.actions = vec![typing("hello", 300)];
        assert!(b.validate().is_ok());
        for mode in [TriggerMode::Repeat, TriggerMode::Toggle] {
            b.trigger.mode = mode;
            assert_eq!(b.validate(), Err(("err.repeat_typing", None)));
        }
        b.trigger.mode = TriggerMode::LongPress;
        assert!(b.validate().is_ok());
    }

    #[test]
    fn erase_count_follows_the_fired_shortcut() {
        use crate::shortcut::{Stroke, MOD_CTRL};
        let mut b = Binding::new(1, ALL_PROFILES);
        let digits = Shortcut::Sequence { steps: (0x31..=0x33).map(|vk| Stroke { mods: 0, vk }).collect() };
        b.shortcuts = vec![Shortcut::Standard { mods: MOD_CTRL, vk: 0x51 }, digits, Shortcut::Combo { keys: vec![0x41, 0x53] }];
        assert_eq!(erase_count(&b, 0), 0, "standard: swallowed");
        assert_eq!(erase_count(&b, 1), 3);
        assert_eq!(erase_count(&b, 2), 2);
        assert_eq!(erase_count(&b, 7), 0, "no such shortcut");
        b.trigger.mode = TriggerMode::DoubleTap;
        assert_eq!(erase_count(&b, 1), 6, "typed twice");
    }

    #[test]
    fn excerpt_is_one_short_line() {
        assert_eq!(excerpt("Hello,\n  world"), "Hello, world");
        let long = excerpt("The quick brown fox jumps over the lazy dog");
        assert!(long.ends_with('…'));
        assert!(long.chars().count() <= EXCERPT_CHARS + 1);
        assert_eq!(excerpt(""), "");
        let word_at_limit = format!("{} tail", "w".repeat(EXCERPT_CHARS));
        assert_eq!(excerpt(&word_at_limit), format!("{}…", "w".repeat(EXCERPT_CHARS)), "a word ending at the limit");
        assert_eq!(excerpt(&"x".repeat(EXCERPT_CHARS)), "x".repeat(EXCERPT_CHARS), "exactly the limit: no ellipsis");
        assert_eq!(excerpt(&"y".repeat(EXCERPT_CHARS + 1)), format!("{}…", "y".repeat(EXCERPT_CHARS)));
        assert_eq!(normalize_newlines("a\r\nb\rc\nd"), "a\nb\nc\nd");
    }
}
