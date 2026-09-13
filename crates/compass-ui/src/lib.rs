//! `compass-ui` — The Iced-based launcher UI wired to real search results.
//!
//! This crate provides the main launcher application using Iced 0.14 with
//! the Wayland backend. It connects the UI to the real application index
//! and handles the launcher lifecycle.

#![deny(missing_docs)]

pub mod app;
pub mod message;

pub use app::{AppFlags, LauncherApp};
pub use message::Message;
