//! The label editor (the plan's §5.9): the text of the one label that is
//! open, its caret and selection, and where each of them is on the board.
//!
//! The editor is the canvas's own, not a second widget (the plan's spike,
//! S8): the widget turns keys and the input method's events into
//! [`LabelAction`]s, the view applies them to its [`Label`], and every
//! change of text goes to the board at once, so the label on screen is
//! always the scene's own. What the editor adds is drawn over it: the
//! caret and the selection. Its lines are laid out by the scene's own wrap
//! ([`scene::wrap`]) in the label's own box, so a caret sits where the
//! scene draws its character and nothing moves when the edit ends.
//!
//! Everything here is in board units and takes the measurer as an
//! argument, so it runs with no window and no font.

use std::ops::Range;

use wren_board::scene::{self, Face, Measure, Point, Rect};
use wren_board::{shape, Id, Named, Node, NodeKind, Rgba, Shape, GRID};

/// One edit of the open label, stamped with the view's epoch in
/// [`crate::Message::Label`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelAction {
    /// Typed or committed text; a line break is `"\n"`.
    Insert(String),
    /// Move the caret, extending the selection when `select` is set.
    Move {
        motion: Motion,
        select: bool,
    },
    SelectAll,
    /// Erase the selection, or the character before the caret.
    Backspace,
    /// Erase the selection, or the character after the caret.
    Delete,
    /// Put the selected text on the clipboard.
    Copy,
    /// Put the selected text on the clipboard and erase it.
    Cut,
    /// Ask for the clipboard's text, which comes back as
    /// [`crate::Message::Pasted`] and replaces the selection.
    Paste,
    /// Put the label back as it was when this edit began (§5.9 rule 3).
    Undo,
    /// Restore the typing [`LabelAction::Undo`] took back.
    Redo,
    /// The input method's composition so far, and its selection as a byte
    /// range of `text`; empty text ends it.
    Preedit {
        text: String,
        selection: Option<Range<usize>>,
    },
}

/// Where a caret move goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    Start,
    End,
}

/// A measured width at most this much over its box still fits, as in the
/// scene's wrap.
const SLACK: f32 = 0.001;

/// No node is grown past this, the scene's own limit for a coordinate.
const MAX_SIDE: i32 = scene::COORDINATE_LIMIT as i32;

/// How a label is laid out, in board units (§3.7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    /// A shape's label box: wrapped at its width, each line centred across
    /// and the block centred down.
    Shape(Rect),
    /// Free text's box: wrapped at its width, at the top left.
    Free(Rect),
    /// A connector's label: broken at line feeds only, centred on a point.
    Connector(Point),
}

impl Place {
    /// Where the label of `node`, drawn in `bounds`, is laid out.
    pub(crate) fn of_node(node: &Node, bounds: Rect) -> Self {
        match node.kind.as_ref().and_then(Named::known) {
            Some(NodeKind::Shape) => {
                Self::Shape(shape::label_box(shape::drawn(&node.shape), bounds))
            }
            Some(NodeKind::Text) => Self::Free(bounds),
            // A placeholder's label is drawn as a rectangle's.
            None => Self::Shape(shape::label_box(Shape::Rect, bounds)),
        }
    }

    /// The font size the label is drawn at.
    pub(crate) fn text_size(self) -> f32 {
        match self {
            Self::Shape(_) | Self::Free(_) => scene::LABEL_SIZE,
            Self::Connector(_) => scene::SMALL_SIZE,
        }
    }

    /// The width its lines wrap at; a connector's label does not wrap.
    fn wrap(self) -> Option<f32> {
        match self {
            Self::Shape(area) | Self::Free(area) => Some(area.width),
            Self::Connector(_) => None,
        }
    }
}

/// One laid-out line: the bytes of the text it draws, where its first
/// character starts and how wide the scene measures it. A character
/// between this line's start and the next line's is on this line; the
/// white space a break dropped is after `range`.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    range: Range<usize>,
    x: f32,
    y: f32,
}

/// An input method's composition, shown at the caret until it commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preedit {
    pub text: String,
    /// The part of `text` the input method marks, in bytes.
    pub selection: Option<Range<usize>>,
}

/// The typing an undo took back.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Typing {
    text: String,
    caret: usize,
    anchor: usize,
}

/// What an action did, beyond moving the caret.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// The text changed.
    pub(crate) changed: bool,
    /// The text is again what it was when the edit began, by
    /// [`LabelAction::Undo`].
    pub(crate) restored: bool,
    /// Text to put on the clipboard.
    pub(crate) copied: Option<String>,
    /// The clipboard's text is wanted.
    pub(crate) paste: bool,
    /// An insert was refused: the label would pass
    /// [`scene::MAX_LABEL_BYTES`].
    pub(crate) full: bool,
}

/// A label as the board stored it when its edit began: a node's text and
/// size, or a connector's labels. Undo in the label, or typing back to
/// that text, puts it back as it was, trailing blank lines and auto-grow
/// included, so the label leaves no unit and no draft (§5.9 rules 3 and 4).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Before {
    Node { text: String, size: [i32; 2] },
    Edge(Vec<wren_board::Label>),
}

/// The open label: one element's text while it is edited.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    id: Id,
    text: String,
    /// The caret, and the other end of the selection: equal when nothing
    /// is selected. Both are byte offsets at grapheme boundaries.
    caret: usize,
    anchor: usize,
    /// The text when the edit began.
    began: String,
    undone: Option<Typing>,
    preedit: Option<Preedit>,
    /// Where a run of Up and Down keys is heading across.
    goal: Option<f32>,
    place: Place,
    rows: Vec<Row>,
    /// The element's label as the board stored it when the edit began,
    /// which a label that ends as it began puts back verbatim.
    pub(crate) before: Option<Before>,
    /// Whether this edit's own creation made the element, or for a
    /// connector its label (§5.9 rule 8).
    pub(crate) made: bool,
    /// The colour the label is drawn in, for the caret.
    pub(crate) ink: Rgba,
}

