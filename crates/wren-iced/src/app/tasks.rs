//! Tasks' app side (discovery/tasks.md): the one clock every task feature
//! reads, and the day change that refreshes them. All of it waits on the
//! `tasks` setting the app launched with.

use std::time::Duration;

use iced::Subscription;
use wren_markdown::date::Date;

use super::{App, Message};

/// How often a running Wren looks for a new local date while tasks are on.
const DAY_CHECK: Duration = Duration::from_secs(60);

/// Today as the app last saw it, and the date a test pinned.
#[derive(Debug, Clone)]
pub(super) struct Clock {
    /// The date `App::today` gives in place of the local one, set by
    /// `WREN_TEST_TODAY` at launch or the `tasks.set_today` request.
    #[cfg(any(test, feature = "smoke-test"))]
    pinned: Option<Date>,
    /// The day the last check saw.
    seen: Date,
    /// How many day changes the app has handled.
    changes: u64,
}

impl Clock {
    #[cfg_attr(not(any(test, feature = "smoke-test")), allow(unused_variables))]
    pub(super) fn new(pinned: Option<Date>) -> Self {
        #[cfg(any(test, feature = "smoke-test"))]
        let seen = pinned.unwrap_or_else(wren_core::ops::local_today);
        #[cfg(not(any(test, feature = "smoke-test")))]
        let seen = wren_core::ops::local_today();
        Self {
            #[cfg(any(test, feature = "smoke-test"))]
            pinned,
            seen,
            changes: 0,
        }
    }

    /// The day the last check saw.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn seen(&self) -> Date {
        self.seen
    }

    /// How many day changes the app has handled.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn changes(&self) -> u64 {
        self.changes
    }
}

impl App {
    /// Today's date in local time: the one source every task feature reads.
    pub(super) fn today(&self) -> Date {
        #[cfg(any(test, feature = "smoke-test"))]
        if let Some(pinned) = self.clock.pinned {
            return pinned;
        }
        wren_core::ops::local_today()
    }

    /// Pin today to `date`, then look for the day change as the timer does.
    #[cfg(any(test, feature = "smoke-test"))]
    pub(super) fn pin_today(&mut self, date: Date) {
        self.clock.pinned = Some(date);
        self.check_day();
    }

    /// Compare today with the day last seen, handling a change once. With
    /// tasks off there is nothing to refresh, so nothing is handled.
    pub(super) fn check_day(&mut self) {
        if !self.preferences.tasks {
            return;
        }
        let today = self.today();
        if today != self.clock.seen {
            self.clock.seen = today;
            self.day_changed();
        }
    }

    /// The local date moved on while Wren was open. What shows a date
    /// relative to today refreshes from here.
    fn day_changed(&mut self) {
        self.clock.changes += 1;
    }

    /// The timer that looks for a new day: only while tasks are on.
    pub(super) fn day_subscription(&self) -> Option<Subscription<Message>> {
        self.preferences
            .tasks
            .then(|| iced::time::every(DAY_CHECK).map(|_| Message::DayCheck))
    }
}

#[cfg(test)]
mod tests {
    use wren_core::keybind::{self, Platform};

    use super::super::commands::{definitions, CommandId};
    use super::super::palette::{Key as Row, Mode};
    use super::super::preferences::Preferences;
    use super::super::Options;
    use super::*;

    fn date(text: &str) -> Date {
        Date::parse_iso(text).unwrap()
    }

    /// An app whose `config.conf` holds `settings`.
    fn app(settings: &str) -> (tempfile::TempDir, App) {
        let directory = crate::test_support::tempdir();
        let config = directory.path().join("config.conf");
        std::fs::write(&config, settings).unwrap();
        let options = Options {
            editor: true,
            ..Options::default()
        };
        let (app, _) = App::with_config(options, None, Some(config));
        (directory, app)
    }

