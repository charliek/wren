//! The board's tool dock (discovery/board-controls.md §3.1 and §3.3): a bar
//! floating at the bottom centre of the canvas with a button for each tool
//! and one for the keys sheet. A button runs its `board.*` command through
//! `App::dispatch`, as its key does, so the dock holds no state of the
//! board's: what is lit and locked is the machine's. [`dock`] decides what
//! the dock shows as a pure function; `App::board_dock_view` draws it.

use std::fmt::Write as _;

use iced::widget::{
    button, column, container, mouse_area, row, scrollable, space, stack, text, tooltip, Space,
};
use iced::{Border, Element, Fill};
use wren_board::scene::{Point, Rect, Segment};
use wren_board::{Access, Shape};
use wren_canvas::{Command, Tool};
use wren_core::keybind::Platform;
use wren_core::settings::KeyBadges;
use wren_core::theme::Rgb;

use super::board_keys::{Keys, LETTERED};
use super::commands::{Command as AppCommand, CommandId};
use super::palette::Mode;
use super::{App, Message};
use crate::chrome::{self, Icon};
use crate::{fonts, theme};

/// Below this canvas width the six shape buttons fold into one.
pub const FOLD_WIDTH: f32 = 560.0;
/// A button's size, and the folded shapes button's width.
pub(super) const BUTTON: (f32, f32) = (34.0, 32.0);
pub(super) const WIDE: f32 = 44.0;
/// Between the bar's edge and its buttons, and between two buttons.
pub(super) const PAD: f32 = 4.0;
pub(super) const GAP: f32 = 2.0;
/// A rule between groups of buttons: one pixel with room either side.
pub(super) const RULE: f32 = 7.0;
pub(super) const BORDER: f32 = 1.0;
/// From the bottom of the canvas to the bar, and the least room left
/// beside it.
pub const BOTTOM: f32 = 12.0;
pub(super) const SIDE: f32 = 8.0;
/// How tall the bar is.
pub const HEIGHT: f32 = BUTTON.1 + 2.0 * (PAD + BORDER);
/// How far above the bar a fitted board ends: the canvas's fit margin
/// covers the rest.
pub(super) const CLEAR: f32 = 16.0;
/// A menu's width, and the height of one of its rows.
pub(super) const MENU_WIDTH: f32 = 196.0;
pub(super) const MENU_ROW: f32 = 30.0;

/// The shapes the "more shapes" menu holds: the two with no key.
const MORE: [Shape; 2] = [Shape::Rounded, Shape::Document];

/// One button of the dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Tool(Tool),
    /// Rounded rectangle and Document.
    More,
    /// The six keyed shapes folded into one, on a narrow canvas.
    Shapes,
    /// The keys sheet.
    Keys,
}

/// A menu the dock opens above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// Rounded rectangle and Document.
    More,
    /// All eight shapes, from the folded button.
    Shapes,
}

impl Menu {
    /// What the harness calls it.
    #[cfg(feature = "smoke-test")]
    pub fn name(self) -> &'static str {
        match self {
            Self::More => "more",
            Self::Shapes => "shapes",
        }
    }

    fn shapes(self) -> Vec<Shape> {
        match self {
            Self::More => MORE.to_vec(),
            Self::Shapes => LETTERED.into_iter().chain(MORE).collect(),
        }
    }
}

impl Item {
    /// The command the button runs; a menu button opens its menu instead.
    pub fn command(self) -> Option<Command> {
        match self {
            Self::Tool(tool) => Some(Command::Tool(tool)),
            Self::Keys => Some(Command::Keys),
            Self::More | Self::Shapes => None,
        }
    }

    fn menu(self) -> Option<Menu> {
        match self {
            Self::More => Some(Menu::More),
            Self::Shapes => Some(Menu::Shapes),
            Self::Tool(_) | Self::Keys => None,
        }
    }

    /// What its tooltip calls it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Tool(Tool::Select) => "Select",
            Self::Tool(Tool::Hand) => "Hand",
            Self::Tool(Tool::Text) => "Text",
            Self::Tool(Tool::Connector) => "Connector",
            Self::Tool(Tool::Shape(shape)) => shape_name(shape),
            Self::More => "More shapes",
            Self::Shapes => "Shapes",
            Self::Keys => "Board keys",
        }
    }

    /// The id of the widget around the button, which the harness reads
    /// its place by.
    pub fn id(self) -> &'static str {
        match self {
            Self::Tool(Tool::Select) => "board-dock-select",
            Self::Tool(Tool::Hand) => "board-dock-hand",
            Self::Tool(Tool::Text) => "board-dock-text",
            Self::Tool(Tool::Connector) => "board-dock-connector",
            Self::Tool(Tool::Shape(shape)) => match shape {
                Shape::Rect => "board-dock-rectangle",
                Shape::Rounded => "board-dock-rounded",
                Shape::Pill => "board-dock-pill",
                Shape::Ellipse => "board-dock-ellipse",
                Shape::Diamond => "board-dock-diamond",
                Shape::Parallelogram => "board-dock-parallelogram",
                Shape::Cylinder => "board-dock-cylinder",
                Shape::Document => "board-dock-document",
            },
            Self::More => "board-dock-more",
            Self::Shapes => "board-dock-shapes",
            Self::Keys => "board-dock-keys",
        }
    }
}

