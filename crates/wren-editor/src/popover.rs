//! The anchored popover: a short list of rows hung under a range of a line
//! or under the caret. The editor owns where it goes, which keys it takes,
//! what closes it, and how keys and presses are routed that arrive in the
//! batch of events that opens it; the host supplies the rows and acts on
//! the one chosen. The suggestions menu ([`crate::SpellMenu`]) is its first
//! user, and its events ([`PopoverEvent`]) keep that menu's names.
//!
//! A popover is open in one of two [`Mode`]s.
//!
//! **Menu** takes the keyboard. Up and Down move the selection, stopping at
//! the ends, and Enter chooses the selected row; a row's [`Accelerator`]
//! chooses it directly. Escape closes it and the editor keeps the keyboard.
//! Any other key closes it and is then handled as if it had not been open,
//! and a press in the editor only closes it.
//!
//! **Suggest** leaves the keyboard with the text. Up, Down, Enter and Tab
//! are the popover's, whatever the host has bound them to; Escape closes it
//! and hides suggestions for the caret's line; every other key and every
//! press is the editor's, as with no popover. After a character is typed
//! or deleted the host's [`SuggestSource`] is asked again, so the popover
//! follows the typing; whatever else the key or press does closes it.
//!
//! While an input method composes, no key is a popover's, of either mode,
//! and the source is not asked.
//!
//! Both close on anything else that moves the caret, scrolls a whole line,
//! or changes the text, and when the editor or the window loses the
//! keyboard, the pointer is pressed outside the editor, an input method
//! starts to compose, or the editor's size or look changes. Closing never
//! moves the caret or the selection.
//!
//! **Events of one batch.** iced hands a widget every event queued since
//! the last frame before the host applies any message the widget published,
//! so a key can arrive after the message that opens a popover and before
//! the host has opened it. The widget does not guess what the host will
//! open: it opens the popover itself, in the same pass, by asking what the
//! host will ask.
//!
//! - A key the host bound to a popover of its own ([`KeyRoute::Popover`])
//!   asks the host's [`MenuSource`] for the rows, with the caret's line as
//!   the batch's earlier keys leave it. The host, handed
//!   [`PopoverEvent::Request`], asks the same source and opens the same
//!   menu, so the keys after the request are routed against its rows, as
//!   in any later batch. A source may answer that the host takes the
//!   keyboard for something of its own ([`Requested::Keyboard`]): the keys
//!   after the request are then the host's, handed over as
//!   [`PopoverEvent::Stroke`].
//! - A character typed asks the host's [`SuggestSource`] the same way, and
//!   the keys after it find the suggestions open, or not.
//!
//! The widget knows the caret's line while the batch has only typed
//! characters at the caret. After anything else that changes the text or
//! moves the caret (a deletion, a motion, a click, a chosen row), it knows
//! neither, and until the batch ends it treats nothing as open and opens
//! nothing: a popover's keys are then the text's, and the host closes
//! whatever it opened for an earlier message when they act. A popover the
//! user has not been shown, and the widget cannot vouch for, takes no key.
//!
//! The suggestions menu is the exception, kept as it was built: after a
//! request for it (its key, or a right-click), Up, Down and Enter go to
//! the host as [`PopoverEvent::Key`] and a press in the editor as
//! [`PopoverEvent::Press`], which the host resolves against the menu it
//! opened, or handles as the editor would with none.
//!
//! **Escape** belongs to whoever has the keyboard when it is pressed, with
//! no Ctrl, Alt or logo key held, which is the Escape a host navigates
//! with; with one of them held it is any other key. An open menu of the
//! host's and open suggestions take it: the widget publishes it in its
//! order ([`PopoverEvent::Key`]) and leaves the press uncaptured, and the
//! host, whose own key handling hears of the same press, drops that one
//! ([`crate::EditorView::take_claimed_escape`]). With nothing open it is
//! the host's alone. The suggestions menu leaves Escape to the host's key
//! handling entirely, as it always has.
//!
//! The popover's overlay sees a batch before the widget does. It takes the
//! pointer over its rows until an event of the batch ends or changes what
//! is open, as the widget will find when it routes that event, and leaves
//! the presses after it to the widget, which sees them after it. Up and
//! Down leave the rows as they are drawn, and a press on a row after them
//! still chooses that row.
//!
//! [`KeyRoute::Popover`]: crate::KeyRoute::Popover

pub(crate) mod menu;

use std::cell::{Cell, RefCell};
use std::ops::Range;

use iced::advanced::mouse::click::Kind;
use iced::keyboard::key::{Named, Physical};
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor::{Action, Edit, KeyPress};
use iced::Point;

use crate::Message;

pub(crate) use menu::{fit, height, measure, place, Fit, InOrder, MenuOverlay};

/// Where a popover hangs: under the first row that holds its anchor, at the
/// anchor's left edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// Byte `range` of `line`. An empty range is a place between two
    /// characters.
    Range { line: usize, range: Range<usize> },
    /// The caret, where it is when the popover opens.
    Caret,
}

/// How an open popover shares the keyboard with the text; see the module's
/// description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The popover takes the keyboard until it closes.
    Menu,
    /// The text keeps the keyboard, and the popover follows the typing.
    Suggest,
}

/// A key that chooses a row of a menu directly: one key, alone or with
/// Shift. Letters match in either case and by the key's position on a
/// keyboard of another script; Ctrl, Alt and the logo key never match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accelerator {
    key: char,
    shift: bool,
}

impl Accelerator {
    /// `key` pressed alone; a space is the space bar.
    pub const fn plain(key: char) -> Self {
        Self { key, shift: false }
    }

    /// `key` pressed with Shift, named as the key reads without it.
    pub const fn shifted(key: char) -> Self {
        Self { key, shift: true }
    }

    /// Whether a press of `key` with `modifiers` is this accelerator.
    pub(crate) fn matches(self, key: &Key, physical: Physical, modifiers: Modifiers) -> bool {
        if modifiers.control() || modifiers.alt() || modifiers.logo() {
            return false;
        }
        let pressed = match key.as_ref() {
            Key::Named(Named::Space) => Some(' '),
            _ => key.to_latin(physical),
        };
        modifiers.shift() == self.shift
            && pressed.is_some_and(|pressed| pressed.to_lowercase().eq(self.key.to_lowercase()))
    }
}

/// One row of a popover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The host's name for the row, handed back when it is chosen.
    pub id: u32,
    pub label: String,
    /// The key that chooses the row while a menu is open.
    pub accelerator: Option<Accelerator>,
}

impl Row {
    pub fn new(id: u32, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            accelerator: None,
        }
    }

    pub fn accelerator(mut self, accelerator: Accelerator) -> Self {
        self.accelerator = Some(accelerator);
        self
    }
}

/// An open popover: where it hangs and what it offers. The host opens one
/// with [`crate::EditorView::open_popover`], or through its
/// [`SuggestSource`].
#[derive(Debug, Clone, PartialEq)]
pub struct Popover {
    mode: Mode,
    line: usize,
    range: Range<usize>,
    /// The anchor's text as it was when the popover opened.
    text: String,
    labels: Vec<String>,
    ids: Vec<u32>,
    accelerators: Vec<Option<Accelerator>>,
    selected: Option<usize>,
    /// The suggestions menu, whose word is tinted while it is open and
    /// which leaves Escape to the host's key handling.
    spelling: bool,
}

