//! A board's export as a file: the SVG the scene writes, and that SVG
//! rendered by resvg as a PNG image. The widget, the SVG and the PNG all
//! come from one `scene::build`, so the image shows the lines the canvas
//! drew.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, fontdb, ImageHrefResolver};

use crate::fonts;

/// What an export writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Svg,
    Png,
}

impl Format {
    /// The file name extension, which an export inside the workspace must
    /// have.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
        }
    }

    /// The format's name as a person reads it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Svg => "SVG",
            Self::Png => "PNG",
        }
    }
}

/// How many image pixels a board unit takes in a PNG export.
pub const PNG_SCALE: f32 = 2.0;

/// The largest PNG export, in pixels: 64 megapixels, 256 MB of pixels
/// before encoding.
pub const MAX_PIXELS: u64 = 64_000_000;

/// The faces resvg lays an export's labels out in: the bundled Inter faces
/// the canvas measured with, then the system's, which give the glyphs Inter
/// has not (CJK among them). A system copy of Inter is left out, so a label
/// is never drawn in another version of the font than the one measured.
fn faces() -> Arc<fontdb::Database> {
    static FACES: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    Arc::clone(FACES.get_or_init(|| {
        let mut faces = fontdb::Database::new();
        faces.load_system_fonts();
        let inter: Vec<fontdb::ID> = faces
            .faces()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|(family, _)| family.eq_ignore_ascii_case("Inter"))
            })
            .map(|face| face.id)
            .collect();
        for id in inter {
            faces.remove_face(id);
        }
        for face in &fonts::FACES[..fonts::INTER_FACES] {
            faces.load_font_data(face.to_vec());
        }
        Arc::new(faces)
    }))
}

/// How well a face suits a character Inter lacks, best first: a colour
/// emoji face, a sans-serif one, any other but serif and monospace, then
/// those. resvg's own choice is the first face that has the character,
/// which on a common Linux system draws Japanese in a serif face.
fn fallback_rank(face: &fontdb::FaceInfo) -> u8 {
    let name = face
        .families
        .first()
        .map(|(family, _)| family.to_ascii_lowercase())
        .unwrap_or_default();
    if name.contains("emoji") {
        0
    } else if face.monospaced || name.contains("mono") {
        3
    } else if name.contains("sans") {
        1
    } else if name.contains("serif") {
        3
    } else {
        2
    }
}

/// The face a character Inter lacks is drawn in: the first that has it of
/// the best [`fallback_rank`], in the label's style before another. Faces
/// are tried a rank at a time, so a character found early looks at few.
fn fallback() -> usvg::FallbackSelectionFn<'static> {
    Box::new(|character, used, faces| {
        let faces: &fontdb::Database = faces;
        let style = faces.face(*used.first()?)?.style;
        (0..4u8)
            .flat_map(|rank| [(rank, true), (rank, false)])
            .find_map(|(rank, same_style)| {
                faces
                    .faces()
                    .filter(|face| {
                        fallback_rank(face) == rank
                            && (face.style == style) == same_style
                            && !used.contains(&face.id)
                    })
                    .find(|face| has_char(faces, face.id, character))
                    .map(|face| face.id)
            })
    })
}

/// Whether the face `id` maps `character` to a glyph.
fn has_char(faces: &fontdb::Database, id: fontdb::ID, character: char) -> bool {
    faces
        .with_face_data(id, |data, index| {
            swash::FontRef::from_index(data, index as usize)
                .is_some_and(|font| font.charmap().map(character) != 0)
        })
        .unwrap_or(false)
}

