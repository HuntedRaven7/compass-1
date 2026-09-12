//! Access to the checked-in test corpora.
//!
//! Corpora are byte-exact fixtures. Two of them (`non-utf8.desktop`, `crlf.desktop`) exist
//! precisely because their encoding is the thing under test, so entries expose raw bytes and
//! leave the decoding decision to the caller.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Root of the corpus tree, resolved at compile time relative to this crate.
pub fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus")
}

/// Where a corpus entry came from.
///
/// The distinction matters when a test fails: a `Real` entry failing means we broke something a
/// user has on disk today, while a `Synthetic` entry failing means we broke a deliberately
/// constructed edge case. Those warrant different urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Harvested from a real system.
    Real,
    /// Hand-written to exercise a specific edge case.
    Synthetic,
}

/// One file in a corpus.
#[derive(Debug, Clone)]
pub struct CorpusEntry {
    /// Stable identifier: the file stem, e.g. `localized-names`.
    pub id: String,
    pub path: PathBuf,
    pub provenance: Provenance,
    /// Raw file bytes. Not a `String`: part of the corpus is deliberately not valid UTF-8.
    pub bytes: Vec<u8>,
}

impl CorpusEntry {
    /// The contents as UTF-8, or `None` when the fixture is deliberately not valid UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes).ok()
    }

    /// The contents with invalid UTF-8 replaced, for parsers that accept lossy input.
    pub fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

/// Every desktop-entry fixture, sorted by id so test output is deterministic.
///
/// # Panics
///
/// Panics if the corpus directory is missing or unreadable. That is a broken checkout, not a
/// test failure, and it should be loud.
pub fn desktop_entries() -> Vec<CorpusEntry> {
    let root = corpus_root().join("desktop-entries");
    let mut entries: Vec<CorpusEntry> = WalkDir::new(&root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().is_some_and(|x| x == "desktop"))
        .map(|e| {
            let path = e.path().to_path_buf();
            let provenance = if path.components().any(|c| c.as_os_str() == "real") {
                Provenance::Real
            } else {
                Provenance::Synthetic
            };
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|err| panic!("reading corpus file {}: {err}", path.display()));
            let id = path
                .file_stem()
                .expect("corpus file has a stem")
                .to_string_lossy()
                .into_owned();
            CorpusEntry {
                id,
                path,
                provenance,
                bytes,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

/// The single desktop-entry fixture with this id.
///
/// # Panics
///
/// Panics if no such fixture exists, listing what is available — a typo in a test should say so
/// immediately rather than silently skipping.
pub fn desktop_entry(id: &str) -> CorpusEntry {
    let all = desktop_entries();
    all.iter().find(|e| e.id == id).cloned().unwrap_or_else(|| {
        let available: Vec<&str> = all.iter().map(|e| e.id.as_str()).collect();
        panic!("no desktop-entry fixture {id:?}; available: {available:?}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_is_present_and_non_trivial() {
        let entries = desktop_entries();
        assert!(
            entries.len() >= 20,
            "corpus shrank to {} entries; fixtures should only ever be added",
            entries.len()
        );
        assert!(entries.iter().any(|e| e.provenance == Provenance::Real));
        assert!(
            entries
                .iter()
                .any(|e| e.provenance == Provenance::Synthetic)
        );
    }

    #[test]
    fn ids_are_unique_and_ordering_is_deterministic() {
        let first = desktop_entries();
        let second = desktop_entries();
        let ids: Vec<&str> = first.iter().map(|e| e.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "duplicate fixture ids: {ids:?}");
        assert_eq!(
            ids,
            second.iter().map(|e| e.id.as_str()).collect::<Vec<_>>()
        );
    }

    /// The encodings are the point of these two fixtures. If git normalises them on checkout the
    /// tests that depend on them quietly stop testing anything, so assert the bytes directly.
    #[test]
    fn encoding_fixtures_survived_checkout() {
        let non_utf8 = desktop_entry("non-utf8");
        assert!(
            non_utf8.as_str().is_none(),
            "non-utf8 fixture is valid UTF-8; git or an editor normalised it"
        );

        let crlf = desktop_entry("crlf");
        assert!(
            crlf.bytes.windows(2).any(|w| w == b"\r\n"),
            "crlf fixture has no CRLF left; check .gitattributes"
        );
    }

    #[test]
    fn real_fixtures_look_like_desktop_entries() {
        for entry in desktop_entries()
            .iter()
            .filter(|e| e.provenance == Provenance::Real)
        {
            let text = entry.to_string_lossy();
            assert!(
                text.contains("[Desktop Entry]"),
                "{} has no [Desktop Entry] group",
                entry.id
            );
        }
    }

    #[test]
    fn empty_fixture_is_actually_empty() {
        assert!(desktop_entry("empty").bytes.is_empty());
    }
}
