//! Error type for portal calls.
//!
//! Note what is deliberately *not* an error here:
//!
//! - A missing portal is not an error of the crate. [`Portals::connect`] only
//!   fails when there is no session bus at all; a missing interface surfaces as
//!   [`Availability`] and, if the caller ignores that and calls anyway, as
//!   [`PortalError::Unavailable`] — which callers are expected to treat as a
//!   degraded feature, not a failure.
//! - The user denying a permission dialog is not an error either. It comes back
//!   as [`BindOutcome::Denied`](crate::BindOutcome::Denied) or
//!   [`Dismissed`](crate::OpenOutcome::Dismissed), so it can be matched on
//!   rather than string-matched.
//!
//! [`Portals::connect`]: crate::Portals::connect

use std::time::Duration;

use crate::availability::{Availability, PortalInterface};

/// Everything that can go wrong talking to a portal.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PortalError {
    /// The session bus itself could not be reached. The only fatal case.
    #[error("could not connect to the D-Bus session bus")]
    Bus(#[source] zbus::Error),

    /// The interface needed for this call is not usable.
    #[error("{interface} is not usable: {availability}")]
    Unavailable {
        /// Interface name.
        interface: &'static str,
        /// What the probe found.
        availability: Availability,
    },

    /// The interface exists but is older than this call needs.
    #[error("{interface} is v{found}, but `{method}` needs at least v{required}")]
    VersionTooOld {
        /// Interface name.
        interface: &'static str,
        /// Method that needs the newer version.
        method: &'static str,
        /// Version found.
        found: u32,
        /// Version required.
        required: u32,
    },

    /// The portal did not answer within the configured deadline.
    ///
    /// D-Bus method calls have no inherent timeout, and a portal request
    /// completes only when the backend emits `Response` — which a wedged
    /// backend, or a dialog nobody ever answers, never does. Every call in
    /// this crate is therefore bounded.
    #[error("the portal did not answer `{method}` within {timeout:?}")]
    Timeout {
        /// Portal method that hung.
        method: &'static str,
        /// Deadline that elapsed.
        timeout: Duration,
    },

    /// The call reached the portal and failed.
    #[error("portal call `{method}` failed: {source}")]
    Call {
        /// Portal method that failed.
        method: &'static str,
        /// Underlying error. Boxed: `ashpd::Error` is large and this is the
        /// rare path.
        #[source]
        source: Box<ashpd::Error>,
    },

    /// The portal answered, but not in a shape the interface allows.
    #[error("malformed reply from the portal: {0}")]
    Protocol(String),

    /// A shortcut trigger string could not be parsed.
    #[error(transparent)]
    Trigger(#[from] crate::shortcuts::TriggerParseError),
}

impl PortalError {
    pub(crate) fn call(method: &'static str, source: ashpd::Error) -> Self {
        Self::Call {
            method,
            source: Box::new(source),
        }
    }

    pub(crate) fn unavailable(interface: PortalInterface, availability: Availability) -> Self {
        Self::Unavailable {
            interface: interface.name,
            availability,
        }
    }

    /// True when this error means "the feature is not there", as opposed to
    /// "the feature is there and something went wrong". Callers degrade on the
    /// former and may want to surface the latter.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. } | Self::VersionTooOld { .. })
    }
}

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, PortalError>;
