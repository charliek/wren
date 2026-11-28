//! The task view's test fixtures: the M11 workspace, presses, batches
//! and clicks as iced hands them over, and the imports the tests share.

pub(super) use std::path::Path;

pub(super) use iced::keyboard::key::{Named, NativeCode, Physical};
pub(super) use iced::keyboard::{Key, Modifiers};
pub(super) use wren_core::fs_store::RelPath;
pub(super) use wren_core::ops::{Op, Session};
pub(super) use wren_markdown::date::Date;

pub(super) use super::{Click, Control, MenuFor};
pub(super) use crate::app::columns::Column;
pub(super) use crate::app::commands::Command;
pub(super) use crate::app::host_menu;
pub(super) use crate::app::tasks_view::{Event, Scope};
pub(super) use crate::app::{App, Message, Options};
pub(super) use crate::keys::{Input, Nav};
pub(super) use crate::list_keys::{ListKey, Press};

pub(super) const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/wrentest/fixtures/tasks/list"
);
pub(super) const SETTINGS: &str = "tasks = on\nboards = on\ntasks-me = sam, sam.lee\n";

pub(super) fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// An app on the workspace at `root`, as a launch opens it, today
/// pinned to Thursday 19 November 2026.
pub(super) fn launch(directory: &Path, root: &Path) -> App {
    let config = directory.join("config.conf");
    std::fs::write(&config, SETTINGS).unwrap();
    let options = Options {
        editor: true,
        ..Options::default()
    };
    let (mut app, _) = App::with_config(options, None, Some(config));
    app.pin_today(Date::parse_iso("2026-11-19").unwrap());
    app.workspace_mode = true;
    app.session = Some(Session::open_with(root, app.session_options()).unwrap());
    app.sync_editor(true);
    app.hydrate_navigation();
    app
}

/// The M11 fixture, with the view of the workspace open.
pub(super) fn fixture() -> (tempfile::TempDir, App) {
    let directory = crate::test_support::tempdir();
    let root = directory.path().join("workspace");
    copy(Path::new(FIXTURE), &root);
    let mut app = launch(directory.path(), &root);
    let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
    let _ = app.update(Message::Tick);
    assert!(app.task_view_has_keys());
    (directory, app)
}

pub(super) fn root(directory: &tempfile::TempDir) -> std::path::PathBuf {
    directory.path().join("workspace")
}

pub(super) fn press(key: Key, text: Option<&str>) -> Press {
    Press {
        key,
        physical: Physical::Unidentified(NativeCode::Unidentified),
        modifiers: Modifiers::empty(),
        text: text.map(str::to_owned),
        repeat: false,
    }
}

pub(super) fn named(key: Named) -> Press {
    press(Key::Named(key), None)
}

pub(super) fn letter(c: &str) -> Press {
    press(Key::Character(c.into()), Some(c))
}

/// One batch of presses, as iced hands it over: the view's key
/// wrapper reports each, then the app hears its own copies of those
/// the wrapper lets through, Escape and the shortcuts, in order.
pub(super) fn batch(app: &mut App, presses: &[Press]) {
    let serial = app.task_list.next_serial();
    in_batch(app, serial, presses);
}

/// [`batch`] in the batch `serial`, which may have had other input.
pub(super) fn in_batch(app: &mut App, serial: u64, presses: &[Press]) {
    for press in presses {
        let _ = app.update(Message::TaskView(Event::Key(ListKey {
            press: press.clone(),
            serial,
            after_press: false,
        })));
    }
    copies(app, presses);
}

/// The app's copies of `presses`.
pub(super) fn copies(app: &mut App, presses: &[Press]) {
    for press in presses {
        if press.is_plain_escape() {
            let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        } else if let Some(accel) =
            crate::keys::shortcut(&press.key, press.physical, press.modifiers)
        {
            let _ = app.update(Message::Key(Input::Accel(accel)));
        }
    }
}

/// A click in a batch of its own.
pub(super) fn click(app: &mut App, click: Click) {
    let serial = app.task_list.next_serial();
    let _ = app.update(Message::TaskView(Event::Click { serial, click }));
}

pub(super) fn control(app: &mut App, control: Control) {
    click(app, Click::Control(control));
}

/// Input from the menu over the view; a row chosen is pressed and then
/// released, as a pointer chooses it.
pub(super) fn menu_event(app: &mut App, event: host_menu::Event) {
    let serial = app.task_list.next_serial();
    if let host_menu::Event::Choose { instance, row } = event {
        let window = app.viewport;
        let at = (app.task_list.menu.as_ref())
            .and_then(|menu| menu.row_bounds(row, window))
            .map_or(iced::Point::ORIGIN, |bounds| bounds.center());
        let _ = app.update(Message::TaskView(Event::Click {
            serial,
            click: Click::Menu(host_menu::Event::Pressed {
                instance,
                at,
                primary: true,
            }),
        }));
    }
    let _ = app.update(Message::TaskView(Event::Click {
        serial,
        click: Click::Menu(event),
    }));
}

/// The chord of `action`, as the keymap binds it, with Ctrl or Alt.
pub(super) fn chord(app: &App, action: &str, key: &str) -> Press {
    [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO]
        .into_iter()
        .map(|modifiers| Press {
            modifiers,
            text: None,
            ..letter(key)
        })
        .find(|press| {
            crate::keys::shortcut(&press.key, press.physical, press.modifiers)
                .is_some_and(|accel| app.preferences.keymap.action(&accel) == Some(action))
        })
        .unwrap_or_else(|| panic!("{action} is bound to no chord of {key}"))
}

pub(super) fn document(app: &App) -> Option<String> {
    let document = app.session.as_ref()?.document()?;
    Some(document.path().as_path().to_string_lossy().into_owned())
}

pub(super) fn descriptions(app: &App) -> Vec<String> {
    let listing = &app.task_list.listing;
    listing
        .rows()
        .map(|row| listing.record(row).unwrap().description.clone())
        .collect()
}

pub(super) fn selected(app: &App) -> String {
    let entry = app.task_list.selected.as_ref().unwrap().entry;
    app.task_list
        .listing
        .record(entry)
        .unwrap()
        .description
        .clone()
}

pub(super) fn tasks_toml(directory: &tempfile::TempDir) -> Option<String> {
    std::fs::read_to_string(root(directory).join(".wren/tasks.toml")).ok()
}
