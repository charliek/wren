#![forbid(unsafe_code)]

//! Pure autosave scheduler with injected time.
//!
//! Tracks edits, debounces saves (750 ms idle), enforces a maximum dirty age
//! (750 ms) during continuous typing, and manages at most one in-flight save
//! with revision-aware completion and bounded exponential retry backoff.

use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(750);
const MAX_DIRTY_AGE: Duration = Duration::from_millis(750);
const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// Monotonically increasing document revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

/// Handle for an in-flight save, carrying the document generation and the
/// revision being persisted. Stale tickets (wrong generation or superseded
/// by reset/clean) are silently ignored on completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket {
    generation: u64,
    revision: Revision,
}

impl Ticket {
    /// The document revision this save covers.
    pub fn revision(&self) -> Revision {
        self.revision
    }
}

/// Pure autosave scheduler. Owns no I/O and spawns no threads; the caller
/// drives time, persistence, and timer polling.
#[derive(Debug)]
pub struct Autosave {
    /// Bumped on reset to invalidate tickets from a previous document.
    generation: u64,
    /// Current document revision, incremented on every edit.
    revision: Revision,
    /// Last revision successfully persisted to disk.
    saved_revision: Revision,
    /// Timestamp of the most recent edit (idle debounce reference).
    last_edit_time: Option<Duration>,
    /// Timestamp of the oldest edit not yet persisted (max-age reference).
    oldest_dirty_time: Option<Duration>,
    /// First edit time during the current in-flight save, so a successful
    /// completion can set the correct oldest-dirty time for newer edits.
    unsaved_since_flight: Option<Duration>,
    /// Currently in-flight save, if any.
    in_flight: Option<Ticket>,
    /// Consecutive save failures for backoff calculation.
    consecutive_failures: u32,
    /// When the last failure occurred, for backoff timing.
    last_failure_time: Option<Duration>,
}

impl Default for Autosave {
    fn default() -> Self {
        Self::new()
    }
}

impl Autosave {
    /// Create a scheduler for a new, clean document.
    pub fn new() -> Self {
        Self {
            generation: 0,
            revision: Revision(0),
            saved_revision: Revision(0),
            last_edit_time: None,
            oldest_dirty_time: None,
            unsaved_since_flight: None,
            in_flight: None,
            consecutive_failures: 0,
            last_failure_time: None,
        }
    }

    /// Record a document edit at the given timestamp.
    pub fn edited(&mut self, now: Duration) {
        self.revision.0 += 1;
        self.last_edit_time = Some(now);
        if self.oldest_dirty_time.is_none() {
            self.oldest_dirty_time = Some(now);
        }
        if self.in_flight.is_some() && self.unsaved_since_flight.is_none() {
            self.unsaved_since_flight = Some(now);
        }
    }

    /// Whether there are unsaved edits.
    pub fn dirty(&self) -> bool {
        self.revision > self.saved_revision
    }

    /// Whether a save should be initiated now: dirty, no save in flight,
    /// retry backoff elapsed, and either idle debounce or max dirty age met.
    pub fn due(&self, now: Duration) -> bool {
        if !self.dirty() || self.in_flight.is_some() {
            return false;
        }
        if !self.backoff_elapsed(now) {
            return false;
        }
        let idle = self
            .last_edit_time
            .is_some_and(|t| now.saturating_sub(t) >= DEBOUNCE);
        let aged = self
            .oldest_dirty_time
            .is_some_and(|t| now.saturating_sub(t) >= MAX_DIRTY_AGE);
        idle || aged
    }

    /// Start a save. Returns a ticket if a save can begin (dirty, none
    /// in flight, and backoff elapsed or `force` is set). Explicit flushes
    /// (note switch, quit, move) pass `force = true` to bypass retry backoff.
    pub fn begin(&mut self, now: Duration, force: bool) -> Option<Ticket> {
        if !self.dirty() || self.in_flight.is_some() {
            return None;
        }
        if !force && !self.backoff_elapsed(now) {
            return None;
        }
        let ticket = Ticket {
            generation: self.generation,
            revision: self.revision,
        };
        self.in_flight = Some(ticket);
        self.unsaved_since_flight = None;
        Some(ticket)
    }

