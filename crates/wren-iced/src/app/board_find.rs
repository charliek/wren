//! Finding things on a board (discovery/boards.md §5.18): the jump
//! palette, [`Mode::BoardNodes`], which lists the board's labelled
//! elements, shows the highlighted one and puts the view back when it
//! closes without a choice, as the outline palette does for headings; and
//! the element a workspace search hit lands on. None of it changes the
//! board: the selection and the viewport are never written to the file.

use std::collections::{BTreeSet, HashMap};

use iced::Task;
use wren_board::scene::{Draw, Scene, Source};
use wren_board::{shape, Board, Named, NodeKind, Vocabulary};
use wren_canvas::{hit, SavedView};
use wren_core::index::fuzzy_score;
use wren_core::model::NoteId;

use super::commands::{Disposition, Gateway};
use super::palette::{action_row, Action, Key, Mode, ResultRow};
use super::{App, Message};

/// One labelled element of a board, as the jump palette lists it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Labelled {
    pub element: Source,
    /// A node's text, or a connector's labels one per line.
    pub label: String,
    /// What the row names the element by: its shape, `text`, the kind a
    /// placeholder stands for, or `connector`.
    pub tag: String,
}

/// Every labelled element of `board`, in reading order: by the top, then
/// the left, of where `scene` draws it (a node's box, a connector's
/// labels), ties with nodes first and then by id. A label of only white
/// space is no label, as search has it (§3.6).
pub(super) fn labelled(board: &Board, scene: &Scene) -> Vec<Labelled> {
    let frames = hit::Frames::new(board);
    // Each connector's top-left, of its drawn labels and of all it draws,
    // in one pass over the scene, so the listing stays linear in its size.
    let mut corners: HashMap<&Source, Corners> = HashMap::new();
    for item in &scene.items {
        if !matches!(item.source, Source::Edge(_)) {
            continue;
        }
        let corner = (item.bounds.y, item.bounds.x);
        let entry = corners.entry(&item.source).or_default();
        earliest(&mut entry.any, corner);
        if matches!(item.draw, Draw::Text { .. }) {
            earliest(&mut entry.text, corner);
        }
    }
    let mut placed: Vec<((f32, f32), Labelled)> = Vec::new();
    for (id, node) in &board.nodes {
        if node.text.trim().is_empty() {
            continue;
        }
        let Some(frame) = frames.get(id) else {
            continue;
        };
        let tag = match &node.kind {
            Some(Named::Known(NodeKind::Shape)) => shape::drawn(&node.shape).name(),
            Some(Named::Known(NodeKind::Text)) => "text",
            Some(Named::Other(kind)) => kind.as_str(),
            None => "placeholder",
        };
        placed.push((
            (frame.y, frame.x),
            Labelled {
                element: Source::Node(id.clone()),
                label: node.text.clone(),
                tag: tag.to_owned(),
            },
        ));
    }
    for (id, edge) in &board.edges {
        let labels: Vec<&str> = edge
            .labels
            .iter()
            .map(|label| label.text.as_str())
            .filter(|text| !text.trim().is_empty())
            .collect();
        if labels.is_empty() {
            continue;
        }
        let element = Source::Edge(id.clone());
        // Where its labels are drawn; a connector the scene could not
        // route draws none, and is placed by whatever it draws.
        let corner = corners
            .get(&element)
            .and_then(|drawn| drawn.text.or(drawn.any))
            .unwrap_or_default();
        placed.push((
            corner,
            Labelled {
                element,
                label: labels.join("\n"),
                tag: "connector".to_owned(),
            },
        ));
    }
    placed.sort_by(|(a, first), (b, second)| {
        let rank = |entry: &Labelled| {
            (
                matches!(entry.element, Source::Edge(_)),
                entry.element.id().clone(),
            )
        };
        a.0.total_cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then_with(|| rank(first).cmp(&rank(second)))
    });
    placed.into_iter().map(|(_, entry)| entry).collect()
}

/// The top-left, as `(y, x)`, of what a connector draws.
#[derive(Default)]
struct Corners {
    text: Option<(f32, f32)>,
    any: Option<(f32, f32)>,
}

