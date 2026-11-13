//! The board requests of the test harness (discovery/boards.md §6.2):
//! `board.get`, `board.bounds`, `board.pointer`, `board.key`, `board.type`
//! and `board.apply`. Each first asks the widget tree where the widget
//! under `note-editor` sits and whether it has the keyboard, then answers
//! in [`App::board_probed`]. Input goes in where the widget's own goes:
//! `App::board_edit`, and for a key the app's keyboard path after it.

use std::sync::Arc;

use iced::{Rectangle, Task};
use serde_json::{json, Value};
use wren_board::edit::{Endpoint, Intent};
use wren_board::scene::{self, Draw, Rect, Segment, Source};
use wren_board::{Access, Color, Edge, End, Id, Node, NodeKind, Rgba, Shape, Side, Vocabulary};
use wren_canvas::machine::{self, State};
use wren_canvas::{handles, hit, BoardView, Button, Input, Mods, Scroll, Tool};

use super::board::Surface;
use super::test_requests::Focused;
use super::{focus_editor, App, Message};
use crate::test_ipc::Request;

/// Where the widget under `note-editor` sits in the window, in logical
/// pixels, and whether it has the keyboard.
#[derive(Debug, Clone, Copy, Default)]
pub struct Placement {
    pub root: Option<Rectangle>,
    pub focused: bool,
}

impl Placement {
    fn of(focused: &Focused) -> Self {
        Self {
            root: focused
                .bounds
                .iter()
                .find(|(name, _)| *name == "note-editor")
                .map(|(_, bounds)| *bounds),
            focused: focused.named.contains(&"note-editor"),
        }
    }
}

impl App {
    pub(super) fn board_request(
        &mut self,
        request: &Arc<Request>,
    ) -> Result<Task<Message>, String> {
        if request.op != "board.get" {
            self.board
                .as_ref()
                .ok_or("The open note is not shown as a board")?;
        }
        let request = Arc::clone(request);
        Ok(
            iced::advanced::widget::operate(Focused::default()).map(move |focused| {
                Message::BoardProbed(Arc::clone(&request), Placement::of(&focused))
            }),
        )
    }

    /// Answer a `board.*` request once the widget tree has said where the
    /// board is.
    pub(super) fn board_probed(
        &mut self,
        request: &Arc<Request>,
        placement: Placement,
    ) -> Task<Message> {
        let params = &request.params;
        let (task, result) = match request.op.as_str() {
            "board.get" => {
                let elements = params
                    .get("elements")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                (
                    Task::none(),
                    Ok(self.board_snapshot(elements, placement.focused)),
                )
            }
            "board.bounds" => (Task::none(), self.board_bounds(placement)),
            "board.pointer" => match self.board_pointer(params, placement) {
                Ok(task) => (task, Ok(self.board_snapshot(false, placement.focused))),
                Err(error) => (Task::none(), Err(error)),
            },
            "board.key" => match self.board_key(params, placement) {
                Ok((task, handled)) => {
                    let mut snapshot = self.board_snapshot(false, placement.focused);
                    merge(&mut snapshot, json!({"handled": handled}));
                    (task, Ok(snapshot))
                }
                Err(error) => (Task::none(), Err(error)),
            },
            "board.type" => match self.board_type(params, placement) {
                Ok((task, handled)) => {
                    let mut snapshot = self.board_snapshot(false, placement.focused);
                    merge(&mut snapshot, json!({"handled": handled}));
                    (task, Ok(snapshot))
                }
                Err(error) => (Task::none(), Err(error)),
            },
            "board.apply" => match self.board_apply(params) {
                Ok(task) => (task, Ok(self.board_snapshot(false, placement.focused))),
                Err(error) => (Task::none(), Err(error)),
            },
            _ => (
                Task::none(),
                Err(format!("unknown test operation: {}", request.op)),
            ),
        };
        request.respond(result);
        task
    }

    /// `board.get`: what the open note's surface is, and for a board what
    /// it shows and how it is looked at. `focused` is whether the widget
    /// has the keyboard.
    pub(super) fn board_snapshot(&self, elements: bool, focused: bool) -> Value {
        let surface = self.editor_note.map(|_| match self.surface() {
            Surface::Board => "board",
            Surface::Raw => "raw",
            Surface::Text => "text",
        });
        let mut snapshot = json!({
            "enabled": self.preferences.boards,
            "surface": surface,
            "note": self.editor_note.map(|note| note.0),
            "read_only": false,
            "error": self.board_error.as_ref().filter(|_| self.board.is_none()).map(|error| json!({
                "line": error.line, "column": error.column, "message": error.message,
            })),
            "issues": [],
            "state": null,
            "tool": null,
            "selection": [],
            "cursor": null,
            "editing": null,
            "connecting": null,
            "viewport": null,
            "can_undo": false,
            "can_redo": false,
            "pristine": true,
            "revision": null,
            "counts": null,
        });
        let Some(view) = &self.board else {
            return snapshot;
        };
        let viewport = view.viewport();
        let board = view.board();
        let machine = view.machine();
        let fields = json!({
            "read_only": *view.access() != Access::Editable,
            "issues": view.issues().iter().map(|issue| json!({
                "kind": issue.kind.name(), "id": issue.id().as_str(),
            })).collect::<Vec<_>>(),
            "state": state_name(machine, focused),
            "tool": tool_name(machine.tool()),
            "selection": machine.selection().iter().map(Id::as_str).collect::<Vec<_>>(),
            "cursor": machine.cursor().map(Id::as_str),
            "editing": view.label().map(|label| json!({
                "id": label.id().as_str(), "text": label.text(), "caret": label.caret(),
            })),
            "connecting": machine.connecting().map(|connecting| json!({
                "target": connecting.target.map(|(id, _)| id.as_str()),
                "side": connecting.target.and_then(|(_, side)| side).map(Side::name),
                "query": "",
            })),
            "viewport": {"x": viewport.x, "y": viewport.y, "zoom": viewport.zoom},
            "can_undo": view.can_undo(),
            "can_redo": view.can_redo(),
            "pristine": view.pristine(),
            "revision": self.board_revision,
            "counts": {"nodes": board.nodes.len(), "edges": board.edges.len()},
        });
        merge(&mut snapshot, fields);
        if elements {
            merge(
                &mut snapshot,
                json!({
                    "nodes": board.nodes.iter().map(|(id, node)| node_json(id, node)).collect::<Vec<_>>(),
                    "edges": board.edges.iter().map(|(id, edge)| edge_json(id, edge)).collect::<Vec<_>>(),
                }),
            );
        }
        snapshot
    }

