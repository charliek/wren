//! The macOS menu bar.
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

/// The menu id of "Check for Updates…".
pub const CHECK_FOR_UPDATES: &str = "app.check_for_updates";

thread_local! {
    // NSApp keeps showing the menu only while muda's owner is alive.
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
    static UPDATES: RefCell<Option<MenuItem>> = const { RefCell::new(None) };
}

/// Build and install the menu bar on the main thread. Item shortcuts come
/// from the keymap so a rebinding in `config.conf` shows in the menu.
pub fn install(keymap: &Keymap) {
    let item = |action: &str, label: &str| {
        // The first binding muda can show: it has no key for `+`, so text
        // size shows its `=` binding.
        let accelerator = keymap
            .bindings()
            .filter(|(_, bound)| *bound == action)
            .find_map(|(accel, _)| accel.to_string().parse::<Accelerator>().ok());
        MenuItem::with_id(action, label, true, accelerator)
    };
    let about = AboutMetadata {
        name: Some("Wren".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        ..AboutMetadata::default()
    };
    let updates = MenuItem::with_id(
        CHECK_FOR_UPDATES,
        "Check for Updates…",
        wren_sparkle::can_check(),
        None,
    );
    let app_menu = Submenu::with_items(
        "Wren",
        true,
        &[
            &PredefinedMenuItem::about(None, Some(about)),
            &updates,
            &PredefinedMenuItem::separator(),
            &item("settings.open", "Settings…"),
            &item("theme.choose", "Change Theme…"),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &item("app.quit", "Quit Wren"),
        ],
    );
    let go_menu = Submenu::with_items(
        "Go",
        true,
        &[
            &item("palette.commands", "Command Palette…"),
            &item("navigation.jump", "Jump to Note…"),
            &item("navigation.search", "Search Workspace…"),
            &PredefinedMenuItem::separator(),
            &item("navigation.back", "Back"),
            &item("navigation.forward", "Forward"),
            &PredefinedMenuItem::separator(),
            &item("navigation.column_left", "Column Left"),
            &item("navigation.column_right", "Column Right"),
        ],
    );
    let view_menu = Submenu::with_items(
        "View",
        true,
        &[
            &item("font.increase", "Increase Text Size"),
            &item("font.decrease", "Decrease Text Size"),
            &item("font.reset", "Reset Text Size"),
            &PredefinedMenuItem::separator(),
            &item("font.choose", "Change Editor Font…"),
        ],
    );
    let window_menu = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ],
    );
    let (Ok(app_menu), Ok(go_menu), Ok(view_menu), Ok(window_menu)) =
        (app_menu, go_menu, view_menu, window_menu)
    else {
        return;
    };
    let Ok(menu) = Menu::with_items(&[&app_menu, &go_menu, &view_menu, &window_menu]) else {
        return;
    };
    menu.init_for_nsapp();
    window_menu.set_as_windows_menu_for_nsapp();
    MENU.with(|slot| *slot.borrow_mut() = Some(menu));
    UPDATES.with(|slot| *slot.borrow_mut() = Some(updates));
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

/// Whether "Check for Updates…" is enabled, or `None` before the menu exists.
#[cfg(feature = "smoke-test")]
pub fn updates_enabled() -> Option<bool> {
    UPDATES.with(|slot| slot.borrow().as_ref().map(MenuItem::is_enabled))
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
