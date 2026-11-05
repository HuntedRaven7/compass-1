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
use compass_core::{AppIndex, Config, FrecencyStore, JsonFrecencyStore, SystemClock};
use compass_ipc::{
    ErrorKind, Listener, ProtocolError, QueryHit, Request, Response, SocketPath, WindowCommand,
    WindowLink, WindowOutcome, protocol::PROTOCOL_VERSION,
};
use tokio::sync::{Mutex, RwLock};

use crate::doctor;
use crate::engine::Engine;

mod app_runtime;
mod calculator;
mod files;
mod launch;
mod openers;
pub(crate) mod settings;
mod storage;
mod workspaces;

/// The at-most-one launcher window this engine drives.
///
/// # Last window wins
///
/// A second `compass ui` **replaces** the first rather than being refused.
/// Dropping the old [`WindowLink`] closes its socket, so that window's
/// `next_command` returns `None` and it exits its loop.
///
/// The alternative — refuse the newcomer — reads better until you ask what
/// happens when a window wedges without closing its socket: the engine would
/// hold a link that answers nothing and refuse every replacement, and the only
/// way out would be restarting the daemon. Replacing has no state that can get
/// stuck. It also means the handler needs no reservation between answering
/// `WindowAttached` and the link actually arriving, which is a window (however
/// narrow) in which an attach that died mid-handshake could strand the slot.
///
/// # Death is noticed on the next push, not before
///
/// Nothing polls the link. A window that dies is discovered when the engine
/// next tries to push to it, at which point the slot is cleared and the request
/// is refused with [`no_window`] — correctly, since by then there is none.
/// Proactively watching would mean a second reader on a socket whose only
/// reader is [`WindowLink::push`]'s reply, so it would have to be built into
/// the link rather than bolted beside it.
type WindowSlot = Arc<Mutex<Option<WindowLink>>>;

/// Where launch history lives, under `$XDG_DATA_HOME`.
///
/// Returns `None` when there is no data home to put it in, which is the
/// read-only-session case the caller degrades into an in-memory store for.
fn frecency_path() -> Option<std::path::PathBuf> {
    Some(compass_core::xdg_dirs::data_home()?.join("compass/frecency.json"))
}

/// The engine's state, shared across connections.
///
/// `RwLock` rather than `Mutex` because queries are the hot path and they only
/// read; the write side is frecency updates and, later, reindexing.
pub struct EngineState {
    index: AppIndex,
    frecency: Box<dyn FrecencyStore + Send + Sync>,
    socket: SocketPath,
    /// How many hits a query answers with. `launcher.max_results` from the
    /// user's config, so the wire honours the same limit the UI would.
    max_results: usize,
    /// The attached launcher window, if one is.
    window: WindowSlot,
    /// Clipboard history, once [`crate::clipboard_service::run`] has opened
    /// it. `None` until then, and for good when there is no keyring.
    clipboard: Option<Arc<crate::clipboard_service::ClipboardStore>>,
    /// The clipboard service's switch and preferences, shared with its
    /// recording loop and eviction timer.
    clipboard_control: Arc<crate::clipboard_service::Control>,
    /// The GNOME Shell extension's client, once the session bus answered.
    /// `None` until then, and for good without a session bus.
    shell: Option<Arc<compass_shell::ShellClient>>,
    /// Running extension view commands, which the launcher follows.
    views: Arc<crate::extension_runner::Views>,
    /// Search Files, and the file indexer it supervises.
    files: Arc<crate::file_search::FileSearch>,
    /// Shortcuts (quicklinks). `None` without a data directory to keep them
    /// in, and in tests that build the state around an index.
    shortcuts: Option<compass_core::shortcut_service::ShortcutService>,
    /// Snippets, `None` on the same terms as `shortcuts`.
    snippets: Option<compass_core::snippet_store::SnippetStore>,
    /// Script commands, as the last scan found them.
    scripts: crate::scripts::Scripts,
    /// Script runs the launcher follows.
    script_runs: Arc<crate::scripts::Runs>,
    /// Run Terminal Program's `default-action` preference.
    run_program_default: String,
    /// `compass dmenu` lists waiting on the launcher.
    dmenus: Arc<crate::dmenu::Pending>,
    /// Browse Fonts' families, read once.
    fonts: Arc<tokio::sync::OnceCell<Vec<compass_core::font_service::BrowsedFamily>>>,
    /// Rhai scripts, and their open views.
    rhai: Arc<crate::rhai_scripts::RhaiScripts>,
    /// The Shell extension's client for the scripts' clipboard, once
    /// connected.
    shell_slot: crate::rhai_host::ShellSlot,
    /// The extension stores.
    stores: Arc<crate::stores::Stores>,
    /// Wakes the installer of the extensions `compass.json` declares.
    extension_sync: Arc<crate::extension_sync::Trigger>,
    /// The update check.
    updates: Arc<crate::updates::Updates>,
    /// Snippet keyword expansion and its input server, once started.
    expander: Option<Arc<crate::snippet_expansion::Expander>>,
    /// Launches extensions asked for, and their commands' subtitle overrides.
    launches: Arc<crate::extension_commands::Launches>,
    /// How many times a directory watch has rescanned the catalog; see
    /// [`Request::CatalogGeneration`].
    catalog_generation: u64,
    /// The calculator history's database, once the keyring opened it.
    calculator: Arc<tokio::sync::Mutex<Option<crate::extension_runner::Storage>>>,
    /// Other applications' tray icons (`SniTrayHost`).
    tray: Arc<crate::tray_host::TrayHost>,
    /// Compass's own tray icon (`TrayServiceLinux`): `tray.enabled`.
    tray_icon: Arc<crate::tray_icon::Control>,
    /// The global shortcuts' service: rebinding them, and the recorder's
    /// capture.
    global_shortcuts: Arc<crate::global_shortcuts::Control>,
    /// The calculator's exchange rates.
    exchange_rates: Arc<crate::exchange_rates::ExchangeRateService>,
}

// Hand-written because `dyn FrecencyStore` is not `Debug`, and widening that
// trait to require it would push a formatting concern onto every future store
// implementation for the sake of one log line.
impl std::fmt::Debug for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineState")
            .field("applications", &self.index.len())
            .field("socket", &self.socket)
            .field("max_results", &self.max_results)
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

        let frecency = Self::open_frecency();

        let shortcuts = crate::shortcuts::data_dir().map(|dir| crate::shortcuts::open(&dir));
        let snippets = crate::shortcuts::data_dir().map(|dir| crate::snippets::open(&dir));
        let mut scripts = crate::scripts::Scripts::new(
            compass_core::script_scan::scan_directories(
                &crate::scripts::custom_directories(
                    config.provider_preferences(crate::scripts::PREFERENCES_PROVIDER_ID),
                ),
                &crate::scripts::default_directories(),
            ),
            crate::shortcuts::data_dir().map(|dir| dir.join(crate::scripts::METADATA_FILE)),
        );
        scripts.rescan();
        index.set_scripts(scripts.items());
        if let Some(shortcuts) = &shortcuts {
            index.set_shortcuts(shortcuts.shortcuts().to_vec());
        }

        let shell_slot = crate::rhai_host::ShellSlot::default();
        let rhai = {
            let apps = tokio::runtime::Handle::try_current().ok().map(|handle| {
                crate::extension_apps::EngineApps::new(
                    &index,
                    compass_xdg::mimeapps::Lists::from_environment(),
                    handle,
                )
            });
            let host = crate::rhai_host::EngineHost::new(
                Arc::clone(&shell_slot),
                apps,
                compass_core::xdg_dirs::data_home().map(|home| home.join("compass")),
            );
            let rhai = crate::rhai_scripts::RhaiScripts::new(
                crate::rhai_scripts::Config::from_environment(),
                Arc::new(host),
            );
            rhai.create_user_dir();
            index.set_rhai_scripts(rhai.reload());
            Arc::new(rhai)
        };

        // Started with the engine, as `FileExtension::initialized` starts
        // it, so the index is warm by the time anyone searches it.
        let file_settings = if config_readable {
            settings::file_indexing_settings(&config)
        } else {
            Err("the configuration could not be read".to_owned())
        };
        let files = match file_settings {
            Ok(settings) => crate::file_search::FileSearch::start(settings),
            Err(reason) => {
                tracing::warn!(%reason, "file indexing is disabled until its configuration is valid");
                crate::file_search::FileSearch::without_indexer()
            }
        };

        Self {
            index,
            frecency,
            socket,
            max_results,
            window: WindowSlot::default(),
            clipboard: None,
            clipboard_control: Arc::new(crate::clipboard_service::Control::new(
                &crate::clipboard_service::Settings::from_preferences(
                    config.provider_preferences(crate::clipboard_service::PROVIDER_ID),
                ),
            )),
            shell: None,
            views: Arc::default(),
            files: Arc::new(files),
            shortcuts,
            snippets,
            scripts,
            script_runs: Arc::default(),
            dmenus: Arc::default(),
            fonts: Arc::default(),
            rhai,
            shell_slot,
            stores: Arc::default(),
            extension_sync: Arc::default(),
            updates: Arc::default(),
            expander: None,
            launches: Arc::default(),
            catalog_generation: 0,
            calculator: Arc::default(),
            tray: Arc::default(),
            tray_icon: Arc::default(),
            global_shortcuts: Arc::default(),
            exchange_rates: Arc::new(
                crate::exchange_rates::ExchangeRateService::from_environment(),
            ),
            run_program_default: crate::programs::default_action(config.entrypoint_preferences(
                compass_core::commands::COMMANDS_PROVIDER_ID,
                crate::programs::ENTRYPOINT,
            )),
        }
    }

    /// Opens the launch-history store, degrading to in-memory.
    ///
    /// Not fatal on failure: the launcher works without launch history, just
    /// less well ordered, and refusing to start over a corrupt ranking cache
    /// would be a worse failure than the one it is reporting.
    fn open_frecency() -> Box<dyn FrecencyStore + Send + Sync> {
        let clock = std::sync::Arc::new(SystemClock);
        match frecency_path() {
            Some(path) => match JsonFrecencyStore::open(&path) {
                Ok(store) => Box::new(store),
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(), error = %err,
                        "launch history unreadable; ranking by match score alone this session"
                    );
                    Box::new(JsonFrecencyStore::in_memory(clock))
                }
            },
            None => {
                tracing::warn!("no XDG data home; launch history will not persist");
                Box::new(JsonFrecencyStore::in_memory(clock))
            }
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
            index,
            frecency,
            socket,
            max_results,
            window: WindowSlot::default(),
            clipboard: None,
            clipboard_control: Arc::new(crate::clipboard_service::Control::new(
                &crate::clipboard_service::Settings::default(),
            )),
            shell: None,
            views: Arc::default(),
            files: Arc::default(),
            shortcuts: None,
            snippets: None,
            scripts: crate::scripts::Scripts::default(),
            script_runs: Arc::default(),
            run_program_default: crate::programs::default_action(None),
            dmenus: Arc::default(),
            fonts: Arc::default(),
            rhai: Arc::default(),
            shell_slot: crate::rhai_host::ShellSlot::default(),
            stores: Arc::default(),
            extension_sync: Arc::default(),
            updates: Arc::new(crate::updates::Updates::offline()),
            expander: None,
            launches: Arc::default(),
            catalog_generation: 0,
            calculator: Arc::default(),
            tray: Arc::default(),
            tray_icon: Arc::default(),
            global_shortcuts: Arc::default(),
            exchange_rates: Arc::default(),
        }
    }

    /// Takes `fresh`'s applications, scanned after an application directory
    /// changed, as `AppService::scanSync` ends in `appsChanged`.
    pub fn replace_applications(&mut self, fresh: AppIndex) {
        self.index.replace_applications(fresh);
        self.catalog_generation += 1;
        tracing::info!(
            applications = self.index.len(),
            generation = self.catalog_generation,
            "applications rescanned"
        );
    }

    /// Takes the installed extensions again after an extension directory
    /// changed, as the registry's debounced `requestScan` does.
    pub fn rescan_extensions(&mut self) {
        self.index.rescan_extensions();
        self.catalog_generation += 1;
        tracing::info!(
            commands = self.index.extensions().len(),
            generation = self.catalog_generation,
            "extensions rescanned"
        );
    }

    /// See [`Request::CatalogGeneration`].
    #[must_use]
    pub fn catalog_generation(&self) -> u64 {
        self.catalog_generation
    }

    /// Makes the Shell extension's client available to requests.
    pub fn set_shell(&mut self, shell: Arc<compass_shell::ShellClient>) {
        *self
            .shell_slot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&shell));
        self.shell = Some(shell);
    }

    /// Replaces the engine's Rhai scripts, loading them and listing them in
    /// root search. For tests, which bring their own paths and host.
    pub fn set_rhai_scripts(&mut self, scripts: crate::rhai_scripts::RhaiScripts) {
        self.index.set_rhai_scripts(scripts.reload());
        self.rhai = Arc::new(scripts);
    }

    /// The engine's Rhai scripts.
    #[must_use]
    pub fn rhai_scripts(&self) -> Arc<crate::rhai_scripts::RhaiScripts> {
        Arc::clone(&self.rhai)
    }

    /// Lists `items` in root search as the Rhai scripts, after a reload.
    pub fn set_rhai_items(&mut self, items: Vec<compass_core::rhai_scripts::RhaiScriptItem>) {
        if self.index.rhai_scripts() != items.as_slice() {
            self.index.set_rhai_scripts(items);
        }
    }

    /// Makes clipboard history available to requests.
    pub fn set_clipboard(&mut self, store: Arc<crate::clipboard_service::ClipboardStore>) {
        self.clipboard = Some(store);
    }

    /// The clipboard service's switch and preferences.
    #[must_use]
    pub fn clipboard_control(&self) -> Arc<crate::clipboard_service::Control> {
        Arc::clone(&self.clipboard_control)
    }

    /// The snippet store, when there is a data directory for one.
    #[must_use]
    pub fn snippet_store(&self) -> Option<&compass_core::snippet_store::SnippetStore> {
        self.snippets.as_ref()
    }

    /// The GNOME Shell extension's client, once connected.
    #[must_use]
    pub fn shell_client(&self) -> Option<Arc<compass_shell::ShellClient>> {
        self.shell.clone()
    }

    /// The application index.
    #[must_use]
    pub fn app_index(&self) -> &AppIndex {
        &self.index
    }

    /// Keyword expansion and its input server, once started.
    #[must_use]
    pub fn expander(&self) -> Option<Arc<crate::snippet_expansion::Expander>> {
        self.expander.clone()
    }

    /// Makes keyword expansion reachable from requests.
    pub fn set_expander(&mut self, expander: Arc<crate::snippet_expansion::Expander>) {
        self.expander = Some(expander);
    }

    /// The slot holding the attached launcher window.
    ///
    /// Cloned rather than borrowed because the attach callback outlives any
    /// borrow of the state: it runs on the connection's own task for as long
    /// as that window lives.
    #[must_use]
    pub fn window_slot(&self) -> WindowSlot {
        Arc::clone(&self.window)
    }

    /// Compass's own tray icon's control.
    #[must_use]
    pub fn tray_icon(&self) -> Arc<crate::tray_icon::Control> {
        Arc::clone(&self.tray_icon)
    }

    /// The global shortcuts' service control.
    #[must_use]
    pub fn global_shortcuts(&self) -> Arc<crate::global_shortcuts::Control> {
        Arc::clone(&self.global_shortcuts)
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
        (Arc::clone(&self.stores), Arc::clone(&self.extension_sync))
    }

    /// The application and root item index.
    #[must_use]
    pub fn index(&self) -> &AppIndex {
        &self.index
    }

    /// Number of indexed applications.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether nothing was indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
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
        self.index
            .search_root_all(text, Some(self.frecency.as_ref()))
            .into_iter()
            .take(self.max_results)
            .map(|hit| match hit {
                compass_core::RootHit::App(ranked) => app_hit(&ranked),
                compass_core::RootHit::Command {
                    command,
                    match_score,
                } => QueryHit {
                    id: command.id(),
                    title: command.title.to_owned(),
                    subtitle: Some(command.subtitle.to_owned()),
                    score: match_score,
                },
                compass_core::RootHit::Extension {
                    command,
                    match_score,
                } => QueryHit {
                    id: command.id.clone(),
                    title: command.title.clone(),
                    subtitle: Some(
                        self.launches
                            .subtitle(&command.id)
                            .unwrap_or_else(|| command.extension_title.clone()),
                    ),
                    score: match_score,
                },
                compass_core::RootHit::Script {
                    script,
                    match_score,
                } => QueryHit {
                    id: compass_core::root_items::entrypoint_id(
                        compass_core::script_scan::SCRIPTS_PROVIDER_ID,
                        &script.id,
                    ),
                    title: script.title.clone(),
                    subtitle: Some(script.subtitle.clone()),
                    score: match_score,
                },
                compass_core::RootHit::RhaiScript {
                    script,
                    match_score,
                } => QueryHit {
                    id: script.entrypoint_id(),
                    title: script.title.clone(),
                    subtitle: Some(script.subtitle().to_owned()),
                    score: match_score,
                },
                compass_core::RootHit::Shortcut {
                    shortcut,
                    match_score,
                } => QueryHit {
                    id: compass_core::root_items::entrypoint_id(
                        compass_core::shortcut::SHORTCUTS_PROVIDER_ID,
                        &shortcut.id,
                    ),
                    title: shortcut.name.clone(),
                    subtitle: Some("Shortcut".to_owned()),
                    score: match_score,
                },
            })
            .collect()
    }
}

