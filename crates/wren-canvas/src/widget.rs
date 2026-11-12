//! The board widget: a custom advanced widget, not a `canvas::Program`,
//! which cannot reach focus, the input method or renderer layers.
//!
//! It paints the view's scene through iced's geometry API, one renderer
//! layer per layer of the view's plan. Everything it shows lives in the
//! host's [`BoardView`]; its own state is focus, when the view last moved,
//! and geometry.

use std::cell::RefCell;
use std::time::Instant;

use iced::advanced::graphics::geometry::Renderer as _;
use iced::advanced::renderer::Headless as _;
use iced::advanced::widget::{self, operation, tree, Tree};
use iced::advanced::{layout, mouse, renderer, Clipboard, Layout, Renderer as _, Shell, Widget};
use iced::widget::canvas;
use iced::{window, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector};

use crate::paint::{self, Cull};
use crate::{BoardView, Labels, Message, Style, Viewport, REST};

pub(crate) struct Canvas<'a> {
    view: &'a BoardView,
    id: widget::Id,
    style: Style,
}

impl<'a> Canvas<'a> {
    pub(crate) fn new(view: &'a BoardView, id: &'static str, style: Style) -> Self {
        Self {
            view,
            id: widget::Id::new(id),
            style,
        }
    }

    fn labels(&self, renderer: &Renderer) -> Labels {
        self.style
            .labels
            .unwrap_or_else(|| Labels::for_renderer(&renderer.name()))
    }
}

struct State {
    focused: bool,
    /// The time of the last redraw, which decides whether the view rests.
    now: Instant,
    motion: Motion,
    geometry: RefCell<Geometry>,
}

/// When the view last moved, as its redraws show it. The host moves the
/// viewport from several places and none of them has a clock, so the
/// widget dates a move by the first redraw that shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Motion {
    /// The scene and the viewport of the last redraw.
    seen: Option<(u64, Viewport)>,
    moved: Option<Instant>,
}

impl Motion {
    /// A redraw at `now` shows `viewport` over the scene of `revision`.
    /// Another viewport over the same scene is a move; another scene is
    /// another board, or the same one changed, and that is not.
    fn redraw(&mut self, revision: u64, viewport: Viewport, now: Instant) {
        if let Some((seen, from)) = self.seen {
            if seen == revision && from != viewport {
                self.moved = Some(now);
            }
        }
        self.seen = Some((revision, viewport));
    }

    fn moving(&self, now: Instant) -> bool {
        self.moved
            .is_some_and(|moved| now.saturating_duration_since(moved) < REST)
    }

    /// When a moving view comes to rest.
    fn rests_at(&self) -> Option<Instant> {
        self.moved.map(|moved| moved + REST)
    }
}

/// Cached geometry: one cache per layer, for one frame of one scene.
#[derive(Default)]
struct Geometry {
    /// The scene and the frame the caches hold.
    of: Option<(u64, Cull)>,
    /// The scene items each layer shows in that frame.
    shown: Vec<Vec<usize>>,
    caches: Vec<canvas::Cache>,
}

impl operation::Focusable for State {
    fn is_focused(&self) -> bool {
        self.focused
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }
}

