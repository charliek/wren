use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::theme::Mode;
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Element, Fill, Subscription, Task};
use wren_core::model::{GroupId, NoteId, SectionId};
use wren_core::ops::{Op, Session};
use wren_core::theme::{self as definitions, Theme};
use wren_editor::EditorView;

pub use crate::cli::Options;
use crate::{fonts, theme};

mod columns;
mod workspace;
mod workspace_view;

pub(crate) mod commands;
mod gestures;
mod navigation;
mod palette;
mod preferences;
mod workspaces;

use crate::panes::Widths;
use commands::{Command, Processed};
use navigation::Filters;
use palette::Palette;
use workspace::{Dialog, LoadOutcome, Loading};

#[cfg(feature = "smoke-test")]
mod test_requests;

pub struct App {
    definition: Theme,
    toolkit_theme: iced::Theme,
    system_mode: Mode,
    /// A theme forced by the smoke/test harness, bypassing settings.
    theme_override: Option<Theme>,
    /// The theme or font picker row shown live; see `sync_preview`.
    preview: preferences::Preview,
    preferences: preferences::Preferences,
    editor: EditorView,
    find_open: bool,
    find_query: String,
    words: usize,
    /// When `words` is next recounted, after an edit.
    words_due: Option<Instant>,
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
    conflict_copy: Option<(u64, wren_core::fs_store::RelPath)>,
    dialog: Option<Dialog>,
    quit_pending: bool,
    flush_on_load: bool,
    config: Option<wren_core::config::Config>,
    registry_warning: Option<String>,
    metadata_warning: Option<String>,
    banner_kind: workspace::WarningKind,
    registration_name: Option<String>,
    folder_serial: u64,
    folder_request: Option<u64>,
    folder_correlation: Option<u64>,
    action_serial: u64,
    last_processed: Option<Processed>,
    palette: Option<Palette>,
    /// Where the outline palette opened; see `sync_outline`.
    outline_origin: Option<palette::OutlineOrigin>,
    widths: Widths,
    column: columns::Column,
    /// Browser panes folded from the left (0–2) by keyboard moves.
    fold: usize,
    focus: bool,
    filters: Filters,
    viewport: iced::Size,
    spring_section: Option<SectionId>,
    gesture_generation: u64,
    drag_preview: Option<crate::drag::Preview>,
    resizing: Option<(crate::panes::Pane, f32)>,
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
    /// A test holds the session's timer, so autosave cannot write a draft
    /// before the test has armed a storage fault.
    #[cfg(feature = "smoke-test")]
    timer_held: bool,
    #[cfg(feature = "smoke-test")]
    frames: crate::test_capture::Frames,
    /// How long each recent `view` took to build, for the harness's idle
    /// and per-key ceilings. `view` takes `&self`, hence the cell.
    #[cfg(feature = "smoke-test")]
    view_micros: std::cell::RefCell<std::collections::VecDeque<u64>>,
}

#[derive(Debug, Clone)]
pub enum Message {
    SystemTheme(Mode),
    ChooseTheme(Option<Theme>),
    Editor(wren_editor::Message),
    Core(Op),
    Command(Command),
    PaletteOpen(palette::Mode),
    PaletteQuery(String),
    PaletteChoose(u64, palette::Key),
    PaletteClose,
    PaletteRevealed,
    Key(crate::keys::Input),
    /// A command key chosen from the native menu bar.
    #[cfg(target_os = "macos")]
    MenuCommand(String),
    FolderChosen(u64, Option<PathBuf>),
    Revealed {
        correlation: u64,
        generation: u64,
        result: Result<(), String>,
    },
    Drag(u64, crate::drag::DragEvent),
    Resize(u64, crate::panes::ResizeEvent),
    Tick,
    Loaded(u64, Arc<Mutex<Option<LoadOutcome>>>),
    Reconciled(wren_core::reconcile::ReconcileResult),
    DialogName(String),
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
    /// An editor command's shortcut, as the editor widget publishes it:
    /// `Some` only while the editor has the keyboard.
    #[cfg(feature = "smoke-test")]
    EditorShortcut(
        std::sync::Arc<crate::test_ipc::Request>,
        Option<wren_editor::Intent>,
    ),
}

