//! The kinds of note a workspace holds, decided by the file name alone.
//!
//! Kind is a case-insensitive match of the whole name's last extension, with
//! a non-empty stem in front of it, so `x.md.wren-board` is a board titled
//! `x.md` and `x.wren-board.md` is Markdown.

use std::ffi::OsStr;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NoteKind {
    Markdown,
    Board,
}

impl NoteKind {
    pub const ALL: [NoteKind; 2] = [NoteKind::Markdown, NoteKind::Board];

    /// The kind a file name gives, or `None` for a name that is not a note.
    pub fn from_name(name: &OsStr) -> Option<Self> {
        let name = name.to_string_lossy();
        Self::ALL
            .into_iter()
            .find(|kind| kind.stem_len(&name).is_some())
    }

    /// The extension, with its dot, that every file of this kind carries.
    pub fn extension(self) -> &'static str {
        match self {
            NoteKind::Markdown => ".md",
            NoteKind::Board => ".wren-board",
        }
    }

    /// The title shown for a file of this kind: the name without the kind's
    /// extension. A name that does not carry it is returned whole.
    pub fn title(self, name: &OsStr) -> String {
        let name = name.to_string_lossy();
        match self.stem_len(&name) {
            Some(len) => name[..len].to_owned(),
            None => name.into_owned(),
        }
    }

    /// The bytes a new note of this kind starts with.
    pub fn template(self) -> &'static str {
        match self {
            NoteKind::Markdown => "",
            NoteKind::Board => "schema = 1\n",
        }
    }

    /// The byte length of the stem when `name` ends in this kind's extension
    /// after at least one other byte. The extension is ASCII, so the stem
    /// always ends on a character boundary.
    fn stem_len(self, name: &str) -> Option<usize> {
        let extension = self.extension();
        let stem = name.len().checked_sub(extension.len())?;
        (stem > 0 && name.as_bytes()[stem..].eq_ignore_ascii_case(extension.as_bytes()))
            .then_some(stem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(name: &str) -> Option<NoteKind> {
        NoteKind::from_name(OsStr::new(name))
    }

    #[test]
    fn kind_is_the_last_extension_with_a_stem() {
        for (name, expected) in [
            ("a.md", Some((NoteKind::Markdown, "a"))),
            ("A.MD", Some((NoteKind::Markdown, "A"))),
            (".md", None),
            ("x.wren-board", Some((NoteKind::Board, "x"))),
            ("x.WREN-BOARD", Some((NoteKind::Board, "x"))),
            (
                "x.wren-board.md",
                Some((NoteKind::Markdown, "x.wren-board")),
            ),
            ("x.md.wren-board", Some((NoteKind::Board, "x.md"))),
            ("wren-board", None),
            (".wren-board", None),
            ("md", None),
            ("x.Wren-Board", Some((NoteKind::Board, "x"))),
            ("note.txt", None),
            ("readme", None),
            ("x.wren-boards", None),
            ("x.mdx", None),
        ] {
            let found = kind(name);
            assert_eq!(found, expected.map(|(kind, _)| kind), "{name}");
            if let Some((kind, title)) = expected {
                assert_eq!(kind.title(OsStr::new(name)), title, "{name}");
            }
        }
    }

    #[test]
    fn title_keeps_a_name_without_the_extension() {
        assert_eq!(NoteKind::Markdown.title(OsStr::new(".md")), ".md");
        assert_eq!(NoteKind::Markdown.title(OsStr::new("x.md.md")), "x.md");
        assert_eq!(
            NoteKind::Markdown.title(OsStr::new("data.json")),
            "data.json"
        );
        assert_eq!(NoteKind::Board.title(OsStr::new("Flow.md")), "Flow.md");
        assert_eq!(
            NoteKind::Board.title(OsStr::new(".wren-board")),
            ".wren-board"
        );
    }

    #[test]
    fn markdown_titles_match_released_wren() {
        for (name, title) in [
            ("design.md", "design"),
            ("Design.MD", "Design"),
            ("notes.Md", "notes"),
            ("UPPER.mD", "UPPER"),
            ("a.md", "a"),
            ("file.txt", "file.txt"),
            ("readme", "readme"),
            ("\u{1f426}", "\u{1f426}"),
            ("éé", "éé"),
            ("é.M", "é.M"),
        ] {
            assert_eq!(NoteKind::Markdown.title(OsStr::new(name)), title);
        }
    }

    #[test]
    fn title_keeps_unicode_stems() {
        for (name, title) in [
            ("café.md", "café"),
            ("日本語.md", "日本語"),
            ("🎵.MD", "🎵"),
            ("流れ.wren-board", "流れ"),
        ] {
            let kind = kind(name).unwrap();
            assert_eq!(kind.title(OsStr::new(name)), title);
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_names_keep_their_kind() {
        use std::os::unix::ffi::OsStrExt;
        let name = OsStr::from_bytes(b"\xff.wren-board");
        assert_eq!(NoteKind::from_name(name), Some(NoteKind::Board));
        assert_eq!(NoteKind::Board.title(name), "\u{fffd}");
        let name = OsStr::from_bytes(b"\xff.md");
        assert_eq!(NoteKind::from_name(name), Some(NoteKind::Markdown));
    }

    #[test]
    fn templates() {
        assert_eq!(NoteKind::Markdown.template(), "");
        assert_eq!(NoteKind::Board.template(), "schema = 1\n");
        for kind in NoteKind::ALL {
            assert_eq!(
                NoteKind::from_name(OsStr::new(&format!("n{}", kind.extension()))),
                Some(kind)
            );
        }
    }
}
