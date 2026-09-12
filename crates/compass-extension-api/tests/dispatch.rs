//! Action dispatch: resolving what the UI reported, and surviving the extension
//! disappearing halfway through.

mod common;

use common::*;
use compass_extension_api::*;

fn ext() -> ExtensionId {
    ExtensionId::new("com.example.demo")
}

fn index() -> ActionIndex {
    ActionIndex::from_tree(&ViewTree::new(View::List(list_view())))
}

#[test]
fn the_index_finds_every_action_including_those_nested_in_submenus() {
    let index = index();
    assert!(!index.is_empty());
    for handler in ["h.open", "h.copy", "h.delete", "h.refresh", "h.reload"] {
        index
            .resolve(&HandlerId::new(handler))
            .unwrap_or_else(|e| panic!("{handler}: {e}"));
    }
    let titles: Vec<&str> = index.iter().map(|a| a.title.as_str()).collect();
    assert!(titles.contains(&"Delete Forever"), "{titles:?}");
}

#[test]
fn an_unknown_action_id_is_an_error_naming_the_token() {
    let index = index();
    let mut pending = Pending::new();
    let ghost = HandlerId::new("h.does-not-exist");

    let err = index.resolve(&ghost).unwrap_err();
    assert_eq!(
        err,
        DispatchError::UnknownAction {
            handler: ghost.clone()
        }
    );
    assert!(err.to_string().contains("h.does-not-exist"));

    let err = pending
        .begin(
            &index,
            &ext(),
            &ghost,
            InvocationSource::Primary,
            ActionPayload::None,
        )
        .unwrap_err();
    assert!(matches!(err, DispatchError::UnknownAction { .. }));
    // A rejected dispatch leaves nothing behind.
    assert!(pending.is_empty());
}

#[test]
fn an_action_with_a_shortcut_resolves_by_keystroke_modifier_order_notwithstanding() {
    let index = index();
    let bound = Shortcut::new([KeyModifier::Cmd], "return");
    let found = index.resolve_shortcut(&bound).expect("bound");
    assert_eq!(found.handler, HandlerId::new("h.open"));
    assert_eq!(found.shortcut.as_ref().unwrap(), &bound);

    let reordered = Shortcut::new([KeyModifier::Shift, KeyModifier::Ctrl], "x");
    assert!(
        index.resolve_shortcut(&reordered).is_some(),
        "modifier order must not matter"
    );
}

#[test]
fn an_action_without_a_shortcut_is_reachable_only_by_id() {
    let index = index();
    let copy = index.resolve(&HandlerId::new("h.copy")).unwrap();
    assert!(copy.shortcut.is_none());
    // Not bound to anything, and asking for an unbound keystroke is not an error.
    assert!(index.resolve_shortcut(&Shortcut::bare("f13")).is_none());
    assert!(
        index
            .resolve_shortcut(&Shortcut::new([KeyModifier::Alt], "c"))
            .is_none()
    );
}

#[test]
fn a_full_round_trip_completes_and_clears() {
    let index = index();
    let mut pending = Pending::new();
    let request = pending
        .begin(
            &index,
            &ext(),
            &HandlerId::new("h.open"),
            InvocationSource::Shortcut,
            ActionPayload::SearchText("bug".into()),
        )
        .unwrap();

    assert_eq!(request.extension, ext());
    assert_eq!(request.source, InvocationSource::Shortcut);
    assert_eq!(
        request.node,
        index.resolve(&HandlerId::new("h.open")).unwrap().node
    );
    assert_eq!(pending.len(), 1);
    assert_eq!(pending.get(request.invocation), Some(&request));

    let response = ActionResponse::Completed {
        invocation: request.invocation,
        effects: vec![
            ActionEffect::CloseWindow,
            ActionEffect::Hud {
                text: "Opened".into(),
            },
        ],
    };
    let settled = pending.complete(&response).unwrap();
    assert_eq!(settled, request);
    assert!(pending.is_empty());
}

#[test]
fn form_submission_carries_its_values() {
    let tree = ViewTree::new(View::Form(form_view()));
    let index = ActionIndex::from_tree(&tree);
    let mut pending = Pending::new();
    let request = pending
        .begin(
            &index,
            &ext(),
            &HandlerId::new("h.submit"),
            InvocationSource::Primary,
            ActionPayload::FormValues(form_values()),
        )
        .unwrap();
    match &request.payload {
        ActionPayload::FormValues(values) => {
            assert_eq!(values.get("title"), Some(&FieldValue::Text("Crash".into())));
        }
        other => panic!("expected form values, got {other:?}"),
    }
}

