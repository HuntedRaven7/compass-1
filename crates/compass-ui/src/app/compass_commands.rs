//! Compass's own commands (the C++ `VicinaeExtension`): Configure
//! Fallback Commands, and the commands that open a link or a file or refresh
//! something and hide.
//!
//! A child module of `app` so it can reach the launcher's state; the fallback
//! manager's model is [`crate::fallbacks_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::a11y::{PageSemantics, item};
use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
};
use crate::action_panel::Action;
use crate::compass_pages::{ExtensionsPage, IconsPage, StoragePage, TokensPage, provenance_badge};
use crate::fallbacks_page::{Candidate, FallbacksPage};
use crate::semantics::{Block, Body, Document, markdown_blocks};
use compass_core::root_items::RootEdit;

/// The fallback manager's one action.
const FALLBACK_TOGGLE: &str = "fallback.toggle";
/// A fallback row's "Manage fallback actions" (`ManageFallbackActions`).
pub(super) const MANAGE_FALLBACKS: &str = "root.manage-fallbacks";
/// A fallback row's Open.
pub(super) const OPEN_FALLBACK: &str = "root.open-fallback";

/// Show Installed Extensions' actions.
const EXTENSION_UNINSTALL: &str = "extension.uninstall";
const EXTENSION_COPY_NAME: &str = "extension.copy-name";
const EXTENSION_COPY_ID: &str = "extension.copy-id";
const EXTENSION_COPY_PATH: &str = "extension.copy-path";
const EXTENSION_COPY_AUTHOR: &str = "extension.copy-author";
/// Search Builtin Icons' action.
const ICON_COPY_NAME: &str = "icon.copy-name";
/// Inspect Local Storage's actions.
const STORAGE_BROWSE: &str = "storage.browse";
const STORAGE_SHOW_VALUE: &str = "storage.show-value";
/// Manage OAuth Token Sets' actions.
const TOKEN_REMOVE: &str = "token.remove";
const TOKEN_COPY_ACCESS: &str = "token.copy-access";
const TOKEN_COPY_REFRESH: &str = "token.copy-refresh";
const TOKEN_COPY_ID: &str = "token.copy-id";
const TOKEN_COPY_SCOPES: &str = "token.copy-scopes";
const TOKEN_COPY_EXPIRY: &str = "token.copy-expiry";

/// The question before a token set is removed.
pub const TOKEN_REMOVE_MESSAGE: &str = "You will need to go through the OAuth login flow again \
                                        the next time you want to use this service";

/// What Reload Script Directories says (`ReloadScriptDirectoriesCommand`).
pub const SCRIPTS_RESCANNED: &str = "New scan triggered, index will update shortly";
/// What Refresh Apps says (`RefreshAppsCommand`).
pub const APPS_REFRESHED: &str = "Apps successfully refreshed";
/// What a link command's HUD says (`BuiltinUrlCommand`).
pub const OPENED_IN_BROWSER: &str = "Opened in browser";

impl LauncherApp {
    /// Runs one of Compass's own commands, by its C++ id.
    pub(super) fn open_compass_command(&mut self, id: &'static str) -> Task<Message> {
        match id {
            "manage-fallback" => self.open_manage_fallbacks(),
            "report-bug" => {
                let url = compass_core::bug_report::report_url(None, &system_info());
                self.open_link(url)
            }
            "sponsor" => self.open_link(compass_core::commands::SPONSOR_URL.to_owned()),
            "project-page" => self.open_link(compass_core::commands::PROJECT_URL.to_owned()),
            "open-config-file" => match self.config_path.clone() {
                Some(path) => {
                    let legacy = compass_core::config_migration::legacy_config_path().ok();
                    match compass_core::config::ensure_config_file(&path, legacy.as_deref()) {
                        Ok(_) => self.open_path_and_hide(path, false),
                        Err(error) => self.say_in_root(format!(
                            "Could not create the configuration file: {error}"
                        )),
                    }
                }
                None => self.say_in_root("No configuration file to open".to_owned()),
            },
            "open-default-config" => {
                let written = self
                    .default_config_dir
                    .as_deref()
                    .ok_or_else(|| "No place to write the default configuration".to_owned())
                    .and_then(write_default_config_in);
                match written {
                    Ok(path) => self.open_path_and_hide(path, false),
                    Err(reason) => self.say_in_root(reason),
                }
            }
            "show-logs" => match self.log_path.clone() {
                Some(path) if path.is_file() => self.open_path_and_hide(path, true),
                Some(_) => self.say_in_root(NO_LOG_YET.to_owned()),
                None => self.say_in_root("No log file to show".to_owned()),
            },
            "reload-scripts" => {
                let rescan = self.refresh_scripts_task();
                Task::batch([rescan, self.say_in_root(SCRIPTS_RESCANNED.to_owned())])
            }
            "refresh-apps" => {
                self.app_index.rescan_applications();
                let search = self.search_task();
                Task::batch([search, self.say_in_root(APPS_REFRESHED.to_owned())])
            }
            _ => self.open_compass_view(id),
        }
    }

    /// The root list, its query cleared, with `line` where it says things,
    /// as a C++ callback command's toast over the root search.
    fn say_in_root(&mut self, line: String) -> Task<Message> {
        self.page = Page::Root;
        self.query.clear();
        self.error = Some(line);
        focus_search()
    }

