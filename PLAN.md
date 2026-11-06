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

The selected root must already exist. Sections are its immediate non-hidden directories; groups are directories inside sections. Use the creation dialogs for an empty workspace. Launching without `--workspace` deliberately remains the clearly marked, unsaved editor evaluation; `--editor-spike` still supplies its 5,000-line in-memory fixture.

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