/// Runs a Power Management command through logind, or the desktop's session
/// manager for logout, answering with the command's own sentences.
async fn run_power_command(id: &str) -> Response {
    use compass_power::{Action, PowerManager};
    use std::os::unix::fs::MetadataExt;
    let Some(command) = compass_core::power_commands::command(id) else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no power command has that id",
        ));
    };
    let cannot = || {
        Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            command.cannot_message,
        ))
    };
    let failed = |err: &dyn std::fmt::Display| {
        tracing::warn!(%err, command = id, "power command failed");
        Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            command.failed_message,
        ))
    };
    // Read when run, as the C++ reads `preferenceValues()` in `execute`.
    let config = Config::load().unwrap_or_default();
    let preferences = config.entrypoint_preferences(compass_core::power_commands::EXTENSION_ID, id);
    if let Some(program) = compass_core::power_commands::custom_program(preferences) {
        return run_custom_power_program(program.to_owned()).await;
    }
    let system = match zbus::Connection::system().await {
        Ok(connection) => connection,
        Err(err) => return failed(&err),
    };
    let manager = PowerManager::new(system);
    // The owner of /proc/self is this process's uid, without `unsafe`.
    let uid = std::fs::metadata("/proc/self").map_or(u32::MAX, |m| m.uid());
    let checked = match id {
        "power-off" => Some(Action::PowerOff),
        "reboot" | "soft-reboot" => Some(Action::Reboot),
        "suspend" | "sleep" => Some(Action::Suspend),
        "hibernate" => Some(Action::Hibernate),
        _ => None,
    };
    if let Some(action) = checked {
        match manager.can(action).await {
            Ok(capability) if capability.is_offerable() => {}
            Ok(_) => return cannot(),
            Err(err) => return failed(&err),
        }
    }
    let result = match id {
        "sleep" => manager.sleep().await,
        "soft-reboot" => manager.soft_reboot().await,
        "lock" => manager.lock(uid).await,
        "logout" => {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            match zbus::Connection::session().await {
                Ok(session) => {
                    manager
                        .logout(compass_power::logout_target(&desktop), &session, uid)
                        .await
                }
                Err(err) => return failed(&err),
            }
        }
        _ => match checked {
            Some(action) => manager.perform(action, true).await,
            None => return cannot(),
        },
    };
    match result {
        Ok(()) => Response::Ack,
        Err(err) => failed(&err),
    }
}

/// Runs a power command's `customProgram` in its place, as
/// `AppService::shellProcess` does: `$SHELL -c program` (else `/bin/sh`), on
/// the host, waited for. Like `QProcess::waitForFinished`, only a program
/// that could not start or was killed by a signal is a failure; its exit code
/// is its own business.
async fn run_custom_power_program(program: String) -> Response {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    let run = {
        let program = program.clone();
        tokio::task::spawn_blocking(move || {
            compass_platform_linux::host_command(&shell)
                .args(["-c", &program])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        })
    };
    match run.await {
        Ok(Ok(status)) if status.code().is_some() => Response::Ack,
        outcome => {
            tracing::warn!(?outcome, program, "custom power program failed");
            Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                compass_core::power_commands::custom_program_failure(&program),
            ))
        }
    }
}

/// How long a Search Files query may wait on the indexer: under the
/// launcher's own request timeout, so it hears why rather than a timeout.
const FILE_SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Opens a file with its default application, or shows it in the file
/// browser, through the same application database extensions open with.
/// Answers an error the window shows when nothing opened it, rather than an
/// acknowledgement for a launch that failed out of sight (#253).
async fn open_file(state: &Arc<RwLock<EngineState>>, path: String, reveal: bool) -> Response {
    let target = std::path::PathBuf::from(&path);
    if !target.is_absolute() || !target.exists() {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no file exists at that path",
        ));
    }
    let apps = crate::extension_apps::EngineApps::new(
        &state.read().await.index,
        compass_xdg::mimeapps::Lists::from_environment(),
        tokio::runtime::Handle::current(),
    );
    let opened = if reveal {
        apps.reveal_path(&target).await
    } else {
        apps.open_path(&target).await
    };
    if let Err(reason) = opened {
        tracing::warn!(%reason, path, reveal, "the file was not opened");
        return Response::Error(ProtocolError::new(ErrorKind::Unsupported, reason));
    }
    if !reveal {
        // `OpenFileAction` records the open, so the file tops the empty
        // query next time.
        if let Some(xbel) = compass_xdg::bookmarks::recently_used_path() {
            let recorded = tokio::task::spawn_blocking(move || {
                let mime = compass_xdg::mimeapps::file_mime(&target);
                let now = jiff::Timestamp::now()
                    .strftime("%Y-%m-%dT%H:%M:%S%.6fZ")
                    .to_string();
                compass_xdg::bookmarks::record_access(&xbel, &target, &mime, &now)
            })
            .await;
            if let Ok(Err(error)) = recorded {
                tracing::warn!(%error, "not recording a recent file access");
            }
        }
    }
    Response::Ack
}

/// Uninstalls an extension, as `ExtensionRegistry::uninstall` does: its
/// directory, its support directory and its stored data, then root search
/// forgets its commands.
/// Installed extensions' titles, by extension id.
async fn extension_titles(
    state: &Arc<RwLock<EngineState>>,
) -> std::collections::BTreeMap<String, String> {
    state
        .read()
        .await
        .index
        .extensions()
        .iter()
        .map(|command| {
            (
                command.extension_id.clone(),
                command.extension_title.clone(),
            )
        })
        .collect()
}

/// What Script Permissions lists: the Rhai scripts' grants, then the
/// programs extensions were always allowed to run on the host, each part in
/// id order. Blocking: both read files.
fn all_grants(
    rhai: &crate::rhai_scripts::RhaiScripts,
    titles: &std::collections::BTreeMap<String, String>,
) -> Vec<compass_ipc::ScriptGrantEntry> {
    let mut grants = rhai.grants();
    grants.extend(
        crate::host_commands::Grants::from_environment()
            .entries(|id| titles.get(id).filter(|title| !title.is_empty()).cloned()),
    );
    grants
}

async fn uninstall_extension(state: &Arc<RwLock<EngineState>>, id: String) -> Response {
    let lookup = id.clone();
    let Ok(Some(directory)) =
        tokio::task::spawn_blocking(move || crate::stores::installed_directory(&lookup)).await
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            format!("No extension is installed with the id {id}"),
        ));
    };
    let Some(support) = compass_core::manifest::registry::support_directory(&id) else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "Uninstalling an extension needs a data directory",
        ));
    };
    let removed = tokio::task::spawn_blocking(move || {
        compass_core::store_bundle::uninstall(&directory, &support)
    })
    .await
    .unwrap_or_else(|err| Err(format!("the uninstall task failed: {err}")));
    if let Err(reason) = removed {
        return Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("Failed to uninstall extension: {reason}"),
        ));
    }
    clear_extension_storage(&id).await;
    state.write().await.index.rescan_extensions();
    tracing::info!(%id, "extension uninstalled");
    Response::Ack
}

/// The application database as a request sees it: the index, the
/// associations read now, and the runtime to launch on.
fn engine_apps_now(state: &EngineState) -> crate::extension_apps::EngineApps {
    crate::extension_apps::EngineApps::new(
        &state.index,
        compass_xdg::mimeapps::Lists::from_environment(),
        tokio::runtime::Handle::current(),
    )
}

/// Set Default Browser's and Set Default Terminal's list, as
/// `SetDefaultBrowserViewHost::reloadItems` and its terminal twin build it:
/// what opens [`compass_core::default_app::BROWSER_PROBE_URL`], or every
/// terminal emulator, with the current default first.
async fn list_default_apps(
    state: &Arc<RwLock<EngineState>>,
    kind: compass_ipc::DefaultAppKind,
) -> Response {
    use compass_core::default_app::{PickerApp, browser_picker, terminal_picker};
    use compass_worker_host::application_service::Apps as _;
    let state = state.read().await;
    let apps = engine_apps_now(&state);
    let service = compass_core::app_service::AppService::new(&state.index);
    let picker_app = |app: &compass_worker_host::application_service::Application| {
        let item = service.find_by_id(&app.id);
        PickerApp {
            id: app.id.clone(),
            display_name: app.name.clone(),
            description: item
                .and_then(compass_core::AppItem::comment)
                .unwrap_or_default()
                .to_owned(),
            // The index holds only what the root shows, so everything in it
            // is `displayable()`.
            displayable: true,
            terminal_emulator: item
                .is_some_and(|item| item.categories().iter().any(|c| c == "TerminalEmulator")),
        }
    };
    let picker = match kind {
        compass_ipc::DefaultAppKind::Browser => {
            let default = apps.web_browser().map(|app| app.id);
            let openers: Vec<PickerApp> = apps
                .openers(compass_core::default_app::BROWSER_PROBE_URL)
                .iter()
                .map(picker_app)
                .collect();
            browser_picker(&openers, default.as_deref())
        }
        compass_ipc::DefaultAppKind::Terminal => {
            let default = apps.terminal_emulator().map(|app| app.id);
            let terminals: Vec<PickerApp> =
                apps.terminal_emulators().iter().map(picker_app).collect();
            terminal_picker(&terminals, default.as_deref())
        }
    };
    Response::DefaultApps {
        apps: picker
            .items
            .into_iter()
            .map(|item| compass_ipc::DefaultAppEntry {
                id: item.app.id,
                name: item.app.display_name,
                description: item.app.description,
                is_default: item.is_default,
            })
            .collect(),
    }
}

/// The picker's action: `appDb->setWebBrowser(app)` into the user's
/// `mimeapps.list`, or `xdgpp::setDefaultTerminal(id)` into the user's
/// `xdg-terminals.list`. A failure answers with the picker's own sentence.
async fn set_default_app(
    state: &Arc<RwLock<EngineState>>,
    kind: compass_ipc::DefaultAppKind,
    id: &str,
) -> Response {
    use compass_core::default_app::{BROWSER_FAILURE, TERMINAL_FAILURE};
    let failure = |sentence: &str| {
        Response::Error(ProtocolError::new(ErrorKind::Internal, sentence.to_owned()))
    };
    let Some(config_home) = compass_xdg::mimeapps::config_home() else {
        return failure(match kind {
            compass_ipc::DefaultAppKind::Browser => BROWSER_FAILURE,
            compass_ipc::DefaultAppKind::Terminal => TERMINAL_FAILURE,
        });
    };
    match kind {
        compass_ipc::DefaultAppKind::Browser => {
            let mut apps = engine_apps_now(&*state.read().await);
            let id = id.to_owned();
            let set = tokio::task::spawn_blocking(move || {
                apps.set_web_browser(
                    &id,
                    &config_home.join("mimeapps.list"),
                    &compass_xdg::mimeapps::search_paths(),
                )
            })
            .await
            .unwrap_or(false);
            if set {
                tokio::spawn(show_hud(
                    state.read().await.window_slot(),
                    compass_core::default_app::BROWSER_SUCCESS.to_owned(),
                    Some("globe-01"),
                ));
                Response::Ack
            } else {
                failure(BROWSER_FAILURE)
            }
        }
        compass_ipc::DefaultAppKind::Terminal => {
            match compass_xdg::terminal::set_default_terminal(
                &config_home.join("xdg-terminals.list"),
                id,
                None,
            ) {
                Ok(()) => {
                    tokio::spawn(show_hud(
                        state.read().await.window_slot(),
                        compass_core::default_app::TERMINAL_SUCCESS.to_owned(),
                        None,
                    ));
                    Response::Ack
                }
                Err(error) => {
                    tracing::warn!(%error, "could not write xdg-terminals.list");
                    failure(TERMINAL_FAILURE)
                }
            }
        }
    }
}

