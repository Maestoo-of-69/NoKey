//! Matches shortcuts from a stream of key events (used by the keyboard
//! hook). Reports when a binding's shortcut goes down and when it is
//! released. Pure state machine: no timers, no polling — timeouts are
//! checked lazily against the timestamp of the next event.

use crate::shortcut::{is_modifier, mod_bit, normalize_vk, sort_combo_keys, Shortcut, Stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Down(u32),
    Up(u32),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Hit {
    pub events: Vec<Event>,
    /// The key must not reach other apps (standard shortcuts are consumed,
    /// like RegisterHotKey does).
    pub swallow: bool,
}

struct Chord {
    id: u32,
    priority: u8,
    mods: u8,
    vk: u16,
    latched: bool,
}

struct Combo {
    id: u32,
    priority: u8,
    keys: Vec<u16>,
    latched: bool,
}

struct Sequence {
    id: u32,
    priority: u8,
    steps: Vec<Stroke>,
    pos: usize,
    last: u64,
}

pub struct Matcher {
    chords: Vec<Chord>,
    combos: Vec<Combo>,
    sequences: Vec<Sequence>,
    /// Keys currently held, with the time each went down.
    down: Vec<(u16, u64)>,
    /// Keys whose events are being swallowed until released.
    swallowed: Vec<u16>,
    /// A completed sequence stays "down" until its last key is released.
    sequence_held: Vec<(u32, u16)>,
    combo_window_ms: u64,
    sequence_timeout_ms: u64,
}

impl Matcher {
    pub fn new(combo_window_ms: u64, sequence_timeout_ms: u64) -> Self {
        Self {
            chords: Vec::new(),
            combos: Vec::new(),
            sequences: Vec::new(),
            down: Vec::new(),
            swallowed: Vec::new(),
            sequence_held: Vec::new(),
            combo_window_ms,
            sequence_timeout_ms,
        }
    }

    pub fn add(&mut self, id: u32, priority: u8, shortcut: &Shortcut) {
        match shortcut {
            Shortcut::Standard { mods, vk } => {
                self.chords.push(Chord { id, priority, mods: *mods, vk: normalize_vk(*vk), latched: false })
            }
            Shortcut::Combo { keys } => {
                self.combos.push(Combo { id, priority, keys: sort_combo_keys(keys), latched: false })
            }
            Shortcut::Sequence { steps } => self.sequences.push(Sequence {
                id,
                priority,
                steps: steps.iter().map(|s| Stroke { mods: s.mods, vk: normalize_vk(s.vk) }).collect(),
                pos: 0,
                last: 0,
            }),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.chords.is_empty() && self.combos.is_empty() && self.sequences.is_empty()
    }

    /// Drops keys that are no longer physically held. A key-up can be lost
    /// (Win+L, UAC prompt, RDP switch), which would otherwise leave a key
    /// "stuck" and break later matches.
    pub fn forget_released(&mut self, still_down: impl Fn(u16) -> bool) -> Vec<Event> {
        self.down.retain(|&(k, _)| still_down(k));
        self.swallowed.retain(|&k| still_down(k));
        let down: Vec<u16> = self.down.iter().map(|&(k, _)| k).collect();
        let mut ups = Vec::new();
        for c in self.chords.iter_mut().filter(|c| c.latched && !down.contains(&c.vk)) {
            c.latched = false;
            ups.push(Event::Up(c.id));
        }
        for c in self.combos.iter_mut().filter(|c| c.latched && !c.keys.iter().all(|k| down.contains(k))) {
            c.latched = false;
            ups.push(Event::Up(c.id));
        }
        self.sequence_held.retain(|&(id, vk)| {
            let held = down.contains(&vk);
            if !held {
                ups.push(Event::Up(id));
            }
            held
        });
        ups
    }

    fn held_mods(&self) -> u8 {
        self.down.iter().fold(0, |m, &(k, _)| m | mod_bit(k))
    }

    /// Feeds one key event.
    pub fn key_event(&mut self, raw_vk: u16, is_down: bool, now: u64) -> Hit {
        let vk = normalize_vk(raw_vk);
        if !is_down {
            return self.key_up(vk);
        }
        if self.down.iter().any(|&(k, _)| k == vk) {
            // auto-repeat while the key is held
            return Hit { events: Vec::new(), swallow: self.swallowed.contains(&vk) };
        }
        let mods_before = self.held_mods();
        self.down.push((vk, now));

        let mut hit = Hit::default();
        if let Some(id) = self.match_pressed(vk, mods_before) {
            hit.events.push(Event::Down(id));
        }
        let seq = self.match_sequences(vk, mods_before, now);
        if hit.events.is_empty()
            && let Some(id) = seq
        {
            hit.events.push(Event::Down(id));
            self.sequence_held.push((id, vk));
        }
        if self.swallowed.contains(&vk) {
            hit.swallow = true;
        }
        hit
    }

    fn key_up(&mut self, vk: u16) -> Hit {
        self.down.retain(|&(k, _)| k != vk);
        let swallow = if let Some(i) = self.swallowed.iter().position(|&k| k == vk) {
            self.swallowed.swap_remove(i);
            true
        } else {
            false
        };
        let mut events = Vec::new();
        for c in self.chords.iter_mut().filter(|c| c.latched && c.vk == vk) {
            c.latched = false;
            events.push(Event::Up(c.id));
        }
        for c in self.combos.iter_mut().filter(|c| c.latched && c.keys.contains(&vk)) {
            c.latched = false;
            events.push(Event::Up(c.id));
        }
        self.sequence_held.retain(|&(id, k)| {
            let released = k == vk;
            if released {
                events.push(Event::Up(id));
            }
            !released
        });
        Hit { events, swallow }
    }

    /// Standard shortcuts and simultaneous combos completed by `vk`. The
    /// highest priority wins, then the shortcut with more keys.
    fn match_pressed(&mut self, vk: u16, mods_before: u8) -> Option<u32> {
        // (priority, key count, is_chord, index)
        let mut best: Option<(u8, usize, bool, usize)> = None;
        let better = |cand: (u8, usize, bool, usize), best: &Option<(u8, usize, bool, usize)>| {
            best.is_none_or(|b| (cand.0, cand.1) > (b.0, b.1))
        };
        if !is_modifier(vk) {
            for (i, c) in self.chords.iter().enumerate() {
                if !c.latched && c.vk == vk && c.mods == mods_before {
                    let cand = (c.priority, 1 + c.mods.count_ones() as usize, true, i);
                    if better(cand, &best) {
                        best = Some(cand);
                    }
                }
            }
        }
        let window = self.combo_window_ms;
        for (i, c) in self.combos.iter().enumerate() {
            if c.latched || !c.keys.contains(&vk) {
                continue;
            }
            let times: Option<Vec<u64>> =
                c.keys.iter().map(|k| self.down.iter().find(|(d, _)| d == k).map(|&(_, t)| t)).collect();
            let Some(t) = times else { continue };
            if t.iter().max().unwrap_or(&0) - t.iter().min().unwrap_or(&0) > window {
                continue;
            }
            let cand = (c.priority, c.keys.len(), false, i);
            if better(cand, &best) {
                best = Some(cand);
            }
        }
        let (_, _, is_chord, i) = best?;
        if is_chord {
            let c = &mut self.chords[i];
            c.latched = true;
            self.swallowed.push(vk);
            Some(c.id)
        } else {
            let c = &mut self.combos[i];
            c.latched = true;
            Some(c.id)
        }
    }

    fn match_sequences(&mut self, vk: u16, mods: u8, now: u64) -> Option<u32> {
        let stroke = if is_modifier(vk) { Stroke { mods: 0, vk } } else { Stroke { mods, vk } };
        let timeout = self.sequence_timeout_ms;
        let mut best: Option<(u8, u32)> = None;
        for s in &mut self.sequences {
            if s.pos > 0 && now.saturating_sub(s.last) > timeout {
                s.pos = 0;
            }
            if s.steps[s.pos] == stroke {
                s.pos += 1;
                s.last = now;
            } else if is_modifier(vk) {
                // A modifier may just be the start of a chord like Ctrl+K:
                // only restart if the sequence itself begins with this tap.
                if s.steps[0] == stroke {
                    s.pos = 1;
                    s.last = now;
                }
                continue;
            } else {
                s.pos = usize::from(s.steps[0] == stroke);
                s.last = now;
            }
            if s.pos == s.steps.len() {
                s.pos = 0;
                if best.is_none_or(|b| s.priority > b.0) {
                    best = Some((s.priority, s.id));
                }
            }
        }
        best.map(|b| b.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcut::{MOD_CTRL, VK_CONTROL};
    use Event::*;

    const A: u16 = 0x41;
    const S: u16 = 0x53;
    const D: u16 = 0x44;
    const K: u16 = 0x4B;
    const C: u16 = 0x43;
    const Q: u16 = 0x51;
    const E: u16 = 0x45;
    const F8: u16 = 0x77;

    fn st(vk: u16) -> Stroke {
        Stroke { mods: 0, vk }
    }

    fn downs(h: &Hit) -> Vec<u32> {
        h.events.iter().filter_map(|e| if let Down(id) = e { Some(*id) } else { None }).collect()
    }

    fn tap(m: &mut Matcher, vk: u16, t: u64) -> Vec<u32> {
        let r = downs(&m.key_event(vk, true, t));
        m.key_event(vk, false, t + 10);
        r
    }

    #[test]
    fn combo_down_and_up() {
        let mut m = Matcher::new(300, 1000);
        m.add(1, 0, &Shortcut::Combo { keys: vec![A, S] });
        assert!(m.key_event(A, true, 0).events.is_empty());
        assert_eq!(m.key_event(S, true, 50).events, vec![Down(1)]);
        assert!(m.key_event(S, true, 80).events.is_empty(), "auto-repeat");
        assert_eq!(m.key_event(S, false, 100).events, vec![Up(1)]);
        assert!(m.key_event(A, false, 110).events.is_empty());
        m.key_event(S, true, 200);
        assert_eq!(m.key_event(A, true, 220).events, vec![Down(1)]);
    }

    #[test]
    fn combo_requires_keys_within_window() {
        let mut m = Matcher::new(300, 1000);
        m.add(1, 0, &Shortcut::Combo { keys: vec![A, S] });
        m.key_event(A, true, 0);
        assert!(m.key_event(S, true, 900).events.is_empty());
    }

    #[test]
    fn standard_chord_is_matched_exactly_and_swallowed() {
        let mut m = Matcher::new(300, 1000);
        m.add(5, 0, &Shortcut::Standard { mods: MOD_CTRL, vk: Q });
        m.add(6, 0, &Shortcut::Standard { mods: 0, vk: F8 });
        m.key_event(0xA2, true, 0);
        let h = m.key_event(Q, true, 10);
        assert_eq!(h.events, vec![Down(5)]);
        assert!(h.swallow);
        assert!(m.key_event(Q, true, 20).swallow, "repeats are swallowed too");
        let up = m.key_event(Q, false, 30);
        assert_eq!(up.events, vec![Up(5)]);
        assert!(up.swallow);
        m.key_event(0xA2, false, 40);
        // F8 with Ctrl held is not plain F8
        m.key_event(0xA2, true, 50);
        assert!(m.key_event(F8, true, 60).events.is_empty());
        m.key_event(F8, false, 70);
        m.key_event(0xA2, false, 80);
        assert_eq!(tap(&mut m, F8, 100), vec![6]);
    }

    #[test]
    fn priority_beats_size() {
        let mut m = Matcher::new(300, 1000);
        m.add(1, 5, &Shortcut::Combo { keys: vec![A, S] });
        m.add(2, 0, &Shortcut::Combo { keys: vec![A, S, D] });
        m.key_event(A, true, 0);
        m.key_event(D, true, 5);
        assert_eq!(m.key_event(S, true, 10).events, vec![Down(1)]);
    }

    #[test]
    fn bigger_combo_wins_on_equal_priority() {
        let mut m = Matcher::new(300, 1000);
        m.add(1, 0, &Shortcut::Combo { keys: vec![A, S] });
        m.add(2, 0, &Shortcut::Combo { keys: vec![A, S, D] });
        m.key_event(A, true, 0);
        m.key_event(D, true, 5);
        assert_eq!(m.key_event(S, true, 10).events, vec![Down(2)]);
    }

    #[test]
    fn sequence_fires_in_order_and_releases() {
        let mut m = Matcher::new(300, 1000);
        m.add(3, 0, &Shortcut::Sequence { steps: vec![st(Q), st(Q), st(E)] });
        assert!(tap(&mut m, Q, 0).is_empty());
        assert!(tap(&mut m, Q, 100).is_empty());
        assert_eq!(downs(&m.key_event(E, true, 200)), vec![3]);
        assert_eq!(m.key_event(E, false, 250).events, vec![Up(3)]);
        assert!(tap(&mut m, Q, 300).is_empty());
        assert!(tap(&mut m, E, 400).is_empty());
    }

    #[test]
    fn sequence_times_out() {
        let mut m = Matcher::new(300, 1000);
        m.add(4, 0, &Shortcut::Sequence { steps: vec![st(A), st(D)] });
        tap(&mut m, A, 0);
        assert!(tap(&mut m, D, 5000).is_empty());
    }

    #[test]
    fn sequence_with_lone_modifier_and_chords() {
        let mut m = Matcher::new(300, 1000);
        m.add(5, 0, &Shortcut::Sequence { steps: vec![st(VK_CONTROL), st(K), st(C)] });
        m.add(6, 0, &Shortcut::Sequence { steps: vec![Stroke { mods: MOD_CTRL, vk: K }, st(C)] });
        assert!(tap(&mut m, 0xA2, 0).is_empty());
        assert!(tap(&mut m, K, 100).is_empty());
        assert_eq!(tap(&mut m, C, 200), vec![5]);
        m.key_event(0xA2, true, 1000);
        m.key_event(K, true, 1010);
        m.key_event(K, false, 1020);
        m.key_event(0xA2, false, 1030);
        assert_eq!(tap(&mut m, C, 1100), vec![6]);
    }

    #[test]
    fn one_event_fires_one_binding() {
        let mut m = Matcher::new(300, 1000);
        m.add(1, 0, &Shortcut::Combo { keys: vec![A, S] });
        m.add(2, 0, &Shortcut::Sequence { steps: vec![st(A), st(S)] });
        m.key_event(A, true, 0);
        assert_eq!(m.key_event(S, true, 10).events, vec![Down(1)]);
    }

    #[test]
    fn lost_key_up_does_not_stick() {
        let mut m = Matcher::new(300, 1000);
        m.add(1, 0, &Shortcut::Combo { keys: vec![A, S] });
        m.key_event(A, true, 0);
        assert_eq!(m.key_event(S, true, 10).events, vec![Down(1)]);
        // Both key-ups were lost (e.g. the PC was locked with keys held).
        assert_eq!(m.forget_released(|_| false), vec![Up(1)]);
        m.key_event(A, true, 5000);
        assert_eq!(m.key_event(S, true, 5010).events, vec![Down(1)], "combo must work again");
    }
}
