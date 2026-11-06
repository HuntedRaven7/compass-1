//! Small semantic `insta` snapshots of what `view()` actually lays out.
//!
//! # What this replaces
//!
//! The first version of this file claimed, in its own doc comment, to render
//! "via `iced_test::Simulator`". It did not. It built a string from a
//! hand-written view description and snapshotted that, and its control
//! compared two different hand-written strings — which proves `format!` is
//! injective, not that any widget exists. Deleting the whole of `view()`
//! would have left all five tests green.
//!
//! # Structure, not pixels, and not strings
//!
//! `Simulator::snapshot` compares a committed PNG, which makes the assertion
//! depend on the fonts the runner happens to ship; `app/view.rs`'s view tests
//! already reject that for the same reason. So the snapshot here is the **layout order** of
//! the labels the view puts on screen, read back out of the real Iced tree by
//! selecting each one and taking its bounds. A row that disappears, a section
//! that moves above another, or a branch of `view()` that stops rendering all
//! change it. A font package that does not, does not.
//!
//! # Five surfaces
//!
//! PLAN.md §8.5 names four: results list, empty state, detail view, form.
//! The action panel is the fifth. Detail and form are an extension command's
//! views, reached the way the engine reaches them: `ExtensionCommandStarted`
//! opens the session and `ExtensionViewLoaded` hands `update` the view, so
//! `view()` draws them through the same branches a running extension does.
//! Iced's `rich_text` reports no text to a find, so the detail's Markdown is
//! visible here only because the Markdown viewer labels its headings and
//! paragraphs (`src/labelled.rs`); drop that and the detail snapshot fails.

use compass_core::apps::AppIndex;
use compass_extension_api::action::{Action, ActionPanel};
use compass_extension_api::id::NodeId;
use compass_extension_api::view::{Detail, FieldKind, FieldValue, FormField, FormItem, FormView};
use compass_ui::LauncherApp;
use compass_ui::backend::{ExtensionStart, ExtensionViewState};
use compass_ui::message::Message;
use insta::assert_snapshot;

/// Labels probed in every snapshot, so an unexpected appearance is as visible
/// as a disappearance.
const PROBES: &[&str] = &[
    "Search…",
    "Application 00",
    "Application 01",
    "Application 02",
    "No results",
    "Open",
    "Copy name",
    "COPY",
    "Release notes",
    "Now with paste.",
    "Title",
    "Crash on paste",
    "Shown in the issue list",
    "Urgent",
    "Page someone",
    "Enter: Create Issue    Esc: back",
];

fn app_with(dir: &std::path::Path, count: usize) -> LauncherApp {
    for i in 0..count {
        std::fs::write(
            dir.join(format!("app-{i:02}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=Application {i:02}\nExec=/bin/true\n"),
        )
        .expect("write a desktop entry");
    }
    LauncherApp::with_index(AppIndex::builder().dir(dir).build())
}

/// Opens an extension command's view as the engine would, and hands `update`
/// `view` as its first rendered state.
fn extension_showing(dir: &std::path::Path, view: compass_extension_api::View) -> LauncherApp {
    const SESSION: u64 = 1;
    let mut app = app_with(dir, 3);
    let _ = app.update(Message::ExtensionCommandStarted {
        id: "snapshots.command".to_owned(),
        title: "Snapshots".to_owned(),
        result: Ok(ExtensionStart::View(SESSION)),
    });
    let _ = app.update(Message::ExtensionViewLoaded {
        session: SESSION,
        result: Ok(ExtensionViewState {
            version: 1,
            view: Some(Box::new(view)),
            depth: 1,
            ..ExtensionViewState::default()
        }),
    });
    app
}

fn detail() -> compass_extension_api::View {
    compass_extension_api::View::Detail(Detail {
        markdown: Some("# Release notes\n\nNow with **paste**.".to_owned()),
        ..Detail::default()
    })
}

fn form() -> compass_extension_api::View {
    let field = |name: &str, title: &str, kind: FieldKind, value, info: Option<&str>| {
        FormItem::Field(Box::new(FormField {
            id: NodeId::ROOT,
            name: name.to_owned(),
            title: Some(title.to_owned()),
            error: None,
            info: info.map(str::to_owned),
            autofocus: false,
            value,
            echo: None,
            on_change: None,
            kind,
        }))
    };
    compass_extension_api::View::Form(FormView {
        items: vec![
            field(
                "title",
                "Title",
                FieldKind::Text { placeholder: None },
                Some(FieldValue::Text("Crash on paste".to_owned())),
                Some("Shown in the issue list"),
            ),
            field(
                "urgent",
                "Urgent",
                FieldKind::Checkbox {
                    label: Some("Page someone".to_owned()),
                },
                None,
                None,
            ),
        ],
        actions: Some(ActionPanel::of([Action::new("Create Issue", "submit")])),
        ..FormView::default()
    })
}

/// Renders `view()` headlessly and reports which probes are on screen, in
/// layout order: top to bottom, then left to right.
fn layout(app: &LauncherApp) -> String {
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(800.0, 480.0),
        app.view(),
    );

    let mut found: Vec<(i32, i32, &str)> = Vec::with_capacity(PROBES.len());
    let mut absent: Vec<&str> = Vec::with_capacity(PROBES.len());

    for probe in PROBES {
        match ui.find(*probe) {
            Ok(target) => {
                let bounds = target.bounds();
                // Rounded to whole pixels: sub-pixel layout differs between
                // font stacks and is not what this asserts.
                found.push((bounds.y.round() as i32, bounds.x.round() as i32, probe));
            }
            Err(_) => absent.push(probe),
        }
    }

    found.sort_unstable();

    let mut out = String::new();
    out.push_str("on screen, in layout order:\n");
    for (_, _, probe) in &found {
        out.push_str("  ");
        out.push_str(probe);
        out.push('\n');
    }
    out.push_str("not rendered:\n");
    for probe in &absent {
        out.push_str("  ");
        out.push_str(probe);
        out.push('\n');
    }
    out
}

