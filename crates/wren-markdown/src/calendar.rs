//! The calendar a due date is picked with (discovery/tasks.md §6,
//! "Calendar popover"): a month of days, Monday first, with one selected,
//! and a field the date can be typed into instead. Typing reads what
//! [`date::parse`] reads, relative to today, and moves the selection to the
//! date typed; the arrow keys move it by a day or a week and the page keys
//! by a month. Enter writes the selection, or what the field names.
//!
//! This is the model only: a host draws it, gives its field the keyboard,
//! and hands it the keys the field does not take ([`Key`]).

use std::fmt;

use crate::date::{self, Date, Parsed};

/// The calendar's state: the month shown, the day selected, and what has
/// been typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    today: Date,
    /// The date the task has, which the grid marks.
    current: Option<Date>,
    selected: Date,
    month: Month,
    typed: String,
    /// What [`date::parse`] made of `typed`; `None` for blank text too.
    parsed: Option<Parsed>,
}

/// A key the calendar takes from its field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A week earlier.
    Up,
    /// A week later.
    Down,
    /// A day earlier, while nothing is typed.
    Left,
    /// A day later, while nothing is typed.
    Right,
    /// A month earlier, the day clamped to a shorter month.
    PageUp,
    /// A month later, the day clamped to a shorter month.
    PageDown,
}

/// A month of a year.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Month {
    year: u16,
    month: u8,
}

/// The English month names, January first.
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

impl Month {
    /// The month `date` is in.
    pub fn of(date: Date) -> Self {
        Self {
            year: date.year(),
            month: date.month(),
        }
    }

    pub fn year(self) -> u16 {
        self.year
    }

    /// 1 for January to 12 for December.
    pub fn month(self) -> u8 {
        self.month
    }

    /// Its first day.
    pub fn first(self) -> Date {
        Date::new(self.year, self.month, 1).expect("a month's first day is a date")
    }

    /// The month `months` later (earlier when negative), or `None` past the
    /// years 1 to 9999.
    pub fn add_months(self, months: i32) -> Option<Self> {
        self.first().add_months(months).map(Self::of)
    }

    /// Its English name, `November`.
    pub fn name(self) -> &'static str {
        MONTHS[usize::from(self.month - 1)]
    }
}

/// `November 2026`.
impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.name(), self.year)
    }
}

/// A cell of the month grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Day {
    pub date: Date,
    /// In the month shown, not a neighbour's.
    pub in_month: bool,
    pub today: bool,
    pub selected: bool,
    /// The date the task has.
    pub current: bool,
}

/// What Enter, or the Clear button, makes of the calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Write this due date.
    Write(Date),
    /// Remove the due date.
    Clear,
    /// The field holds no date: this text, trimmed. Nothing is written.
    Invalid(String),
}

/// The line under the field, which says what Enter would write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caption {
    /// Enter writes this date, which is this far from today.
    Date { date: Date, relative: String },
    /// The field holds a date in the shape `YYYY-MM-DD` that is no real
    /// day, as typed.
    NotReal(String),
    /// The field holds nothing [`date::parse`] reads.
    NotADate,
}

impl Caption {
    /// Whether it says the field holds no date.
    pub fn is_invalid(&self) -> bool {
        !matches!(self, Self::Date { .. })
    }
}

/// `Friday 20 November 2026 · tomorrow`, or what is wrong.
impl fmt::Display for Caption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Date { date, relative } => write!(f, "{} · {relative}", long(*date)),
            Self::NotReal(text) => write!(f, "{text} is not a real date"),
            Self::NotADate => f.write_str("Not a date. Try fri, +3d or 2026-12-01."),
        }
    }
}

/// The weekdays' two-letter heads, Monday first.
pub const WEEKDAY_HEADS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

/// The rows of the grid: six weeks hold every month, whatever day it
/// starts on.
pub const WEEKS: usize = 6;

impl Calendar {
    /// A calendar on `current`, the date the task has, or on `today` for a
    /// task with none. Nothing is typed.
    pub fn new(today: Date, current: Option<Date>) -> Self {
        let selected = current.unwrap_or(today);
        Self {
            today,
            current,
            selected,
            month: Month::of(selected),
            typed: String::new(),
            parsed: None,
        }
    }

    pub fn today(&self) -> Date {
        self.today
    }

    pub fn current(&self) -> Option<Date> {
        self.current
    }

    pub fn selected(&self) -> Date {
        self.selected
    }

    /// The month the grid shows.
    pub fn month(&self) -> Month {
        self.month
    }

