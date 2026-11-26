//! A board's edits on their way to the Session (§3.4 of the board plan):
//! one writer, settling before anything that is not passive, the text
//! changing underneath the board, undo and the clipboard; with no window,
//! through the messages the board widget and the command paths send.

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
    let epoch = view(app).epoch();
    let _ = app.update(Message::Board(Sent::Command(epoch, command)));
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

/// §5.16 rule 1 through the app: a colour or fill key is one draft of
/// one line per element, a value already held or nothing selected sends no
/// draft, and undo drafts the opening bytes again.
#[test]
fn a_colour_or_fill_key_is_one_draft_and_a_value_already_held_none() {
    use wren_board::{Fill, Token};
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    lay_out(&mut app);

    assert_eq!(key(&mut app, C::Color(Token::Red)), Disposition::Completed);
    assert_eq!(app.board_revision, 0, "nothing selected, no draft");
    click(&mut app, "pack");
    assert_eq!(key(&mut app, C::Color(Token::Red)), Disposition::Completed);
    assert_eq!(app.board_revision, 1);
    let red = session_text(&app);
    assert_eq!(
        line_diff(FLOW, &red),
        (vec![], vec!["style.color = \"red\""])
    );
    assert!(dirty(&app));
    assert_eq!(key(&mut app, C::Color(Token::Red)), Disposition::Completed);
    assert_eq!(app.board_revision, 1, "already red, no draft");

    assert_eq!(key(&mut app, C::SelectAll), Disposition::Completed);
    assert_eq!(
        run(&mut app, C::Fill(Fill::Solid)),
        (Disposition::Completed, 0)
    );
    assert_eq!(app.board_revision, 2);
    let solid = session_text(&app);
    assert_eq!(
        line_diff(&red, &solid),
        (vec![], vec!["style.fill = \"solid\""; 2]),
        "the two shapes, and not the connector"
    );

    for text in [red.as_str(), FLOW] {
        assert_eq!(key(&mut app, C::Undo), Disposition::Completed);
        assert_eq!(session_text(&app), text);
    }
    assert_eq!(app.board_revision, 4);
    assert_in_step(&app, Surface::Board);
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
            Sent::Command(epoch, C::Move(Dir::Right)),
            Sent::Command(epoch, C::SelectNone),
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
    // A key meant for the first board, pasting into this one.
    let serial = app.action_serial;
    let _ = app.update(Message::Board(Sent::Command(flow, C::Paste)));
    assert_eq!(app.action_serial, serial, "a stale key reaches no command");
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
    let _ = app.update(Message::Board(Sent::Command(1, C::SelectAll)));
    assert_eq!(app.action_serial, serial);
    assert_in_step(&app, Surface::Text);
}

/// A letter pressed on the focused board, as the widget publishes it.
fn press(epoch: u64, letter: &str) -> Message {
    use iced::keyboard::key::{Code, Physical};
    Message::Board(Sent::Key(
        epoch,
        wren_canvas::KeyPress {
            key: iced::keyboard::Key::Character(letter.into()),
            physical: Physical::Code(Code::KeyQ),
            modifiers: iced::keyboard::Modifiers::empty(),
            text: Some(letter.to_owned()),
            repeat: false,
            held: true,
        },
    ))
}

