//! `org.freedesktop.portal.Background`: starting at login.
//!
//! The one request Compass makes is `RequestBackground` with `autostart` and
//! the command line the desktop should run (ADR-0022 §2). The desktop may
//! show a dialog, so the call is bounded by the interactive deadline. The
//! portal answers with what it granted, and that answer is the only truth
//! there is: no method reads it back later, which is why callers record it.

use std::time::Duration;

use ashpd::desktop::ResponseError;
use ashpd::desktop::background::{BackgroundProxy, BackgroundRequestOptions};

use crate::error::Result;
use crate::shortcuts::bounded;

/// What the desktop said to a background request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BackgroundOutcome {
    /// The request succeeded with these grants.
    Granted {
        /// Whether the desktop starts the application at login.
        autostart: bool,
        /// Whether it may keep running with no window open.
        background: bool,
    },
    /// The person dismissed or declined the dialog (response code 1).
    Denied,
    /// The portal ended the request without saying why (response code 2).
    Refused,
}

/// Client for `org.freedesktop.portal.Background`.
#[derive(Debug, Clone)]
pub struct BackgroundPortal {
    conn: zbus::Connection,
    timeout: Duration,
}

impl BackgroundPortal {
    pub(crate) fn new(conn: zbus::Connection, timeout: Duration) -> Self {
        Self { conn, timeout }
    }

    /// Asks the desktop to start `command` at login (`autostart`) or to stop
    /// doing so, saying `reason` in its dialog.
    ///
    /// # Errors
    ///
    /// A call that failed or timed out. The person declining is not an
    /// error: it is [`BackgroundOutcome::Denied`].
    pub async fn request(
        &self,
        autostart: bool,
        command: &[&str],
        reason: &str,
    ) -> Result<BackgroundOutcome> {
        let proxy = bounded(
            self.timeout,
            "Background",
            BackgroundProxy::with_connection(self.conn.clone()),
        )
        .await?;
        let options = BackgroundRequestOptions::default()
            .set_auto_start(autostart)
            .set_dbus_activatable(false)
            .set_command(command.iter().copied())
            .set_reason(reason);
        let request = bounded(
            self.timeout,
            "RequestBackground",
            proxy.request_background(None, options),
        )
        .await?;
        match request.response() {
            Ok(granted) => Ok(BackgroundOutcome::Granted {
                autostart: granted.auto_start(),
                background: granted.run_in_background(),
            }),
            Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(BackgroundOutcome::Denied),
            Err(ashpd::Error::Response(ResponseError::Other)) => Ok(BackgroundOutcome::Refused),
            Err(source) => Err(crate::PortalError::call("RequestBackground", source)),
        }
    }
}
