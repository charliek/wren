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
//! for, shows a gesture's preview on a scratch copy of the board, keeps the
//! pristine text a draft falls back to, and holds the undo history, whose
//! floor that text is (§3.4 and §5.11 of the plan). The one
//! label that is open is the view's too ([`Label`]): its keys arrive as
//! [`LabelAction`]s, and each change of its text is an edit of the board
//! at once (§5.9). With the `test-support` feature a view counts its scene
//! and geometry builds (`Probe`).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iced::{Element, Font};
use wren_board::edit::{self, Area, Intent};
use wren_board::history::{History, Snapshot, Unit};
use wren_board::ids::Random;
use wren_board::scene::{self, Draw, Rect, Scene, Source};
use wren_board::{
    style, walk, Access, Board, Dir, Extra, Fill, Id, Issue, Named, Node, NodeKind, NodeStyle,
    Palette, ParseError, Rgba, Shape, GRID,
};

#[cfg(test)]
mod accept;
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

/// The widest that pasted text from outside is laid out before it wraps,
/// in board units (§5.10 rule 9).
pub const PASTE_WRAP: i32 = 320;

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
    /// A key pressed on the focused board, stamped with the epoch it was
    /// meant for. It is read when it is applied ([`BoardView::press`]),
    /// not when the widget sent it: iced applies the messages of a batch
    /// of events only after the whole batch, and a key pressed quickly
    /// after another must be read in the state that one leaves.
    Key(u64, KeyPress),
    /// A key the board layer binds, stamped with the epoch it was meant
    /// for. The host runs it through its own command path, which reaches
    /// [`BoardView::command`], unless the board has changed since.
    Command(u64, Command),
    /// The clipboard's text, read for an [`Effect::ClipboardRead`] and
    /// stamped with the epoch of the view that asked. It goes to what asked,
    /// the open label or the board, and is dropped if that has gone.
    Pasted(u64, String),
}

/// One key press, as the widget was given it.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyPress {
    pub key: iced::keyboard::Key,
    pub physical: iced::keyboard::key::Physical,
    pub modifiers: iced::keyboard::Modifiers,
    pub text: Option<String>,
    pub repeat: bool,
    /// Whether the widget saw this key go down and it has not come up
    /// since, so a repeat is of a press the view has read.
    pub held: bool,
}

/// Where a view draws the ids of the elements it makes.
#[derive(Clone, Debug)]
enum Draws {
    Random(Random),
    #[cfg(feature = "test-support")]
    Seeded(wren_board::ids::Seeded),
}

