//! Block structure: the one line classifier.
//!
//! [`Scanner`] reads a document forward, one line at a time, keeping a stack
//! of open containers (block quotes and list items) and the open leaf block
//! (a paragraph, a fenced code block, or an HTML block). Each line comes back
//! as a [`LineInfo`]. The rules follow CommonMark's block structure, trimmed
//! to what an editor needs:
//!
//! - **Containers.** `>` (up to 3 spaces of indent, then an optional space)
//!   and list items: a bullet `-` `*` `+`, or 1–9 digits then `.` or `)`,
//!   followed by whitespace or the end of the line. 1–4 columns of space
//!   after the marker set the content column; 5 or more (or none, on an
//!   empty item) put it one column past the marker. Several containers may
//!   open on one line (`> - a`, `- - a`).
//! - **Continuation.** A quote continues on a line with its `>`; a list item
//!   on a line indented to its content column, or a blank line (except the
//!   blank right after an item that opened empty). A paragraph line that
//!   matches neither is a *lazy* continuation when it would not start a
//!   block of its own.
//! - **Interrupting a paragraph.** A list item can only interrupt a paragraph
//!   when it has content and, if ordered, starts at 1. Indented code never
//!   does. `---` under a paragraph is a setext underline, never a break.
//! - **Precedence.** A thematic break wins over a list item (`- - -`).
//! - **Leaves.** ATX headings, setext underlines, thematic breaks, fences
//!   (backtick or tilde, relative to the container's content column),
//!   indented code, HTML blocks (comments, `<script>`-like tags, and
//!   CommonMark's block-level tags), and YAML front matter on line 0.
//!
//! The scanner state is `Clone`, so a highlighter can checkpoint it and
//! resume scanning from any line.
//!
//! **Front matter is the one rule that looks ahead.** Whether line 0's `---`
//! opens front matter depends on a closing delimiter further down, so a
//! scanner fed one line at a time must be told before it starts: build it
//! with [`Scanner::with_front_matter`] and the [`front_matter_end`] of the
//! whole document. A highlighter, which only ever sees one line, carries
//! that value in its settings. The host recomputes it after every edit, and
//! when it changes the highlighter starts again from line 0 and drops its
//! checkpoints, since each one was taken with the old value. Fed that way, a
//! scanner gives exactly what [`scan`] gives.
//!
//! Columns count Unicode scalars with tabs expanded to stops of [`TAB_STOP`];
//! only ASCII space and tab are indentation. Byte offsets in [`LineInfo`] are
//! into the line text.

use std::ops::Range;

use crate::Lines;

/// Tab stops are every 4 columns, as in CommonMark.
pub const TAB_STOP: usize = 4;

/// The tab stop after `col`.
pub const fn next_tab_stop(col: usize) -> usize {
    (col / TAB_STOP + 1) * TAB_STOP
}

/// The column reached after `s`, starting from column `col`.
pub fn advance_column(mut col: usize, s: &str) -> usize {
    for ch in s.chars() {
        col = match ch {
            '\t' => next_tab_stop(col),
            '\u{feff}' => col,
            _ => col + 1,
        };
    }
    col
}

/// The column at which byte `byte` of `line` starts.
pub fn column(line: &str, byte: usize) -> usize {
    advance_column(0, &line[..byte])
}

/// The byte offset of the first character at or past column `col`, or the
/// line's length if the line is shorter.
pub fn byte_at_column(line: &str, col: usize) -> usize {
    let mut at = 0;
    for (i, ch) in line.char_indices() {
        if at >= col {
            return i;
        }
        at = advance_column(at, ch.encode_utf8(&mut [0; 4]));
    }
    line.len()
}

/// The line's leading spaces and tabs, as `(bytes, columns)`.
pub fn indentation(line: &str) -> (usize, usize) {
    let (pos, col) = skip_ws(line.as_bytes(), 0, 0);
    (pos, col)
}

/// The container prefix of a line: its quote markers, list markers, and
/// the indentation that puts it inside its list items.
pub fn container_prefix<'a>(line: &'a str, info: &LineInfo) -> &'a str {
    &line[..info.prefix_end]
}

/// One container's share of a line's container prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Segment {
    /// Byte offset where the share starts, including the whitespace before
    /// a marker.
    pub start: usize,
    /// Byte offset where it ends: at the container's content column, or at
    /// the end of a line too short to reach it.
    pub end: usize,
    /// Byte offset of the container's marker (`>` or a list marker) when
    /// the line carries one. A list item carries its marker only on its
    /// first line.
    pub marker: Option<usize>,
}

/// Split a line's container prefix by container.
///
/// `containers` are the line's own containers, outermost first: on a lazy
/// line, only the ones it continued. The first `continued` of them were
/// open before the line (see [`Scanner::continued`]); the rest opened on
/// it. `index` is the line's index, so a byte order mark on line 0 is
/// skipped. One segment comes back per container.
pub fn segments(
    line: &str,
    index: usize,
    containers: &[Container],
    continued: usize,
) -> Vec<Segment> {
    let b = line.as_bytes();
    let mut pos = if index == 0 && line.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let mut col = 0;
    containers
        .iter()
        .enumerate()
        .map(|(slot, container)| {
            let start = pos;
            let marker = match container {
                Container::Quote { .. } => {
                    let (p, c) = skip_ws(b, pos, col);
                    if b.get(p) == Some(&b'>') {
                        (pos, col) = advance_to(b, p + 1, c + 1, quote_content(b, p, c));
                        Some(p)
                    } else {
                        None
                    }
                }
                Container::Item(item) if slot < continued => {
                    (pos, col) = advance_to(b, pos, col, item.content_column);
                    None
                }
                Container::Item(item) => {
                    let width = item.marker_end - item.marker_start;
                    (pos, col) =
                        advance_to(b, item.marker_end, item.indent + width, item.content_column);
                    Some(item.marker_start)
                }
            };
            Segment {
                start,
                end: pos,
                marker,
            }
        })
        .collect()
}

/// What a line is, once its container prefix is set aside.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Nothing but whitespace after the container prefix.
    Blank,
    /// Paragraph text.
    Prose,
    /// An ATX heading. `text` is the byte range of its title, without the
    /// opening and closing `#` sequences; it may be empty.
    Heading {
        level: u8,
        text: Range<usize>,
    },
    /// A setext underline. `start` is the first line of the heading text.
    SetextUnderline {
        level: u8,
        start: usize,
    },
    ThematicBreak,
    /// An opening fence. `info` is the byte range of the info string.
    FenceOpen {
        fence: Fence,
        info: Range<usize>,
    },
    FenceClose,
    FenceBody,
    IndentedCode,
    /// A front matter line, including both delimiters.
    FrontMatter,
    Html,
}

impl Kind {
    /// Code-like lines are left alone by Markdown editing, apart from
    /// indentation carry-over and their container prefix.
    pub fn is_code_like(&self) -> bool {
        matches!(
            self,
            Self::FenceOpen { .. }
                | Self::FenceClose
                | Self::FenceBody
                | Self::IndentedCode
                | Self::FrontMatter
                | Self::Html
        )
    }
}

/// A code fence: its character (`` ` `` or `~`) and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fence {
    pub ch: char,
    pub len: usize,
}

