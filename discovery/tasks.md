# Tasks: research and proposed design

Status: **discovery.** Written 2026-11-15 against `main` at `0b23f89`; references re-checked against `ec1440b`. Nothing is built. The first release is the editor alone (ring 0, §12); the rollup and kanban follow once that has been refined in use. §2 records what the owner has decided; §3 lists every question and how it was answered. Line references are to `ec1440b`; find the named function if they have moved.

## 1. What we are building and why

Wren notes already hold Markdown task lines (`- [ ] …`), but each one is visible only inside its own note. The goal is to make Wren a better place to organise work and personal life:

- a light syntax on a task line for **status**, a **due date** and an **owner**, readable as plain text and compatible with Obsidian where that costs nothing;
- editor help to write it: a **calendar popover** for the date, and completion for owners;
- a **rollup**: every task in a note set, group, section or workspace in one view, where changing a task writes the change into its note;
- later, a **kanban** view over the same tasks.

Wren is not meant to replace Jira. The intended use is personal organisation: action items from 1:1s and meetings, non-engineering tasks, and coordination with co-workers. Some of those tasks will later be moved into Jira by hand.

## 2. The owner's decisions (2026-11-15)

1. **Compatibility.** Obsidian compatibility is valued: use the syntax that works there unless a clearly better solution exists for Wren's users. A harder implementation is acceptable if it gives a well-made experience.
2. **Dates.** A calendar popover puts a date into the Markdown of a task.
3. **Owner** is free-form. A task with no owner is implied to be the user's own. At work it tracks co-workers' action items from meetings.
4. **Statuses for the MVP:** open, doing, done and cancelled. Build assuming more statuses are added later (on hold, waiting and so on).
5. **Dates in the MVP:** a due date only. Recurrence is out of scope; it would be a new primitive, designed separately.
6. **Rollup scope:** a given set of notes, a group, a section or the workspace, with ways to ignore tasks. Opt-in and opt-out inside documents need to be thought through (§7).
7. **Views.** First a quick view of the tasks for a level of the hierarchy, then a way to see them all and update them, the update changing the document. Views have no files behind them; a metadata file for an index or a view is acceptable, but **the edits themselves go into the note**. The view is also where older work is ignored.
8. **Kanban** comes after the rollup and is still being thought through. Columns start from a query, moving a card between columns changes its status, and card order within a column is metadata kept in a view file.
9. **Owner is `@name`** (Q3, §5.4).
10. **Ignoring works in layers:** `tasks: off` in a note's front matter, and folder exclusions in `.wren/tasks.toml` (Q4, §7).
11. **An unknown character in the box is an open task** for the MVP (Q5, §5.1).
12. **`📅` for the due date, and `✅`/`❌` written on completion and cancellation** (Q1, Q2; §5.2, §5.3). What the user types and how the editor integration works get refined when this is implemented, and must be verified on both Linux and macOS (§13).
13. **Follow Obsidian Tasks' ways of entering metadata,** since they are common: typing the tokens by hand, an auto-suggest menu on task lines, and a "Create or edit task" form (§6).
14. **Priority is in the design,** with Obsidian's indicators (§5.7). The ring it ships in is Q6.
15. **The due date and priority can be set or changed from the rollup and from kanban**, not only in the editor (§9, §11).
16. **Priority ships in the first release** (Q6). **The first release is the Markdown editing and display only** (ring 0, §12): no rollup and no kanban. The task experience is refined in the editor first, and the views are then built on it.
17. **The recommended defaults for Q7 to Q14 are accepted** (§3.1, §3.2). Q9, Q12, Q13 and Q14 are starting points that using ring 0 may revise.

## 3. Questions, and how each was answered

| # | Question | Answer |
|---|---|---|
| Q1 | Is `📅 YYYY-MM-DD` (Obsidian Tasks) the stored form of a due date? | **Resolved (§2 item 12).** |
| Q2 | Does Wren add a done date `✅ YYYY-MM-DD` when a task is completed, and a cancelled date `❌ YYYY-MM-DD` when it is cancelled? | **Resolved (§2 item 12).** |
| Q3 | Is `@name` the owner? | **Resolved (§2 item 9).** |
| Q4 | The opt-out layers: `tasks: off` in a note's front matter, and folder exclusions in `.wren/tasks.toml`? | **Resolved (§2 item 10).** |
| Q5 | An unknown character in the box (`[?]`, `[>]`): an open task, as in Obsidian, or not a task? | **Resolved (§2 item 11).** |
| Q6 | Does priority ship in the first release? | **Resolved (§2 item 16).** |
| Q7 | How many priority levels do Wren's pickers offer? (§3.1) | **Resolved (§2 item 17):** all five, High, Medium and Low first |
| Q8 | Where does `@` completion find owners in ring 0? (§3.1) | **Resolved (§2 item 17):** scan the notes the search index already holds, on demand; no task index until ring 1 |
| Q9 | Are tokens shown as text or collapsed into chips? (§3.2) | **Resolved (§2 item 17):** text with colour first; decide on chips after use |
| Q10 | Where on the line does Wren write tokens, and in what order? (§3.2) | **Resolved (§2 item 17):** the end of the line, in Obsidian Tasks' own order |
| Q11 | Does checking a box by hand add a done date? (§3.2) | **Resolved (§2 item 17):** no |
| Q12 | When does auto-suggest appear? (§3.2) | **Resolved (§2 item 17):** task lines only, after two letters at the start of a word |
| Q13 | Which new commands get default keys? (§3.2) | **Resolved (§2 item 17):** set due date and the task form; the rest in the palette first |
| Q14 | Calendar and date words: week start, date order, language? (§3.2) | **Resolved (§2 item 17):** ISO: weeks start Monday, English date words |

