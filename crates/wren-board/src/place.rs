//! Where keyboard building puts a new node (§5.6 of the board plan).

use crate::{Board, Dir, Id};

/// The top-left corner of the nearest free slot for a node of `size`
/// placed from the node `from` going `dir` (§5.6 rules 2 to 4), or `None`
/// when the search gives up after 200 steps.
pub fn slot(board: &Board, from: &Id, size: [i32; 2], dir: Dir) -> Option<[i32; 2]> {
    let _ = (board, from, size, dir);
    unimplemented!("placement arrives with item E8")
}
