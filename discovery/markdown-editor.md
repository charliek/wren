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

**Hanging indent** (wrapped list items aligned under the item text) is not possible through cosmic-text 0.15's API in any stack. It needs a small vendored cosmic-text layout patch. Wren already vendors `swash` and `iced_winit`. This is a separate spike.


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
  highlighter.rs      MarkdownHighlighter (moved), classifying with wren-markdown::block
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

**Backends.** `widget::Content` wraps `enum Backend { Iced(RawEditor), Cosmic(cosmic::Engine) }`.
- There is one `TextEditor` widget and one `EditorView`, and both backends share one test suite.
- The widget branches only on drawing (`fill_editor` or `fill_raw`) and on the few engine calls: `perform`, `move_to`, `cursor`, `selection`, `line`, `line_count`, `bounds`, `update`, `highlight`, `buffer`.
- Planning runs inside a borrow: `content.with_lines(|lines| plan(lines, sel, intent, &opts))`. `Lines` is implemented over `&cosmic_text::Buffer`, so no line is copied.

**Undo.** Keep the snapshot `History` (200 entries, 32 MiB, 1 s typing coalesce) for both backends.
- Snapshots also record the scroll position.
- Undo and redo apply a snapshot as a prefix/suffix diff (select the changed middle, then replace), not by reloading the content. This keeps shaping, and on Cosmic it lets `set_scroll` restore the exact viewport.
- Change-based undo is deferred unless memory or performance demands it.

**Applying a plan.** `EditorView::apply(plan)` runs one routine on either backend:
1. Snapshot the text, cursor and scroll once.
2. Apply the replacements bottom-up. Iced uses select + paste; Cosmic uses `delete_range` / `insert_at`.
3. Translate each `\n` in the inserted text to the line ending of the line being split. On the last line, which has no ending, use the previous line's ending, else LF.
4. Set the selection.
5. Record one history entry, closing any open typing group.
6. Re-run find, and reset the vertical goal column.

**Settings.**
- `markdown-assist = on | off`, default `on`. It turns the smart keys and Markdown commands on or off on any backend.
- `editor-engine = classic | cosmic`, default `classic` until Cosmic meets the promotion bar (§8).
- When the engine changes live, the app rebuilds the editor from the session draft (`replace_text`, cursor kept, history dropped) and refocuses it.
- Once `classic` is retired, the key is still accepted and ignored for one release.

**Cosmic backend requirements** (from the engine review):
- **Clipping:** draw with `renderer.with_layer(text_bounds, …)`. tiny-skia ignores `Raw.clip_bounds`.
- **Up/Down column:** the engine owns `goal_x`. Vertical motions go through `Buffer::cursor_motion`. `goal_x` is cleared on every other motion, click, edit, undo, and programmatic move.
- **Buffer handle:** `downgrade()` the buffer last in `draw`. The buffer is never held strongly outside the engine, and `debug_assert` checks the strong count.
- **Caret, selection, and IME rectangles** come from `layout_runs()` with per-run `line_top` and `line_height`. Each is an `Option`, never a panic or a fallback to `ORIGIN`. After shaping, the cursor line is laid out explicitly, so `reshape_to_cursor` isn't needed.
- **Line endings:** Enter, Paste, and IME commit go through the same line-ending translation, so a CRLF note never gains LF breaks.
- **Font-system lock:** the global `FontSystem` lock is taken once per public entry point.
- **Highlighter port:** `topmost_line_changed` is updated from every mutation, including undo. The font-version and font-change resets are ported.
- **Scroll:** `replace_text` saves and restores scroll. For the outline jump, set the scroll before the cursor.
- **Acceptance:** measure draw time on tiny-skia, which repaints the whole text area on every caret blink because `Raw` is never equal.

**Every text change goes through `App::edit`.** Smart keys arrive as `Message::Intent`. Commands send the same message through `self.edit(...)`. `App::edit`'s post-change half becomes one helper that issues `EditDraft`, the revision bump, and the word count, so autosave sees every edit. The word count leaves the keystroke path: it is debounced on the existing tick.

## 5. Behavior spec (P0)

Notation: `|` is the cursor, `«…»` is a selection, `⏎` is a line break, `·` is a significant space, and `→` separates before from after. "Column" counts Unicode scalars with tabs expanded to stops of 4. Only ASCII space and tab count as indentation.

