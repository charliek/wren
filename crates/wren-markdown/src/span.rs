//! Typed spans: what the byte ranges of a line are.
//!
//! [`spans`] reads a classified line ([`LineInfo`]) into the spans that lie
//! on it: its inline Markdown (code spans, comments, links, bare URLs,
//! autolinks, strikes and emphasis) and, for a host whose tasks are on, a
//! task line's box and tokens ([`crate::task`]). It is the one reading of
//! what is inside a line, and everything that asks reads it:
//!
//! - a highlighter turns the spans into colours and faces;
//! - spell check reads the prose they leave ([`LineSpans::prose`]);
//! - a click, a right-click or the caret finds the span it is on
//!   ([`LineSpans::at`]).
//!
//! The crate decides what a range *is*; how it looks is its reader's.
//!
//! # Inline Markdown
//!
//! Code spans and HTML comments are found first and hold nothing else.
//! Links and bare URLs come next, so emphasis never pairs the `_` or `*` of
//! a URL. Strikes and emphasis are matched inside each link's text, and
//! then across the line, where they can wrap whole links. So spans lie
//! inside one another: a strike holds emphasis, emphasis holds a link, a
//! link's text holds a code span. They are no tree, though. Each strike
//! and emphasis pairs its own delimiters, so the two of `~~a *b~~ c*`
//! cross, and a task token is read by its own rules, which look at no
//! inline Markdown.
//!
//! Markup that only looks like a construct is no span: an autolink's
//! angle brackets over code or a comment (`` `<a@b>` `` is code), and the
//! `[b](c)` of `[a][b](c)`, whose `[b]` is the first link's label. Spell
//! check has always skipped such bytes, so they stay out of the prose
//! ([`LineSpans::not_prose`]) without being anything a pointer can find.
//!
//! # Tasks
//!
//! With [`Boxes::Status`], a line whose list item opens with a task box
//! also has a [`Box`](SpanKind::Box) span and a span for each thing
//! [`task::parse`] reads on it: its owners, its tags and its tokens. A host
//! with tasks off scans with [`Boxes::Plain`] and gets none of them, so
//! its lines read as they always have.

use std::cell::Cell;
use std::ops::Range;

use crate::block::{Boxes, Kind, LineInfo, Task};
use crate::date::Parsed;
use crate::task::{self, DateField, Kept, Priority, Token};

/// A byte range of a line and what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: SpanKind,
}

/// What a [`Span`] is. Every range a kind carries is in bytes of the line,
/// as the span's own is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanKind {
    /// An inline code span, backticks included.
    Code,
    /// `<!-- … -->`.
    Comment,
    /// `[text](destination)` or `[text][label]`, with an image's `!`.
    Link {
        /// What is between the first brackets.
        text: Range<usize>,
        /// What is between the parentheses, when anything is. `None` for a
        /// `[label]` and for `()`.
        destination: Option<Range<usize>>,
        /// Whether an earlier link holds it, as `[![alt](image)](page)`
        /// holds its image. A reader that shows one link there shows the
        /// outer one.
        nested: bool,
    },
    /// A bare `http://` or `https://` URL, without the punctuation that
    /// ends its sentence.
    Url,
    /// A CommonMark autolink, `<scheme:…>` or `<user@host>`, angle brackets
    /// included.
    Autolink,
    /// `~~struck~~`, tildes included.
    Strike {
        /// See [`SpanKind::Emphasis`].
        in_word: [bool; 2],
    },
    /// `*emphasis*`, `**strong**` or `***both***`, with `_` too, delimiters
    /// included. The delimiters are one character for emphasis, two for
    /// strong and three for both.
    Emphasis {
        strong: bool,
        emphasis: bool,
        /// For the opening and the closing delimiter, whether a letter or
        /// a digit touches it on both sides. Such a delimiter is part of a
        /// word (`snake_case_name`, `un*believ*able`), and the prose runs
        /// through it.
        in_word: [bool; 2],
    },
    /// A link reference definition, `[label]: destination`: the whole of
    /// the line's text. None of it is prose.
    Definition,
    /// A task's box, `[` to `]`.
    Box(Task),
    /// A due date or a done or cancelled stamp: its glyph, or its
    /// `[due:: …]` brackets, to the end of the date.
    Date {
        field: DateField,
        /// The date's ten bytes.
        date: Range<usize>,
        /// The date, or [`Parsed::Invalid`] for one that is no real day.
        value: Parsed,
    },
    /// A priority glyph, and a U+FE0F after it.
    Priority(Priority),
    /// An `@owner`.
    Owner {
        /// The name, without the `@`.
        name: Range<usize>,
    },
    /// `🆔 abc`.
    Id {
        /// The id itself.
        id: Range<usize>,
    },
    /// A `#tag` on a task line.
    Tag,
    /// The `^block-id` that ends a task line.
    BlockId,
    /// A token Wren keeps as written and does not read; never
    /// [`Kept::Tag`], which is a [`SpanKind::Tag`].
    Kept(Kept),
}

impl SpanKind {
    /// Whether the span comes from a task line's reading and not from
    /// inline Markdown.
    pub fn is_task(&self) -> bool {
        matches!(
            self,
            Self::Box(_)
                | Self::Date { .. }
                | Self::Priority(_)
                | Self::Owner { .. }
                | Self::Id { .. }
                | Self::Tag
                | Self::BlockId
                | Self::Kept(_)
        )
    }
}

impl Span {
    /// The opening and the closing delimiter of a strike or of emphasis.
    pub fn delimiters(&self) -> Option<[Range<usize>; 2]> {
        let len = match self.kind {
            SpanKind::Strike { .. } => 2,
            SpanKind::Emphasis {
                strong, emphasis, ..
            } => 2 * usize::from(strong) + usize::from(emphasis),
            _ => return None,
        };
        let Range { start, end } = self.range;
        Some([start..start + len, end - len..end])
    }

    /// What a strike or emphasis holds between its delimiters.
    pub fn inner(&self) -> Option<Range<usize>> {
        self.delimiters().map(|[open, close]| open.end..close.start)
    }

    /// The bytes that are syntax alone and say nothing themselves, which a
    /// highlighter lets recede: a link's `[`, and its `](` and `)` or its
    /// `][label]`, and the delimiters of a strike or of emphasis.
    pub fn marks(&self) -> impl Iterator<Item = Range<usize>> {
        let marks = match &self.kind {
            SpanKind::Link {
                text, destination, ..
            } => {
                let open = text.start - 1..text.start;
                match destination {
                    Some(destination) => [
                        Some(open),
                        Some(text.end..destination.start),
                        Some(destination.end..self.range.end),
                    ],
                    None => [Some(open), Some(text.end..self.range.end), None],
                }
            }
            _ => match self.delimiters() {
                Some([open, close]) => [Some(open), Some(close), None],
                None => [None, None, None],
            },
        };
        marks.into_iter().flatten()
    }

    /// The bytes of the span that are not prose, which spell check does not
    /// read: all of a code span, a comment, a URL, an autolink and a
    /// reference definition; a link's brackets, its `!` and everything
    /// after its text; and a strike's or emphasis's delimiters, but for one
    /// inside a word. A task token is prose still.
    pub fn not_prose(&self) -> impl Iterator<Item = Range<usize>> {
        let whole = || Some(self.range.clone());
        let ranges = match &self.kind {
            SpanKind::Code
            | SpanKind::Comment
            | SpanKind::Url
            | SpanKind::Autolink
            | SpanKind::Definition => [whole(), None],
            SpanKind::Link { text, .. } => [
                Some(self.range.start..text.start),
                Some(text.end..self.range.end),
            ],
            SpanKind::Strike { in_word } | SpanKind::Emphasis { in_word, .. } => {
                match self.delimiters() {
                    Some([open, close]) => [
                        (!in_word[0]).then_some(open),
                        (!in_word[1]).then_some(close),
                    ],
                    None => [None, None],
                }
            }
            SpanKind::Box(_)
            | SpanKind::Date { .. }
            | SpanKind::Priority(_)
            | SpanKind::Owner { .. }
            | SpanKind::Id { .. }
            | SpanKind::Tag
            | SpanKind::BlockId
            | SpanKind::Kept(_) => [None, None],
        };
        ranges.into_iter().flatten()
    }
}

