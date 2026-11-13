//! A board's edits on their way to the Session (§3.4 of the board plan):
//! one writer, settling before anything that is not passive, and the
//! text changing underneath the board; with no window, through the
//! messages the board widget and the command paths send.

use wren_canvas::{Button, Command as C, Input, LabelAction, Message as Sent, Tool};
use wren_core::ops::Op;

use super::board::tests::{
    assert_in_step, file, fixture, flow_and, modified, open_board, open_markdown, parses,
    session_text, tick_to, FLOW,
};
use super::board::Surface;
use super::columns::Column;
use super::commands::{Command, CommandId, Disposition};
use super::{App, Message};
use wren_board::Dir;

/// `FLOW` as a person would have written it: the same board, with
/// comments and blank lines the canonical writer does not keep.
fn hand_written() -> String {
    format!(
        "# Drawn by hand.\n\n{}",
        FLOW.replace("[edge.", "# Then:\n[edge.")
    )
}

fn view(app: &App) -> &wren_canvas::BoardView {
    app.board.as_ref().expect("a board is open")
}

fn dirty(app: &App) -> bool {
    let session = app.session.as_ref().unwrap();
    session.document().unwrap().dirty()
}

/// Give the board its widget's size and the keyboard, as its first frame
/// and a click do.
fn lay_out(app: &mut App) {
    let _ = app.update(Message::Board(Sent::Resized {
        width: 800.0,
        height: 600.0,
    }));
    input(app, Input::Focus(true));
}

fn state(app: &App) -> &'static str {
    view(app).machine().state().name()
}

const PRESS: Button = Button::Left;

/// A click at a point of the widget.
fn click_at(app: &mut App, at: (f32, f32)) {
    input(app, Input::Press { button: PRESS, at });
    input(app, Input::Release { button: PRESS });
}

/// A click on the middle of the node `name`.
fn click(app: &mut App, name: &str) {
    let at = middle(app, name);
    click_at(app, at);
}

/// An action on the open label, as its keys send it.
fn label(app: &mut App, action: LabelAction) {
    let epoch = view(app).epoch();
    let _ = app.update(Message::Board(Sent::Label(epoch, action)));
}

/// Open the label of the node `name` and type `text` over it.
fn relabel(app: &mut App, name: &str, text: &str) {
    click(app, name);
    assert_eq!(key(app, C::EditLabel), Disposition::Completed);
    assert_eq!(state(app), "editing");
    label(app, LabelAction::Insert(text.to_owned()));
}

/// A point of the fitted widget that no element is under.
const EMPTY: (f32, f32) = (6.0, 6.0);

/// Click the text tool on empty canvas: free text with no text yet, its
/// label open. Committed as it is, the node is deleted again.
fn start_free_text(app: &mut App) {
    assert_eq!(key(app, C::Tool(Tool::Text)), Disposition::Completed);
    click_at(app, EMPTY);
    assert_eq!(state(app), "editing");
    assert_eq!(view(app).board().nodes.len(), 3);
}

/// A key the board layer binds, as the focused widget publishes it. What
/// `dispatch` made of it is the disposition.
fn key(app: &mut App, command: C) -> Disposition {
    let serial = app.action_serial;
    let _ = app.update(Message::Board(Sent::Command(command)));
    assert_ne!(app.action_serial, serial, "the key went through dispatch");
    app.last_processed.as_ref().unwrap().disposition
}

/// Input from the widget, meant for the board as it is now.
fn input(app: &mut App, input: Input) {
    let epoch = view(app).epoch();
    let _ = app.update(Message::Board(Sent::Input(epoch, input)));
}

/// Where the middle of the node `name` is in the widget.
fn middle(app: &App, name: &str) -> (f32, f32) {
    let view = view(app);
    let id = wren_board::Id::new(name).unwrap();
    let placed = wren_board::placements(view.board());
    let (at, node) = (placed[&id].at, &view.board().nodes[&id]);
    view.viewport().to_screen((
        at[0] as f32 + node.size[0] as f32 / 2.0,
        at[1] as f32 + node.size[1] as f32 / 2.0,
    ))
}

