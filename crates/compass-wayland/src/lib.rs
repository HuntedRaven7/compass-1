//! `compass-wayland` — Wayland integration for the launcher.
//!
//! This crate provides integration points for Wayland protocols that the
//! launcher needs beyond what Iced/winit provides out of the box:
//! - `xdg-activation-v1` for requesting focus with activation tokens
//! - `keyboard-shortcuts-inhibit-v1` for grabbing keys while focused
//!
//! The actual window creation and `xdg_toplevel` management is handled by
//! Iced/winit. This crate only adds the protocol integrations.

#![deny(missing_docs)]

pub mod activation;
pub mod keyboard_inhibit;

pub use activation::{ActivationError, ActivationManager, ActivationToken};
pub use keyboard_inhibit::{InhibitError, KeyboardInhibitManager};
