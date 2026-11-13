//! Where new element ids come from (§4.2 of the board plan).

use crate::Id;

/// A source of candidate ids for the elements a plan creates. A candidate
/// is 8 characters of `0123456789abcdefghjkmnpqrstvwxyz` (40 bits); the
/// plan draws again while one is taken by the board or by the plan itself.
/// The app draws from `std::hash::RandomState`; a test supplies its own
/// ids, or a seeded source.
pub trait Ids {
    fn draw(&mut self) -> Id;
}
