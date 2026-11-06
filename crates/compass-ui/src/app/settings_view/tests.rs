//! The settings view against a real launcher state: opening it, each kind of
//! control, the extension page's switches, aliases and shortcuts, and who
//! writes the file.

use std::sync::{Arc, Mutex};

use super::*;
use crate::app::AppFlags;
use crate::backend::BackendFuture;
use crate::settings_page::{Shown, alias_key};
use compass_core::settings_catalog::CorePage;
use serde_json::json;

fn settle(app: &mut LauncherApp, task: Task<Message>) {
    use iced::futures::{StreamExt, executor::block_on};
    use iced_winit::runtime::{Action, task};
    let Some(stream) = task::into_stream(task) else {
        return;
    };
    for action in block_on(stream.collect::<Vec<_>>()) {
        if let Action::Output(message) = action {
            let next = app.update(message);
            settle(app, next);
        }
    }
}

fn key(key: Key, modifiers: Modifiers) -> Message {
    let physical =
        iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified);
    Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: physical,
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    })
}

fn app(dir: &std::path::Path) -> LauncherApp {
    for (file, name) in [("firefox.desktop", "Firefox"), ("files.desktop", "Files")] {
        std::fs::write(
            dir.join(file),
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n"),
        )
        .unwrap();
    }
    let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().dir(dir).build());
    app.config_path = Some(dir.join("compass.json"));
    app
}

fn send(app: &mut LauncherApp, message: SettingsMessage) {
    let task = app.update(Message::Settings(message));
    settle(app, task);
}

fn saved(app: &LauncherApp) -> compass_core::Config {
    compass_core::Config::load_from(app.config_path.as_ref().unwrap()).unwrap()
}

fn page(app: &LauncherApp) -> &SettingsPage {
    let Page::Settings(page) = &app.page else {
        panic!("not the settings: {}", app.state_line());
    };
    page
}

fn select(app: &mut LauncherApp, key: &str) {
    let row = page(app).sidebar.index_of_key(key);
    assert!(row >= 0, "no sidebar row {key}");
    send(app, SettingsMessage::SidebarSelected(row as usize));
}

#[test]
fn open_settings_is_a_root_command_and_ctrl_comma_and_escape_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.update(Message::QueryChanged("open settings".into()));
    let Some(crate::app::RootRow::Command(command)) = app.selected_row() else {
        panic!("no command: {}", app.state_line());
    };
    assert_eq!(command.id(), "commands:settings");
    let task = app.update(Message::LaunchSelected);
    settle(&mut app, task);
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::General));

    let _ = app.update(key(Key::Named(Named::Escape), Modifiers::empty()));
    assert!(matches!(app.page, Page::Root));
    let _ = app.update(Message::QueryChanged(String::new()));
    let _ = app.update(key(Key::Character(",".into()), Modifiers::CTRL));
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::General));

    // The arrows walk the sidebar, past the divider.
    for _ in 0..4 {
        let _ = app.update(key(Key::Named(Named::ArrowDown), Modifiers::empty()));
    }
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::About));
    let _ = app.update(key(Key::Named(Named::ArrowDown), Modifiers::empty()));
    assert!(matches!(page(&app).shown(), Shown::Provider(_)));

    // The search field filters it.
    let _ = app.update(Message::Settings(SettingsMessage::QueryChanged(
        "keyb".into(),
    )));
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::Keybindings));
}

#[test]
fn every_page_and_the_recorder_draw() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(None);
    let rows = page(&app).sidebar.rows().len();
    for row in 0..rows {
        send(&mut app, SettingsMessage::SidebarSelected(row));
        let _ = app.view();
    }
    send(
        &mut app,
        SettingsMessage::Record(RecordTarget::Setting("launcher.hotkey".into())),
    );
    let _ = app.view();
    send(&mut app, SettingsMessage::QueryChanged("zzzz".into()));
    let _ = app.view();
}

