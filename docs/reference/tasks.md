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

## Turning tasks on

Tasks are off by default. Set `tasks = on` in [`config.conf`](configuration.md)
and restart Wren. Until then the banner says "Restart Wren to apply
`tasks`."

With `tasks = off`, nothing changes:

- A box holds a space, `x` or `X`. `[/]` and `[-]` are text.
- The Markdown toggle writes no done date.
- Highlighting, [spell check](spell-check.md) and search read a task line
  as they always did.
- No task command is listed, its keys do nothing, and a `keybind` line
  that names one is skipped with a warning.

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
calendar or the form. Owners come from the task lines of the notes whose
text the search index holds. In a workspace past the index's memory limit
(128 MiB, counting each note's text and its lowercase copy), a note whose
content search is unavailable is left out, so a name written only there is
not offered. Owners are compared without regard to case (Unicode case
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
