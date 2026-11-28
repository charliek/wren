//! The app's copies of the keys the task view already read.

use crate::app::App;

/// The app's own copy of a key the view reported, which its key handling
/// hears after the batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum AppCopy {
    Escape,
    Shortcut(wren_core::keybind::Accel),
}

/// The app's copies still to come of the keys the view already read (its
/// list, the menu over it, a row's description field), in input order.
/// Whatever such a key does is done at its place in its batch, so its
/// copy is always dropped: matched to its report by order, as iced hands
/// a widget's report and the app's copy over separately.
#[derive(Debug, Default)]
pub(in crate::app) struct Copies(std::collections::VecDeque<AppCopy>);

impl Copies {
    /// A key the view read, whose copy the app will hear.
    pub(in crate::app) fn push(&mut self, copy: AppCopy) {
        self.0.push_back(copy);
    }

    /// Whether the app's copy `copy` is the next one expected, which is
    /// then spent; a copy of a key the view never read is the app's.
    pub(in crate::app) fn take(&mut self, copy: &AppCopy) -> bool {
        let next = self.0.front() == Some(copy);
        if next {
            self.0.pop_front();
        }
        next
    }

    /// [`Copies::take`] for the app's copy of the shortcut `accel`, read
    /// without copying it.
    pub(in crate::app) fn take_shortcut(&mut self, accel: &wren_core::keybind::Accel) -> bool {
        let next = matches!(self.0.front(), Some(AppCopy::Shortcut(front)) if front == accel);
        if next {
            self.0.pop_front();
        }
        next
    }

    pub(in crate::app) fn clear(&mut self) {
        self.0.clear();
    }

    #[cfg(test)]
    pub(in crate::app) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[cfg(test)]
    pub(in crate::app) fn len(&self) -> usize {
        self.0.len()
    }
}

impl App {
    /// Whether the app's copy `copy` is of a key the view reported, and so
    /// dropped ([`Copies::take`]).
    pub(in crate::app) fn take_task_list_copy(&mut self, copy: &AppCopy) -> bool {
        self.task_list.copies.take(copy)
    }

    /// [`App::take_task_list_copy`] for the app's copy of the shortcut
    /// `accel`, read without copying it.
    pub(in crate::app) fn take_task_list_shortcut(
        &mut self,
        accel: &wren_core::keybind::Accel,
    ) -> bool {
        self.task_list.copies.take_shortcut(accel)
    }
}

#[cfg(test)]
mod tests {
    use wren_core::keybind::{Accel, Mods};

    use super::*;

    fn accel(key: &str) -> Accel {
        Accel {
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
            key: key.into(),
        }
    }

    /// Each copy is spent by its own report, in order: one that is not
    /// the next expected is the app's, and spends nothing.
    #[test]
    fn copies_are_spent_in_the_order_their_keys_were_read() {
        let mut copies = Copies::default();
        copies.push(AppCopy::Shortcut(accel("k")));
        copies.push(AppCopy::Escape);
        copies.push(AppCopy::Shortcut(accel("[")));
        assert!(!copies.take(&AppCopy::Escape), "not the next one");
        assert!(!copies.take_shortcut(&accel("[")), "not the next one");
        assert_eq!(copies.len(), 3);
        assert!(copies.take_shortcut(&accel("k")));
        assert!(!copies.take_shortcut(&accel("k")), "spent once");
        assert!(copies.take(&AppCopy::Escape));
        assert!(copies.take(&AppCopy::Shortcut(accel("["))));
        assert!(copies.is_empty());
        assert!(!copies.take(&AppCopy::Escape), "a copy of no report");
    }
}
