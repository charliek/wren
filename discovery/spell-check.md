# Spell check: plan and contract

Status: **planned; nothing built** (written 2026-11-13 against `main` at `393ae14`, v0.1.1; revised the same night after a four-lens review against the code). The work happens on the integration branch `spell` (worktree `.claude/worktrees/spell`), with no pull request until the owner asks for one. The tag `spell-p1` marks the end of the build. The feature is meant to ship in the same release as the boards work (`discovery/boards.md` on the branch `boards`), which is built at the same time on its own branch; §6.4 says how the two meet.

How to use this document:
- It is the contract. A fresh session starts at §6.1, reads `CLAUDE.md`, then the sections its work item names. §3 is the only contract for interfaces and §4 the only contract for behaviour: work items cite their rule numbers.
- Line references are to `main` at `393ae14`. Later items move them; find the named function.
- Where the code must differ from this text, the chain records the deviation in its own log subsection (§7.5) and the orchestrator folds accepted ones into the spec text after the merge.
- The decision record, the spike sources and results, the plan review, the mockups' link, the workflow scripts and their raw results live in `.claude/spell-notes/` in the main checkout. Git ignores that folder, so nothing in it exists in a chain worktree, on the Mac or in CI. A brief names such a file by its absolute path. This repository is public: no host name, user name or private link goes into a committed file.

## 1. What we are building and why

The advanced editor engine marks misspelled words with a red zigzag underline and offers corrections in a small menu under the word, opened by a right-click or a key. Checking is offline and identical on macOS and Linux: a pure-Rust Hunspell reader (`spellbook`) over a bundled US English dictionary, plus a word list the user edits by hand. It knows Markdown: code, URLs, link destinations, front matter, HTML blocks, one-line HTML comments and the words of inline tags are never checked.

In scope: marks, the suggestions menu, replacing a word, a palette command to turn checking on and off, the `spell-check` setting, the word list file, docs and licence notices.

Out of scope, by the owner's decisions (§2.1): other languages, "Add to dictionary" and "Ignore" in the menu, grammar, autocorrect, "next misspelling", the classic engine, a native context menu or the macOS system checker. §3 keeps the seams these would use.

## 2. Decisions

### 2.1 The owner's decisions (2026-11-13)

1. **Engine:** `spellbook` on both platforms. Its licence, MPL-2.0, is acceptable: it is file-level copyleft that covers only `spellbook`'s own files, and Wren already ships an MPL-2.0 crate (`option-ext`, through `directories`). The notices must name it (§3.5).
2. **Language:** bundled `en_US` only. "With traits to allow expanded spellcheck and dictionary sources in the future."
3. **Opening the menu:** both a right-click and a key. Drawn in the app on both platforms; a native menu can come later "if something looks off".
4. **The menu holds suggestions only.** "Ignore and add can be a future work." A word list the user edits by hand is in.
5. **What is checked:** the planning session's proposal (§4.1, §4.2), with one change by the owner: mixed-case words such as `CommonMark` are flagged, as Word flags them.
6. **Underline:** the pixel zigzag ("A"), no attempt at a smooth wave now.
7. **Menu:** the plain anchored list ("option 1").
8. **Rollout:** the advanced engine only; on by default; a command in the palette turns it off and on.
9. **Grammar:** deferred. A spike of `harper-core` 3.2.0 found it capable but heavy (3 s for a full pass over 136,000 characters, an 18.9 MB test binary against 1.0 MB).
10. **Release:** shipped together with boards, to be tried there.
11. **Cost:** build sessions run on Opus and Sonnet; Fable only where it is required. This plan requires it nowhere (§6.1).

### 2.2 Choices made in writing this plan (the owner has not seen each one)

| # | Choice | Why |
|---|---|---|
| 1 | The suggest key is Alt+. on Linux and ⌘. on macOS. | Ctrl+., the usual "quick fix" key, is IBus's emoji key on GNOME desktops and never reaches the app there. Alt is Wren's own modifier on Linux, and `keys::alt_filter` already keeps an Alt press from typing. |
| 2 | A mark is hidden only while the caret sits at its end (rule 4.3.3). | "Don't mark the word being typed" with no state to keep or invalidate. |
| 3 | Opening the menu does not select the word or move the caret; the word is tinted while the menu is open (rule 4.4.2). | A selected word would be replaced by the next key typed after an Escape. |
| 4 | The word list is `words.txt` beside `config.conf`, and a palette command opens it. | The file needs a home and a way to find it. |
| 5 | Words joined by `_`, `*` or `~` inside (`snake_case`, `un*believ*able`) and all-capitals words with an `s` or `'s` ending (`PRs`, `README's`) are skipped. A word ending `'s` is correct when its base is. | Each is a common source of false marks that the owner's rules do not decide. |
| 6 | Up to five suggestions, computed when the menu opens, on the UI thread. | Measured 5 to 36 ms each (§4.7). |
| 7 | The harness runs with `spell-check = off` unless a test asks for it (§5.1). | Sample notes are full of names; existing tests and scenes keep their look. |
| 8 | The mark colour is the theme's existing `danger`. | Every built-in theme defines it; no new theme field. |
| 9 | Nothing but `settings.open_words` goes in the macOS menu bar. | A menu item would take ⌘. before the editor sees it. |

### 2.3 Open questions, each with the default the plan uses

1. Should `words.txt` be per workspace as well? Default: one global file.
2. Should the smooth wave be tried after the release? Default: no work planned; §8 names the two routes.
3. If suggestions prove slow on real notes, move them off the UI thread? Default: only if rule 4.7.3 is missed at the end review.

## 3. Architecture

### 3.1 Crates

- **`crates/wren-spell`** (new): words and dictionaries. No iced and no Wren type; depends only on `spellbook` and `unicode-segmentation`, so it can be extracted like `wren-markdown`. `[lints] workspace = true`.
- **`crates/wren-editor`**: prose ranges, marks, the menu and the replacement. It must not depend on `wren-spell` or `wren-core`. It defines the `Proofreader` trait and is handed one by the host; its tests use a fake.
- **`crates/wren-markdown`**: unchanged unless the prose ranges need a classifier fact it does not expose; `block` stays the one classifier.
- **`crates/wren-core`**: the `spell-check` setting, the key default and the `words.txt` location and header. No dependency on `wren-spell`.
- **`crates/wren-iced`**: builds the `Speller`, adapts it to `Proofreader` (a local newtype, because of the orphan rule), and owns the commands, the theme mapping, the harness requests and the Escape order.

### 3.2 Public surface

The contract commit (§6.1 step 0) lands these as declared types and stub bodies. Names may change there once; after it, a change is a deviation.

```rust
// crates/wren-spell/src/lib.rs
pub enum Error { Dictionary(String), WordList(String) }          // Display + std::error::Error
/// One place words come from: a dictionary, the user's list, later a system checker.
pub trait WordSource: Send + Sync {
    /// Whether this source knows `word` as correctly spelled.
    fn accepts(&self, word: &str) -> bool;
    /// Corrections for `word`, best first, appended to `out`.
    fn suggest(&self, word: &str, out: &mut Vec<String>);
}
pub struct Hunspell { /* spellbook over one .aff/.dic pair */ }
impl Hunspell {
    pub fn en_us() -> Result<Self, Error>;                          // the bundled dictionary
    pub fn from_text(aff: &str, dic: &str) -> Result<Self, Error>;  // any Hunspell pair
}
pub struct WordList { /* the user's words, §4.6 */ }
impl WordList {
    pub fn parse(text: &str) -> (Self, Vec<String>);                // the list and its warnings
}
impl WordSource for Hunspell { /* … */ }
impl WordSource for WordList { /* … */ }
pub struct Speller { /* sources, asked in order */ }
impl Speller {
    pub fn new(sources: Vec<Box<dyn WordSource>>) -> Self;
    /// The misspelled words of `line` that start and end inside `prose`, as byte
    /// ranges of `line`: sorted, apart, on character boundaries. Appends to `out`.
    pub fn marks(&self, line: &str, prose: &[Range<usize>], out: &mut Vec<Range<usize>>);
    /// Up to `limit` corrections for `word`, best first, without duplicates.
    pub fn suggestions(&self, word: &str, limit: usize) -> Vec<String>;
}
```