fn at(app: &App, name: &str) -> [i32; 2] {
    view(app).board().nodes[&wren_board::Id::new(name).unwrap()].at
}

/// The lines of `before` that `after` lacks, and those it gained.
fn line_diff<'a>(before: &'a str, after: &'a str) -> (Vec<&'a str>, Vec<&'a str>) {
    let gone = before
        .lines()
        .filter(|line| !after.lines().any(|l| l == *line));
    let new = after
        .lines()
        .filter(|line| !before.lines().any(|l| l == *line));
    (gone.collect(), new.collect())
}

#[test]
fn a_board_key_is_one_draft_through_dispatch() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    open_board(&mut app, "Flow");
    lay_out(&mut app);

    // Selecting is looking: no draft, and the document stays clean.
    assert_eq!(key(&mut app, C::SelectAll), Disposition::Completed);
    assert_eq!(key(&mut app, C::Select(Dir::Down)), Disposition::Completed);
    assert_eq!(key(&mut app, C::SelectAll), Disposition::Completed);
    assert_eq!((app.board_revision, session_text(&app).as_str()), (0, FLOW));
    assert!(!dirty(&app));

    // A move is one draft, which the Session holds at once.
    assert_eq!(key(&mut app, C::Move(Dir::Right)), Disposition::Completed);
    assert_eq!(app.board_revision, 1);
    let moved = session_text(&app);
    assert_eq!(
        line_diff(FLOW, &moved),
        (
            vec!["at = [0, 200]", "at = [0, 0]"],
            vec!["at = [20, 200]", "at = [20, 0]"]
        ),
        "only the nodes' places changed, and no connector line"
    );
    assert!(dirty(&app));
    // The view's own draft is not a change underneath it.
    assert_in_step(&app, Surface::Board);
    assert_eq!(parses(&app), 1);
    assert!(view(&app).machine().selection().len() == 3);

    // Autosave writes it, and the board hears nothing new.
    tick_to(&mut app, 2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), moved);
    assert!(!dirty(&app));
    assert_eq!((app.board_revision, parses(&app)), (1, 1));

    // The inverse move drafts the opening text again, byte for byte.
    assert_eq!(key(&mut app, C::Move(Dir::Left)), Disposition::Completed);
    assert_eq!((app.board_revision, session_text(&app).as_str()), (2, FLOW));
    tick_to(&mut app, 4);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), FLOW);
}

#[test]
fn a_hand_written_board_is_rewritten_only_by_an_edit_that_survives() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Hand.wren-board");
    let text = hand_written();
    std::fs::write(&path, &text).unwrap();
    let _ = app.apply_core(Op::Reconcile).unwrap();
    let saved = modified(&path);
    open_board(&mut app, "Hand");
    lay_out(&mut app);

    // Selecting, walking, arming and disarming a tool, zooming and fitting
    // send nothing.
    for command in [
        C::Select(Dir::Down),
        C::ExtendSelection(Dir::Up),
        C::Tool(wren_canvas::Tool::Hand),
        C::Tool(wren_canvas::Tool::Select),
        C::ZoomFit,
        C::ZoomSelection,
        C::SelectNone,
        C::SelectAll,
    ] {
        assert_eq!(key(&mut app, command), Disposition::Completed);
    }
    assert_eq!((app.board_revision, session_text(&app)), (0, text.clone()));

    // An edit sends the canonical form, once.
    key(&mut app, C::Move(Dir::Down));
    let canonical = session_text(&app);
    assert!(!canonical.contains('#') && canonical != text);
    assert!(dirty(&app));
    // Moved back before autosave, the draft is the file's own bytes and
    // the document is clean, so nothing is written.
    key(&mut app, C::Move(Dir::Up));
    assert_eq!(session_text(&app), text);
    assert!(!dirty(&app));
    tick_to(&mut app, 2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert_eq!(modified(&path), saved);
    assert_in_step(&app, Surface::Board);
}

