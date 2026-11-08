# Keybindings

Wren uses each platform's app modifier: **Command** on macOS and **Alt** on
Linux. Ctrl stays with text editing on Linux, so Ctrl+Z, Ctrl+C, Ctrl+V,
and Ctrl+A keep working in the editor and in every text field.

| Action | macOS | Linux | Action name |
|---|---|---|---|
| Command palette | ⇧⌘P or ⌘K | Alt+Shift+P or Alt+K | `palette.commands` |
| Jump to note | ⌘P | Alt+P | `navigation.jump` |
| Search workspace | ⇧⌘F | Alt+Shift+F | `navigation.search` |
| Find in note | ⌘F | Alt+F or Ctrl+F | `navigation.find` |
| New note | ⌘N | Alt+N | `note.new` |
| New dated note | ⇧⌘N | Alt+Shift+N | `note.new_dated` |
| Back / Forward | ⌘[ / ⌘] | Alt+[ / Alt+] | `navigation.back` / `navigation.forward` |
| Column left / right | ⇧⌘[ / ⇧⌘] | Alt+Shift+[ / Alt+Shift+] | `navigation.column_left` / `navigation.column_right` |
| Rename note | ⌘R | Alt+R | `note.rename` |
| Toggle focus mode | ⌘Y | Alt+Y | `view.focus` |
| Increase text size | ⌘+ or ⌘= | Alt++ or Alt+= | `font.increase` |
| Decrease text size | ⌘- | Alt+- | `font.decrease` |
| Reset text size | ⌘0 | Alt+0 | `font.reset` |
| Open settings file | ⌘, | — | `settings.open` |
| Quit (saves first) | ⌘Q | Ctrl+Q | `app.quit` |

Inside the editor: undo is ⌘Z / Ctrl+Z, and redo is ⇧⌘Z / Ctrl+Shift+Z
(Ctrl+Y also works on Linux). In the palette, use ↑ and ↓ to move, ↵ to run
the highlighted row, and Esc to close. While the palette or a dialog is
open, other shortcuts are ignored, except the palette shortcut itself
(which closes the palette) and Quit.

## Moving between columns

One of Sections, Groups, Notes, or the editor has the keyboard at a time.
Its header turns to the accent color with a small dot, and its selected
row gets an accent border.

| Key | In Sections, Groups, or Notes |
|---|---|
| ↑ / ↓ | Select the previous or next row. Selection is live: the next column, and the note, follow right away. |
| ↵ or → | Move one column right. From Notes, this moves into the editor. Needs a selected row. |
| ← | Move one column left. |
| Esc | Return to the editor without changing the selection. |

Column left and right work everywhere, including from the editor, where
the arrow keys and ↵ keep their usual editing jobs. A single click on a row
selects it, the same as the arrow keys; a double-click selects it and moves
one column right.

Moving right (↵, →, ⇧⌘], or a double-click) folds the columns to the left
of the active one, so the editor gets their width. Groups folds Sections;
Notes and the editor fold Sections and Groups. Notes never folds; focus
mode (⌘Y) hides it. Folded columns show as a path at the top of the
leftmost open column, `‹ Section › Group`: click a name, or `‹`, to open
that column again. Moving left only ever unfolds, and a single click never
changes what is folded. Every column is open again after a restart.

## Custom keybindings

Add `keybind` lines to [`config.conf`](../reference/configuration.md):

```conf
keybind = <trigger> = <action>
```

Lines apply in order on top of the defaults. A later line for the same
trigger replaces the earlier one, and the action `unbind` removes a
binding. A user binding also becomes the shortcut shown in the palette and
title bar for that action.

```conf
keybind = ctrl+shift+p = palette.commands   # add a second palette shortcut
keybind = alt+k = unbind                    # free Alt+K
keybind = alt+t = theme.choose              # bind a palette-only command
```

**Modifiers:** `shift`, `ctrl` (`control`), `alt` (`opt`, `option`), and
`super` (`cmd`, `command`). **Keys:** letters, digits, punctuation such as
`[`, `]`, `\`, `,`, written either literally or by name (`bracketleft`,
`backslash`, `comma`, `plus`), plus `enter`, `escape`, `space`, `tab`,
`backspace`, `delete`, arrows (`left`, `right`, `up`, `down`), `home`,
`end`, `pageup`, `pagedown`, and `f1`–`f24`. Keys match the physical key,
so bindings keep working on non-Latin layouts and with macOS Option
characters.

**Actions:** any palette command's name can be bound:

```
note.new  note.new_dated  note.rename  note.delete  note.reveal  note.move
note.open_conflict  group.new  group.rename  group.delete  group.reveal
group.move  group.reorder  section.new  section.rename  section.delete
section.reveal  section.reorder  navigation.jump  navigation.search
navigation.find  navigation.back  navigation.forward
navigation.column_left  navigation.column_right  view.focus
view.reset_widths  notes.sort_modified  notes.sort_title  theme.choose
font.choose  font.increase  font.decrease  font.reset  workspace.switch
workspace.add  workspace.remove  settings.open  settings.reload  app.quit
palette.commands
```

An unknown action or an invalid trigger is skipped, and the line is named
in the warning banner.
