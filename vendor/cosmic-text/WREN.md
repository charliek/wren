# Vendored cosmic-text (Wren)

Upstream [cosmic-text](https://crates.io/crates/cosmic-text) 0.15.0 from
crates.io (git `c82ee1c5b5b8032e91eaff1cb34294b538727a7d`), the version iced
0.14 uses, with one feature added: a **hanging indent** for wrapped rows.
Wren's advanced editor engine uses it so the wrapped rows of a list item or a
block quote line up under the item's text instead of under its marker.
cosmic-text 0.15 cannot do this through its API: a line has one wrap width,
and the only per-row x offset is the alignment.

Only `src/` and the licenses are kept. `Cargo.toml` is the crates.io one
without its `[[test]]` and `[[bench]]` targets and `[profile.test]`, whose
files are not vendored. Each change is marked in the source with a
`Wren:` comment. About 65 lines are added and 13 changed:

* `src/buffer_line.rs`: `BufferLine` gains `hanging_indent: usize`, a byte
  offset into the line, 0 by default, with `hanging_indent()` and
  `set_hanging_indent()`. Setting a different value resets the layout, like
  `set_align`. `reset_new` clears it; `split_off` gives the new line 0.
  `layout` passes it to the shaper.
* `src/shape.rs`, `ShapeLine::layout_to_buffer` takes the offset as a new
  last argument (`ShapeLine::layout` passes 0):
  * The indent is the summed advance of the shaped glyphs that start before
    the offset, so it is measured in the line's own fonts and needs no
    second layout pass. It is capped at half the wrap width, and is 0 with
    `Wrap::None`.
  * Wrapping checks the first row against the full width and every later
    row against the width minus the indent. All ten
    `width_opt.unwrap_or(f32::INFINITY)` checks became `limit(rows so far)`.
  * Each continuation row starts the indent in from its start edge (right
    for RTL), and has that much less room for alignment and justification.
    Its `LayoutLine::w` includes the indent, so measuring a buffer still
    covers every glyph.
* `src/buffer.rs`, `Buffer::hit`: a click left of an LTR row's first glyph
  lands at the row's start. Upstream compared with 0, so a click in the
  indent fell through to the row's end. With no indent and left alignment,
  the first glyph is at 0 and nothing changes.

Because glyph positions carry the indent, drawing, hit-testing,
`LayoutRun::highlight`, and anything else that reads `LayoutGlyph::x`
follow it with no further change. With every indent at 0, which is all iced's
own text widgets ever set, layout is unchanged.

writ's `build_line_hanging` (MIT, on parley) measures its prefix at a
content byte the same way. No code comes from writ or from gpui's
`IndentAdjustment`, the other design reference.

The workspace `[patch.crates-io]` entry wires this in. A patch only shadows
the version it matches, so `make check` asserts that cosmic-text still
resolves here. When iced moves to a newer cosmic-text, re-apply the deltas
above to that release (`layout_to_buffer` is the one place with real
logic), or drop this directory if upstream has gained a hanging indent. The
tests are in `crates/wren-editor/src/backend/owned.rs`
(`wrapped_rows_hang_under_the_content_column` and its neighbours).
