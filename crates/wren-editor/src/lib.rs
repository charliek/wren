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
mod hit;
mod keys;
mod line_edit;
mod plain;
mod popover;
mod proof;
mod widget;

#[cfg(test)]
mod fixtures;

use std::borrow::Cow;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use iced::advanced::graphics::text::cosmic_text::Scroll;
use iced::widget::text_editor::{self, Action, Cursor, Edit, Motion, Position};
use iced::{Color, Element, Font};

pub use backend::{BackendKind, Placement};
pub use edit::Replace;
#[cfg(any(test, feature = "test-support"))]
pub use highlighter::HighlightedLine;
#[cfg(feature = "test-support")]
pub use highlighter::{highlight_lines, Highlight, Highlights, LineDecor, Scale};
pub use hit::Hit;
pub use keys::{KeyPolicy, KeyRoute};
pub use popover::{
    is_plain_escape, Accelerator, Anchor, Choice, Clicked, MenuProvider, MenuSource, Mode, Popover,
    PopoverEvent, Provider, Requested, Row, RowKind, Seen, Stroke, SuggestSource, Suggestion,
    TaskBox,
};
#[cfg(any(test, feature = "test-support"))]
pub use proof::{DrawnMark, SpellFrame};
pub use proof::{MenuKey, MenuPress, Proofreader, SpellEvent, SpellMenu};
pub use widget::Measure;
use wren_markdown::span::LineSpans;
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
    /// The suggestions menu, and every other popover.
    pub menu: MenuStyle,
}

/// Colours and type of a popover's rows. The editor knows no UI font of
/// its own, so the host supplies it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuStyle {
    pub background: Color,
    pub border: Color,
    /// The fill behind the selected row.
    pub selected: Color,
    pub text: Color,
    /// The inactive "No suggestions" row, a row that cannot be chosen,
    /// and the key named beside a row.
    pub muted: Color,
    /// A checked row's label ([`Row::checked`]).
    pub accent: Color,
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
    /// A task's due date, with the host's tasks on.
    pub date: Color,
    /// An unfinished task's due date that is past, or that is no real day.
    pub overdue: Color,
    /// A task's `@owner`.
    pub owner: Color,
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
    /// The open popover's events, and the requests that open the
    /// suggestions menu, its first user, which named them. See
    /// [`EditorView::spell`].
    Spell(SpellEvent),
    /// A copy, cut, or paste from the keyboard. See [`EditorView::clip`].
    Clip(Clip),
    /// A key the host bound to a command of its own that acts on the text
    /// ([`KeyRoute::Command`]), with the host's number for it. The editor
    /// does nothing with it; the host runs the command, in the key's order
    /// with the rest of the batch.
    Command(u32),
    /// A click on a task's box; see [`BoxClick`]. The editor does nothing
    /// with it; the host changes the task.
    BoxClick(BoxClick),
}

/// A primary click on a task's box, `[` to `]`, with the host's tasks on
/// ([`EditorView::set_tasks`]), on the advanced engine: a single click with
/// no modifier held, pressed on the box and let go on the same box, the
/// pointer never more than [`BoxClick::DRAG`] from where it was pressed,
/// inside the editor or out. It leaves the caret and the selection where
/// they were. The widget publishes it once the time in which a second
/// press would make a double click (300 ms from the press, iced's) has
/// passed, or at once before any other input; a second press that makes
/// a double click on the same box takes its place, and selects as a
/// double click does from where it was pressed.
///
/// A press that becomes a drag selects from where it was pressed, as any
/// other; one let go off the box is a click in the text where it was
/// pressed. A click with Shift, Ctrl, Alt or the logo key held is the
/// text's, and a key, another button, a modifier or the editor losing the
/// keyboard before it is let go makes it no click at all.
///
/// The host asks [`EditorView::box_clicked`] which task it was for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxClick {
    line: usize,
    /// [`EditorView::revision`] of the text the box was pressed on.
    revision: u64,
    /// Where the box was pressed, relative to the text origin, for a host
    /// that takes the click as one in the text
    /// ([`EditorView::click_in_text`]).
    at: iced::Point,
}

impl BoxClick {
    /// How far the pointer may move, in pixels, between pressing the box
    /// and letting go, for the press to stay a click on it.
    pub const DRAG: f32 = 4.0;

    pub(crate) fn new(line: usize, revision: u64, at: iced::Point) -> Self {
        Self { line, revision, at }
    }
}

/// What the clipboard keys did. With no selection, ⌘C and ⌘X take the
/// caret's whole line, and ⌘V puts such a line back above the caret's, as
/// VS Code does. The widget has already written the text to the clipboard.
#[derive(Debug, Clone, PartialEq)]
pub enum Clip {
    /// The caret's line went to the clipboard as this text.
    CopyLine(Arc<String>),
    /// As [`Clip::CopyLine`], and the line goes from the text.
    CutLine(Arc<String>),
    /// A selection went to the clipboard, so the last line copied or cut is
    /// no longer what a paste of the same text holds.
    Selection,
    /// A keyboard paste of this text from the clipboard.
    Paste(Arc<String>),
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
    /// The host's tasks, when they are on; see [`EditorView::set_tasks`].
    tasks: Option<wren_markdown::TaskOptions>,
    markdown_assist: bool,
    key_policy: Option<KeyPolicy>,
    proofreader: Option<Arc<dyn Proofreader>>,
    /// The open suggestions menu.
    menu: Option<SpellMenu>,
    /// The host's open popover; see [`EditorView::open_popover`]. At most
    /// one of it and `menu` is open.
    popover: Option<Popover>,
    /// The row the last popover event chose from the host's popover.
    choice: Option<Chosen>,
    suggester: Option<Arc<dyn SuggestSource>>,
    menus: Option<Arc<dyn MenuSource>>,
    /// Who a right-click asks, in order; see
    /// [`EditorView::set_menu_providers`].
    providers: Vec<Provider>,
    /// Which list `providers` is: it changes when the host names another,
    /// so that a click or a row waiting on a provider of the old list is
    /// not handed to whoever has its place in the new one.
    provider_list: u64,
    /// The right-click the last popover event handed to the host, whose
    /// provider took the keyboard; see [`EditorView::take_keyboard`].
    keyboard: Option<Handed>,
    /// How many popovers the editor has opened: the
    /// [`Popover::instance`] of the last.
    opened: u64,
    /// The line Escape hid suggestions for, until the caret leaves it.
    suppressed: Option<usize>,
    /// The next edit asks the source of suggestions nothing; see
    /// [`PopoverEvent::Hold`].
    hold: bool,
    /// How many presses of Escape a popover took that the host has yet to
    /// hear of from its own key handling; see
    /// [`EditorView::take_claimed_escape`].
    escapes: u32,
    /// Where the text starts in the window, as the widget last placed it.
    origin: std::cell::Cell<Option<iced::Point>>,
    /// The left press being held was resolved here as no single click: it
    /// only closed the menu, so its drag selects nothing either (rule
    /// 4.4.5), or it was a double or triple click, which drags nothing.
    drop_drags: bool,
    /// What the widget makes of the left presses it leaves here, which it
    /// counts from in the next batch.
    resolved_press: popover::ResolvedPress,
    /// What the last left press resolved here was taken as, while the
    /// widget's guess at it was wrong: the widget counts the presses after
    /// it in the same batch from that guess, so they are counted again.
    press_chain: Option<Option<iced::advanced::mouse::click::Kind>>,
    /// The text of the last whole-line copy or cut. A paste of exactly this
    /// text goes in as a line, the way VS Code tells a line it copied from
    /// text that only looks like one.
    line_clip: Option<Arc<String>>,
    /// The marks the widget last drew.
    #[cfg(any(test, feature = "test-support"))]
    spell_frame: std::cell::RefCell<SpellFrame>,
    /// See [`EditorView::revision`].
    revision: u64,
}

/// A row chosen from the host's popover, until the host takes it.
struct Chosen {
    /// The revision of the text it was chosen over.
    revision: u64,
    choice: Choice,
    /// The provider whose menu it was chosen from
    /// ([`Popover::provider`]).
    provider: Option<usize>,
    /// What the row does ([`Row::kind`]).
    kind: RowKind,
}

/// A right-click whose provider took the keyboard, until the host takes it.
struct Handed {
    /// The revision of the text it was made over.
    revision: u64,
    provider: usize,
    clicked: Clicked,
}

/// The last revision any editor took; see [`EditorView::revision`].
static REVISIONS: AtomicU64 = AtomicU64::new(0);

