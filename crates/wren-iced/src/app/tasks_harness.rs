//! The task requests of the test harness (discovery/tasks-build.md §1):
//! `tasks.get`, the caret line's task model, how the editor draws its
//! tokens and how the status bar spells the task out (`footer`, `null` off
//! a task line), the effective `tasks` setting and today; and
//! `tasks.set_today`, which pins the clock. With tasks off every request
//! but `tasks.get` is refused, and `tasks.get` says only that they are off,
//! so a lane with the setting off can assert the gate.
//!
//! The calendar (item 4.1): `tasks.calendar`, what it shows and where;
//! `tasks.calendar_type`, the field's text set as a paste or an input
//! method's commit sets it; and `tasks.calendar_click`, a click on a day,
//! a quick pick, a month arrow, Clear, inside the card off its controls,
//! or outside it. While it is open `editor.key` presses a key in its field
//! ([`App::calendar_key`]).
//!
//! Suggestions (item 5.1): `tasks.owners`, the owners they offer after an
//! `@`, best first, and whether those have been read from the index as it
//! is now; the suggestions themselves are `editor.get`'s `popover`.
//!
//! The task form (item 6.1): `tasks.form`, what it shows; `tasks.form_type`,
//! a field's text set as typing or a paste sets it; and `tasks.form_click`,
//! a click on a status, a priority, an owner's suggestion or remove button,
//! the calendar button, Save, Cancel or outside, or on the calendar over
//! the form. While it is open `editor.key` presses a key in it
//! ([`App::form_key`]).

use std::sync::Arc;

use iced::{Color, Task};
use serde_json::{json, Value};
use wren_markdown::date::Date;
use wren_markdown::span::SpanKind;
use wren_markdown::task::DateField;

use super::calendar::Event as CalendarEvent;
use super::task_form::{Event as FormEvent, Slot, PRIORITIES};
use super::test_requests::string;
use super::{App, Message};
use crate::test_ipc::Request;

impl App {
    pub(super) fn tasks_request(
        &mut self,
        request: &Arc<Request>,
    ) -> Result<Task<Message>, String> {
        match request.op.as_str() {
            "tasks.get" => request.respond(Ok(self.tasks_snapshot())),
            "tasks.set_today" => {
                if !self.preferences.tasks {
                    return Err("Tasks are off".into());
                }
                let text = string(&request.params, "date")?;
                let date = Date::parse_iso(text)
                    .ok_or_else(|| format!("date must be YYYY-MM-DD, not {text:?}"))?;
                self.pin_today(date);
                request.respond(Ok(self.tasks_snapshot()));
            }
            "tasks.calendar"
            | "tasks.calendar_type"
            | "tasks.calendar_click"
            | "tasks.owners"
            | "tasks.form"
            | "tasks.form_type"
            | "tasks.form_click"
                if !self.preferences.tasks =>
            {
                return Err("Tasks are off".into());
            }
            "tasks.form" => request.respond(Ok(self.form_snapshot())),
            "tasks.form_type" | "tasks.form_click" => {
                if self.task_form.is_none() {
                    return Err("The task form is not open".into());
                }
                let event = if request.op == "tasks.form_type" {
                    let text = string(&request.params, "text")?.to_owned();
                    match string(&request.params, "field")? {
                        "description" => FormEvent::Description(text),
                        "due" => FormEvent::Due(text),
                        "owners" => FormEvent::Owner(text),
                        other => return Err(format!("unknown form field {other:?}")),
                    }
                } else {
                    self.form_click(&request.params)?
                };
                let task = self.update(Message::TaskForm(event));
                request.respond(Ok(self.form_snapshot()));
                return Ok(task);
            }
            "tasks.calendar" => request.respond(Ok(self.calendar_snapshot())),
            "tasks.owners" => request.respond(Ok(self.owners_snapshot())),
            "tasks.calendar_type" | "tasks.calendar_click" => {
                if self.calendar.is_none() {
                    return Err("The calendar is not open".into());
                }
                let event = if request.op == "tasks.calendar_type" {
                    CalendarEvent::Field(crate::date_field::FieldEvent::Typed(
                        string(&request.params, "text")?.to_owned(),
                    ))
                } else {
                    self.calendar_click(&request.params)?
                };
                let task = self.update(Message::Calendar(event));
                request.respond(Ok(self.calendar_snapshot()));
                return Ok(task);
            }
            _ => return Err(format!("unknown test operation: {}", request.op)),
        }
        Ok(Task::none())
    }

