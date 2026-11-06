use super::*;
use crate::app::RootRow;
use crate::app::test_support::{
    TestBackend, chord, extension_app, greeting_list, pressed, task_messages,
};
use std::sync::Arc;

#[test]
fn an_extension_command_is_a_row_and_enter_hands_it_to_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        keys: vec!["@someone/hello:write".to_owned()],
        ..TestBackend::default()
    });
    let mut app = extension_app(dir.path(), backend.clone());
    for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
        let _ = app.update(message);
    }
    assert_eq!(
        app.selected_row(),
        Some(RootRow::Extension(0)),
        "{}",
        app.state_line()
    );
    assert!(
        app.state_line()
            .contains("selected_title=\"Write Greeting\"")
    );

    let mut ui = iced_test::simulator(app.view());
    assert!(
        ui.find("Hello").is_ok(),
        "the extension's title is the subtitle"
    );
    drop(ui);

    for message in task_messages(app.update(Message::LaunchSelected)) {
        let _ = app.update(message);
    }
    assert_eq!(
        backend.ran.lock().unwrap().as_slice(),
        ["@someone/hello:write"]
    );
    assert!(app.error.is_none());
}

fn open_extension_view(
    view: compass_extension_api::View,
) -> (LauncherApp, Arc<TestBackend>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        keys: vec!["@someone/hello:write".to_owned()],
        view: Some(view),
        ..TestBackend::default()
    });
    let mut app = extension_app(dir.path(), backend.clone());
    for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
        let _ = app.update(message);
    }
    let mut pending = task_messages(app.update(Message::LaunchSelected));
    while let Some(message) = pending.pop() {
        pending.extend(task_messages(app.update(message)));
    }
    (app, backend, dir)
}

#[test]
fn an_extension_list_draws_each_titled_section_under_its_title() {
    use compass_extension_api::view::{ListItem, ListSection, ListView};
    let mut result = ListSection::untitled([ListItem::new("1.3608 kg")]);
    result.title = Some("Result".into());
    let mut others = ListSection::untitled([ListItem::new("48 oz"), ListItem::new("1360 g")]);
    others.title = Some("Other mass units".into());
    let list = ListView {
        sections: vec![
            result,
            others,
            ListSection::untitled([ListItem::new("untitled")]),
        ],
        ..ListView::default()
    };
    let (app, _backend, _dir) = open_extension_view(compass_extension_api::View::List(list));
    let mut ui = iced_test::simulator(app.view());
    for shown in [
        "Result",
        "Other mass units",
        "1.3608 kg",
        "48 oz",
        "untitled",
    ] {
        assert!(
            ui.find(shown).is_ok(),
            "{shown} is not drawn: {}",
            app.state_line()
        );
    }
}

#[test]
fn in_a_form_with_a_text_area_enter_is_a_newline_and_ctrl_enter_submits() {
    use compass_extension_api::action::{Action, ActionPanel};
    use compass_extension_api::view::{FieldKind, FormField, FormItem, FormView};
    let form = compass_extension_api::View::Form(FormView {
        items: vec![FormItem::Field(Box::new(FormField {
            id: compass_extension_api::id::NodeId::ROOT,
            name: "body".into(),
            title: Some("Body".into()),
            error: None,
            info: None,
            autofocus: false,
            value: None,
            echo: None,
            on_change: None,
            kind: FieldKind::TextArea {
                placeholder: None,
                markdown: false,
            },
        }))],
        actions: Some(ActionPanel::of([Action::new("Save", "submit")])),
        ..FormView::default()
    });
    let (mut app, backend, _dir) = open_extension_view(form);
    {
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Ctrl+Enter: Save    Esc: back").is_ok());
    }
    let mut pending = task_messages(app.update(pressed(iced::keyboard::key::Named::Enter)));
    while let Some(message) = pending.pop() {
        pending.extend(task_messages(app.update(message)));
    }
    assert!(
        backend.events.lock().unwrap().is_empty(),
        "Enter in a text area is a newline, not a submit"
    );
    let ctrl_enter = Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
        modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
        physical_key: iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        ),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::CTRL,
        text: None,
        repeat: false,
    });
    let mut pending = task_messages(app.update(ctrl_enter));
    while let Some(message) = pending.pop() {
        pending.extend(task_messages(app.update(message)));
    }
    assert_eq!(
        backend.events.lock().unwrap().as_slice(),
        [("submit".to_owned(), vec![serde_json::json!({})])]
    );
}

