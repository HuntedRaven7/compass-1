//! The application index: `.desktop` files turned into searchable items.
//!
//! # Scanning and precedence
//!
//! The index walks the XDG application directories in precedence order —
//! `$XDG_DATA_HOME/applications` first, then each `$XDG_DATA_DIRS/applications` — and recurses
//! into subdirectories. A file's *desktop file id* is its path relative to the applications
//! directory with each `/` replaced by `-`, so
//! `/usr/share/applications/kde4/konsole.desktop` has the id `kde4-konsole.desktop`.
//!
//! **The first directory to supply an id wins, and later directories with the same id are
//! ignored.** That is what makes a file dropped in `~/.local/share/applications` an override
//! rather than a duplicate. Shadowing happens on the *id*, before any visibility check: if the
//! winning file is `Hidden=true` the application is gone, it does not fall back to the system
//! copy. The specification is explicit that `Hidden` means "deleted", and falling back would make
//! it impossible for a user to remove a system entry.
//!
//! # What is indexed
//!
//! * `Type=Application` and `Type=Link` entries. Links launch their URL through
//!   the desktop's URI handler; Directory and unknown types are not launcher rows.
//! * Entries passing [`DesktopEntry::should_show`] against the configured desktops, i.e. not
//!   `Hidden`, not `NoDisplay`, and allowed by `OnlyShowIn`/`NotShowIn`.
//! * Every `[Desktop Action …]` group of an included entry becomes its own [`AppItem`], because
//!   "Firefox → New Private Window" is a thing people search for directly.
//!
//! # `TryExec`
//!
//! An entry whose `TryExec` names a binary that does not resolve is **indexed but marked
//! unlaunchable** ([`AppItem::launchable`]), not dropped. See [`AppIndexBuilder::include_unlaunchable`]
//! for the reasoning.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use compass_search::{FuzzySearchable, WeightedField};
use compass_xdg::{
    DesktopAction, DesktopEntry, DesktopFile, Error, Locale, ParseOptions, scan_desktop_files,
};

/// Weight of the item's own display name.
pub const WEIGHT_NAME: f32 = 1.0;
/// Weight of the parent application's name, on action items only.
pub const WEIGHT_APP_NAME: f32 = 0.75;
/// Weight of `GenericName`.
pub const WEIGHT_GENERIC_NAME: f32 = 0.6;
/// Weight of each `Keywords` entry.
pub const WEIGHT_KEYWORD: f32 = 0.5;
/// Weight of `Comment`.
pub const WEIGHT_COMMENT: f32 = 0.3;
/// Weight of each `Categories` entry.
pub const WEIGHT_CATEGORY: f32 = 0.2;

/// The separator between a desktop file id and an action id in an [`AppItem::key`].
pub const ACTION_KEY_SEPARATOR: &str = "::";

/// Why a file in an application directory did not produce an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// A directory earlier in the search path already supplied this desktop file id.
    Shadowed {
        /// The id that was already taken.
        id: String,
        /// The file that won.
        winner: PathBuf,
    },
    /// The file could not be read.
    Unreadable(String),
    /// The file is not a well-formed desktop entry.
    Malformed(String),
    /// The entry is neither `Type=Application` nor `Type=Link`.
    NotAnApplication,
    /// `Hidden`, `NoDisplay`, or excluded by `OnlyShowIn`/`NotShowIn`.
    NotShown,
    /// `TryExec` did not resolve and unlaunchable entries were excluded.
    TryExecMissing(String),
    /// A `Type=Application` entry with no `Exec` key: there is nothing to launch.
    NoExec,
    /// A `Type=Link` entry without a nonempty URL.
    NoUrl,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::Shadowed { id, winner } => {
                write!(f, "shadowed: {id} already provided by {}", winner.display())
            }
            SkipReason::Unreadable(err) => write!(f, "unreadable: {err}"),
            SkipReason::Malformed(err) => write!(f, "malformed: {err}"),
            SkipReason::NotAnApplication => f.write_str("not Type=Application or Type=Link"),
            SkipReason::NotShown => f.write_str("hidden in this environment"),
            SkipReason::TryExecMissing(exec) => write!(f, "TryExec {exec} did not resolve"),
            SkipReason::NoExec => f.write_str("no Exec key"),
            SkipReason::NoUrl => f.write_str("no URL key"),
        }
    }
}

/// A file the scanner looked at and did not index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    /// The file.
    pub path: PathBuf,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// One searchable thing: an application, or one of its desktop actions.
#[derive(Debug, Clone)]
pub struct AppItem {
    key: String,
    desktop_id: String,
    action_id: Option<String>,
    entry: Arc<DesktopEntry>,
    action_index: Option<usize>,
    name: String,
    app_name: String,
    launchable: bool,
}

impl AppItem {
    /// A stable identifier for this item, unique within an index and usable as a frecency key.
    ///
    /// `firefox.desktop` for an application, `firefox.desktop::new-private-window` for an action.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The desktop file id, e.g. `org.mozilla.firefox.desktop`.
    #[must_use]
    pub fn desktop_id(&self) -> &str {
        &self.desktop_id
    }

    /// The action id, for an action item.
    #[must_use]
    pub fn action_id(&self) -> Option<&str> {
        self.action_id.as_deref()
    }

    /// Whether this item is a desktop action rather than the application itself.
    #[must_use]
    pub fn is_action(&self) -> bool {
        self.action_id.is_some()
    }

    /// The name to show: the action name for actions, the application name otherwise.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The owning application's name, for both applications and actions.
    #[must_use]
    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    /// A one-line label: `Firefox` or `Firefox → New Private Window`.
    #[must_use]
    pub fn display_name(&self) -> String {
        if self.is_action() {
            format!("{} \u{2192} {}", self.app_name, self.name)
        } else {
            self.name.clone()
        }
    }

    /// The `GenericName`, e.g. "Web Browser". Actions inherit the application's.
    #[must_use]
    pub fn generic_name(&self) -> Option<&str> {
        self.entry.generic_name()
    }

    /// The `Comment`. Actions inherit the application's.
    #[must_use]
    pub fn comment(&self) -> Option<&str> {
        self.entry.comment()
    }