    /// What `tasks.calendar_click` names: `{"day": "YYYY-MM-DD"}`, a day
    /// the grid shows; `{"chip": label}`; or `{"target": "previous" |
    /// "next" | "clear" | "inside" | "outside"}`.
    fn calendar_click(&self, params: &Value) -> Result<CalendarEvent, String> {
        let open = self.calendar.as_ref().ok_or("The calendar is not open")?;
        if let Some(day) = params.get("day").and_then(Value::as_str) {
            let date = Date::parse_iso(day).ok_or_else(|| format!("{day:?} is no date"))?;
            let shown = open
                .model
                .grid()
                .iter()
                .flatten()
                .flatten()
                .any(|cell| cell.date == date);
            return if shown {
                Ok(CalendarEvent::Pick(date))
            } else {
                Err(format!("{day} is not in the grid"))
            };
        }
        if let Some(label) = params.get("chip").and_then(Value::as_str) {
            return super::calendar::chip_date(label, open.model.today())
                .map(CalendarEvent::Pick)
                .ok_or_else(|| format!("no quick pick {label:?}"));
        }
        Ok(match string(params, "target")? {
            "previous" => CalendarEvent::Show(-1),
            "next" => CalendarEvent::Show(1),
            "clear" if open.has_due() => CalendarEvent::Clear,
            "clear" => return Err("Clear is not shown for a task with no due date".into()),
            "inside" => CalendarEvent::Refocus,
            "outside" => CalendarEvent::Outside,
            other => return Err(format!("unknown calendar target {other:?}")),
        })
    }

    /// A key pressed in the calendar's field, as the field and then the
    /// app's own key handling take it: the calendar's keys, a character
    /// typed (with no Ctrl, Alt or logo key) or Backspace, as iced's text
    /// input takes them at the end of the text, and Escape and shortcuts
    /// heard by the app as a real press is.
    pub(super) fn calendar_key(
        &mut self,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Task<Message> {
        let Some(open) = &self.calendar else {
            return Task::none();
        };
        let typed = open.model.typed().to_owned();
        self.calendar_field_key(&typed, key, modifiers, Message::Calendar)
    }

    /// [`App::calendar_key`] for a calendar whose field holds `typed`, its
    /// input sent as `send` makes it a message.
    fn calendar_field_key(
        &mut self,
        typed: &str,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
        send: fn(CalendarEvent) -> Message,
    ) -> Task<Message> {
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;
        let typed = typed.to_owned();
        let mut tasks = Vec::new();
        let plain = !(modifiers.control() || modifiers.alt() || modifiers.logo());
        let captured = match crate::date_field::route(&key, modifiers, false, typed.is_empty()) {
            Some(taken) => {
                if let Some(event) = taken {
                    tasks.push(self.update(send(CalendarEvent::Field(event))));
                }
                true
            }
            None => {
                let text = match key.as_ref() {
                    Key::Character(character) if plain => Some(format!("{typed}{character}")),
                    Key::Named(Named::Space) if plain => Some(format!("{typed} ")),
                    Key::Named(Named::Backspace) => {
                        let mut text = typed.clone();
                        text.pop();
                        Some(text)
                    }
                    _ => None,
                };
                match text {
                    Some(text) => {
                        if text != typed {
                            tasks.push(self.update(send(CalendarEvent::Field(
                                crate::date_field::FieldEvent::Typed(text),
                            ))));
                        }
                        true
                    }
                    None => false,
                }
            }
        };
        let status = if captured {
            iced::event::Status::Captured
        } else {
            iced::event::Status::Ignored
        };
        let event = iced::keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        };
        if let Some(input) =
            crate::keys::input(&event).filter(|input| crate::keys::reaches_app(input, status))
        {
            tasks.push(self.update(Message::Key(crate::keys::with_status(input, status))));
        }
        Task::batch(tasks)
    }

