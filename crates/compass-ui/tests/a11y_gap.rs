//! #118 — the launcher's accessibility tree, and the part of the gap that is
//! still open.
//!
//! The tree exists (ADR-0022 §1): `compass-ui` describes the window from its
//! state (`compass_ui::semantics`) and `compass-a11y-linux` serves it on
//! AT-SPI, which `compass-a11y-linux/tests/private_bus.rs` reads back off a
//! real accessibility bus. What is still open is #305: no Orca session has
//! read the launcher. These tests keep that written down rather than silent;
//! #305 retires them, the doctor warning and the CUTOVER note together, once
//! Orca reads the launcher in a real session.

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
fn adr_0016_records_the_tree_and_what_is_still_open() {
    let adr = workspace_file("docs/rust-engine/adr/0016-a11y-gap.md");
    for needle in ["AccessKit", "compass-a11y-linux", "#305", "Orca", "#118"] {
        assert!(adr.contains(needle), "ADR-0016 no longer mentions {needle}");
    }
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
