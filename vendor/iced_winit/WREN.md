# Vendored iced_winit (Wren)

Upstream [iced_winit](https://crates.io/crates/iced_winit) 0.14.1 from
crates.io (iced git `38237dd294da3256c2ac4d3f8288fc73956c91e0`, `winit/`),
the version iced 0.14 uses, with three changes. `Cargo.toml` and
`README.md` are the crates.io ones, and `LICENSE` is the MIT license from
that upstream revision; `Cargo.lock`, `Cargo.toml.orig` and
`.cargo_vcs_info.json` are not kept. Only `src/` differs, and each change
is marked there with a `Wren:` comment. Against crates.io the source diff
adds about 130 lines and removes 17.

## 1. `try_recv` for `try_next` (`src/lib.rs`)

The futures crate deprecates `Receiver::try_next`, which upstream calls
twice: once for the control channel in the winit handler, once for the
event channel in `run_instance`. Both call `try_recv` instead, so a build
that denies warnings passes; an empty channel still falls through to
waiting, as before.

## 2. Screenshots of a presented frame (`src/lib.rs`, `src/window.rs`)

Upstream's `window::Action::Screenshot` reads the window back at once,
replaying the last primitives; after a layout in between, the editor's
weak text buffers had expired and the capture showed a caret but no
glyphs, and rebuilding the interface for the capture reset hovered and
focused widgets' styles. Instead the action queues its sender on the
window (`Window::pending_screenshots`) and asks for a redraw; right after
that frame is presented, one readback answers every queued request.
Cancelled requests are pruned, a closed window drops its senders, and a
surface error or a zero-size window answers nothing rather than a false
capture. The test harness's `app.screenshot` relies on it.

## 3. Trackpad pinches (`src/pinch.rs`, `src/lib.rs`)

winit 0.30 reports a trackpad pinch on macOS (and iOS) as
`WindowEvent::PinchGesture { delta, phase, .. }`, `delta` being AppKit's
`magnification` for the event. iced 0.14's `Event` has no pinch, and
`conversion::window_event` drops it. `iced_core` is not vendored, so there
is no variant to add, and a pinch reaches Wren's board widget through a
side channel instead:

* On a `PinchGesture` the run loop records `(window, delta)`
  (`pinch::record`) and asks that window to redraw. The phase is not kept:
  a pinch's end carries no change of scale.
* In that window's next `RedrawRequested`, around the interface updates of
  the redraw (the loop that may run the program's `update` and update the
  interface again), a `pinch::Frame` guard moves the window's recorded
  pinches into the frame. During it, `pinch::take` hands them out once;
  when the guard drops, whatever no widget took is dropped too.
* `window::Action::Close` forgets what was recorded for the window, and
  so does a redraw skipped because the window has no size. At most
  `pinch::WAITING` (64) pinches wait for a frame; past that the oldest
  go, so a window that never redraws cannot grow the record.

So a pinch reaches only a widget of the window it happened in, at the
first frame after it, and that widget decides with that frame's pointer
whether it applies. Wren's board widget does not name this crate: the app
hands it `pinch::take` as the place to read pinches from
(`BoardView::set_pinches`), and the widget reads it at each redraw, turning
each delta into a zoom about the pointer while the pointer is over the
board and dropping it otherwise. Nothing else changes: every other event
takes the path it took, on every platform, and on a platform whose winit
sends no pinch the record stays empty.

Rejected, and why:

* *An existing `Event` as a carrier* (a wheel step with ⌘, a touch event
  with a reserved finger, an input-method event): every widget sees it as
  that input, so scrollables would scroll and buttons would release.
* *A subscription carrying the pinch to the app*, as the macOS menu does:
  it arrives after the frame, so after later pointer events, and the app
  does not know where the board is in the window; the widget would have to
  report its bounds on every platform.
* *A global the widget may read at any time*: nothing would tie a pinch to
  its window or to the frame after it.
* *A new `Event` variant*: it means vendoring `iced_core` and every crate
  that matches on `Event`.

The test harness's `board.pinch` calls `pinch::record` for the app's
window, as the run loop does on macOS, so the functional suite drives the
path from the record on, on any platform. The tests:
`crates/wren-iced/src/app/board.rs`
(`the_pinch_record_is_one_windows_for_one_frame`),
`crates/wren-canvas/src/widget.rs` (`a_frames_pinches_*`,
`a_pinch_off_the_board_is_dropped`) and
`tools/wrentest/test_board_native.py`
(`test_native_a_recorded_pinch_zooms_about_the_real_pointer`).

winit also reports a two-finger double tap (`DoubleTapGesture`, AppKit's
smart magnify); it is not recorded.

## Updating

The workspace `[patch.crates-io]` entry wires this in. A patch only shadows
the version it matches, so `make check` asserts that iced_winit still
resolves here. When iced moves to a newer iced_winit, re-apply the changes
above to that release (the redraw branch of `run_instance` is the one place
with real logic), or drop one that upstream has made unnecessary: change 2
if its screenshots capture a presented frame, change 3 if iced's `Event`
gains a pinch.
