//! Socket adapter for the UI's shared application search/history service.

use std::time::Duration;

use compass_ipc::{Request, SocketPath};
use compass_ui::backend::{
    ApplicationBackend, BackendFuture, ClipboardBackend, ClipboardContent, ClipboardRow,
    ClipboardRowKind, DmenuList, ExtensionDraft, ExtensionStart, ExtensionViewState, FileResults,
    FileRow, ProgramList, ScriptOutputState, Shortcut, ShortcutDraft, Snippet, SnippetDraft,
    WindowBackend, WindowRow,
};

/// How long an ordinary request may take: one the engine answers from what
/// it holds. A request the engine may hold longer has its own deadline below.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// How long reading an extension's view may take: the engine holds the
/// request until the view changes or [`compass_ipc::EXTENSION_VIEW_HOLD`]
/// passes, so an idle view is answered only then.
const VIEW_TIMEOUT: Duration = compass_ipc::long_poll_deadline(compass_ipc::EXTENSION_VIEW_HOLD);

/// How long starting an extension command may take: the engine waits for
/// the runtime to take the command, for up to
/// [`crate::extension_runner::LOAD_TIMEOUT`].
const START_TIMEOUT: Duration =
    compass_ipc::long_poll_deadline(crate::extension_runner::LOAD_TIMEOUT);

/// How long asking the desktop about a shortcut may take: the engine waits
/// for the backend's answer for up to [`crate::global_shortcuts::PROBE_TIMEOUT`].
const PROBE_TIMEOUT: Duration =
    compass_ipc::long_poll_deadline(crate::global_shortcuts::PROBE_TIMEOUT);

/// How long asking the desktop for the launcher hotkey may take: the engine
/// waits for the portal's dialog for up to
/// [`crate::global_shortcuts::LAUNCHER_REQUEST_TIMEOUT`].
const LAUNCHER_TIMEOUT: Duration =
    compass_ipc::long_poll_deadline(crate::global_shortcuts::LAUNCHER_REQUEST_TIMEOUT);

/// How long turning login start on or off may take: probing the portals,
/// then the Background portal's dialog for up to
/// [`compass_platform_linux::LOGIN_START_DIALOG_TIMEOUT`].
const LOGIN_START_TIMEOUT: Duration = compass_ipc::long_poll_deadline(
    compass_platform_linux::LOGIN_START_DIALOG_TIMEOUT.saturating_add(Duration::from_secs(30)),
);

/// How long the update check may take: the engine may be asking the release
/// feed, which it allows 15 seconds.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a store listing or detail page may take: it is fetched from the
/// network, where two seconds is not enough.
const STORE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long opening a file may take: the engine waits to hear that the
/// application started, and the `OpenURI` portal it falls back to may ask
/// the person which application to use.
const OPEN_TIMEOUT: Duration = Duration::from_secs(35);

/// How long an install may take: a download of up to the bundle cap, then
/// the unpack.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

const fn store_kind(store: compass_ui::backend::Store) -> compass_ipc::StoreKind {
    match store {
        compass_ui::backend::Store::Vicinae => compass_ipc::StoreKind::Vicinae,
        compass_ui::backend::Store::Raycast => compass_ipc::StoreKind::Raycast,
    }
}

fn store_row(entry: compass_ipc::StoreEntry) -> compass_ui::backend::StoreRow {
    compass_ui::backend::StoreRow {
        id: entry.id,
        name: entry.name,
        author: entry.author,
        author_name: entry.author_name,
        title: entry.title,
        description: entry.description,
        icon_light: entry.icon_light,
        icon_dark: entry.icon_dark,
        downloads: entry.downloads,
        installed: entry.installed,
        update_available: entry.update_available,
        compat: entry.compat,
        author_avatar: entry.author_avatar,
        owner: entry.owner,
    }
}

fn script_grant(entry: compass_ipc::ScriptGrantEntry) -> compass_ui::backend::ScriptGrant {
    compass_ui::backend::ScriptGrant {
        id: entry.id,
        title: entry.title,
        capabilities: entry.capabilities,
        descriptions: entry.descriptions,
    }
}

fn default_app_kind(kind: compass_ui::backend::DefaultApp) -> compass_ipc::DefaultAppKind {
    match kind {
        compass_ui::backend::DefaultApp::Browser => compass_ipc::DefaultAppKind::Browser,
        compass_ui::backend::DefaultApp::Terminal => compass_ipc::DefaultAppKind::Terminal,
    }
}