/// Clears what an extension kept in local storage, and its preference
/// values, as `m_storage.clearNamespace(id)` does. Only when the storage
/// database exists: an extension that never ran has nothing there, and
/// opening the keyring to find that out would be a prompt for nothing.
async fn clear_extension_storage(id: &str) {
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("compass"))
    else {
        return;
    };
    if !data_dir
        .join(crate::extension_runner::STORAGE_DATABASE)
        .is_file()
    {
        return;
    }
    let Some(storage) = extension_storage(&data_dir).await else {
        return;
    };
    let id = id.to_owned();
    let _ = tokio::task::spawn_blocking(move || {
        crate::extension_runner::clear_extension_data(&storage, &id);
    })
    .await;
}

/// Browse Fonts' families, read from the font database the first time they
/// are asked for (or warmed at start), then kept.
async fn installed_fonts(
    fonts: &tokio::sync::OnceCell<Vec<compass_core::font_service::BrowsedFamily>>,
) -> &[compass_core::font_service::BrowsedFamily] {
    fonts
        .get_or_init(|| async {
            tokio::task::spawn_blocking(crate::fonts::browse)
                .await
                .unwrap_or_default()
        })
        .await
}

/// Runs a command line for Run Terminal Program, as `OpenInTerminalAction`
/// and `OpenRawProgramAction` do.
async fn run_program(
    state: &Arc<RwLock<EngineState>>,
    argv: Vec<String>,
    terminal: bool,
    hold: bool,
) -> Response {
    use compass_worker_host::application_service::{Apps, TerminalOptions};
    if argv.is_empty() || crate::programs::program_path(&argv[0]).is_none() {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "Not a valid executable",
        ));
    }
    if terminal {
        let apps = engine_apps(state).await;
        let options = TerminalOptions {
            hold,
            ..TerminalOptions::default()
        };
        return if apps.run_in_terminal(&argv, &options) {
            Response::Ack
        } else {
            Response::Error(ProtocolError::new(
                ErrorKind::Unsupported,
                "No terminal emulator is installed",
            ))
        };
    }
    match compass_platform_linux::run_command(&argv).await {
        Ok(_) => Response::Ack,
        Err(error) => Response::Error(ProtocolError::new(ErrorKind::Internal, error.to_string())),
    }
}

/// Runs a script command in its mode, as `ScriptExecutorAction::execute`
/// does; see [`crate::scripts`].
async fn run_script(state: &Arc<RwLock<EngineState>>, id: &str, arguments: &[String]) -> Response {
    use compass_core::script_command::OutputMode;
    use compass_worker_host::application_service::{Apps, TerminalOptions};
    let bad = |message: String| Response::Error(ProtocolError::new(ErrorKind::BadRequest, message));
    let (path, runs) = {
        let state = state.read().await;
        let Some(script) = state.scripts.find(id) else {
            return bad("no script command has that id".to_owned());
        };
        (script.path.clone(), Arc::clone(&state.script_runs))
    };
    // Read again, as the C++ reloads before every run: the file is the
    // user's and may have changed since the scan.
    let script = match std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|text| compass_core::script_scan::ScriptCommandFile::parse(&path, id, &text))
    {
        Ok(script) => script,
        Err(error) => return bad(format!("Failed to parse script: {error}")),
    };
    let argv = match crate::scripts::command_line(&script, arguments) {
        Ok(argv) => argv,
        Err(error) => return bad(error),
    };
    let cwd = crate::scripts::working_directory(&script);
    let mode = script.data.mode;
    if mode == OutputMode::Terminal {
        let data = &script.data;
        let options = TerminalOptions {
            hold: data.terminal.as_ref().and_then(|t| t.hold).unwrap_or(true),
            app_id: data.terminal.as_ref().and_then(|t| t.app_id.clone()),
            title: data
                .terminal
                .as_ref()
                .and_then(|t| t.title.clone())
                .or_else(|| (!data.title.is_empty()).then(|| data.title.clone())),
            working_directory: data
                .terminal
                .as_ref()
                .and_then(|t| t.working_directory.clone())
                .or_else(|| data.current_directory_path.clone()),
        };
        let apps = engine_apps(state).await;
        return if apps.run_in_terminal(&argv, &options) {
            Response::ScriptStarted { session: None }
        } else {
            Response::Error(ProtocolError::new(
                ErrorKind::Unsupported,
                "Failed to execute script",
            ))
        };
    }
    let (combined, timeout) = if mode == OutputMode::Full {
        (true, None)
    } else {
        (false, Some(crate::scripts::ONE_LINE_TIMEOUT))
    };
    let (session, run, task) = match runs.start(&argv, &cwd, combined, timeout) {
        Ok(started) => started,
        Err(error) => return Response::Error(ProtocolError::new(ErrorKind::Internal, error)),
    };
    match mode {
        OutputMode::Silent => {
            let slot = state.read().await.window_slot();
            tokio::spawn(async move {
                let _ = task.await;
                let (ok, line) = run
                    .lock()
                    .map(|run| (run.exit_code == Some(0), run.first_line()))
                    .unwrap_or_default();
                show_hud(
                    slot,
                    crate::scripts::one_line_message(mode, ok, &line),
                    None,
                )
                .await;
            });
            Response::ScriptStarted { session: None }
        }
        OutputMode::Inline => {
            let state = Arc::clone(state);
            let id = id.to_owned();
            tokio::spawn(async move {
                let _ = task.await;
                let (ok, line) = run
                    .lock()
                    .map(|run| (run.exit_code == Some(0), run.first_line()))
                    .unwrap_or_default();
                if ok {
                    let mut state = state.write().await;
                    let state = &mut *state;
                    state.scripts.save_run(&id, &line);
                    state.index.set_scripts(state.scripts.items());
                }
            });
            Response::ScriptStarted {
                session: Some(session),
            }
        }
        _ => Response::ScriptStarted {
            session: Some(session),
        },
    }
}

/// A root item launched as `cmd launch` launches it, with no arguments: what
/// a command's global shortcut does.
pub(crate) async fn launch_entrypoint(state: &Arc<RwLock<EngineState>>, id: String) -> Response {
    launch::launch_command(state, id, &[], None, None).await
}

/// The snippet list as the wire carries it.
/// Applies what the root row's panel changed: forgets the launch history,
/// or writes the configuration and applies it to root search, as the C++
/// root item manager merges it into the user's file and its metadata.
fn edit_root_item(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    edit: compass_ipc::RootItemEdit,
) -> Response {
    use compass_core::root_items::RootEdit;
    let edit = match edit {
        compass_ipc::RootItemEdit::Favorite(favorite) => RootEdit::Favorite(favorite),
        compass_ipc::RootItemEdit::MoveFavorite { down } => RootEdit::MoveFavorite { down },
        compass_ipc::RootItemEdit::Alias(alias) => RootEdit::Alias(alias),
        compass_ipc::RootItemEdit::Disable => RootEdit::Disable,
        compass_ipc::RootItemEdit::ResetRanking => RootEdit::ResetRanking,
        compass_ipc::RootItemEdit::Shortcut(shortcut) => RootEdit::Shortcut(shortcut),
        compass_ipc::RootItemEdit::Enabled(enabled) => RootEdit::Enabled(enabled),
        compass_ipc::RootItemEdit::Fallback(enabled) => RootEdit::Fallback(enabled),
    };
    let mut state = state.blocking_write();
    // Search Files is a fallback by the C++'s id (`files:search`), which
    // names no root item here.
    let known = state.index.root(id).is_some()
        || (matches!(edit, RootEdit::Fallback(_))
            && compass_core::commands::fallback(id).is_some());
    if !known {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no root item has that id",
        ));
    }
    if edit == RootEdit::ResetRanking {
        let key = state.index.history_key(id);
        return match state.frecency.forget(&key) {
            Ok(_) => Response::Ack,
            Err(err) => Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("could not reset the ranking: {err}"),
            )),
        };
    }
    // A file that does not parse is left alone rather than replaced by one
    // holding only this change.
    let mut config = match Config::load() {
        Ok(config) => config,
        Err(err) => {
            return Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("could not read the configuration: {err}"),
            ));
        }
    };
    if config.apply_root_edit(id, &edit) {
        let saved =
            compass_core::config::default_config_path().and_then(|path| config.save_to(path));
        if let Err(err) = saved {
            return Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("could not save the configuration: {err}"),
            ));
        }
    }
    state.index.apply_root_config(&config.root_config());
    state.global_shortcuts.reload();
    Response::Ack
}

/// `PasteService::pasteContent` for text the engine did not store (the
/// emoji picker's glyph): on the clipboard, then pasted into the window that
/// takes focus ([`crate::paste`]). Where nothing can press the paste, the
/// window copies instead.
async fn paste_text(state: &Arc<RwLock<EngineState>>, text: String) -> Response {
    crate::paste::paste(
        state,
        compass_shell::ClipboardContent::text(text),
        "Pasting",
    )
    .await
}

/// `ClipboardService::copyText` for the window: the text goes on a clipboard
/// the engine owns, so it outlives the window that asked. Over data-control
/// on wlroots, through the Shell extension elsewhere.
async fn copy_text(state: &Arc<RwLock<EngineState>>, text: String) -> Response {
    const WHAT: &str = "Copying";
    if crate::wlroots::detect()
        .await
        .is_some_and(|wlroots| wlroots.capabilities.data_control)
    {
        let offers = text_offers(&text);
        return match tokio::task::spawn_blocking(move || compass_wayland::clipboard::set(offers))
            .await
        {
            Ok(Ok(())) => Response::Ack,
            Ok(Err(err)) => Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("{WHAT} failed: {err}"),
            )),
            Err(err) => Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("{WHAT} failed: {err}"),
            )),
        };
    }
    let Some(shell) = state.read().await.shell_client() else {
        return Response::Error(crate::window_service::no_bus(WHAT));
    };
    match shell
        .set_clipboard(&compass_shell::ClipboardContent::text(text))
        .await
    {
        Ok(()) => Response::Ack,
        Err(err) => Response::Error(crate::window_service::refusal(&err, WHAT)),
    }
}

/// Text as data-control offers: UTF-8 under the types toolkits ask for, so
/// GTK, Qt, terminals and X clients through Xwayland all find one.
fn text_offers(text: &str) -> Vec<compass_wayland::data_control::Offer> {
    ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
        .into_iter()
        .map(|mime_type| compass_wayland::data_control::Offer {
            mime_type: mime_type.to_owned(),
            data: text.as_bytes().to_vec(),
        })
        .collect()
}

/// Why clipboard history answers nothing: almost always a keyring that is
/// missing or locked, which `compass doctor` explains.
const CLIPBOARD_UNAVAILABLE: &str = "clipboard history is unavailable: Compass needs an unlocked \
     keyring to keep it encrypted. Run compass doctor to see what is missing";

fn clipboard_unavailable() -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        CLIPBOARD_UNAVAILABLE,
    ))
}

/// Clipboard history for `query`, of one kind or of every kind.
async fn clipboard_history(
    state: &Arc<RwLock<EngineState>>,
    query: String,
    limit: u32,
    kind: Option<compass_ipc::ClipboardKind>,
) -> Response {
    if limit == 0 {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "a clipboard history request must ask for at least one entry",
        ));
    }
    let Some(store) = state.read().await.clipboard.clone() else {
        return clipboard_unavailable();
    };
    // SQLite is blocking I/O; keep it off the executor.
    match tokio::task::spawn_blocking(move || store.history_of_kind(&query, limit, kind)).await {
        Ok(Ok(entries)) => Response::ClipboardHistory { entries },
        Ok(Err(err)) => Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string())),
        Err(err) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("clipboard history task failed: {err}"),
        )),
    }
}

fn snippets_response(snippets: &compass_core::snippet_store::SnippetStore) -> Response {
    Response::Snippets {
        snippets: snippets
            .snippets()
            .iter()
            .map(crate::snippets::entry)
            .collect(),
    }
}

fn snippets_unavailable() -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        "snippets are unavailable: there is no data directory to keep them in",
    ))
}

/// Creates or updates a text snippet, as `SnippetFormViewHost::submit` does:
/// the form's rules first, then the store's (a keyword belongs to one
/// snippet), each refusal carrying the sentence the form would show.
async fn save_snippet(
    state: &Arc<RwLock<EngineState>>,
    id: Option<String>,
    name: String,
    text: String,
    keyword: Option<String>,
    word: bool,
    apps: Vec<String>,
) -> Response {
    use compass_core::snippet_form::{Submission, submit};
    use compass_core::snippet_store::{Error, SnippetData, SnippetPayload, StoredExpansion};
    let cursors = compass_core::placeholder::parse_snippet_text(&text)
        .placeholders
        .iter()
        .filter(|placeholder| placeholder.id == compass_core::snippet_expander::CURSOR_ID)
        .count();
    let keyword = keyword.unwrap_or_default();
    let (name, content, expansion) =
        match submit(id.as_deref(), &name, &text, cursors, &keyword, word, &apps) {
            Submission::Rejected { errors, toast } => {
                let reason = [errors.name, errors.content, errors.keyword]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("; ");
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("{toast}: {reason}"),
                ));
            }
            Submission::Save {
                name,
                content,
                expansion,
                ..
            } => (name, content, expansion),
        };
    let payload = SnippetPayload {
        name,
        data: SnippetData::Text { text: content },
        expansion: expansion.map(|expansion| StoredExpansion {
            keyword: expansion.keyword,
            apps: expansion.apps,
            word: expansion.word,
        }),
    };
    let mut state = state.write().await;
    let Some(snippets) = state.snippets.as_mut() else {
        return snippets_unavailable();
    };
    let now = crate::shortcuts::now();
    let saved = match &id {
        Some(id) => snippets.update(id, payload, now),
        None => snippets.add(payload, now).map(|_| ()),
    };
    match saved {
        Ok(()) => snippets_response(snippets),
        Err(error @ (Error::KeywordTaken(_) | Error::NoSuchId | Error::LimitReached)) => {
            Response::Error(ProtocolError::new(ErrorKind::BadRequest, error.to_string()))
        }
        Err(error) => Response::Error(ProtocolError::new(ErrorKind::Internal, error.to_string())),
    }
}

