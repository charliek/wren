//! A Markdown syntax highlighter that colours spans from a [`Palette`].
//!
//! Implements `iced::core::text::highlighter::Highlighter`. Lines are
//! classified by [`wren_markdown::block`], the classifier the rest of Wren
//! uses, so nested lists, quotes, `~~~` and container-indented fences,
//! indented code, and front matter highlight the way they edit. What lies
//! inside a line comes from the same classifier as typed spans
//! ([`wren_markdown::span`]): the highlighter decides none of it, and only
//! gives each span its colour and face from the host's [`Palette`], rather
//! than from a fixed third-party theme.
//!
//! Supported constructs:
//!   - Front matter: muted, with faint delimiters.
//!   - Fenced and indented code: muted body, faint fences; fenced blocks get
//!     the widget's code band.
//!   - ATX headings (`#`–`######`): faint markers, bold `heading` H1,
//!     `heading2` H2, and primary H3–H6. Each span carries a [`Scale`]:
//!     H1–H3 text is set larger and their rows are taller, on engines that
//!     can size spans.
//!   - Block quotes: faint markers, italic `quote` text.
//!   - List markers and the boxes of unfinished tasks: accent colour. With
//!     the host's tasks on that is an open box, a doing one (`[/]`) and one
//!     whose character has no status; the character itself tells them
//!     apart.
//!   - Finished tasks, done and with tasks on cancelled (`[-]`): faint
//!     markers and `completed` text, struck through.
//!   - With tasks on, a task's tokens: a due date in `date`, or `overdue`
//!     once today has passed it on an unfinished task (bold) or when it is
//!     no real day; `@owners` in `owner`; stamps and ids faint. A priority
//!     glyph keeps the text's colour. On a finished task the strike stops at
//!     the description and the tokens after it are faint.
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
//! widget to draw: strike-throughs, the code band, and inline code chips,
//! plus the prose ranges spell check reads. It also leaves the typed spans
//! it was coloured from ([`MarkdownHighlighter::spans`]), for the hit test
//! ([`crate::hit`]).
//!
//! All byte ranges returned are valid UTF-8 boundaries.

use std::ops::Range;

use iced::advanced::text::highlighter::{self, Format};
use iced::font::{self, Font};
use iced::Color;
use wren_markdown::block::{self, Boxes, Container, Kind, LineInfo, Scanner};
use wren_markdown::date::{Date, Parsed};
use wren_markdown::span::{self, LineSpans, Span, SpanKind};
use wren_markdown::task::DateField;
use wren_markdown::TaskOptions;

use crate::Palette;

const CHECKPOINT_INTERVAL: usize = 50;

/// Heading text sizes in ems, H1 first.
const HEADING_EMS: [f32; 6] = [1.35, 1.18, 1.05, 1.0, 1.0, 1.0];

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// How a span is sized against body text, as multiples of the text size
/// and of the row height. iced's `Format` has no size, so only the owned
/// engine applies it; classic sets every span at the body size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub text: f32,
    /// The row's height; a row is as tall as its tallest span asks.
    pub row: f32,
}

impl Scale {
    pub const BODY: Self = Self {
        text: 1.0,
        row: 1.0,
    };

    /// A level-`level` heading's text.
    fn heading(level: u8) -> Self {
        let em = HEADING_EMS[usize::from(level.clamp(1, 6)) - 1];
        Self { text: em, row: em }
    }

    /// A heading's markers: the body size, in the heading's row.
    fn heading_markers(level: u8) -> Self {
        Self {
            text: 1.0,
            ..Self::heading(level)
        }
    }
}

/// Highlight produced for a byte range within a line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlight {
    color: Option<Color>,
    font: Option<Font>,
    scale: Scale,
}

impl Highlight {
    /// Convert to the iced `Format` expected by `highlight_with`.
    pub fn to_format(self) -> Format<Font> {
        Format {
            color: self.color,
            font: self.font,
        }
    }

    pub fn scale(self) -> Scale {
        self.scale
    }

    fn colored(color: Color) -> Self {
        Self {
            color: Some(color),
            font: None,
            scale: Scale::BODY,
        }
    }

    fn bold(color: Color, base: Font) -> Self {
        Self {
            color: Some(color),
            font: Some(Font {
                weight: font::Weight::Bold,
                ..base
            }),
            scale: Scale::BODY,
        }
    }

    fn italic(color: Color, base: Font) -> Self {
        Self {
            color: Some(color),
            font: Some(Font {
                style: font::Style::Italic,
                ..base
            }),
            scale: Scale::BODY,
        }
    }

    fn sized(self, scale: Scale) -> Self {
        Self { scale, ..self }
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
    /// The host's tasks, when they are on: a box of any status is then a
    /// task. `None` reads the lines as an editor without tasks does.
    pub tasks: Option<TaskOptions>,
}

impl Settings {
    /// The task boxes these settings read.
    fn boxes(&self) -> Boxes {
        Boxes::with_tasks(self.tasks.is_some())
    }
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
    /// The byte that wrapped rows hang under: the end of the line's
    /// container prefix, so a list item's rows line up under its content
    /// and a quote's under the text after `>`.
    pub hang: usize,
    /// The byte ranges spell check reads: sorted, apart, non-empty, and on
    /// character boundaries.
    pub prose: Vec<Range<usize>>,
}

// ---------------------------------------------------------------------------
// Span colours
// ---------------------------------------------------------------------------

/// The palette plus the font whose bold and italic variants style emphasis,
/// and the day a task's due date is read against.
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
    date: Color,
    overdue: Color,
    owner: Color,
    font: Font,
    /// Today, with the host's tasks on.
    today: Option<Date>,
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
            date,
            overdue,
            owner,
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
            date,
            overdue,
            owner,
            font,
            today: None,
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
    /// `spans[i]`, the typed spans line `i` was highlighted from, for the
    /// same lines.
    spans: Vec<LineSpans>,
}

impl highlighter::Highlighter for MarkdownHighlighter {
    type Settings = Settings;
    type Highlight = Highlight;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Highlight)>;

    fn new(settings: &Settings) -> Self {
        let scanner = Scanner::with_front_matter(settings.front_matter_end, settings.boxes());
        Self {
            settings: *settings,
            checkpoints: vec![scanner.clone()],
            scanner,
            decor: Vec::new(),
            spans: Vec::new(),
        }
    }

    fn update(&mut self, new_settings: &Settings) {
        *self = Self::new(new_settings);
    }

    fn change_line(&mut self, line: usize) {
        // Nothing highlighted so far is at or below `line`.
        if line >= self.scanner.line_index() {
            return;
        }
        let index = (line / CHECKPOINT_INTERVAL).min(self.checkpoints.len() - 1);
        self.checkpoints.truncate(index + 1);
        self.scanner = self.checkpoints[index].clone();
        self.decor.truncate(self.scanner.line_index());
        self.spans.truncate(self.scanner.line_index());
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
        let ink = Ink {
            today: self.settings.tasks.map(|tasks| tasks.today),
            ..Ink::new(self.settings.palette, self.settings.font)
        };
        let boxes = self.settings.boxes();
        let (spans, decor, read) = line_highlights(line, index, &info, own, continued, boxes, &ink);

        self.decor.truncate(index);
        self.decor.push(decor);
        self.spans.truncate(index);
        self.spans.push(read);
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

    /// The typed spans of `line`, under the same condition.
    pub fn spans(&self, line: usize) -> Option<&LineSpans> {
        self.spans.get(line)
    }
}

/// One line's spans, sorted and apart.
#[cfg(feature = "test-support")]
pub type Highlights = Vec<(Range<usize>, Highlight)>;

/// One line's spans and decoration.
#[cfg(any(test, feature = "test-support"))]
pub type HighlightedLine = (Vec<(Range<usize>, Highlight)>, LineDecor);

/// Every line of `text`, split at `\n`, as a fresh highlighter run gives it
/// with `palette`, `font` and the host's `tasks`: its spans and its
/// decoration.
#[cfg(feature = "test-support")]
pub fn highlight_lines(
    text: &str,
    palette: Palette,
    font: Font,
    tasks: Option<TaskOptions>,
) -> Vec<HighlightedLine> {
    let lines: Vec<&str> = text.split('\n').collect();
    let settings = Settings {
        palette,
        font,
        front_matter_end: block::front_matter_end(&lines[..]),
        tasks,
    };
    highlight_all(&lines[..], settings)
}

/// Every line of `lines` as a fresh highlighter with `settings` gives it.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn highlight_all<L: wren_markdown::Lines + ?Sized>(
    lines: &L,
    settings: Settings,
) -> Vec<HighlightedLine> {
    let mut highlighter = <MarkdownHighlighter as highlighter::Highlighter>::new(&settings);
    (0..lines.line_count())
        .map(|index| {
            let spans =
                highlighter::Highlighter::highlight_line(&mut highlighter, lines.line(index))
                    .collect();
            (spans, highlighter.decor[index].clone())
        })
        .collect()
}

