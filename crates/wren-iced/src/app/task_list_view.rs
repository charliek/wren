//! The task view's list (discovery/tasks.md §9; item 10.3): the body of
//! the view in the pane, its filters and its footer, read-only.
//!
//! - **What it lists** is [`task_list::Listing`] built from the task index
//!   for the view's scope, the scope's remembered filters (`.wren/tasks.toml`
//!   through the Session, written only when the user changes one), the
//!   `tasks-me` names and today. It is built again after any message that
//!   changes one of those ([`crate::app::App::sync_task_list`]), never while drawing.
//! - **Only the rows in view are built:** headers and rows have fixed
//!   heights, and the scrolled-out ones are one space each.
//! - **Keys** (the view has the keyboard, [`crate::list_keys`]): ↑/↓ and
//!   j/k move the selection, Home and End, Page Up and Page Down, ↵ opens
//!   the note at the task's line. Tab and Shift+Tab walk the scope chip,
//!   the filters and the list, round the ends; on a control, ↵ or Space
//!   presses it, ← and → step to the next control, and Escape gives the
//!   keyboard back to the list. A menu over the view takes the keys while
//!   it is open ([`super::host_menu`]).
//! - **Escape** closes the menu over the view, else gives a control's
//!   keyboard back to the list, else does nothing: it neither clears the
//!   filters (they are remembered) nor leaves the view (Back does).
//! - **Input in order, batch by batch.** Every key report and click
//!   carries the serial of the view it came from, which is its batch of
//!   events ([`crate::list_keys`]), and the list reads them in input order.
//!   A key that opens something (a menu, the note), Escape that closes a
//!   menu, and any press stop the rest of their batch, which was typed
//!   before the user could see what changed: it is dropped (drop, don't
//!   forward). Escape and the app's shortcuts are read at their place in
//!   the batch, and a shortcut that takes the keyboard from the list (the
//!   palette, a dialog) stops the rest of it. Every key the view reads is
//!   done with there, so the app's own copies of Escape and of the
//!   shortcuts, which come after the batch, are matched one by one to the
//!   reports they copy and always dropped ([`Copies`]). A click on a row
//!   names the listing it was drawn from, and does nothing once that
//!   listing is gone.
//! - **The selection** survives a refresh by note, line and source line
//!   ([`task_list::Listing::find`]), else stays at its place in the list.
//! - **↵ opens the note** at the task as a write would find it
//!   (`wren_core::task_write::locate`), never at a twin of its text in a
//!   fence; with none to find, the note opens where it was and says so.
//!   On the note already open under the view, which records no visit,
//!   Back is still the view (`tasks_view::Trail`).
//! - **Writes** (item 10.4, `task_rows.rs`): keys, clicks and menus on a
//!   row change its task in its note, each with a message with Undo, and a
//!   row written stays where it stood until R refreshes the list.
//! - **Two layouts** (discovery/tasks.md §11; item 12.1): the list, and
//!   the board of status columns. Which one shows is kept for each scope
//!   beside its filters ([`TaskFilters::layout`]), switched by the List |
//!   Board control in the header, `B` while the list or the board has the
//!   keyboard, and the App command `tasks.toggle_board`. A switch is
//!   written as a filter change is, and lets the held rows go; it is no
//!   stop in Back and Forward. On the board the status chips, Show all and
//!   Group by are hidden, and are no Tab stops.
//! - **Its parts:** what a layout of the view shares is apart from each
//!   layout's own (`list.rs`: the list's body, rows, selection and keys;
//!   `board.rs`: the board's).
//!   `build.rs` builds the listing, `controls.rs` is the bar of filters
//!   and the menus it opens, `menu.rs` the menu over the view, `keys.rs`
//!   and `clicks.rs` read the view's input, `copies.rs` holds the app's
//!   copies of the keys the view read, `open.rs` opens a row's note, and
//!   `footer.rs` is the pane's footer under the view.

use iced::Rectangle;
use wren_core::tasks_file::{Layout, TaskFilters};
use wren_markdown::date::Date;

use super::host_menu::HostMenu;
use super::task_list::{self, GroupKey, Listing, RowKey};
use super::tasks_view::Scope;
use crate::list_keys::Drawn;

mod board;
mod build;
mod clicks;
mod controls;
mod copies;
mod footer;
mod keys;
mod list;
mod menu;
mod open;
#[cfg(test)]
mod test_support;

#[cfg(feature = "smoke-test")]
pub(super) use board::COLUMNS as BOARD_COLUMNS;
pub use clicks::Click;
#[cfg(feature = "smoke-test")]
pub(super) use controls::empty_hint;
pub(super) use copies::{AppCopy, Copies};
#[cfg(test)]
pub(super) use list::box_face;
#[cfg(feature = "smoke-test")]
pub(super) use list::{due_cell, source_text};
pub(super) use menu::MenuFor;
#[cfg(test)]
pub(super) use open::STALE_OPEN;

/// The list's scrollable.
pub(super) const LIST: &str = "task-list";

/// The list's height before it is first drawn.
const UNDRAWN_HEIGHT: f32 = 2000.0;

/// A control of the view, by what it does: what Tab walks, what a menu
/// hangs under, and where the harness clicks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Scope,
    /// The List | Board switch.
    Layout,
    /// The status chip at this place in [`task_list::CHIPS`].
    Chip(usize),
    ShowAll,
    Owner,
    GroupBy,
    Due,
    Priority,
    HideOld,
    /// The list itself.
    List,
    /// The source line of the list's entry at this index.
    Source(usize),
    /// The row at this entry, and one of its cells.
    Row(usize),
    Cell(usize, Cell),
}

