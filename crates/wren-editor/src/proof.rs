//! Spell check's seam: the host's [`Proofreader`], and the suggestions menu
//! ([`SpellMenu`]), which is a [`Popover`] driven by the popover's events
//! ([`SpellEvent`]).

use std::ops::{Deref, DerefMut, Range};

#[cfg(test)]
use iced::advanced::mouse::click::Kind;
#[cfg(test)]
use iced::Point;
#[cfg(any(test, feature = "test-support"))]
use iced::Rectangle;

#[cfg(test)]
use crate::popover::{click_kind, OpenMenu};
pub use crate::popover::{MenuKey, MenuPress};
use crate::popover::{Mode, Popover, Row};

/// What the host gives the editor to find and correct mistakes. Line by line,
/// so the editor asks only about rows it draws.
pub trait Proofreader: Send + Sync {
    /// The mistakes of `line` that start and end inside `prose`, as byte
    /// ranges of `line`: sorted, apart, on character boundaries. Appends to
    /// `out`.
    fn marks(&self, line: &str, prose: &[Range<usize>], out: &mut Vec<Range<usize>>);
    /// Up to `limit` corrections for the mistake at `mark` in `line`, best
    /// first.
    fn suggestions(&self, line: &str, mark: Range<usize>, limit: usize) -> Vec<String>;
}

/// The popover's events, by the name of its first user, the suggestions
/// menu.
pub type SpellEvent = crate::popover::PopoverEvent;

/// The open suggestions menu: a menu-mode [`Popover`] anchored to the word
/// it was opened for, which stays tinted, offering the word's suggestions.
#[derive(Debug, Clone, PartialEq)]
pub struct SpellMenu(Popover);

/// The most suggestions the menu shows (rule 4.4.2).
pub(crate) const SUGGESTIONS: usize = 5;

impl SpellMenu {
    /// A menu for `word`, at `mark` of `line`, offering `items`, the first
    /// selected.
    pub(crate) fn new(line: usize, mark: Range<usize>, word: String, items: Vec<String>) -> Self {
        let rows = (0..).zip(items).map(|(id, item)| Row::new(id, item));
        Self(Popover::new(Mode::Menu, line, mark, word, rows.collect()).spelling())
    }

    /// The menu as the `instance`th popover the editor opens.
    pub(crate) fn opened_as(self, instance: u64) -> Self {
        Self(self.0.opened_as(instance))
    }

    /// The line that holds the word.
    pub fn line(&self) -> usize {
        self.0.line()
    }

    /// The word's byte range in its line.
    pub fn mark(&self) -> Range<usize> {
        self.0.range()
    }

    /// The word as it was when the menu opened.
    pub fn word(&self) -> &str {
        self.0.text()
    }

    /// The suggestions, best first; empty when there are none.
    pub fn items(&self) -> &[String] {
        self.0.labels()
    }

    /// The selected row, if any row can be selected.
    pub fn selected(&self) -> Option<usize> {
        self.0.selected()
    }
}

impl Deref for SpellMenu {
    type Target = Popover;

    fn deref(&self) -> &Popover {
        &self.0
    }
}

impl DerefMut for SpellMenu {
    fn deref_mut(&mut self) -> &mut Popover {
        &mut self.0
    }
}

/// The marks and the menu as the last frame drew them, for the host's
/// functional tests.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpellFrame {
    pub marks: Vec<DrawnMark>,
    /// The menu's bounds in window coordinates, when it was drawn.
    pub menu: Option<Rectangle>,
}

/// One mark of the drawn rows.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnMark {
    pub line: usize,
    /// The word's byte range in its line.
    pub range: Range<usize>,
    /// Whether the zigzag was drawn, or hidden by the caret at the word's end.
    pub shown: bool,
    /// The zigzag's extent on each drawn row, in window coordinates and
    /// clipped to the rows, whether or not it was shown.
    pub rects: Vec<Rectangle>,
}

/// The marks `proofreader` finds in `line`'s `prose`. A line with no prose is
/// not asked about, and a range that is empty or not on character boundaries
/// of `line` is dropped, so callers may slice `line` with every mark.
pub(crate) fn line_marks(
    proofreader: &dyn Proofreader,
    line: &str,
    prose: &[Range<usize>],
) -> Vec<Range<usize>> {
    let mut marks = Vec::new();
    if prose.is_empty() {
        return marks;
    }
    proofreader.marks(line, prose, &mut marks);
    marks.retain(|mark| !mark.is_empty() && line.get(mark.clone()).is_some());
    marks
}

