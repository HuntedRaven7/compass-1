//! Search Files in the launcher: the engine's recent files, then its
//! search once typing has paused, the category filter, the selected file's
//! preview, and opening or revealing it.
//!
//! The panel's file actions are `file_actions`'s; this module draws the
//! list and keeps its answers in order against the query.

use super::panel::PanelState;
use super::{
    Alignment, Element, LauncherApp, Length, Message, Padding, Page, Task, a11y, column, container,
    file_actions, focus_search, mouse_area, preview, row, scrollable, text,
};

impl LauncherApp {
    /// Search Files' messages: its query, its answers and opening a file.
    pub(super) fn files_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FilesQueryChanged(query) => {
                if let Page::Files(page) = &mut self.page {
                    page.set_query(query);
                }
                self.files_query_task()
            }
            Message::FilesDebounced(generation) => self.files_search_task(generation),
            Message::FilesCategoryChanged(key) => {
                let Page::Files(page) = &mut self.page else {
                    return Task::none();
                };
                let generation = page.set_category(&key);
                self.view
                    .view_memory
                    .set(crate::view_memory::FILE_CATEGORY, &key);
                Task::batch([self.files_search_task(generation), focus_search()])
            }
            Message::FilesLoaded { generation, result } => {
                if let Page::Files(page) = &mut self.page {
                    page.apply(generation, result);
                }
                self.warm_file_icons();
                crate::scroll::reveal_root_selection()
            }
            Message::FilesSelected(position) => {
                if let Page::Files(page) = &mut self.page
                    && position < page.rows.len()
                {
                    page.selected = position;
                    page.refresh_preview();
                }
                self.open_selected_file(false)
            }
            Message::FileOpened(Ok(())) => self.conceal(),
            Message::FileOpened(Err(reason)) => {
                if let Page::Files(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// Search Files' body: its state, or its heading and rows.
    pub(super) fn files_body<'a>(
        &'a self,
        page: &'a crate::files_page::FilesPage,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        let key = page
            .category
            .clone()
            .unwrap_or_else(|| compass_core::file_search::CATEGORY_FILTER_KEYS[0].to_owned());
        let filter = container(
            iced::widget::pick_list(
                compass_core::file_search::CATEGORY_FILTER_KEYS
                    .iter()
                    .map(|key| (*key).to_owned())
                    .collect::<Vec<_>>(),
                Some(key),
                Message::FilesCategoryChanged,
            )
            .text_size(12)
            .font(self.font())
            .style({
                let palette = self.palette();
                move |_, status| crate::design::dropdown(palette, status)
            })
            .menu_style({
                let palette = self.palette();
                move |_| crate::design::dropdown_menu(palette)
            }),
        )
        .width(Length::Fill)
        .align_x(Alignment::End)
        .padding(Padding::new(4.0).right(10));
        // The loading indicator (`setLoading`): a query is out.
        let indicator = container(
            text(page.indicator().unwrap_or_default())
                .font(self.font())
                .size(12)
                .color(self.palette().muted.to_iced()),
        )
        .padding(Padding::new(8.0).left(12));
        let filter = row![indicator, filter];
        if let Some(empty) = page.empty_notice() {
            return column![filter, self.notice(empty)].into();
        }
        let home =
            compass_core::xdg_dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
        let heading = text(page.heading.clone())
            .font(self.font())
            .size(12)
            .color(self.palette().muted.to_iced());
        let mut list = column![container(heading).padding(Padding::new(4.0).left(10))]
            .spacing(f32::from(geometry.row_spacing));
        for (position, file) in page.rows.iter().enumerate() {
            let selected = position == page.selected;
            let subtitle = self
                .subtitles
                .then(|| crate::files_page::subtitle(file, home.as_deref()));
            let row = self.list_row(
                self.glyph_or_initial(
                    self.icons
                        .then(|| self.file_glyphs.cached(&file.path))
                        .flatten(),
                    &file.name,
                    selected,
                ),
                file.name.clone(),
                subtitle,
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::FilesSelected(position))
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
        let rows: Element<Message> = match &page.preview {
            Some(preview) => row![
                container(rows).width(Length::FillPortion(preview::LIST_PORTION)),
                self.file_preview_pane(preview, true),
            ]
            .into(),
            None => rows.into(),
        };
        match &page.notice {
            Some(notice) => column![filter, rows, self.notice(notice)].into(),
            None => column![filter, rows].into(),
        }
    }

    /// Search Files' list and the selected file's preview, for assistive
    /// technology.
    pub(super) fn files_semantics(
        &self,
        page: &crate::files_page::FilesPage,
    ) -> a11y::PageSemantics {
        let home =
            compass_core::xdg_dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
        let items = page
            .rows
            .iter()
            .map(|file| {
                a11y::item(
                    file.name.clone(),
                    Some(crate::files_page::subtitle(file, home.as_deref())),
                )
            })
            .collect();
        let label = if page.heading.is_empty() {
            "Files"
        } else {
            page.heading.as_str()
        };
        let notice = page.notice.as_deref().or(page.indicator());
        a11y::PageSemantics::list(label, items, page.selected, page.empty_notice())
            .with_status(notice, &page.query)
            .with_detail(
                page.preview
                    .as_ref()
                    .map(|preview| a11y::file_preview_document(preview, true)),
            )
    }

    /// Search Files' panel for the selected file, without the engine's
    /// answer about it: the panel itself waits for that answer before it
    /// opens, but a shortcut only needs to know which action it names.
    pub(super) fn files_panel_now(&self) -> Option<PanelState> {
        let Page::Files(page) = &self.page else {
            return None;
        };
        let path = &page.selected_row()?.path;
        let assumed = crate::backend::FileActions {
            mime: self.file_mime.clone(),
            has_opener: true,
            can_set_wallpaper: true,
            can_paste: true,
        };
        Some(PanelState::new(file_actions::file_panel_sections(
            path, &assumed,
        )))
    }

    /// Opens Search Files with `query` typed, and the remembered category
    /// (`restoreCategoryFilter`: anything but "All").
    pub(super) fn open_search_files(&mut self, query: String) -> Task<Message> {
        let mut page = crate::files_page::FilesPage::default();
        if let Some(key) = self.view.view_memory.get(crate::view_memory::FILE_CATEGORY) {
            page.set_category(key);
        }
        page.set_query(query);
        self.page = Page::Files(page);
        Task::batch([self.files_query_task(), focus_search()])
    }

    /// Asks Search Files' query: at once, or once its debounce runs out.
    fn files_query_task(&mut self) -> Task<Message> {
        let Page::Files(page) = &self.page else {
            return Task::none();
        };
        let generation = page.generation;
        match crate::files_page::debounce_for(&page.query) {
            Some(delay) => Task::perform(
                async move {
                    tokio::time::sleep(delay).await;
                    generation
                },
                Message::FilesDebounced,
            ),
            None => self.files_search_task(generation),
        }
    }

    /// Sends Search Files' query `generation`, unless the text moved on.
    fn files_search_task(&mut self, generation: u64) -> Task<Message> {
        let Page::Files(page) = &mut self.page else {
            return Task::none();
        };
        if page.generation != generation {
            return Task::none();
        }
        let Some(backend) = self.backend.clone() else {
            page.apply(
                generation,
                Err(
                    "Search Files needs the Compass engine, and this window is running \
                     without one"
                        .to_owned(),
                ),
            );
            return Task::none();
        };
        let query = page.query.clone();
        let category = page.category.clone();
        Task::perform(
            async move { backend.search_files(query, category).await },
            move |result| Message::FilesLoaded { generation, result },
        )
    }

    /// Opens the selected file with its default application, or shows it in
    /// the file browser when `reveal`.
    pub(super) fn open_selected_file(&mut self, reveal: bool) -> Task<Message> {
        let Page::Files(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(backend)) = (page.selected_row(), self.backend.clone()) else {
            return Task::none();
        };
        let path = row.path.clone();
        Task::perform(
            async move { backend.open_file(path, reveal).await },
            Message::FileOpened,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::flags::AppFlags;
    use crate::app::test_support::{
        TestBackend, chord, file_row, files_page, index, open_builtin, panel_titles, pressed,
        settle,
    };
    use crate::app::{Fallback, RootRow};
    use std::sync::Arc;

    fn files_app(dir: &std::path::Path) -> (LauncherApp, Arc<TestBackend>) {
        let backend = Arc::new(TestBackend {
            files: vec![
                file_row("/home/me/Documents/quarterly-report.pdf", "Documents"),
                file_row("/home/me/notes.md", "Documents"),
            ],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir));
        app.backend = Some(backend.clone());
        app.query = "search files".into();
        app.search();
        assert_eq!(
            app.selected_row(),
            Some(RootRow::Command(
                compass_core::commands::by_id("commands:search-files").unwrap()
            )),
            "{}",
            app.state_line()
        );
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        (app, backend)
    }

    #[test]
    fn search_files_lists_recent_files_then_searches_after_the_debounce() {
        // The debounce sleeps on tokio's timer, which the iced executor
        // provides in the launcher and this runtime provides here.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let _entered = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = files_app(dir.path());

        let page = files_page(&app);
        assert_eq!(page.heading, "Recently Accessed");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            backend.recorded.lock().unwrap().as_slice(),
            ["commands:search-files"]
        );
        assert!(
            app.state_line().contains("page=files"),
            "{}",
            app.state_line()
        );

        // Two keystrokes inside one debounce: only the second is asked.
        let first = app.update(Message::FilesQueryChanged("quart".into()));
        let second = app.update(Message::FilesQueryChanged("quarterly".into()));
        settle(&mut app, first);
        settle(&mut app, second);
        assert_eq!(
            backend.file_queries.lock().unwrap().as_slice(),
            ["", "quarterly"],
            "the superseded keystroke never reached the engine"
        );
        let page = files_page(&app);
        assert_eq!(page.heading, "Results");
        assert_eq!(
            page.selected_row().map(|row| row.name.as_str()),
            Some("quarterly-report.pdf")
        );

        let back = app.update(pressed(iced::keyboard::key::Named::Escape));
        drop(back);
        assert!(matches!(app.page, Page::Root), "Escape goes back");
    }

    #[test]
    fn enter_opens_the_file_and_ctrl_enter_shows_it_in_the_file_browser() {
        let enter = |modifiers| {
            Message::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
                modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
                physical_key: iced::keyboard::key::Physical::Unidentified(
                    iced::keyboard::key::NativeCode::Unidentified,
                ),
                location: iced::keyboard::Location::Standard,
                modifiers,
                text: None,
                repeat: false,
            })
        };
        let dir = tempfile::tempdir().unwrap();

        let (mut app, backend) = files_app(dir.path());
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(files_page(&app).selected, 1);
        let task = app.update(enter(iced::keyboard::Modifiers::default()));
        settle(&mut app, task);
        assert_eq!(
            backend.opened.lock().unwrap().as_slice(),
            [("/home/me/notes.md".to_owned(), false)]
        );
        assert!(matches!(app.page, Page::Root), "opening hides the launcher");

        let (mut app, backend) = files_app(dir.path());
        let task = app.update(enter(iced::keyboard::Modifiers::CTRL));
        settle(&mut app, task);
        assert_eq!(
            backend.opened.lock().unwrap().as_slice(),
            [("/home/me/Documents/quarterly-report.pdf".to_owned(), true)]
        );
    }

    #[test]
    fn search_files_filters_by_a_remembered_category_and_previews_the_selection() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, "remember the milk").unwrap();
        let picture = dir.path().join("cat.png");
        std::fs::write(&picture, b"png").unwrap();
        let backend = Arc::new(TestBackend {
            files: vec![
                file_row(&notes.to_string_lossy(), "Documents"),
                file_row(&picture.to_string_lossy(), "Images"),
            ],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.view.view_memory =
            crate::view_memory::ViewMemory::load(Some(dir.path().join("view-state.json")));
        open_builtin(&mut app, "search files", "commands:search-files");
        let page = files_page(&app);
        let preview = page.preview.as_ref().expect("the selection is previewed");
        assert_eq!(preview.name, "notes.txt");
        assert_eq!(
            preview.content,
            crate::file_preview::Content::Text("remember the milk".into())
        );
        assert!(preview.modified.is_some());
        let _ = app.view();

        let task = app.update(Message::FilesCategoryChanged("Images".into()));
        settle(&mut app, task);
        let page = files_page(&app);
        assert_eq!(page.rows.len(), 1);
        assert_eq!(
            page.preview.as_ref().map(|p| p.content.clone()),
            Some(crate::file_preview::Content::Image(picture.clone()))
        );
        assert_eq!(
            backend
                .file_queries
                .lock()
                .unwrap()
                .last()
                .map(String::as_str),
            Some(" [Images]")
        );

        // A new opening, even in a new process, starts filtered.
        let mut again = LauncherApp::with_index(index(dir.path()));
        again.backend = Some(backend.clone());
        again.view.view_memory =
            crate::view_memory::ViewMemory::load(Some(dir.path().join("view-state.json")));
        open_builtin(&mut again, "search files", "commands:search-files");
        assert_eq!(files_page(&again).category.as_deref(), Some("Images"));
    }

    #[test]
    fn a_query_offers_search_files_as_a_fallback_that_searches_for_it() {
        // The typed query waits out the indexer's debounce on a timer.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _entered = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            files: vec![file_row("/home/me/zebra-notes.md", "Documents")],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.apply(AppFlags {
            backend: Some(backend.clone()),
            fallbacks: vec!["files:search".into()],
            ..AppFlags::default()
        });
        app.query = "zebra".into();
        app.search();
        assert!(
            matches!(app.results.last(), Some(RootRow::Fallback(Fallback::Command(c)))
                if c.kind == compass_core::commands::CommandKind::SearchFiles),
            "{}",
            app.state_line()
        );
        let _ = app.view();
        app.selected = app.results.len() - 1;
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let page = files_page(&app);
        assert_eq!(page.query, "zebra");
        assert_eq!(page.rows.len(), 1);
        app.query.clear();
        app.search();
        assert!(app.results.is_empty(), "an empty query offers no fallback");
    }

    #[test]
    fn ctrl_b_opens_the_file_panel_and_its_shortcuts_work_with_it_closed() {
        use iced::keyboard::Modifiers;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            files: vec![file_row("/home/me/sunset.png", "Images")],
            file_info: crate::backend::FileActions {
                mime: Some("image/png".into()),
                has_opener: true,
                can_set_wallpaper: true,
                can_paste: true,
            },
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "search files", "commands:search-files");

        let task = app.update(chord("b", Modifiers::CTRL));
        settle(&mut app, task);
        assert!(
            panel_titles(&app).contains(&"Copy file path".to_owned()),
            "{}",
            app.state_line()
        );
        let task = app.update(chord("b", Modifiers::CTRL));
        settle(&mut app, task);
        assert!(app.panel.is_none(), "and Ctrl+B closes it again");

        // Copy file is Ctrl+Shift+C; it needs no panel.
        let task = app.update(chord("C", Modifiers::CTRL | Modifiers::SHIFT));
        settle(&mut app, task);
        assert_eq!(
            backend.file_calls.lock().unwrap().as_slice(),
            [("copy", "/home/me/sunset.png".to_owned())]
        );
        assert!(app.panel.is_none());
    }

    #[test]
    fn search_files_without_an_engine_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.query = "search files".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let page = files_page(&app);
        assert!(
            matches!(&page.status, crate::files_page::Status::Failed(reason)
                if reason.contains("needs the Compass engine")),
            "{:?}",
            page.status
        );
    }
}

