//! Scanning, ids, precedence and visibility.

mod support;

use compass_core::apps::{SkipReason, desktop_file_id};
use compass_testkit::corpus;
use std::path::Path;
use support::{app, builder, write};

#[test]
fn indexes_a_flat_directory() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "firefox.desktop", &app("Firefox", "firefox %u"));
    write(dir.path(), "gimp.desktop", &app("GIMP", "gimp"));

    let index = builder().dir(dir.path()).build();

    assert_eq!(index.len(), 2);
    let mut names: Vec<&str> = index.items().iter().map(|i| i.name()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Firefox", "GIMP"]);
}

#[test]
fn desktop_ids_flatten_subdirectories() {
    let root = Path::new("/usr/share/applications");
    assert_eq!(
        desktop_file_id(root, Path::new("/usr/share/applications/konsole.desktop")).as_deref(),
        Some("konsole.desktop"),
    );
    assert_eq!(
        desktop_file_id(
            root,
            Path::new("/usr/share/applications/kde4/konsole.desktop")
        )
        .as_deref(),
        Some("kde4-konsole.desktop"),
    );
    assert_eq!(
        desktop_file_id(root, Path::new("/elsewhere/konsole.desktop")),
        None,
    );
}

#[test]
fn nested_entries_are_indexed_with_a_flattened_id() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "kde4/konsole.desktop",
        &app("Konsole", "konsole"),
    );

    let index = builder().dir(dir.path()).build();

    assert_eq!(index.len(), 1);
    assert_eq!(index.items()[0].desktop_id(), "kde4-konsole.desktop");
    assert!(index.get("kde4-konsole.desktop").is_some());
}

/// The bug this whole ordering exists to prevent: a user override must replace the system entry,
/// not sit next to it.
#[test]
fn an_earlier_directory_shadows_a_later_one_with_the_same_id() {
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();

    write(
        user.path(),
        "firefox.desktop",
        &app("Firefox (user override)", "firefox-nightly %u"),
    );
    write(
        system.path(),
        "firefox.desktop",
        &app("Firefox", "firefox %u"),
    );
    write(system.path(), "gimp.desktop", &app("GIMP", "gimp"));

    let index = builder().dir(user.path()).dir(system.path()).build();

    assert_eq!(index.len(), 2, "the system Firefox must not appear as well");
    let firefox = index.get("firefox.desktop").expect("firefox is indexed");
    assert_eq!(firefox.name(), "Firefox (user override)");
    assert_eq!(firefox.path().unwrap(), user.path().join("firefox.desktop"));

    let shadowed = index
        .skipped()
        .iter()
        .find(|s| matches!(&s.reason, SkipReason::Shadowed { id, .. } if id == "firefox.desktop"))
        .expect("the shadowed system entry is reported");
    assert_eq!(shadowed.path, system.path().join("firefox.desktop"));
}

/// The other half of precedence: `Hidden` means deleted, so a hidden override must not fall
/// through to the system copy.
#[test]
fn a_hidden_override_deletes_the_entry_rather_than_falling_back() {
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();

    write(
        user.path(),
        "firefox.desktop",
        "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox\nHidden=true\n",
    );
    write(
        system.path(),
        "firefox.desktop",
        &app("Firefox", "firefox %u"),
    );

    let index = builder().dir(user.path()).dir(system.path()).build();

    assert!(index.is_empty(), "indexed: {:?}", index.items());
}

#[test]
fn shadowing_is_by_id_not_by_name() {
    let user = tempfile::tempdir().unwrap();
    let system = tempfile::tempdir().unwrap();

    write(user.path(), "a.desktop", &app("Same Name", "a"));
    write(system.path(), "b.desktop", &app("Same Name", "b"));

    let index = builder().dir(user.path()).dir(system.path()).build();

    assert_eq!(index.len(), 2);
}

#[test]
fn hidden_and_nodisplay_and_wrong_desktop_entries_are_absent() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "shown.desktop", &app("Shown", "shown"));
    write(
        dir.path(),
        "hidden.desktop",
        "[Desktop Entry]\nType=Application\nName=Hidden\nExec=h\nHidden=true\n",
    );
    write(
        dir.path(),
        "nodisplay.desktop",
        "[Desktop Entry]\nType=Application\nName=NoDisplay\nExec=n\nNoDisplay=true\n",
    );
    write(
        dir.path(),
        "kde-only.desktop",
        "[Desktop Entry]\nType=Application\nName=KdeOnly\nExec=k\nOnlyShowIn=KDE;\n",
    );
    write(
        dir.path(),
        "not-gnome.desktop",
        "[Desktop Entry]\nType=Application\nName=NotGnome\nExec=g\nNotShowIn=GNOME;\n",
    );

    let index = builder().dir(dir.path()).build();

    let names: Vec<&str> = index.items().iter().map(|i| i.name()).collect();
    assert_eq!(names, ["Shown"]);
    assert_eq!(
        index
            .skipped()
            .iter()
            .filter(|s| s.reason == SkipReason::NotShown)
            .count(),
        4
    );
}

