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
/// Each client has its own mutex. The focus poll waits on the engine for up to
/// `FOCUS_WAIT`, so a shared lock would hold up every `tabsChanged` behind it;
/// separate locks keep tab reports and focus polling independent. Clients are
/// created on first use and replaced individually after a transport error, so
/// a browser started before Compass — or an engine restart — recovers without
/// restarting the browser.
pub struct IpcBridge {
    runtime: tokio::runtime::Runtime,
    tabs_client: Mutex<Option<Client>>,
    focus_client: Mutex<Option<Client>>,
    path: PathBuf,
    browser_id: String,
    /// Browsers that have reported tabs through this host.
    reported: Mutex<HashSet<String>>,
}

impl IpcBridge {
    /// Builds a bridge without requiring the engine to be up yet.
    ///
    /// # Errors
    ///
    /// A sentence when no async runtime can be made.
    pub fn new(browser_id: String) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("could not start the host's runtime: {error}"))?;
        let path = std::env::var_os("COMPASS_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|| SocketPath::from_env().into_path_buf());
        Ok(Self {
            runtime,
            tabs_client: Mutex::new(None),
            focus_client: Mutex::new(None),
            path,
            browser_id,
            reported: Mutex::new(HashSet::new()),
        })
    }

    fn connect_one(&self) -> Result<Client, String> {
        self.runtime
            .block_on(Client::connect(&self.path))
            .map_err(|error| {
                format!(
                    "could not reach Compass at {}: {error}",
                    self.path.display()
                )
            })
    }

    fn ensure_tabs(&self) -> Result<(), String> {
        let mut client = self
            .tabs_client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if client.is_none() {
            *client = Some(self.connect_one()?);
        }
        Ok(())
    }

    fn ensure_focus(&self) -> Result<(), String> {
        let mut client = self
            .focus_client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if client.is_none() {
            *client = Some(self.connect_one()?);
        }
        Ok(())
    }

    fn reset_tabs(&self) {
        *self
            .tabs_client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    fn reset_focus(&self) {
        *self
            .focus_client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// Reports the browser's tab list to the engine. An empty list means the
    /// browser disconnected; other extensions then see no tabs and no access.
    ///
    /// # Errors
    ///
    /// A sentence from the engine or the transport after one reconnect retry.
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
        for attempt in 0..2 {
            self.ensure_tabs()?;
            let outcome = {
                let mut guard = self
                    .tabs_client
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match guard.as_mut() {
                    Some(client) => self.runtime.block_on(client.request(request.clone())),
                    None => return Err("Compass connection closed".to_owned()),
                }
            };
            match outcome {
                Ok(Response::Ack) => return Ok(()),
                Ok(Response::Error(error)) => return Err(error.message),
                Ok(other) => return Err(format!("unexpected answer from Compass: {other:?}")),
                Err(_) if attempt == 0 => self.reset_tabs(),
                Err(error) => return Err(format!("Compass did not answer: {error}")),
            }
        }
        unreachable!("the loop returns on the second attempt")
    }

    /// Reports a disconnect for every browser that used this host, without
    /// clearing another browser's tabs.
    pub fn disconnect_all(&self) {
        let ids = self.reported_ids();
        for id in ids {
            let _ = self.push_tabs(&id, Vec::new());
        }
    }

    /// The browser ids that have reported tabs, for polling focus requests.
    #[must_use]
    pub fn reported_ids(&self) -> Vec<String> {
        self.reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    /// Waits briefly for one focus request aimed at `browser_id`.
    ///
    /// A transport error is returned rather than reported as "nothing yet":
    /// the caller must back off or stop, or it will spin at full CPU when
    /// Compass has gone away. One reconnect is attempted before giving up.
    ///
    /// # Errors
    ///
    /// A sentence when the engine does not answer or answers unexpectedly.
    pub fn focus_wait(&self, browser_id: &str) -> Result<Option<i64>, String> {
        let request = Request::BrowserFocusWait {
            browser_id: browser_id.to_owned(),
        };
        for attempt in 0..2 {
            self.ensure_focus()?;
            let outcome = {
                let mut guard = self
                    .focus_client
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match guard.as_mut() {
                    Some(client) => self.runtime.block_on(client.request(request.clone())),
                    None => return Err("Compass connection closed".to_owned()),
                }
            };
            match outcome {
                Ok(Response::BrowserFocus { tab_id }) => return Ok(tab_id),
                Ok(Response::Error(error)) => return Err(error.message),
                Ok(other) => return Err(format!("unexpected answer from Compass: {other:?}")),
                Err(_) if attempt == 0 => self.reset_focus(),
                Err(error) => return Err(format!("Compass did not answer: {error}")),
            }
        }
        unreachable!("the loop returns on the second attempt")
    }
}