/// The id of a menu's row for `shape`.
pub fn row_id(shape: Shape) -> &'static str {
    match shape {
        Shape::Rect => "board-dock-menu-rectangle",
        Shape::Rounded => "board-dock-menu-rounded",
        Shape::Pill => "board-dock-menu-pill",
        Shape::Ellipse => "board-dock-menu-ellipse",
        Shape::Diamond => "board-dock-menu-diamond",
        Shape::Parallelogram => "board-dock-menu-parallelogram",
        Shape::Cylinder => "board-dock-menu-cylinder",
        Shape::Document => "board-dock-menu-document",
    }
}

/// The bar's own id, and its open menu's.
pub const BAR_ID: &str = "board-dock";
pub const MENU_ID: &str = "board-dock-menu";

/// Every id the dock gives a widget, for the harness to look for.
#[cfg(any(test, feature = "smoke-test"))]
pub const IDS: [&str; 25] = [
    BAR_ID,
    MENU_ID,
    "board-dock-select",
    "board-dock-hand",
    "board-dock-rectangle",
    "board-dock-ellipse",
    "board-dock-diamond",
    "board-dock-pill",
    "board-dock-cylinder",
    "board-dock-parallelogram",
    "board-dock-rounded",
    "board-dock-document",
    "board-dock-more",
    "board-dock-shapes",
    "board-dock-text",
    "board-dock-connector",
    "board-dock-keys",
    "board-dock-menu-rectangle",
    "board-dock-menu-ellipse",
    "board-dock-menu-diamond",
    "board-dock-menu-pill",
    "board-dock-menu-cylinder",
    "board-dock-menu-parallelogram",
    "board-dock-menu-rounded",
    "board-dock-menu-document",
];

pub(super) fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Rect => "Rectangle",
        Shape::Rounded => "Rounded rectangle",
        Shape::Pill => "Pill",
        Shape::Ellipse => "Ellipse",
        Shape::Diamond => "Diamond",
        Shape::Parallelogram => "Parallelogram",
        Shape::Cylinder => "Cylinder",
        Shape::Document => "Document",
    }
}

/// One button as the dock shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub item: Item,
    /// The armed tool's button, or one standing for it.
    pub lit: bool,
    /// The armed tool is locked: a dot on the button.
    pub locked: bool,
    /// The shape the button draws, for a shape or a menu standing for one.
    pub shape: Option<Shape>,
    /// The command's first key, from the keymap; `None` when it has none.
    pub key: Option<String>,
    /// Whether the key shows in the button's corner now.
    pub badge: bool,
}

impl Button {
    /// What the tooltip says: the name, then the key.
    pub fn tip(&self) -> String {
        match &self.key {
            Some(key) => format!("{}   {key}", self.item.name()),
            None => self.item.name().to_owned(),
        }
    }

    fn width(&self) -> f32 {
        if self.item == Item::Shapes {
            WIDE
        } else {
            BUTTON.0
        }
    }
}

/// A part of the bar, left to right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Button(Button),
    Rule,
}

impl Part {
    fn width(&self) -> f32 {
        match self {
            Self::Button(button) => button.width(),
            Self::Rule => RULE,
        }
    }
}

/// One row of an open menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub shape: Shape,
    pub lit: bool,
    pub locked: bool,
    pub key: Option<String>,
}

/// The dock as it shows now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dock {
    pub parts: Vec<Part>,
    /// Whether the six shapes are folded into one button.
    pub folded: bool,
    pub menu: Option<(Menu, Vec<MenuRow>)>,
}

impl Dock {
    pub fn buttons(&self) -> impl Iterator<Item = &Button> {
        self.parts.iter().filter_map(|part| match part {
            Part::Button(button) => Some(button),
            Part::Rule => None,
        })
    }

    /// The bar's width, border included.
    pub fn width(&self) -> f32 {
        let parts: f32 = self.parts.iter().map(Part::width).sum();
        let gaps = GAP * self.parts.len().saturating_sub(1) as f32;
        parts + gaps + 2.0 * (PAD + BORDER)
    }

    /// How far the middle of `item`'s button is right of the bar's
    /// middle.
    fn offset(&self, item: Item) -> f32 {
        let mut x = PAD + BORDER;
        for part in &self.parts {
            if matches!(part, Part::Button(button) if button.item == item) {
                return x + part.width() / 2.0 - self.width() / 2.0;
            }
            x += part.width() + GAP;
        }
        0.0
    }
}

