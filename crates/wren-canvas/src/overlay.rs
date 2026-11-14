//! What the widget draws over a board's scene (the plan's §5.2, §5.3,
//! §5.8, §5.9 and §5.12): the selection, the cursor ring, the handles, the
//! marquee, the node a dragged connector end would attach to, the open
//! label's caret and selected text, the mode chip and the empty board's
//! hint. None of it is the board's, so none of it is in the scene, an
//! export or a draft.
//!
//! [`marks`] decides what is drawn, in widget-local pixels, as a pure
//! function of the view; [`paint`] draws it.

use std::time::Instant;

use iced::advanced::text::{LineHeight, Shaping};
use iced::widget::canvas::{self, Frame, LineDash, Path, Stroke};
use iced::{alignment, Pixels, Size};
use wren_board::edit::EdgeEnd;
use wren_board::scene::{self, Draw, Face, Measure as _, Rect, Source};
use wren_board::Rgba;

use crate::command::Tool;
use crate::hit::Frames;
use crate::paint::{self, Cull};
use crate::{handles, machine, BoardView};

/// How far outside a selected node's box its outline is drawn, in screen
/// pixels.
const OUTLINE_GAP: f32 = 3.0;
/// How far outside the cursor element its ring is drawn, in screen pixels:
/// outside the selection outline, so the two never meet.
const RING_GAP: f32 = 7.0;
/// Where the mode chip sits, from the widget's top-left corner.
const CHIP_INSET: f32 = 12.0;
/// Room between the chip's edge and its text, across and down.
const CHIP_PAD: (f32, f32) = (8.0, 4.0);
const CHIP_TEXT: f32 = 12.0;
const HINT_TEXT: f32 = 14.0;
/// How wide the open label's caret is drawn, in screen pixels.
const CARET_WIDTH: f32 = 1.5;

/// The mode chip in keyboard Connecting, before its query (§5.2).
const CONNECT_TO: &str = "Connect to";

/// What the empty board says (§5.2).
pub(crate) const HINT: &str = "Tab adds a shape. R O D U T pick a tool.";

/// One thing drawn over the scene. Points and boxes are widget-local
/// pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Mark {
    /// Around a selected node.
    Outline(Rect),
    /// A selected connector: this stroked item of the shown scene, drawn
    /// again.
    Line(usize),
    /// This item of the shown scene, painted again as it is: a selected
    /// connector's label, which its highlight would strike through.
    Over(usize),
    /// Around the cursor element while the selection is something else.
    Ring(Rect),
    /// A resize handle's centre.
    Handle((f32, f32)),
    /// A connect handle's centre.
    Connect((f32, f32)),
    /// A connector end's handle.
    End((f32, f32)),
    /// A side handle of the node a connector end is dragged over, and
    /// whether a release here pins the end to that side (§5.12 rule 5).
    Side((f32, f32), bool),
    /// From where a connector's end is to the pointer dragging it, while a
    /// release would leave the end there (§5.12 rule 6).
    Band((f32, f32), (f32, f32)),
    Marquee(Rect),
    /// Behind the caret of a connector's label that has no text yet: the
    /// scene backs a label only once it has some, and the caret would be
    /// lost on the connector's own line.
    Blank(Rect),
    /// One line's worth of the open label's selected text.
    Selected(Rect),
    /// The open label's caret, [`CARET_WIDTH`] wide, in the colour the
    /// label is drawn in.
    Caret(Rect, Rgba),
    /// The armed tool's name, or keyboard Connecting's query.
    Chip(String),
    Hint,
}

/// The marks over a board, and whether it has the keyboard: an unfocused
/// board draws its selection muted, with no handles or cursor ring (§5.3
/// rule 7).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Overlay {
    pub(crate) marks: Vec<Mark>,
    pub(crate) focused: bool,
}

