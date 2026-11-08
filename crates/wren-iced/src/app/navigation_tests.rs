use std::sync::Arc;

use crate::editor_view;
use iced::widget::text_editor::{Action, Edit, Motion};
use wren_core::config::Config;
use wren_core::fs_store::Fault;
use wren_core::model::{GroupId, ItemRef, NoteId, Parent};
use wren_core::ops::{Op, Session};

use super::commands::{Command, CommandId, Disposition};
use super::palette::{Mode, SearchTarget};
use super::workspace::DialogKind;
use super::{App, Message, Options};
use crate::panes::Pane;

fn fixture() -> (tempfile::TempDir, App) {
    let directory = crate::test_support::tempdir();
    for path in [
        "Alpha",
        "Alpha/Group",
        "Alpha/Inbox",
        "Beta",
        "Beta/Destination",
    ] {
        std::fs::create_dir(directory.path().join(path)).unwrap();
    }
    for (path, text) in [
        ("Alpha/Inbox/First.md", "first body"),
        ("Alpha/Group/Second.md", "second body"),
        ("Beta/Destination/Third.md", "third body"),
    ] {
        std::fs::write(directory.path().join(path), text).unwrap();
    }
    let (mut app, _) = App::new(Options {
        editor: true,
        ..Options::default()
    });
    app.workspace_mode = true;
    app.session = Some(Session::open(directory.path()).unwrap());
    app.sync_editor(true);
    app.hydrate_navigation();
    let first = note(&app, "First");
    let _ = app.process_core(Op::OpenNote { note: Some(first) });
    app.reveal_selection();
    (directory, app)
}

fn note(app: &App, title: &str) -> NoteId {
    app.model()
        .unwrap()
        .notes
        .iter()
        .find(|note| note.title == title)
        .unwrap()
        .id
}
fn group(app: &App, name: &str) -> GroupId {
    app.model()
        .unwrap()
        .groups
        .iter()
        .find(|group| group.name == name)
        .unwrap()
        .id
}
fn append(app: &mut App, text: &str) {
    let _ = app.edit(editor_view::Message::Action(Action::Move(
        Motion::DocumentEnd,
    )));
    let _ = app.edit(editor_view::Message::Action(Action::Edit(Edit::Paste(
        Arc::new(text.into()),
    ))));
}
fn target(app: &App, note: NoteId, query: &str) -> SearchTarget {
    SearchTarget {
        generation: app.workspace_generation(),
        index_revision: app.session.as_ref().unwrap().index().revision(),
        note,
        query: query.into(),
        content_match: (!query.is_empty()).then_some(0..1),
    }
}

#[test]
fn command_context_is_explicit_and_palette_groups_filter_without_empty_headers() {
    let (_directory, mut app) = fixture();
    assert!(app.command_enabled(CommandId::RenameNote));
    assert!(app.command_enabled(CommandId::RenameGroup));
    assert!(app.command_enabled(CommandId::RenameSection));
    let _ = app.open_palette(Mode::Commands);
    let _ = app.palette_query("Reset column widths".into());
    let palette = app.palette.as_ref().unwrap();
    assert_eq!(palette.results.len(), 1);
    assert_eq!(palette.results[0].group, "View");
    assert_eq!(palette.results[0].title, "Reset column widths");
    let _ = app.activate_palette(None);
    assert!(app.palette.is_none());
}

#[test]
fn one_modal_owns_keyboard_and_neither_typing_nor_global_creation_leaks_to_editor() {
    let (_directory, mut app) = fixture();
    let before = app.editor.text();
    let count = app.model().unwrap().notes.len();
    let _ = app.open_palette(Mode::Commands);
    append(&mut app, "not an edit");
    let _ = app.update(super::preferences::shortcut(CommandId::NewDatedNote));
    assert_eq!(app.editor.text(), before);
    assert_eq!(app.model().unwrap().notes.len(), count);
    assert!(app.dialog.is_none());
    let _ = app.palette_query("New note".into());
    let _ = app.update(Message::Key(crate::keys::Input::Nav(
        crate::keys::Nav::Enter,
    )));
    assert!(app.palette.is_none());
    assert!(app.dialog.is_some());
    let _ = app.update(super::preferences::shortcut(CommandId::CommandPalette));
    assert!(app.palette.is_none());
    let _ = app.update(Message::Escape);
    assert!(app.dialog.is_none());
}

#[test]
fn filters_are_pure_and_focus_theme_retain_editor_history_cursor_and_selection() {
    let (_directory, mut app) = fixture();
    append(&mut app, " retained");
    let cursor = app.editor.cursor();
    let selected = app.selected_note;
    let _ = app.dispatch(Command::Filter {
        pane: "notes".into(),
        query: "nothing matches".into(),
    });
    assert!(app.note_hits().is_empty());
    assert_eq!(app.selected_note, selected);
    assert_eq!(app.editor.cursor(), cursor);
    let _ = app.dispatch(Command::Focus(Some(true)));
    let _ = app.update(Message::ChooseTheme(Some(wren_core::theme::LIGHT)));
    let _ = app.dispatch(Command::Focus(Some(false)));
    assert_eq!(app.editor.cursor(), cursor);
    assert!(app.editor.can_undo());
    assert_eq!(app.filters.notes, "nothing matches");
    assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
    assert_eq!(app.selected_note, selected);
}

#[test]
fn explicit_navigation_clears_only_filters_that_hide_its_destination() {
    let (_directory, mut app) = fixture();
    let _ = app.dispatch(Command::Filter {
        pane: "sections".into(),
        query: "Alpha".into(),
    });
    let _ = app.dispatch(Command::Filter {
        pane: "groups".into(),
        query: "Group".into(),
    });
    let _ = app.dispatch(Command::Filter {
        pane: "notes".into(),
        query: "Second".into(),
    });
    let second = note(&app, "Second");
    let result = app.dispatch(Command::OpenNote(target(&app, second, "")));
    assert_eq!(result.processed.disposition, Disposition::Completed);
    assert_eq!(app.filters.sections, "Alpha");
    assert_eq!(app.filters.groups, "Group");
    assert_eq!(app.filters.notes, "Second");
    let third = note(&app, "Third");
    let _ = app.dispatch(Command::OpenNote(target(&app, third, "")));
    assert_eq!(app.filters.sections, "");
    assert_eq!(app.filters.groups, "");
    assert_eq!(app.filters.notes, "");
}