impl Popover {
    /// A popover under `range` of `line`, which reads `text`, offering
    /// `rows`, the first selected.
    pub(crate) fn new(
        mode: Mode,
        line: usize,
        range: Range<usize>,
        text: String,
        rows: Vec<Row>,
    ) -> Self {
        let mut popover = Self {
            mode,
            line,
            range,
            text,
            labels: Vec::with_capacity(rows.len()),
            ids: Vec::with_capacity(rows.len()),
            accelerators: Vec::with_capacity(rows.len()),
            selected: (!rows.is_empty()).then_some(0),
            spelling: false,
        };
        for row in rows {
            popover.labels.push(row.label);
            popover.ids.push(row.id);
            popover.accelerators.push(row.accelerator);
        }
        popover
    }

    /// The suggestions menu.
    pub(crate) fn spelling(mut self) -> Self {
        self.spelling = true;
        self
    }

    pub(crate) fn is_spelling(&self) -> bool {
        self.spelling
    }

    /// Move the selection `delta` rows, stopping at the ends.
    pub(crate) fn step(&mut self, delta: i32) {
        self.selected = stepped(self.selected, self.labels.len(), delta);
    }

    /// Select `row`, if the popover has it.
    pub(crate) fn hover(&mut self, row: usize) {
        self.selected = hovered(self.selected, self.labels.len(), row);
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The line the popover is anchored to.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The anchor's byte range in its line.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// The anchor's text as it was when the popover opened.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The rows' text, in order; a popover with nothing to offer has none.
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// The selected row, if any row can be selected.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The host's name for `row`.
    pub fn id(&self, row: usize) -> Option<u32> {
        self.ids.get(row).copied()
    }

    /// Each row's accelerator.
    pub(crate) fn accelerators(&self) -> &[Option<Accelerator>] {
        &self.accelerators
    }

    /// The anchor, when its text is tinted: the suggestions menu's word
    /// (rule 4.4.2).
    pub(crate) fn tint(&self) -> Option<(usize, Range<usize>)> {
        self.spelling.then(|| (self.line, self.range.clone()))
    }
}

/// The row a user chose from the host's popover; see
/// [`crate::EditorView::take_choice`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The row's [`Row::id`].
    pub id: u32,
    /// The mode of the popover it was chosen from.
    pub mode: Mode,
    /// The popover's anchor, whose text still reads as it did when the
    /// popover opened.
    pub line: usize,
    pub range: Range<usize>,
}

/// What the host gives the editor to offer suggestions as the user types.
/// It must answer the same for the same line and caret: the widget asks it
/// while it routes a batch of events, and the editor asks it again when
/// the host applies the edit.
pub trait SuggestSource: Send + Sync {
    /// What to offer now that a character was typed or deleted and the
    /// caret, with nothing selected, is at byte `caret` of `line`. `None`
    /// offers nothing, which closes the open suggestions.
    fn suggest(&self, line: &str, caret: usize) -> Option<Suggestion>;
}

/// A [`SuggestSource`]'s answer: rows, and the bytes of the line they
/// stand for, which the popover hangs under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub range: Range<usize>,
    pub rows: Vec<Row>,
}

/// What the host gives the editor to open the popovers it names by number
/// ([`PopoverEvent::Request`]). It must answer the same for the same
/// request, line and caret, for the reason a [`SuggestSource`] must.
pub trait MenuSource: Send + Sync {
    /// What request `id` opens with the caret at byte `caret` of `line`.
    /// `None` opens nothing.
    fn menu(&self, id: u32, line: &str, caret: usize) -> Option<Requested>;
}

/// A [`MenuSource`]'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requested {
    /// A menu of these rows, hung under `range` of the caret's line.
    Rows { range: Range<usize>, rows: Vec<Row> },
    /// Something of the host's own that takes the keyboard, a field say,
    /// which the host opens when it is handed the request. The keys
    /// pressed after the request in its batch of events are the host's:
    /// each goes to it as a [`PopoverEvent::Stroke`].
    Keyboard,
}

/// The caret's line: what a source is asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaretLine {
    pub index: usize,
    pub text: String,
    /// The caret's byte in the line.
    pub caret: usize,
    /// Whether text is selected.
    pub selected: bool,
}

/// The suggestions `source` offers at the caret of `line`: its rows, under
/// the range it names. None with a selection, or for an answer with no
/// rows or a range off the line or inside a character. The widget and the
/// editor both ask through here, so they open the same popover.
pub(crate) fn suggested(source: &dyn SuggestSource, line: &CaretLine) -> Option<Popover> {
    if line.selected {
        return None;
    }
    let suggestion = source.suggest(&line.text, line.caret)?;
    let anchored = line.text.get(suggestion.range.clone())?.to_owned();
    (!suggestion.rows.is_empty()).then(|| {
        Popover::new(
            Mode::Suggest,
            line.index,
            suggestion.range,
            anchored,
            suggestion.rows,
        )
    })
}

/// What a request opens; see [`requested`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Answer {
    Menu(Popover),
    /// The host takes the keyboard.
    Keyboard,
    Nothing,
}

/// What `source` opens for request `id` at the caret of `line`: a menu of
/// its rows, unless it has none or its range is off the line or inside a
/// character. The widget and the editor both ask through here.
pub(crate) fn requested(source: &dyn MenuSource, id: u32, line: &CaretLine) -> Answer {
    match source.menu(id, &line.text, line.caret) {
        None => Answer::Nothing,
        Some(Requested::Keyboard) => Answer::Keyboard,
        Some(Requested::Rows { range, rows }) => line
            .text
            .get(range.clone())
            .filter(|_| !rows.is_empty())
            .map_or(Answer::Nothing, |anchored| {
                let anchored = anchored.to_owned();
                Answer::Menu(Popover::new(Mode::Menu, line.index, range, anchored, rows))
            }),
    }
}

