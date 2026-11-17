# Spell check

On the `advanced` [editor engine](configuration.md), Wren underlines
misspelled words with a red zigzag and offers corrections in a small menu
under the word. Checking is offline and works the same on macOS and Linux:
a bundled US English dictionary (SCOWL `en_US`, read by `spellbook`) plus a
word list you edit yourself. The `classic` engine checks nothing.

Spell check is on by default. Turn it off with `spell-check = off` in
[`config.conf`](configuration.md), or with the palette command **Toggle
spell check**, which saves the choice there and applies it at once; its
row shows `on` or `off`. On the classic engine the command still flips the
setting, ready for when you switch engines.

The dictionary loads in the background when Wren starts, so the first
marks can appear a moment after the window does. If it cannot load, the
warning banner says `Spell check is unavailable` with the reason, and the
editor works as usual.

## What is checked

Spell check knows Markdown and reads only the text a reader sees. It
checks Markdown notes only: [boards](boards.md) are not checked, and
neither is a board that does not parse and opens as text in the editor,
where **Suggest spelling** is unavailable too.

| Checked | Not checked |
|---|---|
| Paragraphs, list items, quotes, tasks (open and done), table rows, and headings | Front matter, fenced and indented code, HTML blocks, thematic breaks, and link reference definitions (`[label]: url`) |

Inside checked text, these are skipped:

- Code spans, backticks included.
- HTML comments that open and close on the same line.
- Everything in a link or image but its text: in `[the text](url "title")`
  and `![alt](url)` only `the text` and `alt` are checked.
- Bare URLs and autolinks (`<https://…>`).
- Emphasis and strikethrough markers, so `_word_` is checked as `word`.

A word is skipped whole when it:

- contains `@`, `/`, `\`, `<`, `>`, or `[^`: e-mail addresses, paths,
  `and/or`, inline tags such as `<kbd>`, and footnote marks;
- is a tag such as `#idea`;
- is joined by `_`, `*`, or `~` inside, such as `snake_case` or
  `un*believ*able`;
- holds a digit, an inner `.` or `:`, or a letter past Latin Extended-B
  (U+024F): `v0.1.1`, or a word in Greek, Cyrillic, or Chinese;
- is shorter than 2 or longer than 40 characters;
- is written in capitals, optionally with an `s` or `'s` after them:
  `README`, `README's`, and `PRs`.

Hyphenated words are checked part by part, so `well-knwon` marks only
`knwon`. A dictionary word in lower case is also accepted capitalised, but
a word with a capital inside it, such as `CommonMark`, is checked as
written and marked if the dictionary does not know it; add such names to
your [word list](#your-word-list). A word ending in `'s` is correct when the
word before it is. A curly apostrophe (`’`) counts as a straight one.

The mark under the word you are typing is hidden while the caret sits at
its end, so it appears once you type a space or move away. Marks follow
every edit, undo, and redo at once.

## The corrections menu

Open it on a marked word in either of two ways:

- Right-click the word. The editor takes the keyboard, as a left click
  would.
- With the caret in or touching the word, press **Suggest spelling**:
  Alt+. on Linux, ⌘. on macOS. From the command palette, the same command
  returns the keyboard to the editor and opens the menu at its caret.

Anywhere else, neither opens anything. The menu lists up to five
corrections, best first, with the first one highlighted, or a single
inactive "No suggestions" row. It opens under the word, or above it when
there is no room below. Opening it moves neither the caret nor the
selection; the word is tinted while the menu is open.

| Key or pointer | While the menu is open |
|---|---|
| ↑ / ↓ | Highlight the previous or next correction. |
| ↵ | Replace the word with the highlighted correction. |
| Esc | Close the menu and do nothing else. An open find bar stays open. |
| Moving over a row | Highlight it. |
| Clicking a row | Replace the word with that correction. |
| Clicking elsewhere in the editor | Close the menu; the caret does not move. |
| Any other key, or a click outside the editor | Close the menu, then do what it always does. A modifier key pressed alone does nothing. |

A replacement is one undo step, puts the caret after the new word, and
changes nothing else in the line, line endings included. When the word was
written with a curly apostrophe, the corrections use one too.

The menu also closes when the palette or a dialog opens, the note,
workspace, or engine changes, the text changes, the editor scrolls by a line
or more or loses the keyboard, the window loses focus, a setting or the theme
changes, or an input method starts composing.

The menu offers corrections only. To stop a word being marked, add it to
your word list.

## Your word list

`words.txt` sits in the directory that holds
[`config.conf`](configuration.md): `~/.config/wren/words.txt` by default,
or beside the file `WREN_CONFIG` names. The palette
command **Open spelling word list** creates it, holding only a short
comment header, if it is missing, then opens it in your text editor, as
**Open settings file** opens `config.conf`. On macOS it is also in the Wren
menu as **Spelling Word List…**.

```text
# My words
catppuccin
CommonMark
Kubernetes
```

| Rule | Detail |
|---|---|
| Format | UTF-8, one word per line. Blank lines, lines starting with `#`, and a leading byte-order mark are ignored; spaces around a word are trimmed; `’` reads as `'`. |
| Case | A word in lower case is accepted in lower case, capitalised, and in capitals: `catppuccin` also allows `Catppuccin` and `CATPPUCCIN`. A word with any capital is accepted exactly as written. |
| Bad lines | A line holding more than one word, or longer than 40 characters, is skipped, and the warning banner names the file and the line: `…/words.txt: line 3: not one word`. |
| Reloading | Wren re-reads the file when its window regains focus, on **Reload settings**, and whenever it saves a setting. |
| Writing | Wren never changes the file after creating it. |

A file Wren cannot read (a directory, text that is not UTF-8, or a file
over 1 MiB) shows a warning in the banner and counts as empty; the
dictionary keeps working. The word list only accepts words: corrections
always come from the dictionary.

## Commands

| Command | Action name | Default key | Available |
|---|---|---|---|
| Toggle spell check | `editor.toggle_spell_check` | none | Always. Its palette row shows `on` or `off`. |
| Suggest spelling | `editor.spell_suggest` | Alt+. (Linux), ⌘. (macOS) | On the advanced engine with spell check on, once the dictionary has loaded and a Markdown note is open (not a board, nor a board shown as text). |
| Open spelling word list | `settings.open_words` | none | Always. Like **Open settings file**, it stops with a message when there is no settings location (neither `HOME` nor `WREN_CONFIG` is set). |

All three can be bound with `keybind` lines (see
[Custom keybindings](../getting-started/keybindings.md#custom-keybindings)).
Suggest spelling belongs to the editor: its key works only while the editor
has the keyboard, whether or not Markdown assist is on.
