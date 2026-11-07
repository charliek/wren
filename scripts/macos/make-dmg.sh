#!/usr/bin/env bash
# Package Wren.app into a drag-install DMG: the app plus an /Applications
# symlink, and FIRST-LAUNCH.txt while the build is not on the notarization path.
#
#   scripts/macos/make-dmg.sh target/dist/macos/Wren.app 0.1.0 target/dist/Wren-0.1.0.dmg
#
# Uses hdiutil, which is headless-safe on GitHub's GUI-less macOS runners
# (create-dmg's Finder AppleScript can hang there). Adapted from roost's
# mac/scripts/make-dmg.sh.
set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

[ "$#" -eq 3 ] || die "usage: $0 <path/to/Wren.app> <version> <output.dmg>"
APP="$1"
VERSION="$2"
DMG="$3"
[ -d "${APP}" ] || die "${APP} not found; run make app first"
[ "$(basename "${APP}")" = Wren.app ] || die "expected a bundle named Wren.app, got $(basename "${APP}")"

mkdir -p "$(dirname "${DMG}")"
rm -f "${DMG}"
STAGING="$(mktemp -d)"
trap 'rm -rf "${STAGING}"' EXIT
# ditto keeps the code signature's extended attributes and modes intact.
ditto "${APP}" "${STAGING}/Wren.app"
ln -s /Applications "${STAGING}/Applications"

# Gated on WREN_DEVELOPER_ID_IDENTITY, the same signal sign-app.sh uses to pick
# Developer ID over ad-hoc: with a real identity the build is on the
# notarization path, and the note is omitted.
if [ -z "${WREN_DEVELOPER_ID_IDENTITY:-}" ]; then
  cat > "${STAGING}/FIRST-LAUNCH.txt" <<'EOF'
Wren - first launch on macOS

This build of Wren is ad-hoc signed but not notarized, so macOS Gatekeeper
blocks its first launch. You only need to do this once.

Easiest: drag Wren into the Applications folder, run this once in Terminal,
then open Wren normally:

    xattr -dr com.apple.quarantine /Applications/Wren.app

Or: double-click Wren, dismiss the "Apple could not verify..." warning, then
open System Settings -> Privacy & Security, scroll to the message about Wren,
and click "Open Anyway".

Once a notarized build ships, this goes away and Wren opens with a normal
double-click.
EOF
fi

# hdiutil intermittently fails with "Resource busy" on CI runners (transient
# device/Spotlight contention, often right after codesign touches the bundle).
# Retry, dropping any partial image so a leftover never survives the last try.
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname "Wren ${VERSION}" -srcfolder "${STAGING}" \
       -ov -format UDZO "${DMG}" >/dev/null; then
    break
  fi
  rm -f "${DMG}"
  [ "${attempt}" -lt 5 ] || die "hdiutil create failed after ${attempt} attempts"
  echo "    hdiutil create failed (attempt ${attempt}); retrying in 3s..." >&2
  sleep 3
done

echo "==> DMG: ${DMG}"
ls -lh "${DMG}"
