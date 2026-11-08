use std::sync::{OnceLock, PoisonError};

use iced::advanced::graphics::text::font_system;
use iced::Font;

pub const UI: Font = Font::with_name("Inter");
pub const MEDIUM: Font = Font {
    weight: iced::font::Weight::Medium,
    ..UI
};
pub const SEMIBOLD: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..UI
};
pub const BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..UI
};
/// The bundled editor family, used when `editor-font-family` is unset.
pub const EDITOR_FAMILY: &str = "JetBrains Mono";
pub const EDITOR: Font = Font::with_name(EDITOR_FAMILY);

/// Monospace families listed first when installed, in this order (Roost's
/// picker order), even when their metadata does not say monospace.
const CURATED: &[&str] = &[
    EDITOR_FAMILY,
    "JetBrainsMono Nerd Font",
    "Fira Code",
    "Fira Mono",
    "Hack",
    "Source Code Pro",
    "Cascadia Code",
    "Cascadia Mono",
    "IBM Plex Mono",
    "Inconsolata",
    "Iosevka",
    "DejaVu Sans Mono",
    "Ubuntu Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "SF Mono",
    "Menlo",
    "Monaco",
];

pub const FACES: &[&[u8]] = &[
    include_bytes!("../../../third_party/inter/Inter-Regular.ttf"),
    include_bytes!("../../../third_party/inter/Inter-Medium.ttf"),
    include_bytes!("../../../third_party/inter/Inter-SemiBold.ttf"),
    include_bytes!("../../../third_party/inter/Inter-Bold.ttf"),
    include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-Bold.ttf"),
    include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-Italic.ttf"),
    include_bytes!("../../../third_party/jetbrains-mono/JetBrainsMono-BoldItalic.ttf"),
];

pub const LICENSES: &str = concat!(
    "Inter\n\n",
    include_str!("../../../third_party/inter/LICENSE.txt"),
    "\n\nJetBrains Mono\n\n",
    include_str!("../../../third_party/jetbrains-mono/OFL.txt"),
);

/// Installed font families, from one scan of the renderer's font database.
pub struct Families {
    /// Every family, canonical spelling, sorted case-insensitively.
    all: Vec<String>,
    /// The editor font picker's rows: curated, then other monospace families.
    monospace: Vec<usize>,
}

impl Families {
    fn from_faces(faces: impl IntoIterator<Item = (String, bool)>) -> Self {
        let mut faces: Vec<_> = faces
            .into_iter()
            // The bundled font is always available, even before iced has
            // loaded it into the database.
            .chain([(EDITOR_FAMILY.to_string(), true)])
            // Names starting with `.` are macOS private system fonts.
            .filter(|(name, _)| {
                !name.is_empty()
                    && name.trim() == name
                    && !name.starts_with('.')
                    && !name.chars().any(|c| c.is_control() || c == '"')
            })
            .collect();
        faces.sort_by(|(left, _), (right, _)| {
            left.to_lowercase()
                .cmp(&right.to_lowercase())
                .then_with(|| left.cmp(right))
        });
        let mut all: Vec<String> = Vec::new();
        let mut is_monospace: Vec<bool> = Vec::new();
        for (name, monospace) in faces {
            match all.last() {
                Some(last) if last.eq_ignore_ascii_case(&name) => {
                    *is_monospace.last_mut().expect("parallel vectors") |= monospace;
                }
                _ => {
                    all.push(name);
                    is_monospace.push(monospace);
                }
            }
        }
        let find = |name: &str| {
            all.iter()
                .position(|family| family.eq_ignore_ascii_case(name))
        };
        let mut monospace: Vec<usize> = CURATED.iter().filter_map(|name| find(name)).collect();
        // Color emoji fonts claim a fixed pitch but have no text glyphs.
        let text_font = |name: &str| !name.to_lowercase().contains("emoji");
        for index in (0..all.len()).filter(|&index| is_monospace[index] && text_font(&all[index])) {
            if !monospace.contains(&index) {
                monospace.push(index);
            }
        }
        Self { all, monospace }
    }

    /// The editor font picker's rows.
    pub fn monospace(&self) -> impl Iterator<Item = &str> {
        self.monospace.iter().map(|&index| self.all[index].as_str())
    }

