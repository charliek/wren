//! A scene as an SVG document: the export of what the canvas draws.
//!
//! Every path item is one `<path>` and every wrapped line one `<text>` at
//! an explicit position, so a viewer lays nothing out and the file shows
//! the lines the canvas drew. Text is escaped and stripped of what XML
//! cannot carry; colours come only from the scene's `Rgba` values, so no
//! file-derived string reaches an attribute.

use std::fmt::Write as _;

use crate::scene::{self, Draw, Line, Point, Rect, Scene, Segment, Stroke};
use crate::Rgba;

/// The labels' font: Inter, which Wren bundles, then the platform's.
pub const FONT_FAMILY: &str = "Inter, system-ui, sans-serif";

/// Inter's ascent and descent as fractions of the font size, from its
/// `hhea` table (1984 and 494 of 2048 units). A line's glyphs are centred
/// in its line box as cosmic-text centres them, so the baseline sits
/// `(ASCENT - DESCENT) / 2` of the size below the box's middle.
pub const ASCENT: f32 = 0.968_75;
pub const DESCENT: f32 = 0.241_210_94;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Options {
    /// A rectangle of this colour behind everything; `None` leaves the
    /// document transparent.
    pub background: Option<Rgba>,
    /// Space added on every side of the scene's bounds.
    pub padding: f32,
}

/// `scene` as a standalone SVG document whose view box is the scene's
/// bounds grown by the padding.
pub fn write(scene: &Scene, options: &Options) -> String {
    let pad = if options.padding.is_finite() {
        options.padding.max(0.0)
    } else {
        0.0
    };
    let b = scene.bounds;
    let view = Rect::new(
        b.x - pad,
        b.y - pad,
        b.width + 2.0 * pad,
        b.height + 2.0 * pad,
    );
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let (x, y, w, h) = (num(view.x), num(view.y), num(view.width), num(view.height));
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"{x} {y} {w} {h}\">"
    );
    if let Some(color) = options.background {
        let _ = writeln!(
            out,
            "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\"{}/>",
            paint("fill", color)
        );
    }
    for item in &scene.items {
        match &item.draw {
            Draw::Path {
                segments,
                fill,
                stroke,
            } => path(&mut out, segments, *fill, stroke.as_ref()),
            Draw::Text {
                lines, size, color, ..
            } => {
                for line in lines {
                    text(&mut out, line, *size, *color);
                }
            }
        }
    }
    out.push_str("</svg>\n");
    out
}

fn path(out: &mut String, segments: &[Segment], fill: Option<Rgba>, stroke: Option<&Stroke>) {
    let mut d = String::new();
    let p = |p: Point| format!("{} {}", num(p.x), num(p.y));
    for segment in segments {
        let _ = match *segment {
            Segment::MoveTo(to) => write!(d, "M{}", p(to)),
            Segment::LineTo(to) => write!(d, "L{}", p(to)),
            Segment::QuadTo { control, to } => write!(d, "Q{} {}", p(control), p(to)),
            Segment::CubicTo { first, second, to } => {
                write!(d, "C{} {} {}", p(first), p(second), p(to))
            }
            Segment::Close => write!(d, "Z"),
        };
    }
    let _ = write!(out, "<path d=\"{d}\"");
    match fill {
        Some(color) => out.push_str(&paint("fill", color)),
        None => out.push_str(" fill=\"none\""),
    }
    if let Some(stroke) = stroke {
        out.push_str(&paint("stroke", stroke.color));
        let _ = write!(out, " stroke-width=\"{}\"", num(stroke.width));
        if let Some([on, off]) = stroke.dash {
            let _ = write!(out, " stroke-dasharray=\"{} {}\"", num(on), num(off));
        }
    }
    out.push_str("/>\n");
}

fn text(out: &mut String, line: &Line, size: f32, color: Rgba) {
    let middle = line.origin.y + scene::line_height(size) / 2.0;
    let baseline = middle + (ASCENT - DESCENT) / 2.0 * size;
    let _ = writeln!(
        out,
        "<text x=\"{}\" y=\"{}\" font-family=\"{FONT_FAMILY}\" font-size=\"{}\"{} xml:space=\"preserve\">{}</text>",
        num(line.origin.x),
        num(baseline),
        num(size),
        paint("fill", color),
        escape(&line.text)
    );
}

