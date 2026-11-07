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
# An embedded Sparkle.framework (scripts/macos/embed-sparkle.sh) is signed
# first, component by component from the inside out, never with --deep: its
# XPC services, Autoupdate, and Updater.app each need their own signature, and
# Sparkle's own entitlements, not Wren's. A framework signed any other way can
# pass codesign and notarization yet fail when an update is installed.
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
  SIGN=(--force --options runtime --timestamp --sign "${IDENTITY}")
else
  SIGN=(--force --options runtime --sign -)
fi

SPARKLE="${APP}/Contents/Frameworks/Sparkle.framework"
if [ -e "${SPARKLE}" ]; then
  versions="${SPARKLE}/Versions/B"
  for component in "${versions}/XPCServices/Installer.xpc" "${versions}/XPCServices/Downloader.xpc" \
                   "${versions}/Autoupdate" "${versions}/Updater.app"; do
    [ -e "${component}" ] || die "Sparkle component missing: ${component}"
  done
  echo "==> Signing Sparkle.framework (inside out)"
  codesign "${SIGN[@]}" "${versions}/XPCServices/Installer.xpc"
  codesign "${SIGN[@]}" --preserve-metadata=entitlements "${versions}/XPCServices/Downloader.xpc"
  codesign "${SIGN[@]}" "${versions}/Autoupdate"
  codesign "${SIGN[@]}" "${versions}/Updater.app"
  codesign "${SIGN[@]}" "${SPARKLE}"
fi

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
