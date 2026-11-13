//! Stacking keys (§4.7 of the board plan).

/// The `z` key for a slot between the neighbours `lo` (below) and `hi`
/// (above), either of which may be absent, by the rules of §4.7. `None`
/// when no key fits: the scope is renumbered first (the renumber fallback).
pub fn key_between(lo: Option<&str>, hi: Option<&str>) -> Option<String> {
    let _ = (lo, hi);
    unimplemented!("stacking keys arrive with item E2")
}
