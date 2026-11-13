//! The board widget: a custom advanced widget, not a `canvas::Program`,
//! which cannot reach focus, the input method or renderer layers.
//!
//! It paints the view's scene through iced's geometry API, one renderer
//! layer per layer of the view's plan, and turns iced's events into the
//! machine's [`Input`]s. Everything it shows lives in the host's
//! [`BoardView`]; its own state is focus, the pointer, when the view last
//! moved, and geometry.

use std::cell::RefCell;
use std::time::Instant;

use iced::advanced::graphics::geometry::Renderer as _;
use iced::advanced::renderer::Headless as _;
use iced::advanced::widget::{self, operation, tree, Tree};
use iced::advanced::{layout, mouse, renderer, Clipboard, Layout, Renderer as _, Shell, Widget};
use iced::keyboard::{self, key::Named, Key};
use iced::widget::canvas;
use iced::{window, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::keys::{self, Keyed, Out};
use crate::machine::{Button, Input, Scroll};
use crate::overlay;
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
    modifiers: keyboard::Modifiers,
    /// Where the last `CursorMoved` put the pointer. iced hands `update`
    /// the cursor as it is after a whole batch of events, so a press and
    /// the move after it can arrive together; a drag starts from here.
    pointer: Option<Point>,
    /// The button whose press the widget sent, until its release.
    pressed: Option<mouse::Button>,
    motion: Motion,
    geometry: RefCell<Geometry>,
}

impl State {
    fn new() -> Self {
        Self {
            focused: false,
            now: Instant::now(),
            modifiers: keyboard::Modifiers::default(),
            pointer: None,
            pressed: None,
            motion: Motion::default(),
            geometry: RefCell::default(),
        }
    }
}

/// When the view last moved, as its redraws show it. The host moves the
/// viewport from several places and none of them has a clock, so the
/// widget dates a move by the first redraw that shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Motion {
    /// The scene, the view's count of jumps and the viewport of the last
    /// redraw.
    seen: Option<(u64, u64, Viewport)>,
    moved: Option<Instant>,
}

impl Motion {
    /// A redraw at `now` shows `viewport` over the scene of `revision`,
    /// after `jumps` discrete changes of view. Another viewport over the
    /// same scene is a move, unless a jump (a zoom step, a reset, a fit)
    /// put it there: one jump is drawn in full at once. Another scene is
    /// another board, or the same one changed, and that is not a move
    /// either.
    fn redraw(&mut self, revision: u64, jumps: u64, viewport: Viewport, now: Instant) {
        if let Some((seen, jumped, from)) = self.seen {
            if seen == revision && jumped == jumps && from != viewport {
                self.moved = Some(now);
            }
        }
        self.seen = Some((revision, jumps, viewport));
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
        tree::State::new(State::new())
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
        let bounds = layout.bounds();
        let view = self.view;
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            state.now = *now;
            let size = (bounds.width, bounds.height);
            if view.size() != Some(size) && size.0 > 0.0 && size.1 > 0.0 {
                shell.publish(Message::Resized {
                    width: size.0,
                    height: size.1,
                });
            }
            // A fit waiting for the size draws nothing, so it is not seen
            // and its viewport is no move.
            if !view.fit_pending() {
                let revision = view.picture().revision;
                state
                    .motion
                    .redraw(revision, view.jumps(), view.viewport(), *now);
            }
            // Labels left out of a moving frame come back at rest.
            if !self.labels(renderer).moving && state.motion.moving(*now) {
                if let Some(rest) = state.motion.rests_at() {
                    shell.request_redraw_at(rest);
                }
            }
        }
        let sent = translate(state, event, bounds, cursor, view, self.style);
        for message in sent.messages {
            shell.publish(message);
        }
        if sent.captured {
            shell.capture_event();
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
        let picture = view.picture();
        let scene = picture.scene;

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            paint::color(view.palette().canvas),
        );
        // A board waiting to be fitted would show one frame from the
        // wrong place.
        if view.fit_pending() {
            return;
        }

