# Markdown editor: discovery and plan

Status: **in progress** (branch `markdown-editor`, started 2026-11-08).

The goal is to make Wren's editor a capable Markdown editor:

- list continuation;
- indentation carry-over;
- smart Tab and Home;
- an outline;
- the everyday motions a Zed user expects.

It should ship as a pluggable component that can sit behind a setting until it has earned being the default.

This note records what we learned, the options we weighed, the decision, the behavior spec, and the milestone plan. The raw research reports are summarized here, not kept.

## 1. Where the editor is today

`crates/wren-iced/src/editor_view/` is a fork of iced 0.14.2's `text_editor`. It adds a gutter, a current-line band, a caret, and task strike-through. Underneath is `iced_graphics::text::Editor`, which wraps a private `cosmic_text::Editor`.

- **Edits** can only go through iced's coarse `Action` enum (`Insert`, `Paste`, `Enter`, `Backspace`, `Delete`, `Indent`, `Unindent`, motions). There is no range replace, no transactions and no absolute scroll. cosmic-text's own `auto_indent`, `insert_at`/`delete_range`, `Change` tracking and `set_scroll` are all unreachable behind iced's wrapper.
- **Undo** is full-text snapshots (`editor_history.rs`). Undo reloads the whole `Content`, which loses the scroll position.
- **Keys** go through `editor_view::key_binding`. It intercepts undo, redo and find and passes everything else to `Binding::from_key_press`. That function has no access to the text.
- **Missing basics:**
  - Tab does nothing: iced filters `"\t"` as a control character.
  - There is no word-delete (⌥⌫ / Ctrl+⌫).
  - There is no ⌘⌫, and no ⌘↑/↓ to the document start or end.
  - Backspace deletes one Unicode scalar, not a grapheme.
  - Shift+Enter is just Enter.
- **Markdown classifiers:** there are three, and they diverge.
  - the highlighter's line helpers (`markdown_highlighter.rs:363-421`): backtick fences only at column 0, no `~~~`, and list indents above 3 spaces rejected;
  - `done_task_ranges` (task strike-through);
  - `wren-core/src/index.rs`'s preview-line scanner, which accepts `N)` when the highlighter does not.
- **Per-keystroke cost** on a 5,000-line note: about 6 ms in a debug build. Almost all of it is the debug-build `split_whitespace().count()` word count in `App::edit` (5.6 ms debug, 0.2 ms release). The five or six whole-document copies per edit cost about 0.6 ms debug, which is secondary.
- **Outline parse cost**, measured on 5,000 lines with 152 headings:

  | Approach | Release | Debug |
  |---|---|---|
  | `pulldown-cmark` | 0.95 ms | 8 ms |
  | Fence-aware line scan | 0.04 ms | 0.4 ms |

## 2. Licensing: what we may use

Wren is MIT. Every dependency we add must be permissive. We may port code only from permissively licensed sources, and must keep their notices.

| Source | License | Use |
|---|---|---|
| iced, iced_graphics, iced_widget 0.14 | MIT | Already a dependency. `widget.rs` is already a fork. The new engine ports `iced_graphics/src/text/editor.rs` (MIT notice in the file header). |
| cosmic-text 0.15 | MIT OR Apache-2.0 | Already in the tree, re-exported by iced. The new engine owns its `Editor` directly. |
| CodeMirror `@codemirror/lang-markdown` | MIT | Behavior and test-corpus reference. Its 59 `before\| → after\|` command tests port to Rust table tests with the MIT notice. Clone: `../thirdparty/lang-markdown`. |
| Markdown All in One (`yzhang-gh/vscode-markdown`) | MIT | Reference for list editing, renumbering and formatting. Clone: `../thirdparty/vscode-markdown`. |
| `iced_code` 0.1.0 | MIT OR Apache-2.0 | Architecture reference only: it owns a cosmic-text editor in iced and draws through `fill_raw`. It is too young to depend on. |
| floem-editor-core | MIT | Reference for soft-tab and multi-cursor ops. |
| `pulldown-cmark`, `unicode-segmentation` | MIT, MIT/Apache | Allowed as dependencies; both already appear in the lockfile's ecosystem. |
| helix-core | MPL-2.0 | Design reference only. Copying a file would make that file MPL. As a dependency it is too heavy: 118 crates, char-indexed, no Markdown logic. |
| **Zed `editor`, `text`, `rope`, `language`, `markdown`, `outline`, `grammars`** | **GPL-3.0-or-later** | **Behavior only.** We may read the code and tests to learn *what* Zed does, then write our own spec and code. We must not copy, translate or closely paraphrase code or tests. |
| Zed `gpui`, `sum_tree`, `collections`, `util` | Apache-2.0 | Allowed with Apache attribution. gpui's editor-relevant part is a small text-input example. See the GPUI note (§2.1). |
| cosmic-edit | GPL-3.0-only | Behavior and architecture study only: a cosmic-text editor on libcosmic/iced. |

**Clean-room rule for this branch:** behavior learned from GPL sources is written down here, in our own words, as `before → after` cases. The implementation is written from that spec, never with GPL code open side by side.

### 2.1 GPUI and the Apache-licensed parts of Zed

"Apache where marked" means **per crate**. Each Zed crate declares its license in `Cargo.toml` and carries a `LICENSE-APACHE` or `LICENSE-GPL` symlink, and `script/check-licenses` enforces this. At `b5cd310b3b` the counts are:

- **37 crates are Apache-2.0.** These are GPUI: `gpui`, `gpui_*`, `sum_tree`, `collections`, `util`, `refineable`, `html_to_markdown`, and a few others.
- **215 crates are GPL-3.0-or-later**, including every crate with editor logic.
- **None is AGPL any more.**

Files with no mark (`assets/` keymaps and settings, `docs/`, `script/`) fall under the GPL default.

**GPUI has no editor.** Its editor-relevant Apache code is:

- `examples/input.rs`: a 920-line single-line input showing the IME contract.
- `text_system/line_wrapper.rs`: line wrapping with `IndentAdjustment`, a hanging indent for wrapped rows.
- `line_layout.rs`: hit-testing.
- `gpui_wgpu/src/cosmic_text_system.rs`: shaping on cosmic-text 0.19.

`sum_tree` is published as `gpui_sum_tree` and is light. Zed's rope on top of it is GPL, though.

| Project | License | Verdict for Wren |
|---|---|---|
| **gpui-component / GPUI Kit** (Longbridge) | Apache-2.0, except `html5minify` (MIT), icons (ISC), themes, and `.scm` queries | Strong reference for editor *mechanics*: intent-coalesced undo transactions with selection snapshots, IME preedit as one transaction, batched multi-line indent (`editor/indent.rs`), and a wrap/fold display map. It has **no Markdown-aware editing**. Depending on it means adopting GPUI, a toolkit migration. In 2025-09 it briefly depended on Zed's GPL rope, then reverted "for the License reason". So rewrite from it rather than copy verbatim. |
| **gpui-ce** | Apache-2.0 | A community fork of GPUI only, with parley text. Its `editable_text` is a form-field widget. Little value. |
| **writ** (wilfreddenton/writ) | MIT, declared in `Cargo.toml`; the repo has no LICENSE file | **The closest Markdown-behavior reference:** list and quote continuation, a Tab nesting cycle, ordered renumbering, hanging indent for wrapped items, outline, folding. It runs on winit, vello, and parley, so it is a reference, not a dependency. Ask the author to add a LICENSE before copying anything verbatim. |

Clones are in `../thirdparty/` (`gpui-component`, `gpui-ce`, `writ`, `velotype`).

**Clean-room rules** (verified answer from the licensing review):

