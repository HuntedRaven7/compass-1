//! Fixtures shared by the launcher's tests: an index over a tempdir, the
//! recording fakes of the engine's backends, and the key presses and clicks
//! the tests drive the state machine with.
//!
//! Each test sits beside the module it tests; a fixture lives here only
//! when tests in more than one module use it.

use crate::app::flags::AppFlags;
use crate::app::panel::PanelState;
use crate::app::{LauncherApp, Page, RootRow};
use crate::message::Message;
use crate::resident::EngineLink;
use compass_core::AppIndex;
use compass_platform::AppLauncher;
use iced::{Task, window};
use std::fs;
use std::sync::Arc;

mod backend;

pub(crate) use backend::TestBackend;

/// An index over three entries written to a tempdir, so the tests do not
/// depend on what the machine running them has installed.
pub(crate) fn index(dir: &std::path::Path) -> AppIndex {
    for (file, name) in [
        ("firefox.desktop", "Firefox"),
        ("files.desktop", "Files"),
        ("terminal.desktop", "Terminal"),
    ] {
        fs::write(
            dir.join(file),
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n"),
        )
        .expect("write entry");
    }
    AppIndex::builder().dir(dir).build()
}

pub(crate) fn app(dir: &std::path::Path) -> LauncherApp {
    LauncherApp::with_index(index(dir))
}

#[derive(Debug, Default)]
pub(crate) struct RecordingLaunchTarget(pub(crate) std::sync::Mutex<Vec<Option<String>>>);

impl AppLauncher for RecordingLaunchTarget {
    fn launch<'a>(
        &'a self,
        _entry: &'a compass_xdg::DesktopEntry,
        _uris: &'a [&'a str],
    ) -> compass_platform::LaunchFuture<'a> {
        Box::pin(async move {
            self.0.lock().unwrap().push(None);
            Ok(compass_platform::LaunchMethod::Direct)
        })
    }

    fn launch_action<'a>(
        &'a self,
        _entry: &'a compass_xdg::DesktopEntry,
        action_id: &'a str,
        _uris: &'a [&'a str],
    ) -> compass_platform::LaunchFuture<'a> {
        Box::pin(async move {
            self.0.lock().unwrap().push(Some(action_id.to_owned()));
            Ok(compass_platform::LaunchMethod::Direct)
        })
    }
}

/// What clicking the text `label` in the current view produces.
pub(crate) fn click(app: &LauncherApp, label: &str) -> Vec<Message> {
    let mut ui = iced_test::simulator(app.view());
    ui.click(label)
        .unwrap_or_else(|_| panic!("{label:?} is not on screen: {}", app.state_line()));
    ui.into_messages().collect()
}

#[derive(Debug, Default)]
pub(crate) struct LaunchedEntries(pub(crate) std::sync::Mutex<Vec<String>>);

impl AppLauncher for LaunchedEntries {
    fn launch<'a>(
        &'a self,
        entry: &'a compass_xdg::DesktopEntry,
        _uris: &'a [&'a str],
    ) -> compass_platform::LaunchFuture<'a> {
        Box::pin(async move {
            self.0.lock().unwrap().push(entry.name().to_owned());
            Ok(compass_platform::LaunchMethod::Direct)
        })
    }

    fn launch_action<'a>(
        &'a self,
        entry: &'a compass_xdg::DesktopEntry,
        _action_id: &'a str,
        uris: &'a [&'a str],
    ) -> compass_platform::LaunchFuture<'a> {
        self.launch(entry, uris)
    }
}

pub(crate) fn extension_app(dir: &std::path::Path, backend: Arc<TestBackend>) -> LauncherApp {
    let ext = dir.join("extensions/hello");
    fs::create_dir_all(&ext).unwrap();
    fs::write(
        ext.join("package.json"),
        r#"{"name": "hello", "title": "Hello", "author": "someone",
                "commands": [{"name": "write", "title": "Write Greeting", "mode": "no-view"}]}"#,
    )
    .unwrap();
    index(dir);
    let index = AppIndex::builder()
        .dir(dir)
        .extension_dirs([dir.join("extensions")])
        .build();
    let mut app = LauncherApp::with_index(index);
    app.backend = Some(backend);
    app
}