#[test]
fn an_extension_form_is_filled_in_and_enter_submits_its_values() {
    use compass_extension_api::action::{Action, ActionPanel, HandlerId};
    use compass_extension_api::view::{FieldKind, FormField, FormItem, FormView};
    let field = |name: &str, title: &str, kind: FieldKind| {
        FormItem::Field(Box::new(FormField {
            id: compass_extension_api::id::NodeId::ROOT,
            name: name.into(),
            title: Some(title.into()),
            error: None,
            info: None,
            autofocus: false,
            value: None,
            echo: None,
            on_change: Some(HandlerId::new(format!("change-{name}"))),
            kind,
        }))
    };
    let form = compass_extension_api::View::Form(FormView {
        items: vec![
            field("title", "Title", FieldKind::Text { placeholder: None }),
            field(
                "urgent",
                "Urgent",
                FieldKind::Checkbox {
                    label: Some("Page someone".into()),
                },
            ),
        ],
        actions: Some(ActionPanel::of([Action::new("Create Issue", "submit")])),
        ..FormView::default()
    });
    let (mut app, backend, _dir) = open_extension_view(form);
    {
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Title").is_ok(), "{}", app.state_line());
        assert!(ui.find("Enter: Create Issue    Esc: back").is_ok());
    }
    let mut pending = task_messages(app.update(Message::ExtensionFieldEdited(
        "title".into(),
        "Crash on paste".into(),
    )));
    pending.extend(task_messages(
        app.update(Message::ExtensionFieldEdited("urgent".into(), true.into())),
    ));
    pending.extend(task_messages(
        app.update(pressed(iced::keyboard::key::Named::Enter)),
    ));
    while let Some(message) = pending.pop() {
        pending.extend(task_messages(app.update(message)));
    }
    let events = backend.events.lock().unwrap().clone();
    assert!(events.contains(&(
        "change-title".to_owned(),
        vec!["Crash on paste".into(), 1.into()]
    )));
    assert!(events.contains(&(
        "submit".to_owned(),
        vec![serde_json::json!({"title": "Crash on paste", "urgent": true})]
    )));
}

#[test]
fn an_extensions_toast_is_drawn_under_its_view() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        keys: vec!["@someone/hello:write".to_owned()],
        view: Some(greeting_list(true)),
        toast: Some(crate::backend::ExtensionToast {
            failure: true,
            animated: false,
            title: "Offline".into(),
            message: "retrying".into(),
        }),
        ..TestBackend::default()
    });
    let mut app = extension_app(dir.path(), backend);
    for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
        let _ = app.update(message);
    }
    let mut pending = task_messages(app.update(Message::LaunchSelected));
    while let Some(message) = pending.pop() {
        pending.extend(task_messages(app.update(message)));
    }
    let mut ui = iced_test::simulator(app.view());
    assert!(ui.find("hello").is_ok(), "{}", app.state_line());
    assert!(ui.find("Offline — retrying").is_ok());
}

/// The engine answers a view read it held until its deadline with the
/// version the launcher already has. That is "nothing new": the page
/// keeps what it shows and asks again, rather than redrawing or stopping.
#[test]
fn a_held_view_read_that_comes_back_unchanged_just_asks_again() {
    let (mut app, _backend, _dir) = open_extension_view(greeting_list(true));
    let Page::Extension(page) = &mut app.page else {
        panic!("no extension view: {}", app.state_line());
    };
    page.selected = 1;
    let (session, version) = (page.session, page.version);
    let unchanged = crate::backend::ExtensionViewState {
        version,
        view: Some(Box::new(compass_extension_api::View::List(
            compass_extension_api::view::ListView::default(),
        ))),
        ..crate::backend::ExtensionViewState::default()
    };
    let next = task_messages(app.update(Message::ExtensionViewLoaded {
        session,
        result: Ok(unchanged),
    }));
    assert!(
        next.iter()
            .any(|message| matches!(message, Message::ExtensionViewLoaded { .. })),
        "it asks again"
    );
    let Page::Extension(page) = &app.page else {
        unreachable!()
    };
    assert_eq!(page.status, crate::extension_page::Status::Ready);
    assert_eq!(page.shown.len(), 2, "the list on screen is kept");
    assert_eq!(page.selected, 1, "and so is the selection");
}