/// An input to the open popover, or a request to open the suggestions
/// menu; see [`crate::EditorView::spell`]. Positions are relative to the
/// text origin, as a click action's position is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PopoverEvent {
    /// Open the suggestions menu for the misspelled word at the caret.
    OpenAtCaret,
    /// Open the suggestions menu for the misspelled word under a
    /// right-click.
    OpenAt(Point),
    /// Move the selection by this many rows.
    Move(i32),
    /// The pointer is over this row.
    Hover(usize),
    /// Choose this row: the suggestions menu replaces its word with the
    /// row's suggestion, and the host's popover hands the row to the host.
    Accept(usize),
    /// Choose the selected row, or close a popover with nothing selected.
    /// Enter sends it, so it chooses the row the host's popover holds even
    /// when keys of one batch were routed before the host had opened it.
    AcceptSelected,
    /// A press of one of a popover's keys with no modifier: of
    /// suggestions' while they are open, or of the suggestions menu's after
    /// a request for it earlier in the same batch of events, which may not
    /// have opened it. An open popover takes the key as its mode does;
    /// with none open the editor handles the press as it would any other.
    Key(MenuKey),
    /// A key pressed after a request, earlier in the same batch, that the
    /// host answers by taking the keyboard ([`Requested::Keyboard`]): the
    /// key and what it types, for the host to give to what it opened. The
    /// editor does nothing with it.
    Stroke(Stroke),
    /// Open the popover the host names with this number: whatever popover
    /// was open closes, and the host's [`MenuSource`] is asked what to
    /// open at the caret.
    Request(u32),
    /// The edit after this event comes from a key pressed while an input
    /// method composes: the source of suggestions is not asked about it.
    Hold,
    /// Close the open popover as Escape does; see
    /// [`crate::EditorView::dismiss_popover`].
    Dismiss,
    /// A press of the left or right button inside the editor, made after a
    /// menu's opening earlier in the same batch, for the same reason as
    /// [`PopoverEvent::Key`], or a left press after such a press in the
    /// same batch, which may have only closed a menu. An open popover only
    /// closes (rule 4.4.5); with none open the editor handles the press as
    /// it would any other.
    Press(MenuPress),
    Close,
}

/// A pointer press the host resolves against its popover; see
/// [`PopoverEvent::Press`]. Positions are relative to the text origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuPress {
    /// A left press, with the kind of click it makes if each press before
    /// it in its batch that the host resolves is a click. The host takes it
    /// as a single click when the press it follows only closed the menu.
    Left(Point, Kind),
    /// A right press, which opens the menu on a misspelled word.
    Right(Point),
}

/// How the host resolved the left presses the widget left to it
/// ([`PopoverEvent::Press`]), which the widget learns only in a later
/// batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ResolvedPress {
    /// How many such presses the host has resolved, so the widget takes
    /// each verdict once.
    pub count: u64,
    /// What the last one was taken as: a click of this kind, or `None` when
    /// it only closed a menu.
    pub kind: Option<Kind>,
}

/// The click a left press makes that iced counts as a click of kind `raw`,
/// after a press taken as `previous`. A press that only closed a menu
/// (`previous` is `None`) is never the first half of a double or triple
/// click, so the press after it is a single click.
pub(crate) fn click_kind(raw: Kind, previous: Option<Kind>) -> Kind {
    match (raw, previous) {
        (Kind::Single, _) | (_, None) => Kind::Single,
        (_, Some(Kind::Single)) => Kind::Double,
        (_, Some(Kind::Double)) => Kind::Triple,
        (_, Some(Kind::Triple)) => Kind::Double,
    }
}

/// A key an open popover takes for itself (rule 4.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Enter,
    /// Only suggestions take Tab.
    Tab,
    /// Escape with no Ctrl, Alt or logo key held, on a menu of the host's
    /// or on suggestions. The host hears of the same press from its own
    /// key handling as well, and drops that one
    /// ([`crate::EditorView::take_claimed_escape`]).
    Escape,
}

impl MenuKey {
    /// What the key does to an open popover of `mode`: Up and Down move its
    /// selection, Enter chooses the selected row, as Tab does among
    /// suggestions, and Escape dismisses it. `None` when the key is not
    /// that mode's: Tab on a menu.
    pub(crate) fn on(self, mode: Mode) -> Option<PopoverEvent> {
        match (self, mode) {
            (Self::Up, _) => Some(PopoverEvent::Move(-1)),
            (Self::Down, _) => Some(PopoverEvent::Move(1)),
            (Self::Enter, _) | (Self::Tab, Mode::Suggest) => Some(PopoverEvent::AcceptSelected),
            (Self::Escape, _) => Some(PopoverEvent::Dismiss),
            (Self::Tab, Mode::Menu) => None,
        }
    }

    /// The key itself.
    pub(crate) fn named(self) -> Named {
        match self {
            Self::Up => Named::ArrowUp,
            Self::Down => Named::ArrowDown,
            Self::Enter => Named::Enter,
            Self::Tab => Named::Tab,
            Self::Escape => Named::Escape,
        }
    }
}

/// A key press the editor hands to a host that took the keyboard
/// ([`PopoverEvent::Stroke`]): the key and what it types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stroke {
    key: StrokeKey,
    modifiers: Modifiers,
    text: Option<char>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrokeKey {
    Named(Named),
    Character(char),
}

impl Stroke {
    /// `press`, unless its key is neither named nor one character.
    pub(crate) fn of(press: &KeyPress) -> Option<Self> {
        let key = match press.key.as_ref() {
            Key::Named(named) => StrokeKey::Named(named),
            Key::Character(character) => {
                let mut characters = character.chars();
                let first = characters.next()?;
                if characters.next().is_some() {
                    return None;
                }
                StrokeKey::Character(first)
            }
            Key::Unidentified => return None,
        };
        Some(Self {
            key,
            modifiers: press.modifiers,
            text: (press.text.as_deref()).and_then(|text| text.chars().find(|c| !c.is_control())),
        })
    }

    /// The named key pressed, if it is one.
    pub fn named(self) -> Option<Named> {
        match self.key {
            StrokeKey::Named(named) => Some(named),
            StrokeKey::Character(_) => None,
        }
    }

    /// The character of the key pressed, without its modifiers, if it has
    /// one.
    pub fn character(self) -> Option<char> {
        match self.key {
            StrokeKey::Named(_) => None,
            StrokeKey::Character(character) => Some(character),
        }
    }

    pub fn modifiers(self) -> Modifiers {
        self.modifiers
    }

    /// The character the press types, if it types one.
    pub fn text(self) -> Option<char> {
        self.text
    }
}

/// What the widget's copy of the open popover is a copy of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opened {
    /// A menu: the suggestions menu, open or asked for, or one of the
    /// host's.
    Menu,
    Suggest,
    /// Nothing of the editor's: the host took the keyboard for a request
    /// of this batch ([`Requested::Keyboard`]).
    Keyboard,
}

impl From<Mode> for Opened {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Menu => Self::Menu,
            Mode::Suggest => Self::Suggest,
        }
    }
}

/// Whether a press of Escape with `modifiers` is the Escape a popover
/// takes and a host navigates with: with no Ctrl, Alt or logo key held.
/// With one of them it is a shortcut, and any other key to a popover.
pub fn is_plain_escape(modifiers: Modifiers) -> bool {
    !(modifiers.control() || modifiers.alt() || modifiers.logo())
}

/// `selected` moved `delta` rows among `items`, stopping at the ends; a
/// popover with nothing selected stays so.
fn stepped(selected: Option<usize>, items: usize, delta: i32) -> Option<usize> {
    let selected = selected?;
    let last = items.saturating_sub(1);
    let moved = i64::try_from(selected)
        .unwrap_or(i64::MAX)
        .saturating_add(i64::from(delta));
    Some(usize::try_from(moved.max(0)).map_or(last, |row| row.min(last)))
}

/// The selection once the pointer is over `row`: that row, if it is one of
/// `items`.
fn hovered(selected: Option<usize>, items: usize, row: usize) -> Option<usize> {
    if row < items {
        Some(row)
    } else {
        selected
    }
}

