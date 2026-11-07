# Wren

Structured Markdown notes for macOS and Linux. A workspace is a plain folder
of `.md` files in a fixed, shallow hierarchy (**Workspace → Section → Group →
Note**). You edit one note at a time, autosave is always on, and every
action is in a keyboard-first command palette. The folder stays yours:
Obsidian, `grep`, git, and Finder keep working on it unchanged (Wren adds
only a `.wren/` directory).

```
~/notes/work/            ← workspace
├── 1-1s/                ← section
│   ├── Alice Chen/      ← group
│   │   └── 2026-11-04.md
│   └── Team sync.md     ← a note directly in a section
└── Projects/
```

## Features

- Two layouts, **Columns** and **Outline**, with resizable, persistent panes,
  plus a **focus mode** that shows only the editor.
- A Markdown editor with highlighting, a line-number gutter, soft wrap,
  undo/redo, and find-in-note.
- Autosave 750 ms after you stop typing, and on every switch, move, focus
  loss, and quit. External edits reload cleanly, and conflicts keep both
  versions.
- **Command palette** (⇧⌘P on macOS, Alt+Shift+P on Linux), **Jump to note**,
  workspace-wide **search** with snippets, and per-pane filters.
- Move notes and groups by drag and drop or from the palette.
- Several workspaces, switched from the palette.
- Seven built-in themes (Wren Dark/Light, Nord, Gruvbox Dark, Solarized
  Light, Rosé Pine, Everforest Dark), with light/dark following the system.
- Configurable keybindings and settings in `~/.config/wren/config.conf`.

## Install

Releases attach a macOS DMG and Linux `.deb` packages (amd64 and arm64) to
[GitHub Releases](https://github.com/charliek/wren/releases). On Ubuntu
24.04 and Pop!_OS, Wren is also published to the
[charliek apt repository](https://github.com/charliek/apt-charliek):

```sh
sudo apt install wren
```

On macOS, Wren updates itself through Sparkle: it checks for new releases on a
schedule, and **Wren → Check for Updates…** checks on demand.

## Build from source

Requires the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs
it automatically) and, for the test harness, [uv](https://docs.astral.sh/uv/).

```sh
make setup        # fetch locked Rust and Python dependencies
make run          # restore the last workspace, or show onboarding
make run ARGS="--workspace /absolute/path/to/notes"
make package      # build an uninstalled native bundle (Wren.app on macOS)
make check        # formatting, clippy, Rust tests, Python unit tests
```

On Linux the build needs the usual windowing development libraries
(`libxkbcommon`, Wayland, and X11 client libraries); see
`.github/workflows/ci.yml` for the exact package list.

## Documentation

- [Keybindings](docs/keybindings.md): platform defaults and custom bindings
- [Configuration](docs/configuration.md): `config.conf`, themes, and editor
  settings
- [SPEC.md](SPEC.md) and [MOCKUP.md](MOCKUP.md): the product design
- [PLAN.md](PLAN.md): the implementation record and verification evidence
- [CLAUDE.md](CLAUDE.md): architecture and contributor conventions
