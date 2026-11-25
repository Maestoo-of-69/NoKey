//! "Type text" engine. The loop is written against the small `Keyboard`
//! trait so cancel, focus loss, blocked input and timing are tested with a
//! fake keyboard; the real one (SendInput, Win32) is `win::typer`.
//!
//! It runs inline on the binding's worker thread (each run gets its own
//! thread from `App::spawn`), so a TypeText step finishes, or is
//! cancelled, before the next step starts, and the UI thread never waits.

pub mod plan;

use plan::{stroke, Stroke};
use std::sync::atomic::{AtomicBool, Ordering};

/// Why typing stopped early (shown as an error).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeFail {
    /// Another binding is typing right now.
    Busy,
    /// Ctrl / Alt / Shift / Win stayed held after the shortcut.
    Modifiers,
    /// NoKey's own window is in front (e.g. the Run button was used).
    OwnWindow,
    /// An administrator window is in front; Windows (UIPI) would drop the keys.
    Elevated,
    /// SendInput inserted fewer events than asked (UIPI, secure desktop).
    Blocked,
    /// The user switched to another window.
    FocusLost,
    /// No window is in front (lock screen, a switch in progress).
    NoWindow,
}

/// How a typing job ended when it did not finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// Esc, the same shortcut again, the emergency switch or exit.
    Cancelled,
    Failed(TypeFail),
}

/// The keyboard as the typing loop sees it.
pub trait Keyboard {
    /// Sends keystrokes (down + up each) in one go. False = Windows dropped
    /// some events.
    fn send(&mut self, strokes: &[Stroke]) -> bool;
    /// Cancel flag set or Esc pressed.
    fn stop_requested(&mut self) -> bool;
    fn modifiers_held(&mut self) -> bool;
    /// Checks the target once modifiers are up and remembers it.
    fn begin(&mut self) -> Result<(), TypeFail>;
    /// The window remembered by `begin` is still in front.
    fn same_window(&mut self) -> bool;
    /// Identifies the window remembered by `begin`.
    fn target_window(&self) -> isize;
    fn now_ms(&mut self) -> u64;
    fn sleep(&mut self, ms: u32);
}

/// Longest sleep between two checks of the cancel flag.
pub const SLICE_MS: u32 = 20;
/// How long the shortcut's own modifiers may stay held before giving up.
pub const MODIFIER_WAIT_MS: u64 = 1500;
/// After the user releases a modifier mid-text, wait this long before going on.
const RESUME_MS: u64 = 150;
/// A modifier held this long mid-text (not a quick shortcut) ends typing.
pub const MAX_HOLD_MS: u64 = 10_000;
/// At most this much backlog is caught up in a burst after a stall.
const MAX_LAG_MS: u64 = 100;
/// Most characters sent in one batch.
pub const BATCH_CAP: usize = 64;

/// The shortcut's own characters, to be removed before typing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Erase {
    /// Backspaces to send.
    pub count: usize,
    /// Window that was in front when the shortcut fired; nothing is erased
    /// in any other window.
    pub window: isize,
}

/// How many characters are due at `now` when the first one was due at
/// `deadline` (at least one, at most `BATCH_CAP`), and the deadline of the
/// character after them. `delays[i]` is the wait after character `i`.
pub fn due(delays: &[u32], deadline: u64, now: u64) -> (usize, u64) {
    let (mut count, mut next) = (0, deadline);
    for &delay in delays.iter().take(BATCH_CAP) {
        if count > 0 && next > now {
            break;
        }
        next += delay as u64;
        count += 1;
    }
    (count, next)
}

/// Sleeps in slices until `ms` passed; false when a stop was requested.
fn nap<K: Keyboard>(kb: &mut K, ms: u64) -> bool {
    let end = kb.now_ms() + ms;
    loop {
        if kb.stop_requested() {
            return false;
        }
        let now = kb.now_ms();
        if now >= end {
            return true;
        }
        kb.sleep((end - now).min(SLICE_MS as u64) as u32);
    }
}

/// Waits until no modifier is held (at most `MODIFIER_WAIT_MS`).
fn wait_for_modifiers<K: Keyboard>(kb: &mut K) -> Result<(), Stop> {
    let end = kb.now_ms() + MODIFIER_WAIT_MS;
    while kb.modifiers_held() {
        if kb.now_ms() >= end {
            return Err(Stop::Failed(TypeFail::Modifiers));
        }
        if !nap(kb, SLICE_MS as u64) {
            return Err(Stop::Cancelled);
        }
    }
    Ok(())
}

