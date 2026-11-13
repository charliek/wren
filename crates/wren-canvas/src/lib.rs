//! The board canvas: an iced widget that paints a [`wren_board::scene`].
//!
//! Like `wren-editor`, it knows no Wren types and no app messages. The host
//! owns a [`BoardView`] per open board, passes in the board's text, a
//! [`Palette`] and the label font, and mounts [`BoardView::view`] under its
//! own focus id. Everything a board shows lives in the view, so a host or a
//! test can read and drive it with no window, and a rebuilt widget tree
//! loses nothing.
//!
//! How a scene reaches the screen was settled by measurement (the plan's
//! spike record, `discovery/boards.md` §8.5):
//!
//! - [`BoardView`] parses the text, builds the scene once with a
//!   [`Measurer`] over the renderer's own text stack, and splits it into
//!   renderer layers ([`layers`]) so a label never shows through a shape
//!   drawn over it.
//! - The widget builds geometry in widget-local pixels for the items in
//!   view, each point already through the [`Viewport`] and each size
//!   already times the zoom, and keeps it until the scene, the viewport,
//!   the widget's size or the label rule's outcome changes. An idle board
//!   builds nothing.
//! - The software renderer blits every glyph of every frame, so there the
//!   widget draws no label while the view moves and none too small to
//!   read ([`Labels`]).
//!
//! Input reaches the view as [`Message`]s, which [`BoardView::update`]
//! runs through the pure [`Machine`]: the wheel pans, or zooms with ⌘ or
//! Ctrl held, and a middle-button or Space drag pans. Zoom and pan are view
//! state, never the board's. With the `test-support` feature a view counts
//! its scene and geometry builds (`Probe`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iced::{Element, Font};
use wren_board::scene::{self, Scene};
use wren_board::{Access, Board, Issue, Palette, ParseError};

mod clip;
mod command;
#[cfg(test)]
mod fixtures;
pub mod handles;
pub mod hit;
pub mod label;
pub mod layers;
pub mod machine;
mod measure;
mod paint;
mod widget;

pub use command::{Command, Grow, Tool};
pub use label::{LabelAction, Motion};
pub use machine::{Button, Input, Key, Machine, Mods, Scroll};
pub use measure::Measurer;

/// How long after its last pan or zoom step a view counts as at rest.
pub const REST: Duration = Duration::from_millis(150);

/// Space left around a board's content when it is fitted to the widget,
/// in screen pixels.
pub const FIT_MARGIN: f32 = 40.0;

/// The zooms the keyboard steps through.
pub const ZOOM_STOPS: [f32; 9] = [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0];

/// Where a board sits in the widget: the board point at the widget's
/// top-left corner, and how many screen pixels a board unit takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Viewport {
    pub const MIN_ZOOM: f32 = 0.1;
    pub const MAX_ZOOM: f32 = 4.0;

    /// A widget-local point in board units.
    pub fn to_board(self, screen: (f32, f32)) -> (f32, f32) {
        (self.x + screen.0 / self.zoom, self.y + screen.1 / self.zoom)
    }

    /// A board point in widget-local pixels.
    pub fn to_screen(self, board: (f32, f32)) -> (f32, f32) {
        (
            (board.0 - self.x) * self.zoom,
            (board.1 - self.y) * self.zoom,
        )
    }

    /// Moved by a distance in screen pixels: a positive `dx` shows what is
    /// further right.
    pub fn panned(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx / self.zoom,
            y: self.y + dy / self.zoom,
            ..self
        }
    }

    /// Zoomed by `factor`, clamped, keeping the board point under the
    /// widget-local point `about` where it is.
    pub fn zoomed(self, factor: f32, about: (f32, f32)) -> Self {
        self.zoomed_to(self.zoom * factor, about)
    }

    /// At `zoom`, clamped, keeping the board point under the widget-local
    /// point `about` where it is. A zoom the clamp leaves unchanged is no
    /// move at all.
    pub fn zoomed_to(self, zoom: f32, about: (f32, f32)) -> Self {
        let zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        if zoom == self.zoom {
            return self;
        }
        let (x, y) = self.to_board(about);
        Self {
            x: x - about.0 / zoom,
            y: y - about.1 / zoom,
            zoom,
        }
    }

    /// The view of a widget of `size` that shows all of `content` with
    /// [`FIT_MARGIN`] around it, centred, never above 100 %. Content with
    /// no area is shown at 100 %, centred on its corner.
    pub fn fitting(content: scene::Rect, size: (f32, f32)) -> Self {
        let room = |widget: f32, content: f32| {
            if content > 0.0 {
                (widget - 2.0 * FIT_MARGIN) / content
            } else {
                f32::INFINITY
            }
        };
        let zoom = room(size.0, content.width)
            .min(room(size.1, content.height))
            .min(1.0)
            .clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        Self {
            x: content.x + content.width / 2.0 - size.0 / 2.0 / zoom,
            y: content.y + content.height / 2.0 - size.1 / 2.0 / zoom,
            zoom,
        }
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

/// Which labels a frame holds. Shapes are cheap on both renderers; glyphs
/// are what the software renderer spends its frame on once much of a
/// board is in view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Labels {
    /// Text smaller than this on screen, in logical pixels, is not drawn.
    pub min_px: f32,
    /// Whether labels are drawn while the view moves, which is until it
    /// has been still for [`REST`].
    pub moving: bool,
}