/// What is drawn over the view's board as it is shown now: a gesture's
/// preview moves the outlines and handles with it.
pub(crate) fn marks(view: &BoardView) -> Overlay {
    let machine = view.machine();
    let picture = view.picture();
    let viewport = view.viewport();
    let focused = machine.focused();
    let frames = Frames::new(picture.board);
    let screen = |bounds: Rect| {
        let (x, y) = viewport.to_screen((bounds.x, bounds.y));
        Rect::new(
            x,
            y,
            bounds.width * viewport.zoom,
            bounds.height * viewport.zoom,
        )
    };
    let mut marks = Vec::new();

    let selection = machine.selection();
    for element in selection {
        let id = match element {
            Source::Node(id) => {
                if let Some(bounds) = frames.get(id) {
                    marks.push(Mark::Outline(grown(screen(bounds), OUTLINE_GAP)));
                }
                continue;
            }
            Source::Edge(id) => id,
        };
        let stroked = |item: &scene::Item| {
            matches!(
                &item.draw,
                Draw::Path {
                    stroke: Some(_),
                    ..
                }
            )
        };
        let items = picture.scene.items.iter().enumerate();
        let own: Vec<(usize, bool)> = items
            .filter(|(_, item)| matches!(&item.source, Source::Edge(edge) if edge == id))
            .map(|(index, item)| (index, stroked(item)))
            .collect();
        let part = |lines: bool| own.iter().filter(move |(_, line)| *line == lines);
        marks.extend(part(true).map(|(index, _)| Mark::Line(*index)));
        marks.extend(part(false).map(|(index, _)| Mark::Over(*index)));
    }

    if focused {
        if let Some(cursor) = machine.cursor() {
            let alone = selection.len() == 1 && selection.contains(cursor);
            let drawn = crate::bounds(picture.board, picture.scene, cursor);
            if let Some(bounds) = drawn.filter(|_| !alone) {
                marks.push(Mark::Ring(grown(screen(bounds), RING_GAP)));
            }
        }
        // Which handles show is the machine's, read against the board;
        // where they sit follows what is shown.
        let shown = machine.handles(&machine::View {
            viewport,
            now: Instant::now(),
            board: view.board(),
            scene: view.scene(),
            writable: view.writable(),
            size: view.size(),
        });
        if let Some(bounds) = shown.resize.and_then(|id| frames.get(id)) {
            let resize = handles::resize(bounds, viewport);
            marks.extend(resize.into_iter().map(|(_, at)| Mark::Handle(at)));
        }
        if let Some(bounds) = shown.connect.and_then(|id| frames.get(id)) {
            let connect = handles::connect(bounds, viewport);
            marks.extend(connect.into_iter().map(|(_, at)| Mark::Connect(at)));
        }
        let ends = shown
            .ends
            .and_then(|id| handles::ends(picture.board, picture.scene, id));
        for end in ends.into_iter().flatten() {
            marks.push(Mark::End(viewport.to_screen((end.x, end.y))));
        }
    }

    if let Some(connecting) = machine.connecting() {
        let target = connecting.target.and_then(|(id, pinned)| {
            Some((id, picture.board.nodes.get(id)?, frames.get(id)?, pinned))
        });
        if let Some((id, node, bounds, pinned)) = target {
            if !selection.contains(&Source::Node(id.clone())) {
                marks.push(Mark::Outline(grown(screen(bounds), OUTLINE_GAP)));
            }
            let sides = handles::sides(node, bounds, viewport);
            marks.extend(
                sides
                    .into_iter()
                    .map(|(side, at)| Mark::Side(at, pinned == Some(side))),
            );
        }
        let home = connecting.loose.and_then(|(id, end)| {
            let [from, to] = handles::ends(picture.board, picture.scene, id)?;
            Some(match end {
                EdgeEnd::From => from,
                EdgeEnd::To => to,
            })
        });
        if let Some(home) = home {
            let to = viewport.to_screen(connecting.at);
            marks.push(Mark::Band(viewport.to_screen((home.x, home.y)), to));
            marks.push(Mark::End(to));
        }
    }

    // Keyboard Connecting outlines its target; the source is the
    // selection.
    let seeking = machine.seeking();
    let target = seeking.and_then(|seeking| seeking.target);
    if let Some(bounds) = target
        .filter(|id| !selection.contains(&Source::Node((*id).clone())))
        .and_then(|id| frames.get(id))
    {
        marks.push(Mark::Outline(grown(screen(bounds), OUTLINE_GAP)));
    }

    if let Some(area) = machine.marquee() {
        marks.push(Mark::Marquee(screen(area)));
    }
    if let Some(label) = view.label() {
        let measurer = view.measurer();
        let connector = label
            .element()
            .edge()
            .is_some_and(|id| picture.board.edges.contains_key(id));
        if connector && label.stored().is_empty() {
            let [across, down] = scene::EDGE_LABEL_PAD;
            let lines = label.bounds(measurer);
            marks.push(Mark::Blank(screen(Rect::new(
                lines.x - across,
                lines.y - down,
                lines.width + 2.0 * across,
                lines.height + 2.0 * down,
            ))));
        }
        let selected = label.selected(measurer).into_iter();
        marks.extend(selected.map(|line| Mark::Selected(screen(line))));
        // A board without the keyboard has no caret: nothing typed would
        // go there.
        if focused {
            let caret = screen(label.caret_box(measurer));
            let caret = Rect::new(
                caret.x - CARET_WIDTH / 2.0,
                caret.y,
                CARET_WIDTH,
                caret.height,
            );
            marks.push(Mark::Caret(caret, label.ink));
        }
    }
    if let Some(seeking) = seeking {
        marks.push(Mark::Chip(match seeking.query {
            "" => CONNECT_TO.to_owned(),
            query => format!("{CONNECT_TO}: {query}"),
        }));
    } else if let Some(name) = tool_name(machine.tool()) {
        marks.push(Mark::Chip(name.to_owned()));
    }
    let empty = picture.board.nodes.is_empty() && picture.board.edges.is_empty();
    if empty && view.writable() {
        marks.push(Mark::Hint);
    }
    Overlay { marks, focused }
}

