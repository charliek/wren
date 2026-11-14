//! `wren export` run as a program with no display (discovery/boards.md
//! §7.4 EX2, §6.8 Phase 3 item 6): every option, every exit code, nothing
//! written when it fails, the Checkout flow's SVG byte for byte, and the
//! draw.io files of the Checkout flow and of the coverage board as parsed
//! XML (§7.4's draw.io paragraph, §6.8 Phase 3 item 9).
//!
//! `WREN_BLESS=1` rewrites the golden files for review.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FLOW: &str = "Checkout flow.wren-board";
const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/golden/checkout-flow.svg"
);

/// The draw.io study's coverage board, in schema 1.
const COVERAGE: &str = "Coverage.wren-board";

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/wrentest/fixtures/boards/workspace/Boards/Samples")
}

/// A folder holding copies of the sample boards, its path canonical.
fn workspace() -> tempfile::TempDir {
    let root = std::env::temp_dir().canonicalize().unwrap();
    let directory = tempfile::tempdir_in(root).unwrap();
    for name in [FLOW, "Shapes.wren-board", "Future.wren-board"] {
        std::fs::copy(samples().join(name), directory.path().join(name)).unwrap();
    }
    let coverage = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../wren-board/golden/drawio/coverage.wren-board");
    std::fs::copy(coverage, directory.path().join(COVERAGE)).unwrap();
    directory
}

/// `wren` run in `directory` with these arguments and no display to open.
fn wren<S: AsRef<OsStr>>(directory: &Path, args: &[S]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wren"))
        .args(args)
        .current_dir(directory)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WREN_TEST_MODE")
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[track_caller]
fn succeeded(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", stderr(output));
}

/// The names in `directory`, sorted.
fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A PNG's width, height and RGBA pixels.
fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(
        (frame.color_type, frame.bit_depth),
        (png::ColorType::Rgba, png::BitDepth::Eight)
    );
    pixels.truncate(frame.buffer_size());
    (frame.width, frame.height, pixels)
}

/// The width and height of an SVG export, from its root element.
fn svg_size(svg: &str) -> (f64, f64) {
    let attribute = |name: &str| -> f64 {
        let start = svg.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
        let end = start + svg[start..].find('"').unwrap();
        svg[start..end].parse().unwrap()
    };
    (attribute("width"), attribute("height"))
}

#[test]
fn the_flow_exports_with_no_display_as_its_golden_svg() {
    let directory = workspace();
    let board = std::fs::read(directory.path().join(FLOW)).unwrap();
    // A relative path, the output beside it.
    let output = wren(directory.path(), &["export", FLOW]);
    succeeded(&output);
    assert_eq!(stderr(&output), "");
    let written = std::fs::read_to_string(directory.path().join("Checkout flow.svg")).unwrap();
    if std::env::var_os("WREN_BLESS").is_some_and(|value| value == "1") {
        std::fs::write(GOLDEN, &written).unwrap();
    }
    let golden = std::fs::read_to_string(GOLDEN).unwrap();
    assert_eq!(
        written, golden,
        "rerun with WREN_BLESS=1 to review a change"
    );
    assert_eq!(std::fs::read(directory.path().join(FLOW)).unwrap(), board);

    // The same bytes to standard output, and over an existing file.
    let output = wren(directory.path(), &["export", FLOW, "-o", "-"]);
    succeeded(&output);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), golden);
    std::fs::write(directory.path().join("old.svg"), "old").unwrap();
    succeeded(&wren(directory.path(), &["export", FLOW, "-o", "old.svg"]));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("old.svg")).unwrap(),
        golden
    );
    let absolute = directory.path().join(FLOW);
    succeeded(&wren(
        Path::new("/"),
        &[OsStr::new("export"), absolute.as_os_str()],
    ));
    assert!(names(directory.path())
        .iter()
        .all(|name| !name.contains(".tmp")));
}

