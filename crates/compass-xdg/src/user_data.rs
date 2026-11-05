//! The directories that hold a user's Compass data, and deleting them when
//! the user asks to (`compass data remove`).
//!
//! Uninstalling a package never deletes a user's data (`packaging/README.md`):
//! a package runs as root, outside every user's session, and a reinstall
//! should find everything where it was. Deleting is the user's own explicit
//! act, and this module keeps it to what Compass owns:
//!
//! - `compass` under each of the config, data, cache and state homes;
//! - the `vicinae` symlink [`crate::brand::migrate_legacy_dir`] leaves beside
//!   it, which points at it.
//!
//! A real `vicinae` directory, or a `vicinae` link that points anywhere else,
//! belongs to upstream Vicinae, which may still be installed beside Compass
//! (ADR-0017, ADR-0020). It is listed so the user sees it, and removed only
//! when the caller opts in.
//!
//! A symlink is removed as a link. What it points at, a dotfiles checkout for
//! example, is never followed. The runtime directory is left out on purpose:
//! it holds only a running engine's socket and lock, and is emptied at logout.

use std::io;
use std::path::{Path, PathBuf};

use crate::brand::{DIR_NAME, LEGACY_DIR_NAME};

/// The XDG base directory a path sits under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// `$XDG_CONFIG_HOME`: `compass.json`, themes, Rhai scripts.
    Config,
    /// `$XDG_DATA_HOME`: the databases, extensions and clipboard history.
    Data,
    /// `$XDG_CACHE_HOME`: what can be rebuilt, such as the file index and icons.
    Cache,
    /// `$XDG_STATE_HOME`: the engine's log.
    State,
}

impl Base {
    /// Every base, in the order they are listed.
    pub const ALL: [Self; 4] = [Self::Config, Self::Data, Self::Cache, Self::State];

    /// This base directory, from the environment.
    #[must_use]
    pub fn home(self) -> Option<PathBuf> {
        match self {
            Self::Config => crate::config_home(),
            Self::Data => crate::data_home(),
            Self::Cache => crate::cache_home(),
            Self::State => crate::brand::state_home(),
        }
    }

    /// The variable that names this base, for display.
    #[must_use]
    pub const fn variable(self) -> &'static str {
        match self {
            Self::Config => "XDG_CONFIG_HOME",
            Self::Data => "XDG_DATA_HOME",
            Self::Cache => "XDG_CACHE_HOME",
            Self::State => "XDG_STATE_HOME",
        }
    }
}

/// What a path holds, without following a symlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// Nothing is there.
    Missing,
    /// A directory.
    Directory,
    /// A file, or anything else that is neither a directory nor a link.
    File,
    /// A symlink, pointing at `target` as written in the link.
    Symlink {
        /// The link's contents.
        target: PathBuf,
    },
}

impl Found {
    /// What is at `path` now.
    ///
    /// # Errors
    ///
    /// When `path` cannot be inspected for a reason other than not existing.
    pub fn at(path: &Path) -> io::Result<Self> {
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::Missing),
            Err(error) => return Err(error),
        };
        let kind = metadata.file_type();
        Ok(if kind.is_symlink() {
            Self::Symlink {
                target: std::fs::read_link(path)?,
            }
        } else if kind.is_dir() {
            Self::Directory
        } else {
            Self::File
        })
    }
}

/// Whose a path is, which decides whether it is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// Compass's own `compass` entry.
    Compass,
    /// The `vicinae` link the move to the `compass` name left behind; it
    /// points at Compass's directory.
    CompatibilityLink,
    /// A `vicinae` directory, or a `vicinae` link that points elsewhere:
    /// upstream Vicinae's.
    Vicinae,
}

/// One candidate path and what will happen to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPath {
    /// The base directory it sits under.
    pub base: Base,
    /// The path itself.
    pub path: PathBuf,
    /// What is there.
    pub found: Found,
    /// Whose it is.
    pub owner: Owner,
    /// Whether [`remove`] deletes it.
    pub remove: bool,
}

/// Every candidate path under `bases`, present or not, in the order given:
/// `compass` then `vicinae` under each.
///
/// A base listed twice (`XDG_CACHE_HOME` may equal `XDG_DATA_HOME`) is
/// listed once, under its first name. A relative base is skipped, as the
/// base-directory specification says: it would resolve against whatever the
/// working directory happens to be.
///
/// `include_vicinae` marks upstream Vicinae's entries for removal as well.
///
/// # Errors
///
/// When a path cannot be inspected.
pub fn survey(bases: &[(Base, PathBuf)], include_vicinae: bool) -> io::Result<Vec<UserPath>> {
    let mut seen: Vec<&Path> = Vec::with_capacity(bases.len());
    let mut out = Vec::with_capacity(bases.len() * 2);
    for (base, dir) in bases {
        if !dir.is_absolute() || seen.contains(&dir.as_path()) {
            continue;
        }
        seen.push(dir);
        let current = dir.join(DIR_NAME);
        let found = Found::at(&current)?;
        out.push(UserPath {
            base: *base,
            remove: found != Found::Missing,
            path: current.clone(),
            found,
            owner: Owner::Compass,
        });
        let legacy = dir.join(LEGACY_DIR_NAME);
        let found = Found::at(&legacy)?;
        let owner = match &found {
            Found::Symlink { target } if points_at(dir, target, &current) => {
                Owner::CompatibilityLink
            }
            _ => Owner::Vicinae,
        };
        out.push(UserPath {
            base: *base,
            remove: found != Found::Missing
                && (owner == Owner::CompatibilityLink || include_vicinae),
            path: legacy,
            found,
            owner,
        });
    }
    Ok(out)
}

