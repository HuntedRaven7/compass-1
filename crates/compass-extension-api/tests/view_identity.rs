//! Node identity and diffing: the properties a front end relies on to patch rather than
//! rebuild.

mod common;

use common::*;
use compass_extension_api::*;

fn ids(tree: &ViewTree) -> Vec<NodeId> {
    tree.nodes().iter().map(|n| n.id).collect()
}

#[test]
fn the_same_logical_tree_renders_to_the_same_ids() {
    for view in every_view() {
        let a = ViewTree::new(view.clone());
        let b = ViewTree::new(view);
        assert_eq!(ids(&a), ids(&b));
        assert_eq!(a.nodes(), b.nodes());
        assert!(ViewDiff::between(&a, &b).is_empty());
    }
}

#[test]
fn identity_is_stable_across_a_serialisation_round_trip() {
    let a = ViewTree::new(View::List(list_view()));
    let json = serde_json::to_string(&a).unwrap();
    let b: ViewTree = serde_json::from_str(&json).unwrap();
    // Re-assigning identity on the decoded tree reproduces the same ids: nothing about
    // identity depends on state held by the process that first rendered it.
    let c = ViewTree::new(b.into_root());
    assert_eq!(ids(&a), ids(&c));
}

#[test]
fn every_id_in_a_tree_is_unique() {
    for view in every_view() {
        let tree = ViewTree::new(view);
        let mut seen = std::collections::BTreeSet::new();
        for node in tree.nodes() {
            assert!(
                seen.insert(node.id),
                "id {} used twice ({})",
                node.id,
                node.kind
            );
        }
        assert!(seen.len() > 1);
    }
}

#[test]
fn changing_a_title_keeps_ids_and_produces_an_actionable_diff() {
    let before = ViewTree::new(View::List(list_view()));
    let mut changed = list_view();
    changed.sections[0].items[0].title = "Crash on resume".into();
    let after = ViewTree::new(View::List(changed));

    // Identity is content-independent, so nothing is added or removed.
    assert_eq!(ids(&before), ids(&after));

    let diff = ViewDiff::between(&before, &after);
    assert_eq!(diff.len(), 1, "{:?}", diff.changes);
    match &diff.changes[0] {
        NodeChange::Updated { id, kind } => {
            assert_eq!(kind, "list_item");
            assert_eq!(*id, after.nodes()[2].id);
        }
        other => panic!("expected a single update, got {other:?}"),
    }
}

#[test]
fn a_keyed_item_keeps_its_id_when_its_neighbours_move() {
    let mut before_view = list_view();
    before_view.sections[0].items.truncate(2);
    let before = ViewTree::new(View::List(before_view.clone()));
    let kept = before
        .nodes()
        .iter()
        .find(|n| n.kind == "list_item")
        .unwrap()
        .id;

    // Insert a new keyed row at the head; the existing keyed rows must not move.
    let mut after_view = before_view;
    after_view.sections[0]
        .items
        .insert(0, ListItem::new("Brand new").with_key("COMP-0"));
    let after = ViewTree::new(View::List(after_view));

    let after_ids: Vec<NodeId> = after.nodes().iter().map(|n| n.id).collect();
    assert!(
        after_ids.contains(&kept),
        "a keyed row lost its identity when a sibling appeared"
    );

    let diff = ViewDiff::between(&before, &after);
    assert!(
        diff.changes
            .iter()
            .all(|c| matches!(c, NodeChange::Added { .. })),
        "inserting a keyed row should only add nodes, got {:?}",
        diff.changes
    );
}

#[test]
fn an_unkeyed_item_is_identified_by_position() {
    // The documented cost of index keys: inserting before an unkeyed row shifts it.
    let mut a = ListView {
        sections: vec![ListSection::untitled([ListItem::new("b")])],
        ..ListView::default()
    };
    let before = ViewTree::new(View::List(a.clone()));
    a.sections[0].items.insert(0, ListItem::new("a"));
    let after = ViewTree::new(View::List(a));
    let diff = ViewDiff::between(&before, &after);
    assert!(
        diff.changes
            .iter()
            .any(|c| matches!(c, NodeChange::Added { .. })),
        "{:?}",
        diff.changes
    );
    assert!(
        diff.changes
            .iter()
            .any(|c| matches!(c, NodeChange::Updated { .. })),
        "the shifted row should read as an update, got {:?}",
        diff.changes
    );
}