impl Label {
    /// The label of `id`, holding `text`, laid out in `place`. A label
    /// opens with all its text selected (§5.9 rule 1).
    pub(crate) fn open(id: Id, text: &str, place: Place, measure: &dyn Measure) -> Self {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut label = Self {
            id,
            caret: text.len(),
            anchor: 0,
            began: text.clone(),
            text,
            undone: None,
            preedit: None,
            goal: None,
            place,
            rows: Vec::new(),
            before: None,
            made: false,
            ink: Rgba::rgb(0, 0, 0),
        };
        label.lay(measure);
        label
    }

    /// The element whose label this is.
    pub fn id(&self) -> &Id {
        &self.id
    }

    /// The text as typed so far, trailing blank lines included.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The caret, as a byte offset into [`Label::text`].
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected bytes of [`Label::text`]; empty when the caret stands
    /// alone.
    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    /// The input method's composition, while there is one.
    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    /// Whether the text is other than when the edit began.
    pub(crate) fn changed(&self) -> bool {
        self.text != self.began
    }

    /// The text as the board stores it: without trailing blank lines
    /// (§5.9 rule 4), so a label of only white space is empty.
    pub(crate) fn stored(&self) -> &str {
        self.text.get(..drawn_len(&self.text)).unwrap_or_default()
    }

    /// The font size the label is drawn at, in board units.
    pub(crate) fn text_size(&self) -> f32 {
        self.place.text_size()
    }

    /// Lay the label out again in `place`: its node grew, or the board was
    /// built again.
    pub(crate) fn place(&mut self, place: Place, measure: &dyn Measure) {
        if place != self.place {
            self.place = place;
            self.lay(measure);
        }
    }

    /// Apply one action.
    pub(crate) fn apply(&mut self, action: LabelAction, measure: &dyn Measure) -> Outcome {
        let mut outcome = Outcome::default();
        let goal = self.goal.take();
        match action {
            LabelAction::Insert(text) => return self.insert(&text, measure),
            LabelAction::Move { motion, select } => {
                let selection = self.selection();
                let to = match motion {
                    // A plain step out of a selection goes to its edge.
                    Motion::Left if !select && !selection.is_empty() => selection.start,
                    Motion::Right if !select && !selection.is_empty() => selection.end,
                    Motion::Left => before(&self.text, self.caret),
                    Motion::Right => after(&self.text, self.caret),
                    Motion::WordLeft => word_before(&self.text, self.caret),
                    Motion::WordRight => word_after(&self.text, self.caret),
                    Motion::Up | Motion::Down => {
                        let goal = goal.unwrap_or_else(|| self.x_of(self.caret, measure));
                        let row = self.row_of(self.caret);
                        // Past the first and last lines are the ends of
                        // the text, and the column starts again there.
                        let to = match motion {
                            Motion::Up => row.checked_sub(1),
                            _ => Some(row + 1).filter(|next| *next < self.rows.len()),
                        };
                        match to {
                            Some(to) => {
                                self.goal = Some(goal);
                                self.offset_at(to, goal, measure)
                            }
                            None if motion == Motion::Up => 0,
                            None => self.text.len(),
                        }
                    }
                    Motion::LineStart => {
                        let row = self.row_of(self.caret);
                        self.rows.get(row).map_or(0, |row| row.range.start)
                    }
                    Motion::LineEnd => self.limit(self.row_of(self.caret)),
                    Motion::Start => 0,
                    Motion::End => self.text.len(),
                };
                self.caret = to;
                if !select {
                    self.anchor = to;
                }
            }
            LabelAction::SelectAll => {
                self.anchor = 0;
                self.caret = self.text.len();
            }
            LabelAction::Backspace | LabelAction::Delete => {
                let mut erased = self.selection();
                if erased.is_empty() && action == LabelAction::Backspace {
                    erased.start = before(&self.text, self.caret);
                } else if erased.is_empty() {
                    erased.end = after(&self.text, self.caret);
                }
                outcome.changed = self.replace(erased, "", measure);
            }
            LabelAction::Copy | LabelAction::Cut => {
                let selection = self.selection();
                let selected = self.text.get(selection.clone()).unwrap_or_default();
                if !selected.is_empty() {
                    outcome.copied = Some(selected.to_owned());
                    if action == LabelAction::Cut {
                        outcome.changed = self.replace(selection, "", measure);
                    }
                }
            }
            LabelAction::Paste => outcome.paste = true,
            LabelAction::Undo => {
                if self.changed() {
                    self.undone = Some(Typing {
                        text: std::mem::replace(&mut self.text, self.began.clone()),
                        caret: self.caret,
                        anchor: self.anchor,
                    });
                    self.caret = self.text.len();
                    self.anchor = self.caret;
                    self.lay(measure);
                    outcome.changed = true;
                    outcome.restored = true;
                }
            }
            LabelAction::Redo => {
                if let Some(typing) = self.undone.take() {
                    self.text = typing.text;
                    self.caret = typing.caret;
                    self.anchor = typing.anchor;
                    self.lay(measure);
                    outcome.changed = true;
                }
            }
            LabelAction::Preedit { text, selection } => {
                self.preedit = (!text.is_empty()).then_some(Preedit { text, selection });
            }
        }
        outcome
    }

    /// Replace the selection with `text`, as typed, committed by an input
    /// method or pasted: line endings become LF and control characters
    /// other than tab and LF are left out. A composition is over.
    pub(crate) fn insert(&mut self, text: &str, measure: &dyn Measure) -> Outcome {
        self.goal = None;
        self.preedit = None;
        let text: String = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .collect();
        let selection = self.selection();
        let grows = text.len() > selection.len();
        if grows && self.text.len() - selection.len() + text.len() > scene::MAX_LABEL_BYTES {
            return Outcome {
                full: true,
                ..Outcome::default()
            };
        }
        Outcome {
            changed: self.replace(selection, &text, measure),
            ..Outcome::default()
        }
    }

