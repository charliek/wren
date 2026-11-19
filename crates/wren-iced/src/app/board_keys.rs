//! The keys a board shows (discovery/board-controls.md §3.4 and §3.5): the
//! hint line in its footer, which names the keys that fit what the board
//! shows, and the keys sheet, which lists every board key in the groups of
//! `docs/reference/boards.md`. Both read the live keymap, so a rebound key
//! shows as rebound and a command with no key drops out.

use std::collections::BTreeSet;

use wren_board::edit::{Alignment, Axis, Stack};
use wren_board::scene::Source;
use wren_board::{Board, Dir, Shape, Token};
use wren_canvas::{Command, Grow, Tool};
use wren_core::keybind::{Accel, Keymap, Platform};

use iced::widget::{
    column, container, mouse_area, opaque, row, scrollable, stack, text, text_input,
};
use iced::{Element, Fill};

use super::commands::{definitions, CommandId};
use super::palette::{palette_top, Palette, GROUP_HEADER_PADDING, GROUP_HEADER_SIZE};
use super::{App, Message};
use crate::ellipsis::ellipsis;
use crate::{chrome, fonts, theme};

/// The widest the keys sheet grows, and the narrowest a column of it is.
const SHEET_WIDTH: f32 = 1040.0;
const SHEET_COLUMN: f32 = 300.0;

/// One part of the hint line: its keys, each drawn as a keycap, then what
/// they do. A part with no keys is words alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub keys: Vec<String>,
    pub words: &'static str,
}

/// What is selected, as the hint line reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    Nothing,
    /// One shape or text element and nothing else.
    OneShape,
    /// Two elements or more, a shape among them.
    Shapes,
    /// Connectors and nothing else.
    Connectors,
}

impl Selected {
    /// The selection's kind; an element the board no longer holds counts
    /// for nothing.
    pub fn of(board: &Board, selection: &BTreeSet<Source>) -> Self {
        let nodes = selection
            .iter()
            .filter_map(Source::node)
            .filter(|id| board.nodes.contains_key(*id))
            .count();
        let edges = selection
            .iter()
            .filter_map(Source::edge)
            .filter(|id| board.edges.contains_key(*id))
            .count();
        match (nodes, edges) {
            (0, 0) => Self::Nothing,
            (0, _) => Self::Connectors,
            (1, 0) => Self::OneShape,
            _ => Self::Shapes,
        }
    }
}

/// What a board shows, as far as its hint line reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Situation {
    /// The board has the keyboard; an unfocused board has no hint.
    pub focused: bool,
    /// A label is open.
    pub editing: bool,
    /// Keyboard Connecting.
    pub seeking: bool,
    pub tool: Tool,
    /// The armed tool is locked (D3).
    pub locked: bool,
    /// Space is held, so a drag pans.
    pub space: bool,
    /// Holding Space shows the dock's keys: the dock is on and its badges
    /// are `hold` (§3.3).
    pub space_shows_keys: bool,
    pub selected: Selected,
}

/// The labels the live keymap gives board commands, on `platform`.
pub struct Keys<'a> {
    keymap: &'a Keymap,
    platform: Platform,
}

impl<'a> Keys<'a> {
    pub fn new(keymap: &'a Keymap, platform: Platform) -> Self {
        Self { keymap, platform }
    }

    /// The key the keymap shows first for `command`.
    pub(super) fn first(&self, command: Command) -> Option<String> {
        self.keymap.label(command.key(), self.platform)
    }

    /// Every key the board layer binds to `command`, the first shown
    /// first.
    fn all(&self, command: Command) -> Vec<String> {
        self.keymap
            .board_bindings()
            .filter(|(_, action)| *action == command.key())
            .map(|(accel, _)| accel.label(self.platform))
            .collect()
    }

    /// The first key of each command that has one.
    fn each(&self, commands: &[Command]) -> Vec<String> {
        commands
            .iter()
            .filter_map(|command| self.first(*command))
            .collect()
    }

    /// The keys of a command for each direction, left, right, up and down:
    /// `Ctrl+Arrow` (`⌘Arrow`, or `Arrows` with no modifier) when the four
    /// are the arrows with one set of modifiers, else each bound key.
    fn arrows(&self, commands: [Command; 4]) -> Vec<String> {
        const ARROWS: [&str; 4] = ["left", "right", "up", "down"];
        let accels: Vec<Option<&Accel>> = commands
            .iter()
            .map(|command| self.keymap.accel(command.key()))
            .collect();
        let together = accels.iter().zip(ARROWS).all(|(accel, arrow)| {
            accel.is_some_and(|accel| {
                accel.key == arrow && Some(accel.mods) == accels[0].map(|first| first.mods)
            })
        });
        match accels[0] {
            Some(first) if together => {
                let label = first.label(self.platform);
                let mods = label.strip_suffix('←').unwrap_or_default();
                vec![if mods.is_empty() {
                    "Arrows".to_owned()
                } else {
                    format!("{mods}Arrow")
                }]
            }
            _ => self.each(&commands),
        }
    }

    /// The colour keys: `0–7` when the eight colours are the bare digits in
    /// order, else each bound key.
    fn digits(&self) -> Vec<String> {
        let colours = colours();
        let digits = colours.iter().enumerate().all(|(digit, command)| {
            self.keymap.accel(command.key()).is_some_and(|accel| {
                accel.mods == wren_core::keybind::Mods::default() && accel.key == digit.to_string()
            })
        });
        if digits {
            vec!["0–7".to_owned()]
        } else {
            self.each(&colours)
        }
    }

    /// The keys of an app command, such as the text-size keys a board
    /// zooms with, the first shown first. `+` is Shift with `=`, so a
    /// binding on `=` is shown before one on `+`.
    fn app(&self, action: &str) -> Vec<String> {
        let mut bound: Vec<&Accel> = self
            .keymap
            .bindings()
            .filter(|(_, bound)| *bound == action)
            .map(|(accel, _)| accel)
            .collect();
        bound.sort_by_key(|accel| accel.key == "+");
        bound
            .first()
            .map(|accel| accel.label(self.platform))
            .into_iter()
            .collect()
    }
}

fn colours() -> [Command; 8] {
    [
        Token::Neutral,
        Token::Red,
        Token::Orange,
        Token::Yellow,
        Token::Green,
        Token::Cyan,
        Token::Purple,
        Token::Blue,
    ]
    .map(Command::Color)
}

