//! The window-management commands beyond Switch Windows: Switch Workspaces
//! (`SwitchWorkspacesViewHost`) and the fullscreen, floating and overview
//! toggles (`wm-extension.cpp`), offered where the compositor can do them.

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
    text,
};
use crate::action_panel::Action;
use crate::backend::{WORKSPACES_NEED_ENGINE, WindowToggle};
use crate::workspaces_page::{self, WorkspacesPage};

const SWITCH: &str = "workspace.switch";

impl LauncherApp {
    /// Asks the engine what the window manager can do, so root search offers
    /// only the commands it can run.
    pub(super) fn window_capabilities_task(&self) -> Task<Message> {
        let Some(windows) = self.windows.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { windows.window_manager_capabilities().await },
            Message::WindowCapabilities,
        )
    }

    /// Opens Switch Workspaces and asks for the workspaces.
    pub(super) fn open_switch_workspaces(&mut self) -> Task<Message> {
        self.page = Page::Workspaces(WorkspacesPage::default());
        Task::batch([self.list_workspaces_task(), focus_search()])
    }

    fn list_workspaces_task(&mut self) -> Task<Message> {
        let Page::Workspaces(page) = &mut self.page else {
            return Task::none();
        };
        let Some(windows) = self.windows.clone() else {
            page.apply(Err(WORKSPACES_NEED_ENGINE.to_owned()));
            return Task::none();
        };
        Task::perform(
            async move { windows.list_workspaces().await },
            Message::WorkspacesLoaded,
        )
    }

    /// Runs a toggle, as `ToggleFullscreenWindowCommand` and its siblings:
    /// the launcher goes once it is done, and stays to say why it was not.
    pub(super) fn run_window_toggle(&mut self, toggle: WindowToggle) -> Task<Message> {
        let Some(windows) = self.windows.clone() else {
            self.error = Some(WORKSPACES_NEED_ENGINE.to_owned());
            return Task::none();
        };
        Task::perform(
            async move { windows.toggle_window_state(toggle).await },
            Message::WindowToggled,
        )
    }

    fn switch_to_selected_workspace(&mut self) -> Task<Message> {
        self.panel = None;
        let Page::Workspaces(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(windows)) = (page.selected_row(), self.windows.clone()) else {
            return Task::none();
        };
        let id = row.id.clone();
        Task::perform(
            async move { windows.focus_workspace(id).await },
            Message::WorkspaceFocused,
        )
    }

    /// The panel over the selected workspace: its one action.
    pub(super) fn open_workspaces_panel(&mut self) -> Option<Task<Message>> {
        let Page::Workspaces(page) = &self.page else {
            return None;
        };
        page.selected_row()?;
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new(compass_core::window_switcher::switch_to_workspace_label(
                    false,
                ))
                .with_id(SWITCH)
                .with_shortcut("enter"),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a Switch Workspaces panel action, if `id` is one.
    pub(super) fn workspaces_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if id != SWITCH || !matches!(self.page, Page::Workspaces(_)) {
            return None;
        }
        Some(self.switch_to_selected_workspace())
    }

    /// The view's keys.
    pub(super) fn workspaces_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Workspaces(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.switch_to_selected_workspace(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.shown.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Handles the window-management messages.
    pub(super) fn workspaces_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowCapabilities(Ok(caps)) => {
                if caps != self.app_index.window_capabilities() {
                    self.app_index.set_window_capabilities(caps);
                    if matches!(self.page, Page::Root) {
                        return self.search_task();
                    }
                }
                Task::none()
            }
            Message::WindowCapabilities(Err(error)) => {
                tracing::debug!(%error, "no window manager capabilities");
                Task::none()
            }
            Message::WorkspacesQueryChanged(query) => {
                if let Page::Workspaces(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::WorkspacesLoaded(result) => {
                if let Page::Workspaces(page) = &mut self.page {
                    page.apply(result);
                }
                Task::none()
            }
            Message::WorkspaceSelected(position) => {
                if let Page::Workspaces(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.switch_to_selected_workspace()
            }
            Message::WorkspaceFocused(Ok(())) | Message::WindowToggled(Ok(())) => self.conceal(),
            Message::WorkspaceFocused(Err(reason)) => {
                if let Page::Workspaces(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            Message::WindowToggled(Err(reason)) => {
                self.error = Some(reason);
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The view's body.
    pub(super) fn workspaces_body<'a>(&'a self, page: &'a WorkspacesPage) -> Element<'a, Message> {
        if let Some(empty) = page.empty_notice() {
            return self.notice(empty);
        }
        let mut list = column![
            container(text(workspaces_page::SECTION).font(self.font()).size(12))
                .padding(Padding::new(4.0).left(10))
        ]
        .spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(workspace) = page.all.get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let apps = workspace_apps(workspace);
            let row = self.list_row_with(
                self.initial_badge(&workspace.name, selected),
                workspace.name.clone(),
                self.subtitles.then(|| workspaces_page::subtitle(workspace)),
                (!apps.is_empty()).then_some(apps),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::WorkspaceSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// Switch Workspaces' rows, for assistive technology: each workspace's
    /// name, then its line and the applications on it, as the row shows them.
    pub(super) fn workspaces_semantics(&self, page: &WorkspacesPage) -> super::a11y::PageSemantics {
        let items = page
            .shown
            .iter()
            .map(|&index| {
                page.all
                    .get(index)
                    .map_or_else(crate::semantics::Item::default, |workspace| {
                        let description = [
                            workspaces_page::subtitle(workspace),
                            workspace_apps(workspace),
                        ]
                        .into_iter()
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                        super::a11y::item(workspace.name.clone(), Some(description))
                    })
            })
            .collect();
        super::a11y::PageSemantics::list(
            workspaces_page::SECTION,
            items,
            page.selected,
            page.empty_notice(),
        )
        .with_status(page.notice.as_deref(), &page.query)
    }
}

/// The applications on a workspace, as its row lists them beside the title.
fn workspace_apps(workspace: &crate::backend::WorkspaceRow) -> String {
    workspace
        .apps
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RootRow;
    use crate::app::test_support::{FakeWindows, choose, panel_titles, settle, windows_app};
    use crate::backend::WorkspaceRow;
    use crate::resident::UiCommand;
    use crate::semantics::Body;
    use std::sync::Arc;

    fn workspace(id: &str, name: &str, apps: &[&str]) -> WorkspaceRow {
        WorkspaceRow {
            id: id.into(),
            name: name.into(),
            monitor: Some("DP-1".into()),
            window_count: apps.len(),
            apps: apps.iter().map(|a| ((*a).to_owned(), None)).collect(),
            active: false,
        }
    }

    #[test]
    fn workspaces_are_described_as_their_rows_show_them() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = WorkspacesPage::default();
        assert_eq!(
            app.workspaces_semantics(&page).status.as_deref(),
            Some("Loading workspaces…")
        );
        page.apply(Ok(vec![
            workspace("1", "web", &["Firefox", "Foot"]),
            workspace("2", "mail", &[]),
        ]));
        page.selected = 1;
        let semantics = app.workspaces_semantics(&page);
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, workspaces_page::SECTION);
        assert_eq!(list.items[0].name, "web");
        assert_eq!(
            list.items[0].description.as_deref(),
            Some("2 windows - DP-1 · Firefox, Foot")
        );
        assert_eq!(list.items[1].description.as_deref(), Some("empty - DP-1"));
        assert_eq!(list.selected, Some(1));
        assert_eq!(semantics.status, None);

        page.query = "zzzz".into();
        page.refilter();
        app.page = Page::Workspaces(page);
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(semantics.status.as_deref(), Some("No workspaces match"));
    }

    #[test]
    fn workspaces_and_the_toggles_are_offered_where_the_compositor_has_them() {
        use compass_core::window_switcher::Capabilities;
        let dir = tempfile::tempdir().unwrap();
        let workspace = |id: &str, name: &str, apps: &[&str]| crate::backend::WorkspaceRow {
            id: id.into(),
            name: name.into(),
            monitor: Some("DP-1".into()),
            window_count: apps.len(),
            apps: apps.iter().map(|a| ((*a).to_owned(), None)).collect(),
            active: false,
        };
        let windows = Arc::new(FakeWindows {
            caps: Capabilities {
                workspaces: true,
                fullscreen: true,
                toggle_floating: true,
                ..Capabilities::default()
            },
            workspaces: vec![
                workspace("1", "1", &["Firefox"]),
                workspace("3", "music", &["Spotify"]),
            ],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows.clone()));
        let is_command = |app: &LauncherApp, entrypoint: &str| matches!(app.selected_row(), Some(RootRow::Command(c)) if c.entrypoint == entrypoint);

        // Until the engine says what the compositor can do, none is offered.
        app.query = "switch workspaces".into();
        app.search();
        assert!(
            !is_command(&app, "switch-workspaces"),
            "{}",
            app.state_line()
        );
        let task = app.window_capabilities_task();
        settle(&mut app, task);
        app.search();
        assert!(
            is_command(&app, "switch-workspaces"),
            "{}",
            app.state_line()
        );
        app.query = "toggle overview".into();
        app.search();
        assert!(
            !is_command(&app, "toggle-overview"),
            "no overview on this compositor"
        );

        // Switch Workspaces lists them, filters by an application on one,
        // and Enter switches to it, then hides.
        app.query = "switch workspaces".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let Page::Workspaces(page) = &app.page else {
            panic!("not on Switch Workspaces: {}", app.state_line());
        };
        assert_eq!(page.shown.len(), 2);
        let _ = app.update(Message::WorkspacesQueryChanged("spotify".into()));
        let _ = app.update(Message::TogglePanel);
        assert_eq!(panel_titles(&app), ["Switch to workspace"]);
        let task = choose(&mut app, "Switch to workspace");
        settle(&mut app, task);
        assert_eq!(windows.focused_workspaces.lock().unwrap().as_slice(), ["3"]);
        assert!(
            !matches!(app.page, Page::Workspaces(_)),
            "hidden, and back at the root next time"
        );

        // A toggle runs from root search.
        let _ = app.update(Message::Command(UiCommand::Show));
        app.query = "toggle floating".into();
        app.search();
        assert!(is_command(&app, "toggle-floating"));
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            windows.toggles.lock().unwrap().as_slice(),
            [crate::backend::WindowToggle::Floating]
        );
    }

    #[test]
    fn a_refused_toggle_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            caps: compass_core::window_switcher::Capabilities {
                fullscreen: true,
                ..Default::default()
            },
            toggle_refusal: Some("Active window is not on the current workspace"),
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows));
        let task = app.window_capabilities_task();
        settle(&mut app, task);
        app.query = "toggle fullscreen".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            app.error.as_deref(),
            Some("Active window is not on the current workspace")
        );
    }
}
