# Board controls: a mouse path and visible keys

Status: design accepted by the owner on 2026-11-18; build in progress on the
branch `board-controls`. Nothing here is in `docs/` until its step lands.
The working prototype the decisions were taken against is the private
artifact "Wren Board Controls".

## 1. Why

A board is driven almost entirely from the keyboard. Arming a shape tool,
changing a colour, a shape, a fill or a connector style, undo, and zoom each
have a key or a palette row and no direct mouse path, and nothing on the
board says which key does what. The board also sets no cursor, so the arrow
shows while panning and over resize handles. The first design mock had a
toolbar strip; the build plan (`discovery/boards.md`) never carried one.

Nothing existing is removed. Every key and gesture in
`docs/reference/boards.md` stays as it is, Space with a drag included.

## 2. Decisions

| # | Decision | Taken |
|---|---|---|
| D1 | Tools live in a floating dock at the bottom centre of the canvas. | Owner, 2026-11-18 |
| D2 | The style bar is fixed at the top centre of the canvas; it does not follow the selection. | Owner, 2026-11-18 |
| D3 | Tools stay one-shot. Arming a tool that is already armed locks it until Esc or V. The hand tool stays armed until another tool is picked (a change to shipped behaviour). | Owner, 2026-11-18 |
| D4 | The last colour picked sticks for new shapes until another is picked. | Owner, 2026-11-18 (comment) |
| D5 | Right-click keeps opening the actions palette. A menu at the pointer is revisited after the bars land. | Owner, 2026-11-18 |
| D6 | Settings `board-toolbar` and `board-hints`, both on by default. | Owner, 2026-11-18 |
| D7 | Pinch to zoom on macOS only, as the last step; Linux stays on Ctrl+scroll. | Owner, 2026-11-18 |
| D9 | Key badges on the buttons are hidden by default and show while Space is held. | Owner, 2026-11-18 (comment) |
| D10 | Text size for a shape's label and for free text (§3.9). Stored as a number, not a name; bold, italic and Markdown labels are future scope. | Owner, 2026-11-18; the particulars of §3.9 accepted the same day |

Assumed by the design and open to the owner (stated back, not contradicted):

1. Shapes added with Tab or Mod+Arrow take the sticky colour; new
   connectors stay neutral; recolouring a selected shape updates the sticky
   colour; recolouring only connectors does not.
2. The sticky colour is view state for the running app, like the armed tool
   and the zoom. It is never written to the board file (schema 1 is locked)
   and never an undo step. Fill does not stick: a new shape has the tint
   fill.
3. A third setting, `board-key-badges = hold | on | off`, default `hold`.

## 3. The pieces

### 3.1 Tool dock

One bar: Select, Hand, the six shapes that have keys (rectangle, ellipse,
diamond, pill, cylinder, parallelogram), a menu holding Rounded rectangle
and Document, Text, Connector, and a button for the keys sheet.

- A click arms the tool exactly as its key does (`board.tool.*` through
  `App::dispatch`). The armed tool's button is lit; a key press lights it
  the same way.
- Arming the armed tool toggles its lock, shown as a dot on the button.
  A locked tool stays armed after it places a shape. Esc or V unlocks and
  returns to Select. Select and Hand have no lock.
- The hand tool no longer drops back to Select after one drag.
- The icons stay in the theme's text colour whatever the sticky colour
  (§3.2) is; the owner turned down tinting them. The "New shapes" row is
  where that colour shows.
- Below about 560 px of canvas width the six shapes fold into one button
  showing the last shape used, with a menu of all eight.
- The armed-tool chip the canvas shows today at the top left goes away when
  the dock is on, since the dock says the same thing.

As built: the dock is `app/board_dock.rs`, whose `dock` decides the
buttons, what is lit and locked, the badges and the fold as a pure
function of the machine and the settings. On a read-only board it holds
Select, Hand and the keys button only. A click with a label open commits
the label, as the command it runs does, then arms the tool. A canvas too
narrow for even the folded dock shows none. A fit leaves room for the dock
(`BoardView::set_covered`), so a fitted board ends above it. The canvas
drops the tool chip through `Style::tool_chip`. The machine's lock is
`Machine::locked`; a locked tool keeps its tool while the label of what it
made is open, and a press off the label commits it and makes the next.
`board.dock` reports the dock with each button's place.

### 3.2 Style bar

Shown at the top centre when something is selected, built from
`BoardView::offers()`:

- Shapes selected: a shape menu (current shape marked), the eight colour
  swatches (current one ringed), fill none/tint/solid, dashed.
- Connectors selected: swatches, elbow/straight, start head, end head,
  reverse, dashed.
- A last menu: Duplicate, Bring to front, Send to back, Delete, and
  "All actions…", which opens the actions palette.
