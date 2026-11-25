//! Micro-interaction engine (docs/DESIGN.md, "Motion"). Tiny on purpose:
//!
//! - The registry holds only tweens that are in flight, keyed by
//!   (window, slot). Paint code asks `value(hwnd, slot, rest)` and gets the
//!   tween's current value, or `rest` (the logical state) when nothing runs.
//! - Tweens are started only from mouse-event sites (`animate` / `start`).
//!   Programmatic changes (loading, refresh, global hotkeys, keyboard) never
//!   start one, so they snap by construction.
//! - Looping pulses are requested from paint (`pulse`) and expire on their
//!   own when paint stops asking (state ended, window hidden).
//! - One 15 ms timer per window runs while that window has a tween or a live
//!   pulse; each tick invalidates the window (only the pulsing rects when
//!   nothing but pulses is alive). The tick that finds nothing left, or the
//!   window hidden or minimized, paints one final frame and kills the timer.
//! - Windows' "Animation effects" setting (SPI_GETCLIENTAREAANIMATION) off
//!   = every call snaps to its end state and no timer ever starts.
//!
//! Registry borrows never span a Win32 call (paint and timers re-enter).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{COLORREF, HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsIconic, IsWindowVisible, KillTimer, SetTimer, SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
};

/// WM_TIMER id of the frame driver (far from the windows' own ids 1 and 2).
pub const ANIM_TIMER_ID: usize = 0x4E4B_A000;
const FRAME_MS: u32 = 15;
/// A pulse nobody painted for this long is dropped.
const PULSE_EXPIRY_MS: u128 = 250;
/// How long the system setting is cached before it is read again.
const SETTING_CACHE: Duration = Duration::from_secs(2);

/// One running transition from `from` to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub dur_ms: u32,
}

/// Strong ease-out (cubic): fast start, gentle stop.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Overshooting ease-out for things that move: passes the target by about
/// 10 % and settles back.
pub fn ease_back(t: f32) -> f32 {
    let u = t.clamp(0.0, 1.0) - 1.0;
    1.0 + u * u * (2.7 * u + 1.7)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Value of `tw` after `elapsed_ms`.
pub fn sample(tw: &Tween, elapsed_ms: u128) -> f32 {
    sample_curve(tw, elapsed_ms, ease_out)
}

/// `sample` along another easing curve; ends exactly at `to`.
pub fn sample_curve(tw: &Tween, elapsed_ms: u128, curve: fn(f32) -> f32) -> f32 {
    if tw.dur_ms == 0 || elapsed_ms >= tw.dur_ms as u128 {
        return tw.to;
    }
    lerp(tw.from, tw.to, curve(elapsed_ms as f32 / tw.dur_ms as f32))
}

pub fn finished(tw: &Tween, elapsed_ms: u128) -> bool {
    elapsed_ms >= tw.dur_ms as u128
}

/// A new tween towards `to` that starts where `current` (if any, with its
/// elapsed time) is now, so an interrupted transition never jumps.
pub fn retarget(current: Option<(&Tween, u128)>, from_rest: f32, to: f32, dur_ms: u32) -> Tween {
    let from = current.map_or(from_rest, |(tw, el)| sample(tw, el));
    Tween { from, to, dur_ms }
}

/// 0..1..0 sine over `period_ms`.
pub fn pulse_at(elapsed_ms: u128, period_ms: u32) -> f32 {
    let period = period_ms.max(1) as f32;
    let phase = (elapsed_ms as f32 % period) / period;
    0.5 - 0.5 * (phase * std::f32::consts::TAU).cos()
}

/// Mixes two 0x00BBGGRR colors; `t` is clamped to 0..1.
pub fn lerp_color(a: COLORREF, b: COLORREF, t: f32) -> COLORREF {
    let t = t.clamp(0.0, 1.0);
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32 & 0xFF) << shift
    };
    ch(0) | ch(8) | ch(16)
}

// ---------- registry ----------

type Key = (usize, u32);

struct Running {
    tween: Tween,
    start: Instant,
    curve: fn(f32) -> f32,
}

impl Running {
    fn at(&self, now: Instant) -> f32 {
        sample_curve(&self.tween, now.duration_since(self.start).as_millis(), self.curve)
    }
}

struct Pulse {
    start: Instant,
    last_asked: Instant,
    /// Area to repaint each frame (client coordinates); None = whole window.
    area: Option<RECT>,
}