/// Line `index` of `lines` as a fresh highlighter with `settings` gives it,
/// highlighting only the lines down to it: its spans and the typed spans
/// they were coloured from. None past the last line.
#[cfg(feature = "test-support")]
pub(crate) fn highlight_through<L: wren_markdown::Lines + ?Sized>(
    lines: &L,
    index: usize,
    settings: Settings,
) -> Option<(Highlights, LineSpans)> {
    if index >= lines.line_count() {
        return None;
    }
    let mut highlighter = <MarkdownHighlighter as highlighter::Highlighter>::new(&settings);
    for line in 0..index {
        let _ = highlighter::Highlighter::highlight_line(&mut highlighter, lines.line(line));
    }
    let spans =
        highlighter::Highlighter::highlight_line(&mut highlighter, lines.line(index)).collect();
    Some((spans, highlighter.spans.pop()?))
}

/// The typed spans of line `index` of `lines`, from a fresh scan of the
/// lines down to it, for a caller that cannot see the widget's own. Spans
/// depend on neither colour nor font, so only `front_matter_end` and `tasks`
/// of the widget's [`Settings`] matter. None past the last line.
pub fn spans_at(
    lines: &impl wren_markdown::Lines,
    index: usize,
    front_matter_end: Option<usize>,
    tasks: Option<TaskOptions>,
) -> LineSpans {
    if index >= lines.line_count() {
        return LineSpans::default();
    }
    let boxes = Boxes::with_tasks(tasks.is_some());
    let mut scanner = Scanner::with_front_matter(front_matter_end, boxes);
    for line in 0..index {
        scanner.next_line(lines.line(line));
    }
    let line = lines.line(index);
    read(line, &scanner.next_line(line), boxes)
}

/// The spans of `line`, classified as `info` by a scan with `boxes`, as the
/// editor reads them: with tasks on, a task's dates, priority, ids and the
/// tokens Wren keeps are not prose, so spell check reads none of them. Its
/// owners and tags stay prose, as the speller already passes over a chunk
/// with an `@` and a `#tag`.
fn read(line: &str, info: &LineInfo, boxes: Boxes) -> LineSpans {
    let mut read = span::spans(line, info, boxes);
    read.not_prose.extend(
        (read.spans.iter())
            .filter(|span| is_token(&span.kind))
            .map(|span| span.range.clone()),
    );
    read
}

/// Whether a task span is one of the tokens after a task's description,
/// not its box and not an owner or a tag, which may stand in the
/// description.
fn is_token(kind: &SpanKind) -> bool {
    matches!(
        kind,
        SpanKind::Date { .. }
            | SpanKind::Priority(_)
            | SpanKind::Id { .. }
            | SpanKind::BlockId
            | SpanKind::Kept(_)
    )
}

/// The prose ranges of line `index` of `lines`, the ones the widget's
/// highlighter leaves in its [`LineDecor`]; see [`spans_at`].
#[cfg(test)]
pub fn prose_at(
    lines: &impl wren_markdown::Lines,
    index: usize,
    front_matter_end: Option<usize>,
    tasks: Option<TaskOptions>,
) -> Vec<Range<usize>> {
    spans_at(lines, index, front_matter_end, tasks).prose()
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
}

/// The spans and decoration of line `index`, classified as `info` by a scan
/// with `boxes`, and the typed spans they were read from. `own` are the
/// containers the line belongs to, the first `continued` of which were open
/// before it.
fn line_highlights(
    line: &str,
    index: usize,
    info: &LineInfo,
    own: &[Container],
    continued: usize,
    boxes: Boxes,
    ink: &Ink,
) -> (Vec<(Range<usize>, Highlight)>, LineDecor, LineSpans) {
    let read = read(line, info, boxes);
    let mut spans = Spans::default();
    let mut decor = LineDecor {
        hang: info.prefix_end,
        prose: read.prose(),
        ..LineDecor::default()
    };
    let body = info.prefix_end..line.len();

    if info.kind == Kind::FrontMatter {
        let delimiter = matches!(line[body.clone()].trim_end(), "---" | "...");
        let color = if delimiter { ink.faint } else { ink.muted };
        spans.push(body, Highlight::colored(color));
        return (spans.0, decor, read);
    }

    let task = info.item.and_then(|item| item.task);
    // A finished task, done or cancelled, recedes: faint markers, and a
    // dimmed, struck description. Any other box takes the accent, and its
    // own character tells doing or an unknown status from open.
    let done = task.is_some_and(|task| task.status.is_finished());
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
            let faint = Highlight::colored(ink.faint).sized(Scale::heading_markers(*level));
            if text.is_empty() {
                spans.push(body, faint);
            } else {
                let color = match level {
                    1 => ink.heading,
                    2 => ink.heading2,
                    _ => ink.text,
                };
                spans.push(info.prefix_end..text.start, faint);
                spans.push(
                    text.clone(),
                    Highlight::bold(color, ink.font).sized(Scale::heading(*level)),
                );
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
            // Done text recedes, but its links keep their colour: the links
            // it has when read alone, whatever code or comment holds them.
            // With tasks on, the strike stops at the description, and the
            // tokens after it recede unstruck.
            let end = (read.spans.iter())
                .filter(|span| is_token(&span.kind))
                .map(|span| span.range.start)
                .min()
                .map_or(line.len(), |start| start.max(info.text_start));
            let text =
                info.text_start..info.text_start + line[info.text_start..end].trim_end().len();
            let completed = Highlight::colored(ink.completed);
            let links = span::links(line, text.clone());
            let mut covered = text.start;
            for (range, highlight) in inline_highlights(line, text.clone(), &links, ink) {
                spans.push(covered..range.start, completed);
                covered = range.end;
                spans.push(range, highlight);
            }
            spans.push(covered..text.end, completed);
            spans.push(text.end..line.len(), Highlight::colored(ink.faint));
            decor.strikes.push(text);
        }
        Kind::Prose => {
            let text = read.text.clone();
            for span in &read.spans {
                match span.kind {
                    SpanKind::Code => decor.code_spans.push(span.range.clone()),
                    SpanKind::Strike { .. } => decor
                        .strikes
                        .extend(span.inner().filter(|inner| !inner.is_empty())),
                    _ => {}
                }
            }
            // Quoted text between the inline spans is italic.
            let quote = (info.quote_depth > 0).then(|| Highlight::italic(ink.quote, ink.font));
            let mut covered = text.start;
            for (range, highlight) in inline_highlights(line, text.clone(), &read.spans, ink) {
                if let Some(quote) = quote {
                    spans.push(covered..range.start, quote);
                }
                covered = range.end;
                spans.push(range, highlight);
            }
            if let Some(quote) = quote {
                spans.push(covered..text.end, quote);
            }
        }
    }
    (spans.0, decor, read)
}

// ---------------------------------------------------------------------------
// Inline spans to colours
// ---------------------------------------------------------------------------

/// Which colour shows where inline spans overlap, the higher the stronger:
/// emphasised text keeps the colour of a strike it is in, a task's token
/// recolours emphasis but not a strike, struck text keeps the colour of a
/// link, and nothing recolours code, a comment, a destination or the
/// characters that are syntax alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Layer {
    #[default]
    None,
    Emphasis,
    Task,
    Struck,
    Link,
    Own,
}

/// The look of one byte of a line's text.
#[derive(Clone, Copy, Default)]
struct Cell {
    color: Option<Color>,
    layer: Layer,
    bold: bool,
    italic: bool,
    /// Code and comments keep their face inside emphasis.
    fixed_face: bool,
}

/// Whether `span` is inline Markdown or a task's token the highlighter
/// colours. A link nested in another is drawn as that one's text, and an
/// autolink, a reference definition, a task's box, a priority glyph, a tag,
/// a block id and the tokens Wren keeps have no colour of their own.
fn paints(span: &Span) -> bool {
    matches!(
        span.kind,
        SpanKind::Code
            | SpanKind::Comment
            | SpanKind::Url
            | SpanKind::Strike { .. }
            | SpanKind::Emphasis { .. }
            | SpanKind::Link { nested: false, .. }
            | SpanKind::Date { .. }
            | SpanKind::Owner { .. }
            | SpanKind::Id { .. }
    )
}

/// How an unfinished task's token looks: its colour, and whether it is
/// bold. A due date is in `date`, and `overdue` when it is before today,
/// bold, or when it is no real day, not bold: a problem to see, quieter
/// than work that is late. Due today is not late. A done or cancelled stamp
/// and an id are `faint`, an owner is `owner`.
fn token_look(kind: &SpanKind, ink: &Ink) -> Option<(Color, bool)> {
    Some(match kind {
        SpanKind::Date {
            field: DateField::Due,
            value,
            ..
        } => match value {
            Parsed::Invalid => (ink.overdue, false),
            Parsed::Date(date) if ink.today.is_some_and(|today| *date < today) => {
                (ink.overdue, true)
            }
            Parsed::Date(_) => (ink.date, false),
        },
        SpanKind::Date { .. } | SpanKind::Id { .. } => (ink.faint, false),
        SpanKind::Owner { .. } => (ink.owner, false),
        _ => return None,
    })
}