/// The open popover as the editor widget sees it while it handles a batch
/// of events.
///
/// iced hands every event queued since the last frame to the same widget,
/// the popover's overlay first, before the host applies any message the
/// widget published. So the widget keeps its own copy, starting from the
/// popover the host handed it and changed by each message it publishes as
/// the host will change its own, and routes keys and presses against that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenMenu {
    /// The selected row; `None` when the popover has nothing to offer.
    pub selected: Option<usize>,
    /// How many rows it has.
    pub items: usize,
    /// Opened by a message of this batch. The host opens it against the
    /// caret and the text as the batch's earlier messages leave them, which
    /// the widget does not have, so whether it opens and what it offers are
    /// unknown until the next batch: the popover's keys go to the host as
    /// [`PopoverEvent::Key`], which it resolves against its real popover.
    pub opening: bool,
}

impl OpenMenu {
    /// A popover opened in this batch; see [`Self::opening`].
    pub(crate) const OPENING: Self = Self {
        selected: None,
        items: 0,
        opening: true,
    };

    pub(crate) fn of(popover: &Popover) -> Self {
        Self {
            selected: popover.selected(),
            items: popover.labels().len(),
            opening: false,
        }
    }

    /// The menu `message` leaves `menu` as, once the host applies it; see
    /// [`after`].
    pub(crate) fn after(menu: Option<Self>, message: &Message) -> Option<Self> {
        after(menu.map(|menu| (Opened::Menu, menu)), message).map(|(_, menu)| menu)
    }
}

/// The popover `message` leaves `open` as, once the host applies it, and
/// what it is, for every message whose effect the message alone decides.
/// (A request for a popover of the host's and a character typed open what
/// the host's sources say, which [`Now::publish`] asks.)
///
/// A request for the suggestions menu may or may not open a menu
/// ([`OpenMenu::opening`]), and so may a right press the host resolves:
/// with no popover open, it opens one on a misspelled word. A move or a
/// hover moves the selection, and a popover's key acts as it does on an
/// open popover. A stroke leaves the keyboard with the host that took it.
/// Every other message closes the popover: the host closes it on any
/// action, intent, undo or redo (rule 4.4.8), and with no popover open a
/// popover's key leaves none.
pub(crate) fn after(
    open: Option<(Opened, OpenMenu)>,
    message: &Message,
) -> Option<(Opened, OpenMenu)> {
    match message {
        Message::Spell(
            PopoverEvent::OpenAtCaret
            | PopoverEvent::OpenAt(_)
            | PopoverEvent::Press(MenuPress::Right(_)),
        ) => Some((Opened::Menu, OpenMenu::OPENING)),
        Message::Spell(PopoverEvent::Stroke(_)) => {
            open.filter(|(opened, _)| *opened == Opened::Keyboard)
        }
        Message::Spell(PopoverEvent::Key(key)) => {
            let mode = match open?.0 {
                Opened::Menu => Mode::Menu,
                Opened::Suggest => Mode::Suggest,
                Opened::Keyboard => return None,
            };
            after(open, &Message::Spell(key.on(mode)?))
        }
        Message::Spell(PopoverEvent::Move(delta)) => open.map(|(opened, menu)| {
            let selected = stepped(menu.selected, menu.items, *delta);
            (opened, OpenMenu { selected, ..menu })
        }),
        Message::Spell(PopoverEvent::Hover(row)) => open.map(|(opened, menu)| {
            let selected = hovered(menu.selected, menu.items, *row);
            (opened, OpenMenu { selected, ..menu })
        }),
        _ => None,
    }
}

/// Publish `message`, keeping `menu`, the widget's copy, as the host will
/// leave its own once it applies it. For the overlay, whose messages mean
/// the same in either mode; the widget publishes through [`Now`].
pub(crate) fn publish(
    shell: &mut iced::advanced::Shell<'_, Message>,
    menu: &Cell<Option<OpenMenu>>,
    message: Message,
) {
    menu.set(OpenMenu::after(menu.get(), &message));
    shell.publish(message);
}

/// The host's sources, as the widget asks them.
#[derive(Clone, Copy, Default)]
pub(crate) struct Sources<'a> {
    pub menus: Option<&'a dyn MenuSource>,
    pub suggestions: Option<&'a dyn SuggestSource>,
}

/// What is open as the widget's events leave it, through one batch: its
/// copy of the popover the host has open ([`OpenMenu`]), the popover it
/// opened itself for a message of the batch, which the host will open the
/// same when it applies that message, and what it knows of the caret's
/// line to ask the host's sources with.
pub(crate) struct Now {
    open: Cell<Option<OpenMenu>>,
    /// What `open` is a copy of.
    opened: Cell<Opened>,
    /// The popover opened in this batch, when `open` is a copy of that.
    own: RefCell<Option<Popover>>,
    /// The characters the batch has typed at the caret while nothing else
    /// changed the text or moved the caret; `None` once something did,
    /// after which the caret's line is not known.
    typed: RefCell<Option<String>>,
    /// The last message published was [`PopoverEvent::Hold`], so the edit
    /// after it asks for no suggestions.
    held: Cell<bool>,
    /// The line Escape hid suggestions for.
    suppressed: Cell<Option<usize>>,
}

impl Now {
    pub(crate) fn new() -> Self {
        Self {
            open: Cell::new(None),
            opened: Cell::new(Opened::Menu),
            own: RefCell::new(None),
            typed: RefCell::new(Some(String::new())),
            held: Cell::new(false),
            suppressed: Cell::new(None),
        }
    }

    /// Start from `popover`, the one the host has open.
    pub(crate) fn start(&self, popover: Option<&Popover>) {
        self.set(popover.map(|popover| (popover.mode().into(), OpenMenu::of(popover))));
    }

    /// The line the host hides suggestions for.
    pub(crate) fn suppress(&self, line: Option<usize>) {
        self.suppressed.set(line);
    }

    fn set(&self, open: Option<(Opened, OpenMenu)>) {
        self.open.set(open.map(|(_, menu)| menu));
        if let Some((opened, _)) = open {
            self.opened.set(opened);
        }
    }

    /// The copy, whatever it is a copy of.
    pub(crate) fn get(&self) -> Option<OpenMenu> {
        self.open.get()
    }

    /// The copy and what it is a copy of.
    pub(crate) fn live(&self) -> Option<(Opened, OpenMenu)> {
        self.open.get().map(|menu| (self.opened.get(), menu))
    }

    /// Whether a popover of the editor's is open: not when the host only
    /// took the keyboard.
    pub(crate) fn is_open(&self) -> bool {
        (self.live()).is_some_and(|(opened, _)| opened != Opened::Keyboard)
    }

    /// The copy alone, for the overlay.
    pub(crate) fn cell(&self) -> &Cell<Option<OpenMenu>> {
        &self.open
    }

    /// Whether the popover that is open is one the widget opened in this
    /// batch, which the host has not drawn.
    pub(crate) fn is_own(&self) -> bool {
        self.open.get().is_some() && self.own.borrow().is_some()
    }

