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

## Finding notes

| Tool | Scope |
|---|---|
| Column filter boxes | Section names, group names in the selected section, or note titles and text in the selected group. |
| **Jump to note** (⌘P / Alt+P) | Fuzzy titles across the workspace, most recently opened first. |
| **Search workspace** (⇧⌘F / Alt+Shift+F) | Titles and text across the workspace, with snippets. |
| **Find in note** (⌘F / Alt+F) | The open note. |
| **Back / Forward** (⌘[ / ⌘], Alt on Linux) | Notes you visited. |

**Focus mode** (⌘Y / Alt+Y) hides the browser columns so only the editor
shows; press it again to bring them back.
