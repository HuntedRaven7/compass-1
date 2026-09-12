//! Controlled inputs: the rule that keeps a slow extension from overwriting what the user
//! has typed since it started rendering.

mod common;

use compass_extension_api::*;

fn node(n: u64) -> NodeId {
    NodeId::from_raw(n)
}

// ---------------------------------------------------------------------------
// the race this exists for
// ---------------------------------------------------------------------------

#[test]
fn a_render_answering_an_earlier_keystroke_is_not_applied() {
    // The whole point, spelled out: the user types three characters while the extension
    // is still answering the first.
    let mut t = EchoTracker::new();
    let search = node(1);

    let first = t.edited(search); // "a"
    t.edited(search); // "ab"
    let third = t.edited(search); // "abc"

    // The extension's answer to "a" arrives last.
    assert_eq!(
        t.verdict(search, Some(first)),
        Echo::Stale {
            local: third,
            echoed: first
        }
    );
    assert!(!t.verdict(search, Some(first)).applies());

    // Its answer to "abc" is current and does apply.
    assert!(t.verdict(search, Some(third)).applies());
}

#[test]
fn an_extension_setting_a_value_always_wins() {
    // A "Clear" button must clear a box the user is typing into. An absent echo means the
    // extension is setting the value, not answering an edit, so it is not stale -- there
    // is nothing for it to be stale relative to.
    let mut t = EchoTracker::new();
    let n = node(1);
    for _ in 0..50 {
        t.edited(n);
    }
    assert_eq!(t.verdict(n, None), Echo::Authoritative);
    assert!(t.verdict(n, None).applies());
}

#[test]
fn an_untouched_input_accepts_its_first_render() {
    // Before any edit the counter is ZERO, and an extension's initial render carries no
    // echo. Both must apply, or a form could never be populated.
    let t = EchoTracker::new();
    let n = node(7);
    assert_eq!(t.latest(n), Seq::ZERO);
    assert!(t.verdict(n, None).applies());
    assert_eq!(t.verdict(n, Some(Seq::ZERO)), Echo::Current);
}

#[test]
fn counters_are_per_node() {
    // Typing in one field must not make another field's pending answer look stale.
    let mut t = EchoTracker::new();
    let (a, b) = (node(1), node(2));
    let a1 = t.edited(a);
    for _ in 0..10 {
        t.edited(b);
    }
    assert_eq!(t.verdict(a, Some(a1)), Echo::Current);
}

#[test]
fn an_echo_the_host_never_minted_is_refused_and_distinguishable() {
    // A Seq originates in the host, so this is unreachable from a correct extension.
    // It is refused rather than trusted -- applying a value the host cannot place is the
    // worse mistake -- and kept separate from Stale so a host can log it.
    let mut t = EchoTracker::new();
    let n = node(1);
    let real = t.edited(n);
    let invented = Seq::from_raw(real.raw() + 99);
    let verdict = t.verdict(n, Some(invented));
    assert_eq!(
        verdict,
        Echo::FromTheFuture {
            local: real,
            echoed: invented
        }
    );
    assert!(!verdict.applies());
    assert_ne!(
        verdict,
        Echo::Stale {
            local: real,
            echoed: invented
        }
    );
}

// ---------------------------------------------------------------------------
// lifecycle
// ---------------------------------------------------------------------------

#[test]
fn a_recreated_node_does_not_inherit_the_counter_of_its_previous_life() {
    // Ids are derived, so a removed node re-created in the same slot gets the same id.
    // Without forgetting, its first genuine echo would compare against a counter from a
    // node the user is no longer looking at, and be dropped as stale.
    let mut t = EchoTracker::new();
    let n = node(1);
    t.edited(n);
    t.edited(n);
    t.forget(n);

    assert_eq!(t.latest(n), Seq::ZERO);
    let fresh = t.edited(n);
    assert_eq!(t.verdict(n, Some(fresh)), Echo::Current);
}

#[test]
fn retaining_only_live_nodes_bounds_the_map() {
    // Without this the tracker grows for the lifetime of the extension, one entry per
    // input the user has ever touched.
    let mut t = EchoTracker::new();
    for i in 0..100 {
        t.edited(node(i));
    }
    assert_eq!(t.len(), 100);

    let live: std::collections::BTreeSet<NodeId> = (0..3).map(node).collect();
    t.retain_only(&live);
    assert_eq!(t.len(), 3);
    assert_eq!(t.latest(node(50)), Seq::ZERO);
    assert!(t.latest(node(1)) > Seq::ZERO);

    t.retain_only(&std::collections::BTreeSet::new());
    assert!(t.is_empty());
}

#[test]
fn the_counter_saturates_rather_than_wrapping() {
    // Wrapping would make an ancient echo compare as current -- the exact bug this type
    // prevents. Saturating freezes the counter instead, which degrades to the behaviour
    // we had before counting rather than to a silent wrong answer.
    let max = Seq::from_raw(u64::MAX);
    assert_eq!(max.next(), max);
    assert!(max.next() >= max);
}

// ---------------------------------------------------------------------------
// the wire
// ---------------------------------------------------------------------------

#[test]
fn an_absent_echo_stays_absent_on_the_wire() {
    // `None` and `Some(ZERO)` mean different things -- "the extension set this" versus
    // "this answers the state before any edit" -- so the encoding must not conflate them.
    let bare = SearchBar {
        text: Some("hi".into()),
        ..SearchBar::default()
    };
    let json = serde_json::to_string(&bare).unwrap();
    assert!(!json.contains("echo"), "{json}");

    let zero = SearchBar {
        echo: Some(Seq::ZERO),
        ..bare.clone()
    };
    let zero_json = serde_json::to_string(&zero).unwrap();
    assert!(zero_json.contains("echo"), "{zero_json}");
    assert_ne!(json, zero_json);

    assert_eq!(serde_json::from_str::<SearchBar>(&json).unwrap().echo, None);
    assert_eq!(
        serde_json::from_str::<SearchBar>(&zero_json).unwrap().echo,
        Some(Seq::ZERO)
    );
}

