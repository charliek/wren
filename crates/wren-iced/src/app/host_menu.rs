//! A menu the app draws itself (discovery/tasks-build.md §10, 2026-11-26
//! item 1): rows as the editor's popover menus have them
//! ([`wren_editor::Row`], with their [`wren_editor::Accelerator`]s), drawn
//! by the host in the calendar's overlay slots (`App::calendar_view`), so
//! the window's root keeps its shape. The task view's scope chip and its
//! filters open one; the status, priority and task menus of a view's rows
//! are its next clients (item 10.4).
//!
//! - It hangs under a rectangle of the window, at its left edge, or over
//!   it when there is no room below, and stays inside the window.
//! - Each menu has an instance number. A choice names the instance it was
//!   made in, so a choice from a menu that has since closed or been
//!   replaced does nothing.
//! - Up and Down move the selection over the rows that can be chosen,
//!   Home and End go to the first and the last, Enter chooses the selected
//!   row, and a row's accelerator chooses it. Any other key is the menu's
//!   and does nothing. Escape is the app's: it closes the menu without a
//!   choice (`App::take_escape_claim`'s precedence).
//! - A press outside the card closes the menu and is dropped: the layer
//!   under the card takes it, so nothing beneath hears it.
//! - A row is chosen when a press on it is released over it (a touch: a
//!   tap, so a drag still scrolls a long menu). Every press over the card,
//!   of any button, is reported as it comes ([`Event::Pressed`]), so the
//!   host can refuse a press that is not the first of its batch or that
//!   follows a key the menu took in that batch (discovery/tasks.md §6.2);
//!   a refused press chooses nothing at its release. Every press and its
//!   release over the card are the card's: nothing beneath hears either.

use iced::keyboard::key::{Named, Physical};
use iced::keyboard::{Key, Modifiers};
use iced::widget::{button, column, container, mouse_area, opaque, pin, row, scrollable};
use iced::{Element, Fill, Point, Rectangle, Size};
use wren_core::theme::Theme;
use wren_editor::Row;

use crate::ellipsis::ellipsis;
use crate::{chrome, fonts, theme};

/// The gap between the anchor and the card.
const GAP: f32 = 4.0;
/// The room the card keeps from the window's edges.
const MARGIN: f32 = 8.0;
const MIN_WIDTH: f32 = 200.0;
const MAX_WIDTH: f32 = 320.0;
const ROW_HEIGHT: f32 = 30.0;
const TITLE_HEIGHT: f32 = 26.0;
const PADDING: f32 = 4.0;
const GLYPH_WIDTH: f32 = 26.0;
const TEXT_SIZE: f32 = 13.0;

/// Input from a host menu's card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// A press over the card of the menu `instance`, at `at` in the
    /// window: `primary` for the left button or a touch.
    Pressed {
        instance: u64,
        at: Point,
        primary: bool,
    },
    /// The row `row` of the menu `instance` was released on after a press
    /// on it.
    Choose { instance: u64, row: u32 },
    /// A press, or a scroll, outside the card.
    Outside { instance: u64 },
    /// The rows of the menu `instance`, taller than the window, scrolled to
    /// `y`.
    Scrolled { instance: u64, y: f32 },
}

/// The scrollable of a menu's rows, when they are taller than the window.
pub const ROWS: &str = "host-menu-rows";

/// What a key does to an open menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKey {
    /// It chose the row with this id: the host acts on it and closes the
    /// menu.
    Choose(u32),
    /// It moved the selection.
    Moved,
    /// The menu took it, and it does nothing.
    Held,
}

/// An open menu, for `purpose`.
#[derive(Debug, Clone, PartialEq)]
pub struct HostMenu<T> {
    instance: u64,
    pub purpose: T,
    title: Option<String>,
    rows: Vec<Row>,
    selected: Option<usize>,
    anchor: Rectangle,
    /// How far the rows are scrolled, when they scroll.
    scroll: f32,
}

impl<T> HostMenu<T> {
    /// A menu of `rows` under `anchor`, numbered `instance`, selecting the
    /// checked row that can be chosen, else the first that can.
    pub fn new(
        instance: u64,
        purpose: T,
        title: Option<String>,
        rows: Vec<Row>,
        anchor: Rectangle,
    ) -> Self {
        let selected = (rows.iter().position(|row| row.checked && row.enabled))
            .or_else(|| rows.iter().position(|row| row.enabled));
        Self {
            instance,
            purpose,
            title,
            rows,
            selected,
            anchor,
            scroll: 0.0,
        }
    }