        let moving = state.motion.moving(state.now);
        let cull = Cull::new(
            view.viewport(),
            (bounds.width, bounds.height),
            self.labels(renderer),
            moving,
        );
        let mut geometry = state.geometry.borrow_mut();
        let of = Some((picture.revision, cull));
        if geometry.of != of {
            geometry.of = of;
            geometry.shown = paint::shown(scene, picture.plan, &cull);
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
                    paint::item(frame, item, &cull, view.measurer());
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

        // Over every layer of the scene, and rebuilt with each frame: a
        // handful of paths.
        let overlay = overlay::marks(view);
        if !overlay.marks.is_empty() {
            let mut frame = canvas::Frame::new(renderer, bounds.size());
            overlay::paint(&mut frame, &overlay, view, &cull);
            let drawn = frame.into_geometry();
            renderer.with_layer(bounds, |renderer| {
                renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                    renderer.draw_geometry(drawn);
                });
            });
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

/// What one event sends the view, and whether the board takes it.
#[derive(Debug, Default, PartialEq)]
struct Sent {
    messages: Vec<Message>,
    captured: bool,
}

/// iced's `event` to the board's messages (§5.1, §5.13 rule 3): pointer
/// input over the widget, and while a button it pressed is held wherever
/// the pointer goes; keys only while the widget has the focus, through the
/// host's policy; and the focus itself, which reaches the machine with the
/// next event after it changes.
fn translate(
    state: &mut State,
    event: &Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
    view: &BoardView,
    style: Style,
) -> Sent {
    let mut sent = Sent::default();
    let local = |point: Point| (point.x - bounds.x, point.y - bounds.y);
    let input = |input: Input| Message::Input(view.epoch(), input);
    let machine = view.machine();
    // Whether the machine holds something that has to end when the pointer
    // or the window goes.
    let holding = machine.dragging() || machine.space();
    match event {
        Event::Window(window::Event::Unfocused) if holding => {
            sent.messages.push(input(Input::Cancel));
        }
        // The pointer left the window with no button held.
        Event::Mouse(mouse::Event::CursorLeft) if holding && state.pressed.is_none() => {
            sent.messages.push(input(Input::Cancel));
        }
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
            state.modifiers = *modifiers;
            if state.focused || machine.dragging() {
                sent.messages.extend(mods_change(*modifiers, view));
            }
        }
        // The wheel reaches an unfocused board under the pointer.
        Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
            if let Some(at) = pointer(state, cursor, bounds) {
                let delta = match *delta {
                    mouse::ScrollDelta::Lines { x, y } => Scroll::Lines { x, y },
                    mouse::ScrollDelta::Pixels { x, y } => Scroll::Pixels { x, y },
                };
                sent.messages.push(input(Input::Wheel {
                    delta,
                    at: local(at),
                    zoom: state.modifiers.command(),
                    sideways: state.modifiers.shift(),
                    scale_factor: style.scale_factor,
                }));
                sent.captured = true;
            }
        }
        Event::Mouse(mouse::Event::ButtonPressed(button)) => match pointer(state, cursor, bounds) {
            Some(at) => {
                state.focused = true;
                if state.pressed.is_none() {
                    state.pressed = Some(*button);
                    // A Shift-click or a duplicate-drag reads the modifiers
                    // held at its press.
                    sent.messages.extend(mods_change(state.modifiers, view));
                    sent.messages.push(input(Input::Press {
                        button: machine_button(*button),
                        at: local(at),
                    }));
                }
                sent.captured = true;
            }
            None => {
                if state.focused && holding {
                    sent.messages.push(input(Input::Cancel));
                }
                state.focused = false;
            }
        },
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            state.pointer = Some(*position);
            // Wherever the pointer goes while the button is down, outside
            // the widget included.
            if state.pressed.is_some() {
                sent.messages.push(input(Input::Move {
                    at: local(*position),
                }));
                sent.captured = true;
            }
        }
        Event::Mouse(mouse::Event::ButtonReleased(button)) => {
            let ours = state.pressed == Some(*button);
            if let Some(release) = released(&mut state.pressed, *button, machine.dragging()) {
                sent.messages.push(input(release));
                sent.captured = ours;
            }
        }
        // iced hands key events to every widget; only a focused board acts
        // on them, except to let go of a Space it was given.
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            text,
            repeat,
            ..
        }) if state.focused => {
            let context = keys::Context {
                editing: machine.editing().is_some(),
                escapes: machine.escapes(),
                policy: view.key_policy(),
            };
            let keyed = keys::route(
                key,
                *physical_key,
                *modifiers,
                text.as_deref(),
                *repeat,
                context,
            );
            if let Keyed::Take(outs) = keyed {
                #[cfg(feature = "test-support")]
                if keys::is_arrow(key) {
                    view.probe.arrows.set(view.probe.arrows.get() + 1);
                }
                sent.messages.extend(outs.into_iter().map(|out| match out {
                    Out::Input(taken) => input(taken),
                    Out::Command(command) => Message::Command(command),
                }));
                sent.captured = true;
            }
        }
        Event::Keyboard(keyboard::Event::KeyReleased {
            key: Key::Named(Named::Space),
            ..
        }) if machine.space() => {
            sent.messages.push(input(Input::Space { held: false }));
            sent.captured = true;
        }
        _ => {}
    }
    // A focus operation has no shell to say it with, so whether the board
    // has the keyboard reaches the machine with the next event: losing it
    // settles the board (§5.15 rule 4).
    if state.focused != machine.focused() {
        sent.messages.push(input(Input::Focus(state.focused)));
    }
    sent
}