#[test]
fn a_drag_drafts_once_on_release_and_never_when_it_is_cancelled() {
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    let zoom = view(&app).viewport().zoom;
    let from = middle(&app, "pack");
    let to = (from.0 + 60.0 * zoom, from.1);
    let press = Input::Press {
        button: Button::Left,
        at: from,
    };
    let release = Input::Release {
        button: Button::Left,
    };

    app.column = Column::Notes;
    input(&mut app, press);
    assert_eq!(app.column, Column::Editor, "a press is the board's");
    input(&mut app, Input::Move { at: to });
    assert_eq!(view(&app).machine().state().name(), "moving");
    // In flight the board is as it was at the press, and so is the draft.
    assert_eq!(at(&app, "pack"), [0, 200]);
    assert_eq!((app.board_revision, session_text(&app).as_str()), (0, FLOW));
    input(
        &mut app,
        Input::Key {
            key: wren_canvas::Key::Escape,
            repeat: false,
        },
    );
    input(&mut app, release);
    assert_eq!(view(&app).machine().state().name(), "idle");
    assert_eq!((app.board_revision, session_text(&app).as_str()), (0, FLOW));
    assert!(!dirty(&app));

    // Released, it is one draft: one line, that node's place.
    input(&mut app, press);
    input(&mut app, Input::Move { at: to });
    assert_eq!(app.board_revision, 0);
    input(&mut app, release);
    assert_eq!(at(&app, "pack"), [60, 200]);
    assert_eq!(app.board_revision, 1);
    assert_eq!(
        line_diff(FLOW, &session_text(&app)),
        (vec!["at = [0, 200]"], vec!["at = [60, 200]"])
    );
    assert_in_step(&app, Surface::Board);
}

#[test]
fn a_board_under_an_overlay_takes_only_what_looks_or_ends() {
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    key(&mut app, C::SelectAll);
    let press = Input::Press {
        button: Button::Left,
        at: (2.0, 2.0),
    };
    let blocked = |app: &mut App| {
        let serial = app.action_serial;
        let epoch = view(app).epoch();
        let before = view(app).viewport();
        for message in [
            Sent::Command(C::Move(Dir::Right)),
            Sent::Command(C::SelectNone),
            Sent::Input(epoch, press),
            Sent::Input(epoch, Input::Space { held: true }),
            Sent::Label(epoch, wren_canvas::LabelAction::Insert("x".into())),
            Sent::Pasted(epoch, "x".into()),
        ] {
            let _ = app.update(Message::Board(message));
        }
        assert_eq!(app.action_serial, serial, "no command ran");
        assert_eq!(view(app).machine().selection().len(), 3);
        assert!(!view(app).machine().space());
        assert_eq!((app.board_revision, session_text(app).as_str()), (0, FLOW));
        // The wheel still looks, and a focus change still arrives.
        input(
            app,
            Input::Wheel {
                delta: wren_canvas::Scroll::Lines { x: 0.0, y: -1.0 },
                at: (100.0, 100.0),
                zoom: false,
                sideways: false,
                scale_factor: 1.0,
            },
        );
        assert_ne!(view(app).viewport(), before);
        input(app, Input::Focus(true));
        assert!(view(app).machine().focused());
        input(app, Input::Focus(false));
        assert!(!view(app).machine().focused());
    };

    app.column = Column::Notes;
    let _ = app.open_palette(super::palette::Mode::Commands);
    blocked(&mut app);
    let _ = app.close_palette();
    let _ = app.show_dialog(super::workspace::DialogKind::CreateSection);
    assert!(app.dialog.is_some());
    blocked(&mut app);
    assert_eq!(app.column, Column::Notes, "nothing took the column");
    app.dialog = None;

    // With neither open the same press deselects.
    input(&mut app, press);
    input(
        &mut app,
        Input::Release {
            button: Button::Left,
        },
    );
    assert!(view(&app).machine().selection().is_empty());
    assert_eq!(app.column, Column::Editor);
}