#[test]
fn a_board_key_is_read_when_it_is_applied_and_a_bound_one_goes_through_dispatch() {
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    click(&mut app, "pack");
    // Both published before either is applied, as two quick keys are.
    let epoch = view(&app).epoch();
    let (c, h) = (press(epoch, "c"), press(epoch, "h"));
    let serial = app.action_serial;
    let _ = app.update(c);
    assert_ne!(app.action_serial, serial, "C went through dispatch");
    let serial = app.action_serial;
    let _ = app.update(h);
    assert_eq!(app.action_serial, serial, "H is the connect query's");
    let seeking = view(&app).machine().seeking();
    assert_eq!(seeking.map(|seeking| seeking.query), Some("h"));
    assert_eq!(app.column, Column::Editor);

    // A key meant for the board before a reload, or under a palette, does
    // nothing.
    let _ = app.open_palette(super::palette::Mode::Commands);
    let serial = app.action_serial;
    let _ = app.update(press(view(&app).epoch(), "c"));
    assert_eq!(app.action_serial, serial);
    let _ = app.close_palette();
    let serial = app.action_serial;
    let _ = app.update(press(epoch.wrapping_sub(1), "c"));
    assert_eq!(app.action_serial, serial);
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

/// A chord pressed on the focused board, as the widget publishes it; an
/// arrow is named by `"left"`, `"right"`, `"up"` or `"down"`.
fn chord(app: &mut App, name: &str, modifiers: iced::keyboard::Modifiers) {
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::Key;
    let (key, code) = match name {
        "left" => (Key::Named(Named::ArrowLeft), Code::ArrowLeft),
        "right" => (Key::Named(Named::ArrowRight), Code::ArrowRight),
        "up" => (Key::Named(Named::ArrowUp), Code::ArrowUp),
        "down" => (Key::Named(Named::ArrowDown), Code::ArrowDown),
        "a" => (Key::Character("a".into()), Code::KeyA),
        "c" => (Key::Character("c".into()), Code::KeyC),
        "d" => (Key::Character("d".into()), Code::KeyD),
        "u" => (Key::Character("u".into()), Code::KeyU),
        "v" => (Key::Character("v".into()), Code::KeyV),
        "x" => (Key::Character("x".into()), Code::KeyX),
        "y" => (Key::Character("y".into()), Code::KeyY),
        "z" => (Key::Character("z".into()), Code::KeyZ),
        _ => panic!("no key {name}"),
    };
    let epoch = view(app).epoch();
    let press = wren_canvas::KeyPress {
        key,
        physical: Physical::Code(code),
        modifiers,
        text: None,
        repeat: false,
        held: true,
    };
    let _ = app.update(Message::Board(Sent::Key(epoch, press)));
}

/// Select `pack` alone, by a click on empty canvas and one on it, which
/// is no double click.
fn reselect(app: &mut App) {
    click_at(app, EMPTY);
    click(app, "pack");
}

/// discovery/keybindings.md §2 items 1, 3 and 4, end to end through the
/// app in each scheme: on a board Mod+arrow connects and the other key
/// with an arrow moves, Mod+D duplicates, Mod+A selects all, Mod+C, X and
/// V are the clipboard's, and Mod+Z and Mod+Shift+Z undo and redo, on the
/// board and in an open label. In Alt mode the Ctrl versions of the
/// clipboard, select all and undo keys work too. Ctrl+Y redoes nothing.
#[test]
fn the_board_and_its_label_keys_follow_the_command_key_in_each_scheme() {
    use iced::keyboard::Modifiers;
    use wren_core::keybind::{LinuxModifier, Logical, Platform};
    let held = |scheme: wren_core::keybind::Scheme, token| match scheme.modifier(token) {
        "super" => Modifiers::LOGO,
        "ctrl" => Modifiers::CTRL,
        _ => Modifiers::ALT,
    };
    for scheme in super::preferences::SCHEMES {
        let (modifier, opt) = (held(scheme, Logical::Mod), held(scheme, Logical::Opt));
        let shifted = modifier | Modifiers::SHIFT;
        let alt_mode = scheme.platform == Platform::Linux && scheme.linux == LinuxModifier::Alt;
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        app.preferences.keymap = super::preferences::build_keymap(scheme, &[], false).0;
        app.apply_preferences();
        lay_out(&mut app);
        let nodes = |app: &App| view(app).board().nodes.len();
        let edges = |app: &App| view(app).board().edges.len();

        // On the board.
        reselect(&mut app);
        chord(&mut app, "d", modifier);
        assert_eq!(nodes(&app), 3, "{scheme:?}: Mod+D duplicates");
        chord(&mut app, "z", modifier);
        assert_eq!(nodes(&app), 2, "{scheme:?}: Mod+Z undoes");
        chord(&mut app, "z", shifted);
        assert_eq!(nodes(&app), 3, "{scheme:?}: Mod+Shift+Z redoes");
        chord(&mut app, "z", modifier);
        let revision = app.board_revision;
        for ctrl_y in [Modifiers::CTRL, modifier] {
            chord(&mut app, "y", ctrl_y);
            assert_eq!(nodes(&app), 2, "{scheme:?}: {ctrl_y:?}+Y redoes nothing");
        }
        assert_eq!(app.board_revision, revision, "{scheme:?}");
        reselect(&mut app);
        let before = at(&app, "pack");
        chord(&mut app, "right", opt);
        assert_ne!(at(&app, "pack"), before, "{scheme:?}: Opt+→ moves");
        chord(&mut app, "z", modifier);
        reselect(&mut app);
        chord(&mut app, "right", modifier);
        assert_eq!(
            (nodes(&app), edges(&app)),
            (3, 2),
            "{scheme:?}: Mod+→ connects"
        );
        let _ = app.update(Message::Board(Sent::Input(
            view(&app).epoch(),
            Input::Key {
                key: wren_canvas::Key::Escape,
                repeat: false,
            },
        )));
        chord(&mut app, "z", modifier);
        assert_eq!((nodes(&app), edges(&app)), (2, 1), "{scheme:?}");
        chord(&mut app, "a", modifier);
        assert_eq!(
            view(&app).machine().selection().len(),
            3,
            "{scheme:?}: Mod+A selects every element"
        );
        // The clipboard's keys are the board's copy, cut and paste.
        let board_keys: &[Modifiers] = if alt_mode {
            &[modifier, Modifiers::CTRL]
        } else {
            &[modifier]
        };
        let read = |app: &App, letter: &str, modifiers| {
            use iced::keyboard::key::{Code, Physical};
            let code = match letter {
                "c" => Code::KeyC,
                "x" => Code::KeyX,
                _ => Code::KeyV,
            };
            view(app).key(
                &iced::keyboard::Key::Character(letter.into()),
                Physical::Code(code),
                modifiers,
                None,
                false,
                false,
            )
        };
        for &each in board_keys {
            let epoch = view(&app).epoch();
            for (letter, command) in [("c", C::Copy), ("x", C::Cut), ("v", C::Paste)] {
                assert_eq!(
                    read(&app, letter, each),
                    Some(vec![Sent::Command(epoch, command)]),
                    "{scheme:?}: {each:?}+{letter}"
                );
            }
            chord(&mut app, "x", each);
            assert_eq!(nodes(&app), 0, "{scheme:?}: {each:?}+X cuts");
            chord(&mut app, "z", modifier);
            assert_eq!(nodes(&app), 2, "{scheme:?}");
        }

        // In a label.
        for &each in board_keys {
            click_at(&mut app, EMPTY);
            relabel(&mut app, "pack", "Ship");
            let text = |app: &App| view(app).label().map(|label| label.text().to_owned());
            chord(&mut app, "z", each);
            assert_eq!(
                text(&app).as_deref(),
                Some("Pack"),
                "{scheme:?}: {each:?}+Z"
            );
            for ctrl_y in [Modifiers::CTRL, modifier] {
                chord(&mut app, "y", ctrl_y);
                assert_eq!(
                    text(&app).as_deref(),
                    Some("Pack"),
                    "{scheme:?}: {ctrl_y:?}+Y"
                );
            }
            chord(&mut app, "z", each | Modifiers::SHIFT);
            assert_eq!(
                text(&app).as_deref(),
                Some("Ship"),
                "{scheme:?}: {each:?}+Shift+Z"
            );
            chord(&mut app, "a", each);
            label(&mut app, LabelAction::Insert("Go".into()));
            assert_eq!(text(&app).as_deref(), Some("Go"), "{scheme:?}: {each:?}+A");
            // Copy, cut and paste are the label's, not the board's.
            let epoch = view(&app).epoch();
            let label_keys = [
                ("c", LabelAction::Copy),
                ("x", LabelAction::Cut),
                ("v", LabelAction::Paste),
            ];
            for (letter, action) in label_keys {
                assert_eq!(
                    read(&app, letter, each),
                    Some(vec![Sent::Label(epoch, action)]),
                    "{scheme:?}: {each:?}+{letter}"
                );
            }
            // A held paste pastes again in a label, though not on the board.
            let held_v = |app: &App| {
                use iced::keyboard::key::{Code, Physical};
                view(app).key(
                    &iced::keyboard::Key::Character("v".into()),
                    Physical::Code(Code::KeyV),
                    each,
                    None,
                    true,
                    false,
                )
            };
            assert_eq!(
                held_v(&app),
                Some(vec![Sent::Label(epoch, LabelAction::Paste)]),
                "{scheme:?}: held {each:?}+V"
            );
            assert_eq!(key(&mut app, C::CommitLabel), Disposition::Completed);
            assert_eq!(
                held_v(&app),
                Some(Vec::new()),
                "{scheme:?}: held {each:?}+V"
            );
            chord(&mut app, "z", modifier);
        }
    }
}

/// A keymap of `scheme`'s defaults with the user's `keybind` lines, given
/// to the app and its open board.
fn bind(app: &mut App, scheme: wren_core::keybind::Scheme, user: &[(&str, &str)]) {
    let user: Vec<(String, String)> = user
        .iter()
        .map(|(trigger, action)| ((*trigger).to_owned(), (*action).to_owned()))
        .collect();
    let (keymap, warnings) = super::preferences::build_keymap(scheme, &user, false);
    assert!(warnings.is_empty(), "{warnings:?}");
    app.preferences.keymap = keymap;
    app.apply_preferences();
}

/// Branch review, severity 3: with `ctrl+alt+e = view.focus`, AltGr+E,
/// which arrives as Ctrl+Alt+E typing `€` on some Linux layouts, types in
/// an open label in either mode, and the app's copy of the press does
/// not toggle Focus mode; a real Ctrl+Alt+E still does.
#[test]
fn altgr_text_types_in_a_label_whatever_app_command_its_chord_is() {
    use iced::keyboard::key::{Code, Physical};
    use iced::keyboard::{Key, Modifiers};
    use wren_core::keybind::{LinuxModifier, Platform, Scheme};
    let ctrl_alt = Modifiers::CTRL | Modifiers::ALT;
    let event = |text: Option<&str>| iced::keyboard::Event::KeyPressed {
        key: Key::Character("e".into()),
        modified_key: Key::Character(text.unwrap_or("e").into()),
        physical_key: Physical::Code(Code::KeyE),
        location: iced::keyboard::Location::Standard,
        modifiers: ctrl_alt,
        text: text.map(Into::into),
        repeat: false,
    };
    for linux in [LinuxModifier::Ctrl, LinuxModifier::Alt] {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        bind(
            &mut app,
            Scheme::new(Platform::Linux, linux),
            &[("ctrl+alt+e", "view.focus")],
        );
        lay_out(&mut app);
        relabel(&mut app, "pack", "Ship");
        let epoch = view(&app).epoch();
        let press = wren_canvas::KeyPress {
            key: Key::Character("e".into()),
            physical: Physical::Code(Code::KeyE),
            modifiers: ctrl_alt,
            text: Some("€".into()),
            repeat: false,
            held: true,
        };
        let _ = app.update(Message::Board(Sent::Key(epoch, press)));
        // What the app's keyboard subscription makes of the same press.
        if let Some(input) = crate::keys::input(&event(Some("€"))) {
            let _ = app.update(Message::Key(input));
        }
        let text = view(&app).label().map(|label| label.text().to_owned());
        if cfg!(target_os = "macos") {
            continue;
        }
        assert_eq!(text.as_deref(), Some("Ship€"), "{linux:?}");
        assert!(!app.focus, "{linux:?}");
        // A Ctrl+Alt+E that types nothing is the app's chord.
        let input = crate::keys::input(&event(None)).expect("the chord");
        let _ = app.update(Message::Key(input));
        assert!(app.focus, "{linux:?}");
    }
}

/// The owner's decision of 2026-11-26: a label follows the board layer's
/// undo on macOS as on Linux. With the Mac keymap ⌘Z undoes the label's
/// typing by default; `super+z = board.unbind` leaves it undoing nothing;
/// with ⌘Z given to the board's copy and ⌘U to its undo, ⌘U undoes and ⌘Z
/// does not. Redo and select all keep ⇧⌘Z and ⌘A. Read with the Mac
/// scheme, so it runs on every host.
#[test]
fn a_mac_label_s_undo_follows_the_board_layer() {
    use iced::keyboard::Modifiers;
    use wren_core::keybind::{LinuxModifier, Platform, Scheme};
    let mac = Scheme::new(Platform::Mac, LinuxModifier::default());
    for (user, undo) in [
        (&[][..], Some("z")),
        (&[("super+z", "board.unbind")][..], None),
        (
            &[("super+z", "board.copy"), ("super+u", "board.undo")][..],
            Some("u"),
        ),
    ] {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        bind(&mut app, mac, user);
        lay_out(&mut app);
        click_at(&mut app, EMPTY);
        relabel(&mut app, "pack", "Ship");
        let text = |app: &App| view(app).label().map(|label| label.text().to_owned());
        chord(&mut app, "z", Modifiers::LOGO);
        let after_z = if undo == Some("z") { "Pack" } else { "Ship" };
        assert_eq!(text(&app).as_deref(), Some(after_z), "{user:?}: ⌘Z");
        if undo == Some("u") {
            chord(&mut app, "u", Modifiers::LOGO);
            assert_eq!(text(&app).as_deref(), Some("Pack"), "{user:?}: ⌘U");
        }
        chord(&mut app, "z", Modifiers::LOGO | Modifiers::SHIFT);
        assert_eq!(text(&app).as_deref(), Some("Ship"), "{user:?}: ⇧⌘Z");
        chord(&mut app, "a", Modifiers::LOGO);
        label(&mut app, LabelAction::Insert("Go".into()));
        assert_eq!(text(&app).as_deref(), Some("Go"), "{user:?}: ⌘A");
    }
}

/// On a Mac host ⌥ with a letter types its character in a label, even on
/// a chord the board layer gives the label's copy.
#[cfg(target_os = "macos")]
#[test]
fn on_macos_option_text_types_in_a_label_whatever_its_chord_is_bound_to() {
    use iced::keyboard::key::{Code, Physical};
    use iced::keyboard::{Key, Modifiers};
    use wren_core::keybind::{LinuxModifier, Platform, Scheme};
    let mac = Scheme::new(Platform::Mac, LinuxModifier::default());
    for user in [&[][..], &[("alt+c", "board.copy")][..]] {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        bind(&mut app, mac, user);
        lay_out(&mut app);
        click_at(&mut app, EMPTY);
        relabel(&mut app, "pack", "Ship");
        let epoch = view(&app).epoch();
        let press = wren_canvas::KeyPress {
            key: Key::Character("c".into()),
            physical: Physical::Code(Code::KeyC),
            modifiers: Modifiers::ALT,
            text: Some("ç".into()),
            repeat: false,
            held: true,
        };
        let _ = app.update(Message::Board(Sent::Key(epoch, press)));
        let text = view(&app).label().map(|label| label.text().to_owned());
        assert_eq!(text.as_deref(), Some("Shipç"), "{user:?}");
    }
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
    selection
        .iter()
        .map(|element| element.id().as_str())
        .collect()
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
    // Refused by key and by command, saying why (§5.15 rule 5).
    let why = "This board is read-only: it needs features this build does not have: frames.";
    for command in [C::Move(Dir::Right), C::Delete, C::EditLabel, C::ConnectNext] {
        app.banner = None;
        assert_eq!(key(&mut app, command), Disposition::Blocked, "{command:?}");
        assert_eq!(app.banner.as_deref(), Some(why), "{command:?}");
        app.banner = None;
        assert_eq!(
            run(&mut app, command).0,
            Disposition::Blocked,
            "{command:?}"
        );
        assert_eq!(app.banner.as_deref(), Some(why), "{command:?}");
    }
    app.banner = None;
    assert_eq!(key(&mut app, C::SelectAll), Disposition::Completed);
    assert_eq!(app.banner, None, "what does not change it says nothing");
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
    let _ = app.update(Message::Board(Sent::Command(epoch, C::SelectAll)));
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

/// Run a board command as a palette row does, and count what is left to
/// run besides giving the board the keyboard, which a command that ran
/// always does.
fn run(app: &mut App, command: C) -> (Disposition, usize) {
    let outcome = app.dispatch(Command::Run(CommandId::Board(command)));
    let ran = outcome.processed.disposition != Disposition::Blocked;
    let units = outcome.task.units();
    assert!(units >= usize::from(ran));
    (outcome.processed.disposition, units - usize::from(ran))
}

#[test]
fn each_undo_is_one_draft_back_to_the_opening_bytes() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    assert!(!view(&app).can_undo());

    // Three gestures: a drag, a run of key moves, a label.
    let from = middle(&app, "pack");
    let zoom = view(&app).viewport().zoom;
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
            at: (from.0 + 60.0 * zoom, from.1),
        },
    );
    input(&mut app, Input::Release { button: PRESS });
    let dragged = session_text(&app);
    key(&mut app, C::Move(Dir::Down));
    key(&mut app, C::Move(Dir::Down));
    let moved = session_text(&app);
    relabel(&mut app, "receive", "Take order");
    input(
        &mut app,
        Input::Key {
            key: wren_canvas::Key::Escape,
            repeat: false,
        },
    );
    let labelled = session_text(&app);
    assert_eq!(app.board_revision, 4);
    tick_to(&mut app, 2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), labelled);

    // Three undos, each one draft, the last the opening text.
    for (step, text) in [moved.as_str(), dragged.as_str(), FLOW]
        .into_iter()
        .enumerate()
    {
        assert!(view(&app).can_undo());
        assert_eq!(key(&mut app, C::Undo), Disposition::Completed);
        assert_eq!(session_text(&app), text);
        assert_eq!(app.board_revision, 5 + step as u64);
    }
    assert!(!view(&app).can_undo() && view(&app).can_redo());
    assert_eq!(key(&mut app, C::Undo), Disposition::Completed);
    assert_eq!(app.board_revision, 7, "nothing left to undo, and no draft");
    assert_in_step(&app, Surface::Board);
    assert_eq!(parses(&app), 1, "undo is the view's own change");
    tick_to(&mut app, 4);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), FLOW);
    assert!(!dirty(&app));

    // Redo, from the palette as from the key.
    assert_eq!(run(&mut app, C::Redo), (Disposition::Completed, 0));
    assert_eq!(session_text(&app), dragged);
    assert_eq!(app.board_revision, 8);
}