#[test]
fn jumping_back_to_another_group_clears_only_a_conflicting_groups_filter() {
    for query in ["Destination", "Inbox"] {
        let (_directory, mut app) = fixture();
        let first = note(&app, "First");
        let second = note(&app, "Second");
        let inbox = group(&app, "Inbox");
        let _ = app.dispatch(Command::OpenNote(target(&app, second, "")));
        app.filters.sections = "Alpha".into();
        app.filters.groups = query.into();
        app.filters.notes = "body".into();
        let expected = if query == "Inbox" { query } else { "" };
        let result = app.dispatch(Command::Run(CommandId::Back));
        assert_eq!(result.processed.disposition, Disposition::Completed);
        assert_eq!(app.selected_note, Some(first));
        assert_eq!(app.selected_group, Some(inbox));
        assert_eq!(app.filters.groups, expected);
        assert_eq!(app.filters.sections, "Alpha");
        assert_eq!(app.filters.notes, "body");

        let _ = app.dispatch(Command::OpenNote(target(&app, second, "")));
        app.filters.groups = query.into();
        let result = app.dispatch(Command::OpenNote(target(&app, first, "")));
        assert_eq!(result.processed.disposition, Disposition::Completed);
        assert_eq!(app.filters.groups, expected);
        assert_eq!(app.filters.sections, "Alpha");
        assert_eq!(app.filters.notes, "body");
    }
}

#[test]
fn moving_into_a_filtered_out_group_reveals_it_without_clearing_other_filters() {
    let (_directory, mut app) = fixture();
    let second = note(&app, "Second");
    let inbox = group(&app, "Inbox");
    let _ = app.dispatch(Command::OpenNote(target(&app, second, "")));
    app.filters.sections = "Alpha".into();
    app.filters.groups = "Group".into();
    app.filters.notes = "Second".into();
    let _ = app.open_palette(Mode::MoveNote(second));
    let destination = super::palette::Key::Destination(Parent::Group(inbox));
    let result = app.activate_palette_key(app.workspace_generation(), destination);
    assert_eq!(result.processed.disposition, Disposition::Completed);
    assert_eq!(app.selected_group, Some(inbox));
    assert_eq!(app.filters.groups, "");
    assert_eq!(app.filters.sections, "Alpha");
    assert_eq!(app.filters.notes, "Second");
}

#[test]
fn a_section_without_groups_is_selectable_but_blocks_note_creation_and_sorting() {
    let (directory, mut app) = fixture();
    std::fs::create_dir(directory.path().join("Empty")).unwrap();
    let _ = app.process_core(Op::Reconcile);
    let empty = app
        .model()
        .unwrap()
        .sections
        .iter()
        .find(|section| section.name == "Empty")
        .unwrap()
        .id;
    let result = app.dispatch(Command::SelectSection(empty));
    assert!(result.processed.disposition.committed());
    assert_eq!(app.selected_section, Some(empty));
    assert_eq!(app.selected_group, None);
    assert_eq!(app.selected_note, None);
    assert!(app.note_hits().is_empty());
    assert!(app.command_enabled(CommandId::NewGroup));
    for id in [
        CommandId::NewNote,
        CommandId::NewDatedNote,
        CommandId::SortModified,
        CommandId::SortTitle,
        CommandId::RenameGroup,
    ] {
        assert!(!app.command_enabled(id), "{id:?}");
    }
    let count = app.model().unwrap().notes.len();
    for id in [CommandId::NewNote, CommandId::NewDatedNote] {
        let result = app.dispatch(Command::Run(id));
        assert_eq!(result.processed.disposition, Disposition::Blocked);
        assert_eq!(
            result.processed.warning.as_deref(),
            Some("Create a group first; notes live in groups")
        );
        assert!(app.dialog.is_none());
    }
    assert_eq!(
        app.dispatch(Command::Sort("title".into()))
            .processed
            .disposition,
        Disposition::Blocked
    );
    assert_eq!(app.model().unwrap().notes.len(), count);

    let _ = app.dispatch(Command::SelectParent(Parent::Group(group(&app, "Inbox"))));
    assert!(app.command_enabled(CommandId::NewNote));
    let _ = app.dispatch(Command::Run(CommandId::NewNote));
    assert!(matches!(
        app.dialog.as_ref().unwrap().kind,
        DialogKind::CreateNote(id) if id == group(&app, "Inbox")
    ));
}

#[test]
fn passive_palette_refresh_returns_a_reveal_task_only_when_result_keys_or_active_position_change() {
    for remove_active in [false, true] {
        let (directory, mut app) = fixture();
        let _ = app.open_palette(Mode::Jump);
        let requested = app.palette_step(2).processed;
        let palette = app.palette.as_ref().unwrap();
        assert_eq!(palette.active, Some(2));
        let active_key = palette.results[2].key.clone();
        let removed = if remove_active { 2 } else { 1 };
        std::fs::remove_file(directory.path().join(&palette.results[removed].path)).unwrap();
        app.session.as_mut().unwrap().apply(Op::Reconcile).unwrap();

        let task = app.update(Message::Tick);
        let palette = app.palette.as_ref().unwrap();
        assert_eq!(palette.results.len(), 2);
        if remove_active {
            assert_eq!(palette.active, Some(0));
        } else {
            assert_eq!(palette.active, Some(1));
            assert_eq!(palette.results[1].key, active_key);
        }
        assert!(
            task.units() > 0,
            "passive result changes must reach the runtime"
        );
        let processed = app.last_processed.as_ref().unwrap();
        assert_eq!(processed.correlation, requested.correlation);
        assert_eq!(processed.disposition, requested.disposition);
        assert_eq!(app.update(Message::Tick).units(), 0);

        let revision = app.palette.as_ref().unwrap().revision;
        let result = app.process_core(Op::EditDraft {
            note: app.selected_note.unwrap(),
            contents: "changed draft with unchanged jump results".into(),
        });
        assert!(app.palette.as_ref().unwrap().revision > revision);
        assert_eq!(result.task.units(), 0);
        assert_eq!(
            app.last_processed.as_ref().unwrap().correlation,
            requested.correlation
        );
    }
}

