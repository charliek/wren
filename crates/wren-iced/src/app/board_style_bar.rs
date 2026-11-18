//! The board's style bar (discovery/board-controls.md §3.2 and §3.3): a bar
//! fixed at the top centre of the canvas while something is selected, with
//! the controls that style it. What it offers is what the actions palette
//! offers, `BoardView::offers`, so the two never disagree; a control runs
//! its `board.*` command through `App::dispatch`, as its key does. With
//! nothing selected and a shape or text tool armed it shows the colour new
//! shapes take. [`bar`] decides what the bar shows as a pure function;
//! `App::board_style_bar_view` draws it.

use iced::widget::{
    button, column, container, mouse_area, row, scrollable, space, stack, text, tooltip, Space,
};
use iced::{Border, Element};
use wren_board::edit::Stack;
use wren_board::{style, Color, Fill, Shape, Token, Vocabulary};
use wren_canvas::{Command, Offer, Tool};
use wren_core::theme::Rgb;

use super::board_dock::{
    panel, shape_name, shape_paths, tool_button, BORDER, BUTTON, GAP, MENU_ROW, MENU_WIDTH, PAD,
    RULE, SIDE, WIDE,
};
use super::board_keys::{Keys, LETTERED};
use super::commands::{Command as AppCommand, CommandId};
use super::{App, Message};
use crate::chrome::{self, Icon};
use crate::{fonts, theme};

/// A swatch's size, and the dot it draws.
const SWATCH: (f32, f32) = (25.0, 32.0);
const DOT: f32 = 15.0;
/// The ring round the current swatch: its width and the gap inside it.
const RING: f32 = 2.0;
const RING_GAP: f32 = 2.0;
/// The dot and its ring's gap while the swatch shows its digit too.
const DOT_BADGED: f32 = 11.0;
const RING_GAP_BADGED: f32 = 1.5;
/// The "New shapes" label's width.
const LABEL: f32 = 80.0;
/// From the top of the canvas to the bar.
pub const TOP: f32 = 12.0;
/// How tall the bar is.
pub const HEIGHT: f32 = BUTTON.1 + 2.0 * (PAD + BORDER);

/// One control of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// The shape menu's button.
    Shapes,
    Swatch(Token),
    Fill(Fill),
    Dashed,
    Route,
    StartHead,
    EndHead,
    Reverse,
    /// The last menu's button.
    More,
}

/// A menu the bar opens beneath it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// The eight shapes, from the shape button.
    Shapes,
    /// The edits and "All actions…", and on a narrow canvas the controls
    /// the bar folds away.
    More,
}

impl Menu {
    /// What the harness calls it.
    #[cfg(feature = "smoke-test")]
    pub fn name(self) -> &'static str {
        match self {
            Self::Shapes => "shapes",
            Self::More => "more",
        }
    }

    fn anchor(self) -> Item {
        match self {
            Self::Shapes => Item::Shapes,
            Self::More => Item::More,
        }
    }
}

impl Item {
    /// The command the control runs; a menu button opens its menu instead.
    pub fn command(self) -> Option<Command> {
        Some(match self {
            Self::Swatch(token) => Command::Color(token),
            Self::Fill(fill) => Command::Fill(fill),
            Self::Dashed => Command::StrokeDashed,
            Self::Route => Command::EdgeRouting,
            Self::StartHead => Command::EdgeStart,
            Self::EndHead => Command::EdgeEnd,
            Self::Reverse => Command::EdgeReverse,
            Self::Shapes | Self::More => return None,
        })
    }

    fn menu(self) -> Option<Menu> {
        match self {
            Self::Shapes => Some(Menu::Shapes),
            Self::More => Some(Menu::More),
            _ => None,
        }
    }

    /// What its tooltip calls it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Shapes => "Change shape",
            Self::More => "More",
            Self::Swatch(token) => colour_name(token),
            Self::Fill(_)
            | Self::Dashed
            | Self::Route
            | Self::StartHead
            | Self::EndHead
            | Self::Reverse => self.command().map_or("", command_name),
        }
    }

    /// The id of the widget around the control.
    pub fn id(self) -> &'static str {
        match self {
            Self::Shapes => "board-style-shape",
            Self::More => "board-style-more",
            Self::Swatch(token) => match token {
                Token::Neutral => "board-style-neutral",
                Token::Red => "board-style-red",
                Token::Orange => "board-style-orange",
                Token::Yellow => "board-style-yellow",
                Token::Green => "board-style-green",
                Token::Cyan => "board-style-cyan",
                Token::Purple => "board-style-purple",
                Token::Blue => "board-style-blue",
            },
            Self::Fill(Fill::None) => "board-style-fill-none",
            Self::Fill(Fill::Tint) => "board-style-fill-tint",
            Self::Fill(Fill::Solid) => "board-style-fill-solid",
            Self::Dashed => "board-style-dashed",
            Self::Route => "board-style-route",
            Self::StartHead => "board-style-start",
            Self::EndHead => "board-style-end",
            Self::Reverse => "board-style-reverse",
        }
    }

    fn width(self) -> f32 {
        match self {
            Self::Shapes => WIDE,
            Self::Swatch(_) => SWATCH.0,
            _ => BUTTON.0,
        }
    }
}

/// Whether the pointer over a control or a row running `command` shows it
/// on the board first: a colour, a fill, a route or a shape.
pub fn previews(command: Command) -> bool {
    matches!(
        command,
        Command::Color(_) | Command::Fill(_) | Command::EdgeRouting | Command::Shape(_)
    )
}

fn colour_name(token: Token) -> &'static str {
    match token {
        Token::Neutral => "Neutral",
        Token::Red => "Red",
        Token::Orange => "Orange",
        Token::Yellow => "Yellow",
        Token::Green => "Green",
        Token::Cyan => "Cyan",
        Token::Purple => "Purple",
        Token::Blue => "Blue",
    }
}

/// What a control or a menu row running `command` is called.
fn command_name(command: Command) -> &'static str {
    match command {
        Command::Shape(shape) => shape_name(shape),
        Command::Color(token) => colour_name(token),
        Command::Fill(Fill::None) => "Fill: none",
        Command::Fill(Fill::Tint) => "Fill: tint",
        Command::Fill(Fill::Solid) => "Fill: solid",
        Command::StrokeDashed => "Solid or dashed",
        Command::EdgeRouting => "Elbow or straight",
        Command::EdgeStart => "Start arrowhead",
        Command::EdgeEnd => "End arrowhead",
        Command::EdgeReverse => "Reverse",
        Command::Duplicate => "Duplicate",
        Command::Arrange(Stack::Front) => "Bring to front",
        Command::Arrange(Stack::Back) => "Send to back",
        Command::Delete => "Delete",
        Command::Actions => "All actions…",
        _ => "",
    }
}

