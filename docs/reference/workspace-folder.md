# Workspace folder

A workspace is a directory you own. Wren reads its folders, Markdown
files, and boards directly and adds one `.wren/` directory for its own
metadata.

| Concept | On disk | Example |
|---|---|---|
| Workspace | A registered root directory | `~/notes/work/` |
| Section | A directory in the workspace root | `1-1s/` |
| Group | A directory inside a section | `1-1s/Alice Chen/` |
| Note | A `.md` file inside a group | `1-1s/Alice Chen/2026-11-04.md` |
| Board | A `.wren-board` file inside a group | `Projects/API Migration/Cutover flow.wren-board` |

## Rules

- **Notes live in groups.** A note, Markdown or board, directly in the
  workspace root or directly in a section folder is ignored, and Wren shows
  a warning naming it.
- **Kind comes from the extension.** A file is a Markdown note when its
  name ends in `.md` and a [board](boards.md) when it ends in `.wren-board`,
  in any case, after at least one other character: `x.md.wren-board` is a
  board titled `x.md`. Boards follow every rule on this page that notes
  do, and keep their kind and extension through rename and move.
- **Depth is capped at two folders.** A folder deeper than `section/group/`
  is not shown as structure. Its notes appear as notes of the
  enclosing group, with their relative path as a subtitle, and Wren warns
  once. Wren never creates deeper folders itself.
- **Ignored:** dot-files and dot-directories (`.obsidian/`, `.git/`,
  `.wren/`), files that are not notes, symlinks, and special files. Wren
  leaves them untouched. With `boards = off` in
  [`config.conf`](configuration.md), `.wren-board` files are ignored too.
- **The title is the filename** without `.md` or `.wren-board`; a board
  has no title inside the file. Renaming a note renames the file.
  Characters that are invalid in filenames (`/ \ : * ? " < > |`) become
  `-`. A Markdown note and a board may share a title in one group
  (`Flow.md` and `Flow.wren-board`); the notes list tells them apart by the
  board icon.
- **Names never collide.** Creating or moving onto an existing name adds a
  suffix (`Notes 2.md`, `Flow 2.wren-board`); Wren never overwrites a
  file. Only a file with the same full name collides.
- **Line endings are kept.** Wren saves a Markdown note with the line
  endings it had (LF, CRLF, CR, or a mix). A line break you type or paste takes the
  ending of the line it splits; on the last line, which has none, it takes
  the line above's, so a CRLF note stays CRLF. In a mixed note, a lone CR
  followed directly by an LF (or an LF by a CR) would read back as one
  break, so a break you type, paste, or leave by deleting a line's text
  takes a neighbouring ending instead, and every line you see is saved.
- **Delete goes to the OS trash.** Wren never unlinks a note or folder.
- **Large files are left alone.** A note over 64 MiB is not opened or
  rewritten. A board over 8 MiB or with more than 50,000 shapes and
  connectors opens as text, and search reads it as text (see
  [Limits](boards.md#limits)).

## `.wren/workspace.toml`

Optional per-workspace state. Missing or malformed metadata never affects
your notes; Wren warns and carries on with defaults. Unknown keys survive
Wren's saves, but comments and formatting are not kept.

| Key | Meaning |
|---|---|
| `schema_version` | Format version, currently `1`. |
| `selection` | The selected section, group, or note (a board included), as a workspace-relative path. |
| `last_open_note` | The Markdown note reopened on launch. |
| `history`, `history_position` | Back/Forward history, Markdown notes only. |
| `recent` | Most recently opened Markdown notes, for Jump to note ranking. |
| `[navigation]` | The same lists with boards in them: `history`, `history_position`, `recent`, and `last_open`. |
| `order` | Manual order of sections and groups. Items missing from it sort alphabetically after it. |
| `widths` | Column widths, under `[widths.columns]`. |
| `sort` | Per-group note order: `modified` (default) or `title`. |

Wren 0.1 rejects the whole file when `last_open_note`, `history`, or
`recent` names anything but a `.md` note, so those keys only ever hold
Markdown paths: they are the lists with the boards left out, and
`history_position` points at the last Markdown entry at or before the real
position. The full lists go in a `[navigation]` table, written only while
one of them names a board and removed otherwise, so a workspace without
boards, or with `boards = off`, keeps a state file an older Wren reads
unchanged. If an older Wren has navigated since, its lists win and the
stale `[navigation]` table is dropped at the next save. A workspace left
on a board reopens on it through `selection` or `last_open`; with
`boards = off` it reopens on the Markdown note an older Wren would.

## `.wren/tasks.toml`

Wren's own file for [tasks](tasks.md), beside `workspace.toml` and kept
inside the workspace so it travels with it. It is created only once
something is set, and never exists with `tasks = off`: with tasks off Wren
neither reads nor writes it.

| Key | Meaning |
|---|---|
| `schema_version` | Format version, currently `1`. |
| `exclude` | Sections and groups, as workspace-relative paths, whose tasks are left out of every summary and count (**Hide tasks from this group**). |
| `[views."<path>"]` | The summary view's remembered filters for one scope, keyed by the section's, group's or note's path, or `"."` for the workspace. |

A view's table holds only the keys that differ from the default, so a view
left as it opens has no table:

| Key | Values | Default |
|---|---|---|
| `statuses` | a list of `open`, `doing`, `done`, `cancelled` | none (every status) |
| `owner` | `me`, or `@name` | anyone |
| `group` | `due`, `priority`, `owner` | `note` |
| `hide_old` | `false` | `true` |
| `due` | `overdue`, `earlier`, `today`, `week`, `next`, `later`, `none` | any date |
| `priority` | `highest`, `high`, `medium`, `low`, `lowest`, `none` | any priority |

```toml
schema_version = 1
exclude = ["Reference/Templates"]

[views."Work/Planning"]
statuses = ["open", "doing"]
group = "due"
```

Renaming or moving a section, group or note inside Wren carries its entries
along, and deleting one inside Wren removes them. Unknown keys survive Wren's saves, but comments and
formatting are not kept. A file that is not valid TOML, not UTF-8 or of
another `schema_version` is never overwritten: Wren warns, uses the
defaults, and leaves the file as it is. A note's own switch,
`tasks: off` in its front matter, lives in the note, not here.

## App configuration

Two files live in the platform config directory, and neither is inside a
workspace:

| File | Owner | Contents |
|---|---|---|
| `config.conf` | You | Theme, editor font, keybindings, and the `boards` and `tasks` settings. See [Configuration](configuration.md). |
| `config.toml` | Wren | Registered workspaces and the last active one. |
