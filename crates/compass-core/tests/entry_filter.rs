//! What the file indexer refuses to walk into.
//!
//! Read off `EntryFilter` (`src/file-indexer/src/entry-filter.cpp`). The
//! curated lists and name checks live here; traversal and ignore-file matching
//! moved onto the `ignore` crate (`file_walk`), so ignore behaviour is pinned
//! there, over real trees, instead of here.

use std::path::{Path, PathBuf};

use compass_core::entry_filter::{
    BUILD_OUTPUT_DIRECTORIES, CACHEDIR_TAG_SIGNATURE, EXCLUDED_ABSOLUTE_PATHS, EXCLUDED_FILENAMES,
    NOINDEX_SUFFIX, basename, excluded_paths, has_cachedir_tag_signature,
    is_build_output_directory, is_cachedir_tag, is_hidden_path, is_machine_trash_directory,
};
use compass_core::file_walk::IndexWalk;

/// A walk with the built-ins resolved under `/home/ada`.
fn walk() -> IndexWalk {
    IndexWalk::new(Some(Path::new("/home/ada")))
}

fn visit(walk: &IndexWalk, path: &str, is_directory: bool) -> bool {
    walk.should_visit(Path::new(path), false, is_directory)
}

fn file(path: &str) -> bool {
    visit(&walk(), path, false)
}

fn directory(path: &str) -> bool {
    visit(&walk(), path, true)
}

#[test]
fn an_ordinary_file_is_visited() {
    assert!(file("/home/ada/notes.txt"));
}

#[test]
fn a_symlink_is_never_followed() {
    // The first check, and the one that keeps a walk from looping for ever
    // through a link back up the tree.
    assert!(!walk().should_visit(Path::new("/home/ada/link"), true, true));
}

#[test]
fn the_kernel_and_runtime_filesystems_are_never_walked() {
    // An indexer that walks /proc never finishes.
    for path in EXCLUDED_ABSOLUTE_PATHS {
        assert!(!directory(path), "{path} should be excluded");
    }
    assert!(EXCLUDED_ABSOLUTE_PATHS.contains(&"/proc"));
    assert!(EXCLUDED_ABSOLUTE_PATHS.contains(&"/sys"));
}

#[test]
fn the_home_relative_exclusions_resolve_under_the_home_directory() {
    let paths = excluded_paths(Some(Path::new("/home/ada")));
    assert!(paths.contains(&PathBuf::from("/home/ada/.cargo/registry")));
    assert!(
        !paths.contains(&PathBuf::from("/home/ada/node_modules")),
        "node_modules is a filename rule, not a path one"
    );
    assert!(paths.contains(&PathBuf::from("/proc")));
}

#[test]
fn a_package_cache_is_excluded_by_its_whole_path_not_its_name() {
    // ~/.cargo/registry is excluded; a directory called "registry" elsewhere
    // is not, because the exclusion is a path and not a filename.
    assert!(!directory("/home/ada/.cargo/registry"));

    let open = IndexWalk::new(Some(Path::new("/home/ada")));
    assert!(visit(&open, "/home/ada/projects/registry", true));
}

#[test]
fn the_home_relative_list_is_the_cpp_list() {
    // Each of these is a directory with an enormous file count and nothing a
    // person searches for by name; one quietly dropped is a search that stops
    // being useful, not a bug anyone reports.
    for expected in [
        ".cargo/registry",
        ".rustup",
        ".npm",
        ".local/share/Trash",
        ".local/share/flatpak/runtime",
        "snap",
    ] {
        assert!(
            compass_core::entry_filter::EXCLUDED_HOME_RELATIVE_PATHS.contains(&expected),
            "{expected} should be excluded"
        );
    }
}

#[test]
fn without_a_home_directory_only_the_absolute_exclusions_apply() {
    let paths = excluded_paths(None);
    assert_eq!(paths.len(), EXCLUDED_ABSOLUTE_PATHS.len());
    assert!(paths.contains(&PathBuf::from("/proc")));
}

