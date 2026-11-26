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
//! are the popover's, whatever the host has bound them to, but for Enter on
//! a row the host marks tentative ([`Row::tentative`]), which is the
//! text's until Up or Down has moved the selection; Escape closes it
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
//!   after the request are then no one's, and dropped (see below).
//! - A character typed asks the host's [`SuggestSource`] the same way, and
//!   the keys after it find the suggestions open, or not.
//!
//! Suggestions a typed character opens in the batch have not been shown,
//! and the host's source may answer otherwise by the time it opens them.
//! Up and Down are theirs, Tab on them is dropped, and Enter is the
//! text's, as with nothing open, but on a selected row that opens a menu
//! of the host's ("Due date…", which writes nothing until a date is
//! picked): the widget names that row ([`PopoverEvent::EnterOn`]), which
//! the host chooses only if its suggestions have it selected, and the
//! batch's keys after it are dropped.
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
//! **A right-click** asks the host's [`Provider`]s, in the host's order,
//! about what lies under the pointer ([`Clicked`]): the first whose answer
//! opens something takes the click, and those after it are not asked. With
//! none, the press changes nothing. The suggestions menu is one of them,
//! the editor's own ([`Provider::Spelling`]), first unless the host says
//! otherwise: it takes a misspelled word, and is asked for as it always
//! was ([`PopoverEvent::OpenAt`]). A [`MenuProvider`] of the host's is
//! asked like a [`MenuSource`], in the same pass, and the host is handed
//! [`PopoverEvent::Context`], which names the provider that answered and
//! what the widget saw ([`Seen`]): the text's revision, the host's list
//! of providers, the popovers the editor had opened, the place clicked,
//! the provider and its answer. The editor asks that provider again and
//! opens its menu only if all of it is still so: a click made over a note
//! the host has since replaced opens nothing.
//!
//! A row of a menu is chosen from that menu or from none. Each popover the
//! editor opens has its own number ([`Popover::instance`]), and a row of
//! any menu but the suggestions menu, a provider's, a request's or one the
//! host opened, is chosen by an event that names the menu
//! ([`PopoverEvent::AcceptOf`]): the editor ignores it when another popover
//! is open by then, one the host opened in between included. So are rows
//! of suggestions with a row that runs a command or is bound to them
//! ([`Row::bound`]).
//!
//! A row may be checked, which marks what the menu's subject is now, and a
//! menu opens with it selected; and a row may be one that cannot be chosen
//! ([`Row::enabled`]), which is drawn muted, which Up, Down and the pointer
//! pass over, and whose accelerator does nothing, the menu staying open.
//!
//! The widget asks about the text as it is laid out, which is the text
//! the host has only while the batch's earlier messages left the text,
//! the caret and the scroll alone. After any that did not, **no provider
//! of the host's is asked, and none answers**: the press does what it did
//! before there were providers, which is to ask for the suggestions menu
//! on a word drawn as misspelled, where the suggestions menu comes before
//! every provider of the host's, and otherwise nothing. So does a right
//! press the host resolves after a request for the suggestions menu in
//! its batch ([`PopoverEvent::Press`]). The user right-clicks again.
//!
//! **Input in the frame of a handover is dropped.** When a source or a
//! provider has the host take the keyboard ([`Requested::Keyboard`]), the
//! host opens its own, a field say, and gives it the keyboard once it has
//! been handed the request or the click, which is after the batch. What
//! the batch still holds was typed before the user could see that field,
//! and belongs to no one. So from the handover to the end of its batch
//! the widget takes every key the host's field would take and does
//! nothing with it; a composition the note had under way is cancelled,
//! and no preedit or committed text reaches the note. What that
//! composition, or one begun in that batch, commits later is discarded
//! too, though the editor still has the keyboard then. Escape and the
//! host's shortcuts stay the host's, as with nothing open. From the next
//! batch on the host's field has the keyboard, by the toolkit's own
//! focus. A row that opens a menu of the host's ([`Row::opens_menu`]) hands
//! it the keyboard the same, chosen with a key or with the pointer.
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
//! is open, as the widget will find when it routes that event (a key, a
//! press off the popover, a wheel that scrolls the editor), and leaves the
//! presses after it to the widget, which sees them after it. Up and Down
//! leave the rows as they are drawn, and a press on a row after them
//! still chooses that row.
//!
//! [`KeyRoute::Popover`]: crate::KeyRoute::Popover

pub(crate) mod menu;

use std::cell::{Cell, RefCell};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use iced::advanced::mouse::click::Kind;
use iced::keyboard::key::{Named, Physical};
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor::{Action, Edit, KeyPress};
use iced::Point;
use wren_markdown::span::LineSpans;

use crate::hit::Hit;
use crate::Message;

pub(crate) use menu::{
    fit_with, height_of, hint_size, measure, place, Extras, Fit, InOrder, MenuOverlay,
};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Accelerator {
    key: char,
    shift: bool,
    /// The key of a row that cannot be chosen: pressed, it does nothing,
    /// and the menu stays open.
    inert: bool,
}

impl Accelerator {
    /// `key` pressed alone; a space is the space bar.
    pub const fn plain(key: char) -> Self {
        Self {
            key,
            shift: false,
            inert: false,
        }
    }

    /// `key` pressed with Shift, named as the key reads without it.
    pub const fn shifted(key: char) -> Self {
        Self {
            key,
            shift: true,
            inert: false,
        }
    }

    /// The accelerator as a row that cannot be chosen has it.
    pub(crate) const fn inert(self) -> Self {
        Self {
            inert: true,
            ..self
        }
    }

    /// Whether it is a row's that cannot be chosen, and so chooses nothing.
    pub(crate) const fn is_inert(self) -> bool {
        self.inert
    }

    /// How a menu names the key beside its row: `Space`, a letter in
    /// capitals, `⇧` before a shifted one, and any other character as it
    /// is.
    pub fn label(self) -> String {
        let key = match self.key {
            ' ' => "Space".to_owned(),
            key => key.to_uppercase().collect(),
        };
        if self.shift {
            format!("\u{21e7}{key}")
        } else {
            key
        }
    }

    /// [`Self::matches`], or for a key that is no letter, a press of any
    /// key that types it (`text`) with no Ctrl, Alt or logo key held, with
    /// or without Shift: on many keyboards `/` and `-` are shifted keys.
    pub(crate) fn matches_press(
        self,
        key: &Key,
        physical: Physical,
        modifiers: Modifiers,
        text: Option<&str>,
    ) -> bool {
        if self.matches(key, physical, modifiers) {
            return true;
        }
        let typed = text.and_then(|text| {
            let mut chars = text.chars();
            chars.next().filter(|_| chars.next().is_none())
        });
        !self.key.is_alphabetic()
            && self.key != ' '
            && !self.shift
            && !(modifiers.control() || modifiers.alt() || modifiers.logo())
            && typed == Some(self.key)
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

/// What choosing a row does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RowKind {
    /// The row goes back to whoever opened its popover, by its id: the
    /// host ([`crate::EditorView::take_choice`]) or the provider that
    /// answered the right-click ([`crate::EditorView::take_provided`]).
    #[default]
    Own,
    /// The row runs the host command its id names, whoever opened the
    /// popover ([`crate::EditorView::take_command`]). Only such a row's id
    /// is ever read as a command.
    Command,
}

/// One row of a popover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The host's name for the row, handed back when it is chosen.
    pub id: u32,
    pub label: String,
    /// The key that chooses the row while a menu is open, named beside
    /// the row ([`Accelerator::label`]).
    pub accelerator: Option<Accelerator>,
    pub kind: RowKind,
    /// A short mark drawn before the label, in a column of its own: a
    /// status's box, a priority's glyph.
    pub glyph: Option<String>,
    /// The row is what the menu's subject is now, a task's status say:
    /// its label is drawn in the accent and heavier, and a menu opens
    /// with it selected.
    pub checked: bool,
    /// Whether the row can be chosen. One that cannot is drawn muted, is
    /// never selected, and its accelerator does nothing.
    pub enabled: bool,
    /// Choosing the row opens another menu of the host's, which the host
    /// opens when it is handed the row. The keys after the choice in its
    /// batch of events, typed before the user could see that menu, are
    /// dropped, as after a handover of the keyboard (see the module's
    /// description).
    pub opens_menu: bool,
    /// What choosing the row opens is a dialog of the host's
    /// ([`Row::opens_dialog`]): an Escape after the choice in its batch is
    /// dropped too, as after a dialog's request
    /// ([`MenuSource::holds_escape`]).
    pub opens_dialog: bool,
    /// Among suggestions, a row Enter does not choose as it is offered:
    /// while it is selected and the selection has not been moved with Up
    /// or Down, Enter closes the suggestions and is the text's, as with
    /// none open. Tab and the pointer still choose it. For suggestions
    /// that may be ordinary words, so that a line ending in one is never
    /// rewritten by the Enter that ends it.
    pub tentative: bool,
    /// Among suggestions, a row chosen only from the suggestions it was
    /// offered in, as a row that runs a command is: Enter and Tab on
    /// suggestions with such a row name them ([`PopoverEvent::AcceptOf`]),
    /// so a key queued while the host replaces them chooses nothing. For
    /// rows whose order a refresh may change, such as owners ranked by use.
    pub bound: bool,
    /// A short note drawn muted at the row's right, where a menu names a
    /// row's key: a date, how often a name is used. A row's key, if it
    /// has one, is drawn there instead.
    pub detail: Option<String>,
}

