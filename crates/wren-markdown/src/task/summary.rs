//! A task line spelled out in words, as the status bar shows the caret's
//! task (discovery/tasks.md §2 item 23): `Doing · due Mon 23 Nov, in 4
//! days · ⏫ High · @alice.chen`.
//!
//! The pieces come in this order, each only when the line has it:
//!
//! - The status' name. A symbol with no row of its own is named by its
//!   type with the symbol after it, `Open [a]`, so the line says both what
//!   the task counts as and what its box holds.
//! - The due date: `due Mon 23 Nov, in 4 days` on unfinished work, the
//!   relative part `today`, `tomorrow`, `in N days`, from two weeks out
//!   `in N weeks`, and once the day has passed `N days overdue`. A finished
//!   task's due date has no relative part: it can no longer be late. A
//!   date that is no real day reads `due 2026-02-30 is not a real date`,
//!   whatever the status.
//! - The priority, its glyph and its level: `⏫ High`.
//! - The owners, `@sam @priya`, or `yours` for a task with none: a task
//!   without an owner is the user's own (§2 item 3).
//! - A done stamp, `done Thu 19 Nov`, then a cancelled one,
//!   `cancelled Mon 16 Nov`, with the same wording as the due date for a
//!   day that is no real one.
//!
//! The words are English, as the date words the task line is typed with
//! are ([`crate::date`]).

use super::{DateField, DateToken, TaskLine};
use crate::date::{Date, Parsed};
use crate::status::{StatusType, STATUSES};

/// What goes between two pieces.
pub const SEPARATOR: &str = " · ";

/// How far ahead a due date is counted in days before it is counted in
/// weeks.
const DAYS_AHEAD: i32 = 14;

/// A task line spelled out: its pieces, in order. See the module's
/// documentation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub pieces: Vec<Piece>,
}

/// One piece of a [`Summary`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub look: Look,
}

/// What a piece says, for the host to draw it by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Look {
    /// The status' name, which leads.
    Status,
    /// Words with nothing to flag.
    Plain,
    /// An unfinished task's due date, passed.
    Overdue,
    /// A date that is no real day.
    Invalid,
    /// The task's `@owner`s.
    Owners,
}

impl Look {
    /// The look's name, in lower case.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Plain => "plain",
            Self::Overdue => "overdue",
            Self::Invalid => "invalid",
            Self::Owners => "owners",
        }
    }
}

impl Summary {
    /// The pieces' text, joined with [`SEPARATOR`].
    pub fn text(&self) -> String {
        let mut text = String::new();
        for (index, piece) in self.pieces.iter().enumerate() {
            if index > 0 {
                text.push_str(SEPARATOR);
            }
            text.push_str(&piece.text);
        }
        text
    }

    /// Whether the task is overdue: unfinished, with a due date before
    /// today.
    pub fn overdue(&self) -> bool {
        self.pieces.iter().any(|piece| piece.look == Look::Overdue)
    }
}

/// `line` spelled out, with `today` the day the relative dates count from.
pub fn summary(line: &TaskLine<'_>, today: Date) -> Summary {
    let task = line.task();
    let mut pieces = Vec::new();

    let row = task.row();
    let status = if STATUSES.iter().any(|known| known.symbol == row.symbol) {
        row.name.to_owned()
    } else {
        let mut name = type_name(task.status).to_owned();
        name.push_str(" [");
        if task.symbol.is_control() {
            name.extend(task.symbol.escape_unicode());
        } else {
            name.push(task.symbol);
        }
        name.push(']');
        name
    };
    pieces.push(Piece {
        text: status,
        look: Look::Status,
    });

    if let Some(due) = line.due() {
        pieces.push(due_piece(line, due, task.status.is_unfinished(), today));
    }

    if let Some(priority) = line.priority() {
        let level = priority.level;
        pieces.push(Piece {
            text: format!("{} {}", level.glyph(), level.name()),
            look: Look::Plain,
        });
    }

    let mut owners: Vec<&str> = Vec::new();
    for name in line.owner_names() {
        if !owners.contains(&name) {
            owners.push(name);
        }
    }
    pieces.push(if owners.is_empty() {
        Piece {
            text: "yours".to_owned(),
            look: Look::Plain,
        }
    } else {
        let mut text = String::new();
        for (index, name) in owners.iter().enumerate() {
            if index > 0 {
                text.push(' ');
            }
            text.push('@');
            text.push_str(name);
        }
        Piece {
            text,
            look: Look::Owners,
        }
    });

    for field in [DateField::Done, DateField::Cancelled] {
        if let Some(stamp) = line.date(field) {
            let word = match field {
                DateField::Done => "done",
                _ => "cancelled",
            };
            pieces.push(match stamp.value {
                Parsed::Date(date) => Piece {
                    text: format!("{word} {}", date.short()),
                    look: Look::Plain,
                },
                Parsed::Invalid => invalid(word, line, stamp),
            });
        }
    }

    Summary { pieces }
}

