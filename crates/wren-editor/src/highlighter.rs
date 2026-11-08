//! A Markdown syntax highlighter that colours spans from a [`Palette`].
//!
//! Implements `iced::core::text::highlighter::Highlighter`. Lines are
//! classified by [`wren_markdown::block`], the classifier the rest of Wren
//! uses, so nested lists, quotes, `~~~` and container-indented fences,
//! indented code, and front matter highlight the way they edit. Inline
//! markup is matched by hand on top, with colours from the host's
//! [`Palette`] rather than a fixed third-party theme.
//!
//! Supported constructs:
//!   - Front matter: muted, with faint delimiters.
//!   - Fenced and indented code: muted body, faint fences; fenced blocks get
//!     the widget's code band.
//!   - ATX headings (`#`–`######`): faint markers, bold `heading` H1,
//!     `heading2` H2, and primary H3–H6.
//!   - Block quotes: faint markers, italic `quote` text.
//!   - List markers and open task boxes: accent colour.
//!   - Completed tasks: faint markers and `completed` text, struck through.
//!   - Thematic breaks, setext underlines, and HTML blocks: faint.
//!   - Inline code spans (`` `...` ``): `code` colour on a code chip.
//!   - Bold (`**...**`, `__...__`) and italic (`*...*`, `_..._`): the font's
//!     bold and italic faces, markers included in faint. Emphasis can wrap
//!     links and style link text, but never pairs markers inside a URL.
//!   - Strikethrough (`~~...~~`): faint tildes, `completed` text, struck,
//!     around any emphasis, links, or code inside.
//!   - Links (`[text](url)`, `[text][ref]`, bare `http(s)://` URLs): `link`
//!     text, a `completed` destination, and faint brackets; on a done task
//!     the link keeps its colour under the strike. Destinations and bare
//!     URLs keep the parentheses they balance.
//!   - Inline HTML comments (`<!--...-->`): faint colour.
//!
//! The syntax colours, the emphasis-marker fonts, links, and the code band
//! follow the design/theme-polish branch (commits df7a6c1 and e17c877).
//!
//! The scanner state is checkpointed every [`CHECKPOINT_INTERVAL`] lines so
//! that `change_line` can resync cheaply without replaying the document.
//! Alongside the spans, each highlighted line leaves a [`LineDecor`] for the
//! widget to draw: strike-throughs, the code band, and inline code chips.
//!
//! All byte ranges returned are valid UTF-8 boundaries.

use std::ops::Range;

use iced::advanced::text::highlighter::{self, Format};
use iced::font::{self, Font};
use iced::Color;
use wren_markdown::block::{self, Container, Kind, LineInfo, Scanner};

use crate::Palette;

const CHECKPOINT_INTERVAL: usize = 50;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Highlight produced for a byte range within a line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlight {
    color: Option<Color>,
    font: Option<Font>,
}

impl Highlight {
    /// Convert to the iced `Format` expected by `highlight_with`.
    pub fn to_format(self) -> Format<Font> {
        Format {
            color: self.color,
            font: self.font,
        }
    }

    fn colored(color: Color) -> Self {
        Self {
            color: Some(color),
            font: None,
        }
    }

    fn bold(color: Color, base: Font) -> Self {
        Self {
            color: Some(color),
            font: Some(Font {
                weight: font::Weight::Bold,
                ..base
            }),
        }
    }

    fn italic(color: Color, base: Font) -> Self {
        Self {
            color: Some(color),
            font: Some(Font {
                style: font::Style::Italic,
                ..base
            }),
        }
    }
}

/// Settings for the [`MarkdownHighlighter`]. Any change re-highlights from
/// the first line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub palette: Palette,
    /// The editor font; bold and italic spans are its variants.
    pub font: Font,
    /// The document's [`block::front_matter_end`]. A highlighter sees one
    /// line at a time and cannot look ahead for the closing delimiter, so
    /// the editor recomputes this after every change; a new value restarts
    /// highlighting, since every checkpoint was scanned with the old one.
    pub front_matter_end: Option<usize>,
}

/// What the widget draws for a line beyond its glyph colours.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineDecor {
    /// The line is part of a fenced code block, fences included.
    pub code: bool,
    /// Byte ranges to strike through.
    pub strikes: Vec<Range<usize>>,
    /// Inline code spans, backticks included, to set on a chip.
    pub code_spans: Vec<Range<usize>>,
}

// ---------------------------------------------------------------------------
// Span colours
// ---------------------------------------------------------------------------

/// The palette plus the font whose bold and italic variants style emphasis.
struct Ink {
    text: Color,
    muted: Color,
    faint: Color,
    accent: Color,
    heading: Color,
    heading2: Color,
    link: Color,
    code: Color,
    quote: Color,
    completed: Color,
    font: Font,
}

impl Ink {
    fn new(palette: Palette, font: Font) -> Self {
        let Palette {
            text,
            muted,
            faint,
            accent,
            heading,
            heading2,
            link,
            code,
            quote,
            completed,
        } = palette;
        Self {
            text,
            muted,
            faint,
            accent,
            heading,
            heading2,
            link,
            code,
            quote,
            completed,
            font,
        }
    }
}

// ---------------------------------------------------------------------------
// Highlighter state
// ---------------------------------------------------------------------------

/// The Wren Markdown syntax highlighter.
pub struct MarkdownHighlighter {
    settings: Settings,
    /// `checkpoints[i]` is the scanner at the start of line
    /// `i * CHECKPOINT_INTERVAL`.
    checkpoints: Vec<Scanner>,
    /// The scanner at the start of the next line to highlight.
    scanner: Scanner,
    /// `decor[i]` for every line before the next one to highlight.
    decor: Vec<LineDecor>,
}

impl highlighter::Highlighter for MarkdownHighlighter {
    type Settings = Settings;
    type Highlight = Highlight;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Highlight)>;

    fn new(settings: &Settings) -> Self {
        let scanner = Scanner::with_front_matter(settings.front_matter_end);
        Self {
            settings: *settings,
            checkpoints: vec![scanner.clone()],
            scanner,
            decor: Vec::new(),
        }
    }

    fn update(&mut self, new_settings: &Settings) {
        *self = Self::new(new_settings);
    }

    fn change_line(&mut self, line: usize) {
        let index = (line / CHECKPOINT_INTERVAL).min(self.checkpoints.len() - 1);
        self.checkpoints.truncate(index + 1);
        self.scanner = self.checkpoints[index].clone();
        self.decor.truncate(self.scanner.line_index());
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        let index = self.scanner.line_index();
        let info = self.scanner.next_line(line);
        let containers = self.scanner.containers();
        let continued = self.scanner.continued();
        // A lazy line keeps containers open that it does not belong to.
        let own = if info.lazy {
            &containers[..continued]
        } else {
            containers
        };
        let ink = Ink::new(self.settings.palette, self.settings.font);
        let (spans, decor) = line_highlights(line, index, &info, own, continued, &ink);

        self.decor.truncate(index);
        self.decor.push(decor);
        let next = self.scanner.line_index();
        if next.is_multiple_of(CHECKPOINT_INTERVAL) {
            self.checkpoints.truncate(next / CHECKPOINT_INTERVAL);
            self.checkpoints.push(self.scanner.clone());
        }

        spans.into_iter()
    }

    fn current_line(&self) -> usize {
        self.scanner.line_index()
    }
}

