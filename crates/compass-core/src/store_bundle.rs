//! Unpacking a store bundle onto disk, and taking it off again.
//!
//! [`crate::extension_install`] plans an install as data; this carries the
//! plan out, with the archive handling the C++ `Unzipper` leaves to trust.
//! The archive comes from the network, so it is treated as hostile:
//!
//! * **zip-slip** — an entry whose name climbs out of the directory
//!   (`../`, an absolute path, a drive prefix) refuses the whole archive,
//!   before anything is written. The C++ unpacks whatever the names say.
//! * **links** — an entry stored as a symbolic link refuses the archive too:
//!   a link written first and a file written through it second is zip-slip by
//!   another route.
//! * **size** — the download, the number of entries and the unpacked total
//!   are each capped ([`LIMITS`]). The declared sizes are checked before
//!   anything is written, and the bytes actually inflated are counted as they
//!   are written, because a declared size is only a claim.
//! * **integrity** — every entry is read to its end, which is where the zip
//!   reader checks its CRC-32, so a corrupt download fails rather than
//!   installing half an extension. The stores publish no signature, and the
//!   Vicinae store's `checksum` is not a digest of the bundle it serves (it
//!   matched no hash of the archive or its manifest), so nothing stronger is
//!   possible; `PARITY.md` records it.
//!
//! Everything happens in the staging directory; the installed version is
//! only removed once its replacement has unpacked and its `package.json`
//! parsed, as [`crate::extension_install::install_steps`] orders it.

use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::extension_install::{InstallFailure, Step, cleanup_after, install_steps};

/// The caps an archive is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest download accepted, in bytes.
    pub max_archive_bytes: u64,
    /// The most bytes it may unpack to.
    pub max_unpacked_bytes: u64,
    /// The most entries it may hold.
    pub max_entries: usize,
}

/// The caps every store install is held to.
///
/// Generous against what the stores serve — the largest Raycast bundles are
/// a few tens of megabytes unpacked — and small against a disk.
pub const LIMITS: Limits = Limits {
    max_archive_bytes: 128 * 1024 * 1024,
    max_unpacked_bytes: 512 * 1024 * 1024,
    max_entries: 20_000,
};

/// The file an install leaves beside the manifest, saying where it came from.
///
/// A dot-file, so nothing that lists an extension's files mistakes it for
/// part of the extension.
pub const MARKER_NAME: &str = ".compass-store.json";

/// Where an installed extension came from, and which build it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    /// `vicinae` or `raycast`.
    pub store: String,
    /// The handle the store files it under: its owner's, which for a Raycast
    /// extension an organisation owns is not its author's. The key keeps its
    /// old name so markers already written still read; until v23 nothing
    /// org-owned could be installed, so every one of those holds an owner.
    pub author: String,
    /// The extension's name in the store.
    pub name: String,
    /// The store's version key for the build installed: the Vicinae store's
    /// checksum, the Raycast store's commit.
    pub version: String,
}

/// Why a bundle was not installed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BundleError {
    /// The id would not name one directory inside the extensions directory.
    #[error("\"{0}\" is not a usable extension id")]
    BadId(String),
    /// The download is over [`Limits::max_archive_bytes`].
    #[error("the download is larger than {0} bytes")]
    ArchiveTooLarge(u64),
    /// It is not a zip the reader can open, or an entry failed its check.
    #[error("the download is not a readable zip archive: {0}")]
    Unreadable(String),
    /// It has more than [`Limits::max_entries`] entries.
    #[error("the archive has more than {0} entries")]
    TooManyEntries(usize),
    /// It unpacks to more than [`Limits::max_unpacked_bytes`].
    #[error("the archive unpacks to more than {0} bytes")]
    TooLarge(u64),
    /// An entry's name leaves the directory it is unpacked into.
    #[error("the archive has an entry outside its directory: {0}")]
    UnsafePath(String),
    /// An entry is a symbolic link.
    #[error("the archive holds a symbolic link: {0}")]
    Link(String),
    /// It unpacked, but holds no `package.json`.
    #[error("the archive holds no package.json, so it is not an extension")]
    NoManifest,
    /// Its `package.json` is not an extension manifest.
    #[error("the archive's package.json is not an extension manifest: {0}")]
    BadManifest(String),
    /// Writing to disk failed.
    #[error("{0}")]
    Io(String),
    /// The staged directory could not be moved into place.
    #[error("the extension could not be moved into place: {0}")]
    RenameFailed(String),
}

