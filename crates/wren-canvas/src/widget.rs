//! The spike's widget: a custom advanced widget, not a `canvas::Program`,
//! which cannot reach focus, the input method or renderer layers.
//!
//! It paints the view's scene through iced's geometry API, one renderer
//! layer per layer of the plan, and translates wheel, middle-button and
//! key events into [`Message`]s. Everything it shows lives in the host's
//! [`View`]; its own state is focus, the pan in flight and geometry.

use std::cell::RefCell;
use std::time::Instant;

use iced::advanced::graphics::geometry::Renderer as _;
use iced::advanced::renderer::Headless as _;
use iced::advanced::widget::{self, operation, tree, Tree};
use iced::advanced::{
    input_method, layout, mouse, renderer, Clipboard, InputMethod, Layout, Renderer as _, Shell,
    Widget,
};
use iced::keyboard::{self, key::Named, Key};
use iced::widget::canvas;
use iced::{
    window, Element, Event, Length, Pixels, Point, Rectangle, Renderer, Size, Theme,
    Transformation, Vector,
};
use wren_board::scene::{Draw, Item, Rect};

use crate::paint::{self, Space};
use crate::spike::{Edit, Labels, Message, Scroll, Strategy, View};
use crate::Viewport;

pub(crate) struct Canvas<'a> {
    view: &'a View,
    id: widget::Id,
}

impl<'a> Canvas<'a> {
    pub(crate) fn new(view: &'a View, id: &'static str) -> Self {
        Self {
            view,
            id: widget::Id::new(id),
        }
    }

    fn input_method<'b>(&'b self, state: &State, bounds: Rectangle) -> InputMethod<&'b str> {
        let (true, true, Some(label)) = (state.focused, state.window_focused, self.view.label())
        else {
            return InputMethod::Disabled;
        };
        let Some((caret, size)) = self.view.caret_on_screen() else {
            return InputMethod::Disabled;
        };
        InputMethod::Enabled {
            cursor: Rectangle::new(
                Point::new(bounds.x + caret.x, bounds.y + caret.y),
                Size::new(caret.width, caret.height),
            ),
            purpose: input_method::Purpose::Normal,
            preedit: label.preedit.as_ref().map(|preedit| input_method::Preedit {
                content: preedit.content.as_str(),
                selection: preedit.selection.clone(),
                text_size: Some(Pixels(size)),
            }),
        }
    }
}

struct State {
    focused: bool,
    window_focused: bool,
    /// The time of the last redraw, which decides whether the view rests.
    now: Instant,
    modifiers: keyboard::Modifiers,
    /// Where the pointer was at the last event that moved it. `update` is
    /// handed the pointer as it is after a whole batch of events, so a
    /// press that arrives with the move that follows it has to be placed
    /// from here.
    pointer: Option<Point>,
    /// The pointer position a middle-button pan last moved from.
    pan: Option<Point>,
    geometry: RefCell<Geometry>,
}

/// Cached geometry, one cache per layer and per strategy.
#[derive(Default)]
struct Geometry {
    revision: Option<u64>,
    board: Vec<canvas::Cache>,
    screen: Vec<canvas::Cache>,
    /// What the screen caches were built for: the viewport, the widget's
    /// size and whether labels were drawn.
    at: Option<(Viewport, Size, bool)>,
}

impl Geometry {
    fn sync(&mut self, revision: u64, layers: usize) {
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.board.clear();
            self.screen.clear();
            self.at = None;
        }
        self.board.resize_with(layers, canvas::Cache::new);
        self.screen.resize_with(layers, canvas::Cache::new);
    }
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
        self.pan = None;
    }
}

/// What a key or input-method event is to a focused canvas.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Route {
    /// The canvas's: captured and published.
    Message(Message),
    /// The canvas's, with nothing to do: captured.
    Swallow,
    /// Not the canvas's: left for the app.
    Pass,
}

