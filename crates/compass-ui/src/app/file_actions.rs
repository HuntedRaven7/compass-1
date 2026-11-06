//! Search Files' action panel, as `FileActions::actionPanel` arranges it:
//! Open, Run executable (an AppImage), Show in file browser, Open with…,
//! Set as wallpaper (an image, where the desktop allows), Create shortcut;
//! then Paste, Copy file, Copy file path, Copy file name and Copy mime type.

use compass_core::shortcut_form::Mode;

use super::{LauncherApp, Message, Page, PanelSection, PanelState, Task, focus_search};
use crate::action_panel::Action;
use crate::backend::FileActions;

const OPEN: &str = "file.open";
const RUN: &str = "file.run";
const REVEAL: &str = "file.reveal";
const OPEN_WITH: &str = "file.open-with";
const WALLPAPER: &str = "file.wallpaper";
const CREATE_SHORTCUT: &str = "file.create-shortcut";
const PASTE: &str = "file.paste";
const COPY_FILE: &str = "file.copy";
const COPY_PATH: &str = "file.copy-path";
const COPY_NAME: &str = "file.copy-name";
const COPY_MIME: &str = "file.copy-mime";

/// The panel over the file at `path`, given what it depends on.
#[must_use]
pub fn file_panel_sections(path: &str, info: &FileActions) -> Vec<PanelSection> {
    let mut main = Vec::new();
    if info.has_opener {
        main.push(Action::new("Open").with_id(OPEN).with_shortcut("enter"));
    }
    if crate::files_page::runs_as_executable(path) {
        let run = Action::new("Run executable").with_id(RUN);
        main.push(if info.has_opener {
            run
        } else {
            run.with_shortcut("enter")
        });
    }
    main.push(
        Action::new("Show in file browser")
            .with_id(REVEAL)
            .with_shortcut("ctrl+enter"),
    );
    main.push(
        Action::new("Open with...")
            .with_id(OPEN_WITH)
            .with_shortcut("ctrl+o"),
    );
    if info.can_set_wallpaper
        && info
            .mime
            .as_deref()
            .is_some_and(|mime| mime.starts_with("image/"))
    {
        main.push(
            Action::new("Set as wallpaper")
                .with_id(WALLPAPER)
                .with_shortcut("ctrl+shift+w"),
        );
    }
    main.push(Action::new("Create shortcut").with_id(CREATE_SHORTCUT));
    let mut utils = Vec::new();
    if info.can_paste {
        utils.push(
            Action::new("Paste to active window")
                .with_id(PASTE)
                .with_shortcut("ctrl+shift+v"),
        );
    }
    utils.push(
        Action::new("Copy file")
            .with_id(COPY_FILE)
            .with_shortcut("ctrl+shift+c"),
    );
    utils.push(Action::new("Copy file path").with_id(COPY_PATH));
    utils.push(Action::new("Copy file name").with_id(COPY_NAME));
    if info.mime.is_some() {
        utils.push(Action::new("Copy mime type").with_id(COPY_MIME));
    }
    vec![
        PanelSection {
            name: String::new(),
            actions: main,
        },
        PanelSection {
            name: String::new(),
            actions: utils,
        },
    ]
}

/// What the HUD says once a file action the engine carries out succeeds:
/// `SetAsWallpaperAction`'s "Wallpaper set" and the copy's "Copied to
/// clipboard"; running and pasting hide without one.
#[must_use]
pub fn file_action_hud(id: &str) -> Option<crate::hud::Hud> {
    match id {
        WALLPAPER => Some(crate::hud::Hud::new("Wallpaper set").with_icon("image")),
        COPY_FILE => Some(crate::hud::Hud::copied()),
        _ => None,
    }
}

impl LauncherApp {
    /// Asks what the selected file's panel depends on; the panel opens when
    /// the answer comes.
    pub(super) fn open_files_panel(&mut self) -> Option<Task<Message>> {
        let Page::Files(page) = &self.page else {
            return None;
        };
        let path = page.selected_row()?.path.clone();
        let Some(backend) = self.backend.clone() else {
            return Some(self.update(Message::FileActionsLoaded {
                path,
                result: Ok(FileActions::default()),
            }));
        };
        Some(Task::perform(
            async move {
                let result = backend.file_actions(path.clone()).await;
                (path, result)
            },
            |(path, result)| Message::FileActionsLoaded { path, result },
        ))
    }