    /// What is open, of the popover the host handed over (`handed`) or
    /// the one opened in this batch: whether it is the suggestions menu,
    /// and its rows' accelerators.
    pub(crate) fn rows(&self, handed: Option<&Popover>) -> (bool, Vec<Option<Accelerator>>) {
        let own = self.own.borrow();
        (own.as_ref().or(handed)).map_or((false, Vec::new()), |popover| {
            (popover.is_spelling(), popover.accelerators().to_vec())
        })
    }

    /// The host closes its popover without a message from the widget.
    pub(crate) fn closed(&self) {
        self.open.set(None);
        self.own.replace(None);
    }

    /// The caret's line as the batch's messages so far leave it, from the
    /// line the host had (`start`); `None` once it is not known.
    fn line(&self, start: &dyn Fn() -> Option<CaretLine>) -> Option<CaretLine> {
        let typed = self.typed.borrow();
        let typed = typed.as_ref()?;
        let mut line = start()?;
        if !typed.is_empty() {
            line.text.insert_str(line.caret, typed);
            line.caret += typed.len();
        }
        Some(line)
    }

    /// Publish `message`, keeping what is open as the host will leave it
    /// once it applies the message. `start` reads the caret's line as the
    /// host has it, and `sources` are the host's, asked for what a request
    /// or a typed character opens.
    pub(crate) fn publish(
        &self,
        shell: &mut iced::advanced::Shell<'_, Message>,
        message: Message,
        start: &dyn Fn() -> Option<CaretLine>,
        sources: Sources<'_>,
    ) {
        let was = self.live();
        let held = self
            .held
            .replace(matches!(message, Message::Spell(PopoverEvent::Hold)));
        // The caret's line once the host applies the message. A typed
        // character lands at the caret; with a selection it replaces that,
        // and what is left is not known.
        let typed = match &message {
            Message::Action(Action::Edit(Edit::Insert(c))) => Some(*c),
            _ => None,
        };
        if sources.menus.is_none() && sources.suggestions.is_none() {
            // With no source to ask, the line is never needed.
        } else if let Some(c) = typed {
            let known = self.line(start).is_some_and(|line| !line.selected);
            let mut typed = self.typed.borrow_mut();
            match typed.as_mut().filter(|_| known) {
                Some(typed) => typed.push(c),
                None => *typed = None,
            }
        } else if !keeps_the_text(&message, was.is_some_and(|(_, menu)| !menu.opening)) {
            self.typed.replace(None);
        }
        let line = || self.line(start);

        let mut next = after(was, &message);
        let mut own = None;
        match &message {
            Message::Spell(PopoverEvent::Request(id)) => {
                let answer = (sources.menus.zip(line()))
                    .map_or(Answer::Nothing, |(source, line)| {
                        requested(source, *id, &line)
                    });
                next = match answer {
                    Answer::Menu(popover) => {
                        let open = OpenMenu::of(&popover);
                        own = Some(popover);
                        Some((Opened::Menu, open))
                    }
                    Answer::Keyboard => Some((Opened::Keyboard, OpenMenu::OPENING)),
                    Answer::Nothing => None,
                };
            }
            Message::Action(Action::Edit(Edit::Insert(_))) if !held => {
                let popover = (sources.suggestions.zip(line()))
                    .filter(|(_, line)| self.suppressed.get() != Some(line.index))
                    .and_then(|(source, line)| suggested(source, &line));
                next = popover.map(|popover| {
                    let open = OpenMenu::of(&popover);
                    own = Some(popover);
                    (Opened::Suggest, open)
                });
            }
            // Escape hides suggestions for the line, as the host will.
            Message::Spell(PopoverEvent::Key(MenuKey::Escape))
                if was.is_some_and(|(opened, _)| opened == Opened::Suggest) =>
            {
                self.suppressed.set(line().map(|line| line.index));
            }
            _ => {}
        }
        // A move, a hover or a key of its own leaves the open popover the
        // one it was.
        let kept = next.is_some()
            && matches!(
                message,
                Message::Spell(
                    PopoverEvent::Move(_) | PopoverEvent::Hover(_) | PopoverEvent::Key(_)
                )
            );
        if !kept {
            self.own.replace(own);
        }
        self.set(next);
        shell.publish(message);
    }
}