impl Labels {
    pub const ALWAYS: Self = Self {
        min_px: 0.0,
        moving: true,
    };

    /// What the software renderer was measured to need: nothing under
    /// 4 px, and nothing while the view moves.
    pub const SOFTWARE: Self = Self {
        min_px: 4.0,
        moving: false,
    };

    /// The rule for the renderer iced reports by this name.
    pub fn for_renderer(name: &str) -> Self {
        if name == "tiny-skia" {
            Self::SOFTWARE
        } else {
            Self::ALWAYS
        }
    }
}

/// How the widget presents a board, beyond the board's own colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Which labels are drawn; `None` takes the renderer's own rule
    /// ([`Labels::for_renderer`]).
    pub labels: Option<Labels>,
    /// The window's scale factor, which iced does not give a widget:
    /// precise wheel deltas arrive in physical pixels.
    pub scale_factor: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            labels: None,
            scale_factor: 1.0,
        }
    }
}

/// What the widget tells its host, which hands it to
/// [`BoardView::update`].
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// An input, stamped with the [`BoardView::epoch`] it was meant for.
    Input(u64, Input),
    /// The widget's size in logical pixels, sent when it differs from the
    /// one the view knows.
    Resized { width: f32, height: f32 },
    /// An edit of the open label, stamped with the epoch it was meant for.
    Label(u64, LabelAction),
    /// A key the board layer binds. The host runs it through its own
    /// command path, which reaches [`BoardView::command`].
    Command(Command),
    /// The clipboard's text, read for an [`Effect::ClipboardRead`] and
    /// stamped with the epoch of the view that asked.
    Pasted(u64, String),
}

/// What a message did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update {
    /// Whether the board itself changed. Looking at it from elsewhere is
    /// not a change.
    pub changed: bool,
    /// What the host has to do.
    pub effects: Vec<Effect>,
}

/// What a board asks for. [`BoardView::update`] applies [`Effect::Look`]
/// itself; the rest reach the host in [`Update::effects`].
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Look at the board from here.
    Look(Viewport),
    /// Put this text on the system clipboard.
    ClipboardWrite(String),
    /// Read the system clipboard and send its text back as
    /// [`Message::Pasted`], stamped with the view's epoch.
    ClipboardRead,
    /// Open the actions palette for the selection.
    OpenActions,
    /// Open the jump palette.
    OpenJump,
    /// Tell the user why something did not happen, such as a paste of a
    /// newer board.
    Notice(String),
}

/// A revision no view has had: geometry built for one scene is never
/// taken for another's, even when the host swaps the view under a widget.
fn revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// One open board: its model, what is wrong with it, the scene that draws
/// it and where the widget looks at it.
pub struct BoardView {
    board: Board,
    issues: Vec<Issue>,
    access: Access,
    synced: String,
    palette: Palette,
    measurer: Measurer,
    scene: Scene,
    plan: layers::Plan,
    cap: usize,
    viewport: Viewport,
    revision: u64,
    machine: Machine,
    /// Bumped whenever the board changes underneath its input, so input
    /// meant for the board before is dropped.
    epoch: u64,
    /// The widget's size, once it has said.
    size: Option<(f32, f32)>,
    /// A fit asked for before the widget's size was known.
    fit_pending: bool,
    /// Bumped by each discrete change of view (a zoom step, a reset, a
    /// fit), which the widget draws in full at once instead of as a move.
    jumps: u64,
    #[cfg(feature = "test-support")]
    pub probe: Probe,
}

impl BoardView {
    /// A view of the board `text` holds, at 100 % with the board's origin
    /// at the widget's corner. Labels are measured in `font`, which the
    /// host has loaded.
    pub fn open(text: &str, palette: Palette, font: Font) -> Result<Self, ParseError> {
        let board = wren_board::parse(text)?;
        #[cfg(feature = "test-support")]
        let probe = Probe::default();
        #[cfg(feature = "test-support")]
        probe.parses.set(1);
        let mut view = Self {
            issues: wren_board::check(&board),
            access: wren_board::access(&board),
            board,
            synced: text.to_owned(),
            palette,
            measurer: Measurer::new(font),
            scene: Scene {
                bounds: scene::Rect::default(),
                items: Vec::new(),
            },
            plan: layers::Plan::default(),
            cap: layers::CAP,
            viewport: Viewport::default(),
            revision: 0,
            machine: Machine::default(),
            epoch: 0,
            size: None,
            fit_pending: false,
            jumps: 0,
            #[cfg(feature = "test-support")]
            probe,
        };
        view.build();
        Ok(view)
    }

    /// The text changed underneath the view. It parses first and changes
    /// nothing if that fails; otherwise the board is replaced, the viewport
    /// kept, a drag in flight dropped and the epoch moved on.
    pub fn reload(&mut self, text: &str) -> Result<(), ParseError> {
        if text == self.synced {
            return Ok(());
        }
        #[cfg(feature = "test-support")]
        self.probe.parses.set(self.probe.parses.get() + 1);
        let board = wren_board::parse(text)?;
        self.issues = wren_board::check(&board);
        self.access = wren_board::access(&board);
        self.board = board;
        text.clone_into(&mut self.synced);
        self.machine.reset();
        self.epoch += 1;
        self.build();
        Ok(())
    }

    /// The text this view last took.
    pub fn synced(&self) -> &str {
        &self.synced
    }