/// The id of a menu's row running `command`.
pub fn row_id(command: Command) -> &'static str {
    match command {
        Command::Shape(Shape::Rect) => "board-style-menu-rectangle",
        Command::Shape(Shape::Rounded) => "board-style-menu-rounded",
        Command::Shape(Shape::Pill) => "board-style-menu-pill",
        Command::Shape(Shape::Ellipse) => "board-style-menu-ellipse",
        Command::Shape(Shape::Diamond) => "board-style-menu-diamond",
        Command::Shape(Shape::Parallelogram) => "board-style-menu-parallelogram",
        Command::Shape(Shape::Cylinder) => "board-style-menu-cylinder",
        Command::Shape(Shape::Document) => "board-style-menu-document",
        Command::Fill(Fill::None) => "board-style-menu-fill-none",
        Command::Fill(Fill::Tint) => "board-style-menu-fill-tint",
        Command::Fill(Fill::Solid) => "board-style-menu-fill-solid",
        Command::StrokeDashed => "board-style-menu-dashed",
        Command::EdgeRouting => "board-style-menu-route",
        Command::EdgeStart => "board-style-menu-start",
        Command::EdgeEnd => "board-style-menu-end",
        Command::EdgeReverse => "board-style-menu-reverse",
        Command::Duplicate => "board-style-menu-duplicate",
        Command::Arrange(Stack::Front) => "board-style-menu-front",
        Command::Arrange(Stack::Back) => "board-style-menu-back",
        Command::Delete => "board-style-menu-delete",
        _ => "board-style-menu-actions",
    }
}

/// The bar's own id, and its open menu's.
pub const BAR_ID: &str = "board-style";
pub const MENU_ID: &str = "board-style-menu";

/// Every id the bar gives a widget, for the harness to look for.
#[cfg(any(test, feature = "smoke-test"))]
pub const IDS: [&str; 41] = [
    BAR_ID,
    MENU_ID,
    "board-style-shape",
    "board-style-more",
    "board-style-neutral",
    "board-style-red",
    "board-style-orange",
    "board-style-yellow",
    "board-style-green",
    "board-style-cyan",
    "board-style-purple",
    "board-style-blue",
    "board-style-fill-none",
    "board-style-fill-tint",
    "board-style-fill-solid",
    "board-style-dashed",
    "board-style-route",
    "board-style-start",
    "board-style-end",
    "board-style-reverse",
    "board-style-menu-rectangle",
    "board-style-menu-ellipse",
    "board-style-menu-diamond",
    "board-style-menu-pill",
    "board-style-menu-cylinder",
    "board-style-menu-parallelogram",
    "board-style-menu-rounded",
    "board-style-menu-document",
    "board-style-menu-fill-none",
    "board-style-menu-fill-tint",
    "board-style-menu-fill-solid",
    "board-style-menu-dashed",
    "board-style-menu-route",
    "board-style-menu-start",
    "board-style-menu-end",
    "board-style-menu-reverse",
    "board-style-menu-duplicate",
    "board-style-menu-front",
    "board-style-menu-back",
    "board-style-menu-delete",
    "board-style-menu-actions",
];

/// One control as the bar shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub item: Item,
    /// It names what the whole selection already has: the swatch is
    /// ringed, the fill lit. For the shape button, whether every selected
    /// shape is one shape, which it draws.
    pub current: bool,
    /// The shape the shape button draws: the selection's, or `None` for a
    /// selection of several shapes.
    pub shape: Option<Shape>,
    /// The command's first key, from the keymap.
    pub key: Option<String>,
    /// Whether the key shows on the control now.
    pub badge: bool,
}

impl Control {
    /// What the tooltip says: the name, then the key.
    pub fn tip(&self) -> String {
        match &self.key {
            Some(key) => format!("{}   {key}", self.item.name()),
            None => self.item.name().to_owned(),
        }
    }
}

/// A part of the bar, left to right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// "New shapes", before the colours of the next shape.
    Label,
    Control(Control),
    Rule,
}

impl Part {
    fn width(&self) -> f32 {
        match self {
            Self::Label => LABEL,
            Self::Control(control) => control.item.width(),
            Self::Rule => RULE,
        }
    }
}

/// One row of an open menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub command: Command,
    pub current: bool,
    pub key: Option<String>,
}

impl MenuRow {
    pub fn name(&self) -> &'static str {
        command_name(self.command)
    }
}

/// The bar as it shows now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bar {
    pub parts: Vec<Part>,
    /// Whether the fill, outline and connector controls are folded into
    /// the last menu, on a canvas too narrow for them.
    pub folded: bool,
    pub menu: Option<(Menu, Vec<MenuRow>)>,
}

impl Bar {
    pub fn controls(&self) -> impl Iterator<Item = &Control> {
        self.parts.iter().filter_map(|part| match part {
            Part::Control(control) => Some(control),
            _ => None,
        })
    }

    /// Whether it is the row of the colours new shapes take.
    #[cfg(any(test, feature = "smoke-test"))]
    pub fn new_shapes(&self) -> bool {
        self.parts.first() == Some(&Part::Label)
    }

    /// The bar's width, border included.
    pub fn width(&self) -> f32 {
        let parts: f32 = self.parts.iter().map(Part::width).sum();
        let gaps = GAP * self.parts.len().saturating_sub(1) as f32;
        parts + gaps + 2.0 * (PAD + BORDER)
    }

    /// How far the middle of `item`'s control is right of the bar's
    /// middle.
    fn offset(&self, item: Item) -> f32 {
        let mut x = PAD + BORDER;
        for part in &self.parts {
            if matches!(part, Part::Control(control) if control.item == item) {
                return x + part.width() / 2.0 - self.width() / 2.0;
            }
            x += part.width() + GAP;
        }
        0.0
    }

    /// The command a control or an open menu's row shows, and whether it
    /// is the current value.
    pub fn shows(&self, command: Command) -> Option<bool> {
        let control = self
            .controls()
            .find(|control| control.item.command() == Some(command))
            .map(|control| control.current);
        let row = self.menu.iter().flat_map(|(_, rows)| rows);
        control.or_else(|| {
            row.clone()
                .find(|row| row.command == command)
                .map(|row| row.current)
        })
    }
}

/// What the bar reads of the board and the app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Situation {
    /// The canvas's width in logical pixels.
    pub width: f32,
    /// The board may be edited.
    pub writable: bool,
    /// Anything the board holds is selected.
    pub selected: bool,
    pub tool: Tool,
    /// The colour new shapes take.
    pub next_color: Token,
    /// The controls show their keys now.
    pub badges: bool,
    pub menu: Option<Menu>,
}

