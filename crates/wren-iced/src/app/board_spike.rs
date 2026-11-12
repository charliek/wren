//! `--board-spike N`: a generated board in memory, shown by the board
//! viewer with no workspace behind it. It is where the viewer is looked at
//! and timed until boards open from a workspace, and it is never saved.

use std::fmt::Write as _;

use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill, Task};
use wren_board::{Palette, Rgba};
use wren_canvas::BoardView;
use wren_core::theme::{Appearance, Rgb, Theme};

use super::{definitions, App, Message};
use crate::{fonts, theme};

/// The colours a theme gives the spike's board. The hues are stand-ins:
/// the app chain derives the real palette from each theme.
pub(super) fn palette(definition: Theme) -> Palette {
    let rgb = |Rgb(r, g, b): Rgb| Rgba::rgb(r, g, b);
    let colors = definition.colors;
    Palette {
        canvas: rgb(colors.editor),
        ink: rgb(colors.text),
        muted: rgb(colors.muted),
        raised: rgb(colors.raised),
        accent: rgb(colors.accent),
        issue: rgb(colors.danger),
        hues: [
            rgb(colors.danger),
            rgb(colors.warning),
            rgb(colors.heading2),
            rgb(colors.success),
            rgb(colors.link),
            rgb(colors.heading),
            rgb(colors.accent),
        ],
        dark: definition.appearance == Appearance::Dark,
    }
}

/// A board of `count` rounded rectangles of 160 by 64 in a grid, each
/// with a two-line label and an elbow connector to its right and lower
/// neighbour. Every tenth label has CJK text and an emoji, and one shape
/// in ten is solid and sits over its left neighbour's label.
pub(super) fn grid(count: usize) -> String {
    const COLORS: [&str; 7] = ["red", "orange", "yellow", "green", "cyan", "purple", "blue"];
    let columns = ((count as f32).sqrt() * 1.25).ceil().max(1.0) as usize;
    let covers = |index: usize| index % 10 == 9 && !index.is_multiple_of(columns);
    let mut board = String::from("schema = 1\n");
    for index in 0..count {
        let (mut x, mut y) = ((index % columns) * 220, (index / columns) * 120);
        if covers(index) {
            x -= 120;
            y += 32;
        }
        let label = if index % 10 == 4 {
            format!("検証 {index} 🐦\\n二行目のラベル")
        } else {
            format!("Step {index}: validate\\nrequest and reply")
        };
        let color = COLORS[index % COLORS.len()];
        let fill = if covers(index) { "solid" } else { "tint" };
        // Writing to a String cannot fail.
        let _ = write!(
            board,
            "\n[node.n{index}]\ntext = \"{label}\"\nkind = \"shape\"\nat = [{x}, {y}]\n\
             shape = \"rounded\"\nsize = [160, 64]\nstyle.color = \"{color}\"\n\
             style.fill = \"{fill}\"\nz = \"{index:06}a\"\n"
        );
        let mut connect = |id: char, to: usize, sides: [&str; 2], z: char| {
            let _ = write!(
                board,
                "\n[edge.{id}{index}]\nkind = \"edge\"\n\
                 from = {{ node = \"n{index}\", side = \"{}\" }}\n\
                 to = {{ node = \"n{to}\", side = \"{}\" }}\nz = \"{index:06}{z}\"\n",
                sides[0], sides[1]
            );
        };
        if index + 1 < count && !(index + 1).is_multiple_of(columns) {
            connect('r', index + 1, ["right", "left"], 'b');
        }
        if index + columns < count {
            connect('d', index + columns, ["bottom", "top"], 'c');
        }
    }
    board
}

impl App {
    pub(super) fn open_board_spike(&mut self, count: usize) -> Result<(), String> {
        self.show_board_spike(&grid(count))
    }

    /// Show the board `text` holds in place of the one on screen.
    fn show_board_spike(&mut self, text: &str) -> Result<(), String> {
        // Labels are measured when the view opens, before a window exists.
        fonts::load();
        let view = BoardView::open(text, palette(self.definition), fonts::UI)
            .map_err(|error| error.to_string())?;
        self.board_spike = Some(view);
        Ok(())
    }

    /// Repaint the spike's board in the current theme's colours.
    pub(super) fn retheme_board_spike(&mut self) {
        let palette = palette(self.definition);
        if let Some(view) = &mut self.board_spike {
            view.set_palette(palette);
        }
    }

    pub(super) fn board_spike_input(&mut self, message: wren_canvas::Message) -> Task<Message> {
        match message {}
    }