impl Row {
    pub fn new(id: u32, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            accelerator: None,
            kind: RowKind::Own,
            glyph: None,
            checked: false,
            enabled: true,
            opens_menu: false,
            opens_dialog: false,
            tentative: false,
            bound: false,
            detail: None,
        }
    }

    /// A row that runs the host command `id` names.
    pub fn command(id: u32, label: impl Into<String>) -> Self {
        Self {
            kind: RowKind::Command,
            ..Self::new(id, label)
        }
    }

    pub fn accelerator(mut self, accelerator: Accelerator) -> Self {
        self.accelerator = Some(accelerator);
        self
    }

    pub fn glyph(mut self, glyph: impl Into<String>) -> Self {
        self.glyph = Some(glyph.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// A row whose choice opens another menu of the host's
    /// ([`Row::opens_menu`]).
    pub fn opens_menu(mut self) -> Self {
        self.opens_menu = true;
        self
    }

    /// A row whose choice opens a dialog of the host's ([`Row::opens_dialog`]),
    /// which takes the keyboard as another menu does ([`Row::opens_menu`]).
    pub fn opens_dialog(mut self) -> Self {
        self.opens_menu = true;
        self.opens_dialog = true;
        self
    }

    /// A suggestion Enter does not choose as it is offered
    /// ([`Row::tentative`]).
    pub fn tentative(mut self, tentative: bool) -> Self {
        self.tentative = tentative;
        self
    }

    /// A suggestion chosen only from the suggestions it was offered in
    /// ([`Row::bound`]).
    pub fn bound(mut self) -> Self {
        self.bound = true;
        self
    }

    /// A note drawn at the row's right ([`Row::detail`]).
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// A row as an open popover keeps it, but for its label and its key: its
/// id and what choosing it does, its glyph, whether it is checked and
/// whether it can be chosen.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Entry {
    id: u32,
    kind: RowKind,
    glyph: Option<String>,
    checked: bool,
    enabled: bool,
    opens_menu: bool,
    opens_dialog: bool,
    tentative: bool,
    bound: bool,
    detail: Option<String>,
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
    accelerators: Vec<Option<Accelerator>>,
    /// The rest of each row.
    entries: Vec<Entry>,
    selected: Option<usize>,
    /// The suggestions menu, whose word is tinted while it is open and
    /// which leaves Escape to the host's key handling.
    spelling: bool,
    /// The provider of the host's list that answered the right-click this
    /// opened for.
    provider: Option<usize>,
    /// Which of the popovers the editor has opened this is; see
    /// [`Self::instance`].
    instance: u64,
    /// Up or Down has moved the selection since it opened: Enter then
    /// chooses even a tentative row ([`Row::tentative`]).
    confirmed: bool,
}

impl Popover {
    /// A popover under `range` of `line`, which reads `text`, offering
    /// `rows`: the first checked row that can be chosen selected, or with
    /// none the first that can be.
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
            accelerators: Vec::with_capacity(rows.len()),
            entries: Vec::with_capacity(rows.len()),
            selected: None,
            spelling: false,
            provider: None,
            instance: 0,
            confirmed: false,
        };
        for row in rows {
            let enabled = row.enabled;
            popover.labels.push(row.label);
            popover.accelerators.push(
                row.accelerator
                    .map(|key| if enabled { key } else { key.inert() }),
            );
            popover.entries.push(Entry {
                id: row.id,
                kind: row.kind,
                glyph: row.glyph,
                checked: row.checked,
                enabled,
                opens_menu: row.opens_menu,
                opens_dialog: row.opens_dialog,
                tentative: row.tentative,
                bound: row.bound,
                detail: row.detail,
            });
        }
        let entries = &popover.entries;
        let live = |row: &usize| entries[*row].enabled;
        let checked = (0..entries.len()).find(|row| live(row) && entries[*row].checked);
        popover.selected = checked.or_else(|| (0..entries.len()).find(live));
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

    /// A menu the provider at `index` of the host's list answered a
    /// right-click with.
    pub(crate) fn provided_by(mut self, index: usize) -> Self {
        self.provider = Some(index);
        self
    }

    /// The place in the host's list of the [`MenuProvider`] whose answer to
    /// a right-click this is; `None` for any other popover.
    pub fn provider(&self) -> Option<usize> {
        self.provider
    }

    /// The popover as the `instance`th the editor opens.
    pub(crate) fn opened_as(mut self, instance: u64) -> Self {
        self.instance = instance;
        self
    }

    /// A number no other popover the editor opens has: each one it opens
    /// gets the next. What a queued choice of a row names its menu by
    /// ([`PopoverEvent::AcceptOf`]).
    pub fn instance(&self) -> u64 {
        self.instance
    }

    /// Move the selection `delta` rows that can be chosen, stopping at the
    /// ends. A move that moved it picks from the rows, so Enter chooses the
    /// one selected ([`Self::takes_enter`]); one stopped at an end picks
    /// nothing.
    pub(crate) fn step(&mut self, delta: i32) {
        let was = self.selected;
        self.selected = stepped(self.selected, self.labels.len(), &self.live(), delta);
        self.confirmed |= self.selected != was;
    }

    /// Whether Enter chooses its selected row: always on a menu; among
    /// suggestions, unless that row is tentative ([`Row::tentative`]) and
    /// the selection has not been moved with Up or Down since they opened.
    pub fn takes_enter(&self) -> bool {
        self.mode == Mode::Menu
            || self.confirmed
            || self
                .selected
                .is_none_or(|row| !self.entries.get(row).is_some_and(|entry| entry.tentative))
    }

    /// Whether Up or Down has moved the selection since it opened.
    pub(crate) fn is_confirmed(&self) -> bool {
        self.confirmed
    }

    /// Whether `row` is drawn as tentative: a tentative row of suggestions
    /// whose selection Up or Down has not moved, which Enter does not
    /// choose ([`Self::takes_enter`]).
    pub fn is_tentative(&self, row: usize) -> bool {
        self.mode == Mode::Suggest
            && !self.confirmed
            && self.entries.get(row).is_some_and(|entry| entry.tentative)
    }

    /// The hint drawn under the rows: for suggestions with a tentative row,
    /// how to choose one, since Enter alone does not.
    pub fn footer(&self) -> Option<&'static str> {
        (self.mode == Mode::Suggest && self.entries.iter().any(|entry| entry.tentative))
            .then_some(TENTATIVE_FOOTER)
    }

    /// Select `row`, if the popover has it and it can be chosen.
    pub(crate) fn hover(&mut self, row: usize) {
        self.selected = hovered(self.selected, self.labels.len(), &self.live(), row);
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
        self.entries.get(row).map(|entry| entry.id)
    }

    /// What choosing `row` does, if the popover has it.
    pub fn kind(&self, row: usize) -> Option<RowKind> {
        self.entries.get(row).map(|entry| entry.kind)
    }

    /// Whether a row of it runs a command of the host's
    /// ([`RowKind::Command`]): a choice of one names the popover by its
    /// [`instance`](Self::instance), as a provider's menu's does.
    pub fn runs_commands(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.kind == RowKind::Command)
    }

    /// Whether these are suggestions whose Enter and Tab choose only by
    /// naming them ([`PopoverEvent::AcceptOf`]): those with a row that runs
    /// a command or is bound to them ([`Row::bound`]).
    pub(crate) fn binds_its_keys(&self) -> bool {
        self.mode == Mode::Suggest
            && (self.entries.iter()).any(|entry| entry.kind == RowKind::Command || entry.bound)
    }

    /// Whether `row` can be chosen and opens a menu of the host's
    /// ([`Row::opens_menu`]).
    pub(crate) fn opens_menu_row(&self, row: usize) -> bool {
        (self.entries.get(row)).is_some_and(|entry| entry.enabled && entry.opens_menu)
    }

    /// Whether `row` is chosen only by a choice that names the popover:
    /// a suggestion that runs a command or is bound ([`Row::bound`]).
    pub(crate) fn wants_its_name(&self, row: usize) -> bool {
        self.mode == Mode::Suggest
            && (self.entries.get(row))
                .is_some_and(|entry| entry.kind == RowKind::Command || entry.bound)
    }

    /// Each row's accelerator, inert for a row that cannot be chosen.
    pub(crate) fn accelerators(&self) -> &[Option<Accelerator>] {
        &self.accelerators
    }

    /// The key named beside `row` ([`Accelerator::label`]), if it has one.
    pub fn hint(&self, row: usize) -> Option<String> {
        self.accelerators
            .get(row)
            .copied()
            .flatten()
            .map(Accelerator::label)
    }

    /// The note drawn at `row`'s right ([`Row::detail`]), if it has one.
    pub fn detail(&self, row: usize) -> Option<&str> {
        self.entries
            .get(row)
            .and_then(|entry| entry.detail.as_deref())
    }

    /// What is drawn at `row`'s right: its key's name, or else its note.
    pub(crate) fn note(&self, row: usize) -> Option<String> {
        self.hint(row)
            .or_else(|| self.detail(row).map(str::to_owned))
    }

    /// The glyph drawn before `row`'s label ([`Row::glyph`]).
    pub fn glyph(&self, row: usize) -> Option<&str> {
        self.entries
            .get(row)
            .and_then(|entry| entry.glyph.as_deref())
    }

    /// Whether `row` is checked ([`Row::checked`]).
    pub fn is_checked(&self, row: usize) -> bool {
        self.entries.get(row).is_some_and(|entry| entry.checked)
    }

    /// Whether `row` is one the popover has that can be chosen
    /// ([`Row::enabled`]).
    pub fn is_enabled(&self, row: usize) -> bool {
        self.entries.get(row).is_some_and(|entry| entry.enabled)
    }

    /// Whether each row can be chosen, in order; empty when every row can,
    /// which is what a row past the list's end is taken as.
    pub(crate) fn live(&self) -> Vec<bool> {
        if self.entries.iter().all(|entry| entry.enabled) {
            return Vec::new();
        }
        self.entries.iter().map(|entry| entry.enabled).collect()
    }

    /// Whether a choice of one of its rows names it
    /// ([`PopoverEvent::AcceptOf`]), so that a choice waiting to be applied
    /// is never made from another popover: every menu but the suggestions
    /// menu, and suggestions with a row that runs a command or is bound
    /// ([`Self::binds_its_keys`]).
    pub(crate) fn named_by_choice(&self) -> bool {
        !self.spelling
            && (self.mode == Mode::Menu || self.provider.is_some() || self.binds_its_keys())
    }

    /// The anchor, when its text is tinted: the suggestions menu's word
    /// (rule 4.4.2).
    pub(crate) fn tint(&self) -> Option<(usize, Range<usize>)> {
        self.spelling.then(|| (self.line, self.range.clone()))
    }
}

