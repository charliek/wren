//! Grouped edit history bounded by snapshot count and retained text capacity.
//!
//! Inserts within [`GROUP_TIMEOUT`] coalesce until a non-edit interaction breaks
//! the group. Time is injected for deterministic tests.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use iced::widget::text_editor::Cursor;

/// Maximum undo entries kept in memory.
const MAX_ENTRIES: usize = 200;

/// Combined String capacities in undo, redo, and the open group, excluding the
/// live document and temporary snapshots. Oversize snapshots are not retained.
const MAX_TEXT_BYTES: usize = 32 * 1024 * 1024;

/// Maximum gap between adjacent inserts that are still merged into one unit.
const GROUP_TIMEOUT: Duration = Duration::from_millis(1_000);

/// The kind of edit being recorded, controlling grouping behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// A single character was inserted (adjacent inserts may be merged).
    Insert,
    /// A multi-character paste was applied (always a standalone unit).
    Paste,
    /// A line break was inserted (always a standalone unit).
    Enter,
    /// A character or range was deleted (always a standalone unit).
    Delete,
    /// An arbitrary programmatic text replacement (always a standalone unit).
    Replace,
}

#[derive(Debug, Clone)]
struct Entry {
    text: String,
    cursor: Cursor,
}

#[derive(Debug)]
struct OpenGroup {
    before: Entry,
    last_at: Instant,
}

/// Bounded undo/redo history with grouped-insert support.
#[derive(Debug)]
pub struct History {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    open_group: Option<OpenGroup>,
}

