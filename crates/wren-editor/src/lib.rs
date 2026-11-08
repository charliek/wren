//! Markdown editor with grouped undo/redo and Unicode-lowercase literal find.
//!
//! Cursor columns and find ranges use UTF-8 byte offsets. Undo and redo edit
//! only the bytes that changed, so the rest of the document keeps its layout;
//! on the owned engine they also return the view to its exact scroll.
//!
//! The text runs on one of two engines ([`BackendKind`]): iced's own
//! `text::Editor`, or a cosmic-text editor this crate owns, which allows range
//! edits and absolute scrolling. Both pass the same test suite.
//!
//! The crate knows nothing about its host: colours arrive as a [`Style`] and a
//! [`Palette`], the font through [`EditorView::set_font`], and which keys are
//! the host's shortcuts or bound editor commands through a [`KeyPolicy`].

mod backend;
mod edit;
mod find;
mod highlighter;
mod history;
mod keys;
mod plain;
mod widget;

#[cfg(test)]
mod fixtures;

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use iced::widget::text_editor::{self, Action, Cursor, Edit, Motion, Position};
use iced::{Color, Element, Font};

pub use backend::{BackendKind, Placement};
pub use edit::Replace;
pub use keys::{KeyPolicy, KeyRoute};
pub use wren_markdown::Intent;

use backend::Lines;
use find::{
    byte_offset_to_position, exact_byte_offset_to_position, find_all_matches, first_content_match,
};
use highlighter::Settings as HighlighterSettings;
use history::{EditKind, History, Snapshot};
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
    /// An editing key that depends on the text around the cursor: Enter,
    /// Tab, Backspace, Home, and the like. See [`EditorView::intent`].
    Intent(Intent),
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
    markdown_assist: bool,
    key_policy: Option<KeyPolicy>,
}

/// How the planner indents and renumbers.
const PLAN_OPTIONS: wren_markdown::Options = wren_markdown::Options {
    indent_width: 4,
    renumber: true,
};

impl EditorView {
    /// Create a new editor pre-populated with `text`, on iced's engine.
    pub fn new(text: &str) -> Self {
        Self::with_backend(BackendKind::Iced, text)
    }

    /// Create a new editor pre-populated with `text`, on the `kind` engine.
    pub fn with_backend(kind: BackendKind, text: &str) -> Self {
        Self {
            content: Content::new(kind, text),
            history: History::new(),
            find_query: String::new(),
            find_matches: Vec::new(),
            find_current: None,
            line_numbers: true,
            metrics: widget::Metrics::default(),
            font: Font::MONOSPACE,
            markdown_assist: true,
            key_policy: None,
        }
    }

    /// Let the host route key presses before the editor handles them; see
    /// [`KeyPolicy`]. Without one, every key is the editor's.
    pub fn set_key_policy(&mut self, policy: KeyPolicy) {
        self.key_policy = Some(policy);
    }

    /// Turn the Markdown-aware keys on or off (on by default). Off, Enter
    /// and Home are iced's own, and the other editing keys work on plain
    /// text; see [`EditorView::intent`].
    pub fn set_markdown_assist(&mut self, on: bool) {
        self.markdown_assist = on;
    }

