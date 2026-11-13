//! `--board-spike N`: a generated board in memory, shown by the board
//! viewer with no workspace behind it. It is where the viewer is looked at
//! and timed until boards open from a workspace, and it is never saved.

use std::fmt::Write as _;

use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill, Task};
use wren_canvas::BoardView;

use super::{definitions, App, Message};
use crate::{fonts, theme};

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

    /// Show the board `text` holds in place of the one on screen, fitted
    /// to the widget as a board opens.
    fn show_board_spike(&mut self, text: &str) -> Result<(), String> {
        #[cfg(feature = "smoke-test")]
        let opened = std::time::Instant::now();
        // Labels are measured when the view opens, before a window exists.
        fonts::load();
        let mut view = BoardView::open(text, theme::board_palette(self.definition), fonts::UI)
            .map_err(|error| error.to_string())?;
        view.fit();
        self.board_spike = Some(view);
        #[cfg(feature = "smoke-test")]
        self.board_perf.opened(opened);
        Ok(())
    }

    /// Repaint the spike's board in the current theme's colours.
    pub(super) fn retheme_board_spike(&mut self) {
        let palette = theme::board_palette(self.definition);
        if let Some(view) = &mut self.board_spike {
            view.set_palette(palette);
        }
    }

    pub(super) fn board_spike_input(&mut self, message: wren_canvas::Message) -> Task<Message> {
        if let Some(view) = &mut self.board_spike {
            view.update(message, std::time::Instant::now());
        }
        Task::none()
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
        let body = column![
            header,
            view.view("note-editor", self.board_style())
                .map(Message::BoardSpike)
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
    use wren_canvas::{BoardView, Labels, Probe, Scroll, Viewport};

    use super::super::{App, Message};
    use crate::test_ipc::Request;

    /// The timings `perf.board` reports that the view does not keep, the
    /// run in flight, and the label rule the harness asked for. They are
    /// the shown board's: the spike's, or the open note's.
    #[derive(Default)]
    pub struct Perf {
        frame_micros: VecDeque<u64>,
        /// Each draft of the open board (§3.4 rule 7): the canonical write
        /// alone, and that write with the Session's op, which re-indexes
        /// the document.
        write_micros: VecDeque<u64>,
        draft_micros: VecDeque<u64>,
        open_micros: Option<u64>,
        /// When the board on screen began to open; launch for the first.
        opened: Option<Instant>,
        /// Whether that opening still waits for its redraws to be watched.
        unwatched: bool,
        /// When the note operation in progress began, which a board it
        /// opens is timed from.
        pub opening: Option<Instant>,
        run: Option<Run>,
        /// `None` leaves the rule to the widget, which picks by renderer.
        pub labels: Option<Labels>,
    }

    impl Perf {
        /// A draft began at `started`, its text was written by `written`,
        /// and the Session had taken it by `ended`.
        pub fn drafted(&mut self, started: Instant, written: Instant, ended: Instant) {
            push(&mut self.write_micros, started, written);
            push(&mut self.draft_micros, started, ended);
        }

        /// A board began to open at `at`: `open_micros` times it afresh.
        pub fn opened(&mut self, at: Instant) {
            self.opened = Some(at);
            self.open_micros = None;
            self.unwatched = true;
        }
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

        /// The board on screen: the spike's, or the open note's.
        fn shown_board(&self) -> Option<&BoardView> {
            self.board_spike.as_ref().or(self.board.as_ref())
        }

        /// `perf.board`: the timing rings, optionally after a pan or zoom
        /// run of one step per redraw.
        pub(in super::super) fn perf_board(
            &mut self,
            request: &Arc<Request>,
        ) -> Result<Task<Message>, String> {
            let params = &request.params;
            self.shown_board().ok_or("the surface is not a board")?;
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
                    self.board_perf.write_micros.clear();
                    self.board_perf.draft_micros.clear();
                    if let Some(view) = self.shown_board() {
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

        /// Time from the start of the board's opening (launch, for the
        /// first) to the first redraw after the board's first `draw` that
        /// painted it returned. Each opening is watched once, from when
        /// the window is up.
        pub(in super::super) fn time_board_open(&mut self) -> Task<Message> {
            if self.window.is_none() || self.shown_board().is_none() || !self.board_perf.unwatched {
                return Task::none();
            }
            self.board_perf.unwatched = false;
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
            let Some(view) = self.board_spike.as_mut().or(self.board.as_mut()) else {
                if let Some(run) = self.board_perf.run.take() {
                    run.request
                        .respond(Err("the board was closed during the run".into()));
                }
                return Task::none();
            };
            let Some(run) = &mut self.board_perf.run else {
                if self.board_perf.open_micros.is_none() {
                    if view.probe.first_draw.get().is_none() {
                        return self.await_board_pulse();
                    }
                    let opened = self.board_perf.opened.unwrap_or(self.launched);
                    self.board_perf.open_micros = Some(
                        now.saturating_duration_since(opened)
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
                // About the widget's middle; the spike's window before it
                // has said its size.
                let middle = view.size().map_or((640.0, 360.0), |(width, height)| {
                    (width / 2.0, height / 2.0)
                });
                view.set_viewport(match run.step {
                    Step::Pan { dx, dy } => viewport.panned(dx, dy),
                    Step::Zoom { factor } => viewport.zoomed(factor, middle),
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
            let (scenes, draws, counts) = self
                .shown_board()
                .map(|view| {
                    let probe = &view.probe;
                    (
                        ring(&probe.scene_micros.borrow()),
                        ring(&probe.draw_micros.borrow()),
                        [probe.parses.get(), probe.scenes.get(), probe.builds.get()],
                    )
                })
                .unwrap_or_default();
            json!({
                "scene_micros": scenes,
                "draw_micros": draws,
                "frame_micros": ring(&self.board_perf.frame_micros),
                "write_micros": ring(&self.board_perf.write_micros),
                "draft_micros": ring(&self.board_perf.draft_micros),
                "open_micros": self.board_perf.open_micros,
                "parses": counts[0],
                "scenes": counts[1],
                "builds": counts[2],
            })
        }

        /// `spike.board`: change what the spike shows, then report it.
        /// `scene` replaces the board (`{"grid": N}` or `{"text": "…"}`),
        /// opened fitted and timed as a board opens; `edit` gives one node a
        /// new label (`{"node": id, "text": "…"}`) and reloads the view with
        /// the canonical text, as an edit from elsewhere would; `labels`
        /// sets the label rule (`null` for the renderer's own), `cap` the
        /// most layers, `viewport` where the view looks; `fit`,
        /// `zoom_step` (stops, up or down) and `zoom_reset` run the view's
        /// own; and `nodes` names the nodes to report the boxes and labels
        /// of.
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
            let (labels, scale_factor) = (self.board_perf.labels, self.scale_factor);
            let view = self.spike()?;
            if let Some(edit) = params.get("edit") {
                let id = edit
                    .get("node")
                    .and_then(Value::as_str)
                    .and_then(Id::new)
                    .ok_or("edit needs a node id")?;
                let text = edit
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("edit needs the label's text")?;
                let mut board = view.board().clone();
                board.nodes.get_mut(&id).ok_or("no such node")?.text = text.to_owned();
                view.reload(&wren_board::write(&board))
                    .map_err(|error| error.to_string())?;
            }
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
            if params.get("fit").and_then(Value::as_bool) == Some(true) {
                view.fit();
            }
            if let Some(by) = params.get("zoom_step") {
                let by = by
                    .as_i64()
                    .and_then(|by| i8::try_from(by).ok())
                    .ok_or("zoom_step must be from -128 to 127")?;
                view.zoom_step(by);
            }
            if params.get("zoom_reset").and_then(Value::as_bool) == Some(true) {
                view.zoom_reset();
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
                "scale_factor": scale_factor,
                "size": view.size().map(|(width, height)| json!({"width": width, "height": height})),
                "epoch": view.epoch(),
                "panning": view.machine().panning(),
                "space": view.machine().space(),
                "arrows": probe.arrows.get(),
                "wheel": probe.wheel.borrow().iter().map(|delta| match *delta {
                    Scroll::Lines { x, y } => json!({"lines": true, "x": x, "y": y}),
                    Scroll::Pixels { x, y } => json!({"lines": false, "x": x, "y": y}),
                }).collect::<Vec<_>>(),
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
    use std::sync::Arc;

    use resvg::{tiny_skia, usvg};
    use wren_board::scene::{self, Draw, Line, Source};
    use wren_board::svg;

    use super::*;

    macro_rules! sample {
        ($name:literal) => {
            (
                $name,
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tools/wrentest/fixtures/boards/workspace/Boards/Samples/",
                    $name,
                    ".wren-board"
                )),
            )
        };
    }

    /// The boards of the sample workspace.
    const SAMPLES: [(&str, &str); 5] = [
        sample!("Checkout flow"),
        sample!("Shapes"),
        sample!("Overlap"),
        sample!("Hand written"),
        sample!("Future"),
    ];

    /// How many pixels a board unit takes when a line is rendered.
    const RENDER_SCALE: f32 = 4.0;

    /// `element`, a `<text>` of an exported SVG, with its content replaced.
    fn with_content(element: &str, content: &str) -> String {
        let open = element.find('>').expect("a start tag") + 1;
        let close = element.rfind("</text>").expect("an end tag");
        format!("{}{content}{}", &element[..open], &element[close..])
    }

    /// The width of the ink of one `<text>` of an exported SVG as resvg
    /// draws it, in board units: the element alone, in a document whose
    /// view box is the line's box widened on every side.
    fn ink_width(element: &str, line: &Line, size: f32, options: &usvg::Options) -> Option<f32> {
        let reach = size * 2.0;
        let (x, y) = (line.origin.x - reach, line.origin.y - reach);
        let (width, height) = (
            line.width + 2.0 * reach,
            scene::line_height(size) + 2.0 * reach,
        );
        let (pixels_x, pixels_y) = (
            (width * RENDER_SCALE).ceil(),
            (height * RENDER_SCALE).ceil(),
        );
        let document = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{pixels_x}\" height=\"{pixels_y}\" \
             viewBox=\"{x} {y} {} {}\">\n{element}\n</svg>\n",
            pixels_x / RENDER_SCALE,
            pixels_y / RENDER_SCALE,
        );
        let tree = usvg::Tree::from_str(&document, options).expect("an exported line is SVG");
        let mut pixmap = tiny_skia::Pixmap::new(pixels_x as u32, pixels_y as u32)?;
        resvg::render(
            &tree,
            tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        let columns = pixmap.width() as usize;
        let inked: Vec<usize> = (0..columns)
            .filter(|column| {
                pixmap
                    .pixels()
                    .iter()
                    .skip(*column)
                    .step_by(columns)
                    .any(|pixel| pixel.alpha() > 0)
            })
            .collect();
        let (first, last) = (inked.first()?, inked.last()?);
        Some((last + 1 - first) as f32 / RENDER_SCALE)
    }

    /// Phase 1's confirmation step: an SVG viewer that has Inter lays each
    /// exported line Inter can draw out as wide as the canvas measured it,
    /// so a label that fits its box on screen fits it in the export. A
    /// line in another script is drawn in whatever face each side falls
    /// back to.
    #[test]
    fn resvg_draws_each_exported_line_as_wide_as_the_canvas_measured_it() {
        fonts::load();
        let inter = swash::FontRef::from_index(fonts::FACES[0], 0).expect("Inter");
        let drawn_in_inter = |text: &str| {
            text.chars()
                .all(|ch| ch.is_whitespace() || inter.charmap().map(ch) != 0)
        };
        let mut options = usvg::Options::default();
        let mut faces = usvg::fontdb::Database::new();
        faces.load_font_data(fonts::FACES[0].to_vec());
        options.fontdb = Arc::new(faces);
        // The largest advance and ink differences, in board units.
        let mut worst = (0.0f32, 0.0f32);
        let mut checked = 0;
        for (name, text) in SAMPLES {
            let view = BoardView::open(text, theme::board_palette(definitions::DARK), fonts::UI)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let exported = svg::write(
                view.scene(),
                &svg::Options {
                    background: Some(view.palette().canvas),
                    padding: 40.0,
                },
            );
            let lines: Vec<(&Line, f32)> = view
                .scene()
                .items
                .iter()
                .filter_map(|item| match &item.draw {
                    Draw::Text { lines, size, .. } => Some(lines.iter().map(|line| (line, *size))),
                    Draw::Path { .. } => None,
                })
                .flatten()
                .collect();
            let elements: Vec<&str> = exported
                .lines()
                .filter(|element| element.starts_with("<text "))
                .collect();
            assert_eq!(elements.len(), lines.len(), "{name}: one <text> per line");
            for (element, (line, size)) in elements.into_iter().zip(lines) {
                if line.text.trim().is_empty() || !drawn_in_inter(&line.text) {
                    continue;
                }
                let ink = |element: &str| {
                    ink_width(element, line, size, &options)
                        .unwrap_or_else(|| panic!("{name}: no ink for {:?}", line.text))
                };
                // The ink stops short of the advance by the first glyph's
                // left bearing and the last one's right bearing ("End" by
                // 2.3 px), so the run's advance is read as the S4 check
                // read the canvas's: written between two bars, less the
                // two bars alone.
                let content = &element[element.find('>').expect("a start tag") + 1
                    ..element.rfind("</text>").expect("an end tag")];
                let advance = ink(&with_content(element, &format!("|{content}|")))
                    - ink(&with_content(element, "||"));
                let difference = advance - line.width;
                assert!(
                    difference.abs() <= 2.0,
                    "{name}: {:?} advances {advance} in resvg and {} measured",
                    line.text,
                    line.width
                );
                if difference.abs() > worst.0.abs() {
                    worst.0 = difference;
                }
                let bearings = ink(element) - line.width;
                if bearings.abs() > worst.1.abs() {
                    worst.1 = bearings;
                }
                checked += 1;
            }
        }
        assert!(checked >= 40, "{checked} lines");
        eprintln!(
            "{checked} lines; resvg's advance minus the measured width is {:.2} at most, \
             and its ink minus the measured width {:.2}",
            worst.0, worst.1
        );
    }

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
        let view = BoardView::open(
            &grid(1000),
            theme::board_palette(definitions::DARK),
            fonts::UI,
        )
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
