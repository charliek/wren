//! The board canvas: an iced widget that paints a [`wren_board::scene`].
//!
//! Like `wren-editor`, it knows no Wren types and no app messages. The host
//! owns a [`BoardView`] per open board, passes in the board's text, a
//! [`Palette`] and the label font, and mounts [`BoardView::view`] under its
//! own focus id. Everything a board shows lives in the view, so a host or a
//! test can read and drive it with no window, and a rebuilt widget tree
//! loses nothing.
//!
//! How a scene reaches the screen was settled by measurement (the plan's
//! spike record, `discovery/boards.md` §8.5):
//!
//! - [`BoardView`] parses the text, builds the scene once with a
//!   [`Measurer`] over the renderer's own text stack, and splits it into
//!   renderer layers ([`layers`]) so a label never shows through a shape
//!   drawn over it.
//! - The widget builds geometry in widget-local pixels for the items in
//!   view, each point already through the [`Viewport`] and each size
//!   already times the zoom, and keeps it until the scene, the viewport,
//!   the widget's size or the label rule's outcome changes. An idle board
//!   builds nothing.
//! - The software renderer blits every glyph of every frame, so there the
//!   widget draws no label while the view moves and none too small to
//!   read ([`Labels`]).
//!
//! Input reaches the view as [`Message`]s, which [`BoardView::update`]
//! runs through the pure [`Machine`]: the wheel pans, or zooms with ⌘ or
//! Ctrl held, and a middle-button or Space drag pans. Zoom and pan are view
//! state, never the board's. The machine also keeps the selection, the
//! armed tool and the pointer gesture in flight, finds what a press lands
//! on with [`hit`] and [`handles`], and answers with [`machine::Act`]s for
//! the view to carry out: it plans and applies each edit the machine asks
//! for, shows a gesture's preview on a scratch copy of the board, and keeps
//! the pristine text a draft falls back to (§3.4 of the plan). The one
//! label that is open is the view's too ([`Label`]): its keys arrive as
//! [`LabelAction`]s, and each change of its text is an edit of the board
//! at once (§5.9). With the `test-support` feature a view counts its scene
//! and geometry builds (`Probe`).

use std::collections::BTreeSet;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iced::{Element, Font};
use wren_board::edit::{self, Intent};
use wren_board::ids::Ids;
use wren_board::scene::{self, Draw, Rect, Scene, Source};
use wren_board::{
    style, walk, Access, Board, Dir, Fill, Id, Issue, Named, NodeKind, Palette, ParseError, Rgba,
    GRID,
};

mod clip;
mod command;
#[cfg(test)]
mod fixtures;
pub mod handles;
pub mod hit;
mod keys;
pub mod label;
pub mod layers;
pub mod machine;
mod measure;
mod overlay;
mod paint;
mod widget;

pub use command::{Command, Grow, Tool};
pub use keys::{KeyPolicy, KeyRoute};
pub use label::{Label, LabelAction, Motion, Preedit};
pub use machine::{Button, Input, Key, Machine, Mods, Scroll};
pub use measure::Measurer;

/// How long after its last pan or zoom step a view counts as at rest.
pub const REST: Duration = Duration::from_millis(150);

/// Space left around a board's content when it is fitted to the widget,
/// in screen pixels.
pub const FIT_MARGIN: f32 = 40.0;

/// The zooms the keyboard steps through.
pub const ZOOM_STOPS: [f32; 9] = [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0];

/// Room kept around the cursor element when a keyboard selection change
/// brings it into view, in screen pixels (§5.7 rule 5).
pub const REVEAL_MARGIN: f32 = 40.0;

/// A label whose text would be smaller than this on screen, in logical
/// pixels, is opened at 100 % instead (§5.9 rule 2).
pub const MIN_LABEL_PX: f32 = 10.0;

/// What the user is told when a label is full ([`scene::MAX_LABEL_BYTES`]).
pub const LABEL_FULL: &str = "A label holds at most 16 KiB of text.";

/// Where a board sits in the widget: the board point at the widget's
/// top-left corner, and how many screen pixels a board unit takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Viewport {
    pub const MIN_ZOOM: f32 = 0.1;
    pub const MAX_ZOOM: f32 = 4.0;

    /// A widget-local point in board units.
    pub fn to_board(self, screen: (f32, f32)) -> (f32, f32) {
        (self.x + screen.0 / self.zoom, self.y + screen.1 / self.zoom)
    }

    /// A board point in widget-local pixels.
    pub fn to_screen(self, board: (f32, f32)) -> (f32, f32) {
        (
            (board.0 - self.x) * self.zoom,
            (board.1 - self.y) * self.zoom,
        )
    }

    /// Moved by a distance in screen pixels: a positive `dx` shows what is
    /// further right.
    pub fn panned(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx / self.zoom,
            y: self.y + dy / self.zoom,
            ..self
        }
    }

    /// Zoomed by `factor`, clamped, keeping the board point under the
    /// widget-local point `about` where it is.
    pub fn zoomed(self, factor: f32, about: (f32, f32)) -> Self {
        self.zoomed_to(self.zoom * factor, about)
    }

    /// At `zoom`, clamped, keeping the board point under the widget-local
    /// point `about` where it is. A zoom the clamp leaves unchanged is no
    /// move at all.
    pub fn zoomed_to(self, zoom: f32, about: (f32, f32)) -> Self {
        let zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        if zoom == self.zoom {
            return self;
        }
        let (x, y) = self.to_board(about);
        Self {
            x: x - about.0 / zoom,
            y: y - about.1 / zoom,
            zoom,
        }
    }

    /// The view of a widget of `size` that shows all of `content` with
    /// [`FIT_MARGIN`] around it, centred, never above 100 %. Content with
    /// no area is shown at 100 %, centred on its corner.
    pub fn fitting(content: scene::Rect, size: (f32, f32)) -> Self {
        let room = |widget: f32, content: f32| {
            if content > 0.0 {
                (widget - 2.0 * FIT_MARGIN) / content
            } else {
                f32::INFINITY
            }
        };
        let zoom = room(size.0, content.width)
            .min(room(size.1, content.height))
            .min(1.0)
            .clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        Self {
            x: content.x + content.width / 2.0 - size.0 / 2.0 / zoom,
            y: content.y + content.height / 2.0 - size.1 / 2.0 / zoom,
            zoom,
        }
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

/// Which labels a frame holds. Shapes are cheap on both renderers; glyphs
/// are what the software renderer spends its frame on once much of a
/// board is in view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Labels {
    /// Text smaller than this on screen, in logical pixels, is not drawn.
    pub min_px: f32,
    /// Whether labels are drawn while the view moves, which is until it
    /// has been still for [`REST`].
    pub moving: bool,
}

impl Labels {
    pub const ALWAYS: Self = Self {
        min_px: 0.0,
        moving: true,
    };

    /// What the software renderer was measured to need: nothing under
    /// 4 px, and nothing while the view moves.
    pub const SOFTWARE: Self = Self {
        min_px: 4.0,
        moving: false,
    };

    /// The rule for the renderer iced reports by this name.
    pub fn for_renderer(name: &str) -> Self {
        if name == "tiny-skia" {
            Self::SOFTWARE
        } else {
            Self::ALWAYS
        }
    }
}

/// How the widget presents a board, beyond the board's own colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Which labels are drawn; `None` takes the renderer's own rule
    /// ([`Labels::for_renderer`]).
    pub labels: Option<Labels>,
    /// The window's scale factor, which iced does not give a widget:
    /// precise wheel deltas arrive in physical pixels.
    pub scale_factor: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            labels: None,
            scale_factor: 1.0,
        }
    }
}

/// What the widget tells its host, which hands it to
/// [`BoardView::update`].
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// An input, stamped with the [`BoardView::epoch`] it was meant for.
    Input(u64, Input),
    /// The widget's size in logical pixels, sent when it differs from the
    /// one the view knows.
    Resized { width: f32, height: f32 },
    /// An edit of the open label, stamped with the epoch it was meant for.
    Label(u64, LabelAction),
    /// A key the board layer binds. The host runs it through its own
    /// command path, which reaches [`BoardView::command`].
    Command(Command),
    /// The clipboard's text, read for an [`Effect::ClipboardRead`] and
    /// stamped with the epoch of the view that asked.
    Pasted(u64, String),
}

/// What a message did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update {
    /// Whether the board itself changed. Looking at it from elsewhere is
    /// not a change.
    pub changed: bool,
    /// What the host has to do.
    pub effects: Vec<Effect>,
}

/// What a board asks for. [`BoardView::update`] applies [`Effect::Look`]
/// itself; the rest reach the host in [`Update::effects`].
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Look at the board from here.
    Look(Viewport),
    /// Put this text on the system clipboard.
    ClipboardWrite(String),
    /// Read the system clipboard and send its text back as
    /// [`Message::Pasted`], stamped with the view's epoch.
    ClipboardRead,
    /// Open the actions palette for the selection.
    OpenActions,
    /// Open the jump palette.
    OpenJump,
    /// Tell the user why something did not happen, such as a paste of a
    /// newer board.
    Notice(String),
}

/// A revision no view has had: geometry built for one scene is never
/// taken for another's, even when the host swaps the view under a widget.
fn revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// One open board: its model, what is wrong with it, the scene that draws
/// it and where the widget looks at it.
pub struct BoardView {
    board: Board,
    issues: Vec<Issue>,
    access: Access,
    synced: String,
    /// The text the board was opened or last reloaded from, which a draft
    /// is while the board still writes as `canonical` (§3.4 rule 2). A
    /// save does not replace it: it is the undo floor (§5.11 rule 5).
    pristine: String,
    /// What the canonical writer makes of the pristine text's board.
    canonical: String,
    palette: Palette,
    measurer: Measurer,
    scene: Scene,
    plan: layers::Plan,
    cap: usize,
    /// The board as a gesture in flight would leave it, shown in place of
    /// the board until the gesture ends (§3.4 rule 4).
    preview: Option<Preview>,
    ids: Fresh,
    viewport: Viewport,
    revision: u64,
    machine: Machine,
    /// Bumped whenever the board changes underneath its input, so input
    /// meant for the board before is dropped.
    epoch: u64,
    /// The widget's size, once it has said.
    size: Option<(f32, f32)>,
    /// A fit asked for before the widget's size was known.
    fit_pending: bool,
    /// Bumped by each discrete change of view (a zoom step, a reset, a
    /// fit), which the widget draws in full at once instead of as a move.
    jumps: u64,
    key_policy: Option<KeyPolicy>,
    /// The open label (§5.9): the machine says whose it is
    /// ([`Machine::editing`]), the view holds its text.
    label: Option<Label>,
    /// The node an edit in progress made to be labelled.
    making: Option<Id>,
    /// Every intent planned, for tests that say what an input asked for
    /// whatever the plan makes of it.
    #[cfg(test)]
    intents: Vec<Intent>,
    #[cfg(feature = "test-support")]
    pub probe: Probe,
}

/// A gesture's picture of the board: a scratch copy with the gesture's
/// plan applied, and its scene. It is never the model, never a draft and
/// never hit-tested.
struct Preview {
    board: Board,
    scene: Scene,
    plan: layers::Plan,
    revision: u64,
}

/// What the widget paints: the board, or a gesture's preview of it.
pub(crate) struct Picture<'a> {
    pub(crate) board: &'a Board,
    pub(crate) scene: &'a Scene,
    pub(crate) plan: &'a layers::Plan,
    pub(crate) revision: u64,
}

/// Candidate ids for what an edit creates (§4.2): the process's random
/// hash keys applied to a counter, spelt in the id alphabet. The plan
/// draws again while a candidate is taken.
#[derive(Default)]
struct Fresh {
    keys: RandomState,
    count: u64,
}

impl Ids for Fresh {
    fn draw(&mut self) -> Id {
        const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
        let mut hasher = self.keys.build_hasher();
        hasher.write_u64(self.count);
        self.count = self.count.wrapping_add(1);
        let bits = hasher.finish();
        let text: String = (0..8)
            .rev()
            .map(|k| char::from(ALPHABET[((bits >> (5 * k)) & 31) as usize]))
            .collect();
        Id::new(&text).expect("eight characters of the id alphabet are an id")
    }
}

impl BoardView {
    /// A view of the board `text` holds, at 100 % with the board's origin
    /// at the widget's corner. Labels are measured in `font`, which the
    /// host has loaded.
    pub fn open(text: &str, palette: Palette, font: Font) -> Result<Self, ParseError> {
        let board = wren_board::parse(text)?;
        #[cfg(feature = "test-support")]
        let probe = Probe::default();
        #[cfg(feature = "test-support")]
        probe.parses.set(1);
        let mut view = Self {
            issues: wren_board::check(&board),
            access: wren_board::access(&board),
            canonical: wren_board::write(&board),
            board,
            synced: text.to_owned(),
            pristine: text.to_owned(),
            palette,
            measurer: Measurer::new(font),
            scene: Scene {
                bounds: scene::Rect::default(),
                items: Vec::new(),
            },
            plan: layers::Plan::default(),
            cap: layers::CAP,
            preview: None,
            ids: Fresh::default(),
            viewport: Viewport::default(),
            revision: 0,
            machine: Machine::default(),
            epoch: 0,
            size: None,
            fit_pending: false,
            jumps: 0,
            key_policy: None,
            label: None,
            making: None,
            #[cfg(test)]
            intents: Vec::new(),
            #[cfg(feature = "test-support")]
            probe,
        };
        view.build();
        Ok(view)
    }

