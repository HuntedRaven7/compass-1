//! The first-party examples in `extensions/rhai-examples/` are part of the
//! product: the tier ships only while they are good enough to copy (PLAN
//! Phase 5, Track C). These tests keep them working and pin what each one
//! demonstrates.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{Fixture, fire, runtime, titles};
use compass_extension_api::{ActionEffect, ActionResponse, CapabilityRegistry, View, ViewTree};
use compass_script::{HostCall, Limits, MemoryHost, ProcessOutcome, ScriptInstance, discovery};

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions/rhai-examples")
}

/// Loads one example with everything it declares granted.
fn example(name: &str) -> Fixture {
    let script = discovery::load(&examples_dir().join(name)).expect("example loads");
    let mut registry = CapabilityRegistry::new();
    script.manifest.declare_into(&mut registry);
    for cap in &script.manifest.capabilities {
        registry.grant(&script.manifest.id, cap).expect("grantable");
    }
    let host = Arc::new(MemoryHost::new());
    let instance = ScriptInstance::load(&script, &registry, host.clone(), Limits::default())
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    Fixture {
        instance,
        host,
        registry,
    }
}

fn completed(response: &ActionResponse) -> &[ActionEffect] {
    match response {
        ActionResponse::Completed { effects, .. } => effects,
        ActionResponse::Failed { error, .. } => panic!("action failed: {error}"),
    }
}

#[test]
fn at_least_four_examples_ship_and_every_one_loads_with_what_it_declares() {
    let scan = discovery::scan(&[examples_dir()]);
    assert!(scan.failed.is_empty(), "{:?}", scan.failed);
    assert!(scan.scripts.len() >= 4, "found {}", scan.scripts.len());
    let rt = runtime();
    for script in &scan.scripts {
        let name = script
            .manifest
            .directory
            .file_name()
            .unwrap()
            .to_string_lossy();
        assert!(script.manifest.icon.is_some(), "{name} has no icon");
        assert!(
            script.manifest.description.is_some(),
            "{name} has no description"
        );
        assert!(
            script.source.lines().take(3).any(|l| l.starts_with("//")),
            "{name} does not open with a comment saying what it shows"
        );
        let fixture = example(&name);
        for query in ["", "a", "10", "uuid 2", "lorem", "10 km"] {
            rt.block_on(fixture.instance.search(query))
                .unwrap_or_else(|error| panic!("{name} on {query:?}: {error}"));
        }
    }
}

#[test]
fn unit_converter_puts_the_asked_for_unit_first_and_copies_it() {
    let fixture = example("unit-converter");
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("10 km to mi")).unwrap();
    let rows = titles(&tree);
    assert_eq!(rows[0], "6.2137 mi");
    assert!(rows.contains(&"10000 m".to_owned()), "{rows:?}");
    assert!(
        !rows.iter().any(|r| r.ends_with(" kg")),
        "only lengths: {rows:?}"
    );

    let response = rt.block_on(fire(&fixture, &tree, "Copy Result"));
    assert_eq!(completed(&response), [ActionEffect::CloseWindow]);
    assert_eq!(fixture.host.clipboard().as_deref(), Some("6.2137"));

    let tree = rt.block_on(fixture.instance.search("212f in c")).unwrap();
    assert_eq!(titles(&tree)[0], "100 c");
    let tree = rt.block_on(fixture.instance.search("hello")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    assert!(list.empty_state.is_some());
}

#[test]
fn epoch_converter_goes_both_ways_and_reads_milliseconds() {
    let fixture = example("epoch-converter");
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("1700000000")).unwrap();
    assert_eq!(titles(&tree)[0], "2023-11-14T22:13:20Z");
    let tree = rt
        .block_on(fixture.instance.search("1700000000000"))
        .unwrap();
    assert_eq!(titles(&tree)[0], "2023-11-14T22:13:20Z");
    let tree = rt
        .block_on(fixture.instance.search(" 2023-11-14T22:13:20Z "))
        .unwrap();
    assert!(titles(&tree).contains(&"1700000000".to_owned()));
    let tree = rt.block_on(fixture.instance.search("2024-03-01")).unwrap();
    assert!(titles(&tree).contains(&"1709251200".to_owned()));
    assert!(
        titles(&tree)
            .iter()
            .any(|t| t.starts_with("Friday, 01 March 2024")),
        "{:?}",
        titles(&tree)
    );
    let tree = rt.block_on(fixture.instance.search("not a date")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    assert!(list.empty_state.is_some());
}

