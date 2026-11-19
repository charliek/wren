//! The board's commands: what a key bound in the board layer, a palette
//! row, a menu item or the harness asks a focused board to do. The host
//! keeps the bindings and the palette rows; each command's key is its
//! stable name there (the plan's §5.19 command table).

use wren_board::edit::{Alignment, Axis, Stack};
use wren_board::{Dir, Fill, Shape, Token};

/// One board command. [`Command::all`] lists the 64 of Phase 2, the 25 of
/// Phase 3, the keys sheet, then the four label sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    /// `board.tool.*`: arm a tool.
    Tool(Tool),
    /// `board.shape.*`: reshape the selected nodes.
    Shape(Shape),
    /// `board.label.edit`
    EditLabel,
    /// `board.label.commit`
    CommitLabel,
    /// `board.connect.next`: Tab.
    ConnectNext,
    /// `board.connect.<dir>`
    Connect(Dir),
    /// `board.select.<dir>`: walk.
    Select(Dir),
    /// `board.select.extend_<dir>`
    ExtendSelection(Dir),
    /// `board.select.connector_<dir>`
    SelectConnector(Dir),
    /// `board.select.all`
    SelectAll,
    /// `board.select.connected`
    SelectConnected,
    /// `board.select.none`
    SelectNone,
    /// `board.move.<dir>`
    Move(Dir),
    /// `board.resize.*`
    Resize(Grow),
    /// `board.delete`
    Delete,
    /// `board.duplicate`
    Duplicate,
    /// `board.copy`
    Copy,
    /// `board.cut`
    Cut,
    /// `board.paste`
    Paste,
    /// `board.undo`
    Undo,
    /// `board.redo`
    Redo,
    /// `board.edge.routing`: straight or elbow.
    EdgeRouting,
    /// `board.edge.start`: the head at the `from` end.
    EdgeStart,
    /// `board.edge.end`: the head at the `to` end.
    EdgeEnd,
    /// `board.edge.reverse`
    EdgeReverse,
    /// `board.stroke.dashed`: solid or dashed.
    StrokeDashed,
    /// `board.zoom.fit`
    ZoomFit,
    /// `board.zoom.selection`
    ZoomSelection,
    /// `board.arrange.*`: restack the selection among its siblings.
    Arrange(Stack),
    /// `board.align.*`
    Align(Alignment),
    /// `board.distribute.*`
    Distribute(Axis),
    /// `board.color.*`: the colour token of the selected elements.
    Color(Token),
    /// `board.fill.*`: the fill of the selected shapes.
    Fill(Fill),
    /// `board.text_size.*`: the label size of the selected nodes, one of
    /// [`TEXT_SIZES`] (discovery/board-controls.md §3.9).
    TextSize(u8),
    /// `board.actions`: the actions palette for the selection.
    Actions,
    /// `board.jump`: the jump-to-node palette.
    Jump,
    /// `board.keys`: the sheet of every board key.
    Keys,
}

/// The label sizes the commands set, in board units: what the style bar's
/// menu and the actions palette offer. A file may hold any other size.
pub const TEXT_SIZES: [u8; 4] = [12, 14, 20, 28];

/// What the pointer does on a board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    #[default]
    Select,
    Hand,
    Shape(Shape),
    Text,
    Connector,
}

/// One step of `board.resize.*`, with the top-left corner fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grow {
    Wider,
    Narrower,
    Taller,
    Shorter,
}