/// The bar for `situation`, built from what the actions palette `offers`
/// for the selection, with each key from `keys`; `None` when it is hidden
/// or the canvas is too narrow for even its colours.
pub fn bar(situation: Situation, offers: &[Offer], keys: &Keys) -> Option<Bar> {
    if !situation.writable {
        return None;
    }
    let control = |item: Item, current: bool, shape: Option<Shape>| {
        let key = item.command().and_then(|command| keys.first(command));
        Part::Control(Control {
            item,
            current,
            shape,
            badge: situation.badges && key.is_some(),
            key,
        })
    };
    let fits = |bar: &Bar| bar.width() + 2.0 * SIDE <= situation.width;
    if !situation.selected {
        if !matches!(situation.tool, Tool::Shape(_) | Tool::Text) {
            return None;
        }
        let swatches = Token::ALL
            .iter()
            .map(|token| control(Item::Swatch(*token), *token == situation.next_color, None));
        let labelled = Bar {
            parts: std::iter::once(Part::Label)
                .chain(swatches.clone())
                .collect(),
            folded: false,
            menu: None,
        };
        let bare = Bar {
            parts: swatches.collect(),
            ..labelled.clone()
        };
        return [labelled, bare].into_iter().find(fits);
    }

    let offered = |command: Command| offers.iter().find(|offer| offer.command == command);
    let current = |command: Command| offered(command).is_some_and(|offer| offer.current);
    let all_shapes = || {
        LETTERED
            .into_iter()
            .chain([Shape::Rounded, Shape::Document])
    };
    let shapes: Vec<Shape> = all_shapes()
        .filter(|shape| offered(Command::Shape(*shape)).is_some())
        .collect();
    // The groups of controls, left to right; the ones that fold are
    // marked.
    let mut groups: Vec<(bool, Vec<Part>)> = Vec::new();
    if !shapes.is_empty() {
        let shared = shapes
            .iter()
            .copied()
            .find(|shape| current(Command::Shape(*shape)));
        groups.push((false, vec![control(Item::Shapes, shared.is_some(), shared)]));
    }
    let colours: Vec<Part> = Token::ALL
        .iter()
        .filter(|token| offered(Command::Color(**token)).is_some())
        .map(|token| control(Item::Swatch(*token), current(Command::Color(*token)), None))
        .collect();
    if !colours.is_empty() {
        groups.push((false, colours));
    }
    let fills: Vec<Part> = Fill::ALL
        .iter()
        .filter(|fill| offered(Command::Fill(**fill)).is_some())
        .map(|fill| control(Item::Fill(*fill), current(Command::Fill(*fill)), None))
        .collect();
    if !fills.is_empty() {
        groups.push((true, fills));
    }
    let connector: Vec<Part> = [Item::Route, Item::StartHead, Item::EndHead, Item::Reverse]
        .into_iter()
        .filter(|item| item.command().and_then(offered).is_some())
        .map(|item| control(item, false, None))
        .collect();
    if !connector.is_empty() {
        groups.push((true, connector));
    }
    if offered(Command::StrokeDashed).is_some() {
        groups.push((true, vec![control(Item::Dashed, false, None)]));
    }
    groups.push((false, vec![control(Item::More, false, None)]));

    let build = |folded: bool| {
        let mut parts = Vec::new();
        for (_, group) in groups.iter().filter(|(folds, _)| !(folded && *folds)) {
            if !parts.is_empty() {
                parts.push(Part::Rule);
            }
            parts.extend(group.iter().cloned());
        }
        Bar {
            parts,
            folded,
            menu: None,
        }
    };
    let folds = groups.iter().any(|(folds, _)| *folds);
    let mut bar = [Some(build(false)), folds.then(|| build(true))]
        .into_iter()
        .flatten()
        .find(fits)?;

    let open = situation
        .menu
        .filter(|menu| bar.controls().any(|control| control.item == menu.anchor()));
    bar.menu = open.map(|menu| {
        let row = |command: Command| MenuRow {
            command,
            current: current(command),
            key: keys.first(command),
        };
        let rows: Vec<MenuRow> = match menu {
            Menu::Shapes => shapes
                .iter()
                .map(|shape| row(Command::Shape(*shape)))
                .collect(),
            Menu::More => {
                let folded = groups
                    .iter()
                    .filter(|(folds, _)| bar.folded && *folds)
                    .flat_map(|(_, group)| group)
                    .filter_map(|part| match part {
                        Part::Control(control) => control.item.command(),
                        _ => None,
                    });
                let edits = [
                    Command::Duplicate,
                    Command::Arrange(Stack::Front),
                    Command::Arrange(Stack::Back),
                    Command::Delete,
                ]
                .into_iter()
                .filter(|command| offered(*command).is_some());
                folded
                    .chain(edits)
                    .chain([Command::Actions])
                    .map(row)
                    .collect()
            }
        };
        (menu, rows)
    });
    Some(bar)
}

/// How many logical pixels along the canvas's top a fit leaves for the
/// bar, beyond the fit's own margin, so it never covers a fitted board.
pub fn covered(shown: bool) -> f32 {
    if shown {
        (TOP + HEIGHT + super::board_dock::CLEAR - wren_canvas::FIT_MARGIN).max(0.0)
    } else {
        0.0
    }
}

/// The message a click on `item` sends: its command through `dispatch`,
/// or its menu opened, or closed when it is open.
pub fn message(item: Item, open: Option<Menu>) -> Message {
    match (item.command(), item.menu()) {
        (Some(command), _) => run(command),
        (None, menu) => Message::StyleMenu(if open == menu { None } else { menu }),
    }
}

/// The message that runs `command` as its key does.
pub fn run(command: Command) -> Message {
    Message::Command(AppCommand::Run(CommandId::Board(command)))
}

/// The pointer coming onto a control that previews, or leaving it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hover {
    Enter(Command),
    Leave(Command),
}

impl App {
    /// The open board's style bar, or `None` with `board-toolbar = off`,
    /// before the canvas has a size, with nothing to style, or on a
    /// canvas too narrow for it.
    pub(super) fn board_style_bar(&self) -> Option<Bar> {
        let view = self.board.as_ref()?;
        if !self.preferences.settings.board_toolbar {
            return None;
        }
        let (width, _) = view.size()?;
        let machine = view.machine();
        let board = view.board();
        let keys = Keys::new(
            &self.preferences.keymap,
            wren_core::keybind::Platform::current(),
        );
        bar(
            Situation {
                width,
                writable: view.writable(),
                selected: machine
                    .selection()
                    .iter()
                    .any(|element| element.exists(board)),
                tool: machine.tool(),
                next_color: view.next_color(),
                badges: self.board_badges(),
                menu: self.style_menu,
            },
            &view.offers(),
            &keys,
        )
    }

    /// The command the pointer previews on the board from the style bar:
    /// the control or row it is over, while the bar shows it and it is not
    /// the selection's value already. Nothing previews under a palette or
    /// a dialog, or while a label is open.
    pub(super) fn style_bar_patch(&self) -> Option<Command> {
        let hovered = self.style_hover?;
        if self.palette.is_some() || self.dialog.is_some() || self.loading.is_some() {
            return None;
        }
        if self.board.as_ref()?.label().is_some() {
            return None;
        }
        let bar = self.board_style_bar()?;
        (bar.shows(hovered) == Some(false)).then_some(hovered)
    }

