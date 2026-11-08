use wren_core::index::{fuzzy_score, Hit, QueryKind, Scope};
use wren_core::model::{Parent, SectionId};

use super::App;
use crate::panes::{Layout, Widths};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Filters {
    pub sections: String,
    pub groups: String,
    pub notes: String,
    pub outline: String,
}

impl Filters {
    pub fn set(&mut self, pane: &str, query: String) -> bool {
        let slot = match pane {
            "sections" => &mut self.sections,
            "groups" => &mut self.groups,
            "notes" => &mut self.notes,
            "outline" => &mut self.outline,
            _ => return false,
        };
        *slot = query;
        true
    }
}

impl App {
    pub(super) fn hydrate_navigation(&mut self) {
        self.cancel_gestures();
        let Some(session) = &self.session else {
            return;
        };
        self.layout = Layout::from_state(session.state());
        self.widths = Widths::from_state(session.state());
        self.focus = false;
        self.filters = Filters::default();
        self.expanded.clear();
        self.spring_section = None;
        self.palette = None;
        self.find_open = false;
        self.find_query.clear();
        self.editor.set_find_query("");
        if let Some(section) = self.selected_section {
            self.expanded.insert(section);
        }
    }

    pub(super) fn reveal_selection(&mut self) {
        let Some(model) = self.model() else {
            return;
        };
        let section = self.selected_section.and_then(|id| model.section(id));
        let group = self.selected_group.and_then(|id| model.group(id));
        let sections_conflict =
            section.is_some_and(|section| !matches_name(&self.filters.sections, &section.name));
        let groups_conflict =
            group.is_some_and(|group| !matches_name(&self.filters.groups, &group.name));
        let outline_conflict = section
            .is_some_and(|section| !matches_name(&self.filters.outline, &section.name))
            && group.is_none_or(|group| !matches_name(&self.filters.outline, &group.name));
        let notes_conflict = self
            .selected_note
            .is_some_and(|note| !self.note_hits().iter().any(|hit| hit.note == note));
        if sections_conflict {
            self.filters.sections.clear();
        }
        if groups_conflict {
            self.filters.groups.clear();
        }
        if outline_conflict {
            self.filters.outline.clear();
        }
        if notes_conflict {
            self.filters.notes.clear();
        }
        if let Some(section) = self.selected_section {
            self.expanded.insert(section);
        }
    }

    pub(super) fn note_hits(&self) -> Vec<Hit> {
        if let Some(loading) = &self.loading {
            return loading.note_hits.clone();
        }
        let (Some(session), Some(Parent::Group(group))) = (&self.session, self.browser_parent())
        else {
            return Vec::new();
        };
        session.index().query(
            &self.filters.notes,
            Scope::Group(group),
            QueryKind::Filter,
            usize::MAX,
        )
    }

    pub(super) fn browser_section(&self) -> Option<SectionId> {
        self.spring_section.or(self.selected_section)
    }

    pub(super) fn browser_parent(&self) -> Option<Parent> {
        if let Some(section) = self.spring_section {
            Some(Parent::Section(section))
        } else {
            self.selected_parent()
        }
    }

    pub(super) fn outline_sections(&self) -> Vec<(SectionId, bool)> {
        let Some(model) = self.model() else {
            return Vec::new();
        };
        let query = self.filters.outline.trim();
        model
            .sections
            .iter()
            .filter_map(|section| {
                let matches_section = matches_name(query, &section.name);
                let matches_group = model
                    .groups
                    .iter()
                    .any(|group| group.section == section.id && matches_name(query, &group.name));
                if !matches_section && !matches_group {
                    return None;
                }
                Some((
                    section.id,
                    self.expanded.contains(&section.id)
                        || (!query.is_empty() && (matches_section || matches_group))
                        || self.spring_section == Some(section.id),
                ))
            })
            .collect()
    }

    pub(super) fn note_sort(&self) -> &str {
        if let Some(loading) = &self.loading {
            return &loading.note_sort;
        }
        self.session
            .as_ref()
            .and_then(|session| {
                self.selected_group
                    .and_then(|group| session.model().group(group))
                    .map(|group| session.state().sort(&group.path))
            })
            .unwrap_or("modified")
    }
}

pub(super) fn matches_name(query: &str, name: &str) -> bool {
    fuzzy_score(query.trim(), name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_filters_preserve_siblings_and_unknown_panes_are_rejected() {
        let mut filters = Filters::default();
        assert!(filters.set("sections", "in".into()));
        assert!(filters.set("notes", "text".into()));
        assert!(!filters.set("editor", "lost".into()));
        assert_eq!(filters.sections, "in");
        assert_eq!(filters.notes, "text");
        assert_eq!(filters.groups, "");
    }
}
