//! Where new element ids come from (§4.2 of the board plan).

use std::collections::BTreeSet;
use std::hash::{BuildHasher, Hasher, RandomState};

use crate::{Board, Id};

/// The characters of a generated id: digits and lower-case letters without
/// `i`, `l`, `o` and `u`, five bits each.
pub const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The length of a generated id: 40 bits.
pub const LEN: usize = 8;

/// How many draws [`fresh`] makes before it stops trusting the source.
const DRAWS: usize = 64;

/// A source of candidate ids for the elements a plan creates. A candidate
/// is 8 characters of `0123456789abcdefghjkmnpqrstvwxyz` (40 bits); the
/// plan draws again while one is taken by the board or by the plan itself.
/// The app draws from `std::hash::RandomState`; a test supplies its own
/// ids, or a seeded source.
pub trait Ids {
    fn draw(&mut self) -> Id;
}

/// Candidates from `std::hash::RandomState`: the process's random hash
/// keys applied to a counter.
#[derive(Clone, Debug, Default)]
pub struct Random {
    keys: RandomState,
    count: u64,
}

impl Random {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Ids for Random {
    fn draw(&mut self) -> Id {
        let mut hasher = self.keys.build_hasher();
        hasher.write_u64(self.count);
        self.count = self.count.wrapping_add(1);
        from_bits(hasher.finish())
    }
}

/// Candidates from a seed, the same on every run: for tests, and for
/// anything else that must repeat.
#[derive(Clone, Debug)]
pub struct Seeded(u64);

impl Seeded {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
}

impl Ids for Seeded {
    fn draw(&mut self) -> Id {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        from_bits(z ^ (z >> 31))
    }
}

/// The generated id spelling the low 40 bits of `bits`, most significant
/// first.
pub fn from_bits(bits: u64) -> Id {
    let text: String = (0..LEN)
        .rev()
        .map(|k| char::from(ALPHABET[((bits >> (5 * k)) & 31) as usize]))
        .collect();
    Id(text)
}

/// A new id from `ids` that names no node and no connector of `board` and
/// is not in `taken`, the ids the plan being built has already used.
///
/// A source that keeps returning taken ids is not drawn from forever:
/// after 64 draws the ids that follow its last one are tried in order,
/// which ends within one more than the number of ids in use.
pub fn fresh(board: &Board, taken: &BTreeSet<Id>, ids: &mut dyn Ids) -> Id {
    let used = |id: &Id| {
        board.nodes.contains_key(id) || board.edges.contains_key(id) || taken.contains(id)
    };
    let mut last = None;
    for _ in 0..DRAWS {
        let id = ids.draw();
        if !used(&id) {
            return id;
        }
        last = Some(id);
    }
    let mut bits = last.as_ref().and_then(bits_of).unwrap_or(0);
    loop {
        bits = bits.wrapping_add(1) & ((1 << (5 * LEN)) - 1);
        let id = from_bits(bits);
        if !used(&id) {
            return id;
        }
    }
}

/// The 40 bits a generated id spells, or `None` for any other id.
fn bits_of(id: &Id) -> Option<u64> {
    let text = id.as_str().as_bytes();
    if text.len() != LEN {
        return None;
    }
    text.iter().try_fold(0, |bits, &c| {
        let digit = ALPHABET.iter().position(|&a| a == c)?;
        Some(bits << 5 | digit as u64)
    })
}

/// Whether `text` is a generated id: 8 characters of [`ALPHABET`].
pub fn is_generated(text: &str) -> bool {
    text.len() == LEN && text.bytes().all(|c| ALPHABET.contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, Edge, Node};

    fn node() -> Node {
        let board = parse("schema = 1\n[node.a]\nkind = \"shape\"\nat = [0, 0]\nsize = [1, 1]\n");
        board.unwrap().nodes["a"].clone()
    }

    fn edge() -> Edge {
        let board = parse(
            "schema = 1\n[edge.a]\nkind = \"edge\"\nfrom = { at = [0, 0] }\nto = { at = [1, 1] }\n",
        );
        board.unwrap().edges["a"].clone()
    }

    /// A board as full as Phase 3's cap allows, 50,000 elements, holding
    /// the first `seeded` ids `Seeded::new(seed)` would draw, half as nodes
    /// and half as connectors, so that drawing from it collides at once.
    fn full_board(seed: u64, seeded: usize) -> Board {
        let mut board = Board::default();
        let mut source = Seeded::new(seed);
        for k in 0..50_000 {
            let id = if k < seeded {
                source.draw()
            } else {
                Id::new(&format!("n{k}")).unwrap()
            };
            if k % 2 == 0 {
                board.nodes.insert(id, node());
            } else {
                board.edges.insert(id, edge());
            }
        }
        board
    }

    /// Draws `count` ids for one plan and checks each.
    fn draws_hold(board: &Board, ids: &mut dyn Ids, count: usize) {
        let mut taken = BTreeSet::new();
        for _ in 0..count {
            let id = fresh(board, &taken, ids);
            assert!(is_generated(id.as_str()), "{id}");
            assert!(!board.nodes.contains_key(&id) && !board.edges.contains_key(&id));
            assert!(taken.insert(id.clone()), "{id} drawn twice");
        }
    }

    #[test]
    fn draws_against_a_full_board_never_collide() {
        let board = full_board(7, 1_000);
        draws_hold(&board, &mut Seeded::new(7), 2_000);
        draws_hold(&board, &mut Random::new(), 2_000);
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_draws_against_a_full_board_never_collide() {
        let board = full_board(11, 20_000);
        draws_hold(&board, &mut Seeded::new(11), 100_000);
        draws_hold(&board, &mut Random::new(), 100_000);
    }

    /// Draws from only 1,024 ids.
    struct Narrow(Seeded);

    impl Ids for Narrow {
        fn draw(&mut self) -> Id {
            let drawn = self.0.draw();
            from_bits(bits_of(&drawn).unwrap() & 0x3ff)
        }
    }

    #[test]
    fn a_source_that_keeps_colliding_still_gives_unused_ids() {
        let mut board = Board::default();
        for bits in 0..1_000 {
            board.nodes.insert(from_bits(bits), node());
        }
        draws_hold(&board, &mut Narrow(Seeded::new(3)), 2_000);
    }

    /// Always the same id.
    struct Stuck(&'static str);

    impl Ids for Stuck {
        fn draw(&mut self) -> Id {
            Id::new(self.0).unwrap()
        }
    }

    #[test]
    fn a_stuck_source_ends() {
        let mut board = Board::default();
        board.nodes.insert(from_bits(0), node());
        board.edges.insert(Id::new("start").unwrap(), edge());
        // After a hand-style id the walk starts from the first generated one.
        let mut taken = BTreeSet::new();
        taken.insert(from_bits(1));
        let id = fresh(&board, &taken, &mut Stuck("start"));
        assert_eq!(id, from_bits(2));
        // A generated one is, wrapping at 40 bits.
        let top = from_bits(u64::MAX);
        assert_eq!(top.as_str(), "zzzzzzzz");
        board.nodes.insert(top, node());
        assert_eq!(
            fresh(&board, &BTreeSet::new(), &mut Stuck("zzzzzzzz")),
            from_bits(1)
        );
    }

    #[test]
    fn a_source_is_trusted_while_it_draws_unused_ids() {
        let board = Board::default();
        let id = fresh(&board, &BTreeSet::new(), &mut Stuck("start"));
        assert_eq!(id.as_str(), "start");
    }

    #[test]
    fn ids_spell_forty_bits() {
        assert_eq!(from_bits(0).as_str(), "00000000");
        assert_eq!(from_bits(31).as_str(), "0000000z");
        assert_eq!(from_bits(32).as_str(), "00000010");
        assert_eq!(from_bits(1 << 40).as_str(), "00000000");
        for bits in [0, 1, 0x3ff, 0x12_3456_789a, (1 << 40) - 1] {
            assert_eq!(bits_of(&from_bits(bits)), Some(bits));
        }
        assert_eq!(bits_of(&Id::new("start").unwrap()), None);
        assert_eq!(bits_of(&Id::new("0000000i").unwrap()), None);
        assert!(!is_generated("0000000l") && !is_generated("0000000") && is_generated("ctrj33th"));
    }

    #[test]
    fn seeded_sources_repeat_and_random_ones_differ() {
        let draw = |ids: &mut dyn Ids| (0..4).map(|_| ids.draw()).collect::<Vec<_>>();
        assert_eq!(draw(&mut Seeded::new(5)), draw(&mut Seeded::new(5)));
        assert_ne!(draw(&mut Seeded::new(5)), draw(&mut Seeded::new(6)));
        assert_ne!(draw(&mut Random::new()), draw(&mut Random::new()));
    }
}