/// The coloured pieces of a span that [`paints`], in line order:
///
/// - Code spans: `code`. Comments, a link's brackets and the delimiters of
///   strikes and emphasis: `faint`.
/// - Links: `link` text and a `completed` destination. Bare URLs: `link`.
/// - Struck text: `completed`. Emphasised text: `text`.
/// - A task's tokens: see [`token_look`].
fn pieces(span: &Span, ink: &Ink, mut piece: impl FnMut(Range<usize>, Color, Layer)) {
    let range = span.range.clone();
    if let Some((color, _)) = token_look(&span.kind, ink) {
        piece(range, color, Layer::Task);
        return;
    }
    match &span.kind {
        SpanKind::Code => piece(range, ink.code, Layer::Own),
        SpanKind::Comment => piece(range, ink.faint, Layer::Own),
        SpanKind::Url => piece(range, ink.link, Layer::Link),
        SpanKind::Link {
            text, destination, ..
        } => {
            piece(text.start - 1..text.start, ink.faint, Layer::Own);
            piece(text.clone(), ink.link, Layer::Link);
            match destination {
                Some(destination) => {
                    piece(text.end..destination.start, ink.faint, Layer::Own);
                    piece(destination.clone(), ink.completed, Layer::Own);
                    piece(destination.end..range.end, ink.faint, Layer::Own);
                }
                None => piece(text.end..range.end, ink.faint, Layer::Own),
            }
        }
        SpanKind::Strike { .. } | SpanKind::Emphasis { .. } => {
            let Some([open, close]) = span.delimiters() else {
                return;
            };
            let (color, layer) = match span.kind {
                SpanKind::Strike { .. } => (ink.completed, Layer::Struck),
                _ => (ink.text, Layer::Emphasis),
            };
            piece(open.clone(), ink.faint, Layer::Own);
            piece(open.end..close.start, color, layer);
            piece(close, ink.faint, Layer::Own);
        }
        _ => {}
    }
}

/// `base` in the bold or italic face, or `None` for neither.
fn face(base: Font, bold: bool, italic: bool) -> Option<Font> {
    (bold || italic).then_some(Font {
        weight: if bold {
            font::Weight::Bold
        } else {
            base.weight
        },
        style: if italic {
            font::Style::Italic
        } else {
            base.style
        },
        ..base
    })
}