#[test]
fn a_root_rows_open_preferences_opens_the_settings_at_its_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.update(Message::QueryChanged("firefox".into()));
    let _ = app.update(Message::TogglePanel);
    let row = app
        .panel
        .as_ref()
        .and_then(|panel| panel.row_titled("Open preferences"))
        .expect("the panel offers the preferences");
    let task = app.update(Message::PanelClicked(row));
    settle(&mut app, task);
    assert!(app.panel.is_none());
    assert!(matches!(page(&app).shown(), Shown::Provider(p) if p.id == "applications"));
}

#[test]
fn a_deeplink_opens_the_tab_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_deeplink("vicinae://settings/open?tab=about");
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::About));
    let _ = app.open_deeplink("vicinae://settings/open?tab=shortcuts");
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::Keybindings));
    let _ = app.open_deeplink("vicinae://settings/open?tab=applications");
    assert!(matches!(page(&app).shown(), Shown::Provider(p) if p.id == "applications"));
}

#[test]
fn each_control_writes_the_file_and_the_launcher_follows_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    std::fs::write(dir.path().join("compass.json"), r#"{"mystery": 1}"#).unwrap();
    let _ = app.open_settings(Some("advanced"));

    send(
        &mut app,
        SettingsMessage::Changed("launcher.wrap_navigation".into(), json!(true)),
    );
    send(
        &mut app,
        SettingsMessage::Changed("launcher.keybinding".into(), json!("emacs")),
    );
    assert!(app.wrap_navigation);
    assert_eq!(app.keybinding, compass_core::keybinding::Scheme::Emacs);
    let config = saved(&app);
    assert!(config.launcher().wrap_navigation());
    assert_eq!(config.launcher().keybinding(), "emacs");
    assert_eq!(
        config.get_path("mystery"),
        Some(json!(1)),
        "the rest is kept"
    );

    // A number is typed, and refused until it is one.
    select(&mut app, "general");
    send(
        &mut app,
        SettingsMessage::DraftEdited("launcher.max_results".into(), "lots".into()),
    );
    send(
        &mut app,
        SettingsMessage::DraftSubmitted("launcher.max_results".into()),
    );
    assert!(page(&app).notice.is_some());
    assert_eq!(saved(&app).launcher().max_results(), 50);
    send(
        &mut app,
        SettingsMessage::DraftEdited("launcher.max_results".into(), "12".into()),
    );
    send(
        &mut app,
        SettingsMessage::DraftSubmitted("launcher.max_results".into()),
    );
    assert_eq!(saved(&app).launcher().max_results(), 12);
    assert!(page(&app).notice.is_none());

    // The clock goes at once.
    send(
        &mut app,
        SettingsMessage::Changed("launcher.clock.enabled".into(), json!(false)),
    );
    assert!(app.clock.is_none());

    // A builtin command's preference, under its row.
    send(
        &mut app,
        SettingsMessage::Changed(
            "providers.power.entrypoints.lock.preferences.confirm".into(),
            json!(true),
        ),
    );
    assert_eq!(app.power_asks.get("lock"), Some(&true));
}

