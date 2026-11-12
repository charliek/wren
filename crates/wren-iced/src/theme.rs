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
pub fn editor_colors(definition: theme::Theme) -> (wren_editor::Style, wren_editor::Palette) {
    let c = definition.colors;
    let style = wren_editor::Style {
        background: color(c.editor),
        value: color(c.body),
        text_selection: color(c.text_selection),
        gutter: color(c.faint),
        gutter_current: color(c.body),
        current_line: color(c.current_line),
        code_block: color(c.code_block),
        caret: color(c.accent),
        strike: color(c.faint),
    };
    let palette = wren_editor::Palette {
        text: color(c.text),
        muted: color(c.muted),
        faint: color(c.faint),
        accent: color(c.accent),
        heading: color(c.heading),
        heading2: color(c.heading2),
        link: color(c.link),
        code: color(c.code),
        quote: color(c.quote),
        completed: color(c.completed),
    };
    (style, palette)
}

/// The colours a board is drawn in under `definition`, in the canvas and in
/// an export alike.
pub fn board_palette(definition: theme::Theme) -> wren_board::Palette {
    theme::board_palette(&definition.colors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_board_takes_the_editor_surface_and_text_of_every_theme() {
        for definition in theme::BUILT_INS {
            let c = definition.colors;
            let palette = board_palette(*definition);
            let rgb = |Rgb(r, g, b): Rgb| wren_board::Rgba::rgb(r, g, b);
            assert_eq!(palette.canvas, rgb(c.editor), "{}", definition.id);
            assert_eq!(palette.ink, rgb(c.text), "{}", definition.id);
            assert_eq!(palette.issue, rgb(c.danger), "{}", definition.id);
            assert_eq!(
                palette.dark,
                definition.appearance == theme::Appearance::Dark,
                "{}",
                definition.id
            );
        }
    }

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
    fn syntax_and_band_fields_use_the_theme_syntax_tokens() {
        for theme in theme::BUILT_INS {
            let c = theme.colors;
            let (style, palette) = editor_colors(*theme);
            assert_eq!(style.text_selection, color(c.text_selection));
            assert_eq!(style.code_block, color(c.code_block));
            assert_eq!(style.gutter_current, color(c.body));
            assert_eq!(palette.heading, color(c.heading));
            assert_eq!(palette.heading2, color(c.heading2));
            assert_eq!(palette.link, color(c.link));
            assert_eq!(palette.code, color(c.code));
            assert_eq!(palette.quote, color(c.quote));
            assert_eq!(palette.completed, color(c.completed));
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
