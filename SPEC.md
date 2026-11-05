# Wren — Structured Notes (Design + Build Spec)

> Name: **Wren**. This document is written to be handed to a coding agent.
> Part 1 is the design and its rationale; Part 2 is the build plan with
> milestones and acceptance criteria. When the two disagree, Part 2 wins.
>
> Companion files in this folder:
> - `MOCKUP.md` — the visual design: every screen, design tokens (colors,
>   type, sizes), and interaction details. Match it for look and feel.
> - `mockup/` — the mockup's source files (`*.dc.html`), for exact values.

---

## Part 1 — Design

### 1.1 Problem

Obsidian is simple and file-based, which is good. But in practice:

- **Tab sprawl.** Every note opens another tab, so many tabs end up open.
- **No enforced structure.** A free-form vault drifts into a flat pile of
  unrelated notes.

Wren keeps what works (plain Markdown files in a directory you own) and fixes
those two problems with **a fixed, shallow hierarchy** and **one open document
at a time**.

### 1.2 Principles

1. **Files are the truth.** A workspace is a plain directory of `.md` files.
   No database. Obsidian, `grep`, git and Finder all keep working on the same
   folder.
2. **Structure by construction.** The hierarchy is exactly
   *Workspace → Section → Group → Note*. The UI never lets you build anything
   deeper.
3. **One document, no tabs.** You edit one note at a time. Quick switching and
   back/forward history take the place of tabs.
4. **Never lose a keystroke.** Autosave is on all the time. There is no Save
   command.
5. **Keyboard first, mouse friendly.** Every action is in the command palette;
   drag-and-drop is a second way to do the same moves.
6. **One op set** (as in roost). Every mutation (create, rename, move, delete,
   edit, switch workspace) is an `Op` applied by the core. UI clicks, drags,
   palette commands and tests all send the same ops. The UI reacts to events
   from the core and never keeps its own copy of the truth.

### 1.3 Concepts and on-disk layout

| Concept    | On disk                                   | Example                        |
|------------|-------------------------------------------|--------------------------------|
| Workspace  | A root directory registered in config     | `~/notes/work/`                |
| Section    | Top-level directory in the workspace      | `1-1s/`, `Projects/`, `Inbox/` |
| Group      | Directory inside a section                | `1-1s/Alice Chen/`             |
| Note       | `.md` file in a group **or** in a section | `1-1s/Alice Chen/2026-11-04.md`|

```
~/notes/work/                     ← workspace root
├── .wren/
│   └── workspace.toml            ← ordering + per-workspace UI state (optional)
├── 1-1s/                         ← section
│   ├── Alice Chen/               ← group
│   │   ├── 2026-10-28.md         ← note
│   │   └── 2026-11-04.md
│   └── Bob Ruiz/
│       └── 2026-11-03.md
├── Projects/
│   ├── Hub Migration/
│   │   └── Design notes.md
│   └── Quarterly planning.md     ← note directly in a section ("ungrouped")
└── Inbox/
    └── scratch.md
```

Rules:

- **Notes may live directly in a section** ("ungrouped") as well as in groups,
  so you never have to invent a group just to write something down. The UI
  shows ungrouped notes under a synthetic "(No group)" row at the top of the
  section's group list.
- **Depth is capped at two directories.** A directory found deeper than
  `section/group/` is not shown as structure. Its `.md` files appear as notes of
  the enclosing group, with their relative path shown as a subtitle, and the
  app shows a one-time warning. The app never creates deeper nesting itself.
- **Ignored:** dot-files and dot-directories (`.obsidian/`, `.git/`, `.wren/`),
  and non-`.md` files (for now; see the attachments roadmap item).
- **The title is the filename** without `.md`. Renaming a note renames the
  file. Characters that are invalid in filenames (`/ \ : * ? " < > |`) are
  replaced with `-` on write, and the title shown is the sanitized one.
- **Name collisions** when creating or moving get a suffix (`Notes 2.md`). The
  app never silently overwrites.

### 1.4 Layout: two switchable layouts, resizable panes