#[test]
fn the_extension_page_switches_aliases_and_records_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(Some("applications"));
    let firefox = page(&app)
        .providers
        .iter()
        .flat_map(|provider| &provider.items)
        .find(|item| item.title == "Firefox")
        .expect("Firefox is listed")
        .id
        .clone();
    let enabled = |app: &LauncherApp| app.app_index.root(&firefox).unwrap().meta.enabled;
    let (provider, entrypoint) = compass_core::root_items::split_entrypoint_id(&firefox).unwrap();

    send(
        &mut app,
        SettingsMessage::ItemToggled(firefox.clone(), false),
    );
    assert!(!enabled(&app));
    assert_eq!(
        saved(&app).root_config().providers[provider].entrypoints[entrypoint].enabled,
        Some(false)
    );
    send(
        &mut app,
        SettingsMessage::ItemToggled(firefox.clone(), true),
    );
    assert!(enabled(&app));

    send(
        &mut app,
        SettingsMessage::ProviderToggled("applications".into(), false),
    );
    assert!(!enabled(&app), "the provider's switch covers its items");
    assert_eq!(
        saved(&app).root_config().providers["applications"].enabled,
        Some(false)
    );
    let row = page(&app)
        .sidebar
        .rows()
        .iter()
        .find(|row| row.key == "applications")
        .unwrap();
    assert!(!row.enabled, "the sidebar greys it");
    send(
        &mut app,
        SettingsMessage::ProviderToggled("applications".into(), true),
    );
    assert!(enabled(&app));

    send(
        &mut app,
        SettingsMessage::DraftEdited(alias_key(&firefox), "ff".into()),
    );
    send(
        &mut app,
        SettingsMessage::DraftSubmitted(alias_key(&firefox)),
    );
    assert_eq!(
        app.app_index.root(&firefox).unwrap().meta.alias.as_deref(),
        Some("ff")
    );
    assert_eq!(
        saved(&app).root_config().providers[provider].entrypoints[entrypoint]
            .alias
            .as_deref(),
        Some("ff")
    );

    send(
        &mut app,
        SettingsMessage::Record(RecordTarget::Item(firefox.clone())),
    );
    assert!(page(&app).recorder.is_some());
    let _ = app.update(key(Key::Named(Named::Control), Modifiers::CTRL));
    let task = app.update(key(Key::Character("j".into()), Modifiers::CTRL));
    settle(&mut app, task);
    assert!(
        page(&app).recorder.is_none(),
        "an accepted shortcut ends it"
    );
    assert_eq!(
        app.app_index
            .root(&firefox)
            .unwrap()
            .meta
            .shortcut
            .as_deref(),
        Some("control+J")
    );
    assert_eq!(
        saved(&app).root_config().providers[provider].entrypoints[entrypoint]
            .shortcut
            .as_deref(),
        Some("control+J")
    );
}

#[test]
fn the_hotkey_is_recorded_into_the_launcher_section() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(None);
    send(
        &mut app,
        SettingsMessage::Record(RecordTarget::Setting("launcher.hotkey".into())),
    );
    assert!(
        app.shortcuts_inhibited(),
        "the compositor's shortcuts reach the recorder"
    );
    let _ = app.update(key(Key::Named(Named::Alt), Modifiers::ALT));
    let task = app.update(key(Key::Named(Named::Space), Modifiers::ALT));
    settle(&mut app, task);
    assert!(page(&app).recorder.is_none());
    assert!(!app.shortcuts_inhibited(), "and are given back after");
    let hotkey = saved(&app).launcher().hotkey().to_owned();
    assert!(
        compass_core::key_combo::KeyCombo::parse(&hotkey).is_some(),
        "{hotkey}"
    );
    assert_ne!(hotkey, compass_core::config::DEFAULT_HOTKEY);
}

#[derive(Debug, Default)]
struct Engine {
    settings: Mutex<Vec<(String, serde_json::Value)>>,
    providers: Mutex<Vec<(String, bool)>>,
    refuse: bool,
    /// The login start changes asked for.
    login: Mutex<Vec<bool>>,
}

impl crate::backend::ApplicationBackend for Engine {
    fn search(&self, _query: String) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn record_launch(&self, _key: String) -> BackendFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn edit_root_item(&self, _id: String, _edit: RootEdit) -> BackendFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn set_setting(&self, key: String, value: serde_json::Value) -> BackendFuture<'_, ()> {
        self.settings.lock().unwrap().push((key, value));
        let refuse = self.refuse;
        Box::pin(async move {
            if refuse {
                Err("unknown theme".to_owned())
            } else {
                Ok(())
            }
        })
    }
    fn set_provider_enabled(&self, provider: String, enabled: bool) -> BackendFuture<'_, ()> {
        self.providers.lock().unwrap().push((provider, enabled));
        Box::pin(async { Ok(()) })
    }
    fn extension_preferences(
        &self,
        id: String,
    ) -> BackendFuture<'_, crate::backend::ExtensionStart> {
        let refuse = self.refuse;
        Box::pin(async move {
            if refuse {
                return Err("no keyring".to_owned());
            }
            Ok(crate::backend::ExtensionStart::NeedsPreferences {
                title: id,
                fields: Vec::new(),
            })
        })
    }
    fn login_start(
        &self,
        enable: Option<bool>,
    ) -> BackendFuture<'_, compass_core::login_start::Status> {
        use compass_core::login_start::{Mechanism, State, Status, Unit};
        let mut asked = self.login.lock().unwrap();
        if let Some(on) = enable {
            asked.push(on);
        }
        let on = asked.last().copied().unwrap_or(false);
        Box::pin(async move {
            Ok(Status {
                state: if on {
                    State::On {
                        mechanism: Mechanism::AutostartFile,
                    }
                } else {
                    State::Off
                },
                unit: Some(Unit::Installed),
            })
        })
    }
}