#[test]
fn non_application_types_are_not_indexed() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "link.desktop",
        "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.com\n",
    );
    write(
        dir.path(),
        "directory.desktop",
        "[Desktop Entry]\nType=Directory\nName=Games\n",
    );
    write(dir.path(), "real.desktop", &app("Real", "real"));

    let index = builder().dir(dir.path()).build();

    let names: Vec<&str> = index.items().iter().map(|i| i.name()).collect();
    assert_eq!(names, ["Real"]);
}

#[test]
fn an_application_without_exec_is_not_indexed() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "noexec.desktop",
        "[Desktop Entry]\nType=Application\nName=No Exec\n",
    );

    let index = builder().dir(dir.path()).build();

    assert!(index.is_empty());
    assert_eq!(index.skipped()[0].reason, SkipReason::NoExec);
}

#[test]
fn files_without_a_desktop_extension_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notes.txt", "hello");
    write(dir.path(), "mimeinfo.cache", "[MIME Cache]\n");
    write(dir.path(), "real.desktop", &app("Real", "real"));

    let index = builder().dir(dir.path()).build();

    assert_eq!(index.len(), 1);
    assert!(index.skipped().is_empty());
}

#[test]
fn a_missing_directory_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let index = builder()
        .dir(dir.path().join("does-not-exist"))
        .dir(dir.path())
        .build();

    assert!(index.is_empty());
    assert!(
        index.skipped().is_empty(),
        "a missing XDG_DATA_DIRS entry is normal, not a diagnostic"
    );
}

// --- TryExec ---------------------------------------------------------------------------------

#[test]
fn tryexec_that_does_not_resolve_marks_the_item_unlaunchable_but_keeps_it() {
    let apps = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    std::fs::write(bin.path().join("present"), "#!/bin/sh\n").unwrap();

    write(
        apps.path(),
        "present.desktop",
        "[Desktop Entry]\nType=Application\nName=Present\nExec=present\nTryExec=present\n",
    );
    write(
        apps.path(),
        "absent.desktop",
        "[Desktop Entry]\nType=Application\nName=Absent\nExec=absent\nTryExec=absent\n",
    );

    let index = builder()
        .dir(apps.path())
        .exec_search_path([bin.path()])
        .build();

    assert_eq!(index.len(), 2, "an unresolved TryExec must not hide an app");
    assert!(index.get("present.desktop").unwrap().launchable());
    assert!(!index.get("absent.desktop").unwrap().launchable());

    let launchable: Vec<&str> = index.launchable_items().map(|i| i.name()).collect();
    assert_eq!(launchable, ["Present"]);
}

#[test]
fn tryexec_with_a_slash_is_resolved_as_a_path() {
    let apps = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let binary = bin.path().join("thing");
    std::fs::write(&binary, "#!/bin/sh\n").unwrap();

    write(
        apps.path(),
        "abs.desktop",
        &format!(
            "[Desktop Entry]\nType=Application\nName=Abs\nExec=thing\nTryExec={}\n",
            binary.display()
        ),
    );
    write(
        apps.path(),
        "abs-missing.desktop",
        &format!(
            "[Desktop Entry]\nType=Application\nName=AbsMissing\nExec=thing\nTryExec={}\n",
            bin.path().join("nope").display()
        ),
    );

    // No exec_search_path at all: a path-shaped TryExec must not need one.
    let index = builder().dir(apps.path()).build();

    assert!(index.get("abs.desktop").unwrap().launchable());
    assert!(!index.get("abs-missing.desktop").unwrap().launchable());
}

#[test]
fn unlaunchable_entries_can_be_excluded_and_are_then_reported() {
    let apps = tempfile::tempdir().unwrap();
    write(
        apps.path(),
        "absent.desktop",
        "[Desktop Entry]\nType=Application\nName=Absent\nExec=absent\nTryExec=absent\n",
    );

    let index = builder()
        .dir(apps.path())
        .include_unlaunchable(false)
        .build();

    assert!(index.is_empty());
    assert_eq!(
        index.skipped()[0].reason,
        SkipReason::TryExecMissing("absent".to_owned())
    );
}