/// A colour attribute, with its opacity when it is not opaque.
fn paint(attribute: &str, color: Rgba) -> String {
    let mut out = format!(
        " {attribute}=\"#{:02x}{:02x}{:02x}\"",
        color.r, color.g, color.b
    );
    if color.a != 255 {
        let _ = write!(
            out,
            " {attribute}-opacity=\"{}\"",
            num(f32::from(color.a) / 255.0)
        );
    }
    out
}

/// A number as SVG reads it: the shortest decimal that is the same `f32`,
/// never in exponent form, with no negative zero and 0 for a non-finite
/// value.
fn num(value: f32) -> String {
    if !value.is_finite() || value == 0.0 {
        return "0".into();
    }
    format!("{value}")
}

/// Text content with markup escaped and every character XML 1.0 cannot
/// carry removed, as are carriage returns, which a parser would turn into
/// line feeds.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push(c),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::tests::{dump, golden_scenes};
    use crate::scene::{build, FixedMeasure, Item, Source};
    use crate::style::tests::LIGHT;
    use crate::{fixtures, parse};

    /// An element's name, attributes and unescaped text.
    type Element = (String, Vec<(String, String)>, String);

    /// A well-formed XML document of elements, attributes, text and the
    /// five predefined entities: what this writer can emit. It returns the
    /// elements in order, each with its attributes and unescaped text, or
    /// what is wrong.
    fn elements(xml: &str) -> Result<Vec<Element>, String> {
        fn unescape(text: &str) -> Result<String, String> {
            let mut out = String::new();
            let mut rest = text;
            while let Some(at) = rest.find('&') {
                out.push_str(&rest[..at]);
                let end = rest[at..].find(';').ok_or("an unterminated entity")?;
                out.push(match &rest[at + 1..at + end] {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    other => return Err(format!("an unknown entity {other}")),
                });
                rest = &rest[at + end + 1..];
            }
            out.push_str(rest);
            Ok(out)
        }
        for c in xml.chars() {
            let legal = matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..);
            if !legal {
                return Err(format!("the character {c:?}"));
            }
        }
        let body = xml
            .strip_prefix("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")
            .ok_or("no declaration")?;
        let mut found: Vec<Element> = Vec::new();
        // The innermost open elements, by name and index into `found`.
        let mut open: Vec<(String, usize)> = Vec::new();
        let mut rest = body;
        while !rest.is_empty() {
            let Some(at) = rest.find('<') else {
                if rest.trim().is_empty() {
                    break;
                }
                return Err(format!("text outside the root: {rest:?}"));
            };
            let content = &rest[..at];
            if content.contains('>') || content.contains("]]>") {
                return Err(format!("a bare > in {content:?}"));
            }
            if let Some(&(_, index)) = open.last() {
                found[index].2.push_str(&unescape(content)?);
            }
            let end = rest[at..].find('>').ok_or("an unterminated tag")? + at;
            let tag = &rest[at + 1..end];
            rest = &rest[end + 1..];
            if let Some(name) = tag.strip_prefix('/') {
                if open.pop().map(|(open, _)| open).as_deref() != Some(name) {
                    return Err(format!("a stray </{name}>"));
                }
                continue;
            }
            let (tag, empty) = match tag.strip_suffix('/') {
                Some(tag) => (tag, true),
                None => (tag, false),
            };
            let (name, mut attributes) = tag.split_once(' ').unwrap_or((tag, ""));
            let mut parsed = Vec::new();
            while !attributes.trim().is_empty() {
                let (key, value) = attributes
                    .trim_start()
                    .split_once("=\"")
                    .ok_or(format!("a bad attribute in <{tag}>"))?;
                let close = value.find('"').ok_or("an unquoted attribute")?;
                if value[..close].contains('<') {
                    return Err("a < in an attribute".into());
                }
                parsed.push((key.to_owned(), unescape(&value[..close])?));
                attributes = &value[close + 1..];
            }
            if open.is_empty() && !found.is_empty() {
                return Err(format!("a second root <{name}>"));
            }
            if !empty {
                open.push((name.to_owned(), found.len()));
            }
            found.push((name.to_owned(), parsed, String::new()));
        }
        if !open.is_empty() {
            return Err(format!("unclosed {open:?}"));
        }
        Ok(found)
    }

    fn texts(xml: &str) -> Vec<String> {
        elements(xml)
            .unwrap_or_else(|e| panic!("{e}\n{xml}"))
            .into_iter()
            .filter(|(name, _, _)| name == "text")
            .map(|(_, _, text)| text)
            .collect()
    }

    fn lines(scene: &Scene) -> Vec<&str> {
        scene
            .items
            .iter()
            .filter_map(|item| match &item.draw {
                Draw::Text { lines, .. } => Some(lines),
                Draw::Path { .. } => None,
            })
            .flatten()
            .map(|line| line.text.as_str())
            .collect()
    }

    fn paths(scene: &Scene) -> usize {
        let is_path = |item: &&Item| matches!(item.draw, Draw::Path { .. });
        scene.items.iter().filter(is_path).count()
    }

    macro_rules! svg_golden {
        ($name:literal) => {
            (
                $name,
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/golden/svg/",
                    $name,
                    ".svg"
                )),
            )
        };
    }

    /// The SVG of each golden scene, the Checkout flow with a background
    /// and padding as the app exports it.
    #[test]
    fn svg_files_match_their_golden_files() {
        let goldens = [
            svg_golden!("checkout-flow"),
            svg_golden!("shapes"),
            svg_golden!("overlap"),
            svg_golden!("future"),
            svg_golden!("hand-written"),
            svg_golden!("password-reset"),
            svg_golden!("shapes-and-ends"),
            svg_golden!("fields"),
            svg_golden!("unknown"),
            svg_golden!("escapes"),
        ];
        let scenes = golden_scenes();
        assert_eq!(goldens.len(), scenes.len());
        for ((name, expected), (scene_name, board, _)) in goldens.into_iter().zip(scenes) {
            assert_eq!(name, scene_name);
            let scene = build(&parse(board).unwrap(), &LIGHT, &FixedMeasure);
            let options = if name == "checkout-flow" {
                Options {
                    background: Some(LIGHT.canvas),
                    padding: 40.0,
                }
            } else {
                Options::default()
            };
            let svg = write(&scene, &options);
            fixtures::assert_golden_file(&format!("svg/{name}.svg"), expected, &svg);
            let found = elements(&svg).unwrap_or_else(|e| panic!("{name}: {e}"));
            let count = |tag: &str| found.iter().filter(|(n, _, _)| n == tag).count();
            assert_eq!(
                count("path"),
                paths(&scene),
                "{name}: one <path> per path item"
            );
            let readable: Vec<String> = lines(&scene).into_iter().map(escape_free).collect();
            assert_eq!(texts(&svg), readable, "{name}: one <text> per line");
            assert_eq!(count("rect"), usize::from(options.background.is_some()));
            assert!(!dump(&scene).is_empty());
        }
    }

    #[track_caller]
    fn label_svg(label: &str) -> String {
        let text = format!(
            "schema = 1\n[node.n]\nkind = \"shape\"\nat = [0, 0]\nsize = [2000, 60]\nz = \"V\"\ntext = {}\n",
            toml::Value::String(label.into())
        );
        write(
            &build(&parse(&text).unwrap(), &LIGHT, &FixedMeasure),
            &Options::default(),
        )
    }

    #[test]
    fn markup_in_labels_is_escaped() {
        for label in [
            "a < b & \"c\"",
            "</text><text>",
            "]]>",
            "<![CDATA[x]]>",
            "&amp; stays &amp;",
            "it's",
            "<?xml?>",
            "<!-- -->",
        ] {
            let svg = label_svg(label);
            assert_eq!(texts(&svg), [label], "{svg}");
        }
        let svg = label_svg("a < b & \"c\"");
        assert!(
            svg.contains(">a &lt; b &amp; &quot;c&quot;</text>"),
            "{svg}"
        );
    }

    #[test]
    fn characters_xml_cannot_carry_are_stripped() {
        // Line feeds and carriage returns break a label into lines.
        let controls: String = ('\u{0}'..='\u{1f}')
            .filter(|&c| c != '\n' && c != '\r')
            .collect();
        let label = format!("a{controls}b\u{fffe}\u{ffff}\u{7f}c\u{feff}é🚀");
        let svg = label_svg(&label);
        assert_eq!(texts(&svg), ["a\tb\u{7f}c\u{feff}é🚀"], "{svg}");
        // So a carriage return reaches the writer only from a model built
        // in code, and is dropped too.
        let all: String = ('\u{0}'..='\u{1f}').collect();
        assert_eq!(escape(&format!("<{all}\u{fffe}\u{ffff}>")), "&lt;\t&gt;");
    }

    #[test]
    fn lines_sit_at_their_scene_positions() {
        let svg = label_svg("Start");
        // The line box's top is at y 20; the baseline is 15.09 px below.
        let found = elements(&svg).unwrap();
        let (_, attributes, _) = found.iter().find(|(n, _, _)| n == "text").unwrap();
        let get = |key: &str| {
            attributes
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .unwrap()
        };
        assert_eq!(get("x"), "982.5");
        assert_eq!(get("y"), "35.092773");
        assert_eq!(get("font-family"), FONT_FAMILY);
        assert_eq!(get("font-size"), "14");
        assert_eq!(get("fill"), "#202428");
    }

    #[test]
    fn options_set_the_view_box_and_background() {
        let scene = Scene {
            bounds: Rect::new(-10.0, 5.0, 100.0, 50.0),
            items: vec![Item {
                source: Source::Node(crate::Id::new("n").unwrap()),
                bounds: Rect::new(-10.0, 5.0, 100.0, 50.0),
                draw: Draw::Path {
                    segments: vec![
                        Segment::MoveTo(Point::new(-10.0, 5.0)),
                        Segment::LineTo(Point::new(90.0, 55.0)),
                    ],
                    fill: Some(Rgba {
                        r: 1,
                        g: 2,
                        b: 3,
                        a: 51,
                    }),
                    stroke: None,
                },
            }],
        };
        let plain = write(&scene, &Options::default());
        assert!(plain.contains("width=\"100\" height=\"50\" viewBox=\"-10 5 100 50\""));
        assert!(plain.contains("<path d=\"M-10 5L90 55\" fill=\"#010203\" fill-opacity=\"0.2\"/>"));
        let framed = write(
            &scene,
            &Options {
                background: Some(Rgba::rgb(255, 255, 255)),
                padding: 20.0,
            },
        );
        assert!(framed.contains("width=\"140\" height=\"90\" viewBox=\"-30 -15 140 90\""));
        assert!(framed
            .contains("<rect x=\"-30\" y=\"-15\" width=\"140\" height=\"90\" fill=\"#ffffff\"/>"));
        for padding in [f32::NAN, f32::INFINITY, -5.0] {
            let odd = write(
                &scene,
                &Options {
                    background: None,
                    padding,
                },
            );
            assert_eq!(odd, plain);
        }
    }

    #[test]
    fn numbers_are_plain_decimals() {
        assert_eq!(num(0.0), "0");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(f32::NAN), "0");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(-16_777_216.0), "-16777216");
        assert_eq!(num(1e-7), "0.0000001");
        assert_eq!(num(3e20), "300000000000000000000");
    }

    /// Random boards: every document is well formed and holds one `<path>`
    /// per path item and one `<text>` per line.
    fn documents_hold(count: usize, seed: u64) {
        let mut rng = fixtures::Rng(seed);
        for _ in 0..count {
            let board = crate::scene::tests::random_board(&mut rng);
            let scene = build(&board, &LIGHT, &FixedMeasure);
            let svg = write(
                &scene,
                &Options {
                    background: Some(LIGHT.canvas),
                    padding: 8.0,
                },
            );
            let found = elements(&svg).unwrap_or_else(|e| panic!("{e}\n{svg}"));
            let paths_written = found.iter().filter(|(n, _, _)| n == "path").count();
            assert_eq!(paths_written, paths(&scene));
            let stripped: Vec<String> = lines(&scene).into_iter().map(escape_free).collect();
            assert_eq!(texts(&svg), stripped);
        }
    }

    /// A line as it reads back from the document.
    fn escape_free(line: &str) -> String {
        line.chars()
            .filter(|&c| c == '\t' || !matches!(c, '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}'))
            .collect()
    }

    #[test]
    fn random_boards_write_well_formed_documents() {
        documents_hold(300, 11);
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_random_boards_write_well_formed_documents() {
        documents_hold(20_000, 12);
    }
}
