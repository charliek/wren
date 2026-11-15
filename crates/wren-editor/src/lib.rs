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
mod proof;
mod widget;

#[cfg(test)]
mod fixtures;

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use iced::advanced::graphics::text::cosmic_text::Scroll;
use iced::widget::text_editor::{self, Action, Cursor, Edit, Motion, Position};
use iced::{Color, Element, Font};

pub use backend::{BackendKind, Placement};
pub use edit::Replace;
pub use keys::{KeyPolicy, KeyRoute};
#[cfg(any(test, feature = "test-support"))]
pub use proof::{DrawnMark, SpellFrame};
pub use proof::{MenuKey, MenuPress, Proofreader, SpellEvent, SpellMenu};
pub use widget::Measure;
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
    /// The fill behind selected text.
    pub text_selection: Color,
    /// Line numbers.
    pub gutter: Color,
    /// The cursor line's number.
    pub gutter_current: Color,
    /// The band behind the cursor's line.
    pub current_line: Color,
    /// The band behind fenced code blocks and the chip behind inline code.
    pub code_block: Color,
    pub caret: Color,
    /// The line through completed tasks and struck text.
    pub strike: Color,
    /// The zigzag under a misspelled word.
    pub misspelling: Color,
    /// The suggestions menu.
    pub menu: MenuStyle,
}

/// Colours and type of the suggestions menu. The editor knows no UI font of
/// its own, so the host supplies it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuStyle {
    pub background: Color,
    pub border: Color,
    /// The fill behind the selected row.
    pub selected: Color,
    pub text: Color,
    /// The inactive "No suggestions" row.
    pub muted: Color,
    pub font: Font,
    pub text_size: f32,
}

/// Colours of the Markdown syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// H3–H6 and emphasis text.
    pub text: Color,
    /// Code blocks and front matter.
    pub muted: Color,
    /// Markup characters, fences, comments, and link destinations.
    pub faint: Color,
    /// List markers and open task boxes.
    pub accent: Color,
    /// H1 text.
    pub heading: Color,
    /// H2 text.
    pub heading2: Color,
    /// Link text and bare URLs.
    pub link: Color,
    /// Inline code.
    pub code: Color,
    /// Block quote text.
    pub quote: Color,
    /// The text of completed tasks and `~~struck~~` text.
    pub completed: Color,
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
    /// The suggestions menu. See [`EditorView::spell`].
    Spell(SpellEvent),
}

// ---------------------------------------------------------------------------
// EditorView
// ---------------------------------------------------------------------------

/// Where the cursor, the selection, and the view were; see
/// [`EditorView::viewport`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    cursor: Cursor,
    scroll: Scroll,
}