/// The due date's piece; see the module's documentation.
fn due_piece(line: &TaskLine<'_>, due: &DateToken, unfinished: bool, today: Date) -> Piece {
    let Parsed::Date(date) = due.value else {
        return invalid("due", line, due);
    };
    let short = date.short();
    if !unfinished {
        return Piece {
            text: format!("due {short}"),
            look: Look::Plain,
        };
    }
    let days = today.days_until(date);
    let relative = match days {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        2..DAYS_AHEAD => format!("in {days} days"),
        // Rounded to the nearest week; a seventh is never a half.
        DAYS_AHEAD.. => format!("in {} weeks", (days + 3) / 7),
        -1 => "1 day overdue".to_owned(),
        _ => format!("{} days overdue", -days),
    };
    Piece {
        text: format!("due {short}, {relative}"),
        look: if days < 0 { Look::Overdue } else { Look::Plain },
    }
}

/// `<word> 2026-02-30 is not a real date`, for a date token whose date is
/// no real day, as it is written.
fn invalid(word: &str, line: &TaskLine<'_>, token: &DateToken) -> Piece {
    Piece {
        text: format!("{word} {} is not a real date", line.text(&token.date)),
        look: Look::Invalid,
    }
}

/// The name a status type's own row gives it.
fn type_name(kind: StatusType) -> &'static str {
    STATUSES
        .iter()
        .find(|row| row.kind == kind)
        .map_or("Open", |row| row.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Boxes, Scanner};
    use crate::task::parse;

    /// Thursday 19 November 2026, the design page's day.
    fn today() -> Date {
        Date::new(2026, 11, 19).unwrap()
    }

    /// `line`, read as a document of its own with tasks on, spelled out
    /// against `today`.
    fn spelled(line: &str, today: Date) -> Summary {
        let task = Scanner::new(Boxes::Status)
            .next_line(line)
            .item
            .and_then(|item| item.task)
            .unwrap_or_else(|| panic!("{line:?} is no task line"));
        let read = parse(line, task).unwrap_or_else(|| panic!("{line:?} is not read"));
        summary(&read, today)
    }

    fn text(line: &str) -> String {
        spelled(line, today()).text()
    }

    /// Each piece as `text` with its look, `<look>text` for any but plain.
    fn looks(line: &str) -> Vec<String> {
        spelled(line, today())
            .pieces
            .into_iter()
            .map(|piece| match piece.look {
                Look::Plain => piece.text,
                look => format!("<{}>{}", look.name(), piece.text),
            })
            .collect()
    }

    #[test]
    fn the_design_example_reads_exactly() {
        let line = "- [/] Draft the cutover runbook @alice.chen ⏫ 📅 2026-11-23";
        assert_eq!(
            text(line),
            "Doing · due Mon 23 Nov, in 4 days · ⏫ High · @alice.chen"
        );
        assert_eq!(
            looks(line),
            [
                "<status>Doing",
                "due Mon 23 Nov, in 4 days",
                "⏫ High",
                "<owners>@alice.chen",
            ]
        );
        assert!(!spelled(line, today()).overdue());
    }

    #[test]
    fn every_status_is_named() {
        for (line, wanted) in [
            ("- [ ] Call", "Open · yours"),
            ("- [/] Call", "Doing · yours"),
            ("- [x] Call", "Done · yours"),
            ("- [X] Call", "Done · yours"),
            ("- [-] Call", "Cancelled · yours"),
            ("- [a] Call", "Open [a] · yours"),
            ("- [?] Call", "Open [?] · yours"),
            ("- [界] Call", "Open [界] · yours"),
            ("- [\u{7}] Call", "Open [\\u{7}] · yours"),
            ("1. [/] Numbered", "Doing · yours"),
            ("> - [ ] Quoted", "Open · yours"),
            ("- [ ]\tTabbed", "Open · yours"),
            ("- [ ]", "Open · yours"),
        ] {
            assert_eq!(text(line), wanted, "{line:?}");
        }
    }

    #[test]
    fn an_unfinished_tasks_due_date_counts_from_today() {
        for (due, wanted) in [
            ("2026-11-19", "due Thu 19 Nov, today"),
            ("2026-11-20", "due Fri 20 Nov, tomorrow"),
            ("2026-11-21", "due Sat 21 Nov, in 2 days"),
            ("2026-11-23", "due Mon 23 Nov, in 4 days"),
            ("2026-12-02", "due Wed 2 Dec, in 13 days"),
            ("2026-12-03", "due Thu 3 Dec, in 2 weeks"),
            ("2026-12-06", "due Sun 6 Dec, in 2 weeks"),
            ("2026-12-07", "due Mon 7 Dec, in 3 weeks"),
            ("2027-01-01", "due Fri 1 Jan, in 6 weeks"),
            ("2027-11-19", "due Fri 19 Nov, in 52 weeks"),
            ("2026-11-18", "due Wed 18 Nov, 1 day overdue"),
            ("2026-11-16", "due Mon 16 Nov, 3 days overdue"),
            ("2025-11-19", "due Wed 19 Nov, 365 days overdue"),
        ] {
            for line in [
                format!("- [ ] Ship 📅 {due}"),
                format!("- [/] Ship 📅 {due}"),
                format!("- [a] Ship 📅 {due}"),
            ] {
                let read = spelled(&line, today());
                assert_eq!(read.pieces[1].text, wanted, "{line:?}");
                let late = due < "2026-11-19";
                assert_eq!(read.overdue(), late, "{line:?}");
                let look = if late { Look::Overdue } else { Look::Plain };
                assert_eq!(read.pieces[1].look, look, "{line:?}");
            }
        }
    }

    #[test]
    fn a_due_alias_reads_as_the_due_date() {
        for line in [
            "- [ ] Ship 📆 2026-11-23",
            "- [ ] Ship 🗓 2026-11-23",
            "- [ ] Ship 📅\u{fe0f} 2026-11-23",
            "- [ ] Ship [due:: 2026-11-23]",
        ] {
            assert_eq!(text(line), "Open · due Mon 23 Nov, in 4 days · yours");
        }
    }

    #[test]
    fn a_finished_tasks_due_date_is_never_late() {
        assert_eq!(
            looks("- [x] Ship 📅 2026-11-12 ✅ 2026-11-16"),
            ["<status>Done", "due Thu 12 Nov", "yours", "done Mon 16 Nov"]
        );
        assert_eq!(
            looks("- [-] Ship 📅 2026-11-30 ❌ 2026-11-19"),
            [
                "<status>Cancelled",
                "due Mon 30 Nov",
                "yours",
                "cancelled Thu 19 Nov",
            ]
        );
        assert!(!spelled("- [x] Ship 📅 2026-11-12", today()).overdue());
    }

    #[test]
    fn a_date_that_is_no_real_day_says_so_whatever_the_status() {
        for (line, wanted) in [
            (
                "- [ ] Pay 📅 2026-02-30",
                vec![
                    "<status>Open",
                    "<invalid>due 2026-02-30 is not a real date",
                    "yours",
                ],
            ),
            (
                "- [x] Pay 📅 2026-13-01 ✅ 2026-11-31",
                vec![
                    "<status>Done",
                    "<invalid>due 2026-13-01 is not a real date",
                    "yours",
                    "<invalid>done 2026-11-31 is not a real date",
                ],
            ),
            (
                "- [-] Pay ❌ 2026-02-29",
                vec![
                    "<status>Cancelled",
                    "yours",
                    "<invalid>cancelled 2026-02-29 is not a real date",
                ],
            ),
        ] {
            assert_eq!(looks(line), wanted, "{line:?}");
            assert!(!spelled(line, today()).overdue(), "{line:?}");
        }
    }

    #[test]
    fn stamps_follow_the_owners() {
        assert_eq!(
            text("- [x] Review the packet @bob 📅 2026-11-12 ✅ 2026-11-16"),
            "Done · due Thu 12 Nov · @bob · done Mon 16 Nov"
        );
        assert_eq!(
            text("- [-] Book the room @sam ❌ 2026-11-16"),
            "Cancelled · @sam · cancelled Mon 16 Nov"
        );
        // Typed by hand onto a line in another state, a stamp still shows.
        assert_eq!(
            text("- [ ] Reopened ✅ 2026-11-18"),
            "Open · yours · done Wed 18 Nov"
        );
        assert_eq!(
            text("- [x] Both ✅ 2026-11-18 ❌ 2026-11-17"),
            "Done · yours · done Wed 18 Nov · cancelled Tue 17 Nov"
        );
    }

    #[test]
    fn each_priority_is_its_glyph_and_level() {
        for (glyph, wanted) in [
            ("🔺", "🔺 Highest"),
            ("⏫", "⏫ High"),
            ("🔼", "🔼 Medium"),
            ("🔽", "🔽 Low"),
            ("⏬", "⏬ Lowest"),
            // Read with U+FE0F, written without it.
            ("⏫\u{fe0f}", "⏫ High"),
        ] {
            let line = format!("- [ ] Plan {glyph}");
            assert_eq!(text(&line), format!("Open · {wanted} · yours"), "{line:?}");
        }
        // Of several, the last is the one the line is read by.
        assert_eq!(text("- [ ] Plan 🔽 ⏫"), "Open · ⏫ High · yours");
    }

    #[test]
    fn owners_are_listed_once_each_in_line_order() {
        assert_eq!(text("- [ ] Call the bank"), "Open · yours");
        assert_eq!(
            looks("- [ ] Call the bank @sam"),
            ["<status>Open", "<owners>@sam"]
        );
        assert_eq!(
            looks("- [ ] @sam and @priya.n. call the bank 📅 2026-11-20"),
            [
                "<status>Open",
                "due Fri 20 Nov, tomorrow",
                "<owners>@sam @priya.n",
            ]
        );
        assert_eq!(
            text("- [ ] Ask @sam, then @sam again @dana ⏫"),
            "Open · ⏫ High · @sam @dana"
        );
        // An address is no owner.
        assert_eq!(text("- [ ] Mail a@b.com"), "Open · yours");
    }

    #[test]
    fn every_piece_at_once_in_order() {
        let line = "- [/] Ship @sam @priya 🆔 a1 🔺 📅 2026-11-17 ✅ 2026-11-18 ❌ 2026-11-19 ^b1";
        assert_eq!(
            looks(line),
            [
                "<status>Doing",
                "<overdue>due Tue 17 Nov, 2 days overdue",
                "🔺 Highest",
                "<owners>@sam @priya",
                "done Wed 18 Nov",
                "cancelled Thu 19 Nov",
            ]
        );
        assert!(spelled(line, today()).overdue());
    }

    #[test]
    fn the_relative_part_moves_with_today() {
        let line = "- [ ] Ship 📅 2026-11-23";
        let at = |day: u8| spelled(line, Date::new(2026, 11, day).unwrap()).text();
        assert_eq!(at(19), "Open · due Mon 23 Nov, in 4 days · yours");
        assert_eq!(at(22), "Open · due Mon 23 Nov, tomorrow · yours");
        assert_eq!(at(23), "Open · due Mon 23 Nov, today · yours");
        assert_eq!(at(24), "Open · due Mon 23 Nov, 1 day overdue · yours");
        assert_eq!(at(30), "Open · due Mon 23 Nov, 7 days overdue · yours");
    }
}