    /// The text changed underneath the view (§3.4 rule 5). It parses first
    /// and changes nothing if that fails; otherwise the board is replaced
    /// and becomes the pristine one, the viewport is kept, a gesture in
    /// flight and an open label are dropped without a commit, the
    /// selection keeps what still exists, and the epoch moves on.
    pub fn reload(&mut self, text: &str) -> Result<(), ParseError> {
        if text == self.synced {
            return Ok(());
        }
        #[cfg(feature = "test-support")]
        self.probe.parses.set(self.probe.parses.get() + 1);
        let board = wren_board::parse(text)?;
        self.issues = wren_board::check(&board);
        self.access = wren_board::access(&board);
        self.canonical = wren_board::write(&board);
        self.board = board;
        text.clone_into(&mut self.synced);
        text.clone_into(&mut self.pristine);
        self.preview = None;
        self.label = None;
        self.machine.reset();
        self.machine.retain(&self.board);
        self.epoch += 1;
        self.build();
        Ok(())
    }

    /// The Session text this view last took or sent.
    pub fn synced(&self) -> &str {
        &self.synced
    }

    /// A theme change: one scene build, and none for the same colours.
    pub fn set_palette(&mut self, palette: Palette) {
        if palette != self.palette {
            self.palette = palette;
            self.build();
            if let Some(preview) = self.preview.take() {
                self.preview = Some(self.picture_of(preview.board));
            }
            self.place_label();
        }
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// The board as committed: never a gesture's preview.
    pub fn board(&self) -> &Board {
        &self.board
    }

    /// The committed board's scene.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The scene the widget shows: a gesture's preview while one is in
    /// flight, else the board's own.
    pub fn shown(&self) -> &Scene {
        self.preview
            .as_ref()
            .map_or(&self.scene, |preview| &preview.scene)
    }

    /// What is wrong with the board that still lets it draw.
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    pub fn access(&self) -> &Access {
        &self.access
    }

    /// Whether the board may be edited: a read-only one still selects,
    /// pans and zooms (§5.2).
    pub fn writable(&self) -> bool {
        self.access == Access::Editable
    }

    /// Whether the board writes as the pristine text's board does, so a
    /// draft now would be the pristine text itself.
    pub fn pristine(&self) -> bool {
        wren_board::write(&self.board) == self.canonical
    }

    /// The renderer layer of each scene item.
    pub fn layers(&self) -> &layers::Plan {
        &self.plan
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Look at the board from `viewport`, its zoom brought within 10 % to
    /// 400 %. A viewport that is not finite is not taken. It replaces a fit
    /// still waiting for the widget's size.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        if viewport.x.is_finite() && viewport.y.is_finite() && viewport.zoom.is_finite() {
            self.viewport = Viewport {
                zoom: viewport.zoom.clamp(Viewport::MIN_ZOOM, Viewport::MAX_ZOOM),
                ..viewport
            };
            self.fit_pending = false;
        }
    }

    /// Step `by` zoom stops ([`ZOOM_STOPS`]) about the widget's centre:
    /// up for a positive `by`, down for a negative one. A zoom between two
    /// stops goes to the next one in that direction.
    pub fn zoom_step(&mut self, by: i8) {
        let zoom = self.viewport.zoom;
        // A stop within a hair of the zoom is the zoom's own.
        let near = |stop: f32| (stop - zoom).abs() <= stop * 1e-4;
        let target = match by.signum() {
            1 => ZOOM_STOPS
                .iter()
                .position(|&stop| stop > zoom && !near(stop))
                .map(|first| first + usize::from(by.unsigned_abs()) - 1),
            -1 => ZOOM_STOPS
                .iter()
                .rposition(|&stop| stop < zoom && !near(stop))
                .map(|last| last.saturating_sub(usize::from(by.unsigned_abs()) - 1)),
            _ => return,
        };
        let stop = target.map_or(zoom, |index| ZOOM_STOPS[index.min(ZOOM_STOPS.len() - 1)]);
        self.jump(self.viewport.zoomed_to(stop, self.centre()));
    }

    /// Back to 100 % about the widget's centre.
    pub fn zoom_reset(&mut self) {
        self.jump(self.viewport.zoomed_to(1.0, self.centre()));
    }

    /// Show all of the board with [`FIT_MARGIN`] around it, never above
    /// 100 %; an empty board at 100 % with its origin in the middle. Before
    /// the widget has said its size the fit waits for it, and the widget
    /// draws no item until then.
    pub fn fit(&mut self) {
        match self.size {
            Some(size) => {
                self.fit_pending = false;
                self.jump(Viewport::fitting(self.scene.bounds, size));
            }
            None => self.fit_pending = true,
        }
    }

    /// Apply a message from the widget at `now`. Input stamped with an
    /// earlier epoch is dropped.
    pub fn update(&mut self, message: Message, now: Instant) -> Update {
        let mut update = Update::default();
        match message {
            Message::Input(epoch, input) if epoch == self.epoch => {
                #[cfg(feature = "test-support")]
                if let Input::Wheel { delta, .. } = input {
                    self.probe.wheeled(delta);
                }
                let view = machine::View {
                    viewport: self.viewport,
                    now,
                    board: &self.board,
                    scene: &self.scene,
                    writable: self.access == Access::Editable,
                };
                let acts = self.machine.step(input, &view);
                self.carry(acts, now, &mut update);
                self.keep_label();
            }
            Message::Label(epoch, action) if epoch == self.epoch => {
                self.label_action(action, now, &mut update);
            }
            Message::Pasted(epoch, text) if epoch == self.epoch => {
                // Outside a label a paste is the board's, which the host
                // wires with the clipboard.
                if let Some(label) = &mut self.label {
                    let outcome = label.paste(&text, &self.measurer);
                    self.label_did(&outcome, now, &mut update);
                }
            }
            Message::Resized { width, height } => {
                if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
                    self.size = Some((width, height));
                    if self.fit_pending {
                        self.fit();
                    }
                }
            }
            Message::Command(command) => return self.command(command, now),
            Message::Input(..) | Message::Label(..) | Message::Pasted(..) => {}
        }
        update
    }

    /// Run a board command at `now`, from a key, a palette row, a menu
    /// item or the harness (§5.1 rule 5). An open label is committed
    /// first. A command that moves the cursor, or selects it, brings it
    /// into view (§5.7 rule 5).
    pub fn command(&mut self, command: Command, now: Instant) -> Update {
        let mut update = Update::default();
        let cursor = self.machine.cursor().cloned();
        let selection = self.machine.selection().clone();
        let view = machine::View {
            viewport: self.viewport,
            now,
            board: &self.board,
            scene: &self.scene,
            writable: self.access == Access::Editable,
        };
        let acts = self.machine.command(command, &view);
        self.carry(acts, now, &mut update);
        self.keep_label();
        let machine = &self.machine;
        let selects = machine
            .cursor()
            .is_some_and(|cursor| machine.selection().contains(cursor));
        if machine.cursor() != cursor.as_ref() || (selects && *machine.selection() != selection) {
            self.reveal();
        }
        update
    }

    /// Before any operation that is not passive (§3.4 rule 3): commit an
    /// open label, cancel a pointer gesture and keyboard Connecting, and
    /// drop a preview.
    pub fn settle(&mut self, now: Instant) -> Update {
        let mut update = Update::default();
        let acts = self.machine.settle();
        self.carry(acts, now, &mut update);
        self.keep_label();
        self.preview = None;
        update
    }

    /// The Session saved `text`. The pristine text stays the one the board
    /// was opened or reloaded from: it is the undo floor, so undoing to it
    /// after a save drafts the original bytes again and the Session saves
    /// them back (§5.11 rule 5). The Session marks a draft clean against
    /// its own saved bytes, so nothing here needs the saved text.
    pub fn saved(&mut self, _text: &str) {}

    /// The text to send as the board's draft, by the pristine rule (§3.4
    /// rule 2): the pristine text while the board writes as the board it
    /// holds, otherwise the canonical text. It becomes the synced text.
    pub fn draft(&mut self) -> String {
        let out = wren_board::write(&self.board);
        let text = if out == self.canonical {
            self.pristine.clone()
        } else {
            out
        };
        self.synced.clone_from(&text);
        text
    }

    /// The host's say over each key the focused board is given (§5.1);
    /// without one, a key with Ctrl, Alt or the logo key is the host's and
    /// every other key the board's.
    pub fn set_key_policy(&mut self, policy: KeyPolicy) {
        self.key_policy = Some(policy);
    }

    pub(crate) fn key_policy(&self) -> Option<&KeyPolicy> {
        self.key_policy.as_ref()
    }

    /// Which board the view's input is for; see [`Message::Input`].
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The widget's size in logical pixels, once it has said.
    pub fn size(&self) -> Option<(f32, f32)> {
        self.size
    }

    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    /// The open label, while the machine is editing one (§5.9).
    pub fn label(&self) -> Option<&Label> {
        self.label.as_ref()
    }

    /// The box around the open label's lines, in board units.
    pub fn label_box(&self) -> Option<Rect> {
        let label = self.label.as_ref()?;
        Some(label.bounds(&self.measurer))
    }

    /// The open label's caret in widget-local pixels, a box of no width,
    /// and the size its text has there: where the input method puts its
    /// candidates and how large it shows a composition (§5.9 rule 9).
    pub fn label_caret(&self) -> Option<(Rect, f32)> {
        let label = self.label.as_ref()?;
        let caret = label.caret_box(&self.measurer);
        let zoom = self.viewport.zoom;
        let (x, y) = self.viewport.to_screen((caret.x, caret.y));
        Some((
            Rect::new(x, y, 0.0, caret.height * zoom),
            label.text_size() * zoom,
        ))
    }

    /// The widget for this view, focusable under the host's `id`.
    pub fn view(&self, id: &'static str, style: Style) -> Element<'_, Message> {
        widget::Canvas::new(self, id, style).into()
    }

    /// Whether a [`fit`](Self::fit) still waits for the widget's size, so
    /// [`viewport`](Self::viewport) is not yet where the board will be
    /// looked at from.
    pub fn fit_pending(&self) -> bool {
        self.fit_pending
    }

    pub(crate) fn jumps(&self) -> u64 {
        self.jumps
    }

    /// The middle of the widget, or its corner before it has a size.
    fn centre(&self) -> (f32, f32) {
        self.size
            .map_or((0.0, 0.0), |(width, height)| (width / 2.0, height / 2.0))
    }

    fn jump(&mut self, viewport: Viewport) {
        if viewport != self.viewport {
            self.set_viewport(viewport);
            self.jumps += 1;
        }
    }

    /// Changes whenever the scene or its layers do.
    #[cfg(test)]
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// What the widget paints.
    pub(crate) fn picture(&self) -> Picture<'_> {
        match &self.preview {
            Some(preview) => Picture {
                board: &preview.board,
                scene: &preview.scene,
                plan: &preview.plan,
                revision: preview.revision,
            },
            None => Picture {
                board: &self.board,
                scene: &self.scene,
                plan: &self.plan,
                revision: self.revision,
            },
        }
    }

    pub(crate) fn measurer(&self) -> &Measurer {
        &self.measurer
    }

    /// Carry out what the machine asked for, in order.
    fn carry(&mut self, acts: Vec<machine::Act>, now: Instant, update: &mut Update) {
        use machine::Act;
        for act in acts {
            match act {
                Act::Look(viewport) => self.set_viewport(viewport),
                // The undo history tells an edit from one that continues
                // a run; each is planned and applied alike.
                Act::Edit(intent) | Act::Continue(intent) => self.edit(&intent, now, update),
                Act::Preview(intent) => self.show(intent.as_ref()),
                Act::OpenLabel(id) => self.open_label(id),
                Act::CommitLabel => self.commit_label(now, update),
                Act::Caret { at, select } => {
                    if let Some(label) = &mut self.label {
                        label.point(at, select, &self.measurer);
                    }
                }
                Act::Run(command) => self.run(command, now, update),
            }
        }
    }

    /// Plan `intent` on the board and apply it, once (§3.4 rule 4). A
    /// preview on show is over. The machine takes the plan back, which is
    /// how a creation selects what it made.
    fn edit(&mut self, intent: &Intent, now: Instant, update: &mut Update) {
        self.preview = None;
        #[cfg(test)]
        self.intents.push(intent.clone());
        let plan = edit::plan(&self.board, intent, &mut self.ids);
        if !plan.is_empty() {
            self.board.apply(&plan.changes);
            self.issues = wren_board::check(&self.board);
            self.build();
            update.changed = true;
        }
        if let Some(notice) = &plan.notice {
            update.effects.push(Effect::Notice(notice.clone()));
        }
        let acts = self.machine.applied(&plan, &self.board);
        self.making.clone_from(&plan.edit);
        self.carry(acts, now, update);
        self.making = None;
    }

    /// Open the label editor on `id` (§5.9 rules 1 and 2): all of an
    /// existing label selected, and the view brought to where the text can
    /// be read. An element the scene gives no label a place, which is a
    /// connector it does not route, is not edited.
    fn open_label(&mut self, id: Id) {
        let text = match (self.board.nodes.get(&id), self.board.edges.get(&id)) {
            (Some(node), _) => Some(node.text.as_str()),
            (None, Some(edge)) => Some(edge.labels.first().map_or("", |label| label.text.as_str())),
            (None, None) => None,
        };
        let (Some(text), Some((place, ink))) = (text, self.label_place(&id)) else {
            self.label = None;
            self.machine.abandon();
            return;
        };
        let mut label = Label::open(id.clone(), text, place, &self.measurer);
        label.ink = ink;
        label.size = self.board.nodes.get(&id).map(|node| node.size);
        label.made = self.making.as_ref() == Some(&id);
        // What has to be in view: the label, and with it the node it is on.
        let lines = label.bounds(&self.measurer);
        let node = hit::Frames::new(&self.board).get(&id);
        let around = node.map_or(lines, |node| union(node, lines));
        let small = label.text_size() * self.viewport.zoom < MIN_LABEL_PX;
        self.label = Some(label);
        let centre = around.center();
        match self.size {
            Some((width, height)) if small => self.jump(Viewport {
                x: centre.x - width / 2.0,
                y: centre.y - height / 2.0,
                zoom: 1.0,
            }),
            // Before the widget has a size, the element stays where it is.
            None if small => {
                let about = self.viewport.to_screen((centre.x, centre.y));
                self.jump(self.viewport.zoomed_to(1.0, about));
            }
            Some(size) if !shows(self.viewport, size, around) => {
                self.jump(revealing(self.viewport, size, around));
            }
            _ => {}
        }
    }

    /// Close the label editor (§5.9 rules 4 and 8). Its text is the
    /// board's already, so an unchanged label changes nothing; what is left
    /// to do is to delete free text that ends empty, when this edit
    /// emptied it or made it.
    fn commit_label(&mut self, now: Instant, update: &mut Update) {
        let Some(label) = self.label.take() else {
            return;
        };
        let free =
            self.board.nodes.get(label.id()).is_some_and(|node| {
                node.kind.as_ref().and_then(Named::known) == Some(NodeKind::Text)
            });
        if free && label.stored().is_empty() && (label.changed() || label.made) {
            let ids = vec![label.id().clone()];
            self.edit(&Intent::Delete { ids }, now, update);
        }
    }

    /// One action on the open label: its keys, the input method's events,
    /// and what a harness types.
    fn label_action(&mut self, action: LabelAction, now: Instant, update: &mut Update) {
        if let Some(label) = &mut self.label {
            let outcome = label.apply(action, &self.measurer);
            self.label_did(&outcome, now, update);
        }
    }

    /// Carry out what an action on the label asked for. A change of its
    /// text is an edit of the board at once (§5.9 rule 6), with the size
    /// the node needs for it (rule 7); undoing the typing puts the size
    /// back as well.
    fn label_did(&mut self, outcome: &label::Outcome, now: Instant, update: &mut Update) {
        if let Some(text) = &outcome.copied {
            update.effects.push(Effect::ClipboardWrite(text.clone()));
        }
        if outcome.paste {
            update.effects.push(Effect::ClipboardRead);
        }
        if outcome.full {
            update.effects.push(Effect::Notice(LABEL_FULL.to_owned()));
        }
        let Some(label) = self.label.as_ref().filter(|_| outcome.changed) else {
            return;
        };
        let id = label.id().clone();
        let text = label.stored().to_owned();
        let intent = match self.board.nodes.get(&id) {
            Some(node) => {
                let size = match label.size {
                    Some(size) if outcome.restored => size,
                    _ => label::grown(node, node.size, &text, &self.measurer),
                };
                Intent::SetText { id, text, size }
            }
            None => Intent::SetEdgeLabel { id, text },
        };
        self.edit(&intent, now, update);
        self.place_label();
    }

    /// Lay the open label out where the board now draws it.
    fn place_label(&mut self) {
        let Some(id) = self.label.as_ref().map(|label| label.id().clone()) else {
            return;
        };
        let place = self.label_place(&id);
        if let (Some(label), Some((place, ink))) = (&mut self.label, place) {
            label.ink = ink;
            label.place(place, &self.measurer);
        }
    }

    /// The label is open only while the machine edits its element: an edit
    /// that removed the element closed it.
    fn keep_label(&mut self) {
        let open = self.label.as_ref().map(Label::id);
        if open != self.machine.editing() {
            self.label = None;
        }
    }

    /// Where the label of `id` is laid out, and the colour it is drawn in:
    /// a node's label box, or the point a connector's first label is
    /// centred on.
    fn label_place(&self, id: &Id) -> Option<(label::Place, Rgba)> {
        if let Some(node) = self.board.nodes.get(id) {
            let bounds = hit::Frames::new(&self.board).get(id)?;
            let ink = match node.kind.as_ref().and_then(Named::known) {
                Some(NodeKind::Shape) => {
                    let fill = node.style.fill.known().unwrap_or(Fill::Tint);
                    style::resolve(&node.style.color, fill, &self.palette).label
                }
                Some(NodeKind::Text) => {
                    style::resolve(&node.style.color, Fill::None, &self.palette).stroke
                }
                None => self.palette.ink,
            };
            return Some((label::Place::of_node(node, bounds), ink));
        }
        let edge = self.board.edges.get(id)?;
        let ink = style::resolve(&edge.style.color, Fill::None, &self.palette).stroke;
        let first = edge.labels.first();
        let centre = if first.is_some_and(|label| !label.text.trim().is_empty()) {
            edge_label_point(&self.scene, id)?
        } else {
            // The scene draws a blank label nowhere, so it is asked where
            // it would draw one: on a copy of the board whose label has
            // text. Routes do not depend on labels.
            #[cfg(feature = "test-support")]
            let started = std::time::Instant::now();
            let mut board = self.board.clone();
            let labels = &mut board.edges.get_mut(id)?.labels;
            match labels.first_mut() {
                Some(label) => "x".clone_into(&mut label.text),
                None => labels.push(wren_board::Label {
                    text: "x".to_owned(),
                    at: wren_board::Label::DEFAULT_AT,
                    extra: wren_board::Extra::new(),
                }),
            }
            let scene = scene::build(&board, &self.palette, &self.measurer);
            #[cfg(feature = "test-support")]
            self.probe.built(started);
            edge_label_point(&scene, id)?
        };
        Some((label::Place::Connector(centre), ink))
    }

    /// Show the board as `intent` would leave it, or as it is.
    fn show(&mut self, intent: Option<&Intent>) {
        self.preview = intent.and_then(|intent| {
            let plan = edit::plan(&self.board, intent, &mut self.ids);
            if plan.is_empty() {
                return None;
            }
            let mut board = self.board.clone();
            board.apply(&plan.changes);
            Some(self.picture_of(board))
        });
    }

    /// A preview of `board`.
    fn picture_of(&mut self, board: Board) -> Preview {
        #[cfg(feature = "test-support")]
        let started = std::time::Instant::now();
        let scene = scene::build(&board, &self.palette, &self.measurer);
        let plan = layers::split(&scene.items, self.cap);
        #[cfg(feature = "test-support")]
        self.probe.built(started);
        Preview {
            board,
            scene,
            plan,
            revision: revision(),
        }
    }

    /// A command the machine left to the view.
    fn run(&mut self, command: Command, now: Instant, update: &mut Update) {
        let ids: Vec<Id> = self.machine.selection().iter().cloned().collect();
        let editable = self.writable() && !ids.is_empty();
        match command {
            Command::Delete if editable => self.edit(&Intent::Delete { ids }, now, update),
            Command::Duplicate if editable => {
                let by = [GRID, GRID];
                self.edit(&Intent::Duplicate { ids, by }, now, update);
            }
            Command::Select(dir) => self.walk(dir, false),
            Command::ExtendSelection(dir) => self.walk(dir, true),
            Command::ZoomFit => self.fit(),
            Command::ZoomSelection => {
                let bounds = ids
                    .iter()
                    .filter_map(|id| bounds(&self.board, &self.scene, id))
                    .reduce(union);
                if let (Some(bounds), Some(size)) = (bounds, self.size) {
                    self.jump(Viewport::fitting(bounds, size));
                }
            }
            _ => {}
        }
    }

    /// An arrow key's walk (§5.7 rules 1 to 3). With nothing selected it
    /// selects the cursor, or with no cursor the node nearest the top,
    /// then the left; otherwise it walks from the cursor, or from the
    /// first selected element when the cursor is gone. Extending adds the
    /// element walked to and never removes.
    fn walk(&mut self, dir: Dir, extend: bool) {
        let selection = self.machine.selection();
        let cursor = self.machine.cursor().cloned();
        let target = if selection.is_empty() {
            cursor.or_else(|| {
                let frames = hit::Frames::new(&self.board);
                let corners = self.board.nodes.keys().filter_map(|id| {
                    let [x, y] = frames.corner(id)?;
                    Some(((y, x), id))
                });
                corners.min().map(|(_, id)| id.clone())
            })
        } else {
            let from = cursor.or_else(|| selection.iter().next().cloned());
            from.and_then(|from| walk::next(&self.board, &from, dir))
        };
        let Some(target) = target else {
            return;
        };
        let mut selection = if extend {
            selection.clone()
        } else {
            BTreeSet::new()
        };
        selection.insert(target.clone());
        self.machine.select(selection, Some(target));
    }

    /// Pan by the least amount that shows the cursor element with
    /// [`REVEAL_MARGIN`] around it (§5.7 rule 5).
    fn reveal(&mut self) {
        let Some(size) = self.size else {
            return;
        };
        let cursor = self.machine.cursor();
        if let Some(bounds) = cursor.and_then(|id| bounds(&self.board, &self.scene, id)) {
            self.jump(revealing(self.viewport, size, bounds));
        }
    }

    /// The model to the display list and its layers.
    fn build(&mut self) {
        #[cfg(feature = "test-support")]
        let started = std::time::Instant::now();
        self.measurer.sweep();
        self.scene = scene::build(&self.board, &self.palette, &self.measurer);
        self.plan = layers::split(&self.scene.items, self.cap);
        self.revision = revision();
        #[cfg(feature = "test-support")]
        self.probe.built(started);
    }

    /// Paint in at most `cap` layers instead of [`layers::CAP`], to see
    /// what sharing a layer does to a stack of shapes.
    #[cfg(feature = "test-support")]
    pub fn set_layer_cap(&mut self, cap: usize) {
        self.cap = cap;
        self.plan = layers::split(&self.scene.items, self.cap);
        self.revision = revision();
    }
}

