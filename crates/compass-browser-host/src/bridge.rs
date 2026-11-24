//! The production relay: native messages on stdio, Compass IPC on the socket.
//!
//! A native host is not an extension worker, so it may read the session's
//! socket path directly. `$COMPASS_SOCKET` overrides the usual
//! `$XDG_RUNTIME_DIR/compass/ipc.sock`, which is how tests and nested sessions
//! address a non-default engine.

use std::path::PathBuf;
use std::sync::Mutex;

use compass_ipc::{BrowserTab, Client, Request, Response, SocketPath};

use crate::protocol::Tab;

/// A host's connection to the engine, driven by a small current-thread runtime.
pub struct IpcBridge {
    runtime: tokio::runtime::Runtime,
    client: Mutex<Client>,
    browser_id: String,
}

impl IpcBridge {
    /// Connects to the engine Compass is running in this session.
    ///
    /// # Errors
    ///
    /// A sentence when no runtime can be made or the engine is not listening.
    pub fn connect(browser_id: String) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("could not start the host's runtime: {error}"))?;
        let path = std::env::var_os("COMPASS_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|| SocketPath::from_env().into_path_buf());
        let client = runtime
            .block_on(Client::connect(&path))
            .map_err(|error| format!("could not reach Compass at {}: {error}", path.display()))?;
        Ok(Self {
            runtime,
            client: Mutex::new(client),
            browser_id,
        })
    }

    /// Reports the browser's tab list to the engine. An empty list means the
    /// browser disconnected; other extensions then see no tabs and no access.
    ///
    /// # Errors
    ///
    /// A sentence from the engine or the transport.
    pub fn push_tabs(&self, tabs: Vec<Tab>) -> Result<(), String> {
        let payload: Vec<BrowserTab> = tabs
            .into_iter()
            .map(|tab| BrowserTab {
                id: tab.id,
                title: tab.title,
                url: tab.url,
                active: tab.active,
            })
            .collect();
        let request = Request::BrowserTabs {
            browser_id: self.browser_id.clone(),
            tabs: payload,
        };
        let mut client = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match self.runtime.block_on(client.request(request)) {
            Ok(Response::Ack) => Ok(()),
            Ok(Response::Error(error)) => Err(error.message),
            Ok(other) => Err(format!("unexpected answer from Compass: {other:?}")),
            Err(error) => Err(format!("Compass did not answer: {error}")),
        }
    }

    /// Waits for one focus request from the engine, if any.
    #[must_use]
    pub fn focus_wait(&self) -> Option<i64> {
        let request = Request::BrowserFocusWait {
            browser_id: self.browser_id.clone(),
        };
        let mut client = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match self.runtime.block_on(client.request(request)) {
            Ok(Response::BrowserFocus { tab_id }) => tab_id,
            _ => None,
        }
    }
}
