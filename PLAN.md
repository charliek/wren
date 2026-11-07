# Wren implementation plan

## Approved direction

Build the complete MVP defined in `SPEC.md`, using `MOCKUP.md` and `mockup/` for visual design. This is not a reduced editor prototype. Part 2 of the spec wins when the supplied documents disagree.

- Use Rust edition 2021, iced `=0.14.0`, and two crates: toolkit-neutral `wren-core` and desktop adapter `wren-iced`.
- Make Linux fully functional and locally verified. Prepare macOS builds and CI, but defer native macOS integration and Japanese IME acceptance testing. Do not report those checks as passed without evidence.
- Keep Markdown files authoritative. All document and structural mutations use the same core operation path from the UI, palette, drag adapters, and tests.
- Treat autosave as a primary reliability requirement. In addition to the 750 ms idle debounce, impose a maximum dirty-age persistence deadline during continuous typing. Test failures, flush barriers, stale completions, and external edits; do not promise zero-loss behavior under disk failure or uncooperative simultaneous writers.
- Support system-following light and dark themes. Centralize semantic color roles and widget styling so additional themes do not require rewriting views. User-authored theme files are not required for the MVP.
- Commit small, verified checkpoints directly to `main`. Do not create branches, push, publish artifacts, or configure a remote without a separate request.
- Make routine implementation decisions autonomously. Escalate genuine access/safety blockers and an iced editor no-go rather than silently changing toolkits.

Preview, attachments, wiki links, templates, tags, pins, daily-note features, a public `wrenctl`, and an updater remain post-MVP.

## Checkpoints

1. **Repository and tooling setup.** Preserve the supplied design, pin local tools and dependencies, establish common commands, and prove that a real iced window builds and renders on Linux. This is the current task, before the full implementation run.
2. **Editor go/no-go (M0).** Exercise a 5,000-line Markdown document, highlighting, wrapping, undo/redo, selection, clipboard, find, and cursor/scroll preservation. Record results in the spike document required by the spec. Distinguish actual native-input evidence from synthetic tests. Stop for a human decision if iced demonstrably fails.
3. **Filesystem and core safety (M1).** Implement scanning, stable IDs, the operation set, collision-safe creation/rename/move, injected trash, atomic persistence, draft/disk revisions, and failure handling.
4. **Safe working application (M2).** Build Columns, editing, creation/rename dialogs, autosave, history, save status, and initial watcher/conflict handling. Bring data-loss prevention forward rather than leaving it for polish.
5. **Workspaces and navigation.** Finish the palette, native folder picker, Add/Switch/Remove, Outline, resizable panes, and per-workspace selection/history/layout/sort/width persistence.
6. **Search (M3).** Add scoped filters, fuzzy Jump with recency, global search, snippets, match navigation, and index updates after internal and external mutations. Measure the specified 5,000-note/25 MB release benchmark.
7. **Moves and gestures (M4).** Complete move pickers, ordering, pure drag rules, actual hit testing/capture, ghosts, cancellation, valid targets, and spring-opening in both layouts.
8. **Complete and harden (M6).** Finish visual fidelity, both themes, focus mode, confirmations, Reveal, empty/error states, usable desktop packaging, and the full definition-of-done audit.

## Architecture and safety constraints

- The session is the ordered mutation owner. Widget buffers are synchronized editor projections, not independent document stores.
- Save barriers cover switches, normal quit, and moves/renames/deletions of an open note or its ancestors. Failed writes retain the draft and prevent destructive transitions.
- Saves remember disk-content revisions; watcher mtime/size suppression alone is not conflict protection. File notifications are reconciliation hints, not one-to-one logical changes.
- Background results carry workspace/document generations. A late save acknowledgment cannot clear newer edits, and an old scan cannot replace a new workspace.
- Use exclusively created sibling temporary files and collision-safe destination operations. Preserve the original on failed writes; never fall back from trash to permanent deletion.
- Persist validated workspace-relative references and resolve fresh session IDs on relaunch. Ancestor mutations update descendants and history.
- Leave unsupported root-level Markdown, symlinks, and unreadable/invalid files untouched and warn clearly. Do not automatically restructure an existing vault.
- Missing or corrupt optional metadata must not damage Markdown. Removing a workspace registration never deletes its files.

## Validation

Add tests with each capability, including regression tests for bugs found during development.

- **Rust:** inline unit tests for pure state machines; filesystem integration tests under the relevant crate's `tests/` directory; injected time, trash, and failure boundaries.
- **Python functional:** launch a fresh, harness-owned real iced process with temporary workspaces/config. A private opt-in test socket will drive production editor, command, and selection paths. Use bounded condition waits and assert actual UI state plus on-disk contents. Never attach to or kill a normal user instance.
- **Rendering:** targeted screenshots verify colors, layout, clipping, overlays, and narrow windows. Exercise wgpu and the tiny-skia fallback.
- **Native input:** small platform-specific tests cover actual typing, shortcuts, clipboard, divider gestures, and drag-and-drop. IPC tests are not evidence of OS input/IME correctness.
- **Durability:** test continuous typing, all flush triggers, failures, clean restart, forced process exit, external clean reload, and conflicts preserving both versions. Process-kill tests do not prove power-loss durability.
- **Review:** independently inspect the storage-safety work and significant/final diffs. Fix actionable findings and rerun affected checks.

Common Rust gates will include:

```sh
cargo test --workspace --locked
cargo build --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

Test-hook configurations and normal builds must both be checked. Keep local commands and CI entry points aligned. Do not silently skip required scenarios or rerun failures merely to obtain a green result.

## CI and delivery

Prepare GitHub Actions for Linux/macOS build/test/lint, Python harness tests, Linux Xvfb functional tests, a focused headless Wayland lane, software-renderer smoke, and scheduled release-performance tests. Preserve JUnit, logs, snapshots, and screenshots on failures. A stable aggregate check must include every required job and reject unexpected skips.

Minimum desktop delivery is a macOS `.app` and Linux executable/desktop integration with icons and bundled dependency/font notices. Developer-ID signing, notarization, remote publication, and auto-updating require separate authorization/resources.

The final handoff must identify verified commands/results, remaining platform/manual checks, checkpoint commits, and any unmet acceptance criteria. An empty build scaffold is not a completed MVP.

## Tooling checkpoint

The Linux setup has been exercised with Rust 1.97.1, Python 3.12.3, uv 0.13.5, and the locked iced 0.14 dependency graph. `make setup`, `make build`, and `make check` pass locally. `make check` covers normal and smoke-enabled Rust tests/Clippy, formatting, and Python harness/CI-contract unit tests.

Both themes render in real windows under Xvfb and headless Weston, using wgpu/Vulkan (Mesa llvmpipe) and tiny-skia. Reproduce these setup checks with:

```sh
ICED_BACKEND=wgpu WGPU_BACKEND=vulkan make smoke-x11
ICED_BACKEND=wgpu WGPU_BACKEND=vulkan make smoke-wayland
ICED_BACKEND=tiny-skia make smoke-x11
ICED_BACKEND=tiny-skia make smoke-wayland
```

Each lane captures real renderer pixels and validates visible content/backgrounds, required renderer identity, opt-in rejection, invalid arguments, and refusal to overwrite existing screenshot output. Logs, PNGs, and JUnit are kept under the ignored `target/wrentest/` directory. The normal build rejects the screenshot CLI even if the test-mode environment variable is set.

The display wrapper owns private displays, runtime/config directories, and a session bus without desktop-service activation. Disabling activation prevents headless tests from spawning desktop portal helpers or leaving FUSE mounts behind. Native folder selection and actual desktop theme-change delivery are not established by these forced-theme rendering tests.

Registry/source preflight found compatible stable candidates for subsequent implementation: `notify =8.2.1`, `notify-debouncer-full =0.7.0`, `trash =5.3.0`, `nucleo-matcher =0.3.1`, and `rfd =0.16.1` with its Rust XDG-portal backend. The newer rfd 0.17.2 dynamically loads libdbus; prefer the checked 0.16.1 portal approach unless later implementation evidence justifies changing it. The core now uses the pinned notify and trash candidates. Its watcher coalesces bounded rescan hints directly rather than adding the full-debouncer dependency. Search and folder-picker dependencies remain unadded. The local desktop has a FileChooser portal, but no interactive picker was opened during preflight.

The initial GitHub Actions run passed Linux/macOS compilation and Rust checks, Python unit tests, and all four Linux renderer/display smoke lanes on the tooling baseline (`5b19143`). Native macOS integration remains unverified. Scheduled performance testing belongs with the search implementation, not a placeholder passing job.

## Editor checkpoint

The real in-memory editor, bounded history, literal find, semantic Markdown highlighting, bundled fonts, and private opt-in main-thread test IPC are implemented. `docs/spike-editor.md` records the Linux go decision, actual 5,000-line measurements, native X11 typing/clipboard/shortcut checks, rendering evidence, and the exact undo/redo viewport limitation. Native macOS IME acceptance remains deferred.

The four local Linux lanes now include real-editor tests: 21 passing cases in each X11 lane and 17 passing cases in each Wayland lane, with the four X11-native cases explicitly deselected on Wayland. CI installs the native-input tool and uses the same targets. These editor changes have not yet run remotely.

At the editor checkpoint, the application did not open or save notes and explicitly warned about its in-memory buffer. The go decision alone was not persistence acceptance.

## Filesystem and autosave core checkpoint

The toolkit-neutral core now implements scanning, stable typed session IDs, every prescribed filesystem operation, workspace switching, navigation history, optional workspace metadata, draft/baseline tracking, bounded autosave, and watcher reconciliation. One ordered Session applies mutations; failed operations carry events for side effects that already committed.

Storage walks validated relative paths through no-follow directory descriptors, including enumeration. A persistent advisory lock prevents cooperating duplicate writers. Creation and moves use atomic no-replace publication and collision suffixes; saves compare exact baseline bytes, preserve ordinary permissions, start sibling temporaries at 0600, fsync their data, atomically replace, and sync the parent. Failed directory syncs retain retryable descriptors rather than pretending publication did not occur. Unsupported files, special files, symlinks, incomplete scans, and malformed metadata cannot silently turn into destructive edits. Reads are bounded at 64 MiB per note; larger files are warned about and left untouched.

Autosave uses injected monotonic time with both a 750 ms idle debounce and a 750 ms oldest-dirty deadline, revision/generation-aware completion, and bounded failure backoff. The synchronous Session save path protects note/ancestor moves, renames, deletions, note/workspace switches, and explicit flush. Observed external conflicts preserve a collision-safe, durably published copy before reloading; deleted or invalid originals never justify discarding the only draft. Clean deletion closes the document. Unsupported external bytes may leave the original untouched and open the saved recovery copy instead.

Core verification on Linux:

```sh
cargo test -p wren-core --locked
cargo clippy -p wren-core --all-targets --locked -- -D warnings
```

The test run passed **192 inline tests, 60 Session filesystem integration tests, and 6 focused recovery regressions (258 total)**. Barrier tables include pre-publication failures, failed conflict-copy persistence, and post-publication directory-sync failures. Tests inject a private quarantine, never use user OS trash, and assert real disk bytes alongside model/draft state. Independent review reproduced and led to fixes for orphaned dirty-document recovery, inactive writes bypassing a loaded baseline, and replaced-root scanning. Additional tests fixed directory names ending in `.md` and duplicate restored history entries.

At this core-only checkpoint, these checks were not yet real-window autosave acceptance. The integration checkpoint below connects the editor, normal close/focus handling, browser operations, and shared test commands to this Session, and verifies actual timer saves, clean restarts, process kills, and external conflicts in the Linux display lanes.

The Unix final compare/replace window cannot be made a content-conditional atomic rename against an uncooperative external writer. OS trash is a path-based API and also retains a final namespace race. These limitations are explicit; no power-loss, hostile-namespace, native macOS, or OS-trash integration guarantee is inferred from temporary-directory tests.

## Live workspace integration checkpoint

The application now opens an existing workspace and edits actual Markdown files. Start the normal build with:

```sh
cargo run --locked -p wren-iced -- --workspace /absolute/path/to/notes
# Equivalent development target:
make run ARGS="--workspace /absolute/path/to/notes"
```

The selected root must already exist. Sections are its immediate non-hidden directories; groups are directories inside sections. Use the creation dialogs for an empty workspace. At this integration checkpoint, launching without `--workspace` remained the unsaved editor evaluation. The navigation checkpoint below replaces ordinary startup with workspace restoration/onboarding and makes that evaluation explicit with `--editor`; `--editor-spike` still supplies its 5,000-line in-memory fixture.

Basic Columns now exposes sections, groups, No Group, notes, counts, breadcrumbs, create/rename/move dialogs, explicit trash confirmation, Back/Forward, workspace-path switching, both semantic themes, and save/retry/error status. Native Return alone cannot confirm trash. The live Content/history/cursor survives own renames, moves, and theme changes. External reloads preserve and clamp the cursor. This is a usable integration surface, not final mockup fidelity or the complete MVP.

Every edit reaches the same ordered Session used by widgets and private test requests. The application timer drives the 750 ms scheduler, and native focus loss and window close use flush barriers. A failed close leaves the window and draft available for recovery. Workspace loading transfers the serial Session owner to a task, disables edits while pending, restores the original session on failure, and defers close/focus flushing until the owner returns. Post-publication errors project committed changes instead of reverting the editor or inventing a failed filesystem operation. Writes over the 64 MiB read limit are now rejected before creating a temporary, preventing publication of a note the application cannot reopen.

Independent frontend review reproduced a native creation-dialog error: the item was published, optional metadata persistence failed, and retrying the still-open dialog created a duplicate. Matching Created events now dismiss only the creation that actually committed, retain its error banner, and return focus to the editor. Unpublished creations stay retryable; unrelated flush events do not dismiss a dialog. Rust regressions cover all three creation kinds, and a native X11 regression reproduces the metadata permission failure without injected faults. The harness also now observes an already-exited owned child before attempting quit IPC, while still reaping it and rejecting nonzero exits. This fixes native-close teardown without suppressing live transport/protocol failures.

Final local gates:

| Command | Result |
| --- | --- |
| `make build check` | Passed normal build, formatting, both Clippy configurations with warnings denied, all Rust tests, and 149 Python harness/CI-contract unit tests. |
| `ICED_BACKEND=wgpu WGPU_BACKEND=vulkan make smoke-x11` | 51 passed. |
| `ICED_BACKEND=tiny-skia make smoke-x11` | 51 passed. |
| `ICED_BACKEND=wgpu WGPU_BACKEND=vulkan make smoke-wayland` | 43 passed; 8 X11-native cases explicitly deselected. |
| `ICED_BACKEND=tiny-skia make smoke-wayland` | 43 passed; 8 X11-native cases explicitly deselected. |

The Rust totals per configuration are **259 core tests** (193 inline, 60 Session integration, 6 recovery), **103 editor-library tests**, and **29 normal application tests / 47 smoke-enabled application tests**. No tests were ignored in those Rust runs. GUI cases assert actual on-disk bytes and editor state, including timer saves, continuous typing across multiple persistence deadlines, clean and forced restarts, a fixed 1.1-second process kill independent of saved acknowledgment, eleven failed-save transition barriers, same-size/restored-mtime external edits, dirty conflict preservation, real permission-denied recovery, workspace switching, corrupt metadata preservation, and both-theme rendered editor glyphs.

The X11-native cases additionally exercise actual typing/clipboard/shortcuts/selection, focus-loss flush, successful close flush, failed close retaining its draft, and committed-creation dialog recovery. A separate one-off native X11 probe ran the **normal, non-test-enabled executable** with tiny-skia and wgpu/Vulkan: each renderer completed two owned launches, native typing and timer saves, WM_DELETE_WINDOW flushing, reopening and editing the restored text, and exit code 0. The normal executable rejected both test IPC and screenshot flags even with the test environment variable enabled. This probe was local, not an additional CI job. Native Wayland input was not exercised.

Actual saved-note measurements use a **5,000-line, 384,999-byte UTF-8 note** in the debug build, one timer save plus eight explicit editor-edit/flush saves per lane. The timer column measures request start through Python observing matching disk bytes and clean state, including scheduling, RPC, and polling; it is not the pure scheduler deadline. Core save timing covers the synchronous read/compare/write/fsync/publication path, not rendering or key-to-photon latency.

| Display / renderer | Timer to observed clean | Core save median / max (9 samples) |
| --- | ---: | ---: |
| X11 / wgpu Vulkan | 820.82 ms | 4.76 / 7.86 ms |
| X11 / tiny-skia | 828.91 ms | 4.94 / 7.86 ms |
| Wayland / wgpu Vulkan | 853.94 ms | 3.78 / 6.85 ms |
| Wayland / tiny-skia | 834.55 ms | 4.80 / 7.81 ms |

Screenshots, measurements, logs, and JUnit remain under ignored `target/wrentest/`; both themes were also visually inspected on X11/tiny-skia and Wayland/wgpu. Functional trash checks use only the private injected quarantine; no user's OS trash was touched. The existing GitHub Actions matrix uses these same targets, but only the earlier tooling baseline has run remotely. CodeRabbit could not authenticate its review on this host, so no successful CodeRabbit review is claimed; independent local editor, storage, and frontend reviews supplied the actionable findings and regression checks.

The requested editor plus filesystem/autosave handoff is ready for review. Remaining MVP work includes workspace registration/native picker, palette and global search, Outline, resizable panes, layout/width controls, drag/order gestures, packaging, and visual polish. Native macOS/IME, native Wayland input, OS-trash integration, release-scale search performance, and power-loss behavior remain unverified. The new checkpoint commits are local on `main`; no new push was requested.

## Next large implementation pass

### Context and intended outcome

The user reviewed the basic editor at `972e979` and requested a substantial continuation toward the finished spec and mockup. This pass targets the missing everyday navigation and workspace experience, both complete layouts, actual gestures, and desktop presentation, rather than another isolated editor increment. Linux remains the full local acceptance platform; macOS native/IME acceptance is still deferred. Both semantic themes remain required. Continue checkpoint commits on `main`; this request is an implementation push, not authorization to publish or push git.

### Work breakdown and ownership

1. **Core navigation foundations.** Add `crates/wren-core/src/index.rs` and `config.rs`; extend `ops.rs`, `workspace_state.rs`, `lib.rs`, the crate manifest and integration tests as needed. Pin `nucleo-matcher` for fuzzy names, `directories` for config discovery, and `rfd` with the Rust XDG-portal/Tokio backend for the desktop folder picker. The index belongs to the Session and is built during background workspace loading, uses descriptor-relative reads, contains authoritative paths/IDs plus content, and updates after own edits and external reconciliation. Queries are in-memory only, scoped by workspace/section/parent, UTF-8 safe, snippet-bearing, and recency-ranked for Jump. Bound retained content and report indexing limitations instead of exhausting memory. Add ordered metadata-only preference/selection operations so the UI never opens a second workspace writer. Preserve malformed/unknown config and metadata safely; removing a registration must never remove files.
2. **Palette, workspaces, and search.** Add `crates/wren-iced/src/palette.rs` and navigation adapters; extend `app.rs`, `workspace.rs`, `test_requests.rs`, `cli.rs`, and `main.rs`. Implement Commands, Jump, global search, Move, and Switch-workspace modes with fuzzy filtering, keyboard navigation, scroll-to-active, Escape, scrim dismissal, and context-enabled actions. Register Add/Switch/Remove through platform config; use a native asynchronous picker with a private test adapter for its result, never a public bypass. Restore the last workspace on ordinary startup and show a real empty onboarding view if none is registered. Preserve the explicitly requested `--editor-spike` route for editor regression tests. Keep every note/workspace transition behind existing flush barriers. Add global platform shortcuts without firing navigation/create/delete shortcuts while a modal text field is editing.
3. **Layouts and presentation.** Add `panes.rs` and supporting view/style modules as warranted; replace the provisional `workspace_view.rs` with the mockup's 48 px title bar, workspace button, quiet controls, uppercase pane headers and plus actions, per-pane filters, two-line note rows, breadcrumb/actions, and saved/dirty footer. Implement shared-selection Columns and Outline, expansion, focus mode, persisted sort/layout/per-layout widths, and empty/error states. Layout/theme/focus changes must retain the live editor Content, history, cursor, selection, and viewport. Remove the ordinary Save command; retain an explicit retry action for actual save failures. Use `MOCKUP.md` and actual `.dc.html` values, not generic toolkit defaults, while adapting semantic roles for light mode.
4. **Native resize and drag.** Add `drag.rs` and custom interaction widgets following inspected Roost patterns. Keep geometry/rules pure and tested separately from iced pointer capture. Resize every browser pane with 7 px handles, keyboard steps, double-click reset, live chip, spec clamping and narrow-window proportional shrink; persist only at interaction completion. Drag notes/groups/sections with a 4 px threshold, valid target feedback, ghost, same-parent insertion lines, 600 ms spring-open, Escape/window-focus-loss cancellation, and exactly one core operation on valid drop. Support both layouts and cross-section moves. Stale/missing targets and failed flushes must not corrupt selection or discard a draft.
5. **Delivery and hardening.** Add only required Linux desktop/icon/bundle assets and macOS bundle scripts, extending `Makefile`/CI rather than inventing successful placeholder jobs. Add the release search benchmark to scheduled CI. Audit the spec's definition of done and document actual evidence and remaining platform/manual limits here.

Independent coding agents will own disjoint core/config/gesture/view/packaging modules. Shared manifest/lockfile, main integration points, acceptance tests, CI, and this plan remain coordinated by the parent. Agree on concrete module APIs before wiring overlapping files. Do not replace `target/debug/wren` with a normal build while functional tests are launching smoke-enabled processes.

### Safety and review hunt list

- Never let an overlay activation or registration update bypass a failed flush or advance last-active workspace before the switch actually commits. Cancelled picker/failed open leaves the previous workspace usable.
- Keep stable IDs in UI actions; resolve fresh targets at activation/drop, and consume partial committed events even on error. Do not infer mutation success from arbitrary preceding flush events.
- Search must not use pathname traversal that weakens the store's no-follow rules. Same-size/restored-mtime edits must refresh search content; unreadable/deleted files cannot silently become empty destructive drafts. Queries must include the current draft without writing it first.
- Search matching/snippets/cursor offsets must handle Unicode lowercase expansions and valid UTF-8 boundaries. A stale search result must not select an arbitrary byte range in newer text.
- Opening overlays, changing layout/theme, filters, or focus mode is not a note transition and must not clear editor history or replace unchanged Content. A successful switch replaces workspace-specific selection, filters, expansion, index, widths, sort, and history.
- Global config is optional but must use exact loaded-byte expectations, exclusive temporary publication, retained post-publication state, and bounded reads. Invalid or unsupported config stays untouched and visible as a warning. Duplicates are canonicalized; removal edits registrations only.
- Pointer capture must survive leaving a divider/row; release/cancel clears every gesture state. A drag cannot accidentally activate a row or become a second drop. Narrow widths never wrap panes below the editor.

### Acceptance and tests

- Add inline Rust tests with new pure logic and real-filesystem Session/config integration tests. Run relevant tests first, then `make build check` for normal and test-enabled configurations at stable checkpoints.
- Add Python functional cases for every palette mode, keyboard activation, scoped/global searches and hit cursor placement, recency, index updates through own/external operations, configuration restoration and registration removal safety, both layouts/focus/width/sort persistence, and failed-save barriers through the new adapters.
- Add actual native X11 keyboard, divider-drag/reset/key-resize, row drag/drop/cancel/invalid-target/spring-open cases. Private RPC may inspect state and drive the same handlers, but is not evidence of native pointer behavior. Use negative controls for new functional assertions before finalizing them.
- Capture and inspect both themes, Columns/Outline, palette/search, focus, resize/drag, and a narrow window. Keep artifacts under ignored `target/wrentest/`.
- Run all four Linux lanes: X11 and headless Wayland, each with wgpu/Vulkan and tiny-skia. Explicitly deselect only X11-native cases on Wayland; report exact results and retain existing autosave acceptance.
- Measure the required release query workload (5,000 notes / 25 MB, under 30 ms per query), with a named ignored benchmark test explicitly run by scheduled CI. Report actual query/build measurements and what they do not measure.
- Significant interfaces receive a scoped simplification/correctness review; data-integrity and pointer/ordering changes get adversarial external review. Fix actionable findings, rerun affected checks, and record unavailable reviewers honestly. Checkpoint commits include their review disposition; no git push without a separate request.

### Plan review disposition and integration contracts

A supervised, read-only Codex Astra plan review completed successfully against baseline `972e979`. The GLM CLI seat failed immediately because the configured model was unavailable; CodeRabbit remains unavailable after the earlier host authentication failure and was not retried. Independent local planning and Roost/mockup inspections supplied additional implementation detail. The review's six integration blockers and three acceptance improvements are accepted as follows:

1. Global Config is application-owned and optional, not part of the Session lifetime. Explicit workspace opens work despite unavailable config. Concurrent instances on different workspace roots must still edit notes; registry writes may conflict/fail visibly without blocking those sessions. Add registers/switches only after observing the actual active canonical root; same-root successful opens count even without a generation change. Removing an active registration leaves its current Session open and clears startup membership. Missing last-active roots lead to usable onboarding/switch UI, not a fatal launch. Test pre/postpublication errors and conflicting writers separately.
2. The UI gateway must expose a requested-action disposition: blocked, committed-with-warning, completed, or async-pending. Match creation events, committed selection, or the target's actual before/after parent/path/order—not arbitrary preceding save events. Close pickers/dialogs only when their requested effect committed; retain visible warnings and never replay committed mutations. Search hit positioning must only run against the actual requested active note. Separate document-save, workspace-metadata, and global-config warning/retry responsibilities.
3. Closed-note content invalidation is independent of TreeChanged metadata. Capture exact descriptor-relative content revisions during reconciliation (prefer sharing scan reads); no mtime/size-only cache validity. Queries use active draft text as authoritative without saving. Verify periodic reconciliation with no watcher hint, same-size/restored-mtime unopened edits, unreadable recovery, and removal of old as well as appearance of new matches. Measure indexing/reconciliation on the release fixture, not just warm queries, and avoid repeated lowercasing/full duplicate reads where possible.
4. UI search actions capture workspace generation, note ID, index/content revision, and original-text UTF-8 byte-range provenance. Results distinguish title-only hits. After the flush/open barrier, validate the actual selected document and recompute the literal hit against current text; never blindly apply an old folded offset or overwrite the user's find query. Test stale identical IDs across workspaces, same-length edits after querying, conflict-cancelled activation, Unicode expansions/combining marks/emoji, all supported line endings, and snippet truncation.
5. Drag spring browsing/outline expansion is transient and separate from committed document/parent selection. A root owner survives source-row removal from the visible tree and owns cleanup. Escape, workspace/layout replacement, stale source, and blur clear file dragging before any fallible flush; blur settles resize. A failed focus-loss save must not retain capture/ghost state. Preferred widths and temporary effective widths are distinct; narrowed rendering never overwrites persisted preferences. Gesture messages are scoped to workspace/layout identities.
6. Only sections and same-section groups reorder. Notes have move targets only, never insertion slots. Resolve filtered-list insertions against stable full-sibling anchors: dropping before a visible sibling inserts before it in the full order (hidden siblings retain their relative order); the trailing slot appends after all siblings. A self-anchor is a no-op, and a deleted/moved anchor cancels rather than appending unexpectedly. Cross-section groups append to destination ordering. Test complete resulting order, restart persistence, and a metadata failure after in-memory ordering commits.
7. Use the proportional width allocator with 120 px effective browser floors, normal persisted clamps, 320 px editor reserve, and overlapping divider hit areas. Below physically feasible widths, clip/nonwrap rather than invent negative sizes. Test wide→narrow→wide/restart, compressed drag semantics, long paths, and short overlay viewports. Keep editor/browser/optional-banner/find widget slots structurally stable; Content retention alone is not proof of viewport retention.
8. Jump MRU is distinct from branchable Back/Forward history. Actual visits including Back/Forward refresh MRU; failed transitions do not. Persist/restore history position and MRU paths, remap/prune known fields, and update committed navigation bookkeeping before optional metadata persistence can fail. Cover A→B→Back→C and duplicate visits/forward-history restarts. Fix the baseline Back/Forward metadata-error bookkeeping while adding this capability.
9. Acceptance must enumerate the entire MVP command list, including Reveal, dated note creation, theme choices, both layouts, reset widths, and context-specific Rename/Delete/Move. Benchmark query correctness, ranking and snippets as well as time, record build/refresh and edit responsiveness on the same 5,000-note/25 MB workload, and account for folded text/temporary rebuild memory. A passing query benchmark is not a claim about UI latency or power-loss durability.

### Core navigation checkpoint

The Session index, streaming scan ingestion, exact parent/section selection, persistent occurrence-based history, independent 100-entry MRU, and optional global registrations are implemented. `cargo build -p wren-core --locked`, `cargo test -p wren-core --locked`, core Clippy with warnings denied, targeted rustfmt, and diff checks passed. The core suite contains **355 passing tests** (243 inline, 46 navigation, 60 Session, 6 recovery), with two workload benchmarks deliberately ignored in ordinary runs and explicitly executed below.

Separate successful read-only Codex Astra reviews covered the config/editor seam and the final navigation/index/scan/metadata changes. Accepted findings were fixed with regressions: stored-root activation cannot redirect through replacement ancestors; exact search selections reject unrepresentable line-ending endpoints; first-hit reveal uses query-bounded streaming rather than all matches; surrounding emphasis survives inline code; moving a parent-selected active note restores the destination on restart; closing an unreadable active document revalidates its now-closed indexed contents even if metadata fails; and fuzzy queries are bounded before pattern allocation (4,096 UTF-8 bytes / 128 words). Config provides nonmutating registration validation for the frontend's pre-open checks, independent of config locks and writability. The three new navigation/index regressions were observed failing before their fixes. Core simplification retained concrete Session operations and reused one outgoing-note read on a committed selection rather than introducing a second draft-availability state machine. Frontend integration, gesture review fixes, and full GUI acceptance remain pending at this checkpoint.

`make perf-search` explicitly ran both release workloads successfully on the checkpoint source. The in-memory 5,000-note / 25,000,000-byte build took 34.80 ms and retained 50,000,000 original-plus-lowercase bytes. Across 20 samples per query, median/max milliseconds were Jump `mtg42` **0.313/0.536**, rare content search **4.150/4.435**, scoped filter **0.323/1.040**, and miss **3.644/3.935**. The filesystem fixture returned 383 Jump matches including `Meeting 42`, and checked counts, ranking, UTF-8 ranges, and snippets. Initial real-filesystem load took **117.69 ms**. Median/max milliseconds across 20 samples were unchanged reconciliation **80.81/92.37**, one same-size/restored-mtime external refresh **81.38/86.58**, reconciliation with a live draft **92.84/106.13**, queued edit service **0.251/0.273**, flush service **5.23/8.00**, and edit queued at reconciliation start through persisted clean **97.77/110.78**. Session remains synchronous, so input can queue behind reconciliation; these warm-cache core measurements are not native input/rendering latency, a process-RSS bound, or a power-loss guarantee.

### Desktop navigation and interaction checkpoint

The everyday desktop surface is now implemented, not just the editor evaluation:

- Ordinary `make run` restores the last registered workspace or shows onboarding. `--workspace /absolute/path` explicitly opens a folder; `--editor` and `--editor-spike` remain unsaved evaluations. Add/Switch/Remove use optional platform config and tagged asynchronous folder-picker completion. Removing a registration leaves its files, active Session, and draft intact. Stored roots are validated before a switch flush or startup open; fresh explicit opens remain usable despite invalid optional config.
- The keyboard palette exposes all **35 commands**, including dated creation, per-item actions, Reveal, conflict-copy opening, group/section reorder, themes, layouts, sort, width reset, and workspace actions. Commands, fuzzy title Jump, global title/content search, move pickers, and workspace selection share correlated blocked/completed/committed-warning/async-pending outcomes. Exact clicked-item identities and workspace generations guard context actions. Failed save barriers preserve recoverable overlays and drafts; optional metadata warnings cannot replay an already committed action.
- Columns and Outline share selection, independent scoped filters, previews, counts, breadcrumbs, persistent sort and per-layout widths. Focus mode centers the editor. The 48 px titlebar, original bird, bundled typography, semantic light/dark styling, quiet controls, dialogs, and footer replace the provisional chrome. Layout/theme/filter/overlay changes preserve editor text, cursor, selection, history, and the actual scrolled viewport.
- Root-owned native gestures support 7 px resize handles, 16 px keyboard steps, reset/Escape, preferred-versus-effective width allocation, 4 px file-drag threshold, valid destinations, ghosts, full-sibling insertion anchors, and 600 ms transient spring browsing. Cancellation survives hidden source subtrees and happens before fallible blur flushing. Notes move but never reorder. Cross-section groups append after all destination siblings, including siblings missing optional ordering metadata.
- Right-click actions and double-click rename target the actual row, not an unrelated selected note. Reorder palettes retain a removed active anchor as a disabled tombstone instead of silently appending. Empty section/group changes now refresh model-dependent palettes even when the note index revision does not change. Conflict banners offer a generation-checked link to the durably preserved copy.

Successful scoped Codex Astra reviews covered gestures, frontend gateways, final integration, and the capture follow-up. The four original gesture findings, five frontend findings, and two final integration findings were fixed with regressions. They covered stale anchors and cursor/event ordering, orphan-release ownership, cancelled conflict effects, missing editor refocus, passive result visibility, deferred blur correlation, hidden No Group filtering, empty-container palette freshness, and capture widget status. The four empty-tree Rust regressions were observed failing before the refresh fix, then passed. Core cross-section append gained six filesystem regressions, bringing the core suite to **361 passing tests** (243 inline, 52 navigation, 60 Session, 6 recovery), with two explicitly run workload benchmarks ignored in ordinary runs. CodeRabbit authentication and the GLM model seat remained unavailable; neither is reported as a successful review.

Simplification kept concrete commands and the existing ordered Session rather than adding another mutation owner. An unsuccessful editor render-epoch gating experiment was removed completely. Palette refresh reuses stable-key replacement, and screenshot batching performs one physical capture for all pending requests on the same window. No broad rename/reformat pass was added.

#### Toolkit rendering correction

`vendor/iced_winit` is the locked upstream **0.14.1** crate from iced commit `38237dd294da3256c2ac4d3f8288fc73956c91e0` (`winit/`). Its MIT license is included from that exact upstream revision (git blob `f1148e26eac8202a1c2a170b41aa766d7dbb0c8d`). Only `src/lib.rs` and `src/window.rs` differ from the registry source; the manifest, README, and remaining sources retain upstream contents. The root patch does not change other locked dependency versions or edges.

The upstream screenshot action could replay expired weak editor text primitives after an intervening layout, producing a caret but no glyphs. Rebuilding in the screenshot action fixed glyph lifetime but reset fresh buttons/text inputs to disabled visual status. The final fix instead queues screenshot senders on their native Window, requests a real redraw, and captures synchronously immediately after that frame successfully presents. It uses the normal redraw update/draw/preedit path, with no extra layout, yield, synthetic redraw event, or subscription feedback. Requests share one readback, cancelled requests are pruned, and closing a window drops its senders. Surface failures and zero-size/hidden windows do not invent successful captures; pending hidden-window behavior and surface-error lifecycle received source review, not dedicated native fault injection.

Both themes were seen red on the glyph assertion when the fresh draw was removed (`23` caret-colored pixels versus `>70`). Restoring the rejected screenshot-specific rebuild in a separate controlled check made all four native hover/focus cases fail: hovered controls lost their raised background, and native focused inputs had `52` accent pixels while captures had `0`. The final queued-frame fix was restored before the positive gates. Native readback also exposed tiny-skia's shadow pass painting over a dialog during damaged-region replay; dialogs now use the scrim and border without that shadow on either backend. The two unchanged upstream `try_next` deprecation warnings remain visible; Wren's own normal/smoke Clippy gates still deny warnings.

#### Final desktop acceptance

| Command | Verified result |
| --- | --- |
| `make build check` | Normal build, formatting, normal/smoke Clippy, all Rust suites, and **237 Python unit tests** passed. Rust suites per configuration: **361 core**, **111 editor-library**, **176 normal / 200 smoke application** tests. Two core workload benchmarks are intentionally ignored here and run explicitly below. |
| `ICED_BACKEND=tiny-skia make smoke-x11` | **142 passed**. |
| `ICED_BACKEND=wgpu WGPU_BACKEND=vulkan make smoke-x11` | **142 passed**. |
| `ICED_BACKEND=tiny-skia make smoke-wayland` | **110 passed; 32 native-X11 cases explicitly deselected**. |
| `ICED_BACKEND=wgpu WGPU_BACKEND=vulkan make smoke-wayland` | **110 passed; 32 native-X11 cases explicitly deselected**. |
| `make perf-search` | Both named release workloads passed on 5,000 notes / 25,000,000 original UTF-8 bytes. |

The GUI suite retains all prior autosave, restart, process-kill, permission failure, failed-barrier, exact-byte external refresh, and conflict preservation acceptance. New functional files contribute **91 cases** for navigation, gestures, completion fixes, and viewport retention. A temporary in-memory assertion-inversion runner observed **91/91 expected red assertion failures**, with zero unexpected setup/teardown outcomes and zero missing cases; it did not modify production code or committed tests. This is an assertion negative control, not comprehensive production fault-injection coverage.

The 32 native-X11 cases use actual OS input or owned client-pixel readback, not an IPC simulation of pointer/keyboard delivery. Native viewport tests compare glyph-bearing XGetImage crops through Outline/focus/palette/resize changes and replay actual undo/redo content afterward. Separate fresh visual captures on X11/tiny-skia were inspected in both themes for Columns, Outline, focus, palette, 720 px narrowing, and an actual held native drag; search render scenes were also inspected. Artifacts are under ignored `target/wrentest/`, including `final-inspection-dark-1au83232` and `final-inspection-light-ld49hwo4`. These are local observations, not native macOS or Wayland-input acceptance.

Final release workload measurements (20 samples per query/workload): in-memory build **38.14 ms**, retained original-plus-lowercase bytes **50,000,000**; median/max query milliseconds: Jump **0.896/1.163**, rare search **4.018/5.142**, scoped filter **0.319/0.857**, miss **3.409/3.871**. Every measured query was below 30 ms. Real-filesystem initial load was **117.40 ms**; median/max milliseconds were unchanged reconciliation **82.70/91.68**, same-size/restored-mtime refresh **82.97/86.11**, live-draft reconciliation **91.75/111.35**, queued edit service **0.247/0.275**, flush service **3.222/7.256**, and reconciliation-start queueing through persisted clean **95.06/118.87**. Query correctness, rank ties, UTF-8 ranges, and snippets are asserted. These warm-cache core timings do not satisfy a general 5 ms UI-I/O budget or establish native key-to-photon latency.

#### Definition-of-done gaps still open

This is a substantial usable desktop milestone, **not a claim that every spec/mockup detail is finished**. Watcher/poll reconciliation now runs off the UI owner (see the background reconciliation checkpoint below); structural operations, conflict-copy publication, Reveal, and the explicit `Op::Reconcile` still rescan synchronously. Toolkit visual seams still include the exact 52 px line-number gutter, current-line background, independent amber caret, and true completed-task strike-through. The wordmark uses bundled semibold rather than a distinct 700 face; ghosts are upright, and long labels clip rather than using the mockup's exact ellipsis treatment. Portable dialogs deliberately omit the broken software-renderer shadow.

Native macOS integration/Japanese IME, native Wayland input, actual folder-portal interaction, OS-trash and file-manager Reveal integration, interactive desktop theme changes, manual Obsidian interoperability, signing/notarization, and power-loss behavior remain unverified. The final compare/replace and OS-trash namespace limitations from the storage checkpoint still apply. Native picker/Reveal code and private completion adapters are not substitutes for those platform checks. New GitHub Actions definitions have been prepared and unit-checked, but only the historical tooling baseline has run remotely; this implementation pass has not been pushed.

### Prototype delivery and CI checkpoint

`make package` and `make package-debug` build normal native executables in a dedicated `target/package-build/<host>/` tree before assembling a new, uninstalled bundle. Existing outputs are never replaced. Linux gets `bin/wren`, a generated absolute-path desktop entry, scalable and hicolor PNG icons, font notices, and dependency notices; macOS assembly produces `Wren.app` with plist, executable mode, a multi-size ICNS, and resources, plus an `actool`-compiled macOS 26 icon catalog (`Assets.car`, `CFBundleIconName`) when a native host has full Xcode 26+. Every icon is derived from `assets/icon/wren-icon.svg` by `make icons` and committed; the runtime window icon embeds the 256 px PNG and the Linux window `application_id` is `wren`, matching `wren.desktop`. macOS layout is unit-tested with fixtures, not a locally executed macOS application. Linux desktop entry relocation requires regeneration; these bundles are not installed, signed, published, or claimed portable across Linux distributions.

The packager pins input ancestors and a private output staging container using directory descriptors, exclusively publishes the bundle, and never cleans up a substituted foreign alias. Tests cover source/parent replacement, late output collisions, ownership-preserving cleanup, permissions, format checks, malformed assets, and notice failures. A successful read-only Codex Astra delivery review led to fixes for archive executable-bit loss, stale smoke binaries under Cargo output overrides, foreign-stage cleanup, source-ancestor races, and unsupported `=` in Linux desktop Exec paths. The descriptor-bearing stage is retained because the exercised namespace races make path-only staging unsafe; no additional packaging framework was introduced.

The locked/offline Cargo collector inventories all non-workspace packages, including transitive/build/dev/other-target entries, and bundles actual local LICENSE/NOTICE/copyright texts rather than substituting an SPDX expression for a grant. The final local graph contains **507 packages and 804 collected texts; 66 packages still require notice review**, with two Wren workspace members deliberately excluded. Font notices are included separately. This is not redistribution clearance: source-text gaps, linked native/system components, and legal review remain open. No Wren project license has been invented or selected.

The actual release command was exercised with conflicting Cargo defaults:

```sh
CARGO_TARGET_DIR=target/not-the-bundle \
CARGO_BUILD_TARGET=aarch64-unknown-linux-gnu \
make package PACKAGE_OUTPUT=target/package/final-linux
```

It built a normal optimized **x86_64-unknown-linux-gnu** ELF at the explicit native target/target-dir and assembled `target/package/final-linux`. Repeating into `target/package/verified-linux` after removing unrelated resolver churn produced byte-identical executables and dependency notices (`cmp` passed). The executable's `--help` advertises ordinary workspace restoration and explicit unsaved evaluations. Archiving with `tar -czf target/package/final-linux.tar.gz -C target/package final-linux` and inspecting the result confirmed **21 members** and executable mode **0755** for `bin/wren`. The CI-facing `make package-debug PACKAGE_OUTPUT=target/package/debug-linux-acceptance` also built and assembled a normal native Linux bundle successfully, with the same notice counts.

A one-off native acceptance probe ran the packaged release, not the smoke binary, under each X11 renderer:

```sh
WREN_NORMAL_BIN="$PWD/target/package/final-linux/bin/wren" ICED_BACKEND=tiny-skia \
uv run --locked --group test python tools/run_display.py x11 -- \
uv run --locked --group test python /tmp/craze/wren_normal_acceptance.py
# Repeat with ICED_BACKEND=wgpu WGPU_BACKEND=vulkan.
```

Each run rejected both private test flags even with `WREN_TEST_MODE=1`, used isolated HOME/XDG directories and an owned window, typed natively, observed exact timer-saved disk bytes, flushed a final edit through WM_DELETE_WINDOW, and relaunched **without arguments** to restore the registered workspace and text. Both owned launches exited zero for each renderer. This probe is temporary local acceptance, not a committed CI test or native Wayland/macOS proof.

CI now assembles normal native debug bundles in the Linux/macOS Rust matrix and uploads tar archives so executable modes survive artifact storage. The existing fail-closed aggregate still requires Rust, harness units, and all four Linux renderer/display lanes. A real nightly/manual performance workflow executes both named release workloads through `make perf-search`, preserves their exit status, and uploads measurements even on failure. Python contracts check the explicit native target-dir, shared targets, artifact archive, unconditional aggregate, and scheduled workload wiring. Workflow execution on GitHub remains unverified for this pass because no push was requested.

The local checkpoints in this continuation are core/index/registrations `fef8410`, desktop navigation/layouts/gestures `1e9015d`, and the following prototype delivery/CI commit. The repository retains the original supplied spec/mockup; no release or additional git push is part of this handoff.

### Background reconciliation checkpoint

Watcher hints and the one-second poll no longer scan on the UI owner. With background reconciliation enabled (the desktop app enables it for every loaded workspace; `Op::Reconcile` and core tests without it stay synchronous), `Op::Tick` prepares a `ReconcileJob`: a duplicated root directory descriptor (`StoreReader`, the same descriptor-relative no-follow walk and per-access root/lock-name revalidation as `FsStore`, never the lock descriptor), a copy of the tree for ID preservation, and shared `Arc<str>` index text. A worker thread scans and reads notes, returning only notes whose exact bytes differ from the indexed text. `Op::FinishReconcile` merges in order on the owner: results from another Session (process-wide tickets), another generation, or an older job are dropped; the active note is re-read on the owner first, so dirty-draft conflicts preserve both versions exactly as before; a tree revision advanced by own creates/renames/moves/deletes or any synchronous rescan discards the result and requests a fresh scan; own saves and selection reads made while the job was in flight keep their newer metadata and index text. One job is in flight at a time; a job returned without a result reports an error and retries, and `Op::OpenWorkspace` releases an in-flight job that cannot be delivered while loading.

`make perf-search` on the 5,000-note / 25 MB fixture (median/max over 20 samples, warm caches): synchronous unchanged reconciliation 76.9/78.2 ms versus background owner cost (Tick snapshot plus apply) **1.95/2.41 ms** with a 101.2/110.2 ms worker scan; same-size external edit owner cost **2.26/2.73 ms**; live-draft owner cost **2.93/3.06 ms**. An edit applied while the worker scans took 0.39/0.41 ms and edit-plus-flush 4.5/14.1 ms, versus 106/126 ms queued behind synchronous reconciliation. These are core owner timings, not key-to-photon latency.
