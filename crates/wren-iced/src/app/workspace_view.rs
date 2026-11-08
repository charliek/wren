use iced::widget::{
    button, column, container, mouse_area, opaque, responsive, row, scrollable, stack, text,
    text_input,
};
use iced::{Border, Element, Fill, Shrink};
use wren_core::model::{GroupId, Note, Parent, SectionId, Workspace};
use wren_core::ops::Op;
use wren_core::theme::Rgb;

use super::columns::Column;
use super::commands::{Command, CommandId};
use super::navigation::matches_name;
use super::palette::Mode;
use super::{workspace::DialogKind, App, Message};
use crate::chrome::{self, Icon};
use crate::drag::RowKind;
use crate::drag_view::Registry;
use crate::ellipsis::ellipsis;
use crate::panes::{self, Pane};
use crate::{fonts, theme};

impl App {
    /// The width of the browser columns left of the editor; 0 when hidden.
    pub(super) fn browser_width(&self) -> f32 {
        if !self.workspace_mode || self.focus || self.model().is_none() {
            return 0.0;
        }
        self.widths
            .effective(self.visible_panes(), self.viewport.width)
            .browser()
    }

    pub(super) fn workspace_view(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let enabled = self.session.is_some()
            && self.loading.is_none()
            && self.palette.is_none()
            && self.dialog.is_none();
        let mut registry = self
            .model()
            .map(Registry::new)
            .unwrap_or_else(|| Registry::new(&Workspace::empty(std::path::PathBuf::new())));
        let effective = self
            .widths
            .effective(self.visible_panes(), self.viewport.width);
        let mut browser = row![].height(Fill).clip(true);
        let browser_visible = !self.focus && self.model().is_some();
        if browser_visible {
            for &pane in self.visible_panes() {
                let content = match pane {
                    Pane::Sections => self.sections_pane(&mut registry),
                    Pane::Groups => self.groups_pane(&mut registry),
                    Pane::Notes => self.notes_pane(&mut registry),
                };
                let generation = self.gesture_generation;
                browser = browser.push(panes::resizable(
                    content,
                    pane,
                    effective.get(pane),
                    self.widths.get(pane),
                    self.definition,
                    move |event| Message::Resize(generation, event),
                ));
            }
        }
        let browser_width = self.browser_width();
        // Reset browser-owned capture/scroll state only on a committed workspace
        // replacement. The editor remains the same sibling throughout.
        let browser = iced::widget::keyed_column([(self.workspace_generation(), browser.into())])
            .height(Fill);
        let content = row![
            container(browser)
                .width(browser_width)
                .height(Fill)
                .clip(true),
            self.saved_editor_view()
        ]
        .height(Fill)
        .clip(true);
        let generation = self.gesture_generation;
        let content = crate::drag_view::area(
            content,
            registry,
            generation,
            enabled,
            self.definition,
            move |event| Message::Drag(generation, event),
        );
        let base = column![self.titlebar(), self.warning_slot(), content]
            .width(Fill)
            .height(Fill);
        let base = container(base)
            .style(move |_| chrome::surface(self.definition, colors.editor, 0.0, false))
            .into();
        let base = self.palette_view(base);
        if let Some(dialog) = &self.dialog {
            let scrim = mouse_area(
                container(iced::widget::space())
                    .width(Fill)
                    .height(Fill)
                    .style(|_| container::Style {
                        background: Some(iced::Color::from_rgba(0.025, 0.028, 0.04, 0.60).into()),
                        ..Default::default()
                    }),
            )
            .on_press(Message::CancelDialog);
            stack![
                base,
                opaque(scrim),
                container(opaque(self.dialog_view(dialog)))
                    .center_x(Fill)
                    .center_y(Fill)
                    .padding(16)
            ]
            .into()
        } else {
            base
        }
    }

