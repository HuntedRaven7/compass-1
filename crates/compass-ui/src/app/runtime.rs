//! Quit and Force Quit: the C++ `QuitAppAction` and `ForceQuitAppAction`.
//!
//! An application's root row offers them in its panel while it is running
//! (`isRunning`: it has a window), with Focus Window and Close Window, as
//! `AppRootItem::newActionPanel` does; the panel opens at once and the
//! running-only actions join it when the engine answers. The window switcher
//! gets the C++'s panel too — Focus Window, Close Window (Ctrl+Q), and Quit
//! and Force Quit for a window whose application is known. Neither asks
//! first, as the C++ does not; both hide the launcher once done and say why
//! when nothing could be quit.

use super::{LauncherApp, Message, Page, PanelState, Task, focus_search};
use crate::action_panel::{Action, PanelSection};
use crate::backend::AppRuntimeInfo;

/// Panel ids for a running application's actions.
pub(super) const APP_FOCUS_WINDOW: &str = "app.focus-window";
pub(super) const APP_CLOSE_WINDOW: &str = "app.close-window";
pub(super) const APP_QUIT: &str = "app.quit";
pub(super) const APP_FORCE_QUIT: &str = "app.force-quit";

/// Panel ids for the window switcher's actions.
const WINDOW_FOCUS: &str = "window.focus";
const WINDOW_CLOSE: &str = "window.close";
const WINDOW_QUIT: &str = "window.quit";
const WINDOW_FORCE_QUIT: &str = "window.force-quit";

/// An application's panel with what it offers while running: Focus Window
/// and Close Window after Open, and Quit (Ctrl+Q) and Force Quit in a
/// section of their own after the copies, as the C++ lifecycle section.
#[must_use]
pub fn running_sections(mut sections: Vec<PanelSection>) -> Vec<PanelSection> {
    if let Some(primary) = sections.first_mut() {
        let at = primary.actions.len().min(1);
        primary
            .actions
            .insert(at, Action::new("Close window").with_id(APP_CLOSE_WINDOW));
        primary
            .actions
            .insert(at, Action::new("Focus window").with_id(APP_FOCUS_WINDOW));
    }
    sections.push(PanelSection {
        name: String::new(),
        actions: vec![
            Action::new("Quit application")
                .with_id(APP_QUIT)
                .with_shortcut("ctrl+q"),
            Action::new("Force quit application").with_id(APP_FORCE_QUIT),
        ],
    });
    sections
}

/// The window switcher's panel for `row`, as `SwitchWindowsSection`
/// builds it.
#[must_use]
pub fn window_sections(row: &crate::backend::WindowRow) -> Vec<PanelSection> {
    let mut sections = vec![PanelSection {
        name: "Window Actions".to_owned(),
        actions: vec![
            Action::new("Focus window")
                .with_id(WINDOW_FOCUS)
                .with_shortcut("enter"),
            Action::new("Close window")
                .with_id(WINDOW_CLOSE)
                .with_shortcut("ctrl+q"),
        ],
    }];
    if row.app_known {
        sections.push(PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("Quit application").with_id(WINDOW_QUIT),
                Action::new("Force quit application").with_id(WINDOW_FORCE_QUIT),
            ],
        });
    }
    sections
}

/// What the HUD says once an application quits, as `QuitAppAction` and
/// `ForceQuitAppAction`.
#[must_use]
pub fn quit_hud(name: &str, force: bool) -> crate::hud::Hud {
    crate::hud::Hud::new(if force {
        format!("Force quit {name}")
    } else {
        format!("Quit {name}")
    })
}