/// What the menu offers for `word` out of the proofreader's `suggestions`:
/// at most [`SUGGESTIONS`], in order, without duplicates, and with each `'`
/// written `’` when `word` holds one (rule 4.4.6). A suggestion is one piece
/// of a line, so an empty one or one holding a line break is dropped.
pub(crate) fn offered(word: &str, suggestions: Vec<String>) -> Vec<String> {
    let curly = word.contains('\u{2019}');
    let mut items: Vec<String> = Vec::with_capacity(SUGGESTIONS);
    for suggestion in suggestions {
        if items.len() == SUGGESTIONS {
            break;
        }
        if suggestion.is_empty() || suggestion.contains(['\r', '\n']) {
            continue;
        }
        let item = if curly {
            suggestion.replace('\'', "\u{2019}")
        } else {
            suggestion
        };
        if !items.contains(&item) {
            items.push(item);
        }
    }
    items
}

/// The mark the keyboard opens the menu for (rule 4.4.1): the first that
/// contains or touches the caret at byte `caret`.
pub(crate) fn mark_at_caret(marks: Vec<Range<usize>>, caret: usize) -> Option<Range<usize>> {
    marks
        .into_iter()
        .find(|mark| mark.start <= caret && caret <= mark.end)
}

/// Whether a caret at byte `caret` of `line` hides `mark` (rule 4.3.3): it
/// sits at the mark's end, or only apostrophes lie between them, so the word
/// being typed is not flagged before it is finished.
pub(crate) fn caret_hides(line: &str, mark: &Range<usize>, caret: usize) -> bool {
    caret >= mark.end
        && line
            .get(mark.end..caret)
            .is_some_and(|gap| gap.chars().all(|ch| matches!(ch, '\'' | '\u{2019}')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::Rejects;

    /// Hands back fixed ranges, whatever it is asked.
    struct Fixed(Vec<Range<usize>>);

    impl Proofreader for Fixed {
        fn marks(&self, _line: &str, _prose: &[Range<usize>], out: &mut Vec<Range<usize>>) {
            out.extend(self.0.iter().cloned());
        }

        fn suggestions(&self, _line: &str, _mark: Range<usize>, _limit: usize) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn a_line_with_no_prose_is_not_asked_about() {
        let proofreader = Rejects::new(&["teh"]);
        assert!(line_marks(&proofreader, "teh", &[]).is_empty());
        assert_eq!(proofreader.calls(), 0);
        let word = 0..3;
        let marks = line_marks(&proofreader, "teh", std::slice::from_ref(&word));
        assert_eq!(marks, std::slice::from_ref(&word));
        assert_eq!(proofreader.calls(), 1);
    }

    #[test]
    fn marks_off_the_line_or_inside_a_character_are_dropped() {
        let line = "né teh";
        let proofreader = Fixed(vec![0..3, 2..3, 4..4, 4..7, 4..9, 8..8]);
        let prose = 0..line.len();
        assert_eq!(
            line_marks(&proofreader, line, std::slice::from_ref(&prose)),
            [0..3, 4..7]
        );
    }

    #[test]
    fn the_keyboard_targets_the_first_mark_containing_or_touching_the_caret() {
        let marks = || vec![0..3, 4..7, 7..9];
        for (caret, target) in [
            (0, Some(0..3)),
            (2, Some(0..3)),
            (3, Some(0..3)),
            (4, Some(4..7)),
            (7, Some(4..7)),
            (8, Some(7..9)),
            (9, Some(7..9)),
            (10, None),
        ] {
            assert_eq!(mark_at_caret(marks(), caret), target, "{caret}");
        }
        assert_eq!(mark_at_caret(Vec::new(), 0), None);
    }

    #[test]
    fn a_new_menu_selects_its_first_suggestion_if_it_has_one() {
        let menu = SpellMenu::new(2, 4..7, "teh".into(), vec!["the".into()]);
        assert_eq!(
            (menu.line(), menu.mark(), menu.word(), menu.selected()),
            (2, 4..7, "teh", Some(0))
        );
        let empty = SpellMenu::new(0, 0..3, "zzz".into(), Vec::new());
        assert_eq!(empty.selected(), None);
        assert!(empty.items().is_empty());
    }

    #[test]
    fn the_menu_offers_five_distinct_one_line_suggestions_in_order() {
        let given = |items: &[&str]| items.iter().map(|&item| item.to_owned()).collect();
        assert_eq!(
            offered(
                "teh",
                given(&["the", "", "the", "a\nb", "c\rd", "tea", "ten"])
            ),
            ["the", "tea", "ten"]
        );
        assert_eq!(
            offered("teh", given(&["a", "b", "c", "d", "e", "f"])),
            ["a", "b", "c", "d", "e"]
        );
        // Duplicates do not use up the five.
        assert_eq!(
            offered("teh", given(&["a", "a", "b", "c", "d", "e"])),
            ["a", "b", "c", "d", "e"]
        );
        assert!(offered("teh", Vec::new()).is_empty());
    }

    #[test]
    fn a_word_written_with_a_curly_apostrophe_is_offered_curly_ones() {
        let given = |items: &[&str]| items.iter().map(|&item| item.to_owned()).collect();
        assert_eq!(
            offered("dosen’t", given(&["doesn't", "dozen", "doesn’t", "o'er'"])),
            ["doesn’t", "dozen", "o’er’"]
        );
        // A straight one, or none, leaves the suggestions as written.
        assert_eq!(
            offered("dosen't", given(&["doesn't", "doesn’t"])),
            ["doesn't", "doesn’t"]
        );
        assert_eq!(offered("teh", given(&["t'he"])), ["t'he"]);
    }

    #[test]
    fn a_press_after_one_that_only_closed_a_menu_is_a_single_click() {
        use Kind::{Double, Single, Triple};
        for raw in [Single, Double, Triple] {
            assert_eq!(click_kind(raw, None), Single);
        }
        // After a click, iced's own count stands.
        for previous in [Single, Double, Triple] {
            assert_eq!(click_kind(Single, Some(previous)), Single);
        }
        assert_eq!(click_kind(Double, Some(Single)), Double);
        assert_eq!(click_kind(Triple, Some(Double)), Triple);
        assert_eq!(click_kind(Double, Some(Triple)), Double);
        // Counted from the press the host took, not iced's count.
        assert_eq!(click_kind(Triple, Some(Single)), Double);
        assert_eq!(click_kind(Double, Some(Double)), Triple);
    }

    #[test]
    fn the_widgets_copy_follows_each_message_as_the_hosts_menu_will() {
        use crate::Message;
        let open = |selected| {
            Some(OpenMenu {
                selected,
                items: 3,
                opening: false,
            })
        };
        let spell = |event| Message::Spell(event);
        let after = |menu, event| OpenMenu::after(menu, &spell(event));
        assert_eq!(after(open(Some(0)), SpellEvent::Move(1)), open(Some(1)));
        assert_eq!(after(open(Some(2)), SpellEvent::Move(1)), open(Some(2)));
        assert_eq!(after(open(Some(0)), SpellEvent::Move(-1)), open(Some(0)));
        assert_eq!(after(open(None), SpellEvent::Move(1)), open(None));
        assert_eq!(after(open(Some(0)), SpellEvent::Hover(2)), open(Some(2)));
        assert_eq!(after(open(Some(0)), SpellEvent::Hover(3)), open(Some(0)));
        assert_eq!(after(None, SpellEvent::Move(1)), None);
        assert_eq!(after(open(Some(1)), SpellEvent::Accept(1)), None);
        assert_eq!(after(open(Some(1)), SpellEvent::AcceptSelected), None);
        assert_eq!(after(open(Some(1)), SpellEvent::Close), None);
        let opening = Some(OpenMenu::OPENING);
        assert_eq!(after(None, SpellEvent::OpenAtCaret), opening);
        assert_eq!(
            after(open(Some(2)), SpellEvent::OpenAt(Point::ORIGIN)),
            opening
        );
        // A menu key acts as on an open menu; on one opened in this batch,
        // the opening stays unknown until Enter closes it.
        assert_eq!(
            after(open(Some(0)), SpellEvent::Key(MenuKey::Down)),
            open(Some(1))
        );
        assert_eq!(after(opening, SpellEvent::Key(MenuKey::Down)), opening);
        assert_eq!(after(opening, SpellEvent::Key(MenuKey::Up)), opening);
        assert_eq!(after(opening, SpellEvent::Key(MenuKey::Enter)), None);
        assert_eq!(after(None, SpellEvent::Key(MenuKey::Down)), None);
        // A left press leaves no menu whatever the host had; a right press
        // may open one.
        let single = iced::advanced::mouse::click::Kind::Single;
        assert_eq!(
            after(
                opening,
                SpellEvent::Press(MenuPress::Left(Point::ORIGIN, single))
            ),
            None
        );
        assert_eq!(
            after(opening, SpellEvent::Press(MenuPress::Right(Point::ORIGIN))),
            opening
        );
        // Every other message closes it, as the host closes its menu.
        for message in [
            Message::Undo,
            Message::Redo,
            Message::Find,
            Message::Intent(crate::Intent::Enter),
            Message::Action(iced::widget::text_editor::Action::SelectAll),
        ] {
            assert_eq!(
                OpenMenu::after(open(Some(0)), &message),
                None,
                "{message:?}"
            );
        }
    }

    #[test]
    fn only_the_caret_at_a_marks_end_or_past_its_apostrophes_hides_it() {
        let line = "wouldn'’' x teh";
        let mark = 0..6;
        for (caret, hides) in [
            (6, true),
            (7, true),
            (10, true),
            (11, true),
            (5, false),
            (0, false),
            // Past the space, or inside the curly apostrophe.
            (12, false),
            (8, false),
            (99, false),
        ] {
            assert_eq!(caret_hides(line, &mark, caret), hides, "{caret}");
        }
        assert!(caret_hides(line, &(12..15), 15));
    }
}