    /// The bar over the board, laid over the canvas's top centre.
    pub(super) fn board_style_bar_view(&self) -> Option<Element<'_, Message>> {
        let bar = self.board_style_bar()?;
        let (width, height) = self.board.as_ref()?.size()?;
        let definition = self.definition;
        let colors = definition.colors;
        let open = bar.menu.as_ref().map(|(menu, _)| *menu);

        let mut strip = row![].spacing(GAP).align_y(iced::Center);
        for part in &bar.parts {
            strip = strip.push(match part {
                Part::Label => container(
                    text("New shapes")
                        .size(12)
                        .font(fonts::UI)
                        .color(theme::color(colors.muted))
                        .wrapping(text::Wrapping::None),
                )
                .width(LABEL)
                .padding(iced::Padding {
                    left: 8.0,
                    right: 4.0,
                    ..iced::Padding::ZERO
                })
                .into(),
                Part::Rule => container(
                    container(Space::new())
                        .width(1)
                        .height(BUTTON.1 - 8.0)
                        .style(move |_| chrome::surface(definition, colors.divider, 0.0, false)),
                )
                .padding([0.0, (RULE - 1.0) / 2.0])
                .into(),
                Part::Control(control) => self.style_control(control, open),
            });
        }
        // A border takes no room of its own in iced, so the padding holds it.
        let strip = container(strip)
            .id(BAR_ID)
            .padding(PAD + BORDER)
            .style(move |_| panel(definition));
        // The bar's own background takes a press, so none reaches the
        // canvas beneath it; it closes an open menu.
        let strip = mouse_area(strip)
            .on_press(Message::StyleMenu(None))
            .interaction(iced::mouse::Interaction::Idle);

        let mut layer = column![strip].spacing(10).align_x(iced::Center);
        if let Some((menu, rows)) = &bar.menu {
            // Under its button, and inside the canvas.
            let room = ((width - MENU_WIDTH) / 2.0 - SIDE).max(0.0);
            let offset = bar.offset(menu.anchor()).clamp(-room, room);
            let mut list = column![].spacing(1);
            let mut drawn = false;
            for each in rows {
                // A rule between the styles folded into the menu and the
                // edits under them.
                let glyph = has_glyph(each.command);
                if drawn && !glyph {
                    list = list.push(
                        container(container(Space::new()).width(iced::Fill).height(1).style(
                            move |_| chrome::surface(definition, colors.divider, 0.0, false),
                        ))
                        .padding([3, 6]),
                    );
                }
                drawn = glyph;
                list = list.push(self.style_menu_row(each));
            }
            // In a canvas too short for every row the menu scrolls.
            let room = (height - TOP - HEIGHT - 10.0 - SIDE).max(MENU_ROW);
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
            .on_press(Message::StyleMenu(Some(*menu)))
            .interaction(iced::mouse::Interaction::Idle);
            let shift = Space::new().width(2.0 * offset.abs());
            layer = layer.push(if offset >= 0.0 {
                row![shift, list]
            } else {
                row![list, shift]
            });
        }
        Some(
            container(layer)
                .width(iced::Fill)
                .height(iced::Fill)
                .align_x(iced::Center)
                .align_y(iced::Top)
                .padding(iced::Padding {
                    top: TOP,
                    ..iced::Padding::ZERO
                })
                .into(),
        )
    }

    fn style_control<'a>(&self, each: &Control, open: Option<Menu>) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let lit = each.current && matches!(each.item, Item::Fill(_));
        let ink = if lit { colors.accent } else { colors.text };
        let glyph = |paths: &str| chrome::paths_icon(paths, 20.0, ink);
        let face: Element<'a, Message> = match each.item {
            Item::Swatch(token) => return self.swatch(each, token),
            Item::Shapes => {
                let drawn = match each.shape {
                    Some(shape) => shape_paths(shape),
                    None => MIXED.to_owned(),
                };
                row![
                    chrome::paths_icon(&drawn, 20.0, ink),
                    chrome::icon(Icon::Chevron, 10.0, ink)
                ]
                .align_y(iced::Center)
                .into()
            }
            Item::More => chrome::icon(Icon::More, 20.0, ink),
            Item::Fill(fill) => glyph(&fill_paths(fill, ink)),
            Item::Dashed => glyph(DASHED),
            Item::Route => glyph(ROUTE),
            Item::StartHead => glyph(START),
            Item::EndHead => glyph(END),
            Item::Reverse => glyph(REVERSE),
        };
        let mut face = stack![container(face).center(iced::Fill)];
        if let Some(key) = each.key.clone().filter(|_| each.badge) {
            face = face.push(badge(key, if lit { colors.accent } else { colors.muted }));
        }
        let pressed = button(face)
            .width(each.item.width())
            .height(BUTTON.1)
            .padding(0)
            .style(move |_, status| tool_button(definition, status, lit))
            .on_press(message(each.item, open));
        self.tipped(container(pressed).id(each.item.id()), each, open)
    }

    /// A swatch: a dot in the colour the board draws, ringed in the accent
    /// when the whole selection has it, its digit under it while the keys
    /// show.
    fn swatch<'a>(&self, each: &Control, token: Token) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let palette = theme::board_palette(definition);
        let hue = style::resolve(&Color::Token(token), Fill::None, &palette).stroke;
        let hue = Rgb(hue.r, hue.g, hue.b);
        let edge = colors.dialog_border;
        // While the keys show, a smaller dot leaves room for the digit
        // under it.
        let shown = each.key.clone().filter(|_| each.badge);
        let (size, gap) = if shown.is_some() {
            (DOT_BADGED, RING_GAP_BADGED)
        } else {
            (DOT, RING_GAP)
        };
        let dot = container(Space::new())
            .width(size)
            .height(size)
            .style(move |_| container::Style {
                background: Some(theme::color(hue).into()),
                border: Border {
                    color: theme::color(edge),
                    width: 1.0,
                    radius: (size / 2.0).into(),
                },
                ..container::Style::default()
            });
        let outer = size + 2.0 * (RING + gap);
        let (current, accent) = (each.current, colors.accent);
        let ringed = container(dot)
            .padding(RING + gap)
            .width(outer)
            .height(outer)
            .style(move |_| container::Style {
                border: Border {
                    color: if current {
                        theme::color(accent)
                    } else {
                        iced::Color::TRANSPARENT
                    },
                    width: RING,
                    radius: (outer / 2.0).into(),
                },
                ..container::Style::default()
            });
        let face: Element<'a, Message> = match shown {
            Some(key) => column![
                ringed,
                text(key)
                    .size(10)
                    .font(fonts::UI)
                    .color(theme::color(if each.current {
                        colors.accent
                    } else {
                        colors.muted
                    }))
            ]
            .align_x(iced::Center)
            .width(iced::Fill)
            .padding(iced::Padding {
                top: 2.0,
                ..iced::Padding::ZERO
            })
            .into(),
            None => container(ringed).center(iced::Fill).into(),
        };
        let pressed = button(face)
            .width(SWATCH.0)
            .height(SWATCH.1)
            .padding(0)
            .style(move |_, status| tool_button(definition, status, false))
            .on_press(message(each.item, None));
        self.tipped(container(pressed).id(each.item.id()), each, None)
    }

    /// `placed` with its tooltip under it, and for a control that
    /// previews, the pointer's coming and going reported.
    fn tipped<'a>(
        &self,
        placed: container::Container<'a, Message>,
        each: &Control,
        open: Option<Menu>,
    ) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let placed: Element<'a, Message> = match each.item.command().filter(|c| previews(*c)) {
            Some(command) => mouse_area(placed)
                .on_enter(Message::StyleHover(Hover::Enter(command)))
                .on_exit(Message::StyleHover(Hover::Leave(command)))
                .into(),
            None => placed.into(),
        };
        // An open menu says what its button holds.
        if each.item.menu().is_some() && each.item.menu() == open {
            return placed;
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
            tooltip::Position::Bottom,
        )
        .gap(9)
        .into()
    }

    fn style_menu_row<'a>(&self, each: &MenuRow) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let ink = if each.current {
            colors.accent
        } else {
            colors.body
        };
        let glyph = match each.command {
            Command::Shape(shape) => Some(shape_paths(shape)),
            Command::Fill(fill) => Some(fill_paths(fill, ink)),
            Command::StrokeDashed => Some(DASHED.to_owned()),
            Command::EdgeRouting => Some(ROUTE.to_owned()),
            Command::EdgeStart => Some(START.to_owned()),
            Command::EdgeEnd => Some(END.to_owned()),
            Command::EdgeReverse => Some(REVERSE.to_owned()),
            _ => None,
        };
        let mut line = row![].spacing(9).align_y(iced::Center);
        if let Some(paths) = glyph {
            line = line.push(chrome::paths_icon(&paths, 18.0, ink));
        }
        line = line.push(
            text(each.name())
                .size(13)
                .font(if each.current {
                    fonts::SEMIBOLD
                } else {
                    fonts::UI
                })
                .color(theme::color(ink)),
        );
        line = line.push(space::horizontal());
        if let Some(key) = &each.key {
            line = line.push(
                text(key.clone())
                    .size(11)
                    .font(fonts::UI)
                    .color(theme::color(colors.faint)),
            );
        }
        let placed = container(
            button(line)
                .width(iced::Fill)
                .height(MENU_ROW)
                .padding([5, 8])
                .style(move |_, status| tool_button(definition, status, false))
                .on_press(run(each.command)),
        )
        .id(row_id(each.command));
        if previews(each.command) {
            mouse_area(placed)
                .on_enter(Message::StyleHover(Hover::Enter(each.command)))
                .on_exit(Message::StyleHover(Hover::Leave(each.command)))
                .into()
        } else {
            placed.into()
        }
    }
}

