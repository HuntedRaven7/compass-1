//! The engine: the thing that actually runs.
//!
//! Everything else in this workspace is a library, and every `compass`
//! subcommand until now has been a *client* looking for a server that did not
//! exist. This module is that server.
//!
//! # Headless on purpose
//!
//! This engine has no window of its own and never opens one. It serves the
//! requests that do not need one — [`Request::Ping`], [`Request::Query`],
//! [`Request::Doctor`], [`Request::Shutdown`] — always, on any machine,
//! display or not.
//!
//! [`Request::Show`], [`Request::Hide`] and [`Request::Toggle`] are forwarded
//! to a **resident launcher window** that attached itself over the same socket
//! ([ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)).
//! With no window attached they are **refused**, with [`ErrorKind::Unsupported`]
//! and a message saying how to start one.
//!
//! Refusing matters more than it looks. `Response::Ack` means "the side effect
//! was performed"; answering `Ack` to `Show` when nothing can be shown would
//! make a client unable to tell a working engine from a windowless one, and
//! would make UI bring-up debug a lie instead of a gap.
//!
//! What it *does* serve is a genuine vertical slice — index the machine's
//! applications, rank a query against them with frecency, answer over the same
//! socket the C++ engine uses — and all of it is exercisable with no display,
//! which is why it is the part built first.

use std::sync::Arc;

use anyhow::{Context, Result};
use compass_core::{AppIndex, Config, FrecencyStore};
use compass_ipc::{
    ErrorKind, Listener, ProtocolError, QueryHit, Request, Response, SocketPath, WindowCommand,
    protocol::PROTOCOL_VERSION,
};
use tokio::sync::RwLock;

use crate::doctor;
use crate::engine::Engine;

mod app_runtime;
mod calculator;
mod clipboard;
mod default_apps;
mod extensions;
mod files;
mod integration;
mod launch;
mod media;
mod openers;
mod power;
mod rhai;
mod scripts;
mod search;
pub(crate) mod settings;
mod shell;
mod shortcuts;
mod snippets;
mod storage;
mod stores;
mod tray;
mod utilities;
mod workspaces;

pub use extensions::VIEW_HOLD_ENV;
pub(crate) use extensions::extension_storage;
use shell::WindowSlot;
pub(crate) use shell::forward;
// Only the window's tests bound the wait themselves; the engine always waits
// `forward`'s full timeout.
#[cfg(test)]
pub(crate) use shell::forward_within;

/// The engine's state, shared across connections.
///
/// `RwLock` rather than `Mutex` because queries are the hot path and they only
/// read; the write side is frecency updates and, later, reindexing.
///
/// # One service per domain
///
/// Each field is one domain's services, built by that domain's module, which
/// also holds the requests that use them: [`handle`] only routes. A request
/// that needs a second domain reaches it through the state like any other,
/// so the dependency is visible where it is taken.
pub struct EngineState {
    socket: SocketPath,
    /// The index, launch history and Search Files.
    search: search::Search,
    /// The launcher window, the Shell extension and waiting dmenus.
    shell: shell::Shell,
    /// Clipboard history and its service's controls.
    clipboard: clipboard::Clipboard,
    /// Running extension commands and their views.
    extensions: extensions::Extensions,
    /// The extension stores and the update check.
    stores: stores::Stores,
    /// Shortcuts (quicklinks).
    shortcuts: shortcuts::Shortcuts,
    /// Snippets and keyword expansion.
    snippets: snippets::Snippets,
    /// Script commands and Run Terminal Program.
    scripts: scripts::Scripts,
    /// Rhai scripts.
    rhai: rhai::Rhai,
    /// Browse Fonts, calculator history and exchange rates.
    utilities: utilities::Utilities,
    /// Both tray services.
    tray: tray::Tray,
    /// Global shortcuts and starting at login.
    integration: integration::Integration,
}

// Hand-written because `dyn FrecencyStore` is not `Debug`, and widening that
// trait to require it would push a formatting concern onto every future store
// implementation for the sake of one log line.
impl std::fmt::Debug for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineState")
            .field("applications", &self.search.index.len())
            .field("socket", &self.socket)
            .field("max_results", &self.search.max_results)
            // Deliberately not the link itself: formatting it would need the
            // lock, and `Debug` is used from log lines that must not block.
            .finish_non_exhaustive()
    }
}