    /// Opens `url` in the browser and hides with "Opened in browser", as
    /// `BuiltinUrlCommand`.
    pub(super) fn open_link(&mut self, url: String) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return self.say_in_root("Opening a link needs the Compass engine".to_owned());
        };
        Task::perform(async move { backend.open_url(url).await }, |result| {
            Message::ActionDone(Some(crate::hud::Hud::new(OPENED_IN_BROWSER)), result)
        })
    }

    /// Opens `path` (or shows it in the file browser when `reveal`), then
    /// hides with the search text cleared, as `OpenVicinaeConfig`.
    fn open_path_and_hide(&mut self, path: std::path::PathBuf, reveal: bool) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return self.say_in_root("Opening a file needs the Compass engine".to_owned());
        };
        self.query.clear();
        let path = path.to_string_lossy().into_owned();
        Task::perform(
            async move { backend.open_file(path, reveal).await },
            |result| Message::ActionDone(None, result),
        )
    }

    /// Every item that can be a fallback, as `isSuitableForFallback`
    /// answers: Search Files, each extension command, and each quicklink
    /// with exactly one argument; each marked with the `fallbacks` entry
    /// that enables it.
    pub(super) fn fallback_candidates(&self) -> Vec<Candidate> {
        use compass_core::commands::{SEARCH_FILES_FALLBACK_ID, fallback};
        let enabled_as =
            |matches: &dyn Fn(&str) -> bool| self.fallbacks.iter().find(|id| matches(id)).cloned();
        let mut candidates = Vec::new();
        if let Some(files) = fallback(SEARCH_FILES_FALLBACK_ID) {
            candidates.push(Candidate {
                id: SEARCH_FILES_FALLBACK_ID.to_owned(),
                title: files.title.to_owned(),
                subtitle: files.subtitle.to_owned(),
                keywords: files.keywords.iter().map(|k| (*k).to_owned()).collect(),
                icon: Some(files.icon),
                enabled_as: enabled_as(&|id| fallback(id).is_some()),
            });
        }
        for command in self.app_index.extensions() {
            candidates.push(Candidate {
                id: command.id.clone(),
                title: command.title.clone(),
                subtitle: command.extension_title.clone(),
                keywords: command.keywords.clone(),
                icon: None,
                enabled_as: enabled_as(&|id| id == command.id),
            });
        }
        for shortcut in self.app_index.shortcuts() {
            if shortcut.link.arguments.len() != 1 {
                continue;
            }
            let id = compass_core::root_items::entrypoint_id(
                compass_core::shortcut::SHORTCUTS_PROVIDER_ID,
                &shortcut.id,
            );
            candidates.push(Candidate {
                title: crate::shortcuts_page::display_name(shortcut).to_owned(),
                subtitle: "Shortcut".to_owned(),
                keywords: Vec::new(),
                icon: None,
                enabled_as: enabled_as(&|known| known == id),
                id,
            });
        }
        candidates
    }

    /// Opens Configure Fallback Commands.
    pub(super) fn open_manage_fallbacks(&mut self) -> Task<Message> {
        self.panel = None;
        self.page = Page::Fallbacks(FallbacksPage::new(
            self.fallback_candidates(),
            self.fallbacks.clone(),
        ));
        focus_search()
    }

    /// Enables or disables the selected item, in this window at once and in
    /// the configuration through the engine, then lists again.
    fn toggle_selected_fallback(&mut self) -> Task<Message> {
        let Page::Fallbacks(page) = &self.page else {
            return Task::none();
        };
        let Some((_, id, enable)) = page.selected_action() else {
            return Task::none();
        };
        self.panel = None;
        compass_core::root_items::set_fallback(&mut self.fallbacks, &id, enable);
        let candidates = self.fallback_candidates();
        let order = self.fallbacks.clone();
        if let Page::Fallbacks(page) = &mut self.page {
            page.notice = None;
            page.reload(candidates, order);
        }
        match self.backend.clone() {
            Some(backend) => Task::batch([
                Task::perform(
                    async move { backend.edit_root_item(id, RootEdit::Fallback(enable)).await },
                    Message::RootItemEdited,
                ),
                focus_search(),
            ]),
            None => focus_search(),
        }
    }

    /// A fallback row's panel in root search (`fallbackActionPanel`): Open
    /// and Manage Fallback Actions.
    pub(super) fn open_fallback_row_panel(&mut self) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root) {
            return None;
        }
        let super::RootRow::Fallback(fallback) = self.selected_row()? else {
            return None;
        };
        let open = match fallback {
            super::Fallback::Shortcut(_) => "Open",
            _ => "Open command",
        };
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new(open)
                    .with_id(OPEN_FALLBACK)
                    .with_shortcut("enter"),
                Action::new("Manage fallback actions")
                    .with_id(MANAGE_FALLBACKS)
                    .with_shortcut("ctrl+enter"),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs one of this module's panel actions, if `id` is one.
    pub(super) fn compass_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        match id {
            OPEN_FALLBACK => {
                self.panel = None;
                Some(self.update(Message::LaunchSelected))
            }
            MANAGE_FALLBACKS => Some(self.open_manage_fallbacks()),
            ICON_COPY_NAME => {
                let Page::Icons(page) = &self.page else {
                    return None;
                };
                let name = page.selected_name()?.to_owned();
                self.panel = None;
                Some(self.copy_with_hud(name))
            }
            EXTENSION_UNINSTALL
            | EXTENSION_COPY_NAME
            | EXTENSION_COPY_ID
            | EXTENSION_COPY_PATH
            | EXTENSION_COPY_AUTHOR => {
                let Page::Extensions(page) = &self.page else {
                    return None;
                };
                let extension = page.selected_extension()?;
                let text = match id {
                    EXTENSION_UNINSTALL => {
                        let id = extension.id.clone();
                        self.panel = None;
                        return Some(self.ask_to_uninstall(id));
                    }
                    EXTENSION_COPY_NAME => extension.name.clone(),
                    EXTENSION_COPY_ID => extension.id.clone(),
                    EXTENSION_COPY_PATH => extension.path.to_string_lossy().into_owned(),
                    _ => extension.author.clone(),
                };
                self.panel = None;
                Some(self.copy_with_hud(text))
            }
            STORAGE_BROWSE => {
                self.panel = None;
                Some(self.browse_selected_namespace())
            }
            STORAGE_SHOW_VALUE => {
                let Page::Storage(page) = &mut self.page else {
                    return None;
                };
                let value = page.selected_item()?.value.clone();
                page.notice = Some(value);
                self.panel = None;
                Some(focus_search())
            }
            TOKEN_REMOVE | TOKEN_COPY_ACCESS | TOKEN_COPY_REFRESH | TOKEN_COPY_ID
            | TOKEN_COPY_SCOPES | TOKEN_COPY_EXPIRY => {
                let Page::Tokens(page) = &self.page else {
                    return None;
                };
                let set = page.selected_set()?;
                let text = match id {
                    TOKEN_REMOVE => {
                        let (extension, provider) =
                            (set.extension_id.clone(), set.provider_id.clone());
                        self.panel = None;
                        self.confirm = Some(super::Confirm {
                            title: "Are you sure?".to_owned(),
                            message: TOKEN_REMOVE_MESSAGE.to_owned(),
                            confirm_text: "Remove".to_owned(),
                            action: super::ConfirmAction::RemoveTokenSet(extension, provider),
                        });
                        return Some(Task::none());
                    }
                    TOKEN_COPY_ACCESS => set.access_token.clone(),
                    TOKEN_COPY_REFRESH => set.refresh_token.clone()?,
                    TOKEN_COPY_ID => set.id_token.clone()?,
                    TOKEN_COPY_SCOPES => set.scope.clone()?,
                    _ => expiry_text(set.expires_at?)?,
                };
                self.panel = None;
                Some(self.copy_with_hud(text))
            }
            _ => self.fallbacks_panel_action(id),
        }
    }

    /// Asks the engine for the namespaces or the token sets.
    fn load_storage_view(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::done(match &self.page {
                Page::Tokens(_) => {
                    Message::TokenSetsLoaded(Err(crate::backend::STORAGE_NEEDS_ENGINE.to_owned()))
                }
                _ => Message::StorageNamespacesLoaded(Err(
                    crate::backend::STORAGE_NEEDS_ENGINE.to_owned()
                )),
            });
        };
        match &self.page {
            Page::Storage(_) => Task::perform(
                async move { backend.local_storage_namespaces().await },
                Message::StorageNamespacesLoaded,
            ),
            Page::Tokens(_) => Task::perform(
                async move { backend.oauth_token_sets().await },
                Message::TokenSetsLoaded,
            ),
            _ => Task::none(),
        }
    }

    /// "Browse namespace": asks for the selected namespace's items.
    fn browse_selected_namespace(&mut self) -> Task<Message> {
        let Page::Storage(page) = &self.page else {
            return Task::none();
        };
        let Some(namespace) = page.selected_namespace().map(str::to_owned) else {
            return Task::none();
        };
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            {
                let namespace = namespace.clone();
                async move { backend.local_storage_items(namespace).await }
            },
            move |result| Message::StorageItemsLoaded {
                namespace: namespace.clone(),
                result,
            },
        )
    }

    /// Removes a token set through the engine, after the question.
    pub(super) fn remove_token_set(
        &mut self,
        extension_id: String,
        provider_id: Option<String>,
    ) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                backend
                    .remove_oauth_token_set(extension_id, provider_id)
                    .await
            },
            Message::TokenSetRemoved,
        )
    }

    /// Opens a store, on its intro the first time or whenever its
    /// `alwaysShowIntro` preference asks (`VicinaeStoreCommand`,
    /// `RaycastStoreCommand`).
    pub(super) fn open_store_or_intro(&mut self, store: crate::backend::Store) -> Task<Message> {
        let entrypoint = match store {
            crate::backend::Store::Vicinae => "store",
            crate::backend::Store::Raycast => "raycast-store",
        };
        let always = self
            .config_path
            .as_deref()
            .and_then(|path| compass_core::Config::load_from(path).ok())
            .and_then(|config| {
                config
                    .entrypoint_preferences(
                        compass_core::commands::COMMANDS_PROVIDER_ID,
                        entrypoint,
                    )?
                    .get("alwaysShowIntro")?
                    .as_bool()
            })
            .unwrap_or(false);
        let completed = self
            .view
            .view_memory
            .get(crate::compass_pages::intro_key(store));
        if crate::compass_pages::shows_intro(always, completed) {
            self.panel = None;
            self.page = Page::StoreIntro(crate::compass_pages::StoreIntroPage::new(store));
            return focus_search();
        }
        self.open_store(store)
    }

    /// "Continue to store": remembers the intro was seen and opens the store.
    pub(super) fn continue_to_store(&mut self) -> Task<Message> {
        let Page::StoreIntro(page) = &self.page else {
            return Task::none();
        };
        let store = page.store;
        self.panel = None;
        self.view
            .view_memory
            .set(crate::compass_pages::intro_key(store), "true");
        self.open_store(store)
    }

    /// The intro's body: its Markdown, then how to go on.
    pub(super) fn store_intro_body<'a>(
        &'a self,
        page: &'a crate::compass_pages::StoreIntroPage,
    ) -> Element<'a, Message> {
        let markdown = iced::widget::markdown::view(&page.markdown, self.markdown_settings())
            .map(Message::ExtensionLinkClicked);
        let hint = iced::widget::text(crate::compass_pages::intro_hint())
            .font(self.font())
            .size(12);
        scrollable(container(column![markdown, hint].spacing(10)).padding(Padding::new(14.0)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }

    /// The intro for assistive technology: its text, then how to go on.
    pub(super) fn store_intro_semantics(
        &self,
        page: &crate::compass_pages::StoreIntroPage,
    ) -> PageSemantics {
        let mut blocks = markdown_blocks(crate::compass_pages::intro_text(page.store));
        blocks.push(Block {
            text: crate::compass_pages::intro_hint(),
            heading: None,
        });
        PageSemantics {
            body: Body::Document(Document {
                label: crate::compass_pages::store_name(page.store).to_owned(),
                blocks,
                metadata: Vec::new(),
            }),
            status: None,
            detail: None,
        }
    }

    /// Opens one of the inspection views, by its C++ id.
    fn open_compass_view(&mut self, id: &'static str) -> Task<Message> {
        self.panel = None;
        match id {
            "list-extensions" => {
                self.page = Page::Extensions(ExtensionsPage::new(self.installed_extensions()));
            }
            "search-builtin-icons" => self.page = Page::Icons(IconsPage::new()),
            "inspect-local-storage" => {
                self.page = Page::Storage(StoragePage::default());
                return Task::batch([self.load_storage_view(), focus_search()]);
            }
            "oauth-token-store" => {
                self.page = Page::Tokens(TokensPage::default());
                return Task::batch([self.load_storage_view(), focus_search()]);
            }
            _ => return Task::none(),
        }
        focus_search()
    }

    /// Every installed extension's manifest, read from the directories root
    /// search found commands in (`ExtensionRegistry::scanAll`).
    fn installed_extensions(&self) -> Vec<compass_core::manifest::ExtensionManifest> {
        let mut directories: Vec<&std::path::Path> = self
            .app_index
            .extensions()
            .iter()
            .map(|command| command.extension_dir.as_path())
            .collect();
        directories.sort_unstable();
        directories.dedup();
        directories
            .into_iter()
            .filter_map(|dir| compass_core::manifest::ExtensionManifest::from_directory(dir).ok())
            .collect()
    }

    /// `UninstallExtensionAction`: asks, then uninstalls through the engine.
    fn ask_to_uninstall(&mut self, id: String) -> Task<Message> {
        self.confirm = Some(super::Confirm {
            title: crate::store_page::CONFIRM_TITLE.to_owned(),
            message: crate::store_page::CONFIRM_MESSAGE.to_owned(),
            confirm_text: "Uninstall".to_owned(),
            action: super::ConfirmAction::UninstallExtension(id),
        });
        Task::none()
    }

    /// Uninstalls extension `id` through the engine.
    pub(super) fn uninstall_extension(&mut self, id: String) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            if let Page::Extensions(page) = &mut self.page {
                page.notice = Some("Uninstalling needs the Compass engine".to_owned());
            }
            return Task::none();
        };
        Task::perform(
            {
                let id = id.clone();
                async move { backend.store_uninstall(id).await }
            },
            move |result| Message::ExtensionUninstalled {
                id: id.clone(),
                result,
            },
        )
    }

    /// The panel over the selected row of an inspection view.
    pub(super) fn open_compass_view_panel(&mut self) -> Option<Task<Message>> {
        let sections = match &self.page {
            Page::Icons(page) => {
                page.selected_name()?;
                vec![PanelSection {
                    name: String::new(),
                    actions: vec![
                        Action::new("Copy icon name")
                            .with_id(ICON_COPY_NAME)
                            .with_shortcut("enter"),
                    ],
                }]
            }
            Page::Storage(page) => {
                let (title, id) = if page.selected_item().is_some() {
                    ("Show value", STORAGE_SHOW_VALUE)
                } else {
                    page.selected_namespace()?;
                    ("Browse namespace", STORAGE_BROWSE)
                };
                vec![PanelSection {
                    name: String::new(),
                    actions: vec![Action::new(title).with_id(id).with_shortcut("enter")],
                }]
            }
            Page::Tokens(page) => {
                let set = page.selected_set()?;
                let mut copies = vec![Action::new("Copy access token").with_id(TOKEN_COPY_ACCESS)];
                if set.refresh_token.is_some() {
                    copies.push(Action::new("Copy refresh token").with_id(TOKEN_COPY_REFRESH));
                }
                if set.id_token.is_some() {
                    copies.push(Action::new("Copy ID token").with_id(TOKEN_COPY_ID));
                }
                if set.scope.is_some() {
                    copies.push(Action::new("Copy scopes").with_id(TOKEN_COPY_SCOPES));
                }
                if set.expires_at.is_some() {
                    copies.push(Action::new("Copy expiration date").with_id(TOKEN_COPY_EXPIRY));
                }
                vec![
                    PanelSection {
                        name: String::new(),
                        actions: vec![
                            Action::new("Remove token set")
                                .with_id(TOKEN_REMOVE)
                                .with_shortcut("enter"),
                        ],
                    },
                    PanelSection {
                        name: "Copy".to_owned(),
                        actions: copies,
                    },
                ]
            }
            Page::Extensions(page) => {
                let extension = page.selected_extension()?;
                let mut copies = vec![
                    Action::new("Copy name").with_id(EXTENSION_COPY_NAME),
                    Action::new("Copy ID").with_id(EXTENSION_COPY_ID),
                    Action::new("Copy path").with_id(EXTENSION_COPY_PATH),
                ];
                if !extension.author.is_empty() {
                    copies.push(Action::new("Copy author").with_id(EXTENSION_COPY_AUTHOR));
                }
                vec![
                    PanelSection {
                        name: String::new(),
                        actions: vec![
                            Action::new("Uninstall")
                                .with_id(EXTENSION_UNINSTALL)
                                .with_shortcut("enter"),
                        ],
                    },
                    PanelSection {
                        name: "Copy".to_owned(),
                        actions: copies,
                    },
                ]
            }
            _ => return None,
        };
        self.panel = Some(PanelState::new(sections));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// The inspection views' keys: the arrows, Escape back, Enter for the
    /// first action.
    pub(super) fn compass_view_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        if key.as_ref() == Key::Named(Named::Escape)
            && let Page::Storage(page) = &mut self.page
            && page.back()
        {
            return focus_search();
        }
        let (len, selected) = match &mut self.page {
            Page::Icons(page) => (page.shown.len(), &mut page.selected),
            Page::Extensions(page) => (page.shown.len(), &mut page.selected),
            Page::Storage(page) => (page.shown.len(), &mut page.selected),
            Page::Tokens(page) => (page.shown.len(), &mut page.selected),
            _ => return Task::none(),
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.compass_view_primary(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            *selected = next_selection(len, *selected, direction, self.wrap_navigation);
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Enter on an inspection view: its panel's first action.
    fn compass_view_primary(&mut self) -> Task<Message> {
        let id = match &self.page {
            Page::Icons(_) => ICON_COPY_NAME,
            Page::Extensions(_) => EXTENSION_UNINSTALL,
            Page::Storage(page) if page.browsing.is_some() => STORAGE_SHOW_VALUE,
            Page::Storage(_) => STORAGE_BROWSE,
            Page::Tokens(_) => TOKEN_REMOVE,
            _ => return Task::none(),
        };
        self.compass_panel_action(id).unwrap_or_else(Task::none)
    }

    /// Handles the inspection views' messages.
    pub(super) fn compass_view_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ExtensionsQueryChanged(query) => {
                if let Page::Extensions(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::IconsQueryChanged(query) => {
                if let Page::Icons(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::CompassRowSelected(position) => {
                match &mut self.page {
                    Page::Icons(page) if position < page.shown.len() => page.selected = position,
                    Page::Extensions(page) if position < page.shown.len() => {
                        page.selected = position;
                    }
                    Page::Storage(page) if position < page.shown.len() => {
                        page.selected = position;
                    }
                    Page::Tokens(page) if position < page.shown.len() => page.selected = position,
                    _ => return Task::none(),
                }
                self.compass_view_primary()
            }
            Message::StorageQueryChanged(query) => {
                if let Page::Storage(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::TokensQueryChanged(query) => {
                if let Page::Tokens(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::StorageNamespacesLoaded(result) => {
                if let Page::Storage(page) = &mut self.page {
                    match result {
                        Ok(namespaces) => page.set_namespaces(namespaces),
                        Err(reason) => page.notice = Some(reason),
                    }
                }
                Task::none()
            }
            Message::StorageItemsLoaded { namespace, result } => {
                if let Page::Storage(page) = &mut self.page
                    && page.selected_namespace() == Some(namespace.as_str())
                {
                    match result {
                        Ok(items) => page.browse(namespace, items),
                        Err(reason) => page.notice = Some(reason),
                    }
                }
                focus_search()
            }
            Message::TokenSetsLoaded(result) => {
                if let Page::Tokens(page) = &mut self.page {
                    match result {
                        Ok(sets) => page.set_sets(sets),
                        Err(reason) => page.notice = Some(reason),
                    }
                }
                Task::none()
            }
            Message::TokenSetRemoved(result) => {
                let Page::Tokens(page) = &mut self.page else {
                    return Task::none();
                };
                match result {
                    Ok(()) => {
                        page.notice = Some("Token set removed".to_owned());
                        self.load_storage_view()
                    }
                    Err(reason) => {
                        page.notice = Some(reason);
                        Task::none()
                    }
                }
            }
            Message::ExtensionUninstalled { id, result } => {
                if let Page::Extensions(page) = &mut self.page {
                    match result {
                        Ok(()) => {
                            page.remove(&id);
                            page.notice = Some(crate::store_page::UNINSTALLED.to_owned());
                        }
                        Err(reason) => {
                            page.notice = Some(format!("Failed to uninstall extension: {reason}"));
                        }
                    }
                }
                self.catalog_task()
            }
            _ => Task::none(),
        }
    }

    /// Show Installed Extensions' and Search Builtin Icons' bodies.
    pub(super) fn compass_view_body(&self) -> Option<Element<'_, Message>> {
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        let notice = match &self.page {
            Page::Icons(page) => {
                if let Some(empty) = page.empty_notice() {
                    return Some(self.notice(empty));
                }
                let names = compass_core::builtin_icon::names();
                for (position, &index) in page.shown.iter().enumerate() {
                    let Some(name) = names.get(index) else {
                        continue;
                    };
                    let selected = position == page.selected;
                    let glyph = crate::icons::Glyph::builtin(*name);
                    let icon = self.glyph_or_initial(Some(&glyph), name, selected);
                    list = list.push(self.compass_row(
                        self.list_row(icon, (*name).to_owned(), None, selected),
                        position,
                        selected,
                    ));
                }
                None
            }
            Page::Storage(page) => {
                if let Some(empty) = page.empty_notice() {
                    return Some(self.notice(empty));
                }
                for (position, title) in page.titles().into_iter().enumerate() {
                    let selected = position == page.selected;
                    let glyph = crate::icons::Glyph::builtin("coin");
                    let icon = self.glyph_or_initial(Some(&glyph), title, selected);
                    list = list.push(self.compass_row(
                        self.list_row(icon, title.to_owned(), None, selected),
                        position,
                        selected,
                    ));
                }
                page.notice.as_deref()
            }
            Page::Tokens(page) => {
                if let Some(empty) = page.empty_notice() {
                    return Some(self.notice(empty));
                }
                for (position, &index) in page.shown.iter().enumerate() {
                    let Some(set) = page.sets.get(index) else {
                        continue;
                    };
                    let selected = position == page.selected;
                    let glyph = crate::icons::Glyph::builtin("key");
                    let icon = self.glyph_or_initial(Some(&glyph), &set.extension_id, selected);
                    let row = self.list_row_with(
                        icon,
                        set.extension_id.clone(),
                        self.subtitles
                            .then(|| set.provider_id.clone().unwrap_or_default()),
                        set.expired.then(|| "Expired".to_owned()),
                        selected,
                    );
                    list = list.push(self.compass_row(row, position, selected));
                }
                page.notice.as_deref()
            }
            Page::Extensions(page) => {
                if let Some(empty) = page.empty_notice() {
                    return Some(self.notice(empty));
                }
                for (position, &index) in page.shown.iter().enumerate() {
                    let Some(extension) = page.all.get(index) else {
                        continue;
                    };
                    let selected = position == page.selected;
                    let icon = self.initial_badge(&extension.title, selected);
                    let row = self.list_row_with(
                        icon,
                        extension.title.clone(),
                        self.subtitles.then(|| extension.description.clone()),
                        Some(provenance_badge(extension.provenance).to_owned()),
                        selected,
                    );
                    list = list.push(self.compass_row(row, position, selected));
                }
                page.notice.as_deref()
            }
            _ => return None,
        };
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        Some(match notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        })
    }

    /// Show Installed Extensions for assistive technology: each row's title,
    /// then its description and where it came from.
    pub(super) fn extensions_semantics(&self, page: &ExtensionsPage) -> PageSemantics {
        let items = page
            .shown
            .iter()
            .map(|&index| {
                page.all
                    .get(index)
                    .map(|extension| {
                        item(
                            extension.title.clone(),
                            Some(joined(&[
                                extension.description.as_str(),
                                provenance_badge(extension.provenance),
                            ])),
                        )
                    })
                    .unwrap_or_default()
            })
            .collect();
        PageSemantics::list("Extensions", items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }

    /// Search Builtin Icons for assistive technology: the icons' names.
    pub(super) fn icons_semantics(&self, page: &IconsPage) -> PageSemantics {
        let names = compass_core::builtin_icon::names();
        let items = page
            .shown
            .iter()
            .map(|&index| {
                names
                    .get(index)
                    .map(|name| item(*name, None))
                    .unwrap_or_default()
            })
            .collect();
        PageSemantics::list("Icons", items, page.selected, page.empty_notice())
            .with_status(None, &page.query)
    }

    /// Inspect Local Storage for assistive technology: the namespaces, or
    /// the keys of the one being browsed.
    pub(super) fn storage_semantics(&self, page: &StoragePage) -> PageSemantics {
        let label = if page.browsing.is_some() {
            "Items"
        } else {
            "Namespaces"
        };
        let items = page
            .titles()
            .into_iter()
            .map(|title| item(title, None))
            .collect();
        PageSemantics::list(label, items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }

    /// Manage OAuth Token Sets for assistive technology: each set's
    /// extension, then its provider and whether it has expired.
    pub(super) fn tokens_semantics(&self, page: &TokensPage) -> PageSemantics {
        let items = page
            .shown
            .iter()
            .map(|&index| {
                page.sets
                    .get(index)
                    .map(|set| {
                        item(
                            set.extension_id.clone(),
                            Some(joined(&[
                                set.provider_id.as_deref().unwrap_or_default(),
                                if set.expired { "Expired" } else { "" },
                            ])),
                        )
                    })
                    .unwrap_or_default()
            })
            .collect();
        PageSemantics::list("Token sets", items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }

    /// A row of an inspection view: clickable, and marked for scrolling
    /// when selected.
    fn compass_row<'a>(
        &'a self,
        row: Element<'a, Message>,
        position: usize,
        selected: bool,
    ) -> Element<'a, Message> {
        let row: Element<Message> = mouse_area(row)
            .on_press(Message::CompassRowSelected(position))
            .into();
        if selected {
            container(row).id(crate::scroll::ROOT_SELECTION).into()
        } else {
            row
        }
    }

    /// The panel over the selected item: its one action.
    pub(super) fn open_fallbacks_panel(&mut self) -> Option<Task<Message>> {
        let Page::Fallbacks(page) = &self.page else {
            return None;
        };
        let (label, _, _) = page.selected_action()?;
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new(label)
                    .with_id(FALLBACK_TOGGLE)
                    .with_shortcut("enter"),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a fallback manager panel action, if `id` is one.
    fn fallbacks_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        (id == FALLBACK_TOGGLE && matches!(self.page, Page::Fallbacks(_)))
            .then(|| self.toggle_selected_fallback())
    }

    /// The view's keys.
    pub(super) fn fallbacks_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Fallbacks(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.toggle_selected_fallback(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.rows.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Handles the fallback manager's messages.
    pub(super) fn fallbacks_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FallbacksQueryChanged(query) => {
                if let Page::Fallbacks(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::FallbackSelected(position) => {
                if let Page::Fallbacks(page) = &mut self.page
                    && position < page.rows.len()
                {
                    page.selected = position;
                }
                self.toggle_selected_fallback()
            }
            _ => Task::none(),
        }
    }

    /// The fallback manager's body: "Enabled", then "Available".
    pub(super) fn fallbacks_body<'a>(&'a self, page: &'a FallbacksPage) -> Element<'a, Message> {
        if let Some(empty) = page.empty_notice() {
            return self.notice(empty);
        }
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, row) in page.rows.iter().enumerate() {
            let Some(candidate) = page.candidates.get(row.candidate) else {
                continue;
            };
            if let Some(heading) = page.heading_at(position) {
                list = list.push(self.section_heading(heading.to_owned()));
            }
            let selected = position == page.selected;
            let icon = match compass_core::commands::fallback(&candidate.id) {
                Some(command) => self.command_icon(command, selected),
                None => self.initial_badge(&candidate.title, selected),
            };
            let line = self.list_row(
                icon,
                candidate.title.clone(),
                self.subtitles.then(|| candidate.subtitle.clone()),
                selected,
            );
            let line: Element<Message> = mouse_area(line)
                .on_press(Message::FallbackSelected(position))
                .into();
            let line: Element<Message> = if selected {
                container(line).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                line
            };
            list = list.push(line);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// The fallback manager for assistive technology: every row, enabled
    /// then available, by its title and subtitle.
    pub(super) fn fallbacks_semantics(&self, page: &FallbacksPage) -> PageSemantics {
        let items = page
            .rows
            .iter()
            .map(|row| {
                page.candidates
                    .get(row.candidate)
                    .map(|candidate| {
                        item(candidate.title.clone(), Some(candidate.subtitle.clone()))
                    })
                    .unwrap_or_default()
            })
            .collect();
        PageSemantics::list(
            "Fallback commands",
            items,
            page.selected,
            page.empty_notice(),
        )
        .with_status(page.notice.as_deref(), &page.query)
    }
}

/// A row's subtitle and accessory text as one description, leaving out the
/// empty ones.
fn joined(parts: &[&str]) -> String {
    parts
        .iter()
        .copied()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A token set's expiry as `QDateTime::toString()` writes it.
fn expiry_text(at: i64) -> Option<String> {
    let seconds = u64::try_from(at).ok()?;
    crate::file_preview::qt_text_date(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
    )
}

/// What Show Log File says before the engine has written its log.
pub const NO_LOG_YET: &str = "The engine has not written its log file yet";

/// `OpenDefaultVicinaeConfig`: the configuration at its defaults, written
/// read-only into `dir` as `default-config.jsonc` (replacing the last one),
/// for the editor to open. The C++ writes it to the runtime directory; the
/// launcher is given the cache directory ([`AppFlags::default_config_dir`]),
/// because inside a Flatpak `$XDG_RUNTIME_DIR` is the sandbox's own and the
/// host's editor cannot see a file there.
///
/// [`AppFlags::default_config_dir`]: crate::AppFlags::default_config_dir
fn write_default_config_in(dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let failed = |_| "Failed to open temporary file".to_owned();
    std::fs::create_dir_all(dir).map_err(failed)?;
    let path = dir.join("default-config.jsonc");
    if path.exists() {
        std::fs::remove_file(&path).map_err(failed)?;
    }
    let text = serde_json::to_string_pretty(&compass_core::config::default_document())
        .map_err(|_| "Failed to open default config file".to_owned())?;
    std::fs::write(&path, text).map_err(failed)?;
    let mut permissions = std::fs::metadata(&path).map_err(failed)?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).map_err(failed)?;
    Ok(path)
}

/// What the bug report is pre-filled with: this build and this machine.
fn system_info() -> compass_core::bug_report::SystemInfo {
    use compass_core::bug_report::{SystemInfo, os_description, parse_os_release};
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let os_release = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| parse_os_release(&text));
    SystemInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        commit: option_env!("COMPASS_GIT_COMMIT")
            .unwrap_or("unknown")
            .to_owned(),
        build_info: format!(
            "rustc - {profile} - {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
        provenance: option_env!("COMPASS_PROVENANCE")
            .unwrap_or("local")
            .to_owned(),
        os: os_description(
            os_release
                .as_ref()
                .map(|(name, version)| (name.as_str(), version.as_str())),
            std::env::consts::OS,
            std::env::consts::ARCH,
        ),
        qt_platform: "wayland".to_owned(),
        desktop: compass_core::xdg_dirs::current_desktops().join(":"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_core::manifest::{ExtensionManifest, Provenance};

    #[test]
    fn the_default_config_is_written_read_only_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_default_config_in(dir.path()).unwrap();
        assert!(std::fs::metadata(&path).unwrap().permissions().readonly());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
        assert_eq!(write_default_config_in(dir.path()).unwrap(), path, "again");
    }

    #[test]
    fn the_report_names_this_build() {
        let info = system_info();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(info.build_info.starts_with("rustc - "));
    }

    fn app() -> LauncherApp {
        LauncherApp::with_index(compass_core::AppIndex::builder().build())
    }

    fn list(semantics: &PageSemantics) -> &crate::semantics::List {
        match &semantics.body {
            Body::List(list) => list,
            other => panic!("expected a list, got {other:?}"),
        }
    }

    fn manifest(id: &str, title: &str, description: &str) -> ExtensionManifest {
        ExtensionManifest {
            path: std::path::PathBuf::from("/extensions").join(id),
            id: id.into(),
            name: id.into(),
            title: title.into(),
            description: description.into(),
            icon: String::new(),
            author: "someone".into(),
            owner: None,
            categories: Vec::new(),
            preferences: Vec::new(),
            commands: Vec::new(),
            needs_raycast_api: false,
            provenance: Provenance::from_id(id),
        }
    }

    #[test]
    fn the_fallback_manager_describes_every_row_in_its_order() {
        let candidate = |id: &str, title: &str, subtitle: &str, enabled: bool| Candidate {
            id: id.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            keywords: Vec::new(),
            icon: None,
            enabled_as: enabled.then(|| id.to_owned()),
        };
        let app = app();
        let mut page = FallbacksPage::new(
            vec![
                candidate("shortcuts:google", "Google", "Quicklink", false),
                candidate("files:search", "Search Files", "Command", true),
            ],
            vec!["files:search".into()],
        );
        page.selected = 1;
        let semantics = app.fallbacks_semantics(&page);
        let rows = list(&semantics);
        assert_eq!(
            rows.items,
            [
                item("Search Files", Some("Command".into())),
                item("Google", Some("Quicklink".into())),
            ],
            "enabled first, as drawn, without the section headings"
        );
        assert_eq!(rows.selected, Some(1));
        assert_eq!(semantics.status, None);

        page.query = "zzzz".into();
        page.refilter();
        let semantics = app.fallbacks_semantics(&page);
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(semantics.status.as_deref(), Some("No commands match"));
        let nothing = FallbacksPage::new(Vec::new(), Vec::new());
        assert_eq!(
            app.fallbacks_semantics(&nothing).status.as_deref(),
            Some("Nothing can be a fallback")
        );
    }

    #[test]
    fn installed_extensions_are_described_with_where_they_came_from() {
        let app = app();
        let none = ExtensionsPage::new(Vec::new());
        assert_eq!(
            app.extensions_semantics(&none).status.as_deref(),
            Some("No extensions are installed")
        );
        let mut page = ExtensionsPage::new(vec![
            manifest("store.raycast.github", "GitHub", "Work with issues"),
            manifest("notes", "Notes", ""),
        ]);
        page.notice = Some(crate::store_page::UNINSTALLED.to_owned());
        let semantics = app.extensions_semantics(&page);
        assert_eq!(
            list(&semantics).items,
            [
                item("GitHub", Some("Work with issues · Raycast".into())),
                item("Notes", Some("Local".into())),
            ]
        );
        assert_eq!(list(&semantics).selected, Some(0));
        assert_eq!(
            semantics.status.as_deref(),
            Some(crate::store_page::UNINSTALLED)
        );
        page.query = "zzzz".into();
        page.refilter();
        assert_eq!(
            app.extensions_semantics(&page).status.as_deref(),
            Some("No extensions match")
        );
    }

    #[test]
    fn builtin_icons_are_described_by_name_and_counted_when_searched() {
        let app = app();
        let mut page = IconsPage::new();
        let names = compass_core::builtin_icon::names();
        let semantics = app.icons_semantics(&page);
        assert_eq!(list(&semantics).items.len(), names.len());
        assert_eq!(list(&semantics).items[0], item(names[page.shown[0]], None));
        assert_eq!(semantics.status, None, "nothing typed, nothing counted");

        page.query = "copy-clipboard".into();
        page.refilter();
        let semantics = app.icons_semantics(&page);
        assert_eq!(list(&semantics).items[0].name, "copy-clipboard");
        assert_eq!(
            semantics.status,
            Some(crate::semantics::result_count(list(&semantics).items.len()))
        );
        page.query = "zzzzqqqq".into();
        page.refilter();
        assert_eq!(
            app.icons_semantics(&page).status.as_deref(),
            Some("No icons match")
        );
    }

    #[test]
    fn local_storage_describes_namespaces_then_the_keys_of_one() {
        use crate::backend::StorageItemRow;
        let app = app();
        let mut page = StoragePage::default();
        assert_eq!(
            app.storage_semantics(&page).status.as_deref(),
            Some("No namespaces")
        );
        page.notice = Some("The engine is not running".into());
        assert_eq!(
            app.storage_semantics(&page).status.as_deref(),
            Some("The engine is not running"),
            "why nothing could be read, in place of the list"
        );
        page.notice = None;
        page.set_namespaces(vec!["@a/notes".into(), "core".into()]);
        let semantics = app.storage_semantics(&page);
        assert_eq!(list(&semantics).label, "Namespaces");
        assert_eq!(
            list(&semantics).items,
            [item("@a/notes", None), item("core", None)]
        );
        page.browse(
            "@a/notes".into(),
            vec![StorageItemRow {
                key: "draft".into(),
                value: "hello".into(),
            }],
        );
        page.notice = Some("hello".into());
        let semantics = app.storage_semantics(&page);
        assert_eq!(list(&semantics).label, "Items");
        assert_eq!(list(&semantics).items, [item("draft", None)]);
        assert_eq!(
            semantics.status.as_deref(),
            Some("hello"),
            "the value shown"
        );
    }

    #[test]
    fn token_sets_are_described_with_their_provider_and_expiry() {
        use crate::backend::TokenSetRow;
        let mut app = app();
        let mut page = TokensPage::default();
        assert_eq!(
            app.tokens_semantics(&page).status.as_deref(),
            Some("No token sets")
        );
        page.set_sets(vec![
            TokenSetRow {
                extension_id: "github".into(),
                provider_id: Some("GitHub".into()),
                expired: true,
                ..TokenSetRow::default()
            },
            TokenSetRow {
                extension_id: "linear".into(),
                ..TokenSetRow::default()
            },
        ]);
        page.selected = 1;
        app.page = Page::Tokens(page);
        let semantics = app.semantics();
        let Body::List(rows) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(
            rows.items,
            [
                item("github", Some("GitHub · Expired".into())),
                item("linear", None),
            ]
        );
        assert_eq!(rows.selected, Some(1));
    }

    #[test]
    fn a_store_intro_is_read_as_its_text_then_its_keys() {
        let mut app = app();
        let page = crate::compass_pages::StoreIntroPage::new(crate::backend::Store::Raycast);
        let semantics = app.store_intro_semantics(&page);
        let Body::Document(document) = &semantics.body else {
            panic!("expected a document, got {:?}", semantics.body);
        };
        assert_eq!(document.label, "Raycast Store");
        assert_eq!(
            document.blocks.first(),
            Some(&Block {
                text: "Welcome to the Raycast Extension Store".to_owned(),
                heading: Some(1),
            })
        );
        assert_eq!(
            document.blocks.last().map(|block| block.text.clone()),
            Some(crate::compass_pages::intro_hint())
        );
        app.page = Page::StoreIntro(page);
        let semantics = app.semantics();
        assert!(semantics.search.is_none());
        assert!(matches!(semantics.body, Body::Document(_)));
    }
}
