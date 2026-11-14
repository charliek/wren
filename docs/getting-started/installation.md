# Installation

## macOS

Download `Wren-<version>.dmg` from the
[latest GitHub release](https://github.com/charliek/wren/releases) and drag
`Wren.app` into `/Applications`.

Wren updates itself through Sparkle. It checks for new releases on a
schedule, and **Wren → Check for Updates…** checks on demand.

## Linux

On Ubuntu 24.04 and Pop!\_OS, Wren is published to the
[charliek apt repository](https://github.com/charliek/apt-charliek). After
adding the repository:

```bash
sudo apt install wren
```

Each release also attaches `.deb` packages for amd64 and arm64 to
[GitHub Releases](https://github.com/charliek/wren/releases).

## The `wren` command

The app is also a command line, used for
[`wren export`](../reference/boards.md#wren-export), which writes a board as
an image or a draw.io file without opening a window.

| Install | The binary |
|---|---|
| macOS | `/Applications/Wren.app/Contents/MacOS/wren`. Add an alias, or a symlink in a folder on your `PATH`, to call it as `wren`. |
| Linux `.deb` | `/usr/bin/wren`, already on your `PATH`. |
| From source | `target/debug/wren` after `make build`, or run it through cargo: `cargo run --locked -p wren-iced -- export …`. |

```bash
wren export "Checkout flow.wren-board" -o flow.png
```

## Build from source

Requires the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs
it automatically) and [uv](https://docs.astral.sh/uv/) for the test harness.
On Linux the build also needs the usual windowing development libraries
(`libxkbcommon`, Wayland, and X11 client libraries); `.github/workflows/ci.yml`
has the exact package list.

```bash
make setup     # fetch locked Rust and Python dependencies
make run       # restore the last workspace, or show onboarding
make package   # build an uninstalled native bundle (Wren.app on macOS)
```

| Command | Effect |
|---|---|
| `make run ARGS="--workspace /absolute/path"` | Open a specific folder as the workspace. |
| `make run ARGS="--editor"` | Open the unsaved editor evaluation (no workspace). |
| `wren --font-licenses` | Print the bundled font licenses. |
