//! What input does to a board, as a pure function of the input and what
//! the view shows. No iced type appears here: the widget turns iced's
//! events into [`Input`]s, the host's command path reaches
//! [`Machine::command`], and the view does what the returned [`Act`]s ask,
//! so every rule below runs in a unit test with no window.
//!
//! The machine holds what is the surface's and not the board's (the
//! plan's §5.2 to §5.5, §5.8, §5.12 and §5.14): whether the board has the
//! keyboard, the armed tool, the selection and the cursor, which label is
//! open, the node the pointer rests on, and the pointer gesture in flight.
//! It never changes the board. A
//! gesture shows itself as a preview and asks for one edit when it is
//! released (§3.4 rule 4); the edit's plan comes back through
//! [`Machine::applied`], which is how a creation selects what it made and
//! opens its label. The open label's text is the view's; the machine says
//! when it opens and closes, and sends it the pointer ([`Act::Caret`]).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use wren_board::edit::{Area, EdgeEnd, Edges, Endpoint, Intent, Plan};
use wren_board::scene::{Rect, Scene};
use wren_board::{snap, Board, Dir, End, Id, NodeKind, Shape, Side, GRID};

use crate::command::{Command, Grow, Tool};
use crate::handles::{self, Handle};
use crate::hit::{self, Frames, Hit, Shown};
use crate::Viewport;

/// Logical pixels a wheel line pans by.
pub const LINE: f32 = 40.0;
/// The zoom factor of one wheel line.
pub const ZOOM_PER_LINE: f32 = 1.1;
/// Wheel events closer together than this are one burst, which keeps the
/// pan-or-zoom choice of its first event: iced drops the scroll phase, and
/// macOS momentum outlives the modifier.
pub const BURST: Duration = Duration::from_millis(150);
/// Screen pixels a pressed pointer travels before the press is a drag.
pub const THRESHOLD: f32 = 4.0;
/// A press this soon after the last one, and within [`DOUBLE_REACH`] of
/// it, is a double-click: iced's own rule.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(300);
/// Screen pixels within which a second press is a double-click.
pub const DOUBLE_REACH: f32 = 6.0;
/// The size of a new shape (§5.5 rule 2).
pub const NODE_SIZE: [i32; 2] = [120, 60];
/// The size of a new text node.
pub const TEXT_SIZE: [i32; 2] = [120, 20];
/// A pause this long ends a run of key moves or key resizes, which is one
/// undo step (§5.11 rule 3).
pub const RUN_PAUSE: Duration = Duration::from_secs(1);
/// How long the pointer rests on a node before the node shows its connect
/// handles (§5.5 rule 8).
pub const HOVER_DELAY: Duration = Duration::from_millis(120);

/// A wheel delta as the toolkit reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    /// Wheel lines; X11 reports touchpads this way too.
    Lines { x: f32, y: f32 },
    /// Physical pixels, from a precise device.
    Pixels { x: f32, y: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    Other,
}

/// The modifiers the machine reads, as the widget resolves them for the
/// platform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    /// Shift: a click toggles, a marquee adds.
    pub shift: bool,
    /// ⌥ on macOS, Ctrl elsewhere: a drag past the threshold moves copies
    /// (§5.8 rule 3).
    pub duplicate: bool,
}

/// A key the board's state reads itself rather than as a command: Escape's
/// ladder (§5.14), and Enter, the arrows and Backspace in keyboard
/// Connecting (§5.1 rule 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Escape,
    Enter,
    Backspace,
    Arrow(Dir),
}

/// One input to the machine. Points are widget-local logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Wheel {
        delta: Scroll,
        at: (f32, f32),
        /// The zoom modifier (⌘ on macOS, Ctrl elsewhere) was held.
        zoom: bool,
        /// Shift was held.
        sideways: bool,
        /// The window's scale factor, which turns `Pixels` into logical
        /// pixels.
        scale_factor: f32,
    },
    Press {
        button: Button,
        at: (f32, f32),
    },
    /// The pointer moved: with a button down wherever it went, and with
    /// none only over the board, where it may come to rest on a node.
    Move {
        at: (f32, f32),
    },
    Release {
        button: Button,
    },
    /// The pointer is over the board no longer, with no button down.
    Leave,
    /// Space went down or came up.
    Space {
        held: bool,
    },
    /// The pointer's gesture cannot go on: the window lost its focus, or
    /// the pointer left with no button held. A gesture in flight is
    /// cancelled, with nothing changed, and Space counts as up.
    Cancel,
    /// The modifiers changed.
    Modifiers(Mods),
    /// A key from [`Key`]; `repeat` when it is held down.
    Key {
        key: Key,
        repeat: bool,
    },
    /// A typed character outside a label: the connect query's text.
    Char(char),
    /// The widget took (`true`) or lost iced focus.
    Focus(bool),
    /// Time passed with no other input, so the deadline the machine set
    /// ([`Machine::deadline`]) may be due.
    Tick,
}

/// What the machine reads of the view. The scene is the board's own: a
/// gesture's preview is never hit-tested.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    pub viewport: Viewport,
    pub now: Instant,
    pub board: &'a Board,
    pub scene: &'a Scene,
    /// Whether the board may be edited. A read-only one still selects,
    /// pans and zooms (§5.2).
    pub writable: bool,
}

/// What the machine asks of the view that owns the board, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Look at the board from here.
    Look(Viewport),
    /// Plan this and apply it, as one undo step and one draft (§3.4 rule
    /// 4). A preview on show is over. The view hands the plan back to
    /// [`Machine::applied`].
    Edit(Intent),
    /// As [`Act::Edit`], but in the undo step of the edit before it: one
    /// more of a run of key moves or key resizes (§5.11 rule 3).
    Continue(Intent),
    /// Show the board as this would leave it, on a scratch copy; `None`
    /// shows the board itself again.
    Preview(Option<Intent>),
    /// Open the label editor on this element (§5.9 rule 1).
    OpenLabel(Id),
    /// Commit the open label and close its editor (§5.9 rule 4). When that
    /// removes the element, the view says so with [`Machine::retain`].
    CommitLabel,
    /// The pointer went down in the open label, or is dragged with the
    /// button held: put its caret at this board point, or with `select`
    /// take its selection there.
    Caret { at: (f32, f32), select: bool },
    /// A command the machine has no rule of its own for. The state allows
    /// it, any open label is committed, and the view runs it on the
    /// selection.
    Run(Command),
}

/// The surface state of §5.2, as the harness names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    /// Another widget has the keyboard.
    Unfocused,
    Idle,
    /// A tool other than Select is armed.
    Armed,
    /// A button is down and the pointer has not yet moved [`THRESHOLD`].
    Pressed,
    Moving,
    Marquee,
    Resizing,
    Creating,
    /// A connector is being drawn, or one of its ends moved (§5.12 rules 1
    /// and 6).
    Connecting,
    /// A label is open.
    Editing,
    Panning,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            Self::Unfocused => "unfocused",
            Self::Idle => "idle",
            Self::Armed => "armed",
            Self::Pressed => "pressed",
            Self::Moving => "moving",
            Self::Marquee => "marquee",
            Self::Resizing => "resizing",
            Self::Creating => "creating",
            Self::Connecting => "connecting",
            Self::Editing => "editing",
            Self::Panning => "panning",
        }
    }
}

/// What a press landed on.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Node(Id),
    Edge(Id),
    Resize(Id, Handle),
    /// A node a connector may be drawn from: by its connect handle on this
    /// side, or with the connector tool anywhere on it.
    Connect(Id, Option<Side>),
    /// An endpoint handle of the selected connector.
    End(Id, EdgeEnd),
    Canvas,
    /// Anywhere, with a shape or text tool armed.
    Create,
}

/// What a Connecting gesture drags.
#[derive(Clone, Debug, PartialEq)]
enum Link {
    /// The `to` end of a new connector from this node.
    New(Id),
    /// One end of this connector.
    End(Id, EdgeEnd),
}

/// The node the pointer rests on with no button down (§5.5 rule 8).
#[derive(Clone, Debug, PartialEq)]
struct Hover {
    id: Id,
    since: Instant,
    /// Whether it has rested [`HOVER_DELAY`], so its connect handles show.
    shown: bool,
}

