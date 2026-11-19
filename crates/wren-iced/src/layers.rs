//! Layers drawn one over another, as iced's `Stack` draws them, that all
//! hear an event something before them took.
//!
//! iced's `Stack` stops handing an event to its layers once the event is
//! captured, which is right for a capture by one of its own layers: a press
//! on a bar floating over the canvas never reaches the canvas. But it stops
//! for a capture made before the stack too, so a press that a filter field
//! in a browser column took never reached the editor or the board beneath
//! the stack, which then kept the keyboard beside the field. These layers
//! stop only at a capture of their own.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell, Widget};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector};

type Renderer = iced::Renderer;

/// The first layer sets the size; each later one is drawn over the ones
/// before it, in the first's bounds.
pub struct Layers<'a, Message> {
    children: Vec<Element<'a, Message, Theme, Renderer>>,
}

/// `children`, the first at the bottom; the bottom one fills its room.
pub fn layers<'a, Message>(
    children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
) -> Layers<'a, Message> {
    Layers {
        children: children.into_iter().collect(),
    }
}

impl<'a, Message: 'a> From<Layers<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(layers: Layers<'a, Message>) -> Self {
        Element::new(layers)
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Layers<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(Length::Fill).height(Length::Fill);
        let Some((base, above)) = self.children.split_first_mut() else {
            return layout::Node::new(limits.resolve(Length::Fill, Length::Fill, Size::ZERO));
        };
        let (base_tree, above_trees) = tree.children.split_at_mut(1);
        let base = base
            .as_widget_mut()
            .layout(&mut base_tree[0], renderer, &limits);
        let size = limits.resolve(Length::Fill, Length::Fill, base.size());
        let limits = layout::Limits::new(Size::ZERO, size);
        let nodes = std::iter::once(base)
            .chain(
                above
                    .iter_mut()
                    .zip(above_trees)
                    .map(|(layer, tree)| layer.as_widget_mut().layout(tree, renderer, &limits)),
            )
            .collect();
        layout::Node::with_children(size, nodes)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for ((child, state), layout) in self
                .children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                child
                    .as_widget_mut()
                    .operate(state, layout, renderer, operation);
            }
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        mut cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // A capture made before the layers is not theirs to stop at.
        let taken_before = shell.is_event_captured();
        let is_over = cursor.is_over(layout.bounds());
        let end = self.children.len().saturating_sub(1);
        for (i, ((child, tree), layout)) in self
            .children
            .iter_mut()
            .rev()
            .zip(tree.children.iter_mut().rev())
            .zip(layout.children().rev())
            .enumerate()
        {
            child.as_widget_mut().update(
                tree, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
            if !taken_before && shell.is_event_captured() {
                return;
            }
            // A layer the pointer is over hides it from the ones beneath.
            if i < end && is_over && !cursor.is_levitating() {
                let interaction = child
                    .as_widget()
                    .mouse_interaction(tree, layout, cursor, viewport, renderer);
                if interaction != mouse::Interaction::None {
                    cursor = cursor.levitate();
                }
            }
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.children
            .iter()
            .rev()
            .zip(tree.children.iter().rev())
            .zip(layout.children().rev())
            .map(|((child, tree), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(tree, layout, cursor, viewport, renderer)
            })
            .find(|interaction| *interaction != mouse::Interaction::None)
            .unwrap_or_default()
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        if layout.bounds().intersection(viewport).is_none() {
            return;
        }
        // The layers under the one the pointer is over draw no hover.
        let covered = if cursor.is_over(layout.bounds()) {
            self.children
                .iter()
                .zip(&tree.children)
                .zip(layout.children())
                .rposition(|((layer, tree), layout)| {
                    layer
                        .as_widget()
                        .mouse_interaction(tree, layout, cursor, viewport, renderer)
                        != mouse::Interaction::None
                })
                .unwrap_or_default()
        } else {
            0
        };
        for (i, ((layer, tree), layout)) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .enumerate()
        {
            let cursor = if i < covered {
                mouse::Cursor::Unavailable
            } else {
                cursor
            };
            if i > 0 {
                renderer.with_layer(*viewport, |renderer| {
                    layer
                        .as_widget()
                        .draw(tree, renderer, theme, style, layout, cursor, viewport);
                });
            } else {
                layer
                    .as_widget()
                    .draw(tree, renderer, theme, style, layout, cursor, viewport);
            }
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
