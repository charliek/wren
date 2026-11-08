mod app;
mod chrome;
mod cli;
mod drag;
mod drag_view;
mod ellipsis;
mod fonts;
mod keys;
#[cfg(target_os = "macos")]
mod macos_menu;
mod palette_scroll;
mod panes;
mod theme;

#[cfg(test)]
mod test_support;

#[cfg(feature = "smoke-test")]
mod smoke;
#[cfg(feature = "smoke-test")]
mod test_capture;
#[cfg(feature = "smoke-test")]
mod test_ipc;
#[cfg(feature = "smoke-test")]
mod test_trash;

use std::error::Error;
use std::process::ExitCode;

use app::App;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Wren: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The Linux window `application_id` (Wayland `app_id`, X11 `WM_CLASS`). It
/// must equal the desktop entry's basename, `wren.desktop`, so docks and app
/// switchers pair the window with that entry and its icon; Wayland ignores
/// window icons entirely.
#[cfg(target_os = "linux")]
const APP_ID: &str = "wren";

/// The 256 px hicolor icon, derived from `assets/icon/wren-icon.svg` by `make icons`.
const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon/hicolor/256x256/apps/wren.png");

fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(1280.0, 800.0),
        exit_on_close_request: false,
        icon: window_icon(),
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: APP_ID.to_owned(),
            ..Default::default()
        },
        ..iced::window::Settings::default()
    }
}

/// Decodes the embedded icon for X11 (`_NET_WM_ICON`) and other platforms that
/// honor window icons; macOS shows the bundle icon instead.
fn window_icon() -> Option<iced::window::Icon> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(ICON_PNG));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut rgba).ok()?;
    if (frame.color_type, frame.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
        return None;
    }
    rgba.truncate(frame.buffer_size());
    iced::window::icon::from_rgba(rgba, frame.width, frame.height).ok()
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let options = match cli::parse(&args, std::env::var_os("WREN_TEST_MODE").as_deref())? {
        cli::Command::Run(options) => options,
        cli::Command::Help => {
            println!("Wren\n\n  wren                        Restore the active workspace or choose a folder\n  wren --workspace /ABS/PATH   Browse and autosave Markdown notes\n  wren --editor               Small, unsaved in-memory editor evaluation\n  wren --editor-spike         Unsaved 5,000-line editor evaluation\n  wren --font-licenses        Show bundled font notices\n\nEvaluation buffers (--editor / --editor-spike) are never saved to disk.");
            return Ok(());
        }
        cli::Command::FontLicenses => {
            print!("{}", fonts::LICENSES);
            return Ok(());
        }
    };

    #[cfg(feature = "smoke-test")]
    let options = {
        let mut options = options;
        if let Some(path) = &options.test_socket {
            options.endpoint = Some(test_ipc::Endpoint::bind(path)?);
        }
        options
    };
    #[cfg(feature = "smoke-test")]
    let completion = options.smoke.as_ref().map(|request| request.completion());

    let title = if options.editor {
        "Wren — unsaved editor evaluation"
    } else {
        "Wren"
    };
    let mut application =
        iced::application(move || App::new(options.clone()), App::update, App::view)
            .title(title)
            .window(window_settings())
            .default_font(fonts::UI)
            .theme(App::theme)
            .subscription(App::subscription);
    for font in fonts::FACES {
        application = application.font(*font);
    }
    application.run()?;

    #[cfg(feature = "smoke-test")]
    if let Some(completion) = completion {
        if !completion.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("renderer smoke capture did not complete successfully".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_window_icon_decodes_to_square_rgba() {
        let (rgba, size) = window_icon().expect("embedded icon decodes").into_raw();
        assert_eq!((size.width, size.height), (256, 256));
        let alpha = |x: usize, y: usize| rgba[(y * 256 + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "rounded corners stay transparent");
        assert_eq!(alpha(128, 8), 255, "the tile is opaque");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn window_settings_carry_the_desktop_entry_application_id() {
        let settings = window_settings();
        assert_eq!(settings.platform_specific.application_id, APP_ID);
        assert!(settings.icon.is_some());
        assert!(!settings.exit_on_close_request);
    }
}