/// What the dock reads of the board and the app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Situation {
    /// The canvas's width in logical pixels.
    pub width: f32,
    pub tool: Tool,
    pub locked: bool,
    /// The board may be edited.
    pub writable: bool,
    /// The buttons show their keys now.
    pub badges: bool,
    /// The shape the folded button shows while no shape is armed: the
    /// last one armed.
    pub shape: Shape,
    pub menu: Option<Menu>,
    /// The keys sheet is open.
    pub sheet: bool,
}

/// The dock for `situation`, with each key from `keys`; `None` when the
/// canvas is too narrow to hold it.
pub fn dock(situation: Situation, keys: &Keys) -> Option<Dock> {
    let armed = situation.tool;
    let armed_shape = match armed {
        Tool::Shape(shape) => Some(shape),
        _ => None,
    };
    let button = |item: Item, lit: bool, shape: Option<Shape>| {
        let key = item.command().and_then(|command| keys.first(command));
        Part::Button(Button {
            item,
            lit,
            locked: lit && situation.locked,
            shape,
            badge: situation.badges && key.is_some(),
            key,
        })
    };
    let tool = |tool: Tool| button(Item::Tool(tool), armed == tool, None);
    let folded = situation.width < FOLD_WIDTH;
    let mut parts = vec![tool(Tool::Select), tool(Tool::Hand), Part::Rule];
    if situation.writable {
        if folded {
            let shape = armed_shape.unwrap_or(situation.shape);
            parts.push(button(Item::Shapes, armed_shape.is_some(), Some(shape)));
        } else {
            for shape in LETTERED {
                parts.push(button(
                    Item::Tool(Tool::Shape(shape)),
                    armed_shape == Some(shape),
                    Some(shape),
                ));
            }
            let more = armed_shape.filter(|shape| MORE.contains(shape));
            parts.push(button(Item::More, more.is_some(), more));
        }
        parts.extend([Part::Rule, tool(Tool::Text), tool(Tool::Connector)]);
        parts.push(Part::Rule);
    }
    parts.push(button(Item::Keys, situation.sheet, None));
    let mut dock = Dock {
        parts,
        folded,
        menu: None,
    };
    if dock.width() + 2.0 * SIDE > situation.width {
        return None;
    }
    let menu = situation.menu.filter(|menu| {
        let item = match menu {
            Menu::More => Item::More,
            Menu::Shapes => Item::Shapes,
        };
        dock.buttons().any(|button| button.item == item)
    });
    dock.menu = menu.map(|menu| {
        let rows = menu.shapes().into_iter().map(|shape| {
            let lit = armed_shape == Some(shape);
            MenuRow {
                shape,
                lit,
                locked: lit && situation.locked,
                key: keys.first(Command::Tool(Tool::Shape(shape))),
            }
        });
        (menu, rows.collect())
    });
    Some(dock)
}

/// How many logical pixels along the canvas's bottom a fit leaves for the
/// dock, beyond the fit's own margin.
pub fn covered(shown: bool) -> f32 {
    if shown {
        (BOTTOM + HEIGHT + CLEAR - wren_canvas::FIT_MARGIN).max(0.0)
    } else {
        0.0
    }
}

/// The SVG path elements of `shape`'s outline, as a board draws it at its
/// default proportions, in an 18 by 12 box in the middle of a 24-unit
/// square.
pub fn shape_paths(shape: Shape) -> String {
    // Built at a size whose rounded corner, cylinder cap and document
    // wave are those of a board's shape, then scaled into the icon.
    const SCALE: f32 = 0.2;
    let bounds = Rect::new(0.0, 0.0, 18.0 / SCALE, 12.0 / SCALE);
    let point = |p: Point| format!("{:.2} {:.2}", 3.0 + p.x * SCALE, 6.0 + p.y * SCALE);
    let mut paths = String::new();
    for segments in [
        wren_board::shape::outline(shape, bounds),
        wren_board::shape::detail(shape, bounds),
    ] {
        if segments.is_empty() {
            continue;
        }
        let mut d = String::new();
        for segment in segments {
            let _ = match segment {
                Segment::MoveTo(to) => write!(d, "M{}", point(to)),
                Segment::LineTo(to) => write!(d, "L{}", point(to)),
                Segment::QuadTo { control, to } => write!(d, "Q{} {}", point(control), point(to)),
                Segment::CubicTo { first, second, to } => {
                    write!(d, "C{} {} {}", point(first), point(second), point(to))
                }
                Segment::Close => write!(d, "Z"),
            };
        }
        let _ = write!(paths, "<path d='{d}'/>");
    }
    paths
}

/// The message a click on `item` sends: its command through `dispatch`,
/// or its menu opened, or closed when it is open.
pub fn message(item: Item, open: Option<Menu>) -> Message {
    match (item.command(), item.menu()) {
        (Some(command), _) => Message::Command(AppCommand::Run(CommandId::Board(command))),
        (None, menu) => Message::DockMenu(if open == menu { None } else { menu }),
    }
}

