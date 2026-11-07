#!/usr/bin/env bash
# Notarize + staple a Wren DMG or Wren.app bundle (an app is zipped for
# submission; notarytool needs an archive). Adapted from roost's
# mac/scripts/notarize.sh, the canonical copy in cc-plugins release-workflows
# (references/mac-signing/notarize.sh.template).
#
#   scripts/macos/notarize.sh target/dist/Wren-0.1.0.dmg
#   scripts/macos/notarize.sh --app target/dist/macos/Wren.app
#
# No-op (exit 0) without credentials, so the release still ships an ad-hoc
# signed, un-notarized DMG until the Apple secrets land. Credentials (either):
#   * WREN_NOTARY_PROFILE: a stored notarytool keychain profile (local:
#       xcrun notarytool store-credentials <name> --apple-id ... --team-id ... --password ...)
#   * APPLE_ID + APPLE_TEAM_ID + APPLE_APP_SPECIFIC_PASSWORD: the CI secrets
set -euo pipefail

MODE="dmg"
if [ "${1:-}" = "--app" ]; then
  MODE="app"
  shift
fi

TARGET="${1:-}"
if [ -z "${TARGET}" ] || [ ! -e "${TARGET}" ]; then
  echo "usage: $0 [--app] <path-to-dmg-or-Wren.app>" >&2
  exit 1
fi

if [ -n "${WREN_NOTARY_PROFILE:-}" ]; then
  AUTH=(--keychain-profile "${WREN_NOTARY_PROFILE}")
elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" ]; then
  AUTH=(--apple-id "${APPLE_ID}" --team-id "${APPLE_TEAM_ID}" --password "${APPLE_APP_SPECIFIC_PASSWORD}")
else
  echo "==> notarize: no credentials set; skipping (the DMG ships un-notarized)."
  echo "    To enable: set WREN_NOTARY_PROFILE, or APPLE_ID + APPLE_TEAM_ID +"
  echo "    APPLE_APP_SPECIFIC_PASSWORD, then re-run."
  echo "    Until then, users clear Gatekeeper once after install with:"
  echo "      xattr -dr com.apple.quarantine /Applications/Wren.app"
  exit 0
fi

# One work dir for the submission zip and notarytool's captured output. BSD
# mktemp only substitutes trailing X's, so take uniqueness from a directory.
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT

# `notarytool submit --wait` exits 0 even when Apple's verdict is Invalid; the
# verdict is in its output. Unchecked, the run walks on to stapler, which fails
# with a bare "Record not found". Read the verdict, and on anything but
# Accepted print Apple's own issue list before stopping.
submit_or_die() {
  local archive="$1"
  local out="${WORK_DIR}/notarytool-submit.out"
  local rc id status

  echo "==> notarytool submit (waits for Apple; usually a few minutes)..."
  set +e
  xcrun notarytool submit "${archive}" "${AUTH[@]}" --wait 2>&1 | tee "${out}"
  rc="${PIPESTATUS[0]}"
  set -e

  # Progress lines read `Current status: In Progress`; a line that STARTS with
  # `status:` is the final verdict.
  id="$(awk '/^[[:space:]]*id:/ { print $2; exit }' "${out}")"
  status="$(awk '/^[[:space:]]*status:/ { s = $2 } END { print s }' "${out}")"

  if [ "${rc}" -ne 0 ] || [ "${status}" != "Accepted" ]; then
    if [ -n "${id}" ]; then
      echo "==> notarytool log ${id} (Apple's reasons):"
      xcrun notarytool log "${id}" "${AUTH[@]}" || true
    fi
    echo "error: notarization failed for ${archive} (status=${status:-unknown}, notarytool exit ${rc})." >&2
    exit 1
  fi
}

if [ "${MODE}" = "app" ]; then
  ZIP="${WORK_DIR}/$(basename "${TARGET}").zip"
  ditto -c -k --keepParent "${TARGET}" "${ZIP}"
  submit_or_die "${ZIP}"
else
  submit_or_die "${TARGET}"
fi

echo "==> stapler staple"
xcrun stapler staple "${TARGET}"
xcrun stapler validate "${TARGET}"
echo "==> Notarized + stapled: ${TARGET}"
