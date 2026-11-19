//! The controls in the footer under a board (discovery/board-controls.md
//! §3.6): undo and redo, then zoom out, the zoom figure, which fits the
//! board, and zoom in. Each runs the command its key runs through
//! `App::dispatch`. [`controls`] decides them as a pure function;
//! `App::board_footer_controls_view` draws them. They show with
//! `board-toolbar = on`; with it off the footer shows the figure alone.

use iced::widget::{button, container, mouse_area, row, text, tooltip};
use iced::{Border, Element};
use wren_canvas::Command as BoardCommand;
use wren_core::theme::Rgb;

use super::commands::{Command, CommandId};
use super::{App, Message};
use crate::{chrome, fonts, theme};

/// One control of the footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Undo,
    Redo,
    ZoomOut,
    /// The zoom figure, which fits the board.
    Zoom,
    ZoomIn,
}

impl Control {
    /// The command it runs: the one its key runs. The text-size commands
    /// zoom a board.
    pub fn command(self) -> CommandId {
        match self {
            Self::Undo => CommandId::Board(BoardCommand::Undo),
            Self::Redo => CommandId::Board(BoardCommand::Redo),
            Self::ZoomOut => CommandId::TextSmaller,
            Self::Zoom => CommandId::Board(BoardCommand::ZoomFit),
            Self::ZoomIn => CommandId::TextBigger,
        }
    }

    /// What its tooltip calls it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::ZoomOut => "Zoom out",
            Self::Zoom => "Zoom to fit",
            Self::ZoomIn => "Zoom in",
        }
    }

    /// The id of the widget around it.
    pub fn id(self) -> &'static str {
        match self {
            Self::Undo => "board-footer-undo",
            Self::Redo => "board-footer-redo",
            Self::ZoomOut => "board-footer-zoom-out",
            Self::Zoom => "board-footer-zoom",
            Self::ZoomIn => "board-footer-zoom-in",
        }
    }
}

/// Every id the footer's controls take, for the harness to look for.
#[cfg(any(test, feature = "smoke-test"))]
pub const IDS: [&str; 5] = [
    "board-footer-undo",
    "board-footer-redo",
    "board-footer-zoom-out",
    "board-footer-zoom",
    "board-footer-zoom-in",
];

/// One control as the footer shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub control: Control,
    /// Whether a click does anything: undo and redo are greyed and inert
    /// with nothing to undo or redo.
    pub enabled: bool,
    /// Its command's first key, from the keymap.
    pub key: Option<String>,
}

impl Button {
    /// What the tooltip says: the name, then the key.
    pub fn tip(&self) -> String {
        match &self.key {
            Some(key) => format!("{}   {key}", self.control.name()),
            None => self.control.name().to_owned(),
        }
    }

    /// What a click on it sends: its command, as its key does, or for a
    /// greyed one only what closes the bars' menus.
    pub fn message(&self) -> Message {
        if self.enabled {
            Message::Command(Command::Run(self.control.command()))
        } else {
            Message::DockMenu(None)
        }
    }
}

/// What the footer's controls read of the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Situation {
    /// The board may be edited: a read-only board has no undo or redo.
    pub writable: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// The footer's controls for `situation`, left to right, each with its
/// key from `key`.
pub fn controls(situation: Situation, key: impl Fn(CommandId) -> Option<String>) -> Vec<Button> {
    let edits = [
        (Control::Undo, situation.can_undo),
        (Control::Redo, situation.can_redo),
    ];
    let zooms = [
        (Control::ZoomOut, true),
        (Control::Zoom, true),
        (Control::ZoomIn, true),
    ];
    edits
        .into_iter()
        .filter(|_| situation.writable)
        .chain(zooms)
        .map(|(control, enabled)| Button {
            control,
            enabled,
            key: key(control.command()),
        })
        .collect()
}

impl App {
    /// The open board's footer controls; none with `board-toolbar = off`.
    pub(super) fn board_footer_controls(&self) -> Vec<Button> {
        let Some(view) = self
            .board
            .as_ref()
            .filter(|_| self.preferences.settings.board_toolbar)
        else {
            return Vec::new();
        };
        controls(
            Situation {
                writable: view.writable(),
                can_undo: view.can_undo(),
                can_redo: view.can_redo(),
            },
            |id| self.shortcut_label(id.key()),
        )
    }

