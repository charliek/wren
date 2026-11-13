//! Stacking keys (§4.7 of the board plan).
//!
//! A key is 1 to 64 characters of `[0-9A-Za-z]`, read as base-62 digits in
//! that order, which is also their bytewise order. Elements sharing a
//! parent are drawn in ascending key order, then by id, a node before a
//! connector; root nodes and every connector form one scope. Keys are
//! opaque, and Wren never generates one ending in `0`, because no key sorts
//! between `X` and `X0`.

use crate::{placements, Board, Id};

/// The longest key: a longer one is flagged and stacked by its first
/// `MAX_LEN` bytes.
pub const MAX_LEN: usize = 64;

/// One element of a stacking scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Element<'a> {
    Node(&'a Id),
    Edge(&'a Id),
}

impl<'a> Element<'a> {
    pub fn id(self) -> &'a Id {
        match self {
            Self::Node(id) | Self::Edge(id) => id,
        }
    }
}

/// What elements of one scope are drawn in ascending order of: the key,
/// by its first [`MAX_LEN`] bytes and a missing one as empty; then the id;
/// then a node before a connector.
pub fn order<'a>(z: Option<&'a str>, id: &'a Id, edge: bool) -> (&'a [u8], &'a str, bool) {
    let z = z.unwrap_or("").as_bytes();
    (&z[..z.len().min(MAX_LEN)], id.as_str(), edge)
}

/// The elements of the scope inside the node `parent`, or of the root
/// scope (root nodes and every connector) for `None`, in drawing order. A
/// node is in the scope of the parent it is drawn inside, so one whose
/// `parent` is dangling or in a cycle is a root.
pub fn scope<'a>(board: &'a Board, parent: Option<&Id>) -> Vec<(Element<'a>, Option<&'a str>)> {
    let placed = placements(board);
    let mut out: Vec<(Element<'a>, Option<&'a str>)> = board
        .nodes
        .iter()
        .filter(|(id, _)| placed.get(id).and_then(|p| p.parent) == parent)
        .map(|(id, node)| (Element::Node(id), node.z.as_deref()))
        .collect();
    if parent.is_none() {
        out.extend(
            board
                .edges
                .iter()
                .map(|(id, edge)| (Element::Edge(id), edge.z.as_deref())),
        );
    }
    out.sort_by(|(a, za), (b, zb)| {
        order(*za, a.id(), matches!(a, Element::Edge(_))).cmp(&order(
            *zb,
            b.id(),
            matches!(b, Element::Edge(_)),
        ))
    });
    out
}

/// The key for a slot between the neighbours `lo` (below) and `hi`
/// (above), either of which may be absent, by the rules of §4.7. `None`
/// when no key fits: `hi` is `lo` followed by zero or more `0`s, the key
/// would pass [`MAX_LEN`] characters, a neighbour is not 0 to 64
/// characters of the alphabet, or `lo` does not sort below `hi`. The scope
/// is then renumbered first ([`insert`]).
pub fn key_between(lo: Option<&str>, hi: Option<&str>) -> Option<String> {
    let lo = match lo {
        Some(lo) => Some(digits(lo)?),
        None => None,
    };
    let hi = match hi {
        Some(hi) => Some(digits(hi)?),
        None => None,
    };
    let key = match (lo, hi) {
        (None, None) => vec![31],
        (Some(lo), None) => {
            let mut n = prefix(&lo) + 1;
            if n % 62 == 0 {
                n += 1;
            }
            if n < CUBE {
                three(n)
            } else {
                bisect(&lo, None)?
            }
        }
        (None, Some(hi)) => {
            let mut n = prefix(&hi) - 1;
            if n % 62 == 0 {
                n -= 1;
            }
            if n >= 1 {
                three(n)
            } else {
                bisect(&[], Some(&hi))?
            }
        }
        (Some(lo), Some(hi)) => bisect(&lo, Some(&hi))?,
    };
    (key.len() <= MAX_LEN).then(|| spell(&key))
}

