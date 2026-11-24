//! The task-line baseline (discovery/tasks-build.md item 0.1): what Wren
//! does today with the lines that tasks will read, pinned so that every
//! later item can show `tasks = off` leaves it alone.
//!
//! One note of task lines (`tools/wrentest/fixtures/tasks/`) is read the
//! ways a user meets it: the block classifier, the highlighter's spans and
//! decoration, spell check's prose and the misspellings the bundled
//! dictionary finds in it, the search preview and snippet, and the bytes
//! after Enter at the line's end, Backspace at its text and
//! `markdown.toggle_task`, on both editor engines. The report must equal
//! `golden/tasks-baseline.txt` byte for byte. That file was written once,
//! from the code before any task work; a later item may change how this
//! test computes a field (a type it reads changed shape), never the bytes
//! it expects. On a mismatch the actual report is written beside the
//! build's other output, `target/tasks-baseline.actual.txt`, for the diff.

use std::fmt::Write as _;
use std::ops::Range;

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor::{Cursor, Position};
use iced::{font, Color, Font};
use wren_core::index::{QueryKind, Scope};
use wren_core::ops::{Op, Session};
use wren_editor::{Highlight, LineDecor, Palette, Scale};
use wren_markdown::block::{self, Boxes, Container, Kind, LineInfo, Marker};
use wren_markdown::status::StatusType;

use super::commands::{Command, CommandId};
use super::spelling::{dictionary, Loaded, Spelling};
use super::{App, Message, Options};

const NOTE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/wrentest/fixtures/tasks/workspace/Tasks/Baseline/Task lines.md"
));

const GOLDEN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/golden/tasks-baseline.txt"
));

const HEADER: &str = "\
# Wren task-line baseline, recorded by tasks build item 0.1 before any task
# code existed. It pins what Wren does with these lines while `tasks = off`.
# Do not regenerate this file to make a later item pass: a difference here
# is a change users would see with the setting off.
";

/// The distinct word each line's description carries, for the search
/// snippet.
const WORDS: [&str; 19] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra",
];

/// A palette whose every role has a colour of its own, so a span's colour
/// names its role.
const ROLES: [(&str, Color); 13] = [
    ("text", Color::from_rgb8(1, 0, 0)),
    ("muted", Color::from_rgb8(2, 0, 0)),
    ("faint", Color::from_rgb8(3, 0, 0)),
    ("accent", Color::from_rgb8(4, 0, 0)),
    ("heading", Color::from_rgb8(5, 0, 0)),
    ("heading2", Color::from_rgb8(6, 0, 0)),
    ("link", Color::from_rgb8(7, 0, 0)),
    ("code", Color::from_rgb8(8, 0, 0)),
    ("quote", Color::from_rgb8(9, 0, 0)),
    ("completed", Color::from_rgb8(10, 0, 0)),
    ("date", Color::from_rgb8(11, 0, 0)),
    ("overdue", Color::from_rgb8(12, 0, 0)),
    ("owner", Color::from_rgb8(13, 0, 0)),
];

fn palette() -> Palette {
    let role = |name: &str| ROLES.iter().find(|(role, _)| *role == name).unwrap().1;
    Palette {
        text: role("text"),
        muted: role("muted"),
        faint: role("faint"),
        accent: role("accent"),
        heading: role("heading"),
        heading2: role("heading2"),
        link: role("link"),
        code: role("code"),
        quote: role("quote"),
        completed: role("completed"),
        date: role("date"),
        overdue: role("overdue"),
        owner: role("owner"),
    }
}

fn lines() -> Vec<&'static str> {
    NOTE.split('\n').collect()
}

fn kind(kind: &Kind) -> String {
    match kind {
        Kind::Blank => "blank".into(),
        Kind::Prose => "prose".into(),
        Kind::Heading { level, text } => format!("heading {level} text {text:?}"),
        Kind::SetextUnderline { level, start } => format!("setext {level} from {start}"),
        Kind::ThematicBreak => "break".into(),
        Kind::FenceOpen { fence, info } => {
            format!("fence open {:?}x{} info {info:?}", fence.ch, fence.len)
        }
        Kind::FenceClose => "fence close".into(),
        Kind::FenceBody => "fence body".into(),
        Kind::IndentedCode => "indented code".into(),
        Kind::FrontMatter => "front matter".into(),
        Kind::Html => "html".into(),
    }
}