/// The command for each direction, left, right, up and down.
fn directions(command: fn(Dir) -> Command) -> [Command; 4] {
    [Dir::Left, Dir::Right, Dir::Up, Dir::Down].map(command)
}

/// The six shape tools that have letters by default.
pub(super) const LETTERED: [Shape; 6] = [
    Shape::Rect,
    Shape::Ellipse,
    Shape::Diamond,
    Shape::Pill,
    Shape::Cylinder,
    Shape::Parallelogram,
];

/// The hint line for a board in `situation` (§3.4). `applies` says whether
/// a command does something on the board as it stands, which drops the
/// parts a read-only board or the selection cannot use.
pub fn hint(situation: Situation, keys: &Keys, applies: impl Fn(Command) -> bool) -> Vec<Segment> {
    let mut line = Vec::new();
    if !situation.focused {
        return line;
    }
    // A part for `commands`, read by `read`, when every one of them
    // applies and has a key.
    let part = |commands: &[Command], read: &dyn Fn(&[Command]) -> Vec<String>, words| {
        let usable = commands.iter().all(|command| applies(*command));
        let keys = if usable { read(commands) } else { Vec::new() };
        (!keys.is_empty()).then_some(Segment { keys, words })
    };
    let first = |commands: &[Command]| keys.each(commands);
    let arrows = |commands: &[Command]| keys.arrows(commands.try_into().expect("four"));
    let digits = |_: &[Command]| keys.digits();
    let fixed = |key: &str, words| Segment {
        keys: vec![key.to_owned()],
        words,
    };
    let plain = |words| Segment {
        keys: Vec::new(),
        words,
    };
    if situation.editing {
        line.extend(part(&[Command::CommitLabel], &first, "done"));
        line.extend(part(
            &[Command::ConnectNext],
            &first,
            "done and add the next shape",
        ));
        line.push(fixed("esc", "done"));
        return line;
    }
    if situation.seeking {
        line.extend([
            plain("type to find"),
            fixed("←→↑↓", "move"),
            fixed("↵", "connect"),
            fixed("esc", "give up"),
        ]);
        return line;
    }
    if situation.space || situation.tool == Tool::Hand {
        line.push(plain("drag to pan"));
        return line;
    }
    match situation.tool {
        Tool::Shape(_) | Tool::Text => {
            line.extend([plain("click to place"), plain("drag to size")]);
            // Tab adds the armed shape; with the text tool it adds a
            // rectangle, which is not what is armed.
            if matches!(situation.tool, Tool::Shape(_)) {
                match situation.selected {
                    Selected::Nothing => {
                        line.extend(part(&[Command::ConnectNext], &first, "add one"));
                    }
                    Selected::OneShape => {
                        let words = "add it after the selection";
                        line.extend(part(&[Command::ConnectNext], &first, words));
                    }
                    Selected::Shapes | Selected::Connectors => {}
                }
            }
            // The tool's own key locks it (D3); a tool with no key has
            // no part.
            let words = if situation.locked {
                "again to unlock"
            } else {
                "again to lock"
            };
            line.extend(part(&[Command::Tool(situation.tool)], &first, words));
            line.push(fixed("esc", "cancel"));
        }
        Tool::Connector => {
            line.extend([
                plain("drag from one shape to another"),
                fixed("esc", "cancel"),
            ]);
        }
        Tool::Select | Tool::Hand => match situation.selected {
            Selected::Nothing => {
                line.extend(part(&[Command::ConnectNext], &first, "add a shape"));
                let tools = LETTERED.map(|shape| Command::Tool(Tool::Shape(shape)));
                line.extend(part(&tools, &first, "pick a shape"));
                if situation.space_shows_keys {
                    line.push(fixed("Space", "show keys"));
                }
                line.extend(part(&[Command::Keys], &first, "all keys"));
            }
            Selected::OneShape => {
                line.extend(part(&[Command::ConnectNext], &first, "next shape"));
                line.extend(part(&directions(Command::Connect), &arrows, "connect"));
                line.extend(part(&[Command::EditLabel], &first, "label"));
                line.extend(part(&colours(), &digits, "colour"));
                line.extend(part(&[Command::Shape(Shape::Diamond)], &first, "reshape"));
                line.extend(part(&[Command::Actions], &first, "actions"));
            }
            Selected::Shapes => {
                line.extend(part(&colours(), &digits, "colour"));
                line.extend(part(&directions(Command::Move), &arrows, "move"));
                let align = applies(Command::Align(Alignment::Left));
                let words = if align {
                    "align and distribute"
                } else {
                    "actions"
                };
                line.extend(part(&[Command::Actions], &first, words));
                line.extend(part(&[Command::Delete], &first, "delete"));
            }
            Selected::Connectors => {
                line.extend(part(&colours(), &digits, "colour"));
                line.extend(part(&[Command::EditLabel], &first, "label"));
                let words = if applies(Command::EdgeRouting) {
                    "route, heads, reverse"
                } else {
                    "actions"
                };
                line.extend(part(&[Command::Actions], &first, words));
                line.extend(part(&[Command::Delete], &first, "delete"));
            }
        },
    }
    line
}

/// One row of the keys sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetRow {
    /// The section of `docs/reference/boards.md` it belongs to.
    pub group: &'static str,
    pub keys: Vec<String>,
    pub words: String,
    /// Its commands do nothing for the selection as it stands.
    pub dimmed: bool,
}