/// What a window still has running after `prune`.
enum Alive {
    Idle,
    /// At least one tween: repaint the whole window.
    Tweens,
    /// Only pulses: repaint these areas (None = whole window).
    Pulses(Vec<Option<RECT>>),
}

thread_local! {
    static TWEENS: RefCell<HashMap<Key, Running>> = RefCell::new(HashMap::new());
    static PULSES: RefCell<HashMap<Key, Pulse>> = RefCell::new(HashMap::new());
    /// Windows whose frame timer is running.
    static TIMERS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static SETTING: Cell<Option<(Instant, bool)>> = const { Cell::new(None) };
    /// > 0 while a mouse click is being handled (see `pointer_scope`).
    static POINTER: Cell<u32> = const { Cell::new(0) };
}

/// Windows' "Animation effects" setting. Cached for two seconds.
pub fn enabled() -> bool {
    let now = Instant::now();
    if let Some((at, on)) = SETTING.with(Cell::get)
        && now.duration_since(at) < SETTING_CACHE
    {
        return on;
    }
    let mut on: i32 = 1;
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut on as *mut i32 as *mut _, 0) };
    let on = ok == 0 || on != 0; // unreadable: keep the default (on)
    SETTING.with(|s| s.set(Some((now, on))));
    on
}

fn ensure_timer(hwnd: HWND) {
    if TIMERS.with(|t| t.borrow().contains(&(hwnd as usize))) {
        return;
    }
    // Recorded only when the timer really exists (a dead window gets none).
    if unsafe { SetTimer(hwnd, ANIM_TIMER_ID, FRAME_MS, None) } != 0 {
        TIMERS.with(|t| t.borrow_mut().push(hwnd as usize));
    }
}

/// Starts (or retargets from the current value) a transition of `slot` to
/// `to`. `from_rest` is used when nothing is running yet. Call from mouse
/// handlers only; paint reads the result with `value`.
pub fn animate(hwnd: HWND, slot: u32, from_rest: f32, to: f32, dur_ms: u32) {
    run(hwnd, slot, from_rest, to, dur_ms, ease_out);
}

/// `animate` with an overshoot (`ease_back`), for knobs and underlines.
pub fn spring(hwnd: HWND, slot: u32, from_rest: f32, to: f32, dur_ms: u32) {
    run(hwnd, slot, from_rest, to, dur_ms, ease_back);
}

fn run(hwnd: HWND, slot: u32, from_rest: f32, to: f32, dur_ms: u32, curve: fn(f32) -> f32) {
    if hwnd.is_null() {
        return;
    }
    let key = (hwnd as usize, slot);
    if !enabled() || dur_ms == 0 {
        cancel(hwnd, slot);
        return;
    }
    let now = Instant::now();
    TWEENS.with(|m| {
        let mut m = m.borrow_mut();
        // Start where the value is shown now, so an interruption never jumps.
        let shown = m.get(&key).map_or(from_rest, |r| r.at(now));
        let tween = retarget(None, shown, to, dur_ms);
        m.insert(key, Running { tween, start: now, curve });
    });
    ensure_timer(hwnd);
}

/// Starts a transition from exactly `from` (a flash restarts at full strength).
pub fn start(hwnd: HWND, slot: u32, from: f32, to: f32, dur_ms: u32) {
    cancel(hwnd, slot);
    animate(hwnd, slot, from, to, dur_ms);
}

/// Current value of `slot`, or `rest` when no transition is running.
pub fn value(hwnd: HWND, slot: u32, rest: f32) -> f32 {
    let now = Instant::now();
    TWEENS.with(|m| m.borrow().get(&(hwnd as usize, slot)).map_or(rest, |r| r.at(now)))
}

/// Drops a running transition (the next paint shows the rest state).
pub fn cancel(hwnd: HWND, slot: u32) {
    TWEENS.with(|m| m.borrow_mut().remove(&(hwnd as usize, slot)));
}

/// Drops every running transition of `hwnd` whose slot matches `pred`.
pub fn cancel_where(hwnd: HWND, pred: impl Fn(u32) -> bool) {
    TWEENS.with(|m| m.borrow_mut().retain(|(h, slot), _| *h != hwnd as usize || !pred(*slot)));
}

