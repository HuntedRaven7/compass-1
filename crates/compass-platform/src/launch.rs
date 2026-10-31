//! Launching an application — the interface, not an implementation.
//!
//! # Why this is a trait now
//!
//! This module used to *be* the Linux launcher: `flatpak-spawn --host`, then
//! the XDG `OpenURI` portal, then a direct spawn, in a crate named
//! `compass-platform` that depended on `compass-portals`. That is an
//! implementation wearing the name of an abstraction, and
//! [ADR-0013](../../../docs/rust-engine/adr/0013-qt-leaves-the-repository.md)
//! makes it a problem rather than a curiosity: Qt leaves the repository, so
//! macOS and Windows get their own phases, and every phase that lands before
//! the seam exists is written against the shape this crate has today.
//!
//! The Linux implementation now lives in `compass-platform-linux` and is
//! selected in the `compass` binary. This crate names what a launcher *is*.
//!
//! # Why the future is boxed
//!
//! `async fn` in traits is stable, but it is not `dyn`-compatible, and the
//! whole point here is to hold an `Arc<dyn AppLauncher>` chosen at
//! composition. Boxing the future is the cost of that, and it is paid once per
//! launch — not once per keystroke.

use std::future::Future;
use std::pin::Pin;

use compass_xdg::DesktopEntry;

/// How the application was launched.
///
/// The variants are Linux-shaped today because the only implementation is.
/// A macOS backend would report its own, and this enum grows then rather than
/// being guessed at now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMethod {
    /// Launched on the host from inside a Flatpak, through the session
    /// helper's `HostCommand` (the call `flatpak-spawn --host` makes).
    FlatpakHost,
    /// Launched via the OpenURI portal.
    OpenUri,
    /// Launched directly (not sandboxed).
    Direct,
}

/// Errors that can occur during launch.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// A desktop link has no usable absolute URI.
    #[error("desktop link has no valid absolute URL")]
    InvalidLinkUrl,
    /// The requested action is not declared by the application.
    #[error("unknown desktop action: {0}")]
    UnknownAction(String),
    /// The platform backend cannot launch desktop actions.
    #[error("desktop actions are not supported by this launcher")]
    ActionsUnsupported,
    /// No Exec key in desktop entry.
    #[error("no Exec key in desktop entry")]
    NoExec,
    /// The Flatpak session helper did not start the process on the host.
    #[error("could not start it on the host: {0}")]
    FlatpakHostFailed(String),
    /// OpenURI portal unavailable.
    #[error("OpenURI portal unavailable: {0}")]
    OpenUriUnavailable(String),
    /// OpenURI portal failed.
    #[error("OpenURI portal failed: {0}")]
    OpenUriFailed(String),
    /// OpenURI dismissed by user.
    #[error("OpenURI dismissed by user")]
    OpenUriDismissed,
    /// OpenURI refused.
    #[error("OpenURI refused: {0}")]
    OpenUriRefused(String),
    /// Direct execution failed.
    #[error("direct execution failed: {0}")]
    DirectFailed(String),
}

/// The future an [`AppLauncher`] returns.
pub type LaunchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<LaunchMethod, LaunchError>> + Send + 'a>>;

/// Launching an application on this platform.
///
/// One implementation exists (`compass-platform-linux`), which means this is
/// not yet proven to be an abstraction — a trait with a single implementation
/// describes that implementation until a second one disagrees with it. The
/// same is true of `compass-shell`'s D-Bus contract, and the answer there was
/// a mock: see [`NullLauncher`], which exists so that a caller can be tested
/// without launching anything, and so that the trait has to survive being
/// implemented twice.
pub trait AppLauncher: std::fmt::Debug + Send + Sync {
    /// Launch `entry`, passing `uris` to its `Exec` field codes.
    fn launch<'a>(&'a self, entry: &'a DesktopEntry, uris: &'a [&'a str]) -> LaunchFuture<'a>;

    /// Launch a declared desktop action, never falling back to the parent application.
    fn launch_action<'a>(
        &'a self,
        _entry: &'a DesktopEntry,
        _action_id: &'a str,
        _uris: &'a [&'a str],
    ) -> LaunchFuture<'a> {
        Box::pin(async { Err(LaunchError::ActionsUnsupported) })
    }
}

/// A launcher that launches nothing and says so.
///
/// Not a stub to be filled in: it is the second implementation that keeps
/// [`AppLauncher`] honest, and it is what a test uses when the thing under
/// test is "did the UI ask to launch the right entry" rather than "did the
/// application start".
#[derive(Debug, Clone, Copy, Default)]
pub struct NullLauncher;

impl AppLauncher for NullLauncher {
    fn launch<'a>(&'a self, entry: &'a DesktopEntry, _uris: &'a [&'a str]) -> LaunchFuture<'a> {
        Box::pin(async move {
            tracing::info!(name = %entry.name(), "NullLauncher: not launching");
            Err(LaunchError::DirectFailed(
                "NullLauncher never launches anything".to_owned(),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_null_launcher_is_an_app_launcher() {
        // Compiles only if NullLauncher satisfies the object-safe trait, which
        // is the property the composition in `compass` depends on.
        let launcher: std::sync::Arc<dyn AppLauncher> = std::sync::Arc::new(NullLauncher);
        assert_eq!(format!("{launcher:?}"), "NullLauncher");
    }

    #[test]
    fn an_unsupported_action_is_not_a_parent_launch() {
        let entry =
            DesktopEntry::parse("[Desktop Entry]\nType=Application\nName=Browser\nExec=parent\n")
                .unwrap();
        let launcher = NullLauncher;
        let mut future = launcher.launch_action(&entry, "private", &[]);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(matches!(
            future.as_mut().poll(&mut context),
            std::task::Poll::Ready(Err(LaunchError::ActionsUnsupported))
        ));
    }
}
