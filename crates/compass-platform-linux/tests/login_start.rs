//! Starting at login against a mock Background portal on a private bus, and
//! the autostart-entry fallback without one.
//!
//! The mock is compass-portals' own: a real `zbus` server on a real
//! `dbus-daemon`, so the request the desktop would see is the one checked.

#[path = "../../compass-portals/tests/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use compass_core::login_start::{self, COMMAND, Entry, Mechanism, State, Unit};
use compass_platform::LoginStarter;
use compass_platform_linux::LinuxLoginStart;
use compass_portals::PortalConfig;
use support::bus::{TestBus, start_or_skip};
use support::mock::{Behaviour, MockOptions, MockPortal};

const SHORT: Duration = Duration::from_secs(2);

fn starter(address: &str, sandboxed: bool, dir: &Path) -> LinuxLoginStart {
    LinuxLoginStart::new(
        PortalConfig::default()
            .with_address(address)
            .with_call_timeout(SHORT)
            .with_dialog_timeout(SHORT),
        sandboxed,
        dir.join("state/login-start.json"),
        dir.join("config"),
        vec![dir.join("config/systemd/user")],
    )
}

fn entry(dir: &Path) -> PathBuf {
    login_start::autostart_path(&dir.join("config"))
}

async fn with_portal(name: &str, options: MockOptions) -> Option<(TestBus, MockPortal)> {
    let bus = start_or_skip(name)?;
    let mock = MockPortal::start(bus.address(), options)
        .await
        .expect("mock");
    Some((bus, mock))
}

#[tokio::test]
async fn the_portal_starts_compass_at_login_and_stops_it_again() {
    let Some((bus, mock)) = with_portal(
        "the_portal_starts_compass_at_login_and_stops_it_again",
        MockOptions::default(),
    )
    .await
    else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // A fallback entry from a session that had no portal.
    login_start::write_autostart(&entry(dir.path())).unwrap();
    let starter = starter(bus.address(), false, dir.path());
    assert_eq!(
        starter.status().await.unwrap().state,
        State::Off,
        "off until chosen, whatever an old entry says"
    );

    let on = State::On {
        mechanism: Mechanism::Portal,
    };
    assert_eq!(starter.set(true).await.unwrap().state, on);
    assert_eq!(
        login_start::autostart_owner(&entry(dir.path())),
        Entry::Absent,
        "one start at login: the portal's, not Compass's own entry too"
    );
    assert_eq!(starter.set(true).await.unwrap().state, on, "idempotent");
    assert_eq!(starter.status().await.unwrap().state, on, "recorded");

    assert_eq!(starter.set(false).await.unwrap().state, State::Off);
    assert_eq!(starter.set(false).await.unwrap().state, State::Off);
    assert_eq!(starter.status().await.unwrap().state, State::Off);

    let command: Vec<String> = COMMAND.iter().map(|word| (*word).to_owned()).collect();
    let requests = mock.background_requests();
    let asked: Vec<Option<bool>> = requests.iter().map(|(autostart, ..)| *autostart).collect();
    assert_eq!(asked, [Some(true), Some(true), Some(false), Some(false)]);
    for (_, commandline, reason) in requests {
        assert_eq!(commandline, command, "compass start --hidden");
        assert_eq!(reason.as_deref(), Some(login_start::REASON));
    }
}

#[tokio::test]
async fn a_refusal_is_shown_as_such_and_never_as_on() {
    let Some((bus, mock)) = with_portal(
        "a_refusal_is_shown_as_such_and_never_as_on",
        MockOptions::default(),
    )
    .await
    else {
        return;
    };
    mock.refuse_autostart();
    let dir = tempfile::tempdir().unwrap();
    let starter = starter(bus.address(), true, dir.path());
    assert_eq!(starter.set(true).await.unwrap().state, State::Refused);
    assert_eq!(starter.status().await.unwrap().state, State::Refused);
}