impl EngineState {
    /// Builds the state by indexing this machine.
    ///
    /// A frecency store that cannot be loaded is **not** fatal: the launcher
    /// works without launch history, just less well ordered, and refusing to
    /// start over a corrupt ranking cache would be a worse failure than the one
    /// it is reporting.
    pub fn from_environment(socket: SocketPath) -> Self {
        let mut index = AppIndex::from_environment();
        tracing::info!(applications = index.len(), "indexed applications");

        // A bad config is reported and then ignored rather than fatal. Refusing
        // to start because `max_results` is misspelled would be a worse outcome
        // than starting with the default and saying so.
        let (config, config_readable) = match Config::load_with_issues() {
            Ok((config, issues)) => {
                let _ = compass_ui::theme::load_default_user_themes();
                crate::config_watch::log_issues(&crate::config_watch::check(&config, issues));
                (config, true)
            }
            Err(err) => {
                let fallback = Config::default();
                tracing::warn!(error = %err, "using default configuration");
                (fallback, false)
            }
        };
        let max_results = config.launcher().max_results();
        index.apply_root_config(&config.root_config());

        let frecency = search::open_frecency();

        let shortcuts = shortcuts::Shortcuts::open();
        let snippets = snippets::Snippets::open();
        let commands = scripts::scan_commands(&config);
        index.set_scripts(commands.items());
        if let Some(shortcuts) = &shortcuts.store {
            index.set_shortcuts(shortcuts.shortcuts().to_vec());
        }

        let rhai = rhai::Rhai::from_environment(&mut index);

        let files = search::start_file_search(&config, config_readable);

        Self {
            socket,
            search: search::Search::new(index, frecency, max_results, files),
            shell: shell::Shell::default(),
            clipboard: clipboard::Clipboard::new(
                &crate::clipboard_service::Settings::from_preferences(
                    config.provider_preferences(crate::clipboard_service::PROVIDER_ID),
                ),
            ),
            extensions: extensions::Extensions::default(),
            stores: stores::Stores::new(crate::updates::Updates::default()),
            shortcuts,
            snippets,
            rhai,
            tray: tray::Tray::default(),
            utilities: utilities::Utilities::new(
                crate::exchange_rates::ExchangeRateService::from_environment(),
            ),
            integration: integration::Integration::new(Some(Arc::new(
                compass_platform_linux::LinuxLoginStart::from_environment(),
            ))),
            scripts: scripts::Scripts::new(
                commands,
                crate::programs::default_action(config.entrypoint_preferences(
                    compass_core::commands::COMMANDS_PROVIDER_ID,
                    crate::programs::ENTRYPOINT,
                )),
            ),
        }
    }

    /// Builds state around an already-built index, for tests.
    #[must_use]
    pub fn with_index(
        index: AppIndex,
        frecency: Box<dyn FrecencyStore + Send + Sync>,
        socket: SocketPath,
        max_results: usize,
    ) -> Self {
        Self {
            socket,
            search: search::Search::new(
                index,
                frecency,
                max_results,
                crate::file_search::FileSearch::default(),
            ),
            shell: shell::Shell::default(),
            clipboard: clipboard::Clipboard::new(&crate::clipboard_service::Settings::default()),
            extensions: extensions::Extensions::default(),
            stores: stores::Stores::new(crate::updates::Updates::offline()),
            shortcuts: shortcuts::Shortcuts::default(),
            snippets: snippets::Snippets::default(),
            scripts: scripts::Scripts::new(
                crate::scripts::Scripts::default(),
                crate::programs::default_action(None),
            ),
            rhai: rhai::Rhai::default(),
            utilities: utilities::Utilities::new(
                crate::exchange_rates::ExchangeRateService::default(),
            ),
            tray: tray::Tray::default(),
            integration: integration::Integration::new(None),
        }
    }

    /// Takes `fresh`'s applications, scanned after an application directory
    /// changed, as `AppService::scanSync` ends in `appsChanged`.
    pub fn replace_applications(&mut self, fresh: AppIndex) {
        self.search.index.replace_applications(fresh);
        self.search.generation += 1;
        tracing::info!(
            applications = self.search.index.len(),
            generation = self.search.generation,
            "applications rescanned"
        );
    }

    /// Takes the installed extensions again after an extension directory
    /// changed, as the registry's debounced `requestScan` does.
    pub fn rescan_extensions(&mut self) {
        self.search.index.rescan_extensions();
        self.search.generation += 1;
        tracing::info!(
            commands = self.search.index.extensions().len(),
            generation = self.search.generation,
            "extensions rescanned"
        );
    }