/// The highlights of the inline Markdown `spans` find in `text` of `line`:
/// sorted, apart, touching ones that look the same merged; see [`pieces`]
/// for the colours. Emphasis is in the bold or italic face, which its
/// delimiters share: cosmic-text shapes a word in the font of its first
/// character, so a regular-weight `**` would leave `**Alice` regular.
fn inline_highlights(
    line: &str,
    text: Range<usize>,
    spans: &[Span],
    ink: &Ink,
) -> Vec<(Range<usize>, Highlight)> {
    let painted = || spans.iter().filter(|span| paints(span));
    let mut highlights = Spans::default();
    // Most lines have no inline Markdown, and most that have any have none
    // inside other: their spans' pieces are the highlights as they come.
    let mut end = text.start;
    if painted().all(|span| std::mem::replace(&mut end, span.range.end) <= span.range.start) {
        for span in painted() {
            let font = match span.kind {
                SpanKind::Emphasis {
                    strong, emphasis, ..
                } => face(ink.font, strong, emphasis),
                _ => token_look(&span.kind, ink).and_then(|(_, bold)| face(ink.font, bold, false)),
            };
            pieces(span, ink, |range, color, _| {
                let highlight = Highlight {
                    color: Some(color),
                    font,
                    scale: Scale::BODY,
                };
                highlights.push(range, highlight);
            });
        }
        return highlights.0;
    }

    // Spans in spans: each byte takes the colour of its strongest layer
    // and the faces of all the emphasis around it.
    let mut cells = vec![Cell::default(); text.len()];
    let within = |range: Range<usize>| range.start - text.start..range.end - text.start;
    for span in painted() {
        let bold = token_look(&span.kind, ink).is_some_and(|(_, bold)| bold);
        pieces(span, ink, |range, color, layer| {
            for cell in &mut cells[within(range)] {
                if layer >= cell.layer {
                    cell.color = Some(color);
                    cell.layer = layer;
                    // A bold token is bold where its colour shows.
                    cell.bold |= bold;
                }
            }
        });
        let cells = &mut cells[within(span.range.clone())];
        match span.kind {
            SpanKind::Code | SpanKind::Comment => {
                cells.iter_mut().for_each(|cell| cell.fixed_face = true);
            }
            SpanKind::Emphasis {
                strong, emphasis, ..
            } => cells.iter_mut().for_each(|cell| {
                cell.bold |= strong;
                cell.italic |= emphasis;
            }),
            _ => {}
        }
    }
    for (at, character) in line[text.clone()].char_indices() {
        let cell = cells[at];
        let font = face(
            ink.font,
            cell.bold && !cell.fixed_face,
            cell.italic && !cell.fixed_face,
        );
        if cell.color.is_some() || font.is_some() {
            let highlight = Highlight {
                color: cell.color,
                font,
                scale: Scale::BODY,
            };
            let start = text.start + at;
            highlights.push(start..start + character.len_utf8(), highlight);
        }
    }
    highlights.0
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
            tasks: None,
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
    fn headings_scale_their_text_and_rows_but_markers_keep_the_body_size() {
        let scales = |line: &str| -> Vec<(String, f32, f32)> {
            highlights_for(line)
                .into_iter()
                .map(|(range, h)| (line[range].to_owned(), h.scale.text, h.scale.row))
                .collect()
        };
        let row = |text: &str, em: f32| vec![("# ".to_owned(), 1.0, em), (text.to_owned(), em, em)];
        assert_eq!(scales("# Title"), row("Title", 1.35));
        assert_eq!(
            scales("## Title ##"),
            [
                ("## ".to_owned(), 1.0, 1.18),
                ("Title".to_owned(), 1.18, 1.18),
                (" ##".to_owned(), 1.0, 1.18)
            ]
        );
        assert_eq!(scales("### T")[1], ("T".to_owned(), 1.05, 1.05));
        for line in ["#### T", "###### T", "Body", "- item", "#notaheading"] {
            assert!(
                highlights_for(line)
                    .iter()
                    .all(|(_, h)| h.scale == Scale::BODY),
                "{line}"
            );
        }
        // An empty heading already takes its row's height.
        assert_eq!(scales("#"), [("#".to_owned(), 1.0, 1.35)]);
        // A quote's marker stays body sized inside a heading row.
        let quoted = scales("> # Q");
        assert_eq!(quoted[0], ("> ".to_owned(), 1.0, 1.0));
        assert_eq!(quoted[2], ("Q".to_owned(), 1.35, 1.35));
    }

    #[test]
    fn closing_hashes_and_quoted_headings_are_marked_up() {
        let p = dark_palette();
        let line = "> ## Title ##";
        let spans = highlights_for(line);
        assert_eq!(
            colored(line, &spans),
            [
                // The quote marker keeps the body row; the heading's sizes it.
                ("> ", Some(p.faint)),
                ("## ", Some(p.faint)),
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

    /// Tasks on, on the day every task test pins.
    fn tasks_on() -> Option<TaskOptions> {
        Some(TaskOptions {
            today: wren_markdown::date::Date::new(2026, 11, 19).expect("a date"),
        })
    }

    /// [`highlight_text`] for a host with these `tasks`.
    fn highlight_tasks(
        palette: Palette,
        text: &str,
        tasks: Option<TaskOptions>,
    ) -> Vec<(LineSpans, LineDecor)> {
        let lines: Vec<&str> = text.split('\n').collect();
        let settings = Settings {
            tasks,
            ..settings(palette, block::front_matter_end(&lines[..]))
        };
        highlight_all(&lines[..], settings)
    }

    /// With tasks on, the box of an unfinished task (open, doing, or a
    /// character with no status) takes the accent with its marker, whatever
    /// the character's width, and its text stays plain.
    #[test]
    fn unfinished_status_boxes_use_the_accent_with_tasks_on() {
        for palette in [fixtures::DARK, fixtures::LIGHT] {
            let p = Ink::new(palette, fixtures::FONT);
            for (line, marker) in [
                ("- [ ] open", "- [ ] "),
                ("- [/] doing", "- [/] "),
                ("- [a] unknown", "- [a] "),
                ("- [1] unknown", "- [1] "),
                ("- [[] unknown", "- [[] "),
                ("- [é] unknown", "- [é] "),
                ("- [界] unknown", "- [界] "),
                ("- [😀]\tunknown", "- [😀]\t"),
                ("12. [/]  doing", "12. [/]  "),
                ("- [/]", "- [/]"),
            ] {
                let (spans, decor) = highlight_tasks(palette, line, tasks_on()).remove(0);
                assert_eq!(colored(line, &spans), [(marker, Some(p.accent))], "{line}");
                assert!(decor.strikes.is_empty(), "{line}");
                assert_eq!(decor.hang, line.find('[').unwrap(), "{line}");
            }
            // Inside a quote the box is the accent's and the quote's marker
            // and text are the quote's.
            let line = "> - [界] quoted";
            let (spans, _) = highlight_tasks(palette, line, tasks_on()).remove(0);
            assert_eq!(
                colored(line, &spans),
                [
                    ("> ", Some(p.faint)),
                    ("- [界] ", Some(p.accent)),
                    ("quoted", Some(p.quote)),
                ]
            );
        }
    }

    /// With tasks on, a cancelled task recedes exactly as a done one does:
    /// faint markers, `completed` text and a strike over the description.
    #[test]
    #[allow(clippy::single_range_in_vec_init)] // a struck line has one strike
    fn done_and_cancelled_tasks_recede_alike_with_tasks_on() {
        for palette in [fixtures::DARK, fixtures::LIGHT] {
            let p = Ink::new(palette, fixtures::FONT);
            for (line, marker, text) in [
                ("- [x] completed", "- [x] ", "completed"),
                ("- [X] completed", "- [X] ", "completed"),
                ("- [-] cancelled", "- [-] ", "cancelled"),
                ("  1. [-]  **cancelled**  ", "1. [-]  ", "**cancelled**"),
                // The quote's faint marker runs into the item's.
                ("> - [-] quoted", "> - [-] ", "quoted"),
            ] {
                let (spans, decor) = highlight_tasks(palette, line, tasks_on()).remove(0);
                let colored = colored(line, &spans);
                assert_eq!(colored[0], (marker, Some(p.faint)), "{line}");
                assert_eq!(colored[1], (text, Some(p.completed)), "{line}");
                assert!(colored[2..]
                    .iter()
                    .all(|(_, color)| *color == Some(p.faint)));
                let start = line.find(text).unwrap();
                assert_eq!(decor.strikes, [start..start + text.len()], "{line}");
            }
            // A cancelled task's link keeps its colour under the strike.
            let line = "- [-] see [docs](u)";
            let (spans, decor) = highlight_tasks(palette, line, tasks_on()).remove(0);
            assert_eq!(decor.strikes, [6..line.len()]);
            assert!(colored(line, &spans).contains(&("docs", Some(p.link))));
        }
    }

    /// With tasks off the same lines are what they were before tasks: only
    /// `[ ]`, `[x]` and `[X]` are boxes, and any other bracket is the
    /// item's text.
    #[test]
    fn with_tasks_off_only_the_three_plain_boxes_are_highlighted() {
        let p = dark_palette();
        for line in [
            "- [/] doing",
            "- [-] cancelled",
            "- [a] unknown",
            "- [界] unknown",
            "- [/]",
        ] {
            let off = highlight_tasks(fixtures::DARK, line, None).remove(0);
            assert_eq!(
                off,
                highlight_text(fixtures::DARK, line).remove(0),
                "{line}"
            );
            let (spans, decor) = off;
            assert_eq!(colored(line, &spans)[0], ("- ", Some(p.accent)), "{line}");
            assert!(decor.strikes.is_empty(), "{line}");
            assert_eq!(decor.hang, 2, "{line}");
            let on = highlight_tasks(fixtures::DARK, line, tasks_on()).remove(0);
            assert_ne!(on.0, spans, "{line}");
        }
        // The three plain boxes look the same either way.
        for line in ["- [ ] open", "- [x] done", "1. [X] done", "- item", "text"] {
            assert_eq!(
                highlight_tasks(fixtures::DARK, line, None),
                highlight_tasks(fixtures::DARK, line, tasks_on()),
                "{line}"
            );
        }
        // Not boxes with tasks on either.
        for line in ["- [✔\u{fe0f}] a", "- []] a", "- [a]: http://u", "[/] a"] {
            assert_eq!(
                highlight_tasks(fixtures::DARK, line, None),
                highlight_tasks(fixtures::DARK, line, tasks_on()),
                "{line}"
            );
        }
    }

    /// Spell check reads a task's description, never its box: with tasks
    /// on the prose of a status task starts after the box, and a finished
    /// one keeps its prose under the strike.
    #[test]
    fn the_prose_of_a_status_task_starts_after_its_box() {
        let prose = |line: &str, tasks| -> Vec<String> {
            prose_at(&[line], 0, None, tasks)
                .into_iter()
                .map(|range| line[range].to_owned())
                .collect()
        };
        for (line, text) in [
            ("- [/] doing task", "doing task"),
            ("- [-] dropped task", "dropped task"),
            ("- [a] unknown task", "unknown task"),
            ("- [界] wide box", "wide box"),
            ("> 1. [😀]\tquoted", "quoted"),
            ("- [x] done task", "done task"),
        ] {
            assert_eq!(prose(line, tasks_on()), [text], "{line}");
            assert_eq!(
                highlight_tasks(fixtures::DARK, line, tasks_on())
                    .remove(0)
                    .1
                    .prose,
                prose_at(&[line], 0, None, tasks_on()),
                "{line}"
            );
        }
        assert_eq!(prose("- [/]", tasks_on()), [] as [&str; 0]);
        // Off, the bracket is part of the item's text, as it was.
        for line in ["- [/] doing task", "- [a] unknown task", "- [界] wide box"] {
            let off = prose(line, None);
            assert_ne!(off, prose(line, tasks_on()), "{line}");
            assert_eq!(
                prose_at(&[line], 0, None, None),
                highlight_text(fixtures::DARK, line).remove(0).1.prose,
                "{line}"
            );
        }
    }

    /// A change of the host's tasks is a change of settings, so the
    /// widget's highlighter starts again and reads every line anew.
    #[test]
    fn a_change_of_tasks_rehighlights_from_the_first_line() {
        let line = "- [/] doing";
        let off = settings(fixtures::DARK, None);
        let on = Settings {
            tasks: tasks_on(),
            ..off
        };
        assert_ne!(off, on);
        let later = Settings {
            tasks: Some(TaskOptions {
                today: wren_markdown::date::Date::new(2026, 11, 20).expect("a date"),
            }),
            ..off
        };
        assert_ne!(on, later, "a new day is a new setting");
        let mut h = MarkdownHighlighter::new(&off);
        let before: LineSpans = h.highlight_line(line).collect();
        assert_eq!(h.current_line(), 1);
        h.update(&on);
        assert_eq!(h.current_line(), 0);
        let after: LineSpans = h.highlight_line(line).collect();
        assert_ne!(before, after);
        assert_eq!(
            after,
            highlight_tasks(fixtures::DARK, line, tasks_on())[0].0
        );
    }

    /// The colour and boldness of the span over `token`, the first place it
    /// appears in `line`: `None` where no span covers it, which draws it in
    /// the body colour.
    fn token_at(
        line: &str,
        spans: &[(Range<usize>, Highlight)],
        token: &str,
    ) -> Option<(Color, bool)> {
        let start = line
            .find(token)
            .unwrap_or_else(|| panic!("{token:?} in {line:?}"));
        let end = start + token.len();
        let covering: Vec<_> = spans
            .iter()
            .filter(|(range, _)| range.start < end && start < range.end)
            .collect();
        let (range, highlight) = covering.first()?;
        assert!(
            covering.len() == 1 && range.start <= start && end <= range.end,
            "{token:?} is one span in {line:?}: {covering:?}"
        );
        let bold = highlight
            .font
            .is_some_and(|font| font.weight == font::Weight::Bold);
        Some((highlight.color?, bold))
    }

    /// With tasks on, an unfinished task's due date is in the date colour,
    /// or overdue and bold when it is before today, whatever its status;
    /// due today is not late, and a date that is no real day is overdue's
    /// colour without the weight. Owners take theirs, stamps and ids
    /// recede, and a priority glyph, a tag, a block id and a kept token are
    /// drawn as the text around them.
    #[test]
    fn an_unfinished_tasks_tokens_take_their_colours_with_tasks_on() {
        for palette in [fixtures::DARK, fixtures::LIGHT] {
            let p = Ink::new(palette, fixtures::FONT);
            let date = Some((p.date, false));
            let overdue = Some((p.overdue, true));
            let invalid = Some((p.overdue, false));
            let faint = Some((p.faint, false));
            let owner = Some((p.owner, false));
            for (line, token, look) in [
                ("- [ ] Future 📅 2026-11-20", "📅 2026-11-20", date),
                ("- [ ] Next year 📅 2027-01-01", "📅 2027-01-01", date),
                ("- [ ] Today 📅 2026-11-19", "📅 2026-11-19", date),
                ("- [ ] Late 📅 2026-11-18", "📅 2026-11-18", overdue),
                ("- [/] Late doing 📅 2026-11-01", "📅 2026-11-01", overdue),
                ("- [a] Late unknown 📅 2025-12-31", "📅 2025-12-31", overdue),
                ("  1. [ ] Numbered 📅 2026-11-18", "📅 2026-11-18", overdue),
                ("> - [ ] Quoted 📅 2026-11-18", "📅 2026-11-18", overdue),
                ("- [ ] Alias 📆 2026-11-18", "📆 2026-11-18", overdue),
                (
                    "- [ ] Field [due:: 2026-11-18]",
                    "[due:: 2026-11-18]",
                    overdue,
                ),
                ("- [ ] Field [due:: 2026-11-20]", "[due:: 2026-11-20]", date),
                ("- [ ] No such day 📅 2026-02-30", "📅 2026-02-30", invalid),
                (
                    "- [ ] Past no such day 📅 2025-13-01",
                    "📅 2025-13-01",
                    invalid,
                ),
                ("- [ ] Ask @alice.chen ⏫", "@alice.chen", owner),
                ("- [ ] Ask @alice.chen ⏫", "⏫", None),
                ("- [ ] Ask @sam and @priya", "@priya", owner),
                ("- [ ] Stamped open ✅ 2026-11-18", "✅ 2026-11-18", faint),
                ("- [ ] Stamped open ❌ 2026-11-18", "❌ 2026-11-18", faint),
                ("- [ ] Card 🆔 a1 🔼", "🆔 a1", faint),
                ("- [ ] Card 🆔 a1 🔼", "🔼", None),
                ("- [ ] Low 🔽\u{fe0f} 📅 2026-11-20", "🔽\u{fe0f}", None),
                ("- [ ] Tagged #home ^b1", "#home", None),
                ("- [ ] Tagged #home ^b1", "^b1", None),
                (
                    "- [ ] Water 🔁 every week ⏳ 2026-11-01",
                    "⏳ 2026-11-01",
                    None,
                ),
                (
                    "- [ ] Water 🔁 every week ⏳ 2026-11-01",
                    "🔁 every week",
                    None,
                ),
            ] {
                let (spans, decor) = highlight_tasks(palette, line, tasks_on()).remove(0);
                assert_eq!(token_at(line, &spans, token), look, "{token:?} in {line:?}");
                assert!(decor.strikes.is_empty(), "{line:?}");
            }
        }
    }

    /// Each due date is read on its own, so on a line with two the one
    /// that is past is overdue.
    #[test]
    fn each_due_date_on_a_line_is_read_against_today() {
        let p = dark_palette();
        let line = "- [ ] Twice 📅 2026-11-01 📅 2026-12-01";
        let (spans, _) = highlight_tasks(fixtures::DARK, line, tasks_on()).remove(0);
        assert_eq!(
            token_at(line, &spans, "📅 2026-11-01"),
            Some((p.overdue, true))
        );
        assert_eq!(
            token_at(line, &spans, "📅 2026-12-01"),
            Some((p.date, false))
        );
    }

    /// A finished task recedes whole: its description struck, its tokens
    /// faint after the strike, a past due date never red, and its owners
    /// dimmed with the text they stand in.
    #[test]
    #[allow(clippy::single_range_in_vec_init)] // a struck line has one strike
    fn a_finished_tasks_tokens_recede_unstruck_with_tasks_on() {
        for palette in [fixtures::DARK, fixtures::LIGHT] {
            let p = Ink::new(palette, fixtures::FONT);
            for (line, description, tail) in [
                (
                    "- [x] Ship the deck @sam 📅 2026-11-01 ✅ 2026-11-18",
                    "Ship the deck @sam",
                    "📅 2026-11-01 ✅ 2026-11-18",
                ),
                (
                    "- [-] Book the room @sam ❌ 2026-11-16",
                    "Book the room @sam",
                    "❌ 2026-11-16",
                ),
                (
                    "- [X] Late but done ⏫ 📅 2026-11-01  ",
                    "Late but done",
                    "⏫ 📅 2026-11-01  ",
                ),
                (
                    "> - [x] See [docs](u) 🆔 a1 ^b1",
                    "See [docs](u)",
                    "🆔 a1 ^b1",
                ),
            ] {
                let (spans, decor) = highlight_tasks(palette, line, tasks_on()).remove(0);
                let start = line.find(description).unwrap();
                assert_eq!(decor.strikes, [start..start + description.len()], "{line}");
                // Everything after the description is faint, to the end.
                let tail = line.find(tail).unwrap()..line.len();
                let after: Vec<_> = spans
                    .iter()
                    .filter(|(range, _)| range.end > start + description.len())
                    .collect();
                assert!(
                    after.iter().all(|(_, look)| look.color == Some(p.faint)),
                    "{line}: {after:?}"
                );
                assert!(after[0].0.start <= tail.start, "{line}: {after:?}");
                assert_eq!(after.last().map(|(range, _)| range.end), Some(tail.end));
                assert!(spans.iter().all(|(_, highlight)| highlight.font.is_none()));
                for (range, highlight) in &spans {
                    for color in [p.date, p.overdue, p.owner] {
                        assert_ne!(highlight.color, Some(color), "{line}: {range:?}");
                    }
                }
                if let Some(at) = line.find("@sam") {
                    assert_eq!(
                        token_at(line, &spans, "@sam"),
                        Some((p.completed, false)),
                        "{line}"
                    );
                    assert!(at < start + description.len());
                }
            }
            // A finished task's link keeps its colour under the strike.
            let line = "> - [x] See [docs](u) 🆔 a1 ^b1";
            let (spans, _) = highlight_tasks(palette, line, tasks_on()).remove(0);
            assert_eq!(token_at(line, &spans, "docs"), Some((p.link, false)));
            // With no token, the strike runs to the end of the text, as it
            // always has.
            let line = "- [x] All words  ";
            let (_, decor) = highlight_tasks(palette, line, tasks_on()).remove(0);
            assert_eq!(decor.strikes, [6..15]);
        }
    }

    /// A token inside a code span or a comment is code or a comment, and
    /// inside a strike or a link it is struck or linked text; emphasis
    /// around an owner keeps its face under the owner's colour.
    #[test]
    fn a_token_in_inline_markdown_yields_to_it() {
        let p = dark_palette();
        for (line, token, look) in [
            ("- [ ] Run ` @sam ` now", "@sam", (p.code, false)),
            ("- [ ] Run <!-- @sam --> now", "@sam", (p.faint, false)),
            ("- [ ] Ask ~~@sam~~ now", "@sam", (p.completed, false)),
            ("- [ ] Ask [@sam](u) now", "@sam", (p.link, false)),
        ] {
            let (spans, _) = highlight_tasks(fixtures::DARK, line, tasks_on()).remove(0);
            assert_eq!(token_at(line, &spans, token), Some(look), "{line}");
        }
        let line = "- [ ] **Ask @sam** now";
        let (spans, _) = highlight_tasks(fixtures::DARK, line, tasks_on()).remove(0);
        assert_eq!(token_at(line, &spans, "@sam"), Some((p.owner, true)));
        assert_eq!(token_at(line, &spans, "Ask"), Some((p.text, true)));
    }

    /// With tasks off a task line's tokens are text: none of them takes a
    /// colour of its own, and the line looks as it did before tasks.
    #[test]
    fn with_tasks_off_task_tokens_have_no_colour() {
        let p = dark_palette();
        for line in [
            "- [ ] Late @sam ⏫ 📅 2026-11-18 🆔 a1",
            "- [ ] No such day 📅 2026-02-30",
            "- [x] Done @sam 📅 2026-11-01 ✅ 2026-11-18",
            "- [/] Doing @sam 📅 2026-11-01",
        ] {
            let off = highlight_tasks(fixtures::DARK, line, None).remove(0);
            assert_eq!(
                off,
                highlight_text(fixtures::DARK, line).remove(0),
                "{line}"
            );
            for (range, highlight) in &off.0 {
                for color in [p.date, p.overdue, p.owner] {
                    assert_ne!(highlight.color, Some(color), "{line}: {range:?}");
                }
            }
            assert_ne!(
                off,
                highlight_tasks(fixtures::DARK, line, tasks_on()).remove(0)
            );
        }
    }

    /// With tasks on spell check reads a task's description and its owners
    /// and tags, which the speller passes over itself, but none of its
    /// other tokens. With tasks off it reads the whole text, as it did.
    #[test]
    fn task_tokens_leave_the_prose_with_tasks_on() {
        let prose = |line: &str, tasks| -> Vec<String> {
            prose_at(&[line], 0, None, tasks)
                .into_iter()
                .map(|range| line[range].to_owned())
                .collect()
        };
        for (line, on) in [
            (
                "- [ ] Call @sam #home ⏫ 📅 2026-11-18",
                vec!["Call @sam #home ", " "],
            ),
            (
                "- [ ] Card 🆔 a1 🔁 every week ⛔ abc,def ^b1",
                vec!["Card ", " ", " ", " "],
            ),
            (
                "- [ ] Field [due:: 2026-11-18] [k:: v]",
                vec!["Field ", " "],
            ),
            ("- [x] Shipped @sam ✅ 2026-11-18", vec!["Shipped @sam "]),
            (
                "- [-] Dropped ❌ 2026-11-18 📅 2026-11-01",
                vec!["Dropped ", " "],
            ),
            ("- [ ] 📅 2026-11-18", vec![]),
            ("- [ ] Plain words", vec!["Plain words"]),
            // Text after a token makes it description, and prose.
            (
                "- [ ] 📅 2026-12-01 then lunch",
                vec!["📅 2026-12-01 then lunch"],
            ),
        ] {
            assert_eq!(prose(line, tasks_on()), on, "{line}");
            assert_eq!(
                highlight_tasks(fixtures::DARK, line, tasks_on())
                    .remove(0)
                    .1
                    .prose,
                prose_at(&[line], 0, None, tasks_on()),
                "{line}"
            );
            // Off, the text is one piece of prose, to the end of the line.
            let off = prose(line, None);
            assert!(off.len() == 1 && line.ends_with(&off[0]), "{line}: {off:?}");
            assert_eq!(
                highlight_tasks(fixtures::DARK, line, None)
                    .remove(0)
                    .1
                    .prose,
                highlight_text(fixtures::DARK, line).remove(0).1.prose,
                "{line}"
            );
        }
    }

    /// A new day is a new setting: the widget's highlighter reads every
    /// line again, and a due date it passed is overdue.
    #[test]
    fn a_new_day_turns_a_due_date_it_passed_overdue() {
        let p = dark_palette();
        let line = "- [ ] Pay the invoice 📅 2026-11-20";
        let day = |day| Settings {
            tasks: Some(TaskOptions {
                today: wren_markdown::date::Date::new(2026, 11, day).expect("a date"),
            }),
            ..settings(fixtures::DARK, None)
        };
        let mut h = MarkdownHighlighter::new(&day(19));
        let before: LineSpans = h.highlight_line(line).collect();
        assert_eq!(
            token_at(line, &before, "📅 2026-11-20"),
            Some((p.date, false))
        );
        h.update(&day(20));
        assert_eq!(h.current_line(), 0);
        let today: LineSpans = h.highlight_line(line).collect();
        assert_eq!(today, before, "due today is not late");
        h.update(&day(21));
        assert_eq!(h.current_line(), 0);
        let after: LineSpans = h.highlight_line(line).collect();
        assert_eq!(
            token_at(line, &after, "📅 2026-11-20"),
            Some((p.overdue, true))
        );
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
        // A change at or past the next line to highlight keeps every line.
        for line in [CHECKPOINT_INTERVAL + 1, 10 * CHECKPOINT_INTERVAL] {
            h.change_line(line);
            assert_eq!(h.current_line(), CHECKPOINT_INTERVAL + 1);
            assert_eq!(h.decor.len(), CHECKPOINT_INTERVAL + 1);
        }
        h.change_line(3);
        assert_eq!(h.current_line(), 0);
        assert!(h.decor(0).is_none());
    }

    // ── Prose ranges ──────────────────────────────────────────────────────────

    /// Each line's prose ranges as the text they cover.
    fn prose_of(text: &str) -> Vec<Vec<&str>> {
        let lines: Vec<&str> = text.split('\n').collect();
        highlight_text(fixtures::DARK, text)
            .into_iter()
            .zip(&lines)
            .map(|((_, decor), line)| decor.prose.iter().map(|r| &line[r.clone()]).collect())
            .collect()
    }

    fn line_prose(line: &str) -> Vec<&str> {
        prose_of(line).remove(0)
    }

    #[test]
    fn paragraph_lines_list_items_quotes_and_tasks_are_prose_after_their_prefix() {
        for (line, prose) in [
            ("Plain words here", "Plain words here"),
            ("  indented a little ", "indented a little "),
            ("- item text", "item text"),
            ("12. ordered words", "ordered words"),
            ("> quoted text", "quoted text"),
            ("> > - deep item", "deep item"),
            ("- [ ] open task", "open task"),
            ("- [x] done task", "done task"),
            ("> - [X] quoted done", "quoted done"),
            ("| a cell | another |", "| a cell | another |"),
        ] {
            assert_eq!(line_prose(line), [prose], "{line:?}");
        }
        // A list item's continuation, and setext heading text.
        assert_eq!(
            prose_of("- item\n  more text\n\nTitle text\n=========="),
            [
                vec!["item"],
                vec!["more text"],
                vec![],
                vec!["Title text"],
                vec![]
            ]
        );
    }

    #[test]
    fn a_lazy_continuation_line_is_prose_of_its_paragraph() {
        assert_eq!(
            prose_of("> a quote\nlazy line\n- an item\nlazy too"),
            [
                vec!["a quote"],
                vec!["lazy line"],
                vec!["an item"],
                vec!["lazy too"]
            ]
        );
    }

    #[test]
    fn atx_heading_text_is_prose_without_its_hashes() {
        for (line, prose) in [
            ("# Title", vec!["Title"]),
            ("## Heading text ##", vec!["Heading text"]),
            ("> ### Quoted heading", vec!["Quoted heading"]),
            ("- # In an item", vec!["In an item"]),
            ("#", vec![]),
            ("## ##", vec![]),
        ] {
            assert_eq!(line_prose(line), prose, "{line:?}");
        }
    }

    #[test]
    fn headings_and_done_tasks_exclude_inline_markup_but_keep_their_look() {
        let heading = "# A `code` <!-- note --> _x_ end";
        assert_eq!(line_prose(heading), ["A ", " ", " ", "x", " end"]);
        let (spans, decor) = highlight_text(fixtures::DARK, heading).remove(0);
        assert_eq!(colored(heading, &spans).len(), 2, "{spans:?}");
        assert!(decor.code_spans.is_empty() && decor.strikes.is_empty());

        let task = "- [x] a `code` <!-- note --> _x_ [link](u) end";
        assert_eq!(line_prose(task), ["a ", " ", " ", "x", " ", "link", " end"]);
        let (spans, decor) = highlight_text(fixtures::DARK, task).remove(0);
        assert!(decor.code_spans.is_empty());
        assert_eq!(decor.strikes, vec![(6..task.len())]);
        assert!(spans.iter().all(|(_, highlight)| highlight.font.is_none()));
    }

    #[test]
    fn a_link_reference_definition_has_no_prose_but_a_footnote_does() {
        for line in [
            "[ref-labl]: https://example.com \"Titel\"",
            "[a label]:",
            "- [label]: https://x.io",
            "> [la\\]bel]: u",
        ] {
            assert_eq!(line_prose(line), Vec::<&str>::new(), "{line:?}");
        }
        for (line, prose) in [
            ("[^1]: a footnote", vec!["[^1]: a footnote"]),
            ("[label] : not one", vec!["[label] : not one"]),
            ("[]: empty label", vec!["[]: empty label"]),
            ("[a [b]]: nested", vec!["[a [b]]: nested"]),
            ("text [label]: later", vec!["text [label]: later"]),
        ] {
            assert_eq!(line_prose(line), prose, "{line:?}");
        }
    }

    #[test]
    fn code_front_matter_html_breaks_and_blank_lines_have_no_prose() {
        let text = "---\ntitle: Notes\n---\n```rust\nlet x = 1;\n```\n\n    indented code\n\n<div>\nhtml block\n</div>\n\n***\nSetext\n---\n~~~\nbody\n~~~";
        let prose = prose_of(text);
        let lines: Vec<&str> = text.split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            let expected: Vec<&str> = if *line == "Setext" {
                vec!["Setext"]
            } else {
                vec![]
            };
            assert_eq!(prose[i], expected, "line {i}: {line:?}");
        }
    }

    #[test]
    fn code_spans_and_comments_are_excluded() {
        for (line, prose) in [
            ("run `cargo test` now", vec!["run ", " now"]),
            ("a ``b ` c`` d", vec!["a ", " d"]),
            ("`all code`", vec![]),
            ("unclosed `tick", vec!["unclosed `tick"]),
            ("a <!-- hidden --> b", vec!["a ", " b"]),
            ("a <!-- open only", vec!["a <!-- open only"]),
        ] {
            assert_eq!(line_prose(line), prose, "{line:?}");
        }
    }

    #[test]
    fn only_a_links_text_is_prose() {
        for (line, prose) in [
            (
                "See [the desing note](https://example.com/spel) or",
                vec!["See ", "the desing note", " or"],
            ),
            ("![alt text](a.png) after", vec!["alt text", " after"]),
            (
                "Read [the guide][guide-ref] now",
                vec!["Read ", "the guide", " now"],
            ),
            ("[**bold** words](u)", vec!["bold", " words"]),
            ("[x](https://example.com/a_b_c)", vec!["x"]),
            ("[not a link] here", vec!["[not a link] here"]),
            ("wow! [a](u)", vec!["wow! ", "a"]),
            // A linked image: only the image's alt text is prose.
            ("[![alt texxt](img.png)](https://x.io)", vec!["alt texxt"]),
            ("[![logo](#logoo)](home) end", vec!["logo", " end"]),
            ("[a ![b](c) d](e)", vec!["a ", "b", " d"]),
            ("[a [b][r] c](e)", vec!["a ", "b", " c"]),
            ("[a [not] c](e)", vec!["a [not] c"]),
        ] {
            assert_eq!(line_prose(line), prose, "{line:?}");
        }
    }

    #[test]
    fn bare_urls_and_autolinks_are_excluded() {
        for (line, prose) in [
            ("go https://x.io/a_b now", vec!["go ", " now"]),
            ("see https://example.com/a.", vec!["see ", "."]),
            ("or <https://example.com/chek>", vec!["or "]),
            ("mail <me@example.com> now", vec!["mail ", " now"]),
            ("<kbd>Ctrll</kbd> stays", vec!["<kbd>Ctrll</kbd> stays"]),
            ("a <b c> d", vec!["a <b c> d"]),
        ] {
            assert_eq!(line_prose(line), prose, "{line:?}");
        }
        // An autolink is excluded without changing how the line looks.
        let p = dark_palette();
        let line = "<https://x.io>";
        assert_eq!(
            colored(line, &highlights_for(line)),
            [("https://x.io", Some(p.link))]
        );
    }

    #[test]
    fn matched_emphasis_and_strike_delimiters_are_excluded() {
        for (line, prose) in [
            ("_word_ and **two**", vec!["word", " and ", "two"]),
            ("~~gone~~ text", vec!["gone", " text"]),
            ("***both*** __b__", vec!["both", " ", "b"]),
            ("~~**x**~~", vec!["x"]),
            ("an *unmatched star", vec!["an *unmatched star"]),
        ] {
            assert_eq!(line_prose(line), prose, "{line:?}");
        }
    }

    #[test]
    fn a_delimiter_inside_a_word_stays_in_the_prose() {
        for (line, prose) in [
            (
                "snake_case_name and un*believ*able",
                vec!["snake_case_name and un*believ*able"],
            ),
            ("a~~b~~c", vec!["a~~b~~c"]),
            ("foo**bar** baz", vec!["foo**bar", " baz"]),
            ("**foo**bar", vec!["foo**bar"]),
            ("x2_y_ end", vec!["x2_y", " end"]),
            ("é*ü*ñ", vec!["é*ü*ñ"]),
        ] {
            assert_eq!(line_prose(line), prose, "{line:?}");
        }
    }

    #[test]
    fn the_acceptance_note_has_the_prose_the_plan_expects() {
        let note = fixtures::SPELL_NOTE;
        let none: Vec<&str> = Vec::new();
        assert_eq!(
            prose_of(note),
            [
                none.clone(),
                none.clone(),
                none.clone(),
                vec!["Spell chek plan"],
                none.clone(),
                vec!["Marks apear under prose as you type, and a"],
                vec!["right-click on one offers replacments."],
                none.clone(),
                vec!["Engine: ", " with a bundeled dictionary"],
                vec!["See ", "the desing note", " or "],
                vec!["README, README's, PRs and v0.1.1 stay unmarked; CommonMark does not"],
                vec!["Catppuccin and Catppuccin’s are in the word list; mail me@exmaple.com"],
                vec![
                    "and/or, #tagg, snake_case_nmae, un*believ*able and <kbd>Ctrll</kbd> are skipped"
                ],
                vec!["A done task with a mispelled word"],
                vec!["It doesn’t mark don't, well-known or a b c; it marks well-knwon"],
                none.clone(),
                vec!["A quote with a misteak."],
                none.clone(),
                none.clone(),
                none.clone(),
                none.clone(),
                none.clone(),
                none.clone(),
                none,
            ]
        );
    }

    #[test]
    fn prose_ranges_are_sorted_apart_non_empty_and_on_boundaries_for_any_text() {
        const PIECES: &[&str] = &[
            "word",
            "Wren",
            "é",
            "ü",
            "🐦",
            "说明",
            "\u{301}",
            "\u{feff}",
            " ",
            "  ",
            "\t",
            "*",
            "**",
            "_",
            "__",
            "~~",
            "~",
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
            "1. ",
            "```",
            "~~~",
            "    ",
            "---",
            "|",
            ":",
            "]:",
            "\\",
            "^",
            "a_b",
            "x:y",
        ];
        let mut rng = Rng(0x5eed_cafe_f00d_d00d);
        for doc in 0..3000 {
            let lines: Vec<String> = (0..1 + rng.below(10))
                .map(|_| {
                    (0..rng.below(14))
                        .map(|_| *rng.pick(PIECES))
                        .collect::<String>()
                })
                .collect();
            let text = lines.join("\n");
            for (i, (line, (_, decor))) in lines
                .iter()
                .zip(highlight_text(fixtures::DARK, &text))
                .enumerate()
            {
                let what = format!("doc {doc} line {i} {line:?}: {:?}", decor.prose);
                let mut previous: Option<&Range<usize>> = None;
                for range in &decor.prose {
                    assert!(range.start < range.end, "empty range in {what}");
                    assert!(range.end <= line.len(), "out of bounds in {what}");
                    assert!(
                        line.is_char_boundary(range.start) && line.is_char_boundary(range.end),
                        "splits a character in {what}"
                    );
                    assert!(
                        previous.is_none_or(|p| p.end < range.start),
                        "not sorted and apart in {what}"
                    );
                    assert!(
                        decor
                            .code_spans
                            .iter()
                            .all(|code| code.end <= range.start || range.end <= code.start),
                        "prose over code in {what}"
                    );
                    previous = Some(range);
                }
            }
        }
    }

    /// A small deterministic generator (xorshift64*), so failures replay.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
            &items[self.below(items.len())]
        }
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
            tasks: None,
        });

        let spans: Vec<_> = h.highlight_line("# Heading").collect();
        assert!(!spans.is_empty());
        assert_eq!(spans[0].1.color, Some(fixtures::LIGHT.faint));
        assert_eq!(spans[1].1.color, Some(fixtures::LIGHT.heading));
        let bold = spans[1].1.font.unwrap();
        assert_eq!(bold.family, font::Family::Name("Fira Code"));
        assert_eq!(bold.weight, font::Weight::Bold);
    }

    // ── The span layer ───────────────────────────────────────────────────────

    /// FNV-1a over the numbers fed to it.
    struct Digest(u64);

    impl Digest {
        fn new() -> Self {
            Self(0xcbf2_9ce4_8422_2325)
        }

        fn add(&mut self, value: u64) {
            for byte in value.to_le_bytes() {
                self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }

        fn ranges(&mut self, ranges: &[Range<usize>]) {
            self.add(ranges.len() as u64);
            for range in ranges {
                self.add(range.start as u64);
                self.add(range.end as u64);
            }
        }

        /// Everything a highlighted line gives the widget: each span's
        /// range, colour, face and scale, and the whole decoration.
        fn lines(&mut self, lines: &[HighlightedLine]) {
            self.add(lines.len() as u64);
            for (spans, decor) in lines {
                self.add(spans.len() as u64);
                for (range, highlight) in spans {
                    self.ranges(std::slice::from_ref(range));
                    match highlight.color {
                        Some(Color { r, g, b, a }) => {
                            for channel in [r, g, b, a] {
                                self.add(u64::from(channel.to_bits()));
                            }
                        }
                        None => self.add(u64::MAX),
                    }
                    self.add(highlight.font.map_or(0, |font| {
                        1 + u64::from(font.weight == font::Weight::Bold)
                            + 2 * u64::from(font.style == font::Style::Italic)
                    }));
                    self.add(u64::from(highlight.scale.text.to_bits()));
                    self.add(u64::from(highlight.scale.row.to_bits()));
                }
                self.add(u64::from(decor.code));
                self.ranges(&decor.strikes);
                self.ranges(&decor.code_spans);
                self.add(decor.hang as u64);
                self.ranges(&decor.prose);
            }
        }
    }

    /// Inline Markdown inside inline Markdown, on each kind of line.
    const NESTED: &str = "\
[a ~~struck~~ and **bold** `code` link](https://x.io/a_b_c) then ~~[a link](u) struck~~
**bold [link *both*](u) and `code` and <!-- a comment --> and https://x.io/_a_**
~~struck **bold _both_** https://x.io and `code`~~ ~~~~ **** ~~a *b~~ c*
[![alt _text_](pic_a_b.png)](https://x.io) ![image](a.png) [text][label] [a][b](c)
snake_case_name un*believ*able a_b_ _c_d ~~e~~f <me@x.io> <https://x.io/_a_> <kbd>
see https://x.io/a_(b)_c, (https://x.io/d). `` a`b `` ` unclosed <!-- unclosed
# A *heading* with `code` and [a link](u) #
> quoted **bold** [link](u) ~~struck~~ `code` https://x.io
> - [x] quoted done [link](u) `[code](u)` **bold** ~~struck~~
- [x] done [link *text*](u) and `[code](u)` https://x.io <!-- [c](d) -->
- [-] dropped **bold** [link](https://x.io) ✅ 2026-11-19 ❌ 2026-11-19
- [/] doing *@sam* [the **plan**](u) #tag 🆔 a1 ⏫ 📅 2026-11-18 ^id
- [ ] open ~~**@alice.chen**~~ `📅 2026-12-01` 🔺\u{fe0f} [due:: 2026-02-30]
1. [ ] numbered `code` 📅 2026-12-01 then *more*
[label]: https://x.io \"*Title*\"
[^note]: a *footnote* with `code`
- [label]: <https://x.io> **bold**
";

    /// The notes the harness and the scenes use, and [`NESTED`].
    const NOTES: [&str; 6] = [
        NESTED,
        fixtures::SPELL_NOTE,
        include_str!("../../../tools/wrentest/fixtures/spell.md"),
        include_str!(
            "../../../tools/wrentest/fixtures/tasks/workspace/Tasks/Baseline/Task lines.md"
        ),
        include_str!("../../../tools/wrentest/fixtures/tasks/workspace/Tasks/Emoji/Emoji spike.md"),
        include_str!("../../../tools/wrentest/fixtures/boards/workspace/Boards/Samples/Notes.md"),
    ];

    /// What a line of Markdown and a task line are made of, to draw
    /// generated lines from.
    const PIECES: &[&str] = &[
        "word",
        "Wren",
        "é",
        "🐦",
        "说明",
        "\u{301}",
        "\u{feff}",
        " ",
        " ",
        "  ",
        "\t",
        "*",
        "**",
        "***",
        "_",
        "__",
        "~~",
        "~",
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
        "## ",
        "- ",
        "> ",
        "- [x] ",
        "- [ ] ",
        "- [/] ",
        "- [-] ",
        "- [?] ",
        "1. ",
        "```",
        "~~~",
        "    ",
        "---",
        "|",
        ":",
        "]:",
        "\\",
        "^",
        "^id",
        "a_b",
        "x:y",
        "@sam",
        "@alice.chen",
        "#tag",
        "📅",
        "📅 ",
        "📆 ",
        "✅ ",
        "❌ ",
        "2026-12-01",
        "2026-02-30",
        "[due:: 2026-11-18]",
        "[k:: v]",
        "⏫",
        "🔺",
        "🔽\u{fe0f}",
        "\u{fe0f}",
        "🆔 a1",
        "🔁 every week",
        "⛔ a,b",
    ];

    /// Delimiters and little else, for lines whose inline Markdown nests
    /// and crosses.
    const DENSE: &[&str] = &[
        "*",
        "*",
        "**",
        "_",
        "__",
        "~~",
        "~",
        "`",
        "``",
        "[",
        "]",
        "(",
        ")",
        "!",
        "<",
        ">",
        " ",
        " ",
        "a",
        "b",
        "é",
        "1",
        "-",
        "<!--",
        "-->",
        "http://x",
        "https://y.z/",
        ":",
        "\\",
        ".",
        ",",
        "@",
        "#",
    ];

    /// What a dense line starts with.
    const STARTS: &[&str] = &[
        "", "", "", "- ", "> ", "- [x] ", "- [ ] ", "- [/] ", "- [-] ", "# ", "1. ", "  ",
        "> - [x] ", "---", "```", "[a]: ",
    ];

    /// The next document of the generated corpus: one to ten lines, each
    /// of [`PIECES`] or, every other document, a start and mostly
    /// [`DENSE`] pieces.
    fn generated(rng: &mut Rng) -> String {
        let dense = rng.below(2) == 0;
        (0..1 + rng.below(10))
            .map(|_| {
                let mut line = String::new();
                if dense {
                    line.push_str(rng.pick(STARTS));
                }
                for _ in 0..rng.below(if dense { 24 } else { 16 }) {
                    let from = if dense && rng.below(5) != 0 {
                        DENSE
                    } else {
                        PIECES
                    };
                    line.push_str(rng.pick(from));
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The digest of [`NOTES`] and of `generated` generated documents as a
    /// host with these `tasks` highlights them.
    fn corpus_digest(generated_documents: usize, tasks: Option<TaskOptions>) -> u64 {
        let mut digest = Digest::new();
        let mut rng = Rng(0x7a5c_5eed_0000_0021);
        let documents = NOTES
            .iter()
            .map(|note| (*note).to_owned())
            .chain((0..generated_documents).map(|_| generated(&mut rng)))
            .collect::<Vec<_>>();
        for document in &documents {
            digest.lines(&highlight_tasks(fixtures::DARK, document, tasks));
        }
        digest.0
    }

    /// A caller without the widget reads a line's spans and prose from a
    /// fresh scan ([`spans_at`]), and gets what the widget's highlighter
    /// holds for that line.
    #[test]
    fn a_fresh_scan_reads_the_spans_and_prose_the_highlighter_holds() {
        let mut rng = Rng(0x7a5c_5eed_0000_0022);
        let documents = NOTES
            .iter()
            .map(|note| (*note).to_owned())
            .chain((0..300).map(|_| generated(&mut rng)));
        for document in documents {
            let lines: Vec<&str> = document.split('\n').collect();
            let end = block::front_matter_end(&lines[..]);
            for tasks in [None, tasks_on()] {
                let mut h = MarkdownHighlighter::new(&Settings {
                    tasks,
                    ..settings(fixtures::DARK, end)
                });
                for (index, line) in lines.iter().enumerate() {
                    let _ = h.highlight_line(line);
                    let fresh = spans_at(&&lines[..], index, end, tasks);
                    assert_eq!(h.spans(index), Some(&fresh), "{line:?}");
                    let decor = h.decor(index).expect("a highlighted line");
                    assert_eq!(decor.prose, fresh.prose(), "{line:?}");
                    assert_eq!(decor.prose, prose_at(&&lines[..], index, end, tasks));
                }
                assert_eq!(
                    spans_at(&&lines[..], lines.len(), end, tasks),
                    span::LineSpans::default()
                );
                // Lines the text changed under are read again, spans too.
                h.change_line(0);
                assert_eq!(h.spans(0), None);
            }
        }
    }

    /// A finished task's line is read whole for its prose, reference
    /// definition or not, so a line built to make one pass slow must not
    /// be: here 100 000 comment openers that all reach one `-->` across a
    /// code span. Read opener by opener, it takes minutes.
    #[test]
    fn a_long_done_line_of_comment_openers_highlights_in_linear_time() {
        let line = format!("- [x] [r]: {}`x`-->", "<!--".repeat(100_000));
        for tasks in [None, tasks_on()] {
            let began = std::time::Instant::now();
            let (spans, decor) = highlight_tasks(fixtures::DARK, &line, tasks).remove(0);
            let took = began.elapsed();
            assert_eq!(decor.strikes, vec![(6..line.len())]);
            assert!(decor.prose.is_empty() && decor.code_spans.is_empty());
            assert_eq!(spans.len(), 2, "the marker with its box, and the text");
            assert!(took.as_secs() < 20, "took {took:?}");
        }
    }

    /// The highlighter reads the classifier's spans (`wren_markdown::span`)
    /// where it once matched inline Markdown itself, and that changed
    /// nothing a line shows: these digests were recorded from the
    /// highlighter as it was before, over the fixture notes and 10 000
    /// generated documents. With tasks off they hold for as long as that
    /// setting promises today's behaviour. The tasks-on digest was recorded
    /// again when tasks gained their colours.
    #[test]
    fn highlighting_is_what_it_was_before_the_span_layer() {
        assert_eq!(
            corpus_digest(10_000, None),
            0x6c5f_4634_ac2b_f122,
            "tasks off"
        );
        assert_eq!(
            corpus_digest(10_000, tasks_on()),
            0x5a63_ae28_5458_2dbe,
            "tasks on"
        );
    }
}
