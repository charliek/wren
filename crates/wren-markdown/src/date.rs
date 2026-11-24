//! Civil dates, as task lines write them: a year, a month and a day of the
//! proleptic Gregorian calendar, with no time and no zone. Weeks start on
//! Monday, and the words [`parse`] reads are English.

use std::fmt;

/// The furthest a relative word (`+Nd`, `+Nw`, `+Nm`) reaches from today,
/// in days, about 27 years. Anything further is not a date.
pub const MAX_OFFSET_DAYS: i32 = 10_000;

/// A day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    /// The week in order, Monday first.
    pub const ALL: [Weekday; 7] = [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ];

    /// Days since Monday: 0 for Monday to 6 for Sunday.
    pub fn days_from_monday(self) -> u8 {
        match self {
            Weekday::Monday => 0,
            Weekday::Tuesday => 1,
            Weekday::Wednesday => 2,
            Weekday::Thursday => 3,
            Weekday::Friday => 4,
            Weekday::Saturday => 5,
            Weekday::Sunday => 6,
        }
    }

    /// The English name, `Monday`.
    pub fn name(self) -> &'static str {
        match self {
            Weekday::Monday => "Monday",
            Weekday::Tuesday => "Tuesday",
            Weekday::Wednesday => "Wednesday",
            Weekday::Thursday => "Thursday",
            Weekday::Friday => "Friday",
            Weekday::Saturday => "Saturday",
            Weekday::Sunday => "Sunday",
        }
    }

    /// The three-letter English name, `Mon`.
    pub fn short_name(self) -> &'static str {
        &self.name()[..3]
    }

    /// The weekday an English word names, ignoring ASCII case: the full
    /// name, its first three letters, or `tues`, `thur` or `thurs`. Exact:
    /// `fr` and `fri!` name nothing.
    pub fn from_name(word: &str) -> Option<Self> {
        match word.to_ascii_lowercase().as_str() {
            "monday" | "mon" => Some(Weekday::Monday),
            "tuesday" | "tue" | "tues" => Some(Weekday::Tuesday),
            "wednesday" | "wed" => Some(Weekday::Wednesday),
            "thursday" | "thu" | "thur" | "thurs" => Some(Weekday::Thursday),
            "friday" | "fri" => Some(Weekday::Friday),
            "saturday" | "sat" => Some(Weekday::Saturday),
            "sunday" | "sun" => Some(Weekday::Sunday),
            _ => None,
        }
    }
}

/// A valid calendar date in the years 1 to 9999, the ones `YYYY-MM-DD`
/// spells. Ordered by time. Arithmetic that would leave those years
/// returns `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

