use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::text_editor::{Action, Cursor, Edit, Motion, Position};
use iced::Task;
use serde_json::{json, Value};

use super::commands::{Command, CommandId, Gateway, ItemTarget, ReorderTarget, Scope};
use super::palette::{Key, Mode};
use super::{focus_editor, App, Message};
use crate::panes::{Pane, PANES};
use crate::test_ipc::Request;
use wren_core::fs_store::Fault;
use wren_core::model::{GroupId, ItemRef, NoteId, Parent, SectionId};
use wren_core::ops::{Op, Session};

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
            "navigation.get" => request.respond(Ok(self.navigation_snapshot())),
            "palette.bounds" => {
                if let Some((generation, key)) = self.palette.as_ref().and_then(|palette|palette.active.and_then(|index|palette.results.get(index)).map(|row|(palette.generation,&row.key))) {
                    let request = Arc::clone(request);
                    return Ok(iced::advanced::widget::operate(crate::palette_scroll::measure("palette-results".into(), super::palette::result_id(generation,key))).map(move |geometry| {
                        let rectangle = |bounds: iced::Rectangle|json!({"x":bounds.x,"y":bounds.y,"width":bounds.width,"height":bounds.height});
                        Message::Reply(Arc::clone(&request),Ok(geometry.map(|geometry|json!({"viewport":rectangle(geometry.viewport),"content":rectangle(geometry.content),"row":rectangle(geometry.row),"translation_y":geometry.translation.y,"fully_visible":geometry.row.y-geometry.translation.y >= geometry.viewport.y-0.5 && geometry.row.y-geometry.translation.y+geometry.row.height <= geometry.viewport.y+geometry.viewport.height+0.5})).unwrap_or(Value::Null)))
                    }));
                }
                request.respond(Ok(Value::Null));
            }
            "navigation.bounds" => {
                let request = Arc::clone(request);
                return Ok(iced::advanced::widget::operate(crate::drag_view::inspect()).map(move |snapshot| {
                    let rectangle = |bounds: iced::Rectangle| json!({"x":bounds.x,"y":bounds.y,"width":bounds.width,"height":bounds.height});
                    Message::Reply(Arc::clone(&request), Ok(json!({"root":rectangle(snapshot.geometry.root),"pressed":snapshot.pressed,"rows":snapshot.geometry.rows.iter().map(|row| json!({"key":row_json(row.row),"bounds":rectangle(row.bounds),"visible":rectangle(row.visible)})).collect::<Vec<_>>()})))
                }));
            }
            "command.run" | "palette.open" | "palette.query" | "palette.step" | "palette.activate" | "palette.close" | "filter.set" | "view.focus" | "view.width" | "view.sort" | "column.open" | "workspace.add" | "workspace.remove" | "test.folder.begin" | "test.folder.complete" => {
                let outcome = self.navigation_request(&request.op, params)?;
                request.respond(Ok(json!({"correlation":outcome.processed.correlation,"disposition":outcome.processed.disposition.key(),"warning":outcome.processed.warning,"navigation":self.navigation_snapshot()})));
                return Ok(outcome.task);
            }
            "key.nav" => {
                // An uncaptured navigation key, as if no text widget had it.
                let nav = match string(params, "key")? {
                    "up" => crate::keys::Nav::Up,
                    "down" => crate::keys::Nav::Down,
                    "left" => crate::keys::Nav::Left,
                    "right" => crate::keys::Nav::Right,
                    "enter" => crate::keys::Nav::Enter,
                    "escape" => crate::keys::Nav::Escape,
                    _ => return Err("key must be up, down, left, right, enter, or escape".into()),
                };
                let task = self.update(Message::Key(crate::keys::Input::Nav(nav)));
                request.respond(Ok(self.navigation_snapshot()));
                return Ok(task);
            }
            "shortcut" => {
                // A key binding firing, with its active-column scoping.
                let id = CommandId::parse(string(params, "id")?).ok_or("Unknown command ID")?;
                if id.scope() == Scope::Editor {
                    // The editor's key policy routes the bound key, and only
                    // a focused editor sees it.
                    let keymap = &self.preferences.keymap;
                    let trigger = keymap.accel(id.key()).ok_or("The command has no key bound")?;
                    let wren_editor::KeyRoute::Intent(intent) =
                        super::preferences::key_route(
                            keymap,
                            trigger,
                            self.editor.markdown_assist(),
                        )
                    else {
                        return Err("The editor does not route this command's key".into());
                    };
                    let request = Arc::clone(request);
                    return Ok(iced::widget::operation::is_focused("note-editor").map(
                        move |focused| {
                            Message::EditorShortcut(Arc::clone(&request), focused.then_some(intent))
                        },
                    ));
                }
                let task = self.run_shortcut(id);
                request.respond(Ok(self.navigation_snapshot()));
                return Ok(task);
            }
            "row.click" => {
                let row = row_param(params)?;
                let generation = self.gesture_generation;
                let event = if params.get("double").and_then(Value::as_bool).unwrap_or(false) {
                    crate::drag::DragEvent::Open { generation, row }
                } else {
                    crate::drag::DragEvent::Activate { generation, row }
                };
                let task = self.drag_event(generation, event);
                request.respond(Ok(self.navigation_snapshot()));
                return Ok(task);
            }
            "dialog.name" => { let task = self.update(Message::DialogName(string(params, "name")?.into())); request.respond(Ok(self.navigation_snapshot())); return Ok(task); }
            "dialog.submit" | "dialog.confirm_trash" | "dialog.close" => {
                let task = self.update(match request.op.as_str() { "dialog.submit" => Message::SubmitDialog, "dialog.confirm_trash" => Message::ConfirmTrash, _ => Message::CancelDialog });
                request.respond(Ok(self.navigation_snapshot())); return Ok(task);
            }
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
            "test.timer.hold" => {
                if self.endpoint.is_none() || self.test_sandbox.is_none() {
                    return Err("Holding the timer requires the private test IPC endpoint".into());
                }
                self.timer_held = params.get("held").and_then(Value::as_bool).ok_or("held must be a boolean")?;
                request.respond(Ok(Value::Null));
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
            "perf.view" => {
                let mut times = self.view_micros.borrow_mut();
                let recent: Vec<u64> = times.iter().copied().collect();
                if params.get("clear").and_then(Value::as_bool).unwrap_or(false) {
                    times.clear();
                }
                request.respond(Ok(serde_json::json!({ "view_micros": recent })));
            }
            "editor.set" => {
                if self.workspace_mode || self.session.is_some() || self.loading.is_some() {
                    return Err("editor.set is only available for the in-memory spike; edit saved notes through the editor".into());
                }
                let contents = string(params, "text")?;
                self.editor.replace_text(contents, false);
                self.set_words(contents);
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
                    "editor.insert" => wren_editor::Message::Action(Action::Edit(Edit::Paste(
                        Arc::new(string(params, "text")?.into()),
                    ))),
                    "editor.undo" => wren_editor::Message::Undo,
                    _ => wren_editor::Message::Redo,
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
                let task = self.edit(wren_editor::Message::Action(action));
                request.respond(Ok(self.editor_snapshot()));
                return Ok(task.chain(focus_editor()));
            }
            "editor.command" => {
                // An editing key as the editor widget sends it, through the
                // same App::edit path.
                self.ensure_editor_ready()?;
                let intent = intent(string(params, "intent")?)?;
                let task = self.edit(wren_editor::Message::Intent(intent));
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
            "focus.get" => {
                let request = Arc::clone(request);
                return Ok(iced::advanced::widget::operate(Focused::default()).map(move |focused| {
                    let bounds: serde_json::Map<String, Value> = focused.bounds.iter().map(|(name, b)| ((*name).to_string(), json!({"x":b.x,"y":b.y,"width":b.width,"height":b.height}))).collect();
                    Message::Reply(Arc::clone(&request), Ok(json!({"named":focused.named,"unnamed":focused.unnamed,"bounds":bounds})))
                }));
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
            "app.screenshot" | "test.capture.after_layout" => {
                let path = PathBuf::from(string(params, "path")?);
                if !path.is_absolute() {
                    return Err("screenshot path must be absolute".into());
                }
                if self.capture_pending {
                    return Err("screenshot already in flight".into());
                }
                let window = self.window.ok_or("window is not open")?;
                self.capture_pending = true;
                if request.op == "test.capture.after_layout" {
                    // Deliberately omit the frame barrier: the toolkit must draw
                    // the rebuilt interface rather than replay expired glyphs.
                    let request = Arc::clone(request);
                    return Ok(iced::window::screenshot(window).map(move |capture| {
                        Message::RpcCaptured(Arc::clone(&request), path.clone(), capture)
                    }));
                }
                return Ok(self.capture_frame(Arc::clone(request), path));
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
            #[cfg(target_os = "macos")]
            "updater.status" => request.respond(Ok(updater_status())),
            #[cfg(target_os = "macos")]
            "updater.check" => {
                wren_sparkle::check_for_update_information()?;
                request.respond(Ok(updater_status()));
            }
            _ => return Err(format!("unknown test operation: {}", request.op)),
        }
        Ok(Task::none())
    }

    fn navigation_request(&mut self, operation: &str, params: &Value) -> Result<Gateway, String> {
        Ok(match operation {
            "command.run" => {
                let id = string(params, "id")?;
                let command = match id {
                    "item.rename" => {
                        Command::Rename(target(params.get("target").ok_or("target is required")?)?)
                    }
                    "item.delete" => {
                        Command::Delete(target(params.get("target").ok_or("target is required")?)?)
                    }
                    "item.reveal" => {
                        Command::Reveal(target(params.get("target").ok_or("target is required")?)?)
                    }
                    _ => Command::Run(CommandId::parse(id).ok_or("Unknown command ID")?),
                };
                self.dispatch(command)
            }
            "palette.open" => {
                let mode = match string(params, "mode")? {
                    "commands" => Mode::Commands,
                    "jump" => Mode::Jump,
                    "jump_section" => Mode::JumpSection,
                    "jump_group" => Mode::JumpGroup,
                    "search" => Mode::Search,
                    "workspaces" => Mode::Workspaces,
                    "themes" => Mode::Themes,
                    "fonts" => Mode::Fonts,
                    "outline" => Mode::Outline,
                    "item_actions" => Mode::ItemActions(target(
                        params.get("target").ok_or("target is required")?,
                    )?),
                    "reorder" => {
                        Mode::Reorder(target(params.get("target").ok_or("target is required")?)?)
                    }
                    "move_note" => Mode::MoveNote(
                        params
                            .get("note")
                            .map(|_| id(params, "note").map(NoteId))
                            .transpose()?
                            .or(self.selected_note)
                            .ok_or("No move note is selected")?,
                    ),
                    "move_group" => Mode::MoveGroup(
                        params
                            .get("group")
                            .map(|_| id(params, "group").map(GroupId))
                            .transpose()?
                            .or(self.selected_group)
                            .ok_or("No move group is selected")?,
                    ),
                    _ => return Err("Unknown palette mode".into()),
                };
                self.open_palette(mode)
            }
            "palette.query" => self.palette_query(string(params, "query")?.into()),
            "palette.step" => self.palette_step(
                params
                    .get("delta")
                    .and_then(Value::as_i64)
                    .and_then(|delta| i32::try_from(delta).ok())
                    .ok_or("delta must be i32")?,
            ),
            "palette.activate" => {
                if let Some(key) = params.get("key") {
                    if params.get("index").is_some() {
                        return Err("Choose either a stable result key or an index".into());
                    }
                    let generation = params
                        .get("generation")
                        .map(|_| id(params, "generation"))
                        .transpose()?
                        .unwrap_or_else(|| self.workspace_generation());
                    self.activate_palette_key(generation, result_key(key)?)
                } else {
                    self.activate_palette(
                        params
                            .get("index")
                            .map(|_| {
                                id(params, "index").and_then(|index| {
                                    usize::try_from(index).map_err(|_| "index must be usize".into())
                                })
                            })
                            .transpose()?,
                    )
                }
            }
            "palette.close" => self.close_palette(),
            "filter.set" => self.dispatch(Command::Filter {
                pane: string(params, "pane")?.into(),
                query: string(params, "query")?.into(),
            }),
            "view.focus" => self.dispatch(Command::Focus(
                params
                    .get("enabled")
                    .map(|enabled| enabled.as_bool().ok_or("enabled must be boolean"))
                    .transpose()?,
            )),
            "view.width" => {
                let pane = match string(params, "pane")? {
                    "sections" => Pane::Sections,
                    "groups" => Pane::Groups,
                    "notes" => Pane::Notes,
                    _ => return Err("Unknown pane".into()),
                };
                let width = params
                    .get("width")
                    .and_then(Value::as_f64)
                    .filter(|width| {
                        width.is_finite() && *width >= 0.0 && *width <= f64::from(u16::MAX)
                    })
                    .ok_or("width must be finite and nonnegative")?
                    as f32;
                self.dispatch(Command::Width { pane, width })
            }
            "view.sort" => self.dispatch(Command::Sort(string(params, "mode")?.into())),
            "column.open" => self.dispatch(Command::OpenColumn(match string(params, "column")? {
                "sections" => super::columns::Column::Sections,
                "groups" => super::columns::Column::Groups,
                "notes" => super::columns::Column::Notes,
                "editor" => super::columns::Column::Editor,
                _ => return Err("column must be sections, groups, notes, or editor".into()),
            })),
            "workspace.add" => self.dispatch(Command::OpenWorkspace {
                path: PathBuf::from(string(params, "path")?),
                name: params
                    .get("name")
                    .map(|name| {
                        name.as_str()
                            .map(str::to_owned)
                            .ok_or("name must be a string")
                    })
                    .transpose()?,
            }),
            "workspace.remove" => self.dispatch(Command::RemoveWorkspace(PathBuf::from(string(
                params, "path",
            )?))),
            "test.folder.begin" => {
                if self.folder_request.is_some() {
                    self.blocked("A folder picker is already pending")
                } else {
                    self.begin_folder_request();
                    self.finish_action(
                        super::commands::Disposition::AsyncPending,
                        None,
                        Task::none(),
                    )
                }
            }
            "test.folder.complete" => {
                let path = match params.get("path").ok_or("path is required")? {
                    Value::Null => None,
                    value => Some(PathBuf::from(
                        value.as_str().ok_or("path must be a string or null")?,
                    )),
                };
                self.folder_chosen(id(params, "tag")?, path)
            }
            _ => return Err("Unknown navigation adapter".into()),
        })
    }

    pub(super) fn navigation_snapshot(&self) -> Value {
        let visible_sections = self
            .model()
            .map(|model| {
                model
                    .sections
                    .iter()
                    .filter(|section| {
                        super::navigation::matches_name(&self.filters.sections, &section.name)
                    })
                    .map(|section| section.id.0)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let visible_groups = self
            .model()
            .map(|model| {
                model
                    .groups
                    .iter()
                    .filter(|group| {
                        Some(group.section) == self.browser_section()
                            && super::navigation::matches_name(&self.filters.groups, &group.name)
                    })
                    .map(|group| group.id.0)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut visible_parents = Vec::new();
        if self.browser_section().is_some() {
            visible_parents.extend(
                visible_groups
                    .iter()
                    .map(|id| row_json(crate::drag::RowKind::Group(GroupId(*id)))),
            );
        }
        let preferred = PANES
            .iter()
            .map(|&pane| (pane.key().to_owned(), json!(self.widths.get(pane))))
            .collect::<serde_json::Map<_, _>>();
        let effective = self
            .widths
            .effective(self.visible_panes(), self.viewport.width);
        let effective = PANES
            .iter()
            .map(|&pane| {
                (
                    pane.key().to_owned(),
                    json!(if self.focus || self.model().is_none() {
                        0.0
                    } else {
                        effective.get(pane)
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let modal = self.palette.as_ref().map(|palette| json!({"mode":palette.mode.key(),"purpose":if palette.workspace_removal {"remove_workspace"} else {"navigate"},"generation":palette.generation,"source":palette.source.map(item_target_json),"query":palette.query,"active_index":palette.active,"active_key":palette.active.and_then(|index| palette.results.get(index)).map(|row| key_json(&row.key)),"error":palette.error,"results":palette.results.iter().map(|row| json!({"key":key_json(&row.key),"note":match row.key { Key::Note(note) => Some(note.0), _ => None },"title":row.title,"path":row.path,"snippet":row.snippet,"highlight":row.highlight.as_ref().map(|range| json!({"start":range.start,"end":range.end})),"hit":if row.content_hit { "content" } else { "title" },"content_match":match &row.action { super::palette::Action::Note(target) => target.content_match.as_ref().map(|range|json!({"start":range.start,"end":range.end})), _ => None },"index_revision":match &row.action { super::palette::Action::Note(target) => Some(target.index_revision), _ => None },"current":row.current,"enabled":row.enabled,"group":row.group,"shortcut":row.shortcut,"state":match row.key { Key::Command(id) => self.command_state(id), _ => None }})).collect::<Vec<_>>()}));
        let dialog = self.dialog.as_ref().map(|dialog| json!({"kind":format!("{:?}",dialog.kind),"name":dialog.name,"error":dialog.error}));
        let gesture = self.drag_preview.as_ref().map(|preview| json!({"source":row_json(preview.source),"position":{"x":preview.position.x,"y":preview.position.y},"target":preview.target.as_ref().map(|target|row_json(target.row)),"insertion":preview.target.as_ref().and_then(|target|target.insertion).map(|line|json!({"x":line.line.x,"y":line.line.y,"width":line.line.width,"height":line.line.height})),"spring":preview.spring.map(|spring|format!("{spring:?}"))}));
        let processed = self.last_processed.as_ref().map(|processed|json!({"correlation":processed.correlation,"disposition":processed.disposition.key(),"warning":processed.warning}));
        json!({
            "root":self.model().map(|model|&model.root),"loading":self.loading.is_some(),"modal":modal,"dialog":dialog,"focus":self.focus,"column":self.column.key(),"fold":self.fold,"selected_section":self.selected_section.map(|id|id.0),"selected_group":self.selected_group.map(|id|id.0),"selected_note":self.selected_note.map(|id|id.0),
            "filters":{"sections":self.filters.sections,"groups":self.filters.groups,"notes":self.filters.notes},
            "preferred_widths":preferred,"effective_widths":effective,"viewport":{"width":self.viewport.width,"height":self.viewport.height},
            "visible":{"sections":visible_sections,"groups":visible_groups,"parents":visible_parents,"notes":self.note_hits().iter().map(|hit| hit.note.0).collect::<Vec<_>>()},
            "history":self.session.as_ref().map(|session| session.state().history().iter().map(|path| path.as_path().to_string_lossy().into_owned()).collect::<Vec<_>>()).unwrap_or_default(),"history_position":self.session.as_ref().and_then(|session|session.state().history_position()),"mru":self.session.as_ref().map(|session| session.state().recent().iter().map(|path|path.as_path().to_string_lossy().into_owned()).collect::<Vec<_>>()).unwrap_or_default(),"can_back":self.session.as_ref().is_some_and(Session::can_back),"can_forward":self.session.as_ref().is_some_and(Session::can_forward),
            "index_revision":self.session.as_ref().map(|session| session.index().revision()),"index_count":self.session.as_ref().map_or(0,|session| session.index().len()),"sort":self.note_sort(),
            "registrations":self.config.as_ref().map(|config| config.workspaces().iter().map(|entry| json!({"name":entry.name,"path":entry.path})).collect::<Vec<_>>()).unwrap_or_default(),"active_registration":self.config.as_ref().and_then(|config| config.active()),"registry_warning":self.registry_warning,"metadata_warning":self.metadata_warning,"config_writable":self.config.as_ref().is_some_and(|config| config.writable()),"config_directory":self.config.as_ref().and_then(|config|config.directory()),"folder_request":self.folder_request,
            "generation":self.workspace_generation(),"gesture_generation":self.gesture_generation,"spring_section":self.spring_section.map(|section|section.0),"gesture_preview":gesture,
            "conflict_copy":self.conflict_copy.as_ref().map(|(generation,path)|json!({"generation":generation,"path":path.as_path().to_string_lossy(),"enabled":self.command_enabled(CommandId::OpenConflictCopy)})),
            "processed":processed,
        })
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
                "id":note.id.0,"parent":parent_json(Parent::Group(note.group)),"title":note.title,
                "path":note.path.as_path().to_string_lossy(),"subtitle":note.subtitle,"bytes":note.bytes,
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "document":document.map(|document| json!({
                "note":document.note.0,"text":document.text,"path":document.path.to_string_lossy(),
                "dirty":document.dirty,"error":document.error,
            })),
            "pending_durability":pending,
            "last_save_micros":u64::try_from(last_save).unwrap_or(u64::MAX),
            "trash_path":self.test_sandbox.as_ref().map(|sandbox| sandbox.join("trash")),
            "navigation":self.navigation_snapshot(),
        })
    }

    fn editor_snapshot(&self) -> Value {
        let cursor = self.editor.cursor();
        let position = |p: Position| json!({"line":p.line,"column":p.column});
        json!({
            "text":self.editor.text(),"cursor":position(cursor.position),
            "selection":cursor.selection.map(position),"selected_text":self.editor.selected_text(),
            "line_count":self.editor.line_count(),"first_visible_line":self.editor.first_visible_line(),"words":self.words,"revision":self.edit_revision,
            "can_undo":self.editor.can_undo(),"can_redo":self.editor.can_redo(),
            "find_open":self.find_open,"find_query":self.find_query,"find_count":self.editor.find_count(),
            "find_current":self.editor.find_current(),"edit_micros":self.edit_micros,
            "soft_wrap":true,"theme":self.definition.id,"markdown_assist":self.editor.markdown_assist(),
            "engine":match self.editor.backend() {
                wren_editor::BackendKind::Iced => "classic",
                wren_editor::BackendKind::Owned => "advanced",
            },
            "font_size":self.editor.font_size(),
            "font_family":match self.editor.font().family {
                iced::font::Family::Name(name) => name,
                _ => "",
            },
        })
    }
}

fn intent(name: &str) -> Result<wren_editor::Intent, String> {
    use wren_editor::Intent;
    Ok(match name {
        "enter" => Intent::Enter,
        "shift_enter" => Intent::ShiftEnter,
        "tab" => Intent::Tab,
        "shift_tab" => Intent::ShiftTab,
        "backspace" => Intent::Backspace,
        "delete" => Intent::Delete,
        "delete_word_back" => Intent::DeleteWordBackward,
        "delete_word_forward" => Intent::DeleteWordForward,
        "delete_to_line_start" => Intent::DeleteToTextStart,
        "home" => Intent::Home { extend: false },
        "select_home" => Intent::Home { extend: true },
        "document_start" => Intent::DocumentStart { extend: false },
        "select_document_start" => Intent::DocumentStart { extend: true },
        "document_end" => Intent::DocumentEnd { extend: false },
        "select_document_end" => Intent::DocumentEnd { extend: true },
        "toggle_task" => Intent::ToggleTask,
        "bold" => Intent::Bold,
        "italic" => Intent::Italic,
        "code" => Intent::Code,
        "move_line_up" => Intent::MoveLineUp,
        "move_line_down" => Intent::MoveLineDown,
        _ => return Err(format!("unknown editor intent {name:?}")),
    })
}

fn result_key(value: &Value) -> Result<Key, String> {
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or("result key must contain exactly one command, note, destination, item_action, reorder, or workspace")?;
    if object.contains_key("command") {
        return Ok(Key::Command(
            CommandId::parse(string(value, "command")?).ok_or("Unknown command ID")?,
        ));
    }
    if object.contains_key("note") {
        return Ok(Key::Note(NoteId(id(value, "note")?)));
    }
    if object.contains_key("section") {
        return Ok(Key::Section(SectionId(id(value, "section")?)));
    }
    if object.contains_key("group") {
        return Ok(Key::Group(GroupId(id(value, "group")?)));
    }
    if let Some(destination) = object.get("destination") {
        return Ok(Key::Destination(parent(destination)?));
    }
    if object.contains_key("theme") {
        let id = string(value, "theme")?;
        return Ok(Key::Theme(if id == "system" {
            None
        } else {
            Some(wren_core::theme::builtin(id).ok_or("Unknown theme")?.id)
        }));
    }
    if object.contains_key("font") {
        let family = string(value, "font")?;
        return crate::fonts::families()
            .find(family)
            .map(Key::Font)
            .ok_or_else(|| "Unknown font family".into());
    }
    if object.contains_key("workspace") {
        return Ok(Key::Workspace(PathBuf::from(string(value, "workspace")?)));
    }
    if let Some(heading) = object.get("heading") {
        if heading.as_object().is_none_or(|object| object.len() != 2) {
            return Err("heading requires revision and line".into());
        }
        return Ok(Key::Heading {
            revision: id(heading, "revision")?,
            line: usize::try_from(id(heading, "line")?).map_err(|_| "line must be usize")?,
        });
    }
    if let Some(action) = object.get("item_action") {
        let object = action
            .as_object()
            .filter(|object| object.len() == 2)
            .ok_or("item_action requires target and command")?;
        return Ok(Key::ItemAction {
            target: item_target(object.get("target").ok_or("target is required")?)?,
            id: CommandId::parse(string(action, "command")?).ok_or("Unknown command ID")?,
        });
    }
    if let Some(reorder) = object.get("reorder") {
        let object = reorder
            .as_object()
            .filter(|object| object.len() == 2)
            .ok_or("reorder requires source and before")?;
        return Ok(Key::Reorder(ReorderTarget {
            source: item_target(object.get("source").ok_or("source is required")?)?,
            before: match object.get("before").ok_or("before is required")? {
                Value::Null => None,
                value => Some(item_target(value)?),
            },
        }));
    }
    Err("Unknown palette result key".into())
}

fn key_json(key: &Key) -> Value {
    match key {
        Key::Command(id) => {
            json!({"command":super::commands::DEFINITIONS.iter().find(|definition| definition.id == *id).map(|definition| definition.key)})
        }
        Key::Note(note) => json!({"note":note.0}),
        Key::Section(section) => json!({"section":section.0}),
        Key::Group(group) => json!({"group":group.0}),
        Key::Destination(parent) => json!({"destination":parent_json(*parent)}),
        Key::Workspace(path) => json!({"workspace":path}),
        Key::Theme(id) => json!({"theme":id.unwrap_or("system")}),
        Key::Font(family) => json!({"font":family}),
        Key::Heading { revision, line } => json!({"heading":{"revision":revision,"line":line}}),
        Key::ItemAction { target, id } => {
            json!({"item_action":{"target":item_target_json(*target),"command":id.key()}})
        }
        Key::Reorder(target) => {
            json!({"reorder":{"source":item_target_json(target.source),"before":target.before.map(item_target_json)}})
        }
    }
}

fn item_target(value: &Value) -> Result<ItemTarget, String> {
    let object = value
        .as_object()
        .filter(|object| object.len() == 3)
        .ok_or("item target requires generation, item, and parent")?;
    Ok(ItemTarget {
        generation: id(value, "generation")?,
        item: target(object.get("item").ok_or("item is required")?)?,
        parent: match object.get("parent").ok_or("parent is required")? {
            Value::Null => None,
            value => Some(parent(value)?),
        },
    })
}

fn item_target_json(target: ItemTarget) -> Value {
    let item = match target.item {
        ItemRef::Section(id) => json!({"section":id.0}),
        ItemRef::Group(id) => json!({"group":id.0}),
        ItemRef::Note(id) => json!({"note":id.0}),
    };
    json!({"generation":target.generation,"item":item,"parent":target.parent.map(parent_json)})
}

fn row_param(params: &Value) -> Result<crate::drag::RowKind, String> {
    let row = params.get("row").ok_or("row is required")?;
    let field = |name: &str| row.get(name).and_then(Value::as_u64);
    if let Some(section) = field("section") {
        Ok(crate::drag::RowKind::Section(SectionId(section)))
    } else if let Some(group) = field("group") {
        Ok(crate::drag::RowKind::Group(GroupId(group)))
    } else if let Some(note) = field("note") {
        Ok(crate::drag::RowKind::Note(NoteId(note)))
    } else {
        Err("row must name a section, group, or note id".into())
    }
}

fn row_json(row: crate::drag::RowKind) -> Value {
    match row {
        crate::drag::RowKind::Section(section) => json!({"section":section.0}),
        crate::drag::RowKind::Group(group) => json!({"group":group.0}),
        crate::drag::RowKind::Note(note) => json!({"note":note.0}),
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
            group: group(params.get("parent").ok_or("parent is required")?)?,
            title: string(params, "title")?.into(),
        },
        "select_parent" => Op::SelectParent {
            parent: parent(params.get("parent").ok_or("parent is required")?)?,
        },
        "select_section" => Op::SelectSection {
            section: SectionId(id(params, "section")?),
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
            to: group(params.get("to").ok_or("to is required")?)?,
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

fn group(value: &Value) -> Result<GroupId, String> {
    match target(value)? {
        ItemRef::Group(id) => Ok(id),
        _ => Err("notes live in groups; expected a group ID".into()),
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

/// Text-entry widgets the `focus.get` op reports by name.
const FOCUS_NAMES: &[&str] = &[
    "filter-sections",
    "filter-groups",
    "filter-notes",
    "note-editor",
    "find-query",
    "palette-query",
    "item-name",
];

/// Every focused widget in the tree: the known ones by name, the rest (pane
/// grips during a resize) as a count. `bounds` places each known widget on
/// screen, focused or not, so a test can click it.
#[derive(Debug, Clone, Default)]
struct Focused {
    named: Vec<&'static str>,
    unnamed: usize,
    bounds: Vec<(&'static str, iced::Rectangle)>,
}

impl iced::advanced::widget::Operation<Focused> for Focused {
    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<Focused>),
    ) {
        operate(self);
    }

    fn focusable(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        state: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        let name = id.and_then(|id| {
            FOCUS_NAMES
                .iter()
                .copied()
                .find(|name| *id == iced::advanced::widget::Id::new(name))
        });
        if let Some(name) = name {
            self.bounds.push((name, bounds));
        }
        if state.is_focused() {
            match name {
                Some(name) => self.named.push(name),
                None => self.unnamed += 1,
            }
        }
    }

    fn finish(&self) -> iced::advanced::widget::operation::Outcome<Focused> {
        iced::advanced::widget::operation::Outcome::Some(self.clone())
    }
}

#[cfg(target_os = "macos")]
fn updater_status() -> Value {
    let status = wren_sparkle::status();
    json!({
        "started": status.started,
        "unavailable": status.unavailable,
        "can_check": wren_sparkle::can_check(),
        "menu_enabled": crate::macos_menu::updates_enabled(),
        "checks": status.checks,
        "last": status.last.map(|check| json!({
            "outcome": check.outcome.as_str(),
            "version": check.version,
            "detail": check.detail,
        })),
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
                to: GroupId(2)
            }
        ));
        assert!(matches!(
            core_op(&json!({"op":"create_note","parent":{"group":2},"title":"Note"})).unwrap(),
            Op::CreateNote {
                group: GroupId(2),
                ..
            }
        ));
    }

    #[test]
    fn palette_result_keys_round_trip_without_accepting_ambiguous_targets() {
        for key in [
            Key::Command(CommandId::NewNote),
            Key::Note(NoteId(3)),
            Key::Destination(Parent::Group(GroupId(2))),
            Key::Workspace("/tmp/workspace".into()),
            Key::Heading {
                revision: 7,
                line: 12,
            },
            Key::ItemAction {
                target: ItemTarget {
                    generation: 4,
                    item: ItemRef::Group(GroupId(2)),
                    parent: Some(Parent::Section(SectionId(1))),
                },
                id: CommandId::ReorderGroup,
            },
            Key::Reorder(ReorderTarget {
                source: ItemTarget {
                    generation: 4,
                    item: ItemRef::Section(SectionId(1)),
                    parent: None,
                },
                before: Some(ItemTarget {
                    generation: 4,
                    item: ItemRef::Section(SectionId(2)),
                    parent: None,
                }),
            }),
            Key::Reorder(ReorderTarget {
                source: ItemTarget {
                    generation: 4,
                    item: ItemRef::Group(GroupId(2)),
                    parent: Some(Parent::Section(SectionId(1))),
                },
                before: None,
            }),
        ] {
            assert_eq!(result_key(&key_json(&key)).unwrap(), key);
        }
        for value in [
            json!({"note":1,"command":"note.new"}),
            json!({"note":-1}),
            json!({"destination":{"note":1}}),
            json!({"command":"save"}),
            json!({"heading":{"line":3}}),
            json!({"heading":{"revision":1,"line":3,"title":"x"}}),
            json!({"reorder":{"source":{"generation":1,"item":{"group":2},"parent":{"section":1}}}}),
            json!({"item_action":{"target":{"generation":1,"item":{"note":2},"parent":{"section":1}},"command":"note.rename","extra":true}}),
            json!({"item_action":{"target":{"generation":1,"item":{"note":2},"parent":{"section":1},"path":"external"},"command":"note.rename"}}),
        ] {
            assert!(result_key(&value).is_err());
        }
    }

    #[test]
    fn requests_reject_ambiguous_ids_paths_and_draft_edit_bypasses() {
        for params in [
            json!({"op":"edit_draft","note":1,"contents":"bypass"}),
            json!({"op":"write_note","note":1,"contents":"bypass"}),
            json!({"op":"create_note","parent":{"note":1},"title":"No"}),
            json!({"op":"create_note","parent":{"section":1},"title":"No"}),
            json!({"op":"move_note","note":3,"to":{"section":1}}),
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

    fn navigation_fixture() -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        std::fs::create_dir_all(directory.path().join("Section/Group")).unwrap();
        std::fs::write(directory.path().join("Section/Group/First.md"), "first").unwrap();
        std::fs::write(directory.path().join("Section/Group/Second.md"), "second").unwrap();
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        app.workspace_mode = true;
        app.session = Some(Session::open(directory.path()).unwrap());
        let first = app
            .model()
            .unwrap()
            .notes
            .iter()
            .find(|note| note.title == "First")
            .unwrap()
            .id;
        let _ = app.process_core(Op::OpenNote { note: Some(first) });
        (directory, app)
    }

    #[test]
    fn navigation_adapters_share_the_failed_flush_barrier_and_return_correlated_disposition() {
        let (directory, mut app) = navigation_fixture();
        let _ = app.edit(wren_editor::Message::Action(Action::Edit(Edit::Paste(
            Arc::new("dirty ".into()),
        ))));
        let _ = app
            .navigation_request("palette.open", &json!({"mode":"jump"}))
            .unwrap();
        let _ = app
            .navigation_request("palette.query", &json!({"query":"Second"}))
            .unwrap();
        app.session.as_mut().unwrap().fail_next(Fault::Write);
        let outcome = app
            .navigation_request("palette.activate", &json!({}))
            .unwrap();
        assert_eq!(outcome.processed.disposition.key(), "blocked");
        assert_eq!(
            app.navigation_snapshot()["processed"]["correlation"],
            outcome.processed.correlation
        );
        assert_eq!(app.navigation_snapshot()["modal"]["query"], "Second");
        assert_eq!(app.editor.text(), "dirty first");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Section/Group/First.md")).unwrap(),
            "first"
        );
    }

    #[test]
    fn navigation_adapters_project_filters_width_sort_and_modal_selection() {
        let (_directory, mut app) = navigation_fixture();
        for (operation, params) in [
            ("filter.set", json!({"pane":"notes","query":"first"})),
            ("view.width", json!({"pane":"sections","width":999})),
            ("view.sort", json!({"mode":"title"})),
            ("view.focus", json!({"enabled":true})),
            ("palette.open", json!({"mode":"commands"})),
            ("palette.query", json!({"query":"Sort notes:"})),
            ("palette.step", json!({"delta":1})),
        ] {
            assert!(
                app.navigation_request(operation, &params)
                    .unwrap()
                    .processed
                    .disposition
                    .committed(),
                "{operation}"
            );
        }
        let state = app.navigation_snapshot();
        assert_eq!(state["preferred_widths"]["sections"], 360.0);
        assert_eq!(state["sort"], "title");
        assert_eq!(state["focus"], true);
        assert_eq!(state["filters"]["notes"], "first");
        assert_eq!(state["modal"]["active_index"], 1);
        assert_eq!(state["modal"]["results"].as_array().unwrap().len(), 2);
        assert!(state["modal"]["active_key"].is_object());
        assert!(app
            .navigation_request("command.run", &json!({"id":"save"}))
            .is_err());
        assert!(app
            .navigation_request("view.width", &json!({"pane":"outline","width":200}))
            .is_err());
        assert!(
            app.navigation_request("filter.set", &json!({"pane":"editor","query":"no"}))
                .unwrap()
                .processed
                .disposition
                == super::super::commands::Disposition::Blocked
        );
    }

    #[test]
    fn fake_picker_adapter_uses_tagged_production_completion_and_cancel_keeps_draft() {
        let (_directory, mut app) = navigation_fixture();
        let _ = app.edit(wren_editor::Message::Action(Action::Edit(Edit::Paste(
            Arc::new("draft ".into()),
        ))));
        let pending = app
            .navigation_request("test.folder.begin", &json!({}))
            .unwrap()
            .processed
            .correlation;
        let tag = app.navigation_snapshot()["folder_request"]
            .as_u64()
            .unwrap();
        let stale = app
            .navigation_request("test.folder.complete", &json!({"tag":tag+1,"path":null}))
            .unwrap();
        assert_eq!(stale.processed.disposition.key(), "blocked");
        let cancelled = app
            .navigation_request("test.folder.complete", &json!({"tag":tag,"path":null}))
            .unwrap();
        assert_eq!(cancelled.processed.disposition.key(), "completed");
        assert_eq!(cancelled.processed.correlation, pending);
        assert!(app.loading.is_none());
        assert_eq!(app.editor.text(), "draft first");
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
    }

    #[test]
    fn private_rpc_exposes_context_reorder_sources_and_stable_typed_keys() {
        let (_directory, mut app) = navigation_fixture();
        let selected = app.selected_note;
        let section = app.selected_section.unwrap();
        assert!(app
            .navigation_request(
                "palette.open",
                &json!({"mode":"item_actions","target":{"section":section.0}})
            )
            .unwrap()
            .processed
            .disposition
            .committed());
        let snapshot = app.navigation_snapshot();
        assert_eq!(snapshot["modal"]["mode"], "item_actions");
        assert_eq!(
            snapshot["modal"]["source"]["item"],
            json!({"section":section.0})
        );
        let key = snapshot["modal"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["key"]["item_action"]["command"] == "section.reorder")
            .unwrap()["key"]
            .clone();
        assert!(app
            .navigation_request("palette.activate", &json!({"key":key}))
            .unwrap()
            .processed
            .disposition
            .committed());
        let snapshot = app.navigation_snapshot();
        assert_eq!(snapshot["modal"]["mode"], "reorder");
        assert_eq!(
            snapshot["modal"]["results"][0]["key"]["reorder"]["before"],
            Value::Null
        );
        assert_eq!(
            snapshot["modal"]["source"]["generation"],
            app.workspace_generation()
        );
        assert_eq!(app.selected_note, selected);
        app.conflict_copy = Some((
            app.workspace_generation(),
            app.model().unwrap().notes[1].path.clone(),
        ));
        assert_eq!(app.navigation_snapshot()["conflict_copy"]["enabled"], true);
        assert!(app
            .navigation_request(
                "palette.open",
                &json!({"mode":"item_actions","target":{"no_group":section.0}})
            )
            .is_err());
    }

    #[test]
    fn private_rpc_query_guards_keep_the_previous_valid_query_and_editor() {
        let (_directory, mut app) = navigation_fixture();
        let before = app.editor.text();
        app.navigation_request("palette.open", &json!({"mode":"commands"}))
            .unwrap();
        app.navigation_request("palette.query", &json!({"query":"theme"}))
            .unwrap();
        let outcome = app
            .navigation_request("palette.query", &json!({"query":"x".repeat(4097)}))
            .unwrap();
        assert_eq!(outcome.processed.disposition.key(), "blocked");
        assert_eq!(app.navigation_snapshot()["modal"]["query"], "theme");
        assert!(app.navigation_snapshot()["modal"]["error"]
            .as_str()
            .unwrap()
            .contains("4,096"));
        app.navigation_request("palette.close", &json!({})).unwrap();
        app.navigation_request("filter.set", &json!({"pane":"notes","query":"First"}))
            .unwrap();
        assert_eq!(
            app.navigation_request(
                "filter.set",
                &json!({"pane":"notes","query":"a ".repeat(129)})
            )
            .unwrap()
            .processed
            .disposition
            .key(),
            "blocked"
        );
        assert_eq!(app.navigation_snapshot()["filters"]["notes"], "First");
        assert_eq!(app.editor.text(), before);
    }

    #[test]
    fn no_workspace_snapshot_has_the_documented_nulls_and_empty_arrays() {
        let (app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
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
        let directory = crate::test_support::tempdir();
        std::fs::create_dir_all(directory.path().join("Section/Group")).unwrap();
        std::fs::write(directory.path().join("Section/Group/Note.md"), "saved").unwrap();
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        app.workspace_mode = true;
        app.session = Some(Session::open(directory.path()).unwrap());
        let note = app.session.as_ref().unwrap().model().notes[0].id;
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        let _ = app.edit(wren_editor::Message::Action(Action::Edit(Edit::Paste(
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
        assert_eq!(snapshot["document"]["path"], "Section/Group/Note.md");
        assert_eq!(snapshot["document"]["dirty"], true);
        assert!(snapshot["document"]["error"].is_null());
        assert!(snapshot["notes"][0]["parent"]["group"].is_u64());
        let _ = app.apply_core(Op::Flush).unwrap();
        assert_eq!(app.session_snapshot()["document"]["dirty"], false);
    }
}
