//! The task view's list (discovery/tasks.md §9; item 10.3): the body of
//! the view in the pane, its filters and its footer, read-only.
//!
//! - **What it lists** is [`task_list::Listing`] built from the task index
//!   for the view's scope, the scope's remembered filters (`.wren/tasks.toml`
//!   through the Session, written only when the user changes one), the
//!   `tasks-me` names and today. It is built again after any message that
//!   changes one of those ([`App::sync_task_list`]), never while drawing.
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
//!   palette, a dialog) stops the rest of it; the app's own copies of
//!   Escape and of the shortcuts, which come after the batch, are matched
//!   one by one to the reports they copy, and each is dropped or let
//!   through as its report was. A click on a row names the listing it
//!   was drawn from, and does nothing once that listing is gone.
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

use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::widget::{button, column, container, mouse_area, row, scrollable, svg, Space};
use iced::{Element, Fill, Rectangle, Shrink, Task};
use wren_core::fs_store::RelPath;
use wren_core::model::ItemRef;
use wren_core::ops::Op;
use wren_core::tasks::TaskRecord;
use wren_core::tasks_file::{DueBucket, GroupBy, OwnerFilter, PriorityFilter, TaskFilters};
use wren_core::theme::Rgb;
use wren_editor::{Accelerator, Row};
use wren_markdown::date::{Date, Parsed};
use wren_markdown::status::StatusType;

use super::columns::Column;
use super::host_menu::{self, HostMenu, MenuKey};
use super::palette::SearchTarget;
use super::task_list::{self, Entry, GroupKey, Listing, Piece, RowKey, HEADER_HEIGHT, ROW_HEIGHT};
use super::tasks_view::{Event, Scope};
use super::{App, Message};
use crate::ellipsis::{ellipsis, ellipsis_runs};
use crate::list_keys::{self, list_keys, probe, Drawn, ListKey, Press};
use crate::{chrome, fonts, theme};

/// The list's scrollable.
pub(super) const LIST: &str = "task-list";
/// The list's height before it is first drawn.
const UNDRAWN_HEIGHT: f32 = 2000.0;
/// What opening a row's note says when its task is no longer there.
pub(super) const STALE_OPEN: &str = "This task changed since the list was built; the note is open";
/// The space between the footer's counts and its hint.
const FOOTER_GAP: f32 = 12.0;
/// What separates the pieces of the footer's hint.
const HINT_SEPARATOR: &str = " \u{b7} ";
/// What the footer says the list's keys are.
pub(super) const HINT: &str =
    "Space toggle \u{b7} S D P \u{b7} E edit \u{b7} F2 rename \u{b7} \u{21b5} open \u{b7} R refresh";

/// A control of the view, by what it does: what Tab walks, what a menu
/// hangs under, and where the harness clicks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Scope,
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

/// What a menu over the view is for, with what its rows name.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum MenuFor {
    /// "Show tasks in": each row's scope, `None` for one there is none of.
    Scope(Vec<Option<Scope>>),
    /// Anyone, Me, then these people.
    Owner(Vec<String>),
    GroupBy,
    Due,
    Priority,
    /// A row's status, priority or task menu, for the task it was opened
    /// on, at the entry of the listing `generation`.
    Row {
        kind: super::task_menus::MenuKind,
        target: super::task_target::RowTarget,
        /// Where the task's box is in its line, as the task index found it.
        boxed: usize,
        entry: usize,
        generation: u64,
    },
}

impl MenuFor {
    #[cfg(feature = "smoke-test")]
    pub(super) const fn name(&self) -> &'static str {
        use super::task_menus::MenuKind;
        match self {
            Self::Scope(_) => "scope",
            Self::Owner(_) => "owner",
            Self::GroupBy => "group",
            Self::Due => "due",
            Self::Priority => "priority",
            Self::Row { kind, .. } => match kind {
                MenuKind::Status => "row_status",
                MenuKind::Priority => "row_priority",
                MenuKind::Task => "row_task",
            },
        }
    }
}

/// A click in the view or in its menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Click {
    /// A row, by the listing it was drawn from and its entry there.
    Select {
        generation: u64,
        entry: usize,
    },
    /// A row's source: open the note at its line.
    Open {
        generation: u64,
        entry: usize,
    },
    /// A cell of a row.
    Cell {
        generation: u64,
        entry: usize,
        cell: Cell,
    },
    /// A right-click on a row: its task menu.
    RowMenu {
        generation: u64,
        entry: usize,
    },
    /// A double-click on a row's description: edit it in place. It comes
    /// in the batch of its second press, after that press's `Select`.
    Describe {
        generation: u64,
        entry: usize,
    },
    Control(Control),
    Menu(host_menu::Event),
}

/// The app's own copy of a key the view reported, which its key handling
/// hears after the batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AppCopy {
    Escape,
    Shortcut(wren_core::keybind::Accel),
}

const GROUPINGS: [(GroupBy, &str, char); 4] = [
    (GroupBy::Note, "Note", 'n'),
    (GroupBy::Due, "Due date", 'd'),
    (GroupBy::Priority, "Priority", 'p'),
    (GroupBy::Owner, "Owner", 'o'),
];

const BUCKET_KEYS: [char; 7] = ['o', 'e', 't', 'w', 'x', 'l', 'n'];

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
    /// shortcuts, in order, each with whether it is dropped.
    pub copies: std::collections::VecDeque<(AppCopy, bool)>,
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
}

