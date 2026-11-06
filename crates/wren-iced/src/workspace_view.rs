use iced::widget::{button, column, container, opaque, row, scrollable, stack, text, text_input};
use iced::{Border, Element, Fill};
use wren_core::model::{ItemRef, Parent};
use wren_core::ops::Op;
use wren_core::theme::{self as definitions, Rgb, Theme};

use super::{workspace::DialogKind, App, Message};
use crate::{fonts, theme};

impl App {
    pub(super) fn workspace_view(&self) -> Element<'_, Message> {
        let colors = self.definition.colors;
        let loading = self.loading.is_some();
        let enabled = !loading && self.session.is_some();
        let mut path = text_input("/absolute/workspace/directory", &self.workspace_path);
        if !loading {
            path = path
                .on_input(Message::WorkspacePath)
                .on_submit(Message::OpenWorkspace);
        }
        let header = panel(
            row![
                text("Wren").size(20).color(theme::color(colors.accent)),
                path,
                button(if loading { "Opening…" } else { "Open" })
                    .on_press_maybe((!loading).then_some(Message::OpenWorkspace)),
                button("Back").on_press_maybe(
                    (enabled
                        && self
                            .session
                            .as_ref()
                            .is_some_and(|session| session.can_back()))
                    .then_some(Message::Core(Op::Back))
                ),
                button("Forward").on_press_maybe(
                    (enabled
                        && self
                            .session
                            .as_ref()
                            .is_some_and(|session| session.can_forward()))
                    .then_some(Message::Core(Op::Forward))
                ),
                button("System").on_press(Message::ChooseTheme(None)),
                button("Dark").on_press(Message::ChooseTheme(Some(definitions::DARK))),
                button("Light").on_press(Message::ChooseTheme(Some(definitions::LIGHT))),
            ]
            .spacing(8)
            .align_y(iced::Center)
            .into(),
            colors.titlebar,
            colors.divider,
        )
        .padding([10, 14]);
        let mut body = column![header];
        if let Some(banner) = &self.banner {
            body = body.push(
                panel(
                    row![
                        text(banner).size(13).color(theme::color(colors.warning)),
                        iced::widget::space::horizontal(),
                        button("Retry save")
                            .on_press_maybe(enabled.then_some(Message::Core(Op::Flush))),
                        button("Dismiss").on_press(Message::DismissBanner),
                    ]
                    .spacing(10)
                    .align_y(iced::Center)
                    .into(),
                    colors.raised,
                    colors.divider,
                )
                .padding([8, 14]),
            );
        }
        let mut sections = column![
            text(format!(
                "Sections · {}",
                self.model().map_or(0, |model| model.sections.len())
            ))
            .size(15),
            button("New section")
                .on_press_maybe(enabled.then_some(Message::ShowDialog(DialogKind::CreateSection)))
                .width(Fill),
        ]
        .spacing(8);
        let mut section_items = column![].spacing(3);
        if let Some(model) = self.model() {
            for section in &model.sections {
                let count = model
                    .notes
                    .iter()
                    .filter(|note| match note.parent {
                        Parent::Section(id) => id == section.id,
                        Parent::Group(id) => model
                            .group(id)
                            .is_some_and(|group| group.section == section.id),
                    })
                    .count();
                section_items = section_items.push(item(
                    format!("{}  ({count})", section.name),
                    enabled.then_some(Message::SelectParent(Parent::Section(section.id))),
                    Some(section.id) == self.selected_section,
                    self.definition,
                ));
            }
        }
        sections = sections.push(scrollable(section_items).height(Fill));
        if let Some(section) = self.selected_section {
            sections = sections.push(
                row![
                    button("Rename").on_press_maybe(enabled.then_some(Message::ShowDialog(
                        DialogKind::Rename(ItemRef::Section(section))
                    ))),
                    button("Trash").on_press_maybe(enabled.then_some(Message::ShowDialog(
                        DialogKind::Delete(ItemRef::Section(section))
                    ))),
                ]
                .spacing(4),
            );
        }
        let mut groups = column![
            text(format!(
                "Groups · {}",
                self.model().map_or(0, |model| model
                    .groups
                    .iter()
                    .filter(|group| Some(group.section) == self.selected_section)
                    .count())
            ))
            .size(15),
            button("New group")
                .on_press_maybe(
                    self.selected_section
                        .filter(|_| enabled)
                        .map(|section| Message::ShowDialog(DialogKind::CreateGroup(section)))
                )
                .width(Fill),
        ]
        .spacing(8);
        let mut group_items = column![].spacing(3);
        if let (Some(model), Some(section)) = (self.model(), self.selected_section) {
            let count = model
                .notes
                .iter()
                .filter(|note| note.parent == Parent::Section(section))
                .count();
            group_items = group_items.push(item(
                format!("No Group  ({count})"),
                enabled.then_some(Message::SelectParent(Parent::Section(section))),
                self.selected_group.is_none(),
                self.definition,
            ));
            for group in model.groups.iter().filter(|group| group.section == section) {
                let count = model
                    .notes
                    .iter()
                    .filter(|note| note.parent == Parent::Group(group.id))
                    .count();
                group_items = group_items.push(item(
                    format!("{}  ({count})", group.name),
                    enabled.then_some(Message::SelectParent(Parent::Group(group.id))),
                    Some(group.id) == self.selected_group,
                    self.definition,
                ));
            }
        }
        groups = groups.push(scrollable(group_items).height(Fill));
        if let Some(group) = self.selected_group {
            groups = groups.push(button("Rename group").width(Fill).on_press_maybe(
                enabled.then_some(Message::ShowDialog(DialogKind::Rename(ItemRef::Group(
                    group,
                )))),
            ));
            groups = groups.push(
                row![
                    button("Move").on_press_maybe(
                        enabled.then_some(Message::ShowDialog(DialogKind::MoveGroup(group)))
                    ),
                    button("Trash").on_press_maybe(enabled.then_some(Message::ShowDialog(
                        DialogKind::Delete(ItemRef::Group(group))
                    ))),
                ]
                .spacing(4),
            );
        }
        let parent = self.selected_parent();
        let count = self.model().map_or(0, |model| {
            model
                .notes
                .iter()
                .filter(|note| Some(note.parent) == parent)
                .count()
        });
        let mut notes = column![
            text(format!("Notes · {count}")).size(15),
            button("New note")
                .on_press_maybe(
                    parent
                        .filter(|_| enabled)
                        .map(|parent| Message::ShowDialog(DialogKind::CreateNote(parent)))
                )
                .width(Fill),
        ]
        .spacing(8);
        let mut note_items = column![].spacing(3);
        if let Some(model) = self.model() {
            for note in model
                .notes
                .iter()
                .filter(|note| Some(note.parent) == parent)
            {
                let mut label = note.title.clone();
                if let Some(subtitle) = &note.subtitle {
                    label.push('\n');
                    label.push_str(subtitle);
                }
                note_items = note_items.push(item(
                    label,
                    enabled.then_some(Message::Core(Op::OpenNote {
                        note: Some(note.id),
                    })),
                    Some(note.id) == self.selected_note,
                    self.definition,
                ));
            }
        }
        notes = notes.push(scrollable(note_items).height(Fill));
        if let Some(note) = self.selected_note {
            notes = notes.push(button("Rename note").width(Fill).on_press_maybe(
                enabled.then_some(Message::ShowDialog(DialogKind::Rename(ItemRef::Note(note)))),
            ));
            notes =
                notes.push(
                    row![
                        button("Move").on_press_maybe(
                            enabled.then_some(Message::ShowDialog(DialogKind::MoveNote(note)))
                        ),
                        button("Trash").on_press_maybe(enabled.then_some(Message::ShowDialog(
                            DialogKind::Delete(ItemRef::Note(note))
                        ))),
                    ]
                    .spacing(4),
                );
        }
        let mut editor = column![panel(
            text(self.breadcrumb())
                .size(13)
                .color(theme::color(colors.muted))
                .into(),
            colors.editor,
            colors.divider,
        )
        .padding([10, 16])];
        if self.find_open {
            editor = editor.push(
                container(
                    row![
                        text_input("Find literal text", &self.find_query)
                            .id("find-query")
                            .on_input(Message::FindChanged)
                            .on_submit(Message::FindNext),
                        text(format!(
                            "{} / {}",
                            self.editor.find_current().map_or(0, |n| n + 1),
                            self.editor.find_count()
                        )),
                        button("Previous").on_press(Message::FindPrevious),
                        button("Next").on_press(Message::FindNext),
                        button("Close").on_press(Message::FindClose),
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                )
                .padding([8, 16]),
            );
        }
        if self.selected_note.is_some() {
            editor = editor.push(self.editor.view(self.definition).map(Message::Editor));
        } else {
            editor = editor.push(
                container(
                    text(if loading {
                        "Opening workspace…"
                    } else if self.session.is_none() {
                        "Open a workspace directory to browse saved notes."
                    } else if self.selected_section.is_none() {
                        "Create a section to start writing."
                    } else {
                        "Select a note, or create one in this location."
                    })
                    .color(theme::color(colors.muted)),
                )
                .center_x(Fill)
                .center_y(Fill),
            );
        }
        let (status, status_color) = self.save_status();
        let cursor = self.editor.cursor();
        editor =
            editor.push(
                container(
                    row![
                        text(format!("Markdown · {} words", self.words)),
                        iced::widget::space::horizontal(),
                        text(format!(
                            "Ln {}, byte col {}",
                            cursor.position.line + 1,
                            cursor.position.column + 1
                        ))
                        .font(fonts::EDITOR)
                        .size(12),
                        text(status).color(theme::color(status_color)),
                        button(
                            if self.session.as_ref().is_some_and(|session| session
                                .pending_durability()
                                || session
                                    .document()
                                    .is_some_and(|document| document.error.is_some()))
                            {
                                "Retry save"
                            } else {
                                "Save"
                            }
                        )
                        .on_press_maybe(
                            (enabled && self.selected_note.is_some())
                                .then_some(Message::Core(Op::Flush))
                        ),
                    ]
                    .spacing(14)
                    .align_y(iced::Center),
                )
                .padding([8, 14]),
            );
        body = body.push(
            row![
                panel(sections.into(), colors.pane, colors.divider)
                    .padding(10)
                    .width(180)
                    .height(Fill),
                panel(groups.into(), colors.pane, colors.divider)
                    .padding(10)
                    .width(190)
                    .height(Fill),
                panel(notes.into(), colors.notes, colors.divider)
                    .padding(10)
                    .width(240)
                    .height(Fill),
                editor.width(Fill).height(Fill),
            ]
            .height(Fill),
        );
        let base: Element<'_, Message> = body.width(Fill).height(Fill).into();
        if let Some(dialog) = &self.dialog {
            let dialog = self.dialog_view(dialog);
            let overlay =
                container(dialog)
                    .center_x(Fill)
                    .center_y(Fill)
                    .style(|_| container::Style {
                        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
                        ..Default::default()
                    });
            stack![base, opaque(overlay)].into()
        } else {
            base
        }
    }