/// A connector end being dragged, as the view draws it (§5.12 rule 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Connecting<'a> {
    /// The node a release would attach the end to, and the side it would
    /// pin it to.
    pub target: Option<(&'a Id, Option<Side>)>,
    /// The end of a connector that a release here would leave where it
    /// was.
    pub loose: Option<(&'a Id, EdgeEnd)>,
    /// The board point under the pointer.
    pub at: (f32, f32),
}

/// The pointer gesture in flight. `anchor` is the board point that was
/// under the pointer when it went down, so a wheel pan mid-gesture carries
/// the gesture with it.
#[derive(Clone, Debug, Default, PartialEq)]
enum Gesture {
    #[default]
    None,
    Pressed {
        at: (f32, f32),
        anchor: (f32, f32),
        on: Target,
        double: bool,
    },
    Moving {
        anchor: (f32, f32),
        /// The pressed node's corner, which lands on the grid; a pressed
        /// connector has none, and the move itself is whole cells.
        origin: Option<[i64; 2]>,
        ids: Vec<Id>,
        /// The originals stay and copies move.
        copy: bool,
        by: [i32; 2],
    },
    Resizing {
        anchor: (f32, f32),
        id: Id,
        handle: Handle,
        /// The node's left, top, right and bottom at the press.
        frame: [i64; 4],
        edges: Edges,
    },
    Marquee {
        anchor: (f32, f32),
        to: (f32, f32),
        /// What the marquee adds to.
        base: BTreeSet<Id>,
        /// The selection to put back if the marquee is cancelled.
        before: BTreeSet<Id>,
    },
    Creating {
        anchor: (f32, f32),
        area: Option<Area>,
    },
    Connecting {
        link: Link,
        /// The board point under the pointer.
        to: (f32, f32),
        /// The node the end is over, and the side it would be pinned to.
        target: Option<(Id, Option<Side>)>,
        /// What a release would do, which is also what is shown.
        plan: Option<Intent>,
    },
    /// A drag moves the view; `from` is where the pointer was last seen.
    Panning { button: Button, from: (f32, f32) },
    /// A drag in the open label selects its text.
    Selecting,
}

/// A run of key moves or of key resizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    Move,
    Resize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Machine {
    focused: bool,
    tool: Tool,
    selection: BTreeSet<Id>,
    cursor: Option<Id>,
    /// The element whose label is open.
    editing: Option<Id>,
    gesture: Gesture,
    space: bool,
    mods: Mods,
    /// When the last wheel event came, and whether its burst zooms.
    burst: Option<(Instant, bool)>,
    /// When and where the last press that may begin a double-click came.
    click: Option<(Instant, (f32, f32))>,
    /// The run the last command continued, and when.
    run: Option<(Run, Instant)>,
    hover: Option<Hover>,
}

impl Machine {
    pub fn step(&mut self, input: Input, view: &View<'_>) -> Vec<Act> {
        let mut acts = Vec::new();
        // Pointer input of any kind ends a run of key moves (§5.11 rule 3).
        if matches!(
            input,
            Input::Wheel { .. } | Input::Press { .. } | Input::Move { .. } | Input::Release { .. }
        ) {
            self.run = None;
        }
        match input {
            Input::Wheel {
                delta,
                at,
                zoom,
                sideways,
                scale_factor,
            } => {
                // The board moves under the pointer.
                self.hover = None;
                let to = self.wheel(delta, at, zoom, sideways, scale_factor, view);
                acts.extend(to.filter(|to| *to != view.viewport).map(Act::Look));
            }
            // A pointer that is nowhere would snap to the origin.
            Input::Press { at, .. } | Input::Move { at }
                if !(at.0.is_finite() && at.1.is_finite()) => {}
            Input::Press { button, at } => self.press(button, at, view, &mut acts),
            Input::Move { at } => self.moved(at, view, &mut acts),
            Input::Release { button } => self.release(button, view, &mut acts),
            // In a label Space is text.
            Input::Space { held } => self.space = held && self.editing.is_none(),
            Input::Cancel => {
                self.cancel(&mut acts);
                self.space = false;
                self.hover = None;
            }
            Input::Leave => self.hover = None,
            Input::Tick => self.rest(view.now),
            Input::Modifiers(mods) => self.mods = mods,
            Input::Key {
                key: Key::Escape,
                repeat: false,
            } => self.escape(&mut acts),
            Input::Focus(true) => self.focused = true,
            // Whatever takes the keyboard (a palette, a dialog, another
            // column) finds the board settled and its tool put away (§5.15
            // rule 4).
            Input::Focus(false) => {
                acts = self.settle();
                self.tool = Tool::Select;
                self.space = false;
                self.focused = false;
                self.hover = None;
            }
            Input::Key { .. } | Input::Char(_) => {}
        }
        acts
    }

    /// Run a board command. A pointer gesture in flight keeps the board to
    /// itself, so a command then does nothing; an open label is committed
    /// first (§5.1 rule 5). A command with no rule here comes back as
    /// [`Act::Run`].
    pub fn command(&mut self, command: Command, view: &View<'_>) -> Vec<Act> {
        let mut acts = Vec::new();
        if self.gesture != Gesture::None {
            return acts;
        }
        let run = self.run.take();
        // A command may change the board or the view under the pointer.
        self.hover = None;
        self.commit(&mut acts);
        match command {
            Command::CommitLabel => {}
            Command::Tool(tool) => {
                if view.writable || matches!(tool, Tool::Select | Tool::Hand) {
                    self.tool = tool;
                }
            }
            Command::EditLabel => {
                if let Some(id) = self.only().filter(|_| view.writable) {
                    self.open(id, &mut acts);
                }
            }
            Command::Shape(shape) => {
                let ids = self.nodes(view.board);
                if view.writable && !ids.is_empty() {
                    acts.push(Act::Edit(Intent::SetShape { ids, shape }));
                }
            }
            Command::ConnectNext => self.extend(None, view, &mut acts),
            Command::Connect(dir) => self.extend(Some(dir), view, &mut acts),
            Command::SelectAll => {
                let (nodes, edges) = (view.board.nodes.keys(), view.board.edges.keys());
                self.selection = nodes.chain(edges).cloned().collect();
            }
            Command::SelectNone => self.selection.clear(),
            Command::Move(dir) => {
                if view.writable && !self.selection.is_empty() {
                    let intent = Intent::Move {
                        ids: self.selection.iter().cloned().collect(),
                        by: self.cell(dir, view.board),
                    };
                    self.nudge(Run::Move, run, intent, view.now, &mut acts);
                }
            }
            Command::Resize(grow) => {
                let ids = self.nodes(view.board);
                if view.writable && !ids.is_empty() {
                    let mut edges = Edges::default();
                    match grow {
                        Grow::Wider => edges.right = GRID,
                        Grow::Narrower => edges.right = -GRID,
                        Grow::Taller => edges.bottom = GRID,
                        Grow::Shorter => edges.bottom = -GRID,
                    }
                    let intent = Intent::Resize { ids, edges };
                    self.nudge(Run::Resize, run, intent, view.now, &mut acts);
                }
            }
            other => acts.push(Act::Run(other)),
        }
        acts
    }

    /// The view applied the plan of an [`Act::Edit`] or [`Act::Continue`],
    /// and `board` is the board after it. What the plan selects becomes
    /// the selection, with the cursor on the first node of it; what the
    /// edit removed leaves it; and a node the plan made to be labelled is
    /// selected alone and its label opened (§5.5 rule 6).
    pub fn applied(&mut self, plan: &Plan, board: &Board) -> Vec<Act> {
        let mut acts = Vec::new();
        if !plan.is_empty() {
            self.hover = None;
        }
        if let Some(select) = &plan.select {
            self.run = None;
            self.selection = select.iter().cloned().collect();
            let node = select.iter().find(|id| board.nodes.contains_key(*id));
            let cursor = node.or_else(|| select.iter().find(|id| exists(board, id)));
            if let Some(cursor) = cursor {
                self.cursor = Some(cursor.clone());
            }
        }
        self.retain(board);
        let made = plan.edit.as_ref().filter(|id| exists(board, id));
        if let Some(id) = made {
            self.run = None;
            self.selection = BTreeSet::from([id.clone()]);
            self.cursor = Some(id.clone());
            self.open(id.clone(), &mut acts);
        }
        acts
    }

    /// Before the host does anything that is not passive (§3.4 rule 3):
    /// the open label is committed and a pointer gesture cancelled, with
    /// the board as it was at the press.
    pub fn settle(&mut self) -> Vec<Act> {
        let mut acts = Vec::new();
        self.run = None;
        self.cancel(&mut acts);
        self.commit(&mut acts);
        acts
    }

    /// The board was replaced underneath (§3.4 rule 5): back to Idle with
    /// Space up, the gesture dropped and the label closed without a
    /// commit. The selection stays for [`Machine::retain`], and the burst
    /// is the wheel's and stays.
    pub fn reset(&mut self) {
        self.gesture = Gesture::None;
        self.editing = None;
        self.tool = Tool::Select;
        self.space = false;
        self.click = None;
        self.run = None;
        self.hover = None;
    }

    /// The view has no label to open on the element it was asked to
    /// ([`Act::OpenLabel`]): one the scene does not draw.
    pub fn abandon(&mut self) {
        self.editing = None;
    }

    /// Forget the elements `board` no longer has: in the selection, as the
    /// cursor, and as the open label.
    pub fn retain(&mut self, board: &Board) {
        self.selection.retain(|id| exists(board, id));
        self.cursor = self.cursor.take().filter(|id| exists(board, id));
        self.editing = self.editing.take().filter(|id| exists(board, id));
        self.hover = self.hover.take().filter(|hover| exists(board, &hover.id));
    }

    /// Put a selection and a cursor back, as undo and redo do (§5.11 rule
    /// 1).
    pub fn select(&mut self, selection: BTreeSet<Id>, cursor: Option<Id>) {
        self.run = None;
        self.hover = None;
        self.selection = selection;
        self.cursor = cursor;
    }

    pub fn state(&self) -> State {
        match self.gesture {
            Gesture::Pressed { .. } => State::Pressed,
            Gesture::Moving { .. } => State::Moving,
            Gesture::Resizing { .. } => State::Resizing,
            Gesture::Marquee { .. } => State::Marquee,
            Gesture::Creating { .. } => State::Creating,
            Gesture::Connecting { .. } => State::Connecting,
            Gesture::Panning { .. } => State::Panning,
            Gesture::Selecting => State::Editing,
            Gesture::None if self.editing.is_some() => State::Editing,
            Gesture::None if !self.focused => State::Unfocused,
            Gesture::None if self.tool != Tool::Select => State::Armed,
            Gesture::None => State::Idle,
        }
    }

    /// The armed tool; Select when none is.
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// The selected elements (§5.3 rule 1).
    pub fn selection(&self) -> &BTreeSet<Id> {
        &self.selection
    }

    /// The element last selected, which outlives the selection (§5.3
    /// rule 2).
    pub fn cursor(&self) -> Option<&Id> {
        self.cursor.as_ref()
    }

    /// The element whose label is open.
    pub fn editing(&self) -> Option<&Id> {
        self.editing.as_ref()
    }

    /// The marquee being dragged, in board units.
    pub fn marquee(&self) -> Option<Rect> {
        match self.gesture {
            Gesture::Marquee { anchor, to, .. } => Some(rect(anchor, to)),
            _ => None,
        }
    }

    /// Whether the board has the keyboard.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Whether a drag is moving the view.
    pub fn panning(&self) -> bool {
        matches!(self.gesture, Gesture::Panning { .. })
    }

    /// Whether a button is down on the board, so the widget must go on
    /// sending moves and the release wherever the pointer is.
    pub fn dragging(&self) -> bool {
        self.gesture != Gesture::None
    }

    /// The modifiers as last reported.
    pub fn mods(&self) -> Mods {
        self.mods
    }

    /// Whether Space is held, so a left drag pans.
    pub fn space(&self) -> bool {
        self.space
    }

    /// Whether Escape has a rung of its ladder to act on here (§5.14
    /// rungs 3 to 7): the widget takes the key then, and leaves it to the
    /// app otherwise.
    pub fn escapes(&self) -> bool {
        self.gesture != Gesture::None
            || self.editing.is_some()
            || self.tool != Tool::Select
            || !self.selection.is_empty()
    }

    /// When the machine next has something to do with no input, which is
    /// when the node the pointer rests on shows its connect handles (§5.5
    /// rule 8). Whoever feeds the machine sends [`Input::Tick`] then.
    pub fn deadline(&self) -> Option<Instant> {
        let waiting = self.hover.as_ref().filter(|hover| !hover.shown);
        waiting.map(|hover| hover.since + HOVER_DELAY)
    }

    /// Whether the pointer passing over the board with no button down
    /// matters (§5.5 rule 8): the board has the keyboard and may be edited,
    /// nothing is in flight, and no tool is armed, or a shape tool, whose
    /// shape a connector drawn from a handle then ends in.
    pub fn hovers(&self, writable: bool) -> bool {
        self.focused
            && writable
            && self.editing.is_none()
            && self.gesture == Gesture::None
            && matches!(self.tool, Tool::Select | Tool::Shape(_))
    }

    /// Whether the pointer was last seen over a node, so the machine has to
    /// hear when it leaves the board ([`Input::Leave`]).
    pub fn hovering(&self) -> bool {
        self.hover.is_some()
    }

    /// The connector end being dragged.
    pub fn connecting(&self) -> Option<Connecting<'_>> {
        let Gesture::Connecting {
            link,
            to,
            target,
            plan,
        } = &self.gesture
        else {
            return None;
        };
        let loose = match link {
            Link::End(id, end) if plan.is_none() => Some((id, *end)),
            _ => None,
        };
        Some(Connecting {
            target: target.as_ref().map(|(id, side)| (id, *side)),
            loose,
            at: *to,
        })
    }

    /// The handles on screen: the eight of the one selected node, or the
    /// two of the one selected connector, and the four connect handles of
    /// the node the pointer has rested on, while the board has the
    /// keyboard, may be edited and has no label open (§5.3 rule 7, §5.5
    /// rule 8, §5.8 rule 5).
    pub fn handles(&self, view: &View<'_>) -> Shown<'_> {
        let mut shown = Shown::default();
        if !self.focused || !view.writable || self.editing.is_some() {
            return shown;
        }
        let rested = self.hover.as_ref().filter(|hover| hover.shown);
        shown.connect = rested.map(|hover| &hover.id);
        let mut selected = self.selection.iter();
        if let (Some(id), None) = (selected.next(), selected.next()) {
            if view.board.nodes.contains_key(id) {
                shown.resize = Some(id);
            } else if view.board.edges.contains_key(id) {
                shown.ends = Some(id);
            }
        }
        shown
    }

    fn press(&mut self, button: Button, at: (f32, f32), view: &View<'_>, acts: &mut Vec<Act>) {
        // A press during a gesture means the gesture's release never came
        // (the widget lost it with its state), so the press starts over.
        self.cancel(acts);
        self.run = None;
        let pans = match button {
            Button::Middle => true,
            Button::Left => self.space || self.tool == Tool::Hand,
            _ => false,
        };
        // Taken before the press gives the board the keyboard: a board
        // without it shows no handle to press.
        let hit = (!pans && matches!(button, Button::Left | Button::Right)).then(|| {
            hit::hit(
                view.board,
                view.scene,
                view.viewport,
                at,
                self.handles(view),
            )
        });
        self.focused = true;
        if pans {
            self.hover = None;
            self.gesture = Gesture::Panning { button, from: at };
            return;
        }
        let Some(hit) = hit else {
            return;
        };
        // A press on the element being labelled is the label's own: it
        // puts the caret there, and a drag from it selects.
        if self.editing.is_some() && hit.element() == self.editing.as_ref() {
            if button == Button::Left {
                self.gesture = Gesture::Selecting;
                acts.push(Act::Caret {
                    at: view.viewport.to_board(at),
                    select: self.mods.shift,
                });
            }
            return;
        }
        self.commit(acts);
        if button == Button::Right {
            // §5.17 rule 2; in Phase 2 a right-click does nothing more.
            match hit.element() {
                Some(id) if self.selection.contains(id) => {}
                Some(id) => self.select_only(id.clone()),
                None => self.selection.clear(),
            }
            return;
        }
        let on = match (self.tool, hit) {
            // A connect handle shows with no tool armed or a shape tool
            // (`hovers`), and is taken before the tool makes its shape.
            (Tool::Select | Tool::Shape(_), Hit::Connect(id, side)) if view.writable => {
                Target::Connect(id, Some(side))
            }
            (Tool::Shape(_) | Tool::Text, _) if view.writable => Target::Create,
            (Tool::Select, hit) => match hit {
                Hit::Resize(id, handle) => Target::Resize(id, handle),
                Hit::Node(id) | Hit::Connect(id, _) => Target::Node(id),
                Hit::Edge(id) => Target::Edge(id),
                Hit::End(id, end) => Target::End(id, end),
                Hit::Canvas => Target::Canvas,
            },
            // The connector tool draws from a node, a handle on its box
            // included, and does nothing anywhere else (§5.12 rule 1).
            (Tool::Connector, Hit::Node(id) | Hit::Resize(id, _) | Hit::Connect(id, _))
                if view.writable =>
            {
                Target::Connect(id, None)
            }
            _ => return,
        };
        // What a click makes or runs, it does at each click.
        let once = matches!(on, Target::Create | Target::Connect(..));
        let again = self.click.take().is_some_and(|(time, from)| {
            view.now.saturating_duration_since(time) <= DOUBLE_CLICK
                && distance(from, at) < DOUBLE_REACH
        });
        let double = again && !once;
        if !double && !once {
            self.click = Some((view.now, at));
        }
        self.gesture = Gesture::Pressed {
            at,
            anchor: view.viewport.to_board(at),
            on,
            double,
        };
    }

    fn moved(&mut self, at: (f32, f32), view: &View<'_>, acts: &mut Vec<Act>) {
        let to = view.viewport.to_board(at);
        let tool = self.tool;
        match &mut self.gesture {
            Gesture::None => self.hover(at, view),
            Gesture::Pressed { at: from, .. } => {
                if distance(*from, at) < THRESHOLD {
                    return;
                }
                let Gesture::Pressed { anchor, on, .. } = std::mem::take(&mut self.gesture) else {
                    return;
                };
                self.click = None;
                self.gesture = self.drag(on, anchor, view);
                self.moved(at, view, acts);
            }
            Gesture::Panning { from, .. } => {
                let look = view.viewport.panned(from.0 - at.0, from.1 - at.1);
                *from = at;
                if look != view.viewport {
                    acts.push(Act::Look(look));
                }
            }
            Gesture::Moving {
                anchor,
                origin,
                ids,
                copy,
                by,
            } => {
                let (dx, dy) = (to.0 - anchor.0, to.1 - anchor.1);
                let now = match origin {
                    Some([x, y]) => [landing(*x, dx), landing(*y, dy)],
                    None => [snap::grid(dx), snap::grid(dy)],
                };
                if now != *by {
                    *by = now;
                    acts.push(Act::Preview(
                        (now != [0, 0]).then(|| moving(ids, *copy, now)),
                    ));
                }
            }
            Gesture::Resizing {
                anchor,
                id,
                handle,
                frame,
                edges,
            } => {
                let (dx, dy) = (to.0 - anchor.0, to.1 - anchor.1);
                let [left, top, right, bottom] = *frame;
                let mut now = Edges::default();
                let [across, down] = handle.sides();
                match across {
                    Some(Side::Left) => now.left = landing(left, dx),
                    Some(_) => now.right = landing(right, dx),
                    None => {}
                }
                match down {
                    Some(Side::Top) => now.top = landing(top, dy),
                    Some(_) => now.bottom = landing(bottom, dy),
                    None => {}
                }
                if now != *edges {
                    *edges = now;
                    acts.push(Act::Preview(
                        (now != Edges::default()).then(|| resizing(id, now)),
                    ));
                }
            }
            Gesture::Marquee {
                anchor,
                to: corner,
                base,
                ..
            } => {
                *corner = to;
                let mut selection = base.clone();
                selection.extend(hit::inside(view.board, view.scene, rect(*anchor, to)));
                self.selection = selection;
            }
            Gesture::Creating { anchor, area } => {
                let now = Some(dragged(*anchor, to));
                if now != *area {
                    *area = now;
                    acts.extend(now.map(|area| Act::Preview(Some(creating(self.tool, area)))));
                }
            }
            Gesture::Connecting {
                link,
                to: pointer,
                target,
                plan,
            } => {
                *pointer = to;
                let over = hit::attach(view.board, view.scene, view.viewport, at);
                let (over, now) = connecting(link, over, to, tool, view.board);
                *target = over;
                if now != *plan {
                    plan.clone_from(&now);
                    acts.push(Act::Preview(now));
                }
            }
            Gesture::Selecting => acts.push(Act::Caret {
                at: to,
                select: true,
            }),
        }
    }

    /// The pointer moved to the widget-local point `at` with no button
    /// down (§5.5 rule 8). The node under it is the one it rests on, from
    /// when it came there. Its connect handles sit outside it, so once they
    /// show, the pointer may cross the gap to them without losing them.
    fn hover(&mut self, at: (f32, f32), view: &View<'_>) {
        if !self.hovers(view.writable) {
            self.hover = None;
            return;
        }
        let hit = hit::hit(
            view.board,
            view.scene,
            view.viewport,
            at,
            self.handles(view),
        );
        let over = match hit {
            Hit::Node(id) | Hit::Resize(id, _) | Hit::Connect(id, _) => Some(id),
            Hit::Edge(_) | Hit::End(..) | Hit::Canvas => None,
        };
        self.hover = match (over, self.hover.take()) {
            (Some(id), Some(hover)) if hover.id == id => Some(hover),
            (Some(id), _) => Some(Hover {
                id,
                since: view.now,
                shown: false,
            }),
            (None, Some(hover)) if hover.shown => {
                let reach = handles::CONNECT_GAP + handles::REACH;
                let around = Frames::new(view.board).get(&hover.id).map(|bounds| {
                    let (left, top) = view.viewport.to_screen((bounds.x, bounds.y));
                    let (right, bottom) =
                        view.viewport.to_screen((bounds.right(), bounds.bottom()));
                    (left - reach..=right + reach, top - reach..=bottom + reach)
                });
                around
                    .is_some_and(|(across, down)| across.contains(&at.0) && down.contains(&at.1))
                    .then_some(hover)
            }
            (None, _) => None,
        };
        self.rest(view.now);
    }

    /// The node the pointer rests on shows its connect handles once it has
    /// rested there [`HOVER_DELAY`].
    fn rest(&mut self, now: Instant) {
        if let Some(hover) = &mut self.hover {
            hover.shown |= now.saturating_duration_since(hover.since) >= HOVER_DELAY;
        }
    }

    /// The gesture a press on `on` becomes once the pointer has moved
    /// [`THRESHOLD`].
    fn drag(&mut self, on: Target, anchor: (f32, f32), view: &View<'_>) -> Gesture {
        self.hover = None;
        let connecting = |link| Gesture::Connecting {
            link,
            to: anchor,
            target: None,
            plan: None,
        };
        match on {
            Target::Connect(id, _) if view.writable => connecting(Link::New(id)),
            Target::End(id, end) if view.writable => connecting(Link::End(id, end)),
            Target::Node(id) | Target::Edge(id) if view.writable => {
                // A drag moves the selection; a press outside it selects
                // what it pressed first, alone unless Shift adds it.
                if !self.selection.contains(&id) {
                    if !self.mods.shift {
                        self.selection.clear();
                    }
                    self.selection.insert(id.clone());
                    self.cursor = Some(id.clone());
                }
                Gesture::Moving {
                    anchor,
                    origin: Frames::new(view.board).corner(&id),
                    ids: self.selection.iter().cloned().collect(),
                    copy: self.mods.duplicate,
                    by: [0, 0],
                }
            }
            Target::Resize(id, handle) if view.writable => {
                let corner = Frames::new(view.board).corner(&id);
                let size = view.board.nodes.get(&id).map(|node| node.size);
                match (corner, size) {
                    (Some([x, y]), Some([width, height])) => Gesture::Resizing {
                        anchor,
                        id,
                        handle,
                        frame: [x, y, x + i64::from(width), y + i64::from(height)],
                        edges: Edges::default(),
                    },
                    _ => Gesture::None,
                }
            }
            Target::Canvas => Gesture::Marquee {
                anchor,
                to: anchor,
                base: if self.mods.shift {
                    self.selection.clone()
                } else {
                    BTreeSet::new()
                },
                before: self.selection.clone(),
            },
            Target::Create if view.writable => Gesture::Creating { anchor, area: None },
            // A read-only board has nothing to drag but a marquee.
            _ => Gesture::None,
        }
    }

    fn release(&mut self, button: Button, view: &View<'_>, acts: &mut Vec<Act>) {
        let ends = match self.gesture {
            Gesture::None => false,
            Gesture::Panning { button: held, .. } => held == button,
            _ => button == Button::Left,
        };
        if !ends {
            return;
        }
        match std::mem::take(&mut self.gesture) {
            // The hand tool is one-shot, as every tool is (§5.2).
            Gesture::Panning {
                button: Button::Left,
                ..
            } if self.tool == Tool::Hand => self.tool = Tool::Select,
            Gesture::None | Gesture::Panning { .. } | Gesture::Selecting => {}
            Gesture::Pressed {
                anchor, on, double, ..
            } => self.click(on, double, anchor, view, acts),
            Gesture::Moving { ids, copy, by, .. } => {
                if by != [0, 0] {
                    acts.push(Act::Edit(moving(&ids, copy, by)));
                }
            }
            Gesture::Resizing { id, edges, .. } => {
                if edges != Edges::default() {
                    acts.push(Act::Edit(resizing(&id, edges)));
                }
            }
            Gesture::Marquee { .. } => self.point(view.board),
            Gesture::Creating { anchor, area } => {
                let area = area.unwrap_or_else(|| dragged(anchor, anchor));
                acts.push(Act::Edit(creating(self.tool, area)));
                self.tool = Tool::Select;
            }
            // Released where it changes nothing, on the node a new
            // connector came from or where an end may not go, the gesture
            // is over and the tool stays for another try.
            Gesture::Connecting { plan, .. } => {
                if let Some(intent) = plan {
                    acts.push(Act::Edit(intent));
                    self.tool = Tool::Select;
                }
            }
        }
    }

    /// A press released where it went down (§5.3 rule 3, §5.5 rules 3 to
    /// 5).
    fn click(
        &mut self,
        on: Target,
        double: bool,
        anchor: (f32, f32),
        view: &View<'_>,
        acts: &mut Vec<Act>,
    ) {
        match on {
            Target::Node(id) | Target::Edge(id) | Target::End(id, _) => {
                if double && view.writable {
                    self.select_only(id.clone());
                    self.open(id, acts);
                } else if !self.mods.shift {
                    self.select_only(id);
                } else if !self.selection.remove(&id) {
                    self.selection.insert(id.clone());
                    self.cursor = Some(id);
                }
            }
            Target::Resize(..) | Target::Connect(_, None) => {}
            // A connect handle clicked is `board.connect.<side>` from its
            // node (§5.5 rule 8).
            Target::Connect(id, Some(side)) => {
                self.hover = None;
                self.select_only(id);
                let dir = match side {
                    Side::Top => Dir::Up,
                    Side::Right => Dir::Right,
                    Side::Bottom => Dir::Down,
                    Side::Left => Dir::Left,
                };
                self.extend(Some(dir), view, acts);
            }
            Target::Canvas => {
                if double && view.writable {
                    let (kind, shape, size) = made(Tool::Select);
                    acts.push(Act::Edit(Intent::AddNode {
                        kind,
                        shape,
                        at: centred(anchor, size),
                        size,
                        from: None,
                    }));
                } else if !self.mods.shift {
                    self.selection.clear();
                }
            }
            Target::Create => {
                let (_, _, size) = made(self.tool);
                let area = Area {
                    at: centred(anchor, size),
                    size,
                };
                acts.push(Act::Edit(creating(self.tool, area)));
                self.tool = Tool::Select;
            }
        }
    }

    /// One rung of Escape's ladder, the first that applies (§5.14 rungs 3
    /// to 8; Connecting's rung 5 comes with it).
    fn escape(&mut self, acts: &mut Vec<Act>) {
        self.run = None;
        if self.gesture != Gesture::None {
            self.cancel(acts);
        } else if self.editing.is_some() {
            self.commit(acts);
        } else if self.tool != Tool::Select {
            self.tool = Tool::Select;
        } else {
            self.selection.clear();
        }
    }

    /// Drop the gesture in flight, with the board as it was at the press.
    fn cancel(&mut self, acts: &mut Vec<Act>) {
        let shows = match std::mem::take(&mut self.gesture) {
            Gesture::Moving { by, .. } => by != [0, 0],
            Gesture::Resizing { edges, .. } => edges != Edges::default(),
            Gesture::Creating { area, .. } => area.is_some(),
            Gesture::Connecting { plan, .. } => plan.is_some(),
            Gesture::Marquee { before, .. } => {
                self.selection = before;
                false
            }
            Gesture::None
            | Gesture::Pressed { .. }
            | Gesture::Panning { .. }
            | Gesture::Selecting => false,
        };
        if shows {
            acts.push(Act::Preview(None));
        }
    }

    fn commit(&mut self, acts: &mut Vec<Act>) {
        if self.editing.take().is_some() {
            acts.push(Act::CommitLabel);
        }
    }

    /// Open the label of `id`. A label is edited with no tool armed, and
    /// Space is text in it.
    fn open(&mut self, id: Id, acts: &mut Vec<Act>) {
        self.tool = Tool::Select;
        self.space = false;
        self.editing = Some(id.clone());
        acts.push(Act::OpenLabel(id));
    }

    fn select_only(&mut self, id: Id) {
        self.selection = BTreeSet::from([id.clone()]);
        self.cursor = Some(id);
    }

    /// The one selected element.
    fn only(&self) -> Option<Id> {
        let mut selected = self.selection.iter();
        match (selected.next(), selected.next()) {
            (Some(id), None) => Some(id.clone()),
            _ => None,
        }
    }

    /// The selected nodes.
    fn nodes(&self, board: &Board) -> Vec<Id> {
        let nodes = self.selection.iter();
        nodes
            .filter(|id| board.nodes.contains_key(*id))
            .cloned()
            .collect()
    }

    /// After a marquee the cursor is among what it selected: where it was
    /// if that is selected, else the selected node nearest the top, then
    /// the left, else the first selected connector.
    fn point(&mut self, board: &Board) {
        let kept = self.cursor.as_ref();
        if kept.is_some_and(|cursor| self.selection.contains(cursor)) {
            return;
        }
        let frames = Frames::new(board);
        let corners = self.selection.iter().filter_map(|id| {
            let [x, y] = frames.corner(id)?;
            Some(((y, x), id))
        });
        let first = corners.min().map(|(_, id)| id);
        if let Some(id) = first.or(self.selection.iter().next()) {
            self.cursor = Some(id.clone());
        }
    }

    /// One key move or key resize, in the undo step of the run it
    /// continues (§5.11 rule 3): the same kind of key, with nothing else
    /// between and less than [`RUN_PAUSE`] since the last.
    fn nudge(
        &mut self,
        kind: Run,
        run: Option<(Run, Instant)>,
        intent: Intent,
        now: Instant,
        acts: &mut Vec<Act>,
    ) {
        let joins = run.is_some_and(|(last, at)| {
            last == kind && now.saturating_duration_since(at) < RUN_PAUSE
        });
        self.run = Some((kind, now));
        acts.push(if joins {
            Act::Continue(intent)
        } else {
            Act::Edit(intent)
        });
    }

    /// How far a key moves the selection towards `dir` (§5.8 rule 4): one
    /// cell, or as far as the next grid line when the cursor node is off
    /// the grid that way.
    fn cell(&self, dir: Dir, board: &Board) -> [i32; 2] {
        let cursor = self.cursor.as_ref();
        let corner = cursor
            .filter(|id| self.selection.contains(*id))
            .and_then(|id| Frames::new(board).corner(id));
        let (axis, forward) = match dir {
            Dir::Left => (0, false),
            Dir::Right => (0, true),
            Dir::Up => (1, false),
            Dir::Down => (1, true),
        };
        let off = corner.map_or(0, |corner| corner[axis].rem_euclid(i64::from(GRID)));
        // `off` is under one cell, so it is an `i32`.
        let off = i32::try_from(off).unwrap_or(0);
        let mut by = [0, 0];
        by[axis] = match (off, forward) {
            (0, true) => GRID,
            (0, false) => -GRID,
            (off, true) => GRID - off,
            (off, false) => -off,
        };
        by
    }

    /// `board.connect.next` and `board.connect.<dir>` from the one selected
    /// node, or for the former from the node the one selected connector
    /// points at (§5.6 rules 5 to 7). The armed shape is the new node's,
    /// and the tool is spent.
    fn extend(&mut self, dir: Option<Dir>, view: &View<'_>, acts: &mut Vec<Act>) {
        let Some(id) = self.only().filter(|_| view.writable) else {
            return;
        };
        let board = view.board;
        let from = if board.nodes.contains_key(&id) {
            Some(id)
        } else if dir.is_none() {
            board.edges.get(&id).and_then(|edge| match &edge.to {
                End::Node { node, .. } => Id::new(node).filter(|id| board.nodes.contains_key(id)),
                End::Free { .. } => None,
            })
        } else {
            None
        };
        let Some(from) = from else {
            return;
        };
        let shape = match self.tool {
            Tool::Shape(shape) => Some(shape),
            _ => None,
        };
        self.tool = Tool::Select;
        acts.push(Act::Edit(Intent::Extend { from, dir, shape }));
    }

    fn wheel(
        &mut self,
        delta: Scroll,
        at: (f32, f32),
        zoom: bool,
        sideways: bool,
        scale_factor: f32,
        view: &View<'_>,
    ) -> Option<Viewport> {
        let zoom = match self.burst {
            Some((last, choice)) if view.now.saturating_duration_since(last) < BURST => choice,
            _ => zoom,
        };
        self.burst = Some((view.now, zoom));
        // Everything in lines: `y` is what a zoom reads.
        let (x, y, lines) = match delta {
            Scroll::Lines { x, y } if sideways && x == 0.0 => (y, 0.0, y),
            Scroll::Lines { x, y } => (x, y, y),
            Scroll::Pixels { x, y } => {
                let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
                    scale_factor
                } else {
                    1.0
                };
                let unit = scale * LINE;
                (x / unit, y / unit, y / unit)
            }
        };
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        Some(if zoom {
            view.viewport.zoomed(ZOOM_PER_LINE.powf(lines), at)
        } else {
            view.viewport.panned(-x * LINE, -y * LINE)
        })
    }
}