/// [`survey`] over the base directories this process's environment names.
///
/// # Errors
///
/// When a path cannot be inspected.
pub fn survey_env(include_vicinae: bool) -> io::Result<Vec<UserPath>> {
    let bases: Vec<(Base, PathBuf)> = Base::ALL
        .into_iter()
        .filter_map(|base| Some((base, base.home()?)))
        .collect();
    survey(&bases, include_vicinae)
}

/// Whether a link in `dir` whose contents are `target` resolves to `current`.
///
/// Compared as paths first, which is what the migration writes (a relative
/// `compass`) and what still holds when `current` has since been deleted;
/// then through the filesystem, for an absolute link through another name.
fn points_at(dir: &Path, target: &Path, current: &Path) -> bool {
    let resolved = dir.join(target);
    if resolved == current {
        return true;
    }
    match (resolved.canonicalize(), current.canonicalize()) {
        (Ok(resolved), Ok(current)) => resolved == current,
        _ => false,
    }
}

/// Why one path could not be removed.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    /// The path no longer holds what was listed, so it is left alone rather
    /// than deleting something the user was not shown.
    #[error("{} changed after it was listed, so it was left alone", path.display())]
    Changed {
        /// The path.
        path: PathBuf,
    },
    /// The filesystem refused.
    #[error("could not remove {}: {source}", path.display())]
    Io {
        /// The path.
        path: PathBuf,
        /// The error.
        source: io::Error,
    },
}

/// Deletes each path [`survey`] marked for removal: a directory with all it
/// holds, and a symlink as a link only.
///
/// Each path is inspected again first, and one that no longer holds what was
/// listed is left alone. Returns the outcome for each path attempted, in
/// order; a failure does not stop the rest.
pub fn remove(paths: &[UserPath]) -> Vec<(&UserPath, Result<(), RemoveError>)> {
    paths
        .iter()
        .filter(|entry| entry.remove && entry.found != Found::Missing)
        .map(|entry| (entry, remove_one(entry)))
        .collect()
}