    /// What `tasks.form_click` names: `{"target": "save" | "cancel" |
    /// "outside" | "calendar"}`, `{"status": name}`, `{"priority": name}`
    /// (`"None"` for none), `{"suggestion": name}`, `{"remove": name}`, an
    /// owner's remove button; and on the calendar over the form, `{"day":
    /// "YYYY-MM-DD"}`, `{"chip": label}` or `{"calendar": "previous" |
    /// "next" | "clear" | "inside" | "outside"}`.
    fn form_click(&self, params: &Value) -> Result<FormEvent, String> {
        let open = self.task_form.as_ref().ok_or("The task form is not open")?;
        let text = |key: &str| params.get(key).and_then(Value::as_str);
        if let Some(model) = &open.calendar {
            let calendar = if let Some(day) = text("day") {
                let date = Date::parse_iso(day).ok_or_else(|| format!("{day:?} is no date"))?;
                let shown = model
                    .grid()
                    .iter()
                    .flatten()
                    .flatten()
                    .any(|cell| cell.date == date);
                if !shown {
                    return Err(format!("{day} is not in the grid"));
                }
                CalendarEvent::Pick(date)
            } else if let Some(label) = text("chip") {
                super::calendar::chip_date(label, model.today())
                    .map(CalendarEvent::Pick)
                    .ok_or_else(|| format!("no quick pick {label:?}"))?
            } else {
                match text("calendar").ok_or("the calendar over the form takes the clicks")? {
                    "previous" => CalendarEvent::Show(-1),
                    "next" => CalendarEvent::Show(1),
                    "clear" if !open.form.due().trim().is_empty() => CalendarEvent::Clear,
                    "clear" => return Err("Clear is not shown with no due date".into()),
                    "inside" => CalendarEvent::Refocus,
                    "outside" => CalendarEvent::Outside,
                    other => return Err(format!("unknown calendar target {other:?}")),
                }
            };
            return Ok(FormEvent::Calendar(calendar));
        }
        if let Some(name) = text("status") {
            return wren_markdown::status::STATUSES
                .iter()
                .find(|row| row.name == name)
                .map(|row| FormEvent::Status(row.kind))
                .ok_or_else(|| format!("no status {name:?}"));
        }
        if let Some(name) = text("priority") {
            return PRIORITIES
                .iter()
                .find(|level| level.map_or("None", |level| level.name()) == name)
                .map(|level| FormEvent::Priority(*level))
                .ok_or_else(|| format!("no priority {name:?}"));
        }
        if let Some(name) = text("suggestion") {
            return if self.owner_suggestions().iter().any(|each| each == name) {
                Ok(FormEvent::AddOwner(name.to_owned()))
            } else {
                Err(format!("@{name} is not suggested"))
            };
        }
        if let Some(name) = text("remove") {
            return if open.form.owners().iter().any(|each| each == name) {
                Ok(FormEvent::RemoveOwner(name.to_owned()))
            } else {
                Err(format!("@{name} is no owner of the task"))
            };
        }
        Ok(match string(params, "target")? {
            "save" => FormEvent::Save,
            "cancel" | "outside" => FormEvent::Cancel,
            "calendar" => FormEvent::OpenCalendar,
            other => return Err(format!("unknown form target {other:?}")),
        })
    }

