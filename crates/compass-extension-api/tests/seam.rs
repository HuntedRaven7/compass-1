//! The seam gate.
//!
//! ADR-0005 says this crate must build and pass its tests with no dependency on any host
//! crate, and that if the seam collapses back into the host the third extension tier
//! should be dropped rather than maintained. That is an architectural claim, and an
//! architectural claim that nothing checks decays. So these tests check it mechanically:
//! they read the manifest and the sources and fail if a host, a transport or a runtime
//! creeps in.
//!
//! They are crude. They are also the only tests here that will still be doing their job
//! in two years.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Everything this crate may depend on, ever, without a matching change to ADR-0005.
const ALLOWED_DEPENDENCIES: &[&str] = &["serde", "serde_json", "thiserror", "tracing"];

/// Everything the tests may additionally depend on.
const ALLOWED_DEV_DEPENDENCIES: &[&str] = &["proptest"];

/// Names that must not appear in the crate's own code. Each one is a specific extension
/// host, transport or runtime: if one shows up in a type, a module or a function here,
/// the seam has stopped being front-end agnostic.
const FORBIDDEN_IN_SOURCE: &[&str] = &[
    "rhai", "jsonrpc", "json_rpc", "nodejs", "node_js", "socket", "worker", "tokio", "reqwest",
    "hyper", "zbus", "ashpd", "react", "wayland",
];

/// Crate roots a `use` statement may name.
const ALLOWED_USE_ROOTS: &[&str] = &[
    "std",
    "core",
    "alloc",
    "crate",
    "self",
    "super",
    "serde",
    "serde_json",
    "thiserror",
    "tracing",
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![crate_root().join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    assert!(
        files.len() >= 7,
        "expected the whole crate, found {files:?}"
    );
    files
}

/// The keys of one `[section]` of the manifest, ignoring nested tables.
fn manifest_section(manifest: &str, section: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            // `serde.workspace = true` names the dependency `serde`.
            let key = key.trim().trim_matches('"');
            let key = key.split('.').next().unwrap_or(key);
            out.push((key.to_string(), value.trim().to_string()));
        }
    }
    out
}

fn strip_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.find("//") {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Case-insensitive substring containment.
///
/// Deliberately *not* whole-word: `WebSocket`, `socket_path` and `SOCKET` must all trip
/// it, and a boundary rule that understands snake_case, camelCase and SCREAMING_CASE at
/// once does not exist. Every needle below is chosen so that a substring match cannot be
/// innocent.
fn contains_word(haystack: &str, needle: &str) -> bool {
    haystack.to_ascii_lowercase().contains(needle)
}

#[test]
fn the_dependency_list_is_a_small_allowlist() {
    let manifest = std::fs::read_to_string(crate_root().join("Cargo.toml")).expect("manifest");

    let deps: Vec<(String, String)> = manifest_section(&manifest, "dependencies");
    let names: BTreeSet<&str> = deps.iter().map(|(k, _)| k.as_str()).collect();
    let allowed: BTreeSet<&str> = ALLOWED_DEPENDENCIES.iter().copied().collect();
    assert_eq!(
        names, allowed,
        "the seam's dependency list changed; if that is deliberate, ADR-0005 has to change with it"
    );

    let dev: BTreeSet<String> = manifest_section(&manifest, "dev-dependencies")
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    let dev_names: BTreeSet<&str> = dev.iter().map(String::as_str).collect();
    let dev_allowed: BTreeSet<&str> = ALLOWED_DEV_DEPENDENCIES.iter().copied().collect();
    assert_eq!(
        dev_names, dev_allowed,
        "the seam's dev-dependency list changed"
    );

    assert!(
        manifest_section(&manifest, "build-dependencies").is_empty(),
        "a build script dependency would let a host leak in through the back door"
    );
}

#[test]
fn no_dependency_points_at_another_crate_in_this_repository() {
    let manifest = std::fs::read_to_string(crate_root().join("Cargo.toml")).expect("manifest");
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        for (name, value) in manifest_section(&manifest, section) {
            assert!(
                !name.starts_with("compass-"),
                "`{name}` is a Compass crate; the seam must not depend on the host"
            );
            assert!(
                !value.contains("path"),
                "`{name}` is a path dependency: {value}"
            );
            assert!(
                !value.contains("git"),
                "`{name}` is a git dependency: {value}"
            );
        }
    }
}

#[test]
fn the_lock_file_agrees_that_this_crate_has_only_its_allowlisted_dependencies() {
    // Belt and braces: the manifest could pull something in through a feature. The lock
    // file records what the resolver actually decided.
    let lock = crate_root().join("../../Cargo.lock");
    let Ok(text) = std::fs::read_to_string(&lock) else {
        // The lock file is not always present (a vendored or packaged build); the
        // manifest test above is then the whole check.
        return;
    };
    let mut block = None;
    for chunk in text.split("[[package]]") {
        if chunk.contains("name = \"compass-extension-api\"") {
            block = Some(chunk.to_string());
        }
    }
    let block = block.expect("this crate appears in the lock file");
    let listed: BTreeSet<String> = block
        .lines()
        .skip_while(|l| !l.starts_with("dependencies"))
        .skip(1)
        .take_while(|l| !l.starts_with(']'))
        .map(|l| l.trim().trim_matches(',').trim_matches('"').to_string())
        .filter(|l| !l.is_empty())
        .collect();
    assert!(
        !listed.is_empty(),
        "failed to parse the lock file entry; the check would be vacuous"
    );
    for dep in &listed {
        let name = dep.split_whitespace().next().unwrap_or(dep);
        assert!(
            ALLOWED_DEPENDENCIES.contains(&name) || ALLOWED_DEV_DEPENDENCIES.contains(&name),
            "the resolver gave this crate `{name}`, which is not on the allowlist"
        );
    }
}

#[test]
fn no_host_transport_or_runtime_is_named_in_the_crate_sources() {
    for path in source_files() {
        let source = std::fs::read_to_string(&path).expect("read source");
        let code = strip_comments(&source);
        for forbidden in FORBIDDEN_IN_SOURCE {
            assert!(
                !contains_word(&code, forbidden),
                "`{forbidden}` appears in {}; this crate must not know that exists",
                display(&path)
            );
        }
    }
}

#[test]
fn nothing_is_imported_from_outside_the_allowlist() {
    for path in source_files() {
        let source = std::fs::read_to_string(&path).expect("read source");
        for line in strip_comments(&source).lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("use ") else {
                continue;
            };
            let root = rest
                .trim_start_matches("::")
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .find(|s| !s.is_empty())
                .unwrap_or_default();
            assert!(
                ALLOWED_USE_ROOTS.contains(&root),
                "{} imports from `{root}`, which is not on the allowlist",
                display(&path)
            );
        }
    }
}

#[test]
fn the_crate_declares_no_features_that_could_pull_a_host_in() {
    let manifest = std::fs::read_to_string(crate_root().join("Cargo.toml")).expect("manifest");
    assert!(
        manifest_section(&manifest, "features").is_empty(),
        "an optional feature is how a host dependency usually arrives"
    );
}

fn display(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
