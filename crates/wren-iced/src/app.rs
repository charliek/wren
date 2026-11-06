use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::theme::Mode;
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Element, Fill, Subscription, Task};
use wren_core::model::{GroupId, NoteId, Parent, SectionId};
use wren_core::ops::{Op, Session};
use wren_core::theme::{self as definitions, Theme};
use wren_iced::editor_view::{self, EditorView};

pub use crate::cli::Options;
use crate::{fonts, theme};

#[path = "workspace.rs"]
mod workspace;
#[path = "workspace_view.rs"]
mod workspace_view;

use workspace::{Dialog, DialogKind, LoadOutcome, Loading};

#[cfg(feature = "smoke-test")]
#[path = "test_requests.rs"]
mod test_requests;

pub struct App {
    definition: Theme,
    toolkit_theme: iced::Theme,
    system_mode: Mode,
    follows_system: bool,
    editor: EditorView,
    find_open: bool,
    find_query: String,
    words: usize,
    edit_revision: u64,
    edit_micros: Vec<u64>,
    window: Option<iced::window::Id>,
    session: Option<Session>,
    workspace_mode: bool,
    workspace_path: String,
    loading: Option<Loading>,
    load_generation: u64,
    selected_section: Option<SectionId>,
    selected_group: Option<GroupId>,
    selected_note: Option<NoteId>,
    editor_note: Option<NoteId>,
    banner: Option<String>,
    dialog: Option<Dialog>,
    quit_pending: bool,
    flush_on_load: bool,
    #[cfg(feature = "smoke-test")]
    test_sandbox: Option<PathBuf>,
    #[cfg(feature = "smoke-test")]
    workspace_reply: Option<Arc<crate::test_ipc::Request>>,
    #[cfg(feature = "smoke-test")]
    quit_reply: Option<Arc<crate::test_ipc::Request>>,
    #[cfg(feature = "smoke-test")]
    ready: bool,
    #[cfg(feature = "smoke-test")]
    smoke: Option<crate::smoke::Request>,
    #[cfg(feature = "smoke-test")]
    endpoint: Option<crate::test_ipc::Endpoint>,
    #[cfg(feature = "smoke-test")]
    capture_pending: bool,
    #[cfg(feature = "smoke-test")]
    frames: crate::test_capture::Frames,
}

#[derive(Debug, Clone)]
pub enum Message {
    SystemTheme(Mode),
    ChooseTheme(Option<Theme>),
    Editor(editor_view::Message),
    Core(Op),
    Tick,
    WorkspacePath(String),
    OpenWorkspace,
    Loaded(u64, Arc<Mutex<Option<LoadOutcome>>>),
    SelectParent(Parent),
    ShowDialog(DialogKind),
    DialogName(String),
    DialogDestination(Parent),
    SubmitDialog,
    ConfirmTrash,
    CancelDialog,
    DismissBanner,
    FindChanged(String),
    FindNext,
    FindPrevious,
    FindClose,
    Escape,
    Window(iced::window::Id, iced::window::Event),
    #[cfg(feature = "smoke-test")]
    FirstFrame(Result<(), String>),
    #[cfg(feature = "smoke-test")]
    CaptureFailed(std::sync::Arc<crate::test_ipc::Request>, String),
    #[cfg(feature = "smoke-test")]
    RendererReady(iced::window::Id, iced::system::Information),
    #[cfg(feature = "smoke-test")]
    Captured(iced::window::Screenshot),
    #[cfg(feature = "smoke-test")]
    Test(crate::test_ipc::Incoming),
    #[cfg(feature = "smoke-test")]
    RpcCaptured(
        std::sync::Arc<crate::test_ipc::Request>,
        std::path::PathBuf,
        iced::window::Screenshot,
    ),
    #[cfg(feature = "smoke-test")]
    Reply(
        std::sync::Arc<crate::test_ipc::Request>,
        Result<serde_json::Value, String>,
    ),
}

