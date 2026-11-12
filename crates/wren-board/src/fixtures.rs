//! The committed board files the format tests read: the golden files of
//! `golden/format/` and the boards of the sample workspace. They are
//! written by `tools/board_fixtures.py`, never by hand. Also the seeded
//! generator the property tests share.

use std::path::PathBuf;

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

/// The path of a golden format file by its fixture name, or `None` for a
/// sample board, which only `tools/board_fixtures.py` writes.
pub fn golden_path(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("golden/format")
        .join(format!("{name}.wren-board"));
    path.exists().then_some(path)
}

/// Asserts that `actual` is the committed `expected`. With `WREN_BLESS=1`
/// a golden file is rewritten instead, and the change is reviewed as a
/// diff.
#[track_caller]
pub fn assert_golden(name: &str, expected: &str, actual: &str) {
    if actual == expected {
        return;
    }
    if std::env::var_os("WREN_BLESS").is_some_and(|v| v == "1") {
        if let Some(path) = golden_path(name) {
            if let Err(error) = std::fs::write(&path, actual) {
                panic!("{}: {error}", path.display());
            }
            return;
        }
    }
    let first = expected
        .lines()
        .zip(actual.lines())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
    panic!(
        "{name}: written bytes differ from the golden file, first at line {}\n\
         --- golden\n{expected}--- written\n{actual}",
        first + 1
    );
}

/// SplitMix64: a seeded source for the property tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `0..n`, or 0 when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    pub fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}
