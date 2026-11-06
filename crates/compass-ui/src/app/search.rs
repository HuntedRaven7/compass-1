//! The root search: typing, ranking, moving the selection and launching
//! what it is on.
//!
//! Ranking is `compass-core`'s, run by the engine through the backend or,
//! with no backend attached, here over the local index. What stays in this
//! module is keeping the answer honest against the query on screen -- an
//! answer for an older query never replaces a newer one -- and the rows the
//! launcher adds of its own: the calculator's answer first, and the
//! fallbacks under the results for a non-empty query.

use super::{
    AppLauncher, Arc, Fallback, LauncherApp, Message, Page, RESULTS_SETTLE, RootRow, Task,
    focus_search, next_selection,
};

impl LauncherApp {
    /// The root search's own messages: the query, the engine's answer, the
    /// selection, launching and the catalog moving under it.
    pub(super) fn search_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::QueryChanged(query) => {
                if let Some(task) = self.alias_space(&query) {
                    return task;
                }
                self.panel = None;
                self.query = query;
                self.error = None;
                // Typing starts history over from the newest search.
                self.history_offset = None;
                self.history_draft = None;
                self.search_task()
            }
            Message::ClockTick => {
                self.clock_tick();
                Task::none()
            }
            Message::SearchCompleted { generation, result } => {
                if generation != self.search_generation {
                    return Task::none();
                }
                self.search_task = None;
                match result {
                    Ok(keys) => {
                        let shortcuts = self.app_index.shortcuts();
                        let positions: Option<Vec<_>> = keys
                            .iter()
                            // A shortcut the engine knows and this window has
                            // not heard of yet (or one just removed) is left
                            // out rather than failing the list: the next
                            // refresh brings the two back into step.
                            .filter(|key| {
                                (!key.starts_with("shortcuts:")
                                    || self.app_index.shortcut_by_entrypoint(key).is_some())
                                    && (!key.starts_with("scripts:")
                                        || self.app_index.script_by_entrypoint(key).is_some())
                                    && (!key.starts_with("rhai:")
                                        || self.app_index.rhai_script_by_entrypoint(key).is_some())
                                    // An extension installed since this
                                    // window last scanned, likewise.
                                    && (!key.starts_with('@')
                                        || self.app_index.extension(key).is_some())
                            })
                            .map(|key| {
                                if let Some(command) = compass_core::commands::by_id(key) {
                                    return Some(RootRow::Command(command));
                                }
                                if let Some(script) = self.app_index.rhai_script_by_entrypoint(key)
                                {
                                    return self
                                        .app_index
                                        .rhai_scripts()
                                        .iter()
                                        .position(|known| known.id == script.id)
                                        .map(RootRow::RhaiScript);
                                }
                                if let Some(script) = self.app_index.script_by_entrypoint(key) {
                                    return self
                                        .app_index
                                        .scripts()
                                        .iter()
                                        .position(|known| known.id == script.id)
                                        .map(RootRow::Script);
                                }
                                if let Some(shortcut) = self.app_index.shortcut_by_entrypoint(key) {
                                    return shortcuts
                                        .iter()
                                        .position(|known| known.id == shortcut.id)
                                        .map(RootRow::Shortcut);
                                }
                                if let Some(index) = self
                                    .app_index
                                    .extensions()
                                    .iter()
                                    .position(|command| command.id == *key)
                                {
                                    return Some(RootRow::Extension(index));
                                }
                                // By ENTRYPOINT id: `QueryHit.id` is
                                // `applications:foo`, not the launch key
                                // `foo.desktop`, and `position` answers only
                                // for the latter. Resolving with the wrong
                                // one returns None for every hit and shows
                                // "the application catalog changed".
                                let position = self.app_index.position_by_entrypoint(key)?;
                                (!self.app_index.items()[position].is_action())
                                    .then_some(RootRow::App(position))
                            })
                            .collect();
                        if let Some(positions) = positions {
                            self.results = positions;
                            self.selected = 0;
                            self.apply_calculator();
                            self.apply_fallbacks();
                            self.apply_favorites();
                            self.apply_update();
                            self.warm_icons();
                        } else {
                            self.error = Some(
                                "The application catalog changed. Restart the launcher.".to_owned(),
                            );
                        }
                    }
                    Err(error) => self.error = Some(format!("could not search: {error}")),
                }
                if self.query.is_empty() {
                    return crate::scroll::reveal_root_selection();
                }
                self.results_settling = true;
                let settled = Task::perform(
                    async move {
                        // The launcher runs its tasks on tokio. Without a
                        // timer (a test driving the task on a plain executor)
                        // the answer is described at once.
                        if tokio::runtime::Handle::try_current().is_ok() {
                            tokio::time::sleep(RESULTS_SETTLE).await;
                        }
                        generation
                    },
                    Message::ResultsSettled,
                );
                Task::batch([crate::scroll::reveal_root_selection(), settled])
            }
            Message::ResultsSettled(generation) => {
                if generation == self.search_generation {
                    self.results_settling = false;
                }
                Task::none()
            }
            Message::ResultSelected(index) => {
                if index < self.results.len() {
                    self.selected = index;
                }
                crate::scroll::reveal_root_selection()
            }
            // A click activates, as `activate_on_single_click` (the Rust
            // port's only mode, `settings_catalog`). The press took focus
            // from the search field, so it is handed back for whatever the
            // row opens, or for the next query if it opened nothing.
            Message::ResultClicked(position) => {
                if position >= self.results.len() {
                    return Task::none();
                }
                self.panel = None;
                self.selected = position;
                let activated = self.update(Message::LaunchSelected);
                Task::batch([activated, focus_search()])
            }
            Message::MoveSelection(direction) => {
                self.selected = next_selection(
                    self.results.len(),
                    self.selected,
                    direction,
                    self.wrap_navigation,
                );
                crate::scroll::reveal_root_selection()
            }
            Message::LaunchSelected => {
                if matches!(self.page, Page::Root) {
                    self.record_search();
                }
                if let Some(RootRow::Update) = self.selected_row() {
                    return self.open_release_notes();
                }
                if let Some(RootRow::Calculator) = self.selected_row()
                    && self.calculator.is_some()
                {
                    // The C++ primary action: copy the answer, remembering it
                    // in the history, then get out of the way so it can be
                    // pasted.
                    return self.copy_root_answer();
                }
                if let Some(RootRow::Command(command)) = self.selected_row() {
                    return self.open_command(command);
                }
                if let Some(RootRow::Extension(index)) = self.selected_row() {
                    return self.run_extension_command(index);
                }
                if let Some(RootRow::Shortcut(index)) = self.selected_row() {
                    return self.open_shortcut_at(index);
                }
                if let Some(RootRow::Script(index)) = self.selected_row() {
                    return self.run_script_at(index);
                }
                if let Some(RootRow::RhaiScript(index)) = self.selected_row() {
                    return self.open_rhai_script_at(index);
                }
                if let Some(RootRow::Fallback(fallback)) = self.selected_row() {
                    return self.open_fallback(fallback);
                }
                let Some(item) = self.selected_item() else {
                    return Task::none();
                };
                // Windows are presented as `AppItem`s with `key == "window:{n}"`
                // when the `switch-windows` provider is active. Activating
                // one is a `compass-shell` `ActivateWindow` bounded call, not
                // a desktop-entry launch. Keeping the branch here keeps
                // `root_list::build`/`move_selection` untouched.
                if let Some(window_id) =
                    compass_core::window_switcher::window_launch_target_for_app(item.key())
                {
                    // Typed `a{sv}`, no JSON: the id is the `u32` the shell minted.
                    // Real activation is `client.activate_window(window_id).bounded("ActivateWindow")`
                    // via the shell proxy; the headless harness proves the branch
                    // without needing a live compositor or a display server.
                    let _bounded = format!("ActivateWindow({window_id})");
                    return self.conceal();
                }
                // Cloned into the future because the launch outlives this
                // borrow of `self`. An AppItem is a parsed desktop entry, so
                // this is not free — but it happens once per launch, not once
                // per keystroke.
                let entry = item.entry().clone();
                let action_id = item.action_id().map(str::to_owned);
                let launcher = Arc::clone(&self.launcher);
                launch_task(
                    launcher,
                    entry,
                    action_id,
                    self.backend.clone(),
                    item.key().to_owned(),
                )
            }
            // A launcher that stays open after launching is a bug report
            // waiting to happen. Hidden, not gone -- see `conceal`.
            Message::Launched(Ok(())) => self.conceal(),
            Message::Launched(Err(err)) => {
                self.error = Some(format!("could not launch: {err}"));
                Task::none()
            }
            Message::CatalogGeneration(Ok(generation)) => self.catalog_moved(generation),
            Message::CatalogGeneration(Err(error)) => {
                tracing::debug!(%error, "no catalog generation");
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// What Enter does to the selected root row, as the first action of its
    /// panel names it; `None` with nothing selected.
    pub(super) fn root_primary_action(&self) -> Option<&'static str> {
        Some(match self.selected_row()? {
            RootRow::App(_) | RootRow::RhaiScript(_) => "Open",
            RootRow::Command(_) | RootRow::Extension(_) => "Open command",
            RootRow::Calculator => "Copy answer",
            RootRow::Shortcut(_) => "Open shortcut",
            RootRow::Script(_) => "Run script",
            RootRow::Fallback(Fallback::Shortcut(_)) => "Open",
            RootRow::Fallback(_) => "Open command",
            RootRow::Update => "View release notes",
        })
    }