impl ListState {
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

/// What a row's box shows: its status, or the symbol in the box when it
/// has no status of its own (`[?]`, `[>]`), which counts as open but is
/// shown as written (discovery/tasks.md §5.1).
pub(super) fn box_face(record: &TaskRecord) -> Result<StatusType, char> {
    if wren_markdown::status::status(record.symbol).name == wren_markdown::status::UNKNOWN {
        Err(record.symbol)
    } else {
        Ok(record.status)
    }
}

/// A row's box, by [`box_face`].
fn row_box<'a>(record: &TaskRecord, colors: &wren_core::theme::Colors) -> Element<'a, Message> {
    let symbol = match box_face(record) {
        Ok(status) => return task_box(status, colors),
        Err(symbol) => symbol,
    };
    let (edge, ink) = (colors.muted, colors.text);
    container(
        chrome::label(symbol.to_string(), 10, ink)
            .font(fonts::SEMIBOLD)
            .align_x(iced::Center),
    )
    .center_x(13)
    .center_y(13)
    .style(move |_| container::Style {
        border: iced::Border {
            color: theme::color(edge),
            width: 1.3,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// A task's box, as the editor reads its status: open empty, doing with a
/// stroke, done filled with a tick, cancelled with a dash.
fn task_box<'a>(status: StatusType, colors: &wren_core::theme::Colors) -> Element<'a, Message> {
    let hex = |Rgb(r, g, b): Rgb| format!("#{r:02x}{g:02x}{b:02x}");
    let (rect, fill, path, stroke) = match status {
        StatusType::Open => (colors.muted, None, "", colors.accent),
        StatusType::Doing => (colors.accent, None, "M5.2 10.8l5.6-5.6", colors.accent),
        StatusType::Done => (
            colors.accent,
            Some(colors.accent),
            "M4.8 8.2l2.2 2.2 4.2-4.6",
            colors.editor,
        ),
        StatusType::Cancelled => (colors.muted, None, "M5 8h6", colors.faint),
    };
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16' fill='none' \
         stroke-linecap='round' stroke-linejoin='round'><rect x='2' y='2' width='12' height='12' \
         rx='3' stroke='{}' stroke-width='1.3' fill='{}'/><path d='{path}' stroke='{}' \
         stroke-width='1.9'/></svg>",
        hex(rect),
        fill.map_or_else(|| "none".to_owned(), hex),
        hex(stroke),
    );
    svg(svg::Handle::from_memory(source.into_bytes()))
        .width(16)
        .height(16)
        .into()
}

/// How a row's due cell reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DueCell {
    pub text: String,
    /// `normal`, `overdue`, `invalid` or `finished`.
    pub look: &'static str,
}

/// The due cell of `record` on `today`: the finished work's stamp, else
/// its due date, overdue or not, or a date that is no real day as written
/// and marked; `None` with no date.
pub(super) fn due_cell(record: &TaskRecord, today: Date) -> Option<DueCell> {
    if record.status.is_finished() {
        let stamp = match record.status {
            StatusType::Cancelled => ('\u{2715}', record.cancelled),
            _ => ('\u{2713}', record.done),
        };
        if let (mark, Some(Parsed::Date(date))) = stamp {
            return Some(DueCell {
                text: format!("{mark} {}", date.short()),
                look: "finished",
            });
        }
    }
    match record.due? {
        Parsed::Date(due) => Some(DueCell {
            text: due.short().to_string(),
            look: if record.is_overdue(today) {
                "overdue"
            } else {
                "normal"
            },
        }),
        Parsed::Invalid => Some(DueCell {
            text: format!(
                "{} \u{26a0}",
                task_list::invalid_due(record).unwrap_or("invalid date")
            ),
            look: "invalid",
        }),
    }
}

/// The source line under a row's description: the nearest heading when
/// the list is grouped by note, else the group and the note too.
pub(super) fn source_text(record: &TaskRecord, title: &str, by_note: bool) -> String {
    let heading = record
        .heading
        .as_deref()
        .filter(|heading| !heading.is_empty());
    if by_note {
        return heading.unwrap_or("Top of note").to_owned();
    }
    let group = record
        .path
        .as_path()
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut text = format!("{group} \u{203a} {title}");
    if let Some(heading) = heading {
        text.push_str(" \u{b7} ");
        text.push_str(heading);
    }
    text
}

impl App {
    /// The item the view's scope names, `None` for the workspace; `Err`
    /// when it names one no longer there.
    fn scope_item(&self, scope: &Scope) -> Result<Option<ItemRef>, ()> {
        match scope {
            Scope::Workspace => Ok(None),
            scope => self
                .model()
                .and_then(|model| scope.item(model))
                .map(Some)
                .ok_or(()),
        }
    }

    /// The filters the view of `scope` remembers.
    pub(super) fn task_filters(&self, scope: &Scope) -> TaskFilters {
        match (&self.session, self.scope_item(scope)) {
            (Some(session), Ok(item)) => session.task_filters(item),
            _ => TaskFilters::default(),
        }
    }

    /// A note's title, as the model had it when the listing was built.
    pub(super) fn note_title(&self, note: wren_core::model::NoteId) -> &str {
        self.task_list.titles.get(&note).map_or("", String::as_str)
    }

    /// Build the list again when what it shows changed; keep the menu only
    /// while the view has the keyboard. After every message, with tasks on.
    /// Returns the scroll that keeps the list's widget where the list is.
    pub(super) fn sync_task_list(&mut self) -> Task<Message> {
        let Some(scope) = self.task_view().cloned() else {
            self.task_list.menu = None;
            self.task_list.focus = None;
            self.task_list.held.clear();
            self.task_list.describe = None;
            return Task::none();
        };
        if self.task_list.menu.is_some() && !self.task_view_has_keys() {
            self.task_list.menu = None;
        }
        // An edit in place lasts only while its field can have the keyboard.
        if !self.task_view_has_keys()
            || self.task_list.menu.is_some()
            || self.task_list.focus.is_some()
        {
            self.task_list.describe = None;
        }
        let Some(session) = &self.session else {
            return Task::none();
        };
        let revision = session
            .tasks()
            .map_or(0, wren_core::tasks::TaskIndex::revision);
        let workspace = (session.model().root.clone(), self.workspace_generation());
        // Another scope, or another workspace, holds no row of this one.
        if (self.task_list.built.as_ref())
            .is_some_and(|old| old.scope != scope || old.workspace != workspace)
        {
            self.task_list.held.clear();
        }
        // A held task is found again where the index has it now, or let go.
        if !self.task_list.held.is_empty()
            && (self.task_list.built.as_ref()).is_none_or(|old| old.revision != revision)
        {
            if let Some(tasks) = self.session.as_ref().and_then(|session| session.tasks()) {
                let prefix = scope.prefix();
                let list = &mut self.task_list;
                task_list::relocate_holds(
                    &mut list.held,
                    &list.listing.records,
                    tasks.tasks_under(prefix),
                );
            }
        }
        let Some(session) = &self.session else {
            return Task::none();
        };
        let built = Built {
            workspace,
            filters: self.task_filters(&scope),
            scope,
            revision,
            me: self.preferences.settings.tasks_me.clone(),
            today: self.today(),
            held: self.task_list.held.clone(),
        };
        if self.task_list.built.as_ref() == Some(&built) {
            return Task::none();
        }
        let new_scope = self
            .task_list
            .built
            .as_ref()
            .is_none_or(|old| old.scope != built.scope || old.workspace != built.workspace);
        let records: Vec<TaskRecord> = session
            .tasks()
            .map(|tasks| tasks.tasks_under(built.scope.prefix()).cloned().collect())
            .unwrap_or_default();
        let titles: std::collections::HashMap<_, _> = (session.model().notes.iter())
            .map(|note| (note.id, note.title.clone()))
            .collect();
        let place = |record: &TaskRecord| {
            let title = titles.get(&record.note).map_or("", String::as_str);
            task_list::note_place(&record.path, title)
        };
        let listing = Listing::build_holding(
            records,
            &built.filters,
            &built.me,
            built.today,
            &place,
            &built.held,
        );
        let footer = self.task_list_counts(&built.scope);
        let height = self.list_height();
        let list = &mut self.task_list;
        let before = std::mem::replace(&mut list.listing, listing);
        // Whether the selected row was in sight: if so, it is kept in
        // sight wherever the rebuild puts it, so a key acts on a row the
        // user can see.
        let in_sight = |tops: &[f32], entry: usize, scroll: f32| {
            tops.get(entry)
                .is_some_and(|&top| top >= scroll && top + ROW_HEIGHT <= scroll + height)
        };
        let seen = (list.selected.as_ref())
            .is_some_and(|selected| in_sight(&before.tops, selected.entry, list.scroll));
        list.titles = titles;
        list.footer = footer;
        list.generation += 1;
        list.built = Some(built);
        if new_scope {
            list.selected = None;
            list.scroll = 0.0;
            list.focus = None;
            list.menu = None;
            list.describe = None;
        }
        // An edit follows its row, and ends when the list no longer shows it.
        self.follow_describe(&before.records);
        let list = &mut self.task_list;
        let previous = list.selected.take();
        let entry = previous
            .as_ref()
            .and_then(|selected| list.listing.find(&selected.key, Some(&selected.group)))
            .or_else(|| {
                // Gone: the row now at its place, or the last.
                let at = previous.as_ref()?.entry;
                let rows: Vec<usize> = list.listing.rows().collect();
                rows.iter()
                    .copied()
                    .find(|&row| row >= at)
                    .or_else(|| rows.last().copied())
            })
            .or_else(|| list.listing.rows().next());
        list.selected = entry.map(|entry| select(&list.listing, entry));
        // The sources, rows and cells drawn belong to the old entries.
        (list.drawn.borrow_mut()).retain(|(control, _)| {
            !matches!(
                control,
                Control::Source(_) | Control::Row(_) | Control::Cell(..)
            )
        });
        let scroll = list
            .scroll
            .min((list.listing.height() - list.height.unwrap_or(0.0)).max(0.0));
        if let Some(entry) =
            entry.filter(|&entry| seen && !in_sight(&list.listing.tops, entry, scroll))
        {
            list.scroll = scroll;
            return self.select_entry(entry);
        }
        if !new_scope && scroll == list.scroll {
            return Task::none();
        }
        // The widget keeps its offset across scopes; the list starts at
        // the top.
        list.scroll = scroll;
        iced::widget::operation::scroll_to(
            LIST,
            iced::widget::operation::AbsoluteOffset { x: 0.0, y: scroll },
        )
    }

    /// The list's visible height.
    fn list_height(&self) -> f32 {
        self.task_list
            .height
            .or_else(|| self.task_list.drawn(Control::List).map(|at| at.height))
            .unwrap_or(UNDRAWN_HEIGHT)
    }

    /// Select the entry `entry` and scroll it into view.
    pub(super) fn select_entry(&mut self, entry: usize) -> Task<Message> {
        let Some(selected) = (self.task_list.listing.record(entry).is_some())
            .then(|| select(&self.task_list.listing, entry))
        else {
            return Task::none();
        };
        self.task_list.selected = Some(selected);
        let top = self.task_list.listing.tops[entry];
        let height = self.list_height();
        let scroll = self.task_list.scroll;
        // A group's first row brings its header with it.
        let wanted = if top < scroll {
            Some(
                if entry > 0 && self.task_list.listing.record(entry - 1).is_none() {
                    top - HEADER_HEIGHT
                } else {
                    top
                },
            )
        } else if top + ROW_HEIGHT > scroll + height {
            Some(top + ROW_HEIGHT - height)
        } else {
            None
        };
        match wanted {
            Some(y) => {
                let y = y.max(0.0);
                self.task_list.scroll = y;
                iced::widget::operation::scroll_to(
                    LIST,
                    iced::widget::operation::AbsoluteOffset { x: 0.0, y },
                )
            }
            None => Task::none(),
        }
    }

    /// The selection moved `step` rows, within the list.
    fn step_selection(&mut self, step: isize) -> Task<Message> {
        let rows: Vec<usize> = self.task_list.listing.rows().collect();
        if rows.is_empty() {
            return Task::none();
        }
        let at = self
            .task_list
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|&row| row == selected.entry));
        let next = match at {
            Some(at) => (at as isize + step).clamp(0, rows.len() as isize - 1) as usize,
            None => 0,
        };
        self.select_entry(rows[next])
    }

    /// The slots Tab walks, in order.
    fn task_view_slots(&self) -> Vec<Option<Control>> {
        let filters = self
            .task_list
            .built
            .as_ref()
            .map(|built| built.filters.clone())
            .unwrap_or_default();
        let mut slots = vec![Some(Control::Scope)];
        slots.extend((0..task_list::CHIPS.len()).map(|chip| Some(Control::Chip(chip))));
        if !filters.statuses.is_empty() {
            slots.push(Some(Control::ShowAll));
        }
        slots.extend(
            [
                Control::Owner,
                Control::GroupBy,
                Control::Due,
                Control::Priority,
                Control::HideOld,
            ]
            .map(Some),
        );
        slots.push(None);
        slots
    }

    /// Tab (`step` 1) or Shift+Tab (-1); `controls` keeps to the controls,
    /// as ← and → do.
    fn step_focus(&mut self, step: isize, controls: bool) {
        let mut slots = self.task_view_slots();
        if controls {
            slots.retain(Option::is_some);
        }
        let count = slots.len() as isize;
        let at = slots
            .iter()
            .position(|slot| *slot == self.task_list.focus)
            .map_or(if step > 0 { -1 } else { count }, |at| at as isize);
        self.task_list.focus = slots[(at + step).rem_euclid(count) as usize];
    }

    /// Read input of the batch `serial`: a new batch starts afresh.
    pub(super) fn enter_batch(&mut self, serial: u64) {
        let list = &mut self.task_list;
        if serial != list.batch {
            list.batch = serial;
            list.stopped = false;
            list.menu_keyed = false;
            list.copies.clear();
            list.selected_in_batch = None;
        }
    }

    /// A key reported by the view's key wrapper, at its place in its batch.
    /// See the module's description.
    pub(super) fn task_list_key(&mut self, key: ListKey) -> Task<Message> {
        self.enter_batch(key.serial);
        let press = key.press;
        let copy = if press.is_plain_escape() {
            Some(AppCopy::Escape)
        } else {
            crate::keys::shortcut(&press.key, press.physical, press.modifiers)
                .map(AppCopy::Shortcut)
        };
        // Typed after something opened or was pressed in its batch, or
        // after the view lost the keyboard (the note it opened has it):
        // dropped, and so is the app's copy.
        if self.task_list.stopped || key.after_press || !self.task_view_has_keys() {
            if let Some(copy) = copy {
                self.task_list.copies.push_back((copy, true));
            }
            return Task::none();
        }
        // A held key's repeats toggle nothing and open nothing: a write or
        // a menu at the keyboard's repeat rate is no one's choice.
        if press.repeat && self.ignores_repeat(&press, copy.as_ref()) {
            if let Some(copy) = copy {
                self.task_list.copies.push_back((copy, true));
            }
            return Task::none();
        }
        // Any key that reaches an open menu, a chord it holds included,
        // leaves no press after it in the batch a choice.
        if self.task_list.menu.is_some() {
            self.task_list.menu_keyed = true;
        }
        match copy {
            // Escape is read here, in its place: it closes the menu, which
            // stops the batch, or gives a control's keyboard back to the
            // list. The app's copy has nothing left to do.
            Some(AppCopy::Escape) => {
                self.task_list.copies.push_back((AppCopy::Escape, true));
                if self.close_task_menu() {
                    self.task_list.stopped = true;
                } else {
                    self.unfocus_task_control();
                }
                Task::none()
            }
            // F2 edits the selected row's description in place: the view's
            // own key, whatever the keymap gives it elsewhere.
            Some(AppCopy::Shortcut(accel)) if super::task_describe::is_describe_key(&accel) => {
                self.task_list
                    .copies
                    .push_back((AppCopy::Shortcut(accel), true));
                let (task, stops) = self.describe_key();
                if stops {
                    self.task_list.stopped = true;
                }
                task
            }
            // The view's own chords (toggle and undo) act here, in their
            // place in the batch, and so do the app's own shortcuts: one
            // that takes the keyboard from the list (the palette, a
            // dialog, another surface) stops the rest of the batch, which
            // was typed for the list. Either way its copy is spent. A
            // shortcut of no App command is left to the app's copy.
            Some(AppCopy::Shortcut(accel)) => {
                let id = (self.preferences.keymap.action(&accel))
                    .and_then(super::commands::CommandId::parse);
                if let Some(task) = id.and_then(|id| self.task_view_shortcut(id)) {
                    self.task_list
                        .copies
                        .push_back((AppCopy::Shortcut(accel), true));
                    return task;
                }
                let app = id.filter(|id| id.scope() == super::commands::Scope::App);
                let Some(id) = app else {
                    self.task_list
                        .copies
                        .push_back((AppCopy::Shortcut(accel), false));
                    return Task::none();
                };
                self.task_list
                    .copies
                    .push_back((AppCopy::Shortcut(accel), true));
                let task = self.run_shortcut(id);
                if !self.task_view_has_keys() {
                    self.task_list.stopped = true;
                }
                task
            }
            None => {
                let (task, stops) = self.task_list_press(&press);
                if stops {
                    self.task_list.stopped = true;
                }
                task
            }
        }
    }

    /// Whether a repeat of `press`, whose copy for the app is `copy`, does
    /// nothing: Space, Enter, the toggle and undo chords, + and -, and
    /// every key that opens something (S, P, D, E, F2, ↓ on a control with
    /// a menu), and on an open menu all but its arrows.
    fn ignores_repeat(&self, press: &Press, copy: Option<&AppCopy>) -> bool {
        use super::task_rows::RowAct;
        match copy {
            Some(AppCopy::Escape) => false,
            Some(AppCopy::Shortcut(accel)) => {
                super::task_describe::is_describe_key(accel)
                    || matches!(
                        self.preferences.keymap.action(accel),
                        Some("markdown.toggle_task" | "edit.undo")
                    )
            }
            None if self.task_list.menu.is_some() => !matches!(
                press.key,
                Key::Named(Named::ArrowUp | Named::ArrowDown | Named::PageUp | Named::PageDown)
            ),
            None => {
                matches!(press.key, Key::Named(Named::Enter | Named::Space))
                    || matches!(
                        RowAct::of(press),
                        Some(
                            RowAct::StatusMenu
                                | RowAct::PriorityMenu
                                | RowAct::Calendar
                                | RowAct::Form
                                | RowAct::Raise
                                | RowAct::Lower
                        )
                    )
                    || (press.key == Key::Named(Named::ArrowDown)
                        && self.task_list.focus.is_some_and(Control::opens_menu))
            }
        }
    }

    /// Whether the app's copy `copy` of a key the view reported is dropped.
    /// Each copy is matched to the report it copies, in order; a copy of
    /// a key the view never reported is the app's.
    pub(super) fn take_task_list_copy(&mut self, copy: &AppCopy) -> bool {
        match self.task_list.copies.front() {
            Some((front, drop)) if front == copy => {
                let drop = *drop;
                self.task_list.copies.pop_front();
                drop
            }
            _ => false,
        }
    }

    /// [`App::take_task_list_copy`] for the app's copy of the shortcut
    /// `accel`, read without copying it.
    pub(super) fn take_task_list_shortcut(&mut self, accel: &wren_core::keybind::Accel) -> bool {
        match self.task_list.copies.front() {
            Some((AppCopy::Shortcut(front), drop)) if front == accel => {
                let drop = *drop;
                self.task_list.copies.pop_front();
                drop
            }
            _ => false,
        }
    }

    /// A key for the view: the menu's while one is open, else the focused
    /// control's, else the list's. Returns whether it stops its batch.
    fn task_list_press(&mut self, press: &Press) -> (Task<Message>, bool) {
        let shift = press.modifiers.shift();
        if let Some(menu) = &mut self.task_list.menu {
            return match menu.key(
                &press.key,
                press.physical,
                press.modifiers,
                press.text.as_deref(),
            ) {
                MenuKey::Choose(row) => (self.choose_task_menu(row), true),
                MenuKey::Moved => (self.reveal_task_menu(), false),
                MenuKey::Held => (Task::none(), false),
            };
        }
        if press.key == Key::Named(Named::Tab) {
            self.step_focus(if shift { -1 } else { 1 }, false);
            return (Task::none(), false);
        }
        if let Some(control) = self.task_list.focus {
            return match &press.key {
                Key::Named(Named::ArrowLeft) => {
                    self.step_focus(-1, true);
                    (Task::none(), false)
                }
                Key::Named(Named::ArrowRight) => {
                    self.step_focus(1, true);
                    (Task::none(), false)
                }
                Key::Named(Named::Enter | Named::Space) => {
                    (self.press_control(control), control.opens_menu())
                }
                Key::Named(Named::ArrowDown) if control.opens_menu() => {
                    (self.press_control(control), true)
                }
                _ => (Task::none(), false),
            };
        }
        if let Some(act) = super::task_rows::RowAct::of(press) {
            return self.row_key(act);
        }
        let page = ((self.list_height() / ROW_HEIGHT).floor() as isize - 1).max(1);
        let latin = press.key.to_latin(press.physical);
        let task = match (&press.key, latin) {
            (Key::Named(Named::ArrowDown), _) => self.step_selection(1),
            (Key::Named(Named::ArrowUp), _) => self.step_selection(-1),
            (Key::Named(Named::PageDown), _) => self.step_selection(page),
            (Key::Named(Named::PageUp), _) => self.step_selection(-page),
            (Key::Named(Named::Home), _) => self.step_selection(isize::MIN / 2),
            (Key::Named(Named::End), _) => self.step_selection(isize::MAX / 2),
            (Key::Named(Named::Enter), _) => {
                let entry = self
                    .task_list
                    .selected
                    .as_ref()
                    .map(|selected| selected.entry);
                return match entry {
                    Some(entry) => (self.open_task_at(entry), true),
                    None => (Task::none(), false),
                };
            }
            (_, Some('j')) if !shift => self.step_selection(1),
            (_, Some('k')) if !shift => self.step_selection(-1),
            _ => Task::none(),
        };
        (task, false)
    }

    /// Press a control: a chip toggles, Show all clears the chips, the
    /// checkbox flips, and the others open their menu.
    fn press_control(&mut self, control: Control) -> Task<Message> {
        match control {
            Control::Chip(chip) => {
                let status = task_list::CHIPS[chip];
                self.change_filters(|filters| {
                    match filters
                        .statuses
                        .iter()
                        .position(|pressed| *pressed == status)
                    {
                        Some(at) => {
                            filters.statuses.remove(at);
                        }
                        None => filters.statuses.push(status),
                    }
                });
                // Show all goes with the last chip; the keyboard stays on
                // the chips.
                Task::none()
            }
            Control::ShowAll => {
                self.change_filters(|filters| filters.statuses.clear());
                self.task_list.focus = Some(Control::Chip(0));
                Task::none()
            }
            Control::HideOld => {
                self.change_filters(|filters| filters.hide_old = !filters.hide_old);
                Task::none()
            }
            Control::Scope
            | Control::Owner
            | Control::GroupBy
            | Control::Due
            | Control::Priority => self.open_list_menu(control),
            Control::List | Control::Source(_) | Control::Row(_) | Control::Cell(..) => {
                Task::none()
            }
        }
    }

    /// Change the view's filters with `change`, remembering them for its
    /// scope when they differ.
    pub(super) fn change_filters(&mut self, change: impl FnOnce(&mut TaskFilters)) {
        let Some(scope) = self.task_view().cloned() else {
            return;
        };
        let Ok(item) = self.scope_item(&scope) else {
            return;
        };
        let mut filters = self.task_filters(&scope);
        let before = filters.clone();
        change(&mut filters);
        if filters != before {
            // The rows held move to where the new filters put them.
            self.task_list.held.clear();
            self.task_list.describe = None;
            let _ = self.process_core(Op::SetTaskFilters {
                scope: item,
                filters,
            });
        }
    }

    /// The scopes "Show tasks in" offers for the view of `scope`: the
    /// workspace, and the section, group and note it lies in, from the
    /// scope and, below it, from the tree's selection.
    fn scope_choices(&self, scope: &Scope) -> Vec<Option<Scope>> {
        let Some(model) = self.model() else {
            return vec![Some(Scope::Workspace), None, None, None];
        };
        let path = |item: Option<ItemRef>| item.and_then(|item| model.path(item)).cloned();
        let tree_section = path(self.selected_section.map(ItemRef::Section));
        let tree_group = path(self.selected_group.map(ItemRef::Group));
        let tree_note = path(self.selected_note.map(ItemRef::Note));
        let parent = |path: &RelPath| path.parent();
        let section = match scope {
            Scope::Workspace => tree_section,
            Scope::Section(path) => Some(path.clone()),
            Scope::Group(path) => parent(path),
            Scope::Note(path) => parent(path).as_ref().and_then(parent),
        };
        let group = match scope {
            Scope::Group(path) => Some(path.clone()),
            Scope::Note(path) => parent(path),
            _ => tree_group.filter(|group| parent(group) == section),
        };
        let note = match scope {
            Scope::Note(path) => Some(path.clone()),
            _ => tree_note.filter(|note| group.is_some() && parent(note) == group),
        };
        vec![
            Some(Scope::Workspace),
            section.map(Scope::Section),
            group.map(Scope::Group),
            note.map(Scope::Note),
        ]
    }

    /// Open the menu of `control`, under it.
    fn open_list_menu(&mut self, control: Control) -> Task<Message> {
        let Some(scope) = self.task_view().cloned() else {
            return Task::none();
        };
        let filters = self.task_filters(&scope);
        let (purpose, title, rows) = match control {
            Control::Scope => {
                let choices = self.scope_choices(&scope);
                let rows = choices
                    .iter()
                    .zip([
                        ("Workspace", 'w'),
                        ("Section", 's'),
                        ("Group", 'g'),
                        ("Note", 'n'),
                    ])
                    .enumerate()
                    .map(|(id, (choice, (kind, key)))| {
                        let label = match choice {
                            Some(choice) => format!(
                                "{} ({})",
                                self.task_view_label(choice),
                                kind.to_lowercase()
                            ),
                            None => kind.to_owned(),
                        };
                        let current = choice.as_ref() == Some(&scope);
                        Row::new(id as u32, label)
                            .accelerator(Accelerator::plain(key))
                            .glyph(if current { "\u{2713}" } else { "" })
                            .checked(current)
                            .enabled(choice.is_some())
                    })
                    .collect();
                (MenuFor::Scope(choices), Some("Show tasks in"), rows)
            }
            Control::Owner => {
                let names = self.task_list.listing.owners.clone();
                let mut rows = vec![
                    Row::new(0, "Anyone")
                        .accelerator(Accelerator::plain('a'))
                        .checked(filters.owner == OwnerFilter::Anyone),
                    Row::new(1, "Me")
                        .accelerator(Accelerator::plain('m'))
                        .detail("no owner, or tasks-me")
                        .checked(filters.owner == OwnerFilter::Me),
                ];
                for (index, name) in names.iter().enumerate() {
                    let checked = matches!(&filters.owner, OwnerFilter::Name(chosen)
                        if wren_markdown::task::fold_name(chosen) == wren_markdown::task::fold_name(name));
                    rows.push(Row::new(2 + index as u32, format!("@{name}")).checked(checked));
                }
                (MenuFor::Owner(names), Some("Owner"), rows)
            }
            Control::GroupBy => {
                let rows = GROUPINGS
                    .iter()
                    .enumerate()
                    .map(|(id, (group, label, key))| {
                        Row::new(id as u32, *label)
                            .accelerator(Accelerator::plain(*key))
                            .checked(filters.group_by == *group)
                    })
                    .collect();
                (MenuFor::GroupBy, Some("Group by"), rows)
            }
            Control::Due => {
                let mut rows = vec![Row::new(0, "Any date")
                    .accelerator(Accelerator::plain('a'))
                    .checked(filters.due.is_none())];
                rows.extend(DueBucket::ALL.iter().zip(BUCKET_KEYS).enumerate().map(
                    |(index, (bucket, key))| {
                        Row::new(1 + index as u32, bucket.label())
                            .accelerator(Accelerator::plain(key))
                            .checked(filters.due == Some(*bucket))
                    },
                ));
                (MenuFor::Due, Some("Due"), rows)
            }
            Control::Priority => {
                let mut rows = vec![Row::new(0, "Any priority")
                    .accelerator(Accelerator::plain('a'))
                    .checked(filters.priority.is_none())];
                rows.extend(super::task_menus::PRIORITY_ROWS.iter().enumerate().map(
                    |(index, (level, key))| {
                        let (glyph, name) = level.map_or(("\u{b7}".to_owned(), "None"), |level| {
                            (level.glyph().to_string(), level.name())
                        });
                        let filter = level.map_or(PriorityFilter::Unset, PriorityFilter::Level);
                        Row::new(1 + index as u32, name)
                            .glyph(glyph)
                            .accelerator(*key)
                            .checked(filters.priority == Some(filter))
                    },
                ));
                (MenuFor::Priority, Some("Priority"), rows)
            }
            _ => return Task::none(),
        };
        let anchor = self
            .task_list
            .drawn(control)
            .unwrap_or_else(|| self.unplaced_anchor());
        self.open_host_menu(purpose, title.map(str::to_owned), rows, anchor, None)
    }

    /// Where a menu hangs whose control was never drawn.
    pub(super) fn unplaced_anchor(&self) -> Rectangle {
        Rectangle::new(
            iced::Point::new(self.viewport.width / 2.0 - 100.0, 96.0),
            iced::Size::ZERO,
        )
    }

    /// Open a menu over the view, under `anchor`, in place of any open,
    /// with the row `selected` selected, or else the checked one.
    pub(super) fn open_host_menu(
        &mut self,
        purpose: MenuFor,
        title: Option<String>,
        rows: Vec<Row>,
        anchor: Rectangle,
        selected: Option<u32>,
    ) -> Task<Message> {
        self.task_list.describe = None;
        self.task_list.menus += 1;
        let mut menu = HostMenu::new(self.task_list.menus, purpose, title, rows, anchor);
        if let Some(id) = selected {
            menu.select_id(id);
        }
        self.task_list.menu = Some(menu);
        self.reveal_task_menu()
    }

    /// Scroll the open menu's rows, when they scroll, to show its selected
    /// row.
    fn reveal_task_menu(&mut self) -> Task<Message> {
        let window = self.viewport;
        match self
            .task_list
            .menu
            .as_mut()
            .and_then(|menu| menu.reveal(window))
        {
            Some(y) => iced::widget::operation::scroll_to(
                host_menu::ROWS,
                iced::widget::operation::AbsoluteOffset { x: 0.0, y },
            ),
            None => Task::none(),
        }
    }

    /// Act on the row `row` of the open menu, and close it.
    fn choose_task_menu(&mut self, row: u32) -> Task<Message> {
        let Some(menu) = self.task_list.menu.take() else {
            return Task::none();
        };
        if let MenuFor::Row {
            target,
            boxed,
            entry,
            generation,
            ..
        } = menu.purpose
        {
            return self.choose_row_menu(&target, boxed, entry, generation, row);
        }
        let row = row as usize;
        match menu.purpose {
            MenuFor::Scope(choices) => {
                if let Some(Some(scope)) = choices.get(row).cloned() {
                    if self.task_view() != Some(&scope) {
                        return self
                            .dispatch(super::commands::Command::ShowTasks(scope))
                            .task;
                    }
                }
            }
            MenuFor::Owner(names) => {
                let owner = match row {
                    0 => Some(OwnerFilter::Anyone),
                    1 => Some(OwnerFilter::Me),
                    n => names.get(n - 2).cloned().map(OwnerFilter::Name),
                };
                if let Some(owner) = owner {
                    self.change_filters(|filters| filters.owner = owner);
                }
            }
            MenuFor::GroupBy => {
                if let Some((group, ..)) = GROUPINGS.get(row) {
                    self.change_filters(|filters| filters.group_by = *group);
                }
            }
            MenuFor::Due => {
                let due = match row {
                    0 => Some(None),
                    n => DueBucket::ALL.get(n - 1).copied().map(Some),
                };
                if let Some(due) = due {
                    self.change_filters(|filters| filters.due = due);
                }
            }
            MenuFor::Priority => {
                let priority = match row {
                    0 => Some(None),
                    n => (super::task_menus::PRIORITY_ROWS.get(n - 1)).map(|(level, _)| {
                        Some(level.map_or(PriorityFilter::Unset, PriorityFilter::Level))
                    }),
                };
                if let Some(priority) = priority {
                    self.change_filters(|filters| filters.priority = priority);
                }
            }
            MenuFor::Row { .. } => {}
        }
        Task::none()
    }

    /// Close the menu over the view, if one is open: Escape's step in its
    /// precedence, after the calendar and the form.
    pub(super) fn close_task_menu(&mut self) -> bool {
        self.task_list.menu.take().is_some()
    }

    /// Escape's step for a control: the keyboard goes back to the list.
    pub(super) fn unfocus_task_control(&mut self) -> bool {
        self.task_view_has_keys() && self.task_list.focus.take().is_some()
    }

    /// Open the note of the entry `entry` at its task's line, and give the
    /// editor the keyboard with the caret there.
    pub(super) fn open_task_at(&mut self, entry: usize) -> Task<Message> {
        let Some(record) = self.task_list.listing.record(entry) else {
            return Task::none();
        };
        let task = super::task_target::RowTarget::of(record);
        self.open_task(&task)
    }

    /// Open the note of `task` at the line a write would find it on, and
    /// give the editor the keyboard with the caret there.
    pub(super) fn open_task(&mut self, record: &super::task_target::RowTarget) -> Task<Message> {
        let target = SearchTarget {
            generation: self.workspace_generation(),
            index_revision: self
                .session
                .as_ref()
                .map_or(0, |session| session.index().revision()),
            note: record.note,
            query: String::new(),
            content_match: None,
        };
        // The note already open under the view records no visit: it shows
        // after the view as well, so Back from it is the view.
        let open = (self.session.as_ref())
            .and_then(|session| session.document())
            .map(|document| document.note);
        let under = self
            .task_views
            .shown_id()
            .filter(|_| open == Some(record.note));
        let outcome = self.activate_note(target);
        if let Some(stop) = under
            .filter(|_| outcome.processed.disposition.committed() && self.task_view().is_none())
        {
            self.task_views.pass(stop);
        }
        if !outcome.processed.disposition.committed()
            || self.selected_note != Some(record.note)
            || self.task_view().is_some()
            || self.surface() != super::board::Surface::Text
        {
            return outcome.task;
        }
        // The task where a write would find it: on the line the record
        // names while it is still that task, else the one task line of the
        // same text (a twin in a fence is no task). With none, or two, the
        // note opens where it was, and says why.
        let text = self.editor.text();
        let Some(line) = wren_core::task_write::locate(&text, record.line, &record.expect) else {
            self.notify(super::notice::Notice::new(vec![super::notice::Part::Text(
                STALE_OPEN.into(),
            )]));
            self.column = Column::Editor;
            return Task::batch([outcome.task, self.column_focus()]);
        };
        self.editor
            .scroll_to_line(line, wren_editor::Placement::NearTop);
        self.editor.move_cursor(iced::widget::text_editor::Cursor {
            position: iced::widget::text_editor::Position { line, column: 0 },
            selection: None,
        });
        self.column = Column::Editor;
        Task::batch([outcome.task, self.column_focus()])
    }

    /// Input from the list.
    pub(super) fn task_list_event(&mut self, event: Event) -> Task<Message> {
        // A key from the view goes through its batch's gate before anything
        // asks what the pane now shows.
        if let Event::Key(key) = event {
            return self.task_list_key(key);
        }
        if let Event::Field {
            serial,
            instance,
            event,
        } = event
        {
            return self.describe_event(serial, instance, event);
        }
        if let Event::Click {
            click: Click::Menu(host_menu::Event::Scrolled { instance, y }),
            ..
        } = event
        {
            if let Some(menu) =
                (self.task_list.menu.as_mut()).filter(|menu| menu.instance() == instance)
            {
                menu.scrolled(y);
            }
            return Task::none();
        }
        if let Event::Click { serial, .. } = event {
            self.enter_batch(serial);
        }
        if self.task_view().is_none() || self.dialog.is_some() || self.palette.is_some() {
            return Task::none();
        }
        match event {
            Event::Press => {
                self.column = Column::Editor;
                self.column_focus()
            }
            Event::Scrolled { y, height } => {
                // The user scrolling the list ends an edit in place, writing
                // nothing, before any key after it in the batch: only the
                // list's own scroll (the reveal of the row as the field
                // opens), which is where the view last put it, keeps it.
                if (y - self.task_list.scroll).abs() > 0.5 && self.cancel_describe() {
                    self.task_list.stopped = true;
                }
                self.task_list.scroll = y;
                self.task_list.height = Some(height);
                // An edit whose row is out of the rows built has no field.
                if (self.task_list.describe.as_ref())
                    .is_some_and(|describe| !self.built_rows().contains(&describe.entry))
                {
                    self.task_list.describe = None;
                }
                Task::none()
            }
            // A row of the menu released on: chosen only when its press was
            // allowed to choose it, as it was decided at the press.
            Event::Click {
                click: Click::Menu(host_menu::Event::Choose { instance, row }),
                ..
            } => {
                self.task_list.stopped = true;
                if self.task_list.armed.take() != Some((instance, row)) {
                    return Task::none();
                }
                let chosen =
                    (self.task_list.menu.as_ref()).and_then(|menu| menu.choose(instance, row));
                match chosen {
                    Some(row) => self.choose_task_menu(row),
                    None => Task::none(),
                }
            }
            Event::Click { click, .. } => {
                // A press ends what its batch may still do; one after
                // something opened in the batch, or after another press, is
                // dropped, but for the second press of a double-click on a
                // description, which selects its row and then edits it.
                let first = !std::mem::replace(&mut self.task_list.stopped, true);
                // A press over the menu's card may choose the row it is on
                // at its release: only the batch's first press, of the
                // primary button, with no key reaching the menu before it.
                if let Click::Menu(host_menu::Event::Pressed {
                    instance,
                    at,
                    primary,
                }) = click
                {
                    let window = self.viewport;
                    self.task_list.armed = (first && primary && !self.task_list.menu_keyed)
                        .then_some(self.task_list.menu.as_ref())
                        .flatten()
                        .filter(|menu| menu.instance() == instance)
                        .and_then(|menu| menu.row_at(at, window))
                        .map(|row| (instance, row));
                    return Task::none();
                }
                // The exception is spent on the one edit it lets open.
                let second = matches!(click, Click::Describe { generation, entry }
                    if self.task_list.selected_in_batch == Some((generation, entry)));
                if second {
                    self.task_list.selected_in_batch = None;
                }
                if !first && !second {
                    return Task::none();
                }
                self.task_list_click(click)
            }
            Event::Key(_) | Event::Field { .. } => Task::none(),
        }
    }

    /// A click, read in its batch.
    fn task_list_click(&mut self, click: Click) -> Task<Message> {
        let current = |app: &Self, generation: u64, entry: usize| {
            generation == app.task_list.generation && app.task_list.listing.record(entry).is_some()
        };
        // A click anywhere in the view ends an edit in place, writing
        // nothing, before it does what it does.
        self.task_list.describe = None;
        match click {
            Click::Select { generation, entry } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.task_list.selected_in_batch = Some((generation, entry));
                self.column = Column::Editor;
                self.task_list.focus = None;
                let task = self.select_entry(entry);
                Task::batch([task, self.column_focus()])
            }
            Click::Open { generation, entry } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.column = Column::Editor;
                self.task_list.focus = None;
                let _ = self.select_entry(entry);
                self.open_task_at(entry)
            }
            Click::Cell {
                generation,
                entry,
                cell,
            } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.column = Column::Editor;
                self.task_list.focus = None;
                let selected = self.select_entry(entry);
                let acted = self.click_cell(entry, cell);
                // The calendar the click opened takes the keyboard alone.
                if self.calendar.is_some() || self.task_form.is_some() {
                    return Task::batch([selected, acted]);
                }
                Task::batch([selected, acted, self.column_focus()])
            }
            Click::Describe { generation, entry } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.column = Column::Editor;
                self.task_list.focus = None;
                self.open_describe(entry)
            }
            Click::RowMenu { generation, entry } => {
                if !current(self, generation, entry) {
                    return Task::none();
                }
                self.column = Column::Editor;
                self.task_list.focus = None;
                let selected = self.select_entry(entry);
                let opened = self.open_entry_menu(super::task_menus::MenuKind::Task, entry);
                Task::batch([selected, opened, self.column_focus()])
            }
            Click::Control(control) => {
                // A click leaves the keyboard with the list: only keys
                // walk the controls.
                self.column = Column::Editor;
                let task = self.press_control(control);
                self.task_list.focus = None;
                Task::batch([task, self.column_focus()])
            }
            Click::Menu(event) => match event {
                host_menu::Event::Outside { instance } => {
                    if (self.task_list.menu.as_ref())
                        .is_some_and(|menu| menu.instance() == instance)
                    {
                        self.task_list.menu = None;
                    }
                    Task::none()
                }
                host_menu::Event::Scrolled { .. }
                | host_menu::Event::Pressed { .. }
                | host_menu::Event::Choose { .. } => Task::none(),
            },
        }
    }

    /// The menu over the view, as the window's overlay layers.
    pub(super) fn task_menu_layers(&self) -> Option<(Element<'_, Message>, Element<'_, Message>)> {
        let menu = self
            .task_list
            .menu
            .as_ref()
            .filter(|_| self.task_view().is_some())?;
        // The view's serial: the menu's input is read in the view's batch.
        let serial = self.task_list.serials.get();
        Some(menu.layers(self.definition, self.viewport, move |event| {
            Message::TaskView(Event::Click {
                serial,
                click: Click::Menu(event),
            })
        }))
    }

    /// The scope chip in the view's header: it opens "Show tasks in".
    pub(super) fn task_scope_chip(&self, scope: &Scope) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let label = self.task_view_label(scope);
        let focused = self.task_list.focus == Some(Control::Scope) && self.task_view_has_keys();
        let content = row![
            chrome::icon(chrome::Icon::Tasks, 14.0, colors.accent),
            chrome::label("Tasks in", 13, colors.body),
            ellipsis(label, 13.0, theme::color(colors.text))
                .font(fonts::SEMIBOLD)
                .width(Shrink),
            chrome::label(format!("{} \u{25be}", scope.kind()), 11, colors.faint),
        ]
        .spacing(6)
        .align_y(iced::Center);
        let definition = self.definition;
        let chip = button(content)
            .padding([3, 8])
            .on_press(Message::TaskView(Event::Click {
                serial: self.task_list.serials.get(),
                click: Click::Control(Control::Scope),
            }))
            .style(move |_, status| control_style(definition, status, false, focused));
        container(probe(chip, &self.task_list.drawn, Control::Scope))
            .id("task-view-scope")
            .clip(true)
            .into()
    }

    /// The view's body: its filters over its list.
    pub(super) fn task_list_body(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let list = &self.task_list;
        // Each view built is one batch of input.
        let serial = list.next_serial();
        let filters = list
            .built
            .as_ref()
            .map(|built| built.filters.clone())
            .unwrap_or_default();
        let has_keys = self.task_view_has_keys();
        let definition = self.definition;
        let focused = |control: Control| has_keys && list.focus == Some(control);
        let send = move |control: Control| {
            Message::TaskView(Event::Click {
                serial,
                click: Click::Control(control),
            })
        };
        let mut tools = row![chrome::label("Status", 12, colors.muted)]
            .spacing(6)
            .align_y(iced::Center);
        for (chip, status) in task_list::CHIPS.iter().enumerate() {
            let on = filters.statuses.contains(status);
            let name = task_list::CHIPS[chip].name();
            let mut title: String = name[..1].to_uppercase();
            title.push_str(&name[1..]);
            let count = list.listing.chip_counts[chip];
            let label = row![
                chrome::label(title, 12, if on { colors.accent } else { colors.muted })
                    .font(if on { fonts::SEMIBOLD } else { fonts::UI }),
                chrome::label(
                    count.to_string(),
                    12,
                    if on { colors.accent } else { colors.faint }
                ),
            ]
            .spacing(4);
            let is_focused = focused(Control::Chip(chip));
            let chip_button = button(label)
                .padding([2, 9])
                .on_press(send(Control::Chip(chip)))
                .style(move |_, status| chip_style(definition, status, on, is_focused));
            tools = tools.push(probe(chip_button, &list.drawn, Control::Chip(chip)));
        }
        if filters.statuses.is_empty() {
            tools = tools.push(chrome::label("All shown", 12, colors.faint));
        } else {
            let is_focused = focused(Control::ShowAll);
            let show_all = button(chrome::label("Show all", 12, colors.link))
                .padding([2, 4])
                .on_press(send(Control::ShowAll))
                .style(move |_, status| control_style(definition, status, true, is_focused));
            tools = tools.push(probe(show_all, &list.drawn, Control::ShowAll));
        }
        let owner = match &filters.owner {
            OwnerFilter::Anyone => "Anyone".to_owned(),
            OwnerFilter::Me => "Me".to_owned(),
            OwnerFilter::Name(name) => format!("@{name}"),
        };
        let group = GROUPINGS
            .iter()
            .find(|(group, ..)| *group == filters.group_by)
            .map_or("Note", |(_, label, _)| label);
        let due = filters.due.map_or("Any", DueBucket::label);
        let priority = match filters.priority {
            None => "Any".to_owned(),
            Some(PriorityFilter::Unset) => "None".to_owned(),
            Some(PriorityFilter::Level(level)) => format!("{} {}", level.glyph(), level.name()),
        };
        for (control, caption, value) in [
            (Control::Owner, "Owner", owner),
            (Control::GroupBy, "Group by", group.to_owned()),
            (Control::Due, "Due", due.to_owned()),
            (Control::Priority, "Priority", priority),
        ] {
            let is_focused = focused(control);
            let open = list
                .menu
                .as_ref()
                .is_some_and(|menu| menu_control(&menu.purpose) == control);
            let picker = button(
                row![
                    chrome::label(value, 12, colors.text),
                    chrome::label("\u{25be}", 10, colors.faint)
                ]
                .spacing(4)
                .align_y(iced::Center),
            )
            .padding([2, 6])
            .on_press(send(control))
            .style(move |_, status| picker_style(definition, status, open, is_focused));
            tools = tools.push(
                row![
                    chrome::label(caption, 12, colors.muted),
                    probe(picker, &list.drawn, control)
                ]
                .spacing(5)
                .align_y(iced::Center),
            );
        }
        let is_focused = focused(Control::HideOld);
        let tick = if filters.hide_old {
            task_box(StatusType::Done, &colors)
        } else {
            task_box(StatusType::Open, &colors)
        };
        let hide_old = button(
            row![
                tick,
                chrome::label(
                    format!("Hide done after {} days", task_list::OLD_DAYS),
                    12,
                    colors.muted
                )
            ]
            .spacing(5)
            .align_y(iced::Center),
        )
        .padding([2, 4])
        .on_press(send(Control::HideOld))
        .style(move |_, status| control_style(definition, status, true, is_focused));
        tools = tools.push(probe(hide_old, &list.drawn, Control::HideOld));
        let tools = chrome::bottom_rule(
            container(tools.wrap().vertical_spacing(6.0))
                .padding([9, 14])
                .width(Fill),
            self.definition,
        );
        let body: Element<'_, Message> = if list.listing.entries.is_empty() {
            let scope = self.task_view().cloned().unwrap_or(Scope::Workspace);
            container(
                chrome::label(
                    empty_hint(
                        &filters,
                        &self.task_view_label(&scope),
                        list.listing.old_hidden > 0,
                    ),
                    13,
                    colors.muted,
                )
                .wrapping(iced::widget::text::Wrapping::Word)
                .align_x(iced::Center),
            )
            .padding([40, 20])
            .center_x(Fill)
            .into()
        } else {
            self.list_rows(&filters)
        };
        let content = mouse_area(
            container(column![tools, body].width(Fill).height(Fill))
                .id("task-view")
                .style(move |_| chrome::surface(definition, colors.editor, 0.0, false)),
        )
        .on_press(Message::TaskView(Event::Press));
        // A description edited in place has the keys in its field.
        list_keys(
            content,
            has_keys && list.describe.is_none(),
            serial,
            |key| Message::TaskView(Event::Key(key)),
        )
        .into()
    }

    /// The entries whose rows are built: those in view, and a screen's
    /// worth on each side, so a scroll shows built rows.
    pub(super) fn built_rows(&self) -> std::ops::Range<usize> {
        let height = self.list_height();
        (self.task_list.listing).visible((self.task_list.scroll - height).max(0.0), 3.0 * height)
    }

    /// The rows in view, between spaces for those out of it.
    fn list_rows(&self, filters: &TaskFilters) -> Element<'_, Message> {
        let list = &self.task_list;
        let listing = &list.listing;
        let visible = self.built_rows();
        // Only the rows built are drawn: forget where the others were.
        (list.drawn.borrow_mut()).retain(|(control, _)| match control {
            Control::Source(entry) | Control::Row(entry) | Control::Cell(entry, _) => {
                visible.contains(entry)
            }
            _ => true,
        });
        let mut rows = column![Space::new().height(listing.tops[visible.start])].width(Fill);
        let by_note = filters.group_by == GroupBy::Note;
        for entry in visible.clone() {
            rows = rows.push(match listing.entries[entry] {
                Entry::Header(group) => self.list_header(group),
                Entry::Row { record, .. } => {
                    self.list_row(entry, &listing.records[record], by_note)
                }
            });
        }
        rows = rows.push(Space::new().height(listing.height() - listing.tops[visible.end]));
        let definition = self.definition;
        let scroller = scrollable(rows)
            .id(LIST)
            .height(Fill)
            .on_scroll(|viewport| {
                Message::TaskView(Event::Scrolled {
                    y: viewport.absolute_offset().y,
                    height: viewport.bounds().height,
                })
            })
            .style(move |_, status| chrome::scrollbar(definition, status));
        probe(scroller, &list.drawn, Control::List).into()
    }

    fn list_header(&self, group: usize) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let group = &self.task_list.listing.groups[group];
        let label = row![
            chrome::label(group.label.to_uppercase(), 11, colors.muted).font(fonts::SEMIBOLD),
            chrome::label(group.detail.clone(), 11, colors.faint),
            chrome::label(group.rows.len().to_string(), 11, colors.faint),
        ]
        .spacing(8)
        .align_y(iced::Alignment::End);
        container(label)
            .height(HEADER_HEIGHT)
            .padding(iced::Padding {
                top: 12.0,
                right: 18.0,
                bottom: 4.0,
                left: 18.0,
            })
            .clip(true)
            .into()
    }

    fn list_row<'a>(
        &'a self,
        entry: usize,
        record: &'a TaskRecord,
        by_note: bool,
    ) -> Element<'a, Message> {
        let colors = self.definition.colors;
        let today = self.today();
        let finished = record.status.is_finished();
        let selected = self
            .task_list
            .selected
            .as_ref()
            .is_some_and(|selected| selected.entry == entry);
        let keyed = selected && self.task_list.focus.is_none() && self.task_view_has_keys();
        let pieces = task_list::pieces(record);
        let text_color = theme::color(if finished {
            colors.completed
        } else {
            colors.text
        });
        let owner_color = theme::color(if finished {
            colors.completed
        } else {
            colors.owner
        });
        let mut runs: Vec<(String, iced::Color)> = Vec::new();
        if pieces.is_empty() {
            runs.push(("(no description)".into(), theme::color(colors.faint)));
        }
        for piece in pieces {
            runs.push(match piece {
                Piece::Text(text) => (text.to_owned(), text_color),
                Piece::Owner(owner) if owner.starts_with('@') => (owner.to_owned(), owner_color),
                Piece::Owner(owner) => (format!(" @{owner}"), owner_color),
            });
        }
        // One line, ending in "…" when the row is too narrow for it.
        let description = ellipsis_runs(runs, 13.5, text_color).strikethrough(finished);
        let source = source_text(record, self.note_title(record.note), by_note);
        let serial = self.task_list.serials.get();
        let generation = self.task_list.generation;
        // The description, or the field that edits it in place; a
        // double-click on the description opens that.
        // The click history behind a double-click is this row's in this
        // listing: a row that takes its place after a rebuild starts afresh.
        let description: Element<'a, Message> =
            match (self.task_list.describe.as_ref()).filter(|describe| describe.entry == entry) {
                Some(describe) => self.describe_field(describe, serial),
                None => list_keys::keyed(
                    mouse_area(container(description).clip(true).width(Fill))
                        .on_press(Message::TaskView(Event::Click {
                            serial,
                            click: Click::Select { generation, entry },
                        }))
                        .on_double_click(Message::TaskView(Event::Click {
                            serial,
                            click: Click::Describe { generation, entry },
                        })),
                    (generation, entry),
                )
                .into(),
            };
        let description = probe(
            description,
            &self.task_list.drawn,
            Control::Cell(entry, Cell::Description),
        );
        let source = mouse_area(ellipsis(source, 11.5, theme::color(colors.faint)).width(Shrink))
            .on_press(Message::TaskView(Event::Click {
                serial,
                click: Click::Open { generation, entry },
            }))
            .interaction(iced::mouse::Interaction::Pointer);
        let source = probe(source, &self.task_list.drawn, Control::Source(entry));
        let priority = record
            .priority
            .map(|level| level.glyph().to_string())
            .unwrap_or_default();
        let due = due_cell(record, today);
        let due = match due {
            Some(cell) => {
                let (color, font) = match cell.look {
                    "overdue" => (colors.danger, fonts::SEMIBOLD),
                    "invalid" => (colors.danger, fonts::UI),
                    "finished" => (colors.faint, fonts::UI),
                    _ => (colors.muted, fonts::UI),
                };
                chrome::label(cell.text, 12, color).font(font)
            }
            None => chrome::label("", 12, colors.faint),
        };
        let cell = |content: Element<'a, Message>, cell: Cell| {
            let area = mouse_area(content)
                .on_press(Message::TaskView(Event::Click {
                    serial,
                    click: Click::Cell {
                        generation,
                        entry,
                        cell,
                    },
                }))
                .interaction(iced::mouse::Interaction::Pointer);
            probe(area, &self.task_list.drawn, Control::Cell(entry, cell))
        };
        let line = row![
            cell(
                container(row_box(record, &colors))
                    .center_x(26)
                    .center_y(ROW_HEIGHT - 4.0)
                    .into(),
                Cell::Box
            ),
            column![description, source].spacing(2).width(Fill),
            cell(
                container(chrome::label(priority, 13, colors.text))
                    .center_x(30)
                    .center_y(ROW_HEIGHT - 4.0)
                    .into(),
                Cell::Priority
            ),
            cell(
                container(due).width(112).center_y(ROW_HEIGHT - 4.0).into(),
                Cell::Due
            ),
        ]
        .spacing(4)
        .align_y(iced::Center)
        .padding([0, 10]);
        let card = container(line)
            .height(ROW_HEIGHT - 4.0)
            .center_y(ROW_HEIGHT - 4.0)
            .width(Fill)
            .clip(true)
            .style(move |_| {
                let (background, border) = if keyed {
                    (Some(colors.selected), colors.selected_border)
                } else if selected {
                    (Some(colors.selected_inactive), colors.control_border)
                } else {
                    (None, colors.editor)
                };
                iced::widget::container::Style {
                    background: background.map(|color| theme::color(color).into()),
                    border: iced::Border {
                        color: theme::color(border),
                        width: if background.is_some() { 1.0 } else { 0.0 },
                        radius: 6.0.into(),
                    },
                    ..Default::default()
                }
            });
        let area = mouse_area(container(card).padding([2, 8]).height(ROW_HEIGHT))
            .on_press(Message::TaskView(Event::Click {
                serial,
                click: Click::Select { generation, entry },
            }))
            .on_right_press(Message::TaskView(Event::Click {
                serial,
                click: Click::RowMenu { generation, entry },
            }));
        probe(area, &self.task_list.drawn, Control::Row(entry)).into()
    }

    /// The pane's footer under the view, `width` wide: its counts, what it
    /// leaves out, and as much of its keys' hint as the room they leave
    /// holds, in whole pieces from the first ([`App::task_list_hint`]).
    /// The counts never give way; what is left out ends in "…" only when
    /// the window is too narrow for it beside them.
    pub(super) fn task_list_footer(&self, width: f32) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let (counts, left) = self.task_list.footer.clone();
        let open = format!("{} open \u{b7} ", counts.open_and_doing);
        let overdue = format!("{} overdue", counts.overdue);
        let left = left.map(|left| format!(" \u{b7} {left}"));
        let label = |content: &str| self.label_widths.of(content, 11.0, fonts::UI);
        let used = label(&open) + label(&overdue) + left.as_deref().map_or(0.0, label);
        // The gaps either side of the space between the counts and the hint.
        let hint = self.task_list_hint(width - used - 2.0 * FOOTER_GAP);
        let mut pieces = row![
            chrome::label(open, 11, colors.muted),
            chrome::label(
                overdue,
                11,
                if counts.overdue > 0 {
                    colors.danger
                } else {
                    colors.muted
                }
            ),
        ];
        if let Some(left) = left {
            pieces = pieces.push(ellipsis(left, 11.0, theme::color(colors.muted)).width(Shrink));
        }
        let mut line = row![pieces.clip(true), iced::widget::space::horizontal()]
            .spacing(FOOTER_GAP)
            .align_y(iced::Center)
            .clip(true);
        if !hint.is_empty() {
            line = line.push(chrome::label(hint, 11, colors.faint));
        }
        line.into()
    }

    /// The pieces of the keys' hint ([`HINT`]) that fit in `room`, whole
    /// and from the first, with their separators; empty when not even the
    /// first does. A word is never cut.
    pub(super) fn task_list_hint(&self, room: f32) -> String {
        let pieces: Vec<&str> = HINT.split(HINT_SEPARATOR).collect();
        let widths = (pieces.iter().enumerate()).map(|(index, piece)| {
            let gap = if index > 0 {
                self.label_widths.of(HINT_SEPARATOR, 11.0, fonts::UI)
            } else {
                0.0
            };
            gap + self.label_widths.of(piece, 11.0, fonts::UI)
        });
        let shown = super::task_footer::pieces_that_fit(widths, room);
        pieces[..shown].join(HINT_SEPARATOR)
    }

    /// The footer's counts for `scope`, and what it says is left out.
    pub(super) fn task_list_counts(
        &self,
        scope: &Scope,
    ) -> (wren_core::tasks::Counts, Option<String>) {
        let Some(tasks) = self.session.as_ref().and_then(|session| session.tasks()) else {
            return (wren_core::tasks::Counts::default(), None);
        };
        let counts = tasks.counts_under(scope.prefix(), self.today());
        let left = tasks.left_out_under(scope.prefix());
        let plural = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        let mut parts = Vec::new();
        if left.hidden_notes > 0 {
            parts.push(format!(
                "{} ({}, tasks: off)",
                plural(left.hidden_notes, "note", "notes"),
                plural(left.hidden_tasks, "task", "tasks")
            ));
        }
        if left.excluded_notes > 0 {
            parts.push(format!(
                "{} in hidden folders ({})",
                plural(left.excluded_notes, "note", "notes"),
                plural(left.excluded_tasks, "task", "tasks")
            ));
        }
        if left.unreadable_notes > 0 {
            parts.push(plural(
                left.unreadable_notes,
                "unreadable note",
                "unreadable notes",
            ));
        }
        let left = (!parts.is_empty()).then(|| format!("Left out: {}", parts.join(", ")));
        (counts, left)
    }
}