    /// The rows scrolled to `y`, as the scrollable reports.
    pub fn scrolled(&mut self, y: f32) {
        self.scroll = y;
    }

    /// Where the rows must scroll to show the selected row, in a window of
    /// `window`'s size: `None` when they all show, else the offset, moved
    /// as little as it can be. Asked when the menu opens, since a scrollable
    /// of an earlier menu may keep its offset, and after every key.
    pub fn reveal(&mut self, window: Size) -> Option<f32> {
        let shown = self.bounds(window).height;
        let natural = self.size().height;
        if shown >= natural {
            return None;
        }
        let room = (shown - (natural - self.rows.len() as f32 * ROW_HEIGHT)).max(ROW_HEIGHT);
        let top = self.selected.unwrap_or(0) as f32 * ROW_HEIGHT;
        if top < self.scroll {
            self.scroll = top;
        } else if top + ROW_HEIGHT > self.scroll + room {
            self.scroll = top + ROW_HEIGHT - room;
        }
        Some(self.scroll)
    }

    pub fn instance(&self) -> u64 {
        self.instance
    }

    /// The row a press at `at` falls on, in a window of `window`'s size:
    /// its id, when it can be chosen.
    pub fn row_at(&self, at: Point, window: Size) -> Option<u32> {
        let bounds = self.bounds(window);
        if !bounds.contains(at) {
            return None;
        }
        let title = if self.title.is_some() {
            TITLE_HEIGHT
        } else {
            0.0
        };
        let y = at.y - bounds.y - PADDING - title + self.scroll;
        if y < 0.0 {
            return None;
        }
        let row = self.rows.get((y / ROW_HEIGHT) as usize)?;
        row.enabled.then_some(row.id)
    }

    /// Where the row `id` is drawn, with the rows scrolled as they are.
    #[cfg(any(test, feature = "smoke-test"))]
    pub fn row_bounds(&self, id: u32, window: Size) -> Option<Rectangle> {
        let bounds = self.bounds(window);
        let index = self.rows.iter().position(|row| row.id == id)?;
        let title = if self.title.is_some() {
            TITLE_HEIGHT
        } else {
            0.0
        };
        let y = bounds.y + PADDING + title + index as f32 * ROW_HEIGHT - self.scroll;
        Some(Rectangle::new(
            Point::new(bounds.x + PADDING, y),
            Size::new(bounds.width - 2.0 * PADDING, ROW_HEIGHT),
        ))
    }

    /// Select the row `id`, when it can be chosen.
    pub fn select_id(&mut self, id: u32) {
        if let Some(at) = (self.rows.iter()).position(|row| row.id == id && row.enabled) {
            self.selected = Some(at);
        }
    }

    #[cfg_attr(not(feature = "smoke-test"), allow(dead_code))]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[cfg_attr(not(feature = "smoke-test"), allow(dead_code))]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    #[cfg_attr(not(feature = "smoke-test"), allow(dead_code))]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    #[cfg_attr(not(feature = "smoke-test"), allow(dead_code))]
    pub fn anchor(&self) -> Rectangle {
        self.anchor
    }

    /// The row a click in the menu `instance` chose: its id, when this is
    /// that menu and the row can be chosen.
    pub fn choose(&self, instance: u64, row: u32) -> Option<u32> {
        (instance == self.instance)
            .then(|| self.rows.iter().find(|candidate| candidate.id == row))
            .flatten()
            .filter(|row| row.enabled)
            .map(|row| row.id)
    }

    /// The selection moved `step` rows over those that can be chosen,
    /// round the ends.
    fn step(&mut self, step: isize) {
        let enabled: Vec<usize> = (self.rows.iter().enumerate())
            .filter(|(_, row)| row.enabled)
            .map(|(index, _)| index)
            .collect();
        if enabled.is_empty() {
            return;
        }
        let at = self
            .selected
            .and_then(|selected| enabled.iter().position(|&index| index == selected));
        let count = enabled.len() as isize;
        let next = match at {
            Some(at) => (at as isize + step).rem_euclid(count),
            None if step < 0 => count - 1,
            None => 0,
        };
        self.selected = Some(enabled[next as usize]);
    }

