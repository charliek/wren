//! The board surface: which of the editor and the board viewer shows the
//! open note. `sync_editor` decides it, when another note opens or the
//! Session's text is not what the surface last took, and everything else
//! asks [`App::surface`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use iced::Task;
use wren_board::{Access, Palette, ParseError};
use wren_canvas::{BoardView, Effect, Input, SavedView};
use wren_core::kind::NoteKind;
use wren_core::model::NoteId;
use wren_core::ops::Op;

use super::columns::Column;
use super::commands::{Disposition, Gateway};
use super::workspace::WarningKind;
use super::{App, Message};
use crate::board_export::{self, Format};
use crate::fonts;

/// What shows the open note. Only this surface may write its draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Surface {
    /// A Markdown note, or no note at all, in the editor.
    Text,
    /// A board in the viewer. The editor is empty.
    Board,
    /// A board whose text does not parse, as text in the editor. It is a
    /// board again when it is next opened, or when its text changes
    /// underneath the editor into one that parses; never while it is typed in.
    Raw,
}

/// Where each board opened in this run was last looked at from. It is
/// never stored on disk.
pub(super) type Viewports = HashMap<NoteId, SavedView>;

/// A view of the board `text` holds, looked at from where `note` was left,
/// or fitted the first time it opens in this run, leaving `covered` pixels
/// along its top and its bottom to the host's chrome.
pub(super) fn open(
    text: &str,
    note: NoteId,
    palette: Palette,
    viewports: &Viewports,
    covered: (f32, f32),
) -> Result<BoardView, ParseError> {
    // Labels are measured when the view opens, which can be before iced
    // has loaded the bundled faces for a window.
    fonts::load();
    let mut view = BoardView::open(text, palette, fonts::UI)?;
    view.set_covered(covered.0, covered.1);
    // A trackpad pinch (macOS) is recorded by the vendored iced_winit for
    // the window's next frame, which the widget reads it in.
    view.set_pinches(iced_winit::pinch::take);
    match viewports.get(&note) {
        Some(saved) => view.return_to(*saved),
        None => view.fit(),
    }
    Ok(view)
}

/// Put away the view of `note`, remembering where it was looked at from.
/// A view still waiting to be fitted was never looked at.
pub(super) fn close(view: &BoardView, note: NoteId, viewports: &mut Viewports) {
    if !view.fit_pending() {
        viewports.insert(note, view.saved_view());
    }
}

impl App {
    pub(super) fn surface(&self) -> Surface {
        if self.board.is_some() {
            Surface::Board
        } else if self.board_error.is_some() {
            Surface::Raw
        } else {
            Surface::Text
        }
    }

    /// The colours the current theme gives a board.
    pub(super) fn board_palette(&self) -> Palette {
        crate::theme::board_palette(self.definition)
    }

    /// How the board widget presents a board in this window.
    pub(super) fn board_style(&self) -> wren_canvas::Style {
        wren_canvas::Style {
            #[cfg(feature = "smoke-test")]
            labels: self.board_perf.labels,
            #[cfg(not(feature = "smoke-test"))]
            labels: None,
            scale_factor: self.scale_factor,
            // The dock lights the armed tool's button instead.
            tool_chip: self.board_dock().is_none(),
        }
    }

    /// A message from the widget showing the open board: the twin of
    /// [`App::edit`], and with `dispatch` the only way to the board's draft
    /// (§3.4 rule 1). A key the board layer binds goes on to `dispatch`, as
    /// a palette row or a menu item does.
    ///
    /// Input other than the wheel or a pinch means the board has the
    /// keyboard, as an edit means the editor has it; the wheel and a pinch
    /// reach a board that does not, a cancel or a focus change only says the window or the widget
    /// gained or lost it, and the pointer passing over with no button
    /// down, leaving, or resting until a tick, is nobody acting on the
    /// board. Under a dialog or a palette, and while a workspace loads,
    /// the board takes only what looks at it or ends what was in flight. Input meant for the board as it was before a
    /// reload, or for another board, is dropped.
    pub(super) fn board_edit(&mut self, message: wren_canvas::Message) -> Task<Message> {
        use wren_canvas::Message as Sent;
        let held = self.dialog.is_some() || self.palette.is_some() || self.loading.is_some();
        let Some(view) = &mut self.board else {
            return Task::none();
        };
        match &message {
            // The bars are laid out again for the new size, and may no
            // longer hold the button a menu hangs from: such a menu is
            // forgotten, so it neither comes back with its button nor
            // takes the next Escape. One whose button is still there
            // stays, as a canvas settles into its size over a few frames.
            Sent::Resized { .. } => {
                let update = view.update(message, Instant::now());
                let task = self.board_did(update).task;
                if self.board_dock().is_none_or(|dock| dock.menu.is_none()) {
                    self.dock_menu = None;
                }
                if self.board_style_bar().is_none_or(|bar| bar.menu.is_none()) {
                    self.style_menu = None;
                }
                return task;
            }
            Sent::Command(epoch, command) => {
                if held || *epoch != view.epoch() {
                    return Task::none();
                }
                self.close_board_menus();
                let id = super::commands::CommandId::Board(*command);
                return self.dispatch(super::commands::Command::Run(id)).task;
            }
            // Read now, after the keys before it, and carried out as what
            // it reads as, so a bound key still goes through `dispatch`.
            Sent::Key(epoch, press) => {
                if held || *epoch != view.epoch() {
                    return Task::none();
                }
                let menu = self.dock_menu.is_some() || self.style_menu.is_some();
                (self.dock_menu, self.style_menu, self.style_hover) = (None, None, None);
                // Escape closes the topmost thing only: with a bar's menu
                // open that is the menu, and the armed tool, the selection
                // and the board are as they were.
                let escape = iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape);
                if menu && press.key == escape {
                    return Task::none();
                }
                let read = view.press(press).unwrap_or_default();
                let tasks: Vec<Task<Message>> = read
                    .into_iter()
                    .map(|message| self.board_edit(message))
                    .collect();
                return Task::batch(tasks);
            }
            Sent::Input(epoch, _) | Sent::Label(epoch, _) | Sent::Pasted(epoch, _)
                if *epoch != view.epoch() =>
            {
                return Task::none();
            }
            Sent::Input(_, input) => {
                let passive = matches!(
                    input,
                    Input::Wheel { .. }
                        | Input::Pinch { .. }
                        | Input::Cancel
                        | Input::Focus(_)
                        | Input::Leave
                        | Input::Tick
                );
                let ends = matches!(
                    input,
                    Input::Release { .. } | Input::Modifiers(_) | Input::Space { held: false }
                );
                if held && !(passive || ends) {
                    return Task::none();
                }
                let hovers = matches!(input, Input::Move { .. }) && !view.machine().dragging();
                if !held && !passive && !hovers {
                    self.column = Column::Editor;
                }
                // A press on the canvas closes the bars' menus, as a key
                // the board reads does. So does the board losing the
                // keyboard, which settles it: a control the pointer still
                // rests on must not bring its preview back.
                if matches!(
                    input,
                    Input::Press { .. } | Input::Key { .. } | Input::Char(_) | Input::Focus(false)
                ) {
                    (self.dock_menu, self.style_menu, self.style_hover) = (None, None, None);
                }
            }
            Sent::Label(..) | Sent::Pasted(..) => {
                if held || !view.writable() {
                    return Task::none();
                }
                self.column = Column::Editor;
            }
        }
        let update = view.update(message, Instant::now());
        self.board_did(update).task
    }

    /// Close the dock's and the style bar's menus and forget the control
    /// the pointer was over, as a press or a key on the board does, or a
    /// command. Returns whether a menu was open.
    pub(super) fn close_board_menus(&mut self) -> bool {
        self.style_hover = None;
        let dock = self.dock_menu.take().is_some();
        let style = self.style_menu.take().is_some();
        dock || style
    }

    /// Run a board command on the open board, as its key would. Only
    /// `run_command` calls it, so every command comes through `dispatch`.
    pub(super) fn board_command(&mut self, command: wren_canvas::Command) -> BoardDid {
        let Some(view) = &mut self.board else {
            return BoardDid::default();
        };
        (self.dock_menu, self.style_menu, self.style_hover) = (None, None, None);
        let update = view.command(command, Instant::now());
        self.board_did(update)
    }

    /// Carry out what the board's view answered: its draft when the board
    /// changed, then what it asks of the app.
    pub(super) fn board_did(&mut self, update: wren_canvas::Update) -> BoardDid {
        let mut did = BoardDid::default();
        // The dock's folded shapes button shows the shape last armed.
        if let Some(wren_canvas::Tool::Shape(shape)) =
            self.board.as_ref().map(|view| view.machine().tool())
        {
            self.dock_shape = shape;
        }
        if update.changed {
            did.failed = self.board_changed().err();
        }
        let mut tasks = Vec::new();
        for effect in update.effects {
            match effect {
                Effect::Notice(notice) => {
                    self.banner = Some(notice.clone());
                    self.banner_kind = WarningKind::General;
                    did.notice = Some(notice);
                }
                Effect::ClipboardWrite(text) => tasks.push(self.clipboard_write(text)),
                // The text comes back stamped for the board that asked, so
                // it is dropped if the board is another by then. A
                // clipboard with no text pastes nothing.
                Effect::ClipboardRead => {
                    if let Some(epoch) = self.board.as_ref().map(BoardView::epoch) {
                        tasks.push(self.clipboard_read().map(move |text| {
                            let text = text.unwrap_or_default();
                            Message::Board(wren_canvas::Message::Pasted(epoch, text))
                        }));
                    }
                }
                // A right-click: the actions palette for the selection it
                // made, opened as `board.actions` opens it (§5.17 rule 2).
                Effect::OpenActions => {
                    let id = super::commands::CommandId::Board(wren_canvas::Command::Actions);
                    let opened = self.dispatch(super::commands::Command::Run(id));
                    tasks.push(opened.task);
                }
                // The view moved itself, and the jump palette is the find
                // chain's.
                Effect::Look(_) | Effect::OpenJump => {}
            }
        }
        did.task = Task::batch(tasks);
        did
    }

    /// Put `text` on the clipboard: the system's, or under the harness the
    /// app's own when it asked for that.
    pub(super) fn clipboard_write(&mut self, text: String) -> Task<Message> {
        #[cfg(feature = "smoke-test")]
        if let Some(own) = &mut self.test_clipboard {
            *own = text;
            return Task::none();
        }
        iced::clipboard::write(text)
    }

    /// Read the clipboard [`App::clipboard_write`] writes.
    pub(super) fn clipboard_read(&self) -> Task<Option<String>> {
        #[cfg(feature = "smoke-test")]
        if let Some(own) = &self.test_clipboard {
            return Task::done(Some(own.clone()));
        }
        iced::clipboard::read()
    }

    /// The board changed: its draft goes to the Session, which autosave
    /// saves. This is the only place a board sends `Op::EditDraft` (§3.4
    /// rule 1). The view picks the text by the pristine rule and takes it
    /// as synced before the op runs, so the `sync_editor` that follows
    /// sees its own draft and reloads nothing.
    fn board_changed(&mut self) -> Result<(), String> {
        let (Some(view), Some(note)) = (&mut self.board, self.editor_note) else {
            return Ok(());
        };
        #[cfg(feature = "smoke-test")]
        let started = Instant::now();
        let contents = view.draft();
        #[cfg(feature = "smoke-test")]
        let written = Instant::now();
        self.board_revision += 1;
        let sent = self.apply_core(Op::EditDraft { note, contents }).map(drop);
        #[cfg(feature = "smoke-test")]
        self.board_perf.drafted(started, written, Instant::now());
        sent
    }

    /// Before anything that is not passive (§3.4 rule 3): the open label
    /// is committed and a pointer gesture cancelled, and what that changed
    /// is drafted, so the operation flushes it with the rest. An error is
    /// a draft that did not reach the Session, which blocks the operation.
    pub(super) fn settle_board(&mut self) -> Result<(), String> {
        let Some(view) = &mut self.board else {
            return Ok(());
        };
        let update = view.settle(Instant::now());
        self.board_did(update).failed.map_or(Ok(()), Err)
    }

    /// Something else takes the keyboard: a palette or a dialog opens, or
    /// the window loses its focus (§5.15 rule 4). The board settles and
    /// its armed tool is put away.
    pub(super) fn put_board_away(&mut self) -> Result<(), String> {
        self.close_board_menus();
        self.settle_board()?;
        let Some(view) = &mut self.board else {
            return Ok(());
        };
        let select = wren_canvas::Command::Tool(wren_canvas::Tool::Select);
        let update = view.command(select, Instant::now());
        self.board_did(update).failed.map_or(Ok(()), Err)
    }
}

/// What the app did with a board view's answer.
#[derive(Default)]
pub(super) struct BoardDid {
    /// What is left to run.
    pub task: Task<Message>,
    /// Why the board did not do what was asked, which is the banner now.
    pub notice: Option<String>,
    /// Why the board's draft did not reach the Session.
    pub failed: Option<String>,
}

/// An export of the open board, its document (the SVG, or the draw.io
/// file) made when it was asked for and kept until its save dialog says
/// where it goes; then rendered and written on a thread of its own, so the
/// window keeps answering however long a large PNG takes. One export is
/// under way at a time.
pub(super) struct Export {
    /// The correlation of the action that asked for it, which its
    /// completion reports as.
    tag: u64,
    format: Format,
    stage: Stage,
}

enum Stage {
    /// The save dialog is open; the document as drawn when the command ran.
    Choosing(String),
    Writing(Writing),
}

/// An export being rendered and written by its worker.
struct Writing {
    path: PathBuf,
    /// The worker, which returns the bytes written or the warning to show.
    worker: std::thread::JoinHandle<Result<u64, String>>,
    /// The `board.export` request that asked for it, answered instead of a
    /// banner and a completion.
    #[cfg(feature = "smoke-test")]
    reply: Option<std::sync::Arc<crate::test_ipc::Request>>,
}

