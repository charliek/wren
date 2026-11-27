//! Scanner states kept between reads of a document that changes a little
//! at a time, such as a note being typed in, so that reading line `l`
//! again scans from the nearest kept state at or before it, not from the
//! first line: a host that asks about the caret's line after every edit
//! costs a few dozen lines a keystroke, not the whole note.
//!
//! What is kept is the lines read and the scanner before every
//! [`EVERY`]th of them. [`Checkpoints::update`] compares the kept lines
//! with the document's and forgets everything from the first that
//! differs: a line's classification depends only on the lines before it,
//! so what was read above that line still holds.

use std::ops::Range;

use crate::block::{self, Boxes, Container, LineInfo, Scanner, Task};
use crate::Lines;

/// Lines between two kept scanner states.
pub const EVERY: usize = 64;

/// Bytes of room a forget leaves unreturned.
const SPARE: usize = 64 * 1024;

/// The most bytes the kept states may take. A state holds the containers
/// open at its line, so a deep block quote that lazy lines continue makes
/// each one large: past this, a state is not kept, and a read starts from
/// an earlier one.
pub const STATE_BYTES: usize = 4 * 1024 * 1024;

/// What keeping `scanner` takes.
fn cost(scanner: &Scanner) -> usize {
    std::mem::size_of::<Scanner>() + std::mem::size_of_val(scanner.containers())
}

/// Whether `line` is one that may close front matter.
fn delimiter(line: &str) -> bool {
    matches!(line.trim_end_matches([' ', '\t']), "---" | "...")
}

/// See the module's description.
#[derive(Clone, Debug)]
pub struct Checkpoints {
    boxes: Boxes,
    front_matter_end: Option<usize>,
    /// Whether the lines read are kept; see [`Checkpoints::states`].
    texts: bool,
    /// The lines read, one after another, and where each ends in `text`.
    /// Without their text, every line ends at 0.
    text: String,
    ends: Vec<usize>,
    /// The scanner before some of the lines that are multiples of
    /// [`EVERY`], with the line, in order: always line 0's, and the others
    /// while [`STATE_BYTES`] allows.
    saved: Vec<(usize, Scanner)>,
    /// What `saved` takes, and the most it may.
    saved_bytes: usize,
    state_bytes: usize,
    /// How many lines it has scanned.
    scanned: u64,
}

impl Checkpoints {
    /// Nothing kept yet, for a document whose task boxes are `boxes`.
    pub fn new(boxes: Boxes) -> Self {
        Self {
            boxes,
            front_matter_end: None,
            texts: true,
            text: String::new(),
            ends: Vec::new(),
            saved: vec![(0, Scanner::with_front_matter(None, boxes))],
            saved_bytes: 0,
            state_bytes: STATE_BYTES,
            scanned: 0,
        }
    }

    /// As [`Checkpoints::new`], but keeping none of the document's text:
    /// only how many lines were read and the scanner states. For a host
    /// that must hold no copy of the text and knows its first changed
    /// line, which it tells [`update_from`](Self::update_from);
    /// [`update`](Self::update) has no lines to compare, and forgets
    /// everything.
    pub fn states(boxes: Boxes) -> Self {
        Self {
            texts: false,
            ..Self::new(boxes)
        }
    }

    /// The task boxes it reads.
    pub fn boxes(&self) -> Boxes {
        self.boxes
    }

    /// How many lines it has scanned since it was made: what a read cost.
    pub fn scanned(&self) -> u64 {
        self.scanned
    }

    /// The kept line `i`.
    fn kept(&self, i: usize) -> &str {
        let start = i.checked_sub(1).map_or(0, |before| self.ends[before]);
        &self.text[start..self.ends[i]]
    }

    /// Bring what is kept up to `lines`: forget everything from the first
    /// line that differs from the one kept, and everything once the front
    /// matter ends elsewhere. Returns that line: how many lines of what
    /// was read still hold. Call it whenever `lines` may have changed
    /// since the last read.
    pub fn update<L: Lines + ?Sized>(&mut self, lines: &L) -> usize {
        if !self.texts {
            return self.update_from(lines, 0);
        }
        let front = block::front_matter_end(lines);
        if front != self.front_matter_end {
            self.restart(front);
            return 0;
        }
        let count = lines.line_count().min(self.ends.len());
        let first = (0..count)
            .find(|&i| self.kept(i) != lines.line(i))
            .unwrap_or(count);
        self.forget(first);
        first
    }