    /// See [`Request::CatalogGeneration`].
    #[must_use]
    pub fn catalog_generation(&self) -> u64 {
        self.search.generation
    }

    /// Makes the Shell extension's client available to requests.
    pub fn set_shell(&mut self, shell: Arc<compass_shell::ShellClient>) {
        *self
            .rhai
            .shell
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&shell));
        self.shell.client = Some(shell);
    }

    /// Replaces the engine's Rhai scripts, loading them and listing them in
    /// root search. For tests, which bring their own paths and host.
    pub fn set_rhai_scripts(&mut self, scripts: crate::rhai_scripts::RhaiScripts) {
        self.search.index.set_rhai_scripts(scripts.reload());
        self.rhai.scripts = Arc::new(scripts);
    }

    /// The engine's Rhai scripts.
    #[must_use]
    pub fn rhai_scripts(&self) -> Arc<crate::rhai_scripts::RhaiScripts> {
        Arc::clone(&self.rhai.scripts)
    }

    /// Lists `items` in root search as the Rhai scripts, after a reload.
    pub fn set_rhai_items(&mut self, items: Vec<compass_core::rhai_scripts::RhaiScriptItem>) {
        if self.search.index.rhai_scripts() != items.as_slice() {
            self.search.index.set_rhai_scripts(items);
        }
    }

    /// Makes clipboard history available to requests.
    pub fn set_clipboard(&mut self, store: Arc<crate::clipboard_service::ClipboardStore>) {
        self.clipboard.store = Some(store);
    }

    /// The clipboard service's switch and preferences.
    #[must_use]
    pub fn clipboard_control(&self) -> Arc<crate::clipboard_service::Control> {
        Arc::clone(&self.clipboard.control)
    }

    /// The snippet store, when there is a data directory for one.
    #[must_use]
    pub fn snippet_store(&self) -> Option<&compass_core::snippet_store::SnippetStore> {
        self.snippets.store.as_ref()
    }

    /// The GNOME Shell extension's client, once connected.
    #[must_use]
    pub fn shell_client(&self) -> Option<Arc<compass_shell::ShellClient>> {
        self.shell.client.clone()
    }

    /// The application index.
    #[must_use]
    pub fn app_index(&self) -> &AppIndex {
        &self.search.index
    }

    /// Keyword expansion and its input server, once started.
    #[must_use]
    pub fn expander(&self) -> Option<Arc<crate::snippet_expansion::Expander>> {
        self.snippets.expander.clone()
    }

    /// Makes keyword expansion reachable from requests.
    pub fn set_expander(&mut self, expander: Arc<crate::snippet_expansion::Expander>) {
        self.snippets.expander = Some(expander);
    }

    /// The slot holding the attached launcher window.
    ///
    /// Cloned rather than borrowed because the attach callback outlives any
    /// borrow of the state: it runs on the connection's own task for as long
    /// as that window lives.
    #[must_use]
    pub fn window_slot(&self) -> WindowSlot {
        Arc::clone(&self.shell.window)
    }

    /// Compass's own tray icon's control.
    #[must_use]
    pub fn tray_icon(&self) -> Arc<crate::tray_icon::Control> {
        Arc::clone(&self.tray.icon)
    }

    /// The global shortcuts' service control.
    #[must_use]
    pub fn global_shortcuts(&self) -> Arc<crate::global_shortcuts::Control> {
        Arc::clone(&self.integration.global_shortcuts)
    }

    /// The extension stores, and the trigger of the installer of the
    /// extensions `compass.json` declares.
    #[must_use]
    pub fn extension_sync(
        &self,
    ) -> (
        Arc<crate::stores::Stores>,
        Arc<crate::extension_sync::Trigger>,
    ) {
        (
            Arc::clone(&self.stores.stores),
            Arc::clone(&self.stores.sync),
        )
    }

    /// Starting Compass at login, when this engine has a way to.
    #[must_use]
    pub fn login_start(&self) -> Option<Arc<dyn compass_platform::LoginStarter>> {
        self.integration.login_start.clone()
    }

    /// Starts Compass at login through `starter`, for tests.
    pub fn set_login_start(&mut self, starter: Arc<dyn compass_platform::LoginStarter>) {
        self.integration.login_start = Some(starter);
    }

    /// The application and root item index.
    #[must_use]
    pub fn index(&self) -> &AppIndex {
        &self.search.index
    }

    /// Number of indexed applications.
    #[must_use]
    pub fn len(&self) -> usize {
        self.search.index.len()
    }

    /// Whether nothing was indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.search.index.is_empty()
    }

    /// Ranks `text` against the index.
    ///
    /// # Hits can be ordered "out of order" by score, and that is correct
    ///
    /// Ordering comes from the *combined* score — match quality plus the
    /// frecency boost — while the score reported on the wire is the match score
    /// alone, because that is what the protocol documents and what a client can
    /// interpret. So a frequently launched item with score 90 legitimately
    /// appears above a never-launched one with 95. A client that sorts by the
    /// reported score would undo the ranking; it should preserve the order it
    /// is given.
    #[must_use]
    pub fn query(&self, text: &str) -> Vec<QueryHit> {
        self.search.query(text, &self.extensions.launches)
    }
}