/// The spans of one line; see [`spans`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineSpans {
    /// The bytes read as inline Markdown: a paragraph line's text, from
    /// [`LineInfo::text_start`] to its end, or a heading's title. Empty for
    /// every other line: code, front matter, HTML, a break or a blank one.
    pub text: Range<usize>,
    /// Sorted by start, a span before the spans inside it.
    pub spans: Vec<Span>,
    /// Bytes that are no span's and still not prose: markup that only
    /// looks like a construct, which spell check skips, and what a reader
    /// takes out of the prose itself. In no order.
    pub not_prose: Vec<Range<usize>>,
}

impl LineSpans {
    /// The byte ranges spell check reads: [`text`](Self::text) without
    /// what its spans say is [not prose](Span::not_prose) and without
    /// [`not_prose`](Self::not_prose). Sorted, apart, non-empty, and on
    /// character boundaries.
    pub fn prose(&self) -> Vec<Range<usize>> {
        let mut prose = Vec::new();
        let mut from = self.text.start;
        let mut cut = |range: Range<usize>| {
            let end = range.start.min(self.text.end);
            if from < end {
                prose.push(from..end);
            }
            from = from.max(range.end);
        };
        let filled = |range: &Range<usize>| !range.is_empty();
        let of_spans = (self.spans.iter().flat_map(Span::not_prose)).filter(filled);
        let others = self
            .not_prose
            .iter()
            .filter(|&range| filled(range))
            .cloned();
        // Spans that lie apart give their exclusions in line order, which
        // is every line but one whose markup nests; the other exclusions
        // are in line order when a reader added them so, as a task's
        // tokens are. Two orderly lists are merged as they come, the
        // spans' first where two start together, as a stable sort of the
        // spans' then the others would order them.
        let mut end = 0;
        let apart = (self.spans.iter())
            .filter(|span| !span.kind.is_task())
            .all(|span| std::mem::replace(&mut end, span.range.end) <= span.range.start);
        if apart && self.not_prose.is_sorted_by_key(|range| range.start) {
            let mut others = others.peekable();
            for range in of_spans {
                while let Some(other) = others.next_if(|other| other.start < range.start) {
                    cut(other);
                }
                cut(range);
            }
            others.for_each(&mut cut);
        } else {
            let mut excluded: Vec<Range<usize>> = of_spans.chain(others).collect();
            excluded.sort_by_key(|range| range.start);
            excluded.into_iter().for_each(&mut cut);
        }
        if from < self.text.end {
            prose.push(from..self.text.end);
        }
        prose
    }

    /// The span the character that starts at byte `column` belongs to: the
    /// narrowest one that holds it, which is the innermost where spans
    /// nest, and of two as narrow a task token before inline Markdown.
    pub fn at(&self, column: usize) -> Option<&Span> {
        self.spans
            .iter()
            .filter(|span| span.range.contains(&column))
            .min_by_key(|span| (span.range.len(), !span.kind.is_task()))
    }
}

/// The spans of `line`, which the classifier read as `info` while scanning
/// with `boxes`.
///
/// Inline Markdown is read in a paragraph line's text and in a heading's
/// title, and nowhere else. With [`Boxes::Status`] a task line's box and
/// tokens are spans too, whatever kind of line the task's text makes it.
pub fn spans(line: &str, info: &LineInfo, boxes: Boxes) -> LineSpans {
    let text = match &info.kind {
        Kind::Prose => info.text_start..line.len(),
        Kind::Heading { text, .. } => text.clone(),
        _ => 0..0,
    };
    let (mut spans, mut not_prose) = inline(&line[text.clone()]);
    shift(&mut spans, text.start);
    for range in &mut not_prose {
        *range = range.start + text.start..range.end + text.start;
    }
    if info.kind == Kind::Prose && is_reference_definition(&line[text.clone()]) {
        spans.push(Span {
            range: text.clone(),
            kind: SpanKind::Definition,
        });
    }
    if boxes == Boxes::Status {
        if let Some(task) = info.item.and_then(|item| item.task) {
            task_spans(line, task, &mut spans);
        }
    }
    spans.sort_by(|a, b| (a.range.start.cmp(&b.range.start)).then(b.range.end.cmp(&a.range.end)));
    LineSpans {
        text,
        spans,
        not_prose,
    }
}

/// Only the links and bare URLs of `line[text]`, read with nothing else on
/// the line in mind, so a code span or a comment hides none. For text that
/// is otherwise shown as one piece, as a finished task's is.
pub fn links(line: &str, text: Range<usize>) -> Vec<Span> {
    let piece = &line[text.clone()];
    if !may_hold_markup(piece) {
        return Vec::new();
    }
    let mut scan = Scan::new(piece);
    scan.links();
    scan.bare_urls();
    let mut spans = std::mem::take(&mut scan.spans);
    shift(&mut spans, text.start);
    spans
}

/// Whether `text` holds a character some inline Markdown starts with or,
/// for a bare URL, has in its scheme. Text with none has no span, and is
/// most of what anyone writes.
fn may_hold_markup(text: &str) -> bool {
    text.bytes()
        .any(|byte| matches!(byte, b'`' | b'<' | b'[' | b'*' | b'_' | b'~' | b':'))
}

/// The inline Markdown of `text`: its spans, and the bytes that are no
/// span's and not prose ([`LineSpans::not_prose`]), all in bytes of
/// `text`.
fn inline(text: &str) -> (Vec<Span>, Vec<Range<usize>>) {
    if !may_hold_markup(text) {
        return (Vec::new(), Vec::new());
    }
    let mut scan = Scan::new(text);
    scan.code_spans();
    scan.comments();
    // While only code and comments are locked.
    scan.autolinks();
    let texts = scan.links();
    scan.bare_urls();
    for text in &texts {
        scan.delimited(text.clone());
    }
    for text in texts {
        scan.lock(text);
    }
    scan.delimited(0..text.len());
    (
        std::mem::take(&mut scan.spans),
        std::mem::take(&mut scan.not_prose),
    )
}

/// Moves spans read in a piece of a line that starts at byte `by` into the
/// line.
fn shift(spans: &mut [Span], by: usize) {
    let moved = |range: &mut Range<usize>| *range = range.start + by..range.end + by;
    for span in spans {
        moved(&mut span.range);
        if let SpanKind::Link {
            text, destination, ..
        } = &mut span.kind
        {
            moved(text);
            destination.iter_mut().for_each(moved);
        }
    }
}

/// The box of the task line `line` and what [`task::parse`] reads after it.
fn task_spans(line: &str, task: Task, spans: &mut Vec<Span>) {
    let mut push = |range: Range<usize>, kind: SpanKind| spans.push(Span { range, kind });
    push(task.start..task.end, SpanKind::Box(task));
    let Some(read) = task::parse(line, task) else {
        return;
    };
    for owner in read.owners() {
        let name = owner.name.clone();
        push(owner.range.clone(), SpanKind::Owner { name });
    }
    for tag in read.tags() {
        push(tag, SpanKind::Tag);
    }
    for token in read.tokens() {
        let kind = match token {
            Token::Date(token) => SpanKind::Date {
                field: token.field,
                date: token.date.clone(),
                value: token.value,
            },
            Token::Priority(token) => SpanKind::Priority(token.level),
            Token::Id(token) => SpanKind::Id {
                id: token.id.clone(),
            },
            Token::BlockId(_) => SpanKind::BlockId,
            // Among the line's tags and owners above.
            Token::Kept(token) if token.kind == Kept::Tag => continue,
            Token::Owner(_) => continue,
            Token::Kept(token) => SpanKind::Kept(token.kind),
        };
        push(token.range(), kind);
    }
}