fn assert_empty_reorder_reconciliation(groups: bool) {
    let (directory, mut app) = fixture();
    let revision = app.session.as_ref().unwrap().index().revision();
    let parent = if groups { "Alpha/" } else { "" };
    for name in ["A", "B", "C"] {
        std::fs::create_dir(directory.path().join(format!("{parent}{name}"))).unwrap();
    }
    let setup = app.process_core(Op::Reconcile);
    assert_eq!(setup.processed.disposition, Disposition::Completed);
    assert_eq!(app.session.as_ref().unwrap().index().revision(), revision);
    let source = if groups {
        ItemRef::Group(
            app.model()
                .unwrap()
                .groups
                .iter()
                .find(|group| group.name == "C")
                .unwrap()
                .id,
        )
    } else {
        ItemRef::Section(
            app.model()
                .unwrap()
                .sections
                .iter()
                .find(|section| section.name == "C")
                .unwrap()
                .id,
        )
    };
    let _ = app.open_palette(Mode::Reorder(source));
    let _ = app.palette_query("  Before  ".into());
    let active = app
        .palette
        .as_ref()
        .unwrap()
        .results
        .iter()
        .position(|row| row.title == "Before B")
        .unwrap();
    let _ = app.palette_step(active as i32);
    let palette = app.palette.as_ref().unwrap();
    assert_eq!(palette.active, Some(active));
    assert_eq!(palette.results[active].title, "Before B");
    assert!(palette.results[active].enabled);
    let selected = palette.results[active].key.clone();

    std::fs::remove_dir(directory.path().join(format!("{parent}B"))).unwrap();
    std::fs::create_dir(directory.path().join(format!("{parent}D"))).unwrap();
    let outcome = app.process_core(Op::Reconcile);
    assert_eq!(outcome.processed.disposition, Disposition::Completed);
    assert_eq!(app.session.as_ref().unwrap().index().revision(), revision);
    let palette = app.palette.as_ref().unwrap();
    assert_eq!(palette.revision, revision);
    assert_eq!(palette.query, "  Before  ");
    assert_eq!(palette.active, Some(active));
    let tombstone = &palette.results[active];
    assert_eq!(tombstone.key, selected);
    assert_eq!(tombstone.title, "Before B");
    assert!(
        !tombstone.enabled,
        "the removed anchor must become a tombstone"
    );
    assert!(palette.error.is_some());
    assert!(palette
        .results
        .iter()
        .any(|row| row.title == "Before D" && row.enabled && row.key != selected));
    assert!(outcome.task.units() > 0, "changed results must be revealed");
    assert_eq!(app.process_core(Op::Reconcile).task.units(), 0);
    assert_eq!(
        app.activate_palette(None).processed.disposition,
        Disposition::Blocked
    );
    assert_eq!(app.palette.as_ref().unwrap().results[active].key, selected);
}

#[test]
fn empty_tree_group_reconciliation_refreshes_the_active_reorder_anchor() {
    assert_empty_reorder_reconciliation(true);
}

#[test]
fn empty_tree_section_reconciliation_refreshes_the_active_reorder_anchor() {
    assert_empty_reorder_reconciliation(false);
}

#[test]
fn empty_tree_reconciliation_refreshes_move_destinations_without_an_index_revision() {
    for move_group in [false, true] {
        let (directory, mut app) = fixture();
        let revision = app.session.as_ref().unwrap().index().revision();
        for path in [
            "Empty",
            "Empty/Old",
            "Empty/Shelf",
            "Empty section",
            "Empty section 2",
        ] {
            std::fs::create_dir(directory.path().join(path)).unwrap();
        }
        let setup = app.process_core(Op::Reconcile);
        assert_eq!(setup.processed.disposition, Disposition::Completed);
        assert_eq!(app.session.as_ref().unwrap().index().revision(), revision);
        // Groups move between sections; notes move only between groups.
        let (mode, chosen, removed, added) = if move_group {
            (
                Mode::MoveGroup(app.model().unwrap().groups[0].id),
                "Empty section 2",
                "Empty section",
                "Empty section 3",
            )
        } else {
            (
                Mode::MoveNote(app.selected_note.unwrap()),
                "Empty/Shelf",
                "Empty/Old",
                "Empty/Zed",
            )
        };
        let _ = app.open_palette(mode);
        let _ = app.palette_query("  Empty  ".into());
        let palette = app.palette.as_ref().unwrap();
        assert!(palette
            .results
            .iter()
            .all(|row| row.path.contains('/') != move_group));
        let active = palette
            .results
            .iter()
            .position(|row| row.path == chosen)
            .unwrap();
        let selected = palette.results[active].key.clone();
        let _ = app.palette_step(active as i32);

        std::fs::remove_dir(directory.path().join("Empty/Old")).unwrap();
        std::fs::remove_dir(directory.path().join("Empty section")).unwrap();
        std::fs::create_dir(directory.path().join("Empty/Zed")).unwrap();
        std::fs::create_dir(directory.path().join("Empty section 3")).unwrap();
        let outcome = app.process_core(Op::Reconcile);
        assert_eq!(outcome.processed.disposition, Disposition::Completed);
        assert_eq!(app.session.as_ref().unwrap().index().revision(), revision);
        let palette = app.palette.as_ref().unwrap();
        assert_eq!(palette.revision, revision);
        assert_eq!(palette.mode, mode);
        assert_eq!(palette.query, "  Empty  ");
        assert!(palette.results.iter().all(|row| row.path != removed));
        assert!(palette
            .results
            .iter()
            .any(|row| row.path == added && row.enabled));
        assert!(palette
            .results
            .iter()
            .all(|row| row.path.contains('/') != move_group));
        assert_eq!(palette.active, Some(active - 1));
        assert_eq!(palette.results[palette.active.unwrap()].key, selected);
        assert!(outcome.task.units() > 0, "changed results must be revealed");
        assert_eq!(app.process_core(Op::Reconcile).task.units(), 0);
    }
}

#[test]
fn empty_tree_item_context_refreshes_after_rename_move_and_removal() {
    for group in [false, true] {
        let (directory, mut app) = fixture();
        let revision = app.session.as_ref().unwrap().index().revision();
        std::fs::create_dir(directory.path().join("Empty")).unwrap();
        if group {
            std::fs::create_dir(directory.path().join("Empty/Child")).unwrap();
        }
        let setup = app.process_core(Op::Reconcile);
        assert_eq!(setup.processed.disposition, Disposition::Completed);
        assert_eq!(app.session.as_ref().unwrap().index().revision(), revision);
        let item = if group {
            ItemRef::Group(
                app.model()
                    .unwrap()
                    .groups
                    .iter()
                    .find(|group| group.name == "Child")
                    .unwrap()
                    .id,
            )
        } else {
            ItemRef::Section(
                app.model()
                    .unwrap()
                    .sections
                    .iter()
                    .find(|section| section.name == "Empty")
                    .unwrap()
                    .id,
            )
        };
        let _ = app.open_palette(Mode::ItemActions(item));
        let _ = app.palette_query("  Rename  ".into());
        let selected = app.palette.as_ref().unwrap().results[0].key.clone();
        let renamed = app.process_core(Op::Rename {
            target: item,
            new_name: "Renamed".into(),
        });
        assert_eq!(renamed.processed.disposition, Disposition::Completed);
        assert_eq!(app.session.as_ref().unwrap().index().revision(), revision);
        let palette = app.palette.as_ref().unwrap();
        assert_eq!(palette.query, "  Rename  ");
        assert_eq!(palette.active, Some(0));
        assert_eq!(palette.results[0].key, selected);
        assert_eq!(palette.results[0].path, "Renamed");
        assert!(palette.results[0].enabled);
        assert_eq!(renamed.task.units(), 0);

        if let ItemRef::Group(group) = item {
            let to = app
                .model()
                .unwrap()
                .sections
                .iter()
                .find(|section| section.name == "Beta")
                .unwrap()
                .id;
            let moved = app.process_core(Op::MoveGroup { group, to });
            assert_eq!(moved.processed.disposition, Disposition::Completed);
            assert_eq!(app.session.as_ref().unwrap().index().revision(), revision);
            let palette = app.palette.as_ref().unwrap();
            assert_eq!(palette.results[0].key, selected);
            assert!(!palette.results[0].enabled);
            assert_eq!(moved.task.units(), 0);
        }
        std::fs::remove_dir(
            directory
                .path()
                .join(if group { "Beta/Renamed" } else { "Renamed" }),
        )
        .unwrap();
        let removed = app.process_core(Op::Reconcile);
        assert_eq!(removed.processed.disposition, Disposition::Completed);
        assert_eq!(app.session.as_ref().unwrap().index().revision(), revision);
        let palette = app.palette.as_ref().unwrap();
        assert_eq!(palette.query, "  Rename  ");
        assert_eq!(palette.active, Some(0));
        assert_eq!(palette.results[0].key, selected);
        assert_eq!(palette.results[0].path, "Removed item");
        assert!(!palette.results[0].enabled);
        assert_eq!(removed.task.units(), 0);
    }
}

