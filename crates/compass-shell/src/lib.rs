//! `compass-shell` — client for the Compass GNOME Shell helper extension.
//!
//! # Why this crate exists
//!
//! On GNOME 50/51, Mutter implements none of `ext-foreign-toplevel-list-v1`,
//! `wlr-foreign-toplevel-management`, `wlr-data-control`/`ext-data-control` or
//! `ext-workspace`, and `org.gnome.Shell.Introspect.GetWindows` is allowlisted
//! to the portal backends so third parties cannot call it. A GNOME Shell
//! helper extension talking D-Bus is therefore the *only* mechanism available
//! for window switching, clipboard history and paste. See `PLAN.md` §3.
//!
//! # The contract
//!
//! The extension's surface is deliberately tiny and explicitly versioned, so
//! that a GNOME release breaks a ~200-line extension rather than the launcher.
//! It is defined by the introspection XML checked in under `dbus/` and
//! mirrored by [`contract`] and [`proxy`]:
//!
//! - `org.gnome.Shell.Extensions.Vicinae.Windows`: `Version`, `ListWindows`,
//!   `ActivateWindow`, `CloseWindow`, `WindowsChanged`.
//! - `org.gnome.Shell.Extensions.Vicinae.Clipboard`: `Version`,
//!   `GetClipboard`, `SetClipboard`, `ClipboardChanged`.
//!
//! # Degradation is the normal case
//!
//! [`ShellClient::connect`] succeeds against a session bus with no extension
//! at all. Capabilities are modelled explicitly as [`Availability`], which
//! distinguishes "absent" from "present but speaking a version we do not",
//! and [`ShellCapabilities::degraded`] enumerates exactly which product
//! features are lost for `vicinae doctor` to report. Nothing in the critical
//! path — app search, launch, calculator, emoji, snippets, file search — may
//! depend on this crate.
//!
//! # Reconnection
//!
//! Reloading a GNOME Shell extension restarts `gnome-shell`. The client
//! watches `NameOwnerChanged` for `org.gnome.Shell`: on loss it publishes
//! [`ShellCapabilities::absent`], and on reacquisition it re-probes with
//! [`Backoff`] because the extension's objects appear slightly after the bus
//! name does.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> Result<(), compass_shell::ShellError> {
//! use compass_shell::{ShellClient, ShellConfig};
//!
//! let client = ShellClient::connect(ShellConfig::default()).await?;
//!
//! for feature in client.capabilities().degraded() {
//!     eprintln!("degraded: {} — {}", feature.title(), feature.explanation());
//! }
//!
//! if client.capabilities().windows.is_available() {
//!     for window in client.list_windows().await? {
//!         println!("{} — {}", window.wm_class, window.title);
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

pub mod capability;
pub mod client;
pub mod contract;
pub mod error;
pub mod model;
pub mod proxy;

pub use capability::{Availability, DegradedFeature, ShellCapabilities};
pub use client::{
    Backoff, CapabilityStream, ClipboardStream, ShellClient, ShellConfig, WindowsChangedStream,
};
pub use contract::{
    CLIPBOARD_INTERFACE, CLIPBOARD_PATH, CONTRACT_VERSION, SHELL_SERVICE, WINDOWS_INTERFACE,
    WINDOWS_PATH,
};
pub use error::{Result, ShellError};
pub use model::{ClipboardChange, ClipboardContent, Window, WindowId};
