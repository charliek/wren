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
| 16 | Decided 2026-11-12, after the owner saw the Checkout flow sample in the viewer: "we shouldn't have connectors running through the shapes like that". The router is fixed, not only the fixture. Amends the decision record's "no obstacle avoidance" and §3.7's Routes row, which M4 marked final. | Elbow routes avoid other shapes by a bounded choice among candidate routes and move off other connectors' lines where a cheap rule can (§3.7, Routes); routes stay computed, never stored; work item R1, chain `boards-p1-route`. |

## 3. Architecture

### 3.1 Crates

```
crates/wren-board/    toolkit-neutral. deps: toml (the workspace pin, parsing only), unicode-segmentation.
                      No iced, no wren-core, no app type.
  lib.rs      Board, Node, Edge, Id, End, Palette, GRID (20), EMPTY ("schema = 1\n")
  read.rs     parse(&str) -> Result<Board, ParseError>                     §4.12
  write.rs    write(&Board) -> String, hand-written and canonical           §4.11
  check.rs    check(&Board) -> Vec<Issue>; access(&Board) -> Access;         §4.10, §4.12
              placements(&Board), each node's drawn parent and absolute corner
  shape.rs    outlines, label boxes and anchors of the eight shapes          §3.7
  route.rs    straight and elbow routes among the boxes; sides; route_all    §3.7, §5.12 rule 4
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
  layers.rs   split: the lowest layer each item may take, capped at 4 (§8.5, the canvas chain's deviation 1)
  paint.rs    Cull and shown: what one frame holds of a scene, by layer; each label line laid out in bounded room
  clip.rs     paths reaching far past the widget cut before they reach the renderer (§3.8)
  Phase 2:    hit.rs, handles.rs, label.rs
wren-core     gains kind.rs (NoteKind) and depends on wren-board (label extraction for search, theme::board_palette).
wren-iced     depends on wren-board and wren-canvas (from the contract commit, so the spike needed no Cargo.lock
              change). app/board.rs holds the surface decision and, from Phase 2, board_edit,
              board_changed and settle_board. Phase 3 adds board_export.rs (resvg) and the export CLI.
```

Rules. `wren-board` names no iced, `wren-core` or app type. `wren-canvas` names no `wren-core` type and no app message; the host passes a `Palette`, a `Style`, a font and a key policy. resvg is reachable only from `wren-iced`. No `unsafe`. No new package enters `Cargo.lock` except the six the `canvas` feature brings (`lyon`, `lyon_algorithms`, `lyon_geom`, `lyon_path`, `lyon_tessellation`, `float_next_after`); resvg `=0.45.1` is already locked (`Cargo.lock:2978`); it becomes a dev-dependency of `wren-iced` for the Phase 1 confirmation step and a direct dependency in Phase 3. Property tests use a seeded generator in the test module. Random ids come from `std::hash::RandomState`.

### 3.2 Public surface

```rust
// wren-board
pub fn parse(source: &str) -> Result<Board, ParseError>;   // ParseError { line, column, message }; never panics
pub fn write(board: &Board) -> String;                      // canonical, deterministic, idempotent
pub fn check(board: &Board) -> Vec<Issue>;                  // non-destructive
pub fn access(board: &Board) -> Access;                     // Editable | ReadOnly(Reason::Schema(n) | Reason::Requires(names))
pub fn placements(board: &Board) -> BTreeMap<&Id, Placed>;  // drawn parent and absolute corner; §4.12's parent rules
                                                            // Issue { element: scene::Source, kind: IssueKind }
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
    pub fn layers(&self) -> &layers::Plan;  pub fn palette(&self) -> &Palette;  pub fn epoch(&self) -> u64;
    pub fn size(&self) -> Option<(f32, f32)>;  pub fn machine(&self) -> &Machine;
    pub fn fit_pending(&self) -> bool;                               // a fit still waiting for the widget's size
    // Phase 2: command(Command, now) -> Update;  settle(now) -> Update;  saved(&mut self, text: &str);  draft(&mut self) -> String
}
pub enum Message { Input(u64 /* epoch */, Input), Resized { width: f32, height: f32 }
                   /* Phase 2: Label(u64, LabelAction), Command(Command), Pasted(u64, String) */ }
pub struct Update { pub changed: bool, pub effects: Vec<Effect> }   // Effect: ClipboardWrite, ClipboardRead, OpenActions, OpenJump, Notice
pub struct Style { pub labels: Option<Labels>, pub scale_factor: f32 }   // None: the renderer's own label rule (§5.13 rule 8)
impl Viewport { pub fn fitting(content: Rect, size: (f32, f32)) -> Self; pub fn zoomed_to(self, zoom: f32, about: (f32, f32)) -> Self; }
pub const FIT_MARGIN: f32 = 40.0;  pub const ZOOM_STOPS: [f32; 9];  pub const REST: Duration /* 150 ms */;  pub struct Measurer;
impl Machine { pub fn step(&mut self, input: Input, view: &View) -> Vec<Effect>; }   // Phase 1 has Idle and Panning only
```

`Message::Resized` is how the widget reports its size, which `fit` and the zoom stops need and a `BoardView` cannot ask for; it is not board input (§5.15 rule 1). The machine moves the view through `Effect::Look(Viewport)`, which `BoardView::update` applies itself, so in Phase 1 `Update` is always the default; Phase 2's contract commit keeps `Look` beside the host effects or replaces it with an equivalent. `update` drops input stamped with an older epoch. `reload` with the text the view already has does nothing; `set_viewport` clamps the zoom to 10 to 400 % and ignores a viewport that is not finite.

`BoardView` is defined in `wren-canvas` and owned by `App`, as `EditorView` is defined in `wren-editor`. It holds the committed model, the pristine pair, the synced text, the scene, the viewport, the machine and, from Phase 2, the history, the in-flight plan and the open label's text. None of that lives in widget tree state, so the harness drives and reads it with no window and a rebuilt widget tree loses nothing. Phase 1 ships `Message::Input(epoch, Input)` and the `Machine::step` signature with wheel and pan inputs only; Phase 2 extends both and replaces neither.

### 3.3 The surface seam in the app