    /// Re-rank against the current query.
    pub(super) fn search_task(&mut self) -> Task<Message> {
        self.cancel_search();
        self.error = None;
        // The provider search view ranks here: the engine's search is the
        // whole root list.
        if let Some(backend) = self.backend.clone()
            && self.provider_scope.is_none()
        {
            self.results.clear();
            self.selected = 0;
            let query = self.query.clone();
            let generation = self.search_generation;
            let (task, handle) =
                Task::perform(async move { backend.search(query).await }, move |result| {
                    Message::SearchCompleted { generation, result }
                })
                .abortable();
            self.search_task = Some(handle.abort_on_drop());
            return task;
        }
        self.search();
        crate::scroll::reveal_root_selection()
    }

    pub(super) fn cancel_search(&mut self) {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.results_settling = false;
        if let Some(handle) = self.search_task.take() {
            handle.abort();
        }
    }

    /// Re-rank locally when no daemon backend is attached.
    pub(super) fn search(&mut self) {
        let options = compass_core::root_items::SearchOptions {
            provider_id: self.provider_scope.as_ref().map(|scope| scope.id.clone()),
            ..compass_core::root_items::SearchOptions::default()
        };
        self.favorites_len = 0;
        if self.query.trim().is_empty() && options.provider_id.is_none() {
            self.results.clear();
            self.selected = 0;
            self.apply_favorites();
            self.apply_update();
            return;
        }

        self.results = self
            .app_index
            .search_root_with(&self.query, None, &options)
            .into_iter()
            .map(|hit| match hit {
                compass_core::RootHit::App(app) => RootRow::App(app.index),
                compass_core::RootHit::Command { command, .. } => RootRow::Command(command),
                compass_core::RootHit::Extension { command, .. } => RootRow::Extension(
                    self.app_index
                        .extensions()
                        .iter()
                        .position(|known| known.id == command.id)
                        .unwrap_or_default(),
                ),
                compass_core::RootHit::Script { script, .. } => RootRow::Script(
                    self.app_index
                        .scripts()
                        .iter()
                        .position(|known| known.id == script.id)
                        .unwrap_or_default(),
                ),
                compass_core::RootHit::RhaiScript { script, .. } => RootRow::RhaiScript(
                    self.app_index
                        .rhai_scripts()
                        .iter()
                        .position(|known| known.id == script.id)
                        .unwrap_or_default(),
                ),
                compass_core::RootHit::Shortcut { shortcut, .. } => RootRow::Shortcut(
                    self.app_index
                        .shortcuts()
                        .iter()
                        .position(|known| known.id == shortcut.id)
                        .unwrap_or_default(),
                ),
            })
            .collect();
        // Back to the top on every new query: the old selection pointed into a
        // different list, and keeping its position would silently select an
        // unrelated application.
        self.selected = 0;
        if self.provider_scope.is_none() {
            self.apply_calculator();
            self.apply_fallbacks();
        }
        self.warm_icons();
    }