    /// A key pressed while the menu is open. Escape is not read here: it
    /// is the app's (see the module's description).
    pub fn key(
        &mut self,
        key: &Key,
        physical: Physical,
        modifiers: Modifiers,
        text: Option<&str>,
    ) -> MenuKey {
        let plain = !(modifiers.control() || modifiers.alt() || modifiers.logo());
        if plain {
            match key {
                Key::Named(Named::ArrowUp) => {
                    self.step(-1);
                    return MenuKey::Moved;
                }
                Key::Named(Named::ArrowDown) => {
                    self.step(1);
                    return MenuKey::Moved;
                }
                Key::Named(Named::Home) => {
                    self.selected = self.rows.iter().position(|row| row.enabled);
                    return MenuKey::Moved;
                }
                Key::Named(Named::End) => {
                    self.selected = self.rows.iter().rposition(|row| row.enabled);
                    return MenuKey::Moved;
                }
                Key::Named(Named::Enter) => {
                    return (self.selected)
                        .and_then(|selected| self.rows.get(selected))
                        .filter(|row| row.enabled)
                        .map_or(MenuKey::Held, |row| MenuKey::Choose(row.id));
                }
                _ => {}
            }
        }
        // The first row the key names; one that cannot be chosen holds it.
        let named = self.rows.iter().find(|row| {
            row.accelerator.is_some_and(|accelerator| {
                accelerator.matches_press(key, physical, modifiers, text)
            })
        });
        match named {
            Some(row) if row.enabled => MenuKey::Choose(row.id),
            _ => MenuKey::Held,
        }
    }

    /// The card's size: wide enough for the longest label, within bounds.
    fn size(&self) -> Size {
        let widest = (self.rows.iter())
            .map(|row| {
                let hint = row
                    .accelerator
                    .map(|accelerator| accelerator.label())
                    .or_else(|| row.detail.clone())
                    .map_or(0, |hint| hint.chars().count() + 3);
                row.label.chars().count() + hint
            })
            .chain(self.title.iter().map(|title| title.chars().count()))
            .max()
            .unwrap_or(0);
        let width =
            (2.0 * PADDING + GLYPH_WIDTH + 24.0 + widest as f32 * 7.6).clamp(MIN_WIDTH, MAX_WIDTH);
        let title = if self.title.is_some() {
            TITLE_HEIGHT
        } else {
            0.0
        };
        Size::new(
            width,
            2.0 * PADDING + title + self.rows.len() as f32 * ROW_HEIGHT + 2.0,
        )
    }

    /// Where the card is in a window of `window`'s size.
    pub fn bounds(&self, window: Size) -> Rectangle {
        let mut size = self.size();
        // A menu taller than the window scrolls its rows.
        size.height = size
            .height
            .min((window.height - 2.0 * MARGIN).max(ROW_HEIGHT));
        Rectangle::new(place(self.anchor, size, window), size)
    }

