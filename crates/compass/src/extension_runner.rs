//! Runs installed extensions' `no-view` commands.
//!
//! The first slice of the extension host in the engine (Phase 4, #7). A run
//! starts the extension runtime (the bundle the C++ engine ships as
//! `vicinae-worker-ts`) under Node, loads the command and handshakes, and then
//! serves the session on a thread of its own:
//! - local storage, from Compass's own encrypted database;
//! - HUDs, failure toasts and notifications, as desktop notifications;
//! - alerts, answered "no", because nothing can show one yet.
//!
//! What it does not do yet, and says so rather than failing obscurely:
//! - draw a `view` command (there is no view renderer in the launcher);
//! - collect preferences (a required one without a default is refused by
//!   name).
//!
//! # When a run is over
//!
//! The runtime never tells the host that a `no-view` command finished: it
//! unloads the command's worker itself and says nothing (the C++ has a FIXME
//! for exactly this). So a run ends when the runtime has been quiet for
//! [`IDLE`], or at [`LIFETIME`] regardless, and the runtime process is then
//! stopped. One runtime per run, so a stuck command cannot hold up the next.
//!
//! # Layout
//!
//! This module finds the runtime and starts a run; what a run needs once it
//! has started lives beside it, one concern to a file:
//! - `sandbox`: what the runtime may read, write and execute;
//! - `storage`: the encrypted local-storage database, and the preference
//!   values kept in it;
//! - `session`: the serving thread, and the watchdog that ends a run;
//! - `shell`: what a command would show on screen, as notifications or as a
//!   view's toasts;
//! - `views`: running view commands, as the launcher follows and answers them;
//! - `oauth`: sign-ins waiting on a browser's redirect;
//! - `clipboard`: the clipboard, through GNOME Shell or data-control.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use compass_core::extension_commands::ExtensionCommand;
use compass_core::manifest::CommandMode;
use compass_worker_host::Worker;
use compass_worker_host::extension_manager::{
    Capabilities, CommandEnv, LaunchType, LoadOptions, ManagerClient,
};
use compass_worker_host::file_search_service::FileIndexer;

mod clipboard;
mod oauth;
mod sandbox;
mod session;
mod shell;
mod storage;
mod views;

pub(crate) use clipboard::ShellClipboard;
pub use oauth::{SIGN_IN_TOAST, oauth_redirect};
pub use sandbox::{assets_dir, policy, policy_in, support_dir, tmp_dir};
use session::{Activity, Served, serve, watch};
pub use shell::NO_SELECTED_TEXT;
pub(crate) use storage::open_storage;
pub use storage::{
    Storage, clear_extension_data, load_preferences, preferences_namespace, save_preferences,
};
pub use views::{Answer, ViewState, Views};

/// Overrides where the runtime bundle is looked for.
pub const RUNTIME_ENV: &str = "COMPASS_EXTENSION_RUNTIME";

/// Overrides which Node runs it.
pub const NODE_ENV: &str = "COMPASS_NODE";

/// Overrides where `compass-sandbox-exec` is looked for.
pub const SANDBOX_EXEC_ENV: &str = "COMPASS_SANDBOX_EXEC";

/// Set to `off` to run extensions unconfined, for development on a machine
/// without the launcher. Anything else, or unset, means confined or refused.
pub const SANDBOX_SWITCH_ENV: &str = "COMPASS_EXTENSION_SANDBOX";

/// The sandbox launcher's file name.
pub const SANDBOX_EXEC_NAME: &str = "compass-sandbox-exec";

/// The bundle's file name wherever it is installed.
pub const RUNTIME_FILE_NAME: &str = "extension-runtime.js";

/// Quiet this long after a command's last message, and it is taken as done.
pub const IDLE: Duration = Duration::from_secs(10);

/// No `no-view` run lives longer than this.
pub const LIFETIME: Duration = Duration::from_secs(300);

/// Compass's own database for extensions' local storage (ADR-0017: Compass
/// owns its files; Vicinae's `vicinae.db` is never opened).
pub const STORAGE_DATABASE: &str = "compass-extension-storage.db";