fn marker(marker: Marker) -> String {
    match marker {
        Marker::Bullet(bullet) => format!("bullet {bullet:?}"),
        Marker::Ordered {
            number,
            width,
            delimiter,
        } => format!("ordered {number} width {width} {delimiter:?}"),
    }
}

/// The classifier's reading of one line, in terms that outlive the shape
/// of its types.
fn classified(info: &LineInfo, containers: &[Container]) -> String {
    let mut out = format!(
        "{} depth {} quotes {} prefix_end {} text_start {} content_column {}",
        kind(&info.kind),
        info.depth,
        info.quote_depth,
        info.prefix_end,
        info.text_start,
        info.content_column
    );
    if info.lazy {
        out.push_str(" lazy");
    }
    if let Some(item) = info.item {
        let _ = write!(
            out,
            "\n  item {} at {}..{} indent {} content_column {} text_start {} quotes {}",
            marker(item.marker),
            item.marker_start,
            item.marker_end,
            item.indent,
            item.content_column,
            item.text_start,
            item.quote_depth
        );
        match item.task {
            Some(task) => {
                // With tasks off a box is open or done, and nothing else.
                let state = match task.status {
                    StatusType::Done => "done",
                    StatusType::Open => "open",
                    other => panic!("a {other:?} box with tasks off"),
                };
                let _ = write!(out, "\n  task {state} box {}..{}", task.start, task.end);
            }
            None => out.push_str("\n  task none"),
        }
    }
    let open: Vec<String> = containers
        .iter()
        .map(|container| match container {
            Container::Quote {
                column,
                content_column,
            } => format!("quote {column}/{content_column}"),
            Container::Item(item) => format!("item {}/{}", item.indent, item.content_column),
        })
        .collect();
    let _ = write!(out, "\n  open [{}]", open.join(", "));
    out
}

fn span(line: &str, range: &Range<usize>, highlight: Highlight) -> String {
    let format = highlight.to_format();
    let role = format.color.map_or("none".to_owned(), |color| {
        ROLES
            .iter()
            .find(|(_, role)| *role == color)
            .map_or_else(|| format!("{color:?}"), |(name, _)| (*name).to_owned())
    });
    let mut out = format!("{range:?} {:?} {role}", &line[range.clone()]);
    if let Some(font) = format.font {
        if font.weight != font::Weight::Normal {
            let _ = write!(out, " {:?}", font.weight);
        }
        if font.style != font::Style::Normal {
            let _ = write!(out, " {:?}", font.style);
        }
    }
    let scale = highlight.scale();
    if scale != Scale::BODY {
        let _ = write!(out, " scale {}/{}", scale.text, scale.row);
    }
    out
}

fn texts(line: &str, ranges: &[Range<usize>]) -> String {
    let parts: Vec<String> = ranges
        .iter()
        .map(|range| format!("{range:?} {:?}", &line[range.clone()]))
        .collect();
    format!("[{}]", parts.join(", "))
}

fn decor(line: &str, decor: &LineDecor) -> String {
    let mut out = format!("  hang {}", decor.hang);
    if decor.code {
        out.push_str(" code");
    }
    if !decor.strikes.is_empty() {
        let _ = write!(out, "\n  strikes {}", texts(line, &decor.strikes));
    }
    if !decor.code_spans.is_empty() {
        let _ = write!(out, "\n  code spans {}", texts(line, &decor.code_spans));
    }
    let _ = write!(out, "\n  prose {}", texts(line, &decor.prose));
    out
}

/// A workspace `name` of the note, plus one note per line of it for the
/// search preview, which reads only a note's first body line. A workspace
/// takes one writer, so each app has its own.
fn workspace(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let workspace = root.join(name);
    let group = workspace.join("Tasks/Baseline");
    std::fs::create_dir_all(&group).unwrap();
    std::fs::write(group.join("Task lines.md"), NOTE).unwrap();
    let lines = workspace.join("Tasks/Lines");
    std::fs::create_dir_all(&lines).unwrap();
    for (index, line) in self::lines().iter().enumerate() {
        std::fs::write(lines.join(format!("Line {index:02}.md")), line).unwrap();
    }
    workspace
}