    /// Replace the selection with clipboard text: a board fragment's
    /// labels, one to a line (§5.9 rule 3), or the text itself.
    pub(crate) fn paste(&mut self, text: &str, measure: &dyn Measure) -> Outcome {
        let fragment = text
            .strip_prefix(wren_board::clip::HEADER)
            .and_then(|rest| {
                rest.strip_prefix("\r\n")
                    .or_else(|| rest.strip_prefix('\n'))
            })
            .and_then(|board| wren_board::parse(board).ok());
        match fragment {
            Some(board) => {
                let labels = wren_board::search_text(&board);
                self.insert(labels.trim_end_matches('\n'), measure)
            }
            None => self.insert(text, measure),
        }
    }

    /// Put the caret at the board point `at`, or with `select` take the
    /// selection there.
    pub(crate) fn point(&mut self, at: (f32, f32), select: bool, measure: &dyn Measure) {
        self.goal = None;
        let height = scene::line_height(self.text_size());
        let row = self
            .rows
            .iter()
            .rposition(|row| row.y <= at.1)
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
        // Below the last line is its end, as above the first is the start.
        let below = self.rows.last().is_some_and(|last| at.1 >= last.y + height);
        self.caret = if below {
            self.text.len()
        } else {
            self.offset_at(row, at.0, measure)
        };
        if !select {
            self.anchor = self.caret;
        }
    }

    /// The caret: a box of no width, one line tall.
    pub(crate) fn caret_box(&self, measure: &dyn Measure) -> Rect {
        let height = scene::line_height(self.text_size());
        let y = self
            .rows
            .get(self.row_of(self.caret))
            .map_or(0.0, |row| row.y);
        Rect::new(self.x_of(self.caret, measure), y, 0.0, height)
    }

    /// The selection, one box for each line it reaches.
    pub(crate) fn selected(&self, measure: &dyn Measure) -> Vec<Rect> {
        let selection = self.selection();
        if selection.is_empty() {
            return Vec::new();
        }
        let height = scene::line_height(self.text_size());
        let mut boxes = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            let next = self.rows.get(index + 1).map(|next| next.range.start);
            let owned = row.range.start..next.unwrap_or(self.text.len() + 1);
            if selection.start >= owned.end || selection.end <= owned.start {
                continue;
            }
            let limit = self.limit(index);
            let x = |offset: usize| {
                let to = offset.clamp(row.range.start, limit);
                let before = self.text.get(row.range.start..to).unwrap_or_default();
                row.x + measure.width(before, Face::Regular, self.text_size())
            };
            // A selected line feed shows as a little more.
            let fed = self.text.as_bytes().get(limit) == Some(&b'\n');
            let past = if fed && selection.end > limit {
                self.text_size() * 0.3
            } else {
                0.0
            };
            let left = x(selection.start);
            let width = x(selection.end) - left + past;
            if width > 0.0 {
                boxes.push(Rect::new(left, row.y, width, height));
            }
        }
        boxes
    }

    /// The box around every line, one line tall where there is no text.
    pub(crate) fn bounds(&self, measure: &dyn Measure) -> Rect {
        let size = self.text_size();
        let height = scene::line_height(size);
        let boxes = self.rows.iter().map(|row| {
            let line = self.text.get(row.range.clone()).unwrap_or_default();
            Rect::new(
                row.x,
                row.y,
                measure.width(line, Face::Regular, size),
                height,
            )
        });
        boxes.reduce(crate::union).unwrap_or_default()
    }

    /// Each line that holds text, with where it starts: what the scene
    /// draws of the same label.
    #[cfg(test)]
    pub(crate) fn lines(&self) -> Vec<(&str, f32, f32)> {
        let lines = self.rows.iter().filter(|row| !row.range.is_empty());
        lines
            .map(|row| (&self.text[row.range.clone()], row.x, row.y))
            .collect()
    }

    /// Replace `range` of the text with `with` and put the caret after
    /// it. `false` when that changes nothing.
    fn replace(&mut self, range: Range<usize>, with: &str, measure: &dyn Measure) -> bool {
        if (range.is_empty() && with.is_empty()) || self.text.get(range.clone()).is_none() {
            self.caret = self.caret.min(self.text.len());
            self.anchor = self.caret;
            return false;
        }
        self.caret = range.start + with.len();
        self.anchor = self.caret;
        self.text.replace_range(range, with);
        self.undone = None;
        self.lay(measure);
        true
    }

    /// Lay the text out as the scene lays the label out (§3.7), with the
    /// lines of trailing blank text, which the scene does not hold, below
    /// the block the scene centres.
    fn lay(&mut self, measure: &dyn Measure) {
        let size = self.place.text_size();
        let height = scene::line_height(size);
        let wrapped = scene::wrap(&self.text, self.place.wrap(), Face::Regular, size, measure);
        let drawn = drawn_len(&self.text);
        let block = wrapped
            .iter()
            .filter(|line| line.range.start <= drawn)
            .count()
            .max(1) as f32
            * height;
        let top = match self.place {
            Place::Shape(area) => area.y + (area.height - block) / 2.0,
            Place::Free(area) => area.y,
            Place::Connector(centre) => centre.y - block / 2.0,
        };
        self.rows = wrapped
            .into_iter()
            .enumerate()
            .map(|(index, line)| Row {
                x: match self.place {
                    Place::Shape(area) => area.x + (area.width - line.width) / 2.0,
                    Place::Free(area) => area.x,
                    Place::Connector(centre) => centre.x - line.width / 2.0,
                },
                y: top + index as f32 * height,
                range: line.range,
            })
            .collect();
    }

    /// The line the character at `offset` is on: the last one that starts
    /// at or before it.
    fn row_of(&self, offset: usize) -> usize {
        self.rows
            .iter()
            .rposition(|row| row.range.start <= offset)
            .unwrap_or(0)
    }

    /// The last caret place on line `row`: before the line feed that ends
    /// it, at the end of the text, or where a wrapped line's text ends.
    fn limit(&self, row: usize) -> usize {
        let Some(next) = self.rows.get(row + 1) else {
            return self.text.len();
        };
        let fed = next.range.start.checked_sub(1);
        match fed {
            Some(feed) if self.text.as_bytes().get(feed) == Some(&b'\n') => feed,
            _ => self.rows.get(row).map_or(0, |row| row.range.end),
        }
    }

    /// Where the caret at `offset` is drawn, across.
    fn x_of(&self, offset: usize, measure: &dyn Measure) -> f32 {
        let Some(row) = self.rows.get(self.row_of(offset)) else {
            return 0.0;
        };
        let before = self.text.get(row.range.start..offset).unwrap_or_default();
        row.x + measure.width(before, Face::Regular, self.text_size())
    }

    /// The caret place on line `row` nearest `x`. The widths of a line's
    /// prefixes only grow, so the place is found by halving.
    fn offset_at(&self, row: usize, x: f32, measure: &dyn Measure) -> usize {
        let Some(line) = self.rows.get(row) else {
            return self.text.len();
        };
        let start = line.range.start;
        let rest = self.text.get(start..self.limit(row)).unwrap_or_default();
        let mut stops = vec![start];
        stops.extend(scene::graphemes(rest).scan(start, |end, grapheme| {
            *end += grapheme.len();
            Some(*end)
        }));
        let at = |stop: usize| {
            let before = self.text.get(start..stop).unwrap_or_default();
            line.x + measure.width(before, Face::Regular, self.text_size())
        };
        // The first stop at or past `x`, then the nearer of it and the
        // one before.
        let past = stops.partition_point(|stop| at(*stop) < x);
        match (past.checked_sub(1), stops.get(past)) {
            (Some(left), Some(&right)) => {
                let left = stops[left];
                if x - at(left) <= at(right) - x {
                    left
                } else {
                    right
                }
            }
            (None, Some(&right)) => right,
            (_, None) => stops.last().copied().unwrap_or(start),
        }
    }
}

