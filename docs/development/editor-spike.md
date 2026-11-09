# Editor spike: Linux go, native macOS acceptance deferred

> Historical record of the M0 go/no-go decision on iced's editor. The
> editor has moved on since; see [Architecture](architecture.md) for how it
> fits today.

## Decision

**Go for the Linux prototype on iced 0.14.** The real editor passes typing, shortcuts, Unicode selection/paste, grouped undo/redo, literal find, wrapping, highlighting, and theme/scroll preservation checks. This is not a claim that native macOS Japanese IME or every desktop/input method has been accepted. Those checks are explicitly deferred by the approved plan.

The M0 checkpoint is an **in-memory evaluation**. It deliberately warns that its buffer is not saved. Filesystem persistence and autosave are the next checkpoint; the spike must not be used as a saved-notes application.

## Implementation

`EditorView` owns one iced `text_editor::Content` and exposes the editor actions independently of the application view. Ordinary rendering and theme changes do not clone or replace it. JetBrains Mono and Inter are bundled with their redistribution notices (`wren --font-licenses`). Semantic color tokens provide both light and dark palettes.

iced has no built-in undo stack here, so `EditorView` provides snapshot history. Adjacent insertions coalesce within one second; paste, newline, deletion, cursor movement, and focus/find changes break groups. No-op edits do not dirty history or destroy redo. Retained undo/redo/open-group snapshots are bounded together by 200 entries and 32 MiB of String capacity; live text and temporary edit snapshots are outside that budget. Oversize snapshots may clear history rather than create incorrect skipped states.

Find is literal, Unicode-lowercase matching with lowercase expansions mapped back to original UTF-8 byte ranges. It is not Unicode normalization or full case folding. Cursor columns in the diagnostic interface are UTF-8 byte offsets, not grapheme columns.

**Known viewport limitation:** stock iced does not expose an absolute editor scroll offset. Ordinary theme changes retain the viewport (verified with pixels), but undo/redo snapshot restoration does not restore an exact historical scroll offset. Cursor and selection are restored and clamped to valid UTF-8 boundaries. (Since lifted: undo and redo now apply a diff instead of reloading the content, which keeps the view on both engines, and the advanced engine restores the exact scroll.) Future note move/rename handlers must retain the current Content rather than reload it.

## Reproduction and results

On Linux with Rust 1.97.1, the debug build uses optimized dependency crates and software rendering. These commands run the shared regression suite, not a simulated editor:

```sh
make build check
ICED_BACKEND=wgpu WGPU_BACKEND=vulkan make smoke-x11
ICED_BACKEND=tiny-skia make smoke-x11
ICED_BACKEND=wgpu WGPU_BACKEND=vulkan make smoke-wayland
ICED_BACKEND=tiny-skia make smoke-wayland
```

The X11 lanes require `xdotool` and execute **21 tests each**. The Wayland lanes execute **17 tests each**, explicitly deselecting the four X11-native cases rather than reporting them as Wayland input coverage. The existing six renderer/CLI smoke cases are included in each count. CI installs `xdotool`, uses these same entry points, and retains screenshots, logs, measurements, and JUnit under `target/wrentest/`.

Editor checks cover:

- Real main-thread actions operating on Content: UTF-8, combining accents, emoji, selection, undo/redo, find navigation, no-op edits, and state retention through both themes.
- Actual editor screenshots in both themes, including heading/code syntax colors, multiple painted visual rows from a single logical line, and scrolled pixels surviving a dark/light/dark round trip.
- Native X11 typing, Ctrl+A/C/V, Ctrl+Z, Ctrl+Shift+Z, Ctrl+F, find input, and Escape. Japanese text is pasted through the real system clipboard; **this does not test Japanese IME composition**.
- Native X11 typing at the end of the 5,000-line fixture, checking the resulting editor text and capturing the rendered editor.

### 5,000-line measurements

The fixture has **5,000 logical lines / 334,999 UTF-8 bytes**. Sixteen independent one-character paste actions exercise the production edit handler. RPC measurements include main-thread scheduling, full document serialization/transfer, and client decoding; handler measurements include history and word-count work, **not rendering**.

| Display / renderer | RPC median / maximum | Handler median / maximum |
| --- | --- | --- |
| X11 / wgpu Vulkan | 13.74 / 42.20 ms | 6.75 / 9.94 ms |
| X11 / tiny-skia | 18.11 / 42.82 ms | 5.68 / 5.84 ms |
| Wayland / wgpu Vulkan | 14.26 / 55.74 ms | 6.61 / 14.48 ms |
| Wayland / tiny-skia | 19.18 / 58.36 ms | 5.81 / 6.45 ms |

The wgpu X11, wgpu Wayland, and tiny-skia Wayland runs were concurrent. Numbers are a local debug-build baseline, not a release benchmark or guaranteed latency ceiling. The shared-CI RPC regression ceiling is intentionally much looser (2 seconds) and must not be mistaken for an acceptable interactive target.

The same case also times six smart Enter and six smart Backspace presses (`editor.command`) at the end of the fixture, which is one 5,000-item list. On X11 / tiny-skia, a debug build measured handler medians of 5–11 ms for Enter and 4–6 ms for Backspace across the two engines. Most of that is one scan of the list by the Markdown planner. The test fails when either median passes 100 ms.

Native X11 injection of `native input` (12 characters with a requested 15 ms inter-key delay) reached the observed correct document state in **226.81 ms on wgpu** and **265.49 ms on tiny-skia**. These times include injection delay, process startup for xdotool, and state observation. A subsequent screenshot contains the edited document. They do not measure per-keystroke key-to-photon latency. No claim of sub-frame latency is made.

### Screenshot regression found and fixed

iced's immediate screenshot action can rasterize a previous scene after UI layout has invalidated the weak references to editor text. The resulting image contains the caret, borders, and surrounding widgets but omits editor glyphs. Tests detected this in both renderers.

The feature-gated capture adapter waits for iced's post-draw frame broadcast, then emits the screenshot action from the waiting task **without another App message/layout in between**. No iced fork or toolkit change is needed. Pixel assertions would fail again if editor text disappeared. The normal production build has neither the socket nor the capture adapter; the test build additionally requires `WREN_TEST_MODE=1` and an owned private socket directory.

## Review and regression follow-up

Independent local review reproduced a find crash on CR/LFCR documents, loss of native word/line selections through undo, retained selections after a programmatic deselect, and a failed-restart cleanup gap. All four were fixed and covered by regression tests. The final editor library has 103 passing tests; the feature-gated binary adds 18 tests, and the Python harness/CI units pass 143 tests. Native double/triple-click selection, deletion, undo, replacement, and redo now run in both X11 renderer lanes. All four line-ending forms run through the real editor in every display lane.

A CodeRabbit review was attempted but failed session/Git-provider authentication despite a successful authentication status check. It did not complete and is not counted as a clean review. The independent review and regression checks above are the review evidence for this checkpoint.

## Still requiring platform/manual acceptance

- Native macOS Japanese IME composition, candidate selection, cancellation, and committed text.
- Native macOS Cmd shortcuts, clipboard ownership, and input behavior.
- Broader Linux IME/accessibility and desktop integration checks beyond the tested X11 keyboard/clipboard path.
- Exact historical viewport restoration across undo/redo is not supported by the stock editor API. (Since lifted by diff-applied undo, with the exact scroll on the advanced engine.)

Initial repository CI separately passed Linux/macOS compilation and unit checks on the tooling baseline. That does not establish native macOS integration or validate later unpushed changes.