/// An app on `engine` with the note open, `tasks = off` and `settings`, in
/// a workspace `name` of its own.
fn app(root: &std::path::Path, name: &str, engine: &str, settings: &str) -> App {
    let config = root.join(format!("config-{name}/config.conf"));
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(
        &config,
        format!("editor-engine = {engine}\ntasks = off\n{settings}"),
    )
    .unwrap();
    let options = Options {
        editor: true,
        ..Options::default()
    };
    let (mut app, _) = App::with_config(options, None, Some(config));
    app.workspace_mode = true;
    app.session = Some(Session::open(&workspace(root, name)).unwrap());
    app.sync_editor(true);
    let note = app
        .model()
        .unwrap()
        .notes
        .iter()
        .find(|note| note.path.as_path() == std::path::Path::new("Tasks/Baseline/Task lines.md"))
        .unwrap()
        .id;
    let _ = app.process_core(Op::OpenNote { note: Some(note) });
    assert_eq!(app.editor.text(), NOTE, "{engine}");
    app
}

#[derive(Clone, Copy)]
enum Edit {
    Enter,
    Backspace,
    Toggle,
}

/// The note after `edit` at `column` of `line`, as the lines that changed
/// and the caret; one undo then restores the note.
fn edited(app: &mut App, line: usize, column: usize, edit: Edit) -> String {
    app.editor.move_cursor(Cursor {
        position: Position { line, column },
        selection: None,
    });
    let key = |named| {
        app.editor
            .key_message(Key::Named(named), Modifiers::empty())
    };
    match edit {
        Edit::Enter | Edit::Backspace => {
            let messages = key(match edit {
                Edit::Enter => Named::Enter,
                _ => Named::Backspace,
            });
            for message in messages {
                let _ = app.update(Message::Editor(message));
            }
        }
        Edit::Toggle => {
            let _ = app.dispatch(Command::Run(CommandId::ToggleTask));
        }
    }
    let text = app.editor.text();
    let draft = &app.session.as_ref().unwrap().document().unwrap().text;
    assert_eq!(&text, draft, "the edit reaches the session");
    let caret = app.editor.cursor().position;
    let report = changed(NOTE, &text, (caret.line, caret.column));
    if text != NOTE {
        let _ = app.update(Message::Editor(wren_editor::Message::Undo));
        assert_eq!(app.editor.text(), NOTE, "one undo restores line {line}");
    }
    report
}

/// The lines of `before` that `after` replaced, and with what.
fn changed(before: &str, after: &str, caret: (usize, usize)) -> String {
    let old: Vec<&str> = before.split('\n').collect();
    let new: Vec<&str> = after.split('\n').collect();
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    if head == old.len() && head == new.len() {
        return format!("unchanged, caret {}:{}", caret.0, caret.1);
    }
    format!(
        "lines {head}..{} {:?} -> {:?}, caret {}:{}",
        old.len() - tail,
        &old[head..old.len() - tail],
        &new[head..new.len() - tail],
        caret.0,
        caret.1
    )
}