    /// The `Keywords`. Actions inherit the application's.
    #[must_use]
    pub fn keywords(&self) -> &[String] {
        self.entry.keywords()
    }

    /// The `Categories`. Actions inherit the application's.
    #[must_use]
    pub fn categories(&self) -> &[String] {
        self.entry.categories()
    }

    /// The icon name or path: the action's own if it has one, else the application's.
    #[must_use]
    pub fn icon(&self) -> Option<&str> {
        self.action()
            .and_then(DesktopAction::icon)
            .or_else(|| self.entry.icon())
    }

    /// The file this item came from.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.entry.path()
    }

    /// The parsed desktop entry behind this item.
    #[must_use]
    pub fn entry(&self) -> &DesktopEntry {
        &self.entry
    }

    /// The desktop action behind this item, for action items.
    #[must_use]
    pub fn action(&self) -> Option<&DesktopAction> {
        self.action_index.map(|i| &self.entry.actions()[i])
    }

    /// Whether the item can actually be launched.
    ///
    /// False when `TryExec` names a binary that did not resolve at index time. See
    /// [`AppIndexBuilder::include_unlaunchable`].
    #[must_use]
    pub fn launchable(&self) -> bool {
        self.launchable
    }

    /// Whether the program wants a terminal.
    #[must_use]
    pub fn terminal(&self) -> bool {
        self.entry.terminal()
    }

    /// The argument vector to run, with no URIs.
    #[must_use]
    pub fn command(&self) -> Vec<String> {
        self.command_with(&[])
    }

    /// The argument vector to run, expanding the `Exec` field codes with `uris`.
    #[must_use]
    pub fn command_with(&self, uris: &[&str]) -> Vec<String> {
        match self.action() {
            Some(action) => action.expand_exec_with(uris, false, None),
            None => self.entry.expand_exec_with(uris, false, None),
        }
    }
}

/// Field weighting rationale, in one place so the numbers are reviewable:
///
/// `score_weighted` takes the *maximum* weighted field score per query word and normalises it
/// against a perfect match, so a weight is directly "how close to a perfect match can this field
/// get". Name is the thing users type, so it is the only field worth `1.0`. An action's parent
/// name gets `0.75`: typing `firefox` must surface the actions, but always below Firefox itself,
/// otherwise one application with six actions buries the next application. `GenericName` (`0.6`)
/// is an editorial synonym — "Web Browser" should find Firefox but lose to an app literally named
/// "Web Browser". `Keywords` (`0.5`) is vendor-supplied search bait, useful but noisier. `Comment`
/// (`0.3`) is prose written to be read, not matched, and matches in it are frequently incidental.
/// `Categories` (`0.2`) is a fixed vocabulary shared by dozens of entries, so a category hit
/// discriminates almost nothing; it is kept only so `network` finds something rather than nothing.
impl FuzzySearchable for AppItem {
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
        out.push(WeightedField::new(&self.name, WEIGHT_NAME));

        if self.is_action() {
            out.push(WeightedField::new(&self.app_name, WEIGHT_APP_NAME));
        }

        if let Some(generic) = self.entry.generic_name() {
            out.push(WeightedField::new(generic, WEIGHT_GENERIC_NAME));
        }

        for keyword in self.entry.keywords() {
            out.push(WeightedField::new(keyword, WEIGHT_KEYWORD));
        }

        if let Some(comment) = self.entry.comment() {
            out.push(WeightedField::new(comment, WEIGHT_COMMENT));
        }

        for category in self.entry.categories() {
            out.push(WeightedField::new(category, WEIGHT_CATEGORY));
        }
    }
}

/// How to build an [`AppIndex`].
///
/// Nothing is read from the environment unless you ask for it: [`AppIndexBuilder::default`] scans
/// no directories, resolves `TryExec` against no `PATH`, and uses the default (POSIX) locale. Call
/// [`AppIndexBuilder::from_environment`] for the real thing. That asymmetry is on purpose — a test
/// must never be able to accidentally read the invoking user's applications.
#[derive(Debug, Clone, Default)]
pub struct AppIndexBuilder {
    dirs: Vec<PathBuf>,
    desktops: Vec<String>,
    exec_search_path: Vec<PathBuf>,
    locale: Option<Locale>,
    include_unlaunchable: bool,
    include_actions: bool,
    extension_dirs: Vec<PathBuf>,
}

impl AppIndexBuilder {
    /// A builder that reads nothing. See the type documentation.
    #[must_use]
    pub fn new() -> AppIndexBuilder {
        AppIndexBuilder {
            include_unlaunchable: true,
            include_actions: true,
            ..AppIndexBuilder::default()
        }
    }

    /// A builder configured from `$XDG_DATA_HOME`, `$XDG_DATA_DIRS`, `$XDG_CURRENT_DESKTOP`,
    /// `$PATH` and the process locale.
    #[must_use]
    pub fn from_environment() -> AppIndexBuilder {
        AppIndexBuilder::new()
            .dirs(crate::xdg_dirs::application_dirs())
            .desktops(crate::xdg_dirs::current_desktops())
            .exec_search_path(crate::xdg_dirs::exec_search_path())
            .locale(Locale::system())
            .extension_dirs(crate::manifest::registry::search_paths())
    }