#[test]
fn an_input_change_round_trips() {
    let payload = ActionPayload::InputChanged {
        node: node(42),
        value: FieldValue::Text("abc".into()),
        seq: Seq::from_raw(3),
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        serde_json::from_str::<ActionPayload>(&json).unwrap(),
        payload
    );
}

#[test]
fn event_counted_carries_a_value_and_maps_over_it() {
    let counted = EventCounted::new("abc", Seq::from_raw(4));
    let owned = counted.map(str::to_owned);
    assert_eq!(owned.value, "abc");
    assert_eq!(owned.seq, Seq::from_raw(4));
}

// ---------------------------------------------------------------------------
// the echo must not look like a repaint
// ---------------------------------------------------------------------------

#[test]
fn re_stamping_the_echo_alone_is_not_a_change() {
    // `echo` is bookkeeping, not something the user can see. If it entered the
    // fingerprint, every keystroke's answer would report the whole list as updated.
    let build = |echo: Option<Seq>| {
        ViewTree::new(View::List(ListView {
            search: SearchBar {
                text: Some("same".into()),
                echo,
                ..SearchBar::default()
            },
            ..ListView::default()
        }))
    };
    let before = build(Some(Seq::from_raw(1)));
    let after = build(Some(Seq::from_raw(9)));
    assert!(
        ViewDiff::between(&before, &after).is_empty(),
        "{:?}",
        ViewDiff::between(&before, &after).changes
    );
}

#[test]
fn the_text_beside_the_echo_is_still_a_change() {
    // The other half: excluding the echo must not have excluded the value with it.
    let build = |text: &str| {
        ViewTree::new(View::List(ListView {
            search: SearchBar {
                text: Some(text.into()),
                echo: Some(Seq::from_raw(1)),
                ..SearchBar::default()
            },
            ..ListView::default()
        }))
    };
    assert!(!ViewDiff::between(&build("a"), &build("b")).is_empty());
}

#[test]
fn every_visible_search_field_reaches_the_fingerprint() {
    // `search_content` in tree.rs projects the search bar field by field to drop the
    // echo. A field added later and not added there would silently stop being painted,
    // which is invisible in review. This fails when that happens.
    let base = ListView::default();
    let fingerprint = |search: SearchBar| {
        let tree = ViewTree::new(View::List(ListView {
            search,
            ..base.clone()
        }));
        tree.nodes()[0].fingerprint
    };
    let plain = fingerprint(SearchBar::default());

    let mutations: Vec<(&str, SearchBar)> = vec![
        (
            "placeholder",
            SearchBar {
                placeholder: Some("p".into()),
                ..SearchBar::default()
            },
        ),
        (
            "text",
            SearchBar {
                text: Some("t".into()),
                ..SearchBar::default()
            },
        ),
        (
            "host_filtering",
            SearchBar {
                host_filtering: true,
                ..SearchBar::default()
            },
        ),
        (
            "on_change",
            SearchBar {
                on_change: Some(HandlerId::new("h")),
                ..SearchBar::default()
            },
        ),
        (
            "accessory",
            SearchBar {
                accessory: Some(Dropdown::default()),
                ..SearchBar::default()
            },
        ),
        (
            "accessory.value",
            SearchBar {
                accessory: Some(Dropdown {
                    value: Some("v".into()),
                    ..Dropdown::default()
                }),
                ..SearchBar::default()
            },
        ),
        (
            "accessory.placeholder",
            SearchBar {
                accessory: Some(Dropdown {
                    placeholder: Some("p".into()),
                    ..Dropdown::default()
                }),
                ..SearchBar::default()
            },
        ),
        (
            "accessory.filtering",
            SearchBar {
                accessory: Some(Dropdown {
                    filtering: !Dropdown::default().filtering,
                    ..Dropdown::default()
                }),
                ..SearchBar::default()
            },
        ),
        (
            "accessory.on_change",
            SearchBar {
                accessory: Some(Dropdown {
                    on_change: Some(HandlerId::new("h")),
                    ..Dropdown::default()
                }),
                ..SearchBar::default()
            },
        ),
        (
            "accessory.sections",
            SearchBar {
                accessory: Some(Dropdown {
                    sections: vec![DropdownSection::default()],
                    ..Dropdown::default()
                }),
                ..SearchBar::default()
            },
        ),
    ];

    for (field, mutated) in mutations {
        assert_ne!(
            fingerprint(mutated),
            plain,
            "changing {field} did not change the fingerprint: it is missing from \
             `search_content` in tree.rs and will not be repainted"
        );
    }

    // And the one field that must *not*.
    assert_eq!(
        fingerprint(SearchBar {
            echo: Some(Seq::from_raw(3)),
            ..SearchBar::default()
        }),
        plain,
        "the echo reached the fingerprint"
    );
    assert_eq!(
        fingerprint(SearchBar {
            accessory: Some(Dropdown {
                echo: Some(Seq::from_raw(3)),
                ..Dropdown::default()
            }),
            ..SearchBar::default()
        }),
        fingerprint(SearchBar {
            accessory: Some(Dropdown::default()),
            ..SearchBar::default()
        }),
        "the accessory's echo reached the fingerprint"
    );
}
