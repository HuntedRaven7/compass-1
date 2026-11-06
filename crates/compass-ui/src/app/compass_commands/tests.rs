use super::*;
use crate::app::RootRow;
use crate::app::test_support::{
    TestBackend, choose, enable_commands, extension_app, index, open_builtin, panel_titles,
    pressed, settle, with_resident_hud,
};
use crate::resident::UiCommand;
use compass_core::manifest::{ExtensionManifest, Provenance};
use std::fs;
use std::sync::Arc;

#[test]
fn the_default_config_is_written_read_only_and_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_default_config_in(dir.path()).unwrap();
    assert!(std::fs::metadata(&path).unwrap().permissions().readonly());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
    assert_eq!(write_default_config_in(dir.path()).unwrap(), path, "again");
}

#[test]
fn the_report_names_this_build() {
    let info = system_info();
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert!(info.build_info.starts_with("rustc - "));
}

fn app() -> LauncherApp {
    LauncherApp::with_index(compass_core::AppIndex::builder().build())
}

fn list(semantics: &PageSemantics) -> &crate::semantics::List {
    match &semantics.body {
        Body::List(list) => list,
        other => panic!("expected a list, got {other:?}"),
    }
}

fn manifest(id: &str, title: &str, description: &str) -> ExtensionManifest {
    ExtensionManifest {
        path: std::path::PathBuf::from("/extensions").join(id),
        id: id.into(),
        name: id.into(),
        title: title.into(),
        description: description.into(),
        icon: String::new(),
        author: "someone".into(),
        owner: None,
        categories: Vec::new(),
        preferences: Vec::new(),
        commands: Vec::new(),
        needs_raycast_api: false,
        provenance: Provenance::from_id(id),
    }
}

#[test]
fn the_fallback_manager_describes_every_row_in_its_order() {
    let candidate = |id: &str, title: &str, subtitle: &str, enabled: bool| Candidate {
        id: id.into(),
        title: title.into(),
        subtitle: subtitle.into(),
        keywords: Vec::new(),
        icon: None,
        enabled_as: enabled.then(|| id.to_owned()),
    };
    let app = app();
    let mut page = FallbacksPage::new(
        vec![
            candidate("shortcuts:google", "Google", "Quicklink", false),
            candidate("files:search", "Search Files", "Command", true),
        ],
        vec!["files:search".into()],
    );
    page.selected = 1;
    let semantics = app.fallbacks_semantics(&page);
    let rows = list(&semantics);
    assert_eq!(
        rows.items,
        [
            item("Search Files", Some("Command".into())),
            item("Google", Some("Quicklink".into())),
        ],
        "enabled first, as drawn, without the section headings"
    );
    assert_eq!(rows.selected, Some(1));
    assert_eq!(semantics.status, None);

    page.query = "zzzz".into();
    page.refilter();
    let semantics = app.fallbacks_semantics(&page);
    assert_eq!(semantics.body, Body::Empty);
    assert_eq!(semantics.status.as_deref(), Some("No commands match"));
    let nothing = FallbacksPage::new(Vec::new(), Vec::new());
    assert_eq!(
        app.fallbacks_semantics(&nothing).status.as_deref(),
        Some("Nothing can be a fallback")
    );
}

#[test]
fn installed_extensions_are_described_with_where_they_came_from() {
    let app = app();
    let none = ExtensionsPage::new(Vec::new());
    assert_eq!(
        app.extensions_semantics(&none).status.as_deref(),
        Some("No extensions are installed")
    );
    let mut page = ExtensionsPage::new(vec![
        manifest("store.raycast.github", "GitHub", "Work with issues"),
        manifest("notes", "Notes", ""),
    ]);
    page.notice = Some(crate::store_page::UNINSTALLED.to_owned());
    let semantics = app.extensions_semantics(&page);
    assert_eq!(
        list(&semantics).items,
        [
            item("GitHub", Some("Work with issues · Raycast".into())),
            item("Notes", Some("Local".into())),
        ]
    );
    assert_eq!(list(&semantics).selected, Some(0));
    assert_eq!(
        semantics.status.as_deref(),
        Some(crate::store_page::UNINSTALLED)
    );
    page.query = "zzzz".into();
    page.refilter();
    assert_eq!(
        app.extensions_semantics(&page).status.as_deref(),
        Some("No extensions match")
    );
}

