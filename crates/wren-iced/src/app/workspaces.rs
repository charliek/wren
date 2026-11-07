use std::path::{Path, PathBuf};

use iced::Task;

use super::commands::{Disposition, Gateway};
use super::{focus_editor, App, Message};

impl App {
    pub(super) fn pick_workspace(&mut self) -> Gateway {
        if self.folder_request.is_some() || self.loading.is_some() {
            return self.blocked("A workspace request is already in progress");
        }
        if let Some(palette) = &mut self.palette {
            palette.error = None;
        }
        let tag = self.begin_folder_request();
        let outcome = self.finish_action(
            Disposition::AsyncPending,
            None,
            Task::perform(
                async move {
                    rfd::AsyncFileDialog::new()
                        .set_title("Add a Wren workspace")
                        .pick_folder()
                        .await
                        .map(|folder| folder.path().to_path_buf())
                },
                move |path| Message::FolderChosen(tag, path),
            ),
        );
        self.refresh_palette(false);
        outcome
    }

    pub(super) fn begin_folder_request(&mut self) -> u64 {
        self.folder_serial = self.folder_serial.wrapping_add(1);
        let tag = self.folder_serial;
        self.folder_request = Some(tag);
        self.folder_correlation = Some(self.action_serial.wrapping_add(1));
        tag
    }

    pub(super) fn folder_chosen(&mut self, tag: u64, path: Option<PathBuf>) -> Gateway {
        if self.folder_request != Some(tag) {
            return self.blocked("Ignored a stale folder-picker completion");
        }
        self.folder_request = None;
        let correlation = self.folder_correlation.take();
        let mut outcome = match path {
            None => self.finish_action(Disposition::Completed, None, Task::none()),
            Some(path) => {
                self.dispatch(super::commands::Command::OpenWorkspace { path, name: None })
            }
        };
        if self.loading.is_none() {
            self.refresh_palette(false);
        }
        if outcome.processed.disposition == Disposition::Blocked {
            if let Some(palette) = &mut self.palette {
                palette.error = outcome.processed.warning.clone();
            }
        }
        if let Some(correlation) = correlation {
            outcome.processed.correlation = correlation;
            if let Some(loading) = &mut self.loading {
                loading.correlation = correlation;
            }
            self.last_processed = Some(outcome.processed.clone());
        }
        outcome
    }

    pub(super) fn open_registered_workspace(&mut self, path: PathBuf) -> Gateway {
        let validated = self
            .config
            .as_ref()
            .ok_or_else(|| "Workspace configuration is unavailable".to_owned())
            .and_then(|config| config.validate_registration(&path));
        match validated {
            Ok(path) => self.dispatch(super::commands::Command::OpenWorkspace { path, name: None }),
            Err(error) => self.blocked(error),
        }
    }

    pub(super) fn register_active_workspace(&mut self, name: Option<&str>) -> Option<String> {
        let path = self.session.as_ref()?.model().root.clone();
        let Some(config) = &mut self.config else {
            return None;
        };
        let result = config
            .add(&path, name)
            .and_then(|()| config.set_active(&path));
        self.registry_warning = result.err().or_else(|| config.warning().map(str::to_owned));
        self.registry_warning.clone()
    }

    pub(super) fn remove_registration(&mut self, path: &Path) -> Gateway {
        let Some(config) = &mut self.config else {
            return self.blocked("Workspace configuration is unavailable");
        };
        // This operation intentionally never touches Session, its writer lock,
        // a draft, or the workspace directory, even for the active root.
        let before = config.workspaces().len();
        let result = config.remove(path);
        let committed = config.workspaces().len() < before;
        let warning = result.err();
        self.registry_warning = warning
            .clone()
            .or_else(|| config.warning().map(str::to_owned));
        if warning.is_none() || committed {
            self.palette = None;
        }
        self.finish_action(
            if warning.is_some() {
                if committed {
                    Disposition::CommittedWarning
                } else {
                    Disposition::Blocked
                }
            } else {
                Disposition::Completed
            },
            warning,
            focus_editor(),
        )
    }
}