/// How much of `text` the board stores and the scene draws: all but its
/// trailing blank lines.
fn drawn_len(text: &str) -> usize {
    let mut end = text.len();
    loop {
        let start = text[..end].rfind('\n').map_or(0, |feed| feed + 1);
        if !text[start..end].trim().is_empty() {
            return end;
        }
        if start == 0 {
            return 0;
        }
        end = start - 1;
    }
}

/// The grapheme boundary before `offset`.
fn before(text: &str, offset: usize) -> usize {
    let head = text.get(..offset).unwrap_or(text);
    scene::graphemes(head)
        .next_back()
        .map_or(0, |last| head.len() - last.len())
}

/// The grapheme boundary after `offset`.
fn after(text: &str, offset: usize) -> usize {
    let tail = text.get(offset..).unwrap_or_default();
    scene::graphemes(tail)
        .next()
        .map_or(text.len(), |first| offset + first.len())
}

/// What a word motion takes a grapheme for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Other,
}

fn class(grapheme: &str) -> Class {
    match grapheme.chars().next() {
        Some(c) if c.is_whitespace() => Class::Space,
        Some(c) if c.is_alphanumeric() || c == '_' => Class::Word,
        _ => Class::Other,
    }
}

/// The start of the word before `offset`: back over white space, then over
/// one run of letters and digits, or of anything else.
fn word_before(text: &str, offset: usize) -> usize {
    let head = text.get(..offset).unwrap_or(text);
    let mut at = head.len();
    let mut run = None;
    for grapheme in scene::graphemes(head).rev() {
        let class = class(grapheme);
        match run {
            None if class == Class::Space => {}
            None => run = Some(class),
            Some(run) if run != class => break,
            Some(_) => {}
        }
        at -= grapheme.len();
    }
    at
}

/// The end of the word after `offset`: over white space, then over one run.
fn word_after(text: &str, offset: usize) -> usize {
    let tail = text.get(offset..).unwrap_or_default();
    let mut at = offset.min(text.len());
    let mut run = None;
    for grapheme in scene::graphemes(tail) {
        let class = class(grapheme);
        match run {
            None if class == Class::Space => {}
            None => run = Some(class),
            Some(run) if run != class => break,
            Some(_) => {}
        }
        at += grapheme.len();
    }
    at
}

/// The size `node` needs to hold `text` in its label box, never less than
/// `size` (§5.9 rule 7): taller, in whole cells, while the wrapped text is
/// taller than the box, and wider only for a word wider than the box
/// alone. A blank label draws nothing and grows nothing.
pub(crate) fn grown(node: &Node, size: [i32; 2], text: &str, measure: &dyn Measure) -> [i32; 2] {
    if text.trim().is_empty() {
        return size;
    }
    let area = |[width, height]: [i32; 2]| {
        let bounds = Rect::new(0.0, 0.0, width.max(1) as f32, height.max(1) as f32);
        match Place::of_node(node, bounds) {
            Place::Shape(area) | Place::Free(area) => area,
            Place::Connector(_) => Rect::default(),
        }
    };
    let font = scene::LABEL_SIZE;
    let widest = scene::words(text)
        .map(|word| measure.width(word.trim_end(), Face::Regular, font))
        .fold(0.0, f32::max);
    let [mut width, mut height] = size;
    // A taller pill or parallelogram has a narrower box, so widening may
    // have to follow a growth in height; each round grows something.
    for _ in 0..16 {
        let before = [width, height];
        while widest > area([width, height]).width + SLACK && width < MAX_SIDE {
            width = width.saturating_add(GRID).min(MAX_SIDE);
        }
        let wrap = Some(area([width, height]).width);
        let lines = scene::wrap(text, wrap, Face::Regular, font, measure).len();
        let needed = lines as f32 * scene::line_height(font);
        while needed > area([width, height]).height + SLACK && height < MAX_SIDE {
            height = height.saturating_add(GRID).min(MAX_SIDE);
        }
        if [width, height] == before {
            break;
        }
    }
    [width, height]
}

#[cfg(test)]
mod tests {
    use wren_board::scene::FixedMeasure;

    use super::*;

    /// Under the fixed measurer a grapheme of a 14 px label is 7 wide and
    /// a line 20 tall.
    const M: &FixedMeasure = &FixedMeasure;

    fn id() -> Id {
        Id::new("a").expect("an id")
    }