#[test]
fn general_turns_login_start_on_and_off_through_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let engine = Arc::new(Engine::default());
    app.backend = Some(engine.clone());
    let task = app.open_settings(None);
    settle(&mut app, task);
    let status = app
        .login_start
        .clone()
        .expect("read when the settings open");
    assert!(!status.state.is_on(), "off by default");
    assert!(
        engine.login.lock().unwrap().is_empty(),
        "reading changes nothing"
    );
    assert_eq!(page(&app).controls(None).last(), Some(&Control::LoginStart));

    tab_to(&mut app, &Control::LoginStart);
    press(&mut app, Named::Space, Modifiers::empty());
    assert!(app.login_start.as_ref().unwrap().state.is_on());
    press(&mut app, Named::Enter, Modifiers::empty());
    assert!(!app.login_start.as_ref().unwrap().state.is_on());
    assert_eq!(engine.login.lock().unwrap().as_slice(), [true, false]);

    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(800.0, 600.0),
        app.view(),
    );
    let described = app.login_start.as_ref().unwrap().describe();
    assert!(ui.find("Start at login").is_ok());
    assert!(ui.find(described.as_str()).is_ok(), "{described}");
}

#[test]
fn a_commands_preferences_open_over_the_settings_and_go_back_to_them() {
    for refuse in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.apply(AppFlags {
            backend: Some(Arc::new(Engine {
                refuse,
                ..Engine::default()
            })),
            ..AppFlags::default()
        });
        let _ = app.open_settings(Some("applications"));
        send(
            &mut app,
            SettingsMessage::OpenPreferences("@me/notes:list".into()),
        );
        if refuse {
            assert_eq!(page(&app).notice.as_deref(), Some("no keyring"));
            continue;
        }
        assert!(
            matches!(&app.page, Page::Preferences(form)
                if form.purpose == crate::preferences_page::Purpose::CommandPreferences),
            "{}",
            app.state_line()
        );
        let task = app.update(Message::Back);
        settle(&mut app, task);
        assert!(
            matches!(page(&app).shown(), Shown::Provider(p) if p.id == "applications"),
            "back where it was"
        );
    }
}

#[test]
fn with_an_engine_the_engine_writes_and_a_theme_is_kept_or_put_back() {
    for refuse in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(Engine {
            refuse,
            ..Engine::default()
        });
        let mut app = app(dir.path());
        app.apply(AppFlags {
            backend: Some(engine.clone()),
            ..AppFlags::default()
        });
        app.config_path = Some(dir.path().join("compass.json"));
        let _ = app.open_settings(Some("appearance"));
        send(
            &mut app,
            SettingsMessage::Changed("launcher.appearance.theme".into(), json!("nord")),
        );
        assert_eq!(
            engine.settings.lock().unwrap().as_slice(),
            &[("launcher.appearance.theme".to_owned(), json!("nord"))]
        );
        if refuse {
            assert_eq!(
                app.view.theme_choice,
                crate::theme::Theme::System,
                "put back"
            );
            assert_eq!(page(&app).notice.as_deref(), Some("unknown theme"));
        } else {
            assert_eq!(app.view.theme_choice, crate::theme::Theme::Nord);
            assert!(app.view.theme_preview.is_none(), "kept");
        }
        send(
            &mut app,
            SettingsMessage::ProviderToggled("applications".into(), false),
        );
        assert_eq!(
            engine.providers.lock().unwrap().as_slice(),
            &[("applications".to_owned(), false)]
        );
        assert!(
            !dir.path().join("compass.json").exists(),
            "the engine writes the file, not the window"
        );
    }
}