/// Whether `id` names exactly one ordinary directory: no separator, no
/// `..`, no leading dot (the registry skips those) and not empty.
///
/// The id is built from a name the store sent, so this is what stops a
/// listing naming its extension `../../.config` from installing there.
#[must_use]
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && !id.contains(['/', '\\', '\0'])
        && matches!(
            Path::new(id).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        )
}

/// Where entry `name` goes under `into`, after dropping `strip` leading
/// components; `None` for an entry that strips to nothing (the wrapping
/// directory itself).
fn destination(
    into: &Path,
    enclosed: &Path,
    strip: usize,
    name: &str,
) -> Result<Option<PathBuf>, BundleError> {
    let mut out = into.to_path_buf();
    let mut kept = 0;
    for component in enclosed.components().skip(strip) {
        match component {
            Component::Normal(part) => {
                out.push(part);
                kept += 1;
            }
            Component::CurDir => {}
            _ => return Err(BundleError::UnsafePath(name.to_owned())),
        }
    }
    Ok((kept > 0).then_some(out))
}

/// Unpacks `archive` into `into`, dropping `strip` leading components from
/// every entry, and answers how many bytes it wrote.
///
/// # Errors
///
/// Any [`BundleError`] the checks above describe. Nothing is written when the
/// archive's names or declared sizes are refused; a failure while writing
/// leaves a partial `into`, which the caller discards (it is always staging).
pub fn unpack(
    archive: &[u8],
    into: &Path,
    strip: usize,
    limits: &Limits,
) -> Result<u64, BundleError> {
    if archive.len() as u64 > limits.max_archive_bytes {
        return Err(BundleError::ArchiveTooLarge(limits.max_archive_bytes));
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|err| BundleError::Unreadable(err.to_string()))?;
    if zip.len() > limits.max_entries {
        return Err(BundleError::TooManyEntries(limits.max_entries));
    }

    // Every name and every declared size, before a byte is written.
    let mut declared: u64 = 0;
    for index in 0..zip.len() {
        let entry = zip
            .by_index_raw(index)
            .map_err(|err| BundleError::Unreadable(err.to_string()))?;
        let name = entry
            .name()
            .map_err(|err| BundleError::Unreadable(err.to_string()))?
            .into_owned();
        // `enclosed_name` quietly makes an absolute name relative; an archive
        // that names one is refused instead, since no store bundle does.
        if name.starts_with(['/', '\\']) || name.get(1..2) == Some(":") {
            return Err(BundleError::UnsafePath(name));
        }
        let Some(enclosed) = entry.enclosed_name() else {
            return Err(BundleError::UnsafePath(name));
        };
        destination(into, &enclosed, strip, &name)?;
        if entry.is_symlink() {
            return Err(BundleError::Link(name));
        }
        declared = declared.saturating_add(entry.size());
        if declared > limits.max_unpacked_bytes {
            return Err(BundleError::TooLarge(limits.max_unpacked_bytes));
        }
    }

    let io =
        |path: &Path, err: std::io::Error| BundleError::Io(format!("{}: {err}", path.display()));
    std::fs::create_dir_all(into).map_err(|err| io(into, err))?;
    let mut written: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|err| BundleError::Unreadable(err.to_string()))?;
        let name = entry
            .name()
            .map_err(|err| BundleError::Unreadable(err.to_string()))?
            .into_owned();
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| BundleError::UnsafePath(name.clone()))?;
        let Some(path) = destination(into, &enclosed, strip, &name)? else {
            continue;
        };
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|err| io(&path, err))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| io(parent, err))?;
        }
        let mut file = std::fs::File::create(&path).map_err(|err| io(&path, err))?;
        let remaining = limits.max_unpacked_bytes - written;
        let mut buffer = [0u8; 64 * 1024];
        let mut copied: u64 = 0;
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|err| BundleError::Unreadable(format!("{name}: {err}")))?;
            if read == 0 {
                break;
            }
            copied += read as u64;
            if copied > remaining {
                return Err(BundleError::TooLarge(limits.max_unpacked_bytes));
            }
            file.write_all(&buffer[..read])
                .map_err(|err| io(&path, err))?;
        }
        written += copied;
    }
    Ok(written)
}