### 5.1 Structure (`block.rs`)

A forward scanner keeps a **container stack**: blockquote `>` levels and list items, each with an indent, a content column, and a quote depth. The scanner state is `Clone`, so the highlighter can checkpoint it every 50 lines.

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
| `- \|item` | `- ⏎- \|item` | Inserts an empty item above; the cursor stays with the text. Directly after a paragraph line it inserts a blank line instead: `Para⏎- \|x` → `Para⏎⏎- \|x`. |
| `-\| item`, `- [\| ] x` | as if at `text_start` | The cursor sits inside the markup. |
| `··\|- a` | `··⏎··\|- a` | The cursor sits in the indentation: plain newline with carry-over. |
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

At the outermost level the marker is removed **and a blank line is left**, so the next text is not a lazy continuation:

| Before | After |
|---|---|
| `- a⏎- \|` | `- a⏎⏎\|` |
| `- [ ] \|` (first line) | `\|` |
| `> a⏎> \|` | `> a⏎⏎\|` |
| `> > a⏎> > \|` | `> > a⏎>⏎> \|` |
| `> - \|` | `> \|` |

### 5.3 Shift+Enter

Shift+Enter breaks the line inside the item: the container prefix, then indent to the content column, with no marker and no task box. It never exits.

| Before | After |
|---|---|
| `- [ ] a\|` | `- [ ] a⏎··\|` |
| `10. a\|` | `10. a⏎····\|` |
| `> a\|` (quote) | `> a⏎>⏎> \|` (a paragraph break inside the quote) |

In a code-like context, Shift+Enter is the same as Enter.

### 5.4 Tab and Shift+Tab

**Indent stops on a line** are column 0, plus the content column of each enclosing list item, found by scanning upward within the container. Outside lists the stops are multiples of 4. Indentation is measured from the innermost quote's content start, and `>` prefixes are copied unchanged.

**Tab on a list line** (empty selection, cursor anywhere on the line):
- The item **and its subtree** become children of the **previous sibling**: their indent becomes that sibling's content column. The subtree is the following lines indented deeper than the item, up to the first non-blank line at the item's indent or shallower.
- If there is no previous sibling at that level, Tab does nothing and consumes the key. This guards against `x⏎⏎- a` turning into indented code.
- The new number is the last number of an existing nested list at the target level + 1, otherwise 1. Both runs are renumbered.
- Examples:
  - `- a⏎- b|` → `- a⏎··- b|`
  - `1. a⏎2. b|` → `1. a⏎···1. b|`
  - `1. a⏎···1. x⏎2. b|` → `1. a⏎···1. x⏎···2. b|`
  - `> - a⏎> - b|` → `> - a⏎> ··- b|`
- Indentation characters: keep the existing leading bytes, and append `\t` if the line's indent starts with a tab, otherwise spaces.

**Shift+Tab on a list line:**
- The item and its subtree move to the parent's indent, taking the parent level's marker style and numbering. Its own task state is kept: `1. a⏎···- [x] b|` → `1. a⏎2. [x] b|`.
- Following siblings become its children and are renumbered from 1: `1. a⏎···1. x⏎···2. y|⏎···3. z` → `1. a⏎···1. x⏎2. y|⏎···1. z`.
- A top-level item with 1–3 spaces of indent moves to column 0. At column 0 it does nothing.
- If a tab character crosses the target column, it is replaced with spaces.