/// Every command and its key, in the order of the plan's table.
const TABLE: [(Command, &str); 94] = {
    use Command as C;
    use Dir::{Down, Left, Right, Up};
    [
        (C::Tool(Tool::Select), "board.tool.select"),
        (C::Tool(Tool::Hand), "board.tool.hand"),
        (C::Tool(Tool::Shape(Shape::Rect)), "board.tool.rectangle"),
        (C::Tool(Tool::Shape(Shape::Ellipse)), "board.tool.ellipse"),
        (C::Tool(Tool::Shape(Shape::Diamond)), "board.tool.diamond"),
        (C::Tool(Tool::Shape(Shape::Pill)), "board.tool.pill"),
        (C::Tool(Tool::Shape(Shape::Cylinder)), "board.tool.cylinder"),
        (
            C::Tool(Tool::Shape(Shape::Parallelogram)),
            "board.tool.parallelogram",
        ),
        (C::Tool(Tool::Shape(Shape::Rounded)), "board.tool.rounded"),
        (C::Tool(Tool::Shape(Shape::Document)), "board.tool.document"),
        (C::Tool(Tool::Text), "board.tool.text"),
        (C::Tool(Tool::Connector), "board.tool.connector"),
        (C::Shape(Shape::Rect), "board.shape.rectangle"),
        (C::Shape(Shape::Ellipse), "board.shape.ellipse"),
        (C::Shape(Shape::Diamond), "board.shape.diamond"),
        (C::Shape(Shape::Pill), "board.shape.pill"),
        (C::Shape(Shape::Cylinder), "board.shape.cylinder"),
        (C::Shape(Shape::Parallelogram), "board.shape.parallelogram"),
        (C::Shape(Shape::Rounded), "board.shape.rounded"),
        (C::Shape(Shape::Document), "board.shape.document"),
        (C::EditLabel, "board.label.edit"),
        (C::CommitLabel, "board.label.commit"),
        (C::ConnectNext, "board.connect.next"),
        (C::Connect(Left), "board.connect.left"),
        (C::Connect(Right), "board.connect.right"),
        (C::Connect(Up), "board.connect.up"),
        (C::Connect(Down), "board.connect.down"),
        (C::Select(Left), "board.select.left"),
        (C::Select(Right), "board.select.right"),
        (C::Select(Up), "board.select.up"),
        (C::Select(Down), "board.select.down"),
        (C::ExtendSelection(Left), "board.select.extend_left"),
        (C::ExtendSelection(Right), "board.select.extend_right"),
        (C::ExtendSelection(Up), "board.select.extend_up"),
        (C::ExtendSelection(Down), "board.select.extend_down"),
        (C::SelectConnector(Left), "board.select.connector_left"),
        (C::SelectConnector(Right), "board.select.connector_right"),
        (C::SelectConnector(Up), "board.select.connector_up"),
        (C::SelectConnector(Down), "board.select.connector_down"),
        (C::SelectAll, "board.select.all"),
        (C::SelectConnected, "board.select.connected"),
        (C::SelectNone, "board.select.none"),
        (C::Move(Left), "board.move.left"),
        (C::Move(Right), "board.move.right"),
        (C::Move(Up), "board.move.up"),
        (C::Move(Down), "board.move.down"),
        (C::Resize(Grow::Wider), "board.resize.wider"),
        (C::Resize(Grow::Narrower), "board.resize.narrower"),
        (C::Resize(Grow::Taller), "board.resize.taller"),
        (C::Resize(Grow::Shorter), "board.resize.shorter"),
        (C::Delete, "board.delete"),
        (C::Duplicate, "board.duplicate"),
        (C::Copy, "board.copy"),
        (C::Cut, "board.cut"),
        (C::Paste, "board.paste"),
        (C::Undo, "board.undo"),
        (C::Redo, "board.redo"),
        (C::EdgeRouting, "board.edge.routing"),
        (C::EdgeStart, "board.edge.start"),
        (C::EdgeEnd, "board.edge.end"),
        (C::EdgeReverse, "board.edge.reverse"),
        (C::StrokeDashed, "board.stroke.dashed"),
        (C::ZoomFit, "board.zoom.fit"),
        (C::ZoomSelection, "board.zoom.selection"),
        (C::Arrange(Stack::Front), "board.arrange.front"),
        (C::Arrange(Stack::Back), "board.arrange.back"),
        (C::Arrange(Stack::Forward), "board.arrange.forward"),
        (C::Arrange(Stack::Backward), "board.arrange.backward"),
        (C::Align(Alignment::Left), "board.align.left"),
        (C::Align(Alignment::Center), "board.align.center"),
        (C::Align(Alignment::Right), "board.align.right"),
        (C::Align(Alignment::Top), "board.align.top"),
        (C::Align(Alignment::Middle), "board.align.middle"),
        (C::Align(Alignment::Bottom), "board.align.bottom"),
        (
            C::Distribute(Axis::Horizontal),
            "board.distribute.horizontal",
        ),
        (C::Distribute(Axis::Vertical), "board.distribute.vertical"),
        (C::Color(Token::Neutral), "board.color.neutral"),
        (C::Color(Token::Red), "board.color.red"),
        (C::Color(Token::Orange), "board.color.orange"),
        (C::Color(Token::Yellow), "board.color.yellow"),
        (C::Color(Token::Green), "board.color.green"),
        (C::Color(Token::Cyan), "board.color.cyan"),
        (C::Color(Token::Purple), "board.color.purple"),
        (C::Color(Token::Blue), "board.color.blue"),
        (C::Fill(Fill::None), "board.fill.none"),
        (C::Fill(Fill::Tint), "board.fill.tint"),
        (C::Fill(Fill::Solid), "board.fill.solid"),
        (C::Actions, "board.actions"),
        (C::Jump, "board.jump"),
        (C::Keys, "board.keys"),
        (C::TextSize(12), "board.text_size.12"),
        (C::TextSize(14), "board.text_size.14"),
        (C::TextSize(20), "board.text_size.20"),
        (C::TextSize(28), "board.text_size.28"),
    ]
};