### 3.1 Q7 and Q8 in detail

**Q7: how many priority levels the pickers offer.** A *picker* is any list Wren shows to choose a priority from. In ring 0 there are four:
- the auto-suggest entries that appear when typing `hi`, `med` or `lo` on a task line;
- a **Set priority** list opened by a command;
- the priority field of the "Create or edit task" form;
- the palette's commands, one per level (`task.priority.high` and so on).

Rings 1 and 2 add the rollup's and kanban's priority menus.

Q7 is about those lists only. The text is never limited: Wren reads and shows all five glyphs however they got into the file (typed, pasted, or written by Obsidian), sorts by all five, and raise and lower step through all five.
- *All five* (the answer): matches Obsidian, so a user moving between the two apps sees the same choices. The cost is a longer list.
- *Three* (High, Medium, Low): fewer choices, as Logseq, org-mode and NotePlan offer. Highest and Lowest stay readable and still sort, but Wren gives no way to *set* them except typing the glyph or raising and lowering.

The list is ordered High, Medium, Low, Highest, Lowest, so the common three are always the first keys pressed, while Obsidian's two extremes stay one step away.

**Q8: where `@` completion finds its names.** When the user types `@` on a task line, Wren offers names to complete. The question is which names it knows, given that ring 0 has no rollup.

| Option | Where the names come from | Cost | Gaps |
|---|---|---|---|
| A. The open note | Scan the open note's text for `@name` tokens on task lines | Trivial | In a new meeting note, nobody is offered until they are typed once. The case that matters most (the same co-workers across many meetings) gets the least help. |
| B. Scan on demand | Scan the text the search index already holds for every note (`Index::content`, `crates/wren-core/src/index.rs:188`) when the menu first opens. Cache the names until the index's `revision()` changes (`index.rs:172`). | Small: one pass over text already in memory. Search's own pass over 5,000 notes and 25 MB takes a few milliseconds (`docs/development/architecture.md:307-315`), and a regex-free byte scan for `@` is no slower. No new state, no new update path. | A note whose content the search index dropped (over its 128 MiB budget, `index.rs:17`) contributes no names. In practice that means very large workspaces only. |
| C. The task index | Build §8's index in ring 0, with no view on it | Medium: an index that has to stay correct through every Session write, reconcile and external change. That is ring 1's work, done early. | None for completion. But its other fields (status, dates, priority) would have no user in ring 0, so their correctness goes untested by real use. |
| D. A remembered list | Keep every name the user has accepted from completion, in `.wren/tasks.toml` | Small | Misses names typed in another editor or before the feature existed, and keeps names after they leave every note. |

The answer is **B** (an earlier draft of this note chose C). It gives completion across all the meeting notes, which is the reason for wanting it, without building ring 1's index before the editor experience has settled. In B, a name scores higher the more notes it appears in and the more recently those notes changed, so regular co-workers come first. Ring 1 then replaces B's scan with the index when the rollup needs one, and completion keeps the same behaviour.

Two smaller choices sit under Q8, settled the same way:
- **Scope.** Is a name offered only if it appears on a task line, or anywhere in a note (`@sam` mentioned in prose)? Task lines only, so stray `@` in pasted email text does not pollute the list.
- **Case.** Are `@Sam` and `@sam` one owner? Yes. Names compare case-insensitively, and completion inserts the most common spelling.

### 3.2 Questions settled for ring 0

Each answer below is accepted (§2 item 17). Using ring 0 is what may revise Q9, Q12, Q13 and Q14.

