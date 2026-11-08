# Vendored swash (Wren)

Upstream [swash](https://crates.io/crates/swash) 0.2.10 from crates.io, the
latest release, plus the safety fixes below. They are the same fixes Roost
carries. Each one stops a font with a malformed or unusual table from
crashing Wren. Without them, a debug build aborts on an integer overflow, and
some of them hang or panic in release builds too. Each change is marked in the
source with a `Wren delta vs upstream 0.2.10` comment.

* `src/internal/xmtx.rs::advance` returns `0` when `long_metric_count == 0`.
  A font whose `hhea` or `vhea` declares zero long metrics underflowed
  `long_metric_count - 1`. macOS ships one, GB18030 Bitmap, which fontdb
  reports as monospaced, so it appears in the editor font picker. Choosing
  it aborted a debug build.
* `src/strike.rs::get_location`: the format-4 bitmap-index bisection stepped
  with `l = i + i` instead of `l = i + 1`, so a malformed `EBLC`/`CBLC` spun
  the calling thread forever in either profile.
* `src/internal/var.rs` gets three subtraction guards for malformed variation
  tables: an `fvar` `instanceSize` below 2, a `shortDeltaCount` greater than
  `regionIndexCount`, and an `HVAR`/`VVAR` delta-set map with `mapCount == 0`.
* `src/string.rs::Chars::next` makes the MacRoman read bounds-checked. It
  panicked in both profiles when a name record's length overran the string
  storage.

Roost also turns hinting off on macOS here. That is a rendering change, not a
safety fix, so Wren does not carry it.

The workspace `[patch.crates-io]` entry wires this in. A patch only shadows the
version it matches, so `make check` asserts that swash still resolves here.
Delete this directory and the patch entry once a published swash release ships
all of these fixes. The regression tests are
`crates/wren-iced/tests/swash_zero_metrics.rs` and
`crates/wren-iced/tests/swash_malformed_font.rs`.