    fn titlebar(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let root = self.model().map(|model| model.root.as_path());
        let name = root
            .and_then(|root| {
                self.config
                    .as_ref()
                    .and_then(|config| config.workspaces().iter().find(|entry| entry.path == root))
                    .map(|entry| entry.name.as_str())
            })
            .or_else(|| {
                root.and_then(|root| root.file_name())
                    .and_then(|name| name.to_str())
            })
            .unwrap_or("Choose workspace");
        let workspace = button(
            container(
                row![
                    chrome::icon(Icon::Folder, 15.0, colors.accent),
                    chrome::label(name, 13, colors.text).font(fonts::SEMIBOLD),
                    ellipsis(
                        root.map(chrome::display_path).unwrap_or_default(),
                        11.0,
                        theme::color(colors.muted)
                    )
                    .font(fonts::EDITOR)
                    .width(Shrink),
                    chrome::icon(Icon::Chevron, 14.0, colors.muted)
                ]
                .spacing(8)
                .align_y(iced::Center)
                .clip(true),
            )
            .center_y(Fill),
        )
        .on_press(Message::Command(Command::Run(CommandId::SwitchWorkspace)))
        .height(32)
        .padding([0, 10])
        .clip(true)
        .style(move |_, status| chrome::control(self.definition, status, false, false));
        let mut left = row![
            chrome::icon(Icon::Bird, 18.0, colors.accent),
            text("Wren").size(14).font(fonts::BOLD),
            workspace
        ]
        .spacing(10)
        .align_y(iced::Center)
        .clip(true);
        if self.viewport.width < 1100.0 {
            left = left.width(Fill);
        } else {
            left = left.width(570);
        }
        let compact = self.viewport.width < 1120.0;
        let mut right = row![].spacing(6).align_y(iced::Center);
        if !compact {
            right = right.push(self.scoped_button(
                match self.column {
                    Column::Sections => "Jump to section",
                    Column::Groups => "Jump to group",
                    Column::Notes | Column::Editor => "Jump to note",
                },
                CommandId::Jump,
                Command::Scoped(CommandId::Jump),
            ));
        }
        right = right.push(self.shortcut_button("Search", CommandId::Search));
        right = right.push(
            self.shortcut_button("Commands", CommandId::CommandPalette)
                .on_press(Message::PaletteOpen(Mode::Commands)),
        );
        right = right.push(
            button(chrome::icon(
                Icon::Focus,
                15.0,
                if self.focus {
                    colors.accent
                } else {
                    colors.body
                },
            ))
            .on_press_maybe(
                self.session
                    .as_ref()
                    .map(|_| Message::Command(Command::Focus(None))),
            )
            .width(32)
            .height(32)
            .padding(8)
            .style(move |_, status| chrome::control(self.definition, status, self.focus, false)),
        );
        chrome::bottom_rule(
            container(
                row![left, iced::widget::space::horizontal(), right]
                    .spacing(12)
                    .align_y(iced::Center)
                    .clip(true),
            )
            .padding([7, 12])
            .height(47)
            .width(Fill)
            .style(move |_| chrome::surface(self.definition, colors.titlebar, 0.0, false)),
            self.definition,
        )
        .into()
    }

    fn shortcut_button<'a>(&'a self, label: &'a str, id: CommandId) -> button::Button<'a, Message> {
        self.scoped_button(label, id, Command::Run(id))
    }

