use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use wren_core::theme::{self, Theme};

use crate::board_export::{Background, Format, PNG_SCALE};

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub editor: bool,
    pub large_spike: bool,
    /// The in-memory board spike, with this many shapes.
    pub board_spike: Option<usize>,
    pub workspace: Option<PathBuf>,
    /// Boxed: a request carries a whole theme, which would dwarf the other
    /// `Command` variants.
    #[cfg(feature = "smoke-test")]
    pub smoke: Option<Box<crate::smoke::Request>>,
    #[cfg(feature = "smoke-test")]
    pub test_socket: Option<PathBuf>,
    #[cfg(feature = "smoke-test")]
    pub endpoint: Option<crate::test_ipc::Endpoint>,
}

#[derive(Debug)]
pub enum Command {
    Run(Options),
    Help,
    FontLicenses,
    /// `wren export`: write a board as an image, with no window.
    Export(ExportArgs),
    /// `wren export --help`.
    ExportHelp,
}

/// Arguments `parse` refuses, and the exit code that says so: 2 for
/// `wren export`, 1 for a launch, as before the command existed.
#[derive(Debug)]
pub struct Refusal {
    pub message: String,
    pub code: u8,
}

impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Self { message, code: 1 }
    }
}

impl From<&str> for Refusal {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

/// Where `wren export` writes.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    File(PathBuf),
    Stdout,
}

/// What `wren export` was asked for, every default filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportArgs {
    pub board: PathBuf,
    pub output: Output,
    pub format: Format,
    pub theme: Theme,
    /// PNG pixels per board unit; an SVG has none.
    pub scale: f32,
    pub background: Background,
}

/// The exit code of `wren export` for bad usage.
pub const EXPORT_USAGE_CODE: u8 = 2;

/// `wren export --help`.
pub fn export_usage() -> String {
    let themes: Vec<&str> = theme::BUILT_INS.iter().map(|theme| theme.id).collect();
    format!(
        "Usage: wren export <board> [-o <path>|-] [--format svg|png|drawio] [--theme <id>] [--scale 1..4] [--background theme|transparent]

Write a board as an SVG or PNG image, or as a draw.io file, without opening
a window.

  -o <path>       Where to write; - writes to standard output. An existing
                  file is replaced. Default: beside the board, as
                  <name>.svg, <name>.png or <name>.drawio
  --format        svg, png or drawio. Default: from the extension of -o,
                  else svg
  --theme <id>    The colours to draw in: {}. Default: {}
  --scale <n>     PNG pixels per board unit, from 1 to 4. Default: {PNG_SCALE}
  --background    theme (the theme's canvas colour, the default) or
                  transparent; for SVG and PNG

Exit status: 0 written (warnings about the board go to standard error);
1 the board could not be read or the file not written; 2 bad usage;
3 the file is not a board Wren can draw.
",
        themes.join(", "),
        theme::LIGHT.id
    )
}

const MAX_SPIKE_SHAPES: usize = 100_000;

pub fn parse(args: &[OsString], _test_mode: Option<&OsStr>) -> Result<Command, Refusal> {
    if args.first().is_some_and(|arg| arg == "export") {
        return parse_export(&args[1..]);
    }
    if args == [OsString::from("--help")] || args == [OsString::from("-h")] {
        return Ok(Command::Help);
    }
    if args == [OsString::from("--font-licenses")] {
        return Ok(Command::FontLicenses);
    }
    #[cfg(feature = "smoke-test")]
    if args.first().is_some_and(|arg| arg == "--smoke-test") {
        return Ok(Command::Run(Options {
            smoke: crate::smoke::Request::parse(args, _test_mode)?.map(Box::new),
            ..Options::default()
        }));
    }

    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].to_str() {
            Some("--editor") if !options.editor && !options.large_spike => options.editor = true,
            Some("--editor-spike") if !options.editor && !options.large_spike => {
                options.editor = true;
                options.large_spike = true;
            }
            Some("--board-spike") if !options.editor => {
                index += 1;
                let count = args
                    .get(index)
                    .and_then(|count| count.to_str())
                    .and_then(|count| count.parse::<usize>().ok())
                    .filter(|count| (1..=MAX_SPIKE_SHAPES).contains(count))
                    .ok_or("--board-spike requires a shape count from 1 to 100000")?;
                options.editor = true;
                options.board_spike = Some(count);
            }
            Some("--workspace") if options.workspace.is_none() => {
                index += 1;
                let path =
                    PathBuf::from(args.get(index).ok_or("--workspace requires a directory")?);
                if !path.is_absolute() {
                    return Err("workspace path must be absolute".into());
                }
                options.workspace = Some(path);
            }
            #[cfg(feature = "smoke-test")]
            Some("--test-socket") if options.test_socket.is_none() => {
                if _test_mode != Some(OsStr::new("1")) {
                    return Err("test IPC requires WREN_TEST_MODE=1".into());
                }
                index += 1;
                let path = args.get(index).ok_or("--test-socket requires a path")?;
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err("test socket must be absolute".into());
                }
                options.test_socket = Some(path);
            }
            _ => return Err("unsupported or duplicate option; use --help".into()),
        }
        index += 1;
    }
    if options.workspace.is_some() && options.editor {
        return Err(
            "--workspace and the in-memory --editor/--editor-spike/--board-spike cannot be combined"
                .into(),
        );
    }
    Ok(Command::Run(options))
}