#[test]
fn a_view_command_draws_its_list_and_enter_runs_the_selected_rows_action() {
    let (mut app, backend, _dir) = open_extension_view(greeting_list(true));
    let Page::Extension(page) = &app.page else {
        panic!("no extension view: {}", app.state_line());
    };
    assert_eq!(page.shown.len(), 2);
    {
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("hello").is_ok() && ui.find("goodbye").is_ok());
    }

    let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
    for message in task_messages(app.update(pressed(iced::keyboard::key::Named::Enter))) {
        let _ = app.update(message);
    }
    assert_eq!(
        backend.events.lock().unwrap().as_slice(),
        [("cb-goodbye".to_owned(), vec![])]
    );

    for message in task_messages(app.update(Message::ExtensionQueryChanged("hel".into()))) {
        let _ = app.update(message);
    }
    let Page::Extension(page) = &app.page else {
        unreachable!()
    };
    assert_eq!(page.shown.len(), 1, "a host-filtered list is filtered here");

    let back = app.update(pressed(iced::keyboard::key::Named::Escape));
    for message in task_messages(back) {
        let _ = app.update(message);
    }
    assert!(matches!(app.page, Page::Root));
    assert_eq!(
        backend.closed.lock().unwrap().as_slice(),
        [7],
        "leaving stops the command"
    );
}

#[test]
fn ctrl_b_offers_every_action_of_the_row_and_runs_the_one_chosen() {
    use compass_extension_api::action::{Action, ActionPanel};
    use compass_extension_api::view::{ListItem, ListSection, ListView};
    let view = compass_extension_api::View::List(ListView {
        sections: vec![ListSection::untitled([ListItem::new("repo").with_actions(
            ActionPanel::of([
                Action::new("Open in Browser", "cb-open"),
                Action::new("Copy URL", "cb-copy"),
            ]),
        )])],
        ..ListView::default()
    });
    let (mut app, backend, _dir) = open_extension_view(view);

    let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
    let panel = app
        .panel
        .as_ref()
        .expect("the panel opens over an extension view");
    let titles: Vec<&str> = panel
        .sections
        .iter()
        .flat_map(|section| section.actions.iter().map(|a| a.title.as_str()))
        .collect();
    assert_eq!(titles, ["Open in Browser", "Copy URL"]);

    let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
    for message in task_messages(app.update(pressed(iced::keyboard::key::Named::Enter))) {
        let _ = app.update(message);
    }
    assert_eq!(
        backend.events.lock().unwrap().as_slice(),
        [("cb-copy".to_owned(), vec![])]
    );
    assert!(app.panel.is_none(), "running an action closes the panel");
    assert!(
        matches!(app.page, Page::Extension(_)),
        "and stays in the view"
    );
}

#[test]
fn an_actions_shortcut_runs_it_and_the_panel_shows_the_chord() {
    use compass_extension_api::action::{Action, ActionPanel, KeyModifier, Shortcut};
    use compass_extension_api::view::{ListItem, ListSection, ListView};
    let view = compass_extension_api::View::List(ListView {
        sections: vec![ListSection::untitled([ListItem::new("repo").with_actions(
            ActionPanel::of([
                Action::new("Open", "cb-open"),
                Action::new("Copy URL", "cb-copy")
                    .with_shortcut(Shortcut::new([KeyModifier::Ctrl, KeyModifier::Shift], "c")),
            ]),
        )])],
        ..ListView::default()
    });
    let (mut app, backend, _dir) = open_extension_view(view);

    for message in task_messages(app.update(chord("c", iced::keyboard::Modifiers::default()))) {
        let _ = app.update(message);
    }
    assert!(
        backend.events.lock().unwrap().is_empty(),
        "typing c is not a shortcut"
    );

    let ctrl_shift = iced::keyboard::Modifiers::CTRL | iced::keyboard::Modifiers::SHIFT;
    for message in task_messages(app.update(chord("C", ctrl_shift))) {
        let _ = app.update(message);
    }
    assert_eq!(
        backend.events.lock().unwrap().as_slice(),
        [("cb-copy".to_owned(), vec![])]
    );

    let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
    let shortcuts: Vec<Option<&str>> = app
        .panel
        .as_ref()
        .expect("the panel")
        .sections
        .iter()
        .flat_map(|section| section.actions.iter().map(|a| a.shortcut.as_deref()))
        .collect();
    assert_eq!(shortcuts, [None, Some("Ctrl+Shift+C")]);
}

