//! Helpers shared by the integration tests.
//!
//! Everything here writes into a caller-supplied temporary directory. No test in this crate reads
//! `$HOME`, `$XDG_*` or the real `PATH`.

#![allow(dead_code)]

use std::path::Path;

use compass_core::{AppIndex, AppIndexBuilder};

/// Writes `contents` to `dir/name`, creating parent directories.
pub fn write(dir: &Path, name: &str, contents: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture directory");
    }
    std::fs::write(&path, contents).expect("write fixture");
}

/// A builder pinned to a fixed locale and desktop so results do not depend on the machine running
/// the tests.
pub fn builder() -> AppIndexBuilder {
    AppIndex::builder()
        .desktops(["GNOME"])
        .locale(compass_xdg::Locale::parse("C"))
}

/// A minimal `Type=Application` entry.
pub fn app(name: &str, exec: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n")
}
