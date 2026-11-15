# First run

## Workspaces

On first launch Wren shows onboarding. **Add workspace…** opens a folder
picker; an empty folder is fine. Wren remembers registered workspaces and
reopens the last one on the next launch.

| Command | Effect |
|---|---|
| **Add workspace…** | Register a folder and open it. |
| **Switch workspace…** | Open another registered workspace. The open note saves first. |
| **Remove workspace from list** | Forget a registration. The folder and its files are untouched. |

The workspace button in the title bar opens **Switch workspace…** too.

## Sections, groups, and notes

A section is a top-level folder, a group is a folder inside a section, and a
note is a `.md` file inside a group. Notes always live in a group.

In an empty workspace the editor area walks you through the first of each:
**New section…**, then **New group…**, then **New note…**. Creating a group
selects it, so the next note goes there. The `+` button in each column
header does the same.

| Action | macOS | Linux |
|---|---|---|
| New note | ⌘N | Alt+N |
| New dated note (today's date, no prompt) | ⇧⌘N | Alt+Shift+N |
| Command palette | ⇧⌘P or ⌘K | Alt+Shift+P or Alt+K |

New section, New group, Rename, Delete, Move, and Reveal are in the command
palette. Rename note is also ⌘R (Alt+R on Linux), and moves also work by drag
and drop: a note onto a group, a group onto a section, and sections or
groups reordered within their column. Delete moves items to the OS trash and
asks first when a section or group still holds notes.

## Boards

A note can also be a [board](../reference/boards.md): a canvas of shapes
and connectors for flowcharts and small diagrams, saved as
`Name.wren-board` in the group. In **New note…**, press Tab to switch the
kind pill from Markdown to Board before Enter, or run **New board…** from
the palette. On a new board, Tab adds a shape and opens its label; type,
then Tab again for the next step, connected below it. The letters R, O, D,
U, and T pick a shape or text tool, the digits 0 to 7 colour the
selection, and **Export board as SVG…**, **PNG…**, or **draw.io…** in the
palette writes it out. Boards are on by default; `boards = off` in
[`config.conf`](../reference/configuration.md) hides them.

## Finding notes

| Tool | Scope |
|---|---|
| Column filter boxes | Section names, group names in the selected section, or note titles and text in the selected group. |
| **Jump to note** (⌘P / Alt+P) | Fuzzy titles across the workspace, most recently opened first. |
| **Search workspace** (⇧⌘F / Alt+Shift+F) | Titles and text across the workspace, board labels included, with snippets. A hit on a board opens it with the matching element selected and centred. |
| **Find in note** (⌘F / Alt+F) | The open note. On a board, the board's labels. |
| **Jump to heading** (⇧⌘O / Alt+Shift+O) | The open note's outline. On a board, its labelled shapes and connectors. |
| **Back / Forward** (⌘[ / ⌘], Alt on Linux) | Notes you visited. |

**Focus mode** (⌘Y / Alt+Y) hides the browser columns and centres the
text in the window; press it again to bring them back. Outside focus mode
the text starts at the left of the editor pane. Either way long lines wrap
at 100 characters, or at the pane's edge when it is narrower;
`editor-wrap-column` in [Configuration](../reference/configuration.md)
changes it.
