//! What a frame holds of a scene, and scene items into an iced geometry
//! frame.
//!
//! Geometry is built in widget-local pixels at the viewport it is shown
//! from: each point goes through the viewport and each stroke width, dash
//! and font size is multiplied by the zoom before it reaches the frame. A
//! renderer transformation over geometry in board units would be the
//! same picture, but it cannot cull, and both renderers then pay for
//! every label of the board on every frame.

use iced::advanced::text::{LineHeight, Shaping};
use iced::widget::canvas::{self, Frame, LineDash, Path, Stroke};
use iced::{alignment, Color, Pixels};
use wren_board::scene::{self, Draw, Item, Point, Rect, Scene, Segment};
use wren_board::Rgba;

use crate::clip::{self, Piece, Window};
use crate::layers::Plan;
use crate::{Labels, Measurer, Viewport};

/// How far past a path's bounds its antialiased edge may reach, in screen
/// pixels.
const EDGE: f32 = 1.0;

/// How far past its line box a glyph's ink may reach, as a share of the
/// font size: side bearings and marks, never a whole glyph.
const OVERHANG: f32 = 0.25;

/// How far past the widget a path is drawn before it is cut, in screen
/// pixels: past any stroke's reach, and near enough that what the
/// software renderer is handed stays a few thousand pixels across.
const CUT_MARGIN: f64 = 2048.0;

/// What one frame shows of a board: the part of it in view, and which
/// labels. Two frames with equal culls show the same items of a scene at
/// the same pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cull {
    pub(crate) viewport: Viewport,
    /// The board rectangle the widget shows.
    visible: Rect,
    /// Labels are drawn from this font size up, in board units; no font
    /// size reaches infinity.
    min_size: f32,
}

impl Cull {
    /// The frame of a widget of `size` pixels looking from `viewport`,
    /// under the label rule, with the view `moving` or at rest.
    pub(crate) fn new(viewport: Viewport, size: (f32, f32), labels: Labels, moving: bool) -> Self {
        Self {
            viewport,
            visible: Rect::new(
                viewport.x,
                viewport.y,
                size.0 / viewport.zoom,
                size.1 / viewport.zoom,
            ),
            min_size: if moving && !labels.moving {
                f32::INFINITY
            } else {
                labels.min_px / viewport.zoom
            },
        }
    }

    /// Whether the frame holds `item`: a label only at a size the rule
    /// draws, and anything only if what it paints may reach into view.
    pub(crate) fn shows(&self, item: &Item) -> bool {
        let reach = EDGE / self.viewport.zoom
            + match &item.draw {
                Draw::Text { size, .. } if *size >= self.min_size => size * OVERHANG,
                Draw::Text { .. } => return false,
                Draw::Path { .. } => 0.0,
            };
        let (bounds, visible) = (item.bounds, self.visible);
        bounds.x - reach < visible.right()
            && visible.x < bounds.right() + reach
            && bounds.y - reach < visible.bottom()
            && visible.y < bounds.bottom() + reach
    }

    /// The rectangle paths are cut to, in widget-local pixels.
    fn window(&self) -> Window {
        let zoom = f64::from(self.viewport.zoom);
        Window {
            left: -CUT_MARGIN,
            top: -CUT_MARGIN,
            right: f64::from(self.visible.width) * zoom + CUT_MARGIN,
            bottom: f64::from(self.visible.height) * zoom + CUT_MARGIN,
        }
    }
}

/// The items of `scene` a frame holds, by layer of `plan` and back to
/// front within each.
pub(crate) fn shown(scene: &Scene, plan: &Plan, cull: &Cull) -> Vec<Vec<usize>> {
    let mut layers = vec![Vec::new(); plan.count];
    for (index, (item, layer)) in scene.items.iter().zip(&plan.of).enumerate() {
        if let Some(layer) = layers.get_mut(usize::from(*layer)) {
            if cull.shows(item) {
                layer.push(index);
            }
        }
    }
    layers
}

pub(crate) fn color(color: Rgba) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, f32::from(color.a) / 255.0)
}

