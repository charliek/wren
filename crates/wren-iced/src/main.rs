mod app;
mod chrome;
mod cli;
mod drag;
mod drag_view;
mod editor_view;
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
            .window_size((1280.0, 800.0))
            .exit_on_close_request(false)
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