/// Turns on the default-disabled Compass commands named.
pub(crate) fn enable_commands(app: &mut LauncherApp, entrypoints: &[&str]) {
    let entries: Vec<String> = entrypoints
        .iter()
        .map(|id| format!("\"{id}\":{{\"enabled\":true}}"))
        .collect();
    let config = compass_core::Config::parse(
        &format!(
            "{{\"providers\":{{\"commands\":{{\"entrypoints\":{{{}}}}}}}}}",
            entries.join(",")
        ),
        std::path::Path::new("config.json"),
    )
    .unwrap();
    app.root_config = config.root_config();
    app.app_index.apply_root_config(&app.root_config);
}

pub(crate) fn greeting_list(host_filtering: bool) -> compass_extension_api::View {
    use compass_extension_api::action::{Action, ActionPanel};
    use compass_extension_api::view::{ListItem, ListSection, ListView};
    let item = |title: &str, handler: &str| {
        ListItem::new(title)
            .with_key(title)
            .with_actions(ActionPanel::of([Action::new("Act", handler)]))
    };
    let mut list = ListView {
        sections: vec![ListSection::untitled([
            item("hello", "cb-hello"),
            item("goodbye", "cb-goodbye"),
        ])],
        ..ListView::default()
    };
    list.search.host_filtering = host_filtering;
    if !host_filtering {
        list.search.on_change = Some(compass_extension_api::action::HandlerId::new("cb-search"));
    }
    compass_extension_api::View::List(list)
}

pub(crate) fn task_messages(task: Task<Message>) -> Vec<Message> {
    use iced::futures::{StreamExt, executor::block_on};
    let Some(stream) = iced_winit::runtime::task::into_stream(task) else {
        return Vec::new();
    };
    block_on(
        stream
            .filter_map(|action| async move {
                match action {
                    iced_winit::runtime::Action::Output(message) => Some(message),
                    _ => None,
                }
            })
            .collect(),
    )
}

/// Themes compare by value, and a failed comparison prints two whole
/// palettes. The name is what distinguishes ours, so assert on that.
pub(crate) fn theme_name(app: &LauncherApp) -> String {
    app.theme().to_string()
}

pub(crate) fn pressed(key: iced::keyboard::key::Named) -> Message {
    Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Named(key),
        modified_key: iced::keyboard::Key::Named(key),
        physical_key: iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        ),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::default(),
        text: None,
        repeat: false,
    })
}

/// A `KeyPressed` for a character key with modifiers.
pub(crate) fn chord(character: &str, modifiers: iced::keyboard::Modifiers) -> Message {
    Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Character(character.into()),
        modified_key: iced::keyboard::Key::Character(character.into()),
        physical_key: iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        ),
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    })
}

/// An app over the fixture corpus with several matching rows.
pub(crate) fn app_with_rows(dir: &std::path::Path) -> LauncherApp {
    let mut app = app(dir);
    let _ = app.update(Message::QueryChanged("fi".to_owned()));
    assert!(app.results.len() > 1, "need several rows to move between");
    app
}

pub(crate) fn key_event(
    pressed: bool,
    key: iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Message {
    let physical =
        iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified);
    Message::Keyboard(if pressed {
        iced::keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: physical,
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    } else {
        iced::keyboard::Event::KeyReleased {
            key: key.clone(),
            modified_key: key,
            physical_key: physical,
            location: iced::keyboard::Location::Standard,
            modifiers,
        }
    })
}

/// Whether `task` asks for the window `id` to close.
pub(crate) fn closes(task: Task<Message>, id: window::Id) -> bool {
    use iced::futures::{StreamExt, executor::block_on};
    use iced_winit::runtime::{Action, task, window as runtime_window};
    let Some(stream) = task::into_stream(task) else {
        return false;
    };
    let actions: Vec<_> = block_on(stream.collect());
    actions.iter().any(|action| {
        matches!(action, Action::Window(runtime_window::Action::Close(closed)) if *closed == id)
    })
}

/// A launcher on screen, attached to an engine.
/// The engine's ends of the link come back with it, to keep it open.
pub(crate) fn shown(dir: &std::path::Path) -> (LauncherApp, window::Id, Box<dyn std::any::Any>) {
    let mut app = app(dir);
    let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
    let (sender, outcomes) = tokio::sync::mpsc::unbounded_channel();
    app.link = Some(EngineLink::new(receiver, sender));
    let _ = app.open_window();
    let id = app.pending_window.unwrap();
    let _ = app.update(Message::Opened(id));
    assert!(app.is_visible());
    (app, id, Box::new((commands, outcomes)))
}