/// Where an element is drawn, in board units: a node's box, or the union
/// of what the scene draws for a connector.
pub(crate) fn bounds(board: &Board, scene: &Scene, id: &Id) -> Option<Rect> {
    if board.nodes.contains_key(id) {
        return hit::Frames::new(board).get(id);
    }
    scene
        .items
        .iter()
        .filter(|item| matches!(&item.source, Source::Edge(edge) if edge == id))
        .map(|item| item.bounds)
        .reduce(union)
}

/// Where `scene` centres the first label it draws for the connector `id`
/// (§3.7): the middle of the box it draws behind the label's text.
fn edge_label_point(scene: &Scene, id: &Id) -> Option<scene::Point> {
    let mut items = scene
        .items
        .iter()
        .filter(|item| matches!(&item.source, Source::Edge(of) if of == id))
        .peekable();
    // Each label drawn is its backing, then its text.
    while let Some(item) = items.next() {
        let texts = |item: &&scene::Item| matches!(item.draw, Draw::Text { .. });
        if items.peek().is_some_and(texts) {
            return Some(item.bounds.center());
        }
    }
    None
}

/// Whether all of `bounds` is in a widget of `size` looking from
/// `viewport`.
fn shows(viewport: Viewport, size: (f32, f32), bounds: Rect) -> bool {
    let (left, top) = viewport.to_screen((bounds.x, bounds.y));
    let (right, bottom) = viewport.to_screen((bounds.right(), bounds.bottom()));
    left >= 0.0 && top >= 0.0 && right <= size.0 && bottom <= size.1
}

pub(crate) fn union(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Rect::new(
        x,
        y,
        a.right().max(b.right()) - x,
        a.bottom().max(b.bottom()) - y,
    )
}

/// `viewport` panned by the least amount that shows `bounds` in a widget
/// of `size` with [`REVEAL_MARGIN`] around it; where that does not fit, its
/// top-left corner is shown.
pub fn revealing(viewport: Viewport, size: (f32, f32), bounds: Rect) -> Viewport {
    let margin = REVEAL_MARGIN / viewport.zoom;
    let axis = |start: f32, length: f32, from: f32, room: f32| {
        let (low, high) = (start - margin, start + length + margin);
        if low < from {
            low
        } else if high > from + room {
            (high - room).min(low)
        } else {
            from
        }
    };
    Viewport {
        x: axis(bounds.x, bounds.width, viewport.x, size.0 / viewport.zoom),
        y: axis(bounds.y, bounds.height, viewport.y, size.1 / viewport.zoom),
        ..viewport
    }
}

/// What a view did, for the host's test harness.
#[cfg(feature = "test-support")]
#[derive(Debug, Default)]
pub struct Probe {
    /// How many times this view parsed a board's text, its opening included.
    pub parses: std::cell::Cell<u64>,
    /// How many times the scene was built.
    pub scenes: std::cell::Cell<u64>,
    /// Microseconds each recent scene build took, layers included.
    pub scene_micros: std::cell::RefCell<std::collections::VecDeque<u64>>,
    /// How many times a layer's geometry was built.
    pub builds: std::cell::Cell<u64>,
    /// How many scene items those builds painted in all.
    pub painted: std::cell::Cell<u64>,
    /// How many layers the last `draw` painted.
    pub layers: std::cell::Cell<usize>,
    /// Whether the last `draw` took the view to be moving.
    pub moving: std::cell::Cell<bool>,
    /// Microseconds spent in each recent `draw`.
    pub draw_micros: std::cell::RefCell<std::collections::VecDeque<u64>>,
    /// When the first `draw` returned.
    pub first_draw: std::cell::Cell<Option<std::time::Instant>>,
    /// How many arrow keys the focused widget took.
    pub arrows: std::cell::Cell<u64>,
    /// The recent wheel deltas, as the toolkit reported them.
    pub wheel: std::cell::RefCell<std::collections::VecDeque<Scroll>>,
}

#[cfg(feature = "test-support")]
impl Probe {
    /// How many samples each ring of timings keeps.
    pub const SAMPLES: usize = 256;

    fn push(
        ring: &std::cell::RefCell<std::collections::VecDeque<u64>>,
        started: std::time::Instant,
    ) {
        let mut ring = ring.borrow_mut();
        if ring.len() == Self::SAMPLES {
            ring.pop_front();
        }
        ring.push_back(started.elapsed().as_micros().try_into().unwrap_or(u64::MAX));
    }