/// The name the mode chip gives an armed tool; none for Select.
fn tool_name(tool: Tool) -> Option<&'static str> {
    use wren_board::Shape;
    Some(match tool {
        Tool::Select => return None,
        Tool::Hand => "Hand",
        Tool::Text => "Text",
        Tool::Connector => "Connector",
        Tool::Shape(shape) => match shape {
            Shape::Rect => "Rectangle",
            Shape::Rounded => "Rounded",
            Shape::Pill => "Pill",
            Shape::Ellipse => "Ellipse",
            Shape::Diamond => "Diamond",
            Shape::Parallelogram => "Parallelogram",
            Shape::Cylinder => "Cylinder",
            Shape::Document => "Document",
        },
    })
}

fn grown(bounds: Rect, by: f32) -> Rect {
    Rect::new(
        bounds.x - by,
        bounds.y - by,
        bounds.width + 2.0 * by,
        bounds.height + 2.0 * by,
    )
}

/// Draw `overlay` over the view in a widget of `size`, framed by `cull`.
pub(crate) fn paint(frame: &mut Frame, overlay: &Overlay, view: &BoardView, cull: &Cull) {
    let palette = view.palette();
    let colour = paint::color(if overlay.focused {
        palette.accent
    } else {
        palette.muted
    });
    let canvas_colour = paint::color(palette.canvas);
    let solid = |width: f32| Stroke {
        style: canvas::Style::Solid(colour),
        width,
        ..Stroke::default()
    };
    let rect = |bounds: Rect| {
        Path::rectangle(
            iced::Point::new(bounds.x, bounds.y),
            Size::new(bounds.width, bounds.height),
        )
    };
    let square = |(x, y): (f32, f32)| {
        let half = handles::SIZE / 2.0;
        Path::rectangle(
            iced::Point::new(x - half, y - half),
            Size::new(handles::SIZE, handles::SIZE),
        )
    };
    let dot = |(x, y): (f32, f32)| Path::circle(iced::Point::new(x, y), handles::SIZE / 2.0);
    let scene = view.picture().scene;
    let size = frame.size();
    for mark in &overlay.marks {
        match mark {
            Mark::Outline(bounds) => frame.stroke(&rect(*bounds), solid(1.5)),
            Mark::Line(index) => {
                if let Some(item) = scene.items.get(*index) {
                    paint::trace(frame, item, cull, solid(3.0));
                }
            }
            Mark::Over(index) => {
                if let Some(item) = scene.items.get(*index) {
                    paint::item(frame, item, cull, view.measurer());
                }
            }
            Mark::Blank(bounds) => frame.fill(&rect(*bounds), canvas_colour),
            Mark::Ring(bounds) => frame.stroke(
                &rect(*bounds),
                Stroke {
                    line_dash: LineDash {
                        segments: &[4.0, 3.0],
                        offset: 0,
                    },
                    ..solid(1.5)
                },
            ),
            Mark::Handle(at) => {
                let path = square(*at);
                frame.fill(&path, canvas_colour);
                frame.stroke(&path, solid(1.5));
            }
            Mark::Connect(at) => {
                let path = dot(*at);
                frame.fill(&path, canvas_colour);
                frame.stroke(&path, solid(1.5));
            }
            Mark::End(at) | Mark::Side(at, true) => frame.fill(&dot(*at), colour),
            Mark::Side(at, false) => {
                let path = dot(*at);
                frame.fill(&path, canvas_colour);
                frame.stroke(&path, solid(1.5));
            }
            Mark::Band(from, to) => frame.stroke(
                &Path::line(
                    iced::Point::new(from.0, from.1),
                    iced::Point::new(to.0, to.1),
                ),
                Stroke {
                    line_dash: LineDash {
                        segments: &[4.0, 3.0],
                        offset: 0,
                    },
                    ..solid(1.5)
                },
            ),
            Mark::Marquee(bounds) => {
                let path = rect(*bounds);
                frame.fill(&path, colour.scale_alpha(0.12));
                frame.stroke(&path, solid(1.0));
            }
            // Over the text, which shows through it.
            Mark::Selected(bounds) => frame.fill(&rect(*bounds), colour.scale_alpha(0.3)),
            Mark::Caret(bounds, ink) => frame.fill(&rect(*bounds), paint::color(*ink)),
            Mark::Chip(name) => {
                let width = view.measurer().width(name, Face::Regular, CHIP_TEXT);
                let height = scene::line_height(CHIP_TEXT);
                let chip = Path::rounded_rectangle(
                    iced::Point::new(CHIP_INSET, CHIP_INSET),
                    Size::new(width + 2.0 * CHIP_PAD.0, height + 2.0 * CHIP_PAD.1),
                    4.0.into(),
                );
                frame.fill(&chip, paint::color(palette.raised));
                frame.stroke(
                    &chip,
                    Stroke {
                        style: canvas::Style::Solid(paint::color(palette.muted)),
                        width: 1.0,
                        ..Stroke::default()
                    },
                );
                let at = (CHIP_INSET + CHIP_PAD.0, CHIP_INSET + CHIP_PAD.1);
                text(frame, view, name, at, CHIP_TEXT, palette.ink);
            }
            Mark::Hint => {
                let width = view.measurer().width(HINT, Face::Regular, HINT_TEXT);
                let height = scene::line_height(HINT_TEXT);
                let at = (
                    ((size.width - width) / 2.0).max(0.0),
                    (size.height - height) / 2.0,
                );
                text(frame, view, HINT, at, HINT_TEXT, palette.muted);
            }
        }
    }
}

