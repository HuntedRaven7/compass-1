//! The tray: other applications' icons, which Search Tray lists, and
//! Compass's own icon.

use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

/// Both tray services.
#[derive(Default)]
pub(super) struct Tray {
    /// Other applications' tray icons (`SniTrayHost`).
    pub(super) host: Arc<crate::tray_host::TrayHost>,
    /// Compass's own tray icon (`TrayServiceLinux`): `tray.enabled`.
    pub(super) icon: Arc<crate::tray_icon::Control>,
}

/// The tray requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    let tray = Arc::clone(&state.read().await.tray.host);
    tray_request(&tray, request).await
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
