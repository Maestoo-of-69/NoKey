//! Per-binding cooldown: one press = one action, and a second press inside
//! the cooldown window is ignored (no queueing, no repeats).

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Cooldowns {
    until: HashMap<u32, Instant>,
}

impl Cooldowns {
    /// Starts the cooldown, or returns how long is left if one is running.
    pub fn try_start(&mut self, id: u32, length: Duration, now: Instant) -> Result<(), Duration> {
        if let Some(left) = self.remaining(id, now) {
            return Err(left);
        }
        self.until.insert(id, now + length);
        Ok(())
    }

    pub fn remaining(&self, id: u32, now: Instant) -> Option<Duration> {
        let end = *self.until.get(&id)?;
        (end > now).then(|| end - now)
    }

    /// Extends a running cooldown (used when Discord asks us to slow down).
    pub fn extend_to(&mut self, id: u32, end: Instant) {
        let slot = self.until.entry(id).or_insert(end);
        if end > *slot {
            *slot = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_second_press_inside_window() {
        let mut c = Cooldowns::default();
        let t0 = Instant::now();
        let len = Duration::from_secs(3);
        assert!(c.try_start(1, len, t0).is_ok());
        let left = c.try_start(1, len, t0 + Duration::from_millis(500)).unwrap_err();
        assert_eq!(left, Duration::from_millis(2500));
        assert!(c.try_start(2, len, t0).is_ok(), "other bindings are independent");
        assert!(c.try_start(1, len, t0 + Duration::from_secs(3)).is_ok());
    }

    #[test]
    fn extend_only_moves_forward() {
        let mut c = Cooldowns::default();
        let t0 = Instant::now();
        c.try_start(1, Duration::from_secs(3), t0).unwrap();
        c.extend_to(1, t0 + Duration::from_secs(1));
        assert_eq!(c.remaining(1, t0), Some(Duration::from_secs(3)));
        c.extend_to(1, t0 + Duration::from_secs(10));
        assert_eq!(c.remaining(1, t0), Some(Duration::from_secs(10)));
    }
}
