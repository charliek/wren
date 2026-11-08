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
| Jump to heading | ⇧⌘O | Alt+Shift+O | `navigation.outline` |
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

## Jumping to a heading

**Jump to heading** (⇧⌘O / Alt+Shift+O) opens the open note's outline: its
headings in order, indented by level, each with the headings it sits under.
Headings inside code blocks and front matter are left out. Type to filter
the list, as in the other palettes. It opens on the section the cursor is
in.

Moving the highlight shows that heading in the editor, scrolled near the
top. ↵ keeps it and returns the keyboard to the editor; Esc, or closing the
palette any other way, puts the cursor, the selection, and the scroll back
where they were. The `classic` [editor engine](../reference/configuration.md)
places the heading and returns the scroll as nearly as it can; `advanced`
does both exactly. The outline also works in the in-memory editor
(`wren --editor`).

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

A few shortcuts act on the active column:

| Shortcut | Sections | Groups | Notes | Editor |
|---|---|---|---|---|
| ⌘P / Alt+P | Jump to section | Jump to group, this section's first | Jump to note | Jump to note |
| ⌘N / Alt+N | New section | New group | New note | New note |
| ⌘R / Alt+R | Rename section | Rename group | Rename note | Rename note |
| ⌘F / Alt+F | Filter sections | Filter groups | Search titles and text | Find in note |

Picking a section or group from its jump list selects it and moves one
column right, like ↵. Pressing ⌘P again in that list widens it to every
note. Esc in a filter box leaves the filter and keeps the column. Commands
chosen from the palette always do what their label says, whichever column
is active.

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
navigation.find  navigation.outline  navigation.back  navigation.forward
navigation.column_left  navigation.column_right  view.focus
view.reset_widths  notes.sort_modified  notes.sort_title  theme.choose
font.choose  font.increase  font.decrease  font.reset  workspace.switch
workspace.add  workspace.remove  settings.open  settings.reload  app.quit
palette.commands
```

An unknown action or an invalid trigger is skipped, and the line is named
in the warning banner.
