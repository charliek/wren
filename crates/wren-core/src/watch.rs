//! Bounded rescan hints only; the session owns disk reads and reconciliation.

use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;

use notify::event::{AccessKind, AccessMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hint {
    pub generation: u64,
    pub warning: Option<String>,
}

pub struct Watch {
    _watcher: RecommendedWatcher,
    hints: Hints,
}

impl Watch {
    /// Start before the initial scan so concurrent edits leave a pending hint.
    pub fn new(root: &Path, generation: u64) -> Result<Self, String> {
        let root = std::path::absolute(root)
            .map_err(|error| format!("cannot resolve watcher root: {error}"))?;
        let callback_root = root.clone();
        let (signal, hints) = Hints::channel(generation);
        let mut watcher = RecommendedWatcher::new(
            move |event| signal.event(&callback_root, event),
            Config::default().with_follow_symlinks(false),
        )
        .map_err(|error| format!("cannot create filesystem watcher: {error}"))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| format!("cannot watch workspace {}: {error}", root.display()))?;
        Ok(Self {
            _watcher: watcher,
            hints,
        })
    }

    pub fn take_hint(&mut self) -> Option<Hint> {
        self.hints.take_hint()
    }
}

struct Signal {
    sender: SyncSender<()>,
    warning: Arc<AtomicBool>,
}

impl Signal {
    fn event(&self, root: &Path, event: notify::Result<Event>) {
        let warning = match event {
            Ok(event) if needs_scan(root, &event) => event.need_rescan(),
            Ok(_) => return,
            Err(_) => true,
        };
        if warning {
            // Preserve errors/overflow even when a normal hint already fills the queue.
            self.warning.store(true, Ordering::Release);
        }
        // Every queued unit requests a full rescan, so duplicate/overflow hints coalesce.
        let _ = self.sender.try_send(());
    }
}

struct Hints {
    generation: u64,
    receiver: Receiver<()>,
    warning: Arc<AtomicBool>,
}

impl Hints {
    fn channel(generation: u64) -> (Signal, Self) {
        let (sender, receiver) = mpsc::sync_channel(1);
        let warning = Arc::new(AtomicBool::new(false));
        (
            Signal {
                sender,
                warning: Arc::clone(&warning),
            },
            Self {
                generation,
                receiver,
                warning,
            },
        )
    }

    fn take_hint(&mut self) -> Option<Hint> {
        // One receive is sufficient and bounds work even during a continuous event flood.
        let pending = self.receiver.try_recv().is_ok();
        let warning = self.warning.swap(false, Ordering::AcqRel);
        (pending || warning).then(|| Hint {
            generation: self.generation,
            warning: warning.then(|| {
                "filesystem watcher reported an error or lost events; rescanning workspace".into()
            }),
        })
    }
}