/// Whether `message` leaves the text and the caret where they were, with
/// `open` being whether the widget knows a popover to be open: the
/// popover's own events, and what only asks for a popover. A key the host
/// resolves against a popover the widget knows to be open moves or
/// dismisses it; with none known it may be the text's. Choosing a row is
/// the host's to act on, and may change anything.
fn keeps_the_text(message: &Message, open: bool) -> bool {
    match message {
        Message::Spell(event) => match event {
            PopoverEvent::OpenAtCaret
            | PopoverEvent::OpenAt(_)
            | PopoverEvent::Request(_)
            | PopoverEvent::Move(_)
            | PopoverEvent::Hover(_)
            | PopoverEvent::Hold
            | PopoverEvent::Close
            | PopoverEvent::Dismiss
            | PopoverEvent::Stroke(_)
            | PopoverEvent::Press(MenuPress::Right(_)) => true,
            PopoverEvent::Key(MenuKey::Up | MenuKey::Down | MenuKey::Escape) => open,
            PopoverEvent::Key(_)
            | PopoverEvent::Accept(_)
            | PopoverEvent::AcceptSelected
            | PopoverEvent::Press(MenuPress::Left(..)) => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use crate::Intent;
    use iced::widget::text_editor::Motion;

    fn rows(labels: &[&str]) -> Vec<Row> {
        (0..)
            .zip(labels)
            .map(|(id, label)| Row::new(id, *label))
            .collect()
    }

    #[test]
    fn a_new_popover_selects_its_first_row_and_stops_at_its_ends() {
        let mut popover = Popover::new(Mode::Menu, 2, 4..7, "teh".into(), rows(&["a", "b", "c"]));
        assert_eq!(
            (
                popover.mode(),
                popover.line(),
                popover.range(),
                popover.text()
            ),
            (Mode::Menu, 2, 4..7, "teh")
        );
        assert_eq!(popover.labels(), ["a", "b", "c"]);
        assert_eq!(popover.selected(), Some(0));
        popover.step(-1);
        assert_eq!(popover.selected(), Some(0));
        popover.step(5);
        assert_eq!(popover.selected(), Some(2));
        popover.hover(1);
        assert_eq!(popover.selected(), Some(1));
        popover.hover(3);
        assert_eq!(popover.selected(), Some(1));
        assert_eq!((popover.id(1), popover.id(3)), (Some(1), None));
        // Only the suggestions menu tints its anchor.
        assert_eq!((popover.is_spelling(), popover.tint()), (false, None));
        let spelling = popover.spelling();
        assert_eq!(
            (spelling.is_spelling(), spelling.tint()),
            (true, Some((2, 4..7)))
        );
        // With nothing to offer, nothing is selected.
        let mut empty = Popover::new(Mode::Menu, 0, 0..0, String::new(), Vec::new());
        empty.step(1);
        empty.hover(0);
        assert_eq!(empty.selected(), None);
    }

    #[test]
    fn an_accelerator_is_one_key_alone_or_with_shift() {
        use iced::keyboard::key::{Code, NativeCode};
        let unidentified = Physical::Unidentified(NativeCode::Unidentified);
        let pressed = |accelerator: Accelerator, key: &str, modifiers| {
            accelerator.matches(&Key::Character(key.into()), unidentified, modifiers)
        };
        let none = Modifiers::empty();
        let x = Accelerator::plain('x');
        assert!(pressed(x, "x", none));
        // Caps Lock, or a platform that reports the shifted letter.
        assert!(pressed(x, "X", none));
        assert!(!pressed(x, "x", Modifiers::SHIFT));
        assert!(!pressed(x, "y", none));
        for held in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            assert!(!pressed(x, "x", held), "{held:?}");
        }
        let high = Accelerator::shifted('h');
        assert!(pressed(high, "h", Modifiers::SHIFT));
        assert!(pressed(high, "H", Modifiers::SHIFT));
        assert!(!pressed(high, "h", none));
        assert!(!pressed(high, "h", Modifiers::SHIFT | Modifiers::CTRL));
        assert!(pressed(Accelerator::plain('/'), "/", none));
        // The space bar is a named key.
        let space = Accelerator::plain(' ');
        assert!(space.matches(&Key::Named(Named::Space), unidentified, none));
        assert!(!space.matches(&Key::Named(Named::Enter), unidentified, none));
        // A keyboard of another script: the key in the letter's place.
        let cyrillic = Key::Character("ч".into());
        assert!(x.matches(&cyrillic, Physical::Code(Code::KeyX), none));
        assert!(!x.matches(&cyrillic, Physical::Code(Code::KeyC), none));
    }

    #[test]
    fn each_mode_has_its_own_keys() {
        for mode in [Mode::Menu, Mode::Suggest] {
            assert_eq!(MenuKey::Up.on(mode), Some(PopoverEvent::Move(-1)));
            assert_eq!(MenuKey::Down.on(mode), Some(PopoverEvent::Move(1)));
            assert_eq!(MenuKey::Enter.on(mode), Some(PopoverEvent::AcceptSelected));
            assert_eq!(MenuKey::Escape.on(mode), Some(PopoverEvent::Dismiss));
        }
        assert_eq!(
            MenuKey::Tab.on(Mode::Suggest),
            Some(PopoverEvent::AcceptSelected)
        );
        assert_eq!(MenuKey::Tab.on(Mode::Menu), None);
        assert_eq!(MenuKey::Escape.named(), Named::Escape);
        assert_eq!(MenuKey::Tab.named(), Named::Tab);
    }

    #[test]
    fn escape_is_a_popovers_with_no_ctrl_alt_or_logo_key() {
        assert!(is_plain_escape(Modifiers::empty()));
        assert!(is_plain_escape(Modifiers::SHIFT));
        for held in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            assert!(!is_plain_escape(held), "{held:?}");
            assert!(!is_plain_escape(held | Modifiers::SHIFT), "{held:?}");
        }
    }

    fn pressed(key: Key, modifiers: Modifiers, text: Option<&str>) -> KeyPress {
        KeyPress {
            modified_key: key.clone(),
            key,
            physical_key: Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            modifiers,
            text: text.map(Into::into),
            status: iced::widget::text_editor::Status::Focused { is_hovered: false },
        }
    }

    fn letter() -> Stroke {
        Stroke::of(&pressed(
            Key::Character("x".into()),
            Modifiers::empty(),
            Some("x"),
        ))
        .unwrap()
    }

    #[test]
    fn a_stroke_carries_its_key_and_what_it_types() {
        let x = letter();
        assert_eq!(
            (x.character(), x.named(), x.text(), x.modifiers()),
            (Some('x'), None, Some('x'), Modifiers::empty())
        );
        let left = Stroke::of(&pressed(
            Key::Named(Named::ArrowLeft),
            Modifiers::empty(),
            None,
        ))
        .unwrap();
        assert_eq!(
            (left.character(), left.named(), left.text()),
            (None, Some(Named::ArrowLeft), None)
        );
        // Shift+H types `H` on the key `h`.
        let shifted = Stroke::of(&pressed(
            Key::Character("h".into()),
            Modifiers::SHIFT,
            Some("H"),
        ))
        .unwrap();
        assert_eq!(
            (shifted.character(), shifted.text(), shifted.modifiers()),
            (Some('h'), Some('H'), Modifiers::SHIFT)
        );
        // Control characters are not what a key types.
        let enter = pressed(Key::Named(Named::Enter), Modifiers::empty(), Some("\r"));
        assert_eq!(Stroke::of(&enter).unwrap().text(), None);
        // A key that is not one character cannot be carried.
        for key in [Key::Character("ab".into()), Key::Unidentified] {
            assert!(Stroke::of(&pressed(key, Modifiers::empty(), Some("a"))).is_none());
        }
    }

    fn line(text: &str, caret: usize) -> CaretLine {
        CaretLine {
            index: 4,
            text: text.to_owned(),
            caret,
            selected: false,
        }
    }

    #[test]
    fn the_sources_answers_are_checked_against_the_line() {
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

        impl MenuSource for Fixed {
            fn menu(&self, _id: u32, _line: &str, _caret: usize) -> Option<Requested> {
                Some(Requested::Rows {
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
            let source = Fixed(range.clone(), rows(labels));
            let suggestions = suggested(&source, &line("é x", 3));
            assert_eq!(suggestions.is_some(), opens, "{name}");
            let menu = requested(&source, 1, &line("é x", 3));
            assert_eq!(matches!(menu, Answer::Menu(_)), opens, "{name}");
            if let (Some(suggestions), Answer::Menu(menu)) = (suggestions, menu) {
                for (popover, mode) in [(suggestions, Mode::Suggest), (menu, Mode::Menu)] {
                    assert_eq!(
                        (popover.mode(), popover.line(), popover.range()),
                        (mode, 4, range.clone()),
                        "{name}"
                    );
                    assert_eq!(popover.text(), &"é x"[range.clone()], "{name}");
                    assert_eq!(popover.selected(), Some(0), "{name}");
                }
            }
        }
        // Suggestions are for a caret with nothing selected; a menu opens
        // over a selection too.
        let source = Fixed(0..2, rows(&["a"]));
        let selected = CaretLine {
            selected: true,
            ..line("é x", 3)
        };
        assert!(suggested(&source, &selected).is_none());
        assert!(matches!(requested(&source, 1, &selected), Answer::Menu(_)));
        // A source may take the keyboard, or open nothing.
        let menus = fixtures::Menus::default();
        assert_eq!(
            requested(&menus, fixtures::KEYBOARD, &line("x", 0)),
            Answer::Keyboard
        );
        assert_eq!(requested(&menus, 9, &line("x", 0)), Answer::Nothing);
        assert_eq!(
            requested(&menus, fixtures::STATUS, &line("x", 0)),
            Answer::Nothing
        );
    }

    const OPEN: OpenMenu = OpenMenu {
        selected: Some(0),
        items: 3,
        opening: false,
    };

    fn typed(c: char) -> Message {
        Message::Action(Action::Edit(Edit::Insert(c)))
    }

    #[test]
    fn the_copy_follows_each_message_that_alone_decides_what_is_open() {
        let key = |open, key| after(open, &Message::Spell(PopoverEvent::Key(key)));
        let open = Some((Opened::Suggest, OPEN));
        assert_eq!(
            key(open, MenuKey::Down),
            Some((
                Opened::Suggest,
                OpenMenu {
                    selected: Some(1),
                    ..OPEN
                }
            ))
        );
        assert_eq!(key(open, MenuKey::Up), open);
        // Enter and Tab choose, which closes them, and Escape dismisses.
        assert_eq!(key(open, MenuKey::Enter), None);
        assert_eq!(key(open, MenuKey::Tab), None);
        assert_eq!(key(open, MenuKey::Escape), None);
        // Tab is no key of a menu, and with nothing open no key leaves one.
        let menu = Some((Opened::Menu, OPEN));
        assert_eq!(key(menu, MenuKey::Tab), None);
        assert_eq!(key(menu, MenuKey::Escape), None);
        assert_eq!(key(None, MenuKey::Tab), None);
        // A request for the suggestions menu may open it over anything.
        let keyboard = Some((Opened::Keyboard, OpenMenu::OPENING));
        for was in [None, open, menu, keyboard] {
            assert_eq!(
                after(was, &Message::Spell(PopoverEvent::OpenAtCaret)),
                Some((Opened::Menu, OpenMenu::OPENING))
            );
        }
        // A stroke leaves the keyboard with the host that took it, and is
        // nothing to a popover.
        let stroke = Message::Spell(PopoverEvent::Stroke(letter()));
        assert_eq!(after(keyboard, &stroke), keyboard);
        assert_eq!(after(open, &stroke), None);
        assert_eq!(key(keyboard, MenuKey::Down), None);
        // Every other message closes what is open: what a request or a
        // typed character opens is the sources' to say.
        for message in [
            typed('d'),
            Message::Action(Action::Edit(Edit::Backspace)),
            Message::Action(Action::Move(Motion::Left)),
            Message::Intent(Intent::Backspace),
            Message::Undo,
            Message::Spell(PopoverEvent::Request(1)),
            Message::Spell(PopoverEvent::Hold),
            Message::Spell(PopoverEvent::Dismiss),
            Message::Spell(PopoverEvent::Close),
        ] {
            for was in [open, menu, keyboard] {
                assert_eq!(after(was, &message), None, "{message:?}");
            }
        }
    }

    /// A widget's batch state over the line `text` with the caret at
    /// `caret`, and what it publishes with the test sources.
    struct Batch {
        now: Now,
        start: CaretLine,
        completes: fixtures::Completes,
        menus: fixtures::Menus,
        published: usize,
    }

    impl Batch {
        fn over(text: &str, caret: usize) -> Self {
            Self {
                now: Now::new(),
                start: line(text, caret),
                completes: fixtures::Completes::new(&["due", "duplicate", "today"]),
                menus: fixtures::Menus::default(),
                published: 0,
            }
        }

        fn publish(&mut self, message: Message) -> Option<(Opened, OpenMenu)> {
            let mut messages = Vec::new();
            let mut shell = iced::advanced::Shell::new(&mut messages);
            let sources = Sources {
                menus: Some(&self.menus),
                suggestions: Some(&self.completes),
            };
            let start = self.start.clone();
            self.now
                .publish(&mut shell, message, &|| Some(start.clone()), sources);
            self.published += messages.len();
            self.now.live()
        }

        fn event(&mut self, event: PopoverEvent) -> Option<(Opened, OpenMenu)> {
            self.publish(Message::Spell(event))
        }
    }

    fn suggesting(rows: usize) -> Option<(Opened, OpenMenu)> {
        Some((
            Opened::Suggest,
            OpenMenu {
                selected: Some(0),
                items: rows,
                opening: false,
            },
        ))
    }

    #[test]
    fn a_typed_character_opens_the_suggestions_the_source_offers_for_the_line() {
        let mut batch = Batch::over("pay ", 4);
        assert_eq!(batch.publish(typed('d')), None);
        // The line as the two letters leave it, and the caret after them.
        assert_eq!(batch.publish(typed('u')), suggesting(2));
        assert_eq!(
            batch.completes.asked(),
            [("pay d".to_owned(), 5), ("pay du".to_owned(), 6)]
        );
        assert!(batch.now.is_own());
        // They follow what is typed next, and close when nothing matches.
        assert_eq!(batch.publish(typed('p')), suggesting(1));
        assert_eq!(batch.publish(typed('x')), None);
        assert!(!batch.now.is_own());
        assert_eq!(batch.completes.asked().len(), 4);
        assert_eq!(batch.published, 4);
    }

    #[test]
    fn the_suggestions_keys_keep_them_open_and_keep_the_line_known() {
        let mut batch = Batch::over("", 0);
        batch.publish(typed('d'));
        batch.publish(typed('u'));
        let moved = batch.event(PopoverEvent::Key(MenuKey::Down));
        assert_eq!(moved.map(|(_, menu)| menu.selected), Some(Some(1)));
        assert!(batch.now.is_own());
        // The next letter is asked about with the line the keys left alone.
        assert_eq!(batch.publish(typed('p')), suggesting(1));
        assert_eq!(batch.completes.asked().last().unwrap().0, "dup");
        // Choosing a row is the host's to act on: the line is not known
        // afterwards, and nothing more opens in this batch.
        assert_eq!(batch.event(PopoverEvent::Key(MenuKey::Enter)), None);
        assert_eq!(batch.publish(typed('l')), None);
        assert_eq!(batch.completes.asked().len(), 3);
    }

    #[test]
    fn once_the_line_is_not_known_nothing_opens_until_the_batch_ends() {
        let unknowns = [
            Message::Intent(Intent::Backspace),
            Message::Action(Action::Edit(Edit::Backspace)),
            Message::Action(Action::Edit(Edit::Enter)),
            Message::Action(Action::Edit(Edit::Paste(std::sync::Arc::new("x".into())))),
            Message::Action(Action::Move(Motion::Left)),
            Message::Action(Action::Click(Point::ORIGIN)),
            Message::Intent(Intent::Enter),
            Message::Undo,
            Message::Spell(PopoverEvent::Accept(0)),
            Message::Spell(PopoverEvent::AcceptSelected),
            Message::Spell(PopoverEvent::Key(MenuKey::Down)),
            Message::Spell(PopoverEvent::Press(MenuPress::Left(
                Point::ORIGIN,
                Kind::Single,
            ))),
        ];
        for message in unknowns {
            let mut batch = Batch::over("- [ ] d", 7);
            assert_eq!(batch.publish(message.clone()), None, "{message:?}");
            // Neither suggestions nor a menu, and the sources are not asked.
            assert_eq!(batch.publish(typed('u')), None, "{message:?}");
            assert_eq!(
                batch.event(PopoverEvent::Request(fixtures::STATUS)),
                None,
                "{message:?}"
            );
            assert!(batch.completes.asked().is_empty(), "{message:?}");
            assert!(batch.menus.asked().is_empty(), "{message:?}");
        }
        // What leaves the text and the caret alone leaves the line known.
        for message in [
            Message::Spell(PopoverEvent::Close),
            Message::Spell(PopoverEvent::Hold),
            Message::Spell(PopoverEvent::OpenAtCaret),
            Message::Spell(PopoverEvent::OpenAt(Point::ORIGIN)),
            Message::Spell(PopoverEvent::Press(MenuPress::Right(Point::ORIGIN))),
            Message::Spell(PopoverEvent::Request(9)),
            Message::Spell(PopoverEvent::Hover(1)),
        ] {
            let mut batch = Batch::over("- [ ] d", 7);
            batch.publish(message.clone());
            let menu = batch.event(PopoverEvent::Request(fixtures::STATUS));
            assert_eq!(
                menu.map(|(opened, _)| opened),
                Some(Opened::Menu),
                "{message:?}"
            );
        }
        // A character typed over a selection replaces it: what is left is
        // not known.
        let mut batch = Batch::over("du", 2);
        batch.start.selected = true;
        assert_eq!(batch.publish(typed('e')), None);
        assert_eq!(batch.publish(typed('e')), None);
        assert!(batch.completes.asked().is_empty());
    }

    #[test]
    fn a_request_opens_what_the_menu_source_answers_for_the_caret() {
        let mut batch = Batch::over("- [ ] call", 8);
        let menu = batch.event(PopoverEvent::Request(fixtures::STATUS));
        assert_eq!(
            menu,
            Some((
                Opened::Menu,
                OpenMenu {
                    selected: Some(0),
                    items: 3,
                    opening: false
                }
            ))
        );
        assert_eq!(
            batch.menus.asked(),
            [(fixtures::STATUS, "- [ ] call".to_owned(), 8)]
        );
        // Its rows are known for the keys that follow, and it is no
        // suggestions menu.
        let (spelling, accelerators) = batch.now.rows(None);
        assert!(!spelling && batch.now.is_own());
        assert_eq!(
            accelerators,
            [' ', '/', 'x'].map(|key| Some(Accelerator::plain(key)))
        );
        let moved = batch.event(PopoverEvent::Move(1));
        assert_eq!(moved.map(|(_, menu)| menu.selected), Some(Some(1)));
        assert_eq!(batch.now.rows(None).1.len(), 3);
        assert_eq!(batch.event(PopoverEvent::AcceptSelected), None);
        assert!(batch.now.rows(None).1.is_empty());

        // After typed characters it is asked about the line they leave.
        let mut batch = Batch::over("- [ ] ", 6);
        batch.publish(typed('a'));
        batch.event(PopoverEvent::Request(fixtures::STATUS));
        assert_eq!(
            batch.menus.asked()[0],
            (fixtures::STATUS, "- [ ] a".to_owned(), 7)
        );
        // A request the source opens nothing for closes what was open.
        let mut batch = Batch::over("- [ ] call", 8);
        batch.event(PopoverEvent::Request(fixtures::STATUS));
        assert_eq!(batch.event(PopoverEvent::Request(9)), None);
        assert!(!batch.now.is_own());
        // Not on a line that is no task.
        let mut batch = Batch::over("call", 2);
        assert_eq!(batch.event(PopoverEvent::Request(fixtures::STATUS)), None);
    }

    #[test]
    fn a_host_that_takes_the_keyboard_keeps_it_through_the_strokes_after() {
        let mut batch = Batch::over("- [ ] call", 8);
        let keyboard = Some((Opened::Keyboard, OpenMenu::OPENING));
        assert_eq!(
            batch.event(PopoverEvent::Request(fixtures::KEYBOARD)),
            keyboard
        );
        assert!(!batch.now.is_own());
        assert_eq!(batch.event(PopoverEvent::Stroke(letter())), keyboard);
        assert_eq!(batch.event(PopoverEvent::Stroke(letter())), keyboard);
        // Anything else is the editor's again.
        assert_eq!(
            batch.publish(Message::Action(Action::Click(Point::ORIGIN))),
            None
        );
        assert_eq!(batch.event(PopoverEvent::Stroke(letter())), None);
    }

    #[test]
    fn a_held_edit_and_a_hidden_line_ask_for_no_suggestions() {
        // The edit after a hold opens nothing, and is still typed.
        let mut batch = Batch::over("d", 1);
        batch.event(PopoverEvent::Hold);
        assert_eq!(batch.publish(typed('u')), None);
        assert!(batch.completes.asked().is_empty());
        assert_eq!(batch.publish(typed('e')), suggesting(1));
        assert_eq!(batch.completes.asked(), [("due".to_owned(), 3)]);

        // The line the host hides suggestions for.
        let mut batch = Batch::over("d", 1);
        batch.now.suppress(Some(4));
        assert_eq!(batch.publish(typed('u')), None);
        assert!(batch.completes.asked().is_empty());
        batch.now.suppress(Some(5));
        assert_eq!(batch.publish(typed('p')), suggesting(1));

        // Escape hides them for the line, as the host will.
        let mut batch = Batch::over("", 0);
        batch.publish(typed('d'));
        assert_eq!(batch.publish(typed('u')), suggesting(2));
        assert_eq!(batch.event(PopoverEvent::Key(MenuKey::Escape)), None);
        assert_eq!(batch.publish(typed('e')), None);
        assert_eq!(batch.completes.asked().len(), 2);
    }

    #[test]
    fn the_widgets_copy_starts_from_the_hosts_popover() {
        let popover = Popover::new(Mode::Suggest, 4, 0..2, "du".into(), rows(&["a", "b"]));
        let now = Now::new();
        assert_eq!((now.get(), now.live()), (None, None));
        now.start(Some(&popover));
        assert_eq!(now.live(), Some((Opened::Suggest, OpenMenu::of(&popover))));
        // It is the host's, drawn, with its rows.
        assert!(!now.is_own());
        assert_eq!(now.rows(Some(&popover)), (false, vec![None, None]));
        now.closed();
        assert_eq!(now.get(), None);
        let menu = Popover::new(Mode::Menu, 4, 0..2, "du".into(), rows(&["a"])).spelling();
        now.start(Some(&menu));
        assert_eq!(now.live(), Some((Opened::Menu, OpenMenu::of(&menu))));
        assert_eq!(now.rows(Some(&menu)), (true, vec![None]));
        now.start(None);
        assert_eq!(now.live(), None);
    }

    #[test]
    fn with_no_sources_typing_and_requests_open_nothing() {
        let now = Now::new();
        let mut messages = Vec::new();
        let mut shell = iced::advanced::Shell::new(&mut messages);
        let start = || Some(line("- [ ] d", 7));
        for message in [
            typed('u'),
            Message::Spell(PopoverEvent::Request(fixtures::STATUS)),
        ] {
            now.publish(&mut shell, message, &start, Sources::default());
            assert_eq!(now.live(), None);
        }
        assert_eq!(messages.len(), 2);
    }
}
