//! Library target exposing the editor seam for testing.
//!
//! This crate is primarily a binary (`src/main.rs`).  The library target exists
//! so that the editor-view modules can be compiled and unit-tested without
//! modifying the binary's module declarations.  The parent integrates the
//! editor by adding `mod editor_view;` to `main.rs`.

pub mod editor_view;