#[test]
fn undo_on_a_hand_written_board_after_a_save_restores_its_bytes() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Hand.wren-board");
    let text = hand_written();
    std::fs::write(&path, &text).unwrap();
    let _ = app.apply_core(Op::Reconcile).unwrap();
    open_board(&mut app, "Hand");
    lay_out(&mut app);

    click(&mut app, "pack");
    key(&mut app, C::Move(Dir::Right));
    // Past the autosave delay the canonical form is on disk.
    tick_to(&mut app, 2);
    let canonical = std::fs::read_to_string(&path).unwrap();
    assert!(canonical != text && !canonical.contains('#'));
    assert!(!dirty(&app));

    // The save did not move the floor: undo drafts the file as it was
    // written by hand, and that is what is saved.
    key(&mut app, C::Undo);
    assert_eq!(session_text(&app), text);
    assert!(dirty(&app));
    tick_to(&mut app, 4);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert!(!dirty(&app));
    assert_in_step(&app, Surface::Board);
}

#[test]
fn a_change_underneath_the_board_leaves_nothing_to_undo() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    click(&mut app, "pack");
    key(&mut app, C::Move(Dir::Right));
    key(&mut app, C::Undo);
    tick_to(&mut app, 2);
    assert!(view(&app).can_redo());

    // A clean reload.
    let external = flow_and("External");
    change_on_disk(&mut app, &path, &external);
    assert!(!view(&app).can_undo() && !view(&app).can_redo());
    let revision = app.board_revision;
    for command in [C::Undo, C::Redo] {
        assert_eq!(key(&mut app, command), Disposition::Completed);
    }
    assert_eq!(
        (app.board_revision, session_text(&app)),
        (revision, external.clone())
    );

    // A conflict: the draft went to its copy, and undo does not bring it
    // back over what is on disk.
    key(&mut app, C::Move(Dir::Down));
    assert!(view(&app).can_undo() && dirty(&app));
    change_on_disk(&mut app, &path, FLOW);
    assert!(app.conflict_copy.is_some());
    assert!(!view(&app).can_undo());
    key(&mut app, C::Undo);
    assert_eq!(session_text(&app), FLOW);
    assert!(!dirty(&app));

    // Raw and back, and another note and back, begin with no history too.
    key(&mut app, C::Move(Dir::Down));
    tick_to(&mut app, 4);
    let saved = session_text(&app);
    open_markdown(&mut app, "Note");
    open_board(&mut app, "Flow");
    assert!(!view(&app).can_undo());
    key(&mut app, C::Undo);
    assert_eq!(session_text(&app), saved);
}