impl Date {
    /// The date, or `None` when it is not one: month 13, 31 April, 29
    /// February outside a leap year, or a year outside 1 to 9999.
    pub fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        let valid = (1..=9999).contains(&year)
            && (1..=12).contains(&month)
            && (1..=month_length(year, month)).contains(&day);
        valid.then_some(Self { year, month, day })
    }

    /// Exactly `YYYY-MM-DD`, a valid date in ASCII digits; anything else,
    /// a sign, spaces or a missing zero included, is `None`.
    pub fn parse_iso(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let number = |range: std::ops::Range<usize>| -> Option<u16> {
            bytes[range].iter().try_fold(0u16, |value, byte| {
                byte.is_ascii_digit()
                    .then(|| value * 10 + u16::from(byte - b'0'))
            })
        };
        let month = u8::try_from(number(5..7)?).ok()?;
        let day = u8::try_from(number(8..10)?).ok()?;
        Self::new(number(0..4)?, month, day)
    }

    pub fn year(self) -> u16 {
        self.year
    }

    pub fn month(self) -> u8 {
        self.month
    }

    pub fn day(self) -> u8 {
        self.day
    }

    pub fn weekday(self) -> Weekday {
        // 1 January of year 1 was a Monday.
        Weekday::ALL[self.ordinal().rem_euclid(7) as usize]
    }

    /// Whether this date's year has a 29 February.
    pub fn is_leap_year(self) -> bool {
        is_leap_year(self.year)
    }

    /// The number of days in this date's month, 28 to 31.
    pub fn days_in_month(self) -> u8 {
        month_length(self.year, self.month)
    }

    /// The date `days` later (earlier when negative), or `None` past the
    /// years 1 to 9999.
    pub fn add_days(self, days: i32) -> Option<Self> {
        Self::from_ordinal(i64::from(self.ordinal()) + i64::from(days))
    }

    /// The same day `months` later (earlier when negative), clamped to the
    /// last day of a shorter month: 31 January plus one month is 28 or 29
    /// February. `None` past the years 1 to 9999.
    pub fn add_months(self, months: i32) -> Option<Self> {
        let index = i64::from(self.year) * 12 + i64::from(self.month) - 1 + i64::from(months);
        let year = u16::try_from(index.div_euclid(12)).ok()?;
        let month = u8::try_from(index.rem_euclid(12) + 1).ok()?;
        if !(1..=9999).contains(&year) {
            return None;
        }
        Self::new(year, month, self.day.min(month_length(year, month)))
    }

    /// The days from this date to `other`: positive when `other` is later.
    pub fn days_until(self, other: Self) -> i32 {
        other.ordinal() - self.ordinal()
    }

    /// The Monday that starts this date's week, the date itself on a
    /// Monday.
    pub fn monday_of_week(self) -> Self {
        let ordinal = self.ordinal();
        // Ordinal 0 is a Monday, so a week's Monday is never before it.
        Self::from_ordinal(i64::from(ordinal - ordinal.rem_euclid(7))).unwrap_or(self)
    }

    /// The short English form the status bar shows, `Mon 23 Nov`.
    pub fn short(self) -> Short {
        Short(self)
    }

    /// Days since 1 January of year 1, from 0 to 3 652 058.
    fn ordinal(self) -> i32 {
        let years = i32::from(self.year) - 1;
        let before_year = years * 365 + years / 4 - years / 100 + years / 400;
        let before_month: i32 = (1..self.month)
            .map(|month| i32::from(month_length(self.year, month)))
            .sum();
        before_year + before_month + i32::from(self.day) - 1
    }

    /// The date `ordinal` days after 1 January of year 1, or `None` outside
    /// the years 1 to 9999. Howard Hinnant's `civil_from_days`, counted
    /// from 1 March of year 0 so a leap day ends its year.
    fn from_ordinal(ordinal: i64) -> Option<Self> {
        const LAST: i64 = 3_652_058;
        if !(0..=LAST).contains(&ordinal) {
            return None;
        }
        // 1 January of year 1 is day 306 after 1 March of year 0.
        let days = ordinal + 306;
        let era = days / 146_097;
        let day_of_era = days - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_from_march = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
        let month = if month_from_march < 10 {
            month_from_march + 3
        } else {
            month_from_march - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        Self::new(
            u16::try_from(year).ok()?,
            u8::try_from(month).ok()?,
            u8::try_from(day).ok()?,
        )
    }
}

/// `YYYY-MM-DD`.
impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A date's short English form, from [`Date::short`]: the weekday, the day
/// with no leading zero and the month, `Tue 1 Dec`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Short(Date);

impl fmt::Display for Short {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let date = self.0;
        write!(
            f,
            "{} {} {}",
            date.weekday().short_name(),
            date.day,
            MONTHS[usize::from(date.month - 1)]
        )
    }
}

/// Whether `year` has a 29 February: every fourth year, except centuries
/// not divisible by 400 (2000 is a leap year, 2100 is not).
pub fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

/// The number of days in `month` of `year`, or `None` for a month outside
/// 1 to 12.
pub fn days_in_month(year: u16, month: u8) -> Option<u8> {
    (1..=12).contains(&month).then(|| month_length(year, month))
}

fn month_length(year: u16, month: u8) -> u8 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// What [`parse`] makes of a typed date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// A date.
    Date(Date),
    /// `YYYY-MM-DD` in shape but no real date (`2026-02-30`,
    /// `2026-13-01`), kept as written and shown as a problem rather than
    /// dropped.
    Invalid,
}

impl Parsed {
    /// What text in the shape `YYYY-MM-DD` (four, two and two ASCII digits
    /// joined by hyphens) names: its date, or [`Parsed::Invalid`] when it
    /// is no real one. `None` for text in any other shape, spaces around
    /// it included.
    pub fn from_iso(text: &str) -> Option<Self> {
        iso_shaped(text).then(|| Date::parse_iso(text).map_or(Self::Invalid, Self::Date))
    }
}

