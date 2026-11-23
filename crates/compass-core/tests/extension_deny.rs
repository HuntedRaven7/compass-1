//! A denied extension is gone from search, from deep-link lookup and from a
//! rescan — the three ways a blocked command could otherwise come back.

use compass_core::AppIndex;
use compass_core::extension_deny::{DenyRule, ExtensionDeny};

fn index_with_github_extension() -> (tempfile::TempDir, AppIndex) {
    let apps = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let ext = installed.path().join("store.raycast.github");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("package.json"),
        r#"{"name": "github", "title": "GitHub", "author": "thomaslombart", "owner": "raycast",
            "commands": [{"name": "my-pull-requests", "title": "My Pull Requests", "mode": "view"}]}"#,
    )
    .unwrap();
    let index = AppIndex::builder()
        .dir(apps.path())
        .extension_dirs([installed.path()])
        .build();
    (installed, index)
}

#[test]
fn a_denied_store_disappears_from_search_and_deep_links() {
    let (_installed, mut index) = index_with_github_extension();
    assert!(
        index
            .extension_by_link("raycast", "github", "my-pull-requests")
            .is_some()
    );

    index.set_deny(ExtensionDeny::new(&[DenyRule {
        store: Some("raycast".into()),
        reason: Some("not on this image".into()),
        ..DenyRule::default()
    }]));

    assert!(index.extensions().is_empty());
    assert!(
        index
            .extension_by_link("raycast", "github", "my-pull-requests")
            .is_none()
    );
    assert!(
        !index
            .search_root_all("My Pull Requests", None)
            .iter()
            .any(|hit| matches!(hit, compass_core::RootHit::Extension { .. })),
        "the command's root row is gone too"
    );

    // A directory change must not bring it back: the policy lives on the index.
    index.rescan_extensions();
    assert!(index.extensions().is_empty());
}

#[test]
fn a_denied_author_matches_owner_and_keeps_other_extensions() {
    let (_installed, mut index) = index_with_github_extension();
    index.set_deny(ExtensionDeny::new(&[DenyRule {
        author: Some("raycast".into()),
        ..DenyRule::default()
    }]));
    assert!(index.extensions().is_empty());

    let (_installed, mut index) = index_with_github_extension();
    index.set_deny(ExtensionDeny::new(&[DenyRule {
        author: Some("someone-else".into()),
        ..DenyRule::default()
    }]));
    assert_eq!(
        index.extensions().len(),
        1,
        "other publishers are unaffected"
    );
}

#[test]
fn removing_the_deny_rule_brings_the_extension_back() {
    let (_installed, mut index) = index_with_github_extension();
    index.set_deny(ExtensionDeny::new(&[DenyRule {
        id: Some("store.raycast.*".into()),
        ..DenyRule::default()
    }]));
    assert!(index.extensions().is_empty());
    index.set_deny(ExtensionDeny::default());
    assert_eq!(index.extensions().len(), 1);
}
