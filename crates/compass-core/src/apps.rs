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
//! * `Type=Application` entries only. `Link` and `Directory` entries are not launchable
//!   applications and belong to other providers.
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
use compass_xdg::{DesktopAction, DesktopEntry, Locale, ParseOptions};

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

/// Maximum directory depth the scanner descends to, as a cycle guard for symlinked trees.
const MAX_SCAN_DEPTH: usize = 8;

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
    /// The entry is not `Type=Application`.
    NotAnApplication,
    /// `Hidden`, `NoDisplay`, or excluded by `OnlyShowIn`/`NotShowIn`.
    NotShown,
    /// `TryExec` did not resolve and unlaunchable entries were excluded.
    TryExecMissing(String),
    /// A `Type=Application` entry with no `Exec` key: there is nothing to launch.
    NoExec,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::Shadowed { id, winner } => {
                write!(f, "shadowed: {id} already provided by {}", winner.display())
            }
            SkipReason::Unreadable(err) => write!(f, "unreadable: {err}"),
            SkipReason::Malformed(err) => write!(f, "malformed: {err}"),
            SkipReason::NotAnApplication => f.write_str("not Type=Application"),
            SkipReason::NotShown => f.write_str("hidden in this environment"),
            SkipReason::TryExecMissing(exec) => write!(f, "TryExec {exec} did not resolve"),
            SkipReason::NoExec => f.write_str("no Exec key"),
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
        let mut items: Vec<AppItem> = Vec::new();
        let mut skipped: Vec<SkippedEntry> = Vec::new();
        // Desktop file id -> the file that claimed it. Claiming happens before any visibility
        // check, so a higher-precedence Hidden entry really does delete the lower-precedence one.
        let mut claimed: HashMap<String, PathBuf> = HashMap::new();
        let mut by_key: HashMap<String, usize> = HashMap::new();

        let desktops: Vec<&str> = self.desktops.iter().map(String::as_str).collect();

        for dir in &self.dirs {
            let mut files = Vec::new();
            collect_desktop_files(dir, dir, 0, &mut files, &mut skipped);
            files.sort_by(|a, b| a.0.cmp(&b.0));

            for (id, path) in files {
                if let Some(winner) = claimed.get(&id) {
                    skipped.push(SkippedEntry {
                        path,
                        reason: SkipReason::Shadowed {
                            id,
                            winner: winner.clone(),
                        },
                    });
                    continue;
                }
                claimed.insert(id.clone(), path.clone());

                self.index_file(&id, &path, &desktops, &mut items, &mut by_key, &mut skipped);
            }
        }

        AppIndex {
            items,
            by_key,
            skipped,
        }
    }

    fn index_file(
        &self,
        id: &str,
        path: &Path,
        desktops: &[&str],
        items: &mut Vec<AppItem>,
        by_key: &mut HashMap<String, usize>,
        skipped: &mut Vec<SkippedEntry>,
    ) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::debug!(path = %path.display(), %err, "could not read desktop entry");
                skipped.push(SkippedEntry {
                    path: path.to_path_buf(),
                    reason: SkipReason::Unreadable(err.to_string()),
                });
                return;
            }
        };

        // Read lossily rather than strictly: `non-utf8.desktop` exists in the wild (latin-1
        // Comment lines are common in old entries) and losing the whole application over one bad
        // byte in a tooltip is not a trade worth making.
        let text = String::from_utf8_lossy(&bytes);

        let opts = ParseOptions {
            id: Some(id.to_owned()),
            locale: self.locale.clone(),
            path: Some(path.to_path_buf()),
        };

        let entry = match DesktopEntry::parse_with(&text, &opts) {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(path = %path.display(), %err, "malformed desktop entry");
                skipped.push(SkippedEntry {
                    path: path.to_path_buf(),
                    reason: SkipReason::Malformed(err.to_string()),
                });
                return;
            }
        };

        if !entry.is_application() {
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
            return;
        }

        if entry.exec().is_none() {
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

        if !self.include_actions {
            return;
        }

        for (index, action) in entry.actions().iter().enumerate() {
            // An action with no name has nothing to search for, and one with no Exec has nothing
            // to do; the spec requires both, so a missing one means a broken entry.
            if action.name().is_empty() || action.exec().is_none() {
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
                    name: action.name().to_owned(),
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

/// Recursively collects `(desktop file id, path)` pairs under `dir`.
fn collect_desktop_files(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<(String, PathBuf)>,
    skipped: &mut Vec<SkippedEntry>,
) {
    if depth > MAX_SCAN_DEPTH {
        tracing::warn!(dir = %dir.display(), "application directory nested too deeply, not descending");
        return;
    }

    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(err) => {
            // A missing $XDG_DATA_DIRS entry is completely normal, so this is not a diagnostic
            // worth surfacing unless the directory exists but is unreadable.
            if err.kind() != std::io::ErrorKind::NotFound {
                skipped.push(SkippedEntry {
                    path: dir.to_path_buf(),
                    reason: SkipReason::Unreadable(err.to_string()),
                });
            }
            return;
        }
    };

    for entry in read.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        // `is_dir` follows symlinks, which is what we want: distributions symlink application
        // directories around. MAX_SCAN_DEPTH is the loop guard.
        if file_type.is_dir() || (file_type.is_symlink() && path.is_dir()) {
            collect_desktop_files(root, &path, depth + 1, out, skipped);
            continue;
        }

        if path.extension().is_none_or(|ext| ext != "desktop") {
            continue;
        }

        if let Some(id) = desktop_file_id(root, &path) {
            out.push((id, path));
        }
    }
}

/// The desktop file id of `path` relative to the applications directory `root`.
///
/// Returns `None` for a path outside `root` or one that is not valid Unicode; an id that cannot be
/// spelled cannot be matched against `Actions`, `DBusActivatable` or the MIME cache anyway.
#[must_use]
pub fn desktop_file_id(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut id = String::new();
    for component in relative.components() {
        let part = component.as_os_str().to_str()?;
        if !id.is_empty() {
            id.push('-');
        }
        id.push_str(part);
    }
    (!id.is_empty()).then_some(id)
}

/// A built, searchable index of applications and their actions.
#[derive(Debug, Clone, Default)]
pub struct AppIndex {
    items: Vec<AppItem>,
    by_key: HashMap<String, usize>,
    skipped: Vec<SkippedEntry>,
}

impl AppIndex {
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
    /// This is what `vicinae doctor` should print when a user asks why their application is
    /// missing.
    #[must_use]
    pub fn skipped(&self) -> &[SkippedEntry] {
        &self.skipped
    }

    /// Ranks the index against `query`, best first, ignoring launch history.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<compass_search::Scored<&AppItem>> {
        compass_search::rank(query, &self.items)
    }
}
