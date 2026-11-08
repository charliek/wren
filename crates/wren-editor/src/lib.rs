//! Markdown editor with grouped undo/redo and Unicode-lowercase literal find.
//!
//! Cursor columns and find ranges use UTF-8 byte offsets. Ordinary interactions
//! retain the iced Content; snapshot restoration replaces it. Stock iced does
//! not expose an absolute viewport offset, so undo/redo cannot restore scroll.
//!
//! The crate knows nothing about its host: colours arrive as a [`Style`] and a
//! [`Palette`], and the font through [`EditorView::set_font`].

mod backend;
mod find;
mod highlighter;
mod history;
mod keys;
mod widget;

#[cfg(test)]
mod fixtures;

use std::ops::Range;
use std::time::Instant;

use iced::widget::text_editor::{self, Action, Cursor, Edit, Motion, Position};
use iced::{Color, Element, Font};

use backend::Lines;
use find::{
    byte_offset_to_position, exact_byte_offset_to_position, find_all_matches, first_content_match,
};
use highlighter::Settings as HighlighterSettings;
use history::{EditKind, History};
use widget::{Content, TextEditor};

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

/// Colours of the editor surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub background: Color,
    /// Unhighlighted text.
    pub value: Color,
    pub selection: Color,
    /// Line numbers.
    pub gutter: Color,
    /// The band behind the cursor's line.
    pub current_line: Color,
    pub caret: Color,
    /// The line through a completed task.
    pub strike: Color,
}

/// Colours of the Markdown syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Heading and emphasis text.
    pub text: Color,
    /// Code and blockquotes.
    pub muted: Color,
    /// Markup characters, fences, comments, and completed tasks.
    pub faint: Color,
    /// The first-level heading, list markers, and links.
    pub accent: Color,
    /// Strikethrough.
    pub danger: Color,
    /// Inline code.
    pub success: Color,
}

// ---------------------------------------------------------------------------
// Public message type
// ---------------------------------------------------------------------------

/// Messages produced by the [`EditorView`] widget.
#[derive(Debug, Clone)]
pub enum Message {
    /// An iced text-editor action (typing, cursor motion, scroll, mouse, IME).
    Action(text_editor::Action),
    /// Undo the last edit unit (Ctrl/Cmd+Z).
    Undo,
    /// Redo the next edit unit (Ctrl/Cmd+Shift+Z, Linux Ctrl+Y).
    Redo,
    /// Open the find bar (Ctrl/Cmd+F).  The parent owns the bar UI and focus.
    Find,
}

// ---------------------------------------------------------------------------
// EditorView
// ---------------------------------------------------------------------------

/// A self-contained Markdown editor backed by a fork of iced's text editor
/// that adds the mockup's line-number gutter, current-line band, and caret.
///
/// Owns the [`Content`], the [`History`], and the find state.  The parent is
/// responsible for the find bar UI and focus tasks; it shows the bar in
/// response to [`Message::Find`] and feeds queries via
/// [`EditorView::set_find_query`].
pub struct EditorView {
    content: Content,
    history: History,
    find_query: String,
    /// Byte `(start, end)` pairs, refreshed whenever the query or text changes.
    find_matches: Vec<(usize, usize)>,
    /// Index into `find_matches` of the currently selected hit.
    find_current: Option<usize>,
    line_numbers: bool,
    metrics: widget::Metrics,
    font: Font,
}

impl EditorView {
    /// Create a new editor pre-populated with `text`.
    pub fn new(text: &str) -> Self {
        Self {
            content: Content::with_text(text),
            history: History::new(),
            find_query: String::new(),
            find_matches: Vec::new(),
            find_current: None,
            line_numbers: true,
            metrics: widget::Metrics::default(),
            font: Font::MONOSPACE,
        }
    }

    /// Editor text size in points; row and caret heights scale with it.
    pub fn set_font_size(&mut self, size: f32) {
        self.metrics = widget::Metrics::for_size(size);
    }

    /// The editor font; bold and italic spans use its variants. Defaults to
    /// [`Font::MONOSPACE`].
    pub fn set_font(&mut self, font: Font) {
        self.font = font;
    }

    #[cfg(feature = "test-support")]
    pub fn font(&self) -> Font {
        self.font
    }

    #[cfg(feature = "test-support")]
    pub fn font_size(&self) -> f32 {
        self.metrics.text
    }

    /// Show or hide the line-number gutter (shown by default).
    pub fn set_line_numbers(&mut self, line_numbers: bool) {
        self.line_numbers = line_numbers;
    }

    // -----------------------------------------------------------------------
    // View
    // -----------------------------------------------------------------------

    /// Build the iced widget tree for the editor.
    ///
    /// Uses the editor font, 14 px / 24 px line height scaled with the text
    /// size, soft-wrap, a 52 px line-number gutter unless hidden, `style` for
    /// the surface, and Markdown highlighting in `palette` colours.  Returns
    /// `Element<'_, Message>` so the parent can map it to its own message type.
    pub fn view(&self, style: Style, palette: Palette) -> Element<'_, Message> {
        TextEditor::new(
            &self.content,
            "note-editor",
            HighlighterSettings {
                palette,
                font: self.font,
            },
            style,
            self.line_numbers,
            self.metrics,
        )
        .into()
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    /// Apply `action` to the content.
    ///
    /// Returns `true` only when the text changes. Non-edit actions break typing
    /// coalescing without discarding history; no-op edits also preserve redo.
    pub fn perform(&mut self, action: Action) -> bool {
        if action.is_edit() {
            let kind = edit_kind(&action);
            let now = Instant::now();
            let before_text = self.content.text();
            let before_cursor = self.snapshot_cursor();
            self.content.perform(action);
            if self.content.text() == before_text {
                self.history.break_group();
                return false;
            }
            self.history
                .before_edit(before_text, before_cursor, kind, now);
            self.recompute_find();
            true
        } else {
            self.history.break_group();
            self.content.perform(action);
            false
        }
    }

    /// Undo the last edit unit.
    ///
    /// Returns `true` if something was restored.  After undo, the content is
    /// replaced with the saved snapshot; scroll position is not restored (stock
    /// iced does not expose an absolute viewport offset).
    pub fn undo(&mut self) -> bool {
        if !self.history.can_undo() {
            return false;
        }
        let current_text = self.content.text();
        let current_cursor = self.snapshot_cursor();
        if let Some((text, cursor)) = self.history.undo(current_text, current_cursor) {
            self.load_snapshot(text, cursor);
            true
        } else {
            false
        }
    }