    /// Free text in a box 70 wide at (100, 50): ten graphemes to a line.
    fn free(text: &str) -> Label {
        let place = Place::Free(Rect::new(100.0, 50.0, 70.0, 20.0));
        Label::open(id(), text, place, M)
    }

    fn moved(label: &mut Label, motion: Motion) -> usize {
        label.apply(
            LabelAction::Move {
                motion,
                select: false,
            },
            M,
        );
        label.caret()
    }

    fn extended(label: &mut Label, motion: Motion) -> Range<usize> {
        label.apply(
            LabelAction::Move {
                motion,
                select: true,
            },
            M,
        );
        label.selection()
    }

    fn typed(label: &mut Label, text: &str) -> Outcome {
        label.apply(LabelAction::Insert(text.to_owned()), M)
    }

    #[test]
    fn a_label_opens_with_all_its_text_selected() {
        let label = free("Start");
        assert_eq!(label.text(), "Start");
        assert_eq!(label.selection(), 0..5);
        assert_eq!(label.caret(), 5);
        assert!(!label.changed());
        let empty = free("");
        assert_eq!(empty.selection(), 0..0);
        // A CR never reaches the text.
        assert_eq!(free("a\r\nb\rc").text(), "a\nb\nc");
    }

    #[test]
    fn typing_replaces_the_selection_and_moves_the_caret() {
        let mut label = free("Start");
        assert!(typed(&mut label, "E").changed);
        assert_eq!((label.text(), label.caret()), ("E", 1));
        typed(&mut label, "nd");
        assert_eq!((label.text(), label.selection()), ("End", 3..3));
        moved(&mut label, Motion::Left);
        typed(&mut label, "\n");
        assert_eq!((label.text(), label.caret()), ("En\nd", 3));
        // Line endings become LF, and other control characters are left
        // out; a tab is kept.
        typed(&mut label, "a\r\nb\rc\u{7}\u{1b}\td");
        assert_eq!(label.text(), "En\na\nb\nc\tdd");
        // Nothing typed is no change.
        assert_eq!(typed(&mut label, ""), Outcome::default());
        assert_eq!(typed(&mut label, "\u{8}"), Outcome::default());
    }

    #[test]
    fn backspace_and_delete_erase_a_grapheme_or_the_selection() {
        let mut label = free("ae\u{301}👨‍👩‍👧b");
        moved(&mut label, Motion::End);
        moved(&mut label, Motion::Left);
        assert!(label.apply(LabelAction::Backspace, M).changed);
        assert_eq!(label.text(), "ae\u{301}b");
        assert!(label.apply(LabelAction::Backspace, M).changed);
        assert_eq!((label.text(), label.caret()), ("ab", 1));
        assert!(label.apply(LabelAction::Delete, M).changed);
        assert_eq!((label.text(), label.caret()), ("a", 1));
        // At the ends there is nothing to erase.
        assert!(!label.apply(LabelAction::Delete, M).changed);
        moved(&mut label, Motion::Start);
        assert!(!label.apply(LabelAction::Backspace, M).changed);
        assert_eq!(label.text(), "a");
        label.apply(LabelAction::SelectAll, M);
        assert!(label.apply(LabelAction::Delete, M).changed);
        assert_eq!((label.text(), label.selection()), ("", 0..0));
    }

    #[test]
    fn the_caret_steps_over_graphemes_and_out_of_a_selection() {
        let mut label = free("ae\u{301}👨‍👩‍👧b");
        let end = label.text().len();
        // A plain step out of a selection goes to its edge.
        assert_eq!(moved(&mut label, Motion::Left), 0);
        label.apply(LabelAction::SelectAll, M);
        assert_eq!(moved(&mut label, Motion::Right), end);
        assert_eq!(moved(&mut label, Motion::Right), end);
        assert_eq!(moved(&mut label, Motion::Left), end - 1);
        assert_eq!(moved(&mut label, Motion::Left), 4);
        assert_eq!(moved(&mut label, Motion::Left), 1);
        assert_eq!(moved(&mut label, Motion::Left), 0);
        assert_eq!(moved(&mut label, Motion::Left), 0);
        assert_eq!(moved(&mut label, Motion::Right), 1);
        assert_eq!(moved(&mut label, Motion::Right), 4);
        // Extending keeps the other end.
        assert_eq!(extended(&mut label, Motion::Left), 1..4);
        assert_eq!(extended(&mut label, Motion::Start), 0..4);
        assert_eq!(extended(&mut label, Motion::End), 4..end);
        assert_eq!(label.caret(), end);
    }

    #[test]
    fn word_motions_stop_at_the_edges_of_words() {
        let text = "one  two_2, é!";
        let mut label = free(text);
        moved(&mut label, Motion::Start);
        let stops: Vec<usize> = (0..6)
            .map(|_| moved(&mut label, Motion::WordRight))
            .collect();
        // After "one", "two_2", ",", "é", "!" and then the end.
        assert_eq!(stops, [3, 10, 11, 14, 15, 15]);
        let stops: Vec<usize> = (0..6)
            .map(|_| moved(&mut label, Motion::WordLeft))
            .collect();
        assert_eq!(stops, [14, 12, 10, 5, 0, 0]);
        assert_eq!(text.len(), 15);
    }

