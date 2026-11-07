# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/). Releases are
cut with `/release-workflows:release vX.Y.Z`: it turns the section below into a
`## vX.Y.Z — YYYY-MM-DD` entry, commits, bumps `[workspace.package].version` in
`Cargo.toml` with `scripts/release/update-version.sh`, tags `vX.Y.Z`, and
pushes. The tag triggers `.github/workflows/release.yml`, which builds the
macOS DMG and the Linux `.deb`s, publishes the GitHub Release with this
version's section as its notes, and publishes to the apt repo. See
[RELEASING.md](RELEASING.md).

## [Unreleased]

## v0.0.2 — 2026-11-07

### Changed

- **The macOS DMG is signed with a Developer ID and notarized.** Wren now opens
  with a normal Gatekeeper check instead of needing the
  `xattr -dr com.apple.quarantine` bypass. Wren 0.0.1 picks this release up
  through "Wren → Check for Updates…".

### Fixed

- The appcast unit tests and `tools/check_updater.py` no longer treat the
  committed update feed as empty, which broke CI once 0.0.1 was published to
  it.

## v0.0.1 — 2026-11-07

The first public build of Wren: a desktop app for structured Markdown notes,
for macOS (Apple silicon) and Linux (amd64, arm64).

### Added

- **Plain Markdown workspaces.** A workspace is a directory of `.md` files you
  own; Wren keeps no database, so git, `grep` and other editors keep working
  on the same folder. Several workspaces can be registered and switched.
- **A fixed, shallow hierarchy:** Workspace → Section → Group → Note, with
  notes allowed directly in a section. Create, rename, move and delete notes,
  groups and sections from the UI; deleted items go to the OS trash.
- **One open note at a time, no tabs.** Back/forward history and a quick
  switcher (⌘P / Alt+P) replace tab sprawl; fuzzy title search and indexed
  full-text search (⇧⌘F / Alt+Shift+F) cover the whole workspace.
- **Always-on autosave.** There is no Save command; writes are durable, and
  changes made on disk by other tools are picked up live.
- **A Markdown editor** with a line-number gutter, current-line highlight,
  accent caret, and struck-through completed tasks.
- **Two layouts** (columns, or a section/group outline) with resizable panes,
  plus a command palette (⌘K / Alt+K) for every action.
- **Settings and keybindings** in one user-owned `config.conf` (editor font
  size, line numbers, theme and more; see [docs/configuration.md](docs/configuration.md)
  and [docs/keybindings.md](docs/keybindings.md)), with several built-in
  themes.
- **Native integration:** the owl app icon on both platforms (the macOS 26
  glass icon where Xcode 26 builds it), a native macOS menu bar whose Quit
  saves first, and a Linux desktop entry and hicolor icons.
- **Release packaging:** a DMG for macOS and `.deb` packages for Ubuntu 24.04+
  (also published to the apt repo at `apt.stridelabs.ai`).
- **Automatic updates on macOS** through Sparkle: "Wren → Check for Updates…"
  plus Sparkle's scheduled checks, offering each stable release as a signed
  update. On Linux, updates come through apt.
