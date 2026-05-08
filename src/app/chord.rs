use std::time::Instant;

use super::types::LEADER_WINDOW;

/// Two-key chord state machine. A "leader" is armed by the first keypress
/// and consumed by a matching second one (e.g. `gg`, `dd`, `fp`, `fc`).
/// Replaces the older trio of `record_key` / `arm_leader` / `match_prev`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Chord {
    pending: Option<(char, Instant)>,
}

impl Chord {
    /// Stamp `k` as the most-recent key, replacing any prior leader.
    pub fn arm(&mut self, k: char) {
        self.pending = Some((k, Instant::now()));
    }

    /// If a pending leader equals `prev` (within the window), consume it
    /// and return true. Does not arm a new leader on miss.
    pub fn consume(&mut self, prev: char) -> bool {
        if self.active() == Some(prev) {
            self.pending = None;
            true
        } else {
            false
        }
    }

    /// Same-key chord: if `k` is already armed, consume and return true;
    /// otherwise arm `k` and return false. Used for `gg`, `dd`.
    pub fn toggle(&mut self, k: char) -> bool {
        if self.consume(k) {
            true
        } else {
            self.arm(k);
            false
        }
    }

    /// Currently armed leader, or None if expired or absent.
    pub fn active(&self) -> Option<char> {
        self.pending
            .filter(|(_, t)| t.elapsed() < LEADER_WINDOW)
            .map(|(k, _)| k)
    }

    /// When the current leader (if any) goes stale.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|(_, t)| t + LEADER_WINDOW)
    }

    /// True when a leader is set but has expired — the event loop uses this
    /// to trigger a redraw so the status-bar indicator clears.
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
    fn chord_toggle_consumes_matching_pair() {
        let mut c = Chord::default();
        assert!(!c.toggle('g'));
        assert_eq!(c.active(), Some('g'));
        assert!(c.toggle('g'));
        assert!(c.active().is_none());
    }

    #[test]
    fn chord_consume_only_matches_armed_leader() {
        let mut c = Chord::default();
        c.arm('f');
        assert!(!c.consume('g'));
        assert_eq!(c.active(), Some('f'));
        assert!(c.consume('f'));
        assert!(c.active().is_none());
        // Empty consume returns false without arming.
        assert!(!c.consume('f'));
        assert!(c.active().is_none());
    }

    #[test]
    fn chord_active_expires_after_window() {
        let stale = Instant::now() - LEADER_WINDOW - Duration::from_millis(10);
        let c = Chord {
            pending: Some(('g', stale)),
        };
        assert!(c.active().is_none());
        assert!(c.should_clear());
    }
}
