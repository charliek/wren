//! Wren: trackpad pinches, which iced's `Event` cannot carry.
//!
//! winit reports a pinch as `WindowEvent::PinchGesture` (macOS and iOS
//! only), and iced 0.14 drops it. Instead of putting it in iced's event
//! stream, where every widget would see it as some other input, the run
//! loop records each pinch for its window ([`record`]) and asks that
//! window to redraw. While the window's next redraw updates its interface,
//! and only then, [`take`] hands out what was recorded for it; whatever no
//! widget took is dropped when that update ends. So a pinch reaches only a
//! widget of the window it happened in, at the first frame after it, and
//! the widget decides with the pointer of that frame where it applies.
//!
//! The loop runs on one thread, so the record is a thread-local. Nothing
//! here runs on a platform whose winit sends no pinch, unless a test calls
//! [`record`] itself.

use std::cell::RefCell;

use crate::core::window::Id;

thread_local! {
    /// Pinches not yet in a frame, in order, each with its window.
    static RECORDED: RefCell<Vec<(Id, f64)>> = const { RefCell::new(Vec::new()) };
    /// The pinches of the frame being updated, until a widget takes them.
    static FRAME: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
}

/// How many pinches wait for a frame at most. A frame follows each one, so
/// more than a few waiting means no frame is coming; the oldest go.
pub const WAITING: usize = 64;

/// Records a pinch on `window`: `delta` is the change in scale winit
/// reports (AppKit's `magnification`), positive to zoom in. It is kept as
/// given; the reader decides what is not a number.
pub fn record(window: Id, delta: f64) {
    RECORDED.with(|recorded| {
        let mut recorded = recorded.borrow_mut();
        if recorded.len() >= WAITING {
            let _ = recorded.remove(0);
        }
        recorded.push((window, delta));
    });
}

/// The pinches of the frame being updated, in order, taken: a second call
/// in the same frame, or a call outside a frame, finds none.
pub fn take() -> Vec<f64> {
    FRAME.with(|frame| std::mem::take(&mut *frame.borrow_mut()))
}

/// Drops what was recorded for a window that has closed, or whose frame
/// was not drawn.
pub fn forget(window: Id) {
    RECORDED.with(|recorded| recorded.borrow_mut().retain(|(id, _)| *id != window));
}

/// One window's frame: from [`Frame::begin`] until it is dropped, [`take`]
/// hands out the pinches recorded for that window before it began.
#[must_use = "the frame ends when it is dropped"]
pub struct Frame(());

impl Frame {
    /// Begins `window`'s frame, moving its recorded pinches into it and
    /// leaving other windows' where they are.
    pub fn begin(window: Id) -> Self {
        let pinches = RECORDED.with(|recorded| {
            let mut recorded = recorded.borrow_mut();
            let mut ours = Vec::new();
            recorded.retain(|(id, delta)| {
                let mine = *id == window;
                if mine {
                    ours.push(*delta);
                }
                !mine
            });
            ours
        });
        FRAME.with(|frame| *frame.borrow_mut() = pinches);
        Self(())
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        FRAME.with(|frame| frame.borrow_mut().clear());
    }
}
