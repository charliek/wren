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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The row `row` was clicked in the menu `instance`.
    Choose { instance: u64, row: u32 },
    /// A press, or a scroll, outside the card.
    Outside { instance: u64 },
}

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
        }
    }

    pub fn instance(&self) -> u64 {
        self.instance
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
        send: fn(Event) -> M,
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
                    .height(Fill)
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
        let tall = HostMenu::new(1, (), None, many, Rectangle::default()).bounds(window);
        assert_eq!(
            (tall.y, tall.height),
            (MARGIN, window.height - 2.0 * MARGIN)
        );
    }
}