    #[test]
    fn lines_are_the_scene_s_and_the_caret_follows_them() {
        // Ten graphemes to a line: "alpha beta" fills the first.
        let mut label = free("alpha beta gamma\nx");
        let rows: Vec<(Range<usize>, f32, f32)> = label
            .rows
            .iter()
            .map(|row| (row.range.clone(), row.x, row.y))
            .collect();
        assert_eq!(
            rows,
            [
                (0..10, 100.0, 50.0),
                (11..16, 100.0, 70.0),
                (17..18, 100.0, 90.0),
            ]
        );
        let caret = |label: &Label| {
            let caret = label.caret_box(M);
            (caret.x, caret.y, caret.width, caret.height)
        };
        moved(&mut label, Motion::Start);
        assert_eq!(caret(&label), (100.0, 50.0, 0.0, 20.0));
        // The space a wrap dropped is still on the line it ended.
        label.caret = 10;
        assert_eq!(caret(&label), (170.0, 50.0, 0.0, 20.0));
        label.caret = 11;
        assert_eq!(caret(&label), (100.0, 70.0, 0.0, 20.0));
        // Before a line feed is the end of its line; after it, the next.
        label.caret = 16;
        assert_eq!(caret(&label), (135.0, 70.0, 0.0, 20.0));
        label.caret = 17;
        assert_eq!(caret(&label), (100.0, 90.0, 0.0, 20.0));
        label.anchor = 17;
        assert_eq!(moved(&mut label, Motion::LineEnd), 18);
        assert_eq!(moved(&mut label, Motion::LineStart), 17);
        label.caret = 13;
        label.anchor = 13;
        assert_eq!(moved(&mut label, Motion::LineStart), 11);
        assert_eq!(moved(&mut label, Motion::LineEnd), 16);
        label.caret = 3;
        label.anchor = 3;
        // A wrapped line ends where its text does.
        assert_eq!(moved(&mut label, Motion::LineEnd), 10);
        assert_eq!(moved(&mut label, Motion::LineStart), 0);
    }

    #[test]
    fn white_space_before_a_line_feed_is_on_its_line() {
        let mut label = free("abc  \nx");
        assert_eq!(label.rows[0].range, 0..3);
        label.point((100.0, 55.0), false, M);
        // The end of the line is before its line feed, past the spaces
        // the scene's line leaves out, and the caret is drawn after them.
        assert_eq!(moved(&mut label, Motion::LineEnd), 5);
        assert_eq!(label.caret_box(M).x, 135.0);
        label.point((900.0, 55.0), false, M);
        assert_eq!(label.caret(), 5);
        label.point((125.0, 55.0), false, M);
        assert_eq!(label.caret(), 4);
        // Selected, they show.
        label.apply(LabelAction::SelectAll, M);
        assert_eq!(
            label.selected(M)[0],
            Rect::new(100.0, 50.0, 35.0 + 14.0 * 0.3, 20.0)
        );
    }

    #[test]
    fn up_and_down_keep_their_column_through_a_short_line() {
        let mut label = free("abcdefgh\nij\nklmnopqr");
        label.caret = 6;
        label.anchor = 6;
        assert_eq!(moved(&mut label, Motion::Down), 11);
        // The column is remembered past the short line.
        assert_eq!(moved(&mut label, Motion::Down), 18);
        assert_eq!(moved(&mut label, Motion::Up), 11);
        assert_eq!(moved(&mut label, Motion::Up), 6);
        // Past the first and last lines are the start and the end.
        assert_eq!(moved(&mut label, Motion::Up), 0);
        assert_eq!(moved(&mut label, Motion::Down), 9);
        // Any other move forgets the column.
        label.point((142.0, 55.0), false, M);
        assert_eq!(moved(&mut label, Motion::Down), 11);
        moved(&mut label, Motion::Left);
        assert_eq!(moved(&mut label, Motion::Down), 13);
        assert_eq!(extended(&mut label, Motion::Down), 13..20);
    }

    #[test]
    fn a_point_puts_the_caret_at_the_nearest_place() {
        let mut label = free("alpha beta gamma\nx");
        let mut at = |x: f32, y: f32| {
            label.point((x, y), false, M);
            label.caret()
        };
        assert_eq!(at(100.0, 55.0), 0);
        assert_eq!(at(103.0, 55.0), 0);
        assert_eq!(at(104.0, 55.0), 1);
        assert_eq!(at(121.0, 69.9), 3);
        // Past a wrapped line's text is its end, and left of it the start.
        assert_eq!(at(900.0, 55.0), 10);
        assert_eq!(at(-900.0, 75.0), 11);
        assert_eq!(at(900.0, 75.0), 16);
        // Above the first line is on it; below the last is the end.
        assert_eq!(at(107.0, -40.0), 1);
        assert_eq!(at(100.0, 95.0), 17);
        assert_eq!(at(100.0, 400.0), 18);
        // Extending keeps the other end.
        label.point((100.0, 55.0), false, M);
        label.point((121.0, 75.0), true, M);
        assert_eq!(label.selection(), 0..14);
        assert_eq!(label.caret(), 14);
    }

    #[test]
    fn a_shape_s_lines_are_centred_and_trailing_blank_lines_hang_below() {
        // A label box 70 wide and 60 tall at (10, 10).
        let place = Place::Shape(Rect::new(10.0, 10.0, 70.0, 60.0));
        let mut label = Label::open(id(), "abcd\nef", place, M);
        let rows = |label: &Label| -> Vec<(f32, f32)> {
            label.rows.iter().map(|row| (row.x, row.y)).collect()
        };
        // Two lines of 28 and 14: centred across, and as a block down.
        assert_eq!(rows(&label), [(31.0, 20.0), (38.0, 40.0)]);
        // A trailing line feed is not stored, so the block the scene
        // centres stays where it is and the caret goes below it.
        moved(&mut label, Motion::End);
        typed(&mut label, "\n");
        assert_eq!(label.stored(), "abcd\nef");
        assert_eq!(rows(&label), [(31.0, 20.0), (38.0, 40.0), (45.0, 60.0)]);
        let caret = label.caret_box(M);
        assert_eq!((caret.x, caret.y), (45.0, 60.0));
        // Typed on, the line is the label's and the block is three lines.
        typed(&mut label, "g");
        assert_eq!(label.stored(), "abcd\nef\ng");
        assert_eq!(rows(&label), [(31.0, 10.0), (38.0, 30.0), (41.5, 50.0)]);
        // An empty label has one line, in the middle.
        let empty = Label::open(id(), "", place, M);
        assert_eq!(rows(&empty), [(45.0, 30.0)]);
        let blank = Label::open(id(), " \n\n", place, M);
        assert_eq!(blank.stored(), "");
        assert_eq!(rows(&blank), [(45.0, 30.0), (45.0, 50.0), (45.0, 70.0)]);
    }