/// Whether a menu row draws its style beside its name.
fn has_glyph(command: Command) -> bool {
    matches!(
        command,
        Command::Shape(_)
            | Command::Fill(_)
            | Command::StrokeDashed
            | Command::EdgeRouting
            | Command::EdgeStart
            | Command::EdgeEnd
            | Command::EdgeReverse
    )
}

/// A key shown in a control's corner while the keys show.
fn badge<'a>(key: String, color: Rgb) -> Element<'a, Message> {
    container(
        text(key)
            .size(10)
            .font(fonts::UI)
            .color(theme::color(color)),
    )
    .width(iced::Fill)
    .height(iced::Fill)
    .align_x(iced::Right)
    .align_y(iced::Bottom)
    .padding(iced::Padding {
        right: 3.0,
        bottom: 1.0,
        ..iced::Padding::ZERO
    })
    .into()
}

/// A selection of several shapes: a square and a circle over it.
const MIXED: &str =
    "<rect x='3' y='5' width='11' height='9' rx='1'/><circle cx='15.5' cy='14.5' r='5'/>";
const DASHED: &str = "<path d='M3 12h4M10 12h4M17 12h4'/>";
const ROUTE: &str = "<path d='M4 18h8V6h8'/>";
const START: &str = "<path d='M5 12h15M10 7l-5 5 5 5'/>";
const END: &str = "<path d='M4 12h15M14 7l5 5-5 5'/>";
const REVERSE: &str = "<path d='M4 8h15M15 4l4 4-4 4M20 16H5M9 12l-4 4 4 4'/>";

/// A box filled as `fill` fills a shape: not at all, lightly, or whole,
/// in `ink`.
fn fill_paths(fill: Fill, ink: Rgb) -> String {
    let opacity = match fill {
        Fill::None => 0.0,
        Fill::Tint => 0.3,
        Fill::Solid => 1.0,
    };
    format!(
        "<rect x='4' y='6' width='16' height='12' rx='2' fill='#{:02x}{:02x}{:02x}' fill-opacity='{opacity}'/>",
        ink.0, ink.1, ink.2
    )
}

#[cfg(test)]
mod tests {
    use wren_board::scene::Source;
    use wren_core::keybind::Platform;

    use super::super::board::tests::{fixture, open_board};
    use super::super::board_keys::tests::{focus, keymap};
    use super::*;

    const WIDE_CANVAS: f32 = 1000.0;

    const SELECTED: Situation = Situation {
        width: WIDE_CANVAS,
        writable: true,
        selected: true,
        tool: Tool::Select,
        next_color: Token::Neutral,
        badges: false,
        menu: None,
    };

    /// The rows the actions palette would offer: each command with
    /// whether it is current, under its own group.
    fn offers(commands: impl IntoIterator<Item = (Command, bool)>) -> Vec<Offer> {
        commands
            .into_iter()
            .map(|(command, current)| Offer {
                group: command.offer_group().unwrap_or("Board"),
                command,
                current,
            })
            .collect()
    }

    fn shapes(current: Option<Shape>) -> Vec<(Command, bool)> {
        LETTERED
            .into_iter()
            .chain([Shape::Rounded, Shape::Document])
            .map(|shape| (Command::Shape(shape), Some(shape) == current))
            .collect()
    }

    fn colours(current: Option<Token>) -> Vec<(Command, bool)> {
        Token::ALL
            .iter()
            .map(|token| (Command::Color(*token), Some(*token) == current))
            .collect()
    }

    fn fills(current: Option<Fill>) -> Vec<(Command, bool)> {
        Fill::ALL
            .iter()
            .map(|fill| (Command::Fill(*fill), Some(*fill) == current))
            .collect()
    }

    fn connectors() -> Vec<(Command, bool)> {
        [
            Command::EdgeRouting,
            Command::EdgeStart,
            Command::EdgeEnd,
            Command::EdgeReverse,
        ]
        .map(|command| (command, false))
        .to_vec()
    }

