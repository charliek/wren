//! Task lines: what follows a task's box, read into byte ranges, and
//! written back without moving anything the change does not name.
//!
//! [`parse`] reads a line the classifier found a task box on
//! ([`block::Task`]) into a [`TaskLine`]: its description, its `@owner`s
//! and the tokens after the description. The tokens Wren reads are a due
//! date (`📅 2026-12-01`), a done or a cancelled stamp (`✅`, `❌`), a
//! priority (`🔺 ⏫ 🔼 🔽 ⏬`), an id (`🆔 abc`) and a trailing `^block-id`.
//! Every other token Obsidian Tasks or Dataview would read there is
//! [`Kept`]: found, so the tokens before it are still read, and never
//! changed.
//!
//! # Reading
//!
//! Tokens are read backwards from the end of the line, as Obsidian Tasks
//! reads them, so the two agree on what a line says:
//!
//! - White space at the end is skipped. A `^block-id` (letters, digits and
//!   `-`, after white space) is read once, at the very end.
//! - Then a token is taken from the end again and again until the end is
//!   no token. What is left is the description, so a token with other text
//!   after it is description: `📅 2026-12-01 then lunch` has no due date.
//! - White space between tokens is skipped and none is needed, except
//!   before a `#tag`.
//! - A date token is its glyph, any number of spaces (none too) and text in
//!   the shape `YYYY-MM-DD`. A date in that shape that is no real one
//!   (`2026-02-30`) is still the token, read as [`Parsed::Invalid`]; text
//!   in another shape (`📅 tomorrow`) is no token, and ends the tokens.
//!   `📆`, `🗓` and Dataview's `[due:: 2026-12-01]` are read as due dates.
//! - Every glyph is read with or without U+FE0F after it.
//!
//! Where Wren reads more than Obsidian Tasks does, or otherwise:
//!
//! - An `@owner` standing as a word of its own among the tokens does not
//!   end them (Obsidian Tasks lets a `#tag` stand there, and to it `@sam`
//!   is plain text), and neither does a `[key:: value]` field, whose value
//!   may hold a `#tag` or brackets that pair up (`[[a link]]`).
//! - With several tokens of one kind (two priorities, two due dates) the
//!   last on the line is the one read. Obsidian Tasks reads the first.
//! - A token is read the same whatever follows it. Obsidian Tasks tries
//!   each kind once per pass, so it reads `#x⏫ 📅 2026-12-01` as the tag
//!   `#x⏫`; Wren reads the tag `#x` and a priority.
//! - A `#tag` ends at any white space, and a `^block-id` may follow any
//!   white space, not only a space. Any number of tokens is read.
//!
//! The description runs from the first text after the box to the first
//! token that is not an `@owner` or a `#tag`, so `Call the bank @sam #home`
//! is all description. Owners are read in the description (§5.4 of the
//! design: an `@` at its start or after white space, then a letter) and
//! among the tokens.
//!
//! # Writing
//!
//! Each `set_…` method returns the whole line with one thing changed:
//!
//! - A token that is there is changed in place: only the bytes of its
//!   date are replaced, so a `🗓`, a `[due:: …]` field and a U+FE0F after
//!   a date's glyph stay as they were written. A priority of another level
//!   is a new glyph, written bare in the old one's place, U+FE0F and all.
//! - A new token is written as Obsidian Tasks writes it (`📅`, `✅`, `❌`
//!   and the priority glyphs with no U+FE0F, one space before the date) and
//!   goes as late on the line as that tool's order allows: before the
//!   tokens at the end that its order puts after it, and so always before
//!   the `^block-id`. The order is `🆔`, `⛔`, priority, `🔁`, `🏁`, `➕`,
//!   `🛫`, `⏳`, due, cancelled, done, then the block id. A tag, a field
//!   or an owner among the tokens has no place in that order and is
//!   looked past, never moved.
//! - Of several tokens of one kind, a new date changes the last, the one
//!   the line is read by, and leaves the others, so writing the date a
//!   line is read as having changes nothing. Setting a priority replaces
//!   them all with one, as the design asks.
//! - Clearing a value removes every token of its kind. A removed token
//!   takes the white space before it with it. Where the next token touched
//!   it, the white space stays, or a space goes in, so that token is not
//!   left touching what it would be read as part of.
//! - Only what was read as a token is ever taken out. Text the line held
//!   as description stays, even where it is read as a token afterwards.
//!
//! Nothing else on the line moves or changes: not an `@owner`, not a kept
//! token, and not the order of tokens someone wrote by hand.
//!
//! Every writer of a task line in a document goes through [`edit`], which
//! applies a [`Change`] with these writers and the stamps a status change
//! asks for, as one replacement that knows the line it expects.

use std::cell::{Cell, OnceCell};
use std::ops::Range;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

use crate::block::{self, Boxes};
use crate::date::{Date, Parsed};

mod change;
mod form;
mod selection;
pub mod suggest;
mod summary;

pub use change::{edit, rewrite, Change, LineEdit, Stale};
pub use form::{Form, FormKind, Problem};
pub use selection::{join, toggle, Refused};
pub use summary::{summary, Look, Piece, Summary, SEPARATOR};

/// The variation selector an emoji picker may put after a glyph.
const VS16: char = '\u{fe0f}';

/// The bytes of `YYYY-MM-DD`.
const ISO_LEN: usize = 10;

/// The characters a `#tag` ends at, besides white space: Obsidian Tasks'
/// own list.
const TAG_STOPS: &str = "!@#$%^&*(),.?\":{}|<>";

/// Punctuation that can close the word before it; see [`closes`].
const CLOSERS: &str = ".,;:!?)]}'\"\u{2019}\u{201d}\u{2026}\u{bb}";

/// A task's priority. A task with none sorts between
/// [`Medium`](Self::Medium) and [`Low`](Self::Low).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Priority {
    Highest,
    High,
    Medium,
    Low,
    Lowest,
}

impl Priority {
    /// Every level, the highest first.
    pub const ALL: [Self; 5] = [
        Self::Highest,
        Self::High,
        Self::Medium,
        Self::Low,
        Self::Lowest,
    ];

    /// The glyph that writes this level, as one character with no U+FE0F.
    pub const fn glyph(self) -> char {
        match self {
            Self::Highest => '🔺',
            Self::High => '⏫',
            Self::Medium => '🔼',
            Self::Low => '🔽',
            Self::Lowest => '⏬',
        }
    }

    /// The level `glyph` writes.
    pub fn from_glyph(glyph: char) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.glyph() == glyph)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Highest => "Highest",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
            Self::Lowest => "Lowest",
        }
    }
}

/// The dates Wren reads and writes on a task line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateField {
    Due,
    /// The day the task was cancelled.
    Cancelled,
    /// The day the task was done.
    Done,
}

impl DateField {
    pub const ALL: [Self; 3] = [Self::Due, Self::Cancelled, Self::Done];

    /// The glyph Wren writes the date with. A due date is also read from
    /// `📆`, `🗓` and `[due:: …]`.
    pub const fn glyph(self) -> char {
        match self {
            Self::Due => '📅',
            Self::Cancelled => '❌',
            Self::Done => '✅',
        }
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Due => 9,
            Self::Cancelled => 10,
            Self::Done => 11,
        }
    }
}

/// Where a priority goes among the tokens; see [`Token::rank`].
const PRIORITY_RANK: u8 = 3;

/// A token Wren keeps exactly as written and does not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kept {
    /// `➕ 2026-11-19`.
    Created,
    /// `🛫 2026-11-19`.
    Start,
    /// `⏳ 2026-11-19`, or with `⌛`.
    Scheduled,
    /// `🔁 every week`.
    Recurrence,
    /// `🏁 delete`.
    OnCompletion,
    /// `⛔ abc,def`.
    DependsOn,
    /// A `#tag` after another token. One before every token is part of
    /// the description.
    Tag,
    /// `[key:: value]`, a Dataview field other than a due date.
    Field,
}

/// A due date, or a done or cancelled stamp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateToken {
    /// The whole token: its glyph, or its brackets, to the end of the
    /// date.
    pub range: Range<usize>,
    pub field: DateField,
    /// The glyph it is written with, which for a due date may be `📆` or
    /// `🗓`; `None` for a `[due:: …]` field.
    pub glyph: Option<char>,
    /// The date's ten bytes.
    pub date: Range<usize>,
    /// The date, or [`Parsed::Invalid`] for text in a date's shape that is
    /// no real date, kept as written.
    pub value: Parsed,
}

/// A priority glyph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriorityToken {
    /// The glyph and a U+FE0F after it.
    pub range: Range<usize>,
    pub level: Priority,
}

/// `🆔 abc`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdToken {
    pub range: Range<usize>,
    /// The id itself.
    pub id: Range<usize>,
}

/// A token Wren does not read; see [`Kept`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptToken {
    pub range: Range<usize>,
    pub kind: Kept,
}

/// One token after a task's description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Date(DateToken),
    Priority(PriorityToken),
    Id(IdToken),
    /// The `^block-id` that ends the line, with its `^`.
    BlockId(Range<usize>),
    Kept(KeptToken),
    /// An `@owner` written after another token, with its `@`. It is also
    /// one of [`TaskLine::owners`].
    Owner(Range<usize>),
}

impl Token {
    /// The token's bytes in the line.
    pub fn range(&self) -> Range<usize> {
        match self {
            Self::Date(token) => token.range.clone(),
            Self::Priority(token) => token.range.clone(),
            Self::Id(token) => token.range.clone(),
            Self::Kept(token) => token.range.clone(),
            Self::BlockId(range) | Self::Owner(range) => range.clone(),
        }
    }

    /// The token's place in the order Obsidian Tasks writes a line in, the
    /// smaller the earlier. `None` for a token that order has no place
    /// for.
    fn rank(&self) -> Option<u8> {
        Some(match self {
            Self::Id(_) => 1,
            Self::Priority(_) => PRIORITY_RANK,
            Self::Date(token) => token.field.rank(),
            Self::BlockId(_) => 12,
            Self::Kept(token) => match token.kind {
                Kept::DependsOn => 2,
                Kept::Recurrence => 4,
                Kept::OnCompletion => 5,
                Kept::Created => 6,
                Kept::Start => 7,
                Kept::Scheduled => 8,
                Kept::Tag | Kept::Field => return None,
            },
            Self::Owner(_) => return None,
        })
    }
}

/// An `@owner`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    /// The `@` and the name.
    pub range: Range<usize>,
    /// The name, without the `@` and without a `.` that ends it.
    pub name: Range<usize>,
}

/// A task line as read by [`parse`]. Every range is in bytes of
/// [`line`](Self::line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskLine<'a> {
    line: &'a str,
    task: block::Task,
    description: Range<usize>,
    owners: Vec<Owner>,
    tokens: Vec<Token>,
}

/// Read the task line whose box the classifier found at `task`. `None`
/// when `line` has no such box there, so a `task` of another line is
/// refused, not misread.
///
/// The reading needs no settings and no clock: a date is the day it
/// names, and whether it is overdue is the caller's to say.
pub fn parse(line: &str, task: block::Task) -> Option<TaskLine<'_>> {
    if block::task_box(line, task.start, Boxes::Status) != Some(task) {
        return None;
    }
    let after_box = line.get(task.end..)?;
    let floor = line.len() - after_box.trim_start().len();
    let reader = Reader {
        line,
        floor,
        tagless: Cell::new((0, 0)),
        brackets: OnceCell::new(),
    };
    let mut end = reader.trim(line.len());
    let mut tokens = Vec::new();
    if let Some(range) = reader.block_id(end) {
        end = reader.trim(range.start);
        tokens.push(Token::BlockId(range));
    }
    while let Some(token) = reader.token(end) {
        end = reader.trim(token.range().start);
        tokens.push(token);
    }
    tokens.reverse();
    // An owner or a tag before every other token is the description's.
    let leading = tokens
        .iter()
        .take_while(|token| {
            matches!(
                token,
                Token::Owner(_)
                    | Token::Kept(KeptToken {
                        kind: Kept::Tag,
                        ..
                    })
            )
        })
        .count();
    if let Some(last) = tokens[..leading].last() {
        end = last.range().end;
    }
    tokens.drain(..leading);
    let description = floor..end;
    let mut owners = owners_in(line, description.clone());
    owners.extend(tokens.iter().filter_map(|token| match token {
        Token::Owner(range) => Some(Owner {
            range: range.clone(),
            name: range.start + 1..range.end,
        }),
        _ => None,
    }));
    Some(TaskLine {
        line,
        task,
        description,
        owners,
        tokens,
    })
}

/// The text after a task's box, read from its end. Every `end` is a
/// character boundary of `line` at or after `floor`, where the text
/// starts.
struct Reader<'a> {
    line: &'a str,
    floor: usize,
    /// A stretch `(from, to]` of ends a tag is known not to end at, so a
    /// long run of glyphs with no space between them is not walked again
    /// for each one.
    tagless: Cell<(usize, usize)>,
    /// Each `]` of the text that closes a `[`, with that `[`, in line
    /// order; found once, when a field is first looked for.
    brackets: OnceCell<Vec<(usize, usize)>>,
}

impl<'a> Reader<'a> {
    fn text(&self, end: usize) -> &'a str {
        self.line.get(self.floor..end).unwrap_or_default()
    }

    /// `end` moved back over white space.
    fn trim(&self, end: usize) -> usize {
        self.floor + self.text(end).trim_end().len()
    }

    /// Where the run of `allowed` characters that ends at `end` starts.
    fn back(&self, end: usize, allowed: impl Fn(char) -> bool) -> usize {
        self.floor + self.text(end).trim_end_matches(allowed).len()
    }

    fn spaces(&self, end: usize) -> usize {
        self.back(end, |c| c == ' ')
    }

    /// The character the text ends with at `end`, one U+FE0F after it
    /// left out, and where that character starts. `None` when it does not
    /// start a grapheme cluster: a `🔺` that a zero-width joiner ties to
    /// the emoji before it is part of that emoji, and taking it as a token
    /// would split it.
    fn glyph(&self, end: usize) -> Option<(usize, char)> {
        let text = self.text(end);
        let text = text.strip_suffix(VS16).unwrap_or(text);
        let glyph = text.chars().next_back()?;
        let start = self.floor + text.len() - glyph.len_utf8();
        let whole = GraphemeCursor::new(start, self.line.len(), true)
            .is_boundary(self.line, 0)
            .unwrap_or(false);
        whole.then_some((start, glyph))
    }

    /// Where `glyph` starts when the text ends with it at `end`.
    fn glyph_is(&self, end: usize, glyph: char) -> Option<usize> {
        self.glyph(end)
            .and_then(|(start, found)| (found == glyph).then_some(start))
    }

    /// The token the text ends with at `end`. The kinds are tried in the
    /// order Obsidian Tasks tries them, which decides the few ends two
    /// kinds could claim. A field comes before a tag, or the `#home]` that
    /// ends `[area:: #home]` would be read as one.
    fn token(&self, end: usize) -> Option<Token> {
        self.priority(end)
            .or_else(|| self.date(end))
            .or_else(|| self.recurrence(end))
            .or_else(|| self.on_completion(end))
            .or_else(|| self.field(end))
            .or_else(|| self.tag(end))
            .or_else(|| self.id(end))
            .or_else(|| self.depends_on(end))
            .or_else(|| self.owner(end))
    }

    /// The `[` that the `]` at `close` closes, brackets pairing as they
    /// nest.
    fn opening(&self, close: usize) -> Option<usize> {
        let pairs = self.brackets.get_or_init(|| {
            let mut open = Vec::new();
            let mut pairs = Vec::new();
            for (at, c) in self.text(self.line.len()).char_indices() {
                match c {
                    '[' => open.push(self.floor + at),
                    ']' => pairs.extend(open.pop().map(|start| (self.floor + at, start))),
                    _ => {}
                }
            }
            pairs
        });
        let found = pairs.binary_search_by_key(&close, |pair| pair.0).ok()?;
        Some(pairs[found].1)
    }

    fn block_id(&self, end: usize) -> Option<Range<usize>> {
        let id = self.back(end, |c| c.is_ascii_alphanumeric() || c == '-');
        let before = self.text(id).strip_suffix('^').filter(|_| id < end)?;
        (before.is_empty() || before.ends_with(char::is_whitespace))
            .then(|| self.floor + before.len()..end)
    }

    fn priority(&self, end: usize) -> Option<Token> {
        let (start, glyph) = self.glyph(end)?;
        Some(Token::Priority(PriorityToken {
            range: start..end,
            level: Priority::from_glyph(glyph)?,
        }))
    }

    fn date(&self, end: usize) -> Option<Token> {
        let start = end.checked_sub(ISO_LEN).filter(|at| *at >= self.floor)?;
        let date = start..end;
        let value = Parsed::from_iso(self.line.get(date.clone())?)?;
        let (start, glyph) = self.glyph(self.spaces(start))?;
        let range = start..end;
        let read = match glyph {
            '📅' | '📆' | '🗓' => Ok(DateField::Due),
            '✅' => Ok(DateField::Done),
            '❌' => Ok(DateField::Cancelled),
            '➕' => Err(Kept::Created),
            '🛫' => Err(Kept::Start),
            '⏳' | '⌛' => Err(Kept::Scheduled),
            _ => return None,
        };
        Some(match read {
            Ok(field) => Token::Date(DateToken {
                range,
                field,
                glyph: Some(glyph),
                date,
                value,
            }),
            Err(kind) => Token::Kept(KeptToken { range, kind }),
        })
    }

    fn recurrence(&self, end: usize) -> Option<Token> {
        let rule = self.back(end, |c| {
            c.is_ascii_alphanumeric() || matches!(c, ' ' | ',' | '!')
        });
        let start = self.glyph_is(rule, '🔁').filter(|_| rule < end)?;
        Some(Token::Kept(KeptToken {
            range: start..end,
            kind: Kept::Recurrence,
        }))
    }

    fn on_completion(&self, end: usize) -> Option<Token> {
        let action = self.back(end, |c| c.is_ascii_alphabetic());
        let start = self
            .glyph_is(self.spaces(action), '🏁')
            .filter(|_| action < end)?;
        Some(Token::Kept(KeptToken {
            range: start..end,
            kind: Kept::OnCompletion,
        }))
    }

    fn tag(&self, end: usize) -> Option<Token> {
        let (from, to) = self.tagless.get();
        if from < end && end <= to {
            return None;
        }
        let name = self.back(end, |c| !c.is_whitespace() && !TAG_STOPS.contains(c));
        let before = self
            .text(name)
            .strip_suffix('#')
            .filter(|before| name < end && !before.ends_with(|c: char| !c.is_whitespace()));
        let Some(before) = before else {
            self.tagless.set((name, end));
            return None;
        };
        Some(Token::Kept(KeptToken {
            range: self.floor + before.len()..end,
            kind: Kept::Tag,
        }))
    }

    fn id(&self, end: usize) -> Option<Token> {
        let id = self.back(end, id_char);
        let start = self.glyph_is(self.spaces(id), '🆔').filter(|_| id < end)?;
        Some(Token::Id(IdToken {
            range: start..end,
            id: id..end,
        }))
    }

    fn depends_on(&self, end: usize) -> Option<Token> {
        let ids = self.back(end, |c| id_char(c) || matches!(c, ' ' | ','));
        let start = self.glyph_is(ids, '⛔')?;
        let listed = self.line.get(ids..end)?.split(',').all(|id| {
            let id = id.trim_matches(' ');
            !id.is_empty() && !id.contains(' ')
        });
        listed.then_some(Token::Kept(KeptToken {
            range: start..end,
            kind: Kept::DependsOn,
        }))
    }

    /// `[key:: value]`. The value may hold brackets that pair up, as a
    /// `[[link]]` does; the key holds none.
    fn field(&self, end: usize) -> Option<Token> {
        let close = self.floor + self.text(end).strip_suffix(']')?.len();
        let open = self.opening(close)?;
        let inside = self.line.get(open + 1..close)?;
        let (key, value) = inside.split_once("::")?;
        if key.trim().is_empty() || key.contains(['[', ']']) {
            return None;
        }
        let range = open..end;
        let due = (key.trim_start_matches(' ') == "due")
            .then(|| Parsed::from_iso(value.trim_matches(' ')))
            .flatten();
        Some(match due {
            Some(parsed) => {
                // Past the `[`, the key, the `::` and the spaces before the date.
                let indent = value.len() - value.trim_start_matches(' ').len();
                let start = range.start + 1 + key.len() + 2 + indent;
                Token::Date(DateToken {
                    range,
                    field: DateField::Due,
                    glyph: None,
                    date: start..start + ISO_LEN,
                    value: parsed,
                })
            }
            None => Token::Kept(KeptToken {
                range,
                kind: Kept::Field,
            }),
        })
    }

    /// An `@owner` that is the whole last word.
    fn owner(&self, end: usize) -> Option<Token> {
        let text = self.text(end);
        for (at, c) in text.char_indices().rev() {
            if c == '@' {
                let name = &text[at + 1..];
                let whole = !name.is_empty() && owner_len(name) == name.len();
                let apart = !text[..at].ends_with(|c: char| !c.is_whitespace());
                return (whole && apart).then(|| Token::Owner(self.floor + at..end));
            }
            if !name_char(c) && !extends(c) {
                return None;
            }
        }
        None
    }
}

