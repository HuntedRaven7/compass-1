//! Walking a directory tree the way the file indexer walks it, on the `ignore` crate.
//!
//! Ports `FileSystemWalker`. The traversal machinery — reading directories,
//! matching ignore files, pruning hidden trees — is the `ignore` crate's, the
//! same engine ripgrep walks with. What is the launcher's own sits in
//! [`IndexWalk`]'s policy: the curated exclusion lists, the `.noindex`
//! convention, machine trash directories, `CACHEDIR.TAG` abandonment, and
//! refusing symlinks outright.
//!
//! # Deltas from the C++, on purpose
//!
//! * Ignore files mean what git means now: patterns are scoped to the
//!   directory holding the ignore file, `**` works, and `!` negations are
//!   honoured. The C++ matched only the filename with `fnmatch`, which its own
//!   comment calls a hack that mishandles separators — so `build/*.o` never
//!   matched, and `/target` matched a `target` anywhere. Those two behaviours
//!   change here; everything else an ignore file did still holds.
//! * Custom ignore filenames ride the crate's custom-ignore support, with the
//!   crate's precedence (custom names outrank `.gitignore`/`.ignore`).
//! * Build output beside its manifest (`target` next to `Cargo.toml`, …) is
//!   never walked, and an excluded directory is not reported merely because
//!   it holds a `CACHEDIR.TAG` (#320).

use std::collections::HashSet;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use ignore::{DirEntry, WalkBuilder, gitignore::GitignoreBuilder};

use crate::entry_filter::{
    CACHEDIR_TAG_SIGNATURE, EXCLUDED_FILENAMES, NOINDEX_SUFFIX, basename, excluded_paths,
    is_build_output_directory, is_cachedir_tag, is_hidden_path, is_machine_trash_directory,
};

/// One entry as the walk sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    /// Where it is.
    pub path: PathBuf,
    /// Whether it is a directory.
    pub is_directory: bool,
    /// Whether it is a symlink. The filter refuses these outright, so a walk
    /// never reports one — the field records the refusal, not an outcome.
    pub is_symlink: bool,
}

impl WalkEntry {
    /// A plain file.
    #[must_use]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            is_directory: false,
            is_symlink: false,
        }
    }

    /// A directory.
    #[must_use]
    pub fn directory(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            is_directory: true,
            is_symlink: false,
        }
    }
}

/// How the indexer walks a tree, and what it refuses to look at.
#[derive(Debug)]
pub struct IndexWalk {
    /// Extra paths to exclude, from the configuration.
    excluded_paths: Vec<PathBuf>,
    /// Extra filenames to exclude.
    excluded_filenames: Vec<String>,
    /// The ignore files to honour, such as `.gitignore`. Empty means none are
    /// read at all, which is also the C++ default — nothing configures a list
    /// today.
    ignore_files: Vec<String>,
    /// Whether dotted paths are skipped.
    ignore_hidden_paths: bool,
    /// The built-in excluded paths, resolved against a home directory.
    builtin_excluded: Vec<PathBuf>,
    /// Whether directories are descended into at all.
    recursive: bool,
    /// How deep to descend, in components below the root.
    max_depth: Option<usize>,
    /// Cache directories met during a walk, whose contents are abandoned
    /// while the directories themselves are still reported. Parents always
    /// filter before their children in a serial walk, so recording them here
    /// prunes whole subtrees regardless of listing order. A mutex rather than
    /// a cell because the crate shares the hook across threads.
    cache_dirs: Mutex<HashSet<PathBuf>>,
    /// Whether a walk in progress should give up. Shared across clones so an
    /// interrupt reaches the walk however the configuration got there; the
    /// C++ walker is not copyable at all, this is the closest shape.
    stopped: Arc<AtomicBool>,
}

impl Clone for IndexWalk {
    fn clone(&self) -> Self {
        Self {
            excluded_paths: self.excluded_paths.clone(),
            excluded_filenames: self.excluded_filenames.clone(),
            ignore_files: self.ignore_files.clone(),
            ignore_hidden_paths: self.ignore_hidden_paths,
            builtin_excluded: self.builtin_excluded.clone(),
            recursive: self.recursive,
            max_depth: self.max_depth,
            // A walk's abandoned directories belong to that walk, not to the
            // configuration: a clone starts recording from nothing. An
            // interrupt is not configuration either, but dropping it on clone
            // would strand a walk started from a copy: the flag is shared.
            cache_dirs: Mutex::new(HashSet::new()),
            stopped: Arc::clone(&self.stopped),
        }
    }
}

