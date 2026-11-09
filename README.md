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
│   │   └── 2026-11-04.md ← note
│   └── Team/
│       └── Team sync.md
└── Projects/
```

## Features

- Sections, Groups, and Notes columns with resizable, persistent widths,
  plus a **focus mode** that shows only the editor.
- A Markdown editor with highlighting, a line-number gutter, soft wrap,
  undo/redo, and find-in-note. Enter continues lists, tasks, and quotes,
  Tab nests list items, and commands toggle tasks, bold, italic, and code
  and move lines. **Jump to heading** (⇧⌘O, Alt+Shift+O on Linux) lists
  the note's outline.
- Autosave 750 ms after you stop typing, and on every switch, move, focus
  loss, and quit. External edits reload cleanly, and conflicts keep both
  versions.
- **Command palette** (⇧⌘P on macOS, Alt+Shift+P on Linux), **Jump to note**,
  workspace-wide **search** with snippets, and per-pane filters.
- Move notes and groups by drag and drop or from the palette.
- Several workspaces, switched from the palette.
- Eleven built-in themes (Wren Dark/Light, Nord, Gruvbox Dark, Solarized
  Light, Rosé Pine, Everforest Dark, Tokyo Night, Dracula, Catppuccin Mocha
  and Macchiato), with light/dark following the system.
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

The documentation site is at <https://charliek.github.io/wren/>, built from
`docs/` with [Zensical](https://zensical.org/):

- [First run](docs/getting-started/first-run.md): workspaces, sections,
  groups, and notes
- [Keybindings](docs/getting-started/keybindings.md): platform defaults and
  custom bindings
- [Configuration](docs/reference/configuration.md): `config.conf`, themes,
  and editor settings
- [Architecture](docs/development/architecture.md) and
  [Testing](docs/development/testing.md): for contributors

Preview it locally with `uv run --group docs zensical serve`
(<http://127.0.0.1:7072/>). Designs that are planned but not built yet live
in [`discovery/`](discovery/README.md). Contributor conventions are in
[CLAUDE.md](CLAUDE.md).