pub(crate) fn file_row(path: &str, category: &str) -> crate::backend::FileRow {
    crate::backend::FileRow {
        path: path.into(),
        name: path.rsplit('/').next().unwrap_or(path).into(),
        category: category.into(),
    }
}

pub(crate) fn files_page(app: &LauncherApp) -> &crate::files_page::FilesPage {
    let Page::Files(page) = &app.page else {
        panic!("not on Search Files: {}", app.state_line())
    };
    page
}

pub(crate) fn stored_shortcut(id: &str, name: &str, url: &str) -> crate::backend::Shortcut {
    crate::backend::Shortcut {
        id: id.into(),
        name: name.into(),
        icon: "icon://omnicast/link".into(),
        url: url.into(),
        app: "default".into(),
        ..crate::backend::Shortcut::default()
    }
}

/// An app whose engine holds two shortcuts, already listed.
pub(crate) fn shortcuts_app(dir: &std::path::Path) -> (LauncherApp, Arc<TestBackend>) {
    let backend = Arc::new(TestBackend {
        shortcuts: std::sync::Mutex::new(vec![
            stored_shortcut("sct-docs", "Crate Docs", "https://docs.rs/{crate}"),
            stored_shortcut("sct-news", "Hacker News", "https://news.ycombinator.com"),
        ]),
        ..TestBackend::default()
    });
    let mut app = LauncherApp::with_index(index(dir));
    app.backend = Some(backend.clone());
    let task = app.refresh_shortcuts_task();
    settle(&mut app, task);
    assert_eq!(app.app_index.shortcuts().len(), 2);
    (app, backend)
}

pub(crate) fn open_builtin(app: &mut LauncherApp, query: &str, id: &str) {
    app.query = query.into();
    app.search();
    let Some(RootRow::Command(command)) = app.selected_row() else {
        panic!("{query:?} did not select a command: {}", app.state_line());
    };
    assert_eq!(command.id(), id);
    let task = app.update(Message::LaunchSelected);
    settle(app, task);
}

/// How many actions other than messages a task carries: the widget
/// operations, such as focusing a field.
pub(crate) fn task_actions(task: Task<Message>) -> usize {
    use iced::futures::{StreamExt, executor::block_on};
    let Some(stream) = iced_winit::runtime::task::into_stream(task) else {
        return 0;
    };
    block_on(stream.collect::<Vec<_>>())
        .into_iter()
        .filter(|action| matches!(action, iced_winit::runtime::Action::Widget(_)))
        .count()
}

pub(crate) fn opener(id: &str, name: &str, default: bool) -> crate::backend::OpenerRow {
    crate::backend::OpenerRow {
        id: id.into(),
        name: name.into(),
        icon: None,
        default,
    }
}

pub(crate) fn store_backend(dir: &std::path::Path) -> Arc<TestBackend> {
    let row = |name: &str, title: &str| crate::backend::StoreRow {
        id: format!("store.vicinae.{name}"),
        name: name.into(),
        author: "zoe".into(),
        // Filed under an organisation, as Raycast's own extensions are.
        owner: "clockworks".into(),
        title: title.into(),
        description: format!("{title} does things"),
        downloads: "12".into(),
        ..crate::backend::StoreRow::default()
    };
    Arc::new(TestBackend {
        store_rows: std::sync::Mutex::new(vec![row("clock", "Clock"), row("timer", "Timer")]),
        store_dir: Some(dir.to_path_buf()),
        ..TestBackend::default()
    })
}

pub(crate) fn script_item(
    id: &str,
    title: &str,
    mode: compass_core::script_command::OutputMode,
    arguments: usize,
) -> compass_core::script_scan::ScriptItem {
    compass_core::script_scan::ScriptItem {
        id: id.into(),
        title: title.into(),
        subtitle: "scripts".into(),
        keywords: vec![],
        mode,
        needs_confirmation: false,
        arguments: (0..arguments)
            .map(|n| compass_core::script_command::ScriptArgument {
                argument_type: compass_core::script_command::ArgumentType::Text,
                placeholder: Some(format!("arg{n}")),
                optional: false,
                percent_encoded: false,
                data: None,
            })
            .collect(),
        path: format!("/scripts/{id}"),
    }
}

