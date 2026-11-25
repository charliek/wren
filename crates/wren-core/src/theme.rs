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
    /// H1 text in the editor.
    pub heading: Rgb,
    /// H2 text in the editor; H3 and deeper use `text`.
    pub heading2: Rgb,
    /// Link text in the editor; the URL itself stays `faint`.
    pub link: Rgb,
    /// Inline code spans. Fenced block text stays `muted` on `code_block`.
    pub code: Rgb,
    /// The band behind fenced code blocks.
    pub code_block: Rgb,
    /// Block-quote text.
    pub quote: Rgb,
    /// The text of completed tasks and struck-through words: readable, but
    /// quieter than `body`. Their markers and strike lines stay `faint`.
    pub completed: Rgb,
    /// A task's due date in the editor; an overdue one takes `danger`.
    pub date: Rgb,
    /// A task's `@owner` in the editor.
    pub owner: Rgb,
    /// Selected rows in columns without keyboard focus.
    pub selected_inactive: Rgb,
    /// The editor's text-selection fill, kept apart from list rows so faint
    /// markers stay legible inside a selection.
    pub text_selection: Rgb,
    /// The board's colour tokens, `red` to `blue`: a shape's outline and
    /// solid fill, and a connector with its label. Each theme takes them
    /// from its own palette where it has one, each at its own lightness, so
    /// that every pair stays apart under colour blindness (the tests below).
    pub red: Rgb,
    pub orange: Rgb,
    pub yellow: Rgb,
    pub green: Rgb,
    pub cyan: Rgb,
    pub purple: Rgb,
    pub blue: Rgb,
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
        pane: Rgb(0x1a, 0x1d, 0x23),
        notes: Rgb(0x18, 0x1a, 0x20),
        input: Rgb(0x12, 0x14, 0x19),
        control: Rgb(0x18, 0x1b, 0x21),
        raised: Rgb(0x1c, 0x1f, 0x26),
        selected: Rgb(0x2a, 0x31, 0x40),
        current_line: Rgb(0x1c, 0x20, 0x28),
        divider: Rgb(0x22, 0x26, 0x2e),
        control_border: Rgb(0x2a, 0x2f, 0x38),
        dialog_border: Rgb(0x33, 0x39, 0x44),
        selected_border: Rgb(0x3a, 0x44, 0x56),
        text: Rgb(0xe6, 0xe8, 0xec),
        body: Rgb(0xd5, 0xd9, 0xe0),
        muted: Rgb(0xa3, 0xaa, 0xb5),
        faint: Rgb(0x8a, 0x91, 0x9c),
        accent: Rgb(0xf0, 0xb4, 0x5a),
        success: Rgb(0x93, 0xd4, 0x73),
        warning: Rgb(0xef, 0x8a, 0x4c),
        danger: Rgb(0xf0, 0x88, 0x88),
        heading: Rgb(0xf0, 0xb4, 0x5a),
        heading2: Rgb(0xdc, 0xbf, 0x91),
        link: Rgb(0x7f, 0xb0, 0xe6),
        code: Rgb(0x7f, 0xd1, 0xa0),
        code_block: Rgb(0x18, 0x1b, 0x21),
        quote: Rgb(0xa3, 0xaa, 0xb5),
        completed: Rgb(0xa3, 0xaa, 0xb5),
        date: Rgb(0x72, 0xd6, 0xe3),
        owner: Rgb(0x7e, 0xb1, 0xe7),
        selected_inactive: Rgb(0x20, 0x24, 0x2c),
        text_selection: Rgb(0x28, 0x30, 0x42),
        red: Rgb(0xfd, 0x94, 0x94),
        orange: Rgb(0xe4, 0x7d, 0x3d),
        yellow: Rgb(0xf3, 0xdf, 0x8c),
        green: Rgb(0x8f, 0xcf, 0x70),
        cyan: Rgb(0x72, 0xd6, 0xe3),
        purple: Rgb(0xa9, 0x77, 0xde),
        blue: Rgb(0x7e, 0xb1, 0xe7),
    },
};

pub const WREN_LIGHT: Theme = Theme {
    id: "wren-light",
    name: "Wren Light",
    appearance: Appearance::Light,
    colors: Colors {
        editor: Rgb(0xf7, 0xf8, 0xfa),
        titlebar: Rgb(0xdf, 0xe3, 0xea),
        pane: Rgb(0xec, 0xef, 0xf3),
        notes: Rgb(0xf1, 0xf3, 0xf6),
        input: Rgb(0xff, 0xff, 0xff),
        control: Rgb(0xf7, 0xf8, 0xfa),
        raised: Rgb(0xff, 0xff, 0xff),
        selected: Rgb(0xd2, 0xdd, 0xec),
        current_line: Rgb(0xeb, 0xee, 0xf4),
        divider: Rgb(0xd5, 0xd9, 0xe0),
        control_border: Rgb(0xc4, 0xca, 0xd4),
        dialog_border: Rgb(0xb5, 0xbd, 0xc9),
        selected_border: Rgb(0xa0, 0xae, 0xc1),
        text: Rgb(0x20, 0x24, 0x2d),
        body: Rgb(0x31, 0x38, 0x44),
        muted: Rgb(0x58, 0x61, 0x70),
        faint: Rgb(0x60, 0x6a, 0x78),
        accent: Rgb(0xa1, 0x48, 0x08),
        success: Rgb(0x3b, 0x7a, 0x12),
        warning: Rgb(0x7a, 0x6a, 0x00),
        danger: Rgb(0xb3, 0x2d, 0x36),
        heading: Rgb(0xa1, 0x48, 0x08),
        heading2: Rgb(0x7a, 0x4a, 0x1e),
        link: Rgb(0x1f, 0x5f, 0xbf),
        code: Rgb(0x19, 0x73, 0x42),
        code_block: Rgb(0xef, 0xf1, 0xf5),
        quote: Rgb(0x58, 0x61, 0x70),
        completed: Rgb(0x58, 0x61, 0x70),
        date: Rgb(0x04, 0x77, 0x84),
        owner: Rgb(0x04, 0x4e, 0xa4),
        selected_inactive: Rgb(0xe2, 0xe7, 0xed),
        text_selection: Rgb(0xd3, 0xde, 0xf0),
        red: Rgb(0xbd, 0x36, 0x3d),
        orange: Rgb(0x90, 0x3c, 0x01),
        yellow: Rgb(0x80, 0x70, 0x02),
        green: Rgb(0x3e, 0x68, 0x27),
        cyan: Rgb(0x04, 0x7b, 0x89),
        purple: Rgb(0x7f, 0x3d, 0xc7),
        blue: Rgb(0x04, 0x4e, 0xa4),
    },
};