#[test]
fn removing_an_item_reports_it_and_its_whole_subtree_as_removed() {
    let before = ViewTree::new(View::List(list_view()));
    let mut shorter = list_view();
    shorter.sections[0].items.pop();
    let after = ViewTree::new(View::List(shorter));

    let diff = ViewDiff::between(&before, &after);
    assert!(!diff.is_empty());
    assert!(
        diff.changes
            .iter()
            .all(|c| matches!(c, NodeChange::Removed { .. }))
    );
    assert!(
        diff.changes
            .iter()
            .any(|c| matches!(c, NodeChange::Removed { kind, .. } if kind == "list_item"))
    );
}

#[test]
fn changing_the_view_shape_replaces_the_whole_tree() {
    let list = ViewTree::new(View::List(list_view()));
    let form = ViewTree::new(View::Form(form_view()));
    let diff = ViewDiff::between(&list, &form);
    assert!(
        diff.changes
            .iter()
            .any(|c| matches!(c, NodeChange::Added { .. }))
    );
    assert!(
        diff.changes
            .iter()
            .any(|c| matches!(c, NodeChange::Removed { .. }))
    );
    assert!(
        !diff
            .changes
            .iter()
            .any(|c| matches!(c, NodeChange::Updated { .. }))
    );
}

#[test]
fn a_form_field_is_identified_by_its_name_not_its_position() {
    let before = ViewTree::new(View::Form(form_view()));
    let field_ids: Vec<NodeId> = before
        .nodes()
        .iter()
        .filter(|n| n.kind == "form_field")
        .map(|n| n.id)
        .collect();

    let mut reordered = form_view();
    reordered.items.swap(2, 3);
    let after = ViewTree::new(View::Form(reordered));
    let after_field_ids: std::collections::BTreeSet<NodeId> = after
        .nodes()
        .iter()
        .filter(|n| n.kind == "form_field")
        .map(|n| n.id)
        .collect();

    for id in field_ids {
        assert!(
            after_field_ids.contains(&id),
            "field {id} lost identity when reordered"
        );
    }
}

#[test]
fn sibling_slots_do_not_collide() {
    // A view's own action panel and its first item's action panel are different nodes
    // even though both are "the actions of something".
    let tree = ViewTree::new(View::List(list_view()));
    let panels: Vec<NodeId> = tree
        .nodes()
        .iter()
        .filter(|n| n.kind == "action_panel")
        .map(|n| n.id)
        .collect();
    let unique: std::collections::BTreeSet<_> = panels.iter().collect();
    assert_eq!(panels.len(), unique.len());
    assert!(panels.len() >= 3);
}

#[test]
fn node_ids_render_readably() {
    let id = NodeId::ROOT.child_keyed("items", Some("x"), 0);
    let text = id.to_string();
    assert!(text.starts_with("n:"), "{text}");
    assert_eq!(text.len(), 18);
    assert_eq!(NodeId::from_raw(id.raw()), id);
}

#[test]
fn keys_and_indices_are_different_namespaces() {
    let by_key = NodeId::ROOT.child("items", NodeKey::Stable("0"));
    let by_index = NodeId::ROOT.child("items", NodeKey::Index(0));
    assert_ne!(by_key, by_index);
    assert_ne!(
        NodeId::ROOT.child("items", NodeKey::Stable("ab")),
        NodeId::ROOT.child("itemsa", NodeKey::Stable("b")),
        "slot and key must be framed so they cannot run together"
    );
}

// ---------------------------------------------------------------------------
// duplicate keys
//
// Nothing stops an extension emitting the same key on two siblings. Until the ordinal
// fallback in `Slot` these tests all failed: the two rows derived one id, and the diff
// -- which matches by id -- attributed one row's fields to the other. Found by the
// `arbitrary_titles_and_keys_produce_unique_stable_ids` property test, which shrank to
// two items whose only shared feature was an empty key.
// ---------------------------------------------------------------------------

