//! Keyboard shortcut model: standard chords, simultaneous combos and sequences.
//! Pure logic (no Win32 calls) so it can be unit-tested.

use serde::{Deserialize, Serialize};

pub const MOD_CTRL: u8 = 1;
pub const MOD_ALT: u8 = 2;
pub const MOD_SHIFT: u8 = 4;
pub const MOD_WIN: u8 = 8;

pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_ESCAPE: u16 = 0x1B;

pub const MAX_COMBO_KEYS: usize = 6;
pub const MAX_SEQUENCE_STEPS: usize = 8;

/// One key press, optionally with modifiers held. `vk` may itself be a
/// modifier (a lone tap of Ctrl, for example) inside sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Stroke {
    pub mods: u8,
    pub vk: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Shortcut {
    /// Modifiers + one key. Registered with RegisterHotKey.
    Standard { mods: u8, vk: u16 },
    /// Two or more keys held together. Matched by the keyboard hook.
    Combo { keys: Vec<u16> },
    /// Keys pressed one after another. Matched by the keyboard hook.
    Sequence { steps: Vec<Stroke> },
}

/// Folds left/right variants into one code (LCtrl/RCtrl -> Ctrl, RWin -> LWin).
pub fn normalize_vk(vk: u16) -> u16 {
    match vk {
        0xA0 | 0xA1 => VK_SHIFT,
        0xA2 | 0xA3 => VK_CONTROL,
        0xA4 | 0xA5 => VK_MENU,
        0x5C => VK_LWIN,
        other => other,
    }
}

pub fn is_modifier(vk: u16) -> bool {
    mod_bit(vk) != 0
}

pub fn mod_bit(vk: u16) -> u8 {
    match normalize_vk(vk) {
        VK_CONTROL => MOD_CTRL,
        VK_MENU => MOD_ALT,
        VK_SHIFT => MOD_SHIFT,
        VK_LWIN => MOD_WIN,
        _ => 0,
    }
}

/// Display order of modifiers: Ctrl, Alt, Shift, Win.
fn modifier_rank(vk: u16) -> u8 {
    match normalize_vk(vk) {
        VK_CONTROL => 0,
        VK_MENU => 1,
        VK_SHIFT => 2,
        VK_LWIN => 3,
        _ => 4,
    }
}

pub fn key_name(vk: u16) -> String {
    let fixed = match vk {
        VK_CONTROL | 0xA2 | 0xA3 => "Ctrl",
        VK_MENU | 0xA4 | 0xA5 => "Alt",
        VK_SHIFT | 0xA0 | 0xA1 => "Shift",
        VK_LWIN | 0x5C => "Win",
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x13 => "Pause",
        0x03 => "Break",
        0x14 => "CapsLock",
        VK_ESCAPE => "Esc",
        0x20 => "Space",
        0x21 => "PgUp",
        0x22 => "PgDn",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2C => "PrtSc",
        0x2D => "Insert",
        0x2E => "Delete",
        0x5D => "Menu",
        0x6A => "Num *",
        0x6B => "Num +",
        0x6D => "Num -",
        0x6E => "Num .",
        0x6F => "Num /",
        0x90 => "NumLock",
        0x91 => "ScrollLock",
        0xAD => "Mute",
        0xAE => "Volume Down",
        0xAF => "Volume Up",
        0xB0 => "Next Track",
        0xB1 => "Prev Track",
        0xB3 => "Play/Pause",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => format!("Key {vk:#04X}"),
    }
}

fn mods_names(mods: u8) -> Vec<&'static str> {
    let mut out = Vec::new();
    if mods & MOD_CTRL != 0 {
        out.push("Ctrl");
    }
    if mods & MOD_ALT != 0 {
        out.push("Alt");
    }
    if mods & MOD_SHIFT != 0 {
        out.push("Shift");
    }
    if mods & MOD_WIN != 0 {
        out.push("Win");
    }
    out
}

pub fn stroke_label(s: &Stroke) -> String {
    let mut parts: Vec<String> = mods_names(s.mods).into_iter().map(String::from).collect();
    if !(is_modifier(s.vk) && s.mods & mod_bit(s.vk) != 0) {
        parts.push(key_name(s.vk));
    }
    parts.join(" + ")
}

