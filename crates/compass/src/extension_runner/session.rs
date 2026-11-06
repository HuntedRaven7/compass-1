//! Serving one run: the thread [`super::start`] hands the runtime to once it
//! has loaded the command.
//!
//! It wires the services the session answers through, pumps the session
//! until the runtime closes or crashes, and turns what defers into an answer:
//! an OAuth sign-in, a host program, an alert. Beside it is the watchdog that
//! ends a run, since the runtime never says a `no-view` command is done.
//! Separate from `start`, which only gets the command loaded, because this is
//! what lives for the command's whole run.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use compass_worker_host::Worker;
use compass_worker_host::application_service::ApplicationService;
use compass_worker_host::browser_service::BrowserService;
use compass_worker_host::clipboard_service::ClipboardService;
use compass_worker_host::command_service::CommandService;
use compass_worker_host::file_search_service::FileSearchService;
use compass_worker_host::host_command_service::HostCommandService;
use compass_worker_host::oauth_service::{AuthorizeService, OAuthService};
use compass_worker_host::session::{Router, Session, Turn};
use compass_worker_host::storage_service::StorageService;
use compass_worker_host::ui_service::UiService;
use compass_worker_host::ui_shell_service::{CommandInfo, UiShellService};
use compass_worker_host::wallpaper_service::WallpaperService;

use super::clipboard::ShellClipboard;
use super::oauth::{EngineAuthorizer, OAUTH, begin_authorization};
use super::shell::{HeadlessShell, Selection};
use super::storage::{Storage, open_storage};
use super::views::ViewHandle;
use super::{IDLE, Launcher};

pub(super) struct Served {
    pub(super) session_id: String,
    pub(super) extension_id: String,
    pub(super) extension_title: String,
    pub(super) title: String,
    pub(super) name: String,
    pub(super) namespace: String,
    pub(super) apps: Option<crate::extension_apps::EngineApps>,
    pub(super) files: Option<Arc<crate::file_search::FileSearch>>,
    pub(super) commands: Option<crate::extension_commands::EngineCommands>,
    pub(super) assets: PathBuf,
    pub(super) launcher: Option<Launcher>,
}

