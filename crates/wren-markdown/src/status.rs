//! Task statuses: the table a box's character is read through.
//!
//! A status has a symbol (the character in the box), a name, a type and the
//! symbol the done toggle writes in its place, as in Obsidian Tasks. Filters,
//! toggling and rollups work on the type, so a status such as on hold or
//! waiting is another row of [`STATUSES`], not another case in the code that
//! reads them.

/// What a status counts as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusType {
    Open,
    Doing,
    Done,
    Cancelled,
}

impl StatusType {
    /// The type's name as Markdown tools spell it, in lower case.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Doing => "doing",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    /// Work still to do: an open or a doing task.
    pub const fn is_unfinished(self) -> bool {
        matches!(self, Self::Open | Self::Doing)
    }

    /// Work no longer to do: a done or a cancelled task.
    pub const fn is_finished(self) -> bool {
        !self.is_unfinished()
    }
}

/// One row of the status table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Status {
    /// The character between the box's brackets.
    pub symbol: char,
    pub name: &'static str,
    pub kind: StatusType,
    /// The symbol the done toggle writes in place of this one.
    pub next: char,
}

/// The symbol of the status a new or reopened task gets.
pub const OPEN: char = ' ';

/// The symbol the done toggle writes on an unfinished task.
pub const DONE: char = 'x';

/// The statuses Wren knows, in the order a menu lists them.
pub const STATUSES: [Status; 4] = [
    Status {
        symbol: OPEN,
        name: "Open",
        kind: StatusType::Open,
        next: DONE,
    },
    Status {
        symbol: '/',
        name: "Doing",
        kind: StatusType::Doing,
        next: DONE,
    },
    Status {
        symbol: DONE,
        name: "Done",
        kind: StatusType::Done,
        next: OPEN,
    },
    Status {
        symbol: '-',
        name: "Cancelled",
        kind: StatusType::Cancelled,
        next: OPEN,
    },
];

/// The name of a status whose symbol has no row.
pub const UNKNOWN: &str = "Unknown";

/// The status a box holding `symbol` has. `X` reads as `x`. A symbol with
/// no row is an open task that keeps its own character.
pub fn status(symbol: char) -> Status {
    let known = if symbol == 'X' { DONE } else { symbol };
    STATUSES
        .iter()
        .copied()
        .find(|status| status.symbol == known)
        .unwrap_or(Status {
            symbol,
            name: UNKNOWN,
            kind: StatusType::Open,
            next: DONE,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_statuses_read_by_their_symbols() {
        for (symbol, name, kind, next) in [
            (' ', "Open", StatusType::Open, 'x'),
            ('/', "Doing", StatusType::Doing, 'x'),
            ('x', "Done", StatusType::Done, ' '),
            ('X', "Done", StatusType::Done, ' '),
            ('-', "Cancelled", StatusType::Cancelled, ' '),
        ] {
            let status = status(symbol);
            assert_eq!(
                (status.name, status.kind, status.next),
                (name, kind, next),
                "{symbol:?}"
            );
        }
        assert_eq!(status('X').symbol, 'x', "a capital X is the done row");
    }

    #[test]
    fn an_unknown_symbol_is_an_open_task_with_its_own_character() {
        for symbol in [
            'a', '1', '?', '!', '>', '<', '[', '界', 'é', '✓', '\t', '\u{301}', '\u{fe0f}', 'ｘ',
        ] {
            assert_eq!(
                status(symbol),
                Status {
                    symbol,
                    name: "Unknown",
                    kind: StatusType::Open,
                    next: 'x',
                },
                "{symbol:?}"
            );
        }
    }

    /// The table is data the rest of the code can rely on: one row per
    /// symbol, each `next` a row of its own, and the done toggle's two
    /// targets are the open and the done rows.
    #[test]
    fn the_table_is_closed_under_next_and_has_one_row_per_symbol() {
        for (index, row) in STATUSES.iter().enumerate() {
            assert!(
                STATUSES[..index]
                    .iter()
                    .all(|other| other.symbol != row.symbol),
                "{:?} has two rows",
                row.symbol
            );
            assert_eq!(status(row.symbol), *row);
            let next = status(row.next);
            assert_ne!(next.name, UNKNOWN, "{:?} toggles to no row", row.symbol);
            assert_eq!(
                next.kind,
                if row.kind.is_unfinished() {
                    StatusType::Done
                } else {
                    StatusType::Open
                },
                "{:?}",
                row.symbol
            );
        }
        assert_eq!(status(OPEN).kind, StatusType::Open);
        assert_eq!(status(DONE).kind, StatusType::Done);
    }

    #[test]
    fn every_type_is_unfinished_or_finished_and_has_a_name() {
        for (kind, name, finished) in [
            (StatusType::Open, "open", false),
            (StatusType::Doing, "doing", false),
            (StatusType::Done, "done", true),
            (StatusType::Cancelled, "cancelled", true),
        ] {
            assert_eq!(kind.name(), name);
            assert_eq!(kind.is_finished(), finished);
            assert_eq!(kind.is_unfinished(), !finished);
        }
    }
}
