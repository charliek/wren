# Keybindings: one switchable command key on Linux

Status: built on branch `linux-mod` (2026-11-25). Owner decisions are in §2, the build in §5, and what changed in the building in §6.

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

The palette input, dialogs, the find bar, the calendar field and the task form's fields are iced `text_input`s, which take Ctrl for clipboard, select all and undo. In Alt mode, a small wrapper turns Alt+C/X/V/A/Z (and Shift+Z) into what the input expects before passing the event on. `form_keys.rs` and `date_field.rs` already wrap their inputs; extend them, and add one shared wrapper for the rest. The board's label editor (`wren-canvas`) reads its clipboard and select-all keys from the host's key policy instead of hard-coded Ctrl/⌘. With the default Ctrl-mode keymap no wrapper changes anything.

As built (`field_keys.rs`), on Linux only: any `edit.*` binding that is not the input's own Ctrl press is rewritten to it, in either mode, so a Ctrl-mode user's `alt+c = edit.copy` copies in a field too; a native chord the keymap gives another command or unbinds is taken from the focused field, unless it is another edit, which wins; every press first tells the input its real modifiers, so a field mounted with Ctrl held still pastes. The keys sheet's filter gives its own toggle to the sheet at routing time, so `what?` pasted filters and `mod+x = board.keys` closes the sheet without cutting.

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
| K6 Finish | — | astra (branch) and a Fable judge | One branch review by both, in parallel, then one fix round. Verification: CI's four display lanes in the harness's pinned Alt mode, one local full X11 run in Ctrl mode, macOS clippy and one Mac check (Mac defaults unchanged; check ⌘D against the Board menu). Open the PR, watch it green, and merge it as a merge commit (the owner authorised auto-merge). |

Stop conditions, as in `discovery/tasks-build.md` §9:
- a change to Mac defaults;
- a collision that can't be resolved without a new owner decision;
- two failed fix rounds on one item;
- CI failing twice for no known reason;
- a touch to `vendor/` or golden files.

## 6. As built

The owner approved a faster run on 2026-11-25:
- K2, K3 and K5 were built in parallel worktrees;
- K4 was folded into K1 (the harness pin) and K2 (`test_keys_modifier.py`);
- each item had one Codex review, with a second only for severity 4 or more;
- the final verification is CI plus one local run in Ctrl mode.

What differs from §3 and §4:
- **Word motion.** On Linux it is Ctrl+←/→ only; Alt+←/→ was never bound. Alt+Backspace and Alt+Delete still delete a word.
- **`edit.*` is Linux only.** On macOS the editor keeps its own ⌘Z, ⌘⇧Z and ⌘F, and a `keybind` line naming an `edit.*` action is refused. The palette rows are hidden.
- **Unbound and reassigned clipboard chords.** An explicitly unbound Ctrl+C/X/V/A does nothing, in the editor and in a focused field. One given to another command does only that command.
- **Text fields have no undo.** iced's text input has none, so Alt+Z and Ctrl+Z do nothing there.
- **Branch review fixes** (2026-11-25):
  1. a native chord bound to a Markdown command stays that command's with Markdown assist off, so iced neither cuts nor copies for it;
  2. a native chord given another edit (Alt mode, `ctrl+v = edit.copy`) does that edit in a field, not nothing;
  3. an open palette is redrawn after a keymap change read from the file, keeping its query and row;
  4. the keys sheet's own key, bound to a clipboard chord, closes the sheet and edits nothing in its filter: the focused filter is its one owner, and the app no longer closes the sheet from its own copy of the press (which closed it again on the press that opened it);
  5. the sheet closes on its key pressed, not on a `?` in the filter's text;
  6. field rewrites apply in both Linux modes (the default Ctrl keymap has none), and the keymap records the triggers a line unbinds (`Keymap::unbound`).
- **CI** runs one Linux smoke lane (X11, wgpu) in Ctrl mode, the shipped default.
- **Ctrl+Y in Ctrl mode is Focus mode,** on a board too, where an open label stays open.
- **Board labels.** The board's select all, copy, cut, paste, undo and redo keys act on an open label's text. Typed text, AltGr text and the label's caret and erase keys win over any binding. A held paste repeats in a label.
- **Harness.** `WRENTEST_LINUX_MODIFIER` picks a lane's mode (default `alt`). `mod` and `opt` work in `editor.key` and `board.key`, and `native_mod`, `native_opt` and `mod_label` cover native chords.

Deferred:
- the settings writer turns CRLF line endings into LF when it replaces a line (pre-existing);
- on macOS, changing only `linux-modifier` reinstalls the native menu.