    /// The two layers the menu takes over the window: the one that closes
    /// it on a press outside, and the card at its place. Input is sent as
    /// `send` makes it a message.
    pub fn layers<'a, M: Clone + 'a>(
        &'a self,
        definition: Theme,
        window: Size,
        send: impl Fn(Event) -> M + Copy + 'a,
    ) -> (Element<'a, M>, Element<'a, M>) {
        let instance = self.instance;
        let outside = send(Event::Outside { instance });
        let scrim = mouse_area(container(iced::widget::space()).width(Fill).height(Fill))
            .on_press(outside.clone())
            .on_right_press(outside.clone())
            .on_middle_press(outside.clone())
            .on_scroll(move |_| outside.clone());
        let bounds = self.bounds(window);
        let colors = definition.colors;
        let mut card = column![].spacing(0);
        if let Some(title) = &self.title {
            card = card.push(
                container(chrome::label(title.as_str(), 11, colors.muted).font(fonts::SEMIBOLD))
                    .height(TITLE_HEIGHT)
                    .padding([0, 8])
                    .center_y(TITLE_HEIGHT),
            );
        }
        let mut lines = column![].spacing(0);
        for (index, entry) in self.rows.iter().enumerate() {
            let selected = self.selected == Some(index);
            let enabled = entry.enabled;
            let checked = enabled && entry.checked;
            let tint = if !enabled {
                colors.faint
            } else if checked {
                colors.accent
            } else {
                colors.text
            };
            let glyph = container(chrome::label(
                entry.glyph.clone().unwrap_or_default(),
                TEXT_SIZE as u32,
                if enabled { colors.text } else { colors.faint },
            ))
            .center_x(GLYPH_WIDTH);
            let label = ellipsis(entry.label.clone(), TEXT_SIZE, theme::color(tint))
                .font(if checked { fonts::SEMIBOLD } else { fonts::UI })
                .width(Fill);
            let hint = entry
                .accelerator
                .map(|accelerator| accelerator.label())
                .or_else(|| entry.detail.clone())
                .unwrap_or_default();
            let line = row![glyph, label, chrome::label(hint, 11, colors.muted)]
                .spacing(4)
                .align_y(iced::Center)
                .padding([0, 8]);
            let choose = send(Event::Choose {
                instance,
                row: entry.id,
            });
            let line = button(container(line).height(ROW_HEIGHT).center_y(ROW_HEIGHT))
                .padding(0)
                .width(Fill)
                .on_press_maybe(enabled.then_some(choose))
                .style(move |_, status| {
                    let hovered = enabled
                        && matches!(status, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: (selected || hovered).then(|| {
                            theme::color(if selected {
                                colors.selected
                            } else {
                                colors.control
                            })
                            .into()
                        }),
                        border: iced::Border {
                            radius: 4.0.into(),
                            ..iced::Border::default()
                        },
                        text_color: theme::color(tint),
                        ..button::Style::default()
                    }
                });
            lines = lines.push(line);
        }
        let card = if bounds.height < self.size().height {
            card.push(
                scrollable(lines)
                    .id(ROWS)
                    .height(Fill)
                    .on_scroll(move |viewport| {
                        send(Event::Scrolled {
                            instance,
                            y: viewport.absolute_offset().y,
                        })
                    })
                    .style(move |_, status| chrome::scrollbar(definition, status)),
            )
        } else {
            card.push(lines)
        };
        let card = container(card)
            .padding(PADDING)
            .width(bounds.width)
            .height(bounds.height)
            .style(move |_| container::Style {
                border: iced::Border {
                    color: theme::color(colors.dialog_border),
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..chrome::surface(definition, colors.raised, 8.0, true)
            });
        let card = mouse_area(card).on_scroll(move |_| send(Event::Outside { instance }));
        let card = presses(card, move |at, primary| {
            send(Event::Pressed {
                instance,
                at,
                primary,
            })
        });
        (
            opaque(scrim),
            pin(opaque(card)).x(bounds.x).y(bounds.y).into(),
        )
    }
}

/// Where a card of `size` goes for `anchor` in a window of `window`'s
/// size: under it at its left edge, over it when it does not fit below and
/// does above, and inside the window.
fn place(anchor: Rectangle, size: Size, window: Size) -> Point {
    let bottom = window.height - MARGIN;
    let below = anchor.y + anchor.height + GAP;
    let above = anchor.y - GAP - size.height;
    let y = if below + size.height <= bottom || above < MARGIN {
        below
    } else {
        above
    };
    let y = y.min(bottom - size.height).max(MARGIN);
    let x = anchor.x.min(window.width - MARGIN - size.width).max(MARGIN);
    Point::new(x, y)
}

/// `content`, reporting each press over it as it comes (`on_press` with
/// the press's place and whether it is the primary button or a touch),
/// before its own widgets see it, and capturing the release of each press
/// it saw after they have.
fn presses<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
    on_press: impl Fn(Point, bool) -> M + 'a,
) -> Element<'a, M> {
    Element::new(Presses {
        content: content.into(),
        on_press: Box::new(on_press),
    })
}

struct Presses<'a, M> {
    content: Element<'a, M>,
    on_press: Box<dyn Fn(Point, bool) -> M + 'a>,
}

/// The presses over the card not yet released.
#[derive(Default)]
struct Held {
    buttons: Vec<iced::mouse::Button>,
    fingers: Vec<iced::touch::Finger>,
}

