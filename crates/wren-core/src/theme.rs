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

pub const WREN_DARK: Theme = Theme {
    id: "wren-dark",
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

pub const WREN_LIGHT: Theme = Theme {
    id: "wren-light",
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

pub const NORD: Theme = Theme {
    id: "nord",
    name: "Nord",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x2e, 0x34, 0x40),
        titlebar: Rgb(0x29, 0x2e, 0x39),
        pane: Rgb(0x32, 0x38, 0x46),
        notes: Rgb(0x30, 0x35, 0x3f),
        input: Rgb(0x2a, 0x2f, 0x3a),
        control: Rgb(0x34, 0x3b, 0x49),
        raised: Rgb(0x3b, 0x42, 0x52),
        selected: Rgb(0x43, 0x4c, 0x5e),
        current_line: Rgb(0x35, 0x3c, 0x4a),
        divider: Rgb(0x3b, 0x42, 0x52),
        control_border: Rgb(0x4c, 0x56, 0x6a),
        dialog_border: Rgb(0x4c, 0x56, 0x6a),
        selected_border: Rgb(0x5e, 0x81, 0xac),
        text: Rgb(0xec, 0xef, 0xf4),
        body: Rgb(0xe5, 0xe9, 0xf0),
        muted: Rgb(0xc0, 0xc8, 0xd6),
        faint: Rgb(0xa8, 0xb2, 0xc3),
        accent: Rgb(0x88, 0xc0, 0xd0),
        success: Rgb(0xa3, 0xbe, 0x8c),
        warning: Rgb(0xeb, 0xcb, 0x8b),
        danger: Rgb(0xec, 0x8f, 0x8f),
    },
};

pub const GRUVBOX_DARK: Theme = Theme {
    id: "gruvbox-dark",
    name: "Gruvbox Dark",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x28, 0x28, 0x28),
        titlebar: Rgb(0x1d, 0x20, 0x21),
        pane: Rgb(0x2e, 0x2c, 0x2a),
        notes: Rgb(0x2b, 0x2a, 0x28),
        input: Rgb(0x23, 0x23, 0x23),
        control: Rgb(0x32, 0x30, 0x2f),
        raised: Rgb(0x3c, 0x38, 0x36),
        selected: Rgb(0x50, 0x49, 0x45),
        current_line: Rgb(0x32, 0x30, 0x2f),
        divider: Rgb(0x3c, 0x38, 0x36),
        control_border: Rgb(0x50, 0x49, 0x45),
        dialog_border: Rgb(0x66, 0x5c, 0x54),
        selected_border: Rgb(0x7c, 0x6f, 0x64),
        text: Rgb(0xfb, 0xf1, 0xc7),
        body: Rgb(0xeb, 0xdb, 0xb2),
        muted: Rgb(0xd5, 0xc4, 0xa1),
        faint: Rgb(0xbd, 0xae, 0x93),
        accent: Rgb(0xfa, 0xbd, 0x2f),
        success: Rgb(0xb8, 0xbb, 0x26),
        warning: Rgb(0xfe, 0x80, 0x19),
        danger: Rgb(0xfb, 0x6b, 0x5b),
    },
};

pub const SOLARIZED_LIGHT: Theme = Theme {
    id: "solarized-light",
    name: "Solarized Light",
    appearance: Appearance::Light,
    colors: Colors {
        editor: Rgb(0xfd, 0xf6, 0xe3),
        titlebar: Rgb(0xf2, 0xea, 0xd4),
        pane: Rgb(0xf7, 0xf0, 0xdc),
        notes: Rgb(0xfa, 0xf3, 0xe0),
        input: Rgb(0xff, 0xfb, 0xf0),
        control: Rgb(0xf7, 0xf0, 0xdc),
        raised: Rgb(0xff, 0xfb, 0xf0),
        selected: Rgb(0xe9, 0xe1, 0xc8),
        current_line: Rgb(0xf4, 0xec, 0xd6),
        divider: Rgb(0xe2, 0xda, 0xc2),
        control_border: Rgb(0xd3, 0xcb, 0xb3),
        dialog_border: Rgb(0xc6, 0xbe, 0xa6),
        selected_border: Rgb(0xb9, 0xb0, 0x98),
        text: Rgb(0x07, 0x36, 0x42),
        body: Rgb(0x27, 0x4a, 0x52),
        muted: Rgb(0x3e, 0x5a, 0x62),
        faint: Rgb(0x4f, 0x6a, 0x72),
        accent: Rgb(0x9d, 0x5a, 0x00),
        success: Rgb(0x3d, 0x6b, 0x00),
        warning: Rgb(0x9a, 0x4f, 0x00),
        danger: Rgb(0xc0, 0x26, 0x2b),
    },
};