#[test]
fn escape_pops_a_pushed_view_and_only_the_root_view_closes_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        keys: vec!["@someone/hello:write".to_owned()],
        view: Some(greeting_list(true)),
        depth: std::sync::Mutex::new(2),
        ..TestBackend::default()
    });
    let mut app = extension_app(dir.path(), backend.clone());
    for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
        let _ = app.update(message);
    }
    let mut pending = task_messages(app.update(Message::LaunchSelected));
    while let Some(message) = pending.pop() {
        pending.extend(task_messages(app.update(message)));
    }

    for message in task_messages(app.update(pressed(iced::keyboard::key::Named::Escape))) {
        let _ = app.update(message);
    }
    assert_eq!(
        backend.events.lock().unwrap().as_slice(),
        [("pop".to_owned(), vec![])]
    );
    assert!(
        matches!(app.page, Page::Extension(_)),
        "still in the extension"
    );
    assert!(backend.closed.lock().unwrap().is_empty());

    // The extension re-renders its root view.
    if let Page::Extension(page) = &mut app.page {
        page.apply(crate::backend::ExtensionViewState {
            version: 9,
            view: Some(Box::new(greeting_list(true))),
            problem: None,
            ended: false,
            depth: 1,
            alert: None,
            toast: None,
        });
    }
    for message in task_messages(app.update(pressed(iced::keyboard::key::Named::Escape))) {
        let _ = app.update(message);
    }
    assert!(matches!(app.page, Page::Root));
    assert_eq!(backend.closed.lock().unwrap().as_slice(), [7]);
}

#[test]
fn an_alert_is_shown_and_enter_or_escape_answers_it() {
    for (key, confirmed) in [
        (iced::keyboard::key::Named::Enter, true),
        (iced::keyboard::key::Named::Escape, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["@someone/hello:write".to_owned()],
            view: Some(greeting_list(true)),
            alert: Some(crate::backend::ExtensionPrompt {
                title: "Delete the repository?".into(),
                message: "This cannot be undone.".into(),
                confirm_text: "Delete".into(),
                cancel_text: "Keep".into(),
                remember_text: None,
            }),
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
            let _ = app.update(message);
        }
        let mut pending = task_messages(app.update(Message::LaunchSelected));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Delete the repository?").is_ok());
            assert!(ui.find("Enter: Delete    Esc: Keep").is_ok());
        }
        for message in task_messages(app.update(pressed(key))) {
            let _ = app.update(message);
        }
        assert_eq!(backend.answers.lock().unwrap().as_slice(), [confirmed]);
        assert!(backend.events.lock().unwrap().is_empty(), "no action ran");
        assert!(
            matches!(&app.page, Page::Extension(page) if page.alert.is_none()),
            "answered, and still in the view"
        );
    }
}