pub const NORD: Theme = Theme {
    id: "nord",
    name: "Nord",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x2e, 0x34, 0x40),
        titlebar: Rgb(0x29, 0x2e, 0x39),
        pane: Rgb(0x36, 0x3d, 0x4b),
        notes: Rgb(0x30, 0x35, 0x3f),
        input: Rgb(0x2a, 0x2f, 0x3a),
        control: Rgb(0x34, 0x3b, 0x49),
        raised: Rgb(0x3b, 0x42, 0x52),
        selected: Rgb(0x43, 0x4c, 0x5e),
        current_line: Rgb(0x35, 0x3c, 0x4a),
        divider: Rgb(0x3b, 0x42, 0x52),
        control_border: Rgb(0x4c, 0x56, 0x6a),
        dialog_border: Rgb(0x4c, 0x56, 0x6a),
        selected_border: Rgb(0x4c, 0x56, 0x6a),
        text: Rgb(0xec, 0xef, 0xf4),
        body: Rgb(0xe5, 0xe9, 0xf0),
        muted: Rgb(0xc0, 0xc8, 0xd6),
        faint: Rgb(0x8b, 0x98, 0xb1),
        accent: Rgb(0x88, 0xc0, 0xd0),
        success: Rgb(0xa3, 0xbe, 0x8c),
        warning: Rgb(0xeb, 0xcb, 0x8b),
        danger: Rgb(0xec, 0x8f, 0x8f),
        heading: Rgb(0x88, 0xc0, 0xd0),
        heading2: Rgb(0xd0, 0xb3, 0x77),
        link: Rgb(0x8a, 0xab, 0xcb),
        code: Rgb(0xd4, 0x91, 0x7b),
        code_block: Rgb(0x2a, 0x2f, 0x3a),
        quote: Rgb(0xc0, 0xc8, 0xd6),
        completed: Rgb(0xa8, 0xb2, 0xc3),
        date: Rgb(0x92, 0xd3, 0xe3),
        owner: Rgb(0x83, 0xb4, 0xe0),
        selected_inactive: Rgb(0x3d, 0x44, 0x56),
        text_selection: Rgb(0x40, 0x48, 0x60),
        red: Rgb(0xed, 0x7b, 0x8e),
        orange: Rgb(0xd7, 0x8d, 0x6d),
        yellow: Rgb(0xec, 0xce, 0x89),
        green: Rgb(0xae, 0xc8, 0x9b),
        cyan: Rgb(0x92, 0xd3, 0xe3),
        purple: Rgb(0xbe, 0x93, 0xba),
        blue: Rgb(0x83, 0xb4, 0xe0),
    },
};

pub const GRUVBOX_DARK: Theme = Theme {
    id: "gruvbox-dark",
    name: "Gruvbox Dark",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x28, 0x28, 0x28),
        titlebar: Rgb(0x1d, 0x20, 0x21),
        pane: Rgb(0x32, 0x30, 0x2f),
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
        faint: Rgb(0xa8, 0x99, 0x84),
        accent: Rgb(0xfa, 0xbd, 0x2f),
        success: Rgb(0xb8, 0xbb, 0x26),
        warning: Rgb(0xfe, 0x80, 0x19),
        danger: Rgb(0xfb, 0x6b, 0x5b),
        heading: Rgb(0xfa, 0xbd, 0x2f),
        heading2: Rgb(0xfe, 0x80, 0x19),
        link: Rgb(0x83, 0xa5, 0x98),
        code: Rgb(0x8e, 0xc0, 0x7c),
        code_block: Rgb(0x1d, 0x20, 0x21),
        quote: Rgb(0xbd, 0xae, 0x93),
        completed: Rgb(0xbd, 0xae, 0x93),
        date: Rgb(0x4e, 0xb1, 0xa0),
        owner: Rgb(0x72, 0x9d, 0xbb),
        selected_inactive: Rgb(0x3c, 0x38, 0x36),
        text_selection: Rgb(0x45, 0x40, 0x3a),
        red: Rgb(0xff, 0x56, 0x3d),
        orange: Rgb(0xff, 0x82, 0x1b),
        yellow: Rgb(0xf8, 0xbb, 0x2d),
        green: Rgb(0x9f, 0xcc, 0x5e),
        cyan: Rgb(0x4e, 0xb1, 0xa0),
        purple: Rgb(0xd1, 0xa6, 0xcd),
        blue: Rgb(0x6d, 0x99, 0xb9),
    },
};

