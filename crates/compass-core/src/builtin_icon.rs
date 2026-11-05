//! The built-in icon set: the names `Icon.*` resolves to.
//!
//! Ports `BuiltinIconService`
//! (`src/server/src/services/builtin-icon/`). An extension writes
//! `Icon.AlarmRinging` and the host has to turn that into a picture; the name
//! in between is `"alarm-ringing"`, and the file is
//! `extra/builtin-icons/alarm-ringing.svg`.
//!
//! As with [`crate::glyph`], the table is not written down twice: `build.rs`
//! parses the 852 names out of the `Icon` enum in `@vicinae/api`
//! (`src/typescript/api/src/api/icon.ts`), so the host and the API extensions
//! import cannot disagree about which icons exist or what they are called.
//!
//! # Where the C++ looks, and why this cannot
//!
//! `pathForName` returns `":icons/<name>"`, a **Qt resource** path — the SVGs
//! are compiled into the binary. There is no such thing here, so
//! [`file_name`] gives the file's name and leaves the directory to whoever
//! ships the assets. Inventing a path would be inventing a packaging decision.

include!(concat!(env!("OUT_DIR"), "/builtin_icon_table.rs"));

/// The icon shown when a name is not one of ours.
///
/// `BuiltinIconService::unknownIcon()` is `QuestionMarkCircle`, and the
/// TypeScript API falls back to the same one when an extension's `Image` has
/// no source at all.
pub const UNKNOWN: &str = "question-mark-circle";

/// Every icon name, in the C++ enum's order.
#[must_use]
pub fn names() -> &'static [&'static str] {
    ICON_NAMES
}

/// Whether `name` is one of the built-in icons.
#[must_use]
pub fn is_builtin(name: &str) -> bool {
    ICON_NAMES.contains(&name)
}

/// The file `name` is drawn from, without a directory.
///
/// `None` for a name that is not built in, rather than a path that does not
/// exist: a caller that wants the fallback asks for [`UNKNOWN`] on purpose.
#[must_use]
pub fn file_name(name: &str) -> Option<String> {
    is_builtin(name).then(|| format!("{name}.svg"))
}

/// Where the icon files are: `$COMPASS_BUILTIN_ICONS`, else the first
/// `compass/builtin-icons` under `$XDG_DATA_HOME` or `$XDG_DATA_DIRS` (the
/// Flatpak installs them under `/app/share`).
#[must_use]
pub fn directory() -> Option<std::path::PathBuf> {
    directory_in(
        std::env::var_os("COMPASS_BUILTIN_ICONS").map(Into::into),
        crate::xdg_dirs::data_home(),
        std::env::var("XDG_DATA_DIRS").ok().as_deref(),
    )
}

/// [`directory`], from its inputs.
#[must_use]
pub fn directory_in(
    override_dir: Option<std::path::PathBuf>,
    data_home: Option<std::path::PathBuf>,
    data_dirs: Option<&str>,
) -> Option<std::path::PathBuf> {
    if override_dir.is_some() {
        return override_dir;
    }
    let data_dirs = data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or("/usr/local/share:/usr/share");
    data_home
        .into_iter()
        .chain(data_dirs.split(':').map(std::path::PathBuf::from))
        .map(|dir| dir.join("compass/builtin-icons"))
        .find(|dir| dir.is_dir())
}

/// The file `name` is drawn from, when it is built in and installed.
#[must_use]
pub fn path(name: &str) -> Option<std::path::PathBuf> {
    let path = directory()?.join(file_name(name)?);
    path.is_file().then_some(path)
}

/// Every `(builtin, symbolic)` pair [`symbolic_name`] knows, sorted by builtin.
#[must_use]
pub fn symbolic_names() -> &'static [(&'static str, &'static str)] {
    SYMBOLIC
}

/// The GNOME (Adwaita) symbolic icon that stands in for builtin `name`.
///
/// The builtin set is Vicinae's own artwork. On a GNOME desktop the launcher
/// should look like the rest of it, so the host draws the theme's
/// `<name>-symbolic` icon instead when the theme has one, and keeps the
/// builtin SVG for every name without a counterpart here (brand logos, mostly).
#[must_use]
pub fn symbolic_name(name: &str) -> Option<&'static str> {
    SYMBOLIC
        .binary_search_by(|(builtin, _)| builtin.cmp(&name))
        .ok()
        .map(|index| SYMBOLIC[index].1)
}

