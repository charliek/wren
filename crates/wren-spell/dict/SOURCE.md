# The bundled dictionary

`en_US.aff` and `en_US.dic` are SCOWL's Hunspell `en_US` dictionary (the
standard size, SCOWL size 60; not `en_US-large`), copied unmodified from
SCOWL's published release archive.

- Project: SCOWL (Spell Checker Oriented Word Lists), https://wordlist.aspell.net/dicts/
- Release: 2026.02.25 (tag `rel-2026.02.25` of https://github.com/en-wl/wordlist)
- Archive: https://github.com/en-wl/wordlist/releases/download/rel-2026.02.25/hunspell-en_US-2026.02.25.zip
  - SHA-256 `ac8e73310e951d88c52c2cf2ba54ceaca34f8486a81630ac8a75dc5f931179f9`

| File | SHA-256 |
|---|---|
| `en_US.aff` | `526e1dc32d1c0e8806dd72809c017b5546f2104b08b9424a4c47311aca526cca` |
| `en_US.dic` | `ab012ae86ce4b46b60db5b150869288de9c4a87bc747911764e582125b1b961e` |

The archive's third file, `README_en_US.txt`, carries the copyright and
permission notices of SCOWL and of the affix file. It ships with every bundle
from `third_party/scowl/README_en_US.txt` (SHA-256
`d63c711523fc9504fb71e1278489c6d7bb8cafcba75158c107cd17844955470f`), and
`THIRD_PARTY_NOTICES.txt` names it (`tools/package.py`).

To update: download the newer release's `hunspell-en_US-<release>.zip`,
replace the three files, and update this page, the release named in
`tools/package.py`'s `NOTICES`, and the hashes above. The packaging unit
tests check that the release and the hashes agree.
