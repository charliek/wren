#!/usr/bin/env bash
# Bump Wren's release version across the manifests it owns.
#
# Single source of truth is [workspace.package].version in the root Cargo.toml;
# both members (wren-core, wren-iced) inherit it via `version.workspace = true`.
# Cargo.lock is refreshed offline so their entries match, because CI builds
# everything with --locked.
#
# NOT bumped: pyproject.toml (and uv.lock), which version the Python test
# harness only and keep their own cadence.
#
# Wren's Cargo.toml uses vanilla single-space style (`version = "X.Y.Z"`). The
# substitution is confined to the [workspace.package] table so a column-0
# `version` key in any other table can never be rewritten by accident.
#
# Contract (cc-plugins release-workflows references/update-version/README.md):
#   - one arg: semver string, no `v` prefix
#   - idempotent (same-version re-run leaves the tree unchanged)
#   - no network (--offline)
#   - verifies its own work
#   - does not `git add` (the release skill stages + commits)
#
# Adapted from the release-workflows cargo-workspace.sh template.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <X.Y.Z>   e.g. $0 0.0.2" >&2
  exit 2
fi
V="$1"

if [[ ! "$V" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$ ]]; then
  echo "error: '$V' is not semver (X.Y.Z or X.Y.Z-suffix)" >&2
  exit 2
fi

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

workspace_version() {
  sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml
}

# 1. Bump [workspace.package].version, and only that table's key.
sed -i.bak -E '/^\[workspace\.package\]/,/^\[/s/^version[[:space:]]*=[[:space:]]*"[^"]+"/version = "'"$V"'"/' Cargo.toml
rm -f Cargo.toml.bak

# 2. Verify. A sed that matches nothing still exits 0.
if [[ "$(workspace_version)" != "$V" ]]; then
  echo "error: Cargo.toml's [workspace.package].version did not update to $V." >&2
  echo "       Inspect by hand: the sed pattern may not match the current layout." >&2
  exit 1
fi

# 3. Refresh Cargo.lock's workspace member entries. --offline is safe: only
#    internal version strings change, never the dependency graph.
#
#    Resolve cargo through `mise exec` when the repo pins its toolchain there
#    (.mise.toml), else use cargo from PATH (CI runners with rustup). A
#    non-interactive shell often lacks cargo on PATH even when mise has
#    provisioned the pinned toolchain.
if command -v mise >/dev/null 2>&1 && [[ -f .mise.toml ]]; then
  cargo=(mise exec -- cargo)
elif command -v cargo >/dev/null 2>&1; then
  cargo=(cargo)
else
  echo "error: cargo not found and 'mise exec' unavailable." >&2
  echo "       Install Rust (via rustup) or run inside a shell where" >&2
  echo "       \`mise exec -- cargo --version\` or \`cargo --version\` works." >&2
  exit 1
fi
"${cargo[@]}" update --workspace --offline --quiet

# 4. Verify every workspace member's lock entry moved; a member that overrides
#    the inherited version would be left behind.
for member in wren-core wren-iced; do
  locked="$(awk -v name="$member" '
    $0 == "name = \"" name "\"" { found = 1; next }
    found && /^version = / { gsub(/^version = "|"$/, ""); print; exit }
  ' Cargo.lock)"
  if [[ "$locked" != "$V" ]]; then
    echo "error: Cargo.lock has $member at '${locked:-<missing>}', expected $V." >&2
    exit 1
  fi
done

echo "Bumped Cargo.toml + Cargo.lock to $V"
