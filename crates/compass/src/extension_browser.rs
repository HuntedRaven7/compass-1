//! The browser bridge an extension reaches through `BrowserExtension/*`.
//!
//! ADR-0008 took browser control out of launcher core; ADR-0022 chose a
//! launcher-shipped native host that relays to this engine over IPC. The
//! native host pushes its tab list into [`BrowserRegistry`] and picks focus
//! requests back up; extension workers see one through [`EngineBrowser`].
//! [`NoBrowsers`] stays for tests and for installs without a host.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use compass_ipc::BrowserTab;
use compass_worker_host::browser_service::{Browser, Tab};

/// The live tab lists reported by connected browser hosts, and focus requests
/// waiting to be collected by them.
#[derive(Debug, Default)]
pub struct BrowserRegistry {
    browsers: Mutex<HashMap<String, Vec<BrowserTab>>>,
    focus: Mutex<VecDeque<(String, i64)>>,
    notify: tokio::sync::Notify,
}

impl BrowserRegistry {
    /// Replaces one browser's tab list. An empty list means it disconnected.
    pub fn update(&self, browser_id: &str, tabs: Vec<BrowserTab>) {
        let mut browsers = self
            .browsers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if tabs.is_empty() {
            browsers.remove(browser_id);
        } else {
            browsers.insert(browser_id.to_owned(), tabs);
        }
    }

    /// Whether at least one browser has reported tabs.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.browsers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|tabs| !tabs.is_empty())
    }

    /// The tabs, as the extension API sees them.
    #[must_use]
    pub fn tabs(&self) -> Vec<Tab> {
        let browsers = self
            .browsers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        browsers
            .iter()
            .flat_map(|(browser_id, tabs)| {
                tabs.iter().map(move |tab| Tab {
                    id: tab.id,
                    title: tab.title.clone(),
                    url: tab.url.clone(),
                    active: tab.active,
                    browser_id: browser_id.clone(),
                })
            })
            .collect()
    }

    /// Queues a focus request for the host that owns `browser_id`.
    pub fn request_focus(&self, browser_id: &str, tab_id: i64) {
        self.focus
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back((browser_id.to_owned(), tab_id));
        self.notify.notify_waiters();
    }

    /// Waits briefly for a focus request aimed at `browser_id`, then takes the
    /// oldest one. A host polls this between native messages because Compass's
    /// IPC has no server-push channel.
    pub async fn focus_wait(&self, browser_id: &str, budget: Duration) -> Option<i64> {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            if let Some(tab_id) = self.take_focus(browser_id) {
                return Some(tab_id);
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return None;
            }
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep_until(deadline) => {}
            }
        }
    }

    /// Takes the oldest focus request for this browser, if one is waiting.
    #[must_use]
    pub fn take_focus(&self, browser_id: &str) -> Option<i64> {
        let mut focus = self
            .focus
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        focus
            .iter()
            .position(|(id, _)| id == browser_id)
            .and_then(|index| focus.remove(index))
            .map(|(_, tab_id)| tab_id)
    }
}

/// A `Browser` backed by the live registry, installed in extension sessions.
#[derive(Debug, Clone)]
pub struct EngineBrowser {
    registry: Arc<BrowserRegistry>,
}

impl EngineBrowser {
    /// Serves extensions from `registry`.
    #[must_use]
    pub const fn new(registry: Arc<BrowserRegistry>) -> Self {
        Self { registry }
    }
}

impl Browser for EngineBrowser {
    fn tabs(&self) -> Vec<Tab> {
        self.registry.tabs()
    }

    fn focus_tab(&self, browser_id: &str, tab_id: i64) {
        self.registry.request_focus(browser_id, tab_id);
    }
}

/// [`Browser`] with no browser connected, for tests and installs without a host.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBrowsers;

impl Browser for NoBrowsers {
    fn tabs(&self) -> Vec<Tab> {
        Vec::new()
    }

    fn focus_tab(&self, browser_id: &str, tab_id: i64) {
        tracing::debug!(browser_id, tab_id, "focusTab with no browser connected");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: i64) -> BrowserTab {
        BrowserTab {
            id,
            title: None,
            url: "https://example.com".into(),
            active: false,
        }
    }

    #[test]
    fn a_browser_that_reports_tabs_is_connected() {
        let registry = BrowserRegistry::default();
        assert!(!registry.is_connected());
        registry.update("firefox", vec![tab(1)]);
        assert!(registry.is_connected());
        assert_eq!(registry.tabs()[0].browser_id, "firefox");
        registry.update("firefox", Vec::new());
        assert!(!registry.is_connected());
    }

    #[test]
    fn focus_requests_are_routed_to_their_browser() {
        let registry = BrowserRegistry::default();
        registry.update("firefox", vec![tab(1)]);
        registry.update("chrome", vec![tab(2)]);
        registry.request_focus("firefox", 1);
        registry.request_focus("chrome", 2);
        assert_eq!(registry.take_focus("chrome"), Some(2));
        assert_eq!(registry.take_focus("firefox"), Some(1));
        assert_eq!(registry.take_focus("firefox"), None);
    }
}