pub fn spike_text(large: bool) -> String {
    if large {
        (0..5000)
            .map(|line| {
                format!("- Line {line:04}: Markdown **editing**, `code`, and Unicode café 🐦.")
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        "# Wren editor spike\n\nThis is an in-memory editor evaluation, not a saved note.\n\n## Try the editor\n\n- Type Markdown, select text, and use the system clipboard.\n- Use **Ctrl+Z** to undo and **Ctrl+Shift+Z** to redo.\n- Press Ctrl+F to find text; Enter moves to the next match.\n- Soft wrap is enabled. Emoji and accents remain plain UTF-8.\n\n```rust\nlet notes = \"files are the truth\";\n```\n\n- [ ] Evaluate typing and navigation\n- [x] Keep the editor behind a toolkit seam\n".into()
    }
}

impl App {
    pub fn new(options: Options) -> (Self, Task<Message>) {
        #[cfg(feature = "smoke-test")]
        let definition = options
            .smoke
            .as_ref()
            .map_or(definitions::DARK, |request| request.theme);
        #[cfg(not(feature = "smoke-test"))]
        let definition = definitions::DARK;
        #[cfg(feature = "smoke-test")]
        let follows_system = options.smoke.is_none();
        #[cfg(not(feature = "smoke-test"))]
        let follows_system = true;
        let workspace = options.workspace.clone();
        let contents = if workspace.is_some() {
            String::new()
        } else {
            spike_text(options.large_spike)
        };
        let words = contents.split_whitespace().count();
        let mut app = Self {
            definition,
            toolkit_theme: theme::from_definition(definition),
            system_mode: Mode::None,
            follows_system,
            editor: EditorView::new(&contents),
            find_open: false,
            find_query: String::new(),
            words,
            edit_revision: 0,
            edit_micros: Vec::new(),
            window: None,
            session: None,
            workspace_mode: workspace.is_some(),
            workspace_path: String::new(),
            loading: None,
            load_generation: 0,
            selected_section: None,
            selected_group: None,
            selected_note: None,
            editor_note: None,
            banner: None,
            dialog: None,
            quit_pending: false,
            flush_on_load: false,
            #[cfg(feature = "smoke-test")]
            test_sandbox: options
                .test_socket
                .as_ref()
                .and_then(|path| path.parent())
                .map(PathBuf::from),
            #[cfg(feature = "smoke-test")]
            workspace_reply: None,
            #[cfg(feature = "smoke-test")]
            quit_reply: None,
            #[cfg(feature = "smoke-test")]
            ready: false,
            #[cfg(feature = "smoke-test")]
            smoke: options.smoke,
            #[cfg(feature = "smoke-test")]
            endpoint: options.endpoint,
            #[cfg(feature = "smoke-test")]
            capture_pending: false,
            #[cfg(feature = "smoke-test")]
            frames: crate::test_capture::Frames::default(),
        };
        let opening = workspace.map_or_else(Task::none, |path| {
            app.apply_core(Op::OpenWorkspace { path })
                .unwrap_or_else(|error| {
                    app.banner = Some(error);
                    Task::none()
                })
        });
        (
            app,
            Task::batch([iced::system::theme().map(Message::SystemTheme), opening]),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            #[cfg(feature = "smoke-test")]
            Message::Test(event) => match event {
                crate::test_ipc::Incoming::Request(request) => return self.handle_test(request),
                crate::test_ipc::Incoming::Failed(error) => {
                    self.banner = Some(format!("Test transport stopped: {error}. The window remains open so unsaved changes can be recovered."));
                }
            },
            #[cfg(feature = "smoke-test")]
            Message::RpcCaptured(request, path, capture) => {
                self.capture_pending = false;
                request.respond(crate::smoke::save_capture(&capture, &path)
                    .map(|()| serde_json::json!({"width":capture.size.width,"height":capture.size.height}))
                    .map_err(|error| error.to_string()));
            }
            #[cfg(feature = "smoke-test")]
            Message::Reply(request, result) => request.respond(result),
            Message::SystemTheme(mode) => {
                self.system_mode = mode;
                if self.follows_system {
                    self.set_theme(theme::definition_for_system(mode));
                }
            }
            Message::ChooseTheme(definition) => {
                self.follows_system = definition.is_none();
                self.set_theme(
                    definition.unwrap_or_else(|| theme::definition_for_system(self.system_mode)),
                );
            }
            Message::Editor(message) => return self.edit(message),
            Message::Core(op) => return self.apply_core(op).unwrap_or_else(|_| Task::none()),
            Message::Tick => {
                if self.session.is_some() {
                    let _ = self.apply_core(Op::Tick);
                }
            }
            Message::WorkspacePath(path) => self.workspace_path = path,
            Message::OpenWorkspace => {
                return self
                    .apply_core(Op::OpenWorkspace {
                        path: PathBuf::from(&self.workspace_path),
                    })
                    .unwrap_or_else(|_| Task::none());
            }
            Message::Loaded(generation, outcome) => {
                return self.workspace_loaded(generation, outcome)
            }
            Message::SelectParent(parent) => {
                let _ = self.select_parent(parent);
            }
            Message::ShowDialog(kind) => return self.show_dialog(kind),
            Message::DialogName(name) => {
                if let Some(dialog) = &mut self.dialog {
                    dialog.name = name;
                }
            }
            Message::DialogDestination(parent) => {
                if let Some(dialog) = &mut self.dialog {
                    dialog.destination = Some(parent);
                }
            }
            Message::SubmitDialog => return self.submit_dialog(false),
            Message::ConfirmTrash => return self.submit_dialog(true),
            Message::CancelDialog => {
                self.dialog = None;
                return focus_editor();
            }
            Message::DismissBanner => self.banner = None,
            Message::FindChanged(query) => {
                self.find_query = query;
                self.editor.set_find_query(&self.find_query);
                self.editor.find_next();
            }
            Message::FindNext => {
                self.editor.find_next();
            }
            Message::FindPrevious => {
                self.editor.find_previous();
            }
            Message::FindClose => {
                self.find_open = false;
                return focus_editor();
            }
            Message::Escape => {
                if self.dialog.take().is_none() {
                    self.find_open = false;
                }
                return focus_editor();
            }
            Message::Window(id, event) => match event {
                iced::window::Event::Opened { .. } => {
                    self.window = Some(id);
                    #[cfg(feature = "smoke-test")]
                    {
                        if self.smoke.is_some() {
                            return iced::system::information()
                                .map(move |info| Message::RendererReady(id, info));
                        }
                        if self.endpoint.is_some() {
                            return focus_editor().chain(
                                Task::future(self.frames.next(id)).map(Message::FirstFrame),
                            );
                        }
                        return focus_editor();
                    }
                    #[cfg(not(feature = "smoke-test"))]
                    return focus_editor();
                }
                iced::window::Event::CloseRequested => {
                    return self.request_quit().unwrap_or_else(|_| Task::none());
                }
                iced::window::Event::Unfocused => {
                    let _ = self.window_unfocused();
                }
                _ => {}
            },
            #[cfg(feature = "smoke-test")]
            Message::FirstFrame(result) => match result {
                Ok(()) => self.ready = true,
                Err(error) => eprintln!("Wren test readiness: {error}"),
            },
            #[cfg(feature = "smoke-test")]
            Message::CaptureFailed(request, error) => {
                self.capture_pending = false;
                request.respond(Err(error));
            }
            #[cfg(feature = "smoke-test")]
            Message::RendererReady(id, information) => {
                eprintln!("WREN_SMOKE_BACKEND={}", information.graphics_backend);
                eprintln!("WREN_SMOKE_ADAPTER={}", information.graphics_adapter);
                return iced::window::screenshot(id).map(Message::Captured);
            }
            #[cfg(feature = "smoke-test")]
            Message::Captured(capture) => {
                if let Some(request) = &self.smoke {
                    if let Err(error) = request.save(&capture) {
                        eprintln!("Wren smoke capture: {error}");
                    }
                }
                return iced::exit();
            }
        }
        Task::none()
    }

    fn edit(&mut self, message: editor_view::Message) -> Task<Message> {
        if self.ensure_editor_ready().is_err() || self.dialog.is_some() {
            return Task::none();
        }
        let started = Instant::now();
        let changed = match message {
            editor_view::Message::Action(action) => self.editor.perform(action),
            editor_view::Message::Undo => self.editor.undo(),
            editor_view::Message::Redo => self.editor.redo(),
            editor_view::Message::Find => {
                self.editor.break_group();
                self.find_open = true;
                return iced::widget::operation::focus("find-query");
            }
        };
        if changed {
            if let Some(note) = self.selected_note {
                let _ = self.apply_core(Op::EditDraft {
                    note,
                    contents: self.editor.text(),
                });
            }
            self.edit_revision += 1;
            self.words = self.editor.text().split_whitespace().count();
            if self.edit_micros.len() == 256 {
                self.edit_micros.remove(0);
            }
            self.edit_micros
                .push(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        #[cfg(feature = "smoke-test")]
        if self.smoke.is_some() {
            return self.smoke_view();
        }
        if self.workspace_mode {
            return self.workspace_view();
        }
        let colors = self.definition.colors;
        let header = container(
            row![
                text("Wren").size(18).color(theme::color(colors.accent)),
                text("Editor spike · in memory").size(13),
                iced::widget::space::horizontal(),
                button("System").on_press(Message::ChooseTheme(None)),
                button("Dark").on_press(Message::ChooseTheme(Some(definitions::DARK))),
                button("Light").on_press(Message::ChooseTheme(Some(definitions::LIGHT))),
            ]
            .spacing(10)
            .align_y(iced::Center),
        )
        .padding([10, 16])
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(theme::color(colors.titlebar).into()),
            ..Default::default()
        });
        let mut body = column![
            header,
            container(
                text(
                    self.banner
                        .as_deref()
                        .unwrap_or("Editor evaluation only — this buffer is not saved to disk.")
                )
                .color(theme::color(colors.warning))
            )
            .padding([8, 16])
            .width(Fill),
        ];
        if self.find_open {
            body = body.push(
                container(
                    row![
                        text_input("Find literal text", &self.find_query)
                            .id("find-query")
                            .on_input(Message::FindChanged)
                            .on_submit(Message::FindNext),
                        text(format!(
                            "{} / {}",
                            self.editor.find_current().map_or(0, |n| n + 1),
                            self.editor.find_count()
                        )),
                        button("Previous").on_press(Message::FindPrevious),
                        button("Next").on_press(Message::FindNext),
                        button("Close").on_press(Message::FindClose),
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                )
                .padding([8, 16]),
            );
        }
        body = body.push(self.editor.view(self.definition).map(Message::Editor));
        let cursor = self.editor.cursor();
        body = body.push(
            container(
                row![
                    text(format!(
                        "Markdown · {} lines · {} words",
                        self.editor.line_count(),
                        self.words
                    )),
                    iced::widget::space::horizontal(),
                    text(format!(
                        "Ln {}, byte col {}",
                        cursor.position.line + 1,
                        cursor.position.column + 1
                    ))
                    .font(fonts::EDITOR),
                    text("Not saved · spike").color(theme::color(colors.warning)),
                ]
                .spacing(20),
            )
            .padding([8, 16])
            .width(Fill),
        );
        body.width(Fill).height(Fill).into()
    }

    #[cfg(feature = "smoke-test")]
    fn smoke_view(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        container(
            column![
                text("Wren").size(36).color(theme::color(colors.accent)),
                text("Renderer verification").size(24),
                text("Linux tooling and renderer verification.").size(16),
                text("The editor is exercised separately through the real application.")
                    .color(theme::color(colors.muted)),
                row![
                    button("System").on_press(Message::ChooseTheme(None)),
                    button("Dark").on_press(Message::ChooseTheme(Some(definitions::DARK))),
                    button("Light").on_press(Message::ChooseTheme(Some(definitions::LIGHT))),
                ]
                .spacing(8),
                text(self.definition.name).color(theme::color(colors.muted)),
            ]
            .spacing(16)
            .padding(32),
        )
        .center_x(Fill)
        .center_y(Fill)
        .into()
    }

    pub fn theme(&self) -> iced::Theme {
        self.toolkit_theme.clone()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            iced::system::theme_changes().map(Message::SystemTheme),
            iced::window::events().map(|(id, event)| Message::Window(id, event)),
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::Escape),
                _ => None,
            }),
        ];
        if self.session.is_some() {
            subscriptions.push(iced::time::every(Duration::from_millis(25)).map(|_| Message::Tick));
        }
        #[cfg(feature = "smoke-test")]
        let subscriptions = {
            let mut subscriptions = subscriptions;
            if let Some(endpoint) = &self.endpoint {
                subscriptions.push(endpoint.subscription().map(Message::Test));
                subscriptions.push(self.frames.subscription());
            }
            subscriptions
        };
        Subscription::batch(subscriptions)
    }

    fn set_theme(&mut self, definition: Theme) {
        self.definition = definition;
        self.toolkit_theme = theme::from_definition(definition);
    }
}

fn focus_editor() -> Task<Message> {
    iced::widget::operation::focus("note-editor")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_theme_stays_selected_until_system_mode_is_restored() {
        let (mut app, _) = App::new(Options::default());
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert_eq!(app.definition, definitions::LIGHT);
        let _ = app.update(Message::ChooseTheme(Some(definitions::DARK)));
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert_eq!(app.definition, definitions::DARK);
        let _ = app.update(Message::ChooseTheme(None));
        assert_eq!(app.definition, definitions::LIGHT);
    }

    #[test]
    fn large_fixture_is_five_thousand_lines_and_keeps_unicode() {
        let contents = spike_text(true);
        assert_eq!(contents.lines().count(), 5000);
        assert!(contents.contains("café 🐦"));
    }
}