#[test]
fn the_clipboard_is_written_and_read_through_the_app() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    open_board(&mut app, "Flow");
    lay_out(&mut app);

    // With no node selected there is nothing to copy, cut or duplicate.
    for command in [C::Copy, C::Cut, C::Duplicate] {
        assert!(!app.command_enabled(CommandId::Board(command)));
        assert_eq!(run(&mut app, command), (Disposition::Blocked, 0));
    }
    key(&mut app, C::SelectAll);
    // Copy writes the clipboard and changes nothing; paste asks for it.
    assert_eq!(run(&mut app, C::Copy), (Disposition::Completed, 1));
    assert_eq!(run(&mut app, C::Paste), (Disposition::Completed, 1));
    assert_eq!((app.board_revision, session_text(&app).as_str()), (0, FLOW));

    // What the clipboard held comes back for this board: three new
    // blocks, and the old ones as they were.
    let fragment = wren_board::clip::copy(view(&app).board(), &{
        let selection = view(&app).machine().selection();
        selection.iter().cloned().collect::<Vec<_>>()
    })
    .expect("a fragment");
    let paste = |app: &mut App| {
        let epoch = view(app).epoch();
        let _ = app.update(Message::Board(Sent::Pasted(epoch, fragment.clone())));
    };
    paste(&mut app);
    assert_eq!(app.board_revision, 1);
    let once = session_text(&app);
    let (gone, new) = line_diff(FLOW, &once);
    assert!(gone.is_empty(), "{gone:?}");
    let blocks = |lines: &[&str]| lines.iter().filter(|line| line.starts_with('[')).count();
    assert_eq!(blocks(&new), 3);
    assert_eq!(view(&app).board().nodes.len(), 4);
    paste(&mut app);
    let twice = session_text(&app);
    let (gone, new) = line_diff(&once, &twice);
    assert!(gone.is_empty(), "{gone:?}");
    assert_eq!(blocks(&new), 3);
    assert_eq!(app.board_revision, 2);

    // Cut is the clipboard and one draft, and one undo.
    assert_eq!(run(&mut app, C::Cut), (Disposition::Completed, 1));
    assert_eq!(session_text(&app), once);
    key(&mut app, C::Undo);
    assert_eq!(session_text(&app), twice);

    // It is saved, and a new Session reads the same board.
    tick_to(&mut app, 2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), twice);
    let reopened = wren_board::parse(&twice).expect("a board");
    assert_eq!((reopened.nodes.len(), reopened.edges.len()), (6, 3));
    assert!(wren_board::check(&reopened)
        .iter()
        .all(|issue| { wren_board::check(view(&app).board()).contains(issue) }));

    // A paste that cannot be made is the banner, and nothing changes.
    app.banner = None;
    let epoch = view(&app).epoch();
    let newer = "# wren-board fragment 1\nschema = 2\n".to_owned();
    let _ = app.update(Message::Board(Sent::Pasted(epoch, newer)));
    assert!(app
        .banner
        .as_deref()
        .is_some_and(|text| text.starts_with("Cannot paste")));
    assert_eq!((app.board_revision, session_text(&app)), (4, twice));
}