/// The selection of the row at `entry`.
fn select(listing: &Listing, entry: usize) -> Selected {
    let (group, record) = match listing.entries[entry] {
        Entry::Row { group, record } => (group, record),
        Entry::Header(_) => unreachable!("only rows are selected"),
    };
    Selected {
        key: RowKey::of(&listing.records[record]),
        group: listing.groups[group].key.clone(),
        entry,
    }
}

/// The control a menu hangs under.
fn menu_control(purpose: &MenuFor) -> Control {
    match purpose {
        MenuFor::Scope(_) => Control::Scope,
        MenuFor::Owner(_) => Control::Owner,
        MenuFor::GroupBy => Control::GroupBy,
        MenuFor::Due => Control::Due,
        MenuFor::Priority => Control::Priority,
        MenuFor::Row { .. } => Control::List,
    }
}

/// What the empty list says: what to change to see tasks again. `old` is
/// whether "Hide done after 14 days" hides tasks the other filters keep.
pub(super) fn empty_hint(filters: &TaskFilters, label: &str, old: bool) -> String {
    let old = old && filters.hide_old;
    if !task_list::narrowed(filters) && !old {
        return format!(
            "No tasks match in {label}. Widen the scope from the menu at the top left."
        );
    }
    let mut fixes = Vec::new();
    let unhide = format!("turn off Hide done after {} days", task_list::OLD_DAYS);
    if old {
        fixes.push(unhide.as_str());
    }
    if !filters.statuses.is_empty() {
        fixes.push("press Show all");
    }
    if filters.owner != OwnerFilter::Anyone {
        fixes.push("set Owner to Anyone");
    }
    if filters.due.is_some() {
        fixes.push("set Due to Any");
    }
    if filters.priority.is_some() {
        fixes.push("set Priority to Any");
    }
    let mut text = String::from("No tasks match. ");
    let first = fixes.remove(0);
    text.push_str(&first[..1].to_uppercase());
    text.push_str(&first[1..]);
    for fix in fixes {
        text.push_str(", ");
        text.push_str(fix);
    }
    text.push_str(", or widen the scope.");
    text
}