#[test]
fn a_consent_alert_takes_ctrl_enter_as_always_allow() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        keys: vec!["@someone/hello:write".to_owned()],
        view: Some(greeting_list(true)),
        alert: Some(crate::backend::ExtensionPrompt {
            title: "Allow Brew to run brew?".into(),
            message: String::new(),
            confirm_text: "Allow Once".into(),
            cancel_text: "Deny".into(),
            remember_text: Some("Always Allow".into()),
        }),
        ..TestBackend::default()
    });
    let mut app = extension_app(dir.path(), backend.clone());
    for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
        let _ = app.update(message);
    }
    let mut pending = task_messages(app.update(Message::LaunchSelected));
    while let Some(message) = pending.pop() {
        pending.extend(task_messages(app.update(message)));
    }
    {
        let mut ui = iced_test::simulator(app.view());
        assert!(
            ui.find("Enter: Allow Once    Ctrl+Enter: Always Allow    Esc: Deny")
                .is_ok()
        );
    }
    let ctrl_enter = Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
        modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
        physical_key: iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        ),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::CTRL,
        text: None,
        repeat: false,
    });
    for message in task_messages(app.update(ctrl_enter)) {
        let _ = app.update(message);
    }
    assert_eq!(backend.remembered.lock().unwrap().len(), 1);
    assert!(backend.answers.lock().unwrap().is_empty());
}

#[test]
fn a_command_missing_a_preference_asks_for_it_then_runs() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        keys: vec!["@someone/hello:write".to_owned()],
        needs: vec![crate::backend::PreferenceInput {
            name: "token".into(),
            title: "Token".into(),
            description: "From your account settings".into(),
            placeholder: String::new(),
            required: true,
            kind: crate::backend::PreferenceInputKind::Password,
            value: None,
        }],
        ..TestBackend::default()
    });
    let mut app = extension_app(dir.path(), backend.clone());
    for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
        let _ = app.update(message);
    }
    for message in task_messages(app.update(Message::LaunchSelected)) {
        let _ = app.update(message);
    }
    assert!(
        matches!(app.page, Page::Preferences(_)),
        "{}",
        app.state_line()
    );
    {
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Token *").is_ok(), "required fields are marked");
    }

    let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
    let Page::Preferences(page) = &app.page else {
        unreachable!()
    };
    assert_eq!(page.notice.as_deref(), Some("Fill in Token"));
    assert!(backend.saved.lock().unwrap().is_empty());

    let _ = app.update(Message::PreferenceEdited(
        0,
        crate::preferences_page::FieldValue::Text("ghp_x".into()),
    ));
    let mut pending = task_messages(app.update(pressed(iced::keyboard::key::Named::Enter)));
    while let Some(message) = pending.pop() {
        pending.extend(task_messages(app.update(message)));
    }
    assert_eq!(
        backend.saved.lock().unwrap().as_slice(),
        [serde_json::json!({"token": "ghp_x"})
            .as_object()
            .unwrap()
            .clone()]
    );
    assert_eq!(
        backend.ran.lock().unwrap().as_slice(),
        ["@someone/hello:write", "@someone/hello:write"],
        "saved, then run again"
    );
    assert!(matches!(app.page, Page::Root), "and it ran");
}

#[test]
fn a_command_with_arguments_asks_for_them_and_runs_with_what_was_entered() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        keys: vec!["@someone/hello:write".to_owned()],
        wants: vec![crate::backend::PreferenceInput {
            name: "name".into(),
            title: "Name".into(),
            description: String::new(),
            placeholder: "Name".into(),
            required: true,
            kind: crate::backend::PreferenceInputKind::Text,
            value: None,
        }],
        ..TestBackend::default()
    });
    let mut app = extension_app(dir.path(), backend.clone());
    for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
        let _ = app.update(message);
    }
    for message in task_messages(app.update(Message::LaunchSelected)) {
        let _ = app.update(message);
    }
    assert!(
        matches!(&app.page, Page::Preferences(page)
            if page.purpose == crate::preferences_page::Purpose::Arguments),
        "{}",
        app.state_line()
    );

    let _ = app.update(Message::PreferenceEdited(
        0,
        crate::preferences_page::FieldValue::Text("Ada".into()),
    ));
    let mut pending = task_messages(app.update(pressed(iced::keyboard::key::Named::Enter)));
    while let Some(message) = pending.pop() {
        pending.extend(task_messages(app.update(message)));
    }
    assert!(
        backend.saved.lock().unwrap().is_empty(),
        "arguments are this run's, never kept"
    );
    assert_eq!(
        backend.given.lock().unwrap().as_slice(),
        [
            None,
            Some(
                serde_json::json!({"name": "Ada"})
                    .as_object()
                    .unwrap()
                    .clone()
            )
        ]
    );
    assert!(matches!(app.page, Page::Root), "and it ran");
}