/// The date a typed word or ISO date names, relative to `today`, or `None`
/// when the text is not a date at all. Text is trimmed and words ignore
/// ASCII case. English only, and weeks start on Monday.
///
/// - `today`, `tomorrow`.
/// - A weekday name ([`Weekday::from_name`]): the next one strictly after
///   today, so a name equal to today's means a week on.
/// - `next <weekday>`: that day in the week after the current one (weeks
///   run Monday to Sunday), never further: `next mon` is `next week`. On
///   Thursday 19 November 2026, `fri` is the 20th and `next fri` the 27th.
///   Where the weekday name alone already lands in next week (`mon` on any
///   day, `fri` on a Saturday, any name on a Sunday), the two agree: on
///   Saturday the 21st, `fri` and `next fri` are both the 27th.
/// - `next week`: the Monday of the week after the current one.
/// - `+Nd`, `+Nw`, `+Nm`: N ASCII digits of days, weeks or months on;
///   `+0d` is today. Months keep the day, clamped to a shorter month's last
///   (31 January `+1m` is the end of February). A result more than
///   [`MAX_OFFSET_DAYS`] away is not a date.
/// - `YYYY-MM-DD`: that date, or [`Parsed::Invalid`] when it is the shape
///   of one but no real date.
///
/// Slash dates (`19/11/2026`, `11/19`) are refused because their order is
/// ambiguous, and so is anything else: no prefixes (`fr`), no punctuation
/// (`fri!`), no joined words (`nextfri`). A word that would land outside
/// the years 1 to 9999 is not a date either.
pub fn parse(text: &str, today: Date) -> Option<Parsed> {
    let text = text.trim();
    Parsed::from_iso(text).or_else(|| parse_word(text, today).map(Parsed::Date))
}

