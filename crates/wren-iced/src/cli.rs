use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub editor: bool,
    pub large_spike: bool,
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
}

pub fn parse(args: &[OsString], _test_mode: Option<&OsStr>) -> Result<Command, String> {
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
            "--workspace and the in-memory --editor/--editor-spike cannot be combined".into(),
        );
    }
    Ok(Command::Run(options))
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