/// The ring drawn round a control with the keyboard.
fn focus_ring(definition: wren_core::theme::Theme, style: &mut button::Style) {
    style.border = iced::Border {
        color: theme::color(definition.colors.accent),
        width: 2.0,
        radius: style.border.radius,
    };
}

fn control_style(
    definition: wren_core::theme::Theme,
    status: button::Status,
    quiet: bool,
    focused: bool,
) -> button::Style {
    let mut style = chrome::control(definition, status, false, quiet);
    if focused {
        focus_ring(definition, &mut style);
    }
    style
}

fn picker_style(
    definition: wren_core::theme::Theme,
    status: button::Status,
    open: bool,
    focused: bool,
) -> button::Style {
    let colors = definition.colors;
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let mut style = button::Style {
        background: Some(
            theme::color(if hovered || open {
                colors.control
            } else {
                colors.raised
            })
            .into(),
        ),
        text_color: theme::color(colors.text),
        border: iced::Border {
            color: theme::color(if open {
                colors.accent
            } else {
                colors.control_border
            }),
            width: 1.0,
            radius: 6.0.into(),
        },
        ..button::Style::default()
    };
    if focused {
        focus_ring(definition, &mut style);
    }
    style
}

fn chip_style(
    definition: wren_core::theme::Theme,
    status: button::Status,
    on: bool,
    focused: bool,
) -> button::Style {
    let colors = definition.colors;
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let mut style = button::Style {
        background: Some(
            theme::color(if on {
                colors.selected
            } else if hovered {
                colors.control
            } else {
                colors.raised
            })
            .into(),
        ),
        text_color: theme::color(if on { colors.accent } else { colors.muted }),
        border: iced::Border {
            color: theme::color(if on {
                colors.accent
            } else {
                colors.control_border
            }),
            width: 1.0,
            radius: 99.0.into(),
        },
        ..button::Style::default()
    };
    if focused {
        focus_ring(definition, &mut style);
    }
    style
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use iced::keyboard::key::{NativeCode, Physical};
    use iced::keyboard::Modifiers;
    use wren_core::ops::Session;

    use super::super::commands::Command;
    use super::super::Options;
    use super::*;
    use crate::keys::{Input, Nav};

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/wrentest/fixtures/tasks/list"
    );
    const SETTINGS: &str = "tasks = on\nboards = on\ntasks-me = sam, sam.lee\n";

    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// An app on the workspace at `root`, as a launch opens it, today
    /// pinned to Thursday 19 November 2026.
    fn launch(directory: &Path, root: &Path) -> App {
        let config = directory.join("config.conf");
        std::fs::write(&config, SETTINGS).unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(config));
        app.pin_today(Date::parse_iso("2026-11-19").unwrap());
        app.workspace_mode = true;
        app.session = Some(Session::open_with(root, app.session_options()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        app
    }

    /// The M11 fixture, with the view of the workspace open.
    fn fixture() -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let root = directory.path().join("workspace");
        copy(Path::new(FIXTURE), &root);
        let mut app = launch(directory.path(), &root);
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        assert!(app.task_view_has_keys());
        (directory, app)
    }

    fn root(directory: &tempfile::TempDir) -> std::path::PathBuf {
        directory.path().join("workspace")
    }

    fn press(key: Key, text: Option<&str>) -> Press {
        Press {
            key,
            physical: Physical::Unidentified(NativeCode::Unidentified),
            modifiers: Modifiers::empty(),
            text: text.map(str::to_owned),
            repeat: false,
        }
    }

    fn named(key: Named) -> Press {
        press(Key::Named(key), None)
    }

    fn letter(c: &str) -> Press {
        press(Key::Character(c.into()), Some(c))
    }

    /// One batch of presses, as iced hands it over: the view's key
    /// wrapper reports each, then the app hears its own copies of those
    /// the wrapper lets through, Escape and the shortcuts, in order.
    fn batch(app: &mut App, presses: &[Press]) {
        let serial = app.task_list.next_serial();
        in_batch(app, serial, presses);
    }

    /// [`batch`] in the batch `serial`, which may have had other input.
    fn in_batch(app: &mut App, serial: u64, presses: &[Press]) {
        for press in presses {
            let _ = app.update(Message::TaskView(Event::Key(ListKey {
                press: press.clone(),
                serial,
                after_press: false,
            })));
        }
        copies(app, presses);
    }

    /// The app's copies of `presses`.
    fn copies(app: &mut App, presses: &[Press]) {
        for press in presses {
            if press.is_plain_escape() {
                let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
            } else if let Some(accel) =
                crate::keys::shortcut(&press.key, press.physical, press.modifiers)
            {
                let _ = app.update(Message::Key(Input::Accel(accel)));
            }
        }
    }

    /// A click in a batch of its own.
    fn click(app: &mut App, click: Click) {
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Click { serial, click }));
    }

    fn control(app: &mut App, control: Control) {
        click(app, Click::Control(control));
    }

    /// Input from the menu over the view; a row chosen is pressed and then
    /// released, as a pointer chooses it.
    fn menu_event(app: &mut App, event: host_menu::Event) {
        let serial = app.task_list.next_serial();
        if let host_menu::Event::Choose { instance, row } = event {
            let window = app.viewport;
            let at = (app.task_list.menu.as_ref())
                .and_then(|menu| menu.row_bounds(row, window))
                .map_or(iced::Point::ORIGIN, |bounds| bounds.center());
            let _ = app.update(Message::TaskView(Event::Click {
                serial,
                click: Click::Menu(host_menu::Event::Pressed {
                    instance,
                    at,
                    primary: true,
                }),
            }));
        }
        let _ = app.update(Message::TaskView(Event::Click {
            serial,
            click: Click::Menu(event),
        }));
    }

    /// The chord of `action`, as the keymap binds it, with Ctrl or Alt.
    fn chord(app: &App, action: &str, key: &str) -> Press {
        [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO]
            .into_iter()
            .map(|modifiers| Press {
                modifiers,
                text: None,
                ..letter(key)
            })
            .find(|press| {
                crate::keys::shortcut(&press.key, press.physical, press.modifiers)
                    .is_some_and(|accel| app.preferences.keymap.action(&accel) == Some(action))
            })
            .unwrap_or_else(|| panic!("{action} is bound to no chord of {key}"))
    }

    fn document(app: &App) -> Option<String> {
        let document = app.session.as_ref()?.document()?;
        Some(document.path().as_path().to_string_lossy().into_owned())
    }

    fn descriptions(app: &App) -> Vec<String> {
        let listing = &app.task_list.listing;
        listing
            .rows()
            .map(|row| listing.record(row).unwrap().description.clone())
            .collect()
    }

    fn selected(app: &App) -> String {
        let entry = app.task_list.selected.as_ref().unwrap().entry;
        app.task_list
            .listing
            .record(entry)
            .unwrap()
            .description
            .clone()
    }

    fn tasks_toml(directory: &tempfile::TempDir) -> Option<String> {
        std::fs::read_to_string(root(directory).join(".wren/tasks.toml")).ok()
    }

    #[test]
    fn the_list_shows_the_scopes_tasks_in_order_with_the_old_work_hidden() {
        let (directory, app) = fixture();
        let listing = &app.task_list.listing;
        let labels: Vec<&str> = listing.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["Standup", "Roadmap"]);
        assert_eq!(listing.shown, 12, "{:?}", descriptions(&app));
        assert!(!descriptions(&app).contains(&"Archive old docs".to_owned()));
        assert_eq!(selected(&app), "Order badges", "the first row");
        // The hidden note is left out, and said so.
        let (counts, left) = app.task_list_counts(&Scope::Workspace);
        assert_eq!((counts.open_and_doing, counts.overdue), (8, 1));
        assert_eq!(
            left.as_deref(),
            Some("Left out: 1 note (2 tasks, tasks: off)")
        );
        // Opening the view writes nothing.
        assert_eq!(tasks_toml(&directory), None);
    }

    #[test]
    fn the_keys_move_the_selection_and_enter_opens_the_note_at_the_line() {
        let (_directory, mut app) = fixture();
        batch(&mut app, &[letter("j")]);
        assert_eq!(selected(&app), "Ping finance @SAM");
        batch(
            &mut app,
            &[named(Named::ArrowDown), named(Named::ArrowDown)],
        );
        assert_eq!(selected(&app), "Close the sprint");
        batch(&mut app, &[letter("k"), named(Named::ArrowUp)]);
        assert_eq!(selected(&app), "Ping finance @SAM");
        batch(&mut app, &[named(Named::End)]);
        assert_eq!(selected(&app), "Tidy the wiki");
        batch(&mut app, &[named(Named::Home)]);
        assert_eq!(selected(&app), "Order badges");
        batch(&mut app, &[named(Named::PageDown)]);
        assert_ne!(selected(&app), "Order badges");
        batch(&mut app, &[named(Named::PageUp)]);
        assert_eq!(selected(&app), "Order badges");
        // Down past the end stays on the last row.
        for _ in 0..20 {
            batch(&mut app, &[letter("j")]);
        }
        assert_eq!(selected(&app), "Tidy the wiki");

        batch(&mut app, &[named(Named::Home), letter("j"), letter("j")]);
        let entry = app.task_list.selected.as_ref().unwrap().entry;
        let record = app.task_list.listing.record(entry).unwrap().clone();
        assert_eq!(record.description, "Follow up with legal @priya");
        // Enter opens the note at the line; the keys after it in its batch
        // were typed before the note showed, and are dropped.
        batch(&mut app, &[named(Named::Enter), letter("j"), letter("x")]);
        assert!(app.task_view().is_none());
        assert_eq!(app.selected_note, Some(record.note));
        assert_eq!(app.column, Column::Editor);
        assert_eq!(app.editor.cursor().position.line, record.line);
        assert_eq!(
            app.editor.line_text(record.line).as_deref(),
            Some(record.source.as_str())
        );
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
    }

    #[test]
    fn a_chip_narrows_the_list_and_is_remembered_after_a_restart() {
        let (directory, mut app) = fixture();
        control(&mut app, Control::Chip(1));
        assert_eq!(descriptions(&app), ["Review the budget @sam"]);
        assert_eq!(
            app.task_list.focus, None,
            "a click leaves the keyboard with the list"
        );
        assert_eq!(
            tasks_toml(&directory).unwrap(),
            "schema_version = 1\n\n[views.\".\"]\nstatuses = [\"doing\"]\n"
        );
        control(&mut app, Control::Chip(3));
        assert_eq!(app.task_list.listing.shown, 2);
        // Escape neither clears the filters nor leaves the view.
        batch(&mut app, &[named(Named::Escape)]);
        assert!(app.task_view().is_some());
        assert_eq!(app.task_list.listing.shown, 2);

        // After a restart the view of the workspace has them again.
        drop(app);
        let mut app = launch(directory.path(), &root(&directory));
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        assert_eq!(app.task_list.listing.shown, 2);
        // A group's view has its own, the defaults.
        let group = Scope::Group(RelPath::new("Work/Planning").unwrap());
        let _ = app.dispatch(Command::ShowTasks(group));
        let _ = app.update(Message::Tick);
        assert_eq!(app.task_list.listing.shown, 7);

        // Show all clears the chips.
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        control(&mut app, Control::ShowAll);
        assert_eq!(app.task_list.listing.shown, 12);
        assert_eq!(tasks_toml(&directory).unwrap(), "schema_version = 1\n");
    }

    #[test]
    fn tab_walks_the_controls_and_escape_gives_the_keyboard_back() {
        let (_directory, mut app) = fixture();
        batch(&mut app, &[named(Named::Tab)]);
        assert_eq!(app.task_list.focus, Some(Control::Scope));
        batch(&mut app, &[named(Named::Tab), named(Named::Tab)]);
        assert_eq!(app.task_list.focus, Some(Control::Chip(1)));
        // Space presses a chip, and the keyboard stays on it.
        batch(&mut app, &[named(Named::Space)]);
        assert_eq!(descriptions(&app), ["Review the budget @sam"]);
        assert_eq!(app.task_list.focus, Some(Control::Chip(1)));
        // Show all now stands after the chips.
        batch(
            &mut app,
            &[
                named(Named::ArrowRight),
                named(Named::ArrowRight),
                named(Named::ArrowRight),
            ],
        );
        assert_eq!(app.task_list.focus, Some(Control::ShowAll));
        batch(&mut app, &[named(Named::Enter)]);
        assert_eq!(app.task_list.listing.shown, 12);
        assert_eq!(app.task_list.focus, Some(Control::Chip(0)));
        // The selection stayed with its task through the chips. j is no
        // list key while a control has the keyboard.
        assert_eq!(selected(&app), "Review the budget @sam");
        batch(&mut app, &[letter("j")]);
        assert_eq!(selected(&app), "Review the budget @sam");
        // Escape gives it back to the list, and the list's keys work.
        batch(&mut app, &[named(Named::Escape)]);
        assert_eq!(app.task_list.focus, None);
        batch(&mut app, &[letter("j")]);
        assert_eq!(selected(&app), "Fix the date");
        // Shift+Tab from the list goes to the last control, round the end.
        let mut back = named(Named::Tab);
        back.modifiers = Modifiers::SHIFT;
        batch(&mut app, &[back.clone()]);
        assert_eq!(app.task_list.focus, Some(Control::HideOld));
        batch(&mut app, &[named(Named::Tab)]);
        assert_eq!(app.task_list.focus, None);
        batch(&mut app, &[named(Named::Tab)]);
        assert_eq!(app.task_list.focus, Some(Control::Scope));
        batch(&mut app, &[back]);
        assert_eq!(app.task_list.focus, None);
    }

    #[test]
    fn the_scope_menu_opens_by_keys_and_a_choice_shows_that_scope() {
        let (_directory, mut app) = fixture();
        // The tree's selection names the group and note below the scope.
        let roadmap = (app.model().unwrap().notes.iter())
            .find(|note| note.title == "Roadmap")
            .unwrap()
            .id;
        let _ = app.process_core(Op::OpenNote {
            note: Some(roadmap),
        });
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        // Enter on the chip opens the menu; the keys after it in its batch
        // are dropped, Escape too, so the menu stays.
        batch(
            &mut app,
            &[
                named(Named::Tab),
                named(Named::Enter),
                letter("g"),
                named(Named::Escape),
            ],
        );
        let menu = app.task_list.menu.as_ref().expect("the scope menu");
        assert_eq!(menu.title(), Some("Show tasks in"));
        assert!(matches!(menu.purpose, MenuFor::Scope(_)));
        let checked: Vec<bool> = menu.rows().iter().map(|row| row.checked).collect();
        assert_eq!(checked, [true, false, false, false]);
        assert!(app.task_list.copies.is_empty(), "the copy found its report");
        assert!(matches!(app.task_view(), Some(Scope::Workspace)));
        // In a batch of its own, Escape closes it without a choice.
        batch(&mut app, &[named(Named::Escape)]);
        assert!(app.task_list.menu.is_none());
        assert!(matches!(app.task_view(), Some(Scope::Workspace)));
        assert_eq!(app.task_list.focus, Some(Control::Scope));
        // Its accelerator chooses a row: the section.
        batch(&mut app, &[named(Named::Enter)]);
        batch(&mut app, &[letter("s")]);
        assert_eq!(
            app.task_view(),
            Some(&Scope::Section(RelPath::new("Work").unwrap()))
        );
        assert!(app.task_list.menu.is_none());
        // A new scope starts at the list.
        assert_eq!(app.task_list.focus, None);
        // Down and Enter choose the group.
        batch(&mut app, &[named(Named::Tab), named(Named::ArrowDown)]);
        let menu = app.task_list.menu.as_ref().unwrap();
        assert_eq!(menu.selected(), Some(1));
        let labels: Vec<&str> = menu.rows().iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "workspace (workspace)",
                "Work (section)",
                "Planning (group)",
                "Roadmap (note)"
            ]
        );
        batch(&mut app, &[named(Named::ArrowDown)]);
        batch(&mut app, &[named(Named::Enter)]);
        assert_eq!(
            app.task_view(),
            Some(&Scope::Group(RelPath::new("Work/Planning").unwrap()))
        );
    }

    #[test]
    fn a_stale_menu_choice_and_a_press_outside_do_nothing() {
        let (directory, mut app) = fixture();
        control(&mut app, Control::GroupBy);
        let first = app.task_list.menu.as_ref().unwrap().instance();
        menu_event(&mut app, host_menu::Event::Outside { instance: first });
        assert!(app.task_list.menu.is_none());
        control(&mut app, Control::GroupBy);
        let second = app.task_list.menu.as_ref().unwrap().instance();
        assert_ne!(first, second);
        // A click on a row of the first menu chooses nothing, and a press
        // outside the first closes nothing.
        menu_event(
            &mut app,
            host_menu::Event::Choose {
                instance: first,
                row: 1,
            },
        );
        menu_event(&mut app, host_menu::Event::Outside { instance: first });
        assert!(app.task_list.menu.is_some());
        assert_eq!(tasks_toml(&directory), None);
        menu_event(
            &mut app,
            host_menu::Event::Choose {
                instance: second,
                row: 1,
            },
        );
        assert!(app.task_list.menu.is_none());
        let labels: Vec<&str> = (app.task_list.listing.groups.iter())
            .map(|group| group.label.as_str())
            .collect();
        assert_eq!(labels[0], "Overdue");
        assert!(tasks_toml(&directory).unwrap().contains("group = \"due\""));
        // The palette takes the keyboard, and the menu goes.
        control(&mut app, Control::Owner);
        assert!(app.task_list.menu.is_some());
        let _ = app.open_palette(super::super::palette::Mode::Commands);
        let _ = app.update(Message::Tick);
        assert!(app.task_list.menu.is_none());
    }

    #[test]
    fn owner_me_reads_tasks_me_and_a_person_narrows() {
        let (_directory, mut app) = fixture();
        control(&mut app, Control::Owner);
        let menu = app.task_list.menu.as_ref().unwrap();
        let labels: Vec<&str> = menu.rows().iter().map(|row| row.label.as_str()).collect();
        // Each person once, as first written in the tree's order.
        assert_eq!(labels, ["Anyone", "Me", "@priya", "@SAM", "@Sam.Lee"]);
        batch(&mut app, &[letter("m")]);
        assert_eq!(app.task_list.listing.shown, 11);
        assert!(!descriptions(&app).contains(&"Follow up with legal @priya".to_owned()));
        control(&mut app, Control::Owner);
        batch(&mut app, &[named(Named::ArrowDown), named(Named::Enter)]);
        assert_eq!(
            descriptions(&app),
            [
                "Follow up with legal @priya",
                "Book the offsite @priya @Sam.Lee"
            ]
        );
    }

    #[test]
    fn the_selection_follows_its_task_through_a_refresh() {
        let (_directory, mut app) = fixture();
        batch(&mut app, &[named(Named::End)]);
        batch(&mut app, &[letter("k"), letter("k")]);
        assert_eq!(selected(&app), "Write the retro");
        control(&mut app, Control::GroupBy);
        batch(&mut app, &[letter("p")]);
        assert_eq!(selected(&app), "Write the retro", "now under Low");
        control(&mut app, Control::Due);
        batch(&mut app, &[letter("o")]);
        // Gone from the list: the row at its place, else the last.
        assert_eq!(descriptions(&app), ["Draft the Q1 plan"]);
        assert_eq!(selected(&app), "Draft the Q1 plan");
    }

    #[test]
    fn enter_then_back_in_one_batch_opens_the_note_and_back_does_not_run() {
        let (_directory, mut app) = fixture();
        let back = chord(&app, "navigation.back", "[");
        batch(&mut app, &[named(Named::Home)]);
        assert_eq!(selected(&app), "Order badges");
        // Enter opens the note; Back and Escape after it in the batch were
        // pressed at the view, and are dropped, their app copies too.
        batch(
            &mut app,
            &[named(Named::Enter), back.clone(), named(Named::Escape)],
        );
        assert!(app.task_view().is_none());
        assert_eq!(document(&app).as_deref(), Some("Work/Meetings/Standup.md"));
        assert!(
            app.task_list.copies.is_empty(),
            "each copy found its report"
        );
        // In a batch of its own, Back runs: to the view.
        let accel = crate::keys::shortcut(&back.key, back.physical, back.modifiers).unwrap();
        let _ = app.update(Message::Key(Input::Accel(accel)));
        assert!(app.task_view().is_some());
    }

    #[test]
    fn escape_then_a_key_on_an_open_menu_closes_it_and_drops_the_key() {
        let (_directory, mut app) = fixture();
        batch(&mut app, &[named(Named::Tab)]);
        batch(&mut app, &[named(Named::Enter)]);
        assert!(app.task_list.menu.is_some());
        // Escape, read in its place, closes the menu; `s` after it is
        // dropped, never chosen from the menu that was open.
        batch(&mut app, &[named(Named::Escape), letter("s")]);
        assert!(app.task_list.menu.is_none());
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        assert_eq!(app.task_list.focus, Some(Control::Scope));
        assert!(app.task_list.copies.is_empty());
    }

    #[test]
    fn each_dropped_copy_is_matched_to_its_own_key() {
        let (_directory, mut app) = fixture();
        let back = chord(&app, "navigation.back", "[");
        let f5 = press(Key::Named(Named::F5), None);
        batch(&mut app, &[named(Named::Tab)]);
        // An unbound function key, Enter on the chip, then Back: the
        // function key's copy is the app's (and does nothing), Back's is
        // dropped with the batch Enter stopped.
        batch(&mut app, &[f5, named(Named::Enter), back]);
        assert!(app.task_list.menu.is_some());
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        assert!(app.task_list.copies.is_empty());
        // A key after a press in its batch is dropped too.
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Key(ListKey {
            press: letter("w"),
            serial,
            after_press: true,
        })));
        assert!(app.task_list.menu.is_some());
    }

    #[test]
    fn a_press_outside_the_menu_then_enter_in_one_batch_drops_enter() {
        let (_directory, mut app) = fixture();
        control(&mut app, Control::GroupBy);
        let instance = app.task_list.menu.as_ref().unwrap().instance();
        // The press is taken by the layer under the menu, so the view's
        // keys never see it; it carries the view's serial, and stops the
        // batch, so Enter does not open the selected note.
        let serial = app.task_list.next_serial();
        let _ = app.update(Message::TaskView(Event::Click {
            serial,
            click: Click::Menu(host_menu::Event::Outside { instance }),
        }));
        in_batch(&mut app, serial, &[named(Named::Enter)]);
        assert!(app.task_list.menu.is_none());
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        // Enter in the next batch opens it.
        batch(&mut app, &[named(Named::Enter)]);
        assert!(app.task_view().is_none());
    }

    #[test]
    fn a_click_on_a_row_of_a_listing_since_rebuilt_does_nothing() {
        let (_directory, mut app) = fixture();
        let generation = app.task_list.generation;
        let order_badges = app.task_list.listing.rows().next().unwrap();
        // Doing's chip has the keys: Enter narrows the list to Review the
        // budget, which now stands where Order badges was drawn.
        batch(
            &mut app,
            &[named(Named::Tab), named(Named::Tab), named(Named::Tab)],
        );
        assert_eq!(app.task_list.focus, Some(Control::Chip(1)));
        let serial = app.task_list.next_serial();
        in_batch(&mut app, serial, &[named(Named::Enter)]);
        assert_eq!(descriptions(&app), ["Review the budget @sam"]);
        assert_ne!(app.task_list.generation, generation);
        // The press on Order badges' source, queued in the same batch.
        let _ = app.update(Message::TaskView(Event::Click {
            serial,
            click: Click::Open {
                generation,
                entry: order_badges,
            },
        }));
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        // In a batch of its own it still names the old listing.
        click(
            &mut app,
            Click::Open {
                generation,
                entry: order_badges,
            },
        );
        assert_eq!(app.task_view(), Some(&Scope::Workspace));
        assert_eq!(document(&app), None);
        // The row as drawn now opens.
        let entry = app.task_list.listing.rows().next().unwrap();
        let generation = app.task_list.generation;
        click(&mut app, Click::Open { generation, entry });
        assert_eq!(document(&app).as_deref(), Some("Work/Planning/Roadmap.md"));
    }

    #[test]
    fn another_workspace_never_shows_the_last_ones_list() {
        let (directory, mut app) = fixture();
        assert!(descriptions(&app).contains(&"Draft the Q1 plan".to_owned()));
        // The same notes, one task renamed: its index counts the same
        // revisions as the first's.
        let other = directory.path().join("other");
        copy(Path::new(FIXTURE), &other);
        let roadmap = other.join("Work/Planning/Roadmap.md");
        let text = std::fs::read_to_string(&roadmap).unwrap();
        std::fs::write(&roadmap, text.replace("Q1 plan", "Q2 plan")).unwrap();
        app.session = Some(Session::open_with(&other, app.session_options()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        assert!(app.task_list.selected.is_none(), "the old list is gone");
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        let shown = descriptions(&app);
        assert!(shown.contains(&"Draft the Q2 plan".to_owned()), "{shown:?}");
        assert!(!shown.contains(&"Draft the Q1 plan".to_owned()));
        // Even with the list kept, the key tells the workspaces apart.
        let session = Session::open_with(&root(&directory), app.session_options()).unwrap();
        app.session = Some(session);
        let _ = app.update(Message::Tick);
        assert!(descriptions(&app).contains(&"Draft the Q1 plan".to_owned()));
    }

    #[test]
    fn the_scope_menu_marks_only_the_current_scope() {
        let (_directory, mut app) = fixture();
        control(&mut app, Control::Scope);
        let menu = app.task_list.menu.as_ref().unwrap();
        let glyphs: Vec<&str> = (menu.rows().iter())
            .map(|row| row.glyph.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(glyphs, ["\u{2713}", "", "", ""]);
    }

    /// A narrow window: the footer's hint gives way in whole pieces from
    /// its end, never a word cut, before the counts do.
    #[test]
    fn the_footer_hint_gives_way_in_whole_pieces() {
        crate::fonts::load();
        let (_directory, app) = fixture();
        let pieces: Vec<&str> = HINT.split(HINT_SEPARATOR).collect();
        assert_eq!(pieces.len(), 6);
        assert_eq!(app.task_list_hint(10_000.0), HINT);
        assert_eq!(app.task_list_hint(0.0), "");
        assert_eq!(app.task_list_hint(-40.0), "");
        let width = |text: &str| super::super::task_footer::label_width(text, 11.0, fonts::UI);
        let mut room = 0.0;
        for count in 1..=pieces.len() {
            let gap = if count > 1 {
                width(HINT_SEPARATOR)
            } else {
                0.0
            };
            room += gap + width(pieces[count - 1]);
            let whole = pieces[..count].join(HINT_SEPARATOR);
            assert_eq!(app.task_list_hint(room), whole, "{room}");
            assert_eq!(
                app.task_list_hint(room - 0.5),
                pieces[..count - 1].join(HINT_SEPARATOR),
                "{room}"
            );
        }
        // Every room shows whole pieces from the first.
        for room in (0..600).step_by(7) {
            let shown = app.task_list_hint(room as f32);
            assert!(
                shown.is_empty()
                    || (1..=pieces.len())
                        .any(|count| shown == pieces[..count].join(HINT_SEPARATOR)),
                "{shown:?}"
            );
        }
    }

    #[test]
    fn the_empty_list_says_how_to_widen_it() {
        let filters = TaskFilters {
            statuses: vec![StatusType::Done],
            owner: OwnerFilter::Me,
            priority: Some(PriorityFilter::Unset),
            ..TaskFilters::default()
        };
        assert_eq!(
            empty_hint(&filters, "Work", false),
            "No tasks match. Press Show all, set Owner to Anyone, set Priority to Any, or \
             widen the scope."
        );
        assert_eq!(
            empty_hint(&TaskFilters::default(), "Work", false),
            "No tasks match in Work. Widen the scope from the menu at the top left."
        );
        // Ring judge: Hide done alone hides every task.
        assert_eq!(
            empty_hint(&TaskFilters::default(), "Work", true),
            "No tasks match. Turn off Hide done after 14 days, or widen the scope."
        );
        assert_eq!(
            empty_hint(&filters, "Work", true),
            "No tasks match. Turn off Hide done after 14 days, press Show all, set Owner to \
             Anyone, set Priority to Any, or widen the scope."
        );
    }

    #[test]
    fn a_due_cell_marks_overdue_finished_and_invalid_dates() {
        let (_directory, app) = fixture();
        let today = app.today();
        let cell = |description: &str| {
            let record = (app.task_list.listing.records.iter())
                .find(|record| record.description == description)
                .unwrap();
            due_cell(record, today)
        };
        let look = |description: &str| cell(description).map(|cell| (cell.text, cell.look));
        assert_eq!(
            look("Draft the Q1 plan"),
            Some(("Tue 17 Nov".into(), "overdue"))
        );
        assert_eq!(
            look("Review the budget @sam"),
            Some(("Thu 19 Nov".into(), "normal"))
        );
        assert_eq!(
            look("Fix the date"),
            Some(("2026-02-30 \u{26a0}".into(), "invalid"))
        );
        assert_eq!(
            look("Send the minutes"),
            Some(("\u{2713} Thu 5 Nov".into(), "finished"))
        );
        assert_eq!(
            look("Cancel the vendor call"),
            Some(("\u{2715} Thu 12 Nov".into(), "finished"))
        );
        assert_eq!(look("Tidy the wiki"), None);
        assert_eq!(look("Order badges"), None);
    }

    /// Builds the list and its view for 5,000 tasks in `notes` notes of
    /// one group, and returns how long the list took to build once, the
    /// view a frame, and the list's rebuild after a write while 100 rows
    /// written before it are held.
    fn time_list(
        notes: usize,
    ) -> (
        std::time::Duration,
        std::time::Duration,
        std::time::Duration,
    ) {
        let directory = crate::test_support::tempdir();
        let root = directory.path().join("workspace");
        std::fs::create_dir_all(root.join("Work/Big")).unwrap();
        let per_note = 5000 / notes;
        for note in 0..notes {
            let mut text = String::new();
            for line in 0..per_note {
                let day = 1 + (note + line) % 28;
                text.push_str(&format!(
                    "- [ ] Task {note}.{line} @sam 📅 2026-11-{day:02}\n"
                ));
            }
            std::fs::write(root.join(format!("Work/Big/Note {note}.md")), text).unwrap();
        }
        let mut app = launch(directory.path(), &root);
        let start = std::time::Instant::now();
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        let built = start.elapsed();
        assert_eq!(app.task_list.listing.shown, 5000);
        assert_eq!(app.task_list.listing.groups.len(), notes);
        let start = std::time::Instant::now();
        let frames = 50;
        for _ in 0..frames {
            let view = app.view();
            std::hint::black_box(&view);
        }
        let frame = start.elapsed() / frames;
        // 100 rows written without R, each held; then one more write, whose
        // revision relocates every hold and builds the list round them.
        let write = |app: &mut App, row: usize| {
            let entry = app.task_list.listing.rows().nth(row).unwrap();
            let record = app.task_list.listing.record(entry).unwrap();
            let target = super::super::task_target::RowTarget::of(record);
            let _ = app.write_view_row(
                &target,
                Some(record.box_at),
                wren_markdown::task::Change::RaisePriority,
                super::super::task_rows::What::Raise,
            );
        };
        for row in 0..100 {
            write(&mut app, row * 7);
            let _ = app.update(Message::Tick);
        }
        assert_eq!(app.task_list.held.len(), 100);
        write(&mut app, 4000);
        let start = std::time::Instant::now();
        let _ = app.sync_task_list();
        (built, frame, start.elapsed())
    }

    /// The list's view builds only the rows in view: with 5,000 tasks in
    /// one note or in 5,000, a frame's build stays well under a frame.
    #[test]
    #[ignore = "a timing; run with --ignored on a quiet machine"]
    fn perf_task_list_view() {
        for notes in [1, 5000] {
            let (built, frame, held) = time_list(notes);
            eprintln!(
                "5,000 tasks in {notes} notes: list built in {built:?}, view built in \
                 {frame:?} a frame, rebuilt round 100 held rows in {held:?}"
            );
            assert!(frame < std::time::Duration::from_millis(16), "{frame:?}");
            assert!(
                held < built * 3 + std::time::Duration::from_millis(5),
                "{held:?}"
            );
        }
    }
}