> **Visual reference:** `mockup/` next to this file (see `MOCKUP.md`). The
> mockup is the source of truth for look, spacing and colors; this section is
> the source of truth for behavior.

The hierarchy has a fixed depth, so the browser is built around it rather than
around a free tree. Wren ships **two layouts** and the user switches between
them at any time. Both always keep the **editor on the right**: panes never
wrap or stack below it.

**Layout A — Columns** (default). One column per level, each with its own
search box, like Apple Notes, Bear or Finder's column view:

```
┌───────────────────────────────────────────────────────────────────────────────────┐
│ 🐦 Wren  [▭ work ~/notes/work ▾]          [▥|▤]  Jump ⌘P  Search ⌘⇧F  Commands ⌘K ⛶ │
├────────────┬┬─────────────┬┬────────────────┬┬───────────────────────────────────┤
│ SECTIONS + ││ GROUPS·1:1s+││ NOTES·Alice  + ││ 1:1s › Alice Chen › 2026-11-04  Move… │
│ [🔍 filter]││ [🔍 filter ]││ [🔍 titles+text]│├───────────────────────────────────────┤
│ ▌1:1s   11 ││ (No group) 1││ ▌2026-11-04     ││  1  # 1:1 — Alice Chen · Nov 4        │
│  Projects 6││ ▌Alice Chen5││  2 min · API…   ││  2                                    │
│  Inbox   3 ││  Bob Ruiz  3││  2026-10-28     ││  3  ## Agenda                         │
│  Reference5││  Priya Nair2││  Oct 28 · …     ││  4  - API migration: cutover plan…    │
│            ││             ││ Sorted by mod.  ││ Markdown · 16 lines      Ln 16 ● Saved │
└────────────┴┴─────────────┴┴────────────────┴┴───────────────────────────────────┘
                ↑ drag handles between every pane (││)
```

**Layout B — Outline.** Sections and groups share one pane as a two-level
outline (sections expand/collapse with a chevron; groups are indented under
them), then the Notes column, then the editor. It gives the editor roughly
200 px more than Layout A at the same window width.

```
┌──────────────────┬┬────────────────┬┬──────────────────────────────────────┐
│ SECTIONS & GROUPS││ NOTES·Alice    ││ 1:1s › Alice Chen › 2026-11-04        │
│ [🔍 filter      ]││ [🔍 titles+text]││                                       │
│ ▾ 1:1s        11 ││ ▌2026-11-04     ││  1  # 1:1 — Alice Chen · Nov 4        │
│     (No group) 1 ││  2026-10-28     ││  …                                    │
│   ▌ Alice Chen 5 ││  2026-10-21     ││                                       │
│     Bob Ruiz   3 ││                 ││                                       │
│ ▸ Projects     6 ││                 ││                                       │
│ ▸ Inbox        3 ││                 ││                                       │
└──────────────────┴┴────────────────┴┴──────────────────────────────────────┘
```

Shared rules for both layouts:

- **Switching layout:** a two-button segmented control in the title bar
  (Columns / Outline, with `aria-pressed`), plus palette commands
  "Layout: Columns" (`⌘⌥1`) and "Layout: Outline" (`⌘⌥2`). Switching keeps the
  current selection, open note, cursor and scroll position; it is a pure view
  change and never touches the core. The choice is stored per workspace in
  `.wren/workspace.toml` (`layout = "columns" | "outline"`).
- **Selection is shared.** Both layouts render from the same UI selection
  state (section, group, note). In Outline, clicking a section toggles its
  expansion and, if it is not the current section, selects it (and its first
  group and note). Moving to a note from anywhere (jump, search, move) expands
  its section in the outline.
- **Notes column order:** most recently modified first by default (good for
  dated 1:1 notes). Can be switched to title order per group.
- **Sections and groups:** manual order (set by dragging). New items go at the
  end. The order is stored in `.wren/workspace.toml`, and items missing from
  it fall back to alphabetical order.