    /// A theme change: one scene build, and none for the same colours.
    pub fn set_palette(&mut self, palette: Palette) {
        if palette != self.palette {
            self.palette = palette;
            self.build();
        }
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// What is wrong with the board that still lets it draw.
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    pub fn access(&self) -> &Access {
        &self.access
    }

    /// The renderer layer of each scene item.
    pub fn layers(&self) -> &layers::Plan {
        &self.plan
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Look at the board from `viewport`, its zoom brought within 10 % to
    /// 400 %. A viewport that is not finite is not taken. It replaces a fit
    /// still waiting for the widget's size.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        if viewport.x.is_finite() && viewport.y.is_finite() && viewport.zoom.is_finite() {
            self.viewport = Viewport {
                zoom: viewport.zoom.clamp(Viewport::MIN_ZOOM, Viewport::MAX_ZOOM),
                ..viewport
            };
            self.fit_pending = false;
        }
    }

    /// Step `by` zoom stops ([`ZOOM_STOPS`]) about the widget's centre:
    /// up for a positive `by`, down for a negative one. A zoom between two
    /// stops goes to the next one in that direction.
    pub fn zoom_step(&mut self, by: i8) {
        let zoom = self.viewport.zoom;
        // A stop within a hair of the zoom is the zoom's own.
        let near = |stop: f32| (stop - zoom).abs() <= stop * 1e-4;
        let target = match by.signum() {
            1 => ZOOM_STOPS
                .iter()
                .position(|&stop| stop > zoom && !near(stop))
                .map(|first| first + usize::from(by.unsigned_abs()) - 1),
            -1 => ZOOM_STOPS
                .iter()
                .rposition(|&stop| stop < zoom && !near(stop))
                .map(|last| last.saturating_sub(usize::from(by.unsigned_abs()) - 1)),
            _ => return,
        };
        let stop = target.map_or(zoom, |index| ZOOM_STOPS[index.min(ZOOM_STOPS.len() - 1)]);
        self.jump(self.viewport.zoomed_to(stop, self.centre()));
    }

    /// Back to 100 % about the widget's centre.
    pub fn zoom_reset(&mut self) {
        self.jump(self.viewport.zoomed_to(1.0, self.centre()));
    }

    /// Show all of the board with [`FIT_MARGIN`] around it, never above
    /// 100 %; an empty board at 100 % with its origin in the middle. Before
    /// the widget has said its size the fit waits for it, and the widget
    /// draws no item until then.
    pub fn fit(&mut self) {
        match self.size {
            Some(size) => {
                self.fit_pending = false;
                self.jump(Viewport::fitting(self.scene.bounds, size));
            }
            None => self.fit_pending = true,
        }
    }

    /// Apply a message from the widget at `now`. Input stamped with an
    /// earlier epoch is dropped.
    pub fn update(&mut self, message: Message, now: Instant) -> Update {
        let mut update = Update::default();
        match message {
            Message::Input(epoch, input) if epoch == self.epoch => {
                #[cfg(feature = "test-support")]
                if let Input::Wheel { delta, .. } = input {
                    self.probe.wheeled(delta);
                }
                let view = machine::View {
                    viewport: self.viewport,
                    now,
                };
                for effect in self.machine.step(input, &view) {
                    match effect {
                        Effect::Look(viewport) => self.set_viewport(viewport),
                        effect => update.effects.push(effect),
                    }
                }
            }
            Message::Resized { width, height } => {
                if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
                    self.size = Some((width, height));
                    if self.fit_pending {
                        self.fit();
                    }
                }
            }
            Message::Command(command) => return self.command(command, now),
            Message::Input(..) | Message::Label(..) | Message::Pasted(..) => {}
        }
        update
    }

    /// Run a board command at `now`, from a key, a palette row, a menu
    /// item or the harness (§5.1 rule 5). An open label is committed
    /// first.
    pub fn command(&mut self, command: Command, now: Instant) -> Update {
        let _ = (command, now);
        Update::default()
    }

    /// Before any operation that is not passive (§3.4 rule 3): commit an
    /// open label, cancel a pointer gesture and keyboard Connecting, and
    /// drop a preview.
    pub fn settle(&mut self, now: Instant) -> Update {
        let _ = now;
        Update::default()
    }

    /// The Session saved `text`, which is now the pristine text (§3.4
    /// rule 2).
    pub fn saved(&mut self, text: &str) {
        let _ = text;
    }

    /// The text to send as the board's draft, by the pristine rule (§3.4
    /// rule 2): the pristine text while the board equals the board it
    /// holds, otherwise the canonical text. It becomes the synced text.
    pub fn draft(&mut self) -> String {
        self.synced.clone()
    }

    /// Which board the view's input is for; see [`Message::Input`].
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The widget's size in logical pixels, once it has said.
    pub fn size(&self) -> Option<(f32, f32)> {
        self.size
    }

    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    /// The widget for this view, focusable under the host's `id`.
    pub fn view(&self, id: &'static str, style: Style) -> Element<'_, Message> {
        widget::Canvas::new(self, id, style).into()
    }

    /// Whether a [`fit`](Self::fit) still waits for the widget's size, so
    /// [`viewport`](Self::viewport) is not yet where the board will be
    /// looked at from.
    pub fn fit_pending(&self) -> bool {
        self.fit_pending
    }

    pub(crate) fn jumps(&self) -> u64 {
        self.jumps
    }

    /// The middle of the widget, or its corner before it has a size.
    fn centre(&self) -> (f32, f32) {
        self.size
            .map_or((0.0, 0.0), |(width, height)| (width / 2.0, height / 2.0))
    }

    fn jump(&mut self, viewport: Viewport) {
        if viewport != self.viewport {
            self.set_viewport(viewport);
            self.jumps += 1;
        }
    }

    /// Changes whenever the scene or its layers do.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn measurer(&self) -> &Measurer {
        &self.measurer
    }

    /// The model to the display list and its layers.
    fn build(&mut self) {
        #[cfg(feature = "test-support")]
        let started = std::time::Instant::now();
        self.measurer.sweep();
        self.scene = scene::build(&self.board, &self.palette, &self.measurer);
        self.plan = layers::split(&self.scene.items, self.cap);
        self.revision = revision();
        #[cfg(feature = "test-support")]
        self.probe.built(started);
    }

    /// Paint in at most `cap` layers instead of [`layers::CAP`], to see
    /// what sharing a layer does to a stack of shapes.
    #[cfg(feature = "test-support")]
    pub fn set_layer_cap(&mut self, cap: usize) {
        self.cap = cap;
        self.plan = layers::split(&self.scene.items, self.cap);
        self.revision = revision();
    }
}