/// The keys a scope of `count` elements gets when it is renumbered, in
/// drawing order: `119165 + 2i` as three characters (`V01`, `V03`, …),
/// which leaves room for one key in every gap and above the top. Past
/// 59,582 elements, more than a board may hold from Phase 3, the same
/// pattern continues at four characters (`V001`, …).
pub fn renumbered(count: usize) -> Vec<String> {
    let mut width = 3;
    // The first key, `V` followed by zeros and a 1, and the room above it.
    let (mut first, mut room) = (119_165_u64, (CUBE as u64 - 119_165).div_ceil(2));
    // Ten characters already number more elements than memory holds, and
    // keep the arithmetic inside `u64`.
    while (count as u64) > room && width < 10 {
        width += 1;
        first = (first - 1) * 62 + 1;
        room = (62_u64.pow(width as u32) - first).div_ceil(2);
    }
    (0..count as u64)
        .map(|i| {
            let mut n = first + 2 * i;
            let mut key = vec![0; width];
            for digit in key.iter_mut().rev() {
                *digit = (n % 62) as u8;
                n /= 62;
            }
            spell(&key)
        })
        .collect()
}

/// New keys for `count` elements put, in their order, into a scope whose
/// keys in drawing order are `keys`, between `keys[at - 1]` and `keys[at]`
/// (on top when `at` is the length).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insert {
    /// The keys of the new elements, ascending.
    pub keys: Vec<String>,
    /// When no key fitted, the new keys of the whole scope, in its drawing
    /// order, given in the same step (the renumber fallback).
    pub renumber: Option<Vec<String>>,
}

/// The keys for `count` elements inserted at `at` into a scope whose keys
/// are `keys` in drawing order (§4.7): each takes [`key_between`] the one
/// below and the old key above, so they keep their order. When one does
/// not fit, the scope is renumbered and the keys are taken between its new
/// keys, or, for more than a gap holds, the new elements are numbered with
/// the rest. An `at` past the end is the top.
pub fn insert(keys: &[Option<&str>], at: usize, count: usize) -> Insert {
    let at = at.min(keys.len());
    if let Some(new) = between(keys, at, count) {
        return Insert {
            keys: new,
            renumber: None,
        };
    }
    let renumber = renumbered(keys.len());
    let spelled: Vec<Option<&str>> = renumber.iter().map(|key| Some(key.as_str())).collect();
    if let Some(new) = between(&spelled, at, count) {
        return Insert {
            keys: new,
            renumber: Some(renumber),
        };
    }
    // More than one gap can hold, some hundreds: number the new elements
    // with the rest.
    let mut renumber = renumbered(keys.len() + count);
    let keys = renumber.drain(at..at + count).collect();
    Insert {
        keys,
        renumber: Some(renumber),
    }
}

/// Keys for `count` elements at `at`, or `None` when one does not fit. A
/// missing neighbour key reads as empty, as it sorts.
fn between(keys: &[Option<&str>], at: usize, count: usize) -> Option<Vec<String>> {
    let hi = keys.get(at).map(|key| key.unwrap_or(""));
    let mut lo = at
        .checked_sub(1)
        .and_then(|k| keys.get(k))
        .map(|key| key.unwrap_or("").to_owned());
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let key = key_between(lo.as_deref(), hi)?;
        lo = Some(key.clone());
        out.push(key);
    }
    Some(out)
}

/// 62 cubed: the three-character keys.
const CUBE: i64 = 62 * 62 * 62;

fn digits(key: &str) -> Option<Vec<u8>> {
    if key.len() > MAX_LEN {
        return None;
    }
    key.bytes().map(digit).collect()
}

fn digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'Z' => Some(c - b'A' + 10),
        b'a'..=b'z' => Some(c - b'a' + 36),
        _ => None,
    }
}

fn spell(key: &[u8]) -> String {
    const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    key.iter()
        .map(|&d| char::from(ALPHABET[usize::from(d)]))
        .collect()
}