/// Tells the input server about keywords added, changed or removed, as
/// `SnippetService::createSnippet`/`updateSnippet`/`removeSnippet` do.
async fn sync_keywords(state: &Arc<RwLock<EngineState>>) {
    let (expander, wanted) = {
        let state = state.read().await;
        let Some(expander) = state.expander.clone() else {
            return;
        };
        let wanted = state
            .snippets
            .as_ref()
            .map(|store| crate::snippet_expansion::keywords(store.snippets()))
            .unwrap_or_default();
        (expander, wanted)
    };
    tokio::spawn(async move { expander.sync(wanted).await });
}

/// `InputServerStatus`, and `SetInputServerEnabled` after saving and applying
/// the setting.
async fn input_server(state: &Arc<RwLock<EngineState>>, enable: Option<bool>) -> Response {
    if let Some(enabled) = enable {
        let saved = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut config = Config::load()?;
            config.input_server_mut().set_enabled(Some(enabled));
            config.save_to(compass_core::config::default_config_path()?)?;
            Ok(())
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("could not save input_server.enabled: {error}"),
                ));
            }
            Err(error) => {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("saving input_server.enabled failed: {error}"),
                ));
            }
        }
    }
    let Some(expander) = state.read().await.expander.clone() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "this engine was started without the input server",
        ));
    };
    if let Some(enabled) = enable {
        expander.server().set_enabled(enabled);
        // Give the helper a moment to come up (or go), so the answer says
        // how it went rather than how it was.
        for _ in 0..20 {
            let status = expander.status();
            if status.running == enabled || status.problem.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    Response::InputServerStatus(expander.status())
}

/// A snippet's text expanded with `arguments` (a file snippet's path),
/// reading the clipboard first when the text asks for it.
async fn expand_snippet(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    arguments: &[(String, String)],
    run_shell: bool,
) -> Result<String, Response> {
    let (snippet, shell) = {
        let state = state.read().await;
        let Some(snippets) = &state.snippets else {
            return Err(snippets_unavailable());
        };
        let Some(snippet) = snippets.find_by_id(id) else {
            return Err(Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no snippet has that id",
            )));
        };
        (snippet.clone(), state.shell.clone())
    };
    let text = match &snippet.data {
        compass_core::snippet_store::SnippetData::Text { text } => text,
        compass_core::snippet_store::SnippetData::File { file } => return Ok(file.clone()),
    };
    let mut clipboard = None;
    if crate::snippets::needs_clipboard(text) {
        match shell {
            Some(shell) => match shell.clipboard().await {
                Ok(content) => clipboard = content.as_text().map(str::to_owned),
                Err(error) => tracing::info!(%error, "could not read the clipboard for a snippet"),
            },
            None => tracing::info!("no GNOME Shell extension; {{clipboard}} expands to nothing"),
        }
    }
    if !run_shell {
        return Ok(crate::snippets::preview(text, arguments, clipboard).to_text());
    }
    Ok(crate::snippets::expand(text, arguments, clipboard)
        .await
        .to_text())
}

/// The shortcut list as the wire carries it.
fn shortcuts_response(shortcuts: &compass_core::shortcut_service::ShortcutService) -> Response {
    Response::Shortcuts {
        shortcuts: shortcuts
            .shortcuts()
            .iter()
            .map(crate::shortcuts::entry)
            .collect(),
    }
}

fn shortcuts_unavailable() -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        "shortcuts are unavailable: there is no data directory to keep them in",
    ))
}

/// The applications an engine request resolves openers against.
async fn engine_apps(state: &Arc<RwLock<EngineState>>) -> crate::extension_apps::EngineApps {
    crate::extension_apps::EngineApps::new(
        &state.read().await.index,
        compass_xdg::mimeapps::Lists::from_environment(),
        tokio::runtime::Handle::current(),
    )
}

/// Creates or updates a shortcut, as `ShortcutFormViewHost::submit` does
/// once the form validates: the `default` icon is resolved to what the link
/// currently offers, and stored as that.
async fn save_shortcut(
    state: &Arc<RwLock<EngineState>>,
    id: Option<String>,
    name: String,
    icon: String,
    url: String,
    app: String,
) -> Response {
    use compass_core::shortcut_form::{Existing, Mode, Submission, submit};
    let icon = if icon == compass_core::shortcut_form::DEFAULT_ICON {
        let apps = engine_apps(state).await;
        crate::shortcuts::resolve_default_icon(&apps, &url)
    } else {
        icon
    };
    let mut state = state.write().await;
    let state = &mut *state;
    let Some(shortcuts) = state.shortcuts.as_mut() else {
        return shortcuts_unavailable();
    };
    let existing = match &id {
        Some(id) => {
            let Some(found) = shortcuts.find_by_id(id) else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no shortcut has that id",
                ));
            };
            Some(Existing {
                id: found.id.clone(),
                name: found.name.clone(),
                url: found.link.raw.clone(),
                app: found.app.clone(),
                icon: found.icon.clone(),
            })
        }
        None => None,
    };
    let mode = if existing.is_some() {
        Mode::Edit
    } else {
        Mode::Create
    };
    let now = crate::shortcuts::now();
    let saved = match submit(mode, existing.as_ref(), &name, &url, &app, &icon, &icon) {
        Submission::Rejected { errors, toast } => {
            let field = if errors.link.is_some() {
                "link"
            } else if errors.app.is_some() {
                "application"
            } else {
                "icon"
            };
            return Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                format!("{toast}: the {field} is required"),
            ));
        }
        Submission::Update {
            id,
            name,
            icon,
            link,
            app,
            failure,
            ..
        } => shortcuts
            .update(&id, &name, &icon, &link, &app, now)
            .then_some(())
            .ok_or(failure),
        Submission::Create {
            name,
            icon,
            link,
            app,
            failure,
            ..
        } => shortcuts
            .create(&name, &icon, &link, &app, now)
            .then_some(())
            .ok_or(failure),
    };
    if let Err(failure) = saved {
        return Response::Error(ProtocolError::new(ErrorKind::Internal, failure));
    }
    state.index.set_shortcuts(shortcuts.shortcuts().to_vec());
    shortcuts_response(shortcuts)
}

/// The shortcut `id` names, expanded with `arguments`, reading the clipboard
/// first when the link asks for it.
async fn expand_shortcut(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    arguments: &[String],
) -> Result<(compass_core::shortcut_service::CachedShortcut, String), Response> {
    let (shortcut, shell) = {
        let state = state.read().await;
        let Some(shortcuts) = &state.shortcuts else {
            return Err(shortcuts_unavailable());
        };
        let Some(shortcut) = shortcuts.find_by_id(id) else {
            return Err(Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no shortcut has that id",
            )));
        };
        (shortcut.clone(), state.shell.clone())
    };
    let mut reserved = crate::shortcuts::Reserved::default();
    if crate::shortcuts::needs_selection(&shortcut) {
        reserved.selection = primary_selection(shell.clone()).await;
    }
    if crate::shortcuts::needs_clipboard(&shortcut) {
        match shell {
            Some(shell) => match shell.clipboard().await {
                Ok(content) => reserved.clipboard = content.as_text().map(str::to_owned),
                Err(error) => tracing::info!(%error, "could not read the clipboard for a shortcut"),
            },
            None => tracing::info!("no GNOME Shell extension; {{clipboard}} expands to nothing"),
        }
    }
    let expanded = compass_core::shortcut::expand(&shortcut.link, arguments, &reserved);
    Ok((shortcut, expanded))
}

/// The selected text, read where `getSelectedText` reads it: the primary
/// selection over data-control on a wlroots compositor, else through the
/// Shell extension on GNOME. `None` when nothing is selected or there is
/// nowhere to read from, which a link expands to nothing, as the C++'s does.
async fn primary_selection(shell: Option<Arc<compass_shell::ShellClient>>) -> Option<String> {
    if crate::wlroots::session().is_some_and(|wlroots| wlroots.capabilities.data_control) {
        return tokio::task::spawn_blocking(compass_wayland::clipboard::read_primary_text)
            .await
            .ok()?
            .unwrap_or_else(|error| {
                tracing::info!(%error, "could not read the primary selection");
                None
            });
    }
    match shell?.primary_selection().await {
        Ok(text) => text,
        Err(error) => {
            tracing::info!(%error, "could not read the primary selection");
            None
        }
    }
}

/// Opens a shortcut, as `OpenShortcutAction::execute` does: expand, find the
/// application, launch, and count the visit.
async fn open_shortcut(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    arguments: &[String],
) -> Response {
    use compass_worker_host::application_service::Apps;
    let (shortcut, expanded) = match expand_shortcut(state, id, arguments).await {
        Ok(expanded) => expanded,
        Err(response) => return response,
    };
    let apps = engine_apps(state).await;
    let Some(app) = crate::shortcuts::resolve_app(&apps, &shortcut.app, &expanded) else {
        let message = if shortcut.app == compass_core::shortcut::DEFAULT_APP_ID {
            format!("No default app to open {expanded}")
        } else {
            format!("No app with id {}", shortcut.app)
        };
        return Response::Error(ProtocolError::new(ErrorKind::Unsupported, message));
    };
    apps.launch(&app, &expanded);
    let mut state = state.write().await;
    let state = &mut *state;
    if let Some(shortcuts) = state.shortcuts.as_mut()
        && shortcuts.register_visit(id, crate::shortcuts::now())
    {
        state.index.set_shortcuts(shortcuts.shortcuts().to_vec());
    }
    Response::Ack
}

/// The player a media command last acted on, which the next one that names
/// none prefers while it is still running.
static LAST_PLAYER: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// `pactl` on the host, bounded by the C++'s timeout.
struct HostPactl;

impl compass_core::audio_control::Pactl for HostPactl {
    fn run(&self, args: &[&str]) -> Option<String> {
        use std::io::Read;
        use wait_timeout::ChildExt;
        let mut child = compass_platform_linux::host_command("pactl")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        // Read while waiting, so a long sink list cannot fill the pipe.
        let reader = std::thread::spawn(move || {
            let mut out = String::new();
            stdout.read_to_string(&mut out).map(|_| out)
        });
        let timeout = std::time::Duration::from_millis(compass_core::audio_control::TIMEOUT_MS);
        match child.wait_timeout(timeout).ok()? {
            Some(status) if status.success() => reader.join().ok()?.ok(),
            Some(_) => None,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                None
            }
        }
    }
}

/// Runs a volume command through `pactl`, answering with the sentence the
/// C++ puts in its HUD, or refusing with its toast.
fn run_volume_command(id: &str, argument: Option<&str>) -> Result<String, &'static str> {
    use compass_core::media_commands::{
        VOLUME_DOWN_STEP, VOLUME_PRESETS, VOLUME_UP_STEP, mute_message, step_fraction,
        volume_hud_text, volume_step,
    };
    let audio = compass_core::audio_control::PactlAudioControl::new(HostPactl);
    match id {
        "volume-up" | "volume-down" => {
            let default = if id == "volume-up" {
                VOLUME_UP_STEP
            } else {
                VOLUME_DOWN_STEP
            };
            let step = volume_step(argument.map(str::trim), default)?;
            audio
                .adjust_volume(step_fraction(step))
                .map(volume_hud_text)
                .ok_or("Failed to adjust volume")
        }
        "toggle-mute" => {
            if !audio.toggle_mute() {
                return Err("Failed to toggle mute");
            }
            Ok(mute_message(audio.is_muted(), audio.volume()))
        }
        _ => {
            let percent = VOLUME_PRESETS
                .iter()
                .map(|(percent, _)| *percent)
                .find(|percent| id.strip_prefix("volume-") == Some(&percent.to_string()))
                .ok_or("Failed to set volume")?;
            audio
                .set_volume(step_fraction(percent))
                .map(volume_hud_text)
                .ok_or("Failed to set volume")
        }
    }
}

/// Shows what the C++ puts in its HUD: in the launcher's HUD where the
/// window has one (`WindowCommand::Hud`, a layer surface), otherwise as a
/// short transient notification, since an `xdg_toplevel` cannot appear
/// without taking the focus.
async fn show_hud(slot: WindowSlot, text: String, icon: Option<&'static str>) {
    let command = WindowCommand::Hud {
        text: text.clone(),
        icon: icon.map(str::to_owned),
    };
    if matches!(forward(&slot, command, "show the HUD").await, Response::Ack) {
        return;
    }
    notify_hud(&text).await;
}

/// The HUD as a transient notification (1.5 s, the HUD's own timer).
async fn notify_hud(text: &str) {
    let shown = notify_rust::Notification::new()
        .appname("Compass")
        .summary(text)
        .hint(notify_rust::Hint::Transient(true))
        .timeout(notify_rust::Timeout::Milliseconds(1500))
        .show_async()
        .await;
    if let Err(err) = shown {
        tracing::info!(%err, text, "HUD not shown");
    }
}

