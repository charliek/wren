# Releasing Wren

The general release framework is `cc-plugins:release-workflows`; this file
documents what's specific to this repo.

## TL;DR

    /release-workflows:release v<NEXT_VERSION>

That's it. Everything else is automatic (after the [one-time setup](#one-time-github-setup)).

## What happens

1. **`release-workflows:release`** (LLM, local):
   - Verifies branch (`main`) + clean tree + `ci-success` green on HEAD
   - Asks/confirms the version
   - Turns `CHANGELOG.md`'s `[Unreleased]` notes (plus `git log
     v<previous>..HEAD`) into a `## vX.Y.Z — YYYY-MM-DD` entry, commits it as
     `docs(changelog): vX.Y.Z entry`
   - Runs `scripts/release/update-version.sh X.Y.Z` → bumps `Cargo.toml` and
     `Cargo.lock`
   - Commits as `chore(version): bump to X.Y.Z`
   - Tags `vX.Y.Z` (annotated) on the version commit
   - `git push --follow-tags` (the admin role bypasses the `ci-success` rule)

2. **`release.yml`** (CI, on the tag):
   - `version-check`: the tag (prerelease suffix included) equals
     `[workspace.package].version`
   - `ci-gate`: polls the tagged commit's `ci-success` check (ci.yml's
     fail-closed aggregate, `tools/ci_gate.py`) and refuses to release unless
     it is green; waits up to 45 minutes for an in-flight run
   - `create-release`: extracts this tag's `CHANGELOG.md` section →
     `gh release create --draft` (`--prerelease` for `vX.Y.Z-…` tags)
   - `linux` (amd64 on `ubuntu-24.04`, arm64 on `ubuntu-24.04-arm`, each
     native): `make deb` builds the release binary, `tools/package.py
     --system-layout` stages the `/usr` tree, nfpm packages it as
     `wren_X.Y.Z_<arch>.deb` (`packaging/nfpm.yaml`).
     `scripts/linux/verify-deb.sh --clean-install` checks the control fields
     and payload, validates the desktop entry, and installs the package into a
     clean `ubuntu:24.04` container where every library the binary dlopens
     must resolve. Then it uploads the `.deb`.
   - `mac` (`macos-26`, Apple silicon): `make app` builds an arm64 release
     `Wren.app` (bundle id `ai.stridelabs.Wren`, version from the tag) and
     signs it (`scripts/macos/sign-app.sh`); asserts the bundle's id, version,
     arch and signature; notarizes + staples the app; `make dmg` packages
     `Wren-X.Y.Z.dmg`; notarizes + staples the DMG; checks the app inside the
     DMG carries its own ticket; uploads the DMG. Signing and notarization are
     all-or-nothing on the six Apple secrets (see [Secrets](#secrets)); until
     they exist the DMG is ad-hoc signed, carries `FIRST-LAUNCH.txt`, and the
     release body gets the Gatekeeper-bypass note.
   - **`publish-release`**: `tools/release_assets.py check` asserts the draft
     holds exactly the amd64 `.deb`, the arm64 `.deb` and the DMG, each named
     for the tag and at least 1 MiB, and nothing else; then flips the draft
     public. **This is the only irreversible step in the pipeline.**
   - `dispatch-apt-charliek`: fires a `repository_dispatch` at
     `charliek/apt-charliek` so the `.deb`s land on `apt.stridelabs.ai`, with
     a release-bot App token scoped to that repo. Skipped for prereleases
     (the receiver tracks `include_prerelease: false`).

No auto-update feed (Sparkle) and no Homebrew cask yet: macOS users download
each DMG from the Release page.

## Draft-until-complete, and how to recover

The Release stays a **draft** until `publish-release` has seen all three
assets. A draft is invisible to anyone without push access, including
apt-charliek, whose `collect-debs.sh` walks releases newest-first and only
*warns* past one with no matching `.deb`; dispatching against a release whose
`.deb`s are missing would silently republish the previous version.

- **A build job failed**: the draft stays a draft; nothing is public and no
  dispatch fires (`publish-release` has a plain `needs: [linux, mac]`). Inspect
  with `gh release view vX.Y.Z --json isDraft,assets`, then **re-run** the
  failed jobs from the Actions UI: `create-release` reuses the draft and every
  upload uses `--clobber`. Or **discard** it with `gh release delete vX.Y.Z
  --yes` (deletes the release, not the tag) and re-run the workflow.
- **`publish-release` failed its assertions**: the draft is untouched and the
  error names the missing, duplicated, truncated or unexpected asset. Fix it
  and re-run the job; against an already-published release it is a no-op.
- **Re-running the whole workflow after a successful release fails at
  `create-release` by design**: it refuses to rebuild into a published
  release. Re-run only the job you need (e.g. `dispatch-apt-charliek`), or
  `gh release delete vX.Y.Z --yes` first, or cut a new patch tag.

## Version files this repo owns

`scripts/release/update-version.sh` bumps:

- `Cargo.toml`: `[workspace.package].version`, the canonical Wren version
  (both `wren-core` and `wren-iced` inherit it). Only that table's key is
  touched.
- `Cargo.lock`: the two workspace member entries, via
  `cargo update --workspace --offline` (CI builds with `--locked`).

NOT bumped:

- `pyproject.toml` / `uv.lock`: they version the Python test harness only.

A prerelease (`v0.2.0-rc1`) puts the full version in `Cargo.toml`, the `.deb`
(`0.2.0~rc1`, so it sorts before `0.2.0`) and the DMG name; the macOS bundle
versions (`CFBundleShortVersionString`/`CFBundleVersion`) must be numeric and
record `0.2.0`.

## Snapshot / dev versioning

Not used. Main between releases shows the last released version. If a build
identity beyond "last released" is ever needed, derive it at build time from
`git describe --tags --dirty` rather than snapshotting the source tree.

## Secrets

| Secret | Purpose | Required? |
|---|---|---|
| `RELEASE_BOT_CLIENT_ID` | `charliek-release-bot` GitHub App client ID (`create-github-app-token`'s `client-id`) | required for the apt-charliek dispatch; unset → the dispatch is skipped with a warning and apt-charliek picks the release up on its next scheduled run |
| `RELEASE_BOT_APP_KEY` | That App's private key (`.pem`) | same |
| `MACOS_CERTIFICATE_P12_BASE64` | Developer ID Application cert + key, base64 of the `.p12` | optional, one of six gated together as `CAN_NOTARIZE` |
| `MACOS_CERTIFICATE_PASSWORD` | The `.p12` export password | optional (`CAN_NOTARIZE`) |
| `WREN_DEVELOPER_ID_IDENTITY` | codesign identity, e.g. `Developer ID Application: Name (TEAMID)`; the same string as roost's `ROOST_DEVELOPER_ID_IDENTITY` when one cert signs both | optional (`CAN_NOTARIZE`) |
| `APPLE_ID` | Apple ID email for notarytool | optional (`CAN_NOTARIZE`) |
| `APPLE_TEAM_ID` | 10-character Apple team ID | optional (`CAN_NOTARIZE`) |
| `APPLE_APP_SPECIFIC_PASSWORD` | App-specific password from appleid.apple.com (not the Apple ID password) | optional (`CAN_NOTARIZE`) |

The six Apple secrets are all-or-nothing: with any one unset the mac job ships
an ad-hoc signed, un-notarized DMG (with the first-launch note) and still
succeeds. Add all six and the next release is Developer ID signed and
notarized with no code change. The cert, team and Apple ID are shared with
roost, so the values are the ones roost already uses (kept locally in roost's
`.secrets/cert.p12` + `.secrets/apple.env`).

## One-time GitHub setup

`charliek/wren` has no ruleset, no branch protection and no secrets yet. Run
these once, in order, before the first release.

1. **Install the release-bot App on `charliek/wren`.** It is already installed
   on roost and apt-charliek. In <https://github.com/settings/installations>,
   open `charliek-release-bot` → Configure → Repository access → add
   `charliek/wren` (or, if each repo has its own install, "Install" it again
   selecting only `charliek/wren`).

2. **Set the release-bot secrets.** The client ID is on the App's settings
   page; the `.pem` is in the password manager.

   ```bash
   gh secret set RELEASE_BOT_CLIENT_ID -R charliek/wren -b "<client id>"
   gh secret set RELEASE_BOT_APP_KEY   -R charliek/wren < ~/Downloads/charliek-release-bot.private-key.pem
   gh secret list -R charliek/wren
   ```

3. **Verify the App wiring:**

   ```bash
   gh workflow run sanity-check-app.yml -R charliek/wren
   gh run watch -R charliek/wren "$(gh run list -R charliek/wren --workflow sanity-check-app.yml --limit 1 --json databaseId -q '.[0].databaseId')"
   ```

   It must print `charliek/wren`, a bot identity ending in `[bot]`, and
   `charliek/apt-charliek`.

4. **Protect `main` with a ruleset** requiring `ci-success`, with the App
   (App ID `3902108`, the same as on roost) and the admin role as bypass
   actors:

   ```bash
   cat > /tmp/wren-main-ruleset.json <<'JSON'
   {
     "name": "main-protection",
     "target": "branch",
     "enforcement": "active",
     "conditions": { "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] } },
     "rules": [
       { "type": "deletion" },
       { "type": "non_fast_forward" },
       { "type": "required_status_checks",
         "parameters": {
           "required_status_checks": [ { "context": "ci-success" } ],
           "strict_required_status_checks_policy": false } }
     ],
     "bypass_actors": [
       { "actor_id": 3902108, "actor_type": "Integration",    "bypass_mode": "always" },
       { "actor_id": 5,       "actor_type": "RepositoryRole", "bypass_mode": "always" }
     ]
   }
   JSON
   gh api -X POST /repos/charliek/wren/rulesets --input /tmp/wren-main-ruleset.json \
     | jq '{id, name, enforcement, bypass_actors}'
   ```

   Record the printed id in [Branch protection](#branch-protection) below.

5. **Optional, for notarized DMGs:** set the six Apple secrets (any time; the
   next release picks them up):

   ```bash
   R=charliek/wren
   base64 -i cert.p12 | gh secret set MACOS_CERTIFICATE_P12_BASE64 -R "$R"
   gh secret set MACOS_CERTIFICATE_PASSWORD  -R "$R" -b "<p12 password>"
   gh secret set WREN_DEVELOPER_ID_IDENTITY  -R "$R" -b "Developer ID Application: <Name> (<TEAMID>)"
   gh secret set APPLE_ID                    -R "$R" -b "<apple id email>"
   gh secret set APPLE_TEAM_ID               -R "$R" -b "<TEAMID>"
   gh secret set APPLE_APP_SPECIFIC_PASSWORD -R "$R" -b "<app-specific password>"
   ```

6. **Register the apt package:** merge the `add-wren` branch of
   `charliek/apt-charliek` (adds `wren` to `packages.yaml`). Until Wren's first
   release, apt-charliek skips it as "no releases yet".

## Branch protection

`main` is protected by ruleset `main-protection` (id `26765325`) with `required_status_checks=['ci-success']` and two bypass
actors:

- `charliek-release-bot` (App ID `3902108`, type `Integration`): for
  post-build pushes to `main`. Wren's release makes none today; it is listed
  per the convention so a future one (an appcast) needs no ruleset change.
- Admin role (id `5`, type `RepositoryRole`): lets `/release-workflows:release`
  push the changelog + version commits and the tag before `ci-success` exists
  on them.

Inspect or edit at <https://github.com/charliek/wren/rules>.

## Building release artifacts locally

The workflow only calls Makefile targets, so the same commands work locally:

    make deb                      # Linux: target/dist/wren_<ver>_<arch>.deb (needs nfpm)
    scripts/linux/verify-deb.sh target/dist/wren_<ver>_<arch>.deb <ver> --clean-install
    make app dmg                  # macOS: target/dist/macos/Wren.app + target/dist/Wren-<ver>.dmg

`RELEASE_VERSION` defaults to the Cargo version. For a local Developer ID
build, import the cert into a keychain first (as the workflow's import step
does), export `WREN_DEVELOPER_ID_IDENTITY`, and notarize with
`scripts/macos/notarize.sh` using `WREN_NOTARY_PROFILE` or the three
`APPLE_*` variables.

## When things break

| Symptom | Cause | Fix |
|---|---|---|
| `git push` rejected: `Required status check "ci-success"` | Pusher not in the ruleset's bypass list | Confirm both the App (`3902108`, Integration) and the admin role (`5`, RepositoryRole) are in `main-protection`'s `bypass_actors` (see the convention's `github-app.md`) |
| `scripts/release/update-version.sh` not found | Convention not adopted | Run `/release-workflows:setup` |
| `update-version.sh`: "Cargo.toml's [workspace.package].version did not update" | `[workspace.package]` no longer has a column-0 `version = "…"` line | Restore that layout, or adjust the sed range in the script |
| Tag pushed, `version-check` fails | The tag and `[workspace.package].version` disagree: the tagged commit didn't run `update-version.sh`, or the tag is mistyped | While the release is still a draft (or absent), fix forward and move the tag: `git push origin :refs/tags/vX.Y.Z`, re-tag, push. **Once the release is public, never move the tag**; cut a new patch version |
| `ci-gate` fails or times out | `ci-success` red (or never ran) on the tagged commit | Nothing is public yet. Fix forward on `main`, let CI pass, then move the tag as above |
| `make deb` / `make app` fail: "release version … does not match the Cargo version" | `tools/package.py` saw a `RELEASE_VERSION` that is not the tree's | Same as the `version-check` row; it guards local builds too |
| `linux` fails at "Verify the .deb": "not resolvable after a --no-install-recommends install" | The binary dlopens a library no Depends supplies | Add the package to `packaging/nfpm.yaml`'s `depends` (and the soname to `verify-deb.sh`; `tools/wrentest_unit/test_release_contract.py` keeps the two in step) |
| `linux` fails at "Install nfpm": checksum mismatch | A corrupted or changed download | Re-run; if it persists, compare against nfpm's published `checksums.txt` before bumping `NFPM_VERSION` |
| `mac` fails at notarize with `status: Invalid` | Apple refused the submission; `notarize.sh` prints Apple's per-binary reasons | Read them. "Not signed with a valid Developer ID certificate" + "no secure timestamp" means the app was ad-hoc signed: a step after `make app` rebuilt or re-signed it |
| `mac` fails at "Assert the app carries its Developer ID signature" | Something re-signed the bundle between signing and notarizing, or the identity secret names the wrong cert | Check `WREN_DEVELOPER_ID_IDENTITY` against `security find-identity -v -p codesigning` |
| `mac` fails at `hdiutil create` after 5 attempts | Persistent runner disk contention | Re-run the `mac` job |
| `create-release` fails: "already exists and is PUBLISHED" | Re-running the whole workflow for a tag that already shipped | Deliberate. Re-run only the job you need, or delete the release first |
| `publish-release` fails: "expected exactly one asset named …" | A build job uploaded nothing, or the same name twice | The draft is untouched: inspect with `gh release view vX.Y.Z --json assets`, fix, re-run |
| `publish-release` fails: "unexpected asset …" | A reused draft still carries an asset from an earlier attempt | `gh release delete-asset vX.Y.Z <name>` and re-run. This matters: apt-charliek globs `wren_*.deb`, so a stale one would ship |
| `publish-release` fails: "… looks truncated" | An upload was interrupted | Re-run the job that produced it (`--clobber` overwrites) |
| Release stuck as a draft | `publish-release` never ran or never passed | See [Draft-until-complete](#draft-until-complete-and-how-to-recover) |
| `dispatch-apt-charliek` warns `RELEASE_BOT_CLIENT_ID not set`, or fails minting a token | The secret is unset, or the App is not installed on `charliek/apt-charliek` | Run `sanity-check-app.yml`; fix the secret or the install, then re-run the job. apt-charliek also re-scans every package on its next scheduled run, so a missed dispatch self-heals |
| `apt install wren` doesn't find the new version | The apt-charliek publish run failed, or `wren` is not in its `packages.yaml` | Check <https://github.com/charliek/apt-charliek/actions>; re-dispatch by re-running `dispatch-apt-charliek` |
| macOS: "Wren wants access to control Finder" on first delete | Expected: deleting moves notes to the Trash through Finder (`trash` crate, osascript) | Allow it once; System Settings → Privacy & Security → Automation lists it afterwards |

## Adopting the convention (for new contributors)

Read [`cc-plugins/plugins/release-workflows/references/convention.md`](https://github.com/charliek/cc-plugins/blob/main/plugins/release-workflows/references/convention.md)
in the framework repo. It defines the contract every file in this repo's
`scripts/release/` and `.github/workflows/release.yml` is written against.
roost (`charliek/roost`) runs the same pipeline in production and is the
reference for anything this file leaves out.

## Notes for this repo

- `tools/package.py` is the single source of the bundle layout. `make deb`
  asks it for the `/usr` tree (`--system-layout`: the desktop entry keeps
  `Exec=wren`/`Icon=wren`), and nfpm only packages that tree; `make app`
  asks it for `Wren.app`. The `.deb`'s runtime `Depends` are hand-kept in
  `packaging/nfpm.yaml`, because winit and wgpu dlopen the whole graphics
  stack and `readelf -d` shows none of it.
- The Developer ID signature uses `packaging/macos/Wren.entitlements`
  (`com.apple.security.automation.apple-events`), and `Info.plist` carries
  `NSAppleEventsUsageDescription`, both for the Finder-based trash path above.
- `tools/release_assets.py` names the uploaded assets and holds the
  `publish-release` rules; `tools/wrentest_unit/test_release_assets.py` and
  `test_release_contract.py` test them and the workflow's structure.