    fn edits(nodes: bool) -> Vec<(Command, bool)> {
        let mut edits = vec![
            (Command::Arrange(Stack::Front), false),
            (Command::Arrange(Stack::Back), false),
            (Command::Arrange(Stack::Forward), false),
            (Command::Arrange(Stack::Backward), false),
        ];
        if nodes {
            edits.extend([(Command::EditLabel, false), (Command::Duplicate, false)]);
        }
        edits.push((Command::Delete, false));
        edits
    }

    /// One shape: a pill, blue, with the tint fill.
    fn one_shape() -> Vec<Offer> {
        let mut all = shapes(Some(Shape::Pill));
        all.extend(colours(Some(Token::Blue)));
        all.extend(fills(Some(Fill::Tint)));
        all.push((Command::StrokeDashed, false));
        all.extend(edits(true));
        offers(all)
    }

    /// A shape and a connector, of two colours.
    fn mixed() -> Vec<Offer> {
        let mut all = shapes(Some(Shape::Rect));
        all.extend(colours(None));
        all.extend(fills(Some(Fill::Tint)));
        all.push((Command::StrokeDashed, false));
        all.extend(connectors());
        all.extend(edits(true));
        offers(all)
    }

    /// Connectors alone, red.
    fn connector() -> Vec<Offer> {
        let mut all = colours(Some(Token::Red));
        all.push((Command::StrokeDashed, false));
        all.extend(connectors());
        all.extend(edits(false));
        offers(all)
    }

    fn styled(situation: Situation, offers: &[Offer]) -> Option<Bar> {
        let keymap = keymap(Platform::Linux, &[]);
        bar(situation, offers, &Keys::new(&keymap, Platform::Linux))
    }