/// Runs a media command over MPRIS, on the player `argument` fuzzy-matches or,
/// when it is empty, on the default one. What the C++ shows in its HUD goes
/// out as a short-lived notification, since the launcher has already hidden.
async fn run_media_command(slot: WindowSlot, id: &str, argument: Option<String>) -> Response {
    use compass_core::media_commands::{self, NoPlayer};
    let refuse =
        |message: String| Response::Error(ProtocolError::new(ErrorKind::Unsupported, message));
    let failed = |message: &str, err: &dyn std::fmt::Display| {
        tracing::warn!(%err, command = id, "media command failed");
        Response::Error(ProtocolError::new(ErrorKind::Internal, message))
    };
    let argument = argument.map(|text| text.trim().to_owned());
    // Without media control, the media extension registers only the volume
    // commands.
    if media_commands::registered_commands(false)
        .iter()
        .any(|command| command == id)
    {
        let owned = id.to_owned();
        let answer =
            tokio::task::spawn_blocking(move || run_volume_command(&owned, argument.as_deref()))
                .await;
        return match answer {
            Ok(Ok(hud)) => {
                show_hud(slot, hud, None).await;
                Response::Ack
            }
            Ok(Err(message)) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            Err(err) => failed("Failed to adjust volume", &err),
        };
    }
    let failure = match id {
        "play-pause" => "Failed to toggle playback",
        "next-track" => "Failed to skip to the next track",
        "previous-track" => "Failed to skip to the previous track",
        _ => {
            return Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no media command has that id",
            ));
        }
    };
    let control = match zbus::Connection::session().await {
        Ok(connection) => compass_media::MediaControl::new(connection),
        Err(err) => return failed(failure, &err),
    };
    let found = match control.players().await {
        Ok(found) => found,
        Err(err) => return failed(failure, &err),
    };
    let players: Vec<media_commands::MediaPlayer> = found.iter().map(command_player).collect();
    let query = argument.unwrap_or_default();
    let last = LAST_PLAYER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let default = compass_media::default_player(&found, last.as_deref());
    if players.is_empty() {
        return refuse(media_commands::no_player_message(&NoPlayer::NothingRunning));
    }
    let matches = if query.is_empty() {
        Vec::new()
    } else {
        media_commands::player_matches(&query, &players)
    };
    let index = match media_commands::resolve_player(&query, default, &matches) {
        Ok(index) => index,
        Err(reason) => return refuse(media_commands::no_player_message(&reason)),
    };
    let player = &players[index];
    let (result, hud) = match id {
        "play-pause" => (
            control.play_pause(&player.id).await,
            media_commands::play_pause_message(player),
        ),
        "next-track" => {
            if let Some(refusal) = media_commands::skip_refusal(player, true) {
                return refuse(refusal);
            }
            (control.next(&player.id).await, "Next Track".to_owned())
        }
        _ => {
            if let Some(refusal) = media_commands::skip_refusal(player, false) {
                return refuse(refusal);
            }
            (
                control.previous(&player.id).await,
                "Previous Track".to_owned(),
            )
        }
    };
    if let Err(err) = result {
        return failed(failure, &err);
    }
    *LAST_PLAYER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(player.id.clone());
    let icon = match id {
        "play-pause" if player.playing => "pause",
        "play-pause" => "play",
        "next-track" => "forward",
        _ => "rewind",
    };
    show_hud(slot, hud, Some(icon)).await;
    Response::Ack
}

/// A player as the media commands decide about it.
fn command_player(found: &compass_media::MediaPlayer) -> compass_core::media_commands::MediaPlayer {
    compass_core::media_commands::MediaPlayer {
        id: found.id.clone(),
        identity: found.identity.clone(),
        title: found.title.clone(),
        artist: found.artist.clone(),
        playing: found.status == compass_media::PlaybackStatus::Playing,
        can_go_next: found.can_go_next,
        can_go_previous: found.can_go_previous,
    }
}

/// Now Playing's list: every running player.
async fn list_media_players() -> Response {
    let failed = |err: &dyn std::fmt::Display| {
        tracing::warn!(%err, "listing media players failed");
        Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            "Failed to list media players",
        ))
    };
    let control = match zbus::Connection::session().await {
        Ok(connection) => compass_media::MediaControl::new(connection),
        Err(err) => return failed(&err),
    };
    match control.players().await {
        Ok(players) => Response::MediaPlayers {
            players: players
                .into_iter()
                .map(|player| compass_ipc::MediaPlayerEntry {
                    playing: player.status == compass_media::PlaybackStatus::Playing,
                    paused: player.status == compass_media::PlaybackStatus::Paused,
                    id: player.id,
                    identity: player.identity,
                    app_id: player.app_id,
                    title: player.title,
                    artist: player.artist,
                    can_go_next: player.can_go_next,
                    can_go_previous: player.can_go_previous,
                })
                .collect(),
        },
        Err(err) => failed(&err),
    }
}

/// Now Playing's actions: one player, by its bus name, and no HUD, as the
/// C++'s panel actions call the provider directly.
async fn control_media_player(player: &str, action: compass_ipc::MediaPlayerAction) -> Response {
    use compass_ipc::MediaPlayerAction;
    let failure = match action {
        MediaPlayerAction::PlayPause => "Failed to toggle playback",
        MediaPlayerAction::Next => "Failed to skip to the next track",
        MediaPlayerAction::Previous => "Failed to skip to the previous track",
    };
    if !compass_media::is_player_name(player) {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a media player's bus name",
        ));
    }
    let control = match zbus::Connection::session().await {
        Ok(connection) => compass_media::MediaControl::new(connection),
        Err(err) => {
            tracing::warn!(%err, "media control failed");
            return Response::Error(ProtocolError::new(ErrorKind::Internal, failure));
        }
    };
    let done = match action {
        MediaPlayerAction::PlayPause => control.play_pause(player).await,
        MediaPlayerAction::Next => control.next(player).await,
        MediaPlayerAction::Previous => control.previous(player).await,
    };
    match done {
        Ok(()) => {
            *LAST_PLAYER
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(player.to_owned());
            Response::Ack
        }
        Err(err) => {
            tracing::warn!(%err, "media control failed");
            Response::Error(ProtocolError::new(ErrorKind::Internal, failure))
        }
    }
}

async fn run_extension_command(
    state: &Arc<RwLock<EngineState>>,
    id: String,
    arguments_json: Option<String>,
) -> Response {
    if let Some(script) = compass_core::rhai_scripts::script_id(&id) {
        return open_rhai_script(state, &id, script).await;
    }
    let Some(command) = state.read().await.index.extension(&id).cloned() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no installed extension command has that id",
        ));
    };
    let given = match arguments_json.as_deref().map(serde_json::from_str) {
        None => None,
        Some(Ok(serde_json::Value::Object(given))) => Some(given),
        Some(_) => {
            return Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "the arguments are not a JSON object",
            ));
        }
    };
    let runtime = match crate::extension_runner::Runtime::locate() {
        Ok(runtime) => runtime,
        Err(reason) => return Response::Error(ProtocolError::new(ErrorKind::Unsupported, reason)),
    };
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("compass"))
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "Running extensions needs a data directory, and $XDG_DATA_HOME and $HOME are unset",
        ));
    };
    let storage = extension_storage(&data_dir).await;
    let stored = match storage.clone() {
        Some(storage) => {
            let extension = command.extension_id.clone();
            tokio::task::spawn_blocking(move || {
                crate::extension_runner::load_preferences(&storage, &extension)
            })
            .await
            .unwrap_or_default()
        }
        None => serde_json::Map::new(),
    };
    let preferences = match command.preferences_with(&stored) {
        Ok(preferences) => preferences,
        Err(missing) if storage.is_none() => {
            let names: Vec<&str> = missing.iter().map(|p| p.title.as_str()).collect();
            return Response::Error(ProtocolError::new(
                ErrorKind::Unsupported,
                format!(
                    "{} needs {} set, and without a keyring Compass has nowhere safe to keep it",
                    command.title,
                    names.join(", ")
                ),
            ));
        }
        Err(_) => {
            return Response::ExtensionNeedsPreferences {
                title: command.title.clone(),
                fields: preference_fields(&command, &stored),
            };
        }
    };
    let arguments = match command.arguments_with(given.as_ref()) {
        Ok(arguments) => arguments,
        Err(_) => {
            return Response::ExtensionNeedsArguments {
                title: command.title.clone(),
                fields: argument_fields(&command, given.as_ref()),
            };
        }
    };
    let (files, launches, known) = {
        let state = state.read().await;
        (
            Arc::clone(&state.files),
            Arc::clone(&state.launches),
            crate::extension_commands::Known::all(&state.index),
        )
    };
    let deliver: crate::extension_commands::Deliver = {
        let (state, handle) = (Arc::clone(state), tokio::runtime::Handle::current());
        Arc::new(move |token| {
            let state = Arc::clone(&state);
            handle.spawn(async move { deliver_launch(&state, token).await });
        })
    };
    let host = crate::extension_runner::Host {
        storage,
        shell: state.read().await.shell.clone(),
        views: Arc::clone(&state.read().await.views),
        preferences,
        arguments,
        apps: Some(crate::extension_apps::EngineApps::new(
            &state.read().await.index,
            compass_xdg::mimeapps::Lists::from_environment(),
            tokio::runtime::Handle::current(),
        )),
        files: Some(files),
        context: launches.take_context(&id),
        commands: Some(crate::extension_commands::EngineCommands::new(
            id.clone(),
            known,
            launches,
            deliver,
        )),
        launcher: Some(launcher_window(state)),
    };
    let started = tokio::task::spawn_blocking(move || {
        crate::extension_runner::start(&runtime, &command, &data_dir, host)
    })
    .await;
    match started {
        Ok(Ok(started)) => {
            let recorded = tokio::task::spawn_blocking({
                let state = Arc::clone(state);
                move || state.blocking_write().frecency.record_launch(&id)
            })
            .await;
            if !matches!(recorded, Ok(Ok(()))) {
                tracing::warn!("could not record running an extension command");
            }
            match started {
                crate::extension_runner::Started::Ran => Response::Ack,
                crate::extension_runner::Started::View(session) => {
                    Response::ExtensionStarted { session }
                }
            }
        }
        Ok(Err(reason)) => Response::Error(ProtocolError::new(ErrorKind::Internal, reason)),
        Err(err) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("the extension task failed: {err}"),
        )),
    }
}

/// The launcher window, for a command's serving thread: it blocks that
/// thread, never the engine's, until the window answers.
fn launcher_window(state: &Arc<RwLock<EngineState>>) -> crate::extension_runner::Launcher {
    let (state, handle) = (Arc::clone(state), tokio::runtime::Handle::current());
    crate::extension_runner::Launcher(Arc::new(move |command| {
        let state = Arc::clone(&state);
        handle.block_on(async move {
            let slot = state.read().await.window_slot();
            matches!(
                forward(&slot, command, "tell the launcher").await,
                Response::Ack
            )
        })
    }))
}

/// Hands the launch under `token` to the launcher window. Without a window,
/// a no-view command runs here as it would from the window; a view command
/// has nowhere to be shown, and the launch is dropped with a warning.
async fn deliver_launch(state: &Arc<RwLock<EngineState>>, token: u64) {
    let slot = state.read().await.window_slot();
    let Response::Error(refused) = forward(
        &slot,
        WindowCommand::Launch(token),
        "take an extension's launch",
    )
    .await
    else {
        return;
    };
    let Some(launch) = state.read().await.launches.take(token) else {
        return;
    };
    let no_view = state
        .read()
        .await
        .index
        .extension(&launch.id)
        .is_some_and(|command| command.mode == compass_core::manifest::CommandMode::NoView);
    if launch.preferences || !no_view {
        tracing::warn!(
            id = %launch.id, reason = %refused.message,
            "an extension's launch had no launcher window to show it"
        );
        return;
    }
    // Asked over the engine's own socket, as the window would ask: this runs
    // inside a command's launch, and calling `run_extension_command` from
    // here would make that function's future contain itself.
    let socket = state.read().await.socket.clone();
    let asked = async {
        compass_ipc::Client::connect(socket.as_path())
            .await?
            .request(Request::RunExtensionCommand {
                id: launch.id.clone(),
                arguments_json: launch.arguments_json,
            })
            .await
    };
    match asked.await {
        Ok(Response::Error(err)) => {
            tracing::warn!(id = %launch.id, error = %err.message, "an extension's launch failed");
        }
        Err(err) => {
            tracing::warn!(id = %launch.id, error = %err, "an extension's launch failed");
        }
        Ok(_) => {}
    }
}

/// The preferences form for the extension command `id`, without running it.
async fn extension_preferences(state: &Arc<RwLock<EngineState>>, id: &str) -> Response {
    let Some(command) = state.read().await.index.extension(id).cloned() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no installed extension command has that id",
        ));
    };
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("compass"))
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "Extension preferences need a data directory, and $XDG_DATA_HOME and $HOME are unset",
        ));
    };
    let Some(storage) = extension_storage(&data_dir).await else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            format!(
                "{}'s preferences need a keyring: without one Compass has nowhere safe to keep them",
                command.title
            ),
        ));
    };
    let extension = command.extension_id.clone();
    let stored = tokio::task::spawn_blocking(move || {
        crate::extension_runner::load_preferences(&storage, &extension)
    })
    .await
    .unwrap_or_default();
    Response::ExtensionNeedsPreferences {
        title: command.title.clone(),
        fields: preference_fields(&command, &stored),
    }
}

/// Opens the Rhai script `script` (root entry `id`) as a view session.
async fn open_rhai_script(state: &Arc<RwLock<EngineState>>, id: &str, script: &str) -> Response {
    let (rhai, session) = {
        let state = state.read().await;
        (Arc::clone(&state.rhai), state.views.reserve())
    };
    if let Err(reason) = rhai.open(script, session).await {
        return Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason));
    }
    let recorded = tokio::task::spawn_blocking({
        let (state, id) = (Arc::clone(state), id.to_owned());
        move || state.blocking_write().frecency.record_launch(&id)
    })
    .await;
    if !matches!(recorded, Ok(Ok(()))) {
        tracing::warn!("could not record opening a Rhai script");
    }
    Response::ExtensionStarted { session }
}

/// Carries out what a Rhai script's action asked of the engine.
async fn rhai_outcomes(
    state: &Arc<RwLock<EngineState>>,
    outcomes: Vec<crate::rhai_scripts::Outcome>,
) {
    for outcome in outcomes {
        if let crate::rhai_scripts::Outcome::Hud(text) = &outcome {
            show_hud(state.read().await.window_slot(), text.clone(), None).await;
        }
        let slot = state.read().await.window_slot();
        if let Response::Error(err) = forward(&slot, WindowCommand::Hide, "hide the launcher").await
        {
            tracing::debug!(message = %err.message, "a script's close reached no window");
        }
    }
}