#[test]
fn input_for_another_board_or_an_earlier_one_is_dropped() {
    let (directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    let flow = view(&app).epoch();
    key(&mut app, C::SelectAll);

    // Another board opens while a click for the first is still on its way:
    // with a tool armed, the click would make a shape there.
    open_board(&mut app, "Other");
    lay_out(&mut app);
    assert_ne!(view(&app).epoch(), flow);
    let rectangle = C::Tool(wren_canvas::Tool::Shape(wren_board::Shape::Rect));
    assert_eq!(key(&mut app, rectangle), Disposition::Completed);
    let click = [
        Input::Press {
            button: Button::Left,
            at: (300.0, 300.0),
        },
        Input::Release {
            button: Button::Left,
        },
    ];
    app.column = Column::Notes;
    for step in click {
        let _ = app.update(Message::Board(Sent::Input(flow, step)));
    }
    let _ = app.update(Message::Board(Sent::Pasted(flow, "stale".into())));
    assert!(view(&app).board().nodes.is_empty());
    assert_eq!(app.column, Column::Notes);
    assert_eq!(app.board_revision, 0);
    // The same click meant for this board makes the shape.
    for step in click {
        input(&mut app, step);
    }
    assert_eq!(view(&app).board().nodes.len(), 1);
    assert_eq!(app.board_revision, 1);

    // A reload gives the same view another epoch.
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    let before = view(&app).epoch();
    std::fs::write(file(&directory, "Flow.wren-board"), flow_and("New")).unwrap();
    let _ = app.apply_core(Op::Reconcile).unwrap();
    assert_ne!(view(&app).epoch(), before);
    let from = middle(&app, "pack");
    let _ = app.update(Message::Board(Sent::Input(
        before,
        Input::Press {
            button: Button::Left,
            at: from,
        },
    )));
    assert!(view(&app).machine().selection().is_empty());

    // With no board open a message has nowhere to go.
    open_markdown(&mut app, "Note");
    let serial = app.action_serial;
    let _ = app.update(Message::Board(Sent::Command(C::SelectAll)));
    assert_eq!(app.action_serial, serial);
    assert_in_step(&app, Surface::Text);
}

#[test]
fn a_board_key_reads_the_keymap_the_app_reads() {
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::{Key, Modifiers};
    use wren_canvas::KeyRoute;
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    // The view asks the policy the app gave it; its widget is not public,
    // so the policy is read as the widget reads it.
    let route = |app: &App, key: Key, code: Code| {
        let policy = super::preferences::board_policy(&app.preferences.keymap);
        policy(&key, Physical::Code(code), Modifiers::empty(), false)
    };
    let letter = |c: &str| Key::Character(c.into());
    assert_eq!(
        route(&app, letter("r"), Code::KeyR),
        KeyRoute::Command(C::Tool(wren_canvas::Tool::Shape(wren_board::Shape::Rect)))
    );
    assert_eq!(
        route(&app, Key::Named(Named::Tab), Code::Tab),
        KeyRoute::Command(C::ConnectNext)
    );

    // A settings reload reaches the open board: `x` now deletes.
    let directory = crate::test_support::tempdir();
    let path = directory.path().join("config.conf");
    std::fs::write(&path, "boards = on\nkeybind = x = board.delete\n").unwrap();
    app.preferences = super::preferences::Preferences::load(Some(path));
    app.preferences.boards = true;
    app.apply_preferences();
    assert_eq!(
        route(&app, letter("x"), Code::KeyX),
        KeyRoute::Command(C::Delete)
    );
}

/// Exit criterion 3 of Phase 2: a board's draft has one sender. Outside
/// the tests, the app builds `Op::EditDraft` in two places: the editor's
/// `text_changed`, which returns early on a board, and `board_changed`.
#[test]
fn a_draft_is_built_only_by_the_editor_s_writer_and_the_board_s() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    let mut folders = vec![source.clone()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let name = path
                .strip_prefix(&source)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if name.ends_with("_tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // Each file's inline tests come last.
            let code = text.split("#[cfg(test)]\nmod tests").next().unwrap();
            let code = code
                .split("#[cfg(test)]\npub(super) mod tests")
                .next()
                .unwrap();
            let builds = code
                .lines()
                .filter(|line| line.contains("Op::EditDraft {") && !line.contains(".."))
                .count();
            if builds > 0 {
                found.push((name.replace('\\', "/"), builds));
            }
        }
    }
    found.sort();
    assert_eq!(
        found,
        [("app.rs".to_owned(), 1), ("app/board.rs".to_owned(), 1)]
    );
}