/// Four, two and two ASCII digits joined by hyphens.
fn iso_shaped(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

fn parse_word(text: &str, today: Date) -> Option<Date> {
    let lower = text.to_ascii_lowercase();
    if let Some(offset) = lower.strip_prefix('+') {
        return parse_offset(offset, today);
    }
    let mut words = lower.split_whitespace();
    let first = words.next()?;
    let second = words.next();
    if words.next().is_some() {
        return None;
    }
    match (first, second) {
        ("today", None) => Some(today),
        ("tomorrow", None) => today.add_days(1),
        ("next", Some("week")) => today.monday_of_week().add_days(7),
        ("next", Some(name)) => {
            let weekday = Weekday::from_name(name)?;
            today
                .monday_of_week()
                .add_days(7 + i32::from(weekday.days_from_monday()))
        }
        (name, None) => {
            let weekday = i32::from(Weekday::from_name(name)?.days_from_monday());
            let now = i32::from(today.weekday().days_from_monday());
            let ahead = (weekday - now - 1).rem_euclid(7) + 1;
            today.add_days(ahead)
        }
        _ => None,
    }
}

/// `Nd`, `Nw` or `Nm` after the `+`, already lowercase.
fn parse_offset(offset: &str, today: Date) -> Option<Date> {
    let unit = offset.chars().last()?;
    let digits = &offset[..offset.len() - unit.len_utf8()];
    if digits.is_empty() {
        return None;
    }
    let count = digits.bytes().try_fold(0i32, |value, byte| {
        if !byte.is_ascii_digit() {
            return None;
        }
        let value = value.checked_mul(10)?.checked_add(i32::from(byte - b'0'))?;
        (value <= MAX_OFFSET_DAYS).then_some(value)
    })?;
    let date = match unit {
        'd' => today.add_days(count),
        'w' => today.add_days(count.checked_mul(7)?),
        'm' => today.add_months(count),
        _ => None,
    }?;
    (today.days_until(date) <= MAX_OFFSET_DAYS).then_some(date)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> Date {
        Date::parse_iso(text).unwrap_or_else(|| panic!("{text} is a date"))
    }

    #[test]
    fn iso_dates_round_trip() {
        for text in [
            "2026-11-19",
            "0001-01-01",
            "9999-12-31",
            "2024-02-29",
            "2000-02-29",
            "2026-01-09",
        ] {
            assert_eq!(date(text).to_string(), text);
        }
        let day = Date::new(2026, 11, 19).unwrap();
        assert_eq!((day.year(), day.month(), day.day()), (2026, 11, 19));
        assert_eq!(date("2026-11-19"), day);
    }

    #[test]
    fn invalid_dates_are_refused() {
        for (year, month, day) in [
            (2026, 2, 29),
            (1900, 2, 29),
            (2026, 4, 31),
            (2026, 13, 1),
            (2026, 0, 1),
            (2026, 1, 0),
            (2026, 12, 32),
            (0, 1, 1),
            (10000, 1, 1),
        ] {
            assert_eq!(Date::new(year, month, day), None, "{year}-{month}-{day}");
        }
        for text in [
            "",
            "2026-11-1",
            "2026-1-19",
            "26-11-19",
            "2026/11/19",
            "2026-11-19 ",
            " 2026-11-19",
            "+026-11-19",
            "2026-11-+9",
            "2026-02-30",
            "2026-00-10",
            "0000-01-01",
            "２０２６-11-19",
            "2026-11-19x",
            "11/19/2026",
        ] {
            assert_eq!(Date::parse_iso(text), None, "{text:?}");
        }
    }

    #[test]
    fn weekdays_follow_the_calendar() {
        assert_eq!(date("2026-11-19").weekday(), Weekday::Thursday);
        for (text, weekday) in [
            ("0001-01-01", Weekday::Monday),
            ("1970-01-01", Weekday::Thursday),
            ("2000-02-29", Weekday::Tuesday),
            ("2024-02-29", Weekday::Thursday),
            ("2026-11-23", Weekday::Monday),
            ("2026-11-22", Weekday::Sunday),
            ("2026-12-31", Weekday::Thursday),
            ("2027-01-01", Weekday::Friday),
            ("9999-12-31", Weekday::Friday),
        ] {
            assert_eq!(date(text).weekday(), weekday, "{text}");
        }
    }

    #[test]
    fn dates_order_by_time() {
        assert!(date("2026-11-19") < date("2026-11-20"));
        assert!(date("2026-11-30") < date("2026-12-01"));
        assert!(date("2025-12-31") < date("2026-01-01"));
    }

    #[test]
    fn weekday_names() {
        for (index, weekday) in Weekday::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(weekday.days_from_monday()), index);
            assert_eq!(Weekday::from_name(weekday.name()), Some(weekday));
            assert_eq!(Weekday::from_name(weekday.short_name()), Some(weekday));
        }
        for (word, weekday) in [
            ("tues", Weekday::Tuesday),
            ("TUE", Weekday::Tuesday),
            ("thur", Weekday::Thursday),
            ("Thurs", Weekday::Thursday),
            ("wEdNeSdAy", Weekday::Wednesday),
        ] {
            assert_eq!(Weekday::from_name(word), Some(weekday), "{word}");
        }
        for word in ["", "fr", "fri!", "frid", "th", "thursd", "mondays", " mon"] {
            assert_eq!(Weekday::from_name(word), None, "{word:?}");
        }
    }

    /// Each word against a day: (word, the date it names).
    fn assert_words(today: &str, table: &[(&str, &str)]) {
        let today = date(today);
        for (word, expected) in table {
            assert_eq!(
                parse(word, today),
                Some(Parsed::Date(date(expected))),
                "{word:?} on {today}"
            );
        }
    }

    #[test]
    fn words_on_the_pinned_thursday() {
        assert_words(
            "2026-11-19",
            &[
                ("today", "2026-11-19"),
                ("tomorrow", "2026-11-20"),
                ("mon", "2026-11-23"),
                ("monday", "2026-11-23"),
                ("tue", "2026-11-24"),
                ("tues", "2026-11-24"),
                ("tuesday", "2026-11-24"),
                ("wed", "2026-11-25"),
                ("wednesday", "2026-11-25"),
                ("thu", "2026-11-26"),
                ("thur", "2026-11-26"),
                ("thurs", "2026-11-26"),
                ("thursday", "2026-11-26"),
                ("fri", "2026-11-20"),
                ("friday", "2026-11-20"),
                ("sat", "2026-11-21"),
                ("saturday", "2026-11-21"),
                ("sun", "2026-11-22"),
                ("sunday", "2026-11-22"),
                ("next mon", "2026-11-23"),
                ("next tue", "2026-11-24"),
                ("next wed", "2026-11-25"),
                ("next thu", "2026-11-26"),
                ("next fri", "2026-11-27"),
                ("next friday", "2026-11-27"),
                ("next sat", "2026-11-28"),
                ("next sun", "2026-11-29"),
                ("next week", "2026-11-23"),
                ("+0d", "2026-11-19"),
                ("+1d", "2026-11-20"),
                ("+3d", "2026-11-22"),
                ("+12d", "2026-12-01"),
                ("+007d", "2026-11-26"),
                ("+0w", "2026-11-19"),
                ("+1w", "2026-11-26"),
                ("+2w", "2026-12-03"),
                ("+0m", "2026-11-19"),
                ("+1m", "2026-12-19"),
                ("+2m", "2027-01-19"),
                ("+12m", "2027-11-19"),
                ("+10000d", "2054-04-06"),
                ("2026-12-01", "2026-12-01"),
                ("2020-01-01", "2020-01-01"),
                // Trimmed, any ASCII case, any space between words.
                ("  Today ", "2026-11-19"),
                ("TOMORROW", "2026-11-20"),
                ("Fri", "2026-11-20"),
                ("NEXT  Fri", "2026-11-27"),
                ("next\tweek", "2026-11-23"),
                ("+3D", "2026-11-22"),
                ("+2W", "2026-12-03"),
                ("\t2026-12-01\n", "2026-12-01"),
            ],
        );
    }

    #[test]
    fn words_from_a_monday_a_saturday_and_a_sunday() {
        assert_words(
            "2026-11-16",
            &[
                ("today", "2026-11-16"),
                ("tomorrow", "2026-11-17"),
                ("mon", "2026-11-23"),
                ("tue", "2026-11-17"),
                ("fri", "2026-11-20"),
                ("sun", "2026-11-22"),
                ("next mon", "2026-11-23"),
                ("next fri", "2026-11-27"),
                ("next sun", "2026-11-29"),
                ("next week", "2026-11-23"),
                ("+1w", "2026-11-23"),
            ],
        );
        // Saturday: Friday of next week is the 27th, as plain `fri` is.
        assert_words(
            "2026-11-21",
            &[
                ("today", "2026-11-21"),
                ("tomorrow", "2026-11-22"),
                ("sun", "2026-11-22"),
                ("mon", "2026-11-23"),
                ("fri", "2026-11-27"),
                ("sat", "2026-11-28"),
                ("next mon", "2026-11-23"),
                ("next fri", "2026-11-27"),
                ("next sat", "2026-11-28"),
                ("next sun", "2026-11-29"),
                ("next week", "2026-11-23"),
            ],
        );
        assert_words(
            "2026-11-22",
            &[
                ("today", "2026-11-22"),
                ("tomorrow", "2026-11-23"),
                ("mon", "2026-11-23"),
                ("fri", "2026-11-27"),
                ("sat", "2026-11-28"),
                ("sun", "2026-11-29"),
                ("next mon", "2026-11-23"),
                ("next fri", "2026-11-27"),
                ("next sun", "2026-11-29"),
                ("next week", "2026-11-23"),
            ],
        );
    }

    #[test]
    fn words_across_month_and_year_ends() {
        assert_words(
            "2026-01-31",
            &[
                ("+1m", "2026-02-28"),
                ("+2m", "2026-03-31"),
                ("+13m", "2027-02-28"),
            ],
        );
        assert_words("2028-01-31", &[("+1m", "2028-02-29")]);
        assert_words(
            "2026-03-31",
            &[
                ("+1m", "2026-04-30"),
                ("+3m", "2026-06-30"),
                ("+1d", "2026-04-01"),
            ],
        );
        assert_words(
            "2026-12-31",
            &[
                ("tomorrow", "2027-01-01"),
                ("+1d", "2027-01-01"),
                ("+1m", "2027-01-31"),
                ("fri", "2027-01-01"),
                ("next week", "2027-01-04"),
                ("next fri", "2027-01-08"),
            ],
        );
        // Tuesday 29 December 2026: next week's Monday is in 2027.
        assert_words("2026-12-29", &[("next week", "2027-01-04")]);
        assert_words(
            "2028-02-28",
            &[("tomorrow", "2028-02-29"), ("+2d", "2028-03-01")],
        );
        assert_words(
            "2028-02-29",
            &[("+1m", "2028-03-29"), ("+12m", "2029-02-28")],
        );
    }

    #[test]
    fn invalid_iso_dates_are_kept_as_invalid() {
        let today = date("2026-11-19");
        for text in [
            "2026-02-30",
            "2026-13-01",
            "2026-00-10",
            "2026-04-31",
            "2027-02-29",
            "2100-02-29",
            "0000-01-01",
            " 2026-02-30 ",
        ] {
            assert_eq!(parse(text, today), Some(Parsed::Invalid), "{text:?}");
        }
    }

    #[test]
    fn the_iso_shape_alone_reads_as_a_date_or_as_invalid() {
        for (text, expected) in [
            ("2026-11-19", Some(Parsed::Date(date("2026-11-19")))),
            ("2026-02-30", Some(Parsed::Invalid)),
            ("0000-00-00", Some(Parsed::Invalid)),
            (" 2026-11-19", None),
            ("2026-11-19 ", None),
            ("2026-1-19", None),
            ("2026/11/19", None),
            ("２０２６-11-19", None),
            ("tomorrow", None),
            ("today", None),
            ("", None),
        ] {
            assert_eq!(Parsed::from_iso(text), expected, "{text:?}");
        }
    }

    #[test]
    fn other_text_is_not_a_date() {
        let today = date("2026-11-19");
        for text in [
            "",
            "   ",
            "19/11/2026",
            "11/19",
            "11/19/2026",
            "2026/11/19",
            "fri!",
            "fr",
            "nextfri",
            "next",
            "next today",
            "next tomorrow",
            "next fri sat",
            "next weeks",
            "today tomorrow",
            "yesterday",
            "+",
            "+d",
            "+3",
            "+3y",
            "+-3d",
            "+ 3d",
            "++3d",
            "3d",
            "-3d",
            "+3 d",
            "+３d",
            "+3dd",
            "+10001d",
            "+1429w",
            "+329m",
            "+99999999999999999999d",
            "2026-11-1",
            "2026-1-19",
            "２０２６-11-19",
            "2026-11-19x",
            "+é",
        ] {
            assert_eq!(parse(text, today), None, "{text:?}");
        }
        assert_eq!(
            parse("+1428w", today),
            Some(Parsed::Date(date("2054-04-02")))
        );
        assert_eq!(
            parse("+328m", today),
            Some(Parsed::Date(date("2054-03-19")))
        );
    }

    #[test]
    fn words_past_the_years_are_not_dates() {
        let last = date("9999-12-31");
        for word in [
            "tomorrow",
            "+1d",
            "+1w",
            "+1m",
            "mon",
            "fri",
            "next week",
            "next fri",
        ] {
            assert_eq!(parse(word, last), None, "{word}");
        }
        assert_eq!(parse("today", last), Some(Parsed::Date(last)));
        assert_eq!(parse("+0d", last), Some(Parsed::Date(last)));
        assert_eq!(
            parse("9999-12-31", date("0001-01-01")),
            Some(Parsed::Date(last))
        );
    }

    #[test]
    fn arithmetic_at_the_ends() {
        let first = date("0001-01-01");
        let last = date("9999-12-31");
        assert_eq!(first.add_days(-1), None);
        assert_eq!(last.add_days(1), None);
        assert_eq!(first.add_days(i32::MIN), None);
        assert_eq!(last.add_days(i32::MAX), None);
        assert_eq!(first.add_days(i32::MAX), None);
        assert_eq!(first.add_days(3_652_058), Some(last));
        assert_eq!(last.add_days(-3_652_058), Some(first));
        assert_eq!(first.days_until(last), 3_652_058);
        assert_eq!(last.days_until(first), -3_652_058);
        assert_eq!(first.add_months(-1), None);
        assert_eq!(last.add_months(1), None);
        assert_eq!(first.add_months(i32::MIN), None);
        assert_eq!(last.add_months(i32::MAX), None);
        assert_eq!(first.add_months(9999 * 12 - 1), Some(date("9999-12-01")));
        assert_eq!(last.add_months(-(9999 * 12 - 1)), Some(date("0001-01-31")));
        assert_eq!(first.monday_of_week(), first);
        assert_eq!(last.monday_of_week(), date("9999-12-27"));
        assert_eq!(date("2026-12-31").add_days(1), Some(date("2027-01-01")));
        assert_eq!(date("2027-01-01").add_days(-1), Some(date("2026-12-31")));
        assert_eq!(date("2026-03-31").add_months(-1), Some(date("2026-02-28")));
        assert_eq!(date("2026-01-15").add_months(-13), Some(date("2024-12-15")));
        assert_eq!(date("2026-11-19").monday_of_week(), date("2026-11-16"));
        assert_eq!(date("2026-11-22").monday_of_week(), date("2026-11-16"));
        assert_eq!(date("2026-11-23").monday_of_week(), date("2026-11-23"));
        assert_eq!(date("2027-01-01").monday_of_week(), date("2026-12-28"));
    }

    #[test]
    fn leap_years_and_month_lengths() {
        for (year, leap) in [
            (2024, true),
            (2026, false),
            (2028, true),
            (2000, true),
            (2100, false),
            (1900, false),
            (2400, true),
            (4, true),
            (1, false),
        ] {
            assert_eq!(is_leap_year(year), leap, "{year}");
            assert_eq!(Date::new(year, 2, 29).is_some(), leap, "{year}");
        }
        assert!(date("2028-02-29").is_leap_year());
        assert_eq!(date("2028-02-01").days_in_month(), 29);
        assert_eq!(date("2100-02-01").days_in_month(), 28);
        assert_eq!(date("2000-02-01").days_in_month(), 29);
        assert_eq!(date("2026-04-10").days_in_month(), 30);
        assert_eq!(date("2026-12-10").days_in_month(), 31);
        assert_eq!(days_in_month(2026, 2), Some(28));
        assert_eq!(days_in_month(2026, 0), None);
        assert_eq!(days_in_month(2026, 13), None);
        assert_eq!(date("2028-02-29").add_days(1), Some(date("2028-03-01")));
        assert_eq!(date("2100-02-28").add_days(1), Some(date("2100-03-01")));
        assert_eq!(date("2000-02-28").add_days(1), Some(date("2000-02-29")));
    }

    #[test]
    fn short_form() {
        assert_eq!(date("2026-11-23").short().to_string(), "Mon 23 Nov");
        assert_eq!(date("2026-11-19").short().to_string(), "Thu 19 Nov");
        assert_eq!(date("2026-12-01").short().to_string(), "Tue 1 Dec");
        assert_eq!(date("2027-01-03").short().to_string(), "Sun 3 Jan");
    }

    /// Every day of 2024 to 2028: the next day is one on and one weekday
    /// later, and adding days round trips through `days_until`.
    #[test]
    fn days_step_through_2024_to_2028() {
        let mut day = date("2024-01-01");
        let end = date("2028-12-31");
        let mut seen = 0;
        while day <= end {
            let next = day.add_days(1).unwrap();
            assert!(next > day);
            assert_eq!(day.days_until(next), 1);
            let after = (day.weekday().days_from_monday() + 1) % 7;
            assert_eq!(next.weekday().days_from_monday(), after, "{day}");
            assert_eq!(Date::parse_iso(&day.to_string()), Some(day));
            for offset in [-400, -31, -7, -1, 0, 1, 7, 29, 366, 1000] {
                let moved = day.add_days(offset).unwrap();
                assert_eq!(day.days_until(moved), offset, "{day} {offset}");
                assert_eq!(moved.add_days(-offset), Some(day));
            }
            let monday = day.monday_of_week();
            assert_eq!(monday.weekday(), Weekday::Monday);
            assert!((0..7).contains(&monday.days_until(day)));
            let month = day.add_months(1).unwrap();
            assert_eq!(month.day(), day.day().min(month.days_in_month()));
            seen += 1;
            day = next;
        }
        assert_eq!(seen, 366 + 365 + 365 + 365 + 366);
    }

    /// Ordinals and dates agree over the whole range, both ways.
    #[test]
    fn ordinals_round_trip_over_every_date() {
        let mut expected = date("0001-01-01");
        for ordinal in 0..=3_652_058 {
            let day = Date::from_ordinal(ordinal).unwrap();
            assert_eq!(day, expected, "{ordinal}");
            assert_eq!(i64::from(day.ordinal()), ordinal);
            expected = match (day.day() < day.days_in_month(), day.month() < 12) {
                (true, _) => Date::new(day.year(), day.month(), day.day() + 1).unwrap(),
                (false, true) => Date::new(day.year(), day.month() + 1, 1).unwrap(),
                (false, false) if day.year() < 9999 => Date::new(day.year() + 1, 1, 1).unwrap(),
                (false, false) => break,
            };
        }
        assert_eq!(Date::from_ordinal(-1), None);
        assert_eq!(Date::from_ordinal(3_652_059), None);
    }
}
