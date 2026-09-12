//! The versioned D-Bus contract between Compass and the GNOME Shell helper
//! extension.
//!
//! The canonical definition is the introspection XML checked in beside this
//! crate, so that the extension and the engine can be reviewed against one
//! another:
//!
//! - [`WINDOWS_XML`] — `dbus/org.gnome.Shell.Extensions.Vicinae.Windows.xml`
//! - [`CLIPBOARD_XML`] — `dbus/org.gnome.Shell.Extensions.Vicinae.Clipboard.xml`
//!
//! Everything below must stay in lockstep with those files.

/// Contract version this build of Compass speaks.
///
/// An extension reporting any other value is reported as
/// [`Availability::VersionMismatch`](crate::Availability::VersionMismatch) and
/// its capability is not used.
pub const CONTRACT_VERSION: u32 = 1;

/// Well-known bus name the helper extension lives behind.
///
/// GNOME Shell extensions cannot own their own name; they export objects on
/// the shell's connection.
pub const SHELL_SERVICE: &str = "org.gnome.Shell";

/// Object path of the windows interface.
pub const WINDOWS_PATH: &str = "/org/gnome/Shell/Extensions/Vicinae/Windows";

/// Name of the windows interface.
pub const WINDOWS_INTERFACE: &str = "org.gnome.Shell.Extensions.Vicinae.Windows";

/// Object path of the clipboard interface.
pub const CLIPBOARD_PATH: &str = "/org/gnome/Shell/Extensions/Vicinae/Clipboard";

/// Name of the clipboard interface.
pub const CLIPBOARD_INTERFACE: &str = "org.gnome.Shell.Extensions.Vicinae.Clipboard";

/// Introspection XML for [`WINDOWS_INTERFACE`].
pub const WINDOWS_XML: &str =
    include_str!("../dbus/org.gnome.Shell.Extensions.Vicinae.Windows.xml");

/// Introspection XML for [`CLIPBOARD_INTERFACE`].
pub const CLIPBOARD_XML: &str =
    include_str!("../dbus/org.gnome.Shell.Extensions.Vicinae.Clipboard.xml");

/// Dictionary keys used by `ListWindows`.
pub mod window_key {
    /// `u`, required.
    pub const ID: &str = "id";
    /// `s`, required.
    pub const TITLE: &str = "title";
    /// `s`, required.
    pub const WM_CLASS: &str = "wm_class";
    /// `s`, optional.
    pub const WM_CLASS_INSTANCE: &str = "wm_class_instance";
    /// `u`, optional.
    pub const PID: &str = "pid";
    /// `b`, optional.
    pub const FOCUSED: &str = "focused";
    /// `i`, optional.
    pub const WORKSPACE: &str = "workspace";
    /// `b`, optional.
    pub const CAN_CLOSE: &str = "can_close";
}