    /// Redo the next edit unit.
    ///
    /// Returns `true` if something was restored.
    pub fn redo(&mut self) -> bool {
        if !self.history.can_redo() {
            return false;
        }
        let current_text = self.content.text();
        let current_cursor = self.snapshot_cursor();
        if let Some((text, cursor)) = self.history.redo(current_text, current_cursor) {
            self.load_snapshot(text, cursor);
            true
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Content accessors
    // -----------------------------------------------------------------------

    /// Return the full document text.
    pub fn text(&self) -> String {
        self.content.text()
    }

    /// Return the current cursor (position + optional selection anchor).
    ///
    /// Column values are **byte offsets** into the UTF-8 line text.
    pub fn cursor(&self) -> Cursor {
        self.content.cursor()
    }

    /// Return the currently selected text, or `None` if there is no selection.
    #[cfg(any(test, feature = "test-support"))]
    pub fn selected_text(&self) -> Option<String> {
        self.content.selection()
    }

    /// Return the number of lines in the document.
    pub fn line_count(&self) -> usize {
        self.content.line_count()
    }

    // -----------------------------------------------------------------------
    // Cursor control
    // -----------------------------------------------------------------------

    /// Move the cursor to `cursor`, clamping to valid byte positions.
    ///
    /// Ends the current typing group without discarding history.
    pub fn move_cursor(&mut self, cursor: Cursor) {
        self.history.break_group();
        let clamped = self.clamped(cursor);
        if clamped.selection.is_none() && self.content.cursor().selection.is_some() {
            // iced's move_to leaves the old selection intact when passed None.
            self.content.perform(Action::Move(Motion::Left));
        }
        self.content.move_to(clamped);
    }

    /// Select a range in the current document's original UTF-8 bytes without
    /// replacing Content, changing find state, or recording an edit.
    /// Invalid ranges leave the cursor and selection untouched. Cached offsets
    /// still require the caller to validate the document's content revision.
    pub fn select_byte_range(&mut self, range: Range<usize>) -> bool {
        if range.start > range.end {
            return false;
        }
        let Some((start, end)) = self.content.with_lines(|lines| {
            Some((
                exact_byte_offset_to_position(lines, range.start)?,
                exact_byte_offset_to_position(lines, range.end)?,
            ))
        }) else {
            return false;
        };
        self.move_cursor(Cursor {
            position: end,
            selection: (range.start != range.end).then_some(start),
        });
        true
    }

    /// Reveal the first literal hit in the actual loaded text. Workspace search
    /// must recheck here after its transition barrier rather than trust an old
    /// index offset. The user's independent find-in-note state stays unchanged.
    pub fn reveal_literal(&mut self, query: &str) -> bool {
        let Some(range) = self
            .content
            .with_lines(|lines| first_content_match(lines, query))
        else {
            return false;
        };
        self.select_byte_range(range)
    }

    // -----------------------------------------------------------------------
    // Content replacement
    // -----------------------------------------------------------------------

    /// Replace the entire document with `text`.
    ///
    /// If `preserve_cursor` is `true`, the current cursor is clamped and
    /// restored.  History is always cleared because the entire document changes.
    pub fn replace_text(&mut self, text: &str, preserve_cursor: bool) {
        let saved_cursor = if preserve_cursor {
            Some(self.content.cursor())
        } else {
            None
        };
        self.content = Content::with_text(text);
        if let Some(cursor) = saved_cursor {
            let clamped = self.clamped(cursor);
            self.content.move_to(clamped);
        }
        self.history.clear();
        self.find_current = None;
        self.recompute_find();
    }

    // -----------------------------------------------------------------------
    // Find
    // -----------------------------------------------------------------------

    /// Set the find query and recompute matches.
    ///
    /// Pass an empty string to clear the query and all matches.
    pub fn set_find_query(&mut self, query: &str) {
        self.history.break_group();
        self.find_query = query.to_owned();
        self.find_current = None;
        self.recompute_find();
    }

    /// Move to the next match, wrapping from the last to the first.
    ///
    /// Returns `true` if at least one match exists.
    pub fn find_next(&mut self) -> bool {
        self.history.break_group();
        if self.find_matches.is_empty() {
            return false;
        }
        let next = match self.find_current {
            None => 0,
            Some(i) => (i + 1) % self.find_matches.len(),
        };
        self.find_current = Some(next);
        self.navigate_to_match(next);
        true
    }

    /// Move to the previous match, wrapping from the first to the last.
    ///
    /// Returns `true` if at least one match exists.
    pub fn find_previous(&mut self) -> bool {
        self.history.break_group();
        if self.find_matches.is_empty() {
            return false;
        }
        let prev = match self.find_current {
            None | Some(0) => self.find_matches.len() - 1,
            Some(i) => i - 1,
        };
        self.find_current = Some(prev);
        self.navigate_to_match(prev);
        true
    }

    /// Total number of matches for the current query.
    pub fn find_count(&self) -> usize {
        self.find_matches.len()
    }

    /// 0-based index of the currently highlighted match, or `None`.
    pub fn find_current(&self) -> Option<usize> {
        self.find_current
    }

    // -----------------------------------------------------------------------
    // History introspection
    // -----------------------------------------------------------------------

    /// End a typing burst without discarding history. The parent should call
    /// this when an interaction it owns moves focus away, such as opening find.
    pub fn break_group(&mut self) {
        self.history.break_group();
    }

    /// Whether the undo stack has any entries.
    #[cfg(any(test, feature = "test-support"))]
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether the redo stack has any entries.
    #[cfg(any(test, feature = "test-support"))]
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn snapshot_cursor(&mut self) -> Cursor {
        let raw = self.content.cursor();
        let Some(selected) = self.content.selection().filter(|text| !text.is_empty()) else {
            return raw;
        };
        let backward = raw.selection.is_some_and(|anchor| {
            (raw.position.line, raw.position.column) < (anchor.line, anchor.column)
        });

        // Word/line selections expose raw click positions, not their bounds.
        // Left collapses to the actual start and clears the native selection mode.
        self.content.perform(Action::Move(Motion::Left));
        let start = self.content.cursor().position;
        let mut end = start;
        // cosmic-text copies every line ending as LF, irrespective of document bytes.
        for (line, text) in selected.split('\n').enumerate() {
            if line == 0 {
                end.column += text.len();
            } else {
                end.line += 1;
                end.column = text.len();
            }
        }
        let cursor = self.clamped(Cursor {
            position: if backward { start } else { end },
            selection: Some(if backward { end } else { start }),
        });
        self.content.move_to(cursor);
        cursor
    }

    /// Replace content with a history snapshot and restore cursor.
    fn load_snapshot(&mut self, text: String, cursor: Cursor) {
        self.content = Content::with_text(&text);
        let clamped = self.clamped(cursor);
        self.content.move_to(clamped);
        self.find_current = None;
        self.recompute_find();
    }

    fn clamped(&self, cursor: Cursor) -> Cursor {
        self.content.with_lines(|lines| clamp_cursor(cursor, lines))
    }

    fn recompute_find(&mut self) {
        self.find_matches = if self.find_query.is_empty() {
            Vec::new()
        } else {
            find_all_matches(&self.content.text(), &self.find_query)
        };
        self.find_current = self
            .find_current
            .filter(|&index| index < self.find_matches.len());
    }

    fn navigate_to_match(&mut self, index: usize) {
        let Some(&(byte_start, byte_end)) = self.find_matches.get(index) else {
            return;
        };
        let (start, end) = self.content.with_lines(|lines| {
            (
                byte_offset_to_position(lines, byte_start),
                byte_offset_to_position(lines, byte_end),
            )
        });
        // Cursor at end, selection anchor at start → visually selects the match.
        self.content.move_to(Cursor {
            position: end,
            selection: Some(start),
        });
    }
}

// ---------------------------------------------------------------------------
// Cursor clamping
// ---------------------------------------------------------------------------

fn clamp_cursor(cursor: Cursor, lines: Lines<'_>) -> Cursor {
    Cursor {
        position: clamp_position(cursor.position, lines),
        selection: cursor.selection.map(|s| clamp_position(s, lines)),
    }
}

pub(crate) fn clamp_position(pos: Position, lines: Lines<'_>) -> Position {
    let line = pos.line.min(lines.len().max(1) - 1);
    let column = lines.get(line).map_or(0, |line| {
        let mut column = pos.column.min(line.text.len());
        while !line.text.is_char_boundary(column) {
            column -= 1;
        }
        column
    });
    Position { line, column }
}

// ---------------------------------------------------------------------------
// Edit-kind classification
// ---------------------------------------------------------------------------

fn edit_kind(action: &Action) -> EditKind {
    match action {
        Action::Edit(Edit::Insert(_)) => EditKind::Insert,
        Action::Edit(Edit::Paste(_)) => EditKind::Paste,
        Action::Edit(Edit::Enter) => EditKind::Enter,
        Action::Edit(Edit::Backspace) | Action::Edit(Edit::Delete) => EditKind::Delete,
        Action::Edit(Edit::Indent) | Action::Edit(Edit::Unindent) => EditKind::Replace,
        _ => EditKind::Replace,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    // Helper: create an EditorView with pre-loaded text.
    fn editor(text: &str) -> EditorView {
        EditorView::new(text)
    }

    // Helper: simulate typing a string character by character, returning the
    // number of history entries that would be produced (each is one insert group).
    fn type_chars(ev: &mut EditorView, s: &str) {
        for c in s.chars() {
            ev.perform(Action::Edit(Edit::Insert(c)));
        }
    }

    #[test]
    fn workspace_hit_reveal_preserves_independent_find_and_edit_history() {
        let mut ev = editor("keep CAFÉ 🐦 İ");
        ev.perform(Action::Move(Motion::DocumentEnd));
        type_chars(&mut ev, "!");
        ev.set_find_query("keep");
        assert!(ev.find_next());
        let find = (
            ev.find_query.clone(),
            ev.find_matches.clone(),
            ev.find_current,
        );
        assert!(ev.reveal_literal("i\u{0307}"));
        assert_eq!(ev.selected_text().as_deref(), Some("İ"));
        assert_eq!(
            (
                ev.find_query.clone(),
                ev.find_matches.clone(),
                ev.find_current
            ),
            find
        );
        assert!(ev.can_undo());
        assert!(ev.undo());
        assert_eq!(ev.text(), "keep CAFÉ 🐦 İ");
        assert!(ev.can_redo());
        assert!(ev.reveal_literal("café"));
        assert_eq!(ev.selected_text().as_deref(), Some("CAFÉ"));
        assert!(ev.can_redo());
        assert!(ev.redo());
        assert_eq!(ev.text(), "keep CAFÉ 🐦 İ!");
    }

    #[test]
    fn workspace_hit_reveal_handles_each_line_ending_without_rewriting_text() {
        for ending in ["\n", "\r", "\r\n", "\n\r"] {
            let original = format!("first{ending}CAFÉ 🐦 İ{ending}last");
            let mut ev = editor(&original);
            assert!(ev.reveal_literal(&format!("🐦 İ{ending}last")));
            assert_eq!(ev.selected_text().as_deref(), Some("🐦 İ\nlast"));
            assert_eq!(ev.cursor().position.line, 2);
            assert_eq!(ev.cursor().position.column, 4);
            assert_eq!(ev.text(), original);
            assert!(!ev.can_undo());
        }
        let mut ev = editor("first\rsecond\n\rthird\r\nlast");
        assert!(ev.reveal_literal("second\n\rthird\r\nlast"));
        assert_eq!(ev.selected_text().as_deref(), Some("second\nthird\nlast"));
    }

    #[test]
    fn exact_ranges_reject_endpoints_inside_paired_line_endings() {
        for ending in ["\r\n", "\n\r"] {
            let original = format!("a{ending}b");
            let mut ev = editor(&original);
            ev.move_cursor(Cursor {
                position: Position { line: 1, column: 1 },
                selection: None,
            });
            let before = ev.cursor();
            for range in [1..2, 2..3, 2..4, 0..2, 2..2] {
                assert!(
                    !ev.select_byte_range(range),
                    "accepted an unrepresentable {ending:?} endpoint"
                );
                assert_eq!(ev.cursor(), before);
            }
            assert!(!ev.reveal_literal(&format!("{}b", &ending[1..])));
            assert_eq!(ev.cursor(), before);
            assert!(ev.select_byte_range(1..3));
            assert_eq!(ev.selected_text().as_deref(), Some("\n"));
            assert!(ev.reveal_literal(&format!("{ending}b")));
            assert_eq!(ev.selected_text().as_deref(), Some("\nb"));
            assert_eq!(ev.text(), original);
            assert!(!ev.can_undo());
        }
    }

    #[test]
    fn stale_or_invalid_workspace_ranges_do_not_change_editor_state() {
        let mut ev = editor("🐦 e\u{0301} text");
        ev.set_find_query("text");
        ev.find_next();
        let cursor = ev.cursor();
        let selected = ev.selected_text();
        let length = ev.text().len();
        for range in [
            1..4,
            0..2,
            6..7,
            length..length + 1,
            Range { start: 5, end: 2 },
        ] {
            assert!(!ev.select_byte_range(range));
            assert_eq!(ev.cursor(), cursor);
            assert_eq!(ev.selected_text(), selected);
            assert_eq!(ev.find_query, "text");
        }
        assert!(!ev.reveal_literal("not present"));
        assert!(!ev.reveal_literal(""));
        assert_eq!(ev.cursor(), cursor);
        assert!(ev.select_byte_range(length..length));
        assert_eq!(ev.selected_text(), None);
        assert!(!ev.can_undo());
    }

    // ── Basic content ────────────────────────────────────────────────────────

    #[test]
    fn new_editor_contains_initial_text() {
        let ev = editor("Hello, world!");
        assert_eq!(ev.text(), "Hello, world!");
    }

    #[test]
    fn line_count_is_accurate() {
        let ev = editor("line1\nline2\nline3");
        assert_eq!(ev.line_count(), 3);
    }

    #[test]
    fn selected_text_none_on_fresh_editor() {
        let ev = editor("some text");
        assert!(ev.selected_text().is_none());
    }

    // ── perform() ────────────────────────────────────────────────────────────

    #[test]
    fn insert_action_returns_true() {
        let mut ev = editor("");
        assert!(ev.perform(Action::Edit(Edit::Insert('a'))));
    }

    #[test]
    fn move_action_returns_false() {
        let mut ev = editor("hello");
        assert!(!ev.perform(Action::Move(iced::widget::text_editor::Motion::Right)));
    }

    #[test]
    fn insert_adds_character_to_content() {
        let mut ev = editor("hello");
        // cursor starts at (0,0); Move to end first
        ev.perform(Action::Move(iced::widget::text_editor::Motion::DocumentEnd));
        ev.perform(Action::Edit(Edit::Insert('!')));
        assert_eq!(ev.text(), "hello!");
    }

    #[test]
    fn enter_inserts_newline() {
        let mut ev = editor("ab");
        ev.perform(Action::Move(iced::widget::text_editor::Motion::End));
        ev.perform(Action::Edit(Edit::Enter));
        assert_eq!(ev.line_count(), 2);
    }

    #[test]
    fn paste_inserts_text() {
        use std::sync::Arc;
        let mut ev = editor("");
        ev.perform(Action::Edit(Edit::Paste(Arc::new("pasted".to_owned()))));
        assert_eq!(ev.text(), "pasted");
    }

    // ── Undo / Redo ───────────────────────────────────────────────────────────

    #[test]
    fn undo_on_empty_history_returns_false() {
        let mut ev = editor("hello");
        assert!(!ev.undo());
    }

    #[test]
    fn redo_on_empty_history_returns_false() {
        let mut ev = editor("hello");
        assert!(!ev.redo());
    }

    #[test]
    fn single_edit_then_undo_restores_original_text() {
        let mut ev = editor("hello");
        ev.perform(Action::Move(text_editor::Motion::DocumentEnd));
        ev.perform(Action::Edit(Edit::Backspace));
        assert!(ev.undo());
        assert_eq!(ev.text(), "hello");
    }

    #[test]
    fn undo_then_redo_restores_edited_text() {
        let mut ev = editor("");
        type_chars(&mut ev, "hi");
        // Undo the entire "hi" group
        ev.undo();
        assert_eq!(ev.text(), "");
        ev.redo();
        assert_eq!(ev.text(), "hi");
    }

    #[test]
    fn new_edit_invalidates_redo_stack() {
        let mut ev = editor("hello");
        ev.perform(Action::Move(text_editor::Motion::DocumentEnd));
        ev.perform(Action::Edit(Edit::Backspace));
        ev.undo();
        assert!(ev.can_redo());
        // Type something new
        ev.perform(Action::Edit(Edit::Insert('x')));
        assert!(!ev.can_redo());
    }

    #[test]
    fn multiple_undo_redo_cycles() {
        let mut ev = editor("");
        // Build up three standalone edits
        ev.perform(Action::Edit(Edit::Enter));
        ev.perform(Action::Edit(Edit::Enter));
        ev.perform(Action::Edit(Edit::Enter));

        assert_eq!(ev.line_count(), 4);
        ev.undo();
        assert_eq!(ev.line_count(), 3);
        ev.undo();
        assert_eq!(ev.line_count(), 2);
        ev.undo();
        assert_eq!(ev.line_count(), 1);
        assert!(!ev.undo()); // empty

        ev.redo();
        assert_eq!(ev.line_count(), 2);
    }

    #[test]
    fn non_edit_action_does_not_dirty_history() {
        let mut ev = editor("hello");
        assert!(!ev.can_undo());
        ev.perform(Action::Move(iced::widget::text_editor::Motion::Right));
        ev.perform(Action::SelectAll);
        assert!(!ev.can_undo());
    }

    #[test]
    fn word_selection_undo_restores_range_for_follow_up_typing() {
        for (text, column, word) in [
            ("hello world", 3, "hello"),
            ("café world", 3, "café"),
            ("one κόσμε two", "one κό".len(), "κόσμε"),
        ] {
            let mut ev = editor(text);
            let click = Position { line: 0, column };
            ev.move_cursor(Cursor {
                position: click,
                selection: None,
            });
            ev.content.perform(Action::SelectWord);
            assert_eq!(ev.cursor().position, click);
            assert_eq!(ev.cursor().selection, Some(click));
            assert_eq!(ev.selected_text().as_deref(), Some(word));

            assert!(ev.perform(Action::Edit(Edit::Backspace)));
            assert_eq!(ev.text(), text.replacen(word, "", 1));
            assert!(ev.undo());
            let start = text.find(word).unwrap();
            assert_eq!(
                ev.cursor(),
                Cursor {
                    position: Position {
                        line: 0,
                        column: start + word.len(),
                    },
                    selection: Some(Position {
                        line: 0,
                        column: start,
                    }),
                }
            );
            assert_eq!(ev.text(), text);
            assert_eq!(ev.selected_text().as_deref(), Some(word));
            assert!(ev.redo());
            assert_eq!(ev.text(), text.replacen(word, "", 1));
            assert!(ev.undo());
            type_chars(&mut ev, "XY");
            assert_eq!(ev.text(), text.replacen(word, "XY", 1));
            assert!(ev.undo());
            assert_eq!(ev.text(), text);
            assert_eq!(ev.selected_text().as_deref(), Some(word));
            assert!(!ev.can_undo());
        }
    }

    #[test]
    fn line_selection_undo_restores_range_for_follow_up_typing() {
        for ending in ["\n", "\r\n", "\r", "\n\r"] {
            let line = "héllo 👋 world";
            let text = format!("before{ending}{line}{ending}after");
            let mut ev = editor(&text);
            let click = Position {
                line: 1,
                column: "hé".len(),
            };
            ev.move_cursor(Cursor {
                position: click,
                selection: None,
            });
            ev.content.perform(Action::SelectLine);
            assert_eq!(ev.cursor().position, click);
            assert_eq!(ev.cursor().selection, Some(click));
            assert_eq!(ev.selected_text().as_deref(), Some(line));

            assert!(ev.perform(Action::Edit(Edit::Backspace)));
            assert_eq!(ev.text(), format!("before{ending}{ending}after"));
            assert!(ev.undo());
            assert_eq!(ev.text(), text);
            assert_eq!(ev.selected_text().as_deref(), Some(line));
            assert_eq!(
                ev.cursor(),
                Cursor {
                    position: Position {
                        line: 1,
                        column: line.len(),
                    },
                    selection: Some(Position { line: 1, column: 0 }),
                }
            );
            assert!(ev.redo());
            assert_eq!(ev.text(), format!("before{ending}{ending}after"));
            assert!(ev.undo());
            type_chars(&mut ev, "XY");
            assert_eq!(ev.text(), format!("before{ending}XY{ending}after"));
            assert!(ev.undo());
            assert_eq!(ev.text(), text);
            assert_eq!(ev.selected_text().as_deref(), Some(line));
            assert!(!ev.can_undo());
        }
    }

    #[test]
    fn drag_selection_undo_preserves_range_and_direction_for_each_line_ending() {
        for ending in ["\n", "\r\n", "\r", "\n\r"] {
            let text = format!("héllo{ending}middle{ending}κόσμε");
            for backward in [false, true] {
                for mode in [None, Some(Action::SelectWord), Some(Action::SelectLine)] {
                    let mut ev = editor(&text);
                    let start = Position { line: 0, column: 0 };
                    let end = Position {
                        line: 2,
                        column: "κόσμε".len(),
                    };
                    let expected = Cursor {
                        position: if backward { start } else { end },
                        selection: Some(if backward { end } else { start }),
                    };
                    let mut anchor = expected.selection.unwrap();
                    if mode.is_some() {
                        anchor.column = if backward { "κό".len() } else { "hé".len() };
                    }
                    ev.move_cursor(Cursor {
                        position: anchor,
                        selection: None,
                    });
                    if let Some(action) = mode {
                        ev.content.perform(action);
                    }
                    // Hit-test beyond the document's bounds, independently of font metrics.
                    let coordinate = if backward { -10_000.0 } else { 10_000.0 };
                    ev.content
                        .perform(Action::Drag(iced::Point::new(coordinate, coordinate)));
                    assert_eq!(ev.cursor().position, expected.position);
                    assert_eq!(ev.cursor().selection, Some(anchor));
                    assert_eq!(ev.selected_text().as_deref(), Some("héllo\nmiddle\nκόσμε"));

                    assert!(ev.perform(Action::Edit(Edit::Backspace)));
                    assert_eq!(ev.text(), "");
                    assert!(ev.undo());
                    assert_eq!(ev.text(), text);
                    assert_eq!(ev.cursor(), expected);
                    assert_eq!(ev.selected_text().as_deref(), Some("héllo\nmiddle\nκόσμε"));
                    type_chars(&mut ev, "XY");
                    assert_eq!(ev.text(), "XY");
                    assert!(ev.undo());
                    assert_eq!(ev.text(), text);
                    assert_eq!(ev.cursor(), expected);
                    assert!(!ev.can_undo());
                }
            }
        }
    }

    #[test]
    fn history_navigation_preserves_native_selection_bounds() {
        for action in [Action::SelectWord, Action::SelectLine] {
            for saving_undo in [false, true] {
                let mut ev = editor("hello world");
                ev.perform(Action::Move(text_editor::Motion::DocumentEnd));
                ev.perform(Action::Edit(Edit::Insert('!')));
                if saving_undo {
                    assert!(ev.undo());
                }
                ev.move_cursor(Cursor {
                    position: Position { line: 0, column: 3 },
                    selection: None,
                });
                ev.content.perform(action.clone());
                let text = ev.text();
                let selected = ev.selected_text().unwrap();
                if saving_undo {
                    assert!(ev.redo());
                    assert!(ev.undo());
                } else {
                    assert!(ev.undo());
                    assert!(ev.redo());
                }
                assert_eq!(ev.text(), text);
                assert_eq!(ev.selected_text(), Some(selected.clone()));
                assert_eq!(ev.cursor().position.column, selected.len());
                assert_eq!(ev.cursor().selection, Some(Position { line: 0, column: 0 }));
                assert!(ev.perform(Action::Edit(Edit::Insert('X'))));
                assert_eq!(ev.text(), text.replacen(&selected, "X", 1));
            }
        }
    }

    #[test]
    fn non_edit_actions_preserve_native_selection_modes() {
        for action in [Action::SelectWord, Action::SelectLine] {
            let mut ev = editor("hello world");
            let raw = Cursor {
                position: Position { line: 0, column: 3 },
                selection: Some(Position { line: 0, column: 3 }),
            };
            ev.move_cursor(Cursor {
                selection: None,
                ..raw
            });
            ev.perform(action);
            let selected = ev.selected_text();
            ev.perform(Action::Scroll { lines: 0 });
            assert!(!ev.undo());
            assert!(!ev.redo());
            assert_eq!(ev.cursor(), raw);
            assert_eq!(ev.selected_text(), selected);
            assert!(!ev.can_undo());
        }
    }

    // ── Unicode / emoji in content ────────────────────────────────────────────

    #[test]
    fn emoji_text_round_trips_through_text() {
        let text = "Hello 👋 world 🌍\nSecond line 🎉";
        let ev = editor(text);
        assert_eq!(ev.text(), text);
        assert_eq!(ev.line_count(), 2);
    }

    #[test]
    fn undo_preserves_multibyte_text() {
        use std::sync::Arc;
        let mut ev = editor("αβγ");
        // Move cursor to end so paste appends rather than prepends.
        ev.perform(Action::Move(iced::widget::text_editor::Motion::DocumentEnd));
        ev.perform(Action::Edit(Edit::Paste(Arc::new(" δεζ".to_owned()))));
        assert_eq!(ev.text(), "αβγ δεζ");
        ev.undo();
        assert_eq!(ev.text(), "αβγ");
    }

    #[test]
    fn combining_chars_do_not_confuse_line_count() {
        // 'é' as 'e' + combining acute accent
        let text = "caf\u{0065}\u{0301}\nline2";
        let ev = editor(text);
        assert_eq!(ev.line_count(), 2);
    }

    // ── Cursor clamping ───────────────────────────────────────────────────────

    #[test]
    fn move_cursor_clamps_line_to_valid_range() {
        let mut ev = editor("only one line");
        ev.move_cursor(Cursor {
            position: Position {
                line: 999,
                column: 0,
            },
            selection: None,
        });
        assert_eq!(ev.cursor().position.line, 0);
    }

    #[test]
    fn move_cursor_clamps_column_to_line_byte_length() {
        let mut ev = editor("hi");
        ev.move_cursor(Cursor {
            position: Position {
                line: 0,
                column: 9999,
            },
            selection: None,
        });
        // "hi".len() == 2; cursor should be clamped to 2
        assert_eq!(ev.cursor().position.column, 2);
    }

    #[test]
    fn move_cursor_clamps_emoji_column_to_valid_boundary() {
        // "👋" = 4 UTF-8 bytes; valid column is 0 or 4
        let mut ev = editor("👋");
        ev.move_cursor(Cursor {
            position: Position { line: 0, column: 2 }, // mid-codepoint — clamped
            selection: None,
        });
        assert_eq!(ev.cursor().position.column, 0);
    }

    #[test]
    fn move_cursor_without_selection_clears_existing_selection() {
        for action in [Action::SelectAll, Action::SelectWord, Action::SelectLine] {
            let mut ev = editor("hello world");
            ev.move_cursor(Cursor {
                position: Position { line: 0, column: 3 },
                selection: None,
            });
            ev.perform(action);
            assert!(ev.selected_text().is_some());
            let cursor = Cursor {
                position: Position { line: 0, column: 5 },
                selection: None,
            };
            ev.move_cursor(cursor);
            assert_eq!(ev.cursor(), cursor);
            assert_eq!(ev.selected_text(), None);
            assert!(ev.perform(Action::Edit(Edit::Insert('X'))));
            assert_eq!(ev.text(), "helloX world");
            assert!(ev.undo());
            assert_eq!(ev.text(), "hello world");
            assert_eq!(ev.cursor(), cursor);
            assert!(!ev.can_undo());
        }
    }

    // ── replace_text ─────────────────────────────────────────────────────────

    #[test]
    fn replace_text_loads_new_content_and_clears_history() {
        let mut ev = editor("old content");
        ev.perform(Action::Move(text_editor::Motion::DocumentEnd));
        ev.perform(Action::Edit(Edit::Backspace));
        assert!(ev.can_undo());

        ev.replace_text("brand new", false);
        assert_eq!(ev.text(), "brand new");
        assert!(!ev.can_undo());
    }

    #[test]
    fn replace_text_with_preserve_cursor_clamps_to_new_content() {
        let mut ev = editor("short");
        // Move cursor to the end
        ev.perform(Action::Move(iced::widget::text_editor::Motion::End));
        // Replace with much longer content; cursor should be preserved (clamped)
        ev.replace_text("a very long new document content", true);
        assert!(ev.cursor().position.column <= "a very long new document content".len());
    }

    // ── Find ─────────────────────────────────────────────────────────────────

    #[test]
    fn find_empty_query_gives_zero_matches() {
        let mut ev = editor("hello world");
        ev.set_find_query("");
        assert_eq!(ev.find_count(), 0);
    }

    #[test]
    fn find_simple_match() {
        let mut ev = editor("hello world hello");
        ev.set_find_query("hello");
        assert_eq!(ev.find_count(), 2);
    }

    #[test]
    fn find_case_insensitive() {
        let mut ev = editor("Hello HELLO hello");
        ev.set_find_query("hello");
        assert_eq!(ev.find_count(), 3);
    }

    #[test]
    fn find_next_cycles_through_matches() {
        let mut ev = editor("aaa");
        ev.set_find_query("a");
        assert_eq!(ev.find_count(), 3);

        assert!(ev.find_next());
        assert_eq!(ev.find_current(), Some(0));

        ev.find_next();
        assert_eq!(ev.find_current(), Some(1));

        ev.find_next();
        assert_eq!(ev.find_current(), Some(2));

        // Wraps around
        ev.find_next();
        assert_eq!(ev.find_current(), Some(0));
    }

    #[test]
    fn find_previous_wraps_around() {
        let mut ev = editor("aaa");
        ev.set_find_query("a");
        // Start: no current; find_previous should jump to last match
        assert!(ev.find_previous());
        assert_eq!(ev.find_current(), Some(2));

        ev.find_previous();
        assert_eq!(ev.find_current(), Some(1));
    }

    #[test]
    fn find_no_match_returns_false() {
        let mut ev = editor("hello world");
        ev.set_find_query("xyz");
        assert!(!ev.find_next());
        assert!(!ev.find_previous());
    }

    #[test]
    fn find_across_lines() {
        let mut ev = editor("first\nsecond\nthird");
        ev.set_find_query("second");
        assert_eq!(ev.find_count(), 1);
        ev.find_next();
        // The match is on line 1; cursor should be positioned there
        assert_eq!(ev.cursor().position.line, 1);
    }

    #[test]
    fn find_after_each_line_ending_selects_and_undo_restores_match() {
        for ending in ["\n", "\r\n", "\r", "\n\r"] {
            let text = format!("aa{ending}bb{ending}cc");
            let mut ev = editor(&text);
            ev.set_find_query("bb");
            assert_eq!(ev.find_count(), 1);
            assert!(ev.find_next());
            assert_eq!(ev.selected_text().as_deref(), Some("bb"));
            let cursor = Cursor {
                position: Position { line: 1, column: 2 },
                selection: Some(Position { line: 1, column: 0 }),
            };
            assert_eq!(ev.cursor(), cursor);
            assert!(ev.perform(Action::Edit(Edit::Insert('X'))));
            assert_eq!(ev.text(), format!("aa{ending}X{ending}cc"));
            assert!(ev.undo());
            assert_eq!(ev.text(), text);
            assert_eq!(ev.cursor(), cursor);
            assert_eq!(ev.selected_text().as_deref(), Some("bb"));
            assert!(ev.redo());
            assert_eq!(ev.text(), format!("aa{ending}X{ending}cc"));
        }
    }

    #[test]
    fn find_multiline_queries_use_each_line_ending() {
        for ending in ["\n", "\r\n", "\r", "\n\r"] {
            let text = format!("aa{ending}bb{ending}cc{ending}");
            for query in [
                format!("a{ending}b"),
                format!("{ending}bb{ending}"),
                ending.to_owned(),
                format!("cc{ending}"),
            ] {
                let mut ev = editor(&text);
                ev.set_find_query(&query);
                assert_eq!(ev.find_count(), text.match_indices(&query).count());
                assert!(ev.find_next());
                // cosmic-text copies all document line endings as LF.
                let selected = query.replace(ending, "\n");
                assert_eq!(ev.selected_text(), Some(selected.clone()));
                let cursor = ev.cursor();
                assert!(ev.perform(Action::Edit(Edit::Insert('X'))));
                let replaced = text.replacen(&query, "X", 1);
                assert_eq!(ev.text(), replaced);
                assert!(ev.undo());
                assert_eq!(ev.text(), text);
                assert_eq!(ev.cursor(), cursor);
                assert_eq!(ev.selected_text(), Some(selected));
                assert!(ev.redo());
                assert_eq!(ev.text(), replaced);
            }
        }
    }

    #[test]
    fn find_mixed_line_endings_selects_correct_range() {
        let text = "aa\rbb\n\rcc\r\ndd\nee";
        let mut ev = editor(text);
        ev.set_find_query("bb\n\rcc\r\ndd");
        assert!(ev.find_next());
        assert_eq!(ev.selected_text().as_deref(), Some("bb\ncc\ndd"));
        let cursor = Cursor {
            position: Position { line: 3, column: 2 },
            selection: Some(Position { line: 1, column: 0 }),
        };
        assert_eq!(ev.cursor(), cursor);
        assert!(ev.perform(Action::Edit(Edit::Insert('X'))));
        assert_eq!(ev.text(), "aa\rX\nee");
        assert!(ev.undo());
        assert_eq!(ev.text(), text);
        assert_eq!(ev.cursor(), cursor);
        assert_eq!(ev.selected_text().as_deref(), Some("bb\ncc\ndd"));
    }

    #[test]
    fn find_unicode_case_insensitive() {
        // Test ASCII case-insensitivity (the most common Markdown case)
        let mut ev = editor("Rust RUST rust");
        ev.set_find_query("rust");
        assert_eq!(ev.find_count(), 3);
    }

    #[test]
    fn find_multibyte_match() {
        let mut ev = editor("Greek αβγ here");
        ev.set_find_query("αβγ");
        assert_eq!(ev.find_count(), 1);
        ev.find_next();
        // Cursor should be on line 0
        assert_eq!(ev.cursor().position.line, 0);
        // Column should be the byte end of "αβγ" (3 × 2 = 6 bytes) after "Greek "
        let expected_col = "Greek ".len() + "αβγ".len();
        assert_eq!(ev.cursor().position.column, expected_col);
    }

    #[test]
    fn find_invalidated_after_edit() {
        let mut ev = editor("hello");
        ev.set_find_query("hello");
        assert_eq!(ev.find_count(), 1);

        // Move cursor to end so Backspace removes the last character.
        ev.perform(Action::Move(iced::widget::text_editor::Motion::DocumentEnd));
        ev.perform(Action::Edit(Edit::Backspace));
        assert_eq!(ev.find_count(), 0);
        assert_eq!(ev.find_current(), None);
        assert!(!ev.find_next());
    }

    #[test]
    fn find_byte_offset_to_position_basic() {
        let ev = editor("line0\nline1\nline2");
        // "line1" starts at byte 6; its 'i' is at byte 7.
        assert_eq!(
            ev.content
                .with_lines(|lines| byte_offset_to_position(lines, 6)),
            Position { line: 1, column: 0 }
        );
        assert_eq!(
            ev.content
                .with_lines(|lines| byte_offset_to_position(lines, 7)),
            Position { line: 1, column: 1 }
        );
    }

    #[test]
    fn find_byte_offset_multibyte() {
        // "αβ" = 4 bytes (each Greek letter = 2 bytes)
        let ev = editor("αβ\nγδ");
        assert_eq!(
            ev.content
                .with_lines(|lines| byte_offset_to_position(lines, 2)), // 'β' start
            Position { line: 0, column: 2 }
        );
        assert_eq!(
            ev.content
                .with_lines(|lines| byte_offset_to_position(lines, 5)), // 'γ' start, after \n at byte 4
            Position { line: 1, column: 0 }
        );
    }

    #[test]
    fn byte_offset_mapping_stays_within_content_and_utf8_boundaries() {
        for ending in ["\n", "\r\n", "\r", "\n\r"] {
            let text = format!("é{ending}👋{ending}");
            let mut ev = editor(&text);
            for offset in 0..=text.len() + 2 {
                ev.content.with_lines(|lines| {
                    let position = byte_offset_to_position(lines, offset);
                    let line = lines.get(position.line).unwrap();
                    assert!(position.column <= line.text.len());
                    assert!(line.text.is_char_boundary(position.column));
                });
                let position = ev
                    .content
                    .with_lines(|lines| byte_offset_to_position(lines, offset));
                ev.content.move_to(Cursor {
                    position,
                    selection: Some(Position { line: 0, column: 0 }),
                });
                assert!(ev.selected_text().is_some());
            }
        }
    }

    // ── view ──────────────────────────────────────────────────────────────────

    #[test]
    fn view_does_not_clone_content() {
        // This is a structural test: view() takes &self, so it cannot clone
        // content by moving it. We verify the text is unchanged after view().
        let ev = editor("test content");
        let _element = ev.view(fixtures::STYLE, fixtures::DARK);
        // If this compiles and runs without panicking, content was borrowed not moved.
        assert_eq!(ev.text(), "test content");
    }

    #[test]
    fn no_op_edits_do_not_record_history_or_invalidate_redo() {
        let mut nonempty = editor("hello");
        assert!(!nonempty.perform(Action::Edit(Edit::Backspace)));
        assert!(!nonempty.can_undo());
        let mut ev = editor("");
        let no_ops = || {
            [
                Action::Edit(Edit::Backspace),
                Action::Edit(Edit::Delete),
                Action::Edit(Edit::Paste(std::sync::Arc::new(String::new()))),
            ]
        };
        for action in no_ops() {
            assert!(!ev.perform(action));
            assert!(!ev.can_undo());
        }
        type_chars(&mut ev, "a");
        assert!(ev.undo());
        for action in no_ops() {
            assert!(!ev.perform(action));
            assert!(!ev.can_undo());
            assert!(ev.can_redo());
        }
        assert!(ev.redo());
        assert_eq!(ev.text(), "a");
    }

    #[test]
    fn replacing_selection_with_identical_text_is_a_no_op() {
        let mut ev = editor("abc");
        ev.perform(Action::SelectAll);
        assert!(!ev.perform(Action::Edit(Edit::Paste(
            std::sync::Arc::new("abc".into(),)
        ))));
        assert!(!ev.can_undo());
    }

    #[test]
    fn non_edit_actions_break_typing_groups_without_losing_history() {
        for action in [
            Action::Move(text_editor::Motion::Home),
            Action::Click(iced::Point::ORIGIN),
            Action::SelectAll,
            Action::Scroll { lines: 1 },
        ] {
            let mut ev = editor("");
            type_chars(&mut ev, "ab");
            assert!(!ev.perform(action));
            let before = ev.text();
            assert!(ev.perform(Action::Edit(Edit::Insert('x'))));
            assert!(ev.undo());
            assert_eq!(ev.text(), before);
            assert!(ev.undo());
            assert_eq!(ev.text(), "");
        }
    }

    #[test]
    fn programmatic_cursor_and_find_operations_break_typing_groups() {
        for operation in 0..7 {
            let mut ev = editor("");
            if operation >= 5 {
                ev.set_find_query("a");
            }
            type_chars(&mut ev, "ab");
            match operation {
                0 => ev.move_cursor(Cursor {
                    position: Position { line: 0, column: 0 },
                    selection: None,
                }),
                1 => ev.set_find_query("a"),
                2 => {
                    assert!(!ev.find_next());
                }
                3 => {
                    assert!(!ev.find_previous());
                }
                4 => ev.break_group(),
                5 => {
                    assert!(ev.find_next());
                }
                _ => {
                    assert!(ev.find_previous());
                }
            }
            type_chars(&mut ev, "x");
            assert!(ev.undo());
            assert_eq!(ev.text(), "ab");
            assert!(ev.undo());
            assert_eq!(ev.text(), "");
        }
    }

    #[test]
    fn cursor_and_selection_floor_to_utf8_boundaries() {
        let mut ev = editor("a👋é");
        ev.move_cursor(Cursor {
            position: Position { line: 0, column: 3 },
            selection: Some(Position { line: 0, column: 6 }),
        });
        assert_eq!(ev.cursor().position.column, 1);
        assert_eq!(ev.cursor().selection.unwrap().column, 5);
        assert_eq!(ev.selected_text().as_deref(), Some("👋"));
        assert!(ev.perform(Action::Edit(Edit::Insert('x'))));
        assert_eq!(ev.text(), "axé");
        ev.undo();
        assert_eq!(ev.selected_text().as_deref(), Some("👋"));
    }

    #[test]
    fn replacement_preserved_cursor_floors_multibyte_positions() {
        let mut ev = editor("abcdef");
        ev.move_cursor(Cursor {
            position: Position { line: 0, column: 3 },
            selection: Some(Position { line: 0, column: 2 }),
        });
        ev.replace_text("👋é", true);
        assert_eq!(ev.cursor().position.column, 0);
        assert_eq!(ev.cursor().selection.unwrap().column, 0);
    }

    #[test]
    fn find_counts_refresh_on_edit_undo_redo_and_replace() {
        let mut ev = editor("a a");
        ev.set_find_query("a");
        ev.find_previous();
        assert_eq!(ev.find_current(), Some(1));
        assert!(ev.perform(Action::Edit(Edit::Backspace)));
        assert_eq!(ev.find_count(), 1);
        assert_eq!(ev.find_current(), None);
        ev.undo();
        assert_eq!(ev.find_count(), 2);
        ev.redo();
        assert_eq!(ev.find_count(), 1);
        ev.replace_text("a a a", false);
        assert_eq!(ev.find_count(), 3);
        ev.find_previous();
        ev.set_find_query("missing");
        assert_eq!(ev.find_current(), None);
        assert_eq!(ev.find_count(), 0);
        assert!(!ev.find_previous());
    }

    #[test]
    fn unicode_lowercase_expansions_map_to_original_text() {
        let text = "İ i\u{307} I 👋İ";
        assert_eq!(
            find_all_matches(text, "i\u{307}"),
            vec![(0, 2), (3, 6), (13, 15)]
        );
        assert_eq!(find_all_matches(text, "İ"), vec![(0, 2), (3, 6), (13, 15)]);
        assert!(find_all_matches("i", "İ").is_empty());
        for (start, end) in find_all_matches(text, "i") {
            assert!(text.is_char_boundary(start));
            assert!(text.is_char_boundary(end));
        }
        let mut ev = editor(text);
        ev.set_find_query("i\u{307}");
        assert_eq!(ev.find_count(), 3);
        ev.find_next();
        assert_eq!(ev.selected_text().as_deref(), Some("İ"));
        ev.perform(Action::Edit(Edit::Insert('x')));
        assert_eq!(ev.find_count(), 2);
        ev.find_next();
        assert_eq!(ev.selected_text().as_deref(), Some("İ"));
    }

    // ── Paste action type ────────────────────────────────────────────────────

    #[test]
    fn edit_kind_classifies_correctly() {
        use std::sync::Arc;
        assert_eq!(
            edit_kind(&Action::Edit(Edit::Insert('a'))),
            EditKind::Insert
        );
        assert_eq!(
            edit_kind(&Action::Edit(Edit::Paste(Arc::new("x".into())))),
            EditKind::Paste
        );
        assert_eq!(edit_kind(&Action::Edit(Edit::Enter)), EditKind::Enter);
        assert_eq!(edit_kind(&Action::Edit(Edit::Backspace)), EditKind::Delete);
        assert_eq!(edit_kind(&Action::Edit(Edit::Delete)), EditKind::Delete);
        assert_eq!(edit_kind(&Action::Edit(Edit::Indent)), EditKind::Replace);
    }
}