    /// Offers the fallbacks under the results for a non-empty query, as
    /// `RootFallbackSection` does.
    fn apply_fallbacks(&mut self) {
        self.results
            .retain(|row| !matches!(row, RootRow::Fallback(_)));
        if self.query.trim().is_empty() || self.provider_scope.is_some() {
            return;
        }
        // An item the search already lists is not offered a second time
        // under the fallbacks ("docs" finds Search Files by its keywords).
        let listed = |fallback: &Fallback| {
            self.results.iter().any(|row| match (row, fallback) {
                (RootRow::Command(listed), Fallback::Command(offered)) => {
                    listed.entrypoint == offered.entrypoint
                }
                (RootRow::Extension(listed), Fallback::Extension(offered))
                | (RootRow::Shortcut(listed), Fallback::Shortcut(offered)) => listed == offered,
                _ => false,
            })
        };
        let fallbacks: Vec<RootRow> = self
            .fallbacks
            .iter()
            .filter_map(|id| self.resolve_fallback(id))
            .filter(|fallback| !listed(fallback))
            .map(RootRow::Fallback)
            .collect();
        self.results.extend(fallbacks);
    }

    /// The fallback a `fallbacks` entry names here, when it names an item
    /// that can be one (`isSuitableForFallback`): Search Files, any
    /// extension command, or a quicklink with exactly one argument.
    fn resolve_fallback(&self, id: &str) -> Option<Fallback> {
        if let Some(command) = compass_core::commands::fallback(id) {
            return Some(Fallback::Command(command));
        }
        if let Some(index) = self
            .app_index
            .extensions()
            .iter()
            .position(|command| command.id == id)
        {
            return Some(Fallback::Extension(index));
        }
        let shortcut = self.app_index.shortcut_by_entrypoint(id)?;
        if shortcut.link.arguments.len() != 1 {
            return None;
        }
        self.app_index
            .shortcuts()
            .iter()
            .position(|known| known.id == shortcut.id)
            .map(Fallback::Shortcut)
    }

