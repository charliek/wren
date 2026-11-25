# Tasks: build plan

Status: **planned; nothing built.** Written 2026-11-24 against `main` at `f42bf57`, from the design in `discovery/tasks.md` (§2 items 1 to 29 are the owner's decisions; §6.1 the primitives), and revised the same day after a planning review by three reviewers (Codex `gpt-6.2-astra`, CodeRabbit, Gemini Neon). This is the contract for two long autonomous runs: **Run A** builds ring 0 (the editor) and **Run B** ring 1 (the summary view). Kanban (ring 2) is planned separately after Run B.

How to use this document:
- A fresh session reads `CLAUDE.md`, `discovery/tasks.md` and this file, then the work item it is on. The design note decides *what*; this file decides *in what order, tested how, by which model, and when an item is done*. `.claude/tasks-notes/HANDOFF.md` (git-ignored) says how to run the loop.
- Line references come from a code survey and the planning review on 2026-11-24. They drift as work lands; find the named function.
- Where the code must differ from the design, the run records the deviation in §10 and the orchestrator folds accepted ones into `discovery/tasks.md` at the run's end.
- The repository is public: no host names, user names or private links in committed files.

## 1. Rules that hold for every item

- **Branch.** Run A works on the branch `tasks-ring0` in the worktree `.claude/worktrees/tasks-ring0`; Run B on `tasks-ring1` in `.claude/worktrees/tasks-ring1`. Tags are `tasks-r0` and `tasks-r1` (different names from the branches). Small verified commits; every commit passes `make check`. All commands for an item run with the worktree as the working directory.
- **Merge.** One pull request per run. The pull request is the third-party build: watched with `/git-commands:watch-pr`, then merged by the run with `/git-commands:merge-pr` as a **merge commit** (not a squash, as PRs #6, #8 and #9) once the run-end checklist (§8) passes on the pull request's final head. The tag goes on the merge commit.
- **The `tasks` gate.** A `tasks` setting, off by default, **restart-only** like `boards` (the macOS menu bar is built once at launch, so a live gate could not reach it). With it off, Wren behaves exactly as today, which includes the shared classifier and editing rules:
  - the box accepts only ` `, `x` and `X`; `[/]`, `[-]` and other characters are not tasks;
  - `markdown.toggle_task` writes no `✅` or `❌` stamp;
  - the highlighter, spell check's prose ranges and search previews are unchanged;
  - the new default keys are unbound (they type nothing new and do nothing);
  - no task command appears in the palette or menus, no `.wren/tasks.toml` is created, and no task index is built.
  The way to carry this is one options value passed to `wren-markdown` (for example `Options { tasks: Option<TaskOptions { today }> }`, `None` when off), to the highlighter settings and to the host. A **baseline fixture** made in 0.1 pins today's behaviour and is re-run with `tasks = off` at every milestone end.
- **Engines.** With `tasks = on`:

  | Feature | Classic engine | Advanced engine |
  |---|---|---|
  | Task colours, overdue colour, done and cancelled dimming | yes | yes |
  | `markdown.toggle_task` with stamps | yes | yes |
  | Palette commands that only rewrite the line (cycle, set priority per level, raise, lower, clear due) | yes | yes |
  | Status-bar task line | yes | yes |
  | Task form (`task.edit`, a host dialog) | yes | yes |
  | Right-click menus, box click, calendar popover, auto-suggest, `task.set_status`/`task.set_priority`/`task.set_due` (they open popovers) | no (commands disabled) | yes |

- **One clock.** `App::today()` is the only source of today's date, in local time. Under the `smoke-test` feature, `WREN_TEST_TODAY=YYYY-MM-DD` and a `tasks.set_today` request pin it. Every task test, fixture and scene pins **Thursday 19 November 2026** (the design page's day) and writes its dates relative to it. The highlighter, footer, calendar, suggestions and the summary view all refresh when the local date changes while Wren is open, tested by moving the pinned date.
- **Primitives before clients.** Each primitive of `discovery/tasks.md` §6.1 lands in its own commit with **no behaviour change**, proven by the existing tests (spell check's especially), before a task feature uses it.
- **No parallel items.** Items run one at a time in the run's worktree (cold builds in extra worktrees cost more than they save). `make check` and a smoke run never overlap in the same worktree (`cargo test` rebuilds `target/debug/wren`). A Codex review is read-only and may run alongside either.
- **What "done" means for an item:** the item's acceptance list in §4 or §5 is met; `make check` passes; the item's smoke files pass in the matrix of §7; every scene the item owns is captured and looked at; the Codex review's findings are fixed or answered; the item's commit is pushed and its RUNLOG entry written.

## 2. Seams (from the survey and the planning review)

1. **The box is three bytes in many places.** `block::task_box` and its caller (`crates/wren-markdown/src/block.rs:980-1007`, `pos + 3`); `insert_above` and `task_text` (`edit.rs` ~449, ~955); the marker-only check (`edit.rs` ~251); `edit/delete.rs` (`may_end_markup` ~41-50, which lists `x`/`X` but not `/`; `markup` ~109, `task.start + 3`); `edit/task.rs` (~64, ~97-103, replaces `at..at + 1`); `crates/wren-core/src/index.rs` (the marker-only check ~1144-1150 and the preview stripper ~969); the highlighter's `done` branch (`crates/wren-editor/src/highlighter.rs:412-486`). `Task` is `{ done: bool, start }` (`block.rs:237`).
2. **Inline tokens are decided in three places:** the highlighter's `Cells`, and `wren-spell/src/words.rs:53-63` (`@` and `#tag` chunk skipping, with its own unit tests, e.g. `rule_1_skips_whole_chunks`). `wren-spell` stays independent; its rules do not move.
3. **The spell menu** (`crates/wren-editor/src/proof.rs:202-279` `OpenMenu`, `proof/menu.rs` `MenuOverlay`, `widget.rs:871-914` placement, `keys.rs:64-97` routing) closes on every key but Up, Down, Enter and Escape, closes on every edit (`lib.rs` `perform`/`apply`/`intent` set `menu = None`), and closes on IME preedit (`widget.rs:779-782`). The widget owns IME (`widget.rs:932-954`). The misspelling hit test is `misspelling_under` (`widget.rs:919`); `:1642` only decodes the right-click.
4. **The key path has no payload:** `Intent` is a `Copy` enum without data; `KeyRoute` is `Host | Intent | Suggest | Pass` (`keys.rs:13-31`); the harness hard-codes the spell menu in `editor_key` (`crates/wren-iced/src/app/test_requests.rs:812-846`). The invariant `editor_scoped_commands_are_exactly_those_with_an_editor_route` (`commands.rs:1730`) moves with new routes.
5. **Menus and keymap invariants:** the macOS menu table may hold App and Board commands only (`crates/wren-iced/src/menu.rs:241-249`), and the board-kind test at `menu.rs:267`; the menu is installed once (`macos_menu.rs:45-52`). The default keymap is frozen by `frozen_base` and its test (`crates/wren-core/src/keybind.rs:578-812`).
6. **Themes:** built-in colours live in `crates/wren-core/src/theme.rs` (`Colors`, contrast tests ~646-772); `crates/wren-iced/src/theme.rs` maps them to the editor `Palette`, which has no date, owner or overdue entries yet.
7. **No emoji font is loaded:** `crates/wren-iced/src/fonts.rs` bundles Inter and JetBrains Mono and excludes emoji fonts from the monospace picker; emoji come from system fallback. CI's smoke job installs `fonts-dejavu-core` only (`.github/workflows/ci.yml:76-79`); the Rust job installs none; the editor's Rust fixtures load JetBrains Mono only (`crates/wren-editor/src/fixtures.rs:77`).
8. **Writing notes:** `Op::WriteNote` refuses all but the active document (`crates/wren-core/src/ops.rs:684-694`); `FsStore::write(.., Expected::Bytes)` is the compare-and-swap (`fs_store.rs:435`), with a documented final race against non-cooperating writers. After every op, `App::process_core` calls `sync_editor` (`crates/wren-iced/src/app/workspace.rs:414-416, 843-933`), which calls `replace_text` (clearing undo) when the Session's text differs from the editor's. `EditorView::apply(&[Replace], cursor)` keeps undo.
9. **The pane and history hold notes only:** `Surface { Text, Board, Raw }` (`app/board.rs:26`), and `sync_editor` reads any non-Text surface as a board (`workspace.rs:862`); `Session.history: Vec<NoteId>` (`ops.rs:249`), projected into `.wren/workspace.toml`, where older builds read `[navigation]` leniently (`workspace_state.rs:48-70`).
10. **Rows in the columns:** a right-click on a tree row opens the item-actions palette (`Mode::ItemActions`, `commands.rs:1074-1079`; `item_commands`, `commands.rs:1546`), not a native menu. The Notes column has a single selected note.
11. **Index update points** (for the task index): `Index::ingest` (`index.rs:394`), `ingest_scanned` (`:434`), each op in `Session::apply` (`ops.rs:536-580`), leaving a note (`:973`), and reconcile (`ops.rs:1623-1650`, `reconcile.rs:95`). `Index::content` can return extracted board text.
12. **Scenes and lanes:** `tools/capture_scenes.py --only` accepts `markdown`, `spell` and `boards` (`:1011`) and defaults to the classic engine; `tools/wrentest/test_editor.py:28` defaults `WRENTEST_ENGINE` to classic locally; CI sets `classic,advanced` on four lanes, X11 and Wayland × wgpu and tiny-skia (`ci.yml:54-68`); `tools/wrentest_unit/test_ci_contract.py` pins the CI matrix.

## 3. Order

Run A: 0.1 → 0.2 → 0.3 → 1.1 → 1.2 → 1.3 → 1.4 → 2.1 → 2.2 → 2.3 → 3.1 → 3.2 → 3.3 → 3.4 → 4.1 → 5.1 → 6.1 → 7.1 → 7.2 → 7.3.
Run B: 8.1 → 9.1 → 10.1 → 10.2 → 10.3 → 10.4 → 10.5 → 11.1 → 11.2.

## 4. Run A: ring 0, the editor

### M0. Foundations for the run

- **0.1 The gate, the clock and the harness** (Opus; sol).
  - `Settings.tasks` (restart-only), the template block and `docs/reference/configuration.md` row; `CommandId::tasks()` filtering the palette, `command_enabled`, the macOS menu builder and the harness, as `boards()` does; the new default keys unbound when off.
  - `App::today()` with the `WREN_TEST_TODAY` override and the `tasks.set_today` request; a day-change refresh hook.
  - The `tasks.*` request family in `crates/wren-iced/src/app/tasks_harness.rs` (precedent `board_harness.rs`): `tasks.get` (the caret's task line model), `tasks.set_today`, later `tasks.popover` and `tasks.view`.
  - `WRENTEST_TASKS=on|off` in `tools/wrentest/ui.py`; task tests write their own settings (`tasks = on`, the engine they test).
  - `capture_scenes.py --only tasks` with a tasks fixture workspace, the advanced engine selected explicitly, the pinned date, and output paths that name engine, renderer, display and font setup.
  - The **baseline fixture**: a note of task lines (`[ ]`, `[x]`, `[X]`, `[/]`, `[-]`, `[a]`, quoted, numbered, tab after the box) with a test that records today's behaviour for Enter, Backspace, toggle, highlighting, previews and spelling; it must stay green with `tasks = off` to the end of Run B.
  - **Acceptance:** settings parse and template tests; with `tasks = off`, no task command in the palette, the new keys do nothing (both platforms' keymaps), and the baseline passes; the harness requests answer; `capture_scenes.py --only tasks` runs; the pinned date reaches `App::today()`.
- **0.2 CI and lanes** (Sonnet; sol). Add `fonts-noto-color-emoji` to the smoke lanes; add `WRENTEST_TASKS=on` to one CI lane and update `tools/wrentest_unit/test_ci_contract.py`; a `make smoke-tasks` convenience target that runs the task files in the §7 matrix. **Acceptance:** CI green on the branch with the new lane; the contract test passes.
- **0.3 Emoji spike** (Opus; sol). A fixture with `📅 ✅ ❌ 🔺 ⏫ 🔼 🔽 ⏬`, each with and without U+FE0F, in task lines that wrap. For each glyph × VS16 form × {X11 tiny-skia, X11 wgpu, Wayland, macOS Metal} × {Noto Color Emoji present, absent (a fontconfig file without it)} record in `.claude/tasks-notes/spike-emoji.md`, with scenes:
  - **Go** needs, with the emoji font present: a recognisable glyph (no tofu box, not blank); caret moves over it in one step, Backspace removes it in one press, and selection covers it, on both engines; a wrapped row's hanging indent stays aligned; hit-testing matches the drawn position.
  - Without the font, a missing glyph is expected and recorded, not a no-go.
  - Rust tests assert only font-independent facts (grapheme boundaries, one-press Backspace, caret steps), so they pass on Linux and macOS CI alike.
  - **A no-go stops the run** only after checking the font setup and ruling out a fix that keeps the decided syntax.
  - macOS Metal: if the Mac preflight (HANDOFF) fails, record "Metal pending" and continue; pending Mac checks block only the merge.

### M1. The task line model (`wren-markdown`)

- **1.1 Status in the box** (Fable; astra).
  - `Task { start, end, symbol: char, status: StatusType }`, where the box is `start..end` in bytes; every `+ 3` and `at..at + 1` in seam 1 uses the range instead. A box symbol is **one Unicode scalar value** other than `]` (`[✔️]`, two scalars, is not a task). The status table is data (symbol, name, type, next symbol); four MVP rows; an unknown symbol reads as open.
  - The extended box applies only with `tasks = on` (§1); with it off, the classifier accepts exactly what it accepts today.
  - Move every consumer in seam 1, including `may_end_markup`, both `index.rs` functions and the outline fixture. Highlighting per status: open and doing use the accent on the box (doing's `/` distinguishable by the symbol itself), done and cancelled dim and strike the description, as the design page's prototype shows.
  - `ToggleTask` over a selection: if any line is open or doing, every task becomes done; otherwise every task becomes open (cancelled lines included).
  - **Acceptance:** classifier tables for every symbol class; `[界]`, `[é]`, `[✓]`, `[X]`, `[✔️]`, `[]]`, `[[]`, `- [1] x`, `- [a]: http://u` (a link reference definition), quoted, numbered and tab-separated tasks survive Enter, insert-above, Backspace, toggle, highlighting and previews on both engines, with `tasks` on and off; the baseline fixture still passes with `tasks = off`.
- **1.2 Civil dates and date words** (Opus; sol). A small civil-date type with no new dependency (owned here, used by everything after), and `date::parse(text, today)`: `today`, `tomorrow`, weekday names (the next after today), `next <day>`, `next week`, `+Nd`, `+Nw`, `+Nm`, ISO; English; weeks start Monday; slash dates refused. **Acceptance:** table tests against the pinned day, month and year ends, leap days, invalid ISO dates kept as invalid.
- **1.3 Tokens** (Fable; astra). A `task` module: parse a task line into status, description range, owners (with byte ranges), due, done and cancelled stamps, priority, `🆔`, the `^block-id`, and unknown tokens kept in place; read backwards from the end; `📆`, `🗓` and `[due:: …]` read; VS16 optional; an invalid date kept and flagged; several priority glyphs → the last wins.
  - **Writing.** A newly written token goes in Obsidian Tasks' order (description, `🆔`, priority, dates with due, cancelled and done last, then the `^block-id`), glyphs without U+FE0F. **Changing an existing token replaces it in place**; Wren never reorders a hand-ordered line (design Q10), and `@owner` never moves.
  - **Acceptance:** exact before/after bytes for hand-ordered lines with `@owner`, unknown fields, duplicate priorities, VS16, a due alias and a `^block-id`; a property corpus run in `make check` and an ignored `fuzz_task_line` (parse then write is stable; unknown tokens survive byte for byte) run at the milestone end.
- **1.4 The line edit primitive** (Opus; sol). `task::edit(line, change, today) -> EditPlan`, carrying the line it expects. Changes: toggle done, cycle, set status, set or clear due, set, raise or lower priority, add or remove an owner, set the description, and a **compound change** (several of these at once, for the form) as one replacement and one undo step. Done and cancelled stamps follow status changes; typing `x` by hand adds no stamp (Q11); Enter on a done task continues with `[ ]` and copies no stamp. **Acceptance:** `edit/task.rs`-style tests (`|` caret notation) for each change, the compound case, the precondition failing on a changed line, and one undo step per change.

### M2. Spans and reading tasks

- **2.1 The span layer, spelling first** (Fable; astra). Typed spans per line in `wren-markdown` (box, due, stamp, priority, owner, `🆔`, tag). The highlighter reads its existing token decisions through them, and spell check's prose ranges come from them, **with no behaviour change**: every spell and highlighter test passes unchanged, `wren-spell` untouched. Specify how classifier spans and the proofreader's dynamic misspelling ranges meet in one hit test without `wren-markdown` depending on `wren-spell`. Commit alone.
- **2.2 Task colours** (Opus; sol). With `tasks = on`: date, overdue (reusing `danger`) and owner entries in `wren-core`'s `Colors` for every built-in theme, extended contrast tests (against the editor background, current line and selection), mapped through `crates/wren-iced/src/theme.rs`; task tokens excluded from spell check's prose; today from `App::today()` through the highlighter settings, re-highlighting on a day change. Classic: colours only. **Acceptance:** span-to-colour tests on both engines; contrast tests; a day-change test; scenes of task lines in three themes, both engines.
- **2.3 The status-bar line** (Opus; sol). "Doing · due Mon 23 Nov, in 4 days · ⏫ High · @alice.chen" for the caret's task in `editor_footer` (`workspace_view.rs:1060`); overdue in the danger colour; it gives way to save or error status in a narrow window. **Acceptance:** formatting tests; `tasks.get` snapshots; a functional caret-move test; a narrow-footer scene.

### M3. The popover, providers and menus

- **3.1 The anchored popover, spelling first** (Fable; astra). Grow the spell menu into a popover primitive anchored to a range or the caret, with two **modes** defined now:
  - **Menu:** Up, Down and Enter, optional single-key accelerators, closes on any other key; used by spelling, status, priority and the task menu.
  - **Suggest:** non-modal; Up, Down, Enter and Tab captured, Esc closes and suppresses for the line, ordinary typing and Backspace pass through to the editor and the popover follows; used by 5.1.
  It owns placement, key routing, the batch-consistency rule (keys arriving in the batch that opens it), Esc and close rules, and exposes the anchor rectangle to the host (for 4.1's host-drawn calendar). Move the spelling menu onto it with **no behaviour change**: every spell test, Rust and functional, passes unchanged. Commit alone.
- **3.2 Context-menu providers, spelling first** (Fable; astra). A host-supplied provider trait (as `Proofreader` is), asked in order for the span under the pointer. Land it with spelling as the only real provider plus a test provider proving the order. No task menu yet. **Acceptance:** ordering tests; right-click hit tests on spans and misspellings; spell tests unchanged.
- **3.3 Payload routes and line commands** (Opus; sol). Payload-carrying `KeyRoute`/message variants and the harness's `editor_key` generalised past the spell menu. Commands: `task.cycle_status`, `task.priority.highest|high|medium|low|lowest|none` (one per level, design Q7), `task.priority.raise`, `task.priority.lower`, `task.clear_due`; a Tasks palette group; `markdown.toggle_task` with stamps (§2 item 20). The frozen keymap test is updated deliberately for the new defaults. Task commands stay out of the macOS menu table (Editor scope; seam 5). **Acceptance:** command-table invariants; each command on the engines §1 allows; with `tasks = off`, none appears and the keys do nothing; the macOS clippy line.
- **3.4 Task menus and the box click** (Opus; sol). The task provider: right-click on a task line opens the task menu (status rows, Set due date, Priority, Edit task); on the box, the status menu (Space, `/`, `x`, `-`); on the priority glyph, the priority menu (H, M, L, ⇧H, ⇧L, N); on the date, the calendar (wired in 4.1; until then the task menu). `task.set_status` and `task.set_priority` open those menus. A primary click inside the box's `[…]` cycles open → doing → done → open (cancelled → open) as one undo step; a drag starting on the box, a double-click and a Shift-click do not cycle; nothing happens on classic or with `tasks = off`. On a misspelled word, spelling wins. **Acceptance:** functional tests per menu and accelerator; native X11 clicks on the box and right-clicks on each token; scenes of each menu.

### M4. The calendar

- **4.1 Calendar** (Opus; astra, for its input and focus handling). The calendar model (month grid, selection, typed field, keys) lives in `wren-markdown` beside the date parser. The view is a **host-drawn anchored overlay** in `wren-iced` with a real iced text input (native IME, selection, paste), positioned from the popover's anchor rectangle, so the same widget serves the editor, the form (6.1) and Run B. Opening hands keyboard focus to the field; closing returns it to the editor with caret and selection unchanged. Keys: ↑↓ a week, ←→ a day while the field is empty, PgUp/PgDn a month, ↵ writes, Clear removes, Esc cancels. `task.set_due` on ⌘D / Ctrl+D; opens on the task's date or today, at the date token or the caret. **Acceptance:** model tests; widget tests for opening and typing in one batch, preedit → commit, Enter during composition, Esc, blur, scroll and a stale anchor; exactly one action per key; cancelling leaves note bytes, caret and selection unchanged; functional tests typing `fri`, `+2w` and an invalid word; native X11 ⌘D/Ctrl+D; scenes of the empty, typed and invalid states; macOS Metal check of the overlay and Command keys.

### M5. Completion

- **5.1 Completion sources** (Opus; sol). A host-supplied source trait on the popover's Suggest mode. Triggers: two letters at a word start on a task line, never the description's first word; `@` alone opens owners; only what the line lacks; ↵ or Tab accepts; Esc hides it for the line; Tab with no popover still indents; a `tasks-suggest` setting turns it off. Sources: date words, priority words, owners. Owners: task lines in `Index::content`, case-insensitive, most common spelling, ranked by note count and recency, cached by index revision **and** workspace identity, off the UI thread or within a measured budget. An `@` typed after the tokens joins the description. Keys in the batch that opens the popover follow 3.1's rule. **Acceptance:** trigger table; owner ranking, rename and removal; a workspace switch drops the cache; a native X11 test typing `du⏎` at xdotool speed; a table of Enter after `to`, `hi`, `lo`, `me`, `fr`, `sa` at line end; IME composition never opens it; a perf test of the owner scan on the `perf-search` workload.

### M6. The task form

- **6.1 Create or edit task** (Opus; sol). A modal dialog in `wren-iced` (precedent `palette.rs:1606`): description, status, due (the 4.1 calendar widget), priority, owners with completion, and a "Writes" line showing the Markdown. `task.edit` on ⇧⌘↵ / Ctrl+Shift+Enter; on a blank line it makes a task. Saving is one compound line edit (1.4) applied through `EditorView::apply` as one undo step, refused if the line changed underneath. Works on both engines. **Acceptance:** functional create, edit and refused-stale tests; keyboard-only path (Tab, ↵ saves, Esc cancels, focus returns to the caret); the calendar inside the form returns focus to the form; scenes of new and edit.

### M7. Ring 0 finish

- **7.1 Docs and record** (Sonnet; sol). `docs/reference/tasks.md` (new) and its `zensical.toml` nav entry; `docs/getting-started/keybindings.md`; `docs/reference/configuration.md` (`tasks`, `tasks-suggest`, `tasks-me`); `docs/reference/spell-check.md`; `docs/development/testing.md` (the harness requests and lanes); the module list in `CLAUDE.md`; `CHANGELOG.md` under Unreleased; `zensical build --strict`.
- **7.2 Scenes and the full gate** (Sonnet; sol for any scene code). Every ring 0 scene in §7 captured; the full §7 matrix; macOS Metal scenes; deviations folded into `discovery/tasks.md`.
- **7.3 Ring review** (Fable read-only judge; one Codex `gpt-6.2-astra` review of the whole branch since its merge base). Fix what matters (each fix through the normal loop), then the run-end checklist (§8), pull request, merge, tag. **Stop and report.**

## 5. Run B: ring 1, the summary view

Starts after the owner's hands-on pass with ring 0 (their notes go in the Run B kickoff prompt), from `main` with `tasks-r0` merged.

### M8. The task index

- **8.1 Index** (Fable; astra). In `wren-core` beside `index.rs`: per-task records (path, line, the **exact source line**, status, due, priority, owners, stamps, description, nearest heading), never note content; boards excluded. Updated at every point in seam 11. Front matter keys `tasks: off` and `TP_ignore_this_file: true` read without a YAML library; folder exclusions and per-view filters from `.wren/tasks.toml` (precedent `workspace_state.rs`, including path remapping on rename and delete); `.wren/tasks.toml` never created while `tasks = off`. Owner completion switches to the index.
  - **Acceptance** (in `crates/wren-core/tests/`): initial scan, active draft, synchronous refresh, background scan completion, rename, delete and read failure; search-content eviction leaves task records intact; a late scan cannot overwrite a newer draft or task write; an unchanged tick reparses nothing; `make perf-tasks` on the `perf-search` workload (5,000 notes, 25 MB) with a recorded budget for the full build and one-note update.

### M9. Writing tasks from outside the editor

- **9.1 One writer per note** (Fable; astra).
  - **The open note:** the App applies the line edit to the editor through `EditorView::apply`, handled **before** `sync_editor` compares texts, keeping the caret and selection (unchanged on another line, mapped on the same line) and the scroll; the text then reaches the Session by the existing `EditDraft`. The editor is the only writer of the open note.
  - **A closed note:** `Op::EditTaskLine { note, line, expect, replace }`: settle the active document first; read; find the line equal to `expect` (compared without its line ending, BOM handled) at `line`, or the one unique task line equal to it elsewhere that is still a task in Markdown context (not in a fence or front matter); compare-and-swap write; `record_file`; update both indexes; emit `NoteSaved`. On a moved-and-ambiguous line or a compare-and-swap conflict, refuse with "This task changed; the list was refreshed" and refresh; **no conflict copy** (conflict copies remain only for settling a dirty open document). Report the store's actual publication outcome.
  - **Front matter:** a variant for Hide tasks in this note: add or remove `tasks: off`, creating or removing the front matter block (CRLF, BOM, a leading `---` that is a thematic break).
  - **Undo from the view:** the inverse edit through the same path with the same precondition.
  - **Acceptance:** an App-level test: type in note A, edit A through the view, edit closed note B, undo in the view, return to A and verify its undo history, selection, draft and disk bytes; plus open, closed, moved line, duplicate lines, a stale form, an external edit before Undo, a second action on a row held until R, a failed active-note flush, the front matter cases.

### M10. The summary view

- **10.1 The surface** (Fable; astra). `Surface::Tasks` in the editor pane, independent of the document underneath (the note stays `Session.document`); `sync_editor`, `ensure_editor_ready`, `App::text_changed` and `App::surface()` guarded so the hidden editor never writes (test like `the_editor_behind_a_board_never_writes_it`, `app/board.rs:1406`). History entries for the view are **session-local** and never written to `.wren/workspace.toml` (recorded as a decision in `discovery/tasks.md` §9). **Acceptance:** note → view → board → Back/Forward; a dirty-draft failure; restart (the last note reopens); `tasks = off` after a restart; the v1 history projection unchanged.
- **10.2 Entry points** (Opus; sol). Header counts in `pane_header` (open and doing, red when overdue), the `task-counts` setting and `view.toggle_task_counts`; `tasks.show` on ⇧⌘T / Alt+Shift+T with the scope rules (Notes column or editor → the group); Show tasks, Hide tasks from this group and Hide tasks in this note in `item_commands` (seam 10), each with Undo; the "tasks off" mark in `notes_pane`; `tasks.show` and `view.toggle_task_counts` in the macOS menu (App scope). **Acceptance:** functional tests per entry point; counts on and off; native X11 Alt+Shift+T; scenes.
- **10.3 The list** (Opus; sol). Scope chip (workspace, section, group, note); status chips that narrow (none pressed = all); owner (Anyone, Me = no owner or a `tasks-me` name, a person); due-bucket and priority filters; grouping (note, due, priority, owner; a multi-owner task under each); sort (open before finished, overdue, priority, due); hide done after 14 days; rows with box, description, priority, due (an invalid date marked), source note and nearest heading; open at line; filters remembered per scope in `.wren/tasks.toml`. **Acceptance:** an M11 fixture (two groups, `tasks-me` aliases, a multi-owner row, undated finished work, stamps 14 and 15 days old) asserting counts, order, strikethrough, persisted filters after restart.
- **10.4 Writes and undo** (Opus; sol). Keys and clicks: Space and ⌘↵ toggle; a click on the box cycles; S, D, P, + and −, E, ↵, R. Every write through 9.1 with its inverse in the same item; a message naming note and line with Undo; ⌘Z / Ctrl+Z in the view; rows stay put until R. **Acceptance:** each action and its undo; row stability until R; stale rows refused.
- **10.5 Later in the ring** (Opus; sol). Inline description edit and saved views (design §9, §12). If either would not fit, record it in §10 for the owner rather than dropping it silently.

### M11. Ring 1 finish

- **11.1 Docs and scenes** (Sonnet; sol): `docs/reference/tasks.md`, `docs/reference/workspace-folder.md` (`.wren/tasks.toml`), configuration, keybindings, changelog; every Run B scene.
- **11.2 Ring review and merge** (Fable judge; Codex `gpt-6.2-astra` on the branch since its merge base), the run-end checklist, pull request, merge, tag `tasks-r1`. Stop and report.

**Not in Run B:** a note-set scope from several selected notes (design §9): the Notes column has single selection, and multi-select is its own feature. Recorded in §10 for the owner.

## 6. Models and review

| Tier | Items | Codex reviewer |
|---|---|---|
| Fable | 1.1, 1.3, 2.1, 3.1, 3.2, 8.1, 9.1, 10.1; the ring judge (7.3, 11.2) | `gpt-6.2-astra`, `--cap-min 25` |
| Opus | 0.1, 0.3, 1.2, 1.4, 2.2, 2.3, 3.3, 3.4, 4.1, 5.1, 6.1, 10.2 to 10.5 | `gpt-6.2-sol`; 4.1 gets `gpt-6.2-astra` |
| Sonnet | 0.2, 7.1, 7.2, 11.1 | `gpt-6.2-sol` |

This table is the only routing rule. The orchestrator is the main session on Opus. A Fable item that falls back to Opus (quota) keeps its astra review.

**Review rule.** One Codex review per item before its commit, through the codex-cli plugin's supervised runner (read-only, in the background, `--changes-since HEAD` from the worktree, a time and file-read budget, a fixed verdict format with severity 1 to 5). Fix findings; after a fix, rerun the affected checks and recapture changed scenes before committing. A second review only when a severity 4 or 5 finding needed a non-trivial fix, at most once per item. A runner exit other than 0 is "no review", never "no findings": retry once, then a CodeRabbit review (`coderabbit review --agent --uncommitted`); if neither gives a usable review, stop before merging (§9). The ring review uses `--changes-since <merge base>`.

## 7. Verification

- **Rust:** inline unit tests; editor tests over `BackendKind::ALL` unless advanced-only; core filesystem tests in `crates/wren-core/tests/`; every test that depends on the date pins it.
- **Functional:** `tools/wrentest/test_tasks_*.py` through the `tasks.*` requests. Each test sets its own settings (`tasks = on` and the engine it needs) and asserts the effective settings; negative cases run with `tasks = off` and on classic.
- **Native input:** X11 cases (the `native_x11` marker) for box clicks, right-clicks on tokens and real ⌘/Ctrl/Alt chords. macOS: Metal scenes over ssh for every UI milestone; native macOS input (Command keys, dead keys, the emoji picker) needs the owner at the console, so it is listed in the run report as a hands-on check.
- **The matrix**, every local smoke run: `WRENTEST_ENGINE=classic,advanced`, and at milestone ends all four lanes, `ICED_BACKEND=wgpu make smoke-x11`, `ICED_BACKEND=tiny-skia make smoke-x11`, `ICED_BACKEND=wgpu make smoke-wayland`, `ICED_BACKEND=tiny-skia make smoke-wayland`, each with `WRENTEST_TASKS=off` and `on`. Record test counts and skips per lane, so a lane that ran no task cases cannot pass silently. Check the branch's CI after each milestone push.
- **Scenes,** captured by the item that adds the state, in `wren-light`, `wren-dark` and `solarized-light`, advanced engine unless the scene is about classic, pinned date: task lines (overdue, done, cancelled, doing, unknown box), the status-bar line (normal and narrow), status, priority and task menus, the calendar (empty, typed, invalid), suggestions (dates, priority, owners), the form (new, edit); in Run B the view (all, narrowed, grouped by due and by owner, empty), counts on and off, the scope menu, the undo message, a hidden note's mark.
- **Gates:** `make check` per commit; `zensical build --strict` when `docs/` changes; the macOS clippy line after touching `keybind.rs`, `commands.rs`, `preferences.rs`, `menu.rs` or `macos_menu.rs`. Never run a plain `cargo build` while a smoke suite runs.

## 8. Run-end checklist

Before merging, on the pull request's final head:
1. Every item's RUNLOG entry is complete.
2. A table in the RUNLOG maps each decision in `discovery/tasks.md` §2 (items 11 to 29 for the ring's scope) and each bullet of §12 for the ring to the unit, functional, native and scene evidence for it.
3. The §7 matrix passed locally, and CI's `ci-success` is green on the final head; after any fix from the ring review or the pull request's bots, the affected gates were re-run on that head.
4. The Mac checks passed, or are listed as pending with the owner told (a pending Mac check blocks the merge).
5. The baseline fixture passes with `tasks = off`.
6. Deviations are folded into `discovery/tasks.md` and the docs build passes.

## 9. Stop conditions

The run continues on its own except when:
1. the emoji spike gives a no-go after the checks in 0.3;
2. moving spell check onto a primitive (2.1, 3.1, 3.2) changes its behaviour and no fix keeps both;
3. the design and the code cannot both hold, and the choice is the owner's (record it in §10 first);
4. the baseline `make check` is red at setup, or CI fails twice for a reason not understood, or a non-task test regresses for a reason not understood;
5. an item fails two fix rounds;
6. a change would touch `vendor/`, the board format's `golden/` files, `.wren/workspace.toml`'s v1 keys, or the board file format;
7. no usable review can be had for an item (runner and fallback both failed);
8. a Mac check is still pending when the run reaches its merge, or the Mac's `~/projects/wren` is not on the external drive when a Mac check is needed (HANDOFF);
9. the disk has less than 20 GB free;
10. the end of each run.

## 10. Deviations

Each entry: date, item, what changed and why, and whether the owner has seen it.

- 2026-11-24, planning: the note-set scope (several selected notes, design §9) is not in Run B; the Notes column has single selection. Owner to confirm.
- 2026-11-24, planning: summary-view history is session-local, never persisted (design §9 updated).
- 2026-11-24, planning: with `tasks = off`, the classifier keeps today's three box characters and the toggle writes no stamps (§1).
- 2026-11-24, Run A, after M2 (owner-approved speed-ups; §9's stop conditions, the tasks-off lanes and baseline, Fable for 3.1 and 3.2 with their no-behaviour-change proof, and every Mac check before the merge are unchanged):
  - **Milestone ends merged.** M2 and M3 keep their own ends. M4 and M5 have no separate end: one combined matrix and one Mac check run after 6.1, covering M4 to M6. Branch CI is still checked at every pushed commit, and a red run is treated as before.
  - **Narrower builder smoke runs.** A builder runs `make check`, its item's own smoke files, `smoke-tasks`, and the spell smoke files when spelling is touched. The whole X11 suite with tasks off moves out of the briefs; the milestone matrix and CI cover it.
  - **Parallel matrix lanes.** From the M3 matrix on, the X11 and Wayland lanes run at the same time from the snapshot worktree, each with its own target directory and display. A lane that fails in a load-related way is rerun alone before it counts as red; after two false reds from running in parallel, the lanes go back to running one at a time.
  - **Minor review findings may be answered.** A severity 1 or 2 performance or polish finding may be answered ("noted for later") instead of sending the builder another round. Each one is listed in the RUNLOG and the run's final report. Correctness findings, and anything of severity 3 or higher, are still fixed.
  - **Overlapping items.** The next item's builder may start while the current item is under Codex review or in its fix round, when the two touch mostly different files. It works in a second worktree branched from the last commit and rebases onto the finished item before its own gates. Two smoke suites, or a smoke suite and `make check`, never run in the same worktree at once. The 20 GB free-disk stop condition applies when a worktree is added. 3.2 does not overlap 3.1 (it builds on 3.1's popover).
- 2026-11-25, item 3.3: four existing expectations change, with tasks on only, because the contract makes `markdown.toggle_task` write stamps with tasks on (§2 item 20) and adds a Tasks palette group. The four are `every_reader_of_a_task_line_takes_the_apps_task_options` in `wren-iced`, the toggle cases of `test_tasks_status.py`, the command catalog of `test_navigation.py`, and the toggle in `test_markdown_keys.py`. Every tasks-off expectation and the baseline are unchanged.
- 2026-11-25, items 3.1 and 3.2: input that a popover opening cannot vouch for is dropped, not forwarded. A right-click in a batch that moved the text first does what it did before (spelling only), and keys and input-method text in the frame of a keyboard handover are dropped. Two deliberate spelling fixes: when Enter and a click on a row arrive in one batch, Enter is handled first; and keys typed while an input method composes are no longer taken by an open menu. Spelling's tests and scenes are otherwise unchanged.
- 2026-11-25, item 3.4: existing expectations changed deliberately. Tests in 3.1 and 3.2 expected a bare `Accept`/`AcceptSelected` from request menus and provider menus; they now expect `AcceptOf { menu, row }`, so a queued choice cannot land on another menu. The Tasks-group listings in 3.3 gain `task.set_status` and `task.set_priority`. The batch-shape table gains a row. A primary click on a task's box cycles it 300 ms after the press (iced's double-click window), so a double-click selects a word instead. A key that opens a submenu ("Priority…") drops the rest of its input batch.
- 2026-11-25, item 4.1: ⌘D / Ctrl+D becomes `task.set_due`'s default with tasks on, so the keymap test that pinned "no task command has a default key" now pins this one key; the board layer's ⌘D still duplicates. A right-click on a due date, and the task menu's Set due date row, open the calendar; the 3.4 tests that expected the task menu there changed with them. Input-method text that arrives after the calendar closes is dropped until the editor's own input: a composition the editor starts, a key that types or moves the caret, a click in the text, or a new handover. With tasks on, the window's root is a fixed stack of three layers; with tasks off it is unchanged.
