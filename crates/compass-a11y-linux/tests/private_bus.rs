//! The launcher's tree, read back off a real accessibility bus the way Orca
//! reads it.
//!
//! The test runs itself again under `dbus-run-session` with a private
//! `XDG_RUNTIME_DIR`, so the session bus, the accessibility bus that
//! `at-spi-bus-launcher` starts on it and the registry are all its own and
//! nothing of the desktop's is touched. In there it turns accessibility on,
//! opens a launcher window's tree from a real `LauncherApp` that has a query
//! typed, and walks the registry with the AT-SPI client proxies: the window,
//! the search field and its text, the results and the selection. Then it
//! moves the selection, presses a result, shows a settings-like view (a grid,
//! a document, a dropdown's options), and closes the window, and checks the
//! bus after each.
//!
//! A machine without `dbus-run-session` or `at-spi-bus-launcher` cannot run
//! it, and it says so rather than failing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::action::ActionProxy;
use atspi::proxy::bus::{BusProxy, StatusProxy};
use atspi::proxy::text::TextProxy;
use atspi::{Role, State, StateSet};
use compass_a11y_linux::AtspiTree;
use compass_platform::AccessibilityTree;
use compass_ui::semantics::{FocusTracker, Target};
use compass_ui::{LauncherApp, Message};

/// Set in the run under `dbus-run-session`.
const INNER: &str = "COMPASS_A11Y_PRIVATE_BUS";
const TEST: &str = "the_launcher_is_read_off_a_private_accessibility_bus";
const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";