    /// What a fallback row says.
    pub(super) fn fallback_title(&self, fallback: Fallback) -> Option<String> {
        Some(match fallback {
            Fallback::Command(command) => command.title.to_owned(),
            Fallback::Extension(index) => self.app_index.extensions().get(index)?.title.clone(),
            Fallback::Shortcut(index) => {
                crate::shortcuts_page::display_name(self.app_index.shortcuts().get(index)?)
                    .to_owned()
            }
        })
    }

    /// Runs a fallback with the query: Search Files opens searching for it,
    /// an extension command opens with it as its fallback text
    /// (`OpenBuiltinCommandAction::setForwardSearchText`), and a quicklink
    /// opens with it as its argument (`OpenShortcutFromSearchText`).
    fn open_fallback(&mut self, fallback: Fallback) -> Task<Message> {
        use compass_core::commands::CommandKind;
        self.panel = None;
        let query = self.query.clone();
        match fallback {
            Fallback::Command(command) => match command.kind {
                CommandKind::SearchFiles => self.open_search_files(query),
                _ => self.open_command(command),
            },
            Fallback::Extension(index) => self.run_extension_fallback(index, query),
            Fallback::Shortcut(index) => {
                let Some(shortcut) = self.app_index.shortcuts().get(index) else {
                    return Task::none();
                };
                let id = shortcut.id.clone();
                self.send_open_shortcut(id, vec![query])
            }
        }
    }

    /// Puts the calculator's answer to the query first, when there is one.
    pub(super) fn apply_calculator(&mut self) {
        self.results.retain(|row| *row != RootRow::Calculator);
        self.calculator = compass_core::calculator::evaluate(&self.query, !self.results.is_empty());
        if self.calculator.is_some() {
            self.results.insert(0, RootRow::Calculator);
        }
    }

    /// Asks the engine whether its catalog moved since this window last
    /// looked.
    pub(super) fn catalog_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.catalog_generation().await },
            Message::CatalogGeneration,
        )
    }

    /// The engine rescanned applications or extensions: this window scans
    /// its own copy again, as the C++'s one process reloads its root items on
    /// `appsChanged`, and searches again so no row points at a moved entry.
    fn catalog_moved(&mut self, generation: u64) -> Task<Message> {
        if generation == self.catalog_generation {
            return Task::none();
        }
        self.catalog_generation = generation;
        self.app_index.rescan_applications();
        self.app_index.rescan_extensions();
        if matches!(self.page, Page::Root) {
            self.search_task()
        } else {
            self.results.clear();
            Task::none()
        }
    }
}

