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
//! The viewer takes no input yet beyond focus. With the `test-support`
//! feature a view counts its scene and geometry builds ([`Probe`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use iced::{Element, Font};
use wren_board::scene::{self, Scene};
use wren_board::{Access, Board, Issue, Palette, ParseError};

#[cfg(test)]
mod fixtures;
pub mod layers;
mod measure;
mod paint;
mod widget;

pub use measure::Measurer;

/// How long after its last pan or zoom step a view counts as at rest.
pub const REST: Duration = Duration::from_millis(150);

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
        let zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let (x, y) = self.to_board(about);
        Self {
            x: x - about.0 / zoom,
            y: y - about.1 / zoom,
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    /// Which labels are drawn; `None` takes the renderer's own rule
    /// ([`Labels::for_renderer`]).
    pub labels: Option<Labels>,
}

/// What the widget tells its host. The viewer has nothing to say: it
/// takes focus and paints.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {}

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
    #[cfg(feature = "test-support")]
    pub probe: Probe,
}

impl BoardView {
    /// A view of the board `text` holds, at 100 % with the board's origin
    /// at the widget's corner. Labels are measured in `font`, which the
    /// host has loaded.
    pub fn open(text: &str, palette: Palette, font: Font) -> Result<Self, ParseError> {
        let board = wren_board::parse(text)?;
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
            #[cfg(feature = "test-support")]
            probe: Probe::default(),
        };
        view.build();
        Ok(view)
    }

    /// The text changed underneath the view. It parses first and changes
    /// nothing if that fails; otherwise the board is replaced and the
    /// viewport kept.
    pub fn reload(&mut self, text: &str) -> Result<(), ParseError> {
        if text == self.synced {
            return Ok(());
        }
        let board = wren_board::parse(text)?;
        self.issues = wren_board::check(&board);
        self.access = wren_board::access(&board);
        self.board = board;
        text.clone_into(&mut self.synced);
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
    /// 400 %. A viewport that is not finite is not taken.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        if viewport.x.is_finite() && viewport.y.is_finite() && viewport.zoom.is_finite() {
            self.viewport = Viewport {
                zoom: viewport.zoom.clamp(Viewport::MIN_ZOOM, Viewport::MAX_ZOOM),
                ..viewport
            };
        }
    }

    /// The widget for this view, focusable under the host's `id`.
    pub fn view(&self, id: &'static str, style: Style) -> Element<'_, Message> {
        widget::Canvas::new(self, id, style).into()
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
        ] {
            let view = open(text);
            assert_eq!(view.layers().count, layers, "{name}");
            assert_eq!(view.layers().overflow, 0, "{name}");
        }
    }

    #[test]
    fn the_renderer_names_its_label_rule() {
        assert_eq!(Labels::for_renderer("tiny-skia"), Labels::SOFTWARE);
        assert_eq!(Labels::for_renderer("wgpu"), Labels::ALWAYS);
        assert_eq!(Style::default().labels, None);
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