impl App {
    /// The open board's dock, or `None` with `board-toolbar = off`, before
    /// the canvas has a size, or on a canvas too narrow for it.
    pub(super) fn board_dock(&self) -> Option<Dock> {
        let view = self.board.as_ref()?;
        let settings = &self.preferences.settings;
        if !settings.board_toolbar {
            return None;
        }
        let (width, _) = view.size()?;
        let machine = view.machine();
        let badges = self.board_badges();
        let keys = Keys::new(&self.preferences.keymap, Platform::current());
        let sheet = self
            .palette
            .as_ref()
            .is_some_and(|palette| palette.mode == Mode::BoardKeys);
        dock(
            Situation {
                width,
                tool: machine.tool(),
                locked: machine.locked(),
                writable: *view.access() == Access::Editable,
                badges,
                shape: self.dock_shape,
                menu: self.dock_menu,
                sheet,
            },
            &keys,
        )
    }

    /// Whether the board's bars show their keys now: while Space is held,
    /// or always or never as `board-key-badges` says.
    pub(super) fn board_badges(&self) -> bool {
        let held = self
            .board
            .as_ref()
            .is_some_and(|view| view.machine().space());
        match self.preferences.settings.board_key_badges {
            KeyBadges::Hold => held,
            KeyBadges::On => true,
            KeyBadges::Off => false,
        }
    }

    /// What the canvas's fit leaves for the style bar along its top and
    /// for the dock along its bottom: room while the setting shows them.
    pub(super) fn board_covered(&self) -> (f32, f32) {
        let shown = self.preferences.settings.board_toolbar;
        (super::board_style_bar::covered(shown), covered(shown))
    }

    /// The dock over the board, laid over the canvas's bottom centre.
    pub(super) fn board_dock_view(&self) -> Option<Element<'_, Message>> {
        let dock = self.board_dock()?;
        let (width, height) = self.board.as_ref()?.size()?;
        let definition = self.definition;
        let colors = definition.colors;
        let open = dock.menu.as_ref().map(|(menu, _)| *menu);

        let mut bar = row![].spacing(GAP).align_y(iced::Center);
        for part in &dock.parts {
            bar = bar.push(match part {
                Part::Rule => container(
                    container(Space::new())
                        .width(1)
                        .height(BUTTON.1 - 8.0)
                        .style(move |_| chrome::surface(definition, colors.divider, 0.0, false)),
                )
                .padding([0.0, (RULE - 1.0) / 2.0])
                .into(),
                Part::Button(each) => self.dock_button(each, open),
            });
        }
        // A border takes no room of its own in iced, so the padding holds it.
        let bar = container(bar)
            .id(BAR_ID)
            .padding(PAD + BORDER)
            .style(move |_| panel(definition));
        // The bar's own background takes a press, so none reaches the
        // canvas beneath it; it closes an open menu. Any button's press:
        // one the bar let through would reach the board as a click off
        // it, which takes the keyboard from the board.
        let bar = mouse_area(bar)
            .on_press(Message::DockMenu(None))
            .on_right_press(Message::DockMenu(None))
            .on_middle_press(Message::DockMenu(None))
            .interaction(iced::mouse::Interaction::Idle);

