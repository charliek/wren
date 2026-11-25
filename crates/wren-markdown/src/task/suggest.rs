//! Auto-suggest on a task line (discovery/tasks.md §6 item 2, §3.2 Q12):
//! what to offer for the word being typed at the caret, and the change
//! that choosing it makes.
//!
//! # When it offers anything
//!
//! Only on a line the whole document reads as a task's, whose box the host
//! hands over, with the caret at the end of a word (the end of the line,
//! or white space after it) and nothing selected, and never in a code
//! span or a comment. Then:
//!
//! - **A word of two letters or more** (ASCII letters only, white space
//!   before it) that starts an entry's word, case aside. Never the
//!   description's first word, so a task that starts "To do" offers
//!   nothing. The entries are "Due date…" (`due`, `date`), which opens
//!   the calendar; Today, Tomorrow, the weekdays and Next week, each
//!   written at once as a due date; and the five priorities (`high`, …,
//!   `priority`), High, Medium and Low first (Q7).
//! - **`@` and the start of a name**, `@` alone included, at the start of
//!   the description or after white space: the owners the host knows
//!   (most used first) whose name starts with what was typed, case folded
//!   ([`super::fold_name`]), one that matches it whole first.
//!
//! Only what the line lacks, as it reads without the word: no date entry
//! once it has a due date, no priority once it has one, and no owner it
//! names already.
//!
//! # Enter and ordinary writing
//!
//! Ordinary writing must never be rewritten. Most entry words are English
//! words too ("go **to**", "call **me**", "ship it **today**"), and a new
//! name may start a known one (`@sam` when only `@samantha` is known), so
//! Enter at the end of a word must keep continuing the list. Each offer
//! says whether Enter may choose it as offered ([`Suggested::enter`]), and
//! only one may: "Due date…", for a word that is still short of `due` or
//! `date` (`du`, `da`, `dat`). It writes nothing until a date is picked in
//! the calendar, and Escape there leaves the line as it was.
//!
//! Every other offer, and "Due date…" for `due` or `date` typed whole,
//! waits for Tab, a click, or Up or Down before Enter. The editor applies
//! that rule; this module only says which is which.

use std::ops::Range;

use super::{fold_name, parse, Change, Priority, TaskLine};
use crate::block;
use crate::date::{self, Date, Parsed};
use crate::span::{self, SpanKind};

/// The fewest letters of a word that open suggestions.
pub const MIN_WORD: usize = 2;

/// The most owners offered at once.
pub const MAX_OWNERS: usize = 6;

/// What a suggestion does when chosen.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    /// Open the calendar, which writes the date picked (and takes the
    /// word out with it).
    Calendar,
    /// Write this due date at once.
    Due(Date),
    /// Write this priority at once.
    Priority(Priority),
    /// Complete the `@name` with this owner's name, the one at `index` of
    /// the owners the offer was made from.
    Owner { name: String, index: usize },
}

/// An owner the host knows: its name as written, and the key its
/// spellings share ([`super::fold_name`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    pub name: String,
    pub key: String,
}

impl Known {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let key = fold_name(&name);
        Self { name, key }
    }
}

/// One suggestion: what it does, how it reads, and whether Enter may
/// choose it as offered (see the module's description).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggested {
    pub item: Item,
    /// "Due date…", "Today", "Friday", "High priority", or the owner's
    /// name with its `@`.
    pub label: String,
    pub enter: bool,
}

/// What to offer for the word at the caret: the word's bytes, which the
/// offer hangs under and replaces, and the suggestions in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub range: Range<usize>,
    pub items: Vec<Suggested>,
}

