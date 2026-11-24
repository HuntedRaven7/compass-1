//! The narrow message set the prototype proves.
//!
//! This is deliberately not the extension API's `BrowserExtension/getTabs`:
//! the native host speaks to the WebExtension, and the WebExtension speaks to
//! the launcher. Keeping the two protocols distinct is what lets the browser
//! vendor churn stay out of launcher core (ADR-0008).

use serde::{Deserialize, Serialize};

/// The version this host implements. The extension and the host must agree;
/// a mismatch is answered with [`HostResponse::Error`] rather than guessed.
pub const PROTOCOL_VERSION: u32 = 1;

/// A message from the WebExtension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HostRequest {
    /// Handshake: prove the peer is talking to a host of this protocol.
    Connect {
        /// The version the extension expects.
        version: u32,
    },
    /// List the tabs of the connected browsers.
    GetTabs,
    /// Focus one tab in one browser.
    FocusTab {
        /// Stable identity of the browser, so two browsers cannot collide.
        browser_id: String,
        /// The browser's own tab id.
        tab_id: i64,
    },
    /// The WebExtension reports its current tabs. The host relays them to the
    /// engine, which is where other extensions read them from. (v2.)
    TabsChanged {
        /// Stable identity of the browser.
        browser_id: String,
        /// Its open tabs; an empty list means the browser disconnected.
        tabs: Vec<Tab>,
    },
}

/// A message back to the WebExtension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HostResponse {
    /// The handshake succeeded.
    Connected {
        /// The version both sides will use.
        version: u32,
        /// The id this host gives its browser.
        browser_id: String,
    },
    /// The tabs, in a deterministic order.
    Tabs {
        /// The open tabs.
        tabs: Vec<Tab>,
    },
    /// The action was performed.
    Ok,
    /// The engine asked the browser to focus one tab. (v2.)
    Focus {
        /// The browser's own tab id.
        tab_id: i64,
    },
    /// The request was refused, with a reason suitable for the extension's log.
    Error {
        /// Why it was refused.
        message: String,
    },
}

/// One tab, as the WebExtension sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    /// The browser's own tab id.
    pub id: i64,
    /// The page title, when the browser reports one.
    pub title: Option<String>,
    /// The page URL.
    pub url: String,
    /// Whether this is the active tab in its window.
    pub active: bool,
}

/// Where the host gets its tabs. The prototype's binary supplies an empty
/// source; #316 replaces it with a client of the engine's IPC socket.
pub trait TabSource {
    /// The tabs, in a stable order.
    fn tabs(&self) -> Vec<Tab>;
    /// Focus `tab_id`, returning whether the browser accepted it.
    fn focus_tab(&self, browser_id: &str, tab_id: i64) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_names_are_camel_case() {
        assert_eq!(
            serde_json::to_string(&HostRequest::GetTabs).unwrap(),
            r#"{"type":"getTabs"}"#
        );
        assert_eq!(
            serde_json::to_string(&HostRequest::FocusTab {
                browser_id: "firefox".into(),
                tab_id: 7,
            })
            .unwrap(),
            r#"{"type":"focusTab","browserId":"firefox","tabId":7}"#
        );
    }

    #[test]
    fn an_unknown_request_is_a_parse_error_not_a_panic() {
        assert!(serde_json::from_str::<HostRequest>(r#"{"type":"openUrl"}"#).is_err());
    }
}