impl<M> iced::advanced::Widget<M, iced::Theme, iced::Renderer> for Presses<'_, M> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<Held>()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(Held::default())
    }

    fn children(&self) -> Vec<iced::advanced::widget::Tree> {
        vec![iced::advanced::widget::Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut iced::advanced::widget::Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<iced::Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        renderer: &iced::Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        event: &iced::Event,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        use iced::{mouse, touch};
        let bounds = layout.bounds();
        let held = tree.state.downcast_mut::<Held>();
        let mut release = false;
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                if let Some(at) = cursor.position_over(bounds) {
                    held.buttons.push(*button);
                    shell.publish((self.on_press)(at, *button == mouse::Button::Left));
                }
            }
            iced::Event::Touch(touch::Event::FingerPressed { id, position })
                if bounds.contains(*position) =>
            {
                held.fingers.push(*id);
                shell.publish((self.on_press)(*position, true));
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(button)) => {
                if let Some(at) = held.buttons.iter().position(|held| held == button) {
                    held.buttons.remove(at);
                    release = true;
                }
            }
            iced::Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) => {
                if let Some(at) = held.fingers.iter().position(|held| held == id) {
                    held.fingers.remove(at);
                    release = true;
                }
            }
            _ => {}
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
        if release {
            shell.capture_event();
        }
    }

    fn draw(
        &self,
        tree: &iced::advanced::widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
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
    }

    fn mouse_interaction(
        &self,
        tree: &iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> iced::mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::{Code, NativeCode};
    use wren_editor::Accelerator;

    use super::*;

    fn rows() -> Vec<Row> {
        vec![
            Row::new(1, "Workspace").accelerator(Accelerator::plain('w')),
            Row::new(2, "Section")
                .accelerator(Accelerator::plain('s'))
                .checked(true),
            Row::new(3, "Group")
                .accelerator(Accelerator::plain('g'))
                .enabled(false),
            Row::new(4, "Note").accelerator(Accelerator::shifted('n')),
        ]
    }

    fn menu() -> HostMenu<()> {
        HostMenu::new(
            7,
            (),
            Some("Show tasks in".into()),
            rows(),
            Rectangle::new(Point::new(100.0, 40.0), Size::new(120.0, 24.0)),
        )
    }

    fn named_any<T>(menu: &mut HostMenu<T>, named: Named) -> MenuKey {
        menu.key(
            &Key::Named(named),
            Physical::Unidentified(NativeCode::Unidentified),
            Modifiers::empty(),
            None,
        )
    }

    fn named(menu: &mut HostMenu<()>, named: Named) -> MenuKey {
        menu.key(
            &Key::Named(named),
            Physical::Unidentified(NativeCode::Unidentified),
            Modifiers::empty(),
            None,
        )
    }

    fn typed(menu: &mut HostMenu<()>, c: &str, code: Code, modifiers: Modifiers) -> MenuKey {
        menu.key(
            &Key::Character(c.into()),
            Physical::Code(code),
            modifiers,
            Some(c),
        )
    }

    #[test]
    fn it_opens_on_the_checked_row_and_moves_over_those_that_can_be_chosen() {
        let mut menu = menu();
        assert_eq!(menu.selected(), Some(1));
        assert_eq!(named(&mut menu, Named::ArrowDown), MenuKey::Moved);
        assert_eq!(menu.selected(), Some(3), "Group cannot be chosen");
        assert_eq!(named(&mut menu, Named::ArrowDown), MenuKey::Moved);
        assert_eq!(menu.selected(), Some(0), "round the end");
        assert_eq!(named(&mut menu, Named::ArrowUp), MenuKey::Moved);
        assert_eq!(menu.selected(), Some(3));
        assert_eq!(named(&mut menu, Named::Home), MenuKey::Moved);
        assert_eq!(menu.selected(), Some(0));
        assert_eq!(named(&mut menu, Named::End), MenuKey::Moved);
        assert_eq!(menu.selected(), Some(3));
        assert_eq!(named(&mut menu, Named::Enter), MenuKey::Choose(4));
        // With no checked row, the first that can be chosen.
        let plain = HostMenu::new(
            1,
            (),
            None,
            vec![Row::new(9, "x").enabled(false), Row::new(8, "y")],
            Rectangle::default(),
        );
        assert_eq!(plain.selected(), Some(1));
    }

    #[test]
    fn an_accelerator_chooses_its_row_and_other_keys_are_held() {
        let mut menu = menu();
        assert_eq!(
            typed(&mut menu, "w", Code::KeyW, Modifiers::empty()),
            MenuKey::Choose(1)
        );
        assert_eq!(
            typed(&mut menu, "S", Code::KeyS, Modifiers::SHIFT),
            MenuKey::Held,
            "S is plain"
        );
        assert_eq!(
            typed(&mut menu, "N", Code::KeyN, Modifiers::SHIFT),
            MenuKey::Choose(4)
        );
        assert_eq!(
            typed(&mut menu, "n", Code::KeyN, Modifiers::empty()),
            MenuKey::Held
        );
        // A row that cannot be chosen holds its key, and so does a key no
        // row names, or a shortcut.
        assert_eq!(
            typed(&mut menu, "g", Code::KeyG, Modifiers::empty()),
            MenuKey::Held
        );
        assert_eq!(
            typed(&mut menu, "q", Code::KeyQ, Modifiers::empty()),
            MenuKey::Held
        );
        assert_eq!(
            typed(&mut menu, "w", Code::KeyW, Modifiers::CTRL),
            MenuKey::Held
        );
        assert_eq!(named(&mut menu, Named::Tab), MenuKey::Held);
        // A letter by its place on a keyboard of another script.
        assert_eq!(
            typed(&mut menu, "ц", Code::KeyW, Modifiers::empty()),
            MenuKey::Choose(1)
        );
        assert_eq!(menu.selected(), Some(1), "no key moved it");
    }

    #[test]
    fn a_click_chooses_only_in_its_own_menu() {
        let menu = menu();
        assert_eq!(menu.choose(7, 2), Some(2));
        assert_eq!(menu.choose(6, 2), None, "a stale menu's row");
        assert_eq!(menu.choose(7, 3), None, "a row that cannot be chosen");
        assert_eq!(menu.choose(7, 99), None);
    }

    #[test]
    fn the_card_hangs_under_its_anchor_and_stays_in_the_window() {
        let menu = menu();
        let window = Size::new(1000.0, 800.0);
        let card = menu.bounds(window);
        assert_eq!(card.position(), Point::new(100.0, 40.0 + 24.0 + GAP));
        assert!(card.width >= MIN_WIDTH && card.width <= MAX_WIDTH);
        assert_eq!(
            card.height,
            2.0 * PADDING + TITLE_HEIGHT + 4.0 * ROW_HEIGHT + 2.0
        );
        // At the right edge it moves in; at the bottom it goes over.
        let at = |x, y| {
            HostMenu::new(
                1,
                (),
                None,
                rows(),
                Rectangle::new(Point::new(x, y), Size::new(10.0, 20.0)),
            )
            .bounds(window)
        };
        let right = at(990.0, 40.0);
        assert_eq!(right.x + right.width, window.width - MARGIN);
        let low = at(10.0, 780.0);
        assert_eq!(low.y + low.height, 780.0 - GAP);
        // Taller than the window, it fits in it, and scrolls.
        let many: Vec<Row> = (0..60).map(|id| Row::new(id, "@someone")).collect();
        let mut tall = HostMenu::new(1, (), None, many, Rectangle::default());
        let card = tall.bounds(window);
        assert_eq!(
            (card.y, card.height),
            (MARGIN, window.height - 2.0 * MARGIN)
        );
        assert!(
            menu.clone().reveal(window).is_none(),
            "a short menu does not scroll"
        );
        // It opens at its top, End brings the last row into view, Home the
        // first, and a row already shown moves nothing.
        assert_eq!(tall.reveal(window), Some(0.0));
        let room = card.height - 2.0 * PADDING - 2.0;
        assert_eq!(named_any(&mut tall, Named::End), MenuKey::Moved);
        let end = tall.reveal(window).unwrap();
        assert_eq!(end, 60.0 * ROW_HEIGHT - room);
        assert_eq!(named_any(&mut tall, Named::ArrowUp), MenuKey::Moved);
        assert_eq!(tall.reveal(window), Some(end));
        assert_eq!(named_any(&mut tall, Named::Home), MenuKey::Moved);
        assert_eq!(tall.reveal(window), Some(0.0));
    }
}
