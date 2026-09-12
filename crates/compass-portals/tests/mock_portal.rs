//! The mock-portal bus suite.
//!
//! Every test spawns its own real `dbus-daemon --session`, passes that address
//! explicitly, and drives the real client against a real `zbus` server
//! implementing `org.freedesktop.portal.*`. No ambient
//! `DBUS_SESSION_BUS_ADDRESS` is ever consulted and nothing is stubbed at the
//! Rust level: the bytes go over a unix socket.
//!
//! There is no `sleep` used for synchronisation anywhere. Signal delivery is
//! ordered by the bus behind the match rules the client installs before it
//! calls anything, and `tokio::time::timeout` appears only as a failure
//! deadline — except where a timeout is itself the thing under test.

mod support;

use std::path::PathBuf;
use std::time::Duration;

use compass_portals::{
    Availability, DegradedFeature, FileChooserOutcome, FileChooserRequest, GLOBAL_SHORTCUTS,
    Modifiers, NotQueryable, OpenOutcome, PortalConfig, PortalError, Portals, ShortcutDescriptor,
    ShortcutEvent, ShortcutsOutcome, Trigger, Unavailable,
};
use support::bus::{TestBus, start_or_skip};
use support::mock::{Behaviour, MockOptions, MockPortal, start_unresponsive_portal};

/// Generous failure deadline. Every await below is event-driven, so a healthy
/// run finishes in milliseconds and only a genuine hang reaches this.
const DEADLINE: Duration = Duration::from_secs(10);

/// Short deadline for the calls whose *timeout* is under test.
const SHORT: Duration = Duration::from_millis(300);

