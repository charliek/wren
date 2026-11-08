# Vendored cosmic-text (Wren)

Upstream [cosmic-text](https://crates.io/crates/cosmic-text) 0.15.0 from
crates.io (git `c82ee1c5b5b8032e91eaff1cb34294b538727a7d`), the version iced
0.14 uses. The source is unchanged so far; vendoring it lets Wren carry a
layout patch for the advanced editor engine.

Only `src/` and the licenses are kept. `Cargo.toml` is the crates.io one
without its `[[test]]` and `[[bench]]` targets and `[profile.test]`, whose
files are not vendored.

The workspace `[patch.crates-io]` entry wires this in. A patch only shadows
the version it matches, so `make check` asserts that cosmic-text still
resolves here.