impl MarkdownHighlighter {
    /// The decoration of `line`, if it has been highlighted since the text
    /// above it last changed.
    pub fn decor(&self, line: usize) -> Option<&LineDecor> {
        self.decor.get(line)
    }
}

// ---------------------------------------------------------------------------
// Per-line highlight computation
// ---------------------------------------------------------------------------

/// Sorted, non-overlapping spans; touching spans that look the same merge.
#[derive(Default)]
struct Spans(Vec<(Range<usize>, Highlight)>);

impl Spans {
    fn push(&mut self, range: Range<usize>, highlight: Highlight) {
        if range.is_empty() {
            return;
        }
        debug_assert!(self
            .0
            .last()
            .is_none_or(|(last, _)| last.end <= range.start));
        match self.0.last_mut() {
            Some((last, previous)) if last.end == range.start && *previous == highlight => {
                last.end = range.end;
            }
            _ => self.0.push((range, highlight)),
        }
    }

    /// Spans found in `line[offset..]`, shifted into `line`.
    fn extend_at(&mut self, offset: usize, spans: Vec<(Range<usize>, Highlight)>) {
        for (range, highlight) in spans {
            self.push(range.start + offset..range.end + offset, highlight);
        }
    }
}

/// The spans and decoration of line `index`, classified as `info`. `own`
/// are the containers the line belongs to, the first `continued` of which
/// were open before it.
fn line_highlights(
    line: &str,
    index: usize,
    info: &LineInfo,
    own: &[Container],
    continued: usize,
    ink: &Ink,
) -> (Vec<(Range<usize>, Highlight)>, LineDecor) {
    let mut spans = Spans::default();
    let mut decor = LineDecor::default();
    let body = info.prefix_end..line.len();

    if info.kind == Kind::FrontMatter {
        let delimiter = matches!(line[body.clone()].trim_end(), "---" | "...");
        let color = if delimiter { ink.faint } else { ink.muted };
        spans.push(body, Highlight::colored(color));
        return (spans.0, decor);
    }

    let task = info.item.and_then(|item| item.task);
    let done = task.is_some_and(|task| task.done);
    for (segment, container) in block::segments(line, index, own, continued)
        .into_iter()
        .zip(own)
    {
        let Some(marker) = segment.marker else {
            continue;
        };
        let color = match container {
            Container::Quote { .. } => ink.faint,
            Container::Item(_) if done => ink.faint,
            Container::Item(_) => ink.accent,
        };
        spans.push(marker..segment.end, Highlight::colored(color));
    }
    if let Some(task) = task {
        let color = if done { ink.faint } else { ink.accent };
        spans.push(task.start..info.text_start, Highlight::colored(color));
    }

    match &info.kind {
        Kind::Blank | Kind::FrontMatter => {}
        Kind::Heading { level, text } => {
            let faint = Highlight::colored(ink.faint);
            if text.is_empty() {
                spans.push(body, faint);
            } else {
                let color = match level {
                    1 => ink.heading,
                    2 => ink.heading2,
                    _ => ink.text,
                };
                spans.push(info.prefix_end..text.start, faint);
                spans.push(text.clone(), Highlight::bold(color, ink.font));
                spans.push(text.end..line.len(), faint);
            }
        }
        Kind::SetextUnderline { .. } | Kind::ThematicBreak | Kind::Html => {
            spans.push(body, Highlight::colored(ink.faint));
        }
        Kind::FenceOpen { .. } | Kind::FenceClose => {
            spans.push(body, Highlight::colored(ink.faint));
            decor.code = true;
        }
        Kind::FenceBody => {
            spans.push(body, Highlight::colored(ink.muted));
            decor.code = true;
        }
        Kind::IndentedCode => spans.push(body, Highlight::colored(ink.muted)),
        Kind::Prose if done => {
            // Done text recedes, but its links keep their colour.
            let text = info.text_start..info.text_start + line[info.text_start..].trim_end().len();
            let completed = Highlight::colored(ink.completed);
            let mut covered = text.start;
            for (range, highlight) in scan_links(&line[text.clone()], ink) {
                let range = range.start + text.start..range.end + text.start;
                spans.push(covered..range.start, completed);
                covered = range.end;
                spans.push(range, highlight);
            }
            spans.push(covered..text.end, completed);
            spans.push(text.end..line.len(), Highlight::colored(ink.faint));
            decor.strikes.push(text);
        }
        Kind::Prose => {
            let start = info.text_start;
            let inline = scan_inline(&line[start..], ink);
            let shift = |r: &Range<usize>| r.start + start..r.end + start;
            decor.strikes.extend(inline.strikes.iter().map(shift));
            decor.code_spans.extend(inline.code.iter().map(shift));
            if info.quote_depth == 0 {
                spans.extend_at(start, inline.spans);
            } else {
                // Quoted text between the inline spans is italic.
                let quote = Highlight::italic(ink.quote, ink.font);
                let mut covered = start;
                for (range, highlight) in inline.spans {
                    let range = range.start + start..range.end + start;
                    spans.push(covered..range.start, quote);
                    covered = range.end;
                    spans.push(range, highlight);
                }
                spans.push(covered..line.len(), quote);
            }
        }
    }
    (spans.0, decor)
}

// ---------------------------------------------------------------------------
// Inline element scanner
// ---------------------------------------------------------------------------

/// The inline Markdown of a line: sorted, non-overlapping spans, and the
/// struck ranges, all relative to the scanned text.
struct Inline {
    spans: Vec<(Range<usize>, Highlight)>,
    strikes: Vec<Range<usize>>,
    /// Inline code spans, backticks included.
    code: Vec<Range<usize>>,
}

/// Scans inline Markdown. Code spans and HTML comments come first and hold
/// nothing else. Links and bare URLs come next, so emphasis never pairs the
/// `_` or `*` of a URL. Strikethrough and emphasis are matched inside each
/// link's text, and then across the line, where they can wrap whole links.
fn scan_inline(line: &str, ink: &Ink) -> Inline {
    let mut cells = Cells::new(line);
    let code = cells.code_spans(ink);
    cells.comments(ink);
    let texts = cells.links(ink);
    cells.bare_urls(ink);
    let mut strikes = Vec::new();
    for text in &texts {
        strikes.extend(cells.delimited(text.clone(), ink));
    }
    for text in texts {
        cells.set(text, |cell| cell.locked = true);
    }
    strikes.extend(cells.delimited(0..line.len(), ink));
    strikes.sort_by_key(|range| range.start);
    Inline {
        spans: cells.spans(ink.font),
        strikes,
        code,
    }
}

