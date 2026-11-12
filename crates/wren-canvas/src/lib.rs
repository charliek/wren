//! The board canvas: an iced widget that paints a [`wren_board::scene`]
//! and turns pointer and key input into board edits.
//!
//! Like `wren-editor`, it knows no Wren types and no app messages: the host
//! passes in a [`wren_board::Palette`], a style, the label font and a key
//! policy, and owns the view state between frames.
