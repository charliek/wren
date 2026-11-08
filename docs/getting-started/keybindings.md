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

In the palette, use ↑ and ↓ to move, ↵ to run the highlighted row, and Esc
to close. While the palette or a dialog is open, other shortcuts are
ignored, except the palette shortcut itself (which closes the palette) and
Quit.

## Inside the editor

| Action | macOS | Linux |
|---|---|---|
| Undo | ⌘Z | Ctrl+Z |
| Redo | ⇧⌘Z | Ctrl+Shift+Z or Ctrl+Y |
| Delete the previous word | ⌥⌫ | Ctrl+Backspace or Alt+Backspace |
| Delete the next word | ⌥⌦ | Ctrl+Delete or Alt+Delete |
| Delete to the start of the line | ⌘⌫ | — |
| Start or end of the note | ⌘↑ / ⌘↓ | Ctrl+Home / Ctrl+End |
| Start of the line (smart, below) | Home, ⌘←, or ⌃A | Home |

Add Shift to Home and the motions to select as you go. Word deletes take
the spaces next to the cursor with the word, and stop at punctuation that
is not part of a word, so `foo!!` loses one `!`. Backspace and Delete
always remove a whole character, so an emoji or an accented letter goes in
one press. Tab never moves the keyboard out of the editor.

### Markdown assist

With `markdown-assist = on`, the default, the editing keys follow the
Markdown around the cursor. `‸` marks the cursor:

| Key | Before | After |
|---|---|---|
| Enter | `- item‸` | `- item`, then `- ‸` on a new line. Quotes (`> `), numbered lists (`9.` then `10.`), and task boxes (a new `- [ ] `) continue the same way. |
| Enter on an empty item | `- ‸` | A nested item moves out one level. At the top level the marker goes, and a blank line separates the list from what you type next. |
| Shift+Enter | `- [ ] a‸` | A new line inside the item, indented under its text, with no marker. |
| Tab | `- a`, `- b‸` | `- b` becomes a child of `- a`, with its own children. A numbered item restarts at 1, and both lists are renumbered. On other lines Tab inserts spaces to the next multiple of 4. |
| Shift+Tab | a nested item | The item moves out one level and takes that level's marker. |
| Backspace | `- [ ] ‸` | `- ‸`, then `‸`: the task box goes first, then the marker. A nested marker moves out a level instead, and `> > ‸` loses one `>`. In indentation, Backspace goes back to the previous indent stop. |
| Home | `- [ ] task‸` | `- [ ] ‸task`, then `‸- [ ] task`, then back. On a long line that wraps, the first press goes to the start of the wrapped row. |
| ⌘⌫ | `- [ ] foo‸` | `- [ ] ‸`, then `‸`. |

Enter in a numbered list renumbers the items after it while they count up
in order; a deliberate gap or a countdown is left alone. Inside code
blocks, front matter, and HTML, Enter only keeps the indentation and any
quote or list prefix, and Tab inserts a tab or spaces like the line above.

With `markdown-assist = off` in [`config.conf`](../reference/configuration.md),
Enter, Shift+Enter, and Home are plain again. Backspace still deletes a
whole character, and Tab still inserts spaces to the next multiple of 4.
Over a selection that spans lines, Tab indents each line by 4 spaces (a tab
on a line that starts with one) instead of replacing them, and Shift+Tab
takes one step off each line, or off the cursor's line. ⌘⌫ deletes to the
start of the line, and the word deletes and note motions work as above.

While an input method is composing text, Enter, Tab, Backspace, and Delete
belong to it, with or without modifiers, and the Markdown commands below
wait until it finishes.

### Markdown commands

| Action | macOS | Linux | Action name |
|---|---|---|---|
| Toggle a task | ⌘↵ | Ctrl+Enter | `markdown.toggle_task` |
| Bold | ⌘B | Ctrl+B | `markdown.bold` |
| Italic | ⌘I | Ctrl+I | `markdown.italic` |
| Inline code | ⌘E | Ctrl+E | `markdown.code` |
| Move line up / down | ⌥↑ / ⌥↓ | Alt+↑ / Alt+↓ | `edit.move_line_up` / `edit.move_line_down` |

- **Toggle a task** checks `- [ ] a` as `- [x] a` and back. A list item
  or a plain line becomes an open task (`- a` and `a` both become
  `- [ ] a`), and a numbered item keeps its number (`1. [ ] a`). Over a
  selection, every task is checked if any is open, otherwise all are
  reopened.
- **Bold, Italic, and Inline code** wrap the selection, or the word at the
  cursor, in `**`, `*`, or `` ` ``; with no word there, they insert an empty
  pair around the cursor. Pressing the key again on wrapped text unwraps it.
  A selection over several lines wraps each line.
- **Move line** moves the lines the selection covers past the line above or
  below, keeping the selection, and renumbers a numbered list on both sides.

These keys belong to the editor: they work only while it has the keyboard,
so in the find bar, a filter box, or a browser column they do nothing. The
palette lists them under **Markdown**; run from there, they act at the
editor's cursor and give it the keyboard. They are unavailable while
`markdown-assist = off`. On macOS, ⌘ with a letter never types the letter,
even when nothing is bound to it.

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
title bar for that action. A key bound to any command other than the
Markdown commands above is the app's even inside the editor, so binding
`ctrl+z` to something else takes it from Undo.

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
palette.commands  markdown.toggle_task  markdown.bold  markdown.italic
markdown.code  edit.move_line_up  edit.move_line_down
```

An unknown action or an invalid trigger is skipped, and the line is named
in the warning banner.
