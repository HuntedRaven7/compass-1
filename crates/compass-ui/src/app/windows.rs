//! Switch Windows in the launcher: the engine's list of open windows,
//! focusing the selected one and closing it.
//!
//! The compositor-specific work is the engine's; this module draws the list
//! and turns Enter and `Ctrl+Q` into requests for it.

use super::{
    Element, LauncherApp, Length, Message, Padding, Page, Task, a11y, column, container,
    mouse_area, scrollable,
};

impl LauncherApp {
    /// Switch Windows' messages: its query, the list, and the engine's
    /// answers to focusing and closing.
    pub(super) fn windows_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowsQueryChanged(query) => {
                if let Page::Windows(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::WindowsLoaded(result) => {
                if let Page::Windows(page) = &mut self.page {
                    page.apply(result, std::process::id());
                }
                self.warm_window_icons();
                crate::scroll::reveal_root_selection()
            }
            Message::WindowSelected(position) => {
                if let Page::Windows(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.activate_selected_window()
            }
            Message::WindowActivated(result) => match result {
                // The window the user picked is in front now; get out of the way.
                Ok(()) => self.conceal(),
                Err(reason) => {
                    if let Page::Windows(page) = &mut self.page {
                        page.notice = Some(reason);
                    }
                    Task::none()
                }
            },
            Message::ShellWindowClosed(result) => {
                if let (Err(reason), Page::Windows(page)) = (&result, &mut self.page) {
                    page.notice = Some(reason.clone());
                }
                self.list_windows_task()
            }
            _ => Task::none(),
        }
    }

    pub(super) fn windows_body<'a>(
        &'a self,
        page: &'a crate::windows_page::WindowsPage,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        if let Some(empty) = page.empty_notice() {
            return self.notice(empty);
        }
        let mut list = column![].spacing(f32::from(geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(window) = page.all.get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let title = crate::windows_page::title(window).to_owned();
            let row = self.list_row(
                self.window_icon(window, selected),
                title,
                self.subtitles.then(|| window.app.clone()),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::WindowSelected(position))
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

    /// The window switcher's list, for assistive technology.
    pub(super) fn windows_semantics(
        &self,
        page: &crate::windows_page::WindowsPage,
    ) -> a11y::PageSemantics {
        let items = page
            .shown
            .iter()
            .map(|&index| {
                page.all
                    .get(index)
                    .map_or_else(crate::semantics::Item::default, |window| {
                        a11y::item(crate::windows_page::title(window), Some(window.app.clone()))
                    })
            })
            .collect();
        a11y::PageSemantics::list("Windows", items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }

    /// Asks the engine for the open windows.
    pub(super) fn list_windows_task(&mut self) -> Task<Message> {
        let Page::Windows(page) = &mut self.page else {
            return Task::none();
        };
        page.notice = None;
        let Some(windows) = self.windows.clone() else {
            page.apply(
                Err(
                    "Window switching needs the Compass engine, and this window is running \
                     without one"
                        .to_owned(),
                ),
                std::process::id(),
            );
            return Task::none();
        };
        Task::perform(
            async move { windows.list_windows().await },
            Message::WindowsLoaded,
        )
    }

    /// Switches to the selected window.
    pub(super) fn activate_selected_window(&mut self) -> Task<Message> {
        let Page::Windows(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(windows)) = (page.selected_row(), self.windows.clone()) else {
            return Task::none();
        };
        let id = row.id;
        Task::perform(
            async move { windows.activate_window(id).await },
            Message::WindowActivated,
        )
    }

    /// Closes the selected window, when it can be closed.
    pub(super) fn close_selected_window(&mut self) -> Task<Message> {
        let Page::Windows(page) = &mut self.page else {
            return Task::none();
        };
        let Some(row) = page.selected_row() else {
            return Task::none();
        };
        if !row.can_close {
            page.notice = Some(format!("{} cannot be closed", row.title));
            return Task::none();
        }
        let Some(windows) = self.windows.clone() else {
            return Task::none();
        };
        let id = row.id;
        Task::perform(
            async move { windows.close_window(id).await },
            Message::ShellWindowClosed,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{
        FakeWindows, chord, open_windows, pressed, settle, window_row, windows_app,
    };
    use std::sync::Arc;

    #[test]
    fn switching_windows_lists_others_and_enter_focuses_one_then_hides() {
        let dir = tempfile::tempdir().unwrap();
        let own = std::process::id();
        let windows = Arc::new(FakeWindows {
            rows: vec![
                window_row(7, "Downloads", "Files", 1),
                window_row(8, "Compass", "Compass", own),
                window_row(9, "notes.txt", "Text Editor", 2),
                // Inside the Flatpak the Shell reports a host pid the
                // launcher cannot see, so its window is known by app id.
                crate::backend::WindowRow {
                    wm_class: crate::APP_ID.to_owned(),
                    ..window_row(10, "Compass", "Compass", 4_000_000)
                },
            ],
            ..FakeWindows::default()
        });
        let (mut app, backend) = windows_app(dir.path(), Some(windows.clone()));
        open_windows(&mut app);
        assert!(app.showing_windows());
        let Page::Windows(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.all.len(), 2, "the launcher's own window is left out");
        assert_eq!(
            backend.recorded.lock().unwrap().as_slice(),
            ["commands:switch-windows"]
        );

        let task = app.update(Message::WindowsQueryChanged("notes".into()));
        settle(&mut app, task);
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(windows.activated.lock().unwrap().as_slice(), [9]);
        assert!(
            !app.showing_windows(),
            "hidden, and back at the root next time"
        );
    }

    #[test]
    fn control_q_closes_the_selected_window_and_reloads_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            rows: vec![window_row(3, "Old tab", "Browser", 1)],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows.clone()));
        open_windows(&mut app);
        let before = windows.listed.load(std::sync::atomic::Ordering::SeqCst);
        let task = app.update(chord("q", iced::keyboard::Modifiers::CTRL));
        settle(&mut app, task);
        assert_eq!(windows.closed.lock().unwrap().as_slice(), [3]);
        assert_eq!(
            windows.listed.load(std::sync::atomic::Ordering::SeqCst),
            before + 1,
            "the list is asked for again"
        );
        assert!(
            app.showing_windows(),
            "closing a window keeps the switcher open"
        );
    }

    #[test]
    fn a_failed_switch_says_why_and_stays_open() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            rows: vec![window_row(3, "Gone", "Browser", 1)],
            fail_activate: true,
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows));
        open_windows(&mut app);
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::Windows(page) = &app.page else {
            panic!("a failed switch must not hide the launcher")
        };
        assert_eq!(page.notice.as_deref(), Some("that window has gone"));
    }

    #[test]
    fn switching_windows_without_an_engine_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) = windows_app(dir.path(), None);
        open_windows(&mut app);
        let Page::Windows(page) = &app.page else {
            unreachable!()
        };
        assert!(
            matches!(&page.status, crate::windows_page::Status::Failed(r) if r.contains("engine")),
            "{:?}",
            page.status
        );
    }
}

