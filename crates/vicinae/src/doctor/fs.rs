//! An injectable view of the filesystem.
//!
//! Two questions only: does this path exist, and what is in this directory.
//! That is everything the doctor's filesystem checks need, and keeping the
//! surface that small is what makes [`FakeFs`] a few lines instead of a
//! reimplementation of `std::fs`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Read-only filesystem queries used by the doctor's checks.
pub trait FsProbe {
    /// Whether `path` exists (following symlinks).
    fn exists(&self, path: &Path) -> bool;

    /// The file names directly inside `path`.
    ///
    /// `Err` carries a human-readable reason (permission denied, not a
    /// directory, …). A directory that does not exist is an `Err` too; callers
    /// that care about the difference ask [`FsProbe::exists`] first, because
    /// "absent" and "present but unreadable" are different diagnoses.
    fn dir_entries(&self, path: &Path) -> Result<Vec<String>, String>;
}

/// [`FsProbe`] backed by the real filesystem.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

impl FsProbe for RealFs {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn dir_entries(&self, path: &Path) -> Result<Vec<String>, String> {
        let read = std::fs::read_dir(path).map_err(|e| e.to_string())?;
        let mut names = Vec::new();
        for entry in read {
            let entry = entry.map_err(|e| e.to_string())?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }
}

/// [`FsProbe`] built from literal facts, for tests.
///
/// Anything not explicitly added is absent, which is the state most of the
/// doctor's interesting cases live in.
#[derive(Debug, Clone, Default)]
pub struct FakeFs {
    files: BTreeSet<PathBuf>,
    dirs: BTreeMap<PathBuf, Result<Vec<String>, String>>,
}

impl FakeFs {
    /// An empty filesystem: nothing exists.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares that `path` exists as a plain file.
    #[must_use]
    pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.files.insert(path.into());
        self
    }

    /// Declares a readable directory holding exactly `entries`.
    #[must_use]
    pub fn with_dir<I, S>(mut self, path: impl Into<PathBuf>, entries: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.dirs.insert(
            path.into(),
            Ok(entries.into_iter().map(Into::into).collect()),
        );
        self
    }

    /// Declares a directory that exists but cannot be read.
    #[must_use]
    pub fn with_unreadable_dir(
        mut self,
        path: impl Into<PathBuf>,
        reason: impl Into<String>,
    ) -> Self {
        self.dirs.insert(path.into(), Err(reason.into()));
        self
    }
}

impl FsProbe for FakeFs {
    fn exists(&self, path: &Path) -> bool {
        self.files.contains(path) || self.dirs.contains_key(path)
    }

    fn dir_entries(&self, path: &Path) -> Result<Vec<String>, String> {
        match self.dirs.get(path) {
            Some(Ok(entries)) => Ok(entries.clone()),
            Some(Err(reason)) => Err(reason.clone()),
            None => Err("No such file or directory (os error 2)".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_reports_only_what_was_declared() {
        let fs = FakeFs::new()
            .with_file("/.flatpak-info")
            .with_dir("/usr/share/applications", ["a.desktop", "b.desktop"]);

        assert!(fs.exists(Path::new("/.flatpak-info")));
        assert!(fs.exists(Path::new("/usr/share/applications")));
        assert!(!fs.exists(Path::new("/usr/local/share/applications")));
        assert_eq!(
            fs.dir_entries(Path::new("/usr/share/applications"))
                .unwrap(),
            ["a.desktop", "b.desktop"]
        );
    }

    #[test]
    fn fake_distinguishes_absent_from_unreadable() {
        let fs = FakeFs::new()
            .with_unreadable_dir("/root/.local/share/applications", "Permission denied");

        assert!(fs.exists(Path::new("/root/.local/share/applications")));
        assert_eq!(
            fs.dir_entries(Path::new("/root/.local/share/applications")),
            Err("Permission denied".to_string())
        );
        assert!(!fs.exists(Path::new("/nope")));
        assert!(fs.dir_entries(Path::new("/nope")).is_err());
    }

    #[test]
    fn real_fs_reads_a_temp_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("one.desktop"), "x").expect("write");
        let fs = RealFs;
        assert!(fs.exists(dir.path()));
        assert_eq!(fs.dir_entries(dir.path()).unwrap(), ["one.desktop"]);
        assert!(fs.dir_entries(&dir.path().join("missing")).is_err());
    }
}