fn press(app: &mut LauncherApp, named: Named, modifiers: Modifiers) {
    let task = app.update(key(Key::Named(named), modifiers));
    settle(app, task);
}

/// Tabs until `target` has the keyboard, failing if it never does.
fn tab_to(app: &mut LauncherApp, target: &Control) {
    for _ in 0..200 {
        if page(app).focused.as_ref() == Some(target) {
            return;
        }
        press(app, Named::Tab, Modifiers::empty());
    }
    panic!("Tab never reached {target:?}");
}

#[test]
fn tab_reaches_every_control_on_every_page_in_order_and_comes_back_to_search() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(None);
    let rows: Vec<(usize, String)> = page(&app)
        .sidebar
        .rows()
        .iter()
        .enumerate()
        .filter(|(_, row)| row.selectable())
        .map(|(at, row)| (at, row.key.clone()))
        .collect();
    let mut kinds = std::collections::BTreeSet::new();
    for (row, key) in rows {
        send(&mut app, SettingsMessage::SidebarSelected(row));
        let controls = page(&app).controls(None);
        let mut reached = Vec::new();
        for _ in 0..controls.len() {
            press(&mut app, Named::Tab, Modifiers::empty());
            reached.push(page(&app).focused.clone().expect("a control"));
        }
        assert_eq!(reached, controls, "{key}: Tab walks the controls as drawn");
        press(&mut app, Named::Tab, Modifiers::empty());
        assert_eq!(page(&app).focused, None, "{key}: then the search field");
        if let Some(last) = controls.last() {
            press(&mut app, Named::Tab, Modifiers::SHIFT);
            assert_eq!(page(&app).focused.as_ref(), Some(last), "{key}: Shift+Tab");
            press(&mut app, Named::Escape, Modifiers::empty());
            assert_eq!(page(&app).focused, None, "{key}: Escape gives it back");
        }
        for control in controls {
            kinds.insert(match control {
                Control::Setting(key) => match settings_catalog::find(&key).unwrap().kind {
                    Kind::Toggle => "switch",
                    Kind::Choice(_) | Kind::Theme => "dropdown",
                    Kind::Shortcut => "hotkey",
                    _ => "field",
                },
                Control::ProviderSwitch(_) | Control::ItemSwitch(_) | Control::LoginStart => {
                    "switch"
                }
                Control::ItemAlias(_) => "field",
                Control::ItemShortcut(_) => "record shortcut",
                Control::ItemPreferences(_) => "preferences",
                Control::Link(_) => "link",
            });
        }
    }
    for kind in [
        "switch",
        "dropdown",
        "hotkey",
        "field",
        "record shortcut",
        "link",
    ] {
        assert!(kinds.contains(kind), "no {kind} was reached: {kinds:?}");
    }
}