fn needs_scan(root: &Path, event: &Event) -> bool {
    if event.need_rescan() {
        return true;
    }
    if let EventKind::Access(kind) = event.kind {
        // Inotify reports Open(Any) for reads too. Only a close-after-write is useful.
        if kind != AccessKind::Close(AccessMode::Write) {
            return false;
        }
    }
    event.paths.is_empty()
        || event.paths.iter().any(|path| {
            path.as_os_str().is_empty()
                || path.strip_prefix(root).is_ok_and(|relative| {
                    !relative.components().any(|component| {
                        matches!(component, Component::Normal(name) if name.as_encoded_bytes().starts_with(b"."))
                    })
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, Flag, ModifyKind, RemoveKind, RenameMode};
    use std::time::{Duration, Instant};

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        paths.iter().fold(Event::new(kind), |event, path| {
            event.add_path((*path).into())
        })
    }

    #[test]
    fn filters_dot_components_relative_to_the_workspace_not_its_ancestors() {
        let root = Path::new("/home/.private/workspace");
        for path in [
            "/home/.private/workspace/.wren/state.json",
            "/home/.private/workspace/.wren.lock",
            "/home/.private/workspace/Section/.temporary",
            "/home/.private/workspace/Section/.hidden/note.md",
            "/home/.private/workspace-other/note.md",
            "/outside/note.md",
        ] {
            assert!(!needs_scan(root, &event(EventKind::Any, &[path])), "{path}");
        }
        for path in [
            "/home/.private/workspace/Section/note.md",
            "/home/.private/workspace/Section.v2/note.md",
            "/home/.private/workspace/日記/東京.md",
        ] {
            assert!(needs_scan(root, &event(EventKind::Any, &[path])), "{path}");
        }
    }

    #[test]
    fn readonly_access_does_not_trigger_scan_feedback() {
        let root = Path::new("/workspace");
        for kind in [
            AccessKind::Any,
            AccessKind::Read,
            AccessKind::Open(AccessMode::Read),
            AccessKind::Open(AccessMode::Any),
            AccessKind::Open(AccessMode::Write),
            AccessKind::Close(AccessMode::Read),
            AccessKind::Close(AccessMode::Any),
            AccessKind::Close(AccessMode::Execute),
            AccessKind::Other,
        ] {
            assert!(!needs_scan(
                root,
                &event(EventKind::Access(kind), &["/workspace/Section/note.md"])
            ));
            assert!(!needs_scan(root, &event(EventKind::Access(kind), &[])));
        }
        assert!(needs_scan(
            root,
            &event(
                EventKind::Access(AccessKind::Close(AccessMode::Write)),
                &["/workspace/Section/note.md"]
            )
        ));
    }

    #[test]
    fn every_visible_mutation_and_unknown_event_requests_scan() {
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Create(CreateKind::Folder),
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            EventKind::Modify(ModifyKind::Any),
            EventKind::Remove(RemoveKind::File),
            EventKind::Remove(RemoveKind::Folder),
            EventKind::Any,
            EventKind::Other,
        ] {
            assert!(needs_scan(
                Path::new("/workspace"),
                &event(kind, &["/workspace/Section/note.md"])
            ));
        }
    }

    #[test]
    fn renames_consider_both_endpoints_in_either_direction() {
        let root = Path::new("/workspace");
        let visible = "/workspace/Section/note.md";
        let hidden = "/workspace/Section/.temporary";
        let kind = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
        assert!(needs_scan(root, &event(kind, &[hidden, visible])));
        assert!(needs_scan(root, &event(kind, &[visible, hidden])));
        assert!(!needs_scan(
            root,
            &event(kind, &[hidden, "/workspace/.hidden/note.md"])
        ));
        for mode in [RenameMode::From, RenameMode::To] {
            let kind = EventKind::Modify(ModifyKind::Name(mode));
            assert!(needs_scan(root, &event(kind, &[visible])));
            assert!(!needs_scan(root, &event(kind, &[hidden])));
        }
    }

    #[test]
    fn root_empty_paths_and_overflow_are_not_filtered_out() {
        let root = Path::new("/workspace");
        assert!(needs_scan(root, &event(EventKind::Any, &["/workspace"])));
        assert!(needs_scan(root, &event(EventKind::Other, &[])));
        assert!(needs_scan(root, &event(EventKind::Any, &[""])));
        for paths in [&[][..], &["/workspace/.hidden"][..], &["/outside"][..]] {
            let overflow = event(EventKind::Access(AccessKind::Read), paths).set_flag(Flag::Rescan);
            assert!(needs_scan(root, &overflow));
        }
    }

    #[test]
    fn a_burst_is_bounded_and_coalesces_to_one_generation_tagged_hint() {
        let (signal, mut hints) = Hints::channel(17);
        let root = Path::new("/workspace");
        assert_eq!(hints.take_hint(), None);
        for _ in 0..10_000 {
            signal.event(root, Ok(event(EventKind::Any, &["/workspace/note.md"])));
        }
        assert_eq!(
            hints.take_hint(),
            Some(Hint {
                generation: 17,
                warning: None,
            })
        );
        assert_eq!(hints.take_hint(), None);
    }

    #[test]
    fn errors_and_rescan_warnings_survive_a_full_queue() {
        let root = Path::new("/workspace");
        for overflow in [false, true] {
            let (signal, mut hints) = Hints::channel(23);
            signal.event(root, Ok(event(EventKind::Any, &["/workspace/note.md"])));
            for _ in 0..100 {
                if overflow {
                    signal.event(
                        root,
                        Ok(event(EventKind::Any, &["/workspace/.hidden"]).set_flag(Flag::Rescan)),
                    );
                } else {
                    signal.event(root, Err(notify::Error::generic("backend failure")));
                }
            }
            let hint = hints.take_hint().unwrap();
            assert_eq!(hint.generation, 23);
            assert!(hint.warning.is_some());
            assert_eq!(hints.take_hint(), None);
            signal.event(root, Ok(event(EventKind::Other, &[])));
            assert!(hints.take_hint().unwrap().warning.is_none());
        }
    }

    #[test]
    fn warning_latch_can_supply_a_hint_independently_of_the_queue() {
        let (signal, mut hints) = Hints::channel(24);
        signal.event(
            Path::new("/workspace"),
            Err(notify::Error::generic("failure")),
        );
        hints.receiver.try_recv().unwrap();
        assert!(hints.take_hint().unwrap().warning.is_some());
        assert_eq!(hints.take_hint(), None);
    }

    #[test]
    fn old_workspace_callbacks_cannot_signal_a_new_watch() {
        let (old_signal, mut old_hints) = Hints::channel(1);
        let (_new_signal, mut new_hints) = Hints::channel(2);
        let root = Path::new("/same-workspace");
        old_signal.event(root, Err(notify::Error::generic("old backend")));
        assert_eq!(old_hints.take_hint().unwrap().generation, 1);
        assert_eq!(new_hints.take_hint(), None);
        drop(old_hints);
        old_signal.event(root, Err(notify::Error::generic("late old backend")));
        assert_eq!(new_hints.take_hint(), None);
    }

    #[test]
    fn real_recursive_watcher_reports_a_tempdir_change_before_deadline() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("Section/Group")).unwrap();
        let mut watch = Watch::new(&root, 91).unwrap();
        std::fs::write(root.join("Section/Group/note.md"), b"external edit").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(hint) = watch.take_hint() {
                assert_eq!(hint.generation, 91);
                assert_eq!(hint.warning, None);
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "no filesystem hint before deadline");
            std::thread::park_timeout(remaining.min(Duration::from_millis(5)));
        }
    }

    #[test]
    fn construction_failure_is_returned() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(Watch::new(&temporary.path().join("missing"), 5).is_err());
    }
}