/// How long the runtime has to answer `load`.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(10);

/// What runs extensions: Node and the runtime bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    /// The Node executable.
    pub node: PathBuf,
    /// The runtime bundle it runs.
    pub bundle: PathBuf,
    /// `compass-sandbox-exec`, which confines the runtime before it starts;
    /// `None` only when [`SANDBOX_SWITCH_ENV`] is `off`.
    pub sandbox: Option<PathBuf>,
}

impl Runtime {
    /// Finds Node and the bundle: the environment overrides first, then an
    /// installed bundle beside the executable (`../share/compass/`) or in the
    /// Flatpak's `/app/share/compass/`, and Node on `PATH`.
    ///
    /// # Errors
    ///
    /// A sentence naming what is missing, for the launcher to show.
    pub fn locate() -> Result<Self, String> {
        if std::env::var_os(crate::cli_commands::NO_EXTENSION_RUNTIME_ENV)
            .is_some_and(|value| !value.is_empty())
        {
            return Err(
                "TypeScript extensions are off: the engine was started with \
                        --no-extension-runtime"
                    .to_owned(),
            );
        }
        let bundle = std::env::var_os(RUNTIME_ENV)
            .map(PathBuf::from)
            .or_else(installed_bundle)
            .filter(|path| path.is_file())
            .ok_or_else(|| {
                "Running extensions needs the extension runtime, which this install of \
                 Compass does not include"
                    .to_owned()
            })?;
        let node = std::env::var_os(NODE_ENV)
            .map(PathBuf::from)
            .or_else(|| on_path("node"))
            .ok_or_else(|| "Running extensions needs Node.js, and none was found".to_owned())?;
        let sandbox = if std::env::var_os(SANDBOX_SWITCH_ENV).is_some_and(|v| v == "off") {
            tracing::warn!("{SANDBOX_SWITCH_ENV}=off: extensions run unconfined");
            None
        } else {
            Some(
                std::env::var_os(SANDBOX_EXEC_ENV)
                    .map(PathBuf::from)
                    .or_else(installed_sandbox)
                    .filter(|path| path.is_file())
                    .ok_or_else(|| {
                        "Running extensions needs compass-sandbox-exec, which this install of \
                         Compass does not include, and Compass will not run them unconfined"
                            .to_owned()
                    })?,
            )
        };
        Ok(Self {
            node,
            bundle,
            sandbox,
        })
    }
}

/// Beside the executable, in the directories the file indexer is looked for
/// in (`../libexec/compass`, `../lib/compass`: the AppImage, Nix and Arch
/// layouts), then the Flatpak's `/app/libexec`.
fn installed_sandbox() -> Option<PathBuf> {
    let installed = std::env::current_exe().ok().and_then(|exe| {
        Some(crate::indexer_client::helper_candidates(
            exe.parent()?,
            SANDBOX_EXEC_NAME,
        ))
    });
    installed
        .into_iter()
        .flatten()
        .chain([Path::new("/app/libexec").join(SANDBOX_EXEC_NAME)])
        .find(|path| path.is_file())
}