    #[test]
    fn a_connector_s_label_is_centred_on_its_point_and_never_wraps() {
        let place = Place::Connector(Point::new(200.0, 100.0));
        let label = Label::open(id(), "a long label that goes on\nYes", place, M);
        // 12 px text: a grapheme is 6 wide and a line 16 tall.
        let rows: Vec<(Range<usize>, f32, f32)> = label
            .rows
            .iter()
            .map(|row| (row.range.clone(), row.x, row.y))
            .collect();
        assert_eq!(rows, [(0..25, 125.0, 84.0), (26..29, 191.0, 100.0)]);
        assert_eq!(label.text_size(), scene::SMALL_SIZE);
        assert_eq!(label.bounds(M), Rect::new(125.0, 84.0, 150.0, 32.0));
    }

    #[test]
    fn the_selection_is_a_box_for_each_line_it_reaches() {
        let mut label = free("alpha beta gamma\nx");
        assert_eq!(
            label.selected(M),
            [
                // The first line with the break after it.
                Rect::new(100.0, 50.0, 70.0, 20.0),
                Rect::new(100.0, 70.0, 35.0 + 14.0 * 0.3, 20.0),
                Rect::new(100.0, 90.0, 7.0, 20.0),
            ]
        );
        label.anchor = 2;
        label.caret = 4;
        assert_eq!(label.selected(M), [Rect::new(114.0, 50.0, 14.0, 20.0)]);
        label.anchor = 14;
        label.caret = 17;
        // To the end of the second line, and the line feed.
        assert_eq!(
            label.selected(M),
            [Rect::new(121.0, 70.0, 14.0 + 14.0 * 0.3, 20.0)]
        );
        label.anchor = 17;
        assert!(label.selected(M).is_empty());
    }

    #[test]
    fn copy_and_cut_give_the_selected_text() {
        let mut label = free("alpha beta");
        label.anchor = 6;
        label.caret = 10;
        let copied = label.apply(LabelAction::Copy, M);
        assert_eq!(copied.copied.as_deref(), Some("beta"));
        assert!(!copied.changed);
        let cut = label.apply(LabelAction::Cut, M);
        assert_eq!(cut.copied.as_deref(), Some("beta"));
        assert!(cut.changed);
        assert_eq!((label.text(), label.caret()), ("alpha ", 6));
        // With nothing selected the clipboard is left alone.
        assert_eq!(label.apply(LabelAction::Copy, M), Outcome::default());
        assert_eq!(label.apply(LabelAction::Cut, M), Outcome::default());
        assert!(label.apply(LabelAction::Paste, M).paste);
    }

    #[test]
    fn a_paste_is_text_or_a_fragment_s_labels() {
        let mut label = free("x");
        label.paste("plain\r\ntext", M);
        assert_eq!(label.text(), "plain\ntext");
        label.apply(LabelAction::SelectAll, M);
        let fragment = "# wren-board fragment 1\nschema = 1\n\n\
                        [node.b]\ntext = \"Below\"\nkind = \"shape\"\nat = [0, 100]\nsize = [120, 60]\n\n\
                        [node.a]\ntext = \"Above\"\nkind = \"shape\"\nat = [0, 0]\nsize = [120, 60]\n";
        label.paste(fragment, M);
        assert_eq!(label.text(), "Above\nBelow");
        // What only looks like a fragment is the text it is.
        label.apply(LabelAction::SelectAll, M);
        label.paste("# wren-board fragment 1\nnot = [a board", M);
        assert_eq!(label.text(), "# wren-board fragment 1\nnot = [a board");
        label.apply(LabelAction::SelectAll, M);
        label.paste("# wren-board fragment 12", M);
        assert_eq!(label.text(), "# wren-board fragment 12");
    }

    #[test]
    fn undo_puts_the_label_back_and_redo_restores_the_typing() {
        let mut label = free("Start");
        assert_eq!(label.apply(LabelAction::Undo, M), Outcome::default());
        assert_eq!(label.apply(LabelAction::Redo, M), Outcome::default());
        moved(&mut label, Motion::End);
        typed(&mut label, " here");
        moved(&mut label, Motion::Left);
        let undone = label.apply(LabelAction::Undo, M);
        assert!(undone.changed && undone.restored);
        assert_eq!((label.text(), label.selection()), ("Start", 5..5));
        assert!(!label.changed());
        // Again is nothing more.
        assert_eq!(label.apply(LabelAction::Undo, M), Outcome::default());
        let redone = label.apply(LabelAction::Redo, M);
        assert!(redone.changed && !redone.restored);
        assert_eq!((label.text(), label.caret()), ("Start here", 9));
        assert_eq!(label.apply(LabelAction::Redo, M), Outcome::default());
        // Typing after an undo leaves nothing to restore.
        label.apply(LabelAction::Undo, M);
        typed(&mut label, "!");
        assert_eq!(label.apply(LabelAction::Redo, M), Outcome::default());
        assert_eq!(label.text(), "Start!");
    }

    #[test]
    fn a_composition_is_beside_the_text_until_it_commits() {
        let mut label = free("ab");
        moved(&mut label, Motion::End);
        let composing = label.apply(
            LabelAction::Preedit {
                text: "にほ".to_owned(),
                selection: Some(0..6),
            },
            M,
        );
        assert_eq!(composing, Outcome::default());
        assert_eq!(label.text(), "ab");
        assert_eq!(
            label.preedit(),
            Some(&Preedit {
                text: "にほ".to_owned(),
                selection: Some(0..6)
            })
        );
        // The commit is an insert, and ends it.
        assert!(typed(&mut label, "日本").changed);
        assert_eq!((label.text(), label.caret()), ("ab日本", 8));
        assert_eq!(label.preedit(), None);
        // Empty text ends a composition with nothing committed.
        let preedit = |text: &str| LabelAction::Preedit {
            text: text.to_owned(),
            selection: None,
        };
        label.apply(preedit("x"), M);
        label.apply(preedit(""), M);
        assert_eq!((label.text(), label.preedit()), ("ab日本", None));
    }