    /// `board.bounds`: where each element of the open board is drawn in
    /// the window, in logical pixels, with the colours it is drawn in.
    pub(super) fn board_bounds(&self, placement: Placement) -> Result<Value, String> {
        let view = self
            .board
            .as_ref()
            .ok_or("The open note is not shown as a board")?;
        let root = placement.root.ok_or("The board is not on screen")?;
        let viewport = view.viewport();
        let on_screen = |rect: Rect| {
            let (x, y) = viewport.to_screen((rect.x, rect.y));
            Rect::new(
                root.x + x,
                root.y + y,
                rect.width * viewport.zoom,
                rect.height * viewport.zoom,
            )
        };
        let window = Rect::new(root.x, root.y, root.width, root.height);
        let placed = wren_board::placements(view.board());
        let nodes: Vec<Value> = view
            .board()
            .nodes
            .iter()
            .filter_map(|(id, node)| {
                let at = placed.get(id)?.at;
                let source = Source::Node(id.clone());
                let box_ = on_screen(Rect::new(
                    at[0] as f32,
                    at[1] as f32,
                    node.size[0] as f32,
                    node.size[1] as f32,
                ));
                let outline = first_path(view, &source);
                Some(json!({
                    "id": id.as_str(),
                    "bounds": rect_json(box_),
                    "visible": intersection(box_, window).map(rect_json),
                    "label": label_rect(view, &source).map(|rect| rect_json(on_screen(rect))),
                    "fill": outline.and_then(|(fill, _)| fill).map(rgb),
                    "stroke": outline.and_then(|(_, stroke)| stroke).map(rgb),
                }))
            })
            .collect();
        let edges: Vec<Value> = view
            .board()
            .edges
            .keys()
            .map(|id| {
                let source = Source::Edge(id.clone());
                // The connector's line is its first item: stroked, unfilled.
                let line = view.scene().items.iter().find_map(|item| match &item.draw {
                    Draw::Path {
                        segments,
                        fill: None,
                        stroke: Some(stroke),
                    } if item.source == source => Some((segments, stroke.color)),
                    _ => None,
                });
                let points: Vec<Value> = line
                    .map(|(segments, _)| {
                        segments
                            .iter()
                            .filter_map(|segment| match *segment {
                                Segment::MoveTo(to)
                                | Segment::LineTo(to)
                                | Segment::QuadTo { to, .. }
                                | Segment::CubicTo { to, .. } => {
                                    let (x, y) = viewport.to_screen((to.x, to.y));
                                    Some(json!([root.x + x, root.y + y]))
                                }
                                Segment::Close => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                json!({
                    "id": id.as_str(),
                    "points": points,
                    "label": label_rect(view, &source).map(|rect| rect_json(on_screen(rect))),
                    "stroke": line.map(|(_, color)| rgb(color)),
                })
            })
            .collect();
        let handles: Vec<Value> = shown_handles(view)
            .into_iter()
            .map(|(id, name, (x, y))| {
                let half = handles::SIZE / 2.0;
                json!({
                    "id": id.as_str(),
                    "handle": name,
                    "bounds": rect_json(Rect::new(
                        root.x + x - half,
                        root.y + y - half,
                        handles::SIZE,
                        handles::SIZE,
                    )),
                })
            })
            .collect();
        Ok(json!({
            "root": rect_json(window),
            "viewport": {"x": viewport.x, "y": viewport.y, "zoom": viewport.zoom},
            "scale_factor": self.scale_factor,
            "nodes": nodes,
            "edges": edges,
            "handles": handles,
            "label_editor": view.label_box().map(|rect| rect_json(on_screen(rect))),
        }))
    }

    /// `board.pointer`: the input the widget would send for a wheel step
    /// or a button pressed, moved, released, clicked or double-clicked at a
    /// node's middle, one of its handles, a connector's middle or one of
    /// its ends, a board point or a window point. A move with no button
    /// down is the pointer passing over the board.
    fn board_pointer(
        &mut self,
        params: &Value,
        placement: Placement,
    ) -> Result<Task<Message>, String> {
        let view = self
            .board
            .as_ref()
            .ok_or("The open note is not shown as a board")?;
        let kind = string(params, "kind")?;
        let at = || -> Result<(f32, f32), String> {
            let at = params.get("at").ok_or("at is required")?;
            pointed(view, at, placement)
        };
        let button = || match params.get("button").and_then(Value::as_str) {
            Some("left") => Ok(Button::Left),
            Some("middle") => Ok(Button::Middle),
            Some("right") => Ok(Button::Right),
            Some(_) => Err("button is left, middle or right".to_owned()),
            None => Err("button is required".to_owned()),
        };
        let mods = params
            .get("mods")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let epoch = view.epoch();
        let mut inputs = Vec::new();
        if kind == "wheel" {
            let mut zoom = false;
            let mut sideways = false;
            for modifier in mods {
                match modifier.as_str() {
                    Some("mod") => zoom = true,
                    Some("shift") => sideways = true,
                    _ => return Err("mods are mod and shift".into()),
                }
            }
            let delta = params.get("delta").ok_or("delta is required")?;
            let component = |name: &str| match delta.get(name) {
                None => Ok(0.0),
                Some(value) => value
                    .as_f64()
                    .map(|value| value as f32)
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| format!("delta {name} must be a number")),
            };
            let (x, y) = (component("x")?, component("y")?);
            let delta = match delta.get("unit").and_then(Value::as_str).unwrap_or("lines") {
                "lines" => Scroll::Lines { x, y },
                "pixels" => Scroll::Pixels { x, y },
                _ => return Err("unit must be lines or pixels".into()),
            };
            inputs.push(Input::Wheel {
                delta,
                at: at()?,
                zoom,
                sideways,
                scale_factor: self.scale_factor,
            });
        } else {
            // The modifiers held, which the widget reports before the
            // button's input when they changed.
            let mut held = Mods::default();
            for modifier in mods {
                match modifier.as_str() {
                    Some("shift") => held.shift = true,
                    Some("duplicate") => held.duplicate = true,
                    _ => return Err("mods are shift and duplicate".into()),
                }
            }
            if held != view.machine().mods() {
                inputs.push(Input::Modifiers(held));
            }
            let clicks = match kind {
                "press" => {
                    inputs.push(Input::Press {
                        button: button()?,
                        at: at()?,
                    });
                    0
                }
                "move" => {
                    // The widget's first event after it got the keyboard
                    // says so, and only then is a pointer passing over
                    // the board's (§5.5 rule 8).
                    if placement.focused && !view.machine().focused() {
                        inputs.push(Input::Focus(true));
                    }
                    inputs.push(Input::Move { at: at()? });
                    0
                }
                "release" => {
                    inputs.push(Input::Release { button: button()? });
                    0
                }
                "click" => 1,
                "double_click" => 2,
                _ => {
                    return Err(
                        "kind must be wheel, press, move, release, click or double_click".into(),
                    )
                }
            };
            let (button, at) = if clicks > 0 {
                (Some(button()?), Some(at()?))
            } else {
                (None, None)
            };
            for _ in 0..clicks {
                if let (Some(button), Some(at)) = (button, at) {
                    inputs.push(Input::Press { button, at });
                    inputs.push(Input::Release { button });
                }
            }
        }
        // A press on the widget gives it the keyboard, as a real one does,
        // and the widget tells the machine once the press is sent.
        let pressed = inputs
            .iter()
            .position(|input| matches!(input, Input::Press { .. }));
        if let Some(index) = pressed.filter(|_| !view.machine().focused()) {
            inputs.insert(index + 1, Input::Focus(true));
        }
        let mut tasks: Vec<Task<Message>> = inputs
            .into_iter()
            .map(|input| self.board_edit(wren_canvas::Message::Input(epoch, input)))
            .collect();
        if pressed.is_some() {
            tasks.push(focus_editor());
        }
        Ok(Task::batch(tasks))
    }

    /// `board.key`: a key pressed on the focused board, as the widget and
    /// then the app's keyboard subscription would take it (§5.1). The board
    /// asks the keymap's policy through `BoardView::key`; a command goes on
    /// to `dispatch` through `board_edit`. Whether the board took the key
    /// decides what the app sees of it, as a captured event does. With an
    /// overlay open, or another widget holding the keyboard, nothing
    /// happens and the key is not handled.
    fn board_key(
        &mut self,
        params: &Value,
        placement: Placement,
    ) -> Result<(Task<Message>, bool), String> {
        let view = self
            .board
            .as_ref()
            .ok_or("The open note is not shown as a board")?;
        let pressed = pressed_key(params)?;
        if !placement.focused || self.palette.is_some() || self.dialog.is_some() {
            return Ok((Task::none(), false));
        }
        let epoch = view.epoch();
        let taken = view.key(
            &pressed.key,
            pressed.physical,
            pressed.modifiers,
            pressed.text.as_deref(),
            pressed.repeat,
            false,
        );
        let handled = taken.is_some();
        let mut messages = Vec::new();
        // The board has the keyboard, which the machine hears of with the
        // widget's first event after it got it.
        if !view.machine().focused() {
            messages.push(wren_canvas::Message::Input(epoch, Input::Focus(true)));
        }
        messages.extend(taken.into_iter().flatten());
        let mut tasks: Vec<Task<Message>> = messages
            .into_iter()
            .map(|message| self.board_edit(message))
            .collect();
        let event = iced::keyboard::Event::KeyPressed {
            key: pressed.key.clone(),
            modified_key: pressed.key,
            physical_key: pressed.physical,
            location: iced::keyboard::Location::Standard,
            modifiers: pressed.modifiers,
            text: pressed.text.map(Into::into),
            repeat: pressed.repeat,
        };
        let status = if handled {
            iced::event::Status::Captured
        } else {
            iced::event::Status::Ignored
        };
        let reaches =
            crate::keys::input(&event).filter(|input| crate::keys::reaches_app(input, status));
        if let Some(input) = reaches {
            tasks.push(self.key_input(crate::keys::with_status(input, status)));
        }
        Ok((Task::batch(tasks), handled))
    }

    /// `board.type`: text typed on the focused board, which is the open
    /// label's, and otherwise one character at a time the board's (the
    /// connect query's, §5.12 rule 3).
    fn board_type(
        &mut self,
        params: &Value,
        placement: Placement,
    ) -> Result<(Task<Message>, bool), String> {
        let view = self
            .board
            .as_ref()
            .ok_or("The open note is not shown as a board")?;
        let text = string(params, "text")?.to_owned();
        if !placement.focused || self.palette.is_some() || self.dialog.is_some() {
            return Ok((Task::none(), false));
        }
        let epoch = view.epoch();
        let messages: Vec<wren_canvas::Message> = if view.label().is_some() {
            vec![wren_canvas::Message::Label(
                epoch,
                wren_canvas::LabelAction::Insert(text),
            )]
        } else {
            text.chars()
                .map(|c| wren_canvas::Message::Input(epoch, Input::Char(c)))
                .collect()
        };
        let tasks: Vec<Task<Message>> = messages
            .into_iter()
            .map(|message| self.board_edit(message))
            .collect();
        Ok((Task::batch(tasks), true))
    }

    /// `board.apply`: edits with explicit ids, planned and applied as one
    /// undo step and one draft, for arranging a board before what a test
    /// is about. A refused edit refuses them all.
    fn board_apply(&mut self, params: &Value) -> Result<Task<Message>, String> {
        let view = self
            .board
            .as_ref()
            .ok_or("The open note is not shown as a board")?;
        let edits = params
            .get("edits")
            .and_then(Value::as_array)
            .ok_or("edits must be a list")?;
        let mut given = Given::default();
        let mut intents = Vec::new();
        for edit in edits {
            let op = string(edit, "op")?;
            intents.extend(intents_of(view, op, edit, &mut given)?);
        }
        if self.palette.is_some() || self.dialog.is_some() || self.loading.is_some() {
            return Err("The board is not taking edits now".into());
        }
        let view = self
            .board
            .as_mut()
            .ok_or("The open note is not shown as a board")?;
        let update = view.apply(&intents, &mut given, std::time::Instant::now());
        let did = self.board_did(update);
        if let Some(error) = did.failed.or(did.notice) {
            return Err(error);
        }
        Ok(did.task)
    }
}

/// The ids `board.apply` names, handed to the plan in the order its edits
/// make them; past them, generated ones.
#[derive(Default)]
struct Given {
    ids: std::collections::VecDeque<Id>,
    random: wren_board::ids::Random,
    /// The nodes the edits make, with their sizes, and the connectors, so
    /// a later edit may name them.
    nodes: std::collections::BTreeMap<Id, [i32; 2]>,
    edges: std::collections::BTreeSet<Id>,
}

impl wren_board::ids::Ids for Given {
    fn draw(&mut self) -> Id {
        self.ids.pop_front().unwrap_or_else(|| self.random.draw())
    }
}

/// The intents of one `board.apply` edit.
fn intents_of(
    view: &BoardView,
    op: &str,
    edit: &Value,
    given: &mut Given,
) -> Result<Vec<Intent>, String> {
    let board = view.board();
    let new_id = |given: &mut Given| -> Result<Id, String> {
        let id = string(edit, "id")?;
        let id = Id::new(id).ok_or("id is not a valid id")?;
        let made = given.nodes.contains_key(&id) || given.edges.contains(&id);
        if board.nodes.contains_key(&id) || board.edges.contains_key(&id) || made {
            return Err(format!("{} is taken", id.as_str()));
        }
        given.ids.push_back(id.clone());
        Ok(id)
    };
    let ids = || -> Result<Vec<Id>, String> {
        edit.get("ids")
            .and_then(Value::as_array)
            .ok_or("ids must be a list")?
            .iter()
            .map(|id| {
                id.as_str()
                    .and_then(Id::new)
                    .ok_or_else(|| "ids are ids".to_owned())
            })
            .collect()
    };
    let cell = |name: &str| -> Result<[i32; 2], String> {
        let (x, y) = pair(
            edit.get(name)
                .ok_or_else(|| format!("{name} is required"))?,
        )?;
        let whole = |v: f32| (v.fract() == 0.0 && v.abs() < 1e9).then_some(v as i32);
        match (whole(x), whole(y)) {
            (Some(x), Some(y)) => Ok([x, y]),
            _ => Err(format!("{name} is two whole numbers")),
        }
    };
    let shape = |name: &str| Shape::from_name(name).ok_or_else(|| format!("no shape {name}"));
    let endpoint = |name: &str| -> Result<Endpoint, String> {
        let end = edit
            .get(name)
            .ok_or_else(|| format!("{name} is required"))?;
        let node = end
            .get("node")
            .and_then(Value::as_str)
            .and_then(Id::new)
            .ok_or_else(|| format!("{name} names a node"))?;
        let side = match end.get("side").and_then(Value::as_str) {
            None => None,
            Some(side) => Some(Side::from_name(side).ok_or("side is top, right, bottom or left")?),
        };
        Ok(Endpoint::Node { node, side })
    };
    Ok(match op {
        "add_node" => {
            let id = new_id(given)?;
            let kind = match edit.get("kind").and_then(Value::as_str).unwrap_or("shape") {
                "shape" => NodeKind::Shape,
                "text" => NodeKind::Text,
                _ => return Err("kind is shape or text".into()),
            };
            let size = cell("size")?;
            given.nodes.insert(id.clone(), size);
            let mut intents = vec![Intent::AddNode {
                kind,
                shape: shape(edit.get("shape").and_then(Value::as_str).unwrap_or("rect"))?,
                at: cell("at")?,
                size,
                from: None,
            }];
            if let Some(text) = edit.get("text").and_then(Value::as_str) {
                intents.push(Intent::SetText {
                    id,
                    text: text.to_owned(),
                    size,
                });
            }
            intents
        }
        "move" => vec![Intent::Move {
            ids: ids()?,
            by: cell("by")?,
        }],
        "delete" => vec![Intent::Delete { ids: ids()? }],
        "connect" => {
            let id = new_id(given)?;
            given.edges.insert(id);
            vec![Intent::AddEdge {
                from: endpoint("from")?,
                to: endpoint("to")?,
            }]
        }
        "set" => {
            let id = string(edit, "id")?;
            let id = Id::new(id).ok_or("id is not a valid id")?;
            let value = string(edit, "value")?.to_owned();
            let size = board
                .nodes
                .get(&id)
                .map(|node| node.size)
                .or_else(|| given.nodes.get(&id).copied());
            let edge = board.edges.contains_key(&id) || given.edges.contains(&id);
            match (string(edit, "field")?, size) {
                ("text", Some(size)) => vec![Intent::SetText {
                    id,
                    text: value,
                    size,
                }],
                ("shape", Some(_)) => vec![Intent::SetShape {
                    ids: vec![id],
                    shape: shape(&value)?,
                }],
                ("text", None) if edge => vec![Intent::SetEdgeLabel {
                    id,
                    text: value,
                    commit: true,
                }],
                _ => return Err("set takes a node's text or shape, or a connector's text".into()),
            }
        }
        _ => return Err("op is add_node, move, set, connect or delete".into()),
    })
}

/// A key `board.key` presses: the key as iced names it, its physical key,
/// the modifiers held, the text it types and whether it repeats.
struct Pressed {
    key: iced::keyboard::Key,
    physical: iced::keyboard::key::Physical,
    modifiers: iced::keyboard::Modifiers,
    text: Option<String>,
    repeat: bool,
}

/// The key `board.key` names: a named key (`tab`, `enter`, `escape`,
/// `backspace`, `delete`, `left`, `right`, `up`, `down`, `home`, `end`,
/// `space`, `f1` to `f12`) or one character, with `mods` of `mod` (⌘ on
/// macOS, Ctrl elsewhere, as document commands use), `ctrl`, `alt`,
/// `shift` and `logo`.
fn pressed_key(params: &Value) -> Result<Pressed, String> {
    use iced::keyboard::key::{Code, Named, NativeCode, Physical};
    use iced::keyboard::{Key, Modifiers};
    let name = string(params, "key")?;
    let mut modifiers = Modifiers::empty();
    for modifier in params
        .get("mods")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        modifiers |= match modifier.as_str() {
            Some("mod") if cfg!(target_os = "macos") => Modifiers::LOGO,
            Some("mod" | "ctrl") => Modifiers::CTRL,
            Some("alt") => Modifiers::ALT,
            Some("shift") => Modifiers::SHIFT,
            Some("logo") => Modifiers::LOGO,
            _ => return Err("mods are mod, ctrl, alt, shift and logo".into()),
        };
    }
    let repeat = params
        .get("repeat")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let named = match name {
        "tab" => Some((Named::Tab, Code::Tab)),
        "enter" => Some((Named::Enter, Code::Enter)),
        "escape" => Some((Named::Escape, Code::Escape)),
        "backspace" => Some((Named::Backspace, Code::Backspace)),
        "delete" => Some((Named::Delete, Code::Delete)),
        "left" => Some((Named::ArrowLeft, Code::ArrowLeft)),
        "right" => Some((Named::ArrowRight, Code::ArrowRight)),
        "up" => Some((Named::ArrowUp, Code::ArrowUp)),
        "down" => Some((Named::ArrowDown, Code::ArrowDown)),
        "home" => Some((Named::Home, Code::Home)),
        "end" => Some((Named::End, Code::End)),
        "space" => Some((Named::Space, Code::Space)),
        "f1" => Some((Named::F1, Code::F1)),
        "f2" => Some((Named::F2, Code::F2)),
        "f3" => Some((Named::F3, Code::F3)),
        "f4" => Some((Named::F4, Code::F4)),
        "f5" => Some((Named::F5, Code::F5)),
        "f6" => Some((Named::F6, Code::F6)),
        "f7" => Some((Named::F7, Code::F7)),
        "f8" => Some((Named::F8, Code::F8)),
        "f9" => Some((Named::F9, Code::F9)),
        "f10" => Some((Named::F10, Code::F10)),
        "f11" => Some((Named::F11, Code::F11)),
        "f12" => Some((Named::F12, Code::F12)),
        _ => None,
    };
    let chord = modifiers.control() || modifiers.alt() || modifiers.logo();
    if let Some((named, code)) = named {
        let text = (named == Named::Space && !chord).then(|| " ".to_owned());
        return Ok(Pressed {
            key: Key::Named(named),
            physical: Physical::Code(code),
            modifiers,
            text,
            repeat,
        });
    }
    let mut chars = name.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return Err(format!("no key {name}"));
    };
    // Shift types a letter's capital; any other character is as given.
    let typed = if modifiers.shift() {
        c.to_uppercase().collect()
    } else {
        c.to_string()
    };
    let physical = character_code(c.to_ascii_lowercase()).map_or(
        Physical::Unidentified(NativeCode::Unidentified),
        Physical::Code,
    );
    Ok(Pressed {
        key: Key::Character(typed.as_str().into()),
        physical,
        modifiers,
        text: (!chord).then_some(typed),
        repeat,
    })
}

/// The US key that types `c` unshifted.
fn character_code(c: char) -> Option<iced::keyboard::key::Code> {
    use iced::keyboard::key::Code;
    const LETTERS: [Code; 26] = [
        Code::KeyA,
        Code::KeyB,
        Code::KeyC,
        Code::KeyD,
        Code::KeyE,
        Code::KeyF,
        Code::KeyG,
        Code::KeyH,
        Code::KeyI,
        Code::KeyJ,
        Code::KeyK,
        Code::KeyL,
        Code::KeyM,
        Code::KeyN,
        Code::KeyO,
        Code::KeyP,
        Code::KeyQ,
        Code::KeyR,
        Code::KeyS,
        Code::KeyT,
        Code::KeyU,
        Code::KeyV,
        Code::KeyW,
        Code::KeyX,
        Code::KeyY,
        Code::KeyZ,
    ];
    const DIGITS: [Code; 10] = [
        Code::Digit0,
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    match c {
        'a'..='z' => LETTERS.get(usize::from(c as u8 - b'a')).copied(),
        '0'..='9' => DIGITS.get(usize::from(c as u8 - b'0')).copied(),
        '[' => Some(Code::BracketLeft),
        ']' => Some(Code::BracketRight),
        '/' => Some(Code::Slash),
        '=' => Some(Code::Equal),
        '-' => Some(Code::Minus),
        _ => None,
    }
}

/// Where `at` points, in widget-local pixels: a node's middle, one of its
/// handles (`nw` to `w`, `connect_top` to `connect_left`, `side_top` to
/// `side_left`), a connector's middle along its drawn line or one of its
/// ends (`from`, `to`), a board point or a window point.
fn pointed(view: &BoardView, at: &Value, placement: Placement) -> Result<(f32, f32), String> {
    let viewport = view.viewport();
    let named = |key: &str| -> Result<Option<Id>, String> {
        at.get(key)
            .map(|name| {
                name.as_str()
                    .and_then(Id::new)
                    .ok_or_else(|| format!("{key} must be an id"))
            })
            .transpose()
    };
    if let Some(id) = named("node")? {
        let frame = hit::Frames::new(view.board())
            .get(&id)
            .ok_or("no such node")?;
        let Some(handle) = at.get("handle") else {
            return Ok(viewport.to_screen((frame.center().x, frame.center().y)));
        };
        let handle = handle.as_str().ok_or("handle must be a name")?;
        if let Some(side) = handle.strip_prefix("connect_") {
            let side = Side::from_name(side).ok_or("no such connect handle")?;
            return handles::connect(frame, viewport)
                .into_iter()
                .find(|(of, _)| *of == side)
                .map(|(_, at)| at)
                .ok_or_else(|| "no such connect handle".into());
        }
        if let Some(side) = handle.strip_prefix("side_") {
            let side = Side::from_name(side).ok_or("no such side handle")?;
            let node = view.board().nodes.get(&id).ok_or("no such node")?;
            return handles::sides(node, frame, viewport)
                .into_iter()
                .find(|(of, _)| *of == side)
                .map(|(_, at)| at)
                .ok_or_else(|| "no such side handle".into());
        }
        return handles::resize(frame, viewport)
            .into_iter()
            .find(|(of, _)| of.name() == handle)
            .map(|(_, at)| at)
            .ok_or_else(|| format!("the node shows no {handle} handle at this zoom"));
    }
    if let Some(id) = named("edge")? {
        if let Some(handle) = at.get("handle") {
            let [from, to] = handles::ends(view.board(), view.scene(), &id)
                .ok_or("the connector is not drawn")?;
            let end = match handle.as_str() {
                Some("from") => from,
                Some("to") => to,
                _ => return Err("a connector's handles are from and to".into()),
            };
            return Ok(viewport.to_screen((end.x, end.y)));
        }
        let points = edge_line(view, &id).ok_or("the connector is not drawn")?;
        let (x, y) = halfway(&points);
        return Ok(viewport.to_screen((x, y)));
    }
    if let Some(point) = at.get("canvas") {
        return Ok(viewport.to_screen(pair(point)?));
    }
    if let Some(point) = at.get("screen") {
        let root = placement.root.ok_or("The board is not on screen")?;
        let (x, y) = pair(point)?;
        return Ok((x - root.x, y - root.y));
    }
    Err("at must name a node, a connector, a canvas point or a screen point".into())
}

/// The points of a connector's drawn line in board units: its first
/// stroked, unfilled path.
fn edge_line(view: &BoardView, id: &Id) -> Option<Vec<(f32, f32)>> {
    let source = Source::Edge(id.clone());
    view.scene().items.iter().find_map(|item| match &item.draw {
        Draw::Path {
            segments,
            fill: None,
            stroke: Some(_),
        } if item.source == source => Some(
            segments
                .iter()
                .filter_map(|segment| match *segment {
                    Segment::MoveTo(to)
                    | Segment::LineTo(to)
                    | Segment::QuadTo { to, .. }
                    | Segment::CubicTo { to, .. } => Some((to.x, to.y)),
                    Segment::Close => None,
                })
                .collect(),
        ),
        _ => None,
    })
}

/// The point halfway along a polyline.
fn halfway(points: &[(f32, f32)]) -> (f32, f32) {
    let length = |(a, b): ((f32, f32), (f32, f32))| (b.0 - a.0).hypot(b.1 - a.1);
    let pairs = || points.iter().copied().zip(points.iter().copied().skip(1));
    let mut left = pairs().map(length).sum::<f32>() / 2.0;
    for (a, b) in pairs() {
        let step = length((a, b));
        if left <= step && step > 0.0 {
            let t = left / step;
            return (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
        }
        left -= step;
    }
    points.last().copied().unwrap_or_default()
}

/// The handles the board shows now, each with its owner, its name and its
/// centre in widget-local pixels: those of the selection and of the node
/// the pointer rests on, and the side handles of the node a connector end
/// is dragged over.
fn shown_handles(view: &BoardView) -> Vec<(Id, &'static str, (f32, f32))> {
    let viewport = view.viewport();
    let board = view.board();
    let frames = hit::Frames::new(board);
    let shown = view.machine().handles(&machine::View {
        viewport,
        now: std::time::Instant::now(),
        board,
        scene: view.scene(),
        writable: view.writable(),
    });
    let mut list = Vec::new();
    if let Some((id, frame)) = shown.resize.and_then(|id| Some((id, frames.get(id)?))) {
        for (handle, at) in handles::resize(frame, viewport) {
            list.push((id.clone(), handle.name(), at));
        }
    }
    if let Some((id, frame)) = shown.connect.and_then(|id| Some((id, frames.get(id)?))) {
        for (side, at) in handles::connect(frame, viewport) {
            let name = match side {
                Side::Top => "connect_top",
                Side::Right => "connect_right",
                Side::Bottom => "connect_bottom",
                Side::Left => "connect_left",
            };
            list.push((id.clone(), name, at));
        }
    }
    if let Some(id) = shown.ends {
        if let Some([from, to]) = handles::ends(board, view.scene(), id) {
            list.push((id.clone(), "from", viewport.to_screen((from.x, from.y))));
            list.push((id.clone(), "to", viewport.to_screen((to.x, to.y))));
        }
    }
    let target = view.machine().connecting().and_then(|connecting| {
        let (id, _) = connecting.target?;
        Some((id, board.nodes.get(id)?, frames.get(id)?))
    });
    if let Some((id, node, frame)) = target {
        for (side, at) in handles::sides(node, frame, viewport) {
            let name = match side {
                Side::Top => "side_top",
                Side::Right => "side_right",
                Side::Bottom => "side_bottom",
                Side::Left => "side_left",
            };
            list.push((id.clone(), name, at));
        }
    }
    list
}

/// The machine's state as `board.get` names it. The machine hears that the
/// board gained or lost the keyboard only with the widget's next event, so
/// at rest the widget's own focus decides between unfocused and the rest.
fn state_name(machine: &wren_canvas::Machine, focused: bool) -> &'static str {
    match machine.state() {
        State::Unfocused | State::Idle | State::Armed if !focused => "unfocused",
        State::Unfocused if machine.tool() != Tool::Select => "armed",
        State::Unfocused => "idle",
        state => state.name(),
    }
}

/// The armed tool as `board.get` names it: the last part of its command's
/// key (`select`, `hand`, `rectangle`, …, `text`, `connector`).
fn tool_name(tool: Tool) -> &'static str {
    let key = wren_canvas::Command::Tool(tool).key();
    key.strip_prefix("board.tool.").unwrap_or(key)
}

fn string<'a>(params: &'a Value, name: &str) -> Result<&'a str, String> {
    params
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn pair(value: &Value) -> Result<(f32, f32), String> {
    let number = |index: usize| {
        value
            .get(index)
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .filter(|value| value.is_finite())
    };
    match (number(0), number(1), value.as_array().map(Vec::len)) {
        (Some(x), Some(y), Some(2)) => Ok((x, y)),
        _ => Err("a point is [x, y]".into()),
    }
}