/// An XML document as one line per element, its path of names, its
/// attributes sorted and its text: what two files that draw.io would read
/// alike share, whatever their attribute order and indentation.
fn parsed(xml: &str) -> String {
    use resvg::usvg::roxmltree;
    let document = roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    let mut out = String::new();
    let mut stack = vec![(document.root_element(), 0)];
    while let Some((node, depth)) = stack.pop() {
        let mut attributes: Vec<(&str, &str)> = node
            .attributes()
            .map(|attribute| (attribute.name(), attribute.value()))
            .collect();
        attributes.sort_unstable();
        let text = node.text().map(str::trim).unwrap_or_default();
        out += &format!(
            "{}{} {attributes:?} {text:?}\n",
            "  ".repeat(depth),
            node.tag_name().name()
        );
        let children: Vec<_> = node.children().filter(|child| child.is_element()).collect();
        stack.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
    }
    out
}

/// The Checkout flow and the coverage board as draw.io files equal their
/// golden files as parsed XML, in Wren Light by default; the same bytes go
/// to standard output, and the file is written beside the board.
#[test]
fn the_flow_and_the_coverage_board_export_as_their_golden_drawio_files() {
    let directory = workspace();
    for (board, name) in [
        (FLOW, "checkout-flow.drawio"),
        (COVERAGE, "coverage.drawio"),
    ] {
        let output = wren(directory.path(), &["export", board, "--format", "drawio"]);
        succeeded(&output);
        let stem = board.trim_end_matches(".wren-board");
        let written =
            std::fs::read_to_string(directory.path().join(format!("{stem}.drawio"))).unwrap();
        if std::env::var_os("WREN_BLESS").is_some_and(|value| value == "1") {
            std::fs::write(golden(name), &written).unwrap();
        }
        let expected = std::fs::read_to_string(golden(name)).unwrap();
        assert_eq!(
            parsed(&written),
            parsed(&expected),
            "{name}: rerun with WREN_BLESS=1 to review a change"
        );
        assert!(written.contains(&format!("<diagram id=\"wren.board\" name=\"{stem}\">")));
        let output = wren(
            directory.path(),
            &["export", board, "-o", "-", "--format", "drawio"],
        );
        succeeded(&output);
        assert_eq!(String::from_utf8(output.stdout).unwrap(), written);
        // By the name's extension, any case; in another theme's colours.
        succeeded(&wren(
            directory.path(),
            &["export", board, "-o", "out.DRAWIO"],
        ));
        assert_eq!(
            std::fs::read_to_string(directory.path().join("out.DRAWIO")).unwrap(),
            written
        );
        let dark = wren(
            directory.path(),
            &[
                "export",
                board,
                "-o",
                "-",
                "--format",
                "drawio",
                "--theme",
                "wren-dark",
            ],
        );
        succeeded(&dark);
        let dark = String::from_utf8(dark.stdout).unwrap();
        assert_ne!(dark, written);
        parsed(&dark);
    }
    // What the coverage board holds that does not draw is a warning, as
    // for an image.
    let output = wren(
        directory.path(),
        &["export", COVERAGE, "--format", "drawio"],
    );
    assert!(
        stderr(&output).contains("connector x-dangling: its end names no node"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn every_option_changes_what_is_written() {
    let directory = workspace();
    let light = wren(directory.path(), &["export", FLOW, "-o", "-"]);
    succeeded(&light);
    let golden = String::from_utf8(light.stdout).unwrap();
    let (width, height) = svg_size(&golden);

    // Dark: the same drawing in other colours.
    let dark = wren(
        directory.path(),
        &["export", FLOW, "-o", "-", "--theme", "wren-dark"],
    );
    succeeded(&dark);
    let dark = String::from_utf8(dark.stdout).unwrap();
    assert_ne!(dark, golden);
    assert_eq!(svg_size(&dark), (width, height));
    assert!(dark.contains("fill=\"#14161b\""), "the dark canvas");
    let alias = wren(
        directory.path(),
        &["export", FLOW, "-o", "-", "--theme", "dark"],
    );
    assert_eq!(String::from_utf8(alias.stdout).unwrap(), dark);

    // A transparent SVG has no background rectangle.
    let clear = wren(
        directory.path(),
        &["export", FLOW, "-o", "-", "--background", "transparent"],
    );
    succeeded(&clear);
    let clear = String::from_utf8(clear.stdout).unwrap();
    assert_eq!(
        golden.matches("<rect").count(),
        clear.matches("<rect").count() + 1
    );

    // PNG by the name, by --format with the default name, at each scale.
    succeeded(&wren(directory.path(), &["export", FLOW, "-o", "flow.png"]));
    let (w, h, pixels) = decode(&std::fs::read(directory.path().join("flow.png")).unwrap());
    assert_eq!(
        (f64::from(w), f64::from(h)),
        ((width * 2.0).ceil(), (height * 2.0).ceil())
    );
    assert_eq!(pixels[..4], [0xf7, 0xf8, 0xfa, 255], "the light canvas");
    succeeded(&wren(
        directory.path(),
        &["export", FLOW, "--format", "png"],
    ));
    assert_eq!(
        std::fs::read(directory.path().join("Checkout flow.png")).unwrap(),
        std::fs::read(directory.path().join("flow.png")).unwrap()
    );
    for scale in ["1", "4"] {
        let output = wren(
            directory.path(),
            &[
                "export", FLOW, "-o", "-", "--format", "png", "--scale", scale,
            ],
        );
        succeeded(&output);
        let factor: f64 = scale.parse().unwrap();
        let (w, h, _) = decode(&output.stdout);
        assert_eq!(
            (f64::from(w), f64::from(h)),
            ((width * factor).ceil(), (height * factor).ceil())
        );
    }
    let output = wren(
        directory.path(),
        &[
            "export",
            FLOW,
            "-o",
            "-",
            "--format",
            "png",
            "--background",
            "transparent",
            "--theme",
            "nord",
        ],
    );
    succeeded(&output);
    let (_, _, pixels) = decode(&output.stdout);
    assert_eq!(pixels[3], 0, "a transparent corner");
    assert!(
        pixels.chunks(4).any(|pixel| pixel[3] == 255),
        "the shapes are drawn"
    );
}

#[test]
fn an_empty_board_is_an_empty_padded_canvas() {
    let directory = workspace();
    for (name, text) in [
        ("blank.wren-board", ""),
        ("empty.wren-board", "schema = 1\n"),
    ] {
        std::fs::write(directory.path().join(name), text).unwrap();
        let output = wren(directory.path(), &["export", name, "-o", "-"]);
        succeeded(&output);
        let svg = String::from_utf8(output.stdout).unwrap();
        assert_eq!(svg_size(&svg), (80.0, 80.0), "{svg}");
        assert_eq!(svg.matches('<').count(), 4, "{svg}");
    }
}

#[test]
fn what_is_wrong_with_a_board_that_draws_is_a_warning() {
    let directory = workspace();
    let shapes = wren(directory.path(), &["export", "Shapes.wren-board"]);
    succeeded(&shapes);
    assert!(
        stderr(&shapes).contains(
            "Wren: warning: Shapes.wren-board: connector dangling: its end names no node"
        ),
        "{}",
        stderr(&shapes)
    );
    let future = wren(
        directory.path(),
        &["export", "Future.wren-board", "-o", "future.svg"],
    );
    succeeded(&future);
    assert!(
        stderr(&future).contains("would open read-only"),
        "{}",
        stderr(&future)
    );
    assert!(directory.path().join("future.svg").is_file());

    // A value past a limit draws, clamped, with a warning for each limit.
    std::fs::write(
        directory.path().join("far.wren-board"),
        "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [16777216, 0]\nsize = [0, 60]\nz = \"V\"\n",
    )
    .unwrap();
    let far = wren(directory.path(), &["export", "far.wren-board"]);
    succeeded(&far);
    for warning in [
        "Wren: warning: far.wren-board: node a: it reaches past ±16777216 on the board; drawn within it",
        "Wren: warning: far.wren-board: node a: a side is shorter than 1; drawn 1 long",
    ] {
        assert!(stderr(&far).contains(warning), "{}", stderr(&far));
    }
    assert!(directory.path().join("far.svg").is_file());
}

#[test]
fn help_is_printed_with_no_display() {
    let directory = workspace();
    let help = wren(directory.path(), &["export", "--help"]);
    succeeded(&help);
    assert!(String::from_utf8(help.stdout)
        .unwrap()
        .starts_with("Usage: wren export <board>"));
    let main = wren(directory.path(), &["--help"]);
    succeeded(&main);
    assert!(String::from_utf8(main.stdout)
        .unwrap()
        .contains("wren export BOARD"));
}

#[test]
fn a_failed_export_says_why_exits_with_its_code_and_writes_nothing() {
    let directory = workspace();
    let path = directory.path();
    std::fs::write(
        path.join("conflict.wren-board"),
        "schema = 1\n<<<<<<< ours\n[node.a]\n=======\n[node.b]\n>>>>>>> theirs\n",
    )
    .unwrap();
    std::fs::write(path.join("binary.wren-board"), [0xff, 0xfe, 0x00]).unwrap();
    let huge = std::fs::File::create(path.join("huge.wren-board")).unwrap();
    huge.set_len(wren_core::fs_store::MAX_FILE_BYTES + 1)
        .unwrap();
    drop(huge);
    // 16,800 pixels square at scale 4.
    std::fs::write(
        path.join("wide.wren-board"),
        "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\n\n\
         [node.b]\nkind = \"shape\"\nat = [4000, 4000]\nsize = [120, 60]\n",
    )
    .unwrap();
    // One byte over the 8 MiB a board may have, and one element over the
    // 50,000: each is refused before a model is built.
    let pad = "x".repeat(wren_board::MAX_BOARD_BYTES - "schema = 1\npad = \"\"\n".len() + 1);
    std::fs::write(
        path.join("large.wren-board"),
        format!("schema = 1\npad = \"{pad}\"\n"),
    )
    .unwrap();
    let crowded: String = (0..=wren_board::MAX_ELEMENTS)
        .map(|i| {
            format!("\n[node.n{i}]\nkind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\nz = \"V\"\n")
        })
        .collect();
    std::fs::write(
        path.join("crowded.wren-board"),
        format!("schema = 1\n{crowded}"),
    )
    .unwrap();
    std::fs::create_dir(path.join("folder.svg")).unwrap();
    std::fs::write(path.join("Note.md"), "a note").unwrap();
    let before = names(path);
    let note = std::fs::read(path.join("Note.md")).unwrap();

    let cases: &[(&[&str], u8, &str)] = &[
        (
            &["export", "missing.wren-board"],
            1,
            "Could not read missing.wren-board",
        ),
        (
            &["export", "folder.svg", "-o", "x.svg"],
            1,
            "it is a folder",
        ),
        (&["export", FLOW, "-o", "folder.svg"], 1, "it is a folder"),
        (
            &["export", FLOW, "-o", "no/such/x.svg"],
            1,
            "Could not write no/such/x.svg",
        ),
        (
            &["export", FLOW, "-o", FLOW, "--format", "svg"],
            1,
            "it is the board being exported",
        ),
        (
            &[
                "export",
                FLOW,
                "-o",
                "./Checkout flow.wren-board",
                "--format",
                "png",
            ],
            1,
            "it is the board being exported",
        ),
        (
            &["export", FLOW, "-o", "Note.md", "--format", "svg"],
            1,
            "it is a note",
        ),
        (
            &[
                "export",
                "wide.wren-board",
                "--format",
                "png",
                "--scale",
                "4",
            ],
            1,
            "over the 64 megapixels",
        ),
        (&["export"], 2, "needs a board"),
        (
            &["export", FLOW, "--format", "pdf"],
            2,
            "--format is svg, png or drawio",
        ),
        (
            &["export", FLOW, "--format", "drawio", "--scale", "2"],
            2,
            "--scale is for a PNG export",
        ),
        (
            &[
                "export",
                FLOW,
                "-o",
                "x.drawio",
                "--background",
                "transparent",
            ],
            2,
            "--background is for an SVG or PNG export",
        ),
        (
            &["export", FLOW, "-o", "Note.md", "--format", "drawio"],
            1,
            "it is a note",
        ),
        (
            &["export", "conflict.wren-board", "--format", "drawio"],
            3,
            "conflict.wren-board is not a board Wren can draw: line 2",
        ),
        (
            &["export", FLOW, "--theme", "sepia"],
            2,
            "there is no theme sepia",
        ),
        (
            &["export", FLOW, "--scale", "2"],
            2,
            "--scale is for a PNG export",
        ),
        (
            &["export", FLOW, "-o", "flow.gif"],
            2,
            "cannot tell the format",
        ),
        (
            &["export", "conflict.wren-board"],
            3,
            "conflict.wren-board is not a board Wren can draw: line 2",
        ),
        (&["export", "binary.wren-board"], 3, "not UTF-8 text"),
        (&["export", "huge.wren-board"], 3, "it is over 64 MiB"),
        (
            &["export", "large.wren-board"],
            3,
            "large.wren-board is not a board Wren can draw: line 1, column 1: \
             the board is 8,388,609 bytes, over the 8,388,608 (8 MiB) a board may have",
        ),
        (
            &["export", "crowded.wren-board", "--format", "png"],
            3,
            "crowded.wren-board is not a board Wren can draw: line 1, column 1: \
             the board has 50,001 nodes and connectors, over the 50,000 a board may have",
        ),
    ];
    for (args, code, message) in cases {
        let output = wren(path, args);
        assert_eq!(
            output.status.code(),
            Some(i32::from(*code)),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(message),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(stderr(&output).starts_with("Wren: "), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert_eq!(names(path), before, "{args:?} wrote something");
    }
    assert_eq!(std::fs::read(path.join("Note.md")).unwrap(), note);
}

#[cfg(unix)]
#[test]
fn a_device_is_refused_for_standard_output() {
    let directory = workspace();
    let output = wren(directory.path(), &["export", FLOW, "-o", "/dev/null"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("it is not a file; -o - writes to standard output"));
}

/// A named pipe or a device as the board is refused at once: opening a
/// pipe would wait for a writer, and a device would be read to the limit.
#[cfg(target_os = "linux")]
#[test]
fn a_board_that_is_not_a_file_is_refused_without_reading_it() {
    let directory = workspace();
    let made = std::process::Command::new("mkfifo")
        .arg(directory.path().join("pipe.wren-board"))
        .status()
        .unwrap();
    assert!(made.success());
    let before = names(directory.path());
    for board in ["pipe.wren-board", "/dev/zero"] {
        // `timeout` ends a run that waits on the pipe with code 124.
        let output = Command::new("timeout")
            .arg("30")
            .arg(env!("CARGO_BIN_EXE_wren"))
            .args(["export", board, "-o", "out.svg"])
            .current_dir(directory.path())
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WREN_TEST_MODE")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{board}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(&format!("Could not read {board}: it is not a file")),
            "{board}: {}",
            stderr(&output)
        );
        assert_eq!(names(directory.path()), before, "{board} wrote something");
    }
}

#[cfg(unix)]
#[test]
fn an_unwritable_folder_fails_and_leaves_nothing() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = workspace();
    let locked = directory.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
    // A superuser can write into a read-only folder.
    if std::fs::write(locked.join("probe"), "").is_ok() {
        return;
    }
    let output = wren(directory.path(), &["export", FLOW, "-o", "locked/flow.svg"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("Could not write locked/flow.svg"));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(names(&locked).is_empty());
}
