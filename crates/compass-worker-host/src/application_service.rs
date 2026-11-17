//! The `Application` half of the extension API.
//!
//! Ports `ExtApplicationService`
//! (`src/server/src/extension/api/application-service.hpp`): five methods over
//! the application database, which sits behind [`Apps`](crate::application_service::Apps) here for the same
//! reason `AppService` is injected there.
//!
//! The database itself is `compass-core`'s [`crate::application_service`]-free
//! business — `AppIndex` already reads the desktop entries, and `compass-xdg`'s
//! `mimeapps` already resolves default openers. What this module owns is the
//! translation: which query each method makes, what it does when the query
//! comes back empty, and the exact shape of an `Application` on the wire.
//!
//! Two calls differ from upstream, which has no sandbox and so gives an
//! extension nothing it did not already have. Here the worker is confined and
//! the engine is not, so:
//!
//! - `runInTerminal` runs nothing and answers `false` ([`TERMINAL_REFUSED`](crate::application_service::TERMINAL_REFUSED)).
//!   The engine would start the command line on the host, unconfined, and
//!   only `HostCommand/run` may do that, for a program the overrides list
//!   names and after asking the person.
//! - `open` of a file the extension could have written (under its support or
//!   assets directory, see [`ApplicationService::with_writable`](crate::application_service::ApplicationService::with_writable)) goes only to
//!   the default opener, and only for a type a viewer shows
//!   ([`is_viewer_type`](crate::application_service::is_viewer_type)): an `appId` naming `java -jar %f` or `wine` would
//!   otherwise run a file the extension wrote.

use std::path::{Path, PathBuf};

use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &[
    "Application/list",
    "Application/open",
    "Application/getDefault",
    "Application/showInFileBrowser",
    "Application/runInTerminal",
];

/// The error `getDefault` fails with, verbatim from the C++.
pub const NO_DEFAULT: &str = "No default application found";

/// Why `runInTerminal` answered `false`. Extensions see the `false`; this is
/// for the engine's log and the documentation.
pub const TERMINAL_REFUSED: &str = "Compass does not run terminal commands for extensions: a \
     command line would run on your computer outside the extension sandbox. Extensions run \
     host programs through HostCommand/run, which asks first.";

/// One application, as `appToTsapi` builds it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Application {
    /// The desktop-entry id, e.g. `org.gnome.Nautilus.desktop`.
    pub id: String,
    /// `displayName()`.
    pub name: String,
    /// `iconUrl()`, stringified. Optional in the IDL but never omitted here,
    /// because the C++ assigns it unconditionally.
    pub icon: String,
    /// The path to the desktop entry.
    pub path: String,
}

/// `LaunchTerminalCommandOptions`, as `runInTerminal` fills it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalOptions {
    /// Keep the terminal open after the command exits.
    pub hold: bool,
    /// Use this terminal rather than the preferred one.
    pub app_id: Option<String>,
    /// The window title to ask for.
    pub title: Option<String>,
    /// Where to run.
    pub working_directory: Option<String>,
}

/// The application database an extension can reach.
pub trait Apps {
    /// Every known application.
    fn list(&self) -> Vec<Application>;

    /// Every application that can open `target`, best first.
    fn openers(&self, target: &str) -> Vec<Application>;

    /// The preferred application for `target`.
    fn default_opener(&self, target: &str) -> Option<Application>;

    /// The application with this desktop-entry id.
    fn by_id(&self, id: &str) -> Option<Application>;

    /// Launches `app` with `target` as its sole argument.
    fn launch(&self, app: &Application, target: &str);

    /// Opens the file browser at `target`, selecting the entry when asked.
    fn show_in_file_browser(&self, target: &str, select: bool);

    /// Runs `cmdline` in a terminal; the bool is what the method returns.
    fn run_in_terminal(&self, cmdline: &[String], options: &TerminalOptions) -> bool;
}

/// Serves `Application` from one database.
#[derive(Debug)]
pub struct ApplicationService<A> {
    apps: A,
    /// The directories the extension may write, resolved.
    writable: Vec<PathBuf>,
}

impl<A: Apps> ApplicationService<A> {
    /// Serves `apps`.
    pub const fn new(apps: A) -> Self {
        Self {
            apps,
            writable: Vec::new(),
        }
    }