/// A list item marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Marker {
    Bullet(char),
    /// `width` is the number of digits as written, so `09.` has width 2.
    Ordered {
        number: u32,
        width: usize,
        delimiter: char,
    },
}

/// A task box: `[ ]`, `[x]`, or `[X]` right at the item's content column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Task {
    pub done: bool,
    /// Byte offset of the `[` on the item's first line.
    pub start: usize,
}

/// A list item, as opened on its first line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ListItem {
    /// Column of the marker.
    pub indent: usize,
    /// Column where the item's content starts (CommonMark's content
    /// column); continuation lines are indented at least this far.
    pub content_column: usize,
    /// Byte offset of the marker on the item's first line.
    pub marker_start: usize,
    /// Byte offset just past the marker on the item's first line.
    pub marker_end: usize,
    /// Byte offset of the item's text on its first line: past the marker,
    /// the spacing, and any task box.
    pub text_start: usize,
    pub marker: Marker,
    pub task: Option<Task>,
    /// Number of block quotes around the item.
    pub quote_depth: usize,
}

/// An open container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    /// `column` is where the `>` is and `content_column` where its content
    /// starts, both on the most recent line that carried the marker.
    Quote {
        column: usize,
        content_column: usize,
    },
    Item(ListItem),
}

impl Container {
    pub fn content_column(&self) -> usize {
        match self {
            Self::Quote { content_column, .. } => *content_column,
            Self::Item(item) => item.content_column,
        }
    }
}

/// One classified line.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LineInfo {
    pub kind: Kind,
    /// Containers this line belongs to.
    pub depth: usize,
    /// Block quotes among those containers.
    pub quote_depth: usize,
    /// The innermost list item that opened on this line.
    pub item: Option<ListItem>,
    /// The content column of the innermost container (0 outside any).
    pub content_column: usize,
    /// Byte offset where the container prefix ends.
    pub prefix_end: usize,
    /// Byte offset of the line's text: past the prefix, any task box, an
    /// ATX heading's opening `#`s, and whitespace.
    pub text_start: usize,
    /// A paragraph line that continues a paragraph inside containers
    /// whose markers it lacks.
    pub lazy: bool,
}

impl LineInfo {
    /// An item whose line holds only its marker (and task box).
    pub fn is_empty_item(&self) -> bool {
        self.item.is_some() && self.kind == Kind::Blank
    }
}

/// The line index of the closing delimiter of the document's YAML front
/// matter, if it has any.
///
/// Front matter needs line 0 to be `---` (a BOM before it and trailing
/// whitespace are allowed), a later `---` or `...` line, and a first
/// non-blank line in between that looks like YAML: `key:` followed by
/// whitespace or the line end, or a `#` comment.
///
/// It reads past line 0 only when line 0 is `---`, and then only as far as
/// the closing delimiter (to the end while the front matter is unclosed),
/// so an editor can call it after every edit.
pub fn front_matter_end<L: Lines + ?Sized>(lines: &L) -> Option<usize> {
    let first = lines.line(0);
    let first = first.strip_prefix('\u{feff}').unwrap_or(first);
    if trim_end_ws(first) != "---" {
        return None;
    }
    let count = lines.line_count();
    let mut i = 1;
    while i < count
        && trim_end_ws(lines.line(i))
            .trim_start_matches([' ', '\t'])
            .is_empty()
    {
        i += 1;
    }
    if i >= count || !looks_like_yaml(lines.line(i)) {
        return None;
    }
    (i + 1..count).find(|&j| matches!(trim_end_ws(lines.line(j)), "---" | "..."))
}