    /// A key pressed in the task form, as its keys and then the app's own
    /// key handling take it: the form's keys ([`crate::form_keys::route`]),
    /// a character typed (with no Ctrl, Alt or logo key) or Backspace, at
    /// the end of the focused field's text, and Escape and shortcuts heard
    /// by the app as a real press is. With the calendar open over the form,
    /// its field takes the key.
    pub(super) fn form_key(
        &mut self,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Task<Message> {
        use crate::form_keys::{next_slot, route, FormKey, Routed};
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;
        let Some(open) = &self.task_form else {
            return Task::none();
        };
        if let Some(model) = &open.calendar {
            let typed = model.typed().to_owned();
            return self.calendar_field_key(&typed, key, modifiers, |event| {
                Message::TaskForm(FormEvent::Calendar(event))
            });
        }
        let focus = open.focus;
        let typed = !open.form.pending_owner().trim().is_empty() || open.suggestion.is_some();
        let text = match focus {
            Slot::Description => Some(open.form.description().to_owned()),
            Slot::Due => Some(open.form.due().to_owned()),
            Slot::Owners => Some(open.form.pending_owner().to_owned()),
            _ => None,
        };
        let plain = !(modifiers.control() || modifiers.alt() || modifiers.logo());
        let mut tasks = Vec::new();
        let mut send = |app: &mut Self, event| tasks.push(app.update(Message::TaskForm(event)));
        // The form's save keys, as its keys read the keymap.
        let save = crate::keys::shortcut(
            &key,
            iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            modifiers,
        )
        .and_then(|accel| {
            self.preferences
                .keymap
                .action(&accel)
                .and_then(super::commands::CommandId::parse)
        })
        .is_some_and(|id| {
            matches!(
                id,
                super::commands::CommandId::TaskEdit | super::commands::CommandId::ToggleTask
            )
        });
        let captured = match route(focus.kind(), &key, modifiers, false, typed, save) {
            Routed::Drop => true,
            Routed::Tab(back) => {
                if focus == Slot::Owners && typed {
                    send(self, FormEvent::Key(FormKey::Complete));
                }
                let next = next_slot(focus.index(), Slot::ALL.len(), back);
                send(self, FormEvent::Key(FormKey::Focus(next)));
                tasks.push(Slot::ALL[next].focus());
                true
            }
            Routed::Key(form_key) => {
                let form_key = match form_key {
                    FormKey::Press(_) => FormKey::Press(focus.index()),
                    other => other,
                };
                send(self, FormEvent::Key(form_key));
                true
            }
            Routed::Pass => {
                let changed = text.and_then(|text| match key.as_ref() {
                    Key::Character(character) if plain => Some(format!("{text}{character}")),
                    Key::Named(Named::Space) if plain => Some(format!("{text} ")),
                    Key::Named(Named::Backspace) => {
                        let mut text = text;
                        text.pop();
                        Some(text)
                    }
                    _ => None,
                });
                match changed {
                    Some(changed) => {
                        let event = match focus {
                            Slot::Description => FormEvent::Description(changed),
                            Slot::Due => FormEvent::Due(changed),
                            _ => FormEvent::Owner(changed),
                        };
                        send(self, event);
                        true
                    }
                    None => false,
                }
            }
        };
        let status = if captured {
            iced::event::Status::Captured
        } else {
            iced::event::Status::Ignored
        };
        let event = iced::keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        };
        if let Some(input) =
            crate::keys::input(&event).filter(|input| crate::keys::reaches_app(input, status))
        {
            tasks.push(self.update(Message::Key(crate::keys::with_status(input, status))));
        }
        Task::batch(tasks)
    }

