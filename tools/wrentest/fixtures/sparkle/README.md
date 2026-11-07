# TEST-ONLY Sparkle key pair

Both halves of a throwaway Ed25519 key pair, committed on purpose so
`tools/check_updater.py` can sign a local appcast and build a bundle that
trusts it. Nothing real is signed with it: `scripts/macos/verify-sparkle-key.sh`
refuses to release a bundle carrying this public key, and the production key
lives only in the `SPARKLE_ED_PRIVATE_KEY` secret (RELEASING.md).