#[test]
fn results_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with(dir.path(), 3);
    let _ = app.update(Message::QueryChanged("Application".to_owned()));
    assert_snapshot!("results_list", layout(&app));
}

#[test]
fn empty_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with(dir.path(), 3);
    let _ = app.update(Message::QueryChanged("zzzzzzzz".to_owned()));
    assert_snapshot!("empty_state", layout(&app));
}

#[test]
fn action_panel() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with(dir.path(), 3);
    let _ = app.update(Message::QueryChanged("Application".to_owned()));
    let _ = app.update(Message::TogglePanel);
    assert_snapshot!("action_panel", layout(&app));
}

#[test]
fn detail_view() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = extension_showing(dir.path(), detail());
    assert_snapshot!("detail_view", layout(&app));
}

#[test]
fn form_view() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = extension_showing(dir.path(), form());
    assert_snapshot!("form_view", layout(&app));
}

/// The labels the snapshot says are on screen, as opposed to the ones it
/// lists as absent.
///
/// A plain `contains` over the whole snapshot cannot tell those apart — every
/// probe appears in it either way. The first version of the control below did
/// exactly that and passed while `view()` was mutated out from under it.
fn on_screen(snapshot: &str) -> Vec<&str> {
    snapshot
        .lines()
        .skip_while(|line| *line != "on screen, in layout order:")
        .skip(1)
        .take_while(|line| *line != "not rendered:")
        .map(str::trim)
        .collect()
}

/// CONTROL. The snapshots above are only worth keeping if they move when the
/// rendered tree moves. Each pair differs in the state fed to the same
/// `view()`, so an equal result means the harness is reading nothing.
#[test]
fn the_snapshots_move_when_the_rendered_tree_moves() {
    let dir = tempfile::tempdir().expect("tempdir");

    let mut listed = app_with(dir.path(), 3);
    let _ = listed.update(Message::QueryChanged("Application".to_owned()));
    let listed = layout(&listed);

    let mut empty = app_with(dir.path(), 3);
    let _ = empty.update(Message::QueryChanged("zzzzzzzz".to_owned()));
    let empty = layout(&empty);

    let mut panelled = app_with(dir.path(), 3);
    let _ = panelled.update(Message::QueryChanged("Application".to_owned()));
    let _ = panelled.update(Message::TogglePanel);
    let panelled = layout(&panelled);

    let detailed = layout(&extension_showing(dir.path(), detail()));
    let formed = layout(&extension_showing(dir.path(), form()));

    assert_ne!(listed, empty, "an empty query must not lay out like a list");
    assert_ne!(listed, panelled, "opening the panel must change the layout");
    assert_ne!(detailed, formed, "a detail must not lay out like a form");
    assert!(
        on_screen(&listed).contains(&"Application 00"),
        "the control is vacuous if the list never rendered a row: {listed}"
    );
    assert!(
        on_screen(&empty).contains(&"No results"),
        "the control is vacuous if the empty state never rendered: {empty}"
    );
    assert!(
        on_screen(&detailed).contains(&"Now with paste."),
        "the control is vacuous if the detail never rendered its Markdown: {detailed}"
    );
    assert!(
        on_screen(&formed).contains(&"Crash on paste"),
        "the control is vacuous if the form never rendered a value: {formed}"
    );
}
