//! The typed, high level view of a desktop entry file.
//!
//! Ported from `src/lib/xdgpp/xdgpp/desktop-entry/entry.cpp` and `action.cpp`.

use std::path::{Path, PathBuf};

use crate::exec::ExecParser;
use crate::locale::Locale;
use crate::reader::{Group, Reader};

/// The name of the required main group.
pub const MAIN_GROUP: &str = "Desktop Entry";

/// The prefix of a desktop action group.
pub const ACTION_GROUP_PREFIX: &str = "Desktop Action ";

/// Everything that can go wrong while turning a file into a [`DesktopEntry`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file does not contain a `[Desktop Entry]` group.
    #[error("no `[{MAIN_GROUP}]` group was found")]
    MissingMainGroup,

    /// The required `Name` key is missing.
    #[error("the `Name` key is required")]
    MissingName,

    /// `Type=Link` requires a `URL` key.
    #[error("the `URL` key is required when `Type=Link`")]
    MissingUrl,

    /// The file could not be read.
    #[error("could not read desktop entry at {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The type of a desktop entry, from the `Type` key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EntryType {
    /// `Type=Application`, also the default when the key is missing.
    #[default]
    Application,
    /// `Type=Link`.
    Link,
    /// `Type=Directory`.
    Directory,
    /// An unknown type. The specification says such entries should be ignored.
    Other(String),
}

/// Options driving [`DesktopEntry::parse_with`].
#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    /// The desktop file id, as computed from its location. Purely informative.
    pub id: Option<String>,
    /// The locale localized keys are resolved against. Defaults to
    /// [`Locale::system`].
    pub locale: Option<Locale>,
    /// The location of the file, used to expand the `%k` field code.
    pub path: Option<PathBuf>,
}

/// An additional action offered by an application, from a
/// `[Desktop Action <id>]` group.
#[derive(Debug, Clone)]
pub struct DesktopAction {
    id: String,
    name: String,
    icon: Option<String>,
    exec: Option<String>,
    entry_path: Option<PathBuf>,
}

impl DesktopAction {
    fn from_group(group: &Group, entry_path: Option<&Path>) -> DesktopAction {
        DesktopAction {
            id: group
                .name()
                .strip_prefix(ACTION_GROUP_PREFIX)
                .unwrap_or_default()
                .to_owned(),
            name: group.string("Name").unwrap_or_default(),
            icon: group.string("Icon"),
            exec: group.string("Exec"),
            entry_path: entry_path.map(Path::to_path_buf),
        }
    }

    /// The action id, i.e. the part of the group name after
    /// `Desktop Action `.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The localized action name. Empty when the (required) key is missing.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// The unexpanded `Exec` value.
    #[must_use]
    pub fn exec(&self) -> Option<&str> {
        self.exec.as_deref()
    }

    /// Expands the action `Exec` key with no URI.
    #[must_use]
    pub fn expand_exec(&self) -> Vec<String> {
        self.expand_exec_with(&[], false, None)
    }

    /// Expands the action `Exec` key. See [`DesktopEntry::expand_exec_with`].
    #[must_use]
    pub fn expand_exec_with(
        &self,
        uris: &[&str],
        force_append: bool,
        launch_prefix: Option<&str>,
    ) -> Vec<String> {
        let Some(exec) = &self.exec else {
            return Vec::new();
        };

        expand(
            ExecParser::new(&self.name)
                .with_icon(self.icon.as_deref())
                .with_entry_path(self.entry_path.as_deref().and_then(Path::to_str))
                .with_force_append(force_append),
            exec,
            uris,
            launch_prefix,
        )
    }
}

fn expand(
    parser: ExecParser<'_>,
    exec: &str,
    uris: &[&str],
    launch_prefix: Option<&str>,
) -> Vec<String> {
    match launch_prefix {
        Some(prefix) => parser.parse(&format!("{prefix} {exec}"), uris),
        None => parser.parse(exec, uris),
    }
}

/// A parsed desktop entry file.
#[derive(Debug, Clone)]
pub struct DesktopEntry {
    id: Option<String>,
    path: Option<PathBuf>,
    entry_type: EntryType,
    version: Option<String>,
    name: String,
    unlocalized_name: Option<String>,
    generic_name: Option<String>,
    comment: Option<String>,
    icon: Option<String>,
    exec: Option<String>,
    try_exec: Option<String>,
    working_directory: Option<PathBuf>,
    url: Option<String>,
    startup_wm_class: Option<String>,
    terminal: bool,
    no_display: bool,
    hidden: bool,
    single_main_window: bool,
    categories: Vec<String>,
    keywords: Vec<String>,
    mime_types: Vec<String>,
    only_show_in: Vec<String>,
    not_show_in: Vec<String>,
    actions: Vec<DesktopAction>,
}