- `App` keeps `editor: EditorView` and gains `board: Option<BoardView>`. `app/board.rs` derives `Surface { Text, Board, Raw }`. The view mounts exactly one of the two widgets under the focus id `note-editor` (hard-coded today at `wren-editor/src/lib.rs:268`; mounted at `app/workspace_view.rs:896-900`), so `focus_editor` (`app.rs:813`) and `column_focus` (`app/columns.rs:121-127`) work unchanged. `Message::Board(wren_canvas::Message)` sits beside `Message::Editor` (`app.rs:122`).
- **Deciding the surface.** `sync_editor` (`app/workspace.rs:777-793`) decides it when the note identity changes, when the note's kind and the surface disagree, or when the Session text differs from what the surface last took (`board.synced()` on Board, the editor's own text on Raw): `BoardView::open` or `reload` succeeding gives Board and empties the editor (`replace_text("", false)`); a parse error gives Raw, with the text in the editor as for Markdown and a banner naming the line. `sync_editor` runs only when an op produced events or an error (`app/workspace.rs:376-378`), so an app test asserts that every Session-driven text change emits an event.
- **Raw to Board.** A raw board flips back when the note is next opened and on a Session-driven text change (an external edit that now parses). It never flips while the user types in it. The banner is a strip of its own at the top of the editor column, not the window's warning slot, so it cannot be dismissed and never displaces a save or settings warning: "This board is shown as text because line L, column C is not valid: MESSAGE. It opens as a board again once the text is valid." The strip is always in the tree, at height 0 unless Raw, so a Markdown note's widget tree and pixels are unchanged. A file that is empty or holds only comments is a valid empty board (§4.1), so Markdown whose only line is a heading, saved under a board's name, opens as an empty board; nothing is written either way.
- **Only the visible surface may write.** `ensure_editor_ready` (`app/workspace.rs:857`) fails on Board. `command_enabled` (`app/commands.rs:480`) is kind-aware: Editor-scope commands and Outline need Text, Find needs Text or Raw (until Phase 3 routes both to the jump palette on a board, §5.18), a Board command needs Board, a mutating one a writable board. `text_changed` (`app.rs:595`) returns early on Board. `apply_editor_engine` (`app/preferences.rs:285-296`) passes `""` on Board. `step_text_size` (`:368`) zooms the board and writes no setting. `reveal_literal` runs only on Text or Raw (`app/palette.rs:1049`). `open_palette(Mode::Outline)` checks the same as the command, since a palette message or the harness can open it without the command; `editor.toggle_markdown_assist` and `editor.toggle_engine` stay enabled on every surface, as settings. Markdown assist is off while Raw (the palette row still shows the setting). Without this, palette "Bold" with a board open would overwrite the board with the hidden editor's text. The A4 item records a table of every `self.editor` site in the app (51 outside `test_requests.rs`, three more than `grep` finds, §8.5) classed as safe on Board, guarded, or unreachable.
- **The setting.** `boards = on | off` in `config.conf`, parsed with the other booleans (`on`, `yes`, `true`, `off`, `no`, `false`, any case), default off and not written into the template, read at launch into `SessionOptions { boards }` and never changed in a run. A value in the file other than the launch value shows "Restart Wren to apply `boards`." in the banner, after any settings warning, and goes when the file is back at the launch value; it is not a settings warning, so a save while it stands still completes. With it off no board is scanned, listed, indexed, created or opened, no board command or pill is listed, and the state file is byte-identical to today's.
- **Footer and issues.** On Board the footer (beside `editor_footer`, `app/workspace_view.rs:984`) reads `Board · 15 nodes · 17 connectors` (hidden under 440 px, as the Markdown counts are), then in the warning colour `N issue(s)` when `check` found any and `Read-only` for `Access::ReadOnly`, then the zoom (`47%`), then the save status; no line and column. On Raw it reads `Board as text · N lines` with the line and column. A flagged element carries a marker drawn by the scene in the `issue` colour, and `board.get` lists the issues. There is no issue list in the UI in the MVP.
- **Viewport.** The app keeps the viewport of each board opened in this run, in memory, by note id. A board opens fitted the first time (§5.13 rule 4) and returns to where it was after a note switch or a reload. A board put away while its first fit still waited for the widget's size was never looked at, so it fits again the next time. The map is cleared when a workspace opens. Nothing about the view is stored on disk.
- **Export SVG** (`note.export_svg`, §5.19). It makes the SVG at once from the view's scene and the current theme's palette, with a canvas-colour background and `FIT_MARGIN` of padding, so what is written is what was on screen when the command ran; then the save dialog opens (the note's title plus `.svg`, an SVG filter). One export waits at a time; a cancelled dialog writes nothing. The file is written to a new temporary file beside the target and renamed over it, so the target is whole or untouched; a failure is the banner "Could not export the board to PATH: ERROR". A destination is refused, with the same kind of banner and nothing written, when its name is a note's (`NoteKind::from_name`, any case, in any folder), or when it lies inside the open workspace and is not an `.svg` file, which also protects `.wren/workspace.toml`: those files are written only through the Session, which keeps a copy of what a write replaces, and an export does not. The dialog and `board.export` share that check.

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
- `ops.rs`: `Op::CreateNote { group, title, kind }` (`:25`, `:561-585`; 21 Rust use sites and one harness decoder, `test_requests.rs:846`, which defaults to Markdown); `Session::relocate` (`:1377`) takes the leaf from the source path's kind; `create_conflict` (`:1138`) names the copy `Flow (conflict …).wren-board`. `Op::CreateNote` refuses a board, writing nothing, when the Session does not list boards ("Boards are turned off; set `boards = on` to create one"), since the next rescan would drop it from the model. `SessionOptions { boards }` (default off) is given to `Session::open_with(path, options)` or `Session::with_options(path, trash, options)` beside `open` and `with_trash`, which still scan Markdown only. It reaches the three scans (`ops.rs:270`, `:1498`, `reconcile.rs:94` through `ReconcileJob::new`, `:60`) as a `scan::Kinds` value (`MARKDOWN` or `ALL`), so no scan runs without one, and a Session made by `Op::OpenWorkspace` inherits it as it inherits `background_reconcile` (`ops.rs:741-747`). A board at the workspace root gets "board at workspace root ignored", and one directly in a section the message a Markdown file there gets.
- **Same title in both kinds.** `Flow.md` and `Flow.wren-board` may share a group: collisions are decided by the full file name, as today. The notes list tells them apart by the kind icon, shown on board rows only. A rename collides only with a file of the same full name.
- **The state-file rule.** Released Wren (0.1.0, 0.1.1) rejects the whole `.wren/workspace.toml` if `history`, `recent` or `last_open_note` names a non-`.md` path (`validate`, `workspace_state.rs:568-656`). So:
  1. The v1 keys `history`, `recent`, `last_open_note` and `history_position` hold Markdown paths only. They are written as the Markdown projection of the real lists: boards filtered out, `recent` still unique, `history_position` re-pointed at the last Markdown entry at or before the real position and removed if there is none (`:604-620` states the two invariants). When the open note is a board, `last_open_note` is left as it was.
  2. `selection` is already `Reference::Any` (`:111`) and may name a board; `load` tries it first (`ops.rs:347-357`), so a board reopens. `Session::open` then tries `last_open` of any kind and last the v1 `last_open_note`, and `resolve_selection` tries the same two in a group, so with the setting off a workspace last left on a board reopens the Markdown note released Wren would.
  3. A new table `[navigation]` (`history`, `history_position`, `recent`, `last_open`) holds the mixed-kind lists. Its keys are written only while one of the lists names a non-Markdown path and removed otherwise, and the table goes once it is empty, so an unknown key a later build puts there survives, and a workspace with no boards, and every user with the setting off, gets a state file byte-identical to today's. One setter, `set_navigation(history, position, recent)`, writes both forms, the v1 position being a projection of the history; the accessors `history()`, `history_position()`, `recent()` and `last_open_note()` return the lists of every kind. With the setting off, the first save drops the boards from the lists and removes the table, so their places in Back and Forward are not kept for when it comes back on.
  4. `[navigation]` is read leniently: a bad entry is skipped, a wrong-typed value ignored, and nothing in it can call `disable` (`:336`). It is trusted only if its Markdown projection equals the v1 keys; otherwise an older build has navigated since, the v1 keys win, and the stale table is dropped from memory at load and from the file at the next save. Only a Markdown `last_open` must equal `last_open_note`: a board or an empty `last_open` constrains nothing (deleting the open board empties `last_open` and leaves `last_open_note` on the earlier Markdown note).
  5. `transform_paths` (`:373`) remaps and prunes `[navigation]` by the same rename and delete rules, and derives the v1 lists from the result rather than transforming them apart, because the v1 rule can move a pruned position onto a board and two separate transforms would then disagree.

### 3.6 Search

`index::Content` (`index.rs:73`) keeps the raw source for change detection and the extracted text for searching: `source: Arc<str>` beside the searchable `text`; `set_content` (`:390-399`) compares `source` first; `content_snapshot` (`:347`) returns `source`. A private `searchable(kind, source)` returns `None` for Markdown (search the source itself: one shared allocation, byte for byte what it was) and, for a board, `wren_board::search_text` (node labels in reading order, top then left by absolute position, ties by id, then connector labels, connectors in id order, one label per line with its own line breaks kept; a label of only white space and a placeholder's caption are left out, and labels are indexed whole, not cut at what the scene draws). An unparseable board, and from Phase 3 one over the limits, indexes its raw text, in the one allocation of its source. The index keeps each entry's kind and drops a content whose kind changed. A hit's range is in the searched text, not the file. Board previews and snippets skip `body_line` (`:557`) and `plain_markdown` (`:754`); the notes-list preview is the first label (empty when there is none), and a snippet shows each line break as ` · `. A hit reveals the first matching label (§5.18 rule 4).

### 3.7 Scene rules and constants

The scene is the single source of what a board looks like: the widget, the SVG and the PNG all come from `scene::build`. The values below are the starting values. Items M4 and M5 may change them, must record the final values here, and must do so before any scene or SVG golden file is blessed; after Phase 1 a change to any of them is a visible change to every exported board.

| Constant | Value |
|---|---|
| Label text [final, M5] | Inter Regular (bundled, `wren-iced/src/fonts.rs:46-50`), 14 px, line height 20 px, at fixed logical sizes independent of editor font settings (`scene::LABEL_SIZE`, `LABEL_LINE_HEIGHT`). The scene lays lines out by `scene::line_height(size)`, not by `Measure::line_height`, so the SVG writer, which has no measurer, places them identically; a host measurer returns the same heights |
| Connector label, placeholder caption [final, M5] | Inter Regular 12 px, line height 16 px (`SMALL_SIZE`, `SMALL_LINE_HEIGHT`). A connector label is broken at line feeds only, never wrapped, each line centred on the route's point at `at` (per-mille of the route's length); it sits on a canvas-colour rectangle padded 4 px across and 2 px down (`EDGE_LABEL_PAD`), unstroked. The first 16 labels are drawn (`MAX_LABELS`). A caption is the kind name, broken at line feeds only, 4 px in from the box's top-left corner (`CAPTION_INSET`) |
| Label box by shape (inset from the node box) [final, M4] | rect, rounded: 8 px sides, 4 px top and bottom. pill: `h/4` sides, 4 px. ellipse: 15 % of the width each side and 15 % of the height top and bottom. diamond: `w/6` sides, `h/6` top and bottom. parallelogram: the skew plus 4 px sides, 4 px. cylinder: 8 px sides, the cap height (twice the half-height, so the label starts below the cap's rim) on top, 4 px below. document: 8 px sides, 4 px top, the wave height (twice the amplitude) below. Insets that meet leave an empty box at the centre |
| Label placement [final, M5] | Wrapped at the label box width with the measurer: a line breaks after a run of white space (a no-break space joins), and a word wider than the box alone is broken between graphemes, at least one per line. Trailing white space is dropped from each line; a blank line keeps its height and draws nothing; a label of only white space draws nothing. Each line is centred across and the block centred down in a shape. Text that overflows the box is drawn in full, never clipped or cut with an ellipsis; the stored `size` is never changed by drawing. Only the first 16 KiB of a label is drawn, cut at a character boundary (`MAX_LABEL_BYTES`) |
| `kind = "text"` [final, M5] | No outline and no fill; the wrap box is the node box with no inset; left-aligned, top-aligned, in the node's colour (the hue, `ink` for `neutral`) |
| Stroke [final, M4] | 1.5 px for outlines and connectors; dashes 6 on, 4 off (`route::STROKE_WIDTH`, `route::DASH`) |
| Shape parameters [final, M4] | rounded: radius `min(8, w/2, h/2)`. pill: radius `min(w, h)/2`. parallelogram: skew `min(w/4, h/3)`, the top edge shifted right. cylinder: cap half-height `c = min(h/6, 12)`; the outline is the top half of the cap ellipse, the sides and the bottom half-ellipse; the cap's lower rim is a separate stroke-only path (`shape::detail`), never filled, so no fill rule matters. document: wave amplitude `a = min(h/8, 8)`; the sides stop at `h - a` and the bottom is two quadratic halves, raised on the right and dipping to the box's bottom on the left. Arcs are cubic quarter ellipses |
| Arrowhead [final, M4] | A filled triangle 10 long and 8 wide in the line colour, its tip at the route's end and pointing along the last segment, the line ending at its base. A route shorter than its heads shares its length between them |
| Connector ends [final, M4] | A side end meets the shape at the middle of that side, on the outline: the box's side middle, except a parallelogram's slanted sides (their midpoints) and a document's bottom (the wave's centre, `h - a`). An `anchor` is that point of the box, clamped to 0 to 1000, and leaves by the nearest side of the box (the nominal side among equally near ones, else top, right, bottom, left). A floating end takes the nominal side (§5.12 rule 4) whenever the two boxes overlap on one axis; when they are apart on both axes an elbow route's floating end may take any side, nominal first (Routes, below), and a straight route's keeps the nominal side. A free point's direction is always the nominal side |
| Routes [final, M4; obstacles, R1] | `straight`: one segment between the nominal ends, whatever lies between. `elbow`, the plain route of a pair of sides: facing sides, a Z through the middle of the gap; perpendicular sides, one turn when the corner lies out of both sides; the same side of both, a U 20 px beyond the further box; otherwise each end runs 20 px out of its side and the route turns between the boxes, or 20 px past both when they overlap on that axis. Ends that meet give a route of no length. Straight runs are merged and repeated points dropped. **Choosing a route** (owner decision of 2026-11-12, §2.4): a floating end may take any side when its two boxes are apart on both axes, else only its nominal side (so §5.12 rule 4 holds); pinned sides, anchors and free points are fixed. For each allowed pair of sides, nominal pair first, the candidates are the plain route's shape with its channel moved to the middle of the gap, 10 px either side of it (`route::NUDGE`), 20 px out of either end, or 20 px outside one of the 16 boxes nearest the ends within 80 px of them (`route::NEAR`; of both ends together, or of each end alone when that area spans more than 64 cells) (at most six positions per channel, four for each of the two channels of a three-turn route, nearest the plain one first), the one-turn route, a Z blocked in its gap taken out, around and back in, and detours pushed past the boxes they cross (at most four pushes). A candidate must leave its first point straight out of its side and arrive straight in through the other's; the plain route of the first pair is always a candidate. The winner enters the fewest node boxes other than its own two (grown by 1 px; its own only by the segment that leaves or arrives), then has the lowest cost: its length, plus 40 per turn (`TURN_COST`), plus 200 for each segment on another's line (`SHARED_COST`: a middle segment within 2 px of the centre line of a node no more than 120 px along it, `REACH`; or any segment sharing more than 4 px of a line already drawn), plus 200 for each line already drawn that it crosses (`CROSSING_COST`); ties go to the earlier candidate. A route that crosses something everywhere still draws its best candidate. **Order** (`route::route_all`, connectors in id order): connectors with no side to choose (straight ones and fixed ends) are routed against the node boxes alone; those with a choice are routed, in id order, against the boxes and the lines already drawn (the fixed connectors' and those of the free connectors before them); then each middle segment of an elbow route that shares more than 4 px of a line already drawn (a straight connector, the first or last segment of any connector, an earlier connector's middle segment) moves aside by 10, 20 or 30 px, either way, to the first place on no other line that enters no more boxes and keeps every segment's direction (a first or last segment at least 10 px), preferring fewer crossings, then the smaller move, then the longer neighbour; where there is none it stays. Each connector is routed on its own against a lookup of the node boxes (256 px cells). **Budget**: the lookup is bounded per connector, so routing a board grows with its connectors, not with connectors times boxes. Each cell visited and each box or line looked at costs one unit; a connector may spend 12,288 while it is routed (`route::BUDGET`) and 1,024 while it moves aside (`SEPARATE_BUDGET`). One that runs out keeps the best route it has fully scored, else the plain route of its nominal sides, and in the separation pass stays where it is. Checkout flow's costliest connector spends 7,152 and its separation at most 282 |
| Unknown kind [final, M5] | A dashed rectangle in `muted`, unfilled, whatever its `shape`, with the kind name as a caption at the top left in `muted` and the label centred in a `rect`'s label box in `ink`. A node with no `kind` is the same placeholder with no caption. A connector of unknown or no kind draws as a connector, from its own route, heads, labels and style |
| Dangling end [final, M5] | A 24 px stub from the valid end in `issue`, out of its pinned side, the side its anchor faces, or its nominal side: right at a `from` end and left at a `to` end (the rule's tie with no other node). It ends in a cross stroke 8 px long, square to the stub and centred on its tip. Nothing is drawn when both ends dangle. An issue marker is a 6 px dot in `issue` centred on the element's top-left corner: a node's box, a connector's drawn line or stub. Markers are drawn after every element, one per flagged element in `check`'s order, so no shape covers one; a connector with both ends dangling has none |
| Colour resolution [final, M5] | `neutral` uses `ink` for stroke and label. `none`: stroke only. `tint`: fill is the hue (for `neutral`, `ink`) mixed into `canvas` at 16 % in a dark theme and 12 % in a light one, per channel, rounded; label in `ink`. `solid`: fill is the hue (for `neutral`, `raised`); the label is whichever of `ink` and `canvas` has the higher WCAG contrast on it, `ink` on a tie. A literal `#rrggbb` is used as the hue unchanged. An unknown colour draws as `neutral`, an unknown fill as `tint`, an unknown stroke as `solid`. A connector's line, arrowheads and label text take its stroke colour (the hue, or `ink`) |
| Out-of-range values [final, M5] | Drawn clamped, the stored values untouched (§4.12): a node's resolved corner to −16,777,216 to 16,777,215 and its size to at least 1 and at most what keeps its far edge within 16,777,216, so every corner of a box is an integer `f32` holds exactly; a free end to ±16,777,216; an anchor and a label position to 0 to 1000; a `z` compared by its first 64 bytes (`MAX_Z`) |

Other scene rules: the parent transform is applied from the first version (nested, through an unknown kind; a node in a cycle is laid out as a root); order is `z` bytewise, ties by id, among siblings, children directly above their parent (§4.7); a floating connector end is drawn on its nominal side whenever the two nodes overlap on one axis and are apart on the other (§5.12 rule 4); routes are computed and never stored, a pure function of the model, and elbow routes keep out of other shapes and off other connectors' lines where the bounded rule of the Routes row allows, and only there. The default node is 120 by 60 (§5.5 rule 2). A Phase 1 test under the real measurer (item V2) asserts that each of the fifteen labels of the acceptance script (§5.20) fits the label box of the shape the script gives it without growing the node, and that `Notify customer` fits every shape but the cylinder. It is 106.1 units wide in Inter at 14 px, two more than the rectangle's label box, so it takes two lines (40 units) in every shape: they fill the diamond's box exactly and do not fit under the cylinder's cap (36). The script uses no cylinder, so no constant changed; changing the cylinder's inset or the default size instead is still open until the Phase 2 golden layout is written. If the test fails, the label box or the font size changes first and the default size last, before the fixtures and that layout are written.

### 3.8 Limits for untrusted files

Loading never writes and never panics, from Phase 1: no indexing or `unwrap` on file-derived data, checked integer conversions, parent sums in `i64`, the `toml` crate's nesting limit of 80 (applied also to unknown content as the canonical writer spells it, §4.12), parent walks bounded by a visited set. The caps of §4.12 arrive in Phase 3: above 8 MiB or 50,000 elements a board opens as raw text and is never parsed by the index; out-of-range values are flagged and drawn clamped. The scene's size follows label length over box width, not file size (a long word in a 1 px box is one line, and one SVG `<text>`, per grapheme: 200 such 16,000-character labels in a 3.2 MB file reach 667 MB resident), so Phase 3 also caps the lines drawn per label, as `MAX_LABEL_BYTES` cuts its text. Coordinates inside the valid range must not reach a renderer in the millions of pixels, where tiny-skia panics: the canvas cuts every path that reaches more than 2,048 px past the widget before drawing it (`clip.rs`, in `f64`): a sub-path wholly outside is dropped, a fill is clipped as an area along the window's edge, a stroke where it crosses, curves are split, and a cut dashed stroke restarts on a whole dash period so its dashes do not move. Each label line is laid out in bounded room, its measured width plus one em times the zoom, capped at 2^24 px, and with `Alignment::Default`, so a right-to-left line is placed from its own box instead of past every glyph position. The general 64 MiB rule is unchanged (`fs_store.rs:27`). SVG and PNG export escape text and attributes, strip the characters XML cannot carry, emit a literal colour only if it matches `#rrggbb`, and never emit `link` as an `href`. PNG export refuses more than 64 megapixels and resolves no image or external reference. Ring 0 reads no `file`, `src` or `link`, so the rule that references resolve only through `RelPath` and `FsStore`, and the attachments folder (`Section/Group/assets/`, content-hashed names), apply from ring 1.

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
| `route` | string | optional | `elbow` | `straight` or `elbow` (right-angled). Reserved: `curved`. Any other value draws as `straight`. The path is computed from the board (the endpoints, the other nodes' boxes and, for an elbow route, the other connectors' lines, §3.7) and never stored. |
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
- Label text (a node's `text`, a label item's `text`) is plain text, never Markdown. On load, CRLF and a lone CR inside it become LF; nothing is trimmed. The writer does the same to label text, so a model built in code with a CR in a label (a paste, Phase 2's label editor) writes what the loader reads back and §4.11 rule 10 holds; every other string, unknown content included, is written as it was read. Other control characters are kept, and export strips the ones XML cannot carry.
- Integers are decimal, with `-` as the only sign, no leading zeros and no separators. Booleans are `true` and `false`. Floats, in unknown content only, are written as Rust formats `f64` with `{:?}` (`1.0`, `0.25`, `-0.0`, `1e21`), and `nan` (of any sign or payload), `inf`, `-inf`. The type read is the type written: `1` stays an integer and `1.0` a float. Dates and times use the crate's `Display`, except that a time read without seconds (TOML 1.1's `07:32`) is written with `:00`, because TOML 1.0 requires them.

### 4.10 Unknown content and `requires`

- Kept through load and save, with value and type: unknown top-level keys; unknown keys in a node, an edge, `style`, an endpoint or a label item; everything under `ext`; and unknown values of the string fields (`kind`, `shape`, colour, fill, stroke, `route`, `side`, heads). They draw as the tables say and are never an error and never flagged.
- An element of unknown kind keeps all its fields. Its envelope is still read and must be well-typed: every node has a box (`at`, `size`) and every edge two endpoints, whatever the kind, so a placeholder sits in the right place and the parent transform passes through it. A known field with the wrong type is an error, not unknown content. TOML has no null, so importers keep nulls out of `ext`.
- `requires` lists feature names (`[a-z0-9-]{1,32}`), written sorted (not deduplicated) and omitted when empty. Ring 0 defines none. An entry outside that pattern is not an error: it is a feature this build does not know. A board naming a feature the build does not know opens **read-only**: it draws, pans, zooms, selects, walks, copies, jumps and exports (app and CLI), and never sends a draft. A `schema` other than 1 that still loads under these rules is read-only in the same way (only one above 1 can come from a file), and when both apply the reason given is the schema.
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
9. A known optional field equal to its default is omitted: empty `text`, `shape = "rect"`, `style.color = "neutral"`, `style.fill = "tint"`, `style.stroke = "solid"`, `route = "elbow"`, `heads = ["none", "arrow"]`, empty `labels`, a label's `at = 500`, empty `requires`. Defaults are compared by the written string, so an unknown value built in code that spells a default is omitted, and a literal colour is written lower case, as a file would read them. A missing `kind` or `z` stays missing. Unknown content is written as read, except a key a typed field owns (`at` in a node's unknown content, `fill` in a node style's, `node` at the top level, `at` in an attached end's): only a model built in code can hold one, the typed field wins, and the output always reads back.
10. The output is a function of the model alone: not of source order or layout, platform, locale or map iteration order. `write(parse(write(m))) == write(m)`.

### 4.12 Loader

`parse` then `check`. The loader parses with `text.parse::<toml::Table>()`, as `config.rs:111` and `workspace_state.rs:50` do, without normalising the text first. That accepts any key and block order, comments, CRLF line endings, a BOM, sub-table and array-of-table layouts, every string form, and the TOML 1.1 syntax the crate allows. A clean hand merge in git can leave blocks out of order; that is valid input. It then builds the model, with known fields typed and everything else kept as `toml::Value`.

**Errors.** The board opens as raw text with the message and a line. Parser errors carry a span; for the others the loader re-parses with `toml::de::DeTable::parse`, which keeps spans, on the error path only. The line and column are those of the key for a field, of the item for an array item, of the id in its header for a record missing `at`, `size`, `from` or `to`, and line 1 for a missing `schema`; the column counts characters, a leading BOM excluded.
- Not valid TOML, including a duplicate key or header and nesting deeper than 80. A key or header of more than 80 dotted parts, which the crate reports without a position, is found by the loader and named by its line.
- Unknown content that the canonical writer would spell past the crate's limit: a dotted key of more than 80 parts, or more than 80 arrays and inline tables open at once. The crate counts a header layout and a dotted key apart from inline nesting, so `[[y]]`, `[[y.y]]`, … 41 deep, or inline tables 80 deep, load and yet write as text that does not; such a file is refused at that field rather than saved unreadable after an edit.
- `schema` missing from a file that has other keys, not an integer, or below 1. `requires` not an array of strings.
- `node` or `edge` is not a table, or one of its entries is not a table. An id that does not match §4.2.
- A known field of the wrong type: a float, `nan`, `inf` or a string where an integer is required, an integer outside `i32`, a pair (`at`, `size`, `anchor`, an endpoint's `at`, `heads`) without exactly two items, a label item without `text`. In a free end, `side` and `anchor` are unknown content. (`nan` and `inf` are legal in unknown content.)
- A node without `at` or `size`. An edge without `from` or `to`, or with an endpoint holding both or neither of `node` and `at`.
- Phase 3: a file over 8 MiB or with more than 50,000 elements.

**Flagged.** The board opens and is editable; the issue is listed (§3.3); the content is kept byte for byte through a save; nothing is repaired without a user action on that element.

| Issue | Drawn as |
|---|---|
| An endpoint names no node (one issue per connector, naming which ends dangle) | A short stub from the other end; nothing if both ends dangle. |
| `parent` names no node | A root node, its stored `at` taken as absolute. |
| A parent cycle, or a chain deeper than 32 (a root is level 1) | Every node in a cycle, and every node past level 32, as a root node with its stored `at` as absolute; the first 32 levels draw as written, and a node hanging from a cycle without being in it is drawn inside its laid-out parent. |
| A node and an edge share an id (one issue on each) | Both, normally. |
| Missing `kind` | A node as a placeholder; an edge as a plain connector. |
| Missing `z`, an empty one, or one with a character outside `[0-9A-Za-z]` (its length is the out-of-range row's) | In bytewise order of what is there. |
| Out of range: resolved geometry (§4.6), a size below 1, an `anchor` or label `at` outside 0 to 1000, a label over 16 KiB, more than 16 labels, a `z` over 64 characters; from Phase 3, a label that wraps to more lines than the per-label cap (§3.8) | Clamped or truncated, from Phase 1 (the line cap from Phase 3). Listed as issues from Phase 3. |

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
3. **Round trip** [P1]. 500 random valid models [20,000], whose labels may hold CR and CRLF: `write(parse(write(m))) == write(m)`. 2,000 generated strings [20,000] and the float set survive `write` then `parse`, and no written line holds a raw control character.
4. **One line per edit** [P1, at model level by mutating the model directly; P2 repeats it through intents]. Move, resize, relabel (multi-line included), recolour and restack each change exactly one line; adding an element adds exactly one block, and deleting one removes it.
5. **TOML 1.0 outside Rust** [P1]. A unit in `tools/wrentest_unit/` parses every `.out` file with Python's `tomllib` (`requires-python >= 3.12`) and, for the files the reference writer can represent, checks that it writes the same bytes.
6. **Mixed version** [P1]. `future.out.wren-board`, with the fields of §4.5, an unknown kind, unknown enum values, and unknown keys that sort before, between and after the known ones at every level, round-trips byte for byte, and an edit to one known field changes one line. A second case checks the key order of every block against a plain sort computed in the test, with no reference to which keys the code knows.
7. **Every error rule** of §4.12 [P1], table-driven: input, message fragment, line. With them: an unknown `requires` entry and `schema = 2` give read-only; `schema = 0`, `schema = "1"` and a missing `schema` beside other keys are errors.
8. **Every flagged rule** of §4.12 [P1]: the issue is reported, the board draws as the table says (resolved geometry for a missing parent, a self-parent, cycles of two and three, a chain of 33; a child's position through an unknown kind), and `write` returns the input bytes.
9. **Never panic** [P1]. 2,000 seeded byte and line mutations of the golden files [100,000], and random `toml::Table` values with a wrong type in each known field: `parse` returns, and whatever loads also checks and writes. Depth 81 is an error, which guards the `unbounded` feature.
10. **Limits** [P3]. Each cap at its value and one past it: file size, element count, coordinate range, parent depth, label size, label count, z length, lines drawn per label.
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
4. **Ends and sides.** An end is attached to a node, floating (the UI's default: no `side`) or pinned (`side` set), or is a free point. The **nominal exit side** of a connector at its `from` node is down when the `to` node lies wholly below the `from` node's bottom edge, up when wholly above its top edge, and otherwise right or left by centre; the nominal entry side is the opposite one. The **side of a connector at a node**, which `board.select.connector_<dir>` uses, is the pinned side, else the side the router draws. The router must draw a floating end on its nominal side whenever the two nodes overlap on one axis and are apart on the other (the acceptance script depends on it). Where they are apart on both axes, an elbow route's floating end may take any side, nominal first, so a diagonal with nothing in the way is one turn rather than a Z (owner decision 16; §3.7, Routes); a straight route keeps the nominal sides.
5. **Pinning.** An end released within 10 screen px of one of the target's side handles is pinned to that side; released anywhere else on the node it floats. Nothing else in ring 0 writes `side`, and nothing writes `anchor`.
6. **Endpoint drag.** A single selected connector shows a handle at each end. Dragging one onto a node re-attaches it (rule 5). Released on empty canvas, an end that was already a free point moves there, snapped; an attached end returns to where it was. The UI never creates a free end; they come only from files.
7. **Re-routing.** Routes are recomputed from current geometry whenever an attached node moves or resizes, during the drag as well as after it. A floating end may change side; a pinned end does not. Moving a node changes no edge line in the file.
8. **Label.** One label per connector in the ring 0 UI: the first entry of `labels`, at the middle of the route. `board.label.edit` or a double-click edits it (§5.9). Further labels in a file are drawn and kept.
9. **Style commands** act on every selected connector, one undo unit each: `board.edge.routing` switches straight and elbow; `board.edge.start` and `board.edge.end` cycle that end's head through none, arrow; `board.edge.reverse` swaps `from` and `to` with their heads; `board.stroke.dashed` switches solid and dashed. In Phase 2 they are reached from the Commands palette.
10. Deleting a node deletes its connectors in the same unit. A second connector between the same pair may be drawn by pointer. A dangling connector in a loaded file is never repaired or dropped by an unrelated edit.

### 5.13 Zoom and pan

1. [P1] **Wheel.** Unmodified, it pans; with Mod held, it zooms about the pointer. The rule is the same for `Lines` and `Pixels` deltas, because X11 reports touchpads as `Lines`. `Pixels` are divided by the window scale factor; one line is 40 logical px. Shift with a `Lines` delta pans sideways. Zoom multiplies by `1.1` per line, within 10 % to 400 %.
2. [P1] **Burst latch.** Wheel events less than 150 ms apart are one burst, and the burst keeps the pan-or-zoom choice of its first event; iced drops the scroll phase, and macOS momentum outlives the modifier.
3. [P1] **Drag to pan:** the middle button anywhere, and Space held with a drag. [P2] The hand tool. Space types a space in Editing and in the connect query. [P1] The focused board takes unmodified Space and the four arrows (Shift allowed); in Phase 1 the arrows are captured and do nothing. A key with Ctrl, Alt or the logo key is the app's. Key repeat of Space sends nothing; a Space release counts when focus has gone elsewhere; letting Space go mid-drag does not end the drag, its button's release does. Losing the window's focus or a click off the board sends `Cancel`: a drag stops and Space counts as up. A release the widget did not see pressed is still forwarded while the machine is panning, uncaptured, and a press during a drag starts a new one, so a drag whose release an overlay took never sticks.
4. [P1] A board opens fitted to its content with a 40 screen px margin, never above 100 %; an empty board opens at 100 % with the board's origin at the middle of the widget, where Phase 2's first Tab node lands (§5.6 rule 5); a widget smaller than its margins fits at 10 %. A fit asked for before the widget has reported its size waits for it, with only the canvas colour painted meanwhile; a viewport set or a zoom before then replaces it. Returning to a board opened earlier in this run restores its viewport (§3.3).
5. [P1] **Keyboard zoom** is the text-size commands: `font.increase` and `font.decrease` step through 10, 25, 50, 75, 100, 150, 200, 300, 400 % about the viewport centre and `font.reset` is 100 %. A zoom within 1 part in 10,000 of a stop counts as that stop; otherwise a step goes to the next stop in its direction. Their keys are ⌘= (or ⌘+), ⌘- and ⌘0 on macOS and Alt+= (or Alt++), Alt+- and Alt+0 on Linux (`keybind.rs:232-237`). There are no separate board zoom-in, zoom-out or reset commands. [P2] `board.zoom.fit` fits all content and `board.zoom.selection` the selection, both with the margin of rule 4 and never above 100 %.
6. Zoom and pan are view state: never in the board file, never a draft, never an undo unit. The wheel works on an unfocused board when the pointer is over it. There are no pan keys in ring 0; walking reveals the cursor (§5.7 rule 5) and the fit commands cover the rest.
7. Pinch does not arrive today: `vendor/iced_winit/src/conversion.rs:214-231` forwards only `MouseWheel`. A side channel is not built in the MVP (§2.4 question 15).
8. [P1] **Labels on the software renderer** (the spike's fallback, S3 and S5). Geometry is rebuilt at every pan and zoom step, never moved by a renderer transformation. Where the renderer is `tiny-skia`, labels under 4 screen px are not drawn, at rest either, and no label is drawn while the view moves: from a wheel step, a drag or `set_viewport` until it has rested 150 ms (`REST`). A zoom step, a reset or a fit is drawn in full at once. On `wgpu` labels are always drawn. So one wheel click on tiny-skia leaves the labels out for 150 ms, and at 25 % a 14 px label (3.5 px) is not drawn there.

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

1. [P1] The board is the editor column. Any board input other than the wheel and `Cancel` (which the widget sends on blur; it is not the user acting on the board) sets `self.column` to the editor column, as editor actions do (`app.rs:567-571`). `Message::Resized` is not input. Nothing changes the column while a dialog or palette is open, though a release that ends a drag the overlay interrupted still reaches the machine; a message with no board open is dropped.
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

`note.export_svg` is enabled only on a Board surface with no export waiting for its dialog, and with the setting off it is neither listed nor runnable from any path (`CommandId::boards()` names the commands that exist only with the setting on; §3.3, Export SVG). That is 64 Board commands in Phase 2 and 25 in Phase 3. ⌘[ and ⌘] stay Back and Forward and ⇧⌘[ and ⇧⌘] stay the column keys (`keybind.rs:224-227`). Ctrl+Arrow on macOS and Ctrl+Alt+Arrow on Linux are left to the desktop. `keys::accel` already maps shifted digits and brackets to their base key (`keys.rs:136-163`), so `⇧1` and `⇧]` need no change there.

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
board.bounds   {}                       -> {"root": RECT, "viewport": {"x","y","zoom"}, "scale_factor": 1.0,
                                            "nodes":   [{"id", "bounds": RECT, "visible": RECT|null, "label": RECT|null,
                                                         "fill": [r,g,b]|null, "stroke": [r,g,b]|null}],
                                            "edges":   [{"id", "points": [[x,y],...], "label": RECT|null, "stroke": [r,g,b]|null}],
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
                                        -> {"scene_micros":[...], "draw_micros":[...], "frame_micros":[...], "open_micros": n,
                                            "parses": n, "scenes": n, "builds": n}

SNAPSHOT = {"enabled": true, "surface": "board|raw|text"|null, "note": 7, "read_only": false,
            "error": null | {"line": 12, "column": 3, "message": "..."}, "issues": [{"kind":"dangling_edge","id":"e9"}],
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
- Phase 1 ships `board.get`, `board.bounds`, `board.export`, `perf.board`, and `board.pointer` limited to `wheel` and a middle-button pan (`press`, `move`, `release` with `"button": "middle"`; `at` a node, a canvas point or a screen point). Phase 2 ships the rest. A chain that changes a request shape edits this block in the same commit.
- In Phase 1 `board.get`'s `state` is `panning`, `idle` or `unfocused`, `revision` is the view's epoch (its reloads, there being no drafts), `pristine` is always true, and the fields of a surface that is not a board are `null` or empty. `board.bounds` gives each element's fill and stroke as drawn and the window's scale factor, which the pixel checks of §6.8 Phase 1 item 3 need; rectangles are in the window's logical pixels.

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
| Frame build, 1,000 labelled shapes, each renderer | `scene_micros` and `draw_micros` of one change of the whole scene (a theme switch) and the draw that paints it: ten changes back to back, the first three left out, median of seven | scene + draw 8 ms; replaced by 10 ms at 100 % and 12 ms fitted (§8.5, `boards-p1-canvas`). Relaxed for the MVP (owner, 2026-11-12): the release run, on a quiet machine, asserts 12 ms on tiny-skia and 16 ms on wgpu; measured at `boards-p1-tune` on a quiet machine 6.2 to 8.7 ms on X11 tiny-skia (scene 5.2 to 7.1, draw 1.1 to 1.5) and 8.9 to 12.9 ms on X11 wgpu under lavapipe (scene 6.8 to 9.8, draw 2.1 to 3.0), and on wgpu up to 17.4 ms while other work loaded the machine. 8 ms waits for Phase 2's per-element rebuild (§8.5, `boards-p1-tune`) | 100 ms median |
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

A missed budget is fixed or recorded with its number and reason; none is silently relaxed. Since the router of item R1, `make perf-board` also times boards on which every connector spends its whole routing budget (§3.7, Routes): long random connectors and a fan-in on the grid, and shapes stacked on one spot joined to one far shape. At 1,000 shapes they take 16 to 55 ms since `boards-p1-tune` (46 to 71 before it), asserted under 150 ms, missing the 5 ms row (about 1.3 ms before R1; the reason is in §8.5, `boards-p1-route`), and at about 10,200 elements 98 to 292 ms (110 to 428 before), asserted under 600 ms, within the 1 s open budget. Workspace open and `make perf-search` must be unchanged within noise against `main`, with the setting off and on.

The frame-build row is timed so that it measures the code and not the processor's power state. A sample taken after an idle pause runs on a core that has dropped to its idle clock: on the development machine (performance, efficiency and low-power efficiency cores under the powersave governor) one build measured 11.6 to 23.5 ms by where it landed when each change followed a 0.4 s sleep. `test_board_perf.py` therefore makes ten whole-scene changes back to back, polling for the draw that rebuilt the geometry instead of sleeping, leaves the first three out (the first is the cold one, about twice the rest), asserts one scene build per change, and writes the scene and draw halves and the left-out samples to `measurements.json`. Adding a 10 ms sleep to the scene build makes the release run fail on both renderers (18.5 to 19.1 ms on tiny-skia, 21.7 to 30.0 ms on wgpu). The ceilings follow the rule the owner set, keep 12 ms where every run's median is under it with room, else the smallest whole number of milliseconds about a quarter over the worst median, as far as a doubled cost still fails. The ceilings are set from medians measured on a quiet machine, and the release run needs one: a run beside other work is no measurement of the code (one tiny-skia run at a load of 13 measured 19.9 ms, and the next, quiet, 10.1), so `measurements.json` records the load average to tell such a run apart. On tiny-skia every quiet median was under 8.7 ms, so 12 ms stays; a doubled scene half (the measurer's sweep, `scene::build` and the layer split run twice) fails it on the efficiency and performance cores (12.3 to 15.8 ms) but passes it unpinned (10.4 to 10.9 ms), and no whole number the rule allows (11 ms) would catch that, so there the sleep is the check. On wgpu the quiet medians were 8.9 to 12.9 ms, so the ceiling is 16 ms, about a quarter over the worst. A doubled scene half then measures 17.7 to 20.9 ms unpinned and on the efficiency cores, and `scene::build` alone run twice 16.4 to 17.0 ms, and both fail; pinned to the performance cores both pass (12.7 to 14.0 ms), so on wgpu a doubling is caught unpinned and on the efficiency cores only. The 20 ms first set here, from a median of 17.4 ms taken while another process held about four cores, passed the doubled scene half unpinned (18.5 to 20.9 ms). The wgpu lanes run on lavapipe, a software renderer whose threads share the processor with the scene build, which is why the scene half is longer there and spreads more. Phase 2 should time with `taskset` on one core class or report both, on a machine with nothing else running.

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

Items 5 and 6, in run 1a.
- *The contract commit.* `wren-board` declares the model and scene types of §3.2: `Board`, `Node`, `Edge`, `Id`, `End`, `Label`, `NodeStyle`, `EdgeStyle`, `Color`, the closed-set enums, `Palette`, `Rgba`, `GRID`, `EMPTY`; `scene::{Scene, Item, Source, Draw, Segment, Stroke, Line, Point, Rect, Face, Measure, FixedMeasure}`. Shapes chosen here, which the Model chain may reshape inside §3.2's meaning: a closed-set string field is `Named<T>` (`Known(T)` or `Other(String)`, so an unknown value is kept); `parent` and an endpoint's `node` are plain strings, because a file may name something that is not a valid id and it is written back as read; unknown content is `Extra = toml::Table` on the board, each record, each style, endpoint and label item; a connector's style has no fill, so an edge's `style.fill` is unknown content; `schema` is `i32` like every known integer; a `Line`'s `origin` is the top-left of its line box; `FixedMeasure` makes each grapheme half the font size wide and a line 1.25 times it, both exact in binary. `wren-canvas` is an empty crate with its own iced features (wren-editor's plus `canvas`) and `test-support`.
- *Deviation:* `wren-iced` depends on `wren-board` and `wren-canvas` from the contract commit, and `smoke-test` enables `wren-canvas/test-support`, so the spike's `--board-spike` mode needs no `Cargo.lock` change (§3.1 has the dependency arrive with the app; A1 still adds `wren-core`'s).
- *Makefile:* the two clippy lines; `fuzz-board` and `perf-board` run the ignored `wren-board` lib tests whose names contain `fuzz_` and `perf_` (perf with one test thread), so M7 names its tests to match; `SMOKE_TESTS ?= tools/wrentest`. *Deviation:* `.gitattributes` has two lines, one per folder, each `-text -whitespace`, because CRLF inputs would otherwise be whitespace errors in `git diff --check`.
- *The reference writer* is `tools/board_format.py`. *Deviation:* the scratch copy did not implement all of §4, so it gained rule 9 of §4.11 (known defaults omitted), the lower-case `#rrggbb` of §4.8, `requires` written sorted (§4.10), CRLF and lone CR in labels read as LF (§4.9), and dates and times written as the `toml` crate's `Display` writes them; the self-checks pass unchanged. A board's `style` table left empty by rule 9, or written empty, is omitted (`defaults.in`).
- *Fixtures.* Not in the plan: `tools/board_fixtures.py` writes every fixture through the reference writer and `tools/wrentest_unit/test_board_fixtures.py` keeps the committed bytes equal to it, so a fixture is regenerated rather than hand-edited. The sample workspace has the six files of §6.1. `Checkout flow` is the keyboard model's layout in cells times 20 with Start at the origin, stacked in the order the acceptance script creates elements (a node, then its connector), with ids from a seeded draw. Golden files in `crates/wren-board/golden/format/`: test 1, `.out` alone: `password-reset` (the example, copied), `fields`, `shapes-and-ends`, `escapes`, `unknown`, `quoted-keys`, `values`, `requires`; test 2, `.in` and `.out`: `empty` (zero bytes, so also the empty board), `blank`, `comments-only`, `crlf`, `bom`, `comments`, `shuffled`, `subtables`, `strings`, `toml11`, `defaults`, `colour-case`; test 6: `future`. Every `.in` and `.out` and every sample board parses with the pinned `toml` crate (a scratch check outside the repository), which reads the floats and dates of `values.out` back to the strings written.
- *For the Model chain.* `tomllib` cannot read `bom.in` or `toml11.in`; their outputs were written from TOML 1.0 spellings of the same boards. Cases the two parsers read differently are left out of the fixtures: CRLF inside a multi-line string in unknown content (the crate keeps `\r\n`, `tomllib` gives LF) and a `+00:00` offset (`tomllib` cannot tell it from `Z`). Open point: a TOML 1.1 time without seconds (`07:32`) reads into a `Datetime` whose `Display` is `07:32`, which is not TOML 1.0, against §4.1; no fixture holds one, and the writer should probably add `:00`.

#### boards-p1-core
Items C1 to C3, session 1.
- *Shapes chosen inside §3.5.* `NoteKind::extension()` includes the dot (`.wren-board` is the 11 bytes of C2). `title` is a method, `kind.title(name)`, which returns the name whole when it does not carry that kind's extension (as `note_title` did for `.md`). Until C4 the scan asks a private `listed_kind`, which lists Markdown only; C4 replaces it with `Kinds`. `create_conflict` takes the kind from the open document's entry; `Session::relocate` takes the leaf from the source path's name, as specified, and returns an error for a name with no kind, which a scanned note never has.
- *Deviation: the op-by-kind table runs for Markdown only until C4.* Every structural op and every reconcile rescans the tree, and `Session::open` lists Markdown only until C4, so a board created by `Op::CreateNote` leaves the model at the next op. C3 writes the table once, `ops_keep_the_note_kind(kind, open_session)` in `tests/session.rs` (create, open, flush, rename twice, move, move and rename of its group, reorder, clean external reload, dirty conflict copy, back and forward, delete), and runs it for Markdown; C4 adds the Board run with `Session::open_with`. Until then `board_ops_write_board_names_and_bytes` checks C3's board paths on disk (the template bytes, a collision, the conflict copy's `.wren-board` name, rename and move names), and the `Flow.md` beside `Flow.wren-board` row is checked from the Markdown side (`markdown_ops_beside_a_board_of_the_same_title_collide_only_by_full_name`); C4 extends it with boards listed.
- *Not in the plan:* the harness's `create_note` decoder accepts an optional `"kind": "markdown" | "board"`, Markdown when absent; any other value is an error.

Items C4, C6 and C7, session 2.
- *C4 shapes.* `scan::Kinds` is a copyable value with `MARKDOWN` (the default) and `ALL`, `lists(kind)` and `of(name)`; `scan`, `scan_with_contents` and `ReconcileJob::new` take it, so no scan call is without one. `SessionOptions { boards }` (default off) is kept by the Session, read with `options()`, and passed to `load` by `Op::OpenWorkspace`. Besides `Session::open_with(path, options)` there is `Session::with_options(path, trash, options)`, the twin of `with_trash`, because the app (A2) and the tests open with their own trash. A board at the root gets "board at workspace root ignored" (the Markdown text names Markdown and is unchanged); a board directly in a section gets the Markdown file's message unchanged, which names no kind.
- *Deviation: `Op::CreateNote` refuses a board when the Session does not list boards* ("Boards are turned off; set `boards = on` to create one") and writes nothing. Otherwise the file would be written and then dropped from the model by the next rescan, which breaks "with it off no board is ... opened" (§3.3). C3's `board_ops_write_board_names_and_bytes` now opens its session with boards listed.
- *Deviation: the Board run of the op-by-kind table moves to C5.* With boards listed, opening a board puts its path into `history`, and `WorkspaceState::set_history` refuses the non-`.md` path and makes the state read-only (`disable`), so every later metadata op of the table (`Reorder` first) fails with "Workspace metadata is read-only". That is the state-file rule of §3.5, item C5. C4 runs the Markdown table again with boards listed (`op_by_kind_table_markdown_with_boards_listed`) and covers the board side of `Flow.md` beside `Flow.wren-board` with boards listed (create, rename and move of a board, and the Markdown row with both listed); C5 adds `op_by_kind_table_board` with `Fixture::board_session`.
- *C6 shapes.* `index::Content` holds `source` and the searched `text`; for Markdown both are one `Arc`, so retained bytes and search results are unchanged, and `retained_bytes` counts an extracted text only when there is one. The index keeps each entry's kind (from the model) and drops a content whose kind changed. `searchable` returns `None` for both kinds until A1. The board preview and snippet paths of §3.6 are in place now and read the searched text: the preview is its first line with words, trimmed and clipped as a Markdown preview is, and a snippet keeps every character and shows each line break as ` · ` (`plain_lines`, which shares the joining code factored out of `plain_markdown`). Until A1 a board's searched text is its TOML source, so its preview reads `schema = 1`; nothing lists a board in the app before A2. The test-only counter is `Index::extractions` (board sources that passed the change test); `ReconcileResult::sent_contents` is test-only as well.
- *C7.* `Settings.boards` parses with the other booleans (`on`, `yes`, `true`, `off`, `no`, `false`, any case, quoted or not), a bad value warns and keeps the earlier value, and `TEMPLATE` does not mention it (a test asserts that). Nothing reads it yet: A2 passes it to `SessionOptions`.

Item C5, session 3.
- *Shapes chosen inside §3.5.* The three list setters became one, `WorkspaceState::set_navigation(history, position, recent)`, because the v1 position is a projection of the history. `set_last_open_note` keeps its name. The accessors `history()`, `history_position()`, `recent()` and `last_open_note()` now return the lists of every kind: from a believed `[navigation]`, otherwise from the v1 keys. `Session::open` reads them unchanged. The "Markdown entry" test is the released validator's own test (`parse_path` with `Reference::Note`), so the v1 keys can never hold a path it refuses. The keys of `[navigation]` are the four of §3.5. When none of the lists names another kind, those keys are removed, and the table goes once it is empty, so an unknown key a later build puts in `[navigation]` survives. A stale `[navigation]` is dropped from memory at load. The file is untouched until the next save, which writes it without the table.
- *Rule 4, refined.* Only a Markdown `last_open` must equal `last_open_note`. A board as `last_open` constrains nothing, and neither does an empty one. Deleting the open board empties `last_open` and leaves `last_open_note` on the earlier Markdown note. The random walk found this: requiring `None` there called a file this build had just written stale.
- *Not in the plan: the Markdown fallback when reopening.* `Session::open` tries `selection`, then `last_open_note()` (any kind), then `markdown_last_open_note()` (the v1 key), and `resolve_selection` tries the same two in a group. With boards off, a workspace whose last open note was a board then reopens the Markdown note released Wren would reopen. Without the fallback, nothing would be selected. With boards off, the first save drops the boards from the lists and removes `[navigation]`, so the boards' places in Back/Forward are not kept for when the setting comes back on.
- *Not in the plan: `transform_paths` derives the v1 lists from the transformed `[navigation]`.* It does not transform them on their own. The v1 rule moves a pruned position to the first remaining visit, and that visit can be a board. Two separate transforms would then disagree, and the next load would call `[navigation]` stale.
- *Tests.*
  - Unit tests in `workspace_state.rs`, one or more per rule:
    - the projection table, including a position with no Markdown at or before it
    - a board as `last_open`
    - removal, which keeps unknown keys
    - a lenient read that never disables the file: wrong types, `../` and absolute entries, a 100,000-entry array
    - skipped entries re-point the position
    - the trust table, including an older build that only went back
    - rename and delete remap and prune `[navigation]`
    - a prune that would move the two positions apart
  - `tests/support/released_v0_1_1.rs`: the frozen validator, copied verbatim with `git show v0.1.1:…`.
  - `tests/released_state.rs`:
    - Back and Forward across kinds, with the exact v1 and `[navigation]` values.
    - Quitting with a board open. It reopens with boards on. With boards off, the Markdown note reopens, and opening writes nothing.
    - A board named in `selection` reopens.
    - Renaming the group of the open board, then deleting the board.
    - The pretend older build. It keeps a stale `[navigation]` and changes the v1 keys, and the v1 keys win.
    - The seeded 200-step random walk over mixed kinds (2,000 steps for three seeds, `#[ignore]`). Its steps:
      - open, Back, Forward
      - create either kind
      - rename and move notes
      - delete
      - rename groups
      - delete a file from outside, then reconcile
      - a dirty conflict copy; the test asserts the walk made one
      - reopen with boards on or off
    - After every step, the walk checks that:
      - the frozen validator accepts the file
      - no v1 key names a board
      - `[navigation]` exists only beside another kind
      - a fresh load believes `[navigation]`
      - when a step wrote the file, it wrote the lists the Session holds
      - after a reopen, the note named in `selection` is the open document
    - Byte identity. `tests/fixtures/main-state.toml` holds what `main` (`393ae14`) writes for a scripted run of 17 ops. To record it, the same script ran against an export of `main` (`git archive`) with its own target folder. The same script must give those bytes with boards off and a board on disk, and with boards on and no board on disk.
  - The Board run of the op-by-kind table, `op_by_kind_table_board`. All three runs of the table now check the file with the frozen validator after every op.

Review (§8.1), at `1248f85`.
- *Counts:* 0 blockers, 0 majors, 2 minors. The reviewer re-ran the gates (`make check`, the X11 tiny-skia suite on both engines with 241 passed and 1 skipped, the aarch64 clippy check) and re-recorded `main-state.toml` from `git archive 393ae14`, byte-identical.
- *Fixed:* `c1d772c`, the fixture's doc comments named a `record_main_bytes` that does not exist; they now give the recording recipe. A comment fix, so no regression test.
- *Not fixed:* the `CLAUDE.md` Layout map lacks `kind.rs` (and, already on `main`, `workspace_state.rs`). §7.1 item 5 forbids a Stage A chain to edit `CLAUDE.md`; it is carried to `boards-p3-docs` (D1), which owns `CLAUDE.md`.
#### boards-p1-model
M1 (model types and `parse`).
- *Surface added inside §3.2's meaning:* `parse` and `ParseError` (with `Display` and `Error`) at the crate root from a private `read.rs`; a `Vocabulary` trait over the closed-set enums (`ALL`, `name`, `from_name`), `Named::{read, as_str, known}`, `Color::read`, `Default` for `Board` (the empty schema-1 board), `NodeStyle` and `EdgeStyle`, `Edge::DEFAULT_HEADS`, `Label::DEFAULT_AT`.
- *Where the plan was silent:* an error's line and column are those of the key for a field (the column counts characters, a leading BOM excluded), of the item for an array item, of the id in its header for a record missing `at`, `size`, `from` or `to`, and line 1 for a missing `schema`. `size`, `anchor`, an endpoint's `at` and `heads` must have exactly two items, as `at` must: each is a known field whose type is a pair. In a free end, `side` and `anchor` are unknown content. A `requires` entry outside `[a-z0-9-]{1,32}` is not an error: it is a feature this build does not know (read-only, M3). `schema` must fit `i32` like every known integer. The `toml` crate's nesting cap is exactly 80: depth 80 loads and 81 fails.
- *Tests split across items:* test 2 compares models in M1 (`requires` compared sorted) and gains the byte comparison with `write` in M2; test 7's read-only cases load in M1 and get their `access` assertions in M3; test 9's "also checks and writes" joins in M2 and M3. Test 9's wrong-type half builds `toml::Table` values directly (33 known-field paths, every wrong type) and asserts the error's path is that field's.

M2 (`write`).
- The Rust writer reproduces every step 0 golden output and sample board byte for byte with no blessing; `WREN_BLESS=1` rewrites a `golden/format/` file (never a sample board, which only `tools/board_fixtures.py` writes).
- *The open point of step 0, settled:* a time read without seconds (TOML 1.1's `07:32`) is written with `:00`, because TOML 1.0 requires seconds; every other date and time is the crate's `Display`, including a fraction of `.0`, which the reference writer (microseconds, trailing zeros dropped) would write without. No fixture holds either case for the reference writer.
- *Where the plan was silent:* a key of unknown content that a typed field owns (`at` in a node's `extra`, `fill` in a node style's, `node` at the top level, `at` in an attached end's) is not written: such a model can only be built in code, the typed field wins, and the output always reads back. Defaults are compared by the written string, so `Named::Other("rect")` or `Color::Unknown("neutral")` built in code is omitted as the default and `Color::Unknown("#ABCDEF")` is written lower case, as a file would read them. A `NaN` of any sign or payload is written `nan`. `requires` is sorted, not deduplicated.
- *Tests:* test 3's model generator keeps CR out of labels, since a valid model's label breaks are LF (§4.9) and a CR would read back as LF; the string test checks that case separately. Test 4 takes "changes exactly one line" as one changed line when the field is present and one added line when it was absent at its default (recolour, restack, relabel of an empty label). Test 6's key-order check parses each line's dotted key itself and also runs over 50 random models. Test 5 is a new unit in `test_board_fixtures.py` over every `.out` file on disk and every canonical sample. Test 9's mutation run now writes whatever loads and checks the written text reads back and writes the same bytes.

M3 (`check` and `access`).
- *Surface added inside §3.2's meaning:* `Issue { element, kind }`, whose `element` is `scene::Source` (`Node(id)` or `Edge(id)`) and whose `IssueKind::name` gives the harness's names (`dangling_edge`, `missing_parent`, `parent_cycle`, `too_deep`, `shared_id`, `missing_kind`, `missing_z`, `invalid_z`); `Access::ReadOnly(Reason)` with `Reason::Schema(n)` or `Reason::Requires(unknown, sorted)` and a `Display` for the footer; `placements(&Board)`, each node's drawn parent and absolute corner, the parent rules of §4.12 in one place for `scene::build` (M5) to use; `MAX_DEPTH` (32) and `SCHEMA` (1).
- *Where the plan was silent:* a dangling connector is one issue naming which ends dangle, not one per end. A node and a connector sharing an id are two issues, one on each element, so each carries its marker. A node's level counts itself, a root being 1, so a chain of 32 is drawn nested and a chain of 33 is flagged (test 8's "a chain of 33"); "every node in it as a root" is taken as every node past level 32, each with its stored `at` as absolute, so the first 32 levels draw as written and nothing above the cut moves. A node hanging from a cycle without being in it is drawn inside its laid-out parent. `access` makes any schema other than 1 read-only, not only a higher one: a lower one cannot load, so the difference shows only for a model built in code. When the schema is newer and `requires` names unknown features too, the reason given is the schema.
- *Deviation:* a `z` over 64 characters is not flagged in Phase 1. §4.12 lists "a `z` outside §4.7's pattern" as flagged and also lists "a `z` over 64 characters" in the out-of-range row, listed from Phase 3; the more specific row wins, so M3 flags a missing `z`, an empty one and one with a character outside `[0-9A-Za-z]`, and the length joins the Phase 3 limits.
- *Tests:* each flagged case is a canonical file, so every case also asserts `write` returns its bytes. Beyond test 8: a property test over random parent graphs (1,000 boards [50,000]) checks that a drawn parent is always the stored one, a walk up drawn parents ends within 32 levels, each corner is its parent's plus its own, and a node is a root exactly when it has no stored parent or is flagged; a chain and a cycle of 20,000 resolve without recursion. The mutation run and the random-model round trip now also check, and every committed board checks with only its known issues (`fields.out` shares the id `full` between a node and an edge; `Shapes` has its dangling connector).

M4 (`shape.rs` and `route.rs`).
- *Surface added inside §3.2's meaning:* two public modules, so the scene (M5), the canvas's hit testing and handles (Phase 2) and `board.bounds` share one geometry. `shape::{outline, detail, label_box, side_point, anchor_point, drawn}` and the shape constants; `route::{route, nominal_exit, drawn, has_arrow, Terminal, Attach, Routed, Drawn}` with `Routed::{length, point_at, draw}`, `sides` (what `board.select.connector_<dir>` reads) and the stroke, dash, arrowhead and elbow constants. Also `Side::{opposite, is_horizontal}` and `scene::{Point::new, Rect::new, Rect::right, Rect::bottom, Rect::center}`. The final values are in §3.7, rows marked "final, M4".
- *Where the plan was silent:* "wholly below" includes touching (`to.top >= from.bottom`), and a tie by centre goes right, so two coincident boxes give right then left and two coincident points give down. Rule 4's "should elsewhere" is taken as always: a floating end, on either route, is drawn on its nominal side, so the rule holds for every pair; a straight route joins the two side middles rather than clipping a centre line at the outlines. A free end routes as an empty box at its point and leaves along the nominal side. An `anchor` end is the anchor point itself (a point of the box, §4.4, not projected onto the outline) and leaves by the nearest side of the box. "15 % each side" of an ellipse is 15 % of the width left and right and of the height top and bottom. The cylinder's "cap height" is the whole cap (twice the half-height) and the document's "wave height" is twice the amplitude, so no label starts over the cap's rim or ends in the wave. The label position `at` is per-mille of the whole route's length, heads not deducted. Ends that meet (touching sides, coincident points) give a route of no length, and its arrowhead points into the side it enters by. The dangling-end stub (§3.7) is left to M5, which has `nominal_exit`.
- *Deviation:* the pill's radius is `min(w, h)/2`, not `h/2`, and the rounded radius is at most half of either side, so a tall pill or a small rounded box keeps a closed outline inside its box. The diamond's label box (`w/6`, `h/6`) is kept as the plan gives it, although its corners lie just outside the diamond; V2's label-fit test is where it changes if it must.
- *Tests:* the routing table runs 20 layouts (apart in eight directions, touching on a side and at a corner, overlapping, contained, coincident, zero-size apart, inside and on an edge, both zero-size, and boxes 16,000,000 apart) by the eight shapes by nine attachments at each end (floating, the four pinned sides, four anchors) by both routes, plus free ends: 25,920 node pairs, each with finite points, axis-aligned elbow segments, a pinned side kept, and each side or floating end at the side's point on the outline. The sixteen pinned pairs of one layout are checked point for point; away from degenerate layouts every elbow leaves and enters through its sides. 2,000 seeded pairs that overlap on one axis and are apart on the other, every shape, both routes: both floating ends sit on the nominal sides, the elbow leaves and enters through them with at most two turns, and reversing the pair swaps the sides. Shapes: every outline closed, finite and inside its box and touching its four sides; exact rect, diamond and parallelogram paths; the parameters at their clamps; the label box of each shape; label boxes that never turn inside out or leave the node; side points on every outline; anchors and their clamping. A throwaway SVG of the eight shapes and the sixteen pinned routes was rendered and looked at; it is not committed.

M5 (`style::resolve` and `scene::build`).
- *Surface added inside §3.2's meaning:* `style::{resolve, Resolved, contrast, TINT_DARK, TINT_LIGHT}`, where `Resolved` is `{ stroke, fill: Option, label }`; `scene::build`, `scene::line_height` and the scene constants (`LABEL_SIZE`, `LABEL_LINE_HEIGHT`, `SMALL_SIZE`, `SMALL_LINE_HEIGHT`, `EDGE_LABEL_PAD`, `CAPTION_INSET`, `STUB_LENGTH`, `STUB_CROSS`, `MARKER_SIZE`, `COORDINATE_LIMIT`, `MAX_LABEL_BYTES`, `MAX_LABELS`, `MAX_Z`); `route::leave`, the point and side an end leaves by, for the dangling stub. No shared scene type changed shape. The final values are in §3.7, rows marked "final, M5".
- *Deviation:* the scene lays lines out with §3.7's fixed line heights (`scene::line_height`: 20 px at 14 px, 16 px at 12 px), not with `Measure::line_height`, because `svg::write` (§3.2) takes no measurer and must place each line's baseline in the same line box; `FixedMeasure::line_height` (1.25 times the size) is therefore unused by `build`. The canvas's measurer (V2) should return `scene::line_height`, as the spike's already does.
- *Where the plan was silent:* a connector label is not wrapped (it has no box), only broken at line feeds. A node with no `kind` is the placeholder without a caption; a connector of unknown or no kind draws as a connector. A text node's colour is its hue at `none`, the stroke colour. The dangling stub leaves a `from` end on the right and a `to` end on the left when the end floats (the nominal rule's tie, since there is no other node), and the cross stroke is square to the stub. Markers go after every element, one per flagged element, centred on the top-left corner of the node's box or of the connector's drawn line or stub; a connector with both ends dangling has no geometry and no marker. A clamped box keeps its far edge within 2^24 as well (corner to 2^24 − 1, size to what fits), so no corner rounds in `f32`. The arrowheads of a connector are one filled path, its line another, and each label a backing rectangle then its text: four kinds of item, all with the connector as their source. A path's `bounds` include half the stroke width; a node outline's are its box grown by that, since every outline lies inside its box (M4).
- *Tests:* nine scene dumps in `golden/scene/` (the five sample boards, `password-reset`, `shapes-and-ends`, `fields`, `unknown`) under `FixedMeasure` and a test palette, one primitive per line, blessed with `WREN_BLESS=1` and read back as a diff; the parent-transform table (nested, through an unknown kind, a missing parent, a self-parent, cycles of two and three, a node hanging from a cycle) and a chain of 33; the stacking table (`z` bytewise with digits before upper before lower case, missing and empty keys, 64-byte truncation, id ties, a node before a connector of the same id, children directly above their parent); the wrap and overflow table for every shape; the resolve table (every token at every fill, literals, unknown strings, both palettes); placeholders, free text, stubs, markers, connector labels and heads, the label caps, clamping, and a palette change moving no geometry. A property test draws 300 random boards [20,000]: finite, deterministic, every item inside the scene's bounds, every node with a box drawn. The loader's mutation run now also builds the scene of whatever loads.

M6 (`svg::write`).
- *Surface added inside §3.2's meaning:* `svg::{FONT_FAMILY, ASCENT, DESCENT}` and `Default` for `Options` (no background, no padding).
- *Where the plan was silent:* the view box is the scene's bounds grown by the padding on every side, and `width` and `height` equal its size in user units. A negative or non-finite padding counts as 0. A `<text>`'s `y` is its baseline: the middle of the line box (`scene::line_height`) plus `(ascent − descent) / 2` of the size, with Inter's `hhea` metrics (1984 and 494 of 2048), which is where cosmic-text centres a line's glyphs (`vendor/cosmic-text/src/buffer.rs:136`); each `<text>` has `xml:space="preserve"` so leading and doubled spaces survive. Text escapes `& < > " '`, keeps tabs, and drops U+0000 to U+001F otherwise (a carriage return included, which a parser would turn into a line feed), U+FFFE and U+FFFF; a line left empty is still one `<text>`. Numbers are the shortest decimal of the `f32`, never in exponent form, with `-0` and any non-finite value written `0`. A colour with alpha below 255 gains `fill-opacity` or `stroke-opacity`; the scene emits none today. Paths keep SVG's default miter joins, as the canvas's do.
- *Tests:* `golden/svg/` holds the SVG of each golden scene (ten, `escapes` added to both folders), the Checkout flow with a background and 40 px of padding as the app exports it; each is checked to be well formed by a small checker in the test, with one `<path>` per path item and one `<text>` per wrapped line reading back as the scene's line minus the stripped characters. A table of markup labels (`a < b & "c"`, `</text><text>`, `]]>`, a CDATA opener, an entity, an apostrophe, a processing instruction and a comment) reads back verbatim; U+0000 to U+001F, U+FFFE and U+FFFF are stripped. 300 random boards [20,000] write well-formed documents. A Python unit, `tools/wrentest_unit/test_board_svg.py`, parses every golden SVG with `xml.etree` and checks its runs against the scene dump. The ten files were rendered with resvg 0.45.1 and Inter in a throwaway program outside the repository and looked at: shapes, labels, arrowheads, label backings, the placeholder, the stub and the marker draw where the scene puts them, and the Overlap sample occludes its labels in order.

M7 (`search_text`, the release targets, crate docs).
- *`search_text`:* node labels in reading order by absolute corner (`placements`, so through parents), top then left, ties by id; then connector labels, connectors in id order and each one's labels in order; one label per line, a label's own line breaks kept. Where the plan was silent: a label of only white space is left out, kind names (a placeholder's caption) are not searchable, and labels are indexed whole, not cut at the 16 KiB the scene draws.
- *`make fuzz-board`* runs six ignored tests at full count in release (7.2 s): the four of M1 to M3 plus `fuzz_random_boards_draw` and `fuzz_random_boards_write_well_formed_documents` (20,000 boards each). *`make perf-board`* runs `perf_parse_write_and_scene` over grid boards of labelled shapes of all eight kinds, each linked to its right-hand neighbour by a labelled connector, median of nine runs, release, on the Linux development machine: 1,975 elements (1,000 shapes) parse 7.6 ms, write 0.5 ms, scene 2.9 ms, SVG 4.7 ms; 10,072 elements (5,100 shapes, 1.2 MiB) parse 63 ms, write 4.1 ms, scene 24 ms, SVG 27 ms, search text 1.0 ms. §6.7's `wren-board` budgets (10,000 elements: parse 75 ms, write 10 ms; 1,000 labelled shapes: scene 5 ms) are met and asserted by the test; the scene figure is under `FixedMeasure`, so the real measurer's cost is the canvas's to measure (V2).
- *Crate docs:* the crate's top-level documentation describes the pipeline (`parse`, `check`, `access`, `placements`, `write`, `scene::build`, `svg::write`, `search_text`), where the contract lives, the three golden folders, blessing, and the two release targets.

Fix step (after the chain's review).
- *Deviations:* the nesting rule now in §4.12 uses the crate's exact limit (80 dotted parts, 80 open arrays and inline tables), not a margin below it, so every file that loads writes back and the next level is refused; a test walks ten layouts (header layouts at the top and in a node, a table header, inline tables, arrays, tables in arrays, a dotted key, style, an endpoint, a label item) through every depth to 82. The crate's one error without a span (a key or header of more than 80 parts) is located by a small scan for dotted keys that skips strings and comments; if it ever finds none the error says the parser named no line and keeps line 1. Wrapping takes the longest run of words that fits by doubling and bisecting, checked against the word-by-word rule over 2,000 random labels (300,000 once in release); the one difference is that a broken word's trailing spaces no longer leave a blank last line. Plan text: the nesting rule in §3.8 and §4.12; the per-label line cap recorded for Phase 3 in §3.8, §4.12's out-of-range row and §4.16 test 10.
- *Review:* 0 blockers, 1 major, 4 minors (all reproduced from the reviewer's evidence).
- *Fix commits:* `0f4ef08` (major: content too deep to write back is refused at its field; minor: the spanless deep-key error names its line; regressions `content_too_deep_to_write_back_is_an_error`, `whatever_nests_and_loads_writes_text_that_loads`, `a_key_with_too_many_parts_is_found_by_its_line`), `2d4034a` (minor: wrapping a line of many words, 6.9 s to about 0.1 s for 50 labels of 8,000 words in 28,000-px boxes; regressions `a_line_of_many_words_takes_few_measurements`, `wrapping_matches_word_by_word_greedy_wrapping` and a 250 ms assertion in `perf_parse_write_and_scene`).
- *Not fixed:* (minor) elbow connectors in `Checkout flow` that run on top of other connectors (`8va037pc` along y = 510 and x = 60; `bm1mpp57` sharing x = 60 and its arrowhead with `e14xq0dp`), because the Z through the middle of a gap of an even number of grid rows falls on a row's centre line: changing it changes §3.7's final routing row and every scene golden, so it is the owner's or the plan's choice, to be made before Phase 2's acceptance layout and scene goldens (options: offset the Z's middle by half the margin when it meets another connector's or a node's centre line; pin sides on the two back edges in the fixture; route a turn-back edge outside the column). (minor) the scene's size grows with label length over box width rather than file size: a Phase 3 limit for `boards-p3-harden`, recorded in §3.8, §4.12 and §4.16 test 10.
#### boards-p1-canvas

**The spike (V1), run 1a. Result: go, with the named fallbacks of S3 and S5.** Geometry is rebuilt at every pan and zoom step and never moved by a renderer transformation, and the software renderer hides labels under 4 screen px and draws none while the view moves. S8: the label editor is owned and drawn by the canvas (candidate (a)). Nothing here waits for the owner.

*What was built.* `crates/wren-canvas`: `layers.rs` (the layer split), `paint.rs` (scene items into a geometry frame), `widget.rs` (the bare pan-and-zoom widget) and `spike.rs` (the generator, the state the host keeps, the label prototype). `wren --board-spike N`, the `perf.board` request, and `tools/board_spike.py`, which reruns every check below against a real window. The numbers are from a 1280 by 800 window under Xvfb on the Linux build machine, lavapipe for wgpu with `ICED_PRESENT_MODE=immediate`, scale factor 2 through `WINIT_X11_SCALE_FACTOR=2`. The machine was shared throughout (load 2 to 24), so a p95 is an upper bound, and lavapipe, which spreads over every core, moves most. Raw results and screenshots are under `target/board-spike/` in the chain's worktree (`checks/<renderer>-s<scale>/*.json` and `*.png`, `perf/<build>-<renderer>-<shapes>-s<scale>-<labels>/perf.json`, `perf-quiet/` for the second lavapipe pass); they are not committed, so copy them out before that `target/` is deleted. The crops named below are committed in `discovery/boards-spike/`.

| # | Result | Evidence |
|---|---|---|
| S1 | Go. | Step 0's check stands (132 scenes byte-identical across the `canvas` commit). With this chain's code the X11 tiny-skia suite on both engines still passes (see the gates below). |
| S2 | Go, with a different split rule (deviation 1). | The generator's 1,000 shapes hold 100 shapes that cover a neighbour's label and paint in 2 layers. In the region where the covering shape lies over the covered label's lines, 0 pixels differ from the shape's fill, at zoom 1 and 2, scale 1 and 2, on both renderers and both strategies; forced into one layer, the same region has 77, 248 or 897 show-through pixels (by zoom times scale), so the check sees what it tests. Crops: `s2-tiny-skia-split.png`, `s2-tiny-skia-one-layer.png`, `s2-lavapipe-split.png`, `s2-lavapipe-one-layer.png`. A layer costs tiny-skia nothing (8.0 to 9.0 ms a frame from 1 to 8 forced layers). On lavapipe at scale 1 a pan frame at 100 % goes 7.0, 8.4, 7.9, 9.7, 11.9 ms for 1, 2, 3, 4 and 8 layers with strategy (b), about 0.7 ms a layer, and 9.9 to 23.9 ms with (a), about 2 ms a layer. At scale 2 it is about 9 ms a layer with (b) (32 to 94 ms) and 15 ms with (a) (23 to 127 ms), taken at load 19. The cap stays at 4. |
| S3 | Go with the fallback: rebuild at every step, with no transformation at any time. | Placement is right either way. For three labels (two plain, one with CJK and an emoji) at zoom 0.25, 0.5, 1, 1.5, 2 and 4, each line's ink leaves its box (`origin`, `width`, the line height) by at most 3.2 board units at zoom 0.25 (0.8 device px), 0.96 at 0.5, 0.07 at 1 and 0 from 1.5 up, against a limit of one unit plus one device pixel; the numbers are the same for (a) and (b), on both renderers, at scale 1 and 2. Both are also sharp at rest: at zoom 2 the label drawn at 14 px under a transformation and the label laid out at 28 px differ in 4 of 40,960 pixels on tiny-skia (largest channel difference 14) and in 28 on lavapipe; every one of them is within 3 units of the box edge, on the outline, and none is in the label (`s3-tiny-skia-zoom2-transform-beside-rebuild.png`, `s3-lavapipe-zoom2-transform-beside-rebuild.png`; (a) on the left). So sharpness does not choose; cost does, and (a) loses on both renderers (S5 and deviation 2). |
| S4 | Go, no margin. | Each of the 24 corpus labels (25 lines) was drawn between two bars, so the ink shows the advance. Drawn minus measured is between -0.66 and +0.25 px at scale 1 on both renderers and between +0.01 and +0.47 px at scale 2. The measurer was a stand-in (one iced `Paragraph` per string, kept per string); V2 builds the real one. |
| S5 | Go with the fallback on tiny-skia; go on wgpu. | Table below. With all labels drawn, strategy (b) at 1,000 shapes on tiny-skia takes 116 ms a pan frame with the whole board in view and 54 ms at 30 %, because every glyph is a separate blit (about 3.3 µs each up to 14 px, 12 µs at 28 px); the shapes themselves cost 7 to 8 ms. Hiding labels under 4 px fixes only the fitted view. With no labels while the view moves as well, 1,000 shapes run at 2.4 to 7.5 ms median (scale 1) and 8.8 to 14.8 ms (scale 2), under the 33 ms budget, and 200 shapes at p95 2.7 to 4.0 ms (scale 1) and 8.1 to 11.5 ms (scale 2), under the 16 ms one. Idle: `view` 0.00 to 0.03 ms median over 30 views, zero geometry builds, `draw` 0.00 ms on a cache hit. On lavapipe a `draw` that rebuilds the whole fitted 1,000-shape board is 2.3 to 4.6 ms median (tessellation), 7.8 ms at most in the quieter pass and up to 22 ms under load, and labels cost 3 to 14 ms of a frame, so wgpu keeps them. |
| S6 | Go. | With the board mounted under `note-editor`, `focus.get` names it at launch; four real arrow keys (xdotool) are captured and counted; `alt+k` opens the Commands palette, which takes the focus, and an arrow pressed then is not counted; Escape closes the palette and `focus.get` names `note-editor` again; Escape on the idle board leaves it focused. Same on both renderers. |
| S7 | Recorded. | X11 (Xvfb, xdotool through XTest): `Lines` of ±1. Buttons 4 and 5 are y +1 and -1, buttons 6 and 7 are x +1 and -1, and each click arrives twice, because both the scroll valuator and the core button reach winit under XTest; a native test must expect two lines a click. Shift with the wheel arrives as a vertical delta with Shift held, so the widget turns it sideways. Wayland, from winit 0.30.13's source and unconfirmed: a wheel gives `Lines` (minus the discrete value), a touchpad `Pixels` in physical pixels. macOS, unconfirmed: precise deltas give `Pixels` in physical pixels, anything else fractional `Lines`. Constants: 40 logical px a line, zoom 1.1 a line, `Pixels` divided by the scale factor, a 150 ms burst latch (a Ctrl pressed after the first click of a run of three did not turn the pan into a zoom). iced gives a widget no scale factor, so the host passes it (`window::scale_factor`, `Rescaled`). |
| S8 | Candidate (a): an editor owned and drawn by the canvas. | The prototype keeps the label's text, caret and preedit in the state the host owns, and the widget routes keys and `InputMethod` events to it by a pure function. Real keys (xdotool) opened the label with Enter and typed ` ok`, Backspace, Enter and `x` into it. A preedit of `にほんご` and a commit of `日本語`, set at the host's seam, left the text alone while composing and inserted the commit at the caret; the same events built as iced `Event::InputMethod` values pass through the routing function in unit tests. At zoom 0.5 and 2 the rectangle the widget hands to `request_input_method` equals the caret's board position through the viewport within 0.5 px, with a preedit text size of 7 and 28 px, and iced draws the preedit there itself (`s8-preedit-zoom0.5.png`, `s8-preedit-zoom2.png`). Escape during the composition is swallowed: the label stays open, the preedit stays, `focus.get` still names `note-editor`. Escape without one commits and leaves `note-editor` focused. Not shown: a real input method, which neither Xvfb nor headless Weston has (Mac console check 4). Candidate (b) was not built: a child editor has its own focus state and asks for the input method with its own unscaled metrics, so both would have to be proxied through the canvas, and (a) needed neither. |

**The strategy for V3.** One frame per layer in widget-local pixels, holding only the items whose bounds meet the viewport, each point already through the viewport and each stroke width, dash and font size already times the zoom. One `geometry::Cache` per layer, cleared when the viewport, the widget's size or the label rule's outcome changes, so an idle board builds nothing. Each wrapped line is its own `fill_text`: unbounded width, the scene's line height as an absolute height, advanced shaping, top-left at `origin`. Layers come from `layers::split`, capped at 4. The label rule is by renderer, which `renderer::Headless::name` tells a widget in both feature sets: `tiny-skia` hides labels under 4 screen px and draws none until the view has rested 150 ms; `wgpu` draws them always.

**Numbers per run.** Median / p95 of `frame_micros` in ms; pans are 60 steps of 8 px, zooms 30 steps of 1.05 about the middle of the window. "Fit" starts with the whole board in view (zoom 0.308 for 200 shapes, 0.139 for 1,000, 0.1 for 5,000); "30 %" and "100 %" start at those zooms. Paint (a) is one transformation over geometry cached in board units, (b) is geometry rebuilt at each step, "(a) then (b)" is (a) while moving and (b) 150 ms after. "Draw" is `draw_micros` during the pan at fit, and "items" how many scene items a step painted there (0 for a cache hit). 1,000 shapes are 5,870 scene items in 2 layers; 5,000 are 29,708.

| Build | Renderer | Scale | Shapes | Paint | Labels | Pan, fit | Pan, 30 % | Pan, 100 % | Zoom from fit | Zoom from 100 % | Draw, pan at fit (median / max) | Items a step at fit | Idle view | Idle builds |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| release | tiny-skia | 1 | 200 | (b) | all | 23.5 / 29.2 | 21.0 / 28.6 | 9.0 / 10.5 | 29.7 / 55.1 | 9.0 / 15.8 | 0.4 / 1.1 | 947 | 0.00 | 0 |
| release | tiny-skia | 1 | 200 | (b) | 4 px and up | 21.9 / 28.4 | 16.4 / 20.5 | 7.8 / 8.7 | 33.8 / 64.9 | 9.9 / 16.0 | 0.3 / 0.5 | 947 | 0.00 | 0 |
| release | tiny-skia | 1 | 200 | (b) | 4 px and up; moving: 300 lines at most | 3.1 / 22.3 | 16.6 / 19.6 | 7.8 / 9.1 | 19.4 / 43.1 | 10.8 / 18.7 | 0.2 / 0.7 | 818 | 0.01 | 0 |
| release | tiny-skia | 1 | 200 | (b) | 4 px and up; none while moving | 2.6 / 3.7 | 2.2 / 2.7 | 2.6 / 3.2 | 3.0 / 4.0 | 2.4 / 4.0 | 0.2 / 0.4 | 781 | 0.00 | 0 |
| release | tiny-skia | 1 | 200 | (a) | all | 23.6 / 32.7 | 21.8 / 27.9 | 26.8 / 32.1 | 33.8 / 46.7 | 33.8 / 62.9 | 0.0 / 0.0 | 0 | 0.00 | 0 |
| release | tiny-skia | 1 | 200 | (a) then (b) | all | 24.8 / 28.4 | 21.9 / 25.6 | 26.0 / 29.8 | 27.1 / 30.3 | 27.5 / 30.5 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | tiny-skia | 1 | 1000 | (b) | all | 115.7 / 218.8 | 54.2 / 64.4 | 8.2 / 8.6 | 108.8 / 223.9 | 7.7 / 14.4 | 1.6 / 3.7 | 4898 | 0.00 | 0 |
| release | tiny-skia | 1 | 1000 | (b) | 4 px and up | 8.2 / 9.1 | 52.6 / 62.1 | 8.4 / 10.1 | 35.5 / 116.1 | 8.9 / 14.3 | 1.1 / 2.1 | 4063 | 0.01 | 0 |
| release | tiny-skia | 1 | 1000 | (b) | 4 px and up; moving: 300 lines at most | 7.4 / 11.6 | 5.0 / 6.8 | 8.8 / 10.4 | 7.8 / 44.4 | 9.0 / 14.5 | 1.0 / 2.0 | 4063 | 0.00 | 0 |
| release | tiny-skia | 1 | 1000 | (b) | 4 px and up; none while moving | 7.5 / 11.6 | 5.1 / 5.6 | 4.0 / 4.5 | 5.7 / 10.8 | 2.4 / 3.0 | 1.1 / 2.1 | 4063 | 0.00 | 0 |
| release | tiny-skia | 1 | 1000 | (b) | the renderer's rule | 9.4 / 16.9 | 5.3 / 6.1 | 2.6 / 4.7 | 5.7 / 8.9 | 2.5 / 4.3 | 1.3 / 2.8 | 4063 | 0.00 | 0 |
| release | tiny-skia | 1 | 1000 | (a) | all | 103.7 / 116.6 | 119.7 / 135.9 | 115.8 / 122.9 | 119.6 / 128.6 | 118.3 / 126.8 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | tiny-skia | 1 | 1000 | (a) then (b) | all | 100.2 / 106.9 | 120.8 / 267.6 | 124.1 / 207.7 | 322.8 / 558.4 | 367.4 / 382.0 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | tiny-skia | 2 | 200 | (b) | all | 71.0 / 92.8 | 54.2 / 73.6 | 25.9 / 29.9 | 89.6 / 156.9 | 29.8 / 44.7 | 0.4 / 0.6 | 947 | 0.00 | 0 |
| release | tiny-skia | 2 | 200 | (b) | 4 px and up | 75.4 / 87.4 | 58.5 / 68.4 | 27.6 / 31.3 | 91.9 / 165.9 | 32.4 / 48.3 | 0.4 / 0.5 | 947 | 0.00 | 0 |
| release | tiny-skia | 2 | 200 | (b) | 4 px and up; moving: 300 lines at most | 7.2 / 64.3 | 51.0 / 64.2 | 26.5 / 30.1 | 58.1 / 136.6 | 31.9 / 50.6 | 0.3 / 0.4 | 818 | 0.00 | 0 |
| release | tiny-skia | 2 | 200 | (b) | 4 px and up; none while moving | 7.3 / 8.9 | 6.6 / 8.1 | 8.1 / 10.1 | 9.6 / 11.1 | 9.3 / 11.5 | 0.3 / 0.5 | 781 | 0.00 | 0 |
| release | tiny-skia | 2 | 200 | (a) | all | 63.0 / 69.7 | 59.5 / 66.1 | 80.4 / 93.5 | 84.3 / 92.9 | 89.2 / 98.3 | 0.0 / 0.0 | 0 | 0.00 | 0 |
| release | tiny-skia | 2 | 200 | (a) then (b) | all | 62.7 / 66.5 | 60.3 / 68.9 | 80.6 / 88.8 | 222.9 / 277.6 | 269.8 / 284.5 | 0.0 / 0.0 | 0 | 0.00 | 0 |
| release | tiny-skia | 2 | 1000 | (b) | all | 300.9 / 371.5 | 166.6 / 177.9 | 27.5 / 31.0 | 337.4 / 640.0 | 31.7 / 50.8 | 1.5 / 2.7 | 4898 | 0.00 | 0 |
| release | tiny-skia | 2 | 1000 | (b) | 4 px and up | 27.1 / 37.4 | 201.7 / 252.6 | 27.7 / 31.9 | 114.8 / 337.4 | 32.9 / 49.6 | 2.4 / 4.5 | 4063 | 0.01 | 0 |
| release | tiny-skia | 2 | 1000 | (b) | 4 px and up; moving: 300 lines at most | 17.8 / 20.0 | 15.4 / 17.6 | 32.2 / 36.9 | 17.5 / 142.2 | 34.7 / 56.4 | 1.6 / 2.4 | 4063 | 0.00 | 0 |
| release | tiny-skia | 2 | 1000 | (b) | 4 px and up; none while moving | 14.8 / 17.5 | 13.9 / 16.5 | 11.3 / 17.4 | 14.5 / 20.5 | 8.8 / 11.9 | 1.4 / 2.6 | 4063 | 0.00 | 0 |
| release | lavapipe | 1 | 200 | (b) | all | 34.4 / 45.5 | 29.9 / 39.4 | 26.1 / 36.3 | 44.6 / 65.2 | 36.0 / 56.0 | 1.7 / 3.6 | 947 | 0.01 | 0 |
| release | lavapipe | 1 | 200 | (b) | 4 px and up | 41.4 / 53.6 | 36.8 / 47.4 | 33.4 / 49.2 | 49.8 / 72.0 | 46.4 / 62.7 | 0.9 / 2.5 | 947 | 0.01 | 0 |
| release | lavapipe | 1 | 200 | (b) | 4 px and up; moving: 300 lines at most | 36.0 / 51.1 | 44.3 / 54.0 | 32.0 / 41.1 | 43.6 / 62.7 | 33.8 / 54.4 | 0.9 / 7.3 | 818 | 0.01 | 0 |
| release | lavapipe | 1 | 200 | (b) | 4 px and up; none while moving | 12.7 / 18.4 | 27.8 / 37.6 | 11.0 / 15.0 | 17.0 / 36.6 | 9.1 / 12.8 | 0.7 / 4.1 | 781 | 0.01 | 0 |
| release | lavapipe | 1 | 200 | (a) | all | 15.0 / 20.2 | 12.8 / 16.6 | 10.5 / 13.5 | 17.0 / 50.9 | 15.1 / 85.5 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | lavapipe | 1 | 200 | (a) then (b) | all | 37.5 / 48.7 | 37.4 / 47.3 | 34.7 / 48.6 | 42.0 / 66.8 | 40.3 / 99.8 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | lavapipe | 1 | 1000 | (b) | all | 59.2 / 110.0 | 47.3 / 67.9 | 23.4 / 38.5 | 97.1 / 167.0 | 18.0 / 21.9 | 3.4 / 19.5 | 4898 | 0.01 | 0 |
| release | lavapipe | 1 | 1000 | (b) | 4 px and up | 58.2 / 80.6 | 41.1 / 63.6 | 35.2 / 50.5 | 41.2 / 60.0 | 21.4 / 34.3 | 4.2 / 21.3 | 4063 | 0.01 | 0 |
| release | lavapipe | 1 | 1000 | (b) | 4 px and up; moving: 300 lines at most | 35.6 / 64.2 | 33.2 / 64.6 | 29.9 / 76.7 | 28.6 / 51.4 | 17.2 / 27.0 | 3.7 / 22.1 | 4063 | 0.01 | 0 |
| release | lavapipe | 1 | 1000 | (b) | 4 px and up; none while moving | 36.6 / 53.5 | 31.1 / 48.0 | 14.6 / 20.9 | 23.2 / 37.3 | 13.5 / 19.3 | 4.6 / 12.2 | 4063 | 0.01 | 0 |
| release | lavapipe | 1 | 1000 | (b) | the renderer's rule | 59.9 / 83.0 | 42.3 / 64.3 | 19.8 / 26.0 | 38.1 / 76.4 | 14.5 / 20.6 | 3.9 / 15.8 | 4898 | 0.01 | 0 |
| release | lavapipe | 1 | 1000 | (a) | all | 39.9 / 49.1 | 27.4 / 33.6 | 13.3 / 22.7 | 52.8 / 70.2 | 20.7 / 110.0 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | lavapipe | 1 | 1000 | (a) then (b) | all | 49.8 / 88.2 | 26.4 / 39.0 | 15.8 / 24.7 | 37.6 / 64.6 | 44.1 / 142.4 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | lavapipe | 2 | 200 | (b) | all | 39.4 / 47.8 | 37.9 / 45.8 | 35.0 / 42.5 | 42.5 / 50.3 | 37.2 / 46.7 | 0.8 / 3.6 | 947 | 0.01 | 0 |
| release | lavapipe | 2 | 200 | (b) | 4 px and up | 34.6 / 44.6 | 34.2 / 47.2 | 34.1 / 45.9 | 38.1 / 44.4 | 33.1 / 40.4 | 0.8 / 3.0 | 947 | 0.01 | 0 |
| release | lavapipe | 2 | 200 | (b) | 4 px and up; moving: 300 lines at most | 34.1 / 42.9 | 31.4 / 38.1 | 32.4 / 41.0 | 37.5 / 45.3 | 32.3 / 41.4 | 0.7 / 3.5 | 818 | 0.01 | 0 |
| release | lavapipe | 2 | 200 | (b) | 4 px and up; none while moving | 30.5 / 37.4 | 29.3 / 38.0 | 33.6 / 68.8 | 32.6 / 40.2 | 30.3 / 37.3 | 0.7 / 3.7 | 781 | 0.01 | 0 |
| release | lavapipe | 2 | 200 | (a) | all | 42.9 / 54.6 | 46.3 / 66.6 | 43.3 / 64.9 | 48.8 / 122.0 | 43.7 / 120.2 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | lavapipe | 2 | 200 | (a) then (b) | all | 39.3 / 47.1 | 34.7 / 43.0 | 35.2 / 42.0 | 38.4 / 66.1 | 36.2 / 99.8 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | lavapipe | 2 | 1000 | (b) | all | 49.0 / 57.3 | 43.7 / 54.7 | 33.3 / 38.9 | 48.2 / 69.8 | 31.6 / 39.6 | 2.9 / 13.1 | 4898 | 0.01 | 0 |
| release | lavapipe | 2 | 1000 | (b) | 4 px and up | 40.1 / 55.1 | 50.1 / 65.6 | 34.1 / 39.9 | 50.5 / 85.0 | 33.8 / 42.5 | 2.5 / 11.8 | 4063 | 0.01 | 0 |
| release | lavapipe | 2 | 1000 | (b) | 4 px and up; moving: 300 lines at most | 39.4 / 48.0 | 37.6 / 47.7 | 31.7 / 37.8 | 42.7 / 55.8 | 37.7 / 48.6 | 2.3 / 9.9 | 4063 | 0.01 | 0 |
| release | lavapipe | 2 | 1000 | (b) | 4 px and up; none while moving | 49.5 / 65.7 | 42.8 / 57.4 | 30.5 / 37.7 | 40.7 / 62.3 | 31.6 / 46.5 | 3.3 / 14.7 | 4063 | 0.01 | 0 |
| release | lavapipe | 2 | 1000 | (a) | all | 56.8 / 67.8 | 48.1 / 62.9 | 34.1 / 45.3 | 51.0 / 78.3 | 37.0 / 121.4 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | lavapipe | 2 | 1000 | (a) then (b) | all | 51.7 / 66.5 | 46.9 / 59.4 | 35.9 / 44.7 | 54.8 / 84.7 | 39.3 / 141.9 | 0.0 / 0.0 | 0 | 0.01 | 0 |
| release | tiny-skia | 1 | 5000 | (b) | all | 416.5 / 448.8 | 61.0 / 68.5 | 8.0 / 9.9 | 206.3 / 749.5 | 9.8 / 17.6 | 5.5 / 9.2 | 19173 | 0.01 | 0 |
| release | tiny-skia | 1 | 5000 | (b) | 4 px and up; none while moving | 26.8 / 31.8 | 5.5 / 6.4 | 3.3 / 4.9 | 12.4 / 32.1 | 4.2 / 6.3 | 5.1 / 8.5 | 15944 | 0.00 | 0 |
| release | lavapipe | 1 | 5000 | (b) | all | 74.1 / 94.2 | 22.3 / 31.3 | 9.1 / 13.9 | 47.5 / 118.1 | 12.7 / 17.8 | 11.5 / 15.1 | 19173 | 0.01 | 0 |
| release | lavapipe | 1 | 5000 | (b) | 4 px and up; none while moving | 55.4 / 69.7 | 14.5 / 22.0 | 8.2 / 11.8 | 19.7 / 45.3 | 6.9 / 10.4 | 11.0 / 34.7 | 15944 | 0.01 | 0 |
| debug | tiny-skia | 1 | 1000 | (b) | all | 196.1 / 234.1 | 106.8 / 125.8 | 19.4 / 22.2 | 274.8 / 618.5 | 28.3 / 41.1 | 2.6 / 4.2 | 4898 | 0.01 | 0 |
| debug | tiny-skia | 1 | 1000 | (b) | 4 px and up; none while moving | 25.2 / 30.3 | 19.8 / 22.6 | 7.1 / 8.3 | 22.4 / 30.6 | 6.2 / 8.2 | 2.0 / 3.3 | 4063 | 0.01 | 0 |
| debug | lavapipe | 1 | 1000 | (b) | all | 38.2 / 49.1 | 26.1 / 29.5 | 10.1 / 14.7 | 44.2 / 68.4 | 14.1 / 17.0 | 5.8 / 16.5 | 4898 | 0.03 | 0 |
| debug | lavapipe | 1 | 1000 | (b) | 4 px and up; none while moving | 24.7 / 28.8 | 19.7 / 33.5 | 8.4 / 12.5 | 17.6 / 31.1 | 8.1 / 12.6 | 5.3 / 7.5 | 4063 | 0.02 | 0 |

Not in the table: (a) at 1,000 shapes and scale 2 on tiny-skia did not finish a 60-step pan inside the harness's 30 s response deadline (over 0.5 s a frame). The lavapipe rows of the table were taken at load 6 to 24. A second pass of (a) and (b) with all labels at load 2 to 6 (`perf-quiet/`) gave these medians over the five columns: at 1,000 shapes and scale 1, (b) 35.7 / 23.8 / 9.5 / 33.0 / 10.4 ms (draw 3.2 median, 7.8 at most) and (a) 40.6 / 27.4 / 11.9 / 33.7 / 15.9 ms; at 200 shapes, (b) 12.7 / 11.1 / 9.6 / 17.9 / 15.6 ms and (a) 14.5 / 12.9 / 8.0 / 18.9 / 14.0 ms; at scale 2 the two are within 5 ms of each other (29 to 53 ms), the software fill of 2560 by 1600 pixels being most of the frame. So on lavapipe (b) is level with (a) or a little ahead in the median, and steadier: (a)'s p95 for a zoom from 100 % is 76 to 125 ms where (b)'s is 14 to 42, because each zoom step makes iced prepare every label of the cached board again. `open_micros`, from launch to the first redraw after the first `draw`: 39 to 53 ms on tiny-skia at 1,000 shapes (107 ms at 5,000, 74 ms in debug) and 300 to 550 ms on lavapipe, which is the adapter starting. `scene_micros` here is the generator with its cold measuring, not `scene::build`: 3.4 ms for 200 shapes, 12.5 to 13.9 ms for 1,000, 58 to 78 ms for 5,000, 27 to 29 ms for 1,000 in debug. 5,000 shapes: with the fallback tiny-skia pans the fitted board at 26.8 ms and a rebuild of all 15,944 visible items takes 5.1 ms; on lavapipe that rebuild takes 11 ms, over the 8 ms budget, which a wgpu board reaches at about 13,000 items in view at once.

**Deviations.**
1. *The split rule (S2).* Opening a new layer whenever a fill overlaps a label already in the current one gives one layer per covering shape in z order: 101 for the generator, so everything past the cap of 4 would show through. `layers::split` gives each item the lowest layer its earlier overlapping items allow, and a fill one above any earlier label it covers, so the count is the depth of the deepest stack of labels and fills (2 for the generator, 3 for three stacked shapes). Two approximations stop that depth spreading through a connected diagram: footprints are drawn in by 1 unit, so shapes and connectors that only meet do not chain, and a path that is only stroked is tested by its segments, not its bounds; a stroke crossing an earlier label stays in the label's layer and is painted under it.
2. *No hybrid (S3).* The plan expected (a) during a burst and (b) after it. (a) cannot cull: iced's tiny-skia frame gives geometry text an infinite clip, so every glyph of the cached board is blitted every frame (104 to 120 ms at 1,000 shapes, whatever is in view), and iced's wgpu prepares every label of the cache again whenever the transformation changes. (b) with culling builds 250 visible items in 0.1 ms (tiny-skia) to 0.5 ms (lavapipe) and the whole fitted 1,000-shape board in 1.6 and 3.4 ms.
3. *The label fallback is by renderer and from the first step of a burst (S5).* It applies only where `Headless::name` is `tiny-skia`. Two softer rules were measured and miss a budget: labels of 4 px and up always (52 ms at 30 %), and labels while moving when at most 300 lines are in view (5.0 to 9.0 ms median at 1,000 shapes and scale 1, but up to 35 ms at scale 2, and 200 shapes then run at p95 9 to 137 ms). A consequence to weigh in V4: one wheel click on tiny-skia blinks the labels for 150 ms. Keeping them for the first step of a burst needs the burst counted in redraws, not time, because a labelled frame slower than 150 ms otherwise ends the burst it belongs to; it was tried and left out.
4. *`perf.board`.* It has the four rings of §6.2. `frame_micros` leaves out the first interval of a run, which covers a frame drawn before the first step; `scene_micros` is the generator's time until `scene::build` exists; `open_micros` runs from launch, there being no `OpenNote` in the spike. The pan and zoom runs were also made from the fitted view and from 30 %, where nothing is culled: at 100 % a 1,000-shape board shows a twentieth of itself and every strategy looks fine.
5. *Additions outside the item's list.* A spike-only harness request, `spike.board`, sets the strategy, the label rule, the layer cap or a forced layer count, the viewport, the scene and the label prototype's edits, and reports what the widget did (geometry builds, raw wheel deltas, captured arrows, the input-method rectangle). `tools/wrentest/ui.py` passes `ICED_PRESENT_MODE` and `WINIT_X11_SCALE_FACTOR` through and knows `--board-spike`; `tools/run_display.py` takes the Xvfb screen from `WREN_XVFB_SCREEN`. `fonts::load` loads the bundled faces before the first window, since text is measured while the app is built. The crops in `discovery/boards-spike/` are committed so the evidence outlives the worktree.
6. *The spike asks for the input method*, for S8. V3's viewer must not (§7.2), and takes `spike.rs` and the routing of label keys out with it.

**For the items that follow.**
- *V2.* Cold measuring is about 6 µs a distinct line, 12.5 ms for 1,000 two-line labels, so the frame-build budget of 8 ms holds only with the cache the item names.
- *V3.* Key geometry by a revision no other view can share: a counter per view let a new scene reuse the old one's geometry when the host swapped the view under the widget. Resize, theme and scene changes all go through that one key and the cache key above.
- *V4.* `update` is handed the pointer as it is after a whole batch of events. A press and the move after it can arrive together (they did on lavapipe), and a pan anchored at `cursor` then starts from where the pointer ended and moves nothing; the widget takes the position from the last `CursorMoved` before the press, and asks `cursor` only whether an overlay has the pointer. Phase 2's 4 px threshold needs the same care.
- *I4.* The label's state lives in the host's view, so `board.type` and the harness reach it with no window. iced draws the preedit itself at the rectangle and text size the widget asks for, so the canvas draws the caret and nothing of the composition. While a preedit exists every key is swallowed before the modifier test. A captured Escape still reaches the app as `FieldEscape`, which refocuses `note-editor`; focusing a focused canvas changes nothing, which is what keeps the composition. An unfocus that arrives through a focus operation has no shell to publish with, so the commit on focus loss comes from the app's settle calls (§3.4 rule 3) or from the widget's next event. The prototype has no selection, no word wrap and no auto-grow, and re-splits layers on every keystroke.

**Gates.** Run at the head of the chain: `make check` (2,335 Rust tests in its two passes, 12 ignored; 288 Python units); `ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11`, 241 passed and 1 skipped, as at step 0; the spike's own checks (`layers`, `zoom`, `measure`, `keys`, `wheel`, `label`) on X11 with tiny-skia and lavapipe, and `layers`, `zoom` and `measure` on headless Weston with tiny-skia (`layers` with lavapipe too), all passing. There are no board test files yet, so the Wayland lane had nothing else to run. One `make check` run failed in `wren-core`'s `config::tests::registration_validation_is_nonmutating_and_independent_of_config_writes` (`Locked`) at load 13 to 19; this chain does not touch `wren-core`, and the test passed five times alone and in the two full runs after it.

**Waiting on others, none blocking.** Metal on the Mac and a desktop GPU run (`wren --board-spike 1000`; the orchestrator and the owner): they decide nothing above, but they are the first real numbers for wgpu and the place to confirm that wgpu should keep its labels while moving. A real input method in a label (Mac console check 4). The first commit of the chain was also checked alone, in a temporary worktree: `make check` passes there.

**The viewer (V2 and V3), run 1b.** Built on the spike's strategy and both of its fallbacks. V4 (input) and the confirmation step are not done.

*What was built.*
- `measure.rs`, `Measurer`: `scene::Measure` over cosmic-text, reached through `iced::advanced::graphics::text`, with one reused buffer and widths kept by face, size and text. `sweep`, called before each scene build, drops the widths that the build before the last did not ask for, and every width when fonts have been loaded since, so the cache follows the board. Line heights are the scene's own.
- `lib.rs`, `BoardView`: `open`, `reload`, `synced`, `set_palette`, `board`, `scene`, `issues`, `access`, `viewport`, `set_viewport` and `view(id, style)` as §3.2 has them, with `Viewport`, `Style`, `Labels` and, under `test-support`, `Probe` (scene builds and their times, geometry builds, items painted, layers painted, whether the last draw took the view to be moving, draw times).
- `paint.rs`: `Cull`, what one frame holds of a scene, and `shown`, the scene items of a frame by layer; both pure. `widget.rs`: the viewer, which keeps one geometry cache per layer for one `(scene revision, Cull)` and builds nothing while that pair stands.
- `spike.rs`, its widget and its label prototype are removed, and with them every `request_input_method`. `wren --board-spike N` stays: it now opens the text of a generated board file (`app/board_spike.rs`, `grid`: the spike's layout as schema-1 nodes and connectors) in a `BoardView`. `perf.board` reads the view's probe, so `scene_micros` is `scene::build` plus the layer split, without the parse. `spike.board` takes a `scene` (`{"grid": N}` or `{"text": "…"}`, any board file), `labels`, `cap`, `viewport` and `nodes`. `tools/board_spike.py` keeps `perf`, `layers`, `zoom` and `measure`, each now against the real scene and measurer.

*Deviations.*
1. *The label-fit test (§3.7) holds, and its second half is wrong in the plan.* All fifteen labels of the acceptance script fit the label boxes of their default-size shapes. But `Notify customer` is 106.1 units wide in Inter at 14 px, two more than the rectangle's label box (104), so it takes two lines in every shape, the script's rectangle included. Two lines are 40 units: they fit the diamond's box exactly (80 by 40) and do not fit the cylinder's (36 under its cap), which is the one shape it does not fit. The test asserts that table. `Payment ok?` (84.9) also takes two lines in its diamond and fills the box exactly. No constant was changed: the script uses no cylinder. Seen at 200 %: the diamond's label box reaches the outline at its corners, so a two-line label's first and last lines end close to the diamond's edges.
2. *The widget dates a move, not the view.* `set_viewport` has no clock in §3.2 and is called from several places, so the widget compares the viewport of each redraw with the one before (`widget::Motion`) and takes the first redraw that shows another viewport over the same scene as the move. Another scene (a reload, a theme change, a view swapped under the widget) is never a move, so a board first seen is drawn with its labels.
3. *`Style` and `Message`.* `Style` holds one thing, an optional label rule (`Labels`), which `None` leaves to the renderer's name; the board's colours are all in `Palette`. `Message` has no variant: the viewer takes focus and nothing else until V4 adds `Input`. `--board-spike` is therefore not pannable by hand until then; `perf.board` and `spike.board` move the viewport.
4. *Small additions to §3.2.* `BoardView::layers()` (the plan of `layers::split`), `palette()`, and `Measurer` as a public type. `reload` with the text the view already has does nothing. `set_viewport` clamps the zoom to 10 to 400 % and ignores a viewport that is not finite. The pristine pair is not kept yet: nothing reads it before `draft()`.
5. *Culling.* An item is painted when its bounds, grown by one screen pixel (and by a quarter of the font size for text, for ink past the line box), reach into the widget. A layer with nothing in view is not handed to the renderer.
6. *The `keys`, `wheel` and `label` checks left `tools/board_spike.py`* with the spike's input and label code. V4 brings input back through `Machine::step`, and should bring the S6 and S7 checks back with it.
7. *Labels under 4 px on tiny-skia* are not drawn at rest either, as the spike recorded. At 25 % a 14 px label is 3.5 px, so the Phase 1 scene `board-zoom-out` shows no label on tiny-skia and all of them on wgpu. The unstroked box behind a connector label is a path, so it is still drawn and leaves a gap in the line.

*Numbers.* Release, 1280 by 800 under Xvfb, the generated board, the renderer's own label rule, median / p95 of `frame_micros` in ms; the machine was shared (load 1 to 10).

| Renderer | Scale | Shapes | Pan, fit | Pan, 30 % | Pan, 100 % | Zoom from fit | Zoom from 100 % | Draw, pan at fit (median / max) | Idle view | Idle builds |
|---|---|---|---|---|---|---|---|---|---|---|
| tiny-skia | 1 | 1000 | 7.4 / 8.9 | 5.0 / 6.4 | 2.6 / 3.4 | 5.7 / 8.9 | 2.4 / 3.1 | 1.0 / 2.9 | 0.006 | 0 |
| tiny-skia | 1 | 200 | 2.9 / 3.7 | 2.0 / 3.4 | 2.7 / 4.0 | 3.5 / 4.4 | 3.1 / 4.3 | 0.2 / 0.7 | 0.007 | 0 |
| tiny-skia | 2 | 1000 | 13.5 / 15.5 | 11.0 / 12.7 | 8.1 / 9.2 | 14.3 / 18.0 | 8.9 / 11.1 | 1.7 / 2.8 | 0.008 | 0 |
| tiny-skia | 2 | 200 | 6.9 / 8.3 | 5.9 / 6.6 | 7.8 / 8.7 | 9.9 / 13.6 | 8.1 / 10.0 | 0.3 / 1.0 | 0.008 | 0 |
| lavapipe | 1 | 1000 | 32.9 / 40.7 | 19.9 / 28.0 | 7.8 / 13.4 | 31.2 / 53.2 | 9.4 / 19.2 | 3.9 / 4.8 | 0.010 | 0 |
| lavapipe | 1 | 200 | 13.0 / 17.5 | 10.9 / 16.2 | 8.5 / 12.6 | 16.8 / 29.3 | 11.8 / 20.3 | 0.8 / 1.2 | 0.010 | 0 |

These meet S5 as the spike left it: `draw_micros` under 8 ms on both renderers (7.3 ms is the largest, a full rebuild at 30 % on lavapipe), tiny-skia medians under 33 ms, 200 shapes at p95 under 16 ms, idle with no scene build and no geometry build over 30 views. No run built a scene. `scene_micros` at 1,000 shapes (5,870 items, 2 layers): 2.7 to 3.1 ms for `scene::build` with a warm measurer plus 0.65 ms for the layer split and 3 µs for the sweep, timed alone in a release test; 6 to 7 ms for the same rebuild inside the app at load 9; 10.5 to 12.3 ms for a new view, whose measurer is cold (1,002 distinct lines); 16 to 27 ms for the first view of a run, which also loads the fallback faces for CJK and emoji. `open_micros` from launch: 49 to 91 ms on tiny-skia, 321 to 331 ms on lavapipe (the adapter starting). The pixel checks pass on X11 with tiny-skia (scale 1 and 2) and lavapipe and on headless Weston with tiny-skia: 0 show-through pixels with the split against 77 and 248 forced into one layer, the same counts the spike's hand-built scene gave; ink within 3.2 board units of its line box at zoom 0.25 (0.8 device px), 0.96 at 0.5 and 0.07 from 100 % up, the spike's figures; drawn minus measured between -0.66 and +0.25 px over the corpus's 25 lines with the real measurer.

*For the items that follow.*
- *V4.* `fit` needs the widget's size, which a `BoardView` does not know; the widget can report it, or the host can pass it. `Pixels` wheel deltas need the window's scale factor, which the host knows and the widget does not; `Style` is where a per-view value from the host arrives. The spike's wheel and middle-button code, with the pointer lesson recorded above, is in `git show d6a2c99:crates/wren-canvas/src/widget.rs`. On tiny-skia one wheel click, one `zoom_step` or a `fit` leaves the labels out for 150 ms, because any viewport change is a move.
- *A3, A8.* Load the label font before `BoardView::open`: widths are measured there. On tiny-skia a screenshot taken within 150 ms of a viewport change has no labels; `probe.moving` says whether the last draw was such a frame, and it is only refreshed by a draw. `perf.board` and its rings live in `app/board_spike.rs` and read `board_spike`; A8 points them at the app's board.
- *The sample `Checkout flow`.* Its loop connectors run through other shapes (through `Backorder`, `Retry payment`, `Pack`, `Ship` and `Send receipt`), as the router has no obstacle avoidance; a stroke that crosses an earlier label is painted under the label (the spike's deviation 1). It is the scene's and the fixture's to change, not the widget's.

**Gates** at the head of V3: `make check` (2,675 Rust tests in its two passes, 28 ignored; 290 Python units); the full X11 tiny-skia suite on both engines, 241 passed and 1 skipped, as before; `cargo clippy --target aarch64-apple-darwin` on `wren-iced` with `smoke-test`, clean. There are no `test_board*.py` files yet, so the board lanes had nothing to run; `tools/board_spike.py` stood in on X11 (both renderers) and Wayland. Scenes of the five sample boards, with `Checkout flow` also at 200 % and 25 %, were captured in dark and light on tiny-skia and lavapipe through `spike.board` and looked at (`target/scenes/canvas/`, not committed).

**Input (V4) and the confirmation step, run 1b.** Both done. The confirmation step's answer is go: the export lays text out as the canvas measures it, so Phase 3's PNG export needs neither outlines nor `textLength` on that account.

*What was built.*
- `machine.rs`, pure and with no iced type: `Machine::step(input, &View) -> Vec<Effect>` with Idle and Panning. `Input` is `Wheel { delta, at, zoom, sideways, scale_factor }`, `Press`, `Move`, `Release`, `Space { held }` and `Cancel`; `Scroll` is `Lines` or `Pixels`; `Button` names left, middle, right and other. The constants are S7's: `LINE` 40, `ZOOM_PER_LINE` 1.1, `BURST` 150 ms, the burst latched on its first event and extended by each one.
- `lib.rs`: `Message::{Input(epoch, Input), Resized { width, height }}`, `Update`, `Effect`, `BoardView::{update, zoom_step, zoom_reset, fit, epoch, size, machine}`, `Style::scale_factor` (default 1), `FIT_MARGIN` 40, `ZOOM_STOPS` (10, 25, 50, 75, 100, 150, 200, 300, 400 %), `Viewport::{zoomed_to, fitting}`. `reload` now resets the machine and moves the epoch on (§3.4 rule 5).
- `widget.rs`: wheel (also unfocused, under the pointer, never through an overlay), presses, moves while a button it saw pressed is held, releases, Space, the arrows, window focus loss. The press position comes from the last `CursorMoved` (the spike's pointer lesson). It reports its size when it differs from the view's.
- The board spike opens fitted, routes `Message::BoardSpike` to `update`, and passes the window's scale factor, which `app.rs` now asks for when the window opens and follows on `Rescaled` (`App::scale_factor`). `spike.board` gains `fit`, `zoom_step`, `zoom_reset`, `edit` (a new label for one node, written canonically and reloaded) and reports `scale_factor`, `size`, `epoch`, `panning`, `space`, `arrows` and `wheel`; every board shown is timed afresh by `open_micros`. `tools/board_spike.py` has its `keys` and `wheel` checks back, with a Space drag and scale-aware distances, and gains `view` (the opening fit, `fit`, the stops) and `open`; `perf` times one edit.
- The confirmation test, `resvg_draws_each_exported_line_as_wide_as_the_canvas_measured_it` in `app/board_spike.rs`, with resvg `=0.45.1` (text only) as a dev-dependency of `wren-iced`; `Cargo.lock` gains the one dependency line and no package.

*Deviations.*
1. *The machine moves the view through an effect.* §3.2 gives `step` a `&View` and a `Vec<Effect>`, and lists only host effects. Phase 1's only effect is `Effect::Look(Viewport)`, which `BoardView::update` applies itself, so `Update::effects` is always empty and `Update::changed` always false in this phase (a look is not a change, §3.4 rule 4). Phase 2's contract commit keeps `Look` beside the host effects or replaces it with an equivalent; the machine's outputs are a superset of what reaches the host.
2. *The widget reports its size* through `Message::Resized`, an addition to §3.2's `Message`, because `fit` and the zoom stops (about the widget's middle) need it and `BoardView` cannot ask. A `fit` before the size is known waits for it, and the widget paints only the canvas colour meanwhile, so a board never shows a frame from the wrong place; `set_viewport` or a zoom replaces a waiting fit, so a viewport the host restores wins. Before any size, the stops zoom about the widget's corner.
3. *The scale factor* comes from the host in `Style::scale_factor` and travels in each `Input::Wheel`, so the rule that divides `Pixels` by it is the machine's and unit-tested. A scale factor that is not a positive number counts as 1.
4. *The epoch check is in `BoardView::update`*, not left to the app: input stamped with an older epoch is dropped there.
5. *A zoom step, a reset or a fit is drawn in full at once* (the V3 open issue). The view counts these jumps and the widget's `Motion` does not take a jump for a move, so on tiny-skia Alt+= shows labels in the first frame. Wheel steps, drags and `set_viewport` are still moves: one wheel click still leaves the labels out for 150 ms on tiny-skia, as the spike recorded.
6. *An empty board fits at 100 % with its origin in the middle of the widget*, where Phase 2's first Tab node lands (§5.6 rule 5). A widget smaller than its margins fits at 10 %.
7. *Keys.* The focused board takes unmodified Space and the four arrows (Shift allowed); a key with Ctrl, Alt or the logo key is the app's. Key repeat of Space is captured and sends nothing. A Space release is honoured when focus has gone elsewhere, and losing the window's focus, or a click off the board, sends `Cancel` (a drag in flight stops, Space counts as up). Letting Space go mid-drag does not end the drag; its release does. Mod is iced's `command()`, ⌘ or Ctrl, as §5's notation has it.
8. *The confirmation criterion as written fails for a reason that is not a mismatch.* Ink stops short of the advance by the first glyph's left bearing and the last one's right bearing: `End` is 23.0 units of ink and 25.26 measured, 2.26 apart. The test therefore reads resvg's advance of each exported `<text>` as the S4 check read the canvas's, the element's text between two bars less the two bars alone, and asserts that within 2 px. Over the 53 non-blank lines of the five samples resvg's advance minus the measured width is at most 0.42 px; raw ink minus measured is at most 2.36 px.
9. *`zoom_step` and a zoom off the stops.* A zoom within 1 part in 10,000 of a stop counts as that stop; otherwise a step goes to the next stop in its direction, and steps past either end stop there.

*Budgets (§6.7).* Release build, 1280 by 800 under Xvfb, the generated board (`grid`), the renderer's own label rule; the machine was shared (load 5 to 17), so medians are given with the load.

| Measure | tiny-skia | lavapipe | Budget | Result |
|---|---|---|---|---|
| `open_micros`, 1,002 elements (347 shapes), median of 3 | 16.5 and 23.2 ms (two runs) | 38.8 ms | 150 ms | confirmed |
| `open_micros`, 10,020 elements (3,380 shapes), median of 3 | 142 and 132 ms | 215 ms | 1 s | confirmed |
| `scene_micros` at open, 1,002 and 10,020 elements | 3.9 to 8.5 ms; 42 to 46 ms | | (part of the above) | |
| Scene + draw after one label edit, 1,000 shapes at 100 % (median, p95) | 8.2 / 12.4 and 8.3 / 10.1 ms; in the fix step 6.6 / 6.8, 5.6 / 6.9 and 6.4 / 10.0 ms | 7.9 / 8.4 ms; in the fix step 8.3 / 19.3, 7.7 / 8.6 and 7.8 / 9.1 ms | 8 ms | replaced: 10 ms (below) |
| The same, fitted (13.9 %) | 9.8 / 12.1 and 12.3 / 20.5 ms; in the fix step 8.1 / 8.3, 6.6 / 6.9 and 8.3 / 11.9 ms | 9.8 / 10.2 ms; in the fix step 10.1 / 11.6, 10.1 / 11.9 and 10.1 / 10.3 ms | 8 ms | replaced: 12 ms (below) |
| Pan at fit, 1,000 shapes, `frame_micros` median | 9.2 and 9.9 ms | 31.9 ms | 33 ms on tiny-skia | confirmed (unchanged from V3) |
| Idle, 30 views with 1,000 shapes open | `view` 1 µs median, 0 scene and 0 geometry builds | 10 µs, 0 and 0 | 1 ms, zero builds | confirmed |

`open_micros` here runs from the start of `show_board_spike` (font load, parse, `check`, scene, layers, the fit's round trip through the widget, the first painted draw) to the first redraw after that draw; reading the file is not in it, and A8 times from `OpenNote` as §6.7 says. With every label forced on (`labels: {min_px: 0, moving: true}`), a fitted 10,000-element board on tiny-skia takes about 380 ms a frame, which is why the spike's label rule exists; the `open` check runs with the renderer's own rule.

The edit budget of 8 ms is replaced, after the review: scene + draw after one edit of a 1,000-shape board is 10 ms at 100 % and 12 ms with the whole board in view, on both renderers; the debug ceiling stays 100 ms. The fix step measured it again, three `perf --shapes 1000` runs on each renderer with the release binary at load 4 to 11, and the medians above are 5.6 to 8.3 ms at 100 % and 6.6 to 10.1 ms fitted. The reason is that an edit rebuilds the whole scene and, in view, all of its geometry. Timed alone in a release test in the fix step, the generated board's 5,870 items take 2.6 to 2.9 ms in `scene::build` with the real measurer (0.5 ms of it the `check` the build runs for its markers) and 0.73 to 0.89 ms in the layer split; inside the app the same build is 4.9 to 6.3 ms on tiny-skia and 6.6 to 7.5 ms on lavapipe, whose driver threads share the CPU with it, and the timed span also drops the old scene. At 100 % the draw adds 0.4 to 0.7 ms; fitted, nothing is culled and it adds 1.6 to 1.9 ms on tiny-skia and 3.5 ms on lavapipe (tessellation). The earlier runs (load 5 to 17, above) were 1 to 4 ms slower. The cut that meets 8 ms is to rebuild only the edited element's scene items and their layers, which needs to know what an edit changed: Phase 1 reloads text from elsewhere and does not know, while Phase 2's `edit::Change` lists say exactly. Phase 2 measures the per-keystroke cost of §3.4 rule 7 on 1,000 nodes; if the scene's share of that keystroke misses, that cut is made there. §6.7's row takes these numbers at the phase end. The parse is outside the budget and was 15 ms of a 23 ms reload at load 9 for 2,975 elements.

The sample fixtures paint in at most 3 layers: `Checkout flow` 1, `Shapes` 2, `Overlap` 3, `Hand written` 1, `Future` 1, with no overflow (the test now includes `Hand written`).

*Checks in a real window.* `view`, `keys`, `wheel`, `layers`, `zoom` and `measure` pass on X11 with tiny-skia and with lavapipe; `view` and `wheel` also at scale factor 2 on tiny-skia; `view`, `layers`, `zoom` and `measure` on headless Weston with tiny-skia. S6 again: `focus.get` names `note-editor` at open, five arrows (one with Shift) are captured, Space is held and let go, `alt+k` opens the Commands palette, an arrow then is not counted, Escape returns the focus, Escape on the idle board keeps it. S7 again: buttons 4 to 7 pan by 40 px a line (two lines a click under XTest), Shift turns the wheel sideways, Ctrl+wheel zooms 1.1 a line about the pointer, a Ctrl pressed after the first click of a fast run does not turn the burst into a zoom, a middle drag and a Space drag pan by the pointer's motion, a left drag without Space does nothing. At scale factor 2 the app reports 2.0 and a drag of 60 physical pixels pans by 30 logical ones. The opening fit equals the reference computation within 0.01 and the widget's reported size equals its focus bounds. The measured-against-drawn difference is at most 0.66 px on every lane, as before.

*Scenes.* Captured with a scratch script through `spike.board`, dark and light, tiny-skia and lavapipe, in `target/scenes/canvas-v4/<renderer>/<theme>/` (not committed): the five samples as they open, `Checkout flow` right after a zoom step, after a reset, right after a move and at rest, and an empty board. Looked at: tiny-skia dark `open-checkout-flow` (fitted at 53 %, centred, labels drawn), `checkout-zoom-step-immediate` (75 %, labels in the first frame) and `checkout-moved-immediate` (no labels mid-move); tiny-skia light `open-shapes` (100 %, centred, every shape, the placeholder, the stub); lavapipe light `open-checkout-flow`; lavapipe dark `open-overlap` (100 %, centred, occlusion right).

*For the items that follow.*
- *A3.* Call `fit()` right after `open` for a board not opened before in this run, and `set_viewport` with the kept viewport otherwise; a viewport set before the widget's first frame replaces the waiting fit. Pass `Style { scale_factor: self.scale_factor, .. }`. Hand each `Message::Board` to `board.update(message, now)`; in Phase 1 the returned `Update` is always the default.
- *A4.* `step_text_size` on a board is `zoom_step(1)`, `zoom_step(-1)` and `zoom_reset()`.
- *A5.* For §5.15 rule 1, `Message::Input` with anything but `Input::Wheel` is board input; `Message::Resized` is not input and must not move the column.
- *A8.* `board.pointer` can send `Message::Input(view.epoch(), Input::…)` straight to `update`. `Perf::opened` starts a new `open_micros` for each board shown; point it at `OpenNote`.
- *Phase 2.* The widget sends a `Move` for every cursor move while a button it saw pressed is held, left drags included, and a `Press` for every button over the board; the 4 px threshold and hit tests can build on them.

**Gates** at the head of V4: `make check` (2,735 Rust tests in its two passes, 28 ignored; 290 Python units); the full X11 tiny-skia suite on both engines, 241 passed and 1 skipped; `cargo clippy --target aarch64-apple-darwin` on `wren-iced` with `smoke-test`, clean; the board checks above in place of the board test files, which are A8's.

**The fix step.**

*Deviations.*
1. *A drag whose release is lost.* The widget's own state is rebuilt when the tree around the board changes during a drag (the palette's `stack!` does it), so it forgot the press and never forwarded the release, and the machine stayed in Panning. The fix has two parts. The widget forwards a release it did not see pressed whenever the machine is panning, and does not capture it, since the release may belong to the overlay. The machine treats a press during a drag as the start of a new one, because the drag's release never came. A `Cancel` before the press, the review's other suggestion, was not used: it also lets Space go, so a Space still held would stop panning.
2. *The edit budget is replaced* (see *Budgets* above): 10 ms at 100 % and 12 ms fitted, with the numbers and the reason. The cut that would meet 8 ms waits for Phase 2's `edit::Change`.
3. *For the serial edit on `boards` at the phase end.* §8.4 leaves those edits to that step, so the review's two minors are listed here:
   - §3.2 gains `Message::Resized { width, height }`, `Effect::Look(Viewport)` (kept or replaced by Phase 2's contract commit), `Style { labels, scale_factor }`, `BoardView::{layers, palette, epoch, size, machine}`, the public `Measurer`, `Viewport::{fitting, zoomed_to}`, and `FIT_MARGIN`, `ZOOM_STOPS` and `REST`.
   - §3.7's last paragraph should read "fits every shape but the cylinder", with V2/V3 deviation 1's reason. The other option is §3.7's own rule: change the cylinder's label inset or the default size before the Phase 2 golden layout is written. The owner or the phase-end review chooses.
   - §6.7's frame-build row takes the replaced budget.

*Review.*
- Counts: 0 blockers, 2 majors (the stuck drag, the missed edit budget), 2 minors (§3.7's `Notify customer` claim, §3.2's surface).
- Fix commits:
  - `50c3a9e` `fix(canvas): end a drag whose release the widget lost` adds two regression tests, `machine::tests::a_press_after_a_lost_release_starts_over` and `widget::tests::a_drag_ends_when_the_widget_state_is_rebuilt_under_it`. The review's xdotool sequence (middle down, `alt+k`, middle up, Escape, then a plain left drag of 50 px) now leaves `panning` false and the viewport where it was; before the fix the viewport moved to (-250, -100).
  - `95fb591` `docs(discovery): replace the board edit budget with measured numbers`.
- Not fixed: the two minors. Both are edits to §3.2 and §3.7, which §8.4 leaves to the phase end; deviation 3 lists them. §3.7's choice is also the owner's or the phase-end review's.

*Gates* at the head of the fix step:
- `make check`: 2,739 Rust tests in its two passes, 28 ignored; 290 Python units.
- The full X11 tiny-skia suite on both engines: 241 passed and 1 skipped.
- `cargo clippy --target aarch64-apple-darwin` on `wren-iced` with `smoke-test`: clean.
- `tools/board_spike.py`: `view`, `keys`, `wheel`, `layers` and `zoom` on X11 tiny-skia, and `view`, `layers` and `zoom` on headless Weston tiny-skia, all passing.
- `perf --shapes 1000`: three release runs on each of tiny-skia and lavapipe, all passing their build counts.

*Verification* at `f4ffc7a`: both majors closed, no blocker or major open, the gates above re-run with the same counts. The review's xdotool sequence and its other lost-release cases (a reload or scene swap mid-drag, Space released in the palette) behave correctly. Re-measured edit cost, release, median scene + draw: tiny-skia fitted 6.6 to 8.5 ms and at 100 % 6.4 to 9.3 ms over five runs, with one run at 15.3 ms at 100 % under contention from another process (its 100 % scene build took 13.9 ms against 6.6 ms for the same fitted build); lavapipe fitted 9.6 to 11.0 ms and at 100 % 7.2 to 8.3 ms over three runs. The lavapipe fitted medians sit close to the new 12 ms, so the phase-end performance lens should measure them again.

#### boards-p1-app

**Items A1 to A4, session 1.** The view is not touched: until A5 mounts the canvas, a board opened with the setting on shows the empty editor, which accepts nothing.

*A1 (search and the palette).*
- `index::searchable` parses a board and returns `search_text`; a board that does not parse is searched as written, in the one allocation of its source. A board with no label has an empty searched text and an empty preview. A hit's `content_match` is a range in the searched text, the labels, not in the file.
- `theme::board_palette(&Colors)`: `canvas`, `ink`, `muted`, `raised` and `accent` are the editor's `editor`, `text`, `muted`, `raised` and `accent`; `issue` is `danger`. The hues are provisional, as the doc comment says: red is the theme's `danger`, and orange, yellow, green, cyan, purple and blue are that colour's HSL saturation and lightness at the hue angles 28, 48, 135, 187, 270 and 215 degrees. `dark` is read from the colours (the editor surface darker than its text), since the function takes no `Theme`. Their contrast is not tuned or tested beyond the test below; that is Phase 3's (ST3).
- *Where the plan was silent.* The C6 counter test now writes valid boards, so the 200 extractions it counts are real label extractions; it also checks one board's searched text.
- *Left for A5.* The thin adapter in `wren-iced/src/theme.rs` is not added: the app asks `App::board_palette()`, which calls the core function. `--board-spike` still paints with its own stand-in palette (`board_spike::palette`); A5 or A8 should point it at the real one.
- *Tests.* `index.rs`: a label word hits (in a node label and a connector label, any case), with the snippet and the highlighted range; `schema`, `kind`, `shape`, `node`, `size` and `labels` do not; the preview is the first label in reading order, not file order; the source is still what change detection snapshots, and the extracted text is counted in `retained_bytes`; the raw fallback (conflict markers) finds `schema` and shares one allocation, and labels take over when the text parses again. `kind.rs`: the template equals `wren_board::EMPTY`. `theme.rs`: in all eleven themes every token at every fill resolves, the seven hues are distinct from each other and from canvas and ink, a stroke is never the canvas colour, a tint is never the canvas colour, and a solid fill's label has at least 3:1 on it; the HSL conversion round-trips theme colours within one step.

*A2 (the setting at launch).*
- `Preferences::boards` is the setting as it was at launch and never changes in a run; `App::session_options()` builds `SessionOptions` from it, and `load_workspace` opens a new Session with them. A Session opened by `Op::OpenWorkspace` inherits them from the one before (C4).
- A `boards` value in the file other than the launch value shows `Restart Wren to apply \`boards\`.` as the banner. It is shown and retracted the way a settings warning is (a reload at the launch value removes it), and stands after any settings warning in the same banner, but `reload_settings` does not return it as a warning: a setting saved while the notice stands still reports `completed`.
- *Tests.* Off: the model lists only the Markdown note and has no warning for the board; a workspace opened from that Session keeps its options even when asked otherwise. On: the board is listed with its title and kind. `session_options` follows the launch value and not the file's later one. The notice appears for off to on and for on to off, joins a settings warning, and goes when the file is back at the launch value.

*A3 (the surface).*
- `App` has `board: Option<BoardView>`, `board_error: Option<ParseError>` (why the open board is raw text; A5's banner and `board.get` read its line and message) and `board_viewports`. `app/board.rs` holds `Surface`, `open` and `close`. `App::surface()` is Board when there is a view, Raw when there is an error, Text otherwise.
- `sync_editor` decides again when the open note's identity changed, when the note's kind and the surface disagree (a board id shown as Text, or the reverse), or when the Session's text is not what the surface last took (`board.synced()` on Board, the editor's text otherwise). A view of the same board takes new text through `reload`, so its viewport and widget state stay; a `reload` that fails puts the view away and the text in the editor. A raw board is tried again with `BoardView::open` only on those two triggers, never because of its own draft, whose text the editor already holds.
- *The viewport rule.* A view put away (another note, the raw fallback) leaves its viewport in the map under its note id, unless its first fit was still waiting for the widget's size: that board was never looked at, so it fits the next time too. `board::open` restores a remembered viewport and otherwise calls `fit()`. The map is cleared when a workspace opens. Nothing is stored on disk.
- A theme change calls `set_palette` from `apply_theme` (one scene build; none for the same theme). The find bar closes when a board opens.
- *Additions to `wren-canvas`* (§3.2 at the phase end): `BoardView::fit_pending()` is public, for the rule above, and `Probe::parses` counts the parses of a view under `test-support`. `wren-iced`'s dev-dependencies enable `wren-canvas/test-support`, as they do the editor's; `Cargo.lock` is unchanged by it.
- *Found while testing.* A file that is empty or holds only comments is a valid empty board (the format goldens `empty` and `comments-only`). So a Markdown file whose only line is a heading, saved under a board's name, opens as an empty board, not as raw text; Markdown with any prose line is a TOML error and opens raw. Nothing is written either way.
- *Tests* (`app/board.rs`, without a window): every transition among Text, Board, Raw and no note, including Back and Forward across kinds, with the invariant that the surface shows exactly the Session's document and the editor behind a board is empty; the text changing underneath (a valid board, conflict markers, another error, fixed again, Markdown bytes, removal); every Session-driven change of the open document reaching the surface (a draft from outside, a dirty conflict with its `.wren-board` copy, `WriteNote`, a clean reload, rename, flush, reconcile, create and delete), which holds only if each emits an event, since `sync_editor` runs on events; typing a valid board into a raw one never flips it, saves exactly the typed bytes, and it is a board at its next opening; 40 ticks, a flush and a reconcile cause no parse and no scene build, and a theme change one build; a board fits once and returns to its viewport after a reload, a note switch and the raw fallback; with the setting off no board surface exists; the five sample boards open as §7.2's exit criterion 5 says and opening writes nothing. `app/workspace.rs`: a workspace whose open note is a board loads onto the board surface, and with the setting off onto the Markdown note open before it.

*A4 (the single writer).*
- `ensure_editor_ready` fails on Board, which stops `App::edit` (every `Message::Editor`), the palette's editor commands and every harness `editor.*` request that acts. `text_changed` returns early on Board, so text that reached the hidden editor some other way is still never a draft. `apply_editor_engine` loads `""` on Board. `reveal_literal` is skipped on Board. `step_text_size` on Board is `zoom_step(1)`, `zoom_step(-1)` or `zoom_reset()` and returns before any setting is read or saved.
- `command_enabled`: the six Editor-scope commands need Text; Outline needs Text and Find needs Text or Raw. *Where the plan was silent:* `open_palette(Mode::Outline)` checks the same, because the harness and a palette message can open it without the command; `editor.toggle_markdown_assist` and `editor.toggle_engine` stay enabled on every surface, as settings.
- Markdown assist is off while Raw: `App::markdown_assist()` is the setting and not Raw, applied to the editor and its key policy by `apply_preferences` and, when the surface changes, by `sync_editor`. The palette row still shows the setting's value.
- `Request::detached` (test only) builds a harness request with no connection, so a unit test can send `editor.insert` through `handle_test`.
- *Tests.* `app/board.rs`: on a board, each Editor-scope command through `dispatch(Run(..))` is blocked; a paste (what `editor.insert` sends), Bold, Enter, Undo, Redo and Find as editor messages change nothing; text put into the hidden editor and followed by `text_changed` is not drafted; an engine switch, twice and from the settings file, leaves the editor empty; then 40 ticks taking the Session's clock to 1 s and to 5 s leave the Session text, the file's bytes and its mtime untouched, and the same clock does save a Markdown draft. The surface table of `command_enabled`, with Markdown assist off in Raw and back on a Markdown note. `font.increase`, `font.decrease` and `font.reset` step a board through 150, 200, 100, 75, 50, 25 and 100 % about the widget's middle, stop at 400 % and 10 %, and leave `editor-font-size`, the editor's size and the bytes of `config.conf` alone, while on a Markdown note and on raw text they size the text and save it. A search hit in a board opens it and selects nothing in the editor. `app/test_requests.rs` (smoke build): `editor.insert`, `editor.undo`, `editor.redo`, `editor.action`, `editor.command`, `editor.cursor` and `editor.find` are refused on a board and `command.run markdown.bold` is blocked. Removing the guard from `ensure_editor_ready` makes the first test fail.

*The `self.editor` sites.* 51 outside `test_requests.rs` at the head of A4: the 47 that `grep 'self\.editor\b'` finds on `main` (the plan says 46), the mount in `saved_editor_view`, which is written as `self` and `.editor` on two lines and so escapes that search, and three this chain adds in `apply_markdown_assist`. Classes: *guarded* (cannot act on Board), *safe* (reads the empty editor or changes only how it looks), *unreachable* (never runs with a board open).

| File and function | Site | Class on Board |
|---|---|---|
| `app.rs` `handle` (`FindChanged`, `FindNext`, `FindPrevious`) | `set_find_query`, `find_next` twice, `find_previous` | safe: find state of the empty editor; the bar is closed when a board opens and Find is disabled |
| `app.rs` `edit` | `perform`, `intent`, `undo`, `redo`, `break_group` | guarded: `ensure_editor_ready` fails first |
| `app.rs` `text_changed` | `text()` as the draft | guarded twice: only `edit` calls it, and it returns early on Board |
| `app.rs` `count_words` | `text()` | safe: read; no recount is ever due on Board |
| `app.rs` `contents` | `find_current`, `find_count`, the mount, `cursor`, `line_count` | unreachable: the `--editor` evaluation view, which has no workspace |
| `commands.rs` `command_enabled` | `markdown_assist()` | guarded: read only on Text |
| `commands.rs` `run_command` (`Find`) | `break_group` | guarded: Find is disabled on Board |
| `gestures.rs` `resize_event` | `break_group` | safe: undo grouping of an empty editor |
| `navigation.rs` `hydrate_navigation` | `set_find_query("")` | safe |
| `palette.rs` `open_palette` | `break_group`; `viewport()` for the outline | safe; guarded (the outline needs Text) |
| `palette.rs` `outline_headings`, `outline_rows` | `text()`, `cursor()` | unreachable: only the outline calls them |
| `palette.rs` `show_heading`, `return_to_origin`, `sync_outline` | `scroll_to_line`, `move_cursor`, `restore_viewport` twice | unreachable: they need an outline origin, which only an open outline sets, and it is dropped when the note changes |
| `palette.rs` `activate_note` | `reveal_literal` | guarded: skipped on Board |
| `preferences.rs` `apply_preferences`, `apply_shown`, `apply_markdown_assist` | `set_line_height`, `set_font_size`, `set_line_numbers`, `set_markdown_assist` twice, `set_key_policy` twice, `markdown_assist()`, `set_font` | safe: how the editor looks and which keys it routes |
| `preferences.rs` `apply_editor_engine` | `backend()`, `text()`, `set_backend` | guarded: Board loads `""`, never the Session's text |
| `preferences.rs` `step_text_size` | `set_font_size` | guarded: Board zooms and returns before it |
| `workspace.rs` `sync_editor` | `text()` to compare, `replace_text` | the decision itself: compared only when there is no view; a board empties the editor |
| `workspace.rs` `window_unfocused`, `show_dialog` | `break_group` | safe |
| `workspace_view.rs` `saved_editor_view`, `editor_footer` | `find_current`, `find_count`, the mount, `cursor`, `line_count` | safe today (they draw the empty editor); A5 mounts the canvas and the board footer instead |

In `test_requests.rs`: `editor.insert`, `editor.undo`, `editor.redo`, `editor.action`, `editor.command`, `editor.cursor` and `editor.find` ask `ensure_editor_ready` first and are refused on Board; `editor.set` is refused whenever a workspace is open; `editor.get` and the Editor branch of `shortcut` only read (the shortcut's intent then goes through `App::edit`, which is guarded); `editor.focus` focuses whatever is mounted under `note-editor`.

*For the items that follow.*
- *A5.* Mount `self.board` when `surface()` is Board; route `Message::Board` to `board.update(message, Instant::now())`. `board_error` holds the line and message for the Raw banner. The editor footer's counts read the empty editor on Board. Raw shows the board's TOML with the editor's Markdown highlighting; whether that should be plain is a view question. `find_open` is already false on Board.
- *A7.* `note.export_svg` is App scope and needs Board in `command_enabled`.
- *A8.* `WrenApp(settings="boards = on\n")` already launches with boards on; `WRENTEST_BOARDS` makes it the suite's switch. `board.get` can report `surface` from `App::surface()` and `error` from `board_error`. A screenshot within 150 ms of a wheel step has no labels on tiny-skia (the canvas chain's note).
- *The phase end.* With the setting on, opening a workspace now parses every board once for the index (M7 measured 7.6 ms for 1,975 elements in release); §6.7's "workspace open unchanged within noise" holds by construction with the setting off and is to be measured with it on.

*Gates* at the head of A4 (`9f055e8`).
- `make check`: 2,780 Rust tests in its two passes, 28 ignored; 290 Python units.
- `ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11`: 241 passed and 1 skipped, as at every chain before, so the setting off disturbs nothing the suite sees.
- `cargo clippy --target aarch64-apple-darwin` on `wren-iced` with `smoke-test`: clean.
- There is no `test_board*.py` yet (A8), so the board lanes had nothing to run. A scratch script over the test socket stood in, on X11 and on headless Weston with tiny-skia, launched with `boards = on` on a copy of the sample workspace, 12 checks passing on each: six boards listed by title; a board opens with the Session holding its text and the editor empty; `editor.insert`, `editor.command`, `editor.action` and `editor.undo` are refused and Bold, Find and Outline blocked; an engine switch leaves the editor empty; the text-size commands write no setting; after a theme switch, a blur, 1.2 s and a note switch the board's bytes and mtime are unchanged and the document is clean; searching `Payment` finds `Checkout flow` with a label snippet and `schema` finds only the conflict-marker board; that board opens as raw text and what is typed is saved exactly; fixing it on disk and reconciling makes it a board again; `boards = off` in the file shows the restart notice and changes nothing; after a restart with it off no board is listed, searched or open and the Markdown note reopens; `Checkout flow` and `Hand written` are byte- and mtime-identical after quit. The script is not committed: A8 writes these as `test_board_view.py` and `test_board_gate.py`.
- `tools/board_spike.py`: `view`, `keys`, `wheel`, `layers` and `zoom` on X11 tiny-skia, all passing, since `wren-canvas` gained two small accessors.
- Scenes: these items draw nothing new, so none was captured. Three screenshots of the scratch run on X11 tiny-skia were looked at: a board open (the notes list shows the seven notes with first-label previews such as `Start` and `Rectangle`, the editor is empty, the footer reads 0 words and Saved); the conflict board as raw text in the light theme (467 lines, the typed line last, Saved); the same workspace with the setting off (one note listed, section and group counts of 1, the Markdown note open).
- Not run, as the chain did not touch `wren-board` or `docs/`: `make fuzz-board`, `make perf-board`, the docs build.

**Items A5 to A7, session 2.** With the setting on, a board now shows in the board viewer, its row carries the kind icon, and it exports as SVG.

*A5 (the view).*
- `saved_editor_view` mounts `self.board`'s widget under `note-editor` when there is a view, and the editor otherwise; exactly one of the two is in the tree. `Message::Board(wren_canvas::Message)` reaches `App::board_input`, which hands the message to `BoardView::update` with `Instant::now()`. `App::board_style()` gives the widget the window's scale factor (and, in the smoke build, `perf.board`'s label override); the spike uses it too.
- *The column rule (§5.15 rule 1).* Any `Input` other than `Wheel` sets the editor column. *Where the plan was silent:* `Cancel` does not either (the widget sends it when the window loses focus during a drag: it is not the user acting on the board); nothing changes the column while a dialog or palette is open (a release that ends a drag the overlay interrupted still reaches the machine, as the canvas chain requires, but the column stays where the overlay returns the keyboard); and a message with no board open is dropped, column included.
- *The footer.* On Board: `Board · 15 nodes · 17 connectors` (hidden under 440 px, as the Markdown counts are), then, in the warning colour, `N issue(s)` when `check` found any and `Read-only` for `Access::ReadOnly`, then the zoom as `47%`, then the save status and its Retry button as today. No line and column. On Raw the counts read `Board as text · N lines` with the line and column, since the text is edited there. "Node" is the plan's user word (`Jump to node…`, §5.19). `BoardFooter::of` builds the text and is what the test reads.
- *The Raw banner.* *Where the plan was silent:* it is a strip of its own at the top of the editor column (raised background, warning text, wrapped), not the window's warning slot, so it cannot be dismissed, never displaces a save or settings warning, and goes with the surface: "This board is shown as text because line L, column C is not valid: MESSAGE. It opens as a board again once the text is valid." The strip is always in the tree, at height 0 unless Raw, as the find bar is, so a Markdown note's widget tree and pixels are unchanged.
- *The theme adapter.* `wren-iced/src/theme.rs` has `board_palette(Theme)` beside `editor_colors`; `App::board_palette`, `sync_editor` and the spike use it. The spike's stand-in palette (`board_spike::palette`) is gone; `tools/board_spike.py` reads the palette from the app, and its five checks pass with the real one.
- *Tests.* `app/board.rs`: the footer of a clean board (two nodes, one connector, `100%`, `75%` after a zoom), a flagged one (`1 issue`), a read-only one (`Read-only`), one with both (`3 issues`, `Read-only`), and the singular forms; the banner names the error's line, column and message and never doubles a full stop; `Message::Board` resizes and fits the view, a wheel pans it and leaves the column, `Cancel` leaves it, a press, a drag and Space take it, nothing takes it under the palette (where the release still ends the pan), and the Session text and dirty flag are untouched throughout. `theme.rs`: every theme's board palette takes the editor surface, text, `danger` and the theme's appearance. The harness focus cases are `tools/wrentest/test_board_view.py`: on a board `note-editor` is the focused widget with the canvas's bounds, the column moves leave it and come back to it, a palette takes the keyboard and Escape returns it, Escape alone keeps it, Enter from the notes list lands on it, a Markdown note mounts the editor under the same id and the board the canvas again, and the board's bytes and mtime are unchanged; a raw board's text is in the focused editor and takes an edit. `tools/wrentest/test_board_native.py` (native X11): a real wheel over the board, with the Notes column active, leaves the column and focus alone, and a real middle click gives the board the editor column and the keyboard. Making the wheel take the column makes that case fail. A8 adds its cases to these two files.

*A6 (the kind icon).* `chrome::Icon::Board` is two rounded boxes joined by an elbow connector, drawn 13 px in `muted` with 6 px before the title, in board rows only. A Markdown row is built exactly as before.

*A7 (Export board as SVG).*
- `note.export_svg`, "Export board as SVG…", group This note, App scope, no default key. `command_enabled`: the surface is Board and no export is waiting for its dialog. `CommandId::boards()` names the commands that exist only with the setting on (this one, for now); the Commands palette skips them when `Preferences::boards` is off, and with no board surface the command is blocked from every path.
- Running it makes the SVG at once, from the view's scene and palette (the current theme), with a canvas-colour background rectangle and `FIT_MARGIN` (40 px) of padding, the margin a fitted board has on screen; closes the palette; and opens `rfd`'s save dialog (title "Export board as SVG", the note's title plus `.svg`, an SVG filter), as `pick_workspace` opens its folder picker. The action is `AsyncPending`; its correlation is the export's tag, and `Message::BoardExportChosen(tag, path)` completes it under that correlation. *Where the plan was silent:* what is written is what was on screen when the command ran, so a theme change while the dialog is open does not change it; one export waits at a time; a cancelled dialog completes and writes nothing; a stale tag is ignored.
- The write: a new file `.<name>.<pid>-<n>.tmp` beside the target (`create_new`), the whole document, `sync_all`, then a rename over the target, so the target is whole or untouched; on failure the temporary file is removed and the error returned. The failure is the banner "Could not export the board to PATH: ERROR" and a blocked outcome with the same warning. Nothing is written into the board or the Session.
- *Pulled forward from A8.* The harness request `board.export {"format": "svg", "path": "/abs/out.svg"} -> {"path", "bytes"}` (§6.2) runs the same document and write without the dialog, because the item's tests name it. It refuses `png` (Phase 3), a relative path, and any surface but Board. A8 need not add it.
- *Tests.* `app/board.rs`: enabled on Board only (not with no note, a Markdown note or raw text), listed and enabled only on a board with the setting on, not listed and blocked with it off, and no other command is held back by the setting; an export waits for its dialog, blocks a second one, keeps the theme it was asked in, ignores a stale tag, writes exactly `svg_document` with the canvas fill, replaces an existing file, writes nothing when cancelled, leaves no temporary file, and after 40 ticks the board's bytes and mtime are unchanged and the document clean; a target in a missing folder, a target that is a folder and one in a read-only folder each give the banner and a blocked outcome and leave nothing; §6.8 Phase 1 item 8: the document parses as XML (`roxmltree`, through the `resvg` dev-dependency), its `<text>` runs are the scene's wrapped lines in order, `a < b & "c"` comes back as itself and is written escaped, and the first element is the background. `app/test_requests.rs`: `board.export` refuses no note, raw text, `png`, a relative path and a missing format, writes the shown board, and returns the path and byte count; an unwritable path is an error. `test_board_view.py`: the palette row is disabled on a Markdown note and enabled on a board, the command is blocked on a note, `board.export` of `Checkout flow` is XML with every label and the background first, a board labelled `a < b & "c"` round-trips through `ElementTree`, an unwritable path is refused, and the board is untouched; with `boards = off` there is no row and the command is blocked.

*For A8.*
- `board.export` exists (above). `board.get` can read `BoardFooter::of` if it wants the footer's words, and `board::raw_banner` for the banner.
- `test_board_view.py` and `test_board_native.py` exist with the cases above; the fixture `samples` copies the sample workspace and `board_app` launches with `boards = on`.
- The save dialog itself is not driven by any test (the harness has no stand-in for `rfd`'s save dialog, as `test.folder.complete` is for the folder picker); `board_export_chosen` is unit-tested from the point the dialog returns.

*Gates* at the head of A7 (`d73b6dc`).
- `make check` before each of the three commits: 2,788 Rust tests in its two passes at A5 and A6 and 2,797 at A7, 28 ignored, 0 failed; 290 Python units.
- `ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11` at `d73b6dc`: 246 passed (the 241 of before and the five new board cases), 1 skipped.
- The board files (`test_board_view.py`, `test_board_native.py`) on X11 tiny-skia (5 passed), headless Weston tiny-skia (4 passed, the native case deselected), X11 wgpu on lavapipe (5 passed) and Weston wgpu (4 passed).
- `cargo clippy --locked --target aarch64-apple-darwin -p wren-iced --all-targets --features smoke-test -- -D warnings`: clean (the chain touched `commands.rs`).
- `tools/board_spike.py` `view`, `keys`, `wheel`, `layers`, `zoom` on X11 tiny-skia: all exit 0 with the spike on the real palette.
- *Existing scenes unchanged.* `capture_scenes.py` in `dark` and `light` on tiny-skia with a smoke build of `8606f7d` (the A4 head, built from `git archive` in a scratch directory) and again after A5, after A6 and after A7: all 66 PNG files byte-identical each time.
- *Board screens looked at* (a scratch script over the test socket on the sample workspace with a conflict-marker board added; not committed, since A8 writes `capture_scenes.py --only boards`), on tiny-skia and on wgpu (lavapipe), dark and light: `Checkout flow` fitted at 47 % with its 15 labels and connectors and the footer `Board · 15 nodes · 17 connectors`, `47%`, `Saved`; `Shapes` with every shape, the placeholder, the dangling stub and `1 issue` in the footer; `Future` with `Read-only`; `Overlap`; the conflict board as text under its banner ("line 1, column 9 is not valid: key with no value, expected `=`") with `Board as text · 6 lines`; the notes list with the board icon on the six boards and none on `Notes` (enlarged crops in both themes); the Commands palette on a board showing "Export board as SVG…" under This note; the columns folded with the board in the wider editor; a Markdown note after a board, with the Markdown footer. Two exports (`Checkout flow` dark, `Shapes` light), rasterised with ImageMagick, match the canvas.
- Not run, as the chain did not touch `wren-board` or `docs/`: `make fuzz-board`, `make perf-board`, the docs build.

**Item A8, session 3.** The harness can read and drive a board, the Phase 1 acceptance items are functional tests in every lane, and `capture_scenes.py --only boards` shoots the Phase 1 scenes.

*The requests* (`app/board_harness.rs`, smoke build only; §6.2 is edited to match).
- `board.get`, `board.bounds` and `board.pointer` first run a widget operation (the `Focused` one `focus.get` uses) for where the widget under `note-editor` sits and whether it has the keyboard, then answer in `App::board_probed` through `Message::BoardProbed`. Unit tests call `board_probed` with a made-up placement. Every request but `board.get` is refused when the surface is not a board.
- *Where the plan was silent.* `board.get` in Phase 1: `state` is `panning`, `idle` or `unfocused` (the widget does not have the keyboard); `tool` is `select`; `revision` is the view's epoch, its reloads, since nothing drafts yet; `pristine` is always true; on a surface that is not a board the board fields are `null` or empty; `error` gains `column`. `elements` defaults to false and lists each node and connector by its schema-1 fields, unknown content left out.
- *Additions to `board.bounds`.* Each node's `fill` and `stroke` and each connector's `stroke`, as the scene draws them, and the window's `scale_factor`, which the pixel checks of §6.8 Phase 1 item 3 need. `visible` is `null` for a node wholly out of view, and is the node's own rectangle when it is wholly in view. A node's `label` is its last text item's lines (after a placeholder's caption).
- *`board.pointer`.* `wheel` (`delta` `x`, `y`, `unit`; `mods` `mod` and `shift`), and `press`, `move` and `release` with the middle button. `at` is a node's centre, a canvas point or a screen point. A press also focuses `note-editor`, as a real press focuses the widget. Left and right buttons, `click`, `double_click`, an edge and a handle are refused until Phase 2. Input goes through `App::board_input`, so the column rule applies.
- *`perf.board`* reports on the open note's board as it did on the spike's. `open_micros` runs from the `OpenNote`, `Back` or `Forward` op that opened the board (from when the view opened otherwise) to the first redraw after its first painted `draw`; each opening is watched once, and only once a window exists. A zoom run zooms about the widget's middle (the spike used a fixed point). A run whose board closes is answered with an error.

*The tests.* `tools/wrentest/boards.py` holds the helpers and imports no pytest, so `capture_scenes.py` can use it on the Mac; the `samples` fixture is in `test_board_view.py`. Item by item of §6.8 Phase 1:
1. `test_board_gate.py`: with the setting off, a workspace with `Flow.wren-board` and `Note.md` lists only the note, has no warning, no board command row, no search hit on a label, `board.get` says `enabled: false`, `note.export_svg` is blocked, and after quit the board's bytes and mtime are unchanged and the state file names no board and has no `[navigation]`. With it on, both are listed by title.
2. `test_board_view.py`: `Checkout flow` is listed with its title, opens with `surface: board`, 15 nodes and 17 connectors, no issue, a zoom within 10 to 100 %, and every node visible and at least 40 px inside the widget.
3. `test_board_view.py`, dark and light: in a screenshot of the fitted flow, a point 6 px inside each filled node, clear of every label, other node and connector, is its fill within 8 per channel (at least five nodes); each node's label box holds at least ten pixels unlike its fill (at least ten labels); at 200 % the middle of at least one connector clear of everything else is its stroke within 24. *Where the plan was silent:* connectors are checked at 200 %, because at the fit's 47 % a 1.5-unit stroke is 0.7 px wide and no pixel holds its colour.
4. `test_board_view.py`: a wheel line moves every node box and label by the same 40 px, Shift sideways; Mod+wheel zooms 1.1 a line and keeps the board point under the pointer within 1 px; `font.reset`, `font.increase` and `font.decrease` step the zoom and leave `config.conf` and the editor's size alone; a middle drag pans by the pointer's motion, reports `panning` during it, and takes the editor column and the keyboard. `test_board_native.py` (X11): real buttons 4 to 7 move every rectangle alike by whole 40 px lines along the wheel's axis, down moving the board up and left and right opposite; Ctrl with buttons 4 and 5 zooms by whole powers of 1.1 about the pointer within 1 px; a real middle drag pans by exactly the pointer's motion; and A5's case (the wheel leaves the column, a middle click takes it).
5. `test_board_gate.py`, for `Checkout flow` and `Hand written`: open, wheel pan, Mod+wheel zoom, a middle drag, `font.increase`, two theme switches, an export, a note switch and back, `window.blur` and a second past the debounce: bytes and mtime unchanged and the document clean, and again after quit.
6. `test_board_gate.py`: `editor.insert` and `editor.command bold` are refused, `markdown.bold` blocked, the engine switch completes and leaves the editor empty; a second later the Session's document is unchanged and clean, and so is the file.
7. `test_board_view.py`: a conflict-marker board opens `raw` with an error on line 1; a typed line is saved as exactly that line before the original text and does not flip the surface; a valid board written on disk and reconciled shows as a board; `Future` is `read_only`; `Shapes` lists a `dangling_edge`.
8. `test_board_view.py` (A7's case): the export parses as XML with every label, the background first, and a label of `a < b & "c"` round-trips and is escaped.
9. `test_board_gate.py`: after open, open, Back, Forward, Back and a flush, `history`, `recent` and `last_open_note` hold Markdown paths only and `[navigation]` holds the board; after a restart the board reopens on the board surface, and `selection` names it. The frozen validator is `wren-core`'s `released_state` test. *Live:* with `WRENTEST_RELEASED_BIN` naming a v0.1.1 smoke build, this build sets a sort and a width with a board open, v0.1.1 opens the workspace with no warning, lists the note only and sets another width, and this build opens it again with both widths and the sort kept and the board shown. Run once in this session against v0.1.1 built from `git archive v0.1.1` in a scratch directory: it passes. Skipped otherwise.
10. `test_board_view.py`: searching `Payment` finds `Checkout flow` as a content hit with `Payment` in its snippet; `schema` finds no board.
11. The core tests (the op-by-kind table and the index counters) are the Core chain's and pass in `make check`.

*`test_board_perf.py`* holds §6.7's debug ceilings in every lane and writes `measurements.json`: `open_micros` of grids of 1,002 and 10,020 elements (the spike's 347 and 3,380 shapes), three opens each, median under 5 s; with a 1,000-shape grid open, no scene build over an idle second and a `perf.view` median under 5 ms; scene plus draw after a change of the whole scene under 100 ms; median `frame_micros` of a 60-step pan and a 30-step zoom at 100 % under 400 ms, with no scene build. `WRENTEST_PERF=release` asserts the release budgets: 150 ms and 1 s to open, 1 ms idle, 33 ms a frame (on wgpu `draw_micros` 8 ms), and 12 ms for the scene change, the canvas chain's replaced number. *Where the plan was silent:* Phase 1 has no edit, so the scene change is a theme switch, which rebuilds the scene once; boards are opened by `row.click`, because `core.apply` answers with the document's text and a 10,000-element board's is over the 1 MiB frame. Debug medians measured (ms; open 1k / open 10k / idle view / scene change / pan frame / zoom frame): X11 tiny-skia 42 / 240 / 0.35 / 34 / 9.0 / 9.7; X11 lavapipe 78 / 267 / 0.33 / 32 / 9.9 / 11.8; Weston tiny-skia 39 / 227 / 0.34 / 35 / 26 / 26; Weston lavapipe 62 / 255 / 0.22 / 32 / 26 / 26. The release run is the phase end's.

*`WRENTEST_BOARDS`* (`tools/wrentest/ui.py`). `off` or `on` adds that `boards` line to the `config.conf` of every `WrenApp` whose settings do not name `boards`. *Deviation:* unset, nothing is added and the build's default holds, where the plan has it default to `off` and write the line. Unset is then exactly the harness before this item, so the pre-existing suite passes unmodified (exit criterion 3(b)) and the harness unit test that no settings file is written without settings still holds; R3 can make unset mean `on`. For the explicit lanes two existing cases changed: `test_theme_and_font_pickers_preview_live_and_restore` compares the settings file with its own contents at the start instead of a literal, and `test_full_command_catalog_and_context_availability` expects `note.export_svg`, disabled on a Markdown note, when the setting is on.

*Scenes* (`capture_scenes.py --only boards`). The sample workspace copied to `target/scene-workspace/boards` with `Conflict.wren-board` added and fixed modification times, opened with `boards = on`: `board-flow` (fitted), `board-flow.svg` (exported then), `board-notes-list` (the flow with the Notes column active), `board-zoom-in` (200 %), `board-zoom-out` (25 %), `board-shapes`, `board-overlap`, `board-raw`, `board-search` (`payment`) and `board-wide` (columns folded, at 50 %: Phase 1 has no fit command, and a board keeps its viewport when the window around it widens, so it sits to the left).

*Gates* at the head of A8.
- `make check`: 2,801 Rust tests in its two passes, 28 ignored, 0 failed; 293 Python units.
- `ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11`: 262 passed, 2 skipped (the live case and the earlier skip). With `WRENTEST_BOARDS=on`: 261 passed and one failed, the command catalog, which now expects the board command with the setting on; after that change it passes with the setting on and off.
- The board files (`test_board*.py`): X11 tiny-skia 21 passed, 1 skipped; X11 lavapipe 21 passed, 1 skipped; Weston tiny-skia and Weston lavapipe 17 passed, 1 skipped, the 4 native cases deselected. The live case against v0.1.1: passed.
- `cargo clippy --locked --target aarch64-apple-darwin -p wren-iced --all-targets --features smoke-test -- -D warnings`: clean.
- `tools/board_spike.py` `view`, `open`, `perf`, `wheel`, `zoom` on X11 tiny-skia, since the spike's `perf.board` code changed: all exit 0.
- The strict docs build: no issues.
- Scenes looked at, tiny-skia and lavapipe: `board-flow` in both themes (47 %, every label, the footer), `board-notes-list` (the Notes column active, the icon on the six boards), `board-zoom-in` (200 %, labels sharp), `board-zoom-out` (25 %: tiny-skia draws no label under 4 px, lavapipe draws them), `board-shapes` (every shape, the free text, the placeholder and its caption, the dangling stub, `1 issue`), `board-overlap` (occlusion right), `board-raw` (the banner naming line 1, column 9, `Board as text · 20 lines`), `board-search` (the snippet with `Payment` highlighted), `board-wide`. `board-flow.svg` rasterised with ImageMagick matches the canvas.
- Not run, as the chain did not touch `wren-board`: `make fuzz-board`, `make perf-board`.

**The fix step, run 1b.**

*Deviations.*
1. *The issue colour stays the red hue until Phase 3.* Reproduced: `issue` and `hues[0]` are both the theme's `danger` in all eleven themes (CIE76 ΔE 0), and in `board-shapes` the issue dot on the red solid `Document` is half lost in its fill, its stub drawn in the same red. It is not changed here because no provisional colour removes the collision, it only moves it: seven hues around the wheel leave no saturated colour far from all of them, and `warning`, the obvious other role, comes within ΔE 4.4 of the derived orange (Dracula) and 7.9 of the derived yellow (Nord). The fix belongs where the colours are chosen for good: ST1 and ST3 add a rule that `issue` is a set distance from every hue at solid and at stroke in every theme, or the scene gives the marker and the stub a canvas-coloured ring so they read on any fill (a `wren-board` scene change, with its goldens). Phase 3 chooses; until then a flagged red element's marker is hard to see, and the footer's `N issue(s)` still names it.
2. *Commit trailers wait for the owner.* Every commit of the chain, this step's included, ends with a `Claude-Session:` link after its `Co-Authored-By:` line, because the session's attribution rule asks for both. Removing it means rewording every commit of the chain before anything is pushed; that is the owner's decision, made once for the whole effort, and is not taken here.

*Review.*
- Counts: 0 blockers, 0 majors, 3 minors (the issue colour equal to red, no check of geometry builds while idle, the `Claude-Session` trailers).
- Fix commits: `0d44daf` `test(boards): check an idle board builds no geometry`. `perf.board` reports the open board's `parses`, `scenes` and `builds` (§6.2 and `docs/development/testing.md` say so), and `test_board_perf.py` asserts that none moves over the idle second with the 1,000-shape grid open, the regression test for exit criterion 9; a pan run must move `builds`, so the idle check cannot pass on a dead counter. All four lanes measured 0, 0 and 0.
- Not fixed: the issue colour (deviation 1: provisional palette, the remedy is Phase 3's, ST1 and ST3 or the scene) and the trailers (deviation 2: the owner decides before any push).

*Gates* at `0d44daf`: `make check` (2,801 Rust tests in its two passes, 28 ignored, 0 failed; 293 Python units); `ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11`, 262 passed and 2 skipped; the board files on headless Weston tiny-skia, 17 passed, 1 skipped, 4 native deselected; `test_board_perf.py` on X11 and Weston with wgpu on lavapipe, passed; the strict docs build, no issues. Nothing draws differently, so no scene was captured; `wren-board` is untouched, so `make fuzz-board` and `make perf-board` were not run.

*Verification* at `a21784a` (the fix step's record, `docs(discovery): record the boards-p1-app fix step`): no blocker or major open, the idle-geometry minor closed by `0d44daf`, the other two minors open as deviations 1 and 2 say. The gates above re-run with the same counts: `make check` 2,801 Rust tests and 293 Python units; the X11 suite 262 passed and 2 skipped, the board files there gate 6 passed and 1 skipped, native 4, perf 1, view 10; Weston tiny-skia 17 passed, 1 skipped, 4 native deselected; `test_board_perf.py` on X11 lavapipe passed; the macOS cross-lint clean; the strict docs build clean. Idle counts with the 1,000-shape grid open were 0 parses, 0 scene builds and 0 geometry builds on X11 tiny-skia and lavapipe. `capture_scenes.py --only boards` on tiny-skia, dark and light, is byte-identical to the review's capture, so the fix step changed nothing drawn.

#### boards-p1-route
Item R1 (elbow connectors that do not run through shapes), owner decision 16 of §2.4, session 1.
- *What was built.* `route::route_all(boxes, connectors)`, which `scene::build` calls once with every drawable connector in id order; `route::route_among(kind, from, to, &Obstacles, ends)` for one connector against a lookup of the node boxes (`Obstacles`, rectangles bucketed in 256 px cells); `route::route` is `route_among` with no boxes. The rule is the Routes row of §3.7. Nothing is stored: no schema change, no new field, no fixture changed.
- *Deviation: the order of comparison.* The brief asks for the fewest shapes crossed, then the fewest turns, then the shortest. Shapes crossed come first as asked; after that a route has a cost, length plus 40 px per turn plus 200 per shared line or crossed fixed line. With turns compared before anything else, `Retry payment -> Charge card` took a Z on top of `Payment ok? -> Retry payment` and its label, the only two-turn route with nothing in the way; with shared lines compared before turns, it took a four-turn route round the back of the main column. A weighted cost picks the three-turn route out of Retry's top into Charge card's right side.
- *Deviation: connectors see some other connectors.* "Routing per connector against a lookup of node boxes" holds for every connector whose sides are fixed (straight ones, pinned and anchored ends, and floating ends held by rule 4, which is 14 of Checkout flow's 17). A connector with a side to choose is also routed against the lines of those fixed connectors (and, since the fix step below, of the free connectors before it in id order), and a last pass (`separate`) moves a middle segment off a line already drawn, in id order. Without the first, `Notify customer -> Check stock` ran up the gap through `In stock? -> Backorder` and its `No` label and met Retry's connector head-on along y = 630; without the second, it and `Reject order -> End` shared x = 340 for 240 px. For Phase 2's incremental rebuild: a moved node re-routes the fixed connectors near it, then the free connectors near those or the node and, in id order, the free connectors after them that are near their lines, then the separation of the lines they touch; nothing depends on connectors far away, and the order is fixed by id.
- *Where the plan was silent or changed: rule 4's "should elsewhere".* M4 drew a floating end on its nominal side always. Now, when the two boxes are apart on both axes, a floating elbow end may take any side, nominal first on a tie. So a diagonal elbow with nothing in the way is one turn (an L), not a Z, as "fewest turns" asks; rule 4's "must" is unchanged, and `floating_ends_take_the_nominal_side_when_apart_on_one_axis` still runs its 2,000 pairs. `board.select.connector_<dir>` reads the side drawn, as §5.12 rule 4 already says. A free point still leaves by its nominal side; a straight route is unchanged.
- *Golden files* (regenerated with `WREN_BLESS=1`, all intended): `checkout-flow.scene` and `.svg`, the three loop connectors: `Reject order -> End` leaves Reject's right side and runs down x = 350 into End's right side; `Notify customer -> Check stock` leaves Notify's right, runs up x = 340 and into Check stock's right (x = 340 is inside x = 350, so the two never cross); `Retry payment -> Charge card` leaves Retry's top, turns at y = 700 and x = 140 and enters Charge card's right. `shapes.scene` and `.svg`: the child-to-hexagon connector leaves the child's right side (shorter, same two turns). `shapes-and-ends.scene` and `.svg`: the free-end connector `e5` turns at y = 290 instead of 240, so it no longer crosses `e4` twice. The other seven scene and SVG files are unchanged. Both SVGs were rendered and looked at.
- *Tests.* `obstacles_between_and_beside_the_ends`: nine layouts (a box between in a column, in a row, wider than both, across an offset Z, at the corner and on the turn of a diagonal, and beside each), every shape: the route keeps out of the box, rule 4's sides hold where it applies, a box out of the way changes nothing, a straight route is the same. `a_route_is_drawn_when_avoidance_is_impossible`: a box walled in on four sides, three end forms. `a_middle_segment_moves_off_a_nodes_centre_line` (and not past `REACH`), `connectors_on_one_line_move_apart`, `a_free_connector_keeps_off_the_lines_of_fixed_ones`. `the_routing_table_among_obstacles`: the 25,920-pair table again with six boxes placed between, beside, across and around each pair, each route also checked deterministic. `random_routes_hold` (300 boards of up to 15 boxes and 11 connectors of every end form, [20,000 in `make fuzz-board`]): deterministic, finite, sound, pinned sides kept, rule 4 kept. `checkout_flow_routes_keep_out_of_shapes_and_off_each_other`: no route enters a box it does not join, and no two connectors share more than 4 px of a line except within 20 px of an end they share. Changed: `pinned_and_free_ends`, where the free point now enters A's right side with one turn rather than its bottom with two.
- *Gates.* `make check` passes (Rust and 293 Python units); `make fuzz-board` (7 tests, 6.5 s); the X11 tiny-skia suite on both engines, 262 passed and 2 skipped (no v0.1.1 build named; the classic engine's hanging indent); the board files on headless Weston, 17 passed and 1 skipped; the board files on X11 wgpu, 21 passed and 1 skipped. No test outside `wren-board` pins route points: the harness's pixel checks take connector midpoints from `board.bounds`, and the canvas's layer counts did not change.
- *Performance* (`make perf-board`, release, median of nine): 1,000 shapes 3.5 ms (5 ms budget; 2.9 ms at M7); 10,072 elements 24.4 ms (24 ms at M7). Those grids have only fixed connectors. A grid of 1,000 shapes whose every connector is diagonal, so each searches its whole candidate set among dense boxes, now printed by the test: 11.2 ms, after two prunes (a pair of sides is skipped when its least possible cost cannot beat a clean route already found, and detours are pushed only while nothing clean is found); 18.5 ms without them. No budget names that board.
- *Scenes.* The board scenes were shot before and after, dark and light, on tiny-skia and wgpu (`target/scenes/before-*` and `after-*` in this worktree). Changed: `board-flow` (PNG and SVG), `board-zoom-in`, `board-zoom-out`, `board-wide`, `board-notes-list` (each shows Checkout flow) and `board-shapes` (the child connector). Byte-identical: `board-overlap`, `board-raw`, `board-search`.
- *Known limits.* A box containing an end, as `frame` contains `child` in Shapes, is entered by every route out of it, so it does not steer the choice: the child's connector now passes the frame placeholder's centred label, where the old one passed above it. Two connectors that each choose a side at one node may still meet on one port when every other side costs more than the shared line (`SHARED_COST`). Connector labels are not obstacles. The canvas chain's note above that Checkout flow's loops run through shapes describes the router before this item.

- *Fix step (session 3).* The review found one major and three minors. Each was reproduced first; the numbers below are release builds, best of three, with the reviewer's own probe boards.
  - *Major, scene build grew with connectors times nearby boxes: fixed.* At the review's head, a 40-column grid of 5,000 nodes took 2.6 s with random connectors and 10.8 s with a fan-in; 5,000 shapes stacked on one spot took 7.9 s chained and 15.4 s joined to one far shape. Each connector now has a lookup budget (§3.7, Routes): every grid cell visited and every box or line looked at costs one unit, 12,288 while it is routed and 1,024 while it moves aside. One that runs out keeps the best route it has fully scored, else the plain route of its nominal sides, and stays put in the separation pass; the count is deterministic and local, so Phase 2's incremental rebuild is unaffected. The boxes that give channels are the 16 nearest the ends (`NEAR`), looked up around each end alone when the hull of both spans more than 64 cells. The same four boards now take 408, 182, 77 and 93 ms. Checkout flow's costliest connector spends 7,152 units and its separation at most 282, so no golden file or scene changed. Regression tests: `a_crowded_connector_keeps_its_plain_route` (40 connectors among 5,000 stacked boxes keep their plain routes; at the review's head it takes a searched route) and three new `make perf-board` boards at 1,000 shapes and about 10,200 elements, under ceilings of 150 ms and 600 ms.
  - *Not met: §6.7's 5 ms scene row on those boards.* At 1,000 shapes they take 46 to 71 ms, against about 1.3 ms before R1. Each connector on them searches until its budget runs out, and the budget must cover Checkout flow's 7,152 units with headroom; a budget small enough for 5 ms (about 1,000 units) would change the owner's sample. The row's own board, the grid with neighbour connectors, still takes 3.1 ms. Cheaper lookups were tried: an array of cells in place of the hash map gained 7 % and was dropped; sorting a query's result only when it merges two cell lists is kept, though it gains little. At 10,000 elements the boards take 110 to 428 ms, within the 1 s open budget.
  - *Minor, stale text: fixed.* The Connector ends row of §3.7 now gives the Routes row's rule; §4.4 says the path is computed from the board; the doc comments of `Attach::Floating` and `Routed::sides` match.
  - *Minor, free connectors on one another's lines: fixed.* Free connectors are routed in id order against the lines of the fixed connectors and of the free ones before them. The reviewer's fan-out (a node joined to three nodes in a column below and to its right) shared trunks of 170 and 370 px and now shares none: the second leaves by the right and the third by the left. The pair each way among four boxes shared 190 px and now shares none. Regression test: `free_connectors_keep_off_each_others_lines`. No golden file changed.
  - *Minor, Shapes' child connector through the frame placeholder's label: not fixed.* The suggested cost, the length run inside a box that holds one of the ends, was tried: it changes no golden file, because the old route runs 320 px inside the frame and the new one 160 px. Treating labels as soft obstacles needs the labels laid out before routing, which `scene::build` does after it, with the host's measurer; that is not a cheap change. Most of the label is already covered by the child, which sits above the frame, so the visible cost is small. It stays a known limit.
- *Fix step gates.* `make check` passes at each commit (2,821 Rust tests, 0 failed, 30 ignored; 293 Python units); `make fuzz-board` 7 passed; `make perf-board` passes (the numbers above; the 1,000-shape grid 3.1 ms, 10,072 elements: parse 48 ms, scene 23 ms). Board files: X11 tiny-skia 21 passed, 1 skipped (no v0.1.1 build named); headless Weston 17 passed, 1 skipped, 4 deselected; X11 wgpu on lavapipe 21 passed, 1 skipped. Scenes: `capture_scenes.py --only boards`, dark and light, tiny-skia and wgpu (`target/scenes/fix-*`), all 40 files byte-identical to `after-*`; `board-flow` and `board-shapes` were looked at.
- *Review.* 0 blockers, 1 major, 3 minors.
- *Fix commits.* `be7ca3b` (the routing budget), `145b61e` (the stale text), `51c73e1` (free connectors see one another).
- *Not fixed.* The Shapes label minor, for the reason above. Of the major, the 5 ms row on boards where every connector exhausts its budget (46 to 71 ms at 1,000 shapes), for the reason above; the growth itself is fixed.
- *Verification* at `05bf4cc`: no blocker open; the gates re-run with the fix step's counts (`make check` 2,821 Rust tests and 293 Python units; `make fuzz-board` 7 passed; `make perf-board` passes, the new boards 46 to 69 ms at 1,000 shapes and 110 to 430 ms at about 10,200 elements; the board files 21 passed and 1 skipped on X11 tiny-skia and on X11 wgpu, 17 passed, 1 skipped and 4 deselected on headless Weston); the 40 board scenes byte-identical to `after-*`. The reviewer's probe boards are linear now (grid fan-in 183 ms at 5,000 nodes and 545 ms at 25,000, against 10.8 s at 5,000 at the review's head). With `BUDGET` set to 1,024, about what 5 ms allows, the Checkout flow golden files and `free_connectors_keep_off_each_others_lines` fail, so the reason above holds. The residual of the major, the 5 ms row on budget-exhausting boards, is open and carried into the Phase 1 end review (§8.1); Phase 2 should know that a full scene rebuild per drag frame costs 46 to 71 ms on such boards unless the incremental rebuild above lands. Minors 1 and 3 are closed; the Shapes label minor stays open.

#### Phase 1 end
Run 1c, from `1a06c6b` (the merge of `boards-p1-route`) to `6732183`. The setting-off gates, the data-safety lens and the Codex pass ran at `1a06c6b`, each review in its own detached worktree; triage, one fix session, a verify session and the final gates followed on `boards`. Raw outputs are under `.claude/board-notes/workflow-results/` (not committed). The deviations accepted in Phase 1's chain subsections are now in the spec text of §3 to §5 and in §6.7's frame-build row; the subsections above keep their history.

*Findings.* The lens reported 2 issues, Codex 2. Triage reproduced all four and dropped none; there were no duplicates (the lens did not look at export destinations or writer idempotence, and Codex's narrow brief of §8.3 did not reach the renderer). After triage: 2 blockers, 2 majors, 0 minors. All four are fixed.

*Fixed.*
1. **Blocker: a right-to-left label crashed the app** (data-safety lens). Every label line was laid out with unbounded width, and cosmic-text starts a right-to-left line at the right edge of its room. *Before:* one board with an Arabic label (`العربية`) panicked debug builds on both renderers (`glyph_cache.rs:100:18`, add with overflow, exit 101) and release tiny-skia (`tiny-skia-path` `size.rs:88`); release wgpu drew an empty box. The state file reopens the board, so the workspace crashed on every launch. The lens's fuzz with right-to-left labels failed at step 2 of seed 2. *Fix* (`5b8078f`): each line is laid out in its measured width plus one em, capped at 2^24 px (§3.8). That alone drew right-to-left lines one em too far right, because iced lays a single right-to-left line out again at its own width only for `Alignment::Default`, so paint now uses it. *After:* the lens's 11 right-to-left probes (Arabic, Hebrew, wrapped mixed direction, `abc\nשלום`, U+200F, U+202E, digits then Arabic, a text node, a connector label, an unknown kind's caption) pass on both renderers in debug and release, and the Arabic label is drawn on release wgpu; the fuzz with right-to-left labels runs seed 2 for 300 steps and seed 3 for 600 with no failure. *Regression test:* `test_right_to_left_labels_are_drawn` (`test_board_view.py`, every lane) checks each label box is inked across its width; it fails with `Alignment::Left`. The Shapes sample's rectangle is now `Rectangle\nמלבן`, so the scenes show a right-to-left line, with its scene and SVG goldens updated; the resvg width check skips lines Inter has no glyphs for.
2. **Blocker: a far arrowhead crashed tiny-skia** (data-safety lens). Paths went to the renderer whole, and culling is per item. *Before:* a connector with an arrowhead between a node at the origin and one at (-8,000,000, 8,000,000), inside the range §4.6 calls valid, panicked release tiny-skia at 300 % (`alpha_runs.rs:189:51`, exit 101), as did free ends out to ±16,777,216; the fuzz's damaged Extremes board panicked debug at 150 %. wgpu drew them correctly. *Fix* (`a34cb12`): `clip.rs` cuts any path reaching more than 2,048 px past the widget (§3.8); a path inside is drawn as before. *After:* all 107 of the lens's far-geometry probes pass on debug tiny-skia and the 26 with heads on release; the Extremes board passes every zoom stop in debug and release. *Regression tests:* `test_a_connector_millions_of_units_long_is_drawn_at_every_zoom` (every zoom stop, a screenshot at each) and 8 unit tests in `clip.rs`.
3. **Major: SVG export could replace a note** (Codex). The export wrote to any destination, around the Session. *Before:* `board.export` to `Checkout flow.wren-board`'s own path replaced its TOML with 7,905 bytes of SVG while the Session said clean; after 3 s and after a relaunch the note was raw SVG text. Triage set it major, not Codex's blocker: every save dialog in use confirms an overwrite (rfd's GTK backend sets `do_overwrite_confirmation`; the portal and macOS confirm too) and offers `<title>.svg`, so the loss needs the user to type another name and confirm. Read §8.1's "lost data" literally and it is a blocker; it is fixed either way. *Fix* (`a714a30`): the rule in §3.3, Export SVG, checked once for the dialog and `board.export`. *After:* six path forms (the board's own path, through `..`, through a symlinked folder, a Markdown note, `.wren/workspace.toml` through `..`, an extensionless name inside the workspace through a symlink) are refused, with bytes, mtime, surface and dirty flag unchanged and the board still a board after a relaunch; a real `.svg` inside the workspace still exports. *Regression tests:* `an_export_never_replaces_a_note_or_a_file_of_the_workspace` (`app/board.rs`) and the harness export case (the open board's path and a Markdown note's refused, bytes and mtime unchanged).
4. **Major: the writer was not idempotent for a CR in a label** (Codex). *Before:* a model built in code with `A\r\nB` or `A\rB` in a node or connector label wrote `\r`, read back as LF and wrote different bytes, against §4.11 rule 10: 3 of triage's 4 cases failed (the LF control passed). No file and no Phase 1 action makes such a model, but Phase 2's label editor and paste would. Codex called it minor; triage raised it for the broken written rule. *Fix* (`6732183`): the writer turns CRLF and a lone CR into LF in label text only (§4.9); `tools/board_format.py` does the same. *After:* every case of a wider set (`\r\n\r`, `x\r\r\n\n\ry`, `\r`, `end\r`, `\r\nstart`, in node and both edge labels, together and apart) holds over two round trips with no CR in the output. *Regression tests:* `a_label_with_a_carriage_return_is_written_as_it_reads_back`, the round-trip generator of §4.16 test 3 now putting CR in labels (it failed before the fix), and a matching unit for the reference writer.

*Not fixed, by choice.*
1. **Major, a missed budget: scene + draw after a change of the whole scene, in release.** §6.7's row is 8 ms, replaced by 12 ms fitted in the canvas chain; the release run of `test_board_perf.py` asserts 12 ms and fails. Measured on X11 tiny-skia, 1,000 shapes fitted, a theme switch, median of five: 12.3, 22.6 and 23.6 ms in the final gates. The spread is the machine: its processor has performance and efficiency cores, and with every process of the run pinned to the efficiency cores the result is 11.6 to 12.5 ms (scene build 10.3 to 11.0, draw 1.3 to 1.5), and pinned to the performance cores 22.5 to 23.5 ms (scene 20.8 to 21.6, draw 2.2 to 2.3); unpinned runs land in either mode; the powersave governor held the performance cores at 0.4 to 1.8 GHz when read idle, against about 4.5 GHz on the others. The phase-end fixes cost nothing here: `1a06c6b`, before them, measures the same in both modes. The router of item R1 does: at `3f72438`, before it, the efficiency cores give 8.1 to 8.9 ms (scene 6.8 to 7.5) and the performance cores 16.6 ms (scene 14.0). So the scene build is the cost, as the canvas chain found, and R1 added about 3 ms to it on the efficiency cores and 6 on the others. The cut is the one the canvas chain named: rebuild only the scene items and layers of what an edit changed, from Phase 2's `edit::Change`, which also bounds the routing share; Phase 2 measures it against §3.4 rule 7 and should time on one core type (`taskset`) or report both. The test's 12 ms is not relaxed, so the release run stays red until that cut or an owner decision. The rest of the release run passes its budgets (X11 tiny-skia, `6732183`, the ranges over the runs above): open 18.4 to 22.5 ms at 1,002 elements (150 ms) and 112 to 158 ms at 10,020 (1 s); idle view 0.09 to 0.17 ms median with zero parses, scene builds and geometry builds (1 ms, zero); pan frames 2.8 to 3.0 ms and zoom frames 3.0 to 4.3 ms median (33 ms). wgpu and Wayland were not run in release.
   - *Corrected 2026-11-12 (`boards-p1-tune`).* The two modes above are the measurement, not the code: each sample followed a 0.4 s sleep, so it ran on a core at its idle clock, and the "performance cores" result is those cores idling at 0.4 to 1.8 GHz. Timed back to back after three changes left out (§6.7), the same build (`2ad7a29`) gives 8.4 to 8.5 ms pinned to the performance cores, 10.5 to 11.0 ms pinned to the efficiency cores and 9.0 to 12.4 ms unpinned, and the performance cores are the fast ones. The router's cost was then cut without changing a route (`boards-p1-tune`, T2): the release run then measures, on a quiet machine, 6.2 to 8.7 ms on X11 tiny-skia and 8.9 to 12.9 ms on X11 wgpu under lavapipe (up to 17.4 while other work loaded the machine). The owner accepted on 2026-11-12 that the budget is relaxed for the MVP: the release run asserts 12 ms on tiny-skia and 16 ms on wgpu (20 ms until the chain's fix step) and passes on both on a quiet machine; 8 ms waits for Phase 2's per-element rebuild. "The test's 12 ms is not relaxed, so the release run stays red" no longer holds.
2. **Major, carried from `boards-p1-route`:** the 5 ms scene row is missed on boards where every connector spends its routing budget (46 to 71 ms at 1,000 shapes), for the reason recorded there; the same Phase 2 cut applies.
   - *Updated 2026-11-12 (`boards-p1-tune`).* 16 to 55 ms at 1,000 shapes after the router's exact savings (T2); still missed, for the same reason.
3. **Minor, carried from `boards-p1-route`:** Shapes' child connector passes the frame placeholder's label; labels are not obstacles, and making them so needs labels laid out before routing.
4. **Minor, carried from `boards-p1-app`:** the issue colour is the red hue in every theme until Phase 3 chooses colours (ST1, ST3) or rings the marker.
5. **Minor, carried from `boards-p1-app`:** every commit ends with a `Claude-Session:` line after `Co-Authored-By:`; removing it means rewording the branch before any push, which is the owner's call.
6. **Minor, carried from `boards-p1-core`:** `CLAUDE.md`'s Layout lacks `kind.rs` and `workspace_state.rs`; `boards-p3-docs` (D1) owns `CLAUDE.md`.
7. **Open choice:** `Notify customer` does not fit the cylinder (§3.7, last paragraph); the cylinder's inset or the default size can still change before Phase 2's golden layout.
8. **Limits of the review:** the lens's clean 7,400 fuzz steps left right-to-left words out of generated labels (after the fix the right-to-left replays above are clean); its wgpu runs were lavapipe under Xvfb, and Wayland and the Mac were not fuzzed; its check of invariant 6 used a Python port of the frozen validator. The far-geometry crash came from a directed probe, not a seed.

*Gates.*
- Setting off, at `1a06c6b`, before the reviews: `make check` 2,821 Rust tests passed, 30 ignored, and 293 Python units; X11 tiny-skia 262 passed, 2 skipped; Wayland tiny-skia 202 passed, 2 skipped, 60 deselected; X11 wgpu 262 passed, 2 skipped; Wayland wgpu 202 passed, 2 skipped, 60 deselected.
- The fix session: `make check` before each commit; the full X11 tiny-skia suite on both engines 264 passed, 2 skipped; the board files on X11 wgpu (23 passed, 1 skipped) and on Weston with each renderer (19 passed, 1 skipped, 4 deselected); `make fuzz-board` 7 passed; `make perf-board` passed; the strict docs build (`docs/development/testing.md` changed). The verify session re-ran these and every reproduction above, in debug and release, and changed nothing.
- Final gates, at `6732183`, one lane at a time with `WRENTEST_ENGINE=classic,advanced`:

| Lane | Setting off | Setting on |
|---|---|---|
| X11 tiny-skia | 264 passed, 2 skipped | 264 passed, 2 skipped |
| Wayland tiny-skia | 204 passed, 2 skipped, 60 deselected | 204 passed, 2 skipped, 60 deselected |
| X11 wgpu (lavapipe) | 264 passed, 2 skipped | 264 passed, 2 skipped |
| Wayland wgpu (lavapipe) | 204 passed, 2 skipped, 60 deselected | 204 passed, 2 skipped, 60 deselected |

  - `make check`: 2,843 Rust tests passed, 0 failed, 30 ignored; 294 Python units.
  - The two skips in every lane are the live v0.1.1 state-file case (no `WRENTEST_RELEASED_BIN` given; it passed against v0.1.1 in A8) and the classic engine's hanging indent. The 60 deselected are the native X11 cases.
  - The acceptance tests of §6.8 Phase 1 run in those lanes (`test_board_gate` 6 passed and the live case skipped, `test_board_native` 4, `test_board_view` 12, `test_board_perf` 1, the native file on X11 only); item 9's core half and item 11 pass in `make check`.
  - `make fuzz-board`: 7 passed in 6.5 s. `make perf-board`: passed; 1,000 shapes (1,975 elements) parse 5.4 ms, write 0.43, scene 3.1, SVG 3.8; 5,100 shapes (10,072 elements) parse 49.6, write 2.4, scene 24.0; routing at 1,000 shapes 11.3 (diagonal), 69.7 (long random), 51.2 (fan-in), 46.3 (stacked); at about 10,200 elements 439.8, 197.0 and 114.7; 50 labels of 8,000 words 97.9 ms.
  - The release performance run: failed on the edit budget alone (Not fixed, item 1).
    - *Updated 2026-11-12 (`boards-p1-tune`).* Since `boards-p1-tune` it passes on X11 tiny-skia and X11 wgpu, on a quiet machine, under the relaxed ceilings of 12 and 16 ms (Not fixed, item 1, its correction).
  - The strict docs build: no issues.
  - Not run at the phase end: the aarch64 cross-lint (no file it covers changed since the app chain ran it), the Mac step, and the board scenes in both themes on both renderers (after the fixes they were shot on X11 tiny-skia, dark: `board-shapes` shows the Hebrew line centred under `Rectangle`, `board-zoom-in` the flow unchanged). Exit criterion 3(d), Markdown scenes byte-identical to `main`, was last checked at A7 (`d73b6dc`); everything changed since draws only on a board.

*Waiting for the owner.*
- **The tag.** `boards-p1` is not tagged: the release edit budget (Not fixed, item 1) fails the release run. Either accept the recorded number for Phase 1 and tag, or hold the tag for Phase 2's cut. The severity of finding 3 (major or blocker) changes nothing now that it is fixed.
  - *Updated 2026-11-12 (`boards-p1-tune`).* The release run now passes on both renderers under the ceilings the owner relaxed for the MVP (Not fixed, item 1, its correction), so the tag waits only on the owner's acceptance of this record and on the Mac step.
- **The Mac step** (§6.6): sync the tree without `target/`, `.venv/` and `.claude/`; `make build fmt-check clippy test check-patches`; the smoke build; `capture_scenes.py --only boards`; copy the scenes back.
- **Mac console checks** of §6.4 due after Phase 1: 5 (trackpad: two fingers pan, ⌘+scroll zooms at the pointer, momentum stays a pan) and 7 (Retina sharpness; pan and zoom of the 1,000-element sample smooth on Metal). The spike's wanted numbers (Metal, a desktop GPU) wait with them. They do not block the tag.
- **The push and CI**, after deciding the commit trailers (Not fixed, item 5). CI runs on every push of `boards`.
- **The hand trials** of §6.4's Phase 1 row, by the recipe of §6.5: open the sample workspace; see shapes, labels and connectors; pan with the wheel, a middle drag and a Space drag; zoom with Ctrl+wheel and Alt+=; switch themes; open the raw fallback; search for a label; export SVG and open it in a browser; with the setting off, the boards are absent.
- **Optional before the tag:** the live v0.1.1 state-file case at this head, with `WRENTEST_RELEASED_BIN` naming a v0.1.1 smoke build.

#### boards-p1-tune
Run 1c-tune, from `2ad7a29`, one session: items T1 to T3, from the Phase 1 end record above and the owner's decision of 2026-11-12, "Tune, then tag": profile the router and take only savings that leave every drawing unchanged, make the measurement independent of the processor's idle clock, and set the ceiling at the measured number. §7.2 has no row for them. Commits: `9634c4d` (T1), `a5426c4` and `ef067e7` (T2), `f3166ac` (T3).

*T1, macOS test portability* (test code only). The four tests the Mac run failed: `a_non_utf8_board_name_gets_the_markdown_warning` is now `#[cfg(target_os = "linux")]`, as the older non-UTF-8 scan tests are, since APFS refuses the name; `a_scan_without_boards_equals_a_scan_of_the_folder_without_them` keeps its non-UTF-8 board to Linux and runs the rest everywhere; it also stripped the temporary directory from the scan's canonical warning paths, a second macOS failure hidden behind the first, and now strips the store's root; `a_scan_with_boards_lists_their_titles_and_kinds` compares warnings with the store's root and `a_session_with_boards_lists_titles_kinds_and_warnings` with the session's. The sweep read every test added since `main` in `wren-core` (scan, file store, kind, index, ops, state file, settings, theme, and the session, state-file and navigation suites), `wren-board`, `wren-canvas`, `wren-iced` and `tools/`: no other test writes a name that is not UTF-8 (`kind.rs`'s builds one in memory only), compares a path with a temporary directory (the app's tests take theirs from `test_support::tempdir`, which canonicalises it, and the state-file suites compare relative paths), or relies on case (`Plan.MD`, `Flow.SVG`, `Map.WREN-BOARD` and `Flow.WREN-BOARD` never share a folder with a name differing only in case); the Python cases read and write under `tmp_path` and `capture_scenes.py` resolves its workspace path. Checked on Linux with `TMPDIR` reached through a symbolic link, as macOS reaches `/private/var` through `/var`: the three path tests fail before the change and pass after, and the whole workspace suite passes; the Python units fail there only where a Unix socket path passes 108 bytes, as on `main`. `cargo clippy --locked --target aarch64-apple-darwin --workspace --all-targets --features smoke-test -- -D warnings` is clean. Not run on a Mac (the orchestrator's step).

*T2, the router's cost with every drawing unchanged.* Profiled with timers in a throwaway copy, release, on the perf test's 1,000-shape grid (`grid_board`, built with the fixed-width measurer) and the `make perf-board` boards. On the grid, pinned to an efficiency core: scene 7.3 ms, `route_all` 1.8 ms, and 1.1 ms more in the scene's own glue around it, which found each connector's two ends through three ordered maps keyed by string to route it and again to draw it. On the routing-heavy boards every connector spends its budget, so the time is cost per budget unit: sorting each query's result to drop repeats (4.7 million entries a build on the 1,000-shape fan-in and stacked boards), SipHash lookups of cells, and building line lookups one hash entry at a time. The savings, each leaving the budget count unchanged (every query visits the same cells and charges the same units, so a connector runs out at the same point), and every user of a query's result counting, testing for any, or sorting it itself:
- a box listed in several cells is taken from the first one the query reaches, by marks on its first column and row in each cell entry, instead of sorting the result (`Grid::query`);
- a candidate's entered boxes are counted once by stamps instead of a sort (`Router::hits`);
- the node boxes, and the lines drawn by fixed connectors and the first and last segments the separation pass starts from, are laid out in one array of cells read without hashing when their cells are compact (no more than four per entry); lines added later go to the map;
- buffers are reused from one connector to the next, `simplify` works in place and the sides to try are an array;
- no line lookup is built when no connector has a side to choose, and the separation pass is skipped when no route has a middle segment;
- the scene finds each drawn node by one hash lookup, with its index among the router's boxes, and draws a routed connector from its route alone, looking its ends up again only for a dangling stub (`ef067e7`).

Not taken: a keyed fast hash for the map (1 % once the array was in), stopping a nudge once nothing can beat its best (it would change how much separation budget the connector's later segments get), and skipping the work of a query that will run out (one per connector, a few per cent). `BUDGET`, `SEPARATE_BUDGET`, `NEAR` and the cost constants are unchanged.

Proof that no route changed: every golden file under `crates/wren-board/golden` is byte-identical (`WREN_BLESS=1` writes nothing new); the 40 files of `capture_scenes.py --only boards` in both themes on tiny-skia and wgpu are byte-identical before and after (`target/scenes/before-*` and `after-*` in the worktree; `board-flow` looked at); and a throwaway harness, outside the repository, dumped `route_all`'s output for 42,900 boards from the fuzz generator (seed 11 at 300 and seed 12 at 20,000, as `random_routes_hold` and its fuzz run, seed 13 at 20,000, and 2,600 denser boards of up to 60, 300 and 600 boxes) and the whole scene of the 13 perf boards (the `make perf-board` boards and the app's grids of 347, 1,000 and 3,380 shapes), at `9634c4d` and at the head: 334 MB, byte-identical. A new unit test, `a_lookup_finds_each_touching_box_once_at_its_cost`, checks the lookup against its definition (each touching box once, the cost in cells and entries, the budget emptied when short) for boxes kept in the array, in the map, too large for cells, and added later.

Scene build before and after, release, the fixed-width measurer, median of seven, best of three rounds (ms):

| Board | Performance core | Efficiency core |
|---|---|---|
| App grid, 1,000 shapes | 5.50 → 3.72 | 7.25 → 4.66 |
| App grid, 347 / 3,380 shapes | 1.78 → 1.24 / 26.3 → 18.4 | 2.36 → 1.59 / 35.3 → 24.4 |
| Neighbour grid, 1,000 / 5,100 shapes | 2.89 → 2.09 / 21.6 → 16.7 | 3.84 → 2.70 / 29.6 → 21.7 |
| Every connector diagonal, 1,000 | 10.2 → 8.2 | 13.0 → 10.3 |
| Long random, 1,000 / 5,100 | 68.5 → 52.2 / 425 → 279 | 94.1 → 65.9 / 577 → 372 |
| Fan-in, 1,000 / 5,100 | 50.6 → 22.7 / 196 → 93.5 | 65.7 → 29.9 / 253 → 127 |
| Stacked, 1,000 / 5,100 | 45.9 → 15.7 / 106 → 101 | 59.3 → 21.5 / 146 → 138 |

`make perf-board` (unpinned, before → after): the 1,000-shape grid 3.1 → 2.3 ms, 10,072 elements 24.9 → 18.6 ms; at 1,000 shapes diagonal 11.7 → 9.3, long random 70.0 → 55.1, fan-in 51.9 → 25.2, stacked 46.6 → 16.5; at about 10,200 elements 447 → 292, 199 → 98, 112 → 104. In the app (release, `perf.board`, the grid board, T3's measurement, X11 tiny-skia), the scene half went from 7.3 to 7.4 ms to 5.3 to 6.5 pinned to the performance cores, from 9.2 to 9.5 to 6.4 to 7.1 pinned to the efficiency cores, and from 7.7 to 10.7 to 5.2 to 7.2 unpinned.

*T3, the release edit measurement.* `test_board_perf.py` now makes ten theme switches back to back, polling every millisecond for the draw that rebuilt the geometry (a request is answered between draws, so that draw's time is in the ring by then) instead of sleeping 0.4 s, leaves the first three out and takes the median of seven; it still asserts one scene build per change and writes `edit_micros`, `edit_scene_micros`, `edit_draw_micros` and `edit_warm_up_micros`. The first change after the idle checks is the cold one, about twice the rest (15.1 ms, then 6.4). Mutation: with a 10 ms sleep added to `scene::build` the release run fails on its edit budget on both renderers (tiny-skia 18.5 to 19.1 ms over four runs, wgpu 21.7 to 30.0 over five, 21.7 to 22.2 on a quiet machine); without it, it passes. The release runs of the `ef067e7` binary, with this test (its ceiling constants changed between runs; its measurement did not), unpinned or with every process pinned with `taskset` to the performance cores (0 to 5), the efficiency cores (6 to 13) or the low-power efficiency cores (14 and 15), medians in ms:

| X11 | Unpinned | Performance cores | Efficiency cores | Low-power cores |
|---|---|---|---|---|
| tiny-skia: edit (scene, draw) | 6.2 to 8.4 (5.2 to 7.0, 1.1 to 1.3), 12 runs | 6.4 to 7.3 (5.3 to 6.0, 1.1 to 1.3), 3 runs | 7.8 to 8.7 (6.4 to 7.1, 1.4 to 1.5), 3 runs | 27.3 to 27.5 (22.0, 5.3 to 5.5), 3 runs |
| wgpu, lavapipe: edit (scene, draw) | 11.3 to 15.9 (8.5 to 11.8, 2.7 to 3.6), 17 runs | 9.1 to 10.3 (7.0 to 7.8, 2.1 to 2.5), 3 runs | 11.6 to 17.4 (8.8 to 12.9, 2.8 to 4.4), 8 runs | 30.2 to 30.6 (23.0 to 23.2, 7.2), 2 runs; the third failed its 1,000-element open (183 ms) first |

The machine is shared: during the run with the 17.4 ms median another process held about four cores, and the six wgpu runs made after waiting for no process to use more than 30 % of a core gave 11.6 to 12.4 ms. The rest of the release run passes its budgets on both renderers (unpinned): open 18 to 26 ms (tiny-skia) and 41 to 69 ms (wgpu) at 1,002 elements, 106 to 147 and 152 to 197 ms at 10,020; idle view 0.09 to 0.17 ms with zero parses, scene builds and geometry builds; pan and zoom frames 2.7 to 4.2 ms on tiny-skia, and on wgpu draws within 8 ms (frames 7.3 to 10.2). The ceiling, by the owner's rule: on tiny-skia every median is under 12 ms with more than a quarter to spare, so 12 ms stays; on wgpu, 16 ms (the deviation below; 20 ms until the fix step). Both are written into §6.7's row and paragraph with the reason, and the debug ceilings are unchanged.

*Deviations.*
- The ceiling is set per renderer, 12 ms on tiny-skia and 16 ms on wgpu, where the brief names one `EDIT`. One value over both would be at least 16 ms, which most doublings of the tiny-skia cost (6 to 9 ms, to 12 to 17) would pass, against the brief's requirement that a real doubling fail; §6.7's row is already per renderer, and lavapipe's threads compete with the scene build for the processor, which a hardware GPU's do not.
- The wgpu ceiling is 16 ms, set from quiet-machine medians only. *Corrected in the fix step:* it was 20 ms, the rule applied to a worst median of 17.4 ms taken while another process held about four cores and then lowered below the 10 ms sleep's 21.7 ms; the review showed a doubled scene half passing it unpinned (18.7 ms). The quiet medians of the implementation, the review and the fix step are 8.9 to 12.9 ms, and 16 ms is about a quarter over the worst; §6.7 has the mutants it catches and the one place (the performance cores) where a doubling still passes. A loaded run is not used to set a ceiling on either renderer, and the release run is documented as needing a quiet machine.
- The low-power efficiency cores (14 and 15, at most 2.5 GHz) are a third core class the brief does not name. Pinning the whole run there, the X server and lavapipe included, squeezes it onto two slow cores (open 53 to 183 ms at 1,000 elements, against 18 to 69 elsewhere); the runs are recorded above but do not set the ceiling, which from them would be 39 ms and would pass a doubling.
- T2's last saving is in `scene.rs`, not the router: the lookups of connector ends that R1 added around `route_all` cost about as much as routing the grid did, and the change draws nothing differently.

*Gates* (at `f3166ac` unless named).
- `make check` passes at each commit; at the head 2,845 Rust tests passed, 0 failed, 30 ignored, and 294 Python units.
- `cargo clippy --locked --target aarch64-apple-darwin --workspace --all-targets --features smoke-test -- -D warnings`: clean at `9634c4d` and at `f3166ac`.
- `make fuzz-board`: 7 passed. `make perf-board`: passed, with the numbers above.
- The X11 tiny-skia suite on both engines: 264 passed, 2 skipped. The board files on headless Weston: 19 passed, 1 skipped, 4 deselected with tiny-skia and with wgpu; on X11 wgpu (lavapipe): 23 passed, 1 skipped. The debug edit medians there: 28 ms (X11 tiny-skia), 29 (Weston tiny-skia), 35 (Weston wgpu), 43 (X11 wgpu), against 100.
- The release performance run, as above, on X11 tiny-skia and X11 wgpu: every median unpinned and pinned to the performance and efficiency cores is under the committed ceilings, and the 12 runs made with the committed test (2 on tiny-skia, 10 on wgpu, 5 of them pinned to the efficiency cores) passed whole; pinned to the low-power cores it fails, as noted.
- T2's three proofs, as above.
- Not run: the Mac step (the orchestrator's), Wayland in release.

*Fix step* (from the review at `dd14794`). Both findings reproduced.
- *Major, the wgpu release ceiling too loose: fixed* (`5b44364`). Reproduced with a release build in which `BoardView::build` runs its scene half (the measurer's sweep, `scene::build` and the layer split) twice: on X11 wgpu it measured 19.8 ms unpinned, passing the 20 ms ceiling (20.0 and 20.9 in two more runs, just over it), and 17.7 and 19.1 ms on the efficiency cores, both passing; the unmodified build measured 12.0 to 12.9 ms unpinned and on the efficiency cores. `test_board_perf.py`'s wgpu `EDIT` is now 16 ms, by the owner's rule over quiet-machine medians only (8.9 to 12.9 ms across the implementation, the review and this step; 16 ms is about a quarter over the worst). Against it, on a machine more than 90 % idle for five seconds before each run: the unmodified build passes every run (unpinned 11.9 to 12.0, efficiency cores 11.7 to 12.0, performance cores 8.9 to 10.1 ms); the doubled scene half fails unpinned (18.5) and on the efficiency cores (17.7); `scene::build` alone run twice fails unpinned (16.8, 17.0) and on the efficiency cores (16.4, 16.6); pinned to the performance cores both pass (13.9 and 12.7), as §6.7 now says. The test also writes the 1-minute, 5-minute and 15-minute load averages into `measurements.json`, so a run beside other work can be recognised. The regression test is the release run of `test_board_perf.py` itself, shown above to fail on those mutants; no smaller test can hold a timing ceiling. §6.7's row and paragraph, the Phase 1 end correction and the second deviation above now give 16 ms and the quiet-machine basis.
- *Minor, the Phase 1 end record contradicting itself: fixed* (the record commit). The tag bullet and the final gates' release-run line each carry a dated `boards-p1-tune` note: the release run passes since this chain, so the tag waits on the owner's acceptance and the Mac step.
- *Noted, not a finding:* on tiny-skia the same doubled scene half passes 12 ms unpinned (10.4 and 10.9 ms) and fails it on the efficiency cores (15.8) and, in the review, on the performance cores (12.3). The owner's rule keeps 12 ms there, every quiet median being under 8.7, and the tightest whole number the rule allows otherwise, 11 ms, would still pass 10.9; the 10 ms sleep fails on both renderers. §6.7 records it.

*Review.*
- Counts: 0 blockers, 1 major (the wgpu release ceiling of 20 ms passed a doubled scene build), 1 minor (the Phase 1 end tag bullet and final gates line still said the release run fails).
- Fix commits: `5b44364` `test(boards): hold the wgpu release edit ceiling at 16 ms`, and this record.
- Not fixed: nothing. The tiny-skia gap above is the owner's rule working as written, not a defect; the 8 ms row stays Phase 2's, as before.

*Fix step gates* (at `5b44364`, the record commit changes `discovery/` only): `make check` 2,845 Rust tests passed, 0 failed, 30 ignored, and 294 Python units; `ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11` 264 passed, 2 skipped; the board files on headless Weston 19 passed, 1 skipped, 4 deselected with tiny-skia and with wgpu, and on X11 wgpu (lavapipe) 23 passed, 1 skipped; `make fuzz-board` 7 passed; `make perf-board` passed (the 1,000-shape grid 2.3 ms, 10,072 elements parse 49.6 and scene 18.5 ms; at 1,000 shapes 9.2 to 53.2 ms, at about 10,200 elements 97.2 to 283.6 ms); the release run as above, and on X11 tiny-skia 6.4 ms. Nothing draws differently (a test constant, the load average and text), so no scene was captured; the review's 40 scenes stand. The aarch64 cross-lint and the strict docs build do not apply (no file they cover changed). Not run: the Mac step.

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