#[test]
fn a_palette_row_click_uses_its_stable_key_not_its_old_index_after_refresh() {
    let (_directory, mut app) = fixture();
    let first = app.selected_note.unwrap();
    let second = note(&app, "Second");
    let _ = app.open_palette(Mode::Jump);
    let generation = app.workspace_generation();
    assert!(
        matches!(app.palette.as_ref().unwrap().results[0].key, super::palette::Key::Note(note) if note == first)
    );
    let clicked = app.palette.as_ref().unwrap().results[0].key.clone();
    let _ = app.process_core(Op::OpenNote { note: Some(second) });
    assert!(
        matches!(app.palette.as_ref().unwrap().results[0].key, super::palette::Key::Note(note) if note == second)
    );
    let _ = app.update(Message::PaletteChoose(generation, clicked));
    assert_eq!(app.selected_note, Some(first));
    assert!(app.palette.is_none());
}

#[test]
fn active_draft_search_does_not_flush_and_title_hits_do_not_override_find() {
    let (directory, mut app) = fixture();
    append(&mut app, " private draft needle");
    let _ = app.open_palette(Mode::Search);
    let _ = app.palette_query("private draft needle".into());
    let palette = app.palette.as_ref().unwrap();
    assert_eq!(palette.results.len(), 1);
    assert!(palette.results[0].content_hit);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("Alpha/Inbox/First.md")).unwrap(),
        "first body"
    );
    let _ = app.close_palette();
    let _ = app.update(Message::FindChanged("body".into()));
    let _ = app.open_palette(Mode::Jump);
    let _ = app.palette_query("Second".into());
    assert!(!app.palette.as_ref().unwrap().results[0].content_hit);
    let _ = app.activate_palette(None);
    assert_eq!(app.find_query, "body");
    assert_eq!(app.selected_note, Some(note(&app, "Second")));
}

#[test]
fn stale_search_range_is_recomputed_against_current_text_without_replacing_find_query() {
    let (directory, mut app) = fixture();
    let first = app.selected_note.unwrap();
    let stale = target(&app, first, "body");
    std::fs::write(directory.path().join("Alpha/Inbox/First.md"), "body first").unwrap();
    let _ = app.process_core(Op::Reconcile);
    let _ = app.update(Message::FindChanged("first".into()));
    let _ = app.dispatch(Command::OpenNote(stale));
    assert_eq!(app.editor.selected_text().as_deref(), Some("body"));
    assert_eq!(app.editor.cursor().selection.unwrap().column, 0);
    assert_eq!(app.find_query, "first");
}

#[test]
fn failed_and_conflict_cancelled_search_activation_preserve_recoverable_overlay() {
    for external_conflict in [false, true] {
        let (directory, mut app) = fixture();
        let first = app.selected_note;
        append(&mut app, " local draft");
        let _ = app.open_palette(Mode::Jump);
        let _ = app.palette_query("Second".into());
        if external_conflict {
            std::fs::write(
                directory.path().join("Alpha/Inbox/First.md"),
                "external bytes",
            )
            .unwrap();
        } else {
            app.session.as_mut().unwrap().fail_next(Fault::Write);
        }
        let result = app.activate_palette(None);
        assert_eq!(result.processed.disposition, Disposition::Blocked);
        assert_eq!(app.selected_note, first);
        assert_eq!(app.palette.as_ref().unwrap().query, "Second");
        assert!(app.palette.as_ref().unwrap().error.is_some());
        if external_conflict {
            assert_eq!(app.editor.text(), "external bytes");
        } else {
            assert!(app.editor.text().ends_with("local draft"));
        }
    }
}

#[test]
fn unrelated_successful_flush_cannot_turn_an_invalid_selection_into_success() {
    let (_directory, mut app) = fixture();
    append(&mut app, " saved by barrier");
    let first = app.selected_note;
    let result = app.process_core(Op::OpenNote {
        note: Some(NoteId(u64::MAX)),
    });
    assert_eq!(result.processed.disposition, Disposition::Blocked);
    assert_eq!(app.selected_note, first);
    assert!(!app.session.as_ref().unwrap().document().unwrap().dirty());
    assert!(result.processed.warning.is_some());
}

#[test]
fn committed_move_warning_dismisses_mutating_picker_and_cannot_replay() {
    let (directory, mut app) = fixture();
    let first = app.selected_note.unwrap();
    let _ = app.open_palette(Mode::MoveNote(first));
    let _ = app.palette_query("Beta Destination".into());
    assert_eq!(app.palette.as_ref().unwrap().results.len(), 1);
    app.session.as_mut().unwrap().fail_next(Fault::FileSync);
    let result = app.activate_palette(None);
    assert_eq!(result.processed.disposition, Disposition::CommittedWarning);
    assert!(app.palette.is_none());
    assert!(app.banner.is_some());
    assert!(directory.path().join("Beta/Destination/First.md").exists());
    assert!(!directory.path().join("Alpha/Inbox/First.md").exists());
    let replay = app.activate_palette(None);
    assert_eq!(replay.processed.disposition, Disposition::Blocked);
    assert!(!directory
        .path()
        .join("Beta/Destination/First 2.md")
        .exists());
}

#[test]
fn committed_selection_metadata_warning_projects_actual_note_and_dismisses_jump() {
    let (_directory, mut app) = fixture();
    let _ = app.open_palette(Mode::Jump);
    let _ = app.palette_query("Second".into());
    app.session.as_mut().unwrap().fail_next(Fault::FileSync);
    let result = app.activate_palette(None);
    assert_eq!(result.processed.disposition, Disposition::CommittedWarning);
    assert_eq!(app.selected_note, Some(note(&app, "Second")));
    assert_eq!(app.editor.text(), "second body");
    assert!(app.palette.is_none());
    let warning = app.banner.clone();
    let _ = app.process_core(Op::Flush);
    assert_eq!(app.banner, warning);
}

