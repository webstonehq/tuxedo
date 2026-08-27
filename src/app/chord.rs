use std::time::{Duration, Instant};

use super::types::LEADER_WINDOW;

/// Two-key chord state machine. A "leader" is armed by the first keypress
/// and consumed by a matching second one (e.g. `gg`, `dd`, `fp`, `fc`).
/// Replaces the older trio of `record_key` / `arm_leader` / `match_prev`.
///
/// With the which-key menu enabled (`which_key: Some(delay)`), an armed
/// leader behaves the way LazyVim's does: it never times out on its own, and
/// once `delay` has elapsed the UI floats a menu of the continuations. With
/// it disabled the leader expires after [`LEADER_WINDOW`], the original
/// behavior.
#[derive(Debug, Default, Clone, Copy)]
pub struct Chord {
    pending: Option<(char, Instant)>,
    which_key: Option<Duration>,
}

impl Chord {
    /// Turn the which-key menu on with the given reveal delay, or off with
    /// `None`. Off restores the plain [`LEADER_WINDOW`] timeout.
    pub fn set_which_key(&mut self, delay: Option<Duration>) {
        self.which_key = delay;
    }

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

    /// Raw pending state — leader plus the instant it was armed — ignoring
    /// expiry. Callers compare it before and after interpreting a key to tell
    /// whether that key touched the chord at all.
    pub fn pending(&self) -> Option<(char, Instant)> {
        self.pending
    }

    /// Currently armed leader, or None if expired or absent.
    pub fn active(&self) -> Option<char> {
        self.pending
            .filter(|(_, t)| self.which_key.is_some() || t.elapsed() < LEADER_WINDOW)
            .map(|(k, _)| k)
    }

    /// The leader whose which-key menu should be on screen right now: armed,
    /// the menu enabled, and the reveal delay elapsed.
    pub fn menu_leader(&self) -> Option<char> {
        let delay = self.which_key?;
        self.pending
            .filter(|(_, t)| t.elapsed() >= delay)
            .map(|(k, _)| k)
    }

    /// Next instant the event loop must wake to repaint: the menu reveal
    /// while it is still pending, otherwise the leader's expiry. `None` when
    /// nothing is scheduled — no leader, or a revealed menu that now waits
    /// indefinitely for its second key.
    pub fn deadline(&self) -> Option<Instant> {
        let (_, armed_at) = self.pending?;
        match self.which_key {
            Some(delay) if armed_at.elapsed() < delay => Some(armed_at + delay),
            Some(_) => None,
            None => Some(armed_at + LEADER_WINDOW),
        }
    }

    /// True when a leader is set but has expired — the event loop uses this
    /// to trigger a redraw so the status-bar indicator clears. Always false
    /// while the which-key menu is enabled: there the leader waits for its
    /// second key rather than lapsing.
    pub fn should_clear(&self) -> bool {
        self.which_key.is_none()
            && self
                .pending
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

    const DELAY: Duration = Duration::from_millis(250);

    fn armed_ago(k: char, ago: Duration, which_key: Option<Duration>) -> Chord {
        Chord {
            pending: Some((k, Instant::now() - ago)),
            which_key,
        }
    }

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
        let c = armed_ago('g', LEADER_WINDOW + Duration::from_millis(10), None);
        assert!(c.active().is_none());
        assert!(c.should_clear());
    }

    #[test]
    fn menu_hidden_before_delay_and_shown_after() {
        let mut c = Chord::default();
        c.set_which_key(Some(DELAY));
        c.arm('f');
        // Freshly armed: leader is live, menu not yet revealed.
        assert_eq!(c.active(), Some('f'));
        assert!(c.menu_leader().is_none());

        let c = armed_ago('f', DELAY + Duration::from_millis(10), Some(DELAY));
        assert_eq!(c.menu_leader(), Some('f'));
    }

    #[test]
    fn menu_disabled_never_reveals() {
        let c = armed_ago('f', Duration::from_secs(5), None);
        assert!(c.menu_leader().is_none());
    }

    #[test]
    fn armed_leader_waits_indefinitely_while_menu_enabled() {
        // Well past LEADER_WINDOW: with which-key on, the leader is still
        // live and must not be cleared out from under the open menu.
        let mut c = armed_ago('y', LEADER_WINDOW * 4, Some(DELAY));
        assert_eq!(c.active(), Some('y'));
        assert!(!c.should_clear());
        assert!(c.consume('y'));
    }

    #[test]
    fn deadline_targets_reveal_then_stops() {
        let now = Instant::now();
        // Menu off: wake at the expiry, as before.
        let c = armed_ago('g', Duration::ZERO, None);
        let d = c.deadline().expect("expiry deadline");
        assert!(d > now && d <= now + LEADER_WINDOW + Duration::from_millis(50));

        // Menu on, pre-reveal: wake at the reveal instant.
        let c = armed_ago('g', Duration::ZERO, Some(DELAY));
        let d = c.deadline().expect("reveal deadline");
        assert!(d <= now + DELAY + Duration::from_millis(50));

        // Menu on and already revealed: nothing left to wake for.
        let c = armed_ago('g', DELAY * 2, Some(DELAY));
        assert!(c.deadline().is_none());

        // No leader at all.
        assert!(Chord::default().deadline().is_none());
    }
}