async fn deadline<F: std::future::Future>(what: &str, fut: F) -> F::Output {
    match tokio::time::timeout(DEADLINE, fut).await {
        Ok(v) => v,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

fn config(bus: &TestBus) -> PortalConfig {
    PortalConfig::default()
        .with_address(bus.address())
        .with_call_timeout(SHORT)
        .with_dialog_timeout(SHORT)
}

async fn portals(bus: &TestBus) -> Portals {
    Portals::connect(config(bus)).await.expect("connect")
}

fn logo_space() -> Trigger {
    Trigger::new(Modifiers::LOGO, "space").expect("valid trigger")
}

fn toggle() -> ShortcutDescriptor {
    // Mirrors GlobalShortcutService::TOGGLE_ID from the C++ build.
    ShortcutDescriptor::new("@toggle-launcher", "Toggle Compass").with_trigger(logo_space())
}

// ---------------------------------------------------------------------------
// Capability probing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_bare_bus_connects_and_reports_everything_absent() {
    let Some(bus) = start_or_skip("a_bare_bus_connects_and_reports_everything_absent") else {
        return;
    };
    // Nobody owns org.freedesktop.portal.Desktop on this bus at all.
    let portals = deadline("connect", portals(&bus)).await;
    let caps = portals.capabilities();

    let absent = Availability::Unavailable {
        reason: Unavailable::NoPortalFrontend,
    };
    assert_eq!(caps.global_shortcuts, absent);
    assert_eq!(caps.open_uri, absent);
    assert_eq!(caps.file_chooser, absent);
    assert!(!caps.any_available());

    // Connecting is not an error, and neither is asking for a missing portal:
    // it is an inspectable, non-fatal refusal.
    let err = deadline("global_shortcuts", portals.global_shortcuts())
        .await
        .expect_err("must not pretend the portal is there");
    assert!(err.is_unavailable(), "{err:?}");
    assert!(portals.open_uri().unwrap_err().is_unavailable());
    assert!(portals.file_chooser().unwrap_err().is_unavailable());
}

#[tokio::test]
async fn a_frontend_without_the_interface_is_the_wlroots_case() {
    let Some(bus) = start_or_skip("a_frontend_without_the_interface_is_the_wlroots_case") else {
        return;
    };
    // xdg-desktop-portal is running and healthy; no backend implements
    // GlobalShortcuts. This is Sway/river/labwc.
    let _mock = MockPortal::start(bus.address(), MockOptions::without_global_shortcuts())
        .await
        .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let caps = portals.capabilities();

    assert_eq!(
        caps.global_shortcuts,
        Availability::Unavailable {
            reason: Unavailable::InterfaceMissing
        }
    );
    // ... and it is distinguishable from there being no portal at all.
    assert_ne!(
        caps.global_shortcuts,
        Availability::Unavailable {
            reason: Unavailable::NoPortalFrontend
        }
    );
    // The rest of the portal is fine, which is the whole point of probing
    // per-interface rather than per-bus-name.
    assert!(caps.open_uri.is_available());
    assert!(caps.file_chooser.is_available());
    assert_eq!(caps.degraded(), vec![DegradedFeature::GlobalHotkeys]);
}

#[tokio::test]
async fn an_interface_below_the_version_floor_is_unavailable() {
    let Some(bus) = start_or_skip("an_interface_below_the_version_floor_is_unavailable") else {
        return;
    };
    let _mock = MockPortal::start(bus.address(), MockOptions::default().at_version(0))
        .await
        .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    assert_eq!(
        portals.capabilities().global_shortcuts,
        Availability::Unavailable {
            reason: Unavailable::VersionTooOld {
                found: 0,
                required: 1
            }
        }
    );
}

#[tokio::test]
async fn an_interface_newer_than_we_know_still_works() {
    let Some(bus) = start_or_skip("an_interface_newer_than_we_know_still_works") else {
        return;
    };
    let _mock = MockPortal::start(bus.address(), MockOptions::default().at_version(99))
        .await
        .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let availability = portals.capabilities().global_shortcuts;
    assert_eq!(availability, Availability::Available { version: 99 });
    // Available, but flagged as outside the range this build was written for.
    assert!(!GLOBAL_SHORTCUTS.is_known_version(99));
}

#[tokio::test]
async fn a_malformed_version_property_is_not_queryable() {
    let Some(bus) = start_or_skip("a_malformed_version_property_is_not_queryable") else {
        return;
    };
    let options = MockOptions {
        malformed_version: true,
        open_uri: false,
        file_chooser: false,
        ..MockOptions::default()
    };
    let _mock = MockPortal::start(bus.address(), options)
        .await
        .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let availability = portals.capabilities().global_shortcuts;
    assert!(
        matches!(
            availability,
            Availability::NotQueryable {
                reason: NotQueryable::MalformedVersion { .. }
            }
        ),
        "{availability:?}"
    );
    // "We could not ask" must not be reported as "it is not there".
    assert!(availability.is_indeterminate());
}

#[tokio::test]
async fn an_unresponsive_frontend_times_out_rather_than_hanging() {
    let Some(bus) = start_or_skip("an_unresponsive_frontend_times_out_rather_than_hanging") else {
        return;
    };
    // Owns the bus name, never answers anything. A D-Bus call has no inherent
    // deadline, so without our own timeout this test would never finish.
    let _wedged = start_unresponsive_portal(bus.address())
        .await
        .expect("unresponsive portal");

    let portals = deadline("connect against a wedged portal", portals(&bus)).await;
    let availability = portals.capabilities().global_shortcuts;
    assert!(
        matches!(
            availability,
            Availability::NotQueryable {
                reason: NotQueryable::ProbeTimedOut { .. }
            }
        ),
        "{availability:?}"
    );
    assert!(availability.is_indeterminate());
}

// ---------------------------------------------------------------------------
// GlobalShortcuts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_bind_round_trips_through_the_portal() {
    let Some(bus) = start_or_skip("a_bind_round_trips_through_the_portal") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let session = deadline("create session", portals.global_shortcuts())
        .await
        .expect("session");
    assert_eq!(session.version(), 2);

    let outcome = deadline(
        "bind",
        session.bind(&[
            toggle(),
            ShortcutDescriptor::new("search", "Search")
                .with_trigger(Trigger::parse("CTRL+SHIFT+p").expect("valid")),
        ]),
    )
    .await
    .expect("bind");

    // What we sent, as the portal saw it.
    assert_eq!(
        mock.bound(),
        vec![
            (
                "@toggle-launcher".to_owned(),
                "Toggle Compass".to_owned(),
                Some("LOGO+space".to_owned())
            ),
            (
                "search".to_owned(),
                "Search".to_owned(),
                Some("CTRL+SHIFT+p".to_owned())
            ),
        ]
    );
    assert_eq!(mock.calls(), vec!["CreateSession", "BindShortcuts"]);

    // What came back: the portal's own trigger description, not ours.
    let ShortcutsOutcome::Granted { shortcuts } = &outcome else {
        panic!("expected Granted, got {outcome:?}");
    };
    assert_eq!(shortcuts.len(), 2);
    assert_eq!(shortcuts[0].id, "@toggle-launcher");
    assert_eq!(shortcuts[0].description, "Toggle Compass");
    assert_eq!(shortcuts[0].trigger_description, "Super+space");

    // And the session can be listed back.
    let listed = deadline("list", session.list()).await.expect("list");
    assert_eq!(listed.shortcuts(), shortcuts.as_slice());
}

