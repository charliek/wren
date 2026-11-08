//! A simple Markdown syntax highlighter that colours spans from a [`Palette`].
//!
//! Implements `iced::core::text::highlighter::Highlighter` using hand-written
//! Markdown pattern matching rather than the syntect-based `iced_highlighter`.
//! This avoids taking colours from a fixed third-party theme and instead takes
//! them from the host's [`Palette`].
//!
//! Supported constructs:
//!   - Fenced code blocks (`` ``` ``): entire block content is code-coloured.
//!   - ATX headings (`#`–`######`): faint markers, bold accent H1/primary H2–H6.
//!   - Block-quotes (`> `): muted colour.
//!   - Unordered/ordered list and open-task markers: accent colour.
//!   - Completed tasks: faint text; the editor widget draws the strike.
//!   - Inline code spans (`` `...` ``): muted colour.
//!   - Bold (`**...**`, `__...__`): bold font weight.
//!   - Italic (`*...*`, `_..._`): italic font style.
//!   - Strikethrough (`~~...~~`): danger colour.
//!   - Inline HTML comments (`<!--...-->`): faint colour.
//!
//! Code-block state is tracked with snapshots every [`SNAPSHOT_INTERVAL`] lines
//! so that `change_line` can resync cheaply without replaying the full document.
//!
//! All byte ranges returned are valid UTF-8 boundaries.

use std::ops::Range;

use iced::advanced::text::highlighter::{self, Format};
use iced::font::{self, Font};
use iced::Color;

use crate::Palette;

const SNAPSHOT_INTERVAL: usize = 50;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Highlight produced for a byte range within a line.
#[derive(Debug, Clone, Copy)]
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

/// Settings for the [`MarkdownHighlighter`].  Changing the palette or the
/// editor font triggers a full re-highlight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub palette: Palette,
    /// The editor font; bold and italic spans are its variants.
    pub font: Font,
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

/// Per-line code-block state (the only context that crosses line boundaries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineState {
    in_code_block: bool,
}

impl LineState {
    const NORMAL: Self = Self {
        in_code_block: false,
    };
}

/// The Wren Markdown syntax highlighter.
pub struct MarkdownHighlighter {
    settings: Settings,
    /// `snapshots[i]` is the state at the start of line `i * SNAPSHOT_INTERVAL`.
    snapshots: Vec<LineState>,
    /// State at the start of the current line.
    current_state: LineState,
    current_line: usize,
}

impl highlighter::Highlighter for MarkdownHighlighter {
    type Settings = Settings;
    type Highlight = Highlight;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Highlight)>;

    fn new(settings: &Settings) -> Self {
        Self {
            settings: *settings,
            snapshots: vec![LineState::NORMAL],
            current_state: LineState::NORMAL,
            current_line: 0,
        }
    }

    fn update(&mut self, new_settings: &Settings) {
        self.settings = *new_settings;
        // New colours or a new font require re-highlighting from the beginning.
        self.reset_to(0, LineState::NORMAL);
    }

    fn change_line(&mut self, line: usize) {
        let snapshot_idx = line / SNAPSHOT_INTERVAL;
        if snapshot_idx < self.snapshots.len() {
            let state = self.snapshots[snapshot_idx];
            self.reset_to(snapshot_idx * SNAPSHOT_INTERVAL, state);
        } else {
            // Snapshot doesn't exist yet — reset from the beginning.
            self.reset_to(0, LineState::NORMAL);
        }
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        let state_before = self.current_state;

        // Advance state: detect code-fence toggles.
        // A fence line starts with 3+ backticks; the rest may be an info string
        // (e.g. "```rust") for the opening fence, or empty for the closing fence.
        let after_backticks = line.trim_start_matches('`');
        let backtick_count = line.len() - after_backticks.len();
        if backtick_count >= 3 {
            self.current_state.in_code_block = !self.current_state.in_code_block;
        }

        let palette = Ink::new(self.settings.palette, self.settings.font);
        let highlights = compute_highlights(line, state_before, &self.current_state, &palette);

        self.current_line += 1;

        // Store a snapshot at every SNAPSHOT_INTERVAL boundary.
        if self.current_line.is_multiple_of(SNAPSHOT_INTERVAL) {
            let idx = self.current_line / SNAPSHOT_INTERVAL;
            if idx == self.snapshots.len() {
                self.snapshots.push(self.current_state);
            } else if idx < self.snapshots.len() {
                self.snapshots[idx] = self.current_state;
            }
        }

        highlights.into_iter()
    }

    fn current_line(&self) -> usize {
        self.current_line
    }
}