/// Only the links of a line, for text that is otherwise one colour.
fn scan_links(line: &str, ink: &Ink) -> Vec<(Range<usize>, Highlight)> {
    let mut cells = Cells::new(line);
    cells.links(ink);
    cells.bare_urls(ink);
    cells.spans(ink.font)
}

/// The style of one byte of scanned text.
#[derive(Clone, Copy, Default)]
struct Cell {
    color: Option<Color>,
    bold: bool,
    italic: bool,
    /// Code and comments keep their face inside emphasis.
    fixed_face: bool,
    /// No later delimiter may match here.
    locked: bool,
}

/// A line's bytes as each kind of construct layers its style onto them. A
/// construct found later can wrap one found earlier but never pair its
/// delimiters inside it.
struct Cells<'a> {
    line: &'a str,
    cells: Vec<Cell>,
}

impl<'a> Cells<'a> {
    fn new(line: &'a str) -> Self {
        Self {
            line,
            cells: vec![Cell::default(); line.len()],
        }
    }

    fn set(&mut self, range: Range<usize>, style: impl Fn(&mut Cell)) {
        self.cells[range].iter_mut().for_each(style);
    }

    /// Faint, locked syntax: brackets, tildes, and the like.
    fn mark(&mut self, range: Range<usize>, ink: &Ink) {
        self.set(range, |cell| {
            cell.color = Some(ink.faint);
            cell.locked = true;
        });
    }

    fn is_free(&self, range: Range<usize>) -> bool {
        self.cells[range].iter().all(|cell| !cell.locked)
    }

    /// The first `marker` in `from..end` that lies on free bytes.
    fn next_free(&self, marker: &str, from: usize, end: usize) -> Option<usize> {
        let mut search = from;
        while let Some(at) = self.line[search..end].find(marker).map(|at| search + at) {
            if self.is_free(at..at + marker.len()) {
                return Some(at);
            }
            search = at + 1;
        }
        None
    }

    /// Backtick code spans, closed by a run of as many backticks as opened
    /// them. Returns their ranges, backticks included.
    fn code_spans(&mut self, ink: &Ink) -> Vec<Range<usize>> {
        let bytes = self.line.as_bytes();
        let run = |from: usize| bytes[from..].iter().take_while(|&&b| b == b'`').count();
        let mut found = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'`' {
                i += 1;
                continue;
            }
            let ticks = run(i);
            let mut k = i + ticks;
            let mut end = None;
            while k < bytes.len() {
                if bytes[k] == b'`' {
                    let length = run(k);
                    if length == ticks {
                        end = Some(k + length);
                        break;
                    }
                    k += length;
                } else {
                    k += 1;
                }
            }
            match end {
                Some(end) => {
                    self.set(i..end, |cell| {
                        cell.color = Some(ink.code);
                        cell.fixed_face = true;
                        cell.locked = true;
                    });
                    found.push(i..end);
                    i = end;
                }
                None => i += ticks,
            }
        }
        found
    }

    /// `<!-- ... -->`, faint.
    fn comments(&mut self, ink: &Ink) {
        let mut search = 0;
        while let Some(open) = self.line[search..].find("<!--").map(|at| search + at) {
            let Some(end) = self.line[open + 4..]
                .find("-->")
                .map(|at| open + 4 + at + 3)
            else {
                break;
            };
            if self.is_free(open..end) {
                self.set(open..end, |cell| {
                    cell.color = Some(ink.faint);
                    cell.fixed_face = true;
                    cell.locked = true;
                });
                search = end;
            } else {
                search = open + 1;
            }
        }
    }

    /// `[text](destination)` and `[text][ref]`: `link` text between faint
    /// brackets, and a `completed` destination. Brackets pair by depth, and
    /// a destination ends at the `)` that balances its parentheses, as in
    /// CommonMark. Returns the text ranges.
    fn links(&mut self, ink: &Ink) -> Vec<Range<usize>> {
        let bytes = self.line.as_bytes();
        let mut open = Vec::new();
        let mut pairs = Vec::new();
        for (i, &byte) in bytes.iter().enumerate() {
            match byte {
                _ if self.cells[i].locked => {}
                b'[' => open.push(i),
                b']' => pairs.extend(open.pop().map(|start| (start, i))),
                _ => {}
            }
        }
        pairs.sort_unstable();
        let mut texts = Vec::new();
        let mut covered = 0;
        for (start, close) in pairs {
            if start < covered {
                continue;
            }
            let Some(end) = self.link_tail(close + 1) else {
                continue;
            };
            covered = end;
            self.mark(start..start + 1, ink);
            let text = start + 1..close;
            self.set(text.clone(), |cell| {
                cell.color.get_or_insert(ink.link);
            });
            let destination = close + 2..end - 1;
            if bytes[close + 1] == b'(' && !destination.is_empty() {
                self.mark(close..destination.start, ink);
                self.set(destination.clone(), |cell| {
                    cell.color = Some(ink.completed);
                    cell.locked = true;
                });
                self.mark(destination.end..end, ink);
            } else {
                self.mark(close..end, ink);
            }
            texts.push(text);
        }
        texts
    }

    /// The end of the `(destination)` or `[ref]` that starts at `start`.
    fn link_tail(&self, start: usize) -> Option<usize> {
        let bytes = self.line.as_bytes();
        let inline = match bytes.get(start)? {
            b'(' => true,
            b'[' => false,
            _ => return None,
        };
        let mut depth = 0_usize;
        for (i, &byte) in bytes.iter().enumerate().skip(start) {
            if self.cells[i].locked {
                return None;
            }
            match byte {
                b'(' if inline => depth += 1,
                b')' if inline => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                b']' if !inline => return Some(i + 1),
                _ => {}
            }
        }
        None
    }

    /// Bare `http://` and `https://` URLs in the link colour. A URL ends at
    /// whitespace or other markup, and keeps [`trim_url`]'s punctuation out.
    fn bare_urls(&mut self, ink: &Ink) {
        let line = self.line;
        let mut search = 0;
        while let Some((start, scheme)) = ["https://", "http://"]
            .iter()
            .filter_map(|scheme| Some((search + line[search..].find(scheme)?, scheme.len())))
            .min()
        {
            let rest = &line[start..];
            let length = rest
                .char_indices()
                .find(|&(i, ch)| ch.is_whitespace() || self.cells[start + i].locked)
                .map_or(rest.len(), |(i, _)| i);
            let url = trim_url(&rest[..length]);
            if url.len() > scheme {
                self.set(start..start + url.len(), |cell| {
                    cell.color = Some(ink.link);
                    cell.locked = true;
                });
            }
            search = start + length.max(1);
        }
    }

    /// Strikethrough, then emphasis from the strongest marker down, inside
    /// `range`. Returns the struck ranges.
    fn delimited(&mut self, range: Range<usize>, ink: &Ink) -> Vec<Range<usize>> {
        let strikes = self.strikethrough(range.clone(), ink);
        for (marker, bold, italic) in [
            ("***", true, true),
            ("___", true, true),
            ("**", true, false),
            ("__", true, false),
            ("*", false, true),
            ("_", false, true),
        ] {
            self.emphasis(range.clone(), marker, bold, italic, ink);
        }
        strikes
    }

    /// `~~struck~~`: faint tildes around `completed` text, keeping the
    /// colour of links and code inside. Returns the struck text ranges.
    fn strikethrough(&mut self, range: Range<usize>, ink: &Ink) -> Vec<Range<usize>> {
        let mut strikes = Vec::new();
        let mut search = range.start;
        while let Some(open) = self.next_free("~~", search, range.end) {
            let Some(close) = self.next_free("~~", open + 2, range.end) else {
                break;
            };
            let inner = open + 2..close;
            self.set(inner.clone(), |cell| {
                cell.color.get_or_insert(ink.completed);
            });
            self.mark(open..inner.start, ink);
            self.mark(close..close + 2, ink);
            if !inner.is_empty() {
                strikes.push(inner);
            }
            search = close + 2;
        }
        strikes
    }

    /// Emphasis delimited by `marker`, which then holds no other emphasis.
    fn emphasis(&mut self, range: Range<usize>, marker: &str, bold: bool, italic: bool, ink: &Ink) {
        let mut search = range.start;
        while let Some(open) = self.next_free(marker, search, range.end) {
            let content = open + marker.len();
            let Some(close) = self.next_free(marker, content, range.end) else {
                break;
            };
            let end = close + marker.len();
            let face = |cell: &mut Cell| {
                cell.bold |= bold;
                cell.italic |= italic;
            };
            self.set(content..close, |cell| {
                if !cell.fixed_face {
                    face(cell);
                }
                cell.color.get_or_insert(ink.text);
            });
            // cosmic-text shapes a word in the font of its first character,
            // so a regular-weight `**` would leave `**Alice` regular. The
            // markers carry the span's font and differ only in colour.
            for markers in [open..content, close..end] {
                self.set(markers, |cell| {
                    cell.color = Some(ink.faint);
                    face(cell);
                });
            }
            self.set(open..end, |cell| cell.locked = true);
            search = end;
        }
    }

    /// The styled bytes as sorted spans, touching ones that look the same
    /// merged.
    fn spans(&self, base: Font) -> Vec<(Range<usize>, Highlight)> {
        let mut spans = Spans::default();
        for (start, character) in self.line.char_indices() {
            let cell = self.cells[start];
            let font = (cell.bold || cell.italic).then_some(Font {
                weight: if cell.bold {
                    font::Weight::Bold
                } else {
                    base.weight
                },
                style: if cell.italic {
                    font::Style::Italic
                } else {
                    base.style
                },
                ..base
            });
            if cell.color.is_some() || font.is_some() {
                let highlight = Highlight {
                    color: cell.color,
                    font,
                };
                spans.push(start..start + character.len_utf8(), highlight);
            }
        }
        spans.0
    }
}