    fn wheeled(&self, delta: Scroll) {
        let mut wheel = self.wheel.borrow_mut();
        if wheel.len() == Self::SAMPLES {
            wheel.pop_front();
        }
        wheel.push_back(delta);
    }

    fn built(&self, started: std::time::Instant) {
        self.scenes.set(self.scenes.get() + 1);
        Self::push(&self.scene_micros, started);
    }

    pub(crate) fn drew(&self, started: std::time::Instant) {
        Self::push(&self.draw_micros, started);
        if self.first_draw.get().is_none() {
            self.first_draw.set(Some(std::time::Instant::now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use wren_board::scene::{Draw, Measure as _};
    use wren_board::{Reason, Rgba};

    use super::*;

    fn open(text: &str) -> BoardView {
        fixtures::load_fonts();
        BoardView::open(text, fixtures::palette(), fixtures::FONT).expect("a sample board")
    }

    #[test]
    fn a_zoom_keeps_the_point_under_the_pointer() {
        let about = (317.0, 211.0);
        let mut viewport = Viewport {
            x: -40.0,
            y: 125.5,
            zoom: 1.0,
        };
        for factor in [1.1, 1.1, 0.5, 3.0, 0.2, 1.05] {
            let before = viewport.to_board(about);
            viewport = viewport.zoomed(factor, about);
            let after = viewport.to_screen(before);
            assert!((after.0 - about.0).abs() < 0.01 && (after.1 - about.1).abs() < 0.01);
        }
    }

    #[test]
    fn zoom_is_clamped_to_ten_and_four_hundred_percent() {
        let viewport = Viewport::default();
        assert_eq!(viewport.zoomed(100.0, (0.0, 0.0)).zoom, 4.0);
        assert_eq!(viewport.zoomed(0.001, (0.0, 0.0)).zoom, 0.1);
    }

    #[test]
    fn a_pan_moves_by_screen_pixels_at_any_zoom() {
        let viewport = Viewport {
            x: 10.0,
            y: 20.0,
            zoom: 2.0,
        };
        let panned = viewport.panned(8.0, -4.0);
        assert_eq!((panned.x, panned.y, panned.zoom), (14.0, 18.0, 2.0));
        assert_eq!(panned.to_screen((14.0, 18.0)), (0.0, 0.0));
    }

    #[test]
    fn screen_and_board_points_convert_both_ways() {
        for zoom in [0.1, 0.25, 1.0, 1.5, 4.0] {
            let viewport = Viewport {
                x: -312.5,
                y: 48.0,
                zoom,
            };
            // The widget's corner is the viewport's own point.
            assert_eq!(viewport.to_board((0.0, 0.0)), (-312.5, 48.0));
            assert_eq!(viewport.to_screen((-312.5, 48.0)), (0.0, 0.0));
            // A board unit is `zoom` pixels.
            let (x, y) = viewport.to_screen((-212.5, 148.0));
            assert!((x - 100.0 * zoom).abs() < 1e-3 && (y - 100.0 * zoom).abs() < 1e-3);
            for screen in [(0.0, 0.0), (640.0, 360.0), (1279.0, 799.0), (-50.0, 900.0)] {
                let back = viewport.to_screen(viewport.to_board(screen));
                assert!(
                    (back.0 - screen.0).abs() < 0.01 && (back.1 - screen.1).abs() < 0.01,
                    "{screen:?} at {zoom}: {back:?}"
                );
            }
        }
    }

    #[test]
    fn a_view_holds_the_board_its_scene_and_the_text_it_took() {
        let view = open(fixtures::CHECKOUT_FLOW);
        assert_eq!(view.synced(), fixtures::CHECKOUT_FLOW);
        assert_eq!(
            (view.board().nodes.len(), view.board().edges.len()),
            (15, 17)
        );
        assert_eq!(view.issues(), &[]);
        assert_eq!(*view.access(), Access::Editable);
        assert_eq!(view.viewport(), Viewport::default());
        // The scene is the board's, built with the view's own measurer.
        let scene = scene::build(view.board(), view.palette(), view.measurer());
        assert_eq!(*view.scene(), scene);
        assert_eq!(view.layers().of.len(), scene.items.len());
        let labels = scene
            .items
            .iter()
            .filter(|item| {
                matches!(item.draw, Draw::Text { .. }) && matches!(item.source, Source::Node(_))
            })
            .count();
        assert_eq!(labels, 15);
    }

    #[test]
    fn an_unedited_board_drafts_its_text_verbatim() {
        let mut view = open(fixtures::HAND_WRITTEN);
        assert_ne!(wren_board::write(view.board()), fixtures::HAND_WRITTEN);
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
        assert_eq!(view.synced(), fixtures::HAND_WRITTEN);
    }

    #[test]
    fn a_view_reports_issues_and_read_only_boards() {
        let shapes = open(fixtures::SHAPES);
        assert!(!shapes.issues().is_empty());
        assert_eq!(shapes.issues(), wren_board::check(shapes.board()));
        assert_eq!(*shapes.access(), Access::Editable);

        let future = open(fixtures::FUTURE);
        assert_eq!(
            *future.access(),
            Access::ReadOnly(Reason::Requires(vec!["frames".into()]))
        );
        assert!(!future.scene().items.is_empty());
    }

    #[test]
    fn text_that_is_not_a_board_opens_no_view() {
        fixtures::load_fonts();
        let conflict = "schema = 1\n<<<<<<< ours\n[node.a]\n";
        let error = BoardView::open(conflict, fixtures::palette(), fixtures::FONT)
            .err()
            .expect("conflict markers are not TOML");
        assert_eq!(error.line, 2);
    }

    #[test]
    fn a_reload_replaces_the_board_and_keeps_the_viewport() {
        let mut view = open(fixtures::OVERLAP);
        let viewport = Viewport {
            x: -80.0,
            y: 40.0,
            zoom: 2.0,
        };
        view.set_viewport(viewport);
        let revision = view.revision();

        view.reload(fixtures::CHECKOUT_FLOW).expect("a board");
        assert_eq!(view.synced(), fixtures::CHECKOUT_FLOW);
        assert_eq!(view.board().nodes.len(), 15);
        assert_eq!(view.viewport(), viewport);
        assert_ne!(view.revision(), revision);
        assert_eq!(
            *view.scene(),
            scene::build(view.board(), view.palette(), view.measurer())
        );
    }

    #[test]
    fn a_reload_that_does_not_parse_changes_nothing() {
        let mut view = open(fixtures::OVERLAP);
        let (revision, scene) = (view.revision(), view.scene().clone());
        let error = view
            .reload("schema = 1\n[node.a\n")
            .expect_err("broken TOML");
        assert_eq!(error.line, 2);
        assert_eq!(view.synced(), fixtures::OVERLAP);
        assert_eq!(view.board().nodes.len(), 3);
        assert_eq!(view.revision(), revision);
        assert_eq!(*view.scene(), scene);
    }

    #[test]
    fn a_palette_recolours_the_scene_and_keeps_its_geometry() {
        let mut view = open(fixtures::OVERLAP);
        let before = view.scene().clone();
        let revision = view.revision();
        let light = Palette {
            canvas: Rgba::rgb(250, 250, 250),
            ink: Rgba::rgb(20, 20, 20),
            dark: false,
            ..fixtures::palette()
        };
        view.set_palette(light);
        assert_eq!(*view.palette(), light);
        assert_ne!(view.revision(), revision);
        let after = view.scene();
        assert_ne!(*after, before);
        assert_eq!(after.items.len(), before.items.len());
        for (new, old) in after.items.iter().zip(&before.items) {
            assert_eq!(new.bounds, old.bounds);
        }
        // The same colours again are no change at all.
        let revision = view.revision();
        view.set_palette(light);
        assert_eq!(view.revision(), revision);
    }

    #[test]
    fn a_viewport_is_clamped_and_never_not_a_number() {
        let mut view = open(fixtures::OVERLAP);
        let revision = view.revision();
        view.set_viewport(Viewport {
            x: 12.0,
            y: -7.0,
            zoom: 9.0,
        });
        assert_eq!(
            view.viewport(),
            Viewport {
                x: 12.0,
                y: -7.0,
                zoom: 4.0
            }
        );
        view.set_viewport(Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 0.0,
        });
        assert_eq!(view.viewport().zoom, 0.1);
        let kept = view.viewport();
        for broken in [
            Viewport {
                x: f32::NAN,
                ..kept
            },
            Viewport {
                y: f32::INFINITY,
                ..kept
            },
            Viewport {
                zoom: f32::NAN,
                ..kept
            },
        ] {
            view.set_viewport(broken);
            assert_eq!(view.viewport(), kept);
        }
        // Looking elsewhere is not a change to the scene.
        assert_eq!(view.revision(), revision);
    }

    #[test]
    fn no_two_views_share_a_revision() {
        let first = open(fixtures::OVERLAP);
        let second = open(fixtures::OVERLAP);
        assert_ne!(first.revision(), second.revision());
        assert_eq!(*first.scene(), *second.scene());
    }

    #[test]
    fn the_sample_boards_paint_in_at_most_three_layers() {
        for (name, text, layers) in [
            ("Checkout flow", fixtures::CHECKOUT_FLOW, 1),
            ("Shapes", fixtures::SHAPES, 2),
            ("Overlap", fixtures::OVERLAP, 3),
            ("Future", fixtures::FUTURE, 1),
            ("Hand written", fixtures::HAND_WRITTEN, 1),
        ] {
            let view = open(text);
            assert_eq!(view.layers().count, layers, "{name}");
            assert_eq!(view.layers().overflow, 0, "{name}");
        }
    }

    const SIZE: (f32, f32) = (1000.0, 600.0);

    fn sized(text: &str) -> BoardView {
        let mut view = open(text);
        view.update(
            Message::Resized {
                width: SIZE.0,
                height: SIZE.1,
            },
            Instant::now(),
        );
        view
    }

    fn input(view: &mut BoardView, input: Input, now: Instant) -> Update {
        view.update(Message::Input(view.epoch(), input), now)
    }

    #[test]
    fn a_fit_shows_everything_with_the_margin_and_never_above_one_hundred_percent() {
        // A board larger than the widget: one axis has the margin exactly.
        let mut view = sized(fixtures::CHECKOUT_FLOW);
        view.fit();
        let (viewport, bounds) = (view.viewport(), view.scene().bounds);
        assert!(viewport.zoom < 1.0, "{viewport:?}");
        let corner = viewport.to_screen((bounds.x, bounds.y));
        let far = viewport.to_screen((bounds.right(), bounds.bottom()));
        let margins = [corner.0, corner.1, SIZE.0 - far.0, SIZE.1 - far.1];
        for margin in margins {
            assert!(margin >= FIT_MARGIN - 0.01, "{margins:?}");
        }
        // Centred, so the margins of each axis are equal, and the tighter
        // axis has exactly the margin.
        assert!((margins[0] - margins[2]).abs() < 0.01 && (margins[1] - margins[3]).abs() < 0.01);
        assert!(margins
            .iter()
            .any(|margin| (margin - FIT_MARGIN).abs() < 0.01));
    }

    #[test]
    fn a_one_shape_board_fits_at_one_hundred_percent_in_the_middle() {
        let text = "schema = 1\n\n[node.a]\nkind = \"shape\"\nat = [300, -40]\nsize = [120, 60]\n";
        let mut view = sized(text);
        view.fit();
        let viewport = view.viewport();
        assert_eq!(viewport.zoom, 1.0);
        let bounds = view.scene().bounds;
        let middle = viewport.to_screen((
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        ));
        assert!((middle.0 - SIZE.0 / 2.0).abs() < 0.01 && (middle.1 - SIZE.1 / 2.0).abs() < 0.01);
    }

    #[test]
    fn an_empty_board_fits_at_one_hundred_percent_with_its_origin_in_the_middle() {
        for text in [wren_board::EMPTY, ""] {
            let mut view = sized(text);
            view.set_viewport(Viewport {
                x: 77.0,
                y: -3.0,
                zoom: 2.5,
            });
            view.fit();
            let viewport = view.viewport();
            assert_eq!(viewport.zoom, 1.0);
            assert_eq!(viewport.to_screen((0.0, 0.0)), (SIZE.0 / 2.0, SIZE.1 / 2.0));
        }
    }

    #[test]
    fn a_widget_smaller_than_its_margins_fits_at_the_least_zoom() {
        let viewport = Viewport::fitting(scene::Rect::new(0.0, 0.0, 500.0, 100.0), (60.0, 60.0));
        assert_eq!(viewport.zoom, Viewport::MIN_ZOOM);
        assert!(viewport.x.is_finite() && viewport.y.is_finite());
        // Content with width but no height fits by its width alone.
        let line = Viewport::fitting(scene::Rect::new(0.0, 10.0, 1840.0, 0.0), (1000.0, 600.0));
        assert!((line.zoom - 0.5).abs() < 1e-6, "{line:?}");
        assert_eq!(line.to_screen((920.0, 10.0)), (500.0, 300.0));
    }

    #[test]
    fn a_fit_before_the_widget_has_a_size_waits_for_it() {
        let mut view = open(fixtures::CHECKOUT_FLOW);
        assert_eq!(view.size(), None);
        view.fit();
        assert!(view.fit_pending());
        assert_eq!(view.viewport(), Viewport::default());
        // A size that is no size is not taken.
        for (width, height) in [(0.0, 600.0), (f32::NAN, 600.0), (800.0, -1.0)] {
            view.update(Message::Resized { width, height }, Instant::now());
            assert!(view.fit_pending());
        }
        let update = view.update(
            Message::Resized {
                width: SIZE.0,
                height: SIZE.1,
            },
            Instant::now(),
        );
        assert_eq!(update, Update::default());
        assert!(!view.fit_pending());
        assert_eq!(view.size(), Some(SIZE));
        assert_eq!(
            view.viewport(),
            Viewport::fitting(view.scene().bounds, SIZE)
        );
        // A later resize keeps where the view looks.
        let fitted = view.viewport();
        view.update(
            Message::Resized {
                width: 400.0,
                height: 300.0,
            },
            Instant::now(),
        );
        assert_eq!(view.viewport(), fitted);
    }

    #[test]
    fn a_viewport_set_before_the_size_replaces_a_waiting_fit() {
        let mut view = open(fixtures::CHECKOUT_FLOW);
        view.fit();
        let chosen = Viewport {
            x: 12.0,
            y: 34.0,
            zoom: 1.5,
        };
        view.set_viewport(chosen);
        assert!(!view.fit_pending());
        view.update(
            Message::Resized {
                width: SIZE.0,
                height: SIZE.1,
            },
            Instant::now(),
        );
        assert_eq!(view.viewport(), chosen);
    }

    #[test]
    fn keyboard_zoom_steps_through_the_stops_about_the_middle() {
        let mut view = sized(fixtures::OVERLAP);
        let middle = (SIZE.0 / 2.0, SIZE.1 / 2.0);
        let under = view.viewport().to_board(middle);
        let mut seen = vec![view.viewport().zoom];
        for _ in 0..6 {
            view.zoom_step(1);
            seen.push(view.viewport().zoom);
        }
        assert_eq!(seen, [1.0, 1.5, 2.0, 3.0, 4.0, 4.0, 4.0]);
        let mut seen = Vec::new();
        for _ in 0..10 {
            view.zoom_step(-1);
            seen.push(view.viewport().zoom);
        }
        assert_eq!(seen, [3.0, 2.0, 1.5, 1.0, 0.75, 0.5, 0.25, 0.1, 0.1, 0.1]);
        let back = view.viewport().to_screen(under);
        assert!((back.0 - middle.0).abs() < 0.01 && (back.1 - middle.1).abs() < 0.01);
        // Several stops at once, and past either end.
        view.zoom_step(3);
        assert_eq!(view.viewport().zoom, 0.75);
        view.zoom_step(100);
        assert_eq!(view.viewport().zoom, 4.0);
        view.zoom_step(-128);
        assert_eq!(view.viewport().zoom, 0.1);
        let kept = view.viewport();
        view.zoom_step(0);
        assert_eq!(view.viewport(), kept);
    }

    #[test]
    fn a_zoom_between_stops_steps_to_the_next_one_each_way() {
        let mut view = sized(fixtures::OVERLAP);
        for (zoom, up, down) in [
            (1.1, 1.5, 1.0),
            (0.3, 0.5, 0.25),
            (0.12, 0.25, 0.1),
            (3.9, 4.0, 3.0),
            // A hair off a stop is that stop.
            (1.000_01, 1.5, 0.75),
            (0.749_99, 1.0, 0.5),
        ] {
            let at = Viewport {
                x: 0.0,
                y: 0.0,
                zoom,
            };
            view.set_viewport(at);
            view.zoom_step(1);
            assert_eq!(view.viewport().zoom, up, "{zoom} up");
            view.set_viewport(at);
            view.zoom_step(-1);
            assert_eq!(view.viewport().zoom, down, "{zoom} down");
        }
    }

    #[test]
    fn a_reset_is_one_hundred_percent_about_the_middle() {
        let mut view = sized(fixtures::OVERLAP);
        view.set_viewport(Viewport {
            x: -100.0,
            y: 50.0,
            zoom: 0.3,
        });
        let middle = (SIZE.0 / 2.0, SIZE.1 / 2.0);
        let under = view.viewport().to_board(middle);
        let jumps = view.jumps();
        view.zoom_reset();
        assert_eq!(view.viewport().zoom, 1.0);
        let back = view.viewport().to_screen(under);
        assert!((back.0 - middle.0).abs() < 0.01 && (back.1 - middle.1).abs() < 0.01);
        assert_eq!(view.jumps(), jumps + 1);
        // At 100 % already, a reset is no jump.
        view.zoom_reset();
        assert_eq!(view.jumps(), jumps + 1);
    }

    #[test]
    fn input_moves_the_view_and_never_the_board() {
        let mut view = sized(fixtures::CHECKOUT_FLOW);
        let now = Instant::now();
        let (text, scenes) = (view.synced().to_owned(), view.revision());
        let wheel = Input::Wheel {
            delta: Scroll::Lines { x: 0.0, y: -2.0 },
            at: (200.0, 100.0),
            zoom: false,
            sideways: false,
            scale_factor: 1.0,
        };
        assert_eq!(input(&mut view, wheel, now), Update::default());
        assert_eq!(view.viewport().y, 2.0 * machine::LINE);
        let zoom = Input::Wheel {
            delta: Scroll::Lines { x: 0.0, y: -2.0 },
            at: (200.0, 100.0),
            zoom: true,
            sideways: false,
            scale_factor: 1.0,
        };
        let later = now + machine::BURST;
        let under = view.viewport().to_board((200.0, 100.0));
        assert_eq!(input(&mut view, zoom, later), Update::default());
        let back = view.viewport().to_screen(under);
        assert!((back.0 - 200.0).abs() < 0.01 && (back.1 - 100.0).abs() < 0.01);
        // A middle drag.
        let before = view.viewport();
        for step in [
            Input::Press {
                button: Button::Middle,
                at: (10.0, 10.0),
            },
            Input::Move { at: (40.0, 10.0) },
            Input::Release {
                button: Button::Middle,
            },
        ] {
            assert_eq!(input(&mut view, step, later), Update::default());
        }
        let dragged = view.viewport().to_screen(before.to_board((10.0, 10.0)));
        assert!((dragged.0 - 40.0).abs() < 0.01 && (dragged.1 - 10.0).abs() < 0.01);
        assert_eq!((view.synced(), view.revision()), (text.as_str(), scenes));
    }

    #[test]
    fn input_for_an_earlier_board_is_dropped() {
        let mut view = sized(fixtures::OVERLAP);
        let now = Instant::now();
        let stale = view.epoch();
        let press = Input::Press {
            button: Button::Middle,
            at: (0.0, 0.0),
        };
        input(&mut view, press, now);
        assert!(view.machine().panning());
        // A reload drops the drag in flight and moves the epoch on.
        view.reload(fixtures::CHECKOUT_FLOW).expect("a board");
        assert!(!view.machine().panning());
        assert_ne!(view.epoch(), stale);
        let kept = view.viewport();
        for old in [press, Input::Move { at: (90.0, 90.0) }] {
            view.update(Message::Input(stale, old), now);
        }
        assert!(!view.machine().panning());
        assert_eq!(view.viewport(), kept);
        // The same text again is no reload and keeps the epoch.
        let epoch = view.epoch();
        view.reload(fixtures::CHECKOUT_FLOW).expect("the same text");
        assert_eq!(view.epoch(), epoch);
    }

    #[test]
    fn space_held_turns_a_left_drag_into_a_pan() {
        let mut view = sized(fixtures::OVERLAP);
        let now = Instant::now();
        input(&mut view, Input::Space { held: true }, now);
        assert!(view.machine().space());
        for step in [
            Input::Press {
                button: Button::Left,
                at: (100.0, 100.0),
            },
            Input::Move { at: (70.0, 60.0) },
            Input::Release {
                button: Button::Left,
            },
            Input::Space { held: false },
        ] {
            input(&mut view, step, now);
        }
        assert_eq!((view.viewport().x, view.viewport().y), (30.0, 40.0));
        assert!(!view.machine().space());
    }

    use fixtures::TWO;

    fn id(name: &str) -> Id {
        Id::new(name).expect("an id")
    }

    /// The middle of a node on screen.
    fn middle(view: &BoardView, name: &str) -> (f32, f32) {
        let bounds = hit::Frames::new(view.board())
            .get(&id(name))
            .expect("a node");
        let centre = bounds.center();
        view.viewport().to_screen((centre.x, centre.y))
    }

    fn press(at: (f32, f32)) -> Input {
        Input::Press {
            button: Button::Left,
            at,
        }
    }

    const RELEASE: Input = Input::Release {
        button: Button::Left,
    };

    /// A left click at `at`, with the updates its press and release gave.
    fn click(view: &mut BoardView, at: (f32, f32), now: Instant) -> Update {
        let mut update = input(view, press(at), now);
        let released = input(view, RELEASE, now);
        update.changed |= released.changed;
        update.effects.extend(released.effects);
        update
    }

    fn click_node(view: &mut BoardView, name: &str, now: Instant) -> Update {
        let at = middle(view, name);
        click(view, at, now)
    }

    fn selected(view: &BoardView) -> Vec<&str> {
        view.machine().selection().iter().map(Id::as_str).collect()
    }

    #[test]
    fn a_drag_shows_a_preview_and_applies_one_move_on_release() {
        let mut view = sized(TWO);
        let now = Instant::now();
        let before = view.board().clone();
        let (x, y) = middle(&view, "a");
        assert_eq!(input(&mut view, press((x, y)), now), Update::default());
        let moved = input(&mut view, Input::Move { at: (x + 43.0, y) }, now);
        // The model is untouched while the gesture shows its preview.
        assert_eq!(moved, Update::default());
        assert_eq!(*view.board(), before);
        assert_ne!(view.shown(), view.scene());
        let preview = view.picture();
        assert_eq!(preview.board.nodes[&id("a")].at, [40, 0]);
        assert_eq!(view.machine().state(), machine::State::Moving);

        let released = input(&mut view, RELEASE, now);
        assert_eq!(
            released,
            Update {
                changed: true,
                effects: Vec::new()
            }
        );
        assert_eq!(view.board().nodes[&id("a")].at, [40, 0]);
        assert_eq!(view.board().nodes[&id("b")].at, [300, 0]);
        assert_eq!(view.shown(), view.scene());
        assert_eq!(
            *view.scene(),
            scene::build(view.board(), view.palette(), view.measurer())
        );
        assert_eq!(view.machine().state(), machine::State::Idle);
        assert_eq!(selected(&view), ["a"]);
        // The draft is the canonical text of the moved board.
        let draft = view.draft();
        assert_eq!(draft, wren_board::write(view.board()));
        assert_eq!(view.synced(), draft);
        assert!(draft.contains("at = [40, 0]"), "{draft}");
    }

    #[test]
    fn a_gesture_cancelled_or_settled_changes_nothing() {
        let now = Instant::now();
        let ends: [fn(&mut BoardView, Instant) -> Update; 3] = [
            |view, now| {
                input(
                    view,
                    Input::Key {
                        key: Key::Escape,
                        repeat: false,
                    },
                    now,
                )
            },
            |view, now| view.settle(now),
            |view, now| input(view, Input::Focus(false), now),
        ];
        for (index, end) in ends.into_iter().enumerate() {
            let mut view = sized(TWO);
            let before = view.board().clone();
            let (x, y) = middle(&view, "b");
            input(&mut view, press((x, y)), now);
            input(&mut view, Input::Move { at: (x, y + 61.0) }, now);
            assert_ne!(view.shown(), view.scene(), "{index}");
            assert_eq!(end(&mut view, now), Update::default(), "{index}");
            assert_eq!(view.shown(), view.scene(), "{index}");
            // The release that follows finds no gesture.
            assert_eq!(input(&mut view, RELEASE, now), Update::default(), "{index}");
            assert_eq!(*view.board(), before, "{index}");
            assert!(view.pristine(), "{index}");
            assert_eq!(view.draft(), TWO, "{index}");
        }
    }

    #[test]
    fn a_click_selects_and_a_click_on_the_canvas_keeps_the_cursor() {
        let mut view = sized(TWO);
        let now = Instant::now();
        assert_eq!(click_node(&mut view, "b", now), Update::default());
        assert_eq!(selected(&view), ["b"]);
        // The connector between them.
        assert_eq!(click(&mut view, (210.0, 30.0), now), Update::default());
        assert_eq!(selected(&view), ["e"]);
        click(&mut view, (600.0, 400.0), now);
        assert!(selected(&view).is_empty());
        assert_eq!(view.machine().cursor(), Some(&id("e")));
        assert!(view.pristine());
    }

    #[test]
    fn key_moves_go_back_to_the_pristine_text_verbatim() {
        let mut view = sized(fixtures::HAND_WRITTEN);
        let now = Instant::now();
        view.command(Command::SelectAll, now);
        let moved = view.command(Command::Move(Dir::Right), now);
        assert!(moved.changed);
        assert!(!view.pristine());
        let draft = view.draft();
        assert_eq!(draft, wren_board::write(view.board()));
        assert_ne!(draft, fixtures::HAND_WRITTEN);

        let back = view.command(Command::Move(Dir::Left), now);
        assert!(back.changed);
        assert!(view.pristine());
        // Not the canonical text: the hand-written file, byte for byte.
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
        assert_eq!(view.synced(), fixtures::HAND_WRITTEN);
    }

    #[test]
    fn undoing_to_the_opening_board_after_a_save_drafts_the_original_bytes() {
        // §5.11 rule 5 and §6.8's "move then undo on Hand written, waiting
        // past the debounce": the save in between does not move the floor.
        let mut view = sized(fixtures::HAND_WRITTEN);
        let now = Instant::now();
        view.command(Command::SelectAll, now);
        view.command(Command::Move(Dir::Down), now);
        let saved = view.draft();
        assert_ne!(saved, fixtures::HAND_WRITTEN);
        view.saved(&saved);
        assert!(!view.pristine());
        assert_eq!(view.draft(), saved);
        view.command(Command::Move(Dir::Up), now);
        assert!(view.pristine());
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
        assert_eq!(view.synced(), fixtures::HAND_WRITTEN);
        // The original saved again, then another edit and back: still the
        // original.
        view.saved(fixtures::HAND_WRITTEN);
        view.command(Command::Move(Dir::Right), now);
        assert_ne!(view.draft(), fixtures::HAND_WRITTEN);
        view.command(Command::Move(Dir::Left), now);
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
    }

    #[test]
    fn a_reload_mid_drag_drops_the_gesture_and_takes_the_new_text_as_pristine() {
        let mut view = sized(TWO);
        let now = Instant::now();
        click_node(&mut view, "a", now);
        let (x, y) = middle(&view, "a");
        input(&mut view, press((x, y)), now);
        input(&mut view, Input::Move { at: (x + 80.0, y) }, now);
        assert_ne!(view.shown(), view.scene());

        view.reload(fixtures::HAND_WRITTEN).expect("a board");
        assert_eq!(view.shown(), view.scene());
        assert_eq!(view.machine().state(), machine::State::Idle);
        // Stale input, and the release, change nothing.
        view.update(Message::Input(view.epoch() - 1, RELEASE), now);
        input(&mut view, RELEASE, now);
        assert!(view.pristine());
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
    }

    #[test]
    fn a_read_only_board_selects_and_never_changes() {
        let mut view = sized(fixtures::FUTURE);
        assert!(!view.writable());
        let now = Instant::now();
        let before = view.board().clone();
        let first = view.board().nodes.keys().next().expect("a node").clone();
        let (x, y) = middle(&view, first.as_str());
        click(&mut view, (x, y), now);
        assert_eq!(selected(&view), [first.as_str()]);
        input(&mut view, press((x, y)), now);
        input(
            &mut view,
            Input::Move {
                at: (x + 60.0, y + 60.0),
            },
            now,
        );
        assert_eq!(view.shown(), view.scene());
        input(&mut view, RELEASE, now);
        for command in [
            Command::Move(Dir::Right),
            Command::Resize(Grow::Wider),
            Command::Delete,
            Command::Duplicate,
            Command::Tool(Tool::Shape(wren_board::Shape::Diamond)),
        ] {
            assert_eq!(view.command(command, now), Update::default(), "{command:?}");
        }
        assert_eq!(view.machine().tool(), Tool::Select);
        view.command(Command::SelectAll, now);
        assert_eq!(view.machine().selection().len(), {
            let board = view.board();
            board
                .nodes
                .keys()
                .chain(board.edges.keys())
                .collect::<BTreeSet<_>>()
                .len()
        });
        assert_eq!(*view.board(), before);
        assert_eq!(view.draft(), fixtures::FUTURE);
    }

    #[test]
    fn an_arrow_with_nothing_selected_selects_the_cursor_or_the_top_left_node_and_shows_it() {
        let mut view = sized(TWO);
        let now = Instant::now();
        let far = Viewport {
            x: 5000.0,
            y: 5000.0,
            zoom: 1.0,
        };
        view.set_viewport(far);
        view.command(Command::Select(Dir::Right), now);
        assert_eq!(selected(&view), ["a"]);
        assert_eq!(view.machine().cursor(), Some(&id("a")));
        // Panned the least that shows `a` with the margin.
        assert_eq!(
            view.viewport(),
            Viewport {
                x: -REVEAL_MARGIN,
                y: -REVEAL_MARGIN,
                zoom: 1.0
            }
        );

        // With the selection cleared, the cursor is selected again; and
        // extending from nothing does the same.
        view.command(Command::SelectNone, now);
        view.set_viewport(far);
        view.command(Command::ExtendSelection(Dir::Down), now);
        assert_eq!(selected(&view), ["a"]);
        assert_eq!(view.viewport().x, -REVEAL_MARGIN);
        // A click on `b` moves the cursor; a pointer never reveals.
        view.set_viewport(Viewport {
            x: 200.0,
            ..Viewport::default()
        });
        let shown = view.viewport();
        click_node(&mut view, "b", now);
        assert_eq!(view.viewport(), shown);
        view.command(Command::SelectNone, now);
        view.set_viewport(far);
        view.command(Command::Select(Dir::Left), now);
        assert_eq!(selected(&view), ["b"]);

        // An empty board has nothing to walk to.
        let mut empty = sized(wren_board::EMPTY);
        let kept = empty.viewport();
        empty.command(Command::Select(Dir::Up), now);
        assert!(empty.machine().selection().is_empty());
        assert_eq!(empty.viewport(), kept);
    }

    #[test]
    fn revealing_pans_the_least_that_shows_the_element() {
        let at = |x: f32, y: f32, zoom: f32| Viewport { x, y, zoom };
        let size = (400.0, 300.0);
        let node = Rect::new(100.0, 100.0, 120.0, 60.0);
        for (viewport, expected) in [
            // In view with room: no pan.
            (at(0.0, 0.0, 1.0), at(0.0, 0.0, 1.0)),
            // Too far right and down: its far edges come in by the margin.
            (at(-200.0, -200.0, 1.0), at(-140.0, -100.0, 1.0)),
            // Too far left and up.
            (at(150.0, 150.0, 1.0), at(60.0, 60.0, 1.0)),
            // The margin is screen pixels: at 200 % it is 20 units.
            (at(300.0, 0.0, 2.0), at(80.0, 30.0, 2.0)),
            // Larger than the view: its top-left corner, with the margin.
            (at(0.0, 0.0, 4.0), at(90.0, 90.0, 4.0)),
        ] {
            assert_eq!(revealing(viewport, size, node), expected, "{viewport:?}");
        }
    }

    #[test]
    fn the_fit_commands_fit_the_board_and_the_selection() {
        let mut view = sized(fixtures::CHECKOUT_FLOW);
        let now = Instant::now();
        view.command(Command::ZoomFit, now);
        assert_eq!(
            view.viewport(),
            Viewport::fitting(view.scene().bounds, SIZE)
        );
        // With nothing selected there is no selection to fit.
        let fitted = view.viewport();
        view.command(Command::ZoomSelection, now);
        assert_eq!(view.viewport(), fitted);
        let first = view.board().nodes.keys().next().expect("a node").clone();
        click_node(&mut view, first.as_str(), now);
        view.command(Command::ZoomSelection, now);
        let bounds = hit::Frames::new(view.board()).get(&first).expect("a node");
        assert_eq!(view.viewport(), Viewport::fitting(bounds, SIZE));
        assert_eq!(view.viewport().zoom, 1.0);
    }

    #[test]
    fn new_ids_are_eight_characters_of_the_alphabet_and_differ() {
        let mut ids = Fresh::default();
        let drawn: BTreeSet<Id> = (0..2000).map(|_| ids.draw()).collect();
        assert_eq!(drawn.len(), 2000);
        for id in &drawn {
            assert_eq!(id.as_str().len(), 8);
            assert!(id
                .as_str()
                .bytes()
                .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte)));
        }
    }

    #[test]
    fn the_renderer_names_its_label_rule() {
        assert_eq!(Labels::for_renderer("tiny-skia"), Labels::SOFTWARE);
        assert_eq!(Labels::for_renderer("wgpu"), Labels::ALWAYS);
        assert_eq!(Style::default().labels, None);
        assert_eq!(Style::default().scale_factor, 1.0);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn the_scene_is_built_once_per_change_and_never_for_a_look() {
        let mut view = open(fixtures::CHECKOUT_FLOW);
        assert_eq!(view.probe.scenes.get(), 1);
        assert_eq!(view.probe.scene_micros.borrow().len(), 1);

        // Reading, looking elsewhere and making the widget build nothing.
        for step in 0..40u8 {
            view.set_viewport(view.viewport().panned(8.0, 0.0));
            view.set_viewport(view.viewport().zoomed(1.05, (640.0, 360.0)));
            let _ = (view.scene(), view.board(), view.issues(), view.layers());
            let _ = view.view("note-editor", Style::default());
            view.reload(fixtures::CHECKOUT_FLOW).expect("the same text");
            view.set_palette(fixtures::palette());
            assert_eq!(view.probe.scenes.get(), 1, "step {step}");
        }

        // A theme change is one build; so is new text.
        view.set_palette(Palette {
            dark: false,
            ..fixtures::palette()
        });
        assert_eq!(view.probe.scenes.get(), 2);
        view.reload(fixtures::OVERLAP).expect("a board");
        assert_eq!(view.probe.scenes.get(), 3);
        // Text that does not parse builds nothing.
        assert!(view.reload("schema = ").is_err());
        assert_eq!(view.probe.scenes.get(), 3);
        // The widget has drawn nothing: there is no renderer here.
        assert_eq!(view.probe.builds.get(), 0);
        assert_eq!(view.probe.layers.get(), 0);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn a_lower_cap_shares_the_last_layer_without_a_scene_build() {
        let mut view = open(fixtures::OVERLAP);
        let revision = view.revision();
        view.set_layer_cap(2);
        assert_eq!(view.layers().count, 2);
        assert!(view.layers().overflow > 0);
        assert_ne!(view.revision(), revision);
        assert_eq!(view.probe.scenes.get(), 1);
        view.set_layer_cap(layers::CAP);
        assert_eq!((view.layers().count, view.layers().overflow), (3, 0));
    }
    fn key(view: &mut BoardView, key: Key, now: Instant) -> Update {
        input(view, Input::Key { key, repeat: false }, now)
    }

    fn edit(view: &mut BoardView, action: LabelAction, now: Instant) -> Update {
        view.update(Message::Label(view.epoch(), action), now)
    }

    fn typed(view: &mut BoardView, text: &str, now: Instant) -> Update {
        edit(view, LabelAction::Insert(text.to_owned()), now)
    }

    fn step(view: &mut BoardView, motion: Motion, select: bool, now: Instant) -> Update {
        edit(view, LabelAction::Move { motion, select }, now)
    }

    /// Select `name` alone and open its label, as Enter does.
    fn open_label(view: &mut BoardView, name: &str, now: Instant) -> Update {
        view.machine
            .select(BTreeSet::from([id(name)]), Some(id(name)));
        input(view, Input::Focus(true), now);
        view.command(Command::EditLabel, now)
    }

    fn label(view: &BoardView) -> &Label {
        view.label().expect("an open label")
    }

    const CHANGED: Update = Update {
        changed: true,
        effects: Vec::new(),
    };

    /// One node of `shape` and the default size, unlabelled.
    fn one(kind: &str, shape: &str) -> String {
        format!(
            "schema = 1\n\n[node.n]\nkind = \"{kind}\"\nat = [40, 40]\nshape = \"{shape}\"\nsize = [120, 60]\nz = \"V\"\n"
        )
    }

    #[test]
    fn a_label_opens_with_its_text_selected_and_changes_nothing() {
        let mut view = sized(TWO);
        let now = Instant::now();
        assert_eq!(view.label(), None);
        assert_eq!(open_label(&mut view, "a", now), Update::default());
        assert_eq!(view.machine().state(), machine::State::Editing);
        let open = label(&view);
        assert_eq!((open.id(), open.text()), (&id("a"), "A"));
        assert_eq!((open.selection(), open.caret()), (0..1, 1));
        assert!(view.pristine());
        assert!(view.intents.is_empty());
        // The caret is at the end of the scene's own line, on screen.
        let Some(Draw::Text { lines, .. }) = view
            .scene()
            .items
            .iter()
            .filter(|item| item.source == Source::Node(id("a")))
            .map(|item| &item.draw)
            .find(|draw| matches!(draw, Draw::Text { .. }))
        else {
            panic!("a label");
        };
        let line = &lines[0];
        let (caret, size) = view.label_caret().expect("a caret");
        assert!((caret.x - (line.origin.x + line.width)).abs() < 0.01);
        assert_eq!(
            (caret.y, caret.width, caret.height),
            (line.origin.y, 0.0, 20.0)
        );
        assert_eq!(size, scene::LABEL_SIZE);
        assert_eq!(
            view.label_box(),
            Some(Rect::new(line.origin.x, line.origin.y, line.width, 20.0))
        );
    }

    #[test]
    fn every_change_of_a_label_is_an_edit_of_the_board_at_once() {
        let mut view = sized(TWO);
        let now = Instant::now();
        open_label(&mut view, "a", now);
        // Typing replaces the selected text.
        assert_eq!(typed(&mut view, "S", now), CHANGED);
        assert_eq!(view.board().nodes[&id("a")].text, "S");
        assert_eq!(
            *view.scene(),
            scene::build(view.board(), view.palette(), view.measurer())
        );
        assert!(!view.pristine());
        for letter in ["t", "a", "r", "t"] {
            assert_eq!(typed(&mut view, letter, now), CHANGED);
        }
        assert_eq!(view.board().nodes[&id("a")].text, "Start");
        let draft = view.draft();
        assert!(draft.contains("text = \"Start\""), "{draft}");
        // One intent a change, each with the node's size as it is.
        assert_eq!(view.intents.len(), 5);
        assert_eq!(
            view.intents[4],
            Intent::SetText {
                id: id("a"),
                text: "Start".to_owned(),
                size: [120, 60]
            }
        );
        // A caret move is no change, and a trailing line feed none of the
        // board's: it is not stored.
        assert_eq!(step(&mut view, Motion::Left, false, now), Update::default());
        assert_eq!(step(&mut view, Motion::End, false, now), Update::default());
        assert_eq!(typed(&mut view, "\n", now), Update::default());
        assert_eq!((label(&view).text(), label(&view).caret()), ("Start\n", 6));
        assert_eq!(view.board().nodes[&id("a")].text, "Start");
        assert_eq!(
            edit(&mut view, LabelAction::Backspace, now),
            Update::default()
        );
        assert_eq!(edit(&mut view, LabelAction::Backspace, now), CHANGED);
        assert_eq!(view.board().nodes[&id("a")].text, "Star");
        // The label stays open through all of it, on the same element.
        assert_eq!(view.machine().editing(), Some(&id("a")));
        assert_eq!(selected(&view), ["a"]);
        assert_eq!(view.board().nodes[&id("b")].text, "B");
    }

    /// The plan's review attack 6: opening a label, moving about in it and
    /// closing it, by every way out, leaves the hand-written text as it
    /// was and asks the board for nothing.
    #[test]
    fn opening_and_closing_a_label_leaves_the_board_pristine() {
        let now = Instant::now();
        type Close = fn(&mut BoardView, Instant) -> Update;
        let ways: [(&str, Close); 6] = [
            ("escape", |view, now| key(view, Key::Escape, now)),
            ("commit", |view, now| {
                view.command(Command::CommitLabel, now)
            }),
            ("settle", |view, now| view.settle(now)),
            ("blur", |view, now| input(view, Input::Focus(false), now)),
            ("click", |view, now| click(view, (900.0, 500.0), now)),
            ("select", |view, now| view.command(Command::SelectNone, now)),
        ];
        for element in ["first", "second", "caption", "flow"] {
            for (name, close) in ways {
                let mut view = sized(fixtures::HAND_WRITTEN);
                open_label(&mut view, element, now);
                assert_eq!(view.machine().editing(), Some(&id(element)), "{element}");
                for action in [
                    LabelAction::Move {
                        motion: Motion::Start,
                        select: false,
                    },
                    LabelAction::Move {
                        motion: Motion::WordRight,
                        select: true,
                    },
                    LabelAction::Move {
                        motion: Motion::Down,
                        select: false,
                    },
                    LabelAction::SelectAll,
                    LabelAction::Undo,
                    LabelAction::Redo,
                ] {
                    assert_eq!(edit(&mut view, action, now), Update::default());
                }
                let copied = edit(&mut view, LabelAction::Copy, now);
                assert!(!copied.changed, "{element}, {name}");
                assert_eq!(
                    close(&mut view, now),
                    Update::default(),
                    "{element}, {name}"
                );
                assert_eq!(view.label(), None, "{element}, {name}");
                assert_eq!(view.machine().editing(), None, "{element}, {name}");
                assert!(view.intents.is_empty(), "{element}, {name}");
                assert!(view.pristine(), "{element}, {name}");
                assert_eq!(view.draft(), fixtures::HAND_WRITTEN, "{element}, {name}");
            }
        }
    }

    #[test]
    fn undoing_the_typing_makes_the_board_pristine_again() {
        let mut view = sized(fixtures::HAND_WRITTEN);
        let now = Instant::now();
        open_label(&mut view, "first", now);
        step(&mut view, Motion::End, false, now);
        // Enough lines to grow the node.
        assert_eq!(typed(&mut view, "\nand\nthen\nmore", now), CHANGED);
        let grown = view.board().nodes[&id("first")].size;
        assert_eq!(grown, [120, 100]);
        assert!(!view.pristine());
        // The label's own undo puts back the text and the size, and the
        // draft is the hand-written file again, byte for byte.
        assert_eq!(edit(&mut view, LabelAction::Undo, now), CHANGED);
        assert_eq!(label(&view).text(), "First step");
        assert_eq!(view.board().nodes[&id("first")].size, [120, 60]);
        assert!(view.pristine());
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
        // Redo is the typing again, grown as it was.
        assert_eq!(edit(&mut view, LabelAction::Redo, now), CHANGED);
        assert_eq!(
            view.board().nodes[&id("first")].text,
            "First step\nand\nthen\nmore"
        );
        assert_eq!(view.board().nodes[&id("first")].size, grown);
        // Committed, the label is the board's and the editor is gone.
        assert_eq!(key(&mut view, Key::Escape, now), Update::default());
        assert_eq!(view.label(), None);
        assert_eq!(
            view.board().nodes[&id("first")].text,
            "First step\nand\nthen\nmore"
        );
        assert_eq!(view.machine().state(), machine::State::Idle);
        assert_eq!(selected(&view), ["first"]);
    }

    #[test]
    fn a_node_grows_as_its_label_is_typed_and_never_shrinks() {
        let mut view = sized(TWO);
        let now = Instant::now();
        open_label(&mut view, "a", now);
        let size = |view: &BoardView| view.board().nodes[&id("a")].size;
        typed(&mut view, "one", now);
        typed(&mut view, "\ntwo", now);
        assert_eq!(size(&view), [120, 60]);
        // A third line is taller than the 52 of the label box.
        typed(&mut view, "\nthree", now);
        assert_eq!(size(&view), [120, 80]);
        assert_eq!(view.board().nodes[&id("a")].at, [0, 0]);
        assert_eq!(
            view.intents.last(),
            Some(&Intent::SetText {
                id: id("a"),
                text: "one\ntwo\nthree".to_owned(),
                size: [120, 80]
            })
        );
        // The caret is where the scene now draws the last line's end.
        let Some(Draw::Text { lines, .. }) = view
            .scene()
            .items
            .iter()
            .filter(|item| item.source == Source::Node(id("a")))
            .map(|item| &item.draw)
            .find(|draw| matches!(draw, Draw::Text { .. }))
        else {
            panic!("a label");
        };
        let last = &lines[2];
        let (caret, _) = view.label_caret().expect("a caret");
        assert!((caret.x - (last.origin.x + last.width)).abs() < 0.01);
        assert_eq!(caret.y, last.origin.y);
        // Erasing lines gives nothing back.
        edit(&mut view, LabelAction::SelectAll, now);
        typed(&mut view, "x", now);
        assert_eq!(size(&view), [120, 80]);
        // Only a word wider than the box widens the node.
        typed(&mut view, " several words that wrap", now);
        assert_eq!(size(&view)[0], 120);
        edit(&mut view, LabelAction::SelectAll, now);
        typed(&mut view, "Pneumonoultramicroscopic", now);
        let [width, height] = size(&view);
        assert!(width > 120 && width % 20 == 0, "{width}");
        assert_eq!(height, 80);
        let area = wren_board::shape::label_box(
            wren_board::Shape::Rect,
            Rect::new(0.0, 0.0, width as f32, height as f32),
        );
        let word = view
            .measurer()
            .width("Pneumonoultramicroscopic", scene::Face::Regular, 14.0);
        assert!(
            word <= area.width && word > area.width - 20.0,
            "{word} in {area:?}"
        );
    }

    /// §5.20: every label of the acceptance script, typed a key at a time
    /// into the shape the script gives it, leaves the node 120 by 60.
    #[test]
    fn the_acceptance_labels_are_typed_without_growing_a_node() {
        let now = Instant::now();
        for (text, shape) in fixtures::SCRIPT {
            let shape = match shape {
                wren_board::Shape::Pill => "pill",
                wren_board::Shape::Diamond => "diamond",
                _ => "rect",
            };
            let mut view = sized(&one("shape", shape));
            open_label(&mut view, "n", now);
            for letter in text.chars() {
                typed(&mut view, letter.encode_utf8(&mut [0; 4]), now);
            }
            key(&mut view, Key::Escape, now);
            let node = &view.board().nodes[&id("n")];
            assert_eq!(
                (node.text.as_str(), node.size),
                (text, [120, 60]),
                "{shape}"
            );
        }
    }

    /// The editor lays a label out as the scene draws it: the same lines
    /// in the same places, for every shape's box, free text and a
    /// connector, so a caret is on its character and nothing moves when
    /// the label closes.
    #[test]
    fn the_editor_s_lines_are_the_scene_s() {
        let now = Instant::now();
        let texts = fixtures::LABELS
            .into_iter()
            .chain(fixtures::SCRIPT.map(|(text, _)| text))
            .chain(["a much longer label that wraps over several lines of its box"]);
        let shapes = [
            ("shape", "rect"),
            ("shape", "rounded"),
            ("shape", "pill"),
            ("shape", "ellipse"),
            ("shape", "diamond"),
            ("shape", "parallelogram"),
            ("shape", "cylinder"),
            ("shape", "document"),
            ("text", "rect"),
            ("future", "rect"),
        ];
        let drawn = |view: &BoardView, source: &Source, size: f32| -> Vec<(String, f32, f32)> {
            let texts = view
                .scene()
                .items
                .iter()
                .filter_map(|item| match &item.draw {
                    Draw::Text {
                        lines, size: of, ..
                    } if item.source == *source && *of == size => Some(lines),
                    _ => None,
                });
            texts
                .flatten()
                .map(|line| (line.text.clone(), line.origin.x, line.origin.y))
                .collect()
        };
        let same = |view: &BoardView, source: &Source, size: f32, what: &str| {
            let scene = drawn(view, source, size);
            let editor = label(view).lines();
            assert_eq!(editor.len(), scene.len(), "{what}");
            for ((text, x, y), (drawn, at, down)) in editor.into_iter().zip(scene) {
                assert_eq!(text, drawn, "{what}");
                assert!((x - at).abs() < 0.01 && (y - down).abs() < 0.01, "{what}");
            }
        };
        for text in texts {
            for (kind, shape) in shapes {
                let mut view = sized(&one(kind, shape));
                open_label(&mut view, "n", now);
                typed(&mut view, text, now);
                let source = Source::Node(id("n"));
                same(
                    &view,
                    &source,
                    scene::LABEL_SIZE,
                    &format!("{text:?} in {kind} {shape}"),
                );
                // And as it is opened again on the stored text.
                key(&mut view, Key::Escape, now);
                open_label(&mut view, "n", now);
                same(
                    &view,
                    &source,
                    scene::LABEL_SIZE,
                    &format!("{text:?} in {kind} {shape}"),
                );
            }
        }
        // A connector's first label, where the scene centres it.
        let mut view = sized(fixtures::HAND_WRITTEN);
        open_label(&mut view, "flow", now);
        assert_eq!(label(&view).text(), "then");
        same(&view, &Source::Edge(id("flow")), scene::SMALL_SIZE, "then");
    }

    #[test]
    fn free_text_left_empty_is_deleted_when_this_edit_emptied_or_made_it() {
        let now = Instant::now();
        let deleted = |view: &BoardView, name: &str| {
            view.intents.last()
                == Some(&Intent::Delete {
                    ids: vec![id(name)],
                })
        };
        // Emptied by this edit.
        let mut view = sized(fixtures::HAND_WRITTEN);
        open_label(&mut view, "caption", now);
        assert_eq!(edit(&mut view, LabelAction::Backspace, now), CHANGED);
        assert_eq!(view.board().nodes[&id("caption")].text, "");
        assert!(!deleted(&view, "caption"));
        key(&mut view, Key::Escape, now);
        assert!(deleted(&view, "caption"));
        assert_eq!(view.label(), None);
        // Only white space is empty too.
        let mut view = sized(fixtures::HAND_WRITTEN);
        open_label(&mut view, "caption", now);
        typed(&mut view, " \n ", now);
        view.settle(now);
        assert!(deleted(&view, "caption"));
        // A shape may have an empty label.
        let mut view = sized(fixtures::HAND_WRITTEN);
        open_label(&mut view, "first", now);
        edit(&mut view, LabelAction::Backspace, now);
        key(&mut view, Key::Escape, now);
        assert!(!deleted(&view, "first"));
        assert_eq!(view.board().nodes[&id("first")].text, "");
        // Free text that was empty before, opened and closed, is left.
        let empty = one("text", "rect");
        let mut view = sized(&empty);
        open_label(&mut view, "n", now);
        key(&mut view, Key::Escape, now);
        assert!(view.intents.is_empty());
        assert_eq!(view.draft(), empty);
        // Unless this edit's own creation made it, as a plan that makes a
        // node to be labelled says while its acts are carried out.
        let mut view = sized(&empty);
        view.making = Some(id("n"));
        open_label(&mut view, "n", now);
        view.making = None;
        assert!(label(&view).made);
        key(&mut view, Key::Escape, now);
        assert!(deleted(&view, "n"));
        // A made node that was given a label is kept.
        let mut view = sized(&empty);
        view.making = Some(id("n"));
        open_label(&mut view, "n", now);
        typed(&mut view, "Note", now);
        key(&mut view, Key::Escape, now);
        assert!(!deleted(&view, "n"));
        assert_eq!(view.board().nodes[&id("n")].text, "Note");
    }

    #[test]
    fn a_connector_s_label_is_edited_where_the_scene_centres_it() {
        let now = Instant::now();
        // One with a label: each change is the connector's own intent.
        let mut view = sized(fixtures::HAND_WRITTEN);
        open_label(&mut view, "flow", now);
        assert_eq!(label(&view).text(), "then");
        assert_eq!(label(&view).selection(), 0..4);
        typed(&mut view, "next", now);
        typed(&mut view, "\n", now);
        // The text as it is stored, without the trailing line feed.
        let next = Intent::SetEdgeLabel {
            id: id("flow"),
            text: "next".to_owned(),
        };
        assert_eq!(view.intents, [next.clone(), next]);
        // Erased, the label is asked away: an empty text removes it.
        edit(&mut view, LabelAction::SelectAll, now);
        edit(&mut view, LabelAction::Backspace, now);
        assert_eq!(
            view.intents.last(),
            Some(&Intent::SetEdgeLabel {
                id: id("flow"),
                text: String::new()
            })
        );
        key(&mut view, Key::Escape, now);
        assert_eq!(view.label(), None);

        // One with none: the caret is at the middle of the route, which
        // runs from (120, 30) to (300, 30).
        let mut view = sized(TWO);
        open_label(&mut view, "e", now);
        assert_eq!(label(&view).text(), "");
        let (caret, size) = view.label_caret().expect("a caret");
        assert!((caret.x - 210.0).abs() < 0.01, "{caret:?}");
        assert_eq!((caret.y, caret.height), (30.0 - 8.0, 16.0));
        assert_eq!(size, scene::SMALL_SIZE);
        key(&mut view, Key::Escape, now);
        assert!(view.intents.is_empty());
        assert!(view.pristine());
    }

    /// A route whose last corner is under its arrowhead: the scene's line
    /// leaves the corner out, so the label's place is read from the box
    /// the scene draws behind it, never worked out again from the line.
    #[test]
    fn a_connector_s_label_is_where_the_scene_drew_it_whatever_the_route() {
        let board = r#"schema = 1

[node.a]
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "a"

[node.b]
kind = "shape"
at = [136, 200]
size = [120, 60]
z = "b"

[edge.e]
kind = "edge"
from = { node = "a", side = "right" }
to = { node = "b", side = "left" }
labels = [{ text = "Yes" }]
z = "c"
"#;
        let mut view = sized(board);
        let now = Instant::now();
        open_label(&mut view, "e", now);
        let Some(Draw::Text { lines, .. }) = view
            .scene()
            .items
            .iter()
            .filter(|item| item.source == Source::Edge(id("e")))
            .map(|item| &item.draw)
            .find(|draw| matches!(draw, Draw::Text { .. }))
        else {
            panic!("a label");
        };
        let (text, x, y) = label(&view).lines()[0];
        assert_eq!(text, "Yes");
        assert!((x - lines[0].origin.x).abs() < 0.01 && (y - lines[0].origin.y).abs() < 0.01);
        let centre = (
            lines[0].origin.x + lines[0].width / 2.0,
            lines[0].origin.y + 8.0,
        );
        // With no label yet, the caret is where the first character's line
        // will be centred, so nothing moves when it is typed.
        let mut blank = sized(&board.replace("labels = [{ text = \"Yes\" }]\n", ""));
        open_label(&mut blank, "e", now);
        let caret = blank.label().expect("a label").caret_box(blank.measurer());
        assert!(
            (caret.x - centre.0).abs() < 0.01,
            "{caret:?} for {centre:?}"
        );
        assert!(
            (caret.y + 8.0 - centre.1).abs() < 0.01,
            "{caret:?} for {centre:?}"
        );
        assert!(blank.pristine());
        assert_eq!(blank.board().edges[&id("e")].labels, []);
    }

    #[test]
    fn a_connector_the_scene_does_not_route_opens_no_label() {
        let dangling = format!("{TWO}\n[edge.stub]\nkind = \"edge\"\nfrom = {{ node = \"a\" }}\nto = {{ node = \"gone\" }}\nz = \"d\"\n");
        let mut view = sized(&dangling);
        let now = Instant::now();
        assert_eq!(open_label(&mut view, "stub", now), Update::default());
        assert_eq!(view.label(), None);
        assert_eq!(view.machine().editing(), None);
        assert_eq!(view.machine().state(), machine::State::Idle);
        assert_eq!(selected(&view), ["stub"]);
    }

    #[test]
    fn the_pointer_places_the_caret_and_drags_a_selection() {
        let mut view = sized(fixtures::HAND_WRITTEN);
        let now = Instant::now();
        open_label(&mut view, "first", now);
        let lines = label(&view).lines();
        let (text, x, y) = lines[0];
        assert_eq!(text, "First step");
        let width = |text: &str| view.measurer().width(text, scene::Face::Regular, 14.0);
        let screen = |board: (f32, f32)| view.viewport().to_screen(board);
        // Between "First" and " step", and at the end of the line.
        let middle = screen((x + width("First") + 0.5, y + 10.0));
        let end = screen((x + width("First step") + 0.5, y + 10.0));
        assert_eq!(input(&mut view, press(middle), now), Update::default());
        assert_eq!((label(&view).caret(), label(&view).selection()), (5, 5..5));
        assert_eq!(view.machine().state(), machine::State::Editing);
        assert_eq!(
            input(&mut view, Input::Move { at: end }, now),
            Update::default()
        );
        assert_eq!(
            (label(&view).caret(), label(&view).selection()),
            (10, 5..10)
        );
        assert_eq!(input(&mut view, RELEASE, now), Update::default());
        assert_eq!(view.machine().editing(), Some(&id("first")));
        assert!(view.pristine());
        // What is typed replaces what the drag selected.
        typed(&mut view, "!", now);
        assert_eq!(view.board().nodes[&id("first")].text, "First!");
        // A press on another node commits, and is that node's press.
        let other = self::middle(&view, "second");
        input(&mut view, press(other), now);
        assert_eq!(view.label(), None);
        input(&mut view, RELEASE, now);
        assert_eq!(selected(&view), ["second"]);
        assert_eq!(view.board().nodes[&id("first")].text, "First!");
    }

    #[test]
    fn a_label_too_small_to_read_opens_at_one_hundred_percent() {
        let now = Instant::now();
        let at = |zoom| Viewport {
            x: -200.0,
            y: -100.0,
            zoom,
        };
        // 14 px text at 50 % is 7 px: the view goes to 100 % with the
        // node, whose middle is at (60, 30), in the middle of the widget.
        let mut view = sized(TWO);
        view.set_viewport(at(0.5));
        let jumps = view.jumps();
        open_label(&mut view, "a", now);
        assert_eq!(
            view.viewport(),
            Viewport {
                x: 60.0 - SIZE.0 / 2.0,
                y: 30.0 - SIZE.1 / 2.0,
                zoom: 1.0
            }
        );
        assert_eq!(view.jumps(), jumps + 1);
        // At 75 % it is 10.5 px and the view stays.
        let mut view = sized(TWO);
        view.set_viewport(at(0.75));
        open_label(&mut view, "a", now);
        assert_eq!(view.viewport(), at(0.75));
        // A connector's 12 px label is 9 px there.
        let mut view = sized(TWO);
        view.set_viewport(at(0.75));
        open_label(&mut view, "e", now);
        assert_eq!(view.viewport().zoom, 1.0);
        let (caret, _) = view.label_caret().expect("a caret");
        assert!((caret.x - SIZE.0 / 2.0).abs() < 0.01);
        // Before the widget has a size, the node stays where it is.
        let mut view = open(TWO);
        view.set_viewport(at(0.5));
        let before = view.viewport().to_screen((60.0, 30.0));
        open_label(&mut view, "a", now);
        assert_eq!(view.viewport().zoom, 1.0);
        let after = view.viewport().to_screen((60.0, 30.0));
        assert!((after.0 - before.0).abs() < 0.01 && (after.1 - before.1).abs() < 0.01);
        // A node out of view is brought into it, at the zoom it has.
        let mut view = sized(TWO);
        view.set_viewport(Viewport {
            x: 2000.0,
            y: 2000.0,
            zoom: 1.0,
        });
        open_label(&mut view, "b", now);
        let bounds = Rect::new(300.0, 0.0, 120.0, 60.0);
        assert_eq!(view.viewport(), revealing(view.viewport(), SIZE, bounds));
        assert!(shows(view.viewport(), SIZE, bounds));
        // One in view is left where it is.
        let mut view = sized(TWO);
        open_label(&mut view, "b", now);
        assert_eq!(view.viewport(), Viewport::default());
    }

    #[test]
    fn a_reload_closes_the_label_without_committing_it() {
        let mut view = sized(TWO);
        let now = Instant::now();
        open_label(&mut view, "a", now);
        typed(&mut view, "Typed", now);
        let epoch = view.epoch();
        view.reload(fixtures::HAND_WRITTEN).expect("a board");
        assert_eq!(view.label(), None);
        assert_eq!(view.machine().state(), machine::State::Idle);
        let count = view.intents.len();
        // What the widget sent for the board before is dropped.
        for message in [
            Message::Label(epoch, LabelAction::Insert("x".to_owned())),
            Message::Pasted(epoch, "x".to_owned()),
        ] {
            assert_eq!(view.update(message, now), Update::default());
        }
        assert_eq!(view.intents.len(), count);
        // Even with a label open on the board that replaced it.
        open_label(&mut view, "first", now);
        let stale = Message::Label(epoch, LabelAction::Insert("x".to_owned()));
        assert_eq!(view.update(stale, now), Update::default());
        let stale = Message::Pasted(epoch, "x".to_owned());
        assert_eq!(view.update(stale, now), Update::default());
        assert_eq!(label(&view).text(), "First step");
        assert!(view.pristine());
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
    }

    #[test]
    fn label_input_with_no_label_open_is_dropped() {
        let mut view = sized(TWO);
        let now = Instant::now();
        click_node(&mut view, "a", now);
        assert_eq!(typed(&mut view, "x", now), Update::default());
        assert_eq!(
            view.update(Message::Pasted(view.epoch(), "x".to_owned()), now),
            Update::default()
        );
        assert!(view.pristine());
        // Nor does a board that may not be edited open one.
        let mut view = sized(fixtures::FUTURE);
        let node = view.board().nodes.keys().next().expect("a node").clone();
        view.machine
            .select(BTreeSet::from([node.clone()]), Some(node));
        view.command(Command::EditLabel, now);
        assert_eq!(view.label(), None);
        assert_eq!(typed(&mut view, "x", now), Update::default());
        assert!(view.pristine());
    }

    #[test]
    fn the_clipboard_and_a_full_label_are_the_host_s_to_act_on() {
        let mut view = sized(TWO);
        let now = Instant::now();
        open_label(&mut view, "a", now);
        typed(&mut view, "copy me", now);
        edit(&mut view, LabelAction::SelectAll, now);
        let effects = |effects: &[Effect]| Update {
            changed: false,
            effects: effects.to_vec(),
        };
        assert_eq!(
            edit(&mut view, LabelAction::Copy, now),
            effects(&[Effect::ClipboardWrite("copy me".to_owned())])
        );
        let cut = edit(&mut view, LabelAction::Cut, now);
        assert!(cut.changed);
        assert_eq!(cut.effects, [Effect::ClipboardWrite("copy me".to_owned())]);
        assert_eq!(view.board().nodes[&id("a")].text, "");
        // A paste asks for the clipboard, and its text comes back.
        assert_eq!(
            edit(&mut view, LabelAction::Paste, now),
            effects(&[Effect::ClipboardRead])
        );
        let pasted = Message::Pasted(view.epoch(), "from\r\noutside".to_owned());
        assert_eq!(view.update(pasted, now), CHANGED);
        assert_eq!(view.board().nodes[&id("a")].text, "from\noutside");
        // A board fragment pastes its labels.
        edit(&mut view, LabelAction::SelectAll, now);
        let fragment = format!("# wren-board fragment 1\n{TWO}");
        view.update(Message::Pasted(view.epoch(), fragment), now);
        assert_eq!(view.board().nodes[&id("a")].text, "A\nB");
        // More than a label holds is refused whole, and said.
        let long = "x".repeat(scene::MAX_LABEL_BYTES);
        let refused = view.update(Message::Pasted(view.epoch(), long), now);
        assert_eq!(refused, effects(&[Effect::Notice(LABEL_FULL.to_owned())]));
        assert_eq!(view.board().nodes[&id("a")].text, "A\nB");
    }

    #[test]
    fn a_command_commits_the_label_and_then_runs() {
        let mut view = sized(TWO);
        let now = Instant::now();
        open_label(&mut view, "a", now);
        typed(&mut view, "Moved", now);
        assert_eq!(view.command(Command::Move(Dir::Right), now), CHANGED);
        assert_eq!(view.label(), None);
        let node = &view.board().nodes[&id("a")];
        assert_eq!((node.text.as_str(), node.at), ("Moved", [20, 0]));
        // Tab commits, then extends from the node.
        open_label(&mut view, "a", now);
        typed(&mut view, "Next", now);
        view.command(Command::ConnectNext, now);
        assert_eq!(
            view.intents.last(),
            Some(&Intent::Extend {
                from: id("a"),
                dir: None,
                shape: None
            })
        );
        assert_eq!(view.board().nodes[&id("a")].text, "Next");
    }

    #[test]
    fn a_composition_is_never_committed_with_the_label() {
        let mut view = sized(TWO);
        let now = Instant::now();
        open_label(&mut view, "a", now);
        step(&mut view, Motion::End, false, now);
        let composing = LabelAction::Preedit {
            text: "にほ".to_owned(),
            selection: None,
        };
        assert_eq!(edit(&mut view, composing.clone(), now), Update::default());
        assert!(label(&view).preedit().is_some());
        assert!(view.pristine());
        // Closing the label mid-composition takes none of it.
        assert_eq!(view.settle(now), Update::default());
        assert_eq!(view.label(), None);
        assert_eq!(view.board().nodes[&id("a")].text, "A");
        assert!(view.pristine());
        // The commit, when it comes, is text like any other.
        open_label(&mut view, "a", now);
        step(&mut view, Motion::End, false, now);
        edit(&mut view, composing, now);
        assert_eq!(typed(&mut view, "日本", now), CHANGED);
        assert_eq!(label(&view).preedit(), None);
        assert_eq!(view.board().nodes[&id("a")].text, "A日本");
    }

    /// The caret is placed by measuring what is before it on its line,
    /// white space at the end included, which the scene's line leaves out.
    #[test]
    fn a_typed_space_moves_the_caret() {
        let mut view = sized(TWO);
        let now = Instant::now();
        open_label(&mut view, "a", now);
        step(&mut view, Motion::End, false, now);
        let (before, _) = view.label_caret().expect("a caret");
        assert_eq!(typed(&mut view, " ", now), CHANGED);
        let (after, _) = view.label_caret().expect("a caret");
        assert!(after.x > before.x + 2.0, "{before:?} then {after:?}");
        assert_eq!(after.y, before.y);
        // The line the scene draws has not moved.
        assert_eq!(label(&view).lines()[0].0, "A");
        assert_eq!(view.board().nodes[&id("a")].text, "A ");
    }

    #[test]
    fn a_new_theme_recolours_the_caret() {
        let mut view = sized(TWO);
        let now = Instant::now();
        open_label(&mut view, "a", now);
        let before = label(&view).ink;
        let mut palette = fixtures::palette();
        palette.ink = Rgba::rgb(1, 2, 3);
        view.set_palette(palette);
        assert_ne!(label(&view).ink, before);
        assert_eq!(label(&view).ink, Rgba::rgb(1, 2, 3));
    }
}