/// The arguments after `wren export`. Paths may be relative.
fn parse_export(args: &[OsString]) -> Result<Command, Refusal> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(Command::ExportHelp);
    }
    let usage = |message: String| Refusal {
        message: format!("{message}; see wren export --help"),
        code: EXPORT_USAGE_CODE,
    };
    let mut board = None;
    let mut output = None;
    let mut format = None;
    let mut theme = None;
    let mut scale = None;
    let mut background = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let Some(flag) = arg
            .to_str()
            .filter(|flag| flag.starts_with('-') && *flag != "-")
        else {
            if board.is_some() {
                return Err(usage("wren export takes one board".into()));
            }
            if arg == "-" {
                return Err(usage(
                    "wren export reads a board from a file, not from standard input".into(),
                ));
            }
            board = Some(PathBuf::from(arg));
            continue;
        };
        let value = rest
            .next()
            .ok_or_else(|| usage(format!("{flag} needs a value")))?;
        let text = || {
            value
                .to_str()
                .ok_or_else(|| usage(format!("{flag} takes text, not {}", value.display())))
        };
        let once = |set: bool| {
            if set {
                Err(usage(format!("{flag} is given twice")))
            } else {
                Ok(())
            }
        };
        match flag {
            "-o" => {
                once(output.is_some())?;
                output = Some(match value.to_str() {
                    Some("-") => Output::Stdout,
                    Some("") => return Err(usage("-o needs a path".into())),
                    _ => Output::File(PathBuf::from(value)),
                });
            }
            "--format" => {
                once(format.is_some())?;
                format = Some(
                    format_named(text()?)
                        .ok_or_else(|| usage("--format is svg, png or drawio".into()))?,
                );
            }
            "--theme" => {
                once(theme.is_some())?;
                let id = text()?;
                theme = Some(theme::builtin(id).ok_or_else(|| {
                    let ids: Vec<&str> = theme::BUILT_INS.iter().map(|theme| theme.id).collect();
                    usage(format!(
                        "there is no theme {id}; the themes are {}",
                        ids.join(", ")
                    ))
                })?);
            }
            "--scale" => {
                once(scale.is_some())?;
                scale = Some(
                    text()?
                        .parse::<f32>()
                        .ok()
                        .filter(|scale| (1.0..=4.0).contains(scale))
                        .ok_or_else(|| usage("--scale is a number from 1 to 4".into()))?,
                );
            }
            "--background" => {
                once(background.is_some())?;
                background = Some(match text()? {
                    "theme" => Background::Theme,
                    "transparent" => Background::Transparent,
                    _ => return Err(usage("--background is theme or transparent".into())),
                });
            }
            _ => return Err(usage(format!("wren export has no option {flag}"))),
        }
    }
    let board = board.ok_or_else(|| usage("wren export needs a board".into()))?;
    // The format a file's name asks for. A name with another extension is
    // refused rather than given an image it does not name.
    let format = match (format, &output) {
        (Some(format), _) => format,
        (None, Some(Output::File(path))) => match path.extension() {
            None => Format::Svg,
            Some(extension) => extension.to_str().and_then(format_named).ok_or_else(|| {
                usage(format!(
                    "cannot tell the format from the name {}; give --format svg, png or drawio",
                    path.display()
                ))
            })?,
        },
        (None, _) => Format::Svg,
    };
    if format != Format::Png && scale.is_some() {
        return Err(usage("--scale is for a PNG export".into()));
    }
    // A draw.io file has no background of its own: draw.io draws its page.
    if format == Format::Drawio && background.is_some() {
        return Err(usage("--background is for an SVG or PNG export".into()));
    }
    let output = output.unwrap_or_else(|| Output::File(board.with_extension(format.extension())));
    Ok(Command::Export(ExportArgs {
        board,
        output,
        format,
        theme: theme.unwrap_or(theme::LIGHT),
        scale: scale.unwrap_or(PNG_SCALE),
        background: background.unwrap_or(Background::Theme),
    }))
}