#[test]
fn the_excluded_filenames_are_skipped_wherever_they_appear() {
    for name in ["node_modules", ".git", "__pycache__", ".venv", "lost+found"] {
        assert!(
            EXCLUDED_FILENAMES.contains(&name),
            "{name} should be on the list"
        );
        let path = format!("/home/ada/project/{name}");
        assert!(!directory(&path), "{path} should be excluded");
    }
}

#[test]
fn a_noindex_suffix_excludes_anything() {
    // The Spotlight convention, honoured by many apps.
    assert_eq!(NOINDEX_SUFFIX, ".noindex");
    assert!(!directory("/home/ada/Big.noindex"));
    assert!(!file("/home/ada/data.noindex"));
}

#[test]
fn configured_filenames_are_excluded_as_well_as_the_built_in_ones() {
    let mut configured = walk();
    configured.set_excluded_filenames(vec!["target".to_owned()]);
    assert!(!visit(&configured, "/home/ada/project/target", true));
    assert!(visit(&configured, "/home/ada/project/src", true));
}

#[test]
fn configured_paths_are_excluded() {
    let mut configured = walk();
    configured.set_excluded_paths(vec![PathBuf::from("/home/ada/Videos")]);
    assert!(!visit(&configured, "/home/ada/Videos", true));
    assert!(visit(&configured, "/home/ada/Videos/holiday", true));
    assert!(configured.is_excluded_path(Path::new("/home/ada/Videos")));
}

#[test]
fn hidden_paths_are_skipped_only_when_asked_for() {
    assert!(file("/home/ada/.bashrc"), "off by default");

    let mut hidden = walk();
    hidden.set_ignore_hidden_paths(true);
    assert!(!visit(&hidden, "/home/ada/.bashrc", false));
}

#[test]
fn a_hidden_ancestor_hides_everything_below_it() {
    // isHiddenPath tests every component, which is what makes this skip whole
    // trees rather than individual files.
    let mut hidden = walk();
    hidden.set_ignore_hidden_paths(true);
    assert!(!visit(
        &hidden,
        "/home/ada/.config/app/settings.json",
        false
    ));
    assert!(is_hidden_path(Path::new(
        "/home/ada/.config/app/settings.json"
    )));
    assert!(!is_hidden_path(Path::new("/home/ada/config/app.json")));
}

#[test]
fn a_machine_cache_directory_is_skipped_but_a_file_of_that_name_is_not() {
    // The check is guarded by is_directory in the C++, so a *file* called
    // GPUCache is indexed. That asymmetry is the port's to keep.
    assert!(!directory("/home/ada/app/GrShaderCache"));
    assert!(file("/home/ada/app/GrShaderCache"));
}

#[test]
fn cache_data_is_only_trash_inside_a_cache() {
    // "Cache_Data" is specific enough to be a machine's, but only where the
    // surrounding path says so.
    assert!(is_machine_trash_directory(Path::new(
        "/home/ada/app/Cache/Cache_Data"
    )));
    assert!(!is_machine_trash_directory(Path::new(
        "/home/ada/notes/Cache_Data"
    )));
}

#[test]
fn a_directory_called_cache_is_only_trash_in_the_right_company() {
    // "cache" alone is far too common to exclude everywhere.
    assert!(is_machine_trash_directory(Path::new(
        "/home/ada/app/Shared Dictionary/cache"
    )));
    assert!(is_machine_trash_directory(Path::new(
        "/home/ada/.var/app/org.x/cache"
    )));
    assert!(!is_machine_trash_directory(Path::new(
        "/home/ada/project/cache"
    )));
}

#[test]
fn a_var_cache_is_only_trash_when_cache_comes_after_var() {
    // pathContainsAfter, not pathContains: the order is the rule.
    assert!(is_machine_trash_directory(Path::new(
        "/home/ada/.var/x/cache"
    )));
    assert!(!is_machine_trash_directory(Path::new(
        "/home/ada/cache/x/.var"
    )));
}