1. **Never open in an implementing session:** Zed's `crates/{editor,rope,text,language,multi_buffer,markdown,markdown_preview,outline,outline_panel,ui,ui_input,vim,grammars}`, Zed's `assets/`, or `thirdparty/cosmic-edit`. Implementing agents get this list in their brief.
2. **GPL behavior reaches the code only through §5 of this note,** written in our own words.
3. **Tests are written from the spec,** never ported from GPL test files.
4. **Every borrowed MIT or Apache piece gets a provenance header.** It names the upstream path, commit, and license, plus a "Modified by Wren" line, following the `widget.rs` model. It also gets an entry in the shipped notices. `tools/package.py` only collects notices for cargo dependencies, not for copied source.

## 3. Options considered

**A. A smart layer on the current fork.**
- How it works: express every smart edit as "select a range, then `Paste`".
- What it gives: the line logic works today, with no new widget work.
- What it can't do: undo still loses the scroll position, and an outline jump can't place the heading exactly; neither problem goes away.
- Further gaps: Up/Down keep a stale column after programmatic moves, and true selection bounds after a double- or triple-click are hard to get.
- This remains the *fallback* path.

**B. Own the cosmic-text engine.**
- How it works: fork iced_graphics' editor glue, about 450 lines once `perform` is excluded. The widget then owns `cosmic_text::Editor` and draws it with `fill_raw`.
- What it gives:
  - real range edits;
  - `Change`-based undo that restores the cursor and the scroll position;
  - absolute scroll;
  - exact selection bounds;
  - control over the Up/Down column;
  - per-span metrics, which could later allow larger headings.
- What it keeps: IME, proportional fonts, bidi and wrapping.
- What it costs: Wren maintains a second fork when iced upgrades.

**C. A headless core with our own rope and layout** (scrive- or Zed-style).
- What it gives: the ceiling — multiple cursors, folding, decorations, very large files.
- What it costs: Wren would rebuild hit-testing, wrapping, IME, scrolling and accessibility. That is months of work. scrive, the closest example, is monospace-only and has no IME.
- Not justified by the current goals.

**Third-party widgets:**
- `iced-code-editor`: canvas, monospace, heavy dependencies.
- `scrive-iced`: monospace grid, no IME.
- `iced-markdown-editor`: WYSIWYG, a different product.
- `iced_code`: too young.

None of them fits as a dependency.

**Hanging indent** (wrapped list items aligned under the item text) is not possible through cosmic-text 0.15's API in any stack. It needs a small vendored cosmic-text layout patch. Wren already vendors `swash` and `iced_winit`. The spike landed on the owned engine (§6, *Hanging indent spike*).


## 4. Decision

Build the editor as **two dedicated crates with no Wren dependencies**, so either can be extracted and published later. Apply every smart edit through a single path, so it works on either backend. Then add an **owned cosmic-text backend** as an opt-in alternative to iced's editor.

```
crates/wren-markdown/   pure Rust; deps: unicode-segmentation only. No iced, no wren-core.
  block.rs     the one line classifier and container scanner (§5.1)
  outline.rs   headings outside code and front matter; section lookup
  edit.rs      Intent → EditPlan planner (§5.2–5.6)
  lib.rs       Lines trait, Pos, Selection, Replace, EditPlan, Intent, Options

crates/wren-editor/     the iced 0.14 editor component; deps: iced, wren-markdown.
  lib.rs              EditorView (moved from wren-iced), Message, Style, Palette
  widget.rs           the forked text_editor widget (moved), backend-neutral
  backend/iced.rs     Backend::Iced: today's iced_graphics::text::Editor
  backend/cosmic.rs   Backend::Cosmic: an owned cosmic_text::Editor, ported from
                      iced_graphics' editor glue (MIT notice), drawn via fill_raw
  highlighter.rs      MarkdownHighlighter (moved), classifying with wren-markdown::block;
                      its Settings carry front_matter_end (§5.1)
  history.rs          snapshot undo (moved) plus a scroll field and diff-apply
  find.rs             literal find (moved out of editor_view.rs)
  keys.rs             key_binding: intents, the IME guard, KeyPolicy

wren-core   depends on wren-markdown (index.rs preview lines use block).
wren-iced   depends on wren-editor; maps wren_core::theme to wren_editor::Style;
            builds the KeyPolicy from the Keymap; owns commands, outline palette.
```

**Extraction rules.**
- `wren-editor` must not name `wren_core` and must not know about App messages.
- The host passes in a `Style` and `Palette` (colors), a font, and metrics.
- The host also passes in a `KeyPolicy` (§5.8) that says, for each accelerator, "host command" or "editor command". This replaces any keymap dependency.
- Test-only accessors live behind a `test-support` feature, which `wren-iced/smoke-test` enables.
- Crate names stay `wren-*` until extraction, when they can be renamed.

**Backends.** `widget::Content` wraps `enum Backend { Iced(RawEditor), Owned(owned::Engine) }`. The owned engine is Wren's own code driving cosmic-text (MIT/Apache, the text library iced already uses); it shares nothing with the GPL cosmic-edit app. Users see it as `editor-engine = advanced`.
- There is one `TextEditor` widget and one `EditorView`, and both backends share one test suite.
- The widget branches only on drawing (`fill_editor` or `fill_raw`) and on the few engine calls: `perform`, `move_to`, `cursor`, `selection`, `line`, `line_count`, `bounds`, `update`, `highlight`, `buffer`.
- Planning runs inside a borrow: `content.with_lines(|lines| plan(lines, sel, intent, &opts))`. Both backends expose a `&cosmic_text::Buffer` (iced's `Editor::buffer()`, and the owned engine's buffer), and `wren-editor` wraps it in a newtype that implements `Lines` with `buffer.lines[i].text()`, so no line is copied. A newtype is needed because the orphan rule forbids implementing `wren_markdown::Lines` for `cosmic_text::Buffer` in `wren-editor`, and `wren-markdown` must not depend on cosmic-text. `Editor::line()` cannot back `Lines`: it returns a `Cow`, not a borrowed `&str`.

**Undo.** Keep the snapshot `History` (200 entries, 32 MiB, 1 s typing coalesce) for both backends.
- Snapshots also record the scroll position.
- Undo and redo apply a snapshot as a prefix/suffix diff (select the changed middle, then replace), not by reloading the content. This keeps shaping, and on the owned engine it lets `set_scroll` restore the exact viewport.
- Change-based undo is deferred unless memory or performance demands it.

**Applying a plan.** `EditorView::apply(plan)` runs one routine on either backend:
1. Snapshot the text, cursor and scroll once.
2. Apply the replacements bottom-up. Iced uses select + paste; the owned engine uses `delete_range` / `insert_at`.
3. Translate each `\n` in the inserted text to the line ending of the line being split. On the last line, which has no ending, use the previous line's ending, else LF.
4. Set the selection.
5. Record one history entry, closing any open typing group.
6. Re-run find, and reset the vertical goal column.

**Settings.**
- `markdown-assist = on | off`, default `on`. It turns the smart keys and Markdown commands on or off on any backend.
- `editor-engine = classic | advanced`, default `classic` until the owned engine meets the promotion bar (§8).
- When the engine changes live, the app rebuilds the editor from the session draft (`replace_text`, cursor kept, history dropped) and refocuses it.
- Once `classic` is retired, the key is still accepted and ignored for one release.

