#!/usr/bin/env bash
# Fetch the official Sparkle release, check it against the pinned SHA-256, and
# stage what Wren.app embeds plus the release tooling:
#
#   out/Sparkle.framework   embedded by scripts/macos/embed-sparkle.sh (cp -R,
#                           keeping the Versions/ symlinks)
#   out/LICENSE             Sparkle's license, shipped in the app's notices
#   out/bin/sign_update     EdDSA-signs a DMG for the appcast (release.yml)
#   out/bin/generate_keys   makes a Sparkle key pair
#
# Nothing here is committed: out/ is git-ignored and rebuilt on demand. The pin
# below is the integrity boundary. Rerunning with an unchanged pin is a no-op.
#
#   third_party/sparkle/fetch.sh           # fetch and stage (idempotent)
#   third_party/sparkle/fetch.sh --force   # discard the stage and refetch
set -euo pipefail

# Bump deliberately: change both lines, rerun with --force, update README.md.
SPARKLE_VERSION="2.9.5"
SPARKLE_SHA256="015336b601493e05c237964954bff6191370003d94edefe663724c88840d73cc"
SPARKLE_URL="https://github.com/sparkle-project/Sparkle/releases/download/${SPARKLE_VERSION}/Sparkle-${SPARKLE_VERSION}.tar.xz"

OUT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/out"
STAMP_FILE="${OUT_DIR}/.stamp"
STAMP_WANT="${SPARKLE_VERSION} ${SPARKLE_SHA256}"

case "${1:-}" in
  --force) rm -rf "${OUT_DIR}" ;;
  "") ;;
  *) echo "usage: $0 [--force]" >&2; exit 1 ;;
esac

# The framework must keep its symlinks (codesign rejects a flattened framework,
# and the app dlopens the top-level Sparkle.framework/Sparkle link), and every
# component scripts/macos/sign-app.sh signs must be present.
validate_stage() {
  local fw="${OUT_DIR}/Sparkle.framework" problems=0 path
  local versions="${fw}/Versions/B"
  if [ ! -L "${fw}/Sparkle" ] || [ ! -f "${fw}/Sparkle" ]; then
    echo "error: ${fw}/Sparkle is not a resolvable symlink" >&2
    problems=1
  fi
  for path in "${versions}/XPCServices/Installer.xpc" "${versions}/XPCServices/Downloader.xpc" \
              "${versions}/Updater.app"; do
    [ -d "${path}" ] || { echo "error: missing Sparkle component ${path}" >&2; problems=1; }
  done
  for path in "${versions}/Autoupdate" "${OUT_DIR}/LICENSE"; do
    [ -f "${path}" ] || { echo "error: missing ${path}" >&2; problems=1; }
  done
  for path in sign_update generate_keys; do
    [ -x "${OUT_DIR}/bin/${path}" ] || { echo "error: missing Sparkle tool ${OUT_DIR}/bin/${path}" >&2; problems=1; }
  done
  return "${problems}"
}

if [ -f "${STAMP_FILE}" ] && [ "$(cat "${STAMP_FILE}")" = "${STAMP_WANT}" ] \
   && validate_stage 2>/dev/null; then
  echo "==> Sparkle ${SPARKLE_VERSION} already staged in ${OUT_DIR}"
  exit 0
fi

echo "==> Fetching Sparkle ${SPARKLE_VERSION}"
work="$(mktemp -d -t wren-sparkle)"
trap 'rm -rf "${work}"' EXIT
curl -fsSL --retry 3 -o "${work}/sparkle.tar.xz" "${SPARKLE_URL}"
echo "${SPARKLE_SHA256}  ${work}/sparkle.tar.xz" | shasum -a 256 -c - >/dev/null \
  || { echo "error: SHA-256 mismatch for ${SPARKLE_URL}; refusing to stage it" >&2; exit 1; }

mkdir -p "${work}/extract"
tar -xJf "${work}/sparkle.tar.xz" -C "${work}/extract"
rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}/bin"
cp -R "${work}/extract/Sparkle.framework" "${OUT_DIR}/Sparkle.framework"
cp "${work}/extract/LICENSE" "${OUT_DIR}/LICENSE"
cp "${work}/extract/bin/sign_update" "${work}/extract/bin/generate_keys" "${OUT_DIR}/bin/"
chmod +x "${OUT_DIR}/bin/sign_update" "${OUT_DIR}/bin/generate_keys"
validate_stage || { echo "error: the staged Sparkle layout is incomplete; see ${OUT_DIR}" >&2; exit 1; }
printf '%s\n' "${STAMP_WANT}" > "${STAMP_FILE}"
echo "==> Sparkle ${SPARKLE_VERSION} staged in ${OUT_DIR}"
