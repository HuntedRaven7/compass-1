//! `compass-platform` — what the platform can be asked to do.
//!
//! This crate names platform operations; it does not implement them. The
//! implementations live in per-platform crates and are selected at
//! composition, in the `compass` binary. See
//! [ADR-0013](../../docs/rust-engine/adr/0013-qt-leaves-the-repository.md).
//!
//! It previously described itself as handling "launching applications, file
//! indexing, clipboard". It handled launching, and did so by being the Linux
//! implementation — `flatpak-spawn`, the XDG `OpenURI` portal, a direct spawn
//! — while depending on `compass-portals`. The other two were never here.
//!
//! What is here today is [`AppLauncher`]. Clipboard, window management, tray,
//! global shortcuts and file indexing are the remaining services ADR-0013
//! names, and they join this crate as they are ported rather than being
//! declared in advance: a trait written before there is an implementation to
//! shape it is a guess.

#![deny(missing_docs)]

pub mod accessibility;
pub mod launch;
pub mod shortcut_inhibit;
pub mod window_material;

pub use accessibility::{AccessibilityTree, ActionSink, accesskit};
pub use launch::{AppLauncher, LaunchError, LaunchFuture, LaunchMethod, NullLauncher};
pub use shortcut_inhibit::ShortcutInhibitor;
pub use window_material::{
    MaterialOutcome, MaterialRegion, NativeWindow, WindowMaterial, raw_window_handle,
};