    /// The task form as it is drawn: `{"open": false}` while it is closed.
    fn form_snapshot(&self) -> Value {
        let Some(open) = &self.task_form else {
            return json!({"open": false});
        };
        let form = &open.form;
        let (caption, invalid) = super::task_form::due_caption(form);
        let rectangle = |bounds: iced::Rectangle| json!({"x": bounds.x, "y": bounds.y, "width": bounds.width, "height": bounds.height});
        json!({
            "open": true,
            "kind": match form.kind() {
                wren_markdown::task::FormKind::Edit => "edit",
                wren_markdown::task::FormKind::New => "new",
                wren_markdown::task::FormKind::Make => "make",
            },
            "title": open.title(),
            "line": form.line(),
            "focus": open.focus.name(),
            "description": form.description(),
            "status": super::task_form::status_name(form.status()),
            "due": form.due(),
            "due_value": super::task_form::due_value_name(form),
            "due_caption": caption,
            "due_invalid": invalid,
            "priority": form.priority().map(|level| level.name()),
            "owners": form.owners(),
            "owner_text": open.form.pending_owner(),
            "suggestions": self.owner_suggestions(),
            "suggestion": open.suggestion,
            "writes": form.preview(),
            "error": open.error,
            "calendar": open.calendar.as_ref().map(|model| json!({
                "typed": model.typed(),
                "selected": model.selected().to_string(),
                "month": model.month().to_string(),
                "caption": model.caption().to_string(),
                "bounds": self.task_form_calendar_bounds().map(rectangle),
            })),
            "due_bounds": self.task_form_due_bounds().map(rectangle),
        })
    }

    /// The calendar as it is drawn: `{"open": false}` while it is closed.
    fn calendar_snapshot(&self) -> Value {
        let Some(open) = &self.calendar else {
            return json!({"open": false});
        };
        let model = &open.model;
        let caption = model.caption();
        let (anchor_line, anchor) = match open.anchor() {
            wren_editor::Anchor::Range { line, range } => {
                (Some(*line), Some([range.start, range.end]))
            }
            wren_editor::Anchor::Caret => (None, None),
        };
        let card = self.calendar_bounds();
        json!({
            "open": true,
            "line": open.line(),
            "typed": model.typed(),
            "selected": model.selected().to_string(),
            "today": model.today().to_string(),
            "current": model.current().map(|date| date.to_string()),
            "month": model.month().to_string(),
            "caption": caption.to_string(),
            "invalid": caption.is_invalid(),
            "clear": open.has_due(),
            "anchor": {"line": anchor_line, "range": anchor},
            "bounds": card.map(|card| json!({
                "x": card.x, "y": card.y, "width": card.width, "height": card.height,
            })),
        })
    }

    fn tasks_snapshot(&self) -> Value {
        if !self.preferences.tasks {
            return json!({"tasks": false});
        }
        let (line, tokens) = self.caret_line();
        json!({
            "tasks": true,
            "today": self.today().to_string(),
            "seen": self.clock.seen().to_string(),
            "day_changes": self.clock.changes(),
            "engine": match self.editor.backend() {
                wren_editor::BackendKind::Iced => "classic",
                wren_editor::BackendKind::Owned => "advanced",
            },
            "line": line,
            "tokens": tokens,
            "footer": self.caret_task().map(|summary| json!({
                "text": summary.text(),
                "overdue": summary.overdue(),
                "pieces": summary.pieces.iter().map(|piece| json!({
                    "text": piece.text,
                    "look": piece.look.name(),
                })).collect::<Vec<_>>(),
            })),
        })
    }