fn installed_bundle() -> Option<PathBuf> {
    let beside_exe = std::env::current_exe().ok().and_then(|exe| {
        Some(
            exe.parent()?
                .parent()?
                .join("share/compass")
                .join(RUNTIME_FILE_NAME),
        )
    });
    beside_exe
        .into_iter()
        .chain([Path::new("/app/share/compass").join(RUNTIME_FILE_NAME)])
        .find(|path| path.is_file())
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

/// Starts `command` and returns once the runtime has loaded it; the run
/// continues on its own thread. Blocking: call it off the async runtime.
///
/// `data_dir` is where the runtime keeps each extension's support and asset
/// directories (`vicinae_path`). `storage` is `None` when there is no keyring;
/// the command then gets "not implemented" for local storage.
///
/// # Errors
///
/// A sentence for the launcher: the command cannot run here, the runtime
/// would not start, or it did not accept the command.
pub fn start(
    runtime: &Runtime,
    command: &ExtensionCommand,
    data_dir: &Path,
    host: Host,
) -> Result<Started, String> {
    let Host {
        storage,
        shell,
        views,
        preferences,
        arguments,
        apps,
        files,
        commands,
        context,
        launcher,
    } = host;

    // The runtime creates these itself, but a sandbox can only grant a path
    // that exists, so they are made first.
    for dir in [tmp_dir(data_dir, command), assets_dir(data_dir, command)] {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::info!(dir = %dir.display(), %err, "could not prepare an extension directory");
        }
    }
    let bundle = [
        compass_worker_host::cgroups::node_heap_flag(),
        runtime.bundle.to_string_lossy().into_owned(),
    ];
    let mut process = match &runtime.sandbox {
        Some(launcher) => {
            policy(runtime, command, data_dir).command(launcher, &runtime.node, &bundle)
        }
        None => {
            let mut process = std::process::Command::new(&runtime.node);
            process.args(bundle);
            process
        }
    };
    process.env("TMPDIR", tmp_dir(data_dir, command));
    // `environment.vicinaeVersion` in the SDK. The runtime reads these, and
    // the `VICINAE_*` names the C++ set only when they are absent.
    process.env("COMPASS_VERSION", crate::developer::git_tag());
    process.env(
        "COMPASS_COMMIT",
        option_env!("COMPASS_GIT_COMMIT").unwrap_or("unknown"),
    );
    let spawned = Worker::spawn(process);
    let mut worker =
        spawned.map_err(|err| format!("The extension runtime would not start: {err}"))?;
    let pid = worker.pid();
    confine_memory(pid, &command.extension_id);

    // The watchdog also bounds the handshake: a runtime that never answers
    // `load` is stopped, which ends the read below.
    let activity = Arc::new(Activity::new());
    watch(pid, Arc::clone(&activity), LOAD_TIMEOUT);

    let options = LoadOptions {
        mode: match command.mode {
            CommandMode::View => compass_worker_host::extension_manager::CommandMode::View,
            CommandMode::NoView => compass_worker_host::extension_manager::CommandMode::NoView,
        },
        env: CommandEnv::Production,
        vicinae_path: data_dir.to_string_lossy().into_owned(),
        entrypoint: command.entrypoint.to_string_lossy().into_owned(),
        is_raycast: command.is_raycast,
        command_name: command.name.clone(),
        extension_id: command.extension_id.clone(),
        extension_name: command.extension_name.clone(),
        owner_or_author_name: command.owner_or_author().to_owned(),
        arguments,
        preferences,
        launch_context: context.as_ref().map_or(serde_json::Value::Null, |context| {
            context.launch_context.clone()
        }),
        launch_type: LaunchType::User,
        capabilities: capabilities(shell.is_some(), files.as_ref()),
        fallback_text: context.and_then(|context| context.fallback_text),
        cwd: None,
    };
    let load_id = ManagerClient::new(&mut worker)
        .load(&options)
        .map_err(|err| format!("The extension runtime did not take the command: {err}"))?;
    let session_id = loop {
        let message = worker
            .next_message()
            .map_err(|err| format!("The extension runtime failed: {err}"))?
            .ok_or_else(|| "The extension runtime stopped before loading the command".to_owned())?;
        if message.id != Some(load_id) {
            continue;
        }
        break message
            .result
            .as_ref()
            .and_then(|result| result.get("session_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("The extension runtime would not load {}", command.title))?
            .to_owned();
    };
    ManagerClient::new(&mut worker)
        .ready(&session_id)
        .map_err(|err| format!("The extension runtime failed: {err}"))?;
    activity.touch();
    // A view lives until the person leaves it; only a no-view run is timed.
    let view = (command.mode == CommandMode::View).then(|| views.open(pid));
    if view.is_some() {
        activity.run_forever();
    } else {
        activity.run_for(LIFETIME);
    }

    let title = command.title.clone();
    let name = command.name.clone();
    let namespace = compass_local_storage::namespace_for(&command.extension_id);
    let extension_id = command.extension_id.clone();
    let extension_title = command.extension_title.clone();
    let assets = command.extension_dir.join("assets");
    let handle = tokio::runtime::Handle::try_current().ok();
    let started = view
        .as_ref()
        .map_or(Started::Ran, |view| Started::View(view.session));
    std::thread::Builder::new()
        .name(format!("extension {}", command.id))
        .spawn(move || {
            serve(
                worker,
                Served {
                    session_id,
                    extension_id,
                    extension_title,
                    title,
                    name,
                    namespace,
                    apps,
                    files,
                    commands,
                    assets,
                    launcher,
                },
                storage,
                ShellClipboard {
                    shell,
                    handle: handle.clone(),
                },
                handle,
                &activity,
                view,
            );
        })
        .map_err(|err| format!("could not start a thread for the command: {err}"))?;
    Ok(started)
}

/// Caps the worker's memory on the user's systemd, where it is reachable.
/// The heap flag already bounds the JavaScript side; this is the rest, and a
/// host without it (a Flatpak, a container) still runs the command.
fn confine_memory(pid: u32, extension_id: &str) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let scope = compass_worker_host::cgroups::scope_name(extension_id, pid);
    handle.spawn(async move {
        match compass_worker_host::cgroups::confine(pid, &scope).await {
            Ok(()) => tracing::debug!(%scope, "extension worker memory capped"),
            Err(err) => tracing::info!(
                %err,
                "no systemd user manager to cap the extension's memory; the heap cap still holds"
            ),
        }
    });
}

/// What one run needs from the engine beyond the command itself.
#[derive(Debug, Default)]
pub struct Host {
    /// Local storage, or `None` without a keyring.
    pub storage: Option<Storage>,
    /// The GNOME Shell extension, which is the clipboard on GNOME.
    pub shell: Option<Arc<compass_shell::ShellClient>>,
    /// Where a view command's session is published.
    pub views: Arc<Views>,
    /// The preference values the command reads, already resolved.
    pub preferences: serde_json::Value,
    /// The argument values it was launched with.
    pub arguments: serde_json::Value,
    /// What `open()` and `getApplications()` reach, or `None` to refuse them.
    pub apps: Option<crate::extension_apps::EngineApps>,
    /// The file index `FileSearch/search` asks, or `None` for no index.
    pub files: Option<Arc<crate::file_search::FileSearch>>,
    /// What `Command/*` reaches, or `None` to refuse it.
    pub commands: Option<crate::extension_commands::EngineCommands>,
    /// The launch context and fallback text another command launched this
    /// one with.
    pub context: Option<crate::extension_commands::Context>,
    /// The launcher window, where a no-view command's HUD and failures go,
    /// or `None` to send them only as desktop notifications.
    pub launcher: Option<Launcher>,
}

/// Hands the launcher window a command (a HUD line, a failure), answering
/// whether it took it. A no-view command has no view to show its failure
/// in, and a desktop notification is lost where no notification daemon
/// runs; the launcher is there whenever Compass is.
#[derive(Clone)]
pub struct Launcher(pub Arc<dyn Fn(compass_ipc::WindowCommand) -> bool + Send + Sync>);

impl std::fmt::Debug for Launcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Launcher")
    }
}

/// `opts.capabilities`, as `ExtensionCommandRuntime` fills it: no browser
/// is ever connected (ADR-0008); windows where a backend lists them; the
/// wallpaper where a backend sets it; files where the indexer runs.
/// Blocking: it may probe the compositor and the wallpaper daemons once.
fn capabilities(shell: bool, files: Option<&Arc<crate::file_search::FileSearch>>) -> Capabilities {
    let windows = crate::wlroots::compositor().is_some()
        || crate::wlroots::session().is_some_and(|wlroots| wlroots.toplevels.is_some())
        || shell;
    Capabilities {
        browser_extension: false,
        window_management: windows,
        wallpaper: crate::extension_wallpaper::EngineWallpaper::new(
            tokio::runtime::Handle::try_current().ok(),
        )
        .can_set(),
        file_search: files.is_some_and(|files| {
            crate::extension_files::EngineFiles::new(Arc::clone(files)).is_available()
        }),
    }
}

/// How a run began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    /// A no-view command, running on its own.
    Ran,
    /// A view command: the launcher follows this session in [`Views`].
    View(u64),
}