#[test]
fn a_detail_draws_its_markdown_rendered_not_as_source() {
    let detail = compass_extension_api::View::Detail(compass_extension_api::view::Detail {
        markdown: Some("# Release notes\n\nNow with **paste**.".into()),
        ..compass_extension_api::view::Detail::default()
    });
    let (app, _backend, _dir) = open_extension_view(detail);
    let Page::Extension(page) = &app.page else {
        panic!("no extension view: {}", app.state_line());
    };
    use iced::widget::markdown::Item;
    assert!(
        matches!(
            page.markdown.as_slice(),
            [Item::Heading(..), Item::Paragraph(..)]
        ),
        "parsed once, when it arrived, into a heading and a paragraph: {:?}",
        page.markdown
    );
    let mut ui = iced_test::simulator(app.view());
    assert!(
        ui.find("# Release notes").is_err(),
        "not the Markdown source"
    );
}

#[test]
fn an_extension_grid_is_drawn_as_tiles_and_the_arrows_move_by_cell_and_row() {
    use compass_extension_api::view::{Color, GridContent, GridItem, GridSection, GridView};
    let cell = |title: &str| GridItem {
        id: compass_extension_api::id::NodeId::ROOT,
        key: None,
        title: title.into(),
        subtitle: None,
        content: GridContent::Color(Color::Literal("#336699".into())),
        tooltip: None,
        keywords: Vec::new(),
        actions: None,
    };
    let grid = compass_extension_api::View::Grid(GridView {
        columns: Some(3),
        sections: vec![GridSection {
            title: Some("Swatches".into()),
            items: ["Navy", "Teal", "Plum", "Rust"]
                .iter()
                .map(|t| cell(t))
                .collect(),
            ..GridSection::default()
        }],
        ..GridView::default()
    });
    let (mut app, _backend, _dir) = open_extension_view(grid);
    {
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Swatches").is_ok(), "the section's title");
        assert!(ui.find("Rust").is_ok(), "each cell's title under its tile");
        assert!(
            ui.find("N").is_err(),
            "a tile, not a row with an initial: {}",
            app.state_line()
        );
    }
    let selected = |app: &LauncherApp| match &app.page {
        Page::Extension(page) => page.selected,
        _ => panic!("left the grid"),
    };
    let _ = app.update(pressed(iced::keyboard::key::Named::ArrowRight));
    assert_eq!(selected(&app), 1, "Right is the next cell");
    let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
    assert_eq!(
        selected(&app),
        3,
        "Down keeps the column, clamped to the short row"
    );
    let _ = app.update(pressed(iced::keyboard::key::Named::ArrowUp));
    assert_eq!(selected(&app), 0, "and Up goes back a row");
}

#[test]
fn a_list_that_filters_itself_gets_the_text_and_the_echo_count() {
    let (mut app, backend, _dir) = open_extension_view(greeting_list(false));
    for message in task_messages(app.update(Message::ExtensionQueryChanged("zz".into()))) {
        let _ = app.update(message);
    }
    assert_eq!(
        backend.events.lock().unwrap().as_slice(),
        [(
            "cb-search".to_owned(),
            vec![serde_json::json!("zz"), serde_json::json!(1)]
        )]
    );
    let Page::Extension(page) = &app.page else {
        unreachable!()
    };
    assert_eq!(
        page.shown.len(),
        2,
        "what matches is the extension's to say"
    );
}

#[test]
fn a_refused_extension_command_says_why_and_stays() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(TestBackend {
        keys: vec!["@someone/hello:write".to_owned()],
        refuse_runs: Some("Running extensions needs Node.js, and none was found".into()),
        ..TestBackend::default()
    });
    let mut app = extension_app(dir.path(), backend);
    for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
        let _ = app.update(message);
    }
    for message in task_messages(app.update(Message::LaunchSelected)) {
        let _ = app.update(message);
    }
    assert_eq!(
        app.error.as_deref(),
        Some("Running extensions needs Node.js, and none was found")
    );
}
