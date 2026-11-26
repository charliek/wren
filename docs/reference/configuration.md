# Configuration

Wren reads one user-owned settings file, `config.conf`, on both macOS and
Linux:

| Platform | Location |
|---|---|
| macOS and Linux | `$XDG_CONFIG_HOME/wren/config.conf`, which defaults to `~/.config/wren/config.conf` |
| Override | set `WREN_CONFIG=/absolute/path/to/config.conf` |

A missing file means every setting uses its default. The palette command
**Open settings file** (⌘, on macOS, Mod+, on Linux) creates the file from a commented
template and opens it in your text editor. Wren re-reads the file when its
window regains focus, or immediately with **Reload settings**. Problems
such as an unknown key or an invalid value show in the warning banner and
never stop Wren from starting.

The same directory holds `words.txt`, the [spell check](spell-check.md)
word list, which Wren re-reads on the same occasions.

The syntax matches Ghostty and Roost: one `key = value` per line, with `#`
comments. When Wren saves a setting itself (choosing a theme or editor
font in the palette, or changing the text size with ⌘+ / ⌘-), it rewrites
only that one line.

> `config.conf` is yours. The separate `config.toml` in the platform config
> directory (`~/Library/Application Support/Wren/` on macOS,
> `~/.config/wren/` on Linux) is Wren's own list of registered workspaces,
> and Wren manages it.

## Settings