#[test]
fn space_and_enter_work_each_kind_of_control() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(None);

    // A switch.
    let close = Control::Setting("launcher.close_on_focus_loss".into());
    tab_to(&mut app, &close);
    let before = saved_or_default(&app).launcher().close_on_focus_loss();
    press(&mut app, Named::Space, Modifiers::empty());
    assert_eq!(saved(&app).launcher().close_on_focus_loss(), !before);
    press(&mut app, Named::Enter, Modifiers::empty());
    assert_eq!(saved(&app).launcher().close_on_focus_loss(), before);

    // The hotkey's button starts the recorder.
    press(&mut app, Named::Escape, Modifiers::empty());
    tab_to(&mut app, &Control::Setting("launcher.hotkey".into()));
    press(&mut app, Named::Enter, Modifiers::empty());
    assert!(page(&app).recorder.is_some());
    press(&mut app, Named::Escape, Modifiers::empty());
    assert!(page(&app).recorder.is_none());
    assert_eq!(
        page(&app).focused,
        Some(Control::Setting("launcher.hotkey".into())),
        "the keyboard is back on the button"
    );

    // A dropdown opens, the arrows move through it, Enter picks.
    select(&mut app, "appearance");
    let scheme = Control::Setting("launcher.appearance.color_scheme".into());
    tab_to(&mut app, &scheme);
    press(&mut app, Named::Space, Modifiers::empty());
    let opened = page(&app).menu.expect("the dropdown opened");
    let (_, options) = page(&app).focused_choice().unwrap();
    let wanted = if opened + 1 < options.len() {
        press(&mut app, Named::ArrowDown, Modifiers::empty());
        opened + 1
    } else {
        press(&mut app, Named::ArrowUp, Modifiers::empty());
        opened - 1
    };
    assert_eq!(page(&app).menu, Some(wanted));
    press(&mut app, Named::Enter, Modifiers::empty());
    assert_eq!(page(&app).menu, None);
    assert_eq!(
        saved(&app).launcher().appearance().color_scheme(),
        options[wanted].0
    );
    // Escape closes an open dropdown without choosing.
    press(&mut app, Named::Enter, Modifiers::empty());
    assert!(page(&app).menu.is_some());
    press(&mut app, Named::Escape, Modifiers::empty());
    assert_eq!(page(&app).menu, None);
    assert_eq!(page(&app).focused.as_ref(), Some(&scheme));

    // A provider's switch and an item's.
    select(&mut app, "applications");
    tab_to(&mut app, &Control::ProviderSwitch("applications".into()));
    press(&mut app, Named::Space, Modifiers::empty());
    assert!(
        !page(&app)
            .providers
            .iter()
            .any(|p| p.id == "applications" && p.enabled)
    );
    let item = page(&app)
        .providers
        .iter()
        .find(|p| p.id == "applications")
        .unwrap()
        .items[0]
        .id
        .clone();
    tab_to(&mut app, &Control::ItemSwitch(item.clone()));
    press(&mut app, Named::Space, Modifiers::empty());
    let item_on = |app: &LauncherApp| {
        page(app)
            .providers
            .iter()
            .flat_map(|p| &p.items)
            .find(|i| i.id == item)
            .unwrap()
            .enabled
    };
    assert!(!item_on(&app));
    tab_to(&mut app, &Control::ItemShortcut(item.clone()));
    press(&mut app, Named::Space, Modifiers::empty());
    assert!(page(&app).recorder.is_some());
    press(&mut app, Named::Escape, Modifiers::empty());

    // Escape, with nothing focused, leaves.
    press(&mut app, Named::Escape, Modifiers::empty());
    press(&mut app, Named::Escape, Modifiers::empty());
    assert!(matches!(app.page, Page::Root));
}

#[test]
fn page_keys_and_space_in_the_search_field_do_not_change_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(None);
    for named in [
        Named::PageDown,
        Named::PageUp,
        Named::Home,
        Named::End,
        Named::Space,
    ] {
        press(&mut app, named, Modifiers::empty());
        assert_eq!(page(&app).focused, None);
        assert_eq!(page(&app).shown(), Shown::Core(CorePage::General));
    }
    assert!(
        !dir.path().join("compass.json").exists(),
        "nothing was written"
    );
}

fn saved_or_default(app: &LauncherApp) -> compass_core::Config {
    compass_core::Config::load_from(app.config_path.as_ref().unwrap()).unwrap_or_default()
}

#[test]
fn a_refused_write_puts_the_switch_back_and_says_why_in_the_error_colour() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    // Nothing can be written under a path whose parent is a file.
    std::fs::write(dir.path().join("blocker"), "").unwrap();
    app.config_path = Some(dir.path().join("blocker").join("compass.json"));
    let _ = app.open_settings(None);
    let setting = settings_catalog::find("launcher.close_on_focus_loss").unwrap();
    assert_eq!(page(&app).value(&setting), json!(false));

    send(
        &mut app,
        SettingsMessage::Changed("launcher.close_on_focus_loss".into(), json!(true)),
    );
    assert_eq!(
        page(&app).value(&setting),
        json!(false),
        "the switch shows what the file still says"
    );
    assert!(
        page(&app)
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("could not save")),
        "{:?}",
        page(&app).notice
    );
    let palette = app.palette();
    assert_ne!(palette.error(), palette.accent, "not drawn as a link");
}