#[test]
fn external_deletion_recovery_cancels_rename_and_delete_without_dismissing_the_dialog() {
    for delete in [false, true] {
        let (directory, mut app) = fixture();
        let first = app.selected_note.unwrap();
        append(&mut app, " recovered draft");
        let _ = app.dispatch(if delete {
            Command::Delete(ItemRef::Note(first))
        } else {
            Command::Rename(ItemRef::Note(first))
        });
        app.dialog.as_mut().unwrap().name = "Must not publish".into();
        std::fs::remove_file(directory.path().join("Alpha/Inbox/First.md")).unwrap();
        let _ = app.submit_dialog(delete);
        let processed = app.last_processed.as_ref().unwrap();
        assert_eq!(processed.disposition, Disposition::Blocked);
        assert!(processed.warning.as_deref().unwrap().contains("cancelled"));
        let dialog = app
            .dialog
            .as_ref()
            .expect("cancelled mutation remains retryable");
        assert!(dialog.error.as_deref().unwrap().contains("cancelled"));
        assert_eq!(dialog.name, "Must not publish");
        assert!(app.model().unwrap().note(first).is_none());
        assert_ne!(app.selected_note, Some(first));
        assert_eq!(app.editor.text(), "first body recovered draft");
        assert!(!directory
            .path()
            .join("Alpha/Inbox/Must not publish.md")
            .exists());
        assert_eq!(app.model().unwrap().notes.len(), 3);
        let recovery = app
            .model()
            .unwrap()
            .note(app.selected_note.unwrap())
            .unwrap();
        assert!(recovery.title.contains("(conflict "));
        assert_eq!(
            std::fs::read_to_string(directory.path().join(recovery.path.as_path())).unwrap(),
            app.editor.text()
        );
    }
}

#[test]
fn committed_rename_and_delete_partial_failures_still_dismiss_with_a_warning() {
    struct RemoveFile;
    impl wren_core::ops::Trash for RemoveFile {
        fn trash(&mut self, path: &std::path::Path) -> Result<(), String> {
            std::fs::remove_file(path).map_err(|error| error.to_string())
        }
    }

    for delete in [false, true] {
        let (directory, mut app) = fixture();
        drop(app.session.take());
        app.session = Some(Session::with_trash(directory.path(), Box::new(RemoveFile)).unwrap());
        app.sync_editor(true);
        let first = app.selected_note.unwrap();
        let _ = app.dispatch(if delete {
            Command::Delete(ItemRef::Note(first))
        } else {
            Command::Rename(ItemRef::Note(first))
        });
        app.dialog.as_mut().unwrap().name = "Published".into();
        app.session.as_mut().unwrap().fail_next(if delete {
            Fault::FileSync
        } else {
            Fault::DirectorySync
        });
        let _ = app.submit_dialog(delete);
        assert_eq!(
            app.last_processed.as_ref().unwrap().disposition,
            Disposition::CommittedWarning
        );
        assert!(app.dialog.is_none());
        assert!(app.banner.is_some());
        assert!(!directory.path().join("Alpha/Inbox/First.md").exists());
        if delete {
            assert!(app.model().unwrap().note(first).is_none());
        } else {
            assert!(directory.path().join("Alpha/Inbox/Published.md").exists());
            assert_eq!(app.selected_note, Some(first));
        }
    }
}

#[test]
fn read_only_workspace_metadata_warns_for_committed_selection_and_preferences_but_not_document_flushes(
) {
    let (directory, mut app) = fixture();
    drop(app.session.take());
    let metadata = directory.path().join(".wren/workspace.toml");
    std::fs::write(&metadata, "invalid = [").unwrap();
    app.session = Some(Session::open(directory.path()).unwrap());
    app.sync_editor(true);
    let reason = app
        .session
        .as_ref()
        .unwrap()
        .state()
        .warning()
        .unwrap()
        .to_owned();
    assert!(!app.session.as_ref().unwrap().state().writable());
    let _ = app.open_palette(Mode::Jump);
    let _ = app.palette_query("Second".into());
    let result = app.activate_palette(None);
    assert_eq!(result.processed.disposition, Disposition::CommittedWarning);
    assert_eq!(result.processed.warning.as_ref(), Some(&reason));
    assert_eq!(app.selected_note, Some(note(&app, "Second")));
    assert!(app.palette.is_none());
    assert_eq!(app.banner.as_ref(), Some(&reason));
    assert_eq!(app.metadata_warning.as_ref(), Some(&reason));
    assert_eq!(app.banner_kind, super::workspace::WarningKind::Metadata);
    assert!(app
        .session
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .error
        .is_none());

    append(&mut app, " saved despite invalid metadata");
    let flushed = app.process_core(Op::Flush);
    assert_eq!(flushed.processed.disposition, Disposition::Completed);
    assert!(flushed.processed.warning.is_none());
    assert_eq!(app.banner.as_ref(), Some(&reason));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("Alpha/Group/Second.md")).unwrap(),
        app.editor.text()
    );
    for command in [
        Command::Width {
            pane: Pane::Groups,
            width: 340.0,
        },
        Command::Sort("title".into()),
        Command::Run(CommandId::ResetWidths),
    ] {
        let result = app.dispatch(command);
        assert_eq!(result.processed.disposition, Disposition::CommittedWarning);
        assert_eq!(result.processed.warning.as_ref(), Some(&reason));
        assert_eq!(app.metadata_warning.as_ref(), Some(&reason));
    }
    append(&mut app, " retryable document error");
    app.session.as_mut().unwrap().fail_next(Fault::Write);
    assert_eq!(
        app.process_core(Op::Flush).processed.disposition,
        Disposition::Blocked
    );
    assert_eq!(app.banner_kind, super::workspace::WarningKind::Document);
    let result = app.dispatch(Command::Sort("modified".into()));
    assert_eq!(result.processed.disposition, Disposition::CommittedWarning);
    assert_eq!(result.processed.warning.as_ref(), Some(&reason));
    assert_eq!(app.banner_kind, super::workspace::WarningKind::Metadata);
    assert_eq!(app.metadata_warning.as_ref(), Some(&reason));
    assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
    assert!(app
        .session
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .error
        .is_some());
    let flushed = app.process_core(Op::Flush);
    assert_eq!(flushed.processed.disposition, Disposition::Completed);
    assert_eq!(app.banner.as_ref(), Some(&reason));
    assert_eq!(std::fs::read_to_string(&metadata).unwrap(), "invalid = [");
    assert!(app
        .session
        .as_ref()
        .unwrap()
        .document()
        .unwrap()
        .error
        .is_none());
}

#[test]
fn committed_rename_dialog_dismisses_but_failed_rename_remains_retryable() {
    let (directory, mut app) = fixture();
    let first = app.selected_note.unwrap();
    let _ = app.dispatch(Command::Rename(ItemRef::Note(first)));
    app.dialog.as_mut().unwrap().name = "Renamed".into();
    app.session.as_mut().unwrap().fail_next(Fault::FileSync);
    let _ = app.submit_dialog(false);
    assert!(app.dialog.is_none());
    assert!(directory.path().join("Alpha/Inbox/Renamed.md").exists());
    append(&mut app, " pending");
    let _ = app.dispatch(Command::Rename(ItemRef::Note(first)));
    app.dialog.as_mut().unwrap().name = "Unpublished".into();
    app.session.as_mut().unwrap().fail_next(Fault::Write);
    let _ = app.submit_dialog(false);
    assert!(app.dialog.is_some());
    assert!(!directory.path().join("Alpha/Inbox/Unpublished.md").exists());
}