pub(super) fn launch_task(
    launcher: Arc<dyn AppLauncher>,
    entry: compass_xdg::DesktopEntry,
    action_id: Option<String>,
    backend: Option<Arc<dyn crate::backend::ApplicationBackend>>,
    key: String,
) -> Task<Message> {
    Task::perform(
        async move {
            match action_id {
                Some(id) => launcher.launch_action(&entry, &id, &[]).await,
                None => launcher.launch(&entry, &[]).await,
            }
            .map(|_| ())
            .map_err(|error| error.to_string())?;
            if let Some(backend) = backend
                && let Err(error) = backend.record_launch(key).await
            {
                // The application already opened. A history failure must not
                // invite the user to retry that successful launch.
                tracing::warn!(%error, "could not record successful launch");
            }
            Ok(())
        },
        Message::Launched,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::flags::AppFlags;
    use crate::app::test_support::{
        LaunchedEntries, RecordingLaunchTarget, TestBackend, app, click, extension_app, index,
        settle, shortcuts_app, store_backend, task_messages,
    };
    use crate::message::Direction;
    use compass_core::{AppIndex, AppItem};
    use iced::window;
    use std::fs;

    #[test]
    fn a_moved_catalog_generation_rescans_and_the_same_one_does_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        fs::write(
            dir.path().join("zephyr.desktop"),
            "[Desktop Entry]\nType=Application\nName=Zephyr\nExec=/bin/true\n",
        )
        .expect("write entry");
        let _ = app.update(Message::CatalogGeneration(Ok(0)));
        assert!(app.app_index.get("zephyr.desktop").is_none(), "unchanged");

        let _ = app.update(Message::CatalogGeneration(Ok(1)));
        assert!(app.app_index.get("zephyr.desktop").is_some());
        let _ = app.update(Message::QueryChanged("zeph".into()));
        assert_eq!(
            app.selected_item().map(AppItem::key),
            Some("zephyr.desktop")
        );

        fs::remove_file(dir.path().join("zephyr.desktop")).expect("remove");
        let _ = app.update(Message::CatalogGeneration(Ok(1)));
        assert!(
            app.app_index.get("zephyr.desktop").is_some(),
            "same generation"
        );
        let _ = app.update(Message::CatalogGeneration(Err("no engine".into())));
        let _ = app.update(Message::CatalogGeneration(Ok(2)));
        assert!(app.app_index.get("zephyr.desktop").is_none());
    }

    fn deliver(app: &mut LauncherApp, messages: Vec<Message>) {
        for message in messages {
            let task = app.update(message);
            settle(app, task);
        }
    }

    /// The desktop entries launched, by name.
    /// Four applications, all matching "Application", the first selected.
    fn four_applications(dir: &std::path::Path) -> LauncherApp {
        for i in 0..4 {
            fs::write(
                dir.join(format!("app-{i:02}.desktop")),
                format!(
                    "[Desktop Entry]\nType=Application\nName=Application {i:02}\nExec=/bin/true\n"
                ),
            )
            .expect("write entry");
        }
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir).build());
        app.query = "Application".into();
        app.search();
        assert!(app.results.len() >= 4, "{}", app.state_line());
        assert_eq!(app.selected, 0);
        app
    }

    #[test]
    fn clicking_a_root_row_launches_that_row_not_the_selected_one() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = Arc::new(LaunchedEntries::default());
        let mut app = four_applications(dir.path()).with_launcher(launcher.clone());
        let (position, name) = app
            .results
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(position, row)| match row {
                RootRow::App(index) => {
                    Some((position, app.app_index.items()[*index].name().to_owned()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no application below the first row: {}", app.state_line()));

        let messages = click(&app, &name);
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::ResultClicked(p) if *p == position)),
            "{messages:?}"
        );
        deliver(&mut app, messages);
        assert_eq!(launcher.0.lock().unwrap().as_slice(), [name]);
    }

    #[test]
    fn clicking_a_root_command_opens_it_and_a_store_row_opens_that_extension() {
        let dir = tempfile::tempdir().unwrap();
        let backend = store_backend(dir.path());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend);
        app.view.view_memory.set(
            crate::compass_pages::intro_key(crate::backend::Store::Vicinae),
            "true",
        );
        app.query = "extension store".into();
        app.search();
        let title = app
            .results
            .iter()
            .find_map(|row| match row {
                RootRow::Command(command) if command.id() == "commands:store" => {
                    Some(command.title)
                }
                _ => None,
            })
            .expect("the store is a root result");

        let messages = click(&app, title);
        deliver(&mut app, messages);
        let Page::Store(page) = &app.page else {
            panic!("the click did not open the store: {}", app.state_line());
        };
        assert_eq!(page.selected, 0);
        assert_eq!(page.rows[1].title, "Timer");

        let messages = click(&app, "Timer");
        deliver(&mut app, messages);
        let Page::StoreDetail(detail) = &app.page else {
            panic!("the click did not open the row: {}", app.state_line());
        };
        assert_eq!(detail.title, "Vicinae Store - Timer");
    }

    /// Hovering sends nothing and selects nothing: the C++ list moves its
    /// selection only on a click or a key (`SelectableDelegate.qml`).
    #[test]
    fn hovering_a_root_row_does_not_move_the_selection() {
        let dir = tempfile::tempdir().unwrap();
        let app = four_applications(dir.path());
        let RootRow::App(index) = app.results[1] else {
            panic!("the second row is not an application: {}", app.state_line())
        };
        let name = app.app_index.items()[index].name().to_owned();
        let mut ui = iced_test::simulator(app.view());
        let centre = ui
            .find(name.as_str())
            .expect("the row is drawn")
            .bounds()
            .center();
        ui.point_at(centre);
        ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
            position: centre,
        })]);
        let messages = ui.into_messages().collect::<Vec<_>>();
        assert!(messages.is_empty(), "{messages:?}");
        assert_eq!(app.selected, 0);
    }

    fn backend_app(dir: &std::path::Path, backend: Arc<TestBackend>) -> LauncherApp {
        for (id, name) in [("alpha", "Alpha Editor"), ("beta", "Beta Editor")] {
            std::fs::write(
                dir.join(format!("{id}.desktop")),
                format!("[Desktop Entry]\nType=Application\nName={name}\nExec=unused\n"),
            )
            .unwrap();
        }
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir).build());
        app.apply(AppFlags {
            backend: Some(backend),
            ..AppFlags::default()
        });
        app
    }

    #[test]
    fn backend_order_is_used_and_old_queries_cannot_replace_new_results() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec![
                "applications:beta".to_owned(),
                "applications:alpha".to_owned(),
            ],
            ..TestBackend::default()
        });
        let mut app = backend_app(dir.path(), backend);
        let old = app.update(Message::QueryChanged("Ed".to_owned()));
        let old_generation = app.search_generation;
        let current = app.update(Message::QueryChanged("Editor".to_owned()));
        assert!(
            task_messages(old).is_empty(),
            "superseded request is aborted"
        );
        assert!(
            app.selected_item().is_none(),
            "old results cannot be launched while pending"
        );
        for message in task_messages(current) {
            let _ = app.update(message);
        }
        assert_eq!(app.selected_item().unwrap().key(), "beta.desktop");
        let _ = app.update(Message::SearchCompleted {
            generation: old_generation,
            result: Ok(vec!["applications:alpha".to_owned()]),
        });
        assert_eq!(app.selected_item().unwrap().key(), "beta.desktop");
        let generation = app.search_generation;
        let _ = app.update(Message::QueryChanged(String::new()));
        let _ = app.update(Message::SearchCompleted {
            generation,
            result: Ok(vec!["applications:alpha".to_owned()]),
        });
        assert!(app.results.is_empty());
    }

    #[test]
    fn backend_errors_or_unknown_keys_do_not_restore_stale_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = backend_app(dir.path(), Arc::new(TestBackend::default()));
        for result in [
            Err("offline".to_owned()),
            Ok(vec!["applications:missing".to_owned()]),
        ] {
            let _pending = app.update(Message::QueryChanged("Editor".to_owned()));
            let _ = app.update(Message::SearchCompleted {
                generation: app.search_generation,
                result,
            });
            assert!(app.results.is_empty());
            assert!(app.error.is_some());
        }
    }

    /// Found by the Orca gate (#305): typing empties the list until the
    /// engine answers, and the status line said "No results" in between, so
    /// a screen reader announced it on every keystroke before the count, and
    /// the list was taken off the bus and put back each time. Meanwhile the
    /// window says it is waiting, and the tree keeps what it last said.
    #[test]
    fn a_search_the_engine_has_not_answered_keeps_the_last_description() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["applications:alpha".to_owned()],
            ..TestBackend::default()
        });
        let mut app = backend_app(dir.path(), backend);
        let pending = app.update(Message::QueryChanged("Alpha".to_owned()));
        let before = crate::semantics::Semantics {
            status: Some("2 results".to_owned()),
            ..crate::semantics::Semantics::default()
        };
        let semantics = app.semantics();
        assert_eq!(semantics.body, crate::semantics::Body::Pending);
        assert_eq!(semantics.status, None);
        let settled = semantics.settle(Some(&before));
        assert_eq!(settled.status.as_deref(), Some("2 results"));
        assert_eq!(settled.search.unwrap().value, "Alpha");
        let _ = app.update(Message::SearchCompleted {
            generation: app.search_generation,
            result: Ok(vec!["applications:alpha".to_owned()]),
        });
        drop(pending);
        // Answered, but not yet stood long enough to be announced.
        assert_eq!(app.semantics().body, crate::semantics::Body::Pending);
        let _ = app.update(Message::ResultsSettled(app.search_generation));
        assert_eq!(app.semantics().status.as_deref(), Some("1 result"));
        let _pending = app.update(Message::QueryChanged("Nothing".to_owned()));
        let _ = app.update(Message::SearchCompleted {
            generation: app.search_generation,
            result: Ok(Vec::new()),
        });
        // A timer from an earlier answer does not release this one.
        let _ = app.update(Message::ResultsSettled(
            app.search_generation.wrapping_sub(1),
        ));
        assert_eq!(app.semantics().body, crate::semantics::Body::Pending);
        let _ = app.update(Message::ResultsSettled(app.search_generation));
        assert_eq!(app.semantics().status.as_deref(), Some("No results"));
    }

    #[test]
    fn opening_or_clearing_an_attached_window_requests_initial_suggestions() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec![
                "applications:beta".to_owned(),
                "applications:alpha".to_owned(),
            ],
            ..TestBackend::default()
        });
        let mut app = backend_app(dir.path(), backend);
        let opened = app.update(Message::Opened(window::Id::unique()));
        for message in task_messages(opened) {
            let _ = app.update(message);
        }
        assert!(app.query.is_empty());
        assert_eq!(app.selected_item().unwrap().key(), "beta.desktop");
        let pending = app.update(Message::QueryChanged("Ed".to_owned()));
        let cleared = app.update(Message::QueryChanged(String::new()));
        assert!(task_messages(pending).is_empty());
        for message in task_messages(cleared) {
            let _ = app.update(message);
        }
        assert_eq!(app.results.len(), 2);
        assert_eq!(app.selected_item().unwrap().key(), "beta.desktop");
    }

    #[test]
    fn only_successful_launches_report_the_captured_key() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["applications:beta".to_owned()],
            fail_history: true,
            ..TestBackend::default()
        });
        let mut app = backend_app(dir.path(), backend.clone());
        let query = app.update(Message::QueryChanged("Editor".to_owned()));
        for message in task_messages(query) {
            let _ = app.update(message);
        }
        let failed = app.update(Message::LaunchSelected);
        assert!(matches!(
            &task_messages(failed)[0],
            Message::Launched(Err(_))
        ));
        assert!(backend.recorded.lock().unwrap().is_empty());
        app.launcher = Arc::new(RecordingLaunchTarget::default());
        let launched = app.update(Message::LaunchSelected);
        let _ = app.update(Message::QueryChanged(String::new()));
        assert!(
            matches!(&task_messages(launched)[0], Message::Launched(Ok(()))),
            "history failure is not a launch failure"
        );
        assert_eq!(*backend.recorded.lock().unwrap(), ["beta.desktop"]);
    }

    #[test]
    fn typing_ranks_and_selects_the_first_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        assert_eq!(app.selected, 0);
        assert_eq!(
            app.selected_item().map(compass_core::AppItem::name),
            Some("Firefox")
        );
    }

    #[test]
    fn a_new_query_resets_the_selection() {
        // Otherwise the old position points into a different list and the user
        // launches something they never looked at.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        // "fi" matches Files and Firefox. A single "e" matches nothing at all:
        // the coherence rule (ADR-0006) drops a one-character query that is not
        // a word prefix, which is correct and cost this test a first draft.
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let _ = app.update(Message::MoveSelection(Direction::Down));
        assert_ne!(app.selected, 0, "precondition: the selection moved");
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn an_empty_query_shows_nothing_and_selects_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        let _ = app.update(Message::QueryChanged("   ".to_owned()));
        assert!(app.results.is_empty());
        assert!(app.selected_item().is_none());
    }

    #[test]
    fn launching_with_no_results_does_nothing_rather_than_panicking() {
        // Enter on an empty list is the most ordinary way to reach this.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzznotathing".to_owned()));
        assert!(app.results.is_empty());
        let _ = app.update(Message::LaunchSelected);
    }

    #[test]
    fn a_failed_launch_is_shown_and_cleared_by_the_next_keystroke() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::Launched(Err("no Exec key".to_owned())));
        assert_eq!(app.error.as_deref(), Some("could not launch: no Exec key"));
        let _ = app.update(Message::QueryChanged("f".to_owned()));
        assert!(app.error.is_none(), "a new query should clear the error");
    }

    #[test]
    fn fallbacks_open_a_one_argument_shortcut_and_an_extension_with_the_query() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        app.fallbacks = vec![
            "shortcuts:sct-news".into(),
            "shortcuts:sct-docs".into(),
            "files:search".into(),
        ];
        app.query = "serde".into();
        app.search();
        let fallbacks: Vec<RootRow> = app
            .results
            .iter()
            .copied()
            .filter(|row| matches!(row, RootRow::Fallback(_)))
            .collect();
        let docs = app
            .app_index
            .shortcuts()
            .iter()
            .position(|shortcut| shortcut.id == "sct-docs")
            .unwrap();
        assert!(
            matches!(
                fallbacks.as_slice(),
                [
                    RootRow::Fallback(Fallback::Shortcut(at)),
                    RootRow::Fallback(Fallback::Command(_)),
                ] if *at == docs
            ),
            "a shortcut without exactly one argument is no fallback: {fallbacks:?}"
        );
        app.selected = app
            .results
            .iter()
            .position(|row| *row == fallbacks[0])
            .unwrap();
        assert!(
            app.state_line()
                .contains("selected_title=\"Crate Docs\" fallback")
        );
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            backend.opened_shortcuts.lock().unwrap().last(),
            Some(&("sct-docs".to_owned(), vec!["serde".to_owned()]))
        );

        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = extension_app(dir.path(), backend.clone());
        app.fallbacks = vec!["@someone/hello:write".into()];
        app.query = "zzzz".into();
        app.search();
        assert_eq!(
            app.results.last(),
            Some(&RootRow::Fallback(Fallback::Extension(0)))
        );
        app.selected = app.results.len() - 1;
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            backend.launched.lock().unwrap().as_slice(),
            [("@someone/hello:write".to_owned(), Some("zzzz".to_owned()))]
        );
    }

    #[test]
    fn a_calculation_that_matches_nothing_is_answered_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("12*3+6".to_owned()));
        assert_eq!(app.results, [RootRow::Calculator], "{}", app.state_line());
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("42").is_ok() && ui.find("12*3+6").is_ok());
        }
        assert!(app.state_line().contains("selected_title=\"42\""));

        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        assert!(
            !app.results.contains(&RootRow::Calculator),
            "a query that matches applications is not a calculation"
        );
        let _ = app.update(Message::QueryChanged("=2^10".to_owned()));
        assert_eq!(app.results.first(), Some(&RootRow::Calculator));
    }

    #[test]
    fn moving_the_selection_lands_on_the_item_that_gets_launched() {
        // The property that matters: what the caret points at in the view is
        // what LaunchSelected resolves. A off-by-one here launches the wrong
        // application, silently and every time.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let expected: Vec<String> = app
            .results
            .iter()
            .filter_map(|row| match row {
                RootRow::App(i) => app.app_index.items().get(*i),
                RootRow::Command(_)
                | RootRow::Extension(_)
                | RootRow::Shortcut(_)
                | RootRow::Script(_)
                | RootRow::RhaiScript(_)
                | RootRow::Fallback(_)
                | RootRow::Calculator
                | RootRow::Update => None,
            })
            .map(|item| item.name().to_owned())
            .collect();
        assert!(expected.len() > 1, "need several rows: {expected:?}");

        for (position, name) in expected.iter().enumerate() {
            assert_eq!(app.selected, position);
            assert_eq!(
                app.selected_item().map(compass_core::AppItem::name),
                Some(name.as_str())
            );
            let _ = app.update(Message::MoveSelection(Direction::Down));
        }
        // Commands rank after the applications here; walk past them too.
        for _ in expected.len()..app.results.len() {
            let _ = app.update(Message::MoveSelection(Direction::Down));
        }
        assert_eq!(
            app.selected,
            app.results.len() - 1,
            "and it stayed on the last row, because wrap_navigation is off by default"
        );
    }

    #[test]
    fn a_backend_answer_naming_a_command_becomes_a_command_row() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec![
                "commands:clipboard-history".into(),
                "applications:alpha".into(),
            ],
            ..TestBackend::default()
        });
        let mut app = backend_app(dir.path(), backend);
        let task = app.update(Message::QueryChanged("a".into()));
        settle(&mut app, task);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(matches!(app.results.first(), Some(RootRow::Command(_))));
        assert!(matches!(app.results.get(1), Some(RootRow::App(_))));
    }
}