/// Each way of leaving a board that is not passive, by what the user does.
type Leave = (&'static str, fn(&mut App));

const LEAVES: [Leave; 9] = [
    ("another note", |app| {
        open_markdown(app, "Note");
        open_board(app, "Flow");
    }),
    ("a flush", |app| {
        let _ = app.apply_core(Op::Flush).unwrap();
    }),
    ("back and forward", |app| {
        let _ = app.dispatch(Command::Run(CommandId::Back));
        assert_eq!(app.surface(), Surface::Text);
        let _ = app.dispatch(Command::Run(CommandId::Forward));
    }),
    ("a rename dialog", |app| {
        let _ = app.dispatch(Command::Run(CommandId::RenameNote));
        assert!(app.dialog.take().is_some());
    }),
    ("the palette", |app| {
        let _ = app.dispatch(Command::Run(CommandId::CommandPalette));
        assert!(app.palette.take().is_some());
    }),
    ("the window losing focus", |app| {
        app.window_unfocused().unwrap();
    }),
    ("quitting", |app| {
        let _ = app.request_quit().unwrap();
    }),
    ("a pane width", |app| {
        let _ = app.dispatch(Command::Width {
            pane: crate::panes::Pane::Notes,
            width: 300.0,
        });
    }),
    ("a reconcile asked for", |app| {
        let _ = app.apply_core(Op::Reconcile).unwrap();
    }),
];

#[test]
fn the_board_settles_before_anything_that_is_not_passive() {
    for (name, leave) in LEAVES {
        let (directory, mut app) = fixture();
        let path = file(&directory, "Flow.wren-board");
        let saved = modified(&path);
        open_markdown(&mut app, "Note");
        open_board(&mut app, "Flow");
        lay_out(&mut app);

        // An open label is committed: free text left empty is deleted, the
        // board is as it was opened, and nothing was written.
        start_free_text(&mut app);
        assert_eq!(app.board_revision, 1, "{name}");
        assert!(dirty(&app), "{name}");
        // What is passive leaves the label open.
        let _ = app.update(Message::Tick);
        assert_eq!(state(&app), "editing", "{name}");
        leave(&mut app);
        assert_ne!(state(&app), "editing", "{name}");
        assert!(view(&app).label().is_none(), "{name}");
        assert_eq!(view(&app).board().nodes.len(), 2, "{name}");
        assert_eq!(session_text(&app), FLOW, "{name}");
        assert!(!dirty(&app), "{name}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FLOW, "{name}");
        assert_eq!(modified(&path), saved, "{name}");
        assert_in_step(&app, Surface::Board);

        // A drag in flight is cancelled with the board as it was pressed,
        // and its release afterwards does nothing.
        let from = middle(&app, "pack");
        input(
            &mut app,
            Input::Press {
                button: PRESS,
                at: from,
            },
        );
        input(
            &mut app,
            Input::Move {
                at: (from.0 + 80.0, from.1),
            },
        );
        assert_eq!(state(&app), "moving", "{name}");
        let revision = app.board_revision;
        leave(&mut app);
        assert!(!view(&app).machine().dragging(), "{name}");
        input(&mut app, Input::Release { button: PRESS });
        assert_eq!(at(&app, "pack"), [0, 200], "{name}");
        assert_eq!(session_text(&app), FLOW, "{name}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FLOW, "{name}");
        // Reopening the note made a new view, whose count starts again.
        if !matches!(name, "another note" | "back and forward") {
            assert_eq!(app.board_revision, revision, "{name}");
        }
    }
}

#[test]
fn a_label_typed_a_moment_ago_is_saved_by_whatever_comes_next() {
    for (name, leave) in LEAVES {
        let (directory, mut app) = fixture();
        let path = file(&directory, "Flow.wren-board");
        open_markdown(&mut app, "Note");
        open_board(&mut app, "Flow");
        lay_out(&mut app);
        relabel(&mut app, "pack", "Ship");
        // Each change of the text is the draft at once.
        assert!(session_text(&app).contains("text = \"Ship\""), "{name}");
        leave(&mut app);
        assert_ne!(state(&app), "editing", "{name}");
        let flushes = !matches!(
            name,
            "a rename dialog" | "the palette" | "a pane width" | "a reconcile asked for"
        );
        if flushes {
            let on_disk = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                line_diff(FLOW, &on_disk),
                (vec!["text = \"Pack\""], vec!["text = \"Ship\""]),
                "{name}"
            );
        } else {
            assert!(dirty(&app), "{name}");
        }
        assert!(session_text(&app).contains("text = \"Ship\""), "{name}");
    }
}

#[test]
fn a_failed_flush_blocks_the_note_switch_with_the_label_committed_and_the_draft_kept() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    let note = super::board::tests::note_id(&app, "Note", wren_core::kind::NoteKind::Markdown);
    let flow = open_board(&mut app, "Flow");
    lay_out(&mut app);
    relabel(&mut app, "pack", "Ship");
    let typed = session_text(&app);

    app.session
        .as_mut()
        .unwrap()
        .fail_next(wren_core::fs_store::Fault::Write);
    let blocked = app.apply_core(Op::OpenNote { note: Some(note) });
    assert!(blocked.is_err());
    assert_eq!(app.editor_note, Some(flow));
    assert_in_step(&app, Surface::Board);
    assert_ne!(state(&app), "editing", "the label was committed");
    assert_eq!(session_text(&app), typed);
    assert!(dirty(&app));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), FLOW);

    // The next try saves it.
    let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
    assert_in_step(&app, Surface::Text);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), typed);
}

