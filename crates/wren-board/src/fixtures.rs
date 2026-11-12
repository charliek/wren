//! The committed board files the format tests read: the golden files of
//! `golden/format/` and the boards of the sample workspace. They are
//! written by `tools/board_fixtures.py`, never by hand.

macro_rules! golden {
    ($name:expr) => {
        (
            $name,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/golden/format/",
                $name,
                ".wren-board"
            )),
        )
    };
}

macro_rules! sample {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tools/wrentest/fixtures/boards/workspace/Boards/Samples/",
                $name,
                ".wren-board"
            )),
        )
    };
}

macro_rules! pair {
    ($name:literal) => {
        (
            $name,
            golden!(concat!($name, ".in")).1,
            golden!(concat!($name, ".out")).1,
        )
    };
}

/// Canonical files: each is its own canonical form.
pub const CANONICAL: &[(&str, &str)] = &[
    golden!("password-reset.out"),
    golden!("fields.out"),
    golden!("shapes-and-ends.out"),
    golden!("escapes.out"),
    golden!("unknown.out"),
    golden!("quoted-keys.out"),
    golden!("values.out"),
    golden!("requires.out"),
    golden!("future.out"),
    golden!("empty.out"),
    sample!("Checkout flow"),
    sample!("Shapes"),
    sample!("Overlap"),
    sample!("Future"),
];

/// Non-canonical inputs, each with the canonical file it writes to.
pub const PAIRS: &[(&str, &str, &str)] = &[
    pair!("empty"),
    pair!("blank"),
    pair!("comments-only"),
    pair!("crlf"),
    pair!("bom"),
    pair!("comments"),
    pair!("shuffled"),
    pair!("subtables"),
    pair!("strings"),
    pair!("toml11"),
    pair!("defaults"),
    pair!("colour-case"),
];

/// A valid board that is not canonical: comments, unsorted blocks, odd
/// spacing.
pub const HAND_WRITTEN: (&str, &str) = sample!("Hand written");

/// Every committed board file, canonical or not.
pub fn all() -> impl Iterator<Item = (&'static str, &'static str)> {
    CANONICAL
        .iter()
        .copied()
        .chain(PAIRS.iter().map(|&(name, input, _)| (name, input)))
        .chain([HAND_WRITTEN])
}