/// A cell of a row that a click or a key acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// The status box: a click cycles it.
    Box,
    /// The priority: a click opens the priority menu.
    Priority,
    /// The due date: a click opens the calendar.
    Due,
    /// The description: a double-click edits it in place.
    Description,
}

impl Cell {
    #[cfg(feature = "smoke-test")]
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Priority => "priority",
            Self::Due => "due",
            Self::Description => "description",
        }
    }
}

impl Control {
    /// The harness's name for it.
    #[cfg(feature = "smoke-test")]
    pub(super) fn name(self) -> String {
        match self {
            Self::Scope => "scope".into(),
            Self::Layout => "layout".into(),
            Self::Chip(chip) => format!("chip:{}", task_list::CHIPS[chip].name()),
            Self::ShowAll => "show_all".into(),
            Self::Owner => "owner".into(),
            Self::GroupBy => "group".into(),
            Self::Due => "due".into(),
            Self::Priority => "priority".into(),
            Self::HideOld => "hide_old".into(),
            Self::List => "list".into(),
            Self::Source(entry) => format!("source:{entry}"),
            Self::Row(entry) => format!("row:{entry}"),
            Self::Cell(entry, cell) => format!("{}:{entry}", cell.name()),
        }
    }

    /// Whether pressing it opens a menu.
    const fn opens_menu(self) -> bool {
        matches!(
            self,
            Self::Scope | Self::Owner | Self::GroupBy | Self::Due | Self::Priority
        )
    }
}

/// What the listing was built from.
#[derive(Debug, Clone, PartialEq)]
struct Built {
    /// The workspace's root and generation: another workspace's index
    /// counts its revisions afresh.
    workspace: (std::path::PathBuf, u64),
    scope: Scope,
    revision: u64,
    filters: TaskFilters,
    me: Vec<String>,
    today: Date,
    held: Vec<task_list::Hold>,
}

/// The selected row: which task, under which group, at which entry.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Selected {
    pub key: RowKey,
    pub group: GroupKey,
    pub entry: usize,
}

/// The list's state beside the Session's truth: what it shows, where, and
/// what has its keyboard.
#[derive(Debug, Default)]
pub(super) struct ListState {
    built: Option<Built>,
    pub listing: Listing,
    pub selected: Option<Selected>,
    /// The list's scroll offset and height, as it last reported them.
    pub scroll: f32,
    height: Option<f32>,
    /// The control with the keyboard; `None` is the list.
    pub focus: Option<Control>,
    pub menu: Option<HostMenu<MenuFor>>,
    menus: u64,
    /// The serial of the view last built: the batch its input comes in.
    serials: std::cell::Cell<u64>,
    /// The batch whose input was read last.
    batch: u64,
    /// Something in that batch opened, closed or was pressed: the rest of
    /// it is dropped.
    pub stopped: bool,
    /// A key reached the menu over the view in that batch: no press after
    /// it there chooses a row (discovery/tasks.md §6.2).
    menu_keyed: bool,
    /// The menu and row a press over the card may choose at its release.
    armed: Option<(u64, u32)>,
    /// The app's copies still to come of the batch's Escapes and
    /// shortcuts, which the view read.
    pub copies: Copies,
    /// Moves each time the listing is built: what a row click names.
    pub generation: u64,
    /// The titles of the workspace's notes, read when the listing is built.
    titles: std::collections::HashMap<wren_core::model::NoteId, String>,
    /// The footer's counts and what it says is left out, for the listing.
    footer: (wren_core::tasks::Counts, Option<String>),
    /// Where the view's controls were last drawn.
    pub drawn: Drawn<Control>,
    /// The rows written since the list was last refreshed, held where they
    /// stood.
    pub held: Vec<task_list::Hold>,
    /// The row whose description is being edited in place, and its field.
    pub describe: Option<super::task_describe::Describe>,
    /// The number of the last description field opened.
    pub describes: u64,
    /// The row a click of this batch selected: the second press of a
    /// double-click on its description goes on to edit it.
    selected_in_batch: Option<(u64, usize)>,
    /// The next build follows from the user's asking the rows to move (a
    /// filter, the grouping, R): a selected row in sight is kept in sight.
    /// A build of the index's own (a rescan) keeps the list's offset.
    pub reveal: bool,
}

impl ListState {
    /// The layout the view shows: the one its listing was built for.
    pub(super) fn layout(&self) -> Layout {
        self.built
            .as_ref()
            .map_or(Layout::List, |built| built.filters.layout)
    }

    /// Forget the list: another workspace is open. The serials and the
    /// menus' numbers go on, so nothing of the old list's is taken for
    /// the new one's.
    pub(super) fn reset(&mut self) {
        *self = Self {
            serials: std::cell::Cell::new(self.serials.get()),
            menus: self.menus,
            describes: self.describes,
            generation: self.generation + 1,
            ..Self::default()
        };
    }

    /// A new serial, for the view being built or for the harness's input.
    pub(super) fn next_serial(&self) -> u64 {
        let serial = self.serials.get() + 1;
        self.serials.set(serial);
        serial
    }

    /// The control at `control`'s place, as last drawn.
    pub(super) fn drawn(&self, control: Control) -> Option<Rectangle> {
        (self.drawn.borrow().iter())
            .find(|(name, _)| *name == control)
            .map(|(_, at)| *at)
    }
}