    pub fn markdown_assist(&self) -> bool {
        self.markdown_assist
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
            self.markdown_assist,
        )
        .key_policy(self.key_policy.as_ref())
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
            let before = self.snapshot();
            let action = self.with_document_breaks(action);
            self.content.perform(action);
            if self.content.text() == before.text {
                self.history.break_group();
                return false;
            }
            self.history.before_edit(before, kind, now);
            self.recompute_find();
            true
        } else {
            self.history.break_group();
            self.content.perform(action);
            false
        }
    }

    /// Apply `edits` as one undoable change, then select `selection`.
    ///
    /// Edit positions refer to the text before the change, and `selection` to
    /// the text after it. Each line break an edit inserts takes the ending of
    /// the line it splits, so a CRLF note stays CRLF. The change closes any
    /// typing group, and find is refreshed.
    ///
    /// Returns whether the text changed. A plan with an edit off the text,
    /// inside a character, backwards, or overlapping another changes nothing.
    pub fn apply(&mut self, edits: &[Replace], selection: Cursor) -> bool {
        let Some(order) = self.content.with_lines(|lines| edit::ordered(edits, lines)) else {
            return false;
        };
        let before = self.snapshot();
        for replace in order.iter().rev() {
            let ending = self
                .content
                .with_lines(|lines| edit::split_ending(lines, replace.end));
            let text = edit::with_breaks(&replace.text, ending);
            self.content
                .replace_range(replace.start, replace.end, &text);
        }
        let changed = self.content.text() != before.text;
        if changed {
            self.history
                .before_edit(before, EditKind::Replace, Instant::now());
            self.recompute_find();
        } else {
            self.history.break_group();
        }
        self.select(selection);
        changed
    }

    /// Carry out an editing [`Intent`] at the cursor as one undoable change.
    ///
    /// With Markdown assist on, `wren_markdown` plans it over the borrowed
    /// lines, and the plan is applied as with [`EditorView::apply`]. Home
    /// first stops at the start of a wrapped visual row, which only the
    /// layout knows. With assist off, Enter and Home are iced's own, Tab
    /// inserts spaces to the next multiple of 4 (or indents the lines of a
    /// selection that spans lines), Shift+Tab outdents, Backspace deletes a
    /// grapheme cluster, the line delete goes to column 0, and the Markdown
    /// commands do nothing; Delete, the word deletes, and the document motions are the
    /// same either way.
    ///
    /// Returns whether the text changed.
    pub fn intent(&mut self, intent: Intent) -> bool {
        // A double- or triple-click selection reports its click point, not
        // its bounds, until it is normalized.
        let cursor = self.snapshot_cursor();
        match intent {
            Intent::Enter | Intent::ShiftEnter if !self.markdown_assist => {
                return self.perform(Action::Edit(Edit::Enter));
            }
            Intent::Home { extend } => {
                let home = if extend {
                    Action::Select(Motion::Home)
                } else {
                    Action::Move(Motion::Home)
                };
                self.history.break_group();
                // iced's Home goes to the start of the visual row. Only a
                // continuation row starts past column 0, and a cursor already
                // at a row start does not move; otherwise that is the stop.
                self.content.perform(home);
                let moved = self.content.cursor().position;
                let row_start = moved.line == cursor.position.line
                    && moved.column > 0
                    && moved != cursor.position;
                if !self.markdown_assist || row_start {
                    return false;
                }
                self.select(cursor);
            }
            _ => {}
        }
        let selection = markdown_selection(cursor);
        let assist = self.markdown_assist;
        let plan = self.content.with_lines(|lines| {
            let plain = (!assist)
                .then(|| plain::plan(lines, selection, intent))
                .flatten();
            plain.unwrap_or_else(|| wren_markdown::plan(&lines, selection, intent, &PLAN_OPTIONS))
        });
        if plan.edits.is_empty() {
            // A motion, or a key with nothing to do: no text to snapshot.
            self.history.break_group();
            self.select(iced_cursor(plan.selection));
            return false;
        }
        let edits: Vec<Replace> = plan.edits.into_iter().map(Replace::from).collect();
        self.apply(&edits, iced_cursor(plan.selection))
    }

    /// Undo the last edit unit.
    ///
    /// Returns `true` if something was restored: the text, the cursor and
    /// selection, and on the owned engine the scroll position.
    pub fn undo(&mut self) -> bool {
        if !self.history.can_undo() {
            return false;
        }
        let current = self.snapshot();
        match self.history.undo(current) {
            Some(snapshot) => {
                self.restore(snapshot);
                true
            }
            None => false,
        }
    }

    /// Redo the next edit unit.
    ///
    /// Returns `true` if something was restored.
    pub fn redo(&mut self) -> bool {
        if !self.history.can_redo() {
            return false;
        }
        let current = self.snapshot();
        match self.history.redo(current) {
            Some(snapshot) => {
                self.restore(snapshot);
                true
            }
            None => false,
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
        self.select(cursor);
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

    /// Scroll so `line` sits at `placement`, without moving the cursor. Near
    /// the end of the text the view stops at the last line. To jump to a
    /// line, scroll first and then move the cursor there.
    ///
    /// The owned engine places the line exactly. iced can only scroll by
    /// whole rows from where it is, and counts a line it has never laid out
    /// as one row, so a wrapped line between the view and the target leaves
    /// the target off by that line's extra rows.
    pub fn scroll_to_line(&mut self, line: usize, placement: Placement) {
        self.history.break_group();
        self.content.scroll_to_line(line, placement);
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
        match saved_cursor {
            Some(cursor) => {
                self.content.replace_text(text);
                let clamped = self.clamped(cursor);
                self.move_cursor(clamped);
            }
            None => self.content = Content::new(self.content.kind(), text),
        }
        self.history.clear();
        self.find_current = None;
        self.recompute_find();
    }

    /// The engine the editor runs on.
    pub fn backend(&self) -> BackendKind {
        self.content.kind()
    }

    /// Move the document to the `kind` engine, loading `text` (the host's
    /// copy of the document). The cursor and selection are clamped and kept,
    /// as are the find query, font, size, and gutter; history is dropped.
    /// Returns whether the engine changed.
    pub fn set_backend(&mut self, kind: BackendKind, text: &str) -> bool {
        if kind == self.content.kind() {
            return false;
        }
        let cursor = self.snapshot_cursor();
        self.content = Content::new(kind, text);
        let clamped = self.clamped(cursor);
        self.content.move_to(clamped);
        self.history.clear();
        self.find_current = None;
        self.recompute_find();
        true
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

    /// Move the cursor to `cursor`, clamped, dropping any old selection.
    fn select(&mut self, cursor: Cursor) {
        let clamped = self.clamped(cursor);
        if clamped.selection.is_none() && self.content.cursor().selection.is_some() {
            // iced's move_to leaves the old selection intact when passed None.
            self.content.perform(Action::Move(Motion::Left));
        }
        self.content.move_to(clamped);
    }

    /// `action` with the line breaks it inserts written in the ending of the
    /// line they split, so Enter, paste, and IME commits match the note.
    fn with_document_breaks(&self, action: Action) -> Action {
        let ending = || {
            let cursor = self.content.cursor();
            let anchor = cursor.selection.unwrap_or(cursor.position);
            let end = std::cmp::max_by_key(anchor, cursor.position, |at| (at.line, at.column));
            self.content
                .with_lines(|lines| edit::split_ending(lines, end))
        };
        match action {
            Action::Edit(Edit::Enter | Edit::Insert('\n')) => match ending() {
                text_editor::LineEnding::Lf => Action::Edit(Edit::Enter),
                other => Action::Edit(Edit::Paste(Arc::new(other.as_str().to_owned()))),
            },
            Action::Edit(Edit::Paste(text)) => match edit::with_breaks(&text, ending()) {
                Cow::Borrowed(_) => Action::Edit(Edit::Paste(text)),
                Cow::Owned(text) => Action::Edit(Edit::Paste(Arc::new(text))),
            },
            action => action,
        }
    }

    fn snapshot(&mut self) -> Snapshot {
        let cursor = self.snapshot_cursor();
        Snapshot {
            text: self.content.text(),
            cursor,
            scroll: self.content.scroll(),
        }
    }

    /// Return to a history snapshot by replacing only the bytes that differ.
    fn restore(&mut self, snapshot: Snapshot) {
        let current = self.content.text();
        let change = history::diff(&current, &snapshot.text);
        let (start, end, text) = self
            .content
            .with_lines(|lines| {
                Some((
                    exact_byte_offset_to_position(lines, change.start)?,
                    exact_byte_offset_to_position(lines, change.from_end)?,
                    &snapshot.text[change.start..change.to_end],
                ))
            })
            .unwrap_or_else(|| {
                let end = self
                    .content
                    .with_lines(|lines| byte_offset_to_position(lines, current.len()));
                (Position { line: 0, column: 0 }, end, &snapshot.text[..])
            });
        self.content.replace_range(start, end, text);
        let cursor = self.clamped(snapshot.cursor);
        self.content.restore_view(cursor, snapshot.scroll);
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

fn markdown_selection(cursor: Cursor) -> wren_markdown::Selection {
    let pos = |at: Position| wren_markdown::Pos::new(at.line, at.column);
    wren_markdown::Selection::new(
        pos(cursor.selection.unwrap_or(cursor.position)),
        pos(cursor.position),
    )
}

fn iced_cursor(selection: wren_markdown::Selection) -> Cursor {
    let position = |at: wren_markdown::Pos| Position {
        line: at.line,
        column: at.col,
    };
    Cursor {
        position: position(selection.head),
        selection: (!selection.is_empty()).then(|| position(selection.anchor)),
    }
}

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
    use widget::Content;

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

    #[test]
    fn switching_engines_keeps_the_text_cursor_find_and_settings_but_drops_history() {
        for (from, to) in [
            (BackendKind::Iced, BackendKind::Owned),
            (BackendKind::Owned, BackendKind::Iced),
        ] {
            let mut ev = EditorView::with_backend(from, "alpha beta\r\ngamma beta");
            assert_eq!(ev.backend(), from);
            ev.set_font_size(18.0);
            ev.set_font(Font::with_name("JetBrains Mono"));
            ev.set_line_numbers(false);
            ev.perform(Action::Move(Motion::DocumentEnd));
            ev.perform(Action::Edit(Edit::Insert('!')));
            ev.set_find_query("beta");
            assert!(ev.find_next());
            let cursor = ev.cursor();
            let text = ev.text();

            assert!(!ev.set_backend(from, &text));
            assert!(ev.can_undo());
            assert!(ev.set_backend(to, &text));
            assert_eq!(ev.backend(), to);
            assert_eq!(ev.text(), text);
            assert_eq!(ev.cursor(), cursor);
            assert_eq!(ev.selected_text().as_deref(), Some("beta"));
            assert_eq!(ev.find_query, "beta");
            assert_eq!(ev.find_count(), 2);
            assert_eq!(ev.find_current(), None);
            assert_eq!(ev.metrics, widget::Metrics::for_size(18.0));
            assert_eq!(ev.font, Font::with_name("JetBrains Mono"));
            assert!(!ev.line_numbers);
            assert!(!ev.can_undo());

            // Edits, undo, and replacement stay on the new engine.
            assert!(ev.perform(Action::Edit(Edit::Insert('X'))));
            assert!(ev.undo());
            assert_eq!(ev.text(), text);
            ev.replace_text("fresh", true);
            assert_eq!(ev.backend(), to);
            ev.replace_text("fresh", false);
            assert_eq!(ev.backend(), to);
        }
    }

    #[test]
    fn switching_engines_clamps_the_cursor_into_the_hosts_text() {
        let mut ev = EditorView::with_backend(BackendKind::Iced, "a much longer line");
        ev.perform(Action::Move(Motion::DocumentEnd));
        assert!(ev.set_backend(BackendKind::Owned, "short\né"));
        assert_eq!(ev.text(), "short\né");
        assert_eq!(ev.cursor().position, Position { line: 0, column: 5 });
    }

    fn numbered_lines(count: usize) -> String {
        (0..count)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn at(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    fn parsed(content: &Content) -> Vec<(String, text_editor::LineEnding)> {
        content.with_lines(|lines| {
            lines
                .iter()
                .map(|line| (line.text.into_owned(), line.ending))
                .collect()
        })
    }

    #[test]
    fn restoring_a_snapshot_leaves_the_lines_a_fresh_load_would() {
        let mut texts = vec![String::new()];
        let mut longest = vec![String::new()];
        for _ in 0..3 {
            longest = longest
                .iter()
                .flat_map(|text| ["a", "é", "\r", "\n"].map(|c| format!("{text}{c}")))
                .collect();
            texts.extend(longest.iter().cloned());
        }
        for kind in BackendKind::ALL {
            for from in &texts {
                for to in &texts {
                    let mut ev = EditorView::with_backend(kind, from);
                    ev.restore(Snapshot {
                        text: to.clone(),
                        cursor: at(9, 9),
                        scroll: Default::default(),
                    });
                    assert_eq!(ev.text(), *to, "{kind:?}: {from:?} -> {to:?}");
                    assert_eq!(
                        parsed(&ev.content),
                        parsed(&Content::new(kind, to)),
                        "{kind:?}: {from:?} -> {to:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn owned_undo_and_redo_return_to_the_exact_scroll() {
        let size = iced::Size::new(300.0, 120.0);
        let mut ev = EditorView::with_backend(BackendKind::Owned, &numbered_lines(200));
        ev.content.lay_out(size);
        ev.move_cursor(at(100, 0));
        ev.content.lay_out(size);
        // The cursor mid-view, which revealing it alone would not produce.
        ev.perform(Action::Scroll { lines: 2 });
        ev.content.lay_out(size);
        let typed_at = ev.content.scroll();
        assert!(ev.perform(Action::Edit(Edit::Insert('x'))));
        ev.content.lay_out(size);
        ev.perform(Action::Scroll { lines: 50 });
        ev.content.lay_out(size);
        let undone_at = ev.content.scroll();
        assert_ne!(undone_at, typed_at);

        assert!(ev.undo());
        assert_eq!(ev.content.scroll(), typed_at);
        ev.content.lay_out(size);
        assert_eq!(ev.content.scroll(), typed_at);
        assert_eq!(ev.cursor(), at(100, 0));
        assert!(matches!(
            ev.content.selection_geometry(),
            Some(text_editor::Selection::Caret(_))
        ));

        // Redo returns to where undo was pressed, then shows the cursor.
        assert!(ev.redo());
        ev.content.lay_out(size);
        assert_eq!(ev.cursor(), at(100, 1));
        assert!(matches!(
            ev.content.selection_geometry(),
            Some(text_editor::Selection::Caret(_))
        ));
        assert_eq!(
            ev.text(),
            numbered_lines(200).replace("line 100", "xline 100")
        );
    }

    /// Laid-out selection, paging, Enter, and history over wrapped CRLF rows
    /// leave both engines with the same text, cursor, selection, and scroll.
    #[test]
    fn laid_out_editing_matches_across_engines() {
        use iced::Point;

        let size = iced::Size::new(300.0, 160.0);
        let long = "wrapped words fill this row ".repeat(4);
        let text = (0..40)
            .map(|i| {
                if i % 3 == 0 {
                    format!("{i} {long}")
                } else {
                    format!("line {i}")
                }
            })
            .collect::<Vec<_>>()
            .join("\r\n");
        let script = || {
            let edit = |edit| Step::Act(Action::Edit(edit));
            vec![
                Step::Act(Action::Click(Point::new(30.0, 30.0))),
                Step::Act(Action::SelectWord),
                edit(Edit::Insert('X')),
                Step::Undo,
                edit(Edit::Insert('Y')),
                Step::Act(Action::Move(Motion::PageDown)),
                Step::Act(Action::Select(Motion::PageDown)),
                Step::Act(Action::Move(Motion::PageUp)),
                Step::Act(Action::Move(Motion::Up)),
                Step::Act(Action::Move(Motion::Up)),
                edit(Edit::Enter),
                Step::Act(Action::Click(Point::new(20.0, 50.0))),
                Step::Act(Action::Drag(Point::new(120.0, 110.0))),
                edit(Edit::Backspace),
                Step::Undo,
                Step::Act(Action::SelectLine),
                edit(Edit::Insert('Z')),
                Step::Undo,
                Step::Redo,
                Step::Undo,
                Step::Undo,
                Step::Undo,
            ]
        };
        let trace = |kind| {
            let mut ev = EditorView::with_backend(kind, &text);
            ev.content.lay_out(size);
            script()
                .into_iter()
                .map(|step| {
                    match step {
                        Step::Act(action) => {
                            ev.perform(action);
                        }
                        Step::Undo => {
                            ev.undo();
                        }
                        Step::Redo => {
                            ev.redo();
                        }
                    }
                    ev.content.lay_out(size);
                    (
                        ev.text(),
                        ev.cursor(),
                        ev.selected_text(),
                        ev.content.scroll().line,
                    )
                })
                .collect::<Vec<_>>()
        };
        enum Step {
            Act(Action),
            Undo,
            Redo,
        }
        let iced = trace(BackendKind::Iced);
        let owned = trace(BackendKind::Owned);
        for (step, (iced, owned)) in iced.iter().zip(&owned).enumerate() {
            assert_eq!(iced, owned, "step {step}");
        }
        // Enter wrote CRLF, and the view scrolled along the way.
        assert!(iced
            .iter()
            .all(|(text, ..)| text.matches('\n').count() == text.matches("\r\n").count()));
        assert!(iced.iter().any(|(.., line)| *line > 0));
        assert_eq!(iced.last().unwrap().0, text);
    }

    /// Instantiates the tests once per engine, as `on_iced` and `on_owned`.
    macro_rules! on_each_backend {
        ($($tests:tt)*) => {
            mod on_iced {
                const KIND: crate::BackendKind = crate::BackendKind::Iced;
                $($tests)*
            }
            mod on_owned {
                const KIND: crate::BackendKind = crate::BackendKind::Owned;
                $($tests)*
            }
        };
    }

    on_each_backend! {
        use super::super::*;
        use crate::fixtures;

        // Helper: create an EditorView with pre-loaded text.
        fn editor(text: &str) -> EditorView {
            EditorView::with_backend(KIND, text)
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
        fn undo_and_redo_keep_the_view_and_its_shaping() {
            let size = iced::Size::new(300.0, 120.0);
            let mut ev = editor(&super::numbered_lines(200));
            ev.content.lay_out(size);
            ev.move_cursor(super::at(100, 0));
            ev.content.lay_out(size);
            ev.perform(Action::Scroll { lines: 2 });
            ev.content.lay_out(size);
            let scroll = ev.content.scroll();
            let shaped = |ev: &EditorView, line| ev.content.is_laid_out(line);
            assert!(shaped(&ev, 99));
            assert!(ev.perform(Action::Edit(Edit::Insert('x'))));
            ev.content.lay_out(size);
            assert!(ev.undo());
            // The lines around the edit keep their layout.
            assert!(shaped(&ev, 99));
            assert!(shaped(&ev, 101));
            ev.content.lay_out(size);
            assert_eq!(ev.content.scroll(), scroll);
            assert!(ev.redo());
            ev.content.lay_out(size);
            assert_eq!(ev.content.scroll(), scroll);
            assert_eq!(ev.cursor(), super::at(100, 1));
        }

        #[test]
        fn undo_restores_each_line_ending_and_a_crlf_selection() {
            for ending in ["\n", "\r\n", "\r", "\n\r"] {
                let original = format!("one{ending}two{ending}three");
                let mut ev = editor(&original);
                ev.move_cursor(Cursor {
                    position: Position { line: 2, column: 2 },
                    selection: Some(Position { line: 0, column: 1 }),
                });
                assert!(ev.perform(Action::Edit(Edit::Backspace)));
                assert_eq!(ev.text(), "oree");
                assert!(ev.undo());
                assert_eq!(ev.text(), original, "{ending:?}");
                assert_eq!(ev.line_count(), 3);
                assert_eq!(ev.selected_text().as_deref(), Some("ne\ntwo\nth"));
                assert!(ev.redo());
                assert_eq!(ev.text(), "oree");
                assert_eq!(ev.cursor(), super::at(0, 1));
            }
        }

        fn pos(line: usize, column: usize) -> Position {
            Position { line, column }
        }

        const ENDINGS: [&str; 4] = ["\n", "\r\n", "\r", "\n\r"];

        #[test]
        fn apply_writes_inserted_breaks_in_the_split_lines_ending() {
            for e in ENDINGS {
                for last in ["", e] {
                    let original = format!("one{e}two{e}three{last}");
                    let mut ev = editor(&original);
                    let selection = Cursor {
                        position: pos(3, 0),
                        selection: Some(pos(2, 0)),
                    };
                    assert!(ev.apply(&[Replace::insert(pos(1, 3), "\nX\n")], selection));
                    assert_eq!(ev.text(), format!("one{e}two{e}X{e}{e}three{last}"), "{e:?}");
                    assert_eq!(ev.cursor(), selection);
                    assert_eq!(ev.selected_text().as_deref(), Some("X\n"));
                    assert!(ev.undo());
                    assert_eq!(ev.text(), original);
                    assert!(!ev.can_undo());
                    assert!(ev.redo());
                    assert_eq!(ev.text(), format!("one{e}two{e}X{e}{e}three{last}"));
                }

                // The last line has no ending, so a split borrows the one above.
                let mut ev = editor(&format!("one{e}two"));
                assert!(ev.apply(&[Replace::insert(pos(1, 3), "\n- ")], super::at(2, 2)));
                assert_eq!(ev.text(), format!("one{e}two{e}- "));
                assert_eq!(ev.cursor(), super::at(2, 2));
            }
        }

        #[test]
        fn apply_without_an_ending_to_borrow_uses_lf() {
            for text in ["", "solo"] {
                let mut ev = editor(text);
                assert!(ev.apply(&[Replace::insert(pos(0, 0), "a\r\nb\n")], super::at(2, 0)));
                assert_eq!(ev.text(), format!("a\nb\n{text}"));
                assert_eq!(ev.cursor(), super::at(2, 0));
            }
        }

        #[test]
        fn apply_runs_a_plan_bottom_up_over_mixed_endings() {
            let mut ev = editor("- a\r\n- b\n- c\r- d");
            let plan = [
                Replace::insert(pos(0, 3), "\n- "),
                Replace {
                    start: pos(1, 2),
                    end: pos(2, 3),
                    text: "B\nC".into(),
                },
                Replace::insert(pos(3, 3), "\n- "),
            ];
            assert!(ev.apply(&plan, super::at(1, 2)));
            // Line 1 ends in LF; the deletion moves line 2's CR onto it.
            assert_eq!(ev.text(), "- a\r\n- \r\n- B\rC\r- d\r- ");
            assert_eq!(ev.line_count(), 6);
            assert!(ev.undo());
            assert_eq!(ev.text(), "- a\r\n- b\n- c\r- d");
        }

        #[test]
        fn apply_is_one_history_entry_that_closes_the_typing_group() {
            let mut ev = editor("ab");
            ev.perform(Action::Move(Motion::DocumentEnd));
            type_chars(&mut ev, "cd");
            ev.set_find_query("x");
            assert_eq!(ev.find_count(), 0);
            let plan = [
                Replace::insert(pos(0, 0), "x"),
                Replace {
                    start: pos(0, 1),
                    end: pos(0, 2),
                    text: "xx".into(),
                },
            ];
            assert!(ev.apply(&plan, super::at(0, 4)));
            assert_eq!(ev.text(), "xaxxcd");
            assert_eq!(ev.find_count(), 3);
            type_chars(&mut ev, "e");
            assert!(ev.undo());
            assert_eq!(ev.text(), "xaxxcd");
            assert!(ev.undo());
            assert_eq!(ev.text(), "abcd");
            assert_eq!(ev.find_count(), 0);
            assert!(ev.undo());
            assert_eq!(ev.text(), "ab");
            assert!(!ev.can_undo());
        }

        #[test]
        fn apply_refuses_invalid_plans_and_no_ops_record_nothing() {
            let mut ev = editor("héllo\nworld");
            ev.move_cursor(Cursor {
                position: pos(1, 2),
                selection: Some(pos(1, 0)),
            });
            let before = ev.cursor();
            for plan in [
                vec![Replace::insert(pos(0, 2), "x")],
                vec![Replace::insert(pos(2, 0), "x")],
                vec![Replace {
                    start: pos(1, 3),
                    end: pos(1, 1),
                    text: String::new(),
                }],
                vec![
                    Replace {
                        start: pos(0, 0),
                        end: pos(1, 1),
                        text: String::new(),
                    },
                    Replace::insert(pos(0, 4), "x"),
                ],
            ] {
                assert!(!ev.apply(&plan, super::at(0, 0)), "{plan:?}");
                assert_eq!(ev.text(), "héllo\nworld");
                assert_eq!(ev.cursor(), before);
            }
            // A plan that changes nothing still moves the cursor.
            let same = Replace {
                start: pos(0, 0),
                end: pos(0, 1),
                text: "h".into(),
            };
            assert!(!ev.apply(&[same], super::at(1, 5)));
            assert_eq!(ev.cursor(), super::at(1, 5));
            assert!(!ev.apply(&[], super::at(0, 1)));
            assert_eq!(ev.cursor(), super::at(0, 1));
            assert!(!ev.can_undo());
        }

        #[test]
        fn enter_and_paste_write_breaks_in_the_notes_ending() {
            for e in ENDINGS {
                let mut ev = editor(&format!("one{e}two"));
                ev.move_cursor(super::at(0, 3));
                assert!(ev.perform(Action::Edit(Edit::Enter)));
                assert_eq!(ev.text(), format!("one{e}{e}two"), "{e:?}");
                assert_eq!(ev.cursor(), super::at(1, 0));
                // A paste, and an IME commit, which arrives as one.
                assert!(ev.perform(Action::Edit(Edit::Paste(Arc::new("x\ny\r\nz".into())))));
                assert_eq!(ev.text(), format!("one{e}x{e}y{e}z{e}two"));
                ev.perform(Action::Move(Motion::DocumentEnd));
                assert!(ev.perform(Action::Edit(Edit::Enter)));
                assert_eq!(ev.text(), format!("one{e}x{e}y{e}z{e}two{e}"));
                assert!(ev.undo());
                assert!(ev.undo());
                assert!(ev.undo());
                assert_eq!(ev.text(), format!("one{e}two"));

                // Over a selection, the split line takes the selection end's ending.
                let mut ev = editor(&format!("a{e}b\nc"));
                ev.move_cursor(Cursor {
                    position: pos(1, 1),
                    selection: Some(pos(0, 1)),
                });
                assert!(ev.perform(Action::Edit(Edit::Enter)));
                assert_eq!(ev.text(), "a\n\nc");
            }
            let mut ev = editor("");
            assert!(ev.perform(Action::Edit(Edit::Enter)));
            assert_eq!(ev.text(), "\n");
        }

        #[test]
        fn edits_over_the_whole_note_keep_its_ending() {
            for e in ENDINGS {
                let original = format!("one{e}two{e}three");
                let mut ev = editor(&original);
                ev.perform(Action::SelectAll);
                assert!(ev.perform(Action::Edit(Edit::Paste(Arc::new("x\r\ny\nz".into())))));
                assert_eq!(ev.text(), format!("x{e}y{e}z"), "{e:?}");

                let mut ev = editor(&original);
                ev.perform(Action::SelectAll);
                assert!(ev.perform(Action::Edit(Edit::Enter)));
                assert_eq!(ev.text(), e, "{e:?}");

                let mut ev = editor(&original);
                let whole = Replace {
                    start: pos(0, 1),
                    end: pos(2, 1),
                    text: "X\nY".into(),
                };
                assert!(ev.apply(&[whole], super::at(1, 1)));
                assert_eq!(ev.text(), format!("oX{e}Yhree"), "{e:?}");
                assert!(ev.undo());
                assert_eq!(ev.text(), original);
            }
        }

        #[test]
        fn scroll_to_line_places_the_line_and_stops_at_the_end() {
            let size = iced::Size::new(300.0, 240.0);
            let mut ev = editor(&super::numbered_lines(200));
            ev.content.lay_out(size);
            let top = |ev: &EditorView| (ev.content.scroll().line, ev.content.scroll().vertical);

            ev.scroll_to_line(100, Placement::Top);
            ev.content.lay_out(size);
            assert_eq!(top(&ev), (100, 0.0));
            // Ten rows in view: a quarter down is two rows.
            ev.scroll_to_line(150, Placement::NearTop);
            ev.content.lay_out(size);
            assert_eq!(top(&ev), (148, 0.0));
            ev.scroll_to_line(20, Placement::NearTop);
            ev.content.lay_out(size);
            assert_eq!(top(&ev), (18, 0.0));
            // Moving the cursor there afterwards keeps the view.
            ev.move_cursor(super::at(20, 0));
            ev.content.lay_out(size);
            assert_eq!(top(&ev), (18, 0.0));
            assert_eq!(ev.cursor(), super::at(20, 0));

            // The last screenful does not scroll past the end.
            ev.scroll_to_line(197, Placement::Top);
            ev.content.lay_out(size);
            assert_eq!(top(&ev), (190, 0.0));
            ev.scroll_to_line(5_000, Placement::Top);
            ev.content.lay_out(size);
            assert_eq!(top(&ev), (190, 0.0));
            ev.scroll_to_line(1, Placement::NearTop);
            ev.content.lay_out(size);
            assert_eq!(top(&ev), (0, 0.0));
        }

        fn caret(line: usize, column: usize) -> Cursor {
            Cursor {
                position: Position { line, column },
                selection: None,
            }
        }

        #[test]
        fn smart_enter_continues_a_list_in_the_notes_ending_as_one_undo_unit() {
            for ending in ["\n", "\r\n", "\r", "\n\r"] {
                let text = ["- a", "- [x] b"].join(ending);
                let mut ev = editor(&text);
                ev.move_cursor(caret(1, 7));
                assert!(ev.intent(Intent::Enter));
                assert_eq!(ev.text(), ["- a", "- [x] b", "- [ ] "].join(ending));
                assert_eq!(ev.cursor(), caret(2, 6));
                // Enter on the empty item exits the list.
                assert!(ev.intent(Intent::Enter));
                assert_eq!(ev.text(), ["- a", "- [x] b", "", ""].join(ending));
                assert!(ev.undo());
                assert!(ev.undo());
                assert_eq!(ev.text(), text);
                assert_eq!(ev.cursor(), caret(1, 7));
                assert!(!ev.can_undo());
            }
        }

        #[test]
        fn tab_and_shift_tab_nest_and_renumber() {
            let mut ev = editor("1. a\n2. b\n3. c");
            ev.move_cursor(caret(1, 4));
            assert!(ev.intent(Intent::Tab));
            assert_eq!(ev.text(), "1. a\n   1. b\n2. c");
            assert_eq!(ev.cursor(), caret(1, 7));
            assert!(ev.intent(Intent::ShiftTab));
            assert_eq!(ev.text(), "1. a\n2. b\n3. c");
            // No previous sibling: Tab is consumed and changes nothing.
            ev.move_cursor(caret(0, 4));
            assert!(!ev.intent(Intent::Tab));
            assert_eq!(ev.text(), "1. a\n2. b\n3. c");
        }

        #[test]
        fn intents_act_on_the_bounds_of_a_word_or_line_selection() {
            let mut ev = editor("hello world");
            ev.move_cursor(caret(0, 2));
            ev.content.perform(Action::SelectWord);
            assert!(ev.intent(Intent::Backspace));
            assert_eq!(ev.text(), " world");
            assert!(ev.undo());
            assert_eq!(ev.selected_text().as_deref(), Some("hello"));

            let mut ev = editor("- one\n- two");
            ev.move_cursor(caret(0, 3));
            ev.content.perform(Action::SelectLine);
            assert_eq!(ev.selected_text().as_deref(), Some("- one"));
            assert!(ev.intent(Intent::Delete));
            assert_eq!(ev.text(), "\n- two");
        }

        #[test]
        fn smart_backspace_and_home_follow_the_markup() {
            let mut ev = editor("- a\n- [ ] ");
            ev.move_cursor(caret(1, 6));
            assert!(ev.intent(Intent::Backspace));
            assert_eq!(ev.text(), "- a\n- ");
            assert!(ev.intent(Intent::Backspace));
            assert_eq!(ev.text(), "- a\n  ");

            let mut ev = editor("> - [ ] task");
            ev.move_cursor(caret(0, 12));
            assert!(!ev.intent(Intent::Home { extend: false }));
            assert_eq!(ev.cursor(), caret(0, 8));
            assert!(!ev.intent(Intent::Home { extend: false }));
            assert_eq!(ev.cursor(), caret(0, 0));
            ev.move_cursor(caret(0, 12));
            ev.intent(Intent::Home { extend: true });
            assert_eq!(
                ev.cursor(),
                Cursor {
                    position: Position { line: 0, column: 8 },
                    selection: Some(Position { line: 0, column: 12 }),
                }
            );
            assert!(!ev.can_undo());
        }

        #[test]
        fn home_stops_at_a_wrapped_rows_start_first() {
            let text = format!("- {}", "wrapped words ".repeat(12));
            let mut ev = editor(&text);
            ev.content.lay_out(iced::Size::new(160.0, 400.0));
            ev.move_cursor(caret(0, text.len()));
            ev.intent(Intent::Home { extend: false });
            let row_start = ev.cursor().position.column;
            assert!(row_start > 2 && row_start < text.len(), "{row_start}");
            // At the row's start, Home plans: the text start, then column 0.
            ev.intent(Intent::Home { extend: false });
            assert_eq!(ev.cursor(), caret(0, 2));
            ev.intent(Intent::Home { extend: false });
            assert_eq!(ev.cursor(), caret(0, 0));
            // On the first row the planner's cycle runs at once.
            ev.move_cursor(caret(0, 5));
            ev.intent(Intent::Home { extend: false });
            assert_eq!(ev.cursor(), caret(0, 2));
        }

        #[test]
        fn without_assist_the_keys_edit_plain_text() {
            let mut ev = editor("- a");
            ev.set_markdown_assist(false);
            ev.move_cursor(caret(0, 3));
            assert!(ev.intent(Intent::Enter));
            assert_eq!(ev.text(), "- a\n");
            assert!(ev.intent(Intent::Tab));
            assert_eq!(ev.text(), "- a\n    ");
            assert!(ev.intent(Intent::ShiftTab));
            assert_eq!(ev.text(), "- a\n");
            assert!(ev.intent(Intent::Tab));
            assert!(ev.intent(Intent::Backspace));
            assert_eq!(ev.text(), "- a\n   ");
            ev.intent(Intent::Home { extend: false });
            assert_eq!(ev.cursor(), caret(1, 0));
            assert!(ev.intent(Intent::Backspace));
            assert_eq!(ev.text(), "- a   ");
            assert!(!ev.intent(Intent::ToggleTask));
            ev.replace_text("e\u{301}🇯🇵", false);
            ev.move_cursor(caret(0, "e\u{301}🇯🇵".len()));
            assert!(ev.intent(Intent::Backspace));
            assert_eq!(ev.text(), "e\u{301}");
            assert!(ev.intent(Intent::DeleteToTextStart));
            assert_eq!(ev.text(), "");
            // Tab over several lines indents them rather than replacing them.
            ev.replace_text("one\ntwo\nthree", false);
            ev.move_cursor(Cursor {
                position: Position { line: 2, column: 5 },
                selection: Some(Position { line: 0, column: 0 }),
            });
            assert!(ev.intent(Intent::Tab));
            assert_eq!(ev.text(), "    one\n    two\n    three");
            assert_eq!(ev.selected_text().as_deref(), Some("    one\n    two\n    three"));
            assert!(ev.intent(Intent::ShiftTab));
            assert_eq!(ev.text(), "one\ntwo\nthree");
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
    }
}
