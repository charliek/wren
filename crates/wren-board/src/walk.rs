//! Walking the selection with the arrow keys (§5.7 of the board plan).

use crate::{Board, Dir, Id};

/// The node an arrow key selects from the element `from` going `dir`
/// (§5.7 rules 1 and 2), or `None` when nothing lies that way.
pub fn next(board: &Board, from: &Id, dir: Dir) -> Option<Id> {
    let _ = (board, from, dir);
    unimplemented!("walking arrives with item E8")
}
