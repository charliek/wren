# Vendored iced_tiny_skia (Wren)

Upstream [iced_tiny_skia](https://crates.io/crates/iced_tiny_skia) 0.14.1
from crates.io, the version iced 0.14.0 resolves to, plus the change below.
`LICENSE` is iced's MIT licence, which the published crate does not include.
The change is marked in the source with a `Wren delta vs upstream 0.14.1`
comment.

* `src/window/compositor.rs::merge_groups`: after `damage::group`, merge any
  two damage groups whose union wastes no more than `damage::group`'s own
  threshold (20,000 square pixels), in any order, until no two can merge.
  `damage::group` sorts the damaged regions by their distance from the
  window's corner and merges only neighbours in that order, so a small region
  inside a large one stays a group of its own whenever another region sorts
  between them. `Renderer::draw` then repaints every layer once per group,
  and each group costs about a millisecond whatever its size. A new
  primitive drawn inside the editor's text, such as a spelling zigzag after a
  note switch, therefore added a repaint of the whole window's layers. The
  merged groups still cover every damaged pixel, and a group is repainted
  from the background up, so the pixels on screen do not change.

Measured on the spell check's acceptance note in release under X11 (see
`discovery/spell-check.md`, §7.5 `spell-endfix`): the frame that switches to
it went from 15 groups and about 23 ms to a few groups and about 10 ms, and
opening it from the test harness from about 56 ms to about 41 ms.

The workspace `[patch.crates-io]` entry wires this in. A patch only shadows the
version it matches, so `make check` asserts that iced_tiny_skia still resolves
here. When iced is upgraded, carry the change to the new release or drop it if
upstream's grouping no longer depends on the order.