pub const ROSE_PINE: Theme = Theme {
    id: "rose-pine",
    name: "Rosé Pine",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x19, 0x17, 0x24),
        titlebar: Rgb(0x15, 0x13, 0x1f),
        pane: Rgb(0x1f, 0x1d, 0x2e),
        notes: Rgb(0x1c, 0x1a, 0x29),
        input: Rgb(0x16, 0x14, 0x1f),
        control: Rgb(0x22, 0x20, 0x2f),
        raised: Rgb(0x26, 0x23, 0x3a),
        selected: Rgb(0x39, 0x35, 0x52),
        current_line: Rgb(0x21, 0x1f, 0x2f),
        divider: Rgb(0x2a, 0x27, 0x3d),
        control_border: Rgb(0x40, 0x3d, 0x52),
        dialog_border: Rgb(0x44, 0x41, 0x5a),
        selected_border: Rgb(0x6e, 0x6a, 0x86),
        text: Rgb(0xe0, 0xde, 0xf4),
        body: Rgb(0xd8, 0xd5, 0xef),
        muted: Rgb(0xb6, 0xb2, 0xcf),
        faint: Rgb(0x98, 0x93, 0xb2),
        accent: Rgb(0xeb, 0xbc, 0xba),
        success: Rgb(0x9c, 0xcf, 0xd8),
        warning: Rgb(0xf6, 0xc1, 0x77),
        danger: Rgb(0xeb, 0x6f, 0x92),
    },
};

pub const EVERFOREST_DARK: Theme = Theme {
    id: "everforest-dark",
    name: "Everforest Dark",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x2d, 0x35, 0x3b),
        titlebar: Rgb(0x27, 0x2e, 0x33),
        pane: Rgb(0x33, 0x3c, 0x43),
        notes: Rgb(0x30, 0x38, 0x3e),
        input: Rgb(0x29, 0x31, 0x36),
        control: Rgb(0x37, 0x41, 0x45),
        raised: Rgb(0x3a, 0x44, 0x49),
        selected: Rgb(0x47, 0x52, 0x58),
        current_line: Rgb(0x34, 0x3e, 0x44),
        divider: Rgb(0x3d, 0x48, 0x4d),
        control_border: Rgb(0x4f, 0x5b, 0x58),
        dialog_border: Rgb(0x56, 0x63, 0x5f),
        selected_border: Rgb(0x7a, 0x84, 0x78),
        text: Rgb(0xd3, 0xc6, 0xaa),
        body: Rgb(0xd3, 0xc6, 0xaa),
        muted: Rgb(0xc4, 0xbb, 0xa2),
        faint: Rgb(0xb8, 0xb2, 0xa0),
        accent: Rgb(0xdb, 0xbc, 0x7f),
        success: Rgb(0xa7, 0xc0, 0x80),
        warning: Rgb(0xe6, 0x98, 0x75),
        danger: Rgb(0xf0, 0x8c, 0x8a),
    },
};

/// The default dark and light themes, kept as short names for callers.
pub const DARK: Theme = WREN_DARK;
pub const LIGHT: Theme = WREN_LIGHT;

pub const BUILT_INS: &[Theme] = &[
    WREN_DARK,
    WREN_LIGHT,
    NORD,
    GRUVBOX_DARK,
    SOLARIZED_LIGHT,
    ROSE_PINE,
    EVERFOREST_DARK,
];

/// Look up a built-in theme. `dark` and `light` remain aliases for the Wren
/// defaults.
pub fn builtin(id: &str) -> Option<Theme> {
    let id = match id {
        "dark" => WREN_DARK.id,
        "light" => WREN_LIGHT.id,
        other => other,
    };
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
        assert_eq!(builtin("dark"), Some(DARK));
        assert_eq!(builtin("light"), Some(LIGHT));
        assert!(BUILT_INS.len() >= 5);
    }

    #[test]
    fn body_and_secondary_text_have_readable_contrast() {
        for theme in BUILT_INS {
            for background in [
                theme.colors.editor,
                theme.colors.pane,
                theme.colors.notes,
                theme.colors.raised,
                theme.colors.current_line,
            ] {
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