- **Q9, chips or text.** Today the editor draws every character of the source; hiding `📅 2026-12-01` behind a chip reading "Dec 1" (or "Tomorrow", or "2 days overdue") means teaching the editor fork to replace a range with a drawn element, and keeping caret movement, selection and Backspace sensible around it. That is real work in both engines. *Answer:* ship coloured text (date red when overdue, owners in an accent, a done task's line dimmed), then decide whether the raw date is noisy enough in daily use to justify chips. A middle step is possible: keep the text but show a hover or status-line hint ("due in 2 days").
- **Q10, where tokens go.** When Wren writes a token, it goes at the end of the line, before an Obsidian `^block-id` if there is one. Wren reads tokens backwards from the end, as Obsidian does, and Obsidian is sensitive to some orders (its issue 1505). *Answer:* when Wren writes, it uses the order Obsidian Tasks' own serializer writes (read from `DefaultTaskSerializer.ts` when implementing and pinned in tests). It never moves `@owner`, which stays in the description where the user typed it. A line someone ordered differently by hand is left alone, except for the one token being changed.
- **Q11, a box checked by hand.** Typing `x` between the brackets is plain editing. Wren could notice it and add `✅ today`, but that rewrites the user's typing, and fixing a mis-typed box would gain and lose dates. *Answer:* only the toggle (⌘↵ on macOS, Ctrl+Enter on Linux, already `markdown.toggle_task`), the status menu and the views write dates.
- **Q12, when auto-suggest appears.** Too eager and it interrupts note-taking; too shy and nobody discovers it. *Answer:* only on task lines, only after two letters at the start of a word that match an entry (`du`, `to`, `hi`), never inside the description's first word, and Escape hides it for the rest of that line. A `tasks.suggest` setting can turn it off.
- **Q13, default keys.** The new commands are: set due date, the task form, set priority, raise, lower, cycle status, and set status. Each default key has to be free on both platforms: the Command layer on macOS, Ctrl for editing on Linux, and IBus's Ctrl+. emoji key (`crates/wren-core/src/keybind.rs:274`). *Answer:* bind set due date and the task form; leave the rest palette-only, and every command can still be bound with a `keybind` line until use shows which deserve a default.
- **Q14, the calendar's conventions.** Which day starts the week, how a typed `3/4` is read, and in which language `fri` or `tomorrow` are understood. *Answer:* ISO conventions: weeks start on Monday, typed dates are `YYYY-MM-DD` or words (`today`, `tomorrow`, weekday names, `+3d`, `+2w`), and slash dates are not accepted because they are ambiguous. English words only. A setting for the week's first day can follow if wanted.

## 4. Research: how other tools write tasks

Sources are in §13. Summary:

| Tool | Status | Due date | Priority | Owner |
|---|---|---|---|---|
| GitHub Markdown | `[ ]` `[x]` | none | none | none |
| Obsidian Tasks | Character in the box, mapped to a status *type*: `[ ]` todo, `[/]` in progress, `[x]` done, `[-]` cancelled, plus custom statuses | `📅 YYYY-MM-DD`; also `⏳` scheduled, `🛫` start, `➕` created, `✅` done, `❌` cancelled | `🔺⏫🔼🔽⏬` | none; an owner field was declined (issue 2396); tags suggested |
| Dataview inline fields | as above | `[due:: 2026-01-15]` | `[priority:: high]` | any key, such as `[assigned:: John]` |
| SilverBullet | `[ ]` `[x]` plus custom states | `[deadline: "2026-03-15"]` (its emoji form was removed in 2.4) | `[priority: high]` | `[assignee: Alice]` |
| Logseq, org-mode | Keywords: `TODO`, `DOING`, `DONE`, `LATER`, `NOW`, `WAIT`, `CANCELLED` | `DEADLINE: <2026-01-10 Sat>` on the next line | `[#A]` to `[#C]` | a `key:: value` property |
| todo.txt | `x ` prefix | `due:2026-07-01` | `(A)` first on the line | none (`@` is context) |
| TaskPaper | `@done(date)` | `@due(2016-03-16)` | `@priority(1)` | none |
| NotePlan | `[ ]` `[x]` `[-]` `[>]` | `>2026-04-24` | `!` `!!` `!!!` | `@mention` |
| Todoist (entry, not file format) | none | natural language | `p1` to `p4` | `+Name` |
| TaskNotes (Obsidian) | `status:` in front matter, one file per task | `due:` | `priority:` | any front matter key |
| Meeting-notes templates | `[ ]` `[x]` | `(due 2026-10-15)` in prose | none | `@owner` or `**Name**:` first on the line |

What carries over:
- **Status lives in the box.** `[ ]`, `[/]`, `[x]` and `[-]` are the closest thing to a shared convention, and they stay valid GitHub Markdown (GitHub shows an unknown box as text). Obsidian Tasks maps each character to a fixed set of status types (todo, in progress, on hold, done, cancelled, non-task); filters and toggling work on types, which is what lets new statuses be added later without new code paths.
- **Dates are stored as ISO and entered loosely.** TaskPaper's date palette, Obsidian Tasks' modal and auto-suggest, and NotePlan all accept "fri" or "next week" and write `YYYY-MM-DD`. No tool stores a relative date, and ISO dates sort as text.
- **Owner has no standard.** The meeting-notes templates converge on one rule: one owner per action item, written the same way every time, so a search finds a person's queue.
- **Emoji are hard to type, especially on Linux.** A SilverBullet user moved off `📅` for this reason. Obsidian Tasks has open bugs about variation selectors (`⏫` typed by hand is not recognised) and about non-breaking spaces. Neither matters if the editor writes the token for the user.
- **Ignoring tasks.** Obsidian Tasks started with a global filter (only lines tagged `#task` are tasks). It added a global query (`path does not include …`, `folder does not include …`) for excluding folders, and is adding a per-file front matter switch (`TP_ignore_this_file: true`, issue 3822). Users with old vaults full of stale checkboxes reach for these first.
- **Kanban comes in two models.** *The file is the board* (Obsidian Kanban: `## Lane` headings, `- [ ]` cards, `## Doing (3)` sets a limit of three cards). *The board is a query* (TaskNotes on Obsidian Bases: columns are the values of a property, and dragging rewrites the property in the source note). A rollup board is the second model. Its hard part is card order, which plain Markdown has nowhere to keep; TaskNotes keeps it in a view file.

## 5. Syntax (proposed)

A task is a list item with a box, as `wren_markdown::block` reads it today (`task_box`, `block.rs:1000`): `- [ ] text`, `* [ ]`, `+ [ ]`, `1. [ ]`, inside quotes too. Tasks in code blocks, front matter and HTML are not tasks; the classifier already says which lines those are.

### 5.1 Status

| Box | Status | Type |
|---|---|---|
| `[ ]` | Open | open |
| `[/]` | Doing | doing |
| `[x]`, `[X]` | Done | done |
| `[-]` | Cancelled | cancelled |
| any other single character | Unknown (shown with its character) | open (Q5) |

- Statuses are a table of symbol, name, type and next symbol, as in Obsidian Tasks. Types are what filters, toggling and the rollup work on, so on hold or waiting (`[h]`, `[?]` and so on) can later be added as rows. A user-edited table is ring 3 (§12).
- `block::Task` holds `done: bool` today (`block.rs:237-243`). It becomes the box's character plus the status type, and `task_box` accepts any single character that is not `]`. The highlighter, Backspace on a task (`edit/delete.rs`), `markdown.toggle_task` (`edit/task.rs`) and the outline tests all read `Task` and move with it. `docs/reference/spell-check.md:29` names "tasks (open and done)" and changes too.
- **Toggling** (`markdown.toggle_task`, extended): open → done, doing → done, done → open, cancelled → open. These are the Obsidian Tasks defaults. A new `task.set_status` command opens a menu of every status, and `task.cycle_status` steps open → doing → done.

### 5.2 Due date

`📅 2026-12-01`: the emoji, one space, then an ISO date. Wren also *reads* Obsidian's other due-date emoji (`📆`, `🗓`) and Dataview's `[due:: 2026-12-01]`, but writes only `📅` (Q1).

- Like Obsidian Tasks, Wren reads metadata backwards from the end of the line. Tokens follow the description, separated by spaces, and an Obsidian `^block-id` stays last. Text after the tokens ends them; Obsidian does the same.
- A date that is not a real calendar date (`2026-02-30`) is kept as written and shown as a problem in the rollup, not silently dropped. Obsidian silently ignores such dates.
- "Today" is the local calendar date.

### 5.3 Done and cancelled dates

When the status type changes to done, Wren appends `✅ YYYY-MM-DD`; moving away from done removes it. Cancelled works the same with `❌ YYYY-MM-DD`. This matches Obsidian Tasks' defaults, and it is what lets the rollup hide work finished more than N days ago (§9). Q2 settled this (§2 item 12); a setting could turn it off.

### 5.4 Owner

`@name`, anywhere in the description. A task with no `@` is the user's own.

- An `@` counts only at the start of the description or after whitespace, and only when followed by a letter. A name runs over letters, digits, `.`, `-` and `_` (`@sam`, `@jane.doe`, `@j-smith`), so `a@b.com` is not an owner. A trailing `.` is left out, so `@sam.` reads as `sam`.
- A task can have more than one owner (`@sam @priya`); the rollup lists it under each.
- Obsidian shows `@sam` as plain text, so this costs nothing there; it is NotePlan's mention and the meeting-notes convention.
- A Jira key such as `ABC-123` stays plain text. A "copy for Jira" command is a ring 2 idea (§12).

### 5.5 Read, never written

Wren keeps every token it does not understand (`⏳`, `🛫`, `🔁 every week`, `#tags`, `[key:: value]`, and `🆔` until kanban writes it, §11) exactly as written, and never reorders tokens it does not write. The editor's chips and highlighting (§6) cover only the tokens in §5.1 to §5.4 and §5.7.

### 5.6 How the date tokens are typed, and where they show

The user never types an emoji. Wren writes the tokens; the user picks a date or checks a box.

**Due date (`📅`, Q1)**

1. On a task line, the user runs **Set due date** (a shortcut, the palette, or the status menu). Alternatively, typing `due` at the end of a task line offers a one-item suggestion "📅 Due date", which Enter accepts. This is how Obsidian Tasks' auto-suggest works.
2. A calendar opens at the caret. Arrows move the day, typing accepts `fri`, `tomorrow`, `+3d` or `2026-12-01`, and Enter picks.
3. Wren appends ` 📅 2026-12-01` to the line, or replaces the date that is there:

   ```markdown
   - [ ] Send the deck to finance @sam 📅 2026-12-01
   ```

**Done date (`✅`, Q2)**

There is nothing to type. When a task is completed through Wren, the date is appended; when it is reopened, the date is removed. Cancelling works the same with `❌`:

```markdown
- [x] Send the deck to finance @sam 📅 2026-12-01 ✅ 2026-11-28
```

"Through Wren" means the toggle shortcut, a click on the box in the rollup, or the status menu. Typing `x` between the brackets by hand is ordinary typing, and Wren does not rewrite it. A task done that way simply has no done date, and the "hide older work" filter treats it as recently done.

**Where they show**

| Place | Due date | Done date |
|---|---|---|
| The note in Wren | The text itself, coloured (red when overdue); possibly a chip such as "Dec 1" or "Tomorrow" (decided while refining ring 0) | The text itself, dimmed |
| The rollup | A Due column, and the due buckets (Overdue, Today, This week, …) | "Done Nov 28" on the row; used by "hide done older than N days" |
| Obsidian with the Tasks plugin | Read natively: queries, sorting, the calendar | Read natively |
| GitHub or any other viewer | The plain text `📅 2026-12-01`, which still reads correctly | The plain text `✅ 2026-11-28` |

**The trade-offs behind Q1 and Q2** (both resolved, §2 item 12; what is typed and how the editor works get refined at implementation)

- **Q1.** The alternative was a text form such as `due:2026-12-01`. It is easier to type by hand and needs no emoji font, but Obsidian does not read it. Since the popover writes the token, the emoji's typing cost mostly disappears; what remains is §13 risk 1 (how the glyph renders).
- **Q2.** Wren writes text the user did not type, which some people dislike. Without it, "hide older work" would need another source of "when was this done": the note's modified time (imprecise, because one edit dates every task in the note), or a record in `.wren/tasks.toml` (outside the note, and lost when the task is checked off in another editor).

### 5.7 Priority

Obsidian Tasks' indicators, appended to the line like the due date:

| Token | Level | Sort rank |
|---|---|---|
| `🔺` | Highest | 1 |
| `⏫` | High | 2 |
| `🔼` | Medium | 3 |
| (none) | None | 4 |
| `🔽` | Low | 5 |
| `⏬` | Lowest | 6 |

```markdown
- [ ] Send the deck to finance @sam 📅 2026-12-01 ⏫
```

- **No priority sorts between Medium and Low**, as in Obsidian Tasks: Low and Lowest mean "deliberately later", so they sort below unmarked tasks, and nobody has to mark every task to use priority at all.
- **Read with or without the variation selector.** An emoji may be followed by U+FE0F, which macOS's emoji picker (Ctrl+Cmd+Space) inserts after `⏫`. Obsidian Tasks failed to read such a `⏫` until 6.1.1 (issue 2273). Wren reads each indicator with or without U+FE0F. It writes the exact code points Obsidian Tasks writes; check those in its serializer (`DefaultTaskSerializer.ts`) when implementing, and pin them in tests.
- **One priority per task.** If a line has several indicators, the last one wins on read. Setting a priority replaces every indicator on the line.
- **Why five levels.** Other tools use fewer: three in Logseq, org-mode and NotePlan (`!`, `!!`, `!!!`), four in Todoist, and todo.txt guides advise using only A to C. Wren still reads all five for compatibility. Its menus put High, Medium and Low first and Highest and Lowest after them (Q7), so the everyday choice is three.
- **Shown as:**
  - in the editor, the glyph itself (no colour of its own, so the line stays calm);
  - in the rollup, a narrow priority column, the default sort (overdue, then priority, then due date), and a "by priority" grouping;
  - on kanban, a badge on the card, and optionally swimlanes by priority (§11).

  The glyphs differ by shape (triangle, double and single arrows), so meaning never rests on colour alone. This follows the colour-blindness rule the boards work kept.
- **Entered** like the due date (§6): typed by hand, chosen from the auto-suggest (typing `high`, `low` and so on on a task line), set in the "Create or edit task" form, or set from a view. In the editor and the rollup, a key raises or lowers the priority one level at a time, as org-mode's Shift+Up and Shift+Down do. Which key is decided at implementation, against the macOS and Linux keymaps.

## 6. Editor assistance

Wren offers Obsidian Tasks' three ways in (§2 item 13). All three write the same text, so a line written one way can be edited any other way, in Wren or in Obsidian.

1. **Typing the tokens by hand.** This always works: Wren reads `📅 2026-12-01`, `⏫` and `@sam` however they got there, including emoji pasted from elsewhere or inserted by the system emoji picker (macOS Ctrl+Cmd+Space; on Linux, IBus's Ctrl+. or the GNOME picker). Nothing is rewritten while the user types.
2. **Auto-suggest.** On a task line (and only there), typing a few letters opens a short menu of matching actions:
   - `du` → "📅 Due date…", which opens the calendar;
   - `tod`, `tom`, `fri` → "📅 Today (Sun 15 Nov)" and the like, written at once;
   - `hi`, `lo`, `med` → "⏫ High priority" and the like;
   - `@` → owners already used in the workspace.

   The menu offers only what the line lacks (no second due date). Enter or Tab accepts, and Escape or continuing to type dismisses it. A minimum match length keeps it out of the way while writing prose, as Obsidian's setting does. It uses the same anchored-menu machinery as the calendar (below).
3. **"Create or edit task" form.** A command on a task line, or on a blank line to create a new task, opens a small form: description, status, due date (with the calendar), priority, and owners. OK rewrites the line. **The same form is what the rollup and kanban open to edit a task** (§9, §11), so all three surfaces share one way to change a task.

Pieces the three share:

- **Calendar popover.** On a task line, `task.set_due` opens a calendar at the caret. Typing in it accepts `today`, `tomorrow`, weekday names, `+3d`, `+2w`, `next mon` or an ISO date. Enter writes or replaces the line's `📅` token; a "Clear" entry removes it. Precedent: the spelling suggestions menu is already a popover anchored in the editor (`SpellMenu`, `crates/wren-editor/src/proof.rs:119`). The date parser belongs in a toolkit-neutral crate (`wren-markdown`, or a small new one) so it can be unit tested on its own.
- **Owner completion.** Typing `@` on a task line offers the owners already used in the workspace: in ring 0 from a scan of the notes the search index holds, and from ring 1 from the task index (Q8, §3.1).
- **Highlighting.** The highlighter (`crates/wren-editor/src/highlighter.rs`) colours the date token, red when overdue, and the owner token. Collapsing `📅 2026-12-01` into a chip reading "Dec 1" would be new work in the editor fork, which cannot hide text today. Whether to do it is decided while refining ring 0.
- **Spell check** skips `@names` and the date tokens, as it already skips `#tags` (`docs/reference/spell-check.md:44`).
- **Commands** follow the project's rule: a `CommandId`, a `definitions!` row, handling in `run_command`, Editor scope where only the focused editor runs the key, and a keybinding default on both platforms.
- **Linux and macOS** (§2 item 12). Every entry path is checked on both:
  - shortcuts use Command on macOS and Alt on Linux (`keys::alt_filter` keeps an Alt press from typing its letter);
  - the system emoji pickers' output, VS16 included, must parse;
  - the auto-suggest must not fight an input method's own popup (IBus, macOS dead keys and composition);
  - the popovers must render on wgpu and tiny-skia, X11 and Wayland, and Metal.

  §13 has the test plan.

## 7. Which tasks a rollup includes, and how to ignore them

Every task counts by default. A per-task opt-in such as Obsidian's `#task` filter would be too noisy for meeting notes. Ignoring works in layers, coarsest first:

1. **Scope.** A rollup covers what the user picks: the workspace, a section, a group, or a set of notes.
2. **Folder exclusions** (Q4). A section or group can be excluded from every rollup ("Hide tasks from this group"), stored in `.wren/tasks.toml` (§10.3). Folders have no front matter, so this is the only place it can live.
3. **A note switch** (Q4). `tasks: off` in a note's front matter excludes the note. This suits grocery lists, templates and checklists that are not work. Wren also honours Obsidian's `TP_ignore_this_file: true`. Reading front matter keys is new: `block.rs` classifies front matter lines but does not parse YAML. Two `key: value` lines need no YAML library; a full parser would.
4. **View filters**, for older work. Hide done and cancelled tasks whose `✅`/`❌` date is older than N days (default 14). A filter for undated open tasks in notes untouched for M days is worth considering. Filters are per view and remembered in `.wren/tasks.toml`.

Not proposed: dismissing a single task without changing its status. It needs task identity (§11), and cancelling the task covers the need.

## 8. The task index

- **Where it comes from.** Tasks are parsed at the same points where the search index takes a note's text: when the workspace opens, on each Session write, and on reconciliation (`docs/development/architecture.md:307-315`). The task index keeps small per-task records (path, line, status, due date, priority, owners, done or cancelled date, description, nearest heading), not note content. The search index drops content past its budget (`MAX_CONTENT_BYTES`, 128 MiB, `crates/wren-core/src/index.rs:17`), so the task index must not rely on that content staying in memory.
- **Place.** `wren-core`, beside `index.rs`, toolkit-neutral and tested inline. Parsing a line is a pure function in `wren-markdown`, next to the classifier.
- **Cost.** Search queries 5,000 notes and 25 MB in a few milliseconds. Task parsing is one pass per changed note; `make perf-search` is the model for a `perf` test of the task index.
- **Boards.** `.wren-board` notes have no tasks.

## 9. The rollup view (ring 1)

- **Opening it.** A command `tasks.show` opens the view for the selected tree item (workspace, section, group or note); a multi-select of notes gives a note set. It is a view, not a note: no file behind it, and it never becomes the open document.
- **Rows.** The status box, the description with its tokens shown as in the editor, the priority, the due date, the owners, and the note (with the nearest heading) it comes from.
- **Grouping:** by note (default), by due bucket (Overdue, Today, This week, Later, No date), by priority, or by owner (Me = no owner). **Sorting** within a group: overdue first, then priority, then due date. **Filters:** status types (default: open and doing), owner, due bucket, priority, and §7's hide-older-work.
- **Changes from the view** (§2 item 15), each writing the note through §10:
  - set the status: click the box to toggle, or the menu for any status;
  - set or clear the due date, with the same calendar as §6;
  - set the priority, raise or lower it one level, or clear it;
  - edit everything at once in the "Create or edit task" form (§6);
  - edit the description inline (later in ring 1).

  Enter, or a click on the note name, opens the note at that line.
- **Keyboard first,** like the tree: arrows move, Space toggles, and single keys open the status menu, the calendar and the priority menu, raise or lower the priority, and open the form.

## 10. Writing from a view

### 10.1 The write path

Every change made in a view is a Session op, per the architecture rule. Today `Op::WriteNote` refuses any note that is not the active document (`crates/wren-core/src/ops.rs:684-694`), so the rollup needs a new op, for example:

```text
Op::EditTaskLine { note, line, expect: String, replace: String }
```

- **The note is open:** the change goes to the draft (`edit_draft`), and the editor's content must be updated from outside the editor. That is new: it resembles an external reload, but keeps the caret and the editor's undo history.
- **The note is not open:** read the file, check that line `line` is exactly `expect`, write `replace` with the same atomic write, baseline and conflict rules autosave uses, update both indexes, and report the save.
- **The line has changed** (the note was edited since the index read it): look for exactly one line equal to `expect`. If there is one, edit it; otherwise refuse with "This task changed; the list was refreshed" and refresh.
- **Settle first.** As with boards, a view action that writes another note settles the active document first (flushes a pending draft), so two writers never race.

### 10.2 Undo

The editor's undo history is per open note and does not cover a closed note. The rollup keeps its own undo for the actions taken in it, each undone by the reverse `EditTaskLine` with the same check. This is open for discussion: no undo in a view, with a "done" toast that offers Undo, is the smaller choice.

### 10.3 `.wren/tasks.toml`

App-managed and beside `.wren/workspace.toml` (`crates/wren-core/src/workspace_state.rs:19`). Inside the workspace, so it travels with it. It holds folder exclusions, each view's remembered filters, and later kanban order (§11). It is a separate file so that `.wren/workspace.toml` stays as it is, the same reason the boards work kept the state file's v1 keys unchanged. With the feature off it is never created.

## 11. Kanban (ring 2)

- Columns are status types by default: Open, Doing, Done, Cancelled. Moving a card rewrites the character in its box through `EditTaskLine`.
- Card order within a column lives in `.wren/tasks.toml`, keyed by a task id. An id that survives edits is the hard part. The Obsidian-compatible answer is `🆔 x7k2`, the token Obsidian Tasks uses for dependencies, added to a task the first time it is reordered on a board. Tasks never reordered need no id and sort by due date.
- Later column queries: by owner (one column per person), by due bucket, or by priority, where a move rewrites the owner, the date or the priority instead of the status.
- **Editing a card** (§2 item 15). A card shows its priority badge, its due date (red when overdue) and its owners. The same keys and menus as the rollup set its status, due date and priority, and the card opens the "Create or edit task" form (§6). Optional swimlanes by priority give a priority-by-status grid, like TaskNotes' swimlanes.

## 12. Rings

The first release is **ring 0 alone**: writing and displaying tasks in the editor (§2 item 16). The views come after the editing experience has been used and refined, and are built on it.

- **Ring 0 (first release): the editor.**
  - statuses in the box, with toggle, cycle and the status menu;
  - the `📅` due date and its calendar popover;
  - priority `🔺⏫🔼🔽⏬`, with raise and lower;
  - `@owner`, highlighted and completed;
  - owner completion from the notes the search index holds (Q8, §3.1);
  - done and cancelled dates;
  - auto-suggest, and the "Create or edit task" form opened from the editor;
  - highlighting: the date red when overdue, owners, done tasks dimmed. Whether tokens collapse into chips is decided while refining this ring;
  - spell check skipping the tokens;
  - a `tasks` setting gating all of it, off by default until the ring is done, as `boards` was.
- **Ring 1: the rollup,** built on ring 0 as refined:
  - the task index (§8), which then also feeds owner completion;
  - the rollup for a scope, with status, due date and priority set from the view, the shared form, and opening at the line;
  - the write path to closed notes (§10) and the view's undo;
  - folder exclusions, the note switch and hide-older-work;
  - later in the ring: inline description edit and saved views.
- **Ring 2: kanban.** Columns by status with saved order (`🆔`), card editing, swimlanes by priority, then owner, due-bucket and priority columns. Also "copy for Jira" (the description, owner and due date as plain text, the link back to the note left out).
- **Ring 3:** a user-defined status table (on hold, waiting); scheduled and start dates; recurrence as its own primitive; reminders.

## 13. Risks and spikes before building

1. **Emoji in the editor, on both platforms.** The editor relies on the system's emoji font (`crates/wren-iced/src/fonts.rs` looks for Noto Color Emoji). Check `📅`, `✅`, `❌` and the five priority glyphs, with and without U+FE0F:
   - on Linux, with and without Noto Color Emoji, on X11 and Wayland, with tiny-skia and wgpu;
   - on macOS, on Metal;
   - for each: the glyph, its width, caret movement and selection across it, and Backspace over it (one grapheme), on both editor engines.

   The board work's tools apply: `capture_scenes.py` for screenshots on every renderer, and the smoke suites (`make smoke-x11`, `make smoke-wayland`) for behaviour. Run the macOS checks against a real Metal window, as `CLAUDE.md` describes. If a glyph fails somewhere, the fallback is a text form for that field (`due:2026-12-01`), giving up Obsidian compatibility for that field only.
2. **Input on both platforms** (§6): system emoji pickers, input methods, and the Command/Alt split. The auto-suggest and form need harness requests so the smoke suites can drive them, as the `board.*` requests do.
3. **Writing a closed note** (§10.1, ring 1): conflict rules against an external editor, autosave interplay, and pushing a change into an open editor without losing its undo history.
4. **Task identity** across edits, for refreshing the rollup and for kanban order (§11).
5. **Changing `block::Task`** touches the classifier every editor feature uses. It needs the full editor test suite on both engines.
6. **Front matter keys** without a YAML dependency (§7, item 3).

## 14. Sources

- Obsidian Tasks: Emoji format, https://publish.obsidian.md/tasks/Reference/Task+Formats/Tasks+Emoji+Format ; Dates, https://publish.obsidian.md/tasks/Getting+Started/Dates ; Statuses and status types, https://publish.obsidian.md/tasks/Getting+Started/Statuses and https://publish.obsidian.md/tasks/Getting+Started/Statuses/Status+Types ; Auto-suggest, https://publish.obsidian.md/tasks/Editing/Auto-Suggest ; Priority, https://publish.obsidian.md/tasks/Getting+Started/Priority ; the `⏫` variation-selector bug, https://github.com/obsidian-tasks-group/obsidian-tasks/issues/2273 ; Global query, https://publish.obsidian.md/tasks/Queries/Global+Query ; owner request declined, https://github.com/obsidian-tasks-group/obsidian-tasks/issues/2396 ; per-file ignore, https://github.com/obsidian-tasks-group/obsidian-tasks/issues/3822
- SilverBullet: https://docs.silverbullet.md/Guide/Task%20Management and https://docs.silverbullet.md/Attribute ; emoji deadline removed in 2.4, https://community.silverbullet.md/t/silverbullet-2-4-released/3772
- Logseq: https://deepwiki.com/logseq/docs/6.1-task-management ; org-mode: https://orgmode.org/manual/Deadlines-and-Scheduling.html and https://orgmode.org/manual/Priorities.html
- todo.txt: https://github.com/too-much-todotxt/spec/blob/main/The-Todo.txt-Format.md
- TaskPaper: https://guide.taskpaper.com/getting-started/ and https://guide.taskpaper.com/reference/dates/
- NotePlan: https://help.noteplan.co/article/42-elements-task
- Todoist: https://www.todoist.com/help/todoist/features/use-task-quick-add-in-todoist-va4Lhpzz
- TaskNotes: https://tasknotes.dev/obsidian/core-concepts/ and https://tasknotes.dev/obsidian/views/kanban-view/
- Obsidian Kanban's format: https://kanbanee.com/kanban/tools/markdown/
- Meeting-notes conventions: https://quik.md/markdown-meeting-notes-template and https://dev.to/lynkk/meeting-notes-templates-for-dev-teams-standup-retro-adr-3gcl