impl LauncherApp {
    /// Asks whether the application under a just-opened panel runs.
    pub(super) fn app_runtime_task(&self, key: String, desktop_id: String) -> Task<Message> {
        let Some(windows) = self.windows.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { windows.app_runtime(desktop_id).await },
            move |result| Message::AppRuntimeLoaded {
                key: key.clone(),
                result,
            },
        )
    }

    /// Handles [`Message::AppRuntimeLoaded`] and [`Message::AppQuit`], which
    /// also answers Focus Window and Close Window from the root row.
    pub(super) fn runtime_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::AppRuntimeLoaded { key, result } => {
                let info = match result {
                    Ok(info) => info,
                    Err(reason) => {
                        tracing::debug!(%reason, "no answer on whether the application runs");
                        return Task::none();
                    }
                };
                if self.selected_item().is_none_or(|item| item.key() != key) {
                    return Task::none();
                }
                let Some(open) = self.panel.as_ref() else {
                    return Task::none();
                };
                let already = open.sections.iter().any(|section| {
                    section
                        .actions
                        .iter()
                        .any(|a| a.id.as_deref() == Some(APP_QUIT))
                });
                if !info.running || already {
                    return Task::none();
                }
                // Into the panel as it is, the root row's item actions and
                // all, keeping what has been typed into its filter.
                let filter = open.filter.clone();
                let mut sections = running_sections(open.sections.clone());
                // The lifecycle section goes before the root row's item actions,
                // which `RootSearchActionGenerator` appends last.
                if matches!(self.page, Page::Root) && sections.len() >= 3 {
                    let at = sections.len() - 1;
                    sections.swap(at - 1, at);
                }
                let mut panel = PanelState::new(sections);
                panel.set_filter(filter);
                self.panel = Some(panel);
                self.app_runtime = Some((key, info));
                Task::none()
            }
            Message::AppQuit(result) => match result {
                Ok(()) => self.conceal(),
                Err(reason) => {
                    match &mut self.page {
                        Page::Windows(page) => page.notice = Some(reason),
                        _ => self.error = Some(reason),
                    }
                    Task::none()
                }
            },
            _ => Task::none(),
        }
    }

    /// Runs a running application's panel action, if `id` is one.
    pub(super) fn app_runtime_action(&mut self, id: &str) -> Option<Task<Message>> {
        if ![APP_FOCUS_WINDOW, APP_CLOSE_WINDOW, APP_QUIT, APP_FORCE_QUIT].contains(&id) {
            return None;
        }
        let item = self.selected_item()?;
        let (key, desktop_id) = (item.key().to_owned(), item.desktop_id().to_owned());
        let name = item.name().to_owned();
        let windows = self.windows.clone()?;
        let first = self
            .app_runtime
            .as_ref()
            .filter(|(known, _)| *known == key)
            .and_then(|(_, info): &(String, AppRuntimeInfo)| info.windows.first())
            .map(|row| row.id);
        self.panel = None;
        let task = match id {
            APP_FOCUS_WINDOW => {
                let first = first?;
                Task::perform(
                    async move { windows.activate_window(first).await },
                    Message::AppQuit,
                )
            }
            APP_CLOSE_WINDOW => {
                let first = first?;
                Task::perform(
                    async move { windows.close_window(first).await },
                    Message::AppQuit,
                )
            }
            _ => {
                let force = id == APP_FORCE_QUIT;
                let hud = quit_hud(&name, force);
                Task::perform(
                    async move { windows.quit_app(desktop_id, force).await },
                    move |result| Message::ActionDone(Some(hud.clone()), result),
                )
            }
        };
        Some(Task::batch([task, focus_search()]))
    }

    /// The window switcher's panel over the selected window.
    pub(super) fn open_windows_panel(&mut self) -> Option<Task<Message>> {
        let Page::Windows(page) = &self.page else {
            return None;
        };
        let row = page.selected_row()?;
        self.panel = Some(PanelState::new(window_sections(row)));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a window switcher panel action, if `id` is one.
    pub(super) fn windows_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if ![WINDOW_FOCUS, WINDOW_CLOSE, WINDOW_QUIT, WINDOW_FORCE_QUIT].contains(&id) {
            return None;
        }
        let Page::Windows(page) = &self.page else {
            return None;
        };
        let selected = page.selected_row()?;
        let (window, name) = (selected.id, selected.app.clone());
        self.panel = None;
        Some(match id {
            WINDOW_FOCUS => self.activate_selected_window(),
            WINDOW_CLOSE => self.close_selected_window(),
            _ => {
                let windows = self.windows.clone()?;
                let force = id == WINDOW_FORCE_QUIT;
                let hud = quit_hud(&name, force);
                Task::perform(
                    async move { windows.quit_window_app(window, force).await },
                    move |result| Message::ActionDone(Some(hud.clone()), result),
                )
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{
        FakeWindows, choose, open_windows, panel_titles, settle, window_row, windows_app,
        with_resident_hud,
    };
    use crate::resident::UiCommand;
    use std::sync::Arc;

    #[test]
    fn a_running_applications_panel_offers_quit_and_force_quit_and_they_reach_the_engine() {
        let dir = tempfile::tempdir().unwrap();
        let running = crate::backend::AppRuntimeInfo {
            running: true,
            frontmost: false,
            windows: vec![window_row(31, "Mozilla Firefox", "Firefox", 5)],
        };
        let windows = Arc::new(FakeWindows {
            running: vec![("firefox.desktop".into(), running)],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows.clone()));

        // Not running: the panel is the plain one, and stays so.
        app.query = "Terminal".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert!(!panel_titles(&app).iter().any(|t| t.contains("Quit")));
        let _ = app.update(Message::TogglePanel);

        app.query = "Firefox".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert_eq!(
            panel_titles(&app),
            [
                "Open",
                "Focus window",
                "Close window",
                "Copy name",
                "Copy path",
                "Quit application",
                "Force quit application",
                "Copy deeplink",
                "Reset ranking",
                "Add to favorites",
                "Set alias",
                "Set global shortcut",
                "Open preferences",
                "Copy ID",
                "Disable item"
            ]
        );
        let quit_row = app
            .panel
            .as_ref()
            .unwrap()
            .sections
            .iter()
            .find(|section| {
                section
                    .actions
                    .iter()
                    .any(|a| a.id.as_deref() == Some(super::APP_QUIT))
            })
            .unwrap();
        assert_eq!(quit_row.actions[0].shortcut.as_deref(), Some("ctrl+q"));

        let task = choose(&mut app, "Force quit application");
        settle(&mut app, task);
        assert_eq!(
            windows.quits.lock().unwrap().as_slice(),
            [("firefox.desktop".to_owned(), true)]
        );

        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Quit application");
        settle(&mut app, task);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Focus window");
        settle(&mut app, task);
        assert_eq!(
            windows.quits.lock().unwrap().last(),
            Some(&("firefox.desktop".to_owned(), false))
        );
        assert_eq!(windows.activated.lock().unwrap().as_slice(), [31]);
    }

    #[test]
    fn a_quit_that_does_nothing_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let running = crate::backend::AppRuntimeInfo {
            running: true,
            frontmost: true,
            windows: vec![window_row(31, "Files", "Files", 5)],
        };
        let windows = Arc::new(FakeWindows {
            running: vec![("files.desktop".into(), running)],
            fail_quit: true,
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows));
        app.query = "Files".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Quit application");
        settle(&mut app, task);
        assert_eq!(app.error.as_deref(), Some("Failed to quit files.desktop"));
    }

    #[test]
    fn quit_and_force_quit_hide_with_the_cpps_hud() {
        let dir = tempfile::tempdir().unwrap();
        let running = crate::backend::AppRuntimeInfo {
            running: true,
            frontmost: true,
            windows: vec![window_row(31, "Files", "Files", 5)],
        };
        let windows = Arc::new(FakeWindows {
            running: vec![("files.desktop".into(), running)],
            ..FakeWindows::default()
        });
        let (app, _) = windows_app(dir.path(), Some(windows));
        let mut app = with_resident_hud(app);
        app.query = "Files".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Quit application");
        settle(&mut app, task);
        assert_eq!(app.hud_content(), Some(&crate::hud::Hud::new("Quit Files")));
        assert!(matches!(app.page, Page::Root), "the launcher hid");

        app.query = "Files".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Force quit application");
        settle(&mut app, task);
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Force quit Files")
        );
    }

    #[test]
    fn the_window_switchers_panel_quits_a_known_windows_application() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            rows: vec![
                window_row(7, "Downloads", "Files", 1),
                crate::backend::WindowRow {
                    app_known: false,
                    ..window_row(9, "xterm", "XTerm", 2)
                },
            ],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows.clone()));
        open_windows(&mut app);
        let _ = app.update(Message::WindowsQueryChanged("Downloads".into()));
        let _ = app.update(Message::TogglePanel);
        assert_eq!(
            panel_titles(&app),
            [
                "Focus window",
                "Close window",
                "Quit application",
                "Force quit application"
            ]
        );
        let task = choose(&mut app, "Force quit application");
        settle(&mut app, task);
        assert_eq!(windows.window_quits.lock().unwrap().as_slice(), [(7, true)]);

        let _ = app.update(Message::Command(UiCommand::Show));
        open_windows(&mut app);
        let _ = app.update(Message::WindowsQueryChanged("xterm".into()));
        let _ = app.update(Message::TogglePanel);
        assert_eq!(
            panel_titles(&app),
            ["Focus window", "Close window"],
            "no application to quit"
        );
        let task = choose(&mut app, "Close window");
        settle(&mut app, task);
        assert_eq!(windows.closed.lock().unwrap().as_slice(), [9]);
    }
}
