//! Every public type must survive a trip through JSON unchanged.

mod common;

use common::*;
use compass_extension_api::*;

fn round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).expect("serialize");
    let back: T = serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize {json}: {e}"));
    assert_eq!(
        value, &back,
        "round trip changed the value; json was {json}"
    );
}

#[test]
fn view_trees_round_trip() {
    for view in every_view() {
        let tree = ViewTree::new(view);
        round_trip(&tree);
        // and ids survive, not just the shape
        let json = serde_json::to_string(&tree).unwrap();
        let back: ViewTree = serde_json::from_str(&json).unwrap();
        assert_eq!(tree.nodes(), back.nodes());
        assert_eq!(tree.actions(), back.actions());
    }
}

#[test]
fn leaf_view_types_round_trip() {
    round_trip(&image());
    round_trip(&panel());
    round_trip(&detail());
    round_trip(&empty_state());
    round_trip(&search_bar());
    round_trip(&dropdown());
    round_trip(&list_view());
    round_trip(&grid_view());
    round_trip(&form_view());
    round_trip(&Accessory::default());
    round_trip(&AspectRatio {
        width: 4,
        height: 3,
    });
    round_trip(&Color::Named("primary".into()));
    round_trip(&Color::Literal("#abcdef".into()));
    round_trip(&ImageMask::RoundedRectangle);
    round_trip(&GridFit::Fill);
    round_trip(&GridInset::Small);
    round_trip(&DatePrecision::Day);
    round_trip(&Pagination {
        has_more: false,
        loaded: 0,
        on_load_more: HandlerId::new("h"),
    });
    for value in [
        FieldValue::Text("x".into()),
        FieldValue::Bool(false),
        FieldValue::Integer(i64::MIN),
        FieldValue::Date("2026-01-01T00:00:00Z".into()),
        FieldValue::Paths(vec!["/a".into(), "/b".into()]),
        FieldValue::Values(vec!["v".into()]),
        FieldValue::Empty,
    ] {
        round_trip(&value);
    }
}

#[test]
fn action_types_round_trip() {
    round_trip(&Action::new("Go", "h.go"));
    round_trip(&Action::new("Go", "h.go").with_shortcut(Shortcut::new(
        [
            KeyModifier::Cmd,
            KeyModifier::Shift,
            KeyModifier::Alt,
            KeyModifier::Ctrl,
            KeyModifier::Meta,
        ],
        "k",
    )));
    round_trip(&ActionStyle::Destructive);
    round_trip(&HandlerId::new("h.go"));
    round_trip(&KeyEquivalent::new("arrowUp"));
    round_trip(&Shortcut::bare("escape"));
}

#[test]
fn identity_and_diff_types_round_trip() {
    round_trip(&NodeId::ROOT);
    round_trip(&NodeId::ROOT.child_keyed("items", Some("a"), 0));
    let a = ViewTree::new(View::List(list_view()));
    let mut other = list_view();
    other.sections[0].items[0].title = "Different".into();
    let b = ViewTree::new(View::List(other));
    round_trip(&ViewDiff::between(&a, &b));
    for node in a.nodes() {
        round_trip(&node);
    }
    for action in a.actions() {
        round_trip(&action);
    }
}

#[test]
fn capability_types_round_trip() {
    let ext = ExtensionId::new("com.example.demo");
    let registry = registry_with(
        &ext,
        &[
            Capability::CLIPBOARD_READ,
            Capability::NETWORK_REQUEST,
            Capability::new("future.thing"),
        ],
        &[Capability::CLIPBOARD_READ],
    );
    round_trip(&registry);
    round_trip(&ext);
    round_trip(&Capability::WINDOW_MANAGE);
    round_trip(&Capability::new("future.thing"));
    round_trip(registry.state(&ext).unwrap());
    let denial = registry
        .check(&ext, &Capability::NETWORK_REQUEST)
        .unwrap_err();
    round_trip(&denial);
    for reason in [
        DenialReason::UnknownExtension,
        DenialReason::NotDeclared,
        DenialReason::NotGranted,
        DenialReason::Revoked,
        DenialReason::UnknownCapability,
    ] {
        round_trip(&reason);
    }
}

#[test]
fn dispatch_types_round_trip() {
    let ext = ExtensionId::new("com.example.demo");
    round_trip(&InvocationId(7));
    round_trip(&InvocationSource::Shortcut);
    round_trip(&ActionPayload::None);
    round_trip(&ActionPayload::FormValues(form_values()));
    round_trip(&ActionPayload::SearchText("q".into()));
    round_trip(&ActionPayload::Selection(NodeId::ROOT));
    round_trip(&ActionRequest {
        invocation: InvocationId(1),
        extension: ext.clone(),
        handler: HandlerId::new("h.open"),
        node: NodeId::ROOT,
        source: InvocationSource::Panel,
        payload: ActionPayload::SearchText("q".into()),
    });
    for effect in [
        ActionEffect::Rerender,
        ActionEffect::PushView,
        ActionEffect::PopView,
        ActionEffect::PopToRoot,
        ActionEffect::CloseWindow,
        ActionEffect::Toast {
            style: ToastStyle::Failure,
            title: "Nope".into(),
            message: Some("why".into()),
        },
        ActionEffect::Hud {
            text: "Copied".into(),
        },
    ] {
        round_trip(&effect);
    }
    for style in [
        ToastStyle::Info,
        ToastStyle::Animated,
        ToastStyle::Success,
        ToastStyle::Failure,
    ] {
        round_trip(&style);
    }
    round_trip(&ActionResponse::Completed {
        invocation: InvocationId(1),
        effects: vec![ActionEffect::Rerender],
    });
    for error in [
        DispatchError::UnknownAction {
            handler: HandlerId::new("gone"),
        },
        DispatchError::UnknownInvocation {
            invocation: InvocationId(9),
        },
        DispatchError::ExtensionGone {
            extension: ext.clone(),
            stage: Stage::InFlight,
        },
        DispatchError::ExtensionGone {
            extension: ext.clone(),
            stage: Stage::BeforeDispatch,
        },
        DispatchError::CapabilityDenied {
            denial: Denial {
                extension: ext.clone(),
                capability: Capability::CLIPBOARD_WRITE,
                reason: DenialReason::NotGranted,
            },
        },
        DispatchError::Failed {
            extension: ext.clone(),
            message: "boom".into(),
        },
        DispatchError::InvalidPayload {
            detail: "expected form values".into(),
        },
    ] {
        round_trip(&error);
        round_trip(&ActionResponse::Failed {
            invocation: InvocationId(2),
            error,
        });
    }
}