/// The first three digits of a key, padded on the right with 0, as a
/// number.
fn prefix(key: &[u8]) -> i64 {
    (0..3).fold(0, |n, k| {
        n * 62 + i64::from(key.get(k).copied().unwrap_or(0))
    })
}

/// `n`, in `0..CUBE`, as three digits.
fn three(n: i64) -> Vec<u8> {
    vec![(n / 3844) as u8, (n / 62 % 62) as u8, (n % 62) as u8]
}

/// The key between `lo` and `hi` (none: above everything) by §4.7's
/// bisection, or `None` when `hi` is `lo` followed by zero or more `0`s or
/// does not sort above it.
fn bisect(lo: &[u8], hi: Option<&[u8]>) -> Option<Vec<u8>> {
    if let Some(hi) = hi {
        if hi <= lo || (hi.starts_with(lo) && hi[lo.len()..].iter().all(|&d| d == 0)) {
            return None;
        }
    }
    let mut hi = hi;
    let mut key = Vec::new();
    // Each step either stops or appends a digit; past both keys' ends with
    // `hi` gone it stops at once, so this is bounded by the longer key.
    for i in 0.. {
        let a = lo.get(i).copied().unwrap_or(0);
        let b = match hi {
            Some(hi) => *hi.get(i)?,
            None => 62,
        };
        if a == b {
            key.push(a);
        } else if b >= a + 2 {
            key.push((a + b) / 2);
            return Some(key);
        } else if b == a + 1 && hi.is_some_and(|hi| hi.len() > i + 1) {
            key.push(b);
            return Some(key);
        } else if b == a + 1 {
            key.push(a);
            hi = None;
        } else {
            return None;
        }
        if key.len() > MAX_LEN {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::Rng;
    use crate::parse;

    fn between(lo: Option<&str>, hi: Option<&str>) -> Option<String> {
        key_between(lo, hi)
    }

    #[test]
    fn the_rules_by_example() {
        let table: &[(Option<&str>, Option<&str>, Option<&str>)] = &[
            // Rule 1.
            (None, None, Some("V")),
            // Rule 2: the next three-character key, skipping one ending in 0.
            (Some("V"), None, Some("V01")),
            (Some("V01"), None, Some("V02")),
            (Some("V0z"), None, Some("V11")),
            (Some("V0y"), None, Some("V0z")),
            (Some("a1"), None, Some("a11")),
            (Some("V01zzz"), None, Some("V02")),
            // Rule 2 past three characters: bisect above.
            (Some("zzz"), None, Some("zzzV")),
            (Some("zzy"), None, Some("zzz")),
            (Some("zzzz"), None, Some("zzzzV")),
            // Rule 3.
            (None, Some("V"), Some("Uzz")),
            (None, Some("V02"), Some("V01")),
            (None, Some("V01"), Some("Uzz")),
            (None, Some("V0"), Some("Uzz")),
            (None, Some("002"), Some("001")),
            // Rule 3 below three characters: bisect from empty.
            (None, Some("001"), Some("000V")),
            (None, Some("0001"), Some("0000V")),
            (None, Some("01"), Some("00z")),
            (None, Some("0"), None),
            (None, Some("000"), None),
            // Rule 4.
            (Some("V01"), Some("V03"), Some("V02")),
            (Some("V01"), Some("V02"), Some("V01V")),
            (Some("V1"), Some("V2x"), Some("V2")),
            (Some("a"), Some("b"), Some("aV")),
            (Some("a"), Some("c"), Some("b")),
            (Some("V"), Some("V001"), Some("V000V")),
            (Some("az"), Some("b"), Some("azV")),
            (Some("a0"), Some("a1"), Some("a0V")),
            // An empty or missing neighbour reads as the empty key.
            (Some(""), Some("1"), Some("0V")),
            (Some(""), None, Some("001")),
            (None, Some(""), None),
            // No key between: equal, `hi` is `lo` and zeros, out of order.
            (Some("V"), Some("V"), None),
            (Some("V"), Some("V0"), None),
            (Some("V"), Some("V000"), None),
            (Some("b"), Some("a"), None),
            // Not keys.
            (Some("a-1"), None, None),
            (None, Some("é"), None),
            (Some("a"), Some("b c"), None),
        ];
        for &(lo, hi, want) in table {
            assert_eq!(between(lo, hi).as_deref(), want, "{lo:?} {hi:?}");
        }
    }

    #[test]
    fn keys_stop_at_sixty_four_characters() {
        let long = "z".repeat(MAX_LEN);
        assert_eq!(between(Some(&long), None), None);
        let short = "z".repeat(MAX_LEN - 1);
        assert_eq!(between(Some(&short), None).unwrap().len(), MAX_LEN);
        assert_eq!(between(Some(&"a".repeat(MAX_LEN + 1)), None), None);
        assert_eq!(between(None, Some(&"a".repeat(MAX_LEN + 1))), None);
        // Room only at the 65th character.
        let lo = format!("{}1", "a".repeat(MAX_LEN - 1));
        let hi = format!("{}2", "a".repeat(MAX_LEN - 1));
        assert_eq!(between(Some(&lo), Some(&hi)), None);
    }

    fn is_key(key: &str) -> bool {
        (1..=MAX_LEN).contains(&key.len()) && key.bytes().all(|c| c.is_ascii_alphanumeric())
    }

    /// A key as a person might write one, or one Wren generated.
    fn random_key(rng: &mut Rng) -> String {
        const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        const HAND: &[&str] = &[
            "a", "a0", "a00", "a1", "a10", "b", "V", "V0", "V01", "Uzz", "z", "zz", "zzz", "0",
            "00", "01", "1", "A", "Z", "a0z", "m",
        ];
        match rng.below(4) {
            0 => (*rng.pick(HAND)).to_owned(),
            1 => {
                // Long keys, near the cap.
                let len = MAX_LEN - rng.below(4);
                (0..len).map(|_| char::from(*rng.pick(DIGITS))).collect()
            }
            _ => {
                // Short keys over a few digits, so that pairs share prefixes.
                let few = &DIGITS[..[2, 3, 62][rng.below(3)]];
                let len = 1 + rng.below(6);
                let base: String = (0..len).map(|_| char::from(*rng.pick(few))).collect();
                if rng.one_in(3) {
                    format!("{base}{}", "0".repeat(rng.below(3)))
                } else {
                    base
                }
            }
        }
    }

    /// §4.16 test 11: over random pairs, the key lies strictly between its
    /// neighbours, is a key, does not end in `0`, and is refused only for
    /// the reasons §4.7 gives.
    fn pairs_hold(count: usize, seed: u64) {
        let mut rng = Rng(seed);
        for _ in 0..count {
            let mut a = random_key(&mut rng);
            let mut b = random_key(&mut rng);
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            let (lo, hi) = match rng.below(4) {
                0 => (Some(a.as_str()), None),
                1 => (None, Some(b.as_str())),
                _ => (Some(a.as_str()), Some(b.as_str())),
            };
            match between(lo, hi) {
                Some(key) => {
                    assert!(is_key(&key) && !key.ends_with('0'), "{lo:?} {hi:?}: {key}");
                    assert!(lo.is_none_or(|lo| lo < key.as_str()), "{lo:?} {key}");
                    assert!(hi.is_none_or(|hi| key.as_str() < hi), "{hi:?} {key}");
                }
                None => {
                    let zeros = |lo: &str, hi: &str| {
                        hi.starts_with(lo) && hi[lo.len()..].bytes().all(|c| c == b'0')
                    };
                    // A key is at most one character longer than its
                    // longer neighbour, so only a 64-character neighbour can
                    // need a 65th; and only an all-zero `hi` has nothing
                    // below it.
                    let reason = match (lo, hi) {
                        (Some(lo), Some(hi)) => {
                            zeros(lo, hi) || lo.len() == MAX_LEN || hi.len() == MAX_LEN
                        }
                        (Some(lo), None) => lo.len() == MAX_LEN,
                        (None, Some(hi)) => zeros("", hi) || hi.len() == MAX_LEN,
                        (None, None) => false,
                    };
                    assert!(reason, "{lo:?} {hi:?} refused");
                }
            }
        }
    }

    #[test]
    fn random_pairs_hold() {
        pairs_hold(20_000, 1);
    }

    #[test]
    #[ignore = "full count; run by make fuzz-board"]
    fn fuzz_random_pairs_hold() {
        pairs_hold(400_000, 2);
    }

    #[test]
    fn appends_and_prepends_stay_at_three_characters() {
        let mut top = "V".to_owned();
        let mut bottom = "V".to_owned();
        for _ in 0..10_000 {
            let up = between(Some(&top), None).unwrap();
            let down = between(None, Some(&bottom)).unwrap();
            assert!(up.len() == 3 && up > top && !up.ends_with('0'), "{up}");
            assert!(
                down.len() == 3 && down < bottom && !down.ends_with('0'),
                "{down}"
            );
            (top, bottom) = (up, down);
        }
    }

    #[test]
    fn same_gap_inserts_end_in_a_renumber_that_keeps_the_order() {
        for below in [false, true] {
            let mut keys = vec!["V01".to_owned(), "V03".to_owned()];
            let mut inserts = 0;
            loop {
                // Always into the gap just above the first key, or just
                // below the last.
                let at = if below { keys.len() - 1 } else { 1 };
                let spelled: Vec<Option<&str>> = keys.iter().map(|k| Some(k.as_str())).collect();
                let insert = insert(&spelled, at, 1);
                let [key] = &insert.keys[..] else {
                    panic!("{insert:?}")
                };
                if let Some(renumber) = &insert.renumber {
                    assert_eq!(renumber, &renumbered(keys.len()));
                    let mut after = renumber.clone();
                    after.insert(at, key.clone());
                    assert!(after.windows(2).all(|w| w[0] < w[1]), "{after:?}");
                    break;
                }
                keys.insert(at, key.clone());
                assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");
                inserts += 1;
            }
            // Each character halves the gap about six times.
            assert!(inserts > 300, "{below}: {inserts}");
        }
    }

    #[test]
    fn renumbered_keys_ascend_from_v01_with_a_gap_each() {
        let keys = renumbered(59_582);
        assert_eq!(&keys[..3], ["V01", "V03", "V05"]);
        assert_eq!(keys.last().unwrap(), "zzz");
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert!(keys.iter().all(|k| k.len() == 3 && !k.ends_with('0')));
        for pair in keys.windows(2).step_by(997) {
            let key = between(Some(&pair[0]), Some(&pair[1])).unwrap();
            assert!(key.len() <= 3, "{pair:?} {key}");
        }
        assert_eq!(between(Some("zzz"), None).unwrap(), "zzzV");
        let more = renumbered(59_583);
        assert_eq!(&more[..2], ["V001", "V003"]);
        assert!(more.windows(2).all(|w| w[0] < w[1]));
        assert!(more.iter().all(|k| k.len() == 4 && !k.ends_with('0')));
        assert!(renumbered(0).is_empty());
    }

    #[test]
    fn inserting_on_top_and_below_and_between() {
        let keys = [Some("V"), Some("W"), Some("X")];
        let insert = |at, count| super::insert(&keys, at, count);
        assert_eq!(insert(3, 2).keys, ["X01", "X02"]);
        assert_eq!(insert(9, 1).keys, ["X01"]);
        assert_eq!(insert(0, 2).keys, ["Uzz", "UzzV"]);
        assert_eq!(insert(1, 3).keys, ["VV", "Vk", "Vs"]);
        assert!([0, 1, 3].iter().all(|&at| insert(at, 3).renumber.is_none()));
        assert_eq!(super::insert(&[], 0, 2).keys, ["V", "V01"]);
        // Into a tie, an invalid key or past a missing one.
        for (keys, at) in [
            (vec![Some("V"), Some("V")], 1),
            (vec![Some("a-1"), Some("b")], 1),
            (vec![None, None], 1),
        ] {
            let insert = super::insert(&keys, at, 2);
            let renumber = insert.renumber.unwrap();
            assert_eq!(renumber, ["V01", "V03"]);
            let mut after = renumber.clone();
            after.splice(at..at, insert.keys);
            assert!(after.windows(2).all(|w| w[0] < w[1]), "{after:?}");
        }
        // A missing key below sorts as empty and needs no renumber.
        let insert = super::insert(&[None, Some("V")], 1, 1);
        assert_eq!((insert.keys, insert.renumber), (vec!["F".to_owned()], None));
        // More than one gap holds.
        let insert = super::insert(&keys, 1, 1_000);
        let all = renumbered(1_003);
        let renumber = insert.renumber.unwrap();
        assert_eq!(renumber, [&*all[0], &all[1_001], &all[1_002]]);
        let mut after = renumber.clone();
        after.splice(1..1, insert.keys);
        assert_eq!(after, all);
    }

    const SCOPES: &str = r#"schema = 1

[node.a]
kind = "shape"
at = [0, 0]
size = [100, 100]
z = "V"

[node.b]
kind = "shape"
at = [0, 0]
size = [10, 10]
z = "V"

[node.c]
kind = "shape"
at = [0, 0]
parent = "a"
size = [10, 10]
z = "b"

[node.d]
kind = "shape"
at = [0, 0]
parent = "a"
size = [10, 10]
z = "a"

[node.e]
kind = "shape"
at = [0, 0]
parent = "gone"
size = [10, 10]

[node.f]
kind = "shape"
at = [0, 0]
parent = "f"
size = [10, 10]
z = "zzzz"

[edge.a]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
z = "V"

[edge.c]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
z = "U"
"#;

    fn spelled(scope: &[(Element<'_>, Option<&str>)]) -> Vec<String> {
        scope
            .iter()
            .map(|(element, _)| match element {
                Element::Node(id) => format!("node.{id}"),
                Element::Edge(id) => format!("edge.{id}"),
            })
            .collect()
    }

    #[test]
    fn a_scope_is_in_drawing_order_with_ties_falling_to_the_id() {
        let board = parse(SCOPES).unwrap();
        let root = scope(&board, None);
        // A missing key sorts first; equal keys by id, then a node before
        // a connector of the same id; a dangling or cyclic parent is a root.
        assert_eq!(
            spelled(&root),
            ["node.e", "edge.c", "node.a", "edge.a", "node.b", "node.f"]
        );
        let a = Id::new("a").unwrap();
        assert_eq!(spelled(&scope(&board, Some(&a))), ["node.d", "node.c"]);
        assert!(scope(&board, Some(&Id::new("b").unwrap())).is_empty());
        // The renumber keeps that order.
        let keys = renumbered(root.len());
        let mut again: Vec<_> = root
            .iter()
            .zip(&keys)
            .map(|((element, _), key)| {
                order(Some(key), element.id(), matches!(element, Element::Edge(_)))
            })
            .collect();
        let before = again.clone();
        again.sort();
        assert_eq!(again, before);
    }

    #[test]
    fn order_compares_sixty_four_bytes_then_the_id_then_the_kind() {
        let [a, b] = [Id::new("a").unwrap(), Id::new("b").unwrap()];
        let long = "V".repeat(MAX_LEN);
        let longer = format!("{long}0");
        assert!(order(Some(&longer), &a, false) < order(Some(&long), &b, false));
        assert!(order(Some("V"), &a, true) < order(Some("V"), &b, false));
        assert!(order(Some("V"), &a, false) < order(Some("V"), &a, true));
        assert!(order(None, &b, false) < order(Some("0"), &a, false));
        assert_eq!(order(None, &a, false), order(Some(""), &a, false));
    }
}
