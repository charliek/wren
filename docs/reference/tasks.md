# Tasks

A task is a Markdown list item with a box: `- [ ] Send the deck`. With
tasks on, Wren reads more of that line: a status in the box, a due date, a
priority and owners after the description. It colours them, spells out the
task under the caret in the status bar, and offers menus, a calendar,
suggestions and a form to change them. Every change is written into the
note as plain text, so the file stays an ordinary Markdown note. The
syntax is Obsidian Tasks', so the two read the core of a line alike;
[Compared with Obsidian Tasks](#compared-with-obsidian-tasks) lists where
they differ.

```markdown
- [ ] Send the deck to finance @sam ⏫ 📅 2026-12-01
- [/] Review the budget @priya 📅 2026-11-23
- [x] Book the room 📅 2026-11-20 ✅ 2026-11-19
- [-] Old idea ❌ 2026-11-18
```

## Turning tasks off

Tasks are on by default. To turn them off, set `tasks = off` in
[`config.conf`](configuration.md) and restart Wren. Until then the banner
says "Restart Wren to apply `tasks`."

With `tasks = off`, task lines are plain Markdown checkboxes again:

- A box holds a space, `x` or `X`. `[/]` and `[-]` are text.
- The Markdown toggle writes no done date.
- Highlighting, [spell check](spell-check.md) and search read a task line
  as plain Markdown.
- No task command is listed, its keys do nothing, and a `keybind` line
  that names one is skipped with a warning.
- There is no [summary view](#the-summary-view) and no count in a column
  header, and Wren does not read or write `.wren/tasks.toml`.

`tasks-suggest = off` turns off only the [suggestions](#suggestions) while
typing. It applies when the settings reload.

## A task line

A task is a list item (`-`, `*`, `+` or a number, in a quote too) whose box
holds one character. Tasks in code blocks, front matter and HTML are not
tasks.

| Box | Status | Counts as |
|---|---|---|
| `[ ]` | Open | open |
| `[/]` | Doing | doing |
| `[x]`, `[X]` | Done | done |
| `[-]` | Cancelled | cancelled |
| any other one character | Unknown | open |

An unknown box keeps its character. The status bar names it, for example
`Open [a]`. The box holds one Unicode character other than `]`: `[é]` and
`[✓]` are tasks, `[✔️]` (two characters) and `[]` are not. `[[]` is a task.
`- [a]: https://example.com` is a link reference definition, not a task,
with tasks on or off: a box is followed by white space.

### After the description

Wren reads these tokens from the end of the line, as Obsidian Tasks does.

| Token | Written as | Notes |
|---|---|---|
| Due date | `📅 2026-12-01` | `📆`, `🗓` and `[due:: 2026-12-01]` are read as due dates too. A new date is always written with `📅`. |
| Done date | `✅ 2026-11-19` | Written when a task becomes done. |
| Cancelled date | `❌ 2026-11-19` | Written when a task becomes cancelled. |
| Priority | `🔺` Highest, `⏫` High, `🔼` Medium, `🔽` Low, `⏬` Lowest | A line with no priority sorts between Medium and Low. |
| Owner | `@sam` | Anywhere in the description, or among the tokens. |
| Id | `🆔 abc123` | Kept. |
| Block id | `^abc` | Kept, always last. |

Dates are ISO, `YYYY-MM-DD`. A date in that shape that is not a real day
(`2026-02-30`) is kept as written and drawn as a problem; it is not
dropped. Text in another shape (`📅 tomorrow`) is not a date.

Every glyph is read with or without U+FE0F after it, so an emoji picker's
output works. A glyph Wren writes new has no U+FE0F.

**Owners.** An `@` counts at the start of the description or after white
space, when a letter follows. A name runs over letters, digits, `.`, `-`
and `_`, and a final `.` is left out, so `@sam.` is `sam` and `a@b.com` is
not an owner. A trailing run of periods is left out, and a combining mark
stays with its letter. A task can have several owners. A task with no owner
is yours.

**Priority.** A line with several priority glyphs is read by the last.
Setting a priority replaces all of them with one.

**Text after the tokens ends them.** In `📅 2026-12-01 then lunch` there is
no due date, because text follows it.

**Everything else is kept.** Tokens Wren does not write (`⏳`, `🛫`, `➕`,
`🔁 every week`, `[key:: value]` fields, `#tags`) stay as written, and Wren
never reorders tokens, yours or its own. Writing a token has three cases:

- **Adding** one puts it in Obsidian Tasks' order: description, `🆔`,
  priority, other dates, due, cancelled, done, then the block id. It goes
  before any token that order ranks after it, so a new due date lands before
  a `✅` and before the block id, not simply at the end. An owner or a tag
  among the tokens is passed, and never moves.
- **Changing** one replaces only its date, or its glyph for a priority of
  another level, where it stands. A due date written with `🗓` or `[due:: …]`
  keeps its alias, its U+FE0F and its spacing. A priority you leave as it is
  keeps its U+FE0F.
- **Clearing** one removes every token of its kind, with the space before it.
  With several due dates, changing a date changes the last, the one the line
  is read by, and clearing removes them all.

### Date words

In the calendar's field and the form's due field, these words work. They
are English, and weeks start on Monday. [Suggestions](#suggestions) match
their own shorter list.

| Typed | Means |
|---|---|
| `today`, `tomorrow` | That day. |
| `mon` to `sun`, or the full name | The next one after today. A name equal to today's means a week on. |
| `next fri` | That day in the week after the current one, Monday to Sunday. On a Thursday, `fri` is tomorrow and `next fri` is eight days on. |
| `next week` | The Monday of next week. |
| `+3d`, `+2w`, `+1m` | That many days, weeks or months on. A month keeps the day, or the month's last day when it is shorter. About 27 years is the furthest. |
| `2026-12-01` | That date. |

Slash dates such as `3/4` are not accepted, because their order is
ambiguous. A date word is turned into an ISO date when it is written; the
file never holds the word. The calendar and the form read these words the
same way.

## In the editor

### Colours

With tasks on, the editor draws a task's tokens in the theme's colours:

- A due date in the date colour. A date before today, on a task that is not
  finished, is bold in the danger colour. A date that is not a real day is
  in the danger colour, not bold.
- Owners in the owner colour.
- A done or a cancelled task has its description struck and dimmed, and
  its tokens faint.

The colours follow the day: they change at midnight while Wren is open.
Both [editor engines](configuration.md) draw them. Spell check skips the
tokens: see [Spell check](spell-check.md#what-is-checked).

### The status bar

With the caret on a task, the line after the line count spells the task
out:

```
Doing · due Mon 23 Nov, in 4 days · ⏫ High · @alice.chen
```

The pieces come in this order, each only when the task has it:

- The status: Open, Doing, Done, Cancelled, or for an unknown box its
  counting and its character, `Open [a]`.
- The due date, `due Mon 23 Nov`, and on unfinished work how far off:
  `today`, `tomorrow`, `in 6 days`, `in 3 weeks` from two weeks out, and
  `2 days overdue` once passed (in the danger colour). A finished task has
  no relative part. A date that is no real day reads `due 2026-02-30 is not
  a real date`.
- The priority, `⏫ High`.
- The owners, or `yours` when there are none.
- `done Thu 19 Nov` and `cancelled Mon 16 Nov`.

The line gives way to the position and save status in a narrow window.
Pieces that do not fit drop whole from the end, and below a footer width of
about 440 px it is not shown. Both engines show it.

### Toggle a task

**Toggle a task** (⌘↵ on macOS, Mod+Enter on Linux, where Mod is Ctrl unless
[`linux-modifier`](configuration.md#settings) says Alt) is the done toggle.
An open, doing or unknown task becomes done and gets `✅` and today's date.
A done or cancelled task becomes open and loses its `✅` and `❌`, so
reopening a cancelled task clears its cancellation date.

Over a selection, the rule is the same for all of its tasks. If any task
is unfinished (open, doing, or a box with no status of its own), every task
that is not done becomes done, and a done task stays as it is: its `x` or
`X` and its date are kept. Otherwise every task becomes open, cancelled ones
included. Lines that are not tasks get an open box only when the selection
holds no task at all, as with tasks off. The whole selection is one undo
step. With Markdown assist off the toggle is unavailable, and Enter does not
continue a list.

Typing `x` in the box yourself is ordinary typing: it writes no date. Enter
at the end of a done task continues the list with a new open task and no
date. Moving a task between two statuses of the same type (`[x]` to `[X]`)
keeps its dates.

### A click on the box

On the `advanced` engine, a click on a task's box changes its status: open
to doing, doing to done, done to open. A cancelled task reopens. The
change is one undo step, and the caret stays where it was. A box with a
character Wren has no status for (`[?]`, `[>]`, `[1]`) is not cycled, so
its character is never lost: the click places the caret, and the
right-click status menu still sets a status.

The status changes 300 ms after the press, once a double click could no
longer follow, so a second click in that time selects a word instead and
cancels the change. The press must be let go on the same box, within a few
pixels. While the button is held, a key, another button, a modifier or the
window losing focus cancels it. After the release, other input settles the
change early, and a change to the text since the press cancels it. A click
with a modifier and a drag that starts on the box do nothing to the task.

### Menus

On the `advanced` engine, a right-click on a task opens a menu: on the box
the status menu, on a priority glyph the priority menu, on a due date the
calendar, and anywhere else the task menu. On a misspelled word the
corrections menu comes first. Each menu takes single keys: the characters
that go in the box for a status, `h`, `m`, `l`, ⇧H, ⇧L and `n` for a
priority. [Keybindings](../getting-started/keybindings.md#task-menus) lists
them. A menu opens on its current choice, marked; an unknown box has no
checked status row. The task menu's date row reads **Change due date…** when
the line has a due date and **Set due date…** when it has none.

The palette's **Tasks** group has the same actions, on both engines: **Cycle
task status**, **Set task priority** at each level, **Raise** and **Lower
task priority**, **Remove task priority**, **Clear task due date**, and
**Create or edit task…**. **Set task status…**, **Set task priority…** and
**Set task due date…** open the menus and the calendar, so they need the
advanced engine.

### The calendar

**Set task due date** (⌘D / Mod+D) opens a calendar under the date, or
the caret, on the `advanced` engine. It opens on the task's due date, or
today. Type a [date word](#date-words), or move with the arrow keys and PgUp and
PgDn, and press ↵ to write the date. A click on a day or on a quick pick
writes it at once and closes the calendar. **Clear** removes the due date;
Esc closes the calendar and writes nothing. The date is one undo step. See
[Keybindings](../getting-started/keybindings.md#task-menus) for the keys.

### Suggestions

On the `advanced` engine, typing on a task line offers what the line
lacks. Two rules open the list:

- **Dates and priorities.** Two or more ASCII letters at the start of a word
  that begin one of the words below, never in the description's first word.
- **Owners.** `@` alone, or `@` and the start of a name, at the start of the
  description or after white space. This works in the first word too.

| Type | Offers |
|---|---|
| `du`, `da`, `due`, `date` | **Due date…**, which opens the calendar |
| `to`, `tom`, `fri`, `next` | Today, Tomorrow, a weekday, Next week, written at once as a due date |
| `hi`, `med`, `lo`, `pri` | High, Medium, Low, Highest and Lowest priority |
| `@`, `@sa` | Owners already on task lines in the workspace |

Suggestions use these words only: `+3d` and ISO dates are typed in the
calendar or the form. Owners come from the task lines of every Markdown
note in the workspace, as the task index holds them. Owners are compared without regard to case (Unicode case
folding, so `@µ` and `@μ` are one owner) and offered in their most common
spelling. They are ranked by the notes they appear in, each note counting
less the longer ago it changed (half as much after 30 days), so this week's
co-workers come before a name from a year ago. A name you
have typed whole goes first. A suggestion is never offered for what the line
has: no second due date, no second priority, no owner it names already.

Tab picks the highlighted row. ↑ and ↓ move, and ↵ then picks the row you
moved to. Esc hides the list for the line. Typing carries on as usual and
the list follows.

Enter alone does not pick. `to`, `hi`, `fri` and `today` are ordinary
words, so Enter at the end of one continues the list and leaves the word
as typed. The one exception is **Due date…** for `du`, `da` and `dat`,
which writes nothing until you pick a date. A row Enter would not pick is
drawn lightly, with "Tab picks · ↑↓ ↵" under the list. Tab with no list
open indents as before.

### The task form

**Create or edit task** (⇧⌘↵ / Mod+Shift+Enter, or **Edit task…** in the
task menu) opens a form for the line at the caret, on either engine. It has
the description, status, due date, priority and owners, and shows the
Markdown it will write.

- On a task it edits the task.
- On a blank line or an empty list item it makes a new task.
- On a list item or a line of text that is not a task it makes that text a
  task, with the text as the description.
- A heading, a thematic break, a code block and front matter have no form.

Saving writes only the fields you changed, as one undo step, so a line in
your own order keeps its order, and an untouched form on a task writes
nothing. On a line that is not a task yet, saving makes the task even with
nothing changed. A due date that names no real day (`📅 2026-02-30`) is
kept as written when you change only other fields. Saving is refused, with
the form and what you typed in it left open, when the due field is not a
date or not a real day, and when the note changed in any way while the form
was open (it was reloaded from disk, say). The due field
takes the [date words](#date-words) and opens the calendar under it. See
[The task form](../getting-started/keybindings.md#the-task-form) for its
keys.

## The summary view

The summary view lists the tasks of a workspace, section, group or note in
the editor pane, and changes them where they are written. It is a view, not
a note: no file stands behind it, it never becomes the open document, and
the note under it keeps its draft, caret and undo history. Everything it
does is written into the notes as plain text, as the editor does.

### Opening it

- **A count in a column header.** Each column's header counts the open and
  doing tasks of what the column belongs to: Sections the workspace, Groups
  the section, Notes the group. The number is red while one of them is
  overdue. A click opens the view of that scope, and the count looks
  pressed while that view shows.
- **Show tasks** (⇧⌘T on macOS, Mod+Shift+T on Linux, `tasks.show`, or
  **Show tasks** in the palette). It opens the view of where the keyboard
  is: from the Sections column the selected section, from the Groups column
  the selected group, from the Notes column the group it lists, from the
  editor or a board the open note's group, and from the view itself the
  same view.
- **A row's actions.** Right-click a section, group or note in a column
  (or open its actions from the palette) for **Show tasks in this
  section**, **group** or **note**.

Opening the view adds no task write, no message and no saved history entry,
but it saves the open note's draft first, as leaving a note does; if that
save fails, the view does not open. It is refused too while the calendar or
the task form is open ("Close the task form or the calendar first").
Back and Forward pass through views as through notes: a view sits in the
history after the note it was opened from, and Back can land on an earlier
view or on the empty pane. A view is never written to
[`.wren/workspace.toml`](workspace-folder.md#wrenworkspacetoml), so the
saved history is what the note visits alone make, and Wren reopens on the
last note.

The counts follow `task-counts` in [`config.conf`](configuration.md#settings);
**Toggle task counts** (`view.toggle_task_counts`) in the palette flips it,
saves it, and shows a message with Undo. If `config.conf` cannot be written
the counts still change for this session, and the message says so. The
setting hides only the counts; the view and Show tasks stay.

### What it shows

The header holds the **scope chip**, for example "Tasks in Planning, group
▾". It opens **Show tasks in**, a menu of the workspace and the section,
group and note the view lies in (below a workspace view, the ones the tree
has selected), with the current one checked. Under it come the filters:

| Control | Choices |
|---|---|
| Status chips | Open, Doing, Done and Cancelled, each with its count under the other filters. None pressed shows every status; pressed chips together show any of their statuses, and **Show all** clears them. |
| Owner | Anyone; **Me**, the tasks with no owner or an owner named in `tasks-me`; or one `@name` from the scope's tasks. |
| Group by | Note (the default), Due date, Priority, or Owner. |
| Due | Any date, Overdue, Earlier, Today, This week, Next week, Later, or No date. |
| Priority | Any priority, each level, or None. |
| Hide done after 14 days | On by default. |

- **Me.** A task with no owner is yours, so Me lists those and the tasks
  that name a `tasks-me` owner; all the `tasks-me` names are one group,
  Me, when grouping by owner. Set `tasks-me = sam, sam.lee` in
  [`config.conf`](configuration.md#settings) for the names you answer to.
  Names are compared without regard to case.
- **Due buckets.** Weeks run Monday to Sunday. Overdue is unfinished work
  due before today and Earlier is finished work due before today. This
  week is after today up to Sunday, Next week the week after, and Later
  everything beyond. A task with no due date, or one that is not a real
  day, is in No date.
- **Grouping.** By note, groups follow the tree's order. By due date they
  are the buckets, by priority Highest to Lowest with No priority between
  Medium and Low, and by owner Me first and then each person by name. A task with several owners
  is listed under each.
- **Order.** Within a group, open and doing tasks come before done and
  cancelled ones, overdue first, then by priority (a task with none ranks
  between Medium and Low), then by due date, then in the tree's order.
- **Hide done after 14 days.** Done and cancelled tasks whose `✅` or `❌`
  date is more than 14 days before today leave the list. Finished work
  with no date counts as recent.

Each filter is remembered for its scope, in
[`.wren/tasks.toml`](workspace-folder.md#wrentaskstoml), and comes back
when you open that scope again. A scope with every filter at its default
leaves no entry. If `.wren/tasks.toml` cannot be written, the filter still
applies for this session and the message says "The change remains applied;
task settings were not saved".

A row has the status box, the description with its owners in the owner
colour, the priority glyph, the due date (red and bold when overdue, a
warning mark when it is not a real day), and under the description its
source: the nearest heading when the list is grouped by note (or "Top of
note"), and otherwise the group, the note and the heading. Done and
cancelled rows are struck through and dimmed, their owners in the dimmed
colour, and the date cell shows the done or cancelled date they carry. A box
whose character has no status (`[?]`, `[>]`) shows that symbol in the row.
In a narrow window a long description ends in "…". An empty list says what to
change to see tasks again; when Hide done after 14 days alone hides every
task, it says to turn that off.

The footer counts the open and doing tasks and the overdue ones of the
scope, and then says what is left out: **Left out: 1 note (2 tasks, tasks:
off), 3 notes in hidden folders (9 tasks)**, and unreadable notes. At the
right it shows the keys: "Space toggle · S D P · E edit · F2 rename · ↵
open · R refresh". When the window is narrow the hint drops whole pieces from
its end; the counts always stay.

### Keys in the view

The keys work while the view has the keyboard, which it takes when it
opens. Tab moves between the controls and the list, and ← → between
controls; ↵ or Space presses a control and ↓ opens a menu control's menu.

| Key | Does |
|---|---|
| ↑ ↓ or `k` `j` | Select the previous or next row. PgUp, PgDn, Home and End move further. |
| ↵ | Open the note at the row's line (found by the rule under [Writing to notes](#writing-to-notes)). If the task is not there, the note opens and a message says "This task changed since the list was built; the note is open". Back returns to the view, also when that note was already under it. |
| Space, ⌘↵ / Mod+Enter | Toggle the task done, or back to open, as in the editor. ⌘↵ is the key of **Toggle a task**, so it follows a rebinding. |
| `s` | Status menu. |
| `d` | The calendar, to set or clear the due date. |
| `p` | Priority menu. |
| `+` `=` and `-` `_` | Raise and lower the priority one level. |
| `e` | The [task form](#the-task-form) for the row. |
| F2 | Rename the description in the row. |
| `r` | Refresh: rows written move to where they now belong. |
| ⌘Z / Mod+Z | Undo the last task write. |
| Esc | Close what is open over the list. |

Escape does nothing else in the view: it does not clear the filters, which
are remembered, and it does not leave the view (Back does). In a filter's
menu it closes the menu, and from a control it gives the keyboard back to
the list. A key that opens a menu, the calendar or the form is the last
input of its batch, so keys typed in the same instant are dropped, not
applied to the row. So is an app shortcut that takes the keyboard (the
palette, a dialog); other app shortcuts run at their place in the batch. A
refused write ("This task changed; the list was refreshed") drops the rest of
its batch too. A held key does not repeat: Space, the toggle key, the undo
key, ↵, `s`, `p`, `d`, `e`, F2, `+` and `-` act once, while the arrows
repeat.

With the mouse: a click on a box cycles the status (open, doing, done, open;
a cancelled task reopens, and a box whose character has no status, such as
`[?]`, is only selected), a click on the priority opens the priority menu, a click on the
due date opens the calendar, and a click on the source line opens the note
at the task. A right-click on a row opens its **task menu**: the editor's
task menu, plus **Open the note at this line** and **Hide tasks in** the
note. The menu opens on "Open the note at this line", so a right-click
then ↵ opens it. A double-click on a row's description renames it, as F2
does.

The status, priority and task menus take the same single keys as in the
editor ([Task menus](../getting-started/keybindings.md#task-menus)), and
hang under the row's box, or its priority.

#### Renaming a description

F2 on the selected row (the view's own key, whatever the keymap gives F2
elsewhere), or a double-click on a row's description, opens a one-line
field holding the description as written: its owners are in it and its
tokens are not, and the caret is at the end. ↵ writes it ("Description
changed", with Undo). Esc, or leaving the field, cancels and writes nothing.
Tab does nothing. An empty description is refused: "A task needs a
description; nothing was written". While the field is open the view's
toggle and undo keys do nothing.

### Writing to notes

Every change from the view is written into the note, and the note's file
stays the one place the task lives.

- **An open note** is changed through its editor, as one undo step, so its
  caret and undo history stay, and on the advanced engine its scroll
  position too (the classic engine may scroll to show the caret). If the
  file changed on disk while the note has an unsaved draft, nothing is
  written: "The open note changed outside Wren; nothing was written, and
  its draft is kept".
- **A closed note** is read, changed on the task's line and written back in
  place, after the open note's draft is saved. The write makes no conflict
  copy of its own note; if saving the open draft needs one, that happens
  first and the write is blocked.
- **A row that is out of date** is refused. Wren looks for the task at the
  row's line, then for the one line anywhere in the note that reads the
  same. If it finds none, or cannot tell two apart, or the file changed
  while it wrote, nothing is written, the list is read again, and the
  message says "This task changed; the list was refreshed".

Every write shows a message under the list: what changed and where, for
example "Done · wrote line 12 of Work/Planning/Roadmap.md", with **Undo**.
It stays six seconds and the next message replaces it. A refusal is said in
the message, with no Undo. Undo, from the message or from ⌘Z / Mod+Z, puts
the line back through the same path and with the same check: if the task
has changed since, it is refused and forgotten. Every undo of a task write
says what it undid and where ("Undone: Done · wrote line 3 of
Work/Tasks/B.md"), and a refused one says "Nothing undone:" and why; undoing a
group exclusion or the counts toggle shows no message. Undo is Wren's own stack of
task-line and note front-matter writes, a hundred deep, and holds until you
leave the workspace. In the open note it is one more step of
the editor's history, not a step back in it.

**Rows stay where they are** until you press `r`, change a filter or the
scope, or leave the view. A task marked done under the Open chip stays in
the list, struck through, rather than vanishing under your cursor. A task
that moves to another line is followed when exactly one line matches; the
hold is let go when the task is gone or cannot be told from another.

### Hiding tasks

Some notes hold tasks that are not yours to track: templates, checklists,
a grocery list. Three layers leave them out of the header counts and the
list:

- **A note.** `tasks: off` in its front matter hides its tasks. Wren also
  honours Obsidian's `TP_ignore_this_file: true`. `tasks: false` and
  `tasks: no` hide too; any other value hides nothing, and the last
  occurrence of a key decides. The note's row carries a muted "tasks off"
  mark. **Hide tasks in this note** (`note.hide_tasks`, a note's actions in
  the Notes column) writes the line, creating the front matter if there is
  none, and the same action reads **Show tasks in this note again** on a
  hidden note. The task menu in the view only hides.
- **A group.** **Hide tasks from this group** (`group.hide_tasks`, a group's
  actions) adds it to `exclude` in `.wren/tasks.toml`, which a folder
  without front matter needs; the group is marked "hidden" and its action
  reads **Show this group's tasks again**. A group hidden only by its
  section has no action of its own. When both hide it, the action reads
  **Remove this group's own exclusion**, and the section still hides it.
- **A filter**, the last one: Hide done after 14 days, above. It changes the
  list and the chip counts only, not the header or footer totals, and has
  no message.

A hide or show of a note or group, and a counts toggle, each show a message
with Undo once they succeed. The footer's "Left out" gives totals by reason,
not the names. Hidden notes and groups keep their tasks in the workspace
index, so owner suggestions still read them.

Wren changes a note's front matter only when that is unambiguous, because
front matter is another program's too, and otherwise writes nothing and says
to edit the note.

- **Hide** is refused when the block has a top-level line that is not a
  plain key (a quoted key, a flow mapping, a merge: any might be `tasks`),
  when the block's keys are indented, when `tasks` is a list or another
  non-switch, and when the note begins with `---` and closes it further down
  without being front matter as Wren reads it.
- **Show** removes every `tasks` line that hides, and the block if only blank
  lines remain. It is refused when `TP_ignore_this_file` is what hides the
  note ("`TP_ignore_this_file` hides this note's tasks; remove it in the
  note to show them"), when what is left would stop reading as front matter
  or make front matter of the note's own lines, or would join a byte-order
  mark to the text.
- **Hide and Show** are refused when the `tasks` line they would change goes
  on in indented lines under it, which YAML reads as more of its value.
- **Hide, Show and Undo** are all refused when a line break on one side of
  the change would run into one on the other ("The note's line endings would
  run together there; edit the note").
- **Undo** puts the front matter back as it was, with its line endings. If
  the note changed so that it would not go back as it was, it writes nothing:
  "This note's front matter changed; nothing was written".

Writing `.wren/tasks.toml` for a hidden group is all or nothing while it is
unpublished: if the file cannot be written, nothing changes and the action is
refused. With `tasks = off` Wren neither reads nor writes the file.

## Which engine

| | `classic` | `advanced` |
|---|---|---|
| Colours, dimming and strikethrough | yes | yes |
| Status bar | yes | yes |
| Toggle with dates | yes | yes |
| Palette commands that rewrite the line (cycle, priority levels, raise, lower, clear due) | yes | yes |
| Task form | yes | yes |
| Right-click menus, box click, the calendar on a task line, suggestions, **Set task status…**, **Set task priority…**, **Set task due date…** | no | yes |
| The calendar inside the task form | yes | yes |

`advanced` also wraps a long task under its description. See
[Configuration](configuration.md) to switch.

## Compared with Obsidian Tasks

A line Wren writes reads the same in Obsidian Tasks, and the reverse. Where
Wren reads differently:

- **Last token wins.** With two due dates or two priorities on a line, Wren
  reads the last. Obsidian Tasks reads the first.
- **Owners among tokens.** An `@owner` or a `[key:: value]` field standing
  among the trailing tokens does not end them. Obsidian Tasks treats `@sam`
  as plain text.
- **Invalid dates are kept.** `📅 2026-02-30` stays on the line and is
  flagged. Obsidian Tasks ignores it.
- **A link reference definition is not a task.** `- [a]: https://example.com`
  stays text, whatever the status table says.
- **Adjacent tokens.** Obsidian Tasks reads `#x⏫` as a tag; Wren reads the
  tag `#x` and a priority. Both read only the run of tokens at the end of the
  line, so a token in the middle of prose is not read.
- **Wren's own words.** The weekday and `+3d` words only help you type; the
  file holds ISO dates.
- **Every task counts.** There is no global filter and no editable status
  table. The four statuses are fixed, and any other character is open.
- **Removing an owner** takes the name and the space before it, except
  where that would join the text before it to what follows. `ask @sam's team`
  becomes `ask 's team`, keeping the separator.

Wren does not write `⏳`, `🛫`, `➕` or `🔁`. It keeps them.

## Limits

- **Emoji colour.** On Linux, `✅ ❌ ⏫ ⏬` can draw in one colour where the
  system has the FreeSans font ahead of a colour emoji font, such as Noto
  Color Emoji. The text is the same, and `📅 🔺 🔼 🔽` are in colour.
  Installing a colour emoji font helps; macOS draws all of them in colour.
- **English only.** Date words are English.
- **Restart to switch.** `tasks` applies when Wren restarts.
- **Boards** have no tasks.
