#!/usr/bin/env bash
# Verify a built Wren .deb before it is uploaded to a release.
#
#   scripts/linux/verify-deb.sh target/dist/wren_0.1.0_amd64.deb 0.1.0 [--clean-install]
#
# Always: the control fields (package, nfpm's Debian version for the release
# version, the architecture the file name claims), the payload paths and the
# executable's mode, and desktop-file-validate on the packaged desktop entry
# when that tool is installed.
#
# --clean-install also installs the package with `apt-get install
# --no-install-recommends` into a fresh ubuntu:24.04 container and checks that
# every library the binary dlopens resolves there. That is the only check of
# the Depends closure: the build runner already carries the whole graphics
# stack, so a Depends line that forgot a library would pass anywhere else. The
# container must match the build host's architecture (the release runs it on
# each native runner).
set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

[ "$#" -ge 2 ] || die "usage: $0 <wren_*.deb> <release-version> [--clean-install]"
DEB="$1"
RELEASE_VERSION="$2"
CLEAN_INSTALL="${3:-}"
[ -z "${CLEAN_INSTALL}" ] || [ "${CLEAN_INSTALL}" = "--clean-install" ] || die "unknown option: ${CLEAN_INSTALL}"
[ -f "${DEB}" ] || die "${DEB} not found"

# nfpm's semver schema: the first `-` becomes `~` (0.1.0-rc1 -> 0.1.0~rc1).
WANT_VERSION="${RELEASE_VERSION/-/\~}"
case "$(basename "${DEB}")" in
  wren_*_amd64.deb) WANT_ARCH=amd64 ;;
  wren_*_arm64.deb) WANT_ARCH=arm64 ;;
  *) die "$(basename "${DEB}") is not named wren_<version>_<amd64|arm64>.deb" ;;
esac

field() { dpkg-deb --field "${DEB}" "$1"; }
[ "$(field Package)" = wren ] || die "Package is '$(field Package)', want 'wren'"
[ "$(field Version)" = "${WANT_VERSION}" ] || die "Version is '$(field Version)', want '${WANT_VERSION}'"
[ "$(field Architecture)" = "${WANT_ARCH}" ] || die "Architecture is '$(field Architecture)', want '${WANT_ARCH}'"
echo "OK: wren ${WANT_VERSION} ${WANT_ARCH}"

listing="$(dpkg-deb --contents "${DEB}")"
expect=(
  ./usr/bin/wren
  ./usr/share/applications/wren.desktop
  ./usr/share/icons/hicolor/scalable/apps/wren.svg
  ./usr/share/doc/wren/copyright
  ./usr/share/doc/wren/LICENSE
  ./usr/share/doc/wren/THIRD_PARTY_NOTICES.txt
  ./usr/share/doc/wren/DEPENDENCY_NOTICES.txt
  ./usr/share/doc/wren/third_party/inter/LICENSE.txt
  ./usr/share/doc/wren/third_party/jetbrains-mono/OFL.txt
)
for size in 16 24 32 48 64 128 256 512; do
  expect+=("./usr/share/icons/hicolor/${size}x${size}/apps/wren.png")
done
for path in "${expect[@]}"; do
  awk -v p="${path}" '$6 == p { found = 1 } END { exit !found }' <<<"${listing}" \
    || die "${path} is missing from the package"
done
awk '$6 == "./usr/bin/wren" && $1 == "-rwxr-xr-x" && $2 == "root/root" { found = 1 } END { exit !found }' \
  <<<"${listing}" || die "/usr/bin/wren is not a root-owned 0755 file"
if awk '$6 !~ /^\.\/usr\// { bad = 1 } END { exit !bad }' <<<"${listing}"; then
  die "the package installs outside /usr"
fi
echo "OK: payload (${#expect[@]} expected paths)"

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
dpkg-deb --extract "${DEB}" "${WORK}/root"
grep -qx 'Exec=wren' "${WORK}/root/usr/share/applications/wren.desktop" || die "desktop entry lacks Exec=wren"
grep -qx 'Icon=wren' "${WORK}/root/usr/share/applications/wren.desktop" || die "desktop entry lacks Icon=wren"
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "${WORK}/root/usr/share/applications/wren.desktop"
  echo "OK: desktop-file-validate"
else
  echo "note: desktop-file-validate not installed; skipped"
fi

if [ "${CLEAN_INSTALL}" = "--clean-install" ]; then
  command -v docker >/dev/null 2>&1 || die "--clean-install needs docker"
  cp "${DEB}" "${WORK}/wren.deb"
  # The sonames the release binary dlopens (winit, wgpu, xkbcommon-dl). Keep in
  # step with packaging/nfpm.yaml's depends.
  sonames="libxkbcommon.so.0 libxkbcommon-x11.so.0 libwayland-client.so.0 libwayland-egl.so.1 libX11.so.6 libX11-xcb.so.1 libXcursor.so.1 libXi.so.6 libEGL.so.1"
  docker run --rm -e DEBIAN_FRONTEND=noninteractive -e SONAMES="${sonames}" \
    -v "${WORK}/wren.deb:/tmp/wren.deb:ro" ubuntu:24.04 bash -euo pipefail -c '
      apt-get update -qq
      apt-get install -y -qq --no-install-recommends /tmp/wren.deb >/dev/null
      dpkg -s wren | grep -q "^Status: install ok installed$"
      test -x /usr/bin/wren
      # Read the cache once: `ldconfig -p | grep -q` under pipefail fails
      # whenever grep exits early and ldconfig takes a SIGPIPE.
      cache="$(ldconfig -p)"
      missing=""
      for so in ${SONAMES}; do
        grep -q "^[[:space:]]*${so} " <<<"${cache}" || missing="${missing} ${so}"
      done
      if [ -n "${missing}" ]; then
        echo "error: not resolvable after a --no-install-recommends install:${missing}" >&2
        exit 1
      fi
      ldd /usr/bin/wren | grep -q "not found" && { ldd /usr/bin/wren; exit 1; }
      echo "OK: clean ubuntu:24.04 install resolves every dlopened library"
    '
fi
