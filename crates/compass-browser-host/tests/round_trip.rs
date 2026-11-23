//! The prototype's actual proof: connect, list and focus across a *process*
//! boundary, over the same stdin/stdout a browser uses.
//!
//! A unit test calling `serve` in-process would prove the protocol but not the
//! thing ADR-0022 chose: that a separately spawned host can be reached by the
//! browser. This spawns the real binary and speaks native messaging to it.

use std::io::Write;
use std::process::{Command, Stdio};

use compass_browser_host::{HostRequest, HostResponse, PROTOCOL_VERSION, read_frame, write_frame};

fn start_host() -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_compass-browser-host"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the native host")
}

fn ask(child: &mut std::process::Child, request: &HostRequest) -> HostResponse {
    let mut stdin = child.stdin.take().expect("stdin");
    write_frame(&mut stdin, &serde_json::to_vec(request).unwrap()).unwrap();
    stdin.flush().unwrap();
    child.stdin = Some(stdin);

    let mut stdout = child.stdout.take().expect("stdout");
    let bytes = read_frame(&mut stdout).expect("the host answered in a frame");
    child.stdout = Some(stdout);
    serde_json::from_slice(&bytes).expect("a valid reply")
}

#[test]
fn a_browser_can_connect_list_and_focus_across_the_process_boundary() {
    let mut child = start_host();

    let connected = ask(
        &mut child,
        &HostRequest::Connect {
            version: PROTOCOL_VERSION,
        },
    );
    assert_eq!(
        connected,
        HostResponse::Connected {
            version: PROTOCOL_VERSION,
            browser_id: "org.tunaos.compass".to_owned(),
        }
    );

    let tabs = ask(&mut child, &HostRequest::GetTabs);
    assert_eq!(tabs, HostResponse::Tabs { tabs: Vec::new() });

    let focused = ask(
        &mut child,
        &HostRequest::FocusTab {
            browser_id: "firefox".to_owned(),
            tab_id: 7,
        },
    );
    assert!(matches!(focused, HostResponse::Error { .. }));

    // The browser closes the pipe; the host exits cleanly rather than hanging.
    drop(child.stdin.take());
    let status = child.wait_with_output().expect("wait for the host");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
}

#[test]
fn a_bad_version_does_not_kill_the_host() {
    let mut child = start_host();
    let bad = ask(&mut child, &HostRequest::Connect { version: 99 });
    assert!(matches!(bad, HostResponse::Error { .. }));
    let good = ask(
        &mut child,
        &HostRequest::Connect {
            version: PROTOCOL_VERSION,
        },
    );
    assert!(matches!(good, HostResponse::Connected { .. }));
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}