// --- Actions ---------------------------------------------------------------------------------

#[test]
fn desktop_actions_become_their_own_items() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "firefox.desktop",
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Firefox\n\
         Exec=firefox %u\n\
         Actions=new-window;new-private-window;\n\
         \n\
         [Desktop Action new-window]\n\
         Name=New Window\n\
         Exec=firefox --new-window\n\
         \n\
         [Desktop Action new-private-window]\n\
         Name=New Private Window\n\
         Exec=firefox --private-window\n",
    );

    let index = builder().dir(dir.path()).build();

    assert_eq!(index.len(), 3);
    let private = index
        .get("firefox.desktop::new-private-window")
        .expect("action is indexed under a composite key");
    assert!(private.is_action());
    assert_eq!(private.name(), "New Private Window");
    assert_eq!(private.app_name(), "Firefox");
    assert_eq!(
        private.display_name(),
        "Firefox \u{2192} New Private Window"
    );
    assert_eq!(private.command(), ["firefox", "--private-window"]);
    assert_eq!(index.applications().count(), 1);
}

#[test]
fn actions_are_not_indexed_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "firefox.desktop",
        "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox\nActions=x;\n\n[Desktop Action x]\nName=X\nExec=firefox --x\n",
    );

    let index = builder().dir(dir.path()).include_actions(false).build();

    assert_eq!(index.len(), 1);
}

#[test]
fn an_action_without_a_name_or_exec_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "broken.desktop",
        "[Desktop Entry]\nType=Application\nName=Broken\nExec=b\nActions=noname;noexec;ok;\n\n\
         [Desktop Action noname]\nExec=b --noname\n\n\
         [Desktop Action noexec]\nName=No Exec\n\n\
         [Desktop Action ok]\nName=Ok\nExec=b --ok\n",
    );

    let index = builder().dir(dir.path()).build();

    let keys: Vec<&str> = index.items().iter().map(|i| i.key()).collect();
    assert_eq!(keys, ["broken.desktop", "broken.desktop::ok"]);
}

// --- The corpus ------------------------------------------------------------------------------

/// The point is not what the corpus produces, it is that nothing in it makes the scanner give up.
#[test]
fn the_whole_desktop_entry_corpus_indexes_without_choking() {
    let dir = tempfile::tempdir().unwrap();
    let fixtures = corpus::desktop_entries();
    assert!(fixtures.len() >= 19, "corpus shrank: {}", fixtures.len());

    for fixture in &fixtures {
        std::fs::write(
            dir.path().join(format!("{}.desktop", fixture.id)),
            &fixture.bytes,
        )
        .expect("stage fixture");
    }

    let index = builder().dir(dir.path()).build();

    assert_eq!(
        index.skipped().len() + index.applications().count(),
        fixtures.len(),
        "every fixture is either indexed or reported as skipped, never dropped silently"
    );
    assert!(
        !index.is_empty(),
        "some corpus fixtures are real applications"
    );

    // The deliberately broken ones are reported, not fatal.
    let malformed: Vec<&str> = index
        .skipped()
        .iter()
        .filter(|s| matches!(s.reason, SkipReason::Malformed(_)))
        .filter_map(|s| s.path.file_stem().and_then(|s| s.to_str()))
        .collect();
    for id in ["empty", "malformed-no-group", "malformed-missing-name"] {
        assert!(
            malformed.contains(&id),
            "{id} should be reported malformed; got {malformed:?}"
        );
    }

    // Non-UTF-8 is read lossily rather than discarded.
    assert!(
        index.get("non-utf8.desktop").is_some(),
        "a latin-1 byte in a Comment must not lose the application"
    );

    // CRLF line endings are just line endings.
    assert!(index.get("crlf.desktop").is_some());
}

#[test]
fn the_corpus_indexes_identically_twice() {
    let dir = tempfile::tempdir().unwrap();
    for fixture in corpus::desktop_entries() {
        std::fs::write(
            dir.path().join(format!("{}.desktop", fixture.id)),
            &fixture.bytes,
        )
        .unwrap();
    }

    let first = builder().dir(dir.path()).build();
    let second = builder().dir(dir.path()).build();

    let keys = |index: &compass_core::AppIndex| -> Vec<String> {
        index.items().iter().map(|i| i.key().to_owned()).collect()
    };
    assert_eq!(keys(&first), keys(&second));
}
