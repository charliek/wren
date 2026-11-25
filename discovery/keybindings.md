# Keybindings: one switchable command key on Linux

Status: planned (2026-11-25). Owner decisions are in §2. The build is §5.

## 1. What changes and why

On macOS every Wren command uses ⌘. On Linux there is no single command key today:

- app commands use Alt (`Platform::primary()`);
- editor, task and board commands use Ctrl;
- undo, redo and find in the editor are hard-coded on Ctrl.

The split was inherited from Roost, a terminal, where Ctrl belongs to the shell. Wren is a text app. Almost every Linux GUI app, VS Code and Zed included, uses Ctrl as its command key. Alt is secondary: menu mnemonics, Alt+←/→ back and forward, Alt+↑/↓ move line. Window managers take Alt+Tab, Alt+F4 and often Alt+drag. Super belongs to the desktop. GNOME redoes with Ctrl+Shift+Z, KDE with Ctrl+Y.

The owner prefers Alt as the command key everywhere, the way ⌘ works on the Mac. So Wren gets **one command key, Mod**:

- ⌘ on macOS;
- Ctrl on Linux by default, as other apps use;
- Alt on Linux with one switch.

## 2. Decisions (owner, 2026-11-25)

1. **One Mod key.** Every default that uses ⌘ on the Mac uses Mod on Linux: app commands, Markdown formatting, tasks, the board, and undo and redo.
2. **The default is Ctrl.** `linux-modifier = ctrl | alt` in `config.conf`, default `ctrl`, ignored on macOS. A palette command, "Use Alt for commands" / "Use Ctrl for commands" (Linux only), switches it, writes `config.conf` and applies it at once.
3. **Alt mode moves everything,** including clipboard and select all. In Alt mode, Alt+C/X/V/A copy, cut, paste and select all, in the editor, on a board and in every text field, and the Ctrl versions keep working. In Ctrl mode they are Ctrl only.
4. **Undo and redo** are Mod+Z and Mod+Shift+Z in the editor and on a board. **Ctrl+Y redo goes away.** In Alt mode Ctrl+Z and Ctrl+Shift+Z keep working.
5. **Everything that is a command is overridable,** as in Roost. Undo, redo, copy, cut, paste, select all and find become keymap actions, and `keybind` lines accept `mod+…`, so an override follows the switch. Keys that belong to one widget stay fixed: Enter, Escape, Tab, the arrows in menus, the calendar and the form, and menu accelerators.
6. **Existing installs get the new default (Ctrl).** The owner flips the switch once. The changelog says so prominently.

## 3. The model

Default tables are written with two logical modifiers:

| Token | macOS | Linux, Ctrl mode (default) | Linux, Alt mode |
|---|---|---|---|
| `mod` | ⌘ (`super`) | Ctrl | Alt |
| `opt` (board only) | ⌥ | Alt | Ctrl |

- **Why `opt` exists.** A board binds both Mod+arrow (connect, select a connector) and ⌥+arrow (move one cell, resize). With Mod = Alt those two would collide, so on a board the secondary key is "the other one".
- **Move line.** It stays Alt+↑/↓ on Linux in both modes, as in code editors, because nothing else binds it.
- **Text editing that is not a command stays Ctrl on Linux in both modes:** word motion (Ctrl+←/→), Ctrl+Backspace/Delete, Ctrl+Home/End. These are toolkit keys that every desktop field shares. The editor also keeps Alt+←/→ word motion on Linux.

**Exceptions, kept few and each with a comment and a test:**

| Action | Rule | Why |
|---|---|---|
| `editor.spell_suggest` | Mod+.; but Alt+. in Linux Ctrl mode | IBus takes Ctrl+. on GNOME |
| `navigation.find`, `app.quit` | In Alt mode, Ctrl+F and Ctrl+Q also bind | Desktop habit |
| clipboard, select all, undo, redo | In Alt mode, the Ctrl versions also bind | Every other text field on the desktop |
| Board pointer modifiers (drag to select or copy, scroll to zoom) | Stay on Ctrl on Linux in both modes, ⌘/⌥ on the Mac | Window managers take Alt+drag; these are not keybindings |

**Resulting Linux defaults (base layer), in Ctrl mode / Alt mode:**

- Mod+Shift+P / Mod+K palette; Mod+P jump; Mod+Shift+F search; Mod+F find; Mod+Shift+O outline.
- Mod+N / Mod+Shift+N new / new dated note; Mod+[ / Mod+] back / forward; Mod+Shift+[ / ] column left / right; Mod+R rename.
- Mod+Y focus mode (Ctrl+Y is now free); Mod+Shift+Y collapse columns; Mod++ / Mod+= / Mod+- / Mod+0 text size; Mod+, settings (new on Linux); Mod+Q quit.
- Mod+Enter toggle task; Mod+B / I / E bold, italic, code; Alt+↑/↓ move line; spell suggest per the table above.
- Mod+Z undo, Mod+Shift+Z redo, Mod+C / X / V copy, cut, paste, Mod+A select all (new actions `edit.*`).
- Tasks (tasks on): Mod+D due date, Mod+Shift+Enter task form.
- Board layer: as today with `m` → `mod` and `alt` → `opt`; Ctrl+Y redo removed.

