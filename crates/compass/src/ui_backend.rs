//! Socket adapter for the UI's shared application search/history service.
//!
//! [`DaemonBackend`] answers the window's
//! [`ApplicationBackend`](compass_ui::backend::ApplicationBackend),
//! [`ClipboardBackend`](compass_ui::backend::ClipboardBackend) and
//! [`WindowBackend`](compass_ui::backend::WindowBackend) by asking the engine
//! over its socket. This module holds what every answer shares: the deadlines,
//! which differ by how long the engine may hold a request, and the one
//! exchange that turns an engine refusal into the sentence the window shows.
//!
//! The answers themselves live in submodules by domain, each with the
//! conversions from the wire that only it needs: `application` for the
//! launcher's own views, with its larger domains in `extensions`, `scripts`,
//! `settings` and `snippets`; `clipboard` for clipboard history; `windows`
//! for the window switcher.

mod application;
mod clipboard;
mod extensions;
mod scripts;
mod settings;
mod snippets;
mod windows;

use std::time::Duration;

use compass_ipc::{Request, SocketPath};

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

/// The engine's message with its first letter capitalised, for display.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
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
