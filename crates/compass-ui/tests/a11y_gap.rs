//! #118 — the launcher's accessibility tree, the Orca gate that proves it,
//! and the part of the gap that is still open.
//!
//! The tree exists (ADR-0022 §1): `compass-ui` describes the window from its
//! state (`compass_ui::semantics`) and `compass-a11y-linux` serves it on
//! AT-SPI. Orca reads and operates it in a real Mutter session: the Orca gate
//! (`scripts/tier2/orca.sh`, #305) runs per pull request against the Flatpak
//! the pull request built. What is still open is recorded in ADR-0016's
//! "Where it stands" and stated by `doctor`; these tests keep the three in
//! step, so none of them can quietly claim more than the others.

use std::path::Path;

use accesskit::Role;
use compass_core::AppIndex;
use compass_ui::LauncherApp;
use compass_ui::semantics::FocusTracker;

fn workspace_file(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path:?}: {error}"))
}

#[test]
fn adr_0016_records_the_tree_the_orca_gate_and_what_is_still_open() {
    let adr = workspace_file("docs/rust-engine/adr/0016-a11y-gap.md");
    for needle in [
        "AccessKit",
        "compass-a11y-linux",
        "scripts/tier2/orca.sh",
        "ORCA-CHECKLIST.md",
        "#304",
        "#118",
    ] {
        assert!(adr.contains(needle), "ADR-0016 no longer mentions {needle}");
    }
}

#[test]
fn the_orca_gate_runs_in_ci_against_the_flatpak() {
    // The claim ADR-0016, CUTOVER and the doctor now make -- Orca reads the
    // launcher -- rests on this step. If it leaves CI, the claim goes with it.
    let tier2 = workspace_file(".github/workflows/tier2.yaml");
    assert!(
        tier2.contains("scripts/tier2/orca.sh"),
        "tier2.yaml no longer runs the Orca gate — ADR-0016, CUTOVER.md and the doctor's a11y check say Orca is verified"
    );
    let manifest = workspace_file("packaging/flatpak/org.tunaos.compass.yaml");
    assert!(
        manifest.contains("- --talk-name=org.a11y.Bus"),
        "the Flatpak lost org.a11y.Bus: inside the sandbox the launcher never learns a screen reader is on"
    );
}

#[test]
fn the_adapter_is_in_the_lock() {
    // The condition #118 found was zero occurrences. If the adapter leaves
    // the workspace, the tree goes with it: ADR-0016, the doctor and this
    // test must say so, not just this assertion change.
    let lock = workspace_file("Cargo.lock");
    assert!(
        lock.contains("name = \"accesskit_unix\""),
        "Cargo.lock no longer has accesskit_unix — update ADR-0016 and the doctor's a11y check with it"
    );
}

#[test]
fn a_fresh_launcher_describes_its_window_and_search_field() {
    let app = LauncherApp::with_index(AppIndex::default());
    let semantics = app.semantics();
    let update = semantics.tree_update(FocusTracker::new().focus(&semantics));
    let root = update.tree.as_ref().expect("a whole tree").root;
    let (_, window) = update
        .nodes
        .iter()
        .find(|(id, _)| *id == root)
        .expect("the root is in the tree");
    assert_eq!(window.role(), Role::Window);
    assert_eq!(window.label(), Some("Compass"));
    assert!(
        update
            .nodes
            .iter()
            .any(|(id, node)| node.role() == Role::SearchInput && *id == update.focus),
        "the search field is there and has the focus"
    );
}
