#!/usr/bin/env bash
# Prove, before a release builds anything, that the Sparkle key pair it ships
# with is the one installed copies of Wren.app trust:
#
#   * the committed public key (make app stamps it into Info.plist as
#     SUPublicEDKey) is not the TEST-ONLY fixture key;
#   * $SPARKLE_ED_PRIVATE_KEY is its private half;
#   * sign_update, given that key, signs a probe file the public key verifies,
#     which is the check Sparkle makes on a user's Mac.
#
# A mismatch would publish updates every installed app silently rejects.
#
# $SPARKLE_ED_PRIVATE_KEY is base64 of the key file (whose own contents are
# base64 of the 32-byte Ed25519 seed, the format `generate_keys -x` writes).
# No key material is printed.
#
#   SPARKLE_ED_PRIVATE_KEY=... scripts/macos/verify-sparkle-key.sh packaging/macos/sparkle-ed-public-key.txt
#
# Needs third_party/sparkle/fetch.sh's sign_update and uv (for `cryptography`).
set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

[ "$#" -eq 1 ] || die "usage: $0 <public-key-file>"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SIGN_UPDATE="${ROOT}/third_party/sparkle/out/bin/sign_update"
TEST_KEY="${ROOT}/tools/wrentest/fixtures/sparkle/TEST-ONLY-public-ed-key.txt"

[ -s "$1" ] || die "$1 is missing or empty"
public="$(tr -d '[:space:]' < "$1")"
[ "${public}" != "$(tr -d '[:space:]' < "${TEST_KEY}")" ] || die "$1 is the TEST-ONLY fixture key"
[ -n "${SPARKLE_ED_PRIVATE_KEY:-}" ] || die "SPARKLE_ED_PRIVATE_KEY is unset (RELEASING.md)"
[ -x "${SIGN_UPDATE}" ] || die "${SIGN_UPDATE} is missing; run third_party/sparkle/fetch.sh"

crypto() {
  uv run --no-project --quiet --with 'cryptography==46.0.3' python3 -I -c '
import base64, sys
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

if sys.argv[1] == "derive":
    try:
        seed = base64.b64decode(open(sys.argv[2], "rb").read().strip(), validate=True)
    except ValueError:
        sys.exit("the key file is not base64")
    if len(seed) != 32:
        sys.exit(f"the key file holds {len(seed)} bytes, not a 32-byte seed")
    public = Ed25519PrivateKey.from_private_bytes(seed).public_key()
    print(base64.b64encode(public.public_bytes(Encoding.Raw, PublicFormat.Raw)).decode())
else:
    public = Ed25519PublicKey.from_public_bytes(base64.b64decode(sys.argv[2], validate=True))
    try:
        public.verify(base64.b64decode(sys.argv[3], validate=True), open(sys.argv[4], "rb").read())
    except InvalidSignature:
        sys.exit("the probe signature does not verify")
' "$@"
}

work="$(mktemp -d)"
chmod 700 "${work}"
trap 'rm -rf "${work}"' EXIT
printf '%s' "${SPARKLE_ED_PRIVATE_KEY}" | base64 --decode > "${work}/key" 2>/dev/null \
  || die "SPARKLE_ED_PRIVATE_KEY is not base64"
derived="$(crypto derive "${work}/key")" || die "SPARKLE_ED_PRIVATE_KEY is not a Sparkle key file (see RELEASING.md)"
[ "${derived}" = "${public}" ] || die "SPARKLE_ED_PRIVATE_KEY is not the private half of $1"
echo "ok: SPARKLE_ED_PRIVATE_KEY is the private half of $1"

head -c 4096 /dev/urandom > "${work}/probe"
"${SIGN_UPDATE}" --ed-key-file "${work}/key" "${work}/probe" > "${work}/sign.txt" \
  || die "sign_update could not sign with SPARKLE_ED_PRIVATE_KEY"
rm -f "${work}/key"
signature="$(sed -n 's/.*sparkle:edSignature="\([^"]*\)".*/\1/p' "${work}/sign.txt")"
[ -n "${signature}" ] || die "sign_update printed no signature"
crypto verify "${public}" "${signature}" "${work}/probe" || die "sign_update signs for a different key"
echo "ok: sign_update's probe signature verifies against $1"