**Owned backend requirements** (from the engine review):
- **Clipping:** draw with `renderer.with_layer(text_bounds, …)`. tiny-skia ignores `Raw.clip_bounds`.
- **Up/Down column:** the engine owns `goal_x`. Vertical motions go through `Buffer::cursor_motion`. `goal_x` is cleared on every other motion, click, edit, undo, and programmatic move.
- **Buffer handle:** `downgrade()` the buffer last in `draw`. The buffer is never held strongly outside the engine, and `debug_assert` checks the strong count.
- **Caret, selection, and IME rectangles** come from `layout_runs()` with per-run `line_top` and `line_height`. Each is an `Option`, never a panic or a fallback to `ORIGIN`. After shaping, the cursor line is laid out explicitly, so `reshape_to_cursor` isn't needed.
- **Line endings:** Enter, Paste, and IME commit go through the same line-ending translation, so a CRLF note never gains LF breaks.
- **Font-system lock:** the global `FontSystem` lock is taken once per public entry point.
- **Highlighter port:** `topmost_line_changed` is updated from every mutation, including undo. The font-version and font-change resets are ported.
- **Scroll:** `replace_text` saves and restores scroll. For the outline jump, set the scroll before the cursor.
- **Acceptance:** measure draw time on tiny-skia, which repaints the whole text area on every caret blink because `Raw` is never equal.

**Every text change goes through `App::edit`.** Smart keys arrive as `Message::Intent`. Commands send the same message through `self.edit(...)`. `App::edit`'s post-change half becomes one helper (`App::text_changed`) that issues `EditDraft` and the revision bump, so autosave sees every edit. The word count leaves the keystroke path: the existing tick recounts it at most 250 ms after the first edit since the last count. That is a throttle rather than a strict debounce, so steady typing still updates the footer; the tick also runs without a session (the `--editor` spike) while a recount is due.

**Planner cost.** A line's scan record depends only on the lines up to it (front matter, which looks ahead for its closing fence, is found over the whole document first), so a plan that reads no later line scans only that far. Delete, the word deletes, and a selection delete make their edit directly, with no scan. Backspace scans only when the text before the cursor could all be indentation and markup. Home and the line delete skip the scan on a line that starts with a letter, and otherwise scan up to the line. Enter on a line outside any container scans up to the line. The draft diff compares only the lines between the untouched ends. A seeded test checks each shortcut against the full scan. Enter and Backspace inside a list, Tab, and the commands still scan the whole note: about 4 ms in a debug build and 0.3 ms in release at the end of a 5,000-item list. A scanner checkpoint cache in the editor, like the highlighter's, would bound those; `test_five_thousand_line_edit_measurements` times smart Enter and Backspace.

## 5. Behavior spec (P0)

Notation: `|` is the cursor, `«…»` is a selection, `⏎` is a line break, `·` is a significant space, and `→` separates before from after. "Column" counts Unicode scalars with tabs expanded to stops of 4. Only ASCII space and tab count as indentation.

### 5.1 Structure (`block.rs`)

A forward scanner keeps a **container stack**: blockquote `>` levels and list items, each with an indent, a content column, and a quote depth. The scanner state is `Clone`, so the highlighter can checkpoint it every 50 lines.

Front matter is the one rule that needs lookahead: line 0's `---` opens front matter only if a closing delimiter comes later. A highlighter sees one line at a time, so it cannot find out itself. Instead:
- The highlighter's `Settings` carry `front_matter_end: Option<usize>`, and it builds its scanner with `Scanner::with_front_matter(end)`. Fed that way, line by line, the scanner gives exactly what `scan()` gives.
- `EditorView` recomputes `block::front_matter_end` after every change (including undo and `replace_text`) and passes it in the settings. It is cheap: it reads past line 0 only when line 0 is `---`, and stops at the closing delimiter.
- When the value changes, iced calls `Highlighter::update`, which drops every checkpoint and restarts from line 0, because each checkpoint embeds the old value (`Scanner::front_matter_end` reports it). Edits that leave the value unchanged keep the usual `change_line` resync.

Each line is classified as one of:

| Kind | Rules |
|---|---|
| prose | Ordinary paragraph text. |
| blank | Empty or whitespace-only. |
| ATX heading | 0–3 spaces of indent within its container, then 1–6 `#` followed by a space or the end of the line. Closing `#`s are stripped. `\#` and an empty `#` are not headings for the outline. |
| setext underline | `===` or `---` directly under a paragraph line. A setext heading can span several paragraph lines. `- foo⏎---` is a list item followed by a thematic break, not a setext heading. |
| thematic break | CommonMark §4.1: 0–3 spaces of indent, then 3 or more of one of `-` `*` `_`, with optional spaces or tabs between. This includes `- - -` and `***`. A thematic break takes precedence over a list item. |
| list item | Bullet `-` `*` `+`, or ordered: 1–9 ASCII digits followed by `.` or `)`. The marker is followed by 1–4 spaces (or the line ends). 5 or more spaces mean the content column is marker + 1. A task item has `[ ]`, `[x]`, or `[X]` plus a space after the marker, on bullets and ordered items alike. Its `content_column` is the CommonMark content start. Its `text_start` is after the task box. |
| blockquote | `>` and an optional space, nestable, and may contain lists. |
| fence open / close | ```` ``` ```` or `~~~` (3 or more), at 0–3 spaces relative to the **container's content column**. The closing fence uses the same character, is at least as long, and has no info string. A backtick fence's info string cannot contain a backtick, so ```` ```rust``` is fine ```` is prose. An unclosed fence ends at the end of its container or the end of the document. |
| fence body | Lines between an opening and a closing fence. |
| indented code | 4 or more columns past the container content column, after a blank line or at the start of a container. It does not interrupt a paragraph. |
| front matter | Only if line 0 is `---` (a BOM before it is allowed, and trailing whitespace too), a later line is `---` or `...`, and the first non-blank inner line matches `^[A-Za-z0-9_-]+:\s` or `^#`. Otherwise line 0 is a thematic break. |
| HTML block | A line starting `<!--` through the line that holds `-->`, and lines starting with a block-level tag (`<div`, `<details`, …) through the next blank line. |

