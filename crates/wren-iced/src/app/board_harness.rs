//! The board requests of the test harness (discovery/boards.md §6.2):
//! `board.get`, `board.bounds` and `board.pointer`. Each first asks the
//! widget tree where the widget under `note-editor` sits and whether it has
//! the keyboard, then answers in [`App::board_probed`].

use std::sync::Arc;

use iced::{Rectangle, Task};
use serde_json::{json, Value};
use wren_board::scene::{self, Draw, Rect, Segment, Source};
use wren_board::{Access, Color, Edge, End, Id, Node, Rgba, Vocabulary};
use wren_canvas::{BoardView, Button, Input, Scroll};

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
        // Phase 1 has no edit: the model is always the one loaded.
        let fields = json!({
            "read_only": *view.access() != Access::Editable,
            "issues": view.issues().iter().map(|issue| json!({
                "kind": issue.kind.name(), "id": issue.id().as_str(),
            })).collect::<Vec<_>>(),
            "state": if view.machine().panning() {
                "panning"
            } else if focused {
                "idle"
            } else {
                "unfocused"
            },
            "tool": "select",
            "viewport": {"x": viewport.x, "y": viewport.y, "zoom": viewport.zoom},
            "revision": view.epoch(),
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
        Ok(json!({
            "root": rect_json(window),
            "viewport": {"x": viewport.x, "y": viewport.y, "zoom": viewport.zoom},
            "scale_factor": self.scale_factor,
            "nodes": nodes,
            "edges": edges,
            "handles": [],
            "label_editor": null,
        }))
    }

    /// `board.pointer`: the input a wheel step or a middle-button drag
    /// would give the widget, through the path the widget's own takes.
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
            let viewport = view.viewport();
            if let Some(name) = at.get("node") {
                if at.get("handle").is_some() {
                    return Err("handles arrive in Phase 2".into());
                }
                let id = name
                    .as_str()
                    .and_then(Id::new)
                    .ok_or("node must be a node id")?;
                let node = view.board().nodes.get(&id).ok_or("no such node")?;
                let placed = wren_board::placements(view.board());
                let corner = placed.get(&id).ok_or("the node is not placed")?.at;
                let centre = (
                    corner[0] as f32 + node.size[0] as f32 / 2.0,
                    corner[1] as f32 + node.size[1] as f32 / 2.0,
                );
                Ok(viewport.to_screen(centre))
            } else if let Some(point) = at.get("canvas") {
                Ok(viewport.to_screen(pair(point)?))
            } else if let Some(point) = at.get("screen") {
                let root = placement.root.ok_or("The board is not on screen")?;
                let (x, y) = pair(point)?;
                Ok((x - root.x, y - root.y))
            } else {
                Err("at must name a node, a canvas point or a screen point".into())
            }
        };
        let button = || match params.get("button").and_then(Value::as_str) {
            Some("middle") => Ok(Button::Middle),
            Some(_) => Err("Phase 1 takes only the middle button".to_owned()),
            None => Err("button is required".to_owned()),
        };
        let input = match kind {
            "wheel" => {
                let mut zoom = false;
                let mut sideways = false;
                for modifier in params
                    .get("mods")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
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
                Input::Wheel {
                    delta,
                    at: at()?,
                    zoom,
                    sideways,
                    scale_factor: self.scale_factor,
                }
            }
            "press" => Input::Press {
                button: button()?,
                at: at()?,
            },
            "move" => Input::Move { at: at()? },
            "release" => Input::Release { button: button()? },
            "click" | "double_click" => return Err("clicks arrive in Phase 2".into()),
            _ => return Err("kind must be wheel, press, move or release".into()),
        };
        let epoch = view.epoch();
        self.board_input(wren_canvas::Message::Input(epoch, input));
        // A press on the widget gives it the keyboard, as a real one does.
        Ok(if matches!(input, Input::Press { .. }) {
            focus_editor()
        } else {
            Task::none()
        })
    }
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
        app.board_input(wren_canvas::Message::Resized {
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
            json!({"kind": "click", "button": "left", "at": {"node": "receive"}}),
            json!({"kind": "press", "button": "left", "at": {"node": "receive"}}),
            json!({"kind": "wheel", "at": {"edge": "then"}, "delta": {"y": 1}}),
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
}