    pub(super) fn board_spike_view<'a>(&'a self, view: &'a BoardView) -> Element<'a, Message> {
        let colors = self.definition.colors;
        let viewport = view.viewport();
        let header = container(
            row![
                text("Wren").size(18).color(theme::color(colors.accent)),
                text(format!(
                    "Board spike · in memory · {} shapes · {:.0}%",
                    view.board().nodes.len(),
                    viewport.zoom * 100.0
                ))
                .size(13),
                iced::widget::space::horizontal(),
                button("System").on_press(Message::ChooseTheme(None)),
                button("Dark").on_press(Message::ChooseTheme(Some(definitions::DARK))),
                button("Light").on_press(Message::ChooseTheme(Some(definitions::LIGHT))),
            ]
            .spacing(10)
            .align_y(iced::Center),
        )
        .padding([10, 16])
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(theme::color(colors.titlebar).into()),
            ..Default::default()
        });
        #[cfg(feature = "smoke-test")]
        let style = wren_canvas::Style {
            labels: self.board_perf.labels,
        };
        #[cfg(not(feature = "smoke-test"))]
        let style = wren_canvas::Style::default();
        let body = column![
            header,
            view.view("note-editor", style).map(Message::BoardSpike)
        ];
        self.palette_view(body.width(Fill).height(Fill).into())
    }
}

#[cfg(feature = "smoke-test")]
pub(super) use harness::Perf;

#[cfg(feature = "smoke-test")]
mod harness {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::time::Instant;

    use iced::Task;
    use serde_json::{json, Value};
    use wren_board::scene::{self, Draw, Rect, Source};
    use wren_board::{Access, Id, Rgba};
    use wren_canvas::{BoardView, Labels, Probe, Viewport};

    use super::super::{App, Message};
    use crate::test_ipc::Request;

    /// The timings `perf.board` reports that the view does not keep, the
    /// run in flight, and the label rule the harness asked for.
    #[derive(Default)]
    pub struct Perf {
        frame_micros: VecDeque<u64>,
        open_micros: Option<u64>,
        run: Option<Run>,
        /// `None` leaves the rule to the widget, which picks by renderer.
        pub labels: Option<Labels>,
    }

    /// A pan or zoom of one step per redraw.
    struct Run {
        request: Arc<Request>,
        step: Step,
        steps: u32,
        /// How many redraws the run has seen.
        pulses: u32,
        last: Option<Instant>,
    }

    #[derive(Clone, Copy)]
    enum Step {
        Pan { dx: f32, dy: f32 },
        Zoom { factor: f32 },
    }

    fn push(ring: &mut VecDeque<u64>, started: Instant, ended: Instant) {
        if ring.len() == Probe::SAMPLES {
            ring.pop_front();
        }
        ring.push_back(
            ended
                .saturating_duration_since(started)
                .as_micros()
                .try_into()
                .unwrap_or(u64::MAX),
        );
    }

    fn rect(rect: Rect) -> Value {
        json!({"x":rect.x,"y":rect.y,"width":rect.width,"height":rect.height})
    }

    fn rgb(color: Rgba) -> Value {
        json!([color.r, color.g, color.b])
    }

