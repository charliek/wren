# Workspace folder

A workspace is a directory you own. Wren reads its folders and Markdown
files directly and adds one `.wren/` directory for its own metadata.

| Concept | On disk | Example |
|---|---|---|
| Workspace | A registered root directory | `~/notes/work/` |
| Section | A directory in the workspace root | `1-1s/` |
| Group | A directory inside a section | `1-1s/Alice Chen/` |
| Note | A `.md` file inside a group | `1-1s/Alice Chen/2026-11-04.md` |

## Rules

- **Notes live in groups.** A `.md` file directly in the workspace root or
  directly in a section folder is ignored, and Wren shows a warning naming
  it.
- **Depth is capped at two folders.** A folder deeper than `section/group/`
  is not shown as structure. Its `.md` files appear as notes of the
  enclosing group, with their relative path as a subtitle, and Wren warns
  once. Wren never creates deeper folders itself.
- **Ignored:** dot-files and dot-directories (`.obsidian/`, `.git/`,
  `.wren/`), non-`.md` files, symlinks, and special files. Wren leaves them
  untouched.
- **The title is the filename** without `.md`. Renaming a note renames the
  file. Characters that are invalid in filenames (`/ \ : * ? " < > |`) become
  `-`.
- **Names never collide.** Creating or moving onto an existing name adds a
  suffix (`Notes 2.md`); Wren never overwrites a file.
- **Delete goes to the OS trash.** Wren never unlinks a note or folder.
- **Large files are left alone.** A note over 64 MiB is not opened or
  rewritten.

## `.wren/workspace.toml`

Optional per-workspace state. Missing or malformed metadata never affects
your notes; Wren warns and carries on with defaults. Unknown keys survive
Wren's saves, but comments and formatting are not kept.

| Key | Meaning |
|---|---|
| `schema_version` | Format version, currently `1`. |
| `selection` | The selected section or group, as a workspace-relative path. |
| `last_open_note` | The note reopened on launch. |
| `history`, `history_position` | Back/Forward history. |
| `recent` | Most recently opened notes, for Jump to note ranking. |
| `order` | Manual order of sections and groups. Items missing from it sort alphabetically after it. |
| `widths` | Column widths, under `[widths.columns]`. |
| `sort` | Per-group note order: `modified` (default) or `title`. |

## App configuration

Two files live in the platform config directory, and neither is inside a
workspace:

| File | Owner | Contents |
|---|---|---|
| `config.conf` | You | Theme, editor font, keybindings. See [Configuration](configuration.md). |
| `config.toml` | Wren | Registered workspaces and the last active one. |
