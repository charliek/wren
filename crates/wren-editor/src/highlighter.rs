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
//!   - Fenced and indented code: muted body, faint fences.
//!   - ATX headings (`#`–`######`): faint markers, bold accent H1/primary H2–H6.
//!   - Block-quote markers: muted.
//!   - List markers and open task boxes: accent colour.
//!   - Completed tasks: faint text; the editor widget draws the strike.
//!   - Thematic breaks, setext underlines, and HTML blocks: faint.
//!   - Inline code spans (`` `...` ``): success colour.
//!   - Bold (`**...**`, `__...__`): bold font weight.
//!   - Italic (`*...*`, `_..._`): italic font style.
//!   - Strikethrough (`~~...~~`): danger colour.
//!   - Inline HTML comments (`<!--...-->`): faint colour.
//!
//! The scanner state is checkpointed every [`CHECKPOINT_INTERVAL`] lines so
//! that `change_line` can resync cheaply without replaying the document.
//! Alongside the spans, each highlighted line leaves a [`LineDecor`] for the
//! widget to draw: the strike through completed tasks and the code band.
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

    fn bold_italic(color: Color, base: Font) -> Self {
        Self {
            color: Some(color),
            font: Some(Font {
                weight: font::Weight::Bold,
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
    danger: Color,
    success: Color,
    font: Font,
}

impl Ink {
    fn new(palette: Palette, font: Font) -> Self {
        let Palette {
            text,
            muted,
            faint,
            accent,
            danger,
            success,
        } = palette;
        Self {
            text,
            muted,
            faint,
            accent,
            danger,
            success,
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
            Container::Quote { .. } => ink.muted,
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
                let color = if *level == 1 { ink.accent } else { ink.text };
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
            let text = info.text_start..info.text_start + line[info.text_start..].trim_end().len();
            spans.push(info.text_start..line.len(), Highlight::colored(ink.faint));
            decor.strikes.push(text);
        }
        Kind::Prose => {
            spans.extend_at(info.text_start, scan_inline(&line[info.text_start..], ink));
        }
    }
    (spans.0, decor)
}

// ---------------------------------------------------------------------------
// Inline element scanner
// ---------------------------------------------------------------------------

/// Returns `(byte_range_within_line, Highlight)` pairs for inline Markdown.
///
/// Handles: inline code (highest priority), HTML comments, bold+italic, bold,
/// italic, strikethrough.  Ranges do not overlap.
fn scan_inline(line: &str, palette: &Ink) -> Vec<(Range<usize>, Highlight)> {
    // We build a priority-ordered list of non-overlapping spans.
    // First collect code spans (they cannot contain other markup).
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let len = line.len();

    let mut spans: Vec<(Range<usize>, Highlight)> = Vec::new();
    let mut occupied = vec![false; len.max(1)];

    // ── 1. Inline code spans ──────────────────────────────────────────────────
    extract_backtick_spans(line, &chars, palette, &mut spans, &mut occupied);

    // ── 2. HTML comments ─────────────────────────────────────────────────────
    extract_between(
        line,
        "<!--",
        "-->",
        Highlight::colored(palette.faint),
        &mut spans,
        &mut occupied,
    );

    // ── 3. Bold-italic (*** or ___) ────────────────────────────────────────────
    extract_emphasis(
        line,
        "***",
        Highlight::bold_italic(palette.text, palette.font),
        palette.faint,
        &mut spans,
        &mut occupied,
    );
    extract_emphasis(
        line,
        "___",
        Highlight::bold_italic(palette.text, palette.font),
        palette.faint,
        &mut spans,
        &mut occupied,
    );

    // ── 4. Bold (** or __) ───────────────────────────────────────────────────
    extract_emphasis(
        line,
        "**",
        Highlight::bold(palette.text, palette.font),
        palette.faint,
        &mut spans,
        &mut occupied,
    );
    extract_emphasis(
        line,
        "__",
        Highlight::bold(palette.text, palette.font),
        palette.faint,
        &mut spans,
        &mut occupied,
    );

    // ── 5. Italic (* or _) ───────────────────────────────────────────────────
    extract_emphasis(
        line,
        "*",
        Highlight::italic(palette.text, palette.font),
        palette.faint,
        &mut spans,
        &mut occupied,
    );
    extract_emphasis(
        line,
        "_",
        Highlight::italic(palette.text, palette.font),
        palette.faint,
        &mut spans,
        &mut occupied,
    );

    // ── 6. Strikethrough (~~) ────────────────────────────────────────────────
    extract_between(
        line,
        "~~",
        "~~",
        Highlight::colored(palette.danger),
        &mut spans,
        &mut occupied,
    );

    // ── 7. Inline links: [text](url) ─────────────────────────────────────────
    extract_links(line, palette, &mut spans, &mut occupied);

    // Sort by start position for deterministic output.
    spans.sort_by_key(|(r, _)| r.start);
    spans
}

/// Extract all backtick-delimited code spans, respecting multi-backtick fences.
fn extract_backtick_spans(
    _line: &str,
    chars: &[(usize, char)],
    palette: &Ink,
    spans: &mut Vec<(Range<usize>, Highlight)>,
    occupied: &mut [bool],
) {
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let (byte_i, ch) = chars[i];
        if ch != '`' || occupied.get(byte_i).copied().unwrap_or(true) {
            i += 1;
            continue;
        }
        // Count run of opening backticks.
        let open_start = i;
        let mut j = i + 1;
        while j < n && chars[j].1 == '`' {
            j += 1;
        }
        let tick_count = j - i;
        // Search for a closing run of exactly tick_count backticks.
        let mut k = j;
        while k < n {
            if chars[k].1 == '`' {
                let close_start = k;
                let mut m = k + 1;
                while m < n && chars[m].1 == '`' && (m - close_start) < tick_count {
                    m += 1;
                }
                if m - close_start == tick_count && (m >= n || chars[m].1 != '`') {
                    // Found a valid closing fence.
                    let byte_start = chars[open_start].0;
                    let last = chars[m - 1];
                    let byte_end = last.0 + last.1.len_utf8();
                    if !any_occupied(occupied, byte_start..byte_end) {
                        mark_occupied(occupied, byte_start..byte_end);
                        spans.push((byte_start..byte_end, Highlight::colored(palette.success)));
                    }
                    i = m;
                    break;
                } else {
                    k = m;
                }
            } else {
                k += 1;
            }
        }
        if k >= n {
            i = j; // No matching close — skip the opening ticks.
        }
    }
}

fn extract_emphasis(
    line: &str,
    marker: &str,
    highlight: Highlight,
    faint: Color,
    spans: &mut Vec<(Range<usize>, Highlight)>,
    occupied: &mut [bool],
) {
    let mut search = 0;
    while let Some(relative) = line[search..].find(marker) {
        let opening = search + relative;
        let content = opening + marker.len();
        if any_occupied(occupied, opening..content) {
            search = opening + 1;
            continue;
        }
        let Some(closing) = line[content..]
            .match_indices(marker)
            .map(|(relative, _)| content + relative)
            .find(|&closing| !any_occupied(occupied, closing..closing + marker.len()))
        else {
            break;
        };
        let end = closing + marker.len();
        spans.push((opening..content, Highlight::colored(faint)));
        let mut run = None;
        for (relative, character) in line[content..closing].char_indices() {
            let byte = content + relative;
            if any_occupied(occupied, byte..byte + character.len_utf8()) {
                if let Some(start) = run.take() {
                    spans.push((start..byte, highlight));
                }
            } else {
                run.get_or_insert(byte);
            }
        }
        if let Some(start) = run {
            spans.push((start..closing, highlight));
        }
        spans.push((closing..end, Highlight::colored(faint)));
        mark_occupied(occupied, opening..end);
        search = end;
    }
}

/// Extract spans delimited by a fixed `open` and `close` marker string,
/// skipping regions already occupied by higher-priority spans.
fn extract_between(
    line: &str,
    open: &str,
    close: &str,
    highlight: Highlight,
    spans: &mut Vec<(Range<usize>, Highlight)>,
    occupied: &mut [bool],
) {
    let open_len = open.len();
    let close_len = close.len();

    let mut search_from = 0;
    while search_from + open_len <= line.len() {
        let Some(rel_open) = line[search_from..].find(open) else {
            break;
        };
        let abs_open = search_from + rel_open;
        if any_occupied(occupied, abs_open..abs_open + open_len) {
            search_from = abs_open + 1;
            continue;
        }
        let content_start = abs_open + open_len;
        // Avoid zero-length content (e.g. `**`) matching itself.
        let Some(rel_close) = line[content_start..].find(close) else {
            break;
        };
        let abs_close_start = content_start + rel_close;
        let abs_close_end = abs_close_start + close_len;
        if any_occupied(occupied, abs_open..abs_close_end) {
            search_from = abs_open + 1;
            continue;
        }
        // Ensure we're on valid UTF-8 boundaries.
        if !line.is_char_boundary(abs_open) || !line.is_char_boundary(abs_close_end) {
            search_from = abs_open + 1;
            continue;
        }
        mark_occupied(occupied, abs_open..abs_close_end);
        spans.push((abs_open..abs_close_end, highlight));
        search_from = abs_close_end;
    }
}

/// Extract inline links in `[text](url)` or `[text][ref]` form.
fn extract_links(
    line: &str,
    palette: &Ink,
    spans: &mut Vec<(Range<usize>, Highlight)>,
    occupied: &mut [bool],
) {
    let mut search_from = 0;
    while search_from < line.len() {
        let Some(rel_bracket) = line[search_from..].find('[') else {
            break;
        };
        let open_bracket = search_from + rel_bracket;
        if any_occupied(occupied, open_bracket..open_bracket + 1) {
            search_from = open_bracket + 1;
            continue;
        }
        let Some(rel_close) = line[open_bracket + 1..].find(']') else {
            break;
        };
        let close_bracket = open_bracket + 1 + rel_close;
        // Look for `(url)` or `[ref]` immediately after.
        let after_bracket = close_bracket + 1;
        if after_bracket >= line.len() {
            search_from = close_bracket + 1;
            continue;
        }
        let next = line.as_bytes().get(after_bracket).copied();
        let span_end = match next {
            Some(b'(') => line[after_bracket + 1..]
                .find(')')
                .map(|r| after_bracket + 1 + r + 1),
            Some(b'[') => line[after_bracket + 1..]
                .find(']')
                .map(|r| after_bracket + 1 + r + 1),
            _ => None,
        };
        if let Some(end) = span_end {
            if !any_occupied(occupied, open_bracket..end) && line.is_char_boundary(end) {
                mark_occupied(occupied, open_bracket..end);
                spans.push((open_bracket..end, Highlight::colored(palette.accent)));
            }
            search_from = end;
        } else {
            search_from = close_bracket + 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Occupied-range helpers
// ---------------------------------------------------------------------------

fn any_occupied(occupied: &[bool], range: Range<usize>) -> bool {
    occupied
        .get(range.clone())
        .is_some_and(|slice| slice.iter().any(|&b| b))
}

fn mark_occupied(occupied: &mut [bool], range: Range<usize>) {
    for cell in occupied.get_mut(range).into_iter().flatten() {
        *cell = true;
    }
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
    fn heading_markers_are_faint_and_only_h1_text_is_accent_in_both_palettes() {
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
                    Some(if level == 0 { p.accent } else { p.text })
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
                ("> ", Some(p.muted)),
                ("## ", Some(p.faint)),
                ("Title", Some(p.text)),
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
    fn list_and_open_task_markers_use_accent_but_done_tasks_are_faint() {
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
            for (line, start) in [("- [x] completed", 0), ("  - [X] **completed**", 2)] {
                let spans = highlight_text(palette, line).remove(0).0;
                assert_eq!(spans.len(), 1);
                assert_eq!(spans[0].0, start..line.len());
                assert_eq!(spans[0].1.color, Some(p.faint));
                assert_eq!(spans[0].1.font, None);
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
        assert!(spans.iter().any(|(range, highlight)| *range == (7..13)
            && highlight.color == Some(dark_palette().success)));
        let bold: Vec<_> = spans
            .iter()
            .filter(|(_, highlight)| {
                highlight
                    .font
                    .is_some_and(|font| font.weight == font::Weight::Bold)
            })
            .map(|(range, _)| range.clone())
            .collect();
        assert_eq!(bold, [2..7, 13..18]);
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
            [("> code", Some(p.muted))]
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
        assert_eq!(lines[4][1].1.color, Some(p.accent));
        // Unclosed, line 0 is a thematic break and the rest is Markdown.
        let lines = spans_of("---\ntitle: Notes\n# Heading");
        assert!(lines[1].is_empty());
        assert_eq!(lines[2][1].1.color, Some(p.accent));
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
    fn blockquote_markers_are_muted_at_every_depth() {
        let p = dark_palette();
        let line = "> > quoted **text**";
        let spans = highlights_for(line);
        assert_eq!(colored(line, &spans)[0], ("> > ", Some(p.muted)));
        let lazy = spans_of("> quote\nlazy");
        assert!(lazy[1].is_empty());
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
    fn bold_text_is_strong_while_its_markers_are_faint() {
        for palette in [fixtures::DARK, fixtures::LIGHT] {
            let p = Ink::new(palette, fixtures::FONT);
            let spans = highlight_text(palette, "This is **bold** text.")
                .remove(0)
                .0;
            let marker = spans.iter().find(|(range, _)| *range == (8..10)).unwrap();
            assert_eq!(marker.1.color, Some(p.faint));
            assert_eq!(marker.1.font, None);
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
    fn italic_text_preserves_style_without_emphasizing_its_markers() {
        let spans = highlights_for("This is *italic* text.");
        let marker = spans.iter().find(|(range, _)| *range == (8..9)).unwrap();
        assert_eq!(marker.1.font, None);
        assert_eq!(marker.1.color, Some(dark_palette().faint));
        let inner = spans.iter().find(|(range, _)| *range == (9..15)).unwrap();
        assert!(inner
            .1
            .font
            .is_some_and(|font| font.style == font::Style::Italic));
        assert_eq!(span_color_at(&spans, 15), Some(dark_palette().faint));
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
    fn strikethrough_coloured_with_danger() {
        let p = dark_palette();
        let spans = highlights_for("This is ~~deleted~~ text.");
        let st = spans.iter().find(|(r, _)| r.start == 8);
        assert!(st.is_some(), "no strikethrough span");
        assert_eq!(st.unwrap().1.color, Some(p.danger));
    }

    // ── Links ─────────────────────────────────────────────────────────────────

    #[test]
    fn inline_link_produces_accent_coloured_span() {
        let p = dark_palette();
        let spans = highlights_for("See [the docs](https://example.com) for more.");
        let link = spans.iter().find(|(r, _)| r.start == 4);
        assert!(link.is_some(), "no link span: {spans:?}");
        assert_eq!(link.unwrap().1.color, Some(p.accent));
    }

    #[test]
    fn reference_link_produces_accent_coloured_span() {
        let spans = highlights_for("Read [the guide][guide-ref] now.");
        let link = spans.iter().find(|(r, _)| r.start == 5);
        assert!(link.is_some(), "no ref link span: {spans:?}");
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
        assert_eq!(spans[1].1.color, Some(fixtures::LIGHT.accent));
        let bold = spans[1].1.font.unwrap();
        assert_eq!(bold.family, font::Family::Name("Fira Code"));
        assert_eq!(bold.weight, font::Weight::Bold);
    }
}