/// Render `document` in `format` and write it to `path`. Runs on the
/// export's worker. Returns the bytes written, or the warning to show.
fn render_and_write(path: &Path, format: Format, document: &str) -> Result<u64, String> {
    let failed = |error: &dyn std::fmt::Display| {
        format!("Could not export the board to {}: {error}", path.display())
    };
    let png;
    let contents = match format {
        Format::Svg | Format::Drawio => document.as_bytes(),
        Format::Png => {
            png = board_export::png(document, board_export::PNG_SCALE)
                .map_err(|error| failed(&error))?;
            &png
        }
    };
    board_export::write(path, contents).map_err(|error| failed(&error))
}

/// The board as an SVG document, in the colours it is drawn in now, on a
/// background of the canvas colour.
pub(super) fn svg_document(view: &BoardView) -> String {
    board_export::svg(view, board_export::Background::Theme)
}

/// What an export in `format` of the board `name` writes, made now: the
/// SVG as drawn, which a PNG is rendered from, or the draw.io file, in
/// [`board_export::DRAWIO_THEME`]'s colours.
fn export_document(view: &BoardView, format: Format, name: &str) -> String {
    match format {
        Format::Svg | Format::Png => svg_document(view),
        Format::Drawio => board_export::drawio(view, board_export::DRAWIO_THEME, name),
    }
}

/// Why an export in `format` may not be written to `path`, if it may not,
/// with the open workspace at `workspace`. An export never replaces a note,
/// here or in another workspace, and inside the open workspace it is only
/// ever a file of its format's extension, so the state Wren keeps there is
/// never replaced either. Those files are written through the Session,
/// which keeps a copy of what a write replaces; an export does not.
pub(super) fn export_refusal(
    path: &Path,
    workspace: Option<&Path>,
    format: Format,
) -> Option<String> {
    let extension = format.extension();
    let name = path.file_name()?;
    if NoteKind::from_name(name).is_some() {
        return Some(format!(
            "Did not export the board over {}: it is a note. Choose a name ending in .{extension}",
            path.display()
        ));
    }
    let own = path
        .extension()
        .is_some_and(|found| found.eq_ignore_ascii_case(extension));
    let inside = || {
        let (Some(parent), Some(root)) = (path.parent(), workspace) else {
            return false;
        };
        let (Ok(parent), Ok(root)) = (parent.canonicalize(), root.canonicalize()) else {
            // A folder that does not exist yet fails the write.
            return false;
        };
        parent.starts_with(root)
    };
    (!own && inside()).then(|| {
        format!(
            "Did not export the board to {}: inside the workspace an export must end in .{extension}",
            path.display()
        )
    })
}

impl App {
    /// `note.export_svg`, `note.export_png` and `note.export_drawio`: make
    /// the open board's document now, as it is drawn, and ask where to save
    /// it.
    pub(super) fn export_board(&mut self, format: Format) -> Gateway {
        let Some(view) = &self.board else {
            return self.blocked("The open note is not shown as a board");
        };
        if let Some(busy) = self.export_busy() {
            return self.blocked(busy);
        }
        let name = self.export_name();
        let document = export_document(view, format, &name);
        self.palette = None;
        let tag = self.action_serial.wrapping_add(1);
        self.board_export = Some(Export {
            tag,
            format,
            stage: Stage::Choosing(document),
        });
        let extension = format.extension();
        self.finish_action(
            Disposition::AsyncPending,
            None,
            Task::perform(
                async move {
                    rfd::AsyncFileDialog::new()
                        .set_title(format!("Export board as {}", format.name()))
                        .set_file_name(format!("{name}.{extension}"))
                        .add_filter(format.kind(), &[extension])
                        .save_file()
                        .await
                        .map(|file| file.path().to_path_buf())
                },
                move |path| Message::BoardExportChosen(tag, path),
            ),
        )
    }

    /// The open board's title, which names its export.
    fn export_name(&self) -> String {
        self.selected_note
            .and_then(|note| self.model()?.note(note))
            .map_or_else(|| "Board".to_owned(), |note| note.title.clone())
    }

    /// Why another export cannot start now, if one is under way.
    fn export_busy(&self) -> Option<String> {
        Some(match &self.board_export.as_ref()?.stage {
            Stage::Choosing(_) => "A board export is already waiting for its file".to_owned(),
            Stage::Writing(writing) => format!(
                "A board export to {} is still being written",
                writing.path.display()
            ),
        })
    }

    /// The save dialog of the export with `tag` closed, on `path` or on
    /// nothing. With a path, the export is rendered and written on its
    /// worker and completes when [`App::board_export_written`] runs; a
    /// refused path is a banner and writes nothing.
    pub(super) fn board_export_chosen(&mut self, tag: u64, path: Option<PathBuf>) -> Gateway {
        let Some(Export {
            format,
            stage: Stage::Choosing(document),
            ..
        }) = self
            .board_export
            .take_if(|export| export.tag == tag && matches!(export.stage, Stage::Choosing(_)))
        else {
            return self.blocked("Ignored a stale board export");
        };
        let mut outcome = match path {
            None => self.finish_action(Disposition::Completed, None, Task::none()),
            Some(path) => match self.start_export(tag, format, path, document) {
                Ok(task) => self.finish_action(Disposition::AsyncPending, None, task),
                Err(warning) => {
                    self.banner = Some(warning.clone());
                    self.banner_kind = WarningKind::General;
                    self.blocked(warning)
                }
            },
        };
        outcome.processed.correlation = tag;
        self.last_processed = Some(outcome.processed.clone());
        outcome
    }

    /// Start the worker that renders `document` in `format` and writes it
    /// to `path`, unless [`export_refusal`] says no, and keep it as the
    /// export with `tag`. The task reports [`Message::BoardExportWritten`]
    /// once the worker has finished.
    fn start_export(
        &mut self,
        tag: u64,
        format: Format,
        path: PathBuf,
        document: String,
    ) -> Result<Task<Message>, String> {
        let workspace = self.model().map(|model| model.root.as_path());
        if let Some(refusal) = export_refusal(&path, workspace, format) {
            return Err(refusal);
        }
        let (sender, receiver) = iced::futures::channel::oneshot::channel::<()>();
        let target = path.clone();
        let worker = std::thread::Builder::new()
            .name("wren-export".into())
            .spawn(move || {
                let written = render_and_write(&target, format, &document);
                let _ = sender.send(());
                written
            })
            .map_err(|error| {
                format!("Could not export the board to {}: {error}", path.display())
            })?;
        self.board_export = Some(Export {
            tag,
            format,
            stage: Stage::Writing(Writing {
                path,
                worker,
                #[cfg(feature = "smoke-test")]
                reply: None,
            }),
        });
        Ok(Task::perform(
            async move {
                // A dropped sender, a worker that panicked, still reports.
                let _ = receiver.await;
            },
            move |()| Message::BoardExportWritten(tag),
        ))
    }

    /// The worker of the export with `tag` has finished: the export
    /// completes, or its warning is the banner. A quit asked for while it
    /// was being written runs now.
    pub(super) fn board_export_written(&mut self, tag: u64) -> Task<Message> {
        let Some(Export {
            stage: Stage::Writing(writing),
            ..
        }) = self
            .board_export
            .take_if(|export| export.tag == tag && matches!(export.stage, Stage::Writing(_)))
        else {
            return Task::none();
        };
        let written = writing.worker.join().unwrap_or_else(|_| {
            Err(format!(
                "Could not export the board to {}: the export stopped unexpectedly",
                writing.path.display()
            ))
        });
        #[cfg(feature = "smoke-test")]
        if let Some(request) = writing.reply {
            request.respond(
                written.map(|bytes| serde_json::json!({"path": writing.path, "bytes": bytes})),
            );
            return self.quit_after_export();
        }
        let mut outcome = match written {
            Ok(_) => self.finish_action(Disposition::Completed, None, Task::none()),
            Err(warning) => {
                self.banner = Some(warning.clone());
                self.banner_kind = WarningKind::General;
                self.blocked(warning)
            }
        };
        outcome.processed.correlation = tag;
        self.last_processed = Some(outcome.processed);
        self.quit_after_export()
    }

    /// The quit that waited for an export, if one did: it flushes again,
    /// since the window took edits while the export was written.
    fn quit_after_export(&mut self) -> Task<Message> {
        if std::mem::take(&mut self.quit_after_export) {
            return self.request_quit().unwrap_or_else(|_| Task::none());
        }
        Task::none()
    }

    /// Whether an export's worker is rendering or writing it now.
    pub(super) fn export_writing(&self) -> bool {
        self.board_export
            .as_ref()
            .is_some_and(|export| matches!(export.stage, Stage::Writing(_)))
    }

    /// `board.export`: write the open board to `path` in `format` as the
    /// command does once its dialog closes, on the same worker and under
    /// the same one-export rule, answering `request` when it is written.
    #[cfg(feature = "smoke-test")]
    pub(super) fn request_board_export(
        &mut self,
        path: PathBuf,
        format: Format,
        request: &std::sync::Arc<crate::test_ipc::Request>,
    ) -> Result<Task<Message>, String> {
        let view = self
            .board
            .as_ref()
            .ok_or("The open note is not shown as a board")?;
        if let Some(busy) = self.export_busy() {
            return Err(busy);
        }
        let document = export_document(view, format, &self.export_name());
        let tag = self.action_serial.wrapping_add(1);
        let task = self.start_export(tag, format, path, document)?;
        if let Some(Export {
            stage: Stage::Writing(writing),
            ..
        }) = &mut self.board_export
        {
            writing.reply = Some(std::sync::Arc::clone(request));
        }
        Ok(task)
    }

    /// Wait for the export being written, as its worker's report would.
    #[cfg(all(test, feature = "smoke-test"))]
    pub(super) fn wait_for_export(&mut self) -> Task<Message> {
        match &self.board_export {
            Some(export) if matches!(export.stage, Stage::Writing(_)) => {
                self.board_export_written(export.tag)
            }
            _ => Task::none(),
        }
    }

    /// What `board.get` says of an export under way: `waiting` for its
    /// dialog, `writing` on its worker, or nothing.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn export_state(&self) -> Option<&'static str> {
        self.board_export.as_ref().map(|export| match export.stage {
            Stage::Choosing(_) => "waiting",
            Stage::Writing(_) => "writing",
        })
    }
}

/// What the footer says of a board: its size, then what is worth a look.
pub(super) struct BoardFooter {
    /// The counts of nodes and connectors.
    pub stats: String,
    /// What is wrong with the board, or why it cannot change.
    pub flags: Vec<String>,
    /// How far in it is zoomed.
    pub zoom: String,
}

impl BoardFooter {
    pub fn of(view: &BoardView) -> Self {
        let count =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let board = view.board();
        let mut flags = Vec::new();
        if !view.issues().is_empty() {
            flags.push(count(view.issues().len(), "issue", "issues"));
        }
        // Why, the schema first when both apply (§4.10).
        match view.access() {
            Access::ReadOnly(wren_board::Reason::Schema(schema)) => {
                flags.push(format!("Read-only: schema {schema}"));
            }
            Access::ReadOnly(wren_board::Reason::Requires(features)) => {
                flags.push(format!("Read-only: needs {}", features.join(", ")));
            }
            Access::Editable => {}
        }
        Self {
            stats: format!(
                "Board · {} · {}",
                count(board.nodes.len(), "node", "nodes"),
                count(board.edges.len(), "connector", "connectors")
            ),
            flags,
            zoom: format!("{:.0}%", view.viewport().zoom * 100.0),
        }
    }
}

/// The banner over a board shown as raw text: where its text stops being
/// a board, and that it is one again once that is fixed.
pub(super) fn raw_banner(error: &ParseError) -> String {
    format!(
        "This board is shown as text because line {}, column {} is not valid: {}. It opens as a board again once the text is valid.",
        error.line,
        error.column,
        error.message.trim_end().trim_end_matches('.')
    )
}

#[cfg(test)]
pub(super) mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use iced::widget::text_editor::{Action, Edit, Motion};
    use wren_core::kind::NoteKind;
    use wren_core::ops::{Op, Session};
    use wren_core::theme as definitions;

    use super::super::{Message, Options};
    use super::*;
    use wren_canvas::Viewport;

    pub const FLOW: &str = r#"schema = 1

[node.pack]
text = "Pack"
kind = "shape"
at = [0, 200]
size = [120, 60]

[node.receive]
text = "Receive order"
kind = "shape"
at = [0, 0]
size = [120, 60]

[edge.then]
kind = "edge"
from = { node = "receive" }
to = { node = "pack" }
"#;

    /// A board `check` finds nothing wrong with.
    const CLEAN: &str = r#"schema = 1

[node.pack]
text = "Pack"
kind = "shape"
at = [0, 200]
size = [120, 60]
z = "b"

[node.receive]
text = "Receive order"
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "a"