/// What one row of the sheet lists.
enum Entry {
    /// A board command, under its palette title, with every key bound to
    /// it.
    One(Command),
    /// A command for each direction, under one title.
    Arrows([Command; 4], &'static str),
    /// A gesture, which no keymap holds, or several that do one thing.
    Fixed(&'static [&'static str], &'static str),
    /// A gesture spelled one way on macOS and another elsewhere.
    Platformed {
        mac: &'static str,
        linux: &'static str,
        words: &'static str,
    },
    /// App commands that act on a board too, after the keys of the board
    /// command they stand in for, which dims the row.
    App(&'static [&'static str], &'static str, Option<Command>),
}

/// The sheet's groups, in the order of `docs/reference/boards.md`, and
/// what each lists. Every board command is in it (a test holds it), and
/// one with no key drops out of the sheet.
fn entries() -> Vec<(&'static str, Vec<Entry>)> {
    use Entry::{App, Arrows, Fixed, One, Platformed};
    // The lettered shapes in the order of their keys, then the two that
    // have none.
    let shapes = |command: fn(Shape) -> Command| {
        LETTERED
            .into_iter()
            .chain([Shape::Rounded, Shape::Document])
            .map(move |shape| One(command(shape)))
    };
    let resize = [Grow::Narrower, Grow::Wider, Grow::Shorter, Grow::Taller].map(Command::Resize);
    let mut tools = vec![
        One(Command::Tool(Tool::Select)),
        One(Command::Tool(Tool::Hand)),
    ];
    tools.extend(shapes(|shape| Command::Tool(Tool::Shape(shape))));
    tools.extend([
        One(Command::Tool(Tool::Text)),
        One(Command::Tool(Tool::Connector)),
    ]);
    tools.extend(shapes(Command::Shape));
    let mut colour: Vec<Entry> = colours().into_iter().map(One).collect();
    colour.extend(
        <wren_board::Fill as wren_board::Vocabulary>::ALL
            .iter()
            .map(|fill| One(Command::Fill(*fill))),
    );
    colour.push(One(Command::StrokeDashed));
    vec![
        ("Tools", tools),
        (
            "Build",
            vec![
                One(Command::ConnectNext),
                Arrows(directions(Command::Connect), "Connect in that direction"),
                Arrows(directions(Command::Move), "Move one cell"),
                Arrows(resize, "Narrower, wider, shorter, taller"),
                One(Command::EditLabel),
                One(Command::CommitLabel),
                Fixed(&["Double-click"], "Edit a label, or add a shape"),
            ],
        ),
        (
            "Select",
            vec![
                Arrows(directions(Command::Select), "Select the nearest shape"),
                Arrows(
                    directions(Command::ExtendSelection),
                    "Add the nearest shape",
                ),
                Arrows(
                    directions(Command::SelectConnector),
                    "Select the connector on that side",
                ),
                One(Command::SelectAll),
                One(Command::SelectConnected),
                One(Command::SelectNone),
                Fixed(&["esc"], "Put the tool away, then clear the selection"),
                Fixed(&["Click"], "Select one"),
                Platformed {
                    mac: "⇧+click",
                    linux: "Shift+click",
                    words: "Add or remove one",
                },
                Fixed(&["Drag"], "Marquee, from empty canvas"),
            ],
        ),
        (
            "Edit",
            vec![
                One(Command::Delete),
                One(Command::Duplicate),
                One(Command::Copy),
                One(Command::Cut),
                One(Command::Paste),
                One(Command::Undo),
                One(Command::Redo),
                One(Command::EdgeRouting),
                One(Command::EdgeStart),
                One(Command::EdgeEnd),
                One(Command::EdgeReverse),
                // The duplicate-drag key, as `wren_canvas` reads it.
                Platformed {
                    mac: "⌥+drag",
                    linux: "Ctrl+drag",
                    words: "Move copies",
                },
            ],
        ),
        ("Colour", colour),
        // No key by default, so the group shows only once one is bound.
        (
            "Text size",
            wren_canvas::TEXT_SIZES
                .into_iter()
                .map(|size| One(Command::TextSize(size)))
                .collect(),
        ),
        (
            "Arrange",
            vec![
                One(Command::Arrange(Stack::Front)),
                One(Command::Arrange(Stack::Back)),
                One(Command::Arrange(Stack::Forward)),
                One(Command::Arrange(Stack::Backward)),
                One(Command::Align(Alignment::Left)),
                One(Command::Align(Alignment::Center)),
                One(Command::Align(Alignment::Right)),
                One(Command::Align(Alignment::Top)),
                One(Command::Align(Alignment::Middle)),
                One(Command::Align(Alignment::Bottom)),
                One(Command::Distribute(Axis::Horizontal)),
                One(Command::Distribute(Axis::Vertical)),
            ],
        ),
        (
            "View",
            vec![
                One(Command::ZoomFit),
                One(Command::ZoomSelection),
                App(
                    &["font.increase", "font.decrease", "font.reset"],
                    "Zoom in, out, to 100 %",
                    None,
                ),
                Platformed {
                    mac: "⌘+scroll",
                    linux: "Ctrl+scroll",
                    words: "Zoom at the pointer",
                },
                Fixed(&["Scroll", "Space+drag", "Middle-drag"], "Pan"),
            ],
        ),
        (
            "Menus",
            vec![
                One(Command::Actions),
                Fixed(&["Right-click"], "Actions for what is under the pointer"),
                // Find and the outline open it on a board (§5.18 rule 3).
                App(
                    &["navigation.find", "navigation.outline"],
                    "Jump to node…",
                    Some(Command::Jump),
                ),
                One(Command::Keys),
            ],
        ),
    ]
}

/// The keys sheet (§3.5): every board key, grouped, with a row dimmed
/// when `applies` says its commands do nothing for the selection.
pub fn sheet(keys: &Keys, applies: impl Fn(Command) -> bool) -> Vec<SheetRow> {
    let title = |command: Command| {
        definitions()
            .find(|definition| definition.id == CommandId::Board(command))
            .map_or("", |definition| definition.label)
    };
    let mut rows = Vec::new();
    for (group, entries) in entries() {
        for entry in entries {
            let (keys, words, commands): (Vec<String>, String, Vec<Command>) = match entry {
                Entry::One(command) => {
                    (keys.all(command), title(command).to_owned(), vec![command])
                }
                Entry::Arrows(commands, words) => {
                    (keys.arrows(commands), words.to_owned(), commands.to_vec())
                }
                Entry::Fixed(gestures, words) => (
                    gestures
                        .iter()
                        .map(|gesture| (*gesture).to_owned())
                        .collect(),
                    words.to_owned(),
                    Vec::new(),
                ),
                Entry::Platformed { mac, linux, words } => (
                    vec![match keys.platform {
                        Platform::Mac => mac,
                        Platform::Linux => linux,
                    }
                    .to_owned()],
                    words.to_owned(),
                    Vec::new(),
                ),
                Entry::App(actions, words, stands_for) => {
                    let own = stands_for
                        .map(|command| keys.all(command))
                        .unwrap_or_default();
                    let app = actions.iter().flat_map(|action| keys.app(action));
                    (
                        own.into_iter().chain(app).collect(),
                        words.to_owned(),
                        stands_for.into_iter().collect(),
                    )
                }
            };
            if keys.is_empty() {
                continue;
            }
            let dimmed = !commands.is_empty() && !commands.iter().any(|command| applies(*command));
            rows.push(SheetRow {
                group,
                keys,
                words,
                dimmed,
            });
        }
    }
    rows
}

impl App {
    /// Whether `command` does something on the open board as it stands:
    /// the app enables it, and a command the actions palette offers for a
    /// selection is offered for this one. A colour with nothing selected
    /// picks the colour new shapes take (discovery/board-controls.md D4).
    /// The hint line drops what does not apply and the keys sheet dims it.
    fn board_applies(&self, offers: &[wren_canvas::Offer], command: Command) -> bool {
        let picks_next = matches!(command, Command::Color(_))
            && self.board.as_ref().is_some_and(|view| {
                let board = view.board();
                !view
                    .machine()
                    .selection()
                    .iter()
                    .any(|element| element.exists(board))
            });
        self.command_enabled(CommandId::Board(command))
            && (command.offer_group().is_none()
                || picks_next
                || offers.iter().any(|offer| offer.command == command))
    }

