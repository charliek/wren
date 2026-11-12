# Boards: plan and contract

Status: **planned; nothing built** (written 2026-11-12 against `main` at `393ae14`, v0.1.1). The work happens on the integration branch `boards` (worktree `.claude/worktrees/boards`), with no pull requests until the MVP is done. Tags `boards-p1`, `boards-p2` and `boards-p3` mark the phase ends. The `boards` setting hides everything until the last code commit of Phase 3 turns it on.

How to use this document:
- It is the contract. A fresh session starts at §7.1, reads `CLAUDE.md`, then the sections its work item names. §4 is the only contract for the file format and §5 the only contract for behaviour: work items cite their rule numbers. Research notes are background and never override them.
- Line references are to `main` at `393ae14` and were re-read when this was written. Later phases move them; find the named function.
- Where the code must differ from this text, the chain records the deviation in its own log subsection (§8.5) and the orchestrator folds accepted ones into the spec text after the merge.
- The research, the decision record, the reference scripts, the workflow scripts and their raw results live in `.claude/board-notes/` in the main checkout. Git ignores that folder (`.gitignore:7`), so nothing in it exists in a chain worktree, on the Mac or in CI. A brief names such a file by its absolute path, and a file that a committed test needs is copied into the repository by the item that first uses it (§7.1). This repository is public: no host name, user name or private link goes into a committed file.

## 1. What we are building and why

Wren gets a second kind of note, a **board**: a canvas of shapes and connectors for flowcharts and small diagrams, saved as `Name.wren-board` beside the Markdown notes of a group. The file is Wren-owned canonical TOML with one field per line, so a board in a git repository is documentation whose changes are tracked and readable. The board is drawn by a native iced widget (no webview) and exports to SVG and PNG, in the app and from a `wren export` command line, so it can be shown anywhere.

**The shape of the design.**
- `wren-board` is a toolkit-neutral crate (as `wren-markdown` is): the model, the reader and canonical writer, the integrity check, pairwise connector routing, and one scene function that turns model, theme colours and a text-measurement trait into a display list of paths and pre-wrapped text lines. The widget paints that list and the SVG writer serialises the same list; PNG is that SVG through resvg.
- `wren-canvas` is the iced widget (as `wren-editor` is): a custom advanced `Widget` that paints the scene, plus a pure interaction machine.
- The Session stays String-typed. The board surface sends the existing `Op::EditDraft` with the serialised board when a gesture commits, so autosave, conflict copies and external reload work as they do for notes.
- Kind is decided by the file extension; a note-kind type replaces every `.md` assumption in `wren-core`.

