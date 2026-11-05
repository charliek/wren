mod app;
mod theme;

#[cfg(feature = "smoke-test")]
mod smoke;

use std::error::Error;
use std::process::ExitCode;

use app::{App, Options};

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

    #[cfg(not(feature = "smoke-test"))]
    let options = {
        if !args.is_empty() {
            return Err("this build does not support command-line options".into());
        }
        Options::default()
    };

    #[cfg(feature = "smoke-test")]
    let options = Options {
        smoke: smoke::Request::parse(&args, std::env::var_os("WREN_TEST_MODE").as_deref())?,
    };

    #[cfg(feature = "smoke-test")]
    let completion = options.smoke.as_ref().map(|request| request.completion());

    iced::application(move || App::new(options.clone()), App::update, App::view)
        .title("Wren — development scaffold")
        .window_size((960.0, 640.0))
        .theme(App::theme)
        .subscription(App::subscription)
        .run()?;

    #[cfg(feature = "smoke-test")]
    if let Some(completion) = completion {
        if !completion.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("renderer smoke capture did not complete successfully".into());
        }
    }

    Ok(())
}