impl wren_board::ids::Ids for Draws {
    fn draw(&mut self) -> wren_board::Id {
        match self {
            Draws::Random(random) => random.draw(),
            #[cfg(feature = "test-support")]
            Draws::Seeded(seeded) => seeded.draw(),
        }
    }
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

/// An epoch no view has had. Epochs are not per view: input meant for one
/// board never passes for another's, when the host opens the next board
/// while a message for the last is still on its way.
fn epoch() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The board, the selection and the cursor as they are: what undo goes
/// back to (§5.11 rule 1).
fn snapshot(board: &Board, machine: &Machine) -> Snapshot {
    Snapshot {
        board: board.clone(),
        selection: machine.selection().clone(),
        cursor: machine.cursor().cloned(),
    }
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
    ids: Draws,
    viewport: Viewport,
    revision: u64,
    machine: Machine,
    /// Replaced whenever the board changes underneath its input, so input
    /// meant for the board before is dropped; see [`epoch`].
    epoch: u64,
    /// The widget's size, once it has said.
    size: Option<(f32, f32)>,
    /// A fit asked for before the widget's size was known.
    fit_pending: bool,
    /// Bumped by each discrete change of view (a zoom step, a reset, a
    /// fit), which the widget draws in full at once instead of as a move.
    jumps: u64,
    key_policy: Option<KeyPolicy>,
    /// The key whose first press was read last, and whether a label was
    /// open then: the repeats of a key held since before a label opened
    /// are not its text.
    pressed: Option<(iced::keyboard::key::Physical, bool)>,
    /// The open label (§5.9): the machine says whose it is
    /// ([`Machine::editing`]), the view holds its text.
    label: Option<Label>,
    /// The node an edit in progress made to be labelled.
    making: Option<Id>,
    /// Undo and redo (§5.11). A group is open only while a label is: the
    /// creation that opened it and what is typed there are one unit.
    history: History,
    /// The clipboard text last pasted into this board and how many times,
    /// which sets each paste one cell further (§5.10 rule 8).
    pasted: Option<(String, u32)>,
    /// How many labels have opened, which tells one open label from the
    /// next.
    labels_opened: u64,
    /// Whom the last clipboard read was for: its text goes there or
    /// nowhere.
    paste_for: Option<PasteFor>,
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

/// Whom a clipboard read was for, by the count of labels opened when it
/// was asked: the label open then, or the board while no label was. The
/// text comes back later, and by then a label may have opened or closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteFor {
    Label(u64),
    Board(u64),
}

/// What the widget paints: the board, or a gesture's preview of it.
pub(crate) struct Picture<'a> {
    pub(crate) board: &'a Board,
    pub(crate) scene: &'a Scene,
    pub(crate) plan: &'a layers::Plan,
    pub(crate) revision: u64,
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
            ids: Draws::Random(Random::new()),
            viewport: Viewport::default(),
            revision: 0,
            machine: Machine::default(),
            epoch: epoch(),
            size: None,
            fit_pending: false,
            jumps: 0,
            key_policy: None,
            pressed: None,
            label: None,
            making: None,
            history: History::new(),
            pasted: None,
            labels_opened: 0,
            paste_for: None,
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
    /// flight and an open label are dropped without a commit, the undo
    /// history is dropped, the selection keeps what still exists, and the
    /// epoch moves on.
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
        self.history.clear();
        self.machine.reset();
        self.machine.retain(&self.board);
        self.epoch = epoch();
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
    /// still waiting for the widget's size. A move puts the hover connect
    /// handles away.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        if viewport.x.is_finite() && viewport.y.is_finite() && viewport.zoom.is_finite() {
            let viewport = Viewport {
                zoom: viewport.zoom.clamp(Viewport::MIN_ZOOM, Viewport::MAX_ZOOM),
                ..viewport
            };
            if viewport != self.viewport {
                self.machine.unhover();
            }
            self.viewport = viewport;
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
                    size: self.size,
                };
                let target = self.seek_target();
                let acts = self.machine.step(input, &view);
                self.carry(acts, Unit::One, now, &mut update);
                self.keep_label();
                // Keyboard Connecting's target is brought into view as the
                // cursor is when a key moves it (§5.7 rule 5).
                let now_target = self.seek_target();
                if let Some(id) = now_target.as_ref().filter(|_| now_target != target) {
                    self.reveal_element(id);
                }
            }
            Message::Label(epoch, action) if epoch == self.epoch => {
                self.label_action(action, now, &mut update);
            }
            Message::Pasted(epoch, text) if epoch == self.epoch => {
                let opened = self.labels_opened;
                match (self.paste_for, &mut self.label) {
                    (Some(PasteFor::Label(n)), Some(label)) if n == opened => {
                        let outcome = label.paste(&text, &self.measurer);
                        self.label_did(&outcome, now, &mut update);
                    }
                    (Some(PasteFor::Board(n)), None) if n == opened => {
                        self.paste(text, now, &mut update);
                    }
                    // Asked for by a label that has closed since, or by the
                    // board before a label opened.
                    _ => {}
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
            Message::Key(epoch, press) if epoch == self.epoch => {
                for message in self.press(&press).into_iter().flatten() {
                    let did = self.update(message, now);
                    update.changed |= did.changed;
                    update.effects.extend(did.effects);
                }
            }
            Message::Command(epoch, command) if epoch == self.epoch => {
                return self.command(command, now);
            }
            Message::Input(..)
            | Message::Label(..)
            | Message::Key(..)
            | Message::Command(..)
            | Message::Pasted(..) => {}
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
            size: self.size,
        };
        let acts = self.machine.command(command, &view);
        // A key move or key resize may join the run before it (§5.11 rule
        // 3); every other command is a unit of its own.
        let unit = match command {
            Command::Move(_) => Unit::Move,
            Command::Resize(_) => Unit::Resize,
            _ => Unit::One,
        };
        self.carry(acts, unit, now, &mut update);
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
        self.carry(acts, Unit::One, now, &mut update);
        self.keep_label();
        self.preview = None;
        update
    }

    /// Whether there is a unit to undo (§5.11).
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether there is a unit to redo.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
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

    /// Draw the ids of new elements from `seed` from now on, so a test or
    /// a scene that builds a board makes the same board every run: routes
    /// depend on where ids sort (§3.7).
    #[cfg(feature = "test-support")]
    pub fn seed_ids(&mut self, seed: u64) {
        self.ids = Draws::Seeded(wren_board::ids::Seeded::new(seed));
    }

    /// What the focused board makes of a press of `key`, which typed
    /// `text` (§5.1): `None` when the key is the host's, else what the
    /// board sends for it in order, which is nothing for a key that only
    /// must not reach the host. `held_from_before` is whether the key went
    /// down before the open label opened, as the Enter that opens one did.
    /// The widget asks this for whether the board takes a key, and
    /// [`BoardView::press`] for what the key does once the keys before it
    /// are applied; a host that delivers keys one at a time itself, as a
    /// test harness does, may ask this alone.
    pub fn key(
        &self,
        key: &iced::keyboard::Key,
        physical: iced::keyboard::key::Physical,
        modifiers: iced::keyboard::Modifiers,
        text: Option<&str>,
        repeat: bool,
        held_from_before: bool,
    ) -> Option<Vec<Message>> {
        let context = keys::Context {
            editing: self.machine.editing().is_some(),
            connecting: self.machine.seeking().is_some(),
            composing: self
                .label
                .as_ref()
                .is_some_and(|label| label.preedit().is_some()),
            escapes: self.machine.escapes(),
            held_from_before,
            policy: self.key_policy.as_ref(),
        };
        let keys::Keyed::Take(outs) = keys::route(key, physical, modifiers, text, repeat, context)
        else {
            return None;
        };
        let epoch = self.epoch;
        Some(
            outs.into_iter()
                .map(|out| match out {
                    keys::Out::Input(input) => Message::Input(epoch, input),
                    keys::Out::Command(command) => Message::Command(epoch, command),
                    keys::Out::Label(action) => Message::Label(epoch, action),
                })
                .collect(),
        )
    }

    /// What the board makes of `press` in the state it is in now: what
    /// [`BoardView::key`] says, with whether a repeat is of a key held
    /// since before the open label opened. A host that applies
    /// [`Message::Key`] itself, to send commands through its own path,
    /// asks this.
    pub fn press(&mut self, press: &KeyPress) -> Option<Vec<Message>> {
        if !press.repeat {
            self.pressed = Some((press.physical, self.machine.editing().is_some()));
        }
        let held_from_before = press.held && self.pressed == Some((press.physical, false));
        self.key(
            &press.key,
            press.physical,
            press.modifiers,
            press.text.as_deref(),
            press.repeat,
            held_from_before,
        )
    }

    /// Apply `intents` in order as one undo unit, each planned on the board
    /// the ones before it left, with new ids drawn from `ids`: how a host
    /// arranges a board without the pointer or the keys (the test harness's
    /// `board.apply`, §6.2). A label open or a gesture in flight is settled
    /// first, as its own unit. Nothing is selected or opened by it, and
    /// what it removes leaves the selection. An intent the board refuses
    /// refuses them all, with its notice among the effects, and changes
    /// nothing.
    pub fn apply(
        &mut self,
        intents: &[Intent],
        ids: &mut dyn wren_board::ids::Ids,
        now: Instant,
    ) -> Update {
        let mut update = self.settle(now);
        let mut board = self.board.clone();
        for intent in intents {
            let plan = edit::plan(&board, intent, ids);
            if let (true, Some(notice)) = (plan.is_empty(), plan.notice) {
                update.effects.push(Effect::Notice(notice));
                return update;
            }
            board.apply(&plan.changes);
        }
        if wren_board::write(&board) == wren_board::write(&self.board) {
            return update;
        }
        self.history.break_run();
        self.history.record(self.snapshot(), Unit::One, now);
        self.board = board;
        self.issues = wren_board::check(&self.board);
        self.build();
        self.machine.retain(&self.board);
        update.changed = true;
        update
    }

    /// Which board the view's input is for; see [`Message::Input`]. No two
    /// views share one, and a reload gives the view another.
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

    /// Carry out what the machine asked for, in order. `unit` is how an
    /// edit among them joins the undo history.
    fn carry(&mut self, acts: Vec<machine::Act>, unit: Unit, now: Instant, update: &mut Update) {
        use machine::Act;
        for act in acts {
            match act {
                Act::Look(viewport) => self.set_viewport(viewport),
                // Planned and applied alike; an edit that does not
                // continue a run of key moves begins a unit of its own.
                Act::Edit(intent) => {
                    self.history.break_run();
                    self.edit(&intent, unit, now, update);
                }
                Act::Continue(intent) => {
                    self.edit(&intent, unit, now, update);
                }
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

    /// Plan `intent` on the board and apply it, once (§3.4 rule 4), as
    /// `unit` of the undo history. A preview on show is over. The machine
    /// takes the plan back, which is how a creation selects what it made.
    /// Returns whether the board changed.
    fn edit(&mut self, intent: &Intent, unit: Unit, now: Instant, update: &mut Update) -> bool {
        self.preview = None;
        #[cfg(test)]
        self.intents.push(intent.clone());
        let plan = edit::plan(&self.board, intent, &mut self.ids);
        let changed = !plan.is_empty();
        if changed {
            // A creation that opens its label begins a group, which the
            // label's typing joins (§3.4 rule 4). A change that joins the
            // open group needs no snapshot of its own, and a label sends
            // one a keystroke.
            if plan.edit.is_some() {
                self.history.open_group(self.snapshot());
            } else if unit != Unit::Label || !self.history.group_open() {
                let (board, machine) = (&self.board, &self.machine);
                self.history
                    .record_with(unit, machine.selection(), now, || snapshot(board, machine));
            }
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
        self.carry(acts, Unit::One, now, update);
        self.making = None;
        changed
    }

    /// The board, the selection and the cursor as they are: what undo
    /// goes back to (§5.11 rule 1).
    fn snapshot(&self) -> Snapshot {
        snapshot(&self.board, &self.machine)
    }

    /// `board.undo` and `board.redo` (§5.11): the board, the selection and
    /// the cursor of the snapshot; the viewport stays.
    fn travel(&mut self, forward: bool, update: &mut Update) {
        let possible = if forward {
            self.history.can_redo()
        } else {
            self.history.can_undo()
        };
        if !possible {
            return;
        }
        let current = self.snapshot();
        let to = if forward {
            self.history.redo(current)
        } else {
            self.history.undo(current)
        };
        // A group that came to nothing leaves nothing to go back to.
        let Some(to) = to else {
            return;
        };
        self.board = to.board;
        self.issues = wren_board::check(&self.board);
        self.build();
        self.machine.select(to.selection, to.cursor);
        self.machine.retain(&self.board);
        update.changed = true;
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
        self.labels_opened += 1;
        let mut label = Label::open(id.clone(), text, place, &self.measurer);
        label.ink = ink;
        label.size = self.board.nodes.get(&id).map(|node| node.size);
        label.made = self.making.as_ref() == Some(&id)
            || self
                .board
                .edges
                .get(&id)
                .is_some_and(|edge| edge.labels.is_empty());
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
    /// to do is to delete free text or a connector label that ends empty,
    /// when this edit emptied it or made it.
    fn commit_label(&mut self, now: Instant, update: &mut Update) {
        let Some(label) = self.label.take() else {
            return;
        };
        self.finish_label(&label, now, update);
        // The creation that opened the label and what was typed in it are
        // one unit, and none when the board is as it was (§3.4 rule 4).
        self.history.close_group(&self.board);
    }

    /// What is left to do for a label that closes: delete free text or a
    /// connector label that ends empty. It belongs to the label's unit.
    fn finish_label(&mut self, label: &Label, now: Instant, update: &mut Update) {
        if !label.stored().is_empty() || !(label.changed() || label.made) {
            return;
        }
        let id = label.id().clone();
        let free =
            self.board.nodes.get(&id).is_some_and(|node| {
                node.kind.as_ref().and_then(Named::known) == Some(NodeKind::Text)
            });
        let labelled = !self.board.nodes.contains_key(&id)
            && self
                .board
                .edges
                .get(&id)
                .is_some_and(|edge| !edge.labels.is_empty());
        if free {
            self.edit(&Intent::Delete { ids: vec![id] }, Unit::Label, now, update);
        } else if labelled {
            let intent = Intent::SetEdgeLabel {
                id,
                text: String::new(),
                commit: true,
            };
            self.edit(&intent, Unit::Label, now, update);
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
            self.paste_for = Some(PasteFor::Label(self.labels_opened));
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
            None => Intent::SetEdgeLabel {
                id,
                text,
                commit: false,
            },
        };
        self.edit(&intent, Unit::Label, now, update);
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
        // No label, no group: one that was abandoned, or whose element an
        // edit removed, ends here.
        if self.label.is_none() {
            self.history.close_group(&self.board);
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

    /// Show the board as `intent` would leave it, or as it is. The plan
    /// draws the ids the edit that ends the gesture will draw, from a copy
    /// of their source, so a release makes the very board its preview
    /// showed: a connector's route depends on where its id sorts (§3.7).
    fn show(&mut self, intent: Option<&Intent>) {
        self.preview = intent.and_then(|intent| {
            let plan = edit::plan(&self.board, intent, &mut self.ids.clone());
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
            Command::Delete if editable => self.delete(ids, now, update),
            Command::Duplicate if editable => {
                let by = [GRID, GRID];
                self.edit(&Intent::Duplicate { ids, by }, Unit::One, now, update);
            }
            // A selection with no node has no fragment and copies nothing
            // (§5.10 rule 2). A read-only board copies too.
            Command::Copy => {
                if let Some(text) = wren_board::clip::copy(&self.board, &ids) {
                    update.effects.push(Effect::ClipboardWrite(text));
                }
            }
            // Copy, then delete what was selected: one unit (rule 5).
            Command::Cut if editable => {
                if let Some(text) = wren_board::clip::copy(&self.board, &ids) {
                    update.effects.push(Effect::ClipboardWrite(text));
                    self.delete(ids, now, update);
                }
            }
            // The text comes back as `Message::Pasted`.
            Command::Paste if self.writable() && self.label.is_none() => {
                self.paste_for = Some(PasteFor::Board(self.labels_opened));
                update.effects.push(Effect::ClipboardRead);
            }
            Command::Undo => self.travel(false, update),
            Command::Redo => self.travel(true, update),
            Command::EdgeRouting
            | Command::EdgeStart
            | Command::EdgeEnd
            | Command::EdgeReverse
            | Command::StrokeDashed
                if editable =>
            {
                if let Some(intent) = edge_style(&self.board, &ids, command) {
                    self.edit(&intent, Unit::One, now, update);
                }
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

    /// Delete `ids` as one unit (§5.10 rule 1). When that takes the cursor
    /// with it, the cursor moves to the remaining node nearest where it
    /// was, by centre, ties by id; nothing is selected after.
    fn delete(&mut self, ids: Vec<Id>, now: Instant, update: &mut Update) {
        let cursor = self.machine.cursor();
        let was = cursor.and_then(|id| bounds(&self.board, &self.scene, id));
        if !self.edit(&Intent::Delete { ids }, Unit::One, now, update) {
            return;
        }
        if self.machine.cursor().is_none() {
            let nearest = was.and_then(|was| nearest(&self.board, was.center()));
            self.machine.select(BTreeSet::new(), nearest);
        }
    }

    /// Paste the clipboard's `text` into the board (§5.10 rules 6 to 9): a
    /// copied fragment one cell further each time the same text is pasted,
    /// or text from outside as one text node in the middle of the view,
    /// sized here for its lines. One unit; a gesture in flight keeps the
    /// board to itself, as it does from a command.
    fn paste(&mut self, text: String, now: Instant, update: &mut Update) {
        if !self.writable() || self.machine.dragging() {
            return;
        }
        let nth = match &self.pasted {
            Some((last, count)) if *last == text => count.saturating_add(1),
            _ => 1,
        };
        let (width, height) = self.size.unwrap_or_default();
        let (left, top) = self.viewport.to_board((0.0, 0.0));
        let (right, bottom) = self.viewport.to_board((width, height));
        let unit = |value: f32| value.round().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
        let view = Area {
            at: [unit(left), unit(top)],
            size: [unit(right - left), unit(bottom - top)],
        };
        let size = if wren_board::clip::is_fragment(&text) {
            [0, 0]
        } else {
            self.pasted_size(&wren_board::clip::label(&text))
        };
        self.history.break_run();
        let intent = Intent::Paste {
            text: text.clone(),
            size,
            view,
            nth,
        };
        if self.edit(&intent, Unit::One, now, update) {
            self.pasted = Some((text, nth));
            self.reveal();
        }
    }

    /// The size of a text node for `text` pasted from outside (§5.10 rule
    /// 9): as wide as its longest line in whole cells, at most
    /// [`PASTE_WRAP`], then grown as a label being typed grows its node.
    fn pasted_size(&self, text: &str) -> [i32; 2] {
        use wren_board::scene::{Face, Measure as _};
        let widest = text
            .lines()
            .map(|line| self.measurer.width(line, Face::Regular, scene::LABEL_SIZE))
            .fold(0.0, f32::max);
        let cells = (widest / GRID as f32).ceil().max(1.0) as i32;
        let width = cells.saturating_mul(GRID).min(PASTE_WRAP);
        let size = [width, GRID];
        let node = Node {
            text: String::new(),
            kind: Some(Named::Known(NodeKind::Text)),
            at: [0, 0],
            parent: None,
            shape: Named::Known(Shape::Rect),
            size,
            style: NodeStyle::default(),
            z: None,
            extra: Extra::new(),
        };
        label::grown(&node, size, text, &self.measurer)
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
        if let Some(cursor) = self.machine.cursor().cloned() {
            self.reveal_element(&cursor);
        }
    }

    /// Pan by the least amount that shows the element `id` with
    /// [`REVEAL_MARGIN`] around it.
    fn reveal_element(&mut self, id: &Id) {
        let Some(size) = self.size else {
            return;
        };
        if let Some(bounds) = bounds(&self.board, &self.scene, id) {
            self.jump(revealing(self.viewport, size, bounds));
        }
    }

    /// The target of keyboard Connecting.
    fn seek_target(&self) -> Option<Id> {
        self.machine.seeking()?.target.cloned()
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

/// The node whose centre is nearest `point`, ties by id: where the cursor
/// goes when what it was on is deleted (§5.10 rule 1).
fn nearest(board: &Board, point: scene::Point) -> Option<Id> {
    let frames = hit::Frames::new(board);
    let distances = board.nodes.keys().filter_map(|id| {
        let centre = frames.get(id)?.center();
        Some(((centre.x - point.x).hypot(centre.y - point.y), id))
    });
    distances
        .min_by(|(a, x), (b, y)| a.total_cmp(b).then_with(|| x.cmp(y)))
        .map(|(_, id)| id.clone())
}

/// What a connector style command does to the selected connectors among
/// `ids` (§5.12 rule 9), or `None` with none selected. Each switch sets
/// every one of them to straight, an arrow or dashed, unless all of them
/// are drawn so already, when it sets elbow, no head or solid: mixed
/// connectors all take the first, as mixed text takes bold. A value this
/// build does not know counts as what it is drawn as (§4.4).
fn edge_style(board: &Board, ids: &[Id], command: Command) -> Option<Intent> {
    use wren_board::edit::{EdgeEnd, EdgeField};
    use wren_board::{Dash, Head, Route};
    let ids: Vec<Id> = ids
        .iter()
        .filter(|id| board.edges.contains_key(*id))
        .cloned()
        .collect();
    if ids.is_empty() {
        return None;
    }
    let all = |has: &dyn Fn(&wren_board::Edge) -> bool| {
        ids.iter().filter_map(|id| board.edges.get(id)).all(has)
    };
    let head = |end: EdgeEnd| {
        let at = usize::from(end == EdgeEnd::To);
        let arrows = all(&|edge| edge.heads[at] != Named::Known(Head::None));
        EdgeField::Head(end, if arrows { Head::None } else { Head::Arrow })
    };
    let field = match command {
        Command::EdgeReverse => return Some(Intent::Reverse { ids }),
        Command::EdgeRouting => {
            let straight = all(&|edge| edge.route != Named::Known(Route::Elbow));
            EdgeField::Route(if straight {
                Route::Elbow
            } else {
                Route::Straight
            })
        }
        Command::EdgeStart => head(EdgeEnd::From),
        Command::EdgeEnd => head(EdgeEnd::To),
        Command::StrokeDashed => {
            let dashed = all(&|edge| edge.style.stroke == Named::Known(Dash::Dashed));
            EdgeField::Stroke(if dashed { Dash::Solid } else { Dash::Dashed })
        }
        _ => return None,
    };
    Some(Intent::SetEdge { ids, field })
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
    use wren_board::{Reason, Rgba, Side};

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
    fn a_text_tool_click_left_empty_leaves_no_node() {
        let now = Instant::now();
        let mut view = sized(wren_board::EMPTY);
        view.command(Command::Tool(Tool::Text), now);
        click(&mut view, (200.0, 200.0), now);
        assert_eq!(view.board().nodes.len(), 1);
        assert!(view.label().is_some());
        key(&mut view, Key::Escape, now);
        assert_eq!(view.label(), None);
        assert!(view.board().nodes.is_empty());
        assert!(view.machine().selection().is_empty());
        assert_eq!(view.draft(), wren_board::EMPTY);
    }

    #[test]
    fn free_text_left_empty_is_deleted_when_this_edit_emptied_or_made_it() {
        let now = Instant::now();
        let deleted = |view: &BoardView, name: &str| {
            let asked = view.intents.last()
                == Some(&Intent::Delete {
                    ids: vec![id(name)],
                });
            asked && !view.board().nodes.contains_key(&id(name))
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
            commit: false,
        };
        assert_eq!(view.intents, [next.clone(), next]);
        // Erased, the label keeps its item while it is edited, and the
        // commit asks it away.
        edit(&mut view, LabelAction::SelectAll, now);
        edit(&mut view, LabelAction::Backspace, now);
        let erased = |commit| Intent::SetEdgeLabel {
            id: id("flow"),
            text: String::new(),
            commit,
        };
        assert_eq!(view.intents.last(), Some(&erased(false)));
        assert_eq!(view.board().edges[&id("flow")].labels.len(), 1);
        key(&mut view, Key::Escape, now);
        assert_eq!(view.label(), None);
        assert_eq!(view.intents.last(), Some(&erased(true)));
        assert!(view.board().edges[&id("flow")].labels.is_empty());

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
        // A label typed and erased in one edit leaves no item behind.
        open_label(&mut view, "e", now);
        typed(&mut view, "x", now);
        edit(&mut view, LabelAction::Backspace, now);
        assert_eq!(view.board().edges[&id("e")].labels.len(), 1);
        key(&mut view, Key::Escape, now);
        assert!(view.board().edges[&id("e")].labels.is_empty());
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
        assert!(view.pristine());
        // Nor does a board that may not be edited open one.
        let mut view = sized(fixtures::FUTURE);
        let node = view.board().nodes.keys().next().expect("a node").clone();
        view.machine
            .select(BTreeSet::from([node.clone()]), Some(node));
        view.command(Command::EditLabel, now);
        assert_eq!(view.label(), None);
        assert_eq!(typed(&mut view, "x", now), Update::default());
        // Nor is a paste the board's own there.
        assert_eq!(
            view.update(Message::Pasted(view.epoch(), "x".to_owned()), now),
            Update::default()
        );
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
    fn the_clipboard_s_text_goes_only_where_it_was_asked_for() {
        let now = Instant::now();
        let escape = Input::Key {
            key: Key::Escape,
            repeat: false,
        };
        // Asked for by a label that closed before the text came back.
        let mut view = sized(TWO);
        open_label(&mut view, "a", now);
        assert_eq!(
            edit(&mut view, LabelAction::Paste, now).effects,
            [Effect::ClipboardRead]
        );
        input(&mut view, escape, now);
        assert_eq!(view.label(), None);
        let late = Message::Pasted(view.epoch(), "meant for the label".to_owned());
        assert_eq!(view.update(late, now), Update::default());
        assert_eq!(view.board().nodes.len(), 2);
        assert!(view.pristine());

        // Asked for by the board before a label opened.
        let mut view = sized(TWO);
        click_node(&mut view, "a", now);
        assert_eq!(
            view.command(Command::Paste, now).effects,
            [Effect::ClipboardRead]
        );
        view.command(Command::EditLabel, now);
        assert_eq!(label(&view).text(), "A");
        let fragment = format!("# wren-board fragment 1\n{TWO}");
        let late = Message::Pasted(view.epoch(), fragment);
        assert_eq!(view.update(late, now), Update::default());
        assert_eq!(label(&view).text(), "A");
        // Nor by a label that has closed and another that opened since.
        edit(&mut view, LabelAction::Paste, now);
        input(&mut view, escape, now);
        open_label(&mut view, "a", now);
        let late = Message::Pasted(view.epoch(), "x".to_owned());
        assert_eq!(view.update(late, now), Update::default());
        assert!(view.pristine());
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
    /// Undo or redo, which must change the board.
    #[track_caller]
    fn travel(view: &mut BoardView, command: Command, now: Instant) {
        assert_eq!(view.command(command, now), CHANGED, "{command:?}");
    }

    fn drag_node(view: &mut BoardView, name: &str, by: (f32, f32), now: Instant) -> Update {
        let (x, y) = middle(view, name);
        input(view, press((x, y)), now);
        input(
            view,
            Input::Move {
                at: (x + by.0, y + by.1),
            },
            now,
        );
        input(view, RELEASE, now)
    }

    #[test]
    fn each_gesture_is_one_undo_step_back_to_the_opening_bytes() {
        let mut view = sized(fixtures::HAND_WRITTEN);
        let now = Instant::now();
        assert!(!view.can_undo() && !view.can_redo());
        // Looking and selecting make no unit.
        view.command(Command::SelectAll, now);
        view.command(Command::SelectNone, now);
        view.command(Command::ZoomFit, now);
        assert!(!view.can_undo());
        assert_eq!(view.command(Command::Undo, now), Update::default());

        let mut boards = vec![view.board().clone()];
        let steps: [fn(&mut BoardView, Instant) -> Update; 6] = [
            |view, now| drag_node(view, "first", (60.0, 40.0), now),
            |view, now| view.command(Command::Duplicate, now),
            |view, now| view.command(Command::Shape(Shape::Diamond), now),
            |view, now| view.command(Command::Resize(Grow::Wider), now),
            |view, now| view.command(Command::Move(Dir::Left), now),
            |view, now| view.command(Command::Delete, now),
        ];
        for step in steps {
            assert_eq!(step(&mut view, now), CHANGED);
            boards.push(view.board().clone());
        }
        assert!(view.can_undo() && !view.can_redo());
        assert!(!view.pristine());

        // As many undos as gestures, each to the board before its gesture.
        let viewport = view.viewport();
        for before in boards.iter().rev().skip(1) {
            travel(&mut view, Command::Undo, now);
            assert_eq!(view.board(), before);
        }
        assert!(!view.can_undo() && view.can_redo());
        assert_eq!(view.command(Command::Undo, now), Update::default());
        // The floor is the opening text itself, not its canonical form.
        assert!(view.pristine());
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
        assert_eq!(view.viewport(), viewport, "undo leaves the view alone");

        // And forward again, to the same boards.
        for after in boards.iter().skip(1) {
            travel(&mut view, Command::Redo, now);
            assert_eq!(view.board(), after);
        }
        assert!(!view.can_redo());
        assert_eq!(view.command(Command::Redo, now), Update::default());
        // A new unit clears the redo side.
        travel(&mut view, Command::Undo, now);
        assert!(view.can_redo());
        click_node(&mut view, "first", now);
        assert_eq!(view.command(Command::Move(Dir::Up), now), CHANGED);
        assert!(!view.can_redo());
    }

    #[test]
    fn undo_puts_the_selection_and_the_cursor_back() {
        let mut view = sized(TWO);
        let now = Instant::now();
        click_node(&mut view, "a", now);
        view.command(Command::ExtendSelection(Dir::Right), now);
        assert_eq!(selected(&view), ["a", "b"]);
        assert_eq!(view.machine().cursor(), Some(&id("b")));
        assert_eq!(view.command(Command::Delete, now), CHANGED);
        assert!(selected(&view).is_empty());
        assert!(view.board().nodes.is_empty() && view.board().edges.is_empty());

        travel(&mut view, Command::Undo, now);
        assert_eq!(selected(&view), ["a", "b"]);
        assert_eq!(view.machine().cursor(), Some(&id("b")));
        assert!(view.pristine());
        assert_eq!(view.draft(), TWO);
        // Redo returns to how it was when undo was asked for.
        travel(&mut view, Command::Redo, now);
        assert!(selected(&view).is_empty());
        assert!(view.board().nodes.is_empty());
    }

    #[test]
    fn a_run_of_key_moves_is_one_step_and_a_pause_or_anything_else_ends_it() {
        let mut view = sized(TWO);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        click_node(&mut view, "a", start);
        for (step, dir) in [Dir::Right, Dir::Right, Dir::Down].into_iter().enumerate() {
            let now = at(step as u64 * 400);
            assert_eq!(view.command(Command::Move(dir), now), CHANGED);
        }
        assert_eq!(view.board().nodes[&id("a")].at, [40, 20]);
        // A second's pause begins another unit.
        view.command(Command::Move(Dir::Right), at(2_000));
        // So does a resize, which then runs on its own.
        view.command(Command::Resize(Grow::Wider), at(2_100));
        view.command(Command::Resize(Grow::Taller), at(2_200));
        // And a look elsewhere between two moves.
        view.command(Command::Move(Dir::Right), at(2_300));
        view.command(Command::SelectAll, at(2_350));
        click_node(&mut view, "a", at(2_360));
        view.command(Command::Move(Dir::Right), at(2_400));

        let places = [[80, 20], [60, 20], [60, 20], [40, 20], [0, 0]];
        let sizes = [[140, 80], [140, 80], [120, 60], [120, 60], [120, 60]];
        for (place, size) in places.into_iter().zip(sizes) {
            travel(&mut view, Command::Undo, at(3_000));
            let node = &view.board().nodes[&id("a")];
            assert_eq!((node.at, node.size), (place, size));
        }
        assert!(!view.can_undo());
        assert!(view.pristine());
    }

    #[test]
    fn a_creation_and_its_label_are_one_step_and_none_when_left_empty() {
        let now = Instant::now();
        let mut view = sized(TWO);
        // Made and left empty: the board is as it was, and nothing to undo.
        view.command(Command::Tool(Tool::Text), now);
        assert_eq!(click(&mut view, (200.0, 300.0), now), CHANGED);
        assert!(view.label().is_some());
        assert_eq!(key(&mut view, Key::Escape, now), CHANGED);
        assert!(!view.can_undo() && !view.can_redo());
        assert!(view.pristine());

        // Made, typed in and committed: one step.
        click_node(&mut view, "b", now);
        view.command(Command::Tool(Tool::Shape(Shape::Diamond)), now);
        click(&mut view, (200.0, 300.0), now);
        typed(&mut view, "Valid", now);
        typed(&mut view, "?", now);
        assert!(view.can_undo());
        key(&mut view, Key::Escape, now);
        assert_eq!(view.board().nodes.len(), 3);
        travel(&mut view, Command::Undo, now);
        assert!(!view.can_undo());
        assert!(view.pristine());
        assert_eq!(selected(&view), ["b"], "as it was before the creation");
        travel(&mut view, Command::Redo, now);
        let made = view
            .board()
            .nodes
            .values()
            .find(|node| node.text == "Valid?");
        assert!(made.is_some());

        // Tab builds the next shape with the label still open: the first
        // label's step ends there and the new shape begins its own.
        let mut view = sized(TWO);
        click_node(&mut view, "b", now);
        view.command(Command::ConnectNext, now);
        typed(&mut view, "C", now);
        view.command(Command::ConnectNext, now);
        typed(&mut view, "D", now);
        key(&mut view, Key::Escape, now);
        assert_eq!(view.board().nodes.len(), 4);
        travel(&mut view, Command::Undo, now);
        assert_eq!(view.board().nodes.len(), 3);
        travel(&mut view, Command::Undo, now);
        assert!(view.pristine() && !view.can_undo());
    }

    #[test]
    fn a_label_edit_is_one_step_and_none_when_it_is_typed_back() {
        let now = Instant::now();
        let mut view = sized(TWO);
        open_label(&mut view, "a", now);
        typed(&mut view, "Start", now);
        typed(&mut view, " here", now);
        // Undo commits the label first, then takes the whole edit back.
        travel(&mut view, Command::Undo, now);
        assert_eq!(view.label(), None);
        assert_eq!(view.board().nodes[&id("a")].text, "A");
        assert!(view.pristine() && !view.can_undo());
        travel(&mut view, Command::Redo, now);
        assert_eq!(view.board().nodes[&id("a")].text, "Start here");

        // The label's own undo never reaches the history.
        let mut view = sized(TWO);
        open_label(&mut view, "a", now);
        typed(&mut view, "Typed", now);
        assert_eq!(edit(&mut view, LabelAction::Undo, now), CHANGED);
        key(&mut view, Key::Escape, now);
        assert!(view.pristine());
        assert!(!view.can_undo() && !view.can_redo());

        // Opened and closed untouched, it makes nothing either.
        open_label(&mut view, "b", now);
        key(&mut view, Key::Escape, now);
        assert!(!view.can_undo());
    }

    #[test]
    fn a_save_keeps_the_history_and_a_reload_drops_it() {
        let now = Instant::now();
        let mut view = sized(fixtures::HAND_WRITTEN);
        click_node(&mut view, "first", now);
        view.command(Command::Move(Dir::Down), now);
        let saved = view.draft();
        view.saved(&saved);
        // After the save, undo still reaches the opening text.
        travel(&mut view, Command::Undo, now);
        assert_eq!(view.draft(), fixtures::HAND_WRITTEN);
        travel(&mut view, Command::Redo, now);
        assert_eq!(view.draft(), saved);
        assert!(view.can_undo());

        // The text changed underneath: nothing to undo or redo, and the
        // reloaded board is the new floor.
        travel(&mut view, Command::Undo, now);
        assert!(view.can_redo());
        view.reload(TWO).expect("a board");
        assert!(!view.can_undo() && !view.can_redo());
        assert_eq!(view.command(Command::Undo, now), Update::default());
        assert_eq!(view.command(Command::Redo, now), Update::default());
        assert_eq!(view.draft(), TWO);
        click_node(&mut view, "a", now);
        view.command(Command::Move(Dir::Down), now);
        travel(&mut view, Command::Undo, now);
        assert_eq!(view.draft(), TWO);
    }

    fn effects(effects: &[Effect]) -> Update {
        Update {
            changed: false,
            effects: effects.to_vec(),
        }
    }

    /// The board asks for the clipboard, and `text` comes back.
    fn pasted(view: &mut BoardView, text: &str, now: Instant) -> Update {
        view.command(Command::Paste, now);
        view.update(Message::Pasted(view.epoch(), text.to_owned()), now)
    }

    #[test]
    fn copy_writes_a_fragment_and_cut_deletes_it_as_one_step() {
        let now = Instant::now();
        let mut view = sized(TWO);
        // Nothing selected, or no node selected: nothing to copy.
        assert_eq!(view.command(Command::Copy, now), Update::default());
        view.machine
            .select(BTreeSet::from([id("e")]), Some(id("e")));
        assert_eq!(view.command(Command::Copy, now), Update::default());
        assert_eq!(view.command(Command::Cut, now), Update::default());

        view.command(Command::SelectAll, now);
        let copied = view.command(Command::Copy, now);
        let [Effect::ClipboardWrite(text)] = copied.effects.as_slice() else {
            panic!("{copied:?}");
        };
        assert!(!copied.changed);
        let (header, rest) = text.split_once('\n').expect("two lines");
        assert_eq!(header, "# wren-board fragment 1");
        let fragment = wren_board::parse(rest).expect("the fragment is a board");
        assert_eq!((fragment.nodes.len(), fragment.edges.len()), (2, 1));
        assert!(view.pristine() && !view.can_undo());

        // A cut puts the same text on the clipboard and deletes.
        let cut = view.command(Command::Cut, now);
        assert!(cut.changed);
        assert_eq!(cut.effects, [Effect::ClipboardWrite(text.clone())]);
        assert!(view.board().nodes.is_empty() && view.board().edges.is_empty());
        travel(&mut view, Command::Undo, now);
        assert!(view.pristine() && !view.can_undo());

        // A board that may not be edited still copies, and cuts nothing.
        let mut view = sized(fixtures::FUTURE);
        view.command(Command::SelectAll, now);
        let copied = view.command(Command::Copy, now);
        assert!(matches!(
            copied.effects.as_slice(),
            [Effect::ClipboardWrite(_)]
        ));
        assert_eq!(view.command(Command::Cut, now), Update::default());
        assert_eq!(view.command(Command::Paste, now), Update::default());
    }

    #[test]
    fn a_fragment_pastes_with_new_ids_a_cell_further_each_time() {
        let now = Instant::now();
        let mut view = sized(TWO);
        view.command(Command::SelectAll, now);
        let copied = view.command(Command::Copy, now);
        let [Effect::ClipboardWrite(text)] = copied.effects.as_slice() else {
            panic!("{copied:?}");
        };
        // The command asks for the clipboard; its text comes back.
        assert_eq!(
            view.command(Command::Paste, now),
            effects(&[Effect::ClipboardRead])
        );
        assert!(view.pristine());

        let places = |view: &BoardView| {
            let mut places: Vec<_> = view.board().nodes.values().map(|node| node.at).collect();
            places.sort_unstable();
            places
        };
        assert_eq!(pasted(&mut view, text, now), CHANGED);
        assert_eq!((view.board().nodes.len(), view.board().edges.len()), (4, 2));
        assert_eq!(places(&view), [[0, 0], [20, 20], [300, 0], [320, 20]]);
        // The copies are selected, the cursor on the first node of them,
        // and none has an id that was on the board.
        let fresh = selected(&view);
        assert_eq!(fresh.len(), 3);
        assert!(fresh.iter().all(|id| !["a", "b", "e"].contains(id)));
        let cursor = view.machine().cursor().expect("a cursor");
        assert!(fresh.contains(&cursor.as_str()) && view.board().nodes.contains_key(cursor));
        assert_eq!(view.label(), None, "a paste opens no label");

        assert_eq!(pasted(&mut view, text, now), CHANGED);
        assert_eq!((view.board().nodes.len(), view.board().edges.len()), (6, 3));
        assert_eq!(
            places(&view),
            [[0, 0], [20, 20], [40, 40], [300, 0], [320, 20], [340, 40]]
        );
        // Each connector joins two nodes that are there.
        assert!(wren_board::check(view.board()).is_empty());
        assert!(wren_board::parse(&view.draft()).is_ok());

        // Each paste is one step.
        travel(&mut view, Command::Undo, now);
        assert_eq!(view.board().nodes.len(), 4);
        travel(&mut view, Command::Undo, now);
        assert!(view.pristine() && !view.can_undo());

        // Out of view, the fragment lands in the middle of the view.
        view.set_viewport(Viewport {
            x: 5_000.0,
            y: 5_000.0,
            zoom: 1.0,
        });
        assert_eq!(pasted(&mut view, text, now), CHANGED);
        let left = places(&view)[2];
        assert_eq!(
            left,
            [5_300, 5_280],
            "centred in a 1000 by 600 view, on the grid"
        );
    }

    #[test]
    fn text_from_outside_pastes_as_one_text_node_in_the_middle_of_the_view() {
        let now = Instant::now();
        let mut view = sized(TWO);
        assert_eq!(pasted(&mut view, "Ship it\r\ntoday\n\n", now), CHANGED);
        assert_eq!(view.board().nodes.len(), 3);
        let made = view.machine().cursor().expect("a cursor").clone();
        assert_eq!(selected(&view), [made.as_str()]);
        let node = &view.board().nodes[&made];
        assert_eq!(node.text, "Ship it\ntoday");
        assert_eq!(node.kind, Some(Named::Known(NodeKind::Text)));
        // As wide as its longer line in whole cells, and two lines tall.
        let width = view
            .measurer()
            .width("Ship it", scene::Face::Regular, scene::LABEL_SIZE);
        assert_eq!(node.size[0], (width / 20.0).ceil() as i32 * 20);
        assert_eq!(node.size[1], 40);
        let middle = [node.at[0] + node.size[0] / 2, node.at[1] + node.size[1] / 2];
        assert!((middle[0] - 500).abs() <= 10 && (middle[1] - 300).abs() <= 10);
        assert_eq!(view.label(), None, "the board stays idle");
        travel(&mut view, Command::Undo, now);
        assert!(view.pristine());

        // Long text wraps at the paste width and grows downwards.
        let long = "word ".repeat(40);
        assert_eq!(pasted(&mut view, &long, now), CHANGED);
        let made = view.machine().cursor().expect("a cursor").clone();
        let node = &view.board().nodes[&made];
        assert_eq!(node.size[0], PASTE_WRAP);
        assert!(node.size[1] > 40, "{:?}", node.size);
        let lines = scene::wrap(
            &node.text,
            Some(PASTE_WRAP as f32),
            scene::Face::Regular,
            scene::LABEL_SIZE,
            view.measurer(),
        );
        assert!(lines.len() as f32 * scene::line_height(scene::LABEL_SIZE) <= node.size[1] as f32);
    }

    #[test]
    fn a_paste_that_cannot_be_made_says_why_and_changes_nothing() {
        let now = Instant::now();
        let mut view = sized(TWO);
        // Nothing, and white space alone, paste nothing and say nothing.
        assert_eq!(pasted(&mut view, "", now), Update::default());
        assert_eq!(pasted(&mut view, " \n\t\n", now), Update::default());
        let notices = |update: &Update| {
            assert!(!update.changed);
            assert!(
                matches!(update.effects.as_slice(), [Effect::Notice(_)]),
                "{update:?}"
            );
        };
        // A fragment that is not a board, one of a newer format, one that
        // needs what this build lacks, and text longer than a label.
        notices(&pasted(&mut view, "# wren-board fragment 1\n[node", now));
        notices(&pasted(
            &mut view,
            "# wren-board fragment 1\nschema = 2\n",
            now,
        ));
        let needs = format!("# wren-board fragment 1\n{}", fixtures::FUTURE);
        notices(&pasted(&mut view, &needs, now));
        notices(&pasted(
            &mut view,
            &"x".repeat(scene::MAX_LABEL_BYTES + 1),
            now,
        ));
        assert!(view.pristine() && !view.can_undo());

        // During a drag the board is the drag's.
        let (x, y) = middle(&view, "a");
        input(&mut view, press((x, y)), now);
        input(&mut view, Input::Move { at: (x + 80.0, y) }, now);
        assert_eq!(pasted(&mut view, "late", now), Update::default());
        input(
            &mut view,
            Input::Key {
                key: Key::Escape,
                repeat: false,
            },
            now,
        );
        input(&mut view, RELEASE, now);
        assert!(view.pristine());
    }

    /// A source that hands out the ids it is given, in order.
    struct Given(Vec<&'static str>);

    impl wren_board::ids::Ids for Given {
        fn draw(&mut self) -> Id {
            id(self.0.remove(0))
        }
    }

    #[test]
    fn applied_intents_are_one_step_with_the_given_ids_and_select_nothing() {
        let mut view = sized(TWO);
        let now = Instant::now();
        click_node(&mut view, "a", now);
        let intents = [
            Intent::AddNode {
                kind: NodeKind::Shape,
                shape: Shape::Diamond,
                at: [0, 200],
                size: [120, 60],
                from: None,
            },
            Intent::SetText {
                id: id("n9"),
                text: "Nine".to_owned(),
                size: [120, 60],
            },
            Intent::AddEdge {
                from: edit::Endpoint::Node {
                    node: id("a"),
                    side: None,
                },
                to: edit::Endpoint::Node {
                    node: id("n9"),
                    side: None,
                },
            },
            Intent::Delete { ids: vec![id("b")] },
        ];
        let update = view.apply(&intents, &mut Given(vec!["n9", "e9"]), now);
        assert_eq!(update, CHANGED);
        let board = view.board();
        assert_eq!(board.nodes[&id("n9")].text, "Nine");
        assert_eq!(board.nodes[&id("n9")].shape, Named::Known(Shape::Diamond));
        assert!(board.edges.contains_key(&id("e9")));
        // The connector from `a` to `b` went with `b`.
        assert!(!board.nodes.contains_key(&id("b")) && !board.edges.contains_key(&id("e")));
        // The selection is the user's: nothing new is in it, and no label
        // opened.
        assert_eq!(selected(&view), ["a"]);
        assert!(view.label().is_none());
        assert_eq!(view.machine().state(), machine::State::Idle);

        travel(&mut view, Command::Undo, now);
        assert!(view.pristine());
        assert_eq!(view.draft(), TWO);
        assert!(!view.can_undo());
    }

    #[test]
    fn applying_settles_first_and_a_refused_intent_changes_nothing() {
        let mut view = sized(TWO);
        let now = Instant::now();
        click_node(&mut view, "a", now);
        view.command(Command::EditLabel, now);
        view.update(
            Message::Label(view.epoch(), LabelAction::Insert("Typed".to_owned())),
            now,
        );
        let moved = [Intent::Move {
            ids: vec![id("b")],
            by: [20, 0],
        }];
        assert_eq!(view.apply(&moved, &mut Given(Vec::new()), now), CHANGED);
        // The label was committed as a step of its own.
        assert!(view.label().is_none());
        assert_eq!(view.board().nodes[&id("a")].text, "Typed");
        assert_eq!(view.board().nodes[&id("b")].at, [320, 0]);
        travel(&mut view, Command::Undo, now);
        assert_eq!(view.board().nodes[&id("a")].text, "Typed");
        assert_eq!(view.board().nodes[&id("b")].at, [300, 0]);

        // Nothing to change is no step.
        let still = [Intent::Move {
            ids: vec![id("b")],
            by: [0, 0],
        }];
        assert_eq!(
            view.apply(&still, &mut Given(Vec::new()), now),
            Update::default()
        );

        // A read-only board refuses every intent, and says why.
        let mut future = sized(fixtures::FUTURE);
        let update = future.apply(&moved, &mut Given(Vec::new()), now);
        assert!(!update.changed);
        assert!(
            matches!(update.effects.as_slice(), [Effect::Notice(_)]),
            "{update:?}"
        );
        assert!(future.pristine() && !future.can_undo());
    }

    #[test]
    fn a_key_is_what_the_widget_would_send_for_it() {
        use iced::keyboard::key::{Code, Named, Physical};
        use iced::keyboard::{Key as Pressed, Modifiers};
        let mut view = sized(TWO);
        let now = Instant::now();
        view.set_key_policy(std::sync::Arc::new(|key, _, modifiers, _| {
            match key.as_ref() {
                _ if modifiers.alt() => KeyRoute::Host,
                Pressed::Character("r") => {
                    KeyRoute::Command(Command::Tool(Tool::Shape(Shape::Rect)))
                }
                Pressed::Named(Named::Enter) => KeyRoute::Command(Command::EditLabel),
                _ => KeyRoute::Pass,
            }
        }));
        let physical = Physical::Code(Code::KeyR);
        let press = |view: &BoardView, key: Pressed, modifiers, text| {
            view.key(&key, physical, modifiers, text, false, false)
        };
        let r = || Pressed::Character("r".into());
        assert_eq!(press(&view, r(), Modifiers::ALT, None), None);
        assert_eq!(
            press(&view, r(), Modifiers::empty(), Some("r")),
            Some(vec![Message::Command(
                view.epoch(),
                Command::Tool(Tool::Shape(Shape::Rect))
            )])
        );
        // Escape has no rung to act on yet, so it is the host's.
        let escape = || Pressed::Named(Named::Escape);
        assert_eq!(press(&view, escape(), Modifiers::empty(), None), None);
        click_node(&mut view, "a", now);
        let epoch = view.epoch();
        assert_eq!(
            press(&view, escape(), Modifiers::empty(), None),
            Some(vec![Message::Input(
                epoch,
                Input::Key {
                    key: Key::Escape,
                    repeat: false
                }
            )])
        );
        // In a label the same letter is text.
        view.command(Command::EditLabel, now);
        assert_eq!(
            press(&view, r(), Modifiers::empty(), Some("r")),
            Some(vec![Message::Label(
                epoch,
                LabelAction::Insert("r".to_owned())
            )])
        );
    }

    /// Rest the pointer on the node `name` until it shows its connect
    /// handles, and say when that was.
    fn hover(view: &mut BoardView, name: &str, now: Instant) -> Instant {
        let at = middle(view, name);
        input(view, Input::Focus(true), now);
        input(view, Input::Move { at }, now);
        let rested = now + machine::HOVER_DELAY;
        input(view, Input::Tick, rested);
        rested
    }

    /// The centre of the connect handle outside `side` of the node `name`.
    fn connect_handle(view: &BoardView, name: &str, side: Side) -> (f32, f32) {
        let bounds = hit::Frames::new(view.board())
            .get(&id(name))
            .expect("a node");
        let handles = handles::connect(bounds, view.viewport());
        let found = handles.into_iter().find(|(of, _)| *of == side);
        found.expect("four handles").1
    }

    /// The lines `new` has that `old` has not, and those `old` has that
    /// `new` has not, each counted as often as it occurs.
    fn line_diff(old: &str, new: &str) -> (Vec<String>, Vec<String>) {
        let extra = |of: &str, than: &str| {
            let mut left: Vec<&str> = than.lines().collect();
            let mut extra = Vec::new();
            for line in of.lines() {
                match left.iter().position(|other| *other == line) {
                    Some(index) => {
                        left.swap_remove(index);
                    }
                    None => extra.push(line.to_owned()),
                }
            }
            extra
        };
        (extra(new, old), extra(old, new))
    }

    /// How many connect handles the overlay draws.
    fn connect_handles(view: &BoardView) -> usize {
        let marks = overlay::marks(view).marks;
        let connect = |mark: &&overlay::Mark| matches!(mark, overlay::Mark::Connect(_));
        marks.iter().filter(connect).count()
    }

    /// §5.5 rule 8: the handles go when the view moves under the pointer,
    /// by the keyboard's zoom or a fit as by the wheel.
    #[test]
    fn the_hover_handles_go_when_the_view_moves_by_any_path() {
        let moves: [fn(&mut BoardView); 5] = [
            |view| view.zoom_step(1),
            |view| view.zoom_step(-1),
            |view| view.fit(),
            |view| {
                view.zoom_step(1);
                view.zoom_reset();
            },
            |view| view.set_viewport(view.viewport().panned(30.0, 0.0)),
        ];
        for (n, moved) in moves.into_iter().enumerate() {
            let mut view = sized(TWO);
            view.set_viewport(Viewport::default());
            hover(&mut view, "a", Instant::now());
            assert_eq!(connect_handles(&view), 4, "{n}");
            moved(&mut view);
            assert_eq!(connect_handles(&view), 0, "{n}");
            assert!(!view.machine().hovering(), "{n}");
        }
        // A viewport set to where it is moves nothing.
        let mut view = sized(TWO);
        hover(&mut view, "a", Instant::now());
        view.set_viewport(view.viewport());
        assert_eq!(connect_handles(&view), 4);
    }

    #[test]
    fn a_connector_drawn_by_pointer_is_one_block_and_one_undo_step() {
        let mut view = sized(TWO);
        let now = hover(&mut view, "a", Instant::now());
        assert_eq!(wren_board::write(view.board()), TWO);
        let before = view.board().clone();
        let from = connect_handle(&view, "a", Side::Bottom);
        assert_eq!(input(&mut view, press(from), now), Update::default());
        // Over `b` the preview is the board as a release would leave it;
        // the model waits for the release.
        let over = Input::Move {
            at: middle(&view, "b"),
        };
        assert_eq!(input(&mut view, over, now), Update::default());
        assert_eq!(view.machine().state(), machine::State::Connecting);
        assert_eq!(*view.board(), before);
        assert_eq!(view.picture().board.edges.len(), 2);
        assert_ne!(view.shown(), view.scene());

        assert_eq!(input(&mut view, RELEASE, now), CHANGED);
        assert_eq!(view.shown(), view.scene());
        assert_eq!(view.machine().state(), machine::State::Idle);
        // A second connector between the pair (§5.12 rule 10), selected.
        assert_eq!(view.board().edges.len(), 2);
        let made = selected(&view)[0].to_owned();
        assert_ne!(made, "e");
        let edge = view.board().edges[&id(&made)].clone();
        let floats = |end: &wren_board::End, name: &str| matches!(end, wren_board::End::Node { node, side: None, anchor: None, .. } if node == name);
        assert!(floats(&edge.from, "a") && floats(&edge.to, "b"));
        // One new block in the file, and nothing else. Its id is drawn at
        // random, so where the block sorts is not known: the lines are
        // compared in order of their text.
        let (mut added, removed) = line_diff(TWO, &view.draft());
        let z = edge.z.expect("a stacking key");
        let mut block = [
            String::new(),
            format!("[edge.{made}]"),
            "kind = \"edge\"".to_owned(),
            "from = { node = \"a\" }".to_owned(),
            "to = { node = \"b\" }".to_owned(),
            format!("z = \"{z}\""),
        ];
        added.sort();
        block.sort();
        assert_eq!(added, block);
        assert!(removed.is_empty(), "{removed:?}");

        travel(&mut view, Command::Undo, now);
        assert!(view.pristine() && !view.can_undo());
        assert_eq!(view.draft(), TWO);
        assert!(selected(&view).is_empty());
    }

    #[test]
    fn a_connector_released_on_empty_canvas_makes_a_node_and_opens_its_label() {
        let mut view = sized(TWO);
        let now = hover(&mut view, "b", Instant::now());
        let from = connect_handle(&view, "b", Side::Bottom);
        input(&mut view, press(from), now);
        input(&mut view, Input::Move { at: (363.0, 247.0) }, now);
        // The preview is the node and its connector; the board has neither.
        let preview = view.picture();
        assert_eq!(
            (preview.board.nodes.len(), preview.board.edges.len()),
            (3, 2)
        );
        assert_eq!((view.board().nodes.len(), view.board().edges.len()), (2, 1));

        assert_eq!(input(&mut view, RELEASE, now), CHANGED);
        assert_eq!(view.machine().state(), machine::State::Editing);
        let made = selected(&view)[0].to_owned();
        let node = &view.board().nodes[&id(&made)];
        // A default rectangle centred on the pointer, on the grid.
        assert_eq!((node.at, node.size), ([300, 220], machine::NODE_SIZE));
        assert_eq!(node.shape, Named::Known(Shape::Rect));
        assert_eq!(label(&view).id().as_str(), made);
        let joins = view.board().edges.values().any(|edge| {
            matches!(&edge.from, wren_board::End::Node { node, side: None, .. } if node == "b")
                && matches!(&edge.to, wren_board::End::Node { node, side: None, .. } if *node == made)
        });
        assert!(joins);

        // The node, its connector and its label are one undo step.
        typed(&mut view, "Next", now);
        key(&mut view, Key::Escape, now);
        assert!(view.label().is_none());
        assert_eq!(view.board().nodes[&id(&made)].text, "Next");
        travel(&mut view, Command::Undo, now);
        assert!(view.pristine() && !view.can_undo());
    }

    #[test]
    fn a_release_makes_the_board_its_preview_showed() {
        // A new connector's route depends on where its id sorts, so the
        // preview draws the ids the release will.
        let ends: [(f32, f32); 2] = [(360.0, 30.0), (363.0, 247.0)];
        for to in ends {
            let mut view = sized(TWO);
            let now = hover(&mut view, "a", Instant::now());
            let from = connect_handle(&view, "a", Side::Bottom);
            input(&mut view, press(from), now);
            // Each step of the drag plans afresh, with the same ids.
            input(&mut view, Input::Move { at: (200.0, 200.0) }, now);
            input(&mut view, Input::Move { at: to }, now);
            let shown = view.picture().board.clone();
            assert_ne!(shown, *view.board());
            assert_eq!(input(&mut view, RELEASE, now), CHANGED);
            assert_eq!(*view.board(), shown);
        }
        // Copies dragged out of their originals keep their ids as well.
        let mut view = sized(TWO);
        let now = Instant::now();
        view.command(Command::SelectAll, now);
        let duplicate = Input::Modifiers(Mods {
            shift: false,
            duplicate: true,
        });
        input(&mut view, duplicate, now);
        let (x, y) = middle(&view, "a");
        input(&mut view, press((x, y)), now);
        input(&mut view, Input::Move { at: (x, y + 100.0) }, now);
        input(&mut view, Input::Move { at: (x, y + 200.0) }, now);
        let shown = view.picture().board.clone();
        assert_eq!(shown.edges.len(), 2);
        assert_eq!(input(&mut view, RELEASE, now), CHANGED);
        assert_eq!(*view.board(), shown);
    }

    #[test]
    fn a_connector_given_up_changes_nothing() {
        let now = Instant::now();
        let escape = Input::Key {
            key: Key::Escape,
            repeat: false,
        };
        let ends: [fn(&mut BoardView, Input, Instant) -> Update; 3] = [
            // Released on the node it came from.
            |view, _, now| {
                let at = middle(view, "a");
                input(view, Input::Move { at }, now);
                input(view, RELEASE, now)
            },
            |view, escape, now| input(view, escape, now),
            |view, _, now| view.settle(now),
        ];
        for (index, end) in ends.into_iter().enumerate() {
            let mut view = sized(TWO);
            let now = hover(&mut view, "a", now);
            let from = connect_handle(&view, "a", Side::Top);
            input(&mut view, press(from), now);
            input(&mut view, Input::Move { at: (500.0, 300.0) }, now);
            assert_ne!(view.shown(), view.scene(), "{index}");
            assert_eq!(end(&mut view, escape, now), Update::default(), "{index}");
            assert_eq!(view.shown(), view.scene(), "{index}");
            assert_eq!(input(&mut view, RELEASE, now), Update::default(), "{index}");
            assert_eq!(view.machine().state(), machine::State::Idle, "{index}");
            assert!(view.pristine() && !view.can_undo(), "{index}");
            assert_eq!(view.draft(), TWO, "{index}");
        }
    }

    #[test]
    fn a_reload_under_a_dragged_connector_drops_it() {
        // §3.4 rule 5: the board changed on disk while a connector was
        // being drawn, and while an end was being moved.
        let other = TWO.replace("at = [300, 0]", "at = [300, 100]");
        let drags: [fn(&mut BoardView, Instant); 2] = [
            |view, now| {
                let now = hover(view, "a", now);
                let from = connect_handle(view, "a", Side::Bottom);
                input(view, press(from), now);
                let over = Input::Move {
                    at: middle(view, "b"),
                };
                input(view, over, now);
            },
            |view, now| {
                click(view, (210.0, 30.0), now);
                input(view, press((300.0, 30.0)), now);
                input(view, Input::Move { at: (362.0, 3.0) }, now);
            },
        ];
        for (index, drag) in drags.into_iter().enumerate() {
            let mut view = sized(TWO);
            let now = Instant::now();
            drag(&mut view, now);
            assert_eq!(view.machine().state(), machine::State::Connecting);
            assert_ne!(view.shown(), view.scene(), "{index}");
            let stale = view.epoch();
            view.reload(&other).expect("a board");
            assert_eq!(view.machine().state(), machine::State::Idle, "{index}");
            assert_eq!(view.shown(), view.scene(), "{index}");
            // The release meant for the board before is dropped, and one
            // for this board finds no gesture.
            let late = view.update(Message::Input(stale, RELEASE), now);
            assert_eq!(late, Update::default(), "{index}");
            assert_eq!(input(&mut view, RELEASE, now), Update::default(), "{index}");
            assert!(view.pristine() && !view.can_undo(), "{index}");
            assert_eq!(view.draft(), other, "{index}");
        }
    }

    #[test]
    fn an_end_dragged_to_a_side_handle_is_pinned_and_dragged_off_it_floats() {
        let mut view = sized(TWO);
        let now = Instant::now();
        click(&mut view, (210.0, 30.0), now);
        assert_eq!(selected(&view), ["e"]);
        // The `to` end of `e` is the middle of `b`'s left side; the middle
        // of its top side is a side handle.
        input(&mut view, press((300.0, 30.0)), now);
        input(&mut view, Input::Move { at: (362.0, 3.0) }, now);
        assert_eq!(view.machine().state(), machine::State::Connecting);
        assert_ne!(view.shown(), view.scene());
        assert_eq!(input(&mut view, RELEASE, now), CHANGED);
        assert_eq!(selected(&view), ["e"]);
        let (added, removed) = line_diff(TWO, &view.draft());
        assert_eq!(added, ["to = { node = \"b\", side = \"top\" }"]);
        assert_eq!(removed, ["to = { node = \"b\" }"]);
        // The end is drawn there now, and so is its handle.
        let [_, to] = handles::ends(view.board(), view.scene(), &id("e")).expect("a route");
        assert_eq!((to.x, to.y), (360.0, 0.0));

        // Onto the node away from every handle it floats again, which is
        // the board as it was opened.
        input(&mut view, press((360.0, 0.0)), now);
        input(&mut view, Input::Move { at: (360.0, 30.0) }, now);
        assert_eq!(input(&mut view, RELEASE, now), CHANGED);
        assert!(view.pristine());
        assert_eq!(view.draft(), TWO);
        // Each drag was its own step.
        travel(&mut view, Command::Undo, now);
        travel(&mut view, Command::Undo, now);
        assert!(!view.can_undo());
    }

    #[test]
    fn an_attached_end_released_on_empty_canvas_stays_where_it_was() {
        let mut view = sized(TWO);
        let now = Instant::now();
        click(&mut view, (210.0, 30.0), now);
        input(&mut view, press((300.0, 30.0)), now);
        assert_eq!(
            input(&mut view, Input::Move { at: (500.0, 300.0) }, now),
            Update::default()
        );
        // Nothing is shown but the board: the UI makes no free end.
        assert_eq!(view.machine().state(), machine::State::Connecting);
        assert_eq!(view.shown(), view.scene());
        assert_eq!(input(&mut view, RELEASE, now), Update::default());
        assert!(view.pristine() && !view.can_undo());
        // Nor onto the node its other end is on.
        input(&mut view, press((300.0, 30.0)), now);
        input(&mut view, Input::Move { at: (60.0, 30.0) }, now);
        assert_eq!(view.shown(), view.scene());
        assert_eq!(input(&mut view, RELEASE, now), Update::default());
        assert!(view.pristine() && !view.can_undo());
        assert_eq!(selected(&view), ["e"]);
    }

    #[test]
    fn a_free_end_from_a_file_moves_on_the_grid_and_attaches() {
        let text = format!(
            "{TWO}\n[edge.loose]\nkind = \"edge\"\nfrom = {{ at = [0, 200] }}\nto = {{ at = [200, 200] }}\nz = \"d\"\n"
        );
        let mut view = sized(&text);
        let now = Instant::now();
        click(&mut view, (100.0, 200.0), now);
        assert_eq!(selected(&view), ["loose"]);
        input(&mut view, press((200.0, 200.0)), now);
        input(&mut view, Input::Move { at: (247.0, 329.0) }, now);
        assert_eq!(input(&mut view, RELEASE, now), CHANGED);
        let (added, removed) = line_diff(&text, &view.draft());
        assert_eq!(added, ["to = { at = [240, 320] }"]);
        assert_eq!(removed, ["to = { at = [200, 200] }"]);
        // Its other end, onto `a`.
        input(&mut view, press((0.0, 200.0)), now);
        let over = Input::Move {
            at: middle(&view, "a"),
        };
        input(&mut view, over, now);
        assert_eq!(input(&mut view, RELEASE, now), CHANGED);
        let (added, removed) = line_diff(&text, &view.draft());
        assert_eq!(
            added,
            ["from = { node = \"a\" }", "to = { at = [240, 320] }"]
        );
        assert_eq!(removed.len(), 2);
    }

    #[test]
    fn connectors_follow_a_dragged_node_and_a_pinned_end_keeps_its_side() {
        // §5.12 rule 7: `e` floats at both ends; `p` leaves `a` by its
        // right side whatever happens.
        let text = format!(
            "{TWO}\n[edge.p]\nkind = \"edge\"\nfrom = {{ node = \"a\", side = \"right\" }}\nto = {{ node = \"b\" }}\nz = \"d\"\n"
        );
        let mut view = sized(&text);
        let now = Instant::now();
        let leaves = |view: &BoardView, name: &str| {
            let picture = view.picture();
            let [from, _] =
                handles::ends(picture.board, picture.scene, &id(name)).expect("a route");
            (from.x, from.y)
        };
        assert_eq!(leaves(&view, "e"), (120.0, 30.0));
        assert_eq!(leaves(&view, "p"), (120.0, 30.0));
        // `b` dragged to below `a`.
        let (x, y) = middle(&view, "b");
        input(&mut view, press((x, y)), now);
        let below = Input::Move {
            at: (x - 300.0, y + 200.0),
        };
        input(&mut view, below, now);
        // During the drag, before anything is applied: the floating end
        // has changed side and the pinned one has not.
        assert_eq!(view.board().nodes[&id("b")].at, [300, 0]);
        assert_eq!(leaves(&view, "e"), (60.0, 60.0));
        assert_eq!(leaves(&view, "p"), (120.0, 30.0));
        assert_eq!(input(&mut view, RELEASE, now), CHANGED);
        assert_eq!(leaves(&view, "e"), (60.0, 60.0));
        assert_eq!(leaves(&view, "p"), (120.0, 30.0));
        // Moving a node changes no connector's line in the file.
        let (added, removed) = line_diff(&text, &view.draft());
        assert_eq!(added, ["at = [0, 200]"]);
        assert_eq!(removed, ["at = [300, 0]"]);
    }

    #[test]
    fn a_connector_s_label_is_edited_at_the_middle_of_its_route() {
        // §5.12 rule 8: a double-click opens the first label, which is new
        // here, and typing writes it in the middle of the route.
        let mut view = sized(TWO);
        let now = Instant::now();
        click(&mut view, (210.0, 30.0), now);
        let later = now + Duration::from_millis(100);
        click(&mut view, (210.0, 30.0), later);
        assert_eq!(view.machine().state(), machine::State::Editing);
        assert_eq!(label(&view).id(), &id("e"));
        typed(&mut view, "Yes", later);
        key(&mut view, Key::Escape, later);
        let (added, removed) = line_diff(TWO, &view.draft());
        assert_eq!(added, ["labels = [", "  { text = \"Yes\" },", "]"]);
        assert!(removed.is_empty());
        // Drawn half way along: `e` runs from 120,30 to 300,30.
        let centre = edge_label_point(view.scene(), &id("e")).expect("a label drawn");
        assert_eq!((centre.x, centre.y), (210.0, 30.0));
        // Emptied again, the label is gone and the board is as it opened.
        click(&mut view, (210.0, 30.0), later + Duration::from_secs(1));
        view.command(Command::EditLabel, later);
        edit(&mut view, LabelAction::Backspace, later);
        key(&mut view, Key::Escape, later);
        assert!(view.pristine());
    }

    #[test]
    fn a_connector_style_command_changes_its_lines_as_one_step() {
        let mut view = sized(TWO);
        let now = Instant::now();
        let styles = [
            Command::EdgeRouting,
            Command::EdgeStart,
            Command::EdgeEnd,
            Command::EdgeReverse,
            Command::StrokeDashed,
        ];
        // With no connector selected they do nothing.
        for command in styles {
            assert_eq!(view.command(command, now), Update::default());
        }
        click_node(&mut view, "a", now);
        for command in styles {
            assert_eq!(view.command(command, now), Update::default());
        }
        click(&mut view, (210.0, 30.0), now + Duration::from_secs(1));
        assert_eq!(selected(&view), ["e"]);
        let lines = |lines: &[&str]| lines.iter().map(|line| (*line).to_owned()).collect();
        let none: Vec<String> = Vec::new();
        for (command, added, removed) in [
            (
                Command::EdgeRouting,
                lines(&["route = \"straight\""]),
                none.clone(),
            ),
            (
                Command::EdgeStart,
                lines(&["heads = [\"arrow\", \"arrow\"]"]),
                none.clone(),
            ),
            (
                Command::EdgeEnd,
                lines(&["heads = [\"none\", \"none\"]"]),
                none.clone(),
            ),
            (
                Command::StrokeDashed,
                lines(&["style.stroke = \"dashed\""]),
                none.clone(),
            ),
            (
                Command::EdgeReverse,
                lines(&["from = { node = \"b\" }", "to = { node = \"a\" }"]),
                lines(&["from = { node = \"a\" }", "to = { node = \"b\" }"]),
            ),
        ] {
            assert_eq!(view.command(command, now), CHANGED, "{command:?}");
            assert_eq!(
                line_diff(TWO, &view.draft()),
                (added, removed),
                "{command:?}"
            );
            // Each switches back, and each is one undo step.
            assert_eq!(view.command(command, now), CHANGED, "{command:?}");
            assert_eq!(view.draft(), TWO, "{command:?}");
            travel(&mut view, Command::Undo, now);
            travel(&mut view, Command::Undo, now);
            assert!(view.pristine() && !view.can_undo(), "{command:?}");
            assert_eq!(selected(&view), ["e"]);
        }
        // A board that cannot be edited takes none of them.
        let mut view = sized("schema = 2\n");
        view.command(Command::SelectAll, now);
        for command in styles {
            assert_eq!(view.command(command, now), Update::default());
        }
    }

    #[test]
    fn a_style_command_over_mixed_connectors_gives_them_all_the_first_value() {
        let text = format!(
            "{TWO}\n[edge.f]\nkind = \"edge\"\nfrom = {{ node = \"b\" }}\nto = {{ node = \"a\" }}\n\
             heads = [\"arrow\", \"none\"]\nroute = \"curved\"\nz = \"d\"\n"
        );
        let mut view = sized(&text);
        let now = Instant::now();
        view.command(Command::SelectAll, now);
        let edge = |view: &BoardView, name: &str| view.board().edges[&id(name)].clone();
        // `f`'s unknown route is drawn straight, `e`'s is elbow: both go
        // straight, then both elbow.
        view.command(Command::EdgeRouting, now);
        for name in ["e", "f"] {
            assert_eq!(
                edge(&view, name).route,
                Named::Known(wren_board::Route::Straight)
            );
        }
        view.command(Command::EdgeRouting, now);
        for name in ["e", "f"] {
            assert_eq!(
                edge(&view, name).route,
                Named::Known(wren_board::Route::Elbow)
            );
        }
        // `e` has an arrow at its end and `f` none: both get one.
        view.command(Command::EdgeEnd, now);
        let arrow = Named::Known(wren_board::Head::Arrow);
        assert_eq!(edge(&view, "e").heads[1], arrow);
        assert_eq!(edge(&view, "f").heads[1], arrow);
        // Reversed, each swaps its own ends.
        view.command(Command::EdgeReverse, now);
        let ends = |view: &BoardView, name: &str| {
            let edge = edge(view, name);
            let node = |end: &wren_board::End| match end {
                wren_board::End::Node { node, .. } => node.clone(),
                wren_board::End::Free { .. } => String::new(),
            };
            (node(&edge.from), node(&edge.to))
        };
        assert_eq!(ends(&view, "e"), ("b".to_owned(), "a".to_owned()));
        assert_eq!(ends(&view, "f"), ("a".to_owned(), "b".to_owned()));
    }

    #[test]
    fn after_a_delete_the_cursor_goes_to_the_nearest_node_left() {
        // `c` is 200 below `a` and `b` 300 to its right; `d` and `e` are
        // as far from `f` as each other.
        let node = |name: &str, x: i32, y: i32| {
            format!(
                "\n[node.{name}]\nkind = \"shape\"\nat = [{x}, {y}]\nsize = [120, 60]\nz = \"{name}\"\n"
            )
        };
        let text = [
            "schema = 1\n".to_owned(),
            node("a", 0, 0),
            node("b", 300, 0),
            node("c", 0, 200),
            node("d", 1000, 0),
            node("e", 1400, 0),
            node("f", 1200, 0),
        ]
        .concat();
        let mut view = sized(&text);
        let now = Instant::now();
        view.machine
            .select(BTreeSet::from([id("a")]), Some(id("a")));
        assert_eq!(view.command(Command::Delete, now), CHANGED);
        assert!(selected(&view).is_empty());
        assert_eq!(view.machine().cursor(), Some(&id("c")));
        // Undo puts the selection and the cursor back.
        travel(&mut view, Command::Undo, now);
        assert_eq!(selected(&view), ["a"]);
        assert_eq!(view.machine().cursor(), Some(&id("a")));
        // A tie goes to the lower id.
        view.machine
            .select(BTreeSet::from([id("f")]), Some(id("f")));
        view.command(Command::Delete, now);
        assert_eq!(view.machine().cursor(), Some(&id("d")));
        // Cut moves it the same way.
        view.machine
            .select(BTreeSet::from([id("d")]), Some(id("d")));
        view.command(Command::Cut, now);
        assert_eq!(view.machine().cursor(), Some(&id("e")));
        // A cursor the delete leaves stays where it is.
        view.machine
            .select(BTreeSet::from([id("b"), id("c")]), Some(id("a")));
        view.command(Command::Delete, now);
        assert_eq!(view.machine().cursor(), Some(&id("a")));
        // With no node left there is no cursor.
        view.command(Command::SelectAll, now);
        view.command(Command::Delete, now);
        assert_eq!(view.machine().cursor(), None);
    }

    #[test]
    fn a_deleted_connector_leaves_the_cursor_on_the_node_nearest_its_middle() {
        let mut view = sized(TWO);
        let now = Instant::now();
        // `e` runs from 120,30 to 300,30, so its middle is as far from
        // either end's node, and the tie goes to `a`.
        view.machine
            .select(BTreeSet::from([id("e")]), Some(id("e")));
        view.command(Command::Delete, now);
        assert_eq!(view.machine().cursor(), Some(&id("a")));
    }

    #[test]
    fn tab_with_nothing_selected_makes_a_node_in_the_middle_of_the_view() {
        let mut view = sized(wren_board::EMPTY);
        let now = Instant::now();
        let middle = view.viewport().to_board((SIZE.0 / 2.0, SIZE.1 / 2.0));
        let first = wren_board::place::start(view.board(), [middle.0, middle.1], [120, 60]);
        assert_eq!(view.command(Command::ConnectNext, now), CHANGED);
        assert_eq!(view.machine().state(), machine::State::Editing);
        let made = |view: &BoardView| {
            let mut nodes: Vec<[i32; 2]> =
                view.board().nodes.values().map(|node| node.at).collect();
            nodes.sort_unstable();
            nodes
        };
        assert_eq!(made(&view).first().copied(), first);
        assert!(view.board().edges.is_empty());
        typed(&mut view, "Start", now);
        // The next one, with nothing selected, steps right of it by its
        // width and a gap.
        key(&mut view, Key::Escape, now);
        view.command(Command::SelectNone, now);
        view.command(Command::ConnectNext, now);
        let [at, next] = made(&view)[..] else {
            panic!("{:?}", made(&view))
        };
        assert_eq!(next, [at[0] + 200, at[1]]);
        assert!(view.board().edges.is_empty());
        // Several selected: nothing.
        key(&mut view, Key::Escape, now);
        view.command(Command::SelectAll, now);
        assert_eq!(view.command(Command::ConnectNext, now), Update::default());
    }

    #[test]
    fn the_connect_query_is_typed_with_the_tool_letters_and_enter_connects() {
        use iced::keyboard::key::{Code, Named as Pressed, Physical};
        use iced::keyboard::{Key as Typed, Modifiers};
        let mut view = sized(TWO);
        let now = Instant::now();
        // `a` and `d` are tool keys, Enter edits a label and the arrows walk.
        view.set_key_policy(std::sync::Arc::new(|key, _, _, _| match key.as_ref() {
            Typed::Character("a") => KeyRoute::Command(Command::Tool(Tool::Connector)),
            Typed::Character("d") => KeyRoute::Command(Command::Tool(Tool::Shape(Shape::Diamond))),
            Typed::Named(Pressed::Enter) => KeyRoute::Command(Command::EditLabel),
            Typed::Named(Pressed::ArrowLeft) => KeyRoute::Command(Command::Select(Dir::Left)),
            _ => KeyRoute::Pass,
        }));
        let press = |view: &mut BoardView, key: Typed, text: Option<&str>| {
            let physical = Physical::Code(Code::KeyA);
            let sent = view.key(&key, physical, Modifiers::empty(), text, false, false);
            let mut update = Update::default();
            for message in sent.expect("the board's key") {
                let one = view.update(message, now);
                update.changed |= one.changed;
            }
            update
        };
        click_node(&mut view, "b", now);
        press(&mut view, Typed::Character("a".into()), Some("a"));
        assert_eq!(view.machine().state(), machine::State::Connecting);
        // The same letter is now the query's, and so is `d`.
        press(&mut view, Typed::Character("d".into()), Some("d"));
        press(&mut view, Typed::Named(Pressed::Backspace), None);
        press(&mut view, Typed::Character("a".into()), Some("a"));
        let seeking = view.machine().seeking().expect("connecting");
        assert_eq!((seeking.target, seeking.query), (Some(&id("a")), "a"));
        // An arrow walks: left of `b` is `a` too.
        press(&mut view, Typed::Named(Pressed::ArrowLeft), None);
        let seeking = view.machine().seeking().expect("connecting");
        assert_eq!((seeking.target, seeking.query), (Some(&id("a")), ""));
        assert_eq!(view.machine().tool(), Tool::Select);
        let update = press(&mut view, Typed::Named(Pressed::Enter), None);
        assert!(update.changed);
        assert_eq!(view.machine().state(), machine::State::Idle);
        let [made] = selected(&view)[..] else {
            panic!("{:?}", selected(&view))
        };
        let edge = &view.board().edges[&id(made)];
        let names = |end: &wren_board::End| match end {
            wren_board::End::Node {
                node,
                side: None,
                anchor: None,
                ..
            } => node.clone(),
            _ => String::new(),
        };
        assert_eq!(
            (names(&edge.from), names(&edge.to)),
            ("b".to_owned(), "a".to_owned())
        );
        // One block and one undo step.
        let (added, removed) = line_diff(TWO, &view.draft());
        assert_eq!(added.len(), 6, "{added:?}");
        assert!(removed.is_empty());
        travel(&mut view, Command::Undo, now);
        assert!(view.pristine() && !view.can_undo());
    }
}