fn exists(board: &Board, id: &Id) -> bool {
    board.nodes.contains_key(id) || board.edges.contains_key(id)
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The box between two board points.
fn rect(a: (f32, f32), b: (f32, f32)) -> Rect {
    let (x, y) = (a.0.min(b.0), a.1.min(b.1));
    Rect::new(x, y, (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

/// How far an edge at `from` moves to land on the grid line nearest
/// `from + by` (§5.8 rules 2 and 5).
fn landing(from: i64, by: f32) -> i32 {
    let to = i64::from(snap::grid(from as f32 + by));
    i32::try_from(to - from).unwrap_or(if to < from { i32::MIN } else { i32::MAX })
}

/// What a pointer move asks for: the selection moved, or copies of it.
fn moving(ids: &[Id], copy: bool, by: [i32; 2]) -> Intent {
    let ids = ids.to_vec();
    if copy {
        Intent::Duplicate { ids, by }
    } else {
        Intent::Move { ids, by }
    }
}

fn resizing(id: &Id, edges: Edges) -> Intent {
    Intent::Resize {
        ids: vec![id.clone()],
        edges,
    }
}

/// The kind, shape and default size of what `tool` creates; without a
/// shape tool, a rectangle (§5.5 rules 1 and 2).
fn made(tool: Tool) -> (NodeKind, Shape, [i32; 2]) {
    match tool {
        Tool::Text => (NodeKind::Text, Shape::Rect, TEXT_SIZE),
        Tool::Shape(shape) => (NodeKind::Shape, shape, NODE_SIZE),
        _ => (NodeKind::Shape, Shape::Rect, NODE_SIZE),
    }
}

fn creating(tool: Tool, area: Area) -> Intent {
    let (kind, shape, _) = made(tool);
    Intent::AddNode {
        kind,
        shape,
        at: area.at,
        size: area.size,
        from: None,
    }
}

/// What releasing a dragged connector end at the board point `to` would
/// do (§5.12 rules 1, 5 and 6), with `over` the node it would attach to
/// and the side it would be pinned to: that target, where it is one the
/// end may take, and the edit.
///
/// A new connector ends on the node, or on empty canvas in a new node
/// centred there, in the armed shape; over the node it came from it is
/// nothing. An end of a connector moves to the node, unless the other end
/// is there already; on empty canvas an end that is a free point moves
/// there, on the grid, and an attached one stays where it was.
fn connecting(
    link: &Link,
    over: Option<(Id, Option<Side>)>,
    to: (f32, f32),
    tool: Tool,
    board: &Board,
) -> (Option<(Id, Option<Side>)>, Option<Intent>) {
    let attached = |(node, side): &(Id, Option<Side>)| Endpoint::Node {
        node: node.clone(),
        side: *side,
    };
    match link {
        Link::New(from) => {
            let source = Endpoint::Node {
                node: from.clone(),
                side: None,
            };
            match over {
                Some((node, _)) if node == *from => (None, None),
                Some(target) => {
                    let intent = Intent::AddEdge {
                        from: source,
                        to: attached(&target),
                    };
                    (Some(target), Some(intent))
                }
                None => {
                    let (kind, shape, size) = made(tool);
                    let intent = Intent::AddNode {
                        kind,
                        shape,
                        at: centred(to, size),
                        size,
                        from: Some(source),
                    };
                    (None, Some(intent))
                }
            }
        }
        Link::End(id, end) => {
            let Some(edge) = board.edges.get(id) else {
                return (None, None);
            };
            let (own, other) = match end {
                EdgeEnd::From => (&edge.from, &edge.to),
                EdgeEnd::To => (&edge.to, &edge.from),
            };
            let rebind = |to| Intent::Rebind {
                id: id.clone(),
                end: *end,
                to,
            };
            match over {
                Some((node, _)) if matches!(other, End::Node { node: there, .. } if *there == node.as_str()) => {
                    (None, None)
                }
                Some(target) => {
                    let intent = rebind(attached(&target));
                    (Some(target), Some(intent))
                }
                None if matches!(own, End::Free { .. }) => {
                    let at = [snap::grid(to.0), snap::grid(to.1)];
                    (None, Some(rebind(Endpoint::Free { at })))
                }
                None => (None, None),
            }
        }
    }
}

/// The corner of a box of `size` centred on `point`, on the grid (§5.5
/// rule 3).
fn centred(point: (f32, f32), size: [i32; 2]) -> [i32; 2] {
    [
        snap::grid(point.0 - size[0] as f32 / 2.0),
        snap::grid(point.1 - size[1] as f32 / 2.0),
    ]
}

/// The box a drag from `anchor` to `to` creates: both corners on the
/// grid, and at least one cell each way, towards the drag (§5.5 rule 3).
fn dragged(anchor: (f32, f32), to: (f32, f32)) -> Area {
    let span = |from: f32, to: f32| {
        let (a, b) = (snap::grid(from), snap::grid(to));
        match a.cmp(&b) {
            std::cmp::Ordering::Less => (a, b.saturating_sub(a)),
            std::cmp::Ordering::Greater => (b, a.saturating_sub(b)),
            std::cmp::Ordering::Equal if to < from => (a.saturating_sub(GRID), GRID),
            std::cmp::Ordering::Equal => (a, GRID),
        }
    };
    let (x, width) = span(anchor.0, to.0);
    let (y, height) = span(anchor.1, to.1);
    Area {
        at: [x, y],
        size: [width, height],
    }
}

#[cfg(test)]
mod tests {
    use wren_board::edit::{self, Change};
    use wren_board::ids::{Ids, Seeded};
    use wren_board::scene::{self, FixedMeasure};
    use wren_board::{parse, Extra, Named, Node, NodeStyle};

    use super::*;
    use crate::fixtures;

    const AT: (f32, f32) = (317.0, 211.0);

    fn wheel(delta: Scroll, zoom: bool) -> Input {
        Input::Wheel {
            delta,
            at: AT,
            zoom,
            sideways: false,
            scale_factor: 1.0,
        }
    }

    fn lines(y: f32) -> Scroll {
        Scroll::Lines { x: 0.0, y }
    }

    /// One step over an empty board.
    fn step(machine: &mut Machine, input: Input, viewport: Viewport, now: Instant) -> Vec<Act> {
        let board = Board::default();
        let scene = Scene {
            bounds: Rect::default(),
            items: Vec::new(),
        };
        let view = View {
            viewport,
            now,
            board: &board,
            scene: &scene,
            writable: true,
        };
        machine.step(input, &view)
    }

    /// Steps `inputs` from `viewport`, `gap` apart, applying each look.
    fn run(
        machine: &mut Machine,
        viewport: &mut Viewport,
        start: Instant,
        gap: Duration,
        inputs: &[Input],
    ) -> Instant {
        let mut now = start;
        for input in inputs {
            for act in step(machine, *input, *viewport, now) {
                if let Act::Look(to) = act {
                    *viewport = to;
                }
            }
            now += gap;
        }
        now
    }

    fn look(machine: &mut Machine, viewport: Viewport, input: Input, now: Instant) -> Viewport {
        match step(machine, input, viewport, now).as_slice() {
            [Act::Look(to)] => *to,
            [] => viewport,
            more => panic!("one look at most: {more:?}"),
        }
    }

    #[test]
    fn the_wheel_pans_by_forty_pixels_a_line_at_any_zoom() {
        let now = Instant::now();
        for zoom in [0.1, 0.5, 1.0, 4.0] {
            let viewport = Viewport {
                x: 10.0,
                y: 20.0,
                zoom,
            };
            let mut machine = Machine::default();
            // Wheel down shows what is below.
            let to = look(&mut machine, viewport, wheel(lines(-1.0), false), now);
            assert_eq!(to.zoom, zoom);
            assert_eq!(to.to_screen((10.0, 20.0)), (0.0, -LINE));
            // Sideways lines pan sideways.
            let mut machine = Machine::default();
            let to = look(
                &mut machine,
                viewport,
                wheel(Scroll::Lines { x: 2.0, y: 0.0 }, false),
                now,
            );
            assert_eq!(to.to_screen((10.0, 20.0)), (2.0 * LINE, 0.0));
        }
    }

    #[test]
    fn shift_turns_a_vertical_line_sideways() {
        let now = Instant::now();
        let viewport = Viewport::default();
        let mut machine = Machine::default();
        let input = Input::Wheel {
            delta: lines(-1.0),
            at: AT,
            zoom: false,
            sideways: true,
            scale_factor: 1.0,
        };
        let to = look(&mut machine, viewport, input, now);
        assert_eq!((to.x, to.y), (LINE, 0.0));
    }

    #[test]
    fn pixels_are_divided_by_the_scale_factor() {
        let now = Instant::now();
        let viewport = Viewport::default();
        for scale_factor in [1.0, 1.5, 2.0] {
            let input = Input::Wheel {
                delta: Scroll::Pixels { x: 0.0, y: -30.0 },
                at: AT,
                zoom: false,
                sideways: false,
                scale_factor,
            };
            let to = look(&mut Machine::default(), viewport, input, now);
            assert!(
                (to.y - 30.0 / scale_factor).abs() < 1e-4,
                "{scale_factor}: {to:?}"
            );
            // As a zoom, the same delta is that share of a line.
            let input = Input::Wheel {
                delta: Scroll::Pixels {
                    x: 0.0,
                    y: 80.0 * scale_factor,
                },
                at: AT,
                zoom: true,
                sideways: false,
                scale_factor,
            };
            let to = look(&mut Machine::default(), viewport, input, now);
            assert!((to.zoom - ZOOM_PER_LINE * ZOOM_PER_LINE).abs() < 1e-5);
        }
    }

    #[test]
    fn a_zoom_step_keeps_the_board_point_under_the_pointer() {
        let start = Instant::now();
        let mut viewport = Viewport {
            x: -40.0,
            y: 125.5,
            zoom: 1.0,
        };
        let mut machine = Machine::default();
        let mut now = start;
        for y in [1.0, 1.0, -3.0, 0.5, 2.0, -1.0, 4.0] {
            let under = viewport.to_board(AT);
            // Bursts apart, so each reads its own modifier.
            viewport = look(&mut machine, viewport, wheel(lines(y), true), now);
            now += BURST * 2;
            let after = viewport.to_screen(under);
            assert!(
                (after.0 - AT.0).abs() < 0.01 && (after.1 - AT.1).abs() < 0.01,
                "{y}: {after:?}"
            );
        }
        assert!((viewport.zoom - ZOOM_PER_LINE.powf(4.5)).abs() < 1e-4);
    }

    #[test]
    fn wheel_zoom_is_clamped_to_ten_and_four_hundred_percent() {
        let start = Instant::now();
        let mut viewport = Viewport::default();
        let mut machine = Machine::default();
        let up = [wheel(lines(10.0), true); 10];
        run(&mut machine, &mut viewport, start, BURST, &up);
        assert_eq!(viewport.zoom, Viewport::MAX_ZOOM);
        // At the clamp a further step neither zooms nor pans.
        let at_max = viewport;
        let more = step(
            &mut machine,
            wheel(lines(1.0), true),
            viewport,
            start + BURST * 30,
        );
        assert!(more.is_empty());
        assert_eq!(viewport, at_max);
        let down = [wheel(lines(-10.0), true); 10];
        run(
            &mut machine,
            &mut viewport,
            start + BURST * 40,
            BURST,
            &down,
        );
        assert_eq!(viewport.zoom, Viewport::MIN_ZOOM);
    }

    #[test]
    fn a_burst_keeps_the_choice_of_its_first_event() {
        let start = Instant::now();
        let gap = Duration::from_millis(20);
        let mut viewport = Viewport::default();
        let mut machine = Machine::default();
        // A pan, then the zoom modifier goes down mid-burst: still a pan.
        let end = run(
            &mut machine,
            &mut viewport,
            start,
            gap,
            &[
                wheel(lines(-1.0), false),
                wheel(lines(-1.0), true),
                wheel(lines(-1.0), true),
            ],
        );
        assert_eq!((viewport.y, viewport.zoom), (3.0 * LINE, 1.0));
        // A zoom burst whose modifier comes up, as macOS momentum does:
        // still a zoom.
        let mut viewport = Viewport::default();
        let end = run(
            &mut machine,
            &mut viewport,
            end + BURST,
            gap,
            &[wheel(lines(1.0), true), wheel(lines(1.0), false)],
        );
        assert!((viewport.zoom - ZOOM_PER_LINE * ZOOM_PER_LINE).abs() < 1e-5);
        // Each event extends the burst, so a long run at a short gap is one.
        let mut viewport = Viewport::default();
        let steps = [wheel(lines(-1.0), true); 20];
        let mut now = end + BURST;
        now = run(
            &mut machine,
            &mut viewport,
            now,
            gap,
            &[wheel(lines(-1.0), false)],
        );
        run(&mut machine, &mut viewport, now, gap, &steps);
        assert_eq!((viewport.y, viewport.zoom), (21.0 * LINE, 1.0));
    }

    #[test]
    fn a_new_burst_reads_the_modifier_again() {
        let start = Instant::now();
        let mut machine = Machine::default();
        let viewport = Viewport::default();
        let panned = look(&mut machine, viewport, wheel(lines(-1.0), false), start);
        assert_eq!(panned.zoom, 1.0);
        // Exactly the burst apart is a new burst.
        let zoomed = look(
            &mut machine,
            viewport,
            wheel(lines(1.0), true),
            start + BURST,
        );
        assert!((zoomed.zoom - ZOOM_PER_LINE).abs() < 1e-6);
        // A clock that reads earlier than the last event is still a burst.
        let early = look(&mut machine, viewport, wheel(lines(-1.0), false), start);
        assert!(early.zoom < 1.0);
    }

    #[test]
    fn a_middle_drag_pans_by_the_pointer_motion() {
        let now = Instant::now();
        let mut machine = Machine::default();
        let mut viewport = Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 2.0,
        };
        let under = viewport.to_board((100.0, 100.0));
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                Input::Press {
                    button: Button::Middle,
                    at: (100.0, 100.0),
                },
                Input::Move { at: (130.0, 90.0) },
                Input::Move { at: (160.0, 115.0) },
            ],
        );
        assert!(machine.panning());
        // The board point that was under the pointer still is.
        assert_eq!(viewport.to_screen(under), (160.0, 115.0));
        // A release of another button does not end the drag.
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[Input::Release {
                button: Button::Left,
            }],
        );
        assert!(machine.panning());
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                Input::Release {
                    button: Button::Middle,
                },
                Input::Move { at: (0.0, 0.0) },
            ],
        );
        assert!(!machine.panning());
        assert_eq!(viewport.to_screen(under), (160.0, 115.0));
    }

    #[test]
    fn a_left_drag_pans_only_with_space_held() {
        let now = Instant::now();
        let mut machine = Machine::default();
        let mut viewport = Viewport::default();
        let drag = [
            Input::Press {
                button: Button::Left,
                at: (10.0, 10.0),
            },
            Input::Move { at: (50.0, 30.0) },
            Input::Release {
                button: Button::Left,
            },
        ];
        run(&mut machine, &mut viewport, now, Duration::ZERO, &drag);
        assert_eq!(viewport, Viewport::default());

        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[Input::Space { held: true }],
        );
        assert!(machine.space());
        run(&mut machine, &mut viewport, now, Duration::ZERO, &drag);
        assert_eq!((viewport.x, viewport.y), (-40.0, -20.0));

        // Space let go mid-drag: the drag goes on to its release.
        let mut viewport = Viewport::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[drag[0], Input::Space { held: false }, drag[1], drag[2]],
        );
        assert_eq!((viewport.x, viewport.y), (-40.0, -20.0));
        assert!(!machine.panning() && !machine.space());
        // Neither the right button nor another one pans.
        for button in [Button::Right, Button::Other] {
            step(&mut machine, Input::Space { held: true }, viewport, now);
            let press = Input::Press {
                button,
                at: (0.0, 0.0),
            };
            step(&mut machine, press, viewport, now);
            assert!(!machine.panning());
        }
    }

    #[test]
    fn cancel_ends_a_drag_and_lets_space_go() {
        let now = Instant::now();
        let mut machine = Machine::default();
        let mut viewport = Viewport::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                Input::Space { held: true },
                Input::Press {
                    button: Button::Left,
                    at: (0.0, 0.0),
                },
                Input::Move { at: (8.0, 0.0) },
                Input::Cancel,
                Input::Move { at: (80.0, 0.0) },
            ],
        );
        assert_eq!(viewport.x, -8.0);
        assert!(!machine.panning() && !machine.space());
    }

    #[test]
    fn a_press_after_a_lost_release_starts_over() {
        let now = Instant::now();
        let middle = Input::Press {
            button: Button::Middle,
            at: (100.0, 100.0),
        };
        // A middle drag whose release never came, then a plain left drag.
        let mut machine = Machine::default();
        let mut viewport = Viewport::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                middle,
                Input::Press {
                    button: Button::Left,
                    at: (600.0, 400.0),
                },
                Input::Move { at: (650.0, 400.0) },
                Input::Release {
                    button: Button::Left,
                },
            ],
        );
        assert_eq!(viewport, Viewport::default());
        assert!(!machine.panning());
        // A second middle drag pans from its own press, with no jump.
        let mut machine = Machine::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                middle,
                Input::Press {
                    button: Button::Middle,
                    at: (600.0, 400.0),
                },
                Input::Move { at: (650.0, 400.0) },
            ],
        );
        assert_eq!((viewport.x, viewport.y), (-50.0, 0.0));
        assert!(machine.panning());
    }

    #[test]
    fn a_wheel_during_a_drag_still_works() {
        let now = Instant::now();
        let mut machine = Machine::default();
        let mut viewport = Viewport::default();
        run(
            &mut machine,
            &mut viewport,
            now,
            Duration::ZERO,
            &[
                Input::Press {
                    button: Button::Middle,
                    at: (0.0, 0.0),
                },
                wheel(lines(-1.0), false),
                Input::Move { at: (-10.0, 0.0) },
            ],
        );
        assert_eq!((viewport.x, viewport.y), (10.0, LINE));
        assert!(machine.panning());
    }

    #[test]
    fn nothing_that_is_not_a_number_moves_the_view() {
        let now = Instant::now();
        let viewport = Viewport::default();
        for delta in [
            Scroll::Lines {
                x: f32::NAN,
                y: 0.0,
            },
            Scroll::Pixels {
                x: 0.0,
                y: f32::INFINITY,
            },
        ] {
            for zoom in [false, true] {
                let acts = step(&mut Machine::default(), wheel(delta, zoom), viewport, now);
                assert!(acts.is_empty());
            }
        }
        // A scale factor that is no scale counts as 1.
        for scale_factor in [0.0, -2.0, f32::NAN] {
            let input = Input::Wheel {
                delta: Scroll::Pixels { x: 0.0, y: -1.0 },
                at: AT,
                zoom: false,
                sideways: false,
                scale_factor,
            };
            let to = look(&mut Machine::default(), viewport, input, now);
            assert_eq!((to.x, to.y), (0.0, 1.0), "{scale_factor}");
        }
    }

    #[test]
    fn a_step_that_moves_nothing_says_nothing() {
        let now = Instant::now();
        let viewport = Viewport::default();
        let mut machine = Machine::default();
        let still = wheel(Scroll::Lines { x: 0.0, y: 0.0 }, false);
        assert!(step(&mut machine, still, viewport, now).is_empty());
        let press = Input::Press {
            button: Button::Middle,
            at: (5.0, 5.0),
        };
        step(&mut machine, press, viewport, now);
        let same = Input::Move { at: (5.0, 5.0) };
        assert!(step(&mut machine, same, viewport, now).is_empty());
    }

    /// Five nodes and two connectors. `kid` is drawn inside `box`, at
    /// 420,220; `off` is off the grid both ways; `e` runs along y = 30 from
    /// `a` to `b`, and `free` along y = 400 between two free ends.
    const BOARD: &str = r#"schema = 1

[node.a]
text = "A"
kind = "shape"
at = [0, 0]
size = [120, 60]
z = "A"

[node.b]
text = "B"
kind = "shape"
at = [200, 0]
size = [120, 60]
z = "B"

[node.box]
kind = "shape"
at = [400, 200]
size = [200, 120]
z = "C"

[node.kid]
text = "Kid"
kind = "shape"
at = [20, 20]
parent = "box"
size = [80, 40]
z = "A"

[node.off]
text = "Off"
kind = "shape"
at = [7, 213]
size = [120, 60]
z = "D"

[edge.e]
kind = "edge"
from = { node = "a" }
to = { node = "b" }
z = "E"