/// A pulse while paint keeps asking for it. It starts at `rest` (0 or 1)
/// and swings to the other end and back over `period_ms`; with animations
/// off it stays at `rest`. `area` is what each frame repaints (client
/// coordinates of `hwnd`; None = the whole window).
pub fn pulse(hwnd: HWND, slot: u32, period_ms: u32, rest: f32, area: Option<RECT>) -> f32 {
    if hwnd.is_null() || !enabled() {
        return rest;
    }
    let now = Instant::now();
    let start = PULSES.with(|m| {
        let mut m = m.borrow_mut();
        let p = m.entry((hwnd as usize, slot)).or_insert(Pulse { start: now, last_asked: now, area });
        p.last_asked = now;
        p.area = area;
        p.start
    });
    ensure_timer(hwnd);
    let beat = pulse_at(now.duration_since(start).as_millis(), period_ms);
    if rest >= 0.5 { 1.0 - beat } else { beat }
}

/// Stops a pulse at once (its state ended).
pub fn cancel_pulse(hwnd: HWND, slot: u32) {
    PULSES.with(|m| m.borrow_mut().remove(&(hwnd as usize, slot)));
}

/// Removes finished tweens and stale pulses of `hwnd` and says what is left.
fn prune(hwnd: HWND, now: Instant) -> Alive {
    let h = hwnd as usize;
    let tweens = TWEENS.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|(k, _), r| *k != h || !finished(&r.tween, now.duration_since(r.start).as_millis()));
        m.keys().any(|(k, _)| *k == h)
    });
    let areas: Vec<Option<RECT>> = PULSES.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|(k, _), p| *k != h || now.duration_since(p.last_asked).as_millis() < PULSE_EXPIRY_MS);
        m.iter().filter(|((k, _), _)| *k == h).map(|(_, p)| p.area).collect()
    });
    match (tweens, areas.is_empty()) {
        (true, _) => Alive::Tweens,
        (false, false) => Alive::Pulses(areas),
        (false, true) => Alive::Idle,
    }
}

/// Frame tick. Windows and subclasses call it from WM_TIMER; true means the
/// timer was the animation timer (handled). Every tick invalidates `hwnd`;
/// the tick that finds nothing left (or the window hidden) paints that last
/// frame and kills the timer, so an idle window has no timer.
pub fn on_timer(hwnd: HWND, timer_id: usize) -> bool {
    if timer_id != ANIM_TIMER_ID {
        return false;
    }
    let shown = unsafe { IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 };
    let alive = if shown { prune(hwnd, Instant::now()) } else { Alive::Idle };
    match alive {
        Alive::Pulses(areas) if areas.iter().all(Option::is_some) => {
            for rc in areas.iter().flatten() {
                unsafe { InvalidateRect(hwnd, rc, 0) };
            }
        }
        Alive::Idle => {
            forget(hwnd);
            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
        }
        _ => unsafe {
            InvalidateRect(hwnd, std::ptr::null(), 0);
        },
    }
    true
}

/// Forgets everything about `hwnd` and stops its timer (WM_NCDESTROY, hide).
pub fn forget(hwnd: HWND) {
    let h = hwnd as usize;
    TWEENS.with(|m| m.borrow_mut().retain(|(k, _), _| *k != h));
    PULSES.with(|m| m.borrow_mut().retain(|(k, _), _| *k != h));
    let had = TIMERS.with(|t| {
        let mut t = t.borrow_mut();
        let before = t.len();
        t.retain(|&x| x != h);
        t.len() != before
    });
    if had {
        unsafe { KillTimer(hwnd, ANIM_TIMER_ID) };
    }
}

/// Runs `f` (normally `DefSubclassProc` for a mouse-button message) marked
/// as pointer-driven, so command handlers it triggers can tell a mouse click
/// from the keyboard or a global hotkey with `by_pointer`.
pub fn pointer_scope<R>(f: impl FnOnce() -> R) -> R {
    POINTER.with(|p| p.set(p.get() + 1));
    let r = f();
    POINTER.with(|p| p.set(p.get().saturating_sub(1)));
    r
}

/// Runs `f` as if no mouse click were being handled, so nothing it does
/// animates (e.g. re-creating windows after a theme change).
pub fn still_scope<R>(f: impl FnOnce() -> R) -> R {
    let depth = POINTER.with(|p| p.replace(0));
    let r = f();
    POINTER.with(|p| p.set(depth));
    r
}