    fn breadcrumb(&self) -> String {
        let mut parts = Vec::new();
        if let Some(section) = self
            .selected_section
            .and_then(|id| self.model()?.section(id))
        {
            parts.push(section.name.as_str());
            parts.push(
                self.selected_group
                    .and_then(|id| self.model()?.group(id))
                    .map_or("No Group", |group| group.name.as_str()),
            );
        }
        if let Some(note) = self.selected_note.and_then(|id| self.model()?.note(id)) {
            parts.push(note.title.as_str());
        }
        if parts.is_empty() {
            "Workspace".into()
        } else {
            parts.join("  /  ")
        }
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
            DialogKind::CreateSection => ("Create section".to_string(), "Create"),
            DialogKind::CreateGroup(_) => ("Create group".to_string(), "Create"),
            DialogKind::CreateNote(_) => ("Create note".to_string(), "Create"),
            DialogKind::Rename(_) => ("Rename item".to_string(), "Rename"),
            DialogKind::Delete(target) => (
                format!(
                    "Move “{}” to trash?",
                    self.item_name(target).unwrap_or("item")
                ),
                "Move to Trash",
            ),
            DialogKind::MoveNote(_) => ("Move note".to_string(), "Move"),
            DialogKind::MoveGroup(_) => ("Move group".to_string(), "Move"),
        };
        let mut content = column![text(heading).size(22)].spacing(16);
        match dialog.kind {
            DialogKind::Delete(_) => {
                content = content.push(text("This moves the item and its contents to the operating system trash. It does not permanently delete them.").color(theme::color(colors.muted)));
            }
            DialogKind::MoveNote(_) | DialogKind::MoveGroup(_) => {
                content =
                    content.push(text("Choose a destination").color(theme::color(colors.muted)));
                let mut destinations = column![].spacing(4);
                if let Some(model) = self.model() {
                    for section in &model.sections {
                        let parent = Parent::Section(section.id);
                        let label = if matches!(dialog.kind, DialogKind::MoveNote(_)) {
                            format!("{} / No Group", section.name)
                        } else {
                            section.name.clone()
                        };
                        destinations = destinations.push(item(
                            label,
                            Some(Message::DialogDestination(parent)),
                            dialog.destination == Some(parent),
                            self.definition,
                        ));
                        if matches!(dialog.kind, DialogKind::MoveNote(_)) {
                            for group in model
                                .groups
                                .iter()
                                .filter(|group| group.section == section.id)
                            {
                                let parent = Parent::Group(group.id);
                                destinations = destinations.push(item(
                                    format!("{} / {}", section.name, group.name),
                                    Some(Message::DialogDestination(parent)),
                                    dialog.destination == Some(parent),
                                    self.definition,
                                ));
                            }
                        }
                    }
                }
                content = content.push(scrollable(destinations).height(260));
            }
            _ => {
                content = content.push(
                    text_input("Name", &dialog.name)
                        .id("item-name")
                        .on_input(Message::DialogName)
                        .on_submit(Message::SubmitDialog),
                );
            }
        }
        if let Some(error) = &dialog.error {
            content = content.push(text(error).color(theme::color(colors.danger)));
        }
        content = content.push(
            row![
                iced::widget::space::horizontal(),
                button("Cancel").on_press(Message::CancelDialog),
                button(submit).on_press(if matches!(dialog.kind, DialogKind::Delete(_)) {
                    Message::ConfirmTrash
                } else {
                    Message::SubmitDialog
                }),
            ]
            .spacing(8),
        );
        panel(content.into(), colors.raised, colors.dialog_border)
            .padding(24)
            .width(520)
            .into()
    }
}

fn panel(
    content: Element<'_, Message>,
    background: Rgb,
    border: Rgb,
) -> container::Container<'_, Message> {
    container(content).style(move |_| container::Style {
        background: Some(theme::color(background).into()),
        border: Border {
            color: theme::color(border),
            width: 1.0,
            ..Default::default()
        },
        ..Default::default()
    })
}

fn item<'a>(
    label: String,
    message: Option<Message>,
    selected: bool,
    definition: Theme,
) -> Element<'a, Message> {
    button(text(label).size(13))
        .on_press_maybe(message)
        .width(Fill)
        .padding([9, 8])
        .style(move |_, status| {
            let colors = definition.colors;
            let background = if selected {
                colors.selected
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                colors.raised
            } else {
                colors.pane
            };
            button::Style {
                background: Some(theme::color(background).into()),
                text_color: theme::color(colors.text),
                border: Border {
                    color: theme::color(if selected {
                        colors.selected_border
                    } else {
                        background
                    }),
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
}