fn earliest(slot: &mut Option<(f32, f32)>, corner: (f32, f32)) {
    let earlier = |seen: (f32, f32)| {
        corner
            .0
            .total_cmp(&seen.0)
            .then(corner.1.total_cmp(&seen.1))
            .is_lt()
    };
    if slot.is_none_or(earlier) {
        *slot = Some(corner);
    }
}

/// The first element of `entries`, in their order, whose label contains
/// `query`, ignoring case as search does: what a search hit on a board
/// lands on (§5.18 rule 4). An empty query finds nothing.
pub(super) fn first_containing<'a>(entries: &'a [Labelled], query: &str) -> Option<&'a Labelled> {
    let lower = |text: &str| -> String { text.chars().flat_map(char::to_lowercase).collect() };
    let query = lower(query.trim());
    if query.is_empty() {
        return None;
    }
    entries
        .iter()
        .find(|entry| lower(&entry.label).contains(&query))
}

/// Where the board was when the jump palette opened, so leaving it any
/// way but choosing an element puts it back.
pub struct NodesOrigin {
    note: Option<NoteId>,
    viewport: SavedView,
    selection: BTreeSet<Source>,
    cursor: Option<Source>,
    /// The board's draft count and its view's epoch when `entries` were
    /// listed: either moves when the board changes or is reloaded.
    stamp: (u64, u64),
    entries: Vec<Labelled>,
    /// The highlighted row as of the last change the board acted on. It
    /// starts as the row the palette opens on, which only a highlight move
    /// or Enter shows.
    highlighted: Option<Source>,
    /// Whether the board still shows the origin: no row has been previewed,
    /// or none has been highlighted since.
    at_origin: bool,
}

impl App {
    fn board_stamp(&self) -> (u64, u64) {
        (
            self.board_revision,
            self.board.as_ref().map_or(0, wren_canvas::BoardView::epoch),
        )
    }

    /// The jump palette is opening on the open board, settled: take down
    /// where the board is, after putting back a jump palette still open.
    pub(super) fn begin_board_nodes(&mut self) {
        if let Some(origin) = self.board_nodes.take() {
            self.return_to_board_origin(origin);
        }
        let stamp = self.board_stamp();
        let Some(view) = &self.board else {
            return;
        };
        let machine = view.machine();
        self.board_nodes = Some(NodesOrigin {
            note: self.editor_note,
            viewport: view.saved_view(),
            selection: machine.selection().clone(),
            cursor: machine.cursor().cloned(),
            stamp,
            entries: labelled(view.board(), view.scene()),
            highlighted: None,
            at_origin: true,
        });
    }

    /// The jump palette has its first rows: the one it opens on is not
    /// shown until the highlight moves.
    pub(super) fn board_nodes_opened(&mut self) {
        let highlighted = self.highlighted_element();
        if let Some(origin) = &mut self.board_nodes {
            origin.highlighted = highlighted;
        }
    }

    /// The jump palette's rows for `query`: every labelled element, the
    /// first line of its label as the title and the rest beside it, fuzzy
    /// matched on the whole label like the outline's headings, in reading
    /// order where the scores are equal. The palette opens on the cursor.
    pub(super) fn board_node_rows(&self, query: &str) -> Vec<ResultRow> {
        let Some(origin) = &self.board_nodes else {
            return Vec::new();
        };
        let mut ranked: Vec<_> = origin
            .entries
            .iter()
            .filter_map(|entry| {
                let haystack = entry.label.replace('\n', " ");
                fuzzy_score(query, &haystack).map(|score| (score, entry))
            })
            .collect();
        // Stable: equal scores (an empty query) keep the reading order.
        ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        ranked
            .into_iter()
            .map(|(_, entry)| {
                let mut lines = entry.label.lines();
                let title = lines.next().unwrap_or_default().to_owned();
                let rest: Vec<&str> = lines.filter(|line| !line.trim().is_empty()).collect();
                let element = entry.element.clone();
                let mut row = action_row(
                    Key::Element(element.clone()),
                    title,
                    rest.join(" · "),
                    true,
                    Action::Element(element),
                );
                row.current = origin.cursor.as_ref() == Some(&entry.element);
                row.tag = Some(entry.tag.clone());
                row
            })
            .collect()
    }