/// True inside `pointer_scope` (a mouse click is being handled).
pub fn by_pointer() -> bool {
    POINTER.with(|p| p.get() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn ease_out_is_monotonic_and_front_loaded() {
        assert!(close(ease_out(0.0), 0.0));
        assert!(close(ease_out(1.0), 1.0));
        assert!(close(ease_out(0.5), 0.875));
        assert!(close(ease_out(-1.0), 0.0) && close(ease_out(2.0), 1.0));
        let mut last = 0.0;
        for i in 1..=20 {
            let v = ease_out(i as f32 / 20.0);
            assert!(v > last);
            last = v;
        }
    }

    #[test]
    fn sample_runs_from_start_to_end() {
        let tw = Tween { from: 0.0, to: 10.0, dur_ms: 100 };
        assert!(close(sample(&tw, 0), 0.0));
        assert!(close(sample(&tw, 50), 8.75));
        assert!(close(sample(&tw, 100), 10.0));
        assert!(close(sample(&tw, 5000), 10.0));
        assert!(!finished(&tw, 99) && finished(&tw, 100));
        let instant = Tween { from: 3.0, to: 7.0, dur_ms: 0 };
        assert!(close(sample(&instant, 0), 7.0));
    }

    #[test]
    fn ease_back_overshoots_a_little_and_ends_on_target() {
        assert!(close(ease_back(0.0), 0.0) && close(ease_back(1.0), 1.0));
        let peak = (0..=100).map(|i| ease_back(i as f32 / 100.0)).fold(0.0f32, f32::max);
        assert!(peak > 1.05 && peak < 1.12, "{peak}");
        let tw = Tween { from: 2.0, to: 4.0, dur_ms: 100 };
        assert!(close(sample_curve(&tw, 100, ease_back), 4.0));
        assert!(sample_curve(&tw, 60, ease_back) > 4.0, "past the target on the way");
    }

    #[test]
    fn retarget_starts_from_the_current_value() {
        let fade_in = Tween { from: 0.0, to: 1.0, dur_ms: 100 };
        let at = sample(&fade_in, 50);
        let back = retarget(Some((&fade_in, 50)), 0.0, 0.0, 100);
        assert!(close(back.from, at), "no jump when interrupted");
        assert!(close(back.to, 0.0));
        assert!(close(sample(&back, 0), at));
        let fresh = retarget(None, 0.25, 1.0, 120);
        assert!(close(fresh.from, 0.25) && fresh.dur_ms == 120);
    }

    #[test]
    fn prune_drops_finished_tweens_and_stale_pulses() {
        // The idle-stop path: once prune() reports nothing left, on_timer kills the timer.
        let hwnd = 0x7000 as HWND;
        let t0 = Instant::now();
        TWEENS.with(|m| m.borrow_mut().insert((0x7000, 1), Running { tween: Tween { from: 0.0, to: 1.0, dur_ms: 100 }, start: t0, curve: ease_out }));
        let area = RECT { left: 1, top: 2, right: 3, bottom: 4 };
        PULSES.with(|m| m.borrow_mut().insert((0x7000, 2), Pulse { start: t0, last_asked: t0, area: Some(area) }));
        assert!(matches!(prune(hwnd, t0 + Duration::from_millis(50)), Alive::Tweens), "tween still running");
        match prune(hwnd, t0 + Duration::from_millis(200)) {
            Alive::Pulses(areas) => assert!(matches!(areas.as_slice(), [Some(rc)] if rc.left == area.left && rc.bottom == area.bottom)),
            _ => panic!("only the pulse should be left"),
        }
        assert!(matches!(prune(hwnd, t0 + Duration::from_millis(400)), Alive::Idle), "everything finished or stale");
        assert!(TWEENS.with(|m| m.borrow().is_empty()) && PULSES.with(|m| m.borrow().is_empty()));
    }

    #[test]
    fn pulse_goes_zero_one_zero() {
        assert!(close(pulse_at(0, 1000), 0.0));
        assert!(close(pulse_at(500, 1000), 1.0));
        assert!(close(pulse_at(1000, 1000), 0.0));
        assert!(close(pulse_at(250, 1000), 0.5));
    }

    #[test]
    fn lerp_color_mixes_channels() {
        let (black, white) = (0x0000_0000, 0x00FF_FFFF);
        assert_eq!(lerp_color(black, white, 0.0), black);
        assert_eq!(lerp_color(black, white, 1.0), white);
        assert_eq!(lerp_color(black, white, 0.5), 0x0080_8080);
        // 0x00BBGGRR: each channel separately; t is clamped.
        assert_eq!(lerp_color(0x0000_00FF, 0x00FF_0000, 0.5), 0x0080_0080);
        assert_eq!(lerp_color(0x0010_2030, 0x0050_6070, 2.0), 0x0050_6070);
        assert_eq!(lerp_color(0x0010_2030, 0x0050_6070, -1.0), 0x0010_2030);
    }
}
