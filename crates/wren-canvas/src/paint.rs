//! Scene items into an iced geometry frame.

use iced::advanced::text::{LineHeight, Shaping};
use iced::widget::canvas::{self, Frame, LineDash, Path, Stroke};
use iced::{alignment, Color, Font, Pixels};
use wren_board::scene::{Draw, Item, Point, Segment};
use wren_board::Rgba;

use crate::Viewport;

/// How board coordinates reach the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Space {
    /// Board units: the renderer scales and moves the finished geometry.
    Board,
    /// Widget-local pixels at this viewport, sizes and positions scaled
    /// before they reach the frame.
    Screen(Viewport),
}

impl Space {
    fn zoom(self) -> f32 {
        match self {
            Self::Board => 1.0,
            Self::Screen(viewport) => viewport.zoom,
        }
    }

    fn point(self, point: Point) -> iced::Point {
        match self {
            Self::Board => iced::Point::new(point.x, point.y),
            Self::Screen(viewport) => {
                let (x, y) = viewport.to_screen((point.x, point.y));
                iced::Point::new(x, y)
            }
        }
    }
}

pub(crate) fn color(color: Rgba) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, f32::from(color.a) / 255.0)
}

/// Paint one item. Each wrapped line is its own unwrapped text: the two
/// renderers disagree about the wrap width of text in a scaled frame, so
/// the frame never wraps.
pub(crate) fn item(frame: &mut Frame, item: &Item, space: Space, font: Font, line_height: f32) {
    let zoom = space.zoom();
    match &item.draw {
        Draw::Path {
            segments,
            fill,
            stroke,
        } => {
            let path = Path::new(|builder| {
                for segment in segments {
                    match *segment {
                        Segment::MoveTo(to) => builder.move_to(space.point(to)),
                        Segment::LineTo(to) => builder.line_to(space.point(to)),
                        Segment::QuadTo { control, to } => {
                            builder.quadratic_curve_to(space.point(control), space.point(to));
                        }
                        Segment::CubicTo { first, second, to } => builder.bezier_curve_to(
                            space.point(first),
                            space.point(second),
                            space.point(to),
                        ),
                        Segment::Close => builder.close(),
                    }
                }
            });
            if let Some(fill) = fill {
                frame.fill(&path, color(*fill));
            }
            if let Some(stroke) = stroke {
                let dash = stroke.dash.map(|[on, off]| [on * zoom, off * zoom]);
                frame.stroke(
                    &path,
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
            size,
            color: ink,
            ..
        } => {
            for line in lines {
                frame.fill_text(canvas::Text {
                    content: line.text.clone(),
                    position: space.point(line.origin),
                    max_width: f32::INFINITY,
                    color: color(*ink),
                    size: Pixels(size * zoom),
                    line_height: LineHeight::Absolute(Pixels(line_height * zoom)),
                    font,
                    align_x: iced::advanced::text::Alignment::Left,
                    align_y: alignment::Vertical::Top,
                    shaping: Shaping::Advanced,
                });
            }
        }
    }
}