    /// [`update`](Self::update), told by the caller that the lines before
    /// `first` are as they were when last read (an editor that knows its
    /// earliest changed line): nothing is compared, and what is kept is
    /// forgotten from `first`, or all of it once the front matter ends
    /// elsewhere. Returns the line kept states now hold from.
    pub fn update_from<L: Lines + ?Sized>(&mut self, lines: &L, first: usize) -> usize {
        let first = first.min(lines.line_count()).min(self.ends.len());
        // Where the front matter ends depends only on the lines up to its
        // close, or, while it is unclosed, on whether a later line closes
        // it: with those lines as they were, it ends where it did.
        let front = match self.front_matter_end {
            Some(end) if first > end => Some(end),
            None if first > 0 && !delimiter(lines.line(0).trim_start_matches('\u{feff}')) => None,
            None if first > 0 && !(first..lines.line_count()).any(|j| delimiter(lines.line(j))) => {
                None
            }
            _ => block::front_matter_end(lines),
        };
        if front != self.front_matter_end {
            self.restart(front);
            return 0;
        }
        self.forget(first);
        first
    }

    /// Forget everything: the front matter now ends at `front`.
    fn restart(&mut self, front: Option<usize>) {
        self.front_matter_end = front;
        self.forget(0);
        self.saved[0] = (0, Scanner::with_front_matter(front, self.boxes));
    }

    /// Forget the lines from `from` on, and the states they made. What
    /// a larger document held is given back: another note read after a
    /// long one keeps no more than it needs.
    fn forget(&mut self, from: usize) {
        if from < self.ends.len() {
            let start = from.checked_sub(1).map_or(0, |before| self.ends[before]);
            self.text.truncate(start);
            self.ends.truncate(from);
        }
        let kept = self.saved.partition_point(|(line, _)| *line <= from).max(1);
        for (_, scanner) in self.saved.drain(kept..) {
            self.saved_bytes -= cost(&scanner);
        }
        if self.text.capacity() > 2 * self.text.len() + SPARE {
            self.text.shrink_to_fit();
        }
        if self.ends.capacity() > 2 * self.ends.len() + SPARE / 8 {
            self.ends.shrink_to_fit();
        }
        if self.saved.capacity() > 2 * self.saved.len() + 8 {
            self.saved.shrink_to_fit();
        }
    }

    /// The bytes of text and line ends kept, and those the states take.
    pub fn retained(&self) -> (usize, usize) {
        (
            self.text.capacity() + self.ends.capacity() * std::mem::size_of::<usize>(),
            self.saved_bytes + self.saved.capacity() * std::mem::size_of::<(usize, Scanner)>(),
        )
    }

    /// Read lines `range` of `lines`, which [`update`](Self::update) has
    /// seen as they are: `each` is given each line's number, text and
    /// classification, and the scanner just after it (its containers).
    /// The lines before `range` are scanned from the nearest kept state,
    /// and every line read is kept.
    pub fn read<L: Lines + ?Sized>(
        &mut self,
        lines: &L,
        range: Range<usize>,
        mut each: impl FnMut(usize, &str, &LineInfo, &Scanner),
    ) {
        let end = range.end.min(lines.line_count());
        let from = range.start.min(end);
        let start = from.min(self.ends.len());
        let k = self.saved.partition_point(|(line, _)| *line <= start) - 1;
        let (begin, scanner) = &self.saved[k];
        let (begin, mut scanner) = (*begin, scanner.clone());
        for i in begin..end {
            let last = self.saved.last().map_or(0, |(line, _)| *line);
            if i % EVERY == 0 && i > last {
                let bytes = cost(&scanner);
                if self.saved_bytes + bytes <= self.state_bytes {
                    self.saved_bytes += bytes;
                    self.saved.push((i, scanner.clone()));
                }
            }
            let text = lines.line(i);
            if i == self.ends.len() {
                if self.texts {
                    self.text.push_str(text);
                }
                self.ends.push(self.text.len());
            }
            let info = scanner.next_line(text);
            self.scanned += 1;
            if i >= from {
                each(i, text, &info, &scanner);
            }
        }
    }