fn remove_one(entry: &UserPath) -> Result<(), RemoveError> {
    let io_error = |source| RemoveError::Io {
        path: entry.path.clone(),
        source,
    };
    if Found::at(&entry.path).map_err(io_error)? != entry.found {
        return Err(RemoveError::Changed {
            path: entry.path.clone(),
        });
    }
    // `remove_dir_all` does not follow symlinks inside the tree, so a link
    // in the data directory never takes its target with it.
    match entry.found {
        Found::Directory => std::fs::remove_dir_all(&entry.path),
        Found::File | Found::Symlink { .. } => std::fs::remove_file(&entry.path),
        Found::Missing => Ok(()),
    }
    .map_err(io_error)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn bases(root: &Path) -> Vec<(Base, PathBuf)> {
        Base::ALL
            .into_iter()
            .map(|base| (base, root.join(base.variable())))
            .collect()
    }

    fn entry<'a>(paths: &'a [UserPath], path: &Path) -> &'a UserPath {
        paths
            .iter()
            .find(|entry| entry.path == path)
            .expect("the path is listed")
    }

    #[test]
    fn every_base_lists_compass_and_vicinae_present_or_not() {
        let root = tempfile::tempdir().expect("tempdir");
        let paths = survey(&bases(root.path()), false).expect("survey");
        assert_eq!(paths.len(), 8);
        assert!(paths.iter().all(|entry| entry.found == Found::Missing));
        assert!(paths.iter().all(|entry| !entry.remove));
    }

    #[test]
    fn compass_and_its_compatibility_link_go_and_vicinae_s_directory_stays() {
        let root = tempfile::tempdir().expect("tempdir");
        let config = root.path().join("XDG_CONFIG_HOME");
        let data = root.path().join("XDG_DATA_HOME");
        std::fs::create_dir_all(config.join("compass")).expect("mkdir");
        symlink("compass", config.join("vicinae")).expect("link");
        std::fs::create_dir_all(data.join("compass")).expect("mkdir");
        std::fs::create_dir_all(data.join("vicinae")).expect("mkdir");

        let paths = survey(&bases(root.path()), false).expect("survey");
        let link = entry(&paths, &config.join("vicinae"));
        assert_eq!(link.owner, Owner::CompatibilityLink);
        assert!(link.remove);
        let upstream = entry(&paths, &data.join("vicinae"));
        assert_eq!(upstream.owner, Owner::Vicinae);
        assert!(!upstream.remove);

        let outcomes = remove(&paths);
        assert!(outcomes.iter().all(|(_, outcome)| outcome.is_ok()));
        assert!(!config.join("compass").exists());
        assert!(config.join("vicinae").symlink_metadata().is_err());
        assert!(!data.join("compass").exists());
        assert!(data.join("vicinae").is_dir());
    }

    #[test]
    fn a_dangling_compatibility_link_is_still_compass_s() {
        let root = tempfile::tempdir().expect("tempdir");
        let cache = root.path().join("XDG_CACHE_HOME");
        std::fs::create_dir_all(&cache).expect("mkdir");
        symlink("compass", cache.join("vicinae")).expect("link");
        let paths = survey(&bases(root.path()), false).expect("survey");
        assert_eq!(
            entry(&paths, &cache.join("vicinae")).owner,
            Owner::CompatibilityLink
        );
    }

    #[test]
    fn include_vicinae_removes_its_directory_too() {
        let root = tempfile::tempdir().expect("tempdir");
        let data = root.path().join("XDG_DATA_HOME");
        std::fs::create_dir_all(data.join("vicinae/extensions")).expect("mkdir");
        let paths = survey(&bases(root.path()), true).expect("survey");
        assert!(entry(&paths, &data.join("vicinae")).remove);
        remove(&paths);
        assert!(!data.join("vicinae").exists());
    }

    #[test]
    fn a_linked_compass_directory_loses_the_link_and_keeps_its_target() {
        let root = tempfile::tempdir().expect("tempdir");
        let config = root.path().join("XDG_CONFIG_HOME");
        let dotfiles = root.path().join("dotfiles/compass");
        std::fs::create_dir_all(&dotfiles).expect("mkdir");
        std::fs::write(dotfiles.join("compass.json"), "{}").expect("write");
        std::fs::create_dir_all(&config).expect("mkdir");
        symlink(&dotfiles, config.join("compass")).expect("link");

        let paths = survey(&bases(root.path()), false).expect("survey");
        remove(&paths);
        assert!(config.join("compass").symlink_metadata().is_err());
        assert!(dotfiles.join("compass.json").is_file());
    }

    #[test]
    fn a_link_inside_the_data_directory_does_not_take_its_target() {
        let root = tempfile::tempdir().expect("tempdir");
        let data = root.path().join("XDG_DATA_HOME");
        let outside = root.path().join("Documents");
        std::fs::create_dir_all(&outside).expect("mkdir");
        std::fs::write(outside.join("notes.txt"), "keep").expect("write");
        std::fs::create_dir_all(data.join("compass")).expect("mkdir");
        symlink(&outside, data.join("compass/documents")).expect("link");

        let paths = survey(&bases(root.path()), false).expect("survey");
        remove(&paths);
        assert!(!data.join("compass").exists());
        assert!(outside.join("notes.txt").is_file());
    }

    #[test]
    fn neighbours_are_untouched() {
        let root = tempfile::tempdir().expect("tempdir");
        let config = root.path().join("XDG_CONFIG_HOME");
        for name in ["compass", "compass-notes", "other-app"] {
            std::fs::create_dir_all(config.join(name)).expect("mkdir");
        }
        std::fs::write(config.join("compass.json"), "{}").expect("write");
        let paths = survey(&bases(root.path()), false).expect("survey");
        remove(&paths);
        assert!(!config.join("compass").exists());
        assert!(config.join("compass-notes").is_dir());
        assert!(config.join("other-app").is_dir());
        assert!(config.join("compass.json").is_file());
    }

    #[test]
    fn a_shared_base_is_listed_once_and_a_relative_one_not_at_all() {
        let root = tempfile::tempdir().expect("tempdir");
        let shared = root.path().join("shared");
        let paths = survey(
            &[
                (Base::Data, shared.clone()),
                (Base::Cache, shared),
                (Base::State, PathBuf::from("relative")),
            ],
            false,
        )
        .expect("survey");
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|entry| entry.base == Base::Data));
    }

    #[test]
    fn a_path_that_changed_after_the_listing_is_left_alone() {
        let root = tempfile::tempdir().expect("tempdir");
        let state = root.path().join("XDG_STATE_HOME");
        std::fs::create_dir_all(&state).expect("mkdir");
        let target = root.path().join("elsewhere");
        std::fs::create_dir_all(&target).expect("mkdir");
        symlink(&target, state.join("compass")).expect("link");
        let paths = survey(&bases(root.path()), false).expect("survey");

        std::fs::remove_file(state.join("compass")).expect("unlink");
        std::fs::create_dir_all(state.join("compass/kept")).expect("mkdir");
        let outcomes = remove(&paths);
        assert!(matches!(
            outcomes.as_slice(),
            [(_, Err(RemoveError::Changed { .. }))]
        ));
        assert!(state.join("compass/kept").is_dir());
    }
}