/// One line of text in the label face, its top-left at `at`.
fn text(
    frame: &mut Frame,
    view: &BoardView,
    content: &str,
    at: (f32, f32),
    size: f32,
    colour: wren_board::Rgba,
) {
    let width = view.measurer().width(content, Face::Regular, size);
    frame.fill_text(canvas::Text {
        content: content.to_owned(),
        position: iced::Point::new(at.0, at.1),
        max_width: width + size,
        color: paint::color(colour),
        size: Pixels(size),
        line_height: LineHeight::Absolute(Pixels(scene::line_height(size))),
        font: view.measurer().font(Face::Regular),
        align_x: iced::advanced::text::Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
    });
}

#[cfg(test)]
mod tests {
    use wren_board::Shape;

    use super::*;
    use crate::machine::{Button, Input, Key, State};
    use crate::{fixtures, Command, Message, Update, Viewport};

    fn open(text: &str) -> BoardView {
        fixtures::load_fonts();
        let mut view = BoardView::open(text, fixtures::palette(), fixtures::FONT).expect("a board");
        view.update(
            Message::Resized {
                width: 800.0,
                height: 600.0,
            },
            Instant::now(),
        );
        view
    }

    fn input(view: &mut BoardView, input: Input) -> Update {
        view.update(Message::Input(view.epoch(), input), Instant::now())
    }

    fn click(view: &mut BoardView, at: (f32, f32)) {
        input(
            view,
            Input::Press {
                button: Button::Left,
                at,
            },
        );
        input(
            view,
            Input::Release {
                button: Button::Left,
            },
        );
    }

    /// The marks of a kind, by a test of each.
    fn count(overlay: &Overlay, kind: fn(&Mark) -> bool) -> usize {
        overlay.marks.iter().filter(|mark| kind(mark)).count()
    }

    #[test]
    fn a_selected_node_shows_its_outline_and_eight_handles() {
        let mut view = open(fixtures::TWO);
        click(&mut view, (60.0, 30.0));
        let overlay = marks(&view);
        assert!(overlay.focused);
        // Three pixels outside the box, which is at 100 % from the origin.
        assert_eq!(
            overlay.marks[0],
            Mark::Outline(Rect::new(-3.0, -3.0, 126.0, 66.0))
        );
        let handles: Vec<&Mark> = overlay
            .marks
            .iter()
            .filter(|mark| matches!(mark, Mark::Handle(_)))
            .collect();
        assert_eq!(handles.len(), 8);
        assert_eq!(handles[0], &Mark::Handle((0.0, 0.0)));
        assert_eq!(handles[4], &Mark::Handle((120.0, 60.0)));
        // The selection is the cursor alone: no ring, no chip, no hint.
        assert_eq!(overlay.marks.len(), 9);
    }