impl MarkdownHighlighter {
    fn reset_to(&mut self, line: usize, state: LineState) {
        self.current_line = line;
        self.current_state = state;
        // Discard snapshots past this point; they will be rebuilt on demand.
        let keep = (line / SNAPSHOT_INTERVAL) + 1;
        self.snapshots.truncate(keep);
        if self.snapshots.is_empty() {
            self.snapshots.push(LineState::NORMAL);
        }
    }
}

// ---------------------------------------------------------------------------
// Per-line highlight computation
// ---------------------------------------------------------------------------

/// Returns a list of `(byte_range, Highlight)` pairs for `line`.
/// `state_before` is the block-level state entering the line;
/// `state_after` is what it became after processing the line.
fn compute_highlights(
    line: &str,
    state_before: LineState,
    _state_after: &LineState,
    palette: &Ink,
) -> Vec<(Range<usize>, Highlight)> {
    if line.is_empty() {
        return vec![];
    }

    // ── Inside a fenced code block ───────────────────────────────────────────
    if state_before.in_code_block {
        // The fence line itself toggles state_before.in_code_block == false,
        // but that logic is handled in highlight_line by looking at state_before.
        // Here, entering with in_code_block==true means the whole line is code.
        return vec![(0..line.len(), Highlight::colored(palette.muted))];
    }

    // ── Fenced code-block opening/closing line ───────────────────────────────
    if is_code_fence(line) {
        return vec![(0..line.len(), Highlight::colored(palette.faint))];
    }

    // ── ATX heading ──────────────────────────────────────────────────────────
    if let Some(level) = heading_level(line) {
        let marker_end = level + usize::from(line.as_bytes().get(level) == Some(&b' '));
        let mut result = vec![(0..marker_end, Highlight::colored(palette.faint))];
        if marker_end < line.len() {
            let color = if level == 1 {
                palette.accent
            } else {
                palette.text
            };
            result.push((marker_end..line.len(), Highlight::bold(color, palette.font)));
        }
        return result;
    }

    // ── Blockquote ───────────────────────────────────────────────────────────
    if let Some(rest_offset) = blockquote_prefix(line) {
        let mut result = vec![(0..rest_offset, Highlight::colored(palette.muted))];
        let rest = &line[rest_offset..];
        for (r, h) in scan_inline(rest, palette) {
            result.push((r.start + rest_offset..r.end + rest_offset, h));
        }
        return result;
    }

    // ── Thematic break (--- / *** / ___) ─────────────────────────────────────
    if is_thematic_break(line) {
        return vec![(0..line.len(), Highlight::colored(palette.faint))];
    }

    // ── List item marker ─────────────────────────────────────────────────────
    if done_task_text(line).is_some() {
        return vec![(0..line.len(), Highlight::colored(palette.faint))];
    }
    if let Some(mut marker_end) = list_marker_end(line) {
        let rest = &line[marker_end..];
        if rest.starts_with("[ ] ") {
            marker_end += 4;
        }
        let mut result = vec![(0..marker_end, Highlight::colored(palette.accent))];
        let rest = &line[marker_end..];
        for (r, h) in scan_inline(rest, palette) {
            result.push((r.start + marker_end..r.end + marker_end, h));
        }
        return result;
    }

    // ── Normal paragraph line: scan inline elements ───────────────────────────
    scan_inline(line, palette)
}

// ---------------------------------------------------------------------------
// Completed tasks
// ---------------------------------------------------------------------------

/// The struck text of a completed task line (after its `- [x] ` marker, with
/// trailing blanks trimmed), or `None` if `line` is not one.
fn done_task_text(line: &str) -> Option<Range<usize>> {
    let marker_end = list_marker_end(line)?;
    let rest = &line[marker_end..];
    if !(rest.starts_with("[x] ") || rest.starts_with("[X] ")) {
        return None;
    }
    let start = marker_end + 4;
    Some(start..line.trim_end().len().max(start))
}

/// Yields, for each line in order, its completed-task text range. Lines inside
/// fenced code blocks are code, matching [`MarkdownHighlighter`]'s fence state.
pub(super) fn done_task_ranges<'a, I: Iterator<Item = &'a str>>(
    lines: I,
) -> impl Iterator<Item = Option<Range<usize>>> + use<'a, I> {
    let mut in_code_block = false;
    lines.map(move |line| {
        let range = if in_code_block {
            None
        } else {
            done_task_text(line)
        };
        if is_code_fence(line) {
            in_code_block = !in_code_block;
        }
        range
    })
}