/// Paint one item as `cull` shows it, text in the font it was measured
/// in. Each wrapped line is its own unwrapped text, placed where the scene
/// put it: the two renderers disagree about the wrap width of text in a
/// scaled frame, so the frame never wraps. A path reaching far past the
/// widget is cut first ([`clip`]).
pub(crate) fn item(frame: &mut Frame, item: &Item, cull: &Cull, fonts: &Measurer) {
    let viewport = cull.viewport;
    let zoom = viewport.zoom;
    let point = |point: Point| {
        let (x, y) = viewport.to_screen((point.x, point.y));
        iced::Point::new(x, y)
    };
    match &item.draw {
        Draw::Path {
            segments,
            fill,
            stroke,
        } => {
            let pieces = pieces(segments, viewport);
            let window = cull.window();
            let whole = clip::inside(&pieces, &window).then(|| path(&pieces));
            if let Some(fill) = fill {
                let cut;
                let path = match &whole {
                    Some(path) => path,
                    None => {
                        cut = path(&clip::area(&pieces, &window));
                        &cut
                    }
                };
                frame.fill(path, color(*fill));
            }
            if let Some(stroke) = stroke {
                let dash = stroke.dash.map(|[on, off]| [on * zoom, off * zoom]);
                let cut;
                let path = match &whole {
                    Some(path) => path,
                    None => {
                        let period = dash.map(|[on, off]| f64::from(on) + f64::from(off));
                        cut = path(&clip::outline(&pieces, &window, period));
                        &cut
                    }
                };
                frame.stroke(
                    path,
                    Stroke {
                        style: canvas::Style::Solid(color(stroke.color)),
                        width: stroke.width * zoom,
                        line_dash: LineDash {
                            segments: dash.as_ref().map_or(&[], |dash| dash.as_slice()),
                            offset: 0,
                        },
                        ..Stroke::default()
                    },
                );
            }
        }
        Draw::Text {
            lines,
            face,
            size,
            color: ink,
        } => {
            let height = scene::line_height(*size);
            let font = fonts.font(*face);
            for line in lines.iter().filter(|line| !line.text.is_empty()) {
                frame.fill_text(canvas::Text {
                    content: line.text.clone(),
                    position: point(line.origin),
                    max_width: room(line.width, *size, zoom),
                    color: color(*ink),
                    size: Pixels(size * zoom),
                    line_height: LineHeight::Absolute(Pixels(height * zoom)),
                    font,
                    // Not `Left`: iced lays a right-to-left line out
                    // again at its own width only under the default, and
                    // otherwise leaves it against the right of its room.
                    align_x: iced::advanced::text::Alignment::Default,
                    align_y: alignment::Vertical::Top,
                    shaping: Shaping::Advanced,
                });
            }
        }
    }
}

/// Stroke the outline of a path item again with `stroke`, as `cull`
/// frames it: a selected connector's highlight. Text draws nothing.
pub(crate) fn trace(frame: &mut Frame, item: &Item, cull: &Cull, stroke: Stroke<'_>) {
    let Draw::Path { segments, .. } = &item.draw else {
        return;
    };
    let pieces = pieces(segments, cull.viewport);
    let window = cull.window();
    let path = if clip::inside(&pieces, &window) {
        path(&pieces)
    } else {
        path(&clip::outline(&pieces, &window, None))
    };
    frame.stroke(&path, stroke);
}

/// `segments` in widget-local pixels at `viewport`, in double precision:
/// a board coordinate millions of units out is cut against the window
/// without losing where its line crosses into view.
fn pieces(segments: &[Segment], viewport: Viewport) -> Vec<Piece> {
    let (x, y, zoom) = (
        f64::from(viewport.x),
        f64::from(viewport.y),
        f64::from(viewport.zoom),
    );
    let at = |point: Point| {
        (
            (f64::from(point.x) - x) * zoom,
            (f64::from(point.y) - y) * zoom,
        )
    };
    segments
        .iter()
        .map(|segment| match *segment {
            Segment::MoveTo(to) => Piece::Move(at(to)),
            Segment::LineTo(to) => Piece::Line(at(to)),
            Segment::QuadTo { control, to } => Piece::Quad(at(control), at(to)),
            Segment::CubicTo { first, second, to } => Piece::Cubic(at(first), at(second), at(to)),
            Segment::Close => Piece::Close,
        })
        .collect()
}