**Smart Tab on a blank or whitespace-only line** (the cursor at the line's end):
- The first Tab pads to the **nearest stop that is deeper than the current indent**, considering the previous non-blank line's indent and content column. After `- [ ] task` the stop is 2, never 6.
- Later presses add 4 spaces.
- On the first line of a document, Tab inserts spaces to the next multiple of 4.

**Selections:**
- A selection that ends at column 0 does not include that line. Blank lines are never indented.
- With a multi-line selection, Tab and Shift+Tab apply one delta to every line, taken from the first list line, or 4 if there is none. Shift+Tab removes `min(delta, indent)` from each line. The selection is kept.
- With a single-line non-empty selection, Tab indents the line and never replaces the selection.

**Plain lines:**
- Tab inserts spaces to the next multiple of 4 at the cursor, or `\t` if the line's indent starts with a tab.
- Shift+Tab removes leading whitespace back to the previous stop, or does nothing at column 0.

**Code-like contexts:**
- Tab inserts `\t` if the line, or the nearest non-blank code line, starts with a tab. Otherwise it inserts spaces to the next multiple of 4, counted from the block's own indent.
- Shift+Tab outdents the line by one step.

Tab never moves focus.

### 5.5 Backspace, Delete, and Home

**Backspace** with an empty selection, in order:

1. **Cursor exactly at the end of the markup.** The first applicable rule wins:
   1. Extra spaces after the markup go first: `>···|` → `> |`, `> -····|` → `> - |`.
   2. A task box goes next: `- [ ] |` → `- |`.
   3. A nested item outdents one level, with the same marker rules as Shift+Tab: `····- |` → `··- |`.
   4. An item that has a previous sibling has its marker replaced by spaces, becoming a continuation paragraph: `- a⏎- |b` → `- a⏎··|b`. It is then renumbered.
   5. A top-level item with no previous sibling has its marker removed: `- |` → `|`.
   6. A quote level is removed: `> > |` → `> |`.
2. **Only whitespace on the line before the cursor, and nothing after it.** Delete back to the previous indent stop (§5.4).
3. **Otherwise,** delete one grapheme cluster.

**Delete** removes one grapheme cluster.

**Word deletes:**
- ⌥⌫ / Ctrl+⌫ deletes back to the previous word boundary (Unicode word segmentation), and ⌥⌦ / Ctrl+⌦ forward to the next one.
- ⌘⌫ (macOS) deletes back to `text_start`; from `text_start` it deletes to column 0: `- [ ] foo|` → `- [ ] |`.

**Home and Shift+Home** (also ⌘← and ⌃A on macOS; ⌃A becomes the smart toggle on purpose):
- The cycle is: start of the visual row (when the cursor is on a wrapped continuation row), then `text_start`, then column 0, then repeat.
- A cursor inside the indentation or markup moves forward to `text_start`.
- `text_start` is after the indent, quote markers, list marker, task box, and an ATX heading's `#`s.
- Shift+Home extends the selection through the same stops.

**⌘↑ / ⌘↓** (macOS) move to the start and end of the document.

### 5.6 Renumbering

The **sibling run** is the ordered items with the same container, quote depth, indent, and delimiter.
- It skips blank lines, child lines, and continuation lines.
- It ends at a less-indented non-blank line, a heading, a fence, a thematic break, or a different delimiter or bullet.

**When it runs:**
- **After Enter, Backspace, or exit,** renumber the rest of the run only while the existing numbers are sequential.
- **All-same numbering:** if a neighbouring sibling has the same number as the current item (`1.`, `1.`), keep that number and renumber nothing.
- **After Tab, Shift+Tab, or a line move,** renumber both affected runs from their start numbers.

**Number formatting:** keep a zero-padded width (`09.` → `10.`, `01.` → `02.`). `0.` is a valid start.

**Marker width changes:** when renumbering changes an item's marker width by Δ, shift its continuation and child lines by Δ in the same change: `8. a|⏎9. b⏎···- c` Enter → `8. a⏎9. |⏎10. b⏎····- c`.

### 5.7 Commands

| Command key | macOS | Linux | Behavior |
|---|---|---|---|
| `markdown.toggle_task` | ⌘↵ | Ctrl+Enter | `- [ ] a\|` ↔ `- [x] a\|`; `- a` → `- [ ] a`; `1. a` → `1. [ ] a`; `> a` → `> - [ ] a`; `a` → `- [ ] a`; an empty document → `- [ ] \|`. The cursor keeps its text offset. A selection marks every task line done if any is open, otherwise marks them all open. Plain lines are promoted only when no selected line is a task. Blank lines are skipped. In code it does nothing. |
| `markdown.bold` / `markdown.italic` / `markdown.code` | ⌘B / ⌘I / ⌘E | Ctrl+B / Ctrl+I / Ctrl+E | Wrap with `**`, `*`, or `` ` ``. The selection is trimmed of edge whitespace and clamped to `text_start`; with a multi-line selection, each line is wrapped. With no selection, the word at the cursor is wrapped, or, not on a word, an empty pair is inserted around the cursor. Already wrapped (markers just inside or just outside the selection, `*`/`_` alike) means unwrap. Code that contains a backtick uses a longer fence and padding. |
| `edit.move_line_up` / `edit.move_line_down` | ⌥↑ / ⌥↓ | Alt+↑ / Alt+↓ | Moves the lines the selection covers, keeping the selection and each line's own ending, then renumbers both runs. Moving the last line, which has no ending, works: `a⏎b\|` up → `b\|⏎a`. At the top or bottom of the document it does nothing. Never renumbers inside code. |
| `navigation.outline` | ⇧⌘O | Alt+Shift+O | The outline palette (§5.9). |

The `markdown.*` and `edit.*` commands have **Editor scope** (§5.8). In the palette they always run: they focus the editor and act at its cursor. With `markdown-assist = off` they are disabled.

### 5.8 Key routing

Each `definitions!` row gains a scope, `App` or `Editor`. The host gives the widget a `KeyPolicy` built from the keymap, and `key_binding` consults it **first**:
- An accelerator bound to an **Editor** command → `Binding::Custom(Message::Command(id))`. `App::key_input` ignores Editor-scoped commands, so they fire only while the editor is focused.
- An accelerator bound to an **App** command → `None`. The editor doesn't also handle it.
- On macOS, a key with ⌘ held never becomes an `Insert`, so an unbound ⌘B types nothing.
- While an IME preedit is non-empty, Enter, Tab, Backspace, and Delete go to the IME untouched.
- With assist on, Enter, Shift+Enter, Tab, Shift+Tab, Backspace, Delete, the word and line deletes, Home, and Shift+Home map to `Message::Intent`.
- Editor commands stay out of the macOS menu bar in P0.

### 5.9 Outline palette

`Mode::Outline` works with or without a session, so it is available in `--editor` mode too.

- **Rows:** headings from `outline.rs`, indented by level. Each row shows its parent chain as the path and is fuzzy-matched. With an empty query, the section containing the cursor is preselected.
- **Opening:** the app stores `outline_origin` (cursor, selection, and scroll) on `App`.
- **Moving the highlight** previews the jump. On Cosmic the heading is set near the top of the view (in the last screenful, as near as the scroll allows). On Iced it is placed best-effort.
- **Enter** clears the origin, then commits and focuses the editor.
- **Closing any other way** restores the origin in `sync_preview`: Escape, ⌘K, a dialog, or a workspace switch.
- **Rows are keyed** by `(edit_revision, line)`. If the text changes underneath, the palette refreshes.

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
- The **hanging indent** cosmic-text patch spike (gpui's Apache `IndentAdjustment` and writ's MIT wrap are references).
- Larger heading sizes on Cosmic (`Attrs.metrics_opt`).
- Exact selection bounds after a double- or triple-click.

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

**`wren-editor`:**
- The existing `editor_view` and widget tests run against `Backend::ALL`.
- New tests cover plan application, undo and redo restoring the cursor (and, on Cosmic, the scroll), line endings, graphemes, `goal_x`, the IME guard (in widget state), `KeyPolicy` routing, and the macOS ⌘ insert guard.

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

## 8. Milestones and commit order

Each commit builds and passes `make check`. The work is done in parallel worktrees and merged onto `markdown-editor`.

1. **Extract the editor into `crates/wren-editor`** with no behavior change: `Style`/`Palette` instead of `wren_core::theme`, a `test-support` feature, and a CLAUDE.md layout update.
2. **`wren-markdown`:** `block` and `outline` with tests. *Parallel with 1.*
3. **`wren-markdown`:** the `edit` planner for §5.2–5.7, the CodeMirror corpus, and the divergence table.
4. **`wren-editor`:** the `Backend` seam with Iced only. Then Cosmic behind `editor-engine`. Then the snapshot scroll field, diff-apply, `goal_x`, and line-ending normalization. *Parallel with 3.*
5. **Point the highlighter, `done_task_ranges`, and `index.rs` at `wren-markdown::block`.**
6. **Intents:**
   - the `Message::Intent` / `EditorView::apply` path;
   - the `App::edit` helper and the debounced word count;
   - the `markdown-assist` setting;
   - key mapping and the IME guard;
   - the `editor.command` RPC and `native_x11` cases.
7. **Command scope and `KeyPolicy`,** the ⌘ insert guard, and the §5.7 commands with docs.
8. **The outline palette.**
9. **Harness work:** settings support, engine parametrization, and the scenes. Then review.

**Promotion bar for `editor-engine = cosmic` as the default:**
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