pub(super) fn serve(
    worker: Worker,
    served: Served,
    storage: Option<Storage>,
    clipboard: ShellClipboard,
    handle: Option<tokio::runtime::Handle>,
    activity: &Activity,
    view: Option<ViewHandle>,
) {
    let Served {
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
    } = served;
    let title = title.as_str();
    let clipboard_handle = clipboard.handle.clone();
    let windows = compass_worker_host::window_service::WindowService::new(
        crate::extension_windows::EngineWindows::detect(
            clipboard.shell.clone(),
            clipboard.handle.clone(),
            apps.as_ref()
                .map(crate::extension_apps::EngineApps::window_classes)
                .unwrap_or_default(),
        ),
    );
    let selection = Selection {
        shell: clipboard.shell.clone(),
        handle: clipboard.handle.clone(),
    };
    let database = storage.and_then(|storage| open_storage(&storage));
    let local = database
        .as_ref()
        .map(compass_local_storage::LocalStorage::new);
    let scoped = local.as_ref().map(|local| local.scoped(&namespace));
    let storage_service = scoped.map(StorageService::new);
    let shell = UiShellService::new(
        HeadlessShell {
            title: title.to_owned(),
            handle,
            alert: std::sync::Mutex::new(None),
            view: view.as_ref().map(|view| view.state.clone()),
            selection,
            assets: Some(assets),
            launcher: if view.is_none() { launcher } else { None },
        },
        CommandInfo {
            name: name.to_owned(),
            icon: String::new(),
        },
    );
    let clipboard = ClipboardService::new(clipboard);
    let ui = UiService::new();
    let mut router = Router::new()
        .with(&shell)
        .with(&clipboard)
        .with(&ui)
        .with(&windows);
    if let Some(service) = &storage_service {
        router = router.with(service);
    }
    let applications = apps.map(ApplicationService::new);
    if let Some(service) = &applications {
        router = router.with(service);
    }
    let tokens = database.as_ref().map(|db| {
        OAuthService::new(
            compass_oauth_store::TokenStore::new(db),
            extension_id.as_str(),
        )
    });
    if let Some(service) = &tokens {
        router = router.with(service);
    }
    let authorize = AuthorizeService::new(EngineAuthorizer::default());
    router = router.with(&authorize);
    let file_search =
        files.map(|files| FileSearchService::new(crate::extension_files::EngineFiles::new(files)));
    if let Some(service) = &file_search {
        router = router.with(service);
    }
    let wallpaper = WallpaperService::new(crate::extension_wallpaper::EngineWallpaper::new(
        clipboard_handle.clone(),
    ));
    router = router.with(&wallpaper);
    let browser = BrowserService::new(crate::extension_browser::NoBrowsers);
    router = router.with(&browser);
    let command_service = commands.map(CommandService::new);
    if let Some(service) = &command_service {
        router = router.with(service);
    }
    let broker = crate::host_commands::SessionBroker::new(
        extension_id.as_str(),
        if extension_title.is_empty() {
            title
        } else {
            extension_title.as_str()
        },
        crate::host_commands::Grants::from_environment(),
        crate::host_commands::Runner::detect(),
        compass_core::raycast_overrides::Manifest::shipped(),
    );
    let host_commands = HostCommandService::new(&*broker);
    router = router.with(&host_commands);
    let mut session = Session::new(worker, session_id.as_str(), router);
    if let Some(view) = &view {
        view.attach(session.events());
    }
    let mut ended = None;
    // The last call this host refused, with why: an extension that lets the
    // refusal escape crashes with whatever its runtime makes of it, which
    // has been no more than "undefined".
    let mut refused_last: Option<String> = None;
    loop {
        let turn = match session.pump_once() {
            Ok(turn) => turn,
            Err(err) => {
                if !activity.stopped() {
                    tracing::warn!(command = title, error = %err, "extension session ended");
                    ended = Some(format!("{title} stopped: {err}"));
                }
                break;
            }
        };
        activity.touch();
        match turn {
            Turn::Closed => break,
            Turn::Crashed { reason } => {
                tracing::warn!(command = title, %reason, "extension command crashed");
                let reason = crash_reason(&reason, refused_last.take());
                ended = Some(format!("{title} crashed: {reason}"));
                break;
            }
            Turn::Answered { method } if method == "UI/render" => {
                if let (Some(view), Some(root)) = (&view, ui.top()) {
                    let depth = u32::try_from(ui.stack().len()).unwrap_or(u32::MAX);
                    view.publish(compass_worker_host::view_model::to_view(&root), depth);
                }
            }
            Turn::Deferred { method, deferral } if method == "OAuth/authorize" => {
                let request = authorize
                    .authorizer()
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                let events = session.events();
                let refused = match request {
                    Some(request) => begin_authorization(
                        &session_id,
                        request,
                        deferral.clone(),
                        events,
                        view.as_ref().map(|view| view.state.clone()),
                        applications.as_ref().map(ApplicationService::apps),
                    )
                    .err(),
                    None => Some("the authorization request was lost".to_owned()),
                };
                if let Some(reason) = refused {
                    tracing::warn!(command = title, %reason, "refused an authorization");
                    if let Err(err) = session.fail_deferred(&deferral, &reason) {
                        tracing::warn!(command = title, error = %err, "could not refuse an authorization");
                        break;
                    }
                    refused_last = Some(reason);
                }
            }
            Turn::Deferred { method, deferral } if method == "HostCommand/run" => {
                match broker.take() {
                    Some(request) => broker.dispatch(
                        request,
                        deferral,
                        Arc::new(session.events()),
                        view.as_ref()
                            .map(|view| view as &dyn crate::host_commands::Asker),
                    ),
                    None => {
                        if let Err(err) =
                            session.fail_deferred(&deferral, "the host command request was lost")
                        {
                            tracing::warn!(command = title, error = %err, "could not refuse a host command");
                            break;
                        }
                    }
                }
            }
            Turn::Deferred { method, deferral } => {
                // Otherwise only an alert defers. A view shows it and the launcher
                // answers; a command with no view has nowhere to show it, and
                // "no" is the answer a dismissed alert gives.
                let alert = shell
                    .shell()
                    .alert
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let (Some(view), Some(alert)) = (&view, alert) {
                    view.ask(alert, deferral);
                    continue;
                }
                tracing::info!(command = title, %method, "answered a dialog with no");
                if let Err(err) = session.answer_deferred(&deferral, serde_json::json!(false)) {
                    tracing::warn!(command = title, error = %err, "could not answer a dialog");
                    break;
                }
            }
            Turn::Answered { .. } | Turn::Nothing | Turn::OtherSession { .. } => {}
        }
    }
    activity.stop();
    OAUTH.abandon(&session_id);
    if let Some(view) = view {
        view.end(ended);
    } else if let Some(reason) = ended {
        shell.shell().fail(&reason, "");
    }
}

