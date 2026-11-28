//! The task view's controls: the scope chip, the List | Board switch, the
//! status chips, Show all, the owner, grouping, due and priority pickers
//! and Hide done, with the menus they open and what a choice there
//! changes. On the board the status chips, Show all and Group by do not
//! apply, and are not drawn.

use iced::widget::{button, container, row};
use iced::{Element, Fill, Shrink, Task};
use wren_core::fs_store::RelPath;
use wren_core::model::ItemRef;
use wren_core::ops::Op;
use wren_core::tasks_file::{DueBucket, GroupBy, Layout, OwnerFilter, PriorityFilter, TaskFilters};
use wren_editor::{Accelerator, Row};
use wren_markdown::status::StatusType;

use super::clicks::Click;
use super::keys::task_view_slots;
use super::list::task_box;
use super::menu::MenuFor;
use super::{Control, LIST};
use crate::app::commands::Gateway;
use crate::app::task_list;
use crate::app::tasks_view::{Event, Scope};
use crate::app::{App, Message};
use crate::ellipsis::ellipsis;
use crate::list_keys::probe;
use crate::{chrome, fonts, theme};

const GROUPINGS: [(GroupBy, &str, char); 4] = [
    (GroupBy::Note, "Note", 'n'),
    (GroupBy::Due, "Due date", 'd'),
    (GroupBy::Priority, "Priority", 'p'),
    (GroupBy::Owner, "Owner", 'o'),
];

