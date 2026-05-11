//! Parsing and resolution for `t:` threshold tags.
//!
//! Threshold dates hide a task from the active list until a date is reached.
//! The grammar accepts two forms:
//!
//! ```text
//! t:YYYY-MM-DD     // absolute ISO date
//! t:[+-]?Nu        // relative offset, `u` ∈ {d, w, m, b}
//! ```
//!
//! Relative offsets anchor on `due:` if present, else `created_date`. With no
//! anchor the threshold is ignored (the task stays visible) — keeping the
//! filter rule monotonically permissive avoids surprising the user when they
//! delete a `due:` and tasks vanish.
//!
//! Unlike `rec:` (see [`crate::recurrence`]), threshold accepts a leading `-`
//! (offset *before* the anchor — the common case for "show 3 days before
//! due") and `0` (on the anchor). Year (`y`) is intentionally not in the
//! grammar; `-1y` as a threshold is rare enough to not justify the surface.

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};

use crate::recurrence::RecUnit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdSpec {
    Absolute(NaiveDate),
    /// `before: true` subtracts the offset from the anchor, `false` adds it.
    /// `n` may be zero (`t:0d` resolves to the anchor itself).
    Relative {
        before: bool,
        n: u32,
        unit: RecUnit,
    },
}

/// Parse the *value* of a `t:` tag, e.g. `"2026-08-01"`, `"-3d"`, `"+1w"`,
/// `"7d"`. Returns `None` for unrecognized forms — callers treat that as
/// "no threshold" rather than as a hard error.
pub fn parse_threshold(value: &str) -> Option<ThresholdSpec> {
    if value.is_empty() {
        return None;
    }
    if let Ok(d) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(ThresholdSpec::Absolute(d));
    }
    parse_relative(value)
}

