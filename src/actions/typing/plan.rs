//! Pure timing planner for "Type text": which keystroke each character
//! becomes and how long to wait after it. No Win32 here, so it is fully
//! unit-tested. Delays come from a tiny deterministic PRNG (xorshift64):
//! the same seed always gives the same rhythm.

use crate::config::{type_text::normalize_newlines, TYPE_CPM_RANGE};

/// What one character becomes on the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stroke {
    Enter,
    Tab,
    /// Erases a character the triggering shortcut typed.
    Backspace,
    /// UTF-16 code units sent with KEYEVENTF_UNICODE (2 for emoji).
    Unicode { units: [u16; 2], len: usize },
}

pub fn stroke(c: char) -> Stroke {
    match c {
        '\n' => Stroke::Enter,
        '\t' => Stroke::Tab,
        _ => {
            let mut units = [0u16; 2];
            let len = c.encode_utf16(&mut units).len();
            Stroke::Unicode { units, len }
        }
    }
}

/// The characters that will be typed: line breaks normalized to '\n',
/// other control characters dropped (they would be unpredictable keys).
pub fn prepare(text: &str) -> Vec<char> {
    normalize_newlines(text).chars().filter(|&c| c == '\n' || c == '\t' || !c.is_control()).collect()
}

/// xorshift64: tiny, deterministic, good enough for jitter.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Zero is xorshift's fixed point; remap it.
        Rng(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

/// Per-character jitter in human mode (±35 %).
const JITTER: f64 = 0.35;
/// Share of spaces that get a small "thinking" pause.
const WORD_PAUSE_CHANCE: f64 = 0.05;

/// Extra pause after a character, on top of its own delay: multiples of the
/// base delay (lo, hi) and clamps in ms (min, max).
struct PauseRule {
    times: (f64, f64),
    clamp_ms: (f64, f64),
}

const COMMA: PauseRule = PauseRule { times: (2.0, 4.0), clamp_ms: (100.0, 1200.0) };
const SENTENCE: PauseRule = PauseRule { times: (7.0, 13.0), clamp_ms: (330.0, 2300.0) };
const NEWLINE: PauseRule = PauseRule { times: (5.0, 9.0), clamp_ms: (200.0, 2000.0) };
const WORD: PauseRule = PauseRule { times: (1.0, 3.0), clamp_ms: (0.0, 800.0) };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pause {
    None,
    Word,
    Comma,
    Newline,
    Sentence,
}

fn is_closer(c: char) -> bool {
    matches!(c, '"' | '\'' | ')' | ']' | '}' | '»' | '”' | '’' | '“')
}

fn is_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…')
}

fn is_clause(c: char) -> bool {
    matches!(c, ',' | ';' | ':' | '—' | '–')
}

/// Pause class after `chars[i]`. Punctuation pauses only count at a break
/// (followed by whitespace or the end), so "3.14" and "..." mid-way don't
/// pause; a closing quote or bracket carries the pause of the mark before it.
fn pause_kind(chars: &[char], i: usize) -> Pause {
    let c = chars[i];
    if c == '\n' {
        return Pause::Newline;
    }
    let next = chars.get(i + 1).copied();
    if next.is_some_and(is_closer) {
        return Pause::None; // the closer will pause instead
    }
    let mut k = i;
    while k > 0 && is_closer(chars[k]) {
        k -= 1;
    }
    let mark = chars[k];
    let at_break = next.is_none_or(char::is_whitespace);
    if at_break && is_sentence_end(mark) {
        return Pause::Sentence;
    }
    if at_break && is_clause(mark) {
        return Pause::Comma;
    }
    if c == ' ' { Pause::Word } else { Pause::None }
}

fn extra(rule: &PauseRule, base: f64, rng: &mut Rng) -> f64 {
    (base * rng.range(rule.times.0, rule.times.1)).clamp(rule.clamp_ms.0, rule.clamp_ms.1)
}

/// Milliseconds between two characters at `cpm` (clamped to the allowed range).
pub fn base_delay_ms(cpm: u32) -> f64 {
    60_000.0 / cpm.clamp(TYPE_CPM_RANGE.0, TYPE_CPM_RANGE.1) as f64
}

/// Delay after each character (same length as `chars`). Plain mode: the
/// base delay everywhere. Human mode: ±35 % jitter, plus pauses after
/// clauses, sentences and line breaks, and now and then between words.
pub fn plan(chars: &[char], cpm: u32, human: bool, seed: u64) -> Vec<u32> {
    let base = base_delay_ms(cpm);
    let mut rng = Rng::new(seed);
    let exact = (0..chars.len())
        .map(|i| {
            if !human {
                return base;
            }
            let own = base * rng.range(1.0 - JITTER, 1.0 + JITTER);
            let more = match pause_kind(chars, i) {
                Pause::None => 0.0,
                Pause::Comma => extra(&COMMA, base, &mut rng),
                Pause::Sentence => extra(&SENTENCE, base, &mut rng),
                Pause::Newline => extra(&NEWLINE, base, &mut rng),
                Pause::Word if rng.unit() < WORD_PAUSE_CHANCE => extra(&WORD, base, &mut rng),
                Pause::Word => 0.0,
            };
            own + more
        });
    whole_ms(exact)
}