/// The hint under suggestions with a tentative row ([`Popover::footer`]).
pub const TENTATIVE_FOOTER: &str = "Tab picks · ↑↓ ↵";

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

    /// [`SuggestSource::suggest`], told also the number of the caret's
    /// line in the text as the host has it, for a source that knows more
    /// of that text than the line: whether the whole text reads it as a
    /// task's, say. It must answer the same for the same line number,
    /// line and caret. What the editor asks; by default,
    /// [`SuggestSource::suggest`]'s answer.
    fn suggest_on(&self, index: usize, line: &str, caret: usize) -> Option<Suggestion> {
        let _ = index;
        self.suggest(line, caret)
    }
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

    /// [`MenuSource::menu`], told also the number of the caret's line in
    /// the text as the host has it, for a source that knows more of that
    /// text than the line: what the line is a part of, say. It must answer
    /// the same for the same request, line number, line and caret. What
    /// the editor asks; by default, [`MenuSource::menu`]'s answer.
    fn menu_on(&self, id: u32, index: usize, line: &str, caret: usize) -> Option<Requested> {
        let _ = index;
        self.menu(id, line, caret)
    }

    /// Whether request `id` is a dialog's, for which the host takes the
    /// keyboard whatever the line says, on either engine: the rest of its
    /// batch is dropped as after [`Requested::Keyboard`], and so is an
    /// Escape in it, which a dialog must not close before it shows,
    /// published as [`PopoverEvent::Stroke`] so the host's key handling
    /// drops its copy. By default no request is.
    fn holds_escape(&self, id: u32) -> bool {
        let _ = id;
        false
    }

    /// Whether what request `id` opens, where the source answers it by
    /// taking the keyboard ([`Requested::Keyboard`]), holds Escape as a
    /// dialog does ([`MenuSource::holds_escape`]): an Escape after it in
    /// its batch is dropped, published as [`PopoverEvent::Stroke`]. Where
    /// the source does not take the keyboard, the keys after the request
    /// are left as they are. By default no request is.
    fn holds_escape_when_taken(&self, id: u32) -> bool {
        let _ = id;
        false
    }
}

/// A host's source of menus on the engine that opens no popovers: it opens
/// nothing, but a dialog's request ([`MenuSource::holds_escape`]) takes the
/// keyboard as on the other engine.
pub(crate) struct KeyboardOnly(pub(crate) Arc<dyn MenuSource>);

impl MenuSource for KeyboardOnly {
    fn menu(&self, _id: u32, _line: &str, _caret: usize) -> Option<Requested> {
        None
    }

    fn holds_escape(&self, id: u32) -> bool {
        self.0.holds_escape(id)
    }
}

/// A [`MenuSource`]'s answer, or a [`MenuProvider`]'s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requested {
    /// A menu of these rows, hung under `range` of the line asked about:
    /// the caret's, or the one under a right-click.
    Rows { range: Range<usize>, rows: Vec<Row> },
    /// Something of the host's own that takes the keyboard, a field say,
    /// which the host opens, and gives the keyboard, when it is handed the
    /// request or the right-click ([`PopoverEvent::Context`], then
    /// [`crate::EditorView::take_keyboard`]). What is typed or composed
    /// after it in its batch of events is dropped: it reaches neither the
    /// note nor the host (see the module's description).
    Keyboard,
}

/// What a right-click fell on: what a [`MenuProvider`] is asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clicked {
    /// The line of the row under the pointer.
    pub line: usize,
    /// The line's text, without its ending.
    pub text: String,
    /// The line's typed spans: what says, of a place outside every one of
    /// them, what kind of line it is on.
    pub spans: LineSpans,
    /// The byte of the line at which the glyph under the pointer starts.
    /// `None` where the row has no glyph under the pointer: past the end
    /// of its text, or left of it.
    pub column: Option<usize>,
    /// What lies there: a misspelled word before one of the line's typed
    /// spans ([`Hit`]). `None` outside every span.
    pub hit: Option<Hit>,
}

/// What the host gives the editor to open a menu of its own for a
/// right-click; see [`Provider`]. It must answer the same for the same
/// place, for the reason a [`SuggestSource`] must: the widget asks it
/// while it routes a batch of events, and the editor asks it again when
/// the host hands it the right-click.
pub trait MenuProvider: Send + Sync {
    /// What a right-click on `clicked` opens. `None` passes the click to
    /// the next provider, as an answer that opens nothing does: a menu
    /// with no rows, or one whose range is off the line.
    fn menu(&self, clicked: &Clicked) -> Option<Requested>;

    /// Whether what this provider opens by taking the keyboard
    /// ([`Requested::Keyboard`]) holds Escape as a dialog does
    /// ([`MenuSource::holds_escape`]): an Escape after the right-click in
    /// its batch is dropped, published as [`PopoverEvent::Stroke`]. By
    /// default no provider's is.
    fn holds_escape(&self) -> bool {
        false
    }

    /// [`Self::menu`], for a click on a line of a task's text after the
    /// line of its box, which `task` is, as the whole text reads it; with
    /// the host's tasks off, on the box's own line and off every task,
    /// `task` is `None`. The menu still hangs on the line clicked. Asks
    /// [`Self::menu`] unless a provider answers for such lines.
    fn menu_in_task(&self, clicked: &Clicked, task: Option<&TaskBox>) -> Option<Requested> {
        let _ = task;
        self.menu(clicked)
    }
}

/// The line of a task's box, for a right-click on a line of the task's
/// text after it ([`MenuProvider::menu_in_task`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBox {
    pub line: usize,
    /// The line's text, without its ending.
    pub text: String,
    /// The line's typed spans, its box among them.
    pub spans: LineSpans,
}

/// One of the providers a right-click asks, in the order of the host's
/// list ([`crate::EditorView::set_menu_providers`]). The first whose
/// answer opens something takes the click.
#[derive(Clone)]
pub enum Provider {
    /// The editor's own: the suggestions menu on a misspelled word, from
    /// the host's [`crate::Proofreader`]. It takes every misspelled word,
    /// so it comes first unless the host puts a provider before it; a list
    /// without it opens no suggestions menu for a right-click.
    Spelling,
    /// One of the host's.
    Host(Arc<dyn MenuProvider>),
}

impl Provider {
    /// Whether `self` and `other` are the same provider: the editor's
    /// own, or one object of the host's.
    pub(crate) fn is(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Spelling, Self::Spelling) => true,
            (Self::Host(one), Self::Host(other)) => {
                std::ptr::addr_eq(Arc::as_ptr(one), Arc::as_ptr(other))
            }
            _ => false,
        }
    }
}

/// Whether the suggestions menu comes before every provider of the host's
/// in `providers`, so that it answers a right-click on a misspelled word
/// with no other provider asked first. Not in a list without it.
pub(crate) fn spelling_leads(providers: &[Provider]) -> bool {
    matches!(providers.first(), Some(Provider::Spelling))
}

/// The list a host that names none has: the suggestions menu alone.
pub(crate) static SPELLING_ALONE: [Provider; 1] = [Provider::Spelling];

/// The text and the list of providers a right-click was asked about: the
/// editor's, as the widget was built over them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Edition {
    /// [`crate::EditorView::revision`] of the text.
    pub revision: u64,
    /// Which list of providers the host had named; it changes with the
    /// list.
    pub list: u64,
    /// How many popovers the editor had opened: the
    /// [`Popover::instance`] of the last, and one less than the next's.
    pub opened: u64,
}

/// What the widget saw of a right-click that a provider of the host's
/// took, carried by [`PopoverEvent::Context`]: the text, the list of
/// providers, the popovers opened so far, the place clicked, and which
/// provider answered what for it. The editor opens nothing for a click it
/// finds otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    edition: Edition,
    /// The place and the answer ([`fingerprint`]).
    answer: u64,
}

impl Seen {
    pub(crate) fn new(edition: Edition, answer: u64) -> Self {
        Self { edition, answer }
    }

    /// Whether the click was made over this text and this list of
    /// providers, with no popover opened since.
    pub(crate) fn over(self, edition: Edition) -> bool {
        self.edition == edition
    }

    /// Whether the click was on the place, and answered with the answer,
    /// that `print` is of.
    pub(crate) fn answered(self, print: u64) -> bool {
        self.answer == print
    }
}