#[test]
fn a_command_that_did_nothing_says_why() {
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Other");
    lay_out(&mut app);
    assert_eq!(run(&mut app, C::ZoomFit), (Disposition::Completed, 0));
    assert!(app.banner.is_none());
    let notice = wren_canvas::Update {
        changed: false,
        effects: vec![wren_canvas::Effect::Notice("No room.".into())],
    };
    let did = app.board_did(notice);
    assert_eq!(did.notice.as_deref(), Some("No room."));
    assert_eq!(app.banner.as_deref(), Some("No room."));
    assert_eq!(app.board_revision, 0);
}

/// The command keys of the open palette's rows, and which is highlighted.
fn palette_rows(app: &App) -> (Vec<&'static str>, Option<&'static str>) {
    use super::palette::Key;
    let palette = app.palette.as_ref().expect("a palette is open");
    let key = |key: &Key| match key {
        Key::Command(id) => id.key(),
        _ => "",
    };
    let keys = palette.results.iter().map(|row| key(&row.key)).collect();
    let active = palette.active.map(|index| key(&palette.results[index].key));
    (keys, active)
}

fn actions_open(app: &App) -> bool {
    app.palette
        .as_ref()
        .is_some_and(|palette| palette.mode == super::palette::Mode::BoardActions)
}

