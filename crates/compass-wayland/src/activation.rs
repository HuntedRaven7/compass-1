//! `xdg-activation-v1` support for requesting window focus with activation tokens.
//!
//! This is a stub implementation. The actual Wayland protocol integration
//! will be completed when wiring up the Iced application.

use tracing::debug;
use wayland_client::protocol::wl_surface::WlSurface;

/// An activation token received from the compositor.
#[derive(Debug, Clone)]
pub struct ActivationToken {
    /// The token string.
    pub token: String,
}

/// Manager for xdg-activation-v1 requests.
pub struct ActivationManager {
    // Stub - will hold the actual xdg_activation proxy when implemented
}

impl ActivationManager {
    /// Create a new activation manager.
    ///
    /// Returns `None` if the compositor doesn't support xdg-activation-v1.
    pub fn new(
        _connection: &wayland_client::Connection,
        _queue_handle: &wayland_client::QueueHandle<Self>,
    ) -> Option<Self> {
        // TODO: Bind xdg_activation_v1 global
        None
    }

    /// Request activation for a surface.
    ///
    /// Returns the activation token that can be used with the GlobalShortcuts portal
    /// to prove the activation was user-initiated.
    pub async fn request_activation(
        &self,
        _surface: &WlSurface,
        _app_id: Option<&str>,
    ) -> Result<ActivationToken, ActivationError> {
        // TODO: Implement actual xdg-activation-v1 request
        debug!("Activation requested (stub)");
        Err(ActivationError::Unsupported)
    }
}

/// Errors during activation.
#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    /// Activation request timed out.
    #[error("activation request timed out")]
    Timeout,
    /// Activation request was cancelled.
    #[error("activation request cancelled")]
    Cancelled,
    /// Compositor doesn't support xdg-activation-v1.
    #[error("compositor doesn't support xdg-activation-v1")]
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_error_display() {
        let err = ActivationError::Timeout;
        assert!(err.to_string().contains("timed out"));

        let err = ActivationError::Unsupported;
        assert!(err.to_string().contains("doesn't support"));
    }
}
