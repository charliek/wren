//! The rows of the macOS menu bar, as data.
//!
//! `macos_menu` builds the native menu from this table; it lives apart so
//! its rules are checked on every platform, not only on a Mac. A row that
//! runs a command names the command's key, which the menu sends back to the
//! app to run through the keyboard's path.

use wren_core::keybind::{Accel, Keymap};

/// The menu id of "Check for Updates…", the one item that is not a command.
#[cfg(any(target_os = "macos", test))]
pub const CHECK_FOR_UPDATES: &str = "app.check_for_updates";

/// A row AppKit or the updater provides rather than a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Native {
    About,
    CheckForUpdates,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Fullscreen,
    BringAllToFront,
}

#[derive(Debug, Clone, Copy)]
pub enum Row {
    /// A command key and the item's title.
    Command(&'static str, &'static str),
    Native(Native),
    Separator,
    Submenu(&'static str, &'static [Row]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    /// Built only with the `boards` setting on. Its items show only ⌘
    /// shortcuts and follow `command_enabled`.
    Board,
    /// AppKit's Window menu, which lists the open windows.
    Window,
}

#[derive(Debug, Clone, Copy)]
pub struct Menu {
    pub title: &'static str,
    pub kind: Kind,
    pub rows: &'static [Row],
}

/// The menu bar, left to right (discovery/boards.md §5.19). A row added
/// here needs nothing else: the native menu, its enabling and the harness
/// all read this table.
pub const MENUS: [Menu; 5] = {
    use Row::{Command as C, Native as N, Separator as S};
    [
        Menu {
            title: "Wren",
            kind: Kind::Plain,
            rows: &[
                N(Native::About),
                N(Native::CheckForUpdates),
                S,
                C("settings.open", "Settings…"),
                C("settings.open_words", "Spelling Word List…"),
                C("theme.choose", "Change Theme…"),
                S,
                N(Native::Services),
                S,
                N(Native::Hide),
                N(Native::HideOthers),
                N(Native::ShowAll),
                S,
                C("app.quit", "Quit Wren"),
            ],
        },
        Menu {
            title: "Go",
            kind: Kind::Plain,
            rows: &[
                C("palette.commands", "Command Palette…"),
                C("navigation.jump", "Jump to Note…"),
                C("navigation.search", "Search Workspace…"),
                // On a board it opens the jump to a node (§5.18 rule 3).
                C("navigation.outline", "Jump in Note…"),
                S,
                C("navigation.back", "Back"),
                C("navigation.forward", "Forward"),
                S,
                C("navigation.column_left", "Column Left"),
                C("navigation.column_right", "Column Right"),
            ],
        },
        Menu {
            title: "View",
            kind: Kind::Plain,
            rows: &[
                C("font.increase", "Increase Text Size"),
                C("font.decrease", "Decrease Text Size"),
                C("font.reset", "Reset Text Size"),
                S,
                C("font.choose", "Change Editor Font…"),
                S,
                C("view.focus", "Toggle Focus Mode"),
                C("view.collapse_columns", "Toggle Collapsed Columns"),
            ],
        },
        Menu {
            title: "Board",
            kind: Kind::Board,
            rows: &[
                C("note.new_board", "New Board…"),
                S,
                C("board.duplicate", "Duplicate"),
                S,
                C("board.arrange.front", "Bring to Front"),
                C("board.arrange.forward", "Bring Forward"),
                C("board.arrange.backward", "Send Backward"),
                C("board.arrange.back", "Send to Back"),
                S,
                Row::Submenu(
                    "Align",
                    &[
                        C("board.align.left", "Left"),
                        C("board.align.center", "Center"),
                        C("board.align.right", "Right"),
                        S,
                        C("board.align.top", "Top"),
                        C("board.align.middle", "Middle"),
                        C("board.align.bottom", "Bottom"),
                    ],
                ),
                C("board.distribute.horizontal", "Distribute Horizontally"),
                C("board.distribute.vertical", "Distribute Vertically"),
                S,
                C("board.select.connected", "Select Connected"),
                S,
                C("note.export_svg", "Export as SVG…"),
                C("note.export_png", "Export as PNG…"),
                C("note.export_drawio", "Export as draw.io…"),
                S,
                C("board.keys", "Board Keys"),
            ],
        },
        Menu {
            title: "Window",
            kind: Kind::Window,
            rows: &[
                N(Native::Minimize),
                N(Native::Fullscreen),
                S,
                N(Native::BringAllToFront),
            ],
        },
    ]
};

/// The menus built with the `boards` setting as given.
pub fn menus(boards: bool) -> impl Iterator<Item = &'static Menu> {
    MENUS
        .iter()
        .filter(move |menu| boards || menu.kind != Kind::Board)
}