#[test]
fn generators_make_what_was_asked_for_and_paste_it() {
    let fixture = example("generators");
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("uuid 5")).unwrap();
    let ids = titles(&tree);
    assert_eq!(ids.len(), 5);
    for id in &ids {
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4", "version 4: {id}");
    }
    let tree = rt.block_on(fixture.instance.search("password 32")).unwrap();
    assert_eq!(titles(&tree)[0].chars().count(), 32);
    let response = rt.block_on(fire(&fixture, &tree, "Paste"));
    assert_eq!(completed(&response), [ActionEffect::CloseWindow]);
    assert!(matches!(&fixture.host.calls()[..], [HostCall::Paste(p)] if p.chars().count() == 32));

    let tree = rt.block_on(fixture.instance.search("lorem 2")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    assert!(list.show_detail);
    let text = list.sections[0].items[0]
        .detail
        .as_ref()
        .and_then(|d| d.markdown.clone())
        .unwrap();
    assert_eq!(text.split("\n\n").count(), 2);
}

#[test]
fn quick_notes_adds_filters_and_deletes_through_storage() {
    let fixture = example("quick-notes");
    let rt = runtime();
    let id = fixture.instance.manifest().id.clone();

    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert!(titles(&tree).is_empty());

    for text in ["buy milk", "call Ada"] {
        let tree = rt.block_on(fixture.instance.search(text)).unwrap();
        let response = rt.block_on(fire(&fixture, &tree, "Add Note"));
        assert!(completed(&response).contains(&ActionEffect::Rerender));
    }
    assert!(fixture.host.storage(&id).contains_key("notes"));

    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert_eq!(titles(&tree), ["call Ada", "buy milk"], "newest first");
    let tree = rt.block_on(fixture.instance.search("milk")).unwrap();
    assert_eq!(titles(&tree), ["Add note: milk", "buy milk"]);

    let response = rt.block_on(fire(&fixture, &tree, "Delete Note"));
    assert!(completed(&response).contains(&ActionEffect::Rerender));
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert_eq!(titles(&tree), ["call Ada"]);
}

#[test]
fn web_search_opens_an_encoded_url() {
    let fixture = example("web-search");
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("rust & rhai")).unwrap();
    let response = rt.block_on(fire(&fixture, &tree, "Open in Browser"));
    assert_eq!(completed(&response), [ActionEffect::CloseWindow]);
    assert_eq!(
        fixture.host.calls(),
        [HostCall::Open(
            "https://duckduckgo.com/?q=rust%20%26%20rhai".to_owned()
        )]
    );
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert!(tree.actions().is_empty(), "nothing to open without a query");
}

/// The command lines `system-health` runs, as `MemoryHost` keys them.
const IS_SYSTEM_RUNNING: &[&str] = &["systemctl", "is-system-running"];
const FAILED_SYSTEM_UNITS: &[&str] = &[
    "systemctl",
    "list-units",
    "--failed",
    "--plain",
    "--no-legend",
    "--no-pager",
];
const FAILED_USER_UNITS: &[&str] = &[
    "systemctl",
    "--user",
    "list-units",
    "--failed",
    "--plain",
    "--no-legend",
    "--no-pager",
];