| Key | Values | Default | Effect |
|---|---|---|---|
| `theme` | `system`, a theme id, or `light:<id>,dark:<id>` | `system` | The color theme. `system` uses Wren Dark or Wren Light to match the OS appearance. |
| `editor-font-size` | number from 9 to 32 | `14` | Editor text size in points. ⌘+ / ⌘- / ⌘0 (Mod on Linux) change it one point at a time, or reset it, and save the result here. |
| `editor-font-family` | an installed font family, quoted | `"JetBrains Mono"` (bundled) | The editor font. **Change editor font…** lists the monospace ones; any installed family works here. An unknown family warns and uses JetBrains Mono. |
| `editor-line-height` | number from 1.2 to 2.2 | `1.7` | The height of an editor row as a multiple of the text size: `1.7` gives 24 px rows at 14 pt. Rows round to whole pixels, and the ratio holds when the text size changes. The gutter numbers, caret, current-line band, and selection follow it on both engines. |
| `editor-wrap-column` | characters from 20 to 400, or `none` | `100` | Where long lines wrap, counted in the width of the editor font's `0`, so the same number of characters fits at any text size. The text starts at the left of the editor pane, after the line numbers, and wraps at this column or 24 px short of the pane's right edge, whichever comes first; the current-line and code bands span the pane. `none` wraps at the pane's edge. [Focus mode](../getting-started/first-run.md) centres the text at this column (100 when this is `none`), with the line numbers in its left margin; the margins stay at least the gutter's width, 48 px, and 6% of the window wide. Lines only soft-wrap: the file keeps its own line breaks. On the advanced engine the larger H1–H3 headings wrap sooner, since the column counts body-size characters. |
| `line-numbers` | `true` / `false` | `true` | Show the editor's line-number gutter, in focus mode too. |
| `editor-engine` | `classic` / `advanced` | `classic` | The editor engine. `classic` is the current editor; `advanced` is Wren's new editor engine, which is still being evaluated. It keeps the exact scroll position through undo, places outline jumps precisely, sets H1–H3 headings larger (1.35, 1.18, and 1.05 times the text size, in taller rows), and wraps a list item's or a quote's long lines under its text instead of under the marker; ↑ and ↓ keep the caret's horizontal position across those rows. `classic` draws every line at the text size and wraps every row at the left edge. A change applies on reload: the open note moves to the new engine with its text and cursor, but its undo history is cleared. The palette command **Switch editor engine (classic / advanced)** (`editor.toggle_engine`) flips it, saves it here, and applies it at once; its row shows the engine in use. |
| `markdown-assist` | `on` / `off` | `on` | The Markdown-aware editing keys: Enter continues lists and quotes, Tab and Shift+Tab nest list items, Backspace removes markup, and Home stops after the markup. It also enables the [Markdown commands](../getting-started/keybindings.md#markdown-commands) (toggle task, bold, italic, inline code, move line). Off, Enter and Home are plain and the Markdown commands are unavailable; Tab still inserts spaces (or indents the lines of a selection) and Backspace still deletes a whole character. See [Inside the editor](../getting-started/keybindings.md#inside-the-editor). Applies on reload. The palette command **Toggle Markdown assist** (`editor.toggle_markdown_assist`) flips it, saves it here, and applies it at once; its row shows `on` or `off`. |
| `spell-check` | `on` / `off` | `on` | Underline misspelled words and offer corrections, on the `advanced` engine only; `classic` checks nothing. Your own words go in `words.txt`, beside this file. See [Spell check](spell-check.md). The palette command **Toggle spell check** (`editor.toggle_spell_check`) flips it, saves it here, and applies it at once; its row shows `on` or `off`. |
| `boards` | `on` / `off` | `on` | [Boards](boards.md): `.wren-board` notes of shapes and connectors. Off, boards are neither listed, opened, searched, nor created, no board command or New note pill appears, and their files are left alone. Applies when Wren restarts; until then the banner says "Restart Wren to apply `boards`." `wren export` works either way. |
| `board-hints` | `on` / `off` | `on` | The footer under a focused board names the keys that fit what is selected or armed, between the counts and the zoom. See [Boards](boards.md#hints-and-the-keys-sheet). Off, the footer shows only the counts, any issues, the zoom, and the save status; the keys sheet (`?`) still opens. Applies on reload. |
| `board-toolbar` | `on` / `off` | `on` | The board's bars: the [tool dock](boards.md#the-tool-dock) at the bottom centre of a board, a button for each tool and one for the keys sheet; the [style bar](boards.md#the-style-bar) at the top centre while something is selected; and the [footer's undo, redo and zoom buttons](boards.md#the-footers-controls). Off, there is no dock and no style bar, the board names an armed tool in a chip at its top left, a fit uses the whole canvas, and the footer shows the zoom as text. Applies on reload. |
| `board-key-badges` | `hold` / `on` / `off` | `hold` | When the dock's buttons and the style bar's swatches show their keys: while Space is held on the board (`hold`), always (`on`), or never (`off`). A button's tooltip names its key whatever this says. See [Boards](boards.md#the-tool-dock). Applies on reload. |
| `tasks` | `on` / `off` | `off` | [Tasks](tasks.md): statuses, due dates, priorities and owners on task lines, with their colours, status-bar line, menus, calendar, suggestions, form, commands and keys. Off, task lines edit, highlight, spell check and search exactly as before, and no task command, key or file exists. Applies when Wren restarts; until then the banner says "Restart Wren to apply `tasks`." |
| `tasks-suggest` | `on` / `off` | `on` | With `tasks = on` on the `advanced` engine, typing on a task line offers a due date, a priority or an owner. See [Suggestions](tasks.md#suggestions). Off, nothing is offered and Tab and Enter act as in any list. Applies on reload. |
| `linux-modifier` | `ctrl` / `alt` | `ctrl` | The command key, **Mod**, on Linux: Ctrl, as in most Linux apps, or Alt, the way ⌘ works on macOS. With `alt`, every Mod shortcut moves to Alt, copy, cut, paste, select all, undo and redo included, and only these Ctrl keys keep working too: Ctrl+F, Ctrl+Q, Ctrl+Z, Ctrl+Shift+Z, Ctrl+C, Ctrl+X, Ctrl+V and Ctrl+A. Other Ctrl shortcuts, such as Ctrl+N, do not. Applies at once. The palette command **Use Alt for commands** (**Use Ctrl for commands** while Alt is in use) flips it and saves it here. macOS ignores it. See [The command key on Linux](../getting-started/keybindings.md#the-command-key-on-linux). |
| `keybind` | `<trigger> = <action>` | see [Keybindings](../getting-started/keybindings.md) | Add, replace, or remove a shortcut. Repeatable; later lines win. Board shortcuts are listed under [On a board](../getting-started/keybindings.md#on-a-board). |

## Themes

| Id | Name | Appearance |
|---|---|---|
| `wren-dark` | Wren Dark | dark |
| `wren-light` | Wren Light | light |
| `nord` | Nord | dark |
| `gruvbox-dark` | Gruvbox Dark | dark |
| `solarized-light` | Solarized Light | light |
| `rose-pine` | Rosé Pine | dark |
| `everforest-dark` | Everforest Dark | dark |
| `tokyo-night` | Tokyo Night | dark |
| `dracula` | Dracula | dark |
| `catppuccin-mocha` | Catppuccin Mocha | dark |
| `catppuccin-macchiato` | Catppuccin Macchiato | dark |

Every theme also supplies its own seven [board](boards.md#colour) colours,
so a board recolours with the theme and its file does not change.

**Change theme…** in the command palette shows a small preview of each
theme, previews the highlighted one live, and saves your pick as
`theme = <id>`. To follow the OS appearance with your own pair:

```conf
theme = light:solarized-light,dark:nord
```

## Editor font

**Change editor font…** in the command palette lists the monospace fonts
installed on your system, with the bundled JetBrains Mono first. Moving the
highlight previews each font in the open note; ↵ saves it as
`editor-font-family`, and Esc keeps the font you had. **Change theme…**
previews the same way.

```conf
editor-font-family = "Fira Code"
editor-font-size = 16
```

A family the picker doesn't list (a proportional font such as `"Inter"`)
works too when you set it here. Markdown tables and code blocks won't line
up in a proportional font.

## Example

```conf
theme = light:wren-light,dark:rose-pine
editor-font-size = 15

# VS Code muscle memory on Linux
keybind = ctrl+shift+p = palette.commands
keybind = ctrl+p = navigation.jump
```
