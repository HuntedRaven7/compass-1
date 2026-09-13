//! `compass-ui` — The Iced-based launcher UI wired to real search results.
//!
//! This crate provides the main launcher application using Iced 0.14 with
//! the Wayland backend. It connects the UI to the real application index
//! and handles the launcher lifecycle.

#![deny(missing_docs)]

pub mod app;
pub mod message;

pub use app::{AppFlags, LauncherApp, next_selection};

/// Opens the launcher window and runs until it closes.
///
/// Takes over the calling thread: Iced's event loop owns it, and on Wayland it
/// must be the process's main thread. That constraint is the whole reason this
/// is a separate entry point rather than something `vicinae serve` calls — see
/// ADR-0011.
///
/// # Errors
///
/// Returns Iced's error when the window cannot be created, which on a machine
/// with no compositor is the normal outcome rather than a bug.
pub fn run(flags: AppFlags) -> iced::Result {
    let window = flags.window_config.clone();
    iced::application(
        move || LauncherApp::new(flags.clone()),
        LauncherApp::update,
        LauncherApp::view,
    )
    .theme(LauncherApp::theme)
    .subscription(LauncherApp::subscription)
    .window(window)
    .run()
}
pub use message::{Direction, Message};
