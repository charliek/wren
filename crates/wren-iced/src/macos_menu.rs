//! The macOS menu bar, built from the table in `crate::menu`.
//!
//! Installing it replaces winit's default menu, whose Quit item calls
//! `terminate:` and would exit without Wren's autosave flush. Here Quit, like
//! every item that maps to a command, sends that command's key back to the
//! app, which runs it through the same path as the keyboard shortcut.
//! "Check for Updates…" is the one item that is not a command: it drives the
//! Sparkle updater (`wren_sparkle`), and is enabled only while that updater
//! can check.

use std::cell::RefCell;
use std::sync::Mutex;

use iced::futures::channel::mpsc;
use iced::Subscription;
use muda::accelerator::Accelerator;
use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use wren_core::keybind::Keymap;

use crate::menu::{self, Kind, Native, Row, CHECK_FOR_UPDATES};

/// An item of the Board menu, and whether it was last set enabled.
struct BoardItem {
    key: &'static str,
    item: MenuItem,
    enabled: bool,
}

thread_local! {
    // NSApp keeps showing the menu only while muda's owner is alive.
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
    static UPDATES: RefCell<Option<MenuItem>> = const { RefCell::new(None) };
    static BOARD: RefCell<Vec<BoardItem>> = const { RefCell::new(Vec::new()) };
}

/// What building the rows produced besides the items themselves.
struct Built {
    updates: Option<MenuItem>,
    board: Vec<BoardItem>,
}

/// Build and install the menu bar on the main thread. Item shortcuts come
/// from the keymap so a rebinding in `config.conf` shows in the menu; the
/// Board menu is built only with the `boards` setting on, and a task
/// command's item only with `tasks` on.
pub fn install(keymap: &Keymap, boards: bool, tasks: bool) -> muda::Result<()> {
    let menu = Menu::new();
    let mut built = Built {
        updates: None,
        board: Vec::new(),
    };
    let mut window_menu = None;
    for spec in menu::menus(boards) {
        let submenu = Submenu::new(spec.title, true);
        append(&submenu, spec.rows, spec.kind, keymap, tasks, &mut built)?;
        menu.append(&submenu)?;
        if spec.kind == Kind::Window {
            window_menu = Some(submenu);
        }
    }
    menu.init_for_nsapp();
    if let Some(window_menu) = window_menu {
        window_menu.set_as_windows_menu_for_nsapp();
    }
    MENU.with(|slot| *slot.borrow_mut() = Some(menu));
    UPDATES.with(|slot| *slot.borrow_mut() = built.updates);
    BOARD.with(|slot| *slot.borrow_mut() = built.board);
    Ok(())
}

fn append(
    submenu: &Submenu,
    rows: &'static [Row],
    kind: Kind,
    keymap: &Keymap,
    tasks: bool,
    built: &mut Built,
) -> muda::Result<()> {
    for row in rows.iter().filter(|row| menu::row_built(row, tasks)) {
        match *row {
            Row::Command(key, title) => {
                // The first trigger muda can show: it has no key for `+`, so
                // text size shows its `=` binding.
                let accelerator = menu::triggers(keymap, key, kind)
                    .find_map(|accel| accel.to_string().parse::<Accelerator>().ok());
                let item = MenuItem::with_id(key, title, true, accelerator);
                submenu.append(&item)?;
                if kind == Kind::Board {
                    built.board.push(BoardItem {
                        key,
                        item,
                        enabled: true,
                    });
                }
            }
            Row::Native(native) => match predefined(native) {
                Some(item) => submenu.append(&item)?,
                None => {
                    let item = MenuItem::with_id(
                        CHECK_FOR_UPDATES,
                        "Check for Updates…",
                        wren_sparkle::can_check(),
                        None,
                    );
                    submenu.append(&item)?;
                    built.updates = Some(item);
                }
            },
            Row::Separator => submenu.append(&PredefinedMenuItem::separator())?,
            Row::Submenu(title, rows) => {
                let inner = Submenu::new(title, true);
                append(&inner, rows, kind, keymap, tasks, built)?;
                submenu.append(&inner)?;
            }
        }
    }
    Ok(())
}

/// The AppKit item for `native`, or `None` for the updater's own item.
fn predefined(native: Native) -> Option<PredefinedMenuItem> {
    Some(match native {
        Native::About => PredefinedMenuItem::about(
            None,
            Some(AboutMetadata {
                name: Some("Wren".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                ..AboutMetadata::default()
            }),
        ),
        Native::Services => PredefinedMenuItem::services(None),
        Native::Hide => PredefinedMenuItem::hide(None),
        Native::HideOthers => PredefinedMenuItem::hide_others(None),
        Native::ShowAll => PredefinedMenuItem::show_all(None),
        Native::Minimize => PredefinedMenuItem::minimize(None),
        Native::Fullscreen => PredefinedMenuItem::fullscreen(None),
        Native::BringAllToFront => PredefinedMenuItem::bring_all_to_front(None),
        Native::CheckForUpdates => return None,
    })
}

/// Enable "Check for Updates…" exactly when the updater can check: never in a
/// build without Sparkle, and not while a check is already running.
pub fn sync_updates() {
    UPDATES.with(|slot| {
        if let Some(item) = slot.borrow().as_ref() {
            item.set_enabled(wren_sparkle::can_check());
        }
    });
}

/// Enable each Board menu item as `enabled` says of its command key,
/// touching only the items that change.
pub fn sync_board(mut enabled: impl FnMut(&str) -> bool) {
    BOARD.with(|slot| {
        for board in slot.borrow_mut().iter_mut() {
            let now = enabled(board.key);
            if now != board.enabled {
                board.item.set_enabled(now);
                board.enabled = now;
            }
        }
    });
}

/// Whether "Check for Updates…" is enabled, or `None` before the menu exists.
#[cfg(feature = "smoke-test")]
pub fn updates_enabled() -> Option<bool> {
    UPDATES.with(|slot| slot.borrow().as_ref().map(MenuItem::is_enabled))
}

/// Each Board menu item's key and whether the native item is enabled, in
/// menu order; empty before the menu exists or with the setting off.
#[cfg(feature = "smoke-test")]
pub fn board_enabled() -> Vec<(&'static str, bool)> {
    BOARD.with(|slot| {
        slot.borrow()
            .iter()
            .map(|board| (board.key, board.item.is_enabled()))
            .collect()
    })
}

/// Command keys chosen from the menu bar.
pub fn events() -> Subscription<String> {
    Subscription::run(|| {
        iced::stream::channel(16, |output: mpsc::Sender<String>| async move {
            let output = Mutex::new(output);
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Ok(mut output) = output.lock() {
                    let _ = output.try_send(event.id.0);
                }
            }));
            std::future::pending::<()>().await;
        })
    })
}