- Hovering a swatch, a fill, a route or a shape row previews it on the
  board through `set_patch`, as the actions palette's rows do; a click
  applies it as one undo step; leaving drops the preview.
- With nothing selected the bar is hidden, except while a shape or text
  tool is armed: then it shows the colour row alone, labelled "New shapes",
  with the sticky colour ringed.
- With nothing selected, `0` to `7` set the sticky colour.

As built: the bar is `app/board_style_bar.rs`, whose `bar` is a pure
function of `BoardView::offers()` (which controls show and which are
current), the armed tool, `BoardView::next_color` and the settings. The
connector toggles (route, heads, dashed) are never shown lit, since
`offers` marks no toggle current. Hover previews go through
`App::style_bar_patch` into `set_patch` from `sync_actions`, so a palette, a
dialog, an open label or a command drops them. Where the whole bar does not
fit, the fills, dashed and the connector controls fold into the top of the
last menu; the New shapes row drops its label, then itself. A fit leaves
room along the top as along the bottom (`BoardView::set_covered(top,
bottom)`). The sticky colour is `BoardView::next_color`, kept in
`SavedView` with the viewport; the canvas recolours the new nodes of an
`AddNode` or `Extend` plan, so `wren-board` is unchanged. The keys sheet no
longer dims the colour rows with nothing selected. `board.style_bar`
reports the bar and `board.get` reports `next_color`.

### 3.3 Key badges

Each dock button shows its key in a corner and each swatch its digit, only
while Space is held on a focused board (D9). Space is already the
hold-to-pan key, so the board knows when it is down. A button's tooltip
names its key at all times.

### 3.4 Hint line

The footer names the keys that fit the current state, between the counts
and the zoom. It extends the hint an empty board shows today.

| State | Hint |
|---|---|
| Nothing selected | Tab add a shape · R O D U Y P pick a shape · Space show keys · ? all keys |
| One shape | Tab next shape · Mod+Arrow connect · Enter label · 0–7 colour · Shift+D reshape · / actions |
| Several shapes | 0–7 colour · Alt+Arrow move · / align and distribute · Backspace delete |
| Connectors only | 0–7 colour · Enter label · / route, heads, reverse · Backspace delete |
| A shape or text tool armed | Click to place · drag to size · Tab add it after the selection · its key again to lock · Esc cancel |
| Connector tool | Drag from one shape to another · Esc cancel |
| Hand tool or Space held | Drag to pan |

The keys shown come from the live keymap, so a rebound key shows as
rebound and an unbound command drops out of the line. "Space show keys"
shows only while Space shows the dock's keys (`board-toolbar = on` and
`board-key-badges = hold`); "its key again to lock" names the armed
tool's key (`D again to lock`), reads "again to unlock" while it is
locked, and drops out for the two shapes with no key. The decision is
`app/board_keys.rs`'s `hint`; `board.get` reports it as `hint`.

### 3.5 Keys sheet

`?` (Shift+/, unbound today) or the dock's last button opens every board
key on one sheet over the canvas, grouped as `docs/reference/boards.md`
groups them. Rows that need a different selection are dimmed. Esc or `?`
closes it. Built from the keymap and `BOARD_DEFINITIONS`. `/` keeps the
actions palette. New command: `board.keys`.

### 3.6 Footer controls

The zoom figure becomes three controls: zoom out, the figure (zoom to
fit), zoom in. Undo and redo buttons sit beside them, greyed when there is
nothing to undo or redo.

As built: `app/board_footer.rs`. The footer is laid as a layer over the
editor column, which keeps its room, so a press on one of its buttons is
taken before the canvas hears it and the board keeps the keyboard and its
armed tool. The layers are `layers.rs`, not iced's `Stack`, which stops at
a capture made before it: a press a browser column's filter took never
reached the canvas under the dock, which kept the keyboard beside the
filter. Zoom out and in run `font.decrease` and `font.increase`, as
their keys do on a board. `board.footer` reports the controls.

### 3.7 Cursors

The canvas widget gains a cursor rule, a pure function of the interaction
state:

| State | Cursor |
|---|---|
| Select tool over canvas, shapes and connectors | arrow |
| Hand tool armed, or Space held | open hand (`Grab`) |
| A pan in progress: Space-drag, middle-drag, hand-drag | closed hand (`Grabbing`) |
| A shape, text or connector tool armed; over a connect handle | crosshair |
| Corner resize handles | the diagonal resize of that corner |
| Side resize handles | horizontal or vertical resize |
| Inside an open label | I-beam |
| Dragging with the copy modifier | copy |

A read-only board arms no editing tool and shows no handles, so it has the
arrow and the two hands only. The rule is `Machine::pointer`, with the
I-beam added by the widget, which knows where the open label is; the
harness reports it as `board.get`'s `pointer`.

### 3.8 Pinch to zoom (macOS)