/// Sorts keys for a combo: modifiers first (Ctrl, Alt, Shift, Win), then by code.
pub fn sort_combo_keys(keys: &[u16]) -> Vec<u16> {
    let mut out: Vec<u16> = keys.iter().map(|&k| normalize_vk(k)).collect();
    out.sort_by_key(|&k| (modifier_rank(k), k));
    out.dedup();
    out
}

/// Keys that produce text. Binding them without Ctrl/Alt/Win would break typing.
fn is_typing_key(vk: u16) -> bool {
    matches!(vk, 0x08 | 0x09 | 0x0D | 0x20 | 0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDE)
}

impl Shortcut {
    pub fn label(&self) -> String {
        match self {
            Shortcut::Standard { mods, vk } => stroke_label(&Stroke { mods: *mods, vk: *vk }),
            Shortcut::Combo { keys } => sort_combo_keys(keys)
                .iter()
                .map(|&k| key_name(k))
                .collect::<Vec<_>>()
                .join(" + "),
            Shortcut::Sequence { steps } => steps.iter().map(stroke_label).collect::<Vec<_>>().join(" → "),
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Shortcut::Standard { .. } => "Standard",
            Shortcut::Combo { .. } => "Simultaneous",
            Shortcut::Sequence { .. } => "Sequence",
        }
    }

    /// True when the shortcut needs the low-level keyboard hook.
    pub fn needs_hook(&self) -> bool {
        !matches!(self, Shortcut::Standard { .. })
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Shortcut::Standard { mods, vk } => {
                if is_modifier(*vk) {
                    return Err("A standard shortcut needs a non-modifier key.".into());
                }
                let strong = mods & (MOD_CTRL | MOD_ALT | MOD_WIN) != 0;
                if !strong && is_typing_key(*vk) {
                    return Err(format!(
                        "{} alone would block normal typing. Add Ctrl or Alt, or use F1–F24.",
                        self.label()
                    ));
                }
                Ok(())
            }
            Shortcut::Combo { keys } => {
                let keys = sort_combo_keys(keys);
                if keys.len() < 2 {
                    return Err("A simultaneous shortcut needs at least two keys.".into());
                }
                if keys.len() > MAX_COMBO_KEYS {
                    return Err(format!("Use at most {MAX_COMBO_KEYS} keys."));
                }
                if keys.iter().all(|&k| is_modifier(k)) {
                    return Err("Add at least one non-modifier key.".into());
                }
                Ok(())
            }
            Shortcut::Sequence { steps } => {
                if steps.len() < 2 {
                    return Err("A sequence needs at least two steps.".into());
                }
                if steps.len() > MAX_SEQUENCE_STEPS {
                    return Err(format!("Use at most {MAX_SEQUENCE_STEPS} steps."));
                }
                Ok(())
            }
        }
    }
}

