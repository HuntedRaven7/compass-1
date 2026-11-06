//! Launches an extension asked for, and the subtitles extensions set.
//!
//! An extension's `launchCommand` and `openCommandPreferences` reach the
//! launcher as a token (`UiCommand::Launch`): the launch is fetched from the
//! engine and taken as if the command had been picked in root search — its
//! arguments form, its preferences form and its view all follow from there.
//! The view that asked is closed first: the launcher shows one extension
//! view at a time, where the C++ pushes the new one above it.

use super::{LauncherApp, Message, Page, Task, focus_search};

impl LauncherApp {
    /// Fetches the launch under `token`.
    pub(super) fn start_launch(&mut self, token: u64) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.fetch_launch(token).await },
            Message::LaunchFetched,
        )
    }

    /// Opens a shortcut, a script command or a Rhai script by its root id,
    /// as picking its row does; `None` when the id is none of those.
    fn open_root_item(&mut self, id: &str) -> Option<Task<Message>> {
        let index = &self.app_index;
        if let Some(index) = index.shortcut_by_entrypoint(id).and_then(|found| {
            index
                .shortcuts()
                .iter()
                .position(|s| std::ptr::eq(s, found))
        }) {
            return Some(self.open_shortcut_at(index));
        }
        if let Some(index) = index
            .script_by_entrypoint(id)
            .and_then(|found| index.scripts().iter().position(|s| std::ptr::eq(s, found)))
        {
            return Some(self.run_script_at(index));
        }
        let found = index.rhai_script_by_entrypoint(id)?;
        let position = index
            .rhai_scripts()
            .iter()
            .position(|s| std::ptr::eq(s, found))?;
        Some(self.open_rhai_script_at(position))
    }

    /// Types `text` into whatever search field the view just opened shows,
    /// as the C++ gives a command its launch's `fallbackText`.
    pub(super) fn type_fallback(&mut self, text: Option<String>) -> Task<Message> {
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return Task::none();
        };
        match self.search_field().2 {
            Some(on_input) => self.update(on_input(text)),
            None => Task::none(),
        }
    }

    /// Asks the engine for the subtitles extensions set.
    pub(super) fn refresh_subtitles_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.extension_subtitles().await },
            Message::ExtensionSubtitlesLoaded,
        )
    }

    /// Handles [`Message::LaunchFetched`], [`Message::ExtensionSubtitlesLoaded`]
    /// and [`Message::PreferencesOpened`].
    pub(super) fn launch_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LaunchFetched(Ok(launch)) => {
                let close = self.close_extension_view();
                self.panel = None;
                self.page = Page::Root;
                let Some(backend) = self.backend.clone() else {
                    return close;
                };
                if launch.preferences {
                    let id = launch.id;
                    let opened = id.clone();
                    return Task::batch([
                        close,
                        Task::perform(
                            async move { backend.extension_preferences(id).await },
                            move |result| Message::PreferencesOpened {
                                id: opened.clone(),
                                result,
                            },
                        ),
                    ]);
                }
                if let Some(command) = compass_core::commands::by_id(&launch.id) {
                    let opened = self.open_command(command);
                    let typed = self.type_fallback(launch.fallback_text);
                    return Task::batch([close, opened, typed]);
                }
                if let Some(opened) = self.open_root_item(&launch.id) {
                    return Task::batch([close, opened]);
                }
                let run = match self
                    .app_index
                    .extensions()
                    .iter()
                    .position(|command| command.id == launch.id)
                {
                    Some(index) => self.run_extension_command_with(index, launch.arguments),
                    // The engine knows a command this window's index does
                    // not yet: run it by id, titled by it until its view
                    // names itself.
                    None => {
                        let (id, title) = (launch.id.clone(), launch.id);
                        let arguments = launch.arguments;
                        let started = id.clone();
                        Task::perform(
                            async move { backend.run_extension_command(id, arguments).await },
                            move |result| Message::ExtensionCommandStarted {
                                id: started.clone(),
                                title: title.clone(),
                                result,
                            },
                        )
                    }
                };
                Task::batch([close, run])
            }
            Message::LaunchFetched(Err(reason)) => {
                self.error = Some(reason);
                Task::none()
            }
            Message::ExtensionSubtitlesLoaded(Ok(subtitles)) => {
                self.extension_subtitles = subtitles.into_iter().collect();
                Task::none()
            }
            Message::ExtensionSubtitlesLoaded(Err(reason)) => {
                tracing::debug!(%reason, "no extension subtitles");
                Task::none()
            }
            Message::PreferencesOpened { id, result } => match result {
                Ok(crate::backend::ExtensionStart::NeedsPreferences { title, fields }) => {
                    self.page =
                        Page::Preferences(Box::new(crate::preferences_page::PreferencesPage::new(
                            crate::preferences_page::Purpose::CommandPreferences,
                            id,
                            title,
                            fields,
                        )));
                    focus_search()
                }
                Ok(other) => {
                    tracing::debug!(?other, "a preferences request answered something else");
                    Task::none()
                }
                Err(reason) => {
                    // Opened from the settings: back to them, saying why.
                    if let Some(mut settings) = self.parked_settings.take() {
                        settings.notice = Some(reason);
                        self.page = Page::Settings(settings);
                        return focus_search();
                    }
                    self.error = Some(reason);
                    Task::none()
                }
            },
            _ => Task::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{TestBackend, extension_app, files_page, index, task_messages};
    use std::sync::Arc;

    #[test]
    fn a_command_line_launch_opens_the_builtin_and_types_its_fallback_text() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend);
        let launch = crate::backend::ExtensionLaunch {
            id: "commands:search-files".into(),
            arguments: None,
            preferences: false,
            fallback_text: Some("report".into()),
        };
        // Not settled: the search waits out its debounce on a timer.
        let _ = app.update(Message::LaunchFetched(Ok(launch)));
        assert_eq!(files_page(&app).query, "report", "{}", app.state_line());

        let launch = crate::backend::ExtensionLaunch {
            id: "commands:manage-snippets".into(),
            arguments: None,
            preferences: false,
            fallback_text: None,
        };
        let _ = app.update(Message::LaunchFetched(Ok(launch)));
        assert!(
            matches!(app.page, Page::Snippets(_)),
            "{}",
            app.state_line()
        );
    }

    #[test]
    fn an_extension_s_launch_runs_its_command_and_its_subtitle_override_shows() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["@someone/hello:write".to_owned()],
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        let _ = app.update(Message::ExtensionSubtitlesLoaded(Ok(vec![(
            "@someone/hello:write".into(),
            "3 unread".into(),
        )])));
        for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
            let _ = app.update(message);
        }
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("3 unread").is_ok(), "the override is the subtitle");
            assert!(
                ui.find("Hello").is_err(),
                "in place of the extension's title"
            );
        }

        let mut arguments = serde_json::Map::new();
        arguments.insert("who".into(), "ada".into());
        let launch = crate::backend::ExtensionLaunch {
            id: "@someone/hello:write".into(),
            arguments: Some(arguments.clone()),
            preferences: false,
            fallback_text: None,
        };
        for message in task_messages(app.update(Message::LaunchFetched(Ok(launch)))) {
            let _ = app.update(message);
        }
        assert_eq!(
            backend.ran.lock().unwrap().as_slice(),
            ["@someone/hello:write"]
        );
        assert_eq!(backend.given.lock().unwrap().last(), Some(&Some(arguments)));
    }

    #[test]
    fn preferences_an_extension_opens_are_saved_without_running_it() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = extension_app(dir.path(), backend.clone());
        let _ = app.update(Message::PreferencesOpened {
            id: "@someone/hello:write".into(),
            result: Ok(crate::backend::ExtensionStart::NeedsPreferences {
                title: "Write Greeting".into(),
                fields: Vec::new(),
            }),
        });
        let Page::Preferences(page) = &app.page else {
            panic!("no preferences form: {}", app.state_line());
        };
        assert_eq!(
            page.purpose,
            crate::preferences_page::Purpose::CommandPreferences
        );
        for message in task_messages(app.update(Message::PreferencesSubmit)) {
            let _ = app.update(message);
        }
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
        assert_eq!(backend.saved.lock().unwrap().len(), 1);
        assert!(
            backend.ran.lock().unwrap().is_empty(),
            "saving does not run it"
        );
    }
}