    /// The field's text, as typed.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// What the field's text reads as: `None` when it is blank or names
    /// no date.
    pub fn parsed(&self) -> Option<Parsed> {
        self.parsed
    }

    /// Take a key the field does not: whether the calendar took it. Left
    /// and Right are the field's while it holds text, to move its caret.
    /// Up, Down and the page keys empty the field, so that Enter writes the
    /// day selected, whether or not the selection can move: at the ends of
    /// the years 1 to 9999 it stays. A key that moves the selection shows
    /// its month.
    pub fn key(&mut self, key: Key) -> bool {
        let moved = match key {
            Key::Left | Key::Right if !self.typed.is_empty() => return false,
            Key::Up => self.selected.add_days(-7),
            Key::Down => self.selected.add_days(7),
            Key::Left => self.selected.add_days(-1),
            Key::Right => self.selected.add_days(1),
            Key::PageUp => self.selected.add_months(-1),
            Key::PageDown => self.selected.add_months(1),
        };
        self.typed.clear();
        self.parsed = None;
        if let Some(date) = moved {
            self.select(date);
        }
        true
    }

    /// The field now holds `text`. It is read relative to today, and a
    /// date it names becomes the selection, its month shown; text that
    /// names none leaves the selection where it was.
    pub fn set_typed(&mut self, text: &str) {
        self.typed = text.to_owned();
        self.parsed = if text.trim().is_empty() {
            None
        } else {
            date::parse(text, self.today)
        };
        if let Some(Parsed::Date(date)) = self.parsed {
            self.select(date);
        }
    }

    /// Show the month `months` after the one shown (before, when negative),
    /// leaving the selection and the field as they are. `false` past the
    /// years 1 to 9999.
    pub fn show(&mut self, months: i32) -> bool {
        match self.month.add_months(months) {
            Some(month) => {
                self.month = month;
                true
            }
            None => false,
        }
    }

    /// Today is now `today`, as when the local date changes while the
    /// calendar is open: what is typed is read again from it, and moves
    /// the selection if it names a date; with nothing typed the selection
    /// stays.
    pub fn set_today(&mut self, today: Date) {
        if today != self.today {
            self.today = today;
            let typed = std::mem::take(&mut self.typed);
            self.set_typed(&typed);
        }
    }

    fn select(&mut self, date: Date) {
        self.selected = date;
        self.month = Month::of(date);
    }

    /// The month shown as six weeks of seven days, Monday first, with the
    /// days of the months either side that fill its first and last weeks.
    /// A cell past 31 December 9999 is `None`.
    pub fn grid(&self) -> [[Option<Day>; 7]; WEEKS] {
        let start = self.month.first().monday_of_week();
        std::array::from_fn(|week| {
            std::array::from_fn(|weekday| {
                let offset = i32::try_from(week * 7 + weekday).expect("a small offset");
                let date = start.add_days(offset)?;
                Some(Day {
                    date,
                    in_month: Month::of(date) == self.month,
                    today: date == self.today,
                    selected: date == self.selected,
                    current: Some(date) == self.current,
                })
            })
        })
    }

    /// What Enter does: write the date the field names, or with nothing
    /// typed the day selected; a field that names no date writes nothing.
    pub fn enter(&self) -> Outcome {
        if self.typed.trim().is_empty() {
            return Outcome::Write(self.selected);
        }
        match self.parsed {
            Some(Parsed::Date(date)) => Outcome::Write(date),
            Some(Parsed::Invalid) | None => Outcome::Invalid(self.typed.trim().to_owned()),
        }
    }

    /// The line under the field.
    pub fn caption(&self) -> Caption {
        match self.enter() {
            Outcome::Write(date) => Caption::Date {
                date,
                relative: relative(self.today, date),
            },
            Outcome::Invalid(text) if self.parsed == Some(Parsed::Invalid) => {
                Caption::NotReal(text)
            }
            Outcome::Invalid(_) | Outcome::Clear => Caption::NotADate,
        }
    }
}

/// `Friday 20 November 2026`.
pub fn long(date: Date) -> String {
    format!(
        "{} {} {} {}",
        date.weekday().name(),
        date.day(),
        Month::of(date).name(),
        date.year()
    )
}