#[test]
fn document_flush_does_not_clear_registry_warning_or_metadata_warning() {
    let (_directory, mut app) = fixture();
    app.registry_warning = Some("registry unavailable".into());
    app.banner = Some("workspace metadata unavailable".into());
    app.banner_kind = super::workspace::WarningKind::Metadata;
    append(&mut app, " saved");
    let _ = app.process_core(Op::Flush);
    assert_eq!(
        app.registry_warning.as_deref(),
        Some("registry unavailable")
    );
    assert_eq!(
        app.banner.as_deref(),
        Some("workspace metadata unavailable")
    );
}

#[test]
fn preference_ops_use_the_session_and_a_narrow_window_does_not_change_preferences() {
    let (directory, mut app) = fixture();
    let _ = app.dispatch(Command::Width {
        pane: Pane::Groups,
        width: 350.0,
    });
    let _ = app.dispatch(Command::Sort("title".into()));
    let _ = app.widths.effective(710.0);
    assert_eq!(app.widths.get(Pane::Groups), 350.0);
    drop(app);
    let session = Session::open(directory.path()).unwrap();
    assert_eq!(session.state().width("groups"), Some(350));
    let parent = session.selected_parent().unwrap();
    assert_eq!(
        session
            .state()
            .sort(session.model().parent_path(parent).unwrap()),
        "title"
    );
}

#[test]
fn picker_cancellation_and_stale_completion_preserve_state_and_never_open() {
    let (_directory, mut app) = fixture();
    append(&mut app, " draft");
    let selected = app.selected_note;
    let tag = app.begin_folder_request();
    let stale = app.folder_chosen(tag.wrapping_sub(1), Some("/missing".into()));
    assert_eq!(stale.processed.disposition, Disposition::Blocked);
    assert_eq!(app.folder_request, Some(tag));
    let cancelled = app.folder_chosen(tag, None);
    assert_eq!(cancelled.processed.disposition, Disposition::Completed);
    assert_eq!(app.selected_note, selected);
    assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
    assert!(app.loading.is_none());
}

#[test]
fn chosen_folder_cannot_bypass_a_failed_save_and_keeps_the_workspace_picker_recoverable() {
    let (directory, mut app) = fixture();
    let next = crate::test_support::tempdir();
    append(&mut app, " retained");
    let _ = app.open_palette(Mode::Workspaces);
    let _ = app.palette_query("original query".into());
    let tag = app.begin_folder_request();
    app.session.as_mut().unwrap().fail_next(Fault::Write);
    let outcome = app.folder_chosen(tag, Some(next.path().into()));
    assert_eq!(outcome.processed.disposition, Disposition::Blocked);
    assert!(app.loading.is_none());
    assert_eq!(app.model().unwrap().root, directory.path());
    assert_eq!(app.palette.as_ref().unwrap().query, "original query");
    assert!(app.palette.as_ref().unwrap().error.is_some());
    assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
}

#[test]
fn remove_active_registration_does_not_close_session_flush_draft_or_delete_files() {
    let (directory, mut app) = fixture();
    let configdir = crate::test_support::tempdir();
    let mut config = Config::load_from(configdir.path());
    config.add(directory.path(), Some("Notes")).unwrap();
    config.set_active(directory.path()).unwrap();
    app.config = Some(config);
    append(&mut app, " unsaved");
    let generation = app.workspace_generation();
    let result = app.dispatch(Command::RemoveWorkspace(directory.path().into()));
    assert_eq!(result.processed.disposition, Disposition::Completed);
    assert_eq!(app.workspace_generation(), generation);
    assert!(app.config.as_ref().unwrap().active().is_none());
    assert!(app.config.as_ref().unwrap().workspaces().is_empty());
    assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("Alpha/Inbox/First.md")).unwrap(),
        "first body"
    );
}

#[test]
fn remove_command_uses_a_keyboard_picker_that_can_remove_an_unavailable_registration() {
    let (directory, mut app) = fixture();
    let configdir = crate::test_support::tempdir();
    let unavailable = crate::test_support::tempdir();
    let missing_path = unavailable.path().to_path_buf();
    let mut config = Config::load_from(configdir.path());
    config.add(directory.path(), Some("Active")).unwrap();
    config.add(&missing_path, Some("Unavailable")).unwrap();
    config.set_active(directory.path()).unwrap();
    drop(unavailable);
    app.config = Some(Config::load_from(configdir.path()));
    append(&mut app, " unsaved");
    let selected = app.selected_note;
    let _ = app.dispatch(Command::Run(CommandId::RemoveWorkspace));
    assert!(app.palette.as_ref().unwrap().workspace_removal);
    let _ = app.palette_query("Unavailable".into());
    assert_eq!(app.palette.as_ref().unwrap().results.len(), 1);
    let result = app.activate_palette(None);
    assert_eq!(result.processed.disposition, Disposition::Completed);
    assert!(app.palette.is_none());
    assert_eq!(app.config.as_ref().unwrap().workspaces().len(), 1);
    assert_eq!(
        app.config.as_ref().unwrap().active(),
        Some(directory.path())
    );
    assert_eq!(app.selected_note, selected);
    assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
    assert!(!missing_path.exists());
}

#[test]
fn normal_startup_is_onboarding_or_registered_root_and_evaluation_is_explicit() {
    let configdir = crate::test_support::tempdir();
    let config = Config::load_from(configdir.path());
    let (app, _) = App::with_config(Options::default(), Some(config), None);
    assert!(app.workspace_mode);
    assert!(app.loading.is_none());
    assert!(app.editor.text().is_empty());
    let root = crate::test_support::tempdir();
    let mut config = Config::load_from(configdir.path());
    config.add(root.path(), None).unwrap();
    config.set_active(root.path()).unwrap();
    let (app, _) = App::with_config(Options::default(), Some(config), None);
    assert!(app.loading.is_some());
    assert_eq!(app.workspace_path, root.path().to_string_lossy());
    assert!(app.editor.text().is_empty());
    let (evaluation, _) = App::new(Options {
        editor: true,
        ..Options::default()
    });
    assert!(!evaluation.workspace_mode);
    assert!(evaluation.config.is_none());
    assert!(evaluation.editor.text().contains("not a saved note"));
}

#[test]
fn passive_autosave_does_not_replace_the_correlated_requested_action_result() {
    let (_directory, mut app) = fixture();
    let requested = app.dispatch(Command::Sort("title".into())).processed;
    append(&mut app, " edit");
    let _ = app.process_core(Op::Tick);
    let processed = app.last_processed.as_ref().unwrap();
    assert_eq!(processed.correlation, requested.correlation);
    assert_eq!(processed.disposition, requested.disposition);
}

