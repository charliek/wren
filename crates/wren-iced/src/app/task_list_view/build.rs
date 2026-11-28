//! What the task view lists: its listing, built from the task index for
//! the view's scope, its filters, `tasks-me` and today, again after any
//! message that changes one of those.

use iced::Task;
use wren_core::model::ItemRef;
use wren_core::tasks::TaskRecord;
use wren_core::tasks_file::TaskFilters;

use super::{Built, Control, Selected, LIST};
use crate::app::task_list::{self, Entry, Listing, RowKey, ROW_HEIGHT};
use crate::app::tasks_view::Scope;
use crate::app::{App, Message};

/// The selection of the row at `entry`.
pub(super) fn select(listing: &Listing, entry: usize) -> Selected {
    let (group, record) = match listing.entries[entry] {
        Entry::Row { group, record } => (group, record),
        Entry::Header(_) => unreachable!("only rows are selected"),
    };
    Selected {
        key: RowKey::of(&listing.records[record]),
        group: listing.groups[group].key.clone(),
        entry,
    }
}

impl App {
    /// The item the view's scope names, `None` for the workspace; `Err`
    /// when it names one no longer there.
    pub(super) fn scope_item(&self, scope: &Scope) -> Result<Option<ItemRef>, ()> {
        match scope {
            Scope::Workspace => Ok(None),
            scope => self
                .model()
                .and_then(|model| scope.item(model))
                .map(Some)
                .ok_or(()),
        }
    }

    /// The filters the view of `scope` remembers.
    pub(in crate::app) fn task_filters(&self, scope: &Scope) -> TaskFilters {
        match (&self.session, self.scope_item(scope)) {
            (Some(session), Ok(item)) => session.task_filters(item),
            _ => TaskFilters::default(),
        }
    }

    /// A note's title, as the model had it when the listing was built.
    pub(in crate::app) fn note_title(&self, note: wren_core::model::NoteId) -> &str {
        self.task_list.titles.get(&note).map_or("", String::as_str)
    }

