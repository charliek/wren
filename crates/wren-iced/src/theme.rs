use iced::theme::Palette;
use iced::{Color, Theme};
use wren_core::theme::{self, Rgb};

pub fn color(Rgb(red, green, blue): Rgb) -> Color {
    Color::from_rgb8(red, green, blue)
}

pub fn from_definition(definition: theme::Theme) -> Theme {
    let colors = definition.colors;
    Theme::custom(
        definition.name,
        Palette {
            background: color(colors.editor),
            text: color(colors.text),
            primary: color(colors.accent),
            success: color(colors.success),
            warning: color(colors.warning),
            danger: color(colors.danger),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolkit_palette_is_derived_from_semantic_tokens() {
        for definition in theme::BUILT_INS {
            let palette = from_definition(*definition).palette();
            assert_eq!(palette.background, color(definition.colors.editor));
            assert_eq!(palette.text, color(definition.colors.text));
            assert_eq!(palette.primary, color(definition.colors.accent));
        }
    }

    #[test]
    fn an_additional_palette_uses_the_same_adapter() {
        let definition = theme::Theme {
            id: "custom",
            name: "Additional theme",
            colors: theme::Colors {
                accent: Rgb(0xa0, 0xb0, 0xff),
                ..theme::DARK.colors
            },
            ..theme::DARK
        };
        assert_eq!(
            from_definition(definition).palette().primary,
            color(definition.colors.accent)
        );
    }
}