/// Rounds to whole milliseconds, carrying the remainder forward, so the
/// total stays exact: 2.4 ms per character must not become a steady 2 ms.
fn whole_ms(delays: impl Iterator<Item = f64>) -> Vec<u32> {
    let (mut exact, mut given) = (0.0f64, 0u64);
    delays
        .map(|d| {
            exact += d;
            let step = (exact.round() as u64).saturating_sub(given);
            given += step;
            step as u32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor ";

    fn avg(v: &[u32]) -> f64 {
        v.iter().map(|&d| d as f64).sum::<f64>() / v.len() as f64
    }

    #[test]
    fn plain_text_averages_the_base_delay_in_both_modes() {
        let chars = prepare(&PLAIN.repeat(60));
        for cpm in [30, 300, 3000, 30_000] {
            let base = base_delay_ms(cpm);
            assert!(plan(&chars, cpm, false, 1).iter().all(|&d| d == base.round() as u32), "constant at {cpm}");
            let human = avg(&plan(&chars, cpm, true, 42));
            assert!((human / base - 1.0).abs() < 0.06, "cpm {cpm}: avg {human:.1} vs base {base:.1}");
        }
    }

    /// Average delay after characters of one class in a long text.
    fn class_avg(cpm: u32, pick: impl Fn(&[char], usize) -> bool) -> f64 {
        let chars = prepare(&"Well, this is it. Then; more words here: yes — ok! ".repeat(80));
        let d = plan(&chars, cpm, true, 7);
        let picked: Vec<u32> = (0..chars.len()).filter(|&i| pick(&chars, i)).map(|i| d[i]).collect();
        assert!(!picked.is_empty());
        avg(&picked)
    }

    #[test]
    fn sentences_pause_longer_than_commas_longer_than_letters() {
        for cpm in [30, 300, 3000] {
            let letters = class_avg(cpm, |c, i| c[i].is_alphabetic());
            let commas = class_avg(cpm, |c, i| c[i] == ',');
            let sentences = class_avg(cpm, |c, i| c[i] == '.' || c[i] == '!');
            assert!(sentences > commas && commas > letters, "cpm {cpm}: {sentences:.0} > {commas:.0} > {letters:.0}");
        }
        let fast_sentence = class_avg(3000, |c, i| c[i] == '.');
        assert!(fast_sentence >= 340.0, "sentence pause keeps its minimum at high speed: {fast_sentence}");
        let slow_sentence = class_avg(30, |c, i| c[i] == '.');
        assert!(slow_sentence <= 2000.0 * 1.35 + 2300.0 + 1.0, "sentence pause is capped: {slow_sentence}");
    }

    #[test]
    fn speed_is_clamped_to_the_allowed_range() {
        assert_eq!(base_delay_ms(1), base_delay_ms(30));
        assert_eq!(base_delay_ms(1_000_000), base_delay_ms(30_000));
        assert_eq!(base_delay_ms(300), 200.0);
        assert_eq!(base_delay_ms(30_000), 2.0);
    }

    #[test]
    fn rounding_keeps_the_total_exact() {
        let chars = prepare(&"a".repeat(1000));
        let delays = plan(&chars, 25_000, false, 1); // 2.4 ms per character
        assert_eq!(delays.iter().map(|&d| d as u64).sum::<u64>(), 2400);
        assert!(delays.iter().all(|&d| d == 2 || d == 3));
    }

    #[test]
    fn newlines_become_enter_and_pause() {
        let chars = prepare("one\r\ntwo\rthree\nfour\u{7}");
        assert_eq!(chars.iter().filter(|&&c| c == '\n').count(), 3, "CRLF, CR and LF are one Enter each");
        assert!(!chars.contains(&'\u{7}'), "other control characters are dropped");
        assert_eq!(stroke('\n'), Stroke::Enter);
        assert_eq!(stroke('\t'), Stroke::Tab);
        let d = plan(&chars, 300, true, 3);
        let nl = chars.iter().position(|&c| c == '\n').unwrap();
        assert!(d[nl] >= 200 + 130, "pause after a line break: {}", d[nl]);
    }

    #[test]
    fn mid_word_punctuation_and_ellipses_do_not_pause() {
        let chars: Vec<char> = "3.14 wait... \"Done.\" ok".chars().collect();
        let kinds: Vec<Pause> = (0..chars.len()).map(|i| pause_kind(&chars, i)).collect();
        assert_eq!(kinds[1], Pause::None, "decimal point");
        let dots: Vec<Pause> = (9..12).map(|i| kinds[i]).collect();
        assert_eq!(dots, vec![Pause::None, Pause::None, Pause::Sentence], "only the last dot of ...");
        let quote_close = chars.iter().rposition(|&c| c == '"').unwrap();
        assert_eq!(kinds[quote_close - 1], Pause::None, "the period before a closing quote");
        assert_eq!(kinds[quote_close], Pause::Sentence, "the quote carries the pause");
    }

    #[test]
    fn emoji_is_a_surrogate_pair_and_rng_is_deterministic() {
        assert_eq!(stroke('👋'), Stroke::Unicode { units: [0xD83D, 0xDC4B], len: 2 });
        assert_eq!(stroke('я'), Stroke::Unicode { units: [0x044F, 0], len: 1 });
        let chars = prepare(PLAIN);
        assert_eq!(plan(&chars, 300, true, 9), plan(&chars, 300, true, 9));
        assert_ne!(Rng::new(0).next(), 0, "seed 0 is remapped");
    }
}