    /// The open board's hint line; none with `board-hints = off`.
    pub(super) fn board_hint(&self) -> Vec<Segment> {
        let Some(view) = self
            .board
            .as_ref()
            .filter(|_| self.preferences.settings.board_hints)
        else {
            return Vec::new();
        };
        let machine = view.machine();
        let settings = &self.preferences.settings;
        let situation = Situation {
            focused: machine.focused(),
            editing: machine.editing().is_some(),
            seeking: machine.seeking().is_some(),
            tool: machine.tool(),
            locked: machine.locked(),
            space: machine.space(),
            space_shows_keys: settings.board_toolbar
                && settings.board_key_badges == wren_core::settings::KeyBadges::Hold,
            selected: Selected::of(view.board(), machine.selection()),
        };
        let offers = view.offers();
        let keys = Keys::new(&self.preferences.keymap, Platform::current());
        hint(situation, &keys, |command| {
            self.board_applies(&offers, command)
        })
    }

    /// The open board's keys sheet, the rows `query` matches.
    pub(super) fn board_sheet(&self, query: &str) -> Vec<SheetRow> {
        let Some(view) = &self.board else {
            return Vec::new();
        };
        let offers = view.offers();
        let keys = Keys::new(&self.preferences.keymap, Platform::current());
        let mut rows = sheet(&keys, |command| self.board_applies(&offers, command));
        rows.retain(|row| {
            let text = format!("{} {} {}", row.group, row.words, row.keys.join(" "));
            wren_core::index::fuzzy_score(query, &text).is_some()
        });
        rows
    }
}

/// How many of the groups, of `heights` in order, each of at most `count`
/// columns takes, so the tallest column is as short as it can be with
/// every group whole and in order.
fn split(heights: &[f32], count: usize) -> Vec<usize> {
    fn best(heights: &[f32], count: usize) -> (f32, Vec<usize>) {
        let total: f32 = heights.iter().sum();
        if count <= 1 || heights.len() <= 1 {
            return (total, vec![heights.len()]);
        }
        let mut found = (total, vec![heights.len()]);
        let mut first = 0.0;
        for taken in 1..heights.len() {
            first += heights[taken - 1];
            let (rest, mut split) = best(&heights[taken..], count - 1);
            let tallest = first.max(rest);
            if tallest < found.0 {
                split.insert(0, taken);
                found = (tallest, split);
            }
        }
        found
    }
    best(heights, count).1
}

/// A row's height on the sheet, and a group header's.
const SHEET_ROW: f32 = 23.0;
const SHEET_HEADER: f32 = 31.0;

impl App {
    /// The keys sheet over the board (§3.5): the palette's frame, scrim
    /// and filter, with the rows in as many columns as the window holds,
    /// so at 1280 by 800 the whole sheet shows without scrolling.
    pub(super) fn keys_sheet_view<'a>(
        &'a self,
        base: Element<'a, Message>,
        palette: &'a Palette,
    ) -> Element<'a, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let top = palette_top(self.viewport.height);
        let width = (self.viewport.width - 32.0).clamp(120.0, SHEET_WIDTH);
        let rows = self.board_sheet(&palette.query);

        let header = row![
            container(
                text("Board keys")
                    .size(12)
                    .font(fonts::SEMIBOLD)
                    .color(theme::color(colors.accent)),
            )
            .padding([3, 8])
            .style(move |_| chrome::surface(definition, colors.selected, 4.0, false)),
            text_input("Filter the board's keys…", &palette.query)
                .id("palette-query")
                .size(15)
                .padding([4, 0])
                .on_input_maybe(self.loading.is_none().then_some(Message::PaletteQuery))
                .style(move |_, status| chrome::input(definition, status, false)),
            chrome::keycap("esc", definition),
        ]
        .spacing(8)
        .align_y(iced::Center);