impl Command {
    /// Every command, in the order of the plan's table.
    pub fn all() -> impl Iterator<Item = Self> {
        TABLE.iter().map(|(command, _)| *command)
    }

    /// The stable name, such as `board.tool.select`.
    pub fn key(self) -> &'static str {
        TABLE
            .iter()
            .find(|(command, _)| *command == self)
            .map_or("", |(_, key)| *key)
    }

    /// The group the actions palette lists the command under when it
    /// offers it for a selection (§5.17 rule 3), or `None` for a command
    /// it never offers for one. [`crate::BoardView::offers`] says which of
    /// these apply to the selection there is.
    pub fn offer_group(self) -> Option<&'static str> {
        Some(match self {
            Self::Shape(_) => "Shape",
            Self::Color(_) => "Colour",
            Self::Fill(_) => "Fill",
            Self::TextSize(_) => "Text size",
            Self::StrokeDashed => "Stroke",
            Self::EdgeRouting | Self::EdgeStart | Self::EdgeEnd | Self::EdgeReverse => "Connector",
            Self::Arrange(_) => "Arrange",
            Self::Align(_) => "Align",
            Self::Distribute(_) => "Distribute",
            Self::EditLabel | Self::Duplicate | Self::Copy | Self::Cut | Self::Delete => "Edit",
            _ => return None,
        })
    }

    /// The command a key names.
    pub fn parse(key: &str) -> Option<Self> {
        TABLE
            .iter()
            .find(|(_, name)| *name == key)
            .map(|(command, _)| *command)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_command_has_one_board_key_and_parses_back() {
        let commands: HashSet<Command> = Command::all().collect();
        let keys: HashSet<&str> = Command::all().map(Command::key).collect();
        assert_eq!((commands.len(), keys.len()), (94, 94));
        for command in Command::all() {
            assert!(command.key().starts_with("board."), "{command:?}");
            assert_eq!(Command::parse(command.key()), Some(command));
        }
        assert_eq!(Command::parse("note.new"), None);
    }

    #[test]
    fn every_colour_token_and_fill_has_a_command() {
        use wren_board::Vocabulary;
        for &token in Token::ALL {
            let command = Command::Color(token);
            assert_eq!(command.key(), format!("board.color.{}", token.name()));
        }
        for &fill in Fill::ALL {
            let command = Command::Fill(fill);
            assert_eq!(command.key(), format!("board.fill.{}", fill.name()));
        }
    }

    #[test]
    fn every_text_size_has_a_command() {
        let sizes: Vec<u8> = Command::all()
            .filter_map(|command| match command {
                Command::TextSize(size) => Some(size),
                _ => None,
            })
            .collect();
        assert_eq!(sizes, TEXT_SIZES);
        for size in TEXT_SIZES {
            let command = Command::TextSize(size);
            assert_eq!(command.key(), format!("board.text_size.{size}"));
            assert_eq!(command.offer_group(), Some("Text size"));
        }
    }

    #[test]
    fn every_shape_has_a_tool_and_a_reshape_command() {
        for &shape in <Shape as wren_board::Vocabulary>::ALL {
            for command in [Command::Tool(Tool::Shape(shape)), Command::Shape(shape)] {
                assert!(
                    Command::all().any(|listed| listed == command),
                    "{command:?}"
                );
            }
        }
    }
}