```rust
// crates/wren-editor/src/proof.rs (new), re-exported from lib.rs
/// What the host gives the editor to find and correct mistakes. Line by line,
/// so the editor asks only about rows it draws.
pub trait Proofreader: Send + Sync {
    fn marks(&self, line: &str, prose: &[Range<usize>], out: &mut Vec<Range<usize>>);
    fn suggestions(&self, line: &str, mark: Range<usize>, limit: usize) -> Vec<String>;
}
/// `OpenAt` is relative to the text origin, as a click action's position is.
pub enum SpellEvent { OpenAtCaret, OpenAt(Point), Move(i32), Hover(usize), Accept(usize), Close }
pub struct SpellMenu { /* line, mark, word, items, selected; read through accessors */ }
// line() -> usize, mark() -> Range<usize>, word() -> &str, items() -> &[String], selected() -> Option<usize>
```

Changes to existing editor types:
- `highlighter::LineDecor` (`highlighter.rs:165`) gains `prose: Vec<Range<usize>>`, the checkable byte ranges of the line (§4.1).
- `Message` (`lib.rs:106`) gains `Spell(SpellEvent)`.
- `KeyRoute` (`keys.rs:13`) gains `Suggest`: a host-bound request for corrections at the caret, carried out as `Message::Spell(SpellEvent::OpenAtCaret)`.
- `Style` (`lib.rs:56`) gains `misspelling: Color` and `menu: MenuStyle { background, border, selected, text, muted, font, text_size }` (five `Color`s, a `Font` and an `f32`; `Copy`, so `Style` stays `Copy`). The editor knows no UI font of its own. Its two constructors outside the editor are `crates/wren-iced/src/theme.rs:27` and `crates/wren-editor/src/fixtures.rs:14`.
- `EditorView` gains `set_proofreader(Option<Arc<dyn Proofreader>>)`, `spell(&mut self, SpellEvent) -> bool` (true only when the text changed, as `perform` reports, `lib.rs:292`), `spell_menu() -> Option<&SpellMenu>` and `close_spell_menu() -> bool` (true when one was open).
- Behind `test-support`: `spell_frame() -> SpellFrame`, the marks and the menu as the last frame drew them (§5.2): `SpellFrame { marks: Vec<DrawnMark>, menu: Option<Rectangle> }` and `DrawnMark { line, range, shown, rects: Vec<Rectangle> }`, all public fields, rectangles in window coordinates (the menu's word, items and selection come from `spell_menu()`); and `key_message(key: Key, modifiers: Modifiers) -> Option<Message>`, the editor's own key routing run on one press without a window.

In `wren-iced`, `App::edit` (the one place that matches `wren_editor::Message`) gets a `Spell` arm, and `CommandId` gets `editor_route() -> Option<KeyRoute>` in place of the assumption that an editor command is an `Intent` (§3.4).

Nothing here names a language, a file or Hunspell: a second dictionary, the macOS checker or a grammar engine is another `WordSource` or another `Proofreader`.

### 3.3 The seams in the editor

- **Prose ranges** are Markdown knowledge only, produced with the rest of `LineDecor`. Today the inline scanner (`scan_inline`, `highlighter.rs:479`) reports only code spans as ranges, and it never runs on heading text or done tasks. M1 therefore adds an `excluded` flag to the scanner's per-byte `Cell`, set by the code-span, comment, link, bare-URL, emphasis and strikethrough passes; gives `Inline` a `prose` list (the scanned text minus excluded bytes); and runs the scan over ATX heading text and done-task text too, for `prose` only, leaving their spans, strikes and chips exactly as they are. The highlighter's `Settings` stay `Copy` and hold no dictionary, so a changed word list never restarts highlighting.
- **Two ways to reach them.** Drawing reads the `LineDecor` of the widget's own highlighter, which lives in the widget's tree state (`widget.rs:434`). `EditorView` cannot see that state, so for one line on demand (the keyboard target, a right-click, the test accessor) it runs a fresh `MarkdownHighlighter` with the same settings over lines `0..=line` and reads that line's `prose`. That costs a pass over the text above, once per menu opening.
- **Marks are computed when a row is drawn**, from the line's text, its prose ranges and the proofreader. Nothing is cached between frames and nothing has to be invalidated: an edit, a reloaded word list or a toggled setting is right on the next frame. This is affordable because a lookup costs 0.1 to 0.2 µs (§4.7).
- **Drawing** uses the strike-through's geometry: byte ranges become per-row x extents through `glyph_span` in `visual_rows` (`widget.rs:1212`), then rectangles (`fill_quad`). Unlike the strike, which is drawn in a layer over the glyphs (`widget.rs:921`), the zigzag is drawn in the base layer after the selection rectangles and before the text, so a selection never hides it and the glyphs paint over it. No new iced feature, no path drawing, no SVG.
- **The owned engine only.** With `BackendKind::Iced` the editor computes no marks and opens no menu, whatever the host passed. The rule lives in the editor so one test proves it.
- **The menu** is drawn by the editor widget's `overlay()`, placed at layout time from the widget's bounds, its padding and the word's glyph extents, none of which `EditorView::view` knows. `view` returns the same widget tree whether the menu is open or not, so the editor's tree state (focus, highlighter checkpoints) is never rebuilt. The host supplies only colours, the UI font and the Escape order (rule 4.4.8).
- **A replacement** is an ordinary range edit through the path `EditorView::apply` uses (`lib.rs:330`), so undo, find matches, the outline and autosave see it as typing. The host learns of it because `spell` returns true, as it learns of any edit.

### 3.4 App wiring

- `Settings.spell_check: bool`, key `spell-check`, default on, parsed and written like `markdown-assist` (`settings.rs:240`, template `:423`).
- The dictionary loads off the UI thread at launch (`Task::perform`), with the first read of `words.txt`. Until it arrives there are no marks and no error.
- After launch, `words.txt` is re-read beside `config.conf` on the occasions `Preferences::reload` already has (window focus, `settings.reload`, every saved setting), synchronously, and the `WordList` is rebuilt only when its text changed. Its warnings join `Preferences::warning()`, prefixed with the file's path, so the banner and its retraction treat them like any settings warning. An unreadable file is such a warning; the dictionary keeps working.
- The editor is handed the proofreader only while the setting is on and a note is open for editing in it, and `None` otherwise, at the call sites that set `markdown_assist` today.
- `App::edit` gets the arm `wren_editor::Message::Spell(event)`: it makes the editor the active column and returns `self.editor.spell(event)`, so an accepted suggestion reaches `text_changed` and the Session draft.
- Key routing: `key_route` in `preferences.rs` returns `KeyRoute::Suggest` for `editor.spell_suggest` whatever Markdown assist is set to, and drops it while an input method composes, as it drops an `Intent`. The rule "an Editor-scope command has an intent" (`commands.rs:1151`) becomes "has an editor route". `Message::EditorShortcut` carries the route.
- Commands and keys: §4.5. The toggle saves through `save_toggle` (`preferences.rs:409`) and reports its state through `command_state` (`:421`).
- `theme.rs` maps `danger` to `Style.misspelling`, and `raised`, `dialog_border`, `selected`, `text`, `muted` and the UI font to `Style.menu`.

### 3.5 The dictionary and the licences

- The dictionary is SCOWL's Hunspell `en_US` (the standard size, not `-large`), release 2020.12.07 or the latest at build time. `en_US.aff` and `en_US.dic` live in `crates/wren-spell/dict/` for `include_str!`, with `SOURCE.md` recording the source address, the release and each file's SHA-256. About 860 KB.
- SCOWL's `README_en_US.txt` carries copyright notices that must ship. It goes to `third_party/scowl/README_en_US.txt` (the bundle copies each notice to its own relative path, and the macOS bundle test fixes the top-level names), is added to `LICENSE_NOTICES` (`tools/package.py:65`), and `NOTICES` gains a paragraph naming SCOWL `en_US`, its release and that path. `tools/wrentest_unit/test_package.py` asserts it.
- `spellbook` is pinned (`=0.4.2`) in `[workspace.dependencies]`, and the skeleton crate depends on it from step 0, so `Cargo.lock` is final before Stage A. It ships its MPL-2.0 text; its `hashbrown` and `foldhash` are already locked. Step 0 records the notices inventory before and after (the commands are in §6.1).
- `spellbook` calls itself alpha and has no phonetic suggestions. It is used unmodified. If a fix is ever needed, it goes upstream or into `vendor/` with a `WREN.md`; the MPL then requires the changed files to stay published, which a public repository satisfies.

## 4. Behaviour spec

### 4.1 What text is checked

A line's prose ranges, by the classifier's kind (`wren_markdown::block`):

| Line | Prose |
|---|---|
| `Kind::Prose`: a paragraph line, list item, block quote, task (open or done), table row, setext heading text | From the end of the container prefix and task box to the end of the line, minus the inline exclusions below. |
| ATX heading | Its text range, minus the inline exclusions. |
| A link reference definition (`[label]:` at the start of the text) | None. |
| Front matter, fenced code (fences and body), indented code, HTML block, thematic break, setext underline, blank | None. |

Inline exclusions:
1. Code spans, backticks included.
2. HTML comments that open and close on the line.
3. A link or image: everything but its text. `[the text](url "title")` and `![alt](url)` are checked in `the text` and `alt` only.
4. Bare URLs and autolinks.
5. Matched emphasis and strikethrough delimiters, so `_word_` is checked as `word`.

Rules:
1. Prose ranges are sorted, apart, non-empty and on character boundaries, for any input.
2. They depend only on the highlighter's state at the line and on the line, like the rest of `LineDecor`.
3. A lazy continuation line is prose of its paragraph.
4. A delimiter of exclusion 5 with a letter or digit directly on both sides is not excluded: it stays inside the prose range, so `snake_case_name` and `un*believ*able` each stay one chunk, which rule 4.2.1 then skips. A mark therefore never covers or abuts a delimiter inside a word.

### 4.2 What a word is

Within each prose range:
1. Split at Unicode white space into chunks. A chunk is skipped whole when it contains `@`, `/`, `\`, `<`, `>` or `[^`; when it starts with `#` followed by a letter; or when it has `_`, `*` or `~` with a letter or digit directly on both sides. That covers addresses, paths, inline tags, footnote marks, tags, identifiers and emphasis inside a word.
2. In the remaining chunks, candidate words are UAX #29 words (`unicode_word_indices`). A hyphenated compound is therefore checked part by part.
3. `’` is looked up as `'`.
4. A word is checked only if all of these hold; otherwise it is never marked:
   - every character is a letter at or below U+024F, or an apostrophe (so digits, an inner `.` or `:`, and non-Latin scripts are out);
   - it has 2 to 40 characters;
   - it is not an all-capitals word: two or more letters, every cased one upper case, optionally followed by `s` or `'s`. So `README`, `README's` and `PRs` are skipped.
5. A checked word is correct when any source accepts it as written. Sources follow Hunspell's case rules: a lower-case entry is also accepted capitalised; a mixed-case word such as `CommonMark` is looked up as written and marked if unknown (decision 5).
6. A word ending in `'s` is also correct when the part before it is.
7. A mark is the byte range of one incorrect word.

### 4.3 Marks

1. Marks exist only on the owned engine, with a proofreader set (§3.3).
2. A mark is drawn on every visual row its word occupies, across the word's glyphs on that row.
3. While the editor has focus and nothing is selected, a mark is not drawn when the caret is at its end, or is separated from its end only by apostrophes (so the mark under `wouldn` does not flash while `wouldn't` is typed). Nothing else hides a mark: not a selection, not the current-line band.
4. The zigzag is made of 1 px squares in a 4 px period: at phases 0, 1, 2, 3 the square sits 2, 1, 0, 1 px below the zigzag's top. The top is `round(baseline + 2)`, moved up if needed so all 3 px stay inside the row. On each row, with `L = ceil(left)` and `R = floor(right)` of the word's glyph extent in logical pixels, there is one square for each integer `x` with `L ≤ x < R`, at phase `(x − L) mod 4`; the phase restarts on every row, and a row with `R ≤ L` draws nothing. Size does not change on heading rows.
5. Its colour is `Style.misspelling`. It is drawn after the selection rectangles and before the text (§3.3), clipped to the rows as the bands are.
6. Marks follow the text as it is on that frame; there is no delay and no stale mark after an edit, undo, redo or replaced text.

### 4.4 The suggestions menu

1. **Target.** From the keyboard: the misspelled word that contains or touches the caret (the caret is the moving end of a selection), whether or not rule 4.3.3 is hiding its mark. From the pointer: the misspelled word under a right-click. Anything else opens nothing, and a right-click elsewhere changes neither focus nor the caret, as today. A right-click that opens the menu first gives the editor the keyboard as a left click does, and the app makes the editor its active column.
2. **Opening** computes up to five suggestions and shows them in order, the first one selected. It does not move the caret or change the selection; while the menu is open the target word is tinted with the selection colour. If the word's first row is not fully in view, opening first scrolls it into view as find does. With no suggestions the menu shows one inactive row, "No suggestions", which is never selected.
3. **Place.** Left edge at the word's left edge on the row where the word starts; top 4 px under that row. If it does not fit under the row inside the editor and fits above, it goes above, 4 px over the row; if it fits neither way it goes where there is more room. It is then moved as needed to stay 8 px inside the editor on every side. Width fits the longest row, between 160 and 320 px, and never more than the editor's width less 16 px; longer text is cut with an ellipsis. Rows are 30 px; 4 px padding, 8 px corner radius, a 1 px border.
4. **Keys while open.** Up and Down with no modifier move the selection and stop at the ends. Enter with no modifier accepts. A press of a modifier key alone does nothing. Escape is the host's (rule 8). Any other key press, modified arrows and modified Enter included, closes the menu and is then handled as if it had not been open. With only the inactive row, Up and Down do nothing and Enter closes the menu; neither reaches the text.
5. **Pointer while open.** Moving over a row selects it; a click on a row accepts it. A press elsewhere inside the editor closes the menu and does nothing more, so the caret does not move. A press outside the editor closes the menu and then does what it does today.
6. **Accepting** replaces the word with the suggestion as one undo step, puts the caret after it and closes the menu. Line endings and every other byte are untouched. If the word as written holds `’`, each `'` of the suggestion is shown and written as `’`. If the bytes at the mark are no longer the word the menu was opened for, accepting changes nothing and closes.
7. **Closing without accepting** leaves the text, the caret and the selection as they were.
8. **Closed by the host.** While the menu is open the editor gives Escape no binding: it neither drops focus nor captures the press. The app, which always receives Escape (`key_input`, `preferences.rs:501`; the `Message::Escape` arm, `app.rs:477`), calls `close_spell_menu()` before anything else Escape or a filter box's Escape does, and stops if it returns true: focus stays in the editor and an open find bar stays open. The menu also closes when the palette or a dialog opens, the note, workspace or engine changes, the text is replaced or changes by any other means, the editor scrolls (except for rule 2), loses focus or is resized, the window loses focus, the font, text size, theme or settings change, or an input method starts composing.
9. The menu never opens on the classic engine, with the setting off, before the dictionary has loaded, or when no note is open for editing (§3.4 withholds the proofreader in the last three cases).

### 4.5 Commands, keys and the setting

| Command | Palette title | Group | Scope | Default key | Enabled |
|---|---|---|---|---|---|
| `editor.toggle_spell_check` | Toggle spell check | Markdown | App | none | always; its row shows `on` or `off` |
| `editor.spell_suggest` | Suggest spelling | Markdown | Editor | Alt+. (Linux), ⌘. (macOS) | advanced engine, setting on, dictionary loaded, editor ready |
| `settings.open_words` | Open spelling word list | App | App | none | always; blocked with the message `settings.open` gives when there is no settings location |

1. The toggle flips `spell-check`, applies it at once and saves it to `config.conf`. On the classic engine it still flips the setting.
2. `editor.spell_suggest` from the palette closes the palette, gives the editor the keyboard, and only then sends `SpellEvent::OpenAtCaret`, so the menu is not closed by the focus change that opened it. With no target it does nothing.
3. `settings.open_words` creates `words.txt` if it is missing, holding only a short comment header, and opens it as `settings.open` opens `config.conf` (`preferences.rs:460`). Creating it is a small sibling of `settings::ensure_exists` in `wren-core`; the directory is the one that holds `config.conf`.
4. `spell-check = on|off` in `config.conf`; default on; the template documents it and says it needs `editor-engine = advanced`.
5. The suggest key can be rebound with a `keybind` line like any other.
6. Of the three, only `settings.open_words` is added to the macOS menu bar, after "Settings…" (`macos_menu.rs`). A menu item for the suggest command would take ⌘. before the editor.

### 4.6 The word list

1. `words.txt` in the directory that holds `config.conf`. UTF-8, one word per line; a leading byte-order mark, blank lines and lines starting with `#` are ignored; surrounding white space is trimmed; `’` is read as `'`.
2. An all-lower-case entry is accepted in lower case, capitalised and upper case. An entry with any capital is accepted exactly as written only.
3. A line that is not one word (inner white space, or longer than 40 characters) is skipped with a warning naming its line number.
4. The list is read when §3.4 says. Wren never writes it, except to create it for rule 4.5.3.
5. It offers no suggestions of its own.

### 4.7 Budgets

Measured in the planning spike (release build, `spellbook` 0.4.2, the distribution's SCOWL `en_US`): dictionary load 8 ms; lookups 0.1 to 0.2 µs per word; suggestions 5 to 36 ms per word.

1. The dictionary loads off the UI thread.
2. Marks for a full editor of text (60 rows of 100 characters) cost under 0.5 ms a frame in release.
3. Opening the menu takes under 100 ms in release at the 95th percentile over the marked words of the acceptance note (§5.5).
4. With the setting on, the time to open the acceptance note and to type 200 characters into it is within 10 % of the setting off, in release.

No test asserts a time. The spec lens of the end review measures rules 2 to 4 and records the numbers in §7.5.

## 5. Testing and local verification

### 5.1 Layers

- **`wren-spell` unit tests:** one table case per rule of §4.2 and §4.6, with the cases of the acceptance note; the bundled dictionary loads and knows a short list of words; a seeded random test over generated Unicode lines and prose ranges (marks in bounds, on character boundaries, sorted, apart, starting and ending inside prose; no panic), which also calls `suggestions` on a sample of what it marks.
- **`wren-editor` unit tests,** with a fake proofreader that rejects a fixed set of words: prose ranges per row and exclusion of §4.1, including a code span, a comment and `_x_` in a heading and in a done task; mark geometry on the owned engine, including a wrapped word, a heading row and rule 4.3.3; no call to the proofreader on the classic engine or with none set; the menu's state and every rule of §4.4 that needs no window; Escape with the menu open producing no message and keeping focus; a replacement under `\n`, `\r\n` and mixed endings, at the start and end of a line and of the text, with multi-byte neighbours and a curly apostrophe. Tests that lay out text load every bundled font face (`fixtures.rs`).
- **`wren-iced` unit tests:** the three commands and their enabled rules; the palette state; the setting's round trip; the suggest key routing to `Suggest` with Markdown assist on and off; one Escape closing only the menu while the find bar is open and a second closing the bar; an accepted suggestion reaching the Session draft; the proofreader being absent with the setting off.
- **Harness,** `tools/wrentest/test_spell.py`, on the advanced engine: the acceptance script (§5.5), one case that the classic engine shows nothing, and native X11 cases marked `native_x11` (each asserts it is not on Wayland, as the existing ones do).
- **The setting in the harness.** `WRENTEST_SPELL=off|on`; unset means `off`, because the build defaults to on. `WrenApp` (`tools/wrentest/ui.py`) applies it through a helper that appends `spell-check = <value>` to the settings of the first launch, writing the file if the test gave none, and adds nothing when the test's own settings already name `spell-check`. Any other value is an error. Three existing assertions change with it: `tools/wrentest_unit/test_ui.py` (the "no file without settings" and exact-text cases), `test_editor.py:361` (which overwrites `config.conf` mid-test and must keep the line) and `test_editor.py:630` (an exact-equality check on the file). `test_spell.py` sets the setting itself. A run with `on` exercises the default on the suites that run on the advanced engine (`WRENTEST_ENGINE`); every other suite runs on the classic engine, where spell check does nothing.
- `WrenApp(words=…)` writes `words.txt` beside the sandbox `config.conf` before the first launch.

### 5.2 Harness requests

Added, with the editor side behind `test-support`:

- **`editor.spell`** (read): `{ "enabled", "ready", "marks": [{ "line", "start", "end", "word", "shown", "rects" }], "menu": null | { "word", "items", "selected", "bounds" } }`. `enabled` is the setting and the engine together; `ready` is the dictionary loaded. `start` and `end` are byte offsets in the line. `marks` are those of the rows the last frame drew, `shown` is whether rule 4.3.3 let it be drawn, and `rects` and `bounds` are in window coordinates, so a native click can aim at them. Tests poll it with `wait_for`, since a frame must have been drawn.
- **`editor.spell_event`** `{ "event": "open_at_caret" | "move" | "hover" | "accept" | "close", "arg" }`: sends `Message::Spell` through `App::edit`, as the widget would.
- **`editor.key`** `{ "key", "mods" }`: runs the editor's own key routing on one press (`key_message`) and handles the resulting message as the widget would, so rules 4.4.4 and 4.4.8 can be driven without a window and on Wayland.
- The existing `shortcut` request accepts an editor command routed to `KeyRoute::Suggest`.

Real keys and the pointer are native X11 cases, through the `xdo` helpers the test modules already use. `tools/wrentest/test_navigation.py` lists every command and is updated in W2: the three keys join `COMMANDS`; the toggle joins the expected `state` map with the lane's value; `editor.spell_suggest` joins the commands that may be disabled; the toggle and `settings.open_words` join the set enabled with no workspace.

### 5.3 Scenes

`capture_scenes.py` gains `spell` as a value of `--only`. That mode writes `spell-check = on`, seeds `words.txt` and loads the acceptance note. Scenes: `spell-marks` (the note, with the caret at the end of a misspelled word), `spell-menu` (open under a word), `spell-menu-empty`, `spell-menu-above` (a word on the last visible row). One run captures one theme and one renderer, so the session loops, under `tools/run_display.py x11 --`:

```sh
for t in <every theme id in crates/wren-core/src/theme.rs>; do
  ICED_BACKEND=tiny-skia python3 tools/capture_scenes.py --only spell --engine advanced --label tiny-skia --theme $t
done
for t in wren-light wren-dark; do
  ICED_BACKEND=wgpu python3 tools/capture_scenes.py --only spell --engine advanced --label wgpu --theme $t
done
```

The session opens the Wren Light and Wren Dark files on both renderers and three other themes, says what each shows, and lists the rest by name.

### 5.4 Gates and operating rules

**Every commit** passes `make check`. The `clippy` target (`Makefile:99`) gains `cargo clippy -p wren-spell --all-targets --locked -- -D warnings`.

**Every chain,** before its review and again before its merge:
- `make check`.
- From W4 on, `test_spell.py` on its own: `make build-smoke`, then
  `ICED_BACKEND=tiny-skia uv run --locked --group test python tools/run_display.py x11 -- uv run --locked --group test pytest tools/wrentest/test_spell.py` and
  `ICED_BACKEND=tiny-skia uv run --locked --group test python tools/run_display.py wayland -- uv run --locked --group test pytest tools/wrentest/test_spell.py -m 'not native_x11'`.
- The whole suite, `ICED_BACKEND=tiny-skia WRENTEST_ENGINE=classic,advanced make smoke-x11`, if the chain touched `wren-editor`, `wren-core`, `wren-iced` or the harness.
- `cargo clippy --locked --target aarch64-apple-darwin -p wren-iced --all-targets --features smoke-test -- -D warnings` if the chain touched `keybind.rs`, `app/commands.rs`, `app/preferences.rs` or `macos_menu.rs`.
- The scenes, from W5 on. `spell-marks` and `spell-menu` are judged by their geometry and state tests; the `spell-wire` review and the end review look at the pictures.
- `uv run --locked --group docs zensical build --strict` if `docs/` changed.

The reviewer re-runs these and does not trust the implementer's report.

**Operating rules,** stated in every brief:
- One session per worktree at a time. Never start a smoke run in the background and keep building.
- `make check`, `make build`, `make run`, and any `cargo test` or `cargo run` without `--features smoke-test` replace `target/debug/wren` with the non-smoke binary; never run one while a smoke run is in progress in the same worktree. Safe beside a smoke run: `make clippy`, and anything with its own `CARGO_TARGET_DIR`.
- Run lanes one at a time; the 120 s per-test timeout does not survive several software renderers competing for the machine.
- `make package-debug` refuses to replace an existing bundle: give each run a fresh `PACKAGE_OUTPUT`.
- Never hand-merge `Cargo.lock`: take one side, run `cargo check` once without `--locked`, then the gates.
- This machine is shared with the boards build, whose performance gates need it quiet. Do not start a second suite beside your own, and do not use `--release` builds except where §4.7 asks for a measurement.
- Never touch the worktree `.claude/worktrees/boards` or any `boards-*` worktree or branch.
- Only the orchestrator uses the Mac. CI is not changed: the matrix does not grow.
- Never `git stash`. Without the owner: no push, and no new dependency beyond `spellbook`.

### 5.5 The acceptance script

The acceptance note is committed as `tools/wrentest/fixtures/spell.md`, exactly this text (the apostrophes in `Catppuccin’s` and `doesn’t` are curly):

````markdown
---
title: Spel chek
---
# Spell chek plan

Marks apear under prose as you type, and a
right-click on one offers replacments.

- Engine: `spelbook` with a bundeled dictionary
- See [the desing note](https://example.com/spel) or <https://example.com/chek>
- README, README's, PRs and v0.1.1 stay unmarked; CommonMark does not
- Catppuccin and Catppuccin’s are in the word list; mail me@exmaple.com
- and/or, #tagg, snake_case_nmae, un*believ*able and <kbd>Ctrll</kbd> are skipped
- [x] A done task with a mispelled word
- It doesn’t mark don't, well-known or a b c; it marks well-knwon

> A quote with a misteak.

```rust
let chek = speller.check(wrd);
```

[ref-labl]: https://example.com "Titel"
````

Its marks, with `words.txt` holding `catppuccin`, are exactly these nine words, in order: `chek` (the heading), `apear`, `replacments`, `bundeled`, `desing`, `CommonMark`, `mispelled`, `knwon`, `misteak`. Every other word of the note was checked against the distribution's SCOWL `en_US` when this was written; if the bundled release disagrees about a correctly spelled word, C2 changes the note's wording, not a rule, and says so.

On the advanced engine with the setting on:

1. Open the note. `editor.spell` reports exactly the nine marks.
2. Type a misspelled word at the end of a line: its mark is not shown; type a space: it is.
3. With the caret in a marked word, open the menu by the suggest command: it lists the expected first suggestion, and the caret and selection have not moved. Down, Up, then Escape: the text is unchanged and the editor still has the keyboard.
4. Open it again and press Enter: the word is replaced, the caret follows it, the mark is gone, and the note autosaves the new text. Undo restores the old text in one step.
5. *(native X11 only)* Right-click a marked word: the menu opens under it; click the second row: that suggestion replaces the word. With the menu open, a click on another note in the list switches notes.
6. Run `editor.toggle_spell_check`: no marks, `config.conf` says `spell-check = off`, and the suggest command does nothing. Run it again: the marks return.
7. Append a marked word to `words.txt`, run `settings.reload`: its mark is gone.
8. Switch to the classic engine: no marks and no menu.

Steps 3 and 4 use `editor.key` and `editor.spell_event`, so they run on Wayland too.

## 6. Build

### 6.1 Execution

**Step 0.** The orchestrator commits this document on `spell` before anything runs, so every chain worktree holds it. Then three setup sessions on the worktree `spell`:
1. *Preflight* (no commit): `xvfb-run`, `xdotool`, `weston` and `dbus-run-session` installed, the `aarch64-apple-darwin` rustup target installed, the Codex CLI logged in. A missing item is reported, not worked around. Confirm the worktree is on `spell` and clean.
2. *The dependency and the dictionary* (§3.5): `spellbook` pinned in the workspace; the crate skeleton `crates/wren-spell`, depending on it, as a workspace member; the dictionary files and `SOURCE.md`; the SCOWL notices and `test_package.py`; `Cargo.lock` committed. Before the change run `uv run --locked --group test python tools/dependency_notices.py --output target/notices-before.txt`; after it and `cargo fetch --locked`, run it again into `target/notices-after.txt`; record both summary lines and the packages added to `Cargo.lock` in §7.5. `make check` and `make package-debug` pass. Its own commits, revertible alone.
3. *The contract commit:* the public surface of §3.2 as declared types and stubs that compile and do nothing, including a `Message::Spell` arm in `App::edit` that returns false and values for the new `Style` fields in `crates/wren-iced/src/theme.rs` and `crates/wren-editor/src/fixtures.rs`; the `clippy` line; the `CLAUDE.md` Layout entry for `wren-spell` and the changed `wren-editor` line; the row in `discovery/README.md`. After it, no Stage A chain edits the root `Cargo.toml`, `Cargo.lock`, the `Makefile` or `CLAUDE.md`.

**Runs.** Three orchestrated runs and one small one, split where the state on disk is clean, because a workflow cannot be resumed from a new session. Each ends by writing `.claude/spell-notes/HANDOFF.md`: head, gates with pass counts, what merged, what waits on the owner.

| Run | Starts from | Does | Ends |
|---|---|---|---|
| 1a | the plan commit | Step 0; Stage A, `spell-core` and `spell-marks` in parallel; merge core, then marks | Words are judged and marks are placed, each proven by its crate's unit tests |
| 1b | the 1a head | `spell-menu`, then `spell-wire` | The feature works end to end |
| 1c | the 1b head | The end review (§7.2) | Tag `spell-p1` |
| join | `spell-p1` | §6.4 | The meeting with boards is proven, or done |

**Scripts.** Two scripts in `.claude/spell-notes/workflows/` run them, each with an arguments file from `runs/` that names the chains, their sessions, the model tier of each and the merge order. `spell-build.js` runs 1a, 1b and the join: per stage it creates the chain worktrees, runs each chain (implementing sessions in order, each handed the earlier reports; a fresh adversarial review; a fix session; a verify session; a second round if a blocker is still open), then merges the ready chains in order, each merge followed by `make check` and the X11 tiny-skia suite. A chain that cannot merge stops the run. `spell-phase-end.js` runs 1c. It does not tag: the orchestrating session runs the Mac step and tags.

**Models** (decision 11). Every session names a tier and each arguments file maps the tiers to models in one place: `"tiers": { "hard": "opus", "main": "opus", "light": "sonnet" }`.

| Tier | Model | Sessions |
|---|---|---|
| hard | Opus | U1 and U2 (the menu's interaction), the text-safety lens. Setting `hard` to `fable` moves exactly these two. |
| main | Opus | Every other implementing session, step 0, every review, fix, verify and merge, the second lens, the triage and the record. |
| light | Sonnet | Sessions that only run commands and report: the preflight, creating worktrees, the gate runs. |

**Chains.** A chain is a branch and a worktree of the same name under `.claude/worktrees/` (`spell-core`, `spell-marks`, `spell-menu`, `spell-wire`). Each is: implementing sessions in the listed order, in small commits that each pass `make check`; a fresh adversarial review; a fix session; a verify session (§7.1). Merges are `git merge --no-ff` in the stated order. A chain that merges after another first merges `spell` into itself and re-runs its gates. A merged chain's `target/` is deleted.

**Resuming.** A run that dies is resumed by hand from disk: for each chain worktree, `git log --oneline spell..HEAD` and `git status --short`; committed work stays (confirm `make check`); uncommitted work is inspected, then finished or discarded; then the chain's next step runs from its brief.

### 6.2 Work items

**`spell-core`** (crate `wren-spell`; contract §3.2, §3.5, §4.2, §4.6). Sessions: C1 and C2; C3 and C4.
- C1. `Hunspell`: the bundled dictionary through `spellbook`; `accepts` and `suggest`; errors returned.
- C2. The word rules of §4.2, all seven, and `Speller::marks`, with the table tests, the acceptance note's cases and the seeded random test.
- C3. `WordList` (§4.6) and its tests.
- C4. `Speller::suggestions`: the limit, no duplicates, sources in order.

*Try to break it:* text that is all combining marks, an emoji run, a 10,000-character line, a prose range that splits a word, RTL text, `’s` after a word-list entry, `words.txt` with a BOM or `\r\n`, an entry with a curly apostrophe; `adn/or`; `<br>`; `[^1]`; any input on which `spellbook` panics or takes over 50 ms to check one word.

**`spell-marks`** (crate `wren-editor`; contract §3.2, §3.3, §4.1, §4.3). Sessions: M1; M2 and M3.
- M1. `LineDecor.prose` for every row and exclusion of §4.1 and rule 4.1.4, by the mechanism of §3.3, with one test per row and exclusion and a seeded test of rule 4.1.1. Nothing the highlighter colours, strikes or chips today may change.
- M2. `Proofreader`, `set_proofreader`, the on-demand prose ranges of §3.3, marks per drawn row on the owned engine only, rule 4.3.3, and the marks half of `spell_frame()`.
- M3. The zigzag (rules 4.3.4 and 4.3.5) with geometry tests in the style of the strike tests (`widget.rs:1516`).

*Try to break it:* a word wrapped mid-word across rows; a mark on a heading row and on a hanging list row; the last row of the view; `un*believ*able` and `snake_case`; a link whose text is one misspelled word; a code span in a heading; a done task; a lazy line in a quote; a reference definition; marks after undo; a selection over a mark; any existing highlighter test or scene that changes; any frame on which the classic engine asks the proofreader anything.

**`spell-menu`** (crate `wren-editor`; contract §3.2, §3.3, §4.4). Sessions: U1 and U2 *(hard)*; U3.
- U1. Targets and opening (rules 1 and 2), `SpellEvent`, `SpellMenu`, `EditorView::spell`, `KeyRoute::Suggest`, the right-click hit test and the focus it takes.
- U2. The overlay, its place (rule 3), pointer and key handling (rules 4 and 5), Escape left to the host (rule 8) and every closing cause the editor can see, `key_message`, and the menu half of `spell_frame()`.
- U3. Accepting (rules 6 and 7) through the range-edit path, with the replacement tests of §5.1.

*Try to break it:* a press that both closes the menu and moves the caret; a key that closes the menu and is then lost, or handled twice; a bare Shift press; Enter with "No suggestions"; `replace_text` or undo with the menu open, then accept; a word at the right edge, on the last row and in an editor 150 px wide; opening with the caret's line scrolled out of view; opening twice; a right-click on an unfocused editor and during a drag selection; a replacement in a `\r\n` note and of `dosen’t`; focus left nowhere after closing; the tree state rebuilt when the menu opens.

**`spell-wire`** (`wren-core`, `wren-iced`, `tools/`, `docs/`; contract §3.4, §4.5, §4.6, §5). Sessions: W1 to W3; W4; W5; W6.
- W1. The setting; loading the dictionary off the UI thread; `words.txt` as §3.4 says, with its warnings; the proofreader handed to the editor; the `App::edit` arm; rule 4.4.9; the `wren-iced` tests of §5.1 for W1 to W3.
- W2. The three commands, the route and key default (§3.4), the palette state, rule 4.5.2's order, `settings.open_words` and its menu-bar entry, and `test_navigation.py` (§5.2).
- W3. The theme mapping; the Escape order and the app-side closing causes of rule 4.4.8.
- W4. The harness: `WRENTEST_SPELL` and the three changed assertions, `WrenApp(words=…)`, the requests of §5.2, the acceptance note, `test_spell.py`.
- W5. The scenes (§5.3).
- W6. Docs: a new page `docs/reference/spell-check.md` (the advanced-engine requirement, what is and is not checked, the menu and its keys, `words.txt` with its location and rules, the three commands) with a nav entry under Reference after Configuration in `zensical.toml`; the `spell-check` row in `configuration.md`; the key in `keybindings.md`; the crate and seams in `architecture.md`; `WRENTEST_SPELL` and the new requests in `testing.md`.

*Try to break it:* the suggest key acting twice, outside the editor or with Markdown assist off; the toggle with nowhere to save; `words.txt` unreadable or a directory, and its warning surviving the next saved toggle; the open note rewritten on disk and reconciled while the menu is open, then Enter; an engine switch or note switch with the menu open; Escape with the find bar open too; the menu opened from the palette closing at once; the setting off but a mark drawn for one frame; a slow dictionary load racing the first note; any mark or menu with `WRENTEST_SPELL=off`.

### 6.3 Exit criteria

1. The acceptance script passes on tiny-skia and wgpu: steps 1 to 4 and 6 to 8 on X11 and Wayland, step 5 on X11.
2. `make check`, the four lanes with `WRENTEST_SPELL=off` and again with `on`, and the strict docs build pass.
3. `wren-editor` depends on neither `wren-spell` nor `wren-core`; `wren-spell` depends on no iced or Wren crate.
4. The notices bundle builds and holds SCOWL's notices and a licence text for every crate added to `Cargo.lock`.
5. The budgets of §4.7 are measured and recorded, and met or explained.
6. The scenes exist for every theme and the named ones were looked at.
7. No open blocker; every open major has an owner decision recorded.

### 6.4 Meeting the boards branch

Both branches were cut from `main` at `393ae14`. `boards` does not touch `crates/wren-editor` or `crates/wren-markdown`. Files both change: the root `Cargo.toml` and `Cargo.lock`, the `Makefile`, `CLAUDE.md`, `discovery/README.md`, `crates/wren-core/src/keybind.rs` and `settings.rs`, `crates/wren-iced/src/app.rs`, `app/commands.rs`, `app/preferences.rs`, `app/palette.rs`, `app/test_requests.rs`, `theme.rs` and `macos_menu.rs`, `tools/package.py`, `tools/capture_scenes.py`, `tools/wrentest/ui.py`, `test_navigation.py` and `test_editor.py`, `tools/wrentest_unit/test_ui.py` and `test_package.py`, and the docs.

- Whichever branch reaches `main` second merges `main` into itself first. The join run does that for `spell` once boards is on `main`. Before then, it proves the meeting without changing `spell`: on a throwaway branch `spell-join`, cut from `spell`, it merges the current head of `boards`, resolves, runs the gates below, records every resolution in §7.5, and deletes nothing of boards'.
- Resolving: keep both sides' rows in every table and list, apply the `Cargo.lock` rule, then run `make check`, the eight lane runs of §7.2 (boards' own lane variables at their defaults), the scenes and the strict docs build.
- Three things to settle in the join:
  1. *Text only.* Boards gives the editor three surfaces. Spell check is for Markdown text: the proofreader is `None` unless the surface is `Surface::Text`, set at the call sites that gate `markdown_assist`; `editor.spell_suggest` is enabled only there; the menu closes when the surface changes. A board that fails to parse and opens as raw text reports no marks, with a harness case.
  2. *The harness setting.* Boards has its own helper for sandbox settings (`_boards_setting`); `WRENTEST_SPELL` goes through the same shape, and boards' edit of `test_editor.py:630` replaces ours.
  3. *Escape.* Boards rewrites Escape handling in `preferences.rs` (a ladder for the board); `close_spell_menu()` stays the first rung.
- After the join the harness can set a fractional display scale (boards adds the variable): capture `spell-marks` at 125 % and record what the zigzag looks like (§8).
- Nothing reaches `main` without the owner's word.

## 7. Reviews

### 7.1 The per-chain adversarial review

Every chain gets one, by a fresh session with no memory of the implementation.

**The reviewer is given** the worktree, the chain's contract sections, the implementers' structured reports, the diff command `git diff spell...HEAD`, and the chain's "try to break it" list.

**The reviewer is told:** do not fix and do not commit; throwaway tests are fine, but delete them and leave `git status` clean; run the chain gates of §5.4 yourself; open every scene and list each file with what it shows; report only problems with evidence; list what you checked that held, so silence can be told from not looking.

**Findings schema:** `{ gates_pass, gates, issues: [{ severity, where, problem, evidence, fix }], verified_ok: [string] }`. Severities:
- **blocker:** a replacement that changes any byte but the word, or breaks undo; a panic reachable from text or a file; a write to a note or to `words.txt` that no rule allows; a mark or a menu on the classic engine or with the setting off; an existing test whose outcome changes with `WRENTEST_SPELL=off` beyond the changes §5 names; a failing gate; a licence violation.
- **major:** specified behaviour wrong or missing; an untested path that edits text; a missed budget; docs that contradict behaviour.
- **minor:** everything else.

**The fix step** is a third session given the review. It reproduces each finding first, fixes every blocker and major, fixes minors that are clearly right and cheap, explains with evidence any finding it rejects, and adds one regression test per real bug, in small commits that each pass `make check`. It returns `{ summary, commits, gates, not_fixed }`.

**The verify step** re-runs the gates and re-tests each blocker and major from its evidence. The chain merges only when the gates pass and no blocker is open. Two failed rounds stop the run for that chain and return it to the owner. A major left unfixed needs its reason in §7.5 and is carried into the end review.

### 7.2 The end review

Run 1c, on `spell`, in this order:

1. `make check`, then the four lanes with the setting off, one at a time.
2. Two lenses and the Codex pass, in parallel, each read-only in its own detached worktree at the head (`git worktree add --detach .claude/worktrees/spell-lens-<key> spell`), each returning the schema of §7.1 plus an `assessment` string.
3. Triage, then one fix session, then a verify session.
4. The final gates, once: `make check`; the eight lane runs below; the strict docs build; the scenes of §5.3.
5. The record in §7.5 and the handoff note. The orchestrator then does the Mac step (`make build fmt-check clippy test check-patches`, `make build-smoke`, `capture_scenes.py --only spell --engine advanced`, and a look at ⌘. and the menu on Metal) and tags.

```sh
for s in off on; do for r in tiny-skia wgpu; do
  export WRENTEST_SPELL=$s ICED_BACKEND=$r WRENTEST_ENGINE=classic,advanced WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1
  make smoke-x11 && make smoke-wayland || break 2
  rm -rf target/wrentest-$s-$r && cp -r target/wrentest target/wrentest-$s-$r
done; done
```

| Lens | What it does |
|---|---|
| Text safety | Hunts for any way spell check changes, loses or wrongly writes text, and for panics. Drives the real app over the test socket for a few thousand seeded steps on tiny-skia: generated notes (mixed line endings, a BOM, multi-byte and RTL text), then random caret moves, typing, opening the menu, accepting a random row, Escape, undo, redo, note switches, toggling the setting, `settings.reload`, and an external rewrite of the open note followed by its reconcile. After every step: the editor's text differs from the model only by the accepted replacements; an accept changes nothing outside its word; after a flush the file equals the editor's text; undoing to the start gives the original bytes; the app has not exited. Every failure reports its seed and step. Greps the new code for `unwrap`, `expect` and indexing and tries to trigger each. |
| Spec and ship | Without reading the implementation first, checks every rule and table row of §4 in the real app, with real X11 input for the pointer rules. Then: the crate rules of §3.1; every package added to `Cargo.lock` with its licence; the notices; docs against behaviour; rules 4.7.2 to 4.7.4 measured, with numbers; and the exit criteria of §6.3, each true or listed with its reason. |

### 7.3 The Codex pass

Beside the lenses, through the installed Codex CLI, in its own detached worktree, read-only by instruction. It is given this document with its exit criteria, the diff since `main` and the findings schema to answer in, not the in-house findings, and a narrow brief:
1. find a line and prose ranges for which a mark is out of bounds, splits a character, overlaps another or lies outside prose;
2. find a sequence of user actions after which accepting a suggestion changes bytes outside the word, or undo does not restore the text;
3. find a path by which spell-check code runs on the classic engine or with the setting off, or blocks the UI thread on loading the dictionary.

**Triage.** A Claude session tries to reproduce each finding as a failing test or a harness transcript. Reproduced findings enter the fix queue with that reproduction as evidence and their severity set again by §7.1. The rest are logged in one line each and dropped. Duplicates of in-house findings are merged, noting who found them.

### 7.4 Recording results

- Raw outputs are saved as JSON under `.claude/spell-notes/workflow-results/`. They are not committed.
- §7.5 holds one subsection per chain, so parallel chains never edit the same lines. A chain writes there, in its own worktree: its deviations, then three lines for its review (counts by severity, the fix commits, what was not fixed and why).
- The end review adds a Fixed list (severity, what, the regression test, who found it), a "Not fixed, by choice" list with reasons, the gates with pass counts per lane, the measured budgets, and what waits for the owner.
- `CHANGELOG.md` is written by the release command, not here.
- When the feature has shipped, what users need is in `docs/`, and this note is trimmed to its decisions and its log, as `CLAUDE.md` asks.

### 7.5 Log

#### Step 0

*Item 2, the dependency and the dictionary.*

- Dictionary: fetched from SCOWL's published release, not a distribution copy. SCOWL now publishes on GitHub (`en-wl/wordlist`); the latest release at build time is 2026.02.25 (`hunspell-en_US-2026.02.25.zip`), and it is the one bundled. `crates/wren-spell/dict/SOURCE.md` has the addresses and hashes; `test_package.py` checks the hashes and that `README_en_US.txt`, `SOURCE.md` and `NOTICES` name the same release.
- Notices inventory, with `target/` created first (the tool will not create its output's directory):
  - before: `Cargo dependency notices: 485 packages, 768 local texts, 65 packages requiring review; 5 workspace members excluded.`
  - after: `Cargo dependency notices: 486 packages, 769 local texts, 65 packages requiring review; 6 workspace members excluded.`
  - The one package added to `Cargo.lock` is `spellbook 0.4.2` (MPL-2.0, its `LICENSE` collected, not in the review list). Its `foldhash 0.1.5` and `hashbrown 0.17.1` were already locked. The lock also gains the `wren-spell` member, depending on `spellbook` and `unicode-segmentation`.

Deviations:
1. *Release 2026.02.25, not 2020.12.07* (§3.5 allows the latest). Its `.dic` is 508,614 bytes, not "about 860 KB": that figure was the distribution's build, which is larger than SCOWL's own standard archive even for 2020.12.07 (551,762 bytes). The new `.aff` adds `ICONV ’ '` and `’` to `WORDCHARS`, so the dictionary itself accepts `can’t`. With `spellbook 0.4.2`, every word of the acceptance note (§5.5) gets the same verdict from the bundled files as from the distribution's 2020.12.07 copy: the nine marked words and the tokens §4.2 skips are rejected, everything else is accepted.
2. *`NOTICES` also names `spellbook`*, its version, the MPL-2.0 and where its source is, as decision 2.1.1 asks and MPL-2.0 §3.2(a) requires for an executable. `tools/package.py` keeps the version in `SPELLBOOK_VERSION` and the release in `SCOWL_RELEASE`; the unit test ties the first to the `Cargo.toml` pin and `Cargo.lock`.
3. *`wren-spell` is in `[workspace.dependencies]` already*, so `spell-wire` adds it to `wren-iced` without touching the root `Cargo.toml` (it still changes `Cargo.lock`, in Stage B).
4. Until W1 links `wren-spell` into `wren-iced`, `NOTICES` says the executable embeds a dictionary it does not yet hold. Harmless on an unreleased branch.

*Item 3, the contract commit.* §3.2 compiled as written; what it left open was decided as below and written back into §3.2.

1. *`Error` carries `String` reasons* (`Dictionary`, `WordList`), so `spellbook`'s types stay out of the public surface; it implements `Display` and `std::error::Error`.
2. *The `wren-spell` types are declared without fields* (`#[non_exhaustive]`, so no other crate can build one): their representation is `spell-core`'s to choose. The stubs build no dictionary; `en_us` and `from_text` return `Ok`, sources accept nothing, and `Speller` finds and suggests nothing.
3. *`spell_frame()` returns `SpellFrame`*, a test-support type §3.2 did not name; see the bullet there. `DrawnMark.range` is the word's byte range, from which the host reports `start`, `end` and `word` (§5.2).
4. *`key_message` takes no physical key*, as §3.2 writes it. The editor's routing reads `Key::to_latin(physical)`; `spell-menu` can pass an unidentified physical key, which falls back to the key's own character.
5. *`KeyRoute::Suggest` is already routed* in `keys::key_binding` to `Message::Spell(SpellEvent::OpenAtCaret)`, the variant's documented meaning, so the match stays exhaustive. Nothing returns the route yet, and the host drops it while an input method composes (§3.4).
6. *`App::edit`'s `Spell` arm returns `false`* without calling `EditorView::spell`, as step 0 says; W1 gives it its body.
7. *`CommandId::editor_route()` is not in the contract commit*: step 0 touches `wren-iced` only where the build needs it, and W2 adds the method with the route.
8. *The theme mapping already holds §3.4's values*: `danger` for the zigzag; `raised`, `dialog_border`, `selected`, `text` and `muted` for the menu; the UI font (`fonts::UI`) at 13 px, the palette's row size, since the plan names no size. W3 owns any change.

#### `spell-core`

*C1 and C2.* Where §4.2 or §3.2 left a case open, this is what `wren-spell` does:

1. *The empty word is not accepted.* `spellbook` counts `""` as correct; `Hunspell::accepts` does not, so no source vouches for nothing.
2. *A run joins, not only a single character* (rule 4.2.1): a chunk is skipped when a run of `_`, `*` or `~` has a letter or digit directly on both sides, so `strong**wrod**ish` is skipped like `un*believ*able`. Read character by character, a doubled delimiter would never count.
3. *A word cut by a prose range is not checked.* A candidate word that starts where its prose range starts with a letter or digit just before it in the line, or ends where the range ends with one just after it, is a piece of a longer word of the line, so it is skipped rather than marked (`marks` promises "words of `line`"). The editor's ranges never cut a word (rule 4.1.4), so this only guards against a bad caller.
4. *Bad prose ranges are tolerated, not trusted.* Each is clamped to the line and shrunk to character boundaries; a range that starts before the end of an earlier one keeps only its part after it, so one out of order is dropped. The marks stay sorted, apart (a gap between any two) and inside the ranges given for any input; the seeded test passes ranges that are unsorted, overlapping, reversed, past the end and inside characters.
5. *`_` belongs to a UAX #29 word* (it is `ExtendNumLet`), so with the delimiter in prose `_word_` or `trailing_` is one candidate that rule 4.2.4 never checks. Matched emphasis delimiters are not prose (exclusion 5), so the editor's `_word_` is still checked as `word`.
6. *"Every cased one upper case"* is read as "upper-cases to itself": a title-case letter such as `ǅ` is not upper case. A word written with a combining mark (`cafe` plus U+0301) is never checked, since the mark is not a letter at or below U+024F.
7. *The acceptance note needs no change*: with the bundled 2026.02.25 release its marks are exactly the nine words, in order (`speller::tests::the_acceptance_note_has_exactly_its_nine_marks`, with the editor's prose ranges given by hand).
8. `Speller::suggestions` is still the stub (C4). The seeded test already calls it on a sample of its marks.

Measured while trying to break it, in a debug test build (`spellbook` at the workspace's `opt-level = 2` for dependencies): the slowest of 200,000 random 2 to 40 character checks took 39 µs, far under the 50 ms the brief names; no input panicked.

#### `spell-marks`

#### `spell-menu`

#### `spell-wire`

#### The end review

#### The join

## 8. After this, and risks

- **Add and Ignore** in the menu: rows under a divider; Add appends to `words.txt` (the first write Wren makes to it), Ignore is per note and needs a place to live.
- **More languages and sources:** another `WordSource` (a second Hunspell pair, system dictionaries on Linux, `NSSpellChecker` on macOS behind the one crate allowed `unsafe`), and a setting to choose.
- **Grammar:** `harper-core` as a `Proofreader` of its own, off the UI thread and paragraph by paragraph, with a second mark colour and a message row in the menu.
- **A smooth wave:** either SVG tiles sized exactly to each word (the software renderer ignores an image's own clip rectangle) or iced's `canvas`, which `boards` already turns on for the app but which needs app-wide antialiasing to look smooth on wgpu.
- **Known limits of the word rules.** A typo beside a slash (`adn/or`) or joined to the next word by a missing space after a full stop is not marked; a comment that spans lines is checked on its later lines; label words of a footnote definition are checked.
- **Risk: fractional display scales.** The zigzag is made of logical pixels; at 125 % or 150 % its steps may snap unevenly. Checked at the join (§6.4).
- **Risk: `spellbook` is alpha.** The seeded tests and the text-safety lens are the guard; a panic found in it is a blocker to work around in `wren-spell`, never to hide.
- **Risk: suggestions on the UI thread.** Bounded by rule 4.7.3 and open question 3.
