//! Parsing and date arithmetic for `rec:[+]Nu` recurrence tags.
//!
//! Format mirrors topydo / SwiftoDo / sleek / dorecur, the de-facto consensus
//! for todo.txt recurrence:
//!
//! ```text
//! rec:Nd     // every N calendar days, anchored to the completion date
//! rec:Nb     // every N business days (Mon-Fri, no holiday calendar)
//! rec:Nw     // every N weeks
//! rec:Nm     // every N months   (clamps month-end: Jan 31 + 1m = Feb 28/29)
//! rec:Ny     // every N years
//! rec:+Nu    // strict: anchored to the previous due date instead
//! ```
//!
//! This module is pure logic — no I/O, no app state. The completion-flow
//! caller decides which anchor date to feed `advance`; this module just
//! moves a `NaiveDate` forward by `RecSpec`.

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecUnit {
    Day,
    BusinessDay,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecSpec {
    /// `+` prefix on the value, e.g. `rec:+1m`. Strict callers anchor on the
    /// previous due date; non-strict anchor on the completion date. The
    /// distinction lives in the caller — this struct just records the bit.
    pub strict: bool,
    pub n: u32,
    pub unit: RecUnit,
}

/// Parse the *value* of a `rec:` tag, e.g. `"+1m"` or `"3b"`. Returns `None`
/// for empty strings, missing or non-positive numbers, and unknown units.
/// Lone `+` (no digits) and `-` prefixes are also rejected — recurrence only
/// moves forward.
pub fn parse_rec_spec(value: &str) -> Option<RecSpec> {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (strict, rest) = if bytes[0] == b'+' {
        (true, &value[1..])
    } else {
        (false, value)
    };
    // Split into digits + trailing unit char. Empty digit run (e.g. `rec:+m`,
    // `rec:d`) and digits-only (e.g. `rec:3`) are both rejected here.
    let unit_byte = *rest.as_bytes().last()?;
    let digits = &rest[..rest.len() - 1];
    if digits.is_empty() {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    if n == 0 {
        return None;
    }
    let unit = match unit_byte {
        b'd' => RecUnit::Day,
        b'b' => RecUnit::BusinessDay,
        b'w' => RecUnit::Week,
        b'm' => RecUnit::Month,
        b'y' => RecUnit::Year,
        _ => return None,
    };
    Some(RecSpec { strict, n, unit })
}

/// Advance `date` by `spec`. Returns `None` only when `chrono` overflows
/// (year ~262_143) — callers treat that as "skip the spawn, flash a notice"
/// rather than panicking.
///
/// Month/year arithmetic uses `chrono::Months`, which clamps the day component
/// to the last valid day of the target month (Jan 31 + 1m → Feb 28 or 29 on a
/// leap year). That matches what topydo, SwiftoDo, and sleek do.
///
/// Business days advance one weekday at a time, skipping Sat/Sun. Starting on
/// a weekend rolls forward to Monday before counting, so "Saturday + 1b"
/// resolves to Monday — same convention as dorecur.
pub fn advance(date: NaiveDate, spec: &RecSpec) -> Option<NaiveDate> {
    match spec.unit {
        RecUnit::Day => date.checked_add_days(Days::new(u64::from(spec.n))),
        RecUnit::Week => date.checked_add_days(Days::new(u64::from(spec.n) * 7)),
        RecUnit::Month => date.checked_add_months(Months::new(spec.n)),
        RecUnit::Year => date.checked_add_months(Months::new(spec.n.checked_mul(12)?)),
        RecUnit::BusinessDay => advance_business(date, spec.n),
    }
}

fn advance_business(mut date: NaiveDate, n: u32) -> Option<NaiveDate> {
    let mut remaining = n;
    while remaining > 0 {
        date = date.checked_add_days(Days::new(1))?;
        if !is_weekend(date) {
            remaining -= 1;
        }
    }
    Some(date)
}

fn is_weekend(date: NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn parses_each_unit() {
        for (s, unit) in [
            ("1d", RecUnit::Day),
            ("2w", RecUnit::Week),
            ("3m", RecUnit::Month),
            ("4y", RecUnit::Year),
            ("5b", RecUnit::BusinessDay),
        ] {
            let spec = parse_rec_spec(s).unwrap();
            assert_eq!(spec.unit, unit);
            assert!(!spec.strict);
        }
    }

    #[test]
    fn parses_strict_prefix() {
        let spec = parse_rec_spec("+1m").unwrap();
        assert!(spec.strict);
        assert_eq!(spec.n, 1);
        assert_eq!(spec.unit, RecUnit::Month);
    }

    #[test]
    fn parses_multi_digit_n() {
        let spec = parse_rec_spec("+10b").unwrap();
        assert!(spec.strict);
        assert_eq!(spec.n, 10);
        assert_eq!(spec.unit, RecUnit::BusinessDay);
    }

    #[test]
    fn rejects_invalid_forms() {
        for bad in [
            "", "d", "1", "+", "+m", "0d", "-1d", "1z", "abc", "1.5d", "1 d", "1dx", " 1d",
        ] {
            assert!(parse_rec_spec(bad).is_none(), "expected None for {:?}", bad);
        }
    }

    #[test]
    fn advance_days_and_weeks() {
        let start = d("2026-05-09");
        assert_eq!(
            advance(start, &parse_rec_spec("1d").unwrap()).unwrap(),
            d("2026-05-10"),
        );
        assert_eq!(
            advance(start, &parse_rec_spec("2w").unwrap()).unwrap(),
            d("2026-05-23"),
        );
    }

    #[test]
    fn advance_months_clamps_to_month_end() {
        // Jan 31 + 1 month → Feb 28 (non-leap) or Feb 29 (leap).
        assert_eq!(
            advance(d("2026-01-31"), &parse_rec_spec("1m").unwrap()).unwrap(),
            d("2026-02-28"),
        );
        assert_eq!(
            advance(d("2024-01-31"), &parse_rec_spec("1m").unwrap()).unwrap(),
            d("2024-02-29"),
        );
        // Mar 31 + 1 month → Apr 30 (April has 30 days).
        assert_eq!(
            advance(d("2026-03-31"), &parse_rec_spec("1m").unwrap()).unwrap(),
            d("2026-04-30"),
        );
    }

    #[test]
    fn advance_year_clamps_leap_day() {
        // Leap day + 1 year → Feb 28 (the next non-leap year).
        assert_eq!(
            advance(d("2024-02-29"), &parse_rec_spec("1y").unwrap()).unwrap(),
            d("2025-02-28"),
        );
    }

    #[test]
    fn advance_business_skips_weekends() {
        // Friday → Monday (single business day).
        assert_eq!(
            advance(d("2026-05-08"), &parse_rec_spec("1b").unwrap()).unwrap(),
            d("2026-05-11"),
        );
        // Saturday + 1b → Monday.
        assert_eq!(
            advance(d("2026-05-09"), &parse_rec_spec("1b").unwrap()).unwrap(),
            d("2026-05-11"),
        );
        // Sunday + 1b → Monday.
        assert_eq!(
            advance(d("2026-05-10"), &parse_rec_spec("1b").unwrap()).unwrap(),
            d("2026-05-11"),
        );
        // Friday + 3b → Wednesday (skip Sat/Sun, count Mon/Tue/Wed).
        assert_eq!(
            advance(d("2026-05-08"), &parse_rec_spec("3b").unwrap()).unwrap(),
            d("2026-05-13"),
        );
    }

    #[test]
    fn advance_overflow_returns_none() {
        // chrono's max is around year 262_143; pushing past it must not panic.
        let near_max = NaiveDate::from_ymd_opt(262_140, 1, 1).unwrap();
        let big = parse_rec_spec("100y").unwrap();
        assert_eq!(advance(near_max, &big), None);
    }
}
