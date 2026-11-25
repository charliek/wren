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
| Toggle collapsed columns | ⇧⌘Y | Alt+Shift+Y | `view.collapse_columns` |
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
| Cut, copy, paste | ⌘X, ⌘C, ⌘V | Ctrl+X, Ctrl+C, Ctrl+V |

Add Shift to Home and the motions to select as you go. Word deletes take
the spaces next to the cursor, then one word or one run of punctuation:
`foo bar  ` loses `bar  `, `foo!!` loses `!!`, and `**bold**` loses the
closing `**`. Words follow Unicode's rules, so `a.b`, `3.14`, and `can't`
go whole, and a run of Chinese or Japanese characters (Han, Hiragana, and
Katakana) goes together up to the next space or punctuation. At either end
of a line, a word delete joins the next or previous line. Backspace and
Delete always remove a whole character, so an emoji or an accented letter
goes in one press. Tab never moves the keyboard out of the editor.

With nothing selected, cut and copy take the whole line the cursor is on,
with its line break, as in VS Code. Cut removes the line, and the cursor
keeps its column on the line that moves up. Pasting a line copied or cut
that way puts it back as a whole line above the cursor's, wherever the
cursor is in its line; anything else on the clipboard pastes at the
cursor, and with a selection a paste replaces it as usual.

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
or after the palette's **Toggle Markdown assist** (which saves the choice
there and shows whether assist is on), Enter, Shift+Enter, and Home are
plain again. Backspace still deletes a
whole character, and Tab still inserts spaces to the next multiple of 4.
Over a selection that spans lines, Tab indents each line by 4 spaces (a tab
on a line that starts with one) instead of replacing them, and Shift+Tab
takes one step off each line, or off the cursor's line. ⌘⌫ deletes to the
start of the line, and the word deletes and note motions work as above.

While an input method is composing text, Enter, Tab, Backspace, and Delete
belong to it, with or without modifiers, and the Markdown commands below
wait until it finishes. When you start composing with the cursor scrolled
out of view, the editor scrolls back to it, so the candidate window opens
beside the text you are writing.

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
  selection that holds a task, every task that is not done is checked if
  any is unfinished (open, doing, or a box with no status of its own), and
  a done task stays as it is; otherwise all are reopened, cancelled ones
  too. Lines that are not tasks get a box only when the selection holds no
  task. With `tasks = on`, checking a task also writes today's done date
  (`✅ 2026-11-19`) and reopening one removes it, and a cancelled task
  (`[-]`) reopens and loses its `❌` date. Typing `x` between the brackets
  yourself writes no date. See [Tasks](../reference/tasks.md#toggle-a-task).
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
`markdown-assist = off`, and their keys then do what they do in any text:
⌘↵ (Ctrl+Enter) starts a new line, and ⌥↑ / ⌥↓ (Alt+↑ / Alt+↓) move the
cursor. On macOS, ⌘ with a letter never types the letter, even when
nothing is bound to it.

### Spell check

| Action | macOS | Linux | Action name |
|---|---|---|---|
| Suggest spelling | ⌘. | Alt+. | `editor.spell_suggest` |

On the `advanced` engine, with the caret in or touching an underlined
word, **Suggest spelling** opens the corrections menu under it; a
right-click on the word does the same. ↑ and ↓ move through the
corrections, ↵ replaces the word, and Esc closes the menu. Like the
Markdown commands, the key belongs to the editor, and the palette lists
the command under **Markdown**; unlike them, it works whether or not
Markdown assist is on. See [Spell check](../reference/spell-check.md).

### Task menus

With `tasks = on`, on the `advanced` engine, a right-click on a task line,
or on a line of its text after the box, opens a menu of what can be done to
the task; on a misspelled word the
corrections menu comes first, as before. A right-click on the box opens
the status menu, on the priority glyph the priority menu, and on the due
date the calendar (below); anywhere else on the line it opens the task
menu, which holds the status rows, **Set due date…** (which opens the
calendar; it reads **Change due date…** when the task has one),
**Priority…** (which opens the priority menu) and **Edit task…** (which
opens the [task form](#the-task-form) for the task you clicked, whatever
line the caret is on).

Each menu marks the task's current status or priority and opens on it; an
unknown box has no status row to mark. ↑
and ↓ move, ↵ chooses, Esc closes it and writes nothing, and a single key
chooses a row directly:

| Menu | Keys |
|---|---|
| Status | Space Open, `/` Doing, `x` Done, `-` Cancelled: the characters that go in the box |
| Priority | `h` High, `m` Medium, `l` Low, ⇧H Highest, ⇧L Lowest, `n` None |
| Task | the status keys, then `d` Set or Change due date…, `p` Priority…, `e` Edit task… |

A choice rewrites the task's line as one undo step, with the done or
cancelled date the status asks for. A click on a task's box cycles its
status, open → doing → done → open; a cancelled task reopens. The caret
stays where it was. A box with a character that has no status (`[?]`,
`[>]`) is not cycled: the click places the caret, and the right-click menu
still sets its status. The status changes 300 ms after the press, once a
double click could no longer follow; a second click in that time cancels
it. While the button is held, a key, another button, a modifier or the
window losing focus cancels it too. A click with a modifier held, a drag
that starts on the box, and a press let go off the box select text or place
the caret as they would anywhere else.

**Set task due date** (⌘D / Ctrl+D) opens the calendar at the caret's
task, on the `advanced` engine: on the task's due date, or today, hung
under the date or the caret. Its field has the keyboard:

| Key | Does |
|---|---|
| Typing | a date: `today`, `tomorrow`, a weekday (`fri`, the next one), `next mon`, `+3d`, `+2w`, `+1m`, or `2026-12-01` |
| ↑ ↓ | a week earlier or later |
| ← → | a day earlier or later, while the field is empty |
| PgUp PgDn | a month earlier or later |
| ↵ | writes the date, or replaces the one there |
| Esc | closes it and writes nothing |

The line under the field says what ↵ would write, or that the field holds
no date, in which case ↵ writes nothing. A click on a day or on a quick
pick writes it at once, and **Clear** removes the due date. A new date is
written as `📅 2026-12-01`, before any done or cancelled date and the
`^block-id` that follow it; a date already there is changed where it
stands, keeping a `🗓`, a `📆` or a `[due:: …]` as written. Each is one undo
step. A click outside the
calendar, a scroll, or a change to the note closes it and writes nothing.
Keys typed in the same instant as ⌘D / Ctrl+D, before the calendar shows,
are dropped rather than typed into the note.

### The task form

**Create or edit task** (⇧⌘↵ / Ctrl+Shift+Enter, `task.edit`, or **Edit
task…** in the task menu) opens a form for the line at the caret, on either
engine: the description, the status, the due date, the priority and the
owners, with a **Writes** line that shows the Markdown saving will write as
you change them. It hangs near the top of the window, as the palette does,
so the calendar can open under its due field. On a task it edits the task.
On a blank line or an empty list item it makes a new task (`- [ ] `, inside
a quote when the line is in one). On a list item or a line of text that is
not a task it makes that text a task, with the text as the description, as
**Toggle task** would. A heading, a thematic break, a code block or front
matter has no form.

| Key | Does |
|---|---|
| Tab, ⇧Tab | the next or the previous field, round the ends: description, status, due, priority, owners, Save, Cancel |
| ← → | on the status or the priority, the previous or the next choice |
| Space `/` `x` `-` | on the status, Open, Doing, Done or Cancelled |
| `h` `m` `l` ⇧H ⇧L `n` | on the priority, High, Medium, Low, Highest, Lowest or None |
| ↓ | in the due field, opens the calendar under it; whatever closes it gives the due field the keyboard back |
| ↵ | saves, from any field but these: in the owners field with a name typed it adds that name as typed, or the owner chosen with ↑ ↓; on Save or Cancel it presses the button |
| Space | on Save or Cancel, presses the button |
| ↑ ↓ | in the owners field, choose among the owners already used in the workspace |
| Tab | in the owners field with a name typed, adds the owner chosen with ↑ ↓, or else the first offered, then moves on |
| ⌫ | in an empty owners field, removes the last owner |
| Esc | in the owners field with a name typed, empties it; otherwise closes the form and writes nothing |
| ⇧⌘↵ / Ctrl+Shift+Enter, ⌘↵ / Ctrl+Enter | save, from any field |

A click outside the form closes it and writes nothing. The due field takes
the same [date words](../reference/tasks.md#date-words) as the calendar; the
line under it says what it names. A name left typed in the owners field is
in the **Writes** line, and saved, as typed. Removing an owner and adding
the name back puts the mention back where it was written.

Saving writes only the fields you changed, as one undo step, so a line you
wrote in your own order keeps it, and saving a form you did not change on
a task writes nothing (on a line that is not a task yet, it makes the
task). A due date already on the task that names no real day is kept as
written when you change only other fields. A due field you typed that is
not a date, or not a real day, cannot be saved: the line under it says so
and the form stays open. If the note changed while the form was open (it
was reloaded from disk, say), saving says so and writes nothing; Cancel and
open the form again.

While the form is open, the app's shortcuts do nothing, except Quit. A key
that saves, cancels, presses a button or opens the calendar is the last
input the form takes from its batch: keys typed in the same instant after
it are dropped, not typed into the note. While an input method is
composing, Enter and Esc belong to it. Closing the form gives the editor
the keyboard back with its caret where it was.

### Task suggestions

With `tasks = on` and `tasks-suggest = on`, on the `advanced` engine, a
task line offers a short list under the word you type. Two letters at the
start of a word open a due date or a priority, never in the description's
first word; `@` alone opens owners, in the first word too. See
[Tasks](../reference/tasks.md#suggestions) for what each word offers.

| Key | Does |
|---|---|
| Tab | picks the highlighted row |
| ↑ ↓ | move; ↵ then picks the row moved to |
| ↵ | with no ↑ or ↓ first, continues the list and keeps the word as typed. Only **Due date…** for `du`, `da` or `dat` opens the calendar |
| Esc | hides the list for the rest of the line |
| any other key | types, and the list follows the word |

An owner is picked by Tab, or by ↑ or ↓ and ↵; ↵ alone never completes a
name. A row that ↵ alone would not pick is drawn lightly, with "Tab picks
· ↑↓ ↵" under the list. With no list open, Tab indents and ↵ continues the
list as always. The list opens only with the caret at the end of a word,
and not while an input method is composing. Tab typed in the same instant
as the word, before the list shows, is dropped, and ↵ then continues the
list (only **Due date…** still opens the calendar).

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
row gets the selection fill and an accent border. The selected rows in the
other columns keep a quieter fill, so the path to the open note stays
visible.

| Key | In Sections, Groups, or Notes |
|---|---|
| ↑ / ↓ | Select the previous or next row. Selection is live: the next column, and the note, follow right away. |
| ↵, →, or Tab | Move one column right. From Notes, this moves into the editor. Needs a selected row. Tab in a filter box stays there. |
| ← or Shift+Tab | Move one column left. |
| Esc | Return to the editor without changing the selection. |

Column left and right work everywhere, including from the editor, where
the arrow keys and ↵ keep their usual editing jobs. A single click on a row
selects it, the same as the arrow keys; a double-click selects it and moves
one column right.

Moving right (↵, →, Tab, ⇧⌘], or a double-click) folds the columns to the
left of the active one, so the editor gets their width. Groups folds
Sections; Notes and the editor fold Sections and Groups. Notes never folds;
focus mode (⌘Y) hides it. ⇧⌘Y (Alt+Shift+Y) folds Sections and Groups and
leaves focus mode; press it again to unfold them. From the editor or Notes
the keyboard stays where it is, and the editor keeps its cursor; from
Sections or Groups, Notes takes the keyboard. Folded columns show as a path at the top of the
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

A new section or group is selected, and its column opens and takes the
keyboard; a new note opens in the editor. Picking a section or group from
its jump list selects it and moves one column right, like ↵. Pressing ⌘P again in that list widens it to every
note. Esc in a filter box leaves the filter and keeps the column. Commands
chosen from the palette always do what their label says, whichever column
is active.

## On a board

These keys work while a [board](../reference/boards.md) has the keyboard.
Their modifier is **Mod**: ⌘ on macOS and **Ctrl** on Linux, as in text
editing, so Linux board shortcuts take Ctrl where the app's take Alt. The
app's shortcuts in the first table above keep working on a board.

| Action | macOS | Linux | Action name |
|---|---|---|---|
| Select tool / Hand tool | V / H | V / H | `board.tool.select` / `board.tool.hand` |
| Rectangle, ellipse, diamond, pill, cylinder, parallelogram tool | R, O, D, U, Y, P | the same | `board.tool.rectangle`, `.ellipse`, `.diamond`, `.pill`, `.cylinder`, `.parallelogram` |
| Rounded rectangle, document tool | — | — | `board.tool.rounded`, `board.tool.document` |
| Text tool / Connector tool | T / C or A | the same | `board.tool.text` / `board.tool.connector` |
| Change shape | ⇧ + the tool letter | Shift + the tool letter | `board.shape.rectangle`, … (`rounded` and `document` have no key) |
| Edit label / Finish label | ↵ or F2 / ⌘↵ | Enter or F2 / Ctrl+Enter | `board.label.edit` / `board.label.commit` |
| Add next shape | Tab | Tab | `board.connect.next` |
| Connect left, right, up, down | ⌘ + arrow | Ctrl + arrow | `board.connect.left`, … |
| Select left, right, up, down | arrow | arrow | `board.select.left`, … |
| Extend selection | ⇧ + arrow | Shift + arrow | `board.select.extend_left`, … |
| Select the connector on that side | ⇧⌘ + arrow | Ctrl+Shift + arrow | `board.select.connector_left`, … |
| Select all / Select connected | ⌘A / ⇧⌘A | Ctrl+A / Ctrl+Shift+A | `board.select.all` / `board.select.connected` |
| Clear selection | — (Esc clears it too) | — (Esc clears it too) | `board.select.none` |
| Move one cell | ⌥ + arrow | Alt + arrow | `board.move.left`, … |
| Wider, narrower, taller, shorter | ⌥⇧ + → ← ↓ ↑ | Alt+Shift + → ← ↓ ↑ | `board.resize.wider`, `.narrower`, `.taller`, `.shorter` |
| Delete / Duplicate | ⌫ or ⌦ / ⌘D | Backspace or Delete / Ctrl+D | `board.delete` / `board.duplicate` |
| Copy / Cut / Paste | ⌘C / ⌘X / ⌘V | Ctrl+C / Ctrl+X / Ctrl+V | `board.copy` / `board.cut` / `board.paste` |
| Undo / Redo | ⌘Z / ⇧⌘Z | Ctrl+Z / Ctrl+Shift+Z or Ctrl+Y | `board.undo` / `board.redo` |
| Zoom to fit / Zoom to selection | ⇧1 / ⇧2 | Shift+1 / Shift+2 | `board.zoom.fit` / `board.zoom.selection` |
| Zoom in / out / to 100 % | ⌘= / ⌘- / ⌘0 | Alt+= / Alt+- / Alt+0 | `font.increase` / `font.decrease` / `font.reset` |
| Bring to front / Send to back | ] / [ | the same | `board.arrange.front` / `board.arrange.back` |
| Bring forward / Send backward | ⇧] / ⇧[ | Shift+] / Shift+[ | `board.arrange.forward` / `board.arrange.backward` |
| Colour: neutral, red, orange, yellow, green, cyan, purple, blue | 0 to 7 | 0 to 7 | `board.color.neutral`, `.red`, `.orange`, `.yellow`, `.green`, `.cyan`, `.purple`, `.blue` |
| Board actions | / or ⇧F10 | / or Shift+F10 | `board.actions` |
| Jump to node | ⇧⌘O or ⌘F | Alt+Shift+O, Alt+F, or Ctrl+F | `board.jump` (reached through `navigation.outline` and `navigation.find`) |
| Board keys (the keys sheet) | ? | ? | `board.keys` (bound as `shift+/`; a `?` typed with another key, as on AZERTY, works too) |

These have no default key; run them from the command palette or the
[actions palette](../reference/boards.md#the-actions-palette), or bind
them:

| Actions | Action names |
|---|---|
| Align left, centre, right, top, middle, bottom | `board.align.left`, `.center`, `.right`, `.top`, `.middle`, `.bottom` |
| Distribute horizontally, vertically | `board.distribute.horizontal`, `board.distribute.vertical` |
| Fill: none, tint, solid | `board.fill.none`, `board.fill.tint`, `board.fill.solid` |
| Text size: 12, 14, 20, 28 | `board.text_size.12`, `.14`, `.20`, `.28` |
| Connector: straight or elbow / start head / end head / reverse | `board.edge.routing`, `board.edge.start`, `board.edge.end`, `board.edge.reverse` |
| Solid or dashed | `board.stroke.dashed` |
| New board, Export board as SVG, PNG, draw.io | `note.new_board`, `note.export_svg`, `note.export_png`, `note.export_drawio` |

A tool's key pressed while that tool is armed locks it, so it stays armed
after each shape it makes; Esc, V, or another tool's key unlocks it. The
hand tool stays armed until another tool is picked. Holding Space shows
each key on the board's [tool dock](../reference/boards.md#the-tool-dock).

Esc steps back one thing at a time: it finishes an open label, ends a
connect, puts away an armed tool, then clears the selection. It never
leaves the board.

While a label is open, only Finish label (⌘↵ / Ctrl+Enter) and Add next
shape (Tab) act as board keys, and every other key edits the label's text,
so ⌘C, ⌘Z, arrows, and Backspace are the label's. While you connect from
the keyboard (C with one shape selected), letters type into the search for
a target. The app's own shortcuts, such as the command palette, work in
both.

The Board group of the command palette appears only while a board is open,
and the Markdown group is hidden then. With `boards = off` no board
command appears anywhere.

**The digit row.** The colour keys are the digits as typed. On a layout
that types a symbol there without Shift, such as AZERTY (`&` on the 1 key),
a digit-row key whose character nothing is bound to acts as its digit, so
0 to 7 still colour the selection. With nothing selected, 0 to 7 pick the
colour new shapes take and change nothing on the board.

**Alt+Shift on Linux.** Some Linux desktops use Alt+Shift to switch
keyboard layout, which can take the resize keys (Alt+Shift + arrow) and the
app's Alt+Shift shortcuts before Wren sees them. Change the desktop's
layout switch, or bind the commands to other keys.

**Board keybindings.** Board actions bind with the same `keybind` lines
(see below), and a bare key such as `x` is allowed for them:

```conf
keybind = x = board.delete          # X deletes on a board
keybind = tab = board.unbind        # Tab no longer adds the next shape
```

`board.unbind` removes a board binding, and `unbind` removes only an app
binding. A board binding on a key the app already uses for one of its own
commands, or on Esc, is dropped with a warning, since the app's shortcut
would always win.

## Custom keybindings

Add `keybind` lines to [`config.conf`](../reference/configuration.md):

```conf
keybind = <trigger> = <action>
```

Lines apply in order on top of the defaults. A later line for the same
trigger replaces the earlier one, and the action `unbind` removes a
binding. A user binding also becomes the shortcut shown in the palette and
title bar for that action. A key bound to any command other than the
Markdown commands and Suggest spelling above is the app's even inside the
editor, so binding
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
navigation.find  navigation.outline  navigation.back  navigation.forward
navigation.column_left  navigation.column_right  view.focus
view.collapse_columns  view.reset_widths  notes.sort_modified  notes.sort_title  theme.choose
font.choose  font.increase  font.decrease  font.reset  workspace.switch
workspace.add  workspace.remove  settings.open  settings.reload  app.quit
palette.commands  markdown.toggle_task  markdown.bold  markdown.italic
markdown.code  edit.move_line_up  edit.move_line_down
editor.toggle_markdown_assist  editor.toggle_engine
editor.toggle_spell_check  editor.spell_suggest  settings.open_words
note.new_board  note.export_svg  note.export_png  note.export_drawio
```

With `tasks = on`, the task line commands can be bound too; they act on
the task at the caret, and only `task.set_due` (⌘D / Ctrl+D) and
`task.edit` (⇧⌘↵ / Ctrl+Shift+Enter) have default keys:

```
task.set_status  task.cycle_status  task.set_priority  task.priority.high
task.priority.medium  task.priority.low  task.priority.highest
task.priority.lowest  task.priority.none  task.priority.raise
task.priority.lower  task.set_due  task.clear_due  task.edit
```

`task.set_status` and `task.set_priority` open the status and priority
menus, and `task.set_due` the calendar, described under
[Task menus](#task-menus), on the `advanced` engine. `task.edit` opens the
task form, on either engine.

The board actions are listed under [On a board](#on-a-board).

`editor.toggle_markdown_assist`, `editor.toggle_engine`, and
`editor.toggle_spell_check` have no default key. They flip
`markdown-assist`, `editor-engine`, and `spell-check`, save the change to
`config.conf`, and apply it at once; switching engines keeps the note's
text and cursor but clears its undo history. `settings.open_words`, which
opens the spelling word list, has no default key either.

An unknown action or an invalid trigger is skipped, and the line is named
in the warning banner.