/// Reads the marker an install left in `dir`, if there is one.
#[must_use]
pub fn read_marker(dir: &Path) -> Option<Marker> {
    let text = std::fs::read_to_string(dir.join(MARKER_NAME)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether the store serves a different build from the one installed.
///
/// Only an extension this launcher installed can say which build it is: one
/// installed by the C++ engine, by hand, or by an earlier Compass carries no
/// marker, and is never reported as out of date. Guessing from file times
/// was tried and rejected: the bundles' own timestamps land a few seconds
/// before the store's publication time, so every freshly installed extension
/// would read as stale.
#[must_use]
pub fn update_available(installed: Option<&Marker>, store_version: &str) -> bool {
    installed.is_some_and(|marker| !store_version.is_empty() && marker.version != store_version)
}

/// Installs `archive` as extension `id` under `extensions_dir`, carrying out
/// [`install_steps`], and answers where it went.
///
/// # Errors
///
/// A [`BundleError`]; after every one but [`BundleError::RenameFailed`] the
/// installed version, if there was one, is untouched.
pub fn install(
    extensions_dir: &Path,
    id: &str,
    archive: &[u8],
    marker: &Marker,
    limits: &Limits,
) -> Result<PathBuf, BundleError> {
    if !is_safe_id(id) {
        return Err(BundleError::BadId(id.to_owned()));
    }
    std::fs::create_dir_all(extensions_dir)
        .map_err(|err| BundleError::Io(format!("{}: {err}", extensions_dir.display())))?;

    let fail = |failure: InstallFailure, error: BundleError| {
        for step in cleanup_after(&failure, extensions_dir, id) {
            if let Step::RemoveAll(path) = step {
                let _ = std::fs::remove_dir_all(path);
            }
        }
        error
    };

    let mut installed = None;
    for step in install_steps(extensions_dir, id) {
        match step {
            Step::RemoveAll(path) => {
                let _ = std::fs::remove_dir_all(path);
            }
            Step::Extract {
                into,
                strip_components,
            } => {
                if let Err(error) = unpack(archive, &into, strip_components, limits) {
                    // Unlike the C++, whose unreadable archive never touches
                    // the disk, a refusal here may follow a partial write, so
                    // staging is always cleared.
                    return Err(fail(InstallFailure::NoManifest, error));
                }
            }
            Step::RequireManifest(manifest) => {
                if !manifest.is_file() {
                    return Err(fail(InstallFailure::NoManifest, BundleError::NoManifest));
                }
                let staged = manifest.parent().unwrap_or(extensions_dir);
                if let Err(err) = crate::manifest::ExtensionManifest::from_directory(staged) {
                    return Err(fail(
                        InstallFailure::NoManifest,
                        BundleError::BadManifest(err.to_string()),
                    ));
                }
                let written = serde_json::to_string_pretty(marker)
                    .map_err(|err| err.to_string())
                    .and_then(|text| {
                        std::fs::write(staged.join(MARKER_NAME), text)
                            .map_err(|err| err.to_string())
                    });
                if let Err(err) = written {
                    return Err(fail(InstallFailure::NoManifest, BundleError::Io(err)));
                }
            }
            Step::Rename { from, to } => {
                if let Err(err) = std::fs::rename(&from, &to) {
                    return Err(fail(
                        InstallFailure::RenameFailed,
                        BundleError::RenameFailed(err.to_string()),
                    ));
                }
                installed = Some(to);
            }
        }
    }
    installed.ok_or_else(|| BundleError::Io("the install plan has no final step".to_owned()))
}

/// Removes an installed extension's directory and its support directory, as
/// `ExtensionRegistry::uninstall` does. The support directory's removal is
/// best-effort; the extension's is not.
///
/// # Errors
///
/// A sentence when `dir` is not a directory or could not be removed.
pub fn uninstall(dir: &Path, support_dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{} is not an installed extension", dir.display()));
    }
    std::fs::remove_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let _ = std::fs::remove_dir_all(support_dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            if name.ends_with('/') {
                out.add_directory(*name, SimpleFileOptions::default())
                    .expect("dir");
            } else {
                out.start_file(*name, SimpleFileOptions::default())
                    .expect("file");
                out.write_all(bytes).expect("write");
            }
        }
        out.finish().expect("finish").into_inner()
    }

    const MANIFEST: &[u8] =
        br#"{"name": "clock", "title": "Clock", "author": "zoe", "commands": [{"name": "show", "title": "Show", "mode": "view"}]}"#;

    fn marker(version: &str) -> Marker {
        Marker {
            store: "vicinae".into(),
            author: "zoe".into(),
            name: "clock".into(),
            version: version.into(),
        }
    }

    #[test]
    fn a_bundle_installs_one_level_up_with_its_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = zip_of(&[
            ("clock/", b""),
            ("clock/package.json", MANIFEST),
            ("clock/show.js", b"export default 1"),
            ("clock/assets/icon.png", b"png"),
        ]);
        let path = install(
            dir.path(),
            "store.vicinae.clock",
            &archive,
            &marker("v1"),
            &LIMITS,
        )
        .expect("installed");
        assert_eq!(path, dir.path().join("store.vicinae.clock"));
        assert!(path.join("package.json").is_file());
        assert!(path.join("assets/icon.png").is_file());
        assert_eq!(read_marker(&path), Some(marker("v1")));
        assert!(
            !dir.path().join(".staging-store.vicinae.clock").exists(),
            "staging is renamed away"
        );
        let scan = crate::manifest::registry::scan(&[dir.path().to_path_buf()]);
        assert_eq!(scan.extensions.len(), 1, "the registry finds it");
    }

    #[test]
    fn a_climbing_entry_refuses_the_archive_before_anything_is_written() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = zip_of(&[
            ("clock/package.json", MANIFEST),
            ("clock/../../escaped.txt", b"gotcha"),
        ]);
        let err = install(
            dir.path(),
            "store.vicinae.clock",
            &archive,
            &marker("v1"),
            &LIMITS,
        )
        .expect_err("refused");
        assert!(matches!(err, BundleError::UnsafePath(_)), "{err:?}");
        assert!(!dir.path().join("escaped.txt").exists());
        assert!(!dir.path().parent().unwrap().join("escaped.txt").exists());
        assert!(!dir.path().join("store.vicinae.clock").exists());
        assert!(!dir.path().join(".staging-store.vicinae.clock").exists());
    }

    #[test]
    fn an_absolute_entry_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = zip_of(&[("/etc/passwd", b"root")]);
        let err = unpack(&archive, dir.path(), 0, &LIMITS).expect_err("refused");
        assert!(matches!(err, BundleError::UnsafePath(_)), "{err:?}");
    }

    #[test]
    fn a_symbolic_link_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        out.add_symlink("clock/link", "/etc", SimpleFileOptions::default())
            .expect("link");
        let archive = out.finish().expect("finish").into_inner();
        let err = unpack(&archive, dir.path(), 1, &LIMITS).expect_err("refused");
        assert!(matches!(err, BundleError::Link(_)), "{err:?}");
    }

    #[test]
    fn oversized_archives_are_refused_by_every_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = zip_of(&[
            ("clock/package.json", MANIFEST),
            ("clock/big", &[0u8; 4096]),
        ]);
        let tight = |archive_bytes, unpacked, entries| Limits {
            max_archive_bytes: archive_bytes,
            max_unpacked_bytes: unpacked,
            max_entries: entries,
        };
        assert_eq!(
            unpack(&archive, dir.path(), 1, &tight(10, 1 << 20, 100)),
            Err(BundleError::ArchiveTooLarge(10))
        );
        assert_eq!(
            unpack(&archive, dir.path(), 1, &tight(1 << 20, 1000, 100)),
            Err(BundleError::TooLarge(1000))
        );
        assert_eq!(
            unpack(&archive, dir.path(), 1, &tight(1 << 20, 1 << 20, 1)),
            Err(BundleError::TooManyEntries(1))
        );
    }

    #[test]
    fn a_bad_download_leaves_the_installed_version_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let good = zip_of(&[("clock/package.json", MANIFEST)]);
        install(
            dir.path(),
            "store.vicinae.clock",
            &good,
            &marker("v1"),
            &LIMITS,
        )
        .expect("v1");

        let not_zip = b"<html>rate limited</html>";
        let err = install(
            dir.path(),
            "store.vicinae.clock",
            not_zip,
            &marker("v2"),
            &LIMITS,
        )
        .expect_err("refused");
        assert!(matches!(err, BundleError::Unreadable(_)), "{err:?}");

        let no_manifest = zip_of(&[("clock/readme.md", b"hi")]);
        let err = install(
            dir.path(),
            "store.vicinae.clock",
            &no_manifest,
            &marker("v2"),
            &LIMITS,
        )
        .expect_err("refused");
        assert_eq!(err, BundleError::NoManifest);

        let installed = dir.path().join("store.vicinae.clock");
        assert_eq!(
            read_marker(&installed),
            Some(marker("v1")),
            "v1 is still there"
        );
        assert!(!dir.path().join(".staging-store.vicinae.clock").exists());
    }

    #[test]
    fn a_corrupt_entry_fails_its_checksum() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        out.start_file(
            "clock/package.json",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .expect("file");
        out.write_all(MANIFEST).expect("write");
        let mut archive = out.finish().expect("finish").into_inner();
        // Flip a byte of the stored (uncompressed) content.
        let at = archive
            .windows(7)
            .position(|w| w == b"\"clock\"")
            .expect("the content is stored");
        archive[at + 1] ^= 0x20;
        let err = unpack(&archive, dir.path(), 1, &LIMITS).expect_err("refused");
        assert!(matches!(err, BundleError::Unreadable(_)), "{err:?}");
    }

    #[test]
    fn ids_that_would_leave_the_directory_are_refused() {
        for bad in ["", ".hidden", "../up", "a/b", "a\\b", "..", "."] {
            assert!(!is_safe_id(bad), "{bad:?}");
        }
        assert!(is_safe_id("store.raycast.spotify-player"));
        let dir = tempfile::tempdir().expect("tempdir");
        let good = zip_of(&[("clock/package.json", MANIFEST)]);
        assert_eq!(
            install(
                dir.path(),
                "store.vicinae../x",
                &good,
                &marker("v1"),
                &LIMITS
            ),
            Err(BundleError::BadId("store.vicinae../x".into()))
        );
    }

    #[test]
    fn only_a_marked_install_with_a_different_build_is_out_of_date() {
        assert!(
            !update_available(None, "v2"),
            "unknown provenance is never stale"
        );
        assert!(!update_available(Some(&marker("v2")), "v2"));
        assert!(update_available(Some(&marker("v1")), "v2"));
        assert!(
            !update_available(Some(&marker("v1")), ""),
            "no version to compare"
        );
    }

    #[test]
    fn uninstalling_removes_the_extension_and_its_support_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ext = dir.path().join("extensions/store.vicinae.clock");
        let support = dir.path().join("support/store.vicinae.clock");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::create_dir_all(&support).unwrap();
        uninstall(&ext, &support).expect("removed");
        assert!(!ext.exists() && !support.exists());
        assert!(uninstall(&ext, &support).is_err(), "not installed any more");
    }
}