    /// Build the list again when what it shows changed; keep the menu only
    /// while the view has the keyboard. After every message, with tasks on.
    /// Returns the scroll that keeps the list's widget where the list is.
    pub(in crate::app) fn sync_task_list(&mut self) -> Task<Message> {
        let Some(scope) = self.task_view().cloned() else {
            self.task_list.menu = None;
            self.task_list.focus = None;
            self.task_list.held.clear();
            self.task_list.describe = None;
            return Task::none();
        };
        if self.task_list.menu.is_some() && !self.task_view_has_keys() {
            self.task_list.menu = None;
        }
        // An edit in place lasts only while its field can have the keyboard.
        if !self.task_view_has_keys()
            || self.task_list.menu.is_some()
            || self.task_list.focus.is_some()
        {
            self.task_list.describe = None;
        }
        let Some(session) = &self.session else {
            return Task::none();
        };
        let revision = session
            .tasks()
            .map_or(0, wren_core::tasks::TaskIndex::revision);
        let workspace = (session.model().root.clone(), self.workspace_generation());
        // Another scope, or another workspace, holds no row of this one.
        if (self.task_list.built.as_ref())
            .is_some_and(|old| old.scope != scope || old.workspace != workspace)
        {
            self.task_list.held.clear();
        }
        // A held task is found again where the index has it now, or let go.
        if !self.task_list.held.is_empty()
            && (self.task_list.built.as_ref()).is_none_or(|old| old.revision != revision)
        {
            if let Some(tasks) = self.session.as_ref().and_then(|session| session.tasks()) {
                let prefix = scope.prefix();
                let list = &mut self.task_list;
                task_list::relocate_holds(
                    &mut list.held,
                    &list.listing.records,
                    tasks.tasks_under(prefix),
                );
            }
        }
        let Some(session) = &self.session else {
            return Task::none();
        };
        let built = Built {
            workspace,
            filters: self.task_filters(&scope),
            scope,
            revision,
            me: self.preferences.settings.tasks_me.clone(),
            today: self.today(),
            held: self.task_list.held.clone(),
        };
        if self.task_list.built.as_ref() == Some(&built) {
            // Asked for nothing that moves a row: nothing to keep in sight.
            self.task_list.reveal = false;
            return Task::none();
        }
        let new_scope = self
            .task_list
            .built
            .as_ref()
            .is_none_or(|old| old.scope != built.scope || old.workspace != built.workspace);
        let records: Vec<TaskRecord> = session
            .tasks()
            .map(|tasks| tasks.tasks_under(built.scope.prefix()).cloned().collect())
            .unwrap_or_default();
        let titles: std::collections::HashMap<_, _> = (session.model().notes.iter())
            .map(|note| (note.id, note.title.clone()))
            .collect();
        let place = |record: &TaskRecord| {
            let title = titles.get(&record.note).map_or("", String::as_str);
            task_list::note_place(&record.path, title)
        };
        let listing = Listing::build_holding(
            records,
            &built.filters,
            &built.me,
            built.today,
            &place,
            &built.held,
        );
        let footer = self.task_list_counts(&built.scope);
        let height = self.list_height();
        let list = &mut self.task_list;
        let before = std::mem::replace(&mut list.listing, listing);
        // Asked by the user, a selected row that was in sight is kept in
        // sight wherever the rebuild puts it, so a key acts on a row the
        // user can see.
        let in_sight = |tops: &[f32], entry: usize, scroll: f32| {
            tops.get(entry)
                .is_some_and(|&top| top >= scroll && top + ROW_HEIGHT <= scroll + height)
        };
        let seen = std::mem::take(&mut list.reveal)
            && (list.selected.as_ref())
                .is_some_and(|selected| in_sight(&before.tops, selected.entry, list.scroll));
        list.titles = titles;
        list.footer = footer;
        list.generation += 1;
        list.built = Some(built);
        if new_scope {
            list.selected = None;
            list.scroll = 0.0;
            list.focus = None;
            list.menu = None;
            list.describe = None;
        }
        // Where the list stays: where it was, as far as the new list goes.
        let scroll = list
            .scroll
            .min((list.listing.height() - list.height.unwrap_or(0.0)).max(0.0));
        // An edit follows its row, and ends when the list no longer shows it.
        self.follow_describe(&before.records, scroll);
        let list = &mut self.task_list;
        let previous = list.selected.take();
        let entry = previous
            .as_ref()
            .and_then(|selected| list.listing.find(&selected.key, Some(&selected.group)))
            .or_else(|| {
                // Gone: the row now at its place, or the last.
                let at = previous.as_ref()?.entry;
                let rows: Vec<usize> = list.listing.rows().collect();
                rows.iter()
                    .copied()
                    .find(|&row| row >= at)
                    .or_else(|| rows.last().copied())
            })
            .or_else(|| list.listing.rows().next());
        list.selected = entry.map(|entry| select(&list.listing, entry));
        // The sources, rows and cells drawn belong to the old entries.
        (list.drawn.borrow_mut()).retain(|(control, _)| {
            !matches!(
                control,
                Control::Source(_) | Control::Row(_) | Control::Cell(..)
            )
        });
        if let Some(entry) =
            entry.filter(|&entry| seen && !in_sight(&list.listing.tops, entry, scroll))
        {
            list.scroll = scroll;
            return self.select_entry(entry);
        }
        if !new_scope && scroll == list.scroll {
            return Task::none();
        }
        // The widget keeps its offset across scopes; the list starts at
        // the top.
        list.scroll = scroll;
        iced::widget::operation::scroll_to(
            LIST,
            iced::widget::operation::AbsoluteOffset { x: 0.0, y: scroll },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn the_list_shows_the_scopes_tasks_in_order_with_the_old_work_hidden() {
        let (directory, app) = fixture();
        let listing = &app.task_list.listing;
        let labels: Vec<&str> = listing.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["Standup", "Roadmap"]);
        assert_eq!(listing.shown, 12, "{:?}", descriptions(&app));
        assert!(!descriptions(&app).contains(&"Archive old docs".to_owned()));
        assert_eq!(selected(&app), "Order badges", "the first row");
        // The hidden note is left out, and said so.
        let (counts, left) = app.task_list_counts(&Scope::Workspace);
        assert_eq!((counts.open_and_doing, counts.overdue), (8, 1));
        assert_eq!(
            left.as_deref(),
            Some("Left out: 1 note (2 tasks, tasks: off)")
        );
        // Opening the view writes nothing.
        assert_eq!(tasks_toml(&directory), None);
    }

    #[test]
    fn the_selection_follows_its_task_through_a_refresh() {
        let (_directory, mut app) = fixture();
        batch(&mut app, &[named(Named::End)]);
        batch(&mut app, &[letter("k"), letter("k")]);
        assert_eq!(selected(&app), "Write the retro");
        control(&mut app, Control::GroupBy);
        batch(&mut app, &[letter("p")]);
        assert_eq!(selected(&app), "Write the retro", "now under Low");
        control(&mut app, Control::Due);
        batch(&mut app, &[letter("o")]);
        // Gone from the list: the row at its place, else the last.
        assert_eq!(descriptions(&app), ["Draft the Q1 plan"]);
        assert_eq!(selected(&app), "Draft the Q1 plan");
    }

    #[test]
    fn another_workspace_never_shows_the_last_ones_list() {
        let (directory, mut app) = fixture();
        assert!(descriptions(&app).contains(&"Draft the Q1 plan".to_owned()));
        // The same notes, one task renamed: its index counts the same
        // revisions as the first's.
        let other = directory.path().join("other");
        copy(Path::new(FIXTURE), &other);
        let roadmap = other.join("Work/Planning/Roadmap.md");
        let text = std::fs::read_to_string(&roadmap).unwrap();
        std::fs::write(&roadmap, text.replace("Q1 plan", "Q2 plan")).unwrap();
        app.session = Some(Session::open_with(&other, app.session_options()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        assert!(app.task_list.selected.is_none(), "the old list is gone");
        let _ = app.dispatch(Command::ShowTasks(Scope::Workspace));
        let _ = app.update(Message::Tick);
        let shown = descriptions(&app);
        assert!(shown.contains(&"Draft the Q2 plan".to_owned()), "{shown:?}");
        assert!(!shown.contains(&"Draft the Q1 plan".to_owned()));
        // Even with the list kept, the key tells the workspaces apart.
        let session = Session::open_with(&root(&directory), app.session_options()).unwrap();
        app.session = Some(session);
        let _ = app.update(Message::Tick);
        assert!(descriptions(&app).contains(&"Draft the Q1 plan".to_owned()));
    }
}
