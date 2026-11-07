//! Keep this wrapper above the complete browser/editor row, outside all scrollables.
//! Row surfaces are identified plain containers, not buttons. Embedded controls and
//! resize seams see a press first; an armed row owns its later motion and release.
//! iced's renderer has no rotation transform, so the ghost is intentionally upright.

use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use iced::advanced::widget::{operation, tree, Id, Operation, Tree};
use iced::advanced::{layout, mouse, overlay, renderer, text, Clipboard, Layout, Shell, Widget};
#[cfg(test)]
use iced::Point;
use iced::{keyboard, window, Color, Element, Event, Font, Length, Rectangle, Size, Vector};
use wren_core::model::Workspace;
use wren_core::theme::{Rgb, Theme};

use crate::drag::{
    CancelReason, Catalog, Drag, DragEvent, Geometry, Preview, RowGeometry, RowKind,
};

#[derive(Debug, Clone)]
pub struct Registry {
    pub catalog: Catalog,
    rows: HashMap<Id, RowKind>,
    excluded: HashSet<Id>,
}

impl Registry {
    /// Always pass the complete workspace, not only the visible/filter-matching rows.
    pub fn new(workspace: &Workspace) -> Self {
        Self {
            catalog: Catalog::from_workspace(workspace),
            rows: HashMap::new(),
            excluded: HashSet::new(),
        }
    }

    /// Register a visible row surface and return its stable container ID.
    pub fn row(&mut self, row: RowKind) -> Id {
        let id = row_id(row);
        self.rows.insert(id.clone(), row);
        id
    }
}

pub fn row_id(row: RowKind) -> Id {
    let name = match row {
        RowKind::Section(id) => format!("section-{}", id.0),
        RowKind::Group(id) => format!("group-{}", id.0),
        RowKind::NoGroup(id) => format!("no-group-{}", id.0),
        RowKind::Note(id) => format!("note-{}", id.0),
    };
    Id::from(format!("wren-drag-row-{name}"))
}

pub fn area_id() -> Id {
    Id::new("wren-file-drag-area")
}

#[cfg(any(test, feature = "smoke-test"))]
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub pressed: bool,
    pub preview: Option<Preview>,
    pub geometry: Geometry,
}

/// A real widget-operation inspection seam; it does not manufacture pointer gestures.
#[cfg(any(test, feature = "smoke-test"))]
pub fn inspect() -> impl Operation<Snapshot> {
    struct Inspect {
        snapshot: Option<Snapshot>,
    }
    impl Operation<Snapshot> for Inspect {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Snapshot>)) {
            operate(self);
        }
        fn custom(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn std::any::Any) {
            if id == Some(&area_id()) {
                if let Some(state) = state.downcast_ref::<State>() {
                    self.snapshot = Some(state.snapshot());
                }
            }
        }
        fn finish(&self) -> operation::Outcome<Snapshot> {
            self.snapshot
                .clone()
                .map_or(operation::Outcome::None, operation::Outcome::Some)
        }
    }
    Inspect { snapshot: None }
}

/// Set `enabled = false` while a modal/palette/scrim is open. Increment `generation`
/// on workspace replacement, NOT on filtering, spring preview, or ordinary re-layout.
/// Apply Commit ops through the application's existing apply_core path.
pub fn area<'a, Message: Send + 'static, R>(
    content: impl Into<Element<'a, Message, iced::Theme, R>>,
    registry: Registry,
    generation: u64,
    enabled: bool,
    theme: Theme,
    on_event: impl Fn(DragEvent) -> Message + 'a,
) -> Element<'a, Message, iced::Theme, R>
where
    R: text::Renderer<Font = Font> + 'a,
{
    Element::new(Area {
        content: content.into(),
        registry,
        generation,
        enabled,
        theme,
        on_event: Box::new(on_event),
    })
}

struct Area<'a, Message, R> {
    content: Element<'a, Message, iced::Theme, R>,
    registry: Registry,
    generation: u64,
    enabled: bool,
    theme: Theme,
    on_event: Box<dyn Fn(DragEvent) -> Message + 'a>,
}

#[derive(Debug)]
struct State {
    drag: Drag,
    started: Instant,
    // None is unknown; Some(Unavailable) records an observed exit or blur.
    last_cursor: Option<mouse::Cursor>,
    geometry: Geometry,
    excluded: Vec<Rectangle>,
    context_pressed: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            drag: Drag::default(),
            started: Instant::now(),
            last_cursor: None,
            geometry: Geometry::new(Rectangle::with_size(Size::ZERO)),
            excluded: vec![],
            context_pressed: false,
        }
    }
}

impl State {
    #[cfg(any(test, feature = "smoke-test"))]
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            pressed: self.drag.is_pressed(),
            preview: self.drag.preview().cloned(),
            geometry: self.geometry.clone(),
        }
    }

    fn cursor(&mut self, event: &Event, batch: mouse::Cursor) -> mouse::Cursor {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                self.last_cursor = Some(mouse::Cursor::Available(*position));
            }
            Event::Mouse(mouse::Event::CursorLeft) | Event::Window(window::Event::Unfocused) => {
                self.last_cursor = Some(mouse::Cursor::Unavailable);
            }
            Event::Mouse(mouse::Event::CursorEntered) | Event::Window(window::Event::Focused)
                if self.last_cursor == Some(mouse::Cursor::Unavailable) =>
            {
                self.last_cursor = None;
            }
            _ => {}
        }
        if batch.is_levitating() {
            batch
        } else {
            self.last_cursor.unwrap_or(batch)
        }
    }
}

/// scrollable() is immediately followed by traverse() in iced. Scope that pending
/// translation to precisely its descendants, restoring both offset and clip afterwards.
struct Measure<'a> {
    registry: &'a Registry,
    geometry: Geometry,
    excluded: Vec<Rectangle>,
    offset: Vector,
    clip: Option<Rectangle>,
    pending_scroll: Option<(Rectangle, Vector)>,
}

impl<'a> Measure<'a> {
    fn new(registry: &'a Registry, root: Rectangle) -> Self {
        Self {
            registry,
            geometry: Geometry::new(root),
            excluded: vec![],
            offset: Vector::ZERO,
            clip: Some(root),
            pending_scroll: None,
        }
    }
}

fn translated(bounds: Rectangle, offset: Vector) -> Rectangle {
    Rectangle {
        x: bounds.x - offset.x,
        y: bounds.y - offset.y,
        ..bounds
    }
}

impl Operation for Measure<'_> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let previous = (self.offset, self.clip);
        if let Some((viewport, translation)) = self.pending_scroll.take() {
            self.clip = self
                .clip
                .and_then(|clip| clip.intersection(&translated(viewport, self.offset)));
            self.offset += translation;
        }
        operate(self);
        (self.offset, self.clip) = previous;
    }

    fn scrollable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        self.pending_scroll = Some((bounds, translation));
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        let Some(id) = id else {
            return;
        };
        let Some(clip) = self.clip else {
            return;
        };
        let bounds = translated(bounds, self.offset);
        if self.registry.excluded.contains(id) {
            if let Some(visible) = bounds.intersection(&clip) {
                self.excluded.push(visible);
            }
        }
        if let Some(&kind) = self.registry.rows.get(id) {
            if let Some(row) = RowGeometry::clipped(kind, bounds, clip) {
                self.geometry.rows.push(row);
            }
        }
    }
}