        let mut layer = column![].spacing(10).align_x(iced::Center);
        if let Some((menu, rows)) = &dock.menu {
            let anchor = match menu {
                Menu::More => Item::More,
                Menu::Shapes => Item::Shapes,
            };
            // Over its button, and inside the canvas.
            let room = ((width - MENU_WIDTH) / 2.0 - SIDE).max(0.0);
            let offset = dock.offset(anchor).clamp(-room, room);
            let mut list = column![].spacing(1);
            for each in rows {
                list = list.push(self.dock_menu_row(each));
            }
            // In a canvas too short for every row the menu scrolls.
            let bar_height = BUTTON.1 + 2.0 * (PAD + BORDER);
            let room = (height - BOTTOM - bar_height - 10.0 - SIDE).max(MENU_ROW);
            let list = mouse_area(
                container(
                    scrollable(list).style(move |_, status| chrome::scrollbar(definition, status)),
                )
                .id(MENU_ID)
                .width(MENU_WIDTH)
                .max_height(room)
                .padding(PAD)
                .style(move |_| panel(definition)),
            )
            .on_press(Message::DockMenu(Some(*menu)))
            .on_right_press(Message::DockMenu(Some(*menu)))
            .on_middle_press(Message::DockMenu(Some(*menu)))
            .interaction(iced::mouse::Interaction::Idle);
            let shift = Space::new().width(2.0 * offset.abs());
            layer = layer.push(if offset >= 0.0 {
                row![shift, list]
            } else {
                row![list, shift]
            });
        }
        layer = layer.push(bar);
        Some(
            container(layer)
                .width(Fill)
                .height(Fill)
                .align_x(iced::Center)
                .align_y(iced::Bottom)
                .padding(iced::Padding {
                    bottom: BOTTOM,
                    ..iced::Padding::ZERO
                })
                .into(),
        )
    }

    fn dock_button<'a>(&self, each: &Button, open: Option<Menu>) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let ink = if each.lit { colors.accent } else { colors.text };
        let glyph = |icon| chrome::icon(icon, 20.0, ink);
        let shape = |shape| chrome::paths_icon(&shape_paths(shape), 20.0, ink);
        let icon: Element<'a, Message> = match (each.item, each.shape) {
            (Item::Tool(Tool::Select), _) => glyph(Icon::Select),
            (Item::Tool(Tool::Hand), _) => glyph(Icon::Hand),
            (Item::Tool(Tool::Text), _) => glyph(Icon::Text),
            (Item::Tool(Tool::Connector), _) => glyph(Icon::Connector),
            (Item::Shapes, Some(drawn)) => {
                row![shape(drawn), chrome::icon(Icon::Chevron, 10.0, ink)]
                    .align_y(iced::Center)
                    .into()
            }
            (_, Some(drawn)) => shape(drawn),
            (Item::More, None) => glyph(Icon::More),
            (_, None) => text("?")
                .size(15)
                .font(fonts::SEMIBOLD)
                .color(theme::color(ink))
                .into(),
        };
        let mut face = stack![container(icon).center(Fill)];
        if each.locked {
            face = face.push(
                container(
                    container(Space::new())
                        .width(5)
                        .height(5)
                        .style(move |_| dot(colors.accent)),
                )
                .padding([3, 3]),
            );
        }
        if let Some(key) = each.key.clone().filter(|_| each.badge) {
            face =
                face.push(
                    container(text(key).size(10).font(fonts::UI).color(theme::color(
                        if each.lit {
                            colors.accent
                        } else {
                            colors.muted
                        },
                    )))
                    .width(Fill)
                    .height(Fill)
                    .align_x(iced::Right)
                    .align_y(iced::Bottom)
                    .padding(iced::Padding {
                        right: 3.0,
                        bottom: 1.0,
                        ..iced::Padding::ZERO
                    }),
                );
        }
        let lit = each.lit;
        let pressed = button(face)
            .width(each.width())
            .height(BUTTON.1)
            .padding(0)
            .style(move |_, status| tool_button(definition, status, lit))
            .on_press(message(each.item, open));
        let placed = container(pressed).id(each.item.id());
        // An open menu says what its button holds.
        if each.item.menu().is_some() && each.item.menu() == open {
            return placed.into();
        }
        tooltip(
            placed,
            container(
                text(each.tip())
                    .size(11)
                    .font(fonts::UI)
                    .color(theme::color(colors.editor)),
            )
            .padding([4, 7])
            .style(move |_| chrome::surface(definition, colors.text, 5.0, false)),
            tooltip::Position::Top,
        )
        .gap(9)
        .into()
    }

    fn dock_menu_row<'a>(&self, each: &MenuRow) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let ink = if each.lit { colors.accent } else { colors.body };
        let mut line = row![
            chrome::paths_icon(&shape_paths(each.shape), 18.0, ink),
            text(shape_name(each.shape))
                .size(13)
                .font(if each.lit { fonts::SEMIBOLD } else { fonts::UI })
                .color(theme::color(ink)),
            space::horizontal(),
        ]
        .spacing(9)
        .align_y(iced::Center);
        if each.locked {
            line = line.push(
                container(Space::new())
                    .width(5)
                    .height(5)
                    .style(move |_| dot(colors.accent)),
            );
        }
        if let Some(key) = &each.key {
            line = line.push(
                text(key.clone())
                    .size(11)
                    .font(fonts::UI)
                    .color(theme::color(colors.faint)),
            );
        }
        let command = Command::Tool(Tool::Shape(each.shape));
        container(
            button(line)
                .width(Fill)
                .height(MENU_ROW)
                .padding([5, 8])
                .style(move |_, status| tool_button(definition, status, false))
                .on_press(Message::Command(AppCommand::Run(CommandId::Board(command)))),
        )
        .id(row_id(each.shape))
        .into()
    }
}

/// The bar's and a menu's frame: raised, with the dialog's border.
pub(super) fn panel(definition: wren_core::theme::Theme) -> container::Style {
    container::Style {
        border: Border {
            color: theme::color(definition.colors.dialog_border),
            width: BORDER,
            radius: 10.0.into(),
        },
        ..chrome::surface(definition, definition.colors.raised, 10.0, true)
    }
}

