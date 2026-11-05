//! Walking a directory tree, the way the file indexer walks it.
//!
//! Behaviour tests over real temporary trees: the traversal machinery is the
//! `ignore` crate's, so what is pinned here is vicinae's policy on top of it —
//! and the two places the crate's real gitignore semantics deliberately move
//! past the old filename-only hack (directory-scoped patterns, anchoring,
//! negation).

mod support;

use std::path::{Path, PathBuf};

use compass_core::entry_filter::CACHEDIR_TAG_SIGNATURE;
use compass_core::file_walk::{IndexWalk, WalkEntry};
use support::{mkdir, symlink, write};
use tempfile::TempDir;

/// A walk with nothing configured, like the C++ with no setters called.
fn walk() -> IndexWalk {
    IndexWalk::new(None)
}

/// The collected entries as root-relative paths, sorted: the crate yields in
/// directory order, which is not a contract, so order is never asserted.
fn rel(root: &Path, entries: &[WalkEntry]) -> Vec<String> {
    let mut paths: Vec<String> = entries
        .iter()
        .map(|entry| {
            entry
                .path
                .strip_prefix(root)
                .expect("entry under the root")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    paths.sort();
    paths
}

fn fixture() -> TempDir {
    tempfile::tempdir().expect("temporary tree")
}

#[test]
fn the_root_is_not_among_the_entries() {
    let dir = fixture();
    write(dir.path(), "a.txt", "notes");

    assert_eq!(rel(dir.path(), &walk().collect(dir.path())), ["a.txt"]);
}

#[test]
fn a_root_that_is_a_file_yields_nothing() {
    // `if (!fs::is_directory(root, ec)) return;` — a file yields nothing
    // rather than reporting itself as its own single entry.
    let dir = fixture();
    write(dir.path(), "a.txt", "notes");

    assert!(walk().collect(&dir.path().join("a.txt")).is_empty());
}

#[test]
fn a_root_that_does_not_exist_yields_nothing() {
    let dir = fixture();

    assert!(walk().collect(&dir.path().join("missing")).is_empty());
}

#[test]
fn a_subdirectory_is_both_reported_and_descended_into() {
    let dir = fixture();
    mkdir(dir.path(), "sub");
    write(dir.path(), "sub/a.txt", "notes");

    assert_eq!(
        rel(dir.path(), &walk().collect(dir.path())),
        ["sub", "sub/a.txt"]
    );
}

#[test]
fn a_walk_that_is_not_recursive_still_reports_the_directories() {
    let dir = fixture();
    mkdir(dir.path(), "sub");
    write(dir.path(), "sub/a.txt", "notes");

    let found = walk().recursive(false).collect(dir.path());

    // The directory is an entry like any other; what changes is that nothing
    // below it is reached.
    assert_eq!(rel(dir.path(), &found), ["sub"]);
}

#[test]
fn a_max_depth_of_zero_reaches_the_roots_own_entries_only() {
    let dir = fixture();
    mkdir(dir.path(), "sub");
    write(dir.path(), "sub/a.txt", "notes");

    let found = walk().max_depth(Some(0)).collect(dir.path());

    assert_eq!(rel(dir.path(), &found), ["sub"]);
}

#[test]
fn a_max_depth_of_one_reaches_one_level_deeper_than_it_reads() {
    // The limit bounds what is *entered*, and entering a directory reports its
    // contents — so a limit of 1 yields entries at depth 2. Off by one against
    // the obvious reading, and it is the shipped behaviour, mapped onto the
    // crate's limit plus one.
    let dir = fixture();
    mkdir(dir.path(), "sub/deep");
    write(dir.path(), "sub/deep/a.txt", "notes");

    let found = walk().max_depth(Some(1)).collect(dir.path());

    assert_eq!(rel(dir.path(), &found), ["sub", "sub/deep"]);
}

#[test]
fn a_symlink_is_never_visited_and_never_descended_into() {
    let dir = fixture();
    write(dir.path(), "a.txt", "notes");
    mkdir(dir.path(), "real");
    write(dir.path(), "real/inside.txt", "notes");
    symlink(dir.path(), "a.txt", "file-link");
    symlink(dir.path(), "real", "dir-link");

    let found = rel(dir.path(), &walk().collect(dir.path()));

    assert_eq!(found, ["a.txt", "real", "real/inside.txt"]);
}

#[test]
fn a_directory_holding_a_cachedir_tag_is_skipped_whole() {
    let dir = fixture();
    mkdir(dir.path(), "cache");
    write(
        dir.path(),
        "cache/CACHEDIR.TAG",
        &format!("{CACHEDIR_TAG_SIGNATURE}\n# created by something cached"),
    );
    write(dir.path(), "cache/blob", "cached data");

    let found = walk().collect(dir.path());

    // The directory itself is still reported by its parent — what is skipped
    // is everything inside it.
    assert_eq!(rel(dir.path(), &found), ["cache"]);
}

#[test]
fn a_cachedir_tag_without_the_signature_is_just_a_file() {
    // A file merely *named* CACHEDIR.TAG is not one, and treating it as one
    // would drop a directory somebody wanted indexed.
    let dir = fixture();
    mkdir(dir.path(), "data");
    write(dir.path(), "data/CACHEDIR.TAG", "something else entirely");
    write(dir.path(), "data/blob", "real data");

    let found = walk().collect(dir.path());

    assert_eq!(
        rel(dir.path(), &found),
        ["data", "data/CACHEDIR.TAG", "data/blob"]
    );
}

#[test]
fn a_cache_directory_does_not_silence_its_siblings() {
    let dir = fixture();
    mkdir(dir.path(), "keep");
    write(dir.path(), "keep/a.txt", "notes");
    mkdir(dir.path(), "cache");
    write(dir.path(), "cache/CACHEDIR.TAG", CACHEDIR_TAG_SIGNATURE);

    let found = walk().collect(dir.path());

    assert_eq!(rel(dir.path(), &found), ["cache", "keep", "keep/a.txt"]);
}

#[test]
fn an_excluded_path_is_not_visited_and_not_descended_into() {
    let dir = fixture();
    mkdir(dir.path(), "skip");
    write(dir.path(), "skip/secret", "hidden");
    write(dir.path(), "a.txt", "notes");

    let mut index = walk();
    index.set_excluded_paths(vec![dir.path().join("skip")]);

    assert_eq!(rel(dir.path(), &index.collect(dir.path())), ["a.txt"]);
}

#[test]
fn an_excluded_filename_is_not_visited() {
    let dir = fixture();
    mkdir(dir.path(), "target");
    write(dir.path(), "target/out", "build output");
    write(dir.path(), "a.txt", "notes");

    let mut index = walk();
    index.set_excluded_filenames(vec!["target".to_owned()]);

    assert_eq!(rel(dir.path(), &index.collect(dir.path())), ["a.txt"]);
}

#[test]
fn a_builtin_excluded_filename_needs_no_configuration() {
    // node_modules is on the shipped list, not a user setting.
    let dir = fixture();
    mkdir(dir.path(), "node_modules");
    write(dir.path(), "node_modules/dep.js", "vendored");
    write(dir.path(), "a.txt", "notes");

    assert_eq!(rel(dir.path(), &walk().collect(dir.path())), ["a.txt"]);
}

#[test]
fn build_output_beside_its_manifest_is_not_walked() {
    let dir = fixture();
    write(dir.path(), "crate/Cargo.toml", "[package]");
    write(dir.path(), "crate/src/main.rs", "fn main() {}");
    write(dir.path(), "crate/target/debug/app", "binary");
    write(dir.path(), "archery/target/scores.txt", "notes");

    assert_eq!(
        rel(dir.path(), &walk().collect(dir.path())),
        [
            "archery",
            "archery/target",
            "archery/target/scores.txt",
            "crate",
            "crate/Cargo.toml",
            "crate/src",
            "crate/src/main.rs"
        ]
    );
}

#[test]
fn an_excluded_cache_directory_is_not_reported_either() {
    // The policy decides before the tag does, as the C++ only lists
    // directories that passed `shouldVisit`.
    let dir = fixture();
    write(
        dir.path(),
        "node_modules/CACHEDIR.TAG",
        CACHEDIR_TAG_SIGNATURE,
    );
    write(dir.path(), "node_modules/dep.js", "vendored");
    write(dir.path(), "a.txt", "notes");

    assert_eq!(rel(dir.path(), &walk().collect(dir.path())), ["a.txt"]);
}

#[test]
fn hidden_paths_are_visited_until_hidden_paths_are_ignored() {
    let dir = fixture();
    write(dir.path(), ".bashrc", "aliases");
    mkdir(dir.path(), ".config/app");
    write(dir.path(), ".config/app/settings.json", "{}");
    write(dir.path(), "a.txt", "notes");

    assert_eq!(
        rel(dir.path(), &walk().collect(dir.path())),
        [
            ".bashrc",
            ".config",
            ".config/app",
            ".config/app/settings.json",
            "a.txt"
        ],
        "hidden trees are walked by default"
    );

    let mut index = walk();
    index.set_ignore_hidden_paths(true);

    // A hidden ancestor hides everything below it, not just itself.
    assert_eq!(rel(dir.path(), &index.collect(dir.path())), ["a.txt"]);
}

#[test]
fn a_noindex_directory_prunes_its_whole_subtree() {
    // The Spotlight convention, honoured by many apps.
    let dir = fixture();
    mkdir(dir.path(), "Big.noindex");
    write(dir.path(), "Big.noindex/inside.txt", "not for the index");
    write(dir.path(), "data.noindex", "also not for the index");
    write(dir.path(), "a.txt", "notes");

    assert_eq!(rel(dir.path(), &walk().collect(dir.path())), ["a.txt"]);
}

#[test]
fn a_gitignore_beside_the_file_hides_it() {
    let dir = fixture();
    write(dir.path(), "p/notes.log", "logs");
    write(dir.path(), "p/notes.txt", "notes");
    write(dir.path(), "p/.gitignore", "*.log\n");

    let mut index = walk();
    index.set_ignore_files(vec![".gitignore".to_owned()]);

    assert_eq!(
        rel(dir.path(), &index.collect(dir.path())),
        ["p", "p/.gitignore", "p/notes.txt"]
    );
}

#[test]
fn a_gitignore_at_the_top_of_a_tree_applies_far_below_it() {
    let dir = fixture();
    mkdir(dir.path(), "repo/a/b/c");
    write(dir.path(), "repo/a/b/c/out.o", "object");
    write(dir.path(), "repo/a/b/c/main.rs", "source");
    write(dir.path(), "repo/.gitignore", "*.o\n");

    let mut index = walk();
    index.set_ignore_files(vec![".gitignore".to_owned()]);

    let found = rel(dir.path(), &index.collect(dir.path()));
    assert!(!found.iter().any(|path| path.ends_with("out.o")));
    assert!(found.iter().any(|path| path.ends_with("main.rs")));
}

#[test]
fn an_unconfigured_gitignore_is_not_honoured() {
    // Nothing configures an ignore list by default, so ignore files on disk
    // are paper until somebody asks for them.
    let dir = fixture();
    write(dir.path(), ".gitignore", "*.log\n");
    write(dir.path(), "notes.log", "logs");

    assert_eq!(
        rel(dir.path(), &walk().collect(dir.path())),
        [".gitignore", "notes.log"]
    );
}

#[test]
fn several_ignore_file_names_can_be_honoured() {
    let dir = fixture();
    write(dir.path(), "x.tmp", "temporary");
    write(dir.path(), "x.bak", "backup");
    write(dir.path(), "a.txt", "notes");
    write(dir.path(), ".ignore", "*.tmp\n");
    write(dir.path(), ".vicinaeignore", "*.bak\n");

    let mut index = walk();
    index.set_ignore_files(vec![".ignore".to_owned(), ".vicinaeignore".to_owned()]);
    assert_eq!(index.ignore_files().len(), 2);

    assert_eq!(
        rel(dir.path(), &index.collect(dir.path())),
        [".ignore", ".vicinaeignore", "a.txt"]
    );
}

#[test]
fn a_dot_ignore_file_is_only_honoured_when_configured() {
    let dir = fixture();
    write(dir.path(), ".ignore", "*.tmp\n");
    write(dir.path(), "x.tmp", "temporary");

    // Asked for .gitignore only: the .ignore beside it is not read.
    let mut git_only = walk();
    git_only.set_ignore_files(vec![".gitignore".to_owned()]);
    assert_eq!(
        rel(dir.path(), &git_only.collect(dir.path())),
        [".ignore", "x.tmp"]
    );

    let mut index = walk();
    index.set_ignore_files(vec![".ignore".to_owned()]);
    assert_eq!(rel(dir.path(), &index.collect(dir.path())), [".ignore"]);
}

#[test]
fn ignore_patterns_are_scoped_to_their_directory() {
    // Past the old filename-only hack: build/*.o keeps build/main.o out while
    // another directory's main.o stays, which the C++ could never express.
    let dir = fixture();
    mkdir(dir.path(), "build");
    mkdir(dir.path(), "other");
    write(dir.path(), "build/main.o", "object");
    write(dir.path(), "other/main.o", "another object");
    write(dir.path(), ".gitignore", "build/*.o\n");

    let mut index = walk();
    index.set_ignore_files(vec![".gitignore".to_owned()]);

    let found = rel(dir.path(), &index.collect(dir.path()));
    assert!(!found.contains(&"build/main.o".to_owned()));
    assert!(found.contains(&"other/main.o".to_owned()));
}

#[test]
fn a_leading_slash_anchors_the_pattern() {
    // /target means the target beside the ignore file — not a target anywhere
    // in the tree, which is what the old stripped-slash hack matched.
    let dir = fixture();
    mkdir(dir.path(), "target");
    write(dir.path(), "target/out", "build output");
    mkdir(dir.path(), "a/target");
    write(dir.path(), "a/target/notes.txt", "notes");
    write(dir.path(), ".gitignore", "/target\n");

    let mut index = walk();
    index.set_ignore_files(vec![".gitignore".to_owned()]);

    let found = rel(dir.path(), &index.collect(dir.path()));
    assert!(
        !found
            .iter()
            .any(|path| path == "target" || path.starts_with("target/"))
    );
    assert!(found.contains(&"a/target/notes.txt".to_owned()));
}

#[test]
fn a_negation_brings_a_file_back() {
    // ! patterns are honoured now; the old matcher read them as literals that
    // could never match a real file.
    let dir = fixture();
    write(dir.path(), "notes.log", "logs");
    write(dir.path(), "keep.log", "keeper");
    write(dir.path(), ".gitignore", "*.log\n!keep.log\n");

    let mut index = walk();
    index.set_ignore_files(vec![".gitignore".to_owned()]);

    let found = rel(dir.path(), &index.collect(dir.path()));
    assert!(!found.contains(&"notes.log".to_owned()));
    assert!(found.contains(&"keep.log".to_owned()));
}

#[test]
fn should_visit_honours_an_ignore_file_on_disk() {
    // The watch enumeration asks the policy directly instead of running the
    // walker, so this pins that wiring — not just the walker's.
    let dir = fixture();
    write(dir.path(), ".gitignore", "*.log\n");

    let mut index = walk();
    index.set_ignore_files(vec![".gitignore".to_owned()]);

    assert!(!index.should_visit(&dir.path().join("notes.log"), false, false));
    assert!(index.should_visit(&dir.path().join("notes.txt"), false, false));
}

#[test]
fn a_nearer_whitelist_overrules_a_farther_ignore() {
    // Git stacks ignore files: the nearest level that decides wins, whether
    // it ignores or whitelists.
    let dir = fixture();
    write(dir.path(), ".gitignore", "*.log\n");
    write(dir.path(), "sub/.gitignore", "!keep.log\n");

    let mut index = walk();
    index.set_ignore_files(vec![".gitignore".to_owned()]);

    assert!(!index.should_visit(&dir.path().join("sub/notes.log"), false, false));
    assert!(index.should_visit(&dir.path().join("sub/keep.log"), false, false));
}

#[test]
fn an_empty_tree_yields_nothing_rather_than_looping() {
    let dir = fixture();
    mkdir(dir.path(), "empty");

    assert_eq!(rel(dir.path(), &walk().collect(dir.path())), ["empty"]);
}

#[test]
fn collecting_gathers_everything_the_walk_visits() {
    // The two entry points agree; one is not a filtered version of the other.
    let dir = fixture();
    mkdir(dir.path(), "sub");
    write(dir.path(), "sub/a.txt", "notes");
    write(dir.path(), "b.txt", "notes");

    let mut via_callback = Vec::new();
    walk().walk(dir.path(), |entry| via_callback.push(entry.clone()));

    assert_eq!(via_callback, walk().collect(dir.path()));
    assert!(!via_callback.iter().any(|entry| entry.is_symlink));
}

#[test]
fn walk_entries_know_files_from_directories() {
    let dir = fixture();
    mkdir(dir.path(), "sub");
    write(dir.path(), "sub/a.txt", "notes");

    let found = walk().collect(dir.path());
    let sub = found
        .iter()
        .find(|entry| entry.path == dir.path().join("sub"))
        .expect("the directory itself is reported");
    assert!(sub.is_directory);
    assert!(
        !found
            .iter()
            .find(|entry| entry.path == dir.path().join("sub/a.txt"))
            .expect("the file is reported")
            .is_directory
    );
    assert_eq!(
        WalkEntry::file(PathBuf::from("/x")),
        WalkEntry {
            path: PathBuf::from("/x"),
            is_directory: false,
            is_symlink: false,
        }
    );
}

#[test]
fn a_stopped_walk_visits_nothing() {
    // `FileSystemWalker::stop` breaks the C++ loop; here the loop breaks at
    // the next entry, so stopping first visits nothing at all.
    let dir = fixture();
    write(dir.path(), "a.txt", "notes");

    let walk = walk();
    walk.stop();
    assert!(walk.collect(dir.path()).is_empty());
}