impl<Message, R: text::Renderer<Font = Font>> Area<'_, Message, R> {
    fn measure(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &R, viewport: &Rectangle) {
        let root = layout
            .bounds()
            .intersection(viewport)
            .unwrap_or(Rectangle::with_size(Size::ZERO));
        let mut measure = Measure::new(&self.registry, root);
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, &mut measure);
        let state = tree.state.downcast_mut::<State>();
        state.geometry = measure.geometry;
        state.excluded = measure.excluded;
    }

    fn publish(&self, events: impl IntoIterator<Item = DragEvent>, shell: &mut Shell<'_, Message>) {
        for event in events {
            shell.publish((self.on_event)(event));
            shell.request_redraw();
        }
    }

    fn timer(state: &State, shell: &mut Shell<'_, Message>) {
        if let Some(deadline) = state.drag.next_deadline() {
            shell.request_redraw_at(state.started + deadline);
        }
    }
}

impl<Message: Send + 'static, R: text::Renderer<Font = Font>> Widget<Message, iced::Theme, R>
    for Area<'_, Message, R>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &R, limits: &layout::Limits) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &R,
        operation: &mut dyn Operation,
    ) {
        self.measure(tree, layout, renderer, &layout.bounds());
        operation.custom(
            Some(&area_id()),
            layout.bounds(),
            tree.state.downcast_mut::<State>(),
        );
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &R,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let cursor = tree.state.downcast_mut::<State>().cursor(event, cursor);
        self.measure(tree, layout, renderer, viewport);
        let state = tree.state.downcast_mut::<State>();
        let now = state.started.elapsed();
        if matches!(
            event,
            Event::Mouse(mouse::Event::CursorLeft) | Event::Window(window::Event::Unfocused)
        ) {
            state.drag.forget_click();
        }
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right))
        ) && std::mem::take(&mut state.context_pressed)
        {
            shell.capture_event();
            return;
        }
        let invalid = if !self.enabled {
            state.drag.cancel(CancelReason::Disabled)
        } else {
            state.drag.validate(self.generation, &self.registry.catalog)
        };
        self.publish(invalid, shell);
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) {
            state.drag.retire_cancelled_release();
        }

        // The child cannot receive half of a pointer sequence. This root survives a
        // spring-open removing the original source's entire scrollable subtree.
        if state.drag.is_pressed() {
            match event {
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    self.publish(
                        state.drag.release(
                            cursor.position(),
                            now,
                            self.generation,
                            &self.registry.catalog,
                            &state.geometry,
                        ),
                        shell,
                    );
                    shell.capture_event();
                    return;
                }
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                    if state.drag.is_active() =>
                {
                    shell.capture_event();
                    return;
                }
                Event::Mouse(mouse::Event::CursorMoved { position }) if state.drag.is_active() => {
                    self.publish(
                        state.drag.motion(
                            *position,
                            now,
                            self.generation,
                            &self.registry.catalog,
                            &state.geometry,
                        ),
                        shell,
                    );
                    Self::timer(state, shell);
                    shell.capture_event();
                    return;
                }
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Escape),
                    ..
                }) if state.drag.is_active() => {
                    self.publish(state.drag.cancel(CancelReason::Escape), shell);
                    shell.capture_event();
                    return;
                }
                Event::Mouse(mouse::Event::CursorLeft) => {
                    self.publish(
                        state.drag.pointer_left(
                            now,
                            self.generation,
                            &self.registry.catalog,
                            &state.geometry,
                        ),
                        shell,
                    );
                }
                Event::Window(window::Event::Unfocused) => {
                    self.publish(state.drag.cancel(CancelReason::Blur), shell);
                }
                _ => {}
            }
        }

        if crate::panes::route_event(
            &mut self.content,
            &mut tree.children[0],
            layout,
            renderer,
            crate::panes::GripInput {
                event,
                cursor,
                enabled: self.enabled,
            },
            shell,
        ) {
            return;
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
        // A wheel event may have just changed scroll state, even when it was captured.
        self.measure(tree, layout, renderer, viewport);
        if self.enabled
            && !shell.is_event_captured()
            && matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(
                    mouse::Button::Left | mouse::Button::Right
                ))
            )
        {
            let interaction = self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            );
            let state = tree.state.downcast_mut::<State>();
            if let Some(position) = cursor.position().filter(|position| {
                !state
                    .excluded
                    .iter()
                    .any(|bounds| bounds.contains(*position))
            }) {
                if interaction == mouse::Interaction::None {
                    if let Some(row) = state.geometry.hit(position).map(|hit| hit.row) {
                        if matches!(
                            event,
                            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
                        ) {
                            state.drag.forget_click();
                            if !state.drag.is_pressed() && row.item().is_some() {
                                state.context_pressed = true;
                                self.publish(
                                    [DragEvent::Context {
                                        generation: self.generation,
                                        row,
                                    }],
                                    shell,
                                );
                                shell.capture_event();
                            }
                        } else if state.drag.press(
                            row,
                            position,
                            self.generation,
                            &self.registry.catalog,
                        ) {
                            shell.capture_event();
                        }
                    }
                }
            }
        }
        let state = tree.state.downcast_mut::<State>();
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left | mouse::Button::Right
            ))
        ) && !state.drag.is_active()
        {
            state.drag.forget_click();
        }
        self.publish(
            state.drag.refresh(
                now,
                self.generation,
                &self.registry.catalog,
                &state.geometry,
            ),
            shell,
        );
        Self::timer(state, shell);
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &R,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        if state.drag.is_dragging() {
            return mouse::Interaction::Grabbing;
        }
        let child = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );
        if child != mouse::Interaction::None {
            return child;
        }
        if self.enabled
            && cursor.position().is_some_and(|position| {
                state.geometry.hit(position).is_some()
                    && !state
                        .excluded
                        .iter()
                        .any(|bounds| bounds.contains(position))
            })
        {
            mouse::Interaction::Pointer
        } else {
            child
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut R,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
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
        if !self.enabled {
            return;
        }
        let state = tree.state.downcast_ref::<State>();
        let Some(preview) = state.drag.preview() else {
            return;
        };
        let Some(root) = layout.bounds().intersection(viewport) else {
            return;
        };
        renderer.with_layer(root, |renderer| {
            draw_preview(
                renderer,
                self.theme,
                preview,
                &state.geometry,
                &self.registry.catalog,
                root,
            )
        });
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
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

fn quad<R: renderer::Renderer>(
    renderer: &mut R,
    bounds: Rectangle,
    fill: Color,
    border: Option<Color>,
) {
    renderer.fill_quad(
        renderer::Quad {
            bounds,
            border: iced::Border {
                color: border.unwrap_or(Color::TRANSPARENT),
                width: if border.is_some() { 1.0 } else { 0.0 },
                radius: 6.0.into(),
            },
            ..renderer::Quad::default()
        },
        fill,
    );
}

fn dashed<R: renderer::Renderer>(renderer: &mut R, bounds: Rectangle, color: Color) {
    for horizontal in [true, false] {
        let extent = if horizontal {
            bounds.width
        } else {
            bounds.height
        };
        let mut offset = 0.0;
        while offset < extent {
            let length = 4.0_f32.min(extent - offset);
            for far in [false, true] {
                let line = if horizontal {
                    Rectangle {
                        x: bounds.x + offset,
                        y: if far {
                            bounds.y + bounds.height - 1.0
                        } else {
                            bounds.y
                        },
                        width: length,
                        height: 1.0,
                    }
                } else {
                    Rectangle {
                        x: if far {
                            bounds.x + bounds.width - 1.0
                        } else {
                            bounds.x
                        },
                        y: bounds.y + offset,
                        width: 1.0,
                        height: length,
                    }
                };
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: line,
                        ..renderer::Quad::default()
                    },
                    color,
                );
            }
            offset += 7.0;
        }
    }
}