/// A character of a task id, as Obsidian Tasks allows them.
fn id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_')
}

/// A character an owner's name runs over after its first letter.
fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '-' | '_')
}

/// Whether `c` joins the character before it in one grapheme cluster, as
/// a combining mark joins its letter.
fn extends(c: char) -> bool {
    if c.is_ascii() {
        return false;
    }
    let mut pair = [b'a'; 5];
    let len = 1 + c.encode_utf8(&mut pair[1..]).len();
    std::str::from_utf8(&pair[..len]).is_ok_and(|pair| pair.graphemes(true).count() == 1)
}

/// The bytes of the owner name `text` starts with, 0 when it starts with
/// none. A name starts with a letter and runs over letters, digits, `.`,
/// `-` and `_`, and a `.` that ends it is left out. A combining mark
/// stays with its letter. Anything else ends the name, white space above
/// all, even where a letter before it (a Malayalam dot reph, say) would
/// take it into its own grapheme cluster.
fn owner_len(text: &str) -> usize {
    let len = owner_run(text);
    text[..len].trim_end_matches('.').len()
}

/// The bytes of the run of name characters `text` starts with, as
/// [`owner_len`] reads them, a `.` at its end kept.
fn owner_run(text: &str) -> usize {
    let mut len = 0;
    for c in text.chars() {
        let named = if len == 0 {
            c.is_alphabetic()
        } else {
            name_char(c) || extends(c)
        };
        if !named {
            break;
        }
        len += c.len_utf8();
    }
    len
}

/// Whether `typed`, what follows an `@` being typed, can be the start of
/// an owner's name as the task line reads one ([`owner_len`]): empty, or
/// all of it the characters a name runs over, a combining mark with its
/// letter. A `.` at its end may yet go on to more of the name.
pub fn is_owner_prefix(typed: &str) -> bool {
    owner_run(typed) == typed.len()
}

/// Whether `after`, the text right behind an owner, starts with
/// punctuation that closes the word before the owner: a run of
/// [`CLOSERS`] with white space or the end of the line behind it. The `.`
/// of `@sam.` does; the `.` of `@sam./notes` starts a path, and the `!`
/// of `@sam![[Map]]` an embed.
fn closes(after: &str) -> bool {
    let rest = after.trim_start_matches(|c: char| CLOSERS.contains(c));
    rest.len() < after.len() && !rest.starts_with(|c: char| !c.is_whitespace())
}

/// The owners in the description at `range` of `line`: each `@` at its
/// start or after white space that a name follows.
fn owners_in(line: &str, range: Range<usize>) -> Vec<Owner> {
    let text = line.get(range.clone()).unwrap_or_default();
    let mut owners = Vec::new();
    let mut apart = true;
    for (at, c) in text.char_indices() {
        if c == '@' && apart {
            let name = range.start + at + 1;
            let len = owner_len(&text[at + 1..]);
            if len > 0 {
                owners.push(Owner {
                    range: name - 1..name + len,
                    name: name..name + len,
                });
            }
        }
        apart = c.is_whitespace();
    }
    owners
}

/// The `#tag`s in the description at `range` of `line`: each `#` at its
/// start or after white space that a name follows. The name runs to white
/// space or to one of [`TAG_STOPS`], as a tag among the tokens does.
fn tags_in(line: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let text = line.get(range.clone()).unwrap_or_default();
    let mut tags = Vec::new();
    let mut apart = true;
    for (at, c) in text.char_indices() {
        if c == '#' && apart {
            let name = &text[at + 1..];
            let len = name
                .find(|c: char| c.is_whitespace() || TAG_STOPS.contains(c))
                .unwrap_or(name.len());
            if len > 0 {
                let start = range.start + at;
                tags.push(start..start + 1 + len);
            }
        }
        apart = c.is_whitespace();
    }
    tags
}

/// The key two spellings of one owner share: `name` with Unicode's simple
/// case folding, so `@Sam` and `@SAM`, `@Σ` and `@ς`, and `@µ` (micro)
/// and `@μ` are one owner. Only for comparing; a name is shown as written.
pub fn fold_name(name: &str) -> String {
    name.chars().map(fold_char).collect()
}

/// `c`'s simple case fold (`CaseFolding.txt`, statuses C and S).
fn fold_char(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let found = FOLDS.binary_search_by(|(first, last, _)| {
        if *last < c {
            std::cmp::Ordering::Less
        } else if *first > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    if let Ok(at) = found {
        let delta = FOLDS[at].2;
        return u32::from(c)
            .checked_add_signed(delta)
            .and_then(char::from_u32)
            .unwrap_or(c);
    }
    // Elsewhere the fold is the lowercase letter, where that is one.
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(lower), None) => lower,
        _ => c,
    }
}

/// Where the simple case fold of Unicode 18.0 differs from Rust's
/// single-character lowercasing: each range of characters, in order, and
/// what is added to each to fold it. Generated by comparing every
/// character's `char::to_lowercase` with `CaseFolding.txt`'s C and S
/// mappings: the folds to another letter than the lowercase one (`µ` to
/// `μ`, `ς` to `σ`, `ſ` to `s`), Cherokee, which folds to its capitals,
/// and letters newer than Rust's own tables.
const FOLDS: [(char, char, i32); 47] = [
    ('\u{b5}', '\u{b5}', 775),
    ('\u{17f}', '\u{17f}', -268),
    ('\u{345}', '\u{345}', 116),
    ('\u{3c2}', '\u{3c2}', 1),
    ('\u{3d0}', '\u{3d0}', -30),
    ('\u{3d1}', '\u{3d1}', -25),
    ('\u{3d5}', '\u{3d5}', -15),
    ('\u{3d6}', '\u{3d6}', -22),
    ('\u{3f0}', '\u{3f0}', -54),
    ('\u{3f1}', '\u{3f1}', -48),
    ('\u{3f5}', '\u{3f5}', -64),
    ('\u{13a0}', '\u{13f5}', 0),
    ('\u{13f8}', '\u{13fd}', -8),
    ('\u{1c80}', '\u{1c80}', -6222),
    ('\u{1c81}', '\u{1c81}', -6221),
    ('\u{1c82}', '\u{1c82}', -6212),
    ('\u{1c83}', '\u{1c84}', -6210),
    ('\u{1c85}', '\u{1c85}', -6211),
    ('\u{1c86}', '\u{1c86}', -6204),
    ('\u{1c87}', '\u{1c87}', -6180),
    ('\u{1c88}', '\u{1c88}', 35267),
    ('\u{1e9b}', '\u{1e9b}', -58),
    ('\u{1fbe}', '\u{1fbe}', -7173),
    ('\u{1fd3}', '\u{1fd3}', -7235),
    ('\u{1fe3}', '\u{1fe3}', -7219),
    ('\u{a7dd}', '\u{a7dd}', -42342),
    ('\u{a7e2}', '\u{a7e2}', -42342),
    ('\u{ab6c}', '\u{ab6d}', -33),
    ('\u{ab70}', '\u{abbf}', -38864),
    ('\u{fb05}', '\u{fb05}', 1),
    ('\u{1df40}', '\u{1df40}', 1),
    ('\u{1df48}', '\u{1df48}', 1),
    ('\u{1df4a}', '\u{1df4a}', 1),
    ('\u{1df4d}', '\u{1df4d}', 1),
    ('\u{1df51}', '\u{1df51}', 1),
    ('\u{1df68}', '\u{1df68}', 1),
    ('\u{1df6a}', '\u{1df6a}', 1),
    ('\u{1df6c}', '\u{1df6c}', 1),
    ('\u{1df6e}', '\u{1df6e}', 1),
    ('\u{1df72}', '\u{1df72}', 1),
    ('\u{1df74}', '\u{1df74}', 1),
    ('\u{1df76}', '\u{1df76}', 1),
    ('\u{1df78}', '\u{1df78}', 1),
    ('\u{1df7a}', '\u{1df7a}', 1),
    ('\u{1df7c}', '\u{1df7c}', 1),
    ('\u{1df7e}', '\u{1df7e}', 1),
    ('\u{1df95}', '\u{1df95}', -122550),
];

/// Whether `name` can be written as an `@owner` and read back whole.
pub fn is_owner_name(name: &str) -> bool {
    !name.is_empty() && owner_len(name) == name.len()
}

/// One replacement in a line: the bytes to replace and their new text.
type Edit = (Range<usize>, String);