pub(crate) fn scripts_app(dir: &std::path::Path) -> (LauncherApp, Arc<TestBackend>) {
    use compass_core::script_command::OutputMode;
    let backend = Arc::new(TestBackend {
        scripts: vec![
            script_item("report.sh", "Disk Report", OutputMode::Full, 1),
            script_item("count.sh", "Count Things", OutputMode::Compact, 0),
            script_item("touch.sh", "Touch Marker", OutputMode::Silent, 0),
        ],
        ..TestBackend::default()
    });
    let mut app = LauncherApp::with_index(index(dir));
    app.backend = Some(backend.clone());
    let task = app.refresh_scripts_task();
    settle(&mut app, task);
    assert_eq!(app.app_index.scripts().len(), 3);
    (app, backend)
}

pub(crate) fn stored_snippet(
    id: &str,
    name: &str,
    text: &str,
    keyword: Option<&str>,
) -> crate::backend::Snippet {
    crate::backend::Snippet {
        id: id.into(),
        name: name.into(),
        data: compass_core::snippet_store::SnippetData::Text { text: text.into() },
        expansion: keyword.map(|keyword| compass_core::snippet_store::StoredExpansion {
            keyword: keyword.into(),
            apps: vec!["org.gnome.TextEditor.desktop".into()],
            word: true,
        }),
        ..crate::backend::Snippet::default()
    }
}

pub(crate) fn ctrl_enter() -> Message {
    Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
        modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
        physical_key: iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        ),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::CTRL,
        text: None,
        repeat: false,
    })
}

#[derive(Debug, Default)]
pub(crate) struct FakeClipboard {
    pub(crate) rows: Vec<crate::backend::ClipboardRow>,
    pub(crate) content: Option<crate::backend::ClipboardContent>,
    pub(crate) queries: std::sync::Mutex<Vec<String>>,
    /// Whether the engine can paste (it has the Shell extension).
    pub(crate) can_paste: bool,
    pub(crate) pasted: std::sync::Mutex<Vec<String>>,
    pub(crate) changes: std::sync::Mutex<Vec<String>>,
    pub(crate) fail_changes: bool,
    /// Each entry's keywords, as set.
    pub(crate) keywords: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    /// Whether copies are being recorded; `None` is never asked.
    pub(crate) monitoring: std::sync::Mutex<Option<bool>>,
    /// The kinds history was asked for.
    pub(crate) kinds: std::sync::Mutex<Vec<Option<crate::backend::ClipboardRowKind>>>,
}

impl crate::backend::ClipboardBackend for FakeClipboard {
    fn clipboard_history_of_kind(
        &self,
        query: String,
        _limit: u32,
        kind: Option<crate::backend::ClipboardRowKind>,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::ClipboardRow>> {
        Box::pin(async move {
            self.queries.lock().unwrap().push(query.clone());
            self.kinds.lock().unwrap().push(kind);
            Ok(self
                .rows
                .iter()
                .filter(|row| row.preview.contains(&query))
                .filter(|row| kind.is_none_or(|kind| row.kind == kind))
                .cloned()
                .collect())
        })
    }

    fn clipboard_detail(
        &self,
        id: String,
    ) -> crate::backend::BackendFuture<'_, crate::backend::ClipboardDetail> {
        Box::pin(async move {
            let row = self
                .rows
                .iter()
                .find(|row| row.id == id)
                .ok_or_else(|| "gone".to_owned())?;
            Ok(crate::backend::ClipboardDetail {
                id: id.clone(),
                mime_type: "text/plain".into(),
                kind: row.kind,
                size: 1536,
                md5: "abc".into(),
                updated_at: 1_700_000_000_000,
                encrypted: true,
                keywords: self
                    .keywords
                    .lock()
                    .unwrap()
                    .get(&id)
                    .cloned()
                    .unwrap_or_default(),
            })
        })
    }

