//! `--board-spike N`: a generated board in memory, to pan and zoom. It
//! measures how a board is best painted before the viewer is built, and is
//! never saved.

use std::time::Instant;

use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill, Task};
use wren_board::{Palette, Rgba};
use wren_canvas::spike::{self, View};
use wren_core::theme::{Appearance, Rgb, Theme};

use super::{columns, definitions, App, Message};
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

impl App {
    pub(super) fn open_board_spike(&mut self, count: usize) {
        fonts::load();
        let started = Instant::now();
        let view = View::grid(count, palette(self.definition), fonts::UI);
        #[cfg(feature = "smoke-test")]
        self.board_perf.built(started);
        #[cfg(not(feature = "smoke-test"))]
        let _ = started;
        self.board_spike = Some(view);
    }

    /// Repaint the spike's board in the current theme's colours.
    pub(super) fn retheme_board_spike(&mut self) {
        let palette = palette(self.definition);
        let Some(view) = &self.board_spike else {
            return;
        };
        if *view.palette() == palette {
            return;
        }
        let (count, viewport) = (view.node_count(), view.viewport());
        let (strategy, labels) = (view.strategy(), view.labels());
        // The rows of measured labels are set by the harness, in one theme.
        if count == 0 {
            return;
        }
        self.open_board_spike(count);
        if let Some(view) = &mut self.board_spike {
            view.set_viewport(viewport, Instant::now());
            view.set_strategy(strategy);
            view.set_labels(labels);
        }
    }

    pub(super) fn board_spike_input(&mut self, message: spike::Message) -> Task<Message> {
        if self.dialog.is_some() || self.palette.is_some() {
            return Task::none();
        }
        // The wheel reaches an unfocused board; anything else means the
        // board has the keyboard.
        if !matches!(message, spike::Message::Wheel { .. }) {
            self.column = columns::Column::Editor;
        }
        if let Some(view) = &mut self.board_spike {
            view.update(message, Instant::now());
        }
        Task::none()
    }