#[tokio::test]
async fn an_activation_signal_reaches_a_subscriber() {
    let Some(bus) = start_or_skip("an_activation_signal_reaches_a_subscriber") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let session = deadline("create session", portals.global_shortcuts())
        .await
        .expect("session");
    let mut events = session.subscribe();
    deadline("bind", session.bind(&[toggle()]))
        .await
        .expect("bind");

    deadline(
        "emit",
        mock.emit_activated("@toggle-launcher", 1_700_000_000, Some("tok-42")),
    )
    .await
    .expect("emit activated");

    let event = deadline("activation", events.recv())
        .await
        .expect("stream open");
    assert_eq!(
        event,
        ShortcutEvent::Activated {
            id: "@toggle-launcher".to_owned(),
            timestamp: Duration::from_millis(1_700_000_000),
            activation_token: Some("tok-42".to_owned()),
        }
    );

    deadline(
        "emit deactivated",
        mock.emit_deactivated("@toggle-launcher", 1_700_000_001),
    )
    .await
    .expect("emit deactivated");
    let event = deadline("deactivation", events.recv())
        .await
        .expect("stream open");
    assert_eq!(
        event,
        ShortcutEvent::Deactivated {
            id: "@toggle-launcher".to_owned(),
            timestamp: Duration::from_millis(1_700_000_001),
        }
    );
}

#[tokio::test]
async fn two_subscribers_both_see_an_activation() {
    let Some(bus) = start_or_skip("two_subscribers_both_see_an_activation") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let session = deadline("create session", portals.global_shortcuts())
        .await
        .expect("session");
    let mut a = session.subscribe();
    let mut b = session.subscribe();
    deadline("bind", session.bind(&[toggle()]))
        .await
        .expect("bind");

    deadline("emit", mock.emit_activated("@toggle-launcher", 7, None))
        .await
        .expect("emit");

    for (name, events) in [("a", &mut a), ("b", &mut b)] {
        let event = deadline(name, events.recv()).await.expect("stream open");
        assert!(
            matches!(event, ShortcutEvent::Activated { ref id, .. } if id == "@toggle-launcher"),
            "{name}: {event:?}"
        );
    }
}

#[tokio::test]
async fn dropping_the_session_closes_subscriptions() {
    let Some(bus) = start_or_skip("dropping_the_session_closes_subscriptions") else {
        return;
    };
    let _mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let session = deadline("create session", portals.global_shortcuts())
        .await
        .expect("session");
    let mut events = session.subscribe();
    drop(session);
    assert_eq!(deadline("close", events.recv()).await, None);
}

#[tokio::test]
async fn a_denied_permission_dialog_is_an_outcome_not_an_error() {
    let Some(bus) = start_or_skip("a_denied_permission_dialog_is_an_outcome_not_an_error") else {
        return;
    };
    // The session is created, then the first-run permission dialog is
    // dismissed: portal response code 1.
    let mock = MockPortal::start(
        bus.address(),
        MockOptions::default().behaving(Behaviour::Deny),
    )
    .await
    .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let session = deadline("create session", portals.global_shortcuts())
        .await
        .expect("session");
    let outcome = deadline("bind", session.bind(&[toggle()]))
        .await
        .expect("a denial is not an Err");

    assert_eq!(outcome, ShortcutsOutcome::Denied);
    assert!(!outcome.is_granted());
    assert!(outcome.shortcuts().is_empty());
    // The call really did reach the portal.
    assert!(mock.calls().contains(&"BindShortcuts"));
}

#[tokio::test]
async fn an_unexplained_refusal_is_distinct_from_a_denial() {
    let Some(bus) = start_or_skip("an_unexplained_refusal_is_distinct_from_a_denial") else {
        return;
    };
    let _mock = MockPortal::start(
        bus.address(),
        MockOptions::default().behaving(Behaviour::Other),
    )
    .await
    .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let session = deadline("create session", portals.global_shortcuts())
        .await
        .expect("session");
    assert_eq!(
        deadline("bind", session.bind(&[toggle()]))
            .await
            .expect("not an Err"),
        ShortcutsOutcome::Refused
    );
}

