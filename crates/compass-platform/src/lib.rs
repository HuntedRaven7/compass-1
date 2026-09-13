//! `compass-platform` — Platform-specific operations: launching applications, file indexing, clipboard.
//!
//! This crate handles:
//! - Launching applications via `flatpak-spawn --host` with `OpenURI` fallback
//! - Reading the application catalogue from the filesystem
//! - Clipboard operations (delegates to portals or GNOME Shell extension)

#![deny(missing_docs)]

pub mod launch;

pub use launch::{LaunchError, LaunchMethod, launch_app, launch_app_with_uris};