    #[test]
    fn today_is_the_local_date_until_a_pin_and_then_the_pin() {
        let (_directory, mut app) = app("tasks = on\n");
        let local = wren_core::ops::local_today();
        let today = app.today();
        assert!(today == local || today == wren_core::ops::local_today());
        app.pin_today(date("2026-11-19"));
        assert_eq!(app.today(), date("2026-11-19"));
        assert_eq!(
            app.today().weekday(),
            wren_markdown::date::Weekday::Thursday
        );
        let pinned = Clock::new(Some(date("2026-11-19")));
        assert_eq!(pinned.seen(), date("2026-11-19"));
        assert_eq!(pinned.changes(), 0);
    }

    /// `WREN_TEST_TODAY`, read at launch into the options, reaches the
    /// clock with no day change.
    #[cfg(feature = "smoke-test")]
    #[test]
    fn the_launch_pin_reaches_today() {
        let options = Options {
            editor: true,
            today: Some(date("2026-11-19")),
            ..Options::default()
        };
        let (app, _) = App::new(options);
        assert_eq!(app.today(), date("2026-11-19"));
        assert_eq!(
            (app.clock.seen(), app.clock.changes()),
            (date("2026-11-19"), 0)
        );
    }

    #[test]
    fn the_day_change_fires_once_per_change() {
        let (_directory, mut app) = app("tasks = on\n");
        app.clock = Clock::new(Some(date("2026-11-19")));
        app.check_day();
        assert_eq!(app.clock.changes(), 0, "the launch day is no change");
        app.pin_today(date("2026-11-20"));
        assert_eq!(
            (app.clock.changes(), app.clock.seen()),
            (1, date("2026-11-20"))
        );
        for _ in 0..3 {
            app.check_day();
            let _ = app.update(Message::DayCheck);
        }
        app.pin_today(date("2026-11-20"));
        assert_eq!(app.clock.changes(), 1, "the same day again is no change");
        app.pin_today(date("2026-11-19"));
        assert_eq!(
            (app.clock.changes(), app.clock.seen()),
            (2, date("2026-11-19"))
        );
    }

    #[test]
    fn with_tasks_off_a_day_check_handles_nothing() {
        let (_directory, mut app) = app("tasks = off\n");
        app.clock = Clock::new(Some(date("2026-11-19")));
        app.pin_today(date("2026-11-20"));
        let _ = app.update(Message::DayCheck);
        assert_eq!(
            (app.clock.changes(), app.clock.seen()),
            (0, date("2026-11-19"))
        );
    }

    /// The dated note and the New note dialog's name read the one clock.
    #[test]
    fn new_dated_notes_take_todays_date_from_the_clock() {
        use wren_core::ops::Session;

        use super::super::commands::Command;
        let directory = crate::test_support::tempdir();
        std::fs::create_dir_all(directory.path().join("Alpha/Inbox")).unwrap();
        std::fs::write(directory.path().join("Alpha/Inbox/First.md"), "first").unwrap();
        let (mut app, _) = App::new(Options {
            editor: true,
            ..Options::default()
        });
        app.workspace_mode = true;
        app.session = Some(Session::open(directory.path()).unwrap());
        app.sync_editor(true);
        app.hydrate_navigation();
        let inbox = app.model().unwrap().groups[0].id;
        let _ = app.dispatch(Command::SelectParent(wren_core::model::Parent::Group(
            inbox,
        )));
        // A date no test machine's clock shows.
        app.pin_today(date("2031-02-03"));
        let _ = app.dispatch(Command::Run(CommandId::NewNote));
        assert_eq!(app.dialog.as_ref().unwrap().name, "2031-02-03");
        let _ = app.update(Message::CancelDialog);
        let _ = app.dispatch(Command::Run(CommandId::NewDatedNote));
        assert!(app
            .model()
            .unwrap()
            .notes
            .iter()
            .any(|note| note.title == "2031-02-03"));
        assert!(directory.path().join("Alpha/Inbox/2031-02-03.md").exists());
    }

    #[test]
    fn the_day_timer_runs_only_with_tasks_on() {
        let (_directory, app) = app("tasks = on\n");
        assert!(app.preferences.tasks);
        assert!(app.day_subscription().is_some());
        for settings in ["", "tasks = off\n"] {
            let (_directory, app) = self::app(settings);
            assert!(!app.preferences.tasks, "{settings:?}");
            assert!(app.day_subscription().is_none(), "{settings:?}");
        }
    }