/// The entries a word opens, in the order they are offered.
enum Entry {
    Calendar,
    Date(&'static str, &'static [&'static str]),
    Priority(Priority, &'static [&'static str]),
}

const DATES: [(&str, &[&str]); 10] = [
    ("Today", &["today"]),
    ("Tomorrow", &["tomorrow"]),
    ("Monday", &["monday"]),
    ("Tuesday", &["tuesday"]),
    ("Wednesday", &["wednesday"]),
    ("Thursday", &["thursday"]),
    ("Friday", &["friday"]),
    ("Saturday", &["saturday"]),
    ("Sunday", &["sunday"]),
    ("Next week", &["next", "week"]),
];

/// The words of "Due date…".
const CALENDAR_WORDS: [&str; 2] = ["due", "date"];

/// The priorities in Q7's order, and the words of each.
const PRIORITIES: [(Priority, &[&str]); 5] = [
    (Priority::High, &["high", "priority"]),
    (Priority::Medium, &["medium", "priority"]),
    (Priority::Low, &["low", "priority"]),
    (Priority::Highest, &["highest", "priority"]),
    (Priority::Lowest, &["lowest", "priority"]),
];

/// What to offer with the caret at byte `caret` of `line`, whose task box
/// the whole document reads at `task`, nothing selected, on the day
/// `today`, from `owners` (most used first). `None` offers nothing, and so
/// does a `task` that is not this line's box.
pub fn suggest(
    line: &str,
    caret: usize,
    task: block::Task,
    today: Date,
    owners: &[Known],
) -> Option<Offer> {
    parse(line, task)?;
    let after = line.get(caret..)?;
    if after.chars().next().is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    let before = &line[..caret];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(at, c)| at + c.len_utf8());
    // The first byte of the text after the box.
    let body = task.end + (line[task.end..].len() - line[task.end..].trim_start().len());
    if start < body || start == caret {
        return None;
    }
    let range = start..caret;
    // Inside a code span or a comment the word is no part of the task's.
    let quoted = span::inline_spans(line, body..line.len())
        .iter()
        .any(|span| {
            matches!(span.kind, SpanKind::Code | SpanKind::Comment)
                && span.range.start < range.end
                && range.start < span.range.end
        });
    if quoted {
        return None;
    }
    let word = &line[range.clone()];
    // The line as it reads without the word: what it lacks.
    let without = format!("{}{}", &line[..start], &line[caret..]);
    let rest = parse(&without, task)?;
    let items = match word.strip_prefix('@') {
        Some(name) => owner_items(name, &rest, owners),
        // Never the description's first word.
        None if start > body => word_items(word, &rest, today),
        None => Vec::new(),
    };
    (!items.is_empty()).then_some(Offer { range, items })
}

/// The owners offered for `@name`, none of those `rest` names already.
fn owner_items(name: &str, rest: &TaskLine<'_>, owners: &[Known]) -> Vec<Suggested> {
    let mut chars = name.chars();
    let shaped = chars.next().is_none_or(char::is_alphabetic)
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if !shaped {
        return Vec::new();
    }
    let typed = fold_name(name);
    let named: Vec<String> = rest.owner_names().map(fold_name).collect();
    let mut found: Vec<(usize, &Known)> = owners
        .iter()
        .enumerate()
        .filter(|(_, owner)| owner.key.starts_with(&typed) && !named.contains(&owner.key))
        .collect();
    // A name typed whole comes first, whatever its rank.
    if let Some(whole) = found.iter().position(|(_, owner)| owner.key == typed) {
        let owner = found.remove(whole);
        found.insert(0, owner);
    }
    found.truncate(MAX_OWNERS);
    found
        .into_iter()
        .map(|(index, owner)| Suggested {
            item: Item::Owner {
                name: owner.name.clone(),
                index,
            },
            label: format!("@{}", owner.name),
            enter: false,
        })
        .collect()
}