#[test]
fn builtin_icons_are_described_by_name_and_counted_when_searched() {
    let app = app();
    let mut page = IconsPage::new();
    let names = compass_core::builtin_icon::names();
    let semantics = app.icons_semantics(&page);
    assert_eq!(list(&semantics).items.len(), names.len());
    assert_eq!(list(&semantics).items[0], item(names[page.shown[0]], None));
    assert_eq!(semantics.status, None, "nothing typed, nothing counted");

    page.query = "copy-clipboard".into();
    page.refilter();
    let semantics = app.icons_semantics(&page);
    assert_eq!(list(&semantics).items[0].name, "copy-clipboard");
    assert_eq!(
        semantics.status,
        Some(crate::semantics::result_count(list(&semantics).items.len()))
    );
    page.query = "zzzzqqqq".into();
    page.refilter();
    assert_eq!(
        app.icons_semantics(&page).status.as_deref(),
        Some("No icons match")
    );
}

#[test]
fn local_storage_describes_namespaces_then_the_keys_of_one() {
    use crate::backend::StorageItemRow;
    let app = app();
    let mut page = StoragePage::default();
    assert_eq!(
        app.storage_semantics(&page).status.as_deref(),
        Some("No namespaces")
    );
    page.notice = Some("The engine is not running".into());
    assert_eq!(
        app.storage_semantics(&page).status.as_deref(),
        Some("The engine is not running"),
        "why nothing could be read, in place of the list"
    );
    page.notice = None;
    page.set_namespaces(vec!["@a/notes".into(), "core".into()]);
    let semantics = app.storage_semantics(&page);
    assert_eq!(list(&semantics).label, "Namespaces");
    assert_eq!(
        list(&semantics).items,
        [item("@a/notes", None), item("core", None)]
    );
    page.browse(
        "@a/notes".into(),
        vec![StorageItemRow {
            key: "draft".into(),
            value: "hello".into(),
        }],
    );
    page.notice = Some("hello".into());
    let semantics = app.storage_semantics(&page);
    assert_eq!(list(&semantics).label, "Items");
    assert_eq!(list(&semantics).items, [item("draft", None)]);
    assert_eq!(
        semantics.status.as_deref(),
        Some("hello"),
        "the value shown"
    );
}

#[test]
fn token_sets_are_described_with_their_provider_and_expiry() {
    use crate::backend::TokenSetRow;
    let mut app = app();
    let mut page = TokensPage::default();
    assert_eq!(
        app.tokens_semantics(&page).status.as_deref(),
        Some("No token sets")
    );
    page.set_sets(vec![
        TokenSetRow {
            extension_id: "github".into(),
            provider_id: Some("GitHub".into()),
            expired: true,
            ..TokenSetRow::default()
        },
        TokenSetRow {
            extension_id: "linear".into(),
            ..TokenSetRow::default()
        },
    ]);
    page.selected = 1;
    app.page = Page::Tokens(page);
    let semantics = app.semantics();
    let Body::List(rows) = &semantics.body else {
        panic!("expected a list, got {:?}", semantics.body);
    };
    assert_eq!(
        rows.items,
        [
            item("github", Some("GitHub · Expired".into())),
            item("linear", None),
        ]
    );
    assert_eq!(rows.selected, Some(1));
}

#[test]
fn a_store_intro_is_read_as_its_text_then_its_keys() {
    let mut app = app();
    let page = crate::compass_pages::StoreIntroPage::new(crate::backend::Store::Raycast);
    let semantics = app.store_intro_semantics(&page);
    let Body::Document(document) = &semantics.body else {
        panic!("expected a document, got {:?}", semantics.body);
    };
    assert_eq!(document.label, "Raycast Store");
    assert_eq!(
        document.blocks.first(),
        Some(&Block {
            text: "Welcome to the Raycast Extension Store".to_owned(),
            heading: Some(1),
        })
    );
    assert_eq!(
        document.blocks.last().map(|block| block.text.clone()),
        Some(crate::compass_pages::intro_hint())
    );
    app.page = Page::StoreIntro(page);
    let semantics = app.semantics();
    assert!(semantics.search.is_none());
    assert!(matches!(semantics.body, Body::Document(_)));
}

