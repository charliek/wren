//! The task requests of the test harness (discovery/tasks-build.md §1):
//! `tasks.get`, the caret line's task model, the effective `tasks` setting
//! and today; and `tasks.set_today`, which pins the clock. With tasks off
//! every request but `tasks.get` is refused, and `tasks.get` says only
//! that they are off, so a lane with the setting off can assert the gate.

use std::sync::Arc;

use iced::Task;
use serde_json::{json, Value};
use wren_markdown::block;
use wren_markdown::date::Date;

use super::board::Surface;
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
        json!({
            "tasks": true,
            "today": self.today().to_string(),
            "seen": self.clock.seen().to_string(),
            "day_changes": self.clock.changes(),
            "engine": match self.editor.backend() {
                wren_editor::BackendKind::Iced => "classic",
                wren_editor::BackendKind::Owned => "advanced",
            },
            "line": self.caret_task_line(),
        })
    }

    /// The caret's line as the task model reads it so far: its number, its
    /// text, and whether the classifier reads it as a task and a done one.
    /// `null` unless a Markdown note is in the editor, or the in-memory
    /// evaluation text outside a workspace.
    fn caret_task_line(&self) -> Value {
        let open = !self.workspace_mode
            || self
                .session
                .as_ref()
                .is_some_and(|session| session.document().is_some());
        if !open || self.surface() != Surface::Text {
            return Value::Null;
        }
        let text = self.editor.text();
        let lines: Vec<&str> = wren_markdown::line_ranges(&text)
            .map(|(range, _)| &text[range])
            .collect();
        let number = self.editor.cursor().position.line;
        let Some(line) = lines.get(number) else {
            return Value::Null;
        };
        let task = block::scan(&lines[..])
            .get(number)
            .and_then(|info| info.item)
            .and_then(|item| item.task);
        json!({
            "number": number,
            "text": line,
            "task": task.is_some(),
            "done": task.map(|task| task.done),
        })
    }
}
