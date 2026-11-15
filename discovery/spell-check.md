# Spell check: plan and contract

Status: **built, end-reviewed and finished; not tagged** (2026-11-14). The feature is complete on `spell` at `d1cab2a` plus this record. Run 1c's end review stopped short of the tag; the extra chains `spell-endfix` and `spell-endfix2` fixed what it left open, and at `d1cab2a` every final gate passes and every exit criterion of §6.3 holds. `spell-p1` waits only for the Mac step of §7.2 step 5; the push, CI, a hand trial and the join with boards wait for the owner. §7.5 "The end review" has the record, and its part "Finish run, 2026-11-14" the gates at this head and the list of what waits. Planned 2026-11-13 against `main` at `393ae14`, v0.1.1, and revised the same night after a four-lens review against the code; the deviations found in the build are folded into §3 and §4. The work happens on the integration branch `spell` (worktree `.claude/worktrees/spell`), with no pull request until the owner asks for one. The tag `spell-p1` marks the end of the build. The feature is meant to ship in the same release as the boards work (`discovery/boards.md` on the branch `boards`), which is built at the same time on its own branch; §6.4 says how the two meet.

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
/// `Accept(row)` is a click on a row (and the harness's); `AcceptSelected` is
/// Enter, accepting whatever row the host's menu has selected (rule 4.4.4).
/// `Key` is Up, Down or Enter pressed after an opening earlier in the same
/// batch (§3.3): an open menu takes it as `MenuKey::event` says, and with
/// none open the editor handles the press as it would with no menu.
/// `Press` is a left or right press inside the editor after such an opening,
/// or a left press after such a press in the same batch: an open menu only
/// closes (rule 4.4.5), and with none open the editor clicks, or opens the
/// menu at the point, as that press would. A left press carries the click
/// kind it makes if each `Press` before it in the batch clicked; the editor
/// counts the press after one that only closed as a single click.
pub enum SpellEvent { OpenAtCaret, OpenAt(Point), Move(i32), Hover(usize), Accept(usize), AcceptSelected, Key(MenuKey), Press(MenuPress), Close }
pub enum MenuKey { Up, Down, Enter }   // event() -> SpellEvent: Move(-1), Move(1), AcceptSelected
pub enum MenuPress { Left(Point, mouse::click::Kind), Right(Point) }   // relative to the text origin
pub struct SpellMenu { /* line, mark, word, items, selected; read through accessors */ }
// line() -> usize, mark() -> Range<usize>, word() -> &str, items() -> &[String], selected() -> Option<usize>
```

Changes to existing editor types:
- `highlighter::LineDecor` (`highlighter.rs:165`) gains `prose: Vec<Range<usize>>`, the checkable byte ranges of the line (§4.1).
- `Message` (`lib.rs:106`) gains `Spell(SpellEvent)`.
- `KeyRoute` (`keys.rs:13`) gains `Suggest`: a host-bound request for corrections at the caret, carried out as `Message::Spell(SpellEvent::OpenAtCaret)`. The editor drops it while an input method composes, as the host does (§3.4).
- `Style` (`lib.rs:56`) gains `misspelling: Color` and `menu: MenuStyle { background, border, selected, text, muted, font, text_size }` (five `Color`s, a `Font` and an `f32`; `Copy`, so `Style` stays `Copy`). The editor knows no UI font of its own. Its two constructors outside the editor are `crates/wren-iced/src/theme.rs:27` and `crates/wren-editor/src/fixtures.rs:14`.
- `EditorView` gains `set_proofreader(Option<Arc<dyn Proofreader>>)`, `spell(&mut self, SpellEvent) -> bool` (true only when the text changed, as `perform` reports, `lib.rs:292`), `spell_menu() -> Option<&SpellMenu>` and `close_spell_menu() -> bool` (true when one was open).
- Behind `test-support` (and the crate's own tests): `spell_frame() -> SpellFrame`, the marks and the menu as the last frame drew them (§5.2): `SpellFrame { marks: Vec<DrawnMark>, menu: Option<Rectangle> }` and `DrawnMark { line, range, shown, rects: Vec<Rectangle> }`, all public fields, rectangles in window coordinates (the menu's word, items and selection come from `spell_menu()`). A mark rule 4.3.3 hides is still listed, with the rectangles it would draw, and `shown` false; `line_text(line) -> Option<String>`, one line as the editor splits its text, so the host can name a mark's word without knowing the line endings; and `key_message(key: Key, modifiers: Modifiers) -> Vec<Message>`, the editor's own key routing run on one press without a window, returning every message the press publishes, in order (a key that closes the menu gives `Spell(Close)` and then its own; §7.5 `spell-menu` item 1).

In `wren-iced`, `App::edit` (the one place that matches `wren_editor::Message`) gets a `Spell` arm, and `CommandId` gets `editor_route() -> Option<KeyRoute>` in place of the assumption that an editor command is an `Intent` (§3.4).

Nothing here names a language, a file or Hunspell: a second dictionary, the macOS checker or a grammar engine is another `WordSource` or another `Proofreader`.

### 3.3 The seams in the editor

- **Prose ranges** are Markdown knowledge only, produced with the rest of `LineDecor`. Today the inline scanner (`scan_inline`, `highlighter.rs:479`) reports only code spans as ranges, and it never runs on heading text or done tasks. M1 therefore adds an `excluded` flag to the scanner's per-byte `Cell`, set by the code-span, comment, link, bare-URL, emphasis and strikethrough passes; gives `Inline` a `prose` list (the scanned text minus excluded bytes); and runs the scan over ATX heading text and done-task text too, for `prose` only, leaving their spans, strikes and chips exactly as they are. The highlighter's `Settings` stay `Copy` and hold no dictionary, so a changed word list never restarts highlighting.
- **Two ways to reach them.** Drawing reads the `LineDecor` of the widget's own highlighter, which lives in the widget's tree state (`widget.rs:434`). `EditorView` cannot see that state, so for one line on demand (the keyboard target, a right-click, the test accessor) it runs a fresh `MarkdownHighlighter` with the same settings over lines `0..=line` and reads that line's `prose`. That costs a pass over the text above, once per menu opening.
- **Marks are computed when a row is drawn**, from the line's text, its prose ranges and the proofreader, once per drawn line (not per wrapped row). Nothing is cached between frames and nothing has to be invalidated: an edit, a reloaded word list or a toggled setting is right on the next frame. This is affordable because a lookup costs 0.1 to 0.2 µs (§4.7). A line with no prose is not asked about, and a returned range that is empty or not on character boundaries of the line is dropped, so later code may slice the line with any mark.
- **Drawing** uses the strike-through's geometry: byte ranges become per-row x extents through `glyph_span` in `visual_rows` (`widget.rs:1212`), then the squares of rule 4.3.4. Each word's squares on a row are one small SVG picture: one path of its squares, on whole window pixels and already clipped to the rows, with the colour in the SVG. Pictures are shared by what they draw (the zigzag's width, the part the rows' clip leaves and the colour), so words of one width share one, and the widget keeps each while frames draw it, so an unchanged frame repeats the same primitives and damages nothing and a note switch reuses the pictures of widths already drawn. tiny-skia repaints every layer once per damage group and blits a picture's whole extent wherever it repaints: one 1 px quad per square (the first design) stalled a large note for seconds (§7.5 `spell-wire` items 22 and 29), and one picture of a whole frame's squares (the second) cost a blit of every marked row per group on a note switch and a group of its own (§7.5 `spell-endfix` item 6). Wren's vendored `iced_tiny_skia` merges damage groups in any order, so the words' small pictures fold into the repaint of the text (`vendor/iced_tiny_skia/WREN.md`). Unlike the strike, which is drawn in a layer over the glyphs (`widget.rs:921`), the pictures are drawn in the base layer after the selection rectangles and the menu word's tint and before the text, so neither hides it and the glyphs paint over it. `wren-editor` turns on iced's `svg` feature, which the app already used; there is no path drawing and no new dependency.
- **The owned engine only.** With `BackendKind::Iced` the editor computes no marks and opens no menu, whatever the host passed. The rule lives in the editor so one test proves it.
- **The menu** is drawn by the editor widget's `overlay()`, placed at layout time from the widget's bounds, its padding and the word's glyph extents, none of which `EditorView::view` knows. `view` returns the same widget tree whether the menu is open or not, so the editor's tree state (focus, highlighter checkpoints) is never rebuilt. The host supplies only colours, the UI font and the Escape order (rule 4.4.8).
- **The menu's state lives in `EditorView`**, which hands it to the widget each frame. iced hands every event queued since the last frame to the widget, the overlay first, before the host applies any message, so within one batch the widget routes against its own copy (`proof::OpenMenu`), which each message it publishes changes as the host will change its menu: a move or a hover moves the selection; any other message, and Escape, closes it. A menu open when the batch starts is the host's, so Up, Down and Enter publish `Move` and `AcceptSelected`, and Enter never needs the number of rows. An opening in the batch (the suggest key or a right-click) is resolved by the host against the caret and text as the batch's earlier messages leave them, which the widget does not have, so the widget cannot know whether a menu opened or what it offers. Until the next batch it takes the opening as a menu for every key but three: Escape is the host's, a lone modifier does nothing, and any other press closes it first; Up, Down and Enter with no modifier publish `SpellEvent::Key`, which the host resolves against its real menu (a move or `AcceptSelected` when one is open, otherwise the press as the editor handles it with no menu), unless the key policy gives the key to the host, which then closes the menu and gets the press uncaptured. Enter leaves no menu either way. A left or right press inside the editor publishes `SpellEvent::Press` in the same way, which the host resolves as a close when a menu opened (wherever the press lands: the new menu has no layout until the next frame, so a press where its row will be drawn only closes it too) and otherwise as the click or the right-click it is with no menu; a left press the host does not take as a single click (one that only closed the menu, or a double or triple click) also drops the drags the widget publishes after it. Whether a left press is a double or triple click depends on what the press before it was, so once the widget has left a left press to the host, every later left press of the batch is a `Press` too, carrying the kind it makes if the earlier ones clicked, and `EditorView` counts it again from its own verdicts when one of them only closed a menu. The widget takes the host's verdict on the last such press (`proof::ResolvedPress`, handed to it with the menu) at the next batch, so a press that only closed a menu, there or over a menu open when the batch started, is never the first half of a double or triple click. A menu open when the batch starts has an overlay, which iced hands the whole batch before the widget: it takes moves over the menu and presses on it, so those come before the batch's keys, and leaves every other press to the widget, which closes the menu for it in its order with the keys; with no menu at the start there is no overlay, and presses and keys reach the widget in their order.
- **A replacement** is an ordinary range edit through the path `EditorView::apply` uses (`lib.rs:330`), so undo, find matches, the outline and autosave see it as typing. The host learns of it because `spell` returns true, as it learns of any edit.

### 3.4 App wiring

- `Settings.spell_check: bool`, key `spell-check`, default on, parsed and written like `markdown-assist` (`settings.rs:240`, template `:423`).
- The app side lives in `crates/wren-iced/src/app/spelling.rs`: `Spelling` holds the dictionary (an `Arc<Hunspell>`), the last read of `words.txt` and the proofreader built from the two (a `Speller` over both, behind a local `Proofreader` newtype).
- The dictionary loads at launch, whatever the setting and the engine, on a thread of its own (a `Task::perform` future alone would run on the UI thread until it first waits), with the first read of `words.txt`. Until it arrives there are no marks and no error. A read of `words.txt` made before the load arrives (a focus, say) is newer and is kept. A dictionary that fails to load (it is bundled, so only a broken build) shows `Spell check is unavailable: …` in the banner, and there is no proofreader.
- After launch, `words.txt` is re-read on the occasions `Preferences::reload` already has (window focus, `settings.reload`, every saved setting), synchronously and before `config.conf`, and the `WordList` is rebuilt only when its text changed. Both files are read through `settings::read_text`, which opens without blocking and refuses anything but a regular file (a symbolic link to one still reads), invalid UTF-8 and more than 1 MiB, enforced while reading, so a FIFO cannot hang the UI thread. Its warnings join `Preferences::warning()` after `config.conf`'s, joined with `; `, each prefixed with its file's path, so the banner and its retraction treat them like any settings warning. An unreadable file gives an empty list and the warning `could not read the file: …`; a missing one is an empty list and no warning. The dictionary keeps working either way.
- The editor is handed the proofreader only while the setting is on and a note is open for editing in it, and `None` otherwise. The hand-over is synced after every message (`App::update`), after `apply_preferences` and after a reload, since "a note is open for editing" changes on many paths; handing over the same `Arc` keeps an open menu. The engine is not part of this rule: the editor ignores a proofreader on the classic engine (§3.3).
- `App::edit` gets the arm `wren_editor::Message::Spell(event)`: it returns `self.editor.spell(event)`, so an accepted suggestion reaches `text_changed` and the Session draft. It makes the editor the active column only for `OpenAtCaret`, `OpenAt`, `Accept`, `AcceptSelected`, `Key` and `Press`; `Close` also comes from a press outside the editor that may choose another column. `OpenAt` also returns the focus operation for `note-editor`, because iced's `text_input` gives up focus on a left press, not a right one, so a focused find field or filter box would otherwise keep the keyboard.
- Key routing: `key_route` in `preferences.rs` returns `KeyRoute::Suggest` for `editor.spell_suggest` whatever Markdown assist is set to, and drops it while an input method composes, as it drops an `Intent`. The rule "an Editor-scope command has an intent" (`commands.rs:1151`) becomes "has an editor route": `CommandId::editor_route()` replaces `intent()` in public use, routing the Markdown commands to `KeyRoute::Intent` and `editor.spell_suggest` to `KeyRoute::Suggest`, and the rule test is `editor_scoped_commands_are_exactly_those_with_an_editor_route`. `Message::EditorShortcut` carries the route.
- Commands and keys: §4.5. The toggle saves through `save_toggle` (`preferences.rs:409`) and reports its state through `command_state` (`:421`).
- `theme.rs` maps `danger` to `Style.misspelling`, and `raised`, `dialog_border`, `selected`, `text`, `muted` and the UI font (`fonts::UI`) at 13 px, the palette's row size, to `Style.menu`. A test checks every built-in theme maps so, with the zigzag, the menu's rows and its border distinct from what they sit on.

### 3.5 The dictionary and the licences

- The dictionary is SCOWL's Hunspell `en_US` (the standard size, not `-large`), release 2026.02.25, the latest at build time, from SCOWL's own published release (`hunspell-en_US-2026.02.25.zip` on GitHub, `en-wl/wordlist`), not a distribution copy. `en_US.aff` and `en_US.dic` live in `crates/wren-spell/dict/` for `include_str!`, with `SOURCE.md` recording the source address, the release and each file's SHA-256. The `.dic` is 508,614 bytes. This release's `.aff` adds `ICONV ’ '` and puts `’` in `WORDCHARS`, so the dictionary itself accepts `can’t`. Every word of the acceptance note (§5.5) gets the same verdict from it as from the distribution's 2020.12.07 copy.
- SCOWL's `README_en_US.txt` carries copyright notices that must ship. It goes to `third_party/scowl/README_en_US.txt` (the bundle copies each notice to its own relative path, and the macOS bundle test fixes the top-level names), is added to `LICENSE_NOTICES` (`tools/package.py:65`), and `NOTICES` gains a paragraph naming SCOWL `en_US`, its release and that path, and `spellbook`, its version, the MPL-2.0 and where its source is (decision 2.1.1; MPL-2.0 §3.2(a) asks it of an executable). `tools/package.py` keeps the two in `SCOWL_RELEASE` and `SPELLBOOK_VERSION`. `tools/wrentest_unit/test_package.py` asserts the dictionary's hashes, that `README_en_US.txt`, `SOURCE.md` and `NOTICES` name the same release, and that `SPELLBOOK_VERSION` matches the `Cargo.toml` pin and `Cargo.lock`.
- `spellbook` is pinned (`=0.4.2`) in `[workspace.dependencies]`, as is `wren-spell`, and the skeleton crate depends on it from step 0, so `Cargo.lock` is final before Stage A. It ships its MPL-2.0 text; its `hashbrown` and `foldhash` are already locked, so it is the one package added. Step 0 records the notices inventory before and after (the commands are in §6.1).
- `spellbook` calls itself alpha and has no phonetic suggestions. It is used unmodified. If a fix is ever needed, it goes upstream or into `vendor/` with a `WREN.md`; the MPL then requires the changed files to stay published, which a public repository satisfies.

## 4. Behaviour spec

### 4.1 What text is checked

A line's prose ranges, by the classifier's kind (`wren_markdown::block`):