    /// Line `line` of `lines` (see [`read`](Self::read)) as `read` takes
    /// it; `None` past the last line.
    pub fn line<L: Lines + ?Sized, R>(
        &mut self,
        lines: &L,
        line: usize,
        read: impl FnOnce(&str, &LineInfo, &Scanner) -> R,
    ) -> Option<R> {
        if line >= lines.line_count() {
            return None;
        }
        let mut read = Some(read);
        let mut out = None;
        self.read(lines, line..line + 1, |_, text, info, scanner| {
            out = read.take().map(|read| read(text, info, scanner));
        });
        out
    }
}

/// The task whose box a line opens, as the whole document reads it: the
/// line's innermost container is a list item that opened on it with a
/// task box. `info` is the line's classification and `scanner` the
/// scanner just after it. A line of code, a lazy line and a line inside a
/// block quote of an item open none.
pub fn opened_task(info: &LineInfo, scanner: &Scanner) -> Option<Task> {
    if info.kind.is_code_like() || (info.kind == block::Kind::Blank && info.item.is_none()) {
        return None;
    }
    let stack = scanner.containers();
    let slot = stack.len().checked_sub(1)?;
    if info.lazy || scanner.continued() > slot {
        return None;
    }
    match stack[slot] {
        Container::Item(item) => item.task,
        Container::Quote { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    /// Notes with lists, tasks, quotes, fences, front matter and lazy
    /// lines, each of more lines than [`EVERY`].
    fn notes() -> Vec<String> {
        let block = "# Head\n\
            - [ ] one @sam\n\
            \x20 more of it\n\
            lazy line\n\
            \n\
            - item\n\
            \x20 - [x] nested\n\
            \x20   - [/] deeper\n\
            > - [ ] quoted\n\
            > > - [-] twice\n\
            - [ ] > quote in an item\n\
            ```\n\
            - [ ] fenced\n\
            ```\n\
            \x20   - [ ] indented code\n\
            1. [ ] ordered\n\
            - [?] unknown\n\
            -  [ ] wide\n\
            - [ ]\n\
            \n";
        let body = block.repeat(12);
        vec![
            body.clone(),
            format!("---\ntitle: x\n---\n{body}"),
            format!("```\n{body}"),
            format!("{body}```\n- [ ] after a late fence\n"),
            "- [ ] alone".to_owned(),
            String::new(),
        ]
    }

    /// Every line's task as a fresh full read finds it.
    fn expected(text: &str, boxes: Boxes) -> Vec<Option<Task>> {
        let doc = Document::new(text);
        (0..doc.line_count())
            .map(|l| {
                crate::edit::task_item(&doc, l, boxes)
                    .filter(|(k, _)| *k == l)
                    .map(|(_, task)| task)
            })
            .collect()
    }

    fn read_all(kept: &mut Checkpoints, text: &str) -> Vec<Option<Task>> {
        let doc = Document::new(text);
        kept.update(&doc);
        let mut found = Vec::new();
        kept.read(&doc, 0..doc.line_count(), |_, _, info, scanner| {
            found.push(opened_task(info, scanner));
        });
        found
    }

    #[test]
    fn a_line_opens_the_task_the_whole_document_reads() {
        for boxes in [Boxes::Status, Boxes::Plain] {
            for text in notes() {
                let mut kept = Checkpoints::new(boxes);
                assert_eq!(
                    read_all(&mut kept, &text),
                    expected(&text, boxes),
                    "{text:?}"
                );
                // Line by line, backwards, from what was kept.
                let doc = Document::new(&text);
                let want = expected(&text, boxes);
                for l in (0..doc.line_count()).rev() {
                    let found = kept.line(&doc, l, |_, info, scanner| opened_task(info, scanner));
                    assert_eq!(found, Some(want[l]), "{l} {text:?}");
                }
            }
        }
    }

    /// Edits anywhere, of every kind, read as a fresh read reads them,
    /// and an edit near the end reads only the lines after the last
    /// kept state before it.
    #[test]
    fn edits_forget_from_the_first_changed_line() {
        for text in notes() {
            let mut kept = Checkpoints::new(Boxes::Status);
            read_all(&mut kept, &text);
            let lines: Vec<&str> = text.split('\n').collect();
            let n = lines.len();
            for at in [0, 1, 2, n / 2, n.saturating_sub(2), n.saturating_sub(1)] {
                for change in ["```", "---", "- [ ] new", "", "lazy", "  - [x] deep", "> "] {
                    let mut edited: Vec<&str> = lines.clone();
                    if at < n {
                        edited[at] = change;
                    }
                    let edited = edited.join("\n");
                    assert_eq!(
                        read_all(&mut kept, &edited),
                        expected(&edited, Boxes::Status),
                        "{at} {change:?}"
                    );
                    // Lines added and taken away.
                    let longer = format!("{edited}\n{change}\n- [ ] tail");
                    assert_eq!(
                        read_all(&mut kept, &longer),
                        expected(&longer, Boxes::Status)
                    );
                    let shorter: String = edited.lines().take(at).collect::<Vec<_>>().join("\n");
                    assert_eq!(
                        read_all(&mut kept, &shorter),
                        expected(&shorter, Boxes::Status)
                    );
                }
            }
        }
    }

    /// Keeping states alone and told the first changed line, it reads
    /// what a fresh read reads, and holds none of the document's text.
    #[test]
    fn states_alone_read_from_the_line_named_and_keep_no_text() {
        let read = |kept: &mut Checkpoints, text: &str, first: usize| {
            let doc = Document::new(text);
            kept.update_from(&doc, first);
            let found: Vec<_> = (0..doc.line_count())
                .map(|l| {
                    kept.line(&doc, l, |_, info, scanner| opened_task(info, scanner))
                        .flatten()
                })
                .collect();
            assert!(kept.text.is_empty());
            assert_eq!(kept.ends.len(), doc.line_count());
            found
        };
        for text in notes() {
            let mut kept = Checkpoints::states(Boxes::Status);
            assert_eq!(read(&mut kept, &text, 0), expected(&text, Boxes::Status));
            let lines: Vec<&str> = text.split('\n').collect();
            let n = lines.len();
            for at in [0, 1.min(n - 1), n / 2, n - 1] {
                for change in ["```", "---", "- [ ] new", "", "  - [x] deep", "> "] {
                    let mut edited: Vec<&str> = lines.clone();
                    edited[at] = change;
                    let edited = edited.join("\n");
                    assert_eq!(
                        read(&mut kept, &edited, at),
                        expected(&edited, Boxes::Status),
                        "{at} {change:?}"
                    );
                    let longer = format!("{edited}\n{change}\n- [ ] tail");
                    assert_eq!(
                        read(&mut kept, &longer, n),
                        expected(&longer, Boxes::Status)
                    );
                    assert_eq!(read(&mut kept, &text, at), expected(&text, Boxes::Status));
                }
            }
            // With no line to compare, `update` forgets every state.
            let doc = Document::new(&text);
            assert_eq!(kept.update(&doc), 0);
            assert!(kept.ends.is_empty());
        }
    }

    #[test]
    fn typing_at_the_end_reads_the_last_few_lines() {
        let mut text = notes()[0].repeat(100);
        text.push_str("- [ ] last");
        let mut kept = Checkpoints::new(Boxes::Status);
        let doc = Document::new(&text);
        let last = doc.line_count() - 1;
        assert!(last > 20_000);
        kept.update(&doc);
        kept.line(&doc, last, |_, _, _| ());
        for c in ["!", "!!", "!!!"] {
            let typed = format!("{text}{c}");
            let doc = Document::new(&typed);
            let before = kept.scanned();
            assert_eq!(kept.update(&doc), last);
            let task = kept.line(&doc, last, |_, info, scanner| opened_task(info, scanner));
            assert!(task.flatten().is_some());
            assert!(
                kept.scanned() - before <= EVERY as u64,
                "{}",
                kept.scanned() - before
            );
        }
        // A caret moving up reads from a kept state.
        let typed = format!("{text}!!!");
        let typed = Document::new(&typed);
        let before = kept.scanned();
        kept.line(&typed, last / 2, |_, _, _| ());
        assert!(kept.scanned() - before <= EVERY as u64);
        // Past the end there is nothing, however far.
        assert_eq!(kept.line(&typed, usize::MAX, |_, _, _| ()), None);
        assert_eq!(kept.line(&typed, last + 1, |_, _, _| ()), None);
        // A short note read after it gives back what the long one held.
        let (bytes, _) = kept.retained();
        assert!(bytes > 4 * SPARE, "{bytes}");
        let short = Document::new("- [ ] one\ntwo");
        kept.update(&short);
        kept.line(&short, 1, |_, _, _| ());
        let (bytes, states) = kept.retained();
        assert!(bytes < 2 * SPARE, "{bytes}");
        assert!(states < 4096, "{states}");
    }

    /// A block quote nested hundreds deep that thousands of lazy lines
    /// continue: every state holds its whole stack of containers, so past
    /// the bound states are no longer kept, and what is read still equals
    /// a full read.
    #[test]
    fn deep_quotes_keep_the_states_bounded() {
        let mut text = format!("{}deep\n", "> ".repeat(200));
        for n in 0..2_000 {
            text.push_str(&format!("lazy {n}\n"));
        }
        text.push_str("\n- [ ] after the quote\n");
        // A small bound, so that a test-sized note passes it.
        let bound = 64 * 1024;
        let mut kept = Checkpoints::new(Boxes::Status);
        kept.state_bytes = bound;
        let found = read_all(&mut kept, &text);
        assert!(kept.saved_bytes <= bound, "{}", kept.saved_bytes);
        assert!(kept.saved.len() < 2_000 / EVERY, "{}", kept.saved.len());
        assert!(
            cost(&kept.saved.last().unwrap().1) > 4096,
            "the states are deep"
        );
        assert_eq!(found, expected(&text, Boxes::Status));
        // Reads after an edit near the end still agree.
        let edited = text.replace("after the quote", "after the quote, edited");
        assert_eq!(
            read_all(&mut kept, &edited),
            expected(&edited, Boxes::Status)
        );
        let doc = Document::new(&edited);
        let last = doc.line_count() - 2;
        assert!(kept
            .line(&doc, last, |_, info, scanner| opened_task(info, scanner))
            .flatten()
            .is_some());
    }

    /// An unclosed front matter header is not read again after an edit
    /// below it that closes nothing, and is once one does.
    #[test]
    fn an_unclosed_header_is_read_again_only_for_a_closing_line() {
        let open = "---\ntitle: x\n- [ ] in the header\n- [ ] still\n";
        let doc = Document::new(open);
        let mut kept = Checkpoints::new(Boxes::Status);
        kept.update(&doc);
        let read = |kept: &mut Checkpoints, text: &str, first: usize| {
            let doc = Document::new(text);
            kept.update_from(&doc, first);
            (0..doc.line_count())
                .map(|l| {
                    kept.line(&doc, l, |_, info, scanner| opened_task(info, scanner))
                        .flatten()
                })
                .collect::<Vec<_>>()
        };
        let typed = format!("{open}- [ ] typed");
        assert_eq!(read(&mut kept, &typed, 4), expected(&typed, Boxes::Status));
        let closed = format!("{open}---\n- [ ] after");
        assert_eq!(
            read(&mut kept, &closed, 4),
            expected(&closed, Boxes::Status)
        );
        assert_eq!(kept.front_matter_end, Some(4));
        let reopened = format!("{open}--\n- [ ] after");
        assert_eq!(
            read(&mut kept, &reopened, 4),
            expected(&reopened, Boxes::Status)
        );
        assert_eq!(kept.front_matter_end, None);
    }
}