impl DesktopEntry {
    /// Parses `data` using the locale of the current process.
    ///
    /// # Errors
    ///
    /// Fails when the `[Desktop Entry]` group or the `Name` key is missing, or
    /// when a `Type=Link` entry has no `URL`.
    pub fn parse(data: &str) -> Result<DesktopEntry, Error> {
        DesktopEntry::parse_with(data, &ParseOptions::default())
    }

    /// Parses `data` with explicit [`ParseOptions`].
    ///
    /// # Errors
    ///
    /// See [`DesktopEntry::parse`].
    pub fn parse_with(data: &str, opts: &ParseOptions) -> Result<DesktopEntry, Error> {
        let locale = opts.locale.clone().unwrap_or_else(Locale::system);
        let reader = Reader::parse(data, locale);
        let group = reader.group(MAIN_GROUP).ok_or(Error::MissingMainGroup)?;

        let entry_type = match group.string("Type").as_deref() {
            None | Some("Application") => EntryType::Application,
            Some("Link") => EntryType::Link,
            Some("Directory") => EntryType::Directory,
            Some(other) => EntryType::Other(other.to_owned()),
        };

        let url = group.string("URL");

        if entry_type == EntryType::Link && url.is_none() {
            return Err(Error::MissingUrl);
        }

        let name = group.string("Name").ok_or(Error::MissingName)?;
        let path = opts.path.clone();

        let actions = group
            .string_list("Actions")
            .iter()
            .filter_map(|id| reader.group(&format!("{ACTION_GROUP_PREFIX}{id}")))
            .map(|group| DesktopAction::from_group(group, path.as_deref()))
            .collect();

        Ok(DesktopEntry {
            id: opts.id.clone(),
            entry_type,
            version: group.string("Version"),
            name,
            unlocalized_name: group.unlocalized_string("Name"),
            generic_name: group.string("GenericName"),
            comment: group.string("Comment"),
            icon: group.string("Icon"),
            exec: group.string("Exec"),
            try_exec: group.string("TryExec"),
            working_directory: group.string("Path").map(PathBuf::from),
            url,
            startup_wm_class: group.string("StartupWMClass"),
            terminal: group.bool("Terminal"),
            no_display: group.bool("NoDisplay"),
            hidden: group.bool("Hidden"),
            single_main_window: group.bool("SingleMainWindow"),
            categories: group.string_list("Categories"),
            keywords: group.string_list("Keywords"),
            mime_types: group.string_list("MimeType"),
            only_show_in: group.string_list("OnlyShowIn"),
            not_show_in: group.string_list("NotShowIn"),
            actions,
            path,
        })
    }

    /// Reads and parses the desktop entry stored at `path`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read, and for the same reasons as
    /// [`DesktopEntry::parse`].
    pub fn from_file(path: impl AsRef<Path>) -> Result<DesktopEntry, Error> {
        DesktopEntry::from_file_with(path, &ParseOptions::default())
    }

    /// Reads and parses the desktop entry stored at `path` with explicit
    /// options. The `path` option is set from `path`.
    ///
    /// # Errors
    ///
    /// See [`DesktopEntry::from_file`].
    pub fn from_file_with(
        path: impl AsRef<Path>,
        opts: &ParseOptions,
    ) -> Result<DesktopEntry, Error> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;

        // Decoded lossily on purpose. The spec says desktop files are UTF-8, but we are scanning a
        // system we do not control, and real ones are not always. Rejecting the file would lose the
        // whole application over one bad byte in a Comment nobody reads -- a much worse outcome for
        // a launcher than showing a replacement character. Strict decoding is a validator's policy,
        // not a scanner's.
        let data = String::from_utf8_lossy(&bytes);