winit 0.30.13 emits `WindowEvent::PinchGesture` on macOS (and iOS) only.
iced 0.14's `conversion.rs` drops it and iced's `Event` has no pinch. The
vendored `iced_winit` records the gesture and the board reads it as one
more zoom input, about the pointer, within 10 % to 400 %. No harness can
synthesise a pinch, so the check is by hand on a trackpad. Linux waits for
a toolkit upgrade. As built: `iced_winit::pinch` holds each pinch for its
window's next frame, the app hands the board `pinch::take`, and the
widget reads it at that redraw (`vendor/iced_winit/WREN.md`, the side
channel and the alternatives rejected; `discovery/boards.md` §5.13 rule
7, the interplay). The harness's `board.pinch` records one as winit's is
recorded, from which point the path is the real one.

### 3.9 Text size

Every label is 14 units today and a connector's labels 12. A node gains an
optional `style.text-size`: a whole number of board units, 8 to 96, written
only when it is not 14, so no existing board changes by a byte. A value out
of range is drawn clamped and listed as an issue. It applies to a shape's
label and to free text; a connector's labels stay 12. A shape grows to fit
a larger label as it grows for a longer one.

- A number and not a name, unlike colour: every theme supplies its own
  hues, and a size is the same in all of them. The exports already write a
  number (`fontSize` in draw.io, `font-size` in SVG).
- The key is `text-size` because `size` is a node's width and height.
- No change of schema: a new optional key is "allowed without a change of
  schema" (`docs/reference/board-format.md`), and Wren 0.2.0 keeps the key
  as unknown content and draws the label at 14.
- The interface offers presets (12, 14, 20, 28) in the style bar's menu
  and the actions palette; any size in range can be written by hand.

As built: `NodeStyle::text_size` in `wren-board`, read only from an
integer (Wren 0.2.0 kept the key as unknown content of any type, so another
type stays unknown content, kept and drawn at 14, never an error) and
written only when it is not 14. `Limit::TextSize` flags a size outside 8
to 96, drawn at the nearer end. `Intent::SetTextSize` carries each node's
box as the canvas grew it with `label::grown`, the rule a longer label
grows by, now at the node's size. The commands are
`board.text_size.{12,14,20,28}` (`Command::TextSize`), offered in a group
"Text size" for any selection with a node, and unbound; the keys sheet
lists them under their own group once one is bound, and the macOS Board
menu, which holds no style command, does not. The style bar's "Aa" button
follows the fills and folds with them; its menu's rows each draw "Aa"
larger as the size is. `board.get` reports `style.text_size` and the
label's input method caret (`editing.ime`).

## 4. How it fits the code

- The dock, the style bar, the sheet and the footer controls are app
  chrome in `wren-iced`, layered over the canvas. `wren-canvas` stays free
  of app types: it gains the cursor rule, the tool lock, the sticky colour
  and what the chrome needs to read (the armed tool, whether it is locked,
  whether Space is held).
- Every button runs a `board.*` command through `App::dispatch`. There is
  no second path to a mutation, and `App::board_changed()` stays the one
  writer.
- A click on the chrome must not take the keyboard from the board: a blur
  settles a label and cancels a gesture.
- With `boards = off` none of this exists. With `board-toolbar = off` the
  canvas is as it is today plus the cursors.

## 5. Build order

Each step lands alone with Rust unit tests, harness cases (with real X11
clicks for the chrome), and a scene screenshot of each new state in both
Wren themes; the gates are those of `CLAUDE.md`.

1. Cursors (§3.7). Landed.
2. The draw.io line layout for Lucidchart (§6). Landed.
3. Hint line and keys sheet (§3.4, §3.5), with `board-hints`. Landed.
4. Tool dock (§3.1), tool lock, the hand tool staying armed, key badges
   (§3.3), `board-toolbar`. Landed.
5. Style bar (§3.2) and the sticky colour. Landed.
6. Footer controls (§3.6). Landed.
7. Pinch on macOS (§3.8). Landed, and tried on a trackpad.
8. Text size (§3.9), after the style bar, which holds its control. Landed.

## 6. Lucidchart and the draw.io export

Found by hand on 2026-11-18, the owner importing variants of one export
into Lucidchart:

- Lucidchart refused Wren's file and took the same file after draw.io had
  saved it again. The two held identical cells.
- The `mxGraphModel` attributes, `host`, the diagram id and the cell ids
  do not matter: a file with all of Wren's values in draw.io's layout
  imported.
- Of the five ways Wren's text layout differed from draw.io's (attribute
  order on `diagram`, on `mxCell` and on `mxGeometry`, `/>` with no space,
  and one element per line), only the last is refused: an `<object>`, its
  `<mxCell>` and its `<mxGeometry>` on one line.

So the writer puts every element on its own line. The rule is pinned by
`drawio::tests::every_element_starts_a_line_of_its_own`.