impl History {
    pub fn new() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            open_group: None,
        }
    }

    /// Record the captured before-state of an edit that actually changed text.
    ///
    /// `text` and `cursor` describe the content **before** the edit.  `now` is
    /// injected so callers can drive time-based grouping in tests.
    ///
    /// For [`EditKind::Insert`], consecutive calls within [`GROUP_TIMEOUT`] do
    /// not create a new undo entry — the before-state of the first call in the
    /// group is preserved.  Every other kind always creates a fresh entry and
    /// closes any open group first.
    ///
    /// Every recorded edit invalidates redo. If a new unit's snapshot exceeds
    /// the text budget, history is cleared rather than letting undo skip it.
    /// Continuing a group retains only its original, budgeted snapshot.
    pub fn before_edit(&mut self, text: String, cursor: Cursor, kind: EditKind, now: Instant) {
        if kind == EditKind::Insert {
            if let Some(group) = &mut self.open_group {
                if now.saturating_duration_since(group.last_at) <= GROUP_TIMEOUT {
                    // Continuing the same typing burst — just refresh the timer.
                    group.last_at = now;
                    return;
                }
            }
            if text.capacity() > MAX_TEXT_BYTES {
                self.clear();
                return;
            }
            self.close_open_group();
            self.redo.clear();
            self.open_group = Some(OpenGroup {
                before: Entry { text, cursor },
                last_at: now,
            });
            self.enforce_limits();
        } else {
            if text.capacity() > MAX_TEXT_BYTES {
                self.clear();
                return;
            }
            // Non-groupable: close any open group and push this state directly.
            self.close_open_group();
            self.redo.clear();
            self.push_undo(Entry { text, cursor });
        }
    }

    /// Return the state to restore for undo, saving `current_text`/`cursor` to redo.
    ///
    /// Returns `None` if there is nothing to undo.
    pub fn undo(
        &mut self,
        current_text: String,
        current_cursor: Cursor,
    ) -> Option<(String, Cursor)> {
        // Finalize any open typing group before undoing.
        self.close_open_group();
        let entry = self.undo.pop_back()?;
        if current_text.capacity() > MAX_TEXT_BYTES {
            // Without this state, older redo entries would skip an undo step.
            self.redo.clear();
        } else {
            self.redo.push(Entry {
                text: current_text,
                cursor: current_cursor,
            });
            self.enforce_limits();
        }
        Some((entry.text, entry.cursor))
    }

    /// Return the state to restore for redo, saving `current_text`/`cursor` to undo.
    ///
    /// Returns `None` if there is nothing to redo.
    pub fn redo(
        &mut self,
        current_text: String,
        current_cursor: Cursor,
    ) -> Option<(String, Cursor)> {
        self.close_open_group();
        let entry = self.redo.pop()?;
        if current_text.capacity() > MAX_TEXT_BYTES {
            self.undo.clear();
        } else {
            self.push_undo(Entry {
                text: current_text,
                cursor: current_cursor,
            });
        }
        Some((entry.text, entry.cursor))
    }

    /// Clear all history.  Call when loading entirely new content.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open_group = None;
    }

    /// Whether there is at least one undo step available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.open_group.is_some()
    }

    /// Whether there is at least one redo step available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// End a typing burst without discarding undo or redo states.
    pub fn break_group(&mut self) {
        self.close_open_group();
    }

    fn close_open_group(&mut self) {
        if let Some(group) = self.open_group.take() {
            self.push_undo(group.before);
        }
    }

    fn push_undo(&mut self, entry: Entry) {
        self.undo.push_back(entry);
        self.enforce_limits();
    }

    fn retained_text_bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .map(|entry| entry.text.capacity())
            .sum::<usize>()
            + self
                .open_group
                .as_ref()
                .map_or(0, |group| group.before.text.capacity())
    }

    fn enforce_limits(&mut self) {
        while self.undo.len() + self.redo.len() + usize::from(self.open_group.is_some())
            > MAX_ENTRIES
            || self.retained_text_bytes() > MAX_TEXT_BYTES
        {
            // Keep the nearest states on each stack; evicting from the other
            // end would make undo or redo jump over a state.
            if self.undo.pop_front().is_none() && !self.redo.is_empty() {
                self.redo.remove(0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::text_editor::Position;

    fn at(offset_ms: u64) -> Instant {
        // Construct a reproducible instant by adding an offset to a fixed base.
        // Using `Instant::now()` once at test start avoids clock issues.
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now) + Duration::from_millis(offset_ms)
    }

    fn cur(line: usize, col: usize) -> Cursor {
        Cursor {
            position: Position { line, column: col },
            selection: None,
        }
    }

    fn cur_sel(line: usize, col: usize, sel_line: usize, sel_col: usize) -> Cursor {
        Cursor {
            position: Position { line, column: col },
            selection: Some(Position {
                line: sel_line,
                column: sel_col,
            }),
        }
    }

    // --- Basic undo/redo ---

    #[test]
    fn undo_on_empty_history_returns_none() {
        let mut h = History::new();
        assert!(h.undo("hello".into(), cur(0, 5)).is_none());
        assert!(!h.can_undo());
    }

    #[test]
    fn redo_on_empty_history_returns_none() {
        let mut h = History::new();
        assert!(h.redo("hello".into(), cur(0, 5)).is_none());
        assert!(!h.can_redo());
    }

    #[test]
    fn single_non_insert_edit_then_undo_restores_state() {
        let mut h = History::new();
        h.before_edit("hello".into(), cur(0, 5), EditKind::Delete, at(0));
        // Simulate: backspace removed 'o', content is now "hell", cursor at col 4
        let result = h.undo("hell".into(), cur(0, 4));
        let (text, cursor) = result.expect("should have undo");
        assert_eq!(text, "hello");
        assert_eq!(cursor, cur(0, 5));
    }

    #[test]
    fn undo_then_redo_round_trips() {
        let mut h = History::new();
        h.before_edit("".into(), cur(0, 0), EditKind::Paste, at(0));
        // After paste, content is "world", cursor at col 5
        let (text, cursor) = h.undo("world".into(), cur(0, 5)).expect("undo");
        assert_eq!(text, "");
        assert_eq!(cursor, cur(0, 0));

        let (text2, cursor2) = h.redo("".into(), cur(0, 0)).expect("redo");
        assert_eq!(text2, "world");
        assert_eq!(cursor2, cur(0, 5));
    }

    // --- Insert grouping ---

    #[test]
    fn adjacent_inserts_within_timeout_produce_one_undo_entry() {
        let mut h = History::new();
        h.before_edit("".into(), cur(0, 0), EditKind::Insert, at(0));
        h.before_edit("a".into(), cur(0, 1), EditKind::Insert, at(100));
        h.before_edit("ab".into(), cur(0, 2), EditKind::Insert, at(200));
        // Three inserts within 1 s → single undo entry at the initial state
        let (text, cursor) = h.undo("abc".into(), cur(0, 3)).expect("undo");
        assert_eq!(text, "");
        assert_eq!(cursor, cur(0, 0));
        // Nothing left to undo
        assert!(h.undo("".into(), cur(0, 0)).is_none());
    }

    #[test]
    fn insert_after_timeout_starts_new_undo_entry() {
        let mut h = History::new();
        h.before_edit("".into(), cur(0, 0), EditKind::Insert, at(0));
        h.before_edit("a".into(), cur(0, 1), EditKind::Insert, at(500));
        // 1_500 ms gap — exceeds GROUP_TIMEOUT (1 000 ms), new group
        h.before_edit("ab".into(), cur(0, 2), EditKind::Insert, at(1_500 + 1));
        h.before_edit("abc".into(), cur(0, 3), EditKind::Insert, at(2_000));

        // First undo restores to "ab" (before the second group)
        let (text, _) = h.undo("abcd".into(), cur(0, 4)).expect("undo 1");
        assert_eq!(text, "ab");

        // Second undo restores to "" (before the first group)
        let (text, _) = h.undo("ab".into(), cur(0, 2)).expect("undo 2");
        assert_eq!(text, "");

        assert!(h.undo("".into(), cur(0, 0)).is_none());
    }

    #[test]
    fn enter_is_always_standalone() {
        let mut h = History::new();
        h.before_edit("hello".into(), cur(0, 5), EditKind::Enter, at(0));
        // A subsequent insert starts its own group, not merged with Enter
        h.before_edit("hello\n".into(), cur(1, 0), EditKind::Insert, at(10));
        h.before_edit("hello\nw".into(), cur(1, 1), EditKind::Insert, at(20));

        // Undo the insert group ("hello\n" → "hello\nw...")
        let (text, _) = h.undo("hello\nworld".into(), cur(1, 5)).expect("undo 1");
        assert_eq!(text, "hello\n");

        // Undo the Enter
        let (text, _) = h.undo("hello\n".into(), cur(1, 0)).expect("undo 2");
        assert_eq!(text, "hello");

        assert!(h.undo("hello".into(), cur(0, 5)).is_none());
    }

    #[test]
    fn paste_is_always_standalone() {
        let mut h = History::new();
        h.before_edit("".into(), cur(0, 0), EditKind::Insert, at(0));
        h.before_edit("a".into(), cur(0, 1), EditKind::Insert, at(100));
        // Paste closes the insert group and is its own entry
        h.before_edit("ab".into(), cur(0, 2), EditKind::Paste, at(200));

        // Undo paste: restores state before paste
        let (text, _) = h
            .undo("ab paste text".into(), cur(0, 13))
            .expect("undo paste");
        assert_eq!(text, "ab");

        // Undo insert group: restores initial empty state
        let (text, _) = h.undo("ab".into(), cur(0, 2)).expect("undo inserts");
        assert_eq!(text, "");
    }

    // --- Redo invalidation ---

    #[test]
    fn new_edit_clears_redo_stack() {
        let mut h = History::new();
        h.before_edit("".into(), cur(0, 0), EditKind::Insert, at(0));
        h.undo("a".into(), cur(0, 1));
        // We now have a redo entry
        assert!(h.can_redo());
        // Any new edit must clear redo
        h.before_edit("".into(), cur(0, 0), EditKind::Insert, at(100));
        assert!(!h.can_redo());
    }

    #[test]
    fn multiple_undo_then_redo_sequences() {
        let mut h = History::new();
        h.before_edit("".into(), cur(0, 0), EditKind::Delete, at(0));
        h.before_edit("step1".into(), cur(0, 5), EditKind::Delete, at(100));
        h.before_edit("step2".into(), cur(0, 5), EditKind::Delete, at(200));

        // Undo all three
        h.undo("step3".into(), cur(0, 5)).unwrap();
        h.undo("step2".into(), cur(0, 5)).unwrap();
        h.undo("step1".into(), cur(0, 5)).unwrap();
        assert!(!h.can_undo());

        // Redo two steps
        let (t, _) = h.redo("".into(), cur(0, 0)).unwrap();
        assert_eq!(t, "step1");
        let (t, _) = h.redo("step1".into(), cur(0, 5)).unwrap();
        assert_eq!(t, "step2");
        assert!(h.can_redo()); // step3 still available

        // A new edit now clears redo
        h.before_edit("step2".into(), cur(0, 5), EditKind::Enter, at(300));
        assert!(!h.can_redo());
    }

    // --- Memory bound ---

    #[test]
    fn undo_stack_bounded_at_max_entries() {
        let mut h = History::new();
        for i in 0..MAX_ENTRIES + 50 {
            h.before_edit(
                format!("state {i}"),
                cur(0, i),
                EditKind::Delete,
                at(i as u64 * 10),
            );
        }
        // We can undo at most MAX_ENTRIES times
        let mut count = 0usize;
        let mut current_text = format!("state {}", MAX_ENTRIES + 50);
        while h.undo(current_text.clone(), cur(0, 0)).is_some() {
            count += 1;
            current_text = format!("undo_{count}");
        }
        assert_eq!(count, MAX_ENTRIES);
    }

    #[test]
    fn break_group_preserves_history_and_redo() {
        let mut h = History::new();
        h.before_edit("".into(), cur(0, 0), EditKind::Insert, at(0));
        h.break_group();
        h.before_edit("a".into(), cur(0, 0), EditKind::Insert, at(10));
        assert_eq!(h.undo("ba".into(), cur(0, 1)).unwrap().0, "a");
        h.break_group();
        assert!(h.can_redo());
        assert_eq!(h.undo("a".into(), cur(0, 1)).unwrap().0, "");
        assert_eq!(h.redo("".into(), cur(0, 0)).unwrap().0, "a");
        assert_eq!(h.redo("a".into(), cur(0, 1)).unwrap().0, "ba");
    }

    #[test]
    fn text_budget_includes_open_group_undo_and_redo() {
        let mut h = History::new();
        let size = MAX_TEXT_BYTES / 4;
        for i in 0..4 {
            h.before_edit("a".repeat(size), cur(0, i), EditKind::Paste, at(i as u64));
        }
        h.before_edit("b".repeat(size), cur(0, 4), EditKind::Insert, at(10));
        assert!(h.open_group.is_some());
        assert_eq!(h.undo.len(), 3);
        assert_eq!(h.retained_text_bytes(), MAX_TEXT_BYTES);
        for i in 0..3 {
            h.undo("c".repeat(size + 1), cur(0, i)).unwrap();
            assert!(h.retained_text_bytes() <= MAX_TEXT_BYTES);
        }
        assert!(!h.can_undo());
        assert_eq!(h.redo.len(), 3);
        while h.can_redo() {
            h.redo("d".repeat(size + 1), cur(0, 0)).unwrap();
            assert!(h.retained_text_bytes() <= MAX_TEXT_BYTES);
        }
    }

    #[test]
    fn oversize_snapshots_clear_history_instead_of_skipping_states() {
        for kind in [EditKind::Insert, EditKind::Paste] {
            let mut h = History::new();
            h.before_edit("a".into(), cur(0, 1), EditKind::Paste, at(0));
            h.before_edit("x".repeat(MAX_TEXT_BYTES + 1), cur(0, 1), kind, at(10));
            assert!(!h.can_undo());
            assert!(!h.can_redo());
            assert_eq!(h.retained_text_bytes(), 0);
        }
    }

    #[test]
    fn oversize_current_state_is_not_retained_on_undo_or_redo() {
        let mut h = History::new();
        h.before_edit("a".into(), cur(0, 1), EditKind::Paste, at(0));
        assert_eq!(
            h.undo("x".repeat(MAX_TEXT_BYTES + 1), cur(0, 0)).unwrap().0,
            "a"
        );
        assert!(!h.can_redo());
        h.before_edit("b".into(), cur(0, 1), EditKind::Paste, at(10));
        h.undo("c".into(), cur(0, 1)).unwrap();
        assert_eq!(
            h.redo("x".repeat(MAX_TEXT_BYTES + 1), cur(0, 0)).unwrap().0,
            "c"
        );
        assert!(!h.can_undo());
        assert_eq!(h.retained_text_bytes(), 0);
    }

    #[test]
    fn text_budget_counts_string_capacity_not_just_length() {
        let mut h = History::new();
        let mut text = String::with_capacity(MAX_TEXT_BYTES + 1);
        text.push('a');
        h.before_edit(text, cur(0, 1), EditKind::Paste, at(0));
        assert!(!h.can_undo());
        assert_eq!(h.retained_text_bytes(), 0);
    }

    #[test]
    fn entry_limit_includes_the_open_group() {
        let mut h = History::new();
        for i in 0..MAX_ENTRIES {
            h.before_edit(format!("{i}"), cur(0, 0), EditKind::Paste, at(i as u64));
        }
        h.before_edit("last".into(), cur(0, 0), EditKind::Insert, at(300));
        assert_eq!(h.undo.len(), MAX_ENTRIES - 1);
        assert!(h.open_group.is_some());
    }

    // --- Cursor with selection preservation ---

    #[test]
    fn cursor_with_selection_is_preserved_through_undo() {
        let mut h = History::new();
        let before_cursor = cur_sel(1, 10, 0, 5);
        h.before_edit("multi\nline".into(), before_cursor, EditKind::Delete, at(0));
        let (_, restored_cursor) = h.undo("line".into(), cur(0, 4)).expect("undo");
        assert_eq!(restored_cursor, before_cursor);
    }

    // --- Clear ---

    #[test]
    fn clear_wipes_undo_redo_and_open_group() {
        let mut h = History::new();
        h.before_edit("a".into(), cur(0, 1), EditKind::Insert, at(0));
        h.undo("ab".into(), cur(0, 2));
        h.clear();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
        assert!(h.undo("any".into(), cur(0, 3)).is_none());
    }

    // --- Unicode line/column storage ---

    #[test]
    fn emoji_cursor_positions_round_trip_through_history() {
        // Position columns are byte offsets; "👋" is 4 bytes.
        let emoji_col = "👋 hello".len(); // 4 + 1 + 5 = 10
        let before = cur(0, emoji_col);
        let mut h = History::new();
        h.before_edit("👋 hell".into(), before, EditKind::Insert, at(0));
        let (text, cursor) = h
            .undo("👋 hello".into(), cur(0, emoji_col + 1))
            .expect("undo");
        assert_eq!(text, "👋 hell");
        assert_eq!(cursor.position.column, emoji_col);
    }
}