/// Route an event for a focused canvas. `editing` is whether a label is
/// open, `composing` whether the input method holds preedit text.
pub(crate) fn route(event: &Event, editing: bool, composing: bool) -> Route {
    match event {
        Event::InputMethod(event) if editing => match event {
            input_method::Event::Opened | input_method::Event::Closed => {
                Route::Message(Message::Edit(Edit::Preedit {
                    content: String::new(),
                    selection: None,
                }))
            }
            input_method::Event::Preedit(content, selection) => {
                Route::Message(Message::Edit(Edit::Preedit {
                    content: content.clone(),
                    selection: selection.clone(),
                }))
            }
            input_method::Event::Commit(text) => {
                Route::Message(Message::Edit(Edit::Commit(text.clone())))
            }
        },
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            text,
            ..
        }) => {
            // While a composition is open every key is the input method's.
            if editing && composing {
                return Route::Swallow;
            }
            if modifiers.control() || modifiers.alt() || modifiers.logo() {
                return Route::Pass;
            }
            let arrow = matches!(
                key,
                Key::Named(
                    Named::ArrowUp | Named::ArrowDown | Named::ArrowLeft | Named::ArrowRight
                )
            );
            if !editing {
                return match key {
                    Key::Named(Named::Enter) => Route::Message(Message::Edit(Edit::Open)),
                    _ if arrow => Route::Message(Message::Arrow),
                    _ => Route::Pass,
                };
            }
            match key {
                Key::Named(Named::Escape) => Route::Message(Message::Edit(Edit::Close)),
                Key::Named(Named::Enter) => {
                    Route::Message(Message::Edit(Edit::Insert("\n".into())))
                }
                Key::Named(Named::Backspace) => Route::Message(Message::Edit(Edit::Backspace)),
                Key::Named(Named::ArrowLeft) => Route::Message(Message::Edit(Edit::Left)),
                Key::Named(Named::ArrowRight) => Route::Message(Message::Edit(Edit::Right)),
                Key::Named(Named::Tab) => Route::Swallow,
                _ if arrow => Route::Swallow,
                _ => match text {
                    Some(text) if !text.chars().any(char::is_control) => {
                        Route::Message(Message::Edit(Edit::Insert(text.to_string())))
                    }
                    _ => Route::Pass,
                },
            }
        }
        _ => Route::Pass,
    }
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