    fn clipboard_set_keywords(
        &self,
        id: String,
        keywords: String,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.changes
                .lock()
                .unwrap()
                .push(format!("keywords {id} {keywords}"));
            self.keywords.lock().unwrap().insert(id, keywords);
            Ok(())
        })
    }

    fn clipboard_remove_all(&self) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.changes.lock().unwrap().push("remove-all".to_owned());
            Ok(())
        })
    }

    fn clipboard_monitoring(
        &self,
        enabled: Option<bool>,
    ) -> crate::backend::BackendFuture<'_, crate::backend::ClipboardMonitoring> {
        Box::pin(async move {
            let mut state = self.monitoring.lock().unwrap();
            if let Some(enabled) = enabled {
                *state = Some(enabled);
            }
            Ok(crate::backend::ClipboardMonitoring {
                supported: true,
                enabled: state.unwrap_or(true),
            })
        })
    }

    fn clipboard_history(
        &self,
        query: String,
        _limit: u32,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::ClipboardRow>> {
        Box::pin(async move {
            self.queries.lock().unwrap().push(query.clone());
            Ok(self
                .rows
                .iter()
                .filter(|row| row.preview.contains(&query))
                .cloned()
                .collect())
        })
    }

    fn clipboard_content(
        &self,
        _id: String,
    ) -> crate::backend::BackendFuture<'_, crate::backend::ClipboardContent> {
        Box::pin(async move { self.content.clone().ok_or_else(|| "gone".to_owned()) })
    }

    fn clipboard_set_pinned(
        &self,
        id: String,
        pinned: bool,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.changes
                .lock()
                .unwrap()
                .push(format!("pin {id} {pinned}"));
            Ok(())
        })
    }

    fn clipboard_remove(&self, id: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.changes.lock().unwrap().push(format!("remove {id}"));
            if self.fail_changes {
                return Err("That entry is already gone".to_owned());
            }
            Ok(())
        })
    }

    fn clipboard_paste(&self, id: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            if !self.can_paste {
                return Err("Pasting needs the Compass GNOME Shell extension".to_owned());
            }
            self.pasted.lock().unwrap().push(id);
            Ok(())
        })
    }
}

pub(crate) fn clip_row(id: &str, preview: &str) -> crate::backend::ClipboardRow {
    crate::backend::ClipboardRow {
        id: id.into(),
        preview: preview.into(),
        kind: crate::backend::ClipboardRowKind::Text,
        pinned: false,
        url_host: None,
    }
}

/// Runs `task`, feeding every message it produces back into `app`, and
/// returns what it wrote to the clipboard. Other actions (focus, closing
/// the window) are allowed and ignored.
pub(crate) fn settle(app: &mut LauncherApp, task: Task<Message>) -> Vec<String> {
    use iced::futures::{StreamExt, executor::block_on};
    use iced_winit::runtime::{Action, clipboard, task};
    let Some(stream) = task::into_stream(task) else {
        return Vec::new();
    };
    let actions: Vec<_> = block_on(stream.collect());
    let mut writes = Vec::new();
    for action in actions {
        match action {
            Action::Output(message) => {
                let next = app.update(message);
                writes.extend(settle(app, next));
            }
            Action::Clipboard(clipboard::Action::Write { contents, .. }) => {
                writes.push(contents);
            }
            _ => {}
        }
    }
    writes
}

#[derive(Debug, Default)]
pub(crate) struct FakeWindows {
    pub(crate) rows: Vec<crate::backend::WindowRow>,
    pub(crate) fail_activate: bool,
    pub(crate) activated: std::sync::Mutex<Vec<u32>>,
    pub(crate) closed: std::sync::Mutex<Vec<u32>>,
    pub(crate) listed: std::sync::atomic::AtomicUsize,
    /// The applications running, by desktop id.
    pub(crate) running: Vec<(String, crate::backend::AppRuntimeInfo)>,
    pub(crate) quits: std::sync::Mutex<Vec<(String, bool)>>,
    pub(crate) window_quits: std::sync::Mutex<Vec<(u32, bool)>>,
    pub(crate) fail_quit: bool,
    /// What the window manager can do.
    pub(crate) caps: compass_core::window_switcher::Capabilities,
    pub(crate) workspaces: Vec<crate::backend::WorkspaceRow>,
    pub(crate) focused_workspaces: std::sync::Mutex<Vec<String>>,
    pub(crate) toggles: std::sync::Mutex<Vec<crate::backend::WindowToggle>>,
    /// What a toggle answers when it fails.
    pub(crate) toggle_refusal: Option<&'static str>,
}