    /// Where installed extensions are looked for, highest precedence first
    /// ([`crate::manifest::registry::search_paths`] in a real install). Their
    /// commands join the root after the builtin commands. None by default,
    /// so an index built for a test reads only what it is given.
    #[must_use]
    pub fn extension_dirs(
        mut self,
        dirs: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> AppIndexBuilder {
        self.extension_dirs = dirs.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the application directories, in precedence order (highest first).
    #[must_use]
    pub fn dirs(mut self, dirs: impl IntoIterator<Item = impl Into<PathBuf>>) -> AppIndexBuilder {
        self.dirs = dirs.into_iter().map(Into::into).collect();
        self
    }

    /// Appends one application directory, lower precedence than everything already added.
    #[must_use]
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> AppIndexBuilder {
        self.dirs.push(dir.into());
        self
    }

    /// Sets the desktop names `OnlyShowIn`/`NotShowIn` are matched against, i.e. the contents of
    /// `$XDG_CURRENT_DESKTOP`.
    #[must_use]
    pub fn desktops(
        mut self,
        desktops: impl IntoIterator<Item = impl Into<String>>,
    ) -> AppIndexBuilder {
        self.desktops = desktops.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the directories a bare `TryExec` is resolved against, i.e. `$PATH`.
    #[must_use]
    pub fn exec_search_path(
        mut self,
        path: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> AppIndexBuilder {
        self.exec_search_path = path.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the locale localized keys are resolved against.
    #[must_use]
    pub fn locale(mut self, locale: Locale) -> AppIndexBuilder {
        self.locale = Some(locale);
        self
    }

    /// Whether entries whose `TryExec` did not resolve are indexed (default: yes).
    ///
    /// **They are kept, and marked [`AppItem::launchable`] `= false`.** `TryExec` resolution is a
    /// guess: the indexer's `PATH` is not necessarily the launcher's, and on the first target
    /// platform it is emphatically not — inside a Flatpak sandbox almost no host binary resolves,
    /// while `flatpak-spawn --host` would run them fine. Treating a failed `TryExec` as "delete
    /// this application" would make most of the user's system vanish from the launcher with no
    /// explanation, which is a far worse failure than showing an entry that turns out not to run.
    /// So the index records the fact and lets the UI decide: grey the item out, sort it last, or
    /// call [`AppIndex::launchable_items`] and never show it. Set this to `false` for a caller
    /// that genuinely wants them gone; they are then reported in [`AppIndex::skipped`] rather than
    /// disappearing silently.
    #[must_use]
    pub fn include_unlaunchable(mut self, include: bool) -> AppIndexBuilder {
        self.include_unlaunchable = include;
        self
    }

    /// Whether `[Desktop Action …]` groups become their own items (default: yes).
    #[must_use]
    pub fn include_actions(mut self, include: bool) -> AppIndexBuilder {
        self.include_actions = include;
        self
    }

    /// Scans the configured directories and builds the index.
    ///
    /// Scanning never fails: an unreadable directory, an unreadable file, or a malformed entry is
    /// recorded in [`AppIndex::skipped`] and the scan continues. A launcher that refuses to start
    /// because one `.desktop` file on the system is broken is not shippable.
    #[must_use]
    pub fn build(self) -> AppIndex {
        let scan = AppIndexBuilder {
            extension_dirs: Vec::new(),
            ..self.clone()
        };
        let mut items: Vec<AppItem> = Vec::new();
        let mut skipped: Vec<SkippedEntry> = Vec::new();
        // Desktop file id -> the file that claimed it. Claiming happens before any visibility
        // check, so a higher-precedence Hidden entry really does delete the lower-precedence one.
        let mut claimed: HashMap<String, PathBuf> = HashMap::new();
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut hidden: Vec<AppItem> = Vec::new();

        for dir in &self.dirs {
            let scan = scan_desktop_files(dir);
            skipped.extend(scan.errors.into_iter().map(|error| SkippedEntry {
                path: error.path,
                reason: SkipReason::Unreadable(error.message),
            }));

            for file in scan.files {
                let id = file.id();
                let path = file.path();
                if let Some(winner) = claimed.get(id) {
                    skipped.push(SkippedEntry {
                        path: path.to_path_buf(),
                        reason: SkipReason::Shadowed {
                            id: id.to_owned(),
                            winner: winner.clone(),
                        },
                    });
                    continue;
                }
                claimed.insert(id.to_owned(), path.to_path_buf());

                self.index_file(
                    &file,
                    &self.desktops,
                    &mut items,
                    &mut by_key,
                    &mut skipped,
                    &mut hidden,
                );
            }
        }

        let mut root_indices: Vec<_> = items
            .iter()
            .enumerate()
            .filter_map(|(index, app)| (!app.is_action()).then_some(index))
            .collect();
        root_indices.sort_by_cached_key(|&index| items[index].name().to_lowercase());
        let mut roots = Vec::with_capacity(root_indices.len());
        for &index in &root_indices {
            let app = &items[index];
            let keywords: Vec<_> = app
                .categories()
                .iter()
                .chain(app.keywords())
                .cloned()
                .collect();
            roots.push(crate::root_items::app_root_item(
                app.desktop_id(),
                app.name(),
                &keywords,
                app.entry().unlocalized_name(),
            ));
        }
        // After every application, so `root_indices[i]` still names the item
        // behind `roots[i]` for each application root; a command's position
        // is past its end. See `crate::commands`.
        roots.extend(
            crate::commands::BUILTIN_COMMANDS
                .iter()
                .map(crate::commands::BuiltinCommand::root_item),
        );
        let extensions = if self.extension_dirs.is_empty() {
            Vec::new()
        } else {
            crate::extension_commands::ExtensionCommand::from_manifests(
                &crate::manifest::registry::scan(&self.extension_dirs).extensions,
            )
        };
        roots.extend(
            extensions
                .iter()
                .map(crate::extension_commands::ExtensionCommand::root_item),
        );
        AppIndex {
            roots,
            root_indices,
            items,
            by_key,
            skipped,
            all_extensions: extensions.clone(),
            extensions,
            deny: crate::extension_deny::ExtensionDeny::default(),
            shortcuts: Vec::new(),
            scripts: Vec::new(),
            rhai_scripts: Vec::new(),
            root_config: crate::root_items::RootConfig::default(),
            extension_dirs: self.extension_dirs,
            scan,
            hidden,
            window_capabilities: crate::window_switcher::Capabilities::default(),
        }
    }

    fn index_file(
        &self,
        file: &DesktopFile,
        desktops: &[String],
        items: &mut Vec<AppItem>,
        by_key: &mut HashMap<String, usize>,
        skipped: &mut Vec<SkippedEntry>,
        hidden: &mut Vec<AppItem>,
    ) {
        let id = file.id();
        let path = file.path();
        let opts = ParseOptions {
            locale: self.locale.clone(),
            path: None,
        };

        let entry = match file.parse_with(&opts) {
            Ok(entry) => entry.into_entry(),
            Err(Error::Io { source, .. }) => {
                tracing::debug!(path = %path.display(), err = %source, "could not read desktop entry");
                skipped.push(SkippedEntry {
                    path: path.to_path_buf(),
                    reason: SkipReason::Unreadable(source.to_string()),
                });
                return;
            }
            Err(err) => {
                tracing::debug!(path = %path.display(), %err, "malformed desktop entry");
                skipped.push(SkippedEntry {
                    path: path.to_path_buf(),
                    reason: SkipReason::Malformed(err.to_string()),
                });
                return;
            }
        };

        let is_link = matches!(entry.entry_type(), compass_xdg::EntryType::Link);
        if !entry.is_application() && !is_link {
            skipped.push(SkippedEntry {
                path: path.to_path_buf(),
                reason: SkipReason::NotAnApplication,
            });
            return;
        }

        if !entry.should_show(desktops) {
            skipped.push(SkippedEntry {
                path: path.to_path_buf(),
                reason: SkipReason::NotShown,
            });
            // `NoDisplay` or another desktop's: not in the root, but still an
            // installed application, which Browse Apps lists as "Hidden".
            // `Hidden=true` is a deletion, as the C++ scan's `deleted()`.
            if !entry.hidden() && entry.is_application() && entry.exec().is_some() {
                let launchable = entry
                    .try_exec()
                    .is_none_or(|try_exec| self.resolves(try_exec));
                let name = entry.name().to_owned();
                hidden.push(AppItem {
                    key: id.to_owned(),
                    desktop_id: id.to_owned(),
                    action_id: None,
                    action_index: None,
                    name: name.clone(),
                    app_name: name,
                    entry: Arc::new(entry),
                    launchable,
                });
            }
            return;
        }

        if is_link && entry.url().is_none_or(|url| url.trim().is_empty()) {
            skipped.push(SkippedEntry {
                path: path.to_path_buf(),
                reason: SkipReason::NoUrl,
            });
            return;
        }

        if !is_link && entry.exec().is_none() {
            skipped.push(SkippedEntry {
                path: path.to_path_buf(),
                reason: SkipReason::NoExec,
            });
            return;
        }

        let launchable = match entry.try_exec() {
            Some(try_exec) => self.resolves(try_exec),
            None => true,
        };

        if !launchable && !self.include_unlaunchable {
            skipped.push(SkippedEntry {
                path: path.to_path_buf(),
                reason: SkipReason::TryExecMissing(entry.try_exec().unwrap_or_default().to_owned()),
            });
            return;
        }

        let app_name = entry.name().to_owned();
        let entry = Arc::new(entry);

        push_item(
            items,
            by_key,
            AppItem {
                key: id.to_owned(),
                desktop_id: id.to_owned(),
                action_id: None,
                action_index: None,
                name: app_name.clone(),
                app_name: app_name.clone(),
                entry: Arc::clone(&entry),
                launchable,
            },
        );

        if !self.include_actions || is_link {
            return;
        }

        for (index, action) in entry.actions().iter().enumerate() {
            // An action with no name has nothing to search for, and one with no Exec has nothing
            // to do; the spec requires both, so a missing one means a broken entry.
            let Some(action_name) = action.name() else {
                continue;
            };
            if action_name.is_empty() || action.exec().is_none() {
                continue;
            }

            push_item(
                items,
                by_key,
                AppItem {
                    key: format!("{id}{ACTION_KEY_SEPARATOR}{}", action.id()),
                    desktop_id: id.to_owned(),
                    action_id: Some(action.id().to_owned()),
                    action_index: Some(index),
                    name: action_name.to_owned(),
                    app_name: app_name.clone(),
                    entry: Arc::clone(&entry),
                    launchable,
                },
            );
        }
    }

    /// Whether a `TryExec` value names something that exists.
    ///
    /// A value containing a `/` is a path, absolute or relative to the entry; anything else is
    /// looked up in the configured search path. With no search path configured nothing bare
    /// resolves, which is why [`AppIndexBuilder::new`] is not the right builder for production.
    fn resolves(&self, try_exec: &str) -> bool {
        if try_exec.is_empty() {
            return false;
        }

        if try_exec.contains('/') {
            return is_executable_file(Path::new(try_exec));
        }

        self.exec_search_path
            .iter()
            .any(|dir| is_executable_file(&dir.join(try_exec)))
    }
}

fn push_item(items: &mut Vec<AppItem>, by_key: &mut HashMap<String, usize>, item: AppItem) {
    // Two actions with the same id inside one file would collide; the last one parsed wins the
    // lookup, matching the "later key wins" rule the reader already applies to duplicate keys.
    by_key.insert(item.key.clone(), items.len());
    items.push(item);
}

fn is_executable_file(path: &Path) -> bool {
    // Deliberately not a permission-bit check: `unsafe_code` is forbidden here and the mode bits
    // would need a `std::os::unix` import that makes this file platform-specific for a check that
    // is advisory anyway. Existence as a non-directory is the useful 95%.
    path.is_file()
}

/// A built, searchable index of applications and their actions.
#[derive(Debug, Clone, Default)]
pub struct AppIndex {
    roots: Vec<crate::root_items::RootItem>,
    root_indices: Vec<usize>,
    items: Vec<AppItem>,
    by_key: HashMap<String, usize>,
    skipped: Vec<SkippedEntry>,
    extensions: Vec<crate::extension_commands::ExtensionCommand>,
    /// Everything the last scan found, before the deny filter. Kept so
    /// changing a rule can restore an extension without another filesystem scan.
    all_extensions: Vec<crate::extension_commands::ExtensionCommand>,
    /// The deny policy applied to extensions and their roots (#401). Kept here
    /// so a rescan cannot silently bring a blocked extension back.
    deny: crate::extension_deny::ExtensionDeny,
    /// Quicklinks, in the store's order; their roots come last in `roots`.
    shortcuts: Vec<crate::shortcut_service::CachedShortcut>,
    /// Script commands, in scan order; their roots come after the shortcuts'.
    scripts: Vec<crate::script_scan::ScriptItem>,
    /// Rhai scripts, in id order; their roots come after the script commands'.
    rhai_scripts: Vec<crate::rhai_scripts::RhaiScriptItem>,
    /// The configuration last applied, kept for roots added later.
    root_config: crate::root_items::RootConfig,
    /// Where installed extensions are looked for, kept for a rescan.
    extension_dirs: Vec<PathBuf>,
    /// How the applications were scanned, kept for a rescan
    /// ([`AppIndex::application_scan`]); without the extension directories,
    /// which [`AppIndex::rescan_extensions`] covers.
    scan: AppIndexBuilder,
    /// Applications installed but not shown (`NoDisplay`, or for another
    /// desktop), in scan order; never in the root.
    hidden: Vec<AppItem>,
    /// What the compositor's window manager can do, which decides the
    /// window-management commands root search offers; none until told.
    window_capabilities: crate::window_switcher::Capabilities,
}

/// One row of a root search over applications and commands.
#[derive(Debug)]
pub enum RootHit<'a> {
    /// An application.
    App(ApplicationRootHit<'a>),
    /// A builtin command.
    Command {
        /// Which one.
        command: &'static crate::commands::BuiltinCommand,
        /// Match score on the IPC scale, excluding frecency.
        match_score: u32,
    },
    /// A command from an installed extension.
    Extension {
        /// Which one.
        command: &'a crate::extension_commands::ExtensionCommand,
        /// Match score on the IPC scale, excluding frecency.
        match_score: u32,
    },
    /// A quicklink.
    Shortcut {
        /// Which one.
        shortcut: &'a crate::shortcut_service::CachedShortcut,
        /// Match score on the IPC scale, excluding frecency.
        match_score: u32,
    },
    /// A script command.
    Script {
        /// Which one.
        script: &'a crate::script_scan::ScriptItem,
        /// Match score on the IPC scale, excluding frecency.
        match_score: u32,
    },
    /// A Rhai script.
    RhaiScript {
        /// Which one.
        script: &'a crate::rhai_scripts::RhaiScriptItem,
        /// Match score on the IPC scale, excluding frecency.
        match_score: u32,
    },
}

/// A root application match with its stable index into the application catalog.
#[derive(Debug)]
pub struct ApplicationRootHit<'a> {
    /// The owning application, never a desktop action.
    pub item: &'a AppItem,
    /// Position in `AppIndex::items`, for UI selection and launch dispatch.
    pub index: usize,
    /// Match score on the IPC scale, excluding frecency (zero for empty input).
    pub match_score: u32,
    /// The root item's `provider:entrypoint` id, e.g.
    /// `applications:org.mozilla.firefox`.
    ///
    /// Carried because the caller could not reconstruct it: `AppItem::key` is
    /// the desktop key (`org.mozilla.firefox.desktop`) and the entrypoint id
    /// is what addresses the item across the protocol. `serve::query` put the
    /// key on the wire, and Suite 0's first real differential caught it — the
    /// C++ engine answers `applications:host--byobu` for the item this side
    /// called `host--byobu.desktop`, so every ranked hit read as a
    /// regression.
    pub entrypoint_id: &'a str,
}

impl AppIndex {
    /// Sets what the window manager can do: root search offers Switch
    /// Workspaces and the toggles only where the C++
    /// `WindowManagementExtension` registers them.
    pub fn set_window_capabilities(&mut self, caps: crate::window_switcher::Capabilities) {
        self.window_capabilities = caps;
    }

    /// What [`Self::set_window_capabilities`] last set.
    #[must_use]
    pub const fn window_capabilities(&self) -> crate::window_switcher::Capabilities {
        self.window_capabilities
    }

    /// Applies user settings without changing catalog positions or launch keys.
    pub fn apply_root_config(&mut self, config: &crate::root_items::RootConfig) {
        self.root_config = config.clone();
        for root in &mut self.roots {
            // Application defaults have no alias or shortcut. Reset them so a
            // removed setting cannot survive a subsequent configuration merge.
            root.meta.alias = None;
            root.meta.shortcut = None;
            let default_disabled = crate::commands::by_id(&root.id)
                .is_some_and(crate::commands::BuiltinCommand::default_disabled);
            root.merge_config(config, default_disabled);
        }
    }

    /// Search application root rows using the root manager's fields and ordering.
    ///
    /// Actions belong in the owning application's panel. An unresolved TryExec
    /// does not hide a root row: a sandbox may not see an executable on the host.
    /// The index still reports that diagnostic through `AppItem::launchable`.
    #[must_use]
    pub fn search_root(
        &self,
        pattern: &str,
        history: Option<&dyn crate::FrecencyStore>,
    ) -> Vec<ApplicationRootHit<'_>> {
        let now = history.map_or(0, crate::FrecencyStore::now);
        let frecency = |index: usize, _: &crate::root_items::RootItem| {
            history
                .and_then(|store| store.record(self.items[self.root_indices[index]].key()))
                .map_or(0.0, |record| record.score_at(now))
        };
        // Applications only: builtin commands share `roots` (see `build`) and
        // are reached through `search_root_all`. The provider filter runs
        // before frecency is asked for, so `frecency` never sees a command.
        let options = crate::root_items::SearchOptions {
            provider_id: Some(crate::root_items::APPS_PROVIDER_ID.to_owned()),
            ..crate::root_items::SearchOptions::default()
        };
        crate::root_items::search_with_frecency(&self.roots, pattern, &options, frecency)
            .into_iter()
            .map(|hit| {
                let index = self.root_indices[hit.index];
                let item = &self.items[index];
                let match_score = if pattern.trim().is_empty() {
                    0
                } else {
                    (hit.score - compass_search::FRECENCY_WEIGHT * frecency(hit.index, hit.item))
                        .round()
                        .clamp(0.0, 100.0) as u32
                };
                ApplicationRootHit {
                    item,
                    index,
                    entrypoint_id: &self.roots[hit.index].id,
                    match_score,
                }
            })
            .collect()
    }

    /// Search applications **and** builtin commands, ranked together.
    ///
    /// One scoring pass over both, so a command competes with applications on
    /// the same fuzzy score, typo tolerance and frecency. A command's frecency
    /// key is its `commands:<entrypoint>` id.
    #[must_use]
    pub fn search_root_all(
        &self,
        pattern: &str,
        history: Option<&dyn crate::FrecencyStore>,
    ) -> Vec<RootHit<'_>> {
        self.search_root_with(
            pattern,
            history,
            &crate::root_items::SearchOptions::default(),
        )
    }

    /// Whether any root item, enabled or not, comes from the provider `id`
    /// (`findProviderById` for the providers that have items).
    #[must_use]
    pub fn has_provider(&self, id: &str) -> bool {
        self.roots.iter().any(|root| root.meta.provider_id == id)
    }

    /// A provider's display name: the one each C++ root provider gives
    /// itself, and an extension's title for an extension. `None` for a
    /// provider no root item comes from.
    #[must_use]
    pub fn provider_title(&self, id: &str) -> Option<String> {
        if !self.has_provider(id) {
            return None;
        }
        Some(match id {
            crate::root_items::APPS_PROVIDER_ID => "Applications".to_owned(),
            crate::shortcut::SHORTCUTS_PROVIDER_ID => "Shortcuts".to_owned(),
            crate::script_scan::SCRIPTS_PROVIDER_ID => "Script Commands".to_owned(),
            crate::rhai_scripts::RHAI_PROVIDER_ID => "Rhai Scripts".to_owned(),
            crate::commands::COMMANDS_PROVIDER_ID => "Commands".to_owned(),
            _ => self
                .extensions
                .iter()
                .find(|command| command.provider_id == id)
                .map_or_else(|| id.to_owned(), |command| command.extension_title.clone()),
        })
    }

    /// [`Self::search_root_all`] with the search's options: the provider
    /// search view's (`providerId`) among them.
    #[must_use]
    pub fn search_root_with(
        &self,
        pattern: &str,
        history: Option<&dyn crate::FrecencyStore>,
        options: &crate::root_items::SearchOptions,
    ) -> Vec<RootHit<'_>> {
        let now = history.map_or(0, crate::FrecencyStore::now);
        let key = |index: usize| -> &str {
            match self.root_indices.get(index) {
                Some(&item) => self.items[item].key(),
                None => &self.roots[index].id,
            }
        };
        let frecency = |index: usize, _: &crate::root_items::RootItem| {
            history
                .and_then(|store| store.record(key(index)))
                .map_or(0.0, |record| record.score_at(now))
        };
        crate::root_items::search_with_frecency(&self.roots, pattern, options, frecency)
            .into_iter()
            .filter_map(|hit| {
                let match_score = if pattern.trim().is_empty() {
                    0
                } else {
                    (hit.score - compass_search::FRECENCY_WEIGHT * frecency(hit.index, hit.item))
                        .round()
                        .clamp(0.0, 100.0) as u32
                };
                let entrypoint_id = &self.roots[hit.index].id;
                match self.root_indices.get(hit.index) {
                    Some(&index) => Some(RootHit::App(ApplicationRootHit {
                        item: &self.items[index],
                        index,
                        entrypoint_id,
                        match_score,
                    })),
                    None => crate::commands::by_id(entrypoint_id)
                        .filter(|command| {
                            crate::window_switcher::command_offered(
                                command.kind,
                                self.window_capabilities,
                            )
                        })
                        .map(|command| RootHit::Command {
                            command,
                            match_score,
                        })
                        .or_else(|| {
                            self.extension(entrypoint_id)
                                .map(|command| RootHit::Extension {
                                    command,
                                    match_score,
                                })
                        })
                        .or_else(|| {
                            self.shortcut_by_entrypoint(entrypoint_id).map(|shortcut| {
                                RootHit::Shortcut {
                                    shortcut,
                                    match_score,
                                }
                            })
                        })
                        .or_else(|| {
                            self.script_by_entrypoint(entrypoint_id)
                                .map(|script| RootHit::Script {
                                    script,
                                    match_score,
                                })
                        })
                        .or_else(|| {
                            self.rhai_script_by_entrypoint(entrypoint_id).map(|script| {
                                RootHit::RhaiScript {
                                    script,
                                    match_score,
                                }
                            })
                        }),
                }
            })
            .collect()
    }

    /// The root row an entrypoint id names, with the metadata the
    /// configuration last applied gave it (alias, favourite, enabled).
    #[must_use]
    pub fn root(&self, entrypoint_id: &str) -> Option<&crate::root_items::RootItem> {
        self.roots.iter().find(|root| root.id == entrypoint_id)
    }

    /// The key an entrypoint's launches are recorded under: an
    /// application's desktop key, anything else's own id.
    #[must_use]
    pub fn history_key(&self, entrypoint_id: &str) -> String {
        self.position_by_entrypoint(entrypoint_id).map_or_else(
            || entrypoint_id.to_owned(),
            |position| self.items[position].key().to_owned(),
        )
    }

    /// Every root item: applications, builtin commands, extension commands,
    /// shortcuts and scripts, disabled ones included, in index order.
    #[must_use]
    pub fn roots(&self) -> &[crate::root_items::RootItem] {
        &self.roots
    }

    /// Installed extensions' commands, in the registry's precedence order.
    #[must_use]
    pub fn extensions(&self) -> &[crate::extension_commands::ExtensionCommand] {
        &self.extensions
    }

    /// The installed command a `raycast://extensions/<owner>/<name>/<command>`
    /// link names: by the extension's manifest name, its owner or author, and
    /// the command's name.
    #[must_use]
    pub fn extension_by_link(
        &self,
        owner_or_author: &str,
        extension_name: &str,
        command: &str,
    ) -> Option<&crate::extension_commands::ExtensionCommand> {
        self.extensions.iter().find(|candidate| {
            candidate.extension_name == extension_name
                && candidate.name == command
                && candidate.is_by(owner_or_author)
        })
    }

    /// The quicklinks root search lists, in the store's order.
    #[must_use]
    pub fn shortcuts(&self) -> &[crate::shortcut_service::CachedShortcut] {
        &self.shortcuts
    }

    /// The quicklink a `shortcuts:<id>` entrypoint id names.
    #[must_use]
    pub fn shortcut_by_entrypoint(
        &self,
        entrypoint_id: &str,
    ) -> Option<&crate::shortcut_service::CachedShortcut> {
        let (provider, id) = crate::root_items::split_entrypoint_id(entrypoint_id)?;
        if provider != crate::shortcut::SHORTCUTS_PROVIDER_ID {
            return None;
        }
        self.shortcuts.iter().find(|shortcut| shortcut.id == id)
    }

    /// Replaces the quicklinks root search lists, applying the configuration
    /// last given to [`AppIndex::apply_root_config`] to their rows, so an
    /// alias or a disabled flag set on one survives the list changing.
    ///
    /// Quicklinks change while the launcher runs, which is why they are not
    /// part of [`AppIndexBuilder::build`]: their rows are dropped and rebuilt
    /// here, after every other root, so no other row's position moves.
    pub fn set_shortcuts(&mut self, shortcuts: Vec<crate::shortcut_service::CachedShortcut>) {
        let roots = shortcuts
            .iter()
            .map(|shortcut| {
                crate::shortcut::root_item(&shortcut.id, &shortcut.name, &shortcut.link.raw)
            })
            .collect();
        self.replace_provider_roots(crate::shortcut::SHORTCUTS_PROVIDER_ID, roots);
        self.shortcuts = shortcuts;
    }

    /// The script commands root search lists, in scan order.
    #[must_use]
    pub fn scripts(&self) -> &[crate::script_scan::ScriptItem] {
        &self.scripts
    }

    /// The script a `scripts:<id>` entrypoint id names.
    #[must_use]
    pub fn script_by_entrypoint(
        &self,
        entrypoint_id: &str,
    ) -> Option<&crate::script_scan::ScriptItem> {
        let (provider, id) = crate::root_items::split_entrypoint_id(entrypoint_id)?;
        if provider != crate::script_scan::SCRIPTS_PROVIDER_ID {
            return None;
        }
        self.scripts.iter().find(|script| script.id == id)
    }

    /// Replaces the script commands root search lists, as
    /// [`AppIndex::set_shortcuts`] replaces the quicklinks.
    pub fn set_scripts(&mut self, scripts: Vec<crate::script_scan::ScriptItem>) {
        let roots = scripts
            .iter()
            .map(crate::script_scan::ScriptItem::root_item)
            .collect();
        self.replace_provider_roots(crate::script_scan::SCRIPTS_PROVIDER_ID, roots);
        self.scripts = scripts;
    }

    /// The Rhai scripts root search lists, in id order.
    #[must_use]
    pub fn rhai_scripts(&self) -> &[crate::rhai_scripts::RhaiScriptItem] {
        &self.rhai_scripts
    }

    /// The Rhai script a `rhai:<id>` entrypoint id names.
    #[must_use]
    pub fn rhai_script_by_entrypoint(
        &self,
        entrypoint_id: &str,
    ) -> Option<&crate::rhai_scripts::RhaiScriptItem> {
        let id = crate::rhai_scripts::script_id(entrypoint_id)?;
        self.rhai_scripts.iter().find(|script| script.id == id)
    }

    /// Replaces the Rhai scripts root search lists, as
    /// [`AppIndex::set_shortcuts`] replaces the quicklinks: scripts come and
    /// go while the launcher runs (hot reload).
    pub fn set_rhai_scripts(&mut self, scripts: Vec<crate::rhai_scripts::RhaiScriptItem>) {
        let roots = scripts
            .iter()
            .map(crate::rhai_scripts::RhaiScriptItem::root_item)
            .collect();
        self.replace_provider_roots(crate::rhai_scripts::RHAI_PROVIDER_ID, roots);
        self.rhai_scripts = scripts;
    }

    /// Drops `provider`'s roots and appends `roots` in their place, merged
    /// with the configuration last applied.
    fn replace_provider_roots(&mut self, provider: &str, roots: Vec<crate::root_items::RootItem>) {
        self.roots.retain(|root| root.meta.provider_id != provider);
        for mut root in roots {
            root.merge_config(&self.root_config, false);
            self.roots.push(root);
        }
    }

    /// The directories applications are scanned from, highest precedence
    /// first: what `AppService` watches (`reinstallWatches(searchPaths())`).
    #[must_use]
    pub fn application_dirs(&self) -> &[PathBuf] {
        &self.scan.dirs
    }

    /// A builder that scans the applications again exactly as this index
    /// was scanned, without the extensions. Built off the lock and handed to
    /// [`AppIndex::replace_applications`], so a rescan never holds up a query.
    #[must_use]
    pub fn application_scan(&self) -> AppIndexBuilder {
        self.scan.clone()
    }

    /// Scans the application directories again and takes what is installed
    /// now, as `AppService::scanSync` does after a directory changed.
    pub fn rescan_applications(&mut self) {
        let fresh = self.application_scan().build();
        self.replace_applications(fresh);
    }

    /// Takes `fresh`'s applications in place of this index's, keeping every
    /// other root (commands, extensions, shortcuts, scripts) and the
    /// configuration last applied, which the new application rows get too:
    /// an alias or a disabled flag survives an application being reinstalled.
    pub fn replace_applications(&mut self, fresh: AppIndex) {
        let mut roots: Vec<crate::root_items::RootItem> = fresh
            .roots
            .into_iter()
            .take(fresh.root_indices.len())
            .collect();
        for root in &mut roots {
            root.merge_config(&self.root_config, false);
        }
        roots.extend(
            self.roots
                .drain(self.root_indices.len().min(self.roots.len())..),
        );
        self.roots = roots;
        self.root_indices = fresh.root_indices;
        self.items = fresh.items;
        self.by_key = fresh.by_key;
        self.skipped = fresh.skipped;
        self.hidden = fresh.hidden;
    }

    /// Installed applications the root does not show: `NoDisplay`, or
    /// meant for another desktop. What `displayable()` is false for, and
    /// what Browse Apps' `showHidden` adds.
    #[must_use]
    pub fn hidden_applications(&self) -> &[AppItem] {
        &self.hidden
    }

    /// Where installed extensions are looked for, highest precedence first:
    /// what the registry watches.
    #[must_use]
    pub fn extension_dirs(&self) -> &[PathBuf] {
        &self.extension_dirs
    }

    /// Scans the extension directories the index was built with again and
    /// takes what is installed now, as `ExtensionRegistry::requestScan`
    /// does after an install or an uninstall.
    pub fn rescan_extensions(&mut self) {
        let extensions = if self.extension_dirs.is_empty() {
            Vec::new()
        } else {
            crate::extension_commands::ExtensionCommand::from_manifests(
                &crate::manifest::registry::scan(&self.extension_dirs).extensions,
            )
        };
        self.set_extensions(extensions);
    }

    /// Replaces the installed extensions' commands, applying the
    /// configuration last given to [`AppIndex::apply_root_config`] to their
    /// rows. Every other row keeps its position.
    pub fn set_extensions(&mut self, extensions: Vec<crate::extension_commands::ExtensionCommand>) {
        self.all_extensions = extensions.clone();
        let old: std::collections::HashSet<&str> = self
            .extensions
            .iter()
            .map(|command| command.id.as_str())
            .collect();
        let first_non_app = self.root_indices.len();
        let mut position = 0;
        self.roots.retain(|root| {
            let keep = position < first_non_app || !old.contains(root.id.as_str());
            position += 1;
            keep
        });
        for command in &extensions {
            if self
                .deny
                .denies(&crate::extension_deny::ExtensionIdentity::from_command(
                    command,
                ))
                .is_some()
            {
                continue;
            }
            let mut root = command.root_item();
            root.merge_config(&self.root_config, false);
            self.roots.push(root);
        }
        self.extensions = extensions
            .into_iter()
            .filter(|command| {
                self.deny
                    .denies(&crate::extension_deny::ExtensionIdentity::from_command(
                        command,
                    ))
                    .is_none()
            })
            .collect();
    }

    /// Applies a deny policy to the installed extensions and their roots.
    /// Called at startup and on config reload; rescans keep the same policy.
    pub fn set_deny(&mut self, deny: crate::extension_deny::ExtensionDeny) {
        self.deny = deny;
        let scanned = self.all_extensions.clone();
        self.set_extensions(scanned);
    }

    /// The installed extension command with this entrypoint id.
    #[must_use]
    pub fn extension(&self, id: &str) -> Option<&crate::extension_commands::ExtensionCommand> {
        self.extensions.iter().find(|command| command.id == id)
    }

    /// Starts building an index. See [`AppIndexBuilder`].
    #[must_use]
    pub fn builder() -> AppIndexBuilder {
        AppIndexBuilder::new()
    }

    /// Builds an index of the current user's applications from the environment.
    #[must_use]
    pub fn from_environment() -> AppIndex {
        AppIndexBuilder::from_environment().build()
    }

    /// Every indexed item: applications and actions, in scan order.
    #[must_use]
    pub fn items(&self) -> &[AppItem] {
        &self.items
    }

    /// Number of indexed items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether nothing was indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The item with this [`AppItem::key`].
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&AppItem> {
        self.by_key.get(key).map(|&index| &self.items[index])
    }

    /// Catalog position of a stable key, valid until this index is replaced.
    #[must_use]
    pub fn position(&self, key: &str) -> Option<usize> {
        self.by_key.get(key).copied()
    }

    /// The position of the item a `provider:entrypoint` id names.
    ///
    /// The protocol identifies a hit by its ENTRYPOINT id
    /// (`applications:org.mozilla.firefox`), which is not the launch key
    /// (`org.mozilla.firefox.desktop`) — so a client that reads `QueryHit.id`
    /// cannot look the item up with [`AppIndex::position`]. Both lookups
    /// exist because both ids are real and neither is a formatting variant of
    /// the other: `RecordLaunch` takes the key, `QueryHit` carries the
    /// entrypoint id.
    #[must_use]
    pub fn position_by_entrypoint(&self, entrypoint_id: &str) -> Option<usize> {
        let root = self
            .roots
            .iter()
            .position(|root| root.id == entrypoint_id)?;
        self.root_indices.get(root).copied()
    }

    /// Only the items that can actually be launched. See
    /// [`AppIndexBuilder::include_unlaunchable`].
    pub fn launchable_items(&self) -> impl Iterator<Item = &AppItem> {
        self.items.iter().filter(|item| item.launchable())
    }

    /// Only the application items, without their actions.
    pub fn applications(&self) -> impl Iterator<Item = &AppItem> {
        self.items.iter().filter(|item| !item.is_action())
    }

    /// Files the scanner looked at and did not index, with the reason.
    ///
    /// This is what `compass doctor` should print when a user asks why their application is
    /// missing.
    #[must_use]
    pub fn skipped(&self) -> &[SkippedEntry] {
        &self.skipped
    }

    /// Ranks the index against `query`, best first, ignoring launch history.
    ///
    /// An empty or whitespace-only query returns every item in index order.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<compass_search::Scored<&AppItem>> {
        compass_search::rank(query, &self.items)
    }

    /// [`AppIndex::search`] with a query parsed once and reused across calls.
    #[must_use]
    pub fn search_with_query(
        &self,
        query: &compass_search::Query,
    ) -> Vec<compass_search::Scored<&AppItem>> {
        compass_search::rank_with_query(query, &self.items)
    }

    /// [`AppIndex::search`] with an explicit quality threshold.
    #[must_use]
    pub fn search_with_options(
        &self,
        query: &str,
        options: compass_search::RankOptions,
    ) -> Vec<compass_search::Scored<&AppItem>> {
        compass_search::rank_with_options(query, &self.items, options)
    }

    /// [`AppIndex::search_with_options`] with a pre-parsed query.
    #[must_use]
    pub fn search_with_query_and_options(
        &self,
        query: &compass_search::Query,
        options: compass_search::RankOptions,
    ) -> Vec<compass_search::Scored<&AppItem>> {
        compass_search::rank_with_query_and_options(query, &self.items, options)
    }
}