#[test]
fn a_document_error_and_successful_retry_do_not_discard_an_earlier_metadata_warning() {
    let (_directory, mut app) = fixture();
    let _ = app.open_palette(Mode::Jump);
    let _ = app.palette_query("Second".into());
    app.session.as_mut().unwrap().fail_next(Fault::FileSync);
    let _ = app.activate_palette(None);
    let metadata = app
        .metadata_warning
        .clone()
        .expect("committed selection metadata warning");
    append(&mut app, " retained");
    app.session.as_mut().unwrap().fail_next(Fault::Write);
    assert_eq!(
        app.process_core(Op::Flush).processed.disposition,
        Disposition::Blocked
    );
    assert_ne!(app.banner.as_deref(), Some(metadata.as_str()));
    let _ = app.process_core(Op::Flush);
    assert_eq!(app.metadata_warning.as_ref(), Some(&metadata));
    assert_eq!(app.banner.as_ref(), Some(&metadata));
}

#[test]
fn spring_browsing_is_transient_and_a_failed_blur_clears_preview_before_the_barrier() {
    let (_directory, mut app) = fixture();
    append(&mut app, " retained");
    let generation = app.gesture_generation;
    let selected = app.selected_note;
    let parent = app.session.as_ref().unwrap().selected_parent();
    let beta = app
        .model()
        .unwrap()
        .sections
        .iter()
        .find(|section| section.name == "Beta")
        .unwrap()
        .id;
    let _ = app.drag_event(
        generation,
        crate::drag::DragEvent::Spring {
            generation,
            section: beta,
        },
    );
    app.drag_preview = Some(crate::drag::Preview {
        generation,
        source: crate::drag::RowKind::Note(selected.unwrap()),
        position: iced::Point::new(100.0, 120.0),
        target: None,
        spring: None,
    });
    assert_eq!(app.spring_section, Some(beta));
    assert_eq!(app.session.as_ref().unwrap().selected_parent(), parent);
    assert_eq!(app.selected_note, selected);
    assert!(app.session.as_ref().unwrap().document().unwrap().dirty());
    app.session.as_mut().unwrap().fail_next(Fault::Write);
    assert!(app.window_unfocused().is_err());
    assert!(app.spring_section.is_none());
    assert!(app.drag_preview.is_none());
    assert_ne!(app.gesture_generation, generation);
    let _ = app.drag_event(
        generation,
        crate::drag::DragEvent::Commit {
            generation,
            op: Op::MoveNote {
                note: selected.unwrap(),
                to: group(&app, "Destination"),
            },
        },
    );
    assert_eq!(app.session.as_ref().unwrap().selected_parent(), parent);
    assert_eq!(app.selected_note, selected);
}

#[test]
fn resize_updates_live_preference_but_persists_only_at_end() {
    let (_directory, mut app) = fixture();
    let generation = app.gesture_generation;
    let _ = app.resize_event(
        generation,
        crate::panes::ResizeEvent::Started { pane: Pane::Groups },
    );
    let _ = app.resize_event(
        generation,
        crate::panes::ResizeEvent::Changed {
            pane: Pane::Groups,
            width: 320.0,
        },
    );
    assert_eq!(app.widths.get(Pane::Groups), 320.0);
    assert_eq!(
        app.session.as_ref().unwrap().state().width("groups"),
        Some(220)
    );
    let _ = app.resize_event(
        generation,
        crate::panes::ResizeEvent::Ended {
            pane: Pane::Groups,
            width: 320.0,
        },
    );
    assert_eq!(
        app.session.as_ref().unwrap().state().width("groups"),
        Some(320)
    );
    assert!(app.resizing.is_none());
}

#[test]
fn stale_generation_with_a_valid_identical_note_id_cannot_change_selection_or_cursor() {
    let (_directory, mut app) = fixture();
    let mut stale = target(&app, note(&app, "Second"), "body");
    stale.generation = stale.generation.wrapping_sub(1);
    let selected = app.selected_note;
    let cursor = app.editor.cursor();
    let result = app.dispatch(Command::OpenNote(stale));
    assert_eq!(result.processed.disposition, Disposition::Blocked);
    assert_eq!(app.selected_note, selected);
    assert_eq!(app.editor.cursor(), cursor);
}

#[test]
fn destructive_command_needs_explicit_target_and_return_is_never_confirmation() {
    let (directory, mut app) = fixture();
    let first = app.selected_note.unwrap();
    let _ = app.dispatch(Command::Delete(ItemRef::Note(first)));
    assert!(
        matches!(app.dialog.as_ref().unwrap().kind, DialogKind::Delete(ItemRef::Note(note)) if note == first)
    );
    let _ = app.update(Message::Key(crate::keys::Input::Nav(
        crate::keys::Nav::Enter,
    )));
    let _ = app.update(Message::SubmitDialog);
    assert!(app.dialog.is_some());
    assert!(directory.path().join("Alpha/Inbox/First.md").exists());
}

mod columns {
    use super::*;
    use crate::app::columns::Column;
    use crate::drag::{DragEvent, RowKind};
    use crate::keys::{Input, Nav};

    fn key(app: &mut App, nav: Nav) {
        let _ = app.update(Message::Key(Input::Nav(nav)));
    }

    fn run(app: &mut App, id: CommandId) -> Disposition {
        app.dispatch(Command::Run(id)).processed.disposition
    }

    fn section(app: &App, name: &str) -> wren_core::model::SectionId {
        app.model()
            .unwrap()
            .sections
            .iter()
            .find(|section| section.name == name)
            .unwrap()
            .id
    }