/// The pointer over the widget: where the last move put it, or iced's
/// cursor when no move has been seen there. `None` when the pointer is
/// elsewhere, or over an overlay drawn above the widget.
fn pointer(state: &State, cursor: mouse::Cursor, bounds: Rectangle) -> Option<Point> {
    let end = cursor.position_over(bounds)?;
    Some(
        state
            .pointer
            .filter(|pointer| bounds.contains(*pointer))
            .unwrap_or(end),
    )
}

/// What a release of `button` sends. The release of the button the widget
/// pressed ends its drag. A release it did not see pressed still reaches a
/// machine with a gesture in flight: iced rebuilds the widget's state when
/// the tree around the board changes during a drag (opening the palette
/// does), and the machine would otherwise wait for a release that has come
/// and gone.
fn released(
    pressed: &mut Option<mouse::Button>,
    button: mouse::Button,
    dragging: bool,
) -> Option<Input> {
    if *pressed == Some(button) {
        *pressed = None;
    } else if pressed.is_some() || !dragging {
        return None;
    }
    Some(Input::Release {
        button: machine_button(button),
    })
}

fn machine_button(button: mouse::Button) -> Button {
    match button {
        mouse::Button::Left => Button::Left,
        mouse::Button::Middle => Button::Middle,
        mouse::Button::Right => Button::Right,
        _ => Button::Other,
    }
}

