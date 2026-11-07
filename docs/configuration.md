# Configuration

Wren reads one user-owned settings file, `config.conf`, on both macOS and
Linux:

| Platform | Location |
|---|---|
| macOS and Linux | `$XDG_CONFIG_HOME/wren/config.conf`, which defaults to `~/.config/wren/config.conf` |
| Override | set `WREN_CONFIG=/absolute/path/to/config.conf` |

A missing file means every setting uses its default. The palette command
**Open settings file** (⌘, on macOS) creates the file from a commented
template and opens it in your text editor. Wren re-reads the file when its
window regains focus, or immediately with **Reload settings**. Problems
such as an unknown key or an invalid value show in the warning banner and
never stop Wren from starting.

The syntax matches Ghostty and Roost: one `key = value` per line, with `#`
comments. When Wren saves a setting itself (choosing a theme in the
palette), it rewrites only that one line.

> `config.conf` is yours. The separate `config.toml` in the platform config
> directory (`~/Library/Application Support/Wren/` on macOS,
> `~/.config/wren/` on Linux) is Wren's own list of registered workspaces,
> and Wren manages it.

## Settings

| Key | Values | Default | Effect |
|---|---|---|---|
| `theme` | `system`, a theme id, or `light:<id>,dark:<id>` | `system` | The color theme. `system` uses Wren Dark or Wren Light to match the OS appearance. |
| `editor-font-size` | number from 9 to 32 | `14` | Editor text size in points. |
| `line-numbers` | `true` / `false` | `true` | Show the editor's line-number gutter. |
| `keybind` | `<trigger> = <action>` | see [Keybindings](keybindings.md) | Add, replace, or remove a shortcut. Repeatable; later lines win. |

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

**Change theme…** in the command palette picks one and saves it as
`theme = <id>`. To follow the OS appearance with your own pair:

```conf
theme = light:solarized-light,dark:nord
```

## Example

```conf
theme = light:wren-light,dark:rose-pine
editor-font-size = 15

# VS Code muscle memory on Linux
keybind = ctrl+shift+p = palette.commands
keybind = ctrl+p = navigation.jump
```
