//! The browser host's IPC requests: tab reports and focus collection.
//!
//! The host is an ordinary IPC client. It pushes the tab list it sees and
//! polls for focus requests aimed at its browser; the engine hands the same
//! registry to extension workers as a `BrowserExtension/*` service.

use std::sync::Arc;
use std::time::Duration;

use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

/// How long a focus poll waits before answering "nothing yet".
const FOCUS_WAIT: Duration = Duration::from_millis(500);

/// The browser domain's state, one registry shared with extension runs.
#[derive(Debug)]
pub(super) struct Browsers {
    pub(super) registry: Arc<crate::extension_browser::BrowserRegistry>,
}

impl Default for Browsers {
    fn default() -> Self {
        Self {
            registry: Arc::new(crate::extension_browser::BrowserRegistry::default()),
        }
    }
}

impl Browsers {
    /// The registry an extension run needs.
    pub(super) fn registry(&self) -> Arc<crate::extension_browser::BrowserRegistry> {
        Arc::clone(&self.registry)
    }
}

/// Serves one browser request.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::BrowserTabs { browser_id, tabs } => {
            let registry = state.read().await.browser.registry();
            if browser_id.trim().is_empty() {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "a browser host must name itself",
                ));
            }
            registry.update(&browser_id, tabs);
            Response::Ack
        }
        Request::BrowserFocusWait { browser_id } => {
            let registry = state.read().await.browser.registry();
            let tab_id = registry.focus_wait(&browser_id, FOCUS_WAIT).await;
            Response::BrowserFocus { tab_id }
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a browser request",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_empty_browser_id_is_refused() {
        let state = Arc::new(RwLock::new(EngineState::with_index(
            compass_core::AppIndex::builder().build(),
            Box::new(compass_core::JsonFrecencyStore::in_memory(Arc::new(
                compass_core::SystemClock,
            ))),
            compass_ipc::SocketPath::exact("/nonexistent/ipc.sock"),
            10,
        )));
        let response = handle(
            &state,
            Request::BrowserTabs {
                browser_id: String::new(),
                tabs: Vec::new(),
            },
        )
        .await;
        assert!(matches!(response, Response::Error(_)));
    }
}
