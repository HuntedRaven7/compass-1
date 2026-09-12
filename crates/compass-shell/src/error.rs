//! Error type for the shell client.

use crate::capability::Availability;

/// Everything that can go wrong talking to the shell helper extension.
///
/// Note what is *not* here: "extension missing" is not an error condition of
/// the crate as a whole. [`ShellClient::connect`](crate::ShellClient::connect)
/// succeeds against a bare session bus with no extension at all; only the
/// window and clipboard calls themselves fail, with
/// [`ShellError::Unavailable`], and callers are expected to treat that as a
/// degraded feature rather than a failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ShellError {
    /// The session bus itself could not be reached. This is the only fatal
    /// case, and it means there is no D-Bus at all.
    #[error("could not connect to the D-Bus session bus")]
    Bus(#[source] zbus::Error),

    /// The capability needed for this call is not usable.
    #[error("shell extension capability unusable: {0}")]
    Unavailable(Availability),

    /// A method call reached the bus but failed.
    #[error("D-Bus call to the shell extension failed")]
    Call(#[source] zbus::Error),

    /// The extension took longer than
    /// [`ShellConfig::call_timeout`](crate::ShellConfig::call_timeout) to
    /// answer. A wedged `gnome-shell` must not wedge the launcher, so calls
    /// are always bounded.
    #[error("the shell extension did not answer `{method}` within {timeout:?}")]
    Timeout {
        /// D-Bus method that hung.
        method: &'static str,
        /// Deadline that elapsed.
        timeout: std::time::Duration,
    },

    /// The extension answered, but not in a shape the contract allows.
    #[error("malformed reply from the shell extension: {0}")]
    Protocol(String),
}

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, ShellError>;