    /// Treats a file under any of `dirs`, the directories the extension may
    /// write, as one it may have written: `open` ignores `appId` for it and
    /// opens it only when [`is_viewer_type`] says a viewer shows it.
    #[must_use]
    pub fn with_writable(mut self, dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        self.writable = dirs
            .into_iter()
            .map(|dir| dir.canonicalize().unwrap_or(dir))
            .collect();
        self
    }

    /// The database this serves.
    pub const fn apps(&self) -> &A {
        &self.apps
    }

    /// Answers `call`, or `None` if it is not an `Application` call.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        let string = |name: &str| {
            call.params
                .get(name)
                .filter(|value| !value.is_null())
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };

        Some(match call.method.as_str() {
            "Application/list" => {
                // `target ? findOpeners(target) : list()` -- an absent target
                // lists everything, a present one narrows to what can open it.
                let apps = match string("target") {
                    Some(target) => self.apps.openers(&target),
                    None => self.apps.list(),
                };
                let apps: Vec<serde_json::Value> = apps.iter().map(application).collect();
                tsapi::reply(id, serde_json::Value::Array(apps))
            }
            "Application/open" => {
                let target = string("target").unwrap_or_default();
                self.open(&target, string("appId").as_deref());
                tsapi::reply(id, serde_json::Value::Null)
            }
            "Application/getDefault" => {
                let target = string("target").unwrap_or_default();
                match self.apps.default_opener(&target) {
                    Some(app) => tsapi::reply(id, application(&app)),
                    None => tsapi::reply_error(id, NO_DEFAULT),
                }
            }
            "Application/showInFileBrowser" => {
                let target = string("target").unwrap_or_default();
                let select = call
                    .params
                    .get("select")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                self.apps.show_in_file_browser(&target, select);
                tsapi::reply(id, serde_json::Value::Null)
            }
            // `Application/runInTerminal`: refused, see [`TERMINAL_REFUSED`].
            // A `false` result, which an extension written for upstream reads
            // as a terminal that would not start, not a rejected promise.
            _ => tsapi::reply(id, serde_json::Value::Bool(false)),
        })
    }

    /// `open`'s two-step lookup.
    ///
    /// The `appId` branch returns *only if the id resolves*: the C++ `if (auto
    /// app = findById(...))` falls through when it does not, so an extension
    /// naming an application that is not installed still gets the file opened
    /// by the default handler rather than nothing happening.
    fn open(&self, target: &str, app_id: Option<&str>) {
        if let Some(file) = self.written_by_extension(target) {
            if is_viewer_type(&file)
                && let Some(opener) = self.apps.default_opener(target)
            {
                self.apps.launch(&opener, target);
            }
            return;
        }
        if let Some(app) = app_id.and_then(|id| self.apps.by_id(id)) {
            self.apps.launch(&app, target);
            return;
        }
        if let Some(opener) = self.apps.default_opener(target) {
            self.apps.launch(&opener, target);
        }
    }
}

impl<A> ApplicationService<A> {
    /// The local file `target` names, when it is under a directory the
    /// extension may write. Links are resolved first, so a link there to a
    /// file elsewhere is that file, and a link elsewhere to a file there is
    /// this one.
    fn written_by_extension(&self, target: &str) -> Option<PathBuf> {
        if self.writable.is_empty() {
            return None;
        }
        let path = if target.starts_with("file:") {
            url::Url::parse(target).ok()?.to_file_path().ok()?
        } else if target.starts_with('/') {
            PathBuf::from(target)
        } else {
            return None;
        };
        let path = path.canonicalize().unwrap_or(path);
        self.writable
            .iter()
            .any(|dir| path.starts_with(dir))
            .then_some(path)
    }
}

/// Whether a viewer shows `path`'s type, going by its name: an image, audio,
/// video, plain text or a PDF. Nothing a program would run.
#[must_use]
pub fn is_viewer_type(path: &Path) -> bool {
    mime_guess::from_path(path).first().is_some_and(|mime| {
        matches!(
            mime.type_(),
            mime_guess::mime::IMAGE | mime_guess::mime::AUDIO | mime_guess::mime::VIDEO
        ) || matches!(mime.essence_str(), "text/plain" | "application/pdf")
    })
}

impl<A: Apps> tsapi::Service for ApplicationService<A> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

/// One `Application`, as the IDL declares it.
fn application(app: &Application) -> serde_json::Value {
    serde_json::json!({
        "id": app.id,
        "name": app.name,
        "icon": app.icon,
        "path": app.path,
    })
}