/// How far `date` is from `today`: `today`, `tomorrow`, `yesterday`, `in
/// N days`, from two weeks out `in N weeks` (to the nearest week), and
/// before yesterday `N days ago`.
pub fn relative(today: Date, date: Date) -> String {
    let days = today.days_until(date);
    match days {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        -1 => "yesterday".to_owned(),
        2..14 => format!("in {days} days"),
        14.. => format!("in {} weeks", (days + 3) / 7),
        _ => format!("{} days ago", -days),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::Weekday;

    fn day(text: &str) -> Date {
        Date::parse_iso(text).unwrap_or_else(|| panic!("{text} is a date"))
    }

    /// Thursday 19 November 2026, the day every task test pins.
    fn today() -> Date {
        day("2026-11-19")
    }

    fn typed(text: &str) -> Calendar {
        let mut calendar = Calendar::new(today(), None);
        calendar.set_typed(text);
        calendar
    }

    #[test]
    fn it_opens_on_the_tasks_date_or_today_with_nothing_typed() {
        let fresh = Calendar::new(today(), None);
        assert_eq!(fresh.selected(), today());
        assert_eq!(fresh.month().to_string(), "November 2026");
        assert_eq!((fresh.typed(), fresh.parsed()), ("", None));
        assert_eq!(fresh.enter(), Outcome::Write(today()));
        assert_eq!(
            fresh.caption().to_string(),
            "Thursday 19 November 2026 · today"
        );
        let dated = Calendar::new(today(), Some(day("2027-01-04")));
        assert_eq!(dated.selected(), day("2027-01-04"));
        assert_eq!(dated.current(), Some(day("2027-01-04")));
        assert_eq!(dated.month().to_string(), "January 2027");
        assert_eq!(dated.enter(), Outcome::Write(day("2027-01-04")));
    }

    #[test]
    fn the_grid_is_six_weeks_monday_first() {
        let calendar = Calendar::new(today(), Some(day("2026-11-23")));
        let grid = calendar.grid();
        let cells: Vec<Day> = grid.iter().flatten().map(|cell| cell.unwrap()).collect();
        assert_eq!(cells.len(), 42);
        // November 2026 starts on a Sunday, so its first row is the last
        // week of October from Monday the 26th.
        assert_eq!(cells[0].date, day("2026-10-26"));
        assert_eq!(cells[41].date, day("2026-12-06"));
        for (index, cell) in cells.iter().enumerate() {
            assert_eq!(cell.date.weekday(), Weekday::ALL[index % 7], "{index}");
            assert_eq!(cell.in_month, cell.date.month() == 11, "{}", cell.date);
        }
        let marked = |pick: fn(&Day) -> bool| -> Vec<Date> {
            cells
                .iter()
                .filter(|cell| pick(cell))
                .map(|cell| cell.date)
                .collect()
        };
        assert_eq!(marked(|cell| cell.today), [today()]);
        assert_eq!(marked(|cell| cell.selected), [day("2026-11-23")]);
        assert_eq!(marked(|cell| cell.current), [day("2026-11-23")]);
        assert_eq!(cells.iter().filter(|cell| cell.in_month).count(), 30);
        assert_eq!(WEEKDAY_HEADS[0], "Mo");
        // A month that starts on a Monday starts the grid itself.
        let june = Calendar::new(today(), Some(day("2026-06-10")));
        assert_eq!(june.grid()[0][0].unwrap().date, day("2026-06-01"));
        // Leap Februaries: 2028 has a 29th, 2100 does not.
        for (text, days) in [("2028-02-10", 29), ("2100-02-10", 28), ("2000-02-10", 29)] {
            let february = Calendar::new(today(), Some(day(text)));
            let count = february
                .grid()
                .iter()
                .flatten()
                .filter(|cell| cell.is_some_and(|cell| cell.in_month))
                .count();
            assert_eq!(count, days, "{text}");
        }
    }

    #[test]
    fn the_grid_stops_at_the_last_day_there_is() {
        let last = Calendar::new(today(), Some(day("9999-12-31")));
        let grid = last.grid();
        let cells: Vec<Option<Day>> = grid.iter().flatten().copied().collect();
        let days: Vec<Date> = cells.iter().flatten().map(|cell| cell.date).collect();
        assert_eq!(days.last(), Some(&day("9999-12-31")));
        assert!(cells
            .iter()
            .skip_while(|cell| cell.is_some())
            .all(Option::is_none));
        let first = Calendar::new(today(), Some(day("0001-01-01")));
        assert_eq!(first.grid()[0][0].unwrap().date, day("0001-01-01"));
    }

    #[test]
    fn arrows_move_a_day_or_a_week_and_page_keys_a_month() {
        let mut calendar = Calendar::new(today(), None);
        let steps = [
            (Key::Down, "2026-11-26"),
            (Key::Down, "2026-12-03"),
            (Key::Right, "2026-12-04"),
            (Key::Up, "2026-11-27"),
            (Key::Left, "2026-11-26"),
            (Key::PageDown, "2026-12-26"),
            (Key::PageDown, "2027-01-26"),
            (Key::PageUp, "2026-12-26"),
            (Key::PageUp, "2026-11-26"),
        ];
        for (key, expected) in steps {
            assert!(calendar.key(key), "{key:?}");
            assert_eq!(calendar.selected(), day(expected), "{key:?}");
            assert_eq!(calendar.month(), Month::of(day(expected)), "{key:?}");
            assert_eq!(calendar.enter(), Outcome::Write(day(expected)));
        }
    }

    #[test]
    fn keys_cross_months_and_years_and_clamp_short_months() {
        let mut calendar = Calendar::new(today(), Some(day("2026-12-31")));
        assert!(calendar.key(Key::Right));
        assert_eq!(calendar.selected(), day("2027-01-01"));
        assert_eq!(calendar.month().to_string(), "January 2027");
        assert!(calendar.key(Key::Up));
        assert_eq!(calendar.selected(), day("2026-12-25"));
        let mut calendar = Calendar::new(today(), Some(day("2027-01-31")));
        assert!(calendar.key(Key::PageDown));
        assert_eq!(calendar.selected(), day("2027-02-28"));
        let mut calendar = Calendar::new(today(), Some(day("2028-01-31")));
        assert!(calendar.key(Key::PageDown));
        assert_eq!(calendar.selected(), day("2028-02-29"));
        assert!(calendar.key(Key::Right));
        assert_eq!(calendar.selected(), day("2028-03-01"));
        let mut calendar = Calendar::new(today(), Some(day("2028-03-01")));
        assert!(calendar.key(Key::Left));
        assert_eq!(calendar.selected(), day("2028-02-29"));
        // At the ends of the calendar a key is taken and moves nothing.
        let mut calendar = Calendar::new(today(), Some(day("9999-12-31")));
        for key in [Key::Right, Key::Down, Key::PageDown] {
            assert!(calendar.key(key));
            assert_eq!(calendar.selected(), day("9999-12-31"));
        }
        // There, Down still empties the field, as the field empties itself.
        calendar.set_typed("9999-12-31");
        assert!(calendar.key(Key::Down));
        assert_eq!((calendar.typed(), calendar.parsed()), ("", None));
        assert_eq!(calendar.selected(), day("9999-12-31"));
        let mut calendar = Calendar::new(today(), Some(day("0001-01-01")));
        for key in [Key::Left, Key::Up, Key::PageUp] {
            assert!(calendar.key(key));
            assert_eq!(calendar.selected(), day("0001-01-01"));
        }
    }

    #[test]
    fn typing_moves_the_selection_to_the_date_typed() {
        for (text, expected) in [
            ("fri", "2026-11-20"),
            ("Friday", "2026-11-20"),
            ("thu", "2026-11-26"),
            ("tomorrow", "2026-11-20"),
            ("today", "2026-11-19"),
            ("+3d", "2026-11-22"),
            ("+2w", "2026-12-03"),
            ("+1m", "2026-12-19"),
            ("next mon", "2026-11-23"),
            ("next fri", "2026-11-27"),
            ("next week", "2026-11-23"),
            ("2026-12-01", "2026-12-01"),
            ("  fri  ", "2026-11-20"),
            ("2028-02-29", "2028-02-29"),
        ] {
            let calendar = typed(text);
            assert_eq!(calendar.selected(), day(expected), "{text:?}");
            assert_eq!(calendar.month(), Month::of(day(expected)), "{text:?}");
            assert_eq!(calendar.enter(), Outcome::Write(day(expected)), "{text:?}");
            assert!(!calendar.caption().is_invalid(), "{text:?}");
            assert_eq!(calendar.typed(), text);
        }
        assert_eq!(
            typed("fri").caption().to_string(),
            "Friday 20 November 2026 · tomorrow"
        );
        assert_eq!(
            typed("+2w").caption().to_string(),
            "Thursday 3 December 2026 · in 2 weeks"
        );
    }

    #[test]
    fn text_that_names_no_date_writes_nothing_and_keeps_the_selection() {
        for (text, caption) in [
            ("2026-02-30", "2026-02-30 is not a real date"),
            ("2027-02-29", "2027-02-29 is not a real date"),
            ("2026-13-01", "2026-13-01 is not a real date"),
            ("blorp", "Not a date. Try fri, +3d or 2026-12-01."),
            ("fr", "Not a date. Try fri, +3d or 2026-12-01."),
            ("19/11/2026", "Not a date. Try fri, +3d or 2026-12-01."),
            ("next", "Not a date. Try fri, +3d or 2026-12-01."),
            ("+0x", "Not a date. Try fri, +3d or 2026-12-01."),
        ] {
            let mut calendar = typed("fri");
            calendar.set_typed(text);
            assert_eq!(calendar.selected(), day("2026-11-20"), "{text:?}");
            assert_eq!(
                calendar.enter(),
                Outcome::Invalid(text.to_owned()),
                "{text:?}"
            );
            assert!(calendar.caption().is_invalid(), "{text:?}");
            assert_eq!(calendar.caption().to_string(), caption, "{text:?}");
        }
        // Emptying the field writes the selection again.
        let mut calendar = typed("blorp");
        calendar.set_typed("   ");
        assert_eq!(calendar.enter(), Outcome::Write(today()));
        assert_eq!(calendar.parsed(), None);
    }

    #[test]
    fn left_and_right_are_the_fields_while_it_holds_text() {
        let mut calendar = typed("fri");
        assert!(!calendar.key(Key::Left));
        assert!(!calendar.key(Key::Right));
        assert_eq!(
            (calendar.typed(), calendar.selected()),
            ("fri", day("2026-11-20"))
        );
        // Up, Down and the page keys move from the date typed and empty
        // the field, so that Enter writes the day moved to.
        assert!(calendar.key(Key::Down));
        assert_eq!(
            (calendar.typed(), calendar.selected()),
            ("", day("2026-11-27"))
        );
        assert_eq!(calendar.enter(), Outcome::Write(day("2026-11-27")));
        let mut calendar = typed("blorp");
        assert!(calendar.key(Key::PageDown));
        assert_eq!(
            (calendar.typed(), calendar.selected()),
            ("", day("2026-12-19"))
        );
        assert!(!calendar.caption().is_invalid());
        // With the field empty again, Left and Right move a day.
        assert!(calendar.key(Key::Left));
        assert_eq!(calendar.selected(), day("2026-12-18"));
    }

    #[test]
    fn the_month_shown_can_change_without_the_selection() {
        let mut calendar = typed("fri");
        assert!(calendar.show(1));
        assert_eq!(calendar.month().to_string(), "December 2026");
        assert!(calendar.show(-2));
        assert_eq!(calendar.month().to_string(), "October 2026");
        assert_eq!(calendar.selected(), day("2026-11-20"));
        assert_eq!(calendar.typed(), "fri");
        let grid = calendar.grid();
        assert!(grid.iter().flatten().flatten().all(|cell| !cell.selected));
        // A key brings the selection's month back.
        assert!(calendar.key(Key::Down));
        assert_eq!(calendar.month().to_string(), "November 2026");
        let mut last = Calendar::new(today(), Some(day("9999-12-01")));
        assert!(!last.show(1));
        assert_eq!(last.month().to_string(), "December 9999");
    }

    #[test]
    fn a_new_day_reads_the_field_again() {
        let mut calendar = typed("fri");
        calendar.set_today(day("2026-11-20"));
        assert_eq!(calendar.today(), day("2026-11-20"));
        assert_eq!(calendar.selected(), day("2026-11-27"));
        assert_eq!(calendar.typed(), "fri");
        assert_eq!(
            calendar.caption().to_string(),
            "Friday 27 November 2026 · in 7 days"
        );
        let mut calendar = Calendar::new(today(), None);
        calendar.set_today(day("2026-11-20"));
        assert_eq!(calendar.selected(), today());
        assert_eq!(
            calendar.caption().to_string(),
            "Thursday 19 November 2026 · yesterday"
        );
        let grid = calendar.grid();
        let marked: Vec<Date> = grid
            .iter()
            .flatten()
            .flatten()
            .filter(|cell| cell.today)
            .map(|cell| cell.date)
            .collect();
        assert_eq!(marked, [day("2026-11-20")]);
    }

    #[test]
    fn relative_days_read_as_the_status_bar_reads_them() {
        for (date, expected) in [
            ("2026-11-19", "today"),
            ("2026-11-20", "tomorrow"),
            ("2026-11-18", "yesterday"),
            ("2026-11-21", "in 2 days"),
            ("2026-12-02", "in 13 days"),
            ("2026-12-03", "in 2 weeks"),
            ("2026-12-06", "in 2 weeks"),
            ("2026-12-07", "in 3 weeks"),
            ("2026-11-16", "3 days ago"),
        ] {
            assert_eq!(relative(today(), day(date)), expected, "{date}");
        }
        assert_eq!(long(day("2026-02-01")), "Sunday 1 February 2026");
    }
}
