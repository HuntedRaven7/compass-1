use super::*;
use crate::action_panel::{Action, PanelSection};
use crate::app::PANEL_INPUT;
use crate::app::flags::AppFlags;
use crate::app::icons::IconLookup;
use crate::app::panel::PanelState;
use crate::design::Appearance;
use crate::message::Direction;
use crate::preset;
use crate::preset::Preset;
use compass_core::AppIndex;

#[test]
fn action_menu_geometry_matches_the_spacing_contract() {
    for (name, _) in preset::NAMES {
        for appearance in Appearance::ALL {
            let mut app = LauncherApp::with_index(AppIndex::builder().build());
            app.apply(AppFlags {
                appearance,
                appearance_preset: preset::resolve(Some(name), None, None),
                ..AppFlags::default()
            });
            let panel = PanelState::new(vec![
                PanelSection {
                    name: String::new(),
                    actions: vec![Action::new("Open").with_shortcut("enter")],
                },
                PanelSection {
                    name: "Copy".into(),
                    actions: vec![Action::new("Copy name"), Action::new("Copy path")],
                },
            ]);
            let mut row = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(288.0, 34.0),
                app.panel_item("Open", Some("enter"), true),
            );
            for label in ["Open", "Enter"] {
                let bounds = row.find(label).unwrap().bounds();
                assert!(
                    (bounds.y + bounds.height / 2.0 - 17.0).abs() < 0.1,
                    "{name}: {label} not centered"
                );
            }
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(300.0, 240.0),
                app.view_panel(&panel),
            );
            let divider = ui
                .find(iced_test::selector::id("panel-divider-line"))
                .unwrap()
                .bounds();
            assert_eq!(
                divider.height, 1.0,
                "padding must not be painted as a divider"
            );
            let open = ui.find("Open").unwrap().bounds();
            let heading = ui.find("COPY").unwrap().bounds();
            let copy = ui.find("Copy name").unwrap().bounds();
            assert_eq!(open.x, heading.x);
            assert_eq!(open.x, copy.x);
            assert!(divider.y > open.y + open.height);
            assert!(heading.y > divider.y + divider.height);
            if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
                ui.click(iced_test::selector::id(PANEL_INPUT)).unwrap();
                assert!(
                    ui.snapshot(&app.theme())
                        .unwrap()
                        .matches_image(
                            std::path::PathBuf::from(directory)
                                .join(format!("{}-{name}-action-menu.png", appearance.name()))
                        )
                        .unwrap()
                );
            }
        }
    }
}

#[test]
fn mixed_result_rows_center_their_labels_and_render_resolved_icons() {
    let dir = tempfile::tempdir().unwrap();
    let icon = dir.path().join("test.svg");
    std::fs::write(
        &icon,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" rx="7" fill="#33a36d"/><path d="M8 9l7 7-7 7m10 0h7" stroke="white" stroke-width="3" fill="none"/></svg>"##,
    )
    .unwrap();
    for (id, comment) in [("Editor", "Comment=Edit documents\n"), ("Terminal", "")] {
        std::fs::write(
            dir.path().join(format!("{id}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=Test {id}\nExec=/bin/true\nIcon=test\n{comment}"),
        )
        .unwrap();
    }
    for (name, _) in preset::NAMES {
        for appearance in Appearance::ALL {
            let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
            let path = icon.clone();
            app.apply(AppFlags {
                appearance,
                appearance_preset: preset::resolve(Some(name), None, None),
                icon_lookup: IconLookup::new(move |_| Some(path.clone())),
                ..AppFlags::default()
            });
            let _ = app.update(Message::QueryChanged("Test".into()));
            for row in &app.results {
                let RootRow::App(index) = row else { continue };
                let item = &app.app_index.items()[*index];
                assert!(matches!(
                    app.row_art(item),
                    Some(crate::icons::IconArt::Vector(_))
                ));
                let mut ui = iced_test::Simulator::with_size(
                    iced::Settings::default(),
                    iced::Size::new(720.0, f32::from(app.geometry.row_height)),
                    app.result_row(item, false),
                );
                let title = ui.find(item.name()).unwrap().bounds();
                let bottom = if app.subtitles && item.comment().is_some() {
                    let subtitle = ui.find(item.comment().unwrap()).unwrap().bounds();
                    subtitle.y + subtitle.height
                } else {
                    title.y + title.height
                };
                assert!(
                    ((title.y + bottom) / 2.0 - f32::from(app.geometry.row_height) / 2.0).abs()
                        < 0.1,
                    "{name}: label block is not centered"
                );
            }
            if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
                let mut ui = iced_test::Simulator::with_size(
                    iced::Settings::default(),
                    iced::Size::new(800.0, 320.0),
                    app.view(),
                );
                assert!(
                    ui.snapshot(&app.theme())
                        .unwrap()
                        .matches_image(
                            std::path::PathBuf::from(directory)
                                .join(format!("{}-{name}-mixed-rows.png", appearance.name()))
                        )
                        .unwrap()
                );
            }
        }
    }
}

