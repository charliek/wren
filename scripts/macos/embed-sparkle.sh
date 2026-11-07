#!/usr/bin/env bash
# Embed the staged Sparkle.framework into Wren.app, before signing.
#
#   scripts/macos/embed-sparkle.sh target/dist/macos/Wren.app
#
# Copies third_party/sparkle/out/Sparkle.framework (run fetch.sh first) to
# Contents/Frameworks with its symlinks intact, and Sparkle's license to
# Contents/Resources/third_party/sparkle/LICENSE. The app dlopens the framework
# at launch (crates/wren-sparkle), so a bundle without it still runs, with
# "Check for Updates…" disabled. tools/package.py writes the SUFeedURL and
# SUPublicEDKey the updater reads; scripts/macos/sign-app.sh signs the result.
set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

APP="${1:-}"
[ -n "${APP}" ] || die "usage: $0 <path/to/Wren.app>"
[ -f "${APP}/Contents/Info.plist" ] || die "${APP} is not an app bundle"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STAGE="${ROOT}/third_party/sparkle/out"
[ -L "${STAGE}/Sparkle.framework/Sparkle" ] || die "Sparkle is not staged; run third_party/sparkle/fetch.sh"
[ -f "${STAGE}/LICENSE" ] || die "${STAGE}/LICENSE is missing; rerun third_party/sparkle/fetch.sh --force"

FRAMEWORKS="${APP}/Contents/Frameworks"
rm -rf "${FRAMEWORKS}/Sparkle.framework"
mkdir -p "${FRAMEWORKS}"
cp -R "${STAGE}/Sparkle.framework" "${FRAMEWORKS}/Sparkle.framework"
[ -L "${FRAMEWORKS}/Sparkle.framework/Sparkle" ] && [ -f "${FRAMEWORKS}/Sparkle.framework/Sparkle" ] \
  || die "the embedded framework lost its symlinks"

NOTICES="${APP}/Contents/Resources/third_party/sparkle"
mkdir -p "${NOTICES}"
install -m 0644 "${STAGE}/LICENSE" "${NOTICES}/LICENSE"
echo "==> Embedded $(cut -d' ' -f1 "${STAGE}/.stamp" 2>/dev/null || echo '?') Sparkle.framework in ${APP}"