fn report() -> String {
    let root = crate::test_support::tempdir();
    let workspace = workspace(root.path(), "search");
    let lines = lines();
    // As `tasks = off` reads the lines: the plain boxes, and no task options.
    let boxes = Boxes::with_tasks(false);
    let infos = block::scan(&lines[..], boxes);
    let mut scanner = block::Scanner::with_front_matter(block::front_matter_end(&lines[..]), boxes);
    let highlighted = wren_editor::highlight_lines(NOTE, palette(), Font::MONOSPACE, None);

    let mut spelling = Spelling::new(None);
    spelling
        .loaded(Loaded::new(dictionary(), None))
        .expect("the bundled dictionary loads");
    let proofreader = spelling.proofreader().unwrap();

    let session = Session::open(&workspace).unwrap();
    let model = session.model();
    let note_named = |path: &str| {
        model
            .notes
            .iter()
            .find(|note| note.path.as_path() == std::path::Path::new(path))
            .unwrap()
            .id
    };
    let whole = note_named("Tasks/Baseline/Task lines.md");

    let mut apps: Vec<(&str, App)> = ["classic", "advanced"]
        .into_iter()
        .map(|engine| (engine, app(root.path(), engine, engine, "")))
        .collect();
    assert_eq!(
        apps[1].1.editor.backend(),
        wren_editor::BackendKind::Owned,
        "the advanced engine is the owned one"
    );
    for (_, app) in &apps {
        assert!(!app.preferences.tasks, "the baseline runs with tasks off");
    }

    let mut out = String::from(HEADER);
    for (index, line) in lines.iter().enumerate() {
        let info = scanner.next_line(line);
        assert_eq!(info, infos[index], "the scanner and scan agree");
        let _ = write!(out, "\n## line {index} {line:?}\n");
        let _ = writeln!(out, "block {}", classified(&info, scanner.containers()));

        let (spans, decoration) = &highlighted[index];
        for (range, highlight) in spans {
            let _ = writeln!(out, "  span {}", span(line, range, *highlight));
        }
        let _ = writeln!(out, "{}", decor(line, decoration));
        let mut marks = Vec::new();
        if !decoration.prose.is_empty() {
            proofreader.marks(line, &decoration.prose, &mut marks);
        }
        let _ = writeln!(out, "  misspelled {}", texts(line, &marks));

        let preview = session
            .index()
            .preview(note_named(&format!("Tasks/Lines/Line {index:02}.md")));
        let _ = writeln!(out, "preview {preview:?}");
        if let Some(word) = WORDS.iter().find(|word| line.contains(*word)) {
            let hit = session
                .index()
                .query(word, Scope::Workspace, QueryKind::Search, 50)
                .into_iter()
                .find(|hit| hit.note == whole)
                .expect("the word is found in the note");
            let _ = writeln!(
                out,
                "snippet {word} {:?} {:?} match {:?}",
                hit.snippet.text, hit.snippet.highlight, hit.content_match
            );
        }

        for (name, column, edit) in [
            ("enter at end", line.len(), Edit::Enter),
            ("backspace at text", info.text_start, Edit::Backspace),
            ("toggle task", line.len(), Edit::Toggle),
        ] {
            let results: Vec<(&str, String)> = apps
                .iter_mut()
                .map(|(engine, app)| (*engine, edited(app, index, column, edit)))
                .collect();
            if results[0].1 == results[1].1 {
                let _ = writeln!(out, "{name} @{column}: {}", results[0].1);
            } else {
                for (engine, result) in results {
                    let _ = writeln!(out, "{name} @{column} [{engine}]: {result}");
                }
            }
        }
    }
    out
}

#[test]
fn tasks_baseline_matches_the_golden_report() {
    let actual = report();
    if actual != GOLDEN {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tasks-baseline.actual.txt");
        let _ = std::fs::write(&path, &actual);
        let first = actual
            .lines()
            .zip(GOLDEN.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| actual.lines().count().min(GOLDEN.lines().count()));
        panic!(
            "the task-line baseline changed at line {}: expected {:?}, got {:?}; the whole \
             report is in {}",
            first + 1,
            GOLDEN.lines().nth(first),
            actual.lines().nth(first),
            path.display()
        );
    }
}

#[test]
fn tasks_baseline_note_holds_every_case_the_plan_names() {
    for case in [
        "- [ ] ",
        "- [x] ",
        "- [X] ",
        "- [/] ",
        "- [-] ",
        "- [a] ",
        "> - [ ] ",
        "1. [ ] ",
        "- [ ]\t",
        "@alice.chen",
        "📅 2026-11-20",
        "⏫",
        "#work",
    ] {
        assert!(NOTE.contains(case), "{case:?}");
    }
    assert!(GOLDEN.starts_with(HEADER));
}

/// The golden's line `index`, from its `## line` heading to the next.
fn golden_line(index: usize) -> &'static str {
    GOLDEN
        .split("\n## line ")
        .find(|block| block.starts_with(&format!("{index} ")))
        .unwrap_or_else(|| panic!("the golden has line {index}"))
}