#[test]
fn query_input_leaves_the_border_and_fill_to_its_container() {
    for theme in [Theme::Light, Theme::Dark] {
        for status in [
            text_input::Status::Active,
            text_input::Status::Hovered,
            text_input::Status::Focused { is_hovered: false },
            text_input::Status::Focused { is_hovered: true },
            text_input::Status::Disabled,
        ] {
            let style = query_input_style(&theme, status, design::LIGHT);
            let default = text_input::default(&theme, status);
            assert_eq!(style.background, Color::TRANSPARENT.into());
            assert_eq!(style.border.width, 0.0);
            assert_eq!(style.value, default.value);
            assert_eq!(style.placeholder, design::LIGHT.muted.to_iced());
            assert_eq!(style.selection, default.selection);
        }
    }
}

fn long_results(dir: &std::path::Path) -> LauncherApp {
    for i in 0..40 {
        std::fs::write(
            dir.join(format!("app-{i:02}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=Application {i:02}\nExec=/bin/true\n"),
        )
        .unwrap();
    }
    let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir).build());
    let _ = app.update(Message::QueryChanged("Application".to_owned()));
    app
}

#[test]
fn long_root_results_scroll_without_moving_the_query_field() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = long_results(dir.path());
    for appearance in Appearance::ALL {
        app.view.appearance = appearance;
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(800.0, 320.0),
            app.view(),
        );
        let field = ui
            .find(iced_test::selector::id(SEARCH_INPUT))
            .unwrap()
            .bounds();
        assert!(
            ui.find("Application 39")
                .unwrap()
                .visible_bounds()
                .is_none()
        );
        if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
            assert!(
                ui.snapshot(&app.theme())
                    .unwrap()
                    .matches_image(
                        std::path::PathBuf::from(directory)
                            .join(format!("{}-root-start.png", appearance.name()))
                    )
                    .unwrap()
            );
        }
        ui.point_at(iced::Point::new(400.0, 200.0));
        ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Pixels {
                x: 0.0,
                y: -10000.0,
            },
        })]);
        assert!(
            ui.find("Application 39")
                .unwrap()
                .visible_bounds()
                .is_some()
        );
        assert_eq!(
            ui.find(iced_test::selector::id(SEARCH_INPUT))
                .unwrap()
                .bounds(),
            field
        );
        if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
            assert!(
                ui.snapshot(&app.theme())
                    .unwrap()
                    .matches_image(
                        std::path::PathBuf::from(directory)
                            .join(format!("{}-root-scrolled.png", appearance.name()))
                    )
                    .unwrap()
            );
        }
    }
}

