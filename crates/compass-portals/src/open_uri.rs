//! `org.freedesktop.portal.OpenURI`.
//!
//! The minimal surface Phase 1 needs: hand a URL or a local file to whatever
//! the desktop considers its default handler. Inside a Flatpak this is also
//! the *only* unprivileged way to do it, so it is not a convenience wrapper
//! around `xdg-open`.

use std::path::Path;
use std::time::Duration;

use ashpd::desktop::ResponseError;
use ashpd::desktop::open_uri::{
    OpenDirectoryRequest, OpenFileRequest, OpenURIProxy, SchemeSupportedOptions,
};

use crate::availability::{OPEN_URI, SCHEME_SUPPORTED_MIN_VERSION};

use crate::error::{PortalError, Result};
use crate::shortcuts::bounded;

/// What happened to an open request.
///
/// The portal's reply says only whether the interaction succeeded, so a user
/// dismissing a "choose an application" dialog is [`OpenOutcome::Dismissed`]
/// and not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenOutcome {
    /// The desktop accepted the request.
    Opened,
    /// The user cancelled the interaction (portal response code 1).
    Dismissed,
    /// The portal ended the request unsuccessfully without saying why
    /// (response code 2).
    Refused,
}

impl OpenOutcome {
    /// True only for [`OpenOutcome::Opened`].
    pub fn is_opened(self) -> bool {
        matches!(self, Self::Opened)
    }
}

/// Client for `org.freedesktop.portal.OpenURI`.
#[derive(Debug, Clone)]
pub struct OpenUriPortal {
    conn: zbus::Connection,
    timeout: Duration,
    version: u32,
}

impl OpenUriPortal {
    pub(crate) fn new(conn: zbus::Connection, timeout: Duration, version: u32) -> Self {
        Self {
            conn,
            timeout,
            version,
        }
    }

    /// Interface version the portal reported.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Ask whether the desktop has a handler for `scheme`.
    ///
    /// Requires interface v5; on anything older this returns
    /// [`PortalError::VersionTooOld`] without making a call, because the
    /// method simply does not exist there.
    pub async fn scheme_supported(&self, scheme: &str) -> Result<bool> {
        if self.version < SCHEME_SUPPORTED_MIN_VERSION {
            return Err(PortalError::VersionTooOld {
                interface: OPEN_URI.name,
                method: "SchemeSupported",
                found: self.version,
                required: SCHEME_SUPPORTED_MIN_VERSION,
            });
        }
        let proxy = bounded(
            self.timeout,
            "OpenURI",
            OpenURIProxy::with_connection(self.conn.clone()),
        )
        .await?;
        bounded(
            self.timeout,
            "SchemeSupported",
            proxy.scheme_supported(scheme, SchemeSupportedOptions::default()),
        )
        .await
    }

    /// Open a URI with the desktop's default handler.
    ///
    /// `file:` URIs are rejected by the portal itself; use [`Self::open_path`],
    /// which passes a file descriptor and therefore also works from inside a
    /// sandbox where the path is not visible to the handler.
    pub async fn open_uri(&self, uri: &str, ask: bool) -> Result<OpenOutcome> {
        let parsed = ashpd::Uri::parse(uri)
            .map_err(|err| PortalError::Protocol(format!("invalid URI `{uri}`: {err}")))?;
        let request = OpenFileRequest::default()
            .connection(Some(self.conn.clone()))
            .ask(ask);
        let request = bounded(self.timeout, "OpenURI", request.send_uri(&parsed)).await?;
        Ok(classify(request.response()))
    }

    /// Open a local file by handing the portal a descriptor for it.
    pub async fn open_path(&self, path: &Path, writable: bool, ask: bool) -> Result<OpenOutcome> {
        let file = tokio::fs::File::open(path).await.map_err(|err| {
            PortalError::Protocol(format!("cannot open {}: {err}", path.display()))
        })?;
        let request = OpenFileRequest::default()
            .connection(Some(self.conn.clone()))
            .writeable(writable)
            .ask(ask);
        let request = bounded(self.timeout, "OpenFile", request.send_file(&file)).await?;
        Ok(classify(request.response()))
    }

    /// Open a directory in the desktop's file manager.
    pub async fn open_directory(&self, path: &Path) -> Result<OpenOutcome> {
        let dir = tokio::fs::File::open(path).await.map_err(|err| {
            PortalError::Protocol(format!("cannot open {}: {err}", path.display()))
        })?;
        let request = OpenDirectoryRequest::default().connection(Some(self.conn.clone()));
        let request = bounded(self.timeout, "OpenDirectory", request.send(&dir)).await?;
        Ok(classify(request.response()))
    }
}

fn classify(response: std::result::Result<(), ashpd::Error>) -> OpenOutcome {
    match response {
        Ok(()) => OpenOutcome::Opened,
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => OpenOutcome::Dismissed,
        Err(ashpd::Error::Response(ResponseError::Other)) => OpenOutcome::Refused,
        Err(err) => {
            // The request completed but the reply was not one of the three
            // response codes. There is nothing usefully actionable here, and
            // failing to open a URL must not be fatal.
            tracing::warn!(%err, "OpenURI reply was not a recognised response");
            OpenOutcome::Refused
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_codes_map_to_outcomes() {
        assert_eq!(classify(Ok(())), OpenOutcome::Opened);
        assert_eq!(
            classify(Err(ashpd::Error::Response(ResponseError::Cancelled))),
            OpenOutcome::Dismissed
        );
        assert_eq!(
            classify(Err(ashpd::Error::Response(ResponseError::Other))),
            OpenOutcome::Refused
        );
        assert_eq!(
            classify(Err(ashpd::Error::NoResponse)),
            OpenOutcome::Refused
        );
        assert!(OpenOutcome::Opened.is_opened());
        assert!(!OpenOutcome::Dismissed.is_opened());
    }
}