    pub(super) fn board_spike_view<'a>(&'a self, view: &'a View) -> Element<'a, Message> {
        let colors = self.definition.colors;
        let viewport = view.viewport();
        let header = container(
            row![
                text("Wren").size(18).color(theme::color(colors.accent)),
                text(format!(
                    "Board spike · in memory · {} shapes · {:.0}%",
                    view.node_count(),
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
        let body = column![header, view.view("note-editor").map(Message::BoardSpike)];
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
    use wren_board::scene::{Draw, Rect};
    use wren_board::Rgba;
    use wren_canvas::spike::{Edit, Labels, Probe, Scroll, Strategy, View};
    use wren_canvas::Viewport;

    use super::super::{App, Message};
    use crate::fonts;
    use crate::test_ipc::Request;

    /// The timings `perf.board` reports, and the run in flight.
    #[derive(Default)]
    pub struct Perf {
        scene_micros: VecDeque<u64>,
        frame_micros: VecDeque<u64>,
        open_micros: Option<u64>,
        run: Option<Run>,
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

    impl Perf {
        pub fn built(&mut self, started: Instant) {
            push(&mut self.scene_micros, started, Instant::now());
        }
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

    impl App {
        fn spike(&mut self) -> Result<&mut View, String> {
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
                    self.board_perf.scene_micros.clear();
                    if let Some(view) = &self.board_spike {
                        view.probe.draws.borrow_mut().clear();
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
                let stepped = match run.step {
                    Step::Pan { dx, dy } => viewport.panned(dx, dy),
                    Step::Zoom { factor } => viewport.zoomed(factor, (640.0, 360.0)),
                };
                view.set_viewport(stepped, now);
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
            let draws = self
                .board_spike
                .as_ref()
                .map(|view| ring(&view.probe.draws.borrow()))
                .unwrap_or_default();
            json!({
                "scene_micros": ring(&self.board_perf.scene_micros),
                "draw_micros": draws,
                "frame_micros": ring(&self.board_perf.frame_micros),
                "open_micros": self.board_perf.open_micros,
            })
        }

        /// `spike.board`: set the spike's knobs, then report its state.
        pub(in super::super) fn spike_board(&mut self, params: &Value) -> Result<Value, String> {
            let now = Instant::now();
            if let Some(scene) = params.get("scene") {
                self.spike()?;
                let palette = super::palette(self.definition);
                fonts::load();
                let started = Instant::now();
                let view = if let Some(labels) = scene.get("labels").and_then(Value::as_array) {
                    let labels: Vec<String> = labels
                        .iter()
                        .map(|label| label.as_str().map(str::to_owned))
                        .collect::<Option<_>>()
                        .ok_or("labels must be strings")?;
                    View::rows(&labels, palette, fonts::UI)
                } else {
                    let count = scene
                        .get("grid")
                        .and_then(Value::as_u64)
                        .and_then(|count| usize::try_from(count).ok())
                        .filter(|count| (1..=100_000).contains(count))
                        .ok_or("scene needs labels or a grid of 1 to 100000 shapes")?;
                    View::grid(count, palette, fonts::UI)
                };
                self.board_perf.built(started);
                self.board_spike = Some(view);
            }
            let view = self.spike()?;
            if let Some(strategy) = params.get("strategy").and_then(Value::as_str) {
                view.set_strategy(match strategy {
                    "transform" => Strategy::Transform,
                    "rebuild" => Strategy::Rebuild,
                    "settle" => Strategy::Settle,
                    _ => return Err("strategy must be transform, rebuild or settle".into()),
                });
            }
            if let Some(layers) = params.get("layers") {
                let layers = match layers {
                    Value::Null => None,
                    value => Some(
                        value
                            .as_u64()
                            .and_then(|layers| usize::try_from(layers).ok())
                            .filter(|layers| (1..=255).contains(layers))
                            .ok_or("layers must be null or from 1 to 255")?,
                    ),
                };
                view.force_layers(layers);
            }
            match params.get("labels") {
                None => {}
                // The renderer's own rule.
                Some(Value::Null) => view.set_labels(None),
                Some(labels) => {
                    let min_px = number(labels, "min_px")?;
                    let moving_lines = match labels.get("moving_lines") {
                        None | Some(Value::Null) => usize::MAX,
                        Some(lines) => lines
                            .as_u64()
                            .and_then(|lines| usize::try_from(lines).ok())
                            .ok_or("moving_lines must be null or a count")?,
                    };
                    view.set_labels(Some(Labels {
                        min_px,
                        moving_lines,
                    }));
                }
            }
            if let Some(cap) = params.get("cap") {
                let cap = cap
                    .as_u64()
                    .and_then(|cap| usize::try_from(cap).ok())
                    .filter(|cap| (1..=255).contains(cap))
                    .ok_or("cap must be from 1 to 255")?;
                view.set_cap(cap);
            }
            if let Some(viewport) = params.get("viewport") {
                let zoom = number(viewport, "zoom")?;
                if !(Viewport::MIN_ZOOM..=Viewport::MAX_ZOOM).contains(&zoom) {
                    return Err("zoom must be from 0.1 to 4".into());
                }
                view.set_viewport(
                    Viewport {
                        x: number(viewport, "x")?,
                        y: number(viewport, "y")?,
                        zoom,
                    },
                    now,
                );
            }
            if let Some(label) = params.get("label") {
                let text = |name: &str| label.get(name).and_then(Value::as_str).map(str::to_owned);
                let edit = if label.get("open").is_some() {
                    Edit::Open
                } else if let Some(text) = text("type") {
                    Edit::Insert(text)
                } else if let Some(content) = text("preedit") {
                    let selection = (!content.is_empty()).then_some(0..content.len());
                    Edit::Preedit { content, selection }
                } else if let Some(text) = text("commit") {
                    Edit::Commit(text)
                } else if label.get("close").is_some() {
                    Edit::Close
                } else {
                    return Err("label needs open, type, preedit, commit or close".into());
                };
                view.edit(edit);
            }
            let view = &*view;
            let viewport = view.viewport();
            let nodes: Vec<Value> = params
                .get("nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|node| usize::try_from(node.as_u64()?).ok())
                .filter_map(|node| {
                    Some(json!({
                        "node": node,
                        "bounds": rect(view.node_bounds(node)?),
                        "lines": view.node_lines(node).iter().map(|line| json!({
                            "text": line.text, "x": line.origin.x, "y": line.origin.y, "width": line.width,
                        })).collect::<Vec<_>>(),
                    }))
                })
                .collect();
            let lines: Vec<Value> = if view.node_count() == 0 {
                view.scene()
                    .items
                    .iter()
                    .filter_map(|item| match &item.draw {
                        Draw::Text { lines, .. } => lines.first(),
                        Draw::Path { .. } => None,
                    })
                    .map(|line| {
                        json!({
                            "text": line.text, "x": line.origin.x, "y": line.origin.y, "width": line.width,
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let palette = view.palette();
            Ok(json!({
                "count": view.node_count(),
                "items": view.scene().items.len(),
                "layers": view.plan().count,
                "overflow": view.plan().overflow,
                "strategy": match view.strategy() {
                    Strategy::Transform => "transform",
                    Strategy::Rebuild => "rebuild",
                    Strategy::Settle => "settle",
                },
                "viewport": {"x": viewport.x, "y": viewport.y, "zoom": viewport.zoom},
                "labels": view.labels().map(|labels| json!({
                    "min_px": labels.min_px,
                    "moving_lines": Some(labels.moving_lines).filter(|lines| *lines != usize::MAX),
                })),
                "moving": view.moving(now),
                "revision": view.revision(),
                "scene_bounds": rect(view.scene().bounds),
                "line_height": view.line_height(wren_board::scene::Face::Regular, 14.0),
                "overlaps": view.overlaps().iter().map(|overlap| json!({
                    "region": rect(overlap.region), "fill": rgb(overlap.fill),
                })).collect::<Vec<_>>(),
                "nodes": nodes,
                "lines": lines,
                "label": view.label().map(|label| json!({
                    "node": label.node, "text": label.text, "caret": label.caret,
                    "preedit": label.preedit.as_ref().map(|preedit| preedit.content.clone()),
                })),
                "caret": view.caret().map(rect),
                "ime": view.probe.ime.get().map(|(cursor, text_size)| json!({
                    "cursor": {"x":cursor[0],"y":cursor[1],"width":cursor[2],"height":cursor[3]},
                    "text_size": text_size,
                })),
                "wheel": view.probe.wheel.borrow().iter().map(|delta| match delta {
                    Scroll::Lines { x, y } => json!({"unit":"lines","x":x,"y":y}),
                    Scroll::Pixels { x, y } => json!({"unit":"pixels","x":x,"y":y}),
                }).collect::<Vec<_>>(),
                "arrows": view.probe.arrows.get(),
                "builds": view.probe.builds.get(),
                "painted": view.probe.painted.get(),
                "painted_layers": view.probe.layers.get(),
                "palette": {
                    "canvas": rgb(palette.canvas), "ink": rgb(palette.ink),
                    "muted": rgb(palette.muted), "accent": rgb(palette.accent),
                },
            }))
        }
    }
}