/// The entries `word` starts, of those `rest` lacks.
fn word_items(word: &str, rest: &TaskLine<'_>, today: Date) -> Vec<Suggested> {
    if word.len() < MIN_WORD || !word.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Vec::new();
    }
    let typed = word.to_ascii_lowercase();
    let starts = |words: &[&str]| words.iter().any(|each| each.starts_with(&typed));
    let mut entries = Vec::new();
    if rest.due().is_none() {
        entries.push(Entry::Calendar);
        entries.extend(DATES.iter().map(|(label, words)| Entry::Date(label, words)));
    }
    if rest.priority().is_none() {
        entries.extend(
            PRIORITIES
                .iter()
                .map(|(level, words)| Entry::Priority(*level, words)),
        );
    }
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Calendar => starts(&CALENDAR_WORDS).then(|| Suggested {
                item: Item::Calendar,
                label: "Due date…".to_owned(),
                // Short of the word, so not the word itself.
                enter: !CALENDAR_WORDS.contains(&typed.as_str()),
            }),
            Entry::Date(label, words) => {
                let date = match date::parse(&label.to_ascii_lowercase(), today)? {
                    Parsed::Date(date) => date,
                    Parsed::Invalid => return None,
                };
                starts(words).then(|| Suggested {
                    item: Item::Due(date),
                    label: label.to_owned(),
                    enter: false,
                })
            }
            Entry::Priority(level, words) => starts(words).then(|| Suggested {
                item: Item::Priority(level),
                label: format!("{} priority", level.name()),
                enter: false,
            }),
        })
        .collect()
}

/// The change choosing `item` makes to `line`, where the word it was
/// offered for is at `range`: the word taken out and the due date or the
/// priority written, as one change; for an owner, the word completed where
/// it is, or, typed after the line's tokens, where an owner there would
/// end them for Obsidian Tasks, taken out and the owner written at the
/// end of the description. `None` for the calendar, which writes when a
/// date is picked ([`erase`]), for a range that is not a word of the
/// line's, and for a `task` that is not the line's box as the whole
/// document reads it.
pub fn completion(
    line: &str,
    range: Range<usize>,
    task: block::Task,
    item: &Item,
) -> Option<Change> {
    line.get(range.clone())?;
    parse(line, task)?;
    Some(match item {
        Item::Calendar => return None,
        Item::Due(date) => Change::Compound(vec![erase(range), Change::SetDue(Some(*date))]),
        Item::Priority(level) => {
            Change::Compound(vec![erase(range), Change::SetPriority(Some(*level))])
        }
        Item::Owner { name, .. } => {
            let without = format!("{}{}", &line[..range.start], &line[range.end..]);
            let rest = parse(&without, task)?;
            let after_tokens = rest
                .tokens()
                .iter()
                .any(|token| token.range().start < range.start);
            if after_tokens {
                Change::Compound(vec![erase(range), Change::AddOwner(name.clone())])
            } else {
                Change::ReplaceWord(range, format!("@{name}"))
            }
        }
    })
}