/// Builtin name to freedesktop symbolic name, sorted by builtin name.
const SYMBOLIC: &[(&str, &str)] = &[
    ("add-person", "contact-new-symbolic"),
    ("alarm", "alarm-symbolic"),
    ("alarm-ringing", "alarm-symbolic"),
    ("app-window", "focus-windows-symbolic"),
    ("app-window-grid-2x2", "view-app-grid-symbolic"),
    ("app-window-grid-3x3", "view-app-grid-symbolic"),
    ("app-window-list", "view-list-symbolic"),
    ("arrow-clockwise", "object-rotate-right-symbolic"),
    ("arrow-counter-clockwise", "object-rotate-left-symbolic"),
    ("arrow-down", "go-down-symbolic"),
    ("arrow-left", "go-previous-symbolic"),
    ("arrow-right", "go-next-symbolic"),
    ("arrow-up", "go-up-symbolic"),
    ("battery", "battery-good-symbolic"),
    ("battery-charging", "battery-good-charging-symbolic"),
    ("bell", "preferences-system-notifications-symbolic"),
    ("blank-document", "x-office-document-symbolic"),
    ("bluetooth", "bluetooth-active-symbolic"),
    ("bolt", "insert-link-symbolic"),
    ("bookmark", "user-bookmarks-symbolic"),
    ("box", "package-x-generic-symbolic"),
    ("brush", "preferences-desktop-appearance-symbolic"),
    ("bug", "dialog-warning-symbolic"),
    ("calculator", "accessories-calculator-symbolic"),
    ("calendar", "x-office-calendar-symbolic"),
    ("camera", "camera-photo-symbolic"),
    ("carousel", "view-paged-symbolic"),
    ("cart", "system-software-install-symbolic"),
    ("check", "object-select-symbolic"),
    ("checkmark", "object-select-symbolic"),
    ("chevron-down", "pan-down-symbolic"),
    ("chevron-left", "pan-start-symbolic"),
    ("chevron-right", "pan-end-symbolic"),
    ("chevron-up", "pan-up-symbolic"),
    ("clock", "preferences-system-time-symbolic"),
    ("code", "applications-engineering-symbolic"),
    ("cog", "emblem-system-symbolic"),
    ("coin", "drive-harddisk-symbolic"),
    ("copy-clipboard", "edit-paste-symbolic"),
    ("desktop", "computer-symbolic"),
    ("download", "folder-download-symbolic"),
    ("duplicate", "edit-copy-symbolic"),
    ("ellipsis", "view-more-symbolic"),
    ("emoji", "face-smile-symbolic"),
    ("envelope", "mail-unread-symbolic"),
    ("exclamationmark", "dialog-warning-symbolic"),
    ("eye", "view-reveal-symbolic"),
    ("eye-disabled", "view-conceal-symbolic"),
    ("floating-window", "window-restore-symbolic"),
    ("folder", "folder-symbolic"),
    ("forward", "media-skip-forward-symbolic"),
    ("fullscreen", "view-fullscreen-symbolic"),
    ("globe-01", "web-browser-symbolic"),
    ("hammer", "applications-engineering-symbolic"),
    ("hard-drive", "drive-harddisk-symbolic"),
    ("heart", "emblem-favorite-symbolic"),
    ("house", "user-home-symbolic"),
    ("image", "image-x-generic-symbolic"),
    ("info-01", "dialog-information-symbolic"),
    ("key", "dialog-password-symbolic"),
    ("keyboard", "input-keyboard-symbolic"),
    ("link", "insert-link-symbolic"),
    ("lock", "system-lock-screen-symbolic"),
    ("logout", "system-log-out-symbolic"),
    ("magnifying-glass", "system-search-symbolic"),
    ("maximize", "view-fullscreen-symbolic"),
    ("microphone", "audio-input-microphone-symbolic"),
    ("minus", "list-remove-symbolic"),
    ("monitor", "video-display-symbolic"),
    ("moon", "weather-clear-night-symbolic"),
    ("music", "audio-x-generic-symbolic"),
    ("network", "network-workgroup-symbolic"),
    ("new-document", "document-new-symbolic"),
    ("new-folder", "folder-new-symbolic"),
    ("overview", "view-app-grid-symbolic"),
    ("paragraph", "text-x-generic-symbolic"),
    ("pause", "media-playback-pause-symbolic"),
    ("pencil", "document-edit-symbolic"),
    ("person", "avatar-default-symbolic"),
    ("pin", "view-pin-symbolic"),
    ("play", "media-playback-start-symbolic"),
    ("plug", "application-x-addon-symbolic"),
    ("plus", "list-add-symbolic"),
    ("power", "system-shutdown-symbolic"),
    ("print", "printer-symbolic"),
    ("question-mark-circle", "dialog-question-symbolic"),
    ("redo", "view-refresh-symbolic"),
    ("repeat", "media-playlist-repeat-symbolic"),
    ("rewind", "media-skip-backward-symbolic"),
    ("rotate-clockwise", "system-reboot-symbolic"),
    ("save-document", "document-save-symbolic"),
    ("shuffle", "media-playlist-shuffle-symbolic"),
    ("snippets", "text-editor-symbolic"),
    ("speaker-down", "audio-volume-low-symbolic"),
    ("speaker-high", "audio-volume-high-symbolic"),
    ("speaker-low", "audio-volume-low-symbolic"),
    ("speaker-off", "audio-volume-muted-symbolic"),
    ("speaker-on", "audio-volume-high-symbolic"),
    ("speaker-up", "audio-volume-high-symbolic"),
    ("star", "starred-symbolic"),
    ("stop", "media-playback-stop-symbolic"),
    ("sun", "weather-clear-symbolic"),
    ("switch-windows", "focus-windows-symbolic"),
    ("terminal", "utilities-terminal-symbolic"),
    ("text", "font-x-generic-symbolic"),
    ("trash", "user-trash-symbolic"),
    ("undo", "edit-undo-symbolic"),
    ("upload", "send-to-symbolic"),
    ("warning", "dialog-warning-symbolic"),
    ("wifi", "network-wireless-symbolic"),
    ("xmark", "window-close-symbolic"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_symbolic_stand_in_is_for_a_builtin_and_the_table_is_sorted() {
        assert!(SYMBOLIC.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for (builtin, _) in SYMBOLIC {
            assert!(is_builtin(builtin), "{builtin}");
        }
        assert_eq!(symbolic_name("cog"), Some("emblem-system-symbolic"));
        assert_eq!(symbolic_name("github"), None);
    }

    #[test]
    fn the_icons_are_found_under_the_first_data_dir_that_has_them() {
        let none = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(app.path().join("compass/builtin-icons")).unwrap();
        let dirs = format!("{}:{}", none.path().display(), app.path().display());
        assert_eq!(
            directory_in(None, None, Some(&dirs)),
            Some(app.path().join("compass/builtin-icons")),
            "the Flatpak's /app/share is one of $XDG_DATA_DIRS"
        );
        assert_eq!(
            directory_in(Some("/icons".into()), None, Some(&dirs)),
            Some(std::path::PathBuf::from("/icons"))
        );
    }
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels below the repository root")
            .to_path_buf()
    }

    #[test]
    fn every_name_has_an_svg_and_every_svg_has_a_name() {
        // The check that matters: a name with no file is a blank icon, and a
        // file with no name is an asset nobody can ask for. Both are silent.
        let dir = repo().join("extra/builtin-icons");
        let files: BTreeSet<String> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension()? == "svg")
                    .then(|| path.file_stem()?.to_str().map(ToOwned::to_owned))
                    .flatten()
            })
            .collect();

        assert!(files.len() > 800, "only {} icons on disk", files.len());

        let named: BTreeSet<String> = names().iter().map(|n| (*n).to_owned()).collect();

        let missing: Vec<&String> = named.difference(&files).collect();
        assert!(
            missing.is_empty(),
            "{} icon names have no file: {missing:?}",
            missing.len()
        );

        let unnamed: Vec<&String> = files.difference(&named).collect();
        assert!(
            unnamed.is_empty(),
            "{} icon files have no name in the enum: {unnamed:?}",
            unnamed.len()
        );
    }

    #[test]
    fn the_names_are_unique_and_kebab_case() {
        let mut seen = BTreeSet::new();
        for name in names() {
            assert!(seen.insert(*name), "{name} appears twice");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{name} is not the kebab-case spelling the files use"
            );
        }
    }

    #[test]
    fn the_unknown_icon_is_one_of_the_names() {
        // The fallback has to exist, or the failure path fails too.
        assert!(is_builtin(UNKNOWN), "{UNKNOWN} is not in the table");
        assert!(
            repo()
                .join("extra/builtin-icons")
                .join("question-mark-circle.svg")
                .exists(),
            "the fallback icon has no file"
        );
    }

    #[test]
    fn a_name_that_is_not_ours_resolves_to_nothing() {
        assert!(!is_builtin("definitely-not-an-icon"));
        assert_eq!(file_name("definitely-not-an-icon"), None);
        assert_eq!(
            file_name("alarm-ringing"),
            Some("alarm-ringing.svg".to_owned())
        );
    }

    #[test]
    fn the_table_is_as_long_as_the_api_enum() {
        // `icon.ts` has one `Name = "name",` line per icon.
        let source = std::fs::read_to_string(repo().join("src/typescript/api/src/api/icon.ts"))
            .expect("read the API's Icon enum");
        let lines = source.matches(" = \"").count();
        assert_eq!(
            names().len(),
            lines,
            "the parser found {} of the enum's {lines} members",
            names().len()
        );
    }
}
