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

## Limits

These guarantees cover cooperating programs and crashes of Wren itself. They
do not cover power loss, or another program writing the same file at the
same instant as Wren's final rename.