- **Counts** show how many notes are under each section and group.
- **Focus mode** (`⌘\`): hides every browser pane so only the editor shows
  (text centered, max ~760 px wide). Toggling again restores the previous
  layout and widths.
- A **dirty / saving / saved** indicator sits at the bottom of the editor.

**Resizable panes.** Every browser pane has a drag handle on its right edge
(between it and the next pane, including the last pane and the editor):

| Pane                     | Default | Min | Max |
|--------------------------|---------|-----|-----|
| Sections (A)             | 200 px  | 140 | 360 |
| Groups (A)               | 220 px  | 150 | 400 |
| Notes (A and B)          | 280 px  | 200 | 480 |
| Sections & groups (B)    | 260 px  | 180 | 420 |
| Editor                   | rest    | 320 | —   |

- The handle is a ~7 px hit area centered on the 1 px divider, cursor
  `col-resize`, highlighted in the accent color on hover and while dragging.
- While dragging, a small accent chip at the top of the divider shows
  "Groups · 268 px". The width updates live and is clamped to the table above.
- **Double-click** a handle to reset that pane to its default. Palette command
  "Reset column widths" resets all of them.
- **Keyboard:** handles are focusable; `←` / `→` change the width by 16 px.
- When the window is too narrow for every pane at its width, browser panes
  shrink proportionally (never below 120 px) before the editor goes below its
  320 px minimum. Panes never wrap onto a new row.
- Widths are stored **per workspace and per layout** in `.wren/workspace.toml`
  (`[widths.columns] sections = 200 …`, `[widths.outline] …`) and written when a
  drag ends, not on every mouse move.

### 1.5 Search (scoped)

Each pane's search box searches **within the current scope**. In Layout B the
Sections & groups filter covers both section and group names (a matching group
shows with its parent section expanded):

| Box      | Scope                       | Matches                                                                 |
|----------|-----------------------------|-------------------------------------------------------------------------|
| Sections | workspace                   | section names (fuzzy)                                                   |
| Groups   | selected section            | group names (fuzzy)                                                     |
| Notes    | selected group, or section if "(No group)"/none selected | note titles (fuzzy) **and** note content (substring, case-insensitive) |

Notes-column search shows a match snippet under each result. Opening a
content match puts the cursor on the first hit.

A **global search** (`⌘⇧F`) searches titles and content across the whole
workspace. It is a palette-style overlay, and each result shows its path
breadcrumb: `1-1s › Alice Chen › 2026-11-04`.

**Jump to note** (`⌘P`) is a fuzzy title search over every note in the
workspace, ranked by match and then by how recently the note was opened. With
back/forward history (`⌘[` / `⌘]`), this is what stands in for tabs.

Implementation: an in-memory index of every note (path, title, mtime, content)
built when a workspace opens and kept current from the core's own writes plus a
file watcher. Work-notes scale (thousands of files, tens of MB) makes a linear
scan per keystroke fast enough. Do **not** add a search engine such as
tantivy for the MVP. Use `nucleo-matcher` for fuzzy title matching.

### 1.6 Moving things (drag-and-drop + palette)

Allowed moves, and only these:

| Drag      | Drop onto                         | Result                                           |
|-----------|-----------------------------------|--------------------------------------------------|
| Note      | a group row (any section)         | file moved into that group directory             |
| Note      | a section row                     | file moved to the section root (ungrouped)       |
| Note      | "(No group)" row                  | same as onto the section row                     |
| Group     | a section row                     | directory moved into that section                |
| Group     | between groups (same column)      | reorder                                          |
| Section   | between sections                  | reorder                                          |

Anything else is an invalid target. A valid target highlights while hovered,
and an invalid drop does nothing.

While dragging over the **Sections** column, hovering a section row for
~600 ms "springs" it open, so the Groups column switches to that section's
groups and a note can be dropped into a group of another section in one drag.

Every move is also a palette command: **Move note to…** and **Move group to…**
open a fuzzy picker of valid destinations. This keyboard path is what
the tests use first.

Moving the currently open note is safe: autosave flushes first (see 1.7), the
file is moved, and the editor keeps its buffer, cursor and scroll position
under the new path.

### 1.7 Editor and autosave

- **MVP editor:** a plain-text Markdown editor (like Zed or VS Code with a
  `.md` file open). Monospace or a user-chosen font, Markdown syntax
  highlighting, soft wrap on by default, undo/redo, find-in-note (`⌘F`). There
  is no rendered preview in the MVP.
- **Autosave:** the note is written **750 ms after the last edit** (debounced),
  and also immediately when you switch notes, move or rename the open note,
  switch workspaces, the window loses focus, or the app quits.
- **Atomic writes:** write to `.<name>.md.wren-tmp` in the same directory,
  fsync, then rename over the target (the same pattern as roost's `state.json`).
- **External edits** (Obsidian, git pull, another machine through sync):
  - Open note, buffer **clean** → reload silently, keeping the cursor at the
    same line where possible.
  - Open note, buffer **dirty** → do not overwrite either side. Save the
    buffer as `<title> (conflict YYYY-MM-DD HHMM).md` next to the original,
    load the disk version, and show a banner linking to the conflict copy.
  - Ignore watcher events caused by the app's own writes (track recent write
    mtimes and sizes).
- **New note shortcut for dated notes:** `⌘N` in a group asks for a title and
  pre-fills today's date (`2026-11-04`). `⌘⇧N` creates the dated note
  straight away with no prompt. This makes the 1:1 flow one keystroke.

### 1.8 Workspaces and the command palette

- **Config** at the platform config dir (`~/Library/Application Support/Wren/`
  on macOS, `$XDG_CONFIG_HOME/wren/` on Linux): `config.toml` with the list
  of workspaces (`name`, `path`) and the last active one. Use the
  `directories` crate.
- **Switching workspaces** happens only through the palette ("Switch
  workspace: …"). It flushes autosave, tears down the index and watcher, and
  rebuilds the whole UI state from the new workspace's
  `.wren/workspace.toml` (selection, column widths, history). Nothing carries
  across workspaces.
- **Add workspace:** a palette command that opens a native folder picker
  (`rfd` crate). An empty folder is valid and starts with no sections.
- **Palette (`⌘K`)**: fuzzy list of every command plus context commands for
  the current selection. Reuse roost's palette model and look (see 2.4).

MVP command list:

```
Switch workspace…            Add workspace…            Remove workspace from list
New section                  New group                 New note        New dated note
Rename…                      Delete… (to OS trash)     Reveal in Finder/Files
Move note to…                Move group to…
Jump to note… (⌘P)           Search workspace… (⌘⇧F)   Find in note (⌘F)
Back (⌘[)   Forward (⌘])     Toggle focus mode (⌘\)
Layout: Columns (⌘⌥1)        Layout: Outline (⌘⌥2)     Reset column widths
Sort notes: modified / title
```

**Delete** always moves to the OS trash (`trash` crate) and never unlinks
directly. Deleting a section or group that has notes in it asks for
confirmation and shows the count.

### 1.9 Toolkit choice: iced vs GPUI

| | **iced 0.14** | **GPUI** (Zed) + `gpui-component` |
|---|---|---|
| Your experience | Shipping roost on it; palette, drag-reorder, theming, test harness already solved | New |
| Text editing | `text_editor` widget (cosmic-text): solid for prose and Markdown with a highlighter; fewer pro-editor features | `gpui-component`'s Editor (tree-sitter, built for large files); closer to the Zed feel |
| Lists / DnD | Build it yourself (roost's `strip_reorder.rs` already does) | Virtual lists, dock and drag primitives included |
| API stability | Pinned release, Elm architecture, predictable | GPUI has no stable public API yet; reached through `gpui-component`'s pinned snapshot, which changes more often |
| Platforms | macOS + Linux (proven in roost) | macOS, Linux, Windows |

**Decision: iced 0.14**, the same pin as roost, with the editor behind a seam
(`EditorView` in the UI crate) and all logic in a toolkit-neutral core. The
risk with iced is that its editor feels basic. Milestone M0 tests that early.
If it fails, the core is untouched and only the UI crate moves to GPUI.

### 1.10 Out of scope for MVP (roadmap)

1. **Pasting images / attaching files:** paste or drop writes the file to
   `<group>/_attachments/<note-title>/<name>` and inserts a relative Markdown
   link (`![](_attachments/...)`). `_attachments` is hidden from the browser.
   This needs a rendered or inline-preview mode to be useful, so it comes
   after the preview.
2. Rendered Markdown preview (side-by-side or toggle), using iced's
   `markdown` widget.
3. Wiki links `[[note]]` with jump-to and rename-safe link updates.
4. Templates per section (for example, the 1:1 template pre-fills agenda
   headings).
5. Tags / front-matter, pinned notes, daily-note section.
6. `wrenctl` CLI over the same op set (as `roostctl` does).

---

## Part 2 — Build Plan (for the coding agent)

### 2.1 Ground rules

- **Language/toolkit:** Rust stable, `edition = "2021"`, iced `=0.14.0`
  (match roost's features: `wgpu` with `tiny-skia` fallback, plus
  `highlighter` for Markdown syntax). Pin exact versions in
  `[workspace.dependencies]`.
- **Platforms:** macOS (primary) and Linux. Both must build and pass tests.
- **Follow roost's conventions** (`/Users/charliek/projects/roost/CLAUDE.md`, *Style*):
  flat modules, concrete types, no `Manager`/`Service`/`Helper` names, errors
  returned rather than logged and swallowed, inline `#[cfg(test)]` unit tests,
  no committed `TODO`s, comments only for non-obvious *why*.
- **Prefer pure-Rust dependencies.** Expected set: `iced`, `notify` (+
  `notify-debouncer-full`), `nucleo-matcher`, `directories`, `serde` + `toml`,
  `trash`, `rfd`, `thiserror`, `anyhow`, `tracing`, `tempfile` (tests),
  `chrono` or `time` (dated titles). Justify anything else.

### 2.2 Crate layout

```
wren/
├── Cargo.toml                    # workspace
├── crates/
│   ├── wren-core/               # toolkit-neutral; no iced dependency
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── model.rs          # Workspace, Section, Group, Note, ids, ordering
│   │       ├── scan.rs           # directory → model (depth rules, ignores)
│   │       ├── ops.rs            # enum Op + apply() → Result<Vec<Event>>
│   │       ├── fs_store.rs       # atomic write, move, rename, trash, collision naming
│   │       ├── index.rs          # in-memory title/content index + queries
│   │       ├── watch.rs          # notify watcher → external-change events; self-write filter
│   │       ├── autosave.rs       # debounce state machine (pure; time injected)
│   │       ├── config.rs         # global config.toml (workspace list)
│   │       └── workspace_state.rs# .wren/workspace.toml (order, widths, selection, history)
│   └── wren-iced/               # the app binary
│       └── src/
│           ├── main.rs
│           ├── app.rs            # App state, Message, update, view; owns a core Session
│           ├── columns.rs        # Layout A: section/group/note column views + filter boxes
│           ├── outline.rs        # Layout B: section/group outline pane
│           ├── panes.rs          # pane widths, resize handles, clamping, persistence
│           ├── editor_view.rs    # EditorView seam wrapping iced text_editor
│           ├── drag.rs           # drag state machine + drop-target hit testing
│           ├── palette.rs        # command palette + pickers (Move to…, Jump, Switch workspace)
│           ├── search_overlay.rs # global search
│           └── theme.rs
└── tests/                        # cross-crate integration tests against wren-core
```

**The op set** (`wren-core::ops`). Every mutation is one of these, applied
through `Session::apply(op) -> Result<Vec<Event>, OpError>`:

```rust
pub enum Op {
    CreateSection { name: String },
    CreateGroup   { section: SectionId, name: String },
    CreateNote    { parent: Parent, title: String },        // Parent::Section | Parent::Group
    Rename        { target: ItemRef, new_name: String },
    Delete        { target: ItemRef },                      // to OS trash
    MoveNote      { note: NoteId, to: Parent },
    MoveGroup     { group: GroupId, to: SectionId },
    Reorder       { target: ItemRef, before: Option<ItemRef> }, // sections / groups only
    WriteNote     { note: NoteId, contents: String },       // called by autosave
    OpenWorkspace { path: PathBuf },
}

pub enum Event {
    TreeChanged,                                 // UI re-reads model
    NoteMoved { note: NoteId, new_path: PathBuf },
    NoteSaved { note: NoteId, mtime: SystemTime },
    ExternalChange { note: NoteId, kind: ExternalChangeKind }, // Reloaded | Conflict { copy: PathBuf } | Deleted
    WorkspaceOpened,
    Warning(String),
}
```

IDs are stable for the life of a session (survive moves and renames). Map IDs
to paths in the model, and never hold `PathBuf`s across ops in the UI.

**Threading.** Same contract as roost: iced `update`/`view` on the main thread.
The file watcher and initial scan run on background tasks and deliver results
through an iced `Subscription` (see roost's `crates/roost-iced/src/engine_feed.rs`
for the pattern). Note writes may run synchronously in `update` at first
(they're small files). Measure, and move them to a task only if a write takes
more than about 5 ms.

### 2.3 Milestones

Each milestone ends with `cargo test --workspace`, `cargo clippy -- -D warnings`
and `cargo fmt --check` passing on macOS and Linux.

**M0 — Scaffold + editor spike (go/no-go on iced)**
- Workspace with two crates. The app opens a window with a hard-coded file in
  an `iced::widget::text_editor`, with Markdown highlighting.
- Check and write down in `docs/spike-editor.md`: typing latency on a
  5,000-line file; undo/redo (if `text_editor` has none built in, implement
  an edit-history stack in `EditorView` and record that); IME input (macOS
  Japanese input as the test); soft wrap; selection and clipboard; find.
- **Exit:** the spike doc gives a go or no-go for iced. If no-go, stop and
  report. Do not start on GPUI without the human's decision.

**M1 — Core model, scan, ops, fs (no UI)**
- `scan.rs`, `model.rs`, `fs_store.rs`, `ops.rs` with every op except
  `OpenWorkspace` switching.
- **Tests** (temp dirs via `tempfile`): scan respects the depth cap and ignore
  rules; each op does the right thing on disk *and* in the model; collision
  naming; filename sanitizing; a moved note keeps its `NoteId`; `Delete` goes
  to trash (inject a trash trait so tests can swap in a fake); atomic write
  leaves no temp file behind on success, and a failure mid-write leaves the
  original intact.

**M2 — Columns + editor + autosave**
- Layout A (1.4) wired to the model: selection, counts, "(No group)"
  row, notes sorted by mtime, breadcrumb, `+` buttons, rename via
  double-click or Enter.
- `autosave.rs` as a pure state machine (`on_edit(now)`, `on_tick(now)`,
  `on_flush_trigger()` → `Option<WriteNote>`), unit-tested with fake time.
  Wire all the flush triggers in 1.7.
- Saved/dirty indicator. Back/forward history.
- **Exit:** create a section, group and note, type, quit, relaunch: the text
  is there. Kill the process 1 s after typing: the text is there.

**M3 — Search**
- `index.rs`: build on open and update on each `Event`. Queries: fuzzy titles
  in scope, content substring in scope with snippet.
- Per-column filter boxes (1.5), Jump to note (`⌘P`) with recency, global
  search (`⌘⇧F`).
- **Tests:** scope correctness at each level; snippet extraction; index stays
  correct after move, rename and delete. Benchmark: 5,000 notes / 25 MB → a
  query in under 30 ms in a release build (test marked `#[ignore]`, run in CI
  nightly).

**M4 — Moving: palette first, then drag-and-drop**
- "Move note to…" and "Move group to…" pickers.
- `drag.rs`: press, threshold (4 px), dragging, hover target, drop or cancel
  (Esc). Draw a ghost row under the cursor, highlight valid targets, show an
  insertion line for reorders, and spring-open sections after a 600 ms hover.
  Use `crates/roost-iced/src/strip_reorder.rs` and `sidebar_resize.rs` in roost
  as the reference for how drag is done in iced here.
- Keep the drag state machine pure (input events in, `Option<Op>` out) and
  unit-test every row in the 1.6 table, including invalid drops.
- Moving the open note keeps the buffer, cursor and scroll position.

**M5 — Workspaces + full palette**
- `config.rs`, `workspace_state.rs`, Add/Switch/Remove workspace, native
  folder picker. Switching tears down and rebuilds as described in 1.8.
- Full MVP command list (1.8) in the palette. Look and keyboard handling
  follow roost's palette (`crates/roost-iced/src/palette_scroll.rs` and the
  palette code in `app.rs`).
- Layout B (outline pane) and the layout switch (title-bar toggle + palette
  commands); resizable panes with clamping, double-click reset and keyboard
  resize (1.4). Unit-test the width clamping and the narrow-window shrink rule.
- Persist per workspace: layout, per-layout pane widths, last selection, last open note,
  history, sort mode.

**M6 — External changes + polish**
- `watch.rs` with self-write filtering; the clean-reload and dirty-conflict
  behavior in 1.7; external create, delete and rename show up in the columns
  within 1 s.
- Delete confirmations, Reveal in Finder/Files, focus mode, resizable columns,
  light/dark theme following the system.
- **Tests:** a watcher integration test that edits a file from outside and
  checks for one `ExternalChange::Reloaded` and no event for the app's own
  writes; a conflict test that checks both versions survive.

### 2.4 Things to reuse from roost (read, don't copy blindly)

| Need                     | Look at (in `/Users/charliek/projects/roost`)                  |
|--------------------------|----------------------------------------------------------------|
| Command palette UX       | `crates/roost-iced/src/app.rs` (palette), `palette_scroll.rs`  |
| Drag reorder in iced     | `crates/roost-iced/src/strip_reorder.rs`                       |
| Resizable panes          | `crates/roost-iced/src/sidebar_resize.rs`                      |
| Background → UI feed     | `crates/roost-iced/src/engine_feed.rs`                         |
| Bundled fonts / theming  | `crates/roost-iced/src/fonts.rs`, `chrome.rs`                  |
| Atomic state writes      | roost-engine's `state.json` persistence                         |
| One-op-set architecture  | `docs/development/vision.md` (DL-11)                           |
| macOS bundle             | `mac/` scripts, `make bundle-iced`                             |

### 2.5 Definition of done (MVP)

- [ ] Two or more workspaces registered; switching through the palette fully replaces the UI.
- [ ] Sections, groups and notes can be created, renamed and deleted (to trash) from the UI and the palette.
- [ ] Notes can live in groups or directly in sections.
- [ ] Both layouts (Columns, Outline) work and can be switched at any time without losing the open note; the editor always stays on the right.
- [ ] Every pane is resizable, clamped, resettable, and its width persists per workspace and layout.
- [ ] Every move in 1.6 works by drag *and* by palette; invalid drops are refused.
- [ ] Scoped search in each column, Jump to note, global search with snippets.
- [ ] Autosave: no Save command exists; killing the process loses at most 750 ms of typing.
- [ ] External edits reload cleanly; conflicts never lose either version.
- [ ] The workspace folder opens in Obsidian with no changes needed (only `.wren/` added).
- [ ] Core test coverage for every op, scan rule, autosave transition and drag rule.
- [ ] Builds and passes tests on macOS and Linux.

### 2.6 Open questions for the human (don't block on these; use the default)

| Question | Default |
|---|---|
| App name | `wren` (decided) |
| Default notes sort | modified, newest first |
| Show `.md` files nested deeper than group as notes, or hide them? | show, with path subtitle and a one-time warning |
| Autosave debounce | 750 ms |
| Dated-note title format | `YYYY-MM-DD` |