**Code-like contexts:** fence open and close lines, fence body, indented code, front matter, and HTML blocks. In a code-like context only §5.2's carry-over and §5.4's code Tab apply. The container prefixes (`>`, and a list item's content indent) are still continued.

### 5.2 Enter

Enter **always** returns a plan; it never falls back to iced's stock Enter, which inserts LF. With a selection, the planner first deletes the selection, then plans Enter at the collapsed point, as one change: `- a«bc»d` → `- a⏎- |d`.

| Before | After | Note |
|---|---|---|
| `- item\|` | `- item⏎- \|` | Same bullet character. |
| `- it\|em` | `- it⏎- \|em` | Splits the item. |
| `- [x] done\|` | `- [x] done⏎- [ ] \|` | New tasks start open. Also `1. [x] a\|` → `2. [ ] \|`. |
| `9. nine\|` | `9. nine⏎10. \|` | Then renumber (§5.6). `1)` style works too. Padded `09.` → `10.`. `999999999. x\|` gives a plain newline. |
| `> quote\|` | `> quote⏎> \|` | Also `> - a\|` → `> - a⏎> - \|`. |
| `··- sub\|` | `··- sub⏎··- \|` | Keeps the indent. |
| `- a\|⏎··- b` | `- a⏎··- \|⏎··- b` | An item with children gets a first child. |
| `- \|item` | `-⏎- \|item` | Inserts an empty item above (trimmed, like any markup-only line); the cursor stays with the text. Where an empty item would not read as one, directly after a paragraph line, it inserts a blank line instead: `Para⏎- \|x` → `Para⏎⏎- \|x`. |
| `-\| item`, `- [\| ] x` | as if at `text_start` | The cursor sits inside the markup. |
| `··\|- a` | `⏎··\|- a` | The cursor sits in the indentation: plain newline with carry-over. The line left behind is trimmed. |
| `- a⏎⏎- b\|` | `- a⏎⏎- b⏎⏎- \|` | A loose list stays loose. |
| `- foo··\|` | `- foo··⏎- \|` | A hard line break is kept. Only lines that become whitespace-only or markup-only are trimmed. |
| `····text\|` | `····text⏎····\|` | Carry-over copies only the whitespace left of the cursor. |
| `····\|` | `⏎····\|` | The line left behind is trimmed. |
| `- a⏎··b\|` | `- a⏎··b⏎··\|` | Continuation paragraph. |
| `---\|`, `# Title\|`, setext underline | plain newline (with carry-over) | |
| code-like | container prefix + carry-over | `> ```⏎> code\|` → `> code⏎> \|`. |

**Exit on an empty item** (the marker alone, with or without its trailing space: `-|`, `1.|`, `- [ ]|`). Nested items outdent first:

| Before | After |
|---|---|
| `- a⏎··- \|` | `- a⏎- \|` |
| `1. a⏎···- \|` | `1. a⏎2. \|` |

The outdented item takes the marker that Enter at the end of its parent would produce: the parent's bullet, or the parent's number + 1 with its delimiter, plus `[ ] ` if the parent is a task.

A marker alone right under a paragraph line in the same container (`- a⏎··- |`) is paragraph text or a setext underline to CommonMark, since an empty item cannot interrupt a paragraph. Enter reads it as the empty item the writer meant.

At the outermost level the marker is removed. **After a paragraph line a blank line is left**, so the next text is not a lazy continuation:

| Before | After |
|---|---|
| `- a⏎- \|` | `- a⏎⏎\|` |
| `- [ ] \|` (first line) | `\|` |
| `> a⏎> \|` | `> a⏎⏎\|` |
| `> > a⏎> > \|` | `> > a⏎>⏎> \|` |
| `> - \|` | `> \|` |

**When text follows on the next line, a blank line is left below the cursor as well**, so the item that follows does not run into what is typed. An ordered item other than 1 cannot interrupt a paragraph, so without the gap `1. a⏎⏎foo⏎2. c` would read `2. c` as paragraph text. The rest of the list is still renumbered (§5.6):

| Before | After |
|---|---|
| `1. a⏎2. \|⏎3. c` | `1. a⏎⏎\|⏎⏎2. c` |
| `- a⏎- \|⏎- c` | `- a⏎⏎\|⏎⏎- c` |
| `> - a⏎> - \|⏎> - c` | `> - a⏎>⏎> \|⏎>⏎> - c` |
| `1. \|⏎2. b` (first line) | `\|⏎⏎1. b` |

### 5.3 Shift+Enter

Shift+Enter breaks the line inside the item: the container prefix, then indent to the content column, with no marker and no task box. It never exits.

| Before | After |
|---|---|
| `- [ ] a\|` | `- [ ] a⏎··\|` |
| `10. a\|` | `10. a⏎····\|` |
| `> a\|` (quote) | `> a⏎>⏎> \|` (a paragraph break inside the quote) |

In a code-like context, Shift+Enter is the same as Enter.

### 5.4 Tab and Shift+Tab

**Indent stops on a line** are column 0, plus the content column of each enclosing list item, found by scanning upward within the container. Outside lists the stops are multiples of 4, and past the deepest item's content column they continue every 4 columns. Indentation is measured from the innermost quote's content start, and `>` prefixes are copied unchanged.

**Tab on a list line** (empty selection, cursor anywhere on the line):
- The item **and its subtree** become children of the **previous sibling**: their indent becomes that sibling's content column. The subtree is the following lines indented deeper than the item, up to the first non-blank line at the item's indent or shallower.
- If there is no previous sibling at that level, Tab does nothing and consumes the key. This guards against `x⏎⏎- a` turning into indented code.
- The new marker is the one Enter at the end of an existing nested list at the target level would give (its bullet, or its last number + 1), otherwise the item's own bullet or 1. Both runs are renumbered.
- Examples:
  - `- a⏎- b|` → `- a⏎··- b|`
  - `1. a⏎2. b|` → `1. a⏎···1. b|`
  - `1. a⏎···1. x⏎2. b|` → `1. a⏎···1. x⏎···2. b|`
  - `> - a⏎> - b|` → `> - a⏎> ··- b|`
- Indentation characters: keep the existing leading bytes, and append `\t` if the line's indent starts with a tab, otherwise spaces. Appended tabs reach the next tab stop at or past the target, which still nests the item (`- x⏎\t- a⏎\t- b|` → `- x⏎\t- a⏎\t\t- b|`). At the top level a leading tab is 4 columns, which makes the line indented code, so there Tab is the code Tab (`\t- a⏎\t- b|` → `\t- a⏎\t- b\t|`). The subtree moves by exactly the same number of columns, gaining tabs only where its indentation is all tabs.

**Shift+Tab on a list line:**
- The item and its subtree move to the parent's indent, taking the parent level's marker style and numbering. Its own task state is kept: `1. a⏎···- [x] b|` → `1. a⏎2. [x] b|`.
- Following siblings become its children and are renumbered from 1: `1. a⏎···1. x⏎···2. y|⏎···3. z` → `1. a⏎···1. x⏎2. y|⏎···1. z`. The first of them is set to 1 before anything is rescanned, because an ordered item that starts a list under a paragraph line reads as paragraph text unless it is 1. When the new marker is wider than the old one, they shift right to stay inside it.
- A top-level item with 1–3 spaces of indent moves to column 0. At column 0 it does nothing.
- If a tab character crosses the target column, it is replaced with spaces.

**Smart Tab on a blank or whitespace-only line** (the cursor at the line's end):
- The first Tab pads to the **nearest stop that is deeper than the current indent**, considering the previous non-blank line's indent and content column and the content columns of the items the blank line is inside. After `- [ ] task` the stop is 2, never 6.
- Later presses, once no such stop is deeper, add 4 spaces.
- On the first line of a document, Tab inserts spaces to the next multiple of 4.

**Selections:**
- A selection that ends at column 0 does not include that line. Blank lines are never indented.
- With a multi-line selection, Tab and Shift+Tab apply one delta to every line, taken from the first list line, or 4 if there is none. Shift+Tab removes `min(delta, indent)` from each line. The selection is kept.
  - The first list line's delta is what Tab or Shift+Tab would move it by: up to its previous sibling's content column, or back to its parent's indent (or its container's column 0). With no previous sibling, multi-line Tab does nothing, the same guard as a single line.
  - Only indentation changes; markers stay. The first list line is numbered to join its new run (1 when it starts a nested list), the sibling after the selection is restarted at 1 when Shift+Tab leaves it behind as a child, and every touched run is renumbered.
- With a single-line non-empty selection, Tab indents the line and never replaces the selection.

**Plain lines:**
- Tab inserts spaces to the next multiple of 4 at the cursor, or `\t` if the line's indent starts with a tab. A cursor inside quote markers inserts after them.
- Shift+Tab removes leading whitespace back to the previous stop, or does nothing at column 0.

**Code-like contexts:**
- Tab inserts `\t` if the line, or the nearest non-blank code line, starts with a tab. Otherwise it inserts spaces to the next multiple of 4, counted from the block's own indent.
- Shift+Tab outdents the line by one step.

Tab never moves focus.

### 5.5 Backspace, Delete, and Home

**Backspace** with an empty selection, in order:

1. **Cursor exactly at the end of the markup**, which is the line's `text_start` when its innermost container's marker is on the line. Continuation lines carry no marker, so they go on to rule 2. Code-like lines skip rules 1 and 2. The first applicable rule wins:
   1. Extra spaces after the markup go first: `>···|` → `> |`, `> -····|` → `> - |`. The markup ends one space or tab past the `>`, the list marker, or the task box.
   2. A task box goes next: `- [ ] |` → `- |`.
   3. A nested item outdents one level, with the same marker rules as Shift+Tab: `····- |` → `··- |`. An empty marker that only reads as an item to the writer (§5.2, under a paragraph line) outdents the way Enter's exit does.
   4. An item that has a previous sibling has its marker replaced by spaces up to that sibling's content column, becoming a continuation paragraph: `- a⏎- |b` → `- a⏎··|b`. The later items are then renumbered.
   5. A top-level item with no previous sibling has its marker removed: `- |` → `|`.
   6. A quote level is removed: `> > |` → `> |`.
2. **Only whitespace on the line before the cursor, and nothing after it.** Delete back to the previous indent stop (§5.4).
3. **Otherwise,** delete one grapheme cluster, or at column 0 the line break before the line.

A byte order mark on the first line is never deleted. With a selection, Backspace and every delete below remove the selection.

**Delete** removes one grapheme cluster, or at the end of a line the line break after it.

**Word deletes:**
- ⌥⌫ / Ctrl+⌫ deletes back one **group**, and ⌥⌦ / Ctrl+⌦ deletes forward one group. This is the convention of CodeMirror's `deleteGroupBackward`, VS Code's `deleteWordLeft`, Zed and Sublime.
  - Whitespace next to the cursor goes first. Then one run of a single character class goes with it.
  - There are two classes: **word** (UAX #29 word segments, so `a.b`, `3.14` and `can't` stay whole) and **punctuation**. Consecutive punctuation is one run.
  - Examples:

    | Before | After |
    |---|---|
    | `foo bar··\|` | `foo \|` |
    | `foo!!\|` | `foo\|` |
    | `**bold**\|` | `**bold\|` |
    | `- [ ] \|` | `- [ \|` |

  - Markup separated by spaces is not grouped further. Clearing a marker is the job of smart Backspace and ⌘⌫.
  - At either end of a line, the line break goes.
- ⌘⌫ (macOS) deletes back to `text_start`; from `text_start` it deletes to column 0: `- [ ] foo|` → `- [ ] |`. A cursor inside the markup also deletes to column 0, and at column 0 the line break goes.

**Home and Shift+Home** (also ⌘← and ⌃A on macOS; ⌃A becomes the smart toggle on purpose):
- The cycle is: start of the visual row (when the cursor is on a wrapped continuation row), then `text_start`, then column 0, then repeat. Only the editor's layout knows where visual rows begin, so the editor takes the first stop itself and plans `Intent::Home` (a motion with no edits) only when the cursor is not on a continuation row, or already at its start.
- A cursor inside the indentation or markup moves forward to `text_start`.
- `text_start` is after the indent, quote markers, list marker, task box, and an ATX heading's `#`s.
- Shift+Home extends the selection through the same stops.

**⌘↑ / ⌘↓** (macOS) move to the start and end of the document.

### 5.6 Renumbering

The **sibling run** is the ordered items with the same container, quote depth, indent, and delimiter.
- It skips blank lines, child lines, and continuation lines.
- It ends at a less-indented non-blank line, a heading, a fence, a thematic break, or a different delimiter or bullet.

**When it runs:**
- **After Enter, Backspace, or exit,** renumber the rest of the run only while the existing numbers are sequential. That includes the first item after the change: it is renumbered only if it was in sequence with the item it followed before the edit, so a deliberate gap or a countdown is left alone (`1. a|⏎5. b` Enter → `1. a⏎2. |⏎5. b`; `3. a|⏎1. b` → `3. a⏎4. |⏎1. b`; Backspace `1. a⏎2. |b⏎5. c` → `1. a⏎···|b⏎5. c`). CodeMirror always renumbers the first one. Items that become children of an outdented item start a new list and are always numbered from 1.
- **All-same numbering:** if a neighbouring sibling has the same number as the current item (`1.`, `1.`), keep that number and renumber nothing.
- **After Tab, Shift+Tab, or a line move,** renumber both affected runs from their start numbers.

**Number formatting:** keep a zero-padded width (`09.` → `10.`, `01.` → `02.`). `0.` is a valid start.

**Marker width changes:** when renumbering changes an item's marker width by Δ, shift its continuation and child lines by Δ in the same change: `8. a|⏎9. b⏎···- c` Enter → `8. a⏎9. |⏎10. b⏎····- c`.

### 5.7 Commands

Two palette commands also flip the rollout settings and save them to `config.conf`, just as the theme picker saves its choice: `editor.toggle_markdown_assist` ("Toggle Markdown assist") and `editor.toggle_engine` ("Switch editor engine (classic / advanced)"). Both are App scope and have no default binding.

| Command key | macOS | Linux | Behavior |
|---|---|---|---|
| `markdown.toggle_task` | ⌘↵ | Ctrl+Enter | `- [ ] a\|` ↔ `- [x] a\|`; `- a` → `- [ ] a`; `1. a` → `1. [ ] a`; `> a` → `> - [ ] a`; `a` → `- [ ] a`; an empty document → `- [ ] \|`. The cursor keeps its text offset. A selection marks every task line done if any is open, otherwise marks them all open. Plain lines are promoted only when no selected line is a task. Blank lines are skipped. In code it does nothing. A continuation or lazy line acts on its item's box. Headings, thematic breaks, and setext headings (the text lines and the underline, so `Foo\|⏎===` stays a heading) are not promoted. A cursor on a blank line gets a task at its indentation. |
| `markdown.bold` / `markdown.italic` / `markdown.code` | ⌘B / ⌘I / ⌘E | Ctrl+B / Ctrl+I / Ctrl+E | Wrap with `**`, `*`, or `` ` ``. The selection is trimmed of edge whitespace and clamped to `text_start`; with a multi-line selection, each line is wrapped. With no selection, the word at the cursor is wrapped, or, not on a word, an empty pair is inserted around the cursor. Already wrapped (markers just inside or just outside the selection, `*`/`_` alike) means unwrap. Code that contains a backtick uses a longer fence and padding. A run of three markers is both bold and italic: `**«a»**` italic → `***«a»***`. Over several lines, every line is unwrapped if all are wrapped, otherwise the unwrapped ones are wrapped. A whitespace-only selection gets an empty pair. Code lines are left alone. |
| `edit.move_line_up` / `edit.move_line_down` | ⌥↑ / ⌥↓ | Alt+↑ / Alt+↓ | Moves the lines the selection covers, keeping the selection, then renumbers both runs. Moving the last line, which has no ending, works: `a⏎b\|` up → `b\|⏎a`. At the top or bottom of the document it does nothing. Never renumbers inside code. Line endings stay in place while line text moves, since a plan writes breaks as `\n` and the editor gives each the ending of the line it splits; only a mixed-ending document can tell. A run counts from the number its first line had before the move (`1. a⏎2. b\|` up → `1. b\|⏎2. a`), and the item that lands first gets that number before anything is rescanned, so a list under a paragraph line survives (`x⏎1. a⏎2. b\|` up → `x⏎1. b\|⏎2. a`, where `2. b` alone could not interrupt `x`). A byte order mark stays at the start. |
| `navigation.outline` | ⇧⌘O | Alt+Shift+O | The outline palette (§5.9). |

The `markdown.*` and `edit.*` commands have **Editor scope** (§5.8). In the palette they always run: they focus the editor and act at its cursor. With `markdown-assist = off` they are disabled.

### 5.8 Key routing

Each `definitions!` row gains a scope, `App` or `Editor`. The host gives the widget a `KeyPolicy` (`Arc<dyn Fn(&Key, Physical, Modifiers) -> KeyRoute>`) built from the keymap, reading keys through the same `keys::shortcut` as the app's own shortcuts, and `key_binding` consults it **first**, right after the focus check and before Undo, Redo, and Find:
- An accelerator bound to an **Editor** command → `KeyRoute::Intent(intent)` → `Binding::Custom(Message::Intent(intent))`, the command's planner intent, so the widget needs no host command ids. `App::key_input` ignores Editor-scoped commands, so they fire only while the editor is focused.
- An accelerator bound to an **App** command → `KeyRoute::Host` → `None`. The editor doesn't also handle it: Ctrl+F on Linux opens find once, through the app, and a user binding of `ctrl+z` takes it from Undo.
- Anything else → `KeyRoute::Pass`, the editor's own handling below.
- On macOS, a key with ⌘ held never becomes an `Insert`, so an unbound ⌘B types nothing.
- While an IME preedit is non-empty, Enter, Tab, Backspace, and Delete go to the IME untouched, whatever the modifiers, and no key runs an Editor command. This is checked before the `KeyPolicy`. The preedit is cleared when the editor loses focus, since the IME's later events go to whichever field has it.
- Backspace, Delete, the word and line deletes, Tab, Shift+Tab, and ⌘↑ / ⌘↓ always map to `Message::Intent`; Enter, Shift+Enter, Home, and Shift+Home do only with assist on. The keys are read by post-processing `Binding::from_key_press`, so iced's platform mappings (⌘← and ⌃A as Home, ⌃H as Backspace) carry over. Word deletes take ⌥ on macOS, and Ctrl or Alt on Linux.
- **With assist off** the editor edits plain text, keeping the fixes that are not about Markdown: Enter, Shift+Enter, and Home are iced's own; Backspace deletes a grapheme cluster (at column 0, the line break); Tab inserts spaces to the next multiple of 4, replacing a selection inside one line as typing does; over a selection that spans lines, Tab indents each covered line (not empty ones) by 4 spaces, or a tab where the line starts with one, and keeps the selection, so it never replaces several lines; Shift+Tab takes the same step (a tab, or up to 4 spaces) off each covered line, or off the cursor's line; ⌘⌫ deletes to column 0; the §5.7 commands do nothing, and the host's `KeyPolicy` passes their keys to the editor's own handling, so Ctrl+Enter (⌘↵) breaks the line and Alt+↑ (⌥↑) moves the cursor as before the commands existed. Delete, the word deletes, and the document motions are the planner's either way, since they know nothing of Markdown.
- Editor commands stay out of the macOS menu bar in P0.

### 5.9 Outline palette

`Mode::Outline` works with or without a session, so it is available in `--editor` mode too.

- **Rows:** headings from `outline.rs`, indented by level below the note's top heading level, with an `H1`–`H6` tag. Each row shows its parent chain as the path. The title alone is fuzzy-matched (matching the path too would list every child of a matching section); matches rank by score, ties in document order. With an empty query, the section containing the cursor is preselected and marked current. A note without headings shows "No headings in this note."
- **Placement:** the outline palette is narrower (460 px) and sits beside the text so the preview stays visible: over the browser columns when they are wide enough, otherwise at the right edge.
- **Opening:** the app stores `outline_origin` (cursor, selection, and scroll) on `App`, with the note's headings, rebuilt only when `edit_revision` changes so a query keystroke just filters them. Opening moves nothing, even with the cursor or selection out of view; the preselected row is shown only by a highlight move or Enter.
- **Moving the highlight** previews the jump. On the owned engine the heading is set near the top of the view (in the last screenful, as near as the scroll allows). On Iced it is placed best-effort: exact when the view moves down, and on the top row when it moves up, because iced settles a scroll and reveals the cursor in the same update. When no row is highlighted (a query with no matches), the editor shows the origin.
- **Enter** clears the origin, then commits and focuses the editor.
- **Closing any other way** restores the origin in `sync_preview`: Escape, ⌘K, a dialog, or a workspace switch. If nothing was previewed (or the highlight came back to no row), the editor already shows the origin and closing moves nothing. The origin is restored only while the same note is open; a workspace switch or a deleted note drops it.
- **Rows are keyed** by `(edit_revision, line)`. If the text changes underneath, the palette refreshes and keeps the same heading highlighted: matched by title and path, counted among its namesakes (the third `### Example` stays the third) while their number is unchanged, else the nearest by line with that title, else the nearest heading. A previewed heading is followed to its new line; a row never previewed stays unshown. A stale key is refused. If another note opens underneath, the palette closes.

## 6. Later (P1/P2)

**P1:**
- Click a task box to toggle it.
- Paste a URL over a selection to make a link.
- Insert link (⇧⌘K / Ctrl+K).
- Headings ⌘1–6 / Ctrl+1–6.
- Strikethrough and toggle blockquote.
- Duplicate line and select line.
- Wrap the selection when typing a pair character over it.
- Find and replace.
- A current-heading breadcrumb in the footer.
- The **hanging indent** cosmic-text patch spike (gpui's Apache `IndentAdjustment` and writ's MIT wrap are references). Landed on the owned engine; see *Hanging indent spike* below.
- Larger heading sizes on the owned engine (`Attrs.metrics_opt`).
- Exact selection bounds after a double- or triple-click.
- A maximum text width outside focus mode (centered column) and an `editor-line-height` setting. Both work on either backend; asked for by the design review.
- A brighter current-line number in the gutter (`Style::gutter_current`).

**Coordination with the theme work** (`design/theme-polish`, session wren-26). The editor rebuild merges first.
- **`theme.rs` stays untouched.** This work does not edit `wren-core/src/theme.rs`.
- **wren-26's highlighter changes come across.** Their commit `df7a6c1` (syntax tokens, the emphasis-marker font fix, links, and the `code_block` band) is folded into `wren-editor`'s highlighter in milestone 5.
- **New color fields.** `wren_editor::Palette` gains `heading`, `link`, `code`, and `quote`. `Style` gains `code_block` and `gutter_current`.
- **Landed in milestone 5.** The selection fill is `Style::text_selection` (renamed from `selection`, rather than a second field). `Palette` dropped `danger` and `success`, which nothing draws with any more: strikethrough is `completed` text between `faint` tildes, and it now gets the strike line that done tasks get. Done-task text is flat `completed` (no emphasis faces or code colour) except for links. The code band is for fenced blocks only; indented code stays muted without one. Under the interim mapping `code_block` equals `current_line`, so the band and the inline-code chip vanish on the cursor's line until the theme gives `code_block` its own colour.
- **Interim mapping.** Until the new tokens land, `editor_colors` maps them as follows:

  | Editor field | Mapped from |
  |---|---|
  | `heading` | `accent` |
  | `link` | `accent` |
  | `code` | `success` |
  | `quote` | `muted` |
  | `code_block` | `current_line` |
  | `gutter_current` | `body` |

  Round 2 adds three more fields:

  | Editor field | Mapped from | Use |
  |---|---|---|
  | `heading2` | `heading` | H2 text. H1 stays `heading`; H3–H6 use plain text color. |
  | `completed` | `faint` | Text of done tasks and `~~strike~~`, readable at ≥ 4.5:1 contrast. Strike quads and markers stay `faint`. |
  | `text_selection` | `selected` | The editor selection fill. |

- **wren-26's second editor commit also comes across.** Milestone 5 absorbs `e17c877` along with `df7a6c1`: H2 uses `heading2`, done and struck text use `completed`, and the selection uses `text_selection`. Theme side `516231c` changes the Wren Light accent to `#a14808`; when merging `test_editor.py`, keep our engine parametrization and their light tuple `(0xA1, 0x48, 0x08)`. Default pane widths now live in `workspace_state.rs` (`PANE_WIDTHS`, used by `Op::ResetWidths`).
- **Notify wren-26** whenever `editor_colors`, `Palette`, `Style`, or the highlight test in `test_editor.py` changes.

**Design review round 2** (P1, widget and highlighter; all four landed in milestone 5):
- **Selected newlines.** Each selected newline and each selected empty line gets a selection cell about one character wide, so a selection across blank lines reads as one block.
- **Link destinations.** In `[text](url)` the URL uses `completed`; brackets and parens stay `faint`.
- **Links in done tasks.** Link text on a done-task line keeps the `link` color, and the strike is still drawn.
- **Inline precedence.** Code spans and HTML comments are found first. Links and bare URLs come next, so emphasis never pairs a `_` or `*` inside a URL. Strikethrough and emphasis are matched inside link text, and then across the line, where they can wrap whole links and other emphasis. Link destinations end at the `)` that balances their parentheses (CommonMark). Bare URLs drop trailing punctuation, and they drop a final `)` only when it is unbalanced (GFM autolinks).
- **Inline code chip** (optional). A `code_block` background behind inline code spans, with about 2 px padding.

**Design-review defaults** for the P1 visual items:

| Item | Default |
|---|---|
| Heading scale | H1 1.35 em, H2 1.18 em, H3 1.05 em, H4–H6 1.0 em. All bold. `#` markers keep the line's base size, in faint. |
| Max text width | `editor-max-width = 760 \| none`, default 760 px, matching focus mode. |
| Line height | `editor-line-height`, a ratio. Default 1.7 (24 px at 14 pt); accepted range 1.2–2.2. |
| Gutter | The current line number uses `gutter_current`; other line numbers stay `faint`. |
| Hanging indent | List items and quotes, aligned to the content column. Landed on `advanced` only. |

**Hanging indent spike** (landed on `md-hang`):
- **What it does.** On the owned engine, the wrapped rows of a line inside a container start at the end of its container prefix (`LineInfo::prefix_end`): the content column after `- `, `10. `, `> `, `> - `, or an item's continuation indent. A task hangs under its box (the content column), not under its text (`text_start`). Lines outside containers, including headings and top-level code, do not hang.
- **The patch.** `vendor/cosmic-text` is cosmic-text 0.15.0 plus 85 added lines, 13 of them replacing upstream lines, in three files, recorded in its `WREN.md`. `BufferLine` gains a hanging indent given as a byte offset. `ShapeLine::layout_to_buffer` measures it from the shaped glyphs before that byte (capped at half the wrap width), wraps every row after the first that much narrower (deciding whether a word must be broken at the width of the row it lands on), and starts those rows that far in; their `LayoutLine::w` includes the indent. `Buffer::hit` treats a click left of a row's first glyph as the row's start (upstream compared with 0). `make check-patches` asserts the patch applies.
- **Decisions.**
  - The indent is a byte offset, not pixels, so cosmic-text measures it in the line's own fonts in the same layout pass, and a font or size change needs nothing from the host. writ measures its prefix the same way.
  - Always on for `advanced`; no setting.
  - The highlighter records each styled line's `prefix_end` in `LineDecor::hang`, and `Engine::highlight` sets it with the attributes, so an edit re-hangs exactly the lines it re-highlights (from the changed line's checkpoint down through the viewport). `Engine::highlight` now takes the concrete `MarkdownHighlighter`.
  - ↑, ↓, PageUp, and PageDown on the owned engine aim at the caret's x (`goal_x: Option<f32>`) and pick the nearest glyph boundary on the target row. cosmic-text aims at a glyph index, which drifts by the indent between a first row and a hanging one.
  - Selection rectangles start at a row's leftmost glyph instead of 0.
  - **Known limitation: moves through unhighlighted lines.** The hang is set only on lines the highlighter has styled (from its checkpoint through the viewport). PageUp, PageDown, and ↑/↓ that cross lines outside the viewport count rows with `line_layout`, which lays those lines out with the hang they last had, 0 if never highlighted. So the walk can count fewer rows than are drawn once those lines are highlighted, and a PageDown can land a row or more away from exactly one page. Fixing it means passing the highlighter into `perform` so `vertical` can set each line's `decor(line).hang` before laying it out; deferred while the engine is opt-in.
  - **Highlighting is part of layout.** `Backend::update` (the widget's `layout`) highlights the visible lines as well as `draw`, because the hang is applied with the highlights and events can arrive before the next frame. Headless Weston draws nothing between a `set` and the next keys, so ↓ there used the un-hung rows until this moved; the second call returns early when nothing changed. Classic gets the same ordering, so its bold and heading widths are in place before a key too.
  - **Classic does nothing.** iced owns its buffer and lays it out without the indent; its rows still start at the left edge. This is an engine difference, documented in the configuration reference.
- **Tests.** `owned.rs`: rows of bullets, ordered items, tasks, quotes, a list in a quote, a nested item, and a continuation paragraph hang at the measured content column; a word that fits the first row but not a continuation row breaks instead of running past the edge, also under a capped deep-quote hang; prose, headings, and top-level code do not; editing the marker moves the hang (and a list opened above turns a line into a hanging continuation); the caret, a click in the indent, and full and partial selections on hanging rows; ↑/↓ keep the visual column across first and hanging rows. `test_editor.py::test_wrapped_list_rows_hang_on_the_advanced_engine` checks ↓ in the real app. The `hanging-indent` scene shows them in both themes.
- **Upgrade risk.** Moderate and local. All the logic is in `layout_to_buffer`, about 530 lines of wrapping and positioning, so a newer release that touches it means re-applying the ten limit checks (six `limit`, four `landing_limit`) and the row offset by hand, then running the owned-engine tests. The `BufferLine` field and the `hit` change are mechanical. `check-patches` makes a silent drop impossible, and §8 already rules out an iced upgrade while two engines exist.
- **Upstreaming.** Feasible. The patch is additive and off by default, layout with every indent at 0 is unchanged, and hanging indents are a common rich-text need (gpui and parley both have one). Upstream would likely want a pixel form alongside the byte form (an enum), the extra `layout_to_buffer` argument folded into a settings struct to avoid a breaking signature change, RTL coverage in its tests, and probably x-based Up/Down to replace its glyph-index `cursor_x_opt` (a `TODO` in `Buffer::cursor_motion`). Worth proposing once the owned engine is the default.

**P2:**
- Folding; expand and shrink the selection.
- Tables.
- Regex find.
- Typewriter mode and focus dimming.
- Spell check.
- Multiple cursors (only with a headless core).

## 7. Testing

**`wren-markdown`:**
- Table tests in `before| → after|` notation (`«»` marks selections).
- Every case runs under `\n`, `\r\n`, `\r`, `\n\r`, mixed endings, with and without a final newline, and on an empty document.
- The CodeMirror `test-commands.ts` corpus (MIT, credited in the test module) is ported. The cases where §5 decides differently are listed in a "divergences" table with the new expected output:
  - quote exit;
  - tight-list move;
  - `- |x` at the start of the text;
  - Backspace on the first item;
  - HTML;
  - tab indent units;
  - multi-cursor cases (dropped).
- A seeded random test plans every intent at random selections (some off the document or inside a character) in 30,000 small generated documents with mixed endings and a BOM. It checks that edits are in bounds, on character boundaries, in order, and apart, that they write only `\n`, and that the selection is valid in the edited lines.

**`wren-editor`:**
- The existing `editor_view` and widget tests run against `Backend::ALL`.
- New tests cover plan application, undo and redo restoring the cursor (and, on the owned engine, the scroll), line endings, graphemes, `goal_x`, the IME guard (in widget state), `KeyPolicy` routing, and the macOS ⌘ insert guard.

**`wren-iced`:**
- App tests check that a command edit reaches the session draft and the revision.
- Setting switches rebuild the editor and keep the draft.
- The outline origin is restored on Escape.

**Functional (`wrentest`):**
- `WrenApp(settings=...)` writes `config.conf` before launch.
- `editor.get` reports `engine`, and the editor suite is parametrized over the engine (`WRENTEST_ENGINE`).
- The `editor.command` RPC (`{"intent": …}`) dispatches through `App::edit`.
- `native_x11` cases cover Return, shift+Return, Tab, shift+Tab, BackSpace, Home, ctrl+Return, and alt+Up.
- One case checks that Alt+↑ with the Notes column active leaves the text alone.

**Visual:** `capture_scenes.py` gets a list-heavy note scene, an outline palette scene, and a tiny-skia mid-row scroll scene, in both themes.

### 7.1 CodeMirror divergences

`crates/wren-markdown/src/edit.rs` ports 39 of the 40 Enter cases of CodeMirror's `insertNewlineContinueMarkup` suite (`test/test-commands.ts` at `f90daf6e`). The cases below assert §5 instead of CodeMirror. "Declines" means CodeMirror's command returns false and the host's plain newline runs; Wren's Enter always plans (§5.2).

| CodeMirror case | Before | CodeMirror | Wren |
|---|---|---|---|
| doesn't continue anything at the top level | `one\|` | declines | `one⏎\|` |
| doesn't do anything in non-Markdown content | `- <div\|>` | declines | `- <div⏎··\|>`: HTML is code-like, and the item's indent continues. |
| will end blockquotes after two empty lines | `> one⏎>⏎> \|` | `> one⏎⏎\|` | `> one⏎>⏎\|`: one Enter on an empty quote line ends the quote (§9). |
| will end nested blockquotes after two empty lines | `> > one⏎> >⏎> > \|` | `> > one⏎> ⏎> \|` | `> > one⏎> >⏎> \|` |
| can drop trailing space when pressing enter in a blockquote | `>··\|` | `>⏎> \|` | `\|`: an empty quote line exits. |
| can move list markup when pressing enter directly after it | ` - one⏎ - \|` | ` - one⏎⏎ - \|` | ` - one⏎⏎\|`: Enter on an empty item exits instead of loosening the list. |
| exits tight list after second item | ` - one⏎ - two⏎ - \|` | ` - one⏎ - two⏎\|` | ` - one⏎ - two⏎⏎\|`: the blank line keeps the next text out of the paragraph. |
| will keep the current ordered list number when moving a marker | ` 1. one⏎ 2. \|` | ` 1. one⏎⏎ 2. \|` | ` 1. one⏎⏎\|` |
| can move list markup inside a blockquote | `> 1. one⏎> 2. \|` | `> 1. one⏎>⏎> 2. \|` | `> 1. one⏎>⏎> \|` |
| doesn't fire when the cursor is before the markup depth | `- a⏎\|bc` | declines | `- a⏎⏎\|bc`, the plain newline. |
| doesn't fire in lazily continued paragraphs | `1. a⏎b c\|` | declines | `1. a⏎b c⏎\|`, the plain newline. |
| doesn't take effect in fenced code | `- ```foo\|`, `> - ```foo\|` | declines | `- ```foo⏎··\|`, `> - ```foo⏎>···\|`: code keeps its container prefix. |
| can continue markup for multiple cursors | | | Dropped: Wren has one cursor. |

`- |x` at the start of the text agrees with CodeMirror (`1. |x⏎2. y` → `1.⏎2. |x⏎3. y`). §5.2 only differs right after a paragraph line, which the corpus does not cover.

`crates/wren-markdown/src/edit/delete.rs` ports 16 of the 17 Backspace cases of the `deleteMarkupBackward` suite at the same commit. Where CodeMirror's command declines, its host deletes a character; Wren's Backspace always plans (§5.5), so the case asserts that plan. Backspace on the first item agrees with CodeMirror (` - |` → `|`), as do the quote, extra-space, marker, and indentation cases.

| CodeMirror case | Before | CodeMirror | Wren |
|---|---|---|---|
| does nothing in regular text | `one\|` | declines | `on\|`, one grapheme. |
| does nothing at the top level | `one⏎\|` | declines | `one\|`, the line break. |
| does nothing in a continued list item | `- Foo⏎-⏎··\|Welcome` | declines | `- Foo⏎-⏎·\|Welcome`: a continuation line has no marker, and text follows the cursor. |
| doesn't delete normal text in continued list items | `- ⏎a \|b` | declines | `- ⏎a\|b` |
| normalizes whitespace on deleting (tab indent unit) | `··- one⏎··- \|` | `··- one⏎\t\|` | `··- one⏎····\|`: tab indent units; the line's indentation has no tab, so it stays spaces. |
| doesn't kick in in a continued paragraph under a quote | `> q⏎rs\|t` | declines | `> q⏎r\|t` |
| can delete for multiple cursors | | | Dropped: Wren has one cursor. |

## 8. Milestones and commit order

Each commit builds and passes `make check`. The work is done in parallel worktrees and merged onto `markdown-editor`.

1. **Extract the editor into `crates/wren-editor`** with no behavior change: `Style`/`Palette` instead of `wren_core::theme`, a `test-support` feature, and a CLAUDE.md layout update.
2. **`wren-markdown`:** `block` and `outline` with tests. *Parallel with 1.*
3. **`wren-markdown`:** the `edit` planner for §5.2–5.7, the CodeMirror corpus, and the divergence table.
4. **`wren-editor`:** the `Backend` seam with Iced only. Then the owned engine behind `editor-engine`. Then the snapshot scroll field, diff-apply, `goal_x`, and line-ending normalization. *Parallel with 3.*
5. **Point the highlighter, `done_task_ranges`, and `index.rs` at `wren-markdown::block`.** Done. The strike ranges come from the highlighter's per-line decorations rather than a separate scan. A note's search preview is its first paragraph line outside front matter and headings (setext ones included), so a note whose only body is code or an empty item has no preview.
6. **Intents** (done on `md-keys`):
   - the `Message::Intent` / `EditorView::apply` path;
   - the `App::edit` helper and the debounced word count;
   - the `markdown-assist` setting;
   - key mapping and the IME guard;
   - the `editor.command` RPC and `native_x11` cases.
7. **Command scope and `KeyPolicy`,** the ⌘ insert guard, and the §5.7 commands with docs (done on `md-keys`). The commands sit in a "Markdown" palette group. With assist off they are disabled in the palette, and their keys stay the editor's and do nothing. The `shortcut` test request runs an Editor command the widget's way: the bound key through the policy, only while the editor is focused.
8. **The outline palette.**
9. **Harness work:** settings support, engine parametrization, and the scenes. Then review.

**Promotion bar for `editor-engine = advanced` as the default:**
- the shared suite passes on both backends in all four lanes;
- tiny-skia draw time is within an agreed margin of `fill_editor`;
- a manual macOS check of IME and ⌘ keys passes;
- dogfooding finds no draft-loss bugs.

**Abandon it if** an iced upgrade is needed first, or the raw draw costs too much. Smart editing still ships on Iced.

There is **no iced upgrade while two backends exist.**

## 9. Open questions

- **⌘K:** it stays the palette alias, and Insert Link is ⇧⌘K. Revisit with a Format menu.
- **Blockquote exit:** one Enter with a blank line left, chosen over CodeMirror's two empty lines.
- **macOS ⌘E** is the system's "Use Selection for Find". We take it for inline code; revisit if it bothers people.
- **A Mac console check is needed** for ⌘+letter and IME. ssh cannot send synthetic keys.
- **IME anchor off screen (decided).** When a composition starts, which is the first non-empty preedit, the editor scrolls the caret into view. TextEdit, VS Code and Zed all reveal the insertion point when typing begins, so the candidate window always has a real anchor. If the caret is still off screen in the frame before that scroll, the anchor is the caret's column, clamped to the nearest view edge, never the corner. Disabling the IME is never an option.
- **Mixed endings, known edge case:** a CR line followed by an emptied LF line (`a\r` then `\n`) serializes as one CRLF break, so the editor shows one more line than a reload of the saved text. Rewriting the empty line's ending to CR after such an edit would close it; not done in this milestone.
