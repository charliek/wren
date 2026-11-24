//! Civil dates, as task lines write them: a year, a month and a day of the
//! proleptic Gregorian calendar, with no time and no zone.

use std::fmt;

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

/// A valid calendar date in the years 1 to 9999, the ones `YYYY-MM-DD`
/// spells. Ordered by time.
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
            && (1..=days_in_month(year, month)).contains(&day);
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
        // Days since 1 January of year 1, a Monday.
        const WEEK: [Weekday; 7] = [
            Weekday::Monday,
            Weekday::Tuesday,
            Weekday::Wednesday,
            Weekday::Thursday,
            Weekday::Friday,
            Weekday::Saturday,
            Weekday::Sunday,
        ];
        WEEK[(self.days_from_epoch() % 7) as usize]
    }

    fn days_from_epoch(self) -> u32 {
        let years = u32::from(self.year) - 1;
        let before_year = years * 365 + years / 4 - years / 100 + years / 400;
        let before_month: u32 = (1..self.month)
            .map(|month| u32::from(days_in_month(self.year, month)))
            .sum();
        before_year + before_month + u32::from(self.day) - 1
    }
}

/// `YYYY-MM-DD`.
impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn is_leap(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
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
}