/// Every argument `command` takes, as the launcher's form draws it, with what
/// was already entered.
fn argument_fields(
    command: &compass_core::extension_commands::ExtensionCommand,
    given: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<compass_ipc::PreferenceField> {
    use compass_core::manifest::ArgumentType;
    use compass_ipc::PreferenceFieldKind;
    command
        .arguments
        .iter()
        .map(|argument| compass_ipc::PreferenceField {
            name: argument.name.clone(),
            // Raycast labels an argument by its placeholder; it has no title.
            title: if argument.placeholder.is_empty() {
                argument.name.clone()
            } else {
                argument.placeholder.clone()
            },
            description: String::new(),
            placeholder: argument.placeholder.clone(),
            required: argument.required,
            kind: match argument.argument_type {
                ArgumentType::Text => PreferenceFieldKind::Text,
                ArgumentType::Password => PreferenceFieldKind::Password,
                ArgumentType::Dropdown => PreferenceFieldKind::Dropdown {
                    options: argument
                        .data
                        .iter()
                        .flatten()
                        .map(|option| (option.title.clone(), option.value.clone()))
                        .collect(),
                },
            },
            value_json: given
                .and_then(|given| given.get(&argument.name))
                .map(ToString::to_string),
        })
        .collect()
}

/// Every preference `command` reads, as the launcher's form draws it.
fn preference_fields(
    command: &compass_core::extension_commands::ExtensionCommand,
    stored: &serde_json::Map<String, serde_json::Value>,
) -> Vec<compass_ipc::PreferenceField> {
    use compass_core::manifest::PreferenceKind;
    use compass_ipc::PreferenceFieldKind;
    command
        .preferences
        .iter()
        .map(|preference| compass_ipc::PreferenceField {
            name: preference.name.clone(),
            title: preference.title.clone(),
            description: preference.description.clone(),
            placeholder: preference.placeholder.clone(),
            required: preference.required,
            kind: match &preference.kind {
                PreferenceKind::TextField => PreferenceFieldKind::Text,
                PreferenceKind::Password => PreferenceFieldKind::Password,
                PreferenceKind::Checkbox { label } => PreferenceFieldKind::Checkbox {
                    label: label.clone(),
                },
                PreferenceKind::Dropdown { options } => PreferenceFieldKind::Dropdown {
                    options: options
                        .iter()
                        .map(|option| (option.title.clone(), option.value.clone()))
                        .collect(),
                },
                PreferenceKind::AppPicker => PreferenceFieldKind::Unsupported {
                    declared: "appPicker".to_owned(),
                },
                PreferenceKind::FilePicker { .. } => PreferenceFieldKind::Unsupported {
                    declared: "file".to_owned(),
                },
                PreferenceKind::DirectoryPicker { .. } => PreferenceFieldKind::Unsupported {
                    declared: "directory".to_owned(),
                },
                PreferenceKind::Unknown { declared } => PreferenceFieldKind::Unsupported {
                    declared: declared.clone(),
                },
            },
            value_json: stored
                .get(&preference.name)
                .or(preference.default.as_ref())
                .map(ToString::to_string),
        })
        .collect()
}

async fn set_extension_preferences(
    state: &Arc<RwLock<EngineState>>,
    id: String,
    values_json: String,
) -> Response {
    let Some(command) = state.read().await.index.extension(&id).cloned() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no installed extension command has that id",
        ));
    };
    let values: serde_json::Map<String, serde_json::Value> =
        match serde_json::from_str(&values_json) {
            Ok(values) => values,
            Err(err) => {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("values_json is not a JSON object: {err}"),
                ));
            }
        };
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("compass"))
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "no data directory",
        ));
    };
    let Some(storage) = extension_storage(&data_dir).await else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "without a keyring Compass has nowhere safe to keep extension preferences",
        ));
    };
    let saved = tokio::task::spawn_blocking(move || {
        crate::extension_runner::save_preferences(&storage, &command.extension_id, &values)
    })
    .await;
    match saved {
        Ok(Ok(())) => Response::Ack,
        Ok(Err(reason)) => Response::Error(ProtocolError::new(ErrorKind::Internal, reason)),
        Err(err) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("the preferences task failed: {err}"),
        )),
    }
}

/// Shortens how long an `ExtensionView` is held, in milliseconds, so a test
/// can hold a view idle past a client's deadline without waiting the whole
/// [`compass_ipc::EXTENSION_VIEW_HOLD`]. Never lengthens it: a client allows
/// [`compass_ipc::long_poll_deadline`] of the constant, not of this.
pub const VIEW_HOLD_ENV: &str = "COMPASS_EXTENSION_VIEW_HOLD_MS";

/// How long an `ExtensionView` is held open waiting for a change.
fn view_hold() -> std::time::Duration {
    static HOLD: std::sync::LazyLock<std::time::Duration> = std::sync::LazyLock::new(|| {
        std::env::var(VIEW_HOLD_ENV)
            .ok()
            .and_then(|ms| ms.trim().parse().ok())
            .map(std::time::Duration::from_millis)
            .map_or(compass_ipc::EXTENSION_VIEW_HOLD, |hold| {
                hold.min(compass_ipc::EXTENSION_VIEW_HOLD)
            })
    });
    *HOLD
}

async fn extension_view(state: &Arc<RwLock<EngineState>>, session: u64, after: u64) -> Response {
    let watched = {
        let state = state.read().await;
        state
            .views
            .watch(session)
            .or_else(|| state.rhai.watch(session))
    };
    let Some(mut watch) = watched else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no extension view is running with that session",
        ));
    };
    // A timeout is an answer too: the same version, so the launcher asks again.
    let _ = tokio::time::timeout(view_hold(), watch.wait_for(|view| view.version > after)).await;
    let view = watch.borrow().clone();
    Response::ExtensionView {
        version: view.version,
        view_json: view.view,
        problem: view.problem,
        ended: view.ended,
        depth: view.depth,
        alert: view.alert,
        toast: view.toast,
    }
}

/// Extensions' local storage, keyed from Compass's own master key; `None`
/// (and local storage answered "not implemented") without a keyring.
pub(crate) async fn extension_storage(
    data_dir: &std::path::Path,
) -> Option<crate::extension_runner::Storage> {
    let keyring = match crate::clipboard_service::Oo7Store::connect().await {
        Ok(keyring) => keyring,
        Err(err) => {
            tracing::info!(error = %err, "no keyring; extension storage unavailable");
            return None;
        }
    };
    match crate::clipboard_service::master_key(&keyring).await {
        Ok(master) => Some(crate::extension_runner::Storage {
            path: data_dir.join(crate::extension_runner::STORAGE_DATABASE),
            key: compass_crypto::keys::derive_all(&master).database,
        }),
        Err(err) => {
            tracing::info!(error = %err, "no master key; extension storage unavailable");
            None
        }
    }
}

fn app_hit(ranked: &compass_core::apps::ApplicationRootHit<'_>) -> QueryHit {
    QueryHit {
        // The ENTRYPOINT id, not the desktop key. The protocol
        // documents this field as the "stable identifier of the
        // underlying root item", and `AppIndex` already builds one
        // (`app_root_item` -> `entrypoint_id(APPS_PROVIDER_ID, ...)`);
        // this put `AppItem::key` there instead, so the wire carried
        // `host--byobu.desktop` for the item every other part of the
        // system — and the C++ engine — calls
        // `applications:host--byobu`.
        id: ranked.entrypoint_id.to_owned(),
        title: ranked.item.display_name(),
        subtitle: None,
        // `match_score`, not `score`. `Ranked::score` is the combined
        // value that includes the frecency boost and is not bounded,
        // whereas the wire documents `0..=100` on compass-search's
        // scale. See the ordering note on `query`.
        score: ranked.match_score,
    }
}

/// The three requests that need a launcher window, when none is connected.
///
/// [ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)
/// makes the window a resident process that connects to this daemon, so the
/// honest answer is about a *connection* rather than about the build. That is
/// the more useful thing to be told: "start the window" is actionable, "this
/// engine is headless" was not.
///
/// The property this preserves is the one that matters. A client can still
/// tell "no window" from "the window was shown", which is the only thing
/// standing between an honest gap and a `toggle` that silently does nothing.
fn no_window(what: &str) -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        format!(
            "cannot {what}: no launcher window is connected to this engine. \
             Start one with `compass ui`. Query and doctor work without it."
        ),
    ))
}

/// `vici develop`'s deeplinks ([`compass_core::develop_link`]): a new or
/// rebuilt extension is rescanned, so it is in root search and its next run
/// uses the new bundle.
async fn develop(
    state: &Arc<RwLock<EngineState>>,
    link: &compass_core::develop_link::DevelopLink,
) -> Response {
    use compass_core::develop_link::DevelopAction;
    match link.action {
        DevelopAction::Start | DevelopAction::Refresh => {
            tracing::info!(id = %link.id, action = ?link.action, "an extension under development was rebuilt");
            state.write().await.rescan_extensions();
        }
        DevelopAction::Stop => {
            tracing::info!(id = %link.id, "an extension's development session ended");
        }
    }
    Response::Ack
}

/// Forwards `command` to the attached window, if there is one.
///
/// Clears the slot when the push fails. That is **hygiene, not behaviour**: a
/// push to a dead socket fails anyway, so the refusal a client sees is the same
/// either way — a control confirmed the end-to-end tests pass with the clearing
/// removed. What it buys is releasing the file descriptor and not paying a
/// doomed write on every subsequent request. See [`WindowSlot`].
pub(crate) async fn forward(slot: &WindowSlot, command: WindowCommand, what: &str) -> Response {
    forward_within(slot, command, what, WINDOW_ANSWER_TIMEOUT).await
}

/// How long the engine waits for the window to answer a command.
///
/// The slot is locked while it waits, so every request that needs the window
/// waits too. Every command is answered as soon as the window has acted on it
/// (a dmenu's choice comes later, outside the push), but the first `Show` waits
/// on the new window's renderer, and a machine without a GPU spends seconds
/// probing Vulkan before it settles on GL. So this bounds a stuck window rather
/// than a slow one, and is longer than the CLI's own 10 seconds: a slow first
/// show must not be refused while it is still coming up.
const WINDOW_ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// [`forward`] with the wait for the window's answer bounded by `timeout`.
///
/// A window that does not answer in time keeps its link. It may only be busy,
/// and dropping it would leave the launcher undriven until it restarts; its
/// late answer is skipped by the next push (see [`WindowLink::push`]).
pub(crate) async fn forward_within(
    slot: &WindowSlot,
    command: WindowCommand,
    what: &str,
    timeout: std::time::Duration,
) -> Response {
    let mut guard = slot.lock().await;

    let Some(link) = guard.as_mut() else {
        return no_window(what);
    };

    let Ok(answer) = tokio::time::timeout(timeout, link.push(command)).await else {
        tracing::warn!(
            seconds = timeout.as_secs_f32(),
            "the launcher window did not answer"
        );
        return Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!(
                "the launcher window did not answer within {} seconds, so it could not {what}",
                timeout.as_secs()
            ),
        ));
    };

    match answer {
        Ok(WindowOutcome::Shown | WindowOutcome::Hidden) => Response::Ack,
        Ok(WindowOutcome::Failed(reason)) => {
            // The window is alive and said no. Keeping the link is the point:
            // a compositor that refused one activation will very likely accept
            // the next, and dropping the window over it would turn a recoverable
            // refusal into a dead launcher.
            tracing::warn!(reason = %reason, "the launcher window refused a command");
            Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("the launcher window could not {what}: {reason}"),
            ))
        }
        Err(err) => {
            tracing::info!(error = %err, "the launcher window went away");
            *guard = None;
            no_window(what)
        }
    }
}

/// `ListWindows`: from KWin's tracker on Plasma, over Wayland on a wlroots
/// compositor, through the Shell extension everywhere else.
pub(crate) async fn list_windows(state: &Arc<RwLock<EngineState>>) -> Response {
    {
        let state = state.read().await;
        if let Some(response) = crate::window_service::kwin_list(&state.index).await {
            return response;
        }
    }
    // wlroots compositors list windows over Wayland; never on GNOME.
    if crate::wlroots::detect().await.is_some() {
        let state = state.read().await;
        if let Some(response) = crate::window_service::wlroots_list(&state.index).await {
            return response;
        }
    }
    let (shell, index_state) = (state.read().await.shell.clone(), Arc::clone(state));
    let Some(shell) = shell else {
        return Response::Error(crate::window_service::no_bus("Window switching"));
    };
    match shell.list_windows().await {
        Ok(windows) => {
            let state = index_state.read().await;
            Response::Windows {
                windows: crate::window_service::rows(
                    windows.into_iter().map(Into::into).collect(),
                    &state.index,
                ),
            }
        }
        Err(err) => Response::Error(crate::window_service::refusal(&err, "Window switching")),
    }
}

/// `ActivateWindow` or, with `close`, `CloseWindow`.
pub(crate) async fn act_on_window(
    state: &Arc<RwLock<EngineState>>,
    id: u32,
    close: bool,
) -> Response {
    let what = if close {
        "Closing a window"
    } else {
        "Switching to a window"
    };
    if let Some(response) = crate::window_service::kwin_act(id, close, what).await {
        return response;
    }
    if let Some(response) = crate::window_service::wlroots_act(id, close, what).await {
        return response;
    }
    let Some(shell) = state.read().await.shell.clone() else {
        return Response::Error(crate::window_service::no_bus(what));
    };
    let id = compass_shell::WindowId(id);
    let done = if close {
        shell.close_window(id).await
    } else {
        shell.activate_window(id).await
    };
    match done {
        Ok(()) => Response::Ack,
        Err(err) => Response::Error(crate::window_service::refusal(&err, what)),
    }
}

