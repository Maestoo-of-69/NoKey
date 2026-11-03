//! Interface languages. Strings live in `locales/*.json` (flat key → text)
//! and are compiled into the exe, so NoKey stays a single portable file.
//! UI code never contains display text directly: it calls `t("key")`.

use crate::config::Language;
use std::cell::RefCell;
use std::collections::HashMap;

const EN: &str = include_str!("../../locales/en.json");
const RU: &str = include_str!("../../locales/ru.json");
const HE: &str = include_str!("../../locales/he.json");
const PAIN: &str = include_str!("../../locales/pain.json");

pub fn source(lang: Language) -> &'static str {
    match lang {
        Language::En => EN,
        Language::Ru => RU,
        Language::He => HE,
        Language::Pain => PAIN,
    }
}

fn parse(text: &str) -> HashMap<String, String> {
    serde_json::from_str(text).unwrap_or_default()
}

struct Strings {
    lang: Language,
    current: HashMap<String, String>,
    fallback: HashMap<String, String>,
}

thread_local! {
    static STRINGS: RefCell<Option<Strings>> = const { RefCell::new(None) };
}

/// Switches the interface language (UI thread).
pub fn set_language(lang: Language) {
    STRINGS.with(|s| {
        *s.borrow_mut() = Some(Strings { lang, current: parse(source(lang)), fallback: parse(EN) });
    });
}

pub fn language() -> Language {
    STRINGS.with(|s| s.borrow().as_ref().map(|x| x.lang).unwrap_or_default())
}

pub fn is_rtl() -> bool {
    language().is_rtl()
}

/// Translated text for `key`; English if missing; the key itself as a last resort.
pub fn t(key: &str) -> String {
    let loaded = STRINGS.with(|s| s.borrow().is_some());
    if !loaded {
        set_language(Language::En);
    }
    STRINGS.with(|s| {
        let s = s.borrow();
        let s = s.as_ref().expect("strings loaded");
        s.current.get(key).or_else(|| s.fallback.get(key)).cloned().unwrap_or_else(|| key.to_string())
    })
}

/// `t` with `{name}` placeholders filled in.
pub fn tf(key: &str, args: &[(&str, &str)]) -> String {
    let mut out = t(key);
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// The Windows display language mapped to a NoKey language (English otherwise).
pub fn system_language() -> Language {
    const LANG_RUSSIAN: u16 = 0x19;
    const LANG_HEBREW: u16 = 0x0D;
    let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    match id & 0x3FF {
        LANG_RUSSIAN => Language::Ru,
        LANG_HEBREW => Language::He,
        _ => Language::En,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn keys(lang: Language) -> BTreeSet<String> {
        let map: HashMap<String, String> =
            serde_json::from_str(source(lang)).unwrap_or_else(|e| panic!("{lang:?} locale is not valid JSON: {e}"));
        for (k, v) in &map {
            assert!(!v.trim().is_empty(), "{lang:?}: empty text for {k}");
        }
        map.into_keys().collect()
    }

    #[test]
    fn all_locales_have_the_same_keys() {
        let en = keys(Language::En);
        for lang in [Language::Ru, Language::He, Language::Pain] {
            let other = keys(lang);
            let missing: Vec<_> = en.difference(&other).collect();
            let extra: Vec<_> = other.difference(&en).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{lang:?}: missing {missing:?}, extra {extra:?}");
        }
    }

    /// Every `t("…")` / `tf("…"` key and every `"err.…"`-style key literal
    /// used in the source exists in en.json.
    #[test]
    fn every_key_used_in_code_exists() {
        let en = keys(Language::En);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let prefixes = [
            "err.", "main.", "editor.", "settings.", "status.", "action.", "trigger.", "theme.", "tray.", "tip.",
            "sort.", "common.", "field.", "hint.", "update.", "discord.", "profile.", "notify.",
        ];
        let mut missing = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for (i, _) in text.match_indices('"') {
                    let rest = &text[i + 1..];
                    let Some(end) = rest.find('"') else { continue };
                    let lit = &rest[..end];
                    let looks_like_key = prefixes.iter().any(|p| lit.starts_with(p))
                        && lit.len() > 4
                        && !lit.ends_with(".com") // host names such as "discord.com"
                        && !lit.ends_with('.') // the prefix list above
                        && lit.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_');
                    if looks_like_key && !en.contains(lit) {
                        missing.push(format!("{}: {lit}", path.file_name().unwrap().to_string_lossy()));
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "keys missing from en.json: {missing:#?}");
    }

    /// Pain Language must be its own (humorous) text, not a copy of Russian.
    #[test]
    fn pain_language_is_not_plain_russian() {
        let ru: HashMap<String, String> = serde_json::from_str(source(Language::Ru)).unwrap();
        let pain: HashMap<String, String> = serde_json::from_str(source(Language::Pain)).unwrap();
        let same = ru.iter().filter(|(k, v)| pain.get(*k) == Some(v) && v.chars().filter(|c| c.is_alphabetic()).count() > 3).count();
        assert!(same * 10 < ru.len(), "{same} of {} Pain strings equal Russian", ru.len());
        assert!(pain.values().all(|v| !v.contains('\u{FFFD}')), "no broken encoding");
        let cyrillic = pain.values().filter(|v| v.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))).count();
        assert!(cyrillic * 10 >= pain.len() * 9, "Pain Language stays Cyrillic");
    }

    #[test]
    fn hebrew_is_hebrew_and_rtl() {
        assert!(Language::He.is_rtl());
        assert!(!Language::Ru.is_rtl());
        let he: HashMap<String, String> = serde_json::from_str(source(Language::He)).unwrap();
        let hebrew = he.values().filter(|v| v.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c))).count();
        assert!(hebrew * 10 >= he.len() * 8, "most Hebrew strings must contain Hebrew letters");
    }

    #[test]
    fn placeholders_are_filled() {
        set_language(Language::En);
        assert!(!tf("status.profile_switched", &[("name", "Gaming")]).contains("{name}"));
        assert_eq!(t("no.such.key"), "no.such.key");
    }
}