impl crate::backend::WindowBackend for FakeWindows {
    fn window_manager_capabilities(
        &self,
    ) -> crate::backend::BackendFuture<'_, compass_core::window_switcher::Capabilities> {
        Box::pin(async move { Ok(self.caps) })
    }
    fn list_workspaces(
        &self,
    ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::WorkspaceRow>> {
        Box::pin(async move { Ok(self.workspaces.clone()) })
    }
    fn focus_workspace(&self, id: String) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.focused_workspaces.lock().unwrap().push(id);
            Ok(())
        })
    }
    fn toggle_window_state(
        &self,
        toggle: crate::backend::WindowToggle,
    ) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            if let Some(refusal) = self.toggle_refusal {
                return Err(refusal.to_owned());
            }
            self.toggles.lock().unwrap().push(toggle);
            Ok(())
        })
    }
    fn list_windows(&self) -> crate::backend::BackendFuture<'_, Vec<crate::backend::WindowRow>> {
        Box::pin(async move {
            self.listed
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.rows.clone())
        })
    }
    fn activate_window(&self, id: u32) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            if self.fail_activate {
                return Err("that window has gone".to_owned());
            }
            self.activated.lock().unwrap().push(id);
            Ok(())
        })
    }
    fn close_window(&self, id: u32) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.closed.lock().unwrap().push(id);
            Ok(())
        })
    }
    fn app_runtime(
        &self,
        id: String,
    ) -> crate::backend::BackendFuture<'_, crate::backend::AppRuntimeInfo> {
        Box::pin(async move {
            Ok(self
                .running
                .iter()
                .find(|(known, _)| *known == id)
                .map(|(_, info)| info.clone())
                .unwrap_or_default())
        })
    }
    fn quit_app(&self, id: String, force: bool) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            if self.fail_quit {
                return Err(format!("Failed to quit {id}"));
            }
            self.quits.lock().unwrap().push((id, force));
            Ok(())
        })
    }
    fn quit_window_app(&self, window: u32, force: bool) -> crate::backend::BackendFuture<'_, ()> {
        Box::pin(async move {
            self.window_quits.lock().unwrap().push((window, force));
            Ok(())
        })
    }
}

pub(crate) fn panel_titles(app: &LauncherApp) -> Vec<String> {
    app.panel
        .as_ref()
        .map(|panel| {
            panel
                .sections
                .iter()
                .flat_map(|section| section.actions.iter().map(|a| a.title.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn choose(app: &mut LauncherApp, title: &str) -> Task<Message> {
    let _ = app.update(Message::PanelFilterChanged(title.to_owned()));
    assert_eq!(
        app.panel
            .as_ref()
            .and_then(PanelState::selected_action)
            .map(|a| a.title.as_str()),
        Some(title)
    );
    app.update(Message::PanelActivate)
}

/// A resident launcher (dismissal hides) whose presentation has a HUD.
pub(crate) fn with_resident_hud(mut app: LauncherApp) -> LauncherApp {
    let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
    let (sender, outcomes) = tokio::sync::mpsc::unbounded_channel();
    // Kept alive for the test's length: a dropped end is a disconnect.
    std::mem::forget((commands, outcomes));
    app.link = Some(EngineLink::new(receiver, sender));
    app.with_hud(true)
}

pub(crate) fn window_row(id: u32, title: &str, app: &str, pid: u32) -> crate::backend::WindowRow {
    crate::backend::WindowRow {
        id,
        title: title.into(),
        app: app.into(),
        wm_class: app.to_lowercase(),
        pid: Some(pid),
        can_close: true,
        app_known: true,
    }
}

pub(crate) fn windows_app(
    dir: &std::path::Path,
    windows: Option<Arc<FakeWindows>>,
) -> (LauncherApp, Arc<TestBackend>) {
    let backend = Arc::new(TestBackend::default());
    let mut app = LauncherApp::with_index(index(dir));
    app.apply(AppFlags {
        windows: windows.map(|w| w as Arc<dyn crate::backend::WindowBackend>),
        ..AppFlags::default()
    });
    app.backend = Some(backend.clone());
    (app, backend)
}

pub(crate) fn open_windows(app: &mut LauncherApp) {
    app.query = "switch windows".into();
    app.search();
    assert_eq!(
        app.selected_row(),
        Some(RootRow::Command(
            compass_core::commands::by_id("commands:switch-windows").unwrap()
        )),
        "{}",
        app.state_line()
    );
    let task = app.update(Message::LaunchSelected);
    settle(app, task);
}