impl<'a> TaskLine<'a> {
    /// The line that was read.
    pub fn line(&self) -> &'a str {
        self.line
    }

    /// The task's box, with its symbol and status.
    pub fn task(&self) -> block::Task {
        self.task
    }

    /// The description: from the first text after the box to the first
    /// token, with no white space at either end. Empty, at the place a
    /// description would start, when the line has none.
    pub fn description(&self) -> Range<usize> {
        self.description.clone()
    }

    pub fn description_text(&self) -> &'a str {
        self.text(&self.description)
    }

    /// Every `@owner` on the line, in line order: those in the
    /// description, then those among the tokens.
    pub fn owners(&self) -> &[Owner] {
        &self.owners
    }

    /// The owners' names, without the `@`.
    pub fn owner_names(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.owners.iter().map(|owner| self.text(&owner.name))
    }

    /// Every `#tag` on the line with its `#`, in line order: those in the
    /// description, then those among the tokens ([`Kept::Tag`]).
    pub fn tags(&self) -> Vec<Range<usize>> {
        let mut tags = tags_in(self.line, self.description.clone());
        tags.extend(self.tokens.iter().filter_map(|token| match token {
            Token::Kept(token) if token.kind == Kept::Tag => Some(token.range.clone()),
            _ => None,
        }));
        tags
    }

    /// The tokens after the description, in line order.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// The line's text at `range`, empty for a range that is not one of
    /// this line's.
    pub fn text(&self, range: &Range<usize>) -> &'a str {
        self.line.get(range.clone()).unwrap_or_default()
    }

    /// Every token of `field`, in line order.
    pub fn dates(&self, field: DateField) -> impl Iterator<Item = &DateToken> {
        self.tokens.iter().filter_map(move |token| match token {
            Token::Date(date) if date.field == field => Some(date),
            _ => None,
        })
    }

    /// The date of `field` the line is read as having: its last.
    pub fn date(&self, field: DateField) -> Option<&DateToken> {
        self.dates(field).last()
    }

    /// The due date.
    pub fn due(&self) -> Option<&DateToken> {
        self.date(DateField::Due)
    }

    /// Every priority glyph among the tokens, in line order.
    pub fn priorities(&self) -> impl Iterator<Item = &PriorityToken> {
        self.tokens.iter().filter_map(|token| match token {
            Token::Priority(priority) => Some(priority),
            _ => None,
        })
    }

    /// The priority the line is read as having: its last.
    pub fn priority(&self) -> Option<&PriorityToken> {
        self.priorities().last()
    }

    /// The `🆔`, the last when there are several.
    pub fn id(&self) -> Option<&IdToken> {
        self.tokens.iter().rev().find_map(|token| match token {
            Token::Id(id) => Some(id),
            _ => None,
        })
    }

    /// The `^block-id` that ends the line, with its `^`.
    pub fn block_id(&self) -> Option<Range<usize>> {
        match self.tokens.last() {
            Some(Token::BlockId(range)) => Some(range.clone()),
            _ => None,
        }
    }

    /// The line with `symbol` in its box. `None` for a symbol no box can
    /// hold: `]`, or a line break. Only the box changes; a stamp is the
    /// caller's to set or clear with [`set_date`](Self::set_date).
    pub fn set_symbol(&self, symbol: char) -> Option<String> {
        if matches!(symbol, ']' | '\n' | '\r') {
            return None;
        }
        Some(self.apply(vec![(self.task.symbol_range(), symbol.to_string())]))
    }

    /// The line with `text` as its description, in the place of the old
    /// one. The `@owner`s of a description are part of its text, so they
    /// stay where `text` has them. White space around `text` is dropped
    /// and a line break in it becomes a space. Text that ends in a token
    /// is read as having one, as it would be if it were typed.
    pub fn set_description(&self, text: &str) -> String {
        self.apply(self.description_edits(text))
    }

    /// The line with its `field` date set to `date`, or with none.
    ///
    /// A date that is there is changed in place: the token the line is
    /// read by (the last of the field) gets the new date and keeps its
    /// glyph, its U+FE0F and its spacing, so a `🗓` stays a `🗓` and a
    /// `[due:: …]` a field. Nothing else changes, an earlier token of the
    /// field included, so writing the date the line already has changes
    /// nothing. With none there, a new one goes in where the module's
    /// writing order puts it.
    ///
    /// `None` removes every token of the field. What the line holds as
    /// description stays, a date inside it too.
    pub fn set_date(&self, field: DateField, date: Option<Date>) -> String {
        self.apply(self.date_edits(field, date))
    }

    /// The line with its priority set to `level`, or with none.
    ///
    /// Setting a level replaces every indicator among the tokens (design
    /// §5.7): the last becomes the new glyph in place, written bare as a
    /// new glyph is (a U+FE0F after the old one goes with it), unless it is
    /// already `level`, when it stays as written, and the others are
    /// removed. So a line with two
    /// indicators changes even when `level` is the one it is read as
    /// having; only a line whose one indicator is `level` comes back as
    /// it was. A caller that saves a form the user did not change the
    /// priority in must not call this, or it would tidy a line nobody
    /// asked it to.
    ///
    /// With none there, the new glyph goes in where the module's writing
    /// order puts it. `None` removes them all. A glyph inside the
    /// description is text, and stays.
    pub fn set_priority(&self, level: Option<Priority>) -> String {
        self.apply(self.priority_edits(level))
    }

    /// The line with `@name` added at the end of its description, or
    /// unchanged when that owner, spelled exactly so, is already on it.
    /// `None` when `name` is no owner name ([`is_owner_name`]).
    pub fn add_owner(&self, name: &str) -> Option<String> {
        self.owner_addition(name).map(|edits| self.apply(edits))
    }

    /// The line without the owners it is read as having under `name`,
    /// wherever they are written. The name is matched exactly, so `sam`
    /// leaves `@Sam`. Each goes with the white space before it, unless
    /// what follows it would then join the word before: `ask @sam.`
    /// becomes `ask.`, but `mail @sam@work.com` becomes `mail @work.com`
    /// and `open @sam./notes` becomes `open ./notes`.
    pub fn remove_owner(&self, name: &str) -> String {
        self.apply(self.owner_removals(name))
    }

    fn description_edits(&self, text: &str) -> Vec<Edit> {
        let text = text.replace(['\n', '\r'], " ");
        let text = text.trim();
        let old = self.description.clone();
        if text == self.description_text() {
            return Vec::new();
        }
        let after = self.line.get(old.end..).unwrap_or_default();
        let edit = if old.is_empty() {
            self.insert_at(0, text)
        } else if text.is_empty() {
            let space = after.len() - after.trim_start().len();
            (old.start..old.end + space, String::new())
        } else if after.starts_with(|c: char| !c.is_whitespace()) {
            // A token that touched the old description would be read as
            // part of the new one's last word: `#home🆔7` is one tag.
            (old, format!("{text} "))
        } else {
            (old, text.to_owned())
        };
        vec![edit]
    }

    fn date_edits(&self, field: DateField, date: Option<Date>) -> Vec<Edit> {
        match (date, self.date(field)) {
            (None, _) => self.removals(self.dates(field).map(|token| token.range.clone())),
            (Some(date), Some(last)) => vec![(last.date.clone(), date.to_string())],
            (Some(date), None) => {
                let token = format!("{} {date}", field.glyph());
                vec![self.insertion(field.rank(), &token)]
            }
        }
    }

    fn priority_edits(&self, level: Option<Priority>) -> Vec<Edit> {
        let ranges = self.priorities().map(|token| token.range.clone());
        match (level, self.priority()) {
            (None, _) => self.removals(ranges),
            (Some(level), Some(last)) => {
                let mut edits = self.removals(ranges.take_while(|range| *range != last.range));
                // Another level is a new glyph, written bare; the same
                // level stays as it is written, U+FE0F and all.
                if last.level != level {
                    edits.push((last.range.clone(), level.glyph().to_string()));
                }
                edits
            }
            (Some(level), None) => {
                vec![self.insertion(PRIORITY_RANK, &level.glyph().to_string())]
            }
        }
    }

    fn owner_addition(&self, name: &str) -> Option<Vec<Edit>> {
        if !is_owner_name(name) {
            return None;
        }
        let there = self.owner_names().any(|owner| owner == name);
        Some(if there {
            Vec::new()
        } else {
            vec![self.insert_at(0, &format!("@{name}"))]
        })
    }

    fn word_edits(&self, range: &Range<usize>, text: &str) -> Option<Vec<Edit>> {
        let fits = range.start <= range.end
            && range.start >= self.description.start
            && self.line.get(range.clone()).is_some()
            && !text.contains(['\n', '\r']);
        if !fits {
            return None;
        }
        Some(if text.is_empty() {
            self.removals(std::iter::once(range.clone()))
        } else {
            vec![(range.clone(), text.to_owned())]
        })
    }

    fn owner_removals(&self, name: &str) -> Vec<Edit> {
        let named = self
            .owners
            .iter()
            .filter(|owner| self.text(&owner.name) == name);
        self.removals(named.map(|owner| owner.range.clone()))
    }

    /// The edit that writes the new token `piece`, whose place in the
    /// writing order is `rank`: as late on the line as it can go and still
    /// be before every token at the end that the order puts after it. A
    /// token the order has no place for (a tag, a field, an owner) is
    /// looked past, so `a 📅 2026-12-01 #home` gets its priority before
    /// the date. Looking back stops at the first token the order puts
    /// before the new one, or at the same place.
    fn insertion(&self, rank: u8, piece: &str) -> Edit {
        let mut index = self.tokens.len();
        for (at, token) in self.tokens.iter().enumerate().rev() {
            match token.rank() {
                Some(other) if other > rank => index = at,
                Some(_) => break,
                None => {}
            }
        }
        self.insert_at(index, piece)
    }

    /// The edit that puts `piece` before token `index`: after the token
    /// before it, or after the description for the first. It gets a space
    /// on each side that has none.
    fn insert_at(&self, index: usize, piece: &str) -> Edit {
        let after = match index.checked_sub(1) {
            Some(before) => Some(self.tokens[before].range().end),
            None => (!self.description.is_empty()).then_some(self.description.end),
        };
        let start = self.description.start;
        let (at, before) = match after {
            Some(at) => (at, " "),
            // Nothing is before it but the box, and its own white space.
            None if start == self.task.end => (start, " "),
            None => (start, ""),
        };
        let rest = self.line.get(at..).unwrap_or_default();
        let touches = rest.starts_with(|c: char| !c.is_whitespace());
        let after = if touches { " " } else { "" };
        (at..at, format!("{before}{piece}{after}"))
    }

    /// The edits that take `ranges` (in line order) out, each with the
    /// white space before it. The first thing after the box takes the
    /// white space after it instead, which leaves the box its own.
    ///
    /// What touches the end of a range must not come to touch what was
    /// before it, where the two could be read as one (`#x🆔7` is one tag,
    /// `mail@work.com` one address). So there the white space before
    /// stays, and where there is none a space goes in. Only punctuation
    /// that closes a word ([`closes`]) may join it: `ask @sam.` becomes
    /// `ask.`.
    fn removals(&self, ranges: impl Iterator<Item = Range<usize>>) -> Vec<Edit> {
        let start = self.description.start;
        let mut edits: Vec<Edit> = Vec::new();
        let mut ranges = ranges.peekable();
        while let Some(mut range) = ranges.next() {
            // What touches goes as one.
            while let Some(next) = ranges.next_if(|next| next.start <= range.end) {
                range.end = range.end.max(next.end);
            }
            let after = self.line.get(range.end..).unwrap_or_default();
            let touched = after.starts_with(|c: char| !c.is_whitespace()) && !closes(after);
            let before = self.line.get(start..range.start).unwrap_or_default();
            let spaced = start + before.trim_end().len();
            let (from, text) = match touched {
                true if spaced == range.start && range.start > start => (range.start, " "),
                true => (range.start, ""),
                false => (spaced, ""),
            };
            match edits.last_mut() {
                Some((last, _)) if from <= last.end => last.end = last.end.max(range.end),
                _ => edits.push((from..range.end, text.to_owned())),
            }
        }
        for (range, _) in &mut edits {
            if range.start == start {
                let after = self.line.get(range.end..).unwrap_or_default();
                range.end += after.len() - after.trim_start().len();
            }
        }
        edits
    }

    /// The line with `edits` made. They do not overlap.
    fn apply(&self, mut edits: Vec<Edit>) -> String {
        edits.sort_by_key(|(range, _)| range.start);
        let mut out = String::with_capacity(self.line.len() + 16);
        let mut at = 0;
        for (range, text) in &edits {
            out.push_str(self.line.get(at..range.start).unwrap_or_default());
            out.push_str(text);
            at = at.max(range.end);
        }
        out.push_str(self.line.get(at..).unwrap_or_default());
        out
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::block::Scanner;
    use crate::status::StatusType;

    /// The box of `line` read as a document of its own, as a host with
    /// tasks on classifies it.
    fn find(line: &str) -> Option<block::Task> {
        Scanner::new(Boxes::Status)
            .next_line(line)
            .item
            .and_then(|item| item.task)
    }

    fn read(line: &str) -> TaskLine<'_> {
        let task = find(line).unwrap_or_else(|| panic!("{line:?} is no task line"));
        parse(line, task).unwrap_or_else(|| panic!("{line:?} is not read"))
    }

    fn day(text: &str) -> Date {
        Date::parse_iso(text).unwrap_or_else(|| panic!("{text} is a date"))
    }

    /// A reading on one line: the description in `<>`, the owners, then
    /// each token. A date is `field=date(glyph)`, with `!=` for a date
    /// that is no real one and `[]` for a Dataview field.
    fn summary(line: &str) -> String {
        let read = read(line);
        let mut parts = vec![format!("<{}>", read.description_text())];
        parts.extend(read.owner_names().map(|name| format!("@{name}")));
        for token in read.tokens() {
            let text = read.text(&token.range());
            parts.push(match token {
                Token::Date(date) => {
                    let field = match date.field {
                        DateField::Due => "due",
                        DateField::Done => "done",
                        DateField::Cancelled => "cancelled",
                    };
                    let equals = match date.value {
                        Parsed::Date(_) => "=",
                        Parsed::Invalid => "!=",
                    };
                    let glyph = date.glyph.map_or("[]".to_owned(), String::from);
                    format!("{field}{equals}{}({glyph})", read.text(&date.date))
                }
                Token::Priority(priority) => priority.level.name().to_lowercase(),
                Token::Id(id) => format!("id={}", read.text(&id.id)),
                Token::BlockId(_) => format!("block={text}"),
                Token::Kept(kept) => {
                    format!("{}[{text}]", format!("{:?}", kept.kind).to_lowercase())
                }
                Token::Owner(_) => format!("owner={text}"),
            });
        }
        parts.join(" ")
    }

    fn check_reads(cases: &[(&str, &str)]) {
        for (line, expected) in cases {
            assert_eq!(summary(line), *expected, "{line:?}");
        }
    }

    fn check_writes(cases: &[(&str, &str)], change: impl Fn(&TaskLine) -> String) {
        for (before, after) in cases {
            assert_eq!(change(&read(before)), *after, "{before:?}");
        }
    }

    #[test]
    fn every_token_kind_is_read() {
        check_reads(&[
            ("- [ ] a", "<a>"),
            ("- [ ] a 📅 2026-12-01", "<a> due=2026-12-01(📅)"),
            ("- [x] a ✅ 2026-11-19", "<a> done=2026-11-19(✅)"),
            ("- [-] a ❌ 2026-11-18", "<a> cancelled=2026-11-18(❌)"),
            ("- [ ] a 🔺", "<a> highest"),
            ("- [ ] a ⏫", "<a> high"),
            ("- [ ] a 🔼", "<a> medium"),
            ("- [ ] a 🔽", "<a> low"),
            ("- [ ] a ⏬", "<a> lowest"),
            ("- [ ] a 🆔 abc-1_2", "<a> id=abc-1_2"),
            ("- [ ] a ^blk-1", "<a> block=^blk-1"),
            ("- [ ] a ➕ 2026-11-01", "<a> created[➕ 2026-11-01]"),
            ("- [ ] a 🛫 2026-11-02", "<a> start[🛫 2026-11-02]"),
            ("- [ ] a ⏳ 2026-11-03", "<a> scheduled[⏳ 2026-11-03]"),
            ("- [ ] a ⌛ 2026-11-03", "<a> scheduled[⌛ 2026-11-03]"),
            ("- [ ] a 🔁 every week", "<a> recurrence[🔁 every week]"),
            (
                "- [ ] a 🔁 every 2 weeks on Monday, Friday when done!",
                "<a> recurrence[🔁 every 2 weeks on Monday, Friday when done!]",
            ),
            ("- [ ] a 🏁 delete", "<a> oncompletion[🏁 delete]"),
            ("- [ ] a ⛔ abc", "<a> dependson[⛔ abc]"),
            ("- [ ] a ⛔ ab , c-d,e_f", "<a> dependson[⛔ ab , c-d,e_f]"),
            ("- [ ] a [key:: value]", "<a> field[[key:: value]]"),
            (
                "- [ ] Send the deck to finance @sam ⏫ 📅 2026-12-01",
                "<Send the deck to finance @sam> @sam high due=2026-12-01(📅)",
            ),
            (
                "- [x] Send the deck to finance @sam 📅 2026-12-01 ✅ 2026-11-28",
                "<Send the deck to finance @sam> @sam due=2026-12-01(📅) done=2026-11-28(✅)",
            ),
            (
                "- [ ] a 🆔 x1 ⛔ y,z ⏫ 🔁 every week 🏁 delete ➕ 2026-01-01 🛫 2026-01-02 \
                 ⏳ 2026-01-03 📅 2026-12-01 ❌ 2026-11-18 ✅ 2026-11-19 ^b1",
                "<a> id=x1 dependson[⛔ y,z] high recurrence[🔁 every week] \
                 oncompletion[🏁 delete] created[➕ 2026-01-01] start[🛫 2026-01-02] \
                 scheduled[⏳ 2026-01-03] due=2026-12-01(📅) cancelled=2026-11-18(❌) \
                 done=2026-11-19(✅) block=^b1",
            ),
        ]);
    }

    #[test]
    fn tokens_are_read_in_any_order_from_the_end() {
        check_reads(&[
            (
                "- [ ] a ✅ 2026-11-19 ^b",
                "<a> done=2026-11-19(✅) block=^b",
            ),
            ("- [ ] a ^b ✅ 2026-11-19", "<a ^b> done=2026-11-19(✅)"),
            (
                "- [ ] a ✅ 2026-11-19 📅 2026-12-01 🔽 🆔 q",
                "<a> done=2026-11-19(✅) due=2026-12-01(📅) low id=q",
            ),
            (
                "- [ ] a 🔁 every week 📅 2026-12-01 🏁 keep ⏫",
                "<a> recurrence[🔁 every week] due=2026-12-01(📅) oncompletion[🏁 keep] high",
            ),
            (
                "- [ ] a 📅 2026-12-01 ⏫ @sam #x ^id",
                "<a> @sam due=2026-12-01(📅) high owner=@sam tag[#x] block=^id",
            ),
        ]);
    }

    #[test]
    fn tags_are_read_in_the_description_and_among_the_tokens() {
        for (line, expected) in [
            ("- [ ] Call the bank", vec![]),
            ("- [ ] Call #home", vec!["#home"]),
            ("- [ ] #home call #a/b-c now", vec!["#home", "#a/b-c"]),
            (
                "- [ ] Call #home ⏫ #work 📅 2026-12-01 #late",
                vec!["#home", "#work", "#late"],
            ),
            // A tag's name ends at white space and at Obsidian Tasks' stops.
            (
                "- [ ] Call #home, #日本. #a\t#b:c",
                vec!["#home", "#日本", "#a", "#b"],
            ),
            // A `#` after other text, or with no name, starts none.
            ("- [ ] a#b (#c) ## # #!x c# #", vec![]),
            // A tag in a field is part of the field.
            ("- [ ] Call [area:: #home] #work", vec!["#work"]),
        ] {
            let read = read(line);
            let tags: Vec<&str> = read.tags().iter().map(|tag| read.text(tag)).collect();
            assert_eq!(tags, expected, "{line:?}");
        }
    }

    #[test]
    fn text_after_the_tokens_ends_them() {
        check_reads(&[
            ("- [ ] pay 📅 2026-12-01 rent", "<pay 📅 2026-12-01 rent>"),
            (
                "- [ ] pay 📅 2026-12-01 rent ⏫",
                "<pay 📅 2026-12-01 rent> high",
            ),
            ("- [ ] a ⏫ b 🔽", "<a ⏫ b> low"),
            ("- [ ] a ✅ 2026-11-19.", "<a ✅ 2026-11-19.>"),
            (
                "- [ ] a 🆔 x y 📅 2026-12-01",
                "<a 🆔 x y> due=2026-12-01(📅)",
            ),
            ("- [ ] a ⛔ x y", "<a ⛔ x y>"),
            ("- [ ] a ⛔ x,,y", "<a ⛔ x,,y>"),
            (
                "- [ ] a 🔁 every week; 📅 2026-12-01",
                "<a 🔁 every week;> due=2026-12-01(📅)",
            ),
            ("- [ ] a 🏁 delete it", "<a 🏁 delete it>"),
            ("- [ ] a 🔁", "<a 🔁>"),
            ("- [ ] a 🆔", "<a 🆔>"),
        ]);
    }

    /// A date token is its glyph, spaces and the shape `YYYY-MM-DD`.
    /// Anything else after the glyph is text, which ends the tokens as any
    /// text does.
    #[test]
    fn a_date_that_is_not_in_iso_shape_is_no_token() {
        check_reads(&[
            ("- [ ] a 📅 tomorrow", "<a 📅 tomorrow>"),
            ("- [ ] a ⏫ 📅 tomorrow", "<a ⏫ 📅 tomorrow>"),
            ("- [ ] a 📅 tomorrow ⏫", "<a 📅 tomorrow> high"),
            ("- [ ] a 📅", "<a 📅>"),
            ("- [ ] a ✅", "<a ✅>"),
            ("- [ ] a 📅 2026-12-1", "<a 📅 2026-12-1>"),
            ("- [ ] a 📅 12026-12-01", "<a 📅 12026-12-01>"),
            ("- [ ] a 📅 2026-12-011", "<a 📅 2026-12-011>"),
            ("- [ ] a 📅 2026/12/01", "<a 📅 2026/12/01>"),
            ("- [ ] a 📅 ２０２６-12-01", "<a 📅 ２０２６-12-01>"),
            ("- [ ] a 📅\t2026-12-01", "<a 📅\t2026-12-01>"),
            ("- [ ] a 📅 x 2026-12-01", "<a 📅 x 2026-12-01>"),
            ("- [ ] a 2026-12-01", "<a 2026-12-01>"),
        ]);
    }

    #[test]
    fn a_date_that_is_no_real_date_is_kept_and_flagged() {
        check_reads(&[
            ("- [ ] a 📅 2026-02-30", "<a> due!=2026-02-30(📅)"),
            ("- [ ] a 📅 2026-13-01 ⏫", "<a> due!=2026-13-01(📅) high"),
            ("- [ ] a 📅 0000-00-00", "<a> due!=0000-00-00(📅)"),
            ("- [x] a ✅ 2027-02-29", "<a> done!=2027-02-29(✅)"),
            ("- [-] a ❌ 2026-04-31", "<a> cancelled!=2026-04-31(❌)"),
            ("- [ ] a [due:: 2026-02-30]", "<a> due!=2026-02-30([])"),
            ("- [ ] a 📅 2024-02-29", "<a> due=2024-02-29(📅)"),
        ]);
        let line = "- [ ] a 📅 2026-02-30";
        let read = read(line);
        let due = read.due().unwrap();
        assert_eq!(due.value, Parsed::Invalid);
        assert_eq!(read.text(&due.date), "2026-02-30");
    }

    #[test]
    fn due_dates_are_read_from_obsidian_and_dataview_forms() {
        check_reads(&[
            ("- [ ] a 📆 2026-12-01", "<a> due=2026-12-01(📆)"),
            ("- [ ] a 🗓 2026-12-01", "<a> due=2026-12-01(🗓)"),
            ("- [ ] a 🗓\u{fe0f} 2026-12-01", "<a> due=2026-12-01(🗓)"),
            ("- [ ] a [due:: 2026-12-01]", "<a> due=2026-12-01([])"),
            ("- [ ] a [due::2026-12-01]", "<a> due=2026-12-01([])"),
            ("- [ ] a [ due::  2026-12-01 ]", "<a> due=2026-12-01([])"),
            (
                "- [ ] a  [due:: 2026-12-01] ⏫",
                "<a> due=2026-12-01([]) high",
            ),
            // Not a due date, and still a field that is kept.
            ("- [ ] a [due:: tomorrow]", "<a> field[[due:: tomorrow]]"),
            (
                "- [ ] a [Due:: 2026-12-01]",
                "<a> field[[Due:: 2026-12-01]]",
            ),
            (
                "- [ ] a [due :: 2026-12-01]",
                "<a> field[[due :: 2026-12-01]]",
            ),
            (
                "- [ ] a [due:: 2026-12-01 x]",
                "<a> field[[due:: 2026-12-01 x]]",
            ),
            (
                "- [ ] a [due::\t2026-12-01]",
                "<a> field[[due::\t2026-12-01]]",
            ),
            ("- [ ] a (due:: 2026-12-01)", "<a (due:: 2026-12-01)>"),
            ("- [ ] a [due:: 2026-12-01],", "<a [due:: 2026-12-01],>"),
        ]);
        let line = "- [ ] a [ due::  2026-12-01 ] ⏫";
        let read = read(line);
        let due = read.due().unwrap();
        assert_eq!(read.text(&due.range), "[ due::  2026-12-01 ]");
        assert_eq!(read.text(&due.date), "2026-12-01");
        assert_eq!(due.glyph, None);
        assert_eq!(due.value, Parsed::Date(day("2026-12-01")));
    }

    #[test]
    fn fields_are_kept_and_other_brackets_are_text() {
        check_reads(&[
            (
                "- [ ] a [k:: v] 📅 2026-12-01 [priority:: high]",
                "<a> field[[k:: v]] due=2026-12-01(📅) field[[priority:: high]]",
            ),
            ("- [ ] a [k::]", "<a> field[[k::]]"),
            // A value may hold a tag, and brackets that pair up.
            (
                "- [ ] Pay 📅 2026-12-01 [area:: #home] ^b",
                "<Pay> due=2026-12-01(📅) field[[area:: #home]] block=^b",
            ),
            (
                "- [ ] Pay 📅 2026-12-01 [project:: [[Roadmap]]]",
                "<Pay> due=2026-12-01(📅) field[[project:: [[Roadmap]]]]",
            ),
            (
                "- [ ] a ⏫ [see:: [x](u) and [y]] #t",
                "<a> high field[[see:: [x](u) and [y]]] tag[#t]",
            ),
            ("- [ ] a ⏫ [k:: v]]", "<a ⏫ [k:: v]]>"),
            ("- [ ] a ⏫ [k:: [v]", "<a ⏫ [k:: [v]>"),
            ("- [ ] a ⏫ [[k]:: v]", "<a ⏫ [[k]:: v]>"),
            ("- [ ] a ⏫ #t]", "<a> high tag[#t]]"),
            ("- [ ] a[k::v]⏫", "<a> field[[k::v]] high"),
            (
                "- [ ] a [two words:: v: w]",
                "<a> field[[two words:: v: w]]",
            ),
            ("- [ ] a [:: v]", "<a [:: v]>"),
            ("- [ ] a [ :: v] ⏫", "<a [ :: v]> high"),
            ("- [ ] a [not a field] ⏫", "<a [not a field]> high"),
            ("- [ ] a [[wiki]] ⏫", "<a [[wiki]]> high"),
            ("- [ ] a [[k:: v]] ⏫", "<a [[k:: v]]> high"),
            ("- [ ] a [x](u) ⏫", "<a [x](u)> high"),
            ("- [ ] a k:: v] ⏫", "<a k:: v]> high"),
            ("- [ ] a ] ⏫", "<a ]> high"),
            ("- [ ] [k:: v]", "<> field[[k:: v]]"),
        ]);
    }

    const DATE_GLYPHS: [char; 9] = ['📅', '📆', '🗓', '✅', '❌', '➕', '🛫', '⏳', '⌛'];

    #[test]
    fn every_glyph_is_read_with_or_without_a_variation_selector() {
        for glyph in DATE_GLYPHS {
            let plain = format!("- [ ] a {glyph} 2026-12-01 ^b");
            let selected = format!("- [ ] a {glyph}\u{fe0f} 2026-12-01 ^b");
            let read = read(&selected);
            assert_eq!(
                summary(&selected).replace('\u{fe0f}', ""),
                summary(&plain),
                "{glyph}"
            );
            assert_eq!(
                read.text(&read.tokens()[0].range()),
                format!("{glyph}\u{fe0f} 2026-12-01")
            );
            // One selector is the glyph's; a second is text.
            let twice = format!("- [ ] a {glyph}\u{fe0f}\u{fe0f} 2026-12-01");
            assert_eq!(self::read(&twice).tokens(), [], "{glyph}");
        }
        for level in Priority::ALL {
            let glyph = level.glyph();
            let selected = format!("- [ ] a {glyph}\u{fe0f} 📅 2026-12-01");
            let read = read(&selected);
            let priority = read.priority().unwrap();
            assert_eq!(priority.level, level);
            assert_eq!(read.text(&priority.range), format!("{glyph}\u{fe0f}"));
            assert_eq!(read.description_text(), "a");
            assert_eq!(read.due().unwrap().value, Parsed::Date(day("2026-12-01")));
        }
        for (glyph, value) in [
            ('🆔', "abc"),
            ('⛔', "a,b"),
            ('🔁', "every day"),
            ('🏁', "delete"),
        ] {
            let plain = format!("- [ ] a {glyph} {value} ⏫");
            let selected = format!("- [ ] a {glyph}\u{fe0f} {value} ⏫");
            assert_eq!(
                summary(&selected).replace('\u{fe0f}', ""),
                summary(&plain),
                "{glyph}"
            );
            assert_eq!(self::read(&selected).tokens().len(), 2, "{glyph}");
        }
    }

    #[test]
    fn the_last_of_several_tokens_of_a_kind_is_the_one_read() {
        check_reads(&[
            ("- [ ] a ⏫ 🔽", "<a> high low"),
            (
                "- [ ] a 🔽 📅 2026-12-01 🔺",
                "<a> low due=2026-12-01(📅) highest",
            ),
            (
                "- [ ] a 📅 2026-12-01 📆 2026-12-05",
                "<a> due=2026-12-01(📅) due=2026-12-05(📆)",
            ),
        ]);
        assert_eq!(
            read("- [ ] a ⏫ 🔽").priority().unwrap().level,
            Priority::Low
        );
        let line = "- [ ] a 🔽 📅 2026-12-01 🔺 [due:: 2026-12-09] 🆔 a 🆔 b";
        let read = read(line);
        assert_eq!(read.priority().unwrap().level, Priority::Highest);
        assert_eq!(read.priorities().count(), 2);
        assert_eq!(read.due().unwrap().value, Parsed::Date(day("2026-12-09")));
        assert_eq!(read.dates(DateField::Due).count(), 2);
        assert_eq!(read.text(&read.id().unwrap().id), "b");
        assert_eq!(read.date(DateField::Done), None);
        assert_eq!(read.date(DateField::Cancelled), None);
    }

    #[test]
    fn spacing_between_and_inside_tokens() {
        check_reads(&[
            // No space is needed before a glyph or a field.
            ("- [ ] a⏫", "<a> high"),
            ("- [ ] a ⏫📅 2026-12-01", "<a> high due=2026-12-01(📅)"),
            (
                "- [ ] a📅2026-12-01⏫🆔x",
                "<a> due=2026-12-01(📅) high id=x",
            ),
            ("- [ ] a 📅   2026-12-01", "<a> due=2026-12-01(📅)"),
            ("- [ ] a 🆔   x 🏁  keep", "<a> id=x oncompletion[🏁  keep]"),
            // Any white space separates tokens, and it ends the line.
            (
                "- [ ] a  ⏫\t📅 2026-12-01\u{a0}✅ 2026-11-19 \t ",
                "<a> high due=2026-12-01(📅) done=2026-11-19(✅)",
            ),
            ("- [ ] a\t\t^b\t", "<a> block=^b"),
        ]);
    }

    #[test]
    fn tags_stand_among_the_tokens() {
        check_reads(&[
            ("- [ ] a #x", "<a #x>"),
            ("- [ ] #x", "<#x>"),
            ("- [ ] a #x #y/z ⏫", "<a #x #y/z> high"),
            (
                "- [ ] a #x 📅 2026-12-01 #y ⏫ #z-1 ^b",
                "<a #x> due=2026-12-01(📅) tag[#y] high tag[#z-1] block=^b",
            ),
            ("- [ ] ⏫ #x", "<> high tag[#x]"),
            ("- [ ] a ⏫\t#日本", "<a> high tag[#日本]"),
            // A tag needs white space before it, and a name.
            ("- [ ] a ⏫#x", "<a ⏫#x>"),
            ("- [ ] a 📅 2026-12-01#x", "<a 📅 2026-12-01#x>"),
            ("- [ ] a ⏫ #", "<a ⏫ #>"),
            ("- [ ] a ⏫ #x!", "<a ⏫ #x!>"),
            ("- [ ] a ⏫ #x#y", "<a ⏫ #x#y>"),
            ("- [ ] a ⏫ ##y", "<a ⏫ ##y>"),
            ("- [ ] a ⏫ http://u/p#x", "<a ⏫ http://u/p#x>"),
            // Wren reads a token the same whatever follows it, so the tag
            // is `#x` both times.
            ("- [ ] a #x⏫", "<a #x> high"),
            (
                "- [ ] a #x⏫ 📅 2026-12-01",
                "<a #x> high due=2026-12-01(📅)",
            ),
        ]);
    }

    #[test]
    fn a_block_id_is_read_only_at_the_very_end() {
        check_reads(&[
            ("- [ ] a ^b", "<a> block=^b"),
            ("- [ ] a ^B-2 ", "<a> block=^B-2"),
            ("- [ ] a ⏫ ^b", "<a> high block=^b"),
            ("- [ ] ^b", "<> block=^b"),
            ("- [ ] a ^b ⏫", "<a ^b> high"),
            ("- [ ] a ^b ^c", "<a ^b> block=^c"),
            ("- [ ] a^b", "<a^b>"),
            ("- [ ] a ⏫^b", "<a ⏫^b>"),
            ("- [ ] a ^b_c", "<a ^b_c>"),
            ("- [ ] a ^", "<a ^>"),
            ("- [ ] a ^é", "<a ^é>"),
        ]);
        assert_eq!(read("- [ ] a ^b ").block_id(), Some(8..10));
        assert_eq!(read("- [ ] a ^b ⏫").block_id(), None);
    }

    #[test]
    fn owners_follow_the_at_rule() {
        let cases: &[(&str, &[&str])] = &[
            ("call @sam", &["sam"]),
            ("@sam call", &["sam"]),
            ("@sam @priya go", &["sam", "priya"]),
            ("ask\t@sam", &["sam"]),
            ("ask\u{a0}@sam", &["sam"]),
            (
                "ask @jane.doe, @j-smith_2 and @x9",
                &["jane.doe", "j-smith_2", "x9"],
            ),
            ("ask @sam.", &["sam"]),
            ("ask @sam... later", &["sam"]),
            ("ask @sam, then @priya's team", &["sam", "priya"]),
            ("@josé @Ωmega @日本", &["josé", "Ωmega", "日本"]),
            ("@jose\u{301} x", &["jose\u{301}"]),
            ("@jose\u{301}. x", &["jose\u{301}"]),
            // U+0D4E takes the character after it into its cluster; a
            // name still ends at white space and at punctuation.
            ("call @a\u{d4e} Bob", &["a\u{d4e}"]),
            ("call @a\u{d4e}, Bob", &["a\u{d4e}"]),
            ("call @a\u{d4e}b Bob", &["a\u{d4e}b"]),
            ("@sam@host", &["sam"]),
            ("@sam-", &["sam-"]),
            // Not owners.
            ("mail a@b.com", &[]),
            ("@1abc", &[]),
            ("(@sam)", &[]),
            ("see [@sam](u) and `@sam`", &[]),
            ("@ sam", &[]),
            ("@", &[]),
            ("@@sam", &[]),
            ("@_x @-x @.x", &[]),
            ("@\u{301}sam", &[]),
            ("x@sam", &[]),
        ];
        for (description, names) in cases {
            let line = format!("- [ ] {description} 📅 2026-12-01");
            let read = read(&line);
            assert_eq!(
                read.owner_names().collect::<Vec<_>>(),
                *names,
                "{description:?}"
            );
            assert_eq!(read.description_text(), *description);
            for owner in read.owners() {
                assert_eq!(owner.name, owner.range.start + 1..owner.range.end);
                assert_eq!(&line[owner.range.start..owner.name.start], "@");
            }
        }
        let line = "- [ ] ask @sam. Then go";
        let read = read(line);
        assert_eq!(read.owners()[0].range, 10..14);
        assert_eq!(read.owners()[0].name, 11..14);
    }

    /// An `@owner` that is a word of its own does not end the tokens, so a
    /// date keeps being read when a name is typed after it.
    #[test]
    fn an_owner_among_the_tokens_does_not_end_them() {
        check_reads(&[
            (
                "- [ ] a 📅 2026-12-01 @sam",
                "<a> @sam due=2026-12-01(📅) owner=@sam",
            ),
            (
                "- [ ] a @kim ⏫ @sam @josé 📅 2026-12-01",
                "<a @kim> @kim @sam @josé high owner=@sam owner=@josé due=2026-12-01(📅)",
            ),
            ("- [ ] @sam ⏫", "<@sam> @sam high"),
            ("- [ ] ⏫ @sam", "<> @sam high owner=@sam"),
            (
                "- [ ] a ⏫ @jose\u{301} @a\u{d4e} 📅 2026-12-01",
                "<a> @jose\u{301} @a\u{d4e} high owner=@jose\u{301} owner=@a\u{d4e} due=2026-12-01(📅)",
            ),
            ("- [ ] a ⏫@sam", "<a ⏫@sam>"),
            // An owner with anything after its name is text there.
            (
                "- [ ] a 📅 2026-12-01 @sam.",
                "<a 📅 2026-12-01 @sam.> @sam",
            ),
            (
                "- [ ] a 📅 2026-12-01 @sam@x",
                "<a 📅 2026-12-01 @sam@x> @sam",
            ),
            ("- [ ] a 📅 2026-12-01 (@sam)", "<a 📅 2026-12-01 (@sam)>"),
            ("- [ ] a 📅 2026-12-01 a@b.com", "<a 📅 2026-12-01 a@b.com>"),
            ("- [ ] a 📅 2026-12-01 @1abc", "<a 📅 2026-12-01 @1abc>"),
            ("- [ ] a 📅 2026-12-01 @", "<a 📅 2026-12-01 @>"),
        ]);
    }

    #[test]
    fn every_form_of_task_line_is_read() {
        check_reads(&[
            ("> - [ ] a 📅 2026-12-01", "<a> due=2026-12-01(📅)"),
            ("> > 1. [x] a ✅ 2026-11-19", "<a> done=2026-11-19(✅)"),
            ("12) [/] a ⏫", "<a> high"),
            ("* [-] a ❌ 2026-11-18", "<a> cancelled=2026-11-18(❌)"),
            ("+ [?] a ^b", "<a> block=^b"),
            ("-\t[ ]\ta\t📅 2026-12-01", "<a> due=2026-12-01(📅)"),
            ("  - [界] a 🔼", "<a> medium"),
            ("- [é] a @sam", "<a @sam> @sam"),
            ("- [✓] 📅 2026-12-01", "<> due=2026-12-01(📅)"),
            ("- [ ]", "<>"),
            ("- [ ] ", "<>"),
            ("- [ ]   ⏫   ", "<> high"),
            ("- - [ ] a ⏫", "<a> high"),
            // What follows a box is the task's text, another box too.
            ("- [ ] - [ ] a ⏫", "<- [ ] a> high"),
        ]);
        for (line, description, symbol, status) in [
            ("- [ ]", 5..5, ' ', StatusType::Open),
            ("- [ ]  ", 7..7, ' ', StatusType::Open),
            ("- [x] ✅ 2026-11-19", 6..6, 'x', StatusType::Done),
            ("- [界] a b ⏫", 8..11, '界', StatusType::Open),
            ("> 1. [/]\ta ", 9..10, '/', StatusType::Doing),
            (
                "- [-]  Call  the bank   ^b",
                7..21,
                '-',
                StatusType::Cancelled,
            ),
        ] {
            let read = read(line);
            assert_eq!(read.description(), description, "{line:?}");
            assert_eq!(read.task().symbol, symbol, "{line:?}");
            assert_eq!(read.task().status, status, "{line:?}");
            assert_eq!(read.line(), line);
        }
    }

    #[test]
    fn ranges_are_bytes_of_the_line() {
        let line = "- [界] Call @sam.  🗓\u{fe0f}  2026-12-01 ⏫\u{fe0f} 🆔 q1 ^b ";
        let read = read(line);
        let at = |text: &str| {
            let start = line.find(text).unwrap();
            start..start + text.len()
        };
        assert_eq!(read.task().start..read.task().end, at("[界]"));
        assert_eq!(read.description(), at("Call @sam."));
        assert_eq!(read.owners()[0].range, at("@sam"));
        assert_eq!(read.owners()[0].name, at("sam"));
        let due = read.due().unwrap();
        assert_eq!(due.range, at("🗓\u{fe0f}  2026-12-01"));
        assert_eq!(due.date, at("2026-12-01"));
        assert_eq!(due.glyph, Some('🗓'));
        assert_eq!(read.priority().unwrap().range, at("⏫\u{fe0f}"));
        let id = read.id().unwrap();
        assert_eq!((id.range.clone(), id.id.clone()), (at("🆔 q1"), at("q1")));
        assert_eq!(read.block_id(), Some(at("^b")));
        assert_eq!(read.tokens().len(), 4);
        assert_eq!(read.text(&(0..line.len() + 1)), "");
        assert_eq!(read.text(&(3..4)), "", "inside a character");
    }

    #[test]
    fn a_task_of_another_line_is_refused() {
        let task = find("- [ ] a").unwrap();
        assert!(parse("- [ ] a ⏫", task).is_some());
        for line in [
            "",
            "-",
            "- [ ]a",
            "  - [ ] a",
            "- [x] a",
            "- [界] a",
            "界界界界",
        ] {
            assert_eq!(parse(line, task), None, "{line:?}");
        }
        let wide = find("- [界] a").unwrap();
        assert_eq!(parse("- [ ] a", wide), None);
        let mut wrong = task;
        wrong.status = StatusType::Done;
        assert_eq!(parse("- [ ] a", wrong), None);
        wrong = task;
        wrong.end = 99;
        assert_eq!(parse("- [ ] a", wrong), None);
    }

    /// More tokens than Obsidian Tasks reads in one line, some with no
    /// space between them.
    #[test]
    fn a_long_run_of_tokens_is_read_whole() {
        let mut line = "- [ ] a".to_owned();
        for index in 0..3000 {
            line.push_str(["⏫", " 🔽", " #t", " 📅 2026-12-01", "⏬\u{fe0f}"][index % 5]);
        }
        let read = read(&line);
        assert_eq!(read.tokens().len(), 3000);
        assert_eq!(read.description_text(), "a");
        assert_eq!(read.priority().unwrap().level, Priority::Lowest);
        assert_eq!(read.set_priority(None).matches('#').count(), 600);
        assert_eq!(
            read.set_priority(Some(Priority::Low)).matches('🔽').count(),
            1
        );
    }

    #[test]
    fn a_due_date_is_changed_in_place_or_written_in_order() {
        let due = day("2026-12-05");
        check_writes(
            &[
                // A line ordered by hand: only the date's bytes change.
                (
                    "- [ ] a 📅 2026-12-01 ⏫ @sam #x ^id",
                    "- [ ] a 📅 2026-12-05 ⏫ @sam #x ^id",
                ),
                ("- [ ] a 📅 2026-12-01 @sam", "- [ ] a 📅 2026-12-05 @sam"),
                (
                    "- [x] a ✅ 2026-11-19 📅   2026-12-01 [k:: v]",
                    "- [x] a ✅ 2026-11-19 📅   2026-12-05 [k:: v]",
                ),
                // An alias and a field stay what they are.
                ("- [ ] a 🗓 2026-12-01", "- [ ] a 🗓 2026-12-05"),
                (
                    "- [ ] a 🗓\u{fe0f} 2026-12-01 ⏫",
                    "- [ ] a 🗓\u{fe0f} 2026-12-05 ⏫",
                ),
                ("- [ ] a 📆  2026-12-01 ^b", "- [ ] a 📆  2026-12-05 ^b"),
                (
                    "- [ ] a [due:: 2026-12-01] ⏫",
                    "- [ ] a [due:: 2026-12-05] ⏫",
                ),
                ("- [ ] a [ due::2026-12-01 ]", "- [ ] a [ due::2026-12-05 ]"),
                (
                    "- [ ] a 📅\u{fe0f}2026-02-30",
                    "- [ ] a 📅\u{fe0f}2026-12-05",
                ),
                // Of several, the last is the one read and the one
                // changed; the others stay as they were written.
                (
                    "- [ ] a 📅 2026-12-01 ⏫ 📆 2026-12-02",
                    "- [ ] a 📅 2026-12-01 ⏫ 📆 2026-12-05",
                ),
                (
                    "- [ ] 🗓 2026-12-01 [due:: 2026-12-02] 📅 2026-12-03 ^b",
                    "- [ ] 🗓 2026-12-01 [due:: 2026-12-02] 📅 2026-12-05 ^b",
                ),
                (
                    "- [ ] Pay 📅 2026-12-01 📆 2026-12-05",
                    "- [ ] Pay 📅 2026-12-01 📆 2026-12-05",
                ),
                // A new one: after what Obsidian Tasks writes before a due
                // date, before what it writes after.
                ("- [ ] a", "- [ ] a 📅 2026-12-05"),
                ("- [ ] a ^b", "- [ ] a 📅 2026-12-05 ^b"),
                ("- [ ] a ⏫", "- [ ] a ⏫ 📅 2026-12-05"),
                // With a space on each side, where the next token had none.
                (
                    "- [ ] a ⏫✅ 2026-11-19",
                    "- [ ] a ⏫ 📅 2026-12-05 ✅ 2026-11-19",
                ),
                (
                    "- [ ] a✅ 2026-11-19",
                    "- [ ] a 📅 2026-12-05 ✅ 2026-11-19",
                ),
                (
                    "- [x] a @sam ✅ 2026-11-19",
                    "- [x] a @sam 📅 2026-12-05 ✅ 2026-11-19",
                ),
                (
                    "- [-] a ❌ 2026-11-18 ✅ 2026-11-19 ^b",
                    "- [-] a 📅 2026-12-05 ❌ 2026-11-18 ✅ 2026-11-19 ^b",
                ),
                (
                    "- [ ] a 🔁 every week ⏳ 2026-11-30 ✅ 2026-11-19",
                    "- [ ] a 🔁 every week ⏳ 2026-11-30 📅 2026-12-05 ✅ 2026-11-19",
                ),
                ("> 1. [ ] a @sam ^b", "> 1. [ ] a @sam 📅 2026-12-05 ^b"),
                // As late as the order allows: after a field and a tag
                // that follow what comes before a due date,
                (
                    "- [ ] a 🔁 every week [key:: value] #t",
                    "- [ ] a 🔁 every week [key:: value] #t 📅 2026-12-05",
                ),
                // and before a stamp, whatever stands after the stamp.
                (
                    "- [x] a ✅ 2026-11-19 @sam ^b",
                    "- [x] a 📅 2026-12-05 ✅ 2026-11-19 @sam ^b",
                ),
                (
                    "- [x] a ⏫ #t ✅ 2026-11-19 [k:: v]",
                    "- [x] a ⏫ #t 📅 2026-12-05 ✅ 2026-11-19 [k:: v]",
                ),
                // A date with text after it is description, so this is new.
                (
                    "- [ ] pay 📅 2026-12-01 rent",
                    "- [ ] pay 📅 2026-12-01 rent 📅 2026-12-05",
                ),
                // Nothing after the box, and white space at the end.
                ("- [ ]", "- [ ] 📅 2026-12-05"),
                ("- [ ] ", "- [ ] 📅 2026-12-05"),
                ("- [ ]\t", "- [ ]\t📅 2026-12-05"),
                ("- [ ] ✅ 2026-11-19", "- [ ] 📅 2026-12-05 ✅ 2026-11-19"),
                ("- [ ] a  ", "- [ ] a 📅 2026-12-05  "),
                ("- [ ] a   ^b", "- [ ] a 📅 2026-12-05   ^b"),
            ],
            |line| line.set_date(DateField::Due, Some(due)),
        );
    }

    #[test]
    fn clearing_a_date_takes_its_token_and_the_space_before_it() {
        check_writes(
            &[
                (
                    "- [ ] a 📅 2026-12-01 ⏫ @sam #x ^id",
                    "- [ ] a ⏫ @sam #x ^id",
                ),
                ("- [ ] a 📅 2026-12-01", "- [ ] a"),
                ("- [ ] a 📅 2026-12-01  ", "- [ ] a  "),
                ("- [ ] a  📅 2026-12-01  ^b", "- [ ] a  ^b"),
                (
                    "- [ ] a\t🗓\u{fe0f} 2026-12-01 ✅ 2026-11-19",
                    "- [ ] a ✅ 2026-11-19",
                ),
                (
                    "- [ ] a [due:: 2026-12-01] ⏫ 📆 2026-12-02 #t 📅 2026-02-30",
                    "- [ ] a ⏫ #t",
                ),
                // The first thing after the box takes the space after it.
                ("- [ ] 📅 2026-12-01 ⏫", "- [ ] ⏫"),
                ("- [ ] 📅 2026-12-01", "- [ ] "),
                ("- [ ]\t📅 2026-12-01 \t", "- [ ]\t"),
                ("- [ ] 📅 2026-12-01 📆 2026-12-02 ^b", "- [ ] ^b"),
                // A token the next one touches leaves it its space.
                (
                    "- [ ] a ⏫ 📅 2026-12-01🔁 every day",
                    "- [ ] a ⏫ 🔁 every day",
                ),
                ("- [ ] a ⏫📅 2026-12-01", "- [ ] a ⏫"),
                ("- [ ] a📅 2026-12-01", "- [ ] a"),
                // And where it has none, it gets one: `#x🆔7` is a tag.
                ("- [ ] a #x📅 2026-12-01🆔7", "- [ ] a #x 🆔7"),
                (
                    "- [ ] a⏫📆 2026-12-01📅 2026-12-02🔁 every day",
                    "- [ ] a⏫ 🔁 every day",
                ),
                // Nothing to clear.
                ("- [ ] a ⏫ ^b", "- [ ] a ⏫ ^b"),
                (
                    "- [ ] pay 📅 2026-12-01 rent",
                    "- [ ] pay 📅 2026-12-01 rent",
                ),
            ],
            |line| line.set_date(DateField::Due, None),
        );
    }

    #[test]
    fn stamps_are_written_last_and_before_the_block_id() {
        let today = day("2026-11-19");
        check_writes(
            &[
                ("- [x] a", "- [x] a ✅ 2026-11-19"),
                (
                    "- [x] a 📅 2026-12-01 ^b",
                    "- [x] a 📅 2026-12-01 ✅ 2026-11-19 ^b",
                ),
                (
                    "- [x] Send the deck to finance @sam ⏫ 📅 2026-12-01",
                    "- [x] Send the deck to finance @sam ⏫ 📅 2026-12-01 ✅ 2026-11-19",
                ),
                (
                    "- [x] a ❌ 2026-11-18",
                    "- [x] a ❌ 2026-11-18 ✅ 2026-11-19",
                ),
                (
                    "- [x] a 🔁 every week [key:: value] ^b",
                    "- [x] a 🔁 every week [key:: value] ✅ 2026-11-19 ^b",
                ),
                // One that is there is re-dated where it stands.
                ("- [x] a ✅ 2026-11-01", "- [x] a ✅ 2026-11-19"),
                (
                    "- [x] a ✅\u{fe0f} 2026-11-01 📅 2026-12-01 ⏫",
                    "- [x] a ✅\u{fe0f} 2026-11-19 📅 2026-12-01 ⏫",
                ),
                (
                    "- [x] a ✅ 2026-11-01 📅 2026-12-01 ✅ 2026-11-02",
                    "- [x] a ✅ 2026-11-01 📅 2026-12-01 ✅ 2026-11-19",
                ),
            ],
            |line| line.set_date(DateField::Done, Some(today)),
        );
        check_writes(
            &[
                ("- [-] a", "- [-] a ❌ 2026-11-19"),
                (
                    "- [-] a 📅 2026-12-01 ✅ 2026-11-17 ^b",
                    "- [-] a 📅 2026-12-01 ❌ 2026-11-19 ✅ 2026-11-17 ^b",
                ),
                ("- [-] a ❌ 2026-11-01 ⏫", "- [-] a ❌ 2026-11-19 ⏫"),
            ],
            |line| line.set_date(DateField::Cancelled, Some(today)),
        );
        check_writes(
            &[
                (
                    "- [ ] a 📅 2026-12-01 ✅ 2026-11-19 ^b",
                    "- [ ] a 📅 2026-12-01 ^b",
                ),
                ("- [ ] a ✅ 2026-11-18 ✅ 2026-11-19", "- [ ] a"),
                (
                    "- [ ] a ❌ 2026-11-18 ✅ 2026-11-19",
                    "- [ ] a ❌ 2026-11-18",
                ),
                ("- [ ] a 📅 2026-12-01", "- [ ] a 📅 2026-12-01"),
            ],
            |line| line.set_date(DateField::Done, None),
        );
        check_writes(
            &[(
                "- [ ] a ❌ 2026-11-18 ✅ 2026-11-19",
                "- [ ] a ✅ 2026-11-19",
            )],
            |line| line.set_date(DateField::Cancelled, None),
        );
    }

    #[test]
    fn a_priority_replaces_every_indicator() {
        check_writes(
            &[
                // New: before the dates, after an id.
                ("- [ ] a", "- [ ] a ⏫"),
                (
                    "- [ ] a @sam 📅 2026-12-01 ^b",
                    "- [ ] a @sam ⏫ 📅 2026-12-01 ^b",
                ),
                (
                    "- [ ] a 🆔 x 🔁 every week 📅 2026-12-01",
                    "- [ ] a 🆔 x ⏫ 🔁 every week 📅 2026-12-01",
                ),
                (
                    "- [ ] a ✅ 2026-11-19 📅 2026-12-01",
                    "- [ ] a ⏫ ✅ 2026-11-19 📅 2026-12-01",
                ),
                // A tag, a field or an owner after the dates is looked
                // past, and stays where it is.
                ("- [ ] a 📅 2026-12-01 #x", "- [ ] a ⏫ 📅 2026-12-01 #x"),
                (
                    "- [ ] A 📅 2026-12-01 #home",
                    "- [ ] A ⏫ 📅 2026-12-01 #home",
                ),
                (
                    "- [ ] a 🆔 x [k:: v] 📅 2026-12-01 @sam ✅ 2026-11-19 #t ^b",
                    "- [ ] a 🆔 x [k:: v] ⏫ 📅 2026-12-01 @sam ✅ 2026-11-19 #t ^b",
                ),
                ("- [ ] a 🆔 x #t [k:: v]", "- [ ] a 🆔 x #t [k:: v] ⏫"),
                ("- [ ] ", "- [ ] ⏫"),
                ("- [ ] 📅 2026-12-01", "- [ ] ⏫ 📅 2026-12-01"),
                ("- [ ] Pay📅 2026-12-01", "- [ ] Pay ⏫ 📅 2026-12-01"),
                ("- [ ] a 🆔 x📅 2026-12-01", "- [ ] a 🆔 x ⏫ 📅 2026-12-01"),
                // In place.
                ("- [ ] a 🔽", "- [ ] a ⏫"),
                // The one indicator is already this level: nothing changes.
                ("- [ ] a ⏫ ^b", "- [ ] a ⏫ ^b"),
                ("- [ ] a ⏫\u{fe0f}", "- [ ] a ⏫\u{fe0f}"),
                // With another before it, setting the level the line is
                // read as having still leaves one indicator.
                ("- [ ] A 🔽 ⏫", "- [ ] A ⏫"),
                ("- [ ] a 🔽 📅 2026-12-01 ⏫", "- [ ] a 📅 2026-12-01 ⏫"),
                ("- [ ] a ⏫ ⏫\u{fe0f} ^b", "- [ ] a ⏫\u{fe0f} ^b"),
                // Another level is a new glyph: its U+FE0F goes too.
                (
                    "- [ ] a 🔽\u{fe0f} 📅 2026-12-01",
                    "- [ ] a ⏫ 📅 2026-12-01",
                ),
                (
                    "- [ ] a 📅 2026-12-01 🔺 @sam [k:: v]",
                    "- [ ] a 📅 2026-12-01 ⏫ @sam [k:: v]",
                ),
                // Several, and another level: the last becomes the new
                // one, the others go.
                (
                    "- [ ] a 🔺 📅 2026-12-01 🔽 ⏬ ^b",
                    "- [ ] a 📅 2026-12-01 ⏫ ^b",
                ),
                ("- [ ] ⏫ ⏫\u{fe0f} 🔼", "- [ ] ⏫"),
                ("- [ ] a⏫🔽", "- [ ] a ⏫"),
                // A glyph inside the description is text.
                ("- [ ] a 🔽 b", "- [ ] a 🔽 b ⏫"),
            ],
            |line| line.set_priority(Some(Priority::High)),
        );
        check_writes(
            &[
                ("- [ ] a ⏫ 🔽 📅 2026-12-01", "- [ ] a 📅 2026-12-01"),
                ("- [ ] a ⏫\u{fe0f} ^b", "- [ ] a ^b"),
                ("- [ ] ⏫ 🔼", "- [ ] "),
                ("- [ ] ⏫ 📅 2026-12-01 🔼 #t", "- [ ] 📅 2026-12-01 #t"),
                ("- [ ] a⏫", "- [ ] a"),
                ("- [ ] a ⏫📅 2026-12-01", "- [ ] a 📅 2026-12-01"),
                ("- [ ] a⏫🔼📅 2026-12-01", "- [ ] a 📅 2026-12-01"),
                ("- [ ] ⏫📅 2026-12-01", "- [ ] 📅 2026-12-01"),
                ("- [ ] a ⏫ 🔼📅 2026-12-01", "- [ ] a 📅 2026-12-01"),
                ("- [ ] a 🔽 b", "- [ ] a 🔽 b"),
                ("- [ ] a 📅 2026-12-01", "- [ ] a 📅 2026-12-01"),
            ],
            |line| line.set_priority(None),
        );
        for level in Priority::ALL {
            let line = read("- [ ] a 📅 2026-12-01").set_priority(Some(level));
            assert_eq!(line, format!("- [ ] a {} 📅 2026-12-01", level.glyph()));
            assert_eq!(read(&line).priority().unwrap().level, level);
        }
    }

    #[test]
    fn the_symbol_changes_alone() {
        for (before, symbol, after) in [
            ("- [ ] a 📅 2026-12-01", 'x', "- [x] a 📅 2026-12-01"),
            ("- [x] a ✅ 2026-11-19", ' ', "- [ ] a ✅ 2026-11-19"),
            ("- [x] a", '界', "- [界] a"),
            ("- [界] a ⏫", '/', "- [/] a ⏫"),
            ("> 1. [/]\ta", '-', "> 1. [-]\ta"),
            ("- [ ]", '?', "- [?]"),
            ("- [ ] a", '[', "- [[] a"),
        ] {
            let line = read(before).set_symbol(symbol);
            assert_eq!(line.as_deref(), Some(after), "{before:?}");
            assert_eq!(read(after).task().symbol, symbol);
        }
        for symbol in [']', '\n', '\r'] {
            assert_eq!(read("- [ ] a").set_symbol(symbol), None, "{symbol:?}");
        }
    }

    #[test]
    fn the_description_is_replaced_where_it_stands() {
        for (before, text, after) in [
            (
                "- [ ] a @sam 📅 2026-12-01 ^b",
                "Call the bank @sam @priya",
                "- [ ] Call the bank @sam @priya 📅 2026-12-01 ^b",
            ),
            (
                "- [ ] a 📅 2026-12-01 ⏫ @sam #x ^id",
                "b",
                "- [ ] b 📅 2026-12-01 ⏫ @sam #x ^id",
            ),
            ("-\t[ ]\ta  ⏫", "b c", "-\t[ ]\tb c  ⏫"),
            // A token that touched the old description gets a space.
            ("- [ ] a⏫", "b", "- [ ] b ⏫"),
            ("- [ ] a🆔7", "b #home", "- [ ] b #home 🆔7"),
            ("- [ ] a⏫", "a", "- [ ] a⏫"),
            ("- [ ] a", " b\nc\r\n ", "- [ ] b c"),
            // A field is a token whatever its value holds.
            (
                "- [ ] Pay 📅 2026-12-01 [area:: #home] ^b",
                "Call",
                "- [ ] Call 📅 2026-12-01 [area:: #home] ^b",
            ),
            (
                "- [ ] Pay 📅 2026-12-01 [project:: [[Roadmap]]]",
                "Call",
                "- [ ] Call 📅 2026-12-01 [project:: [[Roadmap]]]",
            ),
            // None before.
            ("- [ ] 📅 2026-12-01", "a", "- [ ] a 📅 2026-12-01"),
            ("- [ ]", "a", "- [ ] a"),
            ("- [ ] ", "a", "- [ ] a"),
            ("- [ ]", "", "- [ ]"),
            // None after.
            ("- [ ] a 📅 2026-12-01", "", "- [ ] 📅 2026-12-01"),
            ("- [ ] a @sam #x  ⏫", " ", "- [ ] ⏫"),
            ("- [ ] a", "", "- [ ] "),
            ("- [ ] a  ", "", "- [ ] "),
        ] {
            assert_eq!(read(before).set_description(text), after, "{before:?}");
        }
    }

    #[test]
    fn an_owner_is_added_to_the_description_and_removed_from_anywhere() {
        for (before, name, after) in [
            ("- [ ] a 📅 2026-12-01", "sam", "- [ ] a @sam 📅 2026-12-01"),
            ("- [ ] a @sam", "priya", "- [ ] a @sam @priya"),
            ("- [ ] a #x ⏫ ^b", "josé", "- [ ] a #x @josé ⏫ ^b"),
            ("- [ ] 📅 2026-12-01", "sam", "- [ ] @sam 📅 2026-12-01"),
            ("- [ ]", "sam", "- [ ] @sam"),
            ("- [ ] a  ", "jane.doe", "- [ ] a @jane.doe  "),
            (
                "- [ ] Pay📅 2026-12-01",
                "sam",
                "- [ ] Pay @sam 📅 2026-12-01",
            ),
            // Already an owner, wherever it is.
            ("- [ ] a @sam", "sam", "- [ ] a @sam"),
            (
                "- [ ] a 📅 2026-12-01 @sam",
                "sam",
                "- [ ] a 📅 2026-12-01 @sam",
            ),
            ("- [ ] a @Sam", "sam", "- [ ] a @Sam @sam"),
        ] {
            let line = read(before).add_owner(name);
            assert_eq!(line.as_deref(), Some(after), "{before:?}");
            assert!(read(after).owner_names().any(|owner| owner == name));
        }
        for name in [
            "", "1abc", "sam.", "a b", "@sam", "a@b", "_x", "sam\n", " sam",
        ] {
            assert!(!is_owner_name(name), "{name:?}");
            assert_eq!(read("- [ ] a").add_owner(name), None, "{name:?}");
        }
        for name in ["sam", "jane.doe", "j-smith_2", "josé", "x9", "日本"] {
            assert!(is_owner_name(name), "{name:?}");
        }
        for (before, name, after) in [
            (
                "- [ ] ask @sam about it 📅 2026-12-01",
                "sam",
                "- [ ] ask about it 📅 2026-12-01",
            ),
            ("- [ ] @sam call", "sam", "- [ ] call"),
            ("- [ ] @sam", "sam", "- [ ] "),
            (
                "- [ ] a @sam @priya 📅 2026-12-01",
                "sam",
                "- [ ] a @priya 📅 2026-12-01",
            ),
            (
                "- [ ] a 📅 2026-12-01 @sam #x",
                "sam",
                "- [ ] a 📅 2026-12-01 #x",
            ),
            ("- [ ] @sam a @sam ⏫ @sam", "sam", "- [ ] a ⏫"),
            ("- [ ] ask @sam.", "sam", "- [ ] ask."),
            ("- [ ] ask @sam, then go", "sam", "- [ ] ask, then go"),
            // A token that touches the owner keeps the space before it.
            ("- [ ] a #x @sam🆔7", "sam", "- [ ] a #x 🆔7"),
            (
                "- [ ] a ⏫ @sam📅 2026-12-01",
                "sam",
                "- [ ] a ⏫ 📅 2026-12-01",
            ),
            ("- [ ] a @Sam @sam", "sam", "- [ ] a @Sam"),
            ("- [ ] a @sam", "priya", "- [ ] a @sam"),
            ("- [ ] a @sam", "", "- [ ] a @sam"),
            ("- [ ] ask @sam: when?", "sam", "- [ ] ask: when?"),
            ("- [ ] (ask @sam)", "sam", "- [ ] (ask)"),
            // What is no closing punctuation keeps its space, or it would
            // join the word before.
            ("- [ ] mail @sam@work.com", "sam", "- [ ] mail @work.com"),
            ("- [ ] see @sam/notes", "sam", "- [ ] see /notes"),
            ("- [ ] see @sam(x)", "sam", "- [ ] see (x)"),
            // Punctuation closes the word before only when the word ends
            // there: a path and an embed keep their space.
            ("- [ ] open @sam./notes", "sam", "- [ ] open ./notes"),
            ("- [ ] See @sam![[Map]]", "sam", "- [ ] See ![[Map]]"),
            ("- [ ] ask @sam's team", "sam", "- [ ] ask 's team"),
            ("- [ ] ask @sam.) now", "sam", "- [ ] ask.) now"),
            ("- [ ] ask @sam?", "sam", "- [ ] ask?"),
            ("- [ ] ask @sam, @priya", "sam", "- [ ] ask, @priya"),
            // White space ends a name whatever cluster it is in.
            ("- [ ] call @a\u{d4e} Bob", "a\u{d4e}", "- [ ] call Bob"),
            // Only what was read as an owner goes: the second `@sam` here
            // was not one, and is one now.
            ("- [ ] @sam@sam z", "sam", "- [ ] @sam z"),
        ] {
            assert_eq!(read(before).remove_owner(name), after, "{before:?}");
        }
    }

    /// A `^id` is the block id only at the very end, as in Obsidian, so
    /// taking the last token off a line can make one of a `^id` before it,
    /// and tokens of what stood before that. They were description when
    /// the line was read, and clearing leaves description alone.
    #[test]
    fn clearing_takes_only_what_was_read_as_a_token() {
        check_reads(&[
            ("- [ ] a 📅 2026-12-01 ^x ⏫", "<a 📅 2026-12-01 ^x> high"),
            (
                "- [ ] a 📅 2026-12-01 ^x",
                "<a> due=2026-12-01(📅) block=^x",
            ),
            ("- [ ] a 🔽 ^x ⏫", "<a 🔽 ^x> high"),
            ("- [ ] a 🔽 ^x", "<a> low block=^x"),
        ]);
        check_writes(
            &[
                ("- [ ] a 📅 2026-12-01 ^x ⏫", "- [ ] a 📅 2026-12-01 ^x"),
                ("- [ ] a 🔽 ^x ⏫", "- [ ] a 🔽 ^x"),
                ("- [ ] a 🔽 ^x @sam ⏫ 🔼", "- [ ] a 🔽 ^x @sam"),
                ("- [ ] a 🔽 b ⏫", "- [ ] a 🔽 b"),
            ],
            |line| line.set_priority(None),
        );
        check_writes(
            &[
                (
                    "- [ ] a ✅ 2026-11-18 ^x ✅ 2026-11-19",
                    "- [ ] a ✅ 2026-11-18 ^x",
                ),
                ("- [ ] a ✅ 2026-11-18 ^x ⏫", "- [ ] a ✅ 2026-11-18 ^x ⏫"),
            ],
            |line| line.set_date(DateField::Done, None),
        );
        check_writes(&[("- [ ] a @sam ^x @sam", "- [ ] a ^x")], |line| {
            line.remove_owner("sam")
        });
    }

    /// A glyph that a zero-width joiner ties to the emoji before it is
    /// part of that emoji, and no token: taking it out would leave half a
    /// character.
    #[test]
    fn a_glyph_inside_a_joined_emoji_is_no_token() {
        check_reads(&[
            ("- [ ] draw 👩\u{200d}🔺", "<draw 👩\u{200d}🔺>"),
            (
                "- [ ] draw 👩\u{200d}🔺\u{fe0f}",
                "<draw 👩\u{200d}🔺\u{fe0f}>",
            ),
            ("- [ ] draw 👩\u{200d}⏫ 🔽", "<draw 👩\u{200d}⏫> low"),
            (
                "- [ ] draw 👩\u{200d}📅 2026-12-01",
                "<draw 👩\u{200d}📅 2026-12-01>",
            ),
            (
                "- [ ] draw 👩\u{200d}✅ 2026-11-19 ⏫",
                "<draw 👩\u{200d}✅ 2026-11-19> high",
            ),
            // A glyph after one that is whole starts its own.
            ("- [ ] draw 👩🔺", "<draw 👩> highest"),
            ("- [ ] draw ⏫\u{fe0f}🔺", "<draw> high highest"),
        ]);
        check_writes(
            &[
                ("- [ ] draw 👩\u{200d}🔺", "- [ ] draw 👩\u{200d}🔺"),
                ("- [ ] draw 👩\u{200d}⏫ 🔽", "- [ ] draw 👩\u{200d}⏫"),
            ],
            |line| line.set_priority(None),
        );
        check_writes(
            &[("- [ ] draw 👩\u{200d}🔺", "- [ ] draw 👩\u{200d}🔺 ⏫")],
            |line| line.set_priority(Some(Priority::High)),
        );
    }

    /// The glyphs by code point, so an editor that rewrites an emoji in
    /// this file cannot change what is read.
    #[test]
    fn glyphs_are_read_by_code_point() {
        for (glyph, expected) in [
            ('\u{1f4c5}', "due=2026-12-01(\u{1f4c5})"),
            ('\u{1f4c6}', "due=2026-12-01(\u{1f4c6})"),
            ('\u{1f5d3}', "due=2026-12-01(\u{1f5d3})"),
            ('\u{2705}', "done=2026-12-01(\u{2705})"),
            ('\u{274c}', "cancelled=2026-12-01(\u{274c})"),
            ('\u{2795}', "created[\u{2795} 2026-12-01]"),
            ('\u{1f6eb}', "start[\u{1f6eb} 2026-12-01]"),
            ('\u{23f3}', "scheduled[\u{23f3} 2026-12-01]"),
            ('\u{231b}', "scheduled[\u{231b} 2026-12-01]"),
        ] {
            let line = format!("- [ ] a {glyph} 2026-12-01");
            assert_eq!(summary(&line), format!("<a> {expected}"), "{glyph}");
        }
        check_reads(&[
            ("- [ ] a \u{1f194} x", "<a> id=x"),
            ("- [ ] a \u{26d4} x", "<a> dependson[\u{26d4} x]"),
            ("- [ ] a \u{1f501} x", "<a> recurrence[\u{1f501} x]"),
            ("- [ ] a \u{1f3c1} x", "<a> oncompletion[\u{1f3c1} x]"),
            ("- [ ] a \u{1f53a}", "<a> highest"),
            ("- [ ] a \u{23eb}", "<a> high"),
            ("- [ ] a \u{1f53c}", "<a> medium"),
            ("- [ ] a \u{1f53d}", "<a> low"),
            ("- [ ] a \u{23ec}", "<a> lowest"),
        ]);
        assert_eq!(DateField::Due.glyph(), '\u{1f4c5}');
        assert_eq!(DateField::Done.glyph(), '\u{2705}');
        assert_eq!(DateField::Cancelled.glyph(), '\u{274c}');
    }

    /// One change after another, each on the line the last one made.
    fn chain(line: &str, changes: &[&dyn Fn(&TaskLine) -> String]) -> String {
        changes
            .iter()
            .fold(line.to_owned(), |line, change| change(&read(&line)))
    }

    /// Obsidian Tasks writes the description, `🆔`, `⛔`, the priority,
    /// `🔁`, `🏁`, the created, start, scheduled, due, cancelled and done
    /// dates, then the block id. Whatever order Wren's tokens are added
    /// in, they land in theirs.
    #[test]
    fn new_tokens_land_in_obsidian_tasks_order() {
        let priority = |line: &TaskLine| line.set_priority(Some(Priority::High));
        let due = |line: &TaskLine| line.set_date(DateField::Due, Some(day("2026-12-01")));
        let cancelled =
            |line: &TaskLine| line.set_date(DateField::Cancelled, Some(day("2026-11-18")));
        let done = |line: &TaskLine| line.set_date(DateField::Done, Some(day("2026-11-19")));
        let changes: [&dyn Fn(&TaskLine) -> String; 4] = [&priority, &due, &cancelled, &done];
        let mut orders = 0;
        for a in 0..4 {
            for b in (0..4).filter(|b| *b != a) {
                for c in (0..4).filter(|c| *c != a && *c != b) {
                    let d = 6 - a - b - c;
                    let order = [changes[a], changes[b], changes[c], changes[d]];
                    assert_eq!(
                        chain("- [ ] a @sam", &order),
                        "- [ ] a @sam ⏫ 📅 2026-12-01 ❌ 2026-11-18 ✅ 2026-11-19",
                        "{a}{b}{c}{d}"
                    );
                    assert_eq!(
                        chain(
                            "- [ ] a 🆔 x1 ⛔ y,z 🔁 every week 🏁 delete ➕ 2026-01-01 \
                             🛫 2026-01-02 ⏳ 2026-01-03 ^b1",
                            &order
                        ),
                        "- [ ] a 🆔 x1 ⛔ y,z ⏫ 🔁 every week 🏁 delete ➕ 2026-01-01 \
                         🛫 2026-01-02 ⏳ 2026-01-03 📅 2026-12-01 ❌ 2026-11-18 ✅ 2026-11-19 ^b1",
                        "{a}{b}{c}{d}"
                    );
                    orders += 1;
                }
            }
        }
        assert_eq!(orders, 24);
    }

    /// What Wren writes is what Obsidian Tasks writes: each glyph as one
    /// character with no U+FE0F after it, and one space before a date.
    #[test]
    fn written_glyphs_have_no_variation_selector() {
        let line = chain(
            "- [ ] a",
            &[
                &|line| line.set_priority(Some(Priority::Highest)),
                &|line| line.set_date(DateField::Due, Some(day("2026-12-01"))),
                &|line| line.set_date(DateField::Cancelled, Some(day("2026-11-18"))),
                &|line| line.set_date(DateField::Done, Some(day("2026-11-19"))),
            ],
        );
        assert_eq!(
            line,
            "- [ ] a \u{1f53a} \u{1f4c5} 2026-12-01 \u{274c} 2026-11-18 \u{2705} 2026-11-19"
        );
        let glyphs: String = Priority::ALL.into_iter().map(Priority::glyph).collect();
        assert_eq!(glyphs, "\u{1f53a}\u{23eb}\u{1f53c}\u{1f53d}\u{23ec}");
        for level in Priority::ALL {
            assert_eq!(Priority::from_glyph(level.glyph()), Some(level));
        }
        assert_eq!(Priority::from_glyph('📅'), None);
        assert_eq!(Priority::from_glyph('\u{fe0f}'), None);
    }

    /// SplitMix64, seeded, so a failure replays.
    pub(super) struct Rng(pub(super) u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        pub(super) fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }

        fn one_in(&mut self, n: usize) -> bool {
            self.below(n) == 0
        }

        pub(super) fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            items[self.below(items.len())]
        }
    }

    const DATES: [&str; 7] = [
        "2026-12-01",
        "2026-11-19",
        "2024-02-29",
        "9999-12-31",
        "2026-02-30",
        "0000-00-00",
        "2026-13-01",
    ];

    const WORDS: &[&str] = &[
        "👩\u{200d}",
        "@sam@work.com",
        "@sam./notes",
        "@sam![[Map]]",
        "@sam,",
        "@sam.)",
        "@a\u{d4e}",
        "\u{d4e}",
        "[x]",
        "a",
        "Call",
        "the",
        "bank",
        "naïve",
        "日本語",
        "e\u{301}",
        "it's",
        "50%",
        ",",
        ".",
        "a@b.com",
        "(@sam)",
        "@sam",
        "@sam",
        "@priya",
        "@jane.doe",
        "@josé",
        "@sam.",
        "@j-smith_2",
        "@1abc",
        "@",
        "@sam@sam",
        "#",
        "#tag",
        "#a/b",
        "📅",
        "tomorrow",
        "2026-12-01",
        "⏫",
        "✅",
        "🔁",
        "🆔",
        "^x",
        "^",
        "[link](u)",
        "[[wiki]]",
        "[due::",
        "x]",
        "::",
        "\u{fe0f}",
        "🗓\u{fe0f}",
        "every",
        "week",
    ];

    fn random_token(rng: &mut Rng) -> String {
        let selector = if rng.one_in(4) { "\u{fe0f}" } else { "" };
        let gap = rng.pick(&[" ", " ", " ", "", "  "]);
        match rng.below(14) {
            0..=4 => format!(
                "{}{selector}{gap}{}",
                rng.pick(&DATE_GLYPHS),
                rng.pick(&DATES)
            ),
            5..=7 => format!("{}{selector}", rng.pick(&Priority::ALL).glyph()),
            8 => format!("🆔{selector}{gap}{}", rng.pick(&["abc", "x-1_y", "7"])),
            9 => format!(
                "⛔{selector}{gap}{}",
                rng.pick(&["abc", "a,b", "a, b-c ,d"])
            ),
            10 => format!(
                "🔁{selector}{gap}{}",
                rng.pick(&[
                    "every week",
                    "every 2 weeks on Monday, Friday",
                    "every day when done"
                ])
            ),
            11 => format!("🏁{selector}{gap}{}", rng.pick(&["delete", "keep"])),
            12 => rng.pick(&["#x", "#work/sub", "#a-b", "#日本"]).to_owned(),
            _ => rng
                .pick(&[
                    "[k:: v]",
                    "[due:: 2026-12-01]",
                    "[due::2026-02-30]",
                    "[ due::  2026-12-07 ]",
                    "[priority:: high]",
                    "[key::]",
                    "[due:: soon]",
                    "[area:: #home]",
                    "[project:: [[Roadmap]]]",
                ])
                .to_owned(),
        }
    }

    /// A task line of words then tokens, with the odd word among the
    /// tokens, every kind of spacing, and one time in four a character
    /// dropped, doubled or put in at random.
    pub(super) fn random_line(rng: &mut Rng) -> String {
        let mut line = rng
            .pick(&[
                "- ", "- ", "- ", "* ", "+ ", "1. ", "12) ", "> - ", "  - ", "-\t", "> > 3. ",
            ])
            .to_owned();
        line.push('[');
        line.push(rng.pick(&[' ', ' ', 'x', 'X', '/', '-', '?', '界', 'é', '✓']));
        line.push(']');
        let mut pieces: Vec<String> = (0..rng.below(4))
            .map(|_| rng.pick(WORDS).to_owned())
            .collect();
        for _ in 0..rng.below(7) {
            pieces.push(if rng.one_in(6) {
                rng.pick(WORDS).to_owned()
            } else {
                random_token(rng)
            });
        }
        if rng.one_in(3) {
            pieces.push(rng.pick(&["^abc", "^b-1", "^9"]).to_owned());
        }
        for (index, piece) in pieces.iter().enumerate() {
            // A box needs a space or a tab after it to be one.
            let gaps: &[&str] = if index == 0 {
                &[" ", " ", " ", "  ", "\t"]
            } else {
                &[" ", " ", " ", " ", " ", "  ", "\t", "\u{a0}", ""]
            };
            line.push_str(rng.pick(gaps));
            line.push_str(piece);
        }
        line.push_str(rng.pick(&["", "", "", " ", "  ", "\t"]));
        if rng.one_in(4) {
            let starts: Vec<usize> = line.char_indices().map(|(at, _)| at).collect();
            let at = rng.pick(&starts);
            let len = line[at..].chars().next().map_or(0, char::len_utf8);
            match rng.below(3) {
                0 => line.replace_range(at..at + len, ""),
                1 => {
                    let again = line[at..at + len].to_owned();
                    line.insert_str(at, &again);
                }
                _ => line.insert_str(
                    at,
                    rng.pick(&[
                        " ", "\t", "@", "#", "^", "[", "]", ":", ",", ".", "-", "1", "a", "é",
                        "📅", "⏫", "🆔", "\u{fe0f}", "\u{301}", "\u{200d}",
                    ]),
                ),
            }
        }
        line
    }

    /// The reading holds together: every range is on character boundaries
    /// and in order, the description and the tokens cover everything after
    /// the box but white space, and each token says what its text does.
    fn well_formed(read: &TaskLine) {
        let line = read.line();
        let context = format!("{line:?}");
        let description = read.description();
        let text = line.get(description.clone()).expect(&context);
        assert_eq!(text, text.trim(), "{context}");
        let mut at = read.task().end;
        let mut pieces = vec![description.clone()];
        pieces.extend(read.tokens().iter().map(Token::range));
        for (index, piece) in pieces.iter().enumerate() {
            assert!(at <= piece.start && piece.start <= piece.end, "{context}");
            let gap = line.get(at..piece.start).expect(&context);
            assert!(gap.chars().all(char::is_whitespace), "{context}");
            let text = line.get(piece.clone()).expect(&context);
            assert_eq!(text, text.trim(), "{context}");
            assert!(index == 0 || !text.is_empty(), "{context}");
            at = piece.end;
        }
        assert!(line[at..].chars().all(char::is_whitespace), "{context}");
        let block_ids = read
            .tokens()
            .iter()
            .filter(|token| matches!(token, Token::BlockId(_)))
            .count();
        assert_eq!(
            block_ids,
            usize::from(read.block_id().is_some()),
            "{context}"
        );
        assert!(
            !matches!(
                read.tokens().first(),
                Some(Token::Owner(_))
                    | Some(Token::Kept(KeptToken {
                        kind: Kept::Tag,
                        ..
                    }))
            ),
            "{context}"
        );
        for token in read.tokens() {
            match token {
                Token::Date(date) => {
                    assert_eq!(date.date.len(), ISO_LEN, "{context}");
                    assert!(
                        date.range.start < date.date.start && date.date.end <= date.range.end,
                        "{context}"
                    );
                    assert_eq!(
                        Parsed::from_iso(read.text(&date.date)),
                        Some(date.value),
                        "{context}"
                    );
                    let start = &line[date.range.start..];
                    match date.glyph {
                        Some(glyph) => assert!(start.starts_with(glyph), "{context}"),
                        None => assert!(start.starts_with('['), "{context}"),
                    }
                }
                Token::Priority(priority) => {
                    let text = read.text(&priority.range);
                    let glyph = priority.level.glyph();
                    assert!(
                        text == glyph.to_string() || text == format!("{glyph}\u{fe0f}"),
                        "{context}"
                    );
                }
                Token::Id(id) => {
                    assert!(!id.id.is_empty() && id.id.end == id.range.end, "{context}");
                    assert!(read.text(&id.id).chars().all(id_char), "{context}");
                }
                Token::Owner(range) => assert!(
                    read.owners().iter().any(|owner| owner.range == *range),
                    "{context}"
                ),
                Token::BlockId(_) | Token::Kept(_) => {}
            }
        }
        let mut last = 0;
        for owner in read.owners() {
            assert!(last <= owner.range.start, "{context}");
            last = owner.range.end;
            assert_eq!(
                owner.name,
                owner.range.start + 1..owner.range.end,
                "{context}"
            );
            assert_eq!(&line[owner.range.start..owner.name.start], "@", "{context}");
            assert!(is_owner_name(read.text(&owner.name)), "{context}");
            let inside =
                description.start <= owner.range.start && owner.range.end <= description.end;
            let token = read
                .tokens()
                .iter()
                .any(|token| matches!(token, Token::Owner(range) if *range == owner.range));
            assert!(inside != token, "{context}");
        }
    }

    /// A change the corpus makes to every line.
    #[derive(Clone, Debug)]
    enum Change {
        Date(DateField, Option<Date>),
        Priority(Option<Priority>),
        Symbol(char),
        Description(&'static str),
        AddOwner(&'static str),
        RemoveOwner(String),
    }

    impl Change {
        fn make(&self, line: &TaskLine) -> String {
            match self {
                Self::Date(field, date) => line.set_date(*field, *date),
                Self::Priority(level) => line.set_priority(*level),
                Self::Symbol(symbol) => line.set_symbol(*symbol).expect("a symbol"),
                Self::Description(text) => line.set_description(text),
                Self::AddOwner(name) => line.add_owner(name).expect("a name"),
                Self::RemoveOwner(name) => line.remove_owner(name),
            }
        }

        /// The replacements [`make`](Self::make) is made of.
        fn edits(&self, line: &TaskLine) -> Vec<Edit> {
            match self {
                Self::Date(field, date) => line.date_edits(*field, *date),
                Self::Priority(level) => line.priority_edits(*level),
                Self::Symbol(symbol) => vec![(line.task().symbol_range(), symbol.to_string())],
                Self::Description(text) => line.description_edits(text),
                Self::AddOwner(name) => line.owner_addition(name).expect("a name"),
                Self::RemoveOwner(name) => line.owner_removals(name),
            }
        }

        /// The bytes of `line` this change is about.
        fn subject(&self, line: &TaskLine) -> Vec<Range<usize>> {
            match self {
                Self::Date(field, _) => line.dates(*field).map(|date| date.range.clone()).collect(),
                Self::Priority(_) => line.priorities().map(|token| token.range.clone()).collect(),
                Self::Symbol(_) => vec![line.task().symbol_range()],
                Self::Description(_) => vec![line.description()],
                Self::AddOwner(name) => owner_ranges(line, name),
                Self::RemoveOwner(name) => owner_ranges(line, name),
            }
        }

        /// The text this change writes, when it writes any.
        fn written(&self) -> Vec<String> {
            match self {
                Self::Date(field, Some(date)) => {
                    vec![date.to_string(), format!("{} {date}", field.glyph())]
                }
                Self::Priority(Some(level)) => vec![level.glyph().to_string()],
                Self::Symbol(symbol) => vec![symbol.to_string()],
                Self::Description(text) => vec![(*text).to_owned()],
                Self::AddOwner(name) => vec![format!("@{name}")],
                Self::Date(_, None) | Self::Priority(None) | Self::RemoveOwner(_) => Vec::new(),
            }
        }
    }

    fn owner_ranges(line: &TaskLine, name: &str) -> Vec<Range<usize>> {
        line.owners()
            .iter()
            .filter(|owner| line.text(&owner.name) == name)
            .map(|owner| owner.range.clone())
            .collect()
    }

    /// The line without `cut` and without white space: what a change that
    /// is about `cut` must leave byte for byte, in order.
    fn rest(line: &str, cut: &[Range<usize>]) -> String {
        line.char_indices()
            .filter(|(at, c)| !c.is_whitespace() && !cut.iter().any(|range| range.contains(at)))
            .map(|(_, c)| c)
            .collect()
    }

    /// The text of every token Wren does not write, in line order. A tag
    /// is left out, because one that follows the description belongs to
    /// it, and so is the block id, which is one only at the very end of
    /// the line; [`survivors_hold`] follows the bytes of both.
    fn unknown(line: &TaskLine) -> Vec<String> {
        line.tokens()
            .iter()
            .filter(|token| match token {
                Token::Id(_) => true,
                Token::Kept(kept) => kept.kind != Kept::Tag,
                Token::BlockId(_) | Token::Date(_) | Token::Priority(_) | Token::Owner(_) => false,
            })
            .map(|token| line.text(&token.range()).to_owned())
            .collect()
    }

    fn texts<'a>(line: &TaskLine<'a>, ranges: impl Iterator<Item = Range<usize>>) -> Vec<&'a str> {
        ranges.map(|range| line.text(&range)).collect()
    }

    /// Whether the character before `at`, or after it, is white space or
    /// the end of the text.
    fn apart(text: &str, at: usize, before: bool) -> bool {
        let next = if before {
            text[..at].chars().next_back()
        } else {
            text[at..].chars().next()
        };
        next.is_none_or(char::is_whitespace)
    }

    /// What a change is not about survives it, followed byte by byte
    /// through the replacements the change is made of, whatever the new
    /// line is then read as:
    ///
    /// - a replacement takes out some of the change's subject, and besides
    ///   it only white space next to it, and puts in nothing but the new
    ///   value and a space; one that takes nothing out puts the new value
    ///   in;
    /// - no replacement reaches into another token (a tag and a block id
    ///   too) or into the description, taken as the stretches of it on
    ///   each side of an owner the change is about; the bytes of each are
    ///   found unchanged where the replacements before them move them to;
    /// - a token that had white space on a side still has, so nothing
    ///   comes to touch it and be read as part of it;
    /// - and two stretches of the description come to touch only where the
    ///   second starts with punctuation that closes a word.
    ///
    /// The replacements and the surviving pieces are walked together, each
    /// once.
    fn survivors_hold(
        before: &TaskLine,
        change: &Change,
        edits: &[Edit],
        out: &str,
        context: &str,
    ) {
        let line = before.line();
        let subject = change.subject(before);
        let written = change.written();
        let mut edits = edits.to_vec();
        edits.sort_by_key(|(range, _)| (range.start, range.end));

        let mut first = 0;
        for (range, text) in &edits {
            while subject
                .get(first)
                .is_some_and(|part| part.end <= range.start)
            {
                first += 1;
            }
            let (mut at, mut part, mut about) = (range.start, first, false);
            while at < range.end {
                match subject.get(part) {
                    Some(subject) if subject.start <= at => {
                        about = true;
                        at = subject.end;
                        part += 1;
                    }
                    next => {
                        let stop = next.map_or(range.end, |subject| subject.start.min(range.end));
                        let taken = &line[at..stop];
                        assert!(
                            taken.chars().all(char::is_whitespace),
                            "{taken:?} is taken out: {context}"
                        );
                        at = stop;
                    }
                }
            }
            let put = text.trim();
            assert!(
                put.is_empty() || written.iter().any(|new| new == put),
                "{text:?} is put in: {context}"
            );
            assert!(
                about || (range.is_empty() && !put.is_empty()),
                "{range:?} is about nothing: {context}"
            );
        }

        // The surviving pieces in line order, a token marked as one.
        let mut pieces: Vec<(Range<usize>, bool)> = Vec::new();
        let description = before.description();
        let mut from = description.start;
        let inside = subject
            .iter()
            .filter(|part| description.start <= part.start && part.end <= description.end);
        for stop in inside
            .map(|part| part.start..part.end)
            .chain(std::iter::once(description.end..description.end))
        {
            let text = &line[from..stop.start.max(from)];
            let start = from + text.len() - text.trim_start().len();
            pieces.push((start..from + text.trim_end().len(), false));
            from = stop.end;
        }
        let cut: HashSet<&Range<usize>> = subject.iter().collect();
        let tokens = before.tokens().iter().map(Token::range);
        pieces.extend(
            tokens
                .filter(|range| !cut.contains(range))
                .map(|range| (range, true)),
        );
        pieces.retain(|(piece, _)| !piece.is_empty());

        let mut next = 0;
        let mut moved: isize = 0;
        let mut stretch: Option<usize> = None;
        for (piece, token) in pieces {
            while let Some((range, text)) = edits.get(next).filter(|edit| edit.0.end <= piece.start)
            {
                moved += text.len() as isize - range.len() as isize;
                next += 1;
            }
            if let Some((range, _)) = edits.get(next) {
                assert!(
                    range.start >= piece.end,
                    "{range:?} reaches into {piece:?}: {context}"
                );
            }
            let start = piece.start.checked_add_signed(moved).expect(context);
            let end = start + piece.len();
            assert_eq!(out.get(start..end), Some(&line[piece.clone()]), "{context}");
            if token {
                for (was, is) in [
                    (apart(line, piece.start, true), apart(out, start, true)),
                    (apart(line, piece.end, false), apart(out, end, false)),
                ] {
                    assert!(is || !was, "{piece:?} is touched: {context}");
                }
            } else {
                if let Some(last) = stretch {
                    let between = &out[last..start];
                    assert!(between.chars().all(char::is_whitespace), "{context}");
                    let closing = ".,;:!?)]}'\"\u{2019}\u{201d}\u{2026}\u{bb}";
                    let word = line[piece.clone()].trim_start_matches(|c| closing.contains(c));
                    let closes = word.len() < piece.len() && apart(word, 0, false);
                    assert!(
                        !between.is_empty() || closes,
                        "{piece:?} is joined to the word before: {context}"
                    );
                }
                stretch = Some(end);
            }
        }
    }

    /// The check above is what catches a writer that damages what it was
    /// not asked to change: one that writes its token into another,
    #[test]
    #[should_panic(expected = "reaches into")]
    fn a_writer_that_writes_into_an_unknown_token_is_caught() {
        let line = "- [ ] a 📅 2026-12-01 #xy";
        let before = read(line);
        let change = Change::Priority(Some(Priority::High));
        let inside = line.find("#xy").unwrap() + 2;
        let edits = vec![(inside..inside, " ⏫ ".to_owned())];
        let out = before.apply(edits.clone());
        assert_eq!(out, "- [ ] a 📅 2026-12-01 #x ⏫ y");
        survivors_hold(&before, &change, &edits, &out, line);
    }

    /// one that splits a tag among the tokens,
    #[test]
    #[should_panic(expected = "is about nothing")]
    fn a_writer_that_splits_an_unknown_token_is_caught() {
        let line = "- [ ] a ⏫ #xy ^b";
        let before = read(line);
        let change = Change::Priority(None);
        let mut edits = change.edits(&before);
        let inside = line.find("#xy").unwrap() + 2;
        edits.push((inside..inside, " ".to_owned()));
        let out = before.apply(edits.clone());
        assert_eq!(out, "- [ ] a #x y ^b");
        survivors_hold(&before, &change, &edits, &out, line);
    }

    /// one that splits a tag in the description after the owner it takes
    /// out,
    #[test]
    #[should_panic(expected = "is about nothing")]
    fn a_writer_that_splits_the_description_is_caught() {
        let line = "- [ ] ping @sam #work";
        let before = read(line);
        let change = Change::RemoveOwner("sam".to_owned());
        let mut edits = change.edits(&before);
        let inside = line.find("#work").unwrap() + 3;
        edits.push((inside..inside, " ".to_owned()));
        let out = before.apply(edits.clone());
        assert_eq!(out, "- [ ] ping #wo rk");
        survivors_hold(&before, &change, &edits, &out, line);
    }

    /// one that takes white space from somewhere else in the description,
    #[test]
    #[should_panic(expected = "is about nothing")]
    fn a_writer_that_takes_white_space_elsewhere_is_caught() {
        let line = "- [ ] ping the bank @sam";
        let before = read(line);
        let change = Change::RemoveOwner("sam".to_owned());
        let mut edits = change.edits(&before);
        let space = line.find(" the").unwrap();
        edits.push((space..space + 1, String::new()));
        let out = before.apply(edits.clone());
        assert_eq!(out, "- [ ] pingthe bank");
        survivors_hold(&before, &change, &edits, &out, line);
    }

    /// one that joins two words of the description,
    #[test]
    #[should_panic(expected = "is joined to the word before")]
    fn a_writer_that_joins_two_words_is_caught() {
        let line = "- [ ] mail @sam@work.com";
        let before = read(line);
        let change = Change::RemoveOwner("sam".to_owned());
        let start = line.find(" @sam").unwrap();
        let edits = vec![(start..start + " @sam".len(), String::new())];
        let out = before.apply(edits.clone());
        assert_eq!(out, "- [ ] mail@work.com");
        survivors_hold(&before, &change, &edits, &out, line);
    }

    /// and one that leaves a token touching its neighbour.
    #[test]
    #[should_panic(expected = "is touched")]
    fn a_writer_that_joins_an_unknown_token_to_its_neighbour_is_caught() {
        let line = "- [ ] a #x ⏫ 🆔 7";
        let before = read(line);
        let change = Change::Priority(None);
        let start = line.find(" ⏫").unwrap();
        let edits = vec![(start..line.find("🆔").unwrap(), String::new())];
        let out = before.apply(edits.clone());
        assert_eq!(out, "- [ ] a #x🆔 7");
        survivors_hold(&before, &change, &edits, &out, line);
    }

    /// What every change must do, whatever the line.
    fn change_holds(before: &TaskLine, change: &Change) {
        let line = before.line();
        let out = change.make(before);
        let context = format!("{change:?} on {line:?} gave {out:?}");
        let edits = change.edits(before);
        assert_eq!(before.apply(edits.clone()), out, "{context}");
        survivors_hold(before, change, &edits, &out, &context);
        let task = find(&out).unwrap_or_else(|| panic!("no task: {context}"));
        assert_eq!(task.start, before.task().start, "{context}");
        let after = parse(&out, task).unwrap_or_else(|| panic!("not read: {context}"));
        well_formed(&after);
        let subject = change.subject(before);

        // What was taken out is not taken out of what the line held as
        // description, so two things can be left behind that are read
        // only now. A `^id` before the last token becomes the block id
        // once that token is gone, as it does in Obsidian, and what stood
        // before the `^id` is then read as tokens. And the `@sam` that
        // followed an owner with no space is an owner once it is gone.
        let revealed = before.block_id().is_none() && after.block_id().is_some();
        let uncovered = matches!(change, Change::RemoveOwner(_))
            && subject
                .iter()
                .any(|owner| line[owner.end..].starts_with('@'));

        // The change reads back, and making it again changes nothing.
        match change {
            Change::Date(field, Some(date)) => {
                let read = after.date(*field).map(|token| token.value);
                assert_eq!(read, Some(Parsed::Date(*date)), "{context}");
                let count = before.dates(*field).count().max(1);
                assert_eq!(after.dates(*field).count(), count, "{context}");
            }
            Change::Date(field, None) => {
                assert!(revealed || after.date(*field).is_none(), "{context}");
            }
            Change::Priority(Some(level)) => {
                let levels: Vec<Priority> = after.priorities().map(|token| token.level).collect();
                assert_eq!(levels, [*level], "{context}");
            }
            Change::Priority(None) => {
                assert!(revealed || after.priority().is_none(), "{context}");
            }
            Change::Symbol(symbol) => assert_eq!(after.task().symbol, *symbol, "{context}"),
            Change::Description(text) => assert_eq!(after.description_text(), *text, "{context}"),
            Change::AddOwner(name) => {
                assert!(after.owner_names().any(|owner| owner == *name), "{context}");
            }
            Change::RemoveOwner(name) => {
                assert!(
                    uncovered || revealed || after.owner_names().all(|owner| owner != name),
                    "{context}"
                );
            }
        }
        if !revealed && !uncovered {
            assert_eq!(change.make(&after), out, "not stable: {context}");
        }

        // Nothing else moves or changes: outside what the change is about,
        // the same bytes in the same order. (Where more of its kind is
        // read afterwards, `survivors_hold` has said so already.)
        if !revealed && !uncovered {
            assert_eq!(
                rest(line, &subject),
                rest(&out, &change.subject(&after)),
                "{context}"
            );
        }
        // Every other reading stays what it was, short of a block id
        // coming to light, and of an owner taken out of the description's
        // words, which can leave words that read as a token
        // (`🆔 @sam x1`).
        let reworded = matches!(change, Change::RemoveOwner(_))
            && subject
                .iter()
                .any(|owner| owner.start < before.description().end);
        if !revealed && !reworded {
            readings_hold(before, &after, change, &context);
        }

        // A token that is there is changed in place: the line is the old
        // one with the value's bytes replaced.
        let in_place = match change {
            Change::Date(field, Some(date)) => before
                .date(*field)
                .map(|token| (token.date.clone(), date.to_string())),
            // Another level replaces the whole indicator with a bare
            // glyph; the same level leaves it as written.
            Change::Priority(Some(level)) if subject.len() == 1 => before.priority().map(|token| {
                let text = if token.level == *level {
                    before.text(&token.range).to_owned()
                } else {
                    level.glyph().to_string()
                };
                (token.range.clone(), text)
            }),
            _ => None,
        };
        if let Some((range, text)) = in_place {
            let mut expected = line.to_owned();
            expected.replace_range(range, &text);
            assert_eq!(out, expected, "{context}");
        }
        // A token that is not there is put in, and nothing is taken out.
        if subject.is_empty() && matches!(change, Change::Date(..) | Change::Priority(_)) {
            assert!(out.len() >= line.len(), "{context}");
            assert_eq!(
                before.description_text(),
                after.description_text(),
                "{context}"
            );
        }
    }

    /// The readings a change is not about are what they were.
    fn readings_hold(before: &TaskLine, after: &TaskLine, change: &Change, context: &str) {
        assert_eq!(unknown(before), unknown(after), "{context}");
        assert_eq!(
            before.block_id().map(|range| before.text(&range)),
            after.block_id().map(|range| after.text(&range)),
            "{context}"
        );
        if !matches!(change, Change::Symbol(_)) {
            assert_eq!(before.task(), after.task(), "{context}");
        }
        for field in DateField::ALL {
            if !matches!(change, Change::Date(changed, _) if *changed == field) {
                assert_eq!(
                    texts(before, before.dates(field).map(|token| token.range.clone())),
                    texts(after, after.dates(field).map(|token| token.range.clone())),
                    "{context}"
                );
            }
        }
        if !matches!(change, Change::Priority(_)) {
            assert_eq!(
                texts(before, before.priorities().map(|token| token.range.clone())),
                texts(after, after.priorities().map(|token| token.range.clone())),
                "{context}"
            );
        }
        if matches!(
            change,
            Change::Date(..) | Change::Priority(_) | Change::Symbol(_)
        ) {
            assert_eq!(
                before.owner_names().collect::<Vec<_>>(),
                after.owner_names().collect::<Vec<_>>(),
                "{context}"
            );
            // A token taken out can leave an owner or a tag that followed
            // it to the description, which only grows.
            assert!(
                after
                    .description_text()
                    .starts_with(before.description_text()),
                "{context}"
            );
        }
    }

    /// Writing what the line already says changes nothing.
    fn unchanged_holds(read: &TaskLine) {
        let line = read.line();
        assert_eq!(read.set_symbol(read.task().symbol).as_deref(), Some(line));
        assert_eq!(read.set_description(read.description_text()), line);
        for field in DateField::ALL {
            match read.date(field).map(|token| token.value) {
                None => assert_eq!(read.set_date(field, None), line),
                Some(Parsed::Date(date)) => assert_eq!(read.set_date(field, Some(date)), line),
                Some(Parsed::Invalid) => {}
            }
        }
        if read.priorities().count() < 2 {
            let level = read.priority().map(|token| token.level);
            assert_eq!(read.set_priority(level), line);
        }
        for name in read.owner_names() {
            assert_eq!(read.add_owner(name).as_deref(), Some(line));
        }
        assert_eq!(read.remove_owner("nobody"), line);
    }

    /// Every property on one line; whether it was a task line.
    fn line_holds(line: &str, rng: &mut Rng) -> bool {
        let Some(task) = find(line) else {
            return false;
        };
        let read = parse(line, task).unwrap_or_else(|| panic!("{line:?} is not read"));
        well_formed(&read);
        unchanged_holds(&read);
        let date = Date::parse_iso(rng.pick(&DATES[..4]));
        let owner = read
            .owner_names()
            .next()
            .map_or("nobody".to_owned(), str::to_owned);
        let mut changes = vec![
            Change::Priority(Some(rng.pick(&Priority::ALL))),
            Change::Priority(None),
            Change::Symbol(rng.pick(&[' ', 'x', 'X', '/', '-', '界', 'é', '\t', '['])),
            Change::Description("new words @kim"),
            Change::Description("#home"),
            Change::Description(""),
            Change::AddOwner("lee"),
            Change::RemoveOwner(owner),
        ];
        for field in DateField::ALL {
            changes.push(Change::Date(field, date));
            changes.push(Change::Date(field, None));
        }
        for change in &changes {
            change_holds(&read, change);
        }
        true
    }

    /// Any text at all, with a box that may not be its own: nothing
    /// panics, and what is read holds together.
    fn any_text_holds(rng: &mut Rng) {
        const PIECES: [&str; 30] = [
            "- [ ] ",
            "- [x] ",
            "[",
            "]",
            " ",
            "  ",
            "\t",
            "\u{a0}",
            "@",
            "#",
            "^",
            ":",
            "::",
            "a",
            "é",
            "界",
            "e\u{301}",
            "1",
            "-",
            ".",
            ",",
            "📅",
            "🗓\u{fe0f}",
            "⏫",
            "✅",
            "🆔",
            "⛔",
            "🔁",
            "2026-12-01",
            "\u{fe0f}",
        ];
        let text: String = (0..rng.below(24)).map(|_| rng.pick(&PIECES)).collect();
        let task = block::Task {
            start: rng.below(text.len() + 3),
            end: rng.below(text.len() + 6),
            symbol: rng.pick(&[' ', 'x', '界']),
            status: rng.pick(&[StatusType::Open, StatusType::Done]),
        };
        if let Some(read) = parse(&text, task) {
            well_formed(&read);
            unchanged_holds(&read);
            for field in DateField::ALL {
                let _ = read.set_date(field, Date::new(2026, 12, 1));
                let _ = read.set_date(field, None);
            }
            let _ = read.set_priority(Some(Priority::Low));
            let _ = read.set_priority(None);
            let _ = read.set_description("x");
            let _ = read.add_owner("lee");
            let _ = read.remove_owner("lee");
        }
        line_holds(&text, rng);
    }

    fn corpus(count: usize, seed: u64) {
        let mut rng = Rng(seed);
        let mut tasks = 0;
        for _ in 0..count {
            let line = random_line(&mut rng);
            tasks += usize::from(line_holds(&line, &mut rng));
            any_text_holds(&mut rng);
        }
        // Most generated lines are task lines; a dropped box is the rest.
        assert!(tasks * 10 > count * 9, "{tasks} of {count}");
    }

    /// Parse then write is stable, a change reads back, and what the
    /// change is not about survives byte for byte.
    #[test]
    fn random_task_lines_hold() {
        corpus(1500, 3);
    }

    #[test]
    #[ignore = "full count; cargo test -p wren-markdown fuzz_task_line -- --ignored"]
    fn fuzz_task_line() {
        corpus(300_000, 4);
    }

    /// Owner names fold by Unicode's simple case folding, including where
    /// it is not lowercasing.
    #[test]
    fn owner_names_fold_by_simple_case_folding() {
        for (a, b) in [
            ("µ", "μ"),
            ("Μ", "µ"),
            ("ς", "σ"),
            ("Σ", "ς"),
            ("ſ", "s"),
            ("S", "ſ"),
            ("\u{212a}", "k"),
            ("K", "\u{212a}"),
            ("ϐ", "β"),
            ("ẛ", "ṡ"),
            ("\u{1c80}", "в"),
            ("Ꭰ", "ꭰ"),
            ("ᏸ", "Ᏸ"),
            ("Ǆ", "ǅ"),
            ("Sam.K", "sam.k"),
        ] {
            assert_eq!(fold_name(a), fold_name(b), "{a:?} {b:?}");
        }
        for (a, b) in [("s", "z"), ("ß", "ss"), ("İ", "i"), ("µ", "u")] {
            assert_ne!(fold_name(a), fold_name(b), "{a:?} {b:?}");
        }
        // One character stays one: simple folding never lengthens a name.
        assert_eq!(fold_name("İ").chars().count(), 1);
        // The table's ranges are in order and apart.
        for pair in FOLDS.windows(2) {
            assert!(pair[0].1 < pair[1].0, "{pair:?}");
        }
        assert!(FOLDS.iter().all(|(first, last, _)| first <= last));
    }

    /// What may be typed after an `@` is what a name can start with.
    #[test]
    fn an_owner_prefix_is_the_start_of_a_name() {
        for typed in ["", "s", "sam.", "priya.k", "Jose\u{301}", "j-smith_2"] {
            assert!(is_owner_prefix(typed), "{typed:?}");
        }
        for typed in ["\u{301}", "1sam", "sam!", "sa m", ".sam"] {
            assert!(!is_owner_prefix(typed), "{typed:?}");
        }
    }
}