#[test]
fn a_file_with_a_typo_opens_with_the_problem_named() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    std::fs::write(
        dir.path().join("compass.json"),
        r#"{"launcher": {"close_on_focus_los": true, "max_results": 7}}"#,
    )
    .unwrap();
    let _ = app.open_settings(None);
    let notice = page(&app).notice.clone().unwrap_or_default();
    assert!(
        notice.contains("Did you mean launcher.close_on_focus_loss?"),
        "{notice}"
    );
    let max = settings_catalog::find("launcher.max_results").unwrap();
    assert_eq!(
        page(&app).value(&max),
        json!(7),
        "the rest of the file is read"
    );
}

#[test]
fn a_reloaded_file_reaches_the_window_and_an_open_settings_view() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(None);
    let config = compass_core::Config::parse(
        r#"{"launcher": {"wrap_navigation": true, "keybinding": "emacs",
            "appearance": {"theme": "dracula", "preset": "rofi"}}}"#,
        std::path::Path::new("compass.json"),
    )
    .unwrap();
    let task = app.update(Message::ConfigReloaded(Arc::new(config)));
    settle(&mut app, task);
    assert!(app.wrap_navigation);
    assert_eq!(app.keybinding, compass_core::keybinding::Scheme::Emacs);
    assert_eq!(app.view.theme_choice, crate::theme::Theme::Dracula);
    let wrap = settings_catalog::find("launcher.wrap_navigation").unwrap();
    assert_eq!(page(&app).value(&wrap), json!(true));
}

#[test]
fn clear_the_search_on_close_empties_the_query_only_when_it_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.update(Message::QueryChanged("fire".into()));
    let _ = app.conceal();
    assert_eq!(app.query, "fire", "off, as in Vicinae: the search is kept");

    let config = compass_core::Config::parse(
        r#"{"launcher": {"pop_to_root_on_close": true}}"#,
        std::path::Path::new("compass.json"),
    )
    .unwrap();
    let _ = app.update(Message::ConfigReloaded(Arc::new(config)));
    let _ = app.conceal();
    assert_eq!(app.query, "", "on: the next summon starts empty");
}

#[test]
fn an_item_the_search_lists_is_not_offered_again_as_a_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    app.fallbacks = vec![compass_core::commands::SEARCH_FILES_FALLBACK_ID.to_owned()];
    let _ = app.update(Message::QueryChanged("docs".into()));
    let search_files = app
        .results
        .iter()
        .filter(|row| match row {
            crate::app::RootRow::Command(command) => command.entrypoint == "search-files",
            crate::app::RootRow::Fallback(crate::app::Fallback::Command(command)) => {
                command.entrypoint == "search-files"
            }
            _ => false,
        })
        .count();
    assert_eq!(search_files, 1, "{}", app.state_line());

    // A query that does not find it still offers it as the fallback.
    let _ = app.update(Message::QueryChanged("quarterly".into()));
    assert!(
        app.results
            .iter()
            .any(|row| matches!(row, crate::app::RootRow::Fallback(_)))
    );
}

#[test]
fn hibernate_does_not_answer_store() {
    let hibernate = compass_core::power_commands::command("hibernate").unwrap();
    assert!(!hibernate.description.to_lowercase().contains("store"));
}

#[test]
fn the_hotkey_guidance_names_the_compositor_line_and_the_flatpak_command() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    assert!(
        app.open_hint()
            .starts_with("Press Super+Space to open Compass"),
        "{}",
        app.open_hint()
    );
    app.hotkey_compositor = Some(compass_core::hotkey_guide::Compositor::Sway);
    app.flatpak = true;
    assert!(
        app.open_hint()
            .contains("`bindsym $mod+space exec flatpak run org.tunaos.compass toggle`"),
        "{}",
        app.open_hint()
    );
    // The settings view draws the row as guidance, not as a recorder.
    let _ = app.open_settings(None);
    let _ = app.view();
}