/// Answers one request.
///
/// Separated from the serve loop so the whole request surface is testable
/// without a socket, and so the serve loop stays about lifecycle.
pub async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::Ping => Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: std::process::id(),
        },

        Request::Query { text } => {
            let state = state.read().await;
            Response::QueryResults {
                hits: state.query(&text),
            }
        }

        Request::Doctor => {
            let (socket, engine) = {
                let state = state.read().await;
                (state.socket.clone(), Engine::Rust)
            };
            Response::DoctorReport {
                checks: doctor::run_on_this_machine(&socket, engine).await.checks,
            }
        }

        Request::RecordLaunch { key } => {
            let state = Arc::clone(state);
            // JSON persistence is blocking disk work. Keep it off the async
            // executor, and serialize updates through the daemon's store.
            tokio::task::spawn_blocking(move || {
                let mut state = state.blocking_write();
                if state.index.get(&key).is_none() && compass_core::commands::by_id(&key).is_none()
                {
                    return Response::Error(ProtocolError::new(
                        ErrorKind::BadRequest,
                        "launch key is neither an application nor a builtin command",
                    ));
                }
                match state.frecency.record_launch(&key) {
                    Ok(()) => Response::Ack,
                    Err(error) => {
                        tracing::warn!(%error, "could not persist launch history");
                        Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            "could not persist launch history",
                        ))
                    }
                }
            })
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "launch history task failed");
                Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "launch history task failed",
                ))
            })
        }

        Request::ClipboardHistory { query, limit } => {
            clipboard_history(state, query, limit, None).await
        }

        Request::ClipboardHistoryOfKind { query, limit, kind } => {
            clipboard_history(state, query, limit, kind).await
        }

        Request::ClipboardDetail { id } => {
            let Some(store) = state.read().await.clipboard.clone() else {
                return clipboard_unavailable();
            };
            match tokio::task::spawn_blocking(move || store.detail(&id)).await {
                Ok(Ok(Some(detail))) => Response::ClipboardDetail { detail },
                Ok(Ok(None)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard detail task failed: {err}"),
                )),
            }
        }

        Request::ClipboardSetKeywords { id, keywords } => {
            let Some(store) = state.read().await.clipboard.clone() else {
                return clipboard_unavailable();
            };
            match tokio::task::spawn_blocking(move || store.set_keywords(&id, &keywords)).await {
                Ok(Ok(true)) => Response::Ack,
                Ok(Ok(false)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard keywords task failed: {err}"),
                )),
            }
        }

        Request::ClipboardRemoveAll => {
            let (store, control) = {
                let state = state.read().await;
                (state.clipboard.clone(), state.clipboard_control())
            };
            let Some(store) = store else {
                return clipboard_unavailable();
            };
            let preserve = control.preserve_tagged();
            match tokio::task::spawn_blocking(move || store.remove_all(preserve)).await {
                Ok(Ok(_)) => Response::Ack,
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard remove-all task failed: {err}"),
                )),
            }
        }

        Request::ClipboardMonitoring { enabled } => {
            let control = state.read().await.clipboard_control();
            if let Some(enabled) = enabled {
                control.set_monitoring(enabled);
                // Kept as the preference, as `toggleMonitoring` patches it,
                // so the choice outlives the engine.
                let saved = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    // A file that does not parse is left alone rather than
                    // replaced by one holding only this choice.
                    let mut config = Config::load()?;
                    config.set_provider_preference(
                        crate::clipboard_service::PROVIDER_ID,
                        "monitoring",
                        serde_json::Value::Bool(enabled),
                    );
                    config.save_to(compass_core::config::default_config_path()?)?;
                    Ok(())
                })
                .await;
                match saved {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => tracing::warn!(error = %err, "monitoring choice not saved"),
                    Err(err) => tracing::warn!(error = %err, "the monitoring save task failed"),
                }
            }
            Response::ClipboardMonitoring {
                supported: control.supported(),
                enabled: control.monitoring(),
            }
        }

        Request::RootItemEdit { id, edit } => {
            let state = Arc::clone(state);
            tokio::task::spawn_blocking(move || edit_root_item(&state, &id, edit))
                .await
                .unwrap_or_else(|err| {
                    Response::Error(ProtocolError::new(
                        ErrorKind::Internal,
                        format!("the root item task failed: {err}"),
                    ))
                })
        }

        Request::ClipboardContent { id } => {
            let Some(store) = state.read().await.clipboard.clone() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    CLIPBOARD_UNAVAILABLE,
                ));
            };
            match tokio::task::spawn_blocking(move || store.content(&id)).await {
                Ok(Ok(Some((mime_type, data)))) => Response::ClipboardContent { mime_type, data },
                Ok(Ok(None)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard content task failed: {err}"),
                )),
            }
        }

        Request::ListWindows => list_windows(state).await,

        Request::ActivateWindow { id } | Request::CloseWindow { id } => {
            act_on_window(state, id, matches!(request, Request::CloseWindow { .. })).await
        }

        Request::ListCommands => {
            let state = state.read().await;
            Response::Commands {
                commands: launch::commands(&state.index),
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
        | Request::TrayTriggerMenu { .. }) => {
            let tray = Arc::clone(&state.read().await.tray);
            tray_request(&tray, request).await
        }
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
        request @ (Request::SetSetting { .. } | Request::SetProviderEnabled { .. }) => {
            settings::handle(state, request).await
        }
        Request::ShortcutCapture { capturing } => {
            state
                .read()
                .await
                .global_shortcuts()
                .set_capturing(capturing);
            Response::Ack
        }
        Request::UpdateStatus => {
            let updates = Arc::clone(&state.read().await.updates);
            // Read when asked, so turning the setting off stops the next check.
            let enabled = Config::load()
                .map(|config| config.launcher().check_for_updates())
                .unwrap_or(compass_core::config::DEFAULT_CHECK_FOR_UPDATES);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| {
                    i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
                });
            Response::UpdateStatus {
                current: updates.current().to_owned(),
                available: updates.status(enabled, now).await,
            }
        }
        Request::SkipUpdate { tag } => {
            let updates = Arc::clone(&state.read().await.updates);
            match updates.skip(&tag) {
                Ok(()) => Response::Ack,
                Err(message) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            }
        }
        request @ (Request::ExchangeRates | Request::RefreshExchangeRates) => {
            let rates = Arc::clone(&state.read().await.exchange_rates);
            rates.answer(request).await
        }
        Request::ProbeShortcut { trigger } => {
            let Some(combo) = compass_core::key_combo::KeyCombo::parse(&trigger) else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("{trigger:?} is not a key combination"),
                ));
            };
            let control = state.read().await.global_shortcuts();
            Response::ShortcutProbe {
                refusal: control.probe(combo).await,
            }
        }

        Request::RunPowerCommand { id } => run_power_command(&id).await,
        Request::RunMediaCommand { id } => {
            run_media_command(state.read().await.window_slot(), &id, None).await
        }
        Request::RunMediaCommandWith { id, argument } => {
            run_media_command(state.read().await.window_slot(), &id, argument).await
        }
        Request::ListMediaPlayers => list_media_players().await,
        Request::ControlMediaPlayer { player, action } => {
            control_media_player(&player, action).await
        }

        Request::SearchFiles { query, category } => {
            let files = Arc::clone(&state.read().await.files);
            let category = category
                .as_deref()
                .and_then(compass_core::file_search::category_for_key);
            let search = tokio::task::spawn_blocking(move || files.search(&query, category));
            match tokio::time::timeout(FILE_SEARCH_TIMEOUT, search).await {
                Ok(Ok(Ok(found))) => Response::Files {
                    heading: found.heading.to_owned(),
                    files: found.files,
                },
                Ok(Ok(Err(message))) => {
                    Response::Error(ProtocolError::new(ErrorKind::Unsupported, message))
                }
                Ok(Err(err)) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("file search task failed: {err}"),
                )),
                Err(_) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "the file indexer did not answer in time",
                )),
            }
        }

        Request::OpenFile { path, reveal } => open_file(state, path, reveal).await,
        Request::ListShortcuts => {
            let state = state.read().await;
            match &state.shortcuts {
                Some(shortcuts) => shortcuts_response(shortcuts),
                None => shortcuts_unavailable(),
            }
        }
        Request::SaveShortcut {
            id,
            name,
            icon,
            url,
            app,
        } => save_shortcut(state, id, name, icon, url, app).await,
        Request::RemoveShortcut { id } => {
            let mut state = state.write().await;
            let state = &mut *state;
            let Some(shortcuts) = state.shortcuts.as_mut() else {
                return shortcuts_unavailable();
            };
            if shortcuts.find_by_id(&id).is_none() {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no shortcut has that id",
                ));
            }
            if !shortcuts.remove(&id) {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "Failed to remove link",
                ));
            }
            state.index.set_shortcuts(shortcuts.shortcuts().to_vec());
            shortcuts_response(shortcuts)
        }
        Request::OpenShortcut { id, arguments } => open_shortcut(state, &id, &arguments).await,
        Request::ListScripts => {
            let mut state = state.write().await;
            let state = &mut *state;
            state.scripts.rescan();
            let items = state.scripts.items();
            let scripts = items.iter().map(crate::scripts::entry).collect();
            state.index.set_scripts(items);
            Response::Scripts { scripts }
        }
        Request::ScriptIcons => {
            let icons = state.read().await.scripts.icons();
            Response::ScriptIcons { icons }
        }
        Request::RunScript { id, arguments } => run_script(state, &id, &arguments).await,
        Request::ScriptOutput { session } => {
            let runs = Arc::clone(&state.read().await.script_runs);
            let Some(run) = runs.get(session) else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no script run has that session",
                ));
            };
            let run = run.lock().map_err(|_| ()).ok();
            match run {
                Some(run) => Response::ScriptOutput {
                    output: String::from_utf8_lossy(&run.output).into_owned(),
                    finished: run.finished,
                    exit_code: run.exit_code,
                    elapsed_ms: u64::try_from(run.elapsed().as_millis()).unwrap_or(u64::MAX),
                },
                None => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "the script run's state is poisoned",
                )),
            }
        }
        Request::StopScript { session } => {
            state.read().await.script_runs.stop(session);
            Response::Ack
        }
        Request::Dmenu { spec } => {
            let (slot, dmenus) = {
                let state = state.read().await;
                (state.window_slot(), Arc::clone(&state.dmenus))
            };
            let (token, chosen) = dmenus.open(spec);
            match forward(&slot, WindowCommand::Dmenu(token), "show a dmenu list").await {
                Response::Ack => Response::DmenuOutput {
                    output: chosen.await.unwrap_or_default(),
                },
                refused => {
                    dmenus.choose(token, None);
                    refused
                }
            }
        }
        Request::OpenDeeplink { url } => {
            if let Some(link) = compass_core::root_items::parse_launch_link(&url) {
                return launch::open_launch_link(state, url, link).await;
            }
            if compass_core::settings_catalog::parse_settings_link(&url).is_some() {
                let slot = state.read().await.window_slot();
                return forward(&slot, WindowCommand::Deeplink(url), "open the settings").await;
            }
            match compass_core::develop_link::parse_develop_link(&url) {
                Some(Ok(link)) => return develop(state, &link).await,
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
        Request::DmenuFetch { token } => match state.read().await.dmenus.spec(token) {
            Some(spec) => Response::DmenuList { spec },
            None => Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no dmenu list has that token",
            )),
        },
        Request::DmenuChoose { token, output } => {
            if state.read().await.dmenus.choose(token, output) {
                Response::Ack
            } else {
                Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no dmenu list has that token",
                ))
            }
        }
        Request::ListFonts => {
            let fonts = Arc::clone(&state.read().await.fonts);
            let families = installed_fonts(&fonts).await;
            Response::Fonts {
                fonts: families.iter().map(crate::fonts::entry).collect(),
                categories: crate::fonts::category_names(),
            }
        }
        Request::FontSpecimen { name } => {
            let fonts = Arc::clone(&state.read().await.fonts);
            match installed_fonts(&fonts)
                .await
                .iter()
                .find(|f| f.name == name)
            {
                Some(family) => Response::Text {
                    text: crate::fonts::specimen(family),
                },
                None => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no installed font family has that name",
                )),
            }
        }
        Request::StoreBrowse { store, query } => {
            let stores = Arc::clone(&state.read().await.stores);
            match stores.browse(store, &query).await {
                Ok(listed) => Response::StoreListing {
                    heading: listed.heading,
                    entries: listed.entries,
                },
                Err(message) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            }
        }
        Request::StoreExtension { store, owner, name } => {
            let stores = Arc::clone(&state.read().await.stores);
            match stores.detail(store, &owner, &name).await {
                Ok(detail) => Response::StoreExtension { detail },
                Err(message) => Response::Error(ProtocolError::new(ErrorKind::BadRequest, message)),
            }
        }
        Request::StoreInstall { store, owner, name } => {
            let stores = Arc::clone(&state.read().await.stores);
            // A Raycast extension the overrides manifest replaces on Linux
            // installs its replacement instead.
            let (store, owner, name) = match (
                store,
                compass_core::raycast_overrides::Manifest::shipped().raycast_redirect(&name),
            ) {
                (compass_ipc::StoreKind::Raycast, Some(redirect))
                    if redirect.store == "vicinae" =>
                {
                    tracing::info!(
                        from = %name, to = %redirect.name, why = %redirect.why,
                        "installing the Linux-capable replacement of a Raycast extension"
                    );
                    (
                        compass_ipc::StoreKind::Vicinae,
                        redirect.author.clone(),
                        redirect.name.clone(),
                    )
                }
                _ => (store, owner, name),
            };
            match stores.install(store, &owner, &name).await {
                Ok((id, title)) => {
                    state.write().await.index.rescan_extensions();
                    Response::StoreInstalled { id, title }
                }
                Err(message) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            }
        }
        Request::StoreUninstall { id } => uninstall_extension(state, id).await,
        Request::OpenUrl { url } => {
            use compass_worker_host::application_service::Apps;
            if !crate::stores::is_openable_url(&url) {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "only http and https URLs are opened",
                ));
            }
            let apps = engine_apps(state).await;
            match crate::shortcuts::resolve_app(&apps, compass_core::shortcut::DEFAULT_APP_ID, &url)
            {
                Some(app) => {
                    apps.launch(&app, &url);
                    Response::Ack
                }
                None => Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    format!("No default app to open {url}"),
                )),
            }
        }
        Request::CreateExtension {
            author,
            title,
            description,
            location,
            command_title,
            command_description,
            template,
        } => {
            let form = compass_core::create_extension::Form {
                author,
                title,
                description,
                location,
                command_title,
                command_description,
                template_id: template,
            };
            match tokio::task::spawn_blocking(move || crate::developer::create_extension(&form))
                .await
            {
                Ok(response) => response,
                Err(error) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("creating the extension failed: {error}"),
                )),
            }
        }
        Request::SetFont { family } => {
            let family = family.trim().to_owned();
            if family.is_empty() {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "a font family is needed",
                ));
            }
            let saved = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let mut config = Config::load()?;
                config.set_font_family(&family);
                config.save_to(compass_core::config::default_config_path()?)?;
                Ok(())
            })
            .await;
            match saved {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(error)) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("could not save the font: {error}"),
                )),
                Err(error) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("saving the font failed: {error}"),
                )),
            }
        }
        Request::SetTheme { theme } => {
            let _ = tokio::task::spawn_blocking(compass_ui::theme::load_default_user_themes).await;
            let Some(parsed) = compass_ui::theme::Theme::from_name(&theme) else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("unknown theme {theme:?}"),
                ));
            };
            let saved = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let mut config = Config::load()?;
                config
                    .launcher_mut()
                    .appearance_mut()
                    .set_theme(Some(parsed.name().to_owned()));
                config.save_to(compass_core::config::default_config_path()?)?;
                Ok(())
            })
            .await;
            match saved {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(error)) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("could not save the theme: {error}"),
                )),
                Err(error) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("saving the theme failed: {error}"),
                )),
            }
        }
        Request::ListPrograms => {
            let default_action = state.read().await.run_program_default.clone();
            let apps = engine_apps(state).await;
            let programs = tokio::task::spawn_blocking(crate::programs::scan)
                .await
                .unwrap_or_default();
            Response::Programs {
                programs,
                terminal: apps.terminal_name(),
                default_action,
            }
        }
        Request::RunProgram {
            argv,
            terminal,
            hold,
        } => run_program(state, argv, terminal, hold).await,
        Request::ListSnippets => {
            let state = state.read().await;
            match &state.snippets {
                Some(snippets) => snippets_response(snippets),
                None => snippets_unavailable(),
            }
        }
        Request::SaveSnippet {
            id,
            name,
            text,
            keyword,
            word,
            apps,
        } => {
            let response = save_snippet(state, id, name, text, keyword, word, apps).await;
            sync_keywords(state).await;
            response
        }
        Request::RemoveSnippet { id } => {
            let response = {
                let mut state = state.write().await;
                let Some(snippets) = state.snippets.as_mut() else {
                    return snippets_unavailable();
                };
                match snippets.remove(&id) {
                    Ok(_) => snippets_response(snippets),
                    Err(error @ compass_core::snippet_store::Error::NoSuchSnippet) => {
                        Response::Error(ProtocolError::new(
                            ErrorKind::BadRequest,
                            error.to_string(),
                        ))
                    }
                    Err(error) => {
                        Response::Error(ProtocolError::new(ErrorKind::Internal, error.to_string()))
                    }
                }
            };
            sync_keywords(state).await;
            response
        }
        Request::InputServerStatus => input_server(state, None).await,
        Request::SetInputServerEnabled { enabled } => input_server(state, Some(enabled)).await,
        Request::PreviewSnippet { id, arguments } => {
            match expand_snippet(state, &id, &arguments, false).await {
                Ok(text) => Response::Text { text },
                Err(response) => response,
            }
        }
        Request::ExpandSnippet { id, arguments } => {
            match expand_snippet(state, &id, &arguments, true).await {
                Ok(text) => Response::Text { text },
                Err(response) => response,
            }
        }
        Request::PasteSnippet { id, arguments } => {
            const WHAT: &str = "Pasting";
            let text = match expand_snippet(state, &id, &arguments, true).await {
                Ok(text) => text,
                Err(response) => return response,
            };
            crate::paste::paste(state, compass_shell::ClipboardContent::text(text), WHAT).await
        }
        Request::PasteText { text } => paste_text(state, text).await,
        Request::CopyText { text } => copy_text(state, text).await,
        Request::ExpandShortcut { id, arguments } => {
            match expand_shortcut(state, &id, &arguments).await {
                Ok((_, expanded)) => Response::Text { text: expanded },
                Err(response) => response,
            }
        }
        Request::RunExtensionCommand { id, arguments_json } => {
            run_extension_command(state, id, arguments_json).await
        }
        Request::ExtensionView { session, after } => extension_view(state, session, after).await,
        Request::ExtensionEvent {
            session,
            handler,
            args_json,
        } => {
            let args: Vec<serde_json::Value> = match serde_json::from_str(&args_json) {
                Ok(args) => args,
                Err(err) => {
                    return Response::Error(ProtocolError::new(
                        ErrorKind::BadRequest,
                        format!("args_json is not a JSON array: {err}"),
                    ));
                }
            };
            let rhai = Arc::clone(&state.read().await.rhai);
            if rhai.has(session) {
                return match rhai.event(session, &handler, &args).await {
                    Ok(outcomes) => {
                        rhai_outcomes(state, outcomes).await;
                        Response::Ack
                    }
                    Err(reason) => {
                        Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                    }
                };
            }
            let views = Arc::clone(&state.read().await.views);
            match tokio::task::spawn_blocking(move || views.activate(session, &handler, &args))
                .await
            {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the extension event task failed: {err}"),
                )),
            }
        }
        Request::ExtensionAlertAnswer { session, confirmed } => {
            let rhai = Arc::clone(&state.read().await.rhai);
            if rhai.has(session) {
                return match rhai.answer(session, confirmed).await {
                    Ok(()) => Response::Ack,
                    Err(reason) => {
                        Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                    }
                };
            }
            let views = Arc::clone(&state.read().await.views);
            match tokio::task::spawn_blocking(move || views.answer_alert(session, confirmed)).await
            {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the alert answer task failed: {err}"),
                )),
            }
        }
        Request::ExtensionAlertRemember { session } => {
            let views = Arc::clone(&state.read().await.views);
            match tokio::task::spawn_blocking(move || views.answer_alert_always(session)).await {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the alert answer task failed: {err}"),
                )),
            }
        }
        Request::SetExtensionPreferences { id, values_json } => {
            set_extension_preferences(state, id, values_json).await
        }
        Request::ExtensionLaunchFetch { token } => match state.read().await.launches.take(token) {
            Some(launch) => match launch.fallback_text {
                Some(fallback_text) => Response::CommandLaunch {
                    id: launch.id,
                    arguments_json: launch.arguments_json,
                    fallback_text: Some(fallback_text),
                },
                None => Response::ExtensionLaunch {
                    id: launch.id,
                    arguments_json: launch.arguments_json,
                    preferences: launch.preferences,
                },
            },
            None => Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no launch is waiting under that token",
            )),
        },
        Request::ExtensionSubtitles => Response::ExtensionSubtitles {
            subtitles: state.read().await.launches.subtitles(),
        },
        Request::ExtensionPreferences { id } => extension_preferences(state, &id).await,
        Request::ExtensionPop { session } => {
            // A script shows one view; there is nothing above it to pop.
            if state.read().await.rhai.has(session) {
                return Response::Ack;
            }
            let views = Arc::clone(&state.read().await.views);
            match tokio::task::spawn_blocking(move || views.pop(session)).await {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the extension pop task failed: {err}"),
                )),
            }
        }
        Request::CloseExtension { session } => {
            let state = state.read().await;
            if !state.rhai.close(session) {
                state.views.close(session);
            }
            Response::Ack
        }
        Request::CatalogGeneration => Response::CatalogGeneration {
            generation: state.read().await.catalog_generation,
        },
        Request::ListDefaultApps { kind } => list_default_apps(state, kind).await,
        Request::SetDefaultApp { kind, id } => set_default_app(state, kind, &id).await,
        Request::ListScriptGrants => {
            let rhai = Arc::clone(&state.read().await.rhai);
            let titles = extension_titles(state).await;
            match tokio::task::spawn_blocking(move || all_grants(&rhai, &titles)).await {
                Ok(grants) => Response::ScriptGrants { grants },
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("reading script permissions failed: {err}"),
                )),
            }
        }
        Request::RevokeScriptGrant { id } => {
            let rhai = Arc::clone(&state.read().await.rhai);
            let titles = extension_titles(state).await;
            match tokio::task::spawn_blocking(move || {
                let hosts = crate::host_commands::Grants::from_environment();
                match hosts.revoke(&id) {
                    Ok(true) => {
                        tracing::info!(extension = %id, "host-program grants revoked");
                        Ok(())
                    }
                    Ok(false) => rhai.revoke(&id),
                    Err(err) => Err(format!("Compass could not change the permissions: {err}")),
                }
                .map(|()| all_grants(&rhai, &titles))
            })
            .await
            {
                Ok(Ok(grants)) => Response::ScriptGrants { grants },
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("revoking script permissions failed: {err}"),
                )),
            }
        }
        Request::ListRhaiScripts => {
            // A rescan, as `ListScripts` does: the watcher is the fast path,
            // this the one that cannot miss (a full inotify queue, a
            // directory created after start).
            let rhai = Arc::clone(&state.read().await.rhai);
            let scripts = match tokio::task::spawn_blocking(move || rhai.reload()).await {
                Ok(scripts) => scripts,
                Err(err) => {
                    return Response::Error(ProtocolError::new(
                        ErrorKind::Internal,
                        format!("the script scan failed: {err}"),
                    ));
                }
            };
            let entries = scripts
                .iter()
                .map(|script| compass_ipc::RhaiScriptEntry {
                    id: script.id.clone(),
                    title: script.title.clone(),
                    description: script.description.clone(),
                    icon: script.icon.clone(),
                    keywords: script.keywords.clone(),
                })
                .collect();
            state.write().await.set_rhai_items(scripts);
            Response::RhaiScripts { scripts: entries }
        }
        Request::OAuthRedirect { url } => match crate::extension_runner::oauth_redirect(&url) {
            Ok(()) => Response::Ack,
            Err(reason) => Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason)),
        },

        Request::ClipboardSetPinned { .. } | Request::ClipboardRemove { .. } => {
            let Some(store) = state.read().await.clipboard.clone() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    CLIPBOARD_UNAVAILABLE,
                ));
            };
            let changed = tokio::task::spawn_blocking(move || match request {
                Request::ClipboardSetPinned { id, pinned } => store.set_pinned(&id, pinned),
                Request::ClipboardRemove { id } => store.remove(&id),
                _ => unreachable!("matched above"),
            })
            .await;
            match changed {
                Ok(Ok(true)) => Response::Ack,
                Ok(Ok(false)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard update task failed: {err}"),
                )),
            }
        }

        Request::ClipboardPaste { id } => {
            const WHAT: &str = "Pasting";
            if let Some(refused) = crate::paste::no_clipboard(state, WHAT).await {
                return refused;
            }
            let store = state.read().await.clipboard.clone();
            let Some(store) = store else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    CLIPBOARD_UNAVAILABLE,
                ));
            };
            let (mime_type, data) =
                match tokio::task::spawn_blocking(move || store.content(&id)).await {
                    Ok(Ok(Some(content))) => content,
                    Ok(Ok(None)) => {
                        return Response::Error(ProtocolError::new(
                            ErrorKind::BadRequest,
                            "no clipboard history entry has that id",
                        ));
                    }
                    Ok(Err(err)) => {
                        return Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            err.to_string(),
                        ));
                    }
                    Err(err) => {
                        return Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            format!("clipboard content task failed: {err}"),
                        ));
                    }
                };
            let content = compass_shell::ClipboardContent::binary(data, mime_type);
            crate::paste::paste(state, content, WHAT).await
        }

        // Handled by the serve loop, which owns the shutdown signal; reaching
        // here means the loop did not intercept it.
        Request::Shutdown => Response::ShuttingDown,

        Request::Toggle | Request::Show | Request::Hide => {
            let (slot, command, what) = {
                let state = state.read().await;
                match request {
                    Request::Toggle => (
                        state.window_slot(),
                        WindowCommand::Toggle,
                        "toggle a window",
                    ),
                    Request::Show => (state.window_slot(), WindowCommand::Show, "show a window"),
                    _ => (state.window_slot(), WindowCommand::Hide, "hide a window"),
                }
            };
            // The read lock is released before the push: a window that takes a
            // moment to answer must not block queries from other clients.
            forward(&slot, command, what).await
        }

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
        let fonts = Arc::clone(&state.read().await.fonts);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            installed_fonts(&fonts).await;
        });
    }

    // The calculator's exchange rates: fetched when stale, then daily.
    tokio::spawn(crate::exchange_rates::run(Arc::clone(
        &state.read().await.exchange_rates,
    )));

    // Rhai scripts' hot reload.
    tokio::spawn(crate::rhai_scripts::watch(Arc::clone(&state)));

    // Other applications' tray icons, followed from start-up as the C++
    // host does, so Search Tray lists them at once.
    {
        let tray = Arc::clone(&state.read().await.tray);
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

/// Answers the tray requests from the engine's [`crate::tray_host::TrayHost`].
async fn tray_request(tray: &crate::tray_host::TrayHost, request: Request) -> Response {
    let refused = |reason: String| {
        let kind = if reason == crate::tray_host::UNAVAILABLE {
            ErrorKind::Unsupported
        } else {
            ErrorKind::Internal
        };
        Response::Error(ProtocolError::new(kind, reason))
    };
    let acked = |result: Result<(), String>| result.map_or_else(refused, |()| Response::Ack);
    match request {
        Request::TrayItems => tray
            .items()
            .await
            .map_or_else(refused, |items| Response::TrayItems { items }),
        Request::TrayActivate { key, secondary } => acked(tray.activate(&key, secondary).await),
        Request::TrayMenu { key } => tray
            .menu(&key)
            .await
            .map_or_else(refused, |entries| Response::TrayMenu { entries }),
        Request::TrayTriggerMenu { key, id } => acked(tray.trigger(&key, id).await),
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a tray request",
        )),
    }
}
