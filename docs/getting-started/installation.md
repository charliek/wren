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