#[tokio::test]
async fn a_declined_dialog_is_a_refusal() {
    let Some((bus, _mock)) = with_portal(
        "a_declined_dialog_is_a_refusal",
        MockOptions::default().behaving(Behaviour::Deny),
    )
    .await
    else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let starter = starter(bus.address(), true, dir.path());
    assert_eq!(starter.set(true).await.unwrap().state, State::Refused);
}

#[tokio::test]
async fn a_failed_call_changes_nothing_and_says_why() {
    let Some((bus, _mock)) = with_portal(
        "a_failed_call_changes_nothing_and_says_why",
        MockOptions::default().behaving(Behaviour::Silent),
    )
    .await
    else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let starter = starter(bus.address(), true, dir.path());
    let error = starter.set(true).await.expect_err("timed out");
    assert!(
        error.starts_with("Asking your desktop's Background portal failed"),
        "{error}"
    );
    assert_eq!(starter.status().await.unwrap().state, State::Off);
}

#[tokio::test]
async fn the_flatpak_without_a_portal_is_unavailable_and_writes_nothing() {
    let Some((bus, _mock)) = with_portal(
        "the_flatpak_without_a_portal_is_unavailable_and_writes_nothing",
        MockOptions {
            background: false,
            ..MockOptions::default()
        },
    )
    .await
    else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let starter = starter(bus.address(), true, dir.path());
    assert_eq!(starter.status().await.unwrap().state, State::Unavailable);
    assert_eq!(starter.set(true).await.unwrap().state, State::Unavailable);
    assert_eq!(
        login_start::autostart_owner(&entry(dir.path())),
        Entry::Absent
    );
}

#[tokio::test]
async fn without_a_portal_an_unsandboxed_install_writes_and_removes_its_own_entry() {
    let dir = tempfile::tempdir().unwrap();
    let starter = starter("unix:path=/nonexistent/bus", false, dir.path());
    let on = State::On {
        mechanism: Mechanism::AutostartFile,
    };
    assert_eq!(starter.set(true).await.unwrap().state, on);
    assert_eq!(starter.set(true).await.unwrap().state, on, "idempotent");
    assert_eq!(
        login_start::autostart_owner(&entry(dir.path())),
        Entry::Ours
    );
    assert_eq!(starter.status().await.unwrap().state, on);

    // Deleted by hand: the switch follows.
    std::fs::remove_file(entry(dir.path())).unwrap();
    assert_eq!(starter.status().await.unwrap().state, State::Off);

    assert_eq!(starter.set(true).await.unwrap().state, on);
    assert_eq!(starter.set(false).await.unwrap().state, State::Off);
    assert_eq!(
        login_start::autostart_owner(&entry(dir.path())),
        Entry::Absent
    );

    // Someone else's entry is neither replaced nor removed.
    let theirs = "[Desktop Entry]\nType=Application\nExec=compass start\n";
    std::fs::write(entry(dir.path()), theirs).unwrap();
    assert!(starter.set(true).await.is_err());
    assert_eq!(starter.set(false).await.unwrap().state, State::Off);
    assert_eq!(std::fs::read_to_string(entry(dir.path())).unwrap(), theirs);
}

#[tokio::test]
async fn the_packaged_unit_is_reported_beside_the_switch() {
    let dir = tempfile::tempdir().unwrap();
    let wants = dir
        .path()
        .join("config/systemd/user/graphical-session.target.wants");
    std::fs::create_dir_all(&wants).unwrap();
    std::fs::write(wants.join(login_start::SYSTEMD_UNIT), "[Unit]\n").unwrap();
    let starter = starter("unix:path=/nonexistent/bus", false, dir.path());
    let status = starter.status().await.unwrap();
    assert_eq!(status.state, State::Off, "the unit is not Compass's switch");
    assert_eq!(status.unit, Some(Unit::Enabled));
    assert_eq!(
        starter.set(false).await.unwrap().unit,
        Some(Unit::Enabled),
        "and turning Compass's off leaves it alone"
    );
    assert!(wants.join(login_start::SYSTEMD_UNIT).exists());
}