#[tokio::test]
async fn a_request_that_never_answers_is_bounded_by_the_timeout() {
    let Some(bus) = start_or_skip("a_request_that_never_answers_is_bounded_by_the_timeout") else {
        return;
    };
    // The backend returns a request handle and then never emits Response —
    // a dialog nobody ever answers. Without a bound this hangs forever.
    let _mock = MockPortal::start(
        bus.address(),
        MockOptions::default().behaving(Behaviour::Silent),
    )
    .await
    .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let session = deadline("create session", portals.global_shortcuts())
        .await
        .expect("session");

    let err = deadline("bind", session.bind(&[toggle()]))
        .await
        .expect_err("a silent backend must time out");
    match err {
        PortalError::Timeout { method, timeout } => {
            assert_eq!(method, "BindShortcuts");
            assert_eq!(timeout, SHORT);
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn a_session_that_never_answers_is_bounded_by_the_timeout() {
    let Some(bus) = start_or_skip("a_session_that_never_answers_is_bounded_by_the_timeout") else {
        return;
    };
    let options = MockOptions {
        session_behaviour: Behaviour::Silent,
        ..MockOptions::default()
    };
    let _mock = MockPortal::start(bus.address(), options)
        .await
        .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let err = deadline("create session", portals.global_shortcuts())
        .await
        .expect_err("must not hang");
    assert!(
        matches!(
            err,
            PortalError::Timeout {
                method: "CreateSession",
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_malformed_reply_is_an_error_not_a_panic() {
    let Some(bus) = start_or_skip("a_malformed_reply_is_an_error_not_a_panic") else {
        return;
    };
    // Response code 0, but `shortcuts` is a string instead of `a(sa{sv})`.
    let _mock = MockPortal::start(
        bus.address(),
        MockOptions::default().behaving(Behaviour::Malformed),
    )
    .await
    .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let session = deadline("create session", portals.global_shortcuts())
        .await
        .expect("session");
    let err = deadline("bind", session.bind(&[toggle()]))
        .await
        .expect_err("a malformed reply must not be reported as success");
    assert!(matches!(err, PortalError::Call { .. }), "{err:?}");
    assert!(!err.is_unavailable());
}

#[tokio::test]
async fn a_malformed_session_reply_is_an_error_not_a_panic() {
    let Some(bus) = start_or_skip("a_malformed_session_reply_is_an_error_not_a_panic") else {
        return;
    };
    let options = MockOptions {
        session_behaviour: Behaviour::Malformed,
        ..MockOptions::default()
    };
    let _mock = MockPortal::start(bus.address(), options)
        .await
        .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let err = deadline("create session", portals.global_shortcuts())
        .await
        .expect_err("a session handle of the wrong type must not be accepted");
    assert!(matches!(err, PortalError::Call { .. }), "{err:?}");
}

#[tokio::test]
async fn configure_shortcuts_is_gated_on_interface_v2() {
    let Some(bus) = start_or_skip("configure_shortcuts_is_gated_on_interface_v2") else {
        return;
    };
    let _mock = MockPortal::start(bus.address(), MockOptions::default().at_version(1))
        .await
        .expect("mock");

    let portals = deadline("connect", portals(&bus)).await;
    let session = deadline("create session", portals.global_shortcuts())
        .await
        .expect("session");
    assert_eq!(session.version(), 1);

    let err = deadline("configure", session.configure())
        .await
        .expect_err("v1 has no ConfigureShortcuts");
    assert!(
        matches!(
            err,
            PortalError::VersionTooOld {
                method: "ConfigureShortcuts",
                found: 1,
                required: 2,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(err.is_unavailable());
}

// ---------------------------------------------------------------------------
// OpenURI
// ---------------------------------------------------------------------------

#[tokio::test]
async fn open_uri_round_trips() {
    let Some(bus) = start_or_skip("open_uri_round_trips") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let open = portals.open_uri().expect("available");

    assert_eq!(
        deadline("open", open.open_uri("https://example.com/", false))
            .await
            .expect("open"),
        OpenOutcome::Opened
    );
    assert_eq!(
        mock.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .opened_uris,
        vec!["https://example.com/".to_owned()]
    );
}

#[tokio::test]
async fn open_uri_rejects_a_uri_without_a_scheme_before_calling() {
    let Some(bus) = start_or_skip("open_uri_rejects_a_uri_without_a_scheme_before_calling") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let open = portals.open_uri().expect("available");

    let err = deadline("open", open.open_uri("not a uri", false))
        .await
        .expect_err("must be rejected");
    assert!(matches!(err, PortalError::Protocol(_)), "{err:?}");
    assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn open_path_passes_a_real_descriptor_over_the_bus() {
    let Some(bus) = start_or_skip("open_path_passes_a_real_descriptor_over_the_bus") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");

    // Inside a Flatpak the handler never sees our filename, only the fd, so
    // the mock proves the descriptor arrived by reading it.
    let dir = std::env::temp_dir().join(format!("compass-portals-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("note.txt");
    std::fs::write(&path, b"hello from the sandbox").expect("write");

    let portals = deadline("connect", portals(&bus)).await;
    let open = portals.open_uri().expect("available");
    assert_eq!(
        deadline("open", open.open_path(&path, false, false))
            .await
            .expect("open"),
        OpenOutcome::Opened
    );
    assert_eq!(
        mock.opened_fd_contents(),
        vec!["hello from the sandbox".to_owned()]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn opening_a_path_that_does_not_exist_is_an_error_not_a_panic() {
    let Some(bus) = start_or_skip("opening_a_path_that_does_not_exist_is_an_error_not_a_panic")
    else {
        return;
    };
    let _mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let open = portals.open_uri().expect("available");
    let err = deadline(
        "open",
        open.open_path(
            std::path::Path::new("/nonexistent/compass/file"),
            false,
            false,
        ),
    )
    .await
    .expect_err("must fail cleanly");
    assert!(matches!(err, PortalError::Protocol(_)), "{err:?}");
}

#[tokio::test]
async fn a_dismissed_open_is_an_outcome_not_an_error() {
    let Some(bus) = start_or_skip("a_dismissed_open_is_an_outcome_not_an_error") else {
        return;
    };
    let _mock = MockPortal::start(
        bus.address(),
        MockOptions::default().behaving(Behaviour::Deny),
    )
    .await
    .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let open = portals.open_uri().expect("available");

    assert_eq!(
        deadline("open", open.open_uri("https://example.com/", true))
            .await
            .expect("not an Err"),
        OpenOutcome::Dismissed
    );
}

#[tokio::test]
async fn scheme_supported_is_gated_on_interface_v5() {
    let Some(bus) = start_or_skip("scheme_supported_is_gated_on_interface_v5") else {
        return;
    };
    let _mock = MockPortal::start(bus.address(), MockOptions::default().at_version(4))
        .await
        .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let open = portals.open_uri().expect("available");
    let err = deadline("scheme", open.scheme_supported("http"))
        .await
        .expect_err("v4 has no SchemeSupported");
    assert!(
        matches!(
            err,
            PortalError::VersionTooOld {
                method: "SchemeSupported",
                found: 4,
                required: 5,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn scheme_supported_works_on_interface_v5() {
    let Some(bus) = start_or_skip("scheme_supported_works_on_interface_v5") else {
        return;
    };
    let _mock = MockPortal::start(bus.address(), MockOptions::default().at_version(5))
        .await
        .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let open = portals.open_uri().expect("available");
    assert!(
        deadline("scheme", open.scheme_supported("https"))
            .await
            .expect("supported")
    );
    assert!(
        !deadline("scheme", open.scheme_supported("gopher"))
            .await
            .expect("unsupported")
    );
}

#[tokio::test]
async fn open_uri_is_unavailable_when_the_interface_is_missing() {
    let Some(bus) = start_or_skip("open_uri_is_unavailable_when_the_interface_is_missing") else {
        return;
    };
    let options = MockOptions {
        open_uri: false,
        ..MockOptions::default()
    };
    let _mock = MockPortal::start(bus.address(), options)
        .await
        .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;

    assert_eq!(
        portals.capabilities().open_uri,
        Availability::Unavailable {
            reason: Unavailable::InterfaceMissing
        }
    );
    assert!(portals.open_uri().unwrap_err().is_unavailable());
    assert!(
        portals
            .capabilities()
            .degraded()
            .contains(&DegradedFeature::OpenExternally)
    );
}

// ---------------------------------------------------------------------------
// FileChooser
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_file_chooser_returns_decoded_paths() {
    let Some(bus) = start_or_skip("the_file_chooser_returns_decoded_paths") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_chooser_reply(&[
        "file:///home/user/notes.md",
        "file:///home/user/My%20Documents/r%C3%A9sum%C3%A9.pdf",
    ]);

    let portals = deadline("connect", portals(&bus)).await;
    let chooser = portals.file_chooser().expect("available");
    let outcome = deadline(
        "open",
        chooser.open(FileChooserRequest::file("Pick a file").multiple(true)),
    )
    .await
    .expect("open");

    assert_eq!(
        outcome,
        FileChooserOutcome::Selected {
            paths: vec![
                PathBuf::from("/home/user/notes.md"),
                PathBuf::from("/home/user/My Documents/résumé.pdf"),
            ]
        }
    );
    assert_eq!(mock.chooser_option_bool("multiple"), Some(true));
    assert_eq!(mock.chooser_option_bool("directory"), Some(false));
}

#[tokio::test]
async fn the_file_chooser_passes_directory_mode_through() {
    let Some(bus) = start_or_skip("the_file_chooser_passes_directory_mode_through") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_chooser_reply(&["file:///home/user/Projects"]);

    let portals = deadline("connect", portals(&bus)).await;
    let chooser = portals.file_chooser().expect("available");
    let outcome = deadline(
        "open",
        chooser.open(FileChooserRequest::directory("Pick a folder").starting_in("/home/user")),
    )
    .await
    .expect("open");

    assert_eq!(outcome.paths(), [PathBuf::from("/home/user/Projects")]);
    assert_eq!(mock.chooser_option_bool("directory"), Some(true));
}

#[tokio::test]
async fn a_cancelled_file_chooser_is_an_outcome_not_an_error() {
    let Some(bus) = start_or_skip("a_cancelled_file_chooser_is_an_outcome_not_an_error") else {
        return;
    };
    let _mock = MockPortal::start(
        bus.address(),
        MockOptions::default().behaving(Behaviour::Deny),
    )
    .await
    .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let chooser = portals.file_chooser().expect("available");

    assert_eq!(
        deadline("open", chooser.open(FileChooserRequest::file("Pick")))
            .await
            .expect("not an Err"),
        FileChooserOutcome::Cancelled
    );
}

#[tokio::test]
async fn a_non_file_uri_from_the_chooser_is_a_protocol_error() {
    let Some(bus) = start_or_skip("a_non_file_uri_from_the_chooser_is_a_protocol_error") else {
        return;
    };
    let mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_chooser_reply(&["https://example.com/not-a-file"]);

    let portals = deadline("connect", portals(&bus)).await;
    let chooser = portals.file_chooser().expect("available");
    let err = deadline("open", chooser.open(FileChooserRequest::file("Pick")))
        .await
        .expect_err("a non-file URI is not a path");
    assert!(matches!(err, PortalError::Protocol(_)), "{err:?}");
}

#[tokio::test]
async fn a_file_chooser_that_never_answers_is_bounded() {
    let Some(bus) = start_or_skip("a_file_chooser_that_never_answers_is_bounded") else {
        return;
    };
    let _mock = MockPortal::start(
        bus.address(),
        MockOptions::default().behaving(Behaviour::Silent),
    )
    .await
    .expect("mock");
    let portals = deadline("connect", portals(&bus)).await;
    let chooser = portals.file_chooser().expect("available");

    let err = deadline("open", chooser.open(FileChooserRequest::file("Pick")))
        .await
        .expect_err("must time out");
    assert!(
        matches!(
            err,
            PortalError::Timeout {
                method: "OpenFile",
                ..
            }
        ),
        "{err:?}"
    );
}

// ---------------------------------------------------------------------------
// Doctor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_degradation_explains_itself_for_doctor() {
    let Some(bus) = start_or_skip("every_degradation_explains_itself_for_doctor") else {
        return;
    };
    let portals = deadline("connect", portals(&bus)).await;
    let degraded = portals.capabilities().degraded();
    assert_eq!(
        degraded,
        vec![
            DegradedFeature::GlobalHotkeys,
            DegradedFeature::OpenExternally,
            DegradedFeature::FilePicker,
        ]
    );
    for feature in degraded {
        assert!(!feature.title().is_empty());
        assert!(feature.explanation().len() > 40, "{feature:?}");
    }
    // The rendered availability must name the reason, since `doctor` prints it.
    assert!(
        portals
            .capabilities()
            .global_shortcuts
            .to_string()
            .contains("no portal frontend")
    );
}

#[tokio::test]
async fn re_probing_picks_up_a_backend_that_appears_later() {
    let Some(bus) = start_or_skip("re_probing_picks_up_a_backend_that_appears_later") else {
        return;
    };
    let portals = deadline("connect", portals(&bus)).await;
    assert!(!portals.capabilities().global_shortcuts.is_available());

    let _mock = MockPortal::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    let caps = deadline("re-probe", portals.probe()).await;
    assert!(caps.global_shortcuts.is_available());
    assert!(portals.capabilities().global_shortcuts.is_available());
}