impl IndexWalk {
    /// A recursive walk with no depth limit, filtering as the indexer does.
    ///
    /// The built-in exclusions resolve under `home`.
    #[must_use]
    pub fn new(home: Option<&Path>) -> Self {
        Self {
            excluded_paths: Vec::new(),
            excluded_filenames: Vec::new(),
            ignore_files: Vec::new(),
            ignore_hidden_paths: false,
            builtin_excluded: excluded_paths(home),
            recursive: true,
            max_depth: None,
            cache_dirs: Mutex::new(HashSet::new()),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set the configured excluded paths.
    pub fn set_excluded_paths(&mut self, paths: Vec<PathBuf>) {
        self.excluded_paths = paths;
    }

    /// Set the configured excluded filenames.
    pub fn set_excluded_filenames(&mut self, filenames: Vec<String>) {
        self.excluded_filenames = filenames;
    }

    /// Set which ignore files to read.
    pub fn set_ignore_files(&mut self, files: Vec<String>) {
        self.ignore_files = files;
    }

    /// Set whether dotted paths are skipped.
    pub fn set_ignore_hidden_paths(&mut self, value: bool) {
        self.ignore_hidden_paths = value;
    }

    /// The ignore files being honoured.
    #[must_use]
    pub fn ignore_files(&self) -> &[String] {
        &self.ignore_files
    }

    /// Whether `path` is one of the configured excluded paths.
    #[must_use]
    pub fn is_excluded_path(&self, path: &Path) -> bool {
        self.excluded_paths.iter().any(|excluded| excluded == path)
    }

    /// Whether directories are descended into at all.
    #[must_use]
    pub fn recursive(mut self, value: bool) -> Self {
        self.recursive = value;
        self
    }

    /// How deep to descend, in components below the root.
    ///
    /// Like the C++ (`depth <= maxDepth`), this bounds what is *entered*: an
    /// entry one level deeper than the limit is still reported by its parent's
    /// listing. The crate yields directories at its limit without descending,
    /// so the limit maps across plus one.
    #[must_use]
    pub fn max_depth(mut self, value: Option<usize>) -> Self {
        self.max_depth = value;
        self
    }

    /// Whether the indexer should walk into something whose kind is already
    /// known.
    ///
    /// The breadth-first watch enumeration walks directories itself, so it
    /// asks here instead of running the whole walker. This is the C++'s
    /// `shouldVisit`: hidden paths, the exclusion lists, trash directories,
    /// ignore files — plus build output beside its manifest — and, like the
    /// C++, no `CACHEDIR.TAG` check, which lives in the walker's listing loop
    /// rather than the filter.
    ///
    /// Ignore files are matched with the crate's engine, nearest ignore file
    /// first — a nearer whitelist overrules a farther ignore, the way git
    /// stacks them.
    #[must_use]
    pub fn should_visit(&self, path: &Path, is_symlink: bool, is_directory: bool) -> bool {
        if self.ignore_hidden_paths && is_hidden_path(path) {
            return false;
        }
        self.policy(path, is_symlink, is_directory) && !self.is_ignored(path, is_directory)
    }

    /// The policy that needs no ignore files, in the C++'s order: the cheap
    /// string checks first, the ones that touch the disk last.
    ///
    /// Only the build-output rule touches the disk, and only for a directory
    /// named like build output. Hidden paths are not checked here —
    /// in the walker the crate prunes those before this runs (by entry name,
    /// so a hidden-named root still walks), and `should_visit` checks them
    /// itself for its own enumeration — and neither is `CACHEDIR.TAG`, which
    /// the walker's hook handles so the directory is still reported.
    fn policy(&self, path: &Path, is_symlink: bool, is_directory: bool) -> bool {
        if is_symlink {
            return false;
        }
        if self
            .builtin_excluded
            .iter()
            .any(|excluded| excluded == path)
        {
            return false;
        }

        let lossy = path.to_string_lossy();
        let filename = basename(&lossy);

        if EXCLUDED_FILENAMES.contains(&filename) || filename.ends_with(NOINDEX_SUFFIX) {
            return false;
        }
        if self
            .excluded_filenames
            .iter()
            .any(|excluded| excluded == filename)
        {
            return false;
        }
        if is_directory && is_machine_trash_directory(path) {
            return false;
        }
        if self.is_excluded_path(path) {
            return false;
        }
        if is_directory && is_build_output_directory(path) {
            return false;
        }

        true
    }

    /// Whether an ignore file anywhere above `path` matches it.
    ///
    /// Consulted last and only when ignore files are configured: the walker's
    /// crate matching already covers its own traversal, so this exists for
    /// callers that enumerate directories themselves.
    fn is_ignored(&self, path: &Path, is_directory: bool) -> bool {
        if self.ignore_files.is_empty() {
            return false;
        }
        let mut ancestor = path.parent();
        while let Some(dir) = ancestor {
            let mut decided = None;
            for name in &self.ignore_files {
                let file = dir.join(name);
                if !file.is_file() {
                    continue;
                }
                let mut builder = GitignoreBuilder::new(dir);
                if builder.add(&file).is_some() {
                    continue;
                }
                let Ok(matcher) = builder.build() else {
                    continue;
                };
                let matched = matcher.matched(path, is_directory);
                if matched.is_ignore() {
                    decided = Some(true);
                    break;
                }
                if matched.is_whitelist() {
                    decided = Some(false);
                    break;
                }
            }
            if let Some(ignored) = decided {
                return ignored;
            }
            if dir == Path::new("/") || dir.as_os_str().is_empty() {
                break;
            }
            ancestor = dir.parent();
        }

        false
    }

    /// The policy as the crate sees it. Ignore files and hidden trees never
    /// reach here — the builder prunes those before `filter_entry` runs — so
    /// this is the launcher's rules, plus the cache-directory abandonment the C++
    /// does in its listing loop.
    ///
    /// A cache directory is still reported: only its contents are abandoned.
    /// The crate cannot yield-without-descending through this hook, so the
    /// directory is recorded when met and everything beneath it refused —
    /// which prunes the whole subtree whatever order the entries list in.
    /// The policy runs first, as the C++ only lists directories that passed
    /// `shouldVisit`: an excluded cache directory is not reported either.
    fn filter_entry(&self, entry: &DirEntry) -> bool {
        let file_type = entry.file_type();
        let path = entry.path();
        let is_directory = file_type.is_some_and(|kind| kind.is_dir());
        if self
            .cache_dirs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|dir| path.starts_with(dir))
        {
            return false;
        }
        if !self.policy(
            path,
            file_type.is_some_and(|kind| kind.is_symlink()),
            is_directory,
        ) {
            return false;
        }
        if is_directory && is_cache_directory(path) {
            self.cache_dirs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(path.to_path_buf());
        }
        true
    }

    /// Stops a walk in progress: the loop breaks at the next entry, the way
    /// `FileSystemWalker::stop` breaks the C++ walk. A walk that has not
    /// started yet visits nothing. Takes `&self` so a scanner interrupting
    /// from another thread needs no mutable borrow.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// A shareable [`IndexWalk::stop`] over the same flag, for stopping a
    /// walk the caller does not own.
    pub fn stop_handle(&self) -> Arc<dyn Fn() + Send + Sync> {
        let stopped = Arc::clone(&self.stopped);
        Arc::new(move || stopped.store(true, Ordering::SeqCst))
    }

    /// Walks `root`, calling `visit` for every entry that passes the filter.
    ///
    /// The root itself is never visited: the walk reports what is *in* a tree,
    /// and a caller that wanted the root already has it. A root that is not a
    /// directory, or cannot be read, yields nothing rather than one entry.
    pub fn walk(&self, root: &Path, mut visit: impl FnMut(&WalkEntry)) {
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(self.ignore_hidden_paths)
            .parents(true)
            .require_git(false)
            .git_global(false)
            .git_exclude(false)
            .git_ignore(self.ignore_files.iter().any(|name| name == ".gitignore"))
            .ignore(self.ignore_files.iter().any(|name| name == ".ignore"));
        for name in &self.ignore_files {
            if name != ".gitignore" && name != ".ignore" {
                builder.add_custom_ignore_filename(name);
            }
        }
        if !self.recursive {
            builder.max_depth(Some(1));
        } else if let Some(max) = self.max_depth {
            builder.max_depth(Some(max.saturating_add(1)));
        }
        // The crate keeps the filter past the call, so it cannot borrow `self`:
        // the policy it runs is pure over cloned configuration anyway. The
        // abandoned-directory set resets every walk, so a tag removed since
        // the last walk does not haunt the next one.
        self.cache_dirs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let policy = self.clone();
        let walker = builder
            .filter_entry(move |entry| policy.filter_entry(entry))
            .build();
        for result in walker {
            if self.stopped.load(Ordering::SeqCst) {
                break;
            }
            let Ok(entry) = result else {
                continue;
            };
            if entry.depth() == 0 {
                continue;
            }
            let file_type = entry.file_type();
            visit(&WalkEntry {
                path: entry.path().to_path_buf(),
                is_directory: file_type.is_some_and(|kind| kind.is_dir()),
                is_symlink: file_type.is_some_and(|kind| kind.is_symlink()),
            });
        }
    }

    /// Every entry the walk reaches.
    #[must_use]
    pub fn collect(&self, root: &Path) -> Vec<WalkEntry> {
        let mut found = Vec::new();
        self.walk(root, |entry| found.push(entry.clone()));
        found
    }
}

/// Whether `dir` holds a `CACHEDIR.TAG` with the spec signature.
///
/// A directory holding one is abandoned *whole*, whatever order its entries
/// list in: the check runs on the directory before anything inside it is
/// reported, so unlike the C++ there is no listing to throw away. Public for
/// the incremental scanner, which lists directories itself.
#[must_use]
pub fn is_cache_directory(dir: &Path) -> bool {
    let tag = dir.join("CACHEDIR.TAG");
    let Ok(mut file) = File::open(&tag) else {
        return false;
    };
    let mut prefix = [0u8; CACHEDIR_TAG_SIGNATURE.len()];
    file.read_exact(&mut prefix).is_ok() && is_cachedir_tag(&tag, &prefix)
}
