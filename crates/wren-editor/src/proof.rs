//! Spell check's seam: the host's [`Proofreader`], the suggestions menu
//! ([`SpellMenu`]) and the events that drive it ([`SpellEvent`]).

mod menu;

use std::ops::Range;

use iced::Point;
#[cfg(any(test, feature = "test-support"))]
use iced::Rectangle;

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

/// An input to the suggestions menu; see [`crate::EditorView::spell`].
/// `OpenAt` is relative to the text origin, as a click action's position is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpellEvent {
    /// Open the menu for the misspelled word at the caret.
    OpenAtCaret,
    /// Open the menu for the misspelled word under a right-click.
    OpenAt(Point),
    /// Move the selection by this many rows.
    Move(i32),
    /// The pointer is over this row.
    Hover(usize),
    /// Replace the word with this row's suggestion.
    Accept(usize),
    /// Replace the word with the selected row's suggestion, or close a menu
    /// with nothing selected. Enter sends it, so it accepts the row the
    /// host's menu holds even when keys of one batch were routed before the
    /// host had opened it.
    AcceptSelected,
    /// A press of Up, Down or Enter with no modifier, made after an opening
    /// earlier in the same batch of events, when the widget cannot know
    /// whether the host's opening found a word. An open menu takes it as
    /// [`MenuKey::event`]; with none open the editor handles the press as it
    /// would any other.
    Key(MenuKey),
    Close,
}

/// A key the open menu takes for itself (rule 4.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Enter,
}

impl MenuKey {
    /// What the key does to an open menu: Up and Down move its selection,
    /// and Enter accepts the selected row.
    pub fn event(self) -> SpellEvent {
        match self {
            Self::Up => SpellEvent::Move(-1),
            Self::Down => SpellEvent::Move(1),
            Self::Enter => SpellEvent::AcceptSelected,
        }
    }
}

/// The open suggestions menu: the word it was opened for and what it offers.
#[derive(Debug, Clone, PartialEq)]
pub struct SpellMenu {
    line: usize,
    mark: Range<usize>,
    word: String,
    items: Vec<String>,
    selected: Option<usize>,
}

pub(crate) use menu::{fit, height, measure, place, Fit, MenuOverlay};

/// The most suggestions the menu shows (rule 4.4.2).
pub(crate) const SUGGESTIONS: usize = 5;

impl SpellMenu {
    /// A menu for `word`, at `mark` of `line`, offering `items`, the first
    /// selected.
    pub(crate) fn new(line: usize, mark: Range<usize>, word: String, items: Vec<String>) -> Self {
        Self {
            line,
            mark,
            word,
            selected: (!items.is_empty()).then_some(0),
            items,
        }
    }

    /// Move the selection `delta` rows, stopping at the ends.
    pub(crate) fn step(&mut self, delta: i32) {
        self.selected = stepped(self.selected, self.items.len(), delta);
    }

    /// Select `row`, if it holds a suggestion.
    pub(crate) fn hover(&mut self, row: usize) {
        self.selected = hovered(self.selected, self.items.len(), row);
    }

    /// The line that holds the word.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The word's byte range in its line.
    pub fn mark(&self) -> Range<usize> {
        self.mark.clone()
    }

    /// The word as it was when the menu opened.
    pub fn word(&self) -> &str {
        &self.word
    }

    /// The suggestions, best first; empty when there are none.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// The selected row, if any row can be selected.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
}

/// `selected` moved `delta` rows among `items`, stopping at the ends; a menu
/// with nothing selected stays so.
fn stepped(selected: Option<usize>, items: usize, delta: i32) -> Option<usize> {
    let selected = selected?;
    let last = items.saturating_sub(1);
    let moved = i64::try_from(selected)
        .unwrap_or(i64::MAX)
        .saturating_add(i64::from(delta));
    Some(usize::try_from(moved.max(0)).map_or(last, |row| row.min(last)))
}

/// The selection once the pointer is over `row`: that row, if it is one of
/// `items`.
fn hovered(selected: Option<usize>, items: usize, row: usize) -> Option<usize> {
    if row < items {
        Some(row)
    } else {
        selected
    }
}

/// The open suggestions menu as the editor widget sees it while it handles
/// a batch of events.
///
/// iced hands every event queued since the last frame to the same widget,
/// the menu's overlay first, before the host applies any message the widget
/// published. So the widget keeps its own copy, starting from the menu the
/// host handed it and changed by each message it publishes as the host will
/// change its menu, and routes keys and presses against that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenMenu {
    /// The selected row; `None` when the menu has nothing to offer.
    pub selected: Option<usize>,
    /// How many suggestions it has.
    pub items: usize,
    /// Opened by a message of this batch. The host opens it against the
    /// caret and the text as the batch's earlier messages leave them, which
    /// the widget does not have, so whether it opens and what it offers are
    /// unknown until the next batch: the menu's keys go to the host as
    /// [`SpellEvent::Key`], which it resolves against its real menu.
    pub opening: bool,
}

impl OpenMenu {
    /// A menu opened in this batch; see [`Self::opening`].
    const OPENING: Self = Self {
        selected: None,
        items: 0,
        opening: true,
    };

    pub(crate) fn of(menu: &SpellMenu) -> Self {
        Self {
            selected: menu.selected(),
            items: menu.items().len(),
            opening: false,
        }
    }

    /// The menu `message` leaves `menu` as, once the host applies it. An
    /// opening may or may not open a menu ([`Self::opening`]). A move or a
    /// hover moves the selection, and a menu key acts as it does on an open
    /// menu. Every other message closes the menu: the host closes it on any
    /// action, intent, undo or redo (rule 4.4.8), and with no menu open a
    /// menu key leaves none.
    pub(crate) fn after(menu: Option<Self>, message: &crate::Message) -> Option<Self> {
        match message {
            crate::Message::Spell(SpellEvent::OpenAtCaret | SpellEvent::OpenAt(_)) => {
                Some(Self::OPENING)
            }
            crate::Message::Spell(SpellEvent::Key(key)) => {
                Self::after(menu, &crate::Message::Spell(key.event()))
            }
            crate::Message::Spell(SpellEvent::Move(delta)) => menu.map(|menu| Self {
                selected: stepped(menu.selected, menu.items, *delta),
                ..menu
            }),
            crate::Message::Spell(SpellEvent::Hover(row)) => menu.map(|menu| Self {
                selected: hovered(menu.selected, menu.items, *row),
                ..menu
            }),
            _ => None,
        }
    }
}

/// Publish `message`, keeping `menu`, the widget's copy, as the host will
/// leave its own once it applies it.
pub(crate) fn publish(
    shell: &mut iced::advanced::Shell<'_, crate::Message>,
    menu: &std::cell::Cell<Option<OpenMenu>>,
    message: crate::Message,
) {
    menu.set(OpenMenu::after(menu.get(), &message));
    shell.publish(message);
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
