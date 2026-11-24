//! The production relay: native messages on stdio, Compass IPC on the socket.
//!
//! A native host is not an extension worker, so it may read the session's
//! socket path directly. `$COMPASS_SOCKET` overrides the usual
//! `$XDG_RUNTIME_DIR/compass/ipc.sock`, which is how tests and nested sessions
//! address a non-default engine.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use compass_ipc::{BrowserTab, Client, Request, Response, SocketPath};

use crate::protocol::Tab;

/// A host's connections to the engine, driven by a small current-thread runtime.
///
/// Tab reports and the long focus poll use separate connections. The poll holds
/// its connection for up to `FOCUS_WAIT`, and a `std::sync::Mutex` does not hand
/// over fairly, so sharing one would stall every `tabsChanged` behind it.
pub struct IpcBridge {
    runtime: tokio::runtime::Runtime,
    tabs_client: Mutex<Client>,
    focus_client: Mutex<Client>,
    browser_id: String,
    /// Browsers that have reported tabs through this host, so a disconnect can
    /// clear exactly those and not another browser connected through it.
    reported: Mutex<HashSet<String>>,
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
        let tabs_client = runtime
            .block_on(Client::connect(&path))
            .map_err(|error| format!("could not reach Compass at {}: {error}", path.display()))?;
        let focus_client = runtime
            .block_on(Client::connect(&path))
            .map_err(|error| format!("could not reach Compass at {}: {error}", path.display()))?;
        Ok(Self {
            runtime,
            tabs_client: Mutex::new(tabs_client),
            focus_client: Mutex::new(focus_client),
            browser_id,
            reported: Mutex::new(HashSet::new()),
        })
    }

    /// Reports the browser's tab list to the engine. An empty list means the
    /// browser disconnected; other extensions then see no tabs and no access.
    ///
    /// # Errors
    ///
    /// A sentence from the engine or the transport.
    pub fn push_tabs(&self, browser_id: &str, tabs: Vec<Tab>) -> Result<(), String> {
        let empty = tabs.is_empty();
        let payload: Vec<BrowserTab> = tabs
            .into_iter()
            .map(|tab| BrowserTab {
                id: tab.id,
                title: tab.title,
                url: tab.url,
                active: tab.active,
            })
            .collect();
        let id = if browser_id.trim().is_empty() {
            self.browser_id.clone()
        } else {
            browser_id.to_owned()
        };
        if !empty {
            self.reported
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(id.clone());
        }
        let request = Request::BrowserTabs {
            browser_id: id,
            tabs: payload,
        };
        let mut client = self
            .tabs_client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match self.runtime.block_on(client.request(request)) {
            Ok(Response::Ack) => Ok(()),
            Ok(Response::Error(error)) => Err(error.message),
            Ok(other) => Err(format!("unexpected answer from Compass: {other:?}")),
            Err(error) => Err(format!("Compass did not answer: {error}")),
        }
    }

    /// Reports a disconnect for every browser that used this host, without
    /// clearing another browser's tabs.
    pub fn disconnect_all(&self) {
        let ids: Vec<String> = self
            .reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect();
        for id in ids {
            let _ = self.push_tabs(&id, Vec::new());
        }
    }

    /// Waits for one focus request from the engine.
    ///
    /// A transport error is returned rather than reported as "nothing yet":
    /// the caller must stop polling or it will spin at full CPU when Compass
    /// has gone away.
    ///
    /// # Errors
    ///
    /// A sentence when the engine does not answer or answers unexpectedly.
    pub fn focus_wait(&self) -> Result<Option<i64>, String> {
        let request = Request::BrowserFocusWait {
            browser_id: self.browser_id.clone(),
        };
        let mut client = self
            .focus_client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match self.runtime.block_on(client.request(request)) {
            Ok(Response::BrowserFocus { tab_id }) => Ok(tab_id),
            Ok(Response::Error(error)) => Err(error.message),
            Ok(other) => Err(format!("unexpected answer from Compass: {other:?}")),
            Err(error) => Err(format!("Compass did not answer: {error}")),
        }
    }
}
