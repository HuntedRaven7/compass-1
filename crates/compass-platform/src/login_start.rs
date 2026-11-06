//! Starting Compass at login — the interface, not an implementation.
//!
//! What is decided about it (the mechanism, the states, the files Compass
//! owns) is `compass_core::login_start`'s. How a platform carries it out is
//! its backend's: on Linux the Background portal, or an XDG autostart entry
//! where there is none (`compass-platform-linux`).
//!
//! Boxed futures for the same reason as [`crate::AppLauncher`]: the engine
//! holds one `Arc<dyn LoginStarter>` chosen at composition.

use std::pin::Pin;

use compass_core::login_start::Status;

/// What a [`LoginStarter`] answers: the status after the call, or a sentence
/// saying why it could not be read or changed.
pub type LoginStartFuture<'a> = Pin<Box<dyn Future<Output = Result<Status, String>> + Send + 'a>>;

/// Starts Compass at login, or stops doing so.
pub trait LoginStarter: Send + Sync + std::fmt::Debug {
    /// Whether Compass starts at login, as far as the platform can tell,
    /// changing nothing.
    fn status(&self) -> LoginStartFuture<'_>;

    /// Turns starting at login on (`enabled`) or off, answering the status
    /// after. Asking for what is already the case asks again and changes
    /// nothing else. On failure nothing Compass recorded changes.
    fn set(&self, enabled: bool) -> LoginStartFuture<'_>;
}
