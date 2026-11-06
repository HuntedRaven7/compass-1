//! The launcher hotkey's first-run request against a mock GlobalShortcuts
//! portal on a private bus: granted, declined, left without a key because
//! something else has it, and no portal at all.
//!
//! The engine's portal backend and service run as they do in `serve`; only
//! the bus is private. The mock is compass-portals' own `zbus` server.

#[path = "../../compass-portals/tests/support/mod.rs"]
mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use compass::global_shortcuts::{PortalBackend, Service};
use compass_core::global_shortcuts::{self, Binding, LAUNCHER_ID};
use compass_core::launcher_shortcut::LauncherShortcut;
use compass_portals::{PortalConfig, Portals, ShortcutBinder};
use support::bus::start_or_skip;
use support::mock::{Behaviour, MockOptions, MockPortal};

fn desired() -> BTreeMap<String, Binding> {
    let config: compass_core::Config =
        serde_json::from_str(r#"{"launcher": {"hotkey": "super+space"}}"#).unwrap();
    global_shortcuts::desired(&config, |_| None)
}

/// A service over the portal on `address`, deferred as it is while the
/// first-run flow is due, and asked once as Set Up Shortcut asks.
async fn ask(address: &str) -> (LauncherShortcut, LauncherShortcut) {
    let config = PortalConfig::default()
        .with_address(address)
        .with_call_timeout(Duration::from_secs(2))
        .with_dialog_timeout(Duration::from_secs(2));
    let portals = Portals::connect(config).await.expect("the private bus");
    let (events, _presses) = tokio::sync::mpsc::unbounded_channel();
    let mut service = Service::new(PortalBackend::new(ShortcutBinder::new(portals, events)));
    service.defer();
    let desired = desired();
    service.reconcile(&desired).await;
    let before = service.launcher(&desired);
    service.request(&desired).await;
    (before, service.launcher(&desired))
}

#[tokio::test]
async fn nothing_is_asked_until_the_flow_asks_and_a_grant_names_the_key() {
    let Some(bus) = start_or_skip("nothing_is_asked_until_the_flow_asks_and_a_grant_names_the_key")
    else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    let (before, after) = ask(bus.address()).await;
    assert_eq!(before, LauncherShortcut::NotRequested);
    assert_eq!(
        after,
        LauncherShortcut::Granted {
            trigger: Some("Super+space".to_owned())
        },
        "the desktop's own words for the key"
    );
    assert_eq!(
        mock.calls()
            .iter()
            .filter(|call| **call == "BindShortcuts")
            .count(),
        1,
        "one request, made when asked"
    );
    // Only Compass's own shortcuts, in Compass's own session: nothing else
    // the desktop binds is touched.
    let ids: Vec<String> = mock.bound().into_iter().map(|(id, ..)| id).collect();
    assert_eq!(ids, [LAUNCHER_ID]);
}

#[tokio::test]
async fn a_declined_dialog_is_a_refusal() {
    let Some(bus) = start_or_skip("a_declined_dialog_is_a_refusal") else {
        return;
    };
    let _mock = MockPortal::start(
        bus.address(),
        MockOptions::default().behaving(Behaviour::Deny),
    )
    .await
    .expect("mock");
    let (_, after) = ask(bus.address()).await;
    assert_eq!(after, LauncherShortcut::Refused);
    assert!(after.can_request(), "Try Again is offered");
}

#[tokio::test]
async fn a_key_something_else_holds_is_a_conflict() {
    let Some(bus) = start_or_skip("a_key_something_else_holds_is_a_conflict") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.take_trigger("LOGO+space");
    let (_, after) = ask(bus.address()).await;
    assert!(
        matches!(after, LauncherShortcut::Conflict { .. }),
        "{after:?}"
    );
}

#[tokio::test]
async fn a_portal_that_ends_the_request_unexplained_is_a_failure() {
    let Some(bus) = start_or_skip("a_portal_that_ends_the_request_unexplained_is_a_failure") else {
        return;
    };
    let _mock = MockPortal::start(
        bus.address(),
        MockOptions::default().behaving(Behaviour::Other),
    )
    .await
    .expect("mock");
    let (_, after) = ask(bus.address()).await;
    assert!(
        matches!(after, LauncherShortcut::Failed { .. }),
        "{after:?}"
    );
}

#[tokio::test]
async fn a_desktop_without_the_portal_is_unavailable() {
    let Some(bus) = start_or_skip("a_desktop_without_the_portal_is_unavailable") else {
        return;
    };
    let _mock = MockPortal::start(bus.address(), MockOptions::without_global_shortcuts())
        .await
        .expect("mock");
    let (_, after) = ask(bus.address()).await;
    assert_eq!(after, LauncherShortcut::Unavailable);
}