    fn number(params: &Value, name: &str) -> Result<f32, String> {
        params
            .get(name)
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("{name} must be a number"))
    }

    /// A node's box and the lines of its label, in board units.
    fn node(view: &BoardView, name: &str) -> Option<Value> {
        let id = Id::new(name)?;
        let node = view.board().nodes.get(&id)?;
        let at = wren_board::placements(view.board()).get(&id)?.at;
        let source = Source::Node(id);
        let lines: Vec<Value> = view
            .scene()
            .items
            .iter()
            .filter(|item| item.source == source)
            .filter_map(|item| match &item.draw {
                Draw::Text { lines, .. } => Some(lines),
                Draw::Path { .. } => None,
            })
            .flatten()
            .map(|line| {
                json!({
                    "text": line.text, "x": line.origin.x, "y": line.origin.y, "width": line.width,
                })
            })
            .collect();
        Some(json!({
            "id": name,
            "bounds": {"x": at[0], "y": at[1], "width": node.size[0], "height": node.size[1]},
            "lines": lines,
        }))
    }

    impl App {
        fn spike(&mut self) -> Result<&mut BoardView, String> {
            self.board_spike
                .as_mut()
                .ok_or_else(|| "the surface is not a board".to_string())
        }

        /// `perf.board`: the timing rings, optionally after a pan or zoom
        /// run of one step per redraw.
        pub(in super::super) fn perf_board(
            &mut self,
            request: &Arc<Request>,
        ) -> Result<Task<Message>, String> {
            let params = &request.params;
            self.spike()?;
            let step = if let Some(pan) = params.get("pan") {
                Some((
                    Step::Pan {
                        dx: number(pan, "dx")?,
                        dy: number(pan, "dy")?,
                    },
                    pan,
                ))
            } else if let Some(zoom) = params.get("zoom") {
                let factor = number(zoom, "factor")?;
                if factor <= 0.0 {
                    return Err("factor must be positive".into());
                }
                Some((Step::Zoom { factor }, zoom))
            } else {
                None
            };
            let Some((step, run)) = step else {
                request.respond(Ok(self.perf_board_rings()));
                if params.get("clear").and_then(Value::as_bool) == Some(true) {
                    self.board_perf.frame_micros.clear();
                    if let Some(view) = &self.board_spike {
                        view.probe.scene_micros.borrow_mut().clear();
                        view.probe.draw_micros.borrow_mut().clear();
                    }
                }
                return Ok(Task::none());
            };
            if self.board_perf.run.is_some() {
                return Err("a board run is already in flight".into());
            }
            let steps = run
                .get("steps")
                .and_then(Value::as_u64)
                .and_then(|steps| u32::try_from(steps).ok())
                .filter(|steps| (1..=10_000).contains(steps))
                .ok_or("steps must be from 1 to 10000")?;
            self.board_perf.run = Some(Run {
                request: Arc::clone(request),
                step,
                steps,
                pulses: 0,
                last: None,
            });
            Ok(self.await_board_pulse())
        }

        fn await_board_pulse(&self) -> Task<Message> {
            match self.window {
                Some(window) => Task::future(self.frames.next(window)).map(Message::BoardPulse),
                None => Task::done(Message::BoardPulse(Err("window is not open".into()))),
            }
        }

        /// Time from launch to the first redraw after the board's first
        /// `draw` returned.
        pub(in super::super) fn time_board_open(&mut self) -> Task<Message> {
            if self.board_spike.is_none() || self.board_perf.open_micros.is_some() {
                return Task::none();
            }
            self.await_board_pulse()
        }

        /// A redraw began. The interval since the last one spans a whole
        /// update, view, draw and present; the first of a run is left out,
        /// because the frame it covers was drawn before the first step.
        pub(in super::super) fn board_pulse(&mut self, pulse: Result<(), String>) -> Task<Message> {
            let now = Instant::now();
            if let Err(error) = pulse {
                if let Some(run) = self.board_perf.run.take() {
                    run.request.respond(Err(error));
                }
                return Task::none();
            }
            let Some(view) = &mut self.board_spike else {
                return Task::none();
            };
            let Some(run) = &mut self.board_perf.run else {
                if self.board_perf.open_micros.is_none() {
                    if view.probe.first_draw.get().is_none() {
                        return self.await_board_pulse();
                    }
                    self.board_perf.open_micros = Some(
                        now.saturating_duration_since(self.launched)
                            .as_micros()
                            .try_into()
                            .unwrap_or(u64::MAX),
                    );
                }
                return Task::none();
            };
            if let Some(last) = run.last.filter(|_| run.pulses >= 2) {
                push(&mut self.board_perf.frame_micros, last, now);
            }
            run.last = Some(now);
            if run.pulses < run.steps {
                let viewport = view.viewport();
                view.set_viewport(match run.step {
                    Step::Pan { dx, dy } => viewport.panned(dx, dy),
                    Step::Zoom { factor } => viewport.zoomed(factor, (640.0, 360.0)),
                });
            }
            run.pulses += 1;
            if run.pulses <= run.steps + 1 {
                return self.await_board_pulse();
            }
            let request = Arc::clone(&run.request);
            self.board_perf.run = None;
            request.respond(Ok(self.perf_board_rings()));
            Task::none()
        }

        fn perf_board_rings(&self) -> Value {
            let ring = |ring: &VecDeque<u64>| ring.iter().copied().collect::<Vec<_>>();
            let (scenes, draws) = self
                .board_spike
                .as_ref()
                .map(|view| {
                    (
                        ring(&view.probe.scene_micros.borrow()),
                        ring(&view.probe.draw_micros.borrow()),
                    )
                })
                .unwrap_or_default();
            json!({
                "scene_micros": scenes,
                "draw_micros": draws,
                "frame_micros": ring(&self.board_perf.frame_micros),
                "open_micros": self.board_perf.open_micros,
            })
        }

        /// `spike.board`: change what the spike shows, then report it.
        /// `scene` replaces the board (`{"grid": N}` or `{"text": "…"}`),
        /// `labels` sets the label rule (`null` for the renderer's own),
        /// `cap` the most layers, `viewport` where the view looks, and
        /// `nodes` names the nodes to report the boxes and labels of.
        pub(in super::super) fn spike_board(&mut self, params: &Value) -> Result<Value, String> {
            self.spike()?;
            if let Some(scene) = params.get("scene") {
                if let Some(text) = scene.get("text").and_then(Value::as_str) {
                    self.show_board_spike(text)?;
                } else {
                    let count = scene
                        .get("grid")
                        .and_then(Value::as_u64)
                        .and_then(|count| usize::try_from(count).ok())
                        .filter(|count| (1..=100_000).contains(count))
                        .ok_or("scene needs text or a grid of 1 to 100000 shapes")?;
                    self.open_board_spike(count)?;
                }
            }
            match params.get("labels") {
                None => {}
                Some(Value::Null) => self.board_perf.labels = None,
                Some(labels) => {
                    self.board_perf.labels = Some(Labels {
                        min_px: number(labels, "min_px")?,
                        moving: labels
                            .get("moving")
                            .and_then(Value::as_bool)
                            .ok_or("moving must be true or false")?,
                    });
                }
            }
            let labels = self.board_perf.labels;
            let view = self.spike()?;
            if let Some(cap) = params.get("cap") {
                let cap = cap
                    .as_u64()
                    .and_then(|cap| usize::try_from(cap).ok())
                    .filter(|cap| (1..=255).contains(cap))
                    .ok_or("cap must be from 1 to 255")?;
                view.set_layer_cap(cap);
            }
            if let Some(viewport) = params.get("viewport") {
                let zoom = number(viewport, "zoom")?;
                if !(Viewport::MIN_ZOOM..=Viewport::MAX_ZOOM).contains(&zoom) {
                    return Err("zoom must be from 0.1 to 4".into());
                }
                view.set_viewport(Viewport {
                    x: number(viewport, "x")?,
                    y: number(viewport, "y")?,
                    zoom,
                });
            }
            let view = &*view;
            let nodes: Vec<Value> = params
                .get("nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|name| node(view, name.as_str()?))
                .collect();
            let (viewport, palette, probe) = (view.viewport(), view.palette(), &view.probe);
            Ok(json!({
                "count": view.board().nodes.len(),
                "edges": view.board().edges.len(),
                "issues": view.issues().len(),
                "read_only": *view.access() != Access::Editable,
                "items": view.scene().items.len(),
                "layers": view.layers().count,
                "overflow": view.layers().overflow,
                "viewport": {"x": viewport.x, "y": viewport.y, "zoom": viewport.zoom},
                "labels": labels.map(|labels| json!({
                    "min_px": labels.min_px, "moving": labels.moving,
                })),
                "moving": probe.moving.get(),
                "scene_bounds": rect(view.scene().bounds),
                "line_height": scene::line_height(scene::LABEL_SIZE),
                "nodes": nodes,
                "scenes": probe.scenes.get(),
                "builds": probe.builds.get(),
                "painted": probe.painted.get(),
                "painted_layers": probe.layers.get(),
                "palette": {
                    "canvas": rgb(palette.canvas), "ink": rgb(palette.ink),
                    "muted": rgb(palette.muted), "accent": rgb(palette.accent),
                },
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use wren_board::scene::{Draw, Source};

    use super::*;

    #[test]
    fn the_grid_is_a_sound_board_with_every_shape_label_and_connector() {
        let text = grid(1000);
        let board = wren_board::parse(&text).expect("the generated board parses");
        assert_eq!(wren_board::check(&board), []);
        assert_eq!(board.nodes.len(), 1000);
        // 25 rows of 40: each shape but the last of a row connects right,
        // and each but the last row's connects down.
        assert_eq!(board.edges.len(), (1000 - 25) + (1000 - 40));
        let labelled = |needle: &str| {
            board
                .nodes
                .values()
                .filter(|node| node.text.contains(needle))
                .count()
        };
        assert_eq!(labelled("🐦"), 100);
        assert_eq!(labelled("\n"), 1000);
        // It is canonical but for the defaults it spells out, so the
        // writer gives it back with the same elements.
        let written = wren_board::write(&board);
        assert_eq!(wren_board::parse(&written).expect("canonical"), board);
        assert_eq!(
            wren_board::parse(&grid(1)).expect("one shape").edges.len(),
            0
        );
    }

    #[test]
    fn one_shape_in_ten_covers_its_neighbours_label_in_two_layers() {
        fonts::load();
        let view = BoardView::open(&grid(1000), palette(definitions::DARK), fonts::UI)
            .expect("the generated board opens");
        assert_eq!(view.layers().count, 2);
        assert_eq!(view.layers().overflow, 0);
        let raised = view
            .scene()
            .items
            .iter()
            .zip(&view.layers().of)
            .filter(|(item, layer)| {
                **layer == 1
                    && matches!(item.source, Source::Node(_))
                    && matches!(item.draw, Draw::Text { .. })
            })
            .count();
        assert!((90..=100).contains(&raised), "{raised}");
    }
}