pub const SOLARIZED_LIGHT: Theme = Theme {
    id: "solarized-light",
    name: "Solarized Light",
    appearance: Appearance::Light,
    colors: Colors {
        editor: Rgb(0xfd, 0xf6, 0xe3),
        titlebar: Rgb(0xf2, 0xea, 0xd4),
        pane: Rgb(0xf4, 0xec, 0xd5),
        notes: Rgb(0xfa, 0xf3, 0xe0),
        input: Rgb(0xff, 0xfb, 0xf0),
        control: Rgb(0xf7, 0xf0, 0xdc),
        raised: Rgb(0xff, 0xfb, 0xf0),
        selected: Rgb(0xe2, 0xd8, 0xba),
        current_line: Rgb(0xf4, 0xec, 0xd6),
        divider: Rgb(0xe2, 0xda, 0xc2),
        control_border: Rgb(0xd3, 0xcb, 0xb3),
        dialog_border: Rgb(0xc6, 0xbe, 0xa6),
        selected_border: Rgb(0xb9, 0xb0, 0x98),
        text: Rgb(0x07, 0x36, 0x42),
        body: Rgb(0x27, 0x4a, 0x52),
        muted: Rgb(0x4a, 0x63, 0x6b),
        faint: Rgb(0x6c, 0x7e, 0x82),
        accent: Rgb(0xb5, 0x3f, 0x0e),
        success: Rgb(0x3d, 0x6b, 0x00),
        warning: Rgb(0x8a, 0x68, 0x00),
        danger: Rgb(0xc0, 0x26, 0x2b),
        heading: Rgb(0xb5, 0x3f, 0x0e),
        heading2: Rgb(0x5b, 0x5f, 0xb0),
        link: Rgb(0x1f, 0x6a, 0xa6),
        code: Rgb(0x16, 0x75, 0x6d),
        code_block: Rgb(0xf2, 0xe9, 0xce),
        quote: Rgb(0x58, 0x6e, 0x75),
        completed: Rgb(0x58, 0x6e, 0x75),
        date: Rgb(0x02, 0x69, 0x60),
        owner: Rgb(0x21, 0x6f, 0xab),
        selected_inactive: Rgb(0xeb, 0xe3, 0xca),
        text_selection: Rgb(0xe9, 0xe0, 0xc0),
        red: Rgb(0x85, 0x00, 0x04),
        orange: Rgb(0x94, 0x39, 0x00),
        yellow: Rgb(0x90, 0x68, 0x0b),
        green: Rgb(0x5b, 0x74, 0x37),
        cyan: Rgb(0x02, 0x69, 0x60),
        purple: Rgb(0x5b, 0x49, 0x93),
        blue: Rgb(0x22, 0x73, 0xb1),
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
        faint: Rgb(0x7c, 0x78, 0x96),
        accent: Rgb(0xeb, 0xbc, 0xba),
        success: Rgb(0x5e, 0xa8, 0xbf),
        warning: Rgb(0xf6, 0xc1, 0x77),
        danger: Rgb(0xeb, 0x6f, 0x92),
        heading: Rgb(0xeb, 0xbc, 0xba),
        heading2: Rgb(0xf6, 0xc1, 0x77),
        link: Rgb(0xc4, 0xa7, 0xe7),
        code: Rgb(0x9c, 0xcf, 0xd8),
        code_block: Rgb(0x21, 0x1f, 0x30),
        quote: Rgb(0xb6, 0xb2, 0xcf),
        completed: Rgb(0x98, 0x93, 0xb2),
        date: Rgb(0x88, 0xcc, 0xd5),
        owner: Rgb(0x5c, 0x8d, 0xaf),
        selected_inactive: Rgb(0x26, 0x23, 0x3a),
        text_selection: Rgb(0x2f, 0x2b, 0x45),
        red: Rgb(0xec, 0x72, 0x93),
        orange: Rgb(0xe0, 0xa0, 0x66),
        yellow: Rgb(0xe8, 0xc3, 0x6f),
        green: Rgb(0x6c, 0xef, 0x91),
        cyan: Rgb(0x88, 0xcc, 0xd5),
        purple: Rgb(0xc1, 0xa5, 0xe8),
        blue: Rgb(0x57, 0x89, 0xac),
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
        selected: Rgb(0x42, 0x4d, 0x52),
        current_line: Rgb(0x34, 0x3e, 0x44),
        divider: Rgb(0x3d, 0x48, 0x4d),
        control_border: Rgb(0x4f, 0x5b, 0x58),
        dialog_border: Rgb(0x56, 0x63, 0x5f),
        selected_border: Rgb(0x7a, 0x84, 0x78),
        text: Rgb(0xe0, 0xd5, 0xb9),
        body: Rgb(0xd3, 0xc6, 0xaa),
        muted: Rgb(0xa9, 0xb4, 0xab),
        faint: Rgb(0x97, 0xa3, 0x9a),
        accent: Rgb(0xa7, 0xc0, 0x80),
        success: Rgb(0x83, 0xc0, 0x92),
        warning: Rgb(0xdb, 0xbc, 0x7f),
        danger: Rgb(0xe6, 0x7e, 0x80),
        heading: Rgb(0xe6, 0x98, 0x75),
        heading2: Rgb(0xa7, 0xc0, 0x80),
        link: Rgb(0x7f, 0xbb, 0xb3),
        code: Rgb(0xd6, 0x99, 0xb6),
        code_block: Rgb(0x28, 0x30, 0x35),
        quote: Rgb(0xa9, 0xb4, 0xab),
        completed: Rgb(0xa9, 0xb4, 0xab),
        date: Rgb(0x8e, 0xd2, 0xcd),
        owner: Rgb(0x82, 0xa9, 0xe6),
        selected_inactive: Rgb(0x3a, 0x44, 0x49),
        text_selection: Rgb(0x40, 0x50, 0x4a),
        red: Rgb(0xe2, 0x83, 0x89),
        orange: Rgb(0xf1, 0xa6, 0x86),
        yellow: Rgb(0xef, 0xcd, 0x7d),
        green: Rgb(0x8f, 0xae, 0x63),
        cyan: Rgb(0x8e, 0xd2, 0xcd),
        purple: Rgb(0xc4, 0x9b, 0xc0),
        blue: Rgb(0x82, 0xa9, 0xe6),
    },
};