fn looks_like_yaml(line: &str) -> bool {
    if line.starts_with('#') {
        return true;
    }
    let key = line
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        .count();
    key > 0
        && line[key..]
            .strip_prefix(':')
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

/// Classify every line of a document.
pub fn scan<L: Lines + ?Sized>(lines: &L) -> Vec<LineInfo> {
    let mut scanner = Scanner::with_front_matter(front_matter_end(lines));
    (0..lines.line_count())
        .map(|i| scanner.next_line(lines.line(i)))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HtmlEnd {
    Blank,
    Comment,
    Tag(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leaf {
    None,
    Paragraph { start: usize },
    Fence(Fence),
    Html(HtmlEnd),
}

/// The forward block scanner. See the module docs for the rules.
#[derive(Clone, Debug)]
pub struct Scanner {
    line: usize,
    front_matter_end: Option<usize>,
    stack: Vec<Container>,
    leaf: Leaf,
    /// The stack index of an item that opened empty on the previous line;
    /// a blank line right after it ends it.
    empty_item: Option<usize>,
    continued: usize,
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner {
    /// A scanner for text without front matter: a `---` on line 0 is a
    /// thematic break even if front matter follows. For a whole document,
    /// use [`Scanner::with_front_matter`].
    pub fn new() -> Self {
        Self::with_front_matter(None)
    }

    /// A scanner that treats lines `0..=end` as front matter; pass the
    /// result of [`front_matter_end`].
    pub fn with_front_matter(end: Option<usize>) -> Self {
        Self {
            line: 0,
            front_matter_end: end,
            stack: Vec::new(),
            leaf: Leaf::None,
            empty_item: None,
            continued: 0,
        }
    }

    /// The index of the next line to scan.
    pub fn line_index(&self) -> usize {
        self.line
    }

    /// The front matter end this scanner was built with. A checkpoint is
    /// only good for a document whose [`front_matter_end`] still matches.
    pub fn front_matter_end(&self) -> Option<usize> {
        self.front_matter_end
    }

    /// The containers open after the last scanned line, outermost first.
    pub fn containers(&self) -> &[Container] {
        &self.stack
    }

    /// How many of the containers open before the last scanned line that
    /// line continued. Containers at this index and above opened on the
    /// line, except on a lazy line, which continues fewer containers than
    /// it leaves open and opens none.
    pub fn continued(&self) -> usize {
        self.continued
    }

    /// Classify the next line. `text` is the line without its ending.
    pub fn next_line(&mut self, text: &str) -> LineInfo {
        let index = self.line;
        self.line += 1;
        let b = text.as_bytes();
        let start = if index == 0 && text.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        let empty_item = self.empty_item.take();
        self.continued = 0;

        if self.front_matter_end.is_some_and(|end| index <= end) {
            return LineInfo {
                kind: Kind::FrontMatter,
                depth: 0,
                quote_depth: 0,
                item: None,
                content_column: 0,
                prefix_end: start,
                text_start: skip_ws(b, start, 0).0,
                lazy: false,
            };
        }

        let (mut pos, mut col) = (start, 0);
        // Blank after the markers matched so far: `>` alone is a blank line
        // inside its quote.
        let blank_from = |pos| skip_ws(b, pos, 0).0 == b.len();
        let mut base = 0;
        let mut matched = 0;
        while matched < self.stack.len() {
            match self.stack[matched] {
                Container::Quote { .. } => {
                    let (p, c) = skip_ws(b, pos, col);
                    if c.saturating_sub(base) > 3 || b.get(p) != Some(&b'>') {
                        break;
                    }
                    let content = quote_content(b, p, c);
                    (pos, col) = advance_to(b, p + 1, c + 1, content);
                    self.stack[matched] = Container::Quote {
                        column: c,
                        content_column: content,
                    };
                    base = content;
                }
                Container::Item(item) => {
                    if blank_from(pos) {
                        if empty_item == Some(matched) {
                            break;
                        }
                    } else if skip_ws(b, pos, col).1 < item.content_column {
                        break;
                    }
                    (pos, col) = advance_to(b, pos, col, item.content_column);
                    base = item.content_column;
                }
            }
            matched += 1;
        }
        self.continued = matched;
        let all_matched = matched == self.stack.len();

        if all_matched {
            match self.leaf {
                Leaf::Fence(fence) => {
                    let kind = if is_fence_close(b, pos, col, base, fence) {
                        self.leaf = Leaf::None;
                        Kind::FenceClose
                    } else {
                        Kind::FenceBody
                    };
                    return self.info(kind, None, b, pos, skip_ws(b, pos, col).0, false);
                }
                Leaf::Html(end) => {
                    let kind = if blank_from(pos) && end == HtmlEnd::Blank {
                        self.leaf = Leaf::None;
                        Kind::Blank
                    } else {
                        if html_ends(&text[pos..], end) {
                            self.leaf = Leaf::None;
                        }
                        Kind::Html
                    };
                    return self.info(kind, None, b, pos, skip_ws(b, pos, col).0, false);
                }
                Leaf::None | Leaf::Paragraph { .. } => {}
            }
        }

        let tip_paragraph = matches!(self.leaf, Leaf::Paragraph { .. });
        let mut in_paragraph = all_matched && tip_paragraph;
        let mut opened = false;
        let mut item = None;
        loop {
            let (p, c) = skip_ws(b, pos, col);
            if p == b.len() || c.saturating_sub(base) >= 4 {
                break;
            }
            if b[p] == b'>' {
                self.open(&mut matched, &mut opened);
                let content = quote_content(b, p, c);
                (pos, col) = advance_to(b, p + 1, c + 1, content);
                self.stack.push(Container::Quote {
                    column: c,
                    content_column: content,
                });
                matched += 1;
                base = content;
                in_paragraph = false;
                continue;
            }
            if is_thematic_break(b, p) {
                break;
            }
            let quote_depth = self.quote_depth_at(matched);
            let Some((new, p, c)) = list_marker(text, p, c, in_paragraph, quote_depth) else {
                break;
            };
            self.open(&mut matched, &mut opened);
            self.stack.push(Container::Item(new));
            matched += 1;
            (pos, col) = (p, c);
            base = new.content_column;
            in_paragraph = false;
            item = Some(new);
            if new.task.is_some() {
                // The task box belongs to the item's first paragraph, so
                // nothing after it opens another block.
                let kind = if new.text_start == b.len() {
                    self.leaf = Leaf::None;
                    Kind::Blank
                } else {
                    self.leaf = Leaf::Paragraph { start: index };
                    Kind::Prose
                };
                return self.info(kind, item, b, pos, new.text_start, false);
            }
        }

        let (p, c) = skip_ws(b, pos, col);
        if p == b.len() {
            self.close_unmatched(matched);
            self.leaf = Leaf::None;
            if item.is_some() && matches!(self.stack.last(), Some(Container::Item(_))) {
                self.empty_item = Some(self.stack.len() - 1);
            }
            return self.info(Kind::Blank, item, b, pos, p, false);
        }
        let maybe_lazy = tip_paragraph && !opened;
        let paragraph = match self.leaf {
            Leaf::Paragraph { start } if in_paragraph => Some(start),
            _ => None,
        };
        if let Some((kind, text_start)) =
            leaf_start(text, p, c.saturating_sub(base), paragraph, maybe_lazy)
        {
            self.close_unmatched(matched);
            self.leaf = match kind {
                Kind::FenceOpen { fence, .. } => Leaf::Fence(fence),
                Kind::Html => html_start(&text[p..])
                    .filter(|&end| !html_ends(&text[p..], end) || end == HtmlEnd::Blank)
                    .map_or(Leaf::None, Leaf::Html),
                _ => Leaf::None,
            };
            return self.info(kind, item, b, pos, text_start, false);
        }
        if maybe_lazy && !all_matched {
            return self.info(Kind::Prose, None, b, pos, p, true);
        }
        self.close_unmatched(matched);
        if !in_paragraph {
            self.leaf = Leaf::Paragraph { start: index };
        }
        self.info(Kind::Prose, item, b, pos, p, false)
    }

    /// Close the containers this line did not continue, before a new one
    /// opens.
    fn open(&mut self, matched: &mut usize, opened: &mut bool) {
        if !*opened {
            self.close_unmatched(*matched);
            self.leaf = Leaf::None;
            *opened = true;
        }
        *matched = self.stack.len();
    }

    fn close_unmatched(&mut self, matched: usize) {
        if matched < self.stack.len() {
            self.stack.truncate(matched);
            self.leaf = Leaf::None;
        }
    }

    fn quote_depth_at(&self, depth: usize) -> usize {
        self.stack[..depth.min(self.stack.len())]
            .iter()
            .filter(|c| matches!(c, Container::Quote { .. }))
            .count()
    }

    fn info(
        &self,
        kind: Kind,
        item: Option<ListItem>,
        b: &[u8],
        prefix_end: usize,
        text_start: usize,
        lazy: bool,
    ) -> LineInfo {
        debug_assert!(text_start <= b.len());
        LineInfo {
            kind,
            depth: self.stack.len(),
            quote_depth: self.quote_depth_at(self.stack.len()),
            item,
            content_column: self.stack.last().map_or(0, Container::content_column),
            prefix_end,
            text_start,
            lazy,
        }
    }
}

/// A leaf block that starts at `p`, `indent` columns into its container,
/// with the byte offset of its text. `paragraph` is the first line of the
/// paragraph this line could continue.
fn leaf_start(
    text: &str,
    p: usize,
    indent: usize,
    paragraph: Option<usize>,
    maybe_lazy: bool,
) -> Option<(Kind, usize)> {
    let b = text.as_bytes();
    if indent >= 4 {
        return (!maybe_lazy).then_some((Kind::IndentedCode, p));
    }
    if let Some(start) = paragraph {
        if let Some(level) = setext_level(b, p) {
            return Some((Kind::SetextUnderline { level, start }, p));
        }
    }
    if is_thematic_break(b, p) {
        return Some((Kind::ThematicBreak, p));
    }
    if let Some((level, title)) = atx_heading(text, p) {
        let start = title.start;
        return Some((Kind::Heading { level, text: title }, start));
    }
    if let Some((fence, info)) = fence_open(text, p) {
        return Some((Kind::FenceOpen { fence, info }, p));
    }
    if html_start(&text[p..]).is_some() {
        return Some((Kind::Html, p));
    }
    None
}

fn trim_end_ws(s: &str) -> &str {
    s.trim_end_matches([' ', '\t'])
}

fn skip_ws(b: &[u8], mut pos: usize, mut col: usize) -> (usize, usize) {
    while let Some(&ch) = b.get(pos) {
        match ch {
            b' ' => col += 1,
            b'\t' => col = next_tab_stop(col),
            _ => break,
        }
        pos += 1;
    }
    (pos, col)
}

/// Consume whitespace up to column `target`. A tab that straddles `target`
/// is consumed whole; callers measure indentation in absolute columns, so
/// the columns it spills past `target` still count.
fn advance_to(b: &[u8], mut pos: usize, mut col: usize, target: usize) -> (usize, usize) {
    while col < target {
        match b.get(pos) {
            Some(b' ') => col += 1,
            Some(b'\t') => col = next_tab_stop(col),
            _ => break,
        }
        pos += 1;
    }
    (pos, col)
}

/// The content column of a quote whose `>` is at byte `p`, column `c`.
fn quote_content(b: &[u8], p: usize, c: usize) -> usize {
    match b.get(p + 1) {
        Some(b' ' | b'\t') => c + 2,
        _ => c + 1,
    }
}

fn is_thematic_break(b: &[u8], p: usize) -> bool {
    let Some(&ch @ (b'-' | b'*' | b'_')) = b.get(p) else {
        return false;
    };
    let mut count = 0;
    for &x in &b[p..] {
        if x == ch {
            count += 1;
        } else if x != b' ' && x != b'\t' {
            return false;
        }
    }
    count >= 3
}

fn setext_level(b: &[u8], p: usize) -> Option<u8> {
    let level = match b.get(p)? {
        b'=' => 1,
        b'-' => 2,
        _ => return None,
    };
    let run = b[p..].iter().take_while(|&&x| x == b[p]).count();
    b[p + run..]
        .iter()
        .all(|&x| x == b' ' || x == b'\t')
        .then_some(level)
}

fn atx_heading(text: &str, p: usize) -> Option<(u8, Range<usize>)> {
    let b = text.as_bytes();
    let hashes = b[p..].iter().take_while(|&&x| x == b'#').count();
    if !(1..=6).contains(&hashes) || !matches!(b.get(p + hashes), None | Some(b' ' | b'\t')) {
        return None;
    }
    let start = skip_ws(b, p + hashes, 0).0;
    let content = trim_end_ws(&text[start..]);
    let without_closing = content.trim_end_matches('#');
    let end = if without_closing.is_empty() {
        start
    } else if without_closing.len() < content.len() && without_closing.ends_with([' ', '\t']) {
        start + trim_end_ws(without_closing).len()
    } else {
        start + content.len()
    };
    Some((hashes as u8, start..end))
}

fn fence_open(text: &str, p: usize) -> Option<(Fence, Range<usize>)> {
    let b = text.as_bytes();
    let ch = *b.get(p)?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = b[p..].iter().take_while(|&&x| x == ch).count();
    if len < 3 {
        return None;
    }
    let info_start = skip_ws(b, p + len, 0).0;
    let info = trim_end_ws(&text[info_start..]);
    if ch == b'`' && info.contains('`') {
        return None;
    }
    let fence = Fence {
        ch: ch as char,
        len,
    };
    Some((fence, info_start..info_start + info.len()))
}

fn is_fence_close(b: &[u8], pos: usize, col: usize, base: usize, fence: Fence) -> bool {
    let (p, c) = skip_ws(b, pos, col);
    if c.saturating_sub(base) > 3 {
        return false;
    }
    let run = b[p..].iter().take_while(|&&x| x == fence.ch as u8).count();
    run >= fence.len && b[p + run..].iter().all(|&x| x == b' ' || x == b'\t')
}

/// CommonMark's block-level HTML tag names (start condition 6).
const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

/// Tags whose block runs to their closing tag, blank lines included
/// (CommonMark start condition 1).
const RAW_TAGS: &[(&str, &str)] = &[
    ("pre", "</pre>"),
    ("script", "</script>"),
    ("style", "</style>"),
    ("textarea", "</textarea>"),
];

fn html_start(rest: &str) -> Option<HtmlEnd> {
    if rest.starts_with("<!--") {
        return Some(HtmlEnd::Comment);
    }
    let tag = rest.strip_prefix('<')?;
    let (closing, tag) = match tag.strip_prefix('/') {
        Some(tag) => (true, tag),
        None => (false, tag),
    };
    let name_len = tag.bytes().take_while(u8::is_ascii_alphanumeric).count();
    let (name, after) = tag.split_at(name_len);
    if name.is_empty() {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    if !closing {
        if let Some(&(_, end)) = RAW_TAGS.iter().find(|(raw, _)| *raw == lower) {
            if after.is_empty() || after.starts_with([' ', '\t', '>']) {
                return Some(HtmlEnd::Tag(end));
            }
        }
    }
    let ends_name =
        after.is_empty() || after.starts_with([' ', '\t', '>']) || after.starts_with("/>");
    (ends_name && BLOCK_TAGS.contains(&lower.as_str())).then_some(HtmlEnd::Blank)
}

fn html_ends(rest: &str, end: HtmlEnd) -> bool {
    match end {
        HtmlEnd::Blank => false,
        HtmlEnd::Comment => rest.contains("-->"),
        HtmlEnd::Tag(tag) => rest.to_ascii_lowercase().contains(tag),
    }
}

/// A list item whose marker starts at byte `p`, column `c`, with the byte
/// and column where its content starts.
fn list_marker(
    text: &str,
    p: usize,
    c: usize,
    in_paragraph: bool,
    quote_depth: usize,
) -> Option<(ListItem, usize, usize)> {
    let b = text.as_bytes();
    let (marker, end) = match b[p] {
        ch @ (b'-' | b'*' | b'+') => (Marker::Bullet(ch as char), p + 1),
        b'0'..=b'9' => {
            let width = b[p..].iter().take_while(|x| x.is_ascii_digit()).count();
            let delimiter = *b.get(p + width)?;
            if width > 9 || !matches!(delimiter, b'.' | b')') {
                return None;
            }
            let number = text[p..p + width].parse().ok()?;
            let marker = Marker::Ordered {
                number,
                width,
                delimiter: delimiter as char,
            };
            (marker, p + width + 1)
        }
        _ => return None,
    };
    if !matches!(b.get(end), None | Some(b' ' | b'\t')) {
        return None;
    }
    let end_col = c + (end - p);
    let (q, qc) = skip_ws(b, end, end_col);
    let empty = q == b.len();
    if in_paragraph && (empty || matches!(marker, Marker::Ordered { number, .. } if number != 1)) {
        return None;
    }
    let (content_column, pos, col) = if empty || qc - end_col > 4 {
        let (pos, col) = advance_to(b, end, end_col, end_col + 1);
        (end_col + 1, pos, col)
    } else {
        (qc, q, qc)
    };
    let task = (!empty && col == content_column)
        .then(|| task_box(b, pos))
        .flatten();
    let text_start = match task {
        Some(_) => skip_ws(b, pos + 3, 0).0,
        None => skip_ws(b, pos, col).0,
    };
    let item = ListItem {
        indent: c,
        content_column,
        marker_start: p,
        marker_end: end,
        text_start,
        marker,
        task,
        quote_depth,
    };
    Some((item, pos, col))
}

fn task_box(b: &[u8], pos: usize) -> Option<Task> {
    let done = match b.get(pos..pos + 3)? {
        b"[ ]" => false,
        b"[x]" | b"[X]" => true,
        _ => return None,
    };
    matches!(b.get(pos + 3), None | Some(b' ' | b'\t')).then_some(Task { done, start: pos })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    /// A compact code for a line: the container depth (when not 0), a `>`
    /// per quote, the marker of an item opened on the line (with its task
    /// box), then the kind. `~` marks a lazy line.
    fn code(info: &LineInfo) -> String {
        let mut s = String::new();
        if info.depth > 0 {
            s.push_str(&info.depth.to_string());
        }
        s.push_str(&">".repeat(info.quote_depth));
        if let Some(item) = info.item {
            match item.marker {
                Marker::Bullet(ch) => s.push(ch),
                Marker::Ordered {
                    number,
                    width,
                    delimiter,
                } => s.push_str(&format!("{number:0width$}{delimiter}")),
            }
            match item.task {
                Some(Task { done: true, .. }) => s.push_str("[x]"),
                Some(Task { done: false, .. }) => s.push_str("[ ]"),
                None => {}
            }
        }
        if info.lazy {
            s.push('~');
        }
        s.push_str(&match &info.kind {
            Kind::Blank => "B".into(),
            Kind::Prose => "P".into(),
            Kind::Heading { level, .. } => format!("H{level}"),
            Kind::SetextUnderline { level, start } => format!("S{level}@{start}"),
            Kind::ThematicBreak => "TB".into(),
            Kind::FenceOpen { .. } => "FO".into(),
            Kind::FenceClose => "FC".into(),
            Kind::FenceBody => "FB".into(),
            Kind::IndentedCode => "IC".into(),
            Kind::FrontMatter => "FM".into(),
            Kind::Html => "HT".into(),
        });
        s
    }

    fn codes(text: &str) -> Vec<String> {
        scan(&Document::new(text)).iter().map(code).collect()
    }

    const ENDINGS: [&str; 4] = ["\n", "\r\n", "\r", "\n\r"];

    /// Join lines with `endings`, cycling. The cycle order never lets one
    /// ending run into the next and read as a different one.
    fn join(lines: &[&str], endings: &[&str]) -> String {
        let mut text = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                text.push_str(endings[(i - 1) % endings.len()]);
            }
            text.push_str(line);
        }
        text
    }

    /// Check each case under every line ending, mixed endings, and with a
    /// trailing line ending (which adds one blank line).
    #[track_caller]
    fn check(cases: &[(&str, &[&str])]) {
        for (doc, expected) in cases {
            let lines: Vec<&str> = doc.split('\n').collect();
            let mixed = ["\r\n", "\n", "\n\r", "\r"];
            let variants = ENDINGS
                .iter()
                .map(|e| (*e, join(&lines, &[e])))
                .chain([("mixed", join(&lines, &mixed))]);
            for (name, text) in variants {
                let got = codes(&text);
                assert_eq!(got, *expected, "{doc:?} with {name:?} endings");
                let ending = if name == "mixed" { "\r" } else { name };
                let trailing = codes(&format!("{text}{ending}"));
                assert_eq!(&trailing[..expected.len()], *expected, "{doc:?} + {name:?}");
                assert_eq!(trailing.len(), expected.len() + 1, "{doc:?} + {name:?}");
                // Blank, or a blank line inside an unclosed fence.
                assert!(
                    trailing.last().unwrap().ends_with('B'),
                    "{doc:?} + {name:?}"
                );
            }
        }
    }

    fn infos(text: &str) -> Vec<LineInfo> {
        scan(&Document::new(text))
    }

    #[test]
    fn columns_expand_tabs_to_stops_of_four() {
        assert_eq!(next_tab_stop(0), 4);
        assert_eq!(next_tab_stop(3), 4);
        assert_eq!(next_tab_stop(4), 8);
        assert_eq!(column("a\tb", 2), 4);
        assert_eq!(column("  \tb", 3), 4);
        assert_eq!(column("é\t", 2), 1);
        assert_eq!(column("é\t", 3), 4);
        assert_eq!(column("\u{feff}ab", 4), 1);
        assert_eq!(advance_column(2, "\t"), 4);
        assert_eq!(indentation(" \t x"), (3, 5));
        assert_eq!(indentation("x"), (0, 0));
        assert_eq!(indentation("\u{a0}x"), (0, 0));
        assert_eq!(byte_at_column("\tab", 4), 1);
        assert_eq!(byte_at_column("\tab", 2), 1);
        assert_eq!(byte_at_column("éab", 1), 2);
        assert_eq!(byte_at_column("ab", 9), 2);
    }

    #[test]
    fn empty_document_is_one_blank_line() {
        assert_eq!(codes(""), ["B"]);
        assert_eq!(codes("   "), ["B"]);
        assert_eq!(codes("\t"), ["B"]);
    }

    #[test]
    fn prose_blank_and_paragraphs() {
        check(&[
            ("a\nb\n\nc", &["P", "P", "B", "P"]),
            ("  a\n\t", &["P", "B"]),
            ("\u{a0}a", &["P"]),
        ]);
    }

    #[test]
    fn atx_headings() {
        check(&[
            ("# a", &["H1"]),
            ("###### a", &["H6"]),
            ("####### a", &["P"]),
            ("#a", &["P"]),
            ("#\ta", &["H1"]),
            ("#", &["H1"]),
            ("# ", &["H1"]),
            ("   # a", &["H1"]),
            ("    # a", &["IC"]),
            ("\\# a", &["P"]),
            ("a\n# b", &["P", "H1"]),
        ]);
    }

    #[test]
    fn atx_heading_text_ranges_strip_closing_hashes() {
        let cases: &[(&str, &str)] = &[
            ("# Title", "Title"),
            ("#   Title   ", "Title"),
            ("## Title ##", "Title"),
            ("## Title ##   ", "Title"),
            ("# Title#", "Title#"),
            ("# Title \\#", "Title \\#"),
            ("# #", ""),
            ("### ###", ""),
            ("#", ""),
            ("# # Title", "# Title"),
            ("  ## é ##", "é"),
        ];
        for (line, title) in cases {
            let info = &infos(line)[0];
            let Kind::Heading { text, .. } = &info.kind else {
                panic!("{line:?} is not a heading");
            };
            assert_eq!(&line[text.clone()], *title, "{line:?}");
            assert_eq!(info.text_start, text.start, "{line:?}");
        }
    }

    #[test]
    fn setext_underlines() {
        check(&[
            ("a\n===", &["P", "S1@0"]),
            ("a\n---", &["P", "S2@0"]),
            ("a\n-", &["P", "S2@0"]),
            ("a\n=", &["P", "S1@0"]),
            ("a\nb\n---", &["P", "P", "S2@0"]),
            ("a\n   ---  ", &["P", "S2@0"]),
            ("a\n    ---", &["P", "P"]),
            ("a\n- - -", &["P", "TB"]),
            ("a\n***", &["P", "TB"]),
            ("a\n= =", &["P", "P"]),
            ("\n===", &["B", "P"]),
            ("# h\n===", &["H1", "P"]),
            ("x\n\na\nb\n===", &["P", "B", "P", "P", "S1@2"]),
        ]);
    }

    #[test]
    fn a_list_item_then_dashes_is_a_thematic_break() {
        check(&[
            ("- foo\n---", &["1-P", "TB"]),
            ("- foo\n  ---", &["1-P", "1S2@0"]),
            ("> foo\n---", &["1>P", "TB"]),
            ("> foo\n===", &["1>P", "1>~P"]),
        ]);
    }

    #[test]
    fn thematic_breaks() {
        check(&[
            ("---", &["TB"]),
            ("***", &["TB"]),
            ("___", &["TB"]),
            ("- - -", &["TB"]),
            ("* * *", &["TB"]),
            (" **  * ** * ** * **", &["TB"]),
            ("-\t-\t-", &["TB"]),
            ("   ---", &["TB"]),
            ("    ---", &["IC"]),
            ("--", &["P"]),
            ("---a", &["P"]),
            ("*-*", &["P"]),
            ("- ***", &["1-TB"]),
        ]);
    }

    #[test]
    fn bullet_and_ordered_list_items() {
        check(&[
            ("- a", &["1-P"]),
            ("* a", &["1*P"]),
            ("+ a", &["1+P"]),
            ("-a", &["P"]),
            ("1. a", &["11.P"]),
            ("1) a", &["11)P"]),
            ("0. a", &["10.P"]),
            ("09. a", &["109.P"]),
            ("123456789. a", &["1123456789.P"]),
            ("1234567890. a", &["P"]),
            ("1.a", &["P"]),
            ("1: a", &["P"]),
            ("-", &["1-B"]),
            ("- ", &["1-B"]),
            ("1.", &["11.B"]),
            ("- a\n- b", &["1-P", "1-P"]),
            ("- a\n* b", &["1-P", "1*P"]),
            ("1. a\n2. b", &["11.P", "12.P"]),
        ]);
    }

    #[test]
    fn list_item_columns() {
        let item = |line: &str| infos(line)[0].item.unwrap();
        let i = item("- a");
        assert_eq!((i.indent, i.content_column, i.text_start), (0, 2, 2));
        assert_eq!((i.marker_start, i.marker_end), (0, 1));
        let i = item("  10.   a");
        assert_eq!((i.indent, i.content_column, i.text_start), (2, 8, 8));
        assert_eq!(
            i.marker,
            Marker::Ordered {
                number: 10,
                width: 2,
                delimiter: '.'
            }
        );
        // Five spaces after the marker: the content starts one past it, and
        // the rest is indented code.
        let info = &infos("-     code")[0];
        let i = info.item.unwrap();
        assert_eq!((i.content_column, info.prefix_end), (2, 2));
        assert_eq!(info.kind, Kind::IndentedCode);
        assert_eq!(info.text_start, 6);
        // An empty item's content column is one past the marker.
        assert_eq!(item("-").content_column, 2);
        assert_eq!(item("1.  ").content_column, 3);
        // A tab after the marker counts to the next stop.
        let i = item("-\ta");
        assert_eq!((i.content_column, i.text_start), (4, 2));
        let i = item(" -\ta");
        assert_eq!(i.content_column, 4);
    }

    #[test]
    fn task_items() {
        check(&[
            ("- [ ] a", &["1-[ ]P"]),
            ("- [x] a", &["1-[x]P"]),
            ("- [X] a", &["1-[x]P"]),
            ("1. [x] a", &["11.[x]P"]),
            ("1) [ ] a", &["11)[ ]P"]),
            ("- [ ] ", &["1-[ ]B"]),
            ("- [ ]", &["1-[ ]B"]),
            ("- [ ]a", &["1-P"]),
            ("- [y] a", &["1-P"]),
            ("- [  ] a", &["1-P"]),
            ("[ ] a", &["P"]),
            ("- [ ] # a", &["1-[ ]P"]),
            ("-  [ ] a", &["1-[ ]P"]),
        ]);
        let info = &infos("  - [x]  done")[0];
        let item = info.item.unwrap();
        assert_eq!(
            item.task,
            Some(Task {
                done: true,
                start: 4
            })
        );
        assert_eq!((item.text_start, info.text_start), (9, 9));
        assert_eq!(info.prefix_end, 4);
    }

    #[test]
    fn list_continuation_and_nesting() {
        check(&[
            ("- a\n  b", &["1-P", "1P"]),
            ("- a\nb", &["1-P", "1~P"]),
            ("- a\n\n  b", &["1-P", "1B", "1P"]),
            ("- a\n\nb", &["1-P", "1B", "P"]),
            ("- a\n  - b", &["1-P", "2-P"]),
            ("- a\n  - b\n    c\n  d", &["1-P", "2-P", "2P", "2~P"]),
            ("- a\n  - b\n\n  d", &["1-P", "2-P", "2B", "1P"]),
            ("- a\n  - b\n- c", &["1-P", "2-P", "1-P"]),
            ("1. a\n   - b", &["11.P", "2-P"]),
            ("1. a\n  - b", &["11.P", "1-P"]),
            ("- - a", &["2-P"]),
            ("- a\n - b", &["1-P", "1-P"]),
            ("-\n  a", &["1-B", "1P"]),
            ("-\n\n  a", &["1-B", "B", "P"]),
            ("- a\n\n\n  b", &["1-P", "1B", "1B", "1P"]),
        ]);
    }

    #[test]
    fn list_items_interrupting_paragraphs() {
        check(&[
            ("a\n- b", &["P", "1-P"]),
            ("a\n1. b", &["P", "11.P"]),
            ("a\n2. b", &["P", "P"]),
            ("a\n-", &["P", "S2@0"]),
            ("a\n- ", &["P", "S2@0"]),
            ("a\n1.", &["P", "P"]),
            ("a\n\n2. b", &["P", "B", "12.P"]),
            ("1. a\n   2. b", &["11.P", "1P"]),
            ("1. a\n\n   2. b", &["11.P", "1B", "22.P"]),
            ("- a\n-", &["1-P", "1-B"]),
            ("> a\n2. b", &["1>P", "12.P"]),
        ]);
    }

    #[test]
    fn deep_nesting_and_tabbed_indentation() {
        check(&[
            ("\t- a\n\t  b", &["IC", "IC"]),
            ("- a\n\tb", &["1-P", "1P"]),
            ("-\ta\n\tb", &["1-P", "1P"]),
            ("1.\ta\n\tb", &["11.P", "1P"]),
            ("- a\n\t- b", &["1-P", "2-P"]),
            ("> a\n>\tb", &["1>P", "1>P"]),
            (
                "> > - [ ] a\n> >   b\n> > - c",
                &["3>>-[ ]P", "3>>P", "3>>-P"],
            ),
            (
                "- a\n  > b\n  > - c\n  >   d\n  e",
                &["1-P", "2>P", "3>-P", "3>P", "3>~P"],
            ),
            (
                "1. a\n   1. b\n      1. c\n         d",
                &["11.P", "21.P", "31.P", "3P"],
            ),
            ("* a\n\n  * b\n\n    c", &["1*P", "1B", "2*P", "2B", "2P"]),
        ]);
    }

    #[test]
    fn blockquotes() {
        check(&[
            ("> a", &["1>P"]),
            (">a", &["1>P"]),
            ("   > a", &["1>P"]),
            ("    > a", &["IC"]),
            (">", &["1>B"]),
            ("> a\n> b", &["1>P", "1>P"]),
            ("> a\nb", &["1>P", "1>~P"]),
            ("> a\n\nb", &["1>P", "B", "P"]),
            ("> a\n>\n> b", &["1>P", "1>B", "1>P"]),
            ("> > a", &["2>>P"]),
            ("> > a\n> b", &["2>>P", "2>>~P"]),
            ("> > a\n>\n> b", &["2>>P", "1>B", "1>P"]),
            ("> - a\n> - b", &["2>-P", "2>-P"]),
            ("> - a\n>   b", &["2>-P", "2>P"]),
            ("> - a\n> b", &["2>-P", "2>~P"]),
            ("- > a\n  > b", &["2>-P", "2>P"]),
            ("> # h", &["1>H1"]),
            ("> a\n# h", &["1>P", "H1"]),
            ("> a\n- b", &["1>P", "1-P"]),
            // A bare `>` is a blank line inside the quote, which a list item
            // in the quote continues over.
            ("> - a\n>\n>   b", &["2>-P", "2>B", "2>P"]),
            ("> - a\n>\n> b", &["2>-P", "2>B", "1>P"]),
            ("> -\n>\n>   a", &["2>-B", "1>B", "1>P"]),
        ]);
    }

    #[test]
    fn quote_columns_and_prefixes() {
        let text = "> > a\n>> b\n> - c\n>   d";
        let infos = infos(text);
        let lines: Vec<&str> = text.split('\n').collect();
        let prefixes: Vec<&str> = infos
            .iter()
            .zip(&lines)
            .map(|(info, line)| container_prefix(line, info))
            .collect();
        assert_eq!(prefixes, ["> > ", ">> ", "> - ", ">   "]);
        assert_eq!(infos[0].content_column, 4);
        assert_eq!(infos[1].content_column, 3);
        assert_eq!(infos[2].item.unwrap().quote_depth, 1);
        assert_eq!(infos[2].item.unwrap().indent, 2);
        assert_eq!(infos[3].content_column, 4);
        assert_eq!(infos[3].text_start, 4);
    }

    #[test]
    fn tabs_after_quote_markers_count_by_column() {
        // `>` then a tab: one column of the tab is the optional space, and
        // the other two are indentation.
        check(&[
            (">\ta", &["1>P"]),
            (">\t\ta", &["1>IC"]),
            (">\t  a", &["1>IC"]),
            ("> \t a", &["1>P"]),
        ]);
        let info = &infos(">\ta")[0];
        assert_eq!(
            (info.prefix_end, info.text_start, info.content_column),
            (2, 2, 2)
        );
    }

    #[test]
    fn fenced_code() {
        check(&[
            ("```\ncode\n```", &["FO", "FB", "FC"]),
            ("~~~\ncode\n~~~", &["FO", "FB", "FC"]),
            ("```rust\n# no\n```\n# yes", &["FO", "FB", "FC", "H1"]),
            ("``\nx", &["P", "P"]),
            ("````\n```\n````", &["FO", "FB", "FC"]),
            ("```\n~~~\n```", &["FO", "FB", "FC"]),
            ("```\n```` \nx", &["FO", "FC", "P"]),
            ("```\n``` x\n```", &["FO", "FB", "FC"]),
            ("   ```\n   ```", &["FO", "FC"]),
            ("    ```", &["IC"]),
            ("```\n    ```\n```", &["FO", "FB", "FC"]),
            ("```\n\n```", &["FO", "FB", "FC"]),
            (
                "```\nunclosed\n\n# not a heading",
                &["FO", "FB", "FB", "FB"],
            ),
            ("a\n```\nb\n```", &["P", "FO", "FB", "FC"]),
            ("``` a`b", &["P"]),
            ("```rust``` is fine", &["P"]),
            ("~~~ a`b\nx\n~~~", &["FO", "FB", "FC"]),
        ]);
        let info = &infos("```  rust  ")[0];
        let Kind::FenceOpen { fence, info: range } = &info.kind else {
            panic!("not a fence");
        };
        assert_eq!(*fence, Fence { ch: '`', len: 3 });
        assert_eq!(&"```  rust  "[range.clone()], "rust");
    }

    #[test]
    fn fences_in_containers_are_relative_to_the_content_column() {
        check(&[
            ("- ```\n  code\n  ```\n- b", &["1-FO", "1FB", "1FC", "1-P"]),
            (
                "- a\n\n     ```\n     x\n     ```",
                &["1-P", "1B", "1FO", "1FB", "1FC"],
            ),
            ("- a\n\n      ```", &["1-P", "1B", "1IC"]),
            ("> ```\n> # x\n> ```", &["1>FO", "1>FB", "1>FC"]),
            // An unclosed fence ends with its container.
            ("> ```\n> x\ny", &["1>FO", "1>FB", "P"]),
            ("- ```\n  x\ny", &["1-FO", "1FB", "P"]),
            ("> ```\n\n# h", &["1>FO", "B", "H1"]),
            ("- ```\n\n  x", &["1-FO", "1FB", "1FB"]),
            ("1. ```\n   ```", &["11.FO", "1FC"]),
        ]);
    }

    #[test]
    fn indented_code() {
        check(&[
            ("    code", &["IC"]),
            ("\tcode", &["IC"]),
            ("  \tcode", &["IC"]),
            ("a\n    b", &["P", "P"]),
            ("a\n\n    b", &["P", "B", "IC"]),
            ("    a\n\n    b\nc", &["IC", "B", "IC", "P"]),
            ("# h\n    b", &["H1", "IC"]),
            ("- a\n\n      b", &["1-P", "1B", "1IC"]),
            ("- a\n      b", &["1-P", "1P"]),
            ("> a\n    b", &["1>P", "1>~P"]),
            (">     a", &["1>IC"]),
        ]);
    }

    #[test]
    fn front_matter() {
        check(&[
            ("---\ntitle: x\n---\n# h", &["FM", "FM", "FM", "H1"]),
            ("---\ntitle: x\n...\n# h", &["FM", "FM", "FM", "H1"]),
            ("---   \ntags:\n  - a\n---", &["FM", "FM", "FM", "FM"]),
            ("---\n\n# comment\n---", &["FM", "FM", "FM", "FM"]),
            ("---\na-b_C9: x\n---", &["FM", "FM", "FM"]),
            ("---\nnot yaml\n---", &["TB", "P", "S2@1"]),
            ("---\ntitle: x", &["TB", "P"]),
            ("---\n---", &["TB", "TB"]),
            ("---\ntitle:x\n---", &["TB", "P", "S2@1"]),
            ("  ---\ntitle: x\n---", &["TB", "P", "S2@1"]),
            ("# h\n---\ntitle: x\n---", &["H1", "TB", "P", "S2@2"]),
        ]);
        assert_eq!(codes("\u{feff}---\nk: v\n---\nx"), ["FM", "FM", "FM", "P"]);
        assert_eq!(front_matter_end(&["---", "k: v", "---"]), Some(2));
        assert_eq!(front_matter_end(&[""]), None);
    }

    #[test]
    fn html_blocks() {
        check(&[
            ("<div>\n*a*\n\nb", &["HT", "HT", "B", "P"]),
            ("<DIV class=x>\na", &["HT", "HT"]),
            ("</div>\na", &["HT", "HT"]),
            ("<details>\n# x\n\n# y", &["HT", "HT", "B", "H1"]),
            ("<hr/>\n\na", &["HT", "B", "P"]),
            ("<!-- a -->\n# b", &["HT", "H1"]),
            ("<!--\n# a\n\n-->\n# b", &["HT", "HT", "HT", "HT", "H1"]),
            ("<pre>\n# a\n\n</pre>\n# b", &["HT", "HT", "HT", "HT", "H1"]),
            ("<pre>x</pre>\n# b", &["HT", "H1"]),
            ("<span>\na", &["P", "P"]),
            ("<divx>", &["P"]),
            ("<", &["P"]),
            ("a\n<div>\nb", &["P", "HT", "HT"]),
            ("> <div>\n> # a\n# b", &["1>HT", "1>HT", "H1"]),
            ("> <div>\n>\n> # a", &["1>HT", "1>B", "1>H1"]),
            ("    <div>", &["IC"]),
        ]);
    }

    #[test]
    fn code_like_kinds() {
        let infos = infos("---\nk: v\n---\n```\nx\n```\n    y\n\n<div>\n\n# h\n- a");
        let code_like: Vec<bool> = infos.iter().map(|i| i.kind.is_code_like()).collect();
        assert_eq!(
            code_like,
            [true, true, true, true, true, true, true, false, true, false, false, false]
        );
    }

    #[test]
    fn byte_order_mark_is_ignored_on_the_first_line() {
        check(&[
            ("\u{feff}# a", &["H1"]),
            ("\u{feff}- a", &["1-P"]),
            ("\u{feff}    a", &["IC"]),
            ("\u{feff}```\nx\n```", &["FO", "FB", "FC"]),
        ]);
        let info = &infos("\u{feff}- a")[0];
        assert_eq!(info.item.unwrap().marker_start, 3);
        assert_eq!(info.text_start, 5);
        assert_eq!(codes("a\n\u{feff}# b"), ["P", "P"]);
    }

    #[test]
    fn a_scanner_fed_line_by_line_matches_scan_given_the_front_matter_end() {
        let docs = [
            "---\ntitle: x\n---\n# H",
            "\u{feff}---\n# comment\nk: v\n...\n- a\n---",
            "---\ntitle: x\nmore: y",
            "---\nnot yaml\n---\n# H",
            "# H\n---\ntitle: x\n---",
        ];
        for text in docs {
            let lines: Vec<&str> = text.split('\n').collect();
            let full = scan(&lines[..]);
            let end = front_matter_end(&lines[..]);
            let mut scanner = Scanner::with_front_matter(end);
            for (i, line) in lines.iter().enumerate() {
                assert_eq!(scanner.next_line(line), full[i], "{text:?} line {i}");
                let mut resumed = scanner.clone();
                assert_eq!(resumed.front_matter_end(), end);
                let rest: Vec<LineInfo> = lines[i + 1..]
                    .iter()
                    .map(|line| resumed.next_line(line))
                    .collect();
                assert_eq!(rest, full[i + 1..], "{text:?} resumed after {i}");
            }
        }
        // Without the end, line 0 cannot know a closing `---` follows: the
        // front matter reads as a break, prose, and a setext heading.
        let lines = ["---", "title: x", "---", "# H"];
        let mut blind = Scanner::new();
        let kinds: Vec<String> = lines.iter().map(|l| code(&blind.next_line(l))).collect();
        assert_eq!(kinds, ["TB", "P", "S2@1", "H1"]);
    }

    #[test]
    fn scanner_state_can_be_checkpointed_and_resumed() {
        let text = "> - a\n>   ```\n>   x\n>   ```\n> b\n\n1. c\n   d";
        let lines: Vec<&str> = text.split('\n').collect();
        let full: Vec<LineInfo> = scan(&lines[..]);
        for split in 0..lines.len() {
            let mut scanner = Scanner::new();
            for line in &lines[..split] {
                scanner.next_line(line);
            }
            let mut checkpoint = scanner.clone();
            assert_eq!(checkpoint.line_index(), split);
            let rest: Vec<LineInfo> = lines[split..]
                .iter()
                .map(|line| checkpoint.next_line(line))
                .collect();
            assert_eq!(rest, full[split..], "resumed at line {split}");
        }
    }

    #[test]
    fn containers_report_the_open_stack() {
        let mut scanner = Scanner::new();
        scanner.next_line("> - a");
        let stack = scanner.containers();
        assert_eq!(stack.len(), 2);
        assert_eq!(
            stack[0],
            Container::Quote {
                column: 0,
                content_column: 2
            }
        );
        assert!(matches!(stack[1], Container::Item(item) if item.content_column == 4));
        scanner.next_line("");
        assert!(scanner.containers().is_empty());
    }

    #[test]
    fn continued_counts_the_containers_a_line_kept() {
        let mut scanner = Scanner::new();
        let continued: Vec<(usize, usize)> = ["> - a", ">   b", "> - c", "> d", "# e"]
            .iter()
            .map(|line| {
                scanner.next_line(line);
                (scanner.continued(), scanner.containers().len())
            })
            .collect();
        // `> d` is lazy: it keeps the item open without continuing it.
        assert_eq!(continued, [(0, 2), (2, 2), (1, 2), (1, 2), (0, 0)]);
    }

    #[test]
    fn segments_split_the_prefix_by_container() {
        let seg = |start, end, marker| Segment { start, end, marker };
        let segs_of = |text: &str| {
            let doc = Document::new(text);
            let mut scanner = Scanner::new();
            let mut last = Vec::new();
            for i in 0..doc.line_count() {
                scanner.next_line(doc.line(i));
                let stack = scanner.containers().to_vec();
                last = segments(doc.line(i), i, &stack, scanner.continued());
            }
            last
        };
        assert_eq!(segs_of("> - a"), [seg(0, 2, Some(0)), seg(2, 4, Some(2))]);
        assert_eq!(
            segs_of("> - a\n>   b"),
            [seg(0, 2, Some(0)), seg(2, 4, None)]
        );
        assert_eq!(segs_of("  1.  a"), [seg(0, 6, Some(2))]);
        assert_eq!(segs_of(">>a"), [seg(0, 1, Some(0)), seg(1, 2, Some(1))]);
        assert_eq!(segs_of("- a\n\tb"), [seg(0, 1, None)]);
        assert_eq!(segs_of("- a\n"), [seg(0, 0, None)]);
        assert_eq!(segs_of("-"), [seg(0, 1, Some(0))]);
        assert_eq!(segs_of("\u{feff}> a"), [seg(3, 5, Some(3))]);
    }

    #[test]
    fn empty_items_report_as_empty() {
        let infos = infos("- \n1.\n- [ ]\n- a\n> ");
        let empty: Vec<bool> = infos.iter().map(LineInfo::is_empty_item).collect();
        assert_eq!(empty, [true, true, true, false, false]);
    }

    #[test]
    fn non_ascii_text_keeps_byte_offsets_on_char_boundaries() {
        let text = "é\n- ü\n> ✓ x\n# ñ #\n  ```日本\n  語\n  ```";
        for (info, line) in infos(text).iter().zip(text.split('\n')) {
            assert!(line.is_char_boundary(info.prefix_end), "{line:?}");
            assert!(line.is_char_boundary(info.text_start), "{line:?}");
        }
    }
}