impl Widget<Message, Theme, Renderer> for Canvas<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            focused: false,
            window_focused: true,
            now: Instant::now(),
            modifiers: keyboard::Modifiers::default(),
            pointer: None,
            pan: None,
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
        let bounds = layout.bounds();
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                state.now = *now;
                // Both draw differently once the view has come to rest.
                let rests = self.view.strategy() == Strategy::Settle
                    || labels(self.view, renderer).moving_lines != usize::MAX;
                if rests && self.view.moving(*now) {
                    if let Some(rest) = self.view.settles_at() {
                        shell.request_redraw_at(rest);
                    }
                }
                let input_method = self.input_method(state, bounds);
                #[cfg(feature = "test-support")]
                self.view.probe.ime.set(match &input_method {
                    InputMethod::Enabled {
                        cursor, preedit, ..
                    } => Some((
                        [cursor.x, cursor.y, cursor.width, cursor.height],
                        preedit
                            .as_ref()
                            .and_then(|preedit| preedit.text_size)
                            .map_or(0.0, |size| size.0),
                    )),
                    InputMethod::Disabled => None,
                });
                shell.request_input_method(&input_method);
            }
            Event::Window(window::Event::Unfocused) => state.window_focused = false,
            Event::Window(window::Event::Focused) => state.window_focused = true,
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
            }
            // The wheel reaches an unfocused canvas under the pointer.
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if let Some(at) = pointer(state, cursor, bounds) {
                    let delta = match *delta {
                        mouse::ScrollDelta::Lines { x, y } => Scroll::Lines { x, y },
                        mouse::ScrollDelta::Pixels { x, y } => Scroll::Pixels { x, y },
                    };
                    #[cfg(feature = "test-support")]
                    self.view.probe.wheel.borrow_mut().push(delta);
                    shell.publish(Message::Wheel {
                        delta,
                        at: (at.x - bounds.x, at.y - bounds.y),
                        zoom: state.modifiers.command(),
                        sideways: state.modifiers.shift(),
                    });
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                match pointer(state, cursor, bounds) {
                    Some(position) => {
                        state.focused = true;
                        if *button == mouse::Button::Middle {
                            state.pan = Some(position);
                        }
                        shell.capture_event();
                    }
                    None => operation::Focusable::unfocus(state),
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                state.pointer = Some(*position);
                if let Some(last) = state.pan {
                    state.pan = Some(*position);
                    shell.publish(Message::Drag {
                        dx: position.x - last.x,
                        dy: position.y - last.y,
                    });
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)) => {
                state.pan = None;
            }
            // iced hands key events to every widget; only a focused canvas
            // acts on them.
            Event::Keyboard(_) | Event::InputMethod(_) if state.focused => {
                let label = self.view.label();
                let composing = label.is_some_and(|label| label.preedit.is_some());
                match route(event, label.is_some(), composing) {
                    Route::Message(message) => {
                        shell.publish(message);
                        shell.capture_event();
                    }
                    Route::Swallow => shell.capture_event(),
                    Route::Pass => {}
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
        let view = self.view;
        let viewport = view.viewport();
        let (scene, plan) = (view.scene(), view.plan());

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            paint::color(view.palette().canvas),
        );

        let exact = match view.strategy() {
            Strategy::Transform => false,
            Strategy::Rebuild => true,
            Strategy::Settle => !view.moving(state.now),
        };
        let mut geometry = state.geometry.borrow_mut();
        geometry.sync(view.revision(), plan.count);
        let visible = Rect {
            x: viewport.x,
            y: viewport.y,
            width: bounds.width / viewport.zoom,
            height: bounds.height / viewport.zoom,
        };
        let labels = labels(view, renderer);
        let shown = |item: &Item| match &item.draw {
            Draw::Text { size, .. } => {
                size * viewport.zoom >= labels.min_px && intersects(item.bounds, visible)
            }
            Draw::Path { .. } => false,
        };
        let labelled = !exact
            || labels.moving_lines == usize::MAX
            || !view.moving(state.now)
            || scene
                .items
                .iter()
                .filter(|item| shown(item))
                .map(|item| match &item.draw {
                    Draw::Text { lines, .. } => lines.len(),
                    Draw::Path { .. } => 0,
                })
                .try_fold(0usize, |total, lines| {
                    Some(total + lines).filter(|total| *total <= labels.moving_lines)
                })
                .is_some();
        if exact && geometry.at != Some((viewport, bounds.size(), labelled)) {
            geometry.at = Some((viewport, bounds.size(), labelled));
            geometry.screen.iter().for_each(canvas::Cache::clear);
        }
        let paint = |frame: &mut canvas::Frame, layer: usize, space: Space| {
            #[cfg(feature = "test-support")]
            let mut painted = 0;
            for item in plan.items(layer).filter_map(|index| scene.items.get(index)) {
                if space != Space::Board {
                    let seen = match &item.draw {
                        Draw::Text { .. } => labelled && shown(item),
                        Draw::Path { .. } => intersects(item.bounds, visible),
                    };
                    if !seen {
                        continue;
                    }
                }
                paint::item(frame, item, space, view.font(), line_height(view, item));
                #[cfg(feature = "test-support")]
                {
                    painted += 1;
                }
            }
            #[cfg(feature = "test-support")]
            {
                view.probe.builds.set(view.probe.builds.get() + 1);
                view.probe.painted.set(view.probe.painted.get() + painted);
            }
        };

        for layer in 0..plan.count {
            if exact {
                let drawn = geometry.screen[layer].draw(renderer, bounds.size(), |frame| {
                    paint(frame, layer, Space::Screen(viewport));
                });
                renderer.with_layer(bounds, |renderer| {
                    renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                        renderer.draw_geometry(drawn);
                    });
                });
            } else {
                let whole = Rectangle::new(
                    Point::new(scene.bounds.x - 1.0, scene.bounds.y - 1.0),
                    Size::new(scene.bounds.width + 2.0, scene.bounds.height + 2.0),
                );
                let drawn = geometry.board[layer].draw_with_bounds(renderer, whole, |frame| {
                    paint(frame, layer, Space::Board);
                });
                let (x, y) = viewport.to_screen((0.0, 0.0));
                let place = Transformation::translate(bounds.x + x, bounds.y + y)
                    * Transformation::scale(viewport.zoom);
                renderer.with_layer(bounds, |renderer| {
                    renderer.with_transformation(place, |renderer| {
                        renderer.draw_geometry(drawn);
                    });
                });
            }
        }

        if let Some((caret, _)) = view.caret_on_screen().filter(|_| state.focused) {
            renderer.with_layer(bounds, |renderer| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle::new(
                            Point::new(bounds.x + caret.x, bounds.y + caret.y),
                            Size::new(caret.width.max(1.5), caret.height),
                        ),
                        ..renderer::Quad::default()
                    },
                    paint::color(view.palette().accent),
                );
            });
        }

        #[cfg(feature = "test-support")]
        {
            view.probe.layers.set(plan.count);
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

/// Where the pointer was when the event being handled happened, if the
/// canvas has it: `cursor` is the pointer after the whole batch, and is
/// unavailable while an overlay is in front.
fn pointer(state: &State, cursor: mouse::Cursor, bounds: Rectangle) -> Option<Point> {
    let end = cursor.position_over(bounds)?;
    Some(
        state
            .pointer
            .filter(|pointer| bounds.contains(*pointer))
            .unwrap_or(end),
    )
}

/// The view's label rule, or the renderer's when the view names none:
/// only the software renderer needs to drop labels.
fn labels(view: &View, renderer: &Renderer) -> Labels {
    view.labels().unwrap_or_else(|| {
        if renderer.name() == "tiny-skia" {
            Labels::SOFTWARE
        } else {
            Labels::ALWAYS
        }
    })
}

fn line_height(view: &View, item: &Item) -> f32 {
    match &item.draw {
        Draw::Text { face, size, .. } => view.line_height(*face, *size),
        Draw::Path { .. } => 0.0,
    }
}

impl<'a> From<Canvas<'a>> for Element<'a, Message> {
    fn from(canvas: Canvas<'a>) -> Self {
        Element::new(canvas)
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::{key::Physical, Location, Modifiers};

    use super::*;

    fn press(key: Key, modifiers: Modifiers, text: Option<&str>) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: text.map(Into::into),
            repeat: false,
        })
    }

    fn named(key: Named) -> Event {
        press(Key::Named(key), Modifiers::default(), None)
    }

    fn typed(text: &str) -> Event {
        press(
            Key::Character(text.into()),
            Modifiers::default(),
            Some(text),
        )
    }

    fn edit(edit: Edit) -> Route {
        Route::Message(Message::Edit(edit))
    }

    #[test]
    fn an_idle_canvas_captures_arrows_and_leaves_escape_and_shortcuts() {
        for arrow in [
            Named::ArrowUp,
            Named::ArrowDown,
            Named::ArrowLeft,
            Named::ArrowRight,
        ] {
            assert_eq!(
                route(&named(arrow), false, false),
                Route::Message(Message::Arrow)
            );
        }
        assert_eq!(route(&named(Named::Enter), false, false), edit(Edit::Open));
        assert_eq!(route(&named(Named::Escape), false, false), Route::Pass);
        assert_eq!(route(&typed("r"), false, false), Route::Pass);
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            let shortcut = press(Key::Character("k".into()), modifiers, Some("k"));
            assert_eq!(route(&shortcut, false, false), Route::Pass);
            assert_eq!(route(&shortcut, true, false), Route::Pass);
            let arrow = press(Key::Named(Named::ArrowUp), modifiers, None);
            assert_eq!(route(&arrow, false, false), Route::Pass);
        }
        // With no label open the input method is not the canvas's.
        let commit = Event::InputMethod(input_method::Event::Commit("x".into()));
        assert_eq!(route(&commit, false, false), Route::Pass);
    }

    #[test]
    fn an_open_label_takes_text_and_editing_keys() {
        assert_eq!(
            route(&typed("a"), true, false),
            edit(Edit::Insert("a".into()))
        );
        assert_eq!(
            route(
                &press(Key::Character("A".into()), Modifiers::SHIFT, Some("A")),
                true,
                false
            ),
            edit(Edit::Insert("A".into()))
        );
        assert_eq!(
            route(&named(Named::Enter), true, false),
            edit(Edit::Insert("\n".into()))
        );
        assert_eq!(
            route(&named(Named::Backspace), true, false),
            edit(Edit::Backspace)
        );
        assert_eq!(
            route(&named(Named::ArrowLeft), true, false),
            edit(Edit::Left)
        );
        assert_eq!(
            route(&named(Named::ArrowRight), true, false),
            edit(Edit::Right)
        );
        // Escape commits; it is captured, so it never leaves the canvas.
        assert_eq!(route(&named(Named::Escape), true, false), edit(Edit::Close));
        assert_eq!(route(&named(Named::ArrowUp), true, false), Route::Swallow);
        assert_eq!(route(&named(Named::Tab), true, false), Route::Swallow);
    }

    #[test]
    fn a_composition_owns_every_key_until_it_commits() {
        let preedit = Event::InputMethod(input_method::Event::Preedit("にほ".into(), Some(0..6)));
        assert_eq!(
            route(&preedit, true, false),
            edit(Edit::Preedit {
                content: "にほ".into(),
                selection: Some(0..6)
            })
        );
        for key in [
            Named::Enter,
            Named::Escape,
            Named::Backspace,
            Named::Tab,
            Named::ArrowLeft,
        ] {
            assert_eq!(route(&named(key), true, true), Route::Swallow, "{key:?}");
        }
        assert_eq!(route(&typed("n"), true, true), Route::Swallow);
        let commit = Event::InputMethod(input_method::Event::Commit("日本".into()));
        assert_eq!(
            route(&commit, true, true),
            edit(Edit::Commit("日本".into()))
        );
        let cleared = edit(Edit::Preedit {
            content: String::new(),
            selection: None,
        });
        assert_eq!(
            route(&Event::InputMethod(input_method::Event::Closed), true, true),
            cleared
        );
        assert_eq!(
            route(
                &Event::InputMethod(input_method::Event::Preedit(String::new(), None)),
                true,
                true
            ),
            cleared
        );
    }
}