/// The change that takes the word at `range` out, with the white space
/// before it.
pub fn erase(range: Range<usize>) -> Change {
    Change::ReplaceWord(range, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::Weekday;
    use crate::task::edit;
    use crate::{Options, Pos, Selection, TaskOptions};

    fn today() -> Date {
        Date::parse_iso("2026-11-19").unwrap()
    }

    const OWNERS: [&str; 4] = ["sam", "Samantha", "priya.k", "alice.chen"];

    /// The task box a line alone reads, as the whole document reads a line
    /// with nothing before it.
    fn boxed(line: &str) -> Option<block::Task> {
        crate::block::Scanner::new(crate::block::Boxes::Status)
            .next_line(line)
            .item?
            .task
    }

    fn known(owners: &[&str]) -> Vec<Known> {
        owners.iter().map(|owner| Known::new(*owner)).collect()
    }

    /// [`suggest`] on a line read alone, with `owners`.
    fn at(line: &str, caret: usize, owners: &[&str]) -> Option<Offer> {
        suggest(line, caret, boxed(line)?, today(), &known(owners))
    }

    /// The labels offered with the caret at the end of `line`, and whether
    /// Enter takes each.
    fn offered(line: &str) -> Vec<(String, bool)> {
        offered_at(line, line.len())
    }

    fn offered_at(line: &str, caret: usize) -> Vec<(String, bool)> {
        at(line, caret, &OWNERS)
            .map(|offer| {
                offer
                    .items
                    .into_iter()
                    .map(|item| (item.label, item.enter))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn labels(line: &str) -> Vec<String> {
        offered(line).into_iter().map(|(label, _)| label).collect()
    }

    #[test]
    fn the_trigger_table() {
        let none: &[&str] = &[];
        let cases: &[(&str, &[&str])] = &[
            // Two letters at a word's start, past the description's first.
            ("- [ ] Mail the deck du", &["Due date…"]),
            ("- [ ] Mail the deck da", &["Due date…"]),
            ("- [ ] Mail the deck to", &["Today", "Tomorrow"]),
            ("- [ ] Mail the deck tod", &["Today"]),
            ("- [ ] Mail the deck tom", &["Tomorrow"]),
            ("- [ ] Mail the deck fr", &["Friday"]),
            ("- [ ] Mail the deck sa", &["Saturday"]),
            ("- [ ] Mail the deck we", &["Wednesday", "Next week"]),
            ("- [ ] Mail the deck ne", &["Next week"]),
            (
                "- [ ] Mail the deck hi",
                &["High priority", "Highest priority"],
            ),
            ("- [ ] Mail the deck me", &["Medium priority"]),
            (
                "- [ ] Mail the deck lo",
                &["Low priority", "Lowest priority"],
            ),
            (
                "- [ ] Mail the deck pr",
                &[
                    "High priority",
                    "Medium priority",
                    "Low priority",
                    "Highest priority",
                    "Lowest priority",
                ],
            ),
            (
                "- [ ] Mail the deck HI",
                &["High priority", "Highest priority"],
            ),
            ("- [x] Done thing to", &["Today", "Tomorrow"]),
            ("1. [ ] Numbered to", &["Today", "Tomorrow"]),
            ("> - [ ] Quoted to", &["Today", "Tomorrow"]),
            // One letter, no match, or a word that runs past an entry's.
            ("- [ ] Mail the deck t", none),
            ("- [ ] Mail the deck d", none),
            ("- [ ] Mail the deck discuss", none),
            ("- [ ] Mail the deck todays", none),
            ("- [ ] Mail the deck highs", none),
            // The description's first word, and no description at all.
            ("- [ ] to", none),
            ("- [ ] today", none),
            ("- [ ] hi there", none),
            ("- [ ] ", none),
            // Not a task line.
            ("Mail the deck to", none),
            ("- Mail the deck to", none),
            ("    - [ ] code to", none),
            // Not a word of letters, or not at a word's start.
            ("- [ ] Mail the (to", none),
            ("- [ ] Mail the deck t0", none),
            ("- [ ] Mail the deck x-to", none),
            ("- [ ] Mail the deck tö", none),
            // Only what the line lacks.
            ("- [ ] Mail 📅 2026-12-01 to", none),
            ("- [ ] Mail 📅 2026-12-01 du", none),
            ("- [ ] Mail ⏫ hi", none),
            ("- [ ] Mail ⏫\u{fe0f} me", none),
            ("- [ ] Mail ⏫ to", &["Today", "Tomorrow"]),
            (
                "- [ ] Mail 📅 2026-12-01 hi",
                &["High priority", "Highest priority"],
            ),
            // Owners: `@` alone, a start, the first word too.
            (
                "- [ ] Mail @",
                &["@sam", "@Samantha", "@priya.k", "@alice.chen"],
            ),
            ("- [ ] Mail @sa", &["@sam", "@Samantha"]),
            ("- [ ] Mail @SA", &["@sam", "@Samantha"]),
            ("- [ ] @pr", &["@priya.k"]),
            ("- [ ] Mail @samantha", &["@Samantha"]),
            ("- [ ] Mail 📅 2026-12-01 @al", &["@alice.chen"]),
            // A name already there is not offered again.
            ("- [ ] Mail @sam and @sa", &["@Samantha"]),
            ("- [ ] Mail @SAM and @sa", &["@Samantha"]),
            ("- [ ] Mail @zz", none),
            ("- [ ] Mail a@sa", none),
            ("- [ ] Mail @1", none),
            ("- [ ] Mail @s!", none),
        ];
        for (line, expected) in cases {
            assert_eq!(labels(line), *expected, "{line:?}");
        }
    }

    #[test]
    fn the_caret_must_end_the_word_and_follow_the_box() {
        let line = "- [ ] Mail the deck to the bank";
        let after_to = line.find(" the bank").unwrap();
        assert_eq!(labels(&line[..after_to]), ["Today", "Tomorrow"]);
        // At the end of `to`, with white space after it.
        assert_eq!(offered_at(line, after_to).len(), 2);
        // Inside a word, or in the box.
        assert!(offered_at(line, after_to - 1).is_empty());
        assert!(offered_at(line, 3).is_empty());
        assert!(offered_at(line, 0).is_empty());
        // Off the line or inside a character.
        assert!(offered_at(line, line.len() + 1).is_empty());
        assert!(offered_at("- [ ] Mail é to", 12).is_empty());
    }

    #[test]
    fn every_weekday_is_offered_as_the_next_one() {
        for weekday in Weekday::ALL {
            let word = &weekday.name()[..2].to_ascii_lowercase();
            let offer = at(&format!("- [ ] Call {word}"), 11 + 2, &[]).unwrap();
            let item = offer
                .items
                .iter()
                .find(|item| item.label == weekday.name())
                .unwrap_or_else(|| panic!("{weekday:?}"));
            let Item::Due(date) = item.item else {
                panic!("{item:?}")
            };
            assert_eq!(date.weekday(), weekday);
            // The next one after today, Thursday 19 November.
            assert!((1..=7).contains(&today().days_until(date)), "{date}");
        }
        let offer = at("- [ ] Call to", 13, &[]).unwrap();
        let dates: Vec<_> = offer.items.iter().map(|item| item.item.clone()).collect();
        assert_eq!(
            dates,
            [
                Item::Due(today()),
                Item::Due(Date::parse_iso("2026-11-20").unwrap())
            ]
        );
        let offer = at("- [ ] Call next", 15, &[]).unwrap();
        assert_eq!(
            offer.items[0].item,
            Item::Due(Date::parse_iso("2026-11-23").unwrap())
        );
    }

    /// The Enter table: at the end of a line, Enter after each of these
    /// words continues the list (the editor's rule, from `enter`), except
    /// where the offer writes nothing until it is confirmed.
    #[test]
    fn enter_takes_only_what_cannot_rewrite_writing() {
        let cases: &[(&str, bool)] = &[
            ("to", false),
            ("hi", false),
            ("lo", false),
            ("me", false),
            ("fr", false),
            ("sa", false),
            ("tod", false),
            ("today", false),
            ("high", false),
            ("du", true),
            ("da", true),
            ("dat", true),
            ("due", false),
            ("date", false),
        ];
        for (word, enter) in cases {
            let line = format!("- [ ] Meet sam {word}");
            let offer = offered(&line);
            assert!(!offer.is_empty(), "{word}");
            // Enter goes by the row it would choose: the first.
            assert_eq!(offer[0].1, *enter, "{word}");
        }
        // Owners never: a new name may start a known one.
        for line in [
            "- [ ] Mail @sa",
            "- [ ] Mail @sam",
            "- [ ] Mail @Sam",
            "- [ ] Mail @",
        ] {
            let offer = offered(line);
            assert!(!offer.is_empty(), "{line}");
            assert!(offer.iter().all(|(_, enter)| !enter), "{line}");
        }
    }

    #[test]
    fn a_name_typed_whole_comes_first() {
        let offer = at("- [ ] Mail @sam", 15, &["Samantha", "sam"]).unwrap();
        let names: Vec<_> = offer.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(names, ["@sam", "@Samantha"]);
        assert_eq!(offer.range, 11..15);
        let many: Vec<String> = (0..10).map(|n| format!("person{n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let offer = at("- [ ] Mail @p", 13, &many).unwrap();
        assert_eq!(offer.items.len(), MAX_OWNERS);
    }

    /// `line` with `change` made to it, as `task::edit` writes it.
    fn written(line: &str, change: &Change) -> String {
        let options = Options {
            tasks: Some(TaskOptions { today: today() }),
            ..Options::default()
        };
        let at = Selection::caret(Pos::new(0, line.len()));
        edit(&[line], 0, at, change, &options).unwrap().text
    }

    fn chosen(line: &str, label: &str) -> String {
        let offer = at(line, line.len(), &OWNERS).unwrap();
        let item = offer.items.iter().find(|item| item.label == label).unwrap();
        let change =
            completion(line, offer.range.clone(), boxed(line).unwrap(), &item.item).unwrap();
        written(line, &change)
    }

    #[test]
    fn a_choice_takes_the_word_out_and_writes_the_token() {
        let cases: &[(&str, &str, &str)] = &[
            (
                "- [ ] Mail the deck to",
                "Today",
                "- [ ] Mail the deck 📅 2026-11-19",
            ),
            (
                "- [ ] Mail the deck fr",
                "Friday",
                "- [ ] Mail the deck 📅 2026-11-20",
            ),
            (
                "- [ ] Mail the deck hi",
                "High priority",
                "- [ ] Mail the deck ⏫",
            ),
            (
                "- [ ] Mail the deck lo",
                "Lowest priority",
                "- [ ] Mail the deck ⏬",
            ),
            // After tokens, the word ended them; out, they read again.
            (
                "- [ ] Mail ⏫ to",
                "Tomorrow",
                "- [ ] Mail ⏫ 📅 2026-11-20",
            ),
            (
                "- [ ] Mail 📅 2026-12-01 hi",
                "High priority",
                "- [ ] Mail ⏫ 📅 2026-12-01",
            ),
            (
                "- [ ] Mail 📅 2026-12-01 ^ab hi",
                "High priority",
                "- [ ] Mail ⏫ 📅 2026-12-01 ^ab",
            ),
            // An owner is completed where it is typed.
            ("- [ ] Mail @sa", "@Samantha", "- [ ] Mail @Samantha"),
            ("- [ ] @pr", "@priya.k", "- [ ] @priya.k"),
            // After the tokens it joins the description.
            (
                "- [ ] Mail 📅 2026-12-01 @al",
                "@alice.chen",
                "- [ ] Mail @alice.chen 📅 2026-12-01",
            ),
            ("- [ ] Mail ⏫ @sa", "@sam", "- [ ] Mail @sam ⏫"),
        ];
        for (line, label, expected) in cases {
            assert_eq!(chosen(line, label), *expected, "{line:?} {label:?}");
        }
        // A word in the middle of the description.
        let line = "- [ ] Ask @sa about it";
        let task = boxed(line).unwrap();
        let sam = Item::Owner {
            name: "sam".into(),
            index: 0,
        };
        let change = completion(line, 10..13, task, &sam).unwrap();
        assert_eq!(written(line, &change), "- [ ] Ask @sam about it");
        let line = "- [ ] Mail to the bank";
        let task = boxed(line).unwrap();
        let change = completion(line, 11..13, task, &Item::Due(today())).unwrap();
        assert_eq!(written(line, &change), "- [ ] Mail the bank 📅 2026-11-19");
        // The calendar writes later; a range off the line is none.
        assert_eq!(completion(line, 11..13, task, &Item::Calendar), None);
        assert_eq!(completion(line, 11..99, task, &Item::Due(today())), None);
        // A box that is not the line's.
        let elsewhere = block::Task {
            start: 4,
            end: 7,
            ..task
        };
        assert_eq!(
            completion(line, 11..13, elsewhere, &Item::Due(today())),
            None
        );
        // Erasing alone.
        assert_eq!(written("- [ ] Mail du", &erase(11..13)), "- [ ] Mail");
    }

    #[test]
    fn a_word_is_replaced_only_after_the_box_and_on_the_line() {
        let options = Options {
            tasks: Some(TaskOptions { today: today() }),
            ..Options::default()
        };
        let line = "- [ ] Mail é du";
        let at = Selection::caret(Pos::new(0, line.len()));
        let refused = |change: Change| edit(&[line], 0, at, &change, &options).is_none();
        // The box, off the line, inside `é`, backwards, a line break.
        assert!(refused(Change::ReplaceWord(2..5, String::new())));
        assert!(refused(Change::ReplaceWord(14..99, "x".into())));
        assert!(refused(Change::ReplaceWord(12..14, String::new())));
        #[allow(clippy::reversed_empty_ranges)]
        let backwards = 16..14;
        assert!(refused(Change::ReplaceWord(backwards, String::new())));
        assert!(refused(Change::ReplaceWord(14..16, "a\nb".into())));
        let change = Change::ReplaceWord(14..16, "due".into());
        assert_eq!(written(line, &change), "- [ ] Mail é due");
    }

    #[test]
    fn nothing_inside_a_code_span_or_a_comment() {
        let none: &[&str] = &[];
        for (line, caret) in [
            ("- [ ] Run `echo du now`", "- [ ] Run `echo du".len()),
            ("- [ ] Run `du`", "- [ ] Run `du".len()),
            (
                "- [ ] Note <!-- to do --> today",
                "- [ ] Note <!-- to".len(),
            ),
            ("- [ ] Ask `x` @sa", "- [ ] Ask `x` @sa".len()),
        ] {
            let found: Vec<String> = at(line, caret, &OWNERS)
                .map(|offer| offer.items.into_iter().map(|item| item.label).collect())
                .unwrap_or_default();
            if line.ends_with("@sa") {
                // After a closed code span, the word is the task's.
                assert_eq!(found, ["@sam", "@Samantha"], "{line:?}");
            } else {
                assert_eq!(found, none, "{line:?}");
            }
        }
        // An unclosed backtick is no code span.
        assert_eq!(labels("- [ ] Run `echo du"), ["Due date…"]);
    }

    #[test]
    fn the_box_the_document_reads_is_the_one_used() {
        // An indented child task, which a line read alone takes for code.
        let line = "    - [ ] Call fr";
        assert!(boxed(line).is_none());
        let task = block::Task {
            start: 6,
            end: 9,
            symbol: ' ',
            status: crate::status::StatusType::Open,
        };
        let offer = suggest(line, line.len(), task, today(), &[]).unwrap();
        assert_eq!(offer.items[0].label, "Friday");
        let change = completion(line, offer.range.clone(), task, &offer.items[0].item).unwrap();
        let options = Options {
            tasks: Some(TaskOptions { today: today() }),
            ..Options::default()
        };
        let lines = ["- Parent", line];
        let at = Selection::caret(Pos::new(1, line.len()));
        let written = edit(&lines, 1, at, &change, &options).unwrap().text;
        assert_eq!(written, "    - [ ] Call 📅 2026-11-20");
        // A box elsewhere on the line is no task's.
        let wrong = block::Task {
            start: 2,
            end: 5,
            ..task
        };
        assert_eq!(suggest(line, line.len(), wrong, today(), &[]), None);
    }

    #[test]
    fn owners_compare_case_folded() {
        // `@Σ` and `@ς` are one owner; a name is shown as it was written.
        assert_eq!(fold_name("ΣΑΣ"), fold_name("σας"));
        let offer = at("- [ ] Ask @ς", "- [ ] Ask @ς".len(), &["Σοφία", "Sam"]).unwrap();
        assert_eq!(offer.items[0].label, "@Σοφία");
        assert_eq!(
            offer.items[0].item,
            Item::Owner {
                name: "Σοφία".into(),
                index: 0
            }
        );
        // A name the line has, in another case or form, is not offered.
        assert!(at(
            "- [ ] Ask @ΣΟΦΊΑ and @σ",
            "- [ ] Ask @ΣΟΦΊΑ and @σ".len(),
            &["Σοφία"]
        )
        .is_none());
        assert!(at(
            "- [ ] Ask @Sam and @s",
            "- [ ] Ask @Sam and @s".len(),
            &["sam"]
        )
        .is_none());
    }
}
