# Boards

A board is a second kind of note: a canvas of shapes and connectors for
flowcharts and small diagrams. It is saved as `Name.wren-board` beside the
Markdown notes of a group, in a plain text format with one field per line
(see [Board file format](board-format.md)), so a board kept in a git
repository has changes you can read and track. Boards are drawn by Wren
itself and export to SVG, PNG, and draw.io, in the app or from the
`wren export` command line.

Keys below are written macOS first, then Linux. On a board, **Mod** is ⌘ on
macOS and **Ctrl** on Linux, the modifier text editing uses; the app's own
shortcuts keep ⌘ and Alt. The full key list is in
[Keybindings](../getting-started/keybindings.md#on-a-board).

## Creating a board

**New note…** (⌘N / Alt+N) has two kind pills under the name field,
Markdown and Board. Markdown is selected every time the dialog opens; Tab
and Shift+Tab switch the pill while the keyboard stays in the name field.
So a new board is ⌘N, the name, Tab, Enter. **New board…** in the command
palette opens the same dialog with Board already picked. Rename never
changes a note's kind.

A new board opens empty at 100 %, with the hint "Tab adds a shape. R O D U
T pick a tool." Boards rename, move, delete, and appear in Jump to note and
Back/Forward like any note. The notes list marks them with a board icon,
and its second line shows a board's first label.

The footer reads `Board · 15 nodes · 17 connectors`, then, when there are
any, the number of issues and `Read-only`, then the zoom and the save
status.

## Shapes and text

| Tool | Key | |
|---|---|---|
| Select | V | Click to select, drag to move, drag on empty canvas for a marquee. |
| Hand | H | Drag to pan. |
| Rectangle, ellipse, diamond, pill, cylinder, parallelogram | R, O, D, U, Y, P | Click for a 120 by 60 shape centred there, or drag from corner to corner. |
| Rounded rectangle, document | — | The same, from the command palette. |
| Text | T | Free text: no outline and no fill, left- and top-aligned. |
| Connector | C or A | See [Connectors](#connectors). |

Tools are one-shot: after one shape the select tool is back, and Esc or V
puts an armed tool away. A double-click on empty canvas makes a rectangle,
and every new shape opens its label for typing. Shift with a shape's letter
(⇧R, ⇧O, ⇧D, ⇧U, ⇧Y, ⇧P) changes the shape of every selected shape and
keeps its size and label.

Everything snaps to a 20-unit grid when it is created, moved, or resized;
there is no key to turn snapping off. A unit is one pixel at 100 %.

**Building from the keyboard.**

| Key | Effect |
|---|---|
| Tab | Add the next shape: a rectangle (or the armed shape) in the board's flow direction from the selected shape, connected to it, with its label open. With nothing selected it adds an unconnected shape in the middle of the view. |
| Mod+Arrow | Connect in that direction: to the shape already there if the two are not yet connected, otherwise to a new shape. |
| Arrows | Select the nearest shape in that direction. Walking never changes the file. |
| Shift+Arrow | Add that shape to the selection. |
| Shift+Mod+Arrow | Select the connector that meets the selected shape on that side. |
| Alt+Arrow | Move the selection one grid cell. |
| Alt+Shift+Arrow | Make the selected shapes wider (→), narrower (←), taller (↓), or shorter (↑) by one cell. |

The flow direction is the way most connectors on the board point; on a
board with none it is down. A new shape takes the nearest free place in
that direction, so a branch steps aside instead of covering another shape.

**Labels.** Enter or F2 edits the selected element's label, with its text
selected, and a double-click edits the label under the pointer. Inside a
label, Enter and Shift+Enter insert a line break, and the arrows, Backspace,
and Mod+A/C/X/V are the text's. Mod+Enter, Esc, or a click elsewhere
finishes the label, and Tab finishes it and adds the next shape. Mod+Z
inside a label puts it back as it was when you started; after it is
finished, the board's undo does. A label that grows past its shape makes
the shape taller in whole cells as you type; nothing shrinks a shape by
itself. Free text, or a connector label, that you empty is removed when
you finish it. A label holds at most 16 KiB.

**Selection.** Click selects one element, Shift-click adds or removes one,
and a marquee selects what lies wholly inside it. Mod+A selects
everything, ⇧Mod+A adds every shape reachable from the selection by
following connectors in their direction, and Esc clears the selection.
The last element selected keeps a ring (the cursor) when the selection
changes, so walking with the arrows picks up where you were.

**Delete, duplicate, and the clipboard.**

- Backspace or Delete removes the selection, every connector attached to a
  removed shape, and any shape nested inside one.
- Mod+D duplicates the selected shapes, and the connectors among them, one
  cell right and down. Dragging with ⌥ (macOS) or Ctrl (Linux) held leaves
  the originals and moves copies.
- Mod+C copies the selected shapes, the shapes nested in them, and the
  connectors between them as text that starts
  `# wren-board fragment 1`, followed by a board file holding only those
  elements. Mod+V pastes it into any board with new ids, one cell further
  right and down each time; plain text from elsewhere becomes one text
  element, wrapped at 320 units.
- Mod+X copies, then deletes only what it copied, together with the
  connectors attached to the shapes it removed. A connector to a shape
  outside the selection is therefore deleted but not copied (undo brings
  it back), and a selected connector whose ends are not both cut stays on
  the board.

**Undo.** Mod+Z and ⇧Mod+Z (also Ctrl+Y on Linux) undo and redo whole
steps: a drag, a command, a finished label, a run of arrow-key moves. Undo
puts the selection back too. Undoing every step returns the file to the
exact bytes it was opened with. The history holds up to 200 steps and is
dropped when another note opens or the board changes on disk.

## Connectors

**With the pointer.** Rest the pointer on a shape for a moment and four
connect handles appear outside its sides. Drag from one onto another
shape to connect the two, or onto empty canvas to make a new shape there,
connected, with its label open. With the connector tool (C or A), drag
from any shape. Releasing near the middle of one side of the target, where
its side handles are, pins the end to that side; anywhere else on the
target the end floats, and the route picks the side. Select one connector
to show a handle at each end; drag an end onto another shape to move it.

**With the keyboard.** Tab and Mod+Arrow connect as they build (above). To
reach a shape further away, select the source and press C: arrows move the
highlighted target, and typing finds the shape whose label contains what
you type, shown in the corner chip ("Connect to: che"). Enter connects,
Esc gives up.

**Routes and styles.** Connectors are elbow (right-angled) by default, and
an elbow route goes around other shapes and off other connectors' lines
where a short detour allows. Routes are worked out each time the board is
drawn and never stored, so moving a shape changes no connector in the
file. A connector has an arrowhead at its end by default and one label,
drawn on a canvas-coloured backing at the middle of its route. From the
command palette or the [actions palette](#the-actions-palette):

| Command | Effect on the selected connectors |
|---|---|
| Connector: straight or elbow | Switch the route. |
| Connector: start head / end head | Add or remove the arrowhead at that end. |
| Connector: reverse | Turn the arrow round. |
| Solid or dashed | Switch the line, and the outline of selected shapes. |

A connector whose end names a shape that is not on the board is drawn as a
short stub ending in a cross, in the theme's error colour.

## Colour

Every shape, text element, and connector has one of eight colours, set
with the digit keys on the selection:

| Key | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|
| Colour | neutral | red | orange | yellow | green | cyan | purple | blue |

Shapes also have one of three fills, from the palette: **Fill: none** (the
outline only), **Fill: tint** (the colour mixed into the canvas, the
default), and **Fill: solid**. Neutral is the theme's text colour, and its
solid fill is the theme's raised surface colour. A shape's label is in the
theme's text colour, except on a solid fill, where it takes whichever of
the text and canvas colours reads better. Free text, and a connector's
line, arrowheads, and label, take the colour itself.

The file stores the colour's name, not a value, and every theme supplies
its own seven hues, taken from that theme's published palette where it has
one. So a board looks native in any theme and its file never changes when
you switch. In every built-in theme, a label stays at 4.5:1 or better on
its fill, outlines reach 3:1 on the canvas, and every pair of the eight
colours stays distinguishable for normal vision and under simulated
protanopia, deuteranopia, and tritanopia.

A colour written by hand as `#rrggbb` is drawn as written in every theme;
a colour key replaces it with the colour you pick.

On layouts that type a symbol on the digit row, such as AZERTY (which types
`&` for 1), a digit key that types something nothing is bound to acts as its
digit, so the colour keys work without Shift. Layouts with non-Latin
letters already use the physical key.

## Arranging

**Stacking.** ] brings the selection to the front and [ sends it to the
back; ⇧] and ⇧[ move it one step forward or backward past the next
element. Shapes and connectors stack together, and a shape's nested shapes
stack among themselves above it. Each command changes only the moved
elements' `z` lines in the file.

**Align and distribute**, from the palette, the actions palette, or the
macOS Board menu:

| Command | Needs | Effect |
|---|---|---|
| Align left, centre, right | 2 shapes | Line their left edges, centres, or right edges up with the selection's outer box. |
| Align top, middle, bottom | 2 shapes | The same vertically. |
| Distribute horizontally, vertically | 3 shapes | Keep the two outermost where they are and make the gaps between neighbours equal. |

Align and distribute place shapes on whole units without snapping to the
grid, so shapes of different sizes can centre exactly. Running either a
second time changes nothing. Shapes that overlap get equal negative gaps.
When shapes overlap so far that equal spacing would change their order,
distribute leaves that row as it is. A shape selected with one of the
shapes it is nested in moves with its container, not on its own, and
connectors follow the shapes they join.

## The actions palette

/ or Shift+F10, or a right-click (Ctrl-click on macOS), opens the actions
palette for the selection. A right-click on an element outside the
selection selects it alone first; on empty canvas it clears the selection.

It lists only what applies to the selection, grouped: Shape, Colour, Fill,
Stroke, Connector, Arrange, Align, Distribute, and Edit (Edit label,
Duplicate, Copy, Cut, Delete). The row that matches the selection's
current shape, colour, or fill is marked and highlighted when the palette
opens. With nothing selected it lists Paste, Select all, Zoom to fit, and
Export board as SVG….

Moving the highlight over a shape, colour, fill, stroke, or connector row
shows the change on the board without making it. ↵ applies it as one undo
step and returns the keyboard to the board. Esc, or closing the palette any
other way, drops the preview; nothing is written.

## Jumping to an element

**Jump to node…** lists every labelled shape, text element, and connector
on the board in reading order (top, then left), with the label's first line
as the title and the shape's name, `text`, or `connector` at the end of the
row. On a board, Jump to heading (⇧⌘O / Alt+Shift+O) and Find in note
(⌘F / Alt+F or Ctrl+F) open it. Type to filter. Moving the highlight
selects and centres that element; ↵ keeps it and gives the board the
keyboard, and Esc puts the selection and the view back.

A workspace search that matches a board's label opens the board with the
first matching element, in reading order, selected and centred. Labels are
searched as text; the file's own syntax (`kind`, `schema`) is not.

## Zoom and pan

| Action | macOS | Linux |
|---|---|---|
| Pan | Scroll, Shift+scroll sideways, middle-drag, Space+drag, or the hand tool | the same |
| Zoom at the pointer | ⌘+scroll | Ctrl+scroll |
| Zoom in / out / to 100 % | ⌘= / ⌘- / ⌘0 | Alt+= / Alt+- / Alt+0 |
| Zoom to fit / to selection | ⇧1 / ⇧2 | the same |

The text-size keys step through 10, 25, 50, 75, 100, 150, 200, 300, and 400
% on a board and save nothing. A board opens fitted to its content, never
above 100 %, and returns to where you left it when you come back to it in
the same run. The view is never written to the file. Pinch-to-zoom is not
supported. With the software renderer (`ICED_BACKEND=tiny-skia`), labels
are left out while the view moves and when they would be smaller than 4
pixels.

## Exporting

| Command | Writes |
|---|---|
| **Export board as SVG…** | An SVG image of the board in the current theme. |
| **Export board as PNG…** | The same as a PNG image, at two pixels per unit. |
| **Export board as draw.io…** | A draw.io file, always in Wren Light's colours. |

They are in the command palette and, on macOS, the Board menu. The image
is the board as it was drawn when the command ran, on the theme's canvas
colour with a 40-unit margin; then a save dialog opens with the note's
title as the name. Inside the workspace an export must end in its
format's extension (`.svg`, `.png`, or `.drawio`), and an export never
replaces a note, so a stray name cannot overwrite a note or Wren's own
files. The file is written to a temporary file and renamed, so it is
whole or untouched.

### `wren export`

```text
wren export <board> [-o <path>|-] [--format svg|png|drawio] [--theme <id>] [--scale 1..4] [--background theme|transparent]
```

The command line writes the same files with no window, so it works over
ssh, in a script, or in CI. It does not read `config.conf`, so it works
whatever the `boards` setting says. [Installation](../getting-started/installation.md#the-wren-command)
says where the `wren` binary is.

| Option | Default | Meaning |
|---|---|---|
| `-o <path>` | beside the board, `<name>.svg`, `.png`, or `.drawio` | Where to write. `-` writes to standard output. An existing file is replaced. |
| `--format` | from the extension of `-o`, else `svg` | `svg`, `png`, or `drawio`. A name with another extension needs `--format`. |
| `--theme <id>` | `wren-light` | Any [theme id](configuration.md#themes); `dark` and `light` also work. |
| `--scale <n>` | `2` | PNG pixels per board unit, from 1 to 4. PNG only. |
| `--background` | `theme` | `theme` (the theme's canvas colour) or `transparent`. SVG and PNG only. |

```bash
wren export "Checkout flow.wren-board"                       # Checkout flow.svg, Wren Light
wren export "Checkout flow.wren-board" -o flow.png --theme wren-dark --scale 1
wren export "Checkout flow.wren-board" --format drawio -o -  # to standard output
```

| Exit status | Meaning |
|---|---|
| 0 | Written. Problems with the board that still draw, and a board this version would open read-only, are printed to standard error as warnings. |
| 1 | The board could not be read, the output could not be written (a folder, a device, the board itself, or a note), or the image could not be made. |
| 2 | Bad usage. |
| 3 | The file is not a board Wren can draw: not valid (the message names the line), not UTF-8, or over a limit. |

Nothing is written when the command fails.

### PNG images

- An image may have at most 64 megapixels. A larger board exports at a
  lower `--scale`, or as SVG.
- Labels are drawn in the bundled Inter. Characters Inter lacks, such as
  Chinese and Japanese, come from the fonts installed on the system.
- A joined emoji sequence (a skin tone, or emoji joined into one, such as
  👩🏽‍💻) is drawn as boxes, and so is every other emoji in the same label. A
  label of single emoji draws them in colour when a colour emoji font is
  installed.
- Arabic in the same right-to-left run as Hebrew is drawn with its letters
  unjoined; Arabic on its own joins.
- In some Devanagari labels the space between two words is lost.
- Images and links in a board are never fetched.
- A transparent background keeps the canvas-coloured backing behind
  connector labels, which shows as patches on a background of another
  colour.

### draw.io files

The draw.io export is one uncompressed page that draw.io opens as its own
built-in shapes and connectors, with Wren's ids kept as `wrenId` on each
element. It is a one-way export: draw.io gives every element a new id when
it imports or pastes a file, and nothing promises a file comes back to
Wren. What does not survive:

- The colour names: colours are written as fixed values for one theme
  (Wren Light from the app, `--theme` from the command line), and draw.io's
  dark mode works out its own.
- Wren's font and wrapping: draw.io wraps labels in its own font, so a
  label may break differently or overflow its shape.
- Routes: draw.io bends elbow connectors itself and avoids no shapes, so a
  connector can cross a shape, two can share a line, and a floating end on
  a pill, cylinder, or document stops at its box.
- The stacking keys (the order is kept), a placeholder's caption, anything
  in the file Wren does not draw, labels past 16 KiB and connector labels
  past the sixteenth, issue markers, and the look of a dangling connector,
  which becomes a free end.
- A tab in a label is four spaces, and arrowheads are slightly larger.

Only draw.io itself has been tested. Lucidchart's draw.io import is
untested, and Visio and Miro need formats Wren does not write.

## Problems in a file

Wren never repairs a board you did not edit. What can still be drawn is
drawn and listed as an issue: the footer counts them, a small dot marks
each element, and `wren export` prints them. Issues include a connector end
or a `parent` naming a shape that is not there, a value outside its range
(drawn clamped), and an id shared by a shape and a connector.

A board that is not valid opens as text instead, under a banner naming the
line and the problem: "This board is shown as text because line 4, column 2
is not valid: … It opens as a board again once the text is valid." You can
fix it there; what you type is saved exactly as typed, and the board opens
as a board the next time you open it, or as soon as an edit from another
program makes it valid. Merge conflict markers left by git show this way.

A board that names a feature this version does not know (`requires`), or a
newer `schema`, opens **read-only**: you can select, copy, jump, zoom, and
export, and nothing is written.

## Limits

| Limit | Value | Beyond it |
|---|---|---|
| File size | 8 MiB | Opens as text; searched as text. |
| Shapes and connectors | 50,000 | Opens as text; searched as text. |
| Coordinates | ±16,777,216 units | Drawn clamped, listed as an issue. |
| Nesting | 32 levels | Deeper shapes are drawn at the top level, listed as an issue. |
| One label | 16 KiB and 1,024 lines | The rest is not drawn, listed as an issue; typing past 16 KiB is refused. |
| Labels on one connector | 16 | The rest are not drawn, listed as an issue. |
| Label lines on a board | 262,144 | Later labels are cut, listed as an issue. |
| PNG image | 64 megapixels | The export is refused. |

Values past a limit are drawn within it and kept in the file as they are.
Wren opens no note over 64 MiB.

## The setting

Boards are on by default. `boards = off` in
[`config.conf`](configuration.md) turns them off: boards are neither
listed, opened, searched, nor created, no board command appears, and
`.wren-board` files are left alone. The change applies when Wren restarts;
until then the banner says "Restart Wren to apply `boards`."
