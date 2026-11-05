//! What the file indexer refuses to look at.
//!
//! Vicinae's curated policy: the exclusion lists, the `.noindex` convention,
//! machine trash directories, and the `CACHEDIR.TAG` signature check. The
//! traversal and ignore-file machinery that used to live here (`EntryFilter`,
//! `GitIgnoreReader`) now sits on the `ignore` crate in
//! [`crate::file_walk`], which matches real gitignore semantics instead of
//! the old filename-only hack.
//!
//! # Every rule here exists because something once made the index useless
//!
//! An indexer that walks `/proc` never finishes. One that walks
//! `~/.cargo/registry` fills the index with thousands of vendored source files
//! nobody searches for, and the one file they *did* want is ranked below them.
//! So this is a long list of specific names rather than a general rule, and it
//! is worth porting exactly: a name dropped from the list is not a bug anyone
//! sees as a bug, it is a search that quietly stops being useful.

use std::path::{Component, Path, PathBuf};

/// The absolute paths never indexed, from `excludedPaths()`.
///
/// These are the ones that are not under a home directory: kernel and runtime
/// filesystems where walking is either endless or pointless.
pub const EXCLUDED_ABSOLUTE_PATHS: &[&str] =
    &["/sys", "/run", "/proc", "/tmp", "/var/tmp", "/efi", "/dev"];

/// The paths never indexed, relative to the home directory.
///
/// Package caches, language toolchains, container and VM images: places with
/// enormous file counts and nothing a person searches for by name.
pub const EXCLUDED_HOME_RELATIVE_PATHS: &[&str] = &[
    ".local/share/Trash",
    ".conda",
    "anaconda3",
    "miniconda3",
    "miniforge3",
    ".cargo/registry",
    ".cargo/git",
    ".rustup",
    "go/pkg",
    ".go/pkg",
    ".bun/install",
    ".npm",
    ".nvm",
    ".pnpm-store",
    ".yarn",
    ".m2/repository",
    ".ivy2",
    ".wine",
    ".docker",
    ".android",
    ".local/share/flatpak/app",
    ".local/share/flatpak/repo/objects",
    ".local/share/flatpak/runtime",
    ".local/share/pnpm/store",
    ".local/share/Steam/config/htmlcache",
    ".local/share/compass/clipboard-data",
    // The pre-rename path, which the migration leaves as a symlink to the
    // one above; kept in case a symlink-following walk arrives through it.
    ".local/share/vicinae/clipboard-data",
    "snap",
];

/// Directory and file names never indexed, wherever they appear.
pub const EXCLUDED_FILENAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".cache",
    ".clangd",
    ".ccache",
    ".gradle",
    ".terraform",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    "node_modules",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    ".angular",
    ".svelte-kit",
    "GPUCache",
    "Code Cache",
    "CacheStorage",
    "Service Worker",
    "blob_storage",
    "DawnGraphiteCache",
    "DawnWebGPUCache",
    "__MACOSX",
    ".Trash",
    "lost+found",
    ".snapshots",
];

/// Build output directories, each with the manifests whose tool writes it.
///
/// `target` and `build` are ordinary words, so a directory with either name is
/// excluded only beside one of its manifests: a project's `target/` goes, a
/// folder of shooting-range scores called `target` stays. Cargo also tags its
/// `target/` with a `CACHEDIR.TAG`; the manifest rule still catches Maven,
/// Gradle, and a Cargo directory whose tag was removed.
pub const BUILD_OUTPUT_DIRECTORIES: &[(&str, &[&str])] = &[
    ("target", &["Cargo.toml", "pom.xml"]),
    ("build", &["build.gradle", "build.gradle.kts"]),
];

/// The suffix that marks a directory as a cache, by convention.
pub const CACHEDIR_TAG_SUFFIX: &str = "/CACHEDIR.TAG";

/// The first line such a file must start with, from the CACHEDIR.TAG spec.
///
/// Checked byte for byte: a file merely *named* `CACHEDIR.TAG` is not one, and
/// treating it as one would drop a directory somebody wanted indexed.
pub const CACHEDIR_TAG_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

