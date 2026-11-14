# Autosave and external edits

There is no Save command. Wren writes the open note:

- 750 ms after you stop typing, and at least every 750 ms while you keep
  typing;
- before switching notes or workspaces, before moving, renaming, or deleting
  the note or one of its folders;
- when the window loses focus, and when Wren quits.

If a save fails, the transition waits. The draft stays in the editor, the
footer shows the error with a retry, and Wren does not switch, move, or quit
until the note is saved.

## How a save is written

Wren writes a sibling temporary file, syncs it, and renames it over the note,
so a crash leaves either the old file or the new one, never a partial note.
Before replacing the note, Wren checks that the file on disk is still the one
it loaded.

## Edits from other programs

Wren watches the workspace, and also checks it every second, so changes from
Obsidian, `git pull`, or a sync service show up within about a second.

| Situation | What Wren does |
|---|---|
| A closed note changes | The browser and search pick up the change. |
| The open note changes and you have no unsaved edits | It reloads, keeping the cursor on the same line where possible. |
| The open note changes while you have unsaved edits | Wren keeps both. Your draft is saved as `<title> (conflict YYYY-MM-DD HHMM).md` next to the original, the disk version loads, and a banner links to the copy. |
| The open note is deleted while you have unsaved edits | The draft is kept, never discarded. |

## Boards

A [board](boards.md) saves through the same autosave. Each finished change
(a drag when you release it, a command, each keystroke in a label) becomes
the board's unsaved draft, and the draft is written as above: 750 ms after
the last change, and before every switch, move, focus loss, and quit.
Before one of those, an open label is finished and kept, and a drag still
in progress is cancelled, so a half-finished move is never written.
Moving the view, selecting, and walking with the arrows never change the
file.

- **Opening never writes.** A board that is not in Wren's canonical form
  (written by hand, by another tool, or merged by git) keeps its exact
  bytes until you change it; the first save after a change writes it in
  [canonical form](board-format.md#canonical-form). Undoing every change
  writes the original bytes back, and the board is clean again.
- **Changed on disk, no unsaved edits:** the board reloads, keeping the
  view and the selection of elements that still exist. Its undo history is
  cleared and an open label closes.
- **Changed on disk with unsaved edits:** the draft, including a label
  typed a moment ago, is saved as
  `<title> (conflict YYYY-MM-DD HHMM).wren-board` next to the original, and
  the disk version loads.
- **Not valid:** a board Wren cannot read (one left with git conflict
  markers, say) opens as its text under a banner naming the line. What you
  type there is saved exactly as typed. Once the text is valid it opens as
  a board the next time you open it, or at once when the change came from
  another program.
- **Read-only:** a board that needs a newer version of Wren (a newer
  `schema`, or a feature in `requires` this version lacks) is never
  written.

## Limits

These guarantees cover cooperating programs and crashes of Wren itself. They
do not cover power loss, or another program writing the same file at the
same instant as Wren's final rename.
