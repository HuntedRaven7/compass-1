//! The handful of XDG base-directory lookups the core needs.
//!
//! Everything here reads process environment variables. Nothing else in this crate does, so a
//! test that avoids these functions cannot accidentally touch the invoking user's home directory.

use std::path::PathBuf;

/// `/usr/local/share:/usr/share`, the specified fallback for an unset `$XDG_DATA_DIRS`.
pub const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// The application directories, in precedence order: `$XDG_DATA_HOME/applications` first, then
/// each entry of `$XDG_DATA_DIRS` with `/applications` appended.
///
/// Duplicates are removed, keeping the first occurrence, so a `$XDG_DATA_DIRS` that repeats
/// `$XDG_DATA_HOME` does not index everything twice.
#[must_use]
pub fn application_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Some(home) = data_home() {
        dirs.push(home.join("applications"));
    }

    for dir in data_dirs() {
        dirs.push(dir.join("applications"));
    }

    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

/// `$XDG_DATA_HOME`, falling back to `~/.local/share`.
#[must_use]
pub fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => dirs::data_dir(),
    }
}

/// `$XDG_DATA_DIRS`, falling back to [`DEFAULT_DATA_DIRS`].
#[must_use]
pub fn data_dirs() -> Vec<PathBuf> {
    let raw = match std::env::var("XDG_DATA_DIRS") {
        Ok(value) if !value.is_empty() => value,
        _ => DEFAULT_DATA_DIRS.to_owned(),
    };

    raw.split(':')
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The desktop names in `$XDG_CURRENT_DESKTOP`, e.g. `["GNOME"]`.
#[must_use]
pub fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The directories in `$PATH`.
#[must_use]
pub fn exec_search_path() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}