[edge.then]
kind = "edge"
from = { node = "receive" }
to = { node = "pack" }
z = "c"
"#;

    /// `FLOW` with one more shape, as another program would save it.
    pub fn flow_and(label: &str) -> String {
        format!(
            "{FLOW}\n[node.extra]\ntext = \"{label}\"\nkind = \"shape\"\nat = [200, 0]\nsize = [120, 60]\n"
        )
    }

    /// A board left with merge conflict markers, which is not TOML.
    pub fn conflicted() -> String {
        format!("<<<<<<< ours\n{FLOW}=======\n{FLOW}>>>>>>> theirs\n")
    }

    const GROUP: &str = "Boards/Samples";

    /// A workspace with the setting on: two boards, one that does not
    /// parse, and a Markdown note. Nothing is open.
    pub fn fixture() -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let group = directory.path().join(GROUP);
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("Flow.wren-board"), FLOW).unwrap();
        std::fs::write(group.join("Other.wren-board"), wren_board::EMPTY).unwrap();
        std::fs::write(group.join("Broken.wren-board"), conflicted()).unwrap();
        std::fs::write(group.join("Note.md"), "# Note\n\nwords here\n").unwrap();
        let app = app_at(directory.path());
        (directory, app)
    }

    pub fn app_at(root: &Path) -> App {
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        app.preferences.boards = true;
        app.workspace_mode = true;
        app.session = Some(Session::open_with(root, app.session_options()).unwrap());
        app.sync_editor(true);
        app
    }

    pub fn note_id(app: &App, title: &str, kind: NoteKind) -> NoteId {
        app.model()
            .unwrap()
            .notes
            .iter()
            .find(|note| note.title == title && note.kind == kind)
            .unwrap_or_else(|| panic!("no {kind:?} note titled {title}"))
            .id
    }

    pub fn open_board(app: &mut App, title: &str) -> NoteId {
        let note = note_id(app, title, NoteKind::Board);
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        note
    }

    pub fn open_markdown(app: &mut App, title: &str) -> NoteId {
        let note = note_id(app, title, NoteKind::Markdown);
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        note
    }

    pub fn session_text(app: &App) -> String {
        app.session
            .as_ref()
            .and_then(Session::document)
            .map_or_else(String::new, |document| document.text.clone())
    }

    pub fn file(directory: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
        directory.path().join(GROUP).join(name)
    }

    /// The surface shows the Session's open document and nothing else.
    #[track_caller]
    pub fn assert_in_step(app: &App, expected: Surface) {
        assert_eq!(app.surface(), expected);
        let document = app.session.as_ref().unwrap().document();
        assert_eq!(app.editor_note, document.map(|document| document.note));
        let text = session_text(app);
        match expected {
            Surface::Board => {
                assert_eq!(app.board.as_ref().unwrap().synced(), text);
                assert_eq!(app.editor.text(), "", "the hidden editor holds nothing");
                assert!(app.board_error.is_none());
            }
            Surface::Raw => {
                assert!(app.board.is_none());
                assert_eq!(app.editor.text(), text);
                assert!(app.board_error.is_some());
            }
            Surface::Text => {
                assert!(app.board.is_none());
                assert_eq!(app.editor.text(), text);
                assert!(app.board_error.is_none());
            }
        }
    }

    /// Give the view a widget size, as its first frame does.
    fn lay_out(app: &mut App) {
        let _ = app.board.as_mut().unwrap().update(
            wren_canvas::Message::Resized {
                width: 800.0,
                height: 600.0,
            },
            std::time::Instant::now(),
        );
    }

    pub fn parses(app: &App) -> u64 {
        app.board.as_ref().unwrap().probe.parses.get()
    }

    fn scenes(app: &App) -> u64 {
        app.board.as_ref().unwrap().probe.scenes.get()
    }

    #[test]
    fn the_surface_follows_the_open_note_through_every_transition() {
        let (_directory, mut app) = fixture();
        assert_in_step(&app, Surface::Text);
        assert!(app.editor_note.is_none());

        open_markdown(&mut app, "Note");
        assert_in_step(&app, Surface::Text);
        assert_eq!(app.editor.text(), "# Note\n\nwords here\n");
        assert_eq!(app.words, 4);

        // Text to Board: the editor is emptied, never left with the note.
        open_board(&mut app, "Flow");
        assert_in_step(&app, Surface::Board);
        let view = app.board.as_ref().unwrap();
        assert_eq!(view.board().nodes.len(), 2);
        assert_eq!(view.board().edges.len(), 1);
        assert_eq!(app.words, 0);

        // Board to Board.
        open_board(&mut app, "Other");
        assert_in_step(&app, Surface::Board);
        assert!(app.board.as_ref().unwrap().board().nodes.is_empty());

        // Board to Raw: the text is in the editor and the error names a line.
        open_board(&mut app, "Broken");
        assert_in_step(&app, Surface::Raw);
        assert_eq!(app.editor.text(), conflicted());
        assert_eq!(app.board_error.as_ref().unwrap().line, 1);

        // Raw to Board, and Raw to Text.
        open_board(&mut app, "Flow");
        assert_in_step(&app, Surface::Board);
        open_board(&mut app, "Broken");
        assert_in_step(&app, Surface::Raw);
        open_markdown(&mut app, "Note");
        assert_in_step(&app, Surface::Text);
        assert_eq!(app.editor.text(), "# Note\n\nwords here\n");

        // Board to Text shows the note, not stale or empty text.
        open_board(&mut app, "Flow");
        open_markdown(&mut app, "Note");
        assert_in_step(&app, Surface::Text);
        assert_eq!(app.editor.text(), "# Note\n\nwords here\n");

        // Back and Forward cross kinds the same way.
        let _ = app.apply_core(Op::Back).unwrap();
        assert_in_step(&app, Surface::Board);
        let _ = app.apply_core(Op::Forward).unwrap();
        assert_in_step(&app, Surface::Text);

        // No note at all.
        open_board(&mut app, "Flow");
        let _ = app.apply_core(Op::OpenNote { note: None }).unwrap();
        assert_in_step(&app, Surface::Text);
        assert_eq!(app.editor.text(), "");
    }

    #[test]
    fn the_surface_is_decided_again_when_the_text_changes_underneath_it() {
        let (directory, mut app) = fixture();
        let flow = file(&directory, "Flow.wren-board");
        open_board(&mut app, "Flow");
        assert_eq!(parses(&app), 1);

        // A valid board from elsewhere is reloaded into the same view.
        std::fs::write(&flow, flow_and("Ship")).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_in_step(&app, Surface::Board);
        assert_eq!(app.board.as_ref().unwrap().board().nodes.len(), 3);
        assert_eq!(parses(&app), 2);

        // Conflict markers: raw text, with the line.
        std::fs::write(&flow, conflicted()).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_in_step(&app, Surface::Raw);
        assert_eq!(app.board_error.as_ref().unwrap().line, 1);

        // Still not a board: the editor takes the new text and the new error.
        let broken = format!("{FLOW}\n[node.bad]\nat = \"nowhere\"\n");
        std::fs::write(&flow, &broken).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_in_step(&app, Surface::Raw);
        assert_eq!(app.editor.text(), broken);
        assert!(app.board_error.as_ref().unwrap().line > 1);

        // Fixed on disk: a board again.
        std::fs::write(&flow, FLOW).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_in_step(&app, Surface::Board);
        assert_eq!(app.board.as_ref().unwrap().board().nodes.len(), 2);

        // A Markdown file's bytes under a board's name are not a board. (A
        // heading alone would be: it is a TOML comment, and so an empty one.)
        let markdown = "# Not a board\n\nJust *words*.\n";
        std::fs::write(&flow, markdown).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_in_step(&app, Surface::Raw);
        assert_eq!(app.editor.text(), markdown);
        assert_eq!(app.board_error.as_ref().unwrap().line, 3);

        // Removed from outside while open.
        std::fs::write(&flow, FLOW).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_in_step(&app, Surface::Board);
        std::fs::remove_file(&flow).unwrap();
        let _ = app.process_core(Op::Reconcile);
        assert_in_step(&app, Surface::Text);
        assert!(app.editor_note.is_none());
    }

    /// `sync_editor` runs only when an op produced events or an error, so
    /// the surface can follow the Session only if every op that changes the
    /// open document's identity or text emits one. Each step here changes
    /// one of the two from outside the surface.
    #[test]
    fn every_session_driven_text_change_reaches_the_surface() {
        let (directory, mut app) = fixture();
        let flow = file(&directory, "Flow.wren-board");
        let note = open_board(&mut app, "Flow");
        assert_in_step(&app, Surface::Board);
        let mut seen = session_text(&app);
        let mut changed = |app: &App, what: &str| {
            let text = session_text(app);
            assert_ne!(text, seen, "{what} must change the Session's text");
            seen = text;
        };

        // A draft from outside the surface, as a later phase's edit or a
        // harness write would send it.
        let _ = app
            .apply_core(Op::EditDraft {
                note,
                contents: flow_and("Drafted"),
            })
            .unwrap();
        changed(&app, "a draft");
        assert_in_step(&app, Surface::Board);
        assert_eq!(app.board.as_ref().unwrap().board().nodes.len(), 3);

        // That draft meets a change on disk: a conflict copy of the board's
        // own kind, and the original reloaded.
        std::fs::write(&flow, flow_and("External")).unwrap();
        assert!(app.apply_core(Op::Flush).is_err());
        changed(&app, "a conflict");
        assert_in_step(&app, Surface::Board);
        let copy = app.conflict_copy.clone().expect("a conflict copy").1;
        assert!(copy
            .as_path()
            .to_string_lossy()
            .ends_with(NoteKind::Board.extension()));
        assert_eq!(
            std::fs::read_to_string(directory.path().join(copy.as_path())).unwrap(),
            flow_and("Drafted")
        );

        // A write of the whole note.
        let _ = app
            .apply_core(Op::WriteNote {
                note,
                contents: flow_and("Written"),
            })
            .unwrap();
        changed(&app, "a write");
        assert_in_step(&app, Surface::Board);
        assert_eq!(std::fs::read_to_string(&flow).unwrap(), flow_and("Written"));

        // A clean external reload.
        std::fs::write(&flow, flow_and("Reloaded")).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        changed(&app, "a reload");
        assert_in_step(&app, Surface::Board);

        // Ops that keep the text keep the view.
        let scenes_before = scenes(&app);
        let parses_before = parses(&app);
        let _ = app
            .apply_core(Op::Rename {
                target: wren_core::model::ItemRef::Note(note),
                new_name: "Flow renamed".into(),
            })
            .unwrap();
        let _ = app.apply_core(Op::Flush).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_in_step(&app, Surface::Board);
        assert_eq!(app.editor_note, Some(note));
        assert_eq!((scenes(&app), parses(&app)), (scenes_before, parses_before));

        // A new board opens from its template; deleting it leaves no board.
        let group = app.model().unwrap().note(note).unwrap().group;
        let _ = app
            .apply_core(Op::CreateNote {
                group,
                title: "Fresh".into(),
                kind: NoteKind::Board,
            })
            .unwrap();
        assert_in_step(&app, Surface::Board);
        assert_eq!(session_text(&app), wren_board::EMPTY);
        let fresh = app.editor_note.unwrap();
        assert_ne!(fresh, note);
        let _ = app
            .apply_core(Op::Delete {
                target: wren_core::model::ItemRef::Note(fresh),
            })
            .unwrap();
        assert_ne!(app.editor_note, Some(fresh));
        assert_in_step(&app, app.surface());
    }

    #[test]
    fn typing_in_a_raw_board_never_flips_the_surface() {
        let (directory, mut app) = fixture();
        let note = open_board(&mut app, "Broken");
        assert_in_step(&app, Surface::Raw);

        // Replace the conflicted text with a board that parses.
        let _ = app.update(Message::Editor(wren_editor::Message::Action(
            Action::SelectAll,
        )));
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
            Edit::Paste(Arc::new(FLOW.into())),
        ))));
        assert_eq!(session_text(&app), FLOW);
        assert_in_step(&app, Surface::Raw);
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Move(
            Motion::DocumentEnd,
        ))));
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Edit(
            Edit::Paste(Arc::new("# typed\n".into())),
        ))));
        let typed = format!("{FLOW}# typed\n");
        assert_in_step(&app, Surface::Raw);

        // Saving keeps it raw, and saves exactly what was typed.
        let _ = app.apply_core(Op::Flush).unwrap();
        assert_in_step(&app, Surface::Raw);
        assert_eq!(
            std::fs::read_to_string(file(&directory, "Broken.wren-board")).unwrap(),
            typed
        );
        for _ in 0..3 {
            let _ = app.update(Message::Tick);
            let _ = app.apply_core(Op::Reconcile).unwrap();
        }
        assert_in_step(&app, Surface::Raw);

        // It is a board the next time it opens.
        open_markdown(&mut app, "Note");
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        assert_in_step(&app, Surface::Board);
        assert_eq!(app.board.as_ref().unwrap().board().nodes.len(), 2);
    }

    #[test]
    fn an_idle_board_is_never_parsed_or_built_again() {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        lay_out(&mut app);
        let revision = app.edit_revision;
        assert_eq!((parses(&app), scenes(&app)), (1, 1));
        for _ in 0..40 {
            let _ = app.update(Message::Tick);
        }
        let _ = app.apply_core(Op::Flush).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        // The same view: a replaced one would have moved the revision.
        assert_eq!(app.edit_revision, revision);
        assert_eq!((parses(&app), scenes(&app)), (1, 1));

        // A theme change is one scene build and no parse; the same theme
        // again is neither.
        let other = if app.definition == definitions::LIGHT {
            definitions::DARK
        } else {
            definitions::LIGHT
        };
        let _ = app.update(Message::ChooseTheme(Some(other)));
        assert_eq!((parses(&app), scenes(&app)), (1, 2));
        assert_eq!(
            *app.board.as_ref().unwrap().palette(),
            wren_core::theme::board_palette(&other.colors)
        );
        let _ = app.update(Message::ChooseTheme(Some(other)));
        assert_eq!((parses(&app), scenes(&app)), (1, 2));
    }

    #[test]
    fn a_board_opens_fitted_once_and_then_returns_to_where_it_was() {
        let (directory, mut app) = fixture();
        let flow = open_board(&mut app, "Flow");
        // The first opening waits for the widget's size, then fits.
        assert!(app.board.as_ref().unwrap().fit_pending());
        lay_out(&mut app);
        let view = app.board.as_ref().unwrap();
        assert!(!view.fit_pending());
        let fitted = view.viewport();
        // Between the style bar and the tool dock, which the fit leaves
        // room for.
        let (top, bottom) = app.board_covered();
        assert_eq!(top, crate::app::board_style_bar::covered(true));
        assert_eq!(bottom, crate::app::board_dock::covered(true));
        assert!(top > 0.0 && bottom > 0.0);
        let room = Viewport::fitting(view.scene().bounds, (800.0, 600.0 - top - bottom));
        assert_eq!(
            fitted,
            Viewport {
                y: room.y - top / room.zoom,
                ..room
            }
        );
        let moved = Viewport {
            x: 37.0,
            y: -12.0,
            zoom: 1.5,
        };
        app.board.as_mut().unwrap().set_viewport(moved);

        // A reload keeps the viewport.
        std::fs::write(file(&directory, "Flow.wren-board"), flow_and("Ship")).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_eq!(app.board.as_ref().unwrap().board().nodes.len(), 3);
        assert_eq!(app.board.as_ref().unwrap().viewport(), moved);

        // Another board has never been looked at: it fits.
        open_board(&mut app, "Other");
        assert!(app.board.as_ref().unwrap().fit_pending());
        // Left before its first frame, it fits the next time too.
        open_markdown(&mut app, "Note");
        open_board(&mut app, "Other");
        assert!(app.board.as_ref().unwrap().fit_pending());

        // Back on the first board, the view is where it was left.
        let _ = app.apply_core(Op::OpenNote { note: Some(flow) }).unwrap();
        let view = app.board.as_ref().unwrap();
        assert!(!view.fit_pending());
        assert_eq!(view.viewport(), moved);
        lay_out(&mut app);
        assert_eq!(app.board.as_ref().unwrap().viewport(), moved);

        // Through the raw fallback and back as well.
        std::fs::write(file(&directory, "Flow.wren-board"), conflicted()).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_eq!(app.surface(), Surface::Raw);
        std::fs::write(file(&directory, "Flow.wren-board"), FLOW).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        assert_eq!(app.board.as_ref().unwrap().viewport(), moved);

        // Another workspace's notes are other notes: nothing is remembered.
        assert_eq!(app.board_viewports.len(), 1);
        app.sync_editor(true);
        assert!(app.board_viewports.is_empty());
        assert!(app.board.as_ref().unwrap().fit_pending());
    }

    #[test]
    fn with_the_setting_off_no_board_surface_exists() {
        let directory = crate::test_support::tempdir();
        let group = directory.path().join(GROUP);
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("Flow.wren-board"), FLOW).unwrap();
        std::fs::write(group.join("Note.md"), "note").unwrap();
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        app.preferences.boards = false;
        app.workspace_mode = true;
        app.session = Some(Session::open_with(directory.path(), app.session_options()).unwrap());
        app.sync_editor(true);
        let notes = &app.model().unwrap().notes;
        assert_eq!(notes.len(), 1);
        let note = notes[0].id;
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        assert_in_step(&app, Surface::Text);
        assert_eq!(app.editor.text(), "note");
        assert!(app.board_viewports.is_empty());
    }

    /// The sample workspace of the harness and the scenes.
    #[test]
    fn the_sample_boards_open_as_specified() {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/wrentest/fixtures/boards/workspace")
            .join(GROUP);
        let directory = crate::test_support::tempdir();
        let group = directory.path().join(GROUP);
        std::fs::create_dir_all(&group).unwrap();
        for entry in std::fs::read_dir(&samples).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), group.join(entry.file_name())).unwrap();
        }
        let mut app = app_at(directory.path());
        for (title, nodes, edges, issues, editable) in [
            ("Checkout flow", 15, 17, 0, true),
            ("Overlap", 3, 0, 0, true),
            ("Hand written", 3, 1, 0, true),
            ("Future", 4, 1, 0, false),
        ] {
            open_board(&mut app, title);
            assert_in_step(&app, Surface::Board);
            let view = app.board.as_ref().unwrap();
            assert_eq!(view.board().nodes.len(), nodes, "{title}");
            assert_eq!(view.board().edges.len(), edges, "{title}");
            assert_eq!(view.issues().len(), issues, "{title}");
            assert_eq!(
                *view.access() == wren_board::Access::Editable,
                editable,
                "{title}"
            );
        }
        open_board(&mut app, "Shapes");
        assert_in_step(&app, Surface::Board);
        let view = app.board.as_ref().unwrap();
        assert!(!view.issues().is_empty());
        assert_eq!(*view.access(), wren_board::Access::Editable);

        // Opening wrote nothing.
        for entry in std::fs::read_dir(&samples).unwrap() {
            let entry = entry.unwrap();
            assert_eq!(
                std::fs::read(entry.path()).unwrap(),
                std::fs::read(group.join(entry.file_name())).unwrap(),
                "{:?}",
                entry.file_name()
            );
        }
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
    }

    use std::time::{Duration, Instant, SystemTime};

    use super::super::commands::{Command, CommandId, Disposition};
    use super::super::palette::SearchTarget;

    pub fn modified(path: &Path) -> SystemTime {
        std::fs::metadata(path).unwrap().modified().unwrap()
    }

    /// Forty ticks that take the Session's clock to `seconds`, past the
    /// autosave delay, so a dirty draft would be on disk afterwards.
    pub fn tick_to(app: &mut App, seconds: u64) {
        for tick in 1..=40 {
            let _ = app.update(Message::Tick);
            let at = Duration::from_millis(seconds * 1000 * tick / 40);
            let session = app.session.as_mut().unwrap();
            session.apply_at(Op::Tick, at).unwrap();
        }
        app.sync_editor(false);
    }

    fn paste(text: &str) -> Message {
        Message::Editor(wren_editor::Message::Action(Action::Edit(Edit::Paste(
            Arc::new(text.into()),
        ))))
    }

    const EDITOR_COMMANDS: [CommandId; 6] = [
        CommandId::ToggleTask,
        CommandId::Bold,
        CommandId::Italic,
        CommandId::Code,
        CommandId::MoveLineUp,
        CommandId::MoveLineDown,
    ];

    #[test]
    fn the_editor_behind_a_board_never_writes_it() {
        let (directory, mut app) = fixture();
        let path = file(&directory, "Flow.wren-board");
        let saved = modified(&path);
        open_board(&mut app, "Flow");
        let untouched = |app: &App| {
            assert_in_step(app, Surface::Board);
            assert_eq!(session_text(app), FLOW);
            assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), FLOW);
            assert_eq!(modified(&path), saved);
        };

        // The editor is not ready, which every harness `editor.*` write asks
        // before it acts.
        assert!(app.ensure_editor_ready().is_err());

        // Editor commands, from the palette or a menu.
        for id in EDITOR_COMMANDS {
            assert!(!app.command_enabled(id), "{id:?}");
            let outcome = app.dispatch(Command::Run(id));
            assert_eq!(outcome.processed.disposition, Disposition::Blocked);
            untouched(&app);
        }

        // Editor messages, as `editor.insert`, a key or a stale widget
        // would send them.
        let revision = app.edit_revision;
        for message in [
            paste("bypass"),
            Message::Editor(wren_editor::Message::Intent(wren_editor::Intent::Bold)),
            Message::Editor(wren_editor::Message::Intent(wren_editor::Intent::Enter)),
            Message::Editor(wren_editor::Message::Action(Action::Edit(Edit::Enter))),
            Message::Editor(wren_editor::Message::Undo),
            Message::Editor(wren_editor::Message::Redo),
        ] {
            let _ = app.update(message);
            untouched(&app);
        }
        assert!(!app.find_open);
        assert_eq!(app.edit_revision, revision);

        // Even text that reached the hidden editor is not the board's draft.
        app.editor.replace_text("smuggled", false);
        app.text_changed(Instant::now());
        assert_eq!(session_text(&app), FLOW);
        assert_eq!(app.edit_revision, revision);
        app.editor.replace_text("", false);
        untouched(&app);

        // An engine switch rebuilds the editor from nothing, not from the
        // board's text; switching back does too.
        let engine = app.editor.backend();
        for _ in 0..2 {
            let outcome = app.dispatch(Command::Run(CommandId::SwitchEngine));
            assert!(outcome.processed.disposition.committed());
            untouched(&app);
        }
        assert_eq!(app.editor.backend(), engine);
        let _ = app.dispatch(Command::Run(CommandId::SwitchEngine));
        assert_ne!(app.editor.backend(), engine);
        let _ = app.dispatch(Command::Run(CommandId::ToggleMarkdownAssist));
        untouched(&app);

        // Losing the window's focus flushes; there is nothing to flush.
        app.window_unfocused().unwrap();
        tick_to(&mut app, 1);
        untouched(&app);
        tick_to(&mut app, 5);
        untouched(&app);

        // The same clock does save a Markdown draft, so the ticks above
        // would have written a board draft had there been one.
        let note = file(&directory, "Note.md");
        open_markdown(&mut app, "Note");
        let _ = app.update(Message::Editor(wren_editor::Message::Action(Action::Move(
            Motion::DocumentEnd,
        ))));
        let _ = app.update(paste("more"));
        assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
        tick_to(&mut app, 10);
        assert_eq!(
            std::fs::read_to_string(&note).unwrap(),
            "# Note\n\nwords here\nmore"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FLOW);
        assert_eq!(modified(&path), saved);
    }

    #[test]
    fn commands_are_enabled_by_the_surface() {
        let (_directory, mut app) = fixture();
        let enabled = |app: &App, id| app.command_enabled(id);

        open_markdown(&mut app, "Note");
        for id in EDITOR_COMMANDS {
            assert!(enabled(&app, id), "{id:?}");
        }
        assert!(enabled(&app, CommandId::Find));
        assert!(enabled(&app, CommandId::Outline));
        assert!(app.editor.markdown_assist());

        open_board(&mut app, "Flow");
        for id in EDITOR_COMMANDS {
            assert!(!enabled(&app, id), "{id:?}");
        }
        // Find and Outline open the jump palette on a board (§5.18 rule
        // 3); the Markdown outline itself has nothing to list.
        assert!(enabled(&app, CommandId::Find));
        assert!(enabled(&app, CommandId::Outline));
        assert_eq!(
            app.open_palette(super::super::palette::Mode::Outline)
                .processed
                .disposition,
            Disposition::Blocked
        );
        // What does not touch the text stays available.
        for id in [
            CommandId::RenameNote,
            CommandId::DeleteNote,
            CommandId::MoveNote,
            CommandId::TextBigger,
            CommandId::TextSmaller,
            CommandId::TextReset,
            CommandId::SwitchEngine,
            CommandId::ToggleMarkdownAssist,
            CommandId::ChooseTheme,
            CommandId::Jump,
            CommandId::Search,
        ] {
            assert!(enabled(&app, id), "{id:?}");
        }

        // Raw text is edited as plain text: Find works, and the Markdown
        // keys and the outline have nothing to act on.
        open_board(&mut app, "Broken");
        assert_eq!(app.surface(), Surface::Raw);
        assert!(app.ensure_editor_ready().is_ok());
        for id in EDITOR_COMMANDS {
            assert!(!enabled(&app, id), "{id:?}");
        }
        assert!(enabled(&app, CommandId::Find));
        assert!(!enabled(&app, CommandId::Outline));
        assert_eq!(
            app.open_palette(super::super::palette::Mode::Outline)
                .processed
                .disposition,
            Disposition::Blocked
        );
        assert!(!app.editor.markdown_assist());
        assert!(app.preferences.settings.markdown_assist);
        // Toggling the setting twice leaves the raw editor without it.
        for _ in 0..2 {
            let _ = app.dispatch(Command::Run(CommandId::ToggleMarkdownAssist));
            assert!(!app.editor.markdown_assist());
        }
        assert!(app.preferences.settings.markdown_assist);
        // An engine switch keeps the raw text in the editor.
        let _ = app.dispatch(Command::Run(CommandId::SwitchEngine));
        assert_in_step(&app, Surface::Raw);
        assert_eq!(app.editor.text(), conflicted());

        // Back on Markdown the setting applies again.
        open_markdown(&mut app, "Note");
        assert!(app.editor.markdown_assist());
        assert!(enabled(&app, CommandId::Bold));
        let _ = app.dispatch(Command::Run(CommandId::ToggleMarkdownAssist));
        assert!(!app.editor.markdown_assist());
        open_board(&mut app, "Broken");
        open_markdown(&mut app, "Note");
        assert!(!app.editor.markdown_assist());
    }

    #[test]
    fn the_text_size_commands_zoom_a_board_and_change_no_setting() {
        let (directory, _) = fixture();
        let settings = directory.path().join("config.conf");
        let written = "boards = on\neditor-font-size = 15\n";
        std::fs::write(&settings, written).unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (mut app, _) = App::with_config(options, None, Some(settings.clone()));
        assert!(app.preferences.boards);
        app.workspace_mode = true;
        app.session = Some(Session::open_with(directory.path(), app.session_options()).unwrap());
        app.sync_editor(true);
        let unchanged = |app: &App| {
            assert_eq!(app.preferences.settings.editor_font_size, 15.0);
            assert_eq!(app.editor.font_size(), 15.0);
            assert_eq!(std::fs::read_to_string(&settings).unwrap(), written);
        };
        let zoom = |app: &App| app.board.as_ref().unwrap().viewport().zoom;
        let run = |app: &mut App, id| {
            let outcome = app.dispatch(Command::Run(id));
            assert_eq!(outcome.processed.disposition, Disposition::Completed);
            assert!(outcome.processed.warning.is_none());
        };

        open_board(&mut app, "Flow");
        lay_out(&mut app);
        assert_eq!(zoom(&app), 1.0);
        let centre = app
            .board
            .as_ref()
            .unwrap()
            .viewport()
            .to_board((400.0, 300.0));
        let mut seen = Vec::new();
        for id in [
            CommandId::TextBigger,
            CommandId::TextBigger,
            CommandId::TextReset,
            CommandId::TextSmaller,
            CommandId::TextSmaller,
            CommandId::TextSmaller,
            CommandId::TextReset,
        ] {
            run(&mut app, id);
            seen.push(zoom(&app));
            unchanged(&app);
            // The stops zoom about the middle of the board's widget.
            let now = app
                .board
                .as_ref()
                .unwrap()
                .viewport()
                .to_board((400.0, 300.0));
            assert!((now.0 - centre.0).abs() < 0.01 && (now.1 - centre.1).abs() < 0.01);
        }
        assert_eq!(seen, [1.25, 1.5, 1.0, 0.85, 0.75, 0.6, 1.0]);
        // The limits hold.
        for _ in 0..wren_canvas::ZOOM_STOPS.len() {
            run(&mut app, CommandId::TextBigger);
        }
        assert_eq!(zoom(&app), 4.0);
        for _ in 0..wren_canvas::ZOOM_STOPS.len() {
            run(&mut app, CommandId::TextSmaller);
        }
        assert_eq!(zoom(&app), 0.1);
        unchanged(&app);
        assert_in_step(&app, Surface::Board);
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());

        // From the palette too, which closes.
        let _ = app.open_palette(super::super::palette::Mode::Commands);
        run(&mut app, CommandId::TextReset);
        assert!(app.palette.is_none());
        assert_eq!(zoom(&app), 1.0);
        unchanged(&app);

        // On a Markdown note, and on a board's raw text, they size the text.
        open_board(&mut app, "Broken");
        run(&mut app, CommandId::TextBigger);
        assert_eq!(app.preferences.settings.editor_font_size, 16.0);
        open_markdown(&mut app, "Note");
        run(&mut app, CommandId::TextBigger);
        assert_eq!(app.editor.font_size(), 17.0);
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            "boards = on\neditor-font-size = 17\n"
        );
        // The board kept its zoom meanwhile.
        open_board(&mut app, "Flow");
        assert_eq!(zoom(&app), 1.0);

        // An engine change in the file reaches the hidden editor empty.
        let engine = app.editor.backend();
        assert_eq!(engine, wren_editor::BackendKind::Owned);
        std::fs::write(
            &settings,
            "boards = on\neditor-engine = classic\neditor-font-size = 17\n",
        )
        .unwrap();
        let _ = app.reload_settings(false);
        assert_eq!(app.editor.backend(), wren_editor::BackendKind::Iced);
        std::fs::write(
            &settings,
            "boards = on\neditor-engine = advanced\neditor-font-size = 17\n",
        )
        .unwrap();
        let _ = app.reload_settings(false);
        assert_eq!(app.editor.backend(), engine);
        assert_in_step(&app, Surface::Board);
        assert_eq!(session_text(&app), FLOW);
    }

    #[test]
    fn a_search_hit_in_a_board_opens_it_without_touching_the_editor() {
        let (_directory, mut app) = fixture();
        open_markdown(&mut app, "Note");
        // As the search palette does before it searches.
        app.session.as_mut().unwrap().settle_search();
        let session = app.session.as_ref().unwrap();
        let hits = session.index().query(
            "receive",
            wren_core::index::Scope::Workspace,
            wren_core::index::QueryKind::Search,
            10,
        );
        // The label is found in the board, and in the raw text of the one
        // that does not parse.
        let flow = note_id(&app, "Flow", NoteKind::Board);
        let broken = note_id(&app, "Broken", NoteKind::Board);
        let mut found: Vec<_> = hits.iter().map(|hit| hit.note).collect();
        found.sort();
        assert_eq!(found, [flow.min(broken), flow.max(broken)]);
        let hit = hits.iter().find(|hit| hit.note == flow).unwrap();
        let target = SearchTarget {
            generation: app.workspace_generation(),
            index_revision: session.index().revision(),
            note: flow,
            query: "receive".into(),
            content_match: hit.content_match.clone(),
        };
        assert!(target.content_match.is_some());
        let outcome = app.dispatch(Command::OpenNote(target));
        assert!(outcome.processed.disposition.committed());
        assert_in_step(&app, Surface::Board);
        assert_eq!(app.editor.selected_text(), None);
    }

    #[test]
    fn the_footer_names_the_counts_the_issues_the_access_and_the_zoom() {
        fonts::load();
        let open = |text: &str| {
            BoardView::open(
                text,
                crate::theme::board_palette(definitions::DARK),
                fonts::UI,
            )
            .unwrap()
        };
        let mut clean = open(CLEAN);
        let footer = BoardFooter::of(&clean);
        assert_eq!(footer.stats, "Board · 2 nodes · 1 connector");
        assert_eq!(clean.issues(), []);
        assert!(footer.flags.is_empty());
        assert_eq!(footer.zoom, "100%");
        clean.set_viewport(Viewport {
            zoom: 0.754,
            ..clean.viewport()
        });
        assert_eq!(BoardFooter::of(&clean).zoom, "75%");

        let flagged = open(&format!(
            "{CLEAN}\n[edge.loose]\nkind = \"edge\"\nfrom = {{ node = \"pack\" }}\nto = {{ node = \"gone\" }}\nz = \"d\"\n"
        ));
        assert_eq!(flagged.issues().len(), 1);
        let footer = BoardFooter::of(&flagged);
        assert_eq!(footer.stats, "Board · 2 nodes · 2 connectors");
        assert_eq!(footer.flags, ["1 issue"]);

        let read_only =
            open(&CLEAN.replacen("schema = 1\n", "schema = 1\nrequires = [\"frames\"]\n", 1));
        assert_eq!(
            BoardFooter::of(&read_only).flags,
            ["Read-only: needs frames"]
        );
        let both = open(&FLOW.replacen("schema = 1\n", "schema = 1\nrequires = [\"frames\"]\n", 1));
        assert_eq!(
            BoardFooter::of(&both).flags,
            ["3 issues", "Read-only: needs frames"]
        );
        let newer =
            open(&CLEAN.replacen("schema = 1\n", "schema = 2\nrequires = [\"frames\"]\n", 1));
        assert_eq!(BoardFooter::of(&newer).flags, ["Read-only: schema 2"]);

        let one = open("schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\n");
        assert_eq!(BoardFooter::of(&one).stats, "Board · 1 node · 0 connectors");
        let empty = open(wren_board::EMPTY);
        assert_eq!(
            BoardFooter::of(&empty).stats,
            "Board · 0 nodes · 0 connectors"
        );
    }

    #[test]
    fn the_raw_banner_names_where_the_text_stops_being_a_board() {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Broken");
        let error = app.board_error.as_ref().unwrap();
        let banner = raw_banner(error);
        assert!(
            banner.starts_with(&format!(
                "This board is shown as text because line 1, column {} is not valid: ",
                error.column
            )),
            "{banner}"
        );
        assert!(banner.contains(&error.message), "{banner}");
        assert!(banner.ends_with("It opens as a board again once the text is valid."));
        assert!(!banner.contains(".."), "{banner}");
        let ends = ParseError {
            line: 4,
            column: 2,
            message: "expected a value.".into(),
        };
        assert_eq!(
            raw_banner(&ends),
            "This board is shown as text because line 4, column 2 is not valid: expected a value. It opens as a board again once the text is valid."
        );
    }

    fn wheel(app: &App) -> Message {
        Message::Board(wren_canvas::Message::Input(
            app.board.as_ref().unwrap().epoch(),
            Input::Wheel {
                delta: wren_canvas::Scroll::Lines { x: 0.0, y: -1.0 },
                at: (100.0, 100.0),
                zoom: false,
                sideways: false,
                scale_factor: 1.0,
            },
        ))
    }

    fn input(app: &App, input: Input) -> Message {
        Message::Board(wren_canvas::Message::Input(
            app.board.as_ref().unwrap().epoch(),
            input,
        ))
    }

    #[test]
    fn board_input_reaches_the_view_and_all_but_the_wheel_takes_the_column() {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        let _ = app.update(Message::Board(wren_canvas::Message::Resized {
            width: 800.0,
            height: 600.0,
        }));
        let view = app.board.as_ref().unwrap();
        assert!(!view.fit_pending(), "the size reached the view");
        let fitted = view.viewport();
        let text = session_text(&app);

        // The wheel pans an unfocused board and leaves the column alone.
        app.column = Column::Notes;
        let _ = app.update(wheel(&app));
        let panned = app.board.as_ref().unwrap().viewport();
        assert_ne!(panned, fitted);
        assert_eq!(app.column, Column::Notes);
        let _ = app.update(input(&app, Input::Cancel));
        assert_eq!(app.column, Column::Notes);
        // The widget telling the board it lost or took the keyboard is not
        // the user acting on it.
        for focus in [false, true, false] {
            let _ = app.update(input(&app, Input::Focus(focus)));
            assert_eq!(app.column, Column::Notes);
        }

        // A press is the board taking the keyboard.
        let press = Input::Press {
            button: wren_canvas::Button::Middle,
            at: (100.0, 100.0),
        };
        let _ = app.update(input(&app, press));
        assert_eq!(app.column, Column::Editor);
        for step in [
            Input::Move { at: (160.0, 100.0) },
            Input::Release {
                button: wren_canvas::Button::Middle,
            },
        ] {
            let _ = app.update(input(&app, step));
        }
        let dragged = app.board.as_ref().unwrap().viewport();
        assert!((panned.x - dragged.x - 60.0 / dragged.zoom).abs() < 0.01);
        for column in [Column::Notes, Column::Groups] {
            app.column = column;
            let _ = app.update(input(&app, Input::Space { held: true }));
            assert_eq!(app.column, Column::Editor);
            let _ = app.update(input(&app, Input::Space { held: false }));
        }

        // Under an open palette the column stays where the palette returns.
        app.column = Column::Notes;
        let _ = app.open_palette(super::super::palette::Mode::Commands);
        assert!(app.palette.is_some());
        let _ = app.update(input(&app, press));
        let _ = app.update(input(
            &app,
            Input::Release {
                button: wren_canvas::Button::Middle,
            },
        ));
        assert_eq!(app.column, Column::Notes);
        assert!(!app.board.as_ref().unwrap().machine().panning());

        // Looking is not a change: the Session's text is what it was.
        assert_eq!(session_text(&app), text);
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());

        // With no board open the message has nowhere to go.
        let _ = app.close_palette();
        open_markdown(&mut app, "Note");
        app.column = Column::Notes;
        let _ = app.update(Message::Board(wren_canvas::Message::Input(0, press)));
        assert_eq!(app.column, Column::Notes);
        assert_in_step(&app, Surface::Text);
    }

    /// A pinch is looked at as the wheel is: it zooms a board without the
    /// keyboard, under a palette too, takes no column and writes nothing.
    #[test]
    fn a_pinch_zooms_the_board_and_takes_nothing() {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        let _ = app.update(Message::Board(wren_canvas::Message::Resized {
            width: 800.0,
            height: 600.0,
        }));
        let text = session_text(&app);
        let pinch = |delta| Input::Pinch {
            delta,
            at: (100.0, 100.0),
        };
        app.column = Column::Notes;
        let before = app.board.as_ref().unwrap().viewport();
        let _ = app.update(input(&app, pinch(0.25)));
        let after = app.board.as_ref().unwrap().viewport();
        assert!((after.zoom - before.zoom * 1.25).abs() < 1e-5, "{after:?}");
        assert_eq!(app.column, Column::Notes);
        assert!(!app.board.as_ref().unwrap().machine().focused());

        let _ = app.open_palette(super::super::palette::Mode::Commands);
        let _ = app.update(input(&app, pinch(-0.2)));
        let under = app.board.as_ref().unwrap().viewport();
        assert!((under.zoom - after.zoom * 0.8).abs() < 1e-5, "{under:?}");
        assert_eq!(app.column, Column::Notes);

        let view = app.board.as_ref().unwrap();
        assert!(view.pristine() && !view.can_undo());
        assert_eq!((app.board_revision, session_text(&app)), (0, text));
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
    }

    /// The app hands the board the vendored iced_winit's record of pinches,
    /// which gives them out only in the frame of the window they were
    /// recorded for, once each.
    #[test]
    fn the_pinch_record_is_one_windows_for_one_frame() {
        use iced_winit::pinch::{self, Frame};
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        let view = app.board.as_ref().unwrap();
        let (ours, other) = (iced::window::Id::unique(), iced::window::Id::unique());
        pinch::record(ours, 0.25);
        pinch::record(other, 0.5);
        pinch::record(ours, -0.1);
        // Outside a frame there is nothing to take.
        assert!(view.take_pinches().is_empty());
        {
            let _frame = Frame::begin(ours);
            assert_eq!(view.take_pinches(), [0.25, -0.1]);
            // Taken once.
            assert!(view.take_pinches().is_empty());
        }
        assert!(view.take_pinches().is_empty());
        // What no widget took in its frame is gone with the frame, and a
        // pinch recorded during a frame waits for the next.
        pinch::record(ours, 0.3);
        {
            let _frame = Frame::begin(ours);
            pinch::record(ours, 0.4);
        }
        {
            let _frame = Frame::begin(ours);
            assert_eq!(view.take_pinches(), [0.4]);
        }
        {
            let _frame = Frame::begin(other);
            assert_eq!(view.take_pinches(), [0.5]);
        }
        // A window that closed takes its record with it.
        pinch::record(other, 0.5);
        pinch::forget(other);
        {
            let _frame = Frame::begin(other);
            assert!(view.take_pinches().is_empty());
        }
        // With no frame coming, only the newest wait.
        for n in 0..pinch::WAITING + 3 {
            pinch::record(ours, n as f64);
        }
        let _frame = Frame::begin(ours);
        let waited = view.take_pinches();
        assert_eq!(waited.len(), pinch::WAITING);
        assert_eq!(waited[0], 3.0);
    }

    /// The groups and keys of the Commands palette's rows.
    fn command_rows(app: &mut App) -> Vec<(&'static str, CommandId, bool)> {
        let _ = app.open_palette(super::super::palette::Mode::Commands);
        let rows = app
            .palette
            .as_ref()
            .unwrap()
            .results
            .iter()
            .filter_map(|row| match row.key {
                super::super::palette::Key::Command(id) => Some((row.group, id, row.enabled)),
                _ => None,
            })
            .collect();
        let _ = app.close_palette();
        rows
    }

    fn board_ids() -> impl Iterator<Item = CommandId> {
        wren_canvas::Command::all().map(CommandId::Board)
    }

    #[test]
    fn the_palette_lists_the_board_group_only_on_a_board_and_no_markdown_there() {
        let (_directory, mut app) = fixture();
        let is_board = |id: CommandId| id.board_command().is_some();

        open_board(&mut app, "Flow");
        // With one node selected every board command is enabled but align
        // and distribute, which need more.
        let _ = app.dispatch(Command::Run(CommandId::Board(
            wren_canvas::Command::Select(wren_board::Dir::Down),
        )));
        let rows = command_rows(&mut app);
        assert!(rows.iter().all(|(group, ..)| *group != "Markdown"));
        let listed: Vec<_> = rows.iter().filter(|(_, id, _)| is_board(*id)).collect();
        assert_eq!(listed.len(), 94);
        for (_, id, enabled) in &listed {
            let more =
                id.key().starts_with("board.align.") || id.key().starts_with("board.distribute.");
            assert_eq!(*enabled, !more, "{id:?}");
        }
        let groups: Vec<_> = rows.iter().map(|(group, ..)| *group).collect();
        let board = groups.iter().position(|group| *group == "Board").unwrap();
        assert!(groups[..board].iter().all(|group| *group == "This note"));
        assert!(rows.iter().any(|(group, id, _)| *group == "View"
            && *id == CommandId::Board(wren_canvas::Command::ZoomFit)));
        // Jump to node stands for Find and Outline there (§5.1 rule 8).
        assert!(rows
            .iter()
            .any(|(group, id, _)| *group == "Go"
                && *id == CommandId::Board(wren_canvas::Command::Jump)));
        assert!(rows
            .iter()
            .all(|(_, id, _)| !matches!(id, CommandId::Find | CommandId::Outline)));

        // A text note and a board shown as text list none of them, and the
        // Markdown group again.
        for title in ["Note", "Broken"] {
            if title == "Note" {
                open_markdown(&mut app, title);
            } else {
                open_board(&mut app, title);
            }
            let rows = command_rows(&mut app);
            assert!(rows.iter().all(|(_, id, _)| !is_board(*id)), "{title}");
            assert!(
                rows.iter().any(|(group, ..)| *group == "Markdown"),
                "{title}"
            );
            assert!(rows.iter().any(|(_, id, _)| *id == CommandId::Find));
        }

        // With the setting off no board row is listed anywhere.
        app.preferences.boards = false;
        open_markdown(&mut app, "Note");
        assert!(command_rows(&mut app).iter().all(|(_, id, _)| !id.boards()));
    }

    /// A node inside another selected one moves with it and is not counted
    /// toward the two that align needs or the three of distribute.
    #[test]
    fn align_and_distribute_count_a_container_and_its_child_once() {
        let directory = crate::test_support::tempdir();
        let group = directory.path().join(GROUP);
        std::fs::create_dir_all(&group).unwrap();
        let nested = "schema = 1\n\n[node.box]\nkind = \"shape\"\nat = [0, 0]\nsize = [200, 200]\nz = \"a\"\n\n\
                      [node.k]\nkind = \"shape\"\nat = [20, 20]\nparent = \"box\"\nsize = [40, 40]\nz = \"a\"\n";
        let third = "\n[node.c]\nkind = \"shape\"\nat = [400, 0]\nsize = [40, 40]\nz = \"b\"\n";
        std::fs::write(group.join("Nested.wren-board"), nested).unwrap();
        std::fs::write(group.join("Three.wren-board"), format!("{nested}{third}")).unwrap();
        let mut app = app_at(directory.path());
        let select_all = Command::Run(CommandId::Board(wren_canvas::Command::SelectAll));
        let arranging = |app: &App| {
            board_ids()
                .filter(|id| {
                    let key = id.key();
                    key.starts_with("board.align.") || key.starts_with("board.distribute.")
                })
                .filter(|id| app.command_enabled(*id))
                .map(CommandId::key)
                .collect::<Vec<_>>()
        };

        open_board(&mut app, "Nested");
        let _ = app.dispatch(select_all.clone());
        assert_eq!(app.board.as_ref().unwrap().machine().selection().len(), 2);
        assert!(arranging(&app).is_empty(), "{:?}", arranging(&app));

        open_board(&mut app, "Three");
        let _ = app.dispatch(select_all);
        assert_eq!(app.board.as_ref().unwrap().machine().selection().len(), 3);
        let on = arranging(&app);
        assert_eq!(on.len(), 6, "{on:?}");
        assert!(
            on.iter().all(|key| key.starts_with("board.align.")),
            "{on:?}"
        );
    }

    #[test]
    fn board_commands_need_a_board_and_a_writable_one_to_change_it() {
        let (directory, mut app) = fixture();
        open_markdown(&mut app, "Note");
        for id in board_ids() {
            assert!(!app.command_enabled(id), "{id:?}");
        }
        let outcome = app.dispatch(Command::Run(CommandId::Board(
            wren_canvas::Command::SelectAll,
        )));
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        open_board(&mut app, "Broken");
        assert!(board_ids().all(|id| !app.command_enabled(id)));
        open_board(&mut app, "Flow");
        // With nothing selected, a command for one element is off, and so
        // is one that needs a fragment.
        let walk = Command::Run(CommandId::Board(wren_canvas::Command::Select(
            wren_board::Dir::Down,
        )));
        let needs_one = |key: &str| {
            key == "board.label.edit"
                || key.starts_with("board.connect.") && key != "board.connect.next"
                || key.starts_with("board.select.connector_")
        };
        let needs_more =
            |key: &str| key.starts_with("board.align.") || key.starts_with("board.distribute.");
        let needs_a_node = |key: &str| {
            matches!(key, "board.copy" | "board.cut" | "board.duplicate") || needs_more(key)
        };
        for id in board_ids() {
            let key = id.key();
            assert_eq!(
                app.command_enabled(id),
                !needs_one(key) && !needs_a_node(key),
                "{key}"
            );
        }
        // One node selected: every board command but align and distribute.
        let _ = app.dispatch(walk.clone());
        assert_eq!(app.board.as_ref().unwrap().machine().selection().len(), 1);
        for id in board_ids() {
            assert_eq!(app.command_enabled(id), !needs_more(id.key()), "{id:?}");
        }
        // Both nodes and the connector: the commands for one element are
        // off again, and align is on; distribute needs a third node.
        let _ = app.dispatch(Command::Run(CommandId::Board(
            wren_canvas::Command::SelectAll,
        )));
        for id in board_ids() {
            let key = id.key();
            let three = key.starts_with("board.distribute.");
            assert_eq!(app.command_enabled(id), !needs_one(key) && !three, "{id:?}");
        }
        // One connector: no fragment.
        let _ = app.dispatch(Command::Run(CommandId::Board(
            wren_canvas::Command::SelectNone,
        )));
        let view = app.board.as_ref().unwrap();
        let (viewport, placed) = (view.viewport(), wren_board::placements(view.board()));
        let middle = |name: &str| {
            let id = wren_board::Id::new(name).unwrap();
            let (at, size) = (placed[&id].at, view.board().nodes[&id].size);
            (
                at[0] as f32 + size[0] as f32 / 2.0,
                at[1] as f32 + size[1] as f32 / 2.0,
            )
        };
        let (from, to) = (middle("receive"), middle("pack"));
        let on_edge = viewport.to_screen(((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0));
        let _ = app.update(input(
            &app,
            Input::Press {
                button: wren_canvas::Button::Left,
                at: on_edge,
            },
        ));
        let _ = app.update(input(
            &app,
            Input::Release {
                button: wren_canvas::Button::Left,
            },
        ));
        let selection = app.board.as_ref().unwrap().machine().selection();
        assert_eq!(
            selection
                .iter()
                .map(|element| element.id().as_str())
                .collect::<Vec<_>>(),
            ["then"]
        );
        for id in board_ids() {
            assert_eq!(app.command_enabled(id), !needs_a_node(id.key()), "{id:?}");
        }

        // A read-only board takes looking, selecting, copying and zooming.
        std::fs::write(
            file(&directory, "Locked.wren-board"),
            FLOW.replacen("schema = 1\n", "schema = 1\nrequires = [\"frames\"]\n", 1),
        )
        .unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        open_board(&mut app, "Locked");
        assert!(matches!(
            app.board.as_ref().unwrap().access(),
            Access::ReadOnly(_)
        ));
        let _ = app.dispatch(walk);
        let enabled: Vec<_> = board_ids()
            .filter(|id| app.command_enabled(*id))
            .map(CommandId::key)
            .collect();
        assert_eq!(
            enabled,
            [
                "board.tool.select",
                "board.tool.hand",
                "board.select.left",
                "board.select.right",
                "board.select.up",
                "board.select.down",
                "board.select.extend_left",
                "board.select.extend_right",
                "board.select.extend_up",
                "board.select.extend_down",
                "board.select.connector_left",
                "board.select.connector_right",
                "board.select.connector_up",
                "board.select.connector_down",
                "board.select.all",
                "board.select.connected",
                "board.select.none",
                "board.copy",
                "board.zoom.fit",
                "board.zoom.selection",
                "board.actions",
                "board.jump",
                "board.keys",
            ]
        );
    }

    #[test]
    fn a_board_command_from_the_palette_closes_it_and_gives_the_board_the_keyboard() {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        let text = session_text(&app);
        app.column = Column::Notes;
        let _ = app.open_palette(super::super::palette::Mode::Commands);
        let outcome = app.dispatch(Command::Run(CommandId::Board(
            wren_canvas::Command::ZoomFit,
        )));
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert!(app.palette.is_none());
        assert_eq!(app.column, Column::Editor);
        assert!(outcome.task.units() > 0, "the board is focused");
        assert_eq!(session_text(&app), text);
        assert_in_step(&app, Surface::Board);
    }

    #[test]
    fn the_app_leaves_board_command_keys_to_the_board() {
        use crate::keys::Input;
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        let text = session_text(&app);
        let serial = app.action_serial;
        let keymap = app.preferences.keymap.clone();
        let mut pressed = 0;
        for id in board_ids() {
            let Some(accel) = keymap.accel(id.key()) else {
                continue;
            };
            pressed += 1;
            let _ = app.update(Message::Key(Input::Accel(accel.clone())));
            assert_eq!(app.action_serial, serial, "{} ran an action", id.key());
        }
        assert_eq!(pressed, 54 + 13 + 1);
        assert!(app.palette.is_none() && app.dialog.is_none() && !app.focus);
        assert_eq!(session_text(&app), text);
        assert_in_step(&app, Surface::Board);
        // An app command's key still runs, once.
        let _ = app.update(crate::app::preferences::shortcut(CommandId::Jump));
        assert_eq!(app.action_serial, serial.wrapping_add(1));
    }

    fn dialog_kind(app: &App) -> Option<super::super::workspace::DialogKind> {
        app.dialog.as_ref().map(|dialog| dialog.kind.clone())
    }

    #[test]
    fn the_new_note_dialog_opens_on_markdown_and_tab_steps_its_kind() {
        use super::super::workspace::DialogKind;
        let (directory, mut app) = fixture();
        open_markdown(&mut app, "Note");
        let group = app.selected_group.unwrap();
        let create = |kind| Some(DialogKind::CreateNote(group, kind));

        let _ = app.dispatch(Command::Run(CommandId::NewNote));
        assert_eq!(dialog_kind(&app), create(NoteKind::Markdown));
        for (reverse, kind) in [
            (false, NoteKind::Board),
            (false, NoteKind::Markdown),
            (true, NoteKind::Board),
            (true, NoteKind::Markdown),
        ] {
            let _ = app.update(Message::Tab(reverse));
            assert_eq!(dialog_kind(&app), create(kind));
        }
        // A click picks a pill and hands the keyboard back to the name.
        let task = app.update(Message::DialogNoteKind(NoteKind::Board));
        assert!(task.units() > 0);
        assert_eq!(dialog_kind(&app), create(NoteKind::Board));

        // The last choice is never remembered.
        let _ = app.update(Message::CancelDialog);
        let _ = app.dispatch(Command::Run(CommandId::NewNote));
        assert_eq!(dialog_kind(&app), create(NoteKind::Markdown));
        let _ = app.update(Message::CancelDialog);

        // New board… opens it on Board, and Enter makes an empty board,
        // shown at 100 % with the keyboard.
        assert!(app.command_enabled(CommandId::NewBoard));
        let _ = app.dispatch(Command::Run(CommandId::NewBoard));
        assert_eq!(dialog_kind(&app), create(NoteKind::Board));
        app.column = Column::Notes;
        let _ = app.update(Message::DialogName("Fresh".into()));
        let task = app.update(Message::SubmitDialog);
        assert!(app.dialog.is_none());
        assert!(task.units() > 0);
        assert_eq!(app.column, Column::Editor);
        assert_in_step(&app, Surface::Board);
        assert_eq!(session_text(&app), wren_board::EMPTY);
        assert_eq!(app.board.as_ref().unwrap().viewport().zoom, 1.0);
        let fresh = note_id(&app, "Fresh", NoteKind::Board);
        assert_eq!(app.selected_note, Some(fresh));
        assert_eq!(
            std::fs::read_to_string(file(&directory, "Fresh.wren-board")).unwrap(),
            wren_board::EMPTY
        );
        assert!(!file(&directory, "Fresh.md").exists());

        // Markdown is still ⌘N, type, Enter.
        let _ = app.dispatch(Command::Run(CommandId::NewNote));
        let _ = app.update(Message::DialogName("Plain".into()));
        let _ = app.update(Message::SubmitDialog);
        assert_in_step(&app, Surface::Text);
        assert!(file(&directory, "Plain.md").exists());

        // A name a board already has is numbered, as a Markdown note's is,
        // and the board that has it is left alone.
        let flow = std::fs::read(file(&directory, "Flow.wren-board")).unwrap();
        let _ = app.dispatch(Command::Run(CommandId::NewBoard));
        let _ = app.update(Message::DialogName("Flow".into()));
        let _ = app.update(Message::SubmitDialog);
        assert!(app.dialog.is_none());
        assert_eq!(
            app.selected_note,
            Some(note_id(&app, "Flow 2", NoteKind::Board))
        );
        assert_eq!(
            std::fs::read(file(&directory, "Flow.wren-board")).unwrap(),
            flow
        );

        // Rename has no kind, and Tab leaves it alone.
        let _ = app.dispatch(Command::Run(CommandId::RenameNote));
        let rename = dialog_kind(&app);
        assert!(matches!(rename, Some(DialogKind::Rename(_))));
        let _ = app.update(Message::Tab(false));
        let _ = app.update(Message::DialogNoteKind(NoteKind::Board));
        assert_eq!(dialog_kind(&app), rename);
    }

    #[test]
    fn with_the_setting_off_the_new_note_dialog_offers_no_kind() {
        use super::super::workspace::DialogKind;
        let (_directory, mut app) = fixture();
        open_markdown(&mut app, "Note");
        app.preferences.boards = false;
        let group = app.selected_group.unwrap();
        assert!(!app.command_enabled(CommandId::NewBoard));
        let outcome = app.dispatch(Command::Run(CommandId::NewBoard));
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert!(app.dialog.is_none());
        assert!(command_rows(&mut app)
            .iter()
            .all(|(_, id, _)| *id != CommandId::NewBoard));
        let _ = app.dispatch(Command::Run(CommandId::NewNote));
        let _ = app.update(Message::Tab(false));
        let _ = app.update(Message::DialogNoteKind(NoteKind::Board));
        assert_eq!(
            dialog_kind(&app),
            Some(DialogKind::CreateNote(group, NoteKind::Markdown))
        );
    }

    #[test]
    fn only_a_plain_tab_or_shift_tab_steps_the_dialog() {
        use iced::keyboard::key::{Code, Named, Physical};
        use iced::keyboard::{Event, Key, Location, Modifiers};
        let press = |key: Key, modifiers| Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Code(Code::Tab),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        };
        let tab = || Key::Named(Named::Tab);
        let step = super::super::workspace::tab;
        assert_eq!(step(&press(tab(), Modifiers::empty())), Some(false));
        assert_eq!(step(&press(tab(), Modifiers::SHIFT)), Some(true));
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            assert_eq!(step(&press(tab(), modifiers)), None);
        }
        assert_eq!(
            step(&press(Key::Named(Named::Enter), Modifiers::empty())),
            None
        );
    }

    /// The two export commands' rows in the Commands palette, SVG then
    /// PNG, each `None` when it is not listed.
    fn export_rows(app: &mut App) -> [Option<bool>; 3] {
        let _ = app.open_palette(super::super::palette::Mode::Commands);
        let row = |app: &App, title: &str| {
            app.palette
                .as_ref()
                .unwrap()
                .results
                .iter()
                .find(|row| row.title == title)
                .map(|row| row.enabled)
        };
        let rows = [
            row(app, "Export board as SVG…"),
            row(app, "Export board as PNG…"),
            row(app, "Export board as draw.io…"),
        ];
        let _ = app.close_palette();
        rows
    }

    #[test]
    fn export_is_enabled_on_a_board_and_listed_only_with_the_setting_on() {
        let (_directory, mut app) = fixture();
        let exports = [
            CommandId::ExportSvg,
            CommandId::ExportPng,
            CommandId::ExportDrawio,
        ];
        let enabled = |app: &App| exports.map(|id| app.command_enabled(id));
        for (id, key) in
            exports
                .into_iter()
                .zip(["note.export_svg", "note.export_png", "note.export_drawio"])
        {
            assert_eq!(CommandId::parse(key), Some(id));
            assert_eq!(id.scope(), super::super::commands::Scope::App);
            assert!(id.boards());
        }
        assert_eq!(enabled(&app), [false; 3]);
        assert_eq!(export_rows(&mut app), [Some(false); 3]);
        open_markdown(&mut app, "Note");
        assert_eq!(enabled(&app), [false; 3]);
        open_board(&mut app, "Broken");
        assert_eq!(app.surface(), Surface::Raw);
        assert_eq!(enabled(&app), [false; 3]);
        open_board(&mut app, "Flow");
        assert_eq!(enabled(&app), [true; 3]);
        assert_eq!(export_rows(&mut app), [Some(true); 3]);
        // Only the board's own commands are held back by the setting.
        for definition in super::super::commands::definitions() {
            assert_eq!(
                definition.id.boards(),
                matches!(
                    definition.id,
                    CommandId::ExportSvg
                        | CommandId::ExportPng
                        | CommandId::ExportDrawio
                        | CommandId::NewBoard
                ) || definition.scope == super::super::commands::Scope::Board,
                "{}",
                definition.key
            );
        }

        // With the setting off they are listed nowhere and run nowhere.
        let directory = crate::test_support::tempdir();
        std::fs::create_dir_all(directory.path().join(GROUP)).unwrap();
        std::fs::write(directory.path().join(GROUP).join("Note.md"), "note").unwrap();
        let mut off = app_at(directory.path());
        off.preferences.boards = false;
        let note = off.model().unwrap().notes[0].id;
        let _ = off.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        assert_eq!(export_rows(&mut off), [None; 3]);
        for id in exports {
            let outcome = off.dispatch(Command::Run(id));
            assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        }
    }

    /// What an export's dialog closing on `path` comes to: the dialog's
    /// own outcome when nothing was started, else the completion its
    /// worker reports once it has finished.
    fn chosen(app: &mut App, tag: u64, path: Option<PathBuf>) -> Gateway {
        let outcome = app.board_export_chosen(tag, path);
        if outcome.processed.disposition != Disposition::AsyncPending {
            return outcome;
        }
        assert_eq!(outcome.processed.correlation, tag);
        assert!(app.export_writing());
        let _ = app.board_export_written(tag);
        assert!(app.board_export.is_none());
        Gateway {
            processed: app.last_processed.clone().unwrap(),
            task: Task::none(),
            core_error: None,
        }
    }

    /// An export whose file is chosen is rendered and written on its own
    /// thread, and completes as the action that asked for it; until then no
    /// other export starts, and the board is still the user's.
    #[test]
    fn an_export_is_written_on_its_worker_and_one_runs_at_a_time() {
        let (directory, mut app) = fixture();
        let flow = file(&directory, "Flow.wren-board");
        open_board(&mut app, "Flow");
        lay_out(&mut app);
        let drawn = svg_document(app.board.as_ref().unwrap());
        let asked = app.dispatch(Command::Run(CommandId::ExportPng));
        let tag = asked.processed.correlation;
        assert_eq!(app.export_state(), Some("waiting"));

        let out = crate::test_support::tempdir();
        let target = out.path().join("Flow.png");
        let pending = app.board_export_chosen(tag, Some(target.clone()));
        assert_eq!(pending.processed.disposition, Disposition::AsyncPending);
        assert_eq!(pending.processed.correlation, tag);
        assert_eq!(pending.task.units(), 1, "the worker's report");
        assert_eq!(app.export_state(), Some("writing"));
        for (id, format) in [
            (CommandId::ExportPng, Format::Png),
            (CommandId::ExportSvg, Format::Svg),
            (CommandId::ExportDrawio, Format::Drawio),
        ] {
            assert!(!app.command_enabled(id), "{id:?}");
            let refused = app.dispatch(Command::Run(id));
            assert_eq!(refused.processed.disposition, Disposition::Blocked);
            let refused = app.export_board(format);
            assert_eq!(refused.processed.disposition, Disposition::Blocked);
            let warning = refused.processed.warning.unwrap();
            assert!(warning.contains("is still being written"), "{warning}");
        }
        // The dialog's tag again, and another export's report, are stale.
        assert_eq!(
            app.board_export_chosen(tag, Some(target.clone()))
                .processed
                .disposition,
            Disposition::Blocked
        );
        let _ = app.board_export_written(tag + 1);
        assert!(app.export_writing());
        // The board answers while the export is written.
        let _ = app.update(Message::ChooseTheme(Some(definitions::DARK)));
        assert!(app.command_enabled(CommandId::Board(wren_canvas::Command::SelectAll)));

        let _ = app.board_export_written(tag);
        let done = app.last_processed.clone().unwrap();
        assert_eq!(done.disposition, Disposition::Completed);
        assert_eq!(done.correlation, tag);
        assert_eq!(app.export_state(), None);
        assert!(app.command_enabled(CommandId::ExportPng));
        assert_eq!(
            std::fs::read(&target).unwrap(),
            board_export::png(&drawn, board_export::PNG_SCALE).unwrap()
        );
        assert_eq!(files_in(out.path()), ["Flow.png"]);
        assert_eq!(std::fs::read_to_string(&flow).unwrap(), FLOW);
    }

    /// A quit asked for while an export is written waits for the file, and
    /// then flushes and quits; it is not lost.
    #[test]
    fn a_quit_while_an_export_is_written_waits_for_it() {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        let _ = app.dispatch(Command::Run(CommandId::ExportSvg));
        let tag = app.board_export.as_ref().unwrap().tag;
        let out = crate::test_support::tempdir();
        let target = out.path().join("Flow.svg");
        let _ = app.board_export_chosen(tag, Some(target.clone()));
        assert!(app.export_writing());
        let quit = app.dispatch(Command::Run(CommandId::Quit));
        assert_eq!(quit.processed.disposition, Disposition::Completed);
        assert_eq!(
            quit.task.units(),
            0,
            "the window stays until the file is written"
        );
        assert!(app.quit_after_export);

        let exit = app.board_export_written(tag);
        assert!(!app.quit_after_export);
        assert!(exit.units() > 0, "the quit runs");
        assert!(std::fs::read_to_string(&target).unwrap().contains("<svg"));
        // With no export under way a quit is at once.
        assert!(app.dispatch(Command::Run(CommandId::Quit)).task.units() > 0);
        assert!(!app.quit_after_export);
    }

    fn files_in(directory: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// `wren export` writes what the app's export writes for the same board
    /// in the same theme, SVG and PNG, and reads the file it is given, not
    /// the open draft.
    #[test]
    fn the_command_line_export_is_the_app_export() {
        let (directory, mut app) = fixture();
        let flow = file(&directory, "Flow.wren-board");
        for definition in [definitions::DARK, definitions::LIGHT] {
            let _ = app.update(Message::ChooseTheme(Some(definition)));
            open_board(&mut app, "Flow");
            lay_out(&mut app);
            let drawn = svg_document(app.board.as_ref().unwrap());
            let exported = |format: Format, scale: f32| {
                let mut out = Vec::new();
                let mut warnings = Vec::new();
                board_export::run(
                    &crate::cli::ExportArgs {
                        board: flow.clone(),
                        output: crate::cli::Output::Stdout,
                        format,
                        theme: definition,
                        scale,
                        background: board_export::Background::Theme,
                    },
                    &mut out,
                    &mut |warning| warnings.push(warning),
                )
                .unwrap();
                // `FLOW` gives its elements no `z`.
                assert_eq!(warnings.len(), 3, "{warnings:?}");
                out
            };
            assert_eq!(
                String::from_utf8(exported(Format::Svg, 1.0)).unwrap(),
                drawn
            );
            assert_eq!(
                exported(Format::Png, board_export::PNG_SCALE),
                board_export::png(&drawn, board_export::PNG_SCALE).unwrap()
            );
            // The draw.io file is in the colours of the theme asked for:
            // the app's is always Wren Light's, whatever its own theme.
            let drawio = String::from_utf8(exported(Format::Drawio, 1.0)).unwrap();
            let app_drawio = export_document(app.board.as_ref().unwrap(), Format::Drawio, "Flow");
            assert_eq!(drawio == app_drawio, definition == definitions::LIGHT);
            assert!(drawio.contains("<diagram id=\"wren.board\" name=\"Flow\">"));
        }
    }

    /// `note.export_drawio` writes the board as a draw.io file in Wren
    /// Light's colours, made when the command ran, under the note's title;
    /// inside the workspace only as a `.drawio` file.
    #[test]
    fn a_drawio_export_is_the_board_in_wren_light_whatever_the_theme() {
        let (directory, mut app) = fixture();
        let flow = file(&directory, "Flow.wren-board");
        let before = (std::fs::read(&flow).unwrap(), modified(&flow));
        let _ = app.update(Message::ChooseTheme(Some(definitions::DARK)));
        open_board(&mut app, "Flow");
        let view = app.board.as_ref().unwrap();
        let light = wren_board::drawio::write(
            view.board(),
            &crate::theme::board_palette(definitions::LIGHT),
            "Flow",
        );
        assert_eq!(export_document(view, Format::Drawio, "Flow"), light);

        let asked = app.dispatch(Command::Run(CommandId::ExportDrawio));
        assert_eq!(asked.processed.disposition, Disposition::AsyncPending);
        assert!(!app.command_enabled(CommandId::ExportDrawio));
        let tag = asked.processed.correlation;

        // Inside the workspace an export is a .drawio file.
        let inside = directory.path().join(GROUP).join("Flow.svg");
        let refused = chosen(&mut app, tag, Some(inside.clone()));
        assert_eq!(refused.processed.disposition, Disposition::Blocked);
        assert!(app
            .banner
            .as_deref()
            .unwrap()
            .contains("an export must end in .drawio"));
        assert!(!inside.exists());

        let tag = app
            .dispatch(Command::Run(CommandId::ExportDrawio))
            .processed
            .correlation;
        let target = directory.path().join(GROUP).join("Flow.drawio");
        let done = chosen(&mut app, tag, Some(target.clone()));
        assert_eq!(done.processed.disposition, Disposition::Completed);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), light);
        tick_to(&mut app, 2);
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!((std::fs::read(&flow).unwrap(), modified(&flow)), before);
    }

    #[test]
    fn an_export_waits_for_its_dialog_and_writes_the_board_as_it_was_drawn() {
        let (directory, mut app) = fixture();
        let flow = file(&directory, "Flow.wren-board");
        let before = (std::fs::read(&flow).unwrap(), modified(&flow));
        open_board(&mut app, "Flow");
        lay_out(&mut app);
        let drawn = svg_document(app.board.as_ref().unwrap());
        let canvas = app.board.as_ref().unwrap().palette().canvas;
        let _ = app.open_palette(super::super::palette::Mode::Commands);

        let asked = app.dispatch(Command::Run(CommandId::ExportSvg));
        assert_eq!(asked.processed.disposition, Disposition::AsyncPending);
        assert!(app.palette.is_none());
        let tag = app.board_export.as_ref().unwrap().tag;
        assert_eq!(tag, asked.processed.correlation);
        // One dialog at a time.
        assert!(!app.command_enabled(CommandId::ExportSvg));
        assert_eq!(
            app.export_board(Format::Svg).processed.disposition,
            Disposition::Blocked
        );

        // A theme change while the dialog is open does not change what was
        // asked for.
        let other = if app.definition == definitions::LIGHT {
            definitions::DARK
        } else {
            definitions::LIGHT
        };
        let _ = app.update(Message::ChooseTheme(Some(other)));
        assert_ne!(app.board.as_ref().unwrap().palette().canvas, canvas);

        let out = crate::test_support::tempdir();
        let target = out.path().join("Flow.svg");
        assert_eq!(
            chosen(&mut app, tag + 1, Some(target.clone()))
                .processed
                .disposition,
            Disposition::Blocked,
            "a stale tag"
        );
        let done = chosen(&mut app, tag, Some(target.clone()));
        assert_eq!(done.processed.disposition, Disposition::Completed);
        assert_eq!(done.processed.correlation, tag);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), drawn);
        assert!(drawn.contains(&format!(
            "fill=\"#{:02x}{:02x}{:02x}\"",
            canvas.r, canvas.g, canvas.b
        )));
        assert_eq!(files_in(out.path()), ["Flow.svg"]);
        assert!(app.board_export.is_none());
        assert!(app.command_enabled(CommandId::ExportSvg));

        // An existing file is replaced whole; a cancelled dialog writes
        // nothing.
        let _ = app.dispatch(Command::Run(CommandId::ExportSvg));
        let tag = app.board_export.as_ref().unwrap().tag;
        let again = chosen(&mut app, tag, Some(target.clone()));
        assert_eq!(again.processed.disposition, Disposition::Completed);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            svg_document(app.board.as_ref().unwrap())
        );
        let _ = app.dispatch(Command::Run(CommandId::ExportSvg));
        let tag = app.board_export.as_ref().unwrap().tag;
        let cancelled = chosen(&mut app, tag, None);
        assert_eq!(cancelled.processed.disposition, Disposition::Completed);
        assert_eq!(files_in(out.path()), ["Flow.svg"]);
        assert!(app.banner.is_none());

        // Exporting writes nothing into the board.
        tick_to(&mut app, 2);
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!((std::fs::read(&flow).unwrap(), modified(&flow)), before);
    }

    /// `note.export_png` renders the SVG the board was drawn as when the
    /// command ran: twice the size of the SVG, on the canvas colour.
    #[test]
    fn a_png_export_renders_the_board_as_it_was_drawn() {
        use resvg::tiny_skia::Pixmap;

        let (directory, mut app) = fixture();
        let flow = file(&directory, "Flow.wren-board");
        let before = (std::fs::read(&flow).unwrap(), modified(&flow));
        open_board(&mut app, "Flow");
        lay_out(&mut app);
        let view = app.board.as_ref().unwrap();
        let drawn = svg_document(view);
        let canvas = view.palette().canvas;
        let bounds = view.scene().bounds;
        let fill = view
            .scene()
            .items
            .iter()
            .find_map(|item| match &item.draw {
                wren_board::scene::Draw::Path { fill: Some(fill), .. }
                    if matches!(&item.source, wren_board::scene::Source::Node(id) if id.as_str() == "receive") =>
                {
                    Some(*fill)
                }
                _ => None,
            })
            .expect("the shape is filled");

        let asked = app.dispatch(Command::Run(CommandId::ExportPng));
        assert_eq!(asked.processed.disposition, Disposition::AsyncPending);
        let tag = app.board_export.as_ref().unwrap().tag;
        // One dialog at a time, whatever its format.
        assert!(!app.command_enabled(CommandId::ExportPng));
        assert!(!app.command_enabled(CommandId::ExportSvg));
        let other = if app.definition == definitions::LIGHT {
            definitions::DARK
        } else {
            definitions::LIGHT
        };
        let _ = app.update(Message::ChooseTheme(Some(other)));

        let out = crate::test_support::tempdir();
        let target = out.path().join("Flow.png");
        let done = chosen(&mut app, tag, Some(target.clone()));
        assert_eq!(done.processed.disposition, Disposition::Completed);
        assert_eq!(done.processed.correlation, tag);
        let written = std::fs::read(&target).unwrap();
        assert_eq!(
            written,
            board_export::png(&drawn, board_export::PNG_SCALE).unwrap()
        );
        assert_eq!(files_in(out.path()), ["Flow.png"]);
        assert!(app.board_export.is_none());

        let image = Pixmap::decode_png(&written).unwrap();
        let margin = wren_canvas::FIT_MARGIN;
        let (width, height) = (bounds.width + 2.0 * margin, bounds.height + 2.0 * margin);
        assert_eq!(
            (image.width(), image.height()),
            ((width * 2.0).ceil() as u32, (height * 2.0).ceil() as u32)
        );
        let pixel = |x: f32, y: f32| {
            let column = ((x - bounds.x + margin) * image.width() as f32 / width) as u32;
            let row = ((y - bounds.y + margin) * image.height() as f32 / height) as u32;
            let color = image.pixel(column, row).unwrap().demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        };
        assert_eq!(
            pixel(bounds.x - margin / 2.0, bounds.y - margin / 2.0),
            [canvas.r, canvas.g, canvas.b, 255],
            "the canvas of the theme it was asked in"
        );
        // Inside the shape, left of its label.
        assert_eq!(pixel(4.0, 30.0), [fill.r, fill.g, fill.b, 255]);

        // Inside the workspace a PNG export is a .png file.
        let _ = app.dispatch(Command::Run(CommandId::ExportPng));
        let tag = app.board_export.as_ref().unwrap().tag;
        let refused = chosen(&mut app, tag, Some(file(&directory, "Flow.svg")));
        assert_eq!(refused.processed.disposition, Disposition::Blocked);
        let banner = app.banner.take().unwrap();
        assert!(
            banner.ends_with("inside the workspace an export must end in .png"),
            "{banner}"
        );
        assert!(!file(&directory, "Flow.svg").exists());
        let _ = app.dispatch(Command::Run(CommandId::ExportPng));
        let tag = app.board_export.as_ref().unwrap().tag;
        let inside = file(&directory, "Flow.PNG");
        let done = chosen(&mut app, tag, Some(inside.clone()));
        assert_eq!(done.processed.disposition, Disposition::Completed);
        assert!(Pixmap::decode_png(&std::fs::read(&inside).unwrap()).is_ok());

        // Exporting writes nothing into the board.
        tick_to(&mut app, 2);
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
        assert_eq!((std::fs::read(&flow).unwrap(), modified(&flow)), before);
    }

    /// A board too large for a PNG export is a banner, and nothing is
    /// written.
    #[test]
    fn a_png_export_over_the_pixel_limit_writes_nothing() {
        let (directory, mut app) = fixture();
        std::fs::write(
            file(&directory, "Flow.wren-board"),
            format!("{FLOW}\n[node.far]\nkind = \"shape\"\nat = [6000, 6000]\nsize = [120, 60]\n"),
        )
        .unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        open_board(&mut app, "Flow");
        let _ = app.dispatch(Command::Run(CommandId::ExportPng));
        let tag = app.board_export.as_ref().unwrap().tag;
        let out = crate::test_support::tempdir();
        let outcome = chosen(&mut app, tag, Some(out.path().join("Flow.png")));
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        let banner = app.banner.clone().unwrap();
        assert!(
            banner.starts_with("Could not export the board to "),
            "{banner}"
        );
        assert!(banner.contains("over the 64 megapixels"), "{banner}");
        assert!(files_in(out.path()).is_empty());
        // The SVG of the same board has no such limit.
        let _ = app.dispatch(Command::Run(CommandId::ExportSvg));
        let tag = app.board_export.as_ref().unwrap().tag;
        let outcome = chosen(&mut app, tag, Some(out.path().join("Flow.svg")));
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
    }

    #[test]
    fn an_export_that_cannot_be_written_is_a_banner_and_leaves_nothing() {
        let (directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        let out = crate::test_support::tempdir();
        let folder = out.path().join("folder.svg");
        std::fs::create_dir(&folder).unwrap();
        let locked = out.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let mut targets = vec![
            out.path().join("missing").join("Flow.svg"),
            // The path is a folder: the rename onto it fails.
            folder.clone(),
        ];
        let mut permissions = std::fs::metadata(&locked).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&locked, permissions).unwrap();
        // A superuser can write into a read-only folder.
        if std::fs::write(locked.join("probe"), "").is_err() {
            targets.push(locked.join("Flow.svg"));
        }
        for target in targets {
            let _ = app.dispatch(Command::Run(CommandId::ExportSvg));
            let tag = app.board_export.as_ref().unwrap().tag;
            let outcome = chosen(&mut app, tag, Some(target.clone()));
            assert_eq!(
                outcome.processed.disposition,
                Disposition::Blocked,
                "{target:?}"
            );
            let banner = app.banner.clone().unwrap();
            assert!(
                banner.starts_with("Could not export the board to "),
                "{banner}"
            );
            assert_eq!(outcome.processed.warning.as_ref(), Some(&banner));
            assert!(app.board_export.is_none(), "the export is over");
        }
        // Nothing was left behind, half written or whole.
        assert!(folder.is_dir());
        assert!(std::fs::read_dir(&folder).unwrap().next().is_none());
        assert_eq!(files_in(out.path()), ["folder.svg", "locked"]);
        let mut permissions = std::fs::metadata(&locked).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&locked, permissions).unwrap();
        let _ = std::fs::remove_file(locked.join("probe"));
        assert!(files_in(&locked).is_empty());
        assert_eq!(
            std::fs::read_to_string(file(&directory, "Flow.wren-board")).unwrap(),
            FLOW
        );
    }

    #[test]
    fn an_export_never_replaces_a_note_or_a_file_of_the_workspace() {
        let (directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        lay_out(&mut app);
        let state = directory.path().join(".wren").join("workspace.toml");
        let elsewhere = crate::test_support::tempdir();
        let other_note = elsewhere.path().join("Plan.MD");
        std::fs::write(&other_note, "# Plan\n").unwrap();
        let other_board = elsewhere.path().join("Other.wren-board");
        let targets = [
            // The open board itself, another board and a Markdown note.
            file(&directory, "Flow.wren-board"),
            file(&directory, "Other.wren-board"),
            file(&directory, "Note.md"),
            // A note of another workspace, and one not there yet.
            other_note.clone(),
            other_board.clone(),
            // What Wren keeps in the workspace, and a new file beside a note.
            state.clone(),
            file(&directory, "Flow"),
        ];
        for target in &targets {
            let before = std::fs::read(target)
                .ok()
                .map(|bytes| (bytes, modified(target)));
            let _ = app.dispatch(Command::Run(CommandId::ExportSvg));
            let tag = app.board_export.as_ref().unwrap().tag;
            let outcome = chosen(&mut app, tag, Some(target.clone()));
            assert_eq!(
                outcome.processed.disposition,
                Disposition::Blocked,
                "{target:?}"
            );
            let banner = app.banner.clone().unwrap();
            assert!(banner.starts_with("Did not export the board"), "{banner}");
            assert_eq!(
                std::fs::read(target)
                    .ok()
                    .map(|bytes| (bytes, modified(target))),
                before,
                "{target:?}"
            );
        }
        assert!(!other_board.exists());
        assert_eq!(files_in(elsewhere.path()), ["Plan.MD"]);
        assert_eq!(
            files_in(&directory.path().join(GROUP)),
            [
                "Broken.wren-board",
                "Flow.wren-board",
                "Note.md",
                "Other.wren-board"
            ]
        );
        assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());

        // An SVG file is written, in the workspace or out of it.
        for target in [file(&directory, "Flow.SVG"), elsewhere.path().join("Flow")] {
            app.banner = None;
            let _ = app.dispatch(Command::Run(CommandId::ExportSvg));
            let tag = app.board_export.as_ref().unwrap().tag;
            let outcome = chosen(&mut app, tag, Some(target.clone()));
            assert_eq!(outcome.processed.disposition, Disposition::Completed);
            assert!(app.banner.is_none(), "{target:?}");
            assert!(std::fs::read_to_string(&target).unwrap().contains("<svg"));
        }
    }

    /// §6.8 Phase 1 item 8.
    #[test]
    fn the_export_is_xml_with_a_text_run_per_line_and_escaped_labels() {
        use resvg::usvg::roxmltree;
        let (directory, mut app) = fixture();
        let escaped = r#"a < b & "c""#;
        let board = format!(
            "{CLEAN}\n[node.odd]\ntext = \"{}\"\nkind = \"shape\"\nat = [200, 0]\nsize = [120, 60]\nz = \"d\"\n\n[node.long]\ntext = \"A label long enough to wrap over several lines in its box\\nand a second paragraph\"\nkind = \"shape\"\nat = [200, 200]\nsize = [120, 100]\nz = \"e\"\n",
            escaped.replace('"', "\\\"")
        );
        std::fs::write(file(&directory, "Flow.wren-board"), &board).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        open_board(&mut app, "Flow");
        assert_eq!(app.surface(), Surface::Board);
        let svg = svg_document(app.board.as_ref().unwrap());

        let document = roxmltree::Document::parse(&svg).expect("the export is XML");
        let root = document.root_element();
        assert_eq!(root.tag_name().name(), "svg");
        let runs: Vec<_> = root
            .children()
            .filter(|node| node.has_tag_name("text"))
            .map(|node| node.text().unwrap_or_default().to_owned())
            .collect();
        let lines: Vec<_> = app
            .board
            .as_ref()
            .unwrap()
            .scene()
            .items
            .iter()
            .flat_map(|item| match &item.draw {
                wren_board::scene::Draw::Text { lines, .. } => lines.clone(),
                wren_board::scene::Draw::Path { .. } => Vec::new(),
            })
            .map(|line| line.text)
            .collect();
        assert_eq!(runs, lines);
        assert!(runs.len() > 4, "the long label wraps: {runs:?}");
        assert!(runs.iter().any(|run| run == escaped), "{runs:?}");
        assert!(svg.contains(">a &lt; b &amp; &quot;c&quot;</text>"));
        // The first thing drawn is the canvas behind it all.
        let first = root.children().find(|node| node.is_element()).unwrap();
        assert_eq!(first.tag_name().name(), "rect");
    }
}