/// The modifiers for the machine, when they are not what it has.
fn mods_change(modifiers: keyboard::Modifiers, view: &BoardView) -> Option<Message> {
    let mods = keys::mods(modifiers);
    (mods != view.machine().mods()).then(|| Message::Input(view.epoch(), Input::Modifiers(mods)))
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
        motion.redraw(7, 0, at(120.0), start);
        assert!(!motion.moving(start));
        assert_eq!(motion.rests_at(), None);
        // Redrawn where it was, it stays at rest.
        motion.redraw(7, 0, at(120.0), start + REST / 3);
        assert!(!motion.moving(start + REST / 3));
    }

    #[test]
    fn a_view_moves_until_it_has_been_still_for_the_rest() {
        let mut motion = Motion::default();
        let start = Instant::now();
        motion.redraw(7, 0, at(0.0), start);
        motion.redraw(7, 0, at(8.0), start + REST);
        assert!(motion.moving(start + REST));
        assert_eq!(motion.rests_at(), Some(start + REST * 2));
        // Each further step starts the rest again.
        motion.redraw(7, 0, at(16.0), start + REST * 3 / 2);
        assert!(motion.moving(start + REST * 2));
        assert_eq!(motion.rests_at(), Some(start + REST * 5 / 2));
        // A redraw without a step does not.
        motion.redraw(7, 0, at(16.0), start + REST * 2);
        assert_eq!(motion.rests_at(), Some(start + REST * 5 / 2));
        assert!(!motion.moving(start + REST * 5 / 2));
        // A clock that reads earlier than the move is not a panic.
        assert!(motion.moving(start));
    }

    #[test]
    fn another_scene_at_another_place_is_not_a_move() {
        let mut motion = Motion::default();
        let start = Instant::now();
        motion.redraw(7, 0, at(0.0), start);
        // The host swapped the view, or the board was reloaded.
        motion.redraw(8, 0, at(500.0), start + REST);
        assert!(!motion.moving(start + REST));
        // From there on, the new scene's viewport is the one followed.
        motion.redraw(8, 0, at(508.0), start + REST * 2);
        assert!(motion.moving(start + REST * 2));
    }

    #[test]
    fn a_jump_is_drawn_at_once_and_a_move_after_it_is_a_move() {
        let mut motion = Motion::default();
        let start = Instant::now();
        motion.redraw(7, 0, at(0.0), start);
        // A zoom step, a reset or a fit.
        motion.redraw(7, 1, at(300.0), start + REST);
        assert!(!motion.moving(start + REST));
        motion.redraw(7, 1, at(308.0), start + REST * 2);
        assert!(motion.moving(start + REST * 2));
        // A jump during a move does not end the move.
        motion.redraw(7, 2, at(600.0), start + REST * 2);
        assert!(motion.moving(start + REST * 2));
    }

    #[test]
    fn mouse_buttons_reach_the_machine_by_name() {
        assert_eq!(machine_button(mouse::Button::Left), Button::Left);
        assert_eq!(machine_button(mouse::Button::Middle), Button::Middle);
        assert_eq!(machine_button(mouse::Button::Right), Button::Right);
        assert_eq!(machine_button(mouse::Button::Back), Button::Other);
        assert_eq!(machine_button(mouse::Button::Other(9)), Button::Other);
    }

    #[test]
    fn a_drag_ends_when_the_widget_state_is_rebuilt_under_it() {
        use crate::machine::{Machine, View};
        let mut machine = Machine::default();
        let viewport = Viewport::default();
        let board = wren_board::Board::default();
        let scene = wren_board::scene::Scene {
            bounds: wren_board::scene::Rect::default(),
            items: Vec::new(),
        };
        let view = View {
            viewport,
            now: Instant::now(),
            board: &board,
            scene: &scene,
            writable: true,
        };
        let mut state = State::new();
        state.pressed = Some(mouse::Button::Middle);
        machine.step(
            Input::Press {
                button: Button::Middle,
                at: (100.0, 100.0),
            },
            &view,
        );
        assert!(machine.panning());
        // The palette opens over the board: a new tree, a new state.
        let mut state = State::new();
        let release = released(
            &mut state.pressed,
            mouse::Button::Middle,
            machine.dragging(),
        );
        assert_eq!(
            release,
            Some(Input::Release {
                button: Button::Middle
            })
        );
        machine.step(release.unwrap(), &view);
        assert!(!machine.panning());
        // A plain left drag after it does not pan.
        state.pressed = Some(mouse::Button::Left);
        for input in [
            Input::Press {
                button: Button::Left,
                at: (600.0, 400.0),
            },
            Input::Move { at: (650.0, 400.0) },
        ] {
            assert!(machine.step(input, &view).is_empty());
        }
        let release = released(&mut state.pressed, mouse::Button::Left, machine.dragging());
        assert_eq!(
            release,
            Some(Input::Release {
                button: Button::Left
            })
        );
        assert_eq!(state.pressed, None);
        // With no drag in the machine, a release the widget did not press
        // is not the board's; nor is another button's during its own drag.
        assert_eq!(released(&mut None, mouse::Button::Right, false), None);
        let mut held = Some(mouse::Button::Middle);
        assert_eq!(released(&mut held, mouse::Button::Left, true), None);
        assert_eq!(held, Some(mouse::Button::Middle));
    }

    #[test]
    fn the_last_move_places_the_pointer_and_an_overlay_hides_it() {
        let bounds = Rectangle::new(Point::new(100.0, 50.0), Size::new(400.0, 300.0));
        let mut state = State::new();
        let cursor = mouse::Cursor::Available(Point::new(300.0, 200.0));
        assert_eq!(
            pointer(&state, cursor, bounds),
            Some(Point::new(300.0, 200.0))
        );
        // A press batched with the move after it: the move before counts.
        state.pointer = Some(Point::new(120.0, 60.0));
        assert_eq!(
            pointer(&state, cursor, bounds),
            Some(Point::new(120.0, 60.0))
        );
        // A last move outside the widget is not where the pointer is now.
        state.pointer = Some(Point::new(10.0, 10.0));
        assert_eq!(
            pointer(&state, cursor, bounds),
            Some(Point::new(300.0, 200.0))
        );
        // Outside, or under an overlay, there is no pointer for the board.
        let outside = mouse::Cursor::Available(Point::new(20.0, 20.0));
        assert_eq!(pointer(&state, outside, bounds), None);
        let levitating = mouse::Cursor::Levitating(Point::new(300.0, 200.0));
        assert_eq!(pointer(&state, levitating, bounds), None);
    }

    use std::sync::Arc;

    use iced::keyboard::key::{Code, Physical};

    use crate::fixtures;
    use crate::keys::KeyRoute;
    use crate::machine::{Key as MachineKey, Mods, State as Surface};
    use crate::Command;

    /// The widget's place in the window.
    const BOUNDS: Rectangle = Rectangle {
        x: 100.0,
        y: 50.0,
        width: 800.0,
        height: 600.0,
    };

    fn open(text: &str) -> BoardView {
        fixtures::load_fonts();
        BoardView::open(text, fixtures::palette(), fixtures::FONT).expect("a board")
    }

    /// What `event` sends with the pointer at `at` in the window, the view
    /// then taking each message as the host would.
    fn send(view: &mut BoardView, state: &mut State, event: Event, at: Point) -> Sent {
        let cursor = mouse::Cursor::Available(at);
        let sent = translate(state, &event, BOUNDS, cursor, view, Style::default());
        for message in &sent.messages {
            view.update(message.clone(), Instant::now());
        }
        sent
    }

    fn inputs(sent: &Sent) -> Vec<Input> {
        sent.messages
            .iter()
            .filter_map(|message| match message {
                Message::Input(_, input) => Some(*input),
                _ => None,
            })
            .collect()
    }

    fn mouse(event: mouse::Event) -> Event {
        Event::Mouse(event)
    }

    fn key(key: Key, modifiers: keyboard::Modifiers, text: Option<&str>) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Code(Code::KeyA),
            location: keyboard::Location::Standard,
            modifiers,
            text: text.map(Into::into),
            repeat: false,
        })
    }

    /// `a` of the two-node board, in the window.
    const ON_A: Point = Point::new(160.0, 80.0);
    const AWAY: Point = Point::new(20.0, 20.0);

    #[test]
    fn a_left_drag_is_followed_outside_the_widget_until_its_release() {
        let mut view = open(fixtures::TWO);
        let mut state = State::new();
        let press = mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let sent = send(&mut view, &mut state, press, ON_A);
        assert!(sent.captured);
        // The press, in widget-local pixels, then the focus it gave.
        assert_eq!(
            inputs(&sent),
            [
                Input::Press {
                    button: Button::Left,
                    at: (60.0, 30.0)
                },
                Input::Focus(true)
            ]
        );
        assert!(state.focused && view.machine().focused());
        // Out past the widget's corner, the pointer is still the drag's.
        let moved = mouse(mouse::Event::CursorMoved {
            position: Point::new(40.0, 20.0),
        });
        let sent = send(&mut view, &mut state, moved, AWAY);
        assert!(sent.captured);
        assert_eq!(inputs(&sent), [Input::Move { at: (-60.0, -30.0) }]);
        assert_eq!(view.machine().state(), Surface::Moving);
        let release = mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let sent = send(&mut view, &mut state, release, AWAY);
        assert!(sent.captured);
        assert_eq!(
            inputs(&sent),
            [Input::Release {
                button: Button::Left
            }]
        );
        assert_eq!(view.machine().state(), Surface::Idle);
        assert_eq!(
            view.board().nodes.values().next().map(|node| node.at),
            Some([-120, -60])
        );
        // With no button held a move is nobody's business.
        let moved = mouse(mouse::Event::CursorMoved { position: ON_A });
        assert_eq!(send(&mut view, &mut state, moved, ON_A), Sent::default());
    }

    #[test]
    fn a_press_carries_the_modifiers_held_at_it() {
        let mut view = open(fixtures::TWO);
        let mut state = State::new();
        // Shift goes down while the board does not have the keyboard.
        let shift = Event::Keyboard(keyboard::Event::ModifiersChanged(
            keyboard::Modifiers::SHIFT,
        ));
        assert_eq!(send(&mut view, &mut state, shift, AWAY), Sent::default());
        let press = mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let sent = send(&mut view, &mut state, press, ON_A);
        let shifted = Mods {
            shift: true,
            duplicate: false,
        };
        assert_eq!(inputs(&sent)[0], Input::Modifiers(shifted));
        assert_eq!(view.machine().mods(), shifted);
        // Focused, a change is sent at once, and the same modifiers again
        // are not.
        let release = mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        send(&mut view, &mut state, release, ON_A);
        let shift = Event::Keyboard(keyboard::Event::ModifiersChanged(
            keyboard::Modifiers::SHIFT,
        ));
        assert_eq!(send(&mut view, &mut state, shift, ON_A), Sent::default());
        let none = Event::Keyboard(keyboard::Event::ModifiersChanged(
            keyboard::Modifiers::default(),
        ));
        let sent = send(&mut view, &mut state, none, ON_A);
        assert_eq!(inputs(&sent), [Input::Modifiers(Mods::default())]);
        assert!(!sent.captured);
    }

    #[test]
    fn the_pointer_leaving_or_the_window_blurring_cancels_what_the_board_holds() {
        let mut view = open(fixtures::TWO);
        let mut state = State::new();
        let left = mouse(mouse::Event::CursorLeft);
        let blur = Event::Window(window::Event::Unfocused);
        // Holding nothing, neither sends anything.
        for event in [left.clone(), blur.clone()] {
            assert_eq!(send(&mut view, &mut state, event, AWAY), Sent::default());
        }
        // Space held on the focused board.
        state.focused = true;
        send(
            &mut view,
            &mut state,
            mouse(mouse::Event::CursorMoved { position: ON_A }),
            ON_A,
        );
        let space = key(
            Key::Named(Named::Space),
            keyboard::Modifiers::default(),
            Some(" "),
        );
        assert_eq!(
            inputs(&send(&mut view, &mut state, space, ON_A)),
            [Input::Space { held: true }]
        );
        assert!(view.machine().space());
        assert_eq!(
            inputs(&send(&mut view, &mut state, left, AWAY)),
            [Input::Cancel]
        );
        assert!(!view.machine().space());
        // A drag in flight, when the window goes.
        let press = mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        send(&mut view, &mut state, press, ON_A);
        let moved = mouse(mouse::Event::CursorMoved {
            position: Point::new(260.0, 80.0),
        });
        send(&mut view, &mut state, moved, ON_A);
        assert!(view.machine().dragging());
        // The pointer leaving with the button down is not the end of it.
        assert_eq!(
            send(&mut view, &mut state, mouse(mouse::Event::CursorLeft), AWAY),
            Sent::default()
        );
        assert_eq!(
            inputs(&send(&mut view, &mut state, blur, AWAY)),
            [Input::Cancel]
        );
        assert!(!view.machine().dragging());
        assert_eq!(view.shown(), view.scene());
    }

    #[test]
    fn keys_reach_only_a_focused_board_and_its_policy_comes_first() {
        let mut view = open(fixtures::TWO);
        view.set_key_policy(Arc::new(|key, _, modifiers, _| {
            if modifiers.alt() {
                KeyRoute::Host
            } else if *key == Key::Character("d".into()) {
                KeyRoute::Command(Command::Delete)
            } else {
                KeyRoute::Pass
            }
        }));
        let mut state = State::new();
        let plain = keyboard::Modifiers::default();
        let d = key(Key::Character("d".into()), plain, Some("d"));
        assert_eq!(
            send(&mut view, &mut state, d.clone(), AWAY),
            Sent::default()
        );

        state.focus();
        let sent = send(&mut view, &mut state, d, AWAY);
        assert!(sent.captured);
        assert_eq!(
            sent.messages,
            [
                Message::Command(Command::Delete),
                Message::Input(view.epoch(), Input::Focus(true))
            ]
        );
        // The host's key is left to it; an unbound one is taken.
        let host = key(Key::Character("k".into()), keyboard::Modifiers::ALT, None);
        assert_eq!(send(&mut view, &mut state, host, AWAY), Sent::default());
        let tab = key(Key::Named(Named::Tab), plain, Some("\t"));
        let sent = send(&mut view, &mut state, tab, AWAY);
        assert!(sent.captured && sent.messages.is_empty());
        let arrow = key(
            Key::Named(Named::ArrowLeft),
            keyboard::Modifiers::SHIFT,
            None,
        );
        let sent = send(&mut view, &mut state, arrow, AWAY);
        assert!(sent.captured);
        assert_eq!(
            inputs(&sent),
            [Input::Key {
                key: MachineKey::Arrow(wren_board::Dir::Left),
                repeat: false
            }]
        );
    }

    #[test]
    fn escape_is_taken_while_the_ladder_has_a_rung_and_left_to_the_app_after() {
        let mut view = open(fixtures::TWO);
        let mut state = State::new();
        let press = mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        send(&mut view, &mut state, press, ON_A);
        let release = mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        send(&mut view, &mut state, release, ON_A);
        assert_eq!(view.machine().selection().len(), 1);
        let escape = key(
            Key::Named(Named::Escape),
            keyboard::Modifiers::default(),
            None,
        );
        let sent = send(&mut view, &mut state, escape.clone(), ON_A);
        assert!(sent.captured);
        assert!(view.machine().selection().is_empty());
        // Nothing left to clear: the app's Escape keeps the board focused.
        assert_eq!(send(&mut view, &mut state, escape, ON_A), Sent::default());
        assert!(state.focused);
    }

    #[test]
    fn losing_the_focus_reaches_the_machine_with_the_next_event() {
        let mut view = open(fixtures::TWO);
        let mut state = State::new();
        // Focused by an operation: told at the next event of any kind.
        state.focus();
        let redraw = Event::Window(window::Event::RedrawRequested(Instant::now()));
        let sent = send(&mut view, &mut state, redraw.clone(), AWAY);
        assert_eq!(inputs(&sent), [Input::Focus(true)]);
        assert!(!sent.captured);
        assert_eq!(
            send(&mut view, &mut state, redraw.clone(), AWAY),
            Sent::default()
        );
        // Arm a tool, then lose the keyboard to another widget.
        view.command(Command::Tool(crate::Tool::Hand), Instant::now());
        state.unfocus();
        let sent = send(&mut view, &mut state, redraw, AWAY);
        assert_eq!(inputs(&sent), [Input::Focus(false)]);
        assert_eq!(view.machine().state(), Surface::Unfocused);
        assert_eq!(view.machine().tool(), crate::Tool::Select);
        // A press elsewhere in the window is the board losing it too.
        state.focus();
        send(
            &mut view,
            &mut state,
            mouse(mouse::Event::CursorMoved { position: AWAY }),
            AWAY,
        );
        assert!(view.machine().focused());
        let press = mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let sent = send(&mut view, &mut state, press, AWAY);
        assert!(!sent.captured);
        assert_eq!(inputs(&sent), [Input::Focus(false)]);
        assert!(!state.focused);
    }

    #[test]
    fn the_widget_state_takes_and_gives_up_focus() {
        let mut state = State::new();
        assert!(!state.is_focused());
        state.focus();
        assert!(state.is_focused());
        state.unfocus();
        assert!(!state.is_focused());
    }
}