impl Widget<Message, Theme, Renderer> for Canvas<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            focused: false,
            now: Instant::now(),
            motion: Motion::default(),
            geometry: RefCell::default(),
        })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.width(Length::Fill).height(Length::Fill).max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                state.now = *now;
                state
                    .motion
                    .redraw(self.view.revision(), self.view.viewport(), *now);
                // Labels left out of a moving frame come back at rest.
                if !self.labels(renderer).moving && state.motion.moving(*now) {
                    if let Some(rest) = state.motion.rests_at() {
                        shell.request_redraw_at(rest);
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(_)) => {
                if cursor.is_over(layout.bounds()) {
                    state.focused = true;
                    shell.capture_event();
                } else {
                    state.focused = false;
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        #[cfg(feature = "test-support")]
        let started = Instant::now();
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        // A frame cannot be made for a widget with no area.
        if bounds.width < 1.0 || bounds.height < 1.0 {
            return;
        }
        let view = self.view;
        let scene = view.scene();

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            paint::color(view.palette().canvas),
        );

        let moving = state.motion.moving(state.now);
        let cull = Cull::new(
            view.viewport(),
            (bounds.width, bounds.height),
            self.labels(renderer),
            moving,
        );
        let mut geometry = state.geometry.borrow_mut();
        let of = Some((view.revision(), cull));
        if geometry.of != of {
            geometry.of = of;
            geometry.shown = paint::shown(scene, view.layers(), &cull);
            let layers = geometry.shown.len();
            geometry.caches.resize_with(layers, canvas::Cache::new);
            geometry.caches.iter().for_each(canvas::Cache::clear);
        }

        #[cfg(feature = "test-support")]
        let mut painted_layers = 0;
        for (items, cache) in geometry.shown.iter().zip(&geometry.caches) {
            // A layer with nothing in view costs the renderer a pass and
            // shows nothing.
            if items.is_empty() {
                continue;
            }
            let drawn = cache.draw(renderer, bounds.size(), |frame| {
                for item in items.iter().filter_map(|index| scene.items.get(*index)) {
                    paint::item(frame, item, cull.viewport, view.measurer());
                }
                #[cfg(feature = "test-support")]
                {
                    let probe = &view.probe;
                    probe.builds.set(probe.builds.get() + 1);
                    probe.painted.set(probe.painted.get() + items.len() as u64);
                }
            });
            renderer.with_layer(bounds, |renderer| {
                renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                    renderer.draw_geometry(drawn);
                });
            });
            #[cfg(feature = "test-support")]
            {
                painted_layers += 1;
            }
        }

        #[cfg(feature = "test-support")]
        {
            view.probe.layers.set(painted_layers);
            view.probe.moving.set(moving);
            view.probe.drew(started);
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        operation.focusable(Some(&self.id), layout.bounds(), state);
    }
}

impl<'a> From<Canvas<'a>> for Element<'a, Message> {
    fn from(canvas: Canvas<'a>) -> Self {
        Element::new(canvas)
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::widget::operation::Focusable as _;

    use super::*;

    fn at(x: f32) -> Viewport {
        Viewport {
            x,
            y: 0.0,
            zoom: 1.0,
        }
    }

    #[test]
    fn a_board_first_seen_is_at_rest() {
        let mut motion = Motion::default();
        let start = Instant::now();
        assert!(!motion.moving(start));
        motion.redraw(7, at(120.0), start);
        assert!(!motion.moving(start));
        assert_eq!(motion.rests_at(), None);
        // Redrawn where it was, it stays at rest.
        motion.redraw(7, at(120.0), start + REST / 3);
        assert!(!motion.moving(start + REST / 3));
    }

    #[test]
    fn a_view_moves_until_it_has_been_still_for_the_rest() {
        let mut motion = Motion::default();
        let start = Instant::now();
        motion.redraw(7, at(0.0), start);
        motion.redraw(7, at(8.0), start + REST);
        assert!(motion.moving(start + REST));
        assert_eq!(motion.rests_at(), Some(start + REST * 2));
        // Each further step starts the rest again.
        motion.redraw(7, at(16.0), start + REST * 3 / 2);
        assert!(motion.moving(start + REST * 2));
        assert_eq!(motion.rests_at(), Some(start + REST * 5 / 2));
        // A redraw without a step does not.
        motion.redraw(7, at(16.0), start + REST * 2);
        assert_eq!(motion.rests_at(), Some(start + REST * 5 / 2));
        assert!(!motion.moving(start + REST * 5 / 2));
        // A clock that reads earlier than the move is not a panic.
        assert!(motion.moving(start));
    }

    #[test]
    fn another_scene_at_another_place_is_not_a_move() {
        let mut motion = Motion::default();
        let start = Instant::now();
        motion.redraw(7, at(0.0), start);
        // The host swapped the view, or the board was reloaded.
        motion.redraw(8, at(500.0), start + REST);
        assert!(!motion.moving(start + REST));
        // From there on, the new scene's viewport is the one followed.
        motion.redraw(8, at(508.0), start + REST * 2);
        assert!(motion.moving(start + REST * 2));
    }

    #[test]
    fn the_widget_state_takes_and_gives_up_focus() {
        let mut state = State {
            focused: false,
            now: Instant::now(),
            motion: Motion::default(),
            geometry: RefCell::default(),
        };
        assert!(!state.is_focused());
        state.focus();
        assert!(state.is_focused());
        state.unfocus();
        assert!(!state.is_focused());
    }
}
