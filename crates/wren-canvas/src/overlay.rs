//! What the widget draws over a board's scene (the plan's §5.2, §5.3 and
//! §5.8): the selection, the cursor ring, the handles, the marquee, the
//! mode chip and the empty board's hint. None of it is the board's, so
//! none of it is in the scene, an export or a draft.
//!
//! [`marks`] decides what is drawn, in widget-local pixels, as a pure
//! function of the view; [`paint`] draws it.

use std::time::Instant;

use iced::advanced::text::{LineHeight, Shaping};
use iced::widget::canvas::{self, Frame, LineDash, Path, Stroke};
use iced::{alignment, Pixels, Size};
use wren_board::scene::{Draw, Face, Measure as _, Rect, Source};

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
    /// Around the cursor element while the selection is something else.
    Ring(Rect),
    /// A resize handle's centre.
    Handle((f32, f32)),
    /// A connect handle's centre.
    Connect((f32, f32)),
    /// A connector end's handle.
    End((f32, f32)),
    Marquee(Rect),
    /// The armed tool's name.
    Chip(&'static str),
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
    for id in selection {
        if let Some(bounds) = frames.get(id) {
            marks.push(Mark::Outline(grown(screen(bounds), OUTLINE_GAP)));
            continue;
        }
        let lines = picture.scene.items.iter().enumerate().filter(|(_, item)| {
            matches!(&item.source, Source::Edge(edge) if edge == id)
                && matches!(
                    &item.draw,
                    Draw::Path {
                        stroke: Some(_),
                        ..
                    }
                )
        });
        marks.extend(lines.map(|(index, _)| Mark::Line(index)));
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

    if let Some(area) = machine.marquee() {
        marks.push(Mark::Marquee(screen(area)));
    }
    if let Some(name) = tool_name(machine.tool()) {
        marks.push(Mark::Chip(name));
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
            Mark::End(at) => frame.fill(&dot(*at), colour),
            Mark::Marquee(bounds) => {
                let path = rect(*bounds);
                frame.fill(&path, colour.scale_alpha(0.12));
                frame.stroke(&path, solid(1.0));
            }
            Mark::Chip(name) => {
                let width = view.measurer().width(name, Face::Regular, CHIP_TEXT);
                let height = wren_board::scene::line_height(CHIP_TEXT);
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
                let height = wren_board::scene::line_height(HINT_TEXT);
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
        line_height: LineHeight::Absolute(Pixels(wren_board::scene::line_height(size))),
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
            assert_eq!(marks(&view).marks, [Mark::Chip(name)]);
        }
        view.command(Command::Tool(Tool::Select), now);
        assert_eq!(marks(&view).marks, []);
        assert_eq!(tool_name(Tool::Connector), Some("Connector"));
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
}
