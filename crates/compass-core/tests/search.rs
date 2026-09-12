//! Search quality over a realistic index.

mod support;

use compass_core::{AppIndex, AppItem};
use support::{builder, write};

/// A small but realistic slice of a desktop: overlapping prefixes, actions, keywords, and
/// entries that must never show up.
fn realistic_index(dir: &std::path::Path) -> AppIndex {
    write(
        dir,
        "org.mozilla.firefox.desktop",
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Firefox\n\
         GenericName=Web Browser\n\
         Comment=Browse the World Wide Web\n\
         Keywords=Internet;WWW;Browser;Web;\n\
         Categories=Network;WebBrowser;\n\
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
    write(
        dir,
        "org.gnome.Nautilus.desktop",
        "[Desktop Entry]\nType=Application\nName=Files\nGenericName=File Manager\nKeywords=Folder;Manager;Explore;\nCategories=GNOME;System;FileTools;\nExec=nautilus --new-window %U\n",
    );
    write(
        dir,
        "org.gnome.Terminal.desktop",
        "[Desktop Entry]\nType=Application\nName=Terminal\nGenericName=Terminal\nKeywords=shell;prompt;command;\nCategories=GNOME;System;TerminalEmulator;\nExec=gnome-terminal\n",
    );
    write(
        dir,
        "gimp.desktop",
        "[Desktop Entry]\nType=Application\nName=GNU Image Manipulation Program\nGenericName=Image Editor\nKeywords=GIMP;graphic;photo;\nCategories=Graphics;\nExec=gimp-2.10 %U\n",
    );
    write(
        dir,
        "firewall-config.desktop",
        "[Desktop Entry]\nType=Application\nName=Firewall\nGenericName=Firewall Configuration\nCategories=System;\nExec=firewall-config\n",
    );
    write(
        dir,
        "secret.desktop",
        "[Desktop Entry]\nType=Application\nName=Firefox Crash Reporter\nExec=crashreporter\nNoDisplay=true\n",
    );
    write(
        dir,
        "kde-only.desktop",
        "[Desktop Entry]\nType=Application\nName=Firefox KDE Helper\nExec=fkde\nOnlyShowIn=KDE;\n",
    );

    builder().dir(dir).build()
}

fn names(hits: &[compass_search::Scored<&AppItem>]) -> Vec<String> {
    hits.iter().map(|h| h.item.display_name()).collect()
}

#[test]
fn a_prefix_finds_the_application_first() {
    let dir = tempfile::tempdir().unwrap();
    let index = realistic_index(dir.path());

    let hits = index.search("fir");
    let found = names(&hits);

    // "fir" is a genuine tie on match quality: Firefox and Firewall are both exact prefix
    // matches, and nothing in the *text* can separate them. Both must be found, the application
    // must beat its own actions, and breaking the tie is frecency's job — see
    // `frecency::launch_history_breaks_a_tie_between_equally_good_matches`.
    assert!(found.contains(&"Firefox".to_owned()), "ranked: {found:?}");
    assert!(found.contains(&"Firewall".to_owned()), "ranked: {found:?}");
    assert!(
        found[..2].iter().all(|n| n == "Firefox" || n == "Firewall"),
        "the two prefix matches must be the top two: {found:?}"
    );

    // One more character and it is no longer a tie.
    let found = names(&index.search("firef"));
    assert_eq!(
        found.first().map(String::as_str),
        Some("Firefox"),
        "ranked: {found:?}"
    );
}

#[test]
fn the_application_outranks_its_own_actions() {
    let dir = tempfile::tempdir().unwrap();
    let index = realistic_index(dir.path());

    let found = names(&index.search("firefox"));
    let app_at = found.iter().position(|n| n == "Firefox").expect("firefox");
    let action_at = found
        .iter()
        .position(|n| n.starts_with("Firefox \u{2192}"))
        .expect("an action matched too");

    assert!(
        app_at < action_at,
        "the app must come before its actions: {found:?}"
    );
}

#[test]
fn actions_are_findable_by_their_own_name() {
    let dir = tempfile::tempdir().unwrap();
    let index = realistic_index(dir.path());

    let found = names(&index.search("private window"));
    assert_eq!(
        found.first().map(String::as_str),
        Some("Firefox \u{2192} New Private Window"),
        "ranked: {found:?}"
    );
}

#[test]
fn an_action_is_findable_by_application_and_action_together() {
    let dir = tempfile::tempdir().unwrap();
    let index = realistic_index(dir.path());

    let found = names(&index.search("firefox private"));
    assert_eq!(
        found,
        ["Firefox \u{2192} New Private Window"],
        "both words must match, across the action name and its app name"
    );
}

#[test]
fn hidden_and_foreign_desktop_entries_never_appear_in_results() {
    let dir = tempfile::tempdir().unwrap();
    let index = realistic_index(dir.path());

    for query in ["fir", "firefox", "crash", "helper"] {
        let found = names(&index.search(query));
        assert!(
            !found.iter().any(|n| n.contains("Crash Reporter")),
            "NoDisplay entry surfaced for {query:?}: {found:?}"
        );
        assert!(
            !found.iter().any(|n| n.contains("KDE Helper")),
            "OnlyShowIn=KDE entry surfaced for {query:?}: {found:?}"
        );
    }
}

#[test]
fn a_generic_name_finds_the_application() {
    let dir = tempfile::tempdir().unwrap();
    let index = realistic_index(dir.path());

    let found = names(&index.search("web browser"));
    assert!(found.contains(&"Firefox".to_owned()), "ranked: {found:?}");
}

#[test]
fn a_keyword_finds_the_application() {
    let dir = tempfile::tempdir().unwrap();
    let index = realistic_index(dir.path());

    let found = names(&index.search("photo"));
    assert!(
        found.contains(&"GNU Image Manipulation Program".to_owned()),
        "ranked: {found:?}"
    );
}

/// The weighting exists so a name match beats a weaker-field match on another item.
#[test]
fn a_name_match_outranks_a_keyword_match_on_another_item() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "shell.desktop",
        "[Desktop Entry]\nType=Application\nName=Shell\nExec=shell\n",
    );
    write(
        dir.path(),
        "terminal.desktop",
        "[Desktop Entry]\nType=Application\nName=Terminal\nKeywords=shell;\nExec=term\n",
    );

    let index = builder().dir(dir.path()).build();
    let found = names(&index.search("shell"));

    assert_eq!(
        found.first().map(String::as_str),
        Some("Shell"),
        "{found:?}"
    );
    assert!(found.contains(&"Terminal".to_owned()), "{found:?}");
}

#[test]
fn a_name_match_outranks_a_category_match() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "netmon.desktop",
        "[Desktop Entry]\nType=Application\nName=Network\nExec=netmon\n",
    );
    write(
        dir.path(),
        "browser.desktop",
        "[Desktop Entry]\nType=Application\nName=Browser\nCategories=Network;\nExec=browser\n",
    );

    let index = builder().dir(dir.path()).build();
    let found = names(&index.search("network"));

    assert_eq!(
        found.first().map(String::as_str),
        Some("Network"),
        "{found:?}"
    );
}

#[test]
fn an_empty_query_keeps_everything_in_index_order() {
    let dir = tempfile::tempdir().unwrap();
    let index = realistic_index(dir.path());

    let hits = index.search("");
    assert_eq!(hits.len(), index.len());
    assert!(hits.iter().enumerate().all(|(i, h)| h.index == i));
}

#[test]
fn nonsense_matches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let index = realistic_index(dir.path());

    assert!(index.search("qzxwvjkq").is_empty());
}