/// Whether `text`, a line's text, opens a link reference definition:
/// `[label]:`. A footnote definition (`[^label]:`) is not one; its text is
/// prose.
fn is_reference_definition(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('[') else {
        return false;
    };
    if rest.starts_with('^') {
        return false;
    }
    let bytes = rest.as_bytes();
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        match byte {
            b'\\' => i += 1,
            b'[' => return false,
            b']' => {
                return bytes.get(i + 1) == Some(&b':')
                    && rest[..i].chars().any(|ch| !ch.is_whitespace());
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// A piece of a line as its inline Markdown is found, one kind of
/// construct after another. A construct found later can wrap one found
/// earlier but never pair its delimiters inside it. Ranges are in bytes of
/// `text`.
struct Scan<'a> {
    text: &'a str,
    /// For each byte, whether a construct found so far keeps later
    /// delimiters from matching there.
    locked: Vec<bool>,
    spans: Vec<Span>,
    /// See [`LineSpans::not_prose`].
    not_prose: Vec<Range<usize>>,
}

thread_local! {
    /// The lock buffer of this thread's last scan, so that the next one,
    /// of the next line, allocates none.
    static LOCKS: Cell<Vec<bool>> = const { Cell::new(Vec::new()) };
}

/// The longest lock buffer kept between scans, in bytes of a line.
const KEPT_LOCKS: usize = 1 << 16;

impl<'a> Scan<'a> {
    fn new(text: &'a str) -> Self {
        let mut locked = LOCKS.take();
        locked.clear();
        locked.resize(text.len(), false);
        Self {
            text,
            locked,
            spans: Vec::new(),
            not_prose: Vec::new(),
        }
    }

    fn push(&mut self, range: Range<usize>, kind: SpanKind) {
        self.spans.push(Span { range, kind });
    }

    fn lock(&mut self, range: Range<usize>) {
        self.locked[range].fill(true);
    }

    fn is_free(&self, range: Range<usize>) -> bool {
        self.locked[range].iter().all(|locked| !locked)
    }

    /// The first `marker` in `from..end` that lies on free bytes.
    fn next_free(&self, marker: &str, from: usize, end: usize) -> Option<usize> {
        let mut search = from;
        while let Some(at) = self.text[search..end].find(marker).map(|at| search + at) {
            if self.is_free(at..at + marker.len()) {
                return Some(at);
            }
            search = at + 1;
        }
        None
    }

    /// Whether a letter or a digit touches the delimiter at `range` on
    /// both sides.
    fn in_word(&self, range: Range<usize>) -> bool {
        let before = self.text[..range.start].chars().next_back();
        let after = self.text[range.end..].chars().next();
        [before, after]
            .iter()
            .all(|ch| ch.is_some_and(char::is_alphanumeric))
    }

    /// Backtick code spans, closed by a run of as many backticks as opened
    /// them.
    fn code_spans(&mut self) {
        let bytes = self.text.as_bytes();
        let run = |from: usize| bytes[from..].iter().take_while(|&&b| b == b'`').count();
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
                    self.lock(i..end);
                    self.push(i..end, SpanKind::Code);
                    i = end;
                }
                None => i += ticks,
            }
        }
    }

    /// `<!-- ... -->`, from each `<!--` that no comment holds to the first
    /// `-->` after it, unless a code span lies between. Run after
    /// [`Self::code_spans`] alone.
    ///
    /// Many openers can reach one closer and be turned down for the same
    /// code span, so the closer is looked for once, not once for each, and
    /// the code spans are walked along with the openers.
    fn comments(&mut self) {
        // The spans so far: the code spans, in line order.
        let codes = self.spans.len();
        let mut code = 0;
        let mut closer = None;
        let mut search = 0;
        while let Some(open) = self.text[search..].find("<!--").map(|at| search + at) {
            let inside = open + 4;
            let close = match closer {
                Some(close) if close >= inside => close,
                _ => match self.text[inside..].find("-->") {
                    Some(at) => inside + at,
                    None => break,
                },
            };
            closer = Some(close);
            let end = close + 3;
            while code < codes && self.spans[code].range.end <= open {
                code += 1;
            }
            if code == codes || self.spans[code].range.start >= end {
                self.lock(open..end);
                self.push(open..end, SpanKind::Comment);
                search = end;
            } else {
                search = open + 1;
            }
        }
    }

    /// `[text](destination)` and `[text][ref]`. Brackets pair by depth, and
    /// a destination ends at the `)` that balances its parentheses, as in
    /// CommonMark. Returns the text ranges of the links no other link
    /// holds.
    fn links(&mut self) -> Vec<Range<usize>> {
        let bytes = self.text.as_bytes();
        let mut open = Vec::new();
        let mut pairs = Vec::new();
        for (i, &byte) in bytes.iter().enumerate() {
            match byte {
                _ if self.locked[i] => {}
                b'[' => open.push(i),
                b']' => pairs.extend(open.pop().map(|start| (start, i))),
                _ => {}
            }
        }
        pairs.sort_unstable();
        let mut texts = Vec::new();
        let mut covered = 0;
        let mut outer = 0..0;
        for (start, close) in pairs {
            // A link or image inside a link's text, such as a linked
            // image, is a link of its own that the outer one holds.
            let nested = start < covered;
            let Some(end) = self.link_tail(close + 1) else {
                continue;
            };
            let bang = usize::from(start > 0 && bytes[start - 1] == b'!');
            if nested && !(outer.start <= start - bang && end <= outer.end) {
                // The outer link's `[label]` with a tail of its own, as
                // the `[b](c)` of `[a][b](c)`: no link, but as little
                // prose as one.
                self.not_prose.push(start - bang..start + 1);
                self.not_prose.push(close..end);
                continue;
            }
            let text = start + 1..close;
            let destination = Some(close + 2..end - 1)
                .filter(|destination| bytes[close + 1] == b'(' && !destination.is_empty());
            self.push(
                start - bang..end,
                SpanKind::Link {
                    text: text.clone(),
                    destination,
                    nested,
                },
            );
            if !nested {
                covered = end;
                outer = text.clone();
                self.lock(start..start + 1);
                self.lock(close..end);
                texts.push(text);
            }
        }
        texts
    }

    /// The end of the `(destination)` or `[ref]` that starts at `start`.
    fn link_tail(&self, start: usize) -> Option<usize> {
        let bytes = self.text.as_bytes();
        let inline = match bytes.get(start)? {
            b'(' => true,
            b'[' => false,
            _ => return None,
        };
        let mut depth = 0_usize;
        for (i, &byte) in bytes.iter().enumerate().skip(start) {
            if self.locked[i] {
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

    /// Bare `http://` and `https://` URLs. A URL ends at whitespace or
    /// other markup, and keeps [`trim_url`]'s punctuation out.
    fn bare_urls(&mut self) {
        let text = self.text;
        let mut search = 0;
        while let Some((start, scheme)) = ["https://", "http://"]
            .iter()
            .filter_map(|scheme| Some((search + text[search..].find(scheme)?, scheme.len())))
            .min()
        {
            let rest = &text[start..];
            let length = rest
                .char_indices()
                .find(|&(i, ch)| ch.is_whitespace() || self.locked[start + i])
                .map_or(rest.len(), |(i, _)| i);
            let url = trim_url(&rest[..length]);
            if url.len() > scheme {
                self.lock(start..start + url.len());
                self.push(start..start + url.len(), SpanKind::Url);
            }
            search = start + length.max(1);
        }
    }

    /// CommonMark autolinks, `<scheme:…>` and `<user@host>`. They keep
    /// nothing from matching inside them: a bare URL between the angle
    /// brackets is a [`SpanKind::Url`] too. Angle brackets that take in
    /// any of a code span or a comment are no autolink, and no prose.
    fn autolinks(&mut self) {
        let text = self.text;
        let mut search = 0;
        while let Some(open) = text[search..].find('<').map(|at| search + at) {
            let rest = &text[open + 1..];
            let Some(length) = rest.find(['<', '>']) else {
                break;
            };
            let inner = &rest[..length];
            if rest.as_bytes()[length] == b'>' && is_autolink(inner) {
                let end = open + 1 + length + 1;
                if self.is_free(open..end) {
                    self.push(open..end, SpanKind::Autolink);
                } else {
                    self.not_prose.push(open..end);
                }
                search = end;
            } else {
                search = open + 1;
            }
        }
    }

    /// Strikes, then emphasis from the strongest marker down, inside
    /// `range`.
    fn delimited(&mut self, range: Range<usize>) {
        self.strikes(range.clone());
        for (marker, strong, emphasis) in [
            ("***", true, true),
            ("___", true, true),
            ("**", true, false),
            ("__", true, false),
            ("*", false, true),
            ("_", false, true),
        ] {
            self.emphasis(range.clone(), marker, strong, emphasis);
        }
    }

    /// `~~struck~~`. The tildes hold no other delimiter; the text between
    /// them may.
    fn strikes(&mut self, range: Range<usize>) {
        let mut search = range.start;
        while let Some(open) = self.next_free("~~", search, range.end) {
            let Some(close) = self.next_free("~~", open + 2, range.end) else {
                break;
            };
            let end = close + 2;
            let in_word = [self.in_word(open..open + 2), self.in_word(close..end)];
            self.lock(open..open + 2);
            self.lock(close..end);
            self.push(open..end, SpanKind::Strike { in_word });
            search = end;
        }
    }

    /// Emphasis delimited by `marker`, which then holds no other emphasis.
    fn emphasis(&mut self, range: Range<usize>, marker: &str, strong: bool, emphasis: bool) {
        let mut search = range.start;
        while let Some(open) = self.next_free(marker, search, range.end) {
            let content = open + marker.len();
            let Some(close) = self.next_free(marker, content, range.end) else {
                break;
            };
            let end = close + marker.len();
            let in_word = [self.in_word(open..content), self.in_word(close..end)];
            self.lock(open..end);
            self.push(
                open..end,
                SpanKind::Emphasis {
                    strong,
                    emphasis,
                    in_word,
                },
            );
            search = end;
        }
    }
}

impl Drop for Scan<'_> {
    fn drop(&mut self) {
        if self.locked.capacity() <= KEPT_LOCKS {
            LOCKS.set(std::mem::take(&mut self.locked));
        }
    }
}

/// Whether `inner`, the text between `<` and `>`, makes an autolink: a URI
/// (a scheme of 2 to 32 letters, digits, `+`, `.` or `-` that starts with a
/// letter, a `:`, then no space) or an email address.
fn is_autolink(inner: &str) -> bool {
    if inner.is_empty() || inner.contains(|ch: char| ch.is_whitespace() || ch.is_control()) {
        return false;
    }
    if let Some((scheme, _)) = inner.split_once(':') {
        let bytes = scheme.as_bytes();
        if (2..=32).contains(&bytes.len())
            && bytes[0].is_ascii_alphabetic()
            && bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
        {
            return true;
        }
    }
    inner
        .split_once('@')
        .is_some_and(|(user, host)| !user.is_empty() && !host.is_empty() && !host.contains('@'))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Scanner;
    use crate::date::Date;
    use crate::status::StatusType;

    /// Each line of `text` with its spans, as a host scanning with `boxes`
    /// reads the document.
    fn read(text: &str, boxes: Boxes) -> Vec<(&str, LineSpans)> {
        let lines: Vec<&str> = text.split('\n').collect();
        let end = crate::block::front_matter_end(&lines[..]);
        let mut scanner = Scanner::with_front_matter(end, boxes);
        lines
            .into_iter()
            .map(|line| (line, spans(line, &scanner.next_line(line), boxes)))
            .collect()
    }

    /// The spans of the one-line document `line`.
    fn line(line: &str, boxes: Boxes) -> LineSpans {
        read(line, boxes).remove(0).1
    }

    /// A span's kind in a word or two, with the text of the ranges it
    /// carries. A delimiter inside a word is marked `open in word` or
    /// `close in word`.
    fn label(text: &str, span: &Span) -> String {
        let in_word = |in_word: &[bool; 2]| {
            let mut label = String::new();
            for (inside, side) in in_word.iter().zip(["open", "close"]) {
                if *inside {
                    label.push_str(&format!(", {side} in word"));
                }
            }
            label
        };
        match &span.kind {
            SpanKind::Code => "code".to_owned(),
            SpanKind::Comment => "comment".to_owned(),
            SpanKind::Link {
                text: name,
                destination,
                nested,
            } => format!(
                "{}link <{}>{}",
                if *nested { "nested " } else { "" },
                &text[name.clone()],
                destination
                    .as_ref()
                    .map(|destination| format!(" to <{}>", &text[destination.clone()]))
                    .unwrap_or_default(),
            ),
            SpanKind::Url => "url".to_owned(),
            SpanKind::Autolink => "autolink".to_owned(),
            SpanKind::Strike { in_word: inside } => format!("strike{}", in_word(inside)),
            SpanKind::Emphasis {
                strong,
                emphasis,
                in_word: inside,
            } => {
                let face = match (strong, emphasis) {
                    (true, true) => "strong emphasis",
                    (true, false) => "strong",
                    _ => "emphasis",
                };
                format!("{face}{}", in_word(inside))
            }
            SpanKind::Definition => "definition".to_owned(),
            SpanKind::Box(task) => format!("box <{}> {}", task.symbol, task.status.name()),
            SpanKind::Date { field, date, value } => {
                let field = match field {
                    DateField::Due => "due",
                    DateField::Done => "done",
                    DateField::Cancelled => "cancelled",
                };
                let day = match value {
                    Parsed::Date(day) => day.to_string(),
                    Parsed::Invalid => "no date".to_owned(),
                };
                format!("{field} <{}> {day}", &text[date.clone()])
            }
            SpanKind::Priority(level) => format!("priority {}", level.name()),
            SpanKind::Owner { name } => format!("owner <{}>", &text[name.clone()]),
            SpanKind::Id { id } => format!("id <{}>", &text[id.clone()]),
            SpanKind::Tag => "tag".to_owned(),
            SpanKind::BlockId => "block id".to_owned(),
            SpanKind::Kept(kept) => format!("kept {kept:?}"),
        }
    }

    /// `(text, label)` for each span of each line of `text`.
    fn table(text: &str, boxes: Boxes) -> Vec<Vec<(&str, String)>> {
        read(text, boxes)
            .into_iter()
            .map(|(line, read)| {
                read.spans
                    .iter()
                    .map(|span| (&line[span.range.clone()], label(line, span)))
                    .collect()
            })
            .collect()
    }

    /// [`table`] for a one-line document.
    fn row(line: &str, boxes: Boxes) -> Vec<(&str, String)> {
        table(line, boxes).remove(0)
    }

    /// `expected` as [`table`] gives a line.
    fn rows<'a>(expected: &[(&'a str, &str)]) -> Vec<(&'a str, String)> {
        expected
            .iter()
            .map(|(text, label)| (*text, (*label).to_owned()))
            .collect()
    }

    /// The prose of each line of `text`, as text.
    fn prose(text: &str, boxes: Boxes) -> Vec<Vec<&str>> {
        read(text, boxes)
            .into_iter()
            .map(|(line, read)| read.prose().into_iter().map(|range| &line[range]).collect())
            .collect()
    }

    // ── Inline Markdown ──────────────────────────────────────────────────────

    #[test]
    fn each_inline_kind_is_a_span_with_its_bytes() {
        for boxes in [Boxes::Plain, Boxes::Status] {
            for (text, expected) in [
                ("plain words", vec![]),
                ("a `code` b", vec![(2..8, "code")]),
                ("a ``co`de`` b", vec![(2..11, "code")]),
                ("a `open", vec![]),
                ("a <!-- note --> b", vec![(2..15, "comment")]),
                ("[text](dest)", vec![(0..12, "link <text> to <dest>")]),
                ("![alt](pic.png)", vec![(0..15, "link <alt> to <pic.png>")]),
                ("[text][label]", vec![(0..13, "link <text>")]),
                ("[text]()", vec![(0..8, "link <text>")]),
                ("[text] (dest)", vec![]),
                ("see https://x.io/a.", vec![(4..18, "url")]),
                ("<https://x.io>", vec![(0..14, "autolink"), (1..13, "url")]),
                ("<me@x.io>", vec![(0..9, "autolink")]),
                ("<kbd>", vec![]),
                ("~~gone~~", vec![(0..8, "strike")]),
                ("*a*", vec![(0..3, "emphasis")]),
                ("_a_", vec![(0..3, "emphasis")]),
                ("**a**", vec![(0..5, "strong")]),
                ("__a__", vec![(0..5, "strong")]),
                ("***a***", vec![(0..7, "strong emphasis")]),
                ("___a___", vec![(0..7, "strong emphasis")]),
                (
                    "é **界** 🐦 `说`",
                    vec![(3..10, "strong"), (16..21, "code")],
                ),
            ] {
                let read = line(text, boxes);
                let found: Vec<_> = read
                    .spans
                    .iter()
                    .map(|span| (span.range.clone(), label(text, span)))
                    .collect();
                let expected: Vec<_> = expected
                    .into_iter()
                    .map(|(range, label)| (range, label.to_owned()))
                    .collect();
                assert_eq!(found, expected, "{text:?}");
                assert_eq!(read.text, 0..text.len(), "{text:?}");
            }
        }
    }

    #[test]
    fn spans_are_sorted_with_the_outer_one_first() {
        assert_eq!(
            row("~~**[a `b`](u) c**~~ d", Boxes::Plain),
            rows(&[
                ("~~**[a `b`](u) c**~~", "strike"),
                ("**[a `b`](u) c**", "strong"),
                ("[a `b`](u)", "link <a `b`> to <u>"),
                ("`b`", "code"),
            ])
        );
        // An image in a link is a link of its own, which the outer one holds.
        assert_eq!(
            row("[![alt](pic.png)](page)", Boxes::Plain),
            rows(&[
                (
                    "[![alt](pic.png)](page)",
                    "link <![alt](pic.png)> to <page>"
                ),
                ("![alt](pic.png)", "nested link <alt> to <pic.png>"),
            ])
        );
        // Each strike and emphasis pairs its own delimiters, so two cross.
        assert_eq!(
            row("~~a *b~~ c*", Boxes::Plain),
            rows(&[("~~a *b~~", "strike"), ("*b~~ c*", "emphasis")])
        );
    }

    #[test]
    fn nothing_matches_inside_code_a_comment_or_a_url() {
        assert_eq!(
            row(
                "`**a** [b](c)` <!-- _d_ --> https://x.io/_e_/",
                Boxes::Plain
            ),
            rows(&[
                ("`**a** [b](c)`", "code"),
                ("<!-- _d_ -->", "comment"),
                ("https://x.io/_e_/", "url"),
            ])
        );
    }

    /// Angle brackets around code or a comment, or across the edge of
    /// one, are no autolink: the character under the pointer is code. What
    /// they take in is not prose, as it never was.
    #[test]
    fn angle_brackets_over_code_or_a_comment_are_no_autolink() {
        let text = "`<a@b>` <!-- <c@d> --> <https://x.io> <e@f>";
        assert_eq!(
            row(text, Boxes::Plain),
            rows(&[
                ("`<a@b>`", "code"),
                ("<!-- <c@d> -->", "comment"),
                ("<https://x.io>", "autolink"),
                ("https://x.io", "url"),
                ("<e@f>", "autolink"),
            ])
        );
        let read = line(text, Boxes::Plain);
        assert_eq!(read.at(2).map(|span| &span.kind), Some(&SpanKind::Code));
        assert_eq!(read.at(14).map(|span| &span.kind), Some(&SpanKind::Comment));
        assert_eq!(read.at(24).map(|span| &span.kind), Some(&SpanKind::Url));
        assert_eq!(
            read.at(23).map(|span| &span.kind),
            Some(&SpanKind::Autolink)
        );
        // Brackets that open in code and close after it, or the other way.
        let text = "`<a`@b> c <d@`e>` f";
        let read = line(text, Boxes::Plain);
        assert_eq!(
            row(text, Boxes::Plain),
            rows(&[("`<a`", "code"), ("`e>`", "code")])
        );
        assert_eq!(read.not_prose, [1..7, 10..16]);
        assert_eq!(read.at(5), None);
        assert_eq!(read.at(11), None);
        assert_eq!(prose(text, Boxes::Plain), [[" c ", " f"]]);
    }

    /// In `[a][b](c)` the `[b]` is the first link's label, so `[b](c)` is
    /// no second link, and a pointer on `c` finds none. Spell check skips
    /// its brackets and its tail all the same.
    #[test]
    fn a_links_label_with_a_tail_of_its_own_is_no_second_link() {
        let text = "[a][b](c) d";
        let read = line(text, Boxes::Plain);
        assert_eq!(row(text, Boxes::Plain), rows(&[("[a][b]", "link <a>")]));
        assert_eq!(read.not_prose, [3..4, 5..9]);
        assert_eq!(
            read.at(4).map(|span| &text[span.range.clone()]),
            Some("[a][b]")
        );
        assert_eq!(read.at(7), None);
        assert_eq!(prose(text, Boxes::Plain), [["a", " d"]]);
        // A link in a link's text is one, and lies inside it.
        let text = "[a ![b](c) [d](e)](f)";
        assert_eq!(
            row(text, Boxes::Plain),
            rows(&[
                ("[a ![b](c) [d](e)](f)", "link <a ![b](c) [d](e)> to <f>"),
                ("![b](c)", "nested link <b> to <c>"),
                ("[d](e)", "nested link <d> to <e>"),
            ])
        );
        assert!(line(text, Boxes::Plain).not_prose.is_empty());
    }

    #[test]
    fn a_span_gives_its_delimiters_its_inside_and_its_marks() {
        let read = line("**a** ~~b~~ [c](d) [e][f] `g`", Boxes::Plain);
        let parts: Vec<_> = read
            .spans
            .iter()
            .map(|span| {
                (
                    span.delimiters(),
                    span.inner(),
                    span.marks().collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            parts,
            [
                (Some([0..2, 3..5]), Some(2..3), vec![0..2, 3..5]),
                (Some([6..8, 9..11]), Some(8..9), vec![6..8, 9..11]),
                (None, None, vec![12..13, 14..16, 17..18]),
                (None, None, vec![19..20, 21..25]),
                (None, None, vec![]),
            ]
        );
        // Three characters delimit strong emphasis, and one emphasis.
        let both = &line("***a*** _b_", Boxes::Plain).spans;
        assert_eq!(both[0].delimiters(), Some([0..3, 4..7]));
        assert_eq!(both[1].delimiters(), Some([8..9, 10..11]));
    }

    // ── Line kinds ───────────────────────────────────────────────────────────

    #[test]
    fn inline_markdown_is_read_in_paragraph_text_and_heading_titles_only() {
        let text = "---\ntitle: `a`\n---\n# A `b` title #\n\npara `c`\n> quoted `d`\n- item `e`\n  1. deep `f`\n- [ ] open `g`\n- [x] done `h`\n\n```rust\nlet `i`;\n```\n\n    indented `j`\n\n<div>`k`</div>\n\nSetext `l`\n===\n***\n";
        let found = read(text, Boxes::Plain);
        let texts: Vec<_> = found
            .iter()
            .map(|(line, read)| &line[read.text.clone()])
            .collect();
        assert_eq!(
            texts,
            [
                "",
                "",
                "",
                "A `b` title",
                "",
                "para `c`",
                "quoted `d`",
                "item `e`",
                "deep `f`",
                "open `g`",
                "done `h`",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "Setext `l`",
                "",
                "",
                "",
            ]
        );
        let code: Vec<_> = table(text, Boxes::Plain)
            .into_iter()
            .map(|row| row.into_iter().map(|(text, _)| text).collect::<Vec<_>>())
            .collect();
        for (index, (line, _)) in found.iter().enumerate() {
            let expected: Vec<&str> = match texts[index] {
                "" => Vec::new(),
                text => vec![&text[text.find('`').expect("a code span")..][..3]],
            };
            assert_eq!(code[index], expected, "line {index} {line:?}");
        }
    }

    #[test]
    fn a_reference_definition_is_one_span_over_its_text_and_a_footnote_is_none() {
        assert_eq!(
            row("[label]: https://x.io \"T\"", Boxes::Plain),
            rows(&[
                ("[label]: https://x.io \"T\"", "definition"),
                ("https://x.io", "url"),
            ])
        );
        assert_eq!(
            row("- [label]: <https://x.io>", Boxes::Plain),
            rows(&[
                ("[label]: <https://x.io>", "definition"),
                ("<https://x.io>", "autolink"),
                ("https://x.io", "url"),
            ])
        );
        for text in [
            "[^note]: words",
            "[a][b]: c",
            "[ ]: c",
            "\\[a]: c",
            "a [b]: c",
        ] {
            assert!(
                line(text, Boxes::Plain)
                    .spans
                    .iter()
                    .all(|span| span.kind != SpanKind::Definition),
                "{text:?}"
            );
        }
    }

    // ── Prose ────────────────────────────────────────────────────────────────

    #[test]
    fn prose_is_the_text_without_what_its_spans_exclude() {
        for (text, expected) in [
            ("plain words", vec!["plain words"]),
            ("a `code` b <!-- c --> d", vec!["a ", " b ", " d"]),
            (
                "see [the note](https://x.io) now",
                vec!["see ", "the note", " now"],
            ),
            ("![alt text](pic.png)", vec!["alt text"]),
            ("[![alt](pic.png)](page) x", vec!["alt", " x"]),
            ("[text][label] x", vec!["text", " x"]),
            ("at https://x.io, or <me@x.io>.", vec!["at ", ", or ", "."]),
            ("**bold** and ~~gone~~", vec!["bold", " and ", "gone"]),
            (
                "snake_case_name un*believ*able",
                vec!["snake_case_name un*believ*able"],
            ),
            ("a_b_ _c_d", vec!["a_b", " ", "c_d"]),
            ("# A *title* #", vec!["A ", "title"]),
            ("- [x] done `code` words", vec!["done ", " words"]),
            ("[label]: https://x.io", vec![]),
            ("[^note]: a footnote", vec!["[^note]: a footnote"]),
            ("```", vec![]),
            ("", vec![]),
        ] {
            assert_eq!(prose(text, Boxes::Plain), [expected], "{text:?}");
        }
    }

    #[test]
    fn a_delimiter_inside_a_word_is_marked_and_stays_in_the_prose() {
        assert_eq!(
            row("un*believ*able a_b_ ~~c~~d", Boxes::Plain),
            rows(&[
                ("*believ*", "emphasis, open in word, close in word"),
                ("_b_", "emphasis, open in word"),
                ("~~c~~", "strike, close in word"),
            ])
        );
        let read = line("a_b_ ~~c~~d", Boxes::Plain);
        let excluded: Vec<_> = read.spans.iter().flat_map(Span::not_prose).collect();
        assert_eq!(excluded, [3..4, 5..7]);
    }

    // ── Tasks ────────────────────────────────────────────────────────────────

    #[test]
    fn each_task_token_is_a_span_with_tasks_on() {
        for (text, expected) in [
            ("- [ ] Call", vec![(2..5, "box < > open")]),
            ("- [/] Call", vec![(2..5, "box </> doing")]),
            ("- [x] Call", vec![(2..5, "box <x> done")]),
            ("- [-] Call", vec![(2..5, "box <-> cancelled")]),
            ("- [?] Call", vec![(2..5, "box <?> open")]),
            ("- [✓] Call", vec![(2..7, "box <✓> open")]),
            ("1. [界]\tCall", vec![(3..8, "box <界> open")]),
            ("- [ ]", vec![(2..5, "box < > open")]),
            (
                "- [ ] Call 📅 2026-12-01",
                vec![(2..5, "box < > open"), (11..26, "due <2026-12-01> 2026-12-01")],
            ),
            (
                "- [ ] Call 📅\u{fe0f}2026-12-01",
                vec![(2..5, "box < > open"), (11..28, "due <2026-12-01> 2026-12-01")],
            ),
            (
                "- [ ] Call 📆 2026-12-01",
                vec![(2..5, "box < > open"), (11..26, "due <2026-12-01> 2026-12-01")],
            ),
            (
                "- [ ] Call 🗓 2026-12-01",
                vec![(2..5, "box < > open"), (11..26, "due <2026-12-01> 2026-12-01")],
            ),
            (
                "- [ ] Call [due:: 2026-12-01]",
                vec![(2..5, "box < > open"), (11..29, "due <2026-12-01> 2026-12-01")],
            ),
            (
                "- [ ] Call 📅 2026-02-30",
                vec![(2..5, "box < > open"), (11..26, "due <2026-02-30> no date")],
            ),
            (
                "- [x] Call ✅ 2026-11-19",
                vec![(2..5, "box <x> done"), (11..25, "done <2026-11-19> 2026-11-19")],
            ),
            (
                "- [-] Call ❌ 2026-11-19",
                vec![
                    (2..5, "box <-> cancelled"),
                    (11..25, "cancelled <2026-11-19> 2026-11-19"),
                ],
            ),
            (
                "- [ ] Call ⏫",
                vec![(2..5, "box < > open"), (11..14, "priority High")],
            ),
            (
                "- [ ] Call 🔺\u{fe0f}",
                vec![(2..5, "box < > open"), (11..18, "priority Highest")],
            ),
            (
                "- [ ] Call 🔽 ⏬",
                vec![
                    (2..5, "box < > open"),
                    (11..15, "priority Low"),
                    (16..19, "priority Lowest"),
                ],
            ),
            (
                "- [ ] Call @sam and @alice.chen.",
                vec![
                    (2..5, "box < > open"),
                    (11..15, "owner <sam>"),
                    (20..31, "owner <alice.chen>"),
                ],
            ),
            (
                "- [ ] Call ⏫ @zoë",
                vec![
                    (2..5, "box < > open"),
                    (11..14, "priority High"),
                    (15..20, "owner <zoë>"),
                ],
            ),
            (
                "- [ ] Call 🆔 a1-b",
                vec![(2..5, "box < > open"), (11..20, "id <a1-b>")],
            ),
            (
                "- [ ] Call #home now ⏫ #work",
                vec![
                    (2..5, "box < > open"),
                    (11..16, "tag"),
                    (21..24, "priority High"),
                    (25..30, "tag"),
                ],
            ),
            (
                "- [ ] Call #home, a#b ## #",
                vec![(2..5, "box < > open"), (11..16, "tag")],
            ),
            (
                "- [ ] Call ^a-1",
                vec![(2..5, "box < > open"), (11..15, "block id")],
            ),
            (
                "- [ ] Call ➕ 2026-11-01 🛫 2026-11-02 ⏳ 2026-11-03 🔁 every week 🏁 delete ⛔ a,b [k:: v]",
                vec![
                    (2..5, "box < > open"),
                    (11..25, "kept Created"),
                    (26..41, "kept Start"),
                    (42..56, "kept Scheduled"),
                    (57..72, "kept Recurrence"),
                    (73..84, "kept OnCompletion"),
                    (85..92, "kept DependsOn"),
                    (93..100, "kept Field"),
                ],
            ),
            // A token with text after it is description.
            (
                "- [ ] 📅 2026-12-01 then lunch",
                vec![(2..5, "box < > open")],
            ),
        ] {
            let read = line(text, Boxes::Status);
            let found: Vec<_> = read
                .spans
                .iter()
                .map(|span| (span.range.clone(), label(text, span)))
                .collect();
            let expected: Vec<_> = expected
                .into_iter()
                .map(|(range, label)| (range, label.to_owned()))
                .collect();
            assert_eq!(found, expected, "{text:?}");
            assert!(read.spans.iter().all(|span| span.kind.is_task()), "{text:?}");
        }
    }

    #[test]
    fn a_task_span_carries_what_its_reader_needs() {
        let text = "- [/] Call @sam 🆔 a1 🔼 📅 2026-11-18 ^end";
        let kinds: Vec<_> = line(text, Boxes::Status)
            .spans
            .into_iter()
            .map(|span| span.kind)
            .collect();
        let task = Task {
            start: 2,
            end: 5,
            symbol: '/',
            status: StatusType::Doing,
        };
        assert_eq!(
            kinds,
            [
                SpanKind::Box(task),
                SpanKind::Owner { name: 12..15 },
                SpanKind::Id { id: 21..23 },
                SpanKind::Priority(Priority::Medium),
                SpanKind::Date {
                    field: DateField::Due,
                    date: 34..44,
                    value: Parsed::Date(Date::new(2026, 11, 18).expect("a date")),
                },
                SpanKind::BlockId,
            ]
        );
    }

    #[test]
    fn task_spans_lie_on_any_task_line_beside_its_inline_markdown() {
        assert_eq!(
            table(
                "> 1. [ ] Mail **to @sam** the `plan` 📅 2026-12-01\n- [ ] # Title ⏫\n- not a task @sam ⏫\nplain @sam 📅 2026-12-01",
                Boxes::Status
            ),
            [
                rows(&[
                    ("[ ]", "box < > open"),
                    ("**to @sam**", "strong"),
                    ("@sam", "owner <sam>"),
                    ("`plan`", "code"),
                    ("📅 2026-12-01", "due <2026-12-01> 2026-12-01"),
                ]),
                rows(&[("[ ]", "box < > open"), ("⏫", "priority High")]),
                vec![],
                vec![],
            ]
        );
    }

    #[test]
    fn with_tasks_off_a_line_has_no_task_span() {
        let text = "- [ ] Call @sam #home 🆔 a1 ⏫ 📅 2026-12-01 ✅ 2026-11-19 ^end\n- [x] Done **now** @sam ✅ 2026-11-19\n- [/] Doing @sam 📅 2026-12-01";
        assert_eq!(
            table(text, Boxes::Plain),
            [vec![], rows(&[("**now**", "strong")]), vec![]]
        );
        let on = read(text, Boxes::Status);
        assert_eq!(
            on.iter()
                .map(|(_, read)| read.spans.iter().filter(|span| span.kind.is_task()).count())
                .collect::<Vec<_>>(),
            [8, 3, 3]
        );
        // The tokens are spans and prose still: taking them out of what
        // spell check reads is their reader's choice to make.
        assert_eq!(
            prose(text, Boxes::Status)[1],
            ["Done ", "now", " @sam ✅ 2026-11-19"]
        );
        assert_eq!(prose(text, Boxes::Plain)[1], prose(text, Boxes::Status)[1]);
    }

    // ── Finding a span ───────────────────────────────────────────────────────

    #[test]
    fn the_span_at_a_column_is_the_narrowest_that_holds_it() {
        let text = "- [ ] Mail **to @sam** [the `plan`](u) 📅 2026-12-01";
        let read = line(text, Boxes::Status);
        let at = |column: usize| read.at(column).map(|span| &text[span.range.clone()]);
        for (column, expected) in [
            (0, None),
            (2, Some("[ ]")),
            (4, Some("[ ]")),
            (5, None),
            (6, None),
            (11, Some("**to @sam**")),
            (15, Some("**to @sam**")),
            (16, Some("@sam")),
            (19, Some("@sam")),
            (20, Some("**to @sam**")),
            (22, None),
            (23, Some("[the `plan`](u)")),
            (28, Some("`plan`")),
            (36, Some("[the `plan`](u)")),
            (38, None),
            (39, Some("📅 2026-12-01")),
            (text.len() - 1, Some("📅 2026-12-01")),
            (text.len(), None),
            (text.len() + 9, None),
        ] {
            assert_eq!(at(column), expected, "column {column}");
        }
        // Of two spans as narrow, the task's.
        let narrow = LineSpans {
            text: 6..12,
            spans: vec![
                Span {
                    range: 7..11,
                    kind: SpanKind::Url,
                },
                Span {
                    range: 7..11,
                    kind: SpanKind::Tag,
                },
            ],
            not_prose: Vec::new(),
        };
        assert_eq!(narrow.at(8).map(|span| &span.kind), Some(&SpanKind::Tag));
        assert_eq!(narrow.at(11), None);
    }

    // ── Links read alone ─────────────────────────────────────────────────────

    #[test]
    fn links_read_alone_are_hidden_by_no_code_or_comment() {
        let text = "- [x] see `[a](b)` and **https://x.io** <!-- [c](d) -->";
        let found: Vec<_> = links(text, 6..text.len())
            .iter()
            .map(|span| (&text[span.range.clone()], label(text, span)))
            .collect();
        assert_eq!(
            found,
            rows(&[
                ("[a](b)", "link <a> to <b>"),
                ("[c](d)", "link <c> to <d>"),
                ("https://x.io", "url"),
            ])
        );
        assert_eq!(
            row(text, Boxes::Plain),
            rows(&[
                ("`[a](b)`", "code"),
                ("**https://x.io**", "strong"),
                ("https://x.io", "url"),
                ("<!-- [c](d) -->", "comment"),
            ])
        );
    }

    // ── Cost ─────────────────────────────────────────────────────────────────

    /// Many comment openers that all reach one `-->` across a code span:
    /// none is a comment, and each is turned down without the line being
    /// read again. Read opener by opener to the closer, a line this long
    /// takes minutes.
    #[test]
    fn comment_openers_that_cross_code_are_turned_down_in_linear_time() {
        const OPENERS: usize = 100_000;
        for (start, boxes) in [
            ("- [x] [r]: ", Boxes::Plain),
            ("- [x] [r]: ", Boxes::Status),
            ("see ", Boxes::Plain),
        ] {
            let text = format!("{start}{}`x`-->", "<!--".repeat(OPENERS));
            let began = std::time::Instant::now();
            let read = line(&text, boxes);
            let took = began.elapsed();
            let inline: Vec<_> = read
                .spans
                .iter()
                .filter(|span| !span.kind.is_task() && span.kind != SpanKind::Definition)
                .map(|span| (span.range.clone(), &span.kind))
                .collect();
            let code = text.len() - 6..text.len() - 3;
            assert_eq!(inline, [(code, &SpanKind::Code)], "{start:?}");
            assert!(read.prose().len() <= 2, "{start:?}");
            assert!(took.as_secs() < 20, "{start:?} took {took:?}");
        }
        // Where a comment does close before the code, it is found.
        let text = format!("a {}--> `x`-->", "<!--".repeat(1000));
        let read = line(&text, Boxes::Plain);
        assert_eq!(
            read.spans
                .iter()
                .map(|span| (span.range.clone(), &span.kind))
                .collect::<Vec<_>>(),
            [(2..4005, &SpanKind::Comment), (4006..4009, &SpanKind::Code)]
        );
    }

    // ── Any text ─────────────────────────────────────────────────────────────

    /// A small deterministic generator (xorshift64*), so failures replay.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) % n as u64) as usize
        }
    }

    /// The prose with every exclusion sorted first, as `prose` once always
    /// read it.
    fn sorted_prose(read: &LineSpans) -> Vec<Range<usize>> {
        let mut excluded: Vec<Range<usize>> = (read.spans.iter().flat_map(Span::not_prose))
            .chain(read.not_prose.iter().cloned())
            .filter(|range| !range.is_empty())
            .collect();
        excluded.sort_by_key(|range| range.start);
        let mut prose = Vec::new();
        let mut from = read.text.start;
        for range in excluded {
            let end = range.start.min(read.text.end);
            if from < end {
                prose.push(from..end);
            }
            from = from.max(range.end);
        }
        if from < read.text.end {
            prose.push(from..read.text.end);
        }
        prose
    }

    /// `prose` merges the spans' exclusions with others a reader added in
    /// line order, a task's tokens say, without sorting them, and gets
    /// what sorting every exclusion gets, whatever the line.
    #[test]
    fn merged_exclusions_give_the_prose_sorted_ones_give() {
        const PIECES: &[&str] = &[
            "word ",
            "*",
            "**",
            "_",
            "~~",
            "`",
            "[",
            "](",
            ")",
            "][",
            "]",
            "<",
            ">",
            "<!--",
            "-->",
            "https://x.io ",
            "a@b ",
            "@sam ",
            "#tag ",
            "- [ ] ",
            "- [/] ",
            "📅 2026-11-20 ",
            "✅ 2026-11-18 ",
            "⏫ ",
            "🆔 a1 ",
            "^b1",
            "[due:: 2026-11-18] ",
            "[k:: v] ",
            "🔁 every week ",
            " ",
            "é",
        ];
        let mut rng = Rng(0x5eed_0000_0000_0222);
        for _ in 0..20_000 {
            let text: String = (0..rng.below(14))
                .map(|_| PIECES[rng.below(PIECES.len())])
                .collect();
            for boxes in [Boxes::Plain, Boxes::Status] {
                for (line, mut read) in read(&text, boxes) {
                    assert_eq!(read.prose(), sorted_prose(&read), "{line:?}");
                    let tokens: Vec<Range<usize>> = (read.spans.iter())
                        .filter(|span| {
                            span.kind.is_task() && !matches!(span.kind, SpanKind::Box(_))
                        })
                        .map(|span| span.range.clone())
                        .collect();
                    read.not_prose.extend(tokens);
                    assert_eq!(read.prose(), sorted_prose(&read), "{line:?} {boxes:?}");
                }
            }
        }
    }

    #[test]
    fn spans_and_prose_are_well_formed_for_any_text() {
        const PIECES: &[&str] = &[
            "word",
            "é",
            "🐦",
            "说明",
            "\u{301}",
            " ",
            " ",
            "\t",
            "*",
            "**",
            "_",
            "__",
            "~~",
            "`",
            "``",
            "[",
            "]",
            "(",
            ")",
            "![",
            "](",
            "][",
            "<",
            ">",
            "<!--",
            "-->",
            "https://",
            "http://x.io/",
            "a@b",
            "#",
            "# ",
            "- ",
            "> ",
            "- [x] ",
            "- [ ] ",
            "- [/] ",
            "1. ",
            "```",
            "    ",
            "---",
            ":",
            "]:",
            "\\",
            "^id",
            "a_b",
            "@sam",
            "#tag",
            "📅 ",
            "✅ ",
            "2026-12-01",
            "2026-02-30",
            "[due:: 2026-11-18]",
            "⏫",
            "🔽\u{fe0f}",
            "🆔 a1",
        ];
        let mut rng = Rng(0x5eed_0f5b_a115_0001);
        for doc in 0..4000 {
            let lines: Vec<String> = (0..1 + rng.below(8))
                .map(|_| {
                    (0..rng.below(14))
                        .map(|_| PIECES[rng.below(PIECES.len())])
                        .collect()
                })
                .collect();
            let text = lines.join("\n");
            for boxes in [Boxes::Plain, Boxes::Status] {
                for (index, (line, read)) in read(&text, boxes).into_iter().enumerate() {
                    let what = format!("doc {doc} line {index} {line:?} {boxes:?}: {read:?}");
                    let on_line = |range: &Range<usize>| {
                        range.start <= range.end && line.get(range.clone()).is_some()
                    };
                    assert!(on_line(&read.text), "text off the line in {what}");
                    let mut previous: Option<&Range<usize>> = None;
                    for span in &read.spans {
                        let range = &span.range;
                        assert!(!range.is_empty() && on_line(range), "span in {what}");
                        assert!(
                            previous.is_none_or(|p| p.start < range.start
                                || (p.start == range.start && p.end >= range.end)),
                            "not sorted in {what}"
                        );
                        previous = Some(range);
                        let inside = |part: &Range<usize>| {
                            on_line(part) && range.start <= part.start && part.end <= range.end
                        };
                        assert!(span.marks().all(|part| inside(&part)), "marks in {what}");
                        assert!(
                            span.not_prose().all(|part| inside(&part)),
                            "exclusion in {what}"
                        );
                        assert!(span.inner().is_none_or(|part| inside(&part)), "{what}");
                        match &span.kind {
                            SpanKind::Link {
                                text, destination, ..
                            } => {
                                assert!(inside(text), "link text in {what}");
                                assert!(destination.as_ref().is_none_or(inside), "{what}");
                            }
                            SpanKind::Date { date: part, .. }
                            | SpanKind::Owner { name: part }
                            | SpanKind::Id { id: part } => assert!(inside(part), "{what}"),
                            _ => {}
                        }
                        if span.kind.is_task() {
                            assert_eq!(boxes, Boxes::Status, "a task span in {what}");
                        } else {
                            assert!(
                                read.text.start <= range.start && range.end <= read.text.end,
                                "inline Markdown outside the text in {what}"
                            );
                        }
                    }
                    assert!(
                        read.not_prose.iter().all(|range| {
                            on_line(range)
                                && read.text.start <= range.start
                                && range.end <= read.text.end
                        }),
                        "an exclusion outside the text in {what}"
                    );
                    let mut end = None;
                    for range in read.prose() {
                        assert!(!range.is_empty() && on_line(&range), "prose in {what}");
                        assert!(
                            read.text.start <= range.start && range.end <= read.text.end,
                            "prose outside the text in {what}"
                        );
                        assert!(end.is_none_or(|end| end < range.start), "prose in {what}");
                        end = Some(range.end);
                    }
                }
            }
        }
    }
}