/// A locked tool's dot.
fn dot(color: Rgb) -> container::Style {
    container::Style {
        background: Some(theme::color(color).into()),
        border: Border {
            radius: 2.5.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// A dock button: clear until the pointer is on it, and lit in the
/// selection's colours while its tool is armed.
pub(super) fn tool_button(
    definition: wren_core::theme::Theme,
    status: button::Status,
    lit: bool,
) -> button::Style {
    let colors = definition.colors;
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: if lit {
            Some(theme::color(colors.selected).into())
        } else if hovered {
            Some(theme::color(colors.control).into())
        } else {
            None
        },
        text_color: theme::color(colors.text),
        border: Border {
            color: if lit {
                theme::color(colors.selected_border)
            } else {
                iced::Color::TRANSPARENT
            },
            width: if lit { 1.0 } else { 0.0 },
            radius: 7.0.into(),
        },
        ..button::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::board::tests::{fixture, open_board};
    use super::super::board_keys::tests::{focus, keymap};
    use super::*;

    const WIDE_IDS: [&str; 12] = [
        "board-dock-select",
        "board-dock-hand",
        "board-dock-rectangle",
        "board-dock-ellipse",
        "board-dock-diamond",
        "board-dock-pill",
        "board-dock-cylinder",
        "board-dock-parallelogram",
        "board-dock-more",
        "board-dock-text",
        "board-dock-connector",
        "board-dock-keys",
    ];

    const IDLE: Situation = Situation {
        width: 800.0,
        tool: Tool::Select,
        locked: false,
        writable: true,
        badges: false,
        shape: Shape::Rect,
        menu: None,
        sheet: false,
    };

    fn docked(situation: Situation) -> Option<Dock> {
        let keymap = keymap(Platform::Linux, &[]);
        dock(situation, &Keys::new(&keymap, Platform::Linux))
    }

    fn ids(dock: &Dock) -> Vec<&'static str> {
        dock.buttons().map(|button| button.item.id()).collect()
    }

    fn lit(dock: &Dock) -> Vec<&'static str> {
        let lit = dock.buttons().filter(|button| button.lit);
        lit.map(|button| button.item.id()).collect()
    }

    fn find(dock: &Dock, item: Item) -> &Button {
        dock.buttons().find(|button| button.item == item).unwrap()
    }

    #[test]
    fn a_wide_canvas_docks_every_tool_in_its_groups() {
        let dock = docked(IDLE).unwrap();
        assert_eq!(ids(&dock), WIDE_IDS);
        assert!(!dock.folded && dock.menu.is_none());
        // Select and Hand, the shapes, Text and Connector, the sheet.
        let rules = dock
            .parts
            .iter()
            .filter(|part| **part == Part::Rule)
            .count();
        assert_eq!(rules, 3);
        assert_eq!(lit(&dock), ["board-dock-select"]);
        let rect = find(&dock, Item::Tool(Tool::Shape(Shape::Rect)));
        assert_eq!(rect.shape, Some(Shape::Rect));
        assert_eq!(rect.tip(), "Rectangle   R");
        assert_eq!(
            find(&dock, Item::Tool(Tool::Connector)).key.as_deref(),
            Some("C")
        );
        assert_eq!(find(&dock, Item::Keys).key.as_deref(), Some("?"));
        assert_eq!(find(&dock, Item::More).tip(), "More shapes");
        assert!(dock.width() < FOLD_WIDTH - 2.0 * SIDE);
        // Every id the dock gives is one the harness looks for.
        for button in dock.buttons() {
            assert!(IDS.contains(&button.item.id()), "{:?}", button.item);
        }
    }

    #[test]
    fn the_armed_tool_is_lit_and_its_lock_is_shown() {
        let diamond = Tool::Shape(Shape::Diamond);
        let armed = |tool, locked| {
            docked(Situation {
                tool,
                locked,
                ..IDLE
            })
            .unwrap()
        };
        let dock = armed(diamond, false);
        assert_eq!(lit(&dock), ["board-dock-diamond"]);
        assert!(dock.buttons().all(|button| !button.locked));
        let dock = armed(diamond, true);
        let locked: Vec<_> = dock.buttons().filter(|button| button.locked).collect();
        assert_eq!(locked.len(), 1);
        assert_eq!(locked[0].item, Item::Tool(diamond));
        // A shape of the "more" menu lights that button, which shows it.
        let dock = armed(Tool::Shape(Shape::Document), true);
        let more = find(&dock, Item::More);
        assert!(more.lit && more.locked);
        assert_eq!(more.shape, Some(Shape::Document));
        assert_eq!(find(&armed(Tool::Select, false), Item::More).shape, None);
        for tool in [Tool::Hand, Tool::Text, Tool::Connector] {
            assert_eq!(lit(&armed(tool, false)), [Item::Tool(tool).id()]);
        }
        // The keys sheet open lights its button.
        let dock = docked(Situation {
            sheet: true,
            ..IDLE
        })
        .unwrap();
        assert!(find(&dock, Item::Keys).lit);
    }

    #[test]
    fn badges_show_the_bound_keys_only_when_asked() {
        let quiet = docked(IDLE).unwrap();
        assert!(quiet.buttons().all(|button| !button.badge));
        let shown = docked(Situation {
            badges: true,
            ..IDLE
        })
        .unwrap();
        for button in shown.buttons() {
            assert_eq!(button.badge, button.key.is_some(), "{:?}", button.item);
        }
        assert!(!find(&shown, Item::More).badge);
        // A rebound key is the one shown, and an unbound tool shows none.
        let keymap = keymap(
            Platform::Linux,
            &[
                ("k", "board.tool.ellipse"),
                ("o", "board.unbind"),
                ("p", "board.unbind"),
            ],
        );
        let rebound = dock(
            Situation {
                badges: true,
                ..IDLE
            },
            &Keys::new(&keymap, Platform::Linux),
        )
        .unwrap();
        let ellipse = find(&rebound, Item::Tool(Tool::Shape(Shape::Ellipse)));
        assert_eq!((ellipse.key.as_deref(), ellipse.badge), (Some("K"), true));
        let parallelogram = find(&rebound, Item::Tool(Tool::Shape(Shape::Parallelogram)));
        assert_eq!(
            (parallelogram.key.as_deref(), parallelogram.badge),
            (None, false)
        );
        assert_eq!(parallelogram.tip(), "Parallelogram");
    }

    #[test]
    fn a_narrow_canvas_folds_the_shapes_and_a_tiny_one_has_no_dock() {
        let narrow = Situation {
            width: FOLD_WIDTH - 1.0,
            shape: Shape::Cylinder,
            ..IDLE
        };
        let dock = docked(narrow).unwrap();
        assert!(dock.folded);
        assert_eq!(
            ids(&dock),
            [
                "board-dock-select",
                "board-dock-hand",
                "board-dock-shapes",
                "board-dock-text",
                "board-dock-connector",
                "board-dock-keys"
            ]
        );
        // It shows the shape last armed, or the one armed now, lit.
        let folded = find(&dock, Item::Shapes);
        assert_eq!((folded.shape, folded.lit), (Some(Shape::Cylinder), false));
        assert_eq!(folded.key, None);
        let armed = docked(Situation {
            tool: Tool::Shape(Shape::Pill),
            locked: true,
            ..narrow
        })
        .unwrap();
        let folded = find(&armed, Item::Shapes);
        assert_eq!(
            (folded.shape, folded.lit, folded.locked),
            (Some(Shape::Pill), true, true)
        );
        // Its menu holds all eight, keyed ones first.
        let menu = docked(Situation {
            menu: Some(Menu::Shapes),
            tool: Tool::Shape(Shape::Pill),
            ..narrow
        })
        .unwrap()
        .menu
        .unwrap();
        assert_eq!(menu.0, Menu::Shapes);
        let shapes: Vec<Shape> = menu.1.iter().map(|row| row.shape).collect();
        assert_eq!(&shapes[..6], LETTERED);
        assert_eq!(&shapes[6..], MORE);
        let lit: Vec<Shape> = menu
            .1
            .iter()
            .filter(|row| row.lit)
            .map(|row| row.shape)
            .collect();
        assert_eq!(lit, [Shape::Pill]);
        assert_eq!(menu.1[0].key.as_deref(), Some("R"));
        assert_eq!(menu.1[7].key, None);
        // A menu whose button is not shown is not open: the folded menu on
        // a wide canvas, the "more" menu on a narrow one.
        let wide = docked(Situation {
            menu: Some(Menu::Shapes),
            ..IDLE
        })
        .unwrap();
        assert_eq!(wide.menu, None);
        let folded = docked(Situation {
            menu: Some(Menu::More),
            ..narrow
        })
        .unwrap();
        assert_eq!(folded.menu, None);
        // Never wider than the canvas.
        let least = dock.width() + 2.0 * SIDE;
        assert!(docked(Situation {
            width: least,
            ..narrow
        })
        .is_some());
        assert_eq!(
            docked(Situation {
                width: least - 1.0,
                ..narrow
            }),
            None
        );
    }

    #[test]
    fn a_read_only_board_docks_select_hand_and_the_keys() {
        let dock = docked(Situation {
            writable: false,
            ..IDLE
        })
        .unwrap();
        assert_eq!(
            ids(&dock),
            ["board-dock-select", "board-dock-hand", "board-dock-keys"]
        );
        let more = docked(Situation {
            writable: false,
            menu: Some(Menu::More),
            ..IDLE
        })
        .unwrap();
        assert_eq!(more.menu, None);
    }

    #[test]
    fn the_more_menu_holds_the_shapes_with_no_key() {
        let menu = docked(Situation {
            menu: Some(Menu::More),
            tool: Tool::Shape(Shape::Rounded),
            locked: true,
            ..IDLE
        })
        .unwrap()
        .menu
        .unwrap();
        assert_eq!(menu.0, Menu::More);
        assert_eq!(
            menu.1,
            [
                MenuRow {
                    shape: Shape::Rounded,
                    lit: true,
                    locked: true,
                    key: None
                },
                MenuRow {
                    shape: Shape::Document,
                    lit: false,
                    locked: false,
                    key: None
                },
            ]
        );
    }

    #[test]
    fn a_button_sends_its_command_and_a_menu_button_toggles_its_menu() {
        let rect = Tool::Shape(Shape::Rect);
        assert!(matches!(
            message(Item::Tool(rect), None),
            Message::Command(AppCommand::Run(CommandId::Board(Command::Tool(tool)))) if tool == rect
        ));
        // A command button leaves an open menu to the command, which
        // closes it.
        assert!(matches!(
            message(Item::Keys, Some(Menu::More)),
            Message::Command(AppCommand::Run(CommandId::Board(Command::Keys)))
        ));
        assert!(matches!(
            message(Item::More, None),
            Message::DockMenu(Some(Menu::More))
        ));
        assert!(matches!(
            message(Item::More, Some(Menu::More)),
            Message::DockMenu(None)
        ));
        assert!(matches!(
            message(Item::Shapes, Some(Menu::More)),
            Message::DockMenu(Some(Menu::Shapes))
        ));
    }

    #[test]
    fn every_shape_icon_is_its_outline_in_the_icon_box() {
        for shape in LETTERED.into_iter().chain(MORE) {
            let paths = shape_paths(shape);
            let count = paths.matches("<path ").count();
            assert_eq!(
                count,
                if shape == Shape::Cylinder { 2 } else { 1 },
                "{shape:?}"
            );
            assert!(paths.starts_with("<path d='M"), "{shape:?}");
            let numbers = paths
                .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                .filter_map(|word| word.parse::<f32>().ok());
            for number in numbers {
                assert!((3.0..=21.0).contains(&number), "{shape:?}: {number}");
            }
        }
        // The outlines are the board's own, so no two shapes share one.
        let all: std::collections::HashSet<String> =
            LETTERED.into_iter().chain(MORE).map(shape_paths).collect();
        assert_eq!(all.len(), 8);
    }

    #[test]
    fn a_fit_leaves_room_for_the_dock_only_while_it_shows() {
        assert_eq!(covered(false), 0.0);
        assert!(covered(true) > 0.0);
        assert_eq!(
            covered(true) + wren_canvas::FIT_MARGIN,
            BOTTOM + HEIGHT + CLEAR
        );
    }

    #[test]
    fn the_app_docks_the_open_board_and_its_clicks_run_its_commands() {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        // No size yet, no dock.
        assert!(app.board_dock().is_none());
        assert!(app.board_style().tool_chip);
        focus(&mut app);
        let dock = app.board_dock().unwrap();
        assert_eq!(ids(&dock), WIDE_IDS);
        assert!(
            !app.board_style().tool_chip,
            "the dock names the tool instead"
        );

        // A click arms its tool as the key does, and again locks it; the
        // board keeps the keyboard and nothing is written.
        let rect = Item::Tool(Tool::Shape(Shape::Rect));
        let _ = app.update(message(rect, None));
        let machine = app.board.as_ref().unwrap().machine();
        assert_eq!(
            (machine.tool(), machine.locked()),
            (Tool::Shape(Shape::Rect), false)
        );
        assert!(machine.focused());
        let _ = app.update(message(rect, None));
        assert!(app.board.as_ref().unwrap().machine().locked());
        assert!(find(&app.board_dock().unwrap(), rect).locked);

        // A menu opens and closes, and a row's command closes it.
        let _ = app.update(message(Item::More, None));
        assert_eq!(app.board_dock().unwrap().menu.unwrap().0, Menu::More);
        let _ = app.update(Message::Escape);
        assert!(app.board_dock().unwrap().menu.is_none());
        let _ = app.update(message(Item::More, None));
        let _ = app.update(message(
            Item::Tool(Tool::Shape(Shape::Document)),
            Some(Menu::More),
        ));
        assert!(app.dock_menu.is_none());
        assert!(find(&app.board_dock().unwrap(), Item::More).lit);
        assert_eq!(app.dock_shape, Shape::Document);

        // The settings: badges on, then the dock off.
        app.preferences.settings.board_key_badges = KeyBadges::On;
        assert!(find(&app.board_dock().unwrap(), Item::Keys).badge);
        app.preferences.settings.board_key_badges = KeyBadges::Off;
        assert!(!find(&app.board_dock().unwrap(), Item::Keys).badge);
        app.preferences.settings.board_toolbar = false;
        assert!(app.board_dock().is_none());
        assert!(app.board_style().tool_chip);
        assert_eq!(app.board_covered(), (0.0, 0.0));
    }
}