    fn highlighted_element(&self) -> Option<Source> {
        let palette = self
            .palette
            .as_ref()
            .filter(|palette| palette.mode == Mode::BoardNodes)?;
        match &palette.results.get(palette.active?)?.key {
            Key::Element(element) => Some(element.clone()),
            _ => None,
        }
    }

    fn return_to_board_origin(&mut self, origin: NodesOrigin) {
        // Another note, or the board shown as raw text now, has no use for
        // the old view.
        if origin.at_origin || origin.note != self.editor_note {
            return;
        }
        if let Some(view) = &mut self.board {
            view.restore(origin.selection, origin.cursor, origin.viewport);
        }
    }

    /// Enter on a row: keep the element shown and selected and give the
    /// board the keyboard (§5.18 rule 2).
    pub(super) fn choose_element(&mut self, element: Source) -> Gateway {
        let Some(view) = &mut self.board else {
            return self.blocked("The open note is not shown as a board");
        };
        if !element.exists(view.board()) {
            return self.blocked("That element is no longer on the board");
        }
        self.board_nodes = None;
        // A previewed element is in place already, and showing it again
        // moves nothing.
        view.show_element(element);
        self.palette = None;
        let focus = self.return_to_editor();
        self.finish_action(Disposition::Completed, None, focus)
    }

    /// Follow the jump palette's highlighted row on the board, and put the
    /// board back when the palette closes without a choice. Runs after
    /// every message, so it acts only on a change, like `sync_outline`.
    pub(super) fn sync_board_nodes(&mut self) -> Task<Message> {
        let Some(origin) = &self.board_nodes else {
            return Task::none();
        };
        if !self
            .palette
            .as_ref()
            .is_some_and(|palette| palette.mode == Mode::BoardNodes)
        {
            let origin = self.board_nodes.take().expect("jump origin");
            self.return_to_board_origin(origin);
            return Task::none();
        }
        // The palette closes when the note changes, or stops being a board.
        if origin.note != self.editor_note || self.board.is_none() {
            // The view was put away showing a preview: the board is to
            // come back where it was before the palette opened.
            if let (false, Some(note)) = (origin.at_origin, origin.note) {
                if let Some(remembered) = self.board_viewports.get_mut(&note) {
                    *remembered = origin.viewport;
                }
            }
            self.board_nodes = None;
            self.palette = None;
            return self.column_focus();
        }
        let stamp = self.board_stamp();
        let revised = origin.stamp != stamp;
        if revised {
            let view = self.board.as_ref().expect("a board");
            let entries = labelled(view.board(), view.scene());
            let origin = self.board_nodes.as_mut().expect("jump origin");
            origin.stamp = stamp;
            origin.entries = entries;
            // Rows are keyed by element, so the highlight stays on an
            // element that is still there.
            self.refresh_palette(false);
        }
        let highlighted = self.highlighted_element();
        let origin = self.board_nodes.as_mut().expect("jump origin");
        if highlighted == origin.highlighted {
            return Task::none();
        }
        origin.highlighted = highlighted.clone();
        // A row the board never showed stays unshown when a reload moves
        // the highlight.
        if revised && origin.at_origin {
            return Task::none();
        }
        let view = self.board.as_mut().expect("a board");
        match highlighted {
            Some(element) => {
                origin.at_origin = false;
                view.show_element(element);
            }
            None if !origin.at_origin => {
                origin.at_origin = true;
                view.restore(
                    origin.selection.clone(),
                    origin.cursor.clone(),
                    origin.viewport,
                );
            }
            None => {}
        }
        Task::none()
    }

