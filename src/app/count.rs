use std::time::Instant;

use super::types::LEADER_WINDOW;

const DIGIT_THRESHOLD: u32 = 8;

/// Numeric prefix (vim-style count) state machine. Digits accumulate via
/// `push_digit`, up to an 8-digit cap, and `take` consumes the pending
/// count (e.g. `5j`, `12G`).
#[derive(Debug, Default, Clone, Copy)]
pub struct Count {
    pending: Option<(u32, Instant)>,
}

impl Count {
    /// Accumulates a digit onto the pending count. Digits beyond the
    /// 8-digit cap are ignored, at most 8 digits are kept.
    pub fn push_digit(&mut self, d: char) {
        let digit = d
            .to_digit(10)
            .expect("push_digit called with a non-digit char");
        let base = self.active().unwrap_or(0);

        // ilog10 returns the power of 10, to obtain the actual count of digits
        // in a number, we have to add 1 to it
        let digit_count = base.checked_ilog10().unwrap_or(0) + 1;

        if digit_count >= DIGIT_THRESHOLD {
            self.pending = Some((base, Instant::now()));
            return;
        }

        let n = base * 10 + digit;
        self.pending = Some((n, Instant::now()));
    }

    /// Currently pending count, or None if expired or absent.
    pub fn active(&self) -> Option<u32> {
        self.pending
            .filter(|(_, t)| t.elapsed() < LEADER_WINDOW)
            .map(|(n, _)| n)
    }

    /// Returns the pending count and clears it, consuming it like
    /// `Option::take`.
    pub fn take(&mut self) -> Option<u32> {
        let n = self.active();
        self.clear();
        n
    }

    /// True when a count is pending but has expired — the event loop uses
    /// this to trigger a redraw so the status-bar indicator clears.
    pub fn should_clear(&self) -> bool {
        self.pending
            .as_ref()
            .map(|(_, t)| t.elapsed() >= LEADER_WINDOW)
            .unwrap_or(false)
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn count_push_digit_accumulates_multiple_digits() {
        let mut c = Count::default();
        c.push_digit('1');
        c.push_digit('4');
        assert_eq!(c.active(), Some(14));
    }

    #[test]
    fn count_push_digit_starts_fresh_after_stale_pending() {
        let stale = Instant::now() - LEADER_WINDOW - Duration::from_millis(10);
        let mut c = Count {
            pending: Some((1, stale)),
        };
        // The stale '1' must not leak into the new number: typing '4' after
        // the window expired should yield 4, not 14.
        c.push_digit('4');
        assert_eq!(c.active(), Some(4));
    }

    #[test]
    fn count_push_digit_caps_at_eight_digits() {
        let mut c = Count::default();
        for d in ['1', '2', '3', '4', '5', '6', '7', '8', '9'] {
            c.push_digit(d);
        }
        // The 9th digit (and any further one) is dropped: the number stays
        // at its 8-digit cap instead of growing or overflowing.
        assert_eq!(c.active(), Some(12345678));
        c.push_digit('5');
        assert_eq!(c.active(), Some(12345678));
    }

    #[test]
    fn count_active_expires_after_window() {
        let stale = Instant::now() - LEADER_WINDOW - Duration::from_millis(10);
        let c = Count {
            pending: Some((7, stale)),
        };
        assert!(c.active().is_none());
        assert!(c.should_clear());
    }
}