#[test]
fn an_overlay_or_a_blur_puts_the_tool_away_and_an_operation_does_not() {
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    let diamond = C::Tool(Tool::Shape(wren_board::Shape::Diamond));
    let armed = |app: &mut App| {
        assert_eq!(key(app, diamond), Disposition::Completed);
        assert_eq!(state(app), "armed");
    };

    armed(&mut app);
    let _ = app.apply_core(Op::Flush).unwrap();
    let _ = app.update(Message::Tick);
    assert_eq!(
        view(&app).machine().tool(),
        Tool::Shape(wren_board::Shape::Diamond)
    );

    let _ = app.open_palette(super::palette::Mode::Commands);
    assert_eq!(view(&app).machine().tool(), Tool::Select);
    let _ = app.close_palette();

    armed(&mut app);
    let _ = app.show_dialog(super::workspace::DialogKind::CreateSection);
    assert_eq!(view(&app).machine().tool(), Tool::Select);
    app.dialog = None;

    armed(&mut app);
    app.window_unfocused().unwrap();
    assert_eq!(view(&app).machine().tool(), Tool::Select);
    assert_eq!((app.board_revision, session_text(&app).as_str()), (0, FLOW));
}

/// Write `contents` to `path` from outside and let the app find out as it
/// does in a window: ticks, a scan off the app, and its result, all of
/// them passive, so nothing settles the board first.
fn change_on_disk(app: &mut App, path: &std::path::Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    let session = app.session.as_mut().unwrap();
    session.set_background_reconcile(true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while session_text(app) != contents {
        // `Message::Tick` would start the scan on a thread of its own.
        let _ = app.process_core(Op::Tick);
        let job = app.session.as_mut().unwrap().take_reconcile_job();
        if let Some(job) = job {
            let _ = app.update(Message::Reconciled(job.run()));
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the change on disk never reached the Session"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn selected(app: &App) -> Vec<&str> {
    let selection = view(app).machine().selection();
    selection.iter().map(wren_board::Id::as_str).collect()
}

fn wheel() -> Input {
    Input::Wheel {
        delta: wren_canvas::Scroll::Lines { x: 0.0, y: -3.0 },
        at: (100.0, 100.0),
        zoom: false,
        sideways: false,
        scale_factor: 1.0,
    }
}

#[test]
fn a_change_on_disk_reloads_a_clean_board_and_keeps_the_selection_and_the_view() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    click(&mut app, "pack");
    key(&mut app, C::ExtendSelection(Dir::Up));
    input(&mut app, wheel());
    let (viewport, epoch) = (view(&app).viewport(), view(&app).epoch());
    assert_eq!(selected(&app), ["pack", "receive"]);

    change_on_disk(&mut app, &path, &flow_and("New"));
    assert_in_step(&app, Surface::Board);
    assert_eq!(view(&app).board().nodes.len(), 3);
    assert_eq!(selected(&app), ["pack", "receive"]);
    assert_eq!(view(&app).viewport(), viewport);
    assert_ne!(view(&app).epoch(), epoch);
    assert_eq!(app.board_revision, 1, "a reload counts");
    assert_eq!(state(&app), "idle");
    assert!(!dirty(&app));
    assert!(view(&app).pristine());

    // What is no longer on the board leaves the selection.
    let without_pack = "schema = 1\n\n[node.receive]\ntext = \"Receive order\"\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\n";
    change_on_disk(&mut app, &path, without_pack);
    assert_eq!(selected(&app), ["receive"]);
    assert_eq!(view(&app).viewport(), viewport);
    assert_eq!(app.board_revision, 2);
}

#[test]
fn a_change_on_disk_under_a_dirty_board_keeps_the_draft_in_a_copy_of_its_kind() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    // A label typed a moment ago, its editor still open.
    relabel(&mut app, "pack", "Ship");
    let draft = session_text(&app);
    assert!(dirty(&app));

    let external = flow_and("External");
    change_on_disk(&mut app, &path, &external);
    let copy = app.conflict_copy.clone().expect("a conflict copy").1;
    assert!(copy
        .as_path()
        .to_string_lossy()
        .ends_with(wren_core::kind::NoteKind::Board.extension()));
    let copy = directory.path().join(copy.as_path());
    assert_eq!(std::fs::read_to_string(copy).unwrap(), draft);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), external);

    // The board is the one on disk. The label closed without a commit and
    // is not typed again over the reloaded board.
    assert_in_step(&app, Surface::Board);
    assert!(view(&app).label().is_none());
    assert_eq!(state(&app), "idle");
    let pack = wren_board::Id::new("pack").unwrap();
    assert_eq!(view(&app).board().nodes[&pack].text, "Pack");
    assert_eq!(view(&app).board().nodes.len(), 3);
    assert!(!dirty(&app));
    assert_eq!(selected(&app), ["pack"]);
    // What the label's keys sent for the board before is dropped.
    label(&mut app, LabelAction::Insert("late".into()));
    assert_eq!(session_text(&app), external);
}