/// Add `fields` to the object `into`, replacing those already there.
fn merge(into: &mut Value, fields: Value) {
    if let (Value::Object(into), Value::Object(fields)) = (into, fields) {
        into.extend(fields);
    }
}

fn rect_json(rect: Rect) -> Value {
    json!({"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height})
}

fn rgb(color: Rgba) -> Value {
    json!([color.r, color.g, color.b])
}

/// The part of `a` inside `b`, if it has any area: `a` itself when it is
/// whole inside.
fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    if a.x >= b.x && a.y >= b.y && a.right() <= b.right() && a.bottom() <= b.bottom() {
        return (a.width > 0.0 && a.height > 0.0).then_some(a);
    }
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let (right, bottom) = (a.right().min(b.right()), a.bottom().min(b.bottom()));
    (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
}

/// The fill and stroke colours of an element's first path, its outline.
fn first_path(view: &BoardView, source: &Source) -> Option<(Option<Rgba>, Option<Rgba>)> {
    view.scene().items.iter().find_map(|item| match &item.draw {
        Draw::Path { fill, stroke, .. } if item.source == *source => {
            Some((*fill, stroke.map(|stroke| stroke.color)))
        }
        _ => None,
    })
}

/// The box of the lines of an element's label in board units: its last
/// text item, after a placeholder's caption.
fn label_rect(view: &BoardView, source: &Source) -> Option<Rect> {
    let (lines, size) = view
        .scene()
        .items
        .iter()
        .rev()
        .find_map(|item| match &item.draw {
            Draw::Text { lines, size, .. } if item.source == *source && !lines.is_empty() => {
                Some((lines, *size))
            }
            _ => None,
        })?;
    let left = lines
        .iter()
        .map(|line| line.origin.x)
        .fold(f32::INFINITY, f32::min);
    let right = lines
        .iter()
        .map(|line| line.origin.x + line.width)
        .fold(f32::NEG_INFINITY, f32::max);
    let top = lines.first()?.origin.y;
    let height = lines.len() as f32 * scene::line_height(size);
    Some(Rect::new(left, top, right - left, height))
}

fn color_json(color: &Color) -> Value {
    match color {
        Color::Token(token) => json!(token.name()),
        Color::Hex([r, g, b]) => json!(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Unknown(text) => json!(text),
    }
}

/// A node's schema-1 fields by name; unknown content is left out.
fn node_json(id: &Id, node: &Node) -> Value {
    json!({
        "id": id.as_str(),
        "text": node.text,
        "kind": node.kind.as_ref().map(|kind| kind.as_str()),
        "at": node.at,
        "parent": node.parent,
        "shape": node.shape.as_str(),
        "size": node.size,
        "style": {
            "color": color_json(&node.style.color),
            "fill": node.style.fill.as_str(),
            "stroke": node.style.stroke.as_str(),
        },
        "z": node.z,
    })
}

fn end_json(end: &End) -> Value {
    match end {
        End::Node {
            node, side, anchor, ..
        } => json!({
            "node": node,
            "side": side.as_ref().map(|side| side.as_str()),
            "anchor": anchor,
        }),
        End::Free { at, .. } => json!({"at": at}),
    }
}

/// A connector's schema-1 fields by name; unknown content is left out.
fn edge_json(id: &Id, edge: &Edge) -> Value {
    json!({
        "id": id.as_str(),
        "kind": edge.kind.as_ref().map(|kind| kind.as_str()),
        "from": end_json(&edge.from),
        "to": end_json(&edge.to),
        "heads": edge.heads.iter().map(|head| head.as_str()).collect::<Vec<_>>(),
        "labels": edge.labels.iter().map(|label| json!({"text": label.text, "at": label.at})).collect::<Vec<_>>(),
        "route": edge.route.as_str(),
        "style": {
            "color": color_json(&edge.style.color),
            "stroke": edge.style.stroke.as_str(),
        },
        "z": edge.z,
    })
}

#[cfg(test)]
mod tests {
    use iced::{Point, Size};
    use wren_core::kind::NoteKind;
    use wren_core::ops::Op;

    use super::super::board::tests as boards;
    use super::super::columns::Column;
    use super::*;

    /// The widget at (300, 40), 800 by 600, with the keyboard.
    const PLACED: Placement = Placement {
        root: Some(Rectangle {
            x: 300.0,
            y: 40.0,
            width: 800.0,
            height: 600.0,
        }),
        focused: true,
    };

    fn ask(app: &mut App, op: &str, params: Value, placement: Placement) -> Result<Value, String> {
        let (request, mut reply) = Request::detached(op, params);
        let _ = app.board_probed(&request, placement);
        reply.try_recv().unwrap()
    }

    fn lay_out(app: &mut App) {
        let _ = app.board_edit(wren_canvas::Message::Resized {
            width: 800.0,
            height: 600.0,
        });
    }

    #[test]
    fn board_get_names_the_surface_and_a_board_its_state() {
        let (_directory, mut app) = boards::fixture();
        let get = |app: &mut App, params: Value| ask(app, "board.get", params, PLACED).unwrap();
        let none = get(&mut app, json!({}));
        assert_eq!(none["enabled"], true);
        assert_eq!(none["surface"], Value::Null);
        assert_eq!(none["note"], Value::Null);

        let note = boards::note_id(&app, "Note", NoteKind::Markdown);
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        let text = get(&mut app, json!({}));
        assert_eq!(
            (text["surface"].clone(), text["note"].clone()),
            (json!("text"), json!(note.0))
        );
        assert_eq!(text["counts"], Value::Null);

        boards::open_board(&mut app, "Broken");
        let raw = get(&mut app, json!({}));
        assert_eq!(raw["surface"], "raw");
        assert_eq!(raw["error"]["line"], 1);
        assert!(raw["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()));

        boards::open_board(&mut app, "Flow");
        lay_out(&mut app);
        let board = get(&mut app, json!({}));
        assert_eq!(board["surface"], "board");
        assert_eq!(board["error"], Value::Null);
        assert_eq!(board["counts"], json!({"nodes": 2, "edges": 1}));
        assert_eq!(board["read_only"], false);
        assert_eq!(board["state"], "idle");
        assert_eq!(board["pristine"], true);
        // FLOW has no z keys: each element is flagged once.
        let issues: Vec<&str> = board["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| issue["kind"].as_str().unwrap())
            .collect();
        assert_eq!(issues, ["missing_z"; 3]);
        assert!(board.get("nodes").is_none(), "elements only when asked for");
        let unfocused = ask(&mut app, "board.get", json!({}), Placement::default()).unwrap();
        assert_eq!(unfocused["state"], "unfocused");

        let full = get(&mut app, json!({"elements": true}));
        let receive = full["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "receive")
            .unwrap();
        assert_eq!(receive["text"], "Receive order");
        assert_eq!(receive["shape"], "rect");
        assert_eq!(receive["size"], json!([120, 60]));
        assert_eq!(receive["style"]["color"], "neutral");
        let edge = &full["edges"][0];
        assert_eq!(
            edge["from"],
            json!({"node": "receive", "side": null, "anchor": null})
        );
        assert_eq!(edge["heads"], json!(["none", "arrow"]));

        // With the setting off there is no board surface to report.
        app.preferences.boards = false;
        assert_eq!(get(&mut app, json!({}))["enabled"], false);
    }

    #[test]
    fn board_bounds_places_each_element_in_the_window() {
        let (_directory, mut app) = boards::fixture();
        boards::open_board(&mut app, "Flow");
        lay_out(&mut app);
        let bounds = ask(&mut app, "board.bounds", json!({}), PLACED).unwrap();
        assert_eq!(
            bounds["root"],
            json!({"x": 300.0, "y": 40.0, "width": 800.0, "height": 600.0})
        );
        let zoom = bounds["viewport"]["zoom"].as_f64().unwrap();
        assert!(zoom > 0.0 && zoom <= 1.0, "{zoom}");
        let nodes = bounds["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
        for node in nodes {
            let (rect, visible) = (&node["bounds"], &node["visible"]);
            // Fitted: every node is whole and inside the widget.
            assert_eq!(rect, visible, "{node}");
            assert!(rect["x"].as_f64().unwrap() >= 300.0 + 39.0, "{node}");
            assert!((rect["width"].as_f64().unwrap() - 120.0 * zoom).abs() < 1e-3);
            let label = &node["label"];
            assert!(label["width"].as_f64().unwrap() > 0.0, "{node}");
            assert!(label["x"].as_f64().unwrap() >= rect["x"].as_f64().unwrap());
            assert!(
                node["fill"].is_array() && node["stroke"].is_array(),
                "{node}"
            );
        }
        let edge = &bounds["edges"][0];
        let points = edge["points"].as_array().unwrap();
        assert!(points.len() >= 2, "{edge}");
        assert!(edge["stroke"].is_array());
        assert_eq!(edge["label"], Value::Null);

        // Not mounted, not a board.
        assert!(ask(&mut app, "board.bounds", json!({}), Placement::default()).is_err());
        let note = boards::note_id(&app, "Note", NoteKind::Markdown);
        let _ = app.apply_core(Op::OpenNote { note: Some(note) }).unwrap();
        let (request, mut reply) = Request::detached("board.bounds", json!({}));
        let _ = app.handle_test(request);
        assert!(reply.try_recv().unwrap().unwrap_err().contains("board"));
    }

    #[test]
    fn board_pointer_wheels_and_drags_through_the_widgets_path() {
        let (directory, mut app) = boards::fixture();
        boards::open_board(&mut app, "Flow");
        lay_out(&mut app);
        app.column = Column::Notes;
        let viewport = |app: &App| app.board.as_ref().unwrap().viewport();
        let pointer = |app: &mut App, params: Value| ask(app, "board.pointer", params, PLACED);

        // A wheel line pans 40 px and leaves the column.
        let start = viewport(&app);
        pointer(
            &mut app,
            json!({"kind": "wheel", "at": {"canvas": [0, 0]}, "delta": {"y": -1}}),
        )
        .unwrap();
        let panned = viewport(&app);
        assert!(
            (panned.y - (start.y + 40.0 / start.zoom)).abs() < 1e-3,
            "{panned:?}"
        );
        assert_eq!(app.column, Column::Notes);

        // After the burst, Mod+wheel zooms about the pointer.
        std::thread::sleep(wren_canvas::machine::BURST);
        let at = json!({"screen": [700, 340]});
        let local = (400.0, 300.0);
        let under = panned.to_board(local);
        let reply = pointer(
            &mut app,
            json!({"kind": "wheel", "at": at, "mods": ["mod"], "delta": {"y": 1}}),
        )
        .unwrap();
        let zoomed = viewport(&app);
        assert!((zoomed.zoom - panned.zoom * 1.1).abs() < 1e-4, "{zoomed:?}");
        let (x, y) = zoomed.to_board(local);
        assert!((x - under.0).abs() < 1e-2 && (y - under.1).abs() < 1e-2);
        assert_eq!(
            reply["viewport"]["zoom"].as_f64().unwrap() as f32,
            zoomed.zoom
        );

        // A middle drag pans and takes the column.
        pointer(
            &mut app,
            json!({"kind": "press", "button": "middle", "at": {"node": "receive"}}),
        )
        .unwrap();
        assert_eq!(app.column, Column::Editor);
        let held = pointer(&mut app, json!({"kind": "get"}));
        assert!(held.is_err());
        let state = ask(&mut app, "board.get", json!({}), PLACED).unwrap();
        assert_eq!(state["state"], "panning");
        let before = viewport(&app);
        let centre = before.to_screen((60.0, 30.0));
        pointer(
            &mut app,
            json!({"kind": "move", "at": {"screen": [300.0 + centre.0 + 50.0, 40.0 + centre.1]}}),
        )
        .unwrap();
        pointer(&mut app, json!({"kind": "release", "button": "middle"})).unwrap();
        let after = viewport(&app);
        assert!(
            (after.x - (before.x - 50.0 / before.zoom)).abs() < 1e-3,
            "{after:?}"
        );
        assert!(!app.board.as_ref().unwrap().machine().panning());

        for params in [
            json!({"kind": "tap", "button": "left", "at": {"node": "receive"}}),
            json!({"kind": "press", "button": "fourth", "at": {"node": "receive"}}),
            json!({"kind": "press", "at": {"node": "receive"}}),
            json!({"kind": "press", "button": "left", "at": {"node": "receive", "handle": "x"}}),
            json!({"kind": "press", "button": "left", "at": {"node": "receive"}, "mods": ["mod"]}),
            json!({"kind": "click", "button": "left", "at": {"edge": "nobody"}}),
            json!({"kind": "wheel", "at": {"node": "nobody"}, "delta": {"y": 1}}),
            json!({"kind": "wheel", "at": {"canvas": [0]}, "delta": {"y": 1}}),
            json!({"kind": "wheel", "at": {"canvas": [0, 0]}, "mods": ["alt"], "delta": {"y": 1}}),
            json!({"kind": "wheel", "at": {"canvas": [0, 0]}, "delta": {"y": 1, "unit": "pages"}}),
        ] {
            assert!(pointer(&mut app, params.clone()).is_err(), "{params}");
        }
        // Looking around changes nothing in the Session or on disk.
        let document = app.session.as_ref().unwrap().document().unwrap();
        assert!(!document.dirty());
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Boards/Samples/Flow.wren-board"))
                .unwrap(),
            boards::FLOW
        );
    }

    #[test]
    fn placement_reads_the_editor_widget_from_the_focus_report() {
        let bounds = Rectangle::new(Point::new(1.0, 2.0), Size::new(3.0, 4.0));
        let focused = Focused {
            named: vec!["note-editor"],
            unnamed: 0,
            bounds: vec![
                ("filter-notes", Rectangle::default()),
                ("note-editor", bounds),
            ],
        };
        let placement = Placement::of(&focused);
        assert_eq!(placement.root, Some(bounds));
        assert!(placement.focused);
        assert!(!Placement::of(&Focused::default()).focused);
    }

    fn key(app: &mut App, params: Value) -> Value {
        ask(app, "board.key", params, PLACED).unwrap()
    }

    fn pointer(app: &mut App, params: Value) -> Value {
        ask(app, "board.pointer", params, PLACED).unwrap()
    }

    fn node_at(app: &App, id: &str) -> [i32; 2] {
        let board = app.board.as_ref().unwrap().board();
        board.nodes[&Id::new(id).unwrap()].at
    }

    #[test]
    fn board_key_takes_the_path_a_real_key_takes() {
        let (_directory, mut app) = boards::fixture();
        boards::open_board(&mut app, "Flow");
        lay_out(&mut app);
        // A bare tool letter arms the tool, with a command through dispatch.
        let armed = key(&mut app, json!({"key": "r"}));
        assert_eq!(armed["handled"], true);
        assert_eq!(
            (armed["state"].clone(), armed["tool"].clone()),
            (json!("armed"), json!("rectangle"))
        );
        // Escape is a rung of the ladder: the board's, and the tool goes.
        let escaped = key(&mut app, json!({"key": "escape"}));
        assert_eq!(
            (escaped["handled"].clone(), escaped["tool"].clone()),
            (json!(true), json!("select"))
        );
        // On the last rung Escape is the app's, which keeps the board.
        let last = key(&mut app, json!({"key": "escape"}));
        assert_eq!(last["handled"], false);
        assert_eq!(app.column, Column::Editor);

        // An arrow walks from nothing to the top node; Alt+Arrow moves it
        // one cell and drafts once.
        let walked = key(&mut app, json!({"key": "down"}));
        assert_eq!(walked["selection"], json!(["receive"]));
        assert_eq!(walked["cursor"], "receive");
        let revision = walked["revision"].as_u64().unwrap();
        let moved = key(&mut app, json!({"key": "right", "mods": ["alt"]}));
        assert_eq!(node_at(&app, "receive"), [20, 0]);
        assert_eq!(moved["revision"].as_u64().unwrap(), revision + 1);
        assert_eq!(
            (moved["can_undo"].clone(), moved["pristine"].clone()),
            (json!(true), json!(false))
        );
        assert!(boards::session_text(&app).contains("at = [20, 0]"));
        let undone = key(&mut app, json!({"key": "z", "mods": ["mod"]}));
        assert_eq!(node_at(&app, "receive"), [0, 0]);
        assert_eq!(
            (undone["can_redo"].clone(), undone["pristine"].clone()),
            (json!(true), json!(true))
        );
        assert_eq!(boards::session_text(&app), boards::FLOW);

        // Enter opens the label, whose keys are its text; Ctrl+Enter
        // commits it.
        let editing = key(&mut app, json!({"key": "enter"}));
        assert_eq!(editing["state"], "editing");
        assert_eq!(editing["editing"]["id"], "receive");
        let typed = key(&mut app, json!({"key": "r"}));
        assert_eq!(
            (typed["editing"]["text"].clone(), typed["tool"].clone()),
            (json!("r"), json!("select"))
        );
        let done = key(&mut app, json!({"key": "enter", "mods": ["mod"]}));
        assert_eq!(done["state"], "idle");
        assert_eq!(done["editing"], Value::Null);

        // Not focused, or under the palette, the board is not given keys.
        let unfocused = ask(
            &mut app,
            "board.key",
            json!({"key": "r"}),
            Placement::default(),
        )
        .unwrap();
        assert_eq!(
            (unfocused["handled"].clone(), unfocused["tool"].clone()),
            (json!(false), json!("select"))
        );
        app.open_palette(super::super::palette::Mode::Commands);
        assert_eq!(key(&mut app, json!({"key": "r"}))["handled"], false);
        assert_eq!(app.board.as_ref().unwrap().machine().tool(), Tool::Select);

        for params in [
            json!({"key": "nokey"}),
            json!({"key": "r", "mods": ["hyper"]}),
            json!({}),
        ] {
            assert!(
                ask(&mut app, "board.key", params.clone(), PLACED).is_err(),
                "{params}"
            );
        }
    }

    #[test]
    fn board_type_fills_the_open_label_and_drafts_each_change() {
        let (directory, mut app) = boards::fixture();
        boards::open_board(&mut app, "Flow");
        lay_out(&mut app);
        pointer(
            &mut app,
            json!({"kind": "double_click", "button": "left", "at": {"node": "pack"}}),
        );
        let start = ask(&mut app, "board.get", json!({}), PLACED).unwrap();
        assert_eq!(start["editing"]["text"], "Pack");
        let typed = ask(&mut app, "board.type", json!({"text": "Ship it"}), PLACED).unwrap();
        assert_eq!(typed["handled"], true);
        // The label opened with its text selected, so typing replaced it.
        assert_eq!(typed["editing"]["text"], "Ship it");
        assert_eq!(typed["editing"]["caret"], 7);
        assert!(typed["revision"].as_u64() > start["revision"].as_u64());
        assert!(boards::session_text(&app).contains("text = \"Ship it\""));
        // Nothing is on disk before autosave runs.
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Boards/Samples/Flow.wren-board"))
                .unwrap(),
            boards::FLOW
        );
        let unfocused = ask(
            &mut app,
            "board.type",
            json!({"text": "x"}),
            Placement::default(),
        )
        .unwrap();
        assert_eq!(unfocused["handled"], false);
        assert_eq!(unfocused["editing"]["text"], "Ship it");
    }

    #[test]
    fn board_pointer_clicks_drags_and_shows_the_handles() {
        let (_directory, mut app) = boards::fixture();
        boards::open_board(&mut app, "Flow");
        lay_out(&mut app);
        app.column = Column::Notes;
        let clicked = pointer(
            &mut app,
            json!({"kind": "click", "button": "left", "at": {"node": "pack"}}),
        );
        assert_eq!(app.column, Column::Editor);
        assert_eq!(clicked["selection"], json!(["pack"]));
        assert_eq!(clicked["state"], "idle");
        let bounds = ask(&mut app, "board.bounds", json!({}), PLACED).unwrap();
        let names: Vec<&str> = bounds["handles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|handle| handle["handle"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["nw", "n", "ne", "e", "se", "s", "sw", "w"]);
        let se = &bounds["handles"][4];
        assert_eq!(se["id"], "pack");
        assert_eq!(se["bounds"]["width"], 8.0);
        assert_eq!(bounds["label_editor"], Value::Null);

        // Shift adds to the selection; a click on a connector selects it.
        let both = pointer(
            &mut app,
            json!({"kind": "click", "button": "left", "at": {"node": "receive"}, "mods": ["shift"]}),
        );
        assert_eq!(both["selection"], json!(["pack", "receive"]));
        assert_eq!(both["cursor"], "receive");
        let edge = pointer(
            &mut app,
            json!({"kind": "click", "button": "left", "at": {"edge": "then"}}),
        );
        assert_eq!(edge["selection"], json!(["then"]));
        let ends: Vec<Value> = ask(&mut app, "board.bounds", json!({}), PLACED).unwrap()["handles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|handle| handle["handle"].clone())
            .collect();
        assert_eq!(ends, [json!("from"), json!("to")]);

        // A drag drafts once, at the release.
        let zoom = app.board.as_ref().unwrap().viewport().zoom;
        let revision = pointer(
            &mut app,
            json!({"kind": "press", "button": "left", "at": {"node": "pack"}}),
        )["revision"]
            .as_u64()
            .unwrap();
        let start = app
            .board
            .as_ref()
            .unwrap()
            .viewport()
            .to_screen((60.0, 230.0));
        let to = json!({"screen": [300.0 + start.0 + 40.0 * zoom, 40.0 + start.1]});
        let moving = pointer(&mut app, json!({"kind": "move", "at": to}));
        assert_eq!(moving["state"], "moving");
        assert_eq!(node_at(&app, "pack"), [0, 200]);
        assert_eq!(moving["revision"].as_u64().unwrap(), revision);
        let released = pointer(&mut app, json!({"kind": "release", "button": "left"}));
        assert_eq!(released["state"], "idle");
        assert_eq!(node_at(&app, "pack"), [40, 200]);
        assert_eq!(released["revision"].as_u64().unwrap(), revision + 1);

        // A double-click opens the label, which board.bounds places.
        pointer(
            &mut app,
            json!({"kind": "double_click", "button": "left", "at": {"node": "receive"}}),
        );
        let bounds = ask(&mut app, "board.bounds", json!({}), PLACED).unwrap();
        let editor = &bounds["label_editor"];
        let receive = bounds["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "receive")
            .unwrap()["bounds"]
            .clone();
        let inside = |rect: &Value, around: &Value| {
            let f = |value: &Value, key: &str| value[key].as_f64().unwrap();
            f(rect, "x") >= f(around, "x")
                && f(rect, "x") + f(rect, "width") <= f(around, "x") + f(around, "width") + 0.5
        };
        assert!(inside(editor, &receive), "{editor} {receive}");
        assert_eq!(bounds["handles"], json!([]));
    }

    #[test]
    fn board_pointer_draws_and_re_attaches_a_connector() {
        let (_directory, mut app) = boards::fixture();
        boards::open_board(&mut app, "Flow");
        lay_out(&mut app);
        pointer(
            &mut app,
            json!({"kind": "click", "button": "left", "at": {"canvas": [400, 400]}}),
        );
        let handles = |app: &mut App| -> Vec<(String, String)> {
            let bounds = ask(app, "board.bounds", json!({}), PLACED).unwrap();
            let handles = bounds["handles"].as_array().unwrap().iter();
            handles
                .map(|handle| {
                    let text = |key: &str| handle[key].as_str().unwrap().to_owned();
                    (text("id"), text("handle"))
                })
                .collect()
        };
        // The pointer passing over is nobody acting on the board: the
        // keyboard stays where it is, and nothing is drafted.
        app.column = Column::Notes;
        let over = json!({"kind": "move", "at": {"node": "receive"}});
        let resting = pointer(&mut app, over.clone());
        assert_eq!(app.column, Column::Notes);
        assert_eq!(
            (resting["state"].clone(), resting["revision"].clone()),
            (json!("idle"), json!(0))
        );
        assert!(handles(&mut app).is_empty());
        // Rested there, the node shows its four connect handles.
        std::thread::sleep(wren_canvas::machine::HOVER_DELAY);
        pointer(&mut app, over);
        let shown = handles(&mut app);
        let names: Vec<&str> = shown.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "connect_top",
                "connect_right",
                "connect_bottom",
                "connect_left"
            ]
        );
        assert!(shown.iter().all(|(id, _)| id == "receive"));
        assert_eq!(app.column, Column::Notes);

        // A drag from one onto a side handle of another node.
        let pressed = pointer(
            &mut app,
            json!({"kind": "press", "button": "left", "at": {"node": "receive", "handle": "connect_right"}}),
        );
        assert_eq!(pressed["state"], "pressed");
        assert_eq!(app.column, Column::Editor);
        let dragged = pointer(
            &mut app,
            json!({"kind": "move", "at": {"node": "pack", "handle": "side_right"}}),
        );
        assert_eq!(dragged["state"], "connecting");
        assert_eq!(
            dragged["connecting"],
            json!({"target": "pack", "side": "right", "query": ""})
        );
        assert_eq!(dragged["counts"]["edges"], 1);
        assert_eq!(dragged["revision"], 0);
        let names: Vec<String> = handles(&mut app)
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        assert_eq!(
            names,
            ["side_top", "side_right", "side_bottom", "side_left"]
        );
        let released = pointer(&mut app, json!({"kind": "release", "button": "left"}));
        assert_eq!(released["state"], "idle");
        assert_eq!(released["connecting"], Value::Null);
        assert_eq!(released["counts"]["edges"], 2);
        assert_eq!(released["revision"], 1);
        let made = released["selection"][0].as_str().unwrap().to_owned();
        assert!(boards::session_text(&app).contains("to = { node = \"pack\", side = \"right\" }"));

        // The new connector is selected, so it shows its ends; its `to`
        // end dragged to the middle of the node floats.
        assert_eq!(
            handles(&mut app),
            [
                (made.clone(), "from".to_owned()),
                (made.clone(), "to".to_owned())
            ]
        );
        pointer(
            &mut app,
            json!({"kind": "press", "button": "left", "at": {"edge": made, "handle": "to"}}),
        );
        let moved = pointer(&mut app, json!({"kind": "move", "at": {"node": "pack"}}));
        assert_eq!(
            moved["connecting"],
            json!({"target": "pack", "side": null, "query": ""})
        );
        let released = pointer(&mut app, json!({"kind": "release", "button": "left"}));
        assert_eq!(released["revision"], 2);
        assert_eq!(released["selection"], json!([made]));
        assert!(!boards::session_text(&app).contains("side ="));
        // Two gestures, two undo steps, back to the board as it opened.
        use super::super::commands::{Command, CommandId};
        for _ in 0..2 {
            let undo = CommandId::Board(wren_canvas::Command::Undo);
            let _ = app.dispatch(Command::Run(undo));
        }
        let view = app.board.as_ref().unwrap();
        assert!(view.pristine() && !view.can_undo());
        assert_eq!(boards::session_text(&app), boards::FLOW);
    }

    #[test]
    fn board_apply_is_one_step_and_one_draft_with_the_ids_it_names() {
        let (_directory, mut app) = boards::fixture();
        boards::open_board(&mut app, "Flow");
        lay_out(&mut app);
        let apply =
            |app: &mut App, edits: Value| ask(app, "board.apply", json!({"edits": edits}), PLACED);
        let applied = apply(
            &mut app,
            json!([
                {"op": "add_node", "id": "n9", "shape": "diamond", "at": [0, 400], "size": [120, 60], "text": "Nine"},
                {"op": "move", "ids": ["n9"], "by": [20, 0]},
                {"op": "set", "id": "pack", "field": "text", "value": "Packed"},
                {"op": "connect", "id": "e9", "from": {"node": "pack"}, "to": {"node": "n9"}},
                {"op": "set", "id": "e9", "field": "text", "value": "then"},
            ]),
        )
        .unwrap();
        assert_eq!(applied["revision"], 1);
        assert_eq!(applied["counts"], json!({"nodes": 3, "edges": 2}));
        assert_eq!(applied["selection"], json!([]));
        assert_eq!(node_at(&app, "n9"), [20, 400]);
        let text = boards::session_text(&app);
        assert!(
            text.contains("[node.n9]") && text.contains("[edge.e9]") && text.contains("\"Packed\"")
        );
        let undone = key(&mut app, json!({"key": "z", "mods": ["mod"]}));
        assert_eq!(
            (undone["pristine"].clone(), undone["can_undo"].clone()),
            (json!(true), json!(false))
        );
        assert_eq!(boards::session_text(&app), boards::FLOW);

        for edits in [
            json!([{"op": "add_node", "id": "pack", "at": [0, 0], "size": [120, 60]}]),
            json!([{"op": "add_node", "id": "x", "at": [0.5, 0], "size": [120, 60]}]),
            json!([{"op": "add_node", "id": "x", "shape": "star", "at": [0, 0], "size": [120, 60]}]),
            json!([{"op": "set", "id": "then", "field": "shape", "value": "rect"}]),
            json!([{"op": "spin", "ids": ["pack"]}]),
            json!({"op": "move"}),
        ] {
            assert!(apply(&mut app, edits.clone()).is_err(), "{edits}");
        }
        assert_eq!(boards::session_text(&app), boards::FLOW);
    }
}
