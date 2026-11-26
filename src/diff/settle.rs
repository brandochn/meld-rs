//! Layout-settle detection used to delay clearing a page's busy indicator.
//!
//! GtkSourceView lays out a huge buffer progressively, so `vadjustment.upper()`
//! keeps growing after `FileDiff` has finished applying diff tags. Clearing the
//! busy banner/cursor immediately hides the only progress hint while the view
//! is still catching up (issue #45, RESEARCH 4.3).
//!
//! This module is pure (no GTK) so the decision logic is unit-testable. The GUI
//! timer feeds it the observed `upper` values and an elapsed time.

/// Result of observing one polling tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleOutcome {
    /// Keep waiting for the layout to stop changing.
    Waiting,
    /// Layout has been stable for the required number of ticks.
    Settled,
    /// The hard timeout elapsed; clear busy anyway so it cannot get stuck.
    TimedOut,
}

/// Tracks whether a set of scroll adjustments has stopped growing.
#[derive(Debug, Clone)]
pub struct SettleTracker {
    last: Vec<f64>,
    stable_ticks: u32,
    required_ticks: u32,
    timeout_ms: u64,
}

impl SettleTracker {
    /// Create a tracker seeded with the first observed values.
    pub fn new(uppers: Vec<f64>, required_ticks: u32, timeout_ms: u64) -> Self {
        Self {
            last: uppers,
            stable_ticks: 0,
            required_ticks: required_ticks.max(1),
            timeout_ms,
        }
    }

    /// Observe the current `upper` values and the elapsed time in milliseconds.
    pub fn observe(&mut self, uppers: &[f64], elapsed_ms: u64) -> SettleOutcome {
        let changed = uppers.len() != self.last.len()
            || uppers
                .iter()
                .zip(self.last.iter())
                .any(|(a, b)| (a - b).abs() > 1.0);

        self.last.clear();
        self.last.extend_from_slice(uppers);

        if changed {
            self.stable_ticks = 0;
        } else {
            self.stable_ticks += 1;
        }

        if self.stable_ticks >= self.required_ticks {
            SettleOutcome::Settled
        } else if elapsed_ms >= self.timeout_ms {
            SettleOutcome::TimedOut
        } else {
            SettleOutcome::Waiting
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settles_after_required_stable_ticks() {
        let mut t = SettleTracker::new(vec![100.0], 2, 10_000);
        assert_eq!(t.observe(&[100.0], 250), SettleOutcome::Waiting);
        assert_eq!(t.observe(&[100.0], 500), SettleOutcome::Settled);
    }

    #[test]
    fn resets_when_upper_grows() {
        let mut t = SettleTracker::new(vec![100.0], 2, 10_000);
        assert_eq!(t.observe(&[200.0], 250), SettleOutcome::Waiting);
        assert_eq!(t.observe(&[300.0], 500), SettleOutcome::Waiting);
        assert_eq!(t.observe(&[300.0], 750), SettleOutcome::Waiting);
        assert_eq!(t.observe(&[300.0], 1000), SettleOutcome::Settled);
    }

    #[test]
    fn times_out_when_never_stable() {
        let mut t = SettleTracker::new(vec![0.0], 2, 1000);
        assert_eq!(t.observe(&[10.0], 300), SettleOutcome::Waiting);
        assert_eq!(t.observe(&[20.0], 600), SettleOutcome::Waiting);
        assert_eq!(t.observe(&[30.0], 1000), SettleOutcome::TimedOut);
    }

    #[test]
    fn differing_lengths_count_as_change() {
        // Shrinking the observation is treated as a change, so it cannot settle
        // until a following observation is stable at the new length.
        let mut t = SettleTracker::new(vec![0.0, 0.0], 1, 10_000);
        assert_eq!(t.observe(&[0.0], 100), SettleOutcome::Waiting);
        assert_eq!(t.observe(&[0.0], 200), SettleOutcome::Settled);

        // Growing the observation is also a change.
        let mut t2 = SettleTracker::new(vec![0.0], 1, 10_000);
        assert_eq!(t2.observe(&[0.0, 5.0], 100), SettleOutcome::Waiting);
        assert_eq!(t2.observe(&[0.0, 5.0], 200), SettleOutcome::Settled);
    }
}
