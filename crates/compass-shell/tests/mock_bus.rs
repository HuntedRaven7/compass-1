//! Suite 3a — the mock GNOME Shell bus (`PLAN.md` §8.4a).
//!
//! Every test here spawns its own real `dbus-daemon --session` and drives the
//! real `ShellClient` against a real `zbus` server implementing the contract.
//! Nothing is stubbed at the Rust level; the bytes go over a unix socket.
//!
//! There is no `sleep` used for synchronisation anywhere: state changes are
//! awaited through the client's own capability and signal streams, and
//! `tokio::time::timeout` is used only as a failure deadline.

mod support;

use std::time::Duration;

use compass_shell::{
    Availability, ClipboardContent, DegradedFeature, ShellClient, ShellConfig, ShellError, WindowId,
};
use support::bus::{TestBus, start_or_skip};
use support::mock::{MockOptions, MockShell, MockWindow};
use zbus::zvariant::Value;

/// Generous deadline: every await below is event-driven, so a healthy run
/// finishes in milliseconds and only a genuine hang hits this.
const DEADLINE: Duration = Duration::from_secs(10);

async fn deadline<F: std::future::Future>(what: &str, fut: F) -> F::Output {
    match tokio::time::timeout(DEADLINE, fut).await {
        Ok(v) => v,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

fn config(bus: &TestBus) -> ShellConfig {
    ShellConfig::default()
        .with_address(bus.address())
        .with_backoff(compass_shell::Backoff {
            initial: Duration::from_millis(5),
            max: Duration::from_millis(50),
            attempts: 8,
        })
        .with_call_timeout(Duration::from_millis(500))
}

async fn client_with(bus: &TestBus) -> ShellClient {
    ShellClient::connect(config(bus)).await.expect("connect")
}

// ---------------------------------------------------------------------------
// Capability detection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn extension_present_reports_available() {
    let Some(bus) = start_or_skip("extension_present_reports_available") else {
        return;
    };
    let _mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");

    let client = client_with(&bus).await;
    let caps = client.capabilities();

    assert_eq!(
        caps.windows,
        Availability::Available {
            version: compass_shell::CONTRACT_VERSION
        }
    );
    assert_eq!(
        caps.clipboard,
        Availability::Available {
            version: compass_shell::CONTRACT_VERSION
        }
    );
    assert!(caps.degraded().is_empty());
}

#[tokio::test]
async fn extension_absent_degrades_without_error() {
    let Some(bus) = start_or_skip("extension_absent_degrades_without_error") else {
        return;
    };
    // Nobody owns org.gnome.Shell on this bus at all.
    let client = client_with(&bus).await;
    let caps = client.capabilities();

    assert_eq!(caps.windows, Availability::Absent);
    assert_eq!(caps.clipboard, Availability::Absent);
    assert!(!caps.any_available());

    // Every call degrades, none panics, none reports a transport failure.
    match client.list_windows().await {
        Err(ShellError::Unavailable(Availability::Absent)) => {}
        other => panic!("expected Unavailable(Absent), got {other:?}"),
    }
    match client.clipboard().await {
        Err(ShellError::Unavailable(Availability::Absent)) => {}
        other => panic!("expected Unavailable(Absent), got {other:?}"),
    }
    match client.activate_window(WindowId(1)).await {
        Err(ShellError::Unavailable(Availability::Absent)) => {}
        other => panic!("expected Unavailable(Absent), got {other:?}"),
    }

    let degraded = caps.degraded();
    assert_eq!(
        degraded,
        vec![
            DegradedFeature::WindowSwitching,
            DegradedFeature::ClipboardHistory,
            DegradedFeature::Paste,
        ]
    );
    for feature in degraded {
        assert!(!feature.explanation().is_empty());
    }
}

#[tokio::test]
async fn shell_present_but_extension_not_loaded_is_absent() {
    let Some(bus) = start_or_skip("shell_present_but_extension_not_loaded_is_absent") else {
        return;
    };
    // gnome-shell is running (the name is owned) but exports neither object:
    // this is the "extension installed but disabled" case, and must be
    // distinguishable from a transport error, not from `Absent`.
    let _mock = MockShell::start(
        bus.address(),
        MockOptions {
            windows: false,
            clipboard: false,
            ..MockOptions::default()
        },
    )
    .await
    .expect("mock");

    let client = client_with(&bus).await;
    assert_eq!(client.capabilities().windows, Availability::Absent);
    assert_eq!(client.capabilities().clipboard, Availability::Absent);
}

#[tokio::test]
async fn version_mismatch_is_distinct_from_absent() {
    let Some(bus) = start_or_skip("version_mismatch_is_distinct_from_absent") else {
        return;
    };
    let future_version = compass_shell::CONTRACT_VERSION + 41;
    let _mock = MockShell::start(
        bus.address(),
        MockOptions {
            version: future_version,
            ..MockOptions::default()
        },
    )
    .await
    .expect("mock");

    let client = client_with(&bus).await;
    let caps = client.capabilities();

    let expected = Availability::VersionMismatch {
        found: future_version,
        expected: compass_shell::CONTRACT_VERSION,
    };
    assert_eq!(caps.windows, expected);
    assert_eq!(caps.clipboard, expected);
    assert_ne!(caps.windows, Availability::Absent);
    assert!(!caps.windows.is_available());

    match client.list_windows().await {
        Err(ShellError::Unavailable(Availability::VersionMismatch { found, expected })) => {
            assert_eq!(found, future_version);
            assert_eq!(expected, compass_shell::CONTRACT_VERSION);
        }
        other => panic!("expected Unavailable(VersionMismatch), got {other:?}"),
    }

    // And the message a user would see says which is which.
    let rendered = caps.windows.to_string();
    assert!(rendered.contains("version mismatch"), "{rendered}");
    assert!(rendered.contains(&future_version.to_string()), "{rendered}");
}

#[tokio::test]
async fn one_sided_extension_degrades_only_that_half() {
    let Some(bus) = start_or_skip("one_sided_extension_degrades_only_that_half") else {
        return;
    };
    let _mock = MockShell::start(
        bus.address(),
        MockOptions {
            clipboard: false,
            ..MockOptions::default()
        },
    )
    .await
    .expect("mock");

    let client = client_with(&bus).await;
    let caps = client.capabilities();
    assert!(caps.windows.is_available());
    assert_eq!(caps.clipboard, Availability::Absent);
    assert_eq!(
        caps.degraded(),
        vec![DegradedFeature::ClipboardHistory, DegradedFeature::Paste]
    );
    // The working half still works.
    assert!(client.list_windows().await.is_ok());
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn window_list_round_trips_empty() {
    let Some(bus) = start_or_skip("window_list_round_trips_empty") else {
        return;
    };
    let _mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");

    let client = client_with(&bus).await;
    assert_eq!(client.list_windows().await.expect("list"), Vec::new());
}

#[tokio::test]
async fn window_list_round_trips_fields() {
    let Some(bus) = start_or_skip("window_list_round_trips_fields") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_windows(vec![
        MockWindow::new(7, "firefox", "Bugzilla — Mozilla Firefox").focused(),
        MockWindow {
            workspace: None,
            pid: None,
            ..MockWindow::new(8, "Alacritty", "")
        },
    ]);

    let client = client_with(&bus).await;
    let windows = client.list_windows().await.expect("list");

    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].id, WindowId(7));
    assert_eq!(windows[0].title, "Bugzilla — Mozilla Firefox");
    assert_eq!(windows[0].wm_class, "firefox");
    assert_eq!(windows[0].wm_class_instance, "firefox");
    assert_eq!(windows[0].pid, Some(1007));
    assert!(windows[0].focused);
    assert_eq!(windows[0].workspace, Some(0));
    assert!(
        windows[0].can_close,
        "can_close defaults to true when omitted"
    );

    assert_eq!(windows[1].id, WindowId(8));
    assert_eq!(windows[1].title, "");
    assert_eq!(windows[1].pid, None);
    assert_eq!(windows[1].workspace, None);
    assert!(!windows[1].focused);
}

