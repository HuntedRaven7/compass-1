//! A parser for the freedesktop [desktop entry specification][spec].
//!
//! This is a port of the C++ `xdgpp` desktop entry implementation living in
//! `src/lib/xdgpp`.
//!
//! ```
//! use compass_xdg::DesktopEntry;
//!
//! let entry = DesktopEntry::parse(
//!     "[Desktop Entry]\nName=Firefox\nExec=firefox %u\nType=Application\n",
//! )
//! .unwrap();
//!
//! assert_eq!(entry.name(), "Firefox");
//! assert_eq!(entry.expand_exec_with(&["https://example.com"], false, None), [
//!     "firefox",
//!     "https://example.com"
//! ]);
//! ```
//!
//! [spec]: https://specifications.freedesktop.org/desktop-entry-spec/latest/

pub mod entry;
pub mod exec;
pub mod icon;
pub mod locale;
pub mod reader;
pub mod value;
pub mod xdg_dirs;

pub use entry::{
    ACTION_GROUP_PREFIX, DesktopAction, DesktopEntry, EntryType, Error, MAIN_GROUP, ParseOptions,
};
pub use exec::ExecParser;
pub use icon::{IconTheme, IconThemeDir, IconType, default_theme, find_icon};
pub use locale::Locale;
pub use reader::{Group, Reader};
pub use xdg_dirs::{
    DEFAULT_DATA_DIRS, application_dirs, current_desktops, data_dirs, data_home, exec_search_path,
    icon_dirs,
};