impl Viewport {
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }
}

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
    /// Row height as a multiple of the text size.
    line_height: f32,
    font: Font,
    /// The text's `block::front_matter_end`, kept current after every
    /// change for the highlighter, which cannot look ahead.
    front_matter_end: Option<usize>,
    markdown_assist: bool,
    key_policy: Option<KeyPolicy>,
    proofreader: Option<Arc<dyn Proofreader>>,
    /// The open suggestions menu.
    menu: Option<SpellMenu>,
    /// The left press being held only closed the menu, so its drag selects
    /// nothing either (rule 4.4.5).
    press_closed_menu: bool,
    /// The marks the widget last drew.
    #[cfg(any(test, feature = "test-support"))]
    spell_frame: std::cell::RefCell<SpellFrame>,
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
        let mut view = Self {
            content: Content::new(kind, text),
            history: History::new(),
            find_query: String::new(),
            find_matches: Vec::new(),
            find_current: None,
            line_numbers: true,
            metrics: widget::Metrics::default(),
            line_height: widget::DEFAULT_LINE_HEIGHT,
            font: Font::MONOSPACE,
            front_matter_end: None,
            markdown_assist: true,
            key_policy: None,
            proofreader: None,
            menu: None,
            press_closed_menu: false,
            #[cfg(any(test, feature = "test-support"))]
            spell_frame: Default::default(),
        };
        view.refresh_front_matter();
        view
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
        if on != self.markdown_assist {
            self.menu = None;
        }
        self.markdown_assist = on;
    }

    pub fn markdown_assist(&self) -> bool {
        self.markdown_assist
    }

    /// Editor text size in points; row and caret heights scale with it.
    pub fn set_font_size(&mut self, size: f32) {
        self.set_metrics(widget::Metrics::new(size, self.line_height));
    }

    /// Row height as a multiple of the text size (1.7 by default), kept
    /// across text size changes.
    pub fn set_line_height(&mut self, ratio: f32) {
        self.line_height = ratio;
        self.set_metrics(widget::Metrics::new(self.metrics.text, ratio));
    }

    fn set_metrics(&mut self, metrics: widget::Metrics) {
        if metrics != self.metrics {
            self.menu = None;
        }
        self.metrics = metrics;
    }

    /// The editor font; bold and italic spans use its variants. Defaults to
    /// [`Font::MONOSPACE`].
    pub fn set_font(&mut self, font: Font) {
        if font != self.font {
            self.menu = None;
        }
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

    /// The height of a body row, in pixels.
    #[cfg(feature = "test-support")]
    pub fn row_height(&self) -> f32 {
        self.metrics.line
    }

    /// Show or hide the line-number gutter (shown by default).
    pub fn set_line_numbers(&mut self, line_numbers: bool) {
        if line_numbers != self.line_numbers {
            self.menu = None;
        }
        self.line_numbers = line_numbers;
    }

    // -----------------------------------------------------------------------
    // View
    // -----------------------------------------------------------------------

    /// Build the iced widget tree for the editor.
    ///
    /// Uses the editor font, rows the line-height ratio times the text size,
    /// soft-wrap, a 52 px line-number gutter unless hidden, `style` for the
    /// surface, and Markdown highlighting in `palette` colours. `measure`
    /// sets where the text wraps and whether it is centred.
    /// Returns `Element<'_, Message>` so the parent can map it to its own
    /// message type.
    pub fn view(&self, style: Style, palette: Palette, measure: Measure) -> Element<'_, Message> {
        let editor = TextEditor::new(
            &self.content,
            "note-editor",
            HighlighterSettings {
                palette,
                font: self.font,
                front_matter_end: self.front_matter_end,
            },
            style,
            self.line_numbers,
            self.metrics,
            self.markdown_assist,
        )
        .key_policy(self.key_policy.as_ref())
        .proofreader(self.proofreader.as_deref())
        .spell_menu(self.menu.as_ref())
        .measure(measure);
        #[cfg(any(test, feature = "test-support"))]
        let editor = editor.spell_frame(&self.spell_frame);
        editor.into()
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    /// Apply `action` to the content.
    ///
    /// Returns `true` only when the text changes. Non-edit actions break typing
    /// coalescing without discarding history; no-op edits also preserve redo.
    pub fn perform(&mut self, action: Action) -> bool {
        self.menu = None;
        if matches!(action, Action::Drag(_)) {
            if self.press_closed_menu {
                return false;
            }
        } else {
            self.press_closed_menu = false;
        }
        if action.is_edit() {
            let kind = edit_kind(&action);
            let now = Instant::now();
            let before = self.snapshot();
            let action = self.with_document_breaks(action);
            self.content.perform(action);
            let edited = before.cursor.position.line.min(
                before
                    .cursor
                    .selection
                    .map_or(usize::MAX, |anchor| anchor.line),
            );
            let cursor = self.content.cursor();
            self.keep_lines_apart(edited.saturating_sub(1)..=cursor.position.line + 1);
            if self.content.text() == before.text {
                self.history.break_group();
                return false;
            }
            self.history.before_edit(before, kind, now);
            self.text_changed();
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
        self.menu = None;
        let Some(order) = self.content.with_lines(|lines| edit::ordered(edits, lines)) else {
            return false;
        };
        let before = self.snapshot();
        for replace in order.iter().rev() {
            let ending = self.content.with_lines(|lines| {
                edit::break_ending(lines, replace.start, replace.end, &replace.text)
            });
            let text = edit::with_breaks(&replace.text, ending);
            self.content
                .replace_range(replace.start, replace.end, &text);
            let last = replace.start.line + wren_markdown::line_ranges(&text).count();
            self.keep_lines_apart(replace.start.line.saturating_sub(1)..=last);
        }
        let changed = self.content.text() != before.text;
        if changed {
            self.history
                .before_edit(before, EditKind::Replace, Instant::now());
            self.text_changed();
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
        self.menu = None;
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
        self.menu = None;
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
        self.menu = None;
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
        self.menu = None;
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
        self.menu = None;
        self.history.break_group();
        self.content.scroll_to_line(line, placement);
    }

    /// The first line in view, at least partly.
    pub fn first_visible_line(&self) -> usize {
        self.content.scroll().line
    }

    /// The cursor, selection, and scroll, to come back to with
    /// [`EditorView::restore_viewport`]. Taking it moves nothing, except that
    /// classic scrolls a word or line selection (a double or triple click)
    /// into view.
    pub fn viewport(&mut self) -> Viewport {
        let scroll = self.content.scroll();
        let raw = self.content.cursor();
        let cursor = if self.spans_its_selection(raw) {
            raw
        } else {
            let cursor = self.snapshot_cursor();
            // The snapshot moves the cursor, which scrolls to it.
            self.content.return_to(cursor, scroll);
            cursor
        };
        Viewport { cursor, scroll }
    }

    /// Return to `viewport`: the cursor and selection exactly (clamped into
    /// the text), and the scroll exactly on the owned engine, even with the
    /// cursor out of view. iced is best-effort: it scrolls by whole rows, and
    /// once the cursor has moved its next update scrolls to it, so a view
    /// that has to move up past the cursor puts the cursor's line on top.
    pub fn restore_viewport(&mut self, viewport: Viewport) {
        self.menu = None;
        self.history.break_group();
        let cursor = self.clamped(viewport.cursor);
        if cursor.selection.is_none() && self.content.cursor().selection.is_some() {
            // iced's move_to leaves the old selection intact when passed None.
            self.content.perform(Action::Move(Motion::Left));
        }
        self.content.return_to(cursor, viewport.scroll);
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
        self.text_changed();
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
        self.text_changed();
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
        self.menu = None;
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
        self.menu = None;
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
    // Spell check
    // -----------------------------------------------------------------------

    /// The host's proofreader, or `None` to check nothing. Only the owned
    /// engine uses it: the widget asks it about the rows it draws, each
    /// frame, so a new one or a changed word list shows on the next frame.
    pub fn set_proofreader(&mut self, proofreader: Option<Arc<dyn Proofreader>>) {
        let same = match (&self.proofreader, &proofreader) {
            (Some(old), Some(new)) => std::ptr::addr_eq(Arc::as_ptr(old), Arc::as_ptr(new)),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.menu = None;
        }
        self.proofreader = proofreader;
    }

    /// Carry out a suggestions-menu event. Returns whether the text changed,
    /// as [`EditorView::perform`] does.
    ///
    /// Opening finds the misspelled word that contains or touches the caret
    /// ([`SpellEvent::OpenAtCaret`]) or lies under a point relative to the
    /// text origin ([`SpellEvent::OpenAt`]), asks the proofreader for its
    /// suggestions, and scrolls the word's first row into view if it is
    /// not; the caret and the selection stay where they are. With no such
    /// word, on the iced engine, or with no proofreader, nothing opens.
    /// Accepting replaces the word through [`EditorView::apply`], so it is
    /// one undo step. A menu key ([`SpellEvent::Key`]) acts on the open menu,
    /// or with none open is handled as the editor handles that press, which
    /// may edit the text too; these two are the events that can return
    /// true. A pointer press ([`SpellEvent::Press`]) closes the open menu,
    /// or with none open clicks or opens one as that press would.
    pub fn spell(&mut self, event: SpellEvent) -> bool {
        match event {
            SpellEvent::Key(key) if self.menu.is_some() => return self.spell(key.event()),
            SpellEvent::Key(key) => return self.press_without_menu(key),
            SpellEvent::Press(press) => return self.press(press),
            SpellEvent::OpenAtCaret => {
                let caret = self.content.cursor().position;
                let mark = proof::mark_at_caret(self.marks_at(caret.line), caret.column);
                self.open_menu(mark.map(|mark| (caret.line, mark)));
            }
            SpellEvent::OpenAt(point) => {
                let target = self
                    .content
                    .mark_under(point, |line, _| self.marks_at(line));
                self.open_menu(target);
            }
            SpellEvent::Move(delta) => {
                if let Some(menu) = &mut self.menu {
                    menu.step(delta);
                }
            }
            SpellEvent::Hover(row) => {
                if let Some(menu) = &mut self.menu {
                    menu.hover(row);
                }
            }
            SpellEvent::Accept(row) => return self.accept(row),
            SpellEvent::AcceptSelected => match self.menu.as_ref().and_then(SpellMenu::selected) {
                Some(row) => return self.accept(row),
                None => self.menu = None,
            },
            SpellEvent::Close => {
                self.menu = None;
            }
        }
        false
    }

    /// Replace the menu's word with its suggestion in `row`, as one undo
    /// step with the caret after it, and close the menu. A row with no
    /// suggestion, or a word no longer at its mark, closes it and changes
    /// nothing.
    fn accept(&mut self, row: usize) -> bool {
        let Some(menu) = self.menu.take() else {
            return false;
        };
        let Some(suggestion) = menu.items().get(row) else {
            return false;
        };
        let line = menu.line();
        let mark = menu.mark();
        let unchanged = self.content.with_lines(|lines| {
            lines
                .get(line)
                .is_some_and(|text| text.text.get(mark.clone()) == Some(menu.word()))
        });
        if !unchanged {
            return false;
        }
        let at = |column| Position { line, column };
        let edit = Replace {
            start: at(mark.start),
            end: at(mark.end),
            text: suggestion.clone(),
        };
        let caret = Cursor {
            position: at(mark.start + suggestion.len()),
            selection: None,
        };
        self.apply(&[edit], caret)
    }

    /// The open suggestions menu, if any.
    pub fn spell_menu(&self) -> Option<&SpellMenu> {
        self.menu.as_ref()
    }

    /// Close the suggestions menu. Returns whether one was open.
    pub fn close_spell_menu(&mut self) -> bool {
        self.menu.take().is_some()
    }

    /// Open the menu for the misspelled word at `target`, a line and a byte
    /// range of it; with no target, or none the proofreader can still be
    /// asked about, whatever menu was open closes.
    fn open_menu(&mut self, target: Option<(usize, Range<usize>)>) {
        self.menu = None;
        let Some((line, mark)) = target else {
            return;
        };
        let Some(proofreader) = self.proofreader.clone() else {
            return;
        };
        let Some(text) = self
            .content
            .with_lines(|lines| lines.get(line).map(|line| line.text.into_owned()))
        else {
            return;
        };
        let Some(word) = text.get(mark.clone()) else {
            return;
        };
        let items = proof::offered(
            word,
            proofreader.suggestions(&text, mark.clone(), proof::SUGGESTIONS),
        );
        self.content.reveal(Position {
            line,
            column: mark.start,
        });
        self.menu = Some(SpellMenu::new(line, mark, word.to_owned(), items));
    }

    /// The marks and the menu as the last frame drew them.
    #[cfg(any(test, feature = "test-support"))]
    pub fn spell_frame(&self) -> SpellFrame {
        self.spell_frame.borrow().clone()
    }

    /// The text of `line` without its ending, so a host can name the word a
    /// mark of [`spell_frame`](Self::spell_frame) covers.
    #[cfg(any(test, feature = "test-support"))]
    pub fn line_text(&self, line: usize) -> Option<String> {
        self.content
            .with_lines(|lines| lines.get(line).map(|text| text.text.to_string()))
    }

    /// The editor's own key routing run on one press, without a window: the
    /// messages the focused editor would publish for `key` with `modifiers`,
    /// in order. A key that closes the open menu and is then handled as if
    /// it had not been open gives two (rule 4.4.4); Escape with the menu
    /// open gives none. The key is its own physical key, and a character
    /// types unless Ctrl or the logo key is held (or Alt, off macOS). The
    /// clipboard is not there, so Copy and Paste give nothing and Cut only
    /// its delete; dropping focus is no message.
    #[cfg(feature = "test-support")]
    pub fn key_message(
        &self,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Vec<Message> {
        let menu = self.menu.as_ref().map(proof::OpenMenu::of);
        let selection = self.has_selection();
        let mut out = Vec::new();
        match keys::route(key_press(key, modifiers), self.key_context(), menu) {
            None => {}
            Some(keys::Routed::Binding(binding)) => keys::messages(binding, selection, &mut out),
            Some(keys::Routed::Close(binding)) => {
                out.push(Message::Spell(SpellEvent::Close));
                if let Some(binding) = binding {
                    keys::messages(binding, selection, &mut out);
                }
            }
        }
        out
    }

    /// Close the open menu for a pointer press in the editor, which does
    /// nothing more (rule 4.4.5), or with none open carry the press out as
    /// the widget does with no menu. Returns whether the text changed.
    fn press(&mut self, press: MenuPress) -> bool {
        if self.menu.take().is_some() {
            self.press_closed_menu = matches!(press, MenuPress::Left(..));
            return false;
        }
        match press {
            MenuPress::Left(point, kind) => self.perform(match kind {
                iced::advanced::mouse::click::Kind::Single => Action::Click(point),
                iced::advanced::mouse::click::Kind::Double => Action::SelectWord,
                iced::advanced::mouse::click::Kind::Triple => Action::SelectLine,
            }),
            MenuPress::Right(point) => self.spell(SpellEvent::OpenAt(point)),
        }
    }

    /// Handle a press of `key` as the focused editor does with no menu
    /// open, carrying out what it publishes. Returns whether the text
    /// changed.
    fn press_without_menu(&mut self, key: MenuKey) -> bool {
        use iced::keyboard::key::Named;

        let named = match key {
            MenuKey::Up => Named::ArrowUp,
            MenuKey::Down => Named::ArrowDown,
            MenuKey::Enter => Named::Enter,
        };
        let press = key_press(
            iced::keyboard::Key::Named(named),
            iced::keyboard::Modifiers::empty(),
        );
        let Some(binding) = keys::key_binding(press, self.key_context()) else {
            return false;
        };
        let mut messages = Vec::new();
        keys::messages(binding, self.has_selection(), &mut messages);
        let mut changed = false;
        for message in messages {
            changed |= match message {
                Message::Action(action) => self.perform(action),
                Message::Intent(intent) => self.intent(intent),
                Message::Spell(event) => self.spell(event),
                Message::Undo => self.undo(),
                Message::Redo => self.redo(),
                // Only Ctrl or Cmd+F routes to Find, never a plain key.
                Message::Find => false,
            };
        }
        changed
    }

    /// What key routing needs from the editor, as no input method composes.
    fn key_context(&self) -> keys::Context<'_> {
        keys::Context {
            markdown_assist: self.markdown_assist,
            composing: false,
            policy: self.key_policy.as_ref(),
        }
    }

    fn has_selection(&self) -> bool {
        self.content
            .selection()
            .is_some_and(|text| !text.is_empty())
    }

    /// The misspellings of `line`, found without the widget: its prose comes
    /// from a fresh pass over the lines down to it (§3.3), the same marks a
    /// frame draws there. Empty on the iced engine or with no proofreader,
    /// which is then not asked.
    fn marks_at(&self, line: usize) -> Vec<Range<usize>> {
        let Some(proofreader) = self
            .proofreader
            .as_deref()
            .filter(|_| self.content.kind() == BackendKind::Owned)
        else {
            return Vec::new();
        };
        self.content.with_lines(|lines| {
            let Some(text) = lines.get(line) else {
                return Vec::new();
            };
            let prose = highlighter::prose_at(&lines, line, self.front_matter_end);
            proof::line_marks(proofreader, &text.text, &prose)
        })
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

    /// Whether `raw`'s two ends bound the selected text, as they do for a
    /// plain selection; a word or line selection extends past them.
    fn spans_its_selection(&self, raw: Cursor) -> bool {
        let Some(anchor) = raw.selection else {
            return true;
        };
        let Some(selected) = self.content.selection() else {
            return true;
        };
        let (start, end) =
            if (anchor.line, anchor.column) <= (raw.position.line, raw.position.column) {
                (anchor, raw.position)
            } else {
                (raw.position, anchor)
            };
        self.content.with_lines(|lines| {
            let mut spanned = String::new();
            for line in start.line..=end.line {
                let Some(line_text) = lines.get(line).map(|line| line.text) else {
                    return false;
                };
                let from = if line == start.line { start.column } else { 0 };
                let to = if line == end.line {
                    end.column
                } else {
                    line_text.len()
                };
                let Some(part) = line_text.get(from..to) else {
                    return false;
                };
                if line > start.line {
                    // cosmic-text copies every line ending as LF.
                    spanned.push('\n');
                }
                spanned.push_str(part);
            }
            spanned == selected
        })
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

    /// Rewrite the ending of each empty line in `range` that would run into
    /// the line above's on disk, keeping the cursor. Line count and text are
    /// unchanged, so positions elsewhere stay valid.
    fn keep_lines_apart(&mut self, range: std::ops::RangeInclusive<usize>) {
        let cursor = self.content.cursor();
        let mut moved = false;
        while let Some((line, ending)) = self
            .content
            .with_lines(|lines| edit::fused_line(lines, range.clone()))
        {
            let start = text_editor::Position { line, column: 0 };
            let end = text_editor::Position {
                line: line + 1,
                column: 0,
            };
            self.content.replace_range(start, end, ending.as_str());
            moved = true;
        }
        if moved {
            self.select(cursor);
        }
    }

    /// `action` with the line breaks it inserts written in the ending of the
    /// line they split, so Enter, paste, and IME commits match the note.
    fn with_document_breaks(&self, action: Action) -> Action {
        let ending = |text: &str| {
            let cursor = self.content.cursor();
            let anchor = cursor.selection.unwrap_or(cursor.position);
            let key = |at: &text_editor::Position| (at.line, at.column);
            let start = std::cmp::min_by_key(anchor, cursor.position, key);
            let end = std::cmp::max_by_key(anchor, cursor.position, key);
            self.content
                .with_lines(|lines| edit::break_ending(lines, start, end, text))
        };
        match action {
            Action::Edit(Edit::Enter | Edit::Insert('\n')) => match ending("\n") {
                text_editor::LineEnding::Lf => Action::Edit(Edit::Enter),
                other => Action::Edit(Edit::Paste(Arc::new(other.as_str().to_owned()))),
            },
            Action::Edit(Edit::Paste(text)) => match edit::with_breaks(&text, ending(&text)) {
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
        self.text_changed();
    }

    fn clamped(&self, cursor: Cursor) -> Cursor {
        self.content.with_lines(|lines| clamp_cursor(cursor, lines))
    }

    /// Refresh what derives from the text after it changed.
    fn text_changed(&mut self) {
        self.menu = None;
        self.refresh_front_matter();
        self.recompute_find();
    }

    fn refresh_front_matter(&mut self) {
        self.front_matter_end = self.content.with_lines(|lines| {
            (lines.len() > 0).then(|| wren_markdown::block::front_matter_end(&lines))?
        });
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

/// A focused press of `key` with `modifiers`, as a window would deliver it:
/// its own physical key, and the text it types, which a character types
/// unless Ctrl or the logo key is held (or Alt, off macOS).
fn key_press(
    key: iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> text_editor::KeyPress {
    use iced::keyboard::key::{Named, NativeCode, Physical};
    use iced::keyboard::Key;

    let typing = !(modifiers.control()
        || modifiers.logo()
        || (modifiers.alt() && !cfg!(target_os = "macos")));
    let text = match key.as_ref() {
        Key::Character(c) if typing => Some(c.into()),
        Key::Named(Named::Space) if typing => Some(" ".into()),
        Key::Named(Named::Enter) => Some("\r".into()),
        Key::Named(Named::Tab) => Some("\t".into()),
        _ => None,
    };
    text_editor::KeyPress {
        modified_key: key.clone(),
        key,
        physical_key: Physical::Unidentified(NativeCode::Unidentified),
        modifiers,
        text,
        status: text_editor::Status::Focused { is_hovered: false },
    }
}

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

    // ── Spell check ─────────────────────────────────────────────────────────

    /// `(line, range)` of every mark a frame of the whole text draws.
    fn drawn(ev: &mut EditorView) -> Vec<(usize, Range<usize>)> {
        let proofreader = ev.proofreader.clone();
        ev.content
            .drawn_marks(
                iced::Size::new(2000.0, 2000.0),
                ev.front_matter_end,
                proofreader.as_deref(),
                None,
            )
            .into_iter()
            .map(|mark| (mark.line, mark.range))
            .collect()
    }

    /// `(line, range)` of every mark [`EditorView::marks_at`] finds.
    fn on_demand(ev: &EditorView) -> Vec<(usize, Range<usize>)> {
        (0..ev.line_count())
            .flat_map(|line| ev.marks_at(line).into_iter().map(move |mark| (line, mark)))
            .collect()
    }

    #[test]
    fn the_acceptance_note_is_marked_alike_on_demand_and_when_drawn() {
        let proofreader = Arc::new(fixtures::Rejects::new(&[
            "Spel",
            "chek",
            "apear",
            "replacments",
            "spelbook",
            "bundeled",
            "desing",
            "spel",
            "CommonMark",
            "mispelled",
            "knwon",
            "misteak",
            "wrd",
            "labl",
            "Titel",
        ]));
        let mut ev = EditorView::with_backend(BackendKind::Owned, fixtures::SPELL_NOTE);
        ev.set_proofreader(Some(proofreader.clone()));
        let marks = on_demand(&ev);
        assert_eq!(drawn(&mut ev), marks);
        let lines: Vec<&str> = fixtures::SPELL_NOTE.split('\n').collect();
        let words: Vec<&str> = marks
            .iter()
            .map(|(line, range)| &lines[*line][range.clone()])
            .collect();
        assert_eq!(
            words,
            [
                "chek",
                "apear",
                "replacments",
                "bundeled",
                "desing",
                "CommonMark",
                "mispelled",
                "knwon",
                "misteak"
            ]
        );
        assert_eq!(marks[0].0, 3, "the heading's, not the front matter's");
    }

    #[test]
    fn on_demand_marks_need_the_owned_engine_and_a_proofreader() {
        let proofreader = Arc::new(fixtures::Rejects::new(&["teh"]));
        let mut ev = EditorView::with_backend(BackendKind::Iced, "teh cat\n# teh");
        assert!(ev.marks_at(0).is_empty());
        ev.set_proofreader(Some(proofreader.clone()));
        assert!(on_demand(&ev).is_empty());
        assert!(drawn(&mut ev).is_empty());
        assert_eq!(proofreader.calls(), 0);

        let text = ev.text();
        ev.set_backend(BackendKind::Owned, &text);
        assert_eq!(on_demand(&ev), [(0, 0..3), (1, 2..5)]);
        assert_eq!(ev.marks_at(2), []);
        ev.set_proofreader(None);
        assert!(on_demand(&ev).is_empty());
        assert!(drawn(&mut ev).is_empty());
    }

    #[test]
    fn marks_follow_the_text_through_edits_undo_and_replacement() {
        let proofreader = Arc::new(fixtures::Rejects::new(&["teh", "cta"]));
        let mut ev = EditorView::with_backend(BackendKind::Owned, "the cat\r\nteh dog");
        ev.set_proofreader(Some(proofreader));
        let check = |ev: &mut EditorView, expected: &[(usize, Range<usize>)]| {
            assert_eq!(on_demand(ev), expected, "{:?}", ev.text());
            assert_eq!(drawn(ev), expected, "{:?}", ev.text());
        };
        check(&mut ev, &[(1, 0..3)]);
        ev.move_cursor(at(0, 0));
        assert!(ev.perform(Action::Edit(Edit::Paste(Arc::new("cta ".into())))));
        check(&mut ev, &[(0, 0..3), (1, 0..3)]);
        assert!(ev.intent(Intent::Enter));
        check(&mut ev, &[(0, 0..3), (2, 0..3)]);
        assert!(ev.undo());
        check(&mut ev, &[(0, 0..3), (1, 0..3)]);
        assert!(ev.undo());
        check(&mut ev, &[(1, 0..3)]);
        assert!(ev.redo());
        check(&mut ev, &[(0, 0..3), (1, 0..3)]);
        // A line turned into code loses its marks; the text is replaced.
        ev.move_cursor(at(0, 0));
        assert!(ev.perform(Action::Edit(Edit::Paste(Arc::new("    ".into())))));
        check(&mut ev, &[(1, 0..3)]);
        ev.replace_text("---\ntitle: teh\n---\nfine", false);
        check(&mut ev, &[]);
    }

    // ── The suggestions menu ───────────────────────────────────────────────

    const TEH: &[(&str, &[&str])] = &[("teh", &["the", "tech", "ten", "tea", "tee", "Ted"])];

    /// An owned-engine editor over `text`, checked by a proofreader that
    /// rejects `teh` and `zzz` and corrects `teh`.
    fn checked(text: &str) -> (EditorView, Arc<fixtures::Rejects>) {
        let proofreader = Arc::new(fixtures::Rejects::new(&["teh", "zzz"]).suggesting(TEH));
        let mut ev = EditorView::with_backend(BackendKind::Owned, text);
        ev.set_proofreader(Some(proofreader.clone()));
        (ev, proofreader)
    }

    /// `(line, mark, word, items, selected)` of an open menu.
    type MenuState = (usize, Range<usize>, String, Vec<String>, Option<usize>);

    /// The state of the open menu.
    fn menu_of(ev: &EditorView) -> Option<MenuState> {
        ev.spell_menu().map(|menu| {
            (
                menu.line(),
                menu.mark(),
                menu.word().to_owned(),
                menu.items().to_vec(),
                menu.selected(),
            )
        })
    }

    fn select(anchor: (usize, usize), head: (usize, usize)) -> Cursor {
        Cursor {
            position: Position {
                line: head.0,
                column: head.1,
            },
            selection: Some(Position {
                line: anchor.0,
                column: anchor.1,
            }),
        }
    }

    #[test]
    fn the_keyboard_opens_the_menu_for_the_word_containing_or_touching_the_caret() {
        let text = "a cat\nteh cat teh";
        let five: Vec<String> = ["the", "tech", "ten", "tea", "tee"]
            .map(String::from)
            .into();
        for (column, mark) in [(0, 0..3), (2, 0..3), (3, 0..3), (8, 8..11), (11, 8..11)] {
            let (mut ev, proofreader) = checked(text);
            ev.move_cursor(at(1, column));
            assert!(!ev.spell(SpellEvent::OpenAtCaret));
            assert_eq!(
                menu_of(&ev),
                Some((1, mark.clone(), "teh".into(), five.clone(), Some(0))),
                "{column}"
            );
            // Five at most, asked of the proofreader with the whole line.
            assert_eq!(proofreader.asked(), [("teh cat teh".into(), mark, 5)]);
            // Nothing moves, the caret at a mark's end included (rule
            // 4.3.3 hides that mark, but it is still the target).
            assert_eq!(ev.cursor(), at(1, column), "{column}");
            assert_eq!(ev.text(), text);
        }
        // Past the space after a word, inside a correct one, or on a line
        // with no mark, nothing opens and nobody is asked.
        for (line, column) in [(1, 4), (1, 6), (0, 3)] {
            let (mut ev, proofreader) = checked(text);
            ev.move_cursor(at(line, column));
            assert!(!ev.spell(SpellEvent::OpenAtCaret));
            assert!(ev.spell_menu().is_none(), "{line}:{column}");
            assert!(proofreader.asked().is_empty());
        }
    }

    #[test]
    fn the_caret_of_a_selection_is_its_moving_end() {
        let text = "teh cat teh";
        let (mut ev, _) = checked(text);
        for (selection, mark) in [
            (select((0, 1), (0, 9)), 8..11),
            (select((0, 9), (0, 1)), 0..3),
            (select((0, 5), (0, 3)), 0..3),
        ] {
            ev.move_cursor(selection);
            ev.spell(SpellEvent::OpenAtCaret);
            assert_eq!(ev.spell_menu().map(SpellMenu::mark), Some(mark));
            assert_eq!(ev.cursor(), selection, "the selection stays");
            assert!(ev.close_spell_menu());
        }
        // A selection ending inside a correct word opens nothing.
        ev.move_cursor(select((0, 0), (0, 5)));
        ev.spell(SpellEvent::OpenAtCaret);
        assert!(ev.spell_menu().is_none());
    }

    #[test]
    fn a_word_without_suggestions_opens_a_menu_with_nothing_selected() {
        let (mut ev, _) = checked("zzz");
        ev.move_cursor(at(0, 1));
        ev.spell(SpellEvent::OpenAtCaret);
        assert_eq!(
            menu_of(&ev),
            Some((0, 0..3, "zzz".into(), Vec::new(), None))
        );
        // Moving does nothing: there is no row to select.
        ev.spell(SpellEvent::Move(1));
        ev.spell(SpellEvent::Hover(0));
        assert_eq!(ev.spell_menu().and_then(SpellMenu::selected), None);
    }

    #[test]
    fn the_menu_opens_only_on_the_owned_engine_with_a_proofreader() {
        let proofreader = Arc::new(fixtures::Rejects::new(&["teh"]).suggesting(TEH));
        let mut ev = EditorView::with_backend(BackendKind::Iced, "teh");
        ev.set_proofreader(Some(proofreader.clone()));
        ev.content.lay_out(iced::Size::new(400.0, 200.0));
        ev.move_cursor(at(0, 1));
        ev.spell(SpellEvent::OpenAtCaret);
        ev.spell(SpellEvent::OpenAt(iced::Point::new(10.0, 5.0)));
        assert!(ev.spell_menu().is_none());
        assert_eq!(proofreader.calls(), 0);
        assert!(proofreader.asked().is_empty());

        let mut ev = EditorView::with_backend(BackendKind::Owned, "teh");
        ev.move_cursor(at(0, 1));
        ev.spell(SpellEvent::OpenAtCaret);
        assert!(ev.spell_menu().is_none(), "no proofreader");
    }

    #[test]
    fn the_selection_moves_and_stops_at_the_ends() {
        let (mut ev, _) = checked("teh");
        ev.spell(SpellEvent::OpenAtCaret);
        let selected = |ev: &EditorView| ev.spell_menu().and_then(SpellMenu::selected);
        assert_eq!(selected(&ev), Some(0));
        ev.spell(SpellEvent::Move(-1));
        assert_eq!(selected(&ev), Some(0));
        ev.spell(SpellEvent::Move(1));
        ev.spell(SpellEvent::Move(1));
        assert_eq!(selected(&ev), Some(2));
        ev.spell(SpellEvent::Move(i32::MAX));
        assert_eq!(selected(&ev), Some(4));
        ev.spell(SpellEvent::Move(1));
        assert_eq!(selected(&ev), Some(4));
        ev.spell(SpellEvent::Move(i32::MIN));
        assert_eq!(selected(&ev), Some(0));
        ev.spell(SpellEvent::Hover(3));
        assert_eq!(selected(&ev), Some(3));
        ev.spell(SpellEvent::Hover(5));
        assert_eq!(selected(&ev), Some(3), "past the last row");
        assert_eq!(ev.text(), "teh");

        assert!(!ev.spell(SpellEvent::Close));
        assert!(ev.spell_menu().is_none());
        assert!(!ev.close_spell_menu());
        // Events for a closed menu do nothing.
        ev.spell(SpellEvent::Move(1));
        ev.spell(SpellEvent::Hover(1));
        assert!(ev.spell_menu().is_none());
    }

    #[test]
    fn opening_again_reads_the_word_afresh() {
        let (mut ev, proofreader) = checked("teh teh");
        ev.move_cursor(at(0, 0));
        ev.spell(SpellEvent::OpenAtCaret);
        ev.spell(SpellEvent::Move(2));
        ev.move_cursor(at(0, 5));
        ev.spell(SpellEvent::OpenAtCaret);
        assert_eq!(
            ev.spell_menu().map(|menu| (menu.mark(), menu.selected())),
            Some((4..7, Some(0)))
        );
        assert_eq!(proofreader.asked().len(), 2);
        // An opening that finds no word closes the open menu.
        ev.content.lay_out(iced::Size::new(600.0, 400.0));
        ev.spell(SpellEvent::OpenAt(iced::Point::new(1.0, 12.0)));
        assert_eq!(ev.spell_menu().map(SpellMenu::mark), Some(0..3));
        ev.spell(SpellEvent::OpenAt(iced::Point::new(3.5 * CELL, 12.0)));
        assert!(ev.spell_menu().is_none());
        assert_eq!(ev.cursor(), at(0, 5));
    }

    /// The JetBrains Mono advance at 14 px.
    const CELL: f32 = 8.4;

    #[test]
    fn a_point_opens_the_menu_for_the_word_under_it() {
        let (mut ev, _) = checked("cat teh\n\n- [x] done teh\nzzz");
        ev.content.lay_out(iced::Size::new(600.0, 400.0));
        let point = |cells: f32, row: f32| iced::Point::new(cells * CELL, row * 24.0 + 12.0);
        for (at_point, target) in [
            (point(4.5, 0.0), Some((0, 4..7))),
            (point(4.05, 0.0), Some((0, 4..7))),
            (point(6.9, 0.0), Some((0, 4..7))),
            (point(1.0, 0.0), None),
            (point(3.5, 0.0), None),
            (point(7.5, 0.0), None),
            (point(1.0, 1.0), None),
            (point(12.0, 2.0), Some((2, 11..14))),
            (point(1.0, 3.0), Some((3, 0..3))),
            (point(1.0, 4.0), None),
            (iced::Point::new(10.0, -2.0), None),
        ] {
            ev.close_spell_menu();
            ev.spell(SpellEvent::OpenAt(at_point));
            assert_eq!(
                ev.spell_menu().map(|menu| (menu.line(), menu.mark())),
                target,
                "{at_point:?}"
            );
        }
        assert_eq!(ev.cursor(), at(0, 0));
    }

    #[test]
    fn opening_scrolls_the_words_first_row_into_view() {
        let mut text: String = (0..60).map(|i| format!("line {i}\n")).collect();
        text.push_str("teh end");
        let (mut ev, _) = checked(&text);
        let size = iced::Size::new(400.0, 240.0);
        ev.content.lay_out(size);
        // The caret on the last line, the view back at the top.
        let top = ev.content.scroll();
        ev.content.return_to(at(60, 1), top);
        ev.content.lay_out(size);
        assert_eq!(ev.first_visible_line(), 0);
        ev.spell(SpellEvent::OpenAtCaret);
        assert!(ev.spell_menu().is_some());
        ev.content.lay_out(size);
        // Ten rows in view, the last of them the word's.
        assert_eq!(ev.content.scroll().line, 51, "{:?}", ev.content.scroll());
        assert_eq!(ev.content.scroll().vertical, 0.0);
        assert_eq!(ev.cursor(), at(60, 1));

        // A word already in view does not scroll it.
        let scroll = ev.content.scroll();
        ev.spell(SpellEvent::OpenAtCaret);
        ev.content.lay_out(size);
        assert_eq!(ev.content.scroll(), scroll);
    }

    #[test]
    fn opening_brings_a_half_hidden_row_fully_into_view() {
        let text = format!("teh {}", "word ".repeat(200));
        let (mut ev, _) = checked(&text);
        let size = iced::Size::new(400.0, 240.0);
        ev.content.lay_out(size);
        // Half the first row scrolled off the top, the caret on it.
        let mut scroll = ev.content.scroll();
        scroll.vertical = 12.0;
        ev.content.return_to(at(0, 1), scroll);
        ev.content.lay_out(size);
        assert_eq!(ev.content.scroll().vertical, 12.0);
        ev.spell(SpellEvent::OpenAtCaret);
        ev.content.lay_out(size);
        assert_eq!(ev.content.scroll().vertical, 0.0);
        assert_eq!(ev.cursor(), at(0, 1));
    }

    /// The menu open on `teh` at the start of "teh cat".
    fn opened() -> (EditorView, Arc<fixtures::Rejects>) {
        let (mut ev, proofreader) = checked("teh cat");
        ev.move_cursor(at(0, 1));
        ev.spell(SpellEvent::OpenAtCaret);
        assert!(ev.spell_menu().is_some());
        (ev, proofreader)
    }

    #[cfg(feature = "test-support")]
    mod keys_on_a_menu {
        use iced::keyboard::key::Named;
        use iced::keyboard::{Key, Modifiers};

        use super::*;

        /// Carry out `message` as the host does, returning whether the text
        /// changed.
        fn handle(ev: &mut EditorView, message: Message) -> bool {
            match message {
                Message::Action(action) => ev.perform(action),
                Message::Intent(intent) => ev.intent(intent),
                Message::Spell(event) => ev.spell(event),
                Message::Undo => ev.undo(),
                Message::Redo => ev.redo(),
                Message::Find => false,
            }
        }

        fn spell_events(messages: &[Message]) -> Vec<SpellEvent> {
            messages
                .iter()
                .filter_map(|message| match message {
                    Message::Spell(event) => Some(*event),
                    _ => None,
                })
                .collect()
        }

        #[test]
        fn up_down_and_enter_drive_the_menu_and_never_the_text() {
            let (mut ev, _) = opened();
            let none = Modifiers::empty();
            let down = ev.key_message(Key::Named(Named::ArrowDown), none);
            assert_eq!(spell_events(&down), [SpellEvent::Move(1)]);
            assert_eq!(down.len(), 1);
            for message in down {
                handle(&mut ev, message);
            }
            assert_eq!(ev.spell_menu().and_then(SpellMenu::selected), Some(1));
            let up = ev.key_message(Key::Named(Named::ArrowUp), none);
            assert_eq!(spell_events(&up), [SpellEvent::Move(-1)]);
            let enter = ev.key_message(Key::Named(Named::Enter), none);
            assert_eq!(spell_events(&enter), [SpellEvent::AcceptSelected]);
            assert_eq!(ev.text(), "teh cat");
            assert_eq!(ev.cursor(), at(0, 1));
            assert_eq!(enter.len(), 1);
            for message in enter {
                assert!(handle(&mut ev, message));
            }
            assert_eq!(ev.text(), "tech cat");
            assert_eq!(ev.cursor(), at(0, 4));
            assert!(ev.spell_menu().is_none());
        }

        #[test]
        fn escape_with_the_menu_open_produces_nothing_and_keeps_it_open() {
            let (mut ev, _) = opened();
            for modifiers in [Modifiers::empty(), Modifiers::SHIFT] {
                assert!(ev
                    .key_message(Key::Named(Named::Escape), modifiers)
                    .is_empty());
            }
            // A modifier pressed alone, the same.
            assert!(ev
                .key_message(Key::Named(Named::Shift), Modifiers::SHIFT)
                .is_empty());
            assert!(ev.spell_menu().is_some());
            // The host closes it, then Escape is the editor's again, which
            // drops focus: no message.
            assert!(ev.close_spell_menu());
            assert!(ev
                .key_message(Key::Named(Named::Escape), Modifiers::empty())
                .is_empty());
        }

        #[test]
        fn enter_with_nothing_to_offer_closes_the_menu_and_leaves_the_text() {
            let (mut ev, _) = checked("zzz");
            ev.spell(SpellEvent::OpenAtCaret);
            for key in [Named::ArrowDown, Named::ArrowUp] {
                for message in ev.key_message(Key::Named(key), Modifiers::empty()) {
                    assert!(!handle(&mut ev, message));
                }
            }
            assert!(ev.spell_menu().is_some());
            let enter = ev.key_message(Key::Named(Named::Enter), Modifiers::empty());
            assert_eq!(spell_events(&enter), [SpellEvent::AcceptSelected]);
            assert_eq!(enter.len(), 1);
            for message in enter {
                assert!(!handle(&mut ev, message));
            }
            assert!(ev.spell_menu().is_none());
            assert_eq!(ev.text(), "zzz");
        }

        #[test]
        fn any_other_key_closes_the_menu_and_then_acts_once() {
            let none = Modifiers::empty();
            for (key, modifiers, text, column) in [
                (Key::Character("x".into()), none, "txeh cat", 2),
                (Key::Named(Named::ArrowRight), none, "teh cat", 2),
                (
                    Key::Named(Named::ArrowRight),
                    Modifiers::SHIFT,
                    "teh cat",
                    2,
                ),
                (Key::Named(Named::Enter), Modifiers::SHIFT, "t\neh cat", 0),
                (Key::Named(Named::Backspace), none, "eh cat", 0),
            ] {
                let (mut ev, _) = opened();
                let messages = ev.key_message(key.clone(), modifiers);
                assert!(
                    matches!(messages.first(), Some(Message::Spell(SpellEvent::Close))),
                    "{key:?}: {messages:?}"
                );
                assert_eq!(messages.len(), 2, "{key:?}: {messages:?}");
                for message in messages {
                    handle(&mut ev, message);
                }
                assert!(ev.spell_menu().is_none(), "{key:?}");
                assert_eq!(ev.text(), text, "{key:?}");
                assert_eq!(ev.cursor().position.column, column, "{key:?}");
            }
            // Undo, after an edit: closed, then one undo step.
            let (mut ev, _) = checked("teh cat");
            ev.move_cursor(at(0, 7));
            ev.perform(Action::Edit(Edit::Insert('s')));
            ev.spell(SpellEvent::OpenAtCaret);
            ev.move_cursor(at(0, 1));
            ev.spell(SpellEvent::OpenAtCaret);
            let undo = ev.key_message(Key::Character("z".into()), Modifiers::COMMAND);
            assert!(matches!(
                undo.as_slice(),
                [Message::Spell(SpellEvent::Close), Message::Undo]
            ));
            for message in undo {
                handle(&mut ev, message);
            }
            assert_eq!(ev.text(), "teh cat");
        }

        #[test]
        fn a_host_shortcut_closes_the_menu_and_is_left_to_the_host() {
            let (mut ev, _) = opened();
            ev.set_key_policy(Arc::new(|key: &Key, _, modifiers: Modifiers| {
                if modifiers == Modifiers::ALT && *key == Key::Character("p".into()) {
                    KeyRoute::Host
                } else if modifiers == Modifiers::ALT && *key == Key::Character(".".into()) {
                    KeyRoute::Suggest
                } else {
                    KeyRoute::Pass
                }
            }));
            let host = ev.key_message(Key::Character("p".into()), Modifiers::ALT);
            assert_eq!(spell_events(&host), [SpellEvent::Close]);
            assert_eq!(host.len(), 1);
            // The suggest key closes the menu and opens it again.
            let again = ev.key_message(Key::Character(".".into()), Modifiers::ALT);
            assert_eq!(
                spell_events(&again),
                [SpellEvent::Close, SpellEvent::OpenAtCaret]
            );
            for message in again {
                handle(&mut ev, message);
            }
            assert!(ev.spell_menu().is_some());
            // With no menu open, it only opens.
            ev.close_spell_menu();
            let open = ev.key_message(Key::Character(".".into()), Modifiers::ALT);
            assert_eq!(spell_events(&open), [SpellEvent::OpenAtCaret]);
        }
    }

    #[test]
    fn closing_without_accepting_leaves_the_text_caret_and_selection() {
        let (mut ev, _) = checked("teh cat teh");
        let selection = select((0, 9), (0, 1));
        ev.move_cursor(selection);
        ev.spell(SpellEvent::OpenAtCaret);
        ev.spell(SpellEvent::Move(2));
        assert!(!ev.spell(SpellEvent::Close));
        assert_eq!(ev.text(), "teh cat teh");
        assert_eq!(ev.cursor(), selection);
        assert!(!ev.can_undo());
    }

    #[test]
    fn accepting_replaces_the_word_as_one_undo_step_with_the_caret_after_it() {
        let (mut ev, _) = checked("cat teh dog");
        let selection = select((0, 0), (0, 5));
        ev.move_cursor(selection);
        ev.spell(SpellEvent::OpenAtCaret);
        ev.spell(SpellEvent::Move(1));
        assert!(ev.spell(SpellEvent::Accept(1)));
        assert_eq!(ev.text(), "cat tech dog");
        assert_eq!(ev.cursor(), at(0, 8), "after the word, nothing selected");
        assert!(ev.spell_menu().is_none());
        assert!(ev.undo());
        assert_eq!(ev.text(), "cat teh dog");
        assert!(!ev.can_undo(), "one step");
        assert!(ev.redo());
        assert_eq!(ev.text(), "cat tech dog");
        // With the menu shut, accepting does nothing.
        assert!(!ev.spell(SpellEvent::Accept(0)));
        assert_eq!(ev.text(), "cat tech dog");
    }

    #[test]
    fn accepting_closes_a_typing_group_and_is_undone_alone() {
        let (mut ev, _) = checked("teh cat");
        ev.move_cursor(at(0, 7));
        assert!(ev.perform(Action::Edit(Edit::Insert('s'))));
        ev.move_cursor(at(0, 1));
        ev.spell(SpellEvent::OpenAtCaret);
        assert!(ev.spell(SpellEvent::Accept(0)));
        assert_eq!(ev.text(), "the cats");
        assert!(ev.undo());
        assert_eq!(ev.text(), "teh cats");
        assert!(ev.undo());
        assert_eq!(ev.text(), "teh cat");
    }

    /// A text, the caret's line and column, the row accepted, the text
    /// after, and the caret's column after.
    type Replacement = (&'static str, (usize, usize), usize, &'static str, usize);

    #[test]
    fn a_replacement_changes_only_the_words_bytes_under_any_line_endings() {
        let cases: [Replacement; 9] = [
            // The start of a line and of the text.
            ("teh cat\ndog\n", (0, 0), 0, "the cat\ndog\n", 3),
            ("teh cat\r\ndog\r\n", (0, 2), 1, "tech cat\r\ndog\r\n", 4),
            // The end of a line, under each ending.
            ("cat teh\ndog", (0, 7), 1, "cat tech\ndog", 8),
            ("cat teh\r\ndog", (0, 7), 1, "cat tech\r\ndog", 8),
            // Mixed endings: the line between them, and the end of the text.
            ("one\r\ncat teh\nend", (1, 5), 0, "one\r\ncat the\nend", 7),
            (
                "one\r\ncat teh\nlast teh",
                (2, 8),
                1,
                "one\r\ncat teh\nlast tech",
                9,
            ),
            ("one\ncat\r\nteh", (2, 3), 0, "one\ncat\r\nthe", 3),
            // Multi-byte neighbours.
            ("«teh» —teh—\r\n", (0, 12), 1, "«teh» —tech—\r\n", 15),
            ("«teh» —teh—\r\n", (0, 2), 0, "«the» —teh—\r\n", 5),
        ];
        for (text, (line, column), row, after, caret) in cases {
            let (mut ev, _) = checked(text);
            ev.move_cursor(at(line, column));
            ev.spell(SpellEvent::OpenAtCaret);
            assert!(ev.spell_menu().is_some(), "{text:?}");
            assert!(ev.spell(SpellEvent::Accept(row)), "{text:?}");
            assert_eq!(ev.text(), after, "{text:?}");
            assert_eq!(ev.cursor(), at(line, caret), "{text:?}");
            assert!(ev.undo());
            assert_eq!(ev.text(), text, "{text:?}");
        }
    }

    #[test]
    fn a_word_with_a_curly_apostrophe_is_corrected_with_curly_ones() {
        let proofreader = Arc::new(
            fixtures::Rejects::new(&["dosen’t"])
                .suggesting(&[("dosen’t", &["doesn't", "dozen", "doesn’t"])]),
        );
        let text = "It dosen’t work\r\nok";
        let mut ev = EditorView::with_backend(BackendKind::Owned, text);
        ev.set_proofreader(Some(proofreader));
        ev.move_cursor(at(0, 4));
        ev.spell(SpellEvent::OpenAtCaret);
        assert_eq!(
            menu_of(&ev),
            Some((
                0,
                3..12,
                "dosen’t".into(),
                vec!["doesn’t".into(), "dozen".into()],
                Some(0)
            ))
        );
        assert!(ev.spell(SpellEvent::Accept(0)));
        assert_eq!(ev.text(), "It doesn’t work\r\nok");
        assert_eq!(ev.cursor(), at(0, 3 + "doesn’t".len()));
    }

    #[test]
    fn accepting_changes_nothing_when_the_word_is_no_longer_at_its_mark() {
        let position = |line, column| Position { line, column };
        // Changed behind the view's back, so the menu stays open.
        let (mut ev, _) = opened();
        ev.content
            .replace_range(position(0, 0), position(0, 3), "ten");
        assert!(ev.spell_menu().is_some());
        assert!(!ev.spell(SpellEvent::Accept(0)));
        assert_eq!(ev.text(), "ten cat");
        assert!(ev.spell_menu().is_none());
        assert!(!ev.can_undo());
        // Moved along its line.
        let (mut ev, _) = opened();
        ev.content
            .replace_range(position(0, 0), position(0, 0), "x");
        assert!(!ev.spell(SpellEvent::Accept(0)));
        assert_eq!(ev.text(), "xteh cat");
        // Its line gone.
        let (mut ev, _) = opened();
        ev.content.replace_text("");
        assert!(!ev.spell(SpellEvent::Accept(0)));
        assert_eq!(ev.text(), "");

        // Through the view, any change closes the menu first.
        let undone = |ev: &mut EditorView| {
            ev.undo();
        };
        let steps: [(&str, Step); 3] = [
            ("replaced text", |ev| ev.replace_text("teh teh", true)),
            ("undo", undone),
            ("typing", |ev| {
                ev.perform(Action::Edit(Edit::Insert('x')));
            }),
        ];
        for (name, step) in steps {
            let (mut ev, _) = checked("cat");
            ev.move_cursor(at(0, 3));
            ev.perform(Action::Edit(Edit::Insert(' ')));
            ev.perform(Action::Edit(Edit::Insert('t')));
            ev.perform(Action::Edit(Edit::Insert('e')));
            ev.perform(Action::Edit(Edit::Insert('h')));
            ev.spell(SpellEvent::OpenAtCaret);
            assert!(ev.spell_menu().is_some(), "{name}");
            step(&mut ev);
            let text = ev.text();
            let cursor = ev.cursor();
            assert!(!ev.spell(SpellEvent::Accept(0)), "{name}");
            assert_eq!((ev.text(), ev.cursor()), (text, cursor), "{name}");
        }
    }

    #[test]
    fn a_row_without_a_suggestion_closes_the_menu_and_changes_nothing() {
        let (mut ev, _) = opened();
        assert!(!ev.spell(SpellEvent::Accept(5)));
        assert!(ev.spell_menu().is_none());
        assert_eq!((ev.text(), ev.cursor()), ("teh cat".into(), at(0, 1)));
        let (mut ev, _) = checked("zzz");
        ev.spell(SpellEvent::OpenAtCaret);
        assert!(!ev.spell(SpellEvent::Accept(0)));
        assert!(ev.spell_menu().is_none());
        assert_eq!(ev.text(), "zzz");
        assert!(!ev.can_undo());
    }

    /// Something done to an editor.
    type Step = fn(&mut EditorView);

    #[test]
    fn whatever_moves_or_changes_the_text_closes_the_menu() {
        let steps: [(&str, Step); 14] = [
            ("typing", |ev| {
                ev.perform(Action::Edit(Edit::Insert('x')));
            }),
            ("a motion", |ev| {
                ev.perform(Action::Move(Motion::Right));
            }),
            ("a scroll", |ev| {
                ev.perform(Action::Scroll { lines: 1 });
            }),
            ("an intent", |ev| {
                ev.intent(Intent::Home { extend: false });
            }),
            ("an applied edit", |ev| {
                ev.apply(&[], at(0, 0));
            }),
            ("undo", |ev| {
                ev.undo();
            }),
            ("redo", |ev| {
                ev.redo();
            }),
            ("a moved cursor", |ev| ev.move_cursor(at(0, 2))),
            ("a scrolled line", |ev| ev.scroll_to_line(0, Placement::Top)),
            ("a restored viewport", |ev| {
                let viewport = ev.viewport();
                ev.restore_viewport(viewport);
            }),
            ("find", |ev| {
                ev.set_find_query("cat");
                ev.find_next();
            }),
            ("replaced text", |ev| ev.replace_text("teh cat", true)),
            ("a new note", |ev| ev.replace_text("teh dog", false)),
            ("a new engine", |ev| {
                ev.set_backend(BackendKind::Iced, "teh cat");
            }),
        ];
        for (name, step) in steps {
            let (mut ev, _) = opened();
            step(&mut ev);
            assert!(ev.spell_menu().is_none(), "{name}");
        }
        // Setting the query alone, with the find bar taking the keyboard,
        // is the host's to close for.
        let (mut ev, _) = opened();
        ev.set_find_query("cat");
        ev.break_group();
        assert!(ev.spell_menu().is_some());
    }

    #[test]
    fn a_changed_setting_closes_the_menu_and_an_unchanged_one_does_not() {
        let unchanged: [fn(&mut EditorView); 6] = [
            |ev| ev.set_font_size(14.0),
            |ev| ev.set_line_height(widget::DEFAULT_LINE_HEIGHT),
            |ev| ev.set_font(Font::MONOSPACE),
            |ev| ev.set_line_numbers(true),
            |ev| ev.set_markdown_assist(true),
            |ev| {
                let same = ev.proofreader.clone();
                ev.set_proofreader(same);
            },
        ];
        for step in unchanged {
            let (mut ev, _) = opened();
            step(&mut ev);
            assert!(ev.spell_menu().is_some());
        }
        let changed: [fn(&mut EditorView); 7] = [
            |ev| ev.set_font_size(16.0),
            |ev| ev.set_line_height(2.0),
            |ev| ev.set_font(Font::with_name("JetBrains Mono")),
            |ev| ev.set_line_numbers(false),
            |ev| ev.set_markdown_assist(false),
            |ev| ev.set_proofreader(None),
            |ev| ev.set_proofreader(Some(Arc::new(fixtures::Rejects::new(&["teh"])))),
        ];
        for (i, step) in changed.into_iter().enumerate() {
            let (mut ev, _) = opened();
            step(&mut ev);
            assert!(ev.spell_menu().is_none(), "{i}");
        }
    }

    #[test]
    fn spell_frame_starts_empty() {
        let mut ev = EditorView::with_backend(BackendKind::Owned, "teh");
        ev.set_proofreader(Some(Arc::new(fixtures::Rejects::new(&["teh"]))));
        assert_eq!(ev.spell_frame(), SpellFrame::default());
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

    #[test]
    fn a_text_size_change_keeps_the_line_height_ratio() {
        let mut ev = EditorView::new("x");
        ev.set_line_height(2.0);
        assert_eq!(ev.metrics, widget::Metrics::new(14.0, 2.0));
        ev.set_font_size(20.0);
        assert_eq!(ev.metrics.line, 40.0);
        ev.set_line_height(widget::DEFAULT_LINE_HEIGHT);
        assert_eq!(ev.metrics, widget::Metrics::for_size(20.0));
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

    #[test]
    fn undo_after_scrolling_away_shows_the_caret() {
        let size = iced::Size::new(300.0, 120.0);
        for kind in BackendKind::ALL {
            for away in [-60, 60] {
                let mut ev = EditorView::with_backend(kind, &numbered_lines(200));
                ev.content.lay_out(size);
                ev.move_cursor(at(100, 3));
                ev.content.lay_out(size);
                assert!(ev.perform(Action::Edit(Edit::Insert('Q'))));
                ev.content.lay_out(size);
                ev.perform(Action::Scroll { lines: away });
                ev.content.lay_out(size);

                assert!(ev.undo());
                ev.content.lay_out(size);
                assert_eq!(ev.cursor(), at(100, 3), "{kind:?} {away}");
                let Some(text_editor::Selection::Caret(caret)) = ev.content.selection_geometry()
                else {
                    panic!("{kind:?} {away}: a caret");
                };
                assert!(
                    (0.0..size.height).contains(&caret.y),
                    "{kind:?} {away}: {caret:?}"
                );
            }
        }
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
        fn front_matter_end_follows_every_kind_of_change() {
            let mut ev = editor("---\ntitle: x\n---\nbody");
            assert_eq!(ev.front_matter_end, Some(2));
            // Typing into the closing delimiter unbalances the front matter.
            let caret = |line, column| Cursor {
                position: Position { line, column },
                selection: None,
            };
            ev.move_cursor(caret(2, 3));
            assert!(ev.perform(Action::Edit(Edit::Insert('x'))));
            assert_eq!(ev.front_matter_end, None);
            assert!(ev.undo());
            assert_eq!(ev.front_matter_end, Some(2));
            assert!(ev.redo());
            assert_eq!(ev.front_matter_end, None);
            let close = Replace {
                start: Position { line: 2, column: 0 },
                end: Position { line: 2, column: 4 },
                text: "\n...".into(),
            };
            assert!(ev.apply(&[close], caret(0, 0)));
            assert_eq!(ev.front_matter_end, Some(3));
            ev.replace_text("# no front matter", true);
            assert_eq!(ev.front_matter_end, None);
            ev.replace_text("---\n# comment\n---", false);
            assert_eq!(ev.front_matter_end, Some(2));
            let other = crate::BackendKind::ALL
                .into_iter()
                .find(|&kind| kind != KIND)
                .unwrap();
            assert!(ev.set_backend(other, "---\na: b\n\n...\n"));
            assert_eq!(ev.front_matter_end, Some(3));
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
            let _element = ev.view(fixtures::STYLE, fixtures::DARK, Measure::default());
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
        fn inserted_breaks_never_fuse_with_a_neighbouring_lone_cr_or_lf() {
            let reloaded = |ev: &EditorView| EditorView::new(&ev.text()).line_count();
            // A break at column 0 below a lone CR (or LF) line, and one that
            // ends before an empty rest of line whose ending starts with LF.
            for (text, start, end) in [
                ("a\rb\nc", pos(1, 0), pos(1, 0)),
                ("a\nb\rc", pos(1, 0), pos(1, 0)),
                ("a\rb\r\nc", pos(1, 0), pos(1, 0)),
                ("a\nb\n\rc", pos(1, 0), pos(1, 0)),
                ("a\rb\nc", pos(1, 0), pos(1, 1)),
                ("a\rb\rc\nd", pos(1, 0), pos(2, 1)),
                ("a\r\rb\nc", pos(1, 0), pos(2, 1)),
            ] {
                let cursor = Cursor {
                    position: end,
                    selection: (start != end).then_some(start),
                };
                for edit in [
                    Action::Edit(Edit::Enter),
                    Action::Edit(Edit::Paste(Arc::new("\n".into()))),
                    Action::Edit(Edit::Paste(Arc::new("\r".into()))),
                    Action::Edit(Edit::Paste(Arc::new("\nx\n".into()))),
                ] {
                    let mut ev = editor(text);
                    ev.move_cursor(cursor);
                    assert!(ev.perform(edit.clone()));
                    assert_eq!(ev.line_count(), reloaded(&ev), "{text:?} {edit:?}");
                }
                for inserted in ["\n", "\r\n", "\n- ", "x\n"] {
                    let mut ev = editor(text);
                    let plan = [Replace {
                        start,
                        end,
                        text: inserted.into(),
                    }];
                    assert!(ev.apply(&plan, super::at(0, 0)));
                    assert_eq!(ev.line_count(), reloaded(&ev), "{text:?} {inserted:?}");
                }
            }
        }

        #[test]
        fn deleting_a_lines_text_never_fuses_its_ending_with_the_line_above() {
            let reloaded = |ev: &EditorView| EditorView::new(&ev.text()).line_count();
            for (text, start, end) in [
                ("a\rb\nc", pos(1, 0), pos(1, 1)),
                ("a\nb\rc", pos(1, 0), pos(1, 1)),
                ("a\nb\r\nc", pos(1, 0), pos(1, 1)),
                ("a\rb\n\nc", pos(1, 0), pos(1, 1)),
                ("a\rx\rb\nc", pos(1, 0), pos(2, 1)),
            ] {
                for delete in [Edit::Backspace, Edit::Delete] {
                    let mut ev = editor(text);
                    ev.move_cursor(Cursor {
                        position: end,
                        selection: Some(start),
                    });
                    assert!(ev.perform(Action::Edit(delete.clone())));
                    assert_eq!(ev.line_count(), reloaded(&ev), "{text:?} {delete:?}");
                    assert_eq!(ev.cursor(), super::at(start.line, 0), "{text:?}");
                    assert!(ev.undo());
                    assert_eq!(ev.text(), text);
                }
                let mut ev = editor(text);
                let plan = [Replace {
                    start,
                    end,
                    text: String::new(),
                }];
                assert!(ev.apply(&plan, super::at(start.line, 0)));
                assert_eq!(ev.line_count(), reloaded(&ev), "{text:?} apply");
            }
            // A plain Backspace that empties a line.
            let mut ev = editor("a\rb\n\nc");
            ev.move_cursor(super::at(1, 1));
            assert!(ev.perform(Action::Edit(Edit::Backspace)));
            assert_eq!(ev.line_count(), reloaded(&ev));
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
        fn restore_viewport_returns_the_cursor_selection_and_scroll() {
            let size = iced::Size::new(300.0, 240.0);
            let mut ev = editor(&super::numbered_lines(200));
            ev.content.lay_out(size);
            ev.scroll_to_line(60, Placement::Top);
            ev.content.lay_out(size);
            let selected = Cursor {
                position: Position {
                    line: 63,
                    column: 2,
                },
                selection: Some(Position {
                    line: 62,
                    column: 1,
                }),
            };
            ev.move_cursor(selected);
            ev.content.lay_out(size);
            let origin = ev.viewport();
            assert_eq!(origin.cursor(), selected);
            assert_eq!(ev.first_visible_line(), 60);

            for line in [150, 5] {
                ev.scroll_to_line(line, Placement::NearTop);
                ev.move_cursor(super::at(line, 0));
                ev.content.lay_out(size);
                assert_ne!(ev.first_visible_line(), 60);
                ev.restore_viewport(origin);
                ev.content.lay_out(size);
                assert_eq!(ev.cursor(), selected);
                assert_eq!(ev.selected_text().as_deref(), Some("ine 62\nli"));
                let top = match (KIND, line) {
                    // iced settles a scroll and reveals the cursor in one
                    // update, which puts a cursor above the view on top.
                    (BackendKind::Iced, 150) => 63,
                    _ => 60,
                };
                assert_eq!(ev.first_visible_line(), top, "back from line {line}");
            }
            // A caret origin drops the selection made since.
            ev.move_cursor(super::at(70, 0));
            let caret = ev.viewport();
            ev.move_cursor(selected);
            ev.restore_viewport(caret);
            assert_eq!(ev.cursor(), super::at(70, 0));
            assert_eq!(ev.selected_text(), None);
        }

        #[test]
        fn a_viewport_away_from_the_cursor_is_taken_and_restored_in_place() {
            let size = iced::Size::new(300.0, 240.0);
            let selected = Cursor {
                position: Position { line: 5, column: 4 },
                selection: Some(Position { line: 5, column: 0 }),
            };
            for origin_cursor in [super::at(2, 0), selected] {
                let mut ev = editor(&super::numbered_lines(300));
                ev.content.lay_out(size);
                ev.move_cursor(origin_cursor);
                ev.content.lay_out(size);
                ev.perform(Action::Scroll { lines: 100 });
                ev.content.lay_out(size);
                let top = ev.first_visible_line();
                assert!(top > 50, "scrolled to {top}");

                let origin = ev.viewport();
                ev.content.lay_out(size);
                assert_eq!(origin.cursor(), origin_cursor);
                assert_eq!(ev.first_visible_line(), top, "taking {origin_cursor:?}");
                ev.restore_viewport(origin);
                ev.content.lay_out(size);
                assert_eq!(ev.first_visible_line(), top, "restoring {origin_cursor:?}");
                assert_eq!(ev.cursor(), origin_cursor);
            }
        }

        #[test]
        fn a_word_selection_viewport_is_its_bounds() {
            let size = iced::Size::new(300.0, 240.0);
            let mut ev = editor(&super::numbered_lines(300));
            ev.content.lay_out(size);
            ev.move_cursor(super::at(5, 2));
            ev.content.perform(Action::SelectWord);
            ev.content.lay_out(size);
            ev.perform(Action::Scroll { lines: 100 });
            ev.content.lay_out(size);
            let top = ev.first_visible_line();

            let origin = ev.viewport();
            ev.content.lay_out(size);
            let word = Cursor {
                position: Position { line: 5, column: 4 },
                selection: Some(Position { line: 5, column: 0 }),
            };
            assert_eq!(origin.cursor(), word);
            // Normalizing the selection moves the cursor, which iced reveals.
            if KIND == BackendKind::Owned {
                assert_eq!(ev.first_visible_line(), top);
            }
            ev.restore_viewport(origin);
            ev.content.lay_out(size);
            assert_eq!(ev.cursor(), word);
            assert_eq!(ev.selected_text().as_deref(), Some("line"));
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
