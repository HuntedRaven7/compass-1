//! Rhai scripts in the launcher: their root rows and opening one.
//!
//! A script's view is the extension page: the engine opens it as an
//! extension view session, so everything after the first request is the
//! extension page's own code.

use super::{LauncherApp, Message, Task};

const NEEDS_ENGINE: &str =
    "Rhai scripts need the Compass engine, and this window is running without one";

/// A manifest's `icon`, a builtin icon's name, as the row draws it: in the
/// text colour, as the builtin commands' icons are.
fn manifest_icon(name: &str) -> Option<crate::extension_page::RowIcon> {
    manifest_icon_in(&compass_core::builtin_icon::directory()?, name)
}

/// [`manifest_icon`], with the builtin icons in `directory`.
fn manifest_icon_in(
    directory: &std::path::Path,
    name: &str,
) -> Option<crate::extension_page::RowIcon> {
    let path = directory.join(compass_core::builtin_icon::file_name(name)?);
    if !path.is_file() {
        return None;
    }
    Some(crate::extension_page::RowIcon::Art {
        art: crate::icons::classify(&path)?,
        monochrome: true,
        tint: None,
    })
}

impl LauncherApp {
    /// Asks the engine for its Rhai scripts, so root search shows what is on
    /// disk now.
    pub(super) fn refresh_rhai_scripts_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.list_rhai_scripts().await },
            Message::RhaiScriptsLoaded,
        )
    }

    /// Opens the Rhai script at `index` in `AppIndex::rhai_scripts`.
    pub(super) fn open_rhai_script_at(&mut self, index: usize) -> Task<Message> {
        self.panel = None;
        let Some(script) = self.app_index.rhai_scripts().get(index) else {
            return Task::none();
        };
        let Some(backend) = self.backend.clone() else {
            self.error = Some(NEEDS_ENGINE.to_owned());
            return Task::none();
        };
        let id = script.entrypoint_id();
        let title = script.title.clone();
        let started_id = id.clone();
        Task::perform(
            async move { backend.run_extension_command(id, None).await },
            move |result| Message::ExtensionCommandStarted {
                id: started_id.clone(),
                title: title.clone(),
                result,
            },
        )
    }

    /// Handles [`Message::RhaiScriptsLoaded`].
    pub(super) fn rhai_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::RhaiScriptsLoaded(Ok(scripts)) => {
                let changed = self.app_index.rhai_scripts() != scripts.as_slice();
                if changed {
                    self.rhai_icons = scripts
                        .iter()
                        .filter_map(|script| {
                            Some((script.id.clone(), manifest_icon(script.icon.as_deref()?)?))
                        })
                        .collect();
                }
                self.app_index.set_rhai_scripts(scripts);
                if changed
                    && matches!(self.page, super::Page::Root)
                    && !self.query.trim().is_empty()
                {
                    return self.search_task();
                }
                Task::none()
            }
            Message::RhaiScriptsLoaded(Err(reason)) => {
                tracing::debug!(%reason, "could not list Rhai scripts");
                Task::none()
            }
            _ => Task::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{
        TestBackend, extension_app, greeting_list, index, task_messages,
    };
    use crate::app::{Page, RootRow};
    use std::sync::Arc;

    #[test]
    fn a_manifest_icon_is_a_builtin_icon_that_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("calculator.svg"),
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"/>"#,
        )
        .unwrap();
        assert!(matches!(
            manifest_icon_in(dir.path(), "calculator"),
            Some(crate::extension_page::RowIcon::Art {
                monochrome: true,
                ..
            })
        ));
        assert!(manifest_icon_in(dir.path(), "not-an-icon").is_none());
        assert!(
            manifest_icon_in(dir.path(), "globe").is_none(),
            "a builtin name whose file is not installed"
        );
    }

    #[test]
    fn a_rhai_script_is_a_row_opens_as_an_extension_view_and_leaves_when_it_pops() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["rhai:script.hello".to_owned()],
            view: Some(greeting_list(false)),
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        let _ = app.update(Message::RhaiScriptsLoaded(Ok(vec![
            compass_core::rhai_scripts::RhaiScriptItem {
                id: "script.hello".into(),
                title: "Hello Script".into(),
                description: Some("Says hello".into()),
                icon: Some("globe".into()),
                keywords: Vec::new(),
            },
        ])));
        for message in task_messages(app.update(Message::QueryChanged("hello script".into()))) {
            let _ = app.update(message);
        }
        assert_eq!(
            app.selected_row(),
            Some(RootRow::RhaiScript(0)),
            "{}",
            app.state_line()
        );
        assert!(app.state_line().contains("selected_title=\"Hello Script\""));
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(
                ui.find("Says hello").is_ok(),
                "the description is the subtitle"
            );
        }

        // The fake draws the list on the first poll and ends on the second,
        // as a script that popped itself does.
        let mut drawn = false;
        let mut pending = task_messages(app.update(Message::LaunchSelected));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
            if let Page::Extension(page) = &app.page {
                assert!(page.leaves_on_end);
                drawn |= page.shown.len() == 2;
            }
        }
        assert_eq!(
            backend.ran.lock().unwrap().as_slice(),
            ["rhai:script.hello"]
        );
        assert!(drawn, "the script's list was drawn");
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
        assert_eq!(backend.closed.lock().unwrap().as_slice(), [7]);
    }

    #[test]
    fn a_rhai_script_row_draws_its_manifest_icon_when_installed() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        let _ = app.update(Message::RhaiScriptsLoaded(Ok(vec![
            compass_core::rhai_scripts::RhaiScriptItem {
                id: "script.no-icon".into(),
                title: "No Icon".into(),
                description: None,
                icon: Some("not-a-builtin".into()),
                keywords: Vec::new(),
            },
        ])));
        assert!(
            app.rhai_icons.is_empty(),
            "an unknown name draws the initial"
        );
    }
}
