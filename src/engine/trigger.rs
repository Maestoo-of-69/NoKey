//! Trigger modes: turns "shortcut went down / up" events into what to do.
//! Pure state machine; timers and execution live in `app`.

use crate::config::TriggerMode;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Run the binding's actions once.
    Run(u32),
    /// Start the repeat timer (the first run is a separate `Run`).
    StartRepeat(u32),
    StopRepeat(u32),
    /// Start the long-press timer; `hold_elapsed` is called when it fires.
    StartHold(u32),
    CancelHold(u32),
}

#[derive(Default)]
struct State {
    held: bool,
    last_tap: Option<u64>,
    repeating: bool,
}

pub struct TriggerEngine {
    states: HashMap<u32, State>,
    pub double_tap_ms: u64,
}

impl TriggerEngine {
    pub fn new(double_tap_ms: u64) -> Self {
        Self { states: HashMap::new(), double_tap_ms }
    }

    /// The shortcut went down.
    pub fn down(&mut self, id: u32, mode: TriggerMode, now: u64) -> Vec<Effect> {
        let window = self.double_tap_ms;
        let s = self.states.entry(id).or_default();
        if s.held {
            return Vec::new(); // a second shortcut of the same binding, or key repeat
        }
        s.held = true;
        match mode {
            TriggerMode::Press => vec![Effect::Run(id)],
            TriggerMode::Release => Vec::new(),
            TriggerMode::DoubleTap => match s.last_tap {
                Some(t) if now.saturating_sub(t) <= window => {
                    s.last_tap = None;
                    vec![Effect::Run(id)]
                }
                _ => {
                    s.last_tap = Some(now);
                    Vec::new()
                }
            },
            TriggerMode::LongPress => vec![Effect::StartHold(id)],
            TriggerMode::Repeat => {
                s.repeating = true;
                vec![Effect::Run(id), Effect::StartRepeat(id)]
            }
            TriggerMode::Toggle => {
                if s.repeating {
                    s.repeating = false;
                    vec![Effect::StopRepeat(id)]
                } else {
                    s.repeating = true;
                    vec![Effect::Run(id), Effect::StartRepeat(id)]
                }
            }
        }
    }

    /// The shortcut was released.
    pub fn up(&mut self, id: u32, mode: TriggerMode) -> Vec<Effect> {
        let Some(s) = self.states.get_mut(&id) else { return Vec::new() };
        if !s.held {
            return Vec::new();
        }
        s.held = false;
        match mode {
            TriggerMode::Release => vec![Effect::Run(id)],
            TriggerMode::LongPress => vec![Effect::CancelHold(id)],
            TriggerMode::Repeat if s.repeating => {
                s.repeating = false;
                vec![Effect::StopRepeat(id)]
            }
            _ => Vec::new(),
        }
    }

    /// Down and up at once (RegisterHotKey reports presses only).
    pub fn tap(&mut self, id: u32, mode: TriggerMode, now: u64) -> Vec<Effect> {
        let mut out = self.down(id, mode, now);
        out.extend(self.up(id, mode));
        out
    }

    /// The long-press timer fired.
    pub fn hold_elapsed(&mut self, id: u32) -> Vec<Effect> {
        match self.states.get(&id) {
            Some(s) if s.held => vec![Effect::Run(id)],
            _ => Vec::new(),
        }
    }

    /// The repeat stopped by itself (limit reached, error, disabled).
    pub fn repeat_finished(&mut self, id: u32) {
        if let Some(s) = self.states.get_mut(&id) {
            s.repeating = false;
        }
    }

    /// Forgets everything (bindings changed or all disabled). Returns the
    /// timers that must be stopped.
    pub fn reset(&mut self) -> Vec<Effect> {
        let out = self
            .states
            .iter()
            .flat_map(|(&id, s)| {
                let mut v = Vec::new();
                if s.repeating {
                    v.push(Effect::StopRepeat(id));
                }
                if s.held {
                    v.push(Effect::CancelHold(id));
                }
                v
            })
            .collect();
        self.states.clear();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Effect::*;
    use TriggerMode::*;

    #[test]
    fn press_runs_once_per_press() {
        let mut e = TriggerEngine::new(300);
        assert_eq!(e.down(1, Press, 0), vec![Run(1)]);
        assert_eq!(e.down(1, Press, 10), vec![], "second shortcut / repeat while held");
        assert_eq!(e.up(1, Press), vec![]);
        assert_eq!(e.down(1, Press, 20), vec![Run(1)]);
    }

    #[test]
    fn release_runs_on_key_up() {
        let mut e = TriggerEngine::new(300);
        assert_eq!(e.down(1, Release, 0), vec![]);
        assert_eq!(e.up(1, Release), vec![Run(1)]);
        assert_eq!(e.up(1, Release), vec![], "no double run");
    }

    #[test]
    fn double_tap_needs_two_presses_in_window() {
        let mut e = TriggerEngine::new(300);
        assert_eq!(e.tap(1, DoubleTap, 0), vec![]);
        assert_eq!(e.tap(1, DoubleTap, 200), vec![Run(1)]);
        assert_eq!(e.tap(1, DoubleTap, 300), vec![], "the pair was consumed");
        assert_eq!(e.tap(1, DoubleTap, 1000), vec![], "too late: starts a new pair");
        assert_eq!(e.tap(1, DoubleTap, 1100), vec![Run(1)]);
    }

    #[test]
    fn long_press_runs_only_if_still_held() {
        let mut e = TriggerEngine::new(300);
        assert_eq!(e.down(1, LongPress, 0), vec![StartHold(1)]);
        assert_eq!(e.hold_elapsed(1), vec![Run(1)]);
        assert_eq!(e.up(1, LongPress), vec![CancelHold(1)]);
        e.down(1, LongPress, 1000);
        e.up(1, LongPress);
        assert_eq!(e.hold_elapsed(1), vec![], "released before the time");
    }

    #[test]
    fn repeat_runs_while_held() {
        let mut e = TriggerEngine::new(300);
        assert_eq!(e.down(1, Repeat, 0), vec![Run(1), StartRepeat(1)]);
        assert_eq!(e.up(1, Repeat), vec![StopRepeat(1)]);
    }

    #[test]
    fn toggle_starts_and_stops() {
        let mut e = TriggerEngine::new(300);
        assert_eq!(e.tap(1, Toggle, 0), vec![Run(1), StartRepeat(1)]);
        assert_eq!(e.tap(1, Toggle, 500), vec![StopRepeat(1)]);
        assert_eq!(e.tap(1, Toggle, 900), vec![Run(1), StartRepeat(1)]);
        e.repeat_finished(1);
        assert_eq!(e.tap(1, Toggle, 1500), vec![Run(1), StartRepeat(1)], "limit reached: next press starts again");
    }

    #[test]
    fn reset_stops_timers() {
        let mut e = TriggerEngine::new(300);
        e.tap(1, Toggle, 0);
        e.down(2, LongPress, 0);
        let mut stops = e.reset();
        stops.sort_by_key(|x| format!("{x:?}"));
        assert_eq!(stops, vec![CancelHold(2), StopRepeat(1)]);
        assert_eq!(e.tap(1, Toggle, 10), vec![Run(1), StartRepeat(1)]);
    }
}
