//! The task requests of the test harness (discovery/tasks-build.md §1):
//! `tasks.get`, the caret line's task model, how the editor draws its
//! tokens and how the status bar spells the task out (`footer`, `null` off
//! a task line), the effective `tasks` setting and today; and
//! `tasks.set_today`, which pins the clock. With tasks off every request
//! but `tasks.get` is refused, and `tasks.get` says only that they are off,
//! so a lane with the setting off can assert the gate.

use std::sync::Arc;

use iced::{Color, Task};
use serde_json::{json, Value};
use wren_markdown::date::Date;
use wren_markdown::span::SpanKind;
use wren_markdown::task::DateField;

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
            _ => return Err(format!("unknown test operation: {}", request.op)),
        }
        Ok(Task::none())
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