#[tokio::test]
async fn window_list_round_trips_many() {
    let Some(bus) = start_or_skip("window_list_round_trips_many") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    let many: Vec<_> = (0..250)
        .map(|i| MockWindow::new(i, &format!("app{i}"), &format!("window {i}")))
        .collect();
    mock.set_windows(many);

    let client = client_with(&bus).await;
    let windows = client.list_windows().await.expect("list");

    assert_eq!(windows.len(), 250);
    assert_eq!(windows[0].id, WindowId(0));
    assert_eq!(windows[249].id, WindowId(249));
    assert_eq!(windows[249].title, "window 249");
}

#[tokio::test]
async fn unknown_dictionary_keys_are_ignored() {
    let Some(bus) = start_or_skip("unknown_dictionary_keys_are_ignored") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    // Forward compatibility: a newer extension may add fields within v1.
    mock.set_windows(vec![
        MockWindow::new(1, "nautilus", "Home")
            .with_raw("gravity", Value::from("downwards"))
            .with_raw("monitor", Value::from(2u32)),
    ]);

    let client = client_with(&bus).await;
    let windows = client.list_windows().await.expect("list");
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].wm_class, "nautilus");
}

#[tokio::test]
async fn activate_reaches_the_mock_with_the_right_argument() {
    let Some(bus) = start_or_skip("activate_reaches_the_mock_with_the_right_argument") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_windows(vec![
        MockWindow::new(11, "a", "A").focused(),
        MockWindow::new(22, "b", "B"),
    ]);

    let client = client_with(&bus).await;
    client
        .activate_window(WindowId(22))
        .await
        .expect("activate");

    assert_eq!(mock.calls(), vec![("ActivateWindow", 22)]);

    let windows = client.list_windows().await.expect("list");
    assert!(!windows[0].focused);
    assert!(windows[1].focused);
}