    /// A search hit on the open board: select and centre the first
    /// element whose label contains `query` (§5.18 rule 4). The hit was
    /// found in the board's labels, so a query only its TOML holds finds
    /// nothing here and the board stays as it opened.
    pub(super) fn reveal_on_board(&mut self, query: &str) {
        let Some(view) = &mut self.board else {
            return;
        };
        let entries = labelled(view.board(), view.scene());
        if let Some(entry) = first_containing(&entries, query) {
            view.show_element(entry.element.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use wren_board::{Dir, Id};
    use wren_canvas::{Command as C, Message as Sent, Viewport};
    use wren_core::kind::NoteKind;
    use wren_core::ops::Op;

    use super::super::board::tests::{
        app_at, modified, note_id, open_board, open_markdown, session_text,
    };
    use super::super::board::Surface;
    use super::super::columns::Column;
    use super::super::commands::{Command, CommandId, Disposition};
    use super::super::palette::{Key, Mode, SearchTarget};
    use super::super::preferences::shortcut;
    use super::super::{App, Message};
    use super::*;
    use crate::keys::{Input, Nav};

    /// Labels in every place a person can write one: two lines, a
    /// diamond, a connector with two labels and an empty one, free text,
    /// a kind this build does not know, and three elements with no label.
    const FIND: &str = r#"schema = 1

[node.top]
text = "Top right"
kind = "shape"
shape = "diamond"
at = [300, 0]
size = [120, 60]

[node.left]
text = "Top left\nsecond line"
kind = "shape"
at = [0, 0]
size = [120, 60]

[node.note]
text = "A note in text"
kind = "text"
at = [0, 400]
size = [160, 40]

[node.blank]
text = "  "
kind = "shape"
at = [0, 600]
size = [120, 60]

[node.bare]
kind = "shape"
at = [300, 600]
size = [120, 60]

[node.future]
text = "From the future"
kind = "frame"
at = [600, 600]
size = [120, 60]

[edge.go]
kind = "edge"
from = { node = "left" }
to = { node = "note" }
labels = [{ text = "Go on" }, { text = "" }, { text = "twice" }]

[edge.quiet]
kind = "edge"
from = { node = "top" }
to = { node = "bare" }
"#;

    const SIZE: (f32, f32) = (800.0, 600.0);

    /// A workspace holding `FIND`, another board, a board that does not
    /// parse and a Markdown note, with the setting on.
    fn workspace() -> (tempfile::TempDir, App, PathBuf) {
        let directory = crate::test_support::tempdir();
        let group = directory.path().join("Boards/Find");
        std::fs::create_dir_all(&group).unwrap();
        let path = group.join("Find.wren-board");
        std::fs::write(&path, FIND).unwrap();
        std::fs::write(group.join("Other.wren-board"), FIND).unwrap();
        std::fs::write(group.join("Broken.wren-board"), "<<<<<<< ours\n").unwrap();
        std::fs::write(group.join("Note.md"), "# Note\n\nwords\n").unwrap();
        let app = app_at(directory.path());
        (directory, app, path)
    }

    fn node(id: &str) -> Source {
        Source::Node(Id::new(id).unwrap())
    }

    fn edge(id: &str) -> Source {
        Source::Edge(Id::new(id).unwrap())
    }

    fn view(app: &App) -> &wren_canvas::BoardView {
        app.board.as_ref().expect("a board is open")
    }

    fn lay_out(app: &mut App) {
        let _ = app.update(Message::Board(Sent::Resized {
            width: SIZE.0,
            height: SIZE.1,
        }));
    }

    fn run(app: &mut App, id: CommandId) {
        let _ = app.update(Message::Command(Command::Run(id)));
    }

    fn jump(app: &mut App) {
        run(app, CommandId::Board(C::Jump));
        assert_eq!(app.palette.as_ref().unwrap().mode, Mode::BoardNodes);
    }

    fn key(app: &mut App, nav: Nav) {
        let _ = app.update(Message::Key(Input::Nav(nav)));
    }

    fn selection(app: &App) -> Vec<Source> {
        view(app).machine().selection().iter().cloned().collect()
    }

    fn active(app: &App) -> &str {
        let palette = app.palette.as_ref().unwrap();
        &palette.results[palette.active.unwrap()].title
    }

    fn dirty(app: &App) -> bool {
        app.session.as_ref().unwrap().document().unwrap().dirty()
    }

    /// Where the middle of `element` is drawn in the widget.
    fn middle(app: &App, element: &Source) -> (f32, f32) {
        let view = view(app);
        let bounds = match element {
            Source::Node(id) => hit::Frames::new(view.board()).get(id),
            Source::Edge(_) => view
                .scene()
                .items
                .iter()
                .filter(|item| item.source == *element)
                .map(|item| item.bounds)
                .reduce(|a, b| {
                    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
                    let right = (a.x + a.width).max(b.x + b.width);
                    let bottom = (a.y + a.height).max(b.y + b.height);
                    wren_board::scene::Rect::new(x, y, right - x, bottom - y)
                }),
        };
        let centre = bounds.expect("drawn").center();
        view.viewport().to_screen((centre.x, centre.y))
    }

    #[track_caller]
    fn assert_shown(app: &App, element: &Source) {
        assert_eq!(selection(app), std::slice::from_ref(element));
        assert_eq!(view(app).machine().cursor(), Some(element));
        let (x, y) = middle(app, element);
        assert!(
            (x - SIZE.0 / 2.0).abs() < 0.01 && (y - SIZE.1 / 2.0).abs() < 0.01,
            "{element:?} drawn about {:?}",
            (x, y)
        );
    }

    /// The bytes, the mtime and the draft are as they were: nothing here
    /// is a change to the board (§5.18 rule 2).
    #[track_caller]
    fn assert_untouched(app: &App, path: &Path, mtime: std::time::SystemTime) {
        assert_eq!(std::fs::read_to_string(path).unwrap(), FIND);
        assert_eq!(modified(path), mtime);
        assert_eq!(session_text(app), FIND);
        assert!(!dirty(app));
        assert!(view(app).pristine() && !view(app).can_undo());
    }

    #[test]
    fn the_labelled_elements_read_top_to_bottom_then_left_to_right() {
        let (_directory, mut app, _) = workspace();
        open_board(&mut app, "Find");
        let view = view(&app);
        let listed: Vec<_> = labelled(view.board(), view.scene())
            .into_iter()
            .map(|entry| (entry.element, entry.label, entry.tag))
            .collect();
        let row = |element: Source, label: &str, tag: &str| (element, label.into(), tag.into());
        assert_eq!(
            listed,
            [
                row(node("left"), "Top left\nsecond line", "rect"),
                row(node("top"), "Top right", "diamond"),
                // Placed by its labels, between the two nodes it joins.
                row(edge("go"), "Go on\ntwice", "connector"),
                row(node("note"), "A note in text", "text"),
                row(node("future"), "From the future", "frame"),
            ]
        );
        let entries = labelled(view.board(), view.scene());
        let found = |query: &str| first_containing(&entries, query).map(|entry| &entry.element);
        assert_eq!(found("TOP"), Some(&node("left")));
        assert_eq!(found("  right "), Some(&node("top")));
        assert_eq!(found("twice"), Some(&edge("go")));
        assert_eq!(found("line"), Some(&node("left")));
        for nothing in ["", "   ", "diamond", "kind", "quiet", "schema"] {
            assert_eq!(found(nothing), None, "{nothing:?}");
        }
    }

    /// Listing a board's labels walks its scene once, not once per
    /// connector: at about 10,000 elements the per-connector walk took
    /// 1.4 s in a debug build, which every jump and search hit paid.
    #[test]
    fn listing_ten_thousand_elements_stays_linear() {
        const COLUMNS: usize = 50;
        const ROWS: usize = 100;
        let mut text = String::from("schema = 1\n");
        for row in 0..ROWS {
            for column in 0..COLUMNS {
                let (x, y) = (column * 200, row * 120);
                text.push_str(&format!(
                    "\n[node.n{row}x{column}]\ntext = \"Node {row} {column}\"\nkind = \"shape\"\nat = [{x}, {y}]\nsize = [120, 60]\n"
                ));
                if column > 0 {
                    let from = column - 1;
                    text.push_str(&format!(
                        "\n[edge.e{row}x{column}]\nkind = \"edge\"\nfrom = {{ node = \"n{row}x{from}\" }}\nto = {{ node = \"n{row}x{column}\" }}\nlabels = [{{ text = \"to {column}\" }}]\n"
                    ));
                }
            }
        }
        let (_directory, app, _) = workspace();
        crate::fonts::load();
        let palette = crate::theme::board_palette(app.definition);
        let view = wren_canvas::BoardView::open(&text, palette, crate::fonts::UI).unwrap();
        assert_eq!(
            view.board().nodes.len() + view.board().edges.len(),
            ROWS * (2 * COLUMNS - 1)
        );
        let started = std::time::Instant::now();
        let entries = labelled(view.board(), view.scene());
        let took = started.elapsed();
        assert_eq!(entries.len(), ROWS * (2 * COLUMNS - 1));
        // The first row's nodes, then its connectors' labels, drawn lower.
        let element = |at: usize| &entries[at].element;
        assert_eq!(element(COLUMNS - 1), &node(&format!("n0x{}", COLUMNS - 1)));
        assert_eq!(element(COLUMNS), &edge("e0x1"));
        assert_eq!(
            element(2 * COLUMNS - 2),
            &edge(&format!("e0x{}", COLUMNS - 1))
        );
        // Order-of-magnitude only, as the debug ceilings of §6.7.
        assert!(took.as_millis() < 250, "listed in {took:?}");
    }

    #[test]
    fn the_jump_palette_lists_each_label_once_and_find_and_outline_open_it() {
        let (_directory, mut app, _) = workspace();
        open_board(&mut app, "Find");
        lay_out(&mut app);
        jump(&mut app);
        let palette = app.palette.as_ref().unwrap();
        let rows: Vec<_> = palette
            .results
            .iter()
            .map(|row| {
                let tag = row.tag.as_deref().unwrap();
                (row.title.as_str(), row.path.as_str(), tag, row.current)
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("Top left", "second line", "rect", false),
                ("Top right", "", "diamond", false),
                ("Go on", "twice", "connector", false),
                ("A note in text", "", "text", false),
                ("From the future", "", "frame", false),
            ]
        );
        assert_eq!(palette.active, Some(0));
        // Opening shows nothing.
        assert!(selection(&app).is_empty());
        // Matched on the whole label: the second line too.
        let _ = app.update(Message::PaletteQuery("twice".into()));
        assert_eq!(active(&app), "Go on");
        let _ = app.update(Message::Escape);
        assert!(app.palette.is_none() && app.board_nodes.is_none());

        // Find and Outline open the same palette on a board.
        for id in [CommandId::Find, CommandId::Outline] {
            let _ = app.update(shortcut(id));
            assert_eq!(app.palette.as_ref().unwrap().mode, Mode::BoardNodes);
            assert!(!app.find_open);
            let _ = app.update(Message::Escape);
        }

        // Not on a Markdown note, and on a board shown as raw text Find is
        // the editor's own and there is no outline.
        open_markdown(&mut app, "Note");
        let opened = app.open_palette(Mode::BoardNodes);
        assert_eq!(opened.processed.disposition, Disposition::Blocked);
        assert!(!app.command_enabled(CommandId::Board(C::Jump)));
        open_board(&mut app, "Broken");
        assert_eq!(app.surface(), Surface::Raw);
        let opened = app.open_palette(Mode::BoardNodes);
        assert_eq!(opened.processed.disposition, Disposition::Blocked);
        let _ = app.update(shortcut(CommandId::Find));
        assert!(app.find_open && app.palette.is_none());
        assert!(!app.command_enabled(CommandId::Outline));
    }

    #[test]
    fn the_highlight_shows_each_element_and_escape_puts_the_board_back() {
        let (_directory, mut app, path) = workspace();
        let mtime = modified(&path);
        open_board(&mut app, "Find");
        lay_out(&mut app);
        // The arrow selects the top-left node, as the cursor.
        run(&mut app, CommandId::Board(C::Select(Dir::Down)));
        assert_eq!(selection(&app), [node("left")]);
        let origin = view(&app).viewport();
        let revision = app.board_revision;

        jump(&mut app);
        // It opens on the cursor and shows nothing until the highlight moves.
        assert_eq!(active(&app), "Top left");
        assert!(app.palette.as_ref().unwrap().results[0].current);
        assert_eq!(view(&app).viewport(), origin);
        key(&mut app, Nav::Down);
        assert_eq!(active(&app), "Top right");
        assert_shown(&app, &node("top"));
        assert_eq!(view(&app).viewport().zoom, origin.zoom);
        key(&mut app, Nav::Down);
        assert_shown(&app, &edge("go"));
        // The tick, like any message, moves nothing once a row is shown.
        let shown = view(&app).viewport();
        let _ = app.update(Message::Tick);
        assert_eq!(view(&app).viewport(), shown);
        // No match shows the origin; a match shows its element again.
        let _ = app.update(Message::PaletteQuery("zzz".into()));
        assert_eq!(selection(&app), [node("left")]);
        assert_eq!(view(&app).viewport(), origin);
        let _ = app.update(Message::PaletteQuery("note".into()));
        assert_shown(&app, &node("note"));

        let _ = app.update(Message::Escape);
        assert!(app.palette.is_none() && app.board_nodes.is_none());
        assert_eq!(selection(&app), [node("left")]);
        assert_eq!(view(&app).machine().cursor(), Some(&node("left")));
        assert_eq!(view(&app).viewport(), origin);

        // Any other close puts it back too.
        jump(&mut app);
        key(&mut app, Nav::Down);
        assert_shown(&app, &node("top"));
        let _ = app.update(Message::PaletteClose);
        assert_eq!(selection(&app), [node("left")]);
        assert_eq!(view(&app).viewport(), origin);
        jump(&mut app);
        key(&mut app, Nav::Down);
        run(&mut app, CommandId::CommandPalette);
        assert_eq!(app.palette.as_ref().unwrap().mode, Mode::Commands);
        assert_eq!(selection(&app), [node("left")]);
        assert_eq!(view(&app).viewport(), origin);
        let _ = app.update(Message::Escape);

        assert_eq!(app.board_revision, revision);
        assert_untouched(&app, &path, mtime);
    }

    #[test]
    fn enter_keeps_the_element_and_gives_the_board_the_keyboard() {
        let (_directory, mut app, path) = workspace();
        let mtime = modified(&path);
        open_board(&mut app, "Find");
        lay_out(&mut app);
        app.column = Column::Notes;
        jump(&mut app);
        key(&mut app, Nav::Down);
        key(&mut app, Nav::Down);
        key(&mut app, Nav::Enter);
        assert!(app.palette.is_none() && app.board_nodes.is_none());
        assert_eq!(app.column, Column::Editor);
        assert_shown(&app, &edge("go"));
        let _ = app.update(Message::Tick);
        assert_shown(&app, &edge("go"));

        // Enter on the row it opens on, the cursor's, shows that element.
        let elsewhere = view(&app).viewport().panned(-300.0, 200.0);
        app.board.as_mut().unwrap().set_viewport(elsewhere);
        jump(&mut app);
        assert_eq!(active(&app), "Go on");
        assert_eq!(view(&app).viewport(), elsewhere);
        key(&mut app, Nav::Enter);
        assert_shown(&app, &edge("go"));
        // A stale row is refused.
        jump(&mut app);
        let generation = app.palette.as_ref().unwrap().generation;
        let _ = app.update(Message::PaletteChoose(
            generation,
            Key::Element(node("blank")),
        ));
        assert!(app.palette.is_some());
        let _ = app.update(Message::Escape);
        assert_untouched(&app, &path, mtime);
    }

    #[test]
    fn rows_follow_a_reload_and_the_palette_closes_when_the_note_changes() {
        let (_directory, mut app, path) = workspace();
        open_board(&mut app, "Find");
        lay_out(&mut app);
        jump(&mut app);
        key(&mut app, Nav::Down);
        assert_shown(&app, &node("top"));

        // A node above every other: the first row now, and the highlight
        // stays on the element it was on.
        let above = format!(
            "{FIND}\n[node.above]\ntext = \"Above all\"\nkind = \"shape\"\nat = [0, -200]\nsize = [120, 60]\n"
        );
        std::fs::write(&path, &above).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        let _ = app.update(Message::Tick);
        let palette = app.palette.as_ref().unwrap();
        assert_eq!(palette.results[0].title, "Above all");
        assert_eq!(palette.results.len(), 6);
        assert_eq!(active(&app), "Top right");
        assert_eq!(selection(&app), [node("top")]);
        // Gone from the file: another row, which is shown.
        std::fs::write(&path, above.replace("Top right", "")).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        let _ = app.update(Message::Tick);
        let palette = app.palette.as_ref().unwrap();
        assert_eq!(palette.results.len(), 5);
        let Key::Element(highlighted) = &palette.results[palette.active.unwrap()].key else {
            panic!("an element row");
        };
        assert_eq!(selection(&app), std::slice::from_ref(highlighted));

        // Another note closes it, a board as well as a Markdown note.
        for (title, kind) in [("Other", NoteKind::Board), ("Note", NoteKind::Markdown)] {
            open_board(&mut app, "Find");
            lay_out(&mut app);
            jump(&mut app);
            key(&mut app, Nav::Down);
            let note = note_id(&app, title, kind);
            let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
            let _ = app.update(Message::Tick);
            assert!(
                app.palette.is_none() && app.board_nodes.is_none(),
                "{title}"
            );
        }
        assert!(!dirty(&app));
    }

    #[test]
    fn a_note_change_under_a_preview_leaves_the_board_where_it_was() {
        let (_directory, mut app, path) = workspace();
        let mtime = modified(&path);
        open_board(&mut app, "Find");
        lay_out(&mut app);
        let origin = view(&app).viewport();
        let preview = |app: &mut App| {
            jump(app);
            key(app, Nav::Down);
            key(app, Nav::Down);
            assert_shown(app, &edge("go"));
            assert_ne!(view(app).viewport(), origin);
        };

        preview(&mut app);
        open_markdown(&mut app, "Note");
        let _ = app.update(Message::Tick);
        assert!(app.palette.is_none() && app.board_nodes.is_none());
        open_board(&mut app, "Find");
        lay_out(&mut app);
        assert_eq!(view(&app).viewport(), origin);
        assert!(selection(&app).is_empty());
        assert_untouched(&app, &path, mtime);

        // The board turning to raw text underneath, and back.
        preview(&mut app);
        std::fs::write(&path, "<<<<<<< ours\n").unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        let _ = app.update(Message::Tick);
        assert_eq!(app.surface(), Surface::Raw);
        assert!(app.palette.is_none() && app.board_nodes.is_none());
        std::fs::write(&path, FIND).unwrap();
        let _ = app.apply_core(Op::Reconcile).unwrap();
        let _ = app.update(Message::Tick);
        assert_eq!(app.surface(), Surface::Board);
        lay_out(&mut app);
        assert_eq!(view(&app).viewport(), origin);
        assert!(!dirty(&app));
    }

    #[test]
    fn a_search_hit_selects_and_centres_the_first_element_holding_the_query() {
        let (_directory, mut app, path) = workspace();
        let mtime = modified(&path);
        let find = note_id(&app, "Find", NoteKind::Board);
        // As the search palette does before it searches.
        app.session.as_mut().unwrap().settle_search();
        let target = |app: &App, query: &str| {
            let session = app.session.as_ref().unwrap();
            let hits = session.index().query(
                query,
                wren_core::index::Scope::Workspace,
                wren_core::index::QueryKind::Search,
                10,
            );
            SearchTarget {
                generation: app.workspace_generation(),
                index_revision: session.index().revision(),
                note: find,
                query: query.into(),
                content_match: hits
                    .iter()
                    .find(|hit| hit.note == find)
                    .and_then(|hit| hit.content_match.clone()),
            }
        };
        let land = |app: &mut App, target: SearchTarget| {
            open_markdown(app, "Note");
            let outcome = app.dispatch(Command::OpenNote(target));
            assert!(outcome.processed.disposition.committed());
            assert_eq!(app.surface(), Surface::Board);
        };

        // The first time the board opens it waits for its size, fitted,
        // and the element is centred at the zoom the fit gives.
        let hit = target(&app, "TWICE");
        assert!(hit.content_match.is_some());
        land(&mut app, hit);
        assert_eq!(selection(&app), [edge("go")]);
        lay_out(&mut app);
        assert_shown(&app, &edge("go"));
        let fitted = Viewport::fitting(view(&app).scene().bounds, SIZE);
        assert_eq!(view(&app).viewport().zoom, fitted.zoom);

        // Opened again it keeps the zoom it was left at.
        run(&mut app, CommandId::TextBigger);
        let zoom = view(&app).viewport().zoom;
        assert_ne!(zoom, fitted.zoom);
        let hit = target(&app, "top");
        land(&mut app, hit);
        lay_out(&mut app);
        assert_shown(&app, &node("left"));
        assert_eq!(view(&app).viewport().zoom, zoom);

        // A title hit, and a query that only the file's syntax holds, show
        // nothing.
        let hit = target(&app, "Find");
        land(&mut app, hit);
        lay_out(&mut app);
        assert!(selection(&app).is_empty());
        let mut syntax = target(&app, "diamond");
        assert!(syntax.content_match.is_none());
        syntax.content_match = Some(0..7);
        land(&mut app, syntax);
        lay_out(&mut app);
        assert!(selection(&app).is_empty());

        assert_untouched(&app, &path, mtime);
    }
}