fn label<R: text::Renderer<Font = Font>>(
    renderer: &mut R,
    content: String,
    bounds: Rectangle,
    color: Color,
    size: f32,
) {
    renderer.fill_text(
        text::Text {
            content,
            bounds: bounds.size(),
            size: size.into(),
            line_height: text::LineHeight::Relative(1.2),
            font: Font::with_name("Inter"),
            align_x: text::Alignment::Left,
            align_y: iced::alignment::Vertical::Top,
            shaping: text::Shaping::Auto,
            wrapping: text::Wrapping::None,
        },
        bounds.position(),
        color,
        bounds,
    );
}

fn draw_preview<R: text::Renderer<Font = Font>>(
    renderer: &mut R,
    theme: Theme,
    preview: &Preview,
    geometry: &Geometry,
    catalog: &Catalog,
    root: Rectangle,
) {
    let colors = theme.colors;
    let accent = color(colors.accent);
    if let Some(source) = geometry.rows.iter().find(|row| row.row == preview.source) {
        quad(
            renderer,
            source.visible,
            color(if matches!(preview.source, RowKind::Note(_)) {
                colors.notes
            } else {
                colors.pane
            })
            .scale_alpha(0.5),
            None,
        );
        dashed(
            renderer,
            source.visible,
            color(colors.muted).scale_alpha(0.5),
        );
        label(
            renderer,
            "moving…".to_owned(),
            Rectangle {
                x: source.visible.x + (source.visible.width - 65.0).max(4.0),
                y: source.visible.y + 5.0,
                width: source.visible.width.min(60.0),
                height: 18.0,
            },
            color(colors.muted),
            11.0,
        );
    }
    let pending = preview.spring.filter(|spring| !spring.opened);
    if let Some(spring) = pending {
        if let Some(row) = geometry
            .rows
            .iter()
            .find(|row| row.row == RowKind::Section(spring.section))
        {
            quad(renderer, row.visible, accent.scale_alpha(0.08), None);
            dashed(renderer, row.visible, accent);
            label(
                renderer,
                "opening…".to_owned(),
                Rectangle {
                    x: row.visible.x + (row.visible.width - 72.0).max(4.0),
                    y: row.visible.y + 5.0,
                    width: row.visible.width.min(68.0),
                    height: 18.0,
                },
                accent,
                11.0,
            );
        }
    }
    if let Some(target) = preview.target {
        if let Some(insertion) = target.insertion {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: insertion.line,
                    ..renderer::Quad::default()
                },
                accent,
            );
        } else if !pending.is_some_and(|spring| target.row == RowKind::Section(spring.section)) {
            quad(
                renderer,
                target.bounds,
                accent.scale_alpha(0.18),
                Some(accent),
            );
            label(
                renderer,
                "Drop here".to_owned(),
                Rectangle {
                    x: target.bounds.x + (target.bounds.width - 68.0).max(4.0),
                    y: target.bounds.y + 5.0,
                    width: target.bounds.width.min(64.0),
                    height: 18.0,
                },
                accent,
                11.0,
            );
        }
    }
    let width = 260.0_f32.min(root.width);
    let height = 58.0_f32.min(root.height);
    let x = (preview.position.x + 14.0).clamp(root.x, (root.x + root.width - width).max(root.x));
    let y = (preview.position.y + 15.0).clamp(root.y, (root.y + root.height - height).max(root.y));
    let ghost = Rectangle {
        x,
        y,
        width,
        height,
    };
    quad(renderer, ghost, color(colors.raised), Some(accent));
    label(
        renderer,
        catalog.label(preview.source).to_owned(),
        Rectangle {
            x: x + 10.0,
            y: y + 8.0,
            width: (width - 20.0).max(0.0),
            height: 20.0,
        },
        color(colors.text),
        13.0,
    );
    let destination = preview.target.map_or_else(
        || "Choose a valid destination".to_owned(),
        |target| format!("→ {}", catalog.destination_label(target.row)),
    );
    label(
        renderer,
        destination,
        Rectangle {
            x: x + 10.0,
            y: y + 32.0,
            width: (width - 20.0).max(0.0),
            height: 18.0,
        },
        accent,
        11.0,
    );
    if root.height > 100.0 {
        let width = 570.0_f32.min(root.width);
        let toast = Rectangle {
            x: root.center_x() - width / 2.0,
            y: root.y + root.height - 39.0,
            width,
            height: 29.0,
        };
        quad(
            renderer,
            toast,
            color(colors.raised),
            Some(color(colors.dialog_border)),
        );
        label(
            renderer,
            "Hover a section to open it, then drop on a group · esc cancel".to_owned(),
            Rectangle {
                x: toast.x + 10.0,
                y: toast.y + 7.0,
                width: (width - 20.0).max(0.0),
                height: 18.0,
            },
            color(colors.muted),
            12.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::widget::operation::scrollable::AbsoluteOffset;
    use iced::advanced::Clipboard;
    use iced::widget::{button, column, container, row, scrollable, Space};
    use wren_core::fs_store::RelPath;
    use wren_core::model::{Group, GroupId, Note, NoteId, Parent, Section, SectionId};

    #[derive(Debug, Clone)]
    enum Message {
        Drag(DragEvent),
        Resize(crate::panes::ResizeEvent),
        Editor(iced::widget::text_editor::Action),
        Control,
    }

    fn workspace() -> Workspace {
        let mut workspace = Workspace::empty("/unused".into());
        workspace.sections = vec![
            Section {
                id: SectionId(1),
                name: "One".into(),
                path: RelPath::new("One").unwrap(),
            },
            Section {
                id: SectionId(2),
                name: "Two".into(),
                path: RelPath::new("Two").unwrap(),
            },
        ];
        workspace.groups = vec![Group {
            id: GroupId(20),
            section: SectionId(2),
            name: "Destination".into(),
            path: RelPath::new("Two/Destination").unwrap(),
        }];
        workspace.notes = vec![Note {
            id: NoteId(100),
            parent: Parent::Section(SectionId(1)),
            title: "Source".into(),
            path: RelPath::new("One/Source.md").unwrap(),
            subtitle: None,
            modified: std::time::SystemTime::UNIX_EPOCH,
            bytes: 0,
        }];
        workspace
    }

    fn press() -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
    }
    fn release() -> Event {
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
    }
    fn moved(x: f32, y: f32) -> Event {
        Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(x, y),
        })
    }

    fn make_area(
        source_visible: bool,
        enabled: bool,
    ) -> Element<'static, Message, iced::Theme, ()> {
        let mut registry = Registry::new(&workspace());
        let mut left = column![container(
            button(Space::new().width(40.0).height(20.0)).on_press(Message::Control)
        )
        .height(40.0)];
        if source_visible {
            left = left.push(
                container(Space::new().width(180.0).height(36.0))
                    .id(registry.row(RowKind::Note(NoteId(100)))),
            );
        }
        let right = column![
            container(Space::new().width(180.0).height(36.0))
                .id(registry.row(RowKind::Section(SectionId(2)))),
            container(Space::new().width(180.0).height(54.0))
                .id(registry.row(RowKind::Group(GroupId(20))))
        ];
        let left: Element<'static, Message, iced::Theme, ()> = if source_visible {
            scrollable(left).width(200.0).height(300.0).into()
        } else {
            container(Space::new().width(200.0).height(300.0)).into()
        };
        area(
            row![left, right.width(200.0)].height(300.0),
            registry,
            7,
            enabled,
            wren_core::theme::DARK,
            Message::Drag,
        )
    }

    fn layout_widget(
        widget: &mut Element<'_, Message, iced::Theme, ()>,
        tree: &mut Tree,
    ) -> layout::Node {
        widget.as_widget_mut().layout(
            tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(900.0, 600.0)),
        )
    }

    fn send(
        widget: &mut Element<'_, Message, iced::Theme, ()>,
        tree: &mut Tree,
        node: &layout::Node,
        event: &Event,
        batch: Option<Point>,
    ) -> (Vec<Message>, iced::event::Status, window::RedrawRequest) {
        let mut messages = vec![];
        let mut clipboard = iced::advanced::clipboard::Null;
        let mut shell = Shell::new(&mut messages);
        widget.as_widget_mut().update(
            tree,
            event,
            Layout::new(node),
            batch.map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available),
            &(),
            &mut clipboard as &mut dyn Clipboard,
            &mut shell,
            &Rectangle::with_size(Size::new(900.0, 600.0)),
        );
        let status = shell.event_status();
        let redraw = shell.redraw_request();
        (messages, status, redraw)
    }

    #[test]
    fn inspection_measures_the_current_layout_without_a_pointer_event() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        let mut operation = inspect();
        widget.as_widget_mut().operate(
            &mut tree,
            Layout::new(&node),
            &(),
            &mut operation::black_box(&mut operation),
        );
        let operation::Outcome::Some(snapshot) = operation.finish() else {
            panic!("the actual drag widget must be inspectable");
        };
        assert_eq!(snapshot.geometry.rows.len(), 3);
        assert!(snapshot
            .geometry
            .rows
            .iter()
            .any(|row| row.row == RowKind::Note(NoteId(100))));
        let mut replacement = make_area(false, true);
        tree.diff(replacement.as_widget());
        let node = layout_widget(&mut replacement, &mut tree);
        let mut operation = inspect();
        replacement.as_widget_mut().operate(
            &mut tree,
            Layout::new(&node),
            &(),
            &mut operation::black_box(&mut operation),
        );
        let operation::Outcome::Some(snapshot) = operation.finish() else {
            panic!("the replacement must remain inspectable");
        };
        assert_eq!(snapshot.geometry.rows.len(), 2);
        assert!(!snapshot
            .geometry
            .rows
            .iter()
            .any(|row| row.row == RowKind::Note(NoteId(100))));
    }

    #[test]
    fn actual_widget_owns_row_click_and_not_the_header_button() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        let at = Some(Point::new(20.0, 50.0));
        let (messages, status, _) = send(&mut widget, &mut tree, &node, &press(), at);
        assert!(messages.is_empty());
        assert_eq!(status, iced::event::Status::Captured);
        let (messages, _, _) = send(&mut widget, &mut tree, &node, &release(), at);
        assert!(matches!(
            messages.as_slice(),
            [Message::Drag(DragEvent::Activate {
                row: RowKind::Note(NoteId(100)),
                ..
            })]
        ));
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(10.0, 10.0)),
        );
        assert!(messages.is_empty());
        assert!(!tree.state.downcast_ref::<State>().drag.is_pressed());
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &release(),
            Some(Point::new(10.0, 10.0)),
        );
        assert!(matches!(messages.as_slice(), [Message::Control]));
    }

    #[test]
    fn an_embedded_button_inside_a_registered_row_owns_its_whole_click() {
        let mut registry = Registry::new(&workspace());
        let surface = container(row![
            Space::new().width(100.0).height(36.0),
            button(Space::new().width(40.0).height(20.0)).on_press(Message::Control)
        ])
        .id(registry.row(RowKind::Section(SectionId(2))));
        let mut widget: Element<'_, Message, iced::Theme, ()> = area(
            surface,
            registry,
            7,
            true,
            wren_core::theme::DARK,
            Message::Drag,
        );
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(110.0, 10.0)),
        );
        assert!(!tree.state.downcast_ref::<State>().drag.is_pressed());
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &release(),
            Some(Point::new(110.0, 10.0)),
        );
        assert!(matches!(messages.as_slice(), [Message::Control]));
    }

    #[test]
    fn disabling_the_area_settles_an_active_resize_and_blocks_new_seam_presses() {
        use crate::panes::{resizable, Layout as PaneLayout, Pane, ResizeEvent};
        let build = |enabled| -> Element<'static, Message, iced::Theme, ()> {
            let pane = resizable(
                Space::new().width(Length::Fill).height(300.0),
                PaneLayout::Columns,
                Pane::Notes,
                280.0,
                280.0,
                wren_core::theme::DARK,
                Message::Resize,
            );
            area(
                row![pane, Space::new().width(400.0).height(300.0)],
                Registry::new(&workspace()),
                7,
                enabled,
                wren_core::theme::DARK,
                Message::Drag,
            )
        };
        let mut widget = build(true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(282.0, 30.0)),
        );
        let mut disabled = build(false);
        tree.diff(disabled.as_widget());
        let node = layout_widget(&mut disabled, &mut tree);
        let (messages, _, _) = send(
            &mut disabled,
            &mut tree,
            &node,
            &Event::Window(window::Event::RedrawRequested(Instant::now())),
            Some(Point::new(282.0, 30.0)),
        );
        assert!(matches!(
            messages.as_slice(),
            [Message::Resize(ResizeEvent::Ended { .. })]
        ));
        let (messages, _, _) = send(
            &mut disabled,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(282.0, 30.0)),
        );
        assert!(messages.is_empty());
        let (messages, _, _) = send(
            &mut disabled,
            &mut tree,
            &node,
            &release(),
            Some(Point::new(282.0, 30.0)),
        );
        assert!(messages.is_empty());
    }

    #[test]
    fn root_resize_preflight_keeps_a_seam_press_out_of_the_real_sibling_editor() {
        use crate::panes::{resizable, Layout as PaneLayout, Pane, ResizeEvent};
        let content = iced::widget::text_editor::Content::<()>::with_text("A preserved editor");
        let pane = resizable(
            Space::new().width(Length::Fill).height(300.0),
            PaneLayout::Columns,
            Pane::Notes,
            280.0,
            280.0,
            wren_core::theme::DARK,
            Message::Resize,
        );
        let editor = iced::widget::text_editor(&content)
            .width(400.0)
            .height(300.0)
            .on_action(Message::Editor);
        let mut widget: Element<'_, Message, iced::Theme, ()> = area(
            row![pane, editor],
            Registry::new(&workspace()),
            7,
            true,
            wren_core::theme::DARK,
            Message::Drag,
        );
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        let (messages, status, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(282.0, 30.0)),
        );
        assert_eq!(status, iced::event::Status::Captured);
        assert!(matches!(
            messages.as_slice(),
            [Message::Resize(ResizeEvent::Started {
                pane: Pane::Notes,
                ..
            })]
        ));
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &moved(350.0, 30.0),
            Some(Point::new(350.0, 30.0)),
        );
        assert!(matches!(
            messages.as_slice(),
            [Message::Resize(ResizeEvent::Changed { width: 348.0, .. })]
        ));
        let (messages, _, _) = send(&mut widget, &mut tree, &node, &release(), None);
        assert!(matches!(
            messages.as_slice(),
            [Message::Resize(ResizeEvent::Ended { width: 348.0, .. })]
        ));
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(400.0, 30.0)),
        );
        assert!(messages.iter().any(|message| matches!(
            message,
            Message::Editor(iced::widget::text_editor::Action::Click(_))
        )));
    }

    fn resizable_area<'a>(
        content: &'a iced::widget::text_editor::Content<()>,
    ) -> Element<'a, Message, iced::Theme, ()> {
        use crate::panes::{resizable, Layout as PaneLayout, Pane};
        let mut registry = Registry::new(&workspace());
        let source = container(Space::new().width(180.0).height(36.0))
            .id(registry.row(RowKind::Note(NoteId(100))));
        let pane = resizable(
            column![Space::new().height(40.0), source].height(300.0),
            PaneLayout::Columns,
            Pane::Notes,
            280.0,
            280.0,
            wren_core::theme::DARK,
            Message::Resize,
        );
        let editor = iced::widget::text_editor(content)
            .width(400.0)
            .height(300.0)
            .on_action(Message::Editor);
        area(
            row![pane, editor],
            registry,
            7,
            true,
            wren_core::theme::DARK,
            Message::Drag,
        )
    }

    fn cancel_file_drag(
        widget: &mut Element<'_, Message, iced::Theme, ()>,
        tree: &mut Tree,
        node: &layout::Node,
    ) {
        send(
            widget,
            tree,
            node,
            &moved(20.0, 50.0),
            Some(Point::new(20.0, 50.0)),
        );
        send(widget, tree, node, &press(), Some(Point::new(20.0, 50.0)));
        send(
            widget,
            tree,
            node,
            &moved(30.0, 50.0),
            Some(Point::new(30.0, 50.0)),
        );
        let (messages, _, _) = send(
            widget,
            tree,
            node,
            &Event::Window(window::Event::Unfocused),
            None,
        );
        assert!(messages.iter().any(|message| matches!(
            message,
            Message::Drag(DragEvent::Cancelled {
                reason: CancelReason::Blur,
                ..
            })
        )));
        let state = tree.state.downcast_ref::<State>();
        assert!(!state.drag.is_active());
        assert!(state.drag.is_pressed());
    }

    #[test]
    fn root_seam_press_after_file_drag_uses_root_cursor_not_stale_grip_cache() {
        use crate::panes::{Pane, ResizeEvent};
        let content = iced::widget::text_editor::Content::<()>::with_text("A preserved editor");
        let mut widget = resizable_area(&content);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        let batch = Some(Point::new(282.0, 50.0));
        send(&mut widget, &mut tree, &node, &moved(20.0, 50.0), batch);
        let (messages, status, _) = send(&mut widget, &mut tree, &node, &press(), batch);
        assert!(
            tree.state.downcast_ref::<State>().drag.is_active(),
            "source press failed: {messages:?}, {status:?}",
        );
        send(&mut widget, &mut tree, &node, &moved(282.0, 50.0), batch);
        let (_, status, _) = send(&mut widget, &mut tree, &node, &release(), batch);
        assert_eq!(status, iced::event::Status::Captured);
        let (messages, status, _) = send(&mut widget, &mut tree, &node, &press(), batch);
        assert!(
            messages.iter().any(|message| matches!(
                message,
                Message::Resize(ResizeEvent::Started {
                    pane: Pane::Notes,
                    ..
                })
            )),
            "the root's latest cursor must start resizing: {messages:?}"
        );
        assert!(
            !messages.iter().any(|message| matches!(
                message,
                Message::Editor(iced::widget::text_editor::Action::Click(_))
            )),
            "a seam press must not click the real sibling editor"
        );
        assert_eq!(status, iced::event::Status::Captured);
    }

    #[test]
    fn cancelled_file_drag_does_not_consume_a_new_resize_release() {
        use crate::panes::{Pane, ResizeEvent};
        let content = iced::widget::text_editor::Content::<()>::with_text("A preserved editor");
        let mut widget = resizable_area(&content);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        cancel_file_drag(&mut widget, &mut tree, &node);
        send(
            &mut widget,
            &mut tree,
            &node,
            &moved(282.0, 50.0),
            Some(Point::new(282.0, 50.0)),
        );
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(282.0, 50.0)),
        );
        assert!(matches!(
            messages.as_slice(),
            [Message::Resize(ResizeEvent::Started {
                pane: Pane::Notes,
                ..
            })]
        ));
        send(
            &mut widget,
            &mut tree,
            &node,
            &moved(330.0, 50.0),
            Some(Point::new(330.0, 50.0)),
        );
        let (messages, status, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &release(),
            Some(Point::new(330.0, 50.0)),
        );
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Resize(ResizeEvent::Ended {
                    pane: Pane::Notes,
                    width: 328.0,
                    ..
                })]
            ),
            "the new release must end the resize: {messages:?}"
        );
        assert_eq!(status, iced::event::Status::Captured);
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &moved(350.0, 50.0),
            Some(Point::new(350.0, 50.0)),
        );
        assert!(!messages
            .iter()
            .any(|message| matches!(message, Message::Resize(ResizeEvent::Changed { .. }))));
    }

    #[test]
    fn cancelled_file_drag_does_not_consume_a_new_button_release() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        cancel_file_drag(&mut widget, &mut tree, &node);
        let batch = Some(Point::new(10.0, 10.0));
        send(&mut widget, &mut tree, &node, &moved(10.0, 10.0), batch);
        send(&mut widget, &mut tree, &node, &press(), batch);
        let (messages, status, _) = send(&mut widget, &mut tree, &node, &release(), batch);
        assert!(
            matches!(messages.as_slice(), [Message::Control]),
            "the new button click was lost: {messages:?}"
        );
        assert_eq!(status, iced::event::Status::Captured);
        assert!(!tree.state.downcast_ref::<State>().drag.is_pressed());
    }

    #[test]
    fn cancelled_file_drag_still_captures_an_orphan_release_without_a_new_press() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        cancel_file_drag(&mut widget, &mut tree, &node);
        let (messages, status, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &release(),
            Some(Point::new(10.0, 10.0)),
        );
        assert!(messages.is_empty());
        assert_eq!(status, iced::event::Status::Captured);
        assert!(!tree.state.downcast_ref::<State>().drag.is_pressed());
    }

    #[test]
    fn actual_root_commits_after_diff_hides_source_scroll_subtree() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(20.0, 50.0)),
        );
        send(
            &mut widget,
            &mut tree,
            &node,
            &moved(230.0, 55.0),
            Some(Point::new(230.0, 55.0)),
        );
        let mut replacement = make_area(false, true);
        tree.diff(replacement.as_widget());
        let node = layout_widget(&mut replacement, &mut tree);
        let (messages, status, _) = send(
            &mut replacement,
            &mut tree,
            &node,
            &release(),
            Some(Point::new(230.0, 55.0)),
        );
        assert_eq!(status, iced::event::Status::Captured);
        assert_eq!(
            messages
                .iter()
                .filter(|message| matches!(
                    message,
                    Message::Drag(DragEvent::Commit {
                        op: wren_core::ops::Op::MoveNote { .. },
                        ..
                    })
                ))
                .count(),
            1
        );
        let (again, _, _) = send(
            &mut replacement,
            &mut tree,
            &node,
            &release(),
            Some(Point::new(230.0, 55.0)),
        );
        assert!(again.is_empty());
    }

    #[test]
    fn actual_widget_anchors_press_to_the_event_not_the_batch_cursor() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        let batch = Some(Point::new(800.0, 500.0));
        send(&mut widget, &mut tree, &node, &moved(20.0, 50.0), batch);
        send(&mut widget, &mut tree, &node, &press(), batch);
        assert!(tree.state.downcast_ref::<State>().drag.is_pressed());
        send(&mut widget, &mut tree, &node, &moved(230.0, 55.0), batch);
        let (messages, _, _) = send(&mut widget, &mut tree, &node, &release(), batch);
        assert!(messages
            .iter()
            .any(|message| matches!(message, Message::Drag(DragEvent::Commit { .. }))));
    }

    #[test]
    fn actual_widget_release_before_cursor_left_commits_with_unavailable_final_batch() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(20.0, 50.0)),
        );
        send(&mut widget, &mut tree, &node, &moved(230.0, 55.0), None);
        let (messages, status, _) = send(&mut widget, &mut tree, &node, &release(), None);
        assert_eq!(status, iced::event::Status::Captured);
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Drag(DragEvent::Commit {
                    op: wren_core::ops::Op::MoveNote {
                        note: NoteId(100),
                        to: Parent::Group(GroupId(20))
                    },
                    ..
                })]
            ),
            "a later CursorLeft cannot invalidate the earlier release: {messages:?}"
        );
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &Event::Mouse(mouse::Event::CursorLeft),
            None,
        );
        assert!(messages.is_empty());
        assert!(!tree.state.downcast_ref::<State>().drag.is_pressed());
    }

    #[test]
    fn actual_widget_cursor_left_before_release_cancels_in_event_order() {
        for batch in [None, Some(Point::new(230.0, 55.0))] {
            let mut widget = make_area(true, true);
            let mut tree = Tree::new(&widget);
            let node = layout_widget(&mut widget, &mut tree);
            send(
                &mut widget,
                &mut tree,
                &node,
                &press(),
                Some(Point::new(20.0, 50.0)),
            );
            send(&mut widget, &mut tree, &node, &moved(230.0, 55.0), batch);
            send(
                &mut widget,
                &mut tree,
                &node,
                &Event::Mouse(mouse::Event::CursorLeft),
                batch,
            );
            let (messages, status, _) = send(&mut widget, &mut tree, &node, &release(), batch);
            assert_eq!(status, iced::event::Status::Captured);
            assert!(matches!(messages.as_slice(), [Message::Drag(DragEvent::Cancelled { reason: CancelReason::InvalidTarget, .. })]),
                "an observed CursorLeft must invalidate the release, even with a future batch position: {messages:?}");
            assert!(!tree.state.downcast_ref::<State>().drag.is_pressed());
        }
    }

    #[test]
    fn nonmovement_events_preserve_an_explicit_position_without_a_batch_cursor() {
        let mut state = State::default();
        let at = Point::new(230.0, 55.0);
        state.cursor(&moved(at.x, at.y), mouse::Cursor::Unavailable);
        for event in [
            Event::Mouse(mouse::Event::CursorEntered),
            Event::Window(window::Event::Focused),
            press(),
            release(),
        ] {
            assert_eq!(
                state.cursor(&event, mouse::Cursor::Unavailable),
                mouse::Cursor::Available(at)
            );
        }
    }

    #[test]
    fn event_ordered_cursor_preserves_layer_unavailability() {
        let mut state = State::default();
        let modal = mouse::Cursor::Levitating(Point::new(800.0, 500.0));
        assert_eq!(state.cursor(&moved(20.0, 50.0), modal), modal);
        assert_eq!(state.cursor(&press(), modal), modal);
        assert_eq!(state.cursor(&release(), modal), modal);
    }

    #[test]
    fn observed_cursor_exit_or_blur_stays_unavailable_until_reentry() {
        for event in [
            Event::Mouse(mouse::Event::CursorLeft),
            Event::Window(window::Event::Unfocused),
        ] {
            let mut state = State::default();
            let batch = mouse::Cursor::Available(Point::new(20.0, 50.0));
            state.cursor(&moved(230.0, 55.0), batch);
            assert_eq!(state.cursor(&event, batch), mouse::Cursor::Unavailable);
            assert_eq!(state.cursor(&release(), batch), mouse::Cursor::Unavailable);
            let reentered = if matches!(event, Event::Mouse(_)) {
                Event::Mouse(mouse::Event::CursorEntered)
            } else {
                Event::Window(window::Event::Focused)
            };
            assert_eq!(state.cursor(&reentered, batch), batch);
            assert_eq!(state.cursor(&press(), batch), batch);
        }
    }

    #[test]
    fn event_ordered_cursor_does_not_bypass_a_disabled_area() {
        let mut widget = make_area(true, false);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        send(&mut widget, &mut tree, &node, &moved(20.0, 50.0), None);
        let (messages, _, _) = send(&mut widget, &mut tree, &node, &press(), None);
        assert!(messages.is_empty());
        assert!(!tree.state.downcast_ref::<State>().drag.is_pressed());
        send(&mut widget, &mut tree, &node, &moved(230.0, 55.0), None);
        let (messages, _, _) = send(&mut widget, &mut tree, &node, &release(), None);
        assert!(messages.is_empty());
    }

    #[test]
    fn real_scroll_operation_reports_translated_clipped_rows_and_recomputes() {
        let mut registry = Registry::new(&workspace());
        let row = container(Space::new().width(180.0).height(54.0))
            .id(registry.row(RowKind::Note(NoteId(100))));
        let scroll_id = Id::new("test-scroll");
        let mut widget: Element<'_, Message, iced::Theme, ()> = area(
            column![
                Space::new().height(40.0),
                scrollable(column![
                    Space::new().height(100.0),
                    row,
                    Space::new().height(200.0)
                ])
                .id(scroll_id.clone())
                .height(100.0)
                .width(200.0)
            ],
            registry,
            7,
            true,
            wren_core::theme::DARK,
            Message::Drag,
        );
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        let mut scroll = operation::scrollable::scroll_to::<()>(
            scroll_id.clone(),
            AbsoluteOffset {
                x: None,
                y: Some(120.0),
            },
        );
        widget
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &(), &mut scroll);
        send(
            &mut widget,
            &mut tree,
            &node,
            &moved(10.0, 30.0),
            Some(Point::new(10.0, 30.0)),
        );
        let geometry = &tree.state.downcast_ref::<State>().geometry;
        assert_eq!(geometry.rows.len(), 1);
        assert_eq!(geometry.rows[0].bounds.y, 20.0);
        assert_eq!(geometry.rows[0].visible.y, 40.0);
        assert_eq!(geometry.rows[0].visible.height, 34.0);
        assert!(geometry.hit(Point::new(10.0, 30.0)).is_none());
        assert!(geometry.hit(Point::new(10.0, 45.0)).is_some());
        let mut scroll_back = operation::scrollable::scroll_to::<()>(
            scroll_id,
            AbsoluteOffset {
                x: None,
                y: Some(0.0),
            },
        );
        widget
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &(), &mut scroll_back);
        send(
            &mut widget,
            &mut tree,
            &node,
            &moved(10.0, 45.0),
            Some(Point::new(10.0, 45.0)),
        );
        assert!(
            tree.state.downcast_ref::<State>().geometry.rows.is_empty(),
            "offscreen rows must disappear from the next actual geometry pass"
        );
        let mut inspect = inspect();
        let mut inspect_erased = operation::black_box::<Snapshot, ()>(&mut inspect);
        widget
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &(), &mut inspect_erased);
        drop(inspect_erased);
        assert!(matches!(
            inspect.finish(),
            operation::Outcome::Some(Snapshot {
                pressed: false,
                preview: None,
                ..
            })
        ));
    }

    #[test]
    fn real_nested_scrollables_clip_in_window_coordinates() {
        let mut registry = Registry::new(&workspace());
        let source = container(Space::new().width(180.0).height(54.0))
            .id(registry.row(RowKind::Note(NoteId(100))));
        let inner_id = Id::new("nested-inner");
        let outer_id = Id::new("nested-outer");
        let inner = scrollable(column![
            Space::new().height(40.0),
            source,
            Space::new().height(200.0)
        ])
        .id(inner_id.clone())
        .height(100.0)
        .width(200.0);
        let outer = scrollable(column![
            Space::new().height(80.0),
            inner,
            Space::new().height(200.0)
        ])
        .id(outer_id.clone())
        .height(150.0)
        .width(220.0);
        let mut widget: Element<'_, Message, iced::Theme, ()> = area(
            column![Space::new().height(40.0), outer],
            registry,
            7,
            true,
            wren_core::theme::DARK,
            Message::Drag,
        );
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        for (id, offset) in [(outer_id, 50.0), (inner_id, 60.0)] {
            let mut scroll = operation::scrollable::scroll_to::<()>(
                id,
                AbsoluteOffset {
                    x: None,
                    y: Some(offset),
                },
            );
            widget
                .as_widget_mut()
                .operate(&mut tree, Layout::new(&node), &(), &mut scroll);
        }
        send(
            &mut widget,
            &mut tree,
            &node,
            &moved(10.0, 75.0),
            Some(Point::new(10.0, 75.0)),
        );
        let geometry = &tree.state.downcast_ref::<State>().geometry;
        assert_eq!(geometry.rows.len(), 1);
        assert_eq!(geometry.rows[0].bounds.y, 50.0);
        assert_eq!(geometry.rows[0].visible.y, 70.0);
        assert_eq!(geometry.rows[0].visible.height, 34.0);
        assert!(geometry.hit(Point::new(10.0, 60.0)).is_none());
        assert!(geometry.hit(Point::new(10.0, 75.0)).is_some());
    }

    struct Scroll;
    impl operation::Scrollable for Scroll {
        fn snap_to(&mut self, _offset: operation::scrollable::RelativeOffset<Option<f32>>) {}
        fn scroll_to(&mut self, _offset: AbsoluteOffset<Option<f32>>) {}
        fn scroll_by(
            &mut self,
            _offset: AbsoluteOffset,
            _bounds: Rectangle,
            _content_bounds: Rectangle,
        ) {
        }
    }

    #[test]
    fn nested_viewports_accumulate_translation_and_do_not_leak_to_siblings() {
        let mut registry = Registry::new(&workspace());
        let id = registry.row(RowKind::Note(NoteId(100)));
        let mut measure = Measure::new(&registry, Rectangle::with_size(Size::new(500.0, 500.0)));
        let mut scroll = Scroll;
        measure.scrollable(
            None,
            Rectangle::new(Point::new(0.0, 100.0), Size::new(200.0, 200.0)),
            Rectangle::with_size(Size::new(200.0, 600.0)),
            Vector::new(0.0, 50.0),
            &mut scroll,
        );
        measure.traverse(&mut |outer| {
            outer.scrollable(
                None,
                Rectangle::new(Point::new(0.0, 180.0), Size::new(200.0, 80.0)),
                Rectangle::with_size(Size::new(200.0, 400.0)),
                Vector::new(0.0, 30.0),
                &mut Scroll,
            );
            outer.traverse(&mut |inner| {
                inner.container(
                    Some(&id),
                    Rectangle::new(Point::new(0.0, 200.0), Size::new(180.0, 54.0)),
                )
            });
        });
        let row = measure.geometry.rows[0];
        assert_eq!(row.bounds.y, 120.0);
        assert_eq!(row.visible.y, 130.0);
        assert_eq!(row.visible.height, 44.0);
        measure.container(
            Some(&id),
            Rectangle::new(Point::new(250.0, 20.0), Size::new(180.0, 54.0)),
        );
        assert_eq!(measure.geometry.rows[1].bounds.y, 20.0);
    }

    #[test]
    fn spring_hover_requests_a_real_timer_redraw() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(20.0, 50.0)),
        );
        send(
            &mut widget,
            &mut tree,
            &node,
            &moved(230.0, 10.0),
            Some(Point::new(230.0, 10.0)),
        );
        let (_, _, request) = send(
            &mut widget,
            &mut tree,
            &node,
            &Event::Window(window::Event::RedrawRequested(Instant::now())),
            Some(Point::new(230.0, 10.0)),
        );
        assert!(matches!(request, window::RedrawRequest::At(_)));
        tree.state.downcast_mut::<State>().started -= Duration::from_millis(601);
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &Event::Window(window::Event::RedrawRequested(Instant::now())),
            Some(Point::new(230.0, 10.0)),
        );
        assert!(messages.iter().any(|message| matches!(
            message,
            Message::Drag(DragEvent::Spring {
                section: SectionId(2),
                ..
            })
        )));
    }

    #[test]
    fn real_root_owns_both_click_releases_and_emits_rename_for_the_second() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        let at = Some(Point::new(20.0, 50.0));
        send(&mut widget, &mut tree, &node, &press(), at);
        let (messages, status, _) = send(&mut widget, &mut tree, &node, &release(), at);
        assert_eq!(status, iced::event::Status::Captured);
        assert!(matches!(
            messages.as_slice(),
            [Message::Drag(DragEvent::Activate { .. })]
        ));
        send(&mut widget, &mut tree, &node, &press(), at);
        let (messages, status, _) = send(&mut widget, &mut tree, &node, &release(), at);
        assert_eq!(status, iced::event::Status::Captured);
        assert!(matches!(
            messages.as_slice(),
            [Message::Drag(DragEvent::Rename {
                generation: 7,
                row: RowKind::Note(NoteId(100))
            })]
        ));
    }

    #[test]
    fn embedded_control_click_between_row_clicks_breaks_the_double_click_sequence() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        for (at, expected_row) in [
            (Point::new(20.0, 50.0), true),
            (Point::new(10.0, 10.0), false),
            (Point::new(20.0, 50.0), true),
        ] {
            send(&mut widget, &mut tree, &node, &moved(at.x, at.y), Some(at));
            send(&mut widget, &mut tree, &node, &press(), Some(at));
            let (messages, _, _) = send(&mut widget, &mut tree, &node, &release(), Some(at));
            if expected_row {
                assert!(
                    matches!(
                        messages.as_slice(),
                        [Message::Drag(DragEvent::Activate { .. })]
                    ),
                    "{messages:?}"
                );
            } else {
                assert!(matches!(messages.as_slice(), [Message::Control]));
            }
        }
    }

    #[test]
    fn native_right_click_targets_the_hit_row_without_activating_it_and_owns_its_release() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        let right_press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right));
        let right_release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right));
        let at = Some(Point::new(210.0, 50.0));
        let (messages, status, _) = send(&mut widget, &mut tree, &node, &right_press, at);
        assert_eq!(status, iced::event::Status::Captured);
        assert!(matches!(
            messages.as_slice(),
            [Message::Drag(DragEvent::Context {
                generation: 7,
                row: RowKind::Group(GroupId(20))
            })]
        ));
        assert!(!tree.state.downcast_ref::<State>().drag.is_pressed());
        let (messages, status, _) = send(&mut widget, &mut tree, &node, &right_release, at);
        assert!(messages.is_empty());
        assert_eq!(status, iced::event::Status::Captured);
    }

    #[test]
    fn right_click_never_targets_a_header_button_disabled_area_or_an_active_left_sequence() {
        let right = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right));
        for (enabled, at, left_pressed) in [
            (true, Point::new(10.0, 10.0), false),
            (false, Point::new(210.0, 50.0), false),
            (true, Point::new(210.0, 50.0), true),
        ] {
            let mut widget = make_area(true, enabled);
            let mut tree = Tree::new(&widget);
            let node = layout_widget(&mut widget, &mut tree);
            if left_pressed {
                send(
                    &mut widget,
                    &mut tree,
                    &node,
                    &press(),
                    Some(Point::new(20.0, 50.0)),
                );
                send(&mut widget, &mut tree, &node, &moved(at.x, at.y), Some(at));
            }
            let (messages, _, _) = send(&mut widget, &mut tree, &node, &right, Some(at));
            assert!(!messages.iter().any(|message| matches!(
                message,
                Message::Drag(DragEvent::Context { .. } | DragEvent::Rename { .. })
            )));
        }
    }

    #[test]
    fn hiding_the_clicked_row_before_it_reappears_cannot_trigger_a_double_click_rename() {
        let mut widget = make_area(true, true);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        let at = Some(Point::new(20.0, 50.0));
        send(&mut widget, &mut tree, &node, &press(), at);
        send(&mut widget, &mut tree, &node, &release(), at);
        let mut hidden = make_area(false, true);
        tree.diff(hidden.as_widget());
        let node = layout_widget(&mut hidden, &mut tree);
        send(
            &mut hidden,
            &mut tree,
            &node,
            &Event::Window(window::Event::RedrawRequested(Instant::now())),
            at,
        );
        let mut visible = make_area(true, true);
        tree.diff(visible.as_widget());
        let node = layout_widget(&mut visible, &mut tree);
        send(&mut visible, &mut tree, &node, &press(), at);
        let (messages, _, _) = send(&mut visible, &mut tree, &node, &release(), at);
        assert!(matches!(
            messages.as_slice(),
            [Message::Drag(DragEvent::Activate { .. })]
        ));
    }

    #[test]
    fn modal_disable_cancels_and_never_clicks_through() {
        let mut widget = make_area(false, false);
        let mut tree = Tree::new(&widget);
        let node = layout_widget(&mut widget, &mut tree);
        send(
            &mut widget,
            &mut tree,
            &node,
            &press(),
            Some(Point::new(230.0, 10.0)),
        );
        assert!(!tree.state.downcast_ref::<State>().drag.is_pressed());
        let (messages, _, _) = send(
            &mut widget,
            &mut tree,
            &node,
            &release(),
            Some(Point::new(230.0, 10.0)),
        );
        assert!(messages.is_empty());
    }
}