const BUCKET_KEYS: [char; 7] = ['o', 'e', 't', 'w', 'x', 'l', 'n'];

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
pub(in crate::app) fn empty_hint(filters: &TaskFilters, label: &str, old: bool) -> String {
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

impl App {
    /// Press a control: a chip toggles, Show all clears the chips, the
    /// checkbox flips, and the others open their menu.
    pub(super) fn press_control(&mut self, control: Control) -> Task<Message> {
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
            Control::Layout => self
                .change_layout(Layout::other)
                .map_or_else(Task::none, |outcome| outcome.task),
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
    /// scope when they differ. Returns the write's outcome; `None` when
    /// nothing changed, or with no view.
    pub(in crate::app) fn change_filters(
        &mut self,
        change: impl FnOnce(&mut TaskFilters),
    ) -> Option<Gateway> {
        let scope = self.task_view().cloned()?;
        let item = self.scope_item(&scope).ok()?;
        let mut filters = self.task_filters(&scope);
        let before = filters.clone();
        change(&mut filters);
        if filters == before {
            return None;
        }
        // The rows held move to where the new filters put them.
        self.task_list.held.clear();
        self.task_list.describe = None;
        self.task_list.reveal = true;
        Some(self.process_core(Op::SetTaskFilters {
            scope: item,
            filters,
        }))
    }

    /// Show the view's layout `to` gives for the one it shows (item 12.1),
    /// remembered for its scope as a filter change is, the held rows let
    /// go. A switch closes the menu over the view, whose control or row the
    /// other layout may not draw. No stop in Back and Forward. `None` with
    /// no view, or when the layout stays.
    pub(in crate::app) fn change_layout(
        &mut self,
        to: impl FnOnce(Layout) -> Layout,
    ) -> Option<Gateway> {
        let scope = self.task_view().cloned()?;
        let before = self.task_filters(&scope).layout;
        let mut outcome = self.change_filters(|filters| filters.layout = to(filters.layout))?;
        let filters = self.task_filters(&scope);
        if filters.layout == before {
            // Refused: the file is read-only.
            return Some(outcome);
        }
        self.task_list.menu = None;
        // A control the layout now shown does not draw gives the keyboard
        // back to the body.
        if (self.task_list.focus)
            .is_some_and(|focus| !task_view_slots(&filters).contains(&Some(focus)))
        {
            self.task_list.focus = None;
        }
        // The list's scroller was gone while the board showed, and comes
        // back at its top: it goes back to where the list was.
        if filters.layout == Layout::List {
            let scroll = iced::widget::operation::scroll_to(
                LIST,
                iced::widget::operation::AbsoluteOffset {
                    x: 0.0,
                    y: self.task_list.scroll,
                },
            );
            outcome.task =
                Task::batch([std::mem::replace(&mut outcome.task, Task::none()), scroll]);
        }
        Some(outcome)
    }

    /// `tasks.toggle_board`: the view's other layout, as `B` and the
    /// switch's Enter show it.
    pub(in crate::app) fn toggle_task_board(&mut self) -> Gateway {
        let palette = self.palette.take().is_some();
        let Some(mut outcome) = self.change_layout(Layout::other) else {
            return self.blocked("No task view is open");
        };
        if palette {
            outcome.task = Task::batch([
                std::mem::replace(&mut outcome.task, Task::none()),
                self.column_focus(),
            ]);
        }
        outcome
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
                rows.extend(
                    crate::app::task_menus::PRIORITY_ROWS
                        .iter()
                        .enumerate()
                        .map(|(index, (level, key))| {
                            let (glyph, name) = level
                                .map_or(("\u{b7}".to_owned(), "None"), |level| {
                                    (level.glyph().to_string(), level.name())
                                });
                            let filter = level.map_or(PriorityFilter::Unset, PriorityFilter::Level);
                            Row::new(1 + index as u32, name)
                                .glyph(glyph)
                                .accelerator(*key)
                                .checked(filters.priority == Some(filter))
                        }),
                );
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

    /// Act on the row `row` of the scope's or a filter's menu, `purpose`.
    pub(super) fn choose_filter_menu(&mut self, purpose: MenuFor, row: u32) -> Task<Message> {
        let row = row as usize;
        match purpose {
            MenuFor::Scope(choices) => {
                if let Some(Some(scope)) = choices.get(row).cloned() {
                    if self.task_view() != Some(&scope) {
                        return self
                            .dispatch(crate::app::commands::Command::ShowTasks(scope))
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
                    n => (crate::app::task_menus::PRIORITY_ROWS.get(n - 1)).map(|(level, _)| {
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

    /// The scope chip in the view's header: it opens "Show tasks in".
    pub(in crate::app) fn task_scope_chip(&self, scope: &Scope) -> Element<'_, Message> {
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

    /// The List | Board switch in the view's header: two segments, the
    /// layout shown pressed. A click on a segment shows its layout; with
    /// the keyboard, Enter or Space flips it.
    pub(in crate::app) fn task_layout_switch(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let definition = self.definition;
        let list = &self.task_list;
        let shown = list.layout();
        let focused = list.focus == Some(Control::Layout) && self.task_view_has_keys();
        let serial = list.serials.get();
        // The box's 1 px edge and the line between the segments are its
        // background showing round and between them.
        let segment = |layout: Layout, label: &'static str, radius: iced::border::Radius| {
            let on = layout == shown;
            button(
                chrome::label(label, 12, if on { colors.accent } else { colors.body })
                    .font(if on { fonts::SEMIBOLD } else { fonts::UI }),
            )
            .padding([3, 9])
            .on_press(Message::TaskView(Event::Click {
                serial,
                click: Click::Layout(layout),
            }))
            .style(move |_, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
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
                    text_color: theme::color(if on { colors.accent } else { colors.body }),
                    border: iced::Border {
                        radius,
                        ..iced::Border::default()
                    },
                    ..button::Style::default()
                }
            })
        };
        let switch = container(
            row![
                segment(Layout::List, "List", iced::border::radius(0).left(5)),
                segment(Layout::Board, "Board", iced::border::radius(0).right(5)),
            ]
            .spacing(1),
        )
        .padding(1)
        .style(move |_| container::Style {
            background: Some(theme::color(colors.control_border).into()),
            border: iced::Border {
                radius: 6.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        });
        // The keyboard's ring goes round the box, in room kept for it.
        container(probe(switch, &list.drawn, Control::Layout))
            .padding(2)
            .id("task-view-layout")
            .style(move |_| container::Style {
                border: iced::Border {
                    color: theme::color(definition.colors.accent),
                    width: if focused { 2.0 } else { 0.0 },
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    /// The bar of the view's controls, in the batch `serial`: the status
    /// chips and Show all (on the list only), the pickers (Group by on the
    /// list only) and Hide done.
    pub(in crate::app) fn task_list_controls(
        &self,
        filters: &TaskFilters,
        serial: u64,
    ) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let list = &self.task_list;
        let has_keys = self.task_view_has_keys();
        let definition = self.definition;
        let focused = |control: Control| has_keys && list.focus == Some(control);
        let send = move |control: Control| {
            Message::TaskView(Event::Click {
                serial,
                click: Click::Control(control),
            })
        };
        let board = filters.layout == Layout::Board;
        if board {
            // Not drawn: nothing may hang under them or be clicked there.
            (list.drawn.borrow_mut()).retain(|(control, _)| {
                !matches!(
                    control,
                    Control::Chip(_) | Control::ShowAll | Control::GroupBy
                )
            });
        }
        let mut tools = row![].spacing(6).align_y(iced::Center);
        if !board {
            tools = tools.push(chrome::label("Status", 12, colors.muted));
        }
        for (chip, status) in task_list::CHIPS.iter().enumerate().filter(|_| !board) {
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
        if board {
            // Neither the chips' word nor Show all.
        } else if filters.statuses.is_empty() {
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
        ]
        .into_iter()
        .filter(|(control, ..)| !(board && *control == Control::GroupBy))
        {
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
        chrome::bottom_rule(
            container(tools.wrap().vertical_spacing(6.0))
                .padding([9, 14])
                .width(Fill),
            self.definition,
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

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
    fn the_scope_menu_marks_only_the_current_scope() {
        let (_directory, mut app) = fixture();
        control(&mut app, Control::Scope);
        let menu = app.task_list.menu.as_ref().unwrap();
        let glyphs: Vec<&str> = (menu.rows().iter())
            .map(|row| row.glyph.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(glyphs, ["\u{2713}", "", "", ""]);
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
}