#[test]
fn root_keyboard_selection_stays_visible_across_presets_and_query_changes() {
    use iced::futures::{StreamExt, executor::block_on};
    use iced_test::Selector;
    use iced_winit::{
        core::{
            renderer::Headless,
            widget::{
                Operation,
                operation::{self, Outcome},
            },
        },
        runtime::{Action as RuntimeAction, UserInterface, user_interface},
    };

    let mut renderer = block_on(iced::Renderer::new(
        iced::Font::DEFAULT,
        iced::Pixels(16.0),
        None,
    ))
    .unwrap();
    for (name, _) in preset::NAMES {
        let dir = tempfile::tempdir().unwrap();
        let mut app = long_results(dir.path());
        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some(name), None, None),
            ..AppFlags::default()
        });
        app.wrap_navigation = true;
        let mut cache = user_interface::Cache::default();
        let moves = std::iter::repeat_n(Message::MoveSelection(Direction::Down), 39)
            .chain([
                Message::MoveSelection(Direction::Down),
                Message::MoveSelection(Direction::Up),
                Message::QueryChanged("Application 00".to_owned()),
                Message::QueryChanged("Application".to_owned()),
            ])
            .chain(std::iter::repeat_n(
                Message::MoveSelection(Direction::Down),
                39,
            ))
            .chain(std::iter::repeat_n(
                Message::MoveSelection(Direction::Up),
                39,
            ));
        for message in moves {
            let task = app.update(message);
            let mut ui = UserInterface::build(
                app.view(),
                iced::Size::new(800.0, 320.0),
                cache,
                &mut renderer,
            );
            let actions = iced_winit::runtime::task::into_stream(task)
                .map(|stream| block_on(stream.collect::<Vec<_>>()))
                .unwrap_or_default();
            for action in actions {
                let RuntimeAction::Widget(mut operation) = action else {
                    panic!("expected widget operation")
                };
                loop {
                    ui.operate(&renderer, operation.as_mut());
                    match operation.finish() {
                        Outcome::Chain(next) => operation = next,
                        Outcome::None => break,
                        Outcome::Some(()) => panic!("unexpected output"),
                    }
                }
            }
            let mut selected = iced_test::selector::id(crate::scroll::ROOT_SELECTION).find();
            ui.operate(&renderer, &mut operation::black_box(&mut selected));
            let Outcome::Some(Some(selected)) = selected.finish() else {
                panic!("missing selection")
            };
            let visible = selected
                .visible_bounds()
                .expect("selected application is offscreen");
            assert!(
                (visible.height - selected.bounds().height).abs() < 0.1,
                "{name}: clipped selection {selected:?}"
            );
            cache = ui.into_cache();
        }
    }
}

#[test]
fn long_action_panels_keep_a_bounded_scroll_region_below_the_filter() {
    let mut app = LauncherApp::with_index(AppIndex::builder().build());
    for appearance in Appearance::ALL {
        app.view.appearance = appearance;
        let panel = PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: (0..40)
                .map(|i| Action::new(format!("Action {i}")))
                .collect(),
        }]);
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(320.0, 180.0),
            app.view_panel(&panel),
        );
        let filter = ui
            .find(iced_test::selector::id(PANEL_INPUT))
            .unwrap()
            .bounds();
        let scroller = ui
            .find(iced_test::selector::id("panel-results"))
            .unwrap()
            .bounds();
        assert!(filter.y >= 0.0 && filter.y + filter.height <= 180.0);
        assert!(scroller.y >= filter.y + filter.height);
        assert!(scroller.y + scroller.height <= 180.0);
        assert!(scroller.height > 34.0);
        assert!(ui.find("Action 39").unwrap().visible_bounds().is_none());
        if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
            assert!(
                ui.snapshot(&app.theme())
                    .unwrap()
                    .matches_image(
                        std::path::PathBuf::from(directory)
                            .join(format!("{}-panel-start.png", appearance.name()))
                    )
                    .unwrap()
            );
        }
        ui.point_at(iced::Point::new(150.0, 100.0));
        ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Pixels { x: 0.0, y: -2000.0 },
        })]);
        assert!(ui.find("Action 39").unwrap().visible_bounds().is_some());
        assert_eq!(
            ui.find(iced_test::selector::id(PANEL_INPUT))
                .unwrap()
                .bounds(),
            filter
        );
        if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
            assert!(
                ui.snapshot(&app.theme())
                    .unwrap()
                    .matches_image(
                        std::path::PathBuf::from(directory)
                            .join(format!("{}-panel-scrolled.png", appearance.name()))
                    )
                    .unwrap()
            );
        }
    }
}