        // Groups stay whole and in order, read down each column and then
        // across, each column holding about as many lines as the others.
        let mut groups: Vec<(&'static str, Vec<&SheetRow>)> = Vec::new();
        for row in &rows {
            match groups.last_mut() {
                Some((group, members)) if *group == row.group => members.push(row),
                _ => groups.push((row.group, vec![row])),
            }
        }
        let columns = ((width - 28.0) / SHEET_COLUMN).floor().clamp(1.0, 3.0) as usize;
        let heights: Vec<f32> = groups
            .iter()
            .map(|(_, members)| SHEET_HEADER + SHEET_ROW * members.len() as f32)
            .collect();
        let mut stacks: Vec<Vec<(&'static str, Vec<&SheetRow>)>> = Vec::new();
        let mut groups = groups.into_iter();
        for taken in split(&heights, columns) {
            stacks.push(groups.by_ref().take(taken).collect());
        }
        // A filter that leaves fewer groups keeps the columns' width.
        stacks.resize_with(columns, Vec::new);
        let keys_width = ((width - 28.0) / columns as f32 * 0.55).max(80.0);
        let cap = move |key: String, dimmed: bool| {
            container(
                chrome::label(key, 11, if dimmed { colors.faint } else { colors.muted })
                    .font(fonts::UI),
            )
            .padding([1, 6])
            .style(move |_| chrome::surface(definition, colors.input, 4.0, true))
        };
        let mut body = row![].spacing(24);
        for stack in stacks {
            let mut lane = column![].width(Fill);
            for (group, members) in stack {
                lane = lane.push(
                    container(
                        text(group.to_uppercase())
                            .size(GROUP_HEADER_SIZE)
                            .font(fonts::SEMIBOLD)
                            .color(theme::color(colors.muted)),
                    )
                    .padding([GROUP_HEADER_PADDING, 0.0]),
                );
                for member in members {
                    let words = ellipsis(
                        member.words.clone(),
                        13.0,
                        theme::color(if member.dimmed {
                            colors.faint
                        } else {
                            colors.text
                        }),
                    );
                    let mut keys = row![].spacing(4);
                    for key in &member.keys {
                        keys = keys.push(cap(key.clone(), member.dimmed));
                    }
                    lane = lane.push(
                        row![
                            container(words).width(Fill),
                            container(keys.wrap().vertical_spacing(4))
                                .max_width(keys_width)
                                .align_x(iced::alignment::Horizontal::Right),
                        ]
                        .spacing(12)
                        .padding([2, 0])
                        .align_y(iced::Center),
                    );
                }
            }
            body = body.push(lane);
        }
        let body: Element<'_, Message> = if rows.is_empty() {
            container(
                text("No matches.")
                    .size(13)
                    .color(theme::color(colors.muted)),
            )
            .padding(14)
            .into()
        } else {
            container(body).padding([4, 14]).into()
        };

        let hint = |key: String, action: &'static str| {
            row![
                chrome::keycap_owned(key, definition),
                text(action).size(11).color(theme::color(colors.muted))
            ]
            .spacing(6)
            .align_y(iced::Center)
        };
        let mut footer = row![].spacing(14).align_y(iced::Center);
        if let Some(own) = self.shortcut_label(Command::Keys.key()) {
            footer = footer.push(hint(own, "close"));
        }
        footer = footer.push(hint("esc".into(), "close")).push(
            text("Type to filter")
                .size(11)
                .color(theme::color(colors.muted)),
        );

        let panel = column![
            chrome::bottom_rule(container(header).padding([12, 14]), definition),
            scrollable(body)
                .id("palette-results")
                .height(Fill)
                .style(move |_, status| chrome::scrollbar(definition, status)),
            chrome::top_rule(container(footer).padding([9, 14]), definition),
        ];
        let dialog = container(panel)
            .id(format!("palette-{}", palette.mode.key()))
            .max_height((self.viewport.height - top - 8.0).max(24.0))
            .width(width)
            .clip(true)
            .style(move |_| chrome::dialog(definition));
        let scrim = mouse_area(
            container(iced::widget::space())
                .width(Fill)
                .height(Fill)
                .style(move |_| chrome::scrim(definition)),
        )
        .on_press(Message::PaletteClose);
        stack![
            base,
            opaque(scrim),
            container(opaque(dialog))
                .padding(iced::Padding {
                    top,
                    right: 16.0,
                    bottom: 8.0,
                    left: 16.0
                })
                .width(Fill)
                .align_x(iced::alignment::Horizontal::Center)
                .height(Fill)
        ]
        .into()
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::collections::HashSet;

    use wren_core::keybind::{self, Layer};

    use super::*;
    use crate::app::commands::Scope;

