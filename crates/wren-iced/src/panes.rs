//! Preferred widths are workspace state; compressed widths are only layout output.
//! A grip keeps its child tree intact and owns its pointer sequence before delegation.

use std::rc::Rc;
use std::time::{Duration, Instant};

use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{tree, Id, Operation, Tree};
use iced::advanced::{
    layout, mouse, overlay, renderer, text, Clipboard, Layout as WidgetLayout, Shell, Widget,
};
use iced::{keyboard, window, Color, Element, Event, Font, Length, Point, Rectangle, Size, Vector};
use wren_core::theme::{Rgb, Theme};
use wren_core::workspace_state::WorkspaceState;

pub const EDITOR_MIN: f32 = 320.0;
pub const COMPRESSED_MIN: f32 = 120.0;
pub const GRIP_WIDTH: f32 = 7.0;
pub const KEYBOARD_STEP: f32 = 16.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Layout {
    #[default]
    Columns,
    Outline,
}

impl Layout {
    pub fn from_state(state: &WorkspaceState) -> Self {
        match state.layout() {
            "outline" => Self::Outline,
            _ => Self::Columns,
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::Outline => "outline",
        }
    }

    pub const fn panes(self) -> &'static [Pane] {
        match self {
            Self::Columns => &[Pane::Sections, Pane::Groups, Pane::Notes],
            Self::Outline => &[Pane::Outline, Pane::Notes],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pane {
    Sections,
    Groups,
    Notes,
    Outline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

impl Pane {
    pub const fn limits(self) -> Limits {
        let (default, min, max) = match self {
            Self::Sections => (200.0, 140.0, 360.0),
            Self::Groups => (220.0, 150.0, 400.0),
            Self::Notes => (280.0, 200.0, 480.0),
            Self::Outline => (260.0, 180.0, 420.0),
        };
        Limits { default, min, max }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::Sections | Self::Outline => "sections",
            Self::Groups => "groups",
            Self::Notes => "notes",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Sections => "Sections",
            Self::Groups => "Groups",
            Self::Notes => "Notes",
            Self::Outline => "Sections & groups",
        }
    }

    pub fn clamp(self, width: f32) -> f32 {
        let limits = self.limits();
        if width.is_nan() {
            limits.default
        } else {
            width.clamp(limits.min, limits.max)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Widths {
    preferred: [f32; 5],
}

impl Default for Widths {
    fn default() -> Self {
        Self {
            preferred: [200.0, 220.0, 280.0, 260.0, 280.0],
        }
    }
}

fn index(layout: Layout, pane: Pane) -> usize {
    match (layout, pane) {
        (Layout::Columns, Pane::Sections) => 0,
        (Layout::Columns, Pane::Groups) => 1,
        (Layout::Columns, Pane::Notes) => 2,
        (Layout::Outline, Pane::Outline) => 3,
        (Layout::Outline, Pane::Notes) => 4,
        _ => panic!("pane does not belong to this layout"),
    }
}

impl Widths {
    pub fn from_state(state: &WorkspaceState) -> Self {
        let mut widths = Self::default();
        for layout in [Layout::Columns, Layout::Outline] {
            for &pane in layout.panes() {
                if let Some(width) = state.width(layout.key(), pane.key()) {
                    widths.set(layout, pane, f32::from(width));
                }
            }
        }
        widths
    }

    pub fn get(&self, layout: Layout, pane: Pane) -> f32 {
        self.preferred[index(layout, pane)]
    }

    pub fn set(&mut self, layout: Layout, pane: Pane, width: f32) -> f32 {
        let width = pane.clamp(width);
        self.preferred[index(layout, pane)] = width;
        width
    }

    #[cfg(test)]
    pub fn reset(&mut self, layout: Layout, pane: Pane) -> f32 {
        self.set(layout, pane, pane.limits().default)
    }

    #[cfg(test)]
    pub fn reset_layout(&mut self, layout: Layout) {
        for &pane in layout.panes() {
            self.reset(layout, pane);
        }
    }

    pub fn reset_all(&mut self) {
        *self = Self::default();
    }

    /// Updates metadata in memory only. Call this for Ended/Reset/Step, never Changed.
    #[cfg(test)]
    pub fn store(
        &self,
        state: &mut WorkspaceState,
        layout: Layout,
        pane: Pane,
    ) -> Result<(), String> {
        state.set_width(
            layout.key(),
            pane.key(),
            self.get(layout, pane).round() as u16,
        )
    }

    /// `available` is the actual horizontal browser+editor extent, excluding any outer chrome.
    /// Below the sum of the 120px floors, the row overflows rather than wrapping.
    pub fn effective(&self, layout: Layout, available: f32) -> EffectiveWidths {
        let panes = layout.panes();
        let mut widths: Vec<_> = panes.iter().map(|&pane| self.get(layout, pane)).collect();
        let available = if available.is_nan() {
            0.0
        } else {
            available.max(0.0)
        };
        let preferred_sum: f32 = widths.iter().sum();
        let budget = (available - EDITOR_MIN)
            .max(COMPRESSED_MIN * panes.len() as f32)
            .min(preferred_sum);
        let mut fixed = vec![false; widths.len()];
        loop {
            let fixed_sum: f32 = widths
                .iter()
                .zip(&fixed)
                .filter(|(_, fixed)| **fixed)
                .map(|(width, _)| *width)
                .sum();
            let free_sum: f32 = panes
                .iter()
                .zip(&fixed)
                .filter(|(_, fixed)| !**fixed)
                .map(|(&pane, _)| self.get(layout, pane))
                .sum();
            if free_sum == 0.0 {
                break;
            }
            let factor = ((budget - fixed_sum) / free_sum).clamp(0.0, 1.0);
            let mut hit_floor = false;
            for (i, &pane) in panes.iter().enumerate() {
                if !fixed[i] {
                    widths[i] = self.get(layout, pane) * factor;
                    if widths[i] < COMPRESSED_MIN {
                        widths[i] = COMPRESSED_MIN;
                        fixed[i] = true;
                        hit_floor = true;
                    }
                }
            }
            if !hit_floor {
                break;
            }
        }
        let editor = (available - widths.iter().sum::<f32>()).max(0.0);
        EffectiveWidths {
            layout,
            widths,
            editor,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveWidths {
    pub layout: Layout,
    widths: Vec<f32>,
    pub editor: f32,
}

impl EffectiveWidths {
    pub fn get(&self, pane: Pane) -> f32 {
        let position = self
            .layout
            .panes()
            .iter()
            .position(|&candidate| candidate == pane)
            .expect("pane belongs to layout");
        self.widths[position]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResizeEvent {
    Started {
        layout: Layout,
        pane: Pane,
    },
    Changed {
        layout: Layout,
        pane: Pane,
        width: f32,
    },
    Ended {
        layout: Layout,
        pane: Pane,
        width: f32,
    },
    Reset {
        layout: Layout,
        pane: Pane,
        width: f32,
    },
    Step {
        layout: Layout,
        pane: Pane,
        width: f32,
    },
    /// Escape restores the preferred width at press time and does not persist a new width.
    Cancelled {
        layout: Layout,
        pane: Pane,
        width: f32,
    },
    /// The application routes this to iced's focus_next/focus_previous operation.
    Tab {
        reverse: bool,
    },
}

pub fn grip_id(layout: Layout, pane: Pane) -> Id {
    Id::from(format!("wren-grip-{}-{}", layout.key(), pane.key()))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resize {
    pub layout: Layout,
    pub pane: Pane,
    pub origin: Point,
    pub original: f32,
    pub width: f32,
}

impl Resize {
    pub fn new(layout: Layout, pane: Pane, origin: Point, preferred: f32) -> Self {
        let width = pane.clamp(preferred);
        Self {
            layout,
            pane,
            origin,
            original: width,
            width,
        }
    }

    pub fn moved(&mut self, position: Point) -> Option<ResizeEvent> {
        let width = self.pane.clamp(self.original + position.x - self.origin.x);
        if width == self.width {
            return None;
        }
        self.width = width;
        Some(ResizeEvent::Changed {
            layout: self.layout,
            pane: self.pane,
            width,
        })
    }

    pub fn ended(self) -> ResizeEvent {
        ResizeEvent::Ended {
            layout: self.layout,
            pane: self.pane,
            width: self.width,
        }
    }

    pub fn cancelled(self) -> ResizeEvent {
        ResizeEvent::Cancelled {
            layout: self.layout,
            pane: self.pane,
            width: self.original,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    drag: Option<Resize>,
    last_cursor: Option<Point>,
    previous_press: Option<(Instant, Point, Layout, Pane)>,
    swallow_release: bool,
    focused: bool,
    external_width: Option<f32>,
    live_width: f32,
}

impl Focusable for State {
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

pub fn grip_bounds(bounds: Rectangle) -> Rectangle {
    Rectangle {
        x: bounds.x + bounds.width - GRIP_WIDTH / 2.0,
        y: bounds.y,
        width: GRIP_WIDTH,
        height: bounds.height,
    }
}

#[derive(Debug, PartialEq)]
enum Ownership {
    Delegate,
    Own(Vec<ResizeEvent>),
    Broadcast(Vec<ResizeEvent>),
}

struct Seam {
    bounds: Rectangle,
    layout: Layout,
    pane: Pane,
    preferred: f32,
}

fn owns_event(
    state: &mut State,
    event: &Event,
    cursor: mouse::Cursor,
    seam: Seam,
    now: Instant,
) -> Ownership {
    let Seam {
        bounds,
        layout,
        pane,
        preferred,
    } = seam;
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => state.last_cursor = Some(*position),
        Event::Mouse(mouse::Event::CursorLeft) => state.last_cursor = None,
        _ => {}
    }
    if state.external_width != Some(preferred) {
        state.external_width = Some(preferred);
        if state.drag.is_none() {
            state.live_width = pane.clamp(preferred);
        }
    }
    if state
        .drag
        .is_some_and(|drag| drag.layout != layout || drag.pane != pane)
    {
        let drag = state.drag.take().expect("checked live resize");
        state.swallow_release = true;
        return Ownership::Broadcast(vec![drag.cancelled()]);
    }
    let anchored = cursor
        .position()
        .map(|batch| state.last_cursor.unwrap_or(batch));
    match event {
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
            if state.drag.is_some() {
                return Ownership::Own(vec![]);
            }
            let Some(at) = anchored.filter(|&at| grip_bounds(bounds).contains(at)) else {
                state.focused = false;
                return Ownership::Delegate;
            };
            state.focused = true;
            let double =
                state
                    .previous_press
                    .is_some_and(|(then, position, old_layout, old_pane)| {
                        old_layout == layout
                            && old_pane == pane
                            && now.saturating_duration_since(then) <= Duration::from_millis(300)
                            && position.distance(at) < 6.0
                    });
            if double {
                state.previous_press = None;
                state.swallow_release = true;
                state.live_width = pane.limits().default;
                return Ownership::Own(vec![ResizeEvent::Reset {
                    layout,
                    pane,
                    width: state.live_width,
                }]);
            }
            state.swallow_release = false;
            state.previous_press = Some((now, at, layout, pane));
            state.drag = Some(Resize::new(layout, pane, at, state.live_width));
            Ownership::Own(vec![ResizeEvent::Started { layout, pane }])
        }
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            let Some(drag) = &mut state.drag else {
                return Ownership::Delegate;
            };
            if drag.origin.distance(*position) >= 4.0 {
                state.previous_press = None;
            }
            let changed = drag.moved(*position);
            state.live_width = drag.width;
            Ownership::Own(changed.into_iter().collect())
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            if let Some(drag) = state.drag.take() {
                return Ownership::Own(vec![drag.ended()]);
            }
            if std::mem::take(&mut state.swallow_release) {
                Ownership::Own(vec![])
            } else {
                Ownership::Delegate
            }
        }
        Event::Window(window::Event::Unfocused) => {
            state.focused = false;
            state.last_cursor = None;
            state.previous_press = None;
            state.drag.take().map_or(Ownership::Delegate, |drag| {
                Ownership::Broadcast(vec![drag.ended()])
            })
        }
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        }) if state.drag.is_some() => {
            let drag = state.drag.take().expect("checked live resize");
            state.live_width = drag.original;
            state.previous_press = None;
            state.swallow_release = true;
            Ownership::Own(vec![drag.cancelled()])
        }
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key),
            modifiers,
            ..
        }) if state.focused && state.drag.is_none() => {
            use keyboard::key::Named;
            match key {
                Named::ArrowLeft | Named::ArrowRight => {
                    let delta = if *key == Named::ArrowLeft {
                        -KEYBOARD_STEP
                    } else {
                        KEYBOARD_STEP
                    };
                    state.live_width = pane.clamp(state.live_width + delta);
                    Ownership::Own(vec![ResizeEvent::Step {
                        layout,
                        pane,
                        width: state.live_width,
                    }])
                }
                Named::Tab => Ownership::Own(vec![ResizeEvent::Tab {
                    reverse: modifiers.shift(),
                }]),
                _ => Ownership::Delegate,
            }
        }
        _ => Ownership::Delegate,
    }
}

/// Wrap the pane, not the scrollable inside it. No space is added for the overlapping grip.
/// On Started/Reset, focus `grip_id(layout, pane)` to unfocus other application widgets.
/// `drag_view::area` also routes these grips before delegating to any sibling editor.
pub fn resizable<'a, Message: Send + 'static, R>(
    content: impl Into<Element<'a, Message, iced::Theme, R>>,
    layout: Layout,
    pane: Pane,
    effective_width: f32,
    preferred_width: f32,
    theme: Theme,
    on_event: impl Fn(ResizeEvent) -> Message + 'static,
) -> Element<'a, Message, iced::Theme, R>
where
    R: text::Renderer<Font = Font> + 'a,
{
    let _ = index(layout, pane);
    Element::new(Resizable {
        content: content.into(),
        layout,
        pane,
        effective_width,
        preferred_width,
        theme,
        on_event: Rc::new(on_event),
    })
}

struct Resizable<'a, Message, R> {
    content: Element<'a, Message, iced::Theme, R>,
    layout: Layout,
    pane: Pane,
    effective_width: f32,
    preferred_width: f32,
    theme: Theme,
    on_event: Rc<dyn Fn(ResizeEvent) -> Message>,
}

struct GripState<Message> {
    gesture: State,
    layout: Layout,
    pane: Pane,
    preferred: f32,
    root_enabled: bool,
    on_event: Rc<dyn Fn(ResizeEvent) -> Message>,
}

pub(crate) struct GripInput<'a> {
    pub event: &'a Event,
    pub cursor: mouse::Cursor,
    pub enabled: bool,
}

/// A root must decide ownership before Row broadcasts an event to every sibling.
/// In particular iced's text editor does not ignore already-captured seam presses.
pub(crate) fn route_event<Message: Send + 'static, R>(
    content: &mut Element<'_, Message, iced::Theme, R>,
    tree: &mut Tree,
    layout: WidgetLayout<'_>,
    renderer: &R,
    input: GripInput<'_>,
    shell: &mut Shell<'_, Message>,
) -> bool
where
    R: text::Renderer<Font = Font>,
{
    struct Route<'a, Message> {
        event: &'a Event,
        cursor: mouse::Cursor,
        messages: Vec<Message>,
        owned: bool,
        enabled: bool,
    }
    impl<Message: Send + 'static> Operation for Route<'_, Message> {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            if !self.owned {
                operate(self);
            }
        }
        fn custom(&mut self, _id: Option<&Id>, bounds: Rectangle, state: &mut dyn std::any::Any) {
            if self.owned {
                return;
            }
            let Some(grip) = state.downcast_mut::<GripState<Message>>() else {
                return;
            };
            grip.root_enabled = self.enabled;
            if !self.enabled {
                grip.gesture.focused = false;
                grip.gesture.last_cursor = None;
                grip.gesture.previous_press = None;
                if let Some(drag) = grip.gesture.drag.take() {
                    grip.gesture.swallow_release = true;
                    self.messages.push((grip.on_event)(drag.ended()));
                }
                if matches!(
                    self.event,
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                ) && std::mem::take(&mut grip.gesture.swallow_release)
                {
                    self.owned = true;
                }
                return;
            }
            // File drags can hide motion from this grip; the root has the event-ordered cursor.
            grip.gesture.last_cursor = self.cursor.position();
            let ownership = owns_event(
                &mut grip.gesture,
                self.event,
                self.cursor,
                Seam {
                    bounds,
                    layout: grip.layout,
                    pane: grip.pane,
                    preferred: grip.preferred,
                },
                Instant::now(),
            );
            match ownership {
                Ownership::Delegate => {}
                Ownership::Own(events) => {
                    self.owned = true;
                    self.messages
                        .extend(events.into_iter().map(|event| (grip.on_event)(event)));
                }
                Ownership::Broadcast(events) => self
                    .messages
                    .extend(events.into_iter().map(|event| (grip.on_event)(event))),
            }
        }
    }
    let mut route = Route {
        event: input.event,
        cursor: input.cursor,
        messages: vec![],
        owned: false,
        enabled: input.enabled,
    };
    content
        .as_widget_mut()
        .operate(tree, layout, renderer, &mut route);
    if !route.messages.is_empty() || route.owned {
        shell.request_redraw();
    }
    for message in route.messages {
        shell.publish(message);
    }
    if route.owned {
        shell.capture_event();
    }
    route.owned
}

impl<Message: Send + 'static, R: text::Renderer<Font = Font>> Widget<Message, iced::Theme, R>
    for Resizable<'_, Message, R>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<GripState<Message>>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(GripState {
            gesture: State::default(),
            layout: self.layout,
            pane: self.pane,
            preferred: self.preferred_width,
            root_enabled: true,
            on_event: self.on_event.clone(),
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        let grip = tree.state.downcast_mut::<GripState<Message>>();
        grip.on_event = self.on_event.clone();
        if grip.gesture.drag.is_none() {
            grip.gesture.external_width = Some(self.preferred_width);
            grip.gesture.live_width = self.pane.clamp(self.preferred_width);
        }
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        Size::new(
            Length::Fixed(self.effective_width),
            self.content.as_widget().size().height,
        )
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &R, limits: &layout::Limits) -> layout::Node {
        let limits = layout::Limits::with_compression(
            Size::new(self.effective_width, limits.min().height),
            Size::new(self.effective_width, limits.max().height),
            Size::new(false, limits.compression().height),
        );
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &limits)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: WidgetLayout<'_>,
        renderer: &R,
        operation: &mut dyn Operation,
    ) {
        let grip = tree.state.downcast_mut::<GripState<Message>>();
        grip.layout = self.layout;
        grip.pane = self.pane;
        grip.preferred = self.preferred_width;
        grip.on_event = self.on_event.clone();
        operation.custom(
            Some(&grip_id(self.layout, self.pane)),
            layout.bounds(),
            grip,
        );
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
        operation.focusable(
            Some(&grip_id(self.layout, self.pane)),
            grip_bounds(layout.bounds()),
            &mut tree.state.downcast_mut::<GripState<Message>>().gesture,
        );
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: WidgetLayout<'_>,
        cursor: mouse::Cursor,
        renderer: &R,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if shell.is_event_captured() {
            return;
        }
        let grip = tree.state.downcast_mut::<GripState<Message>>();
        let ownership = if grip.root_enabled {
            owns_event(
                &mut grip.gesture,
                event,
                cursor,
                Seam {
                    bounds: layout.bounds(),
                    layout: self.layout,
                    pane: self.pane,
                    preferred: self.preferred_width,
                },
                Instant::now(),
            )
        } else {
            Ownership::Delegate
        };
        match ownership {
            Ownership::Own(events) => {
                for event in events {
                    shell.publish((self.on_event)(event));
                }
                shell.capture_event();
                shell.request_redraw();
                return;
            }
            Ownership::Broadcast(events) => {
                for event in events {
                    shell.publish((self.on_event)(event));
                }
                shell.request_redraw();
            }
            Ownership::Delegate => {}
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: WidgetLayout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &R,
    ) -> mouse::Interaction {
        if tree
            .state
            .downcast_ref::<GripState<Message>>()
            .gesture
            .drag
            .is_some()
            || cursor
                .position()
                .is_some_and(|at| grip_bounds(layout.bounds()).contains(at))
        {
            mouse::Interaction::ResizingHorizontally
        } else {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            )
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut R,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: WidgetLayout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        let state = &tree.state.downcast_ref::<GripState<Message>>().gesture;
        let bounds = layout.bounds();
        let active = state.drag.is_some();
        let hover = cursor
            .position()
            .is_some_and(|at| grip_bounds(bounds).contains(at));
        let accent = color(self.theme.colors.accent);
        let line = Rectangle {
            x: bounds.x + bounds.width - 0.5,
            y: bounds.y,
            width: if active || hover || state.focused {
                2.0
            } else {
                1.0
            },
            height: bounds.height,
        };
        renderer.fill_quad(
            renderer::Quad {
                bounds: line,
                ..renderer::Quad::default()
            },
            if active {
                accent.scale_alpha(0.6)
            } else if hover || state.focused {
                accent.scale_alpha(0.45)
            } else {
                color(self.theme.colors.divider)
            },
        );
        if let Some(drag) = state.drag {
            let label = format!("{} · {:.0} px", self.pane.label(), drag.width);
            let chip_width = (label.chars().count() as f32 * 6.8 + 16.0)
                .min(viewport.width)
                .min((bounds.width - 12.0).max(0.0));
            let x = (line.x - chip_width - 6.0).clamp(
                viewport.x,
                (viewport.x + viewport.width - chip_width).max(viewport.x),
            );
            let chip = Rectangle {
                x,
                y: bounds.y + 8.0,
                width: chip_width,
                height: 25.0,
            };
            renderer.fill_quad(
                renderer::Quad {
                    bounds: chip,
                    border: iced::Border {
                        radius: 5.0.into(),
                        ..iced::Border::default()
                    },
                    ..renderer::Quad::default()
                },
                accent,
            );
            let text_clip = chip
                .intersection(viewport)
                .unwrap_or(Rectangle::with_size(Size::ZERO));
            draw_text(
                renderer,
                label,
                Point::new(chip.x + 8.0, chip.y + 5.0),
                Size::new((chip.width - 16.0).max(0.0), 18.0),
                color(self.theme.colors.editor),
                &text_clip,
            );
        }
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: WidgetLayout<'a>,
        renderer: &R,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, iced::Theme, R>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

fn color(Rgb(r, g, b): Rgb) -> Color {
    Color::from_rgb8(r, g, b)
}

fn draw_text<R: text::Renderer<Font = Font>>(
    renderer: &mut R,
    content: String,
    position: Point,
    bounds: Size,
    color: Color,
    clip: &Rectangle,
) {
    renderer.fill_text(
        text::Text {
            content,
            bounds,
            size: 12.0.into(),
            line_height: text::LineHeight::Relative(1.2),
            font: Font::with_name("Inter"),
            align_x: text::Alignment::Left,
            align_y: iced::alignment::Vertical::Top,
            shaping: text::Shaping::Auto,
            wrapping: text::Wrapping::None,
        },
        position,
        color,
        *clip,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rectangle {
        Rectangle::new(Point::new(20.0, 40.0), Size::new(220.0, 300.0))
    }
    fn press() -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
    }
    fn release() -> Event {
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
    }
    fn moved(x: f32) -> Event {
        Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(x, 50.0),
        })
    }
    fn route(state: &mut State, event: &Event, batch_x: Option<f32>, now: Instant) -> Ownership {
        owns_event(
            state,
            event,
            batch_x.map_or(mouse::Cursor::Unavailable, |x| {
                mouse::Cursor::Available(Point::new(x, 50.0))
            }),
            Seam {
                bounds: bounds(),
                layout: Layout::Columns,
                pane: Pane::Groups,
                preferred: 220.0,
            },
            now,
        )
    }

    #[test]
    fn defaults_clamps_and_separate_layout_preferences() {
        let mut widths = Widths::default();
        assert_eq!(widths.get(Layout::Columns, Pane::Sections), 200.0);
        assert_eq!(widths.get(Layout::Outline, Pane::Outline), 260.0);
        assert_eq!(widths.set(Layout::Columns, Pane::Notes, -5.0), 200.0);
        assert_eq!(widths.get(Layout::Outline, Pane::Notes), 280.0);
        assert_eq!(widths.set(Layout::Columns, Pane::Groups, 900.0), 400.0);
        assert_eq!(widths.set(Layout::Outline, Pane::Outline, f32::NAN), 260.0);
        widths.reset_layout(Layout::Columns);
        assert_eq!(widths, Widths::default());
        widths.set(Layout::Outline, Pane::Notes, 400.0);
        widths.reset_all();
        assert_eq!(widths, Widths::default());
    }

    #[test]
    fn compression_is_proportional_until_floors_then_editor_shrinks() {
        let widths = Widths::default();
        let normal = widths.effective(Layout::Columns, 1200.0);
        assert_eq!(normal.editor, 500.0);
        let squeezed = widths.effective(Layout::Columns, 880.0);
        assert!((squeezed.get(Pane::Sections) - 160.0).abs() < 0.01);
        assert!((squeezed.get(Pane::Groups) - 176.0).abs() < 0.01);
        assert!((squeezed.get(Pane::Notes) - 224.0).abs() < 0.01);
        assert!((squeezed.editor - 320.0).abs() < 0.01);
        let near_floor = widths.effective(Layout::Columns, 700.0);
        assert!((near_floor.get(Pane::Sections) - 120.0).abs() < 0.01);
        assert!((near_floor.get(Pane::Groups) - 120.0).abs() < 0.01);
        assert!((near_floor.get(Pane::Notes) - 140.0).abs() < 0.01);
        let narrow = widths.effective(Layout::Columns, 600.0);
        assert_eq!(narrow.widths, [120.0, 120.0, 120.0]);
        assert_eq!(narrow.editor, 240.0);
        assert_eq!(
            widths,
            Widths::default(),
            "layout never overwrites preferences"
        );
        assert_eq!(
            widths.effective(Layout::Outline, 0.0).widths,
            [120.0, 120.0]
        );
    }

    #[test]
    fn all_compression_extents_are_bounded_and_monotonic() {
        for layout in [Layout::Columns, Layout::Outline] {
            let widths = Widths::default();
            let mut previous = vec![120.0; layout.panes().len()];
            for available in 0..2000 {
                let effective = widths.effective(layout, available as f32);
                for (i, &pane) in layout.panes().iter().enumerate() {
                    let width = effective.get(pane);
                    assert!(width >= 120.0 - 0.001 && width <= widths.get(layout, pane) + 0.001);
                    assert!(width >= previous[i] - 0.001);
                    previous[i] = width;
                }
            }
        }
    }

    #[test]
    fn outline_persistence_uses_sections_key_and_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let store = wren_core::fs_store::FsStore::open(directory.path()).unwrap();
        let mut state = WorkspaceState::load(&store);
        let mut widths = Widths::from_state(&state);
        widths.set(Layout::Outline, Pane::Outline, 310.0);
        widths
            .store(&mut state, Layout::Outline, Pane::Outline)
            .unwrap();
        assert_eq!(state.width("outline", "sections"), Some(310));
        assert_eq!(Widths::from_state(&state), widths);
    }

    #[test]
    fn fixed_pane_boundary_does_not_compress_filling_body_and_rows_to_zero() {
        use iced::widget::{column, container, row, scrollable, Space};
        let body = container(
            column![
                Space::new().width(Length::Fill).height(46.0),
                scrollable(column![container(Space::new().height(36.0))
                    .id("measured-row")
                    .width(Length::Fill)])
                .height(Length::Fill)
                .width(Length::Fill),
            ]
            .height(Length::Fill),
        )
        .id("measured-body")
        .width(Length::Fill)
        .height(Length::Fill);
        let content = row![body, Space::new().width(1.0).height(Length::Fill)].height(Length::Fill);
        let mut widget: Element<'_, (), iced::Theme, ()> = resizable(
            content,
            Layout::Columns,
            Pane::Groups,
            220.0,
            220.0,
            wren_core::theme::DARK,
            |_| (),
        );
        let mut tree = Tree::new(&widget);
        let node = widget.as_widget_mut().layout(
            &mut tree,
            &(),
            &layout::Limits::with_compression(
                Size::ZERO,
                Size::new(220.0, 300.0),
                Size::new(true, false),
            ),
        );
        struct Measure(Vec<(Id, Rectangle)>);
        impl Operation for Measure {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }
            fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
                if let Some(id) = id {
                    self.0.push((id.clone(), bounds));
                }
            }
        }
        let mut measure = Measure(Vec::new());
        widget
            .as_widget_mut()
            .operate(&mut tree, WidgetLayout::new(&node), &(), &mut measure);
        assert_eq!(node.size().width, 220.0);
        for name in ["measured-body", "measured-row"] {
            let bounds = measure
                .0
                .iter()
                .find(|(id, _)| id == &Id::new(name))
                .unwrap()
                .1;
            assert_eq!(bounds.width, 219.0, "{name}: {bounds:?}");
            assert!(bounds.height > 0.0);
        }
    }

    #[test]
    fn overlap_is_exactly_seven_pixels_and_uses_real_child_bounds() {
        let grip = grip_bounds(bounds());
        assert_eq!(
            grip,
            Rectangle {
                x: 236.5,
                y: 40.0,
                width: 7.0,
                height: 300.0
            }
        );
        for x in [236.5, 240.0, 243.4] {
            assert!(grip.contains(Point::new(x, 60.0)));
        }
        assert!(!grip.contains(Point::new(236.4, 60.0)));
        assert!(!grip.contains(Point::new(240.0, 39.9)));
    }

    #[test]
    fn honest_press_anchor_wins_over_a_newer_batch_cursor() {
        let now = Instant::now();
        let mut state = State::default();
        assert_eq!(
            route(&mut state, &moved(241.0), Some(600.0), now),
            Ownership::Delegate
        );
        assert!(matches!(
            route(&mut state, &press(), Some(600.0), now),
            Ownership::Own(_)
        ));
        assert_eq!(state.drag.unwrap().origin.x, 241.0);
        let mut away = State::default();
        route(&mut away, &moved(600.0), Some(240.0), now);
        assert_eq!(
            route(&mut away, &press(), Some(240.0), now),
            Ownership::Delegate
        );
    }

    #[test]
    fn root_preflight_cursor_overrides_a_grips_stale_local_cache() {
        let mut widget: Element<'_, ResizeEvent, iced::Theme, ()> = resizable(
            iced::widget::Space::new().width(Length::Fill).height(100.0),
            Layout::Columns,
            Pane::Groups,
            220.0,
            220.0,
            wren_core::theme::DARK,
            |event| event,
        );
        let mut tree = Tree::new(&widget);
        let node = widget.as_widget_mut().layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(1000.0, 500.0)),
        );
        tree.state
            .downcast_mut::<GripState<ResizeEvent>>()
            .gesture
            .last_cursor = Some(Point::new(20.0, 50.0));
        let mut messages = vec![];
        let mut shell = Shell::new(&mut messages);
        assert!(route_event(
            &mut widget,
            &mut tree,
            WidgetLayout::new(&node),
            &(),
            GripInput {
                event: &press(),
                cursor: mouse::Cursor::Available(Point::new(221.0, 50.0)),
                enabled: true
            },
            &mut shell,
        ));
        assert!(shell.is_event_captured());
        assert_eq!(
            messages,
            vec![ResizeEvent::Started {
                layout: Layout::Columns,
                pane: Pane::Groups
            }]
        );
    }

    #[test]
    fn drag_uses_preferred_width_and_owns_moves_and_release_outside_window() {
        let now = Instant::now();
        let mut state = State::default();
        route(&mut state, &press(), Some(241.0), now);
        assert_eq!(
            route(&mut state, &moved(281.0), None, now),
            Ownership::Own(vec![ResizeEvent::Changed {
                layout: Layout::Columns,
                pane: Pane::Groups,
                width: 260.0
            }])
        );
        assert_eq!(
            route(&mut state, &release(), None, now),
            Ownership::Own(vec![ResizeEvent::Ended {
                layout: Layout::Columns,
                pane: Pane::Groups,
                width: 260.0
            }])
        );
        assert_eq!(
            route(&mut state, &release(), None, now),
            Ownership::Delegate
        );
        let mut resize = Resize::new(Layout::Columns, Pane::Groups, Point::new(120.0, 0.0), 300.0);
        resize.moved(Point::new(136.0, 0.0));
        assert_eq!(
            resize.width, 316.0,
            "squeezed width is not the preference anchor"
        );
        resize.moved(Point::new(-900.0, 0.0));
        assert_eq!(resize.width, 150.0, "there is no collapse behavior");
    }

    #[test]
    fn blur_settles_and_double_click_resets_without_extra_end() {
        let now = Instant::now();
        let mut state = State::default();
        route(&mut state, &press(), Some(240.0), now);
        assert!(
            matches!(route(&mut state, &Event::Window(window::Event::Unfocused), None, now), Ownership::Broadcast(events) if matches!(events.as_slice(), [ResizeEvent::Ended { .. }]))
        );
        route(&mut state, &press(), Some(240.0), now);
        route(&mut state, &release(), Some(240.0), now);
        assert_eq!(
            route(
                &mut state,
                &press(),
                Some(240.0),
                now + Duration::from_millis(100)
            ),
            Ownership::Own(vec![ResizeEvent::Reset {
                layout: Layout::Columns,
                pane: Pane::Groups,
                width: 220.0
            }])
        );
        assert_eq!(
            route(&mut state, &release(), Some(240.0), now),
            Ownership::Own(vec![])
        );
    }

    fn key(named: keyboard::key::Named, modifiers: keyboard::Modifiers) -> Event {
        let key = keyboard::Key::Named(named);
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Escape),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn escape_restores_preference_and_swallows_the_orphan_release() {
        let now = Instant::now();
        let mut state = State::default();
        route(&mut state, &press(), Some(240.0), now);
        route(&mut state, &moved(300.0), Some(300.0), now);
        assert_eq!(
            route(
                &mut state,
                &key(keyboard::key::Named::Escape, keyboard::Modifiers::empty()),
                Some(300.0),
                now
            ),
            Ownership::Own(vec![ResizeEvent::Cancelled {
                layout: Layout::Columns,
                pane: Pane::Groups,
                width: 220.0
            }])
        );
        assert_eq!(state.live_width, 220.0);
        assert_eq!(
            route(&mut state, &release(), None, now),
            Ownership::Own(vec![])
        );
        assert_eq!(
            route(&mut state, &release(), None, now),
            Ownership::Delegate
        );
    }

    #[test]
    fn focused_arrow_steps_are_sixteen_pixels_and_tab_is_routable() {
        let now = Instant::now();
        let mut state = State {
            focused: true,
            ..State::default()
        };
        for width in [236.0, 252.0] {
            assert_eq!(
                route(
                    &mut state,
                    &key(
                        keyboard::key::Named::ArrowRight,
                        keyboard::Modifiers::empty()
                    ),
                    None,
                    now
                ),
                Ownership::Own(vec![ResizeEvent::Step {
                    layout: Layout::Columns,
                    pane: Pane::Groups,
                    width
                }])
            );
        }
        assert_eq!(
            route(
                &mut state,
                &key(
                    keyboard::key::Named::ArrowLeft,
                    keyboard::Modifiers::empty()
                ),
                None,
                now
            ),
            Ownership::Own(vec![ResizeEvent::Step {
                layout: Layout::Columns,
                pane: Pane::Groups,
                width: 236.0
            }])
        );
        assert_eq!(
            route(
                &mut state,
                &key(keyboard::key::Named::Tab, keyboard::Modifiers::SHIFT),
                None,
                now
            ),
            Ownership::Own(vec![ResizeEvent::Tab { reverse: true }])
        );
    }

    #[test]
    fn compressed_widget_floors_overflow_instead_of_disappearing_or_wrapping() {
        let mut widget: Element<'_, ResizeEvent, iced::Theme, ()> = resizable(
            iced::widget::Space::new().width(Length::Fill).height(100.0),
            Layout::Columns,
            Pane::Groups,
            120.0,
            220.0,
            wren_core::theme::DARK,
            |event| event,
        );
        let mut tree = Tree::new(&widget);
        let node = widget.as_widget_mut().layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(50.0, 100.0)),
        );
        assert_eq!(node.size().width, 120.0);
    }

    #[test]
    fn a_real_widget_exposes_focus_and_keeps_child_state_on_diff() {
        use iced::advanced::widget::operation;
        let child = iced::widget::Space::new().width(Length::Fill).height(100.0);
        let mut widget: Element<'_, ResizeEvent, iced::Theme, ()> = resizable(
            child,
            Layout::Columns,
            Pane::Groups,
            220.0,
            220.0,
            wren_core::theme::DARK,
            |event| event,
        );
        let mut tree = Tree::new(&widget);
        let node = widget.as_widget_mut().layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(1000.0, 500.0)),
        );
        let mut focus = operation::focusable::focus::<()>(grip_id(Layout::Columns, Pane::Groups));
        widget
            .as_widget_mut()
            .operate(&mut tree, WidgetLayout::new(&node), &(), &mut focus);
        assert!(
            tree.state
                .downcast_ref::<GripState<ResizeEvent>>()
                .gesture
                .focused
        );
        widget.as_widget().diff(&mut tree);
        assert!(
            tree.state
                .downcast_ref::<GripState<ResizeEvent>>()
                .gesture
                .focused
        );
        assert_eq!(tree.children.len(), 1);
        assert_eq!(node.size().width, 220.0);
    }
}
