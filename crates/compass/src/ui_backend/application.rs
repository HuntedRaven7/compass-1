//! The [`ApplicationBackend`] impl: every request the launcher's own views
//! make of the engine.
//!
//! A trait impl cannot be split across files, so the requests of the larger
//! domains forward to an inherent method of the same name in their own
//! submodule: extensions and the store in `extensions`, script commands in
//! `scripts`, the settings view in `settings`, snippets and shortcuts in
//! `snippets`. An inherent method is found before a trait method of the same
//! name, so a forward does not recurse.

use std::time::Duration;

use compass_ipc::Request;
use compass_ui::backend::{
    ApplicationBackend, BackendFuture, DmenuList, ExtensionDraft, ExtensionStart,
    ExtensionViewState, FileResults, FileRow, ProgramList, ScriptOutputState, Shortcut,
    ShortcutDraft, Snippet, SnippetDraft,
};

use super::{DaemonBackend, OPEN_TIMEOUT, REQUEST_TIMEOUT, STORE_TIMEOUT};

impl ApplicationBackend for DaemonBackend {
    fn search(&self, query: String) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async move {
            let hits =
                tokio::time::timeout(REQUEST_TIMEOUT, crate::ipc::query(&self.socket, &query))
                    .await
                    .map_err(|_| "Application search timed out".to_owned())?
                    .map_err(|error| error.to_string())?;
            Ok(hits.into_iter().map(|hit| hit.id).collect())
        })
    }

    fn record_launch(&self, key: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::timeout(
                REQUEST_TIMEOUT,
                crate::ipc::send_ack(&self.socket, Request::RecordLaunch { key }),
            )
            .await
            .map_err(|_| "Launch history update timed out".to_owned())?
            .map_err(|error| error.to_string())
        })
    }

    fn edit_root_item(
        &self,
        id: String,
        edit: compass_core::root_items::RootEdit,
    ) -> BackendFuture<'_, ()> {
        use compass_core::root_items::RootEdit;
        let edit = match edit {
            RootEdit::Favorite(favorite) => compass_ipc::RootItemEdit::Favorite(favorite),
            RootEdit::MoveFavorite { down } => compass_ipc::RootItemEdit::MoveFavorite { down },
            RootEdit::Alias(alias) => compass_ipc::RootItemEdit::Alias(alias),
            RootEdit::Disable => compass_ipc::RootItemEdit::Disable,
            RootEdit::ResetRanking => compass_ipc::RootItemEdit::ResetRanking,
            RootEdit::Shortcut(shortcut) => compass_ipc::RootItemEdit::Shortcut(shortcut),
            RootEdit::Enabled(enabled) => compass_ipc::RootItemEdit::Enabled(enabled),
            RootEdit::Fallback(enabled) => compass_ipc::RootItemEdit::Fallback(enabled),
        };
        Box::pin(async move {
            match self
                .ask(Request::RootItemEdit { id, edit }, "Changing the item")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_power_command(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::RunPowerCommand { id }, "The power command")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_media_command(&self, id: String, argument: Option<String>) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::RunMediaCommandWith { id, argument },
                    "The media command",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn calculator_history(
        &self,
        query: String,
    ) -> BackendFuture<'_, Vec<compass_ui::backend::CalculatorGroupRow>> {
        Box::pin(async move {
            match self
                .ask(
                    Request::CalculatorHistory { query },
                    "Reading the calculator history",
                )
                .await?
            {
                compass_ipc::Response::CalculatorHistory { groups } => Ok(groups
                    .into_iter()
                    .map(|group| compass_ui::backend::CalculatorGroupRow {
                        name: group.name,
                        records: group
                            .records
                            .into_iter()
                            .map(|record| compass_ui::backend::CalculatorRow {
                                id: record.id,
                                question: record.question,
                                answer: record.answer,
                                conversion: record.conversion,
                                pinned: record.pinned,
                            })
                            .collect(),
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn add_calculator_record(
        &self,
        question: String,
        answer: String,
        conversion: bool,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let request = Request::AddCalculatorRecord {
                question,
                answer,
                conversion,
            };
            match self.ask(request, "Remembering the calculation").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn exchange_rates(
        &self,
    ) -> BackendFuture<'_, Option<compass_core::exchange_rates::ExchangeRates>> {
        Box::pin(async move {
            match self
                .ask(Request::ExchangeRates, "Asking for the exchange rates")
                .await?
            {
                compass_ipc::Response::ExchangeRates { rates } => Ok(rates.map(exchange_rates)),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn refresh_exchange_rates(
        &self,
    ) -> BackendFuture<'_, compass_core::exchange_rates::ExchangeRates> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::RefreshExchangeRates,
                    "Refreshing the exchange rates",
                    STORE_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::ExchangeRates { rates: Some(rates) } => {
                    Ok(exchange_rates(rates))
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn edit_calculator_history(
        &self,
        change: compass_ui::backend::CalculatorChange,
    ) -> BackendFuture<'_, ()> {
        use compass_ui::backend::CalculatorChange;
        Box::pin(async move {
            let edit = match change {
                CalculatorChange::Pin(id) => compass_ipc::CalculatorEdit::Pin(id),
                CalculatorChange::Unpin(id) => compass_ipc::CalculatorEdit::Unpin(id),
                CalculatorChange::Remove(id) => compass_ipc::CalculatorEdit::Remove(id),
                CalculatorChange::RemoveAll => compass_ipc::CalculatorEdit::RemoveAll,
            };
            match self
                .ask(
                    Request::EditCalculatorHistory { edit },
                    "Changing the calculator history",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_script_grants(&self) -> BackendFuture<'_, Vec<compass_ui::backend::ScriptGrant>> {
        self.list_script_grants()
    }

    fn list_openers(
        &self,
        target: String,
    ) -> BackendFuture<'_, Vec<compass_ui::backend::OpenerRow>> {
        Box::pin(async move {
            match self
                .ask(Request::ListOpeners { target }, "Listing applications")
                .await?
            {
                compass_ipc::Response::Openers { apps } => Ok(apps
                    .into_iter()
                    .map(|app| compass_ui::backend::OpenerRow {
                        id: app.id,
                        name: app.name,
                        icon: app.icon,
                        default: app.default,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn open_with(&self, app: String, target: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::OpenWith { app, target }, "Opening")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn local_storage_namespaces(&self) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async move {
            match self
                .ask(Request::LocalStorageNamespaces, "Reading local storage")
                .await?
            {
                compass_ipc::Response::LocalStorageNamespaces { namespaces } => Ok(namespaces),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn local_storage_items(
        &self,
        namespace: String,
    ) -> BackendFuture<'_, Vec<compass_ui::backend::StorageItemRow>> {
        Box::pin(async move {
            match self
                .ask(
                    Request::LocalStorageItems { namespace },
                    "Reading local storage",
                )
                .await?
            {
                compass_ipc::Response::LocalStorageItems { items } => Ok(items
                    .into_iter()
                    .map(|item| compass_ui::backend::StorageItemRow {
                        key: item.key,
                        value: item.value,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn oauth_token_sets(&self) -> BackendFuture<'_, Vec<compass_ui::backend::TokenSetRow>> {
        Box::pin(async move {
            match self
                .ask(Request::OAuthTokenSets, "Reading the token sets")
                .await?
            {
                compass_ipc::Response::OAuthTokenSets { sets } => Ok(sets
                    .into_iter()
                    .map(|set| compass_ui::backend::TokenSetRow {
                        extension_id: set.extension_id,
                        provider_id: set.provider_id,
                        access_token: set.access_token,
                        refresh_token: set.refresh_token,
                        id_token: set.id_token,
                        scope: set.scope,
                        expires_at: set.expires_at,
                        expired: set.expired,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn remove_oauth_token_set(
        &self,
        extension_id: String,
        provider_id: Option<String>,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::RemoveOAuthTokenSet {
                        extension_id,
                        provider_id,
                    },
                    "Removing the token set",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn file_actions(&self, path: String) -> BackendFuture<'_, compass_ui::backend::FileActions> {
        Box::pin(async move {
            match self
                .ask(Request::FileActions { path }, "Reading the file")
                .await?
            {
                compass_ipc::Response::FileActions(info) => Ok(compass_ui::backend::FileActions {
                    mime: info.mime,
                    has_opener: info.has_opener,
                    can_set_wallpaper: info.can_set_wallpaper,
                    can_paste: info.can_paste,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn copy_file(&self, path: String, paste: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::CopyFile { path, paste }, "Copying the file")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_executable(&self, path: String, make_executable: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let request = Request::RunExecutable {
                path,
                make_executable,
            };
            match self.ask(request, "Running the executable").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn set_wallpaper(&self, path: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::SetWallpaper { path },
                    "Setting the wallpaper",
                    Duration::from_secs(35),
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_default_apps(
        &self,
        kind: compass_ui::backend::DefaultApp,
    ) -> BackendFuture<'_, Vec<compass_ui::backend::DefaultAppRow>> {
        self.list_default_apps(kind)
    }

    fn set_default_app(
        &self,
        kind: compass_ui::backend::DefaultApp,
        id: String,
    ) -> BackendFuture<'_, ()> {
        self.set_default_app(kind, id)
    }

    fn catalog_generation(&self) -> BackendFuture<'_, u64> {
        Box::pin(async move {
            match self
                .ask(Request::CatalogGeneration, "Asking what changed")
                .await?
            {
                compass_ipc::Response::CatalogGeneration { generation } => Ok(generation),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn revoke_script_grant(
        &self,
        id: String,
    ) -> BackendFuture<'_, Vec<compass_ui::backend::ScriptGrant>> {
        self.revoke_script_grant(id)
    }

    fn set_font(&self, family: String) -> BackendFuture<'_, ()> {
        self.set_font(family)
    }

    fn set_setting(&self, key: String, value: serde_json::Value) -> BackendFuture<'_, ()> {
        self.set_setting(key, value)
    }

    fn probe_shortcut(&self, trigger: String) -> BackendFuture<'_, Option<String>> {
        self.probe_shortcut(trigger)
    }

    fn launcher_shortcut(
        &self,
        request: bool,
    ) -> BackendFuture<'_, compass_core::launcher_shortcut::LauncherShortcut> {
        self.launcher_shortcut(request)
    }

    fn login_start(
        &self,
        enable: Option<bool>,
    ) -> BackendFuture<'_, compass_core::login_start::Status> {
        self.login_start(enable)
    }

    fn set_shortcut_capture(&self, capturing: bool) -> BackendFuture<'_, ()> {
        self.set_shortcut_capture(capturing)
    }

    fn update_status(&self) -> BackendFuture<'_, Option<compass_ui::backend::UpdateOffer>> {
        self.update_status()
    }

    fn skip_update(&self, tag: String) -> BackendFuture<'_, ()> {
        self.skip_update(tag)
    }

    fn set_provider_enabled(&self, provider: String, enabled: bool) -> BackendFuture<'_, ()> {
        self.set_provider_enabled(provider, enabled)
    }

    fn tray_items(&self) -> BackendFuture<'_, Vec<compass_ui::backend::TrayItemRow>> {
        Box::pin(async move {
            match self.ask(Request::TrayItems, "Listing tray items").await? {
                compass_ipc::Response::TrayItems { items } => Ok(items
                    .into_iter()
                    .map(|item| compass_ui::backend::TrayItemRow {
                        key: item.key,
                        title: item.title,
                        subtitle: item.subtitle,
                        attention: item.attention,
                        has_menu: item.has_menu,
                        item_is_menu: item.item_is_menu,
                        icon_path: item.icon_path,
                        icon_name: item.icon_name,
                        icon_png: item.icon_png,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn tray_activate(&self, key: String, secondary: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::TrayActivate { key, secondary },
                    "Activating the tray item",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn tray_menu(&self, key: String) -> BackendFuture<'_, Vec<compass_ui::backend::TrayMenuRow>> {
        Box::pin(async move {
            match self
                .ask(Request::TrayMenu { key }, "Reading the tray menu")
                .await?
            {
                compass_ipc::Response::TrayMenu { entries } => Ok(entries
                    .into_iter()
                    .map(|entry| compass_ui::backend::TrayMenuRow {
                        id: entry.id,
                        label: entry.label,
                        toggled: entry.toggled,
                        icon_name: entry.icon_name,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn tray_trigger(&self, key: String, id: i32) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::TrayTriggerMenu { key, id },
                    "Running the menu entry",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_media_players(&self) -> BackendFuture<'_, Vec<compass_ui::backend::MediaPlayerRow>> {
        Box::pin(async move {
            match self
                .ask(Request::ListMediaPlayers, "Listing media players")
                .await?
            {
                compass_ipc::Response::MediaPlayers { players } => Ok(players
                    .into_iter()
                    .map(|player| compass_ui::backend::MediaPlayerRow {
                        id: player.id,
                        identity: player.identity,
                        app_id: player.app_id,
                        title: player.title,
                        artist: player.artist,
                        playing: player.playing,
                        paused: player.paused,
                        can_go_next: player.can_go_next,
                        can_go_previous: player.can_go_previous,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn control_media_player(
        &self,
        player: String,
        action: compass_ui::backend::MediaAction,
    ) -> BackendFuture<'_, ()> {
        use compass_ipc::MediaPlayerAction;
        use compass_ui::backend::MediaAction;
        let action = match action {
            MediaAction::PlayPause => MediaPlayerAction::PlayPause,
            MediaAction::Next => MediaPlayerAction::Next,
            MediaAction::Previous => MediaPlayerAction::Previous,
        };
        Box::pin(async move {
            match self
                .ask(
                    Request::ControlMediaPlayer { player, action },
                    "The media player",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn search_files(
        &self,
        query: String,
        category: Option<String>,
    ) -> BackendFuture<'_, FileResults> {
        Box::pin(async move {
            match self
                .ask(Request::SearchFiles { query, category }, "File search")
                .await?
            {
                compass_ipc::Response::Files { heading, files } => Ok(FileResults {
                    heading,
                    files: files
                        .into_iter()
                        .map(|file| FileRow {
                            path: file.path,
                            name: file.name,
                            category: file.category,
                        })
                        .collect(),
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn open_file(&self, path: String, reveal: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::OpenFile { path, reveal },
                    "Opening the file",
                    OPEN_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn create_extension(&self, draft: ExtensionDraft) -> BackendFuture<'_, String> {
        self.create_extension(draft)
    }

    fn set_theme(&self, theme: String) -> BackendFuture<'_, ()> {
        self.set_theme(theme)
    }

    fn list_fonts(&self) -> BackendFuture<'_, compass_ui::backend::FontList> {
        self.list_fonts()
    }

    fn store_browse(
        &self,
        store: compass_ui::backend::Store,
        query: String,
    ) -> BackendFuture<'_, compass_ui::backend::StoreList> {
        self.store_browse(store, query)
    }

    fn store_extension(
        &self,
        store: compass_ui::backend::Store,
        owner: String,
        name: String,
    ) -> BackendFuture<'_, compass_ui::backend::StoreDetail> {
        self.store_extension(store, owner, name)
    }

    fn store_install(
        &self,
        store: compass_ui::backend::Store,
        owner: String,
        name: String,
    ) -> BackendFuture<'_, (String, String)> {
        self.store_install(store, owner, name)
    }

    fn store_uninstall(&self, id: String) -> BackendFuture<'_, ()> {
        self.store_uninstall(id)
    }

    fn open_url(&self, url: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::OpenUrl { url }, "Opening the link")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn font_specimen(&self, name: String) -> BackendFuture<'_, String> {
        self.font_specimen(name)
    }

    fn fetch_launch(&self, token: u64) -> BackendFuture<'_, compass_ui::backend::ExtensionLaunch> {
        self.fetch_launch(token)
    }

    fn extension_subtitles(&self) -> BackendFuture<'_, Vec<(String, String)>> {
        self.extension_subtitles()
    }

    fn extension_preferences(&self, id: String) -> BackendFuture<'_, ExtensionStart> {
        self.extension_preferences(id)
    }

    fn fetch_dmenu(&self, token: u64) -> BackendFuture<'_, DmenuList> {
        Box::pin(async move {
            match self
                .ask(Request::DmenuFetch { token }, "Fetching the dmenu list")
                .await?
            {
                compass_ipc::Response::DmenuList { spec } => Ok(DmenuList {
                    content: spec.content,
                    navigation_title: spec.navigation_title,
                    section_title: spec.section_title,
                    output_index: spec.output_index,
                    placeholder: spec.placeholder,
                    query: spec.query,
                    no_section: spec.no_section,
                    no_quick_look: spec.no_quick_look,
                    width: spec.width,
                    height: spec.height,
                    no_metadata: spec.no_metadata,
                    no_footer: spec.no_footer,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn choose_dmenu(&self, token: u64, output: Option<String>) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::DmenuChoose { token, output },
                    "Answering the dmenu list",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_programs(&self) -> BackendFuture<'_, ProgramList> {
        Box::pin(async move {
            match self.ask(Request::ListPrograms, "Listing programs").await? {
                compass_ipc::Response::Programs {
                    programs,
                    terminal,
                    default_action,
                } => Ok(ProgramList {
                    programs,
                    terminal,
                    default_action,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_program(&self, argv: Vec<String>, terminal: bool, hold: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::RunProgram {
                        argv,
                        terminal,
                        hold,
                    },
                    "Running the program",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_scripts(&self) -> BackendFuture<'_, Vec<compass_core::script_scan::ScriptItem>> {
        self.list_scripts()
    }

    fn list_rhai_scripts(
        &self,
    ) -> BackendFuture<'_, Vec<compass_core::rhai_scripts::RhaiScriptItem>> {
        self.list_rhai_scripts()
    }

    fn run_script(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, Option<u64>> {
        self.run_script(id, arguments)
    }

    fn script_output(&self, session: u64) -> BackendFuture<'_, ScriptOutputState> {
        self.script_output(session)
    }

    fn stop_script(&self, session: u64) -> BackendFuture<'_, ()> {
        self.stop_script(session)
    }

    fn list_snippets(&self) -> BackendFuture<'_, Vec<Snippet>> {
        self.list_snippets()
    }

    fn save_snippet(&self, snippet: SnippetDraft) -> BackendFuture<'_, Vec<Snippet>> {
        self.save_snippet(snippet)
    }

    fn remove_snippet(&self, id: String) -> BackendFuture<'_, Vec<Snippet>> {
        self.remove_snippet(id)
    }

    fn expand_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, String> {
        self.expand_snippet(id, arguments)
    }

    fn preview_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, String> {
        self.preview_snippet(id, arguments)
    }

    fn script_icons(&self) -> BackendFuture<'_, Vec<(String, String)>> {
        self.script_icons()
    }

    fn copy_text(&self, text: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self.ask(Request::CopyText { text }, "Copying").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn paste_text(&self, text: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self.ask(Request::PasteText { text }, "Pasting").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn paste_snippet(&self, id: String, arguments: Vec<(String, String)>) -> BackendFuture<'_, ()> {
        self.paste_snippet(id, arguments)
    }

    fn list_shortcuts(&self) -> BackendFuture<'_, Vec<Shortcut>> {
        self.list_shortcuts()
    }

    fn save_shortcut(&self, shortcut: ShortcutDraft) -> BackendFuture<'_, Vec<Shortcut>> {
        self.save_shortcut(shortcut)
    }

    fn remove_shortcut(&self, id: String) -> BackendFuture<'_, Vec<Shortcut>> {
        self.remove_shortcut(id)
    }

    fn launch_command(&self, id: String, query: Option<String>) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let request = Request::LaunchCommand {
                id,
                args: Vec::new(),
                cwd: None,
                query,
            };
            match self.ask(request, "Launching the command").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn open_shortcut(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, ()> {
        self.open_shortcut(id, arguments)
    }

    fn expand_shortcut(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, String> {
        self.expand_shortcut(id, arguments)
    }

    fn run_extension_command(
        &self,
        id: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> BackendFuture<'_, ExtensionStart> {
        self.run_extension_command(id, arguments)
    }

    fn extension_view(&self, session: u64, after: u64) -> BackendFuture<'_, ExtensionViewState> {
        self.extension_view(session, after)
    }

    fn extension_event(
        &self,
        session: u64,
        handler: String,
        args: Vec<serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        self.extension_event(session, handler, args)
    }

    fn set_extension_preferences(
        &self,
        id: String,
        values: serde_json::Map<String, serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        self.set_extension_preferences(id, values)
    }

    fn extension_alert_answer(&self, session: u64, confirmed: bool) -> BackendFuture<'_, ()> {
        self.extension_alert_answer(session, confirmed)
    }

    fn extension_alert_remember(&self, session: u64) -> BackendFuture<'_, ()> {
        self.extension_alert_remember(session)
    }

    fn extension_pop(&self, session: u64) -> BackendFuture<'_, ()> {
        self.extension_pop(session)
    }

    fn close_extension(&self, session: u64) -> BackendFuture<'_, ()> {
        self.close_extension(session)
    }

    fn choose_files(
        &self,
        choice: compass_ui::extension_fields::FileChoice,
    ) -> BackendFuture<'_, Vec<String>> {
        self.choose_files(choice)
    }
}

/// Exchange rates as the calculator holds them, from the wire. A rate that
/// does not read back is left out, so that currency answers nothing.
fn exchange_rates(
    table: compass_ipc::ExchangeRateTable,
) -> compass_core::exchange_rates::ExchangeRates {
    compass_core::exchange_rates::ExchangeRates {
        date: table.date,
        fetched_at: table.fetched_at,
        rates: table
            .rates
            .into_iter()
            .filter_map(|(code, rate)| Some((code, rate.parse::<f64>().ok()?)))
            .collect(),
    }
}