**Collisions to check in both modes.**
- Ctrl mode: Mod+Y (focus) against redo, now Mod+Shift+Z only. Mod+F against the editor's hard-coded Ctrl+F, which is removed and folded into `navigation.find`. Mod+K. Mod+[ / ].
- Alt mode: the board's Mod+arrow against `opt`+arrow (Ctrl+arrow); Alt+arrow word motion in the editor against nothing bound.
- A test asserts that every trigger is bound at most once per layer and mode.

## 4. Text fields in Alt mode

The palette input, dialogs, the find bar, the calendar field and the task form's fields are iced `text_input`s, which take Ctrl for clipboard, select all and undo. In Alt mode, a small wrapper turns Alt+C/X/V/A/Z (and Shift+Z) into what the input expects before passing the event on. `form_keys.rs` and `date_field.rs` already wrap their inputs; extend them, and add one shared wrapper for the rest. The board's label editor (`wren-canvas`) reads its clipboard and select-all keys from the host's key policy instead of hard-coded Ctrl/⌘. In Ctrl mode no wrapper changes anything.

## 5. The build

The branch is `linux-mod`, from main (8c621f6). Every item follows the tasks runs' loop: brief, builder at the tier given, my check of the diff, gates and screens, one Codex review, fix, commit, RUNLOG entry in `.claude/keys-notes/`.

Gates per item:
- `make check`;
- the item's smoke files, run in both modes;
- the macOS clippy line when `keybind.rs`, `commands.rs`, `preferences.rs` or the menus change.

Existing tests keep their expectations except where a decision above changes them; each such change is listed and approved. The harness pins `linux-modifier` explicitly. Existing smoke files that send `alt+…` app keys run in Alt mode, so they stay valid, and a new smoke file covers Ctrl mode and the switch.

| Item | Tier | Review | What |
|---|---|---|---|
| K1 Keymap core | Opus | sol | `wren-core`: the `mod`/`opt` tokens; `linux-modifier` in `Settings` (parse, template, writer); per-mode default tables for the base, task and board layers; the new `edit.*` actions; the exceptions above; `mod+` in `keybind` lines; collision tests per mode and layer; the frozen keymap tests rewritten deliberately (one Mac table, two Linux tables). |
| K2 Editor and app wiring | Opus | astra | `edit.undo/redo/copy/cut/paste/select_all` routed through the editor's key policy as intents on both engines; the hard-coded Ctrl+Z/Shift+Z/Y/F removed from `wren-editor/src/keys.rs`; the Alt-mode text-field wrapper (§4) for the palette, dialogs, find bar, calendar and form; the palette command that switches modes live; labels follow the mode; `alt_filter` checked in both modes. |
| K3 Board | Opus | sol | Board tables on `mod`/`opt`; Ctrl+Y redo removed; the label editor's clipboard and select-all from the key policy; pointer modifiers stay as they are, documented. |
| K4 Tests | Sonnet | sol | The harness pins the mode; `tools/wrentest/test_keys_modifier.py` covers both modes, the live switch, Alt+C/V/A in the editor, palette, calendar and form in Alt mode, and native X11 chords in each mode; existing files keep running in Alt mode. |
| K5 Docs | Sonnet | sol | `docs/getting-started/keybindings.md` rewritten around Mod (macOS column, Linux "Mod" column, the switch at the top, the exceptions); `docs/reference/configuration.md` (`linux-modifier`); `CLAUDE.md` architecture rule; `docs/development/architecture.md`; the tasks docs (⌘D / Mod+D …); `CHANGELOG.md` under Unreleased, with the new Linux default and the Ctrl+Y change prominent. |
| K6 Finish | — | astra (branch) | One Codex review of the branch; the full matrix (X11 and Wayland × wgpu and tiny-skia, lanes one at a time) in the harness's pinned mode, plus the Ctrl-mode file; the Mac Metal scenes and macOS clippy (Mac defaults unchanged; check ⌘D against the Board menu). Open the PR, watch it green, and **stop for the owner** before merging. |

Stop conditions, as in `discovery/tasks-build.md` §9:
- a change to Mac defaults;
- a collision that can't be resolved without a new owner decision;
- two failed fix rounds on one item;
- CI failing twice for no known reason;
- a touch to `vendor/` or golden files.