pub const TOKYO_NIGHT: Theme = Theme {
    id: "tokyo-night",
    name: "Tokyo Night",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x1a, 0x1b, 0x26),
        titlebar: Rgb(0x15, 0x16, 0x1e),
        pane: Rgb(0x20, 0x22, 0x2f),
        notes: Rgb(0x1c, 0x1d, 0x29),
        input: Rgb(0x16, 0x16, 0x1e),
        control: Rgb(0x1f, 0x22, 0x30),
        raised: Rgb(0x29, 0x2e, 0x42),
        selected: Rgb(0x28, 0x34, 0x57),
        current_line: Rgb(0x1e, 0x21, 0x2e),
        divider: Rgb(0x29, 0x2e, 0x42),
        control_border: Rgb(0x3b, 0x42, 0x61),
        dialog_border: Rgb(0x3a, 0x47, 0x72),
        selected_border: Rgb(0x3d, 0x59, 0xa1),
        text: Rgb(0xc0, 0xca, 0xf5),
        body: Rgb(0xb4, 0xbc, 0xe6),
        muted: Rgb(0xa9, 0xb1, 0xd6),
        faint: Rgb(0x73, 0x7a, 0xa2),
        accent: Rgb(0x7a, 0xa2, 0xf7),
        success: Rgb(0x9e, 0xce, 0x6a),
        warning: Rgb(0xe0, 0xaf, 0x68),
        danger: Rgb(0xf7, 0x76, 0x8e),
        heading: Rgb(0x7a, 0xa2, 0xf7),
        heading2: Rgb(0xc7, 0x95, 0x78),
        link: Rgb(0x7d, 0xcf, 0xff),
        code: Rgb(0xbb, 0x9a, 0xf7),
        code_block: Rgb(0x16, 0x16, 0x1e),
        quote: Rgb(0x9a, 0xa5, 0xce),
        completed: Rgb(0x9a, 0xa5, 0xce),
        date: Rgb(0xa3, 0xf5, 0xff),
        owner: Rgb(0x69, 0x92, 0xe3),
        selected_inactive: Rgb(0x26, 0x2a, 0x3d),
        text_selection: Rgb(0x25, 0x2d, 0x49),
        red: Rgb(0xf4, 0x6d, 0x86),
        orange: Rgb(0xe7, 0x8f, 0x58),
        yellow: Rgb(0xdf, 0xbc, 0x6d),
        green: Rgb(0xc6, 0xf6, 0x8d),
        cyan: Rgb(0xa3, 0xf5, 0xff),
        purple: Rgb(0xc2, 0xa0, 0xfe),
        blue: Rgb(0x69, 0x92, 0xe3),
    },
};

pub const DRACULA: Theme = Theme {
    id: "dracula",
    name: "Dracula",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x28, 0x2a, 0x36),
        titlebar: Rgb(0x21, 0x22, 0x2c),
        pane: Rgb(0x2e, 0x30, 0x3e),
        notes: Rgb(0x29, 0x2b, 0x38),
        input: Rgb(0x21, 0x22, 0x2c),
        control: Rgb(0x2c, 0x2f, 0x3c),
        raised: Rgb(0x34, 0x37, 0x46),
        selected: Rgb(0x3d, 0x40, 0x52),
        current_line: Rgb(0x31, 0x33, 0x41),
        divider: Rgb(0x34, 0x37, 0x46),
        control_border: Rgb(0x44, 0x47, 0x5a),
        dialog_border: Rgb(0x4d, 0x50, 0x65),
        selected_border: Rgb(0x62, 0x72, 0xa4),
        text: Rgb(0xf8, 0xf8, 0xf2),
        body: Rgb(0xf0, 0xf0, 0xea),
        muted: Rgb(0xd6, 0xd7, 0xe0),
        faint: Rgb(0x7f, 0x8b, 0xbd),
        accent: Rgb(0xbd, 0x93, 0xf9),
        success: Rgb(0x8e, 0xcf, 0x9b),
        warning: Rgb(0xff, 0xb8, 0x6c),
        danger: Rgb(0xff, 0x6e, 0x6e),
        heading: Rgb(0xbd, 0x93, 0xf9),
        heading2: Rgb(0xff, 0x79, 0xc6),
        link: Rgb(0x8b, 0xe9, 0xfd),
        code: Rgb(0x50, 0xfa, 0x7b),
        code_block: Rgb(0x22, 0x23, 0x2e),
        quote: Rgb(0xc0, 0xc3, 0xab),
        completed: Rgb(0xb8, 0xbc, 0xd8),
        date: Rgb(0x8e, 0xe9, 0xfd),
        owner: Rgb(0x84, 0xb1, 0xff),
        selected_inactive: Rgb(0x34, 0x37, 0x46),
        text_selection: Rgb(0x3a, 0x3d, 0x4e),
        red: Rgb(0xfe, 0x60, 0x61),
        orange: Rgb(0xe7, 0x9b, 0x4c),
        yellow: Rgb(0xf0, 0xe5, 0x86),
        green: Rgb(0x17, 0xf2, 0x62),
        cyan: Rgb(0x8e, 0xe9, 0xfd),
        purple: Rgb(0xa8, 0x81, 0xe7),
        blue: Rgb(0x84, 0xb1, 0xff),
    },
};

pub const CATPPUCCIN_MOCHA: Theme = Theme {
    id: "catppuccin-mocha",
    name: "Catppuccin Mocha",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x1e, 0x1e, 0x2e),
        titlebar: Rgb(0x18, 0x18, 0x25),
        pane: Rgb(0x25, 0x25, 0x36),
        notes: Rgb(0x20, 0x20, 0x30),
        input: Rgb(0x18, 0x18, 0x25),
        control: Rgb(0x25, 0x25, 0x36),
        raised: Rgb(0x31, 0x32, 0x44),
        selected: Rgb(0x3a, 0x3c, 0x50),
        current_line: Rgb(0x24, 0x24, 0x35),
        divider: Rgb(0x31, 0x32, 0x44),
        control_border: Rgb(0x45, 0x47, 0x5a),
        dialog_border: Rgb(0x4e, 0x51, 0x65),
        selected_border: Rgb(0x58, 0x5b, 0x70),
        text: Rgb(0xcd, 0xd6, 0xf4),
        body: Rgb(0xc4, 0xcc, 0xea),
        muted: Rgb(0xba, 0xc2, 0xde),
        faint: Rgb(0x7f, 0x84, 0x9c),
        accent: Rgb(0xcb, 0xa6, 0xf7),
        success: Rgb(0x94, 0xe2, 0xd5),
        warning: Rgb(0xf9, 0xe2, 0xaf),
        danger: Rgb(0xf3, 0x8b, 0xa8),
        heading: Rgb(0xcb, 0xa6, 0xf7),
        heading2: Rgb(0xe6, 0xa4, 0x7c),
        link: Rgb(0x89, 0xb4, 0xfa),
        code: Rgb(0xa6, 0xe3, 0xa1),
        code_block: Rgb(0x18, 0x18, 0x25),
        quote: Rgb(0xa6, 0xad, 0xc8),
        completed: Rgb(0xa6, 0xad, 0xc8),
        date: Rgb(0x79, 0xc5, 0xd2),
        owner: Rgb(0x72, 0xa0, 0xe7),
        selected_inactive: Rgb(0x2e, 0x2f, 0x41),
        text_selection: Rgb(0x31, 0x32, 0x4c),
        red: Rgb(0xe9, 0x7d, 0x9d),
        orange: Rgb(0xde, 0x9c, 0x70),
        yellow: Rgb(0xff, 0xe8, 0xb4),
        green: Rgb(0x9e, 0xd9, 0x98),
        cyan: Rgb(0x79, 0xc5, 0xd2),
        purple: Rgb(0xd3, 0xae, 0xff),
        blue: Rgb(0x72, 0xa0, 0xe7),
    },
};