    /// The footer's controls, the zoom figure `zoom` among them.
    pub(super) fn board_footer_controls_view(&self, zoom: String) -> Element<'_, Message> {
        let definition = self.definition;
        let colors = definition.colors;
        let mut controls = row![].spacing(2).align_y(iced::Center);
        let mut last = None;
        for each in self.board_footer_controls() {
            // A gap between undo and redo and the zoom.
            if last == Some(Control::Redo) {
                controls = controls.push(iced::widget::Space::new().width(10));
            }
            last = Some(each.control);
            let ink = if each.enabled {
                colors.muted
            } else {
                faded(colors.muted, colors.editor)
            };
            let face: Element<'_, Message> = match each.control {
                Control::Zoom => text(zoom.clone())
                    .size(11)
                    .font(fonts::UI)
                    .color(theme::color(ink))
                    .wrapping(text::Wrapping::None)
                    .into(),
                Control::Undo => chrome::paths_icon(UNDO, 14.0, ink),
                Control::Redo => chrome::paths_icon(REDO, 14.0, ink),
                Control::ZoomOut => chrome::paths_icon(MINUS, 14.0, ink),
                Control::ZoomIn => chrome::paths_icon(PLUS, 14.0, ink),
            };
            let enabled = each.enabled;
            let pressed = button(container(face).center_y(iced::Fill))
                .height(17)
                .padding([0, if each.control == Control::Zoom { 5 } else { 3 }])
                .style(move |_, status| footer_button(definition.colors.control, status, enabled));
            // A control takes every press on it, as the bars' own ground
            // does, greyed or not and whatever the button: one that fell
            // through would reach the board as a click off it, which
            // takes the keyboard from the board and puts its tool away.
            let taken = if each.enabled {
                mouse_area(pressed.on_press(each.message()))
            } else {
                mouse_area(pressed)
                    .on_press(each.message())
                    .interaction(iced::mouse::Interaction::Idle)
            };
            let pressed: Element<'_, Message> = taken
                .on_right_press(Message::DockMenu(None))
                .on_middle_press(Message::DockMenu(None))
                .into();
            controls = controls.push(tooltip(
                container(pressed).id(each.control.id()),
                container(
                    text(each.tip())
                        .size(11)
                        .font(fonts::UI)
                        .color(theme::color(colors.editor)),
                )
                .padding([4, 7])
                .style(move |_| chrome::surface(definition, colors.text, 5.0, false)),
                tooltip::Position::Top,
            ));
        }
        controls.into()
    }
}