fn parse_relative(value: &str) -> Option<ThresholdSpec> {
    let bytes = value.as_bytes();
    let (before, rest) = match bytes[0] {
        b'-' => (true, &value[1..]),
        b'+' => (false, &value[1..]),
        _ => (false, value),
    };
    if rest.is_empty() {
        return None;
    }
    let unit_byte = *rest.as_bytes().last()?;
    let digits = &rest[..rest.len() - 1];
    if digits.is_empty() {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    let unit = match unit_byte {
        b'd' => RecUnit::Day,
        b'b' => RecUnit::BusinessDay,
        b'w' => RecUnit::Week,
        b'm' => RecUnit::Month,
        // Year deliberately omitted from the threshold grammar.
        _ => return None,
    };
    Some(ThresholdSpec::Relative { before, n, unit })
}

/// Resolve `spec` to an absolute date. Absolute thresholds return their date
/// verbatim; relative thresholds need an anchor — `due` first, falling back
/// to `created`. Returns `None` when relative + no usable anchor, or when
/// date arithmetic overflows.
pub fn resolve(
    spec: &ThresholdSpec,
    due: Option<&str>,
    created: Option<&str>,
) -> Option<NaiveDate> {
    match *spec {
        ThresholdSpec::Absolute(d) => Some(d),
        ThresholdSpec::Relative { before, n, unit } => {
            let anchor_str = due.or(created)?;
            let anchor = NaiveDate::parse_from_str(anchor_str, "%Y-%m-%d").ok()?;
            shift(anchor, n, unit, before)
        }
    }
}

fn shift(date: NaiveDate, n: u32, unit: RecUnit, before: bool) -> Option<NaiveDate> {
    match unit {
        RecUnit::Day => {
            let days = Days::new(u64::from(n));
            if before {
                date.checked_sub_days(days)
            } else {
                date.checked_add_days(days)
            }
        }
        RecUnit::Week => {
            let days = Days::new(u64::from(n) * 7);
            if before {
                date.checked_sub_days(days)
            } else {
                date.checked_add_days(days)
            }
        }
        RecUnit::Month => {
            let months = Months::new(n);
            if before {
                date.checked_sub_months(months)
            } else {
                date.checked_add_months(months)
            }
        }
        RecUnit::BusinessDay => {
            if before {
                retreat_business(date, n)
            } else {
                advance_business(date, n)
            }
        }
        // Year isn't in the threshold grammar; if a Relative spec somehow
        // carries it (it can't via parse_threshold), treat as no-op.
        RecUnit::Year => None,
    }
}

fn advance_business(mut date: NaiveDate, n: u32) -> Option<NaiveDate> {
    if n == 0 {
        return Some(date);
    }
    let mut remaining = n;
    while remaining > 0 {
        date = date.checked_add_days(Days::new(1))?;
        if !is_weekend(date) {
            remaining -= 1;
        }
    }
    Some(date)
}

fn retreat_business(mut date: NaiveDate, n: u32) -> Option<NaiveDate> {
    if n == 0 {
        return Some(date);
    }
    let mut remaining = n;
    while remaining > 0 {
        date = date.checked_sub_days(Days::new(1))?;
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
    fn parses_absolute_iso_date() {
        let spec = parse_threshold("2026-08-01").unwrap();
        assert_eq!(spec, ThresholdSpec::Absolute(d("2026-08-01")));
    }

    #[test]
    fn rejects_invalid_absolute_date() {
        // `2026-99-01` is not a real calendar date.
        assert!(parse_threshold("2026-99-01").is_none());
    }

    #[test]
    fn parses_relative_with_minus_prefix() {
        let spec = parse_threshold("-3d").unwrap();
        assert_eq!(
            spec,
            ThresholdSpec::Relative {
                before: true,
                n: 3,
                unit: RecUnit::Day,
            }
        );
    }

    #[test]
    fn parses_relative_with_plus_prefix() {
        let spec = parse_threshold("+1w").unwrap();
        assert_eq!(
            spec,
            ThresholdSpec::Relative {
                before: false,
                n: 1,
                unit: RecUnit::Week,
            }
        );
    }

    #[test]
    fn parses_relative_without_prefix_as_after() {
        let spec = parse_threshold("7d").unwrap();
        assert_eq!(
            spec,
            ThresholdSpec::Relative {
                before: false,
                n: 7,
                unit: RecUnit::Day,
            }
        );
    }

    #[test]
    fn parses_zero_as_relative() {
        // `0d` is "on the anchor" — useful for "show on due date itself"
        // rather than the absolute fallback.
        let spec = parse_threshold("0d").unwrap();
        assert_eq!(
            spec,
            ThresholdSpec::Relative {
                before: false,
                n: 0,
                unit: RecUnit::Day,
            }
        );
    }

    #[test]
    fn parses_each_unit() {
        for (s, unit) in [
            ("1d", RecUnit::Day),
            ("2w", RecUnit::Week),
            ("3m", RecUnit::Month),
            ("4b", RecUnit::BusinessDay),
        ] {
            let spec = parse_threshold(s).unwrap();
            assert!(matches!(
                spec,
                ThresholdSpec::Relative { unit: u, .. } if u == unit
            ));
        }
    }

    #[test]
    fn parses_multi_digit_n() {
        let spec = parse_threshold("-10b").unwrap();
        assert_eq!(
            spec,
            ThresholdSpec::Relative {
                before: true,
                n: 10,
                unit: RecUnit::BusinessDay,
            }
        );
    }

    #[test]
    fn rejects_invalid_forms() {
        for bad in [
            "", "abc", "1y", "1.5d", "d", "-", "+", "-d", "+m", "1z", " 1d", "1d ",
        ] {
            assert!(
                parse_threshold(bad).is_none(),
                "expected None for {:?}",
                bad
            );
        }
    }

    #[test]
    fn resolve_absolute_returns_date_verbatim() {
        let spec = ThresholdSpec::Absolute(d("2026-08-01"));
        assert_eq!(resolve(&spec, None, None), Some(d("2026-08-01")));
        // Anchors don't matter for absolute.
        assert_eq!(
            resolve(&spec, Some("2026-01-01"), Some("2025-12-01")),
            Some(d("2026-08-01"))
        );
    }

    #[test]
    fn resolve_relative_anchors_on_due() {
        let spec = parse_threshold("-3d").unwrap();
        assert_eq!(
            resolve(&spec, Some("2026-05-15"), Some("2026-04-01")),
            Some(d("2026-05-12")),
        );
    }

    #[test]
    fn resolve_relative_falls_back_to_created() {
        let spec = parse_threshold("7d").unwrap();
        assert_eq!(
            resolve(&spec, None, Some("2026-04-01")),
            Some(d("2026-04-08")),
        );
    }

    #[test]
    fn resolve_relative_no_anchor_returns_none() {
        let spec = parse_threshold("-3d").unwrap();
        assert_eq!(resolve(&spec, None, None), None);
    }

    #[test]
    fn resolve_relative_invalid_anchor_returns_none() {
        let spec = parse_threshold("-3d").unwrap();
        assert_eq!(resolve(&spec, Some("not-a-date"), None), None);
    }

    #[test]
    fn resolve_zero_offset_returns_anchor() {
        let spec = parse_threshold("0d").unwrap();
        assert_eq!(
            resolve(&spec, Some("2026-05-15"), None),
            Some(d("2026-05-15")),
        );
    }

    #[test]
    fn resolve_month_subtraction_clamps_month_end() {
        // Mar 31 - 1m → Feb 28 (non-leap year).
        let spec = parse_threshold("-1m").unwrap();
        assert_eq!(
            resolve(&spec, Some("2026-03-31"), None),
            Some(d("2026-02-28")),
        );
    }

    #[test]
    fn resolve_business_days_skip_weekends_backward() {
        // Wed 2026-05-13 - 3b → Fri 2026-05-08 (skip Sun + Sat going back).
        let spec = parse_threshold("-3b").unwrap();
        assert_eq!(
            resolve(&spec, Some("2026-05-13"), None),
            Some(d("2026-05-08")),
        );
    }

    #[test]
    fn resolve_business_days_forward_skip_weekends() {
        // Fri 2026-05-08 + 1b → Mon 2026-05-11.
        let spec = parse_threshold("1b").unwrap();
        assert_eq!(
            resolve(&spec, Some("2026-05-08"), None),
            Some(d("2026-05-11")),
        );
    }

    #[test]
    fn resolve_week_offset() {
        let spec = parse_threshold("-2w").unwrap();
        assert_eq!(
            resolve(&spec, Some("2026-05-15"), None),
            Some(d("2026-05-01")),
        );
    }
}