    #[test]
    fn a_label_holds_no_more_than_the_scene_draws() {
        let cap = scene::MAX_LABEL_BYTES;
        let mut label = free(&"x".repeat(cap - 1));
        moved(&mut label, Motion::End);
        assert!(typed(&mut label, "y").changed);
        let refused = typed(&mut label, "z");
        assert_eq!(
            refused,
            Outcome {
                full: true,
                ..Outcome::default()
            }
        );
        assert_eq!(label.text().len(), cap);
        // A replacement that does not grow it is taken, and so is erasing.
        extended(&mut label, Motion::Left);
        assert!(typed(&mut label, "z").changed);
        assert!(label.apply(LabelAction::Backspace, M).changed);
        // A label a file made longer than the cap can still be cut down.
        let mut long = free(&"x".repeat(cap + 10));
        moved(&mut long, Motion::End);
        assert!(typed(&mut long, "y").full);
        assert!(long.apply(LabelAction::Backspace, M).changed);
    }

    #[test]
    fn stored_text_drops_trailing_blank_lines_only() {
        for (typed, stored) in [
            ("", ""),
            ("a", "a"),
            ("a\n", "a"),
            ("a \n \n\t\n", "a "),
            ("\n\na", "\n\na"),
            ("a\n\nb\n\n", "a\n\nb"),
            (" \n ", ""),
        ] {
            assert_eq!(free(typed).stored(), stored, "{typed:?}");
        }
    }

    fn node(kind: NodeKind, shape: Shape, size: [i32; 2]) -> Node {
        let text = format!(
            "schema = 1\n\n[node.n]\nkind = \"{}\"\nat = [0, 0]\nshape = \"{}\"\nsize = [{}, {}]\n",
            match kind {
                NodeKind::Shape => "shape",
                NodeKind::Text => "text",
            },
            match shape {
                Shape::Diamond => "diamond",
                Shape::Pill => "pill",
                _ => "rect",
            },
            size[0],
            size[1]
        );
        let board = wren_board::parse(&text).expect("a board");
        board.nodes.values().next().expect("a node").clone()
    }

    #[test]
    fn a_node_grows_taller_in_whole_cells_for_its_wrapped_text() {
        // A rectangle's box is 104 by 52: fourteen graphemes to a line,
        // and two lines.
        let rect = node(NodeKind::Shape, Shape::Rect, [120, 60]);
        let grown = |text: &str| grown(&rect, [120, 60], text, M);
        assert_eq!(grown("fits"), [120, 60]);
        assert_eq!(grown("one two three four five"), [120, 60]);
        // Three lines need 60, so one more cell; four need 80, so two.
        assert_eq!(grown("one two three four five six seven"), [120, 80]);
        assert_eq!(grown("a\nb\nc\nd"), [120, 100]);
        // Blank text draws nothing and grows nothing.
        assert_eq!(grown(" \n\n\n\n\n"), [120, 60]);
        // Nothing shrinks: a size already larger is kept.
        assert_eq!(super::grown(&rect, [200, 200], "a", M), [200, 200]);
        // An off-grid size grows by whole cells from where it is.
        assert_eq!(super::grown(&rect, [120, 65], "a\nb\nc\nd", M), [120, 105]);
    }

    #[test]
    fn a_node_grows_wider_only_for_a_word_wider_than_its_box() {
        let rect = node(NodeKind::Shape, Shape::Rect, [120, 60]);
        // Fourteen graphemes fit the 104 of the box; fifteen do not, and
        // the box of a node 140 wide holds them.
        assert_eq!(grown(&rect, [120, 60], "abcdefghijklmn", M), [120, 60]);
        assert_eq!(grown(&rect, [120, 60], "abcdefghijklmno", M), [140, 60]);
        // Words that wrap never widen it.
        assert_eq!(
            grown(&rect, [120, 60], "abcdefghijklmn abcdefghijklmn", M),
            [120, 60]
        );
        // Free text has no inset: a new text node is 120 by 20.
        let text = node(NodeKind::Text, Shape::Rect, [120, 20]);
        assert_eq!(grown(&text, [120, 20], "seventeen letters", M), [120, 20]);
        assert_eq!(grown(&text, [120, 20], "seventeen letters!", M), [120, 40]);
        assert_eq!(grown(&text, [120, 20], "eighteen_graphemes", M), [140, 20]);
        // A diamond's box is two thirds of the node each way.
        let diamond = node(NodeKind::Shape, Shape::Diamond, [120, 60]);
        assert_eq!(grown(&diamond, [120, 60], "ab\ncd", M), [120, 60]);
        assert_eq!(grown(&diamond, [120, 60], "ab\ncd\nef", M), [120, 100]);
        assert_eq!(grown(&diamond, [120, 60], "abcdefghijkl", M), [140, 60]);
        // A taller pill has a narrower box, so it may widen as it grows.
        let pill = node(NodeKind::Shape, Shape::Pill, [120, 60]);
        let size = grown(&pill, [120, 60], "abcdefghijklm\nb\nc\nd\ne", M);
        let area = shape::label_box(
            Shape::Pill,
            Rect::new(0.0, 0.0, size[0] as f32, size[1] as f32),
        );
        assert!(area.width >= 91.0 && area.height >= 100.0, "{size:?}");
        assert_eq!(size, [160, 120]);
    }

    /// A measurer whose every grapheme is wider than any node.
    struct Vast;

    impl Measure for Vast {
        fn width(&self, text: &str, _face: Face, _size: f32) -> f32 {
            text.len() as f32 * 1e8
        }

        fn line_height(&self, face: Face, size: f32) -> f32 {
            M.line_height(face, size)
        }
    }

    #[test]
    fn growth_stops_at_the_scene_s_limit() {
        let rect = node(NodeKind::Shape, Shape::Rect, [120, 60]);
        let size = grown(&rect, [MAX_SIDE - 30, 60], "ab", &Vast);
        assert_eq!(size, [MAX_SIDE, 60]);
        let tall = "a\n".repeat(900_000);
        let size = grown(&rect, [120, MAX_SIDE - 30], tall.trim_end(), M);
        assert_eq!(size, [120, MAX_SIDE]);
    }
}