fn exited(code: i32, stdout: &str) -> ProcessOutcome {
    ProcessOutcome::Exited {
        code: Some(code),
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

/// Every row in a list tree as `(title, subtitle)`.
fn rows(tree: &ViewTree) -> Vec<(String, Option<String>)> {
    let View::List(list) = tree.root() else {
        panic!("not a list: {:?}", tree.root());
    };
    list.sections
        .iter()
        .flat_map(|s| &s.items)
        .map(|i| (i.title.clone(), i.subtitle.clone()))
        .collect()
}

/// The command lines the host was asked to run, each with its timeout.
fn runs(host: &MemoryHost) -> Vec<(Vec<String>, Duration)> {
    host.calls()
        .into_iter()
        .filter_map(|call| match call {
            HostCall::Run { argv, timeout } => Some((argv, timeout)),
            _ => None,
        })
        .collect()
}

#[test]
fn system_health_lists_failed_units_and_update_tools_and_copies_a_follow_up() {
    let fixture = example("system-health");
    let host = &fixture.host;
    // `is-system-running` exits 1 for any state but "running".
    host.set_program(IS_SYSTEM_RUNNING, exited(1, "degraded\n"));
    host.set_program(
        FAILED_SYSTEM_UNITS,
        exited(
            0,
            "systemd-networkd-wait-online.service loaded failed failed Wait for Network to be Configured\n\
             backup.timer                         loaded failed failed Nightly  backup\n",
        ),
    );
    host.set_program(FAILED_USER_UNITS, exited(0, ""));
    host.set_program(&["bootc", "--version"], exited(0, "bootc 1.1.4\n"));
    host.set_program(
        &["rpm-ostree", "--version"],
        exited(0, "rpm-ostree:\n Version: '2024.7'\n Git: 7f8a\n"),
    );
    // flatpak is not installed: no answer set.

    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    let View::List(list) = tree.root() else {
        panic!("not a list");
    };
    assert!(
        list.search.host_filtering,
        "the launcher filters, so typing never runs the probes again"
    );
    assert_eq!(
        list.sections
            .iter()
            .map(|s| s.title.as_deref().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["Failed System Units", "Failed User Units", "Updates"]
    );
    assert_eq!(
        list.sections[0].subtitle.as_deref(),
        Some("State: degraded")
    );
    let some = |text: &str| Some(text.to_owned());
    assert_eq!(
        rows(&tree),
        [
            (
                "systemd-networkd-wait-online.service".to_owned(),
                some("Wait for Network to be Configured")
            ),
            ("backup.timer".to_owned(), some("Nightly  backup")),
            (
                "No failed units".to_owned(),
                some("Every user unit is running or stopped cleanly.")
            ),
            (
                "bootc".to_owned(),
                some("Updates the operating system image")
            ),
            (
                "rpm-ostree".to_owned(),
                some("Updates the operating system image")
            ),
            ("flatpak".to_owned(), some("Updates Flatpak applications")),
        ]
    );
    let versions: Vec<Option<String>> = list.sections[2]
        .items
        .iter()
        .map(|item| item.accessories.first().and_then(|a| a.text.clone()))
        .collect();
    assert_eq!(versions, [some("1.1.4"), some("2024.7"), None]);
    let tags: Vec<Option<String>> = list.sections[2]
        .items
        .iter()
        .map(|item| item.accessories.last().and_then(|a| a.tag.clone()))
        .collect();
    assert_eq!(
        tags,
        [some("Available"), some("Available"), some("Not installed")]
    );

    // Every program had a timeout, none longer than the script asked for.
    let ran = runs(host);
    assert_eq!(ran.len(), 6, "{ran:?}");
    assert!(
        ran.iter().all(|(_, t)| *t <= Duration::from_millis(500)),
        "{ran:?}"
    );

    let response = rt.block_on(fire(&fixture, &tree, "Copy Status Command"));
    assert_eq!(completed(&response), [ActionEffect::CloseWindow]);
    assert_eq!(
        fixture.host.clipboard().as_deref(),
        Some("systemctl status systemd-networkd-wait-online.service")
    );

    let response = rt.block_on(fire(&fixture, &tree, "Refresh"));
    assert!(completed(&response).contains(&ActionEffect::Rerender));
}

#[test]
fn system_health_says_unsupported_without_systemd() {
    let rt = runtime();

    // No systemctl at all.
    let fixture = example("system-health");
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    let all = rows(&tree);
    assert_eq!(
        all[0],
        (
            "systemd is unsupported here".to_owned(),
            Some("systemctl is not installed, so this system does not use systemd.".to_owned())
        )
    );
    assert_eq!(
        titles(&tree)[1..],
        ["bootc", "rpm-ostree", "flatpak"],
        "the update tools are still checked"
    );
    assert!(
        runs(&fixture.host)
            .iter()
            .all(|(argv, _)| argv[..] != *FAILED_SYSTEM_UNITS),
        "no unit list is asked for"
    );

    // systemctl installed, but systemd is not what started the system.
    let fixture = example("system-health");
    fixture.host.set_program(
        IS_SYSTEM_RUNNING,
        ProcessOutcome::Exited {
            code: Some(1),
            stdout: "offline\n".to_owned(),
            stderr:
                "System has not been booted with systemd as init system (PID 1). Can't operate.\n"
                    .to_owned(),
        },
    );
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert_eq!(
        rows(&tree)[0],
        (
            "systemd is unsupported here".to_owned(),
            Some("This system was not started by systemd, so it has no units to check.".to_owned())
        )
    );
}

#[test]
fn system_health_reports_an_unresponsive_bus_and_stops_asking_it() {
    let fixture = example("system-health");
    fixture
        .host
        .set_program(IS_SYSTEM_RUNNING, ProcessOutcome::TimedOut);
    fixture
        .host
        .set_program(&["flatpak", "--version"], exited(0, "Flatpak 1.14.6\n"));
    // A tool that a signal ended has no exit code; it reads as unavailable.
    fixture.host.set_program(
        &["bootc", "--version"],
        ProcessOutcome::Exited {
            code: None,
            stdout: String::new(),
            stderr: "Killed\n".to_owned(),
        },
    );
    fixture
        .host
        .set_program(&["rpm-ostree", "--version"], ProcessOutcome::TimedOut);
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert_eq!(
        rows(&tree)[0],
        (
            "systemd did not answer".to_owned(),
            Some("It did not reply in time. Press Ctrl+R to try again.".to_owned())
        )
    );
    let View::List(list) = tree.root() else {
        panic!("not a list");
    };
    let tags: Vec<Option<&str>> = list.sections[1]
        .items
        .iter()
        .map(|item| item.accessories.last().and_then(|a| a.tag.as_deref()))
        .collect();
    assert_eq!(
        tags,
        [Some("Unavailable"), Some("Timed out"), Some("Available")]
    );
    let asked: Vec<String> = runs(&fixture.host)
        .into_iter()
        .map(|(argv, _)| argv.join(" "))
        .collect();
    assert_eq!(
        asked,
        [
            "systemctl is-system-running",
            "bootc --version",
            "rpm-ostree --version",
            "flatpak --version"
        ],
        "a bus that did not answer once is not asked for the unit lists"
    );
}
