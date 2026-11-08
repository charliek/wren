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

/// The editor's surface and Markdown highlighting colours for `definition`.
///
/// The syntax and band fields map onto existing theme colours until the
/// theme gains tokens of its own (`discovery/markdown-editor.md`, §6).
pub fn editor_colors(definition: theme::Theme) -> (wren_editor::Style, wren_editor::Palette) {
    let c = definition.colors;
    let style = wren_editor::Style {
        background: color(c.editor),
        value: color(c.body),
        text_selection: color(c.selected),
        gutter: color(c.faint),
        current_line: color(c.current_line),
        code_block: color(c.current_line),
        caret: color(c.accent),
        strike: color(c.faint),
    };
    let palette = wren_editor::Palette {
        text: color(c.text),
        muted: color(c.muted),
        faint: color(c.faint),
        accent: color(c.accent),
        heading: color(c.accent),
        heading2: color(c.accent),
        link: color(c.accent),
        code: color(c.success),
        quote: color(c.muted),
        completed: color(c.faint),
    };
    (style, palette)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_style_has_editor_background() {
        let (style, _) = editor_colors(theme::DARK);
        assert_eq!(style.background, color(theme::DARK.colors.editor));
    }

    #[test]
    fn editor_style_text_color_matches_theme() {
        let (dark_style, _) = editor_colors(theme::DARK);
        let (light_style, _) = editor_colors(theme::LIGHT);
        assert_ne!(dark_style.value, light_style.value);
        assert_eq!(dark_style.value, color(theme::DARK.colors.body));
        assert_eq!(light_style.value, color(theme::LIGHT.colors.body));
    }

    #[test]
    fn editor_style_maps_mockup_gutter_line_and_caret_tokens() {
        for theme in [theme::DARK, theme::LIGHT] {
            let (style, _) = editor_colors(theme);
            assert_eq!(style.gutter, color(theme.colors.faint));
            assert_eq!(style.current_line, color(theme.colors.current_line));
            assert_eq!(style.caret, color(theme.colors.accent));
            assert_eq!(style.strike, color(theme.colors.faint));
        }
    }

    #[test]
    fn syntax_and_band_fields_use_their_interim_theme_colours() {
        for theme in theme::BUILT_INS {
            let c = theme.colors;
            let (style, palette) = editor_colors(*theme);
            assert_eq!(style.text_selection, color(c.selected));
            assert_eq!(style.code_block, color(c.current_line));
            for heading in [palette.heading, palette.heading2, palette.link] {
                assert_eq!(heading, color(c.accent));
            }
            assert_eq!(palette.code, color(c.success));
            assert_eq!(palette.quote, color(c.muted));
            assert_eq!(palette.completed, color(c.faint));
        }
    }

    #[test]
    fn editor_palette_maps_the_semantic_text_tokens() {
        for theme in theme::BUILT_INS {
            let (_, palette) = editor_colors(*theme);
            let c = theme.colors;
            assert_eq!(palette.text, color(c.text));
            assert_eq!(palette.muted, color(c.muted));
            assert_eq!(palette.faint, color(c.faint));
            assert_eq!(palette.accent, color(c.accent));
        }
    }

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