#[test]
fn a_change_on_disk_cancels_a_gesture_in_flight() {
    for was_dirty in [false, true] {
        let (directory, mut app) = fixture();
        let path = file(&directory, "Flow.wren-board");
        open_board(&mut app, "Flow");
        lay_out(&mut app);
        if was_dirty {
            click(&mut app, "receive");
            key(&mut app, C::Move(Dir::Right));
            assert!(dirty(&app));
        }
        let draft = session_text(&app);
        let from = middle(&app, "pack");
        input(
            &mut app,
            Input::Press {
                button: PRESS,
                at: from,
            },
        );
        input(
            &mut app,
            Input::Move {
                at: (from.0 + 90.0, from.1 + 40.0),
            },
        );
        assert_eq!(state(&app), "moving");
        let revision = app.board_revision;

        let external = flow_and("External");
        change_on_disk(&mut app, &path, &external);
        assert_eq!(state(&app), "idle", "dirty: {was_dirty}");
        assert!(!view(&app).machine().dragging());
        // The button comes up afterwards, on a board that holds no drag.
        input(&mut app, Input::Release { button: PRESS });
        assert_eq!(at(&app, "pack"), [0, 200]);
        assert_eq!(session_text(&app), external);
        assert_eq!(app.board_revision, revision + 1, "only the reload");
        assert!(!dirty(&app));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
        match &app.conflict_copy {
            // The copy is the draft as it was before the drag began.
            Some((_, copy)) => {
                assert!(was_dirty);
                let copy = directory.path().join(copy.as_path());
                assert_eq!(std::fs::read_to_string(copy).unwrap(), draft);
            }
            None => assert!(!was_dirty),
        }
    }
}

