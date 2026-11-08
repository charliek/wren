# Wren

Wren is a structured Markdown notes app for macOS and Linux. A workspace is a
plain folder of `.md` files in a fixed, shallow hierarchy:
**Workspace → Section → Group → Note**. You edit one note at a time,
autosave is always on, and every action is in a keyboard-first command
palette.

```text
~/notes/work/              ← workspace
├── 1-1s/                  ← section
│   ├── Alice Chen/        ← group
│   │   ├── 2026-10-28.md  ← note
│   │   └── 2026-11-04.md
│   └── Bob Ruiz/
│       └── 2026-11-03.md
└── Projects/
    └── API Migration/
        └── Cutover checklist.md
```

## Principles

- **Files are the truth.** There is no database. Obsidian, `grep`, git, and
  Finder keep working on the same folder; Wren adds only a `.wren/`
  directory.
- **Structure by construction.** The hierarchy is exactly two folders deep.
  The app never builds anything deeper.
- **One document, no tabs.** Jump to note, search, and Back/Forward history
  stand in for tabs.
- **Never lose a keystroke.** There is no Save command. Notes save 750 ms
  after you stop typing, and before every switch, move, focus loss, and quit.
- **Keyboard first, mouse friendly.** Every action is in the command palette;
  drag and drop is a second way to make the same moves.

## Install

Releases attach a macOS DMG and Linux `.deb` packages (amd64 and arm64) to
[GitHub Releases](https://github.com/charliek/wren/releases). On Ubuntu
24.04 and Pop!\_OS:

```bash
sudo apt install wren
```

See [Installation](getting-started/installation.md) for the apt repository
and building from source.

## Next steps

- [First run](getting-started/first-run.md): workspaces, sections, groups,
  and notes.
- [Keybindings](getting-started/keybindings.md): platform defaults and
  custom bindings.
- [Configuration](reference/configuration.md): `config.conf`, themes, and
  editor settings.
- [Workspace folder](reference/workspace-folder.md): what Wren reads and
  writes on disk.
- [Autosave and external edits](reference/autosave.md): when notes are
  written and how conflicts are kept.
- [Architecture](development/architecture.md): how the code is organized,
  for contributors.