/// A number that stands for the place a right-click fell on, for the
/// provider at `provider` of the host's list, and for what it answered:
/// two that differ in the line, its text, the glyph, the hit, the
/// provider, or the menu's anchor or any of its rows have different
/// numbers, but for chance.
pub(crate) fn fingerprint(provider: usize, clicked: &Clicked, answer: &Answer) -> u64 {
    let mut hasher = DefaultHasher::new();
    (provider, clicked.line, &clicked.text, clicked.column).hash(&mut hasher);
    match &clicked.hit {
        None => 0_u8.hash(&mut hasher),
        Some(Hit::Misspelling(word)) => (1_u8, word).hash(&mut hasher),
        // A span's kind is read from the line's text, which is hashed.
        Some(Hit::Span(span)) => (2_u8, &span.range).hash(&mut hasher),
    }
    match answer {
        Answer::Nothing => 0_u8.hash(&mut hasher),
        Answer::Keyboard => 1_u8.hash(&mut hasher),
        Answer::Menu(menu) => {
            (2_u8, menu.line, &menu.range, &menu.text).hash(&mut hasher);
            (&menu.labels, &menu.entries, &menu.accelerators).hash(&mut hasher);
            (menu.selected, menu.provider).hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Who takes a right-click; see [`taken`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Taken {
    /// The suggestions menu.
    Spelling,
    /// One of the host's providers.
    Host {
        /// Its place in the host's list.
        provider: usize,
        /// What it opens: a menu, or the host's own with the keyboard.
        answer: Answer,
        /// The place clicked and the answer ([`fingerprint`]).
        fingerprint: u64,
    },
}

/// The first of `providers` that takes a right-click. `misspelled` says
/// whether a misspelled word is under the pointer, which is all the
/// suggestions menu takes, and `clicked` what is there for the host's
/// providers, each looked for only when a provider that reads it is asked.
/// A provider after the one that takes the click is not asked.
#[cfg(test)]
pub(crate) fn taken(
    providers: &[Provider],
    misspelled: impl FnOnce() -> bool,
    clicked: impl FnOnce() -> Option<Clicked>,
) -> Option<Taken> {
    taken_in(providers, misspelled, || {
        clicked().map(|clicked| (clicked, None))
    })
}

/// [`taken`], with what `clicked` finds the task the clicked line belongs
/// to after its box ([`MenuProvider::menu_in_task`]).
pub(crate) fn taken_in(
    providers: &[Provider],
    misspelled: impl FnOnce() -> bool,
    clicked: impl FnOnce() -> Option<(Clicked, Option<TaskBox>)>,
) -> Option<Taken> {
    let (mut misspelled, mut clicked) = (Some(misspelled), Some(clicked));
    let (mut word, mut place) = (false, None);
    for (index, provider) in providers.iter().enumerate() {
        match provider {
            Provider::Spelling => {
                if let Some(misspelled) = misspelled.take() {
                    word = misspelled();
                }
                if word {
                    return Some(Taken::Spelling);
                }
            }
            Provider::Host(provider) => {
                if let Some(clicked) = clicked.take() {
                    place = clicked();
                }
                let Some((place, task)) = &place else {
                    continue;
                };
                let answer = provided_in(&**provider, index, place, task.as_ref());
                if answer != Answer::Nothing {
                    return Some(Taken::Host {
                        provider: index,
                        fingerprint: fingerprint(index, place, &answer),
                        answer,
                    });
                }
            }
        }
    }
    None
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
    let suggestion = source.suggest_on(line.index, &line.text, line.caret)?;
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
    answered(
        source.menu_on(id, line.index, &line.text, line.caret),
        line.index,
        &line.text,
    )
}

/// What `provider`, the one at `index` of the host's list, opens for a
/// right-click on `clicked`: a menu of its rows, checked as a request's
/// are ([`requested`]). The widget and the editor both ask through here.
#[cfg(test)]
pub(crate) fn provided(provider: &dyn MenuProvider, index: usize, clicked: &Clicked) -> Answer {
    provided_in(provider, index, clicked, None)
}

/// [`provided`], for a click on a line of the text of the task `task`
/// after its box ([`MenuProvider::menu_in_task`]).
pub(crate) fn provided_in(
    provider: &dyn MenuProvider,
    index: usize,
    clicked: &Clicked,
    task: Option<&TaskBox>,
) -> Answer {
    match answered(
        provider.menu_in_task(clicked, task),
        clicked.line,
        &clicked.text,
    ) {
        Answer::Menu(menu) => Answer::Menu(menu.provided_by(index)),
        answer => answer,
    }
}

/// What a source's or a provider's `answer` about line `index`, which
/// reads `text`, opens.
fn answered(answer: Option<Requested>, index: usize, text: &str) -> Answer {
    match answer {
        None => Answer::Nothing,
        Some(Requested::Keyboard) => Answer::Keyboard,
        Some(Requested::Rows { range, rows }) => text
            .get(range.clone())
            .filter(|_| !rows.is_empty())
            .map_or(Answer::Nothing, |anchored| {
                let anchored = anchored.to_owned();
                Answer::Menu(Popover::new(Mode::Menu, index, range, anchored, rows))
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
    /// A right-click at `at` that the host's [`MenuProvider`] at
    /// `provider` of its list takes: whatever popover was open closes, and
    /// that provider is asked what to open for what lies there. Its menu
    /// opens only if the text, the host's list of providers, the place
    /// and the answer are all as the widget saw them (`seen`), and a
    /// provider that takes the keyboard has it on the same terms
    /// ([`crate::EditorView::take_keyboard`]).
    Context {
        provider: usize,
        at: Point,
        seen: Seen,
    },
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
    /// Choose `row` of the popover that is the `menu`th the editor opened
    /// ([`Popover::instance`]), or its selected row for no `row`; the
    /// editor does nothing if another popover is open by then, or none. A
    /// row of a menu that one of the host's providers opened is chosen
    /// this way, by its accelerator, Enter or the pointer, so that a
    /// choice waiting to be applied is never made from another menu.
    AcceptOf {
        menu: u64,
        row: Option<usize>,
    },
    /// Enter on suggestions the widget opened in its own batch, which the
    /// host had not shown, whose selected row it found to be the one with
    /// this id, a row that opens a menu of the host's ("Due date…", which
    /// writes nothing until a date is picked). The editor chooses that row
    /// if the suggestions it opened have it selected; otherwise Enter is
    /// the text's, as with nothing open, and chooses nothing.
    EnterOn {
        id: u32,
    },
    /// A press of one of a popover's keys with no modifier: of
    /// suggestions' while they are open, or of the suggestions menu's after
    /// a request for it earlier in the same batch of events, which may not
    /// have opened it. An open popover takes the key as its mode does;
    /// with none open the editor handles the press as it would any other.
    Key(MenuKey),
    /// A key the field of a host that took the keyboard
    /// ([`Requested::Keyboard`]) would take: the key and what it types.
    /// The widget publishes none: in the batch of a handover it drops
    /// those keys (see the module's description). The editor does nothing
    /// with one handed to it.
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
    /// A right press, which opens the menu on a misspelled word. No
    /// provider of the host's answers it: the widget, which leaves the
    /// press to the host, did not ask them.
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

/// A key press that the field of a host that took the keyboard would
/// take ([`PopoverEvent::Stroke`]): the key and what it types.
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
    /// or a right-click of this batch ([`Requested::Keyboard`]).
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

/// Whether `row` can be chosen, of rows of which `live` says so in order
/// ([`Popover::live`]): a row past its end can.
fn is_live(live: &[bool], row: usize) -> bool {
    live.get(row).copied().unwrap_or(true)
}

/// `selected` moved `delta` rows among `items`, stopping at the ends; a
/// popover with nothing selected stays so. The rows that `live` says
/// cannot be chosen are stepped over, and the selection stays where it is
/// when only such rows lie the way it moves.
fn stepped(selected: Option<usize>, items: usize, live: &[bool], delta: i32) -> Option<usize> {
    let selected = selected?;
    let last = items.saturating_sub(1);
    let moved = i64::try_from(selected)
        .unwrap_or(i64::MAX)
        .saturating_add(i64::from(delta));
    let target = usize::try_from(moved.max(0)).map_or(last, |row| row.min(last));
    if is_live(live, target) {
        return Some(target);
    }
    // The nearest row that can be chosen past the target, the way the
    // selection moves, or else short of it.
    let live = |row: &usize| is_live(live, *row);
    let found = if target > selected {
        (target..=last)
            .find(live)
            .or_else(|| (selected..target).rev().find(live))
    } else {
        (0..=target)
            .rev()
            .find(live)
            .or_else(|| (target + 1..=selected).find(live))
    };
    Some(found.unwrap_or(selected))
}

/// The selection once the pointer is over `row`: that row, if it is one of
/// `items` that can be chosen.
fn hovered(selected: Option<usize>, items: usize, live: &[bool], row: usize) -> Option<usize> {
    if row < items && is_live(live, row) {
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

/// [`after`], for a popover whose rows `live` says can be chosen or not
/// ([`Popover::live`]): a move steps over those that cannot, and a hover
/// does not select them.
pub(crate) fn after_with(
    open: Option<(Opened, OpenMenu)>,
    message: &Message,
    live: &[bool],
) -> Option<(Opened, OpenMenu)> {
    match message {
        Message::Spell(PopoverEvent::Move(delta)) => open.map(|(opened, menu)| {
            let selected = stepped(menu.selected, menu.items, live, *delta);
            (opened, OpenMenu { selected, ..menu })
        }),
        Message::Spell(PopoverEvent::Hover(row)) => open.map(|(opened, menu)| {
            let selected = hovered(menu.selected, menu.items, live, *row);
            (opened, OpenMenu { selected, ..menu })
        }),
        Message::Spell(PopoverEvent::Key(key @ (MenuKey::Up | MenuKey::Down))) => match open?.0 {
            Opened::Keyboard => None,
            opened => {
                let mode = if opened == Opened::Suggest {
                    Mode::Suggest
                } else {
                    Mode::Menu
                };
                after_with(open, &Message::Spell(key.on(mode)?), live)
            }
        },
        _ => after(open, message),
    }
}

/// The popover `message` leaves `open` as, once the host applies it, and
/// what it is, for every message whose effect the message alone decides.
/// (A request for a popover of the host's and a character typed open what
/// the host's sources say, which [`Now::publish`] asks, and a right-click
/// a provider of the host's takes opens what the provider answered
/// ([`Now::publish_provided`]).)
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
            let selected = stepped(menu.selected, menu.items, &[], *delta);
            (opened, OpenMenu { selected, ..menu })
        }),
        Message::Spell(PopoverEvent::Hover(row)) => open.map(|(opened, menu)| {
            let selected = hovered(menu.selected, menu.items, &[], *row);
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
    /// The host had a popover open when the batch started.
    handed: Cell<bool>,
    /// Every message of the batch so far left the text, the caret and the
    /// scroll as the host has them; see [`Self::sees_the_text`].
    still: Cell<bool>,
    /// How many popovers the editor will have opened once it applies the
    /// batch's messages so far ([`Popover::instance`] of the last); `None`
    /// once a message may or may not have opened one.
    count: Cell<Option<u64>>,
    /// The instance the editor will give `own` when it opens it, when
    /// [`Self::count`] is known.
    own_instance: Cell<Option<u64>>,
    /// Whether each row of what is open can be chosen
    /// ([`Popover::live`]).
    live: RefCell<Vec<bool>>,
    /// Up or Down has moved the selection of what is open since it
    /// opened ([`Popover::takes_enter`]).
    confirmed: Cell<bool>,
    /// The host took the keyboard for something that holds Escape too: a
    /// dialog's request, or a row of suggestions or a dialog's row chosen
    /// in this batch.
    holds_escape: Cell<bool>,
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
            handed: Cell::new(false),
            still: Cell::new(true),
            count: Cell::new(None),
            own_instance: Cell::new(None),
            live: RefCell::new(Vec::new()),
            confirmed: Cell::new(false),
            holds_escape: Cell::new(false),
        }
    }

    /// Count the popovers the editor opens from `opened`, how many it had
    /// opened when the widget was built.
    pub(crate) fn count_from(&self, opened: u64) {
        self.count.set(Some(opened));
    }

    /// Start from `popover`, the one the host has open.
    pub(crate) fn start(&self, popover: Option<&Popover>) {
        self.handed.set(popover.is_some());
        self.confirmed
            .set(popover.is_some_and(Popover::is_confirmed));
        self.live
            .replace(popover.map_or_else(Vec::new, Popover::live));
        self.set(popover.map(|popover| (popover.mode().into(), OpenMenu::of(popover))));
    }

    /// Whether the widget sees the text as the host will have it when it
    /// applies the next message, laid out and scrolled as it is drawn: a
    /// right-click is then on what the host will find under it. Not once a
    /// message of the batch may have changed the text, moved the caret or
    /// scrolled, as the host scrolls to show what it opens. Nor once the
    /// popover the host had open has closed: a row chosen from it with the
    /// pointer closes it, which the overlay publishes and the widget does
    /// not see, and the host acts on the row.
    pub(crate) fn sees_the_text(&self) -> bool {
        self.still.get() && !(self.handed.get() && self.open.get().is_none())
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

    /// Whether choosing `row` of what is open, of the popover the host
    /// handed over (`handed`) or the one opened in this batch, or with no
    /// row its selected row, opens another menu of the host's
    /// ([`Row::opens_menu`]).
    pub(crate) fn opens_menu(&self, handed: Option<&Popover>, row: Option<usize>) -> bool {
        let Some(open) = self.open.get() else {
            return false;
        };
        let own = self.own.borrow();
        let row = row.or(open.selected);
        (own.as_ref().or(handed))
            .zip(row)
            .is_some_and(|(popover, row)| {
                popover.is_enabled(row) && popover.entries[row].opens_menu
            })
    }

    /// Whether choosing `row` (as for [`Now::opens_menu`]) opens a dialog
    /// of the host's ([`Row::opens_dialog`]).
    pub(crate) fn opens_dialog(&self, handed: Option<&Popover>, row: Option<usize>) -> bool {
        let Some(open) = self.open.get() else {
            return false;
        };
        let own = self.own.borrow();
        let row = row.or(open.selected);
        (own.as_ref().or(handed))
            .zip(row)
            .is_some_and(|(popover, row)| {
                popover.is_enabled(row) && popover.entries[row].opens_dialog
            })
    }

    /// Whether Enter chooses the selected row of what is open, of the
    /// popover the host handed over (`handed`) or the one opened in this
    /// batch ([`Popover::takes_enter`]), as the batch's moves leave it.
    pub(crate) fn takes_enter(&self, handed: Option<&Popover>) -> bool {
        let Some(open) = self.open.get() else {
            return true;
        };
        let own = self.own.borrow();
        let Some(popover) = own.as_ref().or(handed) else {
            return true;
        };
        popover.mode() == Mode::Menu
            || self.confirmed.get()
            || open.selected.is_none_or(|row| {
                !popover
                    .entries
                    .get(row)
                    .is_some_and(|entry| entry.tentative)
            })
    }

    /// The host will open a menu the widget cannot know of for a row
    /// chosen in this batch: from here to the end of the batch, the input
    /// is no one's, as after a handover of the keyboard; with `escape`,
    /// Escape too.
    pub(crate) fn hand_over(&self, escape: bool) {
        self.set(Some((Opened::Keyboard, OpenMenu::OPENING)));
        self.own.replace(None);
        self.holds_escape.set(escape);
    }

    /// Whether the host took the keyboard for something that holds Escape
    /// too ([`Self::hand_over`]).
    pub(crate) fn holds_escape(&self) -> bool {
        self.holds_escape.get()
    }

    /// What choosing row `row` of `handed`, the popover the host has open,
    /// hands the host: `None` when the row opens nothing of the host's, or
    /// whether what it opens holds Escape, as for the row's key: a dialog's
    /// row ([`Row::opens_dialog`]) or a row of suggestions ("Due date…").
    pub(crate) fn handover_for(&self, handed: &Popover, row: usize) -> Option<bool> {
        self.opens_menu(Some(handed), Some(row))
            .then(|| handed.mode() == Mode::Suggest || self.opens_dialog(Some(handed), Some(row)))
    }

    /// A row of the popover the host has open was chosen with the pointer,
    /// ahead of every event of the batch the widget routes ([`InOrder`]).
    /// The host acts on the row, which may change anything, as for a row
    /// chosen with a key ([`Self::publish`]): the caret's line, the text
    /// under the pointer and the popovers opened are not known after it.
    /// A row that opens a menu or a dialog of the host's (`handover`, from
    /// [`Self::handover_for`]) hands it the keyboard, as its key does.
    pub(crate) fn chosen(&self, handover: Option<bool>) {
        self.still.set(false);
        self.typed.replace(None);
        self.count.set(None);
        self.own_instance.set(None);
        self.confirmed.set(false);
        self.live.replace(Vec::new());
        self.own.replace(None);
        self.open.set(None);
        if let Some(escape) = handover {
            self.hand_over(escape);
        }
    }

    /// The [`Popover::instance`] of what is open, of the popover the host
    /// handed over (`handed`) or the one opened in this batch, when a
    /// choice of one of its rows names it ([`Popover::named_by_choice`]):
    /// what such a choice names it by.
    pub(crate) fn bound(&self, handed: Option<&Popover>) -> Option<u64> {
        let own = self.own.borrow();
        let open = self.open.get().and(own.as_ref().or(handed))?;
        if open.provider().is_some() {
            return Some(open.instance());
        }
        // Any other menu but the suggestions menu, and suggestions with a
        // row that runs a command, by the number the editor gave it or
        // will give it; with that not known, none.
        match &*own {
            _ if !open.named_by_choice() => None,
            Some(_) => self.own_instance.get(),
            None => Some(open.instance()),
        }
    }

    /// Whether what is open are suggestions the widget opened in this
    /// batch, which the host has not shown: its source may answer otherwise
    /// by the time it opens them. Tab on them is dropped, and Enter is the
    /// text's, but on a row that opens a menu ([`Self::predicted_menu_row`]).
    pub(crate) fn predicts_suggestions(&self) -> bool {
        self.open.get().is_some()
            && (self.own.borrow().as_ref()).is_some_and(|own| own.mode() == Mode::Suggest)
    }

    /// The id of the selected row of suggestions the widget opened in this
    /// batch, when it opens a menu of the host's ("Due date…") and Enter
    /// chooses it ([`Popover::takes_enter`]): Enter on it asks the host to
    /// choose that row ([`PopoverEvent::EnterOn`]).
    pub(crate) fn predicted_menu_row(&self) -> Option<u32> {
        if !self.takes_enter(None) {
            return None;
        }
        let row = self.open.get()?.selected?;
        let own = self.own.borrow();
        let own = own.as_ref().filter(|own| own.mode() == Mode::Suggest)?;
        (own.opens_menu_row(row)).then(|| own.id(row)).flatten()
    }

    /// Whether what is open are suggestions with a row that runs a command
    /// or is bound to them: Enter and Tab then choose from them only by
    /// naming them ([`PopoverEvent::AcceptOf`]).
    pub(crate) fn binds_its_keys(&self, handed: Option<&Popover>) -> bool {
        let own = self.own.borrow();
        let open = self.open.get().and(own.as_ref().or(handed));
        open.is_some_and(Popover::binds_its_keys)
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
        self.publish_provided(shell, message, start, sources, Answer::Nothing);
    }

    /// [`Self::publish`], for a right-click that a provider of the host's
    /// takes ([`PopoverEvent::Context`]): `provided` is what the widget
    /// found that provider to answer for the text as it is laid out, as
    /// the editor will open it, or else open nothing. The widget publishes
    /// such a click only while it sees the text as the host will
    /// ([`Self::sees_the_text`]).
    pub(crate) fn publish_provided(
        &self,
        shell: &mut iced::advanced::Shell<'_, Message>,
        message: Message,
        start: &dyn Fn() -> Option<CaretLine>,
        sources: Sources<'_>,
        provided: Answer,
    ) {
        let was = self.live();
        let known = was.is_some_and(|(_, menu)| !menu.opening);
        if !leaves_the_view(&message, known) {
            self.still.set(false);
        }
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
        } else if !keeps_the_text(&message, known) {
            self.typed.replace(None);
        }
        let line = || self.line(start);

        let mut next = after_with(was, &message, &self.live.borrow());
        let mut own = None;
        // Whether the editor may open a popover for the message that the
        // widget does not open the same, which leaves the count unknown.
        let mut unknown = !keeps_the_text(&message, known)
            || matches!(
                message,
                Message::Spell(
                    PopoverEvent::OpenAtCaret
                        | PopoverEvent::OpenAt(_)
                        | PopoverEvent::Press(MenuPress::Right(_))
                )
            );
        let mut answered = |answer| match answer {
            Answer::Menu(popover) => {
                let open = OpenMenu::of(&popover);
                own = Some(popover);
                Some((Opened::Menu, open))
            }
            Answer::Keyboard => Some((Opened::Keyboard, OpenMenu::OPENING)),
            Answer::Nothing => None,
        };
        match &message {
            Message::Spell(PopoverEvent::Request(id)) => {
                unknown = sources.menus.is_some() && line().is_none();
                let answer = (sources.menus.zip(line()))
                    .map_or(Answer::Nothing, |(source, line)| {
                        requested(source, *id, &line)
                    });
                next = answered(answer);
            }
            Message::Spell(PopoverEvent::Context { .. }) => {
                if let Answer::Menu(menu) = &provided {
                    self.count.set(Some(menu.instance()));
                }
                next = answered(provided);
            }
            Message::Action(Action::Edit(Edit::Insert(_))) if !held => {
                unknown = sources.suggestions.is_some() && line().is_none();
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
        // Up or Down that moves the selection of what is open picks from
        // its rows; stopped at an end, it picks nothing.
        let moved = was.map(|(_, menu)| menu.selected) != next.map(|(_, menu)| menu.selected);
        if kept
            && moved
            && matches!(
                message,
                Message::Spell(
                    PopoverEvent::Move(_) | PopoverEvent::Key(MenuKey::Up | MenuKey::Down)
                )
            )
        {
            self.confirmed.set(true);
        }
        if !kept {
            self.confirmed.set(false);
            let provided = matches!(message, Message::Spell(PopoverEvent::Context { .. }));
            let instance = match &own {
                Some(menu) if provided => Some(menu.instance()),
                Some(_) => self.count.get().map(|count| count + 1),
                None => None,
            };
            if own.is_some() && !provided {
                self.count.set(instance);
            }
            self.own_instance.set(instance.filter(|_| !unknown));
            self.live
                .replace(own.as_ref().map_or_else(Vec::new, Popover::live));
            self.own.replace(own);
        }
        if unknown {
            self.count.set(None);
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
            | PopoverEvent::Context { .. }
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
            | PopoverEvent::AcceptOf { .. }
            | PopoverEvent::EnterOn { .. }
            | PopoverEvent::Press(MenuPress::Left(..)) => false,
        },
        _ => false,
    }
}

/// Whether `message` leaves the text, the caret and the scroll where they
/// were, with `open` as [`keeps_the_text`] has it. A message that asks for
/// a popover does not: the host scrolls to show what it opens. Nor does a
/// stroke, which is the host's.
fn leaves_the_view(message: &Message, open: bool) -> bool {
    keeps_the_text(message, open)
        && !matches!(
            message,
            Message::Spell(
                PopoverEvent::OpenAtCaret
                    | PopoverEvent::OpenAt(_)
                    | PopoverEvent::Context { .. }
                    | PopoverEvent::Request(_)
                    | PopoverEvent::Stroke(_)
                    | PopoverEvent::Press(MenuPress::Right(_))
            )
        )
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
    fn enter_chooses_a_tentative_suggestion_only_once_the_selection_moved() {
        let rows = vec![
            Row::new(0, "Today").tentative(true).detail("Thu 19 Nov"),
            Row::new(1, "Tomorrow").tentative(true),
            Row::new(2, "Due date…"),
        ];
        let mut popover = Popover::new(Mode::Suggest, 0, 4..6, "to".into(), rows.clone());
        assert!(!popover.takes_enter());
        assert!(!popover.is_confirmed());
        // Drawn tentative, with the hint under it, a row's height taller.
        assert!(popover.is_tentative(0) && popover.is_tentative(1));
        assert!(!popover.is_tentative(2));
        assert_eq!(popover.footer(), Some(TENTATIVE_FOOTER));
        assert_eq!(height_of(&popover, 3), menu::height(3) + 24.0);
        // Up at the top or Down at the bottom moves nothing, and picks
        // nothing.
        popover.step(-1);
        assert!(!popover.takes_enter());
        let mut one = Popover::new(Mode::Suggest, 0, 4..6, "fr".into(), rows[..1].to_vec());
        one.step(1);
        assert_eq!(one.selected(), Some(0));
        assert!(!one.takes_enter());
        // Moved, even back where it was, the user has picked.
        popover.step(1);
        popover.step(-1);
        assert_eq!(popover.selected(), Some(0));
        assert!(popover.takes_enter());
        assert!(!popover.is_tentative(0));
        assert_eq!(popover.footer(), Some(TENTATIVE_FOOTER));
        // A hover is no pick.
        let mut popover = Popover::new(Mode::Suggest, 0, 4..6, "to".into(), rows.clone());
        popover.hover(1);
        assert!(!popover.takes_enter());
        popover.hover(2);
        assert!(popover.takes_enter());
        // A menu takes Enter whatever its rows say, and has no hint.
        let menu = Popover::new(Mode::Menu, 0, 4..6, "to".into(), rows.clone());
        assert!(menu.takes_enter() && !menu.is_tentative(0));
        assert_eq!(
            (menu.footer(), height_of(&menu, 3)),
            (None, menu::height(3))
        );
        // A note is drawn where a key would be, and a key wins.
        let popover = Popover::new(Mode::Suggest, 0, 4..6, "to".into(), rows);
        assert_eq!(popover.detail(0), Some("Thu 19 Nov"));
        assert_eq!(popover.note(0).as_deref(), Some("Thu 19 Nov"));
        assert_eq!((popover.detail(1), popover.note(1)), (None, None));
        let keyed = Row::new(0, "Open")
            .accelerator(Accelerator::plain(' '))
            .detail("now");
        let menu = Popover::new(Mode::Menu, 0, 2..5, "[ ]".into(), vec![keyed]);
        assert_eq!(menu.note(0).as_deref(), Some("Space"));
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

    // ── A right-click's providers ───────────────────────────────────────────

    /// A place on the line `text`, in no span.
    fn place(text: &str, column: Option<usize>) -> Clicked {
        Clicked {
            line: 3,
            text: text.to_owned(),
            spans: LineSpans::default(),
            column,
            hit: None,
        }
    }

    fn host(answer: fn(&Clicked) -> Option<Requested>) -> (Arc<fixtures::Provides>, Provider) {
        let provides = Arc::new(fixtures::Provides::new(answer));
        (provides.clone(), Provider::Host(provides))
    }

    #[test]
    fn a_providers_answer_is_checked_as_a_requests_is_and_names_its_provider() {
        let task = place("- [ ] call", Some(7));
        let Answer::Menu(menu) =
            provided(&fixtures::Provides::new(fixtures::on_task_lines), 4, &task)
        else {
            panic!("no menu");
        };
        assert_eq!(
            (menu.mode(), menu.line(), menu.range(), menu.text()),
            (Mode::Menu, 3, 2..5, "[ ]")
        );
        assert_eq!((menu.provider(), menu.labels().len()), (Some(4), 3));
        assert!(!menu.is_spelling());
        // A popover opened any other way names none.
        assert_eq!(
            Popover::new(Mode::Menu, 0, 0..1, "a".into(), rows(&["a"])).provider(),
            None
        );
        // No answer, no rows, or a range off the line or inside a
        // character open nothing; the keyboard is the host's to take.
        fn no_rows(_: &Clicked) -> Option<Requested> {
            Some(Requested::Rows {
                range: 0..1,
                rows: Vec::new(),
            })
        }
        fn off_the_line(clicked: &Clicked) -> Option<Requested> {
            let end = clicked.text.len() + 1;
            let range = if clicked.text.is_ascii() {
                0..end
            } else {
                0..1
            };
            Some(Requested::Rows {
                range,
                rows: vec![Row::new(1, "a")],
            })
        }
        let ask =
            |answer, clicked: &Clicked| provided(&fixtures::Provides::new(answer), 0, clicked);
        assert_eq!(
            ask(fixtures::on_task_lines, &place("call", Some(1))),
            Answer::Nothing
        );
        assert_eq!(ask(no_rows, &task), Answer::Nothing);
        assert_eq!(ask(off_the_line, &task), Answer::Nothing);
        assert_eq!(ask(off_the_line, &place("é", Some(0))), Answer::Nothing);
        assert_eq!(
            ask(fixtures::off_task_lines, &place("call", None)),
            Answer::Keyboard
        );
    }

    #[test]
    fn the_first_provider_whose_answer_opens_something_takes_a_right_click() {
        use std::cell::Cell;
        fn no_rows(_: &Clicked) -> Option<Requested> {
            Some(Requested::Rows {
                range: 0..1,
                rows: Vec::new(),
            })
        }
        // What each case looked for: the word, and the place.
        let looked = (Cell::new(0), Cell::new(0));
        let ask = |providers: &[Provider], word: bool, text: Option<&str>| {
            looked.0.set(0);
            looked.1.set(0);
            let misspelled = || {
                looked.0.set(looked.0.get() + 1);
                word
            };
            let clicked = || {
                looked.1.set(looked.1.get() + 1);
                text.map(|text| place(text, Some(0)))
            };
            let taken = taken(providers, misspelled, clicked).map(|taken| match taken {
                Taken::Spelling => (None, None),
                Taken::Host {
                    provider,
                    answer,
                    fingerprint: found,
                } => {
                    // The fingerprint is of the place asked about and
                    // the answer given.
                    let place = place(text.unwrap(), Some(0));
                    assert_eq!(found, fingerprint(provider, &place, &answer));
                    match answer {
                        Answer::Menu(menu) => (Some(provider), menu.provider()),
                        _ => (Some(provider), None),
                    }
                }
            });
            (taken, looked.0.get(), looked.1.get())
        };
        let (lines, on_lines) = host(fixtures::on_task_lines);
        let (others, off_lines) = host(fixtures::off_task_lines);
        let (empty, opens_nothing) = host(no_rows);
        let spelling = Some((None, None));
        let task = Some("- [ ] call");

        // The suggestions menu first: a misspelled word is its own, and
        // nothing is looked for or asked on behalf of those after it.
        let list = [Provider::Spelling, on_lines.clone(), off_lines.clone()];
        assert_eq!(ask(&list, true, task), (spelling, 1, 0));
        assert!(lines.asked().is_empty() && others.asked().is_empty());
        // With no such word, the first of the host's that answers; the
        // place is looked for once, and the one after it is not asked.
        assert_eq!(ask(&list, false, task), (Some((Some(1), Some(1))), 1, 1));
        assert_eq!((lines.asked().len(), others.asked().len()), (1, 0));
        // One that passes hands the click on, as one whose answer opens
        // nothing does; taking the keyboard is an answer.
        assert_eq!(
            ask(&list, false, Some("call")),
            (Some((Some(2), None)), 1, 1)
        );
        assert_eq!((lines.asked().len(), others.asked().len()), (2, 1));
        let list = [opens_nothing.clone(), on_lines.clone()];
        assert_eq!(ask(&list, true, task), (Some((Some(1), Some(1))), 0, 1));
        assert_eq!(empty.asked().len(), 1);

        // A provider ahead of the suggestions menu takes a misspelled
        // word it answers for, and the word is not even looked for.
        let list = [on_lines.clone(), Provider::Spelling];
        assert_eq!(ask(&list, true, task), (Some((Some(0), Some(0))), 0, 1));
        assert_eq!(ask(&list, true, Some("call")), (spelling, 1, 1));
        assert_eq!(ask(&list, false, Some("call")), (None, 1, 1));
        // Off the text there is no place to ask the host's about, and
        // the suggestions menu has no word there.
        let asked = lines.asked().len();
        assert_eq!(ask(&list, false, None), (None, 1, 1));
        assert_eq!(lines.asked().len(), asked);
        // No provider, no menu: an empty list takes nothing.
        assert_eq!(ask(&[], true, task), (None, 0, 0));
        // One provider is itself and no other.
        assert!(Provider::Spelling.is(&Provider::Spelling) && on_lines.is(&on_lines.clone()));
        assert!(!on_lines.is(&off_lines) && !on_lines.is(&Provider::Spelling));
        assert!(!on_lines.is(&host(fixtures::on_task_lines).1));
    }

    /// What the test provider of task lines answers for a click on `text`.
    fn answer(text: &str) -> Answer {
        provided(
            &fixtures::Provides::new(fixtures::on_task_lines),
            1,
            &place(text, Some(0)),
        )
    }

    fn context() -> PopoverEvent {
        PopoverEvent::Context {
            provider: 1,
            at: Point::ORIGIN,
            seen: Seen::new(Edition::default(), 0),
        }
    }

    impl Batch {
        /// Publish a right-click that the provider answered `provided`.
        fn right_click(&mut self, provided: Answer) -> Option<(Opened, OpenMenu)> {
            let mut messages = Vec::new();
            let mut shell = iced::advanced::Shell::new(&mut messages);
            let start = self.start.clone();
            self.now.publish_provided(
                &mut shell,
                Message::Spell(context()),
                &|| Some(start.clone()),
                Sources::default(),
                provided,
            );
            self.published += messages.len();
            self.now.live()
        }
    }

    #[test]
    fn a_right_click_opens_what_its_provider_answered_for_the_keys_after_it() {
        let mut batch = Batch::over("- [ ] call", 8);
        let opened = batch.right_click(answer("- [ ] call"));
        assert_eq!(
            opened,
            Some((
                Opened::Menu,
                OpenMenu {
                    selected: Some(0),
                    items: 3,
                    opening: false
                }
            ))
        );
        assert_eq!(batch.published, 1);
        // Its rows are known for the keys that follow, it is the widget's
        // own until the host has drawn it, and it is no suggestions menu.
        let (spelling, accelerators) = batch.now.rows(None);
        assert!(!spelling && batch.now.is_own());
        assert_eq!(
            accelerators,
            [' ', '/', 'x'].map(|key| Some(Accelerator::plain(key)))
        );
        let moved = batch.event(PopoverEvent::Move(1));
        assert_eq!(moved.map(|(_, menu)| menu.selected), Some(Some(1)));
        assert_eq!(batch.event(PopoverEvent::Accept(1)), None);
        // It closes whatever was open, the provider's answer or none.
        let mut batch = Batch::over("- [ ] ", 6);
        batch.publish(typed('d'));
        assert_eq!(batch.publish(typed('u')), suggesting(2));
        assert_eq!(batch.right_click(Answer::Nothing), None);
        // A provider that has the host take the keyboard: the keys after
        // the click are the host's.
        let mut batch = Batch::over("call", 2);
        let keyboard = Some((Opened::Keyboard, OpenMenu::OPENING));
        assert_eq!(batch.right_click(Answer::Keyboard), keyboard);
        assert!(!batch.now.is_own());
        assert_eq!(batch.event(PopoverEvent::Stroke(letter())), keyboard);
        // A right-click asks for a popover and changes no text, so the
        // caret's line stays known for a request after it.
        let mut batch = Batch::over("- [ ] call", 8);
        batch.right_click(Answer::Nothing);
        assert!(batch
            .event(PopoverEvent::Request(fixtures::STATUS))
            .is_some());
        // As any message `after` does not decide alone, it closes a copy.
        assert_eq!(
            after(Some((Opened::Menu, OPEN)), &Message::Spell(context())),
            None
        );
    }

    #[test]
    fn the_widget_vouches_for_a_right_click_only_while_it_sees_the_text_as_the_host_will() {
        let open = || answer("- [ ] call");
        // What leaves the text, the caret and the scroll alone: the
        // popover's own events on a popover known to be open.
        let mut batch = Batch::over("- [ ] ", 6);
        assert!(batch.now.sees_the_text());
        batch.publish(typed('d'));
        batch.publish(typed('u'));
        assert!(!batch.now.sees_the_text(), "typing changes the text");
        let menu = Popover::new(Mode::Menu, 0, 2..5, "[ ]".into(), rows(&["a", "b"]));
        let passive = [
            PopoverEvent::Move(1),
            PopoverEvent::Hover(0),
            PopoverEvent::Key(MenuKey::Down),
        ];
        for event in passive {
            let mut batch = Batch::over("- [ ] call", 8);
            batch.now.start(Some(&menu));
            batch.event(event);
            assert!(batch.now.sees_the_text(), "{event:?}");
            // The menu the host has open is in the way of a right-click,
            // which the widget takes as a press that closes it; once the
            // batch has, the click is vouched for no more (see below).
        }
        // Closing leaves the text too, with nothing handed over to close.
        for event in [
            PopoverEvent::Close,
            PopoverEvent::Dismiss,
            PopoverEvent::Hold,
        ] {
            let mut batch = Batch::over("- [ ] call", 8);
            batch.event(event);
            assert!(batch.now.sees_the_text(), "{event:?}");
            assert!(batch.right_click(open()).is_some(), "{event:?}");
        }
        // Whatever may change the text, move the caret or scroll: an
        // edit, a motion, a scroll, a click, a row chosen, a key with
        // nothing known to be open, a stroke the host's field may act on,
        // and every request for a popover, which the host scrolls to show.
        for message in [
            typed('d'),
            Message::Action(Action::Edit(Edit::Backspace)),
            Message::Action(Action::Move(Motion::Left)),
            Message::Action(Action::Scroll { lines: 1 }),
            Message::Action(Action::Click(Point::ORIGIN)),
            Message::Intent(Intent::Backspace),
            Message::Undo,
            Message::Spell(PopoverEvent::Accept(0)),
            Message::Spell(PopoverEvent::AcceptSelected),
            Message::Spell(PopoverEvent::Key(MenuKey::Down)),
            Message::Spell(PopoverEvent::Key(MenuKey::Enter)),
            Message::Spell(PopoverEvent::Stroke(letter())),
            Message::Spell(PopoverEvent::OpenAtCaret),
            Message::Spell(PopoverEvent::OpenAt(Point::ORIGIN)),
            Message::Spell(PopoverEvent::Press(MenuPress::Right(Point::ORIGIN))),
            Message::Spell(PopoverEvent::Request(9)),
            Message::Spell(context()),
        ] {
            let mut batch = Batch::over("- [ ] call", 8);
            batch.publish(message.clone());
            assert!(!batch.now.sees_the_text(), "{message:?}");
            // A click published all the same would open what it is
            // handed: the widget is the one that must not publish it.
            assert!(batch.right_click(open()).is_some(), "{message:?}");
        }
        // A popover the host had open that has closed in the batch: a row
        // chosen from it with the pointer closes the copy with no word to
        // the widget, and the host acts on the row.
        for mode in [Mode::Menu, Mode::Suggest] {
            let handed = Popover::new(mode, 0, 2..5, "[ ]".into(), rows(&["a", "b"]));
            let batch = Batch::over("- [ ] call", 8);
            batch.now.start(Some(&handed));
            assert!(batch.now.sees_the_text(), "{mode:?}");
            let mut messages = Vec::new();
            let mut shell = iced::advanced::Shell::new(&mut messages);
            let accept = Message::Spell(PopoverEvent::Accept(1));
            publish(&mut shell, batch.now.cell(), accept);
            assert!(!batch.now.sees_the_text(), "{mode:?}");
        }
        // Suggestions the host has open stay open under a right-click's
        // routing, and the click is vouched for.
        let handed = Popover::new(Mode::Suggest, 0, 2..5, "[ ]".into(), rows(&["a"]));
        let mut batch = Batch::over("- [ ] call", 8);
        batch.now.start(Some(&handed));
        assert!(batch.now.sees_the_text());
        assert_eq!(
            batch.right_click(open()).map(|(opened, _)| opened),
            Some(Opened::Menu)
        );
    }

    #[test]
    fn what_the_widget_saw_of_a_right_click_binds_all_that_the_click_depends_on() {
        let task = place("- [ ] call", Some(7));
        let menu = |clicked: &Clicked| {
            provided(
                &fixtures::Provides::new(fixtures::on_task_lines),
                1,
                clicked,
            )
        };
        let print = fingerprint(1, &task, &menu(&task));
        assert_eq!(print, fingerprint(1, &task.clone(), &menu(&task)));
        // Which popover the editor opens it as is no part of the answer.
        let Answer::Menu(opened) = menu(&task) else {
            panic!("no menu");
        };
        assert_eq!(
            print,
            fingerprint(1, &task, &Answer::Menu(opened.opened_as(9)))
        );
        // Another line, another text, another glyph, another hit, another
        // provider, another answer: each is another click. So is the
        // keyboard taken by another provider, which has no menu to tell
        // the two apart by.
        let mut others = vec![
            (
                1,
                Clicked {
                    line: 4,
                    ..task.clone()
                },
                menu(&task),
            ),
            (1, place("- [ ] calls", Some(7)), menu(&task)),
            (1, place("- [ ] call", Some(8)), menu(&task)),
            (1, place("- [ ] call", None), menu(&task)),
            (2, task.clone(), menu(&task)),
            (1, task.clone(), Answer::Keyboard),
            (2, task.clone(), Answer::Keyboard),
            (1, task.clone(), Answer::Nothing),
        ];
        let misspelled = Clicked {
            hit: Some(Hit::Misspelling(6..10)),
            ..task.clone()
        };
        others.push((1, misspelled, menu(&task)));
        let other_word = Clicked {
            hit: Some(Hit::Misspelling(6..9)),
            ..task.clone()
        };
        others.push((1, other_word, menu(&task)));
        let fewer = Popover::new(Mode::Menu, 3, 2..5, "[ ]".into(), rows(&["Open"]));
        others.push((1, task.clone(), Answer::Menu(fewer.provided_by(1))));
        let elsewhere = Popover::new(Mode::Menu, 3, 0..1, "-".into(), fixtures::status_rows());
        others.push((1, task.clone(), Answer::Menu(elsewhere.provided_by(1))));
        let mut prints: Vec<u64> = (others.iter())
            .map(|(provider, clicked, answer)| fingerprint(*provider, clicked, answer))
            .collect();
        prints.push(print);
        let all = prints.len();
        prints.sort_unstable();
        prints.dedup();
        assert_eq!(prints.len(), all);

        // And it is bound to the text's revision, the list of providers,
        // and the popovers opened so far.
        let edition = Edition {
            revision: 7,
            list: 2,
            opened: 4,
        };
        let seen = Seen::new(edition, print);
        assert!(seen.over(edition) && seen.answered(print));
        assert!(!seen.answered(print.wrapping_add(1)));
        for other in [
            Edition {
                revision: 8,
                ..edition
            },
            Edition { list: 3, ..edition },
            Edition {
                opened: 5,
                ..edition
            },
            Edition::default(),
        ] {
            assert!(!seen.over(other), "{other:?}");
        }
    }

    #[test]
    fn a_row_of_a_providers_menu_is_chosen_by_the_menus_own_number() {
        // A provider's menu, the host's own, and the suggestions menu,
        // each as the seventh popover the editor opened.
        let rows = || rows(&["a", "b"]);
        let provided = Popover::new(Mode::Menu, 0, 2..5, "[ ]".into(), rows())
            .provided_by(1)
            .opened_as(7);
        let hosts = Popover::new(Mode::Menu, 0, 2..5, "[ ]".into(), rows()).opened_as(7);
        let spelling = Popover::new(Mode::Menu, 0, 2..5, "[ ]".into(), rows())
            .spelling()
            .opened_as(7);
        assert_eq!((provided.instance(), provided.provider()), (7, Some(1)));
        // A provider's menu binds the choice of its rows, whether the host
        // has drawn it or the widget opened it in this batch; so does the
        // host's own menu (item 3.4), and the suggestions menu does not.
        let now = Now::new();
        assert_eq!(now.bound(Some(&provided)), None, "nothing is open");
        now.start(Some(&provided));
        assert_eq!(now.bound(Some(&provided)), Some(7));
        for (other, bound) in [(&hosts, Some(7)), (&spelling, None)] {
            now.start(Some(other));
            assert_eq!(now.bound(Some(other)), bound);
        }
        let mut batch = Batch::over("- [ ] call", 8);
        let Answer::Menu(own) = answer("- [ ] call") else {
            panic!("no menu");
        };
        batch.right_click(Answer::Menu(own.opened_as(3)));
        assert_eq!(batch.now.bound(None), Some(3));
        // A row so chosen closes the copy, as any chosen row.
        let chosen = PopoverEvent::AcceptOf {
            menu: 3,
            row: Some(1),
        };
        assert_eq!(batch.event(chosen), None);
        assert_eq!(batch.now.bound(None), None);
        assert!(!batch.now.sees_the_text());
    }

    #[test]
    fn the_suggestions_menu_leads_a_list_it_is_first_in() {
        let (_, host) = host(fixtures::on_task_lines);
        assert!(spelling_leads(&SPELLING_ALONE));
        assert!(spelling_leads(&[Provider::Spelling, host.clone()]));
        assert!(!spelling_leads(&[host.clone(), Provider::Spelling]));
        assert!(!spelling_leads(&[host]));
        assert!(!spelling_leads(&[]));
    }

    #[test]
    fn an_accelerator_names_its_key_and_a_symbol_matches_what_is_typed() {
        use iced::keyboard::key::NativeCode;
        let unidentified = Physical::Unidentified(NativeCode::Unidentified);
        let labels: Vec<String> = [
            Accelerator::plain(' '),
            Accelerator::plain('/'),
            Accelerator::plain('x'),
            Accelerator::plain('-'),
            Accelerator::shifted('h'),
        ]
        .into_iter()
        .map(Accelerator::label)
        .collect();
        assert_eq!(labels, ["Space", "/", "X", "-", "\u{21e7}H"]);
        // `/` typed with Shift on a keyboard where it is a shifted key.
        let slash = Accelerator::plain('/');
        let seven = Key::Character("7".into());
        assert!(slash.matches_press(&seven, unidentified, Modifiers::SHIFT, Some("/")));
        assert!(!slash.matches(&seven, unidentified, Modifiers::SHIFT));
        assert!(!slash.matches_press(&seven, unidentified, Modifiers::CTRL, Some("/")));
        // A letter is never matched by what it types: Shift+x is no `x`.
        let x = Accelerator::plain('x');
        let key = Key::Character("x".into());
        assert!(!x.matches_press(&key, unidentified, Modifiers::SHIFT, Some("X")));
        // Nor is the space bar by a space typed some other way.
        let space = Accelerator::plain(' ');
        assert!(!space.matches_press(&seven, unidentified, Modifiers::empty(), Some(" ")));
    }

    #[test]
    fn a_row_that_cannot_be_chosen_is_stepped_over() {
        let row = |id, enabled: bool| Row::new(id, "r").enabled(enabled);
        let rows = vec![
            row(0, true),
            row(1, false),
            row(2, false),
            row(3, true),
            row(4, false),
        ];
        let mut popover = Popover::new(Mode::Menu, 0, 0..1, "x".into(), rows);
        assert_eq!(popover.selected(), Some(0));
        popover.step(1);
        assert_eq!(popover.selected(), Some(3));
        popover.step(1);
        assert_eq!(
            popover.selected(),
            Some(3),
            "only rows that cannot be chosen after it"
        );
        popover.step(-1);
        assert_eq!(popover.selected(), Some(0));
        popover.step(5);
        assert_eq!(popover.selected(), Some(3));
        popover.hover(2);
        assert_eq!(popover.selected(), Some(3));
        popover.hover(0);
        assert_eq!(popover.selected(), Some(0));
        assert!(popover.accelerators().iter().all(Option::is_none));
        let keyed = vec![Row::new(0, "a")
            .accelerator(Accelerator::plain('a'))
            .enabled(false)];
        let popover = Popover::new(Mode::Menu, 0, 0..1, "x".into(), keyed);
        assert!(popover.accelerators()[0].is_some_and(Accelerator::is_inert));
        assert_eq!(popover.hint(0).as_deref(), Some("A"));
        // The widget's copy steps over them the same way.
        let open = Some((
            Opened::Menu,
            OpenMenu {
                selected: Some(0),
                items: 5,
                opening: false,
            },
        ));
        let live = [true, false, false, true, false];
        let moved = after_with(open, &Message::Spell(PopoverEvent::Move(1)), &live);
        assert_eq!(moved.map(|(_, menu)| menu.selected), Some(Some(3)));
        let key = Message::Spell(PopoverEvent::Key(MenuKey::Down));
        assert_eq!(
            after_with(open, &key, &live).map(|(_, menu)| menu.selected),
            Some(Some(3))
        );
    }

    #[test]
    fn a_row_past_the_64th_keeps_whether_it_can_be_chosen() {
        let mut rows: Vec<Row> = (0..70).map(|id| Row::new(id, "r")).collect();
        rows[66] = Row::new(66, "off")
            .accelerator(Accelerator::plain('o'))
            .enabled(false);
        rows[68] = rows[68].clone().checked(true);
        let mut popover = Popover::new(Mode::Menu, 0, 0..1, "x".into(), rows);
        assert_eq!(
            popover.selected(),
            Some(68),
            "the checked row, past the 64th"
        );
        assert!(!popover.is_enabled(66) && popover.is_enabled(65));
        assert!(popover.accelerators()[66].is_some_and(Accelerator::is_inert));
        popover.step(-2);
        assert_eq!(popover.selected(), Some(65), "the 67th row is stepped over");
        popover.hover(66);
        assert_eq!(popover.selected(), Some(65));
        assert_eq!(popover.live().len(), 70);
        assert!(!popover.live()[66]);
    }
}
