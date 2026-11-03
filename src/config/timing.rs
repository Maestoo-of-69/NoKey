//! Parsing and formatting of durations typed by the user.
//!
//! Durations are stored as whole milliseconds. Decimal seconds are parsed
//! digit by digit (no floating point), so "0.01" is exactly 10 ms.

/// Parses seconds like "0.01", "0,25", "1.5" or "3" into milliseconds.
/// Accepts a dot or a comma as the decimal separator, up to 3 decimals.
pub fn parse_seconds(text: &str) -> Result<u32, &'static str> {
    let t = text.trim();
    if t.is_empty() {
        return Err("err.number_empty");
    }
    if t.starts_with('-') {
        return Err("err.number_negative");
    }
    let (int_part, frac_part) = match t.split_once(['.', ',']) {
        Some((i, f)) => (i, f),
        None => (t, ""),
    };
    let digits_only = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if (int_part.is_empty() && frac_part.is_empty()) || !digits_only(int_part) || !digits_only(frac_part) {
        return Err("err.number_format");
    }
    if frac_part.len() > 3 {
        return Err("err.number_precision");
    }
    let whole: u64 = if int_part.is_empty() { 0 } else { int_part.parse().map_err(|_| "err.number_format")? };
    let mut frac: u64 = if frac_part.is_empty() { 0 } else { frac_part.parse().map_err(|_| "err.number_format")? };
    for _ in frac_part.len()..3 {
        frac *= 10;
    }
    let ms = whole.checked_mul(1000).and_then(|w| w.checked_add(frac)).ok_or("err.number_range")?;
    u32::try_from(ms).map_err(|_| "err.number_range")
}

/// Parses seconds and checks the allowed range (in ms).
pub fn parse_seconds_in(text: &str, (lo, hi): (u32, u32)) -> Result<u32, &'static str> {
    let ms = parse_seconds(text)?;
    if (lo..=hi).contains(&ms) { Ok(ms) } else { Err("err.number_range") }
}

/// Formats milliseconds as seconds without trailing zeros: 10 → "0.01",
/// 1500 → "1.5", 3000 → "3". `comma` uses "," as the separator.
pub fn format_seconds(ms: u32, comma: bool) -> String {
    let whole = ms / 1000;
    let frac = ms % 1000;
    if frac == 0 {
        return whole.to_string();
    }
    let digits = format!("{frac:03}");
    let digits = digits.trim_end_matches('0');
    format!("{whole}{}{digits}", if comma { ',' } else { '.' })
}

/// Parses a whole number of milliseconds and checks the range.
pub fn parse_ms_in(text: &str, (lo, hi): (u32, u32)) -> Result<u32, &'static str> {
    let t = text.trim();
    if t.is_empty() {
        return Err("err.number_empty");
    }
    if t.starts_with('-') {
        return Err("err.number_negative");
    }
    let v: u32 = t.parse().map_err(|_| "err.number_format")?;
    if (lo..=hi).contains(&v) { Ok(v) } else { Err("err.number_range") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_seconds_exactly() {
        assert_eq!(parse_seconds("0.01"), Ok(10));
        assert_eq!(parse_seconds("0,05"), Ok(50));
        assert_eq!(parse_seconds("0.1"), Ok(100));
        assert_eq!(parse_seconds("0.25"), Ok(250));
        assert_eq!(parse_seconds("1.5"), Ok(1500));
        assert_eq!(parse_seconds("3"), Ok(3000));
        assert_eq!(parse_seconds(".5"), Ok(500));
        assert_eq!(parse_seconds("2."), Ok(2000));
        assert_eq!(parse_seconds(" 3600 "), Ok(3_600_000));
        assert_eq!(parse_seconds("0.001"), Ok(1));
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(parse_seconds(""), Err("err.number_empty"));
        assert_eq!(parse_seconds("-1"), Err("err.number_negative"));
        assert_eq!(parse_seconds("abc"), Err("err.number_format"));
        assert_eq!(parse_seconds("1.2.3"), Err("err.number_format"));
        assert_eq!(parse_seconds("1,2.3"), Err("err.number_format"));
        assert_eq!(parse_seconds("0.0001"), Err("err.number_precision"));
        assert_eq!(parse_seconds("."), Err("err.number_format"));
        assert_eq!(parse_seconds("99999999999"), Err("err.number_range"));
        assert_eq!(parse_seconds("1e3"), Err("err.number_format"));
    }

    #[test]
    fn checks_cooldown_range() {
        let range = (10, 3_600_000);
        assert_eq!(parse_seconds_in("0.01", range), Ok(10));
        assert_eq!(parse_seconds_in("0.009", range), Err("err.number_range"));
        assert_eq!(parse_seconds_in("3600.001", range), Err("err.number_range"));
    }

    #[test]
    fn formats_round_trip() {
        for (ms, s) in [(10, "0.01"), (50, "0.05"), (100, "0.1"), (250, "0.25"), (1500, "1.5"), (3000, "3"), (1, "0.001")] {
            assert_eq!(format_seconds(ms, false), s);
            assert_eq!(parse_seconds(s), Ok(ms));
        }
        assert_eq!(format_seconds(1500, true), "1,5");
    }

    #[test]
    fn parses_whole_milliseconds() {
        assert_eq!(parse_ms_in("350", (100, 2000)), Ok(350));
        assert_eq!(parse_ms_in("99", (100, 2000)), Err("err.number_range"));
        assert_eq!(parse_ms_in("1.5", (100, 2000)), Err("err.number_format"));
    }
}