/// The iced path of `pieces`.
fn path(pieces: &[Piece]) -> Path {
    // Each point is in the window or a path left whole, so fits an f32.
    let point = |(x, y): clip::Pixel| iced::Point::new(x as f32, y as f32);
    Path::new(|builder| {
        for piece in pieces {
            match *piece {
                Piece::Move(to) => builder.move_to(point(to)),
                Piece::Line(to) => builder.line_to(point(to)),
                Piece::Quad(control, to) => builder.quadratic_curve_to(point(control), point(to)),
                Piece::Cubic(first, second, to) => {
                    builder.bezier_curve_to(point(first), point(second), point(to));
                }
                Piece::Close => builder.close(),
            }
        }
    })
}

/// The most room a line is given, in pixels: far wider than any label, and
/// far inside the pixel positions a glyph cache can hold.
const MAX_ROOM: f32 = 16_777_216.0;

/// The width a line of `width` units in text of `size` is laid out in at
/// `zoom`, in pixels: its own width and an em to spare, so it does not
/// wrap. It must be bounded: cosmic-text starts a right-to-left line at
/// the right edge of its room before iced fits the room to the line, and
/// an unbounded room puts every glyph past any pixel position.
fn room(width: f32, size: f32, zoom: f32) -> f32 {
    let room = (width + size) * zoom;
    if room.is_finite() {
        room.clamp(0.0, MAX_ROOM)
    } else {
        MAX_ROOM
    }
}

#[cfg(test)]
mod tests {
    use wren_board::scene::{Face, Line, Source, Stroke as Outline, LABEL_SIZE, SMALL_SIZE};
    use wren_board::Id;

    use super::*;
    use crate::{fixtures, layers, BoardView};

    const WINDOW: (f32, f32) = (800.0, 600.0);

    fn source() -> Source {
        Source::Node(Id::new("n").expect("an id"))
    }

    fn shape(bounds: Rect) -> Item {
        Item {
            source: source(),
            bounds,
            draw: Draw::Path {
                segments: Vec::new(),
                fill: Some(Rgba::rgb(1, 2, 3)),
                stroke: Some(Outline {
                    color: Rgba::rgb(4, 5, 6),
                    width: 1.5,
                    dash: None,
                }),
            },
        }
    }

    fn label(bounds: Rect, size: f32) -> Item {
        Item {
            source: source(),
            bounds,
            draw: Draw::Text {
                lines: vec![Line {
                    text: "label".into(),
                    origin: Point::new(bounds.x, bounds.y),
                    width: bounds.width,
                }],
                face: Face::Regular,
                size,
                color: Rgba::rgb(7, 8, 9),
            },
        }
    }

    fn at(x: f32, y: f32, zoom: f32) -> Viewport {
        Viewport { x, y, zoom }
    }

    #[test]
    fn a_line_is_laid_out_in_bounded_room_to_spare() {
        assert_eq!(room(100.0, 14.0, 1.0), 114.0);
        assert_eq!(room(100.0, 14.0, 0.5), 57.0);
        assert_eq!(room(0.0, 12.0, 4.0), 48.0);
        for (width, zoom) in [(f32::INFINITY, 1.0), (f32::NAN, 1.0), (1e30, 4.0)] {
            assert_eq!(room(width, 14.0, zoom), MAX_ROOM, "{width} at {zoom}");
        }
        assert_eq!(room(-500.0, 14.0, 1.0), 0.0);
    }

    #[test]
    fn a_frame_holds_what_reaches_into_view() {
        // The widget shows the board from (100, 50) to (900, 650).
        let cull = Cull::new(at(100.0, 50.0, 1.0), WINDOW, Labels::ALWAYS, false);
        for (bounds, shown) in [
            (Rect::new(400.0, 300.0, 120.0, 60.0), true),
            // Across each edge.
            (Rect::new(40.0, 300.0, 120.0, 60.0), true),
            (Rect::new(840.0, 300.0, 120.0, 60.0), true),
            (Rect::new(400.0, 20.0, 120.0, 60.0), true),
            (Rect::new(400.0, 620.0, 120.0, 60.0), true),
            // Covering the whole view.
            (Rect::new(-1000.0, -1000.0, 5000.0, 5000.0), true),
            // Clear of each edge.
            (Rect::new(-200.0, 300.0, 120.0, 60.0), false),
            (Rect::new(1000.0, 300.0, 120.0, 60.0), false),
            (Rect::new(400.0, -200.0, 120.0, 60.0), false),
            (Rect::new(400.0, 800.0, 120.0, 60.0), false),
            // Past a corner: in both ranges of neither axis alone.
            (Rect::new(950.0, 700.0, 120.0, 60.0), false),
            // A line with no thickness in view is still painted.
            (Rect::new(400.0, 300.0, 0.0, 200.0), true),
            // Bounds no number describes are never in view.
            (Rect::new(f32::NAN, 300.0, 120.0, 60.0), false),
        ] {
            assert_eq!(cull.shows(&shape(bounds)), shown, "{bounds:?}");
            assert_eq!(cull.shows(&label(bounds, LABEL_SIZE)), shown, "{bounds:?}");
        }
    }