#[test]
fn configure_fallback_commands_moves_items_between_its_sections() {
    use compass_core::root_items::RootEdit;
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend::default());
    let mut app = extension_app(dir.path(), backend.clone());
    app.fallbacks = vec!["files:search".into()];
    open_builtin(&mut app, "configure fallback", "commands:manage-fallback");
    let Page::Fallbacks(page) = &app.page else {
        panic!("not the fallback manager: {}", app.state_line());
    };
    let titles: Vec<(&str, Option<&str>)> = page
        .rows
        .iter()
        .enumerate()
        .map(|(at, row)| {
            (
                page.candidates[row.candidate].title.as_str(),
                page.heading_at(at),
            )
        })
        .collect();
    assert_eq!(
        titles,
        [
            ("Search Files", Some("Enabled")),
            ("Write Greeting", Some("Available"))
        ]
    );

    // Enter on the available command enables it, first.
    let _ = app.update(Message::FallbacksQueryChanged("greet".into()));
    let task = app.update(pressed(iced::keyboard::key::Named::Enter));
    settle(&mut app, task);
    assert_eq!(
        app.fallbacks,
        ["@someone/hello:write".to_owned(), "files:search".to_owned()]
    );
    assert_eq!(
        backend.root_edits.lock().unwrap().last(),
        Some(&("@someone/hello:write".to_owned(), RootEdit::Fallback(true)))
    );

    // The panel's action disables Search Files by the id it is stored as.
    let _ = app.update(Message::FallbacksQueryChanged("search files".into()));
    let _ = app.update(Message::TogglePanel);
    let task = choose(&mut app, "Disable fallback");
    settle(&mut app, task);
    assert_eq!(app.fallbacks, ["@someone/hello:write".to_owned()]);
    assert_eq!(
        backend.root_edits.lock().unwrap().last(),
        Some(&("files:search".to_owned(), RootEdit::Fallback(false)))
    );

    // A root fallback row's panel opens the manager.
    let _ = app.update(Message::Back);
    app.query = "zzzz".into();
    app.search();
    app.selected = app
        .results
        .iter()
        .position(|row| matches!(row, RootRow::Fallback(_)))
        .expect("the extension is a fallback now");
    let _ = app.update(Message::TogglePanel);
    assert_eq!(
        panel_titles(&app),
        ["Open command", "Manage fallback actions"]
    );
    let task = choose(&mut app, "Manage fallback actions");
    settle(&mut app, task);
    assert!(matches!(app.page, Page::Fallbacks(_)));
}

#[test]
fn installed_extensions_are_listed_copied_and_uninstalled_after_asking() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend::default());
    let mut app = with_resident_hud(extension_app(dir.path(), backend.clone()));
    open_builtin(&mut app, "installed extensions", "commands:list-extensions");
    let Page::Extensions(page) = &app.page else {
        panic!("not the installed extensions: {}", app.state_line());
    };
    assert_eq!(page.all.len(), 1);
    assert_eq!(page.all[0].title, "Hello");
    let _ = app.update(Message::TogglePanel);
    assert_eq!(
        panel_titles(&app),
        [
            "Uninstall",
            "Copy name",
            "Copy ID",
            "Copy path",
            "Copy author"
        ]
    );
    let task = choose(&mut app, "Copy author");
    assert_eq!(settle(&mut app, task), ["someone"]);
    assert_eq!(app.hud_content(), Some(&crate::hud::Hud::copied()));

    let _ = app.update(Message::Command(UiCommand::Show));
    open_builtin(&mut app, "installed extensions", "commands:list-extensions");
    let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
    assert!(app.confirm.is_some(), "asks first");
    let task = app.update(pressed(iced::keyboard::key::Named::Enter));
    settle(&mut app, task);
    let Page::Extensions(page) = &app.page else {
        panic!("left the view: {}", app.state_line());
    };
    assert!(page.all.is_empty(), "gone from the list");
    assert_eq!(page.notice.as_deref(), Some("Extension uninstalled"));
}

#[test]
fn search_builtin_icons_copies_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
    let config = compass_core::Config::parse(
        r#"{"providers":{"commands":{"entrypoints":{"search-builtin-icons":{"enabled":true}}}}}"#,
        std::path::Path::new("config.json"),
    )
    .unwrap();
    app.root_config = config.root_config();
    app.app_index.apply_root_config(&app.root_config);
    open_builtin(&mut app, "builtin icons", "commands:search-builtin-icons");
    let _ = app.update(Message::IconsQueryChanged("copy-clipboard".into()));
    let task = app.update(pressed(iced::keyboard::key::Named::Enter));
    assert_eq!(settle(&mut app, task), ["copy-clipboard"]);
    assert_eq!(app.hud_content(), Some(&crate::hud::Hud::copied()));
}