    /// The default keymap on `platform`, with the user's `keybind` lines.
    pub(in crate::app) fn keymap(platform: Platform, user: &[(&str, &str)]) -> Keymap {
        let user: Vec<(String, String)> = user
            .iter()
            .map(|(trigger, action)| ((*trigger).to_owned(), (*action).to_owned()))
            .collect();
        let (keymap, warnings) = Keymap::build(
            keybind::default_bindings(platform)
                .into_iter()
                .chain(keybind::default_board_bindings(platform)),
            &user,
            |action| {
                CommandId::parse(action).map(|id| match id.scope() {
                    Scope::Board => Layer::Board,
                    Scope::App | Scope::Editor => Layer::Base,
                })
            },
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        keymap
    }

    const IDLE: Situation = Situation {
        focused: true,
        editing: false,
        seeking: false,
        tool: Tool::Select,
        locked: false,
        space: false,
        space_shows_keys: true,
        selected: Selected::Nothing,
    };

    fn with(selected: Selected) -> Situation {
        Situation { selected, ..IDLE }
    }

    /// The line as text, `keys words` parts joined by ` · `.
    fn line(segments: &[Segment]) -> String {
        segments
            .iter()
            .map(|segment| {
                let mut part = segment.keys.join(" ");
                if !part.is_empty() {
                    part.push(' ');
                }
                part + segment.words
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn shown(situation: Situation, platform: Platform, user: &[(&str, &str)]) -> String {
        let keymap = keymap(platform, user);
        line(&hint(situation, &Keys::new(&keymap, platform), |_| true))
    }

    /// Whether `command` changes the board, as `board_mutates` has it, for
    /// a read-only board's `applies`.
    fn reads_only(command: Command) -> bool {
        matches!(
            command,
            Command::Tool(Tool::Select | Tool::Hand)
                | Command::Select(_)
                | Command::ExtendSelection(_)
                | Command::SelectConnector(_)
                | Command::SelectAll
                | Command::SelectConnected
                | Command::SelectNone
                | Command::Copy
                | Command::ZoomFit
                | Command::ZoomSelection
                | Command::Actions
                | Command::Jump
                | Command::Keys
        )
    }

    #[test]
    fn each_state_of_the_table_has_its_keys_on_linux() {
        let linux = |situation| shown(situation, Platform::Linux, &[]);
        assert_eq!(
            linux(IDLE),
            "Tab add a shape · R O D U Y P pick a shape · Space show keys · ? all keys"
        );
        assert_eq!(
            linux(with(Selected::OneShape)),
            "Tab next shape · Ctrl+Arrow connect · ↵ label · 0–7 colour · Shift+D reshape · / actions"
        );
        assert_eq!(
            linux(with(Selected::Shapes)),
            "0–7 colour · Alt+Arrow move · / align and distribute · ⌫ delete"
        );
        assert_eq!(
            linux(with(Selected::Connectors)),
            "0–7 colour · ↵ label · / route, heads, reverse · ⌫ delete"
        );
        let armed = |tool, selected| Situation {
            tool,
            selected,
            ..IDLE
        };
        let rect = Tool::Shape(Shape::Rect);
        assert_eq!(
            linux(armed(rect, Selected::Nothing)),
            "click to place · drag to size · Tab add one · R again to lock · esc cancel"
        );
        assert_eq!(
            linux(armed(rect, Selected::OneShape)),
            "click to place · drag to size · Tab add it after the selection · R again to lock · \
             esc cancel"
        );
        assert_eq!(
            linux(armed(rect, Selected::Shapes)),
            "click to place · drag to size · R again to lock · esc cancel"
        );
        assert_eq!(
            linux(armed(Tool::Text, Selected::OneShape)),
            "click to place · drag to size · T again to lock · esc cancel"
        );
        assert_eq!(
            linux(armed(Tool::Connector, Selected::Nothing)),
            "drag from one shape to another · esc cancel"
        );
        assert_eq!(linux(armed(Tool::Hand, Selected::OneShape)), "drag to pan");
        let space = Situation {
            space: true,
            ..with(Selected::Shapes)
        };
        assert_eq!(linux(space), "drag to pan");
    }

    #[test]
    fn the_lock_and_the_space_badges_are_named_only_where_they_hold() {
        let linux = |situation| shown(situation, Platform::Linux, &[]);
        let armed = |tool, locked| Situation {
            tool,
            locked,
            ..IDLE
        };
        let diamond = Tool::Shape(Shape::Diamond);
        assert_eq!(
            linux(armed(diamond, true)),
            "click to place · drag to size · Tab add one · D again to unlock · esc cancel"
        );
        // The two shapes with no key have no key to lock them by.
        assert_eq!(
            linux(armed(Tool::Shape(Shape::Rounded), false)),
            "click to place · drag to size · Tab add one · esc cancel"
        );
        // A rebound tool key is the one named.
        assert_eq!(
            shown(
                armed(diamond, false),
                Platform::Linux,
                &[("k", "board.tool.diamond")]
            ),
            "click to place · drag to size · Tab add one · K again to lock · esc cancel"
        );
        // With no dock, or badges on or off, Space shows no keys.
        let quiet = Situation {
            space_shows_keys: false,
            ..IDLE
        };
        assert_eq!(
            linux(quiet),
            "Tab add a shape · R O D U Y P pick a shape · ? all keys"
        );
    }

    #[test]
    fn mod_is_command_on_macos() {
        let mac = |situation| shown(situation, Platform::Mac, &[]);
        assert_eq!(
            mac(with(Selected::OneShape)),
            "Tab next shape · ⌘Arrow connect · ↵ label · 0–7 colour · ⇧D reshape · / actions"
        );
        assert_eq!(
            mac(with(Selected::Shapes)),
            "0–7 colour · ⌥Arrow move · / align and distribute · ⌫ delete"
        );
        let editing = Situation {
            editing: true,
            ..with(Selected::OneShape)
        };
        assert_eq!(
            mac(editing),
            "⌘↵ done · Tab done and add the next shape · esc done"
        );
    }

    #[test]
    fn a_label_and_keyboard_connecting_have_keys_of_their_own() {
        let editing = Situation {
            editing: true,
            ..with(Selected::OneShape)
        };
        assert_eq!(
            shown(editing, Platform::Linux, &[]),
            "Ctrl+↵ done · Tab done and add the next shape · esc done"
        );
        let seeking = Situation {
            seeking: true,
            ..with(Selected::OneShape)
        };
        assert_eq!(
            shown(seeking, Platform::Linux, &[]),
            "type to find · ←→↑↓ move · ↵ connect · esc give up"
        );
    }

    #[test]
    fn an_unfocused_board_has_no_hint() {
        let unfocused = Situation {
            focused: false,
            ..with(Selected::OneShape)
        };
        assert_eq!(shown(unfocused, Platform::Linux, &[]), "");
    }

    #[test]
    fn a_read_only_board_hints_only_what_it_takes() {
        let keymap = keymap(Platform::Linux, &[]);
        let keys = Keys::new(&keymap, Platform::Linux);
        let read_only = |situation| line(&hint(situation, &keys, reads_only));
        assert_eq!(read_only(IDLE), "Space show keys · ? all keys");
        assert_eq!(read_only(with(Selected::OneShape)), "/ actions");
        assert_eq!(read_only(with(Selected::Shapes)), "/ actions");
        assert_eq!(read_only(with(Selected::Connectors)), "/ actions");
    }

    #[test]
    fn a_rebound_key_shows_as_rebound_and_an_unbound_one_drops_out() {
        let user = [
            ("x", "board.delete"),
            ("tab", "board.unbind"),
            ("f1", "board.keys"),
            ("shift+/", "board.unbind"),
            ("o", "board.unbind"),
            ("ctrl+left", "board.unbind"),
            ("ctrl+h", "board.connect.left"),
            ("9", "board.color.blue"),
        ];
        let linux = |situation| shown(situation, Platform::Linux, &user);
        assert_eq!(
            linux(IDLE),
            "R D U Y P pick a shape · Space show keys · F1 all keys"
        );
        // The four connect keys are no longer the arrows with one
        // modifier, so each is shown, and the colours are no longer 0–7.
        assert_eq!(
            linux(with(Selected::OneShape)),
            "Ctrl+H Ctrl+→ Ctrl+↑ Ctrl+↓ connect · ↵ label · 0 1 2 3 4 5 6 9 colour · \
             Shift+D reshape · / actions"
        );
        assert_eq!(
            linux(with(Selected::Shapes)),
            "0 1 2 3 4 5 6 9 colour · Alt+Arrow move · / align and distribute · X delete"
        );
    }

    #[test]
    fn a_part_whose_command_does_not_apply_drops_out() {
        let keymap = keymap(Platform::Linux, &[]);
        let keys = Keys::new(&keymap, Platform::Linux);
        // Two connectors: no label to edit; a text element: no reshape.
        let no_label = |command| command != Command::EditLabel;
        assert_eq!(
            line(&hint(with(Selected::Connectors), &keys, no_label)),
            "0–7 colour · / route, heads, reverse · ⌫ delete"
        );
        let text = |command| !matches!(command, Command::Shape(_));
        assert_eq!(
            line(&hint(with(Selected::OneShape), &keys, text)),
            "Tab next shape · Ctrl+Arrow connect · ↵ label · 0–7 colour · / actions"
        );
        // Two shapes, which align and do not distribute.
        let unaligned = |command| !matches!(command, Command::Align(_));
        assert_eq!(
            line(&hint(with(Selected::Shapes), &keys, unaligned)),
            "0–7 colour · Alt+Arrow move · / actions · ⌫ delete"
        );
    }

    fn rows(
        platform: Platform,
        user: &[(&str, &str)],
        applies: impl Fn(Command) -> bool,
    ) -> Vec<SheetRow> {
        let keymap = keymap(platform, user);
        sheet(&Keys::new(&keymap, platform), applies)
    }

    fn row<'a>(rows: &'a [SheetRow], words: &str) -> Option<&'a SheetRow> {
        rows.iter().find(|row| row.words == words)
    }

    #[test]
    fn every_board_command_has_a_place_on_the_sheet() {
        let mut placed = HashSet::new();
        for (_, entries) in entries() {
            for entry in entries {
                match entry {
                    Entry::One(command) => assert!(placed.insert(command), "{command:?} twice"),
                    Entry::Arrows(commands, _) => {
                        for command in commands {
                            assert!(placed.insert(command), "{command:?} twice");
                        }
                    }
                    Entry::App(_, _, Some(command)) => {
                        assert!(placed.insert(command), "{command:?} twice");
                    }
                    Entry::Fixed(..) | Entry::Platformed { .. } | Entry::App(..) => {}
                }
            }
        }
        let all: HashSet<Command> = Command::all().collect();
        assert_eq!(placed, all);
    }

    /// The label sizes have no key, so they are not on the sheet until
    /// one is bound; then they have a group of their own after Colour.
    #[test]
    fn a_text_size_shows_on_the_sheet_once_it_has_a_key() {
        let plain = rows(Platform::Linux, &[], |_| true);
        assert!(plain.iter().all(|row| row.group != "Text size"));
        let bound = rows(
            Platform::Linux,
            &[("Alt+Shift+9", "board.text_size.28")],
            |_| true,
        );
        let sized: Vec<&SheetRow> = bound
            .iter()
            .filter(|row| row.group == "Text size")
            .collect();
        assert_eq!(sized.len(), 1);
        assert_eq!(sized[0].words, "Text size: 28");
        let mut groups: Vec<&str> = bound.iter().map(|row| row.group).collect();
        groups.dedup();
        let colour = groups.iter().position(|group| *group == "Colour").unwrap();
        assert_eq!(groups[colour + 1], "Text size");
    }

    #[test]
    fn the_sheet_groups_every_bound_key_as_the_reference_does() {
        let sheet = rows(Platform::Linux, &[], |_| true);
        let mut groups: Vec<&str> = sheet.iter().map(|row| row.group).collect();
        groups.dedup();
        assert_eq!(
            groups,
            ["Tools", "Build", "Select", "Edit", "Colour", "Arrange", "View", "Menus"]
        );
        let keys = |words: &str| row(&sheet, words).map(|row| row.keys.clone());
        assert_eq!(keys("Connector tool"), Some(vec!["C".into(), "A".into()]));
        assert_eq!(
            keys("Delete selection"),
            Some(vec!["⌫".into(), "Del".into()])
        );
        assert_eq!(
            keys("Redo"),
            Some(vec!["Ctrl+Shift+Z".into(), "Ctrl+Y".into()])
        );
        assert_eq!(
            keys("Connect in that direction"),
            Some(vec!["Ctrl+Arrow".into()])
        );
        assert_eq!(
            keys("Select the nearest shape"),
            Some(vec!["Arrows".into()])
        );
        assert_eq!(
            keys("Narrower, wider, shorter, taller"),
            Some(vec!["Alt+Shift+Arrow".into()])
        );
        assert_eq!(keys("Colour: blue"), Some(vec!["7".into()]));
        assert_eq!(keys("Board keys"), Some(vec!["?".into()]));
        assert_eq!(
            keys("Zoom in, out, to 100 %"),
            Some(vec!["Alt+=".into(), "Alt+-".into(), "Alt+0".into()])
        );
        assert_eq!(keys("Move copies"), Some(vec!["Ctrl+drag".into()]));
        let jump: Vec<_> = sheet
            .iter()
            .filter(|row| row.words == "Jump to node…")
            .map(|row| row.keys.clone())
            .collect();
        assert_eq!(jump, [vec!["Alt+F".to_owned(), "Alt+Shift+O".to_owned()]]);
        // No default key: the two unlettered tools, align, distribute and
        // the fills are left out until a key is bound.
        for words in [
            "Rounded rectangle tool",
            "Align left",
            "Distribute vertically",
            "Fill: none",
        ] {
            assert!(row(&sheet, words).is_none(), "{words}");
        }
        let bound = rows(
            Platform::Linux,
            &[("ctrl+alt+l", "board.align.left")],
            |_| true,
        );
        assert_eq!(
            row(&bound, "Align left").map(|row| row.keys.clone()),
            Some(vec!["Ctrl+Alt+L".into()])
        );
        let mac = rows(Platform::Mac, &[], |_| true);
        assert_eq!(
            row(&mac, "Connect in that direction").map(|row| row.keys.clone()),
            Some(vec!["⌘Arrow".into()])
        );
        assert_eq!(
            row(&mac, "Move copies").map(|row| row.keys.clone()),
            Some(vec!["⌥+drag".into()])
        );
    }

    use super::super::board::tests::{file, fixture, open_board, open_markdown, FLOW};
    use super::super::commands::{Command as Run, Disposition};
    use super::super::palette::Mode;
    use super::super::Message;
    use crate::keys::{Input, Nav};

    /// Give the open board its widget's size and the keyboard.
    pub(in crate::app) fn focus(app: &mut App) {
        for message in [
            wren_canvas::Message::Resized {
                width: 800.0,
                height: 600.0,
            },
            wren_canvas::Message::Input(
                app.board.as_ref().unwrap().epoch(),
                wren_canvas::Input::Focus(true),
            ),
        ] {
            let _ = app.update(Message::Board(message));
        }
    }

    fn run(app: &mut App, command: Command) -> Disposition {
        app.dispatch(Run::Run(CommandId::Board(command)))
            .processed
            .disposition
    }

    fn sheet_open(app: &App) -> bool {
        app.palette
            .as_ref()
            .is_some_and(|palette| palette.mode == Mode::BoardKeys)
    }

    #[test]
    fn the_footer_hint_follows_the_board_its_focus_and_the_setting() {
        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        assert!(app.board_hint().is_empty(), "the board has no keyboard yet");
        focus(&mut app);
        let platform = Platform::current();
        let keymap = app.preferences.keymap.clone();
        let label = |command: Command| keymap.label(command.key(), platform).unwrap();
        let first = app.board_hint();
        assert_eq!(first[0].keys, [label(Command::ConnectNext)]);
        assert_eq!(first.last().unwrap().keys, ["?"]);
        let _ = run(&mut app, Command::Select(Dir::Down));
        let one = app.board_hint();
        assert_eq!(one[0].words, "next shape");
        let _ = run(&mut app, Command::SelectAll);
        assert_eq!(app.board_hint()[0].words, "colour");
        let _ = run(&mut app, Command::Tool(Tool::Shape(Shape::Diamond)));
        assert_eq!(app.board_hint()[0].words, "click to place");

        app.preferences.settings.board_hints = false;
        assert!(app.board_hint().is_empty());
        app.preferences.settings.board_hints = true;
        let _ = app.update(Message::Board(wren_canvas::Message::Input(
            app.board.as_ref().unwrap().epoch(),
            wren_canvas::Input::Focus(false),
        )));
        assert!(app.board_hint().is_empty(), "unfocused");
        open_markdown(&mut app, "Note");
        assert!(app.board_hint().is_empty());
    }

    #[test]
    fn the_keys_sheet_opens_on_a_board_dims_and_closes_back_to_it() {
        let (directory, mut app) = fixture();
        open_markdown(&mut app, "Note");
        assert_eq!(run(&mut app, Command::Keys), Disposition::Blocked);
        assert!(app.palette.is_none());

        open_board(&mut app, "Flow");
        focus(&mut app);
        let dimmed = |app: &App, words: &str| {
            app.palette
                .as_ref()
                .unwrap()
                .results
                .iter()
                .find(|row| row.title == words)
                .map(|row| !row.enabled)
        };
        assert_eq!(run(&mut app, Command::Keys), Disposition::Completed);
        assert!(sheet_open(&app));
        assert_eq!(dimmed(&app, "Delete selection"), Some(true));
        // With nothing selected a colour picks the one new shapes take.
        assert_eq!(dimmed(&app, "Colour: red"), Some(false));
        assert_eq!(dimmed(&app, "Change shape to rectangle"), Some(true));
        assert_eq!(dimmed(&app, "Select all"), Some(false));
        assert_eq!(dimmed(&app, "Rectangle tool"), Some(false));

        // `?` in the filter closes it, and the board has the keyboard.
        let _ = app.palette_query("?".into());
        assert!(app.palette.is_none());
        assert_eq!(app.column, super::super::columns::Column::Editor);

        // With a shape selected, its rows are lit; a filter narrows them.
        let _ = run(&mut app, Command::Select(Dir::Down));
        let _ = run(&mut app, Command::Keys);
        assert_eq!(dimmed(&app, "Delete selection"), Some(false));
        assert_eq!(dimmed(&app, "Colour: red"), Some(false));
        assert_eq!(dimmed(&app, "Align left"), None, "no key");
        let _ = app.palette_query("colour".into());
        let results = &app.palette.as_ref().unwrap().results;
        assert!(
            results.iter().all(|row| row.group == "Colour"),
            "{:?}",
            results.len()
        );
        assert_eq!(app.board.as_ref().unwrap().machine().selection().len(), 1);

        // Enter closes it too, and so does Escape.
        let _ = app.update(Message::Key(Input::Nav(Nav::Enter)));
        assert!(app.palette.is_none());
        let _ = run(&mut app, Command::Keys);
        let _ = app.update(Message::Key(Input::Nav(Nav::Escape)));
        assert!(app.palette.is_none());

        // A read-only board opens it, with what would change it dimmed.
        std::fs::write(
            file(&directory, "Locked.wren-board"),
            FLOW.replacen("schema = 1\n", "schema = 1\nrequires = [\"frames\"]\n", 1),
        )
        .unwrap();
        let _ = app.apply_core(wren_core::ops::Op::Reconcile).unwrap();
        open_board(&mut app, "Locked");
        assert_eq!(run(&mut app, Command::Keys), Disposition::Completed);
        assert!(sheet_open(&app));
        assert_eq!(dimmed(&app, "Rectangle tool"), Some(true));
        assert_eq!(dimmed(&app, "Hand tool"), Some(false));
    }

    #[test]
    fn a_rebound_sheet_key_closes_the_sheet() {
        let (_directory, mut app) = fixture();
        app.preferences.settings.keybinds = vec![("f1".into(), "board.keys".into())];
        app.preferences.rebuild_keymap();
        open_board(&mut app, "Flow");
        focus(&mut app);
        let _ = run(&mut app, Command::Keys);
        assert!(sheet_open(&app));
        let f1 = wren_core::keybind::parse_trigger("f1").unwrap();
        let _ = app.update(Message::Key(Input::Accel(f1)));
        assert!(app.palette.is_none());
    }

    #[test]
    fn groups_split_into_columns_whole_and_as_even_as_they_can() {
        assert_eq!(
            split(&[19.0, 8.0, 11.0, 9.0, 9.0, 5.0, 5.0, 5.0], 3),
            [2, 2, 4]
        );
        assert_eq!(split(&[1.0, 1.0, 1.0], 1), [3]);
        assert_eq!(split(&[5.0], 3), [1]);
        assert_eq!(split(&[3.0, 3.0, 3.0, 3.0], 2), [2, 2]);
    }

    #[test]
    fn a_row_is_dimmed_when_none_of_its_commands_applies() {
        let colours_only = |command| matches!(command, Command::Color(_));
        let sheet = rows(Platform::Linux, &[], colours_only);
        let dimmed = |words: &str| row(&sheet, words).map(|row| row.dimmed);
        assert_eq!(dimmed("Colour: red"), Some(false));
        assert_eq!(dimmed("Delete selection"), Some(true));
        assert_eq!(dimmed("Connect in that direction"), Some(true));
        // A gesture and an app command stand for no board command.
        assert_eq!(dimmed("Pan"), Some(false));
        assert_eq!(dimmed("Zoom in, out, to 100 %"), Some(false));
        assert_eq!(dimmed("Jump to node…"), Some(true));
    }
}