/// Whether a row is built while the `tasks` setting is `tasks`: a command
/// that exists only with it on is left out with it off.
pub fn row_built(row: &Row, tasks: bool) -> bool {
    match row {
        Row::Command(key, _) => {
            tasks || !crate::app::commands::CommandId::parse(key).is_some_and(|id| id.tasks())
        }
        Row::Native(_) | Row::Separator | Row::Submenu(..) => true,
    }
}

/// Every command row under `rows`, submenus included, in order.
#[cfg(test)]
pub fn commands(rows: &'static [Row]) -> Vec<(&'static str, &'static str)> {
    let mut found = Vec::new();
    for row in rows {
        match row {
            Row::Command(key, title) => found.push((*key, *title)),
            Row::Submenu(_, rows) => found.extend(commands(rows)),
            Row::Native(_) | Row::Separator => {}
        }
    }
    found
}

/// The triggers an item of a `kind` menu may show for `key`, best first;
/// the native menu shows the first one it can draw. A Board menu item
/// shows only a ⌘ trigger: a menu's key equivalent fires before the
/// focused widget sees the press, so a bare key, or one with only Shift or
/// ⌥, would fire while typing in a label or a field.
pub fn triggers<'a>(
    keymap: &'a Keymap,
    key: &'a str,
    kind: Kind,
) -> impl Iterator<Item = &'a Accel> {
    keymap
        .bindings()
        .chain(keymap.board_bindings())
        .filter(move |(accel, bound)| *bound == key && (kind != Kind::Board || accel.mods.logo))
        .map(|(accel, _)| accel)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use wren_core::keybind::{self, Layer, Platform};

    use super::*;
    use crate::app::commands::{CommandId, Scope};

    fn keymap(platform: Platform, user: &[(&str, &str)]) -> Keymap {
        let user: Vec<(String, String)> = user
            .iter()
            .map(|(trigger, action)| ((*trigger).to_owned(), (*action).to_owned()))
            .collect();
        let scheme = keybind::Scheme::new(platform, keybind::LinuxModifier::default());
        let (keymap, warnings) = Keymap::build(
            scheme,
            keybind::default_bindings(scheme)
                .into_iter()
                .chain(keybind::default_board_bindings(scheme)),
            &user,
            |action| {
                CommandId::parse(action).map(|id| match id.scope() {
                    Scope::Board => Layer::Board,
                    Scope::App | Scope::Editor => Layer::Base,
                })
            },
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        keymap
    }

    fn all_commands() -> impl Iterator<Item = (&'static Menu, &'static str, &'static str)> {
        MENUS.iter().flat_map(|menu| {
            commands(menu.rows)
                .into_iter()
                .map(move |(key, title)| (menu, key, title))
        })
    }

    fn shown(keymap: &Keymap, key: &str, kind: Kind) -> Option<String> {
        triggers(keymap, key, kind).next().map(ToString::to_string)
    }

    #[test]
    fn every_item_runs_an_app_or_board_command_and_names_it_once() {
        let mut seen = HashSet::new();
        for (_, key, title) in all_commands() {
            let id = CommandId::parse(key).unwrap_or_else(|| panic!("{key} is no command"));
            // A menu item runs through the app's path; an Editor command
            // runs only from the focused editor's own keys.
            assert!(
                matches!(id.scope(), Scope::App | Scope::Board),
                "{key} is only the editor's"
            );
            assert!(seen.insert(key), "{key} is in the menu twice");
            assert!(!title.is_empty() && !title.ends_with(' '), "{key}");
        }
        assert!(CommandId::parse(CHECK_FOR_UPDATES).is_none());
    }

    #[test]
    fn the_board_menu_sits_between_view_and_window_and_only_with_the_setting() {
        let titles = |boards| menus(boards).map(|menu| menu.title).collect::<Vec<_>>();
        assert_eq!(titles(true), ["Wren", "Go", "View", "Board", "Window"]);
        assert_eq!(titles(false), ["Wren", "Go", "View", "Window"]);
        // With the setting off no row reaches a command that exists only
        // with it on, and every row of the Board menu is one.
        for (menu, key, _) in all_commands() {
            let id = CommandId::parse(key).unwrap();
            assert_eq!(id.boards(), menu.kind == Kind::Board, "{key}");
        }
    }

    #[test]
    fn with_tasks_off_no_row_reaches_a_task_command() {
        for (menu, key, title) in all_commands() {
            let id = CommandId::parse(key).unwrap();
            let row = Row::Command(key, title);
            assert!(row_built(&row, true), "{key}");
            assert_eq!(row_built(&row, false), !id.tasks(), "{} {key}", menu.title);
        }
        for row in [Row::Separator, Row::Native(Native::About)] {
            assert!(row_built(&row, false));
        }
    }

    #[test]
    fn the_board_menu_holds_the_items_the_plan_names() {
        let board = MENUS.iter().find(|menu| menu.kind == Kind::Board).unwrap();
        let keys: Vec<_> = commands(board.rows)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(
            keys,
            [
                "note.new_board",
                "board.duplicate",
                "board.arrange.front",
                "board.arrange.forward",
                "board.arrange.backward",
                "board.arrange.back",
                "board.align.left",
                "board.align.center",
                "board.align.right",
                "board.align.top",
                "board.align.middle",
                "board.align.bottom",
                "board.distribute.horizontal",
                "board.distribute.vertical",
                "board.select.connected",
                "note.export_svg",
                "note.export_png",
                "note.export_drawio",
                "board.keys",
            ]
        );
        // These keys are the text's while a label or field is focused, so
        // no menu offers them (§5.19).
        for key in [
            "board.copy",
            "board.cut",
            "board.paste",
            "board.undo",
            "board.redo",
            "board.select.all",
        ] {
            assert!(all_commands().all(|(_, row, _)| row != key), "{key}");
        }
        let go = MENUS.iter().find(|menu| menu.title == "Go").unwrap();
        assert!(commands(go.rows).contains(&("navigation.outline", "Jump in Note…")));
    }

    #[test]
    fn a_board_item_shows_only_a_command_key_trigger() {
        let mac = keymap(Platform::Mac, &[]);
        assert_eq!(
            shown(&mac, "board.duplicate", Kind::Board).as_deref(),
            Some("super+d")
        );
        assert_eq!(
            shown(&mac, "board.select.connected", Kind::Board).as_deref(),
            Some("shift+super+a")
        );
        // Bare brackets and their shifted forms stay off the menu.
        for key in [
            "board.arrange.front",
            "board.arrange.back",
            "board.arrange.forward",
            "board.arrange.backward",
        ] {
            assert!(triggers(&mac, key, Kind::Board).next().is_none(), "{key}");
        }
        for platform in [Platform::Mac, Platform::Linux] {
            let keymap = keymap(platform, &[]);
            for menu in MENUS.iter().filter(|menu| menu.kind == Kind::Board) {
                for (key, _) in commands(menu.rows) {
                    assert!(
                        triggers(&keymap, key, menu.kind).all(|accel| accel.mods.logo),
                        "{key}"
                    );
                }
            }
        }

        // A user's bare-key binding takes display priority but is passed
        // over, and a ⌘ binding of theirs is shown.
        let mac = keymap(
            Platform::Mac,
            &[
                ("x", "board.duplicate"),
                ("super+shift+f", "board.arrange.front"),
            ],
        );
        assert_eq!(
            shown(&mac, "board.duplicate", Kind::Board).as_deref(),
            Some("super+d")
        );
        assert_eq!(
            shown(&mac, "board.arrange.front", Kind::Board).as_deref(),
            Some("shift+super+f")
        );
    }

    #[test]
    fn align_is_a_submenu_and_appkit_keeps_its_own_items() {
        let menu = |title: &str| MENUS.iter().find(|menu| menu.title == title).unwrap();
        let submenus: Vec<_> = menu("Board")
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Submenu(title, rows) => Some((*title, commands(rows).len())),
                _ => None,
            })
            .collect();
        assert_eq!(submenus, [("Align", 6)]);
        let natives = |title: &str| -> Vec<Native> {
            menu(title)
                .rows
                .iter()
                .filter_map(|row| match row {
                    Row::Native(native) => Some(*native),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            natives("Wren"),
            [
                Native::About,
                Native::CheckForUpdates,
                Native::Services,
                Native::Hide,
                Native::HideOthers,
                Native::ShowAll
            ]
        );
        assert_eq!(
            natives("Window"),
            [
                Native::Minimize,
                Native::Fullscreen,
                Native::BringAllToFront
            ]
        );
        assert_eq!(menu("Window").kind, Kind::Window);
    }

    #[test]
    fn other_menus_show_their_first_binding_as_before() {
        let mac = keymap(Platform::Mac, &[]);
        assert_eq!(
            shown(&mac, "palette.commands", Kind::Plain).as_deref(),
            Some("shift+super+p")
        );
        assert_eq!(
            shown(&mac, "navigation.outline", Kind::Plain).as_deref(),
            Some("shift+super+o")
        );
    }
}