    fn ids(bar: &Bar) -> Vec<&'static str> {
        bar.controls().map(|control| control.item.id()).collect()
    }

    fn find(bar: &Bar, item: Item) -> &Control {
        bar.controls().find(|control| control.item == item).unwrap()
    }

    fn current(bar: &Bar) -> Vec<&'static str> {
        let current = bar.controls().filter(|control| control.current);
        current.map(|control| control.item.id()).collect()
    }

    const SWATCHES: [&str; 8] = [
        "board-style-neutral",
        "board-style-red",
        "board-style-orange",
        "board-style-yellow",
        "board-style-green",
        "board-style-cyan",
        "board-style-purple",
        "board-style-blue",
    ];

    #[test]
    fn one_shape_shows_its_shape_its_colour_and_its_fill() {
        let bar = styled(SELECTED, &one_shape()).unwrap();
        let mut expected = vec!["board-style-shape"];
        expected.extend(SWATCHES);
        expected.extend([
            "board-style-fill-none",
            "board-style-fill-tint",
            "board-style-fill-solid",
            "board-style-dashed",
            "board-style-more",
        ]);
        assert_eq!(ids(&bar), expected);
        assert!(!bar.folded && !bar.new_shapes() && bar.menu.is_none());
        // The shape it is, its ringed colour and its lit fill.
        assert_eq!(
            current(&bar),
            [
                "board-style-shape",
                "board-style-blue",
                "board-style-fill-tint"
            ]
        );
        assert_eq!(find(&bar, Item::Shapes).shape, Some(Shape::Pill));
        // Groups between rules: shape, colours, fills, outline, more.
        let rules = bar.parts.iter().filter(|part| **part == Part::Rule).count();
        assert_eq!(rules, 4);
        // Every id the bar gives is one the harness looks for.
        for control in bar.controls() {
            assert!(IDS.contains(&control.item.id()), "{:?}", control.item);
        }
        let blue = find(&bar, Item::Swatch(Token::Blue));
        assert_eq!(blue.item.command(), Some(Command::Color(Token::Blue)));
        assert_eq!(blue.tip(), "Blue   7");
        assert_eq!(find(&bar, Item::Fill(Fill::None)).tip(), "Fill: none");
        assert_eq!(find(&bar, Item::Shapes).item.command(), None);
    }

    #[test]
    fn shapes_of_several_kinds_and_colours_mark_nothing_current() {
        let mut all = shapes(None);
        all.extend(colours(None));
        all.extend(fills(None));
        all.push((Command::StrokeDashed, false));
        all.extend(edits(true));
        let bar = styled(SELECTED, &offers(all)).unwrap();
        let shape = find(&bar, Item::Shapes);
        assert_eq!((shape.current, shape.shape), (false, None));
        assert!(current(&bar).is_empty());
    }

    #[test]
    fn connectors_show_their_colour_route_heads_and_outline() {
        let bar = styled(SELECTED, &connector()).unwrap();
        let mut expected = SWATCHES.to_vec();
        expected.extend([
            "board-style-route",
            "board-style-start",
            "board-style-end",
            "board-style-reverse",
            "board-style-dashed",
            "board-style-more",
        ]);
        assert_eq!(ids(&bar), expected);
        assert_eq!(current(&bar), ["board-style-red"]);
        assert_eq!(
            find(&bar, Item::Route).item.command(),
            Some(Command::EdgeRouting)
        );
    }

    #[test]
    fn a_mixed_selection_shows_both_and_the_last_menu_its_edits() {
        let bar = styled(SELECTED, &mixed()).unwrap();
        let ids = ids(&bar);
        assert_eq!(ids.first(), Some(&"board-style-shape"));
        assert!(ids.contains(&"board-style-fill-solid") && ids.contains(&"board-style-reverse"));
        assert_eq!(ids.last(), Some(&"board-style-more"));
        assert!(bar.width() + 2.0 * SIDE <= WIDE_CANVAS);
        // The last menu: duplicate, front, back, delete and every action.
        let more = styled(
            Situation {
                menu: Some(Menu::More),
                ..SELECTED
            },
            &mixed(),
        )
        .unwrap();
        let (menu, rows) = more.menu.unwrap();
        assert_eq!(menu, Menu::More);
        let commands: Vec<Command> = rows.iter().map(|row| row.command).collect();
        assert_eq!(
            commands,
            [
                Command::Duplicate,
                Command::Arrange(Stack::Front),
                Command::Arrange(Stack::Back),
                Command::Delete,
                Command::Actions,
            ]
        );
        let keys: Vec<Option<&str>> = rows.iter().map(|row| row.key.as_deref()).collect();
        assert_eq!(
            keys,
            [Some("Ctrl+D"), Some("]"), Some("["), Some("⌫"), Some("/")]
        );
        assert_eq!(rows[4].name(), "All actions…");
        // Connectors alone cannot be duplicated.
        let connectors = styled(
            Situation {
                menu: Some(Menu::More),
                ..SELECTED
            },
            &connector(),
        )
        .unwrap();
        let rows = connectors.menu.unwrap().1;
        assert!(rows.iter().all(|row| row.command != Command::Duplicate));
    }

    #[test]
    fn the_shape_menu_holds_the_eight_shapes_with_the_current_one_marked() {
        let bar = styled(
            Situation {
                menu: Some(Menu::Shapes),
                ..SELECTED
            },
            &one_shape(),
        )
        .unwrap();
        let (menu, rows) = bar.menu.unwrap();
        assert_eq!(menu, Menu::Shapes);
        let shapes: Vec<Command> = rows.iter().map(|row| row.command).collect();
        assert_eq!(shapes.len(), 8);
        assert_eq!(shapes[0], Command::Shape(Shape::Rect));
        let marked: Vec<Command> = rows
            .iter()
            .filter(|row| row.current)
            .map(|row| row.command)
            .collect();
        assert_eq!(marked, [Command::Shape(Shape::Pill)]);
        assert_eq!(rows[0].key.as_deref(), Some("Shift+R"));
        assert_eq!(rows[7].key, None);
        for row in &rows {
            assert!(IDS.contains(&row_id(row.command)), "{:?}", row.command);
            assert!(previews(row.command));
        }
        // A menu whose button is not shown is not open.
        let none = styled(
            Situation {
                menu: Some(Menu::Shapes),
                ..SELECTED
            },
            &connector(),
        )
        .unwrap();
        assert_eq!(none.menu, None);
    }

    #[test]
    fn with_nothing_selected_the_bar_shows_only_for_a_shape_or_text_tool() {
        let idle = Situation {
            selected: false,
            ..SELECTED
        };
        let nothing = offers([
            (Command::Paste, false),
            (Command::SelectAll, false),
            (Command::ZoomFit, false),
        ]);
        for tool in [Tool::Select, Tool::Hand, Tool::Connector] {
            assert_eq!(
                styled(Situation { tool, ..idle }, &nothing),
                None,
                "{tool:?}"
            );
        }
        for tool in [Tool::Shape(Shape::Diamond), Tool::Text] {
            let bar = styled(
                Situation {
                    tool,
                    next_color: Token::Green,
                    ..idle
                },
                &nothing,
            )
            .unwrap();
            assert!(bar.new_shapes());
            assert_eq!(bar.parts[0], Part::Label);
            assert_eq!(ids(&bar), SWATCHES);
            assert_eq!(current(&bar), ["board-style-green"]);
            assert!(bar.menu.is_none());
        }
    }

    #[test]
    fn badges_show_each_swatch_s_digit_only_when_asked() {
        let quiet = styled(SELECTED, &one_shape()).unwrap();
        assert!(quiet.controls().all(|control| !control.badge));
        let shown = styled(
            Situation {
                badges: true,
                ..SELECTED
            },
            &one_shape(),
        )
        .unwrap();
        let digits: Vec<Option<&str>> = Token::ALL
            .iter()
            .map(|token| {
                let swatch = find(&shown, Item::Swatch(*token));
                swatch.key.as_deref().filter(|_| swatch.badge)
            })
            .collect();
        assert_eq!(digits, ["0", "1", "2", "3", "4", "5", "6", "7"].map(Some));
        // Controls with no key show none.
        assert!(!find(&shown, Item::Dashed).badge);
        // A rebound colour shows its own key.
        let keymap = keymap(Platform::Linux, &[("9", "board.color.blue")]);
        let rebound = bar(
            Situation {
                badges: true,
                ..SELECTED
            },
            &one_shape(),
            &Keys::new(&keymap, Platform::Linux),
        )
        .unwrap();
        assert_eq!(
            find(&rebound, Item::Swatch(Token::Blue)).key.as_deref(),
            Some("9")
        );
    }

    #[test]
    fn a_narrow_canvas_folds_the_styles_into_the_last_menu_and_a_tiny_one_has_no_bar() {
        let wide = styled(SELECTED, &mixed()).unwrap();
        let narrow = Situation {
            width: wide.width() + 2.0 * SIDE - 1.0,
            menu: Some(Menu::More),
            ..SELECTED
        };
        let folded = styled(narrow, &mixed()).unwrap();
        assert!(folded.folded);
        let mut expected = vec!["board-style-shape"];
        expected.extend(SWATCHES);
        expected.push("board-style-more");
        assert_eq!(ids(&folded), expected);
        assert!(folded.width() + 2.0 * SIDE <= narrow.width);
        // What folded away is at the top of the last menu, current marks
        // and all.
        let rows = folded.menu.unwrap().1;
        let commands: Vec<Command> = rows.iter().map(|row| row.command).collect();
        assert_eq!(
            commands[..8],
            [
                Command::Fill(Fill::None),
                Command::Fill(Fill::Tint),
                Command::Fill(Fill::Solid),
                Command::EdgeRouting,
                Command::EdgeStart,
                Command::EdgeEnd,
                Command::EdgeReverse,
                Command::StrokeDashed,
            ]
        );
        assert!(rows[1].current && !rows[0].current);
        assert_eq!(commands.last(), Some(&Command::Actions));
        // Too narrow for the colours: no bar.
        let least = styled(
            Situation {
                menu: None,
                ..narrow
            },
            &mixed(),
        )
        .unwrap()
        .width()
            + 2.0 * SIDE;
        let tiny = Situation {
            width: least - 1.0,
            ..narrow
        };
        assert_eq!(styled(tiny, &mixed()), None);
        // The New shapes row drops its label before it goes.
        let row = Situation {
            selected: false,
            tool: Tool::Text,
            ..SELECTED
        };
        let labelled = styled(row, &[]).unwrap();
        let bare = styled(
            Situation {
                width: labelled.width() + 2.0 * SIDE - 1.0,
                ..row
            },
            &[],
        )
        .unwrap();
        assert!(!bare.new_shapes());
        assert_eq!(ids(&bare), SWATCHES);
        assert_eq!(
            styled(
                Situation {
                    width: bare.width() + 2.0 * SIDE - 1.0,
                    ..row
                },
                &[]
            ),
            None
        );
    }

    #[test]
    fn a_read_only_board_has_no_bar() {
        let read_only = Situation {
            writable: false,
            ..SELECTED
        };
        assert_eq!(styled(read_only, &one_shape()), None);
        let armed = Situation {
            selected: false,
            tool: Tool::Text,
            ..read_only
        };
        assert_eq!(styled(armed, &[]), None);
    }

    #[test]
    fn a_control_sends_its_command_and_a_menu_button_toggles_its_menu() {
        assert!(matches!(
            message(Item::Swatch(Token::Red), None),
            Message::Command(AppCommand::Run(CommandId::Board(Command::Color(
                Token::Red
            ))))
        ));
        assert!(matches!(
            message(Item::Shapes, None),
            Message::StyleMenu(Some(Menu::Shapes))
        ));
        assert!(matches!(
            message(Item::Shapes, Some(Menu::Shapes)),
            Message::StyleMenu(None)
        ));
        assert!(matches!(
            message(Item::More, Some(Menu::Shapes)),
            Message::StyleMenu(Some(Menu::More))
        ));
        // What previews: a colour, a fill, a route, a shape.
        for item in [Item::Dashed, Item::StartHead, Item::EndHead, Item::Reverse] {
            assert!(!previews(item.command().unwrap()), "{item:?}");
        }
        for item in [
            Item::Swatch(Token::Red),
            Item::Fill(Fill::Solid),
            Item::Route,
        ] {
            assert!(previews(item.command().unwrap()), "{item:?}");
        }
    }

    #[test]
    fn a_fit_leaves_room_for_the_bar_only_while_it_shows() {
        assert_eq!(covered(false), 0.0);
        assert!(covered(true) > 0.0);
        assert_eq!(
            covered(true) + wren_canvas::FIT_MARGIN,
            TOP + HEIGHT + super::super::board_dock::CLEAR
        );
    }

    fn select(app: &mut App, names: &[&str]) {
        let view = app.board.as_mut().unwrap();
        let _ = view.command(Command::SelectNone, std::time::Instant::now());
        for name in names {
            let id = wren_board::Id::new(name).unwrap();
            let source = Source::of(view.board(), &id).unwrap();
            if names.len() == 1 {
                assert!(view.show_element(source));
            }
        }
        if names.len() > 1 {
            let _ = view.command(Command::SelectAll, std::time::Instant::now());
        }
    }

    #[test]
    fn the_app_shows_the_bar_for_the_selection_and_its_clicks_run_its_commands() {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        assert!(app.board_style_bar().is_none(), "no size yet");
        focus(&mut app);
        assert!(app.board_style_bar().is_none(), "nothing selected");

        // A shape: its bar, from what the actions palette offers.
        select(&mut app, &["pack"]);
        let bar = app.board_style_bar().unwrap();
        assert_eq!(
            current(&bar)[..2],
            ["board-style-shape", "board-style-neutral"]
        );
        // A click recolours it as one undo step, and sets the colour new
        // shapes take; the board keeps the keyboard.
        let _ = app.update(message(Item::Swatch(Token::Red), None));
        let view = app.board.as_ref().unwrap();
        let red = wren_board::Color::Token(Token::Red);
        let pack = wren_board::Id::new("pack").unwrap();
        assert_eq!(view.board().nodes[&pack].style.color, red);
        assert_eq!(view.next_color(), Token::Red);
        assert!(view.machine().focused());
        assert_eq!(
            current(&app.board_style_bar().unwrap())[1],
            "board-style-red"
        );
        let _ = app.update(run(Command::Undo));
        assert!(app.board.as_ref().unwrap().pristine());

        // The pointer over a swatch previews it and writes nothing; off it
        // the preview goes.
        let _ = app.update(Message::StyleHover(Hover::Enter(Command::Color(
            Token::Blue,
        ))));
        let view = app.board.as_ref().unwrap();
        assert_eq!(view.patch(), Some(Command::Color(Token::Blue)));
        assert!(view.pristine());
        let _ = app.update(Message::StyleHover(Hover::Leave(Command::Color(
            Token::Blue,
        ))));
        assert_eq!(app.board.as_ref().unwrap().patch(), None);
        // The current colour previews nothing, and neither does a control
        // the bar does not show.
        let _ = app.update(Message::StyleHover(Hover::Enter(Command::Color(
            Token::Neutral,
        ))));
        assert_eq!(app.board.as_ref().unwrap().patch(), None);
        let _ = app.update(Message::StyleHover(Hover::Enter(Command::EdgeRouting)));
        assert_eq!(app.board.as_ref().unwrap().patch(), None);
        // A control that goes while the pointer is on it takes its hover
        // with it: back again, it previews nothing until the pointer comes.
        let _ = app.update(Message::StyleHover(Hover::Enter(Command::Fill(
            Fill::Solid,
        ))));
        select(&mut app, &[]);
        let _ = app.update(Message::StyleMenu(None));
        assert_eq!(app.style_hover, None);
        select(&mut app, &["pack"]);
        let _ = app.update(Message::StyleMenu(None));
        assert_eq!(app.board.as_ref().unwrap().patch(), None);
        // A hover then a click: the preview applied, as one step.
        let _ = app.update(Message::StyleHover(Hover::Enter(Command::Fill(
            Fill::Solid,
        ))));
        assert_eq!(
            app.board.as_ref().unwrap().patch(),
            Some(Command::Fill(Fill::Solid))
        );
        let _ = app.update(message(Item::Fill(Fill::Solid), None));
        let view = app.board.as_ref().unwrap();
        assert_eq!(view.patch(), None);
        assert!(!view.pristine() && view.can_undo());
        let _ = app.update(run(Command::Undo));
        assert!(app.board.as_ref().unwrap().pristine());
        // A menu over a preview: the palette opening drops it.
        let _ = app.update(Message::StyleHover(Hover::Enter(Command::Color(
            Token::Blue,
        ))));
        let _ = app.update(run(Command::Actions));
        assert!(app.palette.is_some());
        let _ = app.update(Message::Escape);

        // The shape menu opens, a row reshapes, and Escape closes it.
        let _ = app.update(message(Item::Shapes, None));
        let bar = app.board_style_bar().unwrap();
        assert_eq!(bar.menu.as_ref().unwrap().0, Menu::Shapes);
        let _ = app.update(Message::Escape);
        assert!(app.board_style_bar().unwrap().menu.is_none());
        let _ = app.update(message(Item::Shapes, None));
        let _ = app.update(run(Command::Shape(Shape::Ellipse)));
        assert!(app.style_menu.is_none());
        let view = app.board.as_ref().unwrap();
        assert_eq!(
            view.board().nodes[&pack].shape,
            wren_board::Named::Known(Shape::Ellipse)
        );
        // One menu at a time: the dock's closes the bar's.
        let _ = app.update(message(Item::More, None));
        let _ = app.update(Message::DockMenu(Some(
            super::super::board_dock::Menu::More,
        )));
        assert!(app.style_menu.is_none());

        // Two shapes and the connector: both kinds of control.
        select(&mut app, &["pack", "receive"]);
        let ids = ids(&app.board_style_bar().unwrap());
        assert!(ids.contains(&"board-style-shape") && ids.contains(&"board-style-route"));

        // Nothing selected and a shape tool armed: the New shapes row.
        let _ = app.update(run(Command::SelectNone));
        let _ = app.update(run(Command::Tool(Tool::Shape(Shape::Rect))));
        let bar = app.board_style_bar().unwrap();
        assert!(bar.new_shapes());
        assert_eq!(current(&bar), ["board-style-red"]);
        let before = app.board.as_ref().unwrap().board().clone();
        let revision = app.board_revision;
        let _ = app.update(message(Item::Swatch(Token::Green), None));
        let view = app.board.as_ref().unwrap();
        assert_eq!(view.next_color(), Token::Green);
        assert_eq!((view.board(), app.board_revision), (&before, revision));
        assert_eq!(view.machine().tool(), Tool::Shape(Shape::Rect));

        // The setting off: no bar.
        app.preferences.settings.board_toolbar = false;
        assert!(app.board_style_bar().is_none());
    }
}