fn two_rows_sharing_a_key(key: &str) -> View {
    View::List(ListView {
        sections: vec![ListSection::untitled(vec![
            ListItem::new("alpha").with_key(key),
            ListItem::new("beta").with_key(key),
        ])],
        ..ListView::default()
    })
}

#[test]
fn siblings_sharing_a_key_still_get_distinct_ids() {
    for key in ["", "row", "\u{e000}"] {
        let tree = ViewTree::new(two_rows_sharing_a_key(key));
        let mut seen = std::collections::BTreeSet::new();
        for node in tree.nodes() {
            assert!(
                seen.insert(node.id),
                "key {key:?}: id {} used twice ({})",
                node.id,
                node.kind
            );
        }
    }
}

#[test]
fn a_tree_with_duplicate_keys_does_not_differ_from_itself() {
    // The symptom that made the collision visible: two distinct rows sharing an id meant
    // one was compared against the other's fingerprint, so a tree reported a change
    // against an identical copy of itself and the UI repainted the wrong row.
    let a = ViewTree::new(two_rows_sharing_a_key("row"));
    let b = ViewTree::new(two_rows_sharing_a_key("row"));
    assert_eq!(a.nodes(), b.nodes());
    assert!(
        ViewDiff::between(&a, &b).is_empty(),
        "identical trees differ: {:?}",
        ViewDiff::between(&a, &b).changes
    );
}

#[test]
fn the_first_claimant_of_a_key_keeps_the_derived_id() {
    // The demotion has to fall on the *later* sibling, or inserting a duplicate would
    // move an existing row's id and cost the UI its selection.
    let unique = ViewTree::new(View::List(ListView {
        sections: vec![ListSection::untitled(vec![
            ListItem::new("alpha").with_key("row"),
        ])],
        ..ListView::default()
    }));
    let duplicated = ViewTree::new(two_rows_sharing_a_key("row"));
    assert_eq!(unique.nodes()[2].id, duplicated.nodes()[2].id);
}

fn named_field(name: &str, title: &str) -> FormField {
    FormField {
        id: NodeId::ROOT,
        name: name.into(),
        title: Some(title.into()),
        error: None,
        info: None,
        autofocus: false,
        value: None,
        echo: None,
        on_change: None,
        kind: FieldKind::Text { placeholder: None },
    }
}

#[test]
fn form_fields_sharing_a_name_still_get_distinct_ids() {
    let tree = ViewTree::new(View::Form(FormView {
        items: vec![
            FormItem::Field(Box::new(named_field("dupe", "First"))),
            FormItem::Field(Box::new(named_field("dupe", "Second"))),
        ],
        ..FormView::default()
    }));
    let ids = ids(&tree);
    let unique: std::collections::BTreeSet<_> = ids.iter().collect();
    assert_eq!(ids.len(), unique.len(), "form field ids collided: {ids:?}");
}

#[test]
fn decoding_a_tree_re_derives_its_ids() {
    // `ViewTree`'s fields carry ids, so a payload can name any id it likes. Deserialising
    // re-runs assignment, which is what keeps "every id in a ViewTree was derived by this
    // crate" true rather than merely conventional -- a forged duplicate would otherwise
    // reach the diff and misattribute one node's fields to another.
    let honest = ViewTree::new(View::List(list_view()));
    let json = serde_json::to_string(&honest).unwrap();

    let forged = json.replace(
        &honest.nodes()[2].id.raw().to_string(),
        &honest.nodes()[3].id.raw().to_string(),
    );
    assert_ne!(forged, json, "the forgery did not change the payload");

    let decoded: ViewTree = serde_json::from_str(&forged).unwrap();
    assert_eq!(
        ids(&decoded),
        ids(&honest),
        "decoding trusted the ids in the payload"
    );
}

#[test]
fn decoding_round_trips_a_tree_unchanged() {
    for view in every_view() {
        let a = ViewTree::new(view);
        let b: ViewTree = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, b);
    }
}
