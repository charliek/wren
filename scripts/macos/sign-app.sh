#!/usr/bin/env bash
# Code-sign Wren.app.
#
#   scripts/macos/sign-app.sh target/dist/macos/Wren.app
#
# With WREN_DEVELOPER_ID_IDENTITY set (release CI hands it over only when all
# six Apple secrets exist, the CAN_NOTARIZE gate) this signs with that
# Developer ID, the hardened runtime and a secure timestamp, which
# notarization requires, plus packaging/macos/Wren.entitlements. Without it
# the bundle is ad-hoc signed: it launches locally, and a downloaded copy needs
# the Gatekeeper bypass make-dmg.sh's FIRST-LAUNCH.txt describes.
#
# Wren.app has a single Mach-O (Contents/MacOS/wren) and no nested helpers or
# frameworks, so signing the bundle signs everything in it.
set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

APP="${1:-}"
[ -n "${APP}" ] || die "usage: $0 <path/to/Wren.app>"
[ -x "${APP}/Contents/MacOS/wren" ] || die "${APP}/Contents/MacOS/wren is missing or not executable"
command -v codesign >/dev/null 2>&1 || die "codesign not found (install the Xcode Command Line Tools)"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENTITLEMENTS="${ROOT}/packaging/macos/Wren.entitlements"
IDENTITY="${WREN_DEVELOPER_ID_IDENTITY:-}"

if [ -n "${IDENTITY}" ]; then
  [ -f "${ENTITLEMENTS}" ] || die "${ENTITLEMENTS} not found"
  echo "==> Developer ID codesign (identity: ${IDENTITY})"
  codesign --force --options runtime --timestamp \
    --entitlements "${ENTITLEMENTS}" --sign "${IDENTITY}" "${APP}"
else
  echo "==> Ad-hoc codesign (set WREN_DEVELOPER_ID_IDENTITY for a notarizable build)"
  codesign --force --sign - "${APP}"
fi
codesign --verify --strict --verbose=2 "${APP}"