| Line | Prose |
|---|---|
| `Kind::Prose`: a paragraph line, list item, block quote, task (open or done), table row, setext heading text | From the end of the container prefix and task box to the end of the line, minus the inline exclusions below. |
| ATX heading | Its text range, minus the inline exclusions. |
| A link reference definition: any line whose text starts `[label]:`, with a non-empty label holding no unescaped bracket (also where CommonMark would not allow one, since the highlighter sees one line at a time). A footnote definition `[^label]:` is not one and keeps its prose. | None. |
| Front matter, fenced code (fences and body), indented code, HTML block, thematic break, setext underline, blank | None. |

Inline exclusions:
1. Code spans, backticks included.
2. HTML comments that open and close on the line.
3. A link or image: everything but its text. `[the text](url "title")` and `![alt](url)` are checked in `the text` and `alt` only; an image's `!` is excluded with its brackets. A link or image inside a link's text is excluded the same way, so in `[![alt](img.png)](u)` only `alt` is prose; the inner one keeps the outer text's colours and locks, so the line looks as it did.
4. Bare URLs and autolinks: `<scheme:…>` (a scheme as CommonMark's) and `<user@host>`, brackets included. Autolinks are found by a pass that only excludes; it sets neither colour nor lock.
5. Matched emphasis and strikethrough delimiters, so `_word_` is checked as `word`.

Nothing the highlighter colours, strikes or chips changes with the prose ranges.

Rules:
1. Prose ranges are sorted, apart, non-empty and on character boundaries, for any input. They are the maximal runs of text with nothing excluded, so they hold white space and punctuation too, and two consecutive ranges are separated by at least one excluded byte.
2. They depend only on the highlighter's state at the line and on the line, like the rest of `LineDecor`.
3. A lazy continuation line is prose of its paragraph.
4. A delimiter of exclusion 5 with a letter or digit directly on both sides is not excluded: it stays inside the prose range, so `snake_case_name` and `un*believ*able` each stay one chunk, which rule 4.2.1 then skips. A mark therefore never covers or abuts a delimiter inside a word.

### 4.2 What a word is

Within each prose range:
1. Split at Unicode white space into chunks. A chunk is skipped whole when it contains `@`, `/`, `\`, `<`, `>` or `[^`; when it starts with `#` followed by a letter; or when it has a run of `_`, `*` or `~` with a letter or digit directly on both sides (so `strong**wrod**ish` is skipped like `un*believ*able`). That covers addresses, paths, inline tags, footnote marks, tags, identifiers and emphasis inside a word.
2. In the remaining chunks, candidate words are UAX #29 words (`unicode_word_indices`). A hyphenated compound is therefore checked part by part. `_` belongs to a UAX #29 word, so `_word_` left in prose is one candidate that rule 4 never checks; the editor excludes matched delimiters (§4.1), so there it is checked as `word`.
3. `’` is looked up as `'`.
4. A word is checked only if all of these hold; otherwise it is never marked:
   - every character is a letter at or below U+024F, or an apostrophe (so digits, an inner `.` or `:`, non-Latin scripts and a word written with a combining mark are out);
   - it has 2 to 40 characters;
   - it is not an all-capitals word: two or more letters, every cased one upper case (one that upper-cases to itself, so a title-case letter such as `ǅ` is not), optionally followed by `s` or `'s`. So `README`, `README's` and `PRs` are skipped;
   - it is not cut by its prose range: a candidate that starts at its range's start with a letter or digit just before it in the line, or ends at its end with one just after it, or has a UAX #29 mid-word joiner (`'`, `’`, `.`, `:` and the rest of `MidLetter` and `MidNumLet`, not `_`) at the range's edge with a letter beyond, is a piece of a longer word and is skipped. The editor's ranges never cut a word (rule 4.1.4); this guards against a bad caller.
5. A checked word is correct when any source accepts it as written. Sources follow Hunspell's case rules: a lower-case entry is also accepted capitalised; a mixed-case word such as `CommonMark` is looked up as written and marked if unknown (decision 5). No source accepts the empty word.
6. A word ending in `'s` is also correct when the part before it is.
7. A mark is the byte range of one incorrect word. Prose ranges given to `Speller::marks` are tolerated, not trusted: each is clamped to the line and shrunk to character boundaries, and a range that starts before the end of an earlier one keeps only its part after it. The marks stay sorted, apart (a gap between any two) and inside the ranges given, for any input.
8. **Suggestions** (`Speller::suggestions`) ask each source in order with straight apostrophes and read a correction holding `’` with `'`, so `don’t` and `don't` are one correction (turning them back is the editor's, rule 4.4.6). They leave out an empty correction, one holding a control character, and the word itself; stop asking once they have `limit`; and give nothing for a word rule 4 never checks, which keeps a 10,000-character word away from the suggester. Corrections split in two (`mi steak`, `Common Mark`) are kept as the source offers them, since removing them would also remove good ones such as `alot` → `a lot`.

### 4.3 Marks

1. Marks exist only on the owned engine, with a proofreader set (§3.3).
2. A mark is drawn on every visual row its word occupies, across the word's glyphs on that row.
3. While the editor has focus and nothing is selected, a mark is not drawn when the caret is at its end, or is separated from its end only by apostrophes (`'` or `’`, so the mark under `wouldn` does not flash while `wouldn't` is typed). "Has focus" is the editor widget's keyboard focus, whatever the window's; "nothing is selected" means the cursor has no selection anchor (an anchor equal to the caret counts as a selection). The caret's on-screen position plays no part, so a wrapped word's mark stays hidden on its visible rows while the caret's row is scrolled out of view. Nothing else hides a mark: not a selection, not the current-line band.
4. The zigzag is made of 1 px squares in a 4 px period: at phases 0, 1, 2, 3 the square sits 2, 1, 0, 1 px below the zigzag's top. The top is `round(baseline + 2)`, moved up if needed so all 3 px stay inside the row. On each row, with `L = ceil(left)` and `R = floor(right)` of the word's glyph extent in logical pixels, there is one square for each integer `x` with `L ≤ x < R`, at phase `(x − L) mod 4`; the phase restarts on every row, and a row with `R ≤ L` draws nothing. `L`, `R` and the top are rounded after adding the text origin, so the squares sit on the window's pixel grid wherever the editor is. Size does not change on heading rows.
5. Its colour is `Style.misspelling`. It is drawn as pictures, one per word and row, after the selection rectangles and the menu word's tint and before the text (§3.3), clipped to the rows as the bands are.
6. Marks follow the text as it is on that frame; there is no delay and no stale mark after an edit, undo, redo or replaced text.

### 4.4 The suggestions menu

1. **Target.** From the keyboard: the misspelled word that contains or touches the caret (the caret is the moving end of a selection), whether or not rule 4.3.3 is hiding its mark. From the pointer: the misspelled word under a right-click. Anything else opens nothing, and a right-click elsewhere changes neither focus nor the caret, as today; on the classic engine or with no proofreader a right-click finds no word. A right-click that opens the menu first gives the editor the keyboard as a left click does (ending a drag selection in progress, and taking the keyboard from a focused text field, §3.4), and the app makes the editor its active column.
2. **Opening** computes up to five suggestions and shows them in order, the first one selected. When the word holds `’`, every `'` of every suggestion becomes `’` (rule 6); then empty suggestions, ones holding `\r` or `\n` and duplicates are dropped, and the five are counted after that. Opening does not move the caret or change the selection; while the menu is open the target word is tinted with the selection colour. If the word's first row is not fully in view, opening first scrolls it into view as find does. With no suggestions the menu shows one inactive row, "No suggestions", which is never selected. Opening while a menu is open replaces it with a fresh one; an opening that finds no word closes an open menu.
3. **Place.** Left edge at the word's left edge on the row where the word starts (the first visible row holding any of its glyphs); top 4 px under that row. If it does not fit under the row inside the editor less its 8 px margin and fits above, it goes above, 4 px over the row; if it fits neither way it goes where there is more room between the row's 4 px gap and that margin. It is then moved as needed to stay 8 px inside the editor on every side; a menu larger than the editor keeps its top and left edges 8 px inside. Edges are whole pixels. Row text is inset 8 px inside its row (12 px from the menu's edge), so the width is the longest text plus 24 px, rounded up, between 160 and 320 px, and never more than the editor's width less 16 px; longer text is cut between graphemes with an ellipsis, dropping a space before it. Rows are 30 px; 4 px padding, 8 px corner radius, a 1 px border; the selected row's fill has 4 px corners. While the word's first row is out of view no menu is drawn; it stays open and keys still reach it.
4. **Keys while open.** Up and Down with no modifier move the selection and stop at the ends. Enter with no modifier accepts the row the host's menu has selected (`AcceptSelected`). A press of a modifier key alone does nothing; the lock keys (Caps, Num, Fn and Symbol Lock) count as modifier keys. Escape, with or without modifiers, is the host's (rule 8). The suggest key closes the menu and opens it again at the caret. Any other key press, modified arrows and modified Enter included, closes the menu and is then handled as if it had not been open; a key the editor ignores, or a host shortcut, closes it without capturing the press, so the host still gets it. With only the inactive row, Up and Down are taken by the menu and do nothing, and Enter closes it; neither reaches the text. Within one batch of events (§3.3) each key sees the menu as the earlier ones left it: Down then Enter accepts the second row, and a key after one that closed the menu, or after a suggest press that finds no word, is the text's. A suggest press after a key that moved the caret in the same batch opens at the caret as that key left it, and the keys after it act on what that opened. Over a menu open when the batch starts, the pointer's moves over the menu and presses on it come before the batch's keys; a press anywhere else comes in its order with them. A press inside the editor after an opening in the same batch comes in its order with the keys and acts on what that opening did: when it opened a menu, the press closes it and does nothing more, as a press elsewhere does under rule 5, even where that menu's row will be drawn, because the menu is first laid out in the next frame; and otherwise what the press does with no menu (a left press clicks, a right press on a misspelled word opens the menu). After an opening in the same batch, an Up, Down or Enter that the host's key policy gives to the host closes the menu and reaches the host, as with no menu. (Wren's own policy gives the host no key without Ctrl, Alt, Super or a function key, so this concerns other hosts of the editor.)
5. **Pointer while open.** Moving over a row selects it; a left click on a row accepts it, and a click on the inactive row closes the menu; a click on the menu's padding does nothing. Over the menu the pointer is a hand on a suggestion and the default arrow elsewhere. A press of any button elsewhere inside the editor closes the menu and does nothing more, so the caret does not move, and a left press that only closed the menu is not the first half of a double or triple click: a left press after it at the same point is a single click. A press outside the editor closes the menu and then does what it does today.
6. **Accepting** replaces the word with the suggestion as one undo step that closes any typing group, puts the caret after it with nothing selected and closes the menu. Line endings and every other byte are untouched. If the word as written holds `’`, each `'` of the suggestion is shown and written as `’`. If the menu's line no longer holds the menu's word at its mark (the word moved along its line, its line was removed or its bytes changed), accepting changes nothing and closes. Accepting with no menu open, or a row that holds no suggestion (the inactive row, or past the last), changes nothing and closes any open menu.
7. **Closing without accepting** leaves the text, the caret and the selection as they were.
8. **Closed by the host.** While the menu is open the editor gives Escape no binding: it neither drops focus nor captures the press. The app, which always receives Escape (`key_input`, `preferences.rs:501`; the `Message::Escape` arm, `app.rs:477`), calls `close_spell_menu()` before anything else Escape or a filter box's Escape does (in the `Message::Escape` arm, before it cancels gestures), and stops if it returns true: focus stays in the editor and an open find bar stays open. The menu also closes when the palette or a dialog opens, the note, workspace or engine changes, the text is replaced or changes by any other means, the editor scrolls (except for rule 2), loses focus or is resized, the window loses focus, the font, text size, theme or settings change, or an input method starts composing. Who closes it:
   - *The editor, in `EditorView`:* any `perform` (an edit, a motion, a click, a scroll), `intent`, `apply`, `undo`, `redo`, a changed text, `replace_text`, `set_backend`, `move_cursor` (and so `select_byte_range` and `reveal_literal`), `scroll_to_line`, `restore_viewport`, `find_next` and `find_previous`, and a changed font, text size, line height, gutter, Markdown assist or proofreader (another `Arc`, or `None`). Setting one to its current value keeps the menu.
   - *The editor widget,* which publishes `SpellEvent::Close`: the window losing focus; the editor losing focus (a change from focused, so a menu opened in an unfocused editor stays); an input method starting to compose; any press outside the menu; and a change of the widget's size, `Style`, palette, font, text size, gutter or measure made while the menu is open. The widget compares against the size and look the open menu was first laid out with, so a change made just before the menu opened, even in the same batch as the suggest key, does not close it.
   - *The app:* after every message an open palette or dialog closes the menu, whatever opened it; so do a theme that changes (a picker preview, the system appearance, a choice), any applied settings change (a reload that changed `config.conf`, every saved toggle) and the window losing focus. A reload that finds both files unchanged keeps the menu; a changed `words.txt` replaces the proofreader, which closes it. The note, the workspace and the engine close it through the editor (`replace_text`, `set_backend`), and a workspace load withdraws the proofreader (§3.4).
9. The menu never opens on the classic engine, with the setting off, before the dictionary has loaded, or when no note is open for editing (§3.4 withholds the proofreader in the last three cases).

### 4.5 Commands, keys and the setting

| Command | Palette title | Group | Scope | Default key | Enabled |
|---|---|---|---|---|---|
| `editor.toggle_spell_check` | Toggle spell check | Markdown | App | none | always; its row shows `on` or `off` |
| `editor.spell_suggest` | Suggest spelling | Markdown | Editor | Alt+. (Linux), ⌘. (macOS) | advanced engine, setting on, dictionary loaded, editor ready |
| `settings.open_words` | Open spelling word list | App | App | none | always; blocked with the message `settings.open` gives when there is no settings location |

1. The toggle flips `spell-check`, applies it at once and saves it to `config.conf`. On the classic engine it still flips the setting.
2. `editor.spell_suggest` from the palette closes the palette, goes back to the editor as the Markdown commands do (`return_to_editor`, which also unfolds a folded browser), and chains `SpellEvent::OpenAtCaret` after the focus operation, so the menu is not closed by the focus change that opened it. With no target it does nothing; the command reports `completed` either way.
3. `settings.open_words` creates `words.txt` if it is missing, holding only a short comment header, and opens it as `settings.open` opens `config.conf` (`preferences.rs:460`), sharing its opening code. Creating it is a small sibling of `settings::ensure_exists` in `wren-core` (`settings::words_path` names the file); the directory is the one that holds `config.conf`. It creates the directory, then the file with `create_new`, so it never replaces a file that appeared in between, and fails with `settings.open`'s `Could not create …` message when the path is a directory.
4. `spell-check = on|off` in `config.conf`; default on; the template documents it, says it needs `editor-engine = advanced`, and names the suggest key and `words.txt`.
5. The suggest key can be rebound with a `keybind` line like any other.
6. Of the three, only `settings.open_words` is added to the macOS menu bar, after "Settings…" (`macos_menu.rs`), as "Spelling Word List…". A menu item for the suggest command would take ⌘. before the editor.

### 4.6 The word list

1. `words.txt` in the directory that holds `config.conf`. UTF-8, one word per line, split as `str::lines` splits (`\n` or `\r\n`; a stray `\r` at a line's end is trimmed as white space); a byte-order mark at the start of the file (anywhere else it is part of a word), blank lines and lines whose trimmed text starts with `#` are ignored; surrounding white space is trimmed; `’` is read as `'`, both in entries and in the words the list is asked about.
2. An all-lower-case entry (every character lower-cases to itself) is accepted in lower case, capitalised and upper case. An entry with any capital is accepted exactly as written only. "Capitalised" upper-cases the first character, not the first letter, so `'tis` is not also accepted as `'Tis`, as in Hunspell. An entry need not be a word §4.2 checks (`x86` is stored, and harmless).
3. A line that is not one word (inner white space, or longer than 40 characters, counted after `’` becomes `'`) is skipped with a warning naming its line number, counted from 1 over every line: `line N: not one word` or `line N: longer than 40 characters` (a line that is both is "not one word"). The app prefixes the warnings with the file's path (§3.4).
4. The list is read when §3.4 says, and only from a regular file of at most 1 MiB of UTF-8; anything else is an empty list and a warning. Wren never writes it, except to create it for rule 4.5.3.
5. It offers no suggestions of its own.

### 4.7 Budgets

Measured in the planning spike (release build, `spellbook` 0.4.2, the distribution's SCOWL `en_US`): dictionary load 8 ms; lookups 0.1 to 0.2 µs per word; suggestions 5 to 36 ms per word.

1. The dictionary loads off the UI thread.
2. Marks for a full editor of text (60 rows of 100 characters) cost under 0.5 ms a frame in release.
3. Opening the menu takes under 100 ms in release at the 95th percentile over the marked words of the acceptance note (§5.5).
4. With the setting on, the time to open the acceptance note and to type 200 characters into it is within 10 % of the setting off, in release.

No test asserts a time. The spec lens of the end review measures rules 2 to 4 and records the numbers in §7.5. At the end review rules 1 to 3 were met, so suggestions stay on the UI thread (open question 3), and rule 4 was missed on tiny-skia; `spell-endfix` met it on opening, on both renderers (§7.5 `spell-endfix`, E2), and its typing half stays inconclusive.

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
- **`editor.key`** `{ "key", "mods" }`: runs the editor's own key routing on one press (`key_message`) and applies every message it returns, in order, through `App::edit`, as the widget would, so rules 4.4.4 and 4.4.8 can be driven without a window and on Wayland.
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
3. *A word cut by a prose range is not checked.* A candidate word that starts where its prose range starts with a letter or digit just before it in the line, or ends where the range ends with one just after it, is a piece of a longer word of the line, so it is skipped rather than marked (`marks` promises "words of `line`"). So is one with a UAX #29 mid-word joiner (`'`, `’`, `.`, `:` and the rest of `MidLetter` and `MidNumLet`) at the range's edge, inside or outside it, and a letter beyond that: `wouldn't` given `0..6` marks nothing (fixed after the review). `_` is not such a joiner, since outside a range it is an emphasis delimiter. The editor's ranges never cut a word (rule 4.1.4), so this only guards against a bad caller.
4. *Bad prose ranges are tolerated, not trusted.* Each is clamped to the line and shrunk to character boundaries; a range that starts before the end of an earlier one keeps only its part after it, so one out of order is dropped. The marks stay sorted, apart (a gap between any two) and inside the ranges given for any input; the seeded test passes ranges that are unsorted, overlapping, reversed, past the end and inside characters.
5. *`_` belongs to a UAX #29 word* (it is `ExtendNumLet`), so with the delimiter in prose `_word_` or `trailing_` is one candidate that rule 4.2.4 never checks. Matched emphasis delimiters are not prose (exclusion 5), so the editor's `_word_` is still checked as `word`.
6. *"Every cased one upper case"* is read as "upper-cases to itself": a title-case letter such as `ǅ` is not upper case. A word written with a combining mark (`cafe` plus U+0301) is never checked, since the mark is not a letter at or below U+024F.
7. *The acceptance note needs no change*: with the bundled 2026.02.25 release its marks are exactly the nine words, in order (`speller::tests::the_acceptance_note_has_exactly_its_nine_marks`, with the editor's prose ranges given by hand).
8. `Speller::suggestions` was still the stub after C2 (C4 filled it). The seeded test already called it on a sample of its marks.

Measured while trying to break it, in a debug test build (`spellbook` at the workspace's `opt-level = 2` for dependencies): the slowest of 200,000 random 2 to 40 character checks took 39 µs, far under the 50 ms the brief names; no input panicked.

*C3 and C4.* Where §4.6 or §3.2 left a case open:

1. *A comment is a line whose trimmed text starts with `#`*, as `config.conf` reads its comments, so an indented `# note` is ignored too. A byte-order mark is stripped only at the start of the file; anywhere else it is part of a word.
2. *A line is split as `str::lines` splits it*: `\n` or `\r\n`, and a stray `\r` at a line's end is trimmed as white space. Line numbers in warnings count from 1 and include comments and blank lines.
3. *The warnings read* `line N: not one word` and `line N: longer than 40 characters`, the shape `config.conf`'s warnings have, so W1 can prefix them with the path. A line that is both is reported as not one word. The 40 is counted in characters after `’` becomes `'`.
4. *"All lower case" is read as C2 reads "upper case"*: an entry is all lower case when every character lower-cases to itself, so an entry starting with title-case `ǅ` is exact. "Capitalised" upper-cases the first character, not the first letter, so `'tis` is not also accepted as `'Tis`; Hunspell does the same. An entry need not be a word §4.2 checks: `x86` is stored, and harmless.
5. *`WordList::accepts` reads `’` as `'` itself*, like `Hunspell`, so the source keeps rule 4.2.3 when asked directly, not only through `Speller`. `WordList` derives `Debug` and `Default` (the empty list); its field stays private.
6. *`Speller::suggestions` passes and returns straight apostrophes*: the word is asked about with `'`, and a correction holding `’` is read with `'`, so `don’t` and `don't` are one correction. Turning them back into `’` is the editor's job (rule 4.4.6).
7. *It leaves out* an empty correction, one holding a control character (a line break would change the note's lines on accepting), and the word itself. It asks no later source once it has `limit`, and nothing for `limit` 0.
8. *A word that rule 4.2.4 never checks gets no suggestions.* Such a word is never marked, so the menu never asks; the guard also keeps a 10,000-character word away from `spellbook`'s suggester.
9. *Split corrections are kept as the source offers them*: for `misteak` the bundled dictionary offers `mistake`, `mi steak`, `mi-steak`, `steak`; for `CommonMark`, `Common Mark` first. Removing them would also remove good ones such as `alot` → `a lot`. The menu or the owner may want to revisit this.
10. *The first suggestions for the acceptance note* (`speller::tests::the_acceptance_notes_marks_have_these_first_suggestions`) are `cheek` for `chek` (`check` is second), `pear`, `replacements`, `bundled`, `design`, `Common Mark`, `misspelled`, `known` and `mistake`; W4's step 3 can expect them. In a debug test build each took 6 to 21 ms.

*The review* (§7.1, then the fix step):

11. *Suggestion time grows with the word's length*, past what §2.2 choice 6 and §4.7 assume (5 to 36 ms). In the test build (`spellbook` at `opt-level = 2`), on a machine that was not quiet: `teh` 19 ms, `recieve` 27 ms, `definately` 31 ms, `internationalizationnn` (22 characters) 77 ms, `antidisestablishmentarianismm` (29) 102 ms, `Supercalifragilisticexpialidociuos` (34) 116 ms, `edereovernonedantisunlymisantioutseanti` (39) 163 ms; the reviewer's worst over 300 random 2 to 40 character Latin words was 221 ms. Nothing is changed here: open question 3's default decides at the end review. So the end review's rule 4.7.3 measurement, in release, adds a few long misspelled words (20 to 40 characters) to the acceptance note's nine. If they miss 100 ms, the choices are a shorter length limit for suggesting than the 40 for checking, or suggestions off the UI thread (open question 3).

- Counts: 0 blockers, 0 majors, 2 minors.
- Fixed: the word cut at an inner apostrophe (`e497281`, tests `words::tests::a_word_cut_at_an_inner_apostrophe_is_not_checked` and new cases in `speller::tests::a_prose_range_that_cuts_a_word_marks_no_piece_of_it`).
- Not fixed: suggestion time for long words (item 11). Reproduced; bounding it is a behaviour choice that open question 3 leaves to the end review's measurement, so it is recorded and carried there.

#### `spell-marks`

*M1, prose ranges.* Built as §3.3 says: an `excluded` flag on the scanner's `Cell`, `Inline.prose`, and the scan run over ATX heading text and done-task text for `prose` only. A digest of every span, strike, chip, hang and code band over 20,000 generated documents is the same before and after the change.

Deviations:
1. *A footnote definition is not a link reference definition.* `[^label]:` keeps its prose, as §8's known limits assume; rule 4.2.1 skips the `[^label]:` chunk itself.
2. *A reference definition is recognised on any line whose text starts `[label]:`*, not only where CommonMark allows one (it cannot interrupt a paragraph): the highlighter sees one line at a time and the classifier does not report a paragraph's first line. The label must be non-empty and hold no unescaped bracket.
3. *Autolinks are found by a pass that only excludes*: `<scheme:…>` (scheme as CommonMark's) and `<user@host>`, brackets included. It sets neither colour nor lock, so the look of a line does not change; the URL inside keeps the bare-URL colour it had.
4. *Prose ranges are the maximal runs of text with nothing excluded*, so they hold white space and punctuation too, and consecutive ranges are separated by at least one excluded byte. An image's `!` is excluded with its brackets.

*M2 and M3, marks and the zigzag.* One commit: `spell_frame()`'s rectangles are the zigzag's, so the two items share their geometry. The widget asks the proofreader once per drawn line (not per wrapped row), and the on-demand path (`EditorView::marks_at`, a fresh `MarkdownHighlighter` over lines `0..=line`) gives the same marks as a frame for every line of the acceptance note.

Deviations:
1. *`marks_at` is private and only tests call it until `spell-menu` does*, so it carries `#[cfg_attr(not(test), allow(dead_code))]`, which U1 removes. §3.2 names no public on-demand accessor and none was added.
2. *`SpellFrame`, `DrawnMark` and `spell_frame()` compile under `any(test, feature = "test-support")`*, not the feature alone, so the crate's own tests can read them.
3. *A hidden mark is still reported, with its rectangles*: `DrawnMark.rects` is where the zigzag goes on each drawn row, clipped to the rows, whether or not rule 4.3.3 hid it. A row on which the word covers no whole pixel gives no rectangle, but the mark is still listed.
4. *What the editor accepts from a proofreader:* a line with no prose is not asked about, and a returned range that is empty or not on character boundaries of the line is dropped, so later code may slice the line with any mark.
5. *Rule 4.3.3's apostrophes are `'` and `’`*, and "has focus" is the widget's keyboard focus, whatever the window's.
6. *The zigzag's pixels are whole window pixels*: `L`, `R` and the top are rounded after adding the text origin, so the squares stay on the grid wherever the editor sits.

*Fix step.* Deviations:
1. *A link or image inside a link's text is excluded but not restyled.* In `[![alt](img.png)](u)` only `alt` is prose: the inner image's `!`, brackets and destination are excluded as an outer link's are, while their colours and locks stay as the outer link's text, so the line looks as before (a digest of every span, strike, chip, hang and code flag over 30,000 generated bracket-heavy documents is unchanged; their prose changes).
2. *Rule 4.3.3's "nothing is selected" is read from the cursor*: the cursor has no selection anchor. The caret's on-screen position plays no part, so a mark on a wrapped word stays hidden on its visible rows while the caret's row is scrolled out of view. A selection whose anchor equals the caret still hides nothing, as before.

Review: 0 blockers, 1 major, 1 minor.
Fixed: the major in `a3fe7a2` (a linked image's `!`, brackets and destination were prose), the minor in `30d54fe` (a caret on a row out of view did not hide its wrapped word's mark), each with a regression test.
Not fixed: nothing.

#### `spell-menu`

*U1 and U2, opening the menu, drawing it, and its keys and pointer.* The menu's state lives in `EditorView`, which hands it to the widget each frame; the widget draws it as an overlay (`proof/menu.rs`) and routes keys and presses to it. Where §3.2 or §4.4 left a case open:

1. *`key_message` returns `Vec<Message>`*, not `Option<Message>`: a key that closes the menu and is then handled as if it had not been open publishes two messages (`Spell(Close)`, then its own), and the harness must apply both, in order. The clipboard is not there without a window, so Copy and Paste give nothing and Cut only its delete; dropping focus (Escape with no menu) is no message. The physical key is unidentified, so routing falls back to the key's own character.
2. *Opening replaces an open menu* with a fresh one (suggestions asked again, the first selected), and an opening that finds no word closes the open menu. The editor keeps at most five suggestions, whatever the proofreader returns.
3. *A right-click that opens the menu ends a drag selection in progress*, as it takes the keyboard. A right-click on the classic engine, or with no proofreader, finds no word and changes nothing.
4. *A click on the inactive "No suggestions" row closes the menu*, as Enter does on it; a click on the menu's padding does nothing. Any button pressed in the editor off the menu closes it and does nothing more; only the left button accepts a row.
5. *With only the inactive row, Up and Down still route to the menu* (`Move`, which does nothing there), so they are captured and never reach the text.
6. *Escape with any modifier is the host's while the menu is open*, as plain Escape is. *"A modifier key alone"* includes the lock keys (Caps, Num, Fn and Symbol Lock) besides Shift, Ctrl, Alt, AltGr, Super, Meta, Hyper, Fn and Symbol.
7. *A key the editor ignores, or a host shortcut,* closes the menu without capturing the press, so the host still gets it. The suggest key on an open menu closes it and opens it again at the caret. The editor also drops `KeyRoute::Suggest` while an input method composes, as the host does (§3.4).
8. *Place:* "fits under the row" and "fits above" are measured against the editor less its 8 px margin, so a menu that fits never needs moving; "more room" compares the space from the row's 4 px gap to that margin on each side. A menu taller or wider than the editor keeps its top and left edges 8 px inside. Edges are whole pixels. The word's first row is the first visible row holding any of its glyphs, so a word that wraps whole to the next row is placed under that row.
9. *Sizes the plan did not name:* row text is inset 8 px inside a row (12 px from the menu's edge), so the width is the longest text plus 24 px, rounded up; the selected row's fill has 4 px corners. Text is cut between graphemes, and a space before the ellipsis is dropped.
10. *When the word's first row is out of view, no menu is drawn*; it stays open and keys still reach it. Rule 4.4.2 scrolls the row into view on opening, and a later scroll closes the menu, so this is only a frame between the two.
11. *Over the menu the pointer shows a hand on a suggestion and the default arrow elsewhere*, so the editor's text cursor does not show through.
12. *The causes the editor closes the menu for itself* (rule 4.4.8): in `EditorView`, any `perform` (an edit, a motion, a click, a scroll), `intent`, `apply`, `undo`, `redo`, a changed text, `replace_text`, `set_backend`, `move_cursor` (and so `select_byte_range` and `reveal_literal`), `scroll_to_line`, `restore_viewport`, `find_next` and `find_previous`, and a changed font, text size, line height, gutter, Markdown assist or proofreader (another `Arc`, or `None`); setting one to its current value keeps the menu. In the widget, which publishes `SpellEvent::Close` so the host hears of it: the window losing focus, the editor losing focus (a change from focused, so a menu opened in an unfocused editor stays), a change of the widget's size, of its `Style`, palette, font, text size, gutter or measure, an input method starting to compose, and any press outside the menu. The palette, dialogs, the note, the workspace, settings and Escape remain the host's (W3).
13. *`SpellEvent::Accept` with no menu open, or for a row that holds no suggestion* (the inactive row, or past the last), changes nothing and closes any open menu.
14. The menu's word is tinted from the same glyph extents as its marks, after the selection rectangles and before the zigzag, so the zigzag shows on the tint.

Checked in a real window as well as by the unit tests, on X11 with tiny-skia, through a temporary hook in the harness (not committed, since `spell-wire` hands the app its proofreader): the menu under a word, above a word on the last visible row, with "No suggestions", with an ellipsis, in Wren Dark and Wren Light; a native right-click opening it, a native pointer move selecting a row, a press in the editor closing it with the caret unmoved, and a press on the note list closing it and reaching the list.

*U3, accepting.* `EditorView::spell(Accept(row))` checks that the menu's line still holds the menu's word at its mark, then replaces it through `EditorView::apply` with the caret after the suggestion and nothing selected, so it is one undo step that closes any typing group, and `spell` returns true. Where the plan left a case open:

15. *The apostrophe rule of 4.4.6 applies when the menu opens*, so the rows show what accepting writes: when the word holds `’`, every `'` of every suggestion becomes `’`, then duplicates are dropped (`doesn't` and `doesn’t` become one row). A word written with `'` keeps the suggestions as the proofreader gave them.
16. *A suggestion is one piece of a line*: an empty one, or one holding `\r` or `\n`, is dropped when the menu opens, so accepting never inserts a line break and the caret's column is the mark's start plus the suggestion's length. Duplicates are dropped too, and the five are counted after both.
17. *"No longer the word"* is checked against the menu's line index, mark and word together: a word moved along its line, its line removed or its bytes changed all close the menu and change nothing. Every change made through `EditorView` already closes the menu (item 12), so this guard is reached only by a change made beside the view; it is tested by changing the backend directly.

*Fix step.* Deviations:

18. *Within one batch of events the widget routes against its own copy of the menu.* iced hands every event queued since the last frame to the same widget, the menu's overlay first, before the host applies a message, so the menu the view handed over is stale after the first event that changes it. The widget keeps a copy (`proof::OpenMenu`, held by the `TextEditor`, which the view rebuilds once the host has applied the batch). It starts from the view's menu, and each message the widget or its overlay publishes changes it as the host will change its own: a move or a hover moves the selection, any other message closes it (the host closes its menu on every action, intent, undo or redo), and so does Escape, which the host closes the menu on (rule 4.4.8; W3 must do so, or the copy stays closed until the next rebuild). Keys, presses, the overlay's pointer handling, its drawing and its pointer shape all read the copy, and the overlay does nothing once it is closed. So Down then Enter accepts the second row, a key typed after one that closed the menu is the text's, and a click that accepted leaves the next key to the text.
19. *An opening in the batch is taken as a menu with up to five rows and the first selected*, since the widget cannot know what the host's opening finds. A move past the real last row is clamped by the host, and accepting a row past it closes the menu without writing. If the opening finds no word, an Up, Down or Enter later in the same batch does nothing.
20. *The pointer comes before the keys within one batch*, because iced runs the overlay over the whole batch before the widget: Enter followed by a move over the third row, in one batch, accepts the third row. iced gives the two no other order, and the pointer position is the batch's last anyway.
21. *For W1:* a right-click that opens the menu takes the keyboard only from the editor's point of view. iced's `text_input` gives up focus on a left press, not a right one, so the find field or a filter box would keep it and a typed key would reach both (the review's minor). The editor cannot unfocus another widget, so W1's `Spell(OpenAt)` arm in `App::edit` must also return the focus operation for `note-editor` (which unfocuses every other focusable), with a test: focus the find field, right-click a misspelling, type a key, and check the field is unchanged.

Checked live as well as by the unit tests, on X11 with tiny-skia, through a temporary hook (not committed) whose suggester held the UI thread for 400 ms while `xdotool` typed: before the fix, Down Enter and Down Down Up Enter wrote `the` and `x` Enter left `cat txeh dog`; after it they wrote `tech`, and `x` Enter typed the `x` and then broke the line. The menu still drew its moved selection.

- Review: 0 blockers, 1 major, 2 minors.
- Fixed: the major (stale menu within a batch) in `59f4742`, with tests `widget::tests::keys_in_one_batch_see_the_menu_as_the_earlier_keys_left_it`, `a_key_that_closed_the_menu_leaves_the_next_key_to_the_text`, `after_escape_the_next_key_is_the_texts_as_the_host_closes_the_menu`, `a_key_after_the_pointer_in_one_batch_sees_what_the_pointer_did`, `keys_after_an_opening_in_one_batch_go_to_the_menu_it_opens` and `proof::tests::the_widgets_copy_follows_each_message_as_the_hosts_menu_will`; the minor on §3.2 and §5.2 still declaring `key_message -> Option<Message>` in `9faa4ca`.
- Not fixed: the right-click focus minor. It is real (iced's `text_input` unfocuses only on a left press), but only the host can move focus between widgets, so it is carried to W1 as item 21.

#### `spell-wire`

*W1, the setting, the dictionary, `words.txt` and the proofreader.* The app side lives in `crates/wren-iced/src/app/spelling.rs`: `Spelling` holds the dictionary (an `Arc<Hunspell>`), the last read of `words.txt` and the proofreader built from the two (a `Speller` over both, behind a local `Proofreader` newtype); `wren-core` gives the setting and `settings::words_path`. Where §3.4 or §4.4 left a case open:

1. *The proofreader is synced after every message* (`App::update`), and after `apply_preferences` and a reload, not only where `markdown_assist` is set: "a note is open for editing" (`ensure_editor_ready`) changes with the note, the workspace and its loading, on many paths. Handing over the same `Arc` keeps an open menu, so syncing is free. The engine is not part of the rule; the editor ignores a proofreader on the classic engine (§3.3).
2. *The dictionary loads at launch whatever the setting and the engine*, on a thread of its own (`wren-spell`); a `Task::perform` future alone would run on the UI thread until it first waits, as `threaded_load` notes for workspaces.
3. *A read of `words.txt` made before the launch load arrives* (a focus, say) is newer than the load's and is kept. The load's warnings show when they differ from the warning shown at startup, so a banner shown since then is not replaced by the same text.
4. *An unreadable `words.txt`* (a directory, invalid UTF-8, over 1 MiB, as `settings::read_text` reads `config.conf`) gives an empty list and the warning `could not read the file: …`; the dictionary keeps working. A missing file is an empty list and no warning.
5. *The two files' warnings are joined with `; `*, `config.conf`'s first, each after its own path. A reload re-reads `words.txt` before `config.conf`, and returns a warning when either changed.
6. *A dictionary that fails to load* (it is bundled, so only a broken build) shows `Spell check is unavailable: …` in the banner, and there is no proofreader.
7. *`App::edit`'s `Spell` arm makes the editor the active column only for `OpenAtCaret`, `OpenAt` and `Accept`.* The widget publishes `Close` for a press outside the editor too, and that press may choose another column; `Move` and `Hover` change nothing about where the keyboard is. `OpenAt` also returns the focus operation for `note-editor` (§7.5 `spell-menu` item 21); the unit test checks that the task exists, and the real-window check (the find field keeping no key) is W4's.

*W2, the commands and the key.* Where §4.5 or §5.2 left a case open:

8. *`CommandId::editor_route()` replaces `intent()` in the public use*: the Markdown commands route to `KeyRoute::Intent`, `editor.spell_suggest` to `KeyRoute::Suggest`; `intent()` stays as the private half. The rule test is now `editor_scoped_commands_are_exactly_those_with_an_editor_route`.
9. *`editor.spell_suggest` from the palette* goes back to the editor as the Markdown commands do (`return_to_editor`, which also unfolds a folded browser), then chains `Message::Editor(Spell(OpenAtCaret))` after the focus operation. It reports `completed` whether or not a word was at the caret.
10. *`settings.open_words` creates `words.txt` with `create_new`*, after creating the directory, so it never replaces a file that appeared in between; it fails, with `settings.open`'s `Could not create …` message, when the path is a directory. Opening is shared with `settings.open`. The menu-bar item reads "Spelling Word List…".
11. *`config.conf`'s template documents `spell-check`* with the suggest key and `words.txt`.
12. *`test_navigation.py` expects the toggle's state to be `on`*, the build's default, because the harness writes no `spell-check` line before W4; W4 makes it the lane's value.

*W3, the theme and the Escape order.* Where §3.4 or rule 4.4.8 left a case open:

13. *The theme mapping is step 0's*, unchanged; W3 adds a test that every built-in theme maps as §3.4 says and draws the zigzag, the menu's rows and its border in colours distinct from what they sit on.
14. *Escape:* `key_input` closes the menu for a plain Escape and for a filter box's Escape before anything else, and the `Message::Escape` arm does too, before it cancels gestures, so a test or any other sender of the message gets the same order. Closing returns no task, so focus stays where it is.
15. *The app's closing causes.* After every message, an open palette or dialog closes the menu, whatever path opened it. A theme that changes (a picker preview, the system appearance, a choice) closes it, as does any applied settings change (`apply_preferences`: a reload that changed `config.conf`, every saved toggle) and the window losing focus (which the widget also reports). A reload that finds both files unchanged keeps the menu; a changed `words.txt` replaces the proofreader, which closes it. The note, the workspace and the engine close it through the editor (`replace_text`, `set_backend`), and a workspace load withdraws the proofreader (§3.4), which closes it too.

*W4, the harness.* `tools/wrentest/test_spell.py` over `tools/wrentest/fixtures/spell.md`; the requests live in `app/test_requests.rs`. Where §5.1, §5.2 or §5.5 left a case open:

16. *`editor.key` also runs the app's half of the press.* After applying the editor's messages through `App::update`, it hands the press to the app's key handling as the window subscription would, with the status the widget would give it: captured when the editor published anything, or for Escape with no menu open (the editor then drops focus, which the request cannot show); otherwise ignored. So Escape with the menu open reaches `key_input` as plain Escape and closes only the menu, and an arrow the menu took never moves a browser column. Keys are named (`up`, `down`, `left`, `right`, `enter`, `escape`, `tab`, `space`, `backspace`, `delete`, `home`, `end`, and the modifier keys `shift`, `control`, `alt`, `super`, `caps_lock`) or one character; `mods` lists `shift`, `ctrl`, `alt` and `logo`.
17. *`editor.spell`'s menu also reports `line`, `start` and `end`*, the mark it was opened for, and a mark's `word` is read from the editor's own line through a new test-support accessor, `EditorView::line_text`, since the host cannot split the text on the editor's line endings itself. `bounds` is `null` until a frame has drawn the menu.
18. *`editor.spell_event` goes through `App::update`*, as a widget's message does, so the syncs that follow every message run, and replies with the `editor.get` snapshot, as `editor.key` does. It offers no `open_at`: a point is the native cases' job.
19. *`WRENTEST_SPELL`:* any value but `off` or `on` (an empty one included) is a `ValueError` when a `WrenApp` is made. The test's settings name `spell-check` when one of their `key = value` lines has that key; a comment that mentions it does not count. `WrenApp.words_path` names `words.txt`. The unit tests' base clears the variable, so what they expect in `config.conf` does not depend on the lane. `test_editor.py:361` writes the lane's line back with the engine it switches to.
20. *The acceptance script* opens the note as `Notes/Inbox/Spell chek plan.md` in a workspace, beside a second note for step 5. Step 2 types ` wrod` and a space at the end of the quote line, then deletes them. Step 3's expected first suggestion is the one `spell-core` pinned, `cheek` for `chek`, with `check` among the rows. In step 6, "the suggest command does nothing" is checked both ways: the key opens no menu, and the command is blocked, as a disabled one is. Step 5 clicks the second row at the menu's 4 px padding plus one and a half 30 px rows.
21. *Cases beyond the script:* the lane's own setting (no marks and no menu under `off`, the nine marks under `on`); the menu's keys and events without a window (a modifier alone, hover, move, Shift+Down closing and then selecting, accepting a row); and two native X11 cases, the find field giving up the keyboard to a right-click (§7.5 `spell-menu` item 21: the typed key reaches the text and the field is unchanged), and the suggest key, Down, Escape and Down Return with the find bar open (Escape closes only the menu, and the bar stays).
22. *Found by W4's gates, not fixed in W4:* with `WRENTEST_SPELL=on`, `test_markdown_keys.py::test_a_group_of_large_notes_keeps_views_and_smart_keys_cheap[advanced]` fails on X11 with tiny-skia, three runs of three, while `off` passes. Its note is 5,000 list lines of Latin filler, so a screen holds about 115 marks, and the zigzag of rule 4.3.4 is one 1 px quad per pixel of each word, some 5,000 quads a frame. With them drawn, a request answered after the note opens waits 4 to 12 s (the client gives up at 5 s); with the same build drawing everything but the zigzag's quads, 0.2 to 0.7 s. The marks themselves are cheap: `visual_rows` takes under 1 ms on that screen and no `App::update` or widget `update`, `layout` or highlight takes over 30 ms, so the time is the renderer's. It belongs to `spell-marks`' drawing (§3.3, rule 4.3.4), not to the harness, and is left to the chain's review and fix (exit criterion 2 needs the `on` lanes to pass).

*W5, the scenes.* `capture_scenes.py --only spell` puts the acceptance note at `Notes/Inbox/Spell chek plan.md`, as `test_spell.py` does, and each scene asserts the state it shows before the shot. Where §5.3 left a case open:

23. *`--only spell` needs `--engine advanced`*; with the classic engine the script stops with an error instead of shooting four empty scenes. The other modes write no `spell-check` line, so they keep the lane's setting (`off` unless `WRENTEST_SPELL` says otherwise) and their scenes are unchanged.
24. *`spell-marks`* has the caret at the end of `apear`, so that mark is the one rule 4.3.3 hides and the other eight show. *`spell-menu`* opens under `replacments` with the caret inside it.
25. *`spell-menu-empty`:* every marked word of the acceptance note has suggestions, so the scene types ` xqzvkpw` before the quote's full stop (the word `wren-spell`'s `hunspell` tests pin as having none), opens the menu at its end, and undoes the typing afterwards.
26. *`spell-menu-above`:* the acceptance note fits in the view, so no word of it is on the last visible row. The scene appends 60 rows of correctly spelled filler and a last row ending in `wrogn`; typing there leaves that row at the bottom of the view. The caret moves two characters back into the word, so its mark shows under the tint, and the script asserts the menu's bottom edge is above the word.

Captured on X11 for all eleven themes on tiny-skia and for Wren Light and Wren Dark on wgpu (52 files under `target/scenes/advanced/<renderer>/<theme>/`). Looked at: Wren Light and Wren Dark on both renderers, Nord, Dracula and Solarized Light. In each, the eight marks show as zigzags in the theme's misspelling colour and `apear` has none, with no mark in the front matter, the code span, the link targets, the autolink, the e-mail address, `<kbd>`, the done task or the fence; the menu sits 4 px under `replacments` with five rows and the first selected, the word tinted with its zigzag over the tint; "No suggestions" shows one inactive row; and `wrogn`'s menu opens above its row. The two renderers differ only in antialiasing. Not opened: Gruvbox Dark, Rosé Pine, Everforest Dark, Tokyo Night, Catppuccin Mocha and Catppuccin Macchiato.

*W6, the docs.* The pages W6 names, written from the code as built (W1 to W5's choices included), not from §4's text alone. Where W6's list left a case open:

27. *Beyond the list:* `docs/index.md` links the new page under "Next steps"; `configuration.md` also says, under its opening paragraph, that `words.txt` lives in the same directory; `keybindings.md` gets a "Spell check" subsection under "Inside the editor" with the suggest key, adds the three commands to the bindable actions, and says Suggest spelling is, like the Markdown commands, a key the app leaves to the editor; `architecture.md` gains a "Spell check" section on the three crates' seams, the `danger` token's second use and the dictionary's thread; `testing.md` also documents `capture_scenes.py --only spell` (W5's open issue) and `WrenApp(words=…)`.
28. *What the user pages leave out:* the zigzag's geometry, the menu's sizes, the batch rules of `spell-menu` items 18 to 20, and the item 22 cost on tiny-skia. They are contributor detail or not settled, and `architecture.md` names the zigzag's drawing only in outline.

*Fix step.* The review's one finding was item 22. The cost was tiny-skia's, not the quads': its compositor damages each changed primitive on its own, groups the rectangles by their distance from the window's corner, and repaints every layer once per group, so some 5,000 scattered 1 px quads made it repaint every layer many times a frame. Deviations:

29. *A frame's zigzag squares are drawn as one SVG picture*, one path of every square in the frame on whole pixels, with the colour in the SVG. The widget keeps the picture's handle while the squares and the colour stay the same, so an unchanged frame damages nothing. The squares are those of rule 4.3.4, clipped to the rows as before; rule 4.3.4 and decision 2.1.6 are unchanged. `wren-editor` turns on iced's `svg` feature, which the app already used, so `Cargo.lock` does not change. All 52 scene files, recaptured, match W5's quad-drawn ones pixel for pixel outside the header's time, the status bar and the caret's blink, on both renderers. At a fractional scale the picture is rasterized at the physical size, so its edges are antialiased as the quads' were; the join's 125 % capture (§6.4) still applies.
30. *The regression check is a harness case with a generous ceiling*, not "a few ms a frame": no test asserts a §4.7 budget, and debug frames on a shared machine vary. `test_spell.py::test_a_screen_full_of_marks_draws_without_a_stall` opens 600 rows of Latin filler with spell check on, waits for 100 shown marks, then moves five pages and asserts each drawn frame within 2 s. On the quad drawing it fails with the review's `TransportError` after 18.8 s; with the picture the whole case takes 1.4 s. The large-note test now takes 2.8 s with spell check on and 1.9 s off, against 23 s on before the fix at the same load.

- Review: 1 blocker, 0 majors, 0 minors.
- Fixed: the blocker (the zigzag's cost on tiny-skia, item 22) in `aa361a4`, with `widget::tests::a_frames_zigzags_are_one_picture_of_their_squares_on_the_pixel_grid`, `widget::tests::the_zigzag_picture_is_kept_while_its_squares_and_colour_are` and `test_spell.py::test_a_screen_full_of_marks_draws_without_a_stall`; `architecture.md` describes the picture. Gates after it: `make check`; `test_spell.py` 10 passed on X11 and 7 passed, 3 deselected on Wayland; the X11 tiny-skia suite with `WRENTEST_SPELL=on` and with `off`, 251 passed and 1 skipped each; the macOS-target clippy; the strict docs build.
- Not fixed: nothing from the review. The verify step found one new minor, after the fix round, and it is carried to the end review: `test_spell.py::test_menu_keys_close_then_act_and_rows_follow_the_pointer_events` (lines 315 to 318) reads `opened = menu(app)` straight after opening the menu, then asserts `menu(app) == opened` after a bare Shift. The menu's `bounds` comes from the last drawn frame (item 17), so when no frame has drawn the menu before the first read it is `null` there and filled in at the second, and the assertion fails. Seen once in 26 Wayland runs; the fix is to compare without `bounds` or to take `opened` from a drawn menu.

#### The end review

Run 1c reviewed `spell` at `bfadffe`; its one fix session took it to `da90f24`, and this record follows. The deviations of every chain subsection above, and of this one, are folded into §3 and §4 in the same commit; the log keeps them as they were found.

- *Step 1, the gates with the setting off at `bfadffe`:* all passed (the gates table below).
- *Step 2, the lenses and the Codex pass.* The text-safety lens found 1 major and nothing that changes, loses or wrongly writes a note's bytes, and no panic: over 6,000 seeded steps of the real app (366 accepts that changed the text), 450 native X11 key bursts, 12,394 editor-level accepts across every line ending, and the seeded crate tests run at 100 to 330 times their committed counts, every accept changed exactly its word, undo and redo were exact, every flush and switch wrote the editor's text, and with the setting off or on the classic engine no mark was drawn and no menu opened. The spec-ship lens found 1 major and 2 minors; every row and exclusion of §4.1, every rule of §4.2 and §4.6, the zigzag's pixels, the menu's targets, place, keys, pointer, apostrophes, `\r\n` and closing causes, the three commands, the crate rules, the lockfile, the licence and the notices held. The Codex pass reported 1 finding, by reading the source only (it ran no test).
- *Step 3, triage:* five findings in the queue (2 majors, 3 minors), none dropped, no duplicates between the sources. The Codex finding was reproduced as a failing unit test and lowered from major to minor under §7.1: no panic and no write, §3.4 specifies the synchronous re-read, and the same hang predated the branch for `config.conf`.

*Fix session.* The queue held two majors and three minors. Deviations:

1. *`SpellEvent` gains `AcceptSelected`* (§3.2): replace the word with the host's selected row, or close a menu with none selected. Enter sends it while the menu is open, so a batch's keys no longer need the widget to know how many rows the host's menu has. `Accept(usize)` stays for a click on a row and for the harness. This replaces `spell-menu` item 19: the widget now also checks the caret's word before it takes the suggest key's opening as a menu (from its own highlighter, or a fresh pass when that has not reached the line), so keys after an opening that finds no word reach the text. One case is left: a key that moves the caret earlier in the same batch, then the suggest key, is checked against the caret as it was before that batch.
2. *The widget closes the menu for a change of size or look only when the change came while the menu was open* (rule 4.4.8): it records the size and look the open menu is first laid out with and compares against those, before it handles the event. Losing focus is compared with the last event, as before.
3. *`settings::read_text` refuses anything but a regular file*, opening without blocking and enforcing the 1 MiB limit while reading, so a FIFO at `config.conf` or `words.txt` returns an error instead of hanging the UI thread. That is the whole fix: the word list is still re-read on the UI thread, as §3.4 says.

Budget 4.7.4 is not met and waits for the owner. Measured in release, tiny-skia, X11, with a smoke build in its own target directory, on the spec-ship lens's method: per round, the median of 20 `open_note` calls, each followed by an `app.screenshot` frame, and the median of a screenshot of the unchanged frame. Three or four rounds a row, the variants built from the head with throwaway switches:

| Build | Open, on | Open, off | Frame, on | Frame, off |
|---|---|---|---|---|
| Head (one picture a frame) | 62.6 to 67.3 ms | 55.0 to 57.4 ms | 32.4 to 37.4 ms | 25.3 to 29.6 ms |
| Marks computed, picture not drawn | 55.5 to 55.6 ms | 55.2 to 57.4 ms | 28.7 to 28.8 ms | 25.3 to 29.4 ms |
| Proofreader withheld from drawing | 55.2 to 57.0 ms | 55.9 to 56.1 ms | 28.8 to 30.2 ms | 29.0 to 29.3 ms |
| One picture per row | 70.3 to 70.8 ms | 55.8 to 56.4 ms | 28.8 to 29.2 ms | 28.7 to 29.6 ms |

Each cell is the range of the rounds' medians; each row's two settings ran in the same run. By run medians, the head opens 14 to 19 % slower with the setting on, and one picture per row 26 % slower.

Computing the marks costs 32 µs a frame (median; at most 85 µs). The whole cost is drawing the zigzag picture: tiny-skia blits a picture's whole extent on every frame it repaints, and the acceptance note's single picture spans every marked row, about 4 to 6 ms a full frame. Splitting it into one picture per row removed the frame cost but made opening worse, because a note switch makes every picture new and each new one costs about 1.5 ms (some 15 ms over the note's nine marked rows), in iced's tiny-skia SVG path or the repaints it causes; the two were not told apart. The picture's raster is already kept between frames (`spell-wire` item 29). An unchanged frame damages nothing on screen, so the frame cost lands only on frames that repaint the marked rows: a note switch, a scroll, or a change to the marks. The head keeps one picture a frame. Cutting the cost further means a different drawing path (§8, "a smooth wave"), or the owner accepting about 8 ms an open on the software renderer.

Gates after the fixes: `make check`; the X11 tiny-skia suite with `WRENTEST_SPELL=off` and with `on`, 252 passed and 1 skipped each; `test_spell.py` 11 passed on X11 and 8 passed, 3 deselected on Wayland, three runs; the macOS-target clippy.

*Verify session,* at `da90f24`: `make check` (2,642 Rust tests passed, 287 Python units); `test_spell.py` 11 passed on X11 and 8 passed, 3 deselected on Wayland, three runs, all on tiny-skia. It closed four findings on their own evidence: the lens's key-burst script lost no press (it had lost 12 of 12), the lens's closing-cause script opened the menu in 10 runs of 10 (it had failed 6 of 10), the FIFO test fails at once instead of blocking, and the flaky comparison passed some 75 Wayland runs. It found two new problems, both under "Not fixed" below.

**Fixed.** The chains' own fixes are in their subsections.

| Severity | What | Regression test | Found by | Commit |
|---|---|---|---|---|
| major | Within one batch of events, Return, Up or Down after a suggest press that opened nothing was lost, and Down past a short menu's last row followed by Return accepted nothing (rules 4.4.1, 4.4.4). With real X11 input after the fix, Alt+. then Return or Down on a correct word lost 0 of 12 presses, and Alt+. Down Down Down Return on `dosen’t` accepted `don’t` 8 of 8 times. | `widget::tests::keys_after_an_opening_that_finds_no_word_go_to_the_text`, `widget::tests::enter_after_an_opening_in_one_batch_accepts_the_row_the_host_selected` | text-safety lens | `938e869` |
| minor | A FIFO at `words.txt` or `config.conf` blocked the UI thread for good on the next re-read; the `config.conf` half predates the branch. | `settings::tests::a_fifo_or_a_directory_is_an_error_at_once` | Codex | `5e6a018` |
| minor | A menu opened just after a theme, text size or settings change could close again on the next event (rule 4.4.8). The harness case failed 6 runs of 6 on Wayland with the earlier widget. | `widget::tests::a_change_made_before_the_menu_opened_does_not_close_it`, `test_spell.py::test_a_menu_opened_just_after_a_theme_or_text_size_change_stays_open` | spec-ship lens | `bb4f4a9`, `cff1c6e` |
| minor | `test_spell.py::test_menu_keys_close_then_act_and_rows_follow_the_pointer_events` read the menu before a frame had drawn it, so its `bounds` could be `null` (once in 26 Wayland runs). | the test itself, now reading `drawn_menu` | spec-ship lens, carried from `spell-wire`'s verify | `83e4822` |

**Not fixed, by choice.** The end review has one fix round (§7.2 step 3); items 2 and 3 were found by the verify session after it.

1. *Major, budget 4.7.4* (spec-ship lens). Opening the acceptance note with the setting on is 10 to 19 % slower on tiny-skia (numbers below). The two cheaper ways to draw the zigzag were tried: one picture a frame (the head) carries a frame cost, and one picture a row removed it but made opening 26 % slower. Going further needs a different drawing path (§8, "a smooth wave"). The cost lands only on frames that repaint the marked rows (a note switch, a scroll, a change to the marks); an unchanged frame repaints nothing. Waits for the owner: accept about 8 ms an open on the software renderer, or ask for a new drawing path. *Later:* met on opening by `spell-endfix` E2 (`d6a220e`, `1fd734b`; §7.5 `spell-endfix`).
2. *Major, the rest of the batch fix* (verify session). The widget checks the suggest press against the caret as it was before the batch, so a key that moves the caret earlier in the same batch still misroutes, with real X11 input on tiny-skia, line `another teh line`: (a) caret at the end of `teh`, then Right, Alt+., Return in one burst: Return was lost 8 times of 10 (the widget saw `teh` and sent `AcceptSelected`, but the host opened nothing); (b) caret on `line`, then Left, Alt+., Return: a line break at the end of `teh` 6 times of 10 instead of accepting a suggestion (the widget saw no word and sent `Intent(Enter)`, which closed the host's new menu). Case (b) worked at `bfadffe`, so it is a regression of the fix. It needs keys sent within one frame; no byte is written beyond what an ordinary Enter writes, and undo restores it. The verify session's suggested fix: within a batch after an opening, send Enter, Up and Down as events the host resolves against its real menu (accept or move when one is open, otherwise apply the plain intent or action), or have the widget follow the caret through the actions it has already sent in the batch; with widget tests of [Right, suggest, Enter] and [Left, suggest, Enter] in one batch. Waits for the owner: a fix round, or accepting it. *Later:* fixed by `spell-endfix` E1 (`425aaa9`), with its review's fixes there and in `spell-endfix2`.
3. *Minor, a flaky regression case* (verify session). `test_spell.py::test_a_menu_opened_just_after_a_theme_or_text_size_change_stays_open` failed 8 of 95 runs on Wayland tiny-skia, always after `font.increase`: `drawn_menu` returned the previous iteration's bounds, still in the `SpellFrame` because no frame was drawn between that close and the new opening, while the next frames showed the menu open at its new size. The app is right; the test reads stale bounds. The fix is to wait for a drawn frame after each close and each change, or to compare the menus without `bounds`. It passed in every final lane. *Later:* fixed by `spell-endfix` E3 (`8db5203`).
4. *Minor, the word list is still re-read on the UI thread* (Codex's suggestion). §3.4 specifies the synchronous re-read, and with anything but a regular file of at most 1 MiB refused, the read is bounded.
5. *Rule 4.7.4's typing half is inconclusive*: typing 200 characters measured −6 % and +10.4 %, dominated by `xdotool`'s rate.
6. *Split suggestions* (`mi steak`, `Common Mark`; `spell-core` item 9) are kept as the dictionary offers them; the owner judges them in the hand trial.

**Gates.** Every lane with `WRENTEST_ENGINE=classic,advanced`, `WGPU_BACKEND=vulkan` and `LIBGL_ALWAYS_SOFTWARE=1`, one at a time. The final lanes ran as separate commands rather than the §7.2 loop, which would have stopped at the first failure.

| Run | Step 1, `bfadffe` | Final (step 4), `da90f24` |
|---|---|---|
| `make check` | Rust 2,634 passed, 0 failed, 12 ignored; Python 287 OK | Rust 2,642 passed, 0 failed; Python 287 OK |
| off, tiny-skia, X11 | 251 passed, 1 skipped | 252 passed, 1 skipped (twice) |
| off, tiny-skia, Wayland | 192 passed, 1 skipped, 59 deselected | **failed, three runs:** 136, 137 and 124 failed (57, 56 and 69 passed) |
| off, wgpu, X11 | 251 passed, 1 skipped | 252 passed, 1 skipped |
| off, wgpu, Wayland | 192 passed, 1 skipped, 59 deselected | 193 passed, 1 skipped, 59 deselected |
| on, tiny-skia, X11 | not run (step 1 is off only) | 252 passed, 1 skipped |
| on, tiny-skia, Wayland | not run | 193 passed, 1 skipped, 59 deselected (twice) |
| on, wgpu, X11 | not run | 252 passed, 1 skipped |
| on, wgpu, Wayland | not run | 193 passed, 1 skipped, 59 deselected |
| strict docs build | not run | "No issues found" |
| scenes (§5.3) | not run | 13 capture runs, 52 files |

The failed lane: its first failure is `test_editor.py::test_theme_changes_preserve_editor_state_and_noop_edits[advanced]`, and from there every launch exits with code 101 and `Create event loop: WaylandError(Connection(NoCompositor))`, because the headless Weston (13.0.0, the pixman renderer) has died: the kernel log shows Weston segfaulting in `libpixman-1` 0.42.2 once in each run. The load average was 1 to 2.5, so it is not a load timeout. The same Weston and pixman setup passed the `on` tiny-skia Wayland lane twice and both wgpu Wayland lanes, and this lane passed at `bfadffe`. Not established: whether Weston or pixman crashes on something only this lane happens to produce, or a change since `bfadffe` makes the tiny-skia client send what crashes it; the lane was not rerun at `bfadffe` to tell the two apart. Under §7.1 a failing gate is a blocker until shown otherwise. The results are kept in `target/wrentest-off-tiny-skia` (the failed third run) and the other `target/wrentest-<setting>-<renderer>` folders of the worktree.

**Budgets (§4.7),** in release, on X11. Rules 2 and 3 are the spec-ship lens's numbers at `bfadffe`; rule 4 is the lens's and then the fix session's, on tiny-skia only (wgpu was not measured).

| Rule | Measured | Verdict |
|---|---|---|
| 4.7.1 the dictionary loads off the UI thread | on a thread of its own (§3.4); the spec-ship lens recorded it met, and the Codex pass found no path that loads it on the UI thread | met |
| 4.7.2 marks for 60 rows of 100 characters, under 0.5 ms a frame | 0.095 to 0.176 ms for the proofreader; in the app, computing the acceptance note's marks takes 32 µs a frame (median; at most 85 µs) | met |
| 4.7.3 opening the menu, p95 under 100 ms | 33.7 ms over the nine marked words; 68.9 ms with long misspelled words (20 to 40 characters, `spell-core` item 11) added | met, so suggestions stay on the UI thread (open question 3) |
| 4.7.4 open the note and type 200 characters, within 10 % | opening: +9.8 to +12.6 % at `bfadffe` (the lens, four runs, about 6 to 9 ms); +14 to 19 % at `da90f24` (the fix session's rounds, 62.6 to 67.3 ms against 55.0 to 57.4 ms); typing: −6 % and +10.4 %, inconclusive | missed on opening; explained above. Met on opening after `spell-endfix` E2: −0.3 % and −5 % on tiny-skia, two runs (§7.5 `spell-endfix`) |

**Exit criteria (§6.3).**

1. *True.* `test_spell.py` passed in every final lane that completed, so steps 1 to 4 and 6 to 8 pass on X11 and Wayland and step 5 on X11, each on tiny-skia and wgpu. It sets its own setting, so the failed lane's runs of it add nothing the `on` tiny-skia Wayland lane does not cover. Not fixed item 3 is a flaky case in the same file.
2. *Not true.* `make check`, seven of the eight lane runs and the strict docs build pass; `off`, tiny-skia, Wayland failed three times.
3. *True.* `wren-editor` depends on neither `wren-spell` nor `wren-core`; `wren-spell` depends only on `spellbook` and `unicode-segmentation` (spec-ship lens).
4. *True.* The bundle holds SCOWL's `README_en_US.txt`, a `NOTICES` paragraph for SCOWL and for `spellbook`, and `spellbook`'s MPL-2.0 text; `spellbook` is the only package added to `Cargo.lock` (spec-ship lens; step 0's inventory).
5. *Measured and recorded:* rules 1 to 3 met, rule 4 missed on opening and explained; it waits for the owner.
6. *True.* The final gates captured the four scenes for all eleven themes on tiny-skia and for Wren Light and Wren Dark on wgpu (52 files under `target/scenes/advanced/<renderer>/<theme>/`). Looked at: Wren Light (tiny-skia `spell-menu`, wgpu `spell-marks`), Wren Dark (tiny-skia `spell-menu` and `spell-menu-empty`, wgpu `spell-menu-above`), Nord (`spell-marks`), Gruvbox Dark (`spell-menu-above`) and Solarized Light (`spell-menu`). `spell-marks` shows the acceptance note with eight zigzags in the theme's misspelling colour, the done task's `mispelled` under its strike among them, and none under `apear`, which holds the caret; nothing is marked in the front matter, the code span, the link targets, the autolink, the address, `<kbd>` or the fence. `spell-menu` shows five rows under `replacments` (replacements, replacement, emplacements, placements, replants), the first selected and the word tinted with its zigzag over the tint. `spell-menu-empty` shows one inactive "No suggestions" row under `xqzvkpw`. `spell-menu-above` shows `wrogn` on the last row, line 86, with its two rows (wrong, grown) above it. The two renderers differ only in antialiasing. Not opened at this head: Rosé Pine, Everforest Dark, Tokyo Night, Dracula, Catppuccin Mocha and Catppuccin Macchiato.
7. *Not true.* The failed lane is an open blocker until it is shown to be the compositor's, and two majors (not fixed items 1 and 2) have no owner decision recorded.

**What waits for the owner.** The tag `spell-p1` is not made: §6.3's criteria 2 and 7 do not hold.

1. A decision on budget 4.7.4: accept about 8 ms an open on tiny-skia, or a new drawing path for the zigzag.
2. A fix round for the rest of the batch fix (not fixed item 2, a regression in case (b)) and the flaky regression case (item 3), or accepting them.
3. The `off`, tiny-skia, Wayland lane: rerun it at `bfadffe` to tell a Weston crash from a regression, then the eight lanes again.
4. The Mac step of §7.2 step 5 (`make build fmt-check clippy test check-patches`, `make build-smoke`, `capture_scenes.py --only spell --engine advanced`, and a look at ⌘. and the menu on Metal), then the tag.
5. The push and CI (`ci-success`), and whether to open a pull request.
6. A hand trial: the zigzag and the menu by eye on both platforms, and the split suggestions (not fixed item 6).
7. The join with the boards branch (§6.4), with the 125 % capture of `spell-marks`.

**Finish run, 2026-11-14.** After `spell-endfix` and `spell-endfix2` merged (§7.5 below), a last run took the final gates of §7.2 step 4 once more at `d1cab2a`, with no review and no fix (its triage queue was empty). The gates changed no file. Every lane ran with `WRENTEST_ENGINE=classic,advanced`, `WGPU_BACKEND=vulkan` and `LIBGL_ALWAYS_SOFTWARE=1`, one at a time as separate commands, each keeping its results in `target/wrentest-<setting>-<renderer>-<display>`. Each passed on its first run, with the load average at 1.0 to 1.8; no Wayland lane lost its compositor (no `NoCompositor`), as at `8db5203` (§7.5 `spell-endfix`), so the Weston crash of run 1c did not come back, though it was never told apart from a regression fixed since. The `test_spell.py` counts are from each lane's JUnit file.

| Run | At `d1cab2a` | `test_spell.py` | Time |
|---|---|---|---|
| `make check` | Rust 2,656 passed, 0 failed, 12 ignored; Python 287 OK | | |
| off, tiny-skia, X11 | 252 passed, 1 skipped | 11 passed | 203 s |
| off, tiny-skia, Wayland | 193 passed, 1 skipped, 59 deselected | 8 passed | 98 s |
| off, wgpu, X11 | 252 passed, 1 skipped | 11 passed | 215 s |
| off, wgpu, Wayland | 193 passed, 1 skipped, 59 deselected | 8 passed | 144 s |
| on, tiny-skia, X11 | 252 passed, 1 skipped | 11 passed | 201 s |
| on, tiny-skia, Wayland | 193 passed, 1 skipped, 59 deselected | 8 passed | 98 s |
| on, wgpu, X11 | 252 passed, 1 skipped | 11 passed | 210 s |
| on, wgpu, Wayland | 193 passed, 1 skipped, 59 deselected | 8 passed | 141 s |
| strict docs build | "No issues found" | | |
| scenes (§5.3) | 13 capture runs, 52 files, no capture failure | | |

*Scenes looked at* (under `target/scenes/advanced/<renderer>/<theme>/`); with the end review's and `spell-endfix2`'s looks, every theme has now been opened at least once: tiny-skia Wren Light `spell-marks` (eight zigzags, `mispelled` among them under the done task's strike, none under `apear`, which holds the caret at line 6, column 12; nothing marked in `README`, `PRs`, `v0.1.1`, the code span, the link targets, the autolink, the address, `#tagg`, `snake_case_nmae` or the fence); tiny-skia Wren Dark `spell-menu` (five rows under `replacments`, the first, replacements, selected, the word tinted); wgpu Wren Light `spell-menu` (the same, differing from tiny-skia only in antialiasing); wgpu Wren Dark `spell-marks` (the same marks on the dark palette, `apear` unmarked); tiny-skia Rosé Pine `spell-menu` and Dracula `spell-menu` (five rows, the first selected); tiny-skia Everforest Dark `spell-marks` and Catppuccin Macchiato `spell-marks` (the marks, none under the caret's word); tiny-skia Tokyo Night `spell-menu-above` (`wrogn` on the last visible row, line 86, with wrong and grown above it, clear of the status bar); tiny-skia Catppuccin Mocha `spell-menu-empty` (one "No suggestions" row under `xqzvkpw`). Nothing looked wrong. The other 42 files were not opened.

*Budgets.* Not measured again: no commit since `1fd734b`, where `spell-endfix` measured rule 4.7.4, changes the widget's `draw`, the zigzag pictures or the marks (the later commits change event routing and tests), so the table above stands with the 4.7.4 row's note: rules 1 to 3 met, rule 4 met on opening on tiny-skia (−0.3 % and −5 %) with no regression on wgpu, and its typing half inconclusive.

*Exit criteria (§6.3), at `d1cab2a`.*

1. *True.* `test_spell.py` passed in all eight lanes, on tiny-skia and wgpu: its 11 cases on each X11 lane, step 5 among them, and on each Wayland lane its 8 cases without the native X11 ones, which cover steps 1 to 4 and 6 to 8. The flaky case of not fixed item 3 was fixed in `spell-endfix` E3.
2. *True.* `make check`, the four lanes with `WRENTEST_SPELL=off` and again with `on`, and the strict docs build pass, each first time.
3. *True.* `cargo tree --locked -e normal` at this head: the only Wren crate under `wren-editor` is `wren-markdown`; `wren-spell` depends on `spellbook` (with `hashbrown` and `foldhash`) and `unicode-segmentation` only.
4. *True.* Against `main`, `spellbook` and `wren-spell` are still the only packages `Cargo.lock` gains; `spell-endfix`'s vendored `iced_tiny_skia` changes an entry's source, not the package set. `tools/package.py`, `third_party/`, `NOTICES` and `test_package.py` are unchanged since run 1c's spec-ship lens found the bundle complete, and `test_package.py` passed in `make check`; the bundle was not rebuilt in this run. `tools/dependency_notices.py` at this head collects `spellbook`'s MPL-2.0 text and the vendored `iced_tiny_skia`'s MIT `LICENSE`.
5. *True.* Measured and recorded: rules 1 to 3 met, rule 4 met on opening; its typing half is inconclusive, dominated by `xdotool`'s rate (not fixed item 5).
6. *True.* All 52 scene files exist (eleven themes on tiny-skia, Wren Light and Wren Dark on wgpu), and the named ones were looked at (above).
7. *True.* No blocker is open: the failed lane passed at `8db5203` and in every lane here. No major is open: not fixed items 1 and 2 were closed by `spell-endfix` (E2, E1), the major its verify found by `spell-endfix2` (E4, `9712c79`), and `spell-endfix2`'s review and verify found none. What stays open is minor or a recorded limit, each with its reason in its subsection and in §8: the Escape corner after an opening that found nothing (`spell-endfix` item 3), the stale gutter repaint on tiny-skia, which is iced's (`spell-endfix` item 12), input in the frame of an Escape (`spell-endfix2` item 4), a press in the frame of an opening (`spell-endfix2` item 6), and not fixed items 4 to 6.

*What waits for the owner,* replacing the list above, whose items 1 to 3 are settled (the budget and the fix round by `spell-endfix`; the lane passes, though it was never rerun at `bfadffe`):

1. The Mac step of §7.2 step 5 (`make build fmt-check clippy test check-patches`, `make build-smoke`, `capture_scenes.py --only spell --engine advanced`, and a look at ⌘. and the menu on Metal), then the tag `spell-p1`. `check-patches` now covers the vendored `iced_tiny_skia` too.
2. The push and CI (`ci-success`), and whether to open a pull request.
3. Keeping the vendored `iced_tiny_skia` (`spell-endfix` item 7): a fourth patched crate, outside §6.1's plan for the root files, that the iced upgrade has to carry or drop.
4. The recorded limits: accept them or ask for work. Input in the frame of an Escape, case (b), can only be fixed by changing rule 4.4.8's design (the editor's own Escape no longer going through the app's `close_spell_menu()` first), which is the owner's call (`spell-endfix2` item 4).
5. A hand trial: the zigzag and the menu by eye on both platforms, and the split suggestions (not fixed item 6).
6. The join with the boards branch (§6.4), with the 125 % capture of `spell-marks`. Since §6.4 was written, `spell` also changes `vendor/` and the root `[patch.crates-io]`, so the join keeps the `iced_tiny_skia` patch with boards' Cargo changes.

#### `spell-endfix`

An extra fix chain, added after run 1c stopped short of the tag, for the end review's not fixed items 2 (a major, and in case (b) a regression of `938e869`) and 3 (a flaky harness case), in that order (E1, E3), then item 1 (budget 4.7.4, a major; E2). Deviations:

1. *`SpellEvent` gains `Key(MenuKey)`, with `MenuKey { Up, Down, Enter }`* (§3.2, §3.3). The host opens a menu against the caret and the text as the batch's earlier messages leave them, which the widget does not have, so after an opening in the batch the widget no longer guesses what opened: its copy is *opening* until the next batch, and Up, Down and Enter with no modifier publish `Key`, which `EditorView::spell` resolves against its real menu (`MenuKey::event`: `Move(-1)`, `Move(1)` or `AcceptSelected`) or, with none open, handles as the editor handles that press with no menu (the editor's own key routing, carried out in `EditorView`). Enter leaves no menu either way, so the keys after it are the text's. The widget's caret check of `938e869` (end review item 1) is removed. A menu open when the batch starts is the host's own, so its keys still publish `Move` and `AcceptSelected`.
2. *A right-click opening counts as an opening too*, not only the suggest key: the widget finds its word in the last layout, which a scroll or an edit earlier in the same batch makes stale in the same way. So `widget::tests::keys_after_an_opening_in_one_batch_go_to_the_menu_it_opens` (`spell-menu` item 18) now expects `Key` events.
3. *What an opening leaves to the other keys is unchanged:* until the next batch every other key takes it as an open menu, so Escape is the host's, a lone modifier does nothing, and any other press closes it first (a `Close` with no menu open changes nothing). One corner remains, needing an Escape within one frame of a suggest press that found no word, as at `bfadffe`: Escape then leaves the editor focused (the host still gets it). This item first named two more, both fixed after the review (items 10 and 11): a press inside the editor after such an opening only closed, so a left click was lost, which was as at `bfadffe` but a regression from `75de786`, whose caret check had let the click through; and a plain Up, Down or Enter that a host's key policy gives to the host was captured and then dropped.
4. *The two tests of `938e869` are kept,* with their inputs and their assertions on the text and the menu; the first one's message pattern now names `Key(Enter)` for the Enter after the opening.
5. *The flaky case waits for a frame after each close, not after each change.* A frame without the menu resets the drawn bounds, so `drawn_menu` after the next opening waits for a frame that drew the new menu. A frame after the change would let the widget see the change in an event before the menu opens, which is the case the test guards (`bb4f4a9`).

6. *Where budget 4.7.4's time went (E2).* A throwaway copy of `iced_tiny_skia` timing each draw, each damage group and each SVG raster and blit showed that the frame switching to the acceptance note repainted 15 damage groups with the setting off and 16 with it on. The extra group was the picture's own rectangle (496 by 415 px, 3.2 ms): `damage::group` merges only neighbours in its order, by distance from the window's corner, and another region sorted between the picture and the text's. The picture (204,022 px) was also blitted whole in each of the four groups it touched, about 0.6 ms each, and rasterized again (0.25 ms) because the note without marks had dropped it from the renderer's cache; a full frame, such as the harness's screenshot, blits it once more (0.7 ms). Every group cost at least 0.7 ms however small, since each one draws every layer again. Splitting the picture by word or by row made the blits tiny but gave each rectangle a group of its own (23 and 22 groups), so no way of drawing in the editor avoids a group per separate region on iced's own grouping.
7. *Wren vendors `iced_tiny_skia` 0.14.1* (`vendor/iced_tiny_skia`, with a `WREN.md`) and, after `damage::group`, merges any two groups whose union wastes no more than its own threshold, in any order. The groups still cover every damaged pixel and a group is repainted from the background up, so the screen is unchanged; wgpu does not use it. This touches the root `Cargo.toml` (the patch and the workspace's `exclude`), `Cargo.lock` (the entry loses its registry source and checksum; no package is added), the `Makefile` (`check-patches` asserts the patch applies) and `CLAUDE.md`, which §6.1 kept to step 0 for Stage A: the cost is in the renderer, and no drawing in the editor met the budget (the table below). Every tiny-skia frame now repaints fewer groups, so opening the note with the setting off also went from about 56 ms to 42 ms.
8. *Each word's zigzag on a row is its own picture* (§3.3, rule 4.3.5), keyed by what it draws: the zigzag's width, the part of it the rows' clip leaves, and the colour. Words of one width share a picture; a picture is kept while frames draw it and dropped with the first frame that does not, so an unchanged frame repeats the same handles at the same bounds, and a note switch reuses the pictures of widths already drawn. The squares are rule 4.3.4's, unchanged. The tests of `spell-wire` item 29 become `widget::tests::each_zigzag_is_a_picture_of_its_squares_on_the_pixel_grid` and `widget::tests::zigzag_pictures_are_shared_by_what_they_draw_and_kept_while_drawn`. At a fractional scale each picture's origin is now rounded on its own, so the join's 125 % capture (§6.4) still applies.
9. *Pixels.* The 52 spell scenes, recaptured, match the end review's pixel for pixel except for the header's temporary path (a 52 by 10 px box at the same place in every file); the zigzags, the status bar and the caret are identical on both renderers. Because scenes are full redraws, a throwaway native X11 case also compared the window's real pixels with a full redraw after each of three note switches, a scroll, a toggle off and on, on tiny-skia: with the setting on they differed only in 184 pixels of the "Move to…" button's outline, by at most 2 of 255. At `b12e6c6` the window's own pixels after a switch lacked the gutter numbers 12 to 22 and parts of two button outlines, with the setting on or off, so that predates spell check and comes from iced's damage. This item first said the setting off matched exactly at this chain's head; the review found otherwise (item 12): the merge makes these stale repaints rarer, but does not remove them.

- E1: `425aaa9`, with `widget::tests::a_suggest_press_after_a_caret_move_in_one_batch_opens_where_the_caret_went` ([Right, suggest, Enter] and [Left, suggest, Enter] in one batch on `another teh line`, then Down before Enter, and Down on a word the opening did not find) and `keys::tests::a_menu_opened_earlier_in_the_batch_sends_its_keys_to_the_host`. At `75de786` the first two cases left `another teh line` (Return lost) and `another teh\n line` (a break instead of accepting). Real X11 input, tiny-skia, with `xdotool --delay 0`, 12 runs a case: the verify session's script gave Right, Alt+., Return a break before `line` 12 of 12 (it had lost Return 8 of 10), Left, Alt+., Return `another the line` 12 of 12 (it had broken the line 6 of 10), Left, Alt+., Down, Return the second suggestion 12 of 12, Right, Alt+., Down, Return a move to the next line and a break there 12 of 12, and Right, Alt+., Up a move to the line above 12 of 12; the text-safety lens's script accepted `don’t` after Alt+. Down Down Down Return 12 of 12, in one burst and with a pause after Alt+., and lost 0 of 12 presses in each of its eight cases.
- E3: `8db5203`. 100 runs of the case on Wayland tiny-skia and 30 on X11 tiny-skia, all passed; the case as it was at `75de786` failed 1 of 100 Wayland runs in the same hour.
- E2: `d6a220e` (the vendored renderer), `1fd734b` (the pictures), with the two widget tests of item 8. Budget 4.7.4, in release under X11 with a smoke build in its own target directory, `vmstat 1 5` 92 to 99 % idle before each run, by the end review's method: per round, the median of 20 `open_note` calls each followed by an `app.screenshot` frame, the two settings in the same run. Each cell is the range of the rounds' medians, three rounds unless noted; the last column compares the run medians. The variants marked "switch" were built from `b12e6c6` with throwaway switches for the drawing and the merging, beside the instrumented renderer (which only measures when told to).

| Build | Merged groups | Open, on | Open, off | On against off |
|---|---|---|---|---|
| `b12e6c6`, one picture a frame | no | 61.9 to 66.6 ms; 63.9 to 66.0 ms (four rounds) | 54.6 to 60.6 ms; 55.5 to 56.8 ms | +14 %; +15 % |
| Switch: marks computed, not drawn | no | 57.1 to 60.0 ms | as the first row | +2 % |
| Switch: one picture per word | no | 73.5 to 74.7 ms | as the first row | +31 % |
| Switch: one picture per row, the text's width | no | 71.1 to 71.5 ms | as the first row | +27 % |
| Switch: one picture a frame | yes | 45.2 to 47.6 ms | 41.5 to 43.5 ms | +14 % |
| Switch: one picture per word | yes | 38.9 to 43.5 ms | 41.5 to 43.5 ms | +2 % |
| `1fd734b` | yes | 37.5 to 42.8 ms; 36.0 to 44.1 ms (four rounds) | 42.0 to 43.0 ms; 41.2 to 43.4 ms | −0.3 %; −5 % |

  A screenshot of the unchanged frame took 29.6 to 31.7 and 28.4 to 28.9 ms on against 29.6 to 31.6 and 27.9 to 29.6 ms off at `1fd734b` (34.4 to 37.7 ms on, 25.5 to 30.4 ms off at `b12e6c6`). wgpu, once (Vulkan, software): `b12e6c6` opened in 43.2 to 45.5 ms on and 39.7 to 44.4 ms off, `1fd734b` in 40.5 to 42.8 ms on and 38.9 to 43.1 ms off, so no regression. With the setting on, `test_spell.py::test_a_screen_full_of_marks_draws_without_a_stall` took 1.0 s and the large-note test 2.0 s (advanced) in the X11 lane. Typing (the budget's other half) was not measured again and stays inconclusive.
- Gates at `1fd734b`: `make check` at each commit (Rust 2,646 passed, 0 failed; Python 287 OK); `test_spell.py` 11 passed on X11 and 8 passed, 3 deselected on Wayland; the X11 tiny-skia suite with `WRENTEST_ENGINE=classic,advanced`, 252 passed and 1 skipped with `WRENTEST_SPELL=off` and again with `on`; the spell scenes (52 files); the strict docs build. No file of the macOS clippy gate changed.
- Gates at `8db5203`: `make check` (Rust 2,646 passed, 0 failed; Python 287 OK); `test_spell.py` 11 passed on X11 and 8 passed, 3 deselected on Wayland; the X11 tiny-skia suite with `WRENTEST_ENGINE=classic,advanced`, 252 passed and 1 skipped with `WRENTEST_SPELL=off` and again with `on`; and, beyond the chain gates, the `off` tiny-skia Wayland lane, 193 passed, 1 skipped, 59 deselected, with no Weston crash in the kernel log. No file of the macOS clippy gate and nothing under `docs/` changed.

*The review's fix step* reproduced each finding before fixing it. Deviations:

10. *`SpellEvent` gains `Press(MenuPress)`, with `MenuPress { Left(Point, mouse::click::Kind), Right(Point) }`* (§3.2, §3.3, §3.4, rule 4.4.4). A left or right press inside the editor after an opening in the same batch is the pointer's counterpart of `Key`: the widget publishes it in place of a bare `Close`, and `EditorView::spell` resolves it against its real menu. With a menu open, the press only closes it (rule 4.4.5), and a left press also makes `EditorView` drop the `Drag` actions of that press, so the drag selects nothing; with none open, a left press is the click (or double or triple click) it is with no menu, and a right press is `OpenAt`. A press of another button, or with a menu open when the batch starts, still only closes. `App::edit` makes the editor the active column for `Press`. Rule 4.4.4's sentence about the pointer now holds only over a menu open when the batch starts; after an opening in the batch, presses and keys reach the widget in their order.
11. *A key the host's policy takes is left to the host after an opening* (§3.3, rule 4.4.4): a plain Up, Down or Enter that the key policy routes to `KeyRoute::Host` closes the opening and goes to the host uncaptured, instead of being published as `Key`. With a menu open when the batch starts, the menu still takes those three keys, as before. Wren's own policy cannot hit this: `key_policy` asks the keymap only about presses that `keys::shortcut` makes a trigger, which needs Ctrl, Alt, Super or a function key, so no plain arrow or Enter is ever the host's. The fix guards other hosts of the editor.
12. *Item 9 corrected; the stale repaint is not fixed.* The review's switch script, run again in the fix step with the setting off, left the gutter wrong after 3 of 12 note switches (about 1,035 px at x 650 to 665 each, the numbers of a 200-line note missing after switching to it or left behind after switching away), the same count the review measured at `2c00196`. It is iced's damage on tiny-skia, older than spell check and rarer than at `75de786` (12 of 12 with the setting on and 10 of 12 off in the review's run); §8 lists it.

- Review of the chain at `2c00196`: 0 blockers, 1 major, 2 minors.
- Fixed: the major in `0fb6a37` (`widget::tests::a_press_after_an_opening_in_one_batch_clicks_unless_a_menu_opened`, and the `Press` rows of `proof::tests::the_widgets_copy_follows_each_message_as_the_hosts_menu_will`); the host-key minor in `a0ebfa8` (`keys::tests::a_key_the_host_takes_is_left_to_it_after_an_opening_in_the_batch`); the record minor in this commit (items 3, 9 and 12, and §8). With real X11 input on tiny-skia, 12 runs a case, Alt+. on a correctly spelled word then a left click on `teh` moved the caret 12 of 12 (8 of 12 lost at `2c00196`), Alt+. on `teh` then a click only closed the menu 12 of 12, and Alt+. on a correct word then a right-click on `teh` opened its menu 12 of 12; the review's key-burst script still matched in all nine of its cases, 12 of 12 each.
- Not fixed: the stale gutter repaint of item 12, which is iced's renderer and not spell check's, and is only recorded; the Escape corner of item 3, which has no visible effect.
- Gates at `a0ebfa8`: `make check` at each commit (Rust 2,650 passed, 0 failed; Python 287 OK); `test_spell.py` 11 passed on X11 and 8 passed, 3 deselected on Wayland; the X11 tiny-skia suite with `WRENTEST_ENGINE=classic,advanced`, 252 passed and 1 skipped with `WRENTEST_SPELL=off` and again with `on`; the 52 spell scenes, pixel-identical to the review's captures except the header's temporary path. No file of the macOS clippy gate and nothing under `docs/` changed.
- Verify at `25cf0e9`: the three findings are closed, each re-tested from its evidence (the lost click 11 of 12 at `2c00196`, 0 of 12 at `25cf0e9`), and the gates pass (`make check`, Rust 2,650 passed, 0 failed, Python 287 OK; `test_spell.py` 11 passed on X11, 8 passed and 3 deselected on Wayland; the X11 tiny-skia suite with both engines, 252 passed and 1 skipped with the setting off and on, each a first run; the 52 spell scenes, identical except the header's temporary path). It found 0 blockers and 1 new major, introduced by `0fb6a37`.
- Not fixed, the verify's major: a left press that only closes a menu opened earlier in its batch (rule 4.4.5) is still recorded as the widget's last click (`Update::Press` sets `state.last_click`), so a second left press at the same point within the double-click interval is a double click and selects the word at the old caret, which the first press never moved. Real X11 input on tiny-skia, the caret at the end of `teh` on `another teh line`, then Alt+. and two clicks 40 ms apart on the line above in one burst: `teh line` was selected 9 of 12 times at `25cf0e9`; the caret moved to the click 12 of 12 at `2c00196`, and at `25cf0e9` with the menu opened in an earlier batch. It is not fixed because the chain ran its one round and §7.1 gives a second only for a blocker; it is carried into the end review. The likely fix: a press that `EditorView` resolves as close-only leaves the widget's next press with no previous click, with a widget test of [suggest on `teh`, left press] in one batch and a left press at the same point in the next batch giving `Click`, not `SelectWord`. Fixed in `spell-endfix2`.

#### `spell-endfix2`

A second extra fix chain, added after `spell-endfix`'s verify found a major regression in that chain's own fix `0fb6a37` (its last bullet), for that major (E4), with a native X11 burst fuzz before the commit. Deviations:

1. *A left press is counted from what the press before it was taken as* (§3.2, §3.3, rule 4.4.5). iced counts double and triple clicks by time and place alone, so a press that only closed a menu was still the first half of a double click. The widget now keeps, beside iced's last click, what the last left press was taken as (`None` when it only closed a menu), and `proof::click_kind` gives each press its kind: a single click after a press that only closed, otherwise iced's count from the press before. A press the widget leaves to the host (`SpellEvent::Press`) is taken as a click until the host's verdict reaches it: `EditorView` hands the widget a `proof::ResolvedPress` (how many presses it has resolved, and what the last one was) with the menu, and the widget takes it at its next event. Within the batch the widget cannot know the verdict, so every left press after one it left to the host is a `Press` too, carrying the kind it makes if the earlier ones clicked, and `EditorView` counts it again from its own verdicts while one of them only closed a menu. A press that closes a menu open when the batch starts (`Update::Dismiss`, now carrying its button) resets the count in the widget. `MenuPress::Left` keeps its shape; its kind now means the above.
2. *The host drops the drag of any press it does not take as a single click* (`press_closed_menu` becomes `drop_drags`). The widget lets every `Press` drag, so the press after one that only closed drags a selection as a single click does, though the widget guessed a double click.
3. *The overlay takes only moves over the menu and presses on it* (§3.3, rule 4.4.4). The fuzz found this: iced hands the overlay the whole batch before the widget sees its keys, so a press after a key that had closed the menu (an arrow, Escape, a suggest press that found nothing) was still taken as a press on an open menu and only closed it. A double click on `and` and a right-click on `wrold`, then Alt+. (the caret on `and`, so nothing opens), Right, a click at the line's start and Up, sent in one burst, lost the click 2 of 4 times, whenever the burst split after the right-click. A press off the menu now goes to the widget, which closes the menu for it in its order with the keys (its `Dismiss`, which also counts the press out of the next double click). `MenuOverlay::editor` is gone. Presses on the menu and moves over it still come before the batch's keys.
4. *Not fixed: input in the frame of an Escape.* All ten differences of the final fuzz runs have Escape followed by more input in one burst, and in each the burst and the paused run agreed once both waited 0.35 s after each Escape. Two causes, neither in the routing this chain fixes:
   - (a) With no menu open, the editor drops the keyboard on Escape and the app gives it back with a focus task after the frame, so the keys of that frame are lost. With spell check off, a click, Escape and Return in one burst lost Return 8 of 8 times, and 0 of 8 with a 0.5 s wait after Escape. This predates spell check; the paused run's pause after Alt+. only moves keys out of that frame or into it.
   - (b) The app closes the menu on Escape from `key_input`, which iced runs from a subscription after the batch's widget messages, sometimes after the next batch's. An Escape in the batch of an opening therefore closes a menu that a later suggest press of that batch opened (seed 12, case 1). It can also leave the menu open for the next frame's press, which then only closes it (seed 11, case 2, 1 of 4 runs before the last). After a suggest press that found nothing, it leaves the editor focused (`spell-endfix` item 3), so the keys after it reach the text where the paused run loses them to (a) (seed 13, case 52; seed 14, case 9). Fixing (b) means the editor's own Escape no longer goes through the app's `close_spell_menu()` first, which is rule 4.4.8's design and the owner's to change; each case needs Escape and more input within about one frame. §8 lists both.
5. *The fuzz's own conditions.* A left press in the batch of a typed character hits the layout from before that character, on both engines and with spell check off (a typed `1` then a click on the next word put the caret at the end of the line above 11 of 12 times; with a 0.3 s wait, 8 of 8 were right). That predates spell check and is not this chain's, so the fuzz waits 0.35 s after a typed `1` or a Return whenever a press follows, in both runs. The wait is longer than iced's 300 ms double-click window, because a 0.2 s wait let a triple click chain with the click before it in some runs of both kinds (seed 11, case 37, in the run before the last).

- E4: `9712c79`, with `widget::tests::a_press_that_only_closed_the_menu_starts_no_double_click` ([suggest on `teh`, left press] in one batch, then a left press at the same point in the next giving `Action::Click`, and the press after that `SelectWord`; the same three presses in one batch; a triple click after the closing press; an opening that found nothing leaving a double click; the press after a closing one dragging; and a press that closed a menu open at the batch's start, right after a click at that point, then a click) and `proof::tests::a_press_after_one_that_only_closed_a_menu_is_a_single_click`. With `click_kind` made to return iced's count, the widget test fails at its first assertion on the next batch's message. The item-3 fix is `74d840f`, with `widget::tests::a_press_off_an_open_menu_comes_in_its_order_with_the_keys` (the overlay first, then the widget, as iced hands over a batch) and `proof::menu::tests::a_press_off_the_menu_is_left_to_the_editor` (formerly `a_press_elsewhere_closes_the_menu_and_only_reaches_beyond_the_editor`). The tests of `938e869`, `425aaa9`, `0fb6a37` and `a0ebfa8` pass unchanged.
- The verify's case D, real X11 input on tiny-skia with the caret at the end of `teh`: Alt+., a click on the line above, 40 ms, a second click there, left the caret at the click with nothing selected 12 of 12 times (`teh line` was selected 9 of 12 at `25cf0e9`); with the two clicks in one burst, also 12 of 12; with a 0.5 s pause after Alt+., 12 of 12 each.
- The fuzz, a throwaway script (not committed): the note `it is on to me as we go` / `an teh word wrold and recieve tihs end` / `plain closing text` / `last`, whose first line holds only two-letter words, so no edit makes a misspelling there and every menu opens below every pointer target. Each burst starts at one of seven caret positions and holds two to six of: Alt+. (at least one), Up, Down, Return, Escape, Left, Right, a typed `1` (which never makes a misspelling), and single, double and triple left clicks and right-clicks on a random column of a random word of the first two lines (four marked, the rest not). One `xdotool` call sends it with `--delay 0`; the reference sends it again with `sleep 0.5` after each Alt+.. After each, 0.7 s later, it compares the caret, the selection's anchor, the text and the menu (word, line, start, selected row), and reruns a differing case four times each way, once more with a 0.3 s wait after each key that follows an Alt+., and twice each way with a 0.35 s wait after each Escape. It checks first that a click on each target word puts the caret there. On the tree of `74d840f`, tiny-skia, X11:

| Seed | Bursts | Differ | Cases, by item 4's causes |
|---|---|---|---|
| 11 | 60 | 0 | |
| 12 | 60 | 2 | 1 (b), 31 (a) |
| 13 | 60 | 5 | 13 (b, then a), 23 (a), 38 (a), 52 (b, then a), 55 (a) |
| 14 | 60 | 1 | 9 (b, then a) |
| 15 | 60 | 0 | |
| 16 | 60 | 2 | 10 (a), 36 (a) |

  Seed 11 was also run three times before that. With only `9712c79` it found 3 differences: two were the overlay taking a press ahead of the batch's keys in the paused run (cases 2 and 19, which item 3 removed) and one was the stale layout of item 5 (case 28). With the edit wait it then found case 2 again and case 42, which is item 3. With item 3, before the wait was lengthened, it found cases 2 (b) and 37 (item 5's double-click window).
- Gates at `74d840f`: `make check` at each commit (Rust 2,654 passed at `9712c79` and 2,656 at `74d840f`, 0 failed; Python 287 OK); `test_spell.py` 11 passed on X11 and 8 passed, 3 deselected on Wayland; the X11 tiny-skia suite with `WRENTEST_ENGINE=classic,advanced`, 252 passed and 1 skipped with `WRENTEST_SPELL=on` and again with `off`; the 44 tiny-skia spell scenes of the eleven themes, identical to the `spell-endfix` verify's captures except the header's temporary path (a 52 by 10 px box at the same place in every file). No file of the macOS clippy gate and nothing under `docs/` changed.

*The review's fix step* reproduced each finding before fixing it. Deviations:

6. *A press in the batch of an opening that opened a menu only closes it, wherever it lands* (rule 4.4.4, §3.3, §8). Rule 4.4.4 said such a press acts as rule 5 says, and rule 5 accepts a click on a row, while §3.3 said the host resolves it as a close. The widget has no overlay until the next frame lays the menu out, so it cannot tell a press where a row will be from one elsewhere, and the host has no layout to hit-test against; making the widget take the opening's verdict before the press would move the opening into the widget, which §3.3 rules out. The rule now says the press closes the menu and does nothing more, as a press elsewhere does, and §8 lists it. No byte is written wrongly: the press closes, and the click after a pause accepts. Reproduced with the review's probe, real X11 input on tiny-skia, the caret at `(1, 6)` after `teh`, Alt+. and a left click on line 2 column 5, where the menu's first row is drawn: in one burst the text was unchanged and the menu closed 6 of 6 times; with a 0.5 s pause after Alt+., `teh` became `the` 6 of 6. `widget::tests::a_press_after_an_opening_in_one_batch_clicks_unless_a_menu_opened` already pins the close (the widget sees the same `Press` wherever the press lands), so no test was added.

- Review of the chain at `001c13b`: 0 blockers, 0 majors, 3 minors.
- Fixed: the contradiction between rule 4.4.4 and §3.3 about a press in the batch of an opening (minor 1) in `dbf3d47`, by stating what happens (item 6), with no code change; the untested drop of a double click's drag after an opening that found nothing (minor 2) in `ce622a8`, a case of `widget::tests::a_press_that_only_closed_the_menu_starts_no_double_click` ([suggest on `cat`, left press, left press, a move to column 5] in one batch keeps `cat` selected), which fails with `drop_drags = kind.is_none()` (it selects `cat teh`); the scene gate (minor 3) in this commit, whose gates below capture the wgpu pair too. The gates bullet at `74d840f` keeps its 44 files, which is what that run captured: it left out the wgpu pair of §5.3 without saying why.
- Not fixed: nothing. A click in the batch of an opening still closes the menu where a click after a pause accepts its row (item 6, §8): the menu is not laid out until the next frame, and resolving the opening in the widget is ruled out by §3.3.
- Gates at `dbf3d47`: `make check` at each commit (Rust 2,656 passed, 0 failed; Python 287 OK); `test_spell.py` 11 passed on X11 and 8 passed, 3 deselected on Wayland; the X11 tiny-skia suite with `WRENTEST_ENGINE=classic,advanced`, 252 passed and 1 skipped with `WRENTEST_SPELL=on` and again with `off`; the 52 spell scenes of §5.3 (the eleven themes on tiny-skia, Wren Light and Wren Dark on wgpu), identical to the review's captures except the header's temporary path. Looked at: wgpu Wren Light `spell-menu` (five rows under `replacments`, the first selected, the word tinted), wgpu Wren Dark `spell-marks` (eight zigzags, none under `apear`, which holds the caret), tiny-skia Wren Light `spell-marks`, tiny-skia Wren Dark `spell-menu-above` (wrong and grown above `wrogn` on line 86), tiny-skia Nord `spell-menu-empty` (one "No suggestions" row under `xqzvkpw`) and tiny-skia Gruvbox Dark `spell-menu`; the two renderers differ only in antialiasing. No file of the macOS clippy gate and nothing under `docs/` changed.
- Verify at `aeb67a1`: the three minors are closed, each re-tested from its evidence (the review's probe again gave a close in one burst and an accept after a pause, 4 of 4 each; with `drop_drags = kind.is_none()` the new case fails and passes once reverted), and the gates pass (`make check`, Rust 2,656 passed, 0 failed, Python 287 OK; `test_spell.py` 11 passed on X11, 8 passed and 3 deselected on Wayland; the X11 tiny-skia suite with both engines, 252 passed and 1 skipped with the setting on and off; the 52 spell scenes, identical to the review's except the header's temporary path). It found no new finding.

#### The join

The rehearsal of §6.4, 2026-11-15, adapted by the orchestrator: `spell` was already on `main` (merge `4446483`), so the throwaway branch `spell-join` was cut from that `main`, not from `spell`, and merged the head of `boards`, `dc66cbd`, with `git merge --no-ff`. Nothing of `boards` was changed. The branch holds the merge `8fcf537`, four commits after it, and this record; it was not pushed.

*Conflicts* (twenty files, each resolved by keeping both sides):
- `Cargo.toml`: the eight members in one list, `vendor/iced_tiny_skia` kept in `exclude`; the `[patch.crates-io]` entry for it merged cleanly beside boards' `canvas` feature. `Cargo.lock` merged without a conflict, but by the rule it was taken from `main` and `cargo check` (without `--locked`) added boards' packages: `lyon` and its four crates, `float_next_after`, and `lyon_path`/`lyon` as dependencies of `iced_graphics` and `iced_wgpu`, 80 lines and nothing else. The vendored `iced_tiny_skia` builds with `geometry` (`cargo tree -e features` shows `iced_renderer`'s `geometry` turning it on).
- `Makefile`: the three per-crate clippy lines. `CLAUDE.md`: `spelling.rs` and boards' five `app/` modules in one sentence, then boards' `board_export.rs` line. `discovery/README.md`: both rows.
- `crates/wren-core/src/settings.rs`: both fields, defaults, parse arms, template paragraphs (spell check first) and tests.
- `crates/wren-iced/src/app.rs`: boards' `BoardProbed` message and arm beside spell's `EditorShortcut`, which keeps its `KeyRoute`; both launch locals (`words_path`, boards' `launched` and `board_spike`).
- `app/commands.rs`: both sets of `CommandId`s and `run_command` arms; the rule test keeps spell's name and boards' `definitions()`.
- `app/preferences.rs`: main's `settings_warning` (config.conf and words.txt) and boards' restart notice, joined in a new `shown_settings_warning`, which `show_settings_warning` shows and `settings_warning_changed` compares, so the notice is retracted like a warning in both paths.
- `macos_menu.rs`: boards' version, which builds the bar from the table in `menu.rs`; spell's "Spelling Word List…" row moved into that table after "Settings…".
- `theme.rs`: both tests.
- Docs: `architecture.md` (the spell row after the editor's, then boards' two and boards' `wren-iced` row), `testing.md` (both sections; the spell scenes paragraph then boards'), `keybindings.md` (one command list with both rows, spell's wording of the no-default-key paragraph and boards' pointer to "On a board"), `configuration.md` (both rows, boards' `keybind` row). `zensical.toml` merged cleanly with both pages.
- `tools/package.py` and `tools/wrentest_unit/test_package.py`: both notices, both licence paths, both tests.
- `tools/capture_scenes.py`: `--only` takes `markdown`, `spell` or `boards`; both docstring paragraphs, both checks of the arguments, both blocks inside boards' `scene_workspaces()`.
- `tools/wrentest/ui.py`, `tools/wrentest_unit/test_ui.py` and `tools/wrentest/test_editor.py`: point 2 below.

*Conflicts git did not see:* `keybind.rs`'s test of the suggest key called `Keymap::build` with boards' new layer argument wrongly (`base` now), and boards' test that the base layer equals 0.1.1's defaults failed on the suggest key, so its frozen list gained that one binding, with its doc comment saying so. In the first X11 lane, `test_board_gate.py::test_with_no_settings_file_boards_are_on` failed: spell's harness writes `config.conf` for every launch (§5.1), and that case asserts a fresh install has none (deviation 2).

*The three points of §6.4:*
1. *Text only*, `4adadb2`. With a board open, `ensure_editor_ready` already withheld the proofreader; a board that does not parse (`Surface::Raw`) did not, and showed marks. `wanted_proofreader` now also asks for `Surface::Text`; `apply_markdown_assist`, where the surface already gates Markdown assist, now syncs the proofreader too, so the menu closes on the surface change itself, not only after the next message; `editor.spell_suggest` is enabled only on `Surface::Text`. Tests: `app::spelling::tests::spell_check_is_for_markdown_text_and_never_a_board_or_its_raw_text` (a menu open on Markdown closes when a board opens; on raw text no proofreader, no menu, the command disabled and blocked; back on Markdown it opens again; without the surface check it fails at the raw text) and the harness case `test_spell.py::test_a_board_shown_as_raw_text_reports_no_marks` (a raw board with a misspelled heading and a typed misspelling: no marks after a drawn frame, no menu, the command disabled, marks back on the Markdown note; without the check it failed with `mispelled` and `wrod` marked). `docs/reference/spell-check.md` says boards are not checked.
2. *The harness setting*, in the merge. `WRENTEST_SPELL` goes through `_spell_setting(settings)`, the shape of boards' `_boards_setting`, and both append through one `_with_setting(settings, key, value)` that leaves alone a text naming the key (a line-anchored match, as boards' was); `WrenApp` applies spell first, then boards. Boards' edit of `test_editor.py`'s exact-equality check (it compares with the file as written and checks the engine line) replaced ours, and the focus-reload case now rewrites only the engine line of the file as written, so both lane lines stay. Boards' `test_ui.py` cases expect the spell line too, with a new case of both lines. `98aec74` adds `WrenApp(lane_settings=False)` for boards' fresh-install case.
3. *Escape*, `d4627cf`. Boards' ladder is in the board widget (`board_policy` keeps Escape for it, any modifiers); in `preferences.rs` `key_input` still calls `close_spell_menu()` before any other Escape handling, and so does the `Message::Escape` arm, so no code changed. Test: `app::spelling::tests::one_escape_on_a_text_surface_beside_boards_closes_only_the_menu` (boards on, a Markdown note, the find bar and the menu open: one Escape closes the menu only, keeps caret, text and column, returns no focus task; the next closes the find bar).

Deviations:
1. *`make check-patches` asserts `iced_winit` too* (`557fdbe`). It asserted three of the four vendored crates on both branches; the task asks that every patch be asserted.
2. *`WrenApp(lane_settings=False)`* (§5.1). §5.1 writes `config.conf` for every launch, even with no settings, so the lane's value always holds; boards' fresh-install case needs no file. With the flag neither lane line is added and no file is written without the test's own text. Unit test in `test_ui.py`; `testing.md` documents it.
3. *The settings banner joins words.txt's warnings and the restart notice* (`shown_settings_warning`), as described above; neither branch had both.

*Gates* (the eight lanes at `98aec74`, one at a time, with `WRENTEST_ENGINE=classic,advanced WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1`, `WRENTEST_BOARDS` unset):
- `make check`: at the merge, Rust 4,272 passed, 0 failed, Python 305 OK; at `98aec74`, Rust 4,276 passed, 0 failed, Python 306 OK. The macOS clippy gate passed at the merge and at `98aec74`.
- `test_spell.py` at `4adadb2`: 12 passed on X11, 9 passed and 3 deselected on Wayland (tiny-skia).

| `WRENTEST_SPELL` | Renderer | X11 | Wayland |
|---|---|---|---|
| off | tiny-skia | 360 passed, 2 skipped (a first run at `557fdbe`: 1 failed, 359 passed, deviation 2) | 287 passed, 2 skipped, 73 deselected |
| off | wgpu | 360 passed, 2 skipped | 287 passed, 2 skipped, 73 deselected |
| on | tiny-skia | 360 passed, 2 skipped | 287 passed, 2 skipped, 73 deselected |
| on | wgpu | 360 passed, 2 skipped | 287 passed, 2 skipped, 73 deselected |

  The extra run, X11 tiny-skia with `WRENTEST_SPELL=on WRENTEST_BOARDS=on`: 360 passed, 2 skipped. The skips in every lane: the released-binary case (`WRENTEST_RELEASED_BIN` unset) and the classic engine's hanging-indent case. Weston did not crash: every Wayland lane passed on its first run, and the kernel log names no Weston fault.
- The strict docs build: no issues.
- Scenes: the 52 spell scenes of §5.3 and boards' scenes for Wren Light and Wren Dark on tiny-skia (30 PNG files and one SVG each). Against the spell finish run's captures, outside the header's worktree path, `spell-marks` is pixel-identical in all thirteen theme and renderer pairs; `spell-menu` differs only by the caret (a 2 by 18 px blink) and `spell-menu-above` and `spell-menu-empty` only by the status bar, which reads "Saved" where the earlier capture caught "Unsaved changes". Looked at: Wren Light and Wren Dark `spell-marks` on tiny-skia (nine zigzags, none under `apear`, which holds the caret), Wren Light `spell-menu` on wgpu (five rows under `replacments`), Nord `spell-menu-above` (`wrong` and `grown` above `wrogn`), Wren Light `board-flow` (the checkout flowchart fitted at 47 %) and Wren Dark `board-raw` (the conflicted board as text under its banner, no marks on the classic engine).

*At 125 %.* Boards passes `WINIT_X11_SCALE_FACTOR` to the app and `WREN_XVFB_SCREEN` sizes the screen, so `WREN_XVFB_SCREEN=2400x1500 WINIT_X11_SCALE_FACTOR=1.25` captured the spell scenes in a 1600 by 1000 window on tiny-skia, in both Wren themes. Text and layout are right, and every mark sits under its word. The zigzag is not crisp: each word's picture is drawn at 1.25 times its size and resampled, so the 1 px squares become a soft, lighter red band three device pixels tall with a 5 px period, its peaks two pixels wide and partly shaded. The shading is not the same along a word: on `replacments` it flips about halfway, where the bottom row of the band turns from separate dots into a near solid line. It reads as a zigzag at normal viewing distance, fainter than at 100 %, where it is three rows of single solid pixels in a 4 px period. §8's risk is confirmed; snapping the squares to device pixels would need the scale factor in the editor's drawing.

*What the real join must do* (whichever of the two reaches `main` second merges `main` first; `spell` is there, so `boards` merges `main`):
- Merge `main` into `boards`'s head of that day, which may be past `dc66cbd`, and resolve as above: keep both rows everywhere, take `Cargo.lock` from one side and run `cargo check` once without `--locked`, move "Spelling Word List…" into `menu.rs`, join the two settings warnings, and fix the two `keybind.rs` tests.
- Bring over `4adadb2`, `d4627cf`, `557fdbe` and `98aec74` (the code, the tests, the docs and the harness flag), which do not depend on this record.
- Update the status rows of `discovery/README.md` for both notes and this document's status line.
- Fix the two minors an independent check of the rehearsal found: the `editor.spell` test request still reports the last Markdown frame's marks (with `word: null` and their old rects) while a board is shown, because `spell_snapshot` reads `EditorView::spell_frame()`, which only the editor's own draw updates; return no marks unless the surface is `Surface::Text`, or clear the frame when the proofreader is withdrawn, so a harness case on a board can assert none (nothing is drawn; the request alone is stale). And re-wrap `docs/development/testing.md` around the `WRENTEST_BOARDS` sentence, which the merge left at about 100 characters.
- Run `make check`, the macOS clippy gate, the eight lanes, the extra boards-on lane, the scenes and the strict docs build again, and do the Mac step: the menu bar is now built from `menu.rs`, so check on Metal that "Spelling Word List…" is in the Wren menu and that ⌘. still reaches the editor with a board in the workspace.

## 8. After this, and risks

- **Add and Ignore** in the menu: rows under a divider; Add appends to `words.txt` (the first write Wren makes to it), Ignore is per note and needs a place to live.
- **More languages and sources:** another `WordSource` (a second Hunspell pair, system dictionaries on Linux, `NSSpellChecker` on macOS behind the one crate allowed `unsafe`), and a setting to choose.
- **Grammar:** `harper-core` as a `Proofreader` of its own, off the UI thread and paragraph by paragraph, with a second mark colour and a message row in the menu.
- **A smooth wave:** either SVG tiles sized exactly to each word (the software renderer ignores an image's own clip rectangle) or iced's `canvas`, which `boards` already turns on for the app but which needs app-wide antialiasing to look smooth on wgpu.
- **Known limits of the word rules.** A typo beside a slash (`adn/or`) or joined to the next word by a missing space after a full stop is not marked; a comment that spans lines is checked on its later lines; label words of a footnote definition are checked.
- **Known renderer issue: stale damage on tiny-skia.** After some note switches the real window keeps or lacks a few gutter numbers that a full redraw would draw, with spell check on or off (§7.5 `spell-endfix` item 12). It comes from iced's damage tracking and predates spell check; diagnose it before the iced upgrade.
- **Known limit: input in the frame of an Escape.** With no menu open the editor drops the keyboard on Escape and the app gives it back after the frame, so a key in the same frame is lost, with spell check on or off. The app closes the menu on Escape through a subscription that iced runs after the batch's widget messages, so an Escape in the batch of an opening can close a menu that a later suggest press of that batch opened, or arrive after the next frame's press (§7.5 `spell-endfix2` item 4).
- **Known limit: a press in the frame of an opening.** A left press in the same batch as the suggest key or a right-click that opens a menu only closes that menu, even where its row will be drawn, since the menu is laid out in the next frame; with a pause after the opening the same click accepts the row (§7.5 `spell-endfix2` item 6).
- **Risk: fractional display scales.** The zigzag is made of logical pixels; at 125 % it is drawn resampled, a soft band whose shading changes along a word (§7.5 "The join"). Snapping its squares to device pixels needs the scale factor in the editor's drawing.
- **Risk: `spellbook` is alpha.** The seeded tests and the text-safety lens are the guard; a panic found in it is a blocker to work around in `wren-spell`, never to hide.
- **Risk: suggestions on the UI thread.** Bounded by rule 4.7.3 and open question 3.
