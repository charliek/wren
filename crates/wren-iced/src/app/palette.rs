use std::ops::Range;
use std::path::PathBuf;

use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input,
};
use iced::{Element, Fill, Task};
use wren_core::index::{fuzzy_score, QueryKind, Scope};
use wren_core::model::{GroupId, ItemRef, NoteId, Parent};
use wren_core::ops::Op;

use super::commands::{
    item_commands, Command, CommandId, Disposition, Gateway, ItemTarget, ReorderTarget,
    DEFINITIONS, GROUPS,
};
use super::{focus_editor, App, Message};
use crate::ellipsis::ellipsis;
use crate::{chrome, fonts, palette_scroll, theme};
use wren_core::theme as definitions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Commands,
    Jump,
    Search,
    Workspaces,
    MoveNote(NoteId),
    MoveGroup(GroupId),
    ItemActions(ItemRef),
    Reorder(ItemRef),
    Themes,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Jump => "jump",
            Self::Search => "search",
            Self::Workspaces => "workspaces",
            Self::MoveNote(_) => "move_note",
            Self::MoveGroup(_) => "move_group",
            Self::ItemActions(_) => "item_actions",
            Self::Reorder(_) => "reorder",
            Self::Themes => "themes",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::Commands => "Type a command…",
            Self::Jump => "Jump to a note…",
            Self::Search => "Search all notes in this workspace",
            Self::Workspaces => "Find a workspace…",
            Self::MoveNote(_) | Self::MoveGroup(_) => "Choose a destination…",
            Self::ItemActions(_) => "Choose an action for this item…",
            Self::Reorder(_) => "Choose a position among all siblings…",
            Self::Themes => "Choose a theme…",
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
            Self::Themes => Some("Theme"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Command(CommandId),
    Note(NoteId),
    Destination(Parent),
    ItemAction {
        target: ItemTarget,
        id: CommandId,
    },
    Reorder(ReorderTarget),
    Workspace(PathBuf),
    /// `None` follows the system appearance.
    Theme(Option<&'static str>),
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
    Destination(Parent),
    ItemCommand { target: ItemTarget, id: CommandId },
    Reorder(ReorderTarget),
    Workspace(PathBuf),
    Theme(Option<&'static str>),
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
        let retained = selected
            .as_ref()
            .and_then(|selected| results.iter().position(|row| row.key == selected.key));
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
        self.active = retained.or_else(|| (!results.is_empty()).then_some(0));
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
        if !matches!(mode, Mode::Commands | Mode::Workspaces | Mode::Themes)
            && self.session.is_none()
        {
            return self.blocked("Open a workspace first");
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
        let mut palette = Palette::new(mode, self.workspace_generation());
        palette.source = source;
        self.palette = Some(palette);
        self.refresh_palette(true);
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
        self.finish_action(Disposition::Completed, None, focus_editor())
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
                    for definition in DEFINITIONS
                        .iter()
                        .filter(|definition| definition.group == group)
                    {
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
                        });
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
                            path: entry.path.to_string_lossy().into_owned(),
                            snippet: String::new(),
                            highlight: None,
                            content_hit: false,
                            group: "",
                            shortcut: String::new(),
                            current: self.model().is_some_and(|model| model.root == entry.path),
                            enabled: true,
                            action: Action::Workspace(entry.path.clone()),
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
                        Mode::MoveNote(note) => model.note(note).map(|note| note.parent),
                        Mode::MoveGroup(group) => model
                            .group(group)
                            .map(|group| Parent::Section(group.section)),
                        _ => None,
                    };
                    for section in &model.sections {
                        let parent = Parent::Section(section.id);
                        let title = if matches!(mode, Mode::MoveNote(_)) {
                            format!("{} › (No group)", section.name)
                        } else {
                            section.name.clone()
                        };
                        if current != Some(parent) && fuzzy_score(&query, &title).is_some() {
                            rows.push(destination_row(
                                parent,
                                title,
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
            Action::Note(target) => self.dispatch(Command::OpenNote(target)),
            Action::ItemCommand { target, id } => self.dispatch(Command::ItemAction { target, id }),
            Action::Reorder(target) => self.dispatch(Command::Reorder(target)),
            Action::Theme(id) => self.choose_theme(id),
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
                        Mode::MoveNote(note) => self.process_core(Op::MoveNote { note, to }),
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
            if target.content_match.is_some() {
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
        let top =
            (if search { 72.0_f32 } else { 96.0_f32 }).min((self.viewport.height * 0.16).max(8.0));
        let list_height =
            (self.viewport.height - top - 134.0).clamp(40.0, if search { 460.0 } else { 400.0 });
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
        let mut results = column![].spacing(2).padding(6);
        let mut last_group = "";
        for (index, result) in palette.results.iter().enumerate() {
            if result.group != last_group && !result.group.is_empty() {
                results = results.push(
                    container(
                        text(result.group.to_uppercase())
                            .size(11)
                            .font(fonts::SEMIBOLD)
                            .color(theme::color(colors.muted)),
                    )
                    .padding([8, 10]),
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
                label = label.push(ellipsis(
                    result.path.replace('/', " › "),
                    11.0,
                    theme::color(colors.muted),
                ));
            }
            let mut contents = row![label.width(Fill)]
                .spacing(12)
                .align_y(iced::Center)
                .clip(true);
            if search {
                contents = contents.push(chrome::label(
                    if result.content_hit { "text" } else { "title" },
                    11,
                    colors.faint,
                ));
            }
            if result.current {
                contents =
                    contents.push(text("current").size(11).color(theme::color(colors.accent)));
            }
            if !result.shortcut.is_empty() {
                contents = contents.push(chrome::keycap(&result.shortcut, self.definition));
            }
            let active = palette.active == Some(index);
            let row = button(container(contents).center_y(Fill))
                .height(if search {
                    76
                } else if result.path.is_empty() {
                    44
                } else {
                    54
                })
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
                .push(hint("↵", if search { "open at match" } else { "run" }))
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
            .width(if search { 720.0 } else { 620.0 })
            .max_width((self.viewport.width - 32.0).max(120.0))
            .clip(true)
            .style(move |_| chrome::dialog(self.definition));
        let scrim = mouse_area(
            container(iced::widget::space())
                .width(Fill)
                .height(Fill)
                .style(move |_| container::Style {
                    background: Some(iced::Color::from_rgba(0.025, 0.028, 0.04, 0.60).into()),
                    ..Default::default()
                }),
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
                .center_x(Fill)
                .height(Fill)
        ]
        .into()
    }
}

pub(super) fn result_id(generation: u64, key: &Key) -> iced::advanced::widget::Id {
    let key = match key {
        Key::Command(id) => id.key().into(),
        Key::Note(note) => format!("note-{}", note.0),
        Key::Destination(parent) => format!("parent-{parent:?}"),
        Key::ItemAction { target, id } => format!("item-{target:?}-{}", id.key()),
        Key::Reorder(target) => format!("reorder-{target:?}"),
        Key::Workspace(path) => format!("workspace-{}", path.display()),
        Key::Theme(id) => format!("theme-{}", id.unwrap_or("system")),
    };
    iced::advanced::widget::Id::from(format!("palette-row-{generation}-{key}"))
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
        }
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