/// A footer control: clear until the pointer is on it, and never lit
/// while it does nothing.
fn footer_button(hover: Rgb, status: button::Status, enabled: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: (enabled && hovered).then(|| theme::color(hover).into()),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// `ink` faded two thirds of the way into `ground`: a control that does
/// nothing now.
fn faded(ink: Rgb, ground: Rgb) -> Rgb {
    let mix = |a: u8, b: u8| ((f32::from(a) + 2.0 * f32::from(b)) / 3.0).round() as u8;
    Rgb(
        mix(ink.0, ground.0),
        mix(ink.1, ground.1),
        mix(ink.2, ground.2),
    )
}

const UNDO: &str = "<path d='M9 14 4 9l5-5'/><path d='M4 9h10.5a5.5 5.5 0 0 1 0 11H11'/>";
const REDO: &str = "<path d='m15 14 5-5-5-5'/><path d='M20 9H9.5a5.5 5.5 0 0 0 0 11H13'/>";
const MINUS: &str = "<path d='M6 12h12'/>";
const PLUS: &str = "<path d='M12 6v12M6 12h12'/>";

#[cfg(test)]
mod tests {
    use super::*;

    const EDITABLE: Situation = Situation {
        writable: true,
        can_undo: false,
        can_redo: false,
    };

    fn keyed(id: CommandId) -> Option<String> {
        match id {
            CommandId::Board(BoardCommand::Undo) => Some("Ctrl+Z".into()),
            CommandId::TextBigger => Some("Alt+=".into()),
            _ => None,
        }
    }

    fn shown(buttons: &[Button]) -> Vec<(Control, bool)> {
        buttons
            .iter()
            .map(|button| (button.control, button.enabled))
            .collect()
    }

    #[test]
    fn undo_and_redo_are_greyed_with_nothing_to_undo_or_redo() {
        let quiet = controls(EDITABLE, keyed);
        assert_eq!(
            shown(&quiet),
            [
                (Control::Undo, false),
                (Control::Redo, false),
                (Control::ZoomOut, true),
                (Control::Zoom, true),
                (Control::ZoomIn, true),
            ]
        );
        let undoable = controls(
            Situation {
                can_undo: true,
                ..EDITABLE
            },
            keyed,
        );
        assert_eq!(
            shown(&undoable)[..2],
            [(Control::Undo, true), (Control::Redo, false)]
        );
        let both = controls(
            Situation {
                can_undo: true,
                can_redo: true,
                ..EDITABLE
            },
            keyed,
        );
        assert!(both.iter().all(|button| button.enabled));
    }

    #[test]
    fn a_read_only_board_has_the_zoom_and_no_undo() {
        let read_only = controls(
            Situation {
                writable: false,
                can_undo: true,
                can_redo: true,
            },
            keyed,
        );
        assert_eq!(
            shown(&read_only),
            [
                (Control::ZoomOut, true),
                (Control::Zoom, true),
                (Control::ZoomIn, true),
            ]
        );
    }

    #[test]
    fn each_control_runs_its_key_s_command_and_names_its_key() {
        let buttons = controls(EDITABLE, keyed);
        let commands: Vec<CommandId> = buttons
            .iter()
            .map(|button| button.control.command())
            .collect();
        assert_eq!(
            commands,
            [
                CommandId::Board(BoardCommand::Undo),
                CommandId::Board(BoardCommand::Redo),
                CommandId::TextSmaller,
                CommandId::Board(BoardCommand::ZoomFit),
                CommandId::TextBigger,
            ]
        );
        assert_eq!(buttons[0].tip(), "Undo   Ctrl+Z");
        assert_eq!(buttons[1].tip(), "Redo");
        assert_eq!(buttons[4].tip(), "Zoom in   Alt+=");
        for button in &buttons {
            assert!(IDS.contains(&button.control.id()), "{:?}", button.control);
        }
    }

    #[test]
    fn the_app_s_footer_follows_the_board_s_history_and_the_setting() {
        use super::super::board::tests::{fixture, open_board};
        use super::super::board_keys::tests::focus;

        let (_directory, mut app) = fixture();
        open_board(&mut app, "Flow");
        focus(&mut app);
        let enabled = |app: &App| shown(&app.board_footer_controls());
        assert_eq!(enabled(&app)[0], (Control::Undo, false));
        // A colour on a shape is one undo step; the undo button runs it.
        let _ = app.update(Message::Command(Command::Run(CommandId::Board(
            BoardCommand::SelectAll,
        ))));
        let _ = app.update(Message::Command(Command::Run(CommandId::Board(
            BoardCommand::Color(wren_board::Token::Red),
        ))));
        assert_eq!(enabled(&app)[0], (Control::Undo, true));
        let _ = app.update(Message::Command(Command::Run(Control::Undo.command())));
        assert!(!app.board.as_ref().unwrap().can_undo());
        assert_eq!(
            enabled(&app)[..2],
            [(Control::Undo, false), (Control::Redo, true)]
        );
        // The zoom buttons step the board's zoom, and the figure fits it.
        let zoom = |app: &App| app.board.as_ref().unwrap().viewport().zoom;
        let fitted = zoom(&app);
        let _ = app.update(Message::Command(Command::Run(Control::ZoomIn.command())));
        assert!(zoom(&app) > fitted);
        let _ = app.update(Message::Command(Command::Run(Control::Zoom.command())));
        assert_eq!(zoom(&app), fitted);
        let _ = app.update(Message::Command(Command::Run(Control::ZoomOut.command())));
        assert!(zoom(&app) < fitted);
        // With the setting off, the footer has none.
        app.preferences.settings.board_toolbar = false;
        assert!(app.board_footer_controls().is_empty());
    }
}