#[cfg(test)]
mod semantics_tests {
    use super::*;
    use crate::backend::WindowRow;
    use crate::semantics::Body;
    use compass_core::AppIndex;

    fn app() -> LauncherApp {
        LauncherApp::with_index(AppIndex::builder().build())
    }

    fn window(id: u32, title: &str, app: &str) -> WindowRow {
        WindowRow {
            id,
            title: title.into(),
            app: app.into(),
            wm_class: app.to_lowercase(),
            pid: Some(id + 10),
            can_close: true,
            app_known: true,
        }
    }

    #[test]
    fn the_switcher_names_each_window_and_its_application() {
        let mut app = app();
        let mut page = crate::windows_page::WindowsPage::default();
        assert_eq!(
            app.windows_semantics(&page).status.as_deref(),
            Some("Loading windows…")
        );
        page.apply(
            Ok(vec![
                window(1, "Downloads", "Files"),
                window(2, "", "Text Editor"),
            ]),
            99,
        );
        page.selected = 1;
        let semantics = app.windows_semantics(&page);
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Windows");
        assert_eq!(list.items[0].name, "Downloads");
        assert_eq!(list.items[0].description.as_deref(), Some("Files"));
        assert_eq!(
            list.items[1].name, "Text Editor",
            "untitled: its application"
        );
        assert_eq!(list.selected, Some(1));
        assert_eq!(semantics.status, None);

        page.query = "zzzz".into();
        page.refilter();
        app.page = Page::Windows(page);
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(semantics.status.as_deref(), Some("No matching windows"));
    }
}
