//! The task view's list layout: its body, the rows in view between spaces
//! for those out of it, its selection and the keys that move it, and a
//! row's cells.

use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::widget::{column, container, mouse_area, row, scrollable, svg, Space};
use iced::{Element, Fill, Shrink, Task};
use wren_core::tasks::TaskRecord;
use wren_core::tasks_file::{GroupBy, Layout, TaskFilters};
use wren_core::theme::Rgb;
use wren_markdown::date::{Date, Parsed};
use wren_markdown::status::StatusType;

use super::build::select;
use super::clicks::Click;
use super::controls::empty_hint;
use super::{Cell, Control, LIST, UNDRAWN_HEIGHT};
use crate::app::task_list::{self, Entry, Piece, HEADER_HEIGHT, ROW_HEIGHT};
use crate::app::tasks_view::{Event, Scope};
use crate::app::{App, Message};
use crate::ellipsis::{ellipsis, ellipsis_runs};
use crate::list_keys::{self, list_keys, probe, Press};
use crate::{chrome, fonts, theme};

/// What a row's box shows: its status, or the symbol in the box when it
/// has no status of its own (`[?]`, `[>]`), which counts as open but is
/// shown as written (discovery/tasks.md §5.1).
pub(in crate::app) fn box_face(record: &TaskRecord) -> Result<StatusType, char> {
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
pub(super) fn task_box<'a>(
    status: StatusType,
    colors: &wren_core::theme::Colors,
) -> Element<'a, Message> {
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
pub(in crate::app) struct DueCell {
    pub text: String,
    /// `normal`, `overdue`, `invalid` or `finished`.
    pub look: &'static str,
}

/// The due cell of `record` on `today`: the finished work's stamp, else
/// its due date, overdue or not, or a date that is no real day as written
/// and marked; `None` with no date.
pub(in crate::app) fn due_cell(record: &TaskRecord, today: Date) -> Option<DueCell> {
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
pub(in crate::app) fn source_text(record: &TaskRecord, title: &str, by_note: bool) -> String {
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
    /// The list's visible height.
    pub(super) fn list_height(&self) -> f32 {
        self.task_list
            .height
            .or_else(|| self.task_list.drawn(Control::List).map(|at| at.height))
            .unwrap_or(UNDRAWN_HEIGHT)
    }

    /// Select the entry `entry` and scroll it into view.
    pub(in crate::app) fn select_entry(&mut self, entry: usize) -> Task<Message> {
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

    /// A key for the list itself, which has the keyboard: a row's own keys
    /// act on the selected row, and the others move the selection or open
    /// its note. Returns whether it stops its batch.
    pub(super) fn list_press(&mut self, press: &Press) -> (Task<Message>, bool) {
        let shift = press.modifiers.shift();
        if let Some(act) = crate::app::task_rows::RowAct::of(press) {
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

    /// The view's body: its filters over its list, or over its board.
    pub(in crate::app) fn task_list_body(&self) -> Element<'_, Message> {
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
        let tools = self.task_list_controls(&filters, serial);
        let body: Element<'_, Message> = if filters.layout == Layout::Board {
            self.board_body()
        } else if list.listing.entries.is_empty() {
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
    pub(in crate::app) fn built_rows(&self) -> std::ops::Range<usize> {
        self.built_rows_at(self.task_list.scroll)
    }

    /// The rows built with the list scrolled to `scroll`.
    pub(in crate::app) fn built_rows_at(&self, scroll: f32) -> std::ops::Range<usize> {
        let height = self.list_height();
        (self.task_list.listing).visible((scroll - height).max(0.0), 3.0 * height)
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
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

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
            let target = crate::app::task_target::RowTarget::of(record);
            let _ = app.write_view_row(
                &target,
                Some(record.box_at),
                wren_markdown::task::Change::RaisePriority,
                crate::app::task_rows::What::Raise,
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