#[test]
fn inspect_local_storage_browses_a_namespace_and_shows_a_value() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        storage: vec![
            (
                "@a/notes".to_owned(),
                vec![crate::backend::StorageItemRow {
                    key: "draft".into(),
                    value: "hello".into(),
                }],
            ),
            ("core".to_owned(), Vec::new()),
        ],
        ..TestBackend::default()
    });
    let mut app = LauncherApp::with_index(index(dir.path()));
    app.backend = Some(backend);
    enable_commands(&mut app, &["inspect-local-storage"]);
    open_builtin(
        &mut app,
        "inspect local storage",
        "commands:inspect-local-storage",
    );
    let Page::Storage(page) = &app.page else {
        panic!("not local storage: {}", app.state_line());
    };
    assert_eq!(page.titles(), ["@a/notes", "core"]);
    let _ = app.update(Message::TogglePanel);
    assert_eq!(panel_titles(&app), ["Browse namespace"]);
    let task = choose(&mut app, "Browse namespace");
    settle(&mut app, task);
    let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
    let Page::Storage(page) = &app.page else {
        panic!("left local storage");
    };
    assert_eq!(page.titles(), ["draft"]);
    assert_eq!(page.notice.as_deref(), Some("hello"), "Show value");
    let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
    let Page::Storage(page) = &app.page else {
        panic!("Escape left the view instead of the namespace");
    };
    assert!(page.browsing.is_none());
}

#[test]
fn manage_oauth_token_sets_copies_and_removes_after_asking() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        token_sets: std::sync::Mutex::new(vec![crate::backend::TokenSetRow {
            extension_id: "github".into(),
            provider_id: Some("GitHub".into()),
            access_token: "gho_token".into(),
            scope: Some("repo".into()),
            expires_at: Some(0),
            expired: true,
            ..crate::backend::TokenSetRow::default()
        }]),
        ..TestBackend::default()
    });
    let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
    app.backend = Some(backend.clone());
    enable_commands(&mut app, &["oauth-token-store"]);
    open_builtin(
        &mut app,
        "manage oauth token sets",
        "commands:oauth-token-store",
    );
    let _ = app.update(Message::TogglePanel);
    assert_eq!(
        panel_titles(&app),
        [
            "Remove token set",
            "Copy access token",
            "Copy scopes",
            "Copy expiration date"
        ]
    );
    let task = choose(&mut app, "Copy access token");
    assert_eq!(settle(&mut app, task), ["gho_token"]);

    let _ = app.update(Message::Command(UiCommand::Show));
    open_builtin(
        &mut app,
        "manage oauth token sets",
        "commands:oauth-token-store",
    );
    let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
    assert!(app.confirm.is_some(), "asks first");
    let task = app.update(pressed(iced::keyboard::key::Named::Enter));
    settle(&mut app, task);
    assert!(backend.token_sets.lock().unwrap().is_empty());
    let Page::Tokens(page) = &app.page else {
        panic!("left the view");
    };
    assert!(page.sets.is_empty(), "listed again");
    assert_eq!(page.notice.as_deref(), Some("Token set removed"));
}

#[test]
fn the_link_and_refresh_commands_do_what_their_cpp_ones_do() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend::default());
    let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
    app.backend = Some(backend.clone());
    open_builtin(&mut app, "donate", "commands:sponsor");
    assert_eq!(
        backend.opened_urls.lock().unwrap().as_slice(),
        [compass_core::commands::SPONSOR_URL]
    );
    assert_eq!(
        app.hud_content().map(|hud| hud.text.as_str()),
        Some("Opened in browser")
    );

    let _ = app.update(Message::Command(UiCommand::Show));
    open_builtin(&mut app, "report a compass bug", "commands:report-bug");
    let reported = backend.opened_urls.lock().unwrap().last().cloned().unwrap();
    assert!(
        reported.starts_with(compass_core::bug_report::CREATE_ISSUE_URL),
        "{reported}"
    );
    assert!(reported.contains("type=bug"));

    fs::write(
        dir.path().join("zephyr.desktop"),
        "[Desktop Entry]\nType=Application\nName=Zephyr\nExec=/bin/true\n",
    )
    .unwrap();
    let _ = app.update(Message::Command(UiCommand::Show));
    open_builtin(&mut app, "refresh apps", "commands:refresh-apps");
    assert!(app.app_index.get("zephyr.desktop").is_some(), "rescanned");
    assert_eq!(app.error.as_deref(), Some("Apps successfully refreshed"));
}