    #[test]
    fn keyboard_walks_columns_and_selection_is_live_with_flushes() {
        let (directory, mut app) = fixture();
        assert_eq!(app.column, Column::Editor);
        append(&mut app, " edited");
        for expected in [Column::Notes, Column::Groups, Column::Sections] {
            assert_eq!(run(&mut app, CommandId::ColumnLeft), Disposition::Completed);
            assert_eq!(app.column, expected);
        }
        assert_eq!(run(&mut app, CommandId::ColumnLeft), Disposition::Blocked);
        assert_eq!(app.column, Column::Sections);

        // ↓ in Sections selects Beta; its group and last note follow, and the
        // outgoing draft is flushed first.
        key(&mut app, Nav::Down);
        assert_eq!(app.selected_section, Some(section(&app, "Beta")));
        assert_eq!(app.selected_note, Some(note(&app, "Third")));
        assert_eq!(app.editor.text(), "third body");
        assert_eq!(app.column, Column::Sections);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("Alpha/Inbox/First.md")).unwrap(),
            "first body edited"
        );
        key(&mut app, Nav::Down);
        assert_eq!(app.selected_section, Some(section(&app, "Beta")), "clamped");
        // A section opens on its first group.
        key(&mut app, Nav::Up);
        assert_eq!(app.selected_group, Some(group(&app, "Group")));
        assert_eq!(app.selected_note, Some(note(&app, "Second")));

        key(&mut app, Nav::Enter);
        assert_eq!(app.column, Column::Groups);
        key(&mut app, Nav::Down);
        assert_eq!(app.selected_group, Some(group(&app, "Inbox")));
        assert_eq!(app.selected_note, Some(note(&app, "First")));
        assert_eq!(app.editor.text(), "first body edited");
        key(&mut app, Nav::Up);
        assert_eq!(app.selected_group, Some(group(&app, "Group")));
        assert_eq!(app.column, Column::Groups);

        key(&mut app, Nav::Right);
        assert_eq!(app.column, Column::Notes);
        key(&mut app, Nav::Left);
        assert_eq!(app.column, Column::Groups);
        assert_eq!(
            run(&mut app, CommandId::ColumnRight),
            Disposition::Completed
        );
        assert_eq!(
            run(&mut app, CommandId::ColumnRight),
            Disposition::Completed
        );
        assert_eq!(app.column, Column::Editor);
        assert_eq!(app.selected_note, Some(note(&app, "Second")));
        assert_eq!(run(&mut app, CommandId::ColumnRight), Disposition::Blocked);
    }

    #[test]
    fn arrows_belong_to_the_editor_and_escape_returns_to_it() {
        let (_directory, mut app) = fixture();
        let selected = app.selected_note;
        for nav in [Nav::Down, Nav::Up, Nav::Left, Nav::Right, Nav::Enter] {
            key(&mut app, nav);
        }
        assert_eq!(app.column, Column::Editor);
        assert_eq!(app.selected_note, selected);

        let _ = run(&mut app, CommandId::ColumnLeft);
        let _ = run(&mut app, CommandId::ColumnLeft);
        assert_eq!(app.column, Column::Groups);
        let group = app.selected_group;
        key(&mut app, Nav::Escape);
        assert_eq!(app.column, Column::Editor);
        assert_eq!(app.selected_group, group);
        assert_eq!(app.selected_note, selected);
    }

    #[test]
    fn escape_closing_an_overlay_keeps_the_active_column() {
        let (_directory, mut app) = fixture();
        let _ = run(&mut app, CommandId::ColumnLeft);
        let _ = app.open_palette(Mode::Commands);
        key(&mut app, Nav::Down);
        assert_eq!(
            app.column,
            Column::Notes,
            "arrows move the palette, not the column"
        );
        key(&mut app, Nav::Escape);
        assert!(app.palette.is_none());
        assert_eq!(app.column, Column::Notes);
    }

    #[test]
    fn a_click_selects_like_an_arrow_and_a_double_click_moves_right() {
        let (_directory, mut app) = fixture();
        let generation = app.gesture_generation;
        let destination = group(&app, "Destination");
        let _ = app.drag_event(
            generation,
            DragEvent::Activate {
                generation,
                row: RowKind::Section(section(&app, "Beta")),
            },
        );
        assert_eq!(app.column, Column::Sections);
        assert_eq!(app.selected_group, Some(destination));
        let second = note(&app, "Second");
        let _ = app.dispatch(Command::SelectParent(Parent::Group(group(&app, "Group"))));
        let _ = app.drag_event(
            generation,
            DragEvent::Activate {
                generation,
                row: RowKind::Note(second),
            },
        );
        assert_eq!(
            app.column,
            Column::Notes,
            "a note click keeps the keyboard in Notes"
        );
        assert_eq!(app.selected_note, Some(second));
        let _ = app.drag_event(
            generation,
            DragEvent::Open {
                generation,
                row: RowKind::Group(destination),
            },
        );
        assert_eq!(app.selected_group, Some(destination));
        assert_eq!(app.column, Column::Notes);
        assert!(app.dialog.is_none());
    }

    #[test]
    fn moving_right_needs_a_selection_and_an_empty_group_stays_put() {
        let (directory, mut app) = fixture();
        std::fs::create_dir(directory.path().join("Alpha/Empty")).unwrap();
        let _ = app.process_core(Op::Reconcile);
        let empty = group(&app, "Empty");
        let generation = app.gesture_generation;
        let _ = app.drag_event(
            generation,
            DragEvent::Activate {
                generation,
                row: RowKind::Group(empty),
            },
        );
        assert_eq!(app.column, Column::Groups);
        assert_eq!(app.selected_note, None);
        assert_eq!(
            run(&mut app, CommandId::ColumnRight),
            Disposition::Completed
        );
        assert_eq!(app.column, Column::Notes);
        let blocked = app.dispatch(Command::Run(CommandId::ColumnRight));
        assert_eq!(blocked.processed.disposition, Disposition::Blocked);
        assert_eq!(app.column, Column::Notes);
        key(&mut app, Nav::Down);
        assert_eq!(app.selected_note, None);
    }

    #[test]
    fn a_palette_jump_and_history_land_in_the_editor() {
        let (_directory, mut app) = fixture();
        let _ = run(&mut app, CommandId::ColumnLeft);
        let _ = app.open_palette(Mode::Jump);
        let _ = app.palette_query("Third".into());
        let _ = app.activate_palette(None);
        assert_eq!(app.selected_note, Some(note(&app, "Third")));
        assert_eq!(app.column, Column::Editor);
        let _ = run(&mut app, CommandId::ColumnLeft);
        assert_eq!(run(&mut app, CommandId::Back), Disposition::Completed);
        assert_eq!(app.column, Column::Editor);
        assert_eq!(app.selected_note, Some(note(&app, "First")));
    }

    #[test]
    fn a_filter_hides_rows_from_the_arrows() {
        let (_directory, mut app) = fixture();
        let _ = app.dispatch(Command::Filter {
            pane: "sections".into(),
            query: "Beta".into(),
        });
        let _ = run(&mut app, CommandId::ColumnLeft);
        let _ = run(&mut app, CommandId::ColumnLeft);
        let _ = run(&mut app, CommandId::ColumnLeft);
        // Alpha is selected but filtered out; ↓ lands on the first visible row.
        key(&mut app, Nav::Down);
        assert_eq!(app.selected_section, Some(section(&app, "Beta")));
    }

    #[test]
    fn rename_and_column_moves_have_default_shortcuts() {
        let (_directory, mut app) = fixture();
        let _ = app.update(super::super::preferences::shortcut(CommandId::ColumnLeft));
        assert_eq!(app.column, Column::Notes);
        let _ = app.update(super::super::preferences::shortcut(CommandId::ColumnRight));
        assert_eq!(app.column, Column::Editor);
        let _ = app.update(super::super::preferences::shortcut(CommandId::RenameNote));
        assert!(matches!(
            app.dialog.as_ref().unwrap().kind,
            DialogKind::Rename(ItemRef::Note(_))
        ));
    }
}