/// `svg`, a board's SVG export, as a PNG image `scale` pixels to the board
/// unit, its size the SVG's rounded up. An image over [`MAX_PIXELS`] is
/// refused before anything is drawn. No image or other file the document
/// names is read: a board's export names none, and this one never will.
pub fn png(svg: &str, scale: f32) -> Result<Vec<u8>, String> {
    let options = usvg::Options {
        font_family: "Inter".to_owned(),
        fontdb: faces(),
        font_resolver: usvg::FontResolver {
            select_font: usvg::FontResolver::default_font_selector(),
            select_fallback: fallback(),
        },
        image_href_resolver: ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|error| format!("the board's SVG could not be read: {error}"))?;
    let size = tree.size();
    let (width, height) = (
        f64::from(size.width() * scale).ceil(),
        f64::from(size.height() * scale).ceil(),
    );
    if !(width.is_finite() && height.is_finite() && width >= 1.0 && height >= 1.0) {
        return Err(format!("{width} by {height} pixels is no image"));
    }
    if width * height > MAX_PIXELS as f64 {
        return Err(format!(
            "{width} by {height} pixels is over the {} megapixels a PNG export may have",
            MAX_PIXELS / 1_000_000
        ));
    }
    // Both are whole numbers from 1 to 64 million here.
    let (columns, rows) = (width as u32, height as u32);
    let mut pixmap = Pixmap::new(columns, rows)
        .ok_or_else(|| format!("{columns} by {rows} pixels could not be allocated"))?;
    // The SVG's size scaled exactly to the whole pixels, so its background
    // reaches every edge.
    let transform =
        Transform::from_scale(columns as f32 / size.width(), rows as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|error| format!("the image could not be encoded: {error}"))
}