#[test]
fn keyboard_selection_stays_visible_in_the_real_panel_widget_tree() {
    use iced::futures::{StreamExt, executor::block_on};
    use iced_test::Selector;
    use iced_winit::{
        core::{
            renderer::Headless,
            widget::{
                Operation,
                operation::{self, Outcome},
            },
        },
        runtime::{Action as RuntimeAction, UserInterface, user_interface},
    };

    let mut app = LauncherApp::with_index(AppIndex::builder().build());
    app.panel = Some(PanelState::new(vec![PanelSection {
        name: "Actions".to_owned(),
        actions: (0..40)
            .map(|i| Action::new(format!("Action {i:02}")))
            .collect(),
    }]));
    let mut renderer = block_on(iced::Renderer::new(
        iced::Font::DEFAULT,
        iced::Pixels(16.0),
        None,
    ))
    .unwrap();
    let mut cache = user_interface::Cache::default();
    let size = iced::Size::new(320.0, 180.0);
    app.wrap_navigation = true;
    let moves = std::iter::repeat_n(Message::PanelMove(Direction::Down), 39)
        .chain([
            Message::PanelMove(Direction::Down),
            Message::PanelMove(Direction::Up),
            Message::PanelFilterChanged("Action 00".to_owned()),
            Message::PanelFilterChanged(String::new()),
        ])
        .chain(std::iter::repeat_n(Message::PanelMove(Direction::Down), 39))
        .chain(std::iter::repeat_n(Message::PanelMove(Direction::Up), 39));
    for message in moves {
        let task = app.update(message);
        let mut ui = UserInterface::build(
            app.view_panel(app.panel.as_ref().unwrap()),
            size,
            cache,
            &mut renderer,
        );
        let actions = iced_winit::runtime::task::into_stream(task)
            .map(|stream| block_on(stream.collect::<Vec<_>>()))
            .unwrap_or_default();
        for action in actions {
            let RuntimeAction::Widget(mut operation) = action else {
                panic!("expected scroll operation")
            };
            loop {
                ui.operate(&renderer, operation.as_mut());
                match operation.finish() {
                    Outcome::Chain(next) => operation = next,
                    Outcome::None => break,
                    Outcome::Some(()) => panic!("unexpected output"),
                }
            }
        }
        let mut selected = iced_test::selector::id(crate::scroll::PANEL_SELECTION).find();
        ui.operate(&renderer, &mut operation::black_box(&mut selected));
        let Outcome::Some(Some(selected)) = selected.finish() else {
            panic!("missing selected widget")
        };
        let visible = selected
            .visible_bounds()
            .expect("selection must be onscreen");
        assert!(
            (visible.height - selected.bounds().height).abs() < 0.1,
            "selection clipped: {selected:?}"
        );
        cache = ui.into_cache();
    }
}

/// A launcher showing results, built through the code `compass` uses.
fn app_showing_results(dir: &std::path::Path, preset_name: &str) -> LauncherApp {
    // Comments matter: the subtitle is what `subtitles` hides, and a
    // fixture without one would make that assertion measure nothing.
    for (file, name, comment) in [
        ("firefox.desktop", "Firefox", "Browse the web"),
        ("files.desktop", "Files", "Access and organize files"),
        ("terminal.desktop", "Terminal", "Run commands"),
    ] {
        std::fs::write(
            dir.join(file),
            format!(
                "[Desktop Entry]\nType=Application\nName={name}\nComment={comment}\nExec=/bin/true\n"
            ),
        )
        .expect("write entry");
    }
    let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir).build());
    app.apply(AppFlags {
        appearance_preset: preset::resolve(Some(preset_name), None, None),
        ..AppFlags::default()
    });
    // `fi` matches Firefox and Files, so there is a row beneath the first
    // one -- which is the only place a height change is visible.
    let _ = app.update(Message::QueryChanged("fi".to_owned()));
    app
}