#[test]
fn the_named_machine_caches_are_trash_anywhere() {
    for name in [
        "GPUCache",
        "Code Cache",
        "CacheStorage",
        "Service Worker",
        "blob_storage",
        "DawnGraphiteCache",
        "DawnWebGPUCache",
        "mesa_shader_cache",
        "shader_cache",
        "ShaderCache",
        "GrShaderCache",
        "Crashpad",
        "crashpad",
    ] {
        let path = format!("/anywhere/at/all/{name}");
        assert!(
            is_machine_trash_directory(Path::new(&path)),
            "{name} should be machine trash"
        );
    }
}

#[test]
fn a_cachedir_tag_needs_the_signature_and_not_just_the_name() {
    // A file merely named CACHEDIR.TAG is not one, and treating it as one
    // would drop a directory somebody wanted indexed.
    let path = Path::new("/home/ada/build/CACHEDIR.TAG");
    assert!(is_cachedir_tag(path, CACHEDIR_TAG_SIGNATURE.as_bytes()));
    assert!(!is_cachedir_tag(path, b"something else entirely"));
    assert!(!is_cachedir_tag(
        Path::new("/home/ada/CACHEDIR.TAG.bak"),
        CACHEDIR_TAG_SIGNATURE.as_bytes()
    ));
}

#[test]
fn the_cachedir_signature_is_the_one_from_the_spec() {
    assert_eq!(
        CACHEDIR_TAG_SIGNATURE,
        "Signature: 8a477f597d28d172789f06886806bc55"
    );
    assert!(has_cachedir_tag_signature(
        format!("{CACHEDIR_TAG_SIGNATURE}\n# more text").as_bytes()
    ));
    assert!(!has_cachedir_tag_signature(b"Signature: "));
}

#[test]
fn the_basename_is_taken_after_the_last_separator() {
    assert_eq!(basename("/home/ada/notes.txt"), "notes.txt");
    assert_eq!(basename("notes.txt"), "notes.txt");
    assert_eq!(
        basename("/home/ada/"),
        "",
        "a string operation, so a trailing slash gives an empty name"
    );
}

#[test]
fn a_name_exclusion_holds_with_ignore_files_configured() {
    // The cheap name checks run before the disk-touching ignore walk by
    // construction (`policy() && !is_ignored`), so configuring ignore files
    // cannot resurrect an excluded name.
    let mut configured = walk();
    configured.set_ignore_files(vec![".gitignore".to_owned()]);
    assert_eq!(configured.ignore_files().len(), 1);
    assert!(!visit(&configured, "/home/ada/project/node_modules", true));
}

#[test]
fn build_output_is_excluded_only_beside_its_manifest() {
    // `target` is an ordinary word: the folder goes only when a Cargo.toml or
    // pom.xml says a build tool wrote it.
    let tree = tempfile::tempdir().expect("temporary tree");
    for (directory, manifests) in BUILD_OUTPUT_DIRECTORIES {
        for manifest in *manifests {
            let project = tree.path().join(format!("{directory}-{manifest}"));
            std::fs::create_dir_all(project.join(directory)).expect("output dir");
            std::fs::write(project.join(manifest), "").expect("manifest");
            let output = project.join(directory);
            assert!(is_build_output_directory(&output), "{}", output.display());
            assert!(!walk().should_visit(&output, false, true));
        }
    }

    let plain = tree.path().join("archery/target");
    std::fs::create_dir_all(&plain).expect("plain dir");
    assert!(!is_build_output_directory(&plain));
    assert!(walk().should_visit(&plain, false, true));

    // A manifest for another tool does not make the name build output.
    let mismatched = tree.path().join("mixed");
    std::fs::create_dir_all(mismatched.join("build")).expect("build dir");
    std::fs::write(mismatched.join("Cargo.toml"), "").expect("manifest");
    assert!(!is_build_output_directory(&mismatched.join("build")));
}