#[test]
fn what_a_board_may_do_is_decided_again_whenever_its_text_changes_underneath() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    let moves = CommandId::Board(C::Move(Dir::Right));
    let drag = |app: &mut App| {
        let from = middle(app, "pack");
        let to = (from.0 + 90.0, from.1);
        input(
            app,
            Input::Press {
                button: PRESS,
                at: from,
            },
        );
        input(app, Input::Move { at: to });
        input(app, Input::Release { button: PRESS });
    };
    assert!(app.command_enabled(moves));

    // Read-only: a format this build does not know. It selects, walks,
    // zooms, pans and exports, and nothing changes it.
    let locked = FLOW.replacen("schema = 1\n", "schema = 1\nrequires = [\"frames\"]\n", 1);
    change_on_disk(&mut app, &path, &locked);
    assert_in_step(&app, Surface::Board);
    assert!(!view(&app).writable());
    assert!(!app.command_enabled(moves));
    click(&mut app, "pack");
    assert_eq!(selected(&app), ["pack"]);
    assert_eq!(key(&mut app, C::Select(Dir::Up)), Disposition::Completed);
    assert_eq!(selected(&app), ["receive"]);
    assert_eq!(key(&mut app, C::SelectAll), Disposition::Completed);
    assert_eq!(key(&mut app, C::ZoomSelection), Disposition::Completed);
    assert_eq!(key(&mut app, C::ZoomFit), Disposition::Completed);
    let before = view(&app).viewport();
    input(&mut app, wheel());
    assert_ne!(view(&app).viewport(), before);
    assert!(app.command_enabled(CommandId::ExportSvg));
    for command in [C::Move(Dir::Right), C::Delete, C::EditLabel, C::ConnectNext] {
        assert_eq!(key(&mut app, command), Disposition::Blocked, "{command:?}");
    }
    drag(&mut app);
    label(&mut app, LabelAction::Insert("x".into()));
    let epoch = view(&app).epoch();
    let _ = app.update(Message::Board(Sent::Pasted(epoch, "pasted".into())));
    assert_eq!(session_text(&app), locked);
    assert_eq!(at(&app, "pack"), [0, 200]);
    assert_eq!(view(&app).board().nodes.len(), 2);
    assert!(!dirty(&app));

    // Raw: text that is not a board. No board command runs and the
    // board's messages have nowhere to go.
    let broken = super::board::tests::conflicted();
    change_on_disk(&mut app, &path, &broken);
    assert_in_step(&app, Surface::Raw);
    assert!(!app.command_enabled(moves));
    let serial = app.action_serial;
    let _ = app.update(Message::Board(Sent::Command(C::SelectAll)));
    let _ = app.update(Message::Board(Sent::Pasted(epoch, "pasted".into())));
    assert_eq!(app.action_serial, serial);
    assert_eq!(session_text(&app), broken);

    // Writable again: the same drag is one draft.
    change_on_disk(&mut app, &path, FLOW);
    assert_in_step(&app, Surface::Board);
    lay_out(&mut app);
    assert!(view(&app).writable());
    assert!(app.command_enabled(moves));
    assert_eq!(app.board_revision, 0, "a new view");
    drag(&mut app);
    assert_ne!(at(&app, "pack"), [0, 200]);
    assert_eq!(app.board_revision, 1);
    assert!(dirty(&app));
}
