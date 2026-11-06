//! `compass-platform-linux` — the Linux implementations of `compass-platform`.
//!
//! This is where the code that used to live in `compass-platform` belongs: it
//! is not "what the platform can do", it is what *Linux* does. Splitting them
//! is the near-term obligation in
//! [ADR-0013](../../docs/rust-engine/adr/0013-qt-leaves-the-repository.md),
//! which commits macOS and Windows to their own phases and so makes the shape
//! of the seam something the later work inherits rather than fights.
//!
//! Nothing selects this crate except the `compass` binary. That is the point:
//! a platform backend a shared crate can reach is not a backend.

#![deny(missing_docs)]

pub mod compositor;
pub mod dir_watcher;
pub mod keyboard;
mod launch;
mod login_start;

pub use launch::{LinuxLauncher, host_command, run_command};
pub use login_start::{DIALOG_TIMEOUT as LOGIN_START_DIALOG_TIMEOUT, LinuxLoginStart};