/// A right press at the middle of the node `name`, as the widget sends it.
fn right_click(app: &mut App, name: &str) {
    let at = middle(app, name);
    let button = Button::Right;
    input(app, Input::Press { button, at });
    input(app, Input::Release { button });
}

/// §5.17 rules 1 to 3 through the app: `/` and a right-click open the
/// actions palette on the selection, which lists the commands the board
/// takes for it, opens on the current value, and with nothing selected
/// offers paste, select all, zoom to fit and export.
#[test]
fn the_actions_palette_lists_what_the_selection_takes() {
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    lay_out(&mut app);

    assert_eq!(key(&mut app, C::Actions), Disposition::Completed);
    assert!(actions_open(&app));
    assert_eq!(
        palette_rows(&app),
        (
            vec![
                "board.paste",
                "board.select.all",
                "board.zoom.fit",
                "note.export_svg"
            ],
            Some("board.paste")
        )
    );
    let _ = app.update(Message::Escape);
    assert!(app.palette.is_none());

    // A right-click on a node selects it alone and opens the palette on
    // its shape, the first current value.
    right_click(&mut app, "pack");
    assert!(actions_open(&app));
    assert_eq!(selected(&app), ["pack"]);
    let (rows, active) = palette_rows(&app);
    assert_eq!(active, Some("board.shape.rectangle"));
    assert!(rows.contains(&"board.color.red") && rows.contains(&"board.fill.solid"));
    assert!(!rows.iter().any(|key| key.starts_with("board.edge.")));
    assert!(!rows.contains(&"board.paste") && !rows.contains(&"board.align.left"));
    let current: Vec<&str> = {
        let palette = app.palette.as_ref().unwrap();
        let current = palette.results.iter().filter(|row| row.current);
        current
            .map(|row| match &row.key {
                super::palette::Key::Command(id) => id.key(),
                _ => "",
            })
            .collect()
    };
    assert_eq!(
        current,
        [
            "board.shape.rectangle",
            "board.color.neutral",
            "board.fill.tint",
            "board.text_size.14"
        ]
    );
    // Opening previews nothing; only a highlight move does.
    assert_eq!(view(&app).patch(), None);
    let _ = app.update(Message::Escape);

    // Select all: both shapes and the connector.
    assert_eq!(key(&mut app, C::SelectAll), Disposition::Completed);
    assert_eq!(key(&mut app, C::Actions), Disposition::Completed);
    let (rows, _) = palette_rows(&app);
    assert!(rows.contains(&"board.edge.routing") && rows.contains(&"board.align.left"));
    assert!(!rows.contains(&"board.distribute.horizontal"));
    assert!(!rows.contains(&"board.label.edit"));
    // A typed `/` or `]` is the query's, not a command.
    let _ = app.update(Message::PaletteQuery("]/".into()));
    assert!(actions_open(&app));
    assert_eq!((app.board_revision, session_text(&app).as_str()), (0, FLOW));
}

/// The ways out of the actions palette while a row is previewed.
const PREVIEW_LEAVES: [Leave; 5] = [
    ("escape", |app| {
        let _ = app.update(Message::Escape);
    }),
    ("another note", |app| {
        open_markdown(app, "Note");
    }),
    ("the window losing focus", |app| {
        app.window_unfocused().unwrap();
    }),
    ("quitting", |app| {
        let _ = app.dispatch(Command::Run(CommandId::Quit));
    }),
    ("a theme change and escape", |app| {
        let _ = app.update(Message::SystemTheme(iced::theme::Mode::Light));
        let _ = app.update(Message::Escape);
    }),
];