#[tokio::test]
async fn close_reaches_the_mock_with_the_right_argument() {
    let Some(bus) = start_or_skip("close_reaches_the_mock_with_the_right_argument") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_windows(vec![
        MockWindow::new(11, "a", "A"),
        MockWindow::new(22, "b", "B"),
    ]);

    let client = client_with(&bus).await;
    client.close_window(WindowId(11)).await.expect("close");

    assert_eq!(mock.calls(), vec![("CloseWindow", 11)]);
    let remaining = client.list_windows().await.expect("list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, WindowId(22));
}

// ---------------------------------------------------------------------------
// Malformed replies
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_required_field_is_an_error_not_a_panic() {
    let Some(bus) = start_or_skip("missing_required_field_is_an_error_not_a_panic") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_windows(vec![MockWindow::new(1, "gedit", "Untitled").without("id")]);

    let client = client_with(&bus).await;
    match client.list_windows().await {
        Err(ShellError::Protocol(msg)) => {
            assert!(msg.contains("id"), "{msg}");
            assert!(msg.contains("missing"), "{msg}");
        }
        other => panic!("expected Protocol error, got {other:?}"),
    }

    // The client is still usable afterwards.
    mock.set_windows(vec![MockWindow::new(1, "gedit", "Untitled")]);
    assert_eq!(client.list_windows().await.expect("recovered").len(), 1);
}

#[tokio::test]
async fn wrongly_typed_field_is_an_error_not_a_panic() {
    let Some(bus) = start_or_skip("wrongly_typed_field_is_an_error_not_a_panic") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    // `id` as a string rather than a u32 — the classic GJS mistake.
    mock.set_windows(vec![
        MockWindow::new(1, "gedit", "Untitled").with_raw("id", Value::from("one")),
    ]);

    let client = client_with(&bus).await;
    match client.list_windows().await {
        Err(ShellError::Protocol(msg)) => assert!(msg.contains("`id`"), "{msg}"),
        other => panic!("expected Protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn unexpected_reply_signature_is_an_error_not_a_panic() {
    let Some(bus) = start_or_skip("unexpected_reply_signature_is_an_error_not_a_panic") else {
        return;
    };
    // Same object path, but the interface answers `ListWindows` with a plain
    // string (what the old, unversioned JSON contract did).
    let _mock = support::mock::start_wrong_signature_shell(bus.address())
        .await
        .expect("mock");

    let client = client_with(&bus).await;
    match client.list_windows().await {
        Err(ShellError::Call(_)) => {}
        other => panic!("expected a Call error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unresponsive_shell_times_out_rather_than_hanging() {
    let Some(bus) = start_or_skip("an_unresponsive_shell_times_out_rather_than_hanging") else {
        return;
    };
    // The name is owned but nothing ever answers: a wedged gnome-shell.
    // D-Bus method calls have no inherent timeout, so without the client's own
    // deadline this would hang the launcher forever.
    let _mock = support::mock::start_unresponsive_shell(bus.address())
        .await
        .expect("mock");

    let client = client_with(&bus).await;
    // Probing an unresponsive shell degrades rather than blocking.
    assert_eq!(client.capabilities().windows, Availability::Absent);
    match client.list_windows().await {
        Err(ShellError::Unavailable(Availability::Absent)) => {}
        other => panic!("expected Unavailable(Absent), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn windows_changed_signal_is_observed() {
    let Some(bus) = start_or_skip("windows_changed_signal_is_observed") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");

    let client = client_with(&bus).await;
    // The match rule is installed by the time this returns, so the emit below
    // cannot race us. No sleeps required.
    let mut changes = client.windows_changed().await.expect("subscribe");

    mock.set_windows(vec![MockWindow::new(3, "kitty", "kitty")]);
    mock.emit_windows_changed().await.expect("emit");

    deadline("WindowsChanged", changes.next())
        .await
        .expect("a signal");

    assert_eq!(client.list_windows().await.expect("list").len(), 1);

    mock.emit_windows_changed().await.expect("emit");
    deadline("second WindowsChanged", changes.next())
        .await
        .expect("a second signal");
}

#[tokio::test]
async fn clipboard_changed_signal_carries_the_payload() {
    let Some(bus) = start_or_skip("clipboard_changed_signal_carries_the_payload") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");

    let client = client_with(&bus).await;
    let mut changes = client.clipboard_changes().await.expect("subscribe");

    mock.emit_clipboard_changed(
        b"hello bus",
        "text/plain;charset=utf-8",
        "org.gnome.TextEditor",
    )
    .await
    .expect("emit");

    let change = deadline("ClipboardChanged", changes.next())
        .await
        .expect("a signal")
        .expect("well-formed");
    assert_eq!(change.content.data, b"hello bus");
    assert_eq!(change.content.mime_type, "text/plain;charset=utf-8");
    assert_eq!(change.content.as_text(), Some("hello bus"));
    assert_eq!(change.source_app.as_deref(), Some("org.gnome.TextEditor"));

    // Binary payloads and an unknown source app.
    mock.emit_clipboard_changed(&[0x89, b'P', b'N', b'G', 0x00, 0xff], "image/png", "")
        .await
        .expect("emit");
    let change = deadline("binary ClipboardChanged", changes.next())
        .await
        .expect("a signal")
        .expect("well-formed");
    assert_eq!(
        change.content.data,
        vec![0x89, b'P', b'N', b'G', 0x00, 0xff]
    );
    assert_eq!(change.content.mime_type, "image/png");
    assert_eq!(change.content.as_text(), None);
    assert_eq!(change.source_app, None);
}

#[tokio::test]
async fn clipboard_get_and_set_round_trip() {
    let Some(bus) = start_or_skip("clipboard_get_and_set_round_trip") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_clipboard(b"initial", "text/plain;charset=utf-8");

    let client = client_with(&bus).await;
    let content = client.clipboard().await.expect("get");
    assert_eq!(content.as_text(), Some("initial"));

    client
        .set_clipboard(&ClipboardContent::binary(
            vec![1, 2, 3],
            "application/octet-stream",
        ))
        .await
        .expect("set");
    let content = client.clipboard().await.expect("get");
    assert_eq!(content.data, vec![1, 2, 3]);
    assert_eq!(content.mime_type, "application/octet-stream");

    client
        .set_clipboard(&ClipboardContent::text("round trip"))
        .await
        .expect("set");
    assert_eq!(
        client.clipboard().await.expect("get").as_text(),
        Some("round trip")
    );
}

// ---------------------------------------------------------------------------
// Reconnection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shell_disappearing_is_detected_and_reconnected() {
    let Some(bus) = start_or_skip("shell_disappearing_is_detected_and_reconnected") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_windows(vec![MockWindow::new(1, "before", "before")]);

    let client = client_with(&bus).await;
    assert!(client.capabilities().windows.is_available());

    // Subscribe before the disconnect so the transition cannot be missed.
    let mut caps = client.capability_changes();

    // gnome-shell dies: the bus name is released.
    mock.shutdown().await;

    let lost = deadline("capability loss", caps.next())
        .await
        .expect("a transition");
    assert_eq!(lost.windows, Availability::Absent);
    assert_eq!(lost.clipboard, Availability::Absent);
    assert_eq!(client.capabilities().windows, Availability::Absent);

    match client.list_windows().await {
        Err(ShellError::Unavailable(Availability::Absent)) => {}
        other => panic!("expected Unavailable while the shell is down, got {other:?}"),
    }

    // gnome-shell comes back, as it does after every extension reload.
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock restart");
    mock.set_windows(vec![MockWindow::new(2, "after", "after")]);

    let regained = deadline("capability recovery", caps.next())
        .await
        .expect("a transition");
    assert!(regained.windows.is_available());
    assert!(regained.clipboard.is_available());

    let windows = client.list_windows().await.expect("list after reconnect");
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].wm_class, "after");

    // Signals work again on the new connection too.
    let mut changes = client.windows_changed().await.expect("resubscribe");
    mock.emit_windows_changed().await.expect("emit");
    deadline("post-reconnect WindowsChanged", changes.next())
        .await
        .expect("a signal");
}

#[tokio::test]
async fn extension_enabled_after_connect_is_picked_up() {
    let Some(bus) = start_or_skip("extension_enabled_after_connect_is_picked_up") else {
        return;
    };
    // Client starts against a bus with no shell at all.
    let client = client_with(&bus).await;
    assert_eq!(client.capabilities().windows, Availability::Absent);

    let mut caps = client.capability_changes();
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    mock.set_windows(vec![MockWindow::new(5, "late", "late")]);

    let gained = deadline("capability acquisition", caps.next())
        .await
        .expect("a transition");
    assert!(gained.windows.is_available());
    assert_eq!(client.list_windows().await.expect("list").len(), 1);
}

#[tokio::test]
async fn version_mismatch_survives_a_shell_restart() {
    let Some(bus) = start_or_skip("version_mismatch_survives_a_shell_restart") else {
        return;
    };
    let mock = MockShell::start(bus.address(), MockOptions::default())
        .await
        .expect("mock");
    let client = client_with(&bus).await;
    assert!(client.capabilities().windows.is_available());

    let mut caps = client.capability_changes();
    mock.shutdown().await;
    deadline("loss", caps.next()).await.expect("a transition");

    // The user updated GNOME; the extension now speaks a different contract.
    let _mock = MockShell::start(
        bus.address(),
        MockOptions {
            version: 99,
            ..MockOptions::default()
        },
    )
    .await
    .expect("mock restart");

    let after = deadline("post-restart probe", caps.next())
        .await
        .expect("a transition");
    assert_eq!(
        after.windows,
        Availability::VersionMismatch {
            found: 99,
            expected: compass_shell::CONTRACT_VERSION
        }
    );
}

// ---------------------------------------------------------------------------
// The contract itself
// ---------------------------------------------------------------------------

#[test]
fn checked_in_xml_matches_what_the_client_talks_to() {
    // The XML is the reviewable artefact shared with the extension repo; if it
    // and the proxies drift, this is where it shows up.
    for (xml, iface, methods) in [
        (
            compass_shell::contract::WINDOWS_XML,
            compass_shell::WINDOWS_INTERFACE,
            &["ListWindows", "ActivateWindow", "CloseWindow"][..],
        ),
        (
            compass_shell::contract::CLIPBOARD_XML,
            compass_shell::CLIPBOARD_INTERFACE,
            &["GetClipboard", "SetClipboard"][..],
        ),
    ] {
        assert!(
            xml.contains(&format!("<interface name=\"{iface}\">")),
            "{iface} missing from its XML"
        );
        assert!(
            xml.contains("<property name=\"Version\" type=\"u\" access=\"read\"/>"),
            "{iface} must expose a Version property"
        );
        for method in methods {
            assert!(
                xml.contains(&format!("<method name=\"{method}\">")),
                "{iface} missing method {method}"
            );
        }
    }
    assert!(
        compass_shell::contract::WINDOWS_XML.contains("<signal name=\"WindowsChanged\"/>"),
        "windows contract must carry WindowsChanged"
    );
    assert!(
        compass_shell::contract::CLIPBOARD_XML.contains("<signal name=\"ClipboardChanged\">"),
        "clipboard contract must carry ClipboardChanged"
    );
}
