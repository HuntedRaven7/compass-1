//! Desktop integration the settings pages drive: the global shortcuts (and
//! the recorder's capture) and starting Compass at login.

use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

/// The global shortcuts and starting at login.
pub(super) struct Integration {
    /// The global shortcuts' service: rebinding them, and the recorder's
    /// capture.
    pub(super) global_shortcuts: Arc<crate::global_shortcuts::Control>,
    /// Starting Compass at login; `None` in tests that do not give one.
    pub(super) login_start: Option<Arc<dyn compass_platform::LoginStarter>>,
}

impl Integration {
    /// Global shortcuts not yet bound, and login start through `login_start`.
    pub(super) fn new(login_start: Option<Arc<dyn compass_platform::LoginStarter>>) -> Self {
        Self {
            global_shortcuts: Arc::default(),
            login_start,
        }
    }
}

/// The global shortcut and login start requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::ShortcutCapture { capturing } => {
            state
                .read()
                .await
                .global_shortcuts()
                .set_capturing(capturing);
            Response::Ack
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
        Request::LauncherShortcut { request } => {
            let control = state.read().await.global_shortcuts();
            let answer = if request {
                control.request_launcher().await
            } else {
                control.launcher()
            };
            crate::permissions::launcher_response(answer)
        }
        Request::LoginStart { enable } => {
            let Some(starter) = state.read().await.login_start() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    "This engine cannot start Compass at login.",
                ));
            };
            let answer = match enable {
                Some(enabled) => starter.set(enabled).await,
                None => starter.status().await,
            };
            match answer {
                Ok(status) => crate::permissions::login_start_response(status),
                Err(message) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            }
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a integration request",
        )),
    }
}