/// The top of the row whose title is `title`, as the renderer laid it out.
fn row_top(app: &LauncherApp, title: &str) -> f32 {
    let mut ui = iced_test::simulator(app.view());
    ui.find(title)
        .unwrap_or_else(|error| panic!("no row titled {title:?}: {error:?}"))
        .bounds()
        .y
}

#[test]
fn the_view_renders_and_the_results_are_in_it() {
    // The precondition for everything below. If this fails, the assertions
    // that follow are measuring nothing.
    let dir = tempfile::tempdir().expect("tempdir");
    let app = app_showing_results(dir.path(), "gnome");
    assert!(!app.results.is_empty(), "precondition: there are rows");

    let mut ui = iced_test::simulator(app.view());
    assert!(ui.find("Firefox").is_ok(), "the row should be drawn");
}

#[test]
fn each_notice_state_says_what_it_should() {
    // #13 names "results list, empty state, detail view, form" for this
    // coverage. These are the three the launcher actually has; a detail
    // view and a form do not exist yet, and inventing tests for them would
    // be testing nothing.
    //
    // Each of these is one branch of the same `if` in `view`, and the
    // branches are only distinguishable by the words they draw -- which is
    // exactly what a user distinguishes them by.
    let dir = tempfile::tempdir().expect("tempdir");

    // Empty query: the resting state, and the one most people see most.
    let mut app = app_showing_results(dir.path(), "gnome");
    let _ = app.update(Message::QueryChanged(String::new()));
    let mut ui = iced_test::simulator(app.view());
    assert!(ui.find("Type to search").is_ok());
    assert!(
        ui.find("Files").is_err(),
        "an empty query should draw no rows"
    );

    // A query nothing matches.
    let mut app = app_showing_results(dir.path(), "gnome");
    let _ = app.update(Message::QueryChanged("zzzznotathing".to_owned()));
    let mut ui = iced_test::simulator(app.view());
    assert!(ui.find("No results").is_ok());
    assert!(ui.find("Type to search").is_err());

    // A failed launch, which replaces the list rather than sitting beside
    // it -- worth pinning, because the error hiding the results is a
    // deliberate choice and not obviously the right one.
    let mut app = app_showing_results(dir.path(), "gnome");
    let _ = app.update(Message::Launched(Err("no Exec key".to_owned())));
    let mut ui = iced_test::simulator(app.view());
    assert!(
        ui.find("could not launch: no Exec key").is_ok(),
        "the failure should name itself"
    );
    assert!(
        ui.find("Files").is_err(),
        "the error replaces the list while it is shown"
    );
}

#[test]
fn empty_query_suggestions_are_visible_instead_of_the_greeting() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_showing_results(dir.path(), "gnome");
    app.query.clear();
    let mut ui = iced_test::simulator(app.view());
    assert!(ui.find("Firefox").is_ok());
    assert!(ui.find("Type to search").is_err());
}

#[test]
fn a_search_failure_is_not_presented_as_a_failed_launch() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_showing_results(dir.path(), "gnome");
    let _ = app.update(Message::SearchCompleted {
        generation: app.search_generation,
        result: Err("engine unavailable".to_owned()),
    });
    let mut ui = iced_test::simulator(app.view());
    assert!(ui.find("could not search: engine unavailable").is_ok());
    assert!(ui.find("could not launch: engine unavailable").is_err());
}

#[test]
fn the_action_panel_draws_over_the_list_rather_than_replacing_it() {
    // The other half of the same claim: `view` stacks the panel on the
    // card instead of swapping it, and the comment in `view` says so. A
    // test that only checked the panel was visible would pass either way.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_showing_results(dir.path(), "gnome");
    let _ = app.update(Message::TogglePanel);
    assert!(app.panel.is_some(), "precondition: the panel opened");

    let mut ui = iced_test::simulator(app.view());
    assert!(ui.find("Open").is_ok(), "the panel's first action");
    assert!(
        ui.find("Files").is_ok(),
        "the list should still be underneath"
    );
}