    #[test]
    fn a_changed_tasks_setting_waits_for_a_restart_and_says_so() {
        let (directory, mut app) = app("");
        let path = directory.path().join("config.conf");
        std::fs::write(&path, "tasks = on\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(
            app.banner.as_deref(),
            Some("Restart Wren to apply `tasks`.")
        );
        assert!(!app.preferences.tasks);
        assert!(app.preferences.settings.tasks);
        std::fs::write(&path, "tasks = on\nboards = off\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert_eq!(
            app.banner.as_deref(),
            Some("Restart Wren to apply `boards` and `tasks`.")
        );
        std::fs::write(&path, "tasks = off\n").unwrap();
        assert_eq!(app.reload_settings(false), None);
        assert!(app.banner.is_none());
    }

    /// The gate's rule for every command, which a command that `tasks()`
    /// names must keep: with the setting off it is neither listed nor run.
    #[test]
    fn with_tasks_off_no_task_command_is_listed_or_runs() {
        let (_directory, mut app) = app("");
        assert!(!app.preferences.tasks);
        let _ = app.open_palette(Mode::Commands);
        let listed: Vec<CommandId> = app
            .palette
            .as_ref()
            .unwrap()
            .results
            .iter()
            .filter_map(|row| match row.key {
                Row::Command(id) => Some(id),
                _ => None,
            })
            .collect();
        let _ = app.close_palette();
        assert!(!listed.is_empty());
        assert!(listed.iter().all(|id| !id.tasks()), "{listed:?}");
        for definition in definitions().filter(|definition| definition.id.tasks()) {
            assert!(!app.command_enabled(definition.id), "{}", definition.key);
        }
    }

    /// With tasks off the keymap is the one Wren had before tasks, on both
    /// platforms, and a `keybind` line naming a task command is refused.
    #[test]
    fn with_tasks_off_the_keymap_is_unchanged_on_both_platforms() {
        for platform in [Platform::Mac, Platform::Linux] {
            let (off, warnings) = super::super::preferences::build_keymap(platform, &[], false);
            assert!(warnings.is_empty());
            let (before, _) = wren_core::keybind::Keymap::build(
                keybind::default_bindings(platform)
                    .into_iter()
                    .chain(keybind::default_board_bindings(platform)),
                &[],
                super::super::preferences::layer_of,
            );
            assert_eq!(off, before, "{platform:?}");
            let (on, _) = super::super::preferences::build_keymap(platform, &[], true);
            let tasks = keybind::default_task_bindings(platform);
            assert_eq!(
                on.bindings().count(),
                off.bindings().count() + tasks.len(),
                "{platform:?}"
            );
            for (trigger, action) in &tasks {
                let id = CommandId::parse(action).expect("a task default names a command");
                assert!(id.tasks(), "{trigger} → {action}");
                let accel = keybind::parse_trigger(trigger).unwrap();
                assert_ne!(off.action(&accel), Some(*action), "{platform:?}");
            }
        }
        // The keys the design gives task commands (discovery/tasks.md §2
        // items 21) stay as they were with tasks off.
        for (platform, triggers) in [
            (
                Platform::Mac,
                ["super+d", "super+shift+enter", "super+shift+t"],
            ),
            (
                Platform::Linux,
                ["ctrl+d", "ctrl+shift+enter", "alt+shift+t"],
            ),
        ] {
            let (off, _) = super::super::preferences::build_keymap(platform, &[], false);
            for trigger in triggers {
                let accel = keybind::parse_trigger(trigger).unwrap();
                let bound = off.action(&accel).or(off.board_action(&accel));
                assert!(
                    bound
                        .and_then(CommandId::parse)
                        .is_none_or(|id| !id.tasks()),
                    "{platform:?} {trigger} is {bound:?}"
                );
            }
        }
        let preferences = Preferences::load(None);
        assert!(!preferences.tasks && preferences.settings.warnings.is_empty());
    }
}