/// The golden's readings reached through the app's own configured paths:
/// the live editor's highlighter on each engine, the proofreader the app
/// hands the advanced editor with spell check on, and the app's own
/// session's search index.
#[test]
fn tasks_baseline_holds_through_the_apps_own_paths() {
    let root = crate::test_support::tempdir();
    let lines = lines();
    for (engine, backend) in [
        ("classic", wren_editor::BackendKind::Iced),
        ("advanced", wren_editor::BackendKind::Owned),
    ] {
        let mut app = app(
            root.path(),
            &format!("live-{engine}"),
            engine,
            "spell-check = on\n",
        );
        assert_eq!(app.editor.backend(), backend, "{engine}");
        assert!(!app.preferences.tasks, "{engine}");
        let _ = app.update(Message::SpellLoaded(Loaded::new(dictionary(), None)));

        let live = app.editor.highlighted(palette());
        assert_eq!(live.len(), lines.len(), "{engine}");
        for (index, ((spans, decoration), line)) in live.iter().zip(&lines).enumerate() {
            let golden = golden_line(index);
            for (range, highlight) in spans {
                let span = format!("\n  span {}\n", span(line, range, *highlight));
                assert!(golden.contains(&span), "{engine} line {index}: {span:?}");
            }
            let spans_in_golden = golden.matches("\n  span ").count();
            assert_eq!(spans.len(), spans_in_golden, "{engine} line {index}");
            let decor = format!("\n{}\n", decor(line, decoration));
            assert!(golden.contains(&decor), "{engine} line {index}: {decor:?}");

            // The editor's own proofreader: only the advanced engine has one.
            let marks = app.editor.marks(index);
            let expected = if backend == wren_editor::BackendKind::Owned {
                format!("\n  misspelled {}\n", texts(line, &marks))
            } else {
                assert!(marks.is_empty(), "{engine} line {index}");
                "\n  misspelled ".to_owned()
            };
            assert!(
                golden.contains(&expected),
                "{engine} line {index}: {expected:?}"
            );
            for mark in &marks {
                let word = &line[mark.clone()];
                assert!(
                    !word.contains(['@', '#', '📅', '⏫', '✅'])
                        && !word.starts_with(|c: char| c.is_ascii_digit()),
                    "{engine} line {index}: {word:?} is a token"
                );
            }

            let session = app.session.as_ref().unwrap();
            let id = |path: &str| {
                session
                    .model()
                    .notes
                    .iter()
                    .find(|note| note.path.as_path() == std::path::Path::new(path))
                    .unwrap()
                    .id
            };
            let preview = session
                .index()
                .preview(id(&format!("Tasks/Lines/Line {index:02}.md")));
            let preview = format!("\npreview {preview:?}\n");
            assert!(
                golden.contains(&preview),
                "{engine} line {index}: {preview:?}"
            );
            if let Some(word) = WORDS.iter().find(|word| line.contains(*word)) {
                let hit = session
                    .index()
                    .query(word, Scope::Workspace, QueryKind::Search, 50)
                    .into_iter()
                    .find(|hit| hit.note == id("Tasks/Baseline/Task lines.md"))
                    .expect("the word is found in the note");
                let snippet = format!(
                    "\nsnippet {word} {:?} {:?} match {:?}\n",
                    hit.snippet.text, hit.snippet.highlight, hit.content_match
                );
                assert!(golden.contains(&snippet), "{engine} line {index}");
            }
        }
        if backend == wren_editor::BackendKind::Owned {
            let recieve = app.editor.marks(19);
            assert_eq!(
                recieve
                    .iter()
                    .map(|mark| &lines[19][mark.clone()])
                    .collect::<Vec<_>>(),
                ["recieve"]
            );
            for index in [13, 14, 15, 16, 17, 18] {
                assert!(
                    app.editor.marks(index).iter().all(|mark| {
                        let word = &lines[index][mark.clone()];
                        word.chars().all(char::is_alphabetic)
                    }),
                    "line {index}"
                );
            }
        }
    }
}