fn find(program: &str, dirs: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    dirs.iter()
        .map(PathBuf::from)
        .chain(std::env::split_paths(&path))
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

#[test]
fn the_launcher_is_read_off_a_private_accessibility_bus() {
    if std::env::var_os(INNER).is_some() {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(read_the_tree());
        return;
    }
    // The system's `dbus-run-session` first: its configuration activates the
    // system's `org.a11y.Bus` service, which a Homebrew or Nix one may not.
    let Some(run_session) = find("dbus-run-session", &["/usr/bin"]) else {
        eprintln!("skipped: dbus-run-session is not installed");
        return;
    };
    let launcher = ["/usr/libexec", "/usr/lib/at-spi2-core", "/usr/lib/at-spi2"]
        .iter()
        .any(|dir| Path::new(dir).join("at-spi-bus-launcher").is_file());
    if !launcher {
        eprintln!("skipped: at-spi-bus-launcher is not installed");
        return;
    }
    let runtime = tempfile::tempdir().expect("a runtime directory");
    // A runtime directory others can write is refused for D-Bus's own use.
    std::fs::set_permissions(
        runtime.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("a private runtime directory");
    let status = Command::new(run_session)
        .arg("--")
        .arg(std::env::current_exe().expect("this test's binary"))
        .args([TEST, "--exact", "--nocapture"])
        .env(INNER, "1")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .status()
        .expect("dbus-run-session runs");
    assert!(status.success(), "the run on the private bus failed");
}

/// One accessible as a screen reader sees it.
#[derive(Debug)]
struct Seen {
    role: Role,
    name: String,
    description: String,
    states: StateSet,
    bus_name: String,
    path: String,
}

impl Seen {
    fn has(&self, state: State) -> bool {
        self.states.contains(state)
    }
}

/// Every accessible under the application this test process is, depth
/// first; empty until the application is on the bus.
async fn read_app(bus: &zbus::Connection) -> Vec<Seen> {
    let app_name = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let Ok(registry) = AccessibleProxy::builder(bus)
        .destination(REGISTRY)
        .and_then(|b| b.path(ROOT))
        .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
    else {
        return Vec::new();
    };
    let Ok(registry) = registry.build().await else {
        return Vec::new();
    };
    let Ok(apps) = registry.get_children().await else {
        return Vec::new();
    };
    for app in apps {
        let Ok(proxy) = app.as_accessible_proxy(bus).await else {
            continue;
        };
        if proxy.name().await.ok().as_deref() != Some(app_name.as_str()) {
            continue;
        }
        let mut seen = Vec::new();
        let mut pending = proxy.get_children().await.unwrap_or_default();
        pending.reverse();
        while let Some(object) = pending.pop() {
            let Ok(proxy) = object.as_accessible_proxy(bus).await else {
                continue;
            };
            let (Ok(role), Ok(name), Ok(states)) = (
                proxy.get_role().await,
                proxy.name().await,
                proxy.get_state().await,
            ) else {
                continue;
            };
            seen.push(Seen {
                role,
                name,
                description: proxy.description().await.unwrap_or_default(),
                states,
                bus_name: object.name_as_str().unwrap_or_default().to_owned(),
                path: object.path_as_str().to_owned(),
            });
            let mut children = proxy.get_children().await.unwrap_or_default();
            children.reverse();
            pending.extend(children);
        }
        return seen;
    }
    Vec::new()
}

/// Reads the application until `ready` holds of what is there, for up to
/// ten seconds: the adapter exports on its own thread.
async fn eventually(
    bus: &zbus::Connection,
    what: &str,
    ready: impl Fn(&[Seen]) -> bool,
) -> Vec<Seen> {
    for _ in 0..200 {
        let seen = read_app(bus).await;
        if ready(&seen) {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "{what}: never happened; last read {:#?}",
        read_app(bus).await
    );
}

fn one<'a>(seen: &'a [Seen], role: Role, name: &str) -> &'a Seen {
    seen.iter()
        .find(|s| s.role == role && s.name == name)
        .unwrap_or_else(|| panic!("no {role:?} called {name:?} in {seen:#?}"))
}

fn index(dir: &Path) -> compass_core::AppIndex {
    for (file, name, comment) in [
        ("firefox.desktop", "Firefox", "Web Browser"),
        ("files.desktop", "Files", "Access and organize files"),
    ] {
        std::fs::write(
            dir.join(file),
            format!("[Desktop Entry]\nType=Application\nName={name}\nComment={comment}\nExec=/bin/true\n"),
        )
        .expect("write entry");
    }
    compass_core::AppIndex::builder().dir(dir).build()
}

async fn read_the_tree() {
    let session = zbus::Connection::session()
        .await
        .expect("the private session bus");
    StatusProxy::new(&session)
        .await
        .expect("org.a11y.Bus")
        .set_is_enabled(true)
        .await
        .expect("accessibility turned on");
    let address = BusProxy::new(&session)
        .await
        .expect("org.a11y.Bus")
        .get_address()
        .await
        .expect("the accessibility bus's address");
    let a11y = zbus::connection::Builder::address(address.as_str())
        .expect("an address")
        .build()
        .await
        .expect("the accessibility bus");

    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = LauncherApp::with_index(index(dir.path()));
    let _ = app.update(Message::QueryChanged("fi".into()));
    let mut focus = FocusTracker::new();
    let semantics = app.semantics();
    let (requests, received) = mpsc::channel();
    let mut tree = AtspiTree::new();
    tree.open(
        semantics.tree_update(focus.focus(&semantics)),
        Box::new(move |request| {
            let _ = requests.send(request);
        }),
    );
    tree.set_focused(true);

    // Opened: the window, its search field with the query, its results.
    let seen = eventually(&a11y, "the window appearing", |seen| {
        seen.iter().any(|s| s.role == Role::ListItem)
    })
    .await;
    let window = one(&seen, Role::Frame, "Compass");
    assert!(window.has(State::Active), "{window:?}");
    let field = one(&seen, Role::Entry, "Search…");
    assert!(
        field.has(State::Editable) && field.has(State::Focused),
        "{field:?}"
    );
    let text = TextProxy::builder(&a11y)
        .destination(field.bus_name.as_str())
        .and_then(|b| b.path(field.path.as_str()))
        .expect("a proxy")
        .build()
        .await
        .expect("the field's text");
    assert_eq!(text.get_text(0, -1).await.expect("its text"), "fi");
    one(&seen, Role::ListBox, "Results");
    let firefox = one(&seen, Role::ListItem, "Firefox");
    assert_eq!(firefox.description, "Web Browser");
    let items: Vec<&Seen> = seen.iter().filter(|s| s.role == Role::ListItem).collect();
    // Builtin commands match too; the list on the bus is the launcher's list.
    let expected = semantics_item_name(&semantics, 0);
    assert_eq!(items[0].name, expected);
    assert!(items.len() >= 2, "{items:#?}");
    assert!(items[0].has(State::Selected) && !items[1].has(State::Selected));
    one(
        &seen,
        Role::StatusBar,
        &compass_ui::semantics::result_count(items.len()),
    );

    // Moving the selection moves the focus to the result and announces it.
    let _ = app.update(Message::MoveSelection(compass_ui::Direction::Down));
    let semantics = app.semantics();
    tree.update(semantics.tree_update(focus.focus(&semantics)));
    let second = semantics_item_name(&semantics, 1);
    let seen = eventually(&a11y, "the selection moving", |seen| {
        seen.iter()
            .any(|s| s.role == Role::ListItem && s.name == second && s.has(State::Focused))
    })
    .await;
    let item = one(&seen, Role::ListItem, &second);
    assert!(item.has(State::Selected), "{item:?}");

    // Pressing a result reaches the launcher as a click on it.
    let action = ActionProxy::builder(&a11y)
        .destination(item.bus_name.as_str())
        .and_then(|b| b.path(item.path.as_str()))
        .expect("a proxy")
        .build()
        .await
        .expect("the result's actions");
    assert!(action.do_action(0).await.expect("pressed"));
    let request = received
        .recv_timeout(Duration::from_secs(5))
        .expect("the press reached the launcher");
    assert_eq!(request.action, accesskit::Action::Click);
    assert_eq!(semantics.target(request.target_node), Some(Target::Item(1)));

    // A grid, a document beside it and a form's dropdown reach the bus as
    // the table, document and combo box a screen reader knows.
    tree.update(builtin_view().tree_update(accesskit::NodeId(1)));
    let seen = eventually(&a11y, "the builtin view appearing", |seen| {
        seen.iter().any(|s| s.role == Role::TableCell)
    })
    .await;
    one(&seen, Role::Table, "Fonts");
    assert!(one(&seen, Role::TableCell, "Cantarell").has(State::Selected));
    one(&seen, Role::DocumentFrame, "Readme");
    one(&seen, Role::Heading, "Usage");
    one(&seen, Role::ComboBox, "Theme");
    assert!(one(&seen, Role::MenuItem, "Dark").has(State::Selected));
    one(&seen, Role::ToggleButton, "Enabled");

    // Closed: nothing of the window is left.
    tree.close();
    eventually(&a11y, "the window going away", |seen| {
        !seen.iter().any(|s| s.role == Role::Frame)
    })
    .await;
}

fn semantics_item_name(semantics: &compass_ui::semantics::Semantics, position: usize) -> String {
    match &semantics.body {
        compass_ui::semantics::Body::List(list) => list.items[position].name.clone(),
        other => panic!("expected a list, got {other:?}"),
    }
}

/// A view of each new kind of node: a grid of fonts beside a document, over
/// a form with a dropdown and a switch.
fn builtin_view() -> compass_ui::semantics::Semantics {
    use compass_ui::semantics::{
        Body, Document, Field, FieldKind, FieldTarget, Form, Item, List, Semantics, markdown_blocks,
    };
    let field = |label: &str, kind| Field {
        label: label.to_owned(),
        kind,
        description: None,
        required: false,
        invalid: false,
        target: FieldTarget::Control(0),
    };
    let list = List {
        label: "Fonts".to_owned(),
        items: ["Cantarell", "Inter", "Noto Sans"]
            .into_iter()
            .map(|name| Item {
                name: name.to_owned(),
                description: None,
            })
            .collect(),
        selected: Some(0),
        grid: Some(List::grid_of(3, 2)),
    };
    let form = Form {
        label: "Appearance".to_owned(),
        fields: vec![
            field(
                "Theme",
                FieldKind::Dropdown {
                    value: Some("Dark".to_owned()),
                    options: vec!["Light".to_owned(), "Dark".to_owned()],
                    expanded: Some(false),
                    highlighted: None,
                },
            ),
            field("Enabled", FieldKind::Switch { on: true }),
        ],
        focused: None,
    };
    Semantics {
        window: "Compass".to_owned(),
        body: Body::ListAndForm(list, form),
        detail: Some(Document {
            label: "Readme".to_owned(),
            blocks: markdown_blocks("# Usage\n\nPick a font."),
            metadata: Vec::new(),
        }),
        ..Semantics::default()
    }
}