fn next_revision() -> u64 {
    REVISIONS.fetch_add(1, Ordering::Relaxed) + 1
}

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
            tasks: None,
            markdown_assist: true,
            key_policy: None,
            proofreader: None,
            menu: None,
            popover: None,
            choice: None,
            suggester: None,
            menus: None,
            providers: popover::SPELLING_ALONE.to_vec(),
            provider_list: 0,
            keyboard: None,
            opened: 0,
            suppressed: None,
            hold: false,
            escapes: 0,
            origin: std::cell::Cell::new(None),
            drop_drags: false,
            resolved_press: popover::ResolvedPress::default(),
            press_chain: None,
            line_clip: None,
            #[cfg(any(test, feature = "test-support"))]
            spell_frame: Default::default(),
            revision: next_revision(),
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
            self.close_open();
        }
        self.markdown_assist = on;
    }

    pub fn markdown_assist(&self) -> bool {
        self.markdown_assist
    }

    /// Tell the editor whether the host's tasks are on, and today's date
    /// when they are (off by default). It is the one value everything in
    /// the editor that reads a task line takes: the Markdown keys' plans,
    /// the highlighter, and so the prose spell check reads. On, a box of
    /// any status is a task; off, only `[ ]`, `[x]` and `[X]` are.
    pub fn set_tasks(&mut self, tasks: Option<wren_markdown::TaskOptions>) {
        if tasks != self.tasks {
            self.close_open();
        }
        if tasks.is_some() != self.tasks.is_some() {
            // The boxes read change, and with them what a line is.
            self.revision = next_revision();
        }
        self.tasks = tasks;
    }

    /// A number that changes whenever the text does, or the way its lines
    /// are read ([`set_tasks`](Self::set_tasks) turning tasks on or off),
    /// and that no other editor in the process has had: what a host keys
    /// what it read from the text by, such as a line's
    /// [`spans_at`](Self::spans_at).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn tasks(&self) -> Option<wren_markdown::TaskOptions> {
        self.tasks
    }

    /// How the planner indents and renumbers, and the tasks it reads.
    fn plan_options(&self) -> wren_markdown::Options {
        wren_markdown::Options {
            indent_width: 4,
            renumber: true,
            tasks: self.tasks,
        }
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
            self.close_open();
        }
        self.metrics = metrics;
    }

    /// The editor font; bold and italic spans use its variants. Defaults to
    /// [`Font::MONOSPACE`].
    pub fn set_font(&mut self, font: Font) {
        if font != self.font {
            self.close_open();
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
            self.close_open();
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
            self.highlighter_settings(palette),
            style,
            self.line_numbers,
            self.metrics,
            self.markdown_assist,
        )
        .key_policy(self.key_policy.as_ref())
        .proofreader(self.proofreader.as_deref())
        .menu_source(self.menu_source())
        .providers(self.providers(), self.edition())
        .suggest_source(self.suggest_source(), self.suggest_suppressed())
        .origin(&self.origin)
        .resolved_press(self.resolved_press)
        .measure(measure);
        let editor = match &self.menu {
            Some(menu) => editor.spell_menu(Some(menu)),
            None => editor.popover(self.popover.as_ref()),
        };
        #[cfg(any(test, feature = "test-support"))]
        let editor = editor.spell_frame(&self.spell_frame);
        editor.into()
    }

    /// What the widget's highlighter is set up with, for `palette`.
    fn highlighter_settings(&self, palette: Palette) -> HighlighterSettings {
        HighlighterSettings {
            palette,
            font: self.font,
            front_matter_end: self.front_matter_end,
            tasks: self.tasks,
        }
    }

    /// Every line as the widget's highlighter, set up as this editor sets
    /// it up, colours and decorates it with `palette`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn highlighted(&self, palette: Palette) -> Vec<HighlightedLine> {
        let settings = self.highlighter_settings(palette);
        self.content
            .with_lines(|lines| highlighter::highlight_all(&lines, settings))
    }

    /// Line `line` as the widget's highlighter, set up as this editor sets
    /// it up, colours it with `palette`, highlighting only the lines down
    /// to it: its text, its spans and the typed spans they come from. None
    /// past the last line.
    #[cfg(feature = "test-support")]
    pub fn highlighted_line(
        &self,
        line: usize,
        palette: Palette,
    ) -> Option<(String, Highlights, LineSpans)> {
        let settings = self.highlighter_settings(palette);
        self.content.with_lines(|lines| {
            let text = lines.get(line)?.text.to_string();
            let (spans, read) = highlighter::highlight_through(&lines, line, settings)?;
            Some((text, spans, read))
        })
    }

    /// The misspellings the editor finds on `line` with its proofreader,
    /// the ones the suggest key reads.
    #[cfg(feature = "test-support")]
    pub fn marks(&self, line: usize) -> Vec<Range<usize>> {
        self.marks_at(line)
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    /// Apply `action` to the content.
    ///
    /// Returns `true` only when the text changes. Non-edit actions break typing
    /// coalescing without discarding history; no-op edits also preserve redo.
    pub fn perform(&mut self, action: Action) -> bool {
        let held = std::mem::take(&mut self.hold);
        let typed = !held && matches!(action, Action::Edit(Edit::Insert(_) | Edit::Backspace));
        let changed = self.perform_action(action);
        if typed {
            self.suggest();
        }
        changed
    }

    /// [`Self::perform`], without the suggestions that follow typing.
    fn perform_action(&mut self, action: Action) -> bool {
        self.close_open();
        if matches!(action, Action::Drag(_)) {
            if self.drop_drags {
                return false;
            }
        } else {
            self.drop_drags = false;
        }
        if matches!(
            action,
            Action::Click(_) | Action::SelectWord | Action::SelectLine
        ) {
            self.press_chain = None;
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
        self.close_open();
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
        let held = std::mem::take(&mut self.hold);
        let changed = self.perform_intent(intent);
        if !held && matches!(intent, Intent::Backspace) {
            self.suggest();
        }
        changed
    }

    /// [`Self::intent`], without the suggestions that follow a deletion.
    fn perform_intent(&mut self, intent: Intent) -> bool {
        self.close_open();
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
            plain.unwrap_or_else(|| {
                wren_markdown::plan(&lines, selection, intent, &self.plan_options())
            })
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

    /// Carry out a clipboard key; see [`Clip`]. A line cut is one undoable
    /// change, and the caret keeps its column on the line that takes the cut
    /// line's place. A paste of the last line copied or cut, with no
    /// selection, goes in above the caret's line and the caret stays where it
    /// was in its own; any other paste is an ordinary one.
    ///
    /// Returns whether the text changed.
    pub fn clip(&mut self, clip: Clip) -> bool {
        self.close_open();
        match clip {
            Clip::CopyLine(text) => {
                self.line_clip = Some(text);
                false
            }
            Clip::Selection => {
                self.line_clip = None;
                false
            }
            Clip::CutLine(text) => {
                self.line_clip = Some(text);
                let caret = self.content.cursor().position;
                let (replace, landing) = self
                    .content
                    .with_lines(|lines| edit::cut_line(lines, caret.line, caret.column));
                let landing = Cursor {
                    position: landing,
                    selection: None,
                };
                self.apply(&[replace], landing)
            }
            Clip::Paste(text) => {
                let whole_line = !self.has_selection() && self.line_clip.as_ref() == Some(&text);
                if !whole_line {
                    return self.perform(Action::Edit(Edit::Paste(text)));
                }
                let caret = self.content.cursor().position;
                let breaks = wren_markdown::line_ranges(&text).count() - 1;
                let at = Position {
                    line: caret.line,
                    column: 0,
                };
                let landing = Cursor {
                    position: Position {
                        line: caret.line + breaks,
                        column: caret.column,
                    },
                    selection: None,
                };
                self.apply(&[Replace::insert(at, text.as_str())], landing)
            }
        }
    }

    /// The whole-line clip of the caret's line, for a copy or cut with
    /// nothing selected.
    fn caret_line(&self) -> String {
        let line = self.content.cursor().position.line;
        self.content
            .with_lines(|lines| edit::line_clip(lines, line))
    }

    /// Undo the last edit unit.
    ///
    /// Returns `true` if something was restored: the text, the cursor and
    /// selection, and on the owned engine the scroll position.
    pub fn undo(&mut self) -> bool {
        self.close_open();
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
        self.close_open();
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
        self.close_open();
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
        self.close_open();
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
        self.close_open();
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
        self.suppressed = None;
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
        self.suppressed = None;
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
        self.close_open();
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
        self.close_open();
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

    /// Carry out a popover event: the suggestions menu's, or the host's
    /// popover's. Returns whether the text changed, as
    /// [`EditorView::perform`] does.
    ///
    /// Opening the suggestions menu finds the misspelled word that contains
    /// or touches the caret ([`SpellEvent::OpenAtCaret`]) or lies under a
    /// point relative to the text origin ([`SpellEvent::OpenAt`]), asks the
    /// proofreader for its suggestions, and scrolls the word's first row
    /// into view if it is not; the caret and the selection stay where they
    /// are. With no such word, on the iced engine, or with no proofreader,
    /// nothing opens. A request for a popover of the host's
    /// ([`SpellEvent::Request`]) closes what is open and then opens what
    /// the host's [`MenuSource`] answers for the caret, so a host that acts
    /// on the request itself may do so before or after it hands the event
    /// here. A right-click one of the host's providers takes
    /// ([`SpellEvent::Context`]) does the same with what that provider
    /// answers for what lies under the point
    /// ([`EditorView::set_menu_providers`]), if the text, the list of
    /// providers, the place and the answer are what the widget saw when
    /// it published the click; otherwise it only closes what is open. A
    /// provider that so takes the keyboard leaves the click for
    /// [`EditorView::take_keyboard`]. A row chosen from a provider's menu
    /// names the menu ([`SpellEvent::AcceptOf`]), and is chosen only if
    /// that menu is the one open. Accepting a suggestion
    /// replaces the word through [`EditorView::apply`], so it is one undo
    /// step; accepting a row of the host's popover closes it and leaves
    /// the row for [`EditorView::take_choice`], or for
    /// [`EditorView::take_provided`] when a provider opened it.
    ///
    /// A popover's key ([`SpellEvent::Key`]) acts on the open popover as
    /// its mode has it, or with none open is handled as the editor handles
    /// that press, which may edit the text too; it and accepting a
    /// suggestion are the events that can return true. Escape so sent
    /// dismisses the open popover and is claimed
    /// ([`EditorView::take_claimed_escape`]); with none open it does
    /// nothing, and stays the host's. A pointer press
    /// ([`SpellEvent::Press`]) only closes an open menu; over suggestions,
    /// or with nothing open, it clicks or opens the suggestions menu as
    /// that press would. A stroke ([`SpellEvent::Stroke`]) is the host's to
    /// take, and does nothing here.
    pub fn spell(&mut self, event: SpellEvent) -> bool {
        self.choice = None;
        self.keyboard = None;
        self.hold = matches!(event, SpellEvent::Hold);
        match event {
            SpellEvent::Key(key) => {
                return match self.popover().map(|popover| key.on(popover.mode())) {
                    Some(Some(event)) => {
                        if key == MenuKey::Escape {
                            self.escapes += 1;
                        }
                        self.spell(event)
                    }
                    // Not that popover's key, so any other key to it.
                    Some(None) => {
                        self.close_open();
                        self.press_without_popover(key)
                    }
                    None => self.press_without_popover(key),
                };
            }
            SpellEvent::Press(press) => return self.press(press),
            SpellEvent::OpenAtCaret => {
                let caret = self.content.cursor().position;
                let mark = proof::mark_at_caret(self.marks_at(caret.line), caret.column);
                self.open_menu(mark.map(|mark| (caret.line, mark)));
            }
            SpellEvent::OpenAt(point) => {
                let target = (self.speller())
                    .and_then(|_| self.place_under(point))
                    .and_then(|place| Some((place.line, place.mark?)));
                self.open_menu(target);
            }
            SpellEvent::Request(id) => {
                self.close_open();
                let answer = (self.menu_source().zip(self.asked_line()))
                    .map(|(source, line)| popover::requested(source, id, &line));
                if let Some(popover::Answer::Menu(menu)) = answer {
                    self.open_answered(menu);
                }
            }
            SpellEvent::Context { provider, at, seen } => {
                self.close_open();
                match self.provided(provider, at, seen) {
                    Some((_, popover::Answer::Menu(menu))) => self.open_answered(menu),
                    Some((clicked, popover::Answer::Keyboard)) => {
                        self.keyboard = Some(Handed {
                            revision: self.revision,
                            provider,
                            clicked,
                        });
                    }
                    Some((_, popover::Answer::Nothing)) | None => {}
                }
            }
            SpellEvent::Move(delta) => {
                if let Some(popover) = self.popover_mut() {
                    popover.step(delta);
                }
            }
            SpellEvent::Hover(row) => {
                if let Some(popover) = self.popover_mut() {
                    popover.hover(row);
                }
            }
            SpellEvent::Accept(row) => return self.accept(row, false),
            SpellEvent::AcceptSelected => match self.popover().and_then(Popover::selected) {
                Some(row) => return self.accept(row, false),
                None => self.close_open(),
            },
            SpellEvent::AcceptOf { menu, row } => {
                // Another popover, or none, is not the menu the row was
                // chosen from: the choice is no one's.
                let Some(open) = self.popover().filter(|open| open.instance() == menu) else {
                    return false;
                };
                match row.or(open.selected()) {
                    Some(row) => return self.accept(row, true),
                    None => self.close_open(),
                }
            }
            SpellEvent::Dismiss => {
                self.dismiss_popover();
            }
            SpellEvent::Close => {
                self.close_open();
            }
            SpellEvent::Stroke(_) | SpellEvent::Hold => {}
        }
        false
    }

    /// Carry out a message the widget would publish for a key press.
    /// Returns whether the text changed.
    fn carry_out(&mut self, message: Message) -> bool {
        match message {
            Message::Action(action) => self.perform(action),
            Message::Intent(intent) => self.intent(intent),
            Message::Spell(event) => self.spell(event),
            Message::Clip(clip) => self.clip(clip),
            Message::Undo => self.undo(),
            Message::Redo => self.redo(),
            // Only Ctrl or Cmd+F routes to Find, never a plain key; a
            // command and a click on a box are the host's.
            Message::Find | Message::Command(_) | Message::BoxClick(_) => false,
        }
    }

    /// Open `menu`, which a source or a provider of the host's answered,
    /// and scroll its anchor's row into view if it is not.
    fn open_answered(&mut self, menu: Popover) {
        self.content.reveal(Position {
            line: menu.line(),
            column: menu.range().start,
        });
        self.popover = Some(self.opening(menu));
    }

    /// `popover` as the next the editor opens, with its number
    /// ([`Popover::instance`]).
    fn opening(&mut self, popover: Popover) -> Popover {
        self.opened += 1;
        popover.opened_as(self.opened)
    }

    /// What the host's provider at `provider` of its list opens for a
    /// right-click at `at`, relative to the text origin, and the place
    /// clicked, if they are what the widget saw when it published the
    /// click (`seen`). `None` when the text or the list of providers has
    /// changed since, in which case no provider is asked; when the place
    /// under the point or the provider's answer for it is another; for a
    /// place in the list that holds no provider of the host's; and off the
    /// text.
    fn provided(
        &self,
        provider: usize,
        at: iced::Point,
        seen: Seen,
    ) -> Option<(Clicked, popover::Answer)> {
        if !seen.over(self.edition()) {
            return None;
        }
        let Some(Provider::Host(asked)) = self.providers().get(provider) else {
            return None;
        };
        let (clicked, task) = self.reading().clicked_in(&self.content, at)?;
        let answer = popover::provided_in(&**asked, provider, &clicked, task.as_ref());
        seen.answered(popover::fingerprint(provider, &clicked, &answer))
            .then_some((clicked, answer))
    }

    /// Who takes a right-click at `at`, relative to the text origin, of
    /// the providers in order, asked about the text as it is now.
    #[cfg(any(test, feature = "test-support"))]
    fn taken(&self, at: iced::Point) -> Option<popover::Taken> {
        let misspelled = || {
            let place = self.speller().and_then(|_| self.place_under(at));
            place.is_some_and(|place| place.mark.is_some())
        };
        popover::taken_in(self.providers(), misspelled, || {
            self.reading().clicked_in(&self.content, at)
        })
    }

    /// The caret's line: what the host's sources are asked about.
    fn asked_line(&self) -> Option<popover::CaretLine> {
        let cursor = self.content.cursor();
        let caret = cursor.position;
        self.content.with_lines(|lines| {
            Some(popover::CaretLine {
                index: caret.line,
                text: lines.get(caret.line)?.text.into_owned(),
                caret: caret.column,
                selected: cursor.selection.is_some_and(|anchor| anchor != caret),
            })
        })
    }

    /// Choose `row` of the open popover and close it. The suggestions menu
    /// replaces its word with the row's suggestion, as one undo step with
    /// the caret after it; the host's popover leaves the row for
    /// [`EditorView::take_choice`]. A row the popover does not have, one
    /// that cannot be chosen ([`Row::enabled`]), or an anchor whose text no
    /// longer reads as it did when the popover opened, closes it and
    /// changes nothing. So does a row of suggestions that
    /// runs a command, unless the choice named the suggestions (`named`,
    /// [`SpellEvent::AcceptOf`]): a key that did not may have been meant
    /// for suggestions that these replaced.
    fn accept(&mut self, row: usize, named: bool) -> bool {
        let menu = self.menu.take();
        let Some(popover) = menu.as_deref().or(self.popover.as_ref()) else {
            return false;
        };
        let unnamed_command = popover.mode() == Mode::Suggest
            && popover.kind(row) == Some(RowKind::Command)
            && !named;
        if unnamed_command {
            self.close_open();
            return false;
        }
        let (line, range) = (popover.line(), popover.range());
        // A row that cannot be chosen chooses nothing, as a stale one.
        let unchanged = popover.is_enabled(row)
            && self.content.with_lines(|lines| {
                lines
                    .get(line)
                    .is_some_and(|text| text.text.get(range.clone()) == Some(popover.text()))
            });
        let chosen = popover
            .id(row)
            .zip(popover.kind(row))
            .filter(|_| unchanged)
            .map(|(id, kind)| (id, popover.mode(), kind));
        let suggestion = popover.labels().get(row).filter(|_| unchanged).cloned();
        let provider = popover.provider();
        self.popover = None;
        if menu.is_none() {
            self.choice = chosen.map(|(id, mode, kind)| Chosen {
                revision: self.revision,
                choice: Choice {
                    id,
                    mode,
                    line,
                    range,
                },
                provider,
                kind,
            });
            return false;
        }
        let Some(suggestion) = suggestion else {
            return false;
        };
        let at = |column| Position { line, column };
        let edit = Replace {
            start: at(range.start),
            end: at(range.end),
            text: suggestion.clone(),
        };
        let caret = Cursor {
            position: at(range.start + suggestion.len()),
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

    // -----------------------------------------------------------------------
    // Popover
    // -----------------------------------------------------------------------

    /// Open the host's popover of `rows` at `anchor`, in `mode`, with the
    /// first row selected, and scroll the anchor's row into view if it is
    /// not; the caret and the selection stay where they are. Whatever
    /// popover was open closes. Returns whether it opened: not with no
    /// rows, nor at an anchor off the text or inside a character, nor on
    /// the iced engine, which cannot scroll to an anchor.
    ///
    /// The widget draws it and routes its keys ([`Mode`]), and the row the
    /// user chooses comes back through [`EditorView::take_choice`].
    pub fn open_popover(&mut self, anchor: Anchor, mode: Mode, rows: Vec<Row>) -> bool {
        self.close_open();
        if rows.is_empty() || self.content.kind() != BackendKind::Owned {
            return false;
        }
        let Some((line, range, text)) = self.anchored(&anchor) else {
            return false;
        };
        self.content.reveal(Position {
            line,
            column: range.start,
        });
        let popover = Popover::new(mode, line, range, text, rows);
        self.popover = Some(self.opening(popover));
        true
    }

    /// The open popover, if any: the suggestions menu's, or the host's.
    pub fn popover(&self) -> Option<&Popover> {
        self.menu.as_deref().or(self.popover.as_ref())
    }

    fn popover_mut(&mut self) -> Option<&mut Popover> {
        self.menu.as_deref_mut().or(self.popover.as_mut())
    }

    /// Close the open popover, the suggestions menu included. Returns
    /// whether one was open.
    pub fn close_popover(&mut self) -> bool {
        let open = self.popover().is_some();
        self.close_open();
        open
    }

    /// Close the open popover for Escape. The editor leaves Escape on the
    /// suggestions menu to the host, which calls this from its own key
    /// handling; Escape on a menu of the host's or on suggestions it sends
    /// on in its order ([`SpellEvent::Key`]), which does this and claims
    /// the press ([`EditorView::take_claimed_escape`]). Suggestions then
    /// stay hidden for the caret's line; see
    /// [`EditorView::suggest_suppressed`]. Returns whether one was open.
    pub fn dismiss_popover(&mut self) -> bool {
        if self.popover().map(Popover::mode) == Some(Mode::Suggest) {
            self.suppressed = Some(self.content.cursor().position.line);
        }
        self.close_popover()
    }

    /// The row the last event given to [`EditorView::spell`] chose from the
    /// host's popover, once: taking it leaves none. A choice made over a
    /// text that has changed since, or been replaced, is dropped. A row of
    /// a menu one of the host's providers opened is not given here; see
    /// [`EditorView::take_provided`].
    pub fn take_choice(&mut self) -> Option<Choice> {
        let chosen = (self.choice)
            .take_if(|chosen| chosen.provider.is_none() && chosen.kind == RowKind::Own)?;
        (chosen.revision == self.revision).then_some(chosen.choice)
    }

    /// As [`EditorView::take_choice`], for a row of a menu that one of the
    /// host's providers opened for a right-click: the provider's place in
    /// the host's list, for the host to hand the row back to it, and the
    /// row.
    pub fn take_provided(&mut self) -> Option<(usize, Choice)> {
        let chosen = (self.choice)
            .take_if(|chosen| chosen.provider.is_some() && chosen.kind == RowKind::Own)?;
        let provider = chosen.provider?;
        (chosen.revision == self.revision).then_some((provider, chosen.choice))
    }

    /// As [`EditorView::take_choice`], for a row that runs a command of the
    /// host's ([`RowKind::Command`]), whoever opened its menu: the row's id
    /// names the command, and its anchor ([`Choice::line`] and
    /// [`Choice::range`]) the place the menu was opened for, which the
    /// command acts on.
    pub fn take_command(&mut self) -> Option<Choice> {
        let chosen = self
            .choice
            .take_if(|chosen| chosen.kind == RowKind::Command)?;
        (chosen.revision == self.revision).then_some(chosen.choice)
    }

    /// The right-click that the last event given to [`EditorView::spell`]
    /// handed to the host, once: one of the host's providers took it by
    /// taking the keyboard ([`Requested::Keyboard`]). With the provider's
    /// place in the host's list and what was clicked, for the host to open
    /// its own there and give it the keyboard. What the batch of events
    /// still holds after such a click is dropped by the widget, not kept
    /// for the host: the keys, and whatever an input method composes or
    /// commits (see [`popover`]). `None` for a click the editor did not
    /// find as the widget saw it ([`SpellEvent::Context`]), over a text
    /// that has changed since, or after the host named other providers:
    /// the host then opens nothing.
    pub fn take_keyboard(&mut self) -> Option<(usize, Clicked)> {
        let handed = self.keyboard.take()?;
        (handed.revision == self.revision).then_some((handed.provider, handed.clicked))
    }

    /// Whether a popover took a press of Escape that the host has yet to
    /// hear of from its own key handling, once for each such press: the
    /// host calls this when its key handling hears of an Escape with no
    /// Ctrl, Alt or logo key ([`is_plain_escape`]), and does nothing more
    /// with a press it returns true for.
    ///
    /// The editor publishes Escape on a menu of the host's or on
    /// suggestions as [`SpellEvent::Key`], so that the host closes them in
    /// the key's order with the typing around it, and leaves the press
    /// uncaptured. A press is claimed only when that event closed a
    /// popover, so each press has one owner: the popover, or the host.
    pub fn take_claimed_escape(&mut self) -> bool {
        let claimed = self.escapes > 0;
        self.escapes = self.escapes.saturating_sub(1);
        claimed
    }

    /// Where `anchor` is in the window: the extent of its glyphs on the
    /// first row that holds any, or for a place between two characters a
    /// rectangle of no width where a caret there stands, as tall as its
    /// row. The caret's place is where the engine draws it, on the side of
    /// a wrap it is on; any other place between two rows of a wrapped line
    /// is the start of the second. In window coordinates, as the widget
    /// last placed and laid out the text; `None` before it has, for an
    /// anchor off the text, and while the anchor's row is out of view.
    pub fn anchor_bounds(&self, anchor: &Anchor) -> Option<iced::Rectangle> {
        let (line, range, _) = self.anchored(anchor)?;
        let row = self.content.anchor_row(line, range)?;
        Some(row.bounds() + (self.origin.get()? - iced::Point::ORIGIN))
    }

    /// [`EditorView::anchor_bounds`] of the open popover's anchor, which
    /// the popover hangs under.
    pub fn popover_anchor(&self) -> Option<iced::Rectangle> {
        let popover = self.popover()?;
        self.anchor_bounds(&Anchor::Range {
            line: popover.line(),
            range: popover.range(),
        })
    }

    /// The host's source of suggestions, or `None` to offer none. With one,
    /// each character typed or deleted asks it what to offer at the caret,
    /// and its answer opens in [`Mode::Suggest`], replaces the open
    /// suggestions, or closes them. Only the owned engine asks it, as only
    /// it opens popovers. The widget asks it too, for the keys that follow
    /// a typed character before the editor has applied it, so it must
    /// answer the same for the same line and caret.
    pub fn set_suggest_source(&mut self, source: Option<Arc<dyn SuggestSource>>) {
        if self.popover.as_ref().map(Popover::mode) == Some(Mode::Suggest) {
            self.popover = None;
        }
        self.suppressed = None;
        self.suggester = source;
    }

    /// The host's source of the popovers it names by number, or `None`. A
    /// key the host's policy routes as [`KeyRoute::Popover`], and
    /// [`SpellEvent::Request`], open what it answers for the caret. Only
    /// the owned engine asks it.
    pub fn set_menu_source(&mut self, source: Option<Arc<dyn MenuSource>>) {
        self.menus = source;
    }

    /// Who a right-click asks for a menu, in this order: the first whose
    /// answer opens something takes the click, and those after it are not
    /// asked. [`Provider::Spelling`] is the editor's own suggestions menu,
    /// which takes a misspelled word; a [`Provider::Host`] is asked about
    /// what lies under the pointer ([`Clicked`]), which may be a place in
    /// no span, and answers as a [`MenuSource`] does. A row chosen from
    /// its menu comes back through [`EditorView::take_provided`]. The list
    /// is the suggestions menu alone until the host sets another, and a
    /// host that wants a misspelled word to come first keeps it first.
    ///
    /// Only the owned engine asks the providers, as only it opens
    /// popovers. The widget asks them too, for the keys that follow a
    /// right-click before the editor has applied it, so each must answer
    /// the same for the same place. Setting other providers closes a menu
    /// that one of them opened, and drops what waited on one of them: a
    /// row chosen ([`EditorView::take_provided`]), the keyboard handed
    /// over ([`EditorView::take_keyboard`]), and a right-click the widget
    /// has published and the host has yet to hand here.
    pub fn set_menu_providers(&mut self, providers: Vec<Provider>) {
        let same = providers.len() == self.providers.len()
            && (providers.iter().zip(&self.providers)).all(|(new, old)| new.is(old));
        if !same {
            if (self.popover.as_ref()).is_some_and(|open| open.provider().is_some()) {
                self.popover = None;
            }
            self.choice.take_if(|chosen| chosen.provider.is_some());
            self.keyboard = None;
            self.provider_list += 1;
        }
        self.providers = providers;
    }

    /// The text and the list of providers a right-click is asked about,
    /// and the popovers opened so far.
    fn edition(&self) -> popover::Edition {
        popover::Edition {
            revision: self.revision,
            list: self.provider_list,
            opened: self.opened,
        }
    }

    /// The providers a right-click asks, on the engine that opens
    /// popovers.
    fn providers(&self) -> &[Provider] {
        if self.content.kind() == BackendKind::Owned {
            &self.providers
        } else {
            &[]
        }
    }

    /// What a right-click at `point`, relative to the text origin as a
    /// click action's position is, falls on, as the last layout placed the
    /// text: what a provider is asked about. `None` off every row of the
    /// text.
    pub fn clicked(&self, point: iced::Point) -> Option<Clicked> {
        self.reading().clicked(&self.content, point)
    }

    /// How a hit test made without the widget reads the lines.
    fn reading(&self) -> hit::Reading<'_> {
        hit::Reading {
            speller: self.speller(),
            front_matter_end: self.front_matter_end,
            tasks: self.tasks,
        }
    }

    /// The message the widget publishes for a right-click, alone in its
    /// batch of events, on the middle of the character that starts at byte
    /// `column` of `line`, or on the end of the line when `column` is its
    /// length; without a window. With a menu open the press only closes it
    /// (rule 4.4.5); otherwise the providers are asked in order. `None`
    /// where no provider takes the click, which then changes nothing, and
    /// for a place that is not laid out.
    #[cfg(any(test, feature = "test-support"))]
    pub fn right_click_message(&self, line: usize, column: usize) -> Option<Message> {
        let at = self.middle_of(line, column)?;
        if self.popover().map(Popover::mode) == Some(Mode::Menu) {
            return Some(Message::Spell(SpellEvent::Close));
        }
        let event = match self.taken(at)? {
            popover::Taken::Spelling => SpellEvent::OpenAt(at),
            popover::Taken::Host {
                provider,
                fingerprint,
                ..
            } => SpellEvent::Context {
                provider,
                at,
                seen: Seen::new(self.edition(), fingerprint),
            },
        };
        Some(Message::Spell(event))
    }

    /// The line of the task whose box `click` was on, while the text reads
    /// as it did when the box was pressed and the editor still reads task
    /// lines on the advanced engine; `None` otherwise, when the click
    /// changes nothing.
    pub fn box_clicked(&self, click: BoxClick) -> Option<usize> {
        let live = click.revision == self.revision
            && self.content.kind() == BackendKind::Owned
            && self.tasks.is_some();
        live.then_some(click.line)
    }

    /// Carry a click on a box out as a click in the text, which places the
    /// caret where the box was pressed: for a host that finds no task to
    /// change there. Nothing happens once the text has changed since.
    /// Returns whether the text changed, as [`EditorView::perform`] does.
    pub fn click_in_text(&mut self, click: BoxClick) -> bool {
        if click.revision != self.revision {
            return false;
        }
        self.perform(Action::Click(click.at))
    }

    /// The message the widget publishes for a primary click, alone in its
    /// batch of events, pressed and released on the middle of the character
    /// that starts at byte `column` of `line`, or on the end of the line
    /// when `column` is its length; without a window. On a task's box it is
    /// a [`Message::BoxClick`], and anywhere else the click that places the
    /// caret. `None` for a place that is not laid out.
    #[cfg(any(test, feature = "test-support"))]
    pub fn click_message(&self, line: usize, column: usize) -> Option<Message> {
        let at = self.middle_of(line, column)?;
        if self.popover().map(Popover::mode) == Some(Mode::Menu) {
            return Some(Message::Spell(SpellEvent::Close));
        }
        let on_box = self.content.kind() == BackendKind::Owned
            && self.tasks.is_some()
            && self
                .content
                .place_under(at, |_, _| Vec::new())
                .and_then(|place| Some((place.line, place.column?)))
                .is_some_and(|(line, column)| hit::on_box(&self.spans_at(line), column));
        Some(if on_box {
            Message::BoxClick(BoxClick::new(line, self.revision, at))
        } else {
            Message::Action(Action::Click(at))
        })
    }

    /// The middle of the character that starts at byte `column` of `line`,
    /// or of the end of the line when `column` is its length, relative to
    /// the text origin, as the last layout placed it.
    #[cfg(any(test, feature = "test-support"))]
    fn middle_of(&self, line: usize, column: usize) -> Option<iced::Point> {
        let end = self.content.with_lines(|lines| {
            let text = lines.get(line)?;
            let rest = text.text.get(column..)?;
            Some(column + rest.chars().next().map_or(0, char::len_utf8))
        })?;
        let row = self.content.anchor_row(line, column..end)?;
        let mut at = row.bounds().center();
        // A blank has no glyphs to be in the middle of: the middle of its
        // cell, between the places on either side of it.
        let after = (self.content.anchor_row(line, end..end))
            .filter(|after| end > column && row.right <= row.left && after.top == row.top);
        if let Some(after) = after {
            at.x = (row.left + after.left) / 2.0;
        }
        Some(at)
    }

    /// Lay the text out for a text area `width` wide, as the widget does
    /// for a frame, without a window: for a host's tests, which ask where
    /// text is drawn ([`EditorView::right_click_message`],
    /// [`EditorView::click_message`]) after an edit, which leaves the lines
    /// it changed to be laid out by the next frame.
    #[cfg(feature = "test-support")]
    pub fn lay_out(&mut self, width: f32, palette: Palette) {
        let settings = self.highlighter_settings(palette);
        let height = self.metrics.line * self.line_count() as f32 + self.metrics.line;
        self.content
            .lay_out_as(iced::Size::new(width, height), &settings, self.metrics);
    }

    /// The line Escape hid suggestions for ([`EditorView::dismiss_popover`]):
    /// typing there asks the source nothing until the caret has left the
    /// line, the text is replaced, or the host clears it.
    pub fn suggest_suppressed(&self) -> Option<usize> {
        self.suppressed
            .filter(|line| *line == self.content.cursor().position.line)
    }

    /// Offer suggestions on the caret's line again.
    pub fn clear_suggest_suppression(&mut self) {
        self.suppressed = None;
    }

    /// The host's source of suggestions, on the engine that opens popovers.
    fn suggest_source(&self) -> Option<&dyn SuggestSource> {
        self.suggester
            .as_deref()
            .filter(|_| self.content.kind() == BackendKind::Owned)
    }

    /// The host's source of menus, on the engine that opens popovers.
    fn menu_source(&self) -> Option<&dyn MenuSource> {
        self.menus
            .as_deref()
            .filter(|_| self.content.kind() == BackendKind::Owned)
    }

    /// The line and byte range `anchor` names, the caret's being where it
    /// is now, and the text there. `None` for a range off the text or
    /// inside a character.
    fn anchored(&self, anchor: &Anchor) -> Option<(usize, Range<usize>, String)> {
        let (line, range) = match anchor {
            Anchor::Range { line, range } => (*line, range.clone()),
            Anchor::Caret => {
                let caret = self.content.cursor().position;
                (caret.line, caret.column..caret.column)
            }
        };
        let text = self.content.with_lines(|lines| {
            let text = lines.get(line)?;
            text.text.get(range.clone()).map(str::to_owned)
        })?;
        Some((line, range, text))
    }

    /// Close whatever popover is open, as every action, intent, undo and
    /// redo does (rule 4.4.8), and stop hiding suggestions for a line the
    /// caret has left.
    fn close_open(&mut self) {
        self.menu = None;
        self.popover = None;
        self.suppressed = self.suggest_suppressed();
    }

    /// Ask the host's source what to offer at the caret, after a character
    /// was typed or deleted: its rows open as suggestions under the range
    /// it names, in place of any that were open. Nothing is offered with a
    /// selection, on a line Escape hid suggestions for, or for an answer
    /// with no rows or a range off the line ([`popover::suggested`]).
    fn suggest(&mut self) {
        if self.suggest_source().is_none() {
            return;
        }
        self.close_open();
        let suggested = self
            .asked_line()
            .filter(|line| self.suppressed != Some(line.index))
            .and_then(|line| popover::suggested(self.suggest_source()?, &line));
        self.popover = suggested.map(|popover| self.opening(popover));
    }

    /// Open the menu for the misspelled word at `target`, a line and a byte
    /// range of it; with no target, or none the proofreader can still be
    /// asked about, whatever menu was open closes.
    fn open_menu(&mut self, target: Option<(usize, Range<usize>)>) {
        self.close_open();
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
        self.opened += 1;
        let menu = SpellMenu::new(line, mark, word.to_owned(), items);
        self.menu = Some(menu.opened_as(self.opened));
    }

    /// The marks and the menu as the last frame drew them.
    #[cfg(any(test, feature = "test-support"))]
    pub fn spell_frame(&self) -> SpellFrame {
        self.spell_frame.borrow().clone()
    }

    /// The text of `line` without its ending: what a host reads a line's
    /// [`spans_at`](Self::spans_at) against, or names the word a mark of
    /// [`spell_frame`](Self::spell_frame) covers by.
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
    /// clipboard is not there, so Paste gives nothing; Copy and Cut give
    /// their [`Clip`] and Cut of a selection its delete. Dropping focus is
    /// no message.
    #[cfg(feature = "test-support")]
    pub fn key_message(
        &self,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Vec<Message> {
        self.key_routing(key, modifiers).0
    }

    /// [`key_message`](Self::key_message), and whether the focused editor
    /// captures the press, as the widget does: a press it handles through
    /// a binding of its own, but not one it gives up the keyboard for,
    /// nor one that only closes the open menu (a host's shortcut, which
    /// then still reaches the host), nor one it sends on to the host.
    #[cfg(any(test, feature = "test-support"))]
    pub fn key_routing(
        &self,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> (Vec<Message>, bool) {
        use iced::widget::text_editor::Binding;
        let popover = self.popover().map(keys::Target::of);
        let selected = self.has_selection();
        let line = || self.caret_line();
        let mut out = Vec::new();
        let binding = match keys::route(key_press(key, modifiers), self.key_context(), popover) {
            None => None,
            Some(keys::Routed::Binding(binding)) => Some(binding),
            Some(keys::Routed::Close(binding)) => {
                out.push(Message::Spell(SpellEvent::Close));
                binding
            }
            Some(keys::Routed::Pass(message)) => {
                out.push(message);
                None
            }
        };
        let captured = binding
            .as_ref()
            .is_some_and(|binding| !matches!(binding, Binding::Unfocus));
        if let Some(binding) = binding {
            keys::messages(binding, selected, &line, &mut out);
        }
        // A menu other than the suggestions menu, and suggestions with a
        // row that runs a command, are named by a choice from them, as the
        // widget names them: by an accelerator or Enter, and by Enter and
        // Tab on such suggestions.
        let open = self.popover().filter(|open| open.named_by_choice());
        if let Some(open) = open {
            for message in &mut out {
                let row = match message {
                    Message::Spell(SpellEvent::Accept(row)) => Some(Some(*row)),
                    Message::Spell(SpellEvent::AcceptSelected) => Some(None),
                    Message::Spell(SpellEvent::Key(MenuKey::Enter | MenuKey::Tab))
                        if open.mode() == Mode::Suggest && open.runs_commands() =>
                    {
                        Some(None)
                    }
                    _ => None,
                };
                if let Some(row) = row {
                    let menu = open.instance();
                    *message = Message::Spell(SpellEvent::AcceptOf { menu, row });
                }
            }
        }
        (out, captured)
    }

    /// Close the open menu for a pointer press in the editor, which does
    /// nothing more (rule 4.4.5), or with none open carry the press out as
    /// the widget does with no menu, the left press after one that only
    /// closed the menu being a single click. Returns whether the text
    /// changed.
    fn press(&mut self, press: MenuPress) -> bool {
        use iced::advanced::mouse::click::Kind;

        let (point, guess) = match press {
            MenuPress::Left(point, kind) => (point, kind),
            MenuPress::Right(point) => {
                if self.close_menu() {
                    self.drop_drags = false;
                    return false;
                }
                // The widget left the press here without asking the
                // host's providers, so none answers it; nor does the
                // suggestions menu ahead of one that comes before it.
                if !popover::spelling_leads(self.providers()) {
                    return false;
                }
                return self.spell(SpellEvent::OpenAt(point));
            }
        };
        let kind = if self.close_menu() {
            None
        } else {
            Some(match self.press_chain {
                Some(previous) => popover::click_kind(guess, previous),
                None => guess,
            })
        };
        let changed = match kind {
            None => false,
            Some(Kind::Single) => self.perform(Action::Click(point)),
            Some(Kind::Double) => self.perform(Action::SelectWord),
            Some(Kind::Triple) => self.perform(Action::SelectLine),
        };
        self.drop_drags = kind != Some(Kind::Single);
        self.press_chain = (kind != Some(guess)).then_some(kind);
        self.resolved_press = popover::ResolvedPress {
            count: self.resolved_press.count.wrapping_add(1),
            kind,
        };
        changed
    }

    /// Close the open popover for a press in the editor. Returns whether
    /// the press does nothing more: a menu takes it, and suggestions leave
    /// it to the text.
    fn close_menu(&mut self) -> bool {
        let menu = self.popover().map(Popover::mode) == Some(Mode::Menu);
        self.close_open();
        menu
    }

    /// Handle a press of `key` as the focused editor does with no popover
    /// open, carrying out what it publishes. Returns whether the text
    /// changed.
    fn press_without_popover(&mut self, key: MenuKey) -> bool {
        let press = key_press(
            iced::keyboard::Key::Named(key.named()),
            iced::keyboard::Modifiers::empty(),
        );
        let Some(binding) = keys::key_binding(press, self.key_context()) else {
            return false;
        };
        let mut messages = Vec::new();
        keys::messages(
            binding,
            self.has_selection(),
            &|| self.caret_line(),
            &mut messages,
        );
        let mut changed = false;
        for message in messages {
            changed |= self.carry_out(message);
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

    /// What lies at the character that starts at byte `column` of `line`: a
    /// misspelled word before one of the line's typed spans; see [`Hit`].
    /// `None` where neither is, and past the text.
    pub fn hit_at(&self, line: usize, column: usize) -> Option<Hit> {
        let (marks, spans) = self.ranges_at(line);
        hit::at(column, &marks, &spans)
    }

    /// What lies under `point`, relative to the text origin as a click
    /// action's position is, and its line: a misspelled word before one of
    /// the line's typed spans, as the last layout placed their glyphs; see
    /// [`Hit`].
    pub fn hit_under(&self, point: iced::Point) -> Option<(usize, Hit)> {
        let place = self.place_under(point)?;
        let span = || self.spans_at(place.line).at(place.column?).cloned();
        Some((place.line, hit::resolve(place.mark, span)?))
    }

    /// Where `point` falls in the laid out text, among the misspellings
    /// found without the widget ([`Self::marks_at`]).
    fn place_under(&self, point: iced::Point) -> Option<widget::Place> {
        self.content
            .place_under(point, |line, _| self.marks_at(line))
    }

    /// The typed spans of `line`, as its highlighter reads them. None past
    /// the text.
    pub fn spans_at(&self, line: usize) -> LineSpans {
        self.content.with_lines(|lines| {
            highlighter::spans_at(&lines, line, self.front_matter_end, self.tasks)
        })
    }

    /// The host's proofreader, on the engine that checks spelling (§3.3).
    fn speller(&self) -> Option<&dyn Proofreader> {
        self.proofreader
            .as_deref()
            .filter(|_| self.content.kind() == BackendKind::Owned)
    }

    /// The misspellings of `line`, found without the widget; see
    /// [`Self::ranges_at`]. Empty on the iced engine or with no
    /// proofreader, which is then not asked.
    fn marks_at(&self, line: usize) -> Vec<Range<usize>> {
        if self.speller().is_none() {
            return Vec::new();
        }
        self.ranges_at(line).0
    }

    /// The two lists a hit test reads for `line`, found without the widget
    /// from a fresh scan of the lines down to it (§3.3): the misspellings,
    /// the same marks a frame draws there, and the typed spans, whose
    /// prose the proofreader was asked about.
    fn ranges_at(&self, line: usize) -> (Vec<Range<usize>>, LineSpans) {
        self.reading().ranges(&self.content, line)
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
        self.revision = next_revision();
        self.close_open();
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

    // ── Revision ────────────────────────────────────────────────────────────

    /// The revision moves on every change to the text and to how its lines
    /// are read, on both engines, and on nothing else; no two editors share
    /// one.
    #[test]
    fn the_revision_moves_with_the_text_and_the_boxes_read() {
        let today = wren_markdown::date::Date::new(2026, 11, 19).unwrap();
        for kind in BackendKind::ALL {
            let mut ev = EditorView::with_backend(kind, "- [ ] one\ntwo");
            assert_ne!(
                ev.revision(),
                EditorView::with_backend(kind, "- [ ] one\ntwo").revision()
            );
            let mut last = ev.revision();
            let mut moved = |ev: &EditorView, what: &str, wanted: bool| {
                assert_eq!(ev.revision() != last, wanted, "{kind:?}: {what}");
                last = ev.revision();
            };
            ev.move_cursor(Cursor {
                position: Position { line: 1, column: 1 },
                selection: None,
            });
            ev.perform(Action::Move(Motion::Right));
            moved(&ev, "caret moves", false);
            assert!(ev.perform(Action::Edit(Edit::Insert('x'))));
            moved(&ev, "typing", true);
            assert!(ev.undo());
            moved(&ev, "undo", true);
            assert!(ev.redo());
            moved(&ev, "redo", true);
            ev.replace_text("other", false);
            moved(&ev, "a new text", true);
            ev.set_tasks(Some(wren_markdown::TaskOptions { today }));
            moved(&ev, "tasks on", true);
            ev.set_tasks(Some(wren_markdown::TaskOptions {
                today: today.add_days(1).unwrap(),
            }));
            moved(&ev, "a new day", false);
            ev.set_tasks(None);
            moved(&ev, "tasks off", true);
            let other = match kind {
                BackendKind::Iced => BackendKind::Owned,
                BackendKind::Owned => BackendKind::Iced,
            };
            assert!(ev.set_backend(other, "other"));
            moved(&ev, "an engine change", true);
        }
    }

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
                Message::Clip(clip) => ev.clip(clip),
                Message::Undo => ev.undo(),
                Message::Redo => ev.redo(),
                Message::Find | Message::Command(_) | Message::BoxClick(_) => false,
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

    // ── Clipboard keys ──────────────────────────────────────────────────────

    fn clip_text(text: &str) -> Arc<String> {
        Arc::new(text.to_owned())
    }

    #[test]
    fn a_line_cut_and_its_paste_move_whole_lines_on_both_engines() {
        for kind in BackendKind::ALL {
            let mut ev = EditorView::with_backend(kind, "one\ntwo\nthree");
            ev.move_cursor(at(1, 2));
            let line = ev.caret_line();
            assert_eq!(line, "two\n", "{kind:?}");
            assert!(ev.clip(Clip::CutLine(clip_text(&line))));
            assert_eq!(ev.text(), "one\nthree", "{kind:?}");
            assert_eq!(ev.cursor(), at(1, 2), "{kind:?}: same column, next line");

            // Pasted mid-line, it goes in above the caret's line, and the
            // caret keeps its place in its own.
            ev.move_cursor(at(0, 1));
            assert!(ev.clip(Clip::Paste(clip_text("two\n"))));
            assert_eq!(ev.text(), "two\none\nthree", "{kind:?}");
            assert_eq!(ev.cursor(), at(1, 1), "{kind:?}");

            // Each is one undo step.
            assert!(ev.undo());
            assert_eq!(ev.text(), "one\nthree", "{kind:?}");
            assert!(ev.undo());
            assert_eq!(ev.text(), "one\ntwo\nthree", "{kind:?}");

            // Other text, or a paste over a selection, is an ordinary paste.
            ev.move_cursor(at(0, 1));
            assert!(ev.clip(Clip::Paste(clip_text("X\n"))));
            assert_eq!(ev.text(), "oX\nne\ntwo\nthree", "{kind:?}");
            assert!(ev.undo());
            ev.move_cursor(Cursor {
                position: Position { line: 0, column: 3 },
                selection: Some(Position { line: 0, column: 0 }),
            });
            assert!(ev.clip(Clip::Paste(clip_text("two\n"))));
            assert_eq!(ev.text(), "two\n\ntwo\nthree", "{kind:?}");
        }
    }

    #[test]
    fn a_line_cut_at_the_end_or_alone_and_a_line_copy() {
        for kind in BackendKind::ALL {
            // The last line takes the break above it, and the caret its column.
            let mut ev = EditorView::with_backend(kind, "first\nsecond");
            ev.move_cursor(at(1, 5));
            assert!(ev.clip(Clip::CutLine(clip_text("second\n"))));
            assert_eq!(ev.text(), "first", "{kind:?}");
            assert_eq!(ev.cursor(), at(0, 5), "{kind:?}");
            // The only line empties; cut again, there is nothing to change.
            ev.move_cursor(at(0, 2));
            assert!(ev.clip(Clip::CutLine(clip_text("first\n"))));
            assert_eq!(ev.text(), "", "{kind:?}");
            assert_eq!(ev.caret_line(), "\n");
            assert!(!ev.clip(Clip::CutLine(clip_text("\n"))));

            // A copy changes nothing, yet its paste is a line paste.
            let mut ev = EditorView::with_backend(kind, "alpha\nbeta");
            ev.move_cursor(at(1, 2));
            assert!(!ev.clip(Clip::CopyLine(clip_text("beta\n"))));
            assert_eq!(ev.text(), "alpha\nbeta");
            assert!(ev.clip(Clip::Paste(clip_text("beta\n"))));
            assert_eq!(ev.text(), "alpha\nbeta\nbeta", "{kind:?}");
            assert_eq!(ev.cursor(), at(2, 2), "{kind:?}");

            // A selection copied since, even of the same text, is no line.
            assert!(!ev.clip(Clip::Selection));
            ev.move_cursor(at(0, 2));
            assert!(ev.clip(Clip::Paste(clip_text("beta\n"))));
            assert_eq!(ev.text(), "albeta\npha\nbeta\nbeta", "{kind:?}");

            // A shorter line below clamps the caret to its end.
            let mut ev = EditorView::with_backend(kind, "a long line\nab");
            ev.move_cursor(at(0, 9));
            assert!(ev.clip(Clip::CutLine(clip_text("a long line\n"))));
            assert_eq!(ev.cursor(), at(0, 2), "{kind:?}");
        }
    }

    #[test]
    fn a_line_cut_and_paste_keep_a_crlf_note_crlf() {
        for kind in BackendKind::ALL {
            let mut ev = EditorView::with_backend(kind, "one\r\ntwo\r\nthree");
            ev.move_cursor(at(0, 0));
            assert!(ev.clip(Clip::CutLine(clip_text("one\n"))));
            assert_eq!(ev.text(), "two\r\nthree", "{kind:?}");
            ev.move_cursor(at(1, 3));
            assert!(ev.clip(Clip::Paste(clip_text("one\n"))));
            assert_eq!(ev.text(), "two\r\none\r\nthree", "{kind:?}");
        }
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn the_clipboard_keys_take_the_line_only_with_nothing_selected() {
        use iced::keyboard::{Key, Modifiers};
        let command = if cfg!(target_os = "macos") {
            Modifiers::LOGO
        } else {
            Modifiers::CTRL
        };
        for kind in BackendKind::ALL {
            let mut ev = EditorView::with_backend(kind, "one\ntwo");
            ev.move_cursor(at(1, 1));
            let key = |c: &str| Key::Character(c.into());
            assert!(matches!(
                ev.key_message(key("x"), command).as_slice(),
                [Message::Clip(Clip::CutLine(text))] if text.as_str() == "two\n"
            ));
            assert!(matches!(
                ev.key_message(key("c"), command).as_slice(),
                [Message::Clip(Clip::CopyLine(text))] if text.as_str() == "two\n"
            ));
            ev.move_cursor(Cursor {
                position: Position { line: 1, column: 2 },
                selection: Some(Position { line: 1, column: 0 }),
            });
            assert!(matches!(
                ev.key_message(key("x"), command).as_slice(),
                [
                    Message::Clip(Clip::Selection),
                    Message::Action(Action::Edit(Edit::Delete))
                ]
            ));
            assert!(matches!(
                ev.key_message(key("c"), command).as_slice(),
                [Message::Clip(Clip::Selection)]
            ));
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

    // ── Task emoji ──────────────────────────────────────────────────────────

    /// The task glyphs (discovery/tasks.md §5.2, §5.3 and §5.7), each bare
    /// and with U+FE0F, in task lines on both engines: the emoji spike,
    /// discovery/tasks-build.md item 0.3. Whatever font draws a glyph, or
    /// none, it is one grapheme, so these assert steps and byte offsets,
    /// never pixels.
    mod task_emoji {
        use iced::{Point, Rectangle, Size};
        use unicode_segmentation::UnicodeSegmentation;

        use super::*;

        const GLYPHS: [&str; 8] = [
            "\u{1F4C5}", // 📅 due
            "\u{2705}",  // ✅ done
            "\u{274C}",  // ❌ cancelled
            "\u{1F53A}", // 🔺 highest
            "\u{23EB}",  // ⏫ high
            "\u{1F53C}", // 🔼 medium
            "\u{1F53D}", // 🔽 low
            "\u{23EC}",  // ⏬ lowest
        ];

        /// Each glyph bare, then with the emoji variation selector.
        fn clusters() -> impl Iterator<Item = String> {
            GLYPHS
                .iter()
                .flat_map(|glyph| [glyph.to_string(), format!("{glyph}\u{FE0F}")])
        }

        /// A task line with `cluster` as its due token, and where it is.
        fn task_line(cluster: &str) -> (String, Range<usize>) {
            let line = format!("- [ ] Due {cluster} 2026-12-01");
            let start = line.find(cluster).expect("the cluster");
            (line, start..start + cluster.len())
        }

        fn without(line: &str, range: Range<usize>) -> String {
            format!("{}{}", &line[..range.start], &line[range.end..])
        }

        fn selecting(range: Range<usize>) -> Cursor {
            Cursor {
                position: Position {
                    line: 0,
                    column: range.end,
                },
                selection: Some(Position {
                    line: 0,
                    column: range.start,
                }),
            }
        }

        #[test]
        fn each_glyph_bare_or_with_its_selector_is_one_grapheme() {
            for cluster in clusters() {
                assert_eq!(cluster.graphemes(true).count(), 1, "{cluster:?}");
            }
        }

        #[test]
        fn arrows_step_over_each_glyph_once_and_shift_selects_exactly_it() {
            let size = Size::new(600.0, 200.0);
            for kind in BackendKind::ALL {
                for cluster in clusters() {
                    let case = format!("{kind:?} {cluster:?}");
                    let (line, range) = task_line(&cluster);
                    let mut ev = EditorView::with_backend(kind, &line);
                    ev.content.lay_out(size);
                    ev.move_cursor(at(0, range.start));
                    ev.perform(Action::Move(Motion::Right));
                    assert_eq!(ev.cursor(), at(0, range.end), "{case}: Right");
                    ev.perform(Action::Move(Motion::Left));
                    assert_eq!(ev.cursor(), at(0, range.start), "{case}: Left");
                    ev.perform(Action::Select(Motion::Right));
                    assert_eq!(ev.cursor(), selecting(range.clone()), "{case}");
                    assert_eq!(ev.selected_text().as_deref(), Some(cluster.as_str()));
                    // From after it, Shift+Left takes the same text.
                    ev.move_cursor(at(0, range.end));
                    ev.perform(Action::Select(Motion::Left));
                    assert_eq!(ev.selected_text().as_deref(), Some(cluster.as_str()));
                    assert_eq!(ev.text(), line, "{case}: no edit");
                }
            }
        }

        #[test]
        fn one_backspace_after_or_delete_before_takes_the_whole_glyph() {
            for kind in BackendKind::ALL {
                for assist in [true, false] {
                    for cluster in clusters() {
                        let case = format!("{kind:?} assist {assist} {cluster:?}");
                        let (line, range) = task_line(&cluster);
                        let expected = without(&line, range.clone());
                        let mut ev = EditorView::with_backend(kind, &line);
                        ev.set_markdown_assist(assist);
                        ev.move_cursor(at(0, range.end));
                        assert!(ev.intent(Intent::Backspace), "{case}");
                        assert_eq!(ev.text(), expected, "{case}: Backspace");
                        assert_eq!(ev.cursor(), at(0, range.start), "{case}");
                        assert!(ev.undo(), "{case}");
                        assert_eq!(ev.text(), line, "{case}: one undo");

                        ev.move_cursor(at(0, range.start));
                        assert!(ev.intent(Intent::Delete), "{case}");
                        assert_eq!(ev.text(), expected, "{case}: Delete");
                        assert_eq!(ev.cursor(), at(0, range.start), "{case}");
                    }
                }
            }
        }

        /// The keys as the focused editor routes them, not the actions.
        #[cfg(feature = "test-support")]
        #[test]
        fn the_arrow_and_backspace_keys_do_the_same() {
            use iced::keyboard::key::Named;
            use iced::keyboard::{Key, Modifiers};

            fn press(ev: &mut EditorView, key: Named, modifiers: Modifiers) {
                for message in ev.key_message(Key::Named(key), modifiers) {
                    match message {
                        Message::Action(action) => {
                            ev.perform(action);
                        }
                        Message::Intent(intent) => {
                            ev.intent(intent);
                        }
                        other => panic!("{key:?}: {other:?}"),
                    }
                }
            }

            let size = Size::new(600.0, 200.0);
            for kind in BackendKind::ALL {
                for cluster in clusters() {
                    let case = format!("{kind:?} {cluster:?}");
                    let (line, range) = task_line(&cluster);
                    let mut ev = EditorView::with_backend(kind, &line);
                    ev.content.lay_out(size);
                    ev.move_cursor(at(0, range.start));
                    press(&mut ev, Named::ArrowRight, Modifiers::empty());
                    assert_eq!(ev.cursor(), at(0, range.end), "{case}: Right");
                    press(&mut ev, Named::ArrowLeft, Modifiers::empty());
                    assert_eq!(ev.cursor(), at(0, range.start), "{case}: Left");
                    press(&mut ev, Named::ArrowRight, Modifiers::SHIFT);
                    assert_eq!(ev.cursor(), selecting(range.clone()), "{case}");
                    ev.move_cursor(at(0, range.end));
                    press(&mut ev, Named::Backspace, Modifiers::empty());
                    assert_eq!(ev.text(), without(&line, range.clone()), "{case}");
                    assert_eq!(ev.cursor(), at(0, range.start), "{case}");
                }
            }
        }

        /// The one rectangle a selection of `range` on line 0 draws.
        fn drawn(ev: &mut EditorView, range: Range<usize>, size: Size) -> Rectangle {
            ev.move_cursor(selecting(range.clone()));
            ev.content.lay_out(size);
            match ev.content.selection_geometry() {
                Some(text_editor::Selection::Range(rects)) if rects.len() == 1 => rects[0],
                other => panic!("{range:?}: {other:?}"),
            }
        }

        #[test]
        fn clicks_land_beside_each_drawn_glyph_and_wrapped_rows_stay_aligned() {
            // Narrow enough that the second glyph wraps onto a later row.
            let size = Size::new(200.0, 400.0);
            for kind in BackendKind::ALL {
                for cluster in clusters() {
                    let case = format!("{kind:?} {cluster:?}");
                    let line = format!(
                        "- [ ] Due {cluster} 2026-12-01, then words enough to wrap \
                         this task line under its box {cluster} 2026-12-01"
                    );
                    let first = line.find(&cluster).unwrap();
                    let last = line.rfind(&cluster).unwrap();
                    let mut ev = EditorView::with_backend(kind, &line);
                    for start in [first, last] {
                        let range = start..start + cluster.len();
                        let rect = drawn(&mut ev, range.clone(), size);
                        assert!(rect.width > 0.0, "{case}: {rect:?}");
                        // The left half of the glyph is before it, the right
                        // half after it. Both engines take a click in whole
                        // pixels, so each click is a whole pixel inside the
                        // glyph, at least a pixel from its middle. Whatever
                        // font draws it, a glyph too narrow for that must
                        // still land on one of its two sides.
                        let middle = rect.x + rect.width / 2.0;
                        let y = rect.center_y().floor();
                        for (x, column) in [
                            ((rect.x + 1.0).floor(), range.start),
                            ((rect.x + rect.width - 1.0).floor(), range.end),
                        ] {
                            ev.perform(Action::Click(Point::new(x, y)));
                            ev.content.lay_out(size);
                            let landed = ev.cursor();
                            if (x - middle).abs() >= 1.0 && x >= rect.x && x < rect.x + rect.width {
                                assert_eq!(landed, at(0, column), "{case}: {start} at {x}");
                            } else {
                                assert!(
                                    landed == at(0, range.start) || landed == at(0, range.end),
                                    "{case}: {start} at {x} in {rect:?}: {landed:?}"
                                );
                            }
                        }
                    }

                    // The row the last glyph is on starts under the box, at
                    // the item's content column, on the advanced engine, which
                    // hangs wrapped rows, and at the margin on the classic one.
                    let row = drawn(&mut ev, last..last + cluster.len(), size).y;
                    assert!(row > 0.0, "{case}: the glyph is on a wrapped row");
                    let row_start = line
                        .grapheme_indices(true)
                        .filter(|(_, grapheme)| !grapheme.trim().is_empty())
                        .map(|(at, grapheme)| drawn(&mut ev, at..at + grapheme.len(), size))
                        .find(|rect| rect.y == row)
                        .expect("the row's first glyph");
                    let column = match kind {
                        BackendKind::Owned => "- ".len(),
                        BackendKind::Iced => 0,
                    };
                    let margin = drawn(&mut ev, column..column + 1, size).x;
                    assert_eq!(row_start.x, margin, "{case}");
                }
            }
        }
    }

    // ── Task statuses ───────────────────────────────────────────────────────

    /// A box of any status (discovery/tasks.md §5.1) on both engines, with
    /// the host's tasks on and off: tasks build item 1.1.
    #[allow(clippy::single_range_in_vec_init)] // a line often has one strike or prose range
    mod task_status {
        use wren_markdown::date::Date;
        use wren_markdown::TaskOptions;

        use super::*;

        fn tasks_on() -> Option<TaskOptions> {
            Some(TaskOptions {
                today: Date::new(2026, 11, 19).expect("a date"),
            })
        }

        /// An editor on `kind` holding `text`, with the caret at the end of
        /// its first line and the host's `tasks`.
        fn editor(kind: BackendKind, text: &str, tasks: Option<TaskOptions>) -> EditorView {
            let mut ev = EditorView::with_backend(kind, text);
            ev.set_tasks(tasks);
            let end = text.split('\n').next().expect("a line").len();
            ev.move_cursor(at(0, end));
            ev
        }

        const NOTE: &str = "- [ ] open\n- [/] doing\n- [x] done\n- [-] dropped\n- [界] wide";

        /// `(text, colour)` of each span of each line of `ev`.
        fn colours(ev: &EditorView) -> Vec<Vec<(String, Color)>> {
            let text = ev.text();
            let lines: Vec<&str> = text.split('\n').collect();
            ev.highlighted(fixtures::DARK)
                .iter()
                .zip(lines)
                .map(|((spans, _), line)| {
                    spans
                        .iter()
                        .map(|(range, highlight)| {
                            let color = highlight.to_format().color.expect("a coloured span");
                            (line[range.clone()].to_owned(), color)
                        })
                        .collect()
                })
                .collect()
        }

        fn strikes(ev: &EditorView) -> Vec<Vec<Range<usize>>> {
            ev.highlighted(fixtures::DARK)
                .into_iter()
                .map(|(_, decor)| decor.strikes)
                .collect()
        }

        #[test]
        fn tasks_are_off_until_the_host_turns_them_on() {
            for kind in BackendKind::ALL {
                let mut ev = EditorView::with_backend(kind, NOTE);
                assert_eq!(ev.tasks(), None, "{kind:?}");
                assert_eq!(ev.plan_options().tasks, None);
                assert_eq!(ev.highlighter_settings(fixtures::DARK).tasks, None);
                ev.set_tasks(tasks_on());
                assert_eq!(ev.tasks(), tasks_on(), "{kind:?}");
                assert_eq!(ev.plan_options().tasks, tasks_on());
                assert_eq!(ev.highlighter_settings(fixtures::DARK).tasks, tasks_on());
                // A new engine keeps the host's tasks.
                let other = match kind {
                    BackendKind::Iced => BackendKind::Owned,
                    BackendKind::Owned => BackendKind::Iced,
                };
                ev.set_backend(other, NOTE);
                assert_eq!(ev.tasks(), tasks_on(), "{kind:?}");
                assert_eq!(colours(&ev)[1][0].0, "- [/] ");
            }
        }

        #[test]
        fn each_status_is_coloured_by_its_type_on_both_engines() {
            let accent = fixtures::DARK.accent;
            let faint = fixtures::DARK.faint;
            let completed = fixtures::DARK.completed;
            for kind in BackendKind::ALL {
                let on = editor(kind, NOTE, tasks_on());
                assert_eq!(
                    colours(&on),
                    [
                        vec![("- [ ] ".to_owned(), accent)],
                        vec![("- [/] ".to_owned(), accent)],
                        vec![("- [x] ".to_owned(), faint), ("done".to_owned(), completed)],
                        vec![
                            ("- [-] ".to_owned(), faint),
                            ("dropped".to_owned(), completed)
                        ],
                        vec![("- [界] ".to_owned(), accent)],
                    ],
                    "{kind:?}"
                );
                assert_eq!(
                    strikes(&on),
                    [vec![], vec![], vec![6..10], vec![6..13], vec![]],
                    "{kind:?}"
                );

                // Off, the doing, cancelled and unknown boxes are text, and
                // the other two lines are what they are with tasks on.
                let off = editor(kind, NOTE, None);
                let (off_colours, on_colours) = (colours(&off), colours(&on));
                for line in [0, 2] {
                    assert_eq!(off_colours[line], on_colours[line], "{kind:?} line {line}");
                }
                for line in [1, 3, 4] {
                    assert_eq!(
                        off_colours[line][0],
                        ("- ".to_owned(), accent),
                        "{kind:?} line {line}"
                    );
                }
                assert_eq!(
                    strikes(&off),
                    [vec![], vec![], vec![6..10], vec![], vec![]],
                    "{kind:?}"
                );
            }
        }

        /// `(line, caret column or the line's end, key, the text with tasks
        /// on, the text with tasks off)`.
        const KEYS: &[(&str, Option<usize>, Intent, &str, &str)] = &[
            (
                "- [/] doing",
                None,
                Intent::Enter,
                "- [/] doing\n- [ ] ",
                "- [/] doing\n- ",
            ),
            (
                "- [-] dropped",
                None,
                Intent::Enter,
                "- [-] dropped\n- [ ] ",
                "- [-] dropped\n- ",
            ),
            (
                "- [界] wide",
                None,
                Intent::Enter,
                "- [界] wide\n- [ ] ",
                "- [界] wide\n- ",
            ),
            (
                "- [/] doing",
                Some(6),
                Intent::Enter,
                "- [ ]\n- [/] doing",
                "- [/] \n- doing",
            ),
            (
                "- [界] wide",
                Some(8),
                Intent::Enter,
                "- [ ]\n- [界] wide",
                "- [界] \n- wide",
            ),
            (
                "- [/] doing",
                Some(6),
                Intent::Backspace,
                "- doing",
                "- [/]doing",
            ),
            (
                "- [-] dropped",
                Some(6),
                Intent::Backspace,
                "- dropped",
                "- [-]dropped",
            ),
            (
                "- [界] wide",
                Some(8),
                Intent::Backspace,
                "- wide",
                "- [界]wide",
            ),
            ("- [界]", None, Intent::Backspace, "- ", "- [界"),
            (
                "- [/] doing",
                None,
                Intent::ToggleTask,
                "- [x] doing",
                "- [ ] [/] doing",
            ),
            (
                "- [-] dropped",
                None,
                Intent::ToggleTask,
                "- [ ] dropped",
                "- [ ] [-] dropped",
            ),
            (
                "- [界] wide",
                None,
                Intent::ToggleTask,
                "- [x] wide",
                "- [ ] [界] wide",
            ),
            (
                "- [x] done",
                None,
                Intent::ToggleTask,
                "- [ ] done",
                "- [ ] done",
            ),
            (
                "- [✔\u{fe0f}] two",
                None,
                Intent::ToggleTask,
                "- [ ] [✔\u{fe0f}] two",
                "- [ ] [✔\u{fe0f}] two",
            ),
            (
                "- [a]: http://u",
                None,
                Intent::Enter,
                "- [a]: http://u\n- ",
                "- [a]: http://u\n- ",
            ),
        ];

        #[test]
        fn the_markdown_keys_read_a_status_box_only_with_tasks_on() {
            for kind in BackendKind::ALL {
                for (line, column, intent, on, off) in KEYS {
                    for (tasks, expected) in [(tasks_on(), on), (None, off)] {
                        let case = format!("{kind:?} {intent:?} on {line:?}, tasks {tasks:?}");
                        let mut ev = editor(kind, line, tasks);
                        if let Some(column) = column {
                            ev.move_cursor(at(0, *column));
                        }
                        assert!(ev.intent(*intent), "{case}");
                        assert_eq!(ev.text(), **expected, "{case}");
                        assert!(ev.undo(), "{case}");
                        assert_eq!(ev.text(), *line, "{case}: one undo step");
                    }
                }
            }
        }

        /// The toggle over a selection: with an unfinished task in it every
        /// task becomes done; otherwise every task, cancelled ones
        /// included, becomes open. The caret keeps its place in the text
        /// when a wide symbol before it shrinks.
        #[test]
        fn the_toggle_follows_the_statuses_a_selection_holds() {
            let all = |text: &str| Cursor {
                position: Position {
                    line: text.split('\n').count() - 1,
                    column: text.rsplit('\n').next().expect("a line").len(),
                },
                selection: Some(Position { line: 0, column: 0 }),
            };
            for kind in BackendKind::ALL {
                for (before, after) in [
                    ("- [ ] a\n- [-] b\n- [界] c", "- [x] a\n- [x] b\n- [x] c"),
                    ("- [/] a\n- [X] b", "- [x] a\n- [X] b"),
                    ("- [x] a\n- [-] b", "- [ ] a\n- [ ] b"),
                    ("- [-] a\n- [-] b", "- [ ] a\n- [ ] b"),
                ] {
                    let mut ev = editor(kind, before, tasks_on());
                    ev.move_cursor(all(before));
                    assert!(ev.intent(Intent::ToggleTask), "{kind:?} {before:?}");
                    assert_eq!(ev.text(), after, "{kind:?} {before:?}");
                    assert_eq!(ev.cursor(), all(after), "{kind:?} {before:?}");
                }
                let mut ev = editor(kind, "- [界] wide", tasks_on());
                assert!(ev.intent(Intent::ToggleTask));
                assert_eq!(ev.cursor(), at(0, "- [x] wide".len()), "{kind:?}");
            }
        }

        /// Spell check reads what the highlighter calls prose, so with
        /// tasks on a box's own character is never a word.
        #[test]
        fn spelling_reads_a_status_tasks_description_and_never_its_box() {
            let proofreader = Arc::new(fixtures::Rejects::new(&["wrd", "teh"]));
            let text = "- [/] teh wrd\n- [wrd] wrd\n- [-] teh";
            let mut ev = editor(BackendKind::Owned, text, tasks_on());
            ev.set_proofreader(Some(proofreader));
            let marks = |ev: &EditorView| -> Vec<(usize, Range<usize>)> {
                (0..ev.line_count())
                    .flat_map(|line| ev.marks_at(line).into_iter().map(move |mark| (line, mark)))
                    .collect()
            };
            let on = marks(&ev);
            assert_eq!(
                on,
                [(0, 6..9), (0, 10..13), (1, 3..6), (1, 8..11), (2, 6..9)]
            );
            // The marks are found in the prose the widget's highlighter
            // gives with the same settings.
            for (line, (_, decor)) in ev.highlighted(fixtures::DARK).iter().enumerate() {
                for (_, mark) in on.iter().filter(|(at, _)| *at == line) {
                    assert!(
                        decor
                            .prose
                            .iter()
                            .any(|prose| prose.start <= mark.start && mark.end <= prose.end),
                        "line {line}: {mark:?} in {:?}",
                        decor.prose
                    );
                }
            }
            assert_eq!(ev.highlighted(fixtures::DARK)[0].1.prose, [6..13]);
            ev.set_tasks(None);
            assert_eq!(
                marks(&ev),
                on,
                "the same words are misspelled with tasks off"
            );
            assert_ne!(ev.highlighted(fixtures::DARK)[0].1.prose, [6..13]);

            // A box's character that is no word is marked only while the
            // box is text.
            let proofreader = Arc::new(fixtures::Rejects::new(&["z", "teh"]));
            let mut ev = editor(BackendKind::Owned, "- [z] teh", tasks_on());
            ev.set_proofreader(Some(proofreader));
            assert_eq!(marks(&ev), [(0, 6..9)]);
            ev.set_tasks(None);
            assert_eq!(marks(&ev), [(0, 3..4), (0, 6..9)]);
        }

        #[test]
        fn a_change_of_tasks_closes_the_suggestions_menu() {
            let proofreader =
                Arc::new(fixtures::Rejects::new(&["teh"]).suggesting(&[("teh", &["the"])]));
            let mut ev = editor(BackendKind::Owned, "- [/] teh", tasks_on());
            ev.set_proofreader(Some(proofreader));
            ev.move_cursor(at(0, 7));
            ev.open_menu(Some((0, 6..9)));
            assert!(ev.menu.is_some());
            ev.set_tasks(tasks_on());
            assert!(ev.menu.is_some(), "the same tasks change nothing");
            ev.set_tasks(None);
            assert!(ev.menu.is_none());
        }
    }

    // ── The task line edit ──────────────────────────────────────────────────

    /// `wren_markdown::task::edit` applied through [`EditorView::apply`]:
    /// each change is one undo step on both engines (tasks build item 1.4).
    mod task_line_edit {
        use wren_markdown::date::Date;
        use wren_markdown::task::{self, Change, Stale};
        use wren_markdown::{Document, Options, TaskOptions};

        use super::*;

        fn tasks_on() -> Options {
            Options {
                tasks: Some(TaskOptions {
                    today: Date::new(2026, 11, 19).expect("a date"),
                }),
                ..Options::default()
            }
        }

        const NOTE: &str = "# Plan\n\n- [ ] Send the deck 🔽 📅 2026-11-30 ^deck\n\
                            - [x] Book ✅ 2026-11-18\n- [ ] Call @sam about it ⏫";

        /// `(line, caret column, change, the line after it, the caret after
        /// it)`.
        fn cases() -> Vec<(usize, usize, Change, &'static str, usize)> {
            let due = Date::new(2026, 12, 1);
            vec![
                (
                    2,
                    19,
                    Change::ToggleDone,
                    "- [x] Send the deck 🔽 📅 2026-11-30 ✅ 2026-11-19 ^deck",
                    19,
                ),
                (3, 3, Change::ToggleDone, "- [ ] Book", 3),
                (
                    2,
                    0,
                    Change::Cycle,
                    "- [/] Send the deck 🔽 📅 2026-11-30 ^deck",
                    0,
                ),
                (
                    3,
                    10,
                    Change::SetStatus('-'),
                    "- [-] Book ❌ 2026-11-19",
                    10,
                ),
                (
                    2,
                    19,
                    Change::SetDue(due),
                    "- [ ] Send the deck 🔽 📅 2026-12-01 ^deck",
                    19,
                ),
                (
                    2,
                    19,
                    Change::SetDue(None),
                    "- [ ] Send the deck 🔽 ^deck",
                    19,
                ),
                (
                    2,
                    19,
                    Change::SetPriority(Some(task::Priority::High)),
                    "- [ ] Send the deck ⏫ 📅 2026-11-30 ^deck",
                    19,
                ),
                (
                    2,
                    19,
                    Change::RaisePriority,
                    "- [ ] Send the deck 📅 2026-11-30 ^deck",
                    19,
                ),
                (
                    2,
                    19,
                    Change::LowerPriority,
                    "- [ ] Send the deck ⏬ 📅 2026-11-30 ^deck",
                    19,
                ),
                (
                    2,
                    19,
                    Change::AddOwner("sam".into()),
                    "- [ ] Send the deck @sam 🔽 📅 2026-11-30 ^deck",
                    19,
                ),
                // No `@sam` on the line: nothing to undo.
                (
                    3,
                    10,
                    Change::RemoveOwner("sam".into()),
                    "- [x] Book ✅ 2026-11-18",
                    10,
                ),
                (
                    4,
                    21,
                    Change::RemoveOwner("sam".into()),
                    "- [ ] Call about it ⏫",
                    16,
                ),
                (
                    2,
                    8,
                    Change::SetDescription("Mail the deck".into()),
                    "- [ ] Mail the deck 🔽 📅 2026-11-30 ^deck",
                    19,
                ),
                (
                    2,
                    19,
                    Change::Compound(vec![
                        Change::SetDescription("Mail it".into()),
                        Change::SetStatus('/'),
                        Change::SetDue(due),
                        Change::SetPriority(Some(task::Priority::High)),
                        Change::AddOwner("sam".into()),
                    ]),
                    "- [/] Mail it @sam ⏫ 📅 2026-12-01 ^deck",
                    13,
                ),
            ]
        }

        #[test]
        fn each_change_is_one_undo_step_on_both_engines() {
            for kind in BackendKind::ALL {
                for ending in ["\n", "\r\n"] {
                    let note = NOTE.replace('\n', ending);
                    for (line, column, change, after, caret) in cases() {
                        let what = format!("{kind:?} {ending:?} {change:?}");
                        let mut ev = EditorView::with_backend(kind, &note);
                        ev.set_tasks(tasks_on().tasks);
                        ev.move_cursor(at(line, column));
                        let text = ev.text();
                        let doc = Document::new(&text);
                        let selection = markdown_selection(ev.cursor());
                        let edit = task::edit(&doc, line, selection, &change, &tasks_on())
                            .expect("a task line");
                        let plan = edit.plan_for(&doc).expect("the line it read");
                        let edits: Vec<Replace> =
                            plan.edits.iter().cloned().map(Replace::from).collect();
                        let changed = ev.apply(&edits, iced_cursor(plan.selection));
                        assert_eq!(changed, !edit.is_noop(), "{what}");
                        let mut want: Vec<&str> = NOTE.split('\n').collect();
                        want[line] = after;
                        assert_eq!(ev.text(), want.join(ending), "{what}");
                        assert_eq!(ev.cursor(), at(line, caret), "{what}");
                        if !changed {
                            assert!(!ev.can_undo(), "{what}");
                            continue;
                        }
                        // The plan is stale now: its line has changed.
                        let now = ev.text();
                        assert_eq!(edit.plan_for(&Document::new(&now)), Err(Stale), "{what}");
                        // One undo restores the line and the caret.
                        assert!(ev.undo(), "{what}");
                        assert_eq!(ev.text(), note, "{what}");
                        assert_eq!(ev.cursor(), at(line, column), "{what}");
                        assert!(!ev.can_undo(), "{what}: one step");
                        assert!(ev.redo(), "{what}");
                        assert_eq!(ev.text(), want.join(ending), "{what}");
                    }
                }
            }
        }
    }

    mod hits {
        use wren_markdown::date::Date;
        use wren_markdown::span::SpanKind;
        use wren_markdown::task::Priority;
        use wren_markdown::TaskOptions;

        use super::*;

        const TEXT: &str = "- [ ] Call @sam [plna](u) ⏫ 📅 2026-12-01 #home\nplain teh";

        /// An editor on `kind` holding [`TEXT`], with the host's tasks on
        /// or off and a proofreader that rejects four of its words.
        fn editor(kind: BackendKind, tasks: bool) -> EditorView {
            let mut ev = EditorView::with_backend(kind, TEXT);
            ev.set_tasks(tasks.then(|| TaskOptions {
                today: Date::new(2026, 11, 19).expect("a date"),
            }));
            ev.set_proofreader(Some(Arc::new(fixtures::Rejects::new(&[
                "sam", "plna", "home", "teh",
            ]))));
            ev
        }

        /// What a hit on line `line` is, in a word, and the text it covers.
        fn seen(line: usize, hit: Option<Hit>) -> Option<(&'static str, &'static str)> {
            let hit = hit?;
            let line = TEXT.split('\n').nth(line).expect("a line of the text");
            let kind = match &hit {
                Hit::Misspelling(_) => "misspelling",
                Hit::Span(span) => match span.kind {
                    SpanKind::Box(_) => "box",
                    SpanKind::Owner { .. } => "owner",
                    SpanKind::Link { .. } => "link",
                    SpanKind::Priority(Priority::High) => "high priority",
                    SpanKind::Date { .. } => "date",
                    SpanKind::Tag => "tag",
                    _ => "another span",
                },
            };
            Some((kind, &line[hit.range().clone()]))
        }

        /// A misspelled word is the hit where it lies in an owner, a link
        /// or a tag; with no speller, as on the classic engine, the span
        /// is; and a task token is one on both engines.
        #[test]
        fn the_hit_at_a_column_is_a_misspelling_before_a_span_on_both_engines() {
            for kind in BackendKind::ALL {
                let ev = editor(kind, true);
                let spelled = kind == BackendKind::Owned;
                let word = |word: &'static str, kind: &'static str, span: &'static str| {
                    Some(if spelled {
                        ("misspelling", word)
                    } else {
                        (kind, span)
                    })
                };
                for (column, expected) in [
                    (0, None),
                    (3, Some(("box", "[ ]"))),
                    (6, None),
                    (11, Some(("owner", "@sam"))),
                    (12, word("sam", "owner", "@sam")),
                    (15, None),
                    (16, Some(("link", "[plna](u)"))),
                    (17, word("plna", "link", "[plna](u)")),
                    (26, Some(("high priority", "⏫"))),
                    (29, None),
                    (30, Some(("date", "📅 2026-12-01"))),
                    (44, Some(("date", "📅 2026-12-01"))),
                    (46, Some(("tag", "#home"))),
                    (47, word("home", "tag", "#home")),
                    (51, None),
                ] {
                    assert_eq!(seen(0, ev.hit_at(0, column)), expected, "{kind:?} {column}");
                }
                assert_eq!(
                    seen(1, ev.hit_at(1, 7)),
                    spelled.then_some(("misspelling", "teh")),
                    "{kind:?}"
                );
                assert_eq!(ev.hit_at(1, 2), None, "{kind:?}");
                assert_eq!(ev.hit_at(2, 0), None, "{kind:?}: past the text");
            }
        }

        /// With the host's tasks off a task line has no task span, so only
        /// its inline Markdown and its misspellings are hit.
        #[test]
        fn with_tasks_off_no_task_token_is_hit() {
            for kind in BackendKind::ALL {
                let ev = editor(kind, false);
                let spelled = kind == BackendKind::Owned;
                for (column, expected) in [
                    (3, None),
                    (11, None),
                    (12, spelled.then_some(("misspelling", "sam"))),
                    (16, Some(("link", "[plna](u)"))),
                    (26, None),
                    (30, None),
                    (46, None),
                ] {
                    assert_eq!(seen(0, ev.hit_at(0, column)), expected, "{kind:?} {column}");
                }
                assert!(
                    ev.spans_at(0).spans.iter().all(|span| !span.kind.is_task()),
                    "{kind:?}"
                );
                assert_eq!(editor(kind, true).spans_at(0).spans.len(), 6, "{kind:?}");
            }
        }

        /// The same hit under the pointer, by the glyphs the last layout
        /// placed; the suggestions menu opens on the misspelling alone.
        #[test]
        fn the_hit_under_a_point_reads_the_laid_out_glyphs_on_both_engines() {
            for kind in BackendKind::ALL {
                let mut ev = editor(kind, true);
                ev.content.lay_out(iced::Size::new(600.0, 400.0));
                let spelled = kind == BackendKind::Owned;
                let point =
                    |cells: f32, row: f32| iced::Point::new(cells * CELL, row * 24.0 + 12.0);
                for (cells, row, expected) in [
                    (3.5, 0.0, Some(("box", "[ ]"))),
                    (7.5, 0.0, None),
                    (11.5, 0.0, Some(("owner", "@sam"))),
                    (
                        13.5,
                        0.0,
                        Some(if spelled {
                            ("misspelling", "sam")
                        } else {
                            ("owner", "@sam")
                        }),
                    ),
                    (16.5, 0.0, Some(("link", "[plna](u)"))),
                    (7.5, 1.0, spelled.then_some(("misspelling", "teh"))),
                    (2.5, 1.0, None),
                    (2.5, 2.0, None),
                ] {
                    let at_point = point(cells, row);
                    let found = ev.hit_under(at_point);
                    assert!(
                        found.as_ref().is_none_or(|(line, _)| *line == row as usize),
                        "{kind:?} {at_point:?}"
                    );
                    let hit = found.map(|(_, hit)| hit);
                    let misspelled = matches!(hit, Some(Hit::Misspelling(_)));
                    assert_eq!(seen(row as usize, hit), expected, "{kind:?} {at_point:?}");
                    ev.close_spell_menu();
                    ev.spell(SpellEvent::OpenAt(at_point));
                    assert_eq!(
                        ev.spell_menu().is_some(),
                        misspelled,
                        "{kind:?} {at_point:?}"
                    );
                }
            }
        }
    }

    /// Task colours (tasks build item 2.2) on both engines: what each token
    /// is drawn in, with the host's tasks on and off, and what spell check
    /// reads of a task line.
    mod task_colours {
        use wren_markdown::date::Date;
        use wren_markdown::TaskOptions;

        use super::*;

        /// Pinned to Thursday 19 November 2026, as every task test is.
        const NOTE: &str = "\
- [ ] Future @alice.chen ⏫ 📅 2026-11-20
- [ ] Today 📅 2026-11-19
- [/] Overdue @sam 📅 2026-11-18
- [x] Done late 📅 2026-11-01 ✅ 2026-11-18
- [-] Dropped @sam ❌ 2026-11-16
- [ ] No such day 📅 2026-02-30
- [ ] Card 🆔 a1 🔽";

        fn tasks(day: u8) -> Option<TaskOptions> {
            Some(TaskOptions {
                today: Date::new(2026, 11, day).expect("a date"),
            })
        }

        /// The colour, if any, and boldness of `token` on line `line` of
        /// `ev`, drawn with the dark fixture palette.
        fn look(ev: &EditorView, line: usize, token: &str) -> (Option<Color>, bool) {
            let text = NOTE.split('\n').nth(line).expect("a line");
            let start = text.find(token).expect("the token on its line");
            let end = start + token.len();
            let highlighted = ev.highlighted(fixtures::DARK);
            let (spans, _) = &highlighted[line];
            let over: Vec<_> = spans
                .iter()
                .filter(|(range, _)| range.start < end && start < range.end)
                .collect();
            match over.as_slice() {
                [] => (None, false),
                [(range, highlight)] => {
                    assert!(range.start <= start && end <= range.end, "{token:?}");
                    let format = highlight.to_format();
                    let bold = format
                        .font
                        .is_some_and(|font| font.weight == iced::font::Weight::Bold);
                    (format.color, bold)
                }
                more => panic!("{token:?} lies in {} spans: {more:?}", more.len()),
            }
        }

        #[test]
        #[allow(clippy::single_range_in_vec_init)] // a struck line has one strike
        fn each_task_token_takes_its_colour_on_both_engines() {
            let p = fixtures::DARK;
            for kind in BackendKind::ALL {
                let mut ev = EditorView::with_backend(kind, NOTE);
                ev.set_tasks(tasks(19));
                for (line, token, expected) in [
                    (0, "📅 2026-11-20", (Some(p.date), false)),
                    (0, "@alice.chen", (Some(p.owner), false)),
                    (0, "⏫", (None, false)),
                    (1, "📅 2026-11-19", (Some(p.date), false)),
                    (2, "📅 2026-11-18", (Some(p.overdue), true)),
                    (2, "@sam", (Some(p.owner), false)),
                    (3, "📅 2026-11-01 ✅ 2026-11-18", (Some(p.faint), false)),
                    (4, "@sam", (Some(p.completed), false)),
                    (4, "❌ 2026-11-16", (Some(p.faint), false)),
                    (5, "📅 2026-02-30", (Some(p.overdue), false)),
                    (6, "🆔 a1", (Some(p.faint), false)),
                    (6, "🔽", (None, false)),
                ] {
                    assert_eq!(look(&ev, line, token), expected, "{kind:?} {token:?}");
                }
                let strikes: Vec<_> = ev
                    .highlighted(fixtures::DARK)
                    .into_iter()
                    .map(|(_, decor)| decor.strikes)
                    .collect();
                assert_eq!(strikes[3], [6..15], "{kind:?}: the description only");
                assert_eq!(strikes[4], [6..18], "{kind:?}");

                // Off, no token has a colour of its own.
                ev.set_tasks(None);
                for (line, token) in [
                    (0, "📅 2026-11-20"),
                    (0, "@alice.chen"),
                    (2, "📅 2026-11-18"),
                    (5, "📅 2026-02-30"),
                    (6, "🆔 a1"),
                ] {
                    assert_eq!(look(&ev, line, token), (None, false), "{kind:?} {token:?}");
                }
            }
        }

        /// A later day read by the same editor turns the dates it passed
        /// overdue, and only those of unfinished tasks.
        #[test]
        fn a_later_today_turns_the_same_lines_overdue_on_both_engines() {
            let p = fixtures::DARK;
            for kind in BackendKind::ALL {
                let mut ev = EditorView::with_backend(kind, NOTE);
                ev.set_tasks(tasks(19));
                assert_eq!(look(&ev, 0, "📅 2026-11-20"), (Some(p.date), false));
                assert_eq!(look(&ev, 1, "📅 2026-11-19"), (Some(p.date), false));
                ev.set_tasks(tasks(21));
                assert_eq!(look(&ev, 0, "📅 2026-11-20"), (Some(p.overdue), true));
                assert_eq!(look(&ev, 1, "📅 2026-11-19"), (Some(p.overdue), true));
                assert_eq!(look(&ev, 3, "📅 2026-11-01"), (Some(p.faint), false));
            }
        }

        /// Spell check reads a task's words, its owners and tags among
        /// them, but not its dates, priority or id: the speller is asked
        /// about none of them, on the engine that has one.
        #[test]
        fn spell_check_reads_no_task_token_with_tasks_on() {
            let text = "- [ ] Mial @alice.chen ⏫ 📅 2026-11-20 🆔 qzx ^blk";
            for kind in BackendKind::ALL {
                let mut ev = EditorView::with_backend(kind, text);
                ev.set_proofreader(Some(Arc::new(fixtures::Rejects::new(&[
                    "Mial", "qzx", "blk", "2026", "11", "20",
                ]))));
                ev.set_tasks(tasks(19));
                let prose: Vec<&str> = ev
                    .spans_at(0)
                    .prose()
                    .into_iter()
                    .map(|range| &text[range])
                    .collect();
                assert_eq!(prose, ["Mial @alice.chen ", " ", " ", " "], "{kind:?}");
                assert_eq!(
                    ev.highlighted(fixtures::DARK)[0].1.prose,
                    ev.spans_at(0).prose(),
                    "{kind:?}"
                );
                let marks: Vec<&str> = ev
                    .marks_at(0)
                    .into_iter()
                    .map(|range| &text[range])
                    .collect();
                if kind == BackendKind::Owned {
                    assert_eq!(marks, ["Mial"], "{kind:?}");
                } else {
                    assert!(marks.is_empty(), "{kind:?}");
                }
                // Off, the tokens are prose, as they were.
                ev.set_tasks(None);
                let marks: Vec<&str> = ev
                    .marks_at(0)
                    .into_iter()
                    .map(|range| &text[range])
                    .collect();
                if kind == BackendKind::Owned {
                    assert!(
                        marks.contains(&"qzx") && marks.contains(&"blk"),
                        "{marks:?}"
                    );
                }
            }
        }
    }

    // ── The popover ─────────────────────────────────────────────────────────

    fn rows(labels: &[&str]) -> Vec<Row> {
        (10..)
            .zip(labels)
            .map(|(id, label)| Row::new(id, *label))
            .collect()
    }

    fn range(line: usize, range: Range<usize>) -> Anchor {
        Anchor::Range { line, range }
    }

    /// `(mode, line, range, text, labels, selected)` of the open popover.
    type PopoverState = (
        Mode,
        usize,
        Range<usize>,
        String,
        Vec<String>,
        Option<usize>,
    );

    fn popover_of(ev: &EditorView) -> Option<PopoverState> {
        ev.popover().map(|popover| {
            (
                popover.mode(),
                popover.line(),
                popover.range(),
                popover.text().to_owned(),
                popover.labels().to_vec(),
                popover.selected(),
            )
        })
    }

    /// An owned-engine editor over "- [ ] call sam" with a menu of three
    /// rows open on the box.
    fn with_menu() -> EditorView {
        let mut ev = EditorView::with_backend(BackendKind::Owned, "- [ ] call sam\nnext");
        ev.move_cursor(at(0, 8));
        assert!(ev.open_popover(range(0, 2..5), Mode::Menu, rows(&["Open", "Doing", "Done"])));
        ev
    }

    #[test]
    fn a_popover_opens_at_a_range_or_the_caret_with_its_first_row_selected() {
        let ev = with_menu();
        let labels = vec!["Open".to_owned(), "Doing".to_owned(), "Done".to_owned()];
        assert_eq!(
            popover_of(&ev),
            Some((Mode::Menu, 0, 2..5, "[ ]".into(), labels, Some(0)))
        );
        assert_eq!(ev.popover().and_then(|popover| popover.id(2)), Some(12));
        // It is no suggestions menu.
        assert!(ev.spell_menu().is_none());

        // The caret is the moving end of a selection, as it is when the
        // popover opens.
        let mut ev = EditorView::with_backend(BackendKind::Owned, "one\nné two");
        ev.move_cursor(select((0, 1), (1, 3)));
        assert!(ev.open_popover(Anchor::Caret, Mode::Suggest, rows(&["a"])));
        assert_eq!(
            popover_of(&ev),
            Some((
                Mode::Suggest,
                1,
                3..3,
                String::new(),
                vec!["a".into()],
                Some(0)
            ))
        );
        ev.move_cursor(at(0, 0));
        assert!(ev.popover().is_none());

        // Opening again replaces what was open.
        assert!(ev.open_popover(range(0, 0..3), Mode::Menu, rows(&["a", "b"])));
        ev.spell(SpellEvent::Move(1));
        assert!(ev.open_popover(range(1, 0..3), Mode::Menu, rows(&["c"])));
        assert_eq!(
            popover_of(&ev),
            Some((Mode::Menu, 1, 0..3, "né".into(), vec!["c".into()], Some(0)))
        );
    }

    #[test]
    fn a_popover_does_not_open_off_the_text_without_rows_or_on_the_iced_engine() {
        let mut ev = EditorView::with_backend(BackendKind::Owned, "one\nné two");
        for (name, anchor) in [
            ("past the lines", range(2, 0..0)),
            ("past the line", range(0, 2..4)),
            ("inside a character", range(1, 0..2)),
            #[allow(clippy::reversed_empty_ranges)]
            ("backwards", range(0, 2..1)),
        ] {
            assert!(ev.open_popover(range(0, 0..3), Mode::Menu, rows(&["a"])));
            assert!(!ev.open_popover(anchor, Mode::Menu, rows(&["a"])), "{name}");
            // And what was open has closed.
            assert!(ev.popover().is_none(), "{name}");
        }
        assert!(!ev.open_popover(range(0, 0..3), Mode::Menu, Vec::new()));
        assert!(
            ev.open_popover(range(0, 3..3), Mode::Menu, rows(&["a"])),
            "the line's end"
        );

        let mut ev = EditorView::with_backend(BackendKind::Iced, "one");
        assert!(!ev.open_popover(range(0, 0..3), Mode::Menu, rows(&["a"])));
        assert!(!ev.open_popover(Anchor::Caret, Mode::Suggest, rows(&["a"])));
        assert!(ev.popover().is_none());
    }

    #[test]
    fn the_suggestions_menu_and_the_hosts_popover_are_never_both_open() {
        let (mut ev, _) = checked("teh cat");
        ev.move_cursor(at(0, 1));
        assert!(ev.open_popover(range(0, 4..7), Mode::Menu, rows(&["a"])));
        ev.spell(SpellEvent::OpenAtCaret);
        // The popover that is open is the menu's.
        assert_eq!(ev.spell_menu().map(SpellMenu::word), Some("teh"));
        assert_eq!(ev.popover().map(Popover::text), Some("teh"));
        assert!(ev.popover.is_none());
        assert!(ev.open_popover(range(0, 4..7), Mode::Menu, rows(&["a"])));
        assert!(ev.spell_menu().is_none());
        assert_eq!(ev.popover().map(Popover::text), Some("cat"));
        // A request that finds no word closes the host's popover too.
        ev.move_cursor(at(0, 5));
        assert!(ev.open_popover(range(0, 4..7), Mode::Menu, rows(&["a"])));
        ev.spell(SpellEvent::OpenAtCaret);
        assert!(ev.popover().is_none());
        // Closing the suggestions menu alone leaves the host's popover.
        assert!(ev.open_popover(range(0, 4..7), Mode::Menu, rows(&["a"])));
        assert!(!ev.close_spell_menu());
        assert!(ev.popover().is_some());
        assert!(ev.close_popover());
        assert!(!ev.close_popover());
        // And closing any popover closes the suggestions menu.
        ev.move_cursor(at(0, 1));
        ev.spell(SpellEvent::OpenAtCaret);
        assert!(ev.close_popover());
        assert!(ev.spell_menu().is_none());
    }

    #[test]
    fn opening_a_popover_scrolls_its_anchors_row_into_view_and_leaves_the_caret() {
        let mut text: String = (0..60).map(|i| format!("line {i}\n")).collect();
        text.push_str("the end");
        let mut ev = EditorView::with_backend(BackendKind::Owned, &text);
        let size = iced::Size::new(400.0, 240.0);
        ev.content.lay_out(size);
        let selection = select((0, 0), (1, 2));
        ev.move_cursor(selection);
        ev.content.lay_out(size);
        assert_eq!(ev.first_visible_line(), 0);
        assert!(ev.open_popover(range(60, 4..7), Mode::Menu, rows(&["a"])));
        ev.content.lay_out(size);
        // Ten rows in view, the last of them the anchor's.
        assert_eq!(ev.content.scroll().line, 51, "{:?}", ev.content.scroll());
        assert_eq!(ev.cursor(), selection);
        assert!(!ev.can_undo());
    }

    #[test]
    fn the_selection_of_a_popover_moves_and_stops_at_its_ends() {
        let mut ev = with_menu();
        let selected = |ev: &EditorView| ev.popover().and_then(Popover::selected);
        ev.spell(SpellEvent::Move(-1));
        assert_eq!(selected(&ev), Some(0));
        ev.spell(SpellEvent::Move(1));
        ev.spell(SpellEvent::Move(1));
        ev.spell(SpellEvent::Move(1));
        assert_eq!(selected(&ev), Some(2));
        ev.spell(SpellEvent::Hover(1));
        assert_eq!(selected(&ev), Some(1));
        ev.spell(SpellEvent::Hover(3));
        assert_eq!(selected(&ev), Some(1));
    }

    #[test]
    fn a_chosen_row_comes_back_once_with_its_anchor_and_the_text_stays() {
        let chosen = |id| Choice {
            id,
            mode: Mode::Menu,
            line: 0,
            range: 2..5,
        };
        let mut ev = with_menu();
        let cursor = ev.cursor();
        assert!(!ev.spell(SpellEvent::Accept(2)));
        assert!(ev.popover().is_none());
        assert_eq!(ev.take_choice(), Some(chosen(12)));
        assert_eq!(ev.take_choice(), None);
        assert_eq!(
            (ev.text().as_str(), ev.cursor()),
            ("- [ ] call sam\nnext", cursor)
        );
        assert!(!ev.can_undo());

        // Enter chooses the selected row.
        let mut ev = with_menu();
        ev.spell(SpellEvent::Move(1));
        assert!(!ev.spell(SpellEvent::AcceptSelected));
        assert_eq!(ev.take_choice(), Some(chosen(11)));

        // A choice not taken does not outlive the next event.
        let mut ev = with_menu();
        ev.spell(SpellEvent::Accept(0));
        ev.spell(SpellEvent::Close);
        assert_eq!(ev.take_choice(), None);

        // A row the popover does not have closes it and chooses nothing,
        // as closing does.
        for event in [SpellEvent::Accept(3), SpellEvent::Close] {
            let mut ev = with_menu();
            ev.spell(event);
            assert!(ev.popover().is_none(), "{event:?}");
            assert_eq!(ev.take_choice(), None, "{event:?}");
        }
        // Nor does an event with nothing open choose.
        let mut ev = EditorView::with_backend(BackendKind::Owned, "one");
        ev.spell(SpellEvent::Accept(0));
        ev.spell(SpellEvent::AcceptSelected);
        assert_eq!(ev.take_choice(), None);
    }

    #[test]
    fn a_row_chosen_over_a_changed_anchor_chooses_nothing() {
        let mut ev = with_menu();
        // Under the popover, where no edit of the editor's own goes.
        ev.content.perform(Action::Move(Motion::DocumentStart));
        ev.content.perform(Action::Edit(Edit::Delete));
        assert_eq!(ev.content.text(), " [ ] call sam\nnext");
        assert!(ev.popover().is_some());
        assert!(!ev.spell(SpellEvent::Accept(1)));
        assert!(ev.popover().is_none());
        assert_eq!(ev.take_choice(), None);
    }

    /// Each way a popover closes without a choice.
    const CLOSES: [(&str, Step); 5] = [
        ("a close", |ev| {
            ev.spell(SpellEvent::Close);
        }),
        ("the host's close", |ev| {
            ev.close_popover();
        }),
        ("escape", |ev| {
            ev.dismiss_popover();
        }),
        ("a row it does not have", |ev| {
            ev.spell(SpellEvent::Accept(9));
        }),
        ("a right press in the editor", |ev| {
            ev.spell(SpellEvent::Press(MenuPress::Right(iced::Point::new(
                200.0, 12.0,
            ))));
        }),
    ];

    #[test]
    fn closing_a_popover_leaves_the_text_the_caret_and_the_selection() {
        for mode in [Mode::Menu, Mode::Suggest] {
            for (name, close) in CLOSES {
                let mut ev = EditorView::with_backend(BackendKind::Owned, "- [ ] call sam");
                let selection = select((0, 12), (0, 6));
                ev.move_cursor(selection);
                assert!(ev.open_popover(range(0, 2..5), mode, rows(&["a", "b"])));
                ev.spell(SpellEvent::Move(1));
                close(&mut ev);
                assert!(ev.popover().is_none(), "{mode:?} {name}");
                assert_eq!(ev.cursor(), selection, "{mode:?} {name}");
                assert_eq!(ev.text(), "- [ ] call sam", "{mode:?} {name}");
                assert!(!ev.can_undo(), "{mode:?} {name}");
            }
            // Choosing a row leaves them to the host as well.
            let mut ev = EditorView::with_backend(BackendKind::Owned, "- [ ] call sam");
            let selection = select((0, 12), (0, 6));
            ev.move_cursor(selection);
            assert!(ev.open_popover(Anchor::Caret, mode, rows(&["a", "b"])));
            ev.spell(SpellEvent::AcceptSelected);
            assert_eq!(ev.cursor(), selection, "{mode:?}");
            assert_eq!(
                ev.take_choice(),
                Some(Choice {
                    id: 10,
                    mode,
                    line: 0,
                    range: 6..6
                })
            );
        }
    }

    #[test]
    fn a_left_press_in_the_editor_only_closes_a_popover() {
        use iced::advanced::mouse::click::Kind;
        let mut ev = with_menu();
        let cursor = ev.cursor();
        let press = SpellEvent::Press(MenuPress::Left(iced::Point::new(1.0, 12.0), Kind::Single));
        assert!(!ev.spell(press));
        assert!(ev.popover().is_none());
        assert_eq!(ev.cursor(), cursor);
        // With none open, the same press is the click it would have been.
        ev.content.lay_out(iced::Size::new(400.0, 240.0));
        ev.spell(press);
        assert_eq!(ev.cursor(), at(0, 0));
    }

    #[test]
    fn a_press_the_host_resolves_over_suggestions_is_the_ordinary_press() {
        use iced::advanced::mouse::click::Kind;
        // Suggestions leave presses to the text: the press closes them and
        // moves the caret, as it does with nothing open.
        let (mut ev, _) = checked("cat teh");
        ev.content.lay_out(iced::Size::new(400.0, 240.0));
        ev.move_cursor(at(0, 2));
        assert!(ev.open_popover(range(0, 0..2), Mode::Suggest, rows(&["a"])));
        let on_c = iced::Point::new(1.0, 12.0);
        ev.spell(SpellEvent::Press(MenuPress::Left(on_c, Kind::Single)));
        assert!(ev.popover().is_none());
        assert_eq!(ev.cursor(), at(0, 0));
        // A right press on a misspelled word opens the suggestions menu.
        ev.move_cursor(at(0, 2));
        assert!(ev.open_popover(range(0, 0..2), Mode::Suggest, rows(&["a"])));
        let on_teh = iced::Point::new(5.5 * CELL, 12.0);
        ev.spell(SpellEvent::Press(MenuPress::Right(on_teh)));
        assert_eq!(ev.spell_menu().map(SpellMenu::word), Some("teh"));
        assert_eq!(ev.cursor(), at(0, 2));
        // A menu takes either press and does nothing more.
        assert!(ev.open_popover(range(0, 0..2), Mode::Menu, rows(&["a"])));
        ev.spell(SpellEvent::Press(MenuPress::Right(on_teh)));
        assert!(ev.popover().is_none());
    }

    #[test]
    fn whatever_moves_or_changes_the_text_closes_a_popover() {
        let steps: [(&str, Step); 16] = [
            ("typing", |ev| {
                ev.perform(Action::Edit(Edit::Insert('x')));
            }),
            ("a deletion", |ev| {
                ev.intent(Intent::Backspace);
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
            ("a clipboard key", |ev| {
                ev.clip(Clip::Selection);
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
                ev.set_find_query("call");
                ev.find_next();
            }),
            ("replaced text", |ev| {
                ev.replace_text("- [ ] call sam", true)
            }),
            ("a new note", |ev| ev.replace_text("- [ ] call kim", false)),
            ("a new engine", |ev| {
                ev.set_backend(BackendKind::Iced, "- [ ] call sam");
            }),
        ];
        for mode in [Mode::Menu, Mode::Suggest] {
            for (name, step) in steps {
                let mut ev = with_menu();
                assert!(ev.open_popover(range(0, 2..5), mode, rows(&["a"])));
                step(&mut ev);
                assert!(ev.popover().is_none(), "{mode:?} {name}");
            }
        }
        let changed: [Step; 5] = [
            |ev| ev.set_font_size(16.0),
            |ev| ev.set_line_height(2.0),
            |ev| ev.set_font(Font::with_name("JetBrains Mono")),
            |ev| ev.set_line_numbers(false),
            |ev| ev.set_markdown_assist(false),
        ];
        for (i, step) in changed.into_iter().enumerate() {
            let mut ev = with_menu();
            step(&mut ev);
            assert!(ev.popover().is_none(), "{i}");
        }
        // A new proofreader closes the suggestions menu alone, and the same
        // setting nothing.
        let unchanged: [Step; 3] = [
            |ev| ev.set_proofreader(Some(Arc::new(fixtures::Rejects::new(&["teh"])))),
            |ev| ev.set_font_size(14.0),
            |ev| ev.set_line_numbers(true),
        ];
        for (i, step) in unchanged.into_iter().enumerate() {
            let mut ev = with_menu();
            step(&mut ev);
            assert!(ev.popover().is_some(), "{i}");
        }
    }

    // ── Suggestions ─────────────────────────────────────────────────────────

    /// An owned-engine editor over `text`, with a source that completes
    /// `due`, `duplicate` and `today`, and the caret at `(line, column)`.
    fn suggesting(
        text: &str,
        line: usize,
        column: usize,
    ) -> (EditorView, Arc<fixtures::Completes>) {
        let source = Arc::new(fixtures::Completes::new(&["due", "duplicate", "today"]));
        let mut ev = EditorView::with_backend(BackendKind::Owned, text);
        ev.set_suggest_source(Some(source.clone()));
        ev.move_cursor(at(line, column));
        (ev, source)
    }

    fn type_text(ev: &mut EditorView, text: &str) {
        for c in text.chars() {
            ev.perform(Action::Edit(Edit::Insert(c)));
        }
    }

    /// The `(range, labels)` of the open suggestions.
    fn suggested(ev: &EditorView) -> Option<(Range<usize>, Vec<String>)> {
        let popover = ev.popover()?;
        assert_eq!(popover.mode(), Mode::Suggest);
        assert_eq!(popover.selected(), Some(0));
        Some((popover.range(), popover.labels().to_vec()))
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    #[test]
    fn suggestions_follow_what_is_typed_and_deleted() {
        let (mut ev, source) = suggesting("- [ ] pay ", 0, 10);
        type_text(&mut ev, "d");
        assert_eq!(suggested(&ev), None);
        assert_eq!(source.asked(), [("- [ ] pay d".to_owned(), 11)]);
        type_text(&mut ev, "u");
        assert_eq!(
            suggested(&ev),
            Some((10..12, strings(&["due", "duplicate"])))
        );
        assert_eq!(ev.popover().map(Popover::text), Some("du"));
        // The selection starts over with each answer.
        ev.spell(SpellEvent::Move(1));
        type_text(&mut ev, "e");
        assert_eq!(suggested(&ev), Some((10..13, strings(&["due"]))));
        type_text(&mut ev, "x");
        assert_eq!(suggested(&ev), None);
        // Deleting asks again, as a key of the editor's or an action.
        assert!(ev.intent(Intent::Backspace));
        assert_eq!(suggested(&ev), Some((10..13, strings(&["due"]))));
        assert!(ev.perform(Action::Edit(Edit::Backspace)));
        assert_eq!(
            suggested(&ev),
            Some((10..12, strings(&["due", "duplicate"])))
        );
        assert_eq!(source.asked().len(), 6);
        assert_eq!(ev.text(), "- [ ] pay du");
        // Typing over a selection asks once it is gone.
        ev.move_cursor(select((0, 6), (0, 12)));
        type_text(&mut ev, "t");
        assert_eq!(suggested(&ev), None);
        type_text(&mut ev, "o");
        assert_eq!(suggested(&ev), Some((6..8, strings(&["today"]))));
    }

    #[test]
    fn only_typing_and_deleting_ask_the_source() {
        let steps: [(&str, Step); 9] = [
            ("a motion", |ev| {
                ev.perform(Action::Move(Motion::Left));
            }),
            ("a new line", |ev| {
                ev.intent(Intent::Enter);
            }),
            ("a forward delete", |ev| {
                ev.intent(Intent::Delete);
            }),
            ("a word delete", |ev| {
                ev.intent(Intent::DeleteWordBackward);
            }),
            ("a paste, as an input method commits", |ev| {
                ev.perform(Action::Edit(Edit::Paste(Arc::new("e".into()))));
            }),
            ("a clipboard paste", |ev| {
                ev.clip(Clip::Paste(Arc::new("e".into())));
            }),
            ("an applied edit", |ev| {
                let end = Position { line: 0, column: 2 };
                let edit = Replace {
                    start: end,
                    end,
                    text: "e".into(),
                };
                ev.apply(&[edit], at(0, 3));
            }),
            ("undo", |ev| {
                ev.undo();
            }),
            ("a moved cursor", |ev| ev.move_cursor(at(0, 2))),
        ];
        for (name, step) in steps {
            let (mut ev, source) = suggesting("", 0, 0);
            type_text(&mut ev, "du");
            assert!(suggested(&ev).is_some(), "{name}");
            step(&mut ev);
            assert!(ev.popover().is_none(), "{name}");
            assert_eq!(source.asked().len(), 2, "{name}");
        }
        // With a selection left after the key, nothing is asked.
        let (mut ev, source) = suggesting("du", 0, 2);
        ev.content.perform(Action::Select(Motion::Left));
        ev.suggest();
        assert!(ev.popover().is_none());
        assert!(source.asked().is_empty());
    }

    #[test]
    fn an_answer_off_the_line_or_with_no_rows_offers_nothing() {
        /// Answers every request with one range and one list of rows.
        struct Fixed(Range<usize>, Vec<Row>);

        impl SuggestSource for Fixed {
            fn suggest(&self, _line: &str, _caret: usize) -> Option<Suggestion> {
                Some(Suggestion {
                    range: self.0.clone(),
                    rows: self.1.clone(),
                })
            }
        }

        for (name, range, labels, opens) in [
            ("no rows", 0..2, &[][..], false),
            ("past the line", 2..9, &["a"][..], false),
            ("inside a character", 0..1, &["a"][..], false),
            ("a place", 3..3, &["a"][..], true),
            ("a range", 0..2, &["a"][..], true),
        ] {
            let mut ev = EditorView::with_backend(BackendKind::Owned, "é ");
            ev.set_suggest_source(Some(Arc::new(Fixed(range, rows(labels)))));
            ev.move_cursor(at(0, 3));
            type_text(&mut ev, "x");
            assert_eq!(ev.popover().is_some(), opens, "{name}");
        }
    }

    #[test]
    fn the_iced_engine_and_an_editor_with_no_source_suggest_nothing() {
        let source = Arc::new(fixtures::Completes::new(&["due"]));
        let mut ev = EditorView::with_backend(BackendKind::Iced, "");
        ev.set_suggest_source(Some(source.clone()));
        type_text(&mut ev, "du");
        assert!(ev.popover().is_none());
        assert!(source.asked().is_empty());

        // Taking the source away closes its suggestions, and no others.
        let (mut ev, source) = suggesting("", 0, 0);
        type_text(&mut ev, "du");
        assert!(ev.popover().is_some());
        ev.set_suggest_source(None);
        assert!(ev.popover().is_none());
        type_text(&mut ev, "e");
        assert!(ev.popover().is_none());
        assert_eq!(source.asked().len(), 2);
        assert!(ev.open_popover(range(0, 0..3), Mode::Menu, rows(&["a"])));
        ev.set_suggest_source(Some(source));
        assert!(ev.popover().is_some());
    }

    #[test]
    fn escape_hides_suggestions_for_the_line_until_the_caret_leaves_it() {
        let (mut ev, source) = suggesting("one\n", 1, 0);
        type_text(&mut ev, "du");
        assert_eq!(ev.suggest_suppressed(), None);
        assert!(ev.dismiss_popover());
        assert!(ev.popover().is_none());
        assert_eq!(ev.suggest_suppressed(), Some(1));
        // Nothing more is offered on the line, nor asked.
        type_text(&mut ev, "e");
        ev.intent(Intent::Backspace);
        assert!(ev.popover().is_none());
        assert_eq!(source.asked().len(), 2);
        assert_eq!(ev.suggest_suppressed(), Some(1));
        // A second Escape finds nothing to close and hides nothing new.
        assert!(!ev.dismiss_popover());

        // Leaving the line and coming back offers again.
        ev.perform(Action::Move(Motion::Up));
        assert_eq!(ev.suggest_suppressed(), None);
        ev.perform(Action::Move(Motion::Down));
        ev.perform(Action::Move(Motion::End));
        type_text(&mut ev, "e");
        assert_eq!(suggested(&ev), Some((0..3, strings(&["due"]))));
        assert_eq!(ev.suggest_suppressed(), None);

        // The host clears it, and so does a new text.
        assert!(ev.dismiss_popover());
        ev.clear_suggest_suppression();
        assert_eq!(ev.suggest_suppressed(), None);
        ev.intent(Intent::Backspace);
        assert_eq!(suggested(&ev), Some((0..2, strings(&["due", "duplicate"]))));
        assert!(ev.dismiss_popover());
        ev.replace_text("one\ndu", true);
        assert_eq!(ev.suggest_suppressed(), None);
        type_text(&mut ev, "e");
        assert!(suggested(&ev).is_some());

        // Escape on a menu hides no suggestions.
        let (mut ev, _) = suggesting("du", 0, 2);
        assert!(ev.open_popover(Anchor::Caret, Mode::Menu, rows(&["a"])));
        assert!(ev.dismiss_popover());
        assert_eq!(ev.suggest_suppressed(), None);
        type_text(&mut ev, "e");
        assert!(suggested(&ev).is_some());
    }

    #[test]
    fn a_popovers_key_acts_on_what_is_open_and_is_the_texts_with_nothing_open() {
        let chosen = |ev: &mut EditorView| ev.take_choice().map(|choice| (choice.id, choice.mode));
        // Among suggestions, Up and Down move and Enter and Tab choose.
        for key in [MenuKey::Enter, MenuKey::Tab] {
            let (mut ev, _) = suggesting("", 0, 0);
            type_text(&mut ev, "du");
            ev.spell(SpellEvent::Key(MenuKey::Down));
            ev.spell(SpellEvent::Key(MenuKey::Down));
            ev.spell(SpellEvent::Key(MenuKey::Up));
            ev.spell(SpellEvent::Key(MenuKey::Down));
            assert_eq!(ev.popover().and_then(Popover::selected), Some(1), "{key:?}");
            assert!(!ev.spell(SpellEvent::Key(key)), "{key:?}");
            assert_eq!(chosen(&mut ev), Some((1, Mode::Suggest)), "{key:?}");
            assert_eq!(
                (ev.text().as_str(), ev.cursor()),
                ("du", at(0, 2)),
                "{key:?}"
            );
        }
        // With nothing open they are the text's keys.
        let (mut ev, _) = suggesting("- a\n- b", 1, 3);
        assert!(!ev.spell(SpellEvent::Key(MenuKey::Up)));
        assert_eq!(ev.cursor(), at(0, 3));
        assert!(!ev.spell(SpellEvent::Key(MenuKey::Down)));
        assert_eq!(ev.cursor(), at(1, 3));
        assert!(ev.spell(SpellEvent::Key(MenuKey::Tab)));
        assert_eq!(ev.text(), "- a\n  - b");
        assert!(ev.spell(SpellEvent::Key(MenuKey::Enter)));
        assert_eq!(ev.text(), "- a\n  - b\n  - ");
        assert_eq!(chosen(&mut ev), None);
        // A menu takes Up, Down and Enter; Tab is any other key to it,
        // which closes it and indents.
        let mut ev = with_menu();
        ev.spell(SpellEvent::Key(MenuKey::Down));
        assert!(!ev.spell(SpellEvent::Key(MenuKey::Enter)));
        assert_eq!(chosen(&mut ev), Some((11, Mode::Menu)));
        let mut ev = EditorView::with_backend(BackendKind::Owned, "- a\n- [ ] b");
        ev.move_cursor(at(1, 7));
        assert!(ev.open_popover(range(1, 2..5), Mode::Menu, rows(&["Open", "Done"])));
        assert!(ev.spell(SpellEvent::Key(MenuKey::Tab)));
        assert!(ev.popover().is_none());
        assert_eq!(chosen(&mut ev), None);
        assert_eq!(ev.text(), "- a\n  - [ ] b");
    }

    #[test]
    fn an_anchor_has_no_place_in_the_window_before_the_text_is_laid_out() {
        let ev = with_menu();
        assert_eq!(ev.popover_anchor(), None);
        assert_eq!(ev.anchor_bounds(&Anchor::Caret), None);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn key_routing_without_a_window_reads_the_open_popover() {
        use iced::keyboard::key::Named;
        use iced::keyboard::{Key, Modifiers};

        let none = Modifiers::empty();
        let mut ev = EditorView::with_backend(BackendKind::Owned, "- [ ] call sam");
        let status = vec![
            Row::new(1, "Doing").accelerator(Accelerator::plain('/')),
            Row::new(2, "Done").accelerator(Accelerator::plain('x')),
            Row::new(3, "Highest").accelerator(Accelerator::shifted('h')),
        ];
        assert!(ev.open_popover(range(0, 2..5), Mode::Menu, status));
        let events = |ev: &EditorView, key: Key, modifiers| -> Vec<SpellEvent> {
            ev.key_message(key, modifiers)
                .into_iter()
                .map(|message| match message {
                    Message::Spell(event) => event,
                    message => panic!("{message:?}"),
                })
                .collect()
        };
        let character = |c: &str| Key::Character(c.into());
        // A choice names the menu (item 3.4).
        let menu = ev.popover().unwrap().instance();
        let chose = |row| SpellEvent::AcceptOf {
            menu,
            row: Some(row),
        };
        assert_eq!(events(&ev, character("x"), none), [chose(1)]);
        assert_eq!(events(&ev, character("/"), none), [chose(0)]);
        assert_eq!(events(&ev, character("h"), Modifiers::SHIFT), [chose(2)]);
        assert_eq!(
            events(&ev, Key::Named(Named::ArrowDown), none),
            [SpellEvent::Move(1)]
        );
        // Escape on a menu of the host's is sent on in its order; with a
        // shortcut's modifier it is any other key.
        assert_eq!(
            events(&ev, Key::Named(Named::Escape), none),
            [SpellEvent::Key(MenuKey::Escape)]
        );
        assert_eq!(
            events(&ev, Key::Named(Named::Escape), Modifiers::CTRL),
            [SpellEvent::Close]
        );
        // Any other key closes the menu and types.
        let messages = ev.key_message(character("h"), none);
        assert!(
            matches!(
                messages.as_slice(),
                [
                    Message::Spell(SpellEvent::Close),
                    Message::Action(Action::Edit(Edit::Insert('h')))
                ]
            ),
            "{messages:?}"
        );

        // Suggestions leave typing to the text and send their keys on.
        let (ev, _) = {
            let (mut ev, source) = suggesting("", 0, 0);
            type_text(&mut ev, "du");
            (ev, source)
        };
        for (named, key) in [
            (Named::ArrowUp, MenuKey::Up),
            (Named::ArrowDown, MenuKey::Down),
            (Named::Enter, MenuKey::Enter),
            (Named::Tab, MenuKey::Tab),
            (Named::Escape, MenuKey::Escape),
        ] {
            assert_eq!(
                events(&ev, Key::Named(named), none),
                [SpellEvent::Key(key)],
                "{named:?}"
            );
        }
        let messages = ev.key_message(character("e"), none);
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Action(Action::Edit(Edit::Insert('e')))]
            ),
            "{messages:?}"
        );
        // The suggestions menu leaves Escape to the host with no word.
        let (ev, _) = opened();
        assert!(events(&ev, Key::Named(Named::Escape), none).is_empty());
    }

    // ── What the host's sources open, and Escape ────────────────────────────

    /// An owned-engine editor over a task line, the caret after `ca`, with
    /// the test source of menus.
    fn with_menus() -> (EditorView, Arc<fixtures::Menus>) {
        let menus = Arc::new(fixtures::Menus::default());
        let mut ev = EditorView::with_backend(BackendKind::Owned, "- [ ] call sam\nnext");
        ev.set_menu_source(Some(menus.clone()));
        ev.move_cursor(at(0, 8));
        (ev, menus)
    }

    #[test]
    fn a_request_opens_what_the_hosts_source_answers_for_the_caret() {
        let (mut ev, menus) = with_menus();
        let selection = select((0, 12), (0, 8));
        ev.move_cursor(selection);
        assert!(!ev.spell(SpellEvent::Request(fixtures::STATUS)));
        assert_eq!(
            menus.asked(),
            [(fixtures::STATUS, "- [ ] call sam".to_owned(), 8)]
        );
        let labels = strings(&["Open", "Doing", "Done"]);
        assert_eq!(
            popover_of(&ev),
            Some((Mode::Menu, 0, 2..5, "[ ]".into(), labels, Some(0)))
        );
        assert_eq!(ev.cursor(), selection);
        // A row chosen comes back as from any popover of the host's.
        ev.spell(SpellEvent::Accept(2));
        assert_eq!(ev.take_choice().map(|choice| choice.id), Some(22));

        // Whatever was open closes first, whichever order the host acts
        // in: a popover it opened itself before handing the request on is
        // replaced by what the source answers, or closed when that is
        // nothing.
        let (mut ev, _) = with_menus();
        assert!(ev.open_popover(Anchor::Caret, Mode::Menu, rows(&["mine"])));
        ev.spell(SpellEvent::Request(fixtures::STATUS));
        assert_eq!(ev.popover().map(|popover| popover.labels().len()), Some(3));
        for nothing in [9, fixtures::KEYBOARD] {
            ev.spell(SpellEvent::Request(nothing));
            assert!(ev.popover().is_none(), "{nothing}");
            assert!(ev.open_popover(Anchor::Caret, Mode::Menu, rows(&["mine"])));
        }
        let (mut ev, _) = opened();
        ev.spell(SpellEvent::Request(fixtures::STATUS));
        assert!(ev.spell_menu().is_none());
        assert_eq!((ev.text().as_str(), ev.cursor()), ("teh cat", at(0, 1)));

        // On a line that is no task the source opens nothing.
        let (mut ev, _) = with_menus();
        ev.move_cursor(at(1, 2));
        ev.spell(SpellEvent::Request(fixtures::STATUS));
        assert!(ev.popover().is_none());
        // Nor is it asked on the iced engine, or once it is taken away.
        let menus = Arc::new(fixtures::Menus::default());
        let mut ev = EditorView::with_backend(BackendKind::Iced, "- [ ] call");
        ev.set_menu_source(Some(menus.clone()));
        ev.spell(SpellEvent::Request(fixtures::STATUS));
        assert!(ev.popover().is_none() && menus.asked().is_empty());
        let (mut ev, menus) = with_menus();
        ev.set_menu_source(None);
        ev.spell(SpellEvent::Request(fixtures::STATUS));
        assert!(ev.popover().is_none() && menus.asked().is_empty());
    }

    #[test]
    fn a_requested_menu_scrolls_its_anchors_row_into_view() {
        let mut text: String = (0..60).map(|i| format!("line {i}\n")).collect();
        text.push_str("- [ ] the end");
        let mut ev = EditorView::with_backend(BackendKind::Owned, &text);
        ev.set_menu_source(Some(Arc::new(fixtures::Menus::default())));
        let size = iced::Size::new(400.0, 240.0);
        ev.content.lay_out(size);
        let top = ev.content.scroll();
        ev.content.return_to(at(60, 8), top);
        ev.content.lay_out(size);
        assert_eq!(ev.first_visible_line(), 0);
        ev.spell(SpellEvent::Request(fixtures::STATUS));
        assert!(ev.popover().is_some());
        ev.content.lay_out(size);
        assert_eq!(ev.content.scroll().line, 51, "{:?}", ev.content.scroll());
        assert_eq!(ev.cursor(), at(60, 8));
    }

    #[test]
    fn a_stroke_is_the_hosts_and_nothing_to_the_editor() {
        let press = key_press(
            iced::keyboard::Key::Character("x".into()),
            iced::keyboard::Modifiers::empty(),
        );
        let stroke = Stroke::of(&press).unwrap();
        let (mut ev, _) = with_menus();
        assert!(!ev.spell(SpellEvent::Stroke(stroke)));
        assert_eq!(
            (ev.text().as_str(), ev.cursor()),
            ("- [ ] call sam\nnext", at(0, 8))
        );
        // Nor does it close a popover.
        ev.spell(SpellEvent::Request(fixtures::STATUS));
        ev.spell(SpellEvent::Stroke(stroke));
        assert!(ev.popover().is_some());
        assert_eq!(ev.take_choice(), None);
    }

    #[test]
    fn escape_sent_to_a_popover_dismisses_it_and_is_claimed_once() {
        // Suggestions: hidden for the line, before what is typed next.
        let (mut ev, source) = suggesting("", 0, 0);
        type_text(&mut ev, "du");
        assert!(!ev.take_claimed_escape());
        assert!(!ev.spell(SpellEvent::Key(MenuKey::Escape)));
        assert!(ev.popover().is_none());
        assert_eq!(ev.suggest_suppressed(), Some(0));
        // The host's own handling of that press is dropped, once.
        assert!(ev.take_claimed_escape());
        assert!(!ev.take_claimed_escape());
        type_text(&mut ev, "e");
        assert!(ev.popover().is_none());
        assert_eq!(source.asked().len(), 2);

        // A menu of the host's: closed, and nothing hidden.
        let mut ev = with_menu();
        ev.spell(SpellEvent::Key(MenuKey::Escape));
        assert!(ev.popover().is_none());
        assert_eq!(ev.suggest_suppressed(), None);
        assert!(ev.take_claimed_escape());
        assert!(!ev.take_claimed_escape());

        // With nothing open the press is the host's Escape: nothing is
        // claimed, hidden or changed.
        let (mut ev, _) = suggesting("du", 0, 2);
        assert!(!ev.spell(SpellEvent::Key(MenuKey::Escape)));
        assert!(!ev.take_claimed_escape());
        assert_eq!(ev.suggest_suppressed(), None);
        assert_eq!((ev.text().as_str(), ev.cursor()), ("du", at(0, 2)));

        // Each press a popover took is dropped in turn; no other key is
        // ever claimed.
        let (mut ev, _) = suggesting("", 0, 0);
        for _ in 0..2 {
            ev.clear_suggest_suppression();
            type_text(&mut ev, "du");
            ev.spell(SpellEvent::Key(MenuKey::Escape));
            ev.intent(Intent::DeleteWordBackward);
        }
        type_text(&mut ev, "du");
        ev.spell(SpellEvent::Key(MenuKey::Tab));
        assert!(ev.take_claimed_escape());
        assert!(ev.take_claimed_escape());
        assert!(!ev.take_claimed_escape());
    }

    #[test]
    fn a_held_edit_asks_the_source_nothing() {
        let (mut ev, source) = suggesting("d", 0, 1);
        assert!(!ev.spell(SpellEvent::Hold));
        type_text(&mut ev, "u");
        assert!(ev.popover().is_none());
        assert!(source.asked().is_empty());
        // Only the one edit.
        type_text(&mut ev, "e");
        assert_eq!(suggested(&ev), Some((0..3, strings(&["due"]))));
        // A deletion is held the same way, and closes what was open.
        ev.spell(SpellEvent::Hold);
        ev.intent(Intent::Backspace);
        assert!(ev.popover().is_none());
        assert_eq!(source.asked().len(), 1);
        // Any other popover event lets go of it.
        ev.spell(SpellEvent::Hold);
        ev.spell(SpellEvent::Close);
        type_text(&mut ev, "e");
        assert!(ev.popover().is_some());
    }

    #[test]
    fn a_choice_does_not_outlive_the_text_it_was_made_over() {
        let steps: [(&str, Step); 5] = [
            ("a new note", |ev| ev.replace_text("- [x] another", false)),
            ("the same text loaded again", |ev| {
                ev.replace_text("- [ ] call sam\nnext", true)
            }),
            ("an edit", |ev| {
                ev.perform(Action::Edit(Edit::Insert('x')));
            }),
            ("undo", |ev| {
                ev.perform(Action::Edit(Edit::Insert('x')));
                ev.undo();
            }),
            ("another engine", |ev| {
                ev.set_backend(BackendKind::Iced, "- [ ] call sam\nnext");
            }),
        ];
        for (name, step) in steps {
            let mut ev = with_menu();
            ev.spell(SpellEvent::Accept(1));
            step(&mut ev);
            assert_eq!(ev.take_choice(), None, "{name}");
        }
        // What leaves the text alone leaves the choice.
        let mut ev = with_menu();
        ev.spell(SpellEvent::Accept(1));
        ev.perform(Action::Move(Motion::Right));
        ev.set_find_query("call");
        assert_eq!(ev.take_choice().map(|choice| choice.id), Some(11));
    }

    // ── A right-click's providers ───────────────────────────────────────────

    use fixtures::{on_dates, on_task_lines, on_tokens, Provides};

    /// A task with an owner whose name is misspelled, one whose name is
    /// not, a misspelled word, a tag and a due date; and a line that is no
    /// task, with the same misspelled word.
    const PROVIDED: &str = "- [ ] Mail @sam and @lee teh #home [due:: 2026-11-30]\nplain teh text";

    fn provider(answer: fn(&Clicked) -> Option<Requested>) -> Arc<Provides> {
        Arc::new(Provides::new(answer))
    }

    /// An editor on `kind` over [`PROVIDED`], laid out, the caret on its
    /// second line, whose host has tasks on, rejects `sam` and `teh`, and
    /// names `providers`, when it names any.
    fn providing(kind: BackendKind, providers: Option<Vec<Provider>>) -> EditorView {
        let mut ev = EditorView::with_backend(kind, PROVIDED);
        ev.set_tasks(Some(wren_markdown::TaskOptions {
            today: wren_markdown::date::Date::new(2026, 11, 19).expect("a date"),
        }));
        let proofreader = fixtures::Rejects::new(&["sam", "teh"]).suggesting(TEH);
        ev.set_proofreader(Some(Arc::new(proofreader)));
        if let Some(providers) = providers {
            ev.set_menu_providers(providers);
        }
        ev.move_cursor(at(1, 5));
        ev.content.lay_out(iced::Size::new(600.0, 400.0));
        ev
    }

    /// What a right-click on the character at `column` of `line` asks for.
    fn right_click(ev: &EditorView, line: usize, column: usize) -> Option<SpellEvent> {
        match ev.right_click_message(line, column) {
            Some(Message::Spell(event)) => Some(event),
            None => None,
            other => panic!("{other:?}"),
        }
    }

    /// A right-click at `at` that the provider at `provider` takes, as a
    /// widget publishes it that saw the editor as it is now.
    fn context(ev: &EditorView, provider: usize, at: iced::Point) -> SpellEvent {
        let clicked = ev.clicked(at);
        let answer = match (ev.providers().get(provider), &clicked) {
            (Some(Provider::Host(asked)), Some(clicked)) => {
                popover::provided(&**asked, provider, clicked)
            }
            _ => popover::Answer::Nothing,
        };
        let print = clicked.map_or(0, |clicked| {
            popover::fingerprint(provider, &clicked, &answer)
        });
        SpellEvent::Context {
            provider,
            at,
            seen: Seen::new(ev.edition(), print),
        }
    }

    /// The middle of the character at `column` of `line`, relative to the
    /// text origin.
    fn middle(ev: &EditorView, line: usize, column: usize) -> iced::Point {
        let row = ev.content.anchor_row(line, column..column + 1).unwrap();
        row.bounds().center()
    }

    /// The columns of [`PROVIDED`]'s first line a right-click is tried on.
    const MAIL: usize = 7;
    const SAM: usize = 13;
    const LEE: usize = 21;
    const TEH_WORD: usize = 26;
    const DATE: usize = 44;

    #[test]
    fn a_right_click_a_provider_takes_opens_its_menu_and_the_row_comes_back_with_it() {
        let (tokens, lines) = (provider(on_tokens), provider(on_task_lines));
        let providers = || {
            Some(vec![
                Provider::Spelling,
                Provider::Host(tokens.clone()),
                Provider::Host(lines.clone()),
            ])
        };
        let mut ev = providing(BackendKind::Owned, providers());
        let Some(
            event @ SpellEvent::Context {
                provider: 1, at, ..
            },
        ) = right_click(&ev, 0, LEE)
        else {
            panic!("{:?}", right_click(&ev, 0, LEE));
        };
        assert_eq!(at, middle(&ev, 0, LEE));
        assert_eq!(event, context(&ev, 1, at));
        tokens.asked.lock().expect("asked").clear();
        assert!(!ev.spell(event));
        let labels = strings(&["Rename", "Remove"]);
        assert_eq!(
            popover_of(&ev),
            Some((Mode::Menu, 0, 20..24, "@lee".into(), labels, Some(0)))
        );
        assert_eq!(ev.popover().and_then(Popover::provider), Some(1));
        assert!(ev.spell_menu().is_none());
        assert_eq!(ev.cursor(), at_caret());
        // The editor asked the provider the click names, once, about the
        // place the click was on, and the one after it never.
        let asked = tokens.asked();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert_eq!(Some(&asked[0]), ev.clicked(at).as_ref());
        assert!(lines.asked().is_empty());

        // The row chosen comes back with its provider, once, and not as a
        // row of a popover the host opened itself.
        assert!(!ev.spell(SpellEvent::Accept(1)));
        assert!(ev.popover().is_none());
        assert_eq!(ev.take_choice(), None);
        let row = Choice {
            id: 31,
            mode: Mode::Menu,
            line: 0,
            range: 20..24,
        };
        assert_eq!(ev.take_provided(), Some((1, row)));
        assert_eq!(ev.take_provided(), None);
        assert!(ev.open_popover(range(0, 2..5), Mode::Menu, rows(&["mine"])));
        assert_eq!(ev.popover().and_then(Popover::provider), None);
        ev.spell(SpellEvent::Accept(0));
        assert_eq!(ev.take_provided(), None);
        assert_eq!(ev.take_choice().map(|choice| choice.id), Some(10));
        // Nor does it outlive the text it was chosen over, or the event
        // after it.
        for after in [
            |ev: &mut EditorView| {
                ev.perform(Action::Edit(Edit::Insert('x')));
            },
            |ev: &mut EditorView| {
                ev.spell(SpellEvent::Close);
            },
        ] {
            ev = providing(BackendKind::Owned, providers());
            ev.spell(right_click(&ev, 0, LEE).unwrap());
            ev.spell(SpellEvent::AcceptSelected);
            after(&mut ev);
            assert_eq!(ev.take_provided(), None);
        }

        // Whatever was open closes first: suggestions the host had open
        // when the click was made give way to the menu.
        let mut ev = providing(BackendKind::Owned, providers());
        ev.popover = Some(Popover::new(
            Mode::Suggest,
            1,
            0..5,
            "plain".into(),
            rows(&["a"]),
        ));
        ev.spell(right_click(&ev, 0, LEE).unwrap());
        assert_eq!(ev.popover().and_then(Popover::provider), Some(1));
        // A popover opened after the click was made closes too, but the
        // click, which was made with none of it there, opens nothing.
        let mut ev = providing(BackendKind::Owned, providers());
        let event = right_click(&ev, 0, LEE).unwrap();
        ev.spell(SpellEvent::OpenAt(middle(&ev, 0, TEH_WORD)));
        assert!(ev.spell_menu().is_some());
        ev.spell(event);
        assert!(ev.popover().is_none());
        ev.spell(right_click(&ev, 0, LEE).unwrap());
        assert_eq!(ev.popover().and_then(Popover::provider), Some(1));
        let on_mail = middle(&ev, 0, MAIL);
        ev.spell(context(&ev, 1, on_mail));
        assert!(ev.popover().is_none());
        // Only the provider the click names is asked: the one after it,
        // which takes that place, does not answer for it.
        assert!(lines.asked().is_empty());
        ev.spell(context(&ev, 2, on_mail));
        assert_eq!(ev.popover().and_then(Popover::provider), Some(2));
        assert_eq!(ev.popover().map(Popover::range), Some(2..5));

        // A place in the list that holds no provider of the host's opens
        // nothing, the suggestions menu's included, which has its own
        // event; nor does a point off the text.
        let on_teh = middle(&ev, 0, TEH_WORD);
        for (provider, at) in [(0, on_teh), (3, on_teh), (2, iced::Point::new(10.0, 900.0))] {
            ev.close_popover();
            ev.spell(context(&ev, provider, at));
            assert!(ev.popover().is_none(), "{provider} {at:?}");
        }
    }

    #[test]
    fn a_right_click_opens_a_menu_only_over_what_the_widget_saw() {
        let (tokens, lines) = (provider(on_tokens), provider(on_task_lines));
        let providers = || {
            Some(vec![
                Provider::Spelling,
                Provider::Host(tokens.clone()),
                Provider::Host(lines.clone()),
            ])
        };
        // The click as a widget publishes it, and then what the host
        // does before it hands the click to the editor.
        type Step = fn(&mut EditorView);
        let steps: [(&str, Step, bool); 7] = [
            ("nothing", |_| {}, true),
            ("a caret motion", |ev| ev.move_cursor(at(0, 3)), true),
            (
                "a new note",
                |ev| ev.replace_text("- [ ] @ann #x", false),
                false,
            ),
            (
                "the same text loaded again",
                |ev| ev.replace_text(PROVIDED, true),
                false,
            ),
            (
                "an edit",
                |ev| {
                    ev.perform(Action::Edit(Edit::Insert('x')));
                },
                false,
            ),
            ("tasks turned off", |ev| ev.set_tasks(None), false),
            (
                "another engine",
                |ev| {
                    ev.set_backend(BackendKind::Iced, PROVIDED);
                },
                false,
            ),
        ];
        for (name, step, opens) in steps {
            let mut ev = providing(BackendKind::Owned, providers());
            let event = right_click(&ev, 0, LEE).unwrap();
            step(&mut ev);
            ev.content.lay_out(iced::Size::new(600.0, 400.0));
            let asked = tokens.asked().len();
            assert!(!ev.spell(event), "{name}");
            assert_eq!(ev.popover().is_some(), opens, "{name}");
            // Over another text the provider is not even asked.
            assert_eq!(tokens.asked().len(), asked + usize::from(opens), "{name}");
            // A row chosen by a key of the same batch chooses nothing.
            ev.spell(SpellEvent::Accept(1));
            assert_eq!(ev.take_provided().is_some(), opens, "{name}");
        }
        // A click bound to another place or another answer opens nothing:
        // here one on the tag, handed over as if it had been on the owner.
        let mut ev = providing(BackendKind::Owned, providers());
        let (Some(SpellEvent::Context { seen, .. }), on_lee) =
            (right_click(&ev, 0, 30), middle(&ev, 0, LEE))
        else {
            panic!("no click on the tag");
        };
        ev.spell(SpellEvent::Context {
            provider: 1,
            at: on_lee,
            seen,
        });
        assert!(ev.popover().is_none());
        // A click that closes what is open, whatever it finds.
        assert!(ev.open_popover(Anchor::Caret, Mode::Menu, rows(&["mine"])));
        ev.spell(SpellEvent::Context {
            provider: 1,
            at: on_lee,
            seen,
        });
        assert!(ev.popover().is_none());
    }

    #[test]
    fn naming_other_providers_drops_what_waited_on_the_old_ones() {
        let (tokens, dates) = (provider(on_tokens), provider(on_dates));
        let list = |second: &Arc<Provides>| {
            vec![
                Provider::Spelling,
                Provider::Host(tokens.clone()),
                Provider::Host(second.clone()),
            ]
        };
        // A row chosen from one provider's menu is not handed to whoever
        // has its place in another list.
        let mut ev = providing(BackendKind::Owned, Some(list(&dates)));
        ev.spell(right_click(&ev, 0, LEE).unwrap());
        ev.spell(SpellEvent::Accept(0));
        let replaced = provider(on_tokens);
        ev.set_menu_providers(vec![Provider::Spelling, Provider::Host(replaced.clone())]);
        assert_eq!(ev.take_provided(), None);
        // The same list named again keeps it.
        let mut ev = providing(BackendKind::Owned, Some(list(&dates)));
        ev.spell(right_click(&ev, 0, LEE).unwrap());
        ev.spell(SpellEvent::Accept(0));
        ev.set_menu_providers(list(&dates));
        assert_eq!(
            ev.take_provided().map(|(provider, row)| (provider, row.id)),
            Some((1, 30))
        );
        // Nor is the keyboard a provider took.
        let mut ev = providing(BackendKind::Owned, Some(list(&dates)));
        ev.spell(right_click(&ev, 0, DATE).unwrap());
        ev.set_menu_providers(list(&provider(on_dates)));
        assert_eq!(ev.take_keyboard(), None);
        // A click the widget published over the old list opens nothing,
        // and the provider that now has its place is not asked.
        let mut ev = providing(BackendKind::Owned, Some(list(&dates)));
        let click = right_click(&ev, 0, LEE).unwrap();
        ev.set_menu_providers(vec![Provider::Spelling, Provider::Host(replaced.clone())]);
        ev.spell(click);
        assert!(ev.popover().is_none() && replaced.asked().is_empty());
        // A row of a popover the host opened itself waits on no provider.
        let mut ev = providing(BackendKind::Owned, Some(list(&dates)));
        assert!(ev.open_popover(range(0, 2..5), Mode::Menu, rows(&["mine"])));
        ev.spell(SpellEvent::Accept(0));
        ev.set_menu_providers(Vec::new());
        assert_eq!(ev.take_choice().map(|choice| choice.id), Some(10));
    }

    #[test]
    fn a_right_press_left_to_the_editor_closes_a_menu_or_asks_for_the_suggestions_menu_alone() {
        let (tokens, lines, dates) = (
            provider(on_tokens),
            provider(on_task_lines),
            provider(on_dates),
        );
        let providers = || {
            Some(vec![
                Provider::Spelling,
                Provider::Host(tokens.clone()),
                Provider::Host(dates.clone()),
                Provider::Host(lines.clone()),
            ])
        };
        let press = |ev: &mut EditorView, line: usize, column: usize| {
            let at = middle(ev, line, column);
            assert!(!ev.spell(SpellEvent::Press(MenuPress::Right(at))));
            (
                ev.spell_menu().map(|menu| menu.word().to_owned()),
                ev.popover().and_then(Popover::provider),
            )
        };
        // The widget leaves a right press to the editor without asking
        // the host's providers, so none answers it, whatever it is on: it
        // opens the suggestions menu on a misspelled word, as it always
        // did, and with a menu open it only closes that.
        let mut ev = providing(BackendKind::Owned, providers());
        assert_eq!(press(&mut ev, 0, TEH_WORD), (Some("teh".into()), None));
        assert_eq!(press(&mut ev, 0, LEE), (None, None));
        for column in [LEE, MAIL, DATE] {
            assert_eq!(press(&mut ev, 0, column), (None, None), "{column}");
            assert_eq!((ev.take_keyboard(), ev.take_provided()), (None, None));
        }
        assert_eq!(press(&mut ev, 1, 2), (None, None));
        let asked = [&tokens, &lines, &dates].map(|provider| provider.asked().len());
        assert_eq!(asked, [0; 3]);
        // A provider's menu that is open closes for it, as any.
        ev.spell(right_click(&ev, 0, LEE).unwrap());
        assert_eq!(ev.popover().and_then(Popover::provider), Some(1));
        assert_eq!(press(&mut ev, 0, TEH_WORD), (None, None));

        // The suggestions menu answers only where no provider of the
        // host's comes before it, which would have to be asked first; a
        // host that left it out of its list gets none.
        for providers in [
            vec![Provider::Host(tokens.clone())],
            vec![Provider::Host(tokens.clone()), Provider::Spelling],
            Vec::new(),
        ] {
            let mut ev = providing(BackendKind::Owned, Some(providers));
            assert_eq!(press(&mut ev, 0, TEH_WORD), (None, None));
            assert_eq!(press(&mut ev, 0, LEE), (None, None));
        }
        // A host that names no providers has it alone, as it always had.
        let mut ev = providing(BackendKind::Owned, None);
        assert_eq!(press(&mut ev, 0, TEH_WORD), (Some("teh".into()), None));
        ev.close_popover();
        assert_eq!(press(&mut ev, 0, LEE), (None, None));
        // The classic engine opens nothing.
        let mut ev = providing(BackendKind::Iced, providers());
        for column in [TEH_WORD, LEE, MAIL] {
            assert_eq!(press(&mut ev, 0, column), (None, None), "{column}");
        }
    }

    #[test]
    fn a_row_named_with_its_menu_is_chosen_from_that_menu_or_from_none() {
        let tokens = provider(on_tokens);
        let providers = || Some(vec![Provider::Spelling, Provider::Host(tokens.clone())]);
        // The provider's menu, open, and its number.
        let opened = || {
            let mut ev = providing(BackendKind::Owned, providers());
            ev.spell(right_click(&ev, 0, LEE).unwrap());
            let menu = ev.popover().unwrap().instance();
            (ev, menu)
        };
        let chosen = |ev: &mut EditorView, menu: u64, row: Option<usize>| {
            assert!(!ev.spell(SpellEvent::AcceptOf { menu, row }));
            let provided = ev.take_provided().map(|(provider, row)| (provider, row.id));
            (provided, ev.take_choice().map(|row| row.id))
        };
        // A row, or the selected row, of the menu that is open.
        let (mut ev, menu) = opened();
        assert_eq!(chosen(&mut ev, menu, Some(1)), (Some((1, 31)), None));
        assert!(ev.popover().is_none());
        let (mut ev, menu) = opened();
        ev.spell(SpellEvent::Move(1));
        assert_eq!(chosen(&mut ev, menu, None), (Some((1, 31)), None));
        // A choice that names another menu is no one's: the menu that is
        // open stays as it is.
        let (mut ev, menu) = opened();
        for other in [menu + 1, menu - 1, 0] {
            assert_eq!(chosen(&mut ev, other, Some(1)), (None, None), "{other}");
            assert_eq!(ev.popover().map(Popover::instance), Some(menu), "{other}");
        }
        // The host opens a menu of its own between the click and the row
        // chosen after it: the row was not chosen from that menu, though
        // it has a row there, and nothing is chosen.
        let (mut ev, menu) = opened();
        assert!(ev.open_popover(range(0, 2..5), Mode::Menu, rows(&["mine", "also mine"])));
        let hosts = ev.popover().unwrap().instance();
        assert_ne!(hosts, menu);
        for row in [Some(1), None] {
            assert_eq!(chosen(&mut ev, menu, row), (None, None), "{row:?}");
            assert_eq!(ev.popover().map(Popover::instance), Some(hosts), "{row:?}");
        }
        // So with the same menu opened again, by another click on the
        // same place, and with nothing open.
        let (mut ev, menu) = opened();
        ev.close_popover();
        ev.spell(right_click(&ev, 0, LEE).unwrap());
        let again = ev.popover().unwrap().instance();
        assert_eq!(chosen(&mut ev, menu, Some(1)), (None, None));
        assert_eq!(ev.popover().map(Popover::instance), Some(again));
        ev.close_popover();
        assert_eq!(chosen(&mut ev, again, Some(1)), (None, None));

        // Every popover the editor opens has its own number, whoever
        // opens it: the suggestions menu, the host, a source, a provider.
        let mut ev = providing(BackendKind::Owned, providers());
        ev.set_menu_source(Some(Arc::new(fixtures::Menus::default())));
        let mut numbers = Vec::new();
        ev.spell(SpellEvent::OpenAt(middle(&ev, 0, TEH_WORD)));
        numbers.push(ev.popover().unwrap().instance());
        assert!(ev.open_popover(Anchor::Caret, Mode::Menu, rows(&["mine"])));
        numbers.push(ev.popover().unwrap().instance());
        ev.move_cursor(at(0, 8));
        ev.spell(SpellEvent::Request(fixtures::STATUS));
        numbers.push(ev.popover().unwrap().instance());
        ev.close_popover();
        ev.spell(right_click(&ev, 0, LEE).unwrap());
        numbers.push(ev.popover().unwrap().instance());
        assert!(
            numbers.windows(2).all(|pair| pair[0] < pair[1]),
            "{numbers:?}"
        );
    }

    #[test]
    fn a_right_click_without_a_window_only_closes_an_open_menu() {
        let tokens = provider(on_tokens);
        let providers = || Some(vec![Provider::Spelling, Provider::Host(tokens.clone())]);
        let closes = |ev: &EditorView, line: usize, column: usize| {
            matches!(
                ev.right_click_message(line, column),
                Some(Message::Spell(SpellEvent::Close))
            )
        };
        // The suggestions menu is open: a press anywhere in the text,
        // where a provider would answer or where none would, closes it
        // and does no more, as the pointer's press does (rule 4.4.5).
        let mut ev = providing(BackendKind::Owned, providers());
        ev.spell(SpellEvent::OpenAt(middle(&ev, 0, TEH_WORD)));
        assert!(ev.spell_menu().is_some());
        for (line, column) in [
            (1, 2),
            (0, MAIL),
            (0, LEE),
            (0, TEH_WORD),
            (0, PROVIDED.len() - 15),
        ] {
            assert!(closes(&ev, line, column), "{line} {column}");
        }
        ev.spell(SpellEvent::Close);
        assert!(ev.popover().is_none());
        // So it is with a menu of the host's, a provider's included.
        ev.spell(right_click(&ev, 0, LEE).unwrap());
        assert_eq!(ev.popover().and_then(Popover::provider), Some(1));
        assert!(closes(&ev, 1, 2) && closes(&ev, 0, LEE));
        // A place that is not in the text is no press at all.
        assert_eq!(right_click(&ev, 9, 0), None);
        assert_eq!(right_click(&ev, 1, 99), None);
        // Suggestions leave presses to the text: the providers are asked.
        ev.close_popover();
        ev.popover = Some(Popover::new(
            Mode::Suggest,
            1,
            0..5,
            "plain".into(),
            rows(&["a"]),
        ));
        assert_eq!(right_click(&ev, 1, 2), None);
        assert!(matches!(
            right_click(&ev, 0, LEE),
            Some(SpellEvent::Context { provider: 1, .. })
        ));
    }

    fn at_caret() -> Cursor {
        at(1, 5)
    }

    #[test]
    fn a_provider_that_takes_the_keyboard_opens_nothing_of_the_editors() {
        let dates = provider(on_dates);
        let mut ev = providing(
            BackendKind::Owned,
            Some(vec![Provider::Spelling, Provider::Host(dates.clone())]),
        );
        let Some(
            event @ SpellEvent::Context {
                provider: 1, at, ..
            },
        ) = right_click(&ev, 0, DATE)
        else {
            panic!("{:?}", right_click(&ev, 0, DATE));
        };
        assert!(!ev.spell(event));
        assert!(ev.popover().is_none());
        assert_eq!((ev.text().as_str(), ev.cursor()), (PROVIDED, at_caret()));
        // The host is handed the click, once, with the place clicked, for
        // what it opens there.
        let (provider, clicked) = ev.take_keyboard().unwrap();
        assert_eq!((provider, clicked.line, clicked.column), (1, 0, Some(DATE)));
        assert_eq!(Some(&clicked), ev.clicked(at).as_ref());
        assert_eq!(on_dates(&clicked), Some(Requested::Keyboard));
        assert_eq!(ev.take_keyboard(), None);
        // It does not outlive the next event, or the text.
        ev.spell(event);
        ev.spell(SpellEvent::Close);
        assert_eq!(ev.take_keyboard(), None);
        ev.spell(event);
        ev.perform(Action::Edit(Edit::Insert('x')));
        assert_eq!(ev.take_keyboard(), None);
        let ev = providing(
            BackendKind::Owned,
            Some(vec![Provider::Spelling, Provider::Host(dates.clone())]),
        );
        // Elsewhere on the line the provider passes, and no one answers.
        assert_eq!(right_click(&ev, 0, MAIL), None);
    }

    #[test]
    fn the_suggestions_menu_alone_answers_a_right_click_until_the_host_names_providers() {
        // A host that names no providers has the suggestions menu, as an
        // editor always had.
        let ev = providing(BackendKind::Owned, None);
        for column in [SAM, TEH_WORD] {
            let at = middle(&ev, 0, column);
            assert_eq!(right_click(&ev, 0, column), Some(SpellEvent::OpenAt(at)));
        }
        for column in [MAIL, LEE, DATE] {
            assert_eq!(right_click(&ev, 0, column), None, "{column}");
        }
        // A list without it opens none for a right-click; the menu's own
        // events open it as before.
        let mut ev = providing(BackendKind::Owned, Some(Vec::new()));
        assert_eq!(right_click(&ev, 0, TEH_WORD), None);
        ev.spell(SpellEvent::OpenAt(middle(&ev, 0, TEH_WORD)));
        assert_eq!(ev.spell_menu().map(SpellMenu::word), Some("teh"));

        // Naming the same providers again leaves a provider's menu open,
        // and naming others closes it, which no other popover minds.
        let (tokens, lines) = (provider(on_tokens), provider(on_task_lines));
        let list = |ev: &mut EditorView, swapped: bool| {
            let mut providers = vec![
                Provider::Host(tokens.clone()),
                Provider::Host(lines.clone()),
            ];
            if swapped {
                providers.reverse();
            }
            providers.insert(0, Provider::Spelling);
            ev.set_menu_providers(providers);
        };
        let mut ev = providing(BackendKind::Owned, None);
        list(&mut ev, false);
        let event = right_click(&ev, 0, LEE).unwrap();
        ev.spell(event);
        assert_eq!(ev.popover().and_then(Popover::provider), Some(1));
        list(&mut ev, false);
        assert_eq!(ev.popover().and_then(Popover::provider), Some(1));
        list(&mut ev, true);
        assert!(ev.popover().is_none());
        // In the new order the other provider is first, and takes it.
        assert!(matches!(
            right_click(&ev, 0, LEE),
            Some(SpellEvent::Context { provider: 1, .. })
        ));
        ev.spell(right_click(&ev, 0, LEE).unwrap());
        assert_eq!(ev.popover().map(Popover::range), Some(2..5));
        ev.close_popover();
        ev.spell(SpellEvent::OpenAt(middle(&ev, 0, TEH_WORD)));
        list(&mut ev, false);
        assert!(ev.spell_menu().is_some());
        assert!(ev.open_popover(Anchor::Caret, Mode::Menu, rows(&["mine"])));
        list(&mut ev, true);
        assert!(ev.popover().is_some());
    }

    #[test]
    fn the_classic_engine_asks_no_provider_and_reads_the_same_place() {
        use wren_markdown::span::SpanKind;
        let tokens = provider(on_tokens);
        let providers = || Some(vec![Provider::Spelling, Provider::Host(tokens.clone())]);
        let mut ev = providing(BackendKind::Iced, providers());
        for column in [MAIL, SAM, LEE, TEH_WORD, DATE] {
            assert_eq!(right_click(&ev, 0, column), None, "{column}");
        }
        let at = middle(&ev, 0, LEE);
        ev.spell(context(&ev, 1, at));
        assert!(ev.popover().is_none() && tokens.asked().is_empty());
        // What lies under a point is read on both engines; only the owned
        // one has misspellings, so there a misspelled owner is its name.
        for kind in BackendKind::ALL {
            let ev = providing(kind, providers());
            let clicked = ev.clicked(middle(&ev, 0, SAM)).unwrap();
            assert_eq!((clicked.line, clicked.column), (0, Some(SAM)), "{kind:?}");
            assert_eq!(clicked.text, PROVIDED.lines().next().unwrap(), "{kind:?}");
            assert_eq!(clicked.spans, ev.spans_at(0), "{kind:?}");
            match (kind, &clicked.hit) {
                (BackendKind::Owned, Some(Hit::Misspelling(word))) => assert_eq!(*word, 12..15),
                (BackendKind::Iced, Some(Hit::Span(span))) => {
                    assert!(matches!(span.kind, SpanKind::Owner { .. }) && span.range == (11..15));
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(ev.clicked(iced::Point::new(10.0, 900.0)), None, "{kind:?}");
        }
    }

    /// A row that runs a command comes back only as one, with the anchor
    /// its menu was opened for, from a menu of the host's or of a
    /// provider; any other row never does, whatever its id.
    #[test]
    fn a_command_row_comes_back_as_a_command_and_no_other_row_does() {
        for id in [0, 7, 0x8000_0000, u32::MAX] {
            let mut ev = EditorView::with_backend(BackendKind::Owned, "- [ ] call sam\nnext");
            ev.move_cursor(at(0, 8));
            let rows = vec![Row::new(id, "plain"), Row::command(id, "command")];
            assert!(ev.open_popover(range(0, 2..5), Mode::Menu, rows.clone()));
            assert_eq!(
                ev.popover().and_then(|open| open.kind(1)),
                Some(RowKind::Command)
            );
            assert!(!ev.spell(SpellEvent::Accept(0)));
            assert_eq!(ev.take_command(), None, "{id}");
            assert_eq!(ev.take_choice().map(|choice| choice.id), Some(id));
            assert!(ev.open_popover(range(0, 2..5), Mode::Menu, rows));
            assert!(!ev.spell(SpellEvent::Accept(1)));
            assert_eq!(ev.take_choice(), None, "{id}");
            assert_eq!(ev.take_provided(), None, "{id}");
            let command = Choice {
                id,
                mode: Mode::Menu,
                line: 0,
                range: 2..5,
            };
            assert_eq!(ev.take_command(), Some(command));
            assert_eq!(ev.take_command(), None);
        }

        // A provider's: the command comes back with the clicked line, not
        // the caret's, and its own row goes back to it.
        let commands = provider(fixtures::commands_on_task_lines);
        let mut ev = providing(BackendKind::Owned, Some(vec![Provider::Host(commands)]));
        assert_eq!(ev.cursor().position.line, 1);
        for (row, provided, command) in [(0, Some(11), None), (1, None, Some(7))] {
            let event = right_click(&ev, 0, MAIL).expect("the provider takes the click");
            assert!(!ev.spell(event));
            assert_eq!(ev.popover().and_then(Popover::provider), Some(0));
            assert!(!ev.spell(SpellEvent::Accept(row)));
            assert_eq!(ev.take_choice(), None);
            let anchored = |choice: Choice| (choice.id, choice.line, choice.range);
            assert_eq!(
                ev.take_provided().map(|(_, choice)| choice.id),
                provided,
                "row {row}"
            );
            assert_eq!(
                ev.take_command().map(anchored),
                command.map(|id| (id, 0, 0..1))
            );
        }
    }

    /// Enter and Tab on suggestions with rows that run commands name the
    /// suggestions they were pressed on: queued while those are replaced
    /// by others, they choose nothing; and a key that names none never
    /// chooses such a row.
    #[test]
    fn a_queued_key_on_command_suggestions_chooses_only_from_them() {
        use iced::keyboard::key::Named;
        use iced::keyboard::{Key, Modifiers};

        let suggesting = || {
            let mut ev = EditorView::with_backend(BackendKind::Owned, "");
            ev.set_suggest_source(Some(Arc::new(fixtures::CommandSuggestions)));
            for c in ['d', 'u'] {
                ev.perform(Action::Edit(Edit::Insert(c)));
            }
            let open = ev.popover().expect("suggestions for `du`");
            assert_eq!((open.mode(), open.runs_commands()), (Mode::Suggest, true));
            ev
        };
        for key in [Named::Enter, Named::Tab] {
            // Chosen from the suggestions it was pressed on.
            let mut ev = suggesting();
            let a = ev.popover().unwrap().instance();
            let (messages, captured) = ev.key_routing(Key::Named(key), Modifiers::empty());
            assert!(captured);
            assert!(
                matches!(
                    messages.as_slice(),
                    [Message::Spell(SpellEvent::AcceptOf { menu, row: None })] if *menu == a
                ),
                "{key:?}: {messages:?}"
            );
            for message in messages {
                ev.carry_out(message);
            }
            assert_eq!(
                ev.take_command().map(|choice| choice.id),
                Some(7),
                "{key:?}"
            );
            assert_eq!(ev.text(), "du");

            // Queued on A, applied once B has replaced A: nothing is
            // chosen, from B either, and B stays open.
            let mut ev = suggesting();
            let (messages, _) = ev.key_routing(Key::Named(key), Modifiers::empty());
            let rows = vec![Row::command(9, "nine")];
            assert!(ev.open_popover(Anchor::Caret, Mode::Suggest, rows));
            let b = ev.popover().unwrap().instance();
            for message in messages {
                assert!(!ev.carry_out(message));
            }
            assert_eq!(ev.take_command(), None, "{key:?}");
            assert_eq!(ev.take_choice(), None, "{key:?}");
            assert_eq!(ev.popover().map(Popover::instance), Some(b), "{key:?}");
            assert_eq!(ev.text(), "du", "{key:?}");

            // A key that names no suggestions chooses no row that runs a
            // command: they close and nothing is chosen.
            let mut ev = suggesting();
            let menu_key = if key == Named::Enter {
                MenuKey::Enter
            } else {
                MenuKey::Tab
            };
            assert!(!ev.spell(SpellEvent::Key(menu_key)));
            assert_eq!(ev.take_command(), None, "{key:?}");
            assert!(ev.popover().is_none(), "{key:?}");
            assert_eq!(ev.text(), "du", "{key:?}");
        }
        // Plain suggestions' keys are unchanged: Enter is `Key`, not named.
        let mut ev = EditorView::with_backend(BackendKind::Owned, "");
        ev.set_suggest_source(Some(Arc::new(fixtures::Completes::new(&["due"]))));
        for c in ['d', 'u'] {
            ev.perform(Action::Edit(Edit::Insert(c)));
        }
        let (messages, _) = ev.key_routing(Key::Named(Named::Enter), Modifiers::empty());
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Spell(SpellEvent::Key(MenuKey::Enter))]
            ),
            "{messages:?}"
        );
    }
}