/// How long the footer's word count may lag the text. Counting reads the
/// whole note, too slow in a debug build to repeat on every keystroke.
const WORD_COUNT_DELAY: Duration = Duration::from_millis(250);

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
        // Unit tests must never read or write the user's real configuration.
        if cfg!(test) {
            return Self::with_config(options, None, None);
        }
        let config = (!options.editor).then(wren_core::config::Config::load);
        Self::with_config(options, config, wren_core::settings::path())
    }

    fn with_config(
        options: Options,
        config: Option<wren_core::config::Config>,
        settings_path: Option<PathBuf>,
    ) -> (Self, Task<Message>) {
        #[cfg(feature = "smoke-test")]
        let theme_override = options.smoke.as_ref().map(|request| request.theme);
        #[cfg(not(feature = "smoke-test"))]
        let theme_override: Option<Theme> = None;
        let preferences = preferences::Preferences::load(settings_path);
        let definition = preferences.theme(Mode::None).0;
        let mut registry_warning = config
            .as_ref()
            .and_then(|config| config.warning().map(str::to_owned));
        let workspace = options.workspace.clone().or_else(|| {
            let config = config.as_ref()?;
            let active = config.active()?;
            match config.validate_registration(active) {
                Ok(path) => Some(path),
                Err(error) => {
                    registry_warning = Some(match registry_warning.take() {
                        Some(warning) => format!("{warning} {error}"),
                        None => error,
                    });
                    None
                }
            }
        });
        let contents = if options.editor {
            spike_text(options.large_spike)
        } else {
            String::new()
        };
        let words = contents.split_whitespace().count();
        let engine = preferences::backend_kind(preferences.settings.editor_engine);
        let mut app = Self {
            definition,
            toolkit_theme: theme::from_definition(definition),
            system_mode: Mode::None,
            theme_override,
            preview: preferences::Preview::default(),
            preferences,
            editor: EditorView::with_backend(engine, &contents),
            find_open: false,
            find_query: String::new(),
            words,
            words_due: None,
            edit_revision: 0,
            edit_micros: Vec::new(),
            window: None,
            session: None,
            workspace_mode: !options.editor,
            workspace_path: String::new(),
            loading: None,
            load_generation: 0,
            selected_section: None,
            selected_group: None,
            selected_note: None,
            editor_note: None,
            banner: None,
            conflict_copy: None,
            dialog: None,
            quit_pending: false,
            flush_on_load: false,
            config,
            registry_warning,
            metadata_warning: None,
            banner_kind: workspace::WarningKind::General,
            registration_name: None,
            folder_serial: 0,
            folder_request: None,
            folder_correlation: None,
            action_serial: 0,
            last_processed: None,
            palette: None,
            outline_origin: None,
            widths: Widths::default(),
            column: columns::Column::Editor,
            fold: 0,
            focus: false,
            filters: Filters::default(),
            viewport: iced::Size::new(1280.0, 800.0),
            spring_section: None,
            gesture_generation: 1,
            drag_preview: None,
            resizing: None,
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
            smoke: options.smoke.map(|request| *request),
            #[cfg(feature = "smoke-test")]
            endpoint: options.endpoint,
            #[cfg(feature = "smoke-test")]
            capture_pending: false,
            #[cfg(feature = "smoke-test")]
            timer_held: false,
            #[cfg(feature = "smoke-test")]
            frames: crate::test_capture::Frames::default(),
            #[cfg(feature = "smoke-test")]
            view_micros: Default::default(),
        };
        app.apply_preferences();
        if let Some(warning) = app.startup_settings_warning() {
            app.banner = Some(warning);
        }
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
        let task = self.handle(message);
        // Every way a picker row gets highlighted, or the picker closes,
        // arrives as a message, so the preview follows from here.
        let preview = self.sync_preview();
        Task::batch([task, preview])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
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
            #[cfg(feature = "smoke-test")]
            Message::EditorShortcut(request, intent) => {
                let task = intent.map_or_else(Task::none, |intent| {
                    self.update(Message::Editor(wren_editor::Message::Intent(intent)))
                });
                request.respond(Ok(self.navigation_snapshot()));
                return task;
            }
            Message::SystemTheme(mode) => {
                self.system_mode = mode;
                self.apply_theme();
            }
            Message::ChooseTheme(definition) => {
                self.theme_override = definition;
                self.apply_theme();
            }
            Message::Editor(message) => return self.edit(message),
            Message::Core(op) => return self.process_core(op).task,
            Message::Command(command) => return self.dispatch(command).task,
            Message::PaletteOpen(mode) => return self.open_palette(mode).task,
            Message::PaletteQuery(query) => return self.palette_query(query).task,
            Message::PaletteChoose(generation, key) => {
                return self.activate_palette_key(generation, key).task
            }
            Message::PaletteClose => return self.close_palette().task,
            Message::PaletteRevealed => {}
            Message::FolderChosen(tag, path) => return self.folder_chosen(tag, path).task,
            Message::Revealed {
                correlation,
                generation,
                result,
            } => {
                if generation != self.workspace_generation() {
                    return Task::none();
                }
                let warning = result
                    .err()
                    .map(|error| format!("Could not reveal item: {error}"));
                if let Some(warning) = &warning {
                    self.banner = Some(warning.clone());
                    self.banner_kind = workspace::WarningKind::General;
                }
                self.last_processed = Some(Processed {
                    correlation,
                    disposition: if warning.is_some() {
                        commands::Disposition::Blocked
                    } else {
                        commands::Disposition::Completed
                    },
                    warning,
                });
            }
            Message::Drag(generation, event) => return self.drag_event(generation, event),
            Message::Resize(generation, event) => return self.resize_event(generation, event),
            Message::Key(input) => return self.key_input(input),
            #[cfg(target_os = "macos")]
            Message::MenuCommand(key) => return self.menu_command(&key),
            Message::Tick => {
                self.count_words(Instant::now());
                #[cfg(feature = "smoke-test")]
                if self.timer_held {
                    return Task::none();
                }
                if self.session.is_some() {
                    let task = self.process_core(Op::Tick).task;
                    return self.spawn_reconcile(task);
                }
            }
            Message::Reconciled(result) => return self.reconciled(result),
            Message::Loaded(generation, outcome) => {
                return self.workspace_loaded(generation, outcome)
            }
            Message::DialogName(name) => {
                if let Some(dialog) = &mut self.dialog {
                    dialog.name = name;
                }
            }
            Message::SubmitDialog => return self.submit_dialog(false),
            Message::ConfirmTrash => return self.submit_dialog(true),
            Message::CancelDialog => {
                self.dialog = None;
                return focus_editor();
            }
            Message::DismissBanner => {
                self.conflict_copy = None;
                if self.banner.is_some() {
                    if self.banner == self.metadata_warning {
                        self.metadata_warning = None;
                    }
                    self.banner = self.metadata_warning.clone();
                    self.banner_kind = workspace::WarningKind::Metadata;
                } else {
                    self.registry_warning = None;
                }
            }
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
                self.cancel_gestures();
                // Esc closes the topmost overlay and keeps the active column;
                // with none open, it returns the keyboard to the editor.
                if self.palette.take().is_some() || self.dialog.take().is_some() {
                    return self.column_focus();
                }
                self.find_open = false;
                return self.return_to_editor();
            }
            Message::Window(id, event) => match event {
                iced::window::Event::Opened { size, .. } => {
                    self.viewport = size;
                    self.window = Some(id);
                    #[cfg(target_os = "macos")]
                    {
                        crate::macos_menu::install(&self.preferences.keymap);
                        // A missing framework (a bare binary, a bundle built
                        // without Sparkle) only leaves the menu item disabled.
                        let _ = wren_sparkle::start();
                        crate::macos_menu::sync_updates();
                    }
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
                iced::window::Event::Resized(size) => {
                    self.viewport = size;
                }
                iced::window::Event::CloseRequested => {
                    return self.request_quit().unwrap_or_else(|_| Task::none());
                }
                iced::window::Event::Unfocused => {
                    let _ = self.window_unfocused();
                }
                iced::window::Event::Focused => {
                    let _ = self.reload_settings(false);
                    #[cfg(target_os = "macos")]
                    crate::macos_menu::sync_updates();
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

    fn edit(&mut self, message: wren_editor::Message) -> Task<Message> {
        if self.ensure_editor_ready().is_err() || self.dialog.is_some() || self.palette.is_some() {
            return Task::none();
        }
        let started = Instant::now();
        let changed = match message {
            wren_editor::Message::Action(action) => {
                // Scrolling reaches an unfocused editor; anything else means
                // the editor has the keyboard.
                if !matches!(action, iced::widget::text_editor::Action::Scroll { .. }) {
                    self.column = columns::Column::Editor;
                }
                self.editor.perform(action)
            }
            wren_editor::Message::Intent(intent) => {
                self.column = columns::Column::Editor;
                self.editor.intent(intent)
            }
            wren_editor::Message::Undo => self.editor.undo(),
            wren_editor::Message::Redo => self.editor.redo(),
            wren_editor::Message::Find => {
                self.editor.break_group();
                self.find_open = true;
                return iced::widget::operation::focus("find-query");
            }
        };
        if changed {
            self.text_changed(started);
        }
        Task::none()
    }

    /// What follows every change to the editor's text: the session draft,
    /// which autosave saves, the edit revision, and a word recount on a
    /// later tick.
    fn text_changed(&mut self, started: Instant) {
        if let Some(note) = self.selected_note {
            let _ = self.apply_core(Op::EditDraft {
                note,
                contents: self.editor.text(),
            });
        }
        self.edit_revision += 1;
        self.words_due.get_or_insert(started + WORD_COUNT_DELAY);
        if self.edit_micros.len() == 256 {
            self.edit_micros.remove(0);
        }
        self.edit_micros
            .push(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
    }

    /// Recount the words once an edit's delay has passed.
    fn count_words(&mut self, now: Instant) {
        if self.words_due.is_some_and(|due| now >= due) {
            self.set_words(&self.editor.text());
        }
    }

    fn set_words(&mut self, text: &str) {
        self.words = text.split_whitespace().count();
        self.words_due = None;
    }

    #[cfg(feature = "smoke-test")]
    fn capture_frame(
        &self,
        request: Arc<crate::test_ipc::Request>,
        path: PathBuf,
    ) -> Task<Message> {
        let Some(window) = self.window else {
            return Task::done(Message::CaptureFailed(request, "window is not open".into()));
        };
        Task::future(self.frames.next(window)).then(move |result| match result {
            Ok(()) => {
                let request = Arc::clone(&request);
                let path = path.clone();
                iced::window::screenshot(window).map(move |capture| {
                    Message::RpcCaptured(Arc::clone(&request), path.clone(), capture)
                })
            }
            Err(error) => Task::done(Message::CaptureFailed(Arc::clone(&request), error)),
        })
    }

    pub fn view(&self) -> Element<'_, Message> {
        #[cfg(feature = "smoke-test")]
        let started = Instant::now();
        let view = crate::keys::alt_filter(self.contents()).into();
        #[cfg(feature = "smoke-test")]
        {
            let mut times = self.view_micros.borrow_mut();
            if times.len() == 256 {
                times.pop_front();
            }
            times.push_back(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
        }
        view
    }

    fn contents(&self) -> Element<'_, Message> {
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
        let (style, palette) = theme::editor_colors(self.definition);
        body = body.push(
            self.editor
                .view(style, palette, self.editor_max_width())
                .map(Message::Editor),
        );
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
        self.palette_view(body.width(Fill).height(Fill).into())
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
            iced::event::listen_with(|event, status, _| match event {
                iced::Event::Keyboard(event) => crate::keys::input(&event)
                    .filter(|input| crate::keys::reaches_app(input, status))
                    .map(|input| Message::Key(crate::keys::with_status(input, status))),
                _ => None,
            }),
        ];
        if self.session.is_some() || self.words_due.is_some() {
            subscriptions.push(iced::time::every(Duration::from_millis(25)).map(|_| Message::Tick));
        }
        #[cfg(target_os = "macos")]
        subscriptions.push(crate::macos_menu::events().map(Message::MenuCommand));
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
}

fn focus_editor() -> Task<Message> {
    iced::widget::operation::focus("note-editor")
}

#[cfg(test)]
mod navigation_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_theme_stays_selected_until_system_mode_is_restored() {
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert_eq!(app.definition, definitions::LIGHT);
        let _ = app.update(Message::ChooseTheme(Some(definitions::DARK)));
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert_eq!(app.definition, definitions::DARK);
        let _ = app.update(Message::ChooseTheme(None));
        assert_eq!(app.definition, definitions::LIGHT);
    }

    #[test]
    fn the_word_count_follows_edits_on_a_later_tick() {
        use iced::widget::text_editor::{Action, Edit, Motion};

        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        let words = app.words;
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Move(
            Motion::DocumentEnd,
        ))));
        assert_eq!(app.words_due, None);
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
            Edit::Paste(Arc::new(" two words".into())),
        ))));
        let due = app.words_due.expect("a recount is due");
        assert_eq!(app.words, words);
        // Later edits keep the first deadline, so typing cannot starve it.
        let _ = app.update(Message::Editor(wren_editor::Message::Intent(
            wren_editor::Intent::Enter,
        )));
        assert_eq!(app.words_due, Some(due));
        app.count_words(due - Duration::from_millis(1));
        assert_eq!(app.words, words);
        app.count_words(due);
        assert_eq!(app.words, words + 2);
        assert_eq!(app.words_due, None);
        assert_eq!(app.edit_revision, 2);
    }

    #[test]
    fn large_fixture_is_five_thousand_lines_and_keeps_unicode() {
        let contents = spike_text(true);
        assert_eq!(contents.lines().count(), 5000);
        assert!(contents.contains("café 🐦"));
    }
}
