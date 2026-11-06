use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::text_editor::{Action, Cursor, Edit, Motion, Position};
use iced::Task;
use serde_json::{json, Value};

use super::{focus_editor, App, Message};
use crate::test_ipc::Request;
use wren_iced::editor_view;

impl App {
    pub(super) fn handle_test(&mut self, request: Arc<Request>) -> Task<Message> {
        match self.dispatch_test(&request) {
            Ok(task) => task,
            Err(error) => {
                request.respond(Err(error));
                Task::none()
            }
        }
    }

    fn dispatch_test(&mut self, request: &Arc<Request>) -> Result<Task<Message>, String> {
        let params = &request.params;
        match request.op.as_str() {
            "identify" => request.respond(Ok(json!({
                "protocol":1,"pid":std::process::id(),"ready":self.ready && self.window.is_some(),
                "theme":self.definition.id,"editor":true,
            }))),
            "editor.get" => request.respond(Ok(self.editor_snapshot())),
            "editor.set" => {
                let contents = string(params, "text")?;
                self.editor.replace_text(contents, false);
                self.words = contents.split_whitespace().count();
                self.edit_revision += 1;
                self.edit_micros.clear();
                self.find_query.clear();
                self.editor.set_find_query("");
                request.respond(Ok(self.editor_snapshot()));
                return Ok(focus_editor());
            }
            "editor.insert" | "editor.undo" | "editor.redo" => {
                let message = match request.op.as_str() {
                    "editor.insert" => editor_view::Message::Action(Action::Edit(Edit::Paste(
                        Arc::new(string(params, "text")?.into()),
                    ))),
                    "editor.undo" => editor_view::Message::Undo,
                    _ => editor_view::Message::Redo,
                };
                let task = self.edit(message);
                request.respond(Ok(self.editor_snapshot()));
                return Ok(task.chain(focus_editor()));
            }
            "editor.action" => {
                let action = match string(params, "kind")? {
                    "select_all" => Action::SelectAll,
                    "backspace" => Action::Edit(Edit::Backspace),
                    "delete" => Action::Edit(Edit::Delete),
                    "enter" => Action::Edit(Edit::Enter),
                    "scroll" => Action::Scroll {
                        lines: params
                            .get("lines")
                            .and_then(Value::as_i64)
                            .and_then(|n| i32::try_from(n).ok())
                            .ok_or("lines must be i32")?,
                    },
                    "move" | "select" => {
                        let motion = match string(params, "motion")? {
                            "start" => Motion::DocumentStart,
                            "end" => Motion::DocumentEnd,
                            "left" => Motion::Left,
                            "right" => Motion::Right,
                            "up" => Motion::Up,
                            "down" => Motion::Down,
                            "home" => Motion::Home,
                            "line_end" => Motion::End,
                            _ => return Err("unknown editor motion".into()),
                        };
                        if params["kind"] == "select" {
                            Action::Select(motion)
                        } else {
                            Action::Move(motion)
                        }
                    }
                    _ => return Err("unknown editor action".into()),
                };
                let task = self.edit(editor_view::Message::Action(action));
                request.respond(Ok(self.editor_snapshot()));
                return Ok(task.chain(focus_editor()));
            }
            "editor.cursor" => {
                let cursor_position = position(params)?;
                let selection = params
                    .get("selection")
                    .filter(|value| !value.is_null())
                    .map(position)
                    .transpose()?;
                self.editor.move_cursor(Cursor {
                    position: cursor_position,
                    selection,
                });
                request.respond(Ok(self.editor_snapshot()));
            }
            "editor.find" => {
                self.find_open = true;
                if let Some(query) = params.get("query") {
                    self.find_query = query.as_str().ok_or("query must be a string")?.into();
                    self.editor.set_find_query(&self.find_query);
                }
                match params
                    .get("direction")
                    .and_then(Value::as_str)
                    .unwrap_or("next")
                {
                    "next" => {
                        self.editor.find_next();
                    }
                    "previous" => {
                        self.editor.find_previous();
                    }
                    _ => return Err("unknown find direction".into()),
                }
                request.respond(Ok(self.editor_snapshot()));
            }
            "editor.focus" => {
                request.respond(Ok(Value::Null));
                return Ok(focus_editor());
            }
            "theme.set" => {
                let definition =
                    wren_core::theme::builtin(string(params, "id")?).ok_or("unknown theme")?;
                let task = self.update(Message::ChooseTheme(Some(definition)));
                request.respond(Ok(self.editor_snapshot()));
                return Ok(task);
            }
            "clipboard.write" => {
                return Ok(
                    iced::clipboard::write(string(params, "text")?.into()).chain(Task::done(
                        Message::Reply(Arc::clone(request), Ok(Value::Null)),
                    )),
                );
            }
            "clipboard.read" => {
                let request = Arc::clone(request);
                return Ok(iced::clipboard::read().map(move |contents| {
                    Message::Reply(Arc::clone(&request), Ok(json!(contents)))
                }));
            }
            "app.screenshot" => {
                let path = PathBuf::from(string(params, "path")?);
                if !path.is_absolute() {
                    return Err("screenshot path must be absolute".into());
                }
                if self.capture_pending {
                    return Err("screenshot already in flight".into());
                }
                let window = self.window.ok_or("window is not open")?;
                self.capture_pending = true;
                let request = Arc::clone(request);
                return Ok(Task::future(self.frames.next(window)).then(
                    move |result| match result {
                        Ok(()) => {
                            let request = Arc::clone(&request);
                            let path = path.clone();
                            iced::window::screenshot(window).map(move |capture| {
                                Message::RpcCaptured(Arc::clone(&request), path.clone(), capture)
                            })
                        }
                        Err(error) => {
                            Task::done(Message::CaptureFailed(Arc::clone(&request), error))
                        }
                    },
                ));
            }
            "app.quit" => {
                request.respond(Ok(Value::Null));
                return Ok(iced::exit());
            }
            _ => return Err(format!("unknown test operation: {}", request.op)),
        }
        Ok(Task::none())
    }

    fn editor_snapshot(&self) -> Value {
        let cursor = self.editor.cursor();
        let position = |p: Position| json!({"line":p.line,"column":p.column});
        json!({
            "text":self.editor.text(),"cursor":position(cursor.position),
            "selection":cursor.selection.map(position),"selected_text":self.editor.selected_text(),
            "line_count":self.editor.line_count(),"words":self.words,"revision":self.edit_revision,
            "can_undo":self.editor.can_undo(),"can_redo":self.editor.can_redo(),
            "find_open":self.find_open,"find_query":self.find_query,"find_count":self.editor.find_count(),
            "find_current":self.editor.find_current(),"edit_micros":self.edit_micros,
            "soft_wrap":true,"theme":self.definition.id,
        })
    }
}

fn string<'a>(params: &'a Value, name: &str) -> Result<&'a str, String> {
    params
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn position(params: &Value) -> Result<Position, String> {
    let index = |name| {
        params
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| format!("{name} must be usize"))
    };
    Ok(Position {
        line: index("line")?,
        column: index("column")?,
    })
}