#[cfg(test)]
mod semantics_tests {
    use super::*;
    use crate::backend::{FileResults, FileRow};
    use crate::semantics::Body;
    use compass_core::AppIndex;

    fn app() -> LauncherApp {
        LauncherApp::with_index(AppIndex::builder().build())
    }

    #[test]
    fn search_files_describes_its_rows_and_the_selected_files_preview() {
        let dir = tempfile::tempdir().expect("tempdir");
        let notes = dir.path().join("notes.md");
        std::fs::write(&notes, "# Title\n\nbody").expect("write notes");
        let mut app = app();
        let mut page = crate::files_page::FilesPage::default();
        assert_eq!(
            app.files_semantics(&page).status.as_deref(),
            Some("Searching files…")
        );
        let row = FileRow {
            path: notes.to_string_lossy().into_owned(),
            name: "notes.md".into(),
            category: "Documents".into(),
        };
        let generation = page.generation;
        assert!(page.apply(
            generation,
            Ok(FileResults {
                heading: "Recently Accessed".into(),
                files: vec![row.clone()],
            }),
        ));
        let semantics = app.files_semantics(&page);
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Recently Accessed");
        assert_eq!(list.items[0].name, "notes.md");
        let home =
            compass_core::xdg_dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
        assert_eq!(
            list.items[0].description,
            Some(crate::files_page::subtitle(&row, home.as_deref()))
        );
        assert_eq!(list.selected, Some(0));
        let detail = semantics.detail.expect("the preview");
        assert_eq!(detail.label, "notes.md");
        assert!(detail.blocks.iter().any(|block| block.text == "body"));
        assert!(
            detail
                .metadata
                .iter()
                .any(|entry| entry.label == "Name" && entry.value == "notes.md")
        );

        let generation = page.set_query("zzzz".into());
        assert!(page.apply(generation, Ok(FileResults::default())));
        app.page = Page::Files(page);
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(semantics.status.as_deref(), Some("No files found"));
    }
}