/// §5.17 rule 4 through the app: a highlighted style row is drawn on the
/// board and is never the model, a draft or a write. Leaving the palette
/// any way but Enter writes nothing and leaves the document clean; Enter
/// writes exactly the one changed line as one undo step.
#[test]
fn a_previewed_style_reaches_the_file_only_by_enter() {
    use wren_board::Token;
    let red = C::Color(Token::Red);
    for (name, leave) in PREVIEW_LEAVES {
        let (directory, mut app) = fixture();
        let path = file(&directory, "Flow.wren-board");
        let saved = modified(&path);
        open_board(&mut app, "Flow");
        lay_out(&mut app);
        click(&mut app, "pack");
        assert_eq!(key(&mut app, C::Actions), Disposition::Completed, "{name}");
        let _ = app.update(Message::PaletteQuery("colour: red".into()));
        assert_eq!(palette_rows(&app).1, Some("board.color.red"), "{name}");
        assert_eq!(view(&app).patch(), Some(red), "{name}");
        assert_ne!(view(&app).shown(), view(&app).scene(), "{name}");
        // The autosave tick sees nothing to write.
        tick_to(&mut app, 2);
        assert_eq!(view(&app).patch(), Some(red), "{name}");
        assert!(!dirty(&app), "{name}");

        leave(&mut app);
        let _ = app.update(Message::Tick);
        assert!(
            !actions_open(&app) || name == "the window losing focus",
            "{name}"
        );
        if let Some(view) = &app.board {
            assert_eq!(view.patch(), None, "{name}");
            assert_eq!(view.shown(), view.scene(), "{name}");
            assert!(!view.can_undo(), "{name}");
        }
        tick_to(&mut app, 4);
        assert_eq!(app.board_revision, 0, "{name}");
        assert!(!dirty(&app), "{name}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FLOW, "{name}");
        assert_eq!(modified(&path), saved, "{name}");
    }

    // Enter applies the board the preview showed, as one step.
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    click(&mut app, "pack");
    assert_eq!(key(&mut app, C::Actions), Disposition::Completed);
    let _ = app.update(Message::PaletteQuery("colour: red".into()));
    let shown = view(&app).shown().clone();
    let _ = app.update(Message::Key(crate::keys::Input::Nav(
        crate::keys::Nav::Enter,
    )));
    assert!(app.palette.is_none());
    assert_eq!(app.column, Column::Editor);
    assert_eq!(view(&app).scene(), &shown);
    assert_eq!(view(&app).patch(), None);
    assert_eq!(app.board_revision, 1);
    tick_to(&mut app, 2);
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        line_diff(FLOW, &on_disk),
        (vec![], vec!["style.color = \"red\""])
    );
    assert_eq!(key(&mut app, C::Undo), Disposition::Completed);
    assert_eq!(session_text(&app), FLOW);
}

/// A settle while the palette stays open drops the preview, which comes
/// back only when the highlight moves again; the palette closes when its
/// board is no longer the open note.
#[test]
fn a_settle_drops_the_preview_until_the_highlight_moves() {
    use crate::keys::{Input as Keyed, Nav};
    use wren_board::Token;
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    click(&mut app, "pack");
    assert_eq!(key(&mut app, C::Actions), Disposition::Completed);
    let _ = app.update(Message::PaletteQuery("colour".into()));
    assert_eq!(palette_rows(&app).1, Some("board.color.neutral"));
    // The current colour previews nothing.
    assert_eq!(view(&app).patch(), None);
    let _ = app.update(Message::Key(Keyed::Nav(Nav::Down)));
    assert_eq!(view(&app).patch(), Some(C::Color(Token::Red)));
    app.window_unfocused().unwrap();
    let _ = app.update(Message::Tick);
    assert!(actions_open(&app));
    assert_eq!(view(&app).patch(), None);
    let _ = app.update(Message::Key(Keyed::Nav(Nav::Down)));
    assert_eq!(view(&app).patch(), Some(C::Color(Token::Orange)));
    // An arrange row previews nothing.
    let _ = app.update(Message::PaletteQuery("bring to front".into()));
    assert_eq!(palette_rows(&app).1, Some("board.arrange.front"));
    assert_eq!(view(&app).patch(), None);
    assert_eq!(view(&app).shown(), view(&app).scene());
    // Another note closes the palette.
    open_markdown(&mut app, "Note");
    let _ = app.update(Message::Tick);
    assert!(app.palette.is_none());
    assert_eq!(app.board_revision, 0);
}

/// The keys of the Board menu that are enabled, each checked against the
/// command it runs: the menu shares one count between the align items and
/// one between the distribute items, and must still agree with each, except
/// that a board command is greyed while the board takes text. What the
/// last `sync_menu` left the items showing must be the same.
fn board_menu(app: &App) -> Vec<&'static str> {
    let menu = crate::menu::MENUS
        .iter()
        .find(|menu| menu.kind == crate::menu::Kind::Board)
        .unwrap();
    let machine = app.board.as_ref().map(wren_canvas::BoardView::machine);
    let typing =
        machine.is_some_and(|machine| machine.editing().is_some() || machine.seeking().is_some());
    let mut enabled = app.menu_enabled();
    let listed: Vec<(&'static str, bool)> = crate::menu::commands(menu.rows)
        .into_iter()
        .map(|(key, _)| {
            let id = CommandId::parse(key).unwrap();
            let on = enabled(key);
            let runs = app.command_enabled(id) && !(typing && matches!(id, CommandId::Board(_)));
            assert_eq!(on, runs, "{key}");
            (key, on)
        })
        .collect();
    // What the items were last set to, wherever the menu holds them as
    // up to date, is what they would be set to now.
    if app.menu_current() {
        assert_eq!(app.menu_synced, listed, "the items as last synced");
    }
    listed
        .into_iter()
        .filter_map(|(key, on)| on.then_some(key))
        .collect()
}