/// Waits until `deadline`. A modifier pressed mid-text (e.g. the start of
/// Ctrl+Alt+Pause) pauses typing so letters never become shortcuts.
fn wait_until<K: Keyboard>(kb: &mut K, deadline: &mut u64) -> Result<(), Stop> {
    let mut held_since: Option<u64> = None;
    loop {
        if kb.stop_requested() {
            return Err(Stop::Cancelled);
        }
        if !kb.same_window() {
            return Err(Stop::Failed(TypeFail::FocusLost));
        }
        let now = kb.now_ms();
        if kb.modifiers_held() {
            let since = *held_since.get_or_insert(now);
            if now - since >= MAX_HOLD_MS {
                return Err(Stop::Failed(TypeFail::Modifiers));
            }
            *deadline = (*deadline).max(now + RESUME_MS);
            kb.sleep(SLICE_MS);
            continue;
        }
        held_since = None;
        if now >= *deadline {
            return Ok(());
        }
        kb.sleep((*deadline - now).min(SLICE_MS as u64) as u32);
    }
}

/// Erases the shortcut's characters (see `Erase`), then types `chars`,
/// waiting `delays[i]` ms after character `i`. Times are absolute deadlines
/// and every character already due goes out in one batch, so speeds finer
/// than the coarse Windows timer are reached.
pub fn type_chars<K: Keyboard>(kb: &mut K, chars: &[char], delays: &[u32], erase: Erase) -> Result<(), Stop> {
    wait_for_modifiers(kb)?;
    if kb.stop_requested() {
        return Err(Stop::Cancelled);
    }
    kb.begin().map_err(Stop::Failed)?;
    if erase.count > 0 && kb.target_window() == erase.window {
        send(kb, &vec![Stroke::Backspace; erase.count])?;
    }
    let delays: Vec<u32> = (0..chars.len()).map(|i| delays.get(i).copied().unwrap_or(0)).collect();
    let mut deadline = kb.now_ms();
    let mut done = 0;
    while done < chars.len() {
        wait_until(kb, &mut deadline)?;
        let now = kb.now_ms();
        let (count, next) = due(&delays[done..], deadline.max(now.saturating_sub(MAX_LAG_MS)), now);
        let batch: Vec<Stroke> = chars[done..done + count].iter().map(|&c| stroke(c)).collect();
        send(kb, &batch)?;
        deadline = next;
        done += count;
    }
    Ok(())
}

fn send<K: Keyboard>(kb: &mut K, strokes: &[Stroke]) -> Result<(), Stop> {
    if kb.send(strokes) { Ok(()) } else { Err(Stop::Failed(TypeFail::Blocked)) }
}

static TYPING: AtomicBool = AtomicBool::new(false);

/// True while some binding is typing.
pub fn is_busy() -> bool {
    TYPING.load(Ordering::Acquire)
}

/// Held while a binding types; only one typing job runs at a time.
pub struct TypingJob(());

impl TypingJob {
    pub fn try_begin() -> Result<TypingJob, TypeFail> {
        TYPING.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).map_err(|_| TypeFail::Busy)?;
        Ok(TypingJob(()))
    }
}

