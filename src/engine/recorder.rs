//! Records a shortcut from key events while the user is in "Record" mode.

use crate::shortcut::{
    is_modifier, mod_bit, normalize_vk, sort_combo_keys, Shortcut, Stroke, MAX_COMBO_KEYS, MAX_SEQUENCE_STEPS,
    VK_ESCAPE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordMode {
    Standard,
    Combo,
    Sequence,
}

impl RecordMode {
    pub fn label(self) -> &'static str {
        match self {
            RecordMode::Standard => "Standard",
            RecordMode::Combo => "Simultaneous",
            RecordMode::Sequence => "Sequence",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordUpdate {
    Progress(String),
    Done(Shortcut),
    Cancelled,
    Rejected(String),
}

pub struct Recorder {
    mode: RecordMode,
    down: Vec<u16>,
    pressed: Vec<u16>,
    steps: Vec<Stroke>,
    lone_modifier: Option<u16>,
}

impl Recorder {
    pub fn new(mode: RecordMode) -> Self {
        Self { mode, down: Vec::new(), pressed: Vec::new(), steps: Vec::new(), lone_modifier: None }
    }

    fn mods(&self) -> u8 {
        self.down.iter().fold(0, |m, &k| m | mod_bit(k))
    }

    pub fn key_event(&mut self, raw_vk: u16, is_down: bool) -> Option<RecordUpdate> {
        let vk = normalize_vk(raw_vk);
        let repeat = is_down && self.down.contains(&vk);
        if repeat {
            return None;
        }
        match self.mode {
            RecordMode::Standard => self.standard(vk, is_down),
            RecordMode::Combo => self.combo(vk, is_down),
            RecordMode::Sequence => self.sequence(vk, is_down),
        }
    }

    fn standard(&mut self, vk: u16, is_down: bool) -> Option<RecordUpdate> {
        if !is_down {
            self.down.retain(|&k| k != vk);
            return None;
        }
        let mods = self.mods();
        if is_modifier(vk) {
            self.down.push(vk);
            return Some(RecordUpdate::Progress(format!("{} + …", Shortcut::Standard { mods: self.mods(), vk }.label())));
        }
        if vk == VK_ESCAPE && mods == 0 {
            return Some(RecordUpdate::Cancelled);
        }
        Some(RecordUpdate::Done(Shortcut::Standard { mods, vk }))
    }

    fn combo(&mut self, vk: u16, is_down: bool) -> Option<RecordUpdate> {
        if is_down {
            self.down.push(vk);
            if !self.pressed.contains(&vk) {
                self.pressed.push(vk);
            }
            if self.pressed.len() > MAX_COMBO_KEYS {
                self.reset();
                return Some(RecordUpdate::Rejected(format!("Use at most {MAX_COMBO_KEYS} keys.")));
            }
            return Some(RecordUpdate::Progress(Shortcut::Combo { keys: self.pressed.clone() }.label()));
        }
        self.down.retain(|&k| k != vk);
        if !self.down.is_empty() || self.pressed.is_empty() {
            return None;
        }
        let keys = sort_combo_keys(&self.pressed);
        self.reset();
        let plain: Vec<u16> = keys.iter().copied().filter(|&k| !is_modifier(k)).collect();
        match plain.len() {
            0 => Some(RecordUpdate::Rejected("Add at least one non-modifier key.".into())),
            1 if keys.len() == 1 => Some(RecordUpdate::Rejected("Press at least two keys together.".into())),
            1 => {
                // Modifiers + one key is a standard shortcut; RegisterHotKey handles it best.
                let mods = keys.iter().fold(0, |m, &k| m | mod_bit(k));
                Some(RecordUpdate::Done(Shortcut::Standard { mods, vk: plain[0] }))
            }
            _ => Some(RecordUpdate::Done(Shortcut::Combo { keys })),
        }
    }

    fn sequence(&mut self, vk: u16, is_down: bool) -> Option<RecordUpdate> {
        if is_down {
            if is_modifier(vk) {
                self.down.push(vk);
                self.lone_modifier = Some(vk);
                return None;
            }
            let stroke = Stroke { mods: self.mods(), vk };
            self.lone_modifier = None;
            return self.push_step(stroke);
        }
        self.down.retain(|&k| k != vk);
        if is_modifier(vk) && self.lone_modifier == Some(vk) {
            self.lone_modifier = None;
            return self.push_step(Stroke { mods: 0, vk });
        }
        None
    }

    fn push_step(&mut self, stroke: Stroke) -> Option<RecordUpdate> {
        self.steps.push(stroke);
        if self.steps.len() >= MAX_SEQUENCE_STEPS {
            return Some(self.finish());
        }
        Some(RecordUpdate::Progress(format!("{} → …", Shortcut::Sequence { steps: self.steps.clone() }.label())))
    }

    /// Called when the user clicks "Stop" (needed for sequences).
    pub fn finish(&mut self) -> RecordUpdate {
        let steps = std::mem::take(&mut self.steps);
        self.reset();
        match steps.len() {
            0 => RecordUpdate::Cancelled,
            1 => RecordUpdate::Rejected("A sequence needs at least two steps.".into()),
            _ => RecordUpdate::Done(Shortcut::Sequence { steps }),
        }
    }

    fn reset(&mut self) {
        self.down.clear();
        self.pressed.clear();
        self.steps.clear();
        self.lone_modifier = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcut::{MOD_CTRL, MOD_SHIFT, VK_CONTROL};

    #[test]
    fn records_standard_with_modifiers() {
        let mut r = Recorder::new(RecordMode::Standard);
        assert!(matches!(r.key_event(0xA2, true), Some(RecordUpdate::Progress(_))));
        r.key_event(0xA0, true);
        assert_eq!(r.key_event(0x50, true), Some(RecordUpdate::Done(Shortcut::Standard { mods: MOD_CTRL | MOD_SHIFT, vk: 0x50 })));
    }

    #[test]
    fn escape_cancels_standard() {
        let mut r = Recorder::new(RecordMode::Standard);
        assert_eq!(r.key_event(VK_ESCAPE, true), Some(RecordUpdate::Cancelled));
    }

    #[test]
    fn records_combo_on_release() {
        let mut r = Recorder::new(RecordMode::Combo);
        r.key_event(0x41, true);
        r.key_event(0x53, true);
        r.key_event(0x44, true);
        r.key_event(0x41, false);
        r.key_event(0x53, false);
        assert_eq!(r.key_event(0x44, false), Some(RecordUpdate::Done(Shortcut::Combo { keys: vec![0x41, 0x44, 0x53] })));
    }

    #[test]
    fn combo_with_one_key_becomes_standard() {
        let mut r = Recorder::new(RecordMode::Combo);
        r.key_event(VK_CONTROL, true);
        r.key_event(0x51, true);
        r.key_event(0x51, false);
        assert_eq!(r.key_event(VK_CONTROL, false), Some(RecordUpdate::Done(Shortcut::Standard { mods: MOD_CTRL, vk: 0x51 })));
    }

    #[test]
    fn records_sequence_with_modifier_tap() {
        let mut r = Recorder::new(RecordMode::Sequence);
        r.key_event(0xA2, true);
        r.key_event(0xA2, false);
        r.key_event(0x4B, true);
        r.key_event(0x4B, false);
        r.key_event(0x43, true);
        let done = r.finish();
        assert_eq!(
            done,
            RecordUpdate::Done(Shortcut::Sequence {
                steps: vec![Stroke { mods: 0, vk: VK_CONTROL }, Stroke { mods: 0, vk: 0x4B }, Stroke { mods: 0, vk: 0x43 }]
            })
        );
    }

    #[test]
    fn single_step_sequence_is_rejected() {
        let mut r = Recorder::new(RecordMode::Sequence);
        r.key_event(0x41, true);
        assert!(matches!(r.finish(), RecordUpdate::Rejected(_)));
    }
}