/// Write `contents` to `path` whole or not at all: into a new file beside
/// it, renamed over it once written. Returns the bytes written.
pub fn write(path: &Path, contents: &[u8]) -> std::io::Result<u64> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "the path names no file")
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = OsString::from(".");
    temporary.push(name);
    temporary.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = parent.join(temporary);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(contents)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        // The write's own error is the one to report; the half-written
        // file is removed if it was made.
        let _ = std::fs::remove_file(&temporary);
    }
    written.map(|()| contents.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixel at `(x, y)` of a decoded RGBA image `width` pixels wide.
    fn pixel(image: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        [image[at], image[at + 1], image[at + 2], image[at + 3]]
    }

    /// `png`'s image decoded: its width, height and RGBA pixels.
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let pixmap = Pixmap::decode_png(png).expect("a PNG image");
        let (width, height) = (pixmap.width(), pixmap.height());
        let pixels = pixmap
            .pixels()
            .iter()
            .flat_map(|pixel| {
                let color = pixel.demultiply();
                [color.red(), color.green(), color.blue(), color.alpha()]
            })
            .collect();
        (width, height, pixels)
    }

    #[test]
    fn the_image_is_the_svg_scaled_and_rounded_up() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100.5\" height=\"40\" \
                   viewBox=\"-10 -10 100.5 40\"><rect x=\"-10\" y=\"-10\" width=\"100.5\" \
                   height=\"40\" fill=\"#102030\"/><rect x=\"0\" y=\"0\" width=\"20\" \
                   height=\"10\" fill=\"#c04020\"/></svg>";
        let (width, height, pixels) = decode(&png(svg, 2.0).unwrap());
        assert_eq!((width, height), (201, 80));
        // The background reaches every corner, and the shape is where the
        // view box puts it.
        for (x, y) in [(0, 0), (200, 0), (0, 79), (200, 79)] {
            assert_eq!(pixel(&pixels, width, x, y), [0x10, 0x20, 0x30, 255]);
        }
        assert_eq!(pixel(&pixels, width, 40, 30), [0xc0, 0x40, 0x20, 255]);
        let (width, height, _) = decode(&png(svg, 1.0).unwrap());
        assert_eq!((width, height), (101, 40));
    }

    #[test]
    fn an_image_over_the_limit_is_refused_and_a_document_that_is_not_svg_fails() {
        let side = |side: u32| {
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{side}\" height=\"{side}\" \
                 viewBox=\"0 0 {side} {side}\"/>"
            )
        };
        // 8,000 by 8,000 pixels is 64 megapixels; one more row is over.
        let over = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4000\" \
                    height=\"4000.5\" viewBox=\"0 0 4000 4000.5\"/>";
        let error = png(over, 2.0).unwrap_err();
        assert!(error.contains("over the 64 megapixels"), "{error}");
        let error = png(&side(16_000_000), 4.0).unwrap_err();
        assert!(error.contains("megapixels"), "{error}");
        assert!(png("not an image", 2.0).is_err());
        let (width, height, _) = decode(&png(&side(1), 2.0).unwrap());
        assert_eq!((width, height), (2, 2));
    }

    #[test]
    fn no_image_the_document_names_is_drawn() {
        // A red square as SVG, inline and in a file. resvg's defaults would
        // draw both.
        const RED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#;
        const INLINE: &str = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMCIgaGVpZ2h0PSIxMCI+PHJlY3Qgd2lkdGg9IjEwIiBoZWlnaHQ9IjEwIiBmaWxsPSJyZWQiLz48L3N2Zz4=";
        let directory = crate::test_support::tempdir();
        let file = directory.path().join("red.svg");
        std::fs::write(&file, RED).unwrap();
        let red = |pixels: &[u8]| pixels.chunks(4).filter(|pixel| pixel[3] > 0).count();
        for href in [INLINE.to_owned(), file.display().to_string()] {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" \
                 xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"10\" height=\"10\">\
                 <image width=\"10\" height=\"10\" xlink:href=\"{href}\"/></svg>"
            );
            let (_, _, pixels) = decode(&png(&svg, 1.0).unwrap());
            assert_eq!(red(&pixels), 0, "{href}");

            let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
            let mut pixmap = Pixmap::new(10, 10).unwrap();
            resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
            let (_, _, pixels) = decode(&pixmap.encode_png().unwrap());
            assert_eq!(red(&pixels), 100, "the defaults draw {href}");
        }
    }

    #[test]
    fn labels_are_drawn_in_the_bundled_inter() {
        let faces = faces();
        let inter: Vec<_> = faces
            .faces()
            .filter(|face| face.families.iter().any(|(family, _)| family == "Inter"))
            .collect();
        assert_eq!(inter.len(), fonts::INTER_FACES);
        assert!(inter
            .iter()
            .all(|face| matches!(face.source, fontdb::Source::Binary(_))));
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"80\" height=\"30\">\
                   <text x=\"4\" y=\"20\" font-family=\"Inter, system-ui, sans-serif\" \
                   font-size=\"14\" fill=\"#000000\">Inter</text></svg>";
        let (_, _, pixels) = decode(&png(svg, 1.0).unwrap());
        assert!(pixels.chunks(4).filter(|pixel| pixel[3] > 0).count() > 20);
    }

    #[test]
    fn a_character_inter_lacks_takes_an_emoji_then_a_sans_serif_face() {
        let face = |family: &str, monospaced: bool| fontdb::FaceInfo {
            id: fontdb::ID::dummy(),
            source: fontdb::Source::Binary(Arc::new(Vec::<u8>::new())),
            index: 0,
            families: vec![(family.to_owned(), fontdb::Language::English_UnitedStates)],
            post_script_name: String::new(),
            style: fontdb::Style::Normal,
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            monospaced,
        };
        let ranks = [
            ("Noto Color Emoji", false),
            ("Noto Sans CJK JP", false),
            ("Noto Sans Mono CJK JP", false),
            ("Noto Serif CJK JP", false),
            ("Unifont", true),
            ("Lohit Devanagari", false),
            ("DejaVu Sans", false),
        ]
        .map(|(family, monospaced)| fallback_rank(&face(family, monospaced)));
        assert_eq!(ranks, [0, 1, 3, 3, 3, 2, 1]);
    }

    #[test]
    fn a_write_replaces_the_file_whole_and_leaves_nothing_on_failure() {
        let directory = crate::test_support::tempdir();
        let target = directory.path().join("out.png");
        std::fs::write(&target, "old").unwrap();
        assert_eq!(write(&target, b"new bytes").unwrap(), 9);
        assert_eq!(std::fs::read(&target).unwrap(), b"new bytes");
        assert!(write(&directory.path().join("missing/out.png"), b"x").is_err());
        let names: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["out.png"]);
    }
}