[edge.free]
kind = "edge"
from = { at = [0, 400] }
to = { at = [200, 400] }
route = "straight"
z = "F"
"#;

    /// Board points: inside `a`, `b`, `kid` and `off`; on `e` and `free`;
    /// on nothing; and the bottom-right corner of `a`.
    const A: (f32, f32) = (60.0, 30.0);
    const B: (f32, f32) = (260.0, 30.0);
    const KID: (f32, f32) = (460.0, 240.0);
    const OFF: (f32, f32) = (67.0, 243.0);
    const E: (f32, f32) = (160.0, 30.0);
    const FREE: (f32, f32) = (100.0, 400.0);
    const CANVAS: (f32, f32) = (300.0, 150.0);
    const A_CORNER: (f32, f32) = (120.0, 60.0);
    /// The connect handle outside the right side of `a`, at 100 %.
    const A_RIGHT: (f32, f32) = (136.0, 30.0);
    /// The middle of the left side of `b`, where `e` ends.
    const B_LEFT: (f32, f32) = (200.0, 30.0);

    fn id(name: &str) -> Id {
        Id::new(name).expect("an id")
    }

    fn ids(names: &[&str]) -> Vec<Id> {
        names.iter().map(|name| id(name)).collect()
    }

    fn moved(names: &[&str], by: [i32; 2]) -> Intent {
        Intent::Move {
            ids: ids(names),
            by,
        }
    }

    fn resized(name: &str, edges: Edges) -> Intent {
        Intent::Resize {
            ids: ids(&[name]),
            edges,
        }
    }

    fn added(kind: NodeKind, shape: Shape, at: [i32; 2], size: [i32; 2]) -> Intent {
        Intent::AddNode {
            kind,
            shape,
            at,
            size,
            from: None,
        }
    }

    fn shift() -> Input {
        Input::Modifiers(Mods {
            shift: true,
            duplicate: false,
        })
    }

    fn plain() -> Input {
        Input::Modifiers(Mods::default())
    }

    const ESCAPE: Input = Input::Key {
        key: Key::Escape,
        repeat: false,
    };

    /// The plans the rig makes draw no id.
    struct NoIds;

    impl Ids for NoIds {
        fn draw(&mut self) -> Id {
            id("unused")
        }
    }

    /// A machine with the board and the view it would have in a
    /// [`crate::BoardView`], which the rig stands in for: it keeps the
    /// viewport, the clock and the board, and plays the view's part of an
    /// edit.
    struct Rig {
        board: Board,
        scene: Scene,
        machine: Machine,
        viewport: Viewport,
        now: Instant,
        writable: bool,
        /// Where the last press went down, in board units.
        down: (f32, f32),
        made: u32,
    }

    impl Rig {
        /// The sample board, with the keyboard.
        fn new() -> Self {
            let mut rig = Self::with(BOARD);
            rig.step(Input::Focus(true));
            rig
        }

        fn with(text: &str) -> Self {
            let board = parse(text).expect("a board");
            let scene = scene::build(&board, &fixtures::palette(), &FixedMeasure);
            Self {
                board,
                scene,
                machine: Machine::default(),
                viewport: Viewport::default(),
                now: Instant::now(),
                writable: true,
                down: (0.0, 0.0),
                made: 0,
            }
        }

        fn step(&mut self, input: Input) -> Vec<Act> {
            let view = View {
                viewport: self.viewport,
                now: self.now,
                board: &self.board,
                scene: &self.scene,
                writable: self.writable,
            };
            let acts = self.machine.step(input, &view);
            for act in &acts {
                if let Act::Look(to) = act {
                    self.viewport = *to;
                }
            }
            acts
        }

        fn run(&mut self, command: Command) -> Vec<Act> {
            let view = View {
                viewport: self.viewport,
                now: self.now,
                board: &self.board,
                scene: &self.scene,
                writable: self.writable,
            };
            self.machine.command(command, &view)
        }

        fn wait(&mut self, millis: u64) {
            self.now += Duration::from_millis(millis);
        }

        /// Rest the pointer on a board point until the node there shows
        /// its connect handles.
        fn hover(&mut self, point: (f32, f32)) {
            self.to(point);
            self.wait(120);
            self.step(Input::Tick);
        }

        /// The node showing its connect handles.
        fn connect_handles(&self) -> Option<&str> {
            let view = View {
                viewport: self.viewport,
                now: self.now,
                board: &self.board,
                scene: &self.scene,
                writable: self.writable,
            };
            self.machine.handles(&view).connect.map(Id::as_str)
        }

        /// The node a dragged connector end would attach to, and the side
        /// it would be pinned to.
        fn target(&self) -> Option<(&str, Option<Side>)> {
            let connecting = self.machine.connecting().expect("a connector end dragged");
            connecting.target.map(|(id, side)| (id.as_str(), side))
        }

        /// Press `button` on a board point.
        fn press_with(&mut self, button: Button, point: (f32, f32)) -> Vec<Act> {
            self.down = point;
            let at = self.viewport.to_screen(point);
            self.step(Input::Press { button, at })
        }

        fn press(&mut self, point: (f32, f32)) -> Vec<Act> {
            self.press_with(Button::Left, point)
        }

        /// Move the pointer to a board point.
        fn to(&mut self, point: (f32, f32)) -> Vec<Act> {
            let at = self.viewport.to_screen(point);
            self.step(Input::Move { at })
        }

        /// Move the pointer by board units from where it went down.
        fn by(&mut self, dx: f32, dy: f32) -> Vec<Act> {
            self.to((self.down.0 + dx, self.down.1 + dy))
        }

        fn release(&mut self) -> Vec<Act> {
            self.step(Input::Release {
                button: Button::Left,
            })
        }

        /// A click, long enough after the last one to be no double-click.
        fn click(&mut self, point: (f32, f32)) -> Vec<Act> {
            self.wait(400);
            let mut acts = self.press(point);
            acts.extend(self.release());
            acts
        }

        fn double_click(&mut self, point: (f32, f32)) -> Vec<Act> {
            let mut acts = self.click(point);
            self.wait(100);
            acts.extend(self.press(point));
            acts.extend(self.release());
            acts
        }

        fn selected(&self) -> Vec<&str> {
            self.machine.selection().iter().map(Id::as_str).collect()
        }

        fn cursor(&self) -> Option<&str> {
            self.machine.cursor().map(Id::as_str)
        }

        fn editing(&self) -> Option<&str> {
            self.machine.editing().map(Id::as_str)
        }

        fn state(&self) -> State {
            self.machine.state()
        }

        /// A plan that puts one new, unlabelled node on the board and asks
        /// for its label, as a creation's does.
        fn creation(&mut self, kind: NodeKind, shape: Shape, at: [i32; 2], size: [i32; 2]) -> Plan {
            self.made += 1;
            let id = id(&format!("new{}", self.made));
            let node = Node {
                text: String::new(),
                kind: Some(Named::Known(kind)),
                at,
                parent: None,
                shape: Named::Known(shape),
                size,
                style: NodeStyle::default(),
                z: Some("Z".to_owned()),
                extra: Extra::new(),
            };
            Plan {
                changes: vec![Change::PutNode(id.clone(), Box::new(node))],
                select: Some(vec![id.clone()]),
                edit: Some(id),
                notice: None,
            }
        }

        /// The view's part of an edit: plan it, apply it, build the scene
        /// and hand the plan back. Moves are planned for real; a creation
        /// is stood in for, with the node where the intent says or, for
        /// keyboard building, somewhere free.
        fn apply(&mut self, intent: &Intent) -> Vec<Act> {
            let plan = match intent {
                Intent::Move { .. } | Intent::Rebind { .. } => {
                    edit::plan(&self.board, intent, &mut NoIds)
                }
                Intent::AddEdge { .. } => {
                    self.made += 1;
                    edit::plan(&self.board, intent, &mut Seeded::new(u64::from(self.made)))
                }
                Intent::AddNode {
                    kind,
                    shape,
                    at,
                    size,
                    ..
                } => self.creation(*kind, *shape, *at, *size),
                Intent::Extend { shape, .. } => {
                    let at = [1000, 1000 + 100 * self.made as i32];
                    self.creation(NodeKind::Shape, shape.unwrap_or(Shape::Rect), at, NODE_SIZE)
                }
                _ => Plan::default(),
            };
            self.board.apply(&plan.changes);
            self.scene = scene::build(&self.board, &fixtures::palette(), &FixedMeasure);
            self.machine.applied(&plan, &self.board)
        }

        /// Plays the view's part of every edit among `acts`, and gives
        /// what the machine answered.
        fn play(&mut self, acts: &[Act]) -> Vec<Act> {
            let mut answers = Vec::new();
            for act in acts {
                if let Act::Edit(intent) | Act::Continue(intent) = act {
                    answers.extend(self.apply(intent));
                }
            }
            answers
        }
    }

    /// Something a test does to a rig.
    type Play<T = ()> = fn(&mut Rig) -> T;

    /// Where a row of the transition table starts.
    #[derive(Clone, Copy, Debug)]
    enum From {
        Unfocused,
        Idle,
        /// Idle with `a` selected.
        Selected,
        Armed(Tool),
        /// Armed with `a` selected.
        ArmedSelected(Tool),
        PressedNode,
        PressedCanvas,
        /// On a resize handle of the selected `a`.
        PressedHandle,
        /// With the diamond tool armed.
        PressedArmed,
        /// The second press of a double-click on `a`.
        PressedTwiceNode,
        /// The second press of a double-click on empty canvas.
        PressedTwiceCanvas,
        Moving,
        Resizing,
        Marquee,
        Creating,
        /// On the right connect handle of `a`, shown by the pointer's rest.
        PressedConnect,
        /// On `a`, with the connector tool armed.
        PressedTool,
        /// On the `to` end of the selected `e`.
        PressedEnd,
        /// A new connector dragged from a connect handle of `a`.
        Connecting,
        /// A new connector dragged from `a` with the connector tool.
        ConnectingTool,
        /// The `to` end of `e` dragged.
        ConnectingEnd,
        /// The label of `a` open.
        Editing,
        /// The label of `a` open and the left button down in it.
        Selecting,
        /// By the middle button.
        Panning,
        /// By the middle button, with the label of `a` open.
        PanningEditing,
        /// By the left button, with the hand tool.
        PanningHand,
    }

    /// What a row does there.
    #[derive(Clone, Copy, Debug)]
    enum Then {
        Input(Input),
        Command(Command),
        /// The command, then the view's part of the edit it asks for.
        Made(Command),
        /// A left press on `a`.
        PressNode,
        /// A left press on `b`.
        PressOther,
        /// A press on empty canvas.
        Press(Button),
        /// Space, then a left press on empty canvas.
        SpacePress,
        /// The pointer moves 40 units each way from the press.
        Far,
        /// The pointer moves 2 pixels from the press.
        Near,
        Release(Button),
        /// A left release, then the view's part of the edit it asks for.
        ReleaseMade,
        Settle,
    }

    const DIAMOND: Tool = Tool::Shape(Shape::Diamond);

    fn enter(from: From) -> (Rig, State) {
        let mut rig = Rig::new();
        let state = match from {
            From::Unfocused => {
                rig.step(Input::Focus(false));
                State::Unfocused
            }
            From::Idle => State::Idle,
            From::Selected => {
                rig.click(A);
                State::Idle
            }
            From::Armed(tool) => {
                rig.run(Command::Tool(tool));
                State::Armed
            }
            From::ArmedSelected(tool) => {
                rig.click(A);
                rig.run(Command::Tool(tool));
                State::Armed
            }
            From::PressedNode => {
                rig.press(A);
                State::Pressed
            }
            From::PressedCanvas => {
                rig.press(CANVAS);
                State::Pressed
            }
            From::PressedHandle => {
                rig.click(A);
                rig.wait(400);
                rig.press(A_CORNER);
                State::Pressed
            }
            From::PressedArmed => {
                rig.run(Command::Tool(DIAMOND));
                rig.press(CANVAS);
                State::Pressed
            }
            From::PressedTwiceNode => {
                rig.click(A);
                rig.wait(100);
                rig.press(A);
                State::Pressed
            }
            From::PressedTwiceCanvas => {
                rig.click(CANVAS);
                rig.wait(100);
                rig.press(CANVAS);
                State::Pressed
            }
            From::Moving => {
                rig.press(A);
                rig.by(40.0, 40.0);
                State::Moving
            }
            From::Resizing => {
                rig.click(A);
                rig.wait(400);
                rig.press(A_CORNER);
                rig.by(40.0, 40.0);
                State::Resizing
            }
            From::Marquee => {
                rig.press(CANVAS);
                rig.by(40.0, 40.0);
                State::Marquee
            }
            From::Creating => {
                rig.run(Command::Tool(DIAMOND));
                rig.press(CANVAS);
                rig.by(40.0, 40.0);
                State::Creating
            }
            From::PressedConnect | From::Connecting => {
                rig.hover(A);
                rig.press(A_RIGHT);
                if matches!(from, From::Connecting) {
                    rig.by(40.0, 40.0);
                    State::Connecting
                } else {
                    State::Pressed
                }
            }
            From::PressedTool | From::ConnectingTool => {
                rig.run(Command::Tool(Tool::Connector));
                rig.press(A);
                if matches!(from, From::ConnectingTool) {
                    rig.by(0.0, 100.0);
                    State::Connecting
                } else {
                    State::Pressed
                }
            }
            From::PressedEnd | From::ConnectingEnd => {
                rig.click(E);
                rig.wait(400);
                rig.press(B_LEFT);
                if matches!(from, From::ConnectingEnd) {
                    rig.by(40.0, 40.0);
                    State::Connecting
                } else {
                    State::Pressed
                }
            }
            From::Editing => {
                rig.click(A);
                rig.run(Command::EditLabel);
                State::Editing
            }
            From::Selecting => {
                rig.click(A);
                rig.run(Command::EditLabel);
                rig.press(A);
                State::Editing
            }
            From::Panning => {
                rig.press_with(Button::Middle, CANVAS);
                State::Panning
            }
            From::PanningEditing => {
                rig.click(A);
                rig.run(Command::EditLabel);
                rig.press_with(Button::Middle, CANVAS);
                State::Panning
            }
            From::PanningHand => {
                rig.run(Command::Tool(Tool::Hand));
                rig.press(CANVAS);
                State::Panning
            }
        };
        (rig, state)
    }

    fn then(rig: &mut Rig, then: Then) {
        match then {
            Then::Input(input) => {
                rig.step(input);
            }
            Then::Command(command) => {
                rig.run(command);
            }
            Then::Made(command) => {
                let acts = rig.run(command);
                rig.play(&acts);
            }
            Then::PressNode => {
                rig.press(A);
            }
            Then::PressOther => {
                rig.press(B);
            }
            Then::Press(button) => {
                rig.press_with(button, CANVAS);
            }
            Then::SpacePress => {
                rig.step(Input::Space { held: true });
                rig.press(CANVAS);
            }
            Then::Far => {
                rig.by(40.0, 40.0);
            }
            Then::Near => {
                rig.by(2.0, 0.0);
            }
            Then::Release(button) => {
                rig.step(Input::Release { button });
            }
            Then::ReleaseMade => {
                let acts = rig.release();
                rig.play(&acts);
            }
            Then::Settle => {
                rig.machine.settle();
            }
        }
    }

    /// Every transition of the plan's §5.2 table but keyboard Connecting's,
    /// one row each: where it starts, what happens, and the state it
    /// leaves.
    #[test]
    fn every_transition_leaves_the_state_the_table_says() {
        use Button::{Left, Middle, Right};
        use Command as C;
        use Then as T;
        let focus = |on| T::Input(Input::Focus(on));
        let escape = T::Input(ESCAPE);
        let cancel = T::Input(Input::Cancel);
        let scroll = T::Input(wheel(lines(-1.0), false));
        let hand = T::Command(C::Tool(Tool::Hand));
        let mut rows = vec![
            // Unfocused: a click gives the board the keyboard and is a
            // click like any other; the wheel does not.
            (From::Unfocused, focus(true), State::Idle),
            (From::Unfocused, T::Press(Left), State::Pressed),
            (From::Unfocused, T::PressNode, State::Pressed),
            (From::Unfocused, T::Press(Middle), State::Panning),
            (From::Unfocused, scroll, State::Unfocused),
            (From::Unfocused, escape, State::Unfocused),
            // Idle.
            (From::Idle, hand, State::Armed),
            (From::Idle, T::Command(C::Tool(DIAMOND)), State::Armed),
            (From::Idle, T::Command(C::Tool(Tool::Text)), State::Armed),
            (
                From::Idle,
                T::Command(C::Tool(Tool::Connector)),
                State::Armed,
            ),
            (From::Idle, T::Command(C::Tool(Tool::Select)), State::Idle),
            (From::Idle, T::Command(C::EditLabel), State::Idle),
            (From::Selected, T::Command(C::EditLabel), State::Editing),
            (From::Selected, T::Command(C::ConnectNext), State::Idle),
            (From::Selected, T::Made(C::ConnectNext), State::Editing),
            (
                From::Selected,
                T::Made(C::Connect(Dir::Right)),
                State::Editing,
            ),
            (From::Idle, T::Made(C::ConnectNext), State::Idle),
            (From::Idle, T::PressNode, State::Pressed),
            (From::Idle, T::Press(Left), State::Pressed),
            (From::Idle, T::Press(Middle), State::Panning),
            (From::Idle, T::Press(Right), State::Idle),
            (From::Idle, T::SpacePress, State::Panning),
            (From::Idle, focus(false), State::Unfocused),
            (From::Idle, escape, State::Idle),
            (From::Idle, cancel, State::Idle),
            (From::Idle, T::Settle, State::Idle),
            (From::Idle, T::Input(Input::Tick), State::Idle),
            // Armed: the tool is spent by what it makes, and put away by
            // Escape, by Select and by losing the keyboard.
            (From::Armed(DIAMOND), escape, State::Idle),
            (
                From::Armed(DIAMOND),
                T::Command(C::Tool(Tool::Select)),
                State::Idle,
            ),
            (
                From::Armed(DIAMOND),
                T::Command(C::Tool(Tool::Text)),
                State::Armed,
            ),
            (From::Armed(DIAMOND), T::Press(Left), State::Pressed),
            (From::Armed(DIAMOND), T::PressNode, State::Pressed),
            (From::Armed(DIAMOND), T::Press(Middle), State::Panning),
            (From::Armed(DIAMOND), T::SpacePress, State::Panning),
            (From::Armed(DIAMOND), focus(false), State::Unfocused),
            (From::Armed(DIAMOND), T::Settle, State::Armed),
            (
                From::ArmedSelected(DIAMOND),
                T::Command(C::ConnectNext),
                State::Idle,
            ),
            (
                From::ArmedSelected(DIAMOND),
                T::Made(C::ConnectNext),
                State::Editing,
            ),
            (
                From::ArmedSelected(DIAMOND),
                T::Made(C::Connect(Dir::Down)),
                State::Editing,
            ),
            (
                From::ArmedSelected(DIAMOND),
                T::Command(C::EditLabel),
                State::Editing,
            ),
            (From::Armed(Tool::Hand), T::Press(Left), State::Panning),
            // The connector tool takes a press on a node and no other.
            (From::Armed(Tool::Connector), T::Press(Left), State::Armed),
            (From::Armed(Tool::Connector), T::PressNode, State::Pressed),
            // Pressed: four pixels make it a drag, and what was pressed
            // says which.
            (From::PressedNode, T::Near, State::Pressed),
            (From::PressedNode, T::Far, State::Moving),
            (From::PressedHandle, T::Far, State::Resizing),
            (From::PressedArmed, T::Far, State::Creating),
            (From::PressedCanvas, T::Far, State::Marquee),
            (From::PressedTwiceNode, T::Far, State::Moving),
            (From::PressedConnect, T::Near, State::Pressed),
            (From::PressedConnect, T::Far, State::Connecting),
            (From::PressedTool, T::Far, State::Connecting),
            (From::PressedEnd, T::Far, State::Connecting),
            // A connect handle clicked is `board.connect.<side>`; a node
            // clicked with the connector tool is nothing.
            (From::PressedConnect, T::Release(Left), State::Idle),
            (From::PressedConnect, T::ReleaseMade, State::Editing),
            (From::PressedConnect, escape, State::Idle),
            (From::PressedTool, T::Release(Left), State::Armed),
            (From::PressedTool, escape, State::Armed),
            (From::PressedEnd, T::Release(Left), State::Idle),
            (From::PressedNode, T::Release(Left), State::Idle),
            (From::PressedCanvas, T::Release(Left), State::Idle),
            (From::PressedHandle, T::Release(Left), State::Idle),
            (From::PressedArmed, T::Release(Left), State::Idle),
            (From::PressedArmed, T::ReleaseMade, State::Editing),
            (From::PressedTwiceNode, T::Release(Left), State::Editing),
            (From::PressedTwiceCanvas, T::Release(Left), State::Idle),
            (From::PressedTwiceCanvas, T::ReleaseMade, State::Editing),
            (From::PressedNode, T::Release(Right), State::Pressed),
            (From::PressedNode, escape, State::Idle),
            (From::PressedArmed, escape, State::Armed),
            (From::PressedNode, cancel, State::Idle),
            (From::PressedNode, focus(false), State::Unfocused),
            (From::PressedNode, T::Settle, State::Idle),
            (From::PressedNode, hand, State::Pressed),
            (From::PressedNode, T::Press(Middle), State::Panning),
            // Creating leaves its tool armed unless the shape is made.
            (From::Creating, T::Far, State::Creating),
            (From::Creating, T::Release(Left), State::Idle),
            (From::Creating, T::ReleaseMade, State::Editing),
            (From::Creating, T::Release(Right), State::Creating),
            (From::Creating, escape, State::Armed),
            (From::Creating, cancel, State::Armed),
            (From::Creating, T::Settle, State::Armed),
            (From::Creating, focus(false), State::Unfocused),
            (From::Creating, hand, State::Creating),
            // Connecting by pointer: released on empty canvas it makes a
            // node and opens its label; the tool is spent only by what it
            // makes; an end of a connector goes back to Idle either way.
            (From::Connecting, T::Far, State::Connecting),
            (From::Connecting, T::Release(Left), State::Idle),
            (From::Connecting, T::ReleaseMade, State::Editing),
            (From::Connecting, T::Release(Right), State::Connecting),
            (From::Connecting, escape, State::Idle),
            (From::Connecting, cancel, State::Idle),
            (From::Connecting, T::Settle, State::Idle),
            (From::Connecting, focus(false), State::Unfocused),
            (From::Connecting, hand, State::Connecting),
            (
                From::Connecting,
                T::Command(C::EditLabel),
                State::Connecting,
            ),
            (From::Connecting, T::Press(Left), State::Pressed),
            (From::ConnectingTool, T::Release(Left), State::Idle),
            (From::ConnectingTool, T::ReleaseMade, State::Editing),
            (From::ConnectingTool, escape, State::Armed),
            (From::ConnectingTool, cancel, State::Armed),
            (From::ConnectingTool, T::Settle, State::Armed),
            (From::ConnectingTool, focus(false), State::Unfocused),
            (From::ConnectingEnd, T::Far, State::Connecting),
            (From::ConnectingEnd, T::Release(Left), State::Idle),
            (From::ConnectingEnd, escape, State::Idle),
            (From::ConnectingEnd, T::Settle, State::Idle),
            // Editing: every way out but the label's own press commits.
            (From::Editing, T::Command(C::CommitLabel), State::Idle),
            (From::Editing, escape, State::Idle),
            (From::Editing, T::Command(C::ConnectNext), State::Idle),
            (From::Editing, T::Made(C::ConnectNext), State::Editing),
            (From::Editing, hand, State::Armed),
            (From::Editing, T::PressNode, State::Editing),
            (From::Editing, T::PressOther, State::Pressed),
            (From::Editing, T::Press(Left), State::Pressed),
            (From::Editing, T::Press(Right), State::Idle),
            (From::Editing, T::Press(Middle), State::Panning),
            (From::Editing, T::SpacePress, State::Pressed),
            (From::Editing, scroll, State::Editing),
            (From::Editing, cancel, State::Editing),
            (From::Editing, T::Settle, State::Idle),
            (From::Editing, focus(false), State::Unfocused),
            // A drag in the label selects its text, and ends without
            // closing it unless the board is settled or loses the keyboard.
            (From::Selecting, T::Far, State::Editing),
            (From::Selecting, T::Release(Left), State::Editing),
            (From::Selecting, T::Release(Right), State::Editing),
            (From::Selecting, escape, State::Editing),
            (From::Selecting, cancel, State::Editing),
            (From::Selecting, hand, State::Editing),
            (From::Selecting, T::Command(C::CommitLabel), State::Editing),
            (From::Selecting, T::PressOther, State::Pressed),
            (From::Selecting, T::Press(Middle), State::Panning),
            (From::Selecting, T::Settle, State::Idle),
            (From::Selecting, focus(false), State::Unfocused),
            // Panning ends where it began.
            (From::Panning, T::Far, State::Panning),
            (From::Panning, T::Release(Middle), State::Idle),
            (From::Panning, T::Release(Left), State::Panning),
            (From::Panning, escape, State::Idle),
            (From::Panning, cancel, State::Idle),
            (From::Panning, T::Settle, State::Idle),
            (From::Panning, focus(false), State::Unfocused),
            (From::Panning, hand, State::Panning),
            (From::PanningEditing, T::Release(Middle), State::Editing),
            (From::PanningEditing, escape, State::Editing),
            (From::PanningEditing, T::Settle, State::Idle),
            (From::PanningHand, T::Release(Left), State::Idle),
            (From::PanningHand, T::Release(Middle), State::Panning),
        ];
        // Moving, Resizing and Marquee: release ends them, anything that
        // takes the board away cancels them, and no command gets in.
        for (gesture, state) in [
            (From::Moving, State::Moving),
            (From::Resizing, State::Resizing),
            (From::Marquee, State::Marquee),
        ] {
            rows.extend([
                (gesture, T::Far, state),
                (gesture, T::Release(Left), State::Idle),
                (gesture, T::Release(Right), state),
                (gesture, escape, State::Idle),
                (gesture, cancel, State::Idle),
                (gesture, T::Settle, State::Idle),
                (gesture, focus(false), State::Unfocused),
                (gesture, hand, state),
                (gesture, T::Command(C::EditLabel), state),
                (gesture, T::Press(Left), State::Pressed),
            ]);
        }
        for (from, with, to) in rows {
            let (mut rig, state) = enter(from);
            assert_eq!(rig.state(), state, "entering {from:?}");
            then(&mut rig, with);
            assert_eq!(rig.state(), to, "{from:?}, then {with:?}");
        }
    }

    #[test]
    fn every_state_has_its_own_name() {
        let states = [
            State::Unfocused,
            State::Idle,
            State::Armed,
            State::Pressed,
            State::Moving,
            State::Marquee,
            State::Resizing,
            State::Creating,
            State::Connecting,
            State::Editing,
            State::Panning,
        ];
        let names: BTreeSet<&str> = states.iter().map(|state| state.name()).collect();
        assert_eq!(names.len(), states.len());
        assert!(names
            .iter()
            .all(|name| name.bytes().all(|byte| byte.is_ascii_lowercase())));
    }

    #[test]
    fn a_click_selects_and_shift_toggles() {
        let mut rig = Rig::new();
        assert!(rig.click(A).is_empty());
        assert_eq!((rig.selected(), rig.cursor()), (vec!["a"], Some("a")));
        rig.click(B);
        assert_eq!((rig.selected(), rig.cursor()), (vec!["b"], Some("b")));
        // A connector is selected like a node.
        rig.click(E);
        assert_eq!((rig.selected(), rig.cursor()), (vec!["e"], Some("e")));
        rig.click(B);

        rig.step(shift());
        rig.click(A);
        assert_eq!((rig.selected(), rig.cursor()), (vec!["a", "b"], Some("a")));
        rig.click(FREE);
        assert_eq!(rig.selected(), ["a", "b", "free"]);
        assert_eq!(rig.cursor(), Some("free"));
        // Toggled off, an element leaves the selection, and the cursor
        // stays on the element last selected.
        rig.click(A);
        assert_eq!(
            (rig.selected(), rig.cursor()),
            (vec!["b", "free"], Some("free"))
        );
        rig.click(FREE);
        assert_eq!((rig.selected(), rig.cursor()), (vec!["b"], Some("free")));
        // Shift on empty canvas keeps what is selected.
        rig.click(CANVAS);
        assert_eq!(rig.selected(), ["b"]);

        // Empty canvas clears the selection and keeps the cursor.
        rig.step(plain());
        assert!(rig.click(CANVAS).is_empty());
        assert_eq!((rig.selected(), rig.cursor()), (vec![], Some("free")));
        assert_eq!(rig.state(), State::Idle);
    }

    #[test]
    fn a_click_in_a_selection_of_several_selects_that_one() {
        let mut rig = Rig::new();
        rig.run(Command::SelectAll);
        assert_eq!(rig.selected().len(), 7);
        rig.press(B);
        // Not before the release: a drag would move them all.
        assert_eq!(rig.selected().len(), 7);
        rig.release();
        assert_eq!((rig.selected(), rig.cursor()), (vec!["b"], Some("b")));
    }

    #[test]
    fn a_right_click_selects_what_is_not_selected() {
        let mut rig = Rig::new();
        rig.click(A);
        rig.step(shift());
        rig.click(B);
        rig.step(plain());
        // On a selected element the selection stays.
        assert!(rig.press_with(Button::Right, A).is_empty());
        assert_eq!(rig.selected(), ["a", "b"]);
        assert_eq!(rig.state(), State::Idle);
        let release = Input::Release {
            button: Button::Right,
        };
        assert!(rig.step(release).is_empty());
        // On another, that one alone.
        rig.press_with(Button::Right, OFF);
        assert_eq!((rig.selected(), rig.cursor()), (vec!["off"], Some("off")));
        rig.press_with(Button::Right, E);
        assert_eq!((rig.selected(), rig.cursor()), (vec!["e"], Some("e")));
        // On empty canvas, nothing; the cursor stays.
        rig.press_with(Button::Right, CANVAS);
        assert_eq!((rig.selected(), rig.cursor()), (vec![], Some("e")));
        // With a tool armed it still selects and makes nothing.
        rig.run(Command::Tool(DIAMOND));
        assert!(rig.press_with(Button::Right, A).is_empty());
        assert_eq!(rig.selected(), ["a"]);
        assert_eq!(rig.machine.tool(), DIAMOND);
    }

    #[test]
    fn a_marquee_selects_what_is_entirely_inside_it() {
        let mut rig = Rig::new();
        rig.click(OFF);
        rig.press((-20.0, -20.0));
        assert!(rig.to((340.0, 80.0)).is_empty());
        assert_eq!(rig.state(), State::Marquee);
        // Live, while it is dragged.
        assert_eq!(rig.selected(), ["a", "b", "e"]);
        assert_eq!(
            rig.machine.marquee(),
            Some(Rect::new(-20.0, -20.0, 360.0, 100.0))
        );
        // Dragged back, it lets go again.
        rig.to((130.0, 80.0));
        assert_eq!(rig.selected(), ["a"]);
        // And up and to the left of the press it is still a box.
        rig.to((-40.0, -30.0));
        assert_eq!(
            rig.machine.marquee(),
            Some(Rect::new(-40.0, -30.0, 20.0, 10.0))
        );
        assert!(rig.selected().is_empty());
        rig.to((340.0, 80.0));
        assert!(rig.release().is_empty());
        assert_eq!(rig.state(), State::Idle);
        assert_eq!(rig.machine.marquee(), None);
        // The cursor goes to the selected node nearest the top left.
        assert_eq!(
            (rig.selected(), rig.cursor()),
            (vec!["a", "b", "e"], Some("a"))
        );
    }

    #[test]
    fn shift_makes_a_marquee_add() {
        let mut rig = Rig::new();
        rig.click(OFF);
        rig.step(shift());
        rig.press((-20.0, -20.0));
        rig.to((130.0, 80.0));
        assert_eq!(rig.selected(), ["a", "off"]);
        rig.release();
        // The cursor stays where it was, on a selected element.
        assert_eq!(
            (rig.selected(), rig.cursor()),
            (vec!["a", "off"], Some("off"))
        );
    }

    #[test]
    fn a_marquee_of_connectors_puts_the_cursor_on_the_first() {
        let mut rig = Rig::new();
        rig.press((-20.0, 380.0));
        rig.to((220.0, 420.0));
        rig.release();
        assert_eq!((rig.selected(), rig.cursor()), (vec!["free"], Some("free")));
        // One that takes nothing leaves the cursor alone.
        rig.press(CANVAS);
        rig.by(10.0, 10.0);
        rig.release();
        assert_eq!((rig.selected(), rig.cursor()), (vec![], Some("free")));
    }

    #[test]
    fn a_cancelled_marquee_puts_the_selection_back() {
        for cancel in [ESCAPE, Input::Cancel, Input::Focus(false)] {
            let mut rig = Rig::new();
            rig.click(OFF);
            rig.press((-20.0, -20.0));
            rig.to((340.0, 80.0));
            assert_eq!(rig.selected(), ["a", "b", "e"]);
            assert!(rig.step(cancel).is_empty());
            assert_eq!((rig.selected(), rig.cursor()), (vec!["off"], Some("off")));
            assert_eq!(rig.machine.marquee(), None);
            // The release that follows is nobody's.
            assert!(rig.release().is_empty());
            assert_eq!(rig.selected(), ["off"]);
        }
    }

    #[test]
    fn a_drag_previews_the_move_and_commits_it_once_on_release() {
        let mut rig = Rig::new();
        assert!(rig.press(A).is_empty());
        // The press alone selects nothing.
        assert!(rig.selected().is_empty());
        let preview = rig.by(33.0, 8.0);
        assert_eq!(preview, [Act::Preview(Some(moved(&["a"], [40, 0])))]);
        // The drag selected what it pressed.
        assert_eq!((rig.selected(), rig.cursor()), (vec!["a"], Some("a")));
        assert_eq!(rig.state(), State::Moving);
        // A move within the same cell says nothing.
        assert!(rig.by(38.0, 9.0).is_empty());
        assert_eq!(
            rig.by(52.0, 12.0),
            [Act::Preview(Some(moved(&["a"], [60, 20])))]
        );
        let release = rig.release();
        assert_eq!(release, [Act::Edit(moved(&["a"], [60, 20]))]);
        assert_eq!(rig.state(), State::Idle);
        // Nothing moved until the view applied the edit.
        assert_eq!(rig.board.nodes["a"].at, [0, 0]);
        assert!(rig.play(&release).is_empty());
        assert_eq!(rig.board.nodes["a"].at, [60, 20]);
        assert_eq!(rig.selected(), ["a"]);
        // A later move of the pointer is no drag.
        assert!(rig.by(100.0, 100.0).is_empty());
    }

    #[test]
    fn a_press_moved_under_four_pixels_is_a_click() {
        for zoom in [0.25, 1.0, 4.0] {
            let mut rig = Rig::new();
            rig.viewport = Viewport {
                x: -30.0,
                y: 15.0,
                zoom,
            };
            rig.press(A);
            assert!(rig.by(3.9 / zoom, 0.0).is_empty());
            assert_eq!(rig.state(), State::Pressed, "{zoom}");
            assert!(rig.release().is_empty());
            assert_eq!(rig.selected(), ["a"]);
            // Four pixels are a drag, at any zoom.
            rig.wait(400);
            rig.press(B);
            rig.by(0.0, 4.1 / zoom);
            assert_eq!(rig.state(), State::Moving, "{zoom}");
            assert_eq!(rig.selected(), ["b"]);
        }
    }

    #[test]
    fn a_drag_back_to_the_start_changes_nothing() {
        let mut rig = Rig::new();
        rig.press(A);
        assert_eq!(
            rig.by(30.0, 0.0),
            [Act::Preview(Some(moved(&["a"], [40, 0])))]
        );
        assert_eq!(rig.by(5.0, 5.0), [Act::Preview(None)]);
        assert!(rig.release().is_empty());
        assert_eq!(rig.state(), State::Idle);
    }

    #[test]
    fn the_pressed_node_lands_on_the_grid_and_the_rest_keep_their_offsets() {
        // `off` is at 7,213: the nearest grid lines to 9,219 are 0 and 220.
        let mut rig = Rig::new();
        rig.press(OFF);
        assert_eq!(
            rig.by(2.0, 6.0),
            [Act::Preview(Some(moved(&["off"], [-7, 7])))]
        );
        rig.step(Input::Cancel);

        // With `a` selected too, pressing `off` moves both by its delta,
        // which takes `a` off the grid; pressing `a` moves both by cells.
        let mut rig = Rig::new();
        rig.click(A);
        rig.step(shift());
        rig.click(OFF);
        rig.step(plain());
        rig.wait(400);
        rig.press(OFF);
        assert_eq!(
            rig.by(2.0, 6.0),
            [Act::Preview(Some(moved(&["a", "off"], [-7, 7])))]
        );
        rig.step(Input::Cancel);
        rig.press(A);
        assert_eq!(
            rig.by(2.0, 26.0),
            [Act::Preview(Some(moved(&["a", "off"], [0, 20])))]
        );
        assert_eq!(rig.selected(), ["a", "off"]);
    }

    #[test]
    fn a_child_s_drag_is_measured_from_where_it_is_drawn() {
        // `kid` is stored at 20,20 and drawn at 420,220.
        let mut rig = Rig::new();
        rig.board.nodes.get_mut("kid").expect("the node").at = [27, 20];
        rig.scene = scene::build(&rig.board, &fixtures::palette(), &FixedMeasure);
        rig.press(KID);
        // 427 + 9 is nearest 440.
        assert_eq!(
            rig.by(9.0, 4.0),
            [Act::Preview(Some(moved(&["kid"], [13, 0])))]
        );
    }

    #[test]
    fn a_dragged_connector_moves_by_whole_cells() {
        let mut rig = Rig::new();
        rig.press(FREE);
        assert_eq!(
            rig.by(33.0, -8.0),
            [Act::Preview(Some(moved(&["free"], [40, 0])))]
        );
        assert_eq!(rig.selected(), ["free"]);
        assert_eq!(rig.release(), [Act::Edit(moved(&["free"], [40, 0]))]);
    }

    #[test]
    fn a_shift_drag_adds_what_it_pressed_to_the_selection() {
        let mut rig = Rig::new();
        rig.click(A);
        rig.step(shift());
        rig.press(B);
        assert_eq!(
            rig.by(40.0, 0.0),
            [Act::Preview(Some(moved(&["a", "b"], [40, 0])))]
        );
        assert_eq!((rig.selected(), rig.cursor()), (vec!["a", "b"], Some("b")));
    }

    #[test]
    fn a_gesture_is_the_same_at_any_zoom_and_pan() {
        for viewport in [
            Viewport {
                x: -333.0,
                y: 91.5,
                zoom: 0.25,
            },
            Viewport {
                x: 40.0,
                y: -17.0,
                zoom: 4.0,
            },
        ] {
            let mut rig = Rig::new();
            rig.viewport = viewport;
            rig.press(A);
            assert_eq!(
                rig.by(52.0, 28.0),
                [Act::Preview(Some(moved(&["a"], [60, 20])))],
                "{viewport:?}"
            );
            assert_eq!(rig.release(), [Act::Edit(moved(&["a"], [60, 20]))]);
        }
    }

    #[test]
    fn a_wheel_pan_in_mid_drag_carries_the_drag() {
        let mut rig = Rig::new();
        rig.press(A);
        let at = rig.viewport.to_screen((A.0 + 40.0, A.1));
        assert_eq!(
            rig.step(Input::Move { at }),
            [Act::Preview(Some(moved(&["a"], [40, 0])))]
        );
        // One line down: the board point under the pointer is now 40
        // units lower, though the pointer has not moved.
        let scroll = Input::Wheel {
            delta: lines(-1.0),
            at,
            zoom: false,
            sideways: false,
            scale_factor: 1.0,
        };
        assert!(matches!(rig.step(scroll)[..], [Act::Look(_)]));
        assert_eq!(rig.state(), State::Moving);
        assert_eq!(
            rig.step(Input::Move { at }),
            [Act::Preview(Some(moved(&["a"], [40, 40])))]
        );
    }

    #[test]
    fn a_drag_with_the_duplicate_key_moves_copies() {
        let copies = |by| Intent::Duplicate {
            ids: ids(&["a"]),
            by,
        };
        let held = Input::Modifiers(Mods {
            shift: false,
            duplicate: true,
        });
        let mut rig = Rig::new();
        rig.step(held);
        rig.press(A);
        assert_eq!(rig.by(40.0, 20.0), [Act::Preview(Some(copies([40, 20])))]);
        // Let go of the key mid-drag: what counts is the threshold.
        rig.step(plain());
        assert_eq!(rig.by(60.0, 20.0), [Act::Preview(Some(copies([60, 20])))]);
        assert_eq!(rig.release(), [Act::Edit(copies([60, 20]))]);

        // And pressed only after the threshold, it is a plain move.
        let mut rig = Rig::new();
        rig.press(A);
        rig.by(40.0, 20.0);
        rig.step(held);
        assert_eq!(
            rig.by(60.0, 20.0),
            [Act::Preview(Some(moved(&["a"], [60, 20])))]
        );
        // Copies dropped where the originals are would only hide them.
        let mut rig = Rig::new();
        rig.step(held);
        rig.press(A);
        assert!(rig.by(5.0, 5.0).is_empty());
        assert!(rig.release().is_empty());
    }

    #[test]
    fn a_handle_drags_its_edges_to_the_grid() {
        let edges = |left, top, right, bottom| Edges {
            left,
            top,
            right,
            bottom,
        };
        // Where each handle of `a` (0,0, 120 by 60) is, how far it is
        // dragged, and which edges move by how much.
        let cases = [
            ((120.0, 60.0), (33.0, 12.0), edges(0, 0, 40, 20)),
            ((0.0, 0.0), (25.0, 26.0), edges(20, 20, 0, 0)),
            ((120.0, 0.0), (-25.0, -33.0), edges(0, -40, -20, 0)),
            ((0.0, 60.0), (-12.0, 50.0), edges(-20, 0, 0, 60)),
            ((60.0, 0.0), (50.0, -33.0), edges(0, -40, 0, 0)),
            ((60.0, 60.0), (50.0, 33.0), edges(0, 0, 0, 40)),
            ((0.0, 30.0), (-12.0, 50.0), edges(-20, 0, 0, 0)),
            ((120.0, 30.0), (12.0, 50.0), edges(0, 0, 20, 0)),
        ];
        for (handle, (dx, dy), want) in cases {
            let mut rig = Rig::new();
            rig.click(A);
            rig.wait(400);
            rig.press(handle);
            let preview = rig.by(dx, dy);
            assert_eq!(
                preview,
                [Act::Preview(Some(resized("a", want)))],
                "{handle:?}"
            );
            assert_eq!(rig.state(), State::Resizing);
            // The same cell again says nothing.
            assert!(rig.by(dx + 1.0, dy + 1.0).is_empty(), "{handle:?}");
            assert_eq!(rig.release(), [Act::Edit(resized("a", want))]);
            assert_eq!(rig.selected(), ["a"]);
        }
    }

    #[test]
    fn a_resize_takes_an_edge_off_the_grid_onto_it() {
        // `off` is at 7,213, 120 by 60: its right edge is 127 and its
        // bottom 273.
        let mut rig = Rig::new();
        rig.click(OFF);
        rig.wait(400);
        rig.press((127.0, 273.0));
        let want = Edges {
            right: 13,
            bottom: 7,
            ..Edges::default()
        };
        assert_eq!(rig.by(5.0, 5.0), [Act::Preview(Some(resized("off", want)))]);
        // Back where the edges were, nothing is previewed or committed.
        rig.wait(400);
        let mut rig = Rig::new();
        rig.click(A);
        rig.wait(400);
        rig.press(A_CORNER);
        rig.by(30.0, 30.0);
        assert_eq!(rig.by(4.0, 4.0), [Act::Preview(None)]);
        assert!(rig.release().is_empty());
    }

    #[test]
    fn handles_are_pressed_only_on_a_board_that_shows_them() {
        // Two selected: no handle, so the corner of `a` is `a`.
        let mut rig = Rig::new();
        rig.click(A);
        rig.step(shift());
        rig.click(B);
        rig.step(plain());
        rig.wait(400);
        rig.press((119.0, 59.0));
        rig.by(40.0, 40.0);
        assert_eq!(rig.state(), State::Moving);

        // Without the keyboard: the press takes it, and is a press on the
        // node, not on a handle nobody saw.
        let mut rig = Rig::new();
        rig.click(A);
        rig.step(Input::Focus(false));
        assert_eq!(rig.selected(), ["a"]);
        rig.wait(400);
        rig.press((119.0, 59.0));
        assert!(rig.machine.focused());
        rig.by(40.0, 40.0);
        assert_eq!(rig.state(), State::Moving);
    }

    #[test]
    fn the_handles_on_screen_follow_the_selection() {
        let mut rig = Rig::new();
        let shown = |rig: &Rig| {
            let view = View {
                viewport: rig.viewport,
                now: rig.now,
                board: &rig.board,
                scene: &rig.scene,
                writable: rig.writable,
            };
            let shown = rig.machine.handles(&view);
            (
                shown.resize.map(Id::to_string),
                shown.connect.map(Id::to_string),
                shown.ends.map(Id::to_string),
            )
        };
        assert_eq!(shown(&rig), (None, None, None));
        rig.click(A);
        assert_eq!(shown(&rig), (Some("a".to_owned()), None, None));
        rig.click(E);
        assert_eq!(shown(&rig), (None, None, Some("e".to_owned())));
        rig.step(shift());
        rig.click(A);
        assert_eq!(shown(&rig), (None, None, None));
        rig.step(plain());
        rig.click(A);
        // None while its label is open, on a board that cannot be edited,
        // or without the keyboard.
        rig.run(Command::EditLabel);
        assert_eq!(shown(&rig), (None, None, None));
        rig.run(Command::CommitLabel);
        assert_eq!(shown(&rig), (Some("a".to_owned()), None, None));
        rig.writable = false;
        assert_eq!(shown(&rig), (None, None, None));
        rig.writable = true;
        rig.step(Input::Focus(false));
        assert_eq!(shown(&rig), (None, None, None));
    }

    #[test]
    fn a_node_shows_its_connect_handles_once_the_pointer_has_rested_on_it() {
        let mut rig = Rig::new();
        assert!(rig.to(A).is_empty());
        assert!(rig.machine.hovering());
        assert_eq!(rig.connect_handles(), None);
        assert_eq!(rig.machine.deadline(), Some(rig.now + HOVER_DELAY));
        rig.wait(119);
        rig.step(Input::Tick);
        assert_eq!(rig.connect_handles(), None);
        rig.wait(1);
        assert!(rig.step(Input::Tick).is_empty());
        assert_eq!(rig.connect_handles(), Some("a"));
        assert_eq!(rig.machine.deadline(), None);
        // Moving about on the node keeps them, and nothing is selected.
        rig.to((100.0, 50.0));
        assert_eq!(rig.connect_handles(), Some("a"));
        assert!(rig.selected().is_empty());
        assert_eq!(rig.state(), State::Idle);

        // Another node starts its own rest, which a later move ends as a
        // tick does.
        rig.to(B);
        assert_eq!(rig.connect_handles(), None);
        assert_eq!(rig.machine.deadline(), Some(rig.now + HOVER_DELAY));
        rig.wait(120);
        rig.to(B);
        assert_eq!(rig.connect_handles(), Some("b"));
        // A selected node shows them beside its resize handles.
        rig.click(B);
        assert_eq!(rig.connect_handles(), Some("b"));

        // Gone when the pointer leaves, when the view moves under it, and
        // with any command.
        rig.step(Input::Leave);
        assert!(!rig.machine.hovering());
        rig.hover(A);
        rig.step(wheel(lines(-1.0), false));
        assert_eq!(rig.connect_handles(), None);
        rig.hover(A);
        assert_eq!(rig.connect_handles(), Some("a"));
        rig.run(Command::SelectNone);
        assert_eq!(rig.connect_handles(), None);
    }

    #[test]
    fn the_pointer_rests_only_on_a_board_that_is_idle_or_has_a_shape_armed() {
        // Without the keyboard, on a board that cannot be edited, and with
        // a label open, the pointer passing over is nothing.
        let mut rig = Rig::new();
        rig.step(Input::Focus(false));
        rig.hover(A);
        assert!(!rig.machine.hovering());
        let mut rig = Rig::new();
        rig.writable = false;
        rig.hover(A);
        assert!(!rig.machine.hovering());
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(Command::EditLabel);
        rig.hover(B);
        assert!(!rig.machine.hovering());

        for tool in [Tool::Text, Tool::Connector, Tool::Hand] {
            let mut rig = Rig::new();
            rig.run(Command::Tool(tool));
            rig.hover(A);
            assert_eq!(rig.connect_handles(), None, "{tool:?}");
        }
        let mut rig = Rig::new();
        rig.run(Command::Tool(DIAMOND));
        rig.hover(A);
        assert_eq!(rig.connect_handles(), Some("a"));
        // Losing the keyboard puts them away.
        rig.step(Input::Focus(false));
        assert!(!rig.machine.hovering());
    }

    #[test]
    fn the_connect_handles_stay_while_the_pointer_crosses_to_them() {
        let mut rig = Rig::new();
        // Before they show, leaving the node ends the rest.
        rig.to(A);
        rig.to(CANVAS);
        assert!(!rig.machine.hovering());

        rig.hover(A);
        // The gap between the node and its handle, which here is on `e`.
        rig.to((128.0, 30.0));
        assert_eq!(rig.connect_handles(), Some("a"));
        rig.to(A_RIGHT);
        assert_eq!(rig.connect_handles(), Some("a"));
        // Below the node, out to where the bottom handle reaches.
        rig.to((60.0, 82.0));
        assert_eq!(rig.connect_handles(), Some("a"));
        rig.to((60.0, 83.0));
        assert_eq!(rig.connect_handles(), None);
        assert!(!rig.machine.hovering());

        // At another zoom the gap is the same on screen.
        let mut rig = Rig::new();
        rig.viewport = Viewport {
            x: -50.0,
            y: -50.0,
            zoom: 0.5,
        };
        rig.hover(A);
        let handle = rig
            .viewport
            .to_board((50.0 * 0.5 + 60.0 + 16.0, 80.0 * 0.5));
        rig.to(handle);
        assert_eq!(rig.connect_handles(), Some("a"));
        rig.press(handle);
        rig.by(0.0, 300.0);
        assert_eq!(rig.state(), State::Connecting);
    }

    fn extend(from: &str, dir: Dir, shape: Option<Shape>) -> Intent {
        Intent::Extend {
            from: id(from),
            dir: Some(dir),
            shape,
        }
    }

    fn floating(node: &str) -> Endpoint {
        pinned(node, None)
    }

    fn pinned(node: &str, side: Option<Side>) -> Endpoint {
        Endpoint::Node {
            node: id(node),
            side,
        }
    }

    fn joined(from: &str, to: Endpoint) -> Intent {
        Intent::AddEdge {
            from: floating(from),
            to,
        }
    }

    fn rebound(edge: &str, end: EdgeEnd, to: Endpoint) -> Intent {
        Intent::Rebind {
            id: id(edge),
            end,
            to,
        }
    }

    #[test]
    fn a_click_on_a_connect_handle_connects_that_way() {
        let mut rig = Rig::new();
        rig.click(B);
        rig.hover(A);
        // The handle's node is the one connected from, whatever was
        // selected.
        assert_eq!(
            rig.click(A_RIGHT),
            [Act::Edit(extend("a", Dir::Right, None))]
        );
        assert_eq!((rig.selected(), rig.cursor()), (vec!["a"], Some("a")));
        assert_eq!(rig.connect_handles(), None);
        // Each side its own way.
        for (point, dir) in [
            ((60.0, -16.0), Dir::Up),
            ((60.0, 76.0), Dir::Down),
            ((-16.0, 30.0), Dir::Left),
        ] {
            rig.hover(A);
            assert_eq!(rig.click(point), [Act::Edit(extend("a", dir, None))]);
        }
        // With a shape armed, the handle is taken before the tool makes
        // its shape there, and the new node is that shape (§5.6 rule 7).
        rig.run(Command::Tool(DIAMOND));
        rig.hover(A);
        assert_eq!(
            rig.click(A_RIGHT),
            [Act::Edit(extend("a", Dir::Right, Some(Shape::Diamond)))]
        );
        assert_eq!(rig.machine.tool(), Tool::Select);
        // Two clicks in a row are two commands, never a double-click.
        rig.hover(A);
        rig.click(A_RIGHT);
        rig.hover(A);
        rig.wait(100);
        rig.press(A_RIGHT);
        assert_eq!(rig.release(), [Act::Edit(extend("a", Dir::Right, None))]);
    }

    #[test]
    fn a_handle_nobody_saw_is_not_pressed() {
        let mut rig = Rig::new();
        // No rest: the point outside `a` is `e`, which runs through it.
        rig.click(A_RIGHT);
        assert_eq!(rig.selected(), ["e"]);
        // Rested on, but not for long enough.
        let mut rig = Rig::new();
        rig.to(A);
        rig.wait(50);
        rig.press(A_RIGHT);
        rig.by(40.0, 40.0);
        assert_eq!(rig.state(), State::Moving);
    }

    #[test]
    fn a_drag_from_a_connect_handle_draws_a_connector_to_a_node() {
        let mut rig = Rig::new();
        rig.hover(A);
        assert!(rig.press(A_RIGHT).is_empty());
        // The handles stay until the drag begins.
        assert_eq!(rig.connect_handles(), Some("a"));
        let to_kid = joined("a", floating("kid"));
        assert_eq!(rig.to(KID), [Act::Preview(Some(to_kid.clone()))]);
        assert_eq!(rig.state(), State::Connecting);
        assert_eq!(rig.connect_handles(), None);
        assert_eq!(rig.target(), Some(("kid", None)));
        // Nothing more is asked for while the plan stays the same.
        assert!(rig.to((470.0, 245.0)).is_empty());
        // The end floats, and so does the one it was drawn from, whichever
        // handle that was (§5.12 rule 5).
        let acts = rig.release();
        assert_eq!(acts, [Act::Edit(to_kid)]);
        assert!(rig.selected().is_empty());
        rig.play(&acts);
        assert_eq!(rig.state(), State::Idle);
        // The new connector is selected, and is the cursor.
        let made: Vec<&Id> = rig.board.edges.keys().collect();
        assert_eq!(made.len(), 3);
        assert_eq!(rig.selected().len(), 1);
        assert!(!["e", "free"].contains(&rig.selected()[0]));
        assert_eq!(rig.cursor(), Some(rig.selected()[0]));

        // A second connector between the same pair may be drawn (§5.12
        // rule 10).
        rig.hover(A);
        rig.press(A_RIGHT);
        assert_eq!(rig.to(B), [Act::Preview(Some(joined("a", floating("b"))))]);
        let acts = rig.release();
        rig.play(&acts);
        assert_eq!(rig.board.edges.len(), 4);
    }

    #[test]
    fn a_connector_released_on_empty_canvas_ends_in_a_new_node() {
        let mut rig = Rig::new();
        rig.hover(A);
        rig.press(A_RIGHT);
        // A default rectangle centred on the pointer, on the grid.
        let made = Intent::AddNode {
            kind: NodeKind::Shape,
            shape: Shape::Rect,
            at: [240, 120],
            size: NODE_SIZE,
            from: Some(floating("a")),
        };
        assert_eq!(rig.to(CANVAS), [Act::Preview(Some(made.clone()))]);
        assert_eq!(rig.target(), None);
        // The preview follows by whole cells.
        assert!(rig.to((304.0, 153.0)).is_empty());
        assert_eq!(rig.to((310.0, 150.0)).len(), 1);
        rig.to(CANVAS);
        // A connector's line is canvas to a dragged end.
        assert!(matches!(
            rig.to(FREE).as_slice(),
            [Act::Preview(Some(Intent::AddNode { .. }))]
        ));
        rig.to(CANVAS);
        let acts = rig.release();
        assert_eq!(acts, [Act::Edit(made)]);
        let answers = rig.play(&acts);
        assert_eq!(answers, [Act::OpenLabel(id("new1"))]);
        assert_eq!(rig.state(), State::Editing);
        assert_eq!(rig.selected(), ["new1"]);

        // With a shape armed, the new node is that shape, and the tool is
        // spent.
        let mut rig = Rig::new();
        rig.run(Command::Tool(DIAMOND));
        rig.hover(A);
        rig.press(A_RIGHT);
        rig.to(CANVAS);
        assert!(matches!(
            rig.release().as_slice(),
            [Act::Edit(Intent::AddNode {
                shape: Shape::Diamond,
                from: Some(_),
                ..
            })]
        ));
        assert_eq!(rig.machine.tool(), Tool::Select);
    }

    #[test]
    fn a_connector_released_on_its_source_is_cancelled() {
        let mut rig = Rig::new();
        rig.hover(A);
        rig.press(A_RIGHT);
        rig.to(B);
        // Back over the node it came from, there is nothing to show.
        assert_eq!(rig.to(A), [Act::Preview(None)]);
        assert_eq!(rig.target(), None);
        assert!(rig.release().is_empty());
        assert_eq!(rig.state(), State::Idle);
        assert!(rig.selected().is_empty());
        assert_eq!(rig.board.edges.len(), 2);

        // Escape and a lost pointer cancel it too, putting the board back
        // on show; the release that follows finds nothing.
        for end in [ESCAPE, Input::Cancel] {
            let mut rig = Rig::new();
            rig.hover(A);
            rig.press(A_RIGHT);
            rig.to(B);
            assert_eq!(rig.step(end), [Act::Preview(None)]);
            assert_eq!(rig.state(), State::Idle);
            assert!(rig.release().is_empty());
        }
    }

    #[test]
    fn the_connector_tool_draws_from_a_node_and_only_from_one() {
        let mut rig = Rig::new();
        rig.run(Command::Tool(Tool::Connector));
        // A press on empty canvas or on a connector does nothing.
        rig.press(CANVAS);
        assert_eq!(rig.state(), State::Armed);
        rig.press(E);
        assert_eq!(rig.state(), State::Armed);
        // A click on a node does nothing and keeps the tool.
        assert!(rig.click(A).is_empty());
        assert_eq!(rig.state(), State::Armed);
        assert!(rig.selected().is_empty());

        rig.press(A);
        assert_eq!(rig.to(B), [Act::Preview(Some(joined("a", floating("b"))))]);
        // Released on its source it is cancelled and the tool stays for
        // another try.
        rig.to(A);
        assert!(rig.release().is_empty());
        assert_eq!(rig.state(), State::Armed);

        rig.press(A);
        rig.to(B);
        let acts = rig.release();
        assert_eq!(acts, [Act::Edit(joined("a", floating("b")))]);
        assert_eq!(rig.machine.tool(), Tool::Select);
        rig.play(&acts);
        assert_eq!(rig.state(), State::Idle);

        // Released on empty canvas it makes a rectangle.
        rig.run(Command::Tool(Tool::Connector));
        rig.press(B);
        rig.to(CANVAS);
        assert!(matches!(
            rig.release().as_slice(),
            [Act::Edit(Intent::AddNode {
                shape: Shape::Rect,
                kind: NodeKind::Shape,
                from: Some(_),
                ..
            })]
        ));

        // A selected node's resize handle is on its box, so the tool
        // draws from there as well.
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(Command::Tool(Tool::Connector));
        rig.press(A_CORNER);
        rig.to(B);
        assert_eq!(rig.state(), State::Connecting);

        // A board that cannot be edited arms no connector tool.
        let mut rig = Rig::new();
        rig.writable = false;
        rig.run(Command::Tool(Tool::Connector));
        rig.press(A);
        rig.to(B);
        assert_ne!(rig.state(), State::Connecting);
    }

    #[test]
    fn an_end_released_near_a_side_handle_is_pinned_to_that_side() {
        let mut rig = Rig::new();
        rig.hover(A);
        rig.press(A_RIGHT);
        // The middle of the node: floating.
        rig.to(KID);
        assert_eq!(rig.target(), Some(("kid", None)));
        // `kid` is 80 by 40 at 420,220. On the middle of its top side.
        assert_eq!(
            rig.to((460.0, 220.0)),
            [Act::Preview(Some(joined(
                "a",
                pinned("kid", Some(Side::Top))
            )))]
        );
        assert_eq!(rig.target(), Some(("kid", Some(Side::Top))));
        // Within ten pixels of it, inside the node.
        rig.to((466.0, 228.0));
        assert_eq!(rig.target(), Some(("kid", Some(Side::Top))));
        // Just outside it too, though the point is on the node it is
        // drawn in, which is the target only past the handle's reach.
        rig.to((460.0, 213.0));
        assert_eq!(rig.target(), Some(("kid", Some(Side::Top))));
        rig.to((460.0, 209.0));
        assert_eq!(rig.target(), Some(("box", None)));
        // Outside a node on the canvas the handle still reaches, and no
        // further than its ten pixels.
        rig.to((260.0, -7.0));
        assert_eq!(rig.target(), Some(("b", Some(Side::Top))));
        rig.to((260.0, -11.0));
        assert_eq!(rig.target(), None);
        // Further than that from every handle, on the node: floating.
        rig.to((440.0, 240.0));
        assert_eq!(rig.target(), Some(("kid", None)));
        // Each side has its handle.
        for (point, side) in [
            ((500.0, 240.0), Side::Right),
            ((460.0, 260.0), Side::Bottom),
            ((420.0, 240.0), Side::Left),
        ] {
            rig.to(point);
            assert_eq!(rig.target(), Some(("kid", Some(side))), "{side:?}");
        }
        assert_eq!(
            rig.release(),
            [Act::Edit(joined("a", pinned("kid", Some(Side::Left))))]
        );

        // Within reach of a side handle of the node it came from, a new
        // connector is as over that node: nothing.
        let mut rig = Rig::new();
        rig.hover(A);
        rig.press(A_RIGHT);
        rig.to(B);
        assert_eq!(rig.to((127.0, 33.0)), [Act::Preview(None)]);
        assert_eq!(rig.target(), None);
        assert!(rig.release().is_empty());
    }

    #[test]
    fn the_middle_of_a_node_floats_however_small_it_is_drawn() {
        let mut rig = Rig::new();
        rig.viewport = Viewport {
            x: -100.0,
            y: -100.0,
            zoom: 0.25,
        };
        // `b` is 30 by 15 pixels here: every point of it is within ten
        // pixels of a side's middle.
        rig.hover(A);
        rig.press(rig.viewport.to_board((25.0 + 30.0 + 16.0, 25.0 + 7.5)));
        rig.to(B);
        assert_eq!(rig.target(), Some(("b", None)));
        rig.to((260.0, 4.0));
        assert_eq!(rig.target(), Some(("b", Some(Side::Top))));
    }

    #[test]
    fn dragging_an_end_of_a_connector_attaches_it_elsewhere() {
        let mut rig = Rig::new();
        rig.click(E);
        rig.wait(400);
        // The handle at the `to` end of `e` is taken before `b`, whose
        // side it lies on.
        assert!(rig.press(B_LEFT).is_empty());
        let to_kid = rebound("e", EdgeEnd::To, floating("kid"));
        assert_eq!(rig.to(KID), [Act::Preview(Some(to_kid.clone()))]);
        assert_eq!(rig.state(), State::Connecting);
        assert_eq!(rig.target(), Some(("kid", None)));
        assert_eq!(rig.machine.connecting().unwrap().loose, None);
        // Pinned by a side handle, as a new connector's end is.
        let on_top = rebound("e", EdgeEnd::To, pinned("kid", Some(Side::Top)));
        assert_eq!(rig.to((460.0, 221.0)), [Act::Preview(Some(on_top.clone()))]);
        let acts = rig.release();
        assert_eq!(acts, [Act::Edit(on_top)]);
        rig.play(&acts);
        assert_eq!(rig.state(), State::Idle);
        // The connector stays selected.
        assert_eq!(rig.selected(), ["e"]);
        assert!(matches!(
            &rig.board.edges[&id("e")].to,
            End::Node { node, side: Some(_), .. } if node == "kid"
        ));
    }

    #[test]
    fn an_attached_end_released_on_empty_canvas_goes_back() {
        let mut rig = Rig::new();
        rig.click(E);
        rig.wait(400);
        rig.press(B_LEFT);
        // Nothing to show: the connector stays as it is, and the view is
        // told which end is loose.
        assert!(rig.to(CANVAS).is_empty());
        assert_eq!(rig.state(), State::Connecting);
        let connecting = rig.machine.connecting().unwrap();
        assert_eq!(connecting.loose, Some((&id("e"), EdgeEnd::To)));
        assert_eq!((connecting.target, connecting.at), (None, CANVAS));
        // Nor may it join the node its other end is on.
        rig.to(KID);
        assert_eq!(rig.to(A), [Act::Preview(None)]);
        assert_eq!(rig.target(), None);
        assert!(rig.machine.connecting().unwrap().loose.is_some());
        rig.to(CANVAS);
        assert!(rig.release().is_empty());
        assert_eq!(rig.state(), State::Idle);
        assert_eq!(rig.selected(), ["e"]);

        // Onto the node it is on already, floating as it was: an edit
        // that plans nothing.
        rig.wait(400);
        rig.press(B_LEFT);
        rig.to(B);
        let acts = rig.release();
        assert_eq!(acts, [Act::Edit(rebound("e", EdgeEnd::To, floating("b")))]);
        let before = rig.board.clone();
        rig.play(&acts);
        assert_eq!(rig.board, before);
    }

    #[test]
    fn a_free_end_moves_to_the_grid_or_onto_a_node() {
        let mut rig = Rig::new();
        rig.click(FREE);
        rig.wait(400);
        // The `to` end of `free`, at 200,400.
        rig.press((200.0, 400.0));
        let moved = rebound("free", EdgeEnd::To, Endpoint::Free { at: [340, 440] });
        assert_eq!(rig.to((333.0, 444.0)), [Act::Preview(Some(moved.clone()))]);
        assert_eq!(rig.machine.connecting().unwrap().loose, None);
        // Within the cell nothing more is shown.
        assert!(rig.to((338.0, 436.0)).is_empty());
        let acts = rig.release();
        assert_eq!(acts, [Act::Edit(moved)]);
        rig.play(&acts);
        assert!(matches!(
            rig.board.edges[&id("free")].to,
            End::Free { at: [340, 440], .. }
        ));

        // The other end, onto a node.
        rig.wait(400);
        rig.press((0.0, 400.0));
        let on_off = rebound("free", EdgeEnd::From, floating("off"));
        assert_eq!(rig.to(OFF), [Act::Preview(Some(on_off.clone()))]);
        assert_eq!(rig.release(), [Act::Edit(on_off)]);
    }

    #[test]
    fn an_end_handle_clicked_is_its_connector_clicked() {
        let mut rig = Rig::new();
        rig.click(E);
        assert!(rig.click(B_LEFT).is_empty());
        assert_eq!(rig.selected(), ["e"]);
        // And a double-click there edits the connector's label.
        assert_eq!(rig.double_click(B_LEFT), [Act::OpenLabel(id("e"))]);
        assert_eq!(rig.editing(), Some("e"));
    }

    #[test]
    fn a_key_moves_the_selection_a_cell() {
        let mut rig = Rig::new();
        // Nothing selected: nothing to move.
        assert!(rig.run(Command::Move(Dir::Right)).is_empty());
        rig.click(A);
        rig.step(shift());
        rig.click(E);
        rig.step(plain());
        for (dir, by) in [
            (Dir::Right, [20, 0]),
            (Dir::Left, [-20, 0]),
            (Dir::Down, [0, 20]),
            (Dir::Up, [0, -20]),
        ] {
            rig.wait(2000);
            assert_eq!(
                rig.run(Command::Move(dir)),
                [Act::Edit(moved(&["a", "e"], by))],
                "{dir:?}"
            );
        }
    }

    #[test]
    fn a_key_takes_a_cursor_node_off_the_grid_to_the_next_line() {
        // `off` is at 7,213.
        let mut rig = Rig::new();
        rig.click(A);
        rig.step(shift());
        rig.click(OFF);
        rig.step(plain());
        assert_eq!(rig.cursor(), Some("off"));
        for (dir, by) in [
            (Dir::Right, [13, 0]),
            (Dir::Left, [-7, 0]),
            (Dir::Down, [0, 7]),
            (Dir::Up, [0, -13]),
        ] {
            rig.wait(2000);
            assert_eq!(
                rig.run(Command::Move(dir)),
                [Act::Edit(moved(&["a", "off"], by))],
                "{dir:?}"
            );
        }
        // Left of and above the origin the next line is still the next
        // line that way.
        let mut rig = Rig::new();
        rig.board.nodes.get_mut("a").expect("the node").at = [-33, -47];
        rig.scene = scene::build(&rig.board, &fixtures::palette(), &FixedMeasure);
        rig.click((0.0, 0.0));
        assert_eq!(rig.selected(), ["a"]);
        for (dir, by) in [
            (Dir::Right, [13, 0]),
            (Dir::Left, [-7, 0]),
            (Dir::Down, [0, 7]),
            (Dir::Up, [0, -13]),
        ] {
            rig.wait(2000);
            assert_eq!(
                rig.run(Command::Move(dir)),
                [Act::Edit(moved(&["a"], by))],
                "{dir:?}"
            );
        }
        // The cursor decides, and only while it is selected.
        rig.click(B);
        assert_eq!(
            rig.run(Command::Move(Dir::Right)),
            [Act::Edit(moved(&["b"], [20, 0]))]
        );
    }

    #[test]
    fn a_key_resizes_every_selected_node_from_its_corner() {
        let mut rig = Rig::new();
        rig.run(Command::SelectAll);
        let all = ["a", "b", "box", "kid", "off"];
        for (grow, right, bottom) in [
            (Grow::Wider, 20, 0),
            (Grow::Narrower, -20, 0),
            (Grow::Taller, 0, 20),
            (Grow::Shorter, 0, -20),
        ] {
            rig.wait(2000);
            let want = Intent::Resize {
                ids: ids(&all),
                edges: Edges {
                    right,
                    bottom,
                    ..Edges::default()
                },
            };
            assert_eq!(
                rig.run(Command::Resize(grow)),
                [Act::Edit(want)],
                "{grow:?}"
            );
        }
        // Connectors alone have no size.
        rig.click(E);
        assert!(rig.run(Command::Resize(Grow::Wider)).is_empty());
    }

    #[test]
    fn a_run_of_key_moves_is_one_undo_step() {
        let right = moved(&["a"], [20, 0]);
        let wider = Intent::Resize {
            ids: ids(&["a"]),
            edges: Edges {
                right: 20,
                ..Edges::default()
            },
        };
        let mut rig = Rig::new();
        rig.click(A);
        let go = Command::Move(Dir::Right);
        assert_eq!(rig.run(go), [Act::Edit(right.clone())]);
        rig.wait(999);
        assert_eq!(rig.run(go), [Act::Continue(right.clone())]);
        // Another direction is the same run.
        rig.wait(100);
        assert_eq!(
            rig.run(Command::Move(Dir::Down)),
            [Act::Continue(moved(&["a"], [0, 20]))]
        );
        // The view's answer to an edit does not end the run.
        let acts = rig.run(go);
        assert_eq!(acts, [Act::Continue(right.clone())]);
        rig.play(&acts);
        assert_eq!(rig.run(go), [Act::Continue(right.clone())]);
        // A second's pause does.
        rig.wait(1000);
        assert_eq!(rig.run(go), [Act::Edit(right.clone())]);
        // So does a resize, which begins a run of its own.
        let grow = Command::Resize(Grow::Wider);
        assert_eq!(rig.run(grow), [Act::Edit(wider.clone())]);
        assert_eq!(rig.run(grow), [Act::Continue(wider.clone())]);
        assert_eq!(rig.run(go), [Act::Edit(right.clone())]);
        assert_eq!(rig.run(go), [Act::Continue(right.clone())]);

        // And so does anything else: a command, a press, Escape, a
        // selection put back, the board taken away.
        let breaks: [(&str, Play); 7] = [
            ("a command", |rig| {
                rig.run(Command::SelectNone);
                rig.run(Command::SelectAll);
                rig.machine.select(BTreeSet::from([id("a")]), Some(id("a")));
            }),
            ("a command of the view's", |rig| {
                rig.run(Command::Copy);
            }),
            ("a click", |rig| {
                rig.click(A);
            }),
            ("a right press", |rig| {
                rig.press_with(Button::Right, A);
            }),
            ("escape", |rig| {
                rig.run(Command::Tool(Tool::Hand));
                rig.step(ESCAPE);
            }),
            ("settle", |rig| {
                rig.machine.settle();
            }),
            ("a selection put back", |rig| {
                rig.machine.select(BTreeSet::from([id("a")]), Some(id("a")));
            }),
        ];
        for (name, interrupt) in breaks {
            let mut rig = Rig::new();
            rig.click(A);
            rig.run(go);
            assert_eq!(rig.run(go), [Act::Continue(right.clone())], "{name}");
            interrupt(&mut rig);
            assert_eq!(rig.run(go), [Act::Edit(right.clone())], "{name}");
        }
    }

    #[test]
    fn any_pointer_input_ends_a_run_of_key_moves() {
        let right = moved(&["a"], [20, 0]);
        let go = Command::Move(Dir::Right);
        // A press is in the table above.
        let pointer: [(&str, Input); 3] = [
            ("a wheel step", wheel(lines(-1.0), false)),
            ("a pointer move", Input::Move { at: AT }),
            (
                "a release",
                Input::Release {
                    button: Button::Left,
                },
            ),
        ];
        for (name, input) in pointer {
            let mut rig = Rig::new();
            rig.click(A);
            rig.run(go);
            assert_eq!(rig.run(go), [Act::Continue(right.clone())], "{name}");
            rig.step(input);
            assert_eq!(rig.run(go), [Act::Edit(right.clone())], "{name}");
        }
        // Keys that are not pointer input leave it open.
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(go);
        rig.step(plain());
        assert_eq!(rig.run(go), [Act::Continue(right)]);
    }

    #[test]
    fn a_shape_command_reshapes_the_selected_nodes() {
        let mut rig = Rig::new();
        assert!(rig.run(Command::Shape(Shape::Pill)).is_empty());
        rig.click(E);
        assert!(rig.run(Command::Shape(Shape::Pill)).is_empty());
        rig.step(shift());
        rig.click(A);
        rig.click(B);
        rig.step(plain());
        assert_eq!(
            rig.run(Command::Shape(Shape::Pill)),
            [Act::Edit(Intent::SetShape {
                ids: ids(&["a", "b"]),
                shape: Shape::Pill,
            })]
        );
        assert_eq!(rig.selected(), ["a", "b", "e"]);
    }

    #[test]
    fn an_armed_shape_tool_makes_its_shape_where_it_is_clicked() {
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(Command::Tool(DIAMOND));
        assert_eq!(rig.machine.tool(), DIAMOND);
        // 300,150 is the centre of a box at 240,120.
        let acts = rig.click(CANVAS);
        let want = added(NodeKind::Shape, Shape::Diamond, [240, 120], [120, 60]);
        assert_eq!(acts, [Act::Edit(want)]);
        // The tool is spent, and nothing is selected for it yet.
        assert_eq!(rig.machine.tool(), Tool::Select);
        assert_eq!(rig.selected(), ["a"]);
        // The view's answer selects the new node and opens its label.
        assert_eq!(rig.play(&acts), [Act::OpenLabel(id("new1"))]);
        assert_eq!((rig.selected(), rig.cursor()), (vec!["new1"], Some("new1")));
        assert_eq!((rig.state(), rig.editing()), (State::Editing, Some("new1")));
    }

    #[test]
    fn a_tool_makes_a_node_even_over_another() {
        // Each tool, and what a click at the centre of `a` makes: the
        // corner is the grid line nearest the point less half the size.
        let cases = [
            (
                Tool::Shape(Shape::Rect),
                NodeKind::Shape,
                Shape::Rect,
                [0, 0],
                [120, 60],
            ),
            (
                Tool::Shape(Shape::Cylinder),
                NodeKind::Shape,
                Shape::Cylinder,
                [0, 0],
                [120, 60],
            ),
            (Tool::Text, NodeKind::Text, Shape::Rect, [0, 20], [120, 20]),
        ];
        for (tool, kind, shape, at, size) in cases {
            let mut rig = Rig::new();
            rig.run(Command::Tool(tool));
            assert_eq!(
                rig.click(A),
                [Act::Edit(added(kind, shape, at, size))],
                "{tool:?}"
            );
            assert!(rig.selected().is_empty());
        }
    }

    #[test]
    fn a_drag_with_a_tool_makes_a_node_between_two_grid_corners() {
        let made = |at, size| added(NodeKind::Shape, Shape::Diamond, at, size);
        // Where the press and the release are, and the box between their
        // grid corners; at least a cell each way, towards the drag.
        let cases = [
            ((303.0, 148.0), (371.0, 222.0), [300, 140], [80, 80]),
            ((300.0, 140.0), (255.0, 95.0), [260, 100], [40, 40]),
            ((300.0, 140.0), (305.0, 143.0), [300, 140], [20, 20]),
            ((300.0, 140.0), (294.0, 140.0), [280, 140], [20, 20]),
            ((300.0, 140.0), (300.0, 133.0), [300, 120], [20, 20]),
            ((300.0, 140.0), (380.0, 139.0), [300, 120], [80, 20]),
        ];
        for (from, to, at, size) in cases {
            let mut rig = Rig::new();
            rig.run(Command::Tool(DIAMOND));
            rig.press(from);
            assert_eq!(
                rig.to(to),
                [Act::Preview(Some(made(at, size)))],
                "{from:?} to {to:?}"
            );
            assert_eq!(rig.state(), State::Creating);
            assert_eq!(rig.machine.tool(), DIAMOND);
            let acts = rig.release();
            assert_eq!(acts, [Act::Edit(made(at, size))]);
            assert_eq!(rig.machine.tool(), Tool::Select);
            assert_eq!(rig.play(&acts), [Act::OpenLabel(id("new1"))]);
            assert_eq!(rig.state(), State::Editing);
        }
        // The preview follows the pointer, and a move within the cell says
        // nothing.
        let mut rig = Rig::new();
        rig.run(Command::Tool(Tool::Text));
        rig.press((300.0, 140.0));
        let text = |size| added(NodeKind::Text, Shape::Rect, [300, 140], size);
        assert_eq!(rig.by(41.0, 22.0), [Act::Preview(Some(text([40, 20])))]);
        assert!(rig.by(44.0, 25.0).is_empty());
        assert_eq!(rig.by(61.0, 45.0), [Act::Preview(Some(text([60, 40])))]);
        // Cancelled, nothing is made and the tool is still armed.
        assert_eq!(rig.step(ESCAPE), [Act::Preview(None)]);
        assert_eq!(
            (rig.state(), rig.machine.tool()),
            (State::Armed, Tool::Text)
        );
        assert!(rig.release().is_empty());
    }

    #[test]
    fn a_double_click_opens_a_label_or_makes_a_rectangle() {
        let mut rig = Rig::new();
        assert_eq!(rig.double_click(B), [Act::OpenLabel(id("b"))]);
        assert_eq!((rig.state(), rig.editing()), (State::Editing, Some("b")));
        assert_eq!((rig.selected(), rig.cursor()), (vec!["b"], Some("b")));
        rig.run(Command::CommitLabel);

        // A connector's label opens the same way.
        assert_eq!(rig.double_click(E), [Act::OpenLabel(id("e"))]);
        assert_eq!((rig.selected(), rig.editing()), (vec!["e"], Some("e")));
        rig.run(Command::CommitLabel);

        // On empty canvas, a default rectangle centred there.
        let acts = rig.double_click(CANVAS);
        let want = added(NodeKind::Shape, Shape::Rect, [240, 120], [120, 60]);
        assert_eq!(acts, [Act::Edit(want)]);
        assert_eq!(rig.play(&acts), [Act::OpenLabel(id("new1"))]);
        assert_eq!(
            (rig.selected(), rig.editing()),
            (vec!["new1"], Some("new1"))
        );
    }

    #[test]
    fn a_double_click_is_two_presses_close_in_time_and_place() {
        // The gap in milliseconds, how far the second press is from the
        // first in pixels, and whether that is a double-click.
        let cases = [
            (300, 5.9, true),
            (0, 0.0, true),
            (301, 0.0, false),
            (100, 6.0, false),
        ];
        for (gap, apart, double) in cases {
            let mut rig = Rig::new();
            rig.press(A);
            rig.release();
            rig.wait(gap);
            rig.press((A.0 + apart, A.1));
            let acts = rig.release();
            assert_eq!(!acts.is_empty(), double, "{gap} ms, {apart} px");
            assert_eq!(rig.state() == State::Editing, double);
        }
        // A third press is a first one again.
        let mut rig = Rig::new();
        rig.double_click(CANVAS);
        rig.wait(100);
        assert!(rig.click(CANVAS).is_empty());
        // A drag is no first click.
        let mut rig = Rig::new();
        rig.press(A);
        rig.by(40.0, 0.0);
        rig.step(ESCAPE);
        rig.release();
        rig.wait(50);
        rig.press(A);
        assert!(rig.release().is_empty());
        assert_eq!(rig.state(), State::Idle);
        // Nor is a click with a tool, which made something each time.
        let mut rig = Rig::new();
        rig.run(Command::Tool(DIAMOND));
        rig.press(CANVAS);
        rig.release();
        rig.wait(50);
        rig.press(CANVAS);
        assert!(rig.release().is_empty());
    }

    #[test]
    fn keyboard_building_extends_from_the_selected_node() {
        let extend = |from: &str, dir, shape| {
            Act::Edit(Intent::Extend {
                from: id(from),
                dir,
                shape,
            })
        };
        let mut rig = Rig::new();
        rig.click(A);
        assert_eq!(rig.run(Command::ConnectNext), [extend("a", None, None)]);
        assert_eq!(
            rig.run(Command::Connect(Dir::Left)),
            [extend("a", Some(Dir::Left), None)]
        );
        // The armed shape is the new node's, and the tool is spent.
        rig.run(Command::Tool(DIAMOND));
        let acts = rig.run(Command::ConnectNext);
        assert_eq!(acts, [extend("a", None, Some(Shape::Diamond))]);
        assert_eq!(rig.machine.tool(), Tool::Select);
        assert_eq!(rig.play(&acts), [Act::OpenLabel(id("new1"))]);
        assert_eq!(
            (rig.selected(), rig.editing()),
            (vec!["new1"], Some("new1"))
        );
        // Tab in a label commits it and goes on from its node.
        assert_eq!(
            rig.run(Command::ConnectNext),
            [Act::CommitLabel, extend("new1", None, None)]
        );
        // Another tool gives a rectangle and is spent too.
        for tool in [Tool::Text, Tool::Connector, Tool::Hand] {
            let mut rig = Rig::new();
            rig.click(A);
            rig.run(Command::Tool(tool));
            assert_eq!(
                rig.run(Command::Connect(Dir::Up)),
                [extend("a", Some(Dir::Up), None)],
                "{tool:?}"
            );
            assert_eq!(rig.machine.tool(), Tool::Select);
        }
        // From a selected connector, Tab goes on from the node it points
        // at; a direction needs a node.
        let mut rig = Rig::new();
        rig.click(E);
        assert_eq!(rig.run(Command::ConnectNext), [extend("b", None, None)]);
        assert!(rig.run(Command::Connect(Dir::Right)).is_empty());
        // A connector that ends nowhere, several elements, or none: this
        // machine makes nothing, and the tool stays armed.
        rig.click(FREE);
        assert!(rig.run(Command::ConnectNext).is_empty());
        rig.run(Command::SelectAll);
        rig.run(Command::Tool(DIAMOND));
        assert!(rig.run(Command::ConnectNext).is_empty());
        rig.run(Command::SelectNone);
        assert!(rig.run(Command::ConnectNext).is_empty());
        assert!(rig.run(Command::Connect(Dir::Down)).is_empty());
        assert_eq!(rig.machine.tool(), DIAMOND);
    }

    #[test]
    fn a_plan_that_connects_to_a_node_selects_it_and_opens_nothing() {
        // `board.connect.<dir>` onto a node already there (§5.6 rule 6).
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(Command::Connect(Dir::Right));
        let plan = Plan {
            select: Some(ids(&["b"])),
            ..Plan::default()
        };
        assert!(rig.machine.applied(&plan, &rig.board).is_empty());
        assert_eq!((rig.selected(), rig.cursor()), (vec!["b"], Some("b")));
        assert_eq!(rig.state(), State::Idle);
        // The cursor is the first node of what a plan selects.
        let plan = Plan {
            select: Some(ids(&["free", "off", "a"])),
            ..Plan::default()
        };
        rig.machine.applied(&plan, &rig.board);
        assert_eq!(rig.selected(), ["a", "free", "off"]);
        assert_eq!(rig.cursor(), Some("off"));
        // What a plan selects but the board has not is not selected, and
        // a label is opened only on an element that is there.
        let plan = Plan {
            select: Some(ids(&["gone", "e"])),
            edit: Some(id("gone")),
            ..Plan::default()
        };
        assert!(rig.machine.applied(&plan, &rig.board).is_empty());
        assert_eq!((rig.selected(), rig.cursor()), (vec!["e"], Some("e")));
        assert_eq!(rig.state(), State::Idle);
        // A plan that selects nothing leaves the cursor.
        let plan = Plan {
            select: Some(Vec::new()),
            ..Plan::default()
        };
        rig.machine.applied(&plan, &rig.board);
        assert_eq!((rig.selected(), rig.cursor()), (vec![], Some("e")));
    }

    #[test]
    fn what_an_edit_removes_leaves_the_selection() {
        let mut rig = Rig::new();
        rig.run(Command::SelectAll);
        rig.click(A);
        rig.run(Command::SelectAll);
        assert_eq!(rig.cursor(), Some("a"));
        let plan = Plan {
            changes: vec![
                Change::RemoveNode(id("a")),
                Change::RemoveEdge(id("e")),
                Change::RemoveNode(id("kid")),
            ],
            ..Plan::default()
        };
        rig.board.apply(&plan.changes);
        assert!(rig.machine.applied(&plan, &rig.board).is_empty());
        assert_eq!(rig.selected(), ["b", "box", "free", "off"]);
        assert_eq!(rig.cursor(), None);
    }

    #[test]
    fn label_editing_begins_with_one_element_and_ends_by_committing() {
        let mut rig = Rig::new();
        // It needs exactly one element.
        assert!(rig.run(Command::EditLabel).is_empty());
        rig.run(Command::SelectAll);
        assert!(rig.run(Command::EditLabel).is_empty());
        rig.click(E);
        assert_eq!(rig.run(Command::EditLabel), [Act::OpenLabel(id("e"))]);
        assert_eq!((rig.state(), rig.editing()), (State::Editing, Some("e")));
        // The commit command, and only once.
        assert_eq!(rig.run(Command::CommitLabel), [Act::CommitLabel]);
        assert_eq!((rig.state(), rig.editing()), (State::Idle, None));
        assert!(rig.run(Command::CommitLabel).is_empty());
        assert_eq!(rig.selected(), ["e"]);

        // An armed tool is put away by a label.
        rig.run(Command::Tool(DIAMOND));
        rig.run(Command::EditLabel);
        assert_eq!(rig.machine.tool(), Tool::Select);
        // Escape commits, and leaves the selection for its next press.
        assert_eq!(rig.step(ESCAPE), [Act::CommitLabel]);
        assert_eq!(rig.selected(), ["e"]);

        // A press elsewhere commits and is then a press like any other.
        rig.click(A);
        rig.run(Command::EditLabel);
        rig.wait(400);
        assert_eq!(rig.press(B), [Act::CommitLabel]);
        assert_eq!(rig.state(), State::Pressed);
        rig.release();
        assert_eq!((rig.selected(), rig.editing()), (vec!["b"], None));
        // A press on the element itself is the label's: it never moves
        // the node or commits.
        rig.run(Command::EditLabel);
        rig.wait(400);
        let caret = |at, select| Act::Caret { at, select };
        assert_eq!(rig.press(B), [caret(B, false)]);
        assert_eq!(rig.by(40.0, 40.0), [caret((300.0, 70.0), true)]);
        assert!(rig.release().is_empty());
        assert_eq!((rig.state(), rig.editing()), (State::Editing, Some("b")));
        assert!(rig.press_with(Button::Right, B).is_empty());
        assert_eq!(rig.state(), State::Editing);
        // A right press elsewhere commits and selects.
        assert_eq!(rig.press_with(Button::Right, A), [Act::CommitLabel]);
        assert_eq!((rig.selected(), rig.state()), (vec!["a"], State::Idle));
        // Losing the keyboard commits; a cancelled pointer does not.
        rig.run(Command::EditLabel);
        assert!(rig.step(Input::Cancel).is_empty());
        assert_eq!(rig.state(), State::Editing);
        assert_eq!(rig.step(Input::Focus(false)), [Act::CommitLabel]);
        assert_eq!(rig.state(), State::Unfocused);
    }

    #[test]
    fn the_pointer_in_an_open_label_places_its_caret_and_selects() {
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(Command::EditLabel);
        rig.wait(400);
        let caret = |at, select| Act::Caret { at, select };
        // A press puts the caret there; the drag that follows selects,
        // from its first pixel and wherever it goes.
        assert_eq!(rig.press(A), [caret(A, false)]);
        assert!(rig.machine.dragging());
        assert_eq!(rig.state(), State::Editing);
        assert_eq!(rig.by(1.0, 0.0), [caret((61.0, 30.0), true)]);
        assert_eq!(rig.to(CANVAS), [caret(CANVAS, true)]);
        assert!(rig.release().is_empty());
        assert!(!rig.machine.dragging());
        assert_eq!((rig.state(), rig.editing()), (State::Editing, Some("a")));
        assert_eq!(rig.selected(), ["a"]);
        // With Shift the press itself extends the selection.
        rig.step(shift());
        assert_eq!(rig.press(A), [caret(A, true)]);
        rig.release();
        rig.step(plain());
        // The point is the board's at any zoom and pan.
        rig.viewport = Viewport {
            x: -35.0,
            y: 12.0,
            zoom: 2.0,
        };
        let acts = rig.press(A);
        let [Act::Caret { at, select: false }] = acts[..] else {
            panic!("a caret: {acts:?}");
        };
        assert!((at.0 - A.0).abs() < 1e-3 && (at.1 - A.1).abs() < 1e-3);
        // Escape ends the drag and keeps the label; the next commits it.
        assert!(rig.step(ESCAPE).is_empty());
        assert!(!rig.machine.dragging());
        assert_eq!(rig.state(), State::Editing);
        assert!(rig.to(CANVAS).is_empty());
        assert_eq!(rig.step(ESCAPE), [Act::CommitLabel]);
        // A lost pointer ends the drag too, and a command waits for it.
        rig.run(Command::EditLabel);
        rig.press(A);
        assert!(rig.run(Command::CommitLabel).is_empty());
        assert!(rig.step(Input::Cancel).is_empty());
        assert_eq!(
            (rig.state(), rig.machine.dragging()),
            (State::Editing, false)
        );
        // Settling mid-drag commits.
        rig.press(A);
        assert_eq!(rig.machine.settle(), [Act::CommitLabel]);
        assert_eq!((rig.state(), rig.machine.dragging()), (State::Idle, false));
        // The view may find no label to open.
        rig.run(Command::EditLabel);
        rig.machine.abandon();
        assert_eq!((rig.state(), rig.editing()), (State::Idle, None));
        assert!(rig.step(ESCAPE).is_empty());
        assert_eq!(rig.state(), State::Idle);
    }

    #[test]
    fn a_command_in_a_label_commits_it_first() {
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(Command::EditLabel);
        assert_eq!(
            rig.run(Command::Undo),
            [Act::CommitLabel, Act::Run(Command::Undo)]
        );
        assert_eq!(rig.state(), State::Idle);
        rig.run(Command::EditLabel);
        assert_eq!(
            rig.run(Command::Move(Dir::Right)),
            [Act::CommitLabel, Act::Edit(moved(&["a"], [20, 0]))]
        );
        // Asking for the label again closes and reopens it.
        rig.run(Command::EditLabel);
        assert_eq!(
            rig.run(Command::EditLabel),
            [Act::CommitLabel, Act::OpenLabel(id("a"))]
        );
        assert_eq!(rig.state(), State::Editing);
    }

    #[test]
    fn space_is_text_in_a_label() {
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(Command::EditLabel);
        rig.step(Input::Space { held: true });
        assert!(!rig.machine.space());
        // Nor does a Space held before the label opened pan in it.
        rig.run(Command::CommitLabel);
        rig.step(Input::Space { held: true });
        assert!(rig.machine.space());
        rig.run(Command::EditLabel);
        assert!(!rig.machine.space());
        // A middle drag still pans, and the wheel too, with the label
        // open.
        let before = rig.viewport;
        rig.press_with(Button::Middle, CANVAS);
        assert!(matches!(rig.by(-30.0, 0.0)[..], [Act::Look(_)]));
        rig.step(Input::Release {
            button: Button::Middle,
        });
        assert!(matches!(
            rig.step(wheel(lines(1.0), true))[..],
            [Act::Look(_)]
        ));
        assert_ne!(rig.viewport, before);
        assert_eq!((rig.state(), rig.editing()), (State::Editing, Some("a")));
    }

    #[test]
    fn escape_goes_down_its_ladder_one_rung_a_press() {
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(Command::Tool(DIAMOND));
        rig.press(CANVAS);
        rig.by(40.0, 40.0);
        // A gesture in flight: cancelled, with the tool still armed.
        assert!(rig.machine.escapes());
        assert_eq!(rig.step(ESCAPE), [Act::Preview(None)]);
        assert_eq!(rig.state(), State::Armed);
        rig.release();
        // An armed tool: back to Select.
        assert!(rig.machine.escapes());
        assert!(rig.step(ESCAPE).is_empty());
        assert_eq!(
            (rig.state(), rig.machine.tool()),
            (State::Idle, Tool::Select)
        );
        assert_eq!(rig.selected(), ["a"]);
        // A selection: cleared, the cursor kept.
        assert!(rig.machine.escapes());
        assert!(rig.step(ESCAPE).is_empty());
        assert_eq!((rig.selected(), rig.cursor()), (vec![], Some("a")));
        // Nothing: the key is the app's.
        assert!(!rig.machine.escapes());
        assert!(rig.step(ESCAPE).is_empty());
        assert_eq!(rig.state(), State::Idle);

        // A label comes before the tool and the selection, and a held key
        // does not run down the ladder.
        rig.click(B);
        rig.run(Command::EditLabel);
        assert!(rig.machine.escapes());
        let held = Input::Key {
            key: Key::Escape,
            repeat: true,
        };
        assert!(rig.step(held).is_empty());
        assert_eq!(rig.state(), State::Editing);
        assert_eq!(rig.step(ESCAPE), [Act::CommitLabel]);
        assert!(rig.step(held).is_empty());
        assert_eq!(rig.selected(), ["b"]);
        // A press that has not moved is a gesture too.
        rig.wait(400);
        rig.press(A);
        assert!(rig.step(ESCAPE).is_empty());
        assert_eq!((rig.state(), rig.selected()), (State::Idle, vec!["b"]));
        assert!(rig.release().is_empty());
        assert_eq!(rig.selected(), ["b"]);
    }

    #[test]
    fn a_cancelled_gesture_takes_its_preview_away_and_changes_nothing() {
        let cancels: [(&str, Play<Vec<Act>>); 5] = [
            ("escape", |rig| rig.step(ESCAPE)),
            ("cancel", |rig| rig.step(Input::Cancel)),
            ("focus", |rig| rig.step(Input::Focus(false))),
            ("settle", |rig| rig.machine.settle()),
            ("a press", |rig| rig.press_with(Button::Middle, CANVAS)),
        ];
        for (name, cancel) in cancels {
            // A move.
            let mut rig = Rig::new();
            rig.press(A);
            rig.by(40.0, 40.0);
            assert_eq!(cancel(&mut rig), [Act::Preview(None)], "{name}");
            assert!(rig.release().is_empty(), "{name}");
            assert!(rig
                .by(80.0, 80.0)
                .iter()
                .all(|act| matches!(act, Act::Look(_))));
            // A resize.
            let mut rig = Rig::new();
            rig.click(A);
            rig.wait(400);
            rig.press(A_CORNER);
            rig.by(40.0, 40.0);
            assert_eq!(cancel(&mut rig), [Act::Preview(None)], "{name}");
            assert!(rig.release().is_empty(), "{name}");
            // A gesture with nothing on show has nothing to take away.
            let mut rig = Rig::new();
            rig.press(A);
            rig.by(5.0, 5.0);
            assert_eq!(rig.state(), State::Moving);
            assert!(cancel(&mut rig).is_empty(), "{name}");
        }
    }

    #[test]
    fn a_command_waits_for_no_gesture() {
        let mut rig = Rig::new();
        rig.click(A);
        rig.wait(400);
        rig.press(A);
        for command in [
            Command::Move(Dir::Right),
            Command::Delete,
            Command::Tool(DIAMOND),
            Command::SelectNone,
            Command::EditLabel,
            Command::ConnectNext,
        ] {
            assert!(rig.run(command).is_empty(), "{command:?}");
        }
        rig.by(40.0, 0.0);
        assert!(rig.run(Command::Delete).is_empty());
        assert_eq!((rig.state(), rig.selected()), (State::Moving, vec!["a"]));
        assert_eq!(rig.machine.tool(), Tool::Select);
        assert_eq!(rig.release(), [Act::Edit(moved(&["a"], [40, 0]))]);
    }

    #[test]
    fn a_command_with_no_rule_here_is_the_view_s() {
        let mut rig = Rig::new();
        let mut passed = 0;
        for command in Command::all() {
            let own = matches!(
                command,
                Command::Tool(_)
                    | Command::Shape(_)
                    | Command::EditLabel
                    | Command::CommitLabel
                    | Command::ConnectNext
                    | Command::Connect(_)
                    | Command::SelectAll
                    | Command::SelectNone
                    | Command::Move(_)
                    | Command::Resize(_)
            );
            rig.machine.reset();
            rig.run(Command::SelectNone);
            let acts = rig.run(command);
            if own {
                assert!(!acts.contains(&Act::Run(command)), "{command:?}");
            } else {
                assert_eq!(acts, [Act::Run(command)], "{command:?}");
                passed += 1;
            }
        }
        // Walking, the connector commands, delete, duplicate, the
        // clipboard, undo and the two zooms.
        assert_eq!(passed, 27);
    }

    #[test]
    fn selecting_all_and_none() {
        let mut rig = Rig::new();
        rig.click(B);
        rig.run(Command::SelectAll);
        assert_eq!(rig.selected(), ["a", "b", "box", "e", "free", "kid", "off"]);
        assert_eq!(rig.cursor(), Some("b"));
        rig.run(Command::SelectNone);
        assert_eq!((rig.selected(), rig.cursor()), (vec![], Some("b")));
    }

    #[test]
    fn the_hand_tool_pans_once() {
        let mut rig = Rig::new();
        rig.run(Command::Tool(Tool::Hand));
        rig.press(A);
        let before = rig.viewport;
        assert!(matches!(rig.by(-30.0, 10.0)[..], [Act::Look(_)]));
        assert_eq!(
            (rig.viewport.x, rig.viewport.y),
            (before.x + 30.0, before.y - 10.0)
        );
        assert!(rig.release().is_empty());
        // Nothing was selected or made, and the tool is spent (§5.2: no
        // tool lock), so the next press selects.
        assert!(rig.selected().is_empty());
        assert_eq!(
            (rig.state(), rig.machine.tool()),
            (State::Idle, Tool::Select)
        );
        rig.click(A);
        assert_eq!(rig.selected(), ["a"]);
        // A middle drag with the hand armed leaves it armed.
        rig.run(Command::Tool(Tool::Hand));
        rig.press_with(Button::Middle, A);
        rig.step(Input::Release {
            button: Button::Middle,
        });
        assert_eq!(
            (rig.state(), rig.machine.tool()),
            (State::Armed, Tool::Hand)
        );
    }

    #[test]
    fn space_pans_with_any_tool_and_selects_nothing() {
        for tool in [Tool::Select, DIAMOND, Tool::Text, Tool::Connector] {
            let mut rig = Rig::new();
            rig.run(Command::Tool(tool));
            rig.step(Input::Space { held: true });
            rig.press(A);
            assert_eq!(rig.state(), State::Panning, "{tool:?}");
            assert!(matches!(rig.by(20.0, 0.0)[..], [Act::Look(_)]));
            assert!(rig.release().is_empty());
            assert!(rig.selected().is_empty());
            assert_eq!(rig.machine.tool(), tool);
        }
    }

    #[test]
    fn losing_the_keyboard_settles_the_board_and_puts_the_tool_away() {
        let mut rig = Rig::new();
        rig.click(A);
        rig.run(Command::Tool(DIAMOND));
        rig.step(Input::Space { held: true });
        assert!(rig.step(Input::Focus(false)).is_empty());
        assert_eq!(rig.state(), State::Unfocused);
        assert_eq!(rig.machine.tool(), Tool::Select);
        assert!(!rig.machine.space() && !rig.machine.focused());
        // The selection is kept, to be drawn muted.
        assert_eq!(rig.selected(), ["a"]);
        // A command still runs, as from the palette.
        assert_eq!(
            rig.run(Command::Move(Dir::Right)),
            [Act::Edit(moved(&["a"], [20, 0]))]
        );
        // The wheel works without the keyboard and does not take it.
        assert!(matches!(
            rig.step(wheel(lines(-1.0), false))[..],
            [Act::Look(_)]
        ));
        assert_eq!(rig.state(), State::Unfocused);
        // A click takes it and is a click.
        rig.click(B);
        assert_eq!((rig.state(), rig.selected()), (State::Idle, vec!["b"]));
    }

    #[test]
    fn a_board_that_cannot_be_edited_still_selects() {
        let mut rig = Rig::new();
        rig.writable = false;
        rig.click(A);
        assert_eq!(rig.selected(), ["a"]);
        // A double-click opens no label and makes nothing.
        assert!(rig.double_click(B).is_empty());
        assert_eq!((rig.selected(), rig.state()), (vec!["b"], State::Idle));
        assert!(rig.double_click(CANVAS).is_empty());
        // A drag on a node moves nothing; on the canvas it is a marquee.
        rig.wait(400);
        rig.press(A);
        assert!(rig.by(40.0, 40.0).is_empty());
        assert_eq!(rig.state(), State::Idle);
        assert!(rig.release().is_empty());
        rig.press((-20.0, -20.0));
        rig.to((340.0, 80.0));
        rig.release();
        assert_eq!(rig.selected(), ["a", "b", "e"]);
        // No command of the machine's changes anything, and no tool that
        // makes something is armed.
        for command in [
            Command::Move(Dir::Right),
            Command::Resize(Grow::Wider),
            Command::Shape(Shape::Pill),
            Command::ConnectNext,
            Command::Tool(DIAMOND),
            Command::Tool(Tool::Text),
            Command::Tool(Tool::Connector),
        ] {
            assert!(rig.run(command).is_empty(), "{command:?}");
            assert_eq!(rig.state(), State::Idle, "{command:?}");
        }
        rig.click(A);
        assert!(rig.run(Command::EditLabel).is_empty());
        assert!(rig.run(Command::Connect(Dir::Right)).is_empty());
        assert_eq!(rig.state(), State::Idle);
        // The hand still pans, and select-all still selects.
        rig.run(Command::Tool(Tool::Hand));
        assert_eq!(rig.state(), State::Armed);
        rig.run(Command::SelectAll);
        assert_eq!(rig.selected().len(), 7);

        // A tool armed while the board could be edited makes nothing once
        // it cannot.
        let mut rig = Rig::new();
        rig.run(Command::Tool(DIAMOND));
        rig.writable = false;
        assert!(rig.click(CANVAS).is_empty());
        rig.press(CANVAS);
        assert!(rig.by(40.0, 40.0).is_empty());
        assert!(rig.release().is_empty());
        assert_eq!(rig.state(), State::Armed);
    }

    #[test]
    fn a_reload_keeps_the_selection_that_still_exists() {
        let mut rig = Rig::new();
        rig.machine
            .select(BTreeSet::from([id("a"), id("b"), id("e")]), Some(id("a")));
        rig.run(Command::Tool(DIAMOND));
        rig.press(CANVAS);
        rig.by(40.0, 40.0);
        rig.step(Input::Space { held: true });
        assert_eq!(rig.state(), State::Creating);

        rig.machine.reset();
        assert_eq!(rig.state(), State::Idle);
        assert_eq!(rig.machine.tool(), Tool::Select);
        assert!(!rig.machine.space() && rig.machine.focused());
        assert_eq!(
            (rig.selected(), rig.cursor()),
            (vec!["a", "b", "e"], Some("a"))
        );
        // The release of the dropped gesture is nobody's.
        assert!(rig.release().is_empty());

        rig.board.nodes.remove("a");
        rig.board.edges.remove("e");
        rig.machine.retain(&rig.board);
        assert_eq!((rig.selected(), rig.cursor()), (vec!["b"], None));

        // An open label goes with its element, or with the reload.
        rig.run(Command::EditLabel);
        assert_eq!(rig.editing(), Some("b"));
        rig.machine.retain(&rig.board);
        assert_eq!(rig.editing(), Some("b"));
        rig.board.nodes.remove("b");
        rig.machine.retain(&rig.board);
        assert_eq!((rig.editing(), rig.state()), (None, State::Idle));
        rig.scene = scene::build(&rig.board, &fixtures::palette(), &FixedMeasure);
        rig.click(OFF);
        rig.run(Command::EditLabel);
        rig.machine.reset();
        assert_eq!((rig.editing(), rig.selected()), (None, vec!["off"]));
    }

    #[test]
    fn settling_commits_the_label_and_cancels_the_gesture() {
        let mut rig = Rig::new();
        assert!(rig.machine.settle().is_empty());
        rig.click(A);
        rig.run(Command::EditLabel);
        rig.press_with(Button::Middle, CANVAS);
        assert_eq!(rig.machine.settle(), [Act::CommitLabel]);
        assert_eq!(rig.state(), State::Idle);
        assert!(rig.machine.settle().is_empty());
        // The tool and the selection are not the settle's to change.
        rig.run(Command::Tool(DIAMOND));
        rig.machine.settle();
        assert_eq!((rig.state(), rig.selected()), (State::Armed, vec!["a"]));
    }

    #[test]
    fn a_pointer_that_is_nowhere_is_ignored() {
        for nowhere in [(f32::NAN, 0.0), (0.0, f32::INFINITY)] {
            let mut rig = Rig::new();
            let press = Input::Press {
                button: Button::Left,
                at: nowhere,
            };
            assert!(rig.step(press).is_empty());
            assert_eq!(rig.state(), State::Idle);
            rig.press(OFF);
            rig.by(40.0, 0.0);
            assert!(rig.step(Input::Move { at: nowhere }).is_empty());
            assert_eq!(rig.release(), [Act::Edit(moved(&["off"], [33, 7]))]);
        }
    }

    #[test]
    fn the_widget_is_told_when_a_button_is_down() {
        let mut rig = Rig::new();
        assert!(!rig.machine.dragging());
        rig.press(A);
        assert!(rig.machine.dragging() && !rig.machine.panning());
        rig.by(40.0, 0.0);
        assert!(rig.machine.dragging());
        rig.release();
        assert!(!rig.machine.dragging());
        rig.press_with(Button::Middle, A);
        assert!(rig.machine.dragging() && rig.machine.panning());
    }
}