/// The application database as a request sees it: the index, the
/// associations read now, and the runtime to launch on.
fn engine_apps_now(state: &EngineState) -> crate::extension_apps::EngineApps {
    crate::extension_apps::EngineApps::new(
        &state.search.index,
        compass_xdg::mimeapps::Lists::from_environment(),
        tokio::runtime::Handle::current(),
    )
}

/// A root item launched as `cmd launch` launches it, with no arguments: what
/// a command's global shortcut does.
pub(crate) async fn launch_entrypoint(state: &Arc<RwLock<EngineState>>, id: String) -> Response {
    launch::launch_command(state, id, &[], None, None).await
}

/// The applications an engine request resolves openers against.
async fn engine_apps(state: &Arc<RwLock<EngineState>>) -> crate::extension_apps::EngineApps {
    crate::extension_apps::EngineApps::new(
        &state.read().await.search.index,
        compass_xdg::mimeapps::Lists::from_environment(),
        tokio::runtime::Handle::current(),
    )
}

/// Answers one request.
///
/// Separated from the serve loop so the whole request surface is testable
/// without a socket, and so the serve loop stays about lifecycle.
///
/// Only routing lives here: each domain's requests are answered beside the
/// services they use, in that domain's module.
pub async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::Ping => Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: std::process::id(),
        },

        Request::Doctor => {
            let (socket, engine) = {
                let state = state.read().await;
                (state.socket.clone(), Engine::Rust)
            };
            Response::DoctorReport {
                checks: doctor::run_on_this_machine(&socket, engine).await.checks,
            }
        }

        request @ (Request::Query { .. }
        | Request::RecordLaunch { .. }
        | Request::RootItemEdit { .. }
        | Request::SearchFiles { .. }
        | Request::OpenFile { .. }
        | Request::CatalogGeneration) => search::handle(state, request).await,
        request @ (Request::ClipboardHistory { .. }
        | Request::ClipboardHistoryOfKind { .. }
        | Request::ClipboardDetail { .. }
        | Request::ClipboardSetKeywords { .. }
        | Request::ClipboardRemoveAll
        | Request::ClipboardMonitoring { .. }
        | Request::ClipboardContent { .. }
        | Request::PasteText { .. }
        | Request::CopyText { .. }
        | Request::ClipboardStatus
        | Request::ClipboardSetPinned { .. }
        | Request::ClipboardRemove { .. }
        | Request::ClipboardPaste { .. }) => clipboard::handle(state, request).await,
        request @ (Request::ListWindows
        | Request::ActivateWindow { .. }
        | Request::CloseWindow { .. }
        | Request::Dmenu { .. }
        | Request::DmenuFetch { .. }
        | Request::DmenuChoose { .. }
        | Request::Toggle
        | Request::Show
        | Request::Hide) => shell::handle(state, request).await,
        request @ (Request::ListShortcuts
        | Request::SaveShortcut { .. }
        | Request::RemoveShortcut { .. }
        | Request::OpenShortcut { .. }
        | Request::ExpandShortcut { .. }) => shortcuts::handle(state, request).await,
        request @ (Request::ListSnippets
        | Request::SaveSnippet { .. }
        | Request::RemoveSnippet { .. }
        | Request::InputServerStatus
        | Request::SetInputServerEnabled { .. }
        | Request::PreviewSnippet { .. }
        | Request::ExpandSnippet { .. }
        | Request::PasteSnippet { .. }) => snippets::handle(state, request).await,
        request @ (Request::ListScripts
        | Request::ScriptIcons
        | Request::RunScript { .. }
        | Request::ScriptOutput { .. }
        | Request::StopScript { .. }
        | Request::ListPrograms
        | Request::RunProgram { .. }) => scripts::handle(state, request).await,
        request @ (Request::ListScriptGrants
        | Request::RevokeScriptGrant { .. }
        | Request::ListRhaiScripts) => rhai::handle(state, request).await,
        request @ (Request::RunExtensionCommand { .. }
        | Request::ExtensionView { .. }
        | Request::ExtensionEvent { .. }
        | Request::ExtensionAlertAnswer { .. }
        | Request::ExtensionAlertRemember { .. }
        | Request::SetExtensionPreferences { .. }
        | Request::ExtensionLaunchFetch { .. }
        | Request::ExtensionSubtitles
        | Request::ExtensionPreferences { .. }
        | Request::ExtensionPop { .. }
        | Request::CloseExtension { .. }
        | Request::OAuthRedirect { .. }) => extensions::handle(state, request).await,
        request @ (Request::UpdateStatus
        | Request::SkipUpdate { .. }
        | Request::StoreBrowse { .. }
        | Request::StoreExtension { .. }
        | Request::StoreInstall { .. }
        | Request::StoreUninstall { .. }
        | Request::OpenUrl { .. }
        | Request::CreateExtension { .. }) => stores::handle(state, request).await,
        request @ (Request::ExchangeRates
        | Request::RefreshExchangeRates
        | Request::ListFonts
        | Request::FontSpecimen { .. }) => utilities::handle(state, request).await,
        request @ (Request::RunMediaCommand { .. }
        | Request::RunMediaCommandWith { .. }
        | Request::ListMediaPlayers
        | Request::ControlMediaPlayer { .. }) => media::handle(state, request).await,
        request @ (Request::ShortcutCapture { .. }
        | Request::ProbeShortcut { .. }
        | Request::LauncherShortcut { .. }
        | Request::LoginStart { .. }) => integration::handle(state, request).await,

        Request::ListCommands => {
            let state = state.read().await;
            Response::Commands {
                commands: launch::commands(&state.search.index),
            }
        }
        Request::LaunchCommand {
            id,
            args,
            cwd,
            query,
        } => launch::launch_command(state, id, &args, cwd, query).await,
        Request::LaunchApp {
            id,
            args,
            new_instance,
        } => launch::launch_app(state, &id, &args, new_instance).await,
        Request::DescribeWindow => launch::describe_window(state).await,
        Request::FsQuery {
            query,
            limit,
            category,
        } => launch::fs_query(state, query, limit, category).await,
        Request::AppRuntime { id } => app_runtime::describe(state, &id).await,
        Request::QuitApp { id, force } => app_runtime::quit(state, &id, force).await,
        Request::QuitWindowApp { window, force } => {
            app_runtime::quit_window_app(state, window, force).await
        }
        request @ (Request::CalculatorHistory { .. }
        | Request::AddCalculatorRecord { .. }
        | Request::EditCalculatorHistory { .. }) => calculator::handle(state, request).await,
        request @ (Request::LocalStorageNamespaces
        | Request::LocalStorageItems { .. }
        | Request::OAuthTokenSets
        | Request::RemoveOAuthTokenSet { .. }) => storage::handle(state, request).await,
        request @ (Request::TrayItems
        | Request::TrayActivate { .. }
        | Request::TrayMenu { .. }
        | Request::TrayTriggerMenu { .. }) => tray::handle(state, request).await,
        request @ (Request::WindowManagerCapabilities
        | Request::ListWorkspaces
        | Request::FocusWorkspace { .. }
        | Request::ToggleWindowState { .. }) => workspaces::handle(state, request).await,
        Request::ListOpeners { target } => {
            let apps = engine_apps(state).await;
            Response::Openers {
                apps: openers::openers(&apps, &target),
            }
        }
        Request::OpenWith { app, target } => {
            let apps = engine_apps(state).await;
            openers::open_with(&apps, &app, &target)
        }
        request @ (Request::FileActions { .. }
        | Request::CopyFile { .. }
        | Request::RunExecutable { .. }
        | Request::SetWallpaper { .. }) => files::handle(state, request).await,
        request @ (Request::SetSetting { .. }
        | Request::SetProviderEnabled { .. }
        | Request::SetFont { .. }
        | Request::SetTheme { .. }) => settings::handle(state, request).await,
        Request::RunPowerCommand { id } => power::run_power_command(&id).await,
        Request::OpenDeeplink { url } => {
            if let Some(link) = compass_core::root_items::parse_launch_link(&url) {
                return launch::open_launch_link(state, url, link).await;
            }
            if compass_core::settings_catalog::parse_settings_link(&url).is_some() {
                let slot = state.read().await.window_slot();
                return forward(&slot, WindowCommand::Deeplink(url), "open the settings").await;
            }
            match compass_core::develop_link::parse_develop_link(&url) {
                Some(Ok(link)) => return stores::develop(state, &link).await,
                Some(Err(usage)) => {
                    return Response::Error(ProtocolError::new(ErrorKind::BadRequest, usage));
                }
                None => {}
            }
            match compass_core::store_listing::parse_extension_link(&url) {
                Some(Ok(_)) => {
                    let slot = state.read().await.window_slot();
                    forward(&slot, WindowCommand::Deeplink(url), "open a deeplink").await
                }
                Some(Err(usage)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, usage))
                }
                None => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("Compass does not handle this deeplink yet: {url}"),
                )),
            }
        }
        Request::ListDefaultApps { kind } => default_apps::list_default_apps(state, kind).await,
        Request::SetDefaultApp { kind, id } => {
            default_apps::set_default_app(state, kind, &id).await
        }

        // Handled by the serve loop, which owns the shutdown signal; reaching
        // here means the loop did not intercept it.
        Request::Shutdown => Response::ShuttingDown,

        // Accepting is the whole decision: the transport hands the connection
        // over as a `WindowLink`, and `run` writes the acceptance once the link
        // is in the slot. The engine never refuses an attach — see `WindowSlot`
        // for why replacing beats refusing.
        Request::AttachWindow => Response::WindowAttached,

        // An outcome is a *reply* on an attached connection, never a request.
        // Arriving here means a peer sent one on an ordinary connection, which
        // is a confused client rather than a window.
        Request::WindowOutcome(outcome) => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            format!(
                "{outcome:?} is a reply to a pushed window command, not a request;                  send AttachWindow first and answer the commands that follow"
            ),
        )),
    }
}