    fn selected_file(&self) -> Option<(String, String)> {
        let Page::Files(page) = &self.page else {
            return None;
        };
        let row = page.selected_row()?;
        Some((row.path.clone(), row.name.clone()))
    }

    /// Runs a Search Files panel action, if `id` is one.
    pub(super) fn file_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if !id.starts_with("file.") {
            return None;
        }
        let (path, name) = self.selected_file()?;
        self.panel = None;
        let backend = self.backend.clone();
        let hides = |task: Task<Message>| Task::batch([task, focus_search()]);
        let copy = |this: &mut Self, text: String| this.copy_with_hud(text);
        Some(match id {
            OPEN => self.open_selected_file(false),
            REVEAL => self.open_selected_file(true),
            OPEN_WITH => self.open_with(path.clone(), path),
            CREATE_SHORTCUT => {
                let task = self.open_shortcut_form(Mode::Create, None, false);
                if let Page::Preferences(form) = &mut self.page {
                    crate::shortcuts_page::prefill(form, &name, &path);
                }
                task
            }
            COPY_PATH => copy(self, path),
            COPY_NAME => copy(self, name),
            COPY_MIME => match self.file_mime.take() {
                Some(mime) => copy(self, mime),
                None => Task::none(),
            },
            RUN | WALLPAPER | PASTE | COPY_FILE => {
                let Some(backend) = backend else {
                    return Some(Task::none());
                };
                let action = id.to_owned();
                let hud = file_action_hud(id);
                hides(Task::perform(
                    async move {
                        match action.as_str() {
                            RUN => backend.run_executable(path, true).await,
                            WALLPAPER => backend.set_wallpaper(path).await,
                            PASTE => backend.copy_file(path, true).await,
                            _ => backend.copy_file(path, false).await,
                        }
                    },
                    move |result| Message::ActionDone(hud.clone(), result),
                ))
            }
            _ => return None,
        })
    }

    /// Handles the file action messages.
    pub(super) fn file_actions_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FileActionsLoaded { path, result } => {
                if self
                    .selected_file()
                    .is_none_or(|(selected, _)| selected != path)
                {
                    return Task::none();
                }
                let info = result.unwrap_or_else(|error| {
                    tracing::debug!(%error, "no file actions from the engine");
                    FileActions::default()
                });
                self.panel = Some(PanelState::new(file_panel_sections(&path, &info)));
                self.file_mime = info.mime;
                iced::widget::operation::focus(super::PANEL_INPUT)
            }
            Message::FileActionDone(Ok(())) => self.conceal(),
            Message::FileActionDone(Err(reason)) => {
                self.say(reason);
                Task::none()
            }
            _ => Task::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::file_actions;
    use crate::app::test_support::{
        TestBackend, choose, file_row, files_page, index, open_builtin, opener, panel_titles,
        pressed, settle, with_resident_hud,
    };
    use crate::resident::UiCommand;
    use std::sync::Arc;

    #[test]
    fn search_files_panel_is_the_cpps_file_actions() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            files: vec![
                file_row("/home/me/sunset.png", "Images"),
                file_row("/home/me/Tool.AppImage", "Documents"),
            ],
            file_info: crate::backend::FileActions {
                mime: Some("image/png".into()),
                has_opener: true,
                can_set_wallpaper: true,
                can_paste: true,
            },
            openers: vec![opener("gimp.desktop", "GIMP", false)],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "search files", "commands:search-files");
        assert_eq!(files_page(&app).rows.len(), 2, "{}", app.state_line());

        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert_eq!(
            panel_titles(&app),
            [
                "Open",
                "Show in file browser",
                "Open with...",
                "Set as wallpaper",
                "Create shortcut",
                "Paste to active window",
                "Copy file",
                "Copy file path",
                "Copy file name",
                "Copy mime type",
            ]
        );
        let task = choose(&mut app, "Copy mime type");
        assert_eq!(settle(&mut app, task), ["image/png"]);

        let reopen = |app: &mut LauncherApp| {
            let _ = app.update(Message::Command(UiCommand::Show));
            open_builtin(app, "search files", "commands:search-files");
            let task = app.update(Message::TogglePanel);
            settle(app, task);
        };
        reopen(&mut app);
        let task = choose(&mut app, "Copy file name");
        assert_eq!(settle(&mut app, task), ["sunset.png"]);
        reopen(&mut app);
        let task = choose(&mut app, "Copy file");
        settle(&mut app, task);
        reopen(&mut app);
        let task = choose(&mut app, "Paste to active window");
        settle(&mut app, task);
        reopen(&mut app);
        let task = choose(&mut app, "Set as wallpaper");
        settle(&mut app, task);
        let Page::Files(page) = &app.page else {
            panic!("a failure stays: {}", app.state_line());
        };
        assert_eq!(
            page.notice.as_deref(),
            Some("Failed to set wallpaper: no backend")
        );
        assert_eq!(
            backend.file_calls.lock().unwrap().as_slice(),
            [
                ("copy", "/home/me/sunset.png".to_owned()),
                ("paste", "/home/me/sunset.png".to_owned()),
                ("wallpaper", "/home/me/sunset.png".to_owned()),
            ]
        );

        // Create shortcut opens the form with the file's name and path.
        let _ = app.update(Message::TogglePanel);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Create shortcut");
        settle(&mut app, task);
        let Page::Preferences(form) = &app.page else {
            panic!("no shortcut form: {}", app.state_line());
        };
        let (name, link, _, _) = crate::shortcuts_page::form_values(form);
        assert_eq!(
            (name.as_str(), link.as_str()),
            ("sunset.png", "/home/me/sunset.png")
        );

        // Open with… lists the file's openers.
        reopen(&mut app);
        let task = choose(&mut app, "Open with...");
        settle(&mut app, task);
        assert!(
            matches!(app.page, Page::OpenWith(_)),
            "{}",
            app.state_line()
        );
        assert_eq!(
            backend
                .opener_lookups
                .lock()
                .unwrap()
                .last()
                .map(String::as_str),
            Some("/home/me/sunset.png")
        );

        // An AppImage runs, and it is primary only when nothing opens it.
        let _ = app.update(Message::Command(UiCommand::Show));
        open_builtin(&mut app, "search files", "commands:search-files");
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert!(panel_titles(&app).contains(&"Run executable".to_owned()));
        let task = choose(&mut app, "Run executable");
        settle(&mut app, task);
        assert_eq!(
            backend.file_calls.lock().unwrap().last(),
            Some(&("run", "/home/me/Tool.AppImage".to_owned()))
        );
        let sections =
            super::file_panel_sections("/x/Tool.AppImage", &crate::backend::FileActions::default());
        assert_eq!(sections[0].actions[0].title, "Run executable");
        assert_eq!(sections[0].actions[0].shortcut.as_deref(), Some("enter"));
        assert!(
            !sections
                .iter()
                .flat_map(|s| &s.actions)
                .any(|a| a.title == "Set as wallpaper" || a.title == "Copy mime type")
        );
    }

    #[test]
    fn a_set_wallpaper_and_a_copied_file_say_so_and_running_does_not() {
        assert_eq!(
            file_actions::file_action_hud("file.wallpaper"),
            Some(crate::hud::Hud::new("Wallpaper set").with_icon("image"))
        );
        assert_eq!(
            file_actions::file_action_hud("file.copy"),
            Some(crate::hud::Hud::copied())
        );
        assert_eq!(file_actions::file_action_hud("file.run"), None);
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let task = app.update(Message::ActionDone(
            file_actions::file_action_hud("file.wallpaper"),
            Ok(()),
        ));
        settle(&mut app, task);
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Wallpaper set")
        );
        let task = app.update(Message::ActionDone(None, Err("no backend".into())));
        settle(&mut app, task);
        assert_eq!(app.error.as_deref(), Some("no backend"));
    }
}
