//! The macOS menu bar.
//!
//! Installing it replaces winit's default menu, whose Quit item calls
//! `terminate:` and would exit without Wren's autosave flush. Here Quit, like
//! every item that maps to a command, sends that command's key back to the
//! app, which runs it through the same path as the keyboard shortcut.

use std::cell::RefCell;
use std::sync::Mutex;

use iced::futures::channel::mpsc;
use iced::Subscription;
use muda::accelerator::Accelerator;
use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use wren_core::keybind::Keymap;

thread_local! {
    // NSApp keeps showing the menu only while muda's owner is alive.
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

/// Build and install the menu bar on the main thread. Item shortcuts come
/// from the keymap so a rebinding in `config.conf` shows in the menu.
pub fn install(keymap: &Keymap) {
    let item = |action: &str, label: &str| {
        let accelerator = keymap
            .accel(action)
            .and_then(|accel| accel.to_string().parse::<Accelerator>().ok());
        MenuItem::with_id(action, label, true, accelerator)
    };
    let about = AboutMetadata {
        name: Some("Wren".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        ..AboutMetadata::default()
    };
    let app_menu = Submenu::with_items(
        "Wren",
        true,
        &[
            &PredefinedMenuItem::about(None, Some(about)),
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
    let (Ok(app_menu), Ok(go_menu), Ok(window_menu)) = (app_menu, go_menu, window_menu) else {
        return;
    };
    let Ok(menu) = Menu::with_items(&[&app_menu, &go_menu, &window_menu]) else {
        return;
    };
    menu.init_for_nsapp();
    window_menu.set_as_windows_menu_for_nsapp();
    MENU.with(|slot| *slot.borrow_mut() = Some(menu));
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