/// §5.19: the Board menu's items follow `command_enabled`, so New Board
/// follows `note.new_board` and the rest are greyed unless a board is the
/// surface; an item runs through the keyboard's path, and the View menu's
/// text-size items zoom a board.
#[test]
fn the_board_menu_follows_its_commands_and_runs_them_as_the_keys_do() {
    let (_directory, mut app) = fixture();
    open_markdown(&mut app, "Note");
    assert_eq!(board_menu(&app), ["note.new_board"]);

    open_board(&mut app, "Flow");
    lay_out(&mut app);
    let idle = board_menu(&app);
    for key in [
        "note.new_board",
        "note.export_svg",
        "note.export_png",
        "note.export_drawio",
    ] {
        assert!(idle.contains(&key), "{key}");
    }
    for key in [
        "board.duplicate",
        "board.align.left",
        "board.distribute.horizontal",
    ] {
        assert!(!idle.contains(&key), "{key} with nothing selected");
    }

    // Two nodes: align, not distribute, which needs three.
    assert_eq!(key(&mut app, C::SelectAll), Disposition::Completed);
    let both = board_menu(&app);
    assert!(both.contains(&"board.duplicate") && both.contains(&"board.align.middle"));
    assert!(!both.contains(&"board.distribute.vertical"));

    // An item sends its key back, and it runs as the shortcut does.
    click(&mut app, "pack");
    let _ = app.menu_command("board.duplicate");
    assert_eq!(app.board_revision, 1);
    assert_eq!(view(&app).board().nodes.len(), 3);
    let zoom = view(&app).viewport().zoom;
    let _ = app.menu_command("font.increase");
    assert!(view(&app).viewport().zoom > zoom);
    assert_eq!(app.board_revision, 1, "a zoom drafts nothing");
}

/// §5.1 rule 3: while a label is open, or keyboard Connecting takes a
/// query, the board runs no command a menu lists, so the Board menu's
/// board items are greyed and choosing one, or pressing its ⌘ key
/// equivalent, which AppKit gives the menu first, does nothing.
#[test]
fn the_board_menu_greys_its_board_items_while_the_board_takes_text() {
    let (_directory, mut app) = fixture();
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    click(&mut app, "pack");
    assert!(board_menu(&app).contains(&"board.duplicate"));

    assert_eq!(key(&mut app, C::EditLabel), Disposition::Completed);
    label(&mut app, LabelAction::Insert("zz".into()));
    let editing = board_menu(&app);
    assert!(
        editing.iter().all(|key| !key.starts_with("board.")),
        "{editing:?}"
    );
    for key in [
        "note.new_board",
        "note.export_svg",
        "note.export_png",
        "note.export_drawio",
    ] {
        assert!(editing.contains(&key), "{key}");
    }
    let revision = app.board_revision;
    for key in ["board.duplicate", "board.select.connected"] {
        let _ = app.menu_command(key);
        assert_eq!(state(&app), "editing", "{key}");
        assert_eq!(view(&app).label().map(|label| label.text()), Some("zz"));
        assert_eq!(view(&app).board().nodes.len(), 2, "{key}");
        assert_eq!(app.board_revision, revision, "{key}");
    }

    // Committing the label gives the items back.
    assert_eq!(key(&mut app, C::CommitLabel), Disposition::Completed);
    assert_eq!(state(&app), "idle");
    assert!(board_menu(&app).contains(&"board.duplicate"));
    let revision = app.board_revision;

    // Keyboard Connecting from the one selected node.
    assert_eq!(
        key(&mut app, C::Tool(Tool::Connector)),
        Disposition::Completed
    );
    assert!(view(&app).machine().seeking().is_some());
    let connecting = board_menu(&app);
    assert!(
        connecting.iter().all(|key| !key.starts_with("board.")),
        "{connecting:?}"
    );
    let _ = app.menu_command("board.duplicate");
    assert!(view(&app).machine().seeking().is_some());
    assert_eq!(view(&app).board().nodes.len(), 2);
    assert_eq!(app.board_revision, revision);
}

/// The Board menu is worked out again only when something it reads has
/// changed: the autosave tick, forty times a second whenever a workspace
/// is open, and a pointer passing over the board cost a compare, however
/// large the selection. Every change it reads is still seen.
#[test]
fn the_board_menu_is_worked_out_only_when_what_it_reads_changes() {
    let (directory, mut app) = fixture();
    let path = file(&directory, "Flow.wren-board");
    open_board(&mut app, "Flow");
    lay_out(&mut app);
    assert_eq!(key(&mut app, C::SelectAll), Disposition::Completed);
    let _ = board_menu(&app);
    let syncs = app.menu_syncs;
    for _ in 0..100 {
        let _ = app.update(Message::Tick);
    }
    input(&mut app, Input::Move { at: EMPTY });
    assert_eq!(app.menu_syncs, syncs, "nothing the menu reads changed");

    // A selection, an edit, an undo, an open label, a change on disk and
    // another note each bring the items up to date.
    type Change<'a> = &'a dyn Fn(&mut App);
    let changes: [(&str, Change); 7] = [
        ("a selection", &|app| click(app, "pack")),
        ("an edit", &|app| {
            assert_eq!(key(app, C::Duplicate), Disposition::Completed);
        }),
        ("an undo", &|app| {
            assert_eq!(key(app, C::Undo), Disposition::Completed);
        }),
        ("an open label", &|app| {
            assert_eq!(key(app, C::EditLabel), Disposition::Completed);
        }),
        ("a committed label", &|app| {
            label(app, LabelAction::Insert("zz".into()));
            assert_eq!(key(app, C::CommitLabel), Disposition::Completed);
        }),
        ("a change on disk", &|app| {
            change_on_disk(app, &path, &flow_and("External"));
        }),
        ("another note", &|app| {
            open_markdown(app, "Note");
        }),
    ];
    // A test helper may change the app without a message, so a tick
    // stands for the message that follows in a window.
    for (what, change) in changes {
        let before = app.menu_syncs;
        change(&mut app);
        let _ = app.update(Message::Tick);
        assert!(app.menu_syncs > before, "{what}");
        assert!(app.menu_current(), "{what}");
        let _ = board_menu(&app);
        let synced = app.menu_syncs;
        let _ = app.update(Message::Tick);
        assert_eq!(app.menu_syncs, synced, "a tick after {what}");
    }
    assert_eq!(board_menu(&app), ["note.new_board"]);
}