    #[test]
    fn an_edge_one_pixel_out_of_view_is_kept_for_its_antialiasing() {
        for zoom in [0.25, 1.0, 4.0] {
            let cull = Cull::new(at(0.0, 0.0, zoom), WINDOW, Labels::ALWAYS, false);
            let right = WINDOW.0 / zoom;
            let near = Rect::new(right + 0.5 / zoom, 10.0, 50.0, 50.0);
            let far = Rect::new(right + 1.5 / zoom, 10.0, 50.0, 50.0);
            assert!(cull.shows(&shape(near)), "zoom {zoom}");
            assert!(!cull.shows(&shape(far)), "zoom {zoom}");
            // A label's ink may hang a quarter of its size past its box.
            let hanging = Rect::new(right + 0.5 / zoom + 3.0, 10.0, 50.0, 20.0);
            assert!(cull.shows(&label(hanging, LABEL_SIZE)), "zoom {zoom}");
            let clear = Rect::new(right + 1.5 / zoom + 4.0, 10.0, 50.0, 20.0);
            assert!(!cull.shows(&label(clear, LABEL_SIZE)), "zoom {zoom}");
        }
    }

    #[test]
    fn the_view_follows_the_viewport_and_the_zoom() {
        let node = shape(Rect::new(1000.0, 1000.0, 120.0, 60.0));
        let shows = |viewport| Cull::new(viewport, WINDOW, Labels::ALWAYS, false).shows(&node);
        assert!(!shows(at(0.0, 0.0, 1.0)));
        assert!(shows(at(600.0, 700.0, 1.0)));
        // Zoomed out, the same corner shows four times as far.
        assert!(shows(at(0.0, 0.0, 0.25)));
        // Zoomed in, a quarter as far.
        assert!(!shows(at(600.0, 700.0, 4.0)));
        assert!(shows(at(900.0, 950.0, 4.0)));
    }

    #[test]
    fn the_software_rule_drops_small_labels_and_all_of_them_while_moving() {
        let bounds = Rect::new(10.0, 10.0, 100.0, 20.0);
        let (node, small) = (label(bounds, LABEL_SIZE), label(bounds, SMALL_SIZE));
        let frame = |zoom, labels, moving| Cull::new(at(0.0, 0.0, zoom), WINDOW, labels, moving);

        // At rest: 14 px text is 4 px on screen at 28.6 %, 12 px text at a third.
        for (zoom, shows_node, shows_small) in [
            (1.0, true, true),
            (0.34, true, true),
            (0.3, true, false),
            (0.28, false, false),
            (0.1, false, false),
        ] {
            let cull = frame(zoom, Labels::SOFTWARE, false);
            assert_eq!(cull.shows(&node), shows_node, "zoom {zoom}");
            assert_eq!(cull.shows(&small), shows_small, "zoom {zoom}");
            assert!(cull.shows(&shape(bounds)));
        }

        // Moving: shapes only, at any zoom.
        for zoom in [0.1, 1.0, 4.0] {
            let cull = frame(zoom, Labels::SOFTWARE, true);
            assert!(!cull.shows(&node) && !cull.shows(&small), "zoom {zoom}");
            assert!(cull.shows(&shape(bounds)));
        }

        // A renderer that draws labels always has one frame for both.
        for zoom in [0.1, 1.0, 4.0] {
            let rest = frame(zoom, Labels::ALWAYS, false);
            assert!(rest.shows(&node) && rest.shows(&small), "zoom {zoom}");
            assert_eq!(rest, frame(zoom, Labels::ALWAYS, true));
        }
        assert_ne!(
            frame(1.0, Labels::SOFTWARE, false),
            frame(1.0, Labels::SOFTWARE, true)
        );
    }

