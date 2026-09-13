//! `keyboard-shortcuts-inhibit-v1` support for grabbing keys while focused.
//!
//! This is a stub implementation. The actual Wayland protocol integration
//! will be completed when wiring up the Iced application.

use tracing::debug;

/// Manager for keyboard-shortcuts-inhibit-v1.
pub struct KeyboardInhibitManager {
    // Stub - will hold the actual inhibitor when implemented
}

impl KeyboardInhibitManager {
    /// Create a new keyboard inhibit manager.
    ///
    /// Returns `None` if the compositor doesn't support keyboard-shortcuts-inhibit-v1.
    pub fn new(
        _connection: &wayland_client::Connection,
        _queue_handle: &wayland_client::QueueHandle<Self>,
    ) -> Option<Self> {
        // TODO: Bind keyboard_shortcuts_inhibit_v1 global
        None
    }

    /// Inhibit keyboard shortcuts (grab all keys).
    ///
    /// Call this when the launcher window gains focus.
    pub async fn inhibit(&mut self) -> Result<(), InhibitError> {
        debug!("Keyboard shortcuts inhibit requested (stub)");
        Err(InhibitError::Unsupported)
    }

    /// Stop inhibiting keyboard shortcuts.
    ///
    /// Call this when the launcher window loses focus.
    pub async fn uninhibit(&mut self) -> Result<(), InhibitError> {
        debug!("Keyboard shortcuts uninhibit requested (stub)");
        Err(InhibitError::Unsupported)
    }
}

/// Errors during keyboard shortcuts inhibition.
#[derive(Debug, thiserror::Error)]
pub enum InhibitError {
    /// Compositor doesn't support keyboard-shortcuts-inhibit-v1.
    #[error("compositor doesn't support keyboard-shortcuts-inhibit-v1")]
    Unsupported,
    /// Already inhibited.
    #[error("already inhibited")]
    AlreadyInhibited,
    /// Not currently inhibited.
    #[error("not currently inhibited")]
    NotInhibited,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inhibit_error_display() {
        let err = InhibitError::Unsupported;
        assert!(err.to_string().contains("doesn't support"));

        let err = InhibitError::AlreadyInhibited;
        assert!(err.to_string().contains("already inhibited"));
    }
}