/// Binds the socket and serves until shutdown.
///
/// Returns when a client sends [`Request::Shutdown`] or the process is asked to
/// terminate. The socket file is removed on the way out by [`Listener`]'s drop.
///
/// `hotkey` asks the engine to bind the global launcher shortcut. Turning it
/// off is not only a test affordance: on GNOME, binding means a permission
/// prompt, and a user whose compositor already binds a key to `compass toggle`
/// should not be asked for one they will not use.
pub async fn run(socket: &SocketPath, hotkey: bool) -> Result<()> {
    // Before binding, not after: on the `/tmp` fallback the directory may
    // already exist and belong to someone else, and `DirBuilder::recursive`
    // adopts a directory rather than correcting it. See #88.
    socket
        .ensure_private_parent()
        .with_context(|| format!("checking the engine socket directory for {socket}"))?;

    let listener = Listener::bind(socket.as_path())
        .await
        .with_context(|| format!("binding the engine socket at {socket}"))?;
    crate::logs::activate_engine_log();

    let state = Arc::new(RwLock::new(EngineState::from_environment(socket.clone())));
    tracing::info!(socket = %socket, "engine listening");

    // A channel rather than a flag: `Shutdown` must be answered *before* the
    // loop stops, or the client sees its connection drop and reports a crash
    // instead of a clean stop.
    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::channel::<()>(1);

    let window_slot = state.read().await.window_slot();

    // Detached: the hotkey is a convenience, the socket is the contract. A
    // portal that never answers must not keep the engine from listening, so
    // this is spawned rather than awaited or raced against the serve loop.
    // Detached for the same reason: a keyring that is slow or absent costs
    // clipboard history, never the socket.
    tokio::spawn(crate::clipboard_service::run(Arc::clone(&state)));

    // Reading every font's character map takes a moment on a machine with
    // many fonts; done once, after start-up has settled, so Browse Fonts
    // answers at once.
    {
        let fonts = Arc::clone(&state.read().await.utilities.fonts);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            utilities::installed_fonts(&fonts).await;
        });
    }

    // Rhai scripts' hot reload.
    tokio::spawn(crate::rhai_scripts::watch(Arc::clone(&state)));

    // Other applications' tray icons, followed from start-up as the C++
    // host does, so Search Tray lists them at once.
    {
        let tray = Arc::clone(&state.read().await.tray.host);
        tokio::spawn(async move { tray.start().await });
    }

    // Compass's own tray icon, while `tray.enabled`; its Quit stops the
    // engine as `Shutdown` does.
    tokio::spawn(crate::tray_icon::run(Arc::clone(&state), stop_tx.clone()));

    // Applications installed or removed while the engine runs.
    tokio::spawn(crate::catalog_watch::watch_applications(Arc::clone(&state)));
    // An extension a developer builds into place.
    tokio::spawn(crate::catalog_watch::watch_extensions(Arc::clone(&state)));
    // `compass.json` edited by hand or by `compass theme set`.
    tokio::spawn(crate::config_watch::run(Arc::clone(&state)));
    // The extensions `compass.json` declares, installed now and after each
    // change to it; detached, so a slow store never holds up the socket.
    tokio::spawn(crate::extension_sync::run(Arc::clone(&state)));

    // Snippet keyword expansion: the input server, when `input_server.enabled`.
    {
        let enabled = Config::load()
            .map(|config| config.input_server().enabled())
            .unwrap_or(compass_core::config::DEFAULT_INPUT_SERVER_ENABLED);
        tokio::spawn(crate::snippet_expansion::run(Arc::clone(&state), enabled));
    }
    crate::paste::install(&state);

    // KWin's tracker, for window switching on Plasma: detached, like the
    // Shell client, so a slow KWin never holds up the socket.
    tokio::spawn(crate::wlroots::start_kwin());

    // The Shell extension, for window switching. Connecting only fails with
    // no session bus at all; an absent extension is reported per request.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            match compass_shell::ShellClient::connect_session().await {
                Ok(client) => state.write().await.set_shell(Arc::new(client)),
                // Only GNOME switches windows through the Shell extension; the
                // wlroots compositors and KWin list them without the bus.
                Err(err)
                    if compass_wayland::compositor::desktop_is_gnome(
                        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
                    ) =>
                {
                    tracing::warn!(error = %err,
                        "no session bus, so the GNOME Shell extension cannot be reached; window switching is unavailable")
                }
                Err(err) => tracing::info!(error = %err,
                    "no session bus; the GNOME Shell extension is not used on this desktop"),
            }
        });
    }

    if hotkey {
        tokio::spawn(crate::global_shortcuts::run(Arc::clone(&state)));
    } else {
        tracing::info!("not binding the global shortcuts (--no-hotkey)");
        state
            .read()
            .await
            .global_shortcuts()
            .set_launcher(compass_core::launcher_shortcut::LauncherShortcut::Off);
    }

    let serving = {
        let state = Arc::clone(&state);
        listener.serve_with_shutdown_attach(
            move |request| {
                let state = Arc::clone(&state);
                let stop_tx = stop_tx.clone();
                async move {
                    if matches!(request, Request::Shutdown) {
                        tracing::info!("shutdown requested by a client");
                        // Buffered, so the response is written first and this
                        // never blocks the handler.
                        let _ = stop_tx.try_send(());
                        return Response::ShuttingDown;
                    }
                    handle(&state, request).await
                }
            },
            move |link| {
                let window_slot = Arc::clone(&window_slot);
                async move {
                    tracing::info!("a launcher window attached");
                    // Replaces any previous window; see `WindowSlot`. The old
                    // link drops here, closing that window's socket.
                    let mut slot = window_slot.lock().await;
                    let replaced = slot.replace(link).is_some();
                    if replaced {
                        tracing::info!("the previous launcher window was replaced");
                    }
                    // Told only now that it is in the slot, so a request sent
                    // the moment its attach returns finds it.
                    if let Some(link) = slot.as_mut()
                        && let Err(err) = link.confirm().await
                    {
                        tracing::info!(error = %err, "the launcher window left while attaching");
                        *slot = None;
                    }
                }
            },
            async move {
                let _ = stop_rx.recv().await;
            },
        )
    };

    let served = serving.await.context("serving the engine socket");
    crate::wlroots::stop_kwin().await;
    served?;
    tracing::info!("engine stopped");
    Ok(())
}