#[test]
fn invocation_ids_are_distinct_and_a_stale_response_is_rejected() {
    let index = index();
    let mut pending = Pending::new();
    let first = pending
        .begin(
            &index,
            &ext(),
            &HandlerId::new("h.open"),
            InvocationSource::Panel,
            ActionPayload::None,
        )
        .unwrap();
    let second = pending
        .begin(
            &index,
            &ext(),
            &HandlerId::new("h.copy"),
            InvocationSource::Panel,
            ActionPayload::None,
        )
        .unwrap();
    assert_ne!(first.invocation, second.invocation);

    let response = ActionResponse::Completed {
        invocation: first.invocation,
        effects: vec![],
    };
    pending.complete(&response).unwrap();
    // The same answer twice means the two sides disagree; say so rather than shrug.
    let err = pending.complete(&response).unwrap_err();
    assert_eq!(
        err,
        DispatchError::UnknownInvocation {
            invocation: first.invocation
        }
    );
    assert_eq!(pending.len(), 1);
}

#[test]
fn an_extension_that_disappears_mid_action_turns_every_invocation_into_a_reportable_failure() {
    let index = index();
    let mut pending = Pending::new();
    let mine = ext();
    let other = ExtensionId::new("com.example.other");

    let a = pending
        .begin(
            &index,
            &mine,
            &HandlerId::new("h.open"),
            InvocationSource::Primary,
            ActionPayload::None,
        )
        .unwrap();
    let b = pending
        .begin(
            &index,
            &mine,
            &HandlerId::new("h.copy"),
            InvocationSource::Panel,
            ActionPayload::None,
        )
        .unwrap();
    let untouched = pending
        .begin(
            &index,
            &other,
            &HandlerId::new("h.refresh"),
            InvocationSource::Panel,
            ActionPayload::None,
        )
        .unwrap();

    let lost = pending.extension_gone(&mine);
    assert_eq!(lost.len(), 2);
    assert_eq!(lost[0].invocation(), a.invocation);
    assert_eq!(lost[1].invocation(), b.invocation);
    for response in &lost {
        match response {
            ActionResponse::Failed { error, .. } => {
                assert_eq!(
                    error,
                    &DispatchError::ExtensionGone {
                        extension: mine.clone(),
                        stage: Stage::InFlight
                    }
                );
                let text = error.to_string();
                assert!(text.contains("com.example.demo"), "{text}");
                assert!(text.contains("in flight"), "{text}");
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    // Other extensions are untouched, and a late answer from the dead one is refused.
    assert_eq!(pending.len(), 1);
    assert_eq!(pending.get(untouched.invocation), Some(&untouched));
    let late = ActionResponse::Completed {
        invocation: a.invocation,
        effects: vec![],
    };
    assert!(matches!(
        pending.complete(&late),
        Err(DispatchError::UnknownInvocation { .. })
    ));

    // Idempotent: a second death report has nothing left to reap.
    assert!(pending.extension_gone(&mine).is_empty());
}

#[test]
fn an_extension_that_never_started_fails_before_dispatch() {
    let error = DispatchError::ExtensionGone {
        extension: ext(),
        stage: Stage::BeforeDispatch,
    };
    assert!(error.to_string().contains("before dispatch"));
    assert!(error.missing_capability().is_none());
}

#[test]
fn a_capability_denial_becomes_a_dispatch_failure_that_still_names_the_capability() {
    let e = ext();
    let registry = registry_with(&e, &[Capability::CLIPBOARD_WRITE], &[]);
    let index = index();
    let mut pending = Pending::new();
    let request = pending
        .begin(
            &index,
            &e,
            &HandlerId::new("h.copy"),
            InvocationSource::Primary,
            ActionPayload::None,
        )
        .unwrap();

    // The host checks before doing the privileged thing, and turns the refusal into the
    // response the UI shows.
    let denial = registry
        .check(&e, &Capability::CLIPBOARD_WRITE)
        .unwrap_err();
    let response = ActionResponse::Failed {
        invocation: request.invocation,
        error: denial.into(),
    };
    let settled = pending.complete(&response).unwrap();
    assert_eq!(settled.invocation, request.invocation);
    match response {
        ActionResponse::Failed { error, .. } => {
            assert_eq!(
                error.missing_capability(),
                Some(&Capability::CLIPBOARD_WRITE)
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_duplicate_handler_token_resolves_to_the_first_in_render_order() {
    let view = ListView {
        sections: vec![ListSection::untitled([
            ListItem::new("first").with_actions(ActionPanel::of([Action::new("First", "dup")])),
            ListItem::new("second").with_actions(ActionPanel::of([Action::new("Second", "dup")])),
        ])],
        ..ListView::default()
    };
    let index = ActionIndex::from_tree(&ViewTree::new(View::List(view)));
    assert_eq!(index.len(), 1);
    assert_eq!(
        index.resolve(&HandlerId::new("dup")).unwrap().title,
        "First"
    );
}

#[test]
fn a_tree_with_no_actions_has_an_empty_index() {
    let index = ActionIndex::from_tree(&ViewTree::new(View::Detail(Detail::default())));
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
    assert!(index.resolve(&HandlerId::new("anything")).is_err());
}