    /// Complete a save. Ignored if the ticket is stale (generation mismatch
    /// after reset, or not the current in-flight ticket).
    pub fn finish(&mut self, ticket: Ticket, success: bool, now: Duration) {
        if self.in_flight != Some(ticket) {
            return;
        }
        self.in_flight = None;
        if success {
            if ticket.revision >= self.saved_revision {
                self.saved_revision = ticket.revision;
            }
            if self.revision <= self.saved_revision {
                // Fully clean.
                self.oldest_dirty_time = None;
                self.last_edit_time = None;
            } else {
                // Edits arrived during the save; their oldest time survives.
                self.oldest_dirty_time = self.unsaved_since_flight;
            }
            self.consecutive_failures = 0;
            self.last_failure_time = None;
        } else {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            self.last_failure_time = Some(now);
        }
        self.unsaved_since_flight = None;
    }

    /// Reset for a new document. Clears all state and bumps the generation
    /// so any in-flight ticket from the previous document is invalidated.
    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.revision = Revision(0);
        self.saved_revision = Revision(0);
        self.last_edit_time = None;
        self.oldest_dirty_time = None;
        self.unsaved_since_flight = None;
        self.in_flight = None;
        self.consecutive_failures = 0;
        self.last_failure_time = None;
    }

    /// Mark the buffer as clean (e.g., undo back to the saved baseline).
    /// Invalidates any in-flight ticket since the save is no longer needed.
    pub fn mark_clean(&mut self) {
        self.saved_revision = self.revision;
        self.last_edit_time = None;
        self.oldest_dirty_time = None;
        self.unsaved_since_flight = None;
        self.in_flight = None;
        self.consecutive_failures = 0;
        self.last_failure_time = None;
    }

    /// The current document revision.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// The last successfully persisted revision.
    pub fn saved_revision(&self) -> Revision {
        self.saved_revision
    }

    fn backoff_elapsed(&self, now: Duration) -> bool {
        if self.consecutive_failures == 0 {
            return true;
        }
        let delay = self.backoff_delay();
        self.last_failure_time
            .is_none_or(|t| now.saturating_sub(t) >= delay)
    }

    fn backoff_delay(&self) -> Duration {
        if self.consecutive_failures == 0 {
            return Duration::ZERO;
        }
        let shift = (self.consecutive_failures - 1).min(31);
        let base_ms = BACKOFF_BASE.as_millis() as u64;
        let ms = base_ms.saturating_mul(1u64 << shift);
        let capped = ms.min(BACKOFF_CAP.as_millis() as u64);
        Duration::from_millis(capped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    // ── basic lifecycle ─────────────────────────────────────────────

    #[test]
    fn new_is_clean() {
        let s = Autosave::new();
        assert!(!s.dirty());
        assert!(!s.due(ms(0)));
        assert!(!s.due(ms(10_000)));
    }

    #[test]
    fn edit_marks_dirty() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        assert!(s.dirty());
    }

    #[test]
    fn not_due_before_debounce() {
        let mut s = Autosave::new();
        s.edited(ms(100));
        assert!(!s.due(ms(100)));
        assert!(!s.due(ms(849))); // 749 ms after edit
    }

    #[test]
    fn due_after_debounce() {
        let mut s = Autosave::new();
        s.edited(ms(100));
        assert!(s.due(ms(850))); // exactly 750 ms after edit
    }

    #[test]
    fn full_save_cycle() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        assert!(s.due(ms(750)));

        let ticket = s.begin(ms(750), false).unwrap();
        assert!(s.dirty());
        assert!(!s.due(ms(1500))); // in flight

        s.finish(ticket, true, ms(800));
        assert!(!s.dirty());
        assert!(!s.due(ms(10_000)));
    }

    // ── idle debounce versus max dirty age ───────────────────────────

    #[test]
    fn max_age_triggers_during_continuous_typing() {
        let mut s = Autosave::new();
        // Type every 100 ms for 800 ms — debounce never fires but max age does.
        for t in (0..=700).step_by(100) {
            s.edited(ms(t));
        }
        // Debounce: 750 - 700 = 50 ms since last edit — not enough.
        // Max age: 750 - 0 = 750 ms since first edit — fires.
        assert!(s.due(ms(750)));
    }

    #[test]
    fn idle_fires_before_max_age_when_typing_stops_early() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        // Stop typing. At t=750 both idle (750 >= 750) and age (750 >= 750) fire.
        assert!(s.due(ms(750)));
        // But at t=400 with another edit at t=400:
        let mut s2 = Autosave::new();
        s2.edited(ms(0));
        s2.edited(ms(400));
        // At t=749: idle = 349 < 750, age = 749 < 750 → not due.
        assert!(!s2.due(ms(749)));
        // At t=750: age = 750 >= 750 → due (even though idle = 350 < 750).
        assert!(s2.due(ms(750)));
    }

    // ── continuous typing through multiple save deadlines ────────────

    #[test]
    fn continuous_typing_multiple_cycles() {
        let mut s = Autosave::new();

        // Cycle 1: type from t=0..700, max age at t=750.
        for t in (0..=700).step_by(100) {
            s.edited(ms(t));
        }
        assert!(s.due(ms(750)));
        let t1 = s.begin(ms(750), false).unwrap();

        // Continue typing during save.
        for t in (800..=1400).step_by(100) {
            s.edited(ms(t));
        }
        // Save completes at t=1500.
        s.finish(t1, true, ms(1500));
        assert!(s.dirty());
        // oldest_dirty_time is the first edit during save (t=800).
        // At t=1550: age = 1550-800 = 750 → due.
        assert!(s.due(ms(1550)));

        // Cycle 2.
        let t2 = s.begin(ms(1550), false).unwrap();
        for t in (1600..=2200).step_by(100) {
            s.edited(ms(t));
        }
        s.finish(t2, true, ms(2300));
        assert!(s.dirty());
        // oldest_dirty = 1600, at t=2350: age = 750 → due.
        assert!(s.due(ms(2350)));
    }

    // ── in-flight edits ─────────────────────────────────────────────

    #[test]
    fn edits_during_save_stay_dirty() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let ticket = s.begin(ms(750), false).unwrap();

        s.edited(ms(800));
        s.edited(ms(900));

        s.finish(ticket, true, ms(1000));
        assert!(s.dirty());
        assert_eq!(s.saved_revision(), Revision(1));
        assert_eq!(s.revision(), Revision(3));
    }

    #[test]
    fn no_second_begin_while_in_flight() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let _t = s.begin(ms(750), false).unwrap();
        s.edited(ms(800));
        assert!(s.begin(ms(1600), false).is_none());
        assert!(s.begin(ms(1600), true).is_none());
    }

    #[test]
    fn due_false_while_in_flight() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let _t = s.begin(ms(750), false).unwrap();
        s.edited(ms(800));
        assert!(!s.due(ms(5000)));
    }

    // ── stale completion and reset ──────────────────────────────────

    #[test]
    fn stale_ticket_after_reset_ignored() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let ticket = s.begin(ms(750), false).unwrap();

        s.reset();
        s.edited(ms(1000));
        s.finish(ticket, true, ms(1500));

        // The stale ticket must not clear the new document's dirty state.
        assert!(s.dirty());
        assert_eq!(s.saved_revision(), Revision(0));
        assert_eq!(s.revision(), Revision(1));
    }

    #[test]
    fn reset_clears_everything() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        s.edited(ms(100));
        s.reset();
        assert!(!s.dirty());
        assert!(!s.due(ms(10_000)));
        assert_eq!(s.revision(), Revision(0));
    }

    #[test]
    fn reset_then_new_edits() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        s.reset();
        s.edited(ms(1000));
        assert!(s.dirty());
        assert!(s.due(ms(1750)));
    }

    #[test]
    fn stale_ticket_does_not_affect_new_save() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let old_ticket = s.begin(ms(750), false).unwrap();
        s.reset();
        s.edited(ms(1000));
        let new_ticket = s.begin(ms(1750), false).unwrap();

        // Old ticket arrives — ignored because in_flight is the new ticket.
        s.finish(old_ticket, true, ms(1800));
        assert!(s.dirty());
        assert!(s.in_flight.is_some());

        // New ticket succeeds.
        s.finish(new_ticket, true, ms(1900));
        assert!(!s.dirty());
    }

    // ── failure and backoff ─────────────────────────────────────────

    #[test]
    fn failure_keeps_dirty() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let ticket = s.begin(ms(750), false).unwrap();
        s.finish(ticket, false, ms(800));
        assert!(s.dirty());
    }

    #[test]
    fn backoff_prevents_immediate_retry() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t = s.begin(ms(750), false).unwrap();
        s.finish(t, false, ms(800));

        // Backoff = 1 s after first failure.
        assert!(!s.due(ms(1799))); // 999 ms after failure
        assert!(s.due(ms(1800))); // 1000 ms after failure
    }

    #[test]
    fn exponential_backoff() {
        let mut s = Autosave::new();
        s.edited(ms(0));

        // Fail twice.
        let t1 = s.begin(ms(750), false).unwrap();
        s.finish(t1, false, ms(800));
        // Backoff = 1 s.
        let t2 = s.begin(ms(1800), false).unwrap();
        s.finish(t2, false, ms(1900));
        // Backoff = 2 s.
        assert!(!s.due(ms(3899))); // 1999 ms after failure
        assert!(s.due(ms(3900))); // 2000 ms after failure

        // Fail again → backoff = 4 s.
        let t3 = s.begin(ms(3900), false).unwrap();
        s.finish(t3, false, ms(4000));
        assert!(!s.due(ms(7999)));
        assert!(s.due(ms(8000)));
    }

    #[test]
    fn backoff_capped_at_30s() {
        let mut s = Autosave::new();
        s.edited(ms(0));

        let mut t_fail = 750u64;
        // Fail many times to exceed the cap.
        for _ in 0..20 {
            let ticket = s.begin(ms(t_fail), false).unwrap();
            t_fail += 50;
            s.finish(ticket, false, ms(t_fail));
            t_fail += s.backoff_delay().as_millis() as u64;
        }
        // After 20 failures, backoff should be capped at 30 s.
        assert_eq!(s.backoff_delay(), Duration::from_secs(30));
    }

    #[test]
    fn success_resets_backoff() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t1 = s.begin(ms(750), false).unwrap();
        s.finish(t1, false, ms(800));
        assert!(!s.due(ms(801))); // backoff active

        // Wait for backoff, succeed.
        let t2 = s.begin(ms(1800), false).unwrap();
        s.edited(ms(1850)); // keep dirty for next cycle
        s.finish(t2, true, ms(1900));

        // No backoff — immediately due once debounce passes.
        assert!(s.due(ms(2600))); // 750 ms after edit at 1850
    }

    #[test]
    fn force_bypasses_backoff() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t = s.begin(ms(750), false).unwrap();
        s.finish(t, false, ms(800));

        // Backoff would prevent begin(…, false):
        assert!(s.begin(ms(801), false).is_none());
        // But force ignores backoff:
        assert!(s.begin(ms(801), true).is_some());
    }

    #[test]
    fn begin_returns_none_during_backoff() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t = s.begin(ms(750), false).unwrap();
        s.finish(t, false, ms(800));
        assert!(s.begin(ms(900), false).is_none());
    }

    // ── mark_clean / undo-to-baseline ───────────────────────────────

    #[test]
    fn mark_clean_clears_dirty() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        s.edited(ms(100));
        assert!(s.dirty());
        s.mark_clean();
        assert!(!s.dirty());
        assert!(!s.due(ms(10_000)));
    }

    #[test]
    fn mark_clean_invalidates_in_flight() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let ticket = s.begin(ms(750), false).unwrap();
        s.mark_clean();

        // Ticket is now stale.
        s.finish(ticket, true, ms(800));
        assert!(!s.dirty());
        assert_eq!(s.saved_revision(), s.revision());
    }

    #[test]
    fn mark_clean_then_new_edits() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        s.mark_clean();
        assert!(!s.dirty());

        s.edited(ms(500));
        assert!(s.dirty());
        assert!(!s.due(ms(500)));
        assert!(s.due(ms(1250)));

        let ticket = s.begin(ms(1250), false).unwrap();
        s.finish(ticket, true, ms(1300));
        assert!(!s.dirty());
    }

    #[test]
    fn mark_clean_resets_backoff() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t = s.begin(ms(750), false).unwrap();
        s.finish(t, false, ms(800));
        // Backoff active.
        s.mark_clean();
        s.edited(ms(900));
        // Backoff should be gone.
        assert!(s.due(ms(1650)));
        assert!(s.begin(ms(1650), false).is_some());
    }

    // ── backward / saturating time ──────────────────────────────────

    #[test]
    fn backward_time_does_not_panic() {
        let mut s = Autosave::new();
        s.edited(ms(1000));
        // Clock goes backward — saturating_sub prevents underflow.
        assert!(!s.due(ms(500)));
        // Still works forward from the original edit time.
        assert!(s.due(ms(1750)));
    }

    #[test]
    fn backward_time_backoff_does_not_spin() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t = s.begin(ms(750), false).unwrap();
        s.finish(t, false, ms(1000));
        // Clock goes backward — backoff must not appear elapsed.
        assert!(!s.due(ms(500)));
    }

    // ── no spurious saves ───────────────────────────────────────────

    #[test]
    fn begin_returns_none_when_clean() {
        let mut s = Autosave::new();
        assert!(s.begin(ms(0), false).is_none());
        assert!(s.begin(ms(0), true).is_none());
    }

    #[test]
    fn due_stays_false_after_successful_save() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t = s.begin(ms(750), false).unwrap();
        s.finish(t, true, ms(800));
        // No new edits → never due again.
        assert!(!s.due(ms(100_000)));
    }

    #[test]
    fn finish_wrong_ticket_is_noop() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let real = s.begin(ms(750), false).unwrap();
        let fake = Ticket {
            generation: 99,
            revision: Revision(99),
        };
        s.finish(fake, true, ms(800));
        // In-flight should still be active.
        assert!(s.in_flight.is_some());
        // Clean up.
        s.finish(real, true, ms(900));
        assert!(!s.dirty());
    }

    // ── revision tracking ───────────────────────────────────────────

    #[test]
    fn revision_increments_on_edit() {
        let mut s = Autosave::new();
        assert_eq!(s.revision(), Revision(0));
        s.edited(ms(0));
        assert_eq!(s.revision(), Revision(1));
        s.edited(ms(100));
        assert_eq!(s.revision(), Revision(2));
    }

    #[test]
    fn saved_revision_advances_on_success() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        s.edited(ms(100));
        let t = s.begin(ms(850), false).unwrap();
        assert_eq!(t.revision(), Revision(2));
        s.finish(t, true, ms(900));
        assert_eq!(s.saved_revision(), Revision(2));
    }

    #[test]
    fn saved_revision_unchanged_on_failure() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t = s.begin(ms(750), false).unwrap();
        s.finish(t, false, ms(800));
        assert_eq!(s.saved_revision(), Revision(0));
    }

    #[test]
    fn ticket_carries_correct_revision() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        s.edited(ms(100));
        s.edited(ms(200));
        let t = s.begin(ms(950), false).unwrap();
        assert_eq!(t.revision(), Revision(3));
    }

    // ── oldest dirty time tracking ──────────────────────────────────

    #[test]
    fn oldest_dirty_time_set_on_first_edit() {
        let mut s = Autosave::new();
        s.edited(ms(500));
        s.edited(ms(600));
        // Max age fires at oldest + 750 = 1250.
        assert!(!s.due(ms(1249)));
        assert!(s.due(ms(1250)));
    }

    #[test]
    fn oldest_dirty_time_updated_after_save_with_inflight_edits() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t = s.begin(ms(750), false).unwrap();
        s.edited(ms(800)); // first edit during save
        s.edited(ms(900));
        s.finish(t, true, ms(1000));

        // oldest_dirty_time should now be 800 (first edit during save).
        // Due at 800 + 750 = 1550.
        assert!(!s.due(ms(1549)));
        assert!(s.due(ms(1550)));
    }

    #[test]
    fn oldest_dirty_time_preserved_on_failure() {
        let mut s = Autosave::new();
        s.edited(ms(100));
        s.edited(ms(200));
        let t = s.begin(ms(850), false).unwrap();
        s.finish(t, false, ms(900));

        // oldest_dirty_time should still be 100 (unchanged by failure).
        // After backoff (1 s from t=900 → t=1900), due if age met.
        // age at 1900: 1900 - 100 = 1800 >= 750 → yes.
        assert!(s.due(ms(1900)));
    }

    // ── default trait ───────────────────────────────────────────────

    #[test]
    fn default_is_clean() {
        let s = Autosave::default();
        assert!(!s.dirty());
        assert_eq!(s.revision(), Revision(0));
        assert_eq!(s.saved_revision(), Revision(0));
    }

    // ── edge: save completes with exact revision match ──────────────

    #[test]
    fn save_exact_revision_clears_dirty() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        s.edited(ms(100));
        s.edited(ms(200));
        let t = s.begin(ms(950), false).unwrap();
        // No edits during save.
        s.finish(t, true, ms(1000));
        assert!(!s.dirty());
        assert!(!s.due(ms(100_000)));
    }

    // ── edge: generation wrapping ───────────────────────────────────

    #[test]
    fn generation_wraps_safely() {
        let mut s = Autosave::new();
        s.generation = u64::MAX;
        s.reset();
        assert_eq!(s.generation, 0); // wrapped around
        s.edited(ms(0));
        assert!(s.dirty());
    }

    // ── edge: many rapid edits ──────────────────────────────────────

    #[test]
    fn rapid_edits_single_save() {
        let mut s = Autosave::new();
        for i in 0..100 {
            s.edited(ms(i));
        }
        assert_eq!(s.revision(), Revision(100));
        assert!(s.due(ms(750))); // max age: 750 - 0 = 750
        let t = s.begin(ms(750), false).unwrap();
        s.finish(t, true, ms(800));
        assert!(!s.dirty());
    }

    // ── force begin without prior due check ─────────────────────────

    #[test]
    fn force_begin_works_before_debounce() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        // Not due yet (only 100 ms after edit).
        assert!(!s.due(ms(100)));
        // But force succeeds.
        let t = s.begin(ms(100), true).unwrap();
        s.finish(t, true, ms(150));
        assert!(!s.dirty());
    }

    // ── complex scenario: interleaved resets and saves ───────────────

    #[test]
    fn interleaved_reset_and_save() {
        let mut s = Autosave::new();

        // Document A: edit, start save.
        s.edited(ms(0));
        let ta = s.begin(ms(750), false).unwrap();

        // Switch to document B.
        s.reset();
        s.edited(ms(1000));

        // Document A save arrives — stale, ignored.
        s.finish(ta, true, ms(1100));
        assert!(s.dirty());
        assert_eq!(s.revision(), Revision(1));
        assert_eq!(s.saved_revision(), Revision(0));

        // Document B saves normally.
        let tb = s.begin(ms(1750), false).unwrap();
        s.finish(tb, true, ms(1800));
        assert!(!s.dirty());
    }

    // ── complex scenario: failure then mark_clean then edit ─────────

    #[test]
    fn failure_then_clean_then_edit() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t = s.begin(ms(750), false).unwrap();
        s.finish(t, false, ms(800));
        assert!(s.dirty());

        // User undoes everything.
        s.mark_clean();
        assert!(!s.dirty());

        // New edits — should work with no residual backoff.
        s.edited(ms(1000));
        assert!(s.due(ms(1750)));
    }

    // ── complex scenario: multiple in-flight edit bursts ────────────

    #[test]
    fn multiple_inflight_edit_windows() {
        let mut s = Autosave::new();
        s.edited(ms(0));
        let t1 = s.begin(ms(750), false).unwrap();

        // First burst during save.
        s.edited(ms(800));
        s.edited(ms(850));
        // Second burst during save.
        s.edited(ms(900));

        s.finish(t1, true, ms(1000));
        assert!(s.dirty());
        // The "unsaved_since_flight" is the first edit during save = 800.
        // Due at 800 + 750 = 1550.
        assert!(!s.due(ms(1549)));
        assert!(s.due(ms(1550)));
    }
}