/// Uses the same engine/socket as the resident window link.
#[derive(Debug)]
pub struct DaemonBackend {
    socket: SocketPath,
}

impl DaemonBackend {
    /// Bind the adapter to a resolved socket without connecting yet.
    pub fn new(socket: SocketPath) -> Self {
        Self { socket }
    }
}

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
        Box::pin(async move {
            match self
                .ask(Request::ListScriptGrants, "Reading script permissions")
                .await?
            {
                compass_ipc::Response::ScriptGrants { grants } => {
                    Ok(grants.into_iter().map(script_grant).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
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
        Box::pin(async move {
            match self
                .ask(
                    Request::ListDefaultApps {
                        kind: default_app_kind(kind),
                    },
                    "Listing the applications",
                )
                .await?
            {
                compass_ipc::Response::DefaultApps { apps } => Ok(apps
                    .into_iter()
                    .map(|app| compass_ui::backend::DefaultAppRow {
                        id: app.id,
                        name: app.name,
                        description: app.description,
                        is_default: app.is_default,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn set_default_app(
        &self,
        kind: compass_ui::backend::DefaultApp,
        id: String,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::SetDefaultApp {
                        kind: default_app_kind(kind),
                        id,
                    },
                    "Setting the default",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
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
        Box::pin(async move {
            match self
                .ask(Request::RevokeScriptGrant { id }, "Revoking permissions")
                .await?
            {
                compass_ipc::Response::ScriptGrants { grants } => {
                    Ok(grants.into_iter().map(script_grant).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn set_font(&self, family: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::SetFont { family }, "Setting the font")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn set_setting(&self, key: String, value: serde_json::Value) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let value_json = value.to_string();
            match self
                .ask(
                    Request::SetSetting { key, value_json },
                    "Saving the setting",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn probe_shortcut(&self, trigger: String) -> BackendFuture<'_, Option<String>> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::ProbeShortcut { trigger },
                    "Asking the desktop about the shortcut",
                    PROBE_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::ShortcutProbe { refusal } => Ok(refusal),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn launcher_shortcut(
        &self,
        request: bool,
    ) -> BackendFuture<'_, compass_core::launcher_shortcut::LauncherShortcut> {
        Box::pin(async move {
            let deadline = if request {
                LAUNCHER_TIMEOUT
            } else {
                REQUEST_TIMEOUT
            };
            match self
                .ask_within(
                    Request::LauncherShortcut { request },
                    "Asking the desktop for the launcher hotkey",
                    deadline,
                )
                .await?
            {
                compass_ipc::Response::LauncherShortcut { state } => {
                    Ok(crate::permissions::launcher_from_wire(state))
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn login_start(
        &self,
        enable: Option<bool>,
    ) -> BackendFuture<'_, compass_core::login_start::Status> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::LoginStart { enable },
                    "Changing whether Compass starts at login",
                    LOGIN_START_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::LoginStart { state, unit } => {
                    Ok(crate::permissions::login_start_from_wire(state, unit))
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn set_shortcut_capture(&self, capturing: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ShortcutCapture { capturing },
                    "Suspending the global shortcuts",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn update_status(&self) -> BackendFuture<'_, Option<compass_ui::backend::UpdateOffer>> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::UpdateStatus,
                    "Checking for updates",
                    UPDATE_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::UpdateStatus { current, available } => {
                    Ok(available.map(|offer| compass_ui::backend::UpdateOffer {
                        tag: offer.tag,
                        version: offer.version,
                        release_url: offer.release_url,
                        current,
                    }))
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn skip_update(&self, tag: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::SkipUpdate { tag }, "Skipping the update")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn set_provider_enabled(&self, provider: String, enabled: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::SetProviderEnabled { provider, enabled },
                    "Changing the provider",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
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
        Box::pin(async move {
            let request = Request::CreateExtension {
                author: draft.author,
                title: draft.title,
                description: draft.description,
                location: draft.location,
                command_title: draft.command_title,
                command_description: draft.command_description,
                template: draft.template,
            };
            match self.ask(request, "Creating the extension").await? {
                compass_ipc::Response::ExtensionCreated { path } => Ok(path),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn set_theme(&self, theme: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::SetTheme { theme }, "Saving the theme")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_fonts(&self) -> BackendFuture<'_, compass_ui::backend::FontList> {
        Box::pin(async move {
            match self.ask(Request::ListFonts, "Listing the fonts").await? {
                compass_ipc::Response::Fonts { fonts, categories } => {
                    Ok(compass_ui::backend::FontList {
                        fonts: fonts
                            .into_iter()
                            .map(|font| compass_ui::backend::FontListEntry {
                                name: font.name,
                                family: font.family,
                                glyph: font.glyph,
                                color: font.color,
                                primary: font.primary,
                                categories: font.categories,
                            })
                            .collect(),
                        categories,
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn store_browse(
        &self,
        store: compass_ui::backend::Store,
        query: String,
    ) -> BackendFuture<'_, compass_ui::backend::StoreList> {
        Box::pin(async move {
            let request = Request::StoreBrowse {
                store: store_kind(store),
                query,
            };
            match self
                .ask_within(request, "Loading the store", STORE_TIMEOUT)
                .await?
            {
                compass_ipc::Response::StoreListing { heading, entries } => {
                    Ok(compass_ui::backend::StoreList {
                        heading,
                        rows: entries.into_iter().map(store_row).collect(),
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn store_extension(
        &self,
        store: compass_ui::backend::Store,
        owner: String,
        name: String,
    ) -> BackendFuture<'_, compass_ui::backend::StoreDetail> {
        Box::pin(async move {
            let request = Request::StoreExtension {
                store: store_kind(store),
                owner,
                name,
            };
            match self
                .ask_within(request, "Loading the extension", STORE_TIMEOUT)
                .await?
            {
                compass_ipc::Response::StoreExtension { detail } => {
                    Ok(compass_ui::backend::StoreDetail {
                        row: store_row(detail.entry),
                        markdown: detail.markdown,
                        screenshots: detail.screenshots,
                        readme_url: detail.readme_url,
                        source_url: detail.source_url,
                        store_url: detail.store_url,
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn store_install(
        &self,
        store: compass_ui::backend::Store,
        owner: String,
        name: String,
    ) -> BackendFuture<'_, (String, String)> {
        Box::pin(async move {
            let request = Request::StoreInstall {
                store: store_kind(store),
                owner,
                name,
            };
            match self
                .ask_within(request, "Installing the extension", INSTALL_TIMEOUT)
                .await?
            {
                compass_ipc::Response::StoreInstalled { id, title } => Ok((id, title)),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn store_uninstall(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::StoreUninstall { id },
                    "Uninstalling the extension",
                    STORE_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
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
        Box::pin(async move {
            match self
                .ask(Request::FontSpecimen { name }, "Loading the specimen")
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn fetch_launch(&self, token: u64) -> BackendFuture<'_, compass_ui::backend::ExtensionLaunch> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionLaunchFetch { token },
                    "Taking the extension's launch",
                )
                .await?
            {
                compass_ipc::Response::ExtensionLaunch {
                    id,
                    arguments_json,
                    preferences,
                } => Ok(compass_ui::backend::ExtensionLaunch {
                    id,
                    arguments: launch_arguments(arguments_json)?,
                    preferences,
                    fallback_text: None,
                }),
                compass_ipc::Response::CommandLaunch {
                    id,
                    arguments_json,
                    fallback_text,
                } => Ok(compass_ui::backend::ExtensionLaunch {
                    id,
                    arguments: launch_arguments(arguments_json)?,
                    preferences: false,
                    fallback_text,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_subtitles(&self) -> BackendFuture<'_, Vec<(String, String)>> {
        Box::pin(async move {
            match self
                .ask(Request::ExtensionSubtitles, "Reading command subtitles")
                .await?
            {
                compass_ipc::Response::ExtensionSubtitles { subtitles } => Ok(subtitles),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_preferences(&self, id: String) -> BackendFuture<'_, ExtensionStart> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionPreferences { id },
                    "Reading the preferences",
                )
                .await?
            {
                compass_ipc::Response::ExtensionNeedsPreferences { title, fields } => {
                    Ok(ExtensionStart::NeedsPreferences {
                        title,
                        fields: fields.into_iter().map(preference_input).collect(),
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
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
        Box::pin(async move {
            match self
                .ask(Request::ListScripts, "Listing script commands")
                .await?
            {
                compass_ipc::Response::Scripts { scripts } => {
                    Ok(scripts.into_iter().map(script_item).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_rhai_scripts(
        &self,
    ) -> BackendFuture<'_, Vec<compass_core::rhai_scripts::RhaiScriptItem>> {
        Box::pin(async move {
            match self
                .ask(Request::ListRhaiScripts, "Listing Rhai scripts")
                .await?
            {
                compass_ipc::Response::RhaiScripts { scripts } => Ok(scripts
                    .into_iter()
                    .map(|script| compass_core::rhai_scripts::RhaiScriptItem {
                        id: script.id,
                        title: script.title,
                        description: script.description,
                        icon: script.icon,
                        keywords: script.keywords,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_script(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, Option<u64>> {
        Box::pin(async move {
            match self
                .ask(Request::RunScript { id, arguments }, "Running the script")
                .await?
            {
                compass_ipc::Response::ScriptStarted { session } => Ok(session),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn script_output(&self, session: u64) -> BackendFuture<'_, ScriptOutputState> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ScriptOutput { session },
                    "Reading the script's output",
                )
                .await?
            {
                compass_ipc::Response::ScriptOutput {
                    output,
                    finished,
                    exit_code,
                    elapsed_ms,
                } => Ok(ScriptOutputState {
                    output,
                    finished,
                    exit_code,
                    elapsed_ms,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn stop_script(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::StopScript { session }, "Stopping the script")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_snippets(&self) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(
            async move { snippets(self.ask(Request::ListSnippets, "Listing snippets").await?) },
        )
    }

    fn save_snippet(&self, snippet: SnippetDraft) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(async move {
            let request = Request::SaveSnippet {
                id: snippet.id,
                name: snippet.name,
                text: snippet.text,
                keyword: snippet.keyword,
                word: snippet.word,
                apps: snippet.apps,
            };
            snippets(self.ask(request, "Saving the snippet").await?)
        })
    }

    fn remove_snippet(&self, id: String) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(async move {
            snippets(
                self.ask(Request::RemoveSnippet { id }, "Removing the snippet")
                    .await?,
            )
        })
    }

    fn expand_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, String> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExpandSnippet { id, arguments },
                    "Expanding the snippet",
                )
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn preview_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, String> {
        Box::pin(async move {
            match self
                .ask(
                    Request::PreviewSnippet { id, arguments },
                    "Previewing the snippet",
                )
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn script_icons(&self) -> BackendFuture<'_, Vec<(String, String)>> {
        Box::pin(async move {
            match self
                .ask(Request::ScriptIcons, "Listing script icons")
                .await?
            {
                compass_ipc::Response::ScriptIcons { icons } => Ok(icons),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
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
        Box::pin(async move {
            match self
                .ask(
                    Request::PasteSnippet { id, arguments },
                    "Pasting the snippet",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_shortcuts(&self) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async move {
            shortcuts(
                self.ask(Request::ListShortcuts, "Listing shortcuts")
                    .await?,
            )
        })
    }

    fn save_shortcut(&self, shortcut: ShortcutDraft) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async move {
            let request = Request::SaveShortcut {
                id: shortcut.id,
                name: shortcut.name,
                icon: shortcut.icon,
                url: shortcut.url,
                app: shortcut.app,
            };
            shortcuts(self.ask(request, "Saving the shortcut").await?)
        })
    }

    fn remove_shortcut(&self, id: String) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async move {
            shortcuts(
                self.ask(Request::RemoveShortcut { id }, "Removing the shortcut")
                    .await?,
            )
        })
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
        Box::pin(async move {
            match self
                .ask(
                    Request::OpenShortcut { id, arguments },
                    "Opening the shortcut",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn expand_shortcut(&self, id: String, arguments: Vec<String>) -> BackendFuture<'_, String> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExpandShortcut { id, arguments },
                    "Expanding the shortcut",
                )
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn run_extension_command(
        &self,
        id: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> BackendFuture<'_, ExtensionStart> {
        Box::pin(async move {
            let arguments_json =
                arguments.map(|arguments| serde_json::Value::Object(arguments).to_string());
            match self
                .ask_within(
                    Request::RunExtensionCommand { id, arguments_json },
                    "Running the command",
                    START_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(ExtensionStart::Ran),
                compass_ipc::Response::ExtensionStarted { session } => {
                    Ok(ExtensionStart::View(session))
                }
                compass_ipc::Response::ExtensionNeedsPreferences { title, fields } => {
                    Ok(ExtensionStart::NeedsPreferences {
                        title,
                        fields: fields.into_iter().map(preference_input).collect(),
                    })
                }
                compass_ipc::Response::ExtensionNeedsArguments { title, fields } => {
                    Ok(ExtensionStart::NeedsArguments {
                        title,
                        fields: fields.into_iter().map(preference_input).collect(),
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_view(&self, session: u64, after: u64) -> BackendFuture<'_, ExtensionViewState> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::ExtensionView { session, after },
                    "Reading the view",
                    VIEW_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::ExtensionView {
                    version,
                    view_json,
                    problem,
                    ended,
                    depth,
                    alert,
                    toast,
                } => Ok(ExtensionViewState {
                    depth,
                    toast: toast.map(|toast| compass_ui::backend::ExtensionToast {
                        failure: toast.style == compass_ipc::ExtensionToastStyle::Failure,
                        animated: toast.style == compass_ipc::ExtensionToastStyle::Animated,
                        title: toast.title,
                        message: toast.message,
                    }),
                    alert: alert.map(|alert| compass_ui::backend::ExtensionPrompt {
                        title: alert.title,
                        message: alert.message,
                        confirm_text: alert.confirm_text,
                        cancel_text: alert.cancel_text,
                        remember_text: alert.remember_text,
                    }),
                    version,
                    view: view_json
                        .map(|json| serde_json::from_str(&json).map(Box::new))
                        .transpose()
                        .map_err(|err| {
                            format!("The engine sent a view this launcher cannot read: {err}")
                        })?,
                    problem,
                    ended,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_event(
        &self,
        session: u64,
        handler: String,
        args: Vec<serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let args_json = serde_json::Value::Array(args).to_string();
            match self
                .ask(
                    Request::ExtensionEvent {
                        session,
                        handler,
                        args_json,
                    },
                    "Running the action",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn set_extension_preferences(
        &self,
        id: String,
        values: serde_json::Map<String, serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let values_json = serde_json::Value::Object(values).to_string();
            match self
                .ask(
                    Request::SetExtensionPreferences { id, values_json },
                    "Saving the preferences",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_alert_answer(&self, session: u64, confirmed: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionAlertAnswer { session, confirmed },
                    "Answering the extension",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_alert_remember(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionAlertRemember { session },
                    "Answering the extension",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn extension_pop(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ExtensionPop { session }, "Going back")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn close_extension(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::CloseExtension { session }, "Closing the view")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn choose_files(
        &self,
        choice: compass_ui::extension_fields::FileChoice,
    ) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async move {
            // The portal, not a dialog of our own: inside a Flatpak it is also
            // what grants the extension the file it names.
            let portals =
                compass_portals::Portals::connect(compass_portals::PortalConfig::default())
                    .await
                    .map_err(|err| format!("The file chooser is not available: {err}"))?;
            let chooser = portals
                .file_chooser()
                .map_err(|err| format!("The file chooser is not available: {err}"))?;
            // A picker that takes directories and not files asks for a
            // directory; the portal cannot offer both in one dialog.
            let request = if choice.directories && !choice.files {
                compass_portals::FileChooserRequest::directory("Choose a folder")
            } else {
                compass_portals::FileChooserRequest::file("Choose a file")
            }
            .multiple(choice.multiple);
            let outcome = chooser
                .open(request)
                .await
                .map_err(|err| format!("The file chooser failed: {err}"))?;
            Ok(outcome
                .paths()
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect())
        })
    }
}

impl DaemonBackend {
    /// One request, with the engine's own sentence as the error on a refusal:
    /// the window shows it to the user, who does not need to be told that it
    /// came over a socket.
    async fn ask(&self, request: Request, what: &str) -> Result<compass_ipc::Response, String> {
        self.ask_within(request, what, REQUEST_TIMEOUT).await
    }

    /// [`Self::ask`], allowing `timeout` for the answer.
    async fn ask_within(
        &self,
        request: Request,
        what: &str,
        timeout: Duration,
    ) -> Result<compass_ipc::Response, String> {
        let exchange = async {
            let mut client = compass_ipc::Client::connect(self.socket.as_path())
                .await
                .map_err(|_| "The Compass engine is not running".to_owned())?;
            client
                .request(request)
                .await
                .map_err(|error| format!("Could not reach the engine: {error}"))
        };
        match tokio::time::timeout(timeout, exchange).await {
            Err(_) => Err(format!("{what} timed out")),
            Ok(Err(message)) => Err(message),
            Ok(Ok(compass_ipc::Response::Error(error))) => Err(sentence(&error.message)),
            Ok(Ok(response)) => Ok(response),
        }
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

/// A script as the launcher holds it, from the wire.
fn script_item(entry: compass_ipc::ScriptEntry) -> compass_core::script_scan::ScriptItem {
    use compass_core::script_command::{
        ArgumentDataOption, ArgumentType, OutputMode, ScriptArgument,
    };
    compass_core::script_scan::ScriptItem {
        id: entry.id,
        title: entry.title,
        subtitle: entry.subtitle,
        keywords: entry.keywords,
        mode: OutputMode::parse(&entry.mode).unwrap_or_default(),
        needs_confirmation: entry.needs_confirmation,
        path: entry.path,
        arguments: entry
            .arguments
            .into_iter()
            .map(|argument| ScriptArgument {
                argument_type: match argument.kind.as_str() {
                    "password" => ArgumentType::Password,
                    "dropdown" => ArgumentType::Dropdown,
                    _ => ArgumentType::Text,
                },
                placeholder: argument.placeholder,
                optional: argument.optional,
                // The engine encodes; the launcher only asks.
                percent_encoded: false,
                data: argument
                    .options
                    .into_iter()
                    .next()
                    .map(|(title, value)| ArgumentDataOption { title, value }),
            })
            .collect(),
    }
}

/// The snippet list in an engine answer.
fn snippets(response: compass_ipc::Response) -> Result<Vec<Snippet>, String> {
    use compass_core::snippet_store::{SnippetData, StoredExpansion};
    match response {
        compass_ipc::Response::Snippets { snippets } => Ok(snippets
            .into_iter()
            .map(|entry| Snippet {
                id: entry.id,
                name: entry.name,
                data: match (entry.text, entry.file) {
                    (Some(text), _) => SnippetData::Text { text },
                    (None, Some(file)) => SnippetData::File { file },
                    (None, None) => SnippetData::default(),
                },
                created_at: entry.created_at,
                updated_at: entry.updated_at,
                expansion: entry.keyword.map(|keyword| StoredExpansion {
                    keyword,
                    apps: entry.apps,
                    word: entry.word,
                }),
            })
            .collect()),
        other => Err(format!("Unexpected answer from the engine: {other:?}")),
    }
}

/// The shortcut list in an engine answer.
fn shortcuts(response: compass_ipc::Response) -> Result<Vec<Shortcut>, String> {
    match response {
        compass_ipc::Response::Shortcuts { shortcuts } => Ok(shortcuts
            .into_iter()
            .map(|entry| Shortcut {
                id: entry.id,
                name: entry.name,
                icon: entry.icon,
                url: entry.url,
                app: entry.app,
                open_count: entry.open_count,
                created_at: entry.created_at,
                updated_at: entry.updated_at,
                last_used_at: entry.last_used_at,
            })
            .collect()),
        other => Err(format!("Unexpected answer from the engine: {other:?}")),
    }
}

/// The engine's message with its first letter capitalised, for display.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

impl ClipboardBackend for DaemonBackend {
    fn clipboard_history(&self, query: String, limit: u32) -> BackendFuture<'_, Vec<ClipboardRow>> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardHistory { query, limit },
                    "Clipboard history",
                )
                .await?
            {
                compass_ipc::Response::ClipboardHistory { entries } => {
                    Ok(entries.into_iter().map(row).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_content(&self, id: String) -> BackendFuture<'_, ClipboardContent> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardContent { id },
                    "Reading the clipboard entry",
                )
                .await?
            {
                compass_ipc::Response::ClipboardContent { mime_type, data } => {
                    Ok(ClipboardContent { mime_type, data })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_paste(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self.ask(Request::ClipboardPaste { id }, "Pasting").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_set_pinned(&self, id: String, pinned: bool) -> BackendFuture<'_, ()> {
        let what = if pinned { "Pinning" } else { "Unpinning" };
        Box::pin(async move {
            match self
                .ask(Request::ClipboardSetPinned { id, pinned }, what)
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_remove(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ClipboardRemove { id }, "Removing the entry")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_history_of_kind(
        &self,
        query: String,
        limit: u32,
        kind: Option<ClipboardRowKind>,
    ) -> BackendFuture<'_, Vec<ClipboardRow>> {
        Box::pin(async move {
            let request = Request::ClipboardHistoryOfKind {
                query,
                limit,
                kind: kind.map(wire_kind),
            };
            match self.ask(request, "Clipboard history").await? {
                compass_ipc::Response::ClipboardHistory { entries } => {
                    Ok(entries.into_iter().map(row).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_detail(
        &self,
        id: String,
    ) -> BackendFuture<'_, compass_ui::backend::ClipboardDetail> {
        Box::pin(async move {
            match self
                .ask(Request::ClipboardDetail { id }, "Reading the entry")
                .await?
            {
                compass_ipc::Response::ClipboardDetail { detail } => {
                    Ok(compass_ui::backend::ClipboardDetail {
                        id: detail.id,
                        mime_type: detail.mime_type,
                        kind: row_kind(detail.kind),
                        size: detail.size,
                        md5: detail.md5,
                        updated_at: detail.updated_at,
                        encrypted: detail.encrypted,
                        keywords: detail.keywords,
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_set_keywords(&self, id: String, keywords: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardSetKeywords { id, keywords },
                    "Saving the keywords",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_remove_all(&self) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ClipboardRemoveAll, "Removing every entry")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_monitoring(
        &self,
        enabled: Option<bool>,
    ) -> BackendFuture<'_, compass_ui::backend::ClipboardMonitoring> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardMonitoring { enabled },
                    "Clipboard monitoring",
                )
                .await?
            {
                compass_ipc::Response::ClipboardMonitoring { supported, enabled } => {
                    Ok(compass_ui::backend::ClipboardMonitoring { supported, enabled })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

impl WindowBackend for DaemonBackend {
    fn list_windows(&self) -> BackendFuture<'_, Vec<WindowRow>> {
        Box::pin(async move {
            match self.ask(Request::ListWindows, "Listing windows").await? {
                compass_ipc::Response::Windows { windows } => {
                    Ok(windows.into_iter().map(window_row).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn activate_window(&self, id: u32) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ActivateWindow { id }, "Switching windows")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn close_window(&self, id: u32) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::CloseWindow { id }, "Closing a window")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn app_runtime(&self, id: String) -> BackendFuture<'_, compass_ui::backend::AppRuntimeInfo> {
        Box::pin(async move {
            match self
                .ask(Request::AppRuntime { id }, "Asking whether it runs")
                .await?
            {
                compass_ipc::Response::AppRuntime {
                    running,
                    frontmost,
                    windows,
                } => Ok(compass_ui::backend::AppRuntimeInfo {
                    running,
                    frontmost,
                    windows: windows.into_iter().map(window_row).collect(),
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn quit_app(&self, id: String, force: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self.ask(Request::QuitApp { id, force }, "Quitting").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn quit_window_app(&self, window: u32, force: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::QuitWindowApp { window, force }, "Quitting")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn window_manager_capabilities(
        &self,
    ) -> BackendFuture<'_, compass_core::window_switcher::Capabilities> {
        Box::pin(async move {
            match self
                .ask(
                    Request::WindowManagerCapabilities,
                    "Asking what the window manager can do",
                )
                .await?
            {
                compass_ipc::Response::WindowManagerCapabilities(caps) => {
                    Ok(compass_core::window_switcher::Capabilities {
                        workspaces: caps.workspaces,
                        fullscreen: caps.fullscreen,
                        toggle_floating: caps.floating,
                        toggle_overview: caps.overview,
                        ..compass_core::window_switcher::Capabilities::default()
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_workspaces(&self) -> BackendFuture<'_, Vec<compass_ui::backend::WorkspaceRow>> {
        Box::pin(async move {
            match self
                .ask(Request::ListWorkspaces, "Listing workspaces")
                .await?
            {
                compass_ipc::Response::Workspaces { workspaces } => Ok(workspaces
                    .into_iter()
                    .map(|workspace| compass_ui::backend::WorkspaceRow {
                        id: workspace.id,
                        name: workspace.name,
                        monitor: workspace.monitor,
                        window_count: workspace.window_count as usize,
                        apps: workspace
                            .apps
                            .into_iter()
                            .map(|app| (app.name, app.icon))
                            .collect(),
                        active: workspace.active,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn focus_workspace(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::FocusWorkspace { id }, "Switching workspaces")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn toggle_window_state(
        &self,
        toggle: compass_ui::backend::WindowToggle,
    ) -> BackendFuture<'_, ()> {
        use compass_ui::backend::WindowToggle;
        Box::pin(async move {
            let toggle = match toggle {
                WindowToggle::Fullscreen => compass_ipc::WindowToggle::Fullscreen,
                WindowToggle::Floating => compass_ipc::WindowToggle::Floating,
                WindowToggle::Overview => compass_ipc::WindowToggle::Overview,
            };
            match self
                .ask(Request::ToggleWindowState { toggle }, "Toggling")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

fn window_row(window: compass_ipc::WindowInfo) -> WindowRow {
    WindowRow {
        id: window.id,
        app_known: window.app_name.is_some(),
        app: window.app_name.unwrap_or_else(|| window.wm_class.clone()),
        title: window.title,
        wm_class: window.wm_class,
        pid: window.pid,
        can_close: window.can_close,
    }
}

const fn row_kind(kind: compass_ipc::ClipboardKind) -> ClipboardRowKind {
    match kind {
        compass_ipc::ClipboardKind::Text => ClipboardRowKind::Text,
        compass_ipc::ClipboardKind::Link => ClipboardRowKind::Link,
        compass_ipc::ClipboardKind::Image => ClipboardRowKind::Image,
        compass_ipc::ClipboardKind::File => ClipboardRowKind::File,
        compass_ipc::ClipboardKind::Unknown => ClipboardRowKind::Unknown,
    }
}

const fn wire_kind(kind: ClipboardRowKind) -> compass_ipc::ClipboardKind {
    match kind {
        ClipboardRowKind::Text => compass_ipc::ClipboardKind::Text,
        ClipboardRowKind::Link => compass_ipc::ClipboardKind::Link,
        ClipboardRowKind::Image => compass_ipc::ClipboardKind::Image,
        ClipboardRowKind::File => compass_ipc::ClipboardKind::File,
        ClipboardRowKind::Unknown => compass_ipc::ClipboardKind::Unknown,
    }
}

fn row(entry: compass_ipc::ClipboardEntry) -> ClipboardRow {
    ClipboardRow {
        id: entry.id,
        preview: entry.preview,
        kind: row_kind(entry.kind),
        pinned: entry.pinned,
        url_host: entry.url_host,
    }
}

fn preference_input(field: compass_ipc::PreferenceField) -> compass_ui::backend::PreferenceInput {
    use compass_ipc::PreferenceFieldKind;
    use compass_ui::backend::PreferenceInputKind;
    compass_ui::backend::PreferenceInput {
        kind: match field.kind {
            PreferenceFieldKind::Text => PreferenceInputKind::Text,
            PreferenceFieldKind::Password => PreferenceInputKind::Password,
            PreferenceFieldKind::Checkbox { label } => PreferenceInputKind::Checkbox { label },
            PreferenceFieldKind::Dropdown { options } => PreferenceInputKind::Dropdown { options },
            PreferenceFieldKind::Unsupported { declared } => {
                PreferenceInputKind::Unsupported { declared }
            }
        },
        value: field
            .value_json
            .and_then(|json| serde_json::from_str(&json).ok()),
        name: field.name,
        title: field.title,
        description: field.description,
        placeholder: field.placeholder,
        required: field.required,
    }
}

/// A launch's arguments, from the JSON object the engine carries them as.
fn launch_arguments(
    json: Option<String>,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    json.map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|err| format!("The launch's arguments are unreadable: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every request the engine may hold is given longer than the engine
    /// holds it, so the answer, and not the deadline, ends the wait.
    #[test]
    fn a_held_request_outlasts_the_engines_hold() {
        for (deadline, hold) in [
            (VIEW_TIMEOUT, compass_ipc::EXTENSION_VIEW_HOLD),
            (START_TIMEOUT, crate::extension_runner::LOAD_TIMEOUT),
            (PROBE_TIMEOUT, crate::global_shortcuts::PROBE_TIMEOUT),
            (
                LAUNCHER_TIMEOUT,
                crate::global_shortcuts::LAUNCHER_REQUEST_TIMEOUT,
            ),
            (
                LOGIN_START_TIMEOUT,
                compass_platform_linux::LOGIN_START_DIALOG_TIMEOUT,
            ),
        ] {
            assert!(deadline > hold, "{deadline:?} is not longer than {hold:?}");
            assert!(deadline > REQUEST_TIMEOUT);
        }
    }

    #[test]
    fn an_engine_refusal_reads_as_a_sentence() {
        assert_eq!(
            sentence("clipboard history is unavailable: no keyring"),
            "Clipboard history is unavailable: no keyring"
        );
        assert_eq!(sentence(""), "");
    }
}
