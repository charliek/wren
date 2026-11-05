#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colors {
    pub editor: Rgb,
    pub titlebar: Rgb,
    pub pane: Rgb,
    pub notes: Rgb,
    pub input: Rgb,
    pub control: Rgb,
    pub raised: Rgb,
    pub selected: Rgb,
    pub current_line: Rgb,
    pub divider: Rgb,
    pub control_border: Rgb,
    pub dialog_border: Rgb,
    pub selected_border: Rgb,
    pub text: Rgb,
    pub body: Rgb,
    pub muted: Rgb,
    pub faint: Rgb,
    pub accent: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub danger: Rgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub appearance: Appearance,
    pub colors: Colors,
}

pub const DARK: Theme = Theme {
    id: "dark",
    name: "Wren Dark",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x14, 0x16, 0x1b),
        titlebar: Rgb(0x10, 0x12, 0x16),
        pane: Rgb(0x17, 0x19, 0x1f),
        notes: Rgb(0x16, 0x18, 0x1d),
        input: Rgb(0x12, 0x14, 0x19),
        control: Rgb(0x18, 0x1b, 0x21),
        raised: Rgb(0x1c, 0x1f, 0x26),
        selected: Rgb(0x2a, 0x31, 0x40),
        current_line: Rgb(0x19, 0x1d, 0x25),
        divider: Rgb(0x22, 0x26, 0x2e),
        control_border: Rgb(0x2a, 0x2f, 0x38),
        dialog_border: Rgb(0x33, 0x39, 0x44),
        selected_border: Rgb(0x3a, 0x44, 0x56),
        text: Rgb(0xe6, 0xe8, 0xec),
        body: Rgb(0xd5, 0xd9, 0xe0),
        muted: Rgb(0xa3, 0xaa, 0xb5),
        faint: Rgb(0x8a, 0x91, 0x9c),
        accent: Rgb(0xf0, 0xb4, 0x5a),
        success: Rgb(0x7f, 0xd1, 0xa0),
        warning: Rgb(0xf0, 0xb4, 0x5a),
        danger: Rgb(0xf0, 0x88, 0x88),
    },
};

pub const LIGHT: Theme = Theme {
    id: "light",
    name: "Wren Light",
    appearance: Appearance::Light,
    colors: Colors {
        editor: Rgb(0xf7, 0xf8, 0xfa),
        titlebar: Rgb(0xed, 0xef, 0xf3),
        pane: Rgb(0xf0, 0xf2, 0xf5),
        notes: Rgb(0xf3, 0xf4, 0xf7),
        input: Rgb(0xff, 0xff, 0xff),
        control: Rgb(0xf7, 0xf8, 0xfa),
        raised: Rgb(0xff, 0xff, 0xff),
        selected: Rgb(0xe1, 0xe7, 0xf0),
        current_line: Rgb(0xed, 0xf0, 0xf5),
        divider: Rgb(0xd5, 0xd9, 0xe0),
        control_border: Rgb(0xc4, 0xca, 0xd4),
        dialog_border: Rgb(0xb5, 0xbd, 0xc9),
        selected_border: Rgb(0xa0, 0xae, 0xc1),
        text: Rgb(0x20, 0x24, 0x2d),
        body: Rgb(0x31, 0x38, 0x44),
        muted: Rgb(0x58, 0x61, 0x70),
        faint: Rgb(0x60, 0x6a, 0x78),
        accent: Rgb(0x9a, 0x58, 0x00),
        success: Rgb(0x19, 0x73, 0x42),
        warning: Rgb(0x9a, 0x58, 0x00),
        danger: Rgb(0xb3, 0x2d, 0x36),
    },
};

pub const BUILT_INS: &[Theme] = &[DARK, LIGHT];

pub fn builtin(id: &str) -> Option<Theme> {
    BUILT_INS.iter().find(|theme| theme.id == id).copied()
}

pub fn for_appearance(appearance: Appearance) -> Theme {
    match appearance {
        Appearance::Dark => DARK,
        Appearance::Light => LIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Rgb) -> f64 {
        let linear = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.0) + 0.7152 * linear(color.1) + 0.0722 * linear(color.2)
    }

    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn builtin_ids_are_unique_and_resolvable() {
        for (index, theme) in BUILT_INS.iter().enumerate() {
            assert_eq!(builtin(theme.id), Some(*theme));
            assert!(!theme.name.is_empty());
            assert!(!BUILT_INS[..index].iter().any(|other| other.id == theme.id));
        }
        assert_eq!(builtin("missing"), None);
    }

    #[test]
    fn body_and_secondary_text_have_readable_contrast() {
        for theme in BUILT_INS {
            for background in [theme.colors.editor, theme.colors.pane, theme.colors.raised] {
                for foreground in [
                    theme.colors.text,
                    theme.colors.body,
                    theme.colors.muted,
                    theme.colors.faint,
                ] {
                    assert!(
                        contrast(foreground, background) >= 4.5,
                        "{}: {foreground:?} on {background:?}",
                        theme.name
                    );
                }
            }
        }
    }

    #[test]
    fn semantic_status_colors_remain_distinguishable() {
        for theme in BUILT_INS {
            for color in [
                theme.colors.accent,
                theme.colors.success,
                theme.colors.warning,
                theme.colors.danger,
            ] {
                assert!(contrast(color, theme.colors.editor) >= 4.5);
            }
        }
    }

    #[test]
    fn appearance_defaults_resolve_to_matching_palettes() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            assert_eq!(for_appearance(appearance).appearance, appearance);
        }
        assert!(luminance(DARK.colors.editor) < luminance(LIGHT.colors.editor));
    }
}