/// What a view did, for the host's test harness.
#[cfg(feature = "test-support")]
#[derive(Debug, Default)]
pub struct Probe {
    /// How many times this view parsed a board's text, its opening included.
    pub parses: std::cell::Cell<u64>,
    /// How many times the scene was built.
    pub scenes: std::cell::Cell<u64>,
    /// Microseconds each recent scene build took, layers included.
    pub scene_micros: std::cell::RefCell<std::collections::VecDeque<u64>>,
    /// How many times a layer's geometry was built.
    pub builds: std::cell::Cell<u64>,
    /// How many scene items those builds painted in all.
    pub painted: std::cell::Cell<u64>,
    /// How many layers the last `draw` painted.
    pub layers: std::cell::Cell<usize>,
    /// Whether the last `draw` took the view to be moving.
    pub moving: std::cell::Cell<bool>,
    /// Microseconds spent in each recent `draw`.
    pub draw_micros: std::cell::RefCell<std::collections::VecDeque<u64>>,
    /// When the first `draw` returned.
    pub first_draw: std::cell::Cell<Option<std::time::Instant>>,
    /// How many arrow keys the focused widget took and did nothing with.
    pub arrows: std::cell::Cell<u64>,
    /// The recent wheel deltas, as the toolkit reported them.
    pub wheel: std::cell::RefCell<std::collections::VecDeque<Scroll>>,
}

#[cfg(feature = "test-support")]
impl Probe {
    /// How many samples each ring of timings keeps.
    pub const SAMPLES: usize = 256;