        DesktopEntry::parse_with(
            &data,
            &ParseOptions {
                path: Some(path.to_path_buf()),
                ..opts.clone()
            },
        )
    }

    /// The desktop file id, when the entry was created with one.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// The location of the file this entry was read from, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[must_use]
    pub fn entry_type(&self) -> &EntryType {
        &self.entry_type
    }

    #[must_use]
    pub fn is_application(&self) -> bool {
        self.entry_type == EntryType::Application
    }

    #[must_use]
    pub fn is_link(&self) -> bool {
        self.entry_type == EntryType::Link
    }

    #[must_use]
    pub fn is_directory(&self) -> bool {
        self.entry_type == EntryType::Directory
    }

    /// Version of the specification the entry conforms with.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// The specific name of the application, localized when available.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The `Name` key without any locale suffix, when present.
    #[must_use]
    pub fn unlocalized_name(&self) -> Option<&str> {
        self.unlocalized_name.as_deref()
    }

    /// The generic name of the application, e.g. "Web Browser".
    #[must_use]
    pub fn generic_name(&self) -> Option<&str> {
        self.generic_name.as_deref()
    }

    /// Tooltip for the entry, e.g. "View sites on the Internet".
    #[must_use]
    pub fn comment(&self) -> Option<&str> {
        self.comment.as_deref()
    }

    /// The icon name, or an absolute path to an icon file.
    #[must_use]
    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// The unexpanded `Exec` value. Use [`DesktopEntry::expand_exec`] for the
    /// argument vector.
    #[must_use]
    pub fn exec(&self) -> Option<&str> {
        self.exec.as_deref()
    }

    /// An executable whose presence in `$PATH` tells whether the program is
    /// actually installed.
    #[must_use]
    pub fn try_exec(&self) -> Option<&str> {
        self.try_exec.as_deref()
    }

    /// The `Path` key: the working directory to run the program in.
    #[must_use]
    pub fn working_directory(&self) -> Option<&Path> {
        self.working_directory.as_deref()
    }

    /// The `URL` key, required for and specific to `Type=Link` entries.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// Whether the program runs in a terminal window.
    #[must_use]
    pub fn terminal(&self) -> bool {
        self.terminal
    }

    /// "This application exists, but don't display it in the menus".
    #[must_use]
    pub fn no_display(&self) -> bool {
        self.no_display
    }

    /// The `Hidden` key: the entry should be treated as if it did not exist.
    #[must_use]
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn single_main_window(&self) -> bool {
        self.single_main_window
    }

    /// The WM class of at least one window the application will map.
    #[must_use]
    pub fn startup_wm_class(&self) -> Option<&str> {
        self.startup_wm_class.as_deref()
    }

    #[must_use]
    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    /// Strings that may be used in addition to the other metadata to describe
    /// this entry, e.g. to facilitate searching.
    #[must_use]
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }

    /// The MIME types supported by this application.
    #[must_use]
    pub fn mime_types(&self) -> &[String] {
        &self.mime_types
    }

    #[must_use]
    pub fn only_show_in(&self) -> &[String] {
        &self.only_show_in
    }

    #[must_use]
    pub fn not_show_in(&self) -> &[String] {
        &self.not_show_in
    }

    #[must_use]
    pub fn actions(&self) -> &[DesktopAction] {
        &self.actions
    }

    #[must_use]
    pub fn action(&self, id: &str) -> Option<&DesktopAction> {
        self.actions.iter().find(|action| action.id() == id)
    }

    #[must_use]
    pub fn has_category(&self, category: &str) -> bool {
        self.categories.iter().any(|it| it == category)
    }

    #[must_use]
    pub fn supports_mime(&self, mime: &str) -> bool {
        self.mime_types.iter().any(|it| it == mime)
    }

    /// Checks only the `OnlyShowIn` and `NotShowIn` keys against
    /// `current_desktops`, which is what `$XDG_CURRENT_DESKTOP` holds.
    #[must_use]
    pub fn matches_desktop(&self, current_desktops: &[&str]) -> bool {
        let matches = |list: &[String]| {
            list.iter()
                .any(|entry| current_desktops.contains(&entry.as_str()))
        };

        if !self.only_show_in.is_empty() && !matches(&self.only_show_in) {
            return false;
        }

        if matches(&self.not_show_in) {
            return false;
        }

        true
    }

    /// Whether the entry should be shown in the current environment. Combines
    /// the `Hidden`, `NoDisplay`, `OnlyShowIn` and `NotShowIn` keys.
    #[must_use]
    pub fn should_show(&self, current_desktops: &[&str]) -> bool {
        if self.hidden || self.no_display {
            return false;
        }

        self.matches_desktop(current_desktops)
    }

    /// Expands the `Exec` key with no URI. Yields an empty vector when the
    /// entry has no `Exec` key.
    #[must_use]
    pub fn expand_exec(&self) -> Vec<String> {
        self.expand_exec_with(&[], false, None)
    }

    /// Expands the `Exec` key into an argument vector.
    ///
    /// `uris` feeds the `%f`, `%F`, `%u` and `%U` field codes. When
    /// `force_append` is set and none of those field codes was present, the
    /// URIs are appended at the end of the command line. `launch_prefix` is
    /// prepended to the command line before expansion, e.g. to run the program
    /// through a wrapper.
    #[must_use]
    pub fn expand_exec_with(
        &self,
        uris: &[&str],
        force_append: bool,
        launch_prefix: Option<&str>,
    ) -> Vec<String> {
        let Some(exec) = &self.exec else {
            return Vec::new();
        };

        expand(
            ExecParser::new(&self.name)
                .with_icon(self.icon.as_deref())
                .with_entry_path(self.path.as_deref().and_then(Path::to_str))
                .with_force_append(force_append),
            exec,
            uris,
            launch_prefix,
        )
    }
}
