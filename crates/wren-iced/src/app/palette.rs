use std::ops::Range;
use std::path::PathBuf;

use iced::widget::text_editor::{Cursor, Position};
use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input,
};
use iced::{Element, Fill, Task};
use wren_core::index::{fuzzy_score, QueryKind, Scope};
use wren_core::model::{GroupId, ItemRef, NoteId, Parent, SectionId};
use wren_core::ops::Op;
use wren_editor::{Placement, Viewport};

use super::board::Surface;
use super::commands::{
    item_commands, Command, CommandId, Disposition, Gateway, ItemTarget, ReorderTarget,
    DEFINITIONS, GROUPS,
};
use super::{App, Message};
use crate::ellipsis::ellipsis;
use crate::{chrome, fonts, palette_scroll, theme};
use wren_core::theme as definitions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Commands,
    Jump,
    /// ⌘P from the Sections column.
    JumpSection,
    /// ⌘P from the Groups column: every group, the selected section's first.
    JumpGroup,
    Search,
    Workspaces,
    MoveNote(NoteId),
    MoveGroup(GroupId),
    ItemActions(ItemRef),
    Reorder(ItemRef),
    Themes,
    Fonts,
    /// The open note's headings.
    Outline,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Jump => "jump",
            Self::JumpSection => "jump_section",
            Self::JumpGroup => "jump_group",
            Self::Search => "search",
            Self::Workspaces => "workspaces",
            Self::MoveNote(_) => "move_note",
            Self::MoveGroup(_) => "move_group",
            Self::ItemActions(_) => "item_actions",
            Self::Reorder(_) => "reorder",
            Self::Themes => "themes",
            Self::Fonts => "fonts",
            Self::Outline => "outline",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::Commands => "Type a command…",
            Self::Jump => "Jump to a note…",
            Self::JumpSection => "Jump to a section…",
            Self::JumpGroup => "Jump to a group…",
            Self::Search => "Search all notes in this workspace",
            Self::Workspaces => "Find a workspace…",
            Self::MoveNote(_) | Self::MoveGroup(_) => "Choose a destination…",
            Self::ItemActions(_) => "Choose an action for this item…",
            Self::Reorder(_) => "Choose a position among all siblings…",
            Self::Themes => "Choose a theme…",
            Self::Fonts => "Choose an editor font…",
            Self::Outline => "Jump to a heading…",
        }
    }

    fn chip(self) -> Option<&'static str> {
        match self {
            Self::Workspaces => Some("Switch workspace"),
            Self::MoveNote(_) => Some("Move note"),
            Self::MoveGroup(_) => Some("Move group"),
            Self::ItemActions(_) => Some("Item actions"),
            Self::Reorder(_) => Some("Reorder"),
            Self::Jump => Some("Jump"),
            Self::JumpSection => Some("Sections"),
            Self::JumpGroup => Some("Groups"),
            Self::Themes => Some("Theme"),
            Self::Fonts => Some("Editor font"),
            Self::Outline => Some("Outline"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Command(CommandId),
    Note(NoteId),
    Section(SectionId),
    Group(GroupId),
    Destination(Parent),
    ItemAction {
        target: ItemTarget,
        id: CommandId,
    },
    Reorder(ReorderTarget),
    Workspace(PathBuf),
    /// `None` follows the system appearance.
    Theme(Option<&'static str>),
    /// An installed font family.
    Font(&'static str),
    /// A heading of the open note, on `line` at editor revision `revision`.
    Heading {
        revision: u64,
        line: usize,
    },
}

#[derive(Debug, Clone)]
pub struct SearchTarget {
    pub generation: u64,
    pub index_revision: u64,
    pub note: NoteId,
    pub query: String,
    pub content_match: Option<Range<usize>>,
}

#[derive(Debug, Clone)]
pub enum Action {
    Command(CommandId),
    Note(SearchTarget),
    /// Select and move one column right, like ↵ in the browser.
    Section(SectionId),
    Group(GroupId),
    Destination(Parent),
    ItemCommand {
        target: ItemTarget,
        id: CommandId,
    },
    Reorder(ReorderTarget),
    Workspace(PathBuf),
    Theme(Option<&'static str>),
    Font(&'static str),
    Heading {
        revision: u64,
        line: usize,
    },
}

/// How an outline row draws its heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadingRow {
    pub level: u8,
    /// Levels below the note's top heading level, for the indent.
    pub depth: u8,
}

#[derive(Debug, Clone)]
pub struct ResultRow {
    pub key: Key,
    pub title: String,
    pub path: String,
    pub snippet: String,
    pub highlight: Option<Range<usize>>,
    pub content_hit: bool,
    pub group: &'static str,
    pub shortcut: String,
    pub current: bool,
    pub enabled: bool,
    pub action: Action,
    pub heading: Option<HeadingRow>,
}

/// Where the editor was when the outline opened, so leaving the outline any
/// way but choosing a heading puts it back.
pub struct OutlineOrigin {
    viewport: Viewport,
    note: Option<NoteId>,
    /// The `edit_revision` that `headings` were built from.
    revision: u64,
    /// The note's headings, so a query keystroke only filters them.
    headings: Vec<OutlineHeading>,
    /// The highlighted row as of the last change the editor acted on. It
    /// starts as the row the outline opens on, which only a highlight move
    /// or Enter shows.
    highlighted: Option<Key>,
    /// Whether the editor still shows the origin: no row has been previewed,
    /// or none has been highlighted since.
    at_origin: bool,
}

/// One heading of the open note, as the outline lists it.
struct OutlineHeading {
    title: String,
    /// The headings it sits under, outermost first.
    path: String,
    level: u8,
    /// Levels below the note's top heading level.
    depth: u8,
    line: usize,
    /// Whether the outline's origin is in its section.
    current: bool,
}

pub struct Palette {
    pub mode: Mode,
    pub generation: u64,
    pub query: String,
    pub results: Vec<ResultRow>,
    pub active: Option<usize>,
    pub revision: u64,
    pub error: Option<String>,
    pub workspace_removal: bool,
    pub source: Option<ItemTarget>,
}

impl Palette {
    fn new(mode: Mode, generation: u64) -> Self {
        Self {
            mode,
            generation,
            query: String::new(),
            results: Vec::new(),
            active: None,
            revision: u64::MAX,
            error: None,
            workspace_removal: false,
            source: None,
        }
    }

    fn replace(&mut self, mut results: Vec<ResultRow>, reset: bool) {
        let selected = (!reset)
            .then(|| {
                self.active
                    .and_then(|index| self.results.get(index))
                    .cloned()
            })
            .flatten();
        let retained = selected.as_ref().and_then(|selected| {
            results
                .iter()
                .position(|row| row.key == selected.key)
                // Outline keys name an editor revision; after an edit the same
                // heading has a new key.
                .or_else(|| {
                    (self.mode == Mode::Outline)
                        .then(|| same_heading(&self.results, selected, &results))
                        .flatten()
                })
        });
        // Enter must not turn a removed insertion anchor into a different action.
        if retained.is_none() && matches!(self.mode, Mode::Reorder(_) | Mode::ItemActions(_)) {
            if let Some(mut selected) = selected {
                selected.enabled = false;
                let index = self.active.unwrap_or(0).min(results.len());
                results.insert(index, selected);
                self.active = Some(index);
                self.error = Some(
                    "The selected item or position no longer exists in its original parent".into(),
                );
                self.results = results;
                return;
            }
        }
        // A picker of settings opens on the saved choice, and the outline on
        // the cursor's section, so browsing starts from it and the live
        // preview begins only when the highlight moves.
        let saved = (matches!(self.mode, Mode::Themes | Mode::Fonts | Mode::Outline)
            && self.query.trim().is_empty())
        .then(|| results.iter().position(|row| row.current))
        .flatten();
        self.active = retained
            .or(saved)
            .or_else(|| (!results.is_empty()).then_some(0));
        self.results = results;
    }

    pub fn step(&mut self, delta: i32) {
        if self.results.is_empty() {
            self.active = None;
            return;
        }
        let current = self.active.unwrap_or(0) as i64;
        self.active =
            Some((current + i64::from(delta)).clamp(0, self.results.len() as i64 - 1) as usize);
    }
}

impl App {
    pub(super) fn workspace_generation(&self) -> u64 {
        self.session
            .as_ref()
            .map(|session| session.generation())
            .or_else(|| self.loading.as_ref().map(|loading| loading.generation))
            .unwrap_or(0)
    }

    pub(super) fn open_palette(&mut self, mode: Mode) -> Gateway {
        if self.dialog.is_some() || self.loading.is_some() {
            return self.blocked("Close the dialog or wait for the workspace to finish opening");
        }
        if !matches!(
            mode,
            Mode::Commands | Mode::Workspaces | Mode::Themes | Mode::Fonts | Mode::Outline
        ) && self.session.is_none()
        {
            return self.blocked("Open a workspace first");
        }
        if mode == Mode::Outline
            && (self.ensure_editor_ready().is_err() || self.surface() != Surface::Text)
        {
            return self.blocked("Open a note to see its outline");
        }
        if matches!(mode, Mode::MoveNote(note) if self.model().and_then(|model| model.note(note)).is_none())
            || matches!(mode, Mode::MoveGroup(group) if self.model().and_then(|model| model.group(group)).is_none())
        {
            return self.blocked("The item no longer exists");
        }
        let item = match mode {
            Mode::MoveNote(note) => Some(ItemRef::Note(note)),
            Mode::MoveGroup(group) => Some(ItemRef::Group(group)),
            Mode::ItemActions(item) | Mode::Reorder(item) => Some(item),
            _ => None,
        };
        let source = item.and_then(|item| self.item_target(item));
        if item.is_some() && source.is_none() {
            return self.blocked("The item no longer exists");
        }
        if matches!(mode, Mode::Reorder(ItemRef::Note(_))) {
            return self.blocked("Only sections and groups can be reordered");
        }
        self.settle_resize();
        self.cancel_gestures();
        self.editor.break_group();
        if mode == Mode::Outline {
            if let Some(origin) = self.outline_origin.take() {
                self.return_to_origin(origin);
            }
            let viewport = self.editor.viewport();
            self.outline_origin = Some(OutlineOrigin {
                viewport,
                note: self.editor_note,
                revision: self.edit_revision,
                headings: self.outline_headings(viewport.cursor().position.line),
                highlighted: None,
                at_origin: true,
            });
        }
        let mut palette = Palette::new(mode, self.workspace_generation());
        palette.source = source;
        self.palette = Some(palette);
        self.refresh_palette(true);
        if mode == Mode::Outline {
            let active = self.highlighted_heading();
            if let Some(origin) = &mut self.outline_origin {
                origin.highlighted = active;
            }
        }
        self.finish_action(
            Disposition::Completed,
            None,
            Task::batch([
                iced::widget::operation::focus("palette-query"),
                self.reveal_palette_row(),
            ]),
        )
    }

    pub(super) fn open_workspace_removal(&mut self) -> Gateway {
        let outcome = self.open_palette(Mode::Workspaces);
        if let Some(palette) = &mut self.palette {
            palette.workspace_removal = true;
        }
        self.refresh_palette(true);
        outcome
    }

    pub(super) fn palette_query(&mut self, query: String) -> Gateway {
        if self.loading.is_some() {
            return self.blocked("Wait for the workspace to finish opening");
        }
        let Some(palette) = &mut self.palette else {
            return self.blocked("No palette is open");
        };
        if let Err(error) = wren_core::index::validate_query(&query) {
            palette.error = Some(error.into());
            return self.blocked(error);
        }
        palette.query = query;
        palette.error = None;
        self.refresh_palette(true);
        self.finish_action(Disposition::Completed, None, self.reveal_palette_row())
    }

    pub(super) fn palette_step(&mut self, delta: i32) -> Gateway {
        let Some(palette) = &mut self.palette else {
            return self.blocked("No palette is open");
        };
        palette.step(delta);
        self.finish_action(Disposition::Completed, None, self.reveal_palette_row())
    }

    pub(super) fn close_palette(&mut self) -> Gateway {
        self.palette = None;
        // A late native folder completion is unrelated to a dismissed overlay.
        let task = self.column_focus();
        self.finish_action(Disposition::Completed, None, task)
    }

    pub(super) fn refresh_palette(&mut self, reset: bool) {
        let Some(palette) = &self.palette else {
            return;
        };
        let mode = palette.mode;
        let workspace_removal = palette.workspace_removal;
        let source = palette.source;
        let query = palette.query.trim().to_owned();
        let generation = palette.generation;
        let revision = self
            .session
            .as_ref()
            .map_or(0, |session| session.index().revision());
        let mut rows = Vec::new();
        match mode {
            Mode::Commands => {
                for group in GROUPS {
                    for definition in DEFINITIONS.iter().filter(|definition| {
                        definition.group == group
                            && (self.preferences.boards || !definition.id.boards())
                    }) {
                        if fuzzy_score(&query, definition.label).is_none() {
                            continue;
                        }
                        rows.push(ResultRow {
                            key: Key::Command(definition.id),
                            title: definition.label.into(),
                            path: String::new(),
                            snippet: String::new(),
                            highlight: None,
                            content_hit: false,
                            group,
                            shortcut: self.shortcut_label(definition.key).unwrap_or_default(),
                            current: false,
                            enabled: self.command_enabled(definition.id),
                            action: Action::Command(definition.id),
                            heading: None,
                        });
                    }
                }
            }
            Mode::Jump | Mode::Search => {
                if let Some(session) = &self.session {
                    let kind = if mode == Mode::Jump {
                        QueryKind::Jump
                    } else {
                        QueryKind::Search
                    };
                    for hit in session.index().query(&query, Scope::Workspace, kind, 500) {
                        rows.push(ResultRow {
                            key: Key::Note(hit.note),
                            title: hit.title,
                            path: hit.path.as_path().to_string_lossy().into_owned(),
                            snippet: hit.snippet.text,
                            highlight: hit.snippet.highlight,
                            content_hit: hit.content_match.is_some(),
                            group: "",
                            shortcut: String::new(),
                            current: self.selected_note == Some(hit.note),
                            enabled: true,
                            action: Action::Note(SearchTarget {
                                generation,
                                index_revision: revision,
                                note: hit.note,
                                query: query.clone(),
                                content_match: hit.content_match,
                            }),
                            heading: None,
                        });
                    }
                }
            }
            Mode::JumpSection => {
                if let Some(model) = self.model() {
                    let mut ranked: Vec<_> = model
                        .sections
                        .iter()
                        .filter_map(|section| {
                            fuzzy_score(&query, &section.name).map(|score| (score, section))
                        })
                        .collect();
                    // Stable: equal scores (an empty query) keep the manual order.
                    ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                    for (_, section) in ranked {
                        let groups = model
                            .groups
                            .iter()
                            .filter(|group| group.section == section.id)
                            .count();
                        let mut row = action_row(
                            Key::Section(section.id),
                            section.name.clone(),
                            format!("{groups} {}", if groups == 1 { "group" } else { "groups" }),
                            true,
                            Action::Section(section.id),
                        );
                        row.current = self.selected_section == Some(section.id);
                        rows.push(row);
                    }
                }
            }
            Mode::JumpGroup => {
                if let Some(model) = self.model() {
                    let current = self.selected_section;
                    let mut ranked: Vec<_> = model
                        .groups
                        .iter()
                        .filter_map(|group| {
                            let section = model.section(group.section)?;
                            let haystack = format!("{} {}", group.name, section.name);
                            fuzzy_score(&query, &haystack).map(|score| (score, group, section))
                        })
                        .collect();
                    ranked.sort_by(|a, b| {
                        b.0.cmp(&a.0)
                            .then_with(|| (Some(b.2.id) == current).cmp(&(Some(a.2.id) == current)))
                    });
                    for (_, group, section) in ranked {
                        let mut row = action_row(
                            Key::Group(group.id),
                            group.name.clone(),
                            section.name.clone(),
                            true,
                            Action::Group(group.id),
                        );
                        row.current = self.selected_group == Some(group.id);
                        rows.push(row);
                    }
                }
            }
            Mode::Workspaces => {
                if let Some(config) = &self.config {
                    for entry in config.workspaces() {
                        if fuzzy_score(&query, &format!("{} {}", entry.name, entry.path.display()))
                            .is_none()
                        {
                            continue;
                        }
                        rows.push(ResultRow {
                            key: Key::Workspace(entry.path.clone()),
                            title: entry.name.clone(),
                            path: chrome::display_path(&entry.path),
                            snippet: String::new(),
                            highlight: None,
                            content_hit: false,
                            group: "",
                            shortcut: String::new(),
                            current: self.model().is_some_and(|model| model.root == entry.path),
                            enabled: true,
                            action: Action::Workspace(entry.path.clone()),
                            heading: None,
                        });
                    }
                }
                if !workspace_removal && fuzzy_score(&query, "Add workspace").is_some() {
                    rows.push(ResultRow {
                        key: Key::Command(CommandId::AddWorkspace),
                        title: "Add workspace…".into(),
                        path: "Choose an existing folder. Its files stay yours.".into(),
                        snippet: String::new(),
                        highlight: None,
                        content_hit: false,
                        group: "",
                        shortcut: String::new(),
                        current: false,
                        enabled: self.folder_request.is_none(),
                        action: Action::Command(CommandId::AddWorkspace),
                        heading: None,
                    });
                }
            }
            Mode::Themes => {
                let current = self.preferences.settings.theme.clone();
                let system = wren_core::settings::ThemeChoice::system();
                let mut choices = vec![(None, "System".to_string(), {
                    let (light, dark) = (definitions::LIGHT.name, definitions::DARK.name);
                    format!("{dark} or {light}, following the system appearance")
                })];
                choices.extend(definitions::BUILT_INS.iter().map(|theme| {
                    let appearance = match theme.appearance {
                        definitions::Appearance::Dark => "Dark",
                        definitions::Appearance::Light => "Light",
                    };
                    (
                        Some(theme.id),
                        theme.name.to_string(),
                        appearance.to_string(),
                    )
                }));
                for (id, title, path) in choices {
                    if fuzzy_score(&query, &title).is_none() {
                        continue;
                    }
                    let selected = match id {
                        None => current == system,
                        Some(id) => current == wren_core::settings::ThemeChoice::Fixed(id.into()),
                    };
                    let mut row = action_row(Key::Theme(id), title, path, true, Action::Theme(id));
                    row.current = selected;
                    rows.push(row);
                }
            }
            Mode::Fonts => {
                let families = fonts::families();
                let saved = self.preferences.settings.editor_font_family.as_deref();
                let current = match saved {
                    None => Some(fonts::EDITOR_FAMILY),
                    Some(name) => families.find(name),
                };
                // A hand-configured family the picker would not list (a
                // proportional font) still shows, as the current row.
                let configured =
                    current.filter(|name| families.monospace().all(|row| row != *name));
                for family in configured.into_iter().chain(families.monospace()) {
                    if fuzzy_score(&query, family).is_none() {
                        continue;
                    }
                    let note = if family == fonts::EDITOR_FAMILY {
                        "Bundled"
                    } else if Some(family) == configured {
                        "Configured in config.conf"
                    } else {
                        ""
                    };
                    let mut row = action_row(
                        Key::Font(family),
                        family.into(),
                        note.into(),
                        true,
                        Action::Font(family),
                    );
                    row.current = Some(family) == current;
                    rows.push(row);
                }
            }
            Mode::Outline => rows = self.outline_rows(&query),
            Mode::ItemActions(item) => {
                if let Some(target) = source {
                    for id in item_commands(item) {
                        let definition = DEFINITIONS
                            .iter()
                            .find(|entry| entry.id == *id)
                            .expect("item command definition");
                        if fuzzy_score(&query, definition.label).is_some() {
                            rows.push(action_row(
                                Key::ItemAction { target, id: *id },
                                definition.label.into(),
                                self.item_name(item).unwrap_or("Removed item").into(),
                                self.item_target_valid(target),
                                Action::ItemCommand { target, id: *id },
                            ));
                        }
                    }
                }
            }
            Mode::Reorder(_) => {
                if let (Some(model), Some(source)) = (self.model(), source) {
                    let siblings: Vec<_> = match source.item {
                        ItemRef::Section(_) => model
                            .sections
                            .iter()
                            .map(|section| (ItemRef::Section(section.id), section.name.as_str()))
                            .collect(),
                        ItemRef::Group(_) => model
                            .groups
                            .iter()
                            .filter(|group| Some(Parent::Section(group.section)) == source.parent)
                            .map(|group| (ItemRef::Group(group.id), group.name.as_str()))
                            .collect(),
                        ItemRef::Note(_) => Vec::new(),
                    };
                    for (item, name) in siblings {
                        if item == source.item {
                            continue;
                        }
                        let title = format!("Before {name}");
                        if fuzzy_score(&query, &title).is_some() {
                            let target = ReorderTarget {
                                source,
                                before: self.item_target(item),
                            };
                            rows.push(action_row(
                                Key::Reorder(target),
                                title,
                                name.into(),
                                self.item_target_valid(source),
                                Action::Reorder(target),
                            ));
                        }
                    }
                    if fuzzy_score(&query, "Move to end").is_some() {
                        let target = ReorderTarget {
                            source,
                            before: None,
                        };
                        rows.push(action_row(
                            Key::Reorder(target),
                            "Move to end".into(),
                            "After all siblings, including hidden items".into(),
                            self.item_target_valid(source),
                            Action::Reorder(target),
                        ));
                    }
                }
            }
            Mode::MoveNote(_) | Mode::MoveGroup(_) => {
                if let Some(model) = self.model() {
                    let current = match mode {
                        Mode::MoveNote(note) => {
                            model.note(note).map(|note| Parent::Group(note.group))
                        }
                        Mode::MoveGroup(group) => model
                            .group(group)
                            .map(|group| Parent::Section(group.section)),
                        _ => None,
                    };
                    for section in &model.sections {
                        let parent = Parent::Section(section.id);
                        if matches!(mode, Mode::MoveGroup(_))
                            && current != Some(parent)
                            && fuzzy_score(&query, &section.name).is_some()
                        {
                            rows.push(destination_row(
                                parent,
                                section.name.clone(),
                                section.path.as_path().to_string_lossy().into_owned(),
                            ));
                        }
                        if matches!(mode, Mode::MoveNote(_)) {
                            for group in model
                                .groups
                                .iter()
                                .filter(|group| group.section == section.id)
                            {
                                let parent = Parent::Group(group.id);
                                let title = format!("{} › {}", section.name, group.name);
                                if current != Some(parent) && fuzzy_score(&query, &title).is_some()
                                {
                                    rows.push(destination_row(
                                        parent,
                                        title,
                                        group.path.as_path().to_string_lossy().into_owned(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(palette) = &mut self.palette {
            palette.replace(rows, reset);
            palette.revision = revision;
        }
    }

    fn blocked_palette(&mut self, warning: &str) -> Gateway {
        if let Some(palette) = &mut self.palette {
            palette.error = Some(warning.into());
        }
        self.blocked(warning)
    }

    pub(super) fn activate_palette_key(&mut self, generation: u64, key: Key) -> Gateway {
        let Some(palette) = &self.palette else {
            return self.blocked("No palette is open");
        };
        if generation != self.workspace_generation() || palette.generation != generation {
            return self.blocked_palette("This result belongs to a previous workspace");
        }
        let Some(index) = palette.results.iter().position(|result| result.key == key) else {
            return self.blocked_palette("The palette result no longer exists");
        };
        self.activate_palette(Some(index))
    }

    pub(super) fn activate_palette(&mut self, index: Option<usize>) -> Gateway {
        if self.loading.is_some() {
            return self.blocked("Wait for the workspace to finish opening");
        }
        let Some(palette) = &self.palette else {
            return self.blocked("No palette is open");
        };
        if palette.generation != self.workspace_generation() {
            return self.blocked_palette("This picker belongs to a previous workspace");
        }
        let mode = palette.mode;
        let source = palette.source;
        let workspace_removal = palette.workspace_removal;
        let Some(result) = index
            .or(palette.active)
            .and_then(|index| palette.results.get(index))
            .cloned()
        else {
            return self.blocked_palette("No palette result is selected");
        };
        if !result.enabled {
            return self.blocked_palette("This command is unavailable in the current context");
        }
        if let Some(palette) = &mut self.palette {
            palette.error = None;
        }
        let moving = matches!(result.action, Action::Destination(_) | Action::Reorder(_));
        let mut outcome = match result.action {
            Action::Command(id) => self.dispatch(Command::Run(id)),
            Action::Note(target) => {
                self.column = super::columns::Column::Editor;
                self.dispatch(Command::OpenNote(target))
            }
            Action::Section(section) => self.jump_into(
                super::columns::Column::Sections,
                Command::SelectSection(section),
            ),
            Action::Group(group) => self.jump_into(
                super::columns::Column::Groups,
                Command::SelectParent(Parent::Group(group)),
            ),
            Action::ItemCommand { target, id } => self.dispatch(Command::ItemAction { target, id }),
            Action::Reorder(target) => self.dispatch(Command::Reorder(target)),
            Action::Theme(id) => self.choose_theme(id),
            Action::Font(family) => self.choose_font(family),
            Action::Heading { revision, line } => self.choose_heading(revision, line),
            Action::Workspace(path) => {
                if workspace_removal {
                    self.dispatch(Command::RemoveWorkspace(path))
                } else {
                    self.open_registered_workspace(path)
                }
            }
            Action::Destination(to) => {
                if !source.is_some_and(|source| self.item_target_valid(source)) {
                    self.blocked("The move source no longer exists in its original parent")
                } else {
                    match mode {
                        Mode::MoveNote(note) => match to {
                            Parent::Group(to) => self.process_core(Op::MoveNote { note, to }),
                            Parent::Section(_) => self.blocked("A note can only move to a group"),
                        },
                        Mode::MoveGroup(group) => match to {
                            Parent::Section(to) => self.process_core(Op::MoveGroup { group, to }),
                            _ => self.blocked("A group can only move to a section"),
                        },
                        _ => self.blocked("This picker has no move source"),
                    }
                }
            }
        };
        if moving && outcome.processed.disposition.committed() {
            self.reveal_selection();
            outcome = self.close_palette_after(outcome);
        } else if outcome.processed.disposition == Disposition::Blocked {
            if let Some(palette) = &mut self.palette {
                palette.error = outcome.processed.warning.clone();
            }
        }
        outcome
    }

    /// The open note's headings, with the section holding `line` current.
    fn outline_headings(&self, line: usize) -> Vec<OutlineHeading> {
        let text = self.editor.text();
        let document = wren_markdown::Document::new(&text);
        let outline = wren_markdown::outline::Outline::new(&document);
        let headings = outline.headings();
        let top = headings
            .iter()
            .map(|heading| heading.level)
            .min()
            .unwrap_or(1);
        let current = outline.section_at(line);
        headings
            .iter()
            .enumerate()
            .map(|(index, heading)| {
                let mut path: Vec<&str> = outline
                    .ancestors(index)
                    .map(|parent| parent.text.as_str())
                    .collect();
                path.reverse();
                OutlineHeading {
                    title: heading.text.clone(),
                    path: path.join(" › "),
                    level: heading.level,
                    depth: heading.level - top,
                    line: heading.line,
                    current: current == Some(index),
                }
            })
            .collect()
    }

    fn outline_rows(&self, query: &str) -> Vec<ResultRow> {
        let fresh;
        let headings = match &self.outline_origin {
            Some(origin) => &origin.headings,
            None => {
                fresh = self.outline_headings(self.editor.cursor().position.line);
                &fresh
            }
        };
        let mut ranked: Vec<_> = headings
            .iter()
            .filter_map(|heading| fuzzy_score(query, &heading.title).map(|score| (score, heading)))
            .collect();
        // Stable: equal scores (an empty query) keep the document order.
        ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        ranked
            .into_iter()
            .map(|(_, heading)| {
                let (revision, line) = (self.edit_revision, heading.line);
                let mut row = action_row(
                    Key::Heading { revision, line },
                    heading.title.clone(),
                    heading.path.clone(),
                    true,
                    Action::Heading { revision, line },
                );
                row.current = heading.current;
                row.heading = Some(HeadingRow {
                    level: heading.level,
                    depth: heading.depth,
                });
                row
            })
            .collect()
    }

    fn highlighted_heading(&self) -> Option<Key> {
        let palette = self
            .palette
            .as_ref()
            .filter(|palette| palette.mode == Mode::Outline)?;
        let row = palette.results.get(palette.active?)?;
        matches!(row.key, Key::Heading { .. }).then(|| row.key.clone())
    }

    /// Scroll `line` near the top of the editor and put the cursor on it.
    fn show_heading(&mut self, line: usize) {
        self.editor.scroll_to_line(line, Placement::NearTop);
        self.editor.move_cursor(Cursor {
            position: Position { line, column: 0 },
            selection: None,
        });
    }

    fn return_to_origin(&mut self, origin: OutlineOrigin) {
        // A different note (a workspace switch, a deleted note) has no use
        // for the old note's cursor.
        if !origin.at_origin && origin.note == self.editor_note {
            self.editor.restore_viewport(origin.viewport);
        }
    }

    fn choose_heading(&mut self, revision: u64, line: usize) -> Gateway {
        if revision != self.edit_revision {
            return self.blocked("The note changed; choose the heading again");
        }
        self.outline_origin = None;
        // The row the outline opened on was never shown; a previewed one is
        // already in place, and showing it again moves nothing.
        self.show_heading(line);
        self.palette = None;
        self.column = super::columns::Column::Editor;
        self.finish_action(Disposition::Completed, None, super::focus_editor())
    }

    /// Follow the outline's highlighted row in the editor, and put the editor
    /// back when the outline closes without a choice. Runs after every
    /// message, so it acts only on a change: the 25 ms tick moves nothing.
    pub(super) fn sync_outline(&mut self) -> Task<Message> {
        let Some(origin) = &self.outline_origin else {
            return Task::none();
        };
        if !self
            .palette
            .as_ref()
            .is_some_and(|palette| palette.mode == Mode::Outline)
        {
            let origin = self.outline_origin.take().expect("outline origin");
            self.return_to_origin(origin);
            return Task::none();
        }
        if origin.note != self.editor_note {
            self.outline_origin = None;
            self.palette = None;
            return self.column_focus();
        }
        let revised = origin.revision != self.edit_revision;
        if revised {
            let headings = self.outline_headings(origin.viewport.cursor().position.line);
            let origin = self.outline_origin.as_mut().expect("outline origin");
            origin.revision = self.edit_revision;
            origin.headings = headings;
            self.refresh_palette(false);
        }
        let highlighted = self.highlighted_heading();
        let origin = self.outline_origin.as_mut().expect("outline origin");
        if highlighted == origin.highlighted {
            return Task::none();
        }
        origin.highlighted = highlighted.clone();
        // New text renames every row; a row the editor never followed stays
        // unshown, and a followed one is followed to where it moved.
        if revised && origin.at_origin {
            return Task::none();
        }
        match highlighted {
            Some(Key::Heading { line, .. }) => {
                origin.at_origin = false;
                self.show_heading(line);
            }
            _ if !origin.at_origin => {
                origin.at_origin = true;
                let viewport = origin.viewport;
                self.editor.restore_viewport(viewport);
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn activate_note(&mut self, target: SearchTarget) -> Gateway {
        if target.generation != self.workspace_generation()
            || self
                .model()
                .and_then(|model| model.note(target.note))
                .is_none()
        {
            return self.blocked("The search result belongs to an old workspace or a removed note");
        }
        let outcome = self.process_core(Op::OpenNote {
            note: Some(target.note),
        });
        if outcome.processed.disposition.committed() && self.selected_note == Some(target.note) {
            // Offset provenance is retained in the snapshot, but never trusted
            // after the barrier: conflicts and external edits can change bytes.
            let _queried_revision = target.index_revision;
            // A board's match is in its labels, which the empty editor
            // behind it does not hold.
            if target.content_match.is_some() && self.surface() != Surface::Board {
                self.editor.reveal_literal(target.query.trim());
            }
            self.palette = None;
            self.reveal_selection();
        }
        outcome
    }

    pub(super) fn reveal_palette_row(&self) -> Task<Message> {
        if self
            .palette
            .as_ref()
            .and_then(|palette| palette.active)
            .is_none()
        {
            return Task::none();
        }
        let palette = self.palette.as_ref().expect("selected palette row");
        let key = &palette.results[palette.active.expect("selected row")].key;
        iced::advanced::widget::operate(palette_scroll::ensure_visible(
            "palette-results".into(),
            result_id(palette.generation, key),
        ))
        .map(|_| Message::PaletteRevealed)
    }

    pub(super) fn palette_view<'a>(&'a self, base: Element<'a, Message>) -> Element<'a, Message> {
        let Some(palette) = &self.palette else {
            return base;
        };
        let colors = self.definition.colors;
        let search = palette.mode == Mode::Search;
        let outline = palette.mode == Mode::Outline;
        // Every mode shares one frame except the outline, which is narrow so
        // it can sit beside the text it previews.
        let width = if outline { 460.0 } else { PALETTE_WIDTH };
        // The outline previews headings in the editor, so it sits beside the
        // text: over the browser columns when they are wide enough, else at
        // the right, away from where lines start.
        let align = if !outline {
            iced::alignment::Horizontal::Center
        } else if self.browser_width() >= width + 32.0 {
            iced::alignment::Horizontal::Left
        } else {
            iced::alignment::Horizontal::Right
        };
        let top = PALETTE_TOP.min((self.viewport.height * 0.16).max(8.0));
        let mut list_height =
            (self.viewport.height - top - 134.0).clamp(40.0, if search { 460.0 } else { 400.0 });
        if search {
            // Search hugs its results instead of framing a mostly empty panel.
            let rows = palette.results.len() as f32;
            let content = if rows == 0.0 {
                48.0
            } else {
                rows * SEARCH_ROW_HEIGHT + (rows - 1.0) * RESULT_SPACING + 2.0 * RESULT_PADDING
            };
            list_height = list_height.min(content.max(160.0));
        } else {
            list_height = snapped_list_height(&palette.results, list_height);
        }
        let mut input = row![].spacing(8).align_y(iced::Center);
        if search {
            input = input.push(chrome::icon(chrome::Icon::Search, 16.0, colors.muted));
        }
        if let Some(chip) = if palette.workspace_removal {
            Some("Remove workspace")
        } else {
            palette.mode.chip()
        } {
            input = input.push(
                container(
                    text(chip)
                        .size(12)
                        .font(fonts::SEMIBOLD)
                        .color(theme::color(colors.accent)),
                )
                .padding([3, 8])
                .style(move |_| chrome::surface(self.definition, colors.selected, 4.0, false)),
            );
        }
        input = input.push(
            text_input(
                if palette.workspace_removal {
                    "Choose a workspace to remove from the list…"
                } else {
                    palette.mode.placeholder()
                },
                &palette.query,
            )
            .id("palette-query")
            .size(15)
            .padding([4, 0])
            .on_input_maybe(self.loading.is_none().then_some(Message::PaletteQuery))
            .style(move |_, status| chrome::input(self.definition, status, false)),
        );
        input = if self.loading.is_some() {
            input.push(text("Opening…").size(12).color(theme::color(colors.muted)))
        } else if search {
            input.push(
                text(format!("{} results", palette.results.len()))
                    .size(12)
                    .color(theme::color(colors.muted)),
            )
        } else {
            input.push(chrome::keycap("esc", self.definition))
        };
        let mut results = column![].spacing(RESULT_SPACING).padding(RESULT_PADDING);
        let mut last_group = "";
        for (index, result) in palette.results.iter().enumerate() {
            if result.group != last_group && !result.group.is_empty() {
                results = results.push(
                    container(
                        text(result.group.to_uppercase())
                            .size(GROUP_HEADER_SIZE)
                            .font(fonts::SEMIBOLD)
                            .color(theme::color(colors.muted)),
                    )
                    .padding([GROUP_HEADER_PADDING, 10.0]),
                );
                last_group = result.group;
            }
            let mut label = column![].spacing(3);
            if search {
                label = label.push(ellipsis(
                    result.path.replace('/', " › "),
                    11.0,
                    theme::color(colors.muted),
                ));
            }
            label = label.push(
                ellipsis(
                    &result.title,
                    13.0,
                    theme::color(if result.enabled {
                        colors.text
                    } else {
                        colors.faint
                    }),
                )
                .font(fonts::MEDIUM),
            );
            if search && !result.snippet.is_empty() {
                label = label.push(highlighted_text(
                    result.snippet.clone(),
                    result.highlight.clone(),
                    self.definition,
                ));
            }
            if !search && !result.path.is_empty() {
                let path = if matches!(palette.mode, Mode::Workspaces | Mode::Outline) {
                    result.path.clone()
                } else {
                    result.path.replace('/', " › ")
                };
                label = label.push(ellipsis(path, 11.0, theme::color(colors.muted)));
            }
            let indent = result
                .heading
                .map_or(0.0, |heading| f32::from(heading.depth) * 16.0);
            let mut contents = row![].spacing(12).align_y(iced::Center).clip(true);
            if let Key::Theme(id) = &result.key {
                contents = contents.push(theme_swatch(*id));
            }
            contents = contents.push(
                container(label)
                    .padding(iced::Padding {
                        left: indent,
                        ..iced::Padding::ZERO
                    })
                    .width(Fill),
            );
            if search {
                contents = contents.push(chrome::label(
                    if result.content_hit { "text" } else { "title" },
                    11,
                    colors.muted,
                ));
            }
            if result.current {
                contents =
                    contents.push(text("current").size(11).color(theme::color(colors.accent)));
            }
            if let Some(state) = match result.key {
                Key::Command(id) => self.command_state(id),
                _ => None,
            } {
                contents = contents.push(chrome::label(state, 11, colors.muted));
            }
            if let Some(heading) = result.heading {
                contents = contents.push(chrome::label(
                    format!("H{}", heading.level),
                    11,
                    colors.faint,
                ));
            }
            if !result.shortcut.is_empty() {
                contents = contents.push(chrome::keycap(&result.shortcut, self.definition));
            }
            let active = palette.active == Some(index);
            let row = button(container(contents).center_y(Fill))
                .height(row_height(search, result))
                .on_press_maybe((result.enabled && self.loading.is_none()).then_some(
                    Message::PaletteChoose(palette.generation, result.key.clone()),
                ))
                .padding([8, 10])
                .width(Fill)
                .style(move |_, status| chrome::list_button(self.definition, status, active));
            let identified = container(row)
                .width(Fill)
                .id(result_id(palette.generation, &result.key));
            results = results.push(identified);
        }
        if palette.results.is_empty() {
            results = results.push(
                container(
                    text(if search && palette.query.trim().is_empty() {
                        "Search titles and text across this workspace."
                    } else if palette.mode == Mode::Outline && palette.query.trim().is_empty() {
                        "No headings in this note."
                    } else {
                        "No matches."
                    })
                    .size(13)
                    .color(theme::color(colors.muted)),
                )
                .padding(14),
            );
        }
        let mut panel = column![
            chrome::bottom_rule(container(input).padding([12, 14]), self.definition),
            scrollable(results)
                .id("palette-results")
                .height(list_height)
                .style(move |_, status| chrome::scrollbar(self.definition, status)),
        ];
        if let Some(error) = &palette.error {
            panel = panel.push(
                container(text(error).size(12).color(theme::color(colors.danger))).padding([6, 14]),
            );
        }
        let hint = |key: &'static str, action: &'static str| {
            row![
                chrome::keycap(key, self.definition),
                text(action).size(11).color(theme::color(colors.muted))
            ]
            .spacing(6)
            .align_y(iced::Center)
        };
        let footer: Element<'_, Message> = if palette.workspace_removal {
            text("↵ removes the registration only. Folders and notes stay untouched.")
                .size(11)
                .color(theme::color(colors.muted))
                .into()
        } else {
            let mut hints = row![].spacing(14).align_y(iced::Center);
            if search {
                hints = hints.push(
                    text("Titles and text, whole workspace")
                        .size(11)
                        .color(theme::color(colors.muted)),
                );
            } else {
                hints = hints.push(hint("↑↓", "move"));
            }
            hints
                .push(hint(
                    "↵",
                    match palette.mode {
                        Mode::Search => "open at match",
                        Mode::Outline => "jump",
                        _ => "run",
                    },
                ))
                .push(hint("esc", "close"))
                .into()
        };
        panel = panel.push(chrome::top_rule(
            container(footer).padding([9, 14]),
            self.definition,
        ));
        let dialog = container(panel)
            .id(format!("palette-{}", palette.mode.key()))
            .max_height((self.viewport.height - top - 8.0).max(24.0))
            .width(width)
            .max_width((self.viewport.width - 32.0).max(120.0))
            .clip(true)
            .style(move |_| chrome::dialog(self.definition));
        let scrim = mouse_area(
            container(iced::widget::space())
                .width(Fill)
                .height(Fill)
                .style(move |_| chrome::scrim(self.definition)),
        )
        .on_press(Message::PaletteClose);
        stack![
            base,
            opaque(scrim),
            container(opaque(dialog))
                .padding(iced::Padding {
                    top,
                    right: 16.0,
                    bottom: 8.0,
                    left: 16.0
                })
                .width(Fill)
                .align_x(align)
                .height(Fill)
        ]
        .into()
    }
}

/// Every palette mode shares one frame, so switching modes never jumps.
const PALETTE_WIDTH: f32 = 680.0;
const PALETTE_TOP: f32 = 80.0;
const RESULT_SPACING: f32 = 2.0;
const RESULT_PADDING: f32 = 6.0;
const GROUP_HEADER_SIZE: f32 = 11.0;
const GROUP_HEADER_PADDING: f32 = 8.0;
/// The header label at iced's default 1.3 line height, plus its padding.
const GROUP_HEADER_HEIGHT: f32 = GROUP_HEADER_SIZE * 1.3 + 2.0 * GROUP_HEADER_PADDING;

const SEARCH_ROW_HEIGHT: f32 = 76.0;

fn row_height(search: bool, result: &ResultRow) -> f32 {
    if search {
        SEARCH_ROW_HEIGHT
    } else if result.path.is_empty() {
        44.0
    } else {
        54.0
    }
}

/// Shortens an overflowing non-search list to the end of its last whole
/// result, so the visible list never ends on a group header or a cut row.
fn snapped_list_height(results: &[ResultRow], available: f32) -> f32 {
    let mut end = RESULT_PADDING - RESULT_SPACING;
    let mut last_group = "";
    let mut fitted = None;
    for result in results {
        if result.group != last_group && !result.group.is_empty() {
            end += RESULT_SPACING + GROUP_HEADER_HEIGHT;
            last_group = result.group;
        }
        end += RESULT_SPACING + row_height(false, result);
        if end > available {
            return fitted.unwrap_or(available);
        }
        fitted = Some(end);
    }
    available
}

pub(super) fn result_id(generation: u64, key: &Key) -> iced::advanced::widget::Id {
    let key = match key {
        Key::Command(id) => id.key().into(),
        Key::Note(note) => format!("note-{}", note.0),
        Key::Section(section) => format!("section-{}", section.0),
        Key::Group(group) => format!("group-{}", group.0),
        Key::Destination(parent) => format!("parent-{parent:?}"),
        Key::ItemAction { target, id } => format!("item-{target:?}-{}", id.key()),
        Key::Reorder(target) => format!("reorder-{target:?}"),
        Key::Workspace(path) => format!("workspace-{}", path.display()),
        Key::Theme(id) => format!("theme-{}", id.unwrap_or("system")),
        Key::Font(family) => format!("font-{family}"),
        Key::Heading { revision, line } => format!("heading-{revision}-{line}"),
    };
    iced::advanced::widget::Id::from(format!("palette-row-{generation}-{key}"))
}

/// The row in `results` for the heading `selected` highlighted among `old`,
/// after the text changed: the same title and path, counted among its
/// namesakes when their number is unchanged, else the nearest by line.
fn same_heading(old: &[ResultRow], selected: &ResultRow, results: &[ResultRow]) -> Option<usize> {
    let line_of = |row: &ResultRow| match row.key {
        Key::Heading { line, .. } => Some(line),
        _ => None,
    };
    let from = line_of(selected)?;
    let namesake = |row: &ResultRow| row.title == selected.title && row.path == selected.path;
    let before: Vec<_> = old.iter().filter(|row| namesake(row)).collect();
    let after: Vec<_> = (0..results.len())
        .filter(|&index| namesake(&results[index]))
        .collect();
    let nearest = |candidates: &mut dyn Iterator<Item = usize>| {
        candidates.min_by_key(|&index| line_of(&results[index]).map(|line| line.abs_diff(from)))
    };
    if before.len() == after.len() {
        if let Some(occurrence) = before.iter().position(|row| row.key == selected.key) {
            return Some(after[occurrence]);
        }
    }
    nearest(&mut after.iter().copied())
        .or_else(|| {
            nearest(&mut (0..results.len()).filter(|&index| results[index].title == selected.title))
        })
        .or_else(|| nearest(&mut (0..results.len())))
}

fn action_row(key: Key, title: String, path: String, enabled: bool, action: Action) -> ResultRow {
    ResultRow {
        key,
        title,
        path,
        snippet: String::new(),
        highlight: None,
        content_hit: false,
        group: "",
        shortcut: String::new(),
        current: false,
        enabled,
        action,
        heading: None,
    }
}

fn destination_row(parent: Parent, title: String, path: String) -> ResultRow {
    ResultRow {
        key: Key::Destination(parent),
        title,
        path,
        snippet: String::new(),
        highlight: None,
        content_hit: false,
        group: "",
        shortcut: String::new(),
        current: false,
        enabled: true,
        action: Action::Destination(parent),
        heading: None,
    }
}

pub(super) fn highlighted_text<'a>(
    snippet: String,
    highlight: Option<Range<usize>>,
    definition: wren_core::theme::Theme,
) -> Element<'a, Message> {
    use iced::widget::span;
    use iced::widget::text::Wrapping;
    // Snippets span line breaks; show them on one line. Each replacement is
    // one ASCII byte for another, so highlight offsets stay valid.
    let snippet: String = snippet
        .chars()
        .map(|character| match character {
            '\n' | '\r' | '\t' => ' ',
            other => other,
        })
        .collect();
    if let Some(range) = &highlight {
        if let (Some(before), Some(hit), Some(after)) = (
            snippet.get(..range.start),
            snippet.get(range.clone()),
            snippet.get(range.end..),
        ) {
            return iced::widget::rich_text::<(), Message, iced::Theme, iced::Renderer>([
                span(before.to_owned()).color(theme::color(definition.colors.muted)),
                span(hit.to_owned())
                    .color(theme::color(definition.colors.accent))
                    .font(fonts::SEMIBOLD),
                span(after.to_owned()).color(theme::color(definition.colors.muted)),
            ])
            .size(12)
            .wrapping(Wrapping::None)
            .into();
        }
    }
    chrome::label(snippet, 12, definition.colors.muted).into()
}

/// A miniature of a theme for its picker row: the editor surface with a
/// heading stroke and a body stroke. The System row shows its light and dark
/// pair side by side.
fn theme_swatch<'a>(id: Option<&'static str>) -> Element<'a, Message> {
    fn half<'a>(
        definition: definitions::Theme,
        width: f32,
        radius: iced::border::Radius,
    ) -> Element<'a, Message> {
        let colors = definition.colors;
        let stroke = |color, length| {
            container(iced::widget::space())
                .width(length)
                .height(3)
                .style(move |_| container::Style {
                    background: Some(theme::color(color).into()),
                    border: iced::Border {
                        radius: 1.5.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
        };
        container(
            column![
                stroke(colors.heading, width * 0.45),
                stroke(colors.body, width * 0.7),
                stroke(colors.link, width * 0.35),
            ]
            .spacing(3),
        )
        .padding([6, 6])
        .width(width)
        .height(28)
        .style(move |_| container::Style {
            background: Some(theme::color(colors.editor).into()),
            border: iced::Border {
                color: theme::color(colors.control_border),
                width: 1.0,
                radius,
            },
            ..Default::default()
        })
        .into()
    }
    let radius = iced::border::Radius::new(6.0);
    match id.and_then(definitions::builtin) {
        Some(definition) => half(definition, 44.0, radius),
        None => row![
            half(definitions::LIGHT, 22.0, radius.right(0.0)),
            half(definitions::DARK, 22.0, radius.left(0.0)),
        ]
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(note: u64) -> ResultRow {
        ResultRow {
            key: Key::Note(NoteId(note)),
            title: note.to_string(),
            path: String::new(),
            snippet: String::new(),
            highlight: None,
            content_hit: false,
            group: "",
            shortcut: String::new(),
            current: false,
            enabled: true,
            action: Action::Command(CommandId::Jump),
            heading: None,
        }
    }

    const NOTE: &str =
        "# Guide\n\nintro\n\n## Install\n\nsteps\n\n### Linux\n\napt\n\n## Usage\n\nrun it\n";

    fn outline_app(text: &str) -> App {
        let (mut app, _) = App::new(super::super::Options {
            editor: true,
            ..Default::default()
        });
        app.editor.replace_text(text, false);
        app
    }

    fn open_outline(app: &mut App) {
        let _ = app.update(super::super::preferences::shortcut(CommandId::Outline));
        assert_eq!(app.palette.as_ref().unwrap().mode, Mode::Outline);
    }

    fn key(app: &mut App, nav: crate::keys::Nav) {
        let _ = app.update(Message::Key(crate::keys::Input::Nav(nav)));
    }

    fn caret(line: usize, column: usize) -> Cursor {
        Cursor {
            position: Position { line, column },
            selection: None,
        }
    }

    fn active_title(app: &App) -> &str {
        let palette = app.palette.as_ref().unwrap();
        &palette.results[palette.active.unwrap()].title
    }

    #[test]
    fn the_outline_lists_headings_by_level_and_opens_on_the_cursors_section() {
        let mut app = outline_app(NOTE);
        app.editor.move_cursor(caret(10, 1));
        open_outline(&mut app);
        let rows = &app.palette.as_ref().unwrap().results;
        let summary: Vec<_> = rows
            .iter()
            .map(|row| {
                let heading = row.heading.unwrap();
                (
                    row.title.as_str(),
                    row.path.as_str(),
                    heading.level,
                    heading.depth,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("Guide", "", 1, 0),
                ("Install", "Guide", 2, 1),
                ("Linux", "Guide › Install", 3, 2),
                ("Usage", "Guide", 2, 1),
            ]
        );
        assert_eq!(active_title(&app), "Linux");
        assert!(rows[2].current);
        // Opening previews nothing; only a highlight move does.
        assert_eq!(app.editor.cursor(), caret(10, 1));
        let _ = app.update(Message::PaletteQuery("usa".into()));
        assert_eq!(active_title(&app), "Usage");
        assert_eq!(app.editor.cursor(), caret(12, 0));
    }

    #[test]
    fn escape_from_the_outline_restores_the_cursor_and_selection() {
        let mut app = outline_app(NOTE);
        let selected = Cursor {
            position: Position {
                line: 10,
                column: 3,
            },
            selection: Some(Position {
                line: 10,
                column: 0,
            }),
        };
        app.editor.move_cursor(selected);
        open_outline(&mut app);
        key(&mut app, crate::keys::Nav::Down);
        assert_eq!(active_title(&app), "Usage");
        assert_eq!(app.editor.cursor(), caret(12, 0));
        // The tick, like any message, re-moves nothing once a row is shown.
        app.editor.move_cursor(caret(0, 0));
        let _ = app.update(Message::Tick);
        assert_eq!(app.editor.cursor(), caret(0, 0));
        // No match shows the origin; a match previews again.
        let _ = app.update(Message::PaletteQuery("zzz".into()));
        assert_eq!(app.editor.cursor(), selected);
        let _ = app.update(Message::PaletteQuery("inst".into()));
        assert_eq!(app.editor.cursor(), caret(4, 0));

        let _ = app.update(Message::Escape);
        assert!(app.palette.is_none());
        assert!(app.outline_origin.is_none());
        assert_eq!(app.editor.cursor(), selected);
        assert_eq!(app.editor.selected_text().as_deref(), Some("apt"));

        // Closing any other way restores it too.
        open_outline(&mut app);
        key(&mut app, crate::keys::Nav::Up);
        assert_eq!(app.editor.cursor(), caret(4, 0));
        let _ = app.update(Message::PaletteClose);
        assert_eq!(app.editor.cursor(), selected);
    }

    #[test]
    fn enter_in_the_outline_keeps_the_heading_and_focuses_the_editor() {
        let mut app = outline_app(NOTE);
        app.editor.move_cursor(caret(14, 2));
        open_outline(&mut app);
        assert_eq!(active_title(&app), "Usage");
        key(&mut app, crate::keys::Nav::Up);
        key(&mut app, crate::keys::Nav::Up);
        assert_eq!(app.editor.cursor(), caret(4, 0));
        key(&mut app, crate::keys::Nav::Enter);
        assert!(app.palette.is_none());
        assert!(app.outline_origin.is_none());
        assert_eq!(app.column, super::super::columns::Column::Editor);
        assert_eq!(app.editor.cursor(), caret(4, 0));
        let _ = app.update(Message::Escape);
        assert_eq!(app.editor.cursor(), caret(4, 0));

        // Enter on the row it opened on jumps to that heading.
        open_outline(&mut app);
        assert_eq!(active_title(&app), "Install");
        app.editor.move_cursor(caret(6, 1));
        key(&mut app, crate::keys::Nav::Enter);
        assert_eq!(app.editor.cursor(), caret(4, 0));
    }

    #[test]
    fn outline_rows_follow_a_text_change_and_reject_stale_keys() {
        let mut app = outline_app(NOTE);
        open_outline(&mut app);
        let stale = app.palette.as_ref().unwrap().results[3].key.clone();
        key(&mut app, crate::keys::Nav::Down);
        key(&mut app, crate::keys::Nav::Down);
        assert_eq!(active_title(&app), "Linux");
        app.editor
            .replace_text(&format!("# Preface\n\n{NOTE}"), true);
        app.edit_revision += 1;
        let _ = app.update(Message::Tick);
        let palette = app.palette.as_ref().unwrap();
        assert_eq!(palette.results.len(), 5);
        assert!(palette.results.iter().all(
            |row| matches!(row.key, Key::Heading { revision, .. } if revision == app.edit_revision)
        ));
        // The same heading stays highlighted, and the editor follows it down.
        assert_eq!(active_title(&app), "Linux");
        assert_eq!(app.editor.cursor(), caret(10, 0));
        let generation = palette.generation;
        let outcome = app.activate_palette_key(generation, stale);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert!(app.palette.is_some());
    }

    #[test]
    fn a_text_change_before_any_highlight_move_moves_nothing() {
        let mut app = outline_app(NOTE);
        app.editor.move_cursor(caret(14, 2));
        open_outline(&mut app);
        assert_eq!(active_title(&app), "Usage");
        app.editor
            .replace_text(&format!("{NOTE}\n## Added\n"), true);
        app.edit_revision += 1;
        let _ = app.update(Message::Tick);
        assert_eq!(active_title(&app), "Usage");
        assert_eq!(app.editor.cursor(), caret(14, 2));
        // The rebased row is still unshown: moving off it and back shows it.
        key(&mut app, crate::keys::Nav::Up);
        assert_eq!(app.editor.cursor(), caret(8, 0));
        key(&mut app, crate::keys::Nav::Down);
        assert_eq!(app.editor.cursor(), caret(12, 0));
        let _ = app.update(Message::Escape);
        assert_eq!(app.editor.cursor(), caret(14, 2));
    }

    #[test]
    fn a_highlighted_heading_keeps_its_place_among_namesakes_and_renamed_parents() {
        let text = "# A\n\n## Example\n\none\n\n## Example\n\ntwo\n\n## Example\n\nthree\n";
        let mut app = outline_app(text);
        open_outline(&mut app);
        for _ in 0..3 {
            key(&mut app, crate::keys::Nav::Down);
        }
        assert_eq!(app.editor.cursor(), caret(10, 0));
        app.editor.replace_text(&format!("\n{text}"), true);
        app.edit_revision += 1;
        let _ = app.update(Message::Tick);
        assert_eq!(app.palette.as_ref().unwrap().active, Some(3));
        assert_eq!(app.editor.cursor(), caret(11, 0));

        // Renaming the parent changes every path; the nearest namesake wins.
        app.editor
            .replace_text(&format!("\n{}", text.replacen("# A", "# B", 1)), true);
        app.edit_revision += 1;
        let _ = app.update(Message::Tick);
        let palette = app.palette.as_ref().unwrap();
        assert_eq!(palette.active, Some(3));
        assert_eq!(palette.results[3].path, "B");
        assert_eq!(app.editor.cursor(), caret(11, 0));

        // A removed heading hands the highlight to the nearest one.
        app.editor.replace_text("\n# B\n\none\n\n## Usage\n", true);
        app.edit_revision += 1;
        let _ = app.update(Message::Tick);
        assert_eq!(active_title(&app), "Usage");
        assert_eq!(app.editor.cursor(), caret(5, 0));
    }

    #[test]
    fn a_note_without_headings_shows_an_empty_outline() {
        let mut app = outline_app("just text\n```\n# not a heading\n```\n");
        app.editor.move_cursor(caret(2, 4));
        open_outline(&mut app);
        let palette = app.palette.as_ref().unwrap();
        assert!(palette.results.is_empty());
        assert_eq!(palette.active, None);
        key(&mut app, crate::keys::Nav::Enter);
        assert!(app.palette.is_some());
        assert_eq!(app.editor.cursor(), caret(2, 4));
    }

    #[test]
    fn the_outline_needs_an_open_note_in_a_workspace() {
        let (mut app, _) = App::new(super::super::Options::default());
        let outcome = app.open_palette(Mode::Outline);
        assert_eq!(outcome.processed.disposition, Disposition::Blocked);
        assert!(app.palette.is_none());
        assert!(app.outline_origin.is_none());
    }

    fn grouped(note: u64, group: &'static str, path: &str) -> ResultRow {
        ResultRow {
            group,
            path: path.into(),
            ..row(note)
        }
    }

    #[test]
    fn overflowing_lists_end_on_a_whole_row_and_never_on_a_header() {
        let header = GROUP_HEADER_HEIGHT;
        let results = [
            grouped(1, "Note", ""),
            grouped(2, "Note", ""),
            grouped(3, "Note", ""),
            grouped(4, "Go", ""),
            grouped(5, "Go", "a/b"),
        ];
        let third = 6.0 + header + 3.0 * (2.0 + 44.0);
        let go_header = third + 2.0 + header;
        let fourth = go_header + 2.0 + 44.0;
        let fifth = fourth + 2.0 + 54.0;
        let close = |available: f32, expected: f32| {
            let snapped = snapped_list_height(&results, available);
            assert!(
                (snapped - expected).abs() < 0.01,
                "{available}: {snapped} != {expected}"
            );
        };
        close(go_header + 10.0, third);
        close(fourth + 0.01, fourth);
        close(fifth - 1.0, fourth);
        assert_eq!(
            snapped_list_height(&results, fifth + 6.0),
            fifth + 6.0,
            "a list that fits keeps its height"
        );
        assert_eq!(
            snapped_list_height(&results, 40.0),
            40.0,
            "a window too short for one row keeps the minimum"
        );
    }

    #[test]
    fn ungrouped_rows_snap_without_headers() {
        let results: Vec<_> = (0..20).map(row).collect();
        let snapped = snapped_list_height(&results, 400.0);
        assert_eq!(snapped, 6.0 + 8.0 * 46.0 - 2.0);
        assert_eq!(snapped_list_height(&[], 400.0), 400.0);
    }

    #[test]
    fn removed_reorder_anchor_stays_selected_but_disabled_until_an_explicit_query_reset() {
        let source = ItemTarget {
            generation: 7,
            item: ItemRef::Section(wren_core::model::SectionId(1)),
            parent: None,
        };
        let before = ItemTarget {
            generation: 7,
            item: ItemRef::Section(wren_core::model::SectionId(2)),
            parent: None,
        };
        let key = Key::Reorder(ReorderTarget {
            source,
            before: Some(before),
        });
        let anchor = action_row(
            key.clone(),
            "Before second".into(),
            String::new(),
            true,
            Action::Reorder(ReorderTarget {
                source,
                before: Some(before),
            }),
        );
        let append = action_row(
            Key::Reorder(ReorderTarget {
                source,
                before: None,
            }),
            "Move to end".into(),
            String::new(),
            true,
            Action::Reorder(ReorderTarget {
                source,
                before: None,
            }),
        );
        let mut palette = Palette::new(Mode::Reorder(source.item), 7);
        palette.replace(vec![anchor, append.clone()], true);
        palette.replace(vec![append.clone()], false);
        let selected = &palette.results[palette.active.unwrap()];
        assert_eq!(selected.key, key);
        assert!(!selected.enabled);
        assert!(palette.error.is_some());
        palette.replace(vec![append], true);
        assert!(palette.results[palette.active.unwrap()].enabled);
        assert!(matches!(
            palette.results[0].key,
            Key::Reorder(ReorderTarget { before: None, .. })
        ));
    }

    #[test]
    fn refresh_preserves_stable_key_but_query_resets_selection() {
        let mut palette = Palette::new(Mode::Jump, 1);
        palette.replace(vec![row(1), row(2), row(3)], true);
        palette.step(1);
        palette.replace(vec![row(3), row(1), row(2)], false);
        assert_eq!(palette.active, Some(2));
        palette.replace(vec![row(3), row(1), row(2)], true);
        assert_eq!(palette.active, Some(0));
        palette.step(-100);
        assert_eq!(palette.active, Some(0));
        palette.step(100);
        assert_eq!(palette.active, Some(2));
        palette.replace(vec![], false);
        assert_eq!(palette.active, None);
    }
}
