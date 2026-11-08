use iced::widget::{button, container, scrollable, svg, text, text_input};
use iced::{Border, Element, Shadow};
use wren_core::theme::{Rgb, Theme};

use crate::{fonts, theme};

#[derive(Clone, Copy)]
pub enum Icon {
    Bird,
    Folder,
    Chevron,
    Search,
    Focus,
    Plus,
    Back,
    Forward,
    More,
}

pub fn icon<'a, Message: 'a>(kind: Icon, size: f32, tint: Rgb) -> Element<'a, Message> {
    let paths = match kind {
        Icon::Bird => "<path d='M4 15c0-5 4-9 9-9 3 0 5 2 5 4l3 1-3 1c0 4-3 7-8 7H7l-3 2z'/><circle cx='15' cy='9.5' r='.8'/>",
        Icon::Folder => "<path d='M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z'/>",
        Icon::Chevron => "<path d='m6 9 6 6 6-6'/>",
        Icon::Search => "<circle cx='10.5' cy='10.5' r='6.5'/><path d='m16 16 4 4'/>",
        Icon::Focus => "<path d='M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5'/>",
        Icon::Plus => "<path d='M12 5v14M5 12h14'/>",
        Icon::Back => "<path d='m14 6-6 6 6 6'/>",
        Icon::Forward => "<path d='m10 6 6 6-6 6'/>",
        Icon::More => "<circle cx='5' cy='12' r='1'/><circle cx='12' cy='12' r='1'/><circle cx='19' cy='12' r='1'/>",
    };
    let source = format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#{:02x}{:02x}{:02x}' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'>{paths}</svg>", tint.0, tint.1, tint.2);
    svg(svg::Handle::from_memory(source.into_bytes()))
        .width(size)
        .height(size)
        .into()
}

pub fn label<'a>(value: impl text::IntoFragment<'a>, size: u32, color: Rgb) -> text::Text<'a> {
    text(value)
        .size(size)
        .color(theme::color(color))
        .wrapping(iced::widget::text::Wrapping::None)
}

pub fn surface(definition: Theme, background: Rgb, radius: f32, border: bool) -> container::Style {
    container::Style {
        text_color: Some(theme::color(definition.colors.text)),
        background: Some(theme::color(background).into()),
        border: Border {
            color: theme::color(definition.colors.control_border),
            width: if border { 1.0 } else { 0.0 },
            radius: radius.into(),
        },
        ..Default::default()
    }
}

/// The backdrop behind dialogs and the palette. Light themes dim less, so
/// the window recedes instead of turning gray.
pub fn scrim(definition: Theme) -> container::Style {
    let alpha = match definition.appearance {
        wren_core::theme::Appearance::Dark => 0.65,
        wren_core::theme::Appearance::Light => 0.32,
    };
    container::Style {
        background: Some(iced::Color::from_rgba(0.025, 0.028, 0.04, alpha).into()),
        ..Default::default()
    }
}

pub fn dialog(definition: Theme) -> container::Style {
    // tiny-skia's shadow pass ignores damage clipping and can cover the dialog.
    // The scrim and border provide separation consistently on both renderers.
    container::Style {
        border: Border {
            color: theme::color(definition.colors.dialog_border),
            width: 1.0,
            radius: 12.0.into(),
        },
        ..surface(definition, definition.colors.raised, 12.0, true)
    }
}

pub fn control(
    definition: Theme,
    status: button::Status,
    selected: bool,
    quiet: bool,
) -> button::Style {
    let colors = definition.colors;
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let disabled = matches!(status, button::Status::Disabled);
    button::Style {
        background: if selected || hovered {
            Some(
                theme::color(if selected {
                    colors.selected
                } else {
                    colors.raised
                })
                .into(),
            )
        } else if quiet {
            None
        } else {
            Some(theme::color(colors.control).into())
        },
        text_color: theme::color(if disabled {
            colors.faint
        } else if selected {
            colors.accent
        } else {
            colors.body
        }),
        border: Border {
            color: theme::color(if selected {
                colors.selected_border
            } else {
                colors.control_border
            }),
            width: if quiet { 0.0 } else { 1.0 },
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

pub fn list_button(definition: Theme, status: button::Status, selected: bool) -> button::Style {
    let mut style = control(definition, status, selected, true);
    style.text_color = theme::color(if status == button::Status::Disabled {
        definition.colors.faint
    } else {
        definition.colors.text
    });
    style
}

pub fn input(definition: Theme, status: text_input::Status, border: bool) -> text_input::Style {
    let colors = definition.colors;
    text_input::Style {
        background: theme::color(if border { colors.input } else { colors.raised }).into(),
        border: Border {
            color: theme::color(if matches!(status, text_input::Status::Focused { .. }) {
                colors.accent
            } else {
                colors.control_border
            }),
            width: if border { 1.0 } else { 0.0 },
            radius: 6.0.into(),
        },
        icon: theme::color(colors.faint),
        placeholder: theme::color(colors.faint),
        value: theme::color(colors.text),
        selection: theme::color(colors.selected),
    }
}

pub fn scrollbar(definition: Theme, status: scrollable::Status) -> scrollable::Style {
    let colors = definition.colors;
    let visible = !matches!(status, scrollable::Status::Active { .. });
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: theme::color(if visible {
                colors.selected_border
            } else {
                colors.control_border
            })
            .into(),
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: theme::color(colors.raised).into(),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: theme::color(colors.muted),
        },
    }
}

pub fn keycap_owned<'a, Message: 'a>(
    value: String,
    definition: Theme,
) -> container::Container<'a, Message> {
    container(label(value, 11, definition.colors.muted).font(fonts::UI))
        .padding([1, 6])
        .style(move |_| surface(definition, definition.colors.input, 4.0, true))
}

pub fn keycap<'a, Message: 'a>(
    value: &'a str,
    definition: Theme,
) -> container::Container<'a, Message> {
    container(label(value, 11, definition.colors.muted).font(fonts::UI))
        .padding([1, 6])
        .style(move |_| surface(definition, definition.colors.input, 4.0, true))
}

pub fn bottom_rule<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    definition: Theme,
) -> container::Container<'a, Message> {
    // Only a bottom edge, not a box around every pane/header.
    container(iced::widget::column![
        content.into(),
        container(iced::widget::space())
            .width(iced::Fill)
            .height(1)
            .style(move |_| surface(definition, definition.colors.divider, 0.0, false))
    ])
}

pub fn top_rule<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    definition: Theme,
) -> container::Container<'a, Message> {
    container(iced::widget::column![
        container(iced::widget::space())
            .width(iced::Fill)
            .height(1)
            .style(move |_| surface(definition, definition.colors.divider, 0.0, false)),
        content.into()
    ])
}

/// A workspace root for display, with the home directory shown as `~`.
pub fn display_path(root: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| root.strip_prefix(home).ok())
    {
        Some(relative) if relative.as_os_str().is_empty() => "~".into(),
        Some(relative) => format!("~/{}", relative.display()),
        None => root.display().to_string(),
    }
}
