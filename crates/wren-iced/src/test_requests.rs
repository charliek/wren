use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::text_editor::{Action, Cursor, Edit, Motion, Position};
use iced::Task;
use serde_json::{json, Value};

use super::{focus_editor, App, Message};
use crate::test_ipc::Request;
use wren_core::fs_store::Fault;
use wren_core::model::{GroupId, ItemRef, NoteId, Parent, SectionId};
use wren_core::ops::{Op, Session};
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
                "protocol":1,"pid":std::process::id(),"ready":self.ready && self.window.is_some() && self.loading.is_none(),
                "theme":self.definition.id,"editor":true,
            }))),
            "session.get" => request.respond(Ok(self.session_snapshot())),
            "core.apply" => {
                let task = self.apply_core(core_op(params)?)?;
                request.respond(Ok(self.session_snapshot()));
                return Ok(task);
            }
            "workspace.open" => {
                let path = PathBuf::from(string(params, "path")?);
                let task = self.apply_core(Op::OpenWorkspace { path })?;
                self.workspace_reply = Some(Arc::clone(request));
                return Ok(task);
            }
            "test.fail_next" => {
                if self.endpoint.is_none() || self.test_sandbox.is_none() {
                    return Err("Fault injection requires the private test IPC endpoint".into());
                }
                let fault = match string(params, "fault")? {
                    "write" => Fault::Write,
                    "permissions" => Fault::Permissions,
                    "file_sync" => Fault::FileSync,
                    "before_publish" => Fault::BeforePublish,
                    "directory_sync" => Fault::DirectorySync,
                    _ => return Err("Unknown test fault".into()),
                };
                self.session.as_mut().ok_or("No workspace is open, or it is still loading")?.fail_next(fault);
                request.respond(Ok(Value::Null));
            }
            "window.blur" => {
                self.window_unfocused()?;
                request.respond(Ok(self.session_snapshot()));
            }
            "editor.get" => request.respond(Ok(self.editor_snapshot())),
            "editor.set" => {
                if self.workspace_mode || self.session.is_some() || self.loading.is_some() {
                    return Err("editor.set is only available for the in-memory spike; edit saved notes through the editor".into());
                }
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
                self.ensure_editor_ready()?;
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
                self.ensure_editor_ready()?;
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
                self.ensure_editor_ready()?;
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
                self.ensure_editor_ready()?;
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
                if self.quit_reply.is_some() {
                    return Err("A quit request is already waiting for the workspace load".into());
                }
                let task = self.request_quit()?;
                if self.loading.is_some() {
                    self.quit_reply = Some(Arc::clone(request));
                } else {
                    request.respond(Ok(Value::Null));
                }
                return Ok(task);
            }
            _ => return Err(format!("unknown test operation: {}", request.op)),
        }
        Ok(Task::none())
    }

    pub(super) fn session_snapshot(&self) -> Value {
        let model = self.model();
        let document = self.document_projection();
        let generation = self
            .session
            .as_ref()
            .map(Session::generation)
            .or_else(|| self.loading.as_ref().map(|loading| loading.generation))
            .unwrap_or(0);
        let pending = self
            .session
            .as_ref()
            .map(Session::pending_durability)
            .or_else(|| {
                self.loading
                    .as_ref()
                    .map(|loading| loading.pending_durability)
            })
            .unwrap_or(false);
        let last_save = self
            .session
            .as_ref()
            .map(Session::last_save_micros)
            .or_else(|| {
                self.loading
                    .as_ref()
                    .map(|loading| loading.last_save_micros)
            })
            .unwrap_or(0);
        json!({
            "root":model.map(|model| model.root.to_string_lossy()),
            "generation":generation,"loading":self.loading.is_some(),"banner":self.banner,
            "selected_section":self.selected_section.map(|id| id.0),
            "selected_group":self.selected_group.map(|id| id.0),
            "selected_note":self.selected_note.map(|id| id.0),
            "sections":model.map(|model| model.sections.iter().map(|section| json!({
                "id":section.id.0,"name":section.name,"path":section.path.as_path().to_string_lossy(),
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "groups":model.map(|model| model.groups.iter().map(|group| json!({
                "id":group.id.0,"section":group.section.0,"name":group.name,"path":group.path.as_path().to_string_lossy(),
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "notes":model.map(|model| model.notes.iter().map(|note| json!({
                "id":note.id.0,"parent":parent_json(note.parent),"title":note.title,
                "path":note.path.as_path().to_string_lossy(),"subtitle":note.subtitle,"bytes":note.bytes,
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "document":document.map(|document| json!({
                "note":document.note.0,"text":document.text,"path":document.path.to_string_lossy(),
                "dirty":document.dirty,"error":document.error,
            })),
            "pending_durability":pending,
            "last_save_micros":u64::try_from(last_save).unwrap_or(u64::MAX),
            "trash_path":self.test_sandbox.as_ref().map(|sandbox| sandbox.join("trash")),
        })
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

fn core_op(params: &Value) -> Result<Op, String> {
    Ok(match string(params, "op")? {
        "create_section" => Op::CreateSection {
            name: string(params, "name")?.into(),
        },
        "create_group" => Op::CreateGroup {
            section: SectionId(id(params, "section")?),
            name: string(params, "name")?.into(),
        },
        "create_note" => Op::CreateNote {
            parent: parent(params.get("parent").ok_or("parent is required")?)?,
            title: string(params, "title")?.into(),
        },
        "open_note" => Op::OpenNote {
            note: match params.get("note").ok_or("note is required")? {
                Value::Null => None,
                value => Some(NoteId(
                    value
                        .as_u64()
                        .ok_or("note must be an unsigned integer or null")?,
                )),
            },
        },
        "rename" => Op::Rename {
            target: target(params.get("target").ok_or("target is required")?)?,
            new_name: string(params, "new_name")?.into(),
        },
        "delete" => Op::Delete {
            target: target(params.get("target").ok_or("target is required")?)?,
        },
        "move_note" => Op::MoveNote {
            note: NoteId(id(params, "note")?),
            to: parent(params.get("to").ok_or("to is required")?)?,
        },
        "move_group" => Op::MoveGroup {
            group: GroupId(id(params, "group")?),
            to: SectionId(id(params, "to")?),
        },
        "reorder" => Op::Reorder {
            target: target(params.get("target").ok_or("target is required")?)?,
            before: match params.get("before").ok_or("before is required")? {
                Value::Null => None,
                value => Some(target(value)?),
            },
        },
        "flush" => Op::Flush,
        "reconcile" => Op::Reconcile,
        "back" => Op::Back,
        "forward" => Op::Forward,
        _ => return Err("Unknown core operation; draft edits must use the real editor".into()),
    })
}

fn id(params: &Value, name: &str) -> Result<u64, String> {
    params
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{name} must be an unsigned integer"))
}

fn target(value: &Value) -> Result<ItemRef, String> {
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or("target must contain exactly one section, group, or note ID")?;
    if object.contains_key("section") {
        Ok(ItemRef::Section(SectionId(id(value, "section")?)))
    } else if object.contains_key("group") {
        Ok(ItemRef::Group(GroupId(id(value, "group")?)))
    } else if object.contains_key("note") {
        Ok(ItemRef::Note(NoteId(id(value, "note")?)))
    } else {
        Err("target must contain a section, group, or note ID".into())
    }
}

fn parent(value: &Value) -> Result<Parent, String> {
    match target(value)? {
        ItemRef::Section(id) => Ok(Parent::Section(id)),
        ItemRef::Group(id) => Ok(Parent::Group(id)),
        ItemRef::Note(_) => Err("parent must contain a section or group ID".into()),
    }
}

fn parent_json(parent: Parent) -> Value {
    match parent {
        Parent::Section(id) => json!({"section":id.0}),
        Parent::Group(id) => json!({"group":id.0}),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Options;

    #[test]
    fn documented_core_requests_use_ids_and_the_shared_operation_types() {
        for params in [
            json!({"op":"create_section","name":"Section"}),
            json!({"op":"create_group","section":1,"name":"Group"}),
            json!({"op":"create_note","parent":{"section":1},"title":"Note"}),
            json!({"op":"create_note","parent":{"group":2},"title":"Note"}),
            json!({"op":"open_note","note":3}),
            json!({"op":"open_note","note":null}),
            json!({"op":"rename","target":{"note":3},"new_name":"Renamed"}),
            json!({"op":"delete","target":{"group":2}}),
            json!({"op":"move_note","note":3,"to":{"group":2}}),
            json!({"op":"move_group","group":2,"to":1}),
            json!({"op":"reorder","target":{"section":1},"before":null}),
            json!({"op":"reorder","target":{"section":1},"before":{"section":4}}),
            json!({"op":"flush"}),
            json!({"op":"reconcile"}),
            json!({"op":"back"}),
            json!({"op":"forward"}),
        ] {
            assert!(core_op(&params).is_ok(), "{params}");
        }
        assert!(matches!(
            core_op(&json!({"op":"move_note","note":3,"to":{"group":2}})).unwrap(),
            Op::MoveNote {
                note: NoteId(3),
                to: Parent::Group(GroupId(2))
            }
        ));
    }

    #[test]
    fn requests_reject_ambiguous_ids_paths_and_draft_edit_bypasses() {
        for params in [
            json!({"op":"edit_draft","note":1,"contents":"bypass"}),
            json!({"op":"write_note","note":1,"contents":"bypass"}),
            json!({"op":"create_note","parent":{"note":1},"title":"No"}),
            json!({"op":"delete","target":{"note":1,"group":2}}),
            json!({"op":"delete","target":{"path":"Section/Note.md"}}),
            json!({"op":"open_note","note":-1}),
            json!({"op":"open_note","note":"1"}),
            json!({"op":"open_note"}),
            json!({"op":"move_group","group":2,"to":{"section":1}}),
        ] {
            assert!(core_op(&params).is_err(), "{params}");
        }
    }

    #[test]
    fn no_workspace_snapshot_has_the_documented_nulls_and_empty_arrays() {
        let (app, _) = App::new(Options::default());
        let snapshot = app.session_snapshot();
        assert!(snapshot["root"].is_null());
        assert!(snapshot["document"].is_null());
        assert_eq!(snapshot["generation"], 0);
        assert_eq!(snapshot["loading"], false);
        assert_eq!(snapshot["sections"], json!([]));
        assert_eq!(snapshot["groups"], json!([]));
        assert_eq!(snapshot["notes"], json!([]));
        assert_eq!(snapshot["last_save_micros"], 0);
    }

    #[test]
    fn snapshot_reports_active_draft_identity_and_dirty_state() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("Section")).unwrap();
        std::fs::write(directory.path().join("Section/Note.md"), "saved").unwrap();
        let (mut app, _) = App::new(Options::default());
        app.workspace_mode = true;
        app.session = Some(Session::open(directory.path()).unwrap());
        let note = app.session.as_ref().unwrap().model().notes[0].id;
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        let _ = app.edit(editor_view::Message::Action(Action::Edit(Edit::Paste(
            Arc::new("draft ".into()),
        ))));
        let snapshot = app.session_snapshot();
        assert_eq!(
            snapshot["root"],
            directory.path().to_string_lossy().as_ref()
        );
        assert_eq!(snapshot["selected_note"], note.0);
        assert_eq!(snapshot["document"]["note"], note.0);
        assert_eq!(snapshot["document"]["text"], "draft saved");
        assert_eq!(snapshot["document"]["path"], "Section/Note.md");
        assert_eq!(snapshot["document"]["dirty"], true);
        assert!(snapshot["document"]["error"].is_null());
        assert!(snapshot["notes"][0]["parent"]["section"].is_u64());
        let _ = app.apply_core(Op::Flush).unwrap();
        assert_eq!(app.session_snapshot()["document"]["dirty"], false);
    }
}
