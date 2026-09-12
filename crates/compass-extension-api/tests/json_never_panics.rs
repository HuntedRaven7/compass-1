//! Deserialising is a trust boundary: whatever arrives, this crate must return an error,
//! never unwind.
//!
//! Both halves matter. Arbitrary bytes are the easy case; structurally plausible JSON
//! with the wrong types in the right places is the one that finds real bugs, so the
//! second strategy builds documents that look like view trees.

mod common;

use compass_extension_api::*;
use proptest::prelude::*;

/// Where a counterexample gets written so it can be replayed.
///
/// proptest's default `SourceParallel` persistence looks for `lib.rs` or `main.rs` beside
/// the test file. An integration test under `tests/` has neither, so proptest prints
/// "failed to find lib.rs or main.rs" and *discards the seed*. A property that fails on
/// one seed in a few hundred is then unreplayable -- and that is precisely the failure
/// worth keeping, since it will not reproduce on the next run. Writing the seed to a
/// checked-in file turns each such find into a permanent regression case.
fn regressions(cases: u32, file: &'static str) -> ProptestConfig {
    ProptestConfig {
        cases,
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct(file),
        )),
        ..ProptestConfig::default()
    }
}

/// Deserialises `json` into every public root type. Any outcome is fine except a panic.
fn try_every_type(json: &str) {
    let _ = serde_json::from_str::<ViewTree>(json);
    let _ = serde_json::from_str::<View>(json);
    let _ = serde_json::from_str::<ListView>(json);
    let _ = serde_json::from_str::<GridView>(json);
    let _ = serde_json::from_str::<FormView>(json);
    let _ = serde_json::from_str::<Detail>(json);
    let _ = serde_json::from_str::<ActionPanel>(json);
    let _ = serde_json::from_str::<Action>(json);
    let _ = serde_json::from_str::<Shortcut>(json);
    let _ = serde_json::from_str::<Image>(json);
    let _ = serde_json::from_str::<FieldValue>(json);
    let _ = serde_json::from_str::<NodeId>(json);
    let _ = serde_json::from_str::<NodeSummary>(json);
    let _ = serde_json::from_str::<ViewDiff>(json);
    let _ = serde_json::from_str::<Capability>(json);
    let _ = serde_json::from_str::<CapabilityRegistry>(json);
    let _ = serde_json::from_str::<CapabilityState>(json);
    let _ = serde_json::from_str::<Denial>(json);
    let _ = serde_json::from_str::<ActionRequest>(json);
    let _ = serde_json::from_str::<ActionResponse>(json);
    let _ = serde_json::from_str::<ActionEffect>(json);
    let _ = serde_json::from_str::<DispatchError>(json);
}

/// Whatever survives parsing must also be usable: walking, indexing and diffing a tree
/// built from hostile input must not panic either.
fn exercise_anything_that_parsed(json: &str) {
    if let Ok(tree) = serde_json::from_str::<ViewTree>(json) {
        let nodes = tree.nodes();
        let actions = tree.actions();
        let index = ActionIndex::from_tree(&tree);
        for action in &actions {
            let _ = index.resolve(&action.handler);
        }
        let reassigned = ViewTree::new(tree.root().clone());
        let _ = ViewDiff::between(&tree, &reassigned);
        let _ = nodes.len();
    }
    if let Ok(registry) = serde_json::from_str::<CapabilityRegistry>(json) {
        let ids: Vec<ExtensionId> = registry.extensions().cloned().collect();
        for id in &ids {
            for cap in Capability::KNOWN {
                let _ = registry.check(id, cap);
            }
            let _ = registry.check(id, &Capability::new(""));
        }
    }
}

fn plausible_json() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("null".to_string()),
        Just("true".to_string()),
        Just("0".to_string()),
        Just("-1".to_string()),
        Just("1e400".to_string()),
        Just("\"\"".to_string()),
        Just("[]".to_string()),
        Just("{}".to_string()),
        "[a-z_]{0,6}".prop_map(|s| format!("\"{s}\"")),
    ];
    let keys = prop::sample::select(vec![
        "id",
        "kind",
        "view",
        "item",
        "value",
        "title",
        "items",
        "sections",
        "actions",
        "handler",
        "name",
        "root",
        "declared",
        "granted",
        "extensions",
        "capability",
        "reason",
        "modifiers",
        "key",
        "source",
        "markdown",
        "metadata",
        "payload",
        "effect",
        "error",
        "invocation",
    ]);
    leaf.prop_recursive(4, 32, 4, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(|v| format!("[{}]", v.join(","))),
            prop::collection::vec((keys.clone(), inner), 0..5).prop_map(|entries| {
                let body: Vec<String> = entries
                    .into_iter()
                    .map(|(k, v)| format!("\"{k}\":{v}"))
                    .collect();
                format!("{{{}}}", body.join(","))
            }),
        ]
    })
}

proptest! {
    #![proptest_config(regressions(512, "tests/regressions/json_never_panics.txt"))]

    #[test]
    fn arbitrary_bytes_never_panic(raw in ".{0,300}") {
        try_every_type(&raw);
        exercise_anything_that_parsed(&raw);
    }

    #[test]
    fn plausible_documents_never_panic(json in plausible_json()) {
        try_every_type(&json);
        exercise_anything_that_parsed(&json);
    }

    #[test]
    fn arbitrary_capability_and_extension_names_never_panic(
        name in ".{0,64}",
        ext in ".{0,64}",
    ) {
        let mut registry = CapabilityRegistry::new();
        let id = ExtensionId::new(ext);
        let cap = Capability::new(name);
        registry.declare(&id, [cap.clone()]);
        let _ = registry.grant(&id, &cap);
        let denial = registry.check(&id, &cap);
        prop_assert_eq!(denial.is_ok(), cap.is_known());
        let _ = registry.revoke(&id, &cap);
        let json = serde_json::to_string(&registry).unwrap();
        let back: CapabilityRegistry = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(registry, back);
    }

    #[test]
    fn arbitrary_titles_and_keys_produce_unique_stable_ids(
        titles in prop::collection::vec(".{0,32}", 0..8),
        keys in prop::collection::vec(prop::option::of(".{0,32}"), 0..8),
    ) {
        let items: Vec<ListItem> = titles
            .iter()
            .zip(keys.iter().chain(std::iter::repeat(&None)))
            .map(|(t, k)| {
                let item = ListItem::new(t.clone());
                match k {
                    Some(k) => item.with_key(k.clone()),
                    None => item,
                }
            })
            .collect();
        let view = ListView { sections: vec![ListSection::untitled(items)], ..ListView::default() };
        let a = ViewTree::new(View::List(view.clone()));
        let b = ViewTree::new(View::List(view));
        prop_assert_eq!(a.nodes(), b.nodes());
        prop_assert!(ViewDiff::between(&a, &b).is_empty());
    }
}