/// A bare URL without its trailing sentence punctuation, as in GFM's
/// autolinks: a final `)` stays only when it closes a `(` in the URL.
fn trim_url(mut url: &str) -> &str {
    while let Some(last) = url.chars().next_back() {
        let trailing = match last {
            ')' => url.matches(')').count() > url.matches('(').count(),
            '.' | ',' | ';' | ':' | '!' | '?' | '*' | '_' | '~' | '>' | '"' | '\'' => true,
            _ => false,
        };
        if !trailing {
            break;
        }
        url = &url[..url.len() - 1];
    }
    url
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use iced::advanced::text::Highlighter as _;

    fn dark_palette() -> Ink {
        Ink::new(fixtures::DARK, fixtures::FONT)
    }

    fn light_palette() -> Ink {
        Ink::new(fixtures::LIGHT, fixtures::FONT)
    }

    fn settings(palette: Palette, front_matter_end: Option<usize>) -> Settings {
        Settings {
            palette,
            font: fixtures::FONT,
            front_matter_end,
        }
    }

    type LineSpans = Vec<(Range<usize>, Highlight)>;

    /// Highlights every line of `text` the way the editor does, front
    /// matter included, returning each line's spans and decoration.
    fn highlight_text(palette: Palette, text: &str) -> Vec<(LineSpans, LineDecor)> {
        let lines: Vec<&str> = text.split('\n').collect();
        let end = block::front_matter_end(&lines[..]);
        let mut h = MarkdownHighlighter::new(&settings(palette, end));
        lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let spans = h.highlight_line(line).collect();
                (spans, h.decor(i).cloned().unwrap())
            })
            .collect()
    }

    fn spans_of(text: &str) -> Vec<LineSpans> {
        highlight_text(fixtures::DARK, text)
            .into_iter()
            .map(|(spans, _)| spans)
            .collect()
    }

    fn highlights_for(line: &str) -> LineSpans {
        spans_of(line).remove(0)
    }

    /// `(text, color)` for each span of `line`.
    fn colored<'a>(
        line: &'a str,
        spans: &[(Range<usize>, Highlight)],
    ) -> Vec<(&'a str, Option<Color>)> {
        spans
            .iter()
            .map(|(range, highlight)| (&line[range.clone()], highlight.color))
            .collect()
    }

    fn span_color_at(spans: &[(Range<usize>, Highlight)], byte_start: usize) -> Option<Color> {
        spans
            .iter()
            .find(|(r, _)| r.start == byte_start)
            .and_then(|(_, h)| h.color)
    }

    // ── Headings ─────────────────────────────────────────────────────────────

    #[test]
    fn heading_markers_are_faint_and_h1_h2_use_their_heading_colors_in_both_palettes() {
        for palette in [fixtures::DARK, fixtures::LIGHT] {
            let p = Ink::new(palette, fixtures::FONT);
            for (level, prefix) in ["# ", "## ", "### ", "#### ", "##### ", "###### "]
                .into_iter()
                .enumerate()
            {
                let line = format!("{prefix}Heading");
                let spans = highlight_text(palette, &line).remove(0).0;
                assert_eq!(spans.len(), 2);
                assert_eq!(spans[0].0, 0..prefix.len());
                assert_eq!(spans[0].1.color, Some(p.faint));
                assert_eq!(spans[0].1.font, None);
                assert_eq!(spans[1].0, prefix.len()..line.len());
                assert_eq!(
                    spans[1].1.color,
                    Some(match level {
                        0 => p.heading,
                        1 => p.heading2,
                        _ => p.text,
                    })
                );
                assert!(spans[1]
                    .1
                    .font
                    .is_some_and(|font| font.weight == font::Weight::Bold));
            }
        }
    }

    #[test]
    fn closing_hashes_and_quoted_headings_are_marked_up() {
        let p = dark_palette();
        let line = "> ## Title ##";
        let spans = highlights_for(line);
        assert_eq!(
            colored(line, &spans),
            [
                ("> ## ", Some(p.faint)),
                ("Title", Some(p.heading2)),
                (" ##", Some(p.faint)),
            ]
        );
    }

    #[test]
    fn hash_without_space_is_not_a_heading() {
        let spans = highlights_for("#notaheading");
        // Should not produce a full-line accent span
        for (r, _) in &spans {
            assert_ne!(r.start, 0, "unexpected span starting at 0");
        }
    }

    // ── Lists and tasks ──────────────────────────────────────────────────────

    #[test]
    fn list_and_open_task_markers_use_accent_and_done_tasks_recede() {
        for palette in [fixtures::DARK, fixtures::LIGHT] {
            let p = Ink::new(palette, fixtures::FONT);
            for (line, end) in [
                ("- words", 2),
                ("* words", 2),
                ("+ words", 2),
                ("12. words", 4),
                ("3) words", 3),
                ("- [ ] words", 6),
            ] {
                let spans = highlight_text(palette, line).remove(0).0;
                assert_eq!(spans[0].0, 0..end, "{line}");
                assert_eq!(spans[0].1.color, Some(p.accent));
            }
            for (line, marker, text) in [
                ("- [x] completed", "- [x] ", "completed"),
                ("  - [X] **completed**  ", "- [X] ", "**completed**"),
            ] {
                let spans = highlight_text(palette, line).remove(0).0;
                let colored = colored(line, &spans);
                assert_eq!(colored[0], (marker, Some(p.faint)), "{line}");
                assert_eq!(colored[1], (text, Some(p.completed)), "{line}");
                assert!(colored[2..]
                    .iter()
                    .all(|(_, color)| *color == Some(p.faint)));
                assert!(spans.iter().all(|(_, highlight)| highlight.font.is_none()));
            }
        }
    }

    #[test]
    fn nested_list_markers_are_found_at_any_depth() {
        let p = dark_palette();
        let text = "- a\n  - b\n    - [ ] c\n      1. d **e**\n         continued";
        let spans = spans_of(text);
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(colored(lines[1], &spans[1]), [("- ", Some(p.accent))]);
        assert_eq!(colored(lines[2], &spans[2]), [("- [ ] ", Some(p.accent))]);
        assert_eq!(colored(lines[3], &spans[3])[0], ("1. ", Some(p.accent)));
        assert!(spans[3]
            .iter()
            .any(|(range, highlight)| &lines[3][range.clone()] == "e"
                && highlight
                    .font
                    .is_some_and(|font| font.weight == font::Weight::Bold)));
        assert!(spans[4].is_empty(), "{:?}", spans[4]);
    }

    #[test]
    fn completed_tasks_are_struck_at_any_depth_but_never_in_code() {
        let text = "- [x] done  \n  * [X] **café**\n- [ ] open\n- [x] \n```md\n- [x] code\n```\n1. [x] last\n- a\n  - [x] nested\n> - [x] quoted\n~~~\n- [x] tilde code\n~~~";
        let strikes: Vec<_> = highlight_text(fixtures::DARK, text)
            .into_iter()
            .map(|(_, decor)| decor.strikes)
            .collect();
        let expected: Vec<Vec<Range<usize>>> = vec![
            vec![6..10],
            vec![8..17],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![7..11],
            vec![],
            vec![8..14],
            vec![8..14],
            vec![],
            vec![],
            vec![],
        ];
        assert_eq!(strikes, expected);
    }

    #[test]
    fn nested_markup_never_overlaps_code_or_splits_utf8() {
        for line in [
            "**bold `code` text**",
            "***both***",
            "__粗体__ and *café*",
            "- [ ] **café**",
            "<!-- **ignored** -->",
            "**bold** and `code` and ~~deleted~~",
            "# 🐦 heading",
            "> - [x] **done** [link](u)",
        ] {
            let spans = highlights_for(line);
            let mut previous = 0;
            for (range, _) in &spans {
                assert!(range.start < range.end, "empty span in {line:?}");
                assert!(
                    range.start >= previous,
                    "overlapping spans in {line:?}: {spans:?}"
                );
                assert!(line.is_char_boundary(range.start) && line.is_char_boundary(range.end));
                previous = range.end;
            }
        }
        let spans = highlights_for("**bold `code` text**");
        assert!(spans
            .iter()
            .any(|(range, highlight)| *range == (7..13)
                && highlight.color == Some(dark_palette().code)));
        let bold: Vec<_> = spans
            .iter()
            .filter(|(_, highlight)| {
                highlight
                    .font
                    .is_some_and(|font| font.weight == font::Weight::Bold)
            })
            .map(|(range, _)| range.clone())
            .collect();
        assert_eq!(bold, [0..2, 2..7, 13..18, 18..20]);
        assert_eq!(span_color_at(&spans, 0), Some(dark_palette().faint));
        assert_eq!(span_color_at(&spans, 18), Some(dark_palette().faint));
    }

    // ── Code ─────────────────────────────────────────────────────────────────

    #[test]
    fn fences_of_either_kind_are_faint_and_their_bodies_muted_and_banded() {
        let p = dark_palette();
        for text in ["```rust\nlet x = 42;\n```", "~~~\nlet x = 42;\n~~~"] {
            let lines = highlight_text(fixtures::DARK, &format!("{text}\nafter *it*"));
            let fence_len = text.find('\n').unwrap();
            assert_eq!(lines[0].0, [(0..fence_len, Highlight::colored(p.faint))]);
            assert_eq!(lines[1].0, [(0..11, Highlight::colored(p.muted))]);
            assert_eq!(lines[2].0, [(0..3, Highlight::colored(p.faint))]);
            let code: Vec<_> = lines.iter().map(|(_, decor)| decor.code).collect();
            assert_eq!(code, [true, true, true, false], "{text}");
            assert!(lines[3].0.iter().all(|(_, h)| h.color != Some(p.muted)));
        }
        // A tilde fence is not closed by backticks, nor a long fence by a
        // short one.
        let lines = spans_of("~~~~\n```\n~~~\nstill code\n~~~~\nout");
        assert_eq!(lines[3], [(0..10, Highlight::colored(p.muted))]);
        assert!(lines[5].is_empty());
    }

    #[test]
    fn fences_inside_quotes_and_list_items_keep_their_container_markup() {
        let p = dark_palette();
        let text = "> ```\n> code\n> ```\n- item\n\n  ```\n  code\n  ```\nafter";
        let lines: Vec<&str> = text.split('\n').collect();
        let highlighted = highlight_text(fixtures::DARK, text);
        assert_eq!(
            colored(lines[1], &highlighted[1].0),
            [("> ", Some(p.faint)), ("code", Some(p.muted))]
        );
        assert_eq!(
            colored(lines[6], &highlighted[6].0),
            [("code", Some(p.muted))]
        );
        let code: Vec<_> = highlighted.iter().map(|(_, decor)| decor.code).collect();
        assert_eq!(
            code,
            [true, true, true, false, false, true, true, true, false]
        );
    }

    #[test]
    fn indented_code_needs_a_blank_line_before_it() {
        let p = dark_palette();
        let code = "    code **not bold**";
        let lines = spans_of(&format!("para\n    lazy\n\n{code}"));
        assert!(lines[1].is_empty(), "{:?}", lines[1]);
        assert_eq!(lines[3], [(0..code.len(), Highlight::colored(p.muted))]);
    }

    // ── Front matter ─────────────────────────────────────────────────────────

    #[test]
    fn front_matter_is_muted_between_faint_delimiters_not_a_heading() {
        let p = dark_palette();
        let lines = spans_of("---\ntitle: Notes\n# comment\n---\n# Heading");
        assert_eq!(lines[0], [(0..3, Highlight::colored(p.faint))]);
        assert_eq!(lines[1], [(0..12, Highlight::colored(p.muted))]);
        assert_eq!(lines[2], [(0..9, Highlight::colored(p.muted))]);
        assert_eq!(lines[3], [(0..3, Highlight::colored(p.faint))]);
        assert_eq!(lines[4][1].1.color, Some(p.heading));
        // Unclosed, line 0 is a thematic break and the rest is Markdown.
        let lines = spans_of("---\ntitle: Notes\n# Heading");
        assert!(lines[1].is_empty());
        assert_eq!(lines[2][1].1.color, Some(p.heading));
    }

    #[test]
    fn a_new_front_matter_end_restarts_highlighting_from_the_first_line() {
        let p = dark_palette();
        let mut h = MarkdownHighlighter::new(&settings(fixtures::DARK, None));
        for line in ["---", "# title", "---"] {
            h.highlight_line(line).for_each(drop);
        }
        assert_eq!(h.current_line(), 3);
        h.update(&settings(fixtures::DARK, Some(2)));
        assert_eq!(h.current_line(), 0);
        assert!(h.decor(0).is_none());
        h.highlight_line("---").for_each(drop);
        let spans: Vec<_> = h.highlight_line("# title").collect();
        assert_eq!(spans, [(0..7, Highlight::colored(p.muted))]);
    }

    // ── Blockquote ───────────────────────────────────────────────────────────

    #[test]
    fn blockquote_markers_are_faint_and_their_text_is_italic_quote() {
        let p = dark_palette();
        let line = "> quoted **bold** text";
        let spans = highlights_for(line);
        assert_eq!(span_color_at(&spans, 0), Some(p.faint));
        let quoted = spans.iter().find(|(range, _)| range.start == 2).unwrap();
        assert_eq!(&line[quoted.0.clone()], "quoted ");
        assert_eq!(quoted.1.color, Some(p.quote));
        assert!(quoted
            .1
            .font
            .is_some_and(|font| font.style == font::Style::Italic));
        let bold = spans
            .iter()
            .find(|(range, _)| &line[range.clone()] == "bold")
            .unwrap();
        assert!(bold
            .1
            .font
            .is_some_and(|font| font.weight == font::Weight::Bold));
        assert_eq!(spans.last().unwrap().0.end, line.len());

        let nested = "> > deep";
        let spans = highlights_for(nested);
        assert_eq!(
            colored(nested, &spans),
            [("> > ", Some(p.faint)), ("deep", Some(p.quote))]
        );
        // A lazy line continues the quoted paragraph.
        let lazy = spans_of("> quote\nlazy");
        assert_eq!(lazy[1], [(0..4, Highlight::italic(p.quote, p.font))]);
    }

    // ── Inline code ──────────────────────────────────────────────────────────

    #[test]
    fn inline_code_span_identified() {
        let spans = highlights_for("Call `foo()` and return.");
        // The backtick span starts at "Call ".len() = 5
        assert!(spans.iter().any(|(r, _)| r.start == 5 && r.end == 12));
    }

    #[test]
    fn double_backtick_code_span() {
        let spans = highlights_for("See ``code with `backtick` inside``.");
        let found = spans.iter().any(|(r, _)| r.start == 4);
        assert!(found, "double-backtick span not found in {spans:?}");
    }

    #[test]
    fn unclosed_backtick_produces_no_span() {
        let spans = highlights_for("unclosed `code");
        // No span starting at the backtick position
        assert!(!spans.iter().any(|(r, _)| r.start == 9));
    }

    // ── Bold and italic ───────────────────────────────────────────────────────

    #[test]
    fn bold_text_is_strong_while_its_markers_are_faint_and_bold() {
        for palette in [fixtures::DARK, fixtures::LIGHT] {
            let p = Ink::new(palette, fixtures::FONT);
            let spans = highlight_text(palette, "This is **bold** text.")
                .remove(0)
                .0;
            let marker = spans.iter().find(|(range, _)| *range == (8..10)).unwrap();
            assert_eq!(marker.1.color, Some(p.faint));
            assert!(marker
                .1
                .font
                .is_some_and(|font| font.weight == font::Weight::Bold));
            let inner = spans.iter().find(|(range, _)| *range == (10..14)).unwrap();
            assert_eq!(inner.1.color, Some(p.text));
            assert!(inner
                .1
                .font
                .is_some_and(|font| font.weight == font::Weight::Bold));
            assert_eq!(span_color_at(&spans, 14), Some(p.faint));
        }
    }

    #[test]
    fn italic_text_and_its_faint_markers_share_the_italic_face() {
        let spans = highlights_for("This is *italic* text.");
        let marker = spans.iter().find(|(range, _)| *range == (8..9)).unwrap();
        assert!(marker
            .1
            .font
            .is_some_and(|font| font.style == font::Style::Italic));
        assert_eq!(marker.1.color, Some(dark_palette().faint));
        let inner = spans.iter().find(|(range, _)| *range == (9..15)).unwrap();
        assert!(inner
            .1
            .font
            .is_some_and(|font| font.style == font::Style::Italic));
        assert_eq!(span_color_at(&spans, 15), Some(dark_palette().faint));
    }

    #[test]
    fn emphasis_markers_share_the_span_font_so_whole_words_shape_styled() {
        // cosmic-text shapes a word in its first glyph's font, so a regular
        // `**` once left `**Alice Chen**` bold only on its last word.
        for (line, weight, style) in [
            ("**Alice Chen**", font::Weight::Bold, font::Style::Normal),
            ("*Bob*", font::Weight::Normal, font::Style::Italic),
            ("***hard***", font::Weight::Bold, font::Style::Italic),
        ] {
            let spans = highlights_for(line);
            assert_eq!(spans.len(), 3, "{line}");
            for (_, highlight) in &spans {
                let font = highlight.font.expect("styled span");
                assert_eq!((font.weight, font.style), (weight, style), "{line}");
            }
            assert_eq!(spans[0].1.color, Some(dark_palette().faint));
            assert_eq!(spans[2].1.color, Some(dark_palette().faint));
        }
    }

    #[test]
    fn bold_takes_priority_over_italic() {
        // `**` should be matched first, so `*italic inside bold*` inside `** **`
        // doesn't create a spurious italic span.
        let line = "**bold and *nested***";
        let spans = highlights_for(line);
        let bold_count = spans
            .iter()
            .filter(|(_, h)| h.font.is_some_and(|f| f.weight == font::Weight::Bold))
            .count();
        assert!(
            bold_count >= 1,
            "expected at least one bold span in '{line}'"
        );
    }

    #[test]
    fn inline_code_prevents_bold_inside_it() {
        let line = "code: `**not bold**` text";
        let spans = highlights_for(line);
        // The region inside backticks must not have a separate bold span
        let bold_inside_code = spans.iter().any(|(r, h)| {
            r.start > 6 && r.end <= 20 && h.font.is_some_and(|f| f.weight == font::Weight::Bold)
        });
        assert!(
            !bold_inside_code,
            "bold span found inside code span: {spans:?}"
        );
    }

    // ── Strikethrough ─────────────────────────────────────────────────────────

    #[test]
    fn strikethrough_text_is_completed_between_faint_tildes_and_struck() {
        let p = dark_palette();
        let line = "This is ~~deleted~~ text.";
        let (spans, decor) = highlight_text(fixtures::DARK, line).remove(0);
        assert_eq!(
            colored(line, &spans),
            [
                ("~~", Some(p.faint)),
                ("deleted", Some(p.completed)),
                ("~~", Some(p.faint)),
            ]
        );
        assert_eq!(decor.strikes, vec![(10..17)]);
        let item = "- a ~~b~~";
        let (_, decor) = highlight_text(fixtures::DARK, item).remove(0);
        assert_eq!(decor.strikes, vec![(6..7)]);
    }

    // ── Links ─────────────────────────────────────────────────────────────────

    #[test]
    fn inline_link_text_uses_the_link_color_and_its_destination_recedes() {
        let p = dark_palette();
        let line = "See [the docs](https://example.com) for more.";
        assert_eq!(
            colored(line, &highlights_for(line)),
            [
                ("[", Some(p.faint)),
                ("the docs", Some(p.link)),
                ("](", Some(p.faint)),
                ("https://example.com", Some(p.completed)),
                (")", Some(p.faint)),
            ]
        );
    }

    #[test]
    fn links_on_a_done_task_keep_their_colour_under_the_strike() {
        let p = dark_palette();
        let line = "- [x] read [the docs](u) and https://x.io/a now";
        let (spans, decor) = highlight_text(fixtures::DARK, line).remove(0);
        assert_eq!(
            colored(line, &spans),
            [
                ("- [x] ", Some(p.faint)),
                ("read ", Some(p.completed)),
                ("[", Some(p.faint)),
                ("the docs", Some(p.link)),
                ("](", Some(p.faint)),
                ("u", Some(p.completed)),
                (")", Some(p.faint)),
                (" and ", Some(p.completed)),
                ("https://x.io/a", Some(p.link)),
                (" now", Some(p.completed)),
            ]
        );
        assert_eq!(decor.strikes, vec![(6..line.len())]);
    }

    #[test]
    fn inline_code_spans_are_set_on_chips() {
        let line = "> run `cargo test` or **``a ` b``**";
        let (_, decor) = highlight_text(fixtures::DARK, line).remove(0);
        let chips: Vec<_> = decor.code_spans.iter().map(|r| &line[r.clone()]).collect();
        assert_eq!(chips, ["`cargo test`", "``a ` b``"]);
        let (_, decor) = highlight_text(fixtures::DARK, "- [x] `done`").remove(0);
        assert!(decor.code_spans.is_empty());
    }

    #[test]
    fn reference_link_text_uses_the_link_color() {
        let p = dark_palette();
        let line = "Read [the guide][guide-ref] now.";
        assert_eq!(
            colored(line, &highlights_for(line)),
            [
                ("[", Some(p.faint)),
                ("the guide", Some(p.link)),
                ("][guide-ref]", Some(p.faint)),
            ]
        );
    }

    #[test]
    fn bare_urls_are_links_without_trailing_punctuation() {
        let p = dark_palette();
        for (line, url) in [
            (
                "See https://example.com/a?b=1.",
                "https://example.com/a?b=1",
            ),
            ("(http://wren.dev), then", "http://wren.dev"),
            ("https://x.io", "https://x.io"),
        ] {
            let spans = highlights_for(line);
            let link: Vec<_> = spans
                .iter()
                .filter(|(_, highlight)| highlight.color == Some(p.link))
                .map(|(range, _)| &line[range.clone()])
                .collect();
            assert_eq!(link, [url], "{line}");
        }
        assert!(highlights_for("https:// alone").is_empty());
    }

    /// `(text, color, bold, italic)` for each span of `line`.
    fn styled<'a>(
        line: &'a str,
        spans: &[(Range<usize>, Highlight)],
    ) -> Vec<(&'a str, Option<Color>, bool, bool)> {
        spans
            .iter()
            .map(|(range, highlight)| {
                let font = highlight.font.unwrap_or(fixtures::FONT);
                (
                    &line[range.clone()],
                    highlight.color,
                    font.weight == font::Weight::Bold,
                    font.style == font::Style::Italic,
                )
            })
            .collect()
    }

    #[test]
    fn emphasis_never_pairs_markers_inside_a_url() {
        let p = dark_palette();
        for (line, url) in [
            (
                "see https://example.com/a_b_c/d ok",
                "https://example.com/a_b_c/d",
            ),
            (
                "go to https://example.com/x*y*z now",
                "https://example.com/x*y*z",
            ),
        ] {
            assert_eq!(
                styled(line, &highlights_for(line)),
                [(url, Some(p.link), false, false)],
                "{line}"
            );
        }
        let line = "[x](https://example.com/a_b_c)";
        assert_eq!(
            styled(line, &highlights_for(line)),
            [
                ("[", Some(p.faint), false, false),
                ("x", Some(p.link), false, false),
                ("](", Some(p.faint), false, false),
                ("https://example.com/a_b_c", Some(p.completed), false, false),
                (")", Some(p.faint), false, false),
            ]
        );
        // Emphasis around a URL still pairs, and the URL leaves its marker.
        let line = "_see https://x.io/a_b_";
        assert_eq!(
            styled(line, &highlights_for(line)),
            [
                ("_", Some(p.faint), false, true),
                ("see ", Some(p.text), false, true),
                ("https://x.io/a_b", Some(p.link), false, true),
                ("_", Some(p.faint), false, true),
            ]
        );
    }

    #[test]
    fn urls_keep_the_parentheses_they_balance() {
        let p = dark_palette();
        let line = "[wiki](https://en.wikipedia.org/wiki/Foo_(bar)) after";
        assert_eq!(
            colored(line, &highlights_for(line)),
            [
                ("[", Some(p.faint)),
                ("wiki", Some(p.link)),
                ("](", Some(p.faint)),
                ("https://en.wikipedia.org/wiki/Foo_(bar)", Some(p.completed)),
                (")", Some(p.faint)),
            ]
        );
        for (line, url) in [
            (
                "see https://en.wikipedia.org/wiki/Foo_(bar) ok",
                "https://en.wikipedia.org/wiki/Foo_(bar)",
            ),
            ("(see https://x.io/a_(b))", "https://x.io/a_(b)"),
            ("(https://x.io/a).", "https://x.io/a"),
        ] {
            assert_eq!(
                colored(line, &highlights_for(line)),
                [(url, Some(p.link))],
                "{line}"
            );
        }
    }

    #[test]
    fn emphasis_wraps_links_and_styles_link_text_and_strikes_wrap_emphasis() {
        let p = dark_palette();
        let line = "**[bold link](https://x.io)**";
        assert_eq!(
            styled(line, &highlights_for(line)),
            [
                ("**[", Some(p.faint), true, false),
                ("bold link", Some(p.link), true, false),
                ("](", Some(p.faint), true, false),
                ("https://x.io", Some(p.completed), true, false),
                (")**", Some(p.faint), true, false),
            ]
        );
        let line = "[**bold**](https://x.io) and *[a](u)*";
        assert_eq!(
            styled(line, &highlights_for(line)),
            [
                ("[", Some(p.faint), false, false),
                ("**", Some(p.faint), true, false),
                ("bold", Some(p.link), true, false),
                ("**", Some(p.faint), true, false),
                ("](", Some(p.faint), false, false),
                ("https://x.io", Some(p.completed), false, false),
                (")", Some(p.faint), false, false),
                ("*[", Some(p.faint), false, true),
                ("a", Some(p.link), false, true),
                ("](", Some(p.faint), false, true),
                ("u", Some(p.completed), false, true),
                (")*", Some(p.faint), false, true),
            ]
        );
        let line = "~~**x**~~ and ~~[gone](u)~~";
        let (spans, decor) = highlight_text(fixtures::DARK, line).remove(0);
        assert_eq!(
            styled(line, &spans),
            [
                ("~~", Some(p.faint), false, false),
                ("**", Some(p.faint), true, false),
                ("x", Some(p.completed), true, false),
                ("**", Some(p.faint), true, false),
                ("~~", Some(p.faint), false, false),
                ("~~[", Some(p.faint), false, false),
                ("gone", Some(p.link), false, false),
                ("](", Some(p.faint), false, false),
                ("u", Some(p.completed), false, false),
                (")~~", Some(p.faint), false, false),
            ]
        );
        assert_eq!(decor.strikes, [2..7, 16..25]);
    }

    // ── Unicode safety ────────────────────────────────────────────────────────

    #[test]
    fn emoji_in_heading_does_not_panic() {
        let line = "# 🐦 Wren";
        let spans = highlights_for(line);
        assert!(!spans.is_empty());
        // Span must not straddle a char boundary
        for (r, _) in &spans {
            assert!(line.is_char_boundary(r.start), "bad start {}", r.start);
            assert!(line.is_char_boundary(r.end), "bad end {}", r.end);
        }
    }

    #[test]
    fn multibyte_in_code_span_boundaries_are_valid() {
        let line = "说明: `代码块` end";
        let spans = highlights_for(line);
        for (r, _) in &spans {
            assert!(line.is_char_boundary(r.start), "bad start {}", r.start);
            assert!(line.is_char_boundary(r.end), "bad end {}", r.end);
        }
    }

    #[test]
    fn combining_chars_and_a_byte_order_mark_do_not_break_boundaries() {
        // 'é' as e + combining acute accent (2 chars, but check byte boundaries)
        for line in ["* caf\u{0065}\u{0301} item", "\u{feff}- [x] bom"] {
            let spans = highlights_for(line);
            assert!(!spans.is_empty());
            for (r, _) in &spans {
                assert!(line.is_char_boundary(r.start));
                assert!(line.is_char_boundary(r.end));
            }
        }
    }

    // ── Checkpoints ───────────────────────────────────────────────────────────

    #[test]
    fn change_line_resumes_from_the_checkpoint_at_or_before_it() {
        let mut h = MarkdownHighlighter::new(&settings(fixtures::DARK, None));
        let p = dark_palette();

        h.highlight_line("~~~").for_each(drop); // enter code block at line 0
        for _ in 0..2 * CHECKPOINT_INTERVAL {
            h.highlight_line("- [x] code line").for_each(drop);
        }
        assert_eq!(h.decor.len(), 2 * CHECKPOINT_INTERVAL + 1);
        for line in [CHECKPOINT_INTERVAL + 20, CHECKPOINT_INTERVAL] {
            h.change_line(line);
            assert_eq!(h.current_line(), CHECKPOINT_INTERVAL);
            assert_eq!(h.decor.len(), CHECKPOINT_INTERVAL);
            let spans: Vec<_> = h.highlight_line("still code?").collect();
            assert_eq!(
                spans,
                [(0..11, Highlight::colored(p.muted))],
                "the checkpoint must hold the open fence"
            );
            assert!(h.decor(CHECKPOINT_INTERVAL).is_some_and(|decor| decor.code));
        }
        // Past the last checkpoint, it resumes at the last one.
        h.change_line(10 * CHECKPOINT_INTERVAL);
        assert_eq!(h.current_line(), CHECKPOINT_INTERVAL);
        h.change_line(3);
        assert_eq!(h.current_line(), 0);
        assert!(h.decor(0).is_none());
    }

    // ── Palette switching ─────────────────────────────────────────────────────

    #[test]
    fn light_palette_accent_differs_from_dark() {
        let d = dark_palette();
        let l = light_palette();
        assert_ne!(
            d.accent, l.accent,
            "palettes should have distinct accent colours"
        );
    }

    #[test]
    fn update_with_new_palette_and_font_changes_colours_and_bold_family() {
        let mut h = MarkdownHighlighter::new(&settings(fixtures::DARK, None));
        h.update(&Settings {
            palette: fixtures::LIGHT,
            font: Font::with_name("Fira Code"),
            front_matter_end: None,
        });

        let spans: Vec<_> = h.highlight_line("# Heading").collect();
        assert!(!spans.is_empty());
        assert_eq!(spans[0].1.color, Some(fixtures::LIGHT.faint));
        assert_eq!(spans[1].1.color, Some(fixtures::LIGHT.heading));
        let bold = spans[1].1.font.unwrap();
        assert_eq!(bold.family, font::Family::Name("Fira Code"));
        assert_eq!(bold.weight, font::Weight::Bold);
    }
}