    /// The caret's line, read once by a highlighter run through it.
    ///
    /// First, the line as the task model reads it so far: its number, its
    /// text, and its task as the classifier reads it with the app's own
    /// options, `null` when the line opens none. A task is its box's byte
    /// range in the line, the symbol in the box, and its status' name and
    /// type.
    ///
    /// Second, its task spans other than the box, each as the editor's
    /// highlighter draws it: its kind, its text and byte range, the
    /// palette role its first byte is drawn in (`null` for the body
    /// colour, which no span sets) and whether it is bold. The line is
    /// highlighted with [`ROLES`], a palette whose roles all have colours
    /// of their own, so a colour names one role whatever the theme.
    ///
    /// Both are `null` unless a Markdown note is in the editor, or the
    /// in-memory evaluation text outside a workspace.
    fn caret_line(&self) -> (Value, Value) {
        if !self.markdown_in_editor() {
            return (Value::Null, Value::Null);
        }
        let number = self.editor.cursor().position.line;
        let Some((text, highlights, read)) = self.editor.highlighted_line(number, roles_palette())
        else {
            return (Value::Null, Value::Null);
        };
        let task = read.spans.iter().find_map(|span| match span.kind {
            SpanKind::Box(task) => Some(task),
            _ => None,
        });
        let line = json!({
            "number": number,
            "text": text,
            "task": task.map(|task| json!({
                "box": [task.start, task.end],
                "symbol": task.symbol.to_string(),
                "status": task.row().name,
                "type": task.status.name(),
            })),
        });
        let tokens = read.spans.iter().filter_map(|span| {
            let kind = match span.kind {
                SpanKind::Date { field, .. } => match field {
                    DateField::Due => "due",
                    DateField::Done => "done",
                    DateField::Cancelled => "cancelled",
                },
                SpanKind::Priority(_) => "priority",
                SpanKind::Owner { .. } => "owner",
                SpanKind::Id { .. } => "id",
                SpanKind::Tag => "tag",
                SpanKind::BlockId => "block id",
                SpanKind::Kept(_) => "kept",
                _ => return None,
            };
            let format = highlights
                .iter()
                .find(|(range, _)| range.contains(&span.range.start))
                .map(|(_, highlight)| highlight.to_format());
            let colour = format.and_then(|format| format.color).map(|colour| {
                ROLES
                    .iter()
                    .find(|(_, role)| *role == colour)
                    .map_or_else(|| format!("{colour:?}"), |(name, _)| (*name).to_owned())
            });
            let bold = format
                .and_then(|format| format.font)
                .is_some_and(|font| font.weight == iced::font::Weight::Bold);
            Some(json!({
                "kind": kind,
                "text": text.get(span.range.clone()),
                "start": span.range.start,
                "end": span.range.end,
                "colour": colour,
                "bold": bold,
            }))
        });
        let tokens = Value::Array(tokens.collect());
        (line, tokens)
    }
}

/// Every role of the editor's palette with a colour of its own, so the
/// colour of a highlight names its role.
const ROLES: [(&str, Color); 13] = [
    ("text", Color::from_rgb8(1, 0, 0)),
    ("muted", Color::from_rgb8(2, 0, 0)),
    ("faint", Color::from_rgb8(3, 0, 0)),
    ("accent", Color::from_rgb8(4, 0, 0)),
    ("heading", Color::from_rgb8(5, 0, 0)),
    ("heading2", Color::from_rgb8(6, 0, 0)),
    ("link", Color::from_rgb8(7, 0, 0)),
    ("code", Color::from_rgb8(8, 0, 0)),
    ("quote", Color::from_rgb8(9, 0, 0)),
    ("completed", Color::from_rgb8(10, 0, 0)),
    ("date", Color::from_rgb8(11, 0, 0)),
    ("overdue", Color::from_rgb8(12, 0, 0)),
    ("owner", Color::from_rgb8(13, 0, 0)),
];

/// The palette of [`ROLES`].
fn roles_palette() -> wren_editor::Palette {
    let role = |name: &str| {
        ROLES
            .iter()
            .find(|(role, _)| *role == name)
            .map_or(Color::BLACK, |(_, color)| *color)
    };
    wren_editor::Palette {
        text: role("text"),
        muted: role("muted"),
        faint: role("faint"),
        accent: role("accent"),
        heading: role("heading"),
        heading2: role("heading2"),
        link: role("link"),
        code: role("code"),
        quote: role("quote"),
        completed: role("completed"),
        date: role("date"),
        overdue: role("overdue"),
        owner: role("owner"),
    }
}