    #[test]
    fn equal_frames_are_the_same_view_of_the_same_size() {
        let cull = Cull::new(at(10.0, 20.0, 1.5), WINDOW, Labels::SOFTWARE, false);
        assert_eq!(
            cull,
            Cull::new(at(10.0, 20.0, 1.5), WINDOW, Labels::SOFTWARE, false)
        );
        for other in [
            Cull::new(at(11.0, 20.0, 1.5), WINDOW, Labels::SOFTWARE, false),
            Cull::new(at(10.0, 20.0, 1.6), WINDOW, Labels::SOFTWARE, false),
            Cull::new(at(10.0, 20.0, 1.5), (801.0, 600.0), Labels::SOFTWARE, false),
            Cull::new(at(10.0, 20.0, 1.5), WINDOW, Labels::ALWAYS, false),
        ] {
            assert_ne!(cull, other);
        }
    }

    #[test]
    fn the_shown_items_keep_their_layer_and_their_order() {
        let items = vec![
            shape(Rect::new(0.0, 0.0, 240.0, 80.0)),
            label(Rect::new(20.0, 30.0, 200.0, 20.0), LABEL_SIZE),
            // Out of view, in the first layer.
            shape(Rect::new(5000.0, 0.0, 240.0, 80.0)),
            // Covers the first label: a layer up, with its own label.
            shape(Rect::new(80.0, 40.0, 240.0, 80.0)),
            label(Rect::new(100.0, 70.0, 200.0, 20.0), LABEL_SIZE),
        ];
        let plan = layers::split(&items, layers::CAP);
        assert_eq!(plan.of, [0, 0, 0, 1, 1]);
        let scene = Scene {
            bounds: Rect::new(0.0, 0.0, 5240.0, 120.0),
            items,
        };
        let rest = Cull::new(at(0.0, 0.0, 1.0), WINDOW, Labels::SOFTWARE, false);
        assert_eq!(shown(&scene, &plan, &rest), [vec![0, 1], vec![3, 4]]);
        let moving = Cull::new(at(0.0, 0.0, 1.0), WINDOW, Labels::SOFTWARE, true);
        assert_eq!(shown(&scene, &plan, &moving), [vec![0], vec![3]]);
        // Looking at the far shape: the second layer is there and empty.
        let far = Cull::new(at(4900.0, 0.0, 1.0), WINDOW, Labels::SOFTWARE, false);
        assert_eq!(shown(&scene, &plan, &far), [vec![2], vec![]]);
        assert_eq!(
            shown(&scene, &Plan::default(), &rest),
            Vec::<Vec<usize>>::new()
        );
    }

    #[test]
    fn a_fitted_board_shows_every_item_and_a_corner_of_it_a_few() {
        fixtures::load_fonts();
        let view = BoardView::open(fixtures::CHECKOUT_FLOW, fixtures::palette(), fixtures::FONT)
            .expect("the sample board");
        let (scene, plan) = (view.scene(), view.layers());
        let count = |cull: &Cull| shown(scene, plan, cull).iter().map(Vec::len).sum::<usize>();
        let bounds = scene.bounds;

        let whole = at(bounds.x - 40.0, bounds.y - 40.0, 0.5);
        let size = ((bounds.width + 80.0) * 0.5, (bounds.height + 80.0) * 0.5);
        let all = Cull::new(whole, size, Labels::ALWAYS, false);
        assert_eq!(count(&all), scene.items.len());

        // The top-left 200 by 100 units hold the first node and little else.
        let corner = Cull::new(
            at(bounds.x, bounds.y, 1.0),
            (200.0, 100.0),
            Labels::ALWAYS,
            false,
        );
        let few = count(&corner);
        assert!(few >= 2 && few < scene.items.len() / 4, "{few}");
        for index in shown(scene, plan, &corner).into_iter().flatten() {
            let item = scene.items[index].bounds;
            assert!(
                item.x < bounds.x + 205.0 && item.y < bounds.y + 105.0,
                "{item:?}"
            );
        }

        // Nothing of the board is in view far from it.
        let away = Cull::new(
            at(bounds.right() + 500.0, bounds.bottom() + 500.0, 1.0),
            WINDOW,
            Labels::ALWAYS,
            false,
        );
        assert_eq!(count(&away), 0);
    }
}
