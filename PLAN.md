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