/// The format `name`, a `--format` value or an extension, names.
fn format_named(name: &str) -> Option<Format> {
    Format::ALL
        .into_iter()
        .find(|format| name.eq_ignore_ascii_case(format.extension()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn ordinary_launch_and_spike_are_explicitly_distinct() {
        let Command::Run(options) = parse(&[], None).unwrap() else {
            panic!("expected run");
        };
        assert!(!options.large_spike);
        assert!(!options.editor);
        let Command::Run(editor) = parse(&args(&["--editor"]), None).unwrap() else {
            panic!("expected evaluation");
        };
        assert!(editor.editor);
        assert!(!editor.large_spike);
        assert!(parse(&args(&["--editor", "--editor-spike"]), None).is_err());
        assert!(parse(&args(&["--editor", "--editor"]), None).is_err());
        assert!(parse(&args(&["--workspace", "/tmp/notes", "--editor"]), None).is_err());
        let Command::Run(options) = parse(&args(&["--editor-spike"]), None).unwrap() else {
            panic!("expected run");
        };
        assert!(options.large_spike);
        assert!(parse(&args(&["--editor-spike", "--editor-spike"]), None).is_err());
    }

    #[test]
    fn the_board_spike_takes_a_shape_count_and_stands_alone() {
        let Command::Run(options) = parse(&args(&["--board-spike", "1000"]), None).unwrap() else {
            panic!("expected run");
        };
        assert_eq!(options.board_spike, Some(1000));
        assert!(options.editor, "an in-memory mode, with no workspace");
        assert!(!options.large_spike);
        for values in [
            vec!["--board-spike"],
            vec!["--board-spike", "0"],
            vec!["--board-spike", "many"],
            vec!["--board-spike", "100001"],
            vec!["--board-spike", "10", "--board-spike", "10"],
            vec!["--board-spike", "10", "--editor"],
            vec!["--editor-spike", "--board-spike", "10"],
            vec!["--workspace", "/tmp/notes", "--board-spike", "10"],
        ] {
            assert!(parse(&args(&values), None).is_err(), "{values:?}");
        }
    }

    #[test]
    fn workspace_is_explicit_absolute_and_not_an_in_memory_spike() {
        let Command::Run(options) = parse(&args(&["--workspace", "/tmp/notes"]), None).unwrap()
        else {
            panic!("expected run");
        };
        assert_eq!(options.workspace, Some(PathBuf::from("/tmp/notes")));
        for values in [
            vec!["--workspace"],
            vec!["--workspace", "relative"],
            vec!["--workspace", "/tmp/notes", "--workspace", "/tmp/other"],
            vec!["--workspace", "/tmp/notes", "--editor-spike"],
        ] {
            assert!(parse(&args(&values), None).is_err());
        }
    }

    #[test]
    fn licenses_and_help_do_not_need_a_display() {
        assert!(matches!(
            parse(&args(&["--font-licenses"]), None),
            Ok(Command::FontLicenses)
        ));
        assert!(matches!(parse(&args(&["--help"]), None), Ok(Command::Help)));
        assert!(parse(&args(&["--unknown"]), None).is_err());
        assert!(matches!(
            parse(&args(&["export", "b.wren-board"]), None),
            Ok(Command::Export(_))
        ));
    }

    fn export(values: &[&str]) -> ExportArgs {
        match parse(&args(values), None) {
            Ok(Command::Export(export)) => export,
            other => panic!("{values:?}: {other:?}"),
        }
    }

    #[test]
    fn export_fills_in_its_defaults() {
        let flow = export(&["export", "Boards/Checkout flow.wren-board"]);
        assert_eq!(
            flow,
            ExportArgs {
                board: PathBuf::from("Boards/Checkout flow.wren-board"),
                output: Output::File(PathBuf::from("Boards/Checkout flow.svg")),
                format: Format::Svg,
                theme: theme::LIGHT,
                scale: PNG_SCALE,
                background: Background::Theme,
            }
        );
        // The format follows the output's name, and the default output the
        // format.
        let png = export(&["export", "flow.wren-board", "-o", "out/Flow.PNG"]);
        assert_eq!(png.format, Format::Png);
        assert_eq!(png.output, Output::File(PathBuf::from("out/Flow.PNG")));
        let png = export(&["export", "--format", "png", "flow.wren-board"]);
        assert_eq!(png.output, Output::File(PathBuf::from("flow.png")));
        assert_eq!(
            export(&["export", "flow", "-o", "plain"]).format,
            Format::Svg
        );
        let stdout = export(&["export", "flow.wren-board", "-o", "-"]);
        assert_eq!(
            (stdout.output, stdout.format),
            (Output::Stdout, Format::Svg)
        );
        // An explicit format wins over the name.
        assert_eq!(
            export(&["export", "a.wren-board", "-o", "a.svg", "--format", "png"]).format,
            Format::Png
        );
        // A draw.io file by its name or by --format, beside the board.
        let drawio = export(&["export", "flow.wren-board", "-o", "out/Flow.DrawIO"]);
        assert_eq!(drawio.format, Format::Drawio);
        let drawio = export(&["export", "--format", "drawio", "Boards/flow.wren-board"]);
        assert_eq!(
            drawio.output,
            Output::File(PathBuf::from("Boards/flow.drawio"))
        );
        assert_eq!(drawio.theme, theme::LIGHT);
        let themed = export(&["export", "b", "--format", "drawio", "--theme", "nord"]);
        assert_eq!(themed.theme.id, "nord");
    }

    #[test]
    fn export_takes_every_option_in_any_order() {
        let all = export(&[
            "export",
            "--scale",
            "3.5",
            "--theme",
            "dark",
            "/abs/flow.wren-board",
            "--background",
            "transparent",
            "--format",
            "png",
            "-o",
            "../out.png",
        ]);
        assert_eq!(
            all,
            ExportArgs {
                board: PathBuf::from("/abs/flow.wren-board"),
                output: Output::File(PathBuf::from("../out.png")),
                format: Format::Png,
                theme: theme::DARK,
                scale: 3.5,
                background: Background::Transparent,
            }
        );
        for id in theme::BUILT_INS.iter().map(|theme| theme.id) {
            assert_eq!(export(&["export", "b", "--theme", id]).theme.id, id);
        }
        for scale in ["1", "4", "2.25"] {
            export(&[
                "export",
                "b.wren-board",
                "--format",
                "png",
                "--scale",
                scale,
            ]);
        }
    }

    #[test]
    fn export_help_and_bad_usage_exit_two() {
        for values in [
            vec!["export", "--help"],
            vec!["export", "-h"],
            vec!["export", "b.wren-board", "--bogus", "--help"],
        ] {
            assert!(matches!(
                parse(&args(&values), None),
                Ok(Command::ExportHelp)
            ));
        }
        for values in [
            vec!["export"],
            vec!["export", "a.wren-board", "b.wren-board"],
            vec!["export", "-"],
            vec!["export", "b", "--bogus"],
            vec!["export", "b", "-o"],
            vec!["export", "b", "-o", ""],
            vec!["export", "b", "-o", "x.svg", "-o", "y.svg"],
            vec!["export", "b", "-o", "x.jpg"],
            vec!["export", "b", "--format", "gif"],
            vec!["export", "b", "--format", "svg", "--format", "svg"],
            vec!["export", "b", "--theme", "nope"],
            vec!["export", "b", "--background", "white"],
            vec!["export", "b", "--format", "png", "--scale", "0.5"],
            vec!["export", "b", "--format", "png", "--scale", "5"],
            vec!["export", "b", "--format", "png", "--scale", "NaN"],
            vec!["export", "b", "--format", "png", "--scale", "two"],
            vec!["export", "b", "--scale", "2"],
            vec!["export", "b", "--format", "drawio", "--scale", "2"],
            vec![
                "export",
                "b",
                "-o",
                "b.drawio",
                "--background",
                "transparent",
            ],
            vec!["export", "b", "--format", "drawio", "--background", "theme"],
        ] {
            match parse(&args(&values), None) {
                Err(refusal) => {
                    assert_eq!(refusal.code, EXPORT_USAGE_CODE, "{values:?}");
                    assert!(
                        refusal.message.ends_with("see wren export --help"),
                        "{values:?}"
                    );
                }
                other => panic!("{values:?}: {other:?}"),
            }
        }
        // A launch's bad usage still exits 1, and `export` is only a
        // command in the first place.
        assert_eq!(parse(&args(&["--unknown"]), None).unwrap_err().code, 1);
        assert_eq!(
            parse(&args(&["--editor", "export"]), None)
                .unwrap_err()
                .code,
            1
        );
        assert!(export_usage().contains("wren-light"));
    }

    #[cfg(feature = "smoke-test")]
    #[test]
    fn ipc_requires_opt_in_and_an_absolute_path() {
        let values = args(&["--test-socket", "/tmp/wren.sock"]);
        assert!(parse(&values, None).is_err());
        assert!(parse(&values, Some(OsStr::new("1"))).is_ok());
        assert!(parse(&args(&["--test-socket", "relative"]), Some(OsStr::new("1"))).is_err());
        assert!(parse(&args(&["--test-socket"]), Some(OsStr::new("1"))).is_err());
    }

    #[cfg(not(feature = "smoke-test"))]
    #[test]
    fn production_has_no_test_endpoint() {
        assert!(parse(
            &args(&["--test-socket", "/tmp/wren.sock"]),
            Some(OsStr::new("1"))
        )
        .is_err());
        assert!(parse(
            &args(&["--smoke-test", "dark", "/tmp/window.png"]),
            Some(OsStr::new("1"))
        )
        .is_err());
    }
}