    fn scoped_button<'a>(
        &'a self,
        label: &'a str,
        id: CommandId,
        command: Command,
    ) -> button::Button<'a, Message> {
        let mut contents = row![text(label).size(12)].spacing(8).align_y(iced::Center);
        if let Some(shortcut) = self.shortcut_label(id.key()) {
            contents = contents.push(chrome::keycap_owned(shortcut, self.definition));
        }
        button(container(contents).center_y(Fill))
            .height(32)
            .padding([0, 10])
            .on_press_maybe(
                self.command_enabled(id)
                    .then_some(Message::Command(command)),
            )
            .style(move |_, status| chrome::control(self.definition, status, false, false))
    }

    fn warning_slot(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let message = self
            .banner
            .as_deref()
            .or(self.registry_warning.as_deref())
            .unwrap_or_default();
        let retry = self.session.as_ref().is_some_and(|session| {
            session.pending_durability()
                || session
                    .document()
                    .is_some_and(|document| document.error.is_some())
        });
        let mut contents = row![chrome::label(message, 12, colors.warning).width(Fill)]
            .spacing(10)
            .align_y(iced::Center)
            .clip(true);
        if retry {
            contents = contents.push(self.small_button("Retry save", Message::Core(Op::Flush)));
        }
        if self.command_enabled(CommandId::OpenConflictCopy) {
            contents = contents.push(self.small_button(
                "Open conflict copy",
                Message::Command(Command::Run(CommandId::OpenConflictCopy)),
            ));
        }
        contents = contents.push(self.small_button("Dismiss", Message::DismissBanner));
        container(contents)
            .padding([6, 14])
            .height(if message.is_empty() { 0 } else { 40 })
            .width(Fill)
            .clip(true)
            .style(move |_| chrome::surface(self.definition, colors.raised, 0.0, false))
            .into()
    }

    fn pane_header<'a>(
        &'a self,
        label: String,
        command: CommandId,
        column: Column,
    ) -> Element<'a, Message> {
        let colors = self.definition.colors;
        let active = self.column == column;
        // The active column's header turns accent with a leading dot, so the
        // keyboard's place reads at a glance.
        let title = chrome::label(
            label.to_uppercase(),
            11,
            if active { colors.accent } else { colors.muted },
        )
        .font(fonts::SEMIBOLD);
        let title: Element<'a, Message> = if active {
            let dot = container(iced::widget::space())
                .width(6)
                .height(6)
                .style(move |_| chrome::surface(self.definition, colors.accent, 3.0, false));
            row![dot, title]
                .spacing(6)
                .align_y(iced::Center)
                .width(Fill)
                .into()
        } else {
            title.width(Fill).into()
        };
        container(
            row![
                title,
                button(chrome::icon(Icon::Plus, 14.0, colors.muted))
                    .width(26)
                    .height(26)
                    .padding(6)
                    .on_press_maybe(
                        self.command_enabled(command)
                            .then_some(Message::Command(Command::Run(command)))
                    )
                    .style(move |_, status| chrome::control(self.definition, status, false, true))
            ]
            .spacing(4)
            .align_y(iced::Center)
            .clip(true),
        )
        .height(46)
        .padding(iced::Padding {
            top: 10.0,
            right: 10.0,
            bottom: 8.0,
            left: 14.0,
        })
        .into()
    }

    fn pane_filter<'a>(
        &'a self,
        pane: &'static str,
        placeholder: &'a str,
        query: &'a str,
    ) -> Element<'a, Message> {
        let input = text_input(placeholder, query)
            .id(format!("filter-{pane}"))
            .size(12)
            .padding(iced::Padding {
                top: 7.0,
                right: 9.0,
                bottom: 7.0,
                left: 28.0,
            })
            .on_input(move |query| {
                Message::Command(Command::Filter {
                    pane: pane.into(),
                    query,
                })
            })
            .style(move |_, status| text_input::Style {
                background: iced::Color::TRANSPARENT.into(),
                ..chrome::input(self.definition, status, true)
            });
        // The icon and the field's fill sit under a transparent input, not on
        // top of it: a Stack stops updating its children once an event is
        // captured, so anything above the input would hide from it a press
        // another pane already captured, and it would keep its focus.
        let backdrop = container(chrome::icon(
            Icon::Search,
            13.0,
            self.definition.colors.faint,
        ))
        .padding([8, 9])
        .width(Fill)
        .height(Fill)
        .style(move |_| container::Style {
            background: Some(theme::color(self.definition.colors.input).into()),
            border: Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
        container(stack![backdrop, input])
            .padding(iced::Padding {
                top: 0.0,
                right: 10.0,
                bottom: 8.0,
                left: 10.0,
            })
            .height(38)
            .into()
    }

    /// `‹ Section › Group` for the folded columns; each crumb, and `‹`, opens
    /// its column again.
    fn breadcrumb(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let folded = self.folded_columns();
        let mut crumbs = row![].spacing(2).align_y(iced::Center).clip(true);
        if let Some(&last) = folded.last() {
            crumbs = crumbs.push(
                button(chrome::icon(Icon::Back, 14.0, colors.muted))
                    .width(26)
                    .height(26)
                    .padding(6)
                    .on_press(Message::Command(Command::OpenColumn(last)))
                    .style(move |_, status| chrome::control(self.definition, status, false, true)),
            );
        }
        for (index, &column) in folded.iter().enumerate() {
            if index > 0 {
                crumbs = crumbs.push(chrome::label("›", 13, colors.muted));
            }
            let name = match column {
                Column::Sections => self
                    .selected_section
                    .and_then(|id| self.model()?.section(id))
                    .map_or("Sections", |section| section.name.as_str()),
                _ => self
                    .selected_group
                    .and_then(|id| self.model()?.group(id))
                    .map_or("Groups", |group| group.name.as_str()),
            };
            crumbs = crumbs.push(
                button(
                    container(
                        ellipsis(name, 13.0, theme::color(colors.text))
                            .font(fonts::MEDIUM)
                            .width(iced::Length::Shrink),
                    )
                    .max_width(150),
                )
                .height(26)
                .padding([4, 8])
                .on_press(Message::Command(Command::OpenColumn(column)))
                .style(move |_, status| chrome::control(self.definition, status, false, true)),
            );
        }
        column![
            container(crumbs)
                .height(40)
                .width(Fill)
                .align_y(iced::Center)
                .padding([0, 8]),
            container(iced::widget::space())
                .width(Fill)
                .height(1)
                .style(move |_| chrome::surface(self.definition, colors.divider, 0.0, false))
        ]
        .into()
    }

    fn pane_shell<'a>(&'a self, content: Element<'a, Message>, pane: Pane) -> Element<'a, Message> {
        let colors = self.definition.colors;
        let notes = pane == Pane::Notes;
        // The leftmost shown pane carries the breadcrumb for folded ones.
        let content = if self.fold > 0 && self.visible_panes().first() == Some(&pane) {
            column![self.breadcrumb(), content].height(Fill).into()
        } else {
            content
        };
        row![
            container(content)
                .width(Fill)
                .height(Fill)
                .clip(true)
                .style(move |_| chrome::surface(
                    self.definition,
                    if notes { colors.notes } else { colors.pane },
                    0.0,
                    false
                )),
            container(iced::widget::space())
                .width(1)
                .height(Fill)
                .style(move |_| chrome::surface(self.definition, colors.divider, 0.0, false))
        ]
        .height(Fill)
        .into()
    }

    fn sections_pane(&self, registry: &mut Registry) -> Element<'_, Message> {
        let mut items = column![].spacing(2).padding([0, 8]);
        if let Some(model) = self.model() {
            for section in model
                .sections
                .iter()
                .filter(|section| matches_name(&self.filters.sections, &section.name))
            {
                items = items.push(self.tree_row(
                    registry,
                    RowKind::Section(section.id),
                    &section.name,
                    self.section_count(section.id),
                    self.selected_section == Some(section.id),
                ));
            }
        }
        self.pane_shell(
            column![
                self.pane_header("Sections".into(), CommandId::NewSection, Column::Sections),
                self.pane_filter("sections", "Filter sections", &self.filters.sections),
                self.pane_scroll(items, "sections-list")
            ]
            .height(Fill)
            .into(),
            Pane::Sections,
        )
    }

    fn groups_pane(&self, registry: &mut Registry) -> Element<'_, Message> {
        let mut items = column![].spacing(2).padding([0, 8]);
        let section = self.browser_section();
        let mut count = 0;
        if let (Some(model), Some(section)) = (self.model(), section) {
            for group in model.groups.iter().filter(|group| {
                group.section == section && matches_name(&self.filters.groups, &group.name)
            }) {
                items = items.push(self.tree_row(
                    registry,
                    RowKind::Group(group.id),
                    &group.name,
                    group_count(model, group.id),
                    self.selected_group == Some(group.id),
                ));
                count += 1;
            }
        }
        if count == 0 {
            items = items.push(self.empty_label(if section.is_none() {
                "Create a section first."
            } else if self.filters.groups.trim().is_empty() {
                "No groups yet."
            } else {
                "No groups match."
            }));
        }
        let heading = section
            .and_then(|id| self.model()?.section(id))
            .map_or_else(
                || "Groups".into(),
                |section| format!("Groups · {}", section.name),
            );
        self.pane_shell(
            column![
                self.pane_header(heading, CommandId::NewGroup, Column::Groups),
                self.pane_filter("groups", "Filter groups", &self.filters.groups),
                self.pane_scroll(items, "groups-list")
            ]
            .height(Fill)
            .into(),
            Pane::Groups,
        )
    }

    fn notes_pane(&self, registry: &mut Registry) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let mut items = column![].spacing(2).padding([0, 8]);
        let hits = self.note_hits();
        for hit in &hits {
            if let Some(note) = self.model().and_then(|model| model.note(hit.note)) {
                let preview = if self.filters.notes.trim().is_empty() {
                    self.session
                        .as_ref()
                        .and_then(|session| session.index().preview(note.id))
                        .unwrap_or(&hit.snippet.text)
                } else {
                    &hit.snippet.text
                };
                let preview = preview.split_whitespace().collect::<Vec<_>>().join(" ");
                let sub = format!("{} · {}", modified_label(note), preview);
                let mut label = column![
                    ellipsis(&note.title, 13.0, theme::color(colors.text)).font(fonts::MEDIUM)
                ]
                .spacing(3)
                .clip(true);
                if self.filters.notes.trim().is_empty() {
                    label = label.push(ellipsis(sub, 12.0, theme::color(colors.muted)));
                } else {
                    label = label.push(
                        row![
                            chrome::label(format!("{} ·", modified_label(note)), 12, colors.muted),
                            super::palette::highlighted_text(
                                hit.snippet.text.clone(),
                                hit.snippet.highlight.clone(),
                                self.definition
                            )
                        ]
                        .spacing(4)
                        .clip(true),
                    );
                }
                if let Some(path) = &note.subtitle {
                    label = label.push(chrome::label(path, 11, colors.faint).font(fonts::EDITOR));
                }
                let selected = Some(note.id) == self.selected_note;
                items = items.push(
                    container(label)
                        .id(registry.row(RowKind::Note(note.id)))
                        .width(Fill)
                        .height(if note.subtitle.is_some() { 70 } else { 54 })
                        .padding([8, 10])
                        .clip(true)
                        .style(move |_| self.row_style(selected, Column::Notes)),
                );
            }
        }
        if hits.is_empty() {
            let parent = self.browser_parent();
            items = items.push(self.empty_label(if parent.is_none() {
                "Create a section first.".to_string()
            } else if self.spring_section.is_some() {
                "Choose a group.".to_string()
            } else if !matches!(parent, Some(Parent::Group(_))) {
                "Create a group first.".to_string()
            } else if self.filters.notes.is_empty() {
                self.shortcut_label(CommandId::NewNote.key()).map_or_else(
                    || "No notes here yet.".to_string(),
                    |shortcut| format!("No notes here yet. Press {shortcut}."),
                )
            } else {
                "No matches.".to_string()
            }));
        }
        let parent = self.browser_parent();
        let label = parent.and_then(|parent| match parent {
            Parent::Section(id) => self
                .model()?
                .section(id)
                .map(|section| section.name.as_str()),
            Parent::Group(id) => self.model()?.group(id).map(|group| group.name.as_str()),
        });
        let heading = label.map_or_else(|| "Notes".into(), |name| format!("Notes · {name}"));
        let next_sort = if self.note_sort() == "modified" {
            "title"
        } else {
            "modified"
        };
        let sort = button(chrome::label(
            format!("Sorted by {}", self.note_sort()),
            11,
            colors.muted,
        ))
        .padding(0)
        .on_press_maybe(
            matches!(parent, Some(Parent::Group(_)))
                .then(|| Message::Command(Command::Sort(next_sort.into()))),
        )
        .style(move |_, status| chrome::control(self.definition, status, false, true));
        let footer = chrome::top_rule(
            container(
                row![
                    sort,
                    iced::widget::space::horizontal(),
                    chrome::label(
                        self.shortcut_label(CommandId::NewDatedNote.key())
                            .map_or_else(String::new, |shortcut| format!("{shortcut} dated")),
                        11,
                        colors.muted
                    )
                ]
                .spacing(6)
                .clip(true),
            )
            .padding([8, 14]),
            self.definition,
        );
        self.pane_shell(
            column![
                self.pane_header(heading, CommandId::NewNote, Column::Notes),
                self.pane_filter("notes", "Search titles and text", &self.filters.notes),
                self.pane_scroll(items, "notes-list"),
                footer
            ]
            .height(Fill)
            .into(),
            Pane::Notes,
        )
    }

    fn tree_row<'a>(
        &'a self,
        registry: &mut Registry,
        kind: RowKind,
        name: &'a str,
        count: usize,
        selected: bool,
    ) -> Element<'a, Message> {
        let colors = self.definition.colors;
        let column = match kind {
            RowKind::Section(_) => Column::Sections,
            RowKind::Group(_) => Column::Groups,
            RowKind::Note(_) => Column::Notes,
        };
        let content = row![
            ellipsis(name, 13.0, theme::color(colors.text)),
            chrome::label(count.to_string(), 11, colors.muted)
        ]
        .spacing(8)
        .align_y(iced::Center)
        .clip(true);
        container(content)
            .id(registry.row(kind))
            .height(36)
            .width(Fill)
            .align_y(iced::Center)
            .padding([0, 10])
            .clip(true)
            .style(move |_| self.row_style(selected, column))
            .into()
    }

    /// The selected row in the active column gets an accent border; selected
    /// rows elsewhere keep the quieter selection border.
    fn row_style(&self, selected: bool, column: Column) -> container::Style {
        let colors = self.definition.colors;
        container::Style {
            background: selected.then(|| theme::color(colors.selected).into()),
            border: Border {
                color: if !selected {
                    iced::Color::TRANSPARENT
                } else if self.column == column {
                    theme::color(colors.accent)
                } else {
                    theme::color(colors.selected_border)
                },
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        }
    }

    fn section_count(&self, section: SectionId) -> usize {
        self.model().map_or(0, |model| {
            model
                .notes
                .iter()
                .filter(|note| {
                    model
                        .group(note.group)
                        .is_some_and(|group| group.section == section)
                })
                .count()
        })
    }

    fn pane_scroll<'a>(
        &'a self,
        content: iced::widget::Column<'a, Message>,
        id: &'static str,
    ) -> Element<'a, Message> {
        scrollable(content)
            .id(id)
            .height(Fill)
            .style(move |_, status| chrome::scrollbar(self.definition, status))
            .into()
    }

    fn empty_label<'a>(
        &'a self,
        label: impl iced::widget::text::IntoFragment<'a>,
    ) -> Element<'a, Message> {
        container(chrome::label(label, 12, self.definition.colors.muted))
            .padding([10, 6])
            .into()
    }

    fn editor_header(&self, width: f32) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let mut crumbs = row![].spacing(6).align_y(iced::Center).clip(true);
        for (id, icon) in [
            (CommandId::Back, Icon::Back),
            (CommandId::Forward, Icon::Forward),
        ] {
            crumbs = crumbs.push(
                button(chrome::icon(icon, 13.0, colors.muted))
                    .padding(3)
                    .width(20)
                    .on_press_maybe(
                        self.command_enabled(id)
                            .then_some(Message::Command(Command::Run(id))),
                    )
                    .style(move |_, status| chrome::control(self.definition, status, false, true)),
            );
        }
        // Narrow editors keep the note title and actions; the parents and
        // the edited time go first.
        let roomy = width >= 480.0;
        if let Some(section) = self
            .selected_section
            .and_then(|id| self.model()?.section(id))
            .filter(|_| width >= 420.0)
        {
            crumbs = crumbs
                .push(ellipsis(&section.name, 12.0, theme::color(colors.muted)).width(Shrink))
                .push(chrome::label("›", 12, colors.faint));
        }
        if let Some(group) = self
            .selected_group
            .and_then(|id| self.model()?.group(id))
            .filter(|_| width >= 420.0)
        {
            crumbs = crumbs
                .push(ellipsis(&group.name, 12.0, theme::color(colors.muted)).width(Shrink))
                .push(chrome::label("›", 12, colors.faint));
        }
        let note = self.selected_note.and_then(|id| self.model()?.note(id));
        crumbs = crumbs.push(
            ellipsis(
                note.map_or("No note selected", |note| note.title.as_str()),
                12.0,
                theme::color(colors.text),
            )
            .font(fonts::SEMIBOLD)
            .width(Shrink),
        );
        let actions = row![
            chrome::label(
                note.filter(|_| roomy)
                    .map(|note| format!("Edited {}", modified_label(note)))
                    .unwrap_or_default(),
                12,
                colors.muted
            ),
            self.small_button(
                "Move to…",
                Message::Command(Command::Run(CommandId::MoveNote))
            )
            .on_press_maybe(
                self.selected_note
                    .map(|_| Message::Command(Command::Run(CommandId::MoveNote)))
            ),
            button(chrome::icon(Icon::More, 15.0, colors.muted))
                .on_press(Message::PaletteOpen(Mode::Commands))
                .padding(4)
                .style(move |_, status| chrome::control(self.definition, status, false, true))
        ]
        .spacing(8)
        .align_y(iced::Center)
        .clip(true);
        row![crumbs.width(Fill), actions]
            .spacing(8)
            .align_y(iced::Center)
            .clip(true)
            .into()
    }

    fn saved_editor_view(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let header = chrome::bottom_rule(
            container(responsive(move |size| self.editor_header(size.width)))
                .height(49)
                .padding([10, 16]),
            self.definition,
        );
        let find = container(
            row![
                text_input("Find literal text", &self.find_query)
                    .id("find-query")
                    .on_input(Message::FindChanged)
                    .on_submit(Message::FindNext)
                    .size(12)
                    .padding(6)
                    .style(move |_, status| chrome::input(self.definition, status, true)),
                text(format!(
                    "{} / {}",
                    self.editor.find_current().map_or(0, |n| n + 1),
                    self.editor.find_count()
                ))
                .size(11),
                self.small_button("Previous", Message::FindPrevious),
                self.small_button("Next", Message::FindNext),
                self.small_button("Close", Message::FindClose)
            ]
            .spacing(8)
            .align_y(iced::Center),
        )
        .padding([6, 16])
        .height(if self.find_open { 44 } else { 0 })
        .clip(true);
        let (style, palette) = theme::editor_colors(self.definition);
        let editor: Element<'_, Message> = self
            .editor
            .view(style, palette, self.editor_max_width())
            .map(Message::Editor);
        let mut empty = column![].spacing(14).align_x(iced::Center);
        let (prompt, action, command) = if self.selected_section.is_none() {
            (
                "Create a section to start writing.",
                "New section…",
                CommandId::NewSection,
            )
        } else if self.selected_group.is_none() {
            (
                "Create a group to start writing.",
                "New group…",
                CommandId::NewGroup,
            )
        } else {
            (
                "Select a note, or create one here.",
                "New note…",
                CommandId::NewNote,
            )
        };
        if self.selected_note.is_none() {
            empty = empty.push(
                text(if self.loading.is_some() {
                    "Opening workspace…"
                } else if self.session.is_none() {
                    "Your notes. Your folders."
                } else {
                    prompt
                })
                .size(18)
                .color(theme::color(colors.muted)),
            );
            if self.loading.is_none() {
                if self.session.is_none() {
                    empty = empty.push(
                        text("Open an existing Markdown workspace or choose an empty folder.")
                            .size(13)
                            .color(theme::color(colors.muted)),
                    );
                    empty = empty.push(
                        row![
                            self.small_button(
                                "Add workspace…",
                                Message::Command(Command::Run(CommandId::AddWorkspace))
                            ),
                            self.small_button(
                                "Switch workspace…",
                                Message::Command(Command::Run(CommandId::SwitchWorkspace))
                            )
                        ]
                        .spacing(10),
                    );
                } else {
                    empty = empty
                        .push(self.small_button(action, Message::Command(Command::Run(command))));
                }
            }
        }
        let empty = container(empty)
            .center_x(Fill)
            .center_y(Fill)
            .style(move |_| chrome::surface(self.definition, colors.editor, 0.0, false));
        let document: Element<'_, Message> = if self.selected_note.is_none() {
            stack![editor, opaque(empty)].into()
        } else {
            stack![editor].into()
        };
        column![
            header,
            find,
            document,
            chrome::top_rule(
                container(responsive(move |size| self.editor_footer(size.width)))
                    .height(29)
                    .padding([6, 16]),
                self.definition
            )
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn editor_footer(&self, width: f32) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let (status, status_color) = self.save_status();
        let cursor = self.editor.cursor();
        let stats = if width >= 440.0 {
            format!(
                "Markdown · {} lines · {} words",
                self.editor.line_count(),
                self.words
            )
        } else {
            String::new()
        };
        let mut footer = row![
            chrome::label(stats, 11, colors.muted),
            iced::widget::space::horizontal(),
            chrome::label(
                format!(
                    "Ln {}, Col {}",
                    cursor.position.line + 1,
                    cursor.position.column + 1
                ),
                11,
                colors.muted
            ),
            chrome::label(format!("● {status}"), 11, status_color)
        ]
        .spacing(16)
        .align_y(iced::Center)
        .clip(true);
        if self.session.as_ref().is_some_and(|session| {
            session
                .document()
                .is_some_and(|document| document.error.is_some())
                || session.pending_durability()
        }) {
            footer = footer.push(self.small_button("Retry save", Message::Core(Op::Flush)));
        }
        footer.into()
    }

    fn small_button<'a>(&'a self, label: &'a str, message: Message) -> button::Button<'a, Message> {
        button(text(label).size(12))
            .on_press(message)
            .padding([5, 9])
            .style(move |_, status| chrome::control(self.definition, status, false, false))
    }

    fn save_status(&self) -> (&str, Rgb) {
        let colors = self.definition.colors;
        if self.loading.is_some() {
            return ("Loading", colors.muted);
        }
        if let Some(session) = &self.session {
            if session
                .document()
                .is_some_and(|document| document.error.is_some())
            {
                return ("Save error · retry", colors.danger);
            }
            if session.pending_durability() {
                return ("Durability pending", colors.warning);
            }
            if let Some(document) = session.document() {
                return if document.dirty() {
                    ("Unsaved changes", colors.warning)
                } else {
                    ("Saved", colors.success)
                };
            }
        }
        ("No note selected", colors.muted)
    }

    fn dialog_view<'a>(&'a self, dialog: &'a super::workspace::Dialog) -> Element<'a, Message> {
        let colors = self.definition.colors;
        let (heading, submit) = match dialog.kind {
            DialogKind::CreateSection => ("New section".into(), "Create"),
            DialogKind::CreateGroup(_) => ("New group".into(), "Create"),
            DialogKind::CreateNote(_) => ("New note".into(), "Create"),
            DialogKind::Rename(_) => ("Rename item".into(), "Rename"),
            DialogKind::Delete(target) => (
                format!(
                    "Move “{}” to trash?",
                    self.item_name(target).unwrap_or("item")
                ),
                "Move to Trash",
            ),
        };
        let mut content = column![text(heading).size(20).font(fonts::SEMIBOLD)].spacing(16);
        if let DialogKind::Delete(target) = dialog.kind {
            let count = self.model().map_or(0, |model| {
                model.path(target).map_or(0, |path| {
                    model
                        .notes
                        .iter()
                        .filter(|note| note.path.as_path().starts_with(path.as_path()))
                        .count()
                })
            });
            content = content.push(text(format!("This item contains {count} {}. It will move to the operating system trash, not be permanently deleted. Use the button below to confirm; Return alone never confirms.", if count == 1 { "note" } else { "notes" })).size(13).color(theme::color(colors.muted)));
        } else {
            content = content.push(
                text_input("Name", &dialog.name)
                    .id("item-name")
                    .on_input(Message::DialogName)
                    .on_submit(Message::SubmitDialog)
                    .padding(9)
                    .style(move |_, status| chrome::input(self.definition, status, true)),
            );
        }
        if let Some(error) = &dialog.error {
            content = content.push(text(error).size(12).color(theme::color(colors.danger)));
        }
        content = content.push(
            row![
                iced::widget::space::horizontal(),
                self.small_button("Cancel", Message::CancelDialog),
                self.small_button(
                    submit,
                    if matches!(dialog.kind, DialogKind::Delete(_)) {
                        Message::ConfirmTrash
                    } else {
                        Message::SubmitDialog
                    }
                )
            ]
            .spacing(8),
        );
        container(content)
            .padding(24)
            .width(520)
            .max_width((self.viewport.width - 32.0).max(120.0))
            .style(move |_| chrome::dialog(self.definition))
            .into()
    }
}

fn group_count(model: &Workspace, group: GroupId) -> usize {
    model
        .notes
        .iter()
        .filter(|note| note.group == group)
        .count()
}

fn modified_label(note: &Note) -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(note.modified)
        .unwrap_or_default()
        .as_secs();
    if seconds < 60 {
        "just now".into()
    } else if seconds < 3600 {
        format!("{} min ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{} h ago", seconds / 3600)
    } else {
        format!("{} d ago", seconds / 86400)
    }
}