**Rings.**
- Ring 0, the MVP: eight shapes and free text; connectors that stay attached (straight and right-angled, arrowheads, dashes, one label); theme colours by fill level; four stacking commands; align and distribute; grid snap; select, marquee, duplicate, copy and paste (a text clipboard in the board's own format), undo; keyboard building and walking; labels in search; SVG and PNG export; `wren export`.
- Ring 1: note cards, images, frames and groups, a sticky preset, lock, curved and hand-bent connectors, alignment guides, style copy and paste, draw.io export then JSON Canvas export, a layout pass serving `wren import`, Mermaid import and Tidy.
- Ring 2: freehand, sketch style, formatted labels, board previews inside notes, mind-map mode.

**Out of the MVP:** any import, agent integration, git features (merge driver, reading git state), sync, element-level merge of concurrent edits, a sidecar SVG, containers created in the UI, and draw.io export unless the owner schedules the optional Phase 3 chain.

**Three phases, each ending in something to run** (§6.4 says how):
1. *See a board.* With the setting on, a hand-written board opens, renders on wgpu and tiny-skia, pans, zooms and exports to SVG. With it off nothing changes.
2. *Build a flowchart.* Full editing by mouse and by keyboard; the keyboard-only 15-node flowchart test passes; schema 1 is locked.
3. *Finish and harden.* Colours, stacking, align, the actions palette, jump, PNG, the command line, the menu, docs, limits, the final review, and the setting on by default.

For calibration: the research estimated 14 to 18 developer-weeks by hand; the Markdown editor rebuild (three workflow phases and a final review) ran in about seven hours on 2026-11-08. This plan is about 40 implementing sessions in nine workflow runs, an estimated 34 to 45 hours of running time before owner time.

## 2. Decisions

### 2.1 The owner's decisions (2026-11-12)

1. **The board is the next piece of work.** No plumbing-only release; the next release is likely a minor version with this feature. Doing the plumbing first in build order, "to build a strong base", is agreed.
2. **Git.** "Share through git" means boards serve as documentation in a repository. Wren works with a git repository as a workspace and adds no git sync and no git-specific features now. Sync "will likely be a focused feature later"; until then the user syncs by hand.
3. **Command line.** A CLI usable by hand or by automated processes is welcome.
4. **Agents and import.** First-class agent support is an eventual goal, likely through a CLI import. "For a first release no import is required, or agent integration with it."
5. **File format.** A diffable format; TOML works "if we don't lose other things by choosing it that are more valuable". The owner would rarely read a diff, wants changes tracked, distinct and somewhat readable, and does not want the file's text shown in Wren's UI. (The one exception in this plan is the raw-text fallback for a board that cannot be parsed, §3.3.)
6. **Interchange.** Export first, import later. SVG is "very useful for display"; draw.io is "more of a standard"; getting boards into many places is the priority, in a design that scales to more formats. Whether draw.io export is inside the MVP waits on a feasibility study (question 14 below).
7. **How to build.** A longer-lived feature branch, no PRs for now, many commits; automated reviews at bigger milestones as in the editor rebuild; about three phases with a workflow per phase, each ending in something that can be tested and seen in a local build.
8. **Name and extension.** The kind is a **board**, saved as `Name.wren-board`. It is a kind of note. Kind is decided by the last extension, case-insensitive; the title is the file name minus that extension; there is no title inside the file.
9. **Creating one.** The New note dialog gains kind pills, with Tab switching the kind while focus stays in the name field (§5.4 holds one rule per sentence of this decision).
10. **Release point.** The `boards` setting defaults to on and the branch is ready to merge for the next minor version after Phase 3, not after Phase 2.
11. **Reviews.** Fresh Claude agents review every chain adversarially; at the end of each phase a Codex pass also reviews the phase diff, and its findings are triaged and fixed before the next phase starts; a multi-lens final review ends Phase 3.
12. **Model budget.** The owner's Fable quota is limited. Each session uses the right model for its task (Fable, Opus or Sonnet) instead of everything running on Fable; between Sonnet and Opus, "if on the fence I'd default up to opus". §7.1 holds the tier of every session.

### 2.2 Working assumptions (the owner has seen them and not objected; any can still be overridden)

- The syntax is a canonical TOML subset (§4).
- Two crates, `wren-board` and `wren-canvas`; board-surface commands live under `board.*` with `Scope::Board`.
- One surface: shapes first; note cards, images and frames arrive later as new kinds in the same file format.
- Keyboard: plain arrows walk the selection; Alt+arrow moves a shape; Cmd (macOS) or Ctrl (Linux) + arrow quick-connects; Tab adds the next step.
- A sticky-note preset later; no freehand.
- The `boards` setting is off by default until the last code item of Phase 3 (§7.4, R3).

### 2.3 Choices made in assembling this plan

Three reasoned departures from the wording of the decision record, section 3. Each amends it:
- **A drag in flight is cancelled, not committed,** before a non-passive op (note switch, quit, blur). Only an open label is committed. A half-finished move is never written.
- **A dangling connector end is drawn as a short stub** from the valid end, not as a free end, and a connector with two dangling ends is not drawn (it stays in the file and in the issue list).
- **Model equality in the pristine rule is equality of canonical bytes** of two models (§4.13). Canonical bytes are never compared with the file to decide cleanliness.

Where the section drafts disagreed, these answers were chosen; each is one place in §3 to §5 and the owner may override any before its phase starts:
- Colours are `neutral` plus seven hues, named by token in commands (`board.color.red`), on keys 0 to 7 (§4.8).
- A board zooms through the existing text-size commands, so there are no `board.zoom.in/out/reset` commands and no board default shadows an app shortcut (§5.1, §5.13).
- The keyboard's distant connect is an inline query inside the interaction machine, not a palette mode (§5.12 rule 3).
- `BoardView` owns the model, the pristine pair and the synced text from Phase 1 (§3.2).
- Export commands are `note.export_svg`, `note.export_png` and `note.export_drawio`, App scope (§5.19).
- Align and distribute do not snap to the grid (§5.16). An overflowing label is drawn unclipped (§3.7). Deleting a node deletes its descendants (§5.10). `locked` has no effect in ring 0 (§4.5).
- Dropped as beyond ring 0: `board.size.fit`, a sticky style for the next node, honouring `locked`. Fill is set by three explicit commands and there is no cycle command (§5.16).

### 2.4 Open owner questions, each with the default the plan uses until answered

| # | Question | Default |
|---|---|---|
| 1 | Colour vocabulary: `neutral` plus seven hues (red, orange, yellow, green, cyan, purple, blue)? It is frozen at the Phase 2 schema lock. | Yes. |
| 2 | Zoom keys: Linux gets Alt+=, Alt+- and Alt+0 (the text-size shortcuts), not Ctrl; macOS gets Cmd. Ctrl zoom keys on Linux would need a second set of commands. | Accept Alt on Linux. |
| 3 | Review weight at the end of Phases 1 and 2: the data-safety lens plus the Codex pass, or all four lenses and the full fuzz each time? | Data safety plus Codex; four lenses at the end of Phase 3. |
| 4 | Mac console checks (trackpad, Cmd keys, input method, Board menu): do they block closing a phase? | No. Recorded as outstanding; required before the merge to `main`. |
| 5 | A label that does not fit a hand-sized shape: drawn unclipped, or cut with an ellipsis? Frozen into the Phase 1 golden files. | Unclipped. |
| 6 | May `Flow.md` and `Flow.wren-board` sit in one group, told apart by the kind icon? | Yes (§3.5). |
| 7 | Confirm the first two amendments of §2.3 (drag cancelled; dangling end as a stub). | Confirmed as written. |
| 8 | Dashed outlines on shapes and the eight reshape commands are beyond the ring 0 list. Keep or cut? | Keep, as the first two cuts if a phase runs long (§7.3, §7.4). |
| 9 | Will `boards` be pushed during the build? Without a push, macOS code is covered only by the Linux cross-target lint and the Mac over ssh. | The owner pushes at each phase end; no phase waits for it. |
| 10 | May the orchestrator use the Mac over ssh unattended, after the spike and at each phase end? May pytest be installed there? | Yes to ssh builds, Rust tests and scene capture. No pytest: the functional board tests stay Linux-only. |
| 11 | Real-GPU numbers for the spike: are tiny-skia in release, lavapipe and Metal over ssh enough to pass it? | Yes. A desktop run on the Linux machine is wanted, not required. |
| 12 | Nine workflow runs (three per phase, two to five hours each, a handoff note between them) rather than one run per phase? | Nine runs. |
| 13 | The Codex pass sends the phase diff to another provider. Who starts it, with which login? | The orchestrator, through the installed Codex CLI and the owner's existing login. If the login is missing the run stops there and says so. |
| 14 | draw.io export: inside the MVP (the optional Phase 3 chain) or right after? The feasibility study is done and a one-way export works (§7.4): 6 to 9 working days by hand, after connectors, styling and the export command exist. | Right after. The optional chain runs only if the owner asks for it before run 3b. |
| 15 | A macOS pinch side channel in the vendored `iced_winit` (iced 0.14 forwards no pinch event). | Not in the MVP. |

## 3. Architecture

### 3.1 Crates

```
crates/wren-board/    toolkit-neutral. deps: toml (the workspace pin, parsing only), unicode-segmentation.
                      No iced, no wren-core, no app type.
  lib.rs      Board, Node, Edge, Id, End, Palette, GRID (20), EMPTY ("schema = 1\n")
  read.rs     parse(&str) -> Result<Board, ParseError>                     §4.12
  write.rs    write(&Board) -> String, hand-written and canonical           §4.11
  check.rs    check(&Board) -> Vec<Issue>; access(&Board) -> Access          §4.10, §4.12
  shape.rs    outlines, label boxes and anchors of the eight shapes          §3.7
  route.rs    pairwise straight and elbow routes; nominal sides              §5.12 rule 4
  style.rs    resolve(colour, fill, &Palette) -> Resolved                    §3.7
  scene.rs    build(&Board, &Palette, &dyn Measure) -> Scene; Measure; FixedMeasure
  svg.rs      write(&Scene, &Options) -> String
  search.rs   search_text(&Board) -> String
  Phase 2:    ids.rs, z.rs, snap.rs, edit.rs (+ edit/), clip.rs, history.rs, place.rs, walk.rs
crates/wren-canvas/   the iced widget. deps: iced (its own minimal features, as wren-editor's, plus canvas), wren-board.
                      No wren-core, no app message. A test-support feature holds harness accessors.
  lib.rs      BoardView, Message, Update, Effect, Style, Viewport
  widget.rs   a custom advanced Widget (not canvas::Program, which cannot reach the clipboard, the input
              method, focus or layers); paints the Scene through iced's geometry API
  machine.rs  Machine::step, Input, Command; pure, no iced type in the file
  measure.rs  Measurer: scene::Measure over cosmic-text, reached through iced::advanced::graphics::text
  Phase 2:    hit.rs, handles.rs, label.rs
wren-core     gains kind.rs (NoteKind) and depends on wren-board (label extraction for search, theme::board_palette).
wren-iced     depends on wren-canvas. app/board.rs holds the surface decision and, from Phase 2, board_edit,
              board_changed and settle_board. Phase 3 adds board_export.rs (resvg) and the export CLI.
```

Rules. `wren-board` names no iced, `wren-core` or app type. `wren-canvas` names no `wren-core` type and no app message; the host passes a `Palette`, a `Style`, a font and a key policy. resvg is reachable only from `wren-iced`. No `unsafe`. No new package enters `Cargo.lock` except the six the `canvas` feature brings (`lyon`, `lyon_algorithms`, `lyon_geom`, `lyon_path`, `lyon_tessellation`, `float_next_after`); resvg `=0.45.1` is already locked (`Cargo.lock:2978`); it becomes a dev-dependency of `wren-iced` for the Phase 1 confirmation step and a direct dependency in Phase 3. Property tests use a seeded generator in the test module. Random ids come from `std::hash::RandomState`.

### 3.2 Public surface

```rust
// wren-board
pub fn parse(source: &str) -> Result<Board, ParseError>;   // ParseError { line, column, message }; never panics
pub fn write(board: &Board) -> String;                      // canonical, deterministic, idempotent
pub fn check(board: &Board) -> Vec<Issue>;                  // non-destructive
pub fn access(board: &Board) -> Access;                     // Editable | ReadOnly(reason)
pub fn search_text(board: &Board) -> String;
pub struct Palette { pub canvas: Rgba, pub ink: Rgba, pub muted: Rgba, pub raised: Rgba, pub accent: Rgba,
                     pub issue: Rgba, pub hues: [Rgba; 7], pub dark: bool }
pub mod style { pub fn resolve(color: &Color, fill: Fill, palette: &Palette) -> Resolved; }  // Color: Token | Hex | Unknown
pub mod scene {
    pub trait Measure { fn width(&self, text: &str, face: Face, size: f32) -> f32; fn line_height(&self, face: Face, size: f32) -> f32; }
    pub struct Scene { pub bounds: Rect, pub items: Vec<Item> }                 // back to front
    pub struct Item { pub source: Source, pub bounds: Rect, pub draw: Draw }     // Source::Node(Id) | Source::Edge(Id)
    pub enum Draw { Path { segments: Vec<Segment>, fill: Option<Rgba>, stroke: Option<Stroke> },
                    Text { lines: Vec<Line>, face: Face, size: f32, color: Rgba } }   // Line { text, origin, width }
    pub fn build(board: &Board, palette: &Palette, measure: &dyn Measure) -> Scene;
}
pub mod svg { pub struct Options { pub background: Option<Rgba>, pub padding: f32 } pub fn write(scene: &Scene, options: &Options) -> String; }
// Phase 2: edit::{Intent, Change, Plan}, edit::plan(&Board, &Intent, &mut dyn Ids) -> Plan, Board::apply(&[Change]),
//          clip::copy, history::History, walk::next, place::slot, z::key_between

// wren-canvas
impl BoardView {
    pub fn open(text: &str, palette: Palette, font: iced::Font) -> Result<Self, ParseError>;
    pub fn reload(&mut self, text: &str) -> Result<(), ParseError>;   // the Session's text changed underneath; keeps the viewport
    pub fn synced(&self) -> &str;                                    // the Session text this view last took or sent
    pub fn set_palette(&mut self, palette: Palette);                 // theme change: one scene rebuild
    pub fn board(&self) -> &Board;  pub fn scene(&self) -> &Scene;  pub fn issues(&self) -> &[Issue];  pub fn access(&self) -> &Access;
    pub fn viewport(&self) -> Viewport;  pub fn set_viewport(&mut self, v: Viewport);
    pub fn zoom_step(&mut self, by: i8);  pub fn zoom_reset(&mut self);  pub fn fit(&mut self);
    pub fn update(&mut self, message: Message, now: Instant) -> Update;
    pub fn view(&self, id: &'static str, style: Style) -> iced::Element<'_, Message>;
    // Phase 2: command(Command, now) -> Update;  settle(now) -> Update;  saved(&mut self, text: &str);  draft(&mut self) -> String
}
pub enum Message { Input(u64 /* epoch */, Input) /* Phase 2: Label(u64, LabelAction), Command(Command), Pasted(u64, String) */ }
pub struct Update { pub changed: bool, pub effects: Vec<Effect> }   // Effect: ClipboardWrite, ClipboardRead, OpenActions, OpenJump, Notice
impl Machine { pub fn step(&mut self, input: Input, view: &View) -> Vec<Effect>; }   // Phase 1 has Idle and Panning only
```

`BoardView` is defined in `wren-canvas` and owned by `App`, as `EditorView` is defined in `wren-editor`. It holds the committed model, the pristine pair, the synced text, the scene, the viewport, the machine and, from Phase 2, the history, the in-flight plan and the open label's text. None of that lives in widget tree state, so the harness drives and reads it with no window and a rebuilt widget tree loses nothing. Phase 1 ships `Message::Input(epoch, Input)` and the `Machine::step` signature with wheel and pan inputs only; Phase 2 extends both and replaces neither.

### 3.3 The surface seam in the app

- `App` keeps `editor: EditorView` and gains `board: Option<BoardView>`. `app/board.rs` derives `Surface { Text, Board, Raw }`. The view mounts exactly one of the two widgets under the focus id `note-editor` (hard-coded today at `wren-editor/src/lib.rs:268`; mounted at `app/workspace_view.rs:896-900`), so `focus_editor` (`app.rs:813`) and `column_focus` (`app/columns.rs:121-127`) work unchanged. `Message::Board(wren_canvas::Message)` sits beside `Message::Editor` (`app.rs:122`).
- **Deciding the surface.** `sync_editor` (`app/workspace.rs:777-793`) decides it when the note identity changes or when the Session text differs from what the surface last took (`board.synced()` on Board, the editor's own text on Raw): `BoardView::open` or `reload` succeeding gives Board and empties the editor (`replace_text("", false)`); a parse error gives Raw, with the text in the editor as for Markdown and a banner naming the line. `sync_editor` runs only when an op produced events or an error (`app/workspace.rs:376-378`), so an app test asserts that every Session-driven text change emits an event.
- **Raw to Board.** A raw board flips back when the note is next opened and on a Session-driven text change (an external edit that now parses). It never flips while the user types in it. The banner says the board will open as a board again once the text is valid.
- **Only the visible surface may write.** `ensure_editor_ready` (`app/workspace.rs:857`) fails on Board. `command_enabled` (`app/commands.rs:480`) is kind-aware: Editor-scope commands and Outline need Text, Find needs Text or Raw (until Phase 3 routes both to the jump palette on a board, §5.18), a Board command needs Board, a mutating one a writable board. `text_changed` (`app.rs:595`) returns early on Board. `apply_editor_engine` (`app/preferences.rs:285-296`) passes `""` on Board. `step_text_size` (`:368`) zooms the board and writes no setting. `reveal_literal` runs only on Text or Raw (`app/palette.rs:1049`). Markdown assist is off while Raw. Without this, palette "Bold" with a board open would overwrite the board with the hidden editor's text. The A4 item records a table of every `self.editor` site in the app (found with `grep`; 46 outside `test_requests.rs`) classed as safe on Board, guarded, or unreachable.
- **The setting.** `boards = on | off` in `config.conf`, parsed beside `markdown-assist` (`settings.rs:240`), read at launch into `SessionOptions { boards }`; a changed value shows "restart Wren to apply `boards`". With it off no board is scanned, listed, indexed or opened, no board command or pill is listed, and the state file is byte-identical to today's.
- **Footer and issues.** The board footer (beside `editor_footer`, `app/workspace_view.rs:984`) shows node and connector counts, the zoom, "read-only" for `Access::ReadOnly`, and the count of integrity issues. A flagged element carries a marker drawn by the scene in the `issue` colour, and `board.get` lists the issues. There is no issue list in the UI in the MVP.
- **Viewport.** The app keeps the viewport of each board opened in this run, in memory, by note id. A board opens fitted the first time (§5.13 rule 4) and returns to where it was after a note switch or a reload. Nothing about the view is stored on disk.

### 3.4 The Session contract

1. **One writer.** From Phase 2, `App::board_changed()` is the only place a board sends `Op::EditDraft`: it takes `board.draft()` and bumps the revision. Keys, pointer, palette, menu and harness all reach it through `App::board_edit(message)` (the twin of `App::edit`, `app.rs:560`, with the same dialog and palette guard, a writable-board check and the epoch check) or through `dispatch(Command::Run(..))`. In Phase 1 the board sends no draft at all.
2. **Pristine rule.** `BoardView` keeps `pristine = (text, canonical)` where `canonical = write(&model)` at load, at `reload` and at `saved`. `draft()` computes `out = write(&model)`; if `out == pristine.canonical` it returns the pristine text verbatim, otherwise `out`. `Session` marks a draft clean when its bytes equal the baseline (`ops.rs:782-788`), so undoing back to the loaded state makes the document clean again. Nothing is written on open, and a hand-edited non-canonical file stays byte-identical until its first surviving edit, which rewrites it in canonical form once. `draft()` records what it returns as `synced` before the op runs, so the `sync_editor` call that follows the app's own draft sees equal texts and does nothing.
3. **Settle before any non-passive op.** `App::settle_board()` calls `board.settle(now)`, which commits an open label (§5.9 rule 4), cancels a pointer gesture and keyboard Connecting, and drops an actions-palette preview; then `board_changed()` if anything changed. Call sites: the top of `process_core` when the op is not passive (`app/workspace.rs:296-297`: `Tick`, `EditDraft` and `FinishReconcile` are passive), `open_palette` (`app/palette.rs:301`), `show_dialog` (`app/workspace.rs:900`), `window_unfocused` (`:867`). The draft it sends is passive, so it does not recurse. If the following flush fails, the transition is blocked as today, with the label committed and the draft kept.
4. **What each moment does.**

| Moment | Committed model | `EditDraft` | Undo |
|---|---|---|---|
| Press, move under 4 px, hover, selection change, walk, pan, zoom, arming a tool | unchanged | none | none |
| Move past 4 px (Moving, Resizing, Creating, Marquee, Connecting) | unchanged; the preview is the plan so far on a scratch copy, rebuilt at most once per pointer move | none | none |
| Release | `plan` then `apply`, once | one | one step |
| Esc, blur, `settle` or `reload` during a pointer gesture | unchanged | none | none |
| A model command (move, delete, duplicate, paste, reshape, connect, style, arrange) | `plan` then `apply` | one, none if nothing changed | one step; a run of nudges coalesces (§5.11 rule 3) |
| A creation that opens the label editor | applied at once | one | opens a group |
| Each label text change | text, and `size` if it grew | one per change | joins the open group, or opens one |
| Label commit | trailing blank lines trimmed | one only if that changed the text | closes the group: create plus label is one step |
| `board.undo`, `board.redo` | replaced by the snapshot | one | moves the stack |

5. **External change.** Reconcile is passive, so nothing is settled first, by design. Clean document: the Session reloads and `sync_editor` calls `reload`. Dirty document: the Session writes the draft (a label typed in the last 750 ms included; `autosave.rs:11-12`) to a conflict copy named for the kind, reloads the original, and the same `reload` runs. `reload` parses first and changes nothing if that fails (the surface goes Raw). Otherwise it resets the machine to Idle (gesture cancelled, label editor closed without a commit), clears the history, drops selected ids that no longer exist, keeps the viewport, sets pristine and bumps the epoch. A pending label is not re-applied: when it mattered it is in the conflict copy. Messages stamped with an older epoch are dropped, as `drag_event` drops a stale generation (`app/gestures.rs:36`). `process_events` (`app/workspace.rs:748`) forwards `NoteSaved` to `board.saved(text)`. Element-level merge is not in the MVP.
6. **Undo** is whole-model snapshots with the selection and cursor, held by `BoardView` beside the editor, never in widget state: 200 entries and a byte budget, as the editor's (`wren-editor/src/history.rs:16-23`), with time injected. It is dropped when another note opens and on any Session-driven text change.
7. **Cost per draft.** `Session::apply_at` re-indexes the open document after every op (`ops.rs:485-486`), guarded only by text equality (`index.rs:390-399`), so every board draft pays a canonical write plus an index parse and label extraction. Phase 2 measures that sum per label keystroke at 1,000 nodes against an 8 ms release budget (§6.7). If it misses, the index derives the open document's searchable text lazily, on the next query, and the measurement is repeated.

Not negotiable in any phase: a write on open; a second writer to the Session; a board path in a v1 state key (§3.5); `toml::to_string_pretty` for boards; any change to schema 1 output after the lock.

### 3.5 Note kind and the state file in `wren-core`

- `kind.rs`: `NoteKind { Markdown, Board }` with `ALL`, `from_name(&OsStr)`, `extension()`, `title(&OsStr)`, `template()`. Kind is a case-insensitive suffix match on the whole file name with a non-empty stem; the title is the name minus that suffix (`x.md.wren-board` is a board titled `x.md`; `x.wren-board.md` is Markdown). It replaces `scan::is_markdown_name` (`scan.rs:688`) and `model::note_title` (`model.rs:224`), and `Note` (`model.rs:59`) gains `kind`. The board template is the literal `"schema = 1\n"`, with a test that it equals `wren_board::EMPTY` once the dependency exists.
- `fs_store.rs`: `enum Leaf { Directory, Note(NoteKind) }` replaces `note: bool` in `relocate` (`:616`), `sanitized_name` (`:988`) and `candidate_name` (`:1023`); `create_file` (`:504`) takes a kind. A leaf keeps its kind's extension through create, rename and move; a rename never changes kind.
- `ops.rs`: `Op::CreateNote { group, title, kind }` (`:25`, `:561-585`; 21 Rust use sites and one harness decoder, `test_requests.rs:846`, which defaults to Markdown); `Session::relocate` (`:1377`) takes the leaf from the source path's kind; `create_conflict` (`:1138`) names the copy `Flow (conflict …).wren-board`. `SessionOptions { boards }` reaches the three scans (`ops.rs:270`, `:1498`, `reconcile.rs:94` through `ReconcileJob::new`, `:60`) as a `scan::Kinds` value, and a Session made by `Op::OpenWorkspace` inherits it as it inherits `background_reconcile` (`ops.rs:741-747`).
- **Same title in both kinds.** `Flow.md` and `Flow.wren-board` may share a group: collisions are decided by the full file name, as today. The notes list tells them apart by the kind icon, shown on board rows only. A rename collides only with a file of the same full name.
- **The state-file rule.** Released Wren (0.1.0, 0.1.1) rejects the whole `.wren/workspace.toml` if `history`, `recent` or `last_open_note` names a non-`.md` path (`validate`, `workspace_state.rs:568-656`). So:
  1. The v1 keys `history`, `recent`, `last_open_note` and `history_position` hold Markdown paths only. They are written as the Markdown projection of the real lists: boards filtered out, `recent` still unique, `history_position` re-pointed at the last Markdown entry at or before the real position and removed if there is none (`:604-620` states the two invariants). When the open note is a board, `last_open_note` is left as it was.
  2. `selection` is already `Reference::Any` (`:111`) and may name a board; `load` tries it first (`ops.rs:347-357`), so a board reopens.
  3. A new table `[navigation]` (`history`, `history_position`, `recent`, `last_open`) holds the mixed-kind lists. It is written only when one of them names a non-Markdown path and removed otherwise, so a workspace with no boards, and every user with the setting off, gets a state file byte-identical to today's.
  4. `[navigation]` is read leniently: a bad entry is skipped, a wrong-typed value ignored, and nothing in it can call `disable` (`:336`). It is trusted only if its Markdown projection equals the v1 keys; otherwise an older build has navigated since and the v1 keys win.
  5. `transform_paths` (`:373`) remaps and prunes `[navigation]` by the same rename and delete rules.

### 3.6 Search

`index::Content` (`index.rs:73`) keeps the raw source for change detection and the extracted text for searching: `source: Arc<str>` beside the searchable `text`; `set_content` (`:390-399`) compares `source` first; `content_snapshot` (`:347`) returns `source`. A private `searchable(kind, source)` returns `None` for Markdown (search the source itself: one shared allocation, byte for byte what it was) and, for a board, `wren_board::search_text` (node labels in reading order, top then left by absolute position, then connector labels, one per line). An unparseable board, and from Phase 3 one over the limits, indexes its raw text. Board previews and snippets skip `body_line` (`:557`) and `plain_markdown` (`:754`); the notes-list preview is the first label. A hit reveals the first matching label (§5.18 rule 4).

### 3.7 Scene rules and constants

The scene is the single source of what a board looks like: the widget, the SVG and the PNG all come from `scene::build`. The values below are the starting values. Items M4 and M5 may change them, must record the final values here, and must do so before any scene or SVG golden file is blessed; after Phase 1 a change to any of them is a visible change to every exported board.

| Constant | Value |
|---|---|
| Label text | Inter Regular (bundled, `wren-iced/src/fonts.rs:46-50`), 14 px, line height 20 px, at fixed logical sizes independent of editor font settings |
| Connector label, placeholder caption | Inter Regular 12 px, line height 16 px; a connector label sits on a canvas-colour rectangle padded 4 px by 2 px |
| Label box by shape (inset from the node box) | rect, rounded: 8 px sides, 4 px top and bottom. pill: `h/4` sides, 4 px. ellipse: 15 % each side. diamond: `w/6` sides, `h/6` top and bottom. parallelogram: the skew plus 4 px sides, 4 px. cylinder: 8 px sides, the cap height on top, 4 px below. document: 8 px sides, 4 px top, the wave height below |
| Label placement | Wrapped at the label box width with the measurer, a grapheme break inside an over-long word; centred both ways in a shape. Text that overflows the box is drawn in full, never clipped or cut with an ellipsis; the stored `size` is never changed by drawing |
| `kind = "text"` | No outline and no fill; the wrap box is the node box with no inset; left-aligned, top-aligned, in the node's colour |
| Stroke | 1.5 px for outlines and connectors; dashes 6 on, 4 off |
| Shape parameters | rounded: radius 8. pill: radius `h/2`. parallelogram: skew `min(w/4, h/3)`. cylinder: cap half-height `min(h/6, 12)`. document: wave amplitude `min(h/8, 8)` |
| Arrowhead | A filled triangle 10 long and 8 wide in the line colour, the line ending at its base |
| Unknown kind | A dashed rectangle in `muted` with the kind name as a caption at the top left and the label centred |
| Dangling end | A 24 px stub from the valid end on its nominal side, ending in an 8 px cross stroke, in `issue`; an issue marker is a 6 px dot in `issue` at the element's top-left corner |
| Colour resolution | `neutral` uses `ink` for stroke and label. `none`: stroke only. `tint`: fill is the hue (for `neutral`, `ink`) mixed into `canvas` at 16 % in a dark theme and 12 % in a light one; label in `ink`. `solid`: fill is the hue (for `neutral`, `raised`); the label is whichever of `ink` and `canvas` has the higher contrast on it. A literal `#rrggbb` is used as the hue unchanged. An unknown colour draws as `neutral` |
| Out-of-range values | Geometry beyond ±16,777,216, a size below 1, an anchor or label position outside 0 to 1000 are drawn clamped; the stored values are untouched (§4.12) |

Other scene rules: the parent transform is applied from the first version (nested, through an unknown kind; a node in a cycle is laid out as a root); order is `z` bytewise, ties by id, among siblings, children directly above their parent (§4.7); a floating connector end is drawn on its nominal side whenever the two nodes overlap on one axis and are apart on the other (§5.12 rule 4); routes are computed and never stored; there is no obstacle avoidance. The default node is 120 by 60 (§5.5 rule 2). A Phase 1 test under the real measurer (item V2) asserts that each of the fifteen labels of the acceptance script (§5.20) fits the label box of the shape the script gives it without growing the node, and that `Notify customer` fits every shape but the diamond. If it fails, the label box or the font size changes first and the default size last, before the fixtures and the Phase 2 golden layout are written.

### 3.8 Limits for untrusted files

Loading never writes and never panics, from Phase 1: no indexing or `unwrap` on file-derived data, checked integer conversions, parent sums in `i64`, the `toml` crate's nesting limit of 80, parent walks bounded by a visited set. The caps of §4.12 arrive in Phase 3: above 8 MiB or 50,000 elements a board opens as raw text and is never parsed by the index; out-of-range values are flagged and drawn clamped. The general 64 MiB rule is unchanged (`fs_store.rs:27`). SVG and PNG export escape text and attributes, strip the characters XML cannot carry, emit a literal colour only if it matches `#rrggbb`, and never emit `link` as an `href`. PNG export refuses more than 64 megapixels and resolves no image or external reference. Ring 0 reads no `file`, `src` or `link`, so the rule that references resolve only through `RelPath` and `FsStore`, and the attachments folder (`Section/Group/assets/`, content-hashed names), apply from ring 1.

## 4. File format, schema 1

This section is the contract for `read.rs`, `write.rs` and `check.rs` and the source of `docs/reference/board-format.md`. The owner rarely reads the file and Wren adds no git features, so each open point was settled toward the simplest rule that keeps the file deterministic with one field per line. A reference writer for these rules (`board_format.py`) and a check of the `toml` crate's behaviour are in `.claude/board-notes/plan/scratch/`; step 0 copies the reference writer into the repository (§7.1).

### 4.1 File

- The file is `Name.wren-board`. Kind comes from the extension and the title from the file name; nothing inside names it. It is UTF-8, no BOM, LF line endings, exactly one LF at the end. Wren writes TOML 1.0 syntax only, in the canonical form of §4.11, and reads any valid TOML of the same shape (§4.12).
- Top-level keys: `schema` (integer, required, `1`), `requires` (array of strings, optional, §4.10), `node` (table of id to node record), `edge` (table of id to edge record). Any other top-level key is unknown content and is kept; `layout` and `ext` are reserved names.
- A file with no keys at all (empty, blank, or comments only) is an empty schema-1 board. The new-board template is `schema = 1` and a line feed.
- Comments are accepted and, like key order and spacing, are lost at the first edit. An annotation that must survive goes in an `ext.*` field.

### 4.2 Ids

- An id matches `[A-Za-z0-9_-]{1,32}`, is case-sensitive, and is compared bytewise. It is the last part of the block header, always a bare key.
- Node ids are unique among nodes and edge ids among edges; the TOML parser enforces both. A node and an edge sharing an id is flagged (§4.12). `parent`, `from.node` and `to.node` always name nodes.
- Generated ids are 8 characters from `0123456789abcdefghjkmnpqrstvwxyz` (40 bits), redrawn until unused in the board and in the plan being built. `std::hash::RandomState` supplies the bits; tests inject a seeded source. Hand-chosen ids such as `start` are equally valid.

### 4.3 Node record: `[node.<id>]`

| Field | Type | Presence | Default | Values and meaning |
|---|---|---|---|---|
| `text` | string | optional | `""` | The label: plain text, LF line breaks (§4.9). |
| `kind` | string | required | | `shape` or `text` (free text, no outline). Reserved: `frame`, `note`, `image`. Any other value is kept and drawn as a labelled placeholder in the node's box. |
| `at` | `[int, int]` | required | | `[x, y]` of the top-left corner (§4.6). |
| `parent` | string | optional | none | Id of the containing node; `at` is then relative to it. Ring 0 honours it and never creates it. |
| `shape` | string | optional | `rect` | For `kind = "shape"`: `rect`, `rounded`, `pill`, `ellipse`, `diamond`, `parallelogram`, `cylinder`, `document`. Any other value draws as `rect` and is kept. |
| `size` | `[int, int]` | required | | `[w, h]`, each at least 1. Authoritative: never re-measured on load. For `text` it is the wrap box. |
| `style.color` | string | optional | `neutral` | Colour token or `#rrggbb` (§4.8). |
| `style.fill` | string | optional | `tint` | `none`, `tint`, `solid` (§4.8). Not used by `text`. |
| `style.stroke` | string | optional | `solid` | Outline: `solid` or `dashed`. |
| `z` | string | required | | Stacking key (§4.7). |
| `ext.<tool>.*` | table | optional | | Foreign data, namespaced by tool. Any TOML value. Never interpreted. |

### 4.4 Edge record: `[edge.<id>]`

| Field | Type | Presence | Default | Values and meaning |
|---|---|---|---|---|
| `kind` | string | required | | `edge`. Any other value is kept and drawn as a plain connector. |
| `from`, `to` | table | required | | An endpoint, one of the four forms below. |
| `heads` | `[string, string]` | optional | `["none", "arrow"]` | Marker at the `from` end, then at the `to` end: `none` or `arrow`. Any other value draws as `arrow`. |
| `labels` | array of tables | optional | `[]` | Each item is `{ text = string }` with an optional `at`, an integer 0 to 1000 (default 500): the position per-mille along the route from the `from` end. Ring 0 edits the first; all are drawn. |
| `route` | string | optional | `elbow` | `straight` or `elbow` (right-angled). Reserved: `curved`. Any other value draws as `straight`. The path is computed from the endpoints and never stored. |
| `style.color` | string | optional | `neutral` | As for nodes; colours the line, heads and labels. |
| `style.stroke` | string | optional | `solid` | `solid` or `dashed`. |
| `z` | string | required | | Stacking key (§4.7). |
| `ext.<tool>.*` | table | optional | | As for nodes. |

Endpoint forms; exactly one of `node` and `at` is present:

| Form | Meaning |
|---|---|
| `{ node = "id" }` | Attached to the node; the router picks the side. |
| `{ node = "id", side = "right" }` | Attached at the middle of `top`, `right`, `bottom` or `left`. An unknown side reads as none. |
| `{ anchor = [fx, fy], node = "id" }` | Attached at a point of the node's box, per-mille from its top-left, each 0 to 1000: `[500, 0]` is top centre. `anchor` wins over `side`. |
| `{ at = [x, y] }` | A free end at absolute board coordinates. |

### 4.5 Reserved for ring 1

Schema-1 code keeps all of these as unknown content (§4.10) and interprets none of them except `parent`. Each is a new key or a new value of an existing string field, so adding it later is not a format change. Their types are fixed now.

| Name | On | Type | Meaning |
|---|---|---|---|
| `kind = "frame"` | node | | A container; children point at it with `parent`; `text` is its title. |
| `kind = "note"`, `file`, `subpath` | node | strings | A note card. `file` is relative to the board's folder; `subpath` is `#Heading`. |
| `kind = "image"`, `src` | node | string | An image; `src` is relative to the board's folder (under `assets/`). |
| `locked` | node, edge | boolean | Default `false`. No effect in ring 0. |
| `link` | node | string | A URL or a path relative to the board's folder. |
| `rot` | node | integer | Degrees clockwise about the centre. Default 0. |
| `markup` | node | string | Label format (`"md"`). Absent means plain text, so labels written today never change. |
| `points` | edge | array of `[x, y]` | Hand-placed waypoints, absolute. |
| `layout` | top level | table | `direction`, for Tidy and import. |

### 4.6 Coordinates and units

- The unit is one logical pixel at zoom 1; x grows right and y grows down. The 20-unit grid (`wren_board::GRID`) is an editing aid, never stored, and off-grid values are valid.
- `at` is the top-left corner of the node's box, relative to the parent's top-left corner when `parent` is set, otherwise absolute. Free ends and `points` are absolute.
- Fractions are integers per-mille (0 to 1000): `anchor` and a label's `at`. Every number in a known field is therefore an integer; floats occur only in unknown content. That removes float formatting from the known fields.
- An integer must fit `i32`, or the file is an error. A resolved coordinate or size beyond ±16,777,216 (2^24, the range `f32` draws exactly) is flagged and drawn clamped.

### 4.7 Stacking order: z keys

- `z` is 1 to 64 characters of `[0-9A-Za-z]`. Elements that share a parent are drawn in ascending bytewise order of `z` (digits, upper case, lower case), then of id, a node before an edge. Root nodes and all edges form one scope. A node's children are drawn directly above it, in their own order. A missing `z` sorts as the empty string.
- Keys are opaque: a hand-written `a1` is as good as a generated one. Wren never generates a key ending in `0`, because no key sorts between `X` and `X0`.
- Read a character as a digit 0 to 61 in that order. `key_between(lo, hi)` gives the key for a slot between two neighbours, either of which may be absent:
  1. No neighbours: `V`.
  2. Nothing above: take the first three characters of `lo`, padded on the right with `0`, as a base-62 number `n`. Add 1, and 1 more if the result is a multiple of 62. If `n < 62^3` the key is `n` as three characters; otherwise `bisect(lo, none)`.
  3. Nothing below: the same from `hi`, subtracting 1, and 1 more if the result is a multiple of 62. If `n >= 1` the key is `n` as three characters; otherwise `bisect("", hi)`.
  4. Both present: `bisect(lo, hi)`.
- `bisect(lo, hi)`: if `hi` is `lo` followed by zero or more `0`s, fail (any key between them would end in `0`). Otherwise build the key one character at a time. At position `i`, let `a` be the digit of `lo` (0 past its end) and `b` the digit of `hi` (62 when `hi` is none). `a = b`: append it and go on. `b - a >= 2`: append `(a + b) / 2` rounded down and stop. `b = a + 1` and `hi` has more characters: append `b` and stop. `b = a + 1` otherwise: append `a`, treat `hi` as none from here on, and go on.
- **The renumber fallback.** When `bisect` fails, the key would pass 64 characters, or a neighbour's key is not in the alphabet, the command first renumbers the whole scope in the same undo step and then takes its key: the scope's elements, in drawing order, get the three-character keys for `119165 + 2i` (`V01`, `V03`, …; room for 59,581). This is the one case in which a command rewrites the `z` line of elements it did not move.
- Phase 1 only sorts by `z`; generating keys is Phase 2 work. A new element and "bring to front" take `key_between(top, none)`; "send to back" takes `key_between(none, bottom)`; one step forward or backward takes the slot beyond the neighbour being passed. A multi-selection is processed in drawing order, so it keeps its internal order. Each moved element changes one line. Nothing is re-keyed on load or save.

### 4.8 Colour and fill

- `style.color` is a token: `neutral`, `red`, `orange`, `yellow`, `green`, `cyan`, `purple`, `blue`. Each theme supplies the seven hues; the file never changes with the theme. The six hues after `neutral` are JSON Canvas presets 1 to 6, in order.
- A literal `#rrggbb` is also valid, for imports. It is read case-insensitively, written lower case, and never adjusted per theme. Ring-0 commands write tokens only. Any other string is kept and drawn as `neutral`.
- `style.fill`: `none` draws the outline only; `tint` mixes the hue into the canvas colour; `solid` fills with the hue. Any other value is kept and drawn as `tint`. The label colour is derived for contrast and never stored (§3.7).

### 4.9 Strings, labels and numbers

**Every string is written as a single-line basic string; a line break in a label is `\n`.** Every field is then one physical line with no exception, so "a one-field edit changes one line" holds for labels too. The cost is that a long multi-line label is one long line in a diff: still one tracked, readable change.

- Escapes: `\\`, `\"`, `\n`, `\t`, `\r`, and `\uXXXX` (four upper-case hex digits) for every other character from U+0000 to U+001F and for U+007F. Everything else is written raw, with no Unicode normalisation. The loader accepts every TOML string form (`"""`, `'…'`, `'''`).
- Label text (a node's `text`, a label item's `text`) is plain text, never Markdown. On load, CRLF and a lone CR inside it become LF; nothing is trimmed. Other control characters are kept, and export strips the ones XML cannot carry.
- Integers are decimal, with `-` as the only sign, no leading zeros and no separators. Booleans are `true` and `false`. Floats, in unknown content only, are written as Rust formats `f64` with `{:?}` (`1.0`, `0.25`, `-0.0`, `1e21`), and `nan`, `inf`, `-inf`. The type read is the type written: `1` stays an integer and `1.0` a float. Dates and times use the crate's `Display`.

### 4.10 Unknown content and `requires`

- Kept through load and save, with value and type: unknown top-level keys; unknown keys in a node, an edge, `style`, an endpoint or a label item; everything under `ext`; and unknown values of the string fields (`kind`, `shape`, colour, fill, stroke, `route`, `side`, heads). They draw as the tables say and are never an error and never flagged.
- An element of unknown kind keeps all its fields. Its envelope is still read and must be well-typed: every node has a box (`at`, `size`) and every edge two endpoints, whatever the kind, so a placeholder sits in the right place and the parent transform passes through it. A known field with the wrong type is an error, not unknown content. TOML has no null, so importers keep nulls out of `ext`.
- `requires` lists feature names (`[a-z0-9-]{1,32}`), written sorted and omitted when empty. Ring 0 defines none. A board naming a feature the build does not know opens **read-only**: it draws, pans, zooms, selects, walks, copies, jumps and exports (app and CLI), and never sends a draft. A `schema` above 1 that still loads under these rules is read-only in the same way.
- Rule for later rings: a feature adds its name while a board uses it only if a build without the feature could write a wrong file or show a misleading picture. `rot` would; `frame` would not, because `parent` is already honoured.

### 4.11 Canonical writer

`write(&Board) -> String` is hand-written. `toml::to_string_pretty` is never used: it explodes arrays and emits `[node.x.style]` and `[[edge.x.labels]]`.

1. Bytes: UTF-8, no BOM, LF, no tabs, no trailing spaces, no comments, exactly one LF at the end.
2. Line 1 is `schema = 1`. Then `requires = ["a", "b"]` if it is not empty. Then unknown top-level keys in bytewise order, by rules 5 to 8.
3. Then one block per node and then one per edge, each group in bytewise id order. A block is one empty line, the header `[node.<id>]` or `[edge.<id>]`, then its fields. There are no other empty lines.
4. Field order in a node: `text`, `kind`, then every other key bytewise. In an edge: `kind`, `from`, `to`, then every other key bytewise. The order never depends on whether the build knows a key, so builds of different ages write the same bytes.
5. One field per line, `key = value`. A key is bare if it matches `[A-Za-z0-9_-]+` and a basic string otherwise.
6. A table is written as dotted keys, one leaf per line (`style.color = "blue"`), sub-keys in bytewise order inside their own table, recursively. Order is by key, not by the joined text: `style.color` comes before a sibling key `style-x`. Three cases are one-line inline tables, `{ k = v, k = v }` with keys in bytewise order: an edge's `from` and `to`, a table inside an array, and an empty table (`{}`).
7. An array is one line, `[a, b]`, nested arrays included. A field whose array holds a table is written one item per line: `key = [`, each item indented two spaces with a trailing comma, then `]`. Inside an inline table or another array everything stays on one line.
8. Scalars as in §4.9.
9. A known optional field equal to its default is omitted: empty `text`, `shape = "rect"`, `style.color = "neutral"`, `style.fill = "tint"`, `style.stroke = "solid"`, `route = "elbow"`, `heads = ["none", "arrow"]`, empty `labels`, a label's `at = 500`, empty `requires`. A missing `kind` or `z` stays missing. Unknown content is written as read.
10. The output is a function of the model alone: not of source order or layout, platform, locale or map iteration order. `write(parse(write(m))) == write(m)`.

### 4.12 Loader

`parse` then `check`. The loader parses with `text.parse::<toml::Table>()`, as `config.rs:111` and `workspace_state.rs:50` do, without normalising the text first. That accepts any key and block order, comments, CRLF line endings, a BOM, sub-table and array-of-table layouts, every string form, and the TOML 1.1 syntax the crate allows. A clean hand merge in git can leave blocks out of order; that is valid input. It then builds the model, with known fields typed and everything else kept as `toml::Value`.

**Errors.** The board opens as raw text with the message and a line. Parser errors carry a span; for the others the loader re-parses with `toml::de::DeTable::parse`, which keeps spans, on the error path only.
- Not valid TOML, including a duplicate key or header and nesting deeper than 80.
- `schema` missing from a file that has other keys, not an integer, or below 1. `requires` not an array of strings.
- `node` or `edge` is not a table, or one of its entries is not a table. An id that does not match §4.2.
- A known field of the wrong type: a float, `nan`, `inf` or a string where an integer is required, an integer outside `i32`, `at` without exactly two items, a label item without `text`. (`nan` and `inf` are legal in unknown content.)
- A node without `at` or `size`. An edge without `from` or `to`, or with an endpoint holding both or neither of `node` and `at`.
- Phase 3: a file over 8 MiB or with more than 50,000 elements.

**Flagged.** The board opens and is editable; the issue is listed (§3.3); the content is kept byte for byte through a save; nothing is repaired without a user action on that element.

| Issue | Drawn as |
|---|---|
| An endpoint names no node | A short stub from the other end; nothing if both ends dangle. |
| `parent` names no node | A root node, its stored `at` taken as absolute. |
| A parent cycle, or a chain deeper than 32 | Every node in it as a root node. |
| A node and an edge share an id | Both, normally. |
| Missing `kind` | A node as a placeholder; an edge as a plain connector. |
| Missing `z`, or a `z` outside §4.7's pattern | In bytewise order of what is there. |
| Out of range: resolved geometry (§4.6), a size below 1, an `anchor` or label `at` outside 0 to 1000, a label over 16 KiB, more than 16 labels, a `z` over 64 characters | Clamped or truncated, from Phase 1. Listed as issues from Phase 3. |

**A missing `at` or `size` is an error in schema 1**, not a flag: such a node has no geometry, and inventing one would either be written back (a repair nobody asked for) or create a second, unsaved geometry. When import arrives the reader can be relaxed to treat a missing `at` as "unplaced"; that is additive, because a schema-1 build refuses such a file and cannot misread it. The split: what can still be drawn where it is, is flagged; what has no geometry is an error. One trap for hand and LLM authors: a field written after a `[node.a.style]` header lands inside `style`. If the stray field is `at` or `size` the node fails as missing it, with the line; any other field is kept as an unknown style key.

### 4.13 The pristine rule and canonical form

- On load, on external reload and after a save, the board keeps the pristine text and `write(&model)` of the model parsed from it. Model equality is equality of those canonical bytes (§3.4 rule 2). It needs no `PartialEq` over `toml::Value`, where `nan != nan` would make a board with a `nan` in unknown content never equal to itself. Round-trip tests compare canonical bytes too.
- Canonical form is never used to decide whether a file on disk is clean, and nothing is normalised on open. Two builds whose writers differ never rewrite each other's untouched files.
- A read-only board never sends a draft, and the raw-text fallback saves exactly what the user types. The clipboard fragment goes through the same writer.

### 4.14 Example

The full example (9 nodes, 9 edges, every endpoint form, 147 lines) is committed in step 0 as `crates/wren-board/golden/format/password-reset.out.wren-board`. Two of its blocks:

```toml
schema = 1

[node.ctrj33th]
text = "Send reset link\n(expires in 30 min)"
kind = "shape"
at = [160, 400]
shape = "rounded"
size = [160, 80]
style.color = "purple"
z = "V04"

[edge.jppm92yk]
kind = "edge"
from = { node = "k5f7n16m", side = "left" }
to = { node = "av4c4ntg", side = "left" }
labels = [
  { at = 250, text = "no" },
]
style.color = "red"
z = "V0C"
```

### 4.15 The `toml` crate, checked against the pin

The workspace pins `toml = "=1.1.8"` (`Cargo.toml:38`) with default features. Confirmed in the crate source and by a scratch crate:
- `toml::Table` is a `BTreeMap` unless `preserve_order` is on (`src/map.rs:37`), so source order, comments and the spelling of tables are gone after parsing. Feature unification could turn `preserve_order` on, so the writer sorts explicitly and never relies on iteration order.
- `Value` keeps `Integer(i64)` and `Float(f64)` apart and has `Datetime`. A duplicate key or header fails with `duplicate key` and a span; `DeTable::parse` (`src/de/parser/detable.rs:20`) gives a span for any key. Nesting is capped at 80 unless the `unbounded` feature is on (`src/de/parser/mod.rs:71`).
- It accepts a BOM, TOML 1.1 syntax, `nan` and `inf`; Python's `tomllib` rejects the first two, and the writer never emits them. A raw CRLF inside a multi-line string is kept as `\r\n` (the reason for the label rule in §4.9). `{:?}` of a finite `f64` is valid TOML and reads back to the same bits; `{}` prints `1.0` as `1`, which would read back as an integer.
- Parsing 20,000 elements (2.9 MiB) takes 80 to 110 ms in release; 200,000 random byte mutations of a valid file caused no panic.

### 4.16 Tests for the format code

Inline in `wren-board`; files under `crates/wren-board/golden/format/`, read with `include_str!`. `WREN_BLESS=1 cargo test -p wren-board` rewrites golden files, and a blessed change is reviewed as a diff. The `.out` files created in step 0 come from the reference writer, so the Rust writer must reproduce a second implementation byte for byte; blessing alone only proves it agrees with itself. Default-run counts are sized so `cargo test -p wren-board` stays under about 10 s in a debug build (§6.6); the full counts in brackets are `#[ignore]` tests run in release by `make fuzz-board`.

1. **Canonical golden files** [P1]. For each `<name>.out.wren-board`, `write(parse(f)) == f`. Cases: the empty board; the example; every node and edge field at a non-default value; every shape and endpoint form; all escapes of §4.9; unknown kind, shape, colour, fields, `ext` and top-level keys; quoted keys; floats, `nan`, dates and nested tables in unknown content; `requires`; and the canonical files of the sample workspace, included by relative path.
2. **Non-canonical inputs** [P1]. `<name>.in.wren-board` writes to exactly `<name>.out.wren-board`: CRLF, a BOM, comments, shuffled blocks and keys, sub-table and `[[…]]` layouts, `"""` and literal strings, TOML 1.1 syntax, explicit defaults, an empty file.
3. **Round trip** [P1]. 500 random valid models [20,000]: `write(parse(write(m))) == write(m)`. 2,000 generated strings [20,000] and the float set survive `write` then `parse`, and no written line holds a raw control character.
4. **One line per edit** [P1, at model level by mutating the model directly; P2 repeats it through intents]. Move, resize, relabel (multi-line included), recolour and restack each change exactly one line; adding an element adds exactly one block, and deleting one removes it.
5. **TOML 1.0 outside Rust** [P1]. A unit in `tools/wrentest_unit/` parses every `.out` file with Python's `tomllib` (`requires-python >= 3.12`) and, for the files the reference writer can represent, checks that it writes the same bytes.
6. **Mixed version** [P1]. `future.out.wren-board`, with the fields of §4.5, an unknown kind, unknown enum values, and unknown keys that sort before, between and after the known ones at every level, round-trips byte for byte, and an edit to one known field changes one line. A second case checks the key order of every block against a plain sort computed in the test, with no reference to which keys the code knows.
7. **Every error rule** of §4.12 [P1], table-driven: input, message fragment, line. With them: an unknown `requires` entry and `schema = 2` give read-only; `schema = 0`, `schema = "1"` and a missing `schema` beside other keys are errors.
8. **Every flagged rule** of §4.12 [P1]: the issue is reported, the board draws as the table says (resolved geometry for a missing parent, a self-parent, cycles of two and three, a chain of 33; a child's position through an unknown kind), and `write` returns the input bytes.
9. **Never panic** [P1]. 2,000 seeded byte and line mutations of the golden files [100,000], and random `toml::Table` values with a wrong type in each known field: `parse` returns, and whatever loads also checks and writes. Depth 81 is an error, which guards the `unbounded` feature.
10. **Limits** [P3]. Each cap at its value and one past it: file size, element count, coordinate range, parent depth, label size, label count, z length.
11. **z keys** [P2]. The properties of §4.7 over 20,000 random and hand-style pairs [400,000]; 10,000 appends and prepends at three characters; same-gap inserts until the renumber; the renumber keeps the order; sort ties fall to the id.
12. **Pristine** [P2]. A non-canonical fixture sends nothing on open; an edit followed by its undo sends the original bytes; an edit sends canonical bytes; a board with `nan` in `ext` behaves the same.

### 4.17 The schema lock (end of Phase 2)

One commit, after the phase-end review and the owner's confirmation of the three things that cannot be undone: single-line labels (§4.9), per-mille fractions (§4.6) and the colour vocabulary (§4.8). Before it, the candidate palettes for the eleven themes are run through the colour-blindness script (`.claude/board-notes/research/critic/cvd.py`) as a short check that seven distinguishable hues exist in each; if one theme cannot supply them the vocabulary is revisited before the lock, not after.
- `golden/format/` gains boards written through this phase's editor (the built flowchart, a pasted fragment, a connector with a label, heads and dashes) and, through `board.apply`, one with each colour, fill and stroke value, whose commands arrive in Phase 3.
- A test pins a digest of every `.out.wren-board` in a table in the test module, so a `WREN_BLESS=1` rewrite fails until that table and `schema` change together; a stability test asserts `write(parse(x)) == x` for every frozen output.
- This document gains a note "Schema 1 locked `<date>`, `<commit>`". Frozen: syntax and layout rules, key names and order, id and z-key alphabets, value sets, what the reader tolerates and preserves. Allowed without a bump: new optional keys, kinds, shapes and `ext.*`, all preserved by old builds. Needs a `requires` entry: anything an old build would draw misleadingly. Needs `schema = 2`: any change to the bytes written for an existing model.

## 5. Behaviour spec (ring 0)

Every numbered rule is a test target. Tags: **[P1]** viewer, **[P2]** "Build a flowchart", **[P3]** "Finish and harden". Untagged rules in a tagged subsection take the subsection's tag. Rules marked *modelled* were replayed in the keyboard model (`.claude/board-notes/research/keyboard-interaction/keyboard-sim/board_sim.py`).

Notation. **Mod** is ⌘ on macOS and Ctrl on Linux, the modifier document commands already use (`keybind.rs:241-244`). **Alt** is ⌥ or Alt. The app modifier stays ⌘ on macOS and Alt on Linux (`keybind.rs:42-47`). A **cell** is 20 board units; a board unit is one logical pixel at 100 % zoom; "screen px" is after zoom. An **element** is a node or a connector. The **machine** is the pure function of §5.2 in `wren-canvas/src/machine.rs`.

### 5.1 Key routing and the keymap change [P2]

A key press on a focused board is resolved in this order:

1. **Fixed keys.** While an input method has preedit text, every key is its own. Escape runs the ladder (§5.14). Space held in Idle or Armed is the pan modifier.
2. **Base layer.** Today's keymap. A trigger it binds to an App-scope command is the app's (`Host`): the board does not capture it and `App::key_input` runs it as today.
3. **Board layer.** Bindings to `Scope::Board` commands, bare keys included, read only by the focused board widget. A hit is `Command(c)`: captured and published as `Message::Command(c)`.
4. Anything else is `Pass`: captured; it is text in Editing and in the connect query and is dropped elsewhere.

What changes, and nothing more:
- **`wren-core/src/keybind.rs`.** `enum Layer { Base, Board }`. `Keymap` (`:269`) holds a base and a board layer; a binding replaces only within its layer. `default_board_bindings(platform)` sits beside `default_bindings` (`:213`, unchanged). `Keymap::build` takes `layer_of: impl Fn(&str) -> Option<Layer>` in place of `known_action` (`:280`; `None` is the unknown-action warning). The bindable check (`:294`) becomes `accel.is_bindable() || layer == Layer::Board`, so `keybind = x = board.delete` is legal and `keybind = x = note.new` still warns. `unbind` clears the base layer, as today; the action `board.unbind` clears the trigger in the board layer. `action(&accel)` and `bindings()` keep meaning the base layer (their readers are `key_route`, `preferences.rs:61`; `key_input`, `:535-539`; and the macOS menu, `macos_menu.rs:35-38`). New `board_action(&accel)` and `board_bindings()` read the board layer; `accel(action)` and `label` search both.
- **`wren-iced/src/app/commands.rs`.** `Scope::Board` (`:68`). One variant `CommandId::Board(wren_canvas::Command)`, so `command_enabled` (`:480`) and `run_command` (`:641`) gain one arm each instead of about ninety; `definitions!` (`:85-91`) then needs an expression form or a second table that `parse`, `definition` and the protocol test (`:1096`) also read. `CommandId::board_command()` is the twin of `intent()` (`:348`), and the invariant test at `:1151` gains "scope is Board exactly when `board_command()` is `Some`, and every `board.*` key is Board scope". `GROUPS` (`:107`) gains `"Board"` after `"Markdown"`. (`Action` is already a palette type, `palette.rs:131`; the board enum is `Command`.)
- **`wren-iced/src/app/preferences.rs`.** `board_policy(&Keymap)` beside `key_policy` (`:49`) implements the order above. It reads the press with `keys::accel` (`keys.rs:99`), not `keys::shortcut`, which drops bare keys (`keys.rs:62-73`). `rebuild_keymap` (`:124-132`) passes `layer_of` from `CommandId::parse` and the scope, then drops, with a warning, any user board binding whose trigger the base layer gives to an App-scope command. `App::key_input` (`:501`) does not change and never learns whether a board is focused: the base layer holds no Board command, and no board binding shares a trigger with an App command.
- **`wren-iced/src/keys.rs`: no change.** `keys::input` never turns a bare key into a trigger (`:42-56`); bare arrows and Enter reach the app only when no widget captured them (`:78-83`), and the focused board captures them; Escape always reaches the app (`:80`).
- **Kind-aware commands live in `run_command` and nowhere else.** `font.increase`, `font.decrease` and `font.reset` reach `step_text_size` (`commands.rs:757-759`, `preferences.rs:368`), which on a board steps the zoom and saves nothing [P1]. `navigation.find` and `navigation.outline` on a board open the jump palette [P3]. So a shortcut, the macOS menu, a palette row and the harness all do the same thing.

Rules for the widget:
1. The widget consults the policy only while it has iced focus. iced delivers key events to every widget; an unfocused board acts on none.
2. While preedit text exists no command runs (the editor's rule, `wren-editor/src/keys.rs:69-76`, `:84`).
3. In Editing only `board.label.commit` and `board.connect.next` are honoured from the board layer, and every other key edits text; so Mod+C, Mod+Z, arrows and Backspace are the label's. In keyboard Connecting an unmodified printable key (Shift included) is query text and only Enter, Escape, the arrows and Backspace act; so the tool letters C, H, A and D type into the query. `Host` keys go to the app in both states.
4. Auto-repeat (`repeat: true`) is honoured for select, extend, move and resize, and ignored for every other board command. Holding Tab or Delete does one thing.
5. A board command run from the palette, the menu or the harness acts on the board's state, first committing an open label, then gives the board the keyboard, as Editor commands do (`commands.rs:777-785`).
6. Board defaults share a trigger with the base layer only where the base command is Editor scope and so inert on a board: Mod+Enter, `alt+up` and `alt+down`. Two tests guard this on both platforms: no board default equals a trigger the base layer binds to an App command, and the base layer equals a frozen copy of today's `default_bindings`.
7. [P3] The digit row falls back to the physical key when the typed character is unbound, because `keys::accel` uses the typed character for unshifted keys (`keys.rs:115`) and AZERTY types `&` for 1.
8. The Commands palette (`palette.rs:426-449`) lists the Board group only while a board is open and omits the Markdown group then. With `boards` off no board row appears anywhere. [P3] On a board it lists `board.jump` and not the Find and Outline rows.
9. [P3] `docs/getting-started/keybindings.md` gains an "On a board" section: the table of §5.19, `board.unbind`, the digit-row note, and a note that Alt+Shift chords switch keyboard layout on some Linux desktops.

### 5.2 Surface states [P2]

The machine is `Machine::step(&mut self, input, view) -> Vec<Effect>` over toolkit-neutral inputs: a command, text, pointer press, move and release with button and modifiers, wheel, focus change, and a timestamp on each. Effects are intents for the model, an undo boundary, viewport changes and requests (clipboard, actions palette, jump). The widget only translates iced events; the harness injects at the same seam (§6.2).

| State | Leaves by |
|---|---|
| Unfocused | A click on the canvas, or column-right from Notes: Idle. The click is also processed normally. |
| Idle | Tool key: Armed (connector tool with exactly one node selected: Connecting). A creating command: Editing. `board.label.edit`: Editing. Pointer down: Pressed. Space held or middle button: Panning. Palette or dialog opens: Unfocused. |
| Armed(tool) | Esc or V: Idle. Tab or Mod+Arrow: create that shape, Editing, tool back to Select. Pointer down: Pressed. The tool is one-shot; there is no tool lock. |
| Pressed | Moved 4 screen px (`drag::THRESHOLD`, `drag.rs:10`): Moving on a selected or pressed node, Resizing on a resize handle, Connecting on a connect handle or with the connector tool on a node, Creating if a shape or text tool is armed, Marquee on empty canvas. Released sooner: a click (§5.3, §5.5). A second click within 300 ms and 6 px (iced's `mouse::Click`, `iced_core/src/mouse/click.rs:80-90`): double-click. |
| Moving, Resizing, Marquee, Creating, Connecting (pointer) | Release: commit as one undo unit and one draft, then Idle (Creating, and a new connector released on empty canvas: Editing). Esc, focus loss, an overlay opening, `settle` or `reload`: cancel and restore the model as it was at press. |
| Connecting (keyboard) | §5.12 rule 3. Enter: connect, Idle. Esc: Idle. |
| Editing | §5.9. |
| Panning | Button or Space released: the previous state. |

A mode chip in a canvas corner names the state when it is not plain Idle ("Diamond", "Connect to: che", "Hand"). An empty board shows a muted hint: "Tab adds a shape. R O D U T pick a tool." A read-only board allows selection, walking, copy, zoom, pan, jump and export; every mutating command is disabled. The [P1] viewer accepts only pan and zoom and has no selection.

### 5.3 Selection and the cursor [P2]

1. The **selection** is a set of element ids. The **cursor** is one element id: the element last selected. Both live in `BoardView`, are kept by id across reloads, and are never written to the file.
2. The cursor survives deselection and may be a connector (*modelled*: without this, Esc after a distant connect loses the place). It is drawn as a ring distinct from the selection outline whenever the selection is not exactly `{cursor}`.
3. Click selects the element under the pointer and clears the rest. Shift-click toggles it. A click on empty canvas clears the selection and keeps the cursor. A right-click selects as §5.17 rule 2 says; in Phase 2 it does nothing more.
4. Hit order: resize handles of the selected node, connect handles, endpoint handles of the selected connector, nodes from the top of the stacking order down (inside the shape's outline, whatever its fill), connectors within 6 screen px of the routed path, canvas.
5. Marquee selects the elements whose bounds lie entirely inside the rectangle. Shift adds to the selection.
6. `board.select.all` selects every node and connector. `board.select.connected` adds every node reachable from the selection along connectors in their `from` to `to` direction, and those connectors. `board.select.none` is the selection rung of Escape.
7. Commands that need one element (`board.label.edit`, `board.connect.<dir>`, `board.select.connector_<dir>`) are disabled otherwise. Walking with several selected starts from the cursor. An unfocused board draws its selection muted, with no handles or cursor ring.

### 5.4 The New note dialog [P2]

1. The dialog of `note.new` (⌘N or Alt+N, name prefilled, Enter creates; `DialogKind::CreateNote`, `app/workspace.rs:145`; drawn by `dialog_view`, `app/workspace_view.rs:1058`) gains a row of two kind pills under the name field: Markdown and Board.
2. Markdown is selected every time the dialog opens. The last choice is never remembered, so ⌘N, type, Enter creates a `.md` note exactly as today.
3. Tab selects the next pill and Shift+Tab the previous one, wrapping, while focus stays in the name field. A board is ⌘N, type, Tab, Enter. (The dialog handles no Tab today; this is new key handling while that dialog is open.)
4. The pills are clickable, a click keeps focus in the name field, and the row shows a "Tab" hint.
5. The pills appear only when `boards` is on. With it off the dialog is pixel-identical to today's.
6. Rename shows no pills: a rename never changes kind and keeps the extension.
7. `note.new_dated` stays Markdown-only and shows no dialog. The scoped ⌘N in the Sections and Groups columns (new section, new group) shows no pills.
8. `note.new_board` ("New board…", App scope, group "This note", no default key; listed only with `boards` on, enabled with a selected group as `note.new` is, `commands.rs:500-503`) opens the same dialog with Board preselected.
9. Enter sends `Op::CreateNote { group, title, kind }`. A name already taken by a file of the same kind shows the dialog's error as today. A new board opens empty at 100 %, focused and Idle, showing the hint of §5.2.

### 5.5 Tools and creating shapes [P2]

1. Tools: select, hand, the eight shapes (rectangle, rounded, pill, ellipse, diamond, parallelogram, cylinder, document), text, connector. A shape created without an armed shape tool is a rectangle; new nodes never copy the source's shape.
2. **Default size** is 120 by 60 (6 by 3 cells) for every shape. A new text node is 120 by 20.
3. Armed shape or text tool, click on canvas: a default-size node centred on the point, snapped (§5.8). Drag: a node from the press corner to the release corner, both snapped, at least one cell each way.
4. Double-click on empty canvas with the select tool: a default rectangle centred there.
5. Double-click on a node or connector: edit its label.
6. Every creation lands in Editing with the new node selected and an empty label. The node goes on top of the stacking order, with the neutral colour, the default fill and a generated id. A colour or fill command with nothing selected does nothing: there is no sticky style for the next node.
7. Hand tool: a drag pans; nothing is selected or created.
8. Hover connect handles: resting the pointer on a node for 120 ms in Idle shows four handles 16 screen px outside the midpoints of its sides (a deadline from the machine, as `Drag::next_deadline`, `drag.rs:525`). A click on one runs `board.connect.<side>`; a drag from one starts Connecting. Resize handles sit on the bounds, so the two never overlap.
9. `board.shape.<shape>` sets the shape of every selected node, keeping `at`, `size` and label, as one undo unit.

### 5.6 Keyboard building: Tab and quick-connect [P2]

All of this subsection is *modelled*. Geometry is in absolute board units; children of a parent are compared by absolute bounds.

1. **Flow direction.** Each connector between two nodes votes with its nominal exit side (§5.12 rule 4), pins ignored. The majority wins; a tie that includes down, or a board with no connectors, is down; other ties go right, then up, then left. Nothing is stored.
2. **Ideal slot** for a new node of size `w x h` from source `s` in direction `d`, with a gap of 60 along a vertical direction and 80 along a horizontal one: down `(s.x + (s.w - w) / 2, s.y + s.h + 60)`; up `(s.x + (s.w - w) / 2, s.y - 60 - h)`; right `(s.x + s.w + 80, s.y + (s.h - h) / 2)`; left `(s.x - 80 - w, s.y + (s.h - h) / 2)`; then snapped to the grid.
3. **Free** means the slot grown by one cell on every side intersects no other node.
4. **Nearest free slot.** If the ideal slot is not free, try `k = 1, 2, …` steps across the direction, the positive side first (right for a vertical direction, down for a horizontal one), with a pitch of `w + 80` across a vertical direction and `h + 60` across a horizontal one. After 200 steps the command is blocked with a warning.
5. **`board.connect.next` (Tab)** always creates. The source is the single selected node, or the `to` node of the single selected connector. The direction is the flow direction. The new node takes the nearest free slot and a connector from the source. With nothing selected it creates an unconnected node at the viewport centre, snapped, stepping right by one pitch until free. With several selected it is blocked.
6. **`board.connect.<dir>` (Mod+Arrow)** needs one node selected. If a node intersects the ideal slot (the nearest by centre distance, ties by id) and the pair has no connector in either direction, it connects to that node, selects it, and stays in Idle. Otherwise it creates a node in the nearest free slot, connects, and enters Editing. With a shape tool armed it always creates.
7. An armed shape tool sets the new node's shape for rules 5 and 6 and then resets. Armed text, connector and hand tools reset and give a rectangle.
8. Each press commits at once as one undo unit (node and connector together).
9. Connectors made here carry no side hint: `from = { node }`, `to = { node }`, elbow routing, an arrowhead at `to` only. No keyboard command writes `side`.

### 5.7 Walking [P2]

1. `board.select.<dir>` (plain arrows) moves the selection and never dirties the file. From node `o`, a candidate `n` must have its centre ahead of `o`'s centre in direction `d`. `main` is the centre distance along `d`, `cross` the centre distance across it. Tiers (*modelled*; without tier 1, Up from a branch child never reached its parent): 0, in line: either centre lies strictly inside the other node's span across `d`; 1, `n`'s centre is past `o`'s edge on that side and the two share a connector; 2, `main >= cross`; 3, `n`'s centre is past `o`'s edge. Lowest `(tier, main + 2 * cross, id)` wins. No candidate: nothing happens.
2. From a selected connector, an arrow selects whichever end node lies further in that direction. With nothing selected, it selects the cursor if it still exists, otherwise the node with the smallest `(y, x)`.
3. `board.select.extend_<dir>` (Shift+Arrow) adds the walk target from the cursor to the selection and moves the cursor to it. It never removes.
4. `board.select.connector_<dir>` (Shift+Mod+Arrow) selects the connector that meets the selected node on that side (§5.12 rule 4), the lowest id first.
5. After any keyboard selection change the viewport pans by the least amount that shows the cursor element with a 40 screen px margin.

### 5.8 Move and resize [P2]

1. The grid is 20 units and snapping is always on for creating, moving and resizing. There is no key to suspend it. Align and distribute are the one exception (§5.16 rule 3).
2. **Pointer move.** The drag delta is rounded so the pressed node's top-left lands on the grid; every selected node moves by that same delta, so relative offsets, and other nodes' off-grid positions, are kept. A node whose ancestor is also selected is not moved itself. Free connector ends of selected connectors move by the delta. Connectors re-route live.
3. **Duplicate-drag.** With ⌥ (macOS) or Ctrl (Linux) held when the threshold is crossed, the originals stay and copies move (§5.10 rule 3). Alt-drag is not used on Linux; desktops bind it.
4. **`board.move.<dir>` (Alt+Arrow)** moves the selection one cell, or, when the cursor node is off the grid on that axis, to the next grid line that way.
5. **Pointer resize.** Exactly one selected node shows eight handles. The dragged edges snap to the grid; the opposite corner stays; minimum one cell each way. No modifiers and no multi-node resize in ring 0.
6. **`board.resize.*` (Alt+Shift+Arrow)**: Right is wider, Left narrower, Down taller, Up shorter, one cell, top-left fixed, on every selected node, minimum one cell.
7. `at` and `size` are written as integers. A stored `size` is changed only by resize and by label auto-grow (§5.9 rule 7).

### 5.9 Label editing [P2]

1. **Start:** `board.label.edit` (Enter or F2) with one element selected, a double-click on it, or any creation. Typing does not start an edit. An existing label opens with all its text selected.
2. The label is edited in place, at the current zoom, in the label font and wrap width it is drawn with. If the label text would be under 10 screen px tall, the view first zooms to 100 % centred on the node.
3. **Inside a label** Enter and Shift+Enter insert a line break. Arrows, Home, End, Backspace, Delete and Mod+A/C/X/V are the text's. Mod+Z puts the label back as it was when this edit began and Shift+Mod+Z restores the typing; neither reaches the board's history. Pasting a board fragment here pastes its text. On macOS a key with ⌘ held never types.
4. **Commit:** `board.label.commit` (Mod+Enter), Esc, Tab (commit, then `board.connect.next`), a click outside (then processed as a normal click), focus loss, an overlay opening, and `settle` before any non-passive op (§3.4 rule 3). Trailing blank lines are trimmed. An unchanged label makes no undo unit and no draft.
5. **There is no cancel key.** Board undo after the commit restores the old label in one step. A Session-driven text change closes the editor without committing; what was typed is already in the draft (and so in the conflict copy).
6. **Live draft.** Each text change updates the model and sends `Op::EditDraft` through the pristine rule, so a crash, a quit or a note switch never loses a label. The undo unit is still one per edit.
7. **Auto-grow.** While editing, text wraps at the node's label box width (§3.7). If the wrapped text is taller than the label box, the height grows in whole cells with `at` fixed. Width grows, in whole cells, only for a single word wider than the label box. Nothing shrinks. The canvas measures and passes the new `size` in the intent (`SetText { id, text, size }`), so `plan` stays pure and takes no measurer. The grown `size` travels in the live draft and belongs to the label's undo unit. Nothing is re-measured on load.
8. A shape may have an empty label. A text node or a connector label committed empty is deleted; if this edit created it, its creation unit is dropped from the history too.
9. **Input method.** Enabled only in Editing, requested as the editor does (`wren-editor/src/widget.rs:408-430`, `:747`), with the candidate window at the caret in screen coordinates. While preedit text exists, Enter, Tab, Backspace, Delete, Esc and arrows are the input method's, whatever the modifiers. Leaving Editing clears the preedit. The wheel still pans and zooms while editing, and the editor follows the node.
10. **One focus id.** The label editor lives under the focus id `note-editor`; it is never a second focusable widget (§5.14). The mechanism is decided by the Phase 1 spike (§7.2, S8).

### 5.10 Delete, duplicate and the clipboard [P2]

1. **`board.delete`** (Backspace or Delete) removes the selected elements, every connector attached at either end to a removed node, and the descendants of removed nodes, as one undo unit. The selection is then empty and the cursor moves to the nearest remaining node (by centre distance from the removed cursor element, ties by id).
2. **The fragment** of a selection is its nodes, their descendants, and every connector whose two ends are each a node in the fragment or a free point. A selection with no node has no fragment, and copy, cut and duplicate are disabled.
3. **`board.duplicate`** (Mod+D) copies the fragment one cell right and down, with new ids, on top of the stacking order in the same relative order, and selects the copies. One undo unit.
4. **`board.copy`** writes text to the system clipboard (iced 0.14's clipboard is text only): the line `# wren-board fragment 1`, then the canonical writer's output for a board holding only the fragment, starting `schema = 1`. Nodes whose parent is outside the fragment are written top-level with absolute `at`. The text after the header is a valid board file.
5. **`board.cut`** is copy then delete, one undo unit.
6. **`board.paste`** reads the clipboard text. If its first line is exactly `# wren-board fragment 1` the rest is parsed with the file loader and its limits; a parse failure, a newer `schema` or an unknown `requires` blocks the paste with a warning and pastes nothing.
7. **Ids on paste.** Every pasted node and connector gets a new generated id, always. `parent`, `from.node` and `to.node` are remapped. A connector end naming a node outside the fragment drops that connector; a `parent` outside the fragment is cleared.
8. **Place.** The fragment is offset one cell right and down per paste of the same clipboard text into this board (`n` cells on the `n`-th). If that bounding box does not intersect the viewport, it is centred in the viewport instead, snapped. Pasted elements go on top in their relative order and become the selection; the cursor is the first pasted node. One undo unit; the board stays in Idle.
9. **Foreign text** (no header) becomes one text node: line endings to LF, control characters other than tab and LF removed, trailing blank lines trimmed, wrapped at 320 units, sized once by the canvas with the auto-grow rule (the size travels in the intent), centred in the viewport, selected. Empty text does nothing. Text over the 16 KiB label cap is blocked with a warning.

### 5.11 Undo and redo [P2]

1. History is whole-model snapshots (§3.4 rule 6), in memory, for the open board only. Each snapshot also holds the selection and cursor from before the unit; undo restores all three and leaves the viewport alone.
2. **One unit each:** a creation (node and connector together); a label commit that changed text, with its auto-grow; a pointer move, resize, new connector, endpoint drag or duplicate-drag, on release; a run of `board.move.*` or of `board.resize.*`; delete; duplicate; cut; paste; and each shape, colour, fill, connector-style, stacking, align and distribute command.
3. A run of move or resize keys is one unit while the selection is unchanged and no other command, pointer input or 1 s pause intervenes.
4. Selection, cursor, tool, zoom and pan changes make no unit and no draft. A command that changes nothing (align already aligned, front already in front) makes none either.
5. Every unit ends with exactly one `Op::EditDraft`, chosen by the pristine rule. The undo floor is the opening text: undoing to it sends the original bytes, and the document is clean again.
6. `board.undo` and `board.redo` run in Idle and Armed. A new unit clears the redo side. History is dropped when another note opens and on any Session-driven text change.

### 5.12 Connectors [P2]

1. **By pointer.** Drag from a hover connect handle, or arm the connector tool and drag from a node. Over a node the target is outlined and shows four side handles. Release on a node: connect to it. Release on empty canvas: create a default node centred there (the armed shape, else a rectangle), connect, Editing. Release on the source: cancel. A press on empty canvas with the connector tool does nothing.
2. **By keyboard, near:** `board.connect.next` and `board.connect.<dir>` (§5.6).
3. **By keyboard, far** (*modelled*; a loop edge takes five keys). `board.tool.connector` (C or A) with exactly one node selected enters Connecting. Arrows move a highlighted target by the walking rules, from the current target or, at first, from the source. A printable key appends to a query shown in the mode chip, Backspace removes its last character, and the target becomes the best match: labels containing the query, case-insensitive, prefix matches first, then by `(y, x)`. An arrow clears the query. Enter connects the source to the target, selects the new connector, and returns to Idle; with no target, the source as target, or an existing connector from this source to this target, it only returns to Idle. Esc returns to Idle. The query lives in the machine: there is no palette, so the whole script replays as a unit test. The input method is off, so non-Latin labels are reached by arrows or by pointer.
4. **Ends and sides.** An end is attached to a node, floating (the UI's default: no `side`) or pinned (`side` set), or is a free point. The **nominal exit side** of a connector at its `from` node is down when the `to` node lies wholly below the `from` node's bottom edge, up when wholly above its top edge, and otherwise right or left by centre; the nominal entry side is the opposite one. The **side of a connector at a node**, which `board.select.connector_<dir>` uses, is the pinned side, else the side the router draws. The router must draw a floating end on its nominal side whenever the two nodes overlap on one axis and are apart on the other (the acceptance script depends on it), and should elsewhere.
5. **Pinning.** An end released within 10 screen px of one of the target's side handles is pinned to that side; released anywhere else on the node it floats. Nothing else in ring 0 writes `side`, and nothing writes `anchor`.
6. **Endpoint drag.** A single selected connector shows a handle at each end. Dragging one onto a node re-attaches it (rule 5). Released on empty canvas, an end that was already a free point moves there, snapped; an attached end returns to where it was. The UI never creates a free end; they come only from files.
7. **Re-routing.** Routes are recomputed from current geometry whenever an attached node moves or resizes, during the drag as well as after it. A floating end may change side; a pinned end does not. Moving a node changes no edge line in the file.
8. **Label.** One label per connector in the ring 0 UI: the first entry of `labels`, at the middle of the route. `board.label.edit` or a double-click edits it (§5.9). Further labels in a file are drawn and kept.
9. **Style commands** act on every selected connector, one undo unit each: `board.edge.routing` switches straight and elbow; `board.edge.start` and `board.edge.end` cycle that end's head through none, arrow; `board.edge.reverse` swaps `from` and `to` with their heads; `board.stroke.dashed` switches solid and dashed. In Phase 2 they are reached from the Commands palette.
10. Deleting a node deletes its connectors in the same unit. A second connector between the same pair may be drawn by pointer. A dangling connector in a loaded file is never repaired or dropped by an unrelated edit.

### 5.13 Zoom and pan

1. [P1] **Wheel.** Unmodified, it pans; with Mod held, it zooms about the pointer. The rule is the same for `Lines` and `Pixels` deltas, because X11 reports touchpads as `Lines`. `Pixels` are divided by the window scale factor; one line is 40 logical px. Shift with a `Lines` delta pans sideways. Zoom multiplies by `1.1` per line, within 10 % to 400 %.
2. [P1] **Burst latch.** Wheel events less than 150 ms apart are one burst, and the burst keeps the pan-or-zoom choice of its first event; iced drops the scroll phase, and macOS momentum outlives the modifier.
3. [P1] **Drag to pan:** the middle button anywhere, and Space held with a drag. [P2] The hand tool. Space types a space in Editing and in the connect query.
4. [P1] A board opens fitted to its content with a 40 screen px margin, never above 100 %; an empty board opens at 100 %. Returning to a board opened earlier in this run restores its viewport (§3.3).
5. [P1] **Keyboard zoom** is the text-size commands: `font.increase` and `font.decrease` step through 10, 25, 50, 75, 100, 150, 200, 300, 400 % about the viewport centre and `font.reset` is 100 %. Their keys are ⌘= (or ⌘+), ⌘- and ⌘0 on macOS and Alt+= (or Alt++), Alt+- and Alt+0 on Linux (`keybind.rs:232-237`). There are no separate board zoom-in, zoom-out or reset commands. [P2] `board.zoom.fit` fits all content and `board.zoom.selection` the selection, both with the margin of rule 4 and never above 100 %.
6. Zoom and pan are view state: never in the board file, never a draft, never an undo unit. The wheel works on an unfocused board when the pointer is over it. There are no pan keys in ring 0; walking reveals the cursor (§5.7 rule 5) and the fit commands cover the rest.
7. Pinch does not arrive today: `vendor/iced_winit/src/conversion.rs:214-231` forwards only `MouseWheel`. A side channel is not built in the MVP (§2.4 question 15).

### 5.14 The Escape ladder [P2]

One rung per press, the first that applies:

1. Preedit text: the input method's.
2. A palette or dialog is open: the app closes it and `column_focus` returns the keyboard (`app.rs:477-486`). The board is not focused then and does not see the key.
3. A pointer gesture in flight: cancel and restore.
4. Editing: commit.
5. Connecting by keyboard: back to Idle, nothing made.
6. An armed tool: back to Select.
7. A selection: clear it; the cursor stays.
8. Nothing: no effect. Escape never leaves the board and never folds a column, as in the editor (`columns.rs:269-277`).

Rungs 3 to 7 are the widget's, and it captures the event. A captured Escape reaches the app as `Nav::FieldEscape` (`keys.rs:87-93`), which with no overlay only refocuses the active column; on rung 8 the app's `Message::Escape` calls `return_to_editor`. Both refocus `note-editor`, which is the canvas, and that must be a no-op. So the label editor must live under that one focus id, or an Escape meant for a composition would move focus and commit half a label. A test presses Escape in each state and asserts the iced focus and the state afterwards.

### 5.15 Focus and Wren's columns

1. [P1] The board is the editor column. Any board input other than the wheel sets `self.column` to the editor column, as editor actions do (`app.rs:567-571`).
2. [P2] `navigation.column_left` (⇧⌘[ or Alt+Shift+[) is a `Host` key: the board settles pending input, and Notes takes the keyboard. In Notes, → or Enter returns (`columns.rs:194-211`, `preferences.rs:528`); Escape in a browser column returns too.
3. [P2] A click on an unfocused board focuses it and is processed in the same click. Tab never moves focus off the board.
4. [P2] An overlay opening commits an open label, cancels a pointer gesture and keyboard Connecting, and disarms the tool. Window blur does the same through `window_unfocused` (`workspace.rs:867`) and its flush.

### 5.16 Colour, fill, stacking, align and distribute [P3]

1. `board.color.neutral`, `.red`, `.orange`, `.yellow`, `.green`, `.cyan`, `.purple` and `.blue` (bare keys 0 to 7) set the colour token of every selected node and connector, one undo unit. `board.fill.none`, `.tint` and `.solid` set the fill of every selected shape. `board.stroke.dashed` (from Phase 2 for connectors) now also toggles the outline of selected shapes. With nothing selected they do nothing. A command that changes nothing sends no draft. Tokens resolve per theme (§3.7); no command writes literal hex; an unknown token in a file is never rewritten.
2. Stacking order is per parent, nodes and connectors together. `board.arrange.front` and `.back` move the selection above or below all siblings, keeping its relative order. `board.arrange.forward` and `.backward` move it one place past the next unselected sibling. Only the moved elements' `z` keys are rewritten, one line each, except in the renumber fallback of §4.7, which rewrites the whole scope in the same undo step. Already at the end is a no-op.
3. `board.align.*` needs two or more selected nodes and moves each to the selection bounding box's left, horizontal centre, right, top, vertical middle or bottom. `board.distribute.*` needs three or more: sorted by position, the outermost two stay and the gaps between neighbours are made equal, each node placed from the exact fraction rounded once. Results are integers and are **not** snapped to the grid (snapping would break centring of nodes whose sizes differ by an odd number of cells); centre and middle round half away from zero. Both are idempotent: a second run changes nothing and sends no draft. Overlapping nodes give negative equal gaps, not an error. If a node and one of its ancestors are both selected, only the ancestor moves. Connectors are ignored and follow their nodes. Positions are computed in absolute units and written parent-relative.

### 5.17 Right-click and the actions palette [P3]

1. There is no popup menu, as elsewhere in Wren: a right-click on a browser row opens the palette in `Mode::ItemActions` (`app/gestures.rs:83-88`). The board adds `Mode::BoardActions`, built like it (`palette.rs:647-663`; the command list function at `commands.rs:971`).
2. Right-click, or Ctrl-click on macOS, on an element outside the selection selects it alone; on a selected element it keeps the selection; on empty canvas it clears it. Then the palette opens. `board.actions` (`/` or Shift+F10) opens it for the current selection.
3. Rows are the board commands enabled for the selection, and only those, grouped: shape, colour, fill, stroke, connector, arrange, align, distribute, edit (label, duplicate, copy, cut, delete). With nothing selected: paste, select all, zoom to fit, export. The row matching the selection's current value is marked current and preselected on an empty query, as the theme picker does (`palette.rs:268-274`).
4. Moving the highlight over a shape, colour, fill, stroke or connector-style row previews it on the canvas, following `sync_preview` (`preferences.rs:313-334`). The preview is a patch applied when the scene is built: it is never in the model, never in an undo snapshot and never sent as a draft. Enter applies it as one undo unit and returns the keyboard to the board. Every other way of closing drops the patch, and `settle` drops it too (§3.4 rule 3). Arrange, align and distribute rows do not preview.

### 5.18 Jump to node and search reveal [P3]

1. `board.jump` opens the palette in a new `Mode::BoardNodes` ("Jump to node…") listing every node and connector label, first line as the title and the shape or "connector" as the tag, in reading order `(y, x)`, fuzzy-matched on the label like the outline. Elements without a label are not listed.
2. Moving the highlight selects and centres that element as a preview. Enter keeps it and focuses the board. Any other close restores the viewport and selection, as the outline palette restores its origin (`OutlineOrigin`, `palette.rs:178`). Rows are keyed by element id, so a reload underneath keeps the highlight when the element survives; the palette closes when the note changes. Nothing here changes file bytes.
3. `navigation.outline` (⇧⌘O or Alt+Shift+O) and `navigation.find` (⌘F, Alt+F or Ctrl+F) open the same palette on a board, routed in `run_command` (§5.1). Today `open_palette(Mode::Outline)` requires `ensure_editor_ready` (`palette.rs:312`), which fails on a board. A board in the raw fallback keeps the editor's Find and has no Outline.
4. A workspace search hit on a board selects and centres the first element, in reading order, whose label contains the trimmed query, case-insensitive, leaving the zoom alone unless the element would be under 8 screen px tall, in which case it zooms to fit it. It is the board branch beside `reveal_literal` in `activate_note` (`palette.rs:1032-1052`).

### 5.19 Command table

Every `board.*` row is `Scope::Board`; the `note.*` rows are `Scope::App`. "—" means no default key: the command is reached from the palette (and, from Phase 3, the actions palette). Linux keys equal macOS keys with Ctrl for ⌘ and Alt for ⌥ unless shown. This table is the only command list; a chain that adds or renames a command edits it.

| Command key | Palette title | Group | macOS | Linux | Phase |
|---|---|---|---|---|---|
| `note.export_svg` / `note.new_board` / `note.export_png` | Export board as SVG… / New board… / Export board as PNG… | This note | — | — | 1 / 2 / 3 |
| `board.tool.select` / `.hand` | Select tool / Hand tool | Board | V / H | same | 2 |
| `board.tool.{rectangle,ellipse,diamond,pill,cylinder,parallelogram,rounded,document}` | Rectangle tool, … | Board | R / O / D / U / Y / P / — / — | same | 2 |
| `board.tool.text` / `.connector` | Text tool / Connector tool | Board | T / C or A | same | 2 |
| `board.shape.<shape>` (eight) | Change shape to rectangle, … | Board | ⇧ + the tool letter; none for rounded, document | same | 2 |
| `board.label.edit` / `.commit` | Edit label / Finish label | Board | ↵ or F2 / ⌘↵ | Enter or F2 / Ctrl+Enter | 2 |
| `board.connect.next` | Add next shape | Board | Tab | Tab | 2 |
| `board.connect.{left,right,up,down}` | Connect left, … | Board | ⌘ + arrow | Ctrl + arrow | 2 |
| `board.select.{left,right,up,down}` | Select left, … | Board | arrow | arrow | 2 |
| `board.select.extend_{left,right,up,down}` | Extend selection left, … | Board | ⇧ + arrow | Shift + arrow | 2 |
| `board.select.connector_{left,right,up,down}` | Select connector on the left, … | Board | ⇧⌘ + arrow | Ctrl+Shift + arrow | 2 |
| `board.select.all` / `.connected` / `.none` | Select all / Select connected / Clear selection | Board | ⌘A / ⇧⌘A / — | Ctrl+A / Ctrl+Shift+A / — | 2 |
| `board.move.{left,right,up,down}` | Move left, … | Board | ⌥ + arrow | Alt + arrow | 2 |
| `board.resize.{wider,narrower,taller,shorter}` | Make wider, … | Board | ⌥⇧ + → ← ↓ ↑ | Alt+Shift + → ← ↓ ↑ | 2 |
| `board.delete` / `board.duplicate` | Delete selection / Duplicate | Board | ⌫ or Del / ⌘D | Backspace or Delete / Ctrl+D | 2 |
| `board.copy` / `.cut` / `.paste` | Copy / Cut / Paste | Board | ⌘C / ⌘X / ⌘V | Ctrl+C / X / V | 2 |
| `board.undo` / `.redo` | Undo / Redo | Board | ⌘Z / ⇧⌘Z | Ctrl+Z / Ctrl+Shift+Z or Ctrl+Y | 2 |
| `board.edge.{routing,start,end,reverse}`, `board.stroke.dashed` | Connector: straight or elbow / start head / end head / reverse; Solid or dashed | Board | — | — | 2 |
| `board.zoom.fit` / `.selection` | Zoom to fit / Zoom to selection | View | ⇧1 / ⇧2 | same | 2 |
| `board.arrange.{front,back,forward,backward}` | Bring to front / Send to back / Bring forward / Send backward | Board | ] / [ / ⇧] / ⇧[ | same | 3 |
| `board.align.{left,center,right,top,middle,bottom}`, `board.distribute.{horizontal,vertical}` | Align left, … / Distribute horizontally, vertically | Board | — | — | 3 |
| `board.color.{neutral,red,orange,yellow,green,cyan,purple,blue}` | Colour: neutral, … | Board | 0 to 7 | same | 3 |
| `board.fill.{none,tint,solid}` | Fill: none, tint, solid | Board | — | — | 3 |
| `board.actions` | Board actions… | Board | / or ⇧F10 | same | 3 |
| `board.jump` | Jump to node… | Go | — (⇧⌘O and ⌘F reach it) | — (Alt+Shift+O, Alt+F, Ctrl+F reach it) | 3 |

That is 64 Board commands in Phase 2 and 25 in Phase 3. ⌘[ and ⌘] stay Back and Forward and ⇧⌘[ and ⇧⌘] stay the column keys (`keybind.rs:224-227`). Ctrl+Arrow on macOS and Ctrl+Alt+Arrow on Linux are left to the desktop. `keys::accel` already maps shifted digits and brackets to their base key (`keys.rs:136-163`), so `⇧1` and `⇧]` need no change there.

[P3] **The macOS menu** gains a Board submenu between View and Window (`macos_menu.rs:52-107`, assembled at `:113`): New Board, Duplicate, the four arrange items, an Align submenu, the two distribute items, Select Connected, Export as SVG, Export as PNG. A Board item shows an accelerator only when its trigger contains `super`; the `item` helper (`:32-40`) shows the first binding muda can parse, and a bare-key menu equivalent would fire while typing in any field. Copy, cut, paste, undo and select all stay out of the menu for the same reason. The View menu's text-size items zoom a board through `run_command`. "Jump to Heading…" in Go (`:78`) becomes "Jump in Note…". The submenu is not built with the setting off (`install` re-runs on a settings reload, `preferences.rs:249`). Every item is created enabled today (`:39`); a `sync_board` beside `sync_updates` (`:124`) enables each Board item by `command_enabled`, so New Board follows `note.new_board` and the rest are greyed unless a board is the surface. Items run through `menu_command` (`preferences.rs:487-488`), the keyboard's path.

### 5.20 The acceptance script [P2]

macOS keys; Ctrl for ⌘ on Linux. It is replayed twice: as a unit test of the machine in `wren-canvas`, and through `board.key` and `board.type` in every smoke lane.

- `U Tab` Start, `Tab` Receive order, `Esc D Tab` Valid?
- `Esc ⌘→` Reject order, `Esc ⇧⌘← ↵` No `Esc ←`
- `Tab` Check stock, `Esc ⇧⌘↑ ↵` Yes, `Esc D Tab` In stock?
- `Esc ⌘→` Backorder, `Esc ⇧⌘← ↵` No, `Tab` Notify customer, `Esc C` che `↵`, `← ↓`
- `Tab` Charge card, `Esc ⇧⌘↑ ↵` Yes, `Esc D Tab` Payment ok?
- `Esc ⌘→` Retry payment, `Esc ⇧⌘← ↵` No, `Esc → C` cha `↵`, `← ↓`
- `Tab` Pack, `Esc ⇧⌘↑ ↵` Yes, `Tab` Ship, `Tab` Send receipt, `Esc U Tab` End
- `Esc`, `↑` eight times, `→`, `C` end `↵`

Pass means: 15 nodes (two pills, three diamonds, ten rectangles) and 17 connectors; the fifteen labels; the six branch connectors labelled Yes or No; both loops present as cycles; no two node bounds intersect; every node is still 120 by 60 and its position, relative to Start, equals the model's output times 20; every `at` and `size` is on the grid and every connector end is attached to an existing node; the final state is Idle with the last connector selected; 232 key presses (153 label characters, 9 query letters, 70 chords). After the autosave delay the file on disk parses with `tomllib`, is canonical, parses to the same model, and reopening it leaves the bytes unchanged; undoing to the start gives the canonical empty board. Ids are random, so tests assert structure and line diffs, never a byte-for-byte built file. The model's expected layout is copied into the repository as a golden file by the item that first uses it (§7.3, E8); port the layout, not the model's code. The model has no auto-grow, which is why §3.7 requires these labels to fit their default boxes.

## 6. Testing and local verification

This follows the editor's testing contract: pure logic is tested in its own crate, the app is driven over the private test socket, real keys and pointer are checked only where a real display exists, and scenes are captured and looked at.

### 6.1 Layers and fixtures

**Fixtures.** One sample workspace serves the harness, the scenes and hand trials: `tools/wrentest/fixtures/boards/workspace/Boards/Samples/`, created in step 0 from the reference writer (§7.1). It holds `Checkout flow.wren-board` (the 15-node, 17-connector flowchart, canonical), `Shapes.wren-board` (all eight shapes, free text, an unknown kind, an unknown shape, a dangling connector, a child with a `parent`), `Overlap.wren-board` (three stacked shapes whose labels must occlude), `Future.wren-board` (`requires = ["frames"]`, read-only), `Hand written.wren-board` (valid but not canonical: comments, unsorted blocks, odd spacing; LF only) and `Notes.md`. A board with git conflict markers is written at run time by the helper `tools/wrentest/boards.py` (`conflict_board()`), never committed: a committed file with markers trips `git diff --check` at every merge. A `.gitattributes` line marks the fixture and golden folders `-text` so CRLF and BOM golden inputs survive a checkout. Tests copy the workspace to a temporary directory; nothing opens it in place, because opening a workspace writes `.wren/workspace.toml` into it. Large boards come from `boards.py` (`grid_board(n)`), as `large_group` does for notes (`tools/wrentest/test_markdown_keys.py:522`).

**`wren-board`** (inline unit tests; no iced, no filesystem). The format tests of §4.16. Integrity: one case per flagged rule. Geometry: the parent transform (nested, through an unknown kind), anchors on each side of each shape, z ordering with ties, grid snap. Routing: a table over the sixteen side pairs for both routes, with overlapping, touching, coincident and zero-size rectangles and a free end; every elbow segment is axis-aligned, both ends lie on the outline, no NaN. Scene snapshots: `scene::build` with `FixedMeasure`, dumped one primitive per line to `golden/scene/<name>.scene`, the single check that the widget and the SVG writer receive the same wrapped lines. SVG golden files from the same scenes: well-formed XML, `<`, `&` and quotes escaped, no XML-illegal character.

**`wren-core`.** The *op-by-kind table* (`crates/wren-core/tests/session.rs`): for each kind, every `Op` that touches a note (create, open, rename, move, move and rename of its group, reorder, delete, flush, external reload, dirty conflict copy, back, forward), asserting that extension, title and kind survive, with a row for `Flow.md` beside `Flow.wren-board`. The *frozen released validator* (`tests/released_state.rs`): a verbatim copy of v0.1.1's `parse_path` and `validate` (`workspace_state.rs:511-545`, `:568-656`, taken with `git show v0.1.1:…`), run over the state file after every step of the kind table's navigation rows and of a seeded 200-step random walk over mixed kinds [2,000 steps as an ignored test]. Index and reconcile: labels are searchable and TOML syntax (`kind`, `schema`) is not; an unparseable board indexes its raw text; a counter proves zero label extractions and zero re-sent notes across 40 `Op::Tick`s over 200 unchanged boards. The gate: with the option off, a scan of a workspace containing boards equals a scan of the same workspace without them.

**`wren-canvas`.** The machine as a pure function: one table row per transition of §5.2, the Escape ladder, ignored auto-repeat, the 4 px threshold, double-click timing, and the acceptance script. Hit testing at three zoom levels and through a parent transform. The key translation (constructed iced events in, `Input` out). The measurer against the 24-label corpus. The layer-split function. No unit test constructs a renderer (the editor's do not either; its tests loop over its two text engines, `wren-editor/src/backend/mod.rs:31`): wgpu and tiny-skia are covered by the four functional lanes (`.github/workflows/ci.yml:61-68`) and by scenes, which is why the widget only paints a scene built elsewhere.

**`wren-iced`** (inline app tests without a window, as `navigation_fixture` builds one, `app/test_requests.rs:1154`). The three-way surface and its re-derivation; `ensure_editor_ready` fails on a board and `markdown.bold` leaves the Session text alone; `settle` runs before any non-passive op; the pristine rule; undo cleared on a Session-driven change; kind-aware `command_enabled`; no two default bindings in one layer share a trigger; with the setting off no board surface exists.

**Functional** (`tools/wrentest/test_board_*.py`, over the test socket) and **native X11** (`@pytest.mark.native_x11` in `test_board_native.py`): real xdotool input, reusing `xdo` and `focus_native` (`test_editor.py:294-306`) and the pointer helpers of `test_gestures.py`, every target taken from `board.bounds`. The native cases prove only the translation from iced events to machine inputs. Phase 1: wheel pans, Ctrl+wheel zooms about the pointer, a middle-button drag pans. Phase 2: drag a shape; marquee; drag a resize handle; drag from a connect handle onto another shape; double-click, type, Escape; the real keys `r`, `Tab`, `ctrl+Right`, `alt+Right`, `BackSpace`, `ctrl+z`, `ctrl+c`, `ctrl+v`; a letter typed into the open palette arms no tool; the first four nodes of the acceptance script. Phase 3: right-click opens the actions palette; a digit recolours the selection.

### 6.2 Harness requests

The new requests use the existing envelope and rectangle shape (`test_requests.rs:50`). Frames are capped at 1 MiB (`test_ipc.rs:16`), so `board.get` can omit the elements.

```
board.get      {"elements": true}       -> SNAPSHOT
board.bounds   {}                       -> {"root": RECT, "viewport": {"x","y","zoom"},
                                            "nodes":   [{"id", "bounds": RECT, "visible": RECT, "label": RECT|null}],
                                            "edges":   [{"id", "points": [[x,y],...], "label": RECT|null}],
                                            "handles": [{"id", "handle": "nw|n|...|connect_right", "bounds": RECT}],
                                            "label_editor": RECT|null}
board.key      {"key": "tab", "mods": ["mod","shift"], "repeat": false}   -> SNAPSHOT + {"handled": bool}
board.type     {"text": "Receive order"}                                  -> SNAPSHOT
board.pointer  {"kind": "press|move|release|click|double_click|wheel",
                "at": {"node":"n1"} | {"node":"n1","handle":"se"} | {"edge":"e1"} | {"canvas":[x,y]} | {"screen":[x,y]},
                "button": "left|middle|right", "mods": [], "delta": {"x":0,"y":-3,"unit":"lines|pixels"}}  -> SNAPSHOT
board.apply    {"edits": [{"op":"add_node","id":"n9","shape":"rect","at":[0,0],"size":[120,60],"text":"A"},
                          {"op":"move","ids":["n9"],"by":[20,0]}, {"op":"set","id":"n9","field":"text","value":"B"},
                          {"op":"connect","id":"e9","from":{"node":"n1"},"to":{"node":"n9"}}, {"op":"delete","ids":["e9"]}]}  -> SNAPSHOT
board.export   {"format": "svg|png", "path": "/abs/out.svg"}              -> {"path", "bytes"}
perf.board     {"clear": true} | {"pan": {"steps":60,"dx":8,"dy":0}} | {"zoom": {"steps":30,"factor":1.05}}
                                        -> {"scene_micros":[...], "draw_micros":[...], "frame_micros":[...], "open_micros": n}

SNAPSHOT = {"enabled": true, "surface": "board|raw|text"|null, "note": 7, "read_only": false,
            "error": null | {"line": 12, "message": "..."}, "issues": [{"kind":"dangling_edge","id":"e9"}],
            "state": "unfocused|idle|armed|pressed|moving|marquee|resizing|creating|connecting|editing|panning",
            "tool": "select", "selection": ["n1"], "cursor": "n1", "editing": null | {"id","text","caret"},
            "connecting": null | {"target": "n4"|null, "query": "che"},
            "viewport": {"x","y","zoom"}, "can_undo": false, "can_redo": false, "pristine": true, "revision": 0,
            "counts": {"nodes": 15, "edges": 17},
            "nodes": [{"id", ...schema-1 fields by name}], "edges": [{"id", ...schema-1 fields by name}]}
```

- `board.key` takes the path a real key takes: the trigger goes to `board_policy`, a `Command` becomes `Message::Command` and reaches `dispatch`. So the acceptance test tests the bindings. `handled` is false when the policy answers `Host` or `Pass`, when a palette or dialog is open, and when `note-editor` is not focused (the check the Editor branch of `shortcut` makes, `test_requests.rs:92`). `"mod"` means ⌘ on macOS and Ctrl on Linux, so one script runs everywhere. `board.type` feeds text to the label editor or the connect query; `board.pointer` builds an `Input` and calls `App::board_edit`. The `shortcut` request gains a Board branch that routes a command's bound trigger the same way.
- `revision` counts the drafts this board has sent plus its reloads; tests wait on it. `pristine` is true when the model equals the pristine model.
- `board.apply` runs through `plan` and `board_changed` (one undo step, one draft) and takes explicit ids. It is for arranging fixtures and is never the thing under assertion. `core.apply` keeps refusing draft edits (`test_requests.rs:892`).
- `board.export` runs the function the Export command runs, without the save dialog, as `test.folder.complete` stands in for the folder picker (`test_requests.rs:499`).
- Any `board.*` request other than `board.get` returns an error when the surface is not a board. Existing requests cover the rest: `command.run` and `shortcut` for commands, `palette.*` for the actions and jump modes, `session.get`, `test.fail_next`, `test.timer.hold`, `window.blur`, `app.quit`, `app.screenshot`, `clipboard.read` and `clipboard.write`.
- Phase 1 ships `board.get`, `board.bounds`, `board.export`, `perf.board`, and `board.pointer` limited to `wheel` and a middle-button pan. Phase 2 ships the rest. A chain that changes a request shape edits this block in the same commit.

### 6.3 Scenes

`tools/capture_scenes.py` gets `--only boards` (the choice list is at `:222`), which opens the sample workspace with `boards = on`. Step 0 gives the script a fixed workspace path under the worktree's `target/`: today it uses a temporary directory (`:243`) whose path is drawn in the header, so two captures of one unchanged build differ in all 33 files. Scenes are shot in `dark` and `light` on both renderers through Phase 2, and in all eleven themes at the end of Phase 3 by looping `capture_scenes.py --only boards --theme <id>` over the theme ids (`theme.set` takes any built-in id, `test_requests.rs:286-288`). A reviewer who "looks at" scenes lists each file and what it shows in `verified_ok`.

| Phase | Scenes |
|---|---|
| 1 | `board-flow` (fit to view), `board-zoom-in` (200 %), `board-zoom-out` (25 %), `board-shapes` (placeholder and stub visible), `board-overlap`, `board-raw` (conflict markers, banner), `board-notes-list` (kind icon), `board-search` (label hit), `board-wide` (columns folded), and `board-flow.svg` written beside the PNG |
| 2 | `board-new-dialog` (kind pills), `board-selected` (handles and cursor ring), `board-marquee`, `board-editing` (label editor open), `board-armed` (mode chip), `board-connecting` (keyboard target and query), `board-edges` (arrowheads, dashes, labels, both routes), `board-built` (the flowchart the acceptance script produced) |
| 3 | `board-colors` (every colour at every fill level), `board-stacking`, `board-actions`, `board-jump`, and `board-flow.png` from the PNG exporter beside the window capture |

### 6.4 What is tested and what can be seen, by phase

| Phase | Tested automatically | Seen by hand in a local Linux build |
|---|---|---|
| 1 | `wren-board` and `wren-core` tests; app tests for the seam and guard; `test_board_view.py` and `test_board_gate.py` in all four lanes; three native X11 cases; pixel checks on both renderers; the scenes; the spike's numbers | Open the sample workspace; see shapes, labels, connectors; pan with the wheel, middle-drag or Space-drag; zoom with Ctrl+wheel and Alt+=; switch themes; open the raw fallback; search for a label; run Export SVG and open the file in a browser; confirm that with the setting off the boards are simply absent |
| 2 | Machine and key-policy tables; `test_board_edit.py`, `test_board_keys.py`, `test_board_safety.py`, `test_board_dialog.py` in all four lanes; the native X11 cases; the scenes | ⌘N, type, Tab, Enter; build a flowchart with the mouse and with the keyboard; undo and redo; copy and paste within and between boards; quit and reopen; `git diff` the file after one move; edit the file in another editor while it is open, clean and dirty |
| 3 | Colour, contrast and colour-blindness tests over eleven themes; one harness file per chain; `wren export` with no display; limits tests; budgets; the strict docs build; scenes in every theme | Recolour with digits, restack with `]` and `[`, align; the actions palette with live preview; jump to a node; land on a search hit; export PNG; `wren export` in a terminal; read the docs site locally |

**The limits of each platform.**
- Synthetic pointer and key input exists only on X11 (xdotool under Xvfb). The Wayland lanes run everything not marked `native_x11` (`Makefile:121`), so there the board is driven only through `board.key`, `board.type` and `board.pointer`; that is why those requests inject at the machine's seam. Headless Weston has no touchpad and no input method; neither has Xvfb. `Pixels` scroll deltas and composition are covered by constructed events in unit tests and otherwise need a real session.
- Agent sessions have no display, so wgpu runs on software Vulkan (lavapipe) and says little about a real GPU.
- Over ssh the Mac can build, run the Rust tests, capture scenes against a real Metal window while a user is logged in at the console, and run `wren export`. It cannot send keys or pointer input (`docs/development/testing.md:108-113`), and it has no pytest, so the functional board tests are Linux-only; the `board.*` requests still work there, so Mac scenes include interactive states. macOS-only code (`macos_menu.rs`, `menu_command`; `main.rs:9-10`, `preferences.rs:487`) is type-checked on Linux by the cross-target lint of §6.6 and runs only on the Mac or in CI after a push.
- A pixel check shows that something of the right colour was drawn in the right place, not that it looks right. Looking at the scenes is part of the gate.

**Checks that wait for the owner at the Mac console.** They do not block a phase tag; they are recorded as outstanding in §8.5 and required before the merge to `main`. Checks 5 and 7 can be made after Phase 1, checks 3 and 4 after Phase 2, the rest after Phase 3. (1) Board menu: absent with the setting off, board-only items greyed on a Markdown note, each item works, only ⌘ triggers show accelerators. (2) In a label and in the palette input, `]`, `[`, `/` and the digits type their characters. (3) ⌘ keys on a board: D, A, ⇧A, C, X, V, Z, ⇧Z, ⌘+Arrow, ⌘↵; ⌥+Arrow moves; ⌘[ and ⌘] still go Back and Forward; ⌘=, ⌘- and ⌘0 zoom. (4) Input method in a label: the candidate window sits at the label at zoom 0.5 and 2; a composition survives Enter and Tab; with a CJK input source active, tool letters still work outside a label. (5) Trackpad: two fingers pan, ⌘+scroll zooms at the pointer, momentum does not turn a pan into a zoom. (6) Right-click and Ctrl-click open the actions palette on the element under the pointer. (7) Retina: shapes and labels are sharp; pan and zoom of the 1,000-element sample are smooth on Metal. (8) Export through the save dialog writes SVG and PNG that open in Preview and a browser; `Wren.app/Contents/MacOS/wren export` in Terminal writes the file with no window and no Dock icon. (9) Switching the system appearance recolours an open board. (10) "Check for Updates…" is still enabled after the menu change.

### 6.5 Trying it yourself

All recipes run in `.claude/worktrees/boards`; the main checkout is on `main` and has neither the fixtures nor the feature. Hand trials use their own target directory so they never replace the binary a gate is using. The setting is one line, `boards = on`, in `~/.config/wren/config.conf` on both platforms (`settings.rs:5-6`); the recipe uses a throwaway config directory instead. On macOS the workspace registry does not follow `XDG_CONFIG_HOME` (`config.rs:54`, `:951-952`), so there this adds the sample folder to the real registry.

```sh
cd .claude/worktrees/boards
rm -rf /tmp/wren-try && mkdir -p /tmp/wren-try/config/wren
cp -r tools/wrentest/fixtures/boards/workspace /tmp/wren-try/work
printf 'boards = on\n' > /tmp/wren-try/config/wren/config.conf
export CARGO_TARGET_DIR=$PWD/target/try XDG_CONFIG_HOME=/tmp/wren-try/config
make run ARGS="--workspace /tmp/wren-try/work"                          # wgpu
ICED_BACKEND=tiny-skia make run ARGS="--workspace /tmp/wren-try/work"
# Phase 2: (cd /tmp/wren-try/work && git init && git add -A && git commit -m base), then git diff after each edit
# Phase 3: cargo run --locked -p wren-iced -- export "/tmp/wren-try/work/Boards/Samples/Checkout flow.wren-board" -o /tmp/wren-try/flow.svg
```

Delete the `boards` line and run again to see the setting-off behaviour on the same folder.

Scenes and the board tests alone. Scenes land in `target/scenes/<label>/<theme>/board-*.png`. `SMOKE_TESTS` is a new Makefile variable replacing the fixed `tools/wrentest` at `Makefile:118` and `:121`; it defaults to that path, and no glob is written into the Makefile, because the CI contract test requires every `tools/` path there to exist (`tools/wrentest_unit/test_ci_contract.py:54`). Use a path glob, not `-k board`, which would also select every "keyboard" and "clipboard" case.

```sh
make build-smoke
for r in tiny-skia wgpu; do for t in dark light; do
  WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1 ICED_BACKEND=$r python3 tools/run_display.py x11 -- \
    python3 tools/capture_scenes.py --only boards --theme $t --label $r
done; done
cargo test --locked -p wren-board && cargo test --locked -p wren-canvas && cargo test --locked -p wren-core kind
ICED_BACKEND=tiny-skia make smoke-x11 SMOKE_TESTS='tools/wrentest/test_board*.py'      # and smoke-wayland
WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1 ICED_BACKEND=wgpu make smoke-x11 SMOKE_TESTS='tools/wrentest/test_board*.py'
```

### 6.6 Gates and operating rules

**Every commit** passes `make check` (`Makefile:104`). The `clippy` target gains `cargo clippy -p wren-board` and `-p wren-canvas` on their own, as `wren-editor` has (`Makefile:99-102`), so neither crate leans on features the app unifies in. `make test` runs every workspace test twice (`Makefile:33-34`) and workspace crates build unoptimised (`Cargo.toml:61-63`), so default-run test counts are budgeted: `cargo test -p wren-board` stays under about 10 s and the state-file walk under about 5 s in a debug build. The full counts are `#[ignore]` tests behind two new targets, both in release: `make fuzz-board` (property tests at full count) and `make perf-board` (timings over 1,000 and 10,000 generated elements, as `make perf-search` runs its own, `Makefile:39-41`).

**Every chain**, before its review and again before its merge:
- `make check`.
- The board test files on X11 tiny-skia and on Wayland (§6.5).
- The whole suite, `ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11`, if the chain touched `wren-core`, `app.rs`, `app/workspace.rs`, the keymap or the harness.
- `cargo clippy --locked --target aarch64-apple-darwin -p wren-iced --all-targets --features smoke-test -- -D warnings` if the chain touched `keybind.rs`, `app/commands.rs`, `app/preferences.rs` or `macos_menu.rs`. It type-checks and lints the macOS-only code from Linux (35 s cold on `main`; the target is installed) and is deliberately not part of `make check`, since CI's Linux runners lack the target.
- `make fuzz-board` and `make perf-board` if the chain touched `wren-board`.
- The scenes for anything that draws, captured and listed.
- `uv run --locked --group docs zensical build --strict` if `docs/` changed.

The reviewer re-runs these and does not trust the implementer's report. After each merge onto `boards` the orchestrator runs `make check` and the X11 tiny-skia suite, and only when no session is using that worktree. Phase-end gates are in §8.2.

**Operating rules**, stated in every brief:
- One session per worktree at a time. Never start a smoke run in the background and keep building.
- `make check`, `make build`, `make run`, and any `cargo test` or `cargo run` without `--features smoke-test` are *normal builds*: they replace `target/debug/wren` with the non-smoke binary, and every harness launch then fails with "unsupported or duplicate option". (The first `cargo test` of `make check`, `Makefile:33`, does this until the second, `:34`, restores it.) Safe beside a smoke run: `make clippy`, `make package-debug`, `make perf-search`, and anything with its own `CARGO_TARGET_DIR`.
- Run lanes one at a time; the 120 s per-test timeout (`pyproject.toml`) does not survive several software renderers competing for the machine.
- Never hand-merge `Cargo.lock`: take one side, run `cargo check` once without `--locked`, then the gates.
- Only the orchestrator uses the Mac, after the spike and at each phase end: it has one checkout directory, so two sessions must never sync to it at once. Sync the tree without `target/`, `.venv/` and `.claude/`, run `make build fmt-check clippy test check-patches` (the CI line, `ci.yml:26`), the smoke build and `capture_scenes.py --only boards`, and copy the scenes back. The Mac's host name and paths stay in the owner's private notes.
- If `boards` is pushed, CI runs on every push (`ci.yml:3`). The CI matrix does not grow during the build; `tools/wrentest_unit/test_ci_contract.py` pins the workflow's shape.
- Never `git stash`. Without the owner: no push, no new dependency, and no change to the default of `boards` before item R3.

**The setting in the harness.** `WRENTEST_BOARDS=off|on` is read by `WrenApp` (`tools/wrentest/ui.py`), which writes an explicit `boards = off` or `boards = on` line into the sandbox `config.conf` of every launch, so its meaning does not change when the default flips. It defaults to `off` until item R3 and to `on` after. The board test files set the setting themselves. An `off` run is therefore the legacy suite plus the board cases, and an `on` run proves the setting alone disturbs no Markdown behaviour.

### 6.7 Performance

`perf.view` times `App::view` only (`app.rs:644-657`), the right probe for idle cost: the 25 ms tick (`app.rs:796`) rebuilds the view about forty times a second, and the editor effort's one blocker was work done there. `perf.board` adds three rings of 256 samples behind the `smoke-test` feature. `scene_micros` is model to display list. `draw_micros` is the time inside the widget's `draw`, which on wgpu includes tessellation. `frame_micros` is the interval between successive `RedrawRequested` pulses while a `pan` or `zoom` run applies one step per pulse, using the barrier `app.screenshot` uses (`test_capture.rs:23`, `:74-83`); the pulse is the start of a redraw, not a presented frame, and under Xvfb the interval spans the whole update, view, draw and present cycle. `open_micros` runs from the `OpenNote` op to the first pulse after the board's first `draw` returned. On a real display iced presents with vsync by default (`iced_core-0.14.0/src/settings.rs:53`, `iced_wgpu-0.14.0/src/settings.rs:47`), so the interval has a 16.7 ms floor at 60 Hz: wgpu is measured with `ICED_PRESENT_MODE=immediate` (`iced_wgpu` `settings.rs:64-73`) or judged on `draw_micros`.

| Measure | How | Release budget (the spike confirms or replaces it) | Debug ceiling asserted in every lane |
|---|---|---|---|
| Parse, write, scene build, in `wren-board` alone | `make perf-board` | 10,000 elements: parse 75 ms, write 10 ms; 1,000 labelled shapes: scene 5 ms | none (release only) |
| Frame build, 1,000 labelled shapes, each renderer | `scene_micros` and `draw_micros` after one edit | scene + draw 8 ms | 100 ms median |
| Pan and zoom | median `frame_micros` | 33 ms on tiny-skia (11 to 18 ms measured before text); on wgpu, `draw_micros` 8 ms | 400 ms median |
| Load of a large board | `open_micros` at 1,000 and 10,000 elements | 150 ms and 1 s | 5 s |
| Idle | `perf.view` median over 30 views with a 1,000-shape board open; `scene_micros` gains no sample over one idle second | view 1 ms; zero scene builds | 5 ms (the editor's ceiling, `test_markdown_keys.py:574`) |
| One label keystroke at 1,000 nodes (Phase 2) | canonical write plus the Session op with its index parse | 8 ms (§3.4 rule 7) | none |
| Phase 3 additions | style, stacking and align on 1,000 selected nodes; one actions-palette preview step; PNG at scale 2 of 1,000 elements; `wren export` cold start | 50 ms; 16 ms; 1 s; 1.5 s | none |

The debug ceilings live in `test_board_perf.py`, run in every lane, write `measurements.json` beside the other artifacts, and catch only order-of-magnitude regressions. The release numbers come from a separate build so the smoke binary is not replaced, using the harness's `WREN_TEST_BIN` override (`tools/wrentest/ui.py:94-95`); `WRENTEST_PERF=release` tells the test to assert the release budgets:

```sh
cargo build --locked --release -p wren-iced --features smoke-test --target-dir target/perf
WRENTEST_PERF=release WREN_TEST_BIN=$PWD/target/perf/release/wren ICED_BACKEND=tiny-skia python3 tools/run_display.py x11 -- \
  uv run --locked --group test pytest tools/wrentest/test_board_perf.py -s
```

A missed budget is fixed or recorded with its number and reason; none is silently relaxed. Workspace open and `make perf-search` must be unchanged within noise against `main`, with the setting off and on.

### 6.8 Acceptance tests

Each item is one harness test unless it names another layer. A phase is done when all of its items pass in the phase-end gates.

**Phase 1, "See a board".**
1. *Setting off.* A workspace with `Flow.wren-board` and `Note.md`, `boards = off`: `session.get` lists only the note; no palette row for `note.new_board` or export; `board.get` reports `enabled: false`; the board's bytes and mtime are unchanged after quit. The whole existing suite passes unmodified.
2. *Opens.* With `boards = on`, the board is listed with the title `Flow`, `surface` is `board`, the counts match the fixture, and after fit every node has a non-empty `visible` rectangle.
3. *Renders on both renderers, dark and light.* In a screenshot, a point at least 4 px inside a filled shape is its fill colour within 8 per channel, the midpoint of a connector is its stroke colour within 24 per channel (it is antialiased), and a label rectangle contains glyph pixels (the method of `test_editor.py:138-150`).
4. *Pan and zoom.* A wheel step moves every `board.bounds` rectangle by the same delta. Mod+wheel changes `zoom` and keeps the canvas point under the pointer within 1 px. `font.increase` steps the zoom and leaves `editor-font-size` alone. The native X11 case does the wheel cases with the real wheel.
5. *Never writes on open.* For `Checkout flow` and for `Hand written`: bytes and mtime are identical, and `document.dirty` is false, after open, pan, zoom, theme switch, export, note switch, `window.blur` and quit.
6. *Single writer.* With a board open, `editor.insert`, `editor.command {"intent":"bold"}`, `command.run markdown.bold` and an engine switch are refused or blocked, and one second later (past the 750 ms autosave debounce) the file is unchanged.
7. *Raw fallback.* A conflict-marker board opens with `surface: raw` and an `error` naming a line; a text edit saves exactly what was typed; fixing the file on disk and reconciling returns `surface: board`. `Future` reports `read_only: true`. `Shapes` lists its issues.
8. *Export SVG.* The output parses as XML, has one text run per wrapped label line, and escapes a label of `a < b & "c"`.
9. *State file.* After open, back, forward, quit and restart, `history`, `recent` and `last_open_note` hold no board path, the board reopens, and the frozen validator (core test) accepts the file. *Live:* a v0.1.1 binary built with `--features smoke-test` in a temporary worktree and this build, driven in turn over the harness with `WREN_TEST_BIN`, each open the same workspace and keep its order, sort and widths (run at the phase end; skipped when no old binary is given).
10. *Search.* Searching a label word returns the board with a label snippet; searching `schema` does not.
11. *Core.* The op-by-kind table and the index counters pass (§6.1).

**Phase 2, "Build a flowchart".**
1. *The keyboard-only 15-node flowchart* of §5.20, through `board.key` and `board.type` on a new empty board, in all four lanes; the native X11 case replays the first four nodes with real keys.
2. *Saved-file diffs* (`test_board_edit.py`). Each row is flush, keep bytes A, act, flush, keep bytes B, then a line diff:

| Action | Expected diff |
|---|---|
| Open, select, walk, pan, zoom, arm and disarm a tool, open and close the label editor unchanged | none; mtime unchanged; `dirty` false |
| Move one node (Alt+Right; a `board.pointer` drag; a real drag on X11) | one line, that node's `at`; no edge line |
| The inverse move, or undo | bytes equal A |
| Move then undo on `Hand written`, waiting past the debounce | bytes equal the original non-canonical file; `dirty` false |
| Edit a label | the `text` line, plus `size` only if it grew |
| Add a node; add a connector | one new block and nothing else |
| Delete a node with two connectors | three blocks removed and nothing else |
| Paste a two-node, one-connector fragment | three new blocks with new ids; a second paste, three more |
| Reload the saved file and save again after a no-op | none |

3. *Settle before op.* Type into a label, then at once (a) open another note, (b) `window.blur`, (c) `app.quit`: the text is on disk each time. With `test.fail_next {"fault":"write"}` the note switch is blocked and the draft is kept.
4. *External change.* Clean: the board reloads; the selection (by id) and the viewport are kept; `can_undo` is false. Dirty: a conflict copy named with `.wren-board` appears. Mid-drag: the gesture is cancelled, `state` is `idle`, and releasing the button afterwards does nothing.
5. *Undo.* Each gesture is one step; N gestures and N undos return the original bytes.
6. *Mouse.* Create, select, marquee, move with snap, resize, drag-connect, double-click edit and delete, through `board.pointer` in every lane and through xdotool on X11.
7. *Keys.* A letter typed in the palette arms nothing; a letter typed in a label or in the connect query is text; held Tab and Delete act once; Esc descends one rung per press and never leaves the board; every App shortcut pressed on a focused board acts exactly once; Alt+Up in the editor still moves a line.
8. *Copy and paste.* The clipboard text parses as a board fragment; a paste creates new ids at an offset; it survives a restart.
9. *New note dialog* (`test_board_dialog.py`). ⌘N, type, Enter creates `.md`; ⌘N, type, Tab, Enter creates `.wren-board`; Shift+Tab returns to Markdown; no pills with the setting off or in Rename; `note.new_board` preselects Board.
10. *Schema lock* (§4.17). The format golden files are frozen.

**Phase 3, "Finish and harden".**
1. *Colours* (core unit tests beside the existing one at `theme.rs:566`): for every theme, colour and fill level, label on fill is at least 4.5:1 and stroke on canvas at least 3:1; the colour-blindness rule of item ST3 holds.
2. *Stacking, align, distribute.* Model tests for each command. Each stacking command changes one `z` line per moved element.
3. *Actions palette.* It lists only actions enabled for the selection, previews live, and restores on Escape (the pattern of `test_editor.py:617`); a preview never reaches the draft.
4. *Jump and reveal.* Activating a search hit, or a jump row, selects the element and places its centre within the middle third of the canvas.
5. *PNG export.* The file decodes with `tools/wrentest/png.py`, its size is the scene bounds times the scale, and each shape centre has its fill colour within 8 per channel, in the PNG and in a tiny-skia window capture of the same board.
6. *`wren export`.* Run with no display set: exit 0 and bytes equal to the in-app export; a missing or unparseable input exits non-zero with a message and writes nothing.
7. *Limits.* An oversized board opens as raw text; out-of-range coordinates and over-deep parents are flagged; the full mutation run never panics the loader.
8. *Setting, docs, performance, macOS.* The eight lane runs, the strict docs build and the §6.7 budgets all pass; the menu table has a unit test that runs on the Mac over ssh and in CI.
9. *draw.io* (only if the optional chain runs). A golden XML file per sample, well-formed, and one manual open in draw.io.

## 7. Phases

### 7.1 Execution

**Step 0** is the first session of the first run. It has an owner so that nothing later assumes it happened.
1. *Preflight* (no commit): software Vulkan present for the local wgpu lanes, `xvfb-run`, `xdotool`, `weston` and `dbus-run-session` installed, the `aarch64-apple-darwin` rustup target installed, the Codex CLI logged in, the Mac reachable over ssh with a console session. A missing item is reported, not worked around.
2. `boards` was cut from `main` at `393ae14`, the worktree `.claude/worktrees/boards` created and this document committed as `discovery/boards.md` when the plan was written (2026-11-12). Confirm the worktree exists, is on `boards` and is clean.
3. *Capture determinism:* give `capture_scenes.py` its fixed workspace path, confirm that two captures of one unchanged build now match byte for byte, and keep the `main` captures as the baseline for exit criterion 3(d).
4. *The `canvas` feature:* add `"canvas"` to the workspace iced features (`Cargo.toml:19-30`) and commit `Cargo.lock`, as its own commit that can be reverted alone. Check it is inert: `make check`, the X11 tiny-skia suite, scenes byte-identical before and after, `make package-debug` still assembles notices. Record the before and after of `tools/dependency_notices.py`; the five lyon crates ship no licence file, so add their texts under `third_party/lyon/`, list them in `LICENSE_NOTICES` (`tools/package.py:65`) and update `tools/wrentest_unit/test_package.py`.
5. *The Phase 1 contract commit:* both crate skeletons with `[lints] workspace = true`; the public types of §3.2 that Stage A shares (`Board` and its parts as declared types, `Palette`, `Scene`, `Item`, `Draw`, `Measure`, `FixedMeasure`); the workspace members and dependencies; the two `clippy` lines; the `fuzz-board` and `perf-board` targets and the `SMOKE_TESTS` variable in the `Makefile`; the `.gitattributes` line; the `CLAUDE.md` Layout entries for both crates. After it, no Stage A chain edits the root `Cargo.toml`, `Cargo.lock`, `Makefile` or `CLAUDE.md`.
6. *Fixtures:* copy the reference writer to `tools/board_format.py` and the example to `golden/format/password-reset.out.wren-board`; generate the canonical boards of the sample workspace (§6.1) and the format golden outputs of §4.16 with it, `Checkout flow` from the keyboard model's layout; commit them. The Rust writer must later reproduce every one byte for byte.

**Workflow shape.** Each phase is three orchestrated runs, split where the state on disk is clean, because a workflow cannot be resumed from a new session, Phase 1 has a stop point inside it, and the Codex pass and its triage come before the next phase. Each run starts from a named commit or tag and ends by writing `.claude/board-notes/HANDOFF.md`: head, gates with pass counts, what is merged, what waits on the owner.

| Run | Starts from | Does | Ends |
|---|---|---|---|
| 1a | `main` | Step 0; Stage A (Core, Model, the spike) in parallel; merge Model then Core; the spike record | Stop: the owner reads the spike decision |
| 1b | the 1a head | The viewer; then the App chain | Both merged |
| 1c | the 1b head | Phase end (§8.2) | Tag `boards-p1` |
| 2a | `boards-p1` | Contract commit; Wave A (edit, canvas, keys) in parallel; merges | Wave A merged |
| 2b | the 2a head | The wiring chain; then the connector chain | Both merged |
| 2c | the 2b head | Phase end; the schema lock (§4.17) | Tag `boards-p2` |
| 3a | `boards-p2` | Contract commit; Wave A (style, arrange, find, export) | Wave A merged |
| 3b | the 3a head | Wave B (harden, release, optional draw.io, then docs) | Wave B merged, setting on |
| 3c | the 3b head | The final review (§8.2) and the readiness assessment | Tag `boards-p3` |

**Scripts.** Two generic scripts in `.claude/board-notes/workflows/` run all nine runs; each run is that script plus an arguments file in `.claude/board-notes/workflows/runs/` (`1a.json` to `3c.json`) that names the chains, their sessions, the model tier of each session, the stages and the merge order.
- `boards-build.js` runs the a and b runs. Per stage it creates the chain worktrees, runs each chain (implementing sessions in order, each handed the earlier reports; a fresh adversarial review; a fix session; a verify session; a second round if a blocker is still open), then merges the ready chains in the stated order, each merge followed by `make check` and the X11 tiny-skia suite. A required chain that cannot merge stops the run.
- `boards-phase-end.js` runs the c runs: the setting-off gates, the lenses and the Codex pass in detached worktrees, triage, one fix session, a verify session, the final gates, the Phase 2 schema lock, and the record in §8.5. It does not tag: the orchestrating session runs the Mac step and tags.
- The orchestrating session passes the arguments file's content as the workflow's `args`, saves the returned result under `.claude/board-notes/workflow-results/` and writes the handoff note. The briefs are short on purpose: this document is the contract, and a brief names the work items and sections to read.

**Models** (decision 12). Every session names a tier, and each arguments file maps the three tiers to models in one place (`"tiers": { "hard": "fable", "main": "opus", "light": "sonnet" }`), so the split can be retuned without touching a script. The orchestrating session itself runs on Opus.

| Tier | Model | Sessions |
|---|---|---|
| hard | Fable | Phase 1: V1 (the spike), V2 and V3 (measurer and widget), A1 to A4 (the surface seam and the single-writer guard). Phase 2: I1 and I2 (the interaction machine), I4 (the label editor), W1 to W4 (one writer, settle, Session-driven change, undo), X1 (pointer connectors). Every phase end: the data-safety lens. Ten sessions in all. |
| main | Opus | Every other implementing session, step 0 and the contract commits, every per-chain review, fix, verify and merge, the other three lenses, the Codex triage, the phase-end fix and record. |
| light | Sonnet | Sessions that only run commands and report: creating worktrees, the gate runs, scene capture across themes. |

The reasons: Fable goes where the work is novel against iced's limits or where a subtle mistake loses a user's work; everything specified tightly enough in §3 to §5 runs on Opus; Sonnet is kept for plainly mechanical sessions. To run with no Fable at all, set `hard` to `opus`; to avoid Sonnet, set `light` to `opus`. A chain whose review finds a blocker twice on Opus is a candidate for one Fable session, at the orchestrator's judgement.

**Chains.** A chain is a branch and a worktree of the same name under `.claude/worktrees/`, named with its phase (`boards-p1-core`), with hyphens because git cannot hold `boards` and a branch under `boards/` at once. Each chain is: implementing sessions in the listed order, each handed the previous session's report, in small commits that each pass `make check`; a fresh adversarial review; a fix session; a verify session (§8.1). Merges are `git merge --no-ff` in the stated order. A chain that merges after another first merges `boards` into itself and re-runs its gates before its verify step. A merged chain's worktree `target/` is deleted.

**Resuming.** A run that dies is resumed by hand from disk: for each chain worktree, `git log --oneline boards..HEAD` and `git status --short`; committed work stays (confirm it passes `make check`); uncommitted work is inspected, then finished or discarded; then the chain's next step runs from its brief.

### 7.2 Phase 1, "See a board"

**Goal.** With `boards = on`, a hand-written `.wren-board` file in a workspace opens, renders shapes, labels and connectors on wgpu and tiny-skia, pans, zooms and exports to SVG. With the setting off nothing changes. The board sends no draft at all in this phase.

| Stage | Chain | Sessions | Depends on |
|---|---|---|---|
| A | Core, `boards-p1-core` | C1 to C3; C4, C6, C7; C5 | the contract commit |
| A | Model, `boards-p1-model` | M1; M2, M3; M4; M5 to M7 | the contract commit; §4 |
| A | Canvas, `boards-p1-canvas`: the spike | V1 | the contract commit |
| B | Canvas: the viewer | V2, V3; V4 and the confirmation | Model and Core merged; the spike decision |
| C | App, `boards-p1-app` | A1 to A4; A5 to A7; A8 | everything above merged |

Merge order: Model, Core, Canvas (which merges `boards` into itself before stage B, so its meeting with Core is resolved and gated inside the chain), App. The reviewer of the Model chain also reviews the step 0 commits (`git diff main...boards` at the fork). Expected conflicts after the contract commit: `app/test_requests.rs` (Core's `create_note` decoder and the spike's `perf.board`; different hunks) and this document (each chain writes only its own subsection of §8.5).

**Core chain.**

| # | Change | Tests |
|---|---|---|
| C1 | `kind.rs` and `Note.kind` (§3.5). No behaviour change yet: `Board` is never produced by a scan. | Table: `a.md`, `A.MD`, `.md` (none), `x.wren-board`, `x.WREN-BOARD`, `x.wren-board.md` (Markdown), `x.md.wren-board` (board titled `x.md`), `wren-board` (none). |
| C2 | `fs_store.rs`: `Leaf` in `relocate`, `sanitized_name`, `candidate_name`; `create_file` takes a kind. `sanitized_name` strips only that kind's repeated suffix. | Existing tests pass with `NoteKind::Markdown`. Board rows in the name tables (`fs_store.rs:1383-1400`); a stem over `MAX_NAME_BYTES` (240, `:22`) truncates on a character boundary with the 11-byte extension still attached; `Flow.md` beside `Flow.wren-board`. |
| C3 | `ops.rs`: `Op::CreateNote { group, title, kind }`, `relocate`, `create_conflict` (§3.5). | The op-by-kind table (§6.1). |
| C4 | `SessionOptions`, `Session::open_with` beside `open` (`ops.rs:254-262`), `scan::Kinds` through the three scans; the three predicates (`scan.rs:129`, `:236`, `:336`) ask `Kinds`. `Session::open` is unchanged and scans Markdown only. A board at the root or directly in a section gets the "ignored" warning a Markdown file gets. | The gate test. Option on: titles and kinds. A non-UTF-8 board takes the existing path (`ops.rs:178`). |
| C5 | `workspace_state.rs`: the state-file rule (§3.5); the setters called at `ops.rs:881-883`, `:1129-1130`, `:1459-1460`, `:1628-1648`; `load` reads through the new accessors. | Unit tests for each rule; the frozen validator and the random walk (§6.1); inline state files including one written by a pretend older build (v1 keys changed, `[navigation]` stale); bytes identical to `main` with the option off (a recorded fixture). |
| C6 | `index.rs`: the `source` and `text` split (§3.6), with `searchable` returning `None` for a board until A1. | A test-only extraction counter: 200 unchanged boards, 40 ticks, zero extractions; the reconcile job re-sends zero notes; `revision()` is stable (extends the test at `index.rs:1451`). |
| C7 | `settings.rs`: `Settings.boards` (`:120-137`), default off, not written into `TEMPLATE` (`:388`). | Default off; `on`, `yes`, `true` parse (`:156-162`); a bad value warns; the template test (`:655`) still holds. |

**Model chain** (`crates/wren-board`; §4 is its contract).

| # | Change | Tests |
|---|---|---|
| M1 | Model types and `parse`. `ParseError` carries a line, a column and a message. | §4.16 tests 2, 7 and 9; accept and reject tables. |
| M2 | `write`, reproducing the step 0 golden outputs byte for byte. | §4.16 tests 1, 3, 4, 5 and 6. |
| M3 | `check` and `access`. Nothing is repaired or dropped. | §4.16 test 8. |
| M4 | `shape.rs` and `route.rs`: outlines, label boxes and anchors of the eight shapes; `straight` and `elbow` routes from two rectangles and their sides, a free end, arrowheads, dashes; the nominal-side rule of §5.12 rule 4. Records the final shape constants in §3.7. | The routing table (§6.1); a floating end is on its nominal side for every overlap-on-one-axis pair. |
| M5 | `style::resolve` and `scene::build`: everything in §3.7, including `kind = "text"`, the placeholder, the stub, the issue marker, clamped drawing and literal `#rrggbb`. Records the final label constants in §3.7. | Scene dumps in `golden/scene/`; parent-transform, z-order, wrap and overflow tables; a resolve table (each token, a hex, an unknown string, at each fill). |
| M6 | `svg::write`: one `<path>` per path item and one `<text>` per wrapped line with explicit positions, `font-family="Inter, system-ui, sans-serif"`; optional background and padding. | `golden/svg/*.svg`; a label of `a < b & "c"`; `</text>`, `]]>` and U+0000 to U+001F in labels. |
| M7 | `search_text`; the ignored release tests behind `make fuzz-board` and `make perf-board`; crate docs. | Order and content table; both targets run. |

**Canvas chain** (`crates/wren-canvas`).

| # | Change | Tests |
|---|---|---|
| V1 | The spike: a bare pan-and-zoom widget painting real `Scene` values from a generator (N rounded rectangles of 160 by 64 with two-line labels, every tenth with CJK and an emoji, an elbow connector to each right and lower neighbour, one shape in ten overlapping its neighbour's label); a `--board-spike N` mode beside `--editor-spike` (`cli.rs:46`, `app.rs:241`, the help text at `main.rs:86`); the `perf.board` request beside `perf.view` (`test_requests.rs:157`). Writes the record into its §8.5 subsection. | The spike's own measurements. Stop here. |
| V2 | `Measurer` over cosmic-text through `iced::advanced::graphics::text` (never a direct dependency; the path `wren-editor/src/backend/owned.rs:49-50` uses), cached by text, size and face. Copies the 24-label corpus from `.claude/board-notes/research/critic/md_labels.py` into the crate's fixtures. | Inter loaded from fixture bytes; a string's width equals the rendered paragraph's `min_bounds` (the method at `wren-editor/src/widget.rs:456`) within 0.5 px over the corpus; the label-fit test of §3.7. |
| V3 | `BoardView` (§3.2) and the widget: paints a `Scene` by the spike's strategy; culls to the viewport; holds geometry caches; splits layers by a pure function over scene items; implements `operation::Focusable` and `operate` with the host's id (the pattern at `wren-editor/src/widget.rs:544`, `:985-993`). Nothing is rebuilt while idle. The widget never requests an input method in this phase. | The layer-split table (overlap cases, cap honoured), culling, screen-to-board transforms. Counters under `test-support`: scene builds, geometry builds, layers. |
| V4 | Input: `Machine::step` with Idle and Panning only (§5.13 rules 1 to 4): wheel, Mod+wheel, the burst latch, middle-drag and Space-drag pan, `zoom_step`, `zoom_reset`, `fit`, the 10 to 400 % clamp. Arrow keys are captured and do nothing, so they neither move columns nor commit Phase 2 to a meaning. Then the confirmation step below. | The board point under the pointer is fixed by a zoom step within 0.01 px; latch behaviour; clamps; `fit` of an empty and of a one-shape board. |

**The spike.** Gate runs: 200 and 1,000 shapes, at scale factor 1 and 2, in release (`--target-dir target/perf`), on tiny-skia and on lavapipe under Xvfb. Debug and 5,000 shapes are run once each and recorded. Wanted and not gating: Metal on the Mac over ssh (the orchestrator) and a desktop run on the Linux machine (the owner).

| # | Question | Go | Otherwise |
|---|---|---|---|
| S1 | Is the `canvas` feature inert for existing screens? | Step 0's check passed. | Find the difference before anything else. Turning antialiasing off app-wide is not the implementer's call. |
| S2 | Text layering. Both renderers draw a layer's meshes, then its text (`iced_wgpu-0.14.0/src/lib.rs:865-881`; `iced_tiny_skia-0.14.1/src/geometry.rs:223`), and each wgpu layer with triangles costs three passes with MSAA on (`lib.rs:497-530`). Does opening a new `with_layer` only when a filled shape overlaps a label already drawn in the current layer give correct occlusion at a low layer count? | Pixel checks at named points of the generator's overlap case pass on both renderers; the cost of each extra layer is recorded on lavapipe. | Cap the layers at 4 and draw later labels in the last one (show-through past the cap, written down). If two layers already miss the budget: one layer, and drop a label fully covered by a later opaque shape. |
| S3 | Text under zoom. wgpu leaves `max_width` unscaled under a scaled frame and tiny-skia multiplies it (`iced_wgpu-0.14.0/src/geometry.rs:321`; `iced_tiny_skia-0.14.1/src/geometry.rs:215`), so each pre-wrapped line is drawn as its own unwrapped text. Is (a) a renderer transformation over cached geometry or (b) geometry rebuilt at the zoom with pre-scaled coordinates and font size sharp at rest? | Each line's ink bounds in a screenshot lie within 1 px times the zoom of the scene's `origin` and `width`, at zoom 0.25, 0.5, 1, 1.5, 2 and 4, on both renderers. Crops of a label at zoom 2 beside the same label laid out at twice the size are attached. Expected: (a) during a burst, (b) 150 ms after it ends. | If (a) misplaces text mid-burst, rebuild every step and re-measure S5. If neither is sharp, draw labels with `renderer.fill_text` outside the geometry and re-test S2. |
| S4 | Does the measurer agree with what is drawn? | Measured and drawn widths within 1 px over the 24-label corpus. | Add a safety margin to the wrap width and record it. |
| S5 | Performance, the draw and frame half of §6.7. | Release, 1,000 shapes: `draw_micros` at most 8 ms on a cache hit on both renderers; pan and zoom median `frame_micros` at most 33 ms on tiny-skia; idle view at most 1 ms with zero rebuilds. 200 shapes: p95 `frame_micros` at most 16 ms on tiny-skia. | 1,000 misses on tiny-skia: hide labels under about 4 screen px and skip label drawing during a burst, then re-measure. 200 misses after that: stop and report; the approach is wrong. 5,000 sets Phase 3's cap. |
| S6 | Focus and keys. Does a custom geometry widget take focus under `note-editor`, receive keys only when focused, and leave Escape and shortcuts to the app? | `focus.get` (`test_requests.rs:275`) names it; arrows are captured; a bound shortcut still runs. | Fix in the widget; this has no fallback. |
| S7 | Scroll units per platform. | X11 recorded with xdotool; Wayland and macOS taken from winit's source and marked unconfirmed; the pan constants chosen. | (none) |
| S8 | The label editor's mechanism (mandatory; Phase 2's item I4 is built on the answer). Candidates: (a) an editor owned and drawn by the canvas widget, a cosmic-text buffer with keys and `InputMethod` events handled in the canvas's own `update`; (b) a child text editor laid out over the node whose focus the canvas proxies under the one id. A text field pinned over the canvas as a second focusable widget is ruled out by §5.14. | A prototype types, takes a constructed preedit and commits; the caret and the candidate rectangle track the node at zoom 0.5 and 2; Escape leaves `note-editor` focused. The choice and its evidence are written down. | If neither candidate holds a composition, stop for the owner. |

The stop-point decision covers S1, S2, S3, S6, S8 and S5 as written. By the rules above the result is "go", "go with the named fallback" (recorded), or "stop"; only "stop" waits for the owner, but run 1a ends either way. **Confirmation step** at the end of V4, part of exit criterion 10: each `<text>` run of the exported sample SVG, rendered by resvg (a dev-dependency of `wren-iced`, which adds no package), has an ink width within 2 px of its `Line.width`; if not, Phase 3's PNG export converts labels to outlines or sets `textLength`. `scene_micros` and `open_micros` meet §6.7. The sample fixtures use at most 3 layers.

**App chain.**

| # | Change | Tests |
|---|---|---|
| A1 | `wren-core` depends on `wren-board`. `index::searchable` parses a board and returns `search_text`. `theme::board_palette(&Colors) -> wren_board::Palette`, with provisional hues derived from existing theme colours and named provisional in its doc comment. | A label word hits, `schema` and `kind` do not; the preview is the first label; the raw fallback; the C6 counter still reads zero; the template equals `wren_board::EMPTY`; every token resolves in all eleven themes. |
| A2 | `load_workspace` (`app/workspace.rs:204-230`) opens with `SessionOptions { boards }`; `reload_settings` (`app/preferences.rs:240`) shows the restart banner. | Off: no board notes in the model. On: present. |
| A3 | `board: Option<BoardView>`, `Surface`, `app/board.rs` and the board branch of `sync_editor` (§3.3), including the per-note viewport map and the open-fitted rule. | Each transition; every Session-driven text change emits an event; 40 ticks cause zero parses and zero scene builds; a reload keeps the viewport; typing in Raw does not flip the surface. |
| A4 | The single-writer guard (§3.3), `step_text_size` zooming on a board, and the table of `self.editor` sites in the chain's report. | `dispatch(Run(Bold))`, `editor.insert` and an engine switch on a board leave the Session text and, after 40 ticks at 1 s of session time, the file untouched. `font.increase` on a board changes the zoom and no setting. |
| A5 | View: mount the canvas or the editor under `note-editor`; `Message::Board`; board input sets the column (§5.15 rule 1); the footer and the Raw banner (§3.3); the thin adapter in `wren-iced/src/theme.rs` beside `editor_colors` (`:25`). | Harness focus cases; footer text for a clean, a flagged and a read-only board. |
| A6 | `chrome::Icon::Board` (`chrome.rs:8-31`), shown before the title in board rows only (`app/workspace_view.rs:576-583`). Markdown rows are pixel-identical to today. | Scene `board-notes-list`; existing notes-pane scenes unchanged. |
| A7 | `note.export_svg` ("Export board as SVG…", group "This note", App scope, no default key): enabled on Board only, not listed with the setting off. A save dialog in the pattern of `app/workspaces.rs:9-33`, then a temporary file and a rename; a failure is a banner. It exports what is on screen: the current theme's palette with a background rectangle. | `board.export`; §6.8 Phase 1 item 8; an unwritable path. |
| A8 | Harness and scenes: `board.get`, `board.bounds`, `board.export`, the wheel and middle-button `board.pointer`; `tools/wrentest/boards.py`; `test_board_view.py`, `test_board_gate.py`, `test_board_perf.py`, `test_board_native.py`; `WRENTEST_BOARDS` and `WRENTEST_PERF`; `capture_scenes.py --only boards`; the harness section of `docs/development/testing.md`. | §6.8 Phase 1 items 1 to 11. |

Two documentation pull-forwards happen in this phase and no others before Phase 3: the `CLAUDE.md` Layout entries (step 0) and the harness section of `docs/development/testing.md` (A8), so later sessions read a true map.

**Exit criteria.**
1. `make check` passes at the head of `boards`, including the two crate lint lines.
2. The phase-end gates of §8.2 pass: four lanes, each with the setting off and on.
3. **Setting off is zero behaviour change.** (a) The core gate test passes. (b) The whole pre-existing suite passes unmodified. (c) For the same op sequence, `.wren/workspace.toml` is byte-identical to the one `main` writes. (d) Pre-existing tiny-skia scenes are byte-identical to step 0's `main` baseline. (e) §6.8 Phase 1 item 1.
4. **Mixed versions.** The frozen v0.1.1 validator accepts the state file after every step of the kind table and the random walk; no v1 key ever names a board; a stale `[navigation]` loses to the v1 keys; a board named in `selection` reopens after restart; the live check with a v0.1.1 binary passes.
5. Every fixture opens as specified: `Checkout flow`, `Shapes` (issues listed) and `Overlap` as boards, `Hand written` as a board, `Future` read-only, a conflict-marker board as raw text with the error line.
6. Nothing is written on open, and the single-writer guard holds, the engine switch included (§6.8 items 5 and 6).
7. Both renderers draw it; pan and zoom work, including the three native X11 cases; the Phase 1 scenes exist for dark and light on both renderers and each is listed, with what it shows, in a review's `verified_ok`.
8. Export SVG and search work; the op-by-kind table and the index counters pass.
9. Idle is free: with a 1,000-shape board open, 40 ticks cause zero parses, zero scene builds and zero geometry builds, and `perf.view`'s median stays under the 5 ms debug ceiling.
10. The spike record and the confirmation step are complete; every §6.7 budget is confirmed or replaced in this document; the label editor's mechanism is recorded (S8).
11. The final scene constants are recorded in §3.7 and the label-fit test passes.
12. Each chain's review is recorded (§8.5) with no open blocker; an unfixed major has its reason.
13. The phase-end review and the Codex pass are done and their findings fixed or recorded (§8.2, §8.3); `boards` is tagged `boards-p1`.

**What a reviewer should try to break.**
- *Core.* Any path that still appends or strips `.md`: rename a board to `x.md`, to `x.wren-board.wren-board`, to a 250-byte title; move it into a group holding `Flow.md` and `Flow.wren-board`; dirty it, change it on disk, and read the conflict copy's name. Any op order that puts a board path in a v1 key: back and forward across kinds, delete the open board, rename a group holding the open board, quit with a board open, reopen with the setting off. Feed `[navigation]` garbage (wrong types, `../` paths, a 10 MB array) and confirm nothing disables the state. Compare state-file bytes against `main` with the setting off. Look for a scan call that did not receive `Kinds`.
- *Model.* Write black-box cases from §4 before reading the code. Find input that panics `parse` (deep nesting, huge integers, `at = []`, `size = [-1, 0]`, a 33-byte id). Find a model where `write` is not idempotent, or content that a load-then-save drops. Break routing with coincident and zero-size shapes and the scene with parent cycles of one and three. Check that no code path calls `toml::to_string`.
- *Canvas.* Re-run the spike's measurements on both renderers. Stack shapes to exceed the layer cap. Zoom to both clamps. Check that an idle board rebuilds nothing, that a theme change rebuilds exactly once, that resizing the window does not rebuild the scene, that the crate names neither `wren_core` nor an app type, builds alone, and never calls `request_input_method`.
- *App.* With a board open, run every command in the table and every `editor.*` harness request and diff the file afterwards. Switch `editor-engine`, toggle Markdown assist, change the theme and text size, open Find and Outline, blur the window, quit. Replace the open board on disk with conflict markers, then with a valid board, then with a Markdown file's bytes, while clean. Type in the raw fallback, undo, and check the saved bytes are exactly the typed text. Open a board, then a Markdown note, and check the editor shows the note and not stale or empty text. Run the suite with the setting off and look for any board string in the UI.

**Allowed deviations** (recorded in the chain's §8.5 subsection): the module split and private type shapes inside the two crates, provided §3.2 keeps its meaning; the drawing strategy, within the spike's rules; the name and keys of `[navigation]`, until the owner first enables the setting on a shared workspace, after which they are frozen; making `boards` apply live, provided turning it off flushes and closes an open board first; the 150 ms latch and the pan constants, from S7; the scene constants, until the golden files are blessed. **Not allowed:** anything in the last paragraph of §3.4; a board sending `EditDraft` in this phase; a `wren-core` or app dependency in either new crate.

### 7.3 Phase 2, "Build a flowchart"

**Goal.** Interactive editing. The keyboard-only flowchart test passes in the harness, a person can build and save a flowchart by mouse and by keyboard, and schema 1 is locked. Before run 2a the owner is asked the three lock confirmations of §4.17, so the answer is not waited for at the end.

**The contract commit** on `boards` lets Wave A run in parallel. It defines the complete Phase 2 enums and nothing else: `wren_board::edit::{Intent, Change, Plan}` with every variant (`AddNode`, `SetText { id, text, size }`, `Move`, `Resize`, `SetShape`, `Delete`, `Duplicate`, `Paste { text, at, size }`, `AddEdge`, `Rebind`, `SetEdge`, `Reverse`, `SetEdgeLabel`, `Extend { from, dir, shape }`), `plan` and `Board::apply` implemented for `Move` only; the `Ids` trait; stub signatures for `walk::next`, `place::slot` and `z::key_between`; `wren_canvas::{Input, Command, Message, Update, Effect}` with all 64 Phase 2 commands, and `BoardView::command`, `settle`, `saved` and `draft` accepting and ignoring them. It passes `make check`. A chain that finds the contract wrong changes it in its own branch and says so in its report.

| Wave | Chain | Sessions | Crates |
|---|---|---|---|
| A | Editing core, `boards-p2-edit` | E1 to E3; E4 to E7; E8 | `wren-board` |
| A | Canvas interaction, `boards-p2-canvas` | I1, I2; I3, I5; I4 | `wren-canvas` |
| A | Keys, `boards-p2-keys` | K1; K2, K3 | `wren-core` (`keybind.rs`), `wren-iced` |
| B | Wiring, `boards-p2-wire` | W1 to W4; W5 | `wren-iced`, `tools/wrentest` |
| C | Connectors, `boards-p2-connect` | X1; X2; X3 | all three, `tools/wrentest` |

Merge order: edit, canvas, keys; then wiring, forked from that head, which is the first time real gestures reach the Session; then connectors, forked after wiring is merged. With a complete contract the Wave A chains touch disjoint crates; the real conflict surface is the two enums if the contract proves incomplete, and the macOS menu, which destructures the keymap's bindings (`macos_menu.rs:35-38`) and is seen on Linux only by the cross-target lint.

**Editing core** (pure, `wren-board`).
- **E1. Ids** (`ids.rs`, §4.2). Test: 2,000 draws against a full board never collide and always match the pattern [100,000].
- **E2. Stacking keys** (`z.rs`): §4.7 exactly, with §4.16 test 11.
- **E3. Snapping** (`snap.rs`): §5.8 rules 1, 2 and 5. All results are integers.
- **E4. Node intents** (`edit.rs`, `edit/`). `plan(&Board, &Intent, &mut dyn Ids) -> Plan` takes explicit ids, never an ambient selection, and never mutates; `Board::apply(&[Change])` does. `AddNode`, `SetText` (trims trailing blank lines; takes the grown `size` from the caller, §5.9 rule 7), `Move`, `Resize`, `SetShape`, `Delete` (§5.10 rule 1: descendants are deleted), `Duplicate`. For all: unknown fields, kinds and `ext.*` on touched and untouched elements survive; a child is not moved twice when its ancestor is also selected; a plan that changes nothing is empty; `locked` is not consulted. Tests are tables of "board, intent, expected changed lines of `write`", because the line diff is the product promise.
- **E5. Clipboard fragment** (`clip.rs`): §5.10 rules 2, 4 and 6 to 9. Tests: paste twice into the source board; paste into a board that already uses every pasted id; a fragment with a dangling connector, a parent cycle, duplicate ids; a newer schema; arbitrary bytes never panic.
- **E6. History** (`history.rs`): §3.4 rule 6 and §5.11. Snapshots of the board, selection and cursor; `record`, `undo`, `redo`, `clear`, `open_group`, `close_group`; a group that nets to no change records nothing.
- **E7. Property test.** 500 seeded random intent sequences on random boards, some with integrity damage [20,000]: `apply` never panics, `check` reports no finding the input did not have, the canonical bytes round-trip, and undoing every step returns the first board.
- **E8. Connector model, placement, walking.** The connector intents (an end dropped on a node stores `{ node }`; only a side handle stores `side`, §5.12 rule 5); `place.rs` (§5.6) and `walk.rs` (§5.7). Copies the keyboard model's expected layout into `crates/wren-board/golden/accept/`. Test: the acceptance script replayed against these functions, with no app, gives that layout.

**Canvas interaction** (`wren-canvas`).
- **I1. Hit testing and handles** (`hit.rs`, `handles.rs`): §5.3 rules 4 and 5, over Phase 1's `Scene`, tolerances in screen pixels.
- **I2. The machine** (`machine.rs`): every state of §5.2 except Connecting, the Escape ladder, selection (§5.3, including the Phase 2 right-click), tools and creation (§5.5), move and resize (§5.8), duplicate-drag. Tests: one table row per transition, plus scripted sequences asserting the emitted effects.
- **I3. `BoardView` glue** (`lib.rs`): `update`, `command`, `settle`, `reload`, `saved`, `draft` and `synced` (§3.2, §3.4); the preview; the overlays drawn after the scene (selection outline, handles, marquee, cursor ring, mode chip, the empty-board hint); the reveal of §5.7 rule 5.
- **I5. iced translation** (`widget.rs`): pointer events to `Input` (the widget keeps receiving moves and the release outside its bounds while a button is held; cursor-left with no button and window blur cancel); keys only while focused, policy first (§5.1); every key that is not `Host` is captured so it cannot fall through to column navigation (`app/preferences.rs:513-531`). Tests: constructed iced events in, `Input` out.
- **I4. Label editor** (`label.rs`), its own session: §5.9 by the mechanism S8 chose. The preedit is tracked from `Event::InputMethod` in the canvas's own `update` (the stock editor's is private, `iced_widget-0.14.2/src/text_editor.rs:526`). Composition cannot be driven under Xvfb or headless Weston, so it is covered by constructed `InputMethod` events here and by the Mac console check.

**Keys** (`boards-p2-keys`).
- **K1. Keymap layers** (`keybind.rs`): §5.1, with its two guard tests. The cross-target lint is part of this chain's gates.
- **K2. Scope, table, policy, enablement:** §5.1 and the 64 rows of §5.19 with their default bindings; kind-aware `command_enabled`; the palette group rule; the Board branch of the `shortcut` request (`test_requests.rs:74-97`). Tests: twins of `commands.rs:1151` and `:1220`. No user documentation yet: the setting is hidden until Phase 3.
- **K3. New board:** `note.new_board` and the dialog of §5.4, with `DialogKind::CreateNote` carrying the kind (`app/workspace.rs:145`, `:365`, `:909`, `:943`).

**Wiring** (`boards-p2-wire`).
- **W1. One writer** and **W2. Settle** (§3.4 rules 1 and 3). `Message::Board(Message::Command(c))` goes to `dispatch(Command::Run(..))`.
- **W3. Session-driven change** (§3.4 rule 5) and the three document states (writable, read-only, raw) re-derived on every sync; read-only allows what §5.2 lists.
- **W4. Undo, clipboard, fit.** Copy and cut return `Effect::ClipboardWrite`, run as `iced::clipboard::write`; paste runs `iced::clipboard::read` and comes back as `Message::Pasted(epoch, text)`, dropped if the epoch moved.
- **W5. Harness, tests and scenes.** `board.key`, `board.type`, the full `board.pointer`, `board.apply` and the snapshot fields (§6.2); `boards.py` gains `key`, `type_text`, `press`, `drag`, `script` (replays a script written as in §5.20) and `line_diff`; `test_board_edit.py`, `test_board_keys.py`, `test_board_safety.py`, `test_board_dialog.py` and the Phase 2 native cases; the Phase 2 scenes except the connector ones; the per-keystroke cost of §3.4 rule 7.

**Connectors** (`boards-p2-connect`).
- **X1. Pointer:** §5.5 rule 8 and §5.12 rules 1 and 5 to 8; connectors are selectable and their labels editable. Arrowheads, dashes and both routes already draw from Phase 1's scene; anything missing is added to the scene and the SVG writer in the commit that makes it editable.
- **X2. Keyboard and commands:** Tab and Mod+Arrow (§5.6), keyboard Connecting with its inline query (§5.12 rule 3, §5.1 rule 3), `board.select.connector_<dir>`, `board.select.connected`, the style commands of §5.12 rule 9, the cursor move after delete.
- **X3. Acceptance:** the flowchart test in all four lanes and as a unit test of the machine; the connector scenes; the native X11 cases.

**Exit criteria.**
1. §6.8 Phase 2 items 1 to 10 pass in the phase-end gates.
2. With the setting off the existing suite passes unmodified, no board row or pill is listed, and the base keymap equals 0.1.1's.
3. `Op::EditDraft` for a board is sent from exactly one function, and every key, pointer, palette and harness path reaches it through `board_edit` or `dispatch`.
4. Every state in the machine table has a transition test, and every `Intent` a line-diff table.
5. Phase 1's guarantees still hold with editing present: never write on open, single writer, no board path in the v1 state keys.
6. §5 matches what landed, with deviations folded in.
7. The phase-end review and the Codex pass are done, with no open blocker and unfixed majors recorded with reasons.
8. Schema 1 is locked (§4.17) and `boards` is tagged `boards-p2`. The owner's Linux hand trial (§6.4) is listed as awaiting the owner and does not block the tag.

**What the reviewers should try to break.** Data loss first; each attack that finds a bug becomes a regression test.
1. *Label, then leave.* Type a label and within the same tick: click another note, press Back, run `note.rename`, switch workspace, blur the window, quit, quit during a workspace load. Repeat with a write fault armed. The text must be on disk or the transition blocked with the draft kept; never neither.
2. *Drag in flight during an external change.* Hold a drag, a resize, a marquee and a drag-connect while the file is rewritten on disk, clean and dirty. No half-applied move, state Idle, no draft from the cancelled gesture, the conflict copy holds exactly the pre-change draft.
3. *Undo after reload.* Edit, let an external change reload the board, press undo: nothing. Also undo across save, a conflict, read-only and raw round trips, and past the 200-entry cap.
4. *Paste with id collisions* (the E5 cases, through the app). The file must still parse after every one.
5. *Composition through Enter and Tab* (constructed events; the Mac console for the real thing): no half-composed text is committed and no tool key fires.
6. *The pristine rule.* Find any sequence without a surviving edit that leaves the document dirty or rewrites the file: select, walk, open and close a label, move and move back, undo to the start, undo then redo then undo, a reload of reformatted but equal content.
7. *A second writer.* `editor.insert`, `markdown.bold`, the find bar, an engine switch or a settings reload while a board is open; a board command while a text note or a raw board is open; a stale `Message::Board` arriving after a note switch.
8. *Keys in the wrong place.* Bare letters, digits, Tab, Delete and arrows typed in the palette, a dialog, a filter field, the find bar and the Notes column; Tab in the Rename dialog; a tool letter typed during keyboard Connecting.
9. *Esc.* One rung per press from every state; never moves focus off the board, never folds columns, never commits a label through the refocus path.
10. *Model integrity.* After any fuzzed sequence `check` reports nothing new; unknown fields, kinds, shapes and dangling connectors of a loaded file survive unrelated edits byte for byte.
11. *Cost.* No scene rebuild or model clone per frame at idle; the per-keystroke measurement of §3.4 rule 7.
12. *Canvas and keys specifics.* A transition missing from the table; a gesture that mutates the committed model before release; pointer capture lost at the window edge; hit testing under zoom, pan and a parent transform; any iced type in `machine.rs`; the base keymap changing for existing users; a board command enabled on a text note, a raw board or a read-only board; `note.new_board` or the pills visible with the setting off; a self-loop or duplicate connector from quick-connect; placement overlapping a node on a hand-arranged board; walking that cannot reach some node of a connected chart.

**Risks and cuts.** The phase is the largest of the three. Cuts allowed in this order, each recorded: the eight `board.shape.*` reshape commands; hover connect handles (keep handles on the selected node); `board.select.connected`; duplicate-drag; cut; re-attaching a connector end by drag. **Not cuttable:** undo with the first mutating gesture, settle-before-op, the pristine rule, external-change handling, the keyboard flowchart path, the injection seam, the New note dialog, the schema lock. **Allowed deviations:** flat `CommandId` variants instead of the nested one; a different split of files inside the two crates; where this document is wrong or silent, the most defensible behaviour, recorded. **Not allowed without the owner:** moving work across a phase boundary; anything in the last paragraph of §3.4; any change to schema 1 output after the lock.

### 7.4 Phase 3, "Finish and harden"

**Goal.** Ring 0 is complete, hardened and documented; the setting is on by default; the branch is ready to merge. Phase 3 changes no file syntax: if an item seems to need a new field or value, stop, because that is a schema change after the lock.

**The contract commit** on `boards` adds the 25 Phase 3 `Command` variants with their rows and default bindings (§5.19), the `Intent` variants `SetStyle`, `Restack`, `Align` and `Distribute`, and empty `Mode::BoardActions` and `Mode::BoardNodes`, all inert, so the Wave A chains fill in behaviour instead of all editing the same enums and tables.

| Wave | Chain | Sessions | Crates |
|---|---|---|---|
| A | Styling, `boards-p3-style` | ST1 to ST3; ST4, ST5 | `wren-core` (theme), `wren-board`, `wren-canvas`, `wren-iced` |
| A | Arrangement, `boards-p3-arrange` | AR1 to AR3; AR4 after styling is merged | `wren-board`, `wren-iced` |
| A | Find, `boards-p3-find` | F1, F2 | `wren-iced` |
| A | Export, `boards-p3-export` | EX1; EX2 | `wren-iced` |
| B | Harden, `boards-p3-harden` | H1; H2 | all |
| B | Release, `boards-p3-release` | R1; R2, R3 | `wren-iced`, `wren-core`, `tools/` |
| B | draw.io (optional), `boards-p3-drawio` | DX1 | `wren-board`, `wren-iced` |
| B | Docs, `boards-p3-docs` | D1 | `docs/`, `CLAUDE.md`, this document |

Merge order: style, arrange, find, export; then harden, release, draw.io if it runs, docs last so the pages describe what shipped. Each chain has its own harness file (`test_board_style.py`, `test_board_arrange.py`, `test_board_find.py`, `test_board_export.py`, `test_board_limits.py`). Remaining shared files: `app/commands.rs` (`command_enabled`, `run_command`), `app/palette.rs` row builders (`:24-41`, `:98-119`, `:647-663`), `test_requests.rs`, `tools/capture_scenes.py`, `tools/wrentest/boards.py`. Each chain keeps its additions in one block. All user documentation belongs to the docs chain.

**Styling.**
- **ST1. Hues in the theme.** `Colors` (`wren-core/src/theme.rs:11-53`) gains seven hue values, one per token, in each of the eleven `BUILT_INS` (`:485-497`): 77 values. Neutral needs none (§3.7). Choose each theme's hues from that theme's own published palette where it has one, so boards look native to it.
- **ST2. Resolver values.** `theme::board_palette` and the tint ratios get their final values; the signature of `style::resolve` does not change.
- **ST3. Tests over every theme, colour and fill level**, in `theme.rs`'s test module, reusing `contrast` (`:533`) and `lightness` (`:552`): label on its fill at least 4.5:1; stroke on the canvas at least 3:1; connector label on the canvas at least 4.5:1; a tint fill at least 4.0 L* from the canvas (the selection's rule, `:703`); the selection ring (`accent`) at least 3:1 against the canvas and drawn outside the shape. Colour blindness: port the Machado 2009 severity-1.0 matrices for protanopia and deuteranopia from `.claude/board-notes/research/critic/cvd.py` and add tritanopia; for every pair of the seven hues, at solid and at stroke, the simulated colours differ by CIE76 ΔE of at least 15 or by at least 8 L*. These two numbers are starting values: tune them once against real palettes, record the final numbers and the per-theme hue table in this document, and treat any later loosening as a deviation. The Markdown tests are untouched: no existing token changes value.
- **ST4. Commands:** §5.16 rule 1, and the digit-row fallback of §5.1 rule 7.
- **ST5. Real-app checks:** set a colour with `board.key`, assert one changed `style.color` line and the fill pixel in a screenshot; undo restores the bytes; the native X11 digit case; scene `board-colors`.

*Reviewer focus:* a hue pair a colour-blind user cannot tell apart that the test lets through; a theme where tint is invisible; label colour on literal hex; a colour command that dirties a board without a change; an unknown token being rewritten.

**Arrangement.**
- **AR1. Stacking:** §5.16 rule 2 over §4.7, pure functions in `wren-board`. Tests: each command on single and multiple selections, sibling scope with a hand-written parent, ties, 500 alternating steps, the renumber fallback.
- **AR2. Align** and **AR3. Distribute:** §5.16 rule 3.
- **AR4. The actions palette** (after styling is merged): §5.17, with `board.actions` and the right-click.
- Tests: model tests for AR1 to AR3. App tests: a preview followed by Escape leaves `document.dirty` false and the bytes equal; a preview followed by a note switch, `window.blur` or quit writes nothing; Enter writes exactly the one changed line. Harness: the listing for one node, one connector, a mixed selection and nothing selected; right-click through `board.pointer` and through xdotool. Scenes `board-stacking` and `board-actions`.

*Reviewer focus:* a previewed style reaching disk by any path (autosave tick, flush, external reload or theme change during the preview); undo after align of nodes under different parents; distribute with equal positions; stacking that rewrites more than the moved lines outside the fallback; `/` or `]` typed in a label or in the palette input acting as a command; a right-click that opens the palette on the wrong element after a pan or zoom.

**Find.** **F1. Jump to node** (§5.18 rules 1 to 3) and **F2. Search reveal** (rule 4). Scene `board-jump`. *Reviewer focus:* a path where jump or reveal marks the document dirty or changes mtime; reveal on a query that matches only raw syntax; the palette left open across a note change.

**Export.**
- **EX1. PNG.** `note.export_png` beside the SVG command. Pipeline: scene, SVG string, `usvg::Tree`, `resvg::render` into a pixmap, PNG, in a new `wren-iced/src/board_export.rs`. Add `resvg = "=0.45.1"` as a direct dependency with default features off and `text` on, and check with `git diff Cargo.lock` that no package is added. The font database is the four bundled Inter faces (`fonts.rs:46-50`), then system fonts as fallback only, because Inter has no CJK glyphs. Defaults: scale 2, the theme's canvas colour as background, the SVG's bounds and padding. The limits of §3.8 apply.
- **EX2. `wren export`.**

```
wren export <board> [-o <path>|-] [--format svg|png] [--theme <id>] [--scale 1..4] [--background theme|transparent]
```

  - Parsing: `Command::Export(ExportArgs)` when the first argument is `export`, checked in `cli::parse` (`cli.rs:26`) before the flag loop and before the smoke-test arm (`:33-39`). Relative paths are accepted, unlike `--workspace` (`:50-57`). `wren export --help` prints the usage; the main help (`main.rs:86`) gains one line.
  - No window: `run()` returns for help and licences before the application is built (`main.rs:83-93`); export returns at the same point.
  - Fonts with no window: load the Inter faces into iced's global font system directly (`iced_graphics-0.14.0/src/text.rs:116`, `load_font` at `:150`); it needs no renderer. The same measurer then runs, so the scene is the one the app builds. One function does this for the CLI and for tests.
  - Defaults: format from `-o`'s extension, else SVG; output beside the input as `<file name>.svg` or `.png`; `--theme` takes any id `theme::builtin` accepts (`theme.rs:501`), default `wren-light`. The user's `config.conf` is not read, and it works whatever the `boards` setting says.
  - Writing: a temporary file in the target directory, then a rename; an existing output is replaced; writing onto the input is refused; `-o -` writes to standard output.
  - Exit codes: 0 written (integrity warnings and a read-only board's warning go to standard error); 1 could not read the input or write the output, or rendering failed; 2 bad usage; 3 the input is not a board Wren can draw (a parse error with its line, or over a limit). `main()` maps every error to 1 today (`main.rs:32-40`); `run()` returns the code instead. An empty board exports an empty padded canvas.
  - Tests: `cli.rs` unit tests beside `licenses_and_help_do_not_need_a_display` (`:132`); an integration test in `crates/wren-iced/tests/` running the built binary with `DISPLAY` and `WAYLAND_DISPLAY` removed: every option, every exit code, nothing written on failure, bytes equal to the golden SVG; a harness case comparing the CLI's SVG with `board.export` for the same theme; §6.8 Phase 3 items 5 and 6.

*Reviewer focus:* an export that marks the document dirty; labels with XML-illegal characters, `]]>`, right-to-left and CJK text in PNG; a 10,000-element board at scale 4; output paths that are directories, unwritable, or the input; the CLI on a board with conflict markers; whether any code path opens a display.

**Harden.**
- **H1. Limits** (§3.8, §4.12, §4.16 test 10): the caps, the issues for out-of-range values, the export escapes, and the full mutation run of `make fuzz-board`; one fixture per limit; every SVG exported from the fuzz corpus parses as XML. Lower the element cap if opening at the cap takes over 3 s in release on tiny-skia.
- **H2. Performance:** run §6.7 in release and debug on the 1,000 and 10,000 element fixtures and at the cap, on both renderers, and record the table in this document.

**Release.**
- **R1. The macOS menu** (§5.19). The menu rows come from a pure table checked on every platform (each key parses with `CommandId::parse` and has Board or App scope), with a smoke-test accessor like `updates_enabled` (`macos_menu.rs:134`). The cross-target lint is part of this chain's gates.
- **R2. Notices and provenance.** Confirm the lyon texts of step 0 are in the bundle; run `tools/dependency_notices.py` on `main` and on `boards` and record the packages requiring review before and after; add a provenance header and a `NOTICES` line for any source ported into either crate.
- **R3. The setting defaults to on.** The chain's last commit: the default in `settings.rs`, the `TEMPLATE` (`:388`), the default of `WRENTEST_BOARDS`, and the tests that assert the off state. It is merged before the phase-end gates, so the final review and the lanes run the configuration that ships.

**Docs** (`boards-p3-docs`, with the strict docs build).

| Page | Change |
|---|---|
| `docs/reference/boards.md` (new) | What a board is, tools, connectors, colours, arranging, export, limits, the setting. |
| `docs/reference/board-format.md` (new) | Schema 1, normative: §4 as locked, the integrity rules, the limits, the full example. The spec lens tests every statement on it. |
| `zensical.toml`, `docs/reference/configuration.md`, `docs/reference/autosave.md` | Both new pages in the nav; the `boards` row; when a board saves, the conflict copy's name, raw text for an unreadable board. |
| `docs/reference/workspace-folder.md` | Boards as notes; the ignored-files, title and large-file rules; `[navigation]` and the rule that the old keys hold Markdown paths only. |
| `docs/getting-started/keybindings.md` | "On a board" (§5.1 rule 9), with macOS and Linux keys. |
| `docs/development/architecture.md`, `testing.md` | A Boards section (crates, display list, pristine rule, settle-before-op, single writer, state-file rule), the theme hues and the colour-blindness rule; board requests, `perf.board`, the two make targets, the CLI test, scenes, the Mac limits. |
| `docs/getting-started/first-run.md`, `installation.md`, `docs/index.md`, `README.md` | A paragraph each; installation says where the binary is for `wren export` (`tools/package.py:549`, `:564`). |
| `CLAUDE.md`, `CHANGELOG.md`, this document | Layout bullets for the new `wren-iced` files, architecture rules for the single writer and settle-before-op, the verify list; an `### Added` entry under `[Unreleased]` with macOS and Linux keys; status, what landed, and the trim `CLAUDE.md` asks for once a design lands. |

*Reviewer focus for Wave B:* a mutated file that panics, hangs or allocates without bound; a limit that silently drops data; a docs sentence that the app contradicts; a menu accelerator that steals a typed character; a new dependency or licence the notices miss; a measurable slowdown for a Markdown-only user; anything that differs between a fresh config and one that says `boards = on`.

**draw.io export (optional).** Scheduled only by the owner (question 14). The feasibility study is done; its reports, a 458-line reference exporter, a second coverage board and a render harness that loads output in draw.io's own code are in `.claude/board-notes/research/drawio/` (`format.md`, `consumers.md`, `build.md`, `verify.md`, `drawio-export-proof/`). Result: a one-way export works. draw.io 32.6.10 opened the reference output with no errors as native shapes and saved it back with no cell changed; an independent pass could not overturn that. A converter in `wren-board` from the model to an uncompressed `mxfile`, written with the SVG writer's escaper and no new dependency, plus `note.export_drawio` and `--format drawio`. Rules from the study, which are the chain's contract:
- Built-in shapes only, never an `mxgraph.*` stencil name (the stencil libraries carry separate terms); a pill is `rounded=1;arcSize=50`.
- One page, uncompressed, `page="0"` on `mxGraphModel`; ids are any unique strings except `0` and `1`.
- Edges always write both arrowheads and `rounded=0`, because draw.io's stylesheet otherwise adds an arrow and rounding. Edges are parented to the layer so stacking stays exact. A right-angle route with no stored points gets draw.io's own bends.
- Labels use `html=1` with HTML-escaped text, an explicit `fontSize`, and preserved runs of spaces; no font family is written.
- Colours are resolved to hex for one theme (Wren Light by default), which loses the tokens and is said so in the docs. Wren's ids and anything draw.io cannot express go in attributes of an `<object>` wrapper, not in style keys, which ordinary edits strip.
- Golden tests compare parsed XML, since draw.io re-sorts attributes on save. The render check against a local draw.io clone stays outside CI.
- It is a one-way export: draw.io replaces every id on Import and on paste, so the docs promise nothing about bringing a file back. Lucidchart's import is untested (its importer is closed, and users report connector labels going missing); the hand kit is `research/drawio/drawio-export-proof/try-by-hand/`. Visio and Miro need `.vsdx`, which draw.io removed from its public code in 2025.
draw.io is Apache-2.0; the converter is written from the file format, with a provenance note and no copied code. The study's estimate is 6 to 9 working days by hand. The MVP's exit does not depend on this chain.

**Phase 3 is done when** every chain is merged with its review recorded and no open blocker, the ship-readiness checklist of §8.2 is complete, the final colour thresholds (ST3) and the measurements table (H2) are written in this document, the final review's findings are fixed or listed, and the branch is tagged `boards-p3`.

**The MVP is done when**, in addition, the owner has: pushed and seen `ci-success` green, including the macOS Rust job; done the Mac console checks of §6.4; built and saved the 15-node flowchart by mouse and by keyboard on Linux and on macOS, recoloured it, exported it and reopened it after a restart. These wait for the owner and are tracked in the handoff note; none blocks the tag.

**Allowed without asking, if recorded:** tuning the colour-blindness thresholds once; lowering the element cap from measurements; renaming a command key to match what Phases 1 and 2 landed (in §5.19 too); moving a work item between Wave A chains; dropping tritanopia from the pair rule if it forces a palette that fails the red-green rules; a white background option in place of transparent if resvg's output shows fringing; cutting dashed shape outlines. **Not allowed without the owner:** any change to schema 1; a new direct dependency other than the resvg pin; a board path in a v1 state key; cutting any other Phase 3 feature; moving work across phase boundaries; scheduling draw.io; pushing.

## 8. Reviews

### 8.1 The per-chain adversarial review

Every chain in every phase gets one, by a fresh session with no memory of the implementation.

**The reviewer is given** the common brief (worktree, branch, `CLAUDE.md`, the sections of this document that are the chain's contract); the implementers' structured reports; the diff command `git diff boards...HEAD`; and the chain's "try to break it" list from §7.

**The reviewer is told:** do not fix and do not commit; throwaway tests are fine, but delete them and leave `git status` clean; run the chain gates of §6.6 yourself and do not trust the report; open every scene and list each file with what it shows; report only problems with evidence; list what you checked that held, so silence can be told from not looking.

**Findings schema** (the editor effort's): `{ gates_pass, gates, issues: [{ severity, where, problem, evidence, fix }], verified_ok: [string] }`. Severities:
- **blocker:** lost or corrupted data; a panic reachable from input or a file; a write the pristine rule forbids; any change to schema 1 output after the lock; a visible change with `boards = off`; a failing gate; a licence violation.
- **major:** specified behaviour wrong or missing; an untested data path; a missed budget; docs that contradict behaviour.
- **minor:** everything else.

**The fix step** is a third session given the review. It reproduces each finding first, fixes every blocker and major, fixes minors that are clearly right and cheap, explains with evidence any finding it rejects, and adds one regression test per real bug, in small commits that each pass `make check`. It returns `{ summary, commits, gates, not_fixed }`.

**The verify step** is a short fourth session that re-runs the gates and re-tests each blocker and major from its evidence. The chain merges only when the gates pass and no blocker is open. Two failed rounds stop the workflow for that chain and return it to the owner; independent chains still merge. A major left unfixed needs its reason in §8.5 and is carried into the phase-end review.

### 8.2 The phase-end review

Run c of each phase, on `boards`, in this order:

1. `make check`, then the four lanes with the setting off, one at a time. No other session uses the worktree.
2. The review sessions, in parallel, each read-only in its own detached worktree at the phase head (`git worktree add --detach .claude/worktrees/boards-lens-<key> <head>`: one cold build each, under three minutes and about 3 GB), each returning the schema of §8.1 plus an `assessment` string. Phases 1 and 2 run the data-safety lens; Phase 3 runs all four (question 3 of §2.4 can add the others to the earlier phases). The Codex pass (§8.3) runs beside them.
3. Triage, then one fix session on `boards`, by the rules of the fix step above.
4. The final gates, once, after the fixes. This pass gates the tag: all eight lane runs (below), `make fuzz-board`, `make perf-board`, the release performance run of §6.7 with its numbers recorded, the strict docs build when docs changed, the acceptance tests of §6.8, and the Mac over ssh (build, Rust tests, scenes). Phase 2 adds the schema lock commit; Phase 3 adds every board scene in the eleven themes on both renderers.
5. Record the result in §8.5, write the handoff note, tag.

```sh
for b in off on; do for r in tiny-skia wgpu; do
  export WRENTEST_BOARDS=$b ICED_BACKEND=$r WRENTEST_ENGINE=classic,advanced WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1
  make smoke-x11 && make smoke-wayland || break 2
  cp -r target/wrentest target/wrentest-$b-$r    # the next pass overwrites junit.xml and the artifacts
done; done
```

| Lens | What it does |
|---|---|
| Data safety | Hunts for any path that loses, corrupts or wrongly writes a board or a note, and for panics. Runs the real-app fuzz below and the loader mutation run. Greps new code for `unwrap`, `expect` and indexing and tries to trigger each. At Phase 2 it also checks every rule of §4 against the reader and writer, black-box, before the lock. |
| Spec conformance | Without reading the implementation first, takes every rule and table row of §4 and §5 and of `docs/reference/board-format.md` and checks it in the real app through the harness, with a sample of real X11 input: at least 150 cases. Replays the acceptance script and every `wren export` option and exit code. |
| Architecture, licensing, docs | The crate rules of §3.1. `CLAUDE.md` rules. Spike or test-only code in release. One display list feeding the widget and the SVG writer. Provenance headers. Every package added to `Cargo.lock` since `main` with its licence. Notices. Docs against behaviour. The CI contract. |
| Performance and ship readiness | Measures §6.7 in release (its own target directory) and debug, and Markdown-only workloads against `main` with the setting off and on. Writes the ship-readiness assessment against the checklist below. |

**The fuzz** (data-safety lens). A script drives the real app over the test socket for a few thousand seeded steps on tiny-skia and a shorter run on wgpu. Start boards are generated ones, hand-written non-canonical ones, and ones with integrity damage. Steps are random `board.key`, `board.pointer`, `board.type`, `command.run`, undo, redo, palette preview then Escape, jump, search reveal, export, note switch, theme change, `window.blur`, an external rewrite of the file, and quit and restart. In Phase 1 only the read-only steps and reload exist. After every step it checks: (1) the Session draft equals the pristine text when the model equals the pristine model, and the canonical serialisation otherwise; (2) after a flush the disk bytes equal the draft, and parse back to the surface's model; (3) a board that has had no edit has unchanged bytes and mtime; (4) undoing to the start gives the original bytes; (5) no editor text reaches a board file and no board text reaches a note; (6) `.wren/workspace.toml` passes the frozen v0.1.1 validator; (7) the app has not exited and no request has timed out. Every failure reports its seed and step so the fix session can replay it. A failure that will not replay is logged, not waved through.

**Ship-readiness checklist** (Phase 3). Each line is true or is listed, with its reason, under "Not fixed, by choice".
1. `make check`, the eight lane runs and the strict docs build pass on `boards` with the setting on by default.
2. The acceptance tests of §6.8 pass for all three phases.
3. No open blocker; every open major has an owner decision recorded.
4. The fuzz ran its full length on tiny-skia and found no failure at the final head.
5. Every budget of §6.7 is met or recorded with its number.
6. With `boards = off`, the app, the palette, the menu, the workspace listing and the state file match `main`.
7. Schema 1's golden files are unchanged since `boards-p2`.
8. The format page, the user page, the changelog and `CLAUDE.md` match behaviour.
9. `Cargo.lock` additions are listed with licences; the notices bundle builds with `make package-debug`.
10. What waits for the owner is listed: the push and CI, the Mac console checks, the hand trials (§7.4).

### 8.3 The Codex pass

Mandatory at the end of every phase (decision 11), beside the in-house lenses and before the fix session. The orchestrator runs it through the installed Codex CLI. Other model families (Cursor, Grok, or two at once through `craze`) are optional and at the owner's request.

**It is given** a detached worktree at the phase head, read-only by instruction; the diff since the previous tag (`main` for Phase 1); this document; the phase's exit criteria; the findings schema to answer in; and a narrow brief, not "review everything":
1. find inputs where reading then writing a canonical board changes it, or where writing then reading a model changes it;
2. find a sequence of user actions after which the file on disk differs from what the board shows;
3. find a path by which the hidden editor writes to a board, or the board writes without a user edit.

**It is not given** the in-house findings, so it stays independent.

**Triage.** A Claude session tries to reproduce each finding as a failing test or a harness transcript. Reproduced findings enter the fix queue with that reproduction as their evidence, and their severity is set again by the definitions of §8.1. Findings that do not reproduce are logged in one line each and dropped. Duplicates of in-house findings are merged, noting who found them. Reproduced findings are fixed before the next phase starts; a reproduced blocker blocks the tag.

### 8.4 Recording results

- Raw outputs (implementer reports, reviews, fix reports, lens reports, the Codex output) are saved as JSON under `.claude/board-notes/workflow-results/`, named `p<phase>-<chain>-impl|rev|fix.json`, `p<phase>-end-<lens>.json` and `p<phase>-end-codex.json`. They are not committed.
- §8.5 holds one subsection per chain, created now so that parallel chains never edit the same lines. A chain writes there, in its own worktree: its deviations, then three lines for its review (counts by severity, the fix commits, what was not fixed and why). The spike record goes in the Phase 1 canvas subsection.
- Per phase end the orchestrator adds a subsection in the shape of the editor plan's final review: a Fixed list (severity, what, the measured before and after, the regression test, and who found it), a "Not fixed, by choice" list with reasons, the gates with pass counts per lane, and what waits for the owner. The final fix session writes the ship-readiness assessment there in this document's own voice, keeping the numbers.
- `.claude/board-notes/HANDOFF.md` records head, gates and what waits on the owner after every run.

### 8.5 Log

Step 0 and the phase ends are written serially on `boards` and get their subsections when they happen.

#### Step 0
Items 1 to 4, in run 1a.
- *Preflight* (Linux): `xvfb-run`, `xdotool`, `weston`, `dbus-run-session`, the `aarch64-apple-darwin` target and the Codex login are present. Software Vulkan is lavapipe (`mesa-vulkan-drivers` 26.1.6); a wgpu capture under Xvfb drew all 33 scenes, each different from its tiny-skia twin, and repeated byte for byte. `vulkaninfo` is not installed and nothing needs it. The Mac is the orchestrator's check.
- *Worktree*: `boards` forks `main` at `393ae14`; the worktree was on `boards` and clean.
- *Capture determinism* (deviation: the fixed path was not enough). Three more clocks made scenes differ between captures of one build: the footer caught the 750 ms autosave either side of the shot, carets blink on wall-clock time from their last focus, and the first sample note, dated "now", read "just now" or "1 min ago" by run length. `capture_scenes.py` now rebuilds `target/scene-workspace/`, waits before each shot for a clean, durable document, restarts the caret with a new smoke-test request, `test.caret.restart` (it focuses the focused widget again, which also moves a palette input's caret to its end), and dates the sample notes half an hour clear of an hour boundary. Result: three consecutive captures of 132 tiny-skia scenes (dark and light, classic and advanced) are byte-identical, and `--only markdown` matches the full run's files. The header draws the workspace path, so scenes compare only within one worktree: a detached lens worktree must shoot its own baseline from the step 0 commit.
- *Baseline for exit criterion 3(d)*: the 132 scenes at `24abca9` (the main code plus the two capture commits, since `main` itself cannot capture repeatably) are kept in the boards worktree at `target/scenes-baseline/main/`, with `SOURCE` and `SHA256SUMS`.
- *The `canvas` feature* (`9ce239a`, revertible alone): it adds exactly the six packages of §3.1. Inert: `make check` passes (2,277 Rust tests, 283 Python units), the X11 tiny-skia suite passes on both engines (241 passed, 1 skipped, the classic-engine hanging-indent case), all 132 scenes are byte-identical to the baseline, and `make package-debug` assembles with `third_party/lyon/` in the bundle. `tools/dependency_notices.py` before: 485 packages, 768 local texts, 65 requiring review; after: 491, 769, 70, the five added being the lyon crates, which carry no licence file. Their upstream MIT and Apache texts (identical at both source commits of the locked versions) are in `third_party/lyon/`, named in `LICENSE_NOTICES` and `NOTICES`, and a package test keeps the notice's lyon versions equal to `Cargo.lock`'s.

#### boards-p1-core
Nothing yet.
#### boards-p1-model
Nothing yet.
#### boards-p1-canvas
Nothing yet. (Holds the spike record: numbers per run, the S2 and S3 screenshots, the chosen strategy, the S8 decision.)
#### boards-p1-app
Nothing yet.
#### boards-p2-edit
Nothing yet.
#### boards-p2-canvas
Nothing yet.
#### boards-p2-keys
Nothing yet.
#### boards-p2-wire
Nothing yet.
#### boards-p2-connect
Nothing yet.
#### boards-p3-style
Nothing yet. (Holds the final colour-blindness thresholds and the per-theme hue table.)
#### boards-p3-arrange
Nothing yet.
#### boards-p3-find
Nothing yet.
#### boards-p3-export
Nothing yet.
#### boards-p3-harden
Nothing yet. (Holds the measurements table.)
#### boards-p3-release
Nothing yet.
#### boards-p3-docs
Nothing yet.

## 9. After the MVP

Ring 1, in this order, each its own converter to or from the `wren-board` model so that formats scale without touching the editor:
1. **draw.io export**, if the optional Phase 3 chain did not run.
2. **JSON Canvas export** (the colour tokens already follow its presets, §4.8).
3. **A layout pass** in `wren-board`, serving `wren import`: it places nodes that arrive without positions. With it the reader may accept a missing `at` as "unplaced" (§4.12).
4. **Mermaid import and Tidy**, built on that layout pass; this is the route to agent support the owner described (decision 4): an agent writes Mermaid or a position-free board and the CLI imports it.
5. The remaining ring 1 features as new kinds and fields in the same file format (§4.5): frames and groups, note cards, images (attachments under `Section/Group/assets/` with content-hashed names), a sticky preset, lock, curved and hand-bent connectors, alignment guides, style copy and paste.

**Sync** is a separate, focused feature (decision 2), with its own plan. Element-level merge of concurrent edits belongs with it; the one-field-per-line format and id-sorted blocks are what make it possible later. Ring 2 (freehand, sketch style, formatted labels, board previews inside notes, mind-map mode) is not planned.

## 10. Open questions and risks

The owner's open questions are in §2.4. The technical questions are answered by the build itself: label layering (spike S2), sharpness under zoom (S3), the tiny-skia budget with labels (S5), the label editor's mechanism (S8), scroll units on Wayland and macOS (S7, then the owner's devices), whether resvg lays text out as wide as the measurer says (the Phase 1 confirmation step), whether the acceptance labels fit their default boxes (V2), the cost of a draft per label keystroke (W5), whether every theme can supply seven hues that stay distinct under colour blindness (the check before the schema lock, then ST3), and the element cap (H1, H2).

| Risk | Mitigation |
|---|---|
| A missed `.md` assumption or a stale editor draft overwrites a board. | `Leaf` instead of a bool, the op-by-kind table, the single-writer guard with its tests, and a board that sends no draft at all in Phase 1. |
| A build with the setting on writes a state file that 0.1.x rejects. | The state-file rule, the frozen validator after every step, byte identity with the setting off, the live check against a 0.1.1 binary. |
| Text layering or zoom does not hold on one renderer. | The spike's written fallbacks; one scene that the widget and the SVG writer both consume. |
| The label editor and the input method under zoom. | S8 is mandatory and ends in a decision before Phase 2; I4 has its own session; constructed events in tests; the Mac console check. |
| Layering the keymap changes behaviour for existing users. | The frozen base-layer test; no board binding shadows an App trigger; the whole Markdown suite and the cross-target lint in the keys chain's gates. |
| A gate replaces the smoke binary under a running suite, and the failure looks like a product bug. | The operating rules of §6.6; a detached worktree per lens; hand trials in their own target directory. |
| macOS-only code breaks unseen. | The cross-target lint in chain gates; the Mac over ssh at each phase end; the owner's push for CI. |
| Software Vulkan numbers mislead. | Budgets are judged on tiny-skia and on `draw_micros`; Metal and a desktop run are recorded beside them. |
| The scene constants or the provisional palette change after golden files exist. | Constants are recorded in §3.7 before any golden is blessed; the palette lives in one function named provisional; hues are checked before the schema lock. |
| A preview or a read-only action writes. | The preview is a scene-time patch, never model state; fuzz check 3; the arrangement review's list. |
| The headless measurer or resvg disagrees with the app, so exported labels overflow. | One font-loading function; pre-wrapped lines; the confirmation step; the CLI-against-app byte comparison. |
| Limits too tight for real boards, or too loose to stay responsive. | The caps come from H2's measurements, are documented, and the fallback is raw text, never data loss. |
| iced 0.15 lands mid-effort. | No iced upgrade on `boards`; all renderer calls stay in the display-list painter. |