    #[test]
    fn the_cursor_has_a_ring_unless_it_is_the_whole_selection() {
        let mut view = open(fixtures::TWO);
        click(&mut view, (60.0, 30.0));
        input(
            &mut view,
            Input::Modifiers(crate::Mods {
                shift: true,
                duplicate: false,
            }),
        );
        click(&mut view, (360.0, 30.0));
        let overlay = marks(&view);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Outline(_))), 2);
        // Two selected: no handles, and a ring around the cursor, `b`.
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Handle(_))), 0);
        let rings: Vec<&Mark> = overlay
            .marks
            .iter()
            .filter(|mark| matches!(mark, Mark::Ring(_)))
            .collect();
        assert_eq!(rings, [&Mark::Ring(Rect::new(293.0, -7.0, 134.0, 74.0))]);
        // Nothing selected: the ring stays where the cursor is.
        input(
            &mut view,
            Input::Key {
                key: Key::Escape,
                repeat: false,
            },
        );
        let overlay = marks(&view);
        assert_eq!(
            overlay.marks,
            [Mark::Ring(Rect::new(293.0, -7.0, 134.0, 74.0))]
        );
    }

    #[test]
    fn an_unfocused_board_draws_its_selection_muted_with_no_handles_or_ring() {
        let mut view = open(fixtures::TWO);
        click(&mut view, (60.0, 30.0));
        input(
            &mut view,
            Input::Modifiers(crate::Mods {
                shift: true,
                duplicate: false,
            }),
        );
        click(&mut view, (360.0, 30.0));
        input(&mut view, Input::Focus(false));
        assert_eq!(view.machine().state(), State::Unfocused);
        let overlay = marks(&view);
        assert!(!overlay.focused);
        assert_eq!(
            overlay.marks,
            [
                Mark::Outline(Rect::new(-3.0, -3.0, 126.0, 66.0)),
                Mark::Outline(Rect::new(297.0, -3.0, 126.0, 66.0)),
            ]
        );
    }

    #[test]
    fn a_selected_connector_is_traced_and_shows_its_two_ends() {
        let mut view = open(fixtures::TWO);
        click(&mut view, (210.0, 30.0));
        let overlay = marks(&view);
        let lines: Vec<usize> = overlay
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Line(index) => Some(*index),
                _ => None,
            })
            .collect();
        assert!(!lines.is_empty());
        for index in lines {
            let item = &view.scene().items[index];
            assert!(matches!(&item.source, Source::Edge(id) if id.as_str() == "e"));
            assert!(matches!(
                &item.draw,
                Draw::Path {
                    stroke: Some(_),
                    ..
                }
            ));
        }
        let ends: Vec<(f32, f32)> = overlay
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::End(at) => Some(*at),
                _ => None,
            })
            .collect();
        // From the right of `a` to the arrowhead's tip at the left of `b`.
        assert_eq!(ends.len(), 2);
        assert!((ends[0].0 - 120.0).abs() < 2.0 && (ends[0].1 - 30.0).abs() < 0.5);
        assert!((ends[1].0 - 300.0).abs() < 2.0 && (ends[1].1 - 30.0).abs() < 0.5);
    }

    /// The highlight is drawn over the scene, so what the scene draws
    /// over a connector's line is painted again over the highlight: its
    /// label stays readable, and its arrowhead as it was.
    #[test]
    fn a_selected_connector_s_label_is_painted_over_its_highlight() {
        let mut view = open(fixtures::HAND_WRITTEN);
        let flow = Source::Edge(wren_board::Id::new("flow").expect("an id"));
        view.machine
            .select(std::collections::BTreeSet::from([flow.clone()]), Some(flow));
        let overlay = marks(&view);
        let kinds: Vec<&str> = overlay
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Line(_) => Some("line"),
                Mark::Over(index) => Some(match &view.scene().items[*index].draw {
                    Draw::Text { .. } => "text",
                    Draw::Path { .. } => "fill",
                }),
                _ => None,
            })
            .collect();
        // The line traced; then the arrowhead, the label's backing and
        // its text, in the scene's order.
        assert_eq!(kinds, ["line", "fill", "fill", "text"]);
    }

    #[test]
    fn a_connector_s_blank_label_has_a_backing_for_its_caret() {
        let mut view = open(fixtures::TWO);
        click(&mut view, (210.0, 30.0));
        view.command(Command::EditLabel, Instant::now());
        let overlay = marks(&view);
        // One 12 px line of no width at the middle of the route, padded as
        // the scene pads a connector's label, and the caret on it.
        let blank = overlay.marks.iter().find_map(|mark| match mark {
            Mark::Blank(bounds) => Some(*bounds),
            _ => None,
        });
        let blank = blank.expect("a backing");
        assert!((blank.x - 206.0).abs() < 0.01, "{blank:?}");
        assert_eq!((blank.y, blank.width, blank.height), (20.0, 8.0, 20.0));
        let at = |mark: &Mark| overlay.marks.iter().position(|other| other == mark);
        let caret = overlay
            .marks
            .iter()
            .find(|mark| matches!(mark, Mark::Caret(..)));
        assert!(at(&Mark::Blank(blank)) < caret.and_then(at));
        // A node's label needs none, nor does a connector's with text.
        let mut view = open(fixtures::HAND_WRITTEN);
        input(&mut view, Input::Focus(true));
        for name in ["first", "flow"] {
            let id = wren_board::Id::new(name).expect("an id");
            let id = Source::of(view.board(), &id).expect("an element");
            view.machine
                .select(std::collections::BTreeSet::from([id.clone()]), Some(id));
            view.command(Command::EditLabel, Instant::now());
            let overlay = marks(&view);
            assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Blank(_))), 0);
            assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Caret(..))), 1);
        }
        // Nor does a node's blank label where a connector shares its id
        // (§4.12).
        let board = "schema = 1\n\n[node.s]\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\nz = \"V\"\n\n\
                     [edge.s]\nkind = \"edge\"\nfrom = { at = [400, 0] }\nto = { at = [400, 200] }\nz = \"W\"\n";
        let mut view = open(board);
        input(&mut view, Input::Focus(true));
        let node = Source::Node(wren_board::Id::new("s").expect("an id"));
        view.machine
            .select(std::collections::BTreeSet::from([node.clone()]), Some(node));
        view.command(Command::EditLabel, Instant::now());
        let overlay = marks(&view);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Blank(_))), 0);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Caret(..))), 1);
    }

    #[test]
    fn outlines_and_handles_follow_a_drag_and_the_zoom() {
        let mut view = open(fixtures::TWO);
        view.set_viewport(Viewport {
            x: -100.0,
            y: -50.0,
            zoom: 2.0,
        });
        // `a`'s middle is now at (320, 160).
        click(&mut view, (320.0, 160.0));
        input(
            &mut view,
            Input::Press {
                button: Button::Left,
                at: (320.0, 160.0),
            },
        );
        input(&mut view, Input::Move { at: (360.0, 160.0) });
        let overlay = marks(&view);
        // Moved by one cell in the preview: 40 pixels at 200 %.
        assert_eq!(
            overlay.marks[0],
            Mark::Outline(Rect::new(237.0, 97.0, 246.0, 126.0))
        );
        assert!(overlay.marks.contains(&Mark::Handle((240.0, 100.0))));
        assert_eq!(
            view.board().nodes.values().next().map(|node| node.at),
            Some([0, 0])
        );
    }

    #[test]
    fn a_marquee_is_drawn_as_it_is_dragged() {
        let mut view = open(fixtures::TWO);
        input(
            &mut view,
            Input::Press {
                button: Button::Left,
                at: (500.0, 300.0),
            },
        );
        input(&mut view, Input::Move { at: (440.0, 200.0) });
        let overlay = marks(&view);
        assert_eq!(
            overlay.marks,
            [Mark::Marquee(Rect::new(440.0, 200.0, 60.0, 100.0))]
        );
    }

    #[test]
    fn an_armed_tool_is_named_in_the_chip() {
        let mut view = open(fixtures::TWO);
        let now = Instant::now();
        for (tool, name) in [
            (Tool::Shape(Shape::Diamond), "Diamond"),
            (Tool::Shape(Shape::Rect), "Rectangle"),
            (Tool::Text, "Text"),
            (Tool::Hand, "Hand"),
        ] {
            view.command(Command::Tool(tool), now);
            assert_eq!(marks(&view).marks, [Mark::Chip(name.to_owned())]);
        }
        view.command(Command::Tool(Tool::Select), now);
        assert_eq!(marks(&view).marks, []);
        assert_eq!(tool_name(Tool::Connector), Some("Connector"));
    }

    #[test]
    fn keyboard_connecting_outlines_its_target_and_shows_its_query() {
        let mut view = open(fixtures::TWO);
        click(&mut view, (60.0, 30.0));
        view.command(Command::Tool(Tool::Connector), Instant::now());
        let source = Mark::Outline(Rect::new(-3.0, -3.0, 126.0, 66.0));
        let chip = |text: &str| Mark::Chip(text.to_owned());
        let overlay = marks(&view);
        assert!(overlay.marks.contains(&source));
        assert!(overlay.marks.contains(&chip("Connect to")));
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Outline(_))), 1);
        for c in "b".chars() {
            input(&mut view, Input::Char(c));
        }
        let overlay = marks(&view);
        assert!(overlay.marks.contains(&chip("Connect to: b")));
        // The target is brought into view, 40 px from the top.
        let outlines: Vec<&Mark> = overlay
            .marks
            .iter()
            .filter(|mark| matches!(mark, Mark::Outline(_)))
            .collect();
        assert_eq!(
            outlines,
            [
                &Mark::Outline(Rect::new(-3.0, 37.0, 126.0, 66.0)),
                &Mark::Outline(Rect::new(297.0, 37.0, 126.0, 66.0))
            ]
        );
        // An armed tool is named no more.
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Chip(_))), 1);
        input(
            &mut view,
            Input::Key {
                key: Key::Escape,
                repeat: false,
            },
        );
        assert!(!marks(&view)
            .marks
            .iter()
            .any(|mark| matches!(mark, Mark::Chip(_))));
    }

    #[test]
    fn an_empty_board_that_may_be_edited_shows_the_hint() {
        for (text, hint) in [
            (wren_board::EMPTY, true),
            ("", true),
            ("schema = 1\nrequires = [\"frames\"]\n", false),
            (fixtures::TWO, false),
        ] {
            let view = open(text);
            assert_eq!(marks(&view).marks.contains(&Mark::Hint), hint, "{text:?}");
        }
    }

    #[test]
    fn the_hint_and_the_chip_are_measured_in_the_label_face() {
        let view = open(wren_board::EMPTY);
        let width = view.measurer().width(HINT, Face::Regular, HINT_TEXT);
        assert!(width > 200.0 && width < 400.0, "{width}");
    }
    #[test]
    fn an_open_label_shows_its_selection_and_its_caret() {
        use crate::LabelAction;
        let mut view = open(fixtures::TWO);
        click(&mut view, (60.0, 30.0));
        view.command(Command::EditLabel, Instant::now());
        // The scene's line for `a`, which is centred in its box.
        let line = view
            .scene()
            .items
            .iter()
            .find_map(|item| match &item.draw {
                Draw::Text { lines, .. } => lines.first().cloned(),
                _ => None,
            })
            .expect("a label");
        let overlay = marks(&view);
        // No handles while a label is open; the outline stays.
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Handle(_))), 0);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Outline(_))), 1);
        // The label opens with its text selected, the caret after it.
        let selected: Vec<&Mark> = overlay
            .marks
            .iter()
            .filter(|mark| matches!(mark, Mark::Selected(_)))
            .collect();
        let over = Rect::new(line.origin.x, line.origin.y, line.width, 20.0);
        assert_eq!(selected, [&Mark::Selected(over)]);
        let ink = view.label().expect("a label").ink;
        let caret = Rect::new(over.right() - 0.75, over.y, 1.5, 20.0);
        assert!(overlay.marks.contains(&Mark::Caret(caret, ink)));
        // They follow the viewport, in screen pixels.
        view.set_viewport(Viewport {
            x: 10.0,
            y: -20.0,
            zoom: 2.0,
        });
        let zoomed = marks(&view);
        let at = Rect::new(
            (over.x - 10.0) * 2.0,
            (over.y + 20.0) * 2.0,
            over.width * 2.0,
            40.0,
        );
        assert!(zoomed.marks.contains(&Mark::Selected(at)));
        assert!(zoomed.marks.contains(&Mark::Caret(
            Rect::new(at.right() - 0.75, at.y, 1.5, 40.0),
            ink
        )));
        // A caret alone once the selection is gone.
        let end = LabelAction::Move {
            motion: crate::Motion::End,
            select: false,
        };
        view.update(Message::Label(view.epoch(), end), Instant::now());
        let overlay = marks(&view);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Selected(_))), 0);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Caret(..))), 1);
        // Closed, neither.
        input(
            &mut view,
            Input::Key {
                key: Key::Escape,
                repeat: false,
            },
        );
        let overlay = marks(&view);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Caret(..))), 0);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Handle(_))), 8);
        // A label a command opened on a board without the keyboard shows
        // its selection and no caret: no key would reach it.
        let mut view = open(fixtures::TWO);
        let a = Source::Node(wren_board::Id::new("a").expect("an id"));
        view.machine
            .select(std::collections::BTreeSet::from([a.clone()]), Some(a));
        view.command(Command::EditLabel, Instant::now());
        assert!(view.label().is_some());
        let overlay = marks(&view);
        assert!(!overlay.focused);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Selected(_))), 1);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Caret(..))), 0);
    }

    /// Rest the pointer on a widget-local point until the node there shows
    /// its connect handles.
    fn rest(view: &mut BoardView, at: (f32, f32)) {
        input(view, Input::Focus(true));
        input(view, Input::Move { at });
        let rested = Instant::now() + crate::machine::HOVER_DELAY;
        view.update(Message::Input(view.epoch(), Input::Tick), rested);
    }

    fn press(view: &mut BoardView, at: (f32, f32)) {
        input(
            view,
            Input::Press {
                button: Button::Left,
                at,
            },
        );
    }

    #[test]
    fn a_node_the_pointer_rests_on_shows_four_connect_handles() {
        let mut view = open(fixtures::TWO);
        input(&mut view, Input::Focus(true));
        input(&mut view, Input::Move { at: (60.0, 30.0) });
        let connects = |view: &BoardView| -> Vec<(f32, f32)> {
            let marks = marks(view).marks.into_iter();
            marks
                .filter_map(|mark| match mark {
                    Mark::Connect(at) => Some(at),
                    _ => None,
                })
                .collect()
        };
        // Not before it has rested there.
        assert!(connects(&view).is_empty());
        rest(&mut view, (60.0, 30.0));
        // Sixteen pixels outside the middle of each side, clockwise from
        // the top; the node is not selected, so it has no outline.
        assert_eq!(
            connects(&view),
            [(60.0, -16.0), (136.0, 30.0), (60.0, 76.0), (-16.0, 30.0)]
        );
        let overlay = marks(&view);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Outline(_))), 0);
        // Selected, it shows them outside its resize handles.
        click(&mut view, (60.0, 30.0));
        let overlay = marks(&view);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Handle(_))), 8);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Connect(_))), 4);
        // Gone once the pointer has left.
        input(&mut view, Input::Leave);
        assert!(connects(&view).is_empty());
    }

    #[test]
    fn a_dragged_connector_marks_the_node_it_would_end_on() {
        let mut view = open(fixtures::TWO);
        rest(&mut view, (60.0, 30.0));
        press(&mut view, (60.0, 76.0));
        // Over `b`, which is 120 by 60 at 300,0: its outline and the four
        // handles on its sides, none taken.
        input(&mut view, Input::Move { at: (360.0, 30.0) });
        assert_eq!(view.machine().state(), State::Connecting);
        let overlay = marks(&view);
        let sides = |overlay: &Overlay| -> Vec<((f32, f32), bool)> {
            let marks = overlay.marks.iter();
            marks
                .filter_map(|mark| match mark {
                    Mark::Side(at, pinned) => Some((*at, *pinned)),
                    _ => None,
                })
                .collect()
        };
        assert!(overlay
            .marks
            .contains(&Mark::Outline(Rect::new(297.0, -3.0, 126.0, 66.0))));
        assert_eq!(
            sides(&overlay),
            [
                ((360.0, 0.0), false),
                ((420.0, 30.0), false),
                ((360.0, 60.0), false),
                ((300.0, 30.0), false)
            ]
        );
        // The source's connect handles went when the drag began.
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Connect(_))), 0);
        // Near a handle, that one is taken.
        input(&mut view, Input::Move { at: (414.0, 33.0) });
        let taken: Vec<bool> = sides(&marks(&view))
            .into_iter()
            .map(|side| side.1)
            .collect();
        assert_eq!(taken, [false, true, false, false]);
        // Over empty canvas there is no node to mark.
        input(&mut view, Input::Move { at: (500.0, 300.0) });
        let overlay = marks(&view);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Outline(_))), 0);
        assert!(sides(&overlay).is_empty());
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Band(..))), 0);
    }

    #[test]
    fn an_end_that_would_go_back_trails_a_band_to_the_pointer() {
        let mut view = open(fixtures::TWO);
        click(&mut view, (210.0, 30.0));
        // The `to` end of `e`, on `b`'s left side.
        press(&mut view, (300.0, 30.0));
        input(&mut view, Input::Move { at: (500.0, 300.0) });
        let overlay = marks(&view);
        assert!(overlay
            .marks
            .contains(&Mark::Band((300.0, 30.0), (500.0, 300.0))));
        // The connector's two ends where they are, and the one in hand.
        let ends: Vec<&Mark> = overlay
            .marks
            .iter()
            .filter(|mark| matches!(mark, Mark::End(_)))
            .collect();
        assert_eq!(
            ends,
            [
                &Mark::End((120.0, 30.0)),
                &Mark::End((300.0, 30.0)),
                &Mark::End((500.0, 300.0))
            ]
        );
        // Over a node it may take, the preview shows where it would go
        // and the band is gone; a selected node is not outlined twice.
        input(&mut view, Input::Move { at: (360.0, 58.0) });
        let overlay = marks(&view);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Band(..))), 0);
        assert_eq!(count(&overlay, |mark| matches!(mark, Mark::Outline(_))), 1);
        assert_eq!(
            count(&overlay, |mark| matches!(mark, Mark::Side(_, true))),
            1
        );
        assert!(overlay.marks.contains(&Mark::End((360.0, 60.0))));
    }
}