// ---------------------------------------------------------------------------
// Block-level pattern detection
// ---------------------------------------------------------------------------

fn is_code_fence(line: &str) -> bool {
    // A code fence is 3+ backticks at the start of a line; the rest may be an
    // info string (e.g. "```rust") for opening fences.
    let stripped = line.trim_start_matches('`');
    line.len() - stripped.len() >= 3
}

fn heading_level(line: &str) -> Option<usize> {
    let count = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&count) {
        let rest = &line[count..];
        if rest.starts_with(' ') || rest.is_empty() {
            return Some(count);
        }
    }
    None
}

fn blockquote_prefix(line: &str) -> Option<usize> {
    if line.starts_with("> ") {
        return Some(2);
    }
    if line.starts_with('>') {
        return Some(1);
    }
    None
}

fn is_thematic_break(line: &str) -> bool {
    let trimmed = line.trim();
    for ch in ['-', '*', '_'] {
        let chars: Vec<char> = trimmed.chars().collect();
        if chars.len() >= 3 && chars.iter().all(|&c| c == ch || c == ' ') {
            let non_space: usize = chars.iter().filter(|&&c| c == ch).count();
            if non_space >= 3 {
                return true;
            }
        }
    }
    false
}

fn list_marker_end(line: &str) -> Option<usize> {
    // Unordered: `- `, `* `, `+ ` (with optional leading spaces)
    let leading = line.len() - line.trim_start_matches(' ').len();
    if leading > 3 {
        return None; // Too much indentation to be a first-level list marker
    }
    let rest = &line[leading..];
    if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
        return Some(leading + 2);
    }
    // Ordered: `1. `, `12. `, etc.
    let digit_count = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count > 0 && rest[digit_count..].starts_with(". ") {
        return Some(leading + digit_count + 2);
    }
    None
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

    fn highlights_for(line: &str) -> Vec<(Range<usize>, Highlight)> {
        let p = dark_palette();
        compute_highlights(line, LineState::NORMAL, &LineState::NORMAL, &p)
    }

    fn has_span_at(spans: &[(Range<usize>, Highlight)], byte_start: usize) -> bool {
        spans.iter().any(|(r, _)| r.start == byte_start)
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
                let spans = compute_highlights(&line, LineState::NORMAL, &LineState::NORMAL, &p);
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
    fn hash_without_space_is_not_a_heading() {
        let spans = highlights_for("#notaheading");
        // Should not produce a full-line accent span
        for (r, _) in &spans {
            assert_ne!(r.start, 0, "unexpected span starting at 0");
        }
    }

    #[test]
    fn list_and_open_task_markers_use_accent_but_done_tasks_are_faint() {
        for palette in [fixtures::DARK, fixtures::LIGHT] {
            let p = Ink::new(palette, fixtures::FONT);
            for (line, end) in [
                ("- words", 2),
                ("* words", 2),
                ("+ words", 2),
                ("12. words", 4),
                ("- [ ] words", 6),
            ] {
                let spans = compute_highlights(line, LineState::NORMAL, &LineState::NORMAL, &p);
                assert_eq!(spans[0].0, 0..end);
                assert_eq!(spans[0].1.color, Some(p.accent));
            }
            for line in ["- [x] completed", "  - [X] **completed**"] {
                let spans = compute_highlights(line, LineState::NORMAL, &LineState::NORMAL, &p);
                assert_eq!(spans.len(), 1);
                assert_eq!(spans[0].0, 0..line.len());
                assert_eq!(spans[0].1.color, Some(p.faint));
                assert_eq!(spans[0].1.font, None);
            }
        }
    }

    #[test]
    fn done_task_ranges_skip_markers_trailing_blanks_and_code_blocks() {
        let text = "- [x] done  \n  * [X] **café**\n- [ ] open\n- [x] \n```md\n- [x] code\n```\n1. [x] last";
        let ranges: Vec<_> = done_task_ranges(text.lines()).collect();
        assert_eq!(
            ranges,
            [
                Some(6..10),
                Some(8..17),
                None,
                Some(6..6),
                None,
                None,
                None,
                Some(7..11),
            ]
        );
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

    // ── Code block ───────────────────────────────────────────────────────────

    #[test]
    fn code_block_fence_line_is_faint_coloured() {
        let p = dark_palette();
        let spans = compute_highlights(
            "```",
            LineState::NORMAL,
            &LineState {
                in_code_block: true,
            },
            &p,
        );
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].0, 0..3);
    }

    #[test]
    fn line_inside_code_block_is_fully_muted() {
        let p = dark_palette();
        let inside = LineState {
            in_code_block: true,
        };
        let line = "let x = 42;";
        let spans = compute_highlights(line, inside, &inside, &p);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].0, 0..line.len());
    }

    // ── Blockquote ───────────────────────────────────────────────────────────

    #[test]
    fn blockquote_prefix_is_muted() {
        let spans = highlights_for("> quoted text");
        assert!(has_span_at(&spans, 0));
        assert_eq!(span_color_at(&spans, 0), Some(dark_palette().muted));
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
            let spans = compute_highlights(
                "This is **bold** text.",
                LineState::NORMAL,
                &LineState::NORMAL,
                &p,
            );
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
        // Should have exactly one bold span covering 0..line.len() - 1 (or similar)
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
    fn combining_chars_do_not_break_boundaries() {
        // 'é' as e + combining acute accent (2 chars, but check byte boundaries)
        let line = "* caf\u{0065}\u{0301} item";
        let spans = highlights_for(line);
        for (r, _) in &spans {
            assert!(line.is_char_boundary(r.start));
            assert!(line.is_char_boundary(r.end));
        }
    }

    // ── Code-block state machine ──────────────────────────────────────────────

    #[test]
    fn code_block_toggle_tracked_in_highlight_line() {
        let settings = Settings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
        };
        let mut h = MarkdownHighlighter::new(&settings);
        let p = dark_palette();

        // Normal line → no code block
        let spans: Vec<_> = h.highlight_line("normal").collect();
        // We're not in a code block, so it may have no or inline spans — just not the full-line code colour
        let _ = spans;

        // Open code fence
        h.highlight_line("```rust").for_each(drop);

        // Inside code block
        let spans: Vec<_> = h.highlight_line("let x = 1;").collect();
        assert!(!spans.is_empty(), "expected code-block highlights");
        assert_eq!(
            spans[0].1.color,
            Some(p.muted),
            "inside code block should be muted"
        );

        // Close code fence
        h.highlight_line("```").for_each(drop);

        // Back to normal: "after block" is a plain prose line.
        // It should produce zero spans (no inline markup) or non-muted spans —
        // but never a single full-line span with the code-block muted colour.
        let spans: Vec<_> = h.highlight_line("after block").collect();
        let is_full_line_muted = spans.len() == 1
            && spans[0].0 == (0.."after block".len())
            && spans[0].1.color == Some(p.muted);
        assert!(
            !is_full_line_muted,
            "line after code block should not be fully muted"
        );
    }

    #[test]
    fn change_line_resyncs_to_snapshot() {
        let settings = Settings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
        };
        let mut h = MarkdownHighlighter::new(&settings);
        let p = dark_palette();

        // Simulate highlighting past the first snapshot boundary (50 lines)
        h.highlight_line("```").for_each(drop); // enter code block at line 0
        for _ in 0..SNAPSHOT_INTERVAL {
            h.highlight_line("code line").for_each(drop);
        }
        // Now at line SNAPSHOT_INTERVAL+1; a snapshot for line SNAPSHOT_INTERVAL should exist.
        // `change_line` to line SNAPSHOT_INTERVAL should resync to the snapshot.
        h.change_line(SNAPSHOT_INTERVAL);
        // After re-sync, the state should still know we're in a code block.
        let spans: Vec<_> = h.highlight_line("still code?").collect();
        assert!(!spans.is_empty());
        assert_eq!(
            spans[0].1.color,
            Some(p.muted),
            "snapshot should capture code-block state"
        );
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
        let dark_settings = Settings {
            palette: fixtures::DARK,
            font: fixtures::FONT,
        };
        let light_settings = Settings {
            palette: fixtures::LIGHT,
            font: Font::with_name("Fira Code"),
        };
        let mut h = MarkdownHighlighter::new(&dark_settings);
        h.update(&light_settings);

        let spans: Vec<_> = h.highlight_line("# Heading").collect();
        assert!(!spans.is_empty());
        assert_eq!(spans[0].1.color, Some(fixtures::LIGHT.faint));
        assert_eq!(spans[1].1.color, Some(fixtures::LIGHT.accent));
        let bold = spans[1].1.font.unwrap();
        assert_eq!(bold.family, font::Family::Name("Fira Code"));
        assert_eq!(bold.weight, font::Weight::Bold);
    }
}