    /// The installed family named `name`, in any case. Any family resolves,
    /// not only monospace ones, so a proportional font can be configured.
    pub fn find(&self, name: &str) -> Option<&str> {
        self.all
            .iter()
            .find(|family| family.eq_ignore_ascii_case(name.trim()))
            .map(String::as_str)
    }
}

/// The installed families, scanned once on first use.
pub fn families() -> &'static Families {
    static FAMILIES: OnceLock<Families> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let mut system = font_system()
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let db = system.raw().db();
        let faces: Vec<_> = db
            .faces()
            .flat_map(|face| {
                let listed =
                    face.monospaced && db.with_face_data(face.id, has_outlines).unwrap_or(false);
                face.families
                    .iter()
                    .map(move |(name, _)| (name.clone(), listed))
            })
            .collect();
        Families::from_faces(faces)
    })
}

/// Whether a face draws its glyphs from outlines. Bitmap-only faces, such as
/// macOS's GB18030 Bitmap, come in a few fixed sizes and may have no glyph
/// advances, which lays every word out on its own line.
fn has_outlines(data: &[u8], index: u32) -> bool {
    use swash::tag_from_bytes;
    swash::FontRef::from_index(data, index as usize).is_some_and(|font| {
        [b"glyf", b"CFF ", b"CFF2"]
            .iter()
            .any(|tag| font.table(tag_from_bytes(tag)).is_some())
    })
}

/// The editor font for a configured family name; `None` when that family is
/// not installed.
pub fn editor_font(family: Option<&str>) -> Option<Font> {
    match family {
        None => Some(EDITOR),
        Some(name) => families().find(name).map(Font::with_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_dedupe_list_curated_first_and_resolve_any_case() {
        let families = Families::from_faces([
            ("Zed Mono".to_string(), true),
            ("fira code".to_string(), false),
            ("Fira Code".to_string(), true),
            ("Inter".to_string(), false),
            ("Alpha Mono".to_string(), true),
            ("Hack".to_string(), false),
            ("Noto Color Emoji".to_string(), true),
            (".SF NS Mono".to_string(), true),
            ("Bad \"Name\"".to_string(), true),
        ]);
        assert_eq!(
            families.monospace().collect::<Vec<_>>(),
            [EDITOR_FAMILY, "Fira Code", "Hack", "Alpha Mono", "Zed Mono"]
        );
        assert_eq!(families.find("FIRA CODE"), Some("Fira Code"));
        assert_eq!(families.find("inter"), Some("Inter"));
        assert_eq!(families.find(".SF NS Mono"), None);
        assert_eq!(families.find("Missing"), None);
    }

    #[test]
    fn only_outline_faces_count_as_editor_fonts() {
        assert!(FACES.iter().all(|face| has_outlines(face, 0)));
        let bitmap_only = sfnt(&[b"head", b"EBLC", b"EBDT"]);
        assert!(!has_outlines(&bitmap_only, 0));
        assert!(has_outlines(&sfnt(&[b"head", b"CFF "]), 0));
        assert!(!has_outlines(b"not a font", 0));
    }

    /// A font file holding only a table directory, with each table empty.
    fn sfnt(tags: &[&[u8; 4]]) -> Vec<u8> {
        let count = u16::try_from(tags.len()).expect("few tables");
        let mut data = 0x0001_0000u32.to_be_bytes().to_vec();
        data.extend(count.to_be_bytes());
        data.extend([0; 6]);
        let end = u32::try_from(12 + 16 * tags.len()).expect("small directory");
        for tag in tags {
            data.extend(*tag);
            data.extend([0; 4]);
            data.extend(end.to_be_bytes());
            data.extend(0u32.to_be_bytes());
        }
        data
    }

    #[test]
    fn bundled_faces_and_redistribution_notices_are_present() {
        assert_eq!(FACES.len(), 8);
        for face in FACES {
            assert!(face.len() > 1000);
            assert_eq!(&face[..4], &[0, 1, 0, 0]);
        }
        assert!(LICENSES.contains("The Inter Project Authors"));
        assert!(LICENSES.contains("The JetBrains Mono Project Authors"));
        assert_eq!(
            LICENSES
                .matches("SIL OPEN FONT LICENSE Version 1.1")
                .count(),
            2
        );
    }
}