/// What a crash says: the runtime's reason, unless it said nothing useful,
/// and then the refusal that most likely caused it.
fn crash_reason(reason: &str, refused: Option<String>) -> String {
    let said_nothing = matches!(reason.trim(), "" | "undefined" | "null" | "[object Object]");
    match refused {
        Some(refused) if said_nothing => refused,
        _ => reason.to_owned(),
    }
}

/// When a run last said anything, and whether it has been stopped.
pub(super) struct Activity {
    started: Instant,
    last: AtomicU64,
    deadline: AtomicU64,
    stopped: AtomicBool,
}

impl Activity {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            last: AtomicU64::new(0),
            deadline: AtomicU64::new(u64::MAX),
            stopped: AtomicBool::new(false),
        }
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    pub(super) fn touch(&self) {
        self.last.store(self.elapsed_ms(), Ordering::Relaxed);
    }

    /// From now on, the run is over after [`IDLE`] of quiet or `lifetime`.
    pub(super) fn run_for(&self, lifetime: Duration) {
        let lifetime = u64::try_from(lifetime.as_millis()).unwrap_or(u64::MAX);
        self.deadline.store(
            self.elapsed_ms().saturating_add(lifetime),
            Ordering::Relaxed,
        );
    }

    /// A view's run: over only when stopped.
    pub(super) fn run_forever(&self) {
        self.deadline.store(u64::MAX - 1, Ordering::Relaxed);
        self.last.store(u64::MAX, Ordering::Relaxed);
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// Whether the run should be ended now. Before [`Self::run_for`], only
    /// `handshake` bounds it.
    fn expired(&self, handshake: Duration) -> bool {
        let now = self.elapsed_ms();
        let deadline = self.deadline.load(Ordering::Relaxed);
        if deadline == u64::MAX {
            return now > u64::try_from(handshake.as_millis()).unwrap_or(u64::MAX);
        }
        if deadline == u64::MAX - 1 {
            return false;
        }
        let idle = u64::try_from(IDLE.as_millis()).unwrap_or(u64::MAX);
        now > deadline || now.saturating_sub(self.last.load(Ordering::Relaxed)) > idle
    }
}

/// Stops the runtime at `pid` once the run is over, which closes its output
/// and ends [`serve`]'s loop.
pub(super) fn watch(pid: u32, activity: Arc<Activity>, handshake: Duration) {
    let spawned = std::thread::Builder::new()
        .name("extension watchdog".to_owned())
        .spawn(move || {
            while !activity.stopped() {
                if activity.expired(handshake) {
                    activity.stop();
                    // The runtime is our own child; SIGTERM lets Node end its
                    // worker threads, and `serve` then reads end-of-file.
                    // kill(1) rather than kill(2): this crate forbids unsafe.
                    let _ = std::process::Command::new("kill")
                        .arg("-TERM")
                        .arg(pid.to_string())
                        .status();
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(error = %err, "no watchdog for an extension run");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_that_says_nothing_is_told_by_the_refusal_before_it() {
        let refused = Some("No web browser is installed to sign in with".to_owned());
        assert_eq!(
            crash_reason("undefined", refused.clone()),
            "No web browser is installed to sign in with"
        );
        assert_eq!(
            crash_reason("  ", refused.clone()),
            refused.clone().unwrap()
        );
        assert_eq!(
            crash_reason("Error: boom\n    at x", refused),
            "Error: boom\n    at x",
            "a crash with its own reason keeps it"
        );
        assert_eq!(crash_reason("undefined", None), "undefined");
    }
}