impl Drop for TypingJob {
    fn drop(&mut self) {
        TYPING.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: isize = 7;
    const KEEP: Erase = Erase { count: 0, window: 0 };

    /// Scripted keyboard with a virtual clock.
    #[derive(Default)]
    struct Fake {
        now: u64,
        sent: Vec<Stroke>,
        sent_at: Vec<u64>,
        /// Size of each batch.
        batches: Vec<usize>,
        stop_at: Option<u64>,
        mods_until: u64,
        /// Modifier held from .0 until .1 (mid-text).
        mods_from: Option<(u64, u64)>,
        window_lost_at: Option<u64>,
        block_after: Option<usize>,
        begin_err: Option<TypeFail>,
        longest_sleep: u32,
    }

    impl Keyboard for Fake {
        fn send(&mut self, strokes: &[Stroke]) -> bool {
            if self.block_after.is_some_and(|n| self.sent.len() >= n) {
                return false;
            }
            self.batches.push(strokes.len());
            for &s in strokes {
                self.sent.push(s);
                self.sent_at.push(self.now);
            }
            true
        }
        fn target_window(&self) -> isize {
            WINDOW
        }
        fn stop_requested(&mut self) -> bool {
            self.stop_at.is_some_and(|t| self.now >= t)
        }
        fn modifiers_held(&mut self) -> bool {
            self.now < self.mods_until || self.mods_from.is_some_and(|(a, b)| (a..b).contains(&self.now))
        }
        fn begin(&mut self) -> Result<(), TypeFail> {
            self.begin_err.map_or(Ok(()), Err)
        }
        fn same_window(&mut self) -> bool {
            self.window_lost_at.is_none_or(|t| self.now < t)
        }
        fn now_ms(&mut self) -> u64 {
            self.now
        }
        fn sleep(&mut self, ms: u32) {
            self.longest_sleep = self.longest_sleep.max(ms);
            // Windows sleeps are coarse: model a 16 ms tick.
            self.now += (ms as u64).max(1).div_ceil(16) * 16;
        }
    }

    fn chars(s: &str) -> Vec<char> {
        plan::prepare(s)
    }

    #[test]
    fn types_everything_and_keeps_the_average_speed() {
        let text = chars(&"abc ".repeat(50));
        let delays = vec![20u32; text.len()]; // 3000 cpm, finer than the 16 ms tick
        let mut kb = Fake::default();
        assert_eq!(type_chars(&mut kb, &text, &delays, KEEP), Ok(()));
        assert_eq!(kb.sent.len(), text.len());
        let span = kb.sent_at.last().unwrap() - kb.sent_at[0];
        let expected = 20 * (text.len() as u64 - 1);
        assert!(span.abs_diff(expected) <= 40, "deadlines keep the pace: {span} vs {expected}");
        assert!(kb.longest_sleep <= SLICE_MS, "never sleeps past the cancel check");
    }

    #[test]
    fn cancel_stops_between_characters_quickly() {
        let text = chars(&"x".repeat(100));
        let delays = vec![2000u32; text.len()]; // 30 cpm
        let mut kb = Fake { stop_at: Some(5000), ..Fake::default() };
        assert_eq!(type_chars(&mut kb, &text, &delays, KEEP), Err(Stop::Cancelled));
        assert!(kb.now - 5000 <= 32, "noticed within a slice: stopped at {}", kb.now);
        assert_eq!(kb.sent.len(), 3);
    }

    #[test]
    fn waits_for_the_shortcut_modifiers_then_gives_up() {
        let text = chars("hi");
        let mut kb = Fake { mods_until: 400, ..Fake::default() };
        assert_eq!(type_chars(&mut kb, &text, &[10, 10], KEEP), Ok(()));
        assert!(kb.sent_at[0] >= 400, "first key only after release");
        let mut stuck = Fake { mods_until: u64::MAX, ..Fake::default() };
        assert_eq!(type_chars(&mut stuck, &text, &[10, 10], KEEP), Err(Stop::Failed(TypeFail::Modifiers)));
        assert!(stuck.sent.is_empty());
    }

    #[test]
    fn a_modifier_held_mid_text_pauses_then_gives_up() {
        let text = chars(&"z".repeat(10));
        let delays = vec![100u32; text.len()];
        // Held briefly mid-text: typing waits, then finishes.
        let mut brief = Fake { mods_from: Some((250, 600)), ..Fake::default() };
        assert_eq!(type_chars(&mut brief, &text, &delays, KEEP), Ok(()));
        assert_eq!(brief.sent.len(), 10);
        assert!(!brief.sent_at.iter().any(|&t| (250..600).contains(&t)), "nothing typed while held");
        // Held for good: give up after MAX_HOLD_MS.
        let mut stuck = Fake { mods_from: Some((250, u64::MAX)), ..Fake::default() };
        assert_eq!(type_chars(&mut stuck, &text, &delays, KEEP), Err(Stop::Failed(TypeFail::Modifiers)));
        assert!(stuck.now >= 250 + MAX_HOLD_MS && stuck.now < 250 + MAX_HOLD_MS + 100);
    }

    #[test]
    fn focus_change_and_blocked_input_fail() {
        let text = chars(&"y".repeat(20));
        let delays = vec![100u32; text.len()];
        let mut kb = Fake { window_lost_at: Some(450), ..Fake::default() };
        assert_eq!(type_chars(&mut kb, &text, &delays, KEEP), Err(Stop::Failed(TypeFail::FocusLost)));
        assert!(kb.sent.len() <= 6);
        let mut uipi = Fake { block_after: Some(0), ..Fake::default() };
        assert_eq!(type_chars(&mut uipi, &text, &delays, KEEP), Err(Stop::Failed(TypeFail::Blocked)));
        let mut elevated = Fake { begin_err: Some(TypeFail::Elevated), ..Fake::default() };
        assert_eq!(type_chars(&mut elevated, &text, &delays, KEEP), Err(Stop::Failed(TypeFail::Elevated)));
        assert!(elevated.sent.is_empty());
    }

    #[test]
    fn crlf_is_one_enter_and_emoji_one_stroke() {
        let text = chars("a\r\n👋\tb");
        let mut kb = Fake::default();
        assert_eq!(type_chars(&mut kb, &text, &[0; 5], KEEP), Ok(()));
        assert_eq!(kb.sent.len(), 5);
        assert_eq!(kb.sent[1], Stroke::Enter);
        assert_eq!(kb.sent[2], Stroke::Unicode { units: [0xD83D, 0xDC4B], len: 2 });
        assert_eq!(kb.sent[3], Stroke::Tab);
    }

    #[test]
    fn due_batches_every_late_character_up_to_the_cap() {
        let delays = [2u32; 200];
        assert_eq!(due(&delays, 100, 100), (1, 102), "only the first is due");
        assert_eq!(due(&delays, 100, 115), (8, 116), "100, 102 … 114");
        assert_eq!(due(&delays, 100, 116), (9, 118), "due exactly now counts");
        assert_eq!(due(&delays, 100, 10_000), (BATCH_CAP, 100 + 2 * BATCH_CAP as u64));
        assert_eq!(due(&delays[..3], 100, 10_000), (3, 106), "end of the text");
        assert_eq!(due(&[], 100, 100), (0, 100));
        assert_eq!(due(&[500, 500], 100, 100), (1, 600), "slow typing stays one at a time");
    }

    #[test]
    fn top_speed_is_reached_despite_the_coarse_timer() {
        let text = chars(&"q".repeat(1000));
        let delays = vec![2u32; text.len()]; // 30 000 cpm
        let mut kb = Fake::default();
        assert_eq!(type_chars(&mut kb, &text, &delays, KEEP), Ok(()));
        assert_eq!(kb.sent.len(), 1000);
        let span = kb.sent_at.last().unwrap() - kb.sent_at[0];
        assert!(span.abs_diff(1998) <= 32, "span {span}");
        assert!(kb.batches.iter().all(|&n| n <= BATCH_CAP));
        assert!(kb.batches.iter().any(|&n| n > 1), "batched");
    }

    #[test]
    fn cancel_and_focus_are_checked_between_batches() {
        let text = chars(&"q".repeat(1000));
        let delays = vec![2u32; text.len()];
        let mut stopped = Fake { stop_at: Some(200), ..Fake::default() };
        assert_eq!(type_chars(&mut stopped, &text, &delays, KEEP), Err(Stop::Cancelled));
        assert!(stopped.sent.len() < 130, "{}", stopped.sent.len());
        let mut lost = Fake { window_lost_at: Some(200), ..Fake::default() };
        assert_eq!(type_chars(&mut lost, &text, &delays, KEEP), Err(Stop::Failed(TypeFail::FocusLost)));
        assert!(lost.sent.len() < 130, "{}", lost.sent.len());
    }

    #[test]
    fn erases_the_shortcut_after_the_modifiers_are_released() {
        let text = chars("ok");
        let erase = Erase { count: 3, window: WINDOW };
        let mut kb = Fake { mods_until: 300, ..Fake::default() };
        assert_eq!(type_chars(&mut kb, &text, &[10, 10], erase), Ok(()));
        assert_eq!(kb.sent[..3], [Stroke::Backspace; 3]);
        assert!(kb.sent_at[0] >= 300, "after release");
        assert_eq!(kb.sent.len(), 5);
        let mut moved = Fake::default();
        let elsewhere = Erase { window: WINDOW + 1, ..erase };
        assert_eq!(type_chars(&mut moved, &text, &[10, 10], elsewhere), Ok(()));
        assert!(!moved.sent.contains(&Stroke::Backspace), "another window: nothing is erased");
        assert_eq!(moved.sent.len(), 2);
        let mut cancelled = Fake { stop_at: Some(0), ..Fake::default() };
        assert_eq!(type_chars(&mut cancelled, &text, &[10, 10], erase), Err(Stop::Cancelled));
        assert!(cancelled.sent.is_empty());
    }

    #[test]
    fn only_one_job_at_a_time() {
        let first = TypingJob::try_begin().expect("free");
        assert!(matches!(TypingJob::try_begin(), Err(TypeFail::Busy)));
        assert!(is_busy());
        drop(first);
        assert!(TypingJob::try_begin().is_ok());
    }
}