pub const CATPPUCCIN_MACCHIATO: Theme = Theme {
    id: "catppuccin-macchiato",
    name: "Catppuccin Macchiato",
    appearance: Appearance::Dark,
    colors: Colors {
        editor: Rgb(0x24, 0x27, 0x3a),
        titlebar: Rgb(0x1e, 0x20, 0x30),
        pane: Rgb(0x2b, 0x2e, 0x43),
        notes: Rgb(0x26, 0x29, 0x3c),
        input: Rgb(0x1e, 0x20, 0x30),
        control: Rgb(0x2a, 0x2e, 0x41),
        raised: Rgb(0x36, 0x3a, 0x4f),
        selected: Rgb(0x3d, 0x41, 0x57),
        current_line: Rgb(0x29, 0x2d, 0x40),
        divider: Rgb(0x36, 0x3a, 0x4f),
        control_border: Rgb(0x49, 0x4d, 0x64),
        dialog_border: Rgb(0x52, 0x56, 0x6e),
        selected_border: Rgb(0x5b, 0x60, 0x78),
        text: Rgb(0xca, 0xd3, 0xf5),
        body: Rgb(0xc1, 0xc9, 0xeb),
        muted: Rgb(0xb8, 0xc0, 0xe0),
        faint: Rgb(0x80, 0x87, 0xa2),
        accent: Rgb(0xc6, 0xa0, 0xf6),
        success: Rgb(0x8b, 0xd5, 0xca),
        warning: Rgb(0xee, 0xd4, 0x9f),
        danger: Rgb(0xed, 0x87, 0x96),
        heading: Rgb(0xc6, 0xa0, 0xf6),
        heading2: Rgb(0xdd, 0x9a, 0x74),
        link: Rgb(0x8a, 0xad, 0xf4),
        code: Rgb(0xa6, 0xda, 0x95),
        code_block: Rgb(0x1e, 0x20, 0x30),
        quote: Rgb(0xa5, 0xad, 0xcb),
        completed: Rgb(0xa5, 0xad, 0xcb),
        date: Rgb(0x77, 0xc1, 0xce),
        owner: Rgb(0x78, 0x99, 0xdb),
        selected_inactive: Rgb(0x31, 0x35, 0x4a),
        text_selection: Rgb(0x36, 0x3a, 0x53),
        red: Rgb(0xee, 0x84, 0x94),
        orange: Rgb(0xda, 0x98, 0x71),
        yellow: Rgb(0xff, 0xe8, 0xb3),
        green: Rgb(0x9b, 0xd7, 0x8f),
        cyan: Rgb(0x77, 0xc1, 0xce),
        purple: Rgb(0xcc, 0xa6, 0xfe),
        blue: Rgb(0x78, 0x99, 0xdb),
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
    TOKYO_NIGHT,
    DRACULA,
    CATPPUCCIN_MOCHA,
    CATPPUCCIN_MACCHIATO,
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

/// The colours a board is drawn in under a theme with these colours: the
/// theme's seven board hues, and the editor's surface and text.
pub fn board_palette(colors: &Colors) -> wren_board::Palette {
    let rgba = |Rgb(r, g, b): Rgb| wren_board::Rgba::rgb(r, g, b);
    let c = colors;
    let hues = [c.red, c.orange, c.yellow, c.green, c.cyan, c.purple, c.blue].map(rgba);
    wren_board::Palette {
        canvas: rgba(colors.editor),
        ink: rgba(colors.text),
        muted: rgba(colors.muted),
        raised: rgba(colors.raised),
        accent: rgba(colors.accent),
        issue: rgba(colors.danger),
        hues,
        dark: luma(colors.editor) < luma(colors.text),
    }
}

/// A rough brightness, enough to tell a dark surface from its light text.
fn luma(Rgb(r, g, b): Rgb) -> u32 {
    2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An sRGB channel as linear light, 0 to 1.
    fn linear(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(color: Rgb) -> f64 {
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

    /// CIE L* (0–100), for checking that surfaces read as separate steps.
    fn lightness(color: Rgb) -> f64 {
        let y = luminance(color);
        if y > 216.0 / 24389.0 {
            116.0 * y.cbrt() - 16.0
        } else {
            y * 24389.0 / 27.0
        }
    }

    fn step(a: Rgb, b: Rgb) -> f64 {
        (lightness(a) - lightness(b)).abs()
    }

    #[test]
    fn reading_text_has_readable_contrast_on_every_surface() {
        for theme in BUILT_INS {
            let c = theme.colors;
            for background in [c.editor, c.pane, c.notes, c.raised, c.current_line] {
                for foreground in [c.text, c.body, c.muted] {
                    assert!(
                        contrast(foreground, background) >= 4.5,
                        "{}: {foreground:?} on {background:?}",
                        theme.name
                    );
                }
            }
            for foreground in [c.heading, c.heading2, c.link, c.code, c.quote, c.completed] {
                assert!(
                    contrast(foreground, c.editor) >= 4.5,
                    "{}: {foreground:?} on the editor",
                    theme.name
                );
            }
            // A quote and a done task are voices beside the body, never louder.
            let on_editor = |color| contrast(color, c.editor);
            assert!(
                on_editor(c.quote) <= on_editor(c.body),
                "{}: quote",
                theme.name
            );
            assert!(
                on_editor(c.completed) < on_editor(c.body),
                "{}: done",
                theme.name
            );
            // Search matches are drawn in the accent on the active palette row.
            assert!(
                contrast(c.accent, c.selected) >= 4.0,
                "{}: accent on selected",
                theme.name
            );
            // Selected text stays readable; syntax colors may soften a little.
            assert!(contrast(c.body, c.text_selection) >= 4.5, "{}", theme.name);
            for foreground in [c.heading, c.link, c.code] {
                assert!(
                    contrast(foreground, c.text_selection) >= 3.5,
                    "{}: {foreground:?} on the text selection",
                    theme.name
                );
            }
            assert!(
                contrast(c.faint, c.text_selection) >= 3.0,
                "{}: faint markers inside a text selection",
                theme.name
            );
            assert!(contrast(c.code, c.code_block) >= 4.5, "{}", theme.name);
            for background in [c.selected, c.selected_inactive] {
                for foreground in [c.text, c.muted] {
                    assert!(
                        contrast(foreground, background) >= 4.0,
                        "{}: {foreground:?} on selected {background:?}",
                        theme.name
                    );
                }
            }
        }
    }

    /// Faint text (line numbers, syntax markers, done tasks) must stay legible
    /// but recede: brighter than 3:1, and below muted text.
    #[test]
    fn faint_text_recedes_below_muted_but_stays_legible() {
        for theme in BUILT_INS {
            let c = theme.colors;
            for background in [c.editor, c.pane, c.notes, c.raised, c.current_line] {
                assert!(
                    contrast(c.faint, background) >= 3.0,
                    "{}: faint on {background:?}",
                    theme.name
                );
            }
            let on_editor = |color| contrast(color, c.editor);
            assert!(on_editor(c.faint) < on_editor(c.muted), "{}", theme.name);
            assert!(on_editor(c.muted) < on_editor(c.body), "{}", theme.name);
            assert!(on_editor(c.body) <= on_editor(c.text), "{}", theme.name);
        }
    }

    #[test]
    fn semantic_status_colors_are_readable_and_distinct() {
        let distance = |Rgb(r1, g1, b1): Rgb, Rgb(r2, g2, b2): Rgb| {
            let d = |a: u8, b: u8| (f64::from(a) - f64::from(b)).powi(2);
            (d(r1, r2) + d(g1, g2) + d(b1, b2)).sqrt()
        };
        for theme in BUILT_INS {
            let c = theme.colors;
            for color in [c.accent, c.success, c.warning, c.danger] {
                assert!(contrast(color, c.editor) >= 4.5, "{}", theme.name);
            }
            // A second heading level, an editor selection, and a list row
            // must each look like themselves.
            assert!(
                distance(c.text_selection, c.selected_inactive) >= 10.0,
                "{}: text selection vs inactive row",
                theme.name
            );
            assert!(
                distance(c.heading2, c.link) >= 40.0,
                "{}: H2 vs link",
                theme.name
            );
            assert!(
                distance(c.heading2, c.text) >= 40.0,
                "{}: H2 vs H3",
                theme.name
            );
            // Inline code is syntax, not status: it must not read as "Saved".
            for (a, b) in [
                (c.code, c.success),
                (c.accent, c.warning),
                (c.warning, c.danger),
                (c.success, c.warning),
                (c.success, c.danger),
            ] {
                assert!(distance(a, b) >= 40.0, "{}: {a:?} vs {b:?}", theme.name);
            }
        }
    }

    /// A task's due date and owner are read where the text is: on the
    /// editor, on the caret's line and inside a selection. They hold the
    /// reading-text bar on the first two and the bar the other syntax
    /// colours hold in a selection. An overdue date reuses `danger`, which
    /// the editor draws bold and which predates tasks; it must stay legible
    /// on the caret's line and in a selection, though in one theme it sits
    /// below the reading-text bar there.
    #[test]
    fn task_colours_are_readable_where_the_text_is_and_read_apart() {
        let distance = |Rgb(r1, g1, b1): Rgb, Rgb(r2, g2, b2): Rgb| {
            let d = |a: u8, b: u8| (f64::from(a) - f64::from(b)).powi(2);
            (d(r1, r2) + d(g1, g2) + d(b1, b2)).sqrt()
        };
        for theme in BUILT_INS {
            let c = theme.colors;
            let name = theme.name;
            for (role, color) in [("date", c.date), ("owner", c.owner)] {
                for (surface, background) in
                    [("editor", c.editor), ("current line", c.current_line)]
                {
                    assert!(
                        contrast(color, background) >= 4.5,
                        "{name}: {role} on the {surface}"
                    );
                }
                assert!(
                    contrast(color, c.text_selection) >= 3.5,
                    "{name}: {role} in a selection"
                );
                // A coloured token must not read as body text, nor a due
                // date as an overdue one.
                assert!(distance(color, c.body) >= 40.0, "{name}: {role} vs body");
                assert!(
                    distance(color, c.danger) >= 40.0,
                    "{name}: {role} vs overdue"
                );
            }
            assert!(
                distance(c.date, c.owner) >= 30.0,
                "{name}: a date vs an owner"
            );
            assert!(contrast(c.danger, c.editor) >= 4.5, "{name}: overdue");
            assert!(
                contrast(c.danger, c.current_line) >= 3.5,
                "{name}: overdue on the current line"
            );
            assert!(
                contrast(c.danger, c.text_selection) >= 3.0,
                "{name}: overdue in a selection"
            );
        }
    }

    #[test]
    fn surfaces_form_visible_but_calm_steps() {
        for theme in BUILT_INS {
            let c = theme.colors;
            let name = theme.name;
            assert!(step(c.pane, c.editor) >= 2.5, "{name}: pane vs editor");
            assert!(
                step(c.current_line, c.editor) >= 2.5,
                "{name}: current line"
            );
            assert!(step(c.code_block, c.editor) >= 2.0, "{name}: code block");
            assert!(
                step(c.text_selection, c.editor) >= 4.0,
                "{name}: text selection"
            );
            assert!(
                step(c.text_selection, c.current_line) >= 4.0,
                "{name}: a selection on the caret's line"
            );
            assert!(
                step(c.selected, c.selected_inactive) >= 3.0,
                "{name}: the focused selection must read apart from the inactive one"
            );
            assert!(
                step(c.selected_inactive, c.pane) >= 2.5,
                "{name}: inactive row"
            );
            assert!(
                step(c.selected, c.pane) > step(c.selected_inactive, c.pane),
                "{name}: the focused selection must be the stronger one"
            );
            assert!(step(c.pane, c.editor) <= 6.0, "{name}: panes too loud");
        }
    }

    #[test]
    fn appearance_defaults_resolve_to_matching_palettes() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            assert_eq!(for_appearance(appearance).appearance, appearance);
        }
        assert!(luminance(DARK.colors.editor) < luminance(LIGHT.colors.editor));
    }

    #[test]
    fn every_board_token_resolves_in_every_theme() {
        use wren_board::{style, Color, Fill, Token, Vocabulary};

        assert_eq!(BUILT_INS.len(), 11);
        for theme in BUILT_INS {
            let name = theme.name;
            let c = theme.colors;
            let palette = board_palette(&c);
            let rgba = |Rgb(r, g, b): Rgb| wren_board::Rgba::rgb(r, g, b);
            assert_eq!(palette.dark, theme.appearance == Appearance::Dark, "{name}");
            assert_eq!(palette.canvas, rgba(c.editor), "{name}");
            assert_eq!(palette.ink, rgba(c.text), "{name}");
            let hues = [c.red, c.orange, c.yellow, c.green, c.cyan, c.purple, c.blue];
            assert_eq!(palette.hues, hues.map(rgba), "{name}");
            assert_eq!(palette.issue, rgba(c.danger), "{name}");
            for (index, hue) in palette.hues.iter().enumerate() {
                assert!(!palette.hues[..index].contains(hue), "{name}: hue {index}");
                assert!(![palette.canvas, palette.ink].contains(hue), "{name}");
            }
            for token in Token::ALL {
                for fill in Fill::ALL {
                    let colors = style::resolve(&Color::Token(*token), *fill, &palette);
                    let what = format!("{name}: {} at {}", token.name(), fill.name());
                    assert_ne!(colors.stroke, palette.canvas, "{what}");
                    let expected = match token {
                        Token::Neutral => palette.ink,
                        hue => palette.hues[*hue as usize - 1],
                    };
                    assert_eq!(colors.stroke, expected, "{what}");
                    match (fill, colors.fill) {
                        (Fill::None, None) => assert_eq!(colors.label, palette.ink, "{what}"),
                        (Fill::None, Some(_)) | (_, None) => panic!("{what}: fill"),
                        (Fill::Tint, Some(fill)) => {
                            assert_ne!(fill, palette.canvas, "{what}");
                            assert_eq!(colors.label, palette.ink, "{what}");
                        }
                        (Fill::Solid, Some(fill)) => {
                            assert_ne!(colors.label, fill, "{what}");
                            assert!(
                                style::contrast(colors.label, fill) >= 3.0,
                                "{what}: label on fill"
                            );
                        }
                    }
                }
            }
        }
    }

    fn rgb(color: wren_board::Rgba) -> Rgb {
        Rgb(color.r, color.g, color.b)
    }

    /// Every colour token at every fill, in every theme: a label reads on
    /// what it is drawn on, outlines and connectors stand out from the
    /// canvas, a tint is a visible step from the canvas, and so is the
    /// selection. The selection outline is drawn 3 px outside the shape
    /// (`wren-canvas`'s overlay and its outline test), so no fill touches it.
    #[test]
    fn board_colours_are_readable_in_every_theme() {
        use wren_board::{style, Color, Fill, Token, Vocabulary};

        for theme in BUILT_INS {
            let name = theme.name;
            let palette = board_palette(&theme.colors);
            let canvas = rgb(palette.canvas);
            for token in Token::ALL {
                for fill in Fill::ALL {
                    let colors = style::resolve(&Color::Token(*token), *fill, &palette);
                    let what = format!("{name}: {} at {}", token.name(), fill.name());
                    let (label, stroke) = (rgb(colors.label), rgb(colors.stroke));
                    let under = colors.fill.map_or(canvas, rgb);
                    let on_fill = contrast(label, under);
                    assert!(on_fill >= 4.5, "{what}: label on its fill {on_fill:.2}");
                    let outline = contrast(stroke, canvas);
                    assert!(outline >= 3.0, "{what}: stroke {outline:.2}");
                    // A connector's label is drawn in its stroke colour, on
                    // the canvas.
                    assert!(outline >= 4.5, "{what}: connector label {outline:.2}");
                    if *fill == Fill::Tint {
                        let tint = step(under, canvas);
                        assert!(tint >= 4.0, "{what}: tint {tint:.2} L* from the canvas");
                    }
                }
            }
            let c = theme.colors;
            for (ring, colour) in [("focused", c.accent), ("unfocused", c.muted)] {
                let ratio = contrast(colour, canvas);
                assert!(ratio >= 3.0, "{name}: {ring} selection {ratio:.2}");
            }
        }
    }

    /// The style bar's swatches (discovery/board-controls.md §3.2) are each
    /// colour as the board draws an outline, on the bar's raised surface:
    /// every one stands out from it as a control must, at 3:1.
    #[test]
    fn board_swatches_stand_out_on_the_bar_in_every_theme() {
        use wren_board::{style, Color, Fill, Token, Vocabulary};

        for theme in BUILT_INS {
            let palette = board_palette(&theme.colors);
            let raised = theme.colors.raised;
            for token in Token::ALL {
                let swatch = style::resolve(&Color::Token(*token), Fill::None, &palette).stroke;
                let ratio = contrast(rgb(swatch), raised);
                assert!(
                    ratio >= 3.0,
                    "{}: {} swatch {ratio:.2}",
                    theme.name,
                    token.name()
                );
            }
        }
    }

    /// Two board hues must differ by at least this CIE76 ΔE…
    const APART_DELTA_E: f64 = 15.0;
    /// …or by at least this much lightness (L*), for every kind of vision.
    const APART_DELTA_L: f64 = 8.0;

    #[derive(Clone, Copy, Debug)]
    enum Vision {
        Normal,
        Deuteranopia,
        Protanopia,
        Tritanopia,
    }

    /// The colour `vision` sees in place of `color`, as linear light: the
    /// severity-1.0 matrices of Machado, Oliveira and Fernandes (2009),
    /// clamped to the gamut.
    fn simulated(color: Rgb, vision: Vision) -> [f64; 3] {
        let light = [linear(color.0), linear(color.1), linear(color.2)];
        let matrix = match vision {
            Vision::Normal => return light,
            Vision::Deuteranopia => [
                [0.367322, 0.860646, -0.227968],
                [0.280085, 0.672501, 0.047413],
                [-0.011820, 0.042940, 0.968881],
            ],
            Vision::Protanopia => [
                [0.152286, 1.052583, -0.204868],
                [0.114503, 0.786281, 0.099216],
                [-0.003882, -0.048116, 1.051998],
            ],
            Vision::Tritanopia => [
                [1.255528, -0.076749, -0.178779],
                [-0.078411, 0.930809, 0.147602],
                [0.004733, 0.691367, 0.303900],
            ],
        };
        matrix.map(|row| {
            let sum: f64 = row.iter().zip(light).map(|(m, c)| m * c).sum();
            sum.clamp(0.0, 1.0)
        })
    }

    /// CIE L*a*b* under D65 of what `vision` sees in place of `color`.
    fn lab(color: Rgb, vision: Vision) -> [f64; 3] {
        let [r, g, b] = simulated(color, vision);
        let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
        let f = |t: f64| {
            if t > 0.008856 {
                t.cbrt()
            } else {
                7.787 * t + 16.0 / 116.0
            }
        };
        [
            116.0 * f(y) - 16.0,
            500.0 * (f(x) - f(y)),
            200.0 * (f(y) - f(z)),
        ]
    }

    /// Each pair of the eight colour tokens, as a solid fill and as an
    /// outline, stays apart for normal vision and under deuteranopia,
    /// protanopia and tritanopia. `neutral` counts: it is a colour a user
    /// picks, drawn in `ink` (or filled with `raised`), so it must not pass
    /// for a hue.
    #[test]
    fn board_hues_stay_apart_under_colour_blindness() {
        use wren_board::{style, Color, Fill, Token, Vocabulary};

        let visions = [
            Vision::Normal,
            Vision::Deuteranopia,
            Vision::Protanopia,
            Vision::Tritanopia,
        ];
        let hues = Token::ALL;
        assert_eq!(hues.len(), 8);
        for theme in BUILT_INS {
            let palette = board_palette(&theme.colors);
            for (drawn, fill) in [("solid", Fill::Solid), ("stroke", Fill::None)] {
                let colours: Vec<Rgb> = hues
                    .iter()
                    .map(|token| {
                        let colors = style::resolve(&Color::Token(*token), fill, &palette);
                        rgb(match fill {
                            Fill::Solid => colors.fill.expect("a solid fill"),
                            _ => colors.stroke,
                        })
                    })
                    .collect();
                for vision in visions {
                    for (i, j) in (0..8).flat_map(|i| (i + 1..8).map(move |j| (i, j))) {
                        let (a, b) = (lab(colours[i], vision), lab(colours[j], vision));
                        let delta_e = a
                            .iter()
                            .zip(b)
                            .map(|(p, q)| (p - q).powi(2))
                            .sum::<f64>()
                            .sqrt();
                        let delta_l = (a[0] - b[0]).abs();
                        assert!(
                            delta_e >= APART_DELTA_E || delta_l >= APART_DELTA_L,
                            "{}: {} and {} at {drawn} under {vision:?}: ΔE {delta_e:.1}, ΔL* {delta_l:.1}",
                            theme.name,
                            hues[i].name(),
                            hues[j].name(),
                        );
                    }
                }
            }
        }
    }

    /// A yellow darker than its theme's red or orange reads as khaki or
    /// olive, not yellow.
    #[test]
    fn board_yellow_is_the_lightest_warm_hue() {
        for theme in BUILT_INS {
            let c = theme.colors;
            let yellow = lightness(c.yellow);
            for (name, hue) in [("red", c.red), ("orange", c.orange)] {
                let other = lightness(hue);
                assert!(
                    yellow > other,
                    "{}: yellow L* {yellow:.1} not above {name} L* {other:.1}",
                    theme.name
                );
            }
        }
    }

    /// The reference script's values for two colours under each vision.
    #[test]
    fn the_vision_simulation_matches_its_reference_values() {
        let rows = [
            (
                Vision::Normal,
                [67.96, 39.54, 17.54],
                [54.72, 18.79, -70.92],
            ),
            (
                Vision::Deuteranopia,
                [70.85, -2.82, 21.6],
                [52.77, 21.97, -72.72],
            ),
            (
                Vision::Protanopia,
                [63.13, -1.25, 9.83],
                [58.91, 12.12, -64.14],
            ),
            (
                Vision::Tritanopia,
                [67.4, 51.52, 16.66],
                [60.07, -27.33, -20.24],
            ),
        ];
        for (vision, salmon, azure) in rows {
            for (color, want) in [(Rgb(240, 136, 136), salmon), (Rgb(0, 128, 255), azure)] {
                let got = lab(color, vision);
                let close = got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.01);
                assert!(close, "{color:?} under {vision:?}: {got:?}");
            }
        }
    }
}