#[test]
fn the_flow_rule_is_actually_drawn() {
    // THE CONTROL #108 COULD NOT CLOSE.
    //
    // Two launchers identical in every respect except `field_rule`, so the
    // difference measured is that branch and nothing else -- comparing the
    // `flow` and `gnome` presets instead would conflate radius, padding and
    // spacing with the rule.
    //
    // The rule is a one-pixel container between the field and the body, so
    // drawing it moves everything below it down by exactly its height.
    let dir = tempfile::tempdir().expect("tempdir");

    let mut without = app_showing_results(dir.path(), "gnome");
    without.field_rule = false;
    let mut with = app_showing_results(dir.path(), "gnome");
    with.field_rule = true;

    let baseline = row_top(&without, "Firefox");
    let ruled = row_top(&with, "Firefox");

    assert!(
        (ruled - baseline - 1.0).abs() < f32::EPSILON,
        "the rule should push the list down by its own 1px height: \
         {baseline} without, {ruled} with"
    );
}

#[test]
fn hiding_subtitles_removes_them_from_the_drawn_tree() {
    // The trait the rendered surrogate caught by eye and no test could.
    //
    // Asserted by asking the tree for the subtitle's text rather than by
    // measuring anything: rows are a fixed height, so hiding the subtitle
    // re-centres the title inside the row instead of shrinking it, and a
    // geometric assertion here would be measuring the centring. Whether
    // the words are on screen is the thing the option is actually about.
    let dir = tempfile::tempdir().expect("tempdir");

    let mut with = app_showing_results(dir.path(), "gnome");
    with.subtitles = true;
    let mut without = app_showing_results(dir.path(), "gnome");
    without.subtitles = false;

    let mut shown = iced_test::simulator(with.view());
    assert!(
        shown.find("Access and organize files").is_ok(),
        "the subtitle should be drawn when subtitles are on"
    );

    let mut hidden = iced_test::simulator(without.view());
    assert!(
        hidden.find("Access and organize files").is_err(),
        "the subtitle should be gone when subtitles are off"
    );
    // And the title is still there, so the row was not simply dropped.
    assert!(hidden.find("Files").is_ok());
}

#[test]
fn the_dense_preset_really_is_denser_on_screen() {
    // Not a token comparison -- `preset::tests` already does that. This is
    // the laid-out result, which is the thing a user sees.
    //
    // The pitch between two rows, in absolute value: the ranking decides
    // which of the two is on top, and that is not what this is about.
    let dir = tempfile::tempdir().expect("tempdir");
    let gnome = app_showing_results(dir.path(), "gnome");
    let rofi = app_showing_results(dir.path(), "rofi");

    assert!(
        gnome.results.len() >= 2 && rofi.results.len() >= 2,
        "precondition: two rows, or there is no pitch to measure"
    );
    let gnome_pitch = (row_top(&gnome, "Files") - row_top(&gnome, "Firefox")).abs();
    let rofi_pitch = (row_top(&rofi, "Files") - row_top(&rofi, "Firefox")).abs();

    assert!(
        rofi_pitch < gnome_pitch,
        "rofi's rows should be closer together: {rofi_pitch} vs {gnome_pitch}"
    );

    // And each pitch is the preset's own row height plus its spacing.
    //
    // The inequality above is not enough on its own: `rofi` also sets
    // `row_spacing` to 0, so a `view` that ignored `row_height` entirely
    // would still lay rofi out 2 px tighter and satisfy it. A control
    // caught exactly that. Pinning both pitches to their presets' numbers
    // means the height has to be read from the preset for either to hold.
    for (app, preset) in [(&gnome, Preset::Gnome), (&rofi, Preset::Rofi)] {
        let g = preset.geometry();
        let expected = f32::from(g.row_height) + f32::from(g.row_spacing);
        let measured = (row_top(app, "Files") - row_top(app, "Firefox")).abs();
        assert!(
            (measured - expected).abs() < 0.01,
            "{} should lay rows out {expected} apart, measured {measured}",
            preset.name()
        );
    }
}