    fn push(
        ring: &std::cell::RefCell<std::collections::VecDeque<u64>>,
        started: std::time::Instant,
    ) {
        let mut ring = ring.borrow_mut();
        if ring.len() == Self::SAMPLES {
            ring.pop_front();
        }
        ring.push_back(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
    }

    fn wheeled(&self, delta: Scroll) {
        let mut wheel = self.wheel.borrow_mut();
        if wheel.len() == Self::SAMPLES {
            wheel.pop_front();
        }
        wheel.push_back(delta);
    }

    fn built(&self, started: std::time::Instant) {
        self.scenes.set(self.scenes.get() + 1);
        Self::push(&self.scene_micros, started);
    }

    pub(crate) fn drew(&self, started: std::time::Instant) {
        Self::push(&self.draw_micros, started);
        if self.first_draw.get().is_none() {
            self.first_draw.set(Some(std::time::Instant::now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use wren_board::scene::{Draw, Source};
    use wren_board::{Reason, Rgba};

    use super::*;

    fn open(text: &str) -> BoardView {
        fixtures::load_fonts();
        BoardView::open(text, fixtures::palette(), fixtures::FONT).expect("a sample board")
    }

    #[test]
    fn a_zoom_keeps_the_point_under_the_pointer() {
        let about = (317.0, 211.0);
        let mut viewport = Viewport {
            x: -40.0,
            y: 125.5,
            zoom: 1.0,
        };
        for factor in [1.1, 1.1, 0.5, 3.0, 0.2, 1.05] {
            let before = viewport.to_board(about);
            viewport = viewport.zoomed(factor, about);
            let after = viewport.to_screen(before);
            assert!((after.0 - about.0).abs() < 0.01 && (after.1 - about.1).abs() < 0.01);
        }
    }

    #[test]
    fn zoom_is_clamped_to_ten_and_four_hundred_percent() {
        let viewport = Viewport::default();
        assert_eq!(viewport.zoomed(100.0, (0.0, 0.0)).zoom, 4.0);
        assert_eq!(viewport.zoomed(0.001, (0.0, 0.0)).zoom, 0.1);
    }

    #[test]
    fn a_pan_moves_by_screen_pixels_at_any_zoom() {
        let viewport = Viewport {
            x: 10.0,
            y: 20.0,
            zoom: 2.0,
        };
        let panned = viewport.panned(8.0, -4.0);
        assert_eq!((panned.x, panned.y, panned.zoom), (14.0, 18.0, 2.0));
        assert_eq!(panned.to_screen((14.0, 18.0)), (0.0, 0.0));
    }

    #[test]
    fn screen_and_board_points_convert_both_ways() {
        for zoom in [0.1, 0.25, 1.0, 1.5, 4.0] {
            let viewport = Viewport {
                x: -312.5,
                y: 48.0,
                zoom,
            };
            // The widget's corner is the viewport's own point.
            assert_eq!(viewport.to_board((0.0, 0.0)), (-312.5, 48.0));
            assert_eq!(viewport.to_screen((-312.5, 48.0)), (0.0, 0.0));
            // A board unit is `zoom` pixels.
            let (x, y) = viewport.to_screen((-212.5, 148.0));
            assert!((x - 100.0 * zoom).abs() < 1e-3 && (y - 100.0 * zoom).abs() < 1e-3);
            for screen in [(0.0, 0.0), (640.0, 360.0), (1279.0, 799.0), (-50.0, 900.0)] {
                let back = viewport.to_screen(viewport.to_board(screen));
                assert!(
                    (back.0 - screen.0).abs() < 0.01 && (back.1 - screen.1).abs() < 0.01,
                    "{screen:?} at {zoom}: {back:?}"
                );
            }
        }
    }

    #[test]
    fn a_view_holds_the_board_its_scene_and_the_text_it_took() {
        let view = open(fixtures::CHECKOUT_FLOW);
        assert_eq!(view.synced(), fixtures::CHECKOUT_FLOW);
        assert_eq!(
            (view.board().nodes.len(), view.board().edges.len()),
            (15, 17)
        );
        assert_eq!(view.issues(), &[]);
        assert_eq!(*view.access(), Access::Editable);
        assert_eq!(view.viewport(), Viewport::default());
        // The scene is the board's, built with the view's own measurer.
        let scene = scene::build(view.board(), view.palette(), view.measurer());
        assert_eq!(*view.scene(), scene);
        assert_eq!(view.layers().of.len(), scene.items.len());
        let labels = scene
            .items
            .iter()
            .filter(|item| {
                matches!(item.draw, Draw::Text { .. }) && matches!(item.source, Source::Node(_))
            })
            .count();
        assert_eq!(labels, 15);
    }

    #[test]
    fn an_unedited_board_drafts_its_text_verbatim() {
        let mut view = open(fixtures::HAND_WRITTEN);
        assert_ne!(wren_board::write(view.board()), fixtures::HAND_WRITTEN);
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
        assert_eq!(view.synced(), fixtures::HAND_WRITTEN);
    }

    #[test]
    fn a_view_reports_issues_and_read_only_boards() {
        let shapes = open(fixtures::SHAPES);
        assert!(!shapes.issues().is_empty());
        assert_eq!(shapes.issues(), wren_board::check(shapes.board()));
        assert_eq!(*shapes.access(), Access::Editable);

        let future = open(fixtures::FUTURE);
        assert_eq!(
            *future.access(),
            Access::ReadOnly(Reason::Requires(vec!["frames".into()]))
        );
        assert!(!future.scene().items.is_empty());
    }

    #[test]
    fn text_that_is_not_a_board_opens_no_view() {
        fixtures::load_fonts();
        let conflict = "schema = 1\n<<<<<<< ours\n[node.a]\n";
        let error = BoardView::open(conflict, fixtures::palette(), fixtures::FONT)
            .err()
            .expect("conflict markers are not TOML");
        assert_eq!(error.line, 2);
    }

    #[test]
    fn a_reload_replaces_the_board_and_keeps_the_viewport() {
        let mut view = open(fixtures::OVERLAP);
        let viewport = Viewport {
            x: -80.0,
            y: 40.0,
            zoom: 2.0,
        };
        view.set_viewport(viewport);
        let revision = view.revision();

        view.reload(fixtures::CHECKOUT_FLOW).expect("a board");
        assert_eq!(view.synced(), fixtures::CHECKOUT_FLOW);
        assert_eq!(view.board().nodes.len(), 15);
        assert_eq!(view.viewport(), viewport);
        assert_ne!(view.revision(), revision);
        assert_eq!(
            *view.scene(),
            scene::build(view.board(), view.palette(), view.measurer())
        );
    }

    #[test]
    fn a_reload_that_does_not_parse_changes_nothing() {
        let mut view = open(fixtures::OVERLAP);
        let (revision, scene) = (view.revision(), view.scene().clone());
        let error = view
            .reload("schema = 1\n[node.a\n")
            .expect_err("broken TOML");
        assert_eq!(error.line, 2);
        assert_eq!(view.synced(), fixtures::OVERLAP);
        assert_eq!(view.board().nodes.len(), 3);
        assert_eq!(view.revision(), revision);
        assert_eq!(*view.scene(), scene);
    }

    #[test]
    fn a_palette_recolours_the_scene_and_keeps_its_geometry() {
        let mut view = open(fixtures::OVERLAP);
        let before = view.scene().clone();
        let revision = view.revision();
        let light = Palette {
            canvas: Rgba::rgb(250, 250, 250),
            ink: Rgba::rgb(20, 20, 20),
            dark: false,
            ..fixtures::palette()
        };
        view.set_palette(light);
        assert_eq!(*view.palette(), light);
        assert_ne!(view.revision(), revision);
        let after = view.scene();
        assert_ne!(*after, before);
        assert_eq!(after.items.len(), before.items.len());
        for (new, old) in after.items.iter().zip(&before.items) {
            assert_eq!(new.bounds, old.bounds);
        }
        // The same colours again are no change at all.
        let revision = view.revision();
        view.set_palette(light);
        assert_eq!(view.revision(), revision);
    }

    #[test]
    fn a_viewport_is_clamped_and_never_not_a_number() {
        let mut view = open(fixtures::OVERLAP);
        let revision = view.revision();
        view.set_viewport(Viewport {
            x: 12.0,
            y: -7.0,
            zoom: 9.0,
        });
        assert_eq!(
            view.viewport(),
            Viewport {
                x: 12.0,
                y: -7.0,
                zoom: 4.0
            }
        );
        view.set_viewport(Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 0.0,
        });
        assert_eq!(view.viewport().zoom, 0.1);
        let kept = view.viewport();
        for broken in [
            Viewport {
                x: f32::NAN,
                ..kept
            },
            Viewport {
                y: f32::INFINITY,
                ..kept
            },
            Viewport {
                zoom: f32::NAN,
                ..kept
            },
        ] {
            view.set_viewport(broken);
            assert_eq!(view.viewport(), kept);
        }
        // Looking elsewhere is not a change to the scene.
        assert_eq!(view.revision(), revision);
    }

    #[test]
    fn no_two_views_share_a_revision() {
        let first = open(fixtures::OVERLAP);
        let second = open(fixtures::OVERLAP);
        assert_ne!(first.revision(), second.revision());
        assert_eq!(*first.scene(), *second.scene());
    }

    #[test]
    fn the_sample_boards_paint_in_at_most_three_layers() {
        for (name, text, layers) in [
            ("Checkout flow", fixtures::CHECKOUT_FLOW, 1),
            ("Shapes", fixtures::SHAPES, 2),
            ("Overlap", fixtures::OVERLAP, 3),
            ("Future", fixtures::FUTURE, 1),
            ("Hand written", fixtures::HAND_WRITTEN, 1),
        ] {
            let view = open(text);
            assert_eq!(view.layers().count, layers, "{name}");
            assert_eq!(view.layers().overflow, 0, "{name}");
        }
    }

    const SIZE: (f32, f32) = (1000.0, 600.0);

    fn sized(text: &str) -> BoardView {
        let mut view = open(text);
        view.update(
            Message::Resized {
                width: SIZE.0,
                height: SIZE.1,
            },
            Instant::now(),
        );
        view
    }

    fn input(view: &mut BoardView, input: Input, now: Instant) -> Update {
        view.update(Message::Input(view.epoch(), input), now)
    }

    #[test]
    fn a_fit_shows_everything_with_the_margin_and_never_above_one_hundred_percent() {
        // A board larger than the widget: one axis has the margin exactly.
        let mut view = sized(fixtures::CHECKOUT_FLOW);
        view.fit();
        let (viewport, bounds) = (view.viewport(), view.scene().bounds);
        assert!(viewport.zoom < 1.0, "{viewport:?}");
        let corner = viewport.to_screen((bounds.x, bounds.y));
        let far = viewport.to_screen((bounds.right(), bounds.bottom()));
        let margins = [corner.0, corner.1, SIZE.0 - far.0, SIZE.1 - far.1];
        for margin in margins {
            assert!(margin >= FIT_MARGIN - 0.01, "{margins:?}");
        }
        // Centred, so the margins of each axis are equal, and the tighter
        // axis has exactly the margin.
        assert!((margins[0] - margins[2]).abs() < 0.01 && (margins[1] - margins[3]).abs() < 0.01);
        assert!(margins
            .iter()
            .any(|margin| (margin - FIT_MARGIN).abs() < 0.01));
    }

    #[test]
    fn a_one_shape_board_fits_at_one_hundred_percent_in_the_middle() {
        let text = "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [300, -40]\nsize = [120, 60]\n";
        let mut view = sized(text);
        view.fit();
        let viewport = view.viewport();
        assert_eq!(viewport.zoom, 1.0);
        let bounds = view.scene().bounds;
        let middle = viewport.to_screen((
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        ));
        assert!((middle.0 - SIZE.0 / 2.0).abs() < 0.01 && (middle.1 - SIZE.1 / 2.0).abs() < 0.01);
    }

    #[test]
    fn an_empty_board_fits_at_one_hundred_percent_with_its_origin_in_the_middle() {
        for text in [wren_board::EMPTY, ""] {
            let mut view = sized(text);
            view.set_viewport(Viewport {
                x: 77.0,
                y: -3.0,
                zoom: 2.5,
            });
            view.fit();
            let viewport = view.viewport();
            assert_eq!(viewport.zoom, 1.0);
            assert_eq!(viewport.to_screen((0.0, 0.0)), (SIZE.0 / 2.0, SIZE.1 / 2.0));
        }
    }

    #[test]
    fn a_widget_smaller_than_its_margins_fits_at_the_least_zoom() {
        let viewport = Viewport::fitting(scene::Rect::new(0.0, 0.0, 500.0, 100.0), (60.0, 60.0));
        assert_eq!(viewport.zoom, Viewport::MIN_ZOOM);
        assert!(viewport.x.is_finite() && viewport.y.is_finite());
        // Content with width but no height fits by its width alone.
        let line = Viewport::fitting(scene::Rect::new(0.0, 10.0, 1840.0, 0.0), (1000.0, 600.0));
        assert!((line.zoom - 0.5).abs() < 1e-6, "{line:?}");
        assert_eq!(line.to_screen((920.0, 10.0)), (500.0, 300.0));
    }

    #[test]
    fn a_fit_before_the_widget_has_a_size_waits_for_it() {
        let mut view = open(fixtures::CHECKOUT_FLOW);
        assert_eq!(view.size(), None);
        view.fit();
        assert!(view.fit_pending());
        assert_eq!(view.viewport(), Viewport::default());
        // A size that is no size is not taken.
        for (width, height) in [(0.0, 600.0), (f32::NAN, 600.0), (800.0, -1.0)] {
            view.update(Message::Resized { width, height }, Instant::now());
            assert!(view.fit_pending());
        }
        let update = view.update(
            Message::Resized {
                width: SIZE.0,
                height: SIZE.1,
            },
            Instant::now(),
        );
        assert_eq!(update, Update::default());
        assert!(!view.fit_pending());
        assert_eq!(view.size(), Some(SIZE));
        assert_eq!(
            view.viewport(),
            Viewport::fitting(view.scene().bounds, SIZE)
        );
        // A later resize keeps where the view looks.
        let fitted = view.viewport();
        view.update(
            Message::Resized {
                width: 400.0,
                height: 300.0,
            },
            Instant::now(),
        );
        assert_eq!(view.viewport(), fitted);
    }

    #[test]
    fn a_viewport_set_before_the_size_replaces_a_waiting_fit() {
        let mut view = open(fixtures::CHECKOUT_FLOW);
        view.fit();
        let chosen = Viewport {
            x: 12.0,
            y: 34.0,
            zoom: 1.5,
        };
        view.set_viewport(chosen);
        assert!(!view.fit_pending());
        view.update(
            Message::Resized {
                width: SIZE.0,
                height: SIZE.1,
            },
            Instant::now(),
        );
        assert_eq!(view.viewport(), chosen);
    }

    #[test]
    fn keyboard_zoom_steps_through_the_stops_about_the_middle() {
        let mut view = sized(fixtures::OVERLAP);
        let middle = (SIZE.0 / 2.0, SIZE.1 / 2.0);
        let under = view.viewport().to_board(middle);
        let mut seen = vec![view.viewport().zoom];
        for _ in 0..6 {
            view.zoom_step(1);
            seen.push(view.viewport().zoom);
        }
        assert_eq!(seen, [1.0, 1.5, 2.0, 3.0, 4.0, 4.0, 4.0]);
        let mut seen = Vec::new();
        for _ in 0..10 {
            view.zoom_step(-1);
            seen.push(view.viewport().zoom);
        }
        assert_eq!(seen, [3.0, 2.0, 1.5, 1.0, 0.75, 0.5, 0.25, 0.1, 0.1, 0.1]);
        let back = view.viewport().to_screen(under);
        assert!((back.0 - middle.0).abs() < 0.01 && (back.1 - middle.1).abs() < 0.01);
        // Several stops at once, and past either end.
        view.zoom_step(3);
        assert_eq!(view.viewport().zoom, 0.75);
        view.zoom_step(100);
        assert_eq!(view.viewport().zoom, 4.0);
        view.zoom_step(-128);
        assert_eq!(view.viewport().zoom, 0.1);
        let kept = view.viewport();
        view.zoom_step(0);
        assert_eq!(view.viewport(), kept);
    }

    #[test]
    fn a_zoom_between_stops_steps_to_the_next_one_each_way() {
        let mut view = sized(fixtures::OVERLAP);
        for (zoom, up, down) in [
            (1.1, 1.5, 1.0),
            (0.3, 0.5, 0.25),
            (0.12, 0.25, 0.1),
            (3.9, 4.0, 3.0),
            // A hair off a stop is that stop.
            (1.000_01, 1.5, 0.75),
            (0.749_99, 1.0, 0.5),
        ] {
            let at = Viewport {
                x: 0.0,
                y: 0.0,
                zoom,
            };
            view.set_viewport(at);
            view.zoom_step(1);
            assert_eq!(view.viewport().zoom, up, "{zoom} up");
            view.set_viewport(at);
            view.zoom_step(-1);
            assert_eq!(view.viewport().zoom, down, "{zoom} down");
        }
    }

    #[test]
    fn a_reset_is_one_hundred_percent_about_the_middle() {
        let mut view = sized(fixtures::OVERLAP);
        view.set_viewport(Viewport {
            x: -100.0,
            y: 50.0,
            zoom: 0.3,
        });
        let middle = (SIZE.0 / 2.0, SIZE.1 / 2.0);
        let under = view.viewport().to_board(middle);
        let jumps = view.jumps();
        view.zoom_reset();
        assert_eq!(view.viewport().zoom, 1.0);
        let back = view.viewport().to_screen(under);
        assert!((back.0 - middle.0).abs() < 0.01 && (back.1 - middle.1).abs() < 0.01);
        assert_eq!(view.jumps(), jumps + 1);
        // At 100 % already, a reset is no jump.
        view.zoom_reset();
        assert_eq!(view.jumps(), jumps + 1);
    }

    #[test]
    fn input_moves_the_view_and_never_the_board() {
        let mut view = sized(fixtures::CHECKOUT_FLOW);
        let now = Instant::now();
        let (text, scenes) = (view.synced().to_owned(), view.revision());
        let wheel = Input::Wheel {
            delta: Scroll::Lines { x: 0.0, y: -2.0 },
            at: (200.0, 100.0),
            zoom: false,
            sideways: false,
            scale_factor: 1.0,
        };
        assert_eq!(input(&mut view, wheel, now), Update::default());
        assert_eq!(view.viewport().y, 2.0 * machine::LINE);
        let zoom = Input::Wheel {
            delta: Scroll::Lines { x: 0.0, y: -2.0 },
            at: (200.0, 100.0),
            zoom: true,
            sideways: false,
            scale_factor: 1.0,
        };
        let later = now + machine::BURST;
        let under = view.viewport().to_board((200.0, 100.0));
        assert_eq!(input(&mut view, zoom, later), Update::default());
        let back = view.viewport().to_screen(under);
        assert!((back.0 - 200.0).abs() < 0.01 && (back.1 - 100.0).abs() < 0.01);
        // A middle drag.
        let before = view.viewport();
        for step in [
            Input::Press {
                button: Button::Middle,
                at: (10.0, 10.0),
            },
            Input::Move { at: (40.0, 10.0) },
            Input::Release {
                button: Button::Middle,
            },
        ] {
            assert_eq!(input(&mut view, step, later), Update::default());
        }
        let dragged = view.viewport().to_screen(before.to_board((10.0, 10.0)));
        assert!((dragged.0 - 40.0).abs() < 0.01 && (dragged.1 - 10.0).abs() < 0.01);
        assert_eq!((view.synced(), view.revision()), (text.as_str(), scenes));
    }

    #[test]
    fn input_for_an_earlier_board_is_dropped() {
        let mut view = sized(fixtures::OVERLAP);
        let now = Instant::now();
        let stale = view.epoch();
        let press = Input::Press {
            button: Button::Middle,
            at: (0.0, 0.0),
        };
        input(&mut view, press, now);
        assert!(view.machine().panning());
        // A reload drops the drag in flight and moves the epoch on.
        view.reload(fixtures::CHECKOUT_FLOW).expect("a board");
        assert!(!view.machine().panning());
        assert_ne!(view.epoch(), stale);
        let kept = view.viewport();
        for old in [press, Input::Move { at: (90.0, 90.0) }] {
            view.update(Message::Input(stale, old), now);
        }
        assert!(!view.machine().panning());
        assert_eq!(view.viewport(), kept);
        // The same text again is no reload and keeps the epoch.
        let epoch = view.epoch();
        view.reload(fixtures::CHECKOUT_FLOW).expect("the same text");
        assert_eq!(view.epoch(), epoch);
    }

    #[test]
    fn space_held_turns_a_left_drag_into_a_pan() {
        let mut view = sized(fixtures::OVERLAP);
        let now = Instant::now();
        input(&mut view, Input::Space { held: true }, now);
        assert!(view.machine().space());
        for step in [
            Input::Press {
                button: Button::Left,
                at: (100.0, 100.0),
            },
            Input::Move { at: (70.0, 60.0) },
            Input::Release {
                button: Button::Left,
            },
            Input::Space { held: false },
        ] {
            input(&mut view, step, now);
        }
        assert_eq!((view.viewport().x, view.viewport().y), (30.0, 40.0));
        assert!(!view.machine().space());
    }

    #[test]
    fn the_renderer_names_its_label_rule() {
        assert_eq!(Labels::for_renderer("tiny-skia"), Labels::SOFTWARE);
        assert_eq!(Labels::for_renderer("wgpu"), Labels::ALWAYS);
        assert_eq!(Style::default().labels, None);
        assert_eq!(Style::default().scale_factor, 1.0);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn the_scene_is_built_once_per_change_and_never_for_a_look() {
        let mut view = open(fixtures::CHECKOUT_FLOW);
        assert_eq!(view.probe.scenes.get(), 1);
        assert_eq!(view.probe.scene_micros.borrow().len(), 1);

        // Reading, looking elsewhere and making the widget build nothing.
        for step in 0..40u8 {
            view.set_viewport(view.viewport().panned(8.0, 0.0));
            view.set_viewport(view.viewport().zoomed(1.05, (640.0, 360.0)));
            let _ = (view.scene(), view.board(), view.issues(), view.layers());
            let _ = view.view("note-editor", Style::default());
            view.reload(fixtures::CHECKOUT_FLOW).expect("the same text");
            view.set_palette(fixtures::palette());
            assert_eq!(view.probe.scenes.get(), 1, "step {step}");
        }

        // A theme change is one build; so is new text.
        view.set_palette(Palette {
            dark: false,
            ..fixtures::palette()
        });
        assert_eq!(view.probe.scenes.get(), 2);
        view.reload(fixtures::OVERLAP).expect("a board");
        assert_eq!(view.probe.scenes.get(), 3);
        // Text that does not parse builds nothing.
        assert!(view.reload("schema = ").is_err());
        assert_eq!(view.probe.scenes.get(), 3);
        // The widget has drawn nothing: there is no renderer here.
        assert_eq!(view.probe.builds.get(), 0);
        assert_eq!(view.probe.layers.get(), 0);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn a_lower_cap_shares_the_last_layer_without_a_scene_build() {
        let mut view = open(fixtures::OVERLAP);
        let revision = view.revision();
        view.set_layer_cap(2);
        assert_eq!(view.layers().count, 2);
        assert!(view.layers().overflow > 0);
        assert_ne!(view.revision(), revision);
        assert_eq!(view.probe.scenes.get(), 1);
        view.set_layer_cap(layers::CAP);
        assert_eq!((view.layers().count, view.layers().overflow), (3, 0));
    }
}