/// The suffix Spotlight uses to mean "do not index", honoured by many apps.
pub const NOINDEX_SUFFIX: &str = ".noindex";

/// The absolute excluded paths, with the home-relative ones resolved under
/// `home`.
#[must_use]
pub fn excluded_paths(home: Option<&Path>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = EXCLUDED_ABSOLUTE_PATHS.iter().map(PathBuf::from).collect();
    if let Some(home) = home {
        paths.extend(
            EXCLUDED_HOME_RELATIVE_PATHS
                .iter()
                .map(|relative| home.join(relative)),
        );
    }
    paths
}

/// Whether any component of `path` starts with a dot.
///
/// Not just the last one: a file inside `~/.config` is hidden even though its
/// own name is not, which is what makes "ignore hidden paths" skip whole trees
/// rather than individual files.
#[must_use]
pub fn is_hidden_path(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
    })
}

/// The last component of `path`, as `basenameView` computes it.
///
/// A string operation, not a path one: it takes everything after the last `/`,
/// so a trailing slash yields an empty name rather than the directory's.
#[must_use]
pub fn basename(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

/// Whether `path` has `component` as one of its components.
fn path_contains(path: &Path, component: &str) -> bool {
    path.components()
        .any(|part| part.as_os_str() == std::ffi::OsStr::new(component))
}

/// Whether `second` appears as a component somewhere after `first`.
fn path_contains_after(path: &Path, first: &str, second: &str) -> bool {
    let mut seen_first = false;
    for part in path.components() {
        let name = part.as_os_str();
        if seen_first && name == std::ffi::OsStr::new(second) {
            return true;
        }
        if name == std::ffi::OsStr::new(first) {
            seen_first = true;
        }
    }
    false
}

/// Whether `path` is a directory generated by a machine and read by one.
///
/// Shader caches, crash dumps, browser storage. Two of these are conditional
/// rather than by name — `Cache_Data` only inside a `Cache`, and `cache` only
/// inside a `Shared Dictionary` — because those names are ordinary enough that
/// excluding them everywhere would drop real directories.
#[must_use]
pub fn is_machine_trash_directory(path: &Path) -> bool {
    let filename = basename(&path.to_string_lossy()).to_owned();

    match filename.as_str() {
        "Cache_Data" => path_contains(path, "Cache"),
        "GPUCache" | "Code Cache" | "CacheStorage" | "Service Worker" | "blob_storage"
        | "DawnGraphiteCache" | "DawnWebGPUCache" | "mesa_shader_cache" | "shader_cache"
        | "ShaderCache" | "GrShaderCache" | "Crashpad" | "crashpad" => true,
        "cache" => {
            path_contains(path, "Shared Dictionary") || path_contains_after(path, ".var", "cache")
        }
        _ => path_contains_after(path, ".var", "cache"),
    }
}

/// Whether `path` is a build tool's output directory: named in
/// [`BUILD_OUTPUT_DIRECTORIES`] and sitting beside one of its manifests.
///
/// Touches the disk only for a path with one of those names.
#[must_use]
pub fn is_build_output_directory(path: &Path) -> bool {
    let (Some(name), Some(parent)) = (path.file_name(), path.parent()) else {
        return false;
    };
    BUILD_OUTPUT_DIRECTORIES
        .iter()
        .filter(|(directory, _)| name == *directory)
        .any(|(_, manifests)| {
            manifests
                .iter()
                .any(|manifest| parent.join(manifest).is_file())
        })
}

/// Whether `contents` begins with the CACHEDIR.TAG signature.
#[must_use]
pub fn has_cachedir_tag_signature(contents: &[u8]) -> bool {
    contents.starts_with(CACHEDIR_TAG_SIGNATURE.as_bytes())
}

/// Whether `path` names a CACHEDIR.TAG whose contents say so.
#[must_use]
pub fn is_cachedir_tag(path: &Path, contents: &[u8]) -> bool {
    path.to_string_lossy().ends_with(CACHEDIR_TAG_SUFFIX) && has_cachedir_tag_signature(contents)
}