/// Explains why two shortcuts would interfere, if they do.
pub fn conflict(a: &Shortcut, b: &Shortcut) -> Option<String> {
    use Shortcut::*;
    let norm = |s: &Shortcut| match s {
        Combo { keys } => Combo { keys: sort_combo_keys(keys) },
        other => other.clone(),
    };
    let (a, b) = (norm(a), norm(b));
    if a == b {
        return Some("uses the same shortcut".into());
    }
    match (&a, &b) {
        (Sequence { steps: s }, Standard { mods, vk }) | (Standard { mods, vk }, Sequence { steps: s }) => {
            let chord = Stroke { mods: *mods, vk: *vk };
            s.contains(&chord)
                .then(|| format!("{} is also a step of the sequence", stroke_label(&chord)))
        }
        (Sequence { steps: x }, Sequence { steps: y }) => {
            let (short, long) = if x.len() <= y.len() { (x, y) } else { (y, x) };
            long.starts_with(short).then(|| "one sequence is the beginning of the other".into())
        }
        (Combo { keys: x }, Combo { keys: y }) => {
            let (small, big) = if x.len() <= y.len() { (x, y) } else { (y, x) };
            small
                .iter()
                .all(|k| big.contains(k))
                .then(|| "one combination is part of the other".into())
        }
        (Combo { keys }, Standard { mods, vk }) | (Standard { mods, vk }, Combo { keys }) => {
            let mut needed = vec![*vk];
            for (bit, key) in [(MOD_CTRL, VK_CONTROL), (MOD_ALT, VK_MENU), (MOD_SHIFT, VK_SHIFT), (MOD_WIN, VK_LWIN)] {
                if mods & bit != 0 {
                    needed.push(key);
                }
            }
            needed
                .iter()
                .all(|k| keys.contains(k))
                .then(|| "the standard shortcut is part of the combination".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_readable() {
        assert_eq!(Shortcut::Standard { mods: MOD_CTRL | MOD_SHIFT, vk: 0x50 }.label(), "Ctrl + Shift + P");
        assert_eq!(Shortcut::Standard { mods: 0, vk: 0x77 }.label(), "F8");
        assert_eq!(Shortcut::Combo { keys: vec![0x53, 0x41, 0xA2] }.label(), "Ctrl + A + S");
        let seq = Shortcut::Sequence {
            steps: vec![Stroke { mods: 0, vk: VK_CONTROL }, Stroke { mods: 0, vk: 0x4B }, Stroke { mods: 0, vk: 0x43 }],
        };
        assert_eq!(seq.label(), "Ctrl → K → C");
    }

    #[test]
    fn standard_shortcut_rejects_plain_letters() {
        assert!(Shortcut::Standard { mods: 0, vk: 0x41 }.validate().is_err());
        assert!(Shortcut::Standard { mods: MOD_SHIFT, vk: 0x41 }.validate().is_err());
        assert!(Shortcut::Standard { mods: MOD_CTRL, vk: 0x41 }.validate().is_ok());
        assert!(Shortcut::Standard { mods: 0, vk: 0x77 }.validate().is_ok());
        assert!(Shortcut::Standard { mods: MOD_CTRL, vk: VK_CONTROL }.validate().is_err());
    }

    #[test]
    fn combo_and_sequence_limits() {
        assert!(Shortcut::Combo { keys: vec![0x41] }.validate().is_err());
        assert!(Shortcut::Combo { keys: vec![VK_CONTROL, VK_MENU] }.validate().is_err());
        assert!(Shortcut::Combo { keys: vec![0x41, 0x53, 0x44] }.validate().is_ok());
        assert!(Shortcut::Sequence { steps: vec![Stroke { mods: 0, vk: 0x41 }] }.validate().is_err());
    }

    #[test]
    fn left_and_right_modifiers_are_equal() {
        assert_eq!(normalize_vk(0xA3), VK_CONTROL);
        assert_eq!(normalize_vk(0x5C), VK_LWIN);
        assert_eq!(sort_combo_keys(&[0xA2, 0xA3, 0x41]), vec![VK_CONTROL, 0x41]);
    }

    #[test]
    fn detects_conflicts() {
        let f8 = Shortcut::Standard { mods: 0, vk: 0x77 };
        assert!(conflict(&f8, &f8.clone()).is_some());
        let as_combo = Shortcut::Combo { keys: vec![0x41, 0x53] };
        let asd_combo = Shortcut::Combo { keys: vec![0x44, 0x53, 0x41] };
        assert!(conflict(&as_combo, &asd_combo).is_some());
        assert!(conflict(&Shortcut::Combo { keys: vec![0x53, 0x41] }, &as_combo).is_some());
        let seq = Shortcut::Sequence { steps: vec![Stroke { mods: 0, vk: 0x77 }, Stroke { mods: 0, vk: 0x78 }] };
        assert!(conflict(&f8, &seq).is_some());
        let ctrl_q = Shortcut::Standard { mods: MOD_CTRL, vk: 0x51 };
        assert!(conflict(&ctrl_q, &as_combo).is_none());
        let ctrl_q_w = Shortcut::Combo { keys: vec![VK_CONTROL, 0x51, 0x57] };
        assert!(conflict(&ctrl_q, &ctrl_q_w).is_some());
    }
}
