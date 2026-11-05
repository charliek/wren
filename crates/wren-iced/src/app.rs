use iced::theme::Mode;
use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill, Subscription, Task};
use wren_core::theme::{self as definitions, Theme};

use crate::theme;

#[derive(Debug, Clone, Default)]
pub struct Options {
    #[cfg(feature = "smoke-test")]
    pub smoke: Option<crate::smoke::Request>,
}

pub struct App {
    definition: Theme,
    toolkit_theme: iced::Theme,
    system_mode: Mode,
    follows_system: bool,
    #[cfg(feature = "smoke-test")]
    smoke: Option<crate::smoke::Request>,
}

#[derive(Debug, Clone)]
pub enum Message {
    SystemTheme(Mode),
    ChooseTheme(Option<Theme>),
    #[cfg(feature = "smoke-test")]
    Opened(iced::window::Id),
    #[cfg(feature = "smoke-test")]
    RendererReady(iced::window::Id, iced::system::Information),
    #[cfg(feature = "smoke-test")]
    Captured(iced::window::Screenshot),
}

impl App {
    pub fn new(_options: Options) -> (Self, Task<Message>) {
        #[cfg(feature = "smoke-test")]
        let definition = _options
            .smoke
            .as_ref()
            .map_or(definitions::DARK, |request| request.theme);
        #[cfg(not(feature = "smoke-test"))]
        let definition = definitions::DARK;

        #[cfg(feature = "smoke-test")]
        let follows_system = _options.smoke.is_none();
        #[cfg(not(feature = "smoke-test"))]
        let follows_system = true;

        (
            Self {
                definition,
                toolkit_theme: theme::from_definition(definition),
                system_mode: Mode::None,
                follows_system,
                #[cfg(feature = "smoke-test")]
                smoke: _options.smoke,
            },
            iced::system::theme().map(Message::SystemTheme),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SystemTheme(mode) => {
                self.system_mode = mode;
                if self.follows_system {
                    self.set_theme(theme::definition_for_system(mode));
                }
            }
            Message::ChooseTheme(definition) => {
                self.follows_system = definition.is_none();
                self.set_theme(
                    definition.unwrap_or_else(|| theme::definition_for_system(self.system_mode)),
                );
            }
            #[cfg(feature = "smoke-test")]
            Message::Opened(id) => {
                if self.smoke.is_some() {
                    return iced::system::information()
                        .map(move |information| Message::RendererReady(id, information));
                }
            }
            #[cfg(feature = "smoke-test")]
            Message::RendererReady(id, information) => {
                eprintln!("WREN_SMOKE_BACKEND={}", information.graphics_backend);
                eprintln!("WREN_SMOKE_ADAPTER={}", information.graphics_adapter);
                return iced::window::screenshot(id).map(Message::Captured);
            }
            #[cfg(feature = "smoke-test")]
            Message::Captured(capture) => {
                if let Some(request) = &self.smoke {
                    if let Err(error) = request.save(&capture) {
                        eprintln!("Wren smoke capture: {error}");
                    }
                }
                return iced::exit();
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let content = column![
            text("Wren").size(36).color(theme::color(colors.accent)),
            text("Development scaffold").size(24),
            text("Linux tooling and renderer verification.").size(16),
            text("Workspace editing and autosave are not implemented yet.")
                .color(theme::color(colors.muted)),
            text("This window does not open or write note files.")
                .color(theme::color(colors.muted)),
            row![
                button("System").on_press(Message::ChooseTheme(None)),
                button("Dark").on_press(Message::ChooseTheme(Some(definitions::DARK))),
                button("Light").on_press(Message::ChooseTheme(Some(definitions::LIGHT))),
            ]
            .spacing(8),
            text(self.definition.name).color(theme::color(colors.muted)),
        ]
        .spacing(16)
        .padding(32);

        container(content).center_x(Fill).center_y(Fill).into()
    }

    pub fn theme(&self) -> iced::Theme {
        self.toolkit_theme.clone()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let subscriptions = vec![iced::system::theme_changes().map(Message::SystemTheme)];
        #[cfg(feature = "smoke-test")]
        let subscriptions = {
            let mut subscriptions = subscriptions;
            if self.smoke.is_some() {
                subscriptions.push(iced::window::open_events().map(Message::Opened));
            }
            subscriptions
        };
        Subscription::batch(subscriptions)
    }

    fn set_theme(&mut self, definition: Theme) {
        self.definition = definition;
        self.toolkit_theme = theme::from_definition(definition);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_theme_stays_selected_until_system_mode_is_restored() {
        let (mut app, _) = App::new(Options::default());
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert_eq!(app.definition, definitions::LIGHT);
        let _ = app.update(Message::ChooseTheme(Some(definitions::DARK)));
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert_eq!(app.definition, definitions::DARK);
        let _ = app.update(Message::ChooseTheme(None));
        assert_eq!(app.definition, definitions::LIGHT);
    }
}
