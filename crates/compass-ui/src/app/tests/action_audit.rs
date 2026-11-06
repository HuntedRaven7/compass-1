//! The action audit (#254): every item a person can pick in the launcher
//! does something they can see.
//!
//! The items are not listed here by hand. Each test enumerates a registry
//! (the builtin commands, the action panel each view and each kind of root
//! row offers, the settings view's clickable controls), runs every item in a
//! fresh launcher over recording fakes, and compares what changed before and
//! after: the page, a message, a dialog, the HUD, the window hiding, the
//! clipboard, or a call the engine, the window manager or the application
//! launcher recorded. An item that changes none of them is dead and fails the
//! test. What each item did is written to `docs/rust-engine/ACTION-AUDIT.md`
//! and checked against it, so a new item, or one whose effect changes, fails
//! until the inventory says so; `COMPASS_UPDATE_ACTION_AUDIT=1` rewrites it.

use super::*;
use crate::app::test_support::{
    FakeClipboard, FakeWindows, RecordingLaunchTarget, TestBackend, clip_row, enable_commands,
    file_row, index, opener, pressed, script_item, stored_shortcut, stored_snippet, window_row,
    with_resident_hud,
};
use std::collections::BTreeMap;
use std::fs;

/// Fields of the recording fakes that a read fills rather than an action:
/// a launch's ranking record, and the queries a view lists itself with.
/// They change on almost every item, so counting them would hide a dead one.
const READS: &[&str] = &[
    "recorded",
    "file_queries",
    "store_queries",
    "opener_lookups",
    "listed",
    "queries",
    "kinds",
    "captures",
    "probes",
];

/// How deep [`drive`] follows the messages a task's messages produce.
const MAX_DEPTH: usize = 12;

/// How long [`drive`] waits on one task: a view that polls (Now Playing
/// lists its players again after each action) never finishes on its own.
const TASK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The runtime tasks run on: some sleep on Tokio's timer.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    })
}

/// Runs `task`, feeding what it produces back into `app` (as far as
/// [`MAX_DEPTH`]), and returns what it wrote to the clipboard.
fn drive(app: &mut LauncherApp, task: Task<Message>) -> Vec<String> {
    drive_at(app, task, 0)
}

fn drive_at(app: &mut LauncherApp, task: Task<Message>, depth: usize) -> Vec<String> {
    use iced::futures::StreamExt;
    use iced_winit::runtime::{Action, clipboard, task};
    if depth > MAX_DEPTH {
        return Vec::new();
    }
    let Some(stream) = task::into_stream(task) else {
        return Vec::new();
    };
    let actions: Vec<_> = runtime().block_on(async move {
        stream
            .take_until(tokio::time::sleep(TASK_WAIT))
            .collect()
            .await
    });
    let mut writes = Vec::new();
    for action in actions {
        match action {
            Action::Output(message) => {
                let next = app.update(message);
                writes.extend(drive_at(app, next, depth + 1));
            }
            Action::Clipboard(clipboard::Action::Write { contents, .. }) => writes.push(contents),
            _ => {}
        }
    }
    writes
}

/// A launcher over recording fakes holding one of everything.
struct World {
    root: std::path::PathBuf,
    _dir: tempfile::TempDir,
    app: LauncherApp,
    backend: Arc<TestBackend>,
    clipboard: Arc<FakeClipboard>,
    windows: Arc<FakeWindows>,
    launcher: Arc<RecordingLaunchTarget>,
    config_path: std::path::PathBuf,
}

fn fixture() -> World {
    use crate::backend as b;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let apps = root.join("apps");
    fs::create_dir_all(&apps).unwrap();
    drop(index(&apps));
    fs::write(
        apps.join("browser.desktop"),
        "[Desktop Entry]\nType=Application\nName=Browser\nExec=/bin/true\nActions=private;\n\
         [Desktop Action private]\nName=Private Window\nExec=/bin/true --private\n",
    )
    .unwrap();
    let extension = root.join("extensions/hello");
    fs::create_dir_all(&extension).unwrap();
    fs::write(
        extension.join("package.json"),
        r#"{"name": "hello", "title": "Hello", "author": "someone",
            "commands": [{"name": "write", "title": "Write Greeting", "mode": "no-view"}]}"#,
    )
    .unwrap();
    let state = root.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("compass.log"), "log\n").unwrap();
    let notes = root.join("notes.txt");
    fs::write(&notes, "notes").unwrap();
    let config_path = root.join("config/compass.json");

    let backend = Arc::new(TestBackend {
        storage: vec![(
            "@someone/hello".into(),
            vec![b::StorageItemRow {
                key: "draft".into(),
                value: "\"hello\"".into(),
            }],
        )],
        token_sets: std::sync::Mutex::new(vec![b::TokenSetRow {
            extension_id: "@someone/hello".into(),
            provider_id: Some("github".into()),
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            id_token: Some("identity".into()),
            scope: Some("repo".into()),
            expires_at: Some(1_900_000_000),
            expired: false,
        }]),
        tray: vec![b::TrayItemRow {
            key: "nm".into(),
            title: "Network".into(),
            subtitle: "Connected".into(),
            has_menu: true,
            ..b::TrayItemRow::default()
        }],
        default_apps: vec![b::DefaultAppRow {
            id: "firefox.desktop".into(),
            name: "Firefox".into(),
            description: String::new(),
            is_default: false,
        }],
        players: std::sync::Mutex::new(vec![b::MediaPlayerRow {
            id: "org.mpris.MediaPlayer2.spot".into(),
            identity: "Spot".into(),
            app_id: "spot".into(),
            title: "Song".into(),
            artist: "Band".into(),
            playing: true,
            paused: false,
            can_go_next: true,
            can_go_previous: true,
        }]),
        grants: std::sync::Mutex::new(vec![b::ScriptGrant {
            id: "script.hello".into(),
            title: "Hello".into(),
            capabilities: vec!["clipboard".into()],
            descriptions: vec!["Read the clipboard".into()],
        }]),
        calculations: std::sync::Mutex::new(vec![b::CalculatorRow {
            id: "calc-1".into(),
            question: "1+1".into(),
            answer: "2".into(),
            conversion: false,
            pinned: false,
        }]),
        openers: vec![opener("firefox.desktop", "Firefox", true)],
        file_info: b::FileActions {
            mime: Some("text/plain".into()),
            has_opener: true,
            can_set_wallpaper: true,
            can_paste: true,
        },
        files: vec![file_row(&notes.to_string_lossy(), "Documents")],
        shortcuts: std::sync::Mutex::new(vec![
            stored_shortcut("sct-docs", "Crate Docs", "https://docs.rs/{crate}"),
            stored_shortcut("sct-news", "Hacker News", "https://news.ycombinator.com"),
        ]),
        snippets: std::sync::Mutex::new(vec![
            stored_snippet("snp-sig", "Signature", "Best,\nMe", Some(";sig")),
            stored_snippet("snp-hi", "Greeting", "Hello there", None),
        ]),
        scripts: vec![script_item(
            "count.sh",
            "Count Things",
            compass_core::script_command::OutputMode::Compact,
            0,
        )],
        store_rows: std::sync::Mutex::new(vec![b::StoreRow {
            id: "store.vicinae.clock".into(),
            name: "clock".into(),
            author: "zoe".into(),
            title: "Clock".into(),
            description: "Clock does things".into(),
            downloads: "12".into(),
            ..b::StoreRow::default()
        }]),
        store_dir: Some(root.join("installed")),
        update: std::sync::Mutex::new(Some(b::UpdateOffer {
            tag: "v99.0.0".into(),
            version: "99.0.0".into(),
            release_url: "https://example.test/releases/v99.0.0".into(),
            current: env!("CARGO_PKG_VERSION").into(),
        })),
        rates: Some(compass_core::exchange_rates::ExchangeRates {
            date: "2026-09-24".into(),
            fetched_at: 7,
            rates: [("EUR".to_owned(), 1.0), ("USD".to_owned(), 1.25)].into(),
        }),
        rhai_scripts: Some(vec![compass_core::rhai_scripts::RhaiScriptItem {
            id: "script.tidy".into(),
            title: "Tidy Downloads".into(),
            description: None,
            icon: None,
            keywords: Vec::new(),
        }]),
        records_settings: true,
        ..TestBackend::default()
    });
    let clipboard = Arc::new(FakeClipboard {
        rows: vec![clip_row("clip-1", "hello from the clipboard")],
        content: Some(b::ClipboardContent {
            mime_type: "text/plain".into(),
            data: b"hello from the clipboard".to_vec(),
        }),
        can_paste: true,
        ..FakeClipboard::default()
    });
    let windows = Arc::new(FakeWindows {
        rows: vec![window_row(42, "Draft", "Editor", 7)],
        caps: compass_core::window_switcher::Capabilities {
            workspaces: true,
            fullscreen: true,
            toggle_floating: true,
            toggle_overview: true,
            set_sticky: true,
            move_to_workspace: true,
        },
        workspaces: vec![b::WorkspaceRow {
            id: "2".into(),
            name: "Two".into(),
            window_count: 1,
            ..b::WorkspaceRow::default()
        }],
        running: vec![(
            "firefox.desktop".into(),
            b::AppRuntimeInfo {
                running: true,
                frontmost: false,
                windows: vec![window_row(43, "Mozilla", "Firefox", 8)],
            },
        )],
        ..FakeWindows::default()
    });
    let launcher = Arc::new(RecordingLaunchTarget::default());

    let index = AppIndex::builder()
        .dir(&apps)
        .extension_dirs([root.join("extensions")])
        .build();
    let mut app = LauncherApp::with_index(index);
    app.apply(AppFlags {
        launcher: launcher.clone(),
        backend: Some(backend.clone()),
        clipboard: Some(clipboard.clone()),
        windows: Some(windows.clone()),
        config_path: Some(config_path.clone()),
        log_path: Some(state.join("compass.log")),
        default_config_dir: Some(root.join("cache")),
        fallbacks: compass_core::Config::default().fallback_ids(),
        ..AppFlags::default()
    });
    let mut app = with_resident_hud(app);
    app.window = Some(window::Id::unique());
    // Every command root search can offer, the ones off by default too.
    let off: Vec<&str> = compass_core::commands::BUILTIN_COMMANDS
        .iter()
        .filter(|command| command.default_disabled())
        .map(|command| command.entrypoint)
        .collect();
    enable_commands(&mut app, &off);
    let tasks = Task::batch([
        app.refresh_shortcuts_task(),
        app.refresh_scripts_task(),
        app.refresh_rhai_scripts_task(),
        app.refresh_update_task(),
        app.window_capabilities_task(),
        app.exchange_rates_task(),
    ]);
    drive(&mut app, tasks);
    World {
        root,
        _dir: dir,
        app,
        backend,
        clipboard,
        windows,
        launcher,
        config_path,
    }
}

/// What the launcher shows and what the fakes heard, at one moment.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    page_kind: String,
    page: String,
    query: String,
    error: Option<String>,
    confirm: bool,
    hud: Option<String>,
    closing: bool,
    panel: Option<String>,
    calls: BTreeMap<String, String>,
    root_config: String,
    theme: String,
    config_file: Option<String>,
}

/// A struct's pretty `Debug`, cut into its top-level fields.
fn fields(source: &str, debug: &str, calls: &mut BTreeMap<String, String>) {
    let mut name: Option<String> = None;
    for line in debug.lines().skip(1) {
        let top = line
            .strip_prefix("    ")
            .filter(|rest| !rest.starts_with(' '));
        if let Some((field, _)) = top.and_then(|rest| rest.split_once(':')) {
            name = Some(format!("{source}.{field}"));
        }
        if let Some(name) = &name {
            let entry = calls.entry(name.clone()).or_default();
            entry.push_str(line);
            entry.push('\n');
        }
    }
}

impl World {
    fn snapshot(&self) -> Snapshot {
        let app = &self.app;
        let page = format!("{:?}", app.page);
        let page_kind = page
            .split(|c: char| !c.is_alphanumeric())
            .next()
            .unwrap_or_default()
            .to_owned();
        let mut calls = BTreeMap::new();
        fields("engine", &format!("{:#?}", self.backend), &mut calls);
        fields("clipboard", &format!("{:#?}", self.clipboard), &mut calls);
        fields("windows", &format!("{:#?}", self.windows), &mut calls);
        calls.insert(
            "launcher.launched".to_owned(),
            format!("{:?}", self.launcher.0.lock().unwrap()),
        );
        calls.retain(|name, _| {
            !READS
                .iter()
                .any(|read| name.rsplit('.').next() == Some(*read))
        });
        Snapshot {
            page_kind,
            page,
            query: app.query.clone(),
            error: app.error.clone(),
            confirm: app.confirm.is_some() || app.power_confirm.is_some(),
            hud: app.hud_content().map(|hud| hud.text.clone()),
            closing: app.closing,
            panel: app.panel.as_ref().map(|panel| format!("{panel:?}")),
            calls,
            root_config: format!("{:?}", app.root_config),
            theme: format!("{:?} {:?}", app.view.theme_choice, app.view.theme_preview),
            config_file: fs::read_to_string(&self.config_path).ok(),
        }
    }

    /// Runs `task` through the launcher, then says what changed since
    /// `before`.
    fn effects_of(&mut self, before: &Snapshot, task: Task<Message>) -> Vec<String> {
        let writes = drive(&mut self.app, task);
        let after = self.snapshot();
        let mut effects = Vec::new();
        if after.page_kind != before.page_kind {
            effects.push(format!("opens {}", after.page_kind));
        } else if after.page != before.page {
            effects.push(format!("changes {}", after.page_kind));
        }
        // Text put into the search bar, as Put answer in search bar does.
        // Clearing it is how hiding and opening a view tidy up, so only new
        // text counts.
        if !after.query.is_empty() && after.query != before.query {
            effects.push(format!("types {:?}", after.query));
        }
        if after.error.is_some() && after.error != before.error {
            effects.push(format!("says {:?}", self.clean(after.error.as_deref())));
        }
        if after.confirm && !before.confirm {
            effects.push("asks to confirm".to_owned());
        }
        if after.hud.is_some() && after.hud != before.hud {
            effects.push(format!("HUD {:?}", self.clean(after.hud.as_deref())));
        }
        if after.closing && !before.closing {
            effects.push("hides".to_owned());
        }
        if !writes.is_empty() {
            effects.push("copies".to_owned());
        }
        if after.panel.is_some() && after.panel != before.panel {
            effects.push("changes the panel".to_owned());
        }
        for (name, value) in &after.calls {
            if before.calls.get(name) != Some(value) {
                effects.push(format!("calls {name}"));
            }
        }
        if after.root_config != before.root_config {
            effects.push("changes root settings".to_owned());
        }
        if after.theme != before.theme {
            effects.push("changes the theme".to_owned());
        }
        if after.config_file != before.config_file {
            effects.push("writes compass.json".to_owned());
        }
        effects
    }

    /// `text` with this world's directory taken out, so the inventory reads
    /// the same on every run.
    fn clean(&self, text: Option<&str>) -> String {
        let text = text.unwrap_or_default();
        let text = text.replace(&*self.root.to_string_lossy(), "<tmp>");
        match text.char_indices().nth(72) {
            Some((at, _)) => format!("{}…", &text[..at]),
            None => text,
        }
    }

    /// Runs `task` and what follows it; if it asked to confirm, confirms and
    /// says what that did too.
    fn run(&mut self, task: impl FnOnce(&mut LauncherApp) -> Task<Message>) -> Vec<String> {
        let before = self.snapshot();
        let task = task(&mut self.app);
        let mut effects = self.effects_of(&before, task);
        if effects.iter().any(|effect| effect == "asks to confirm") {
            let before = self.snapshot();
            let task = self.app.update(pressed(iced::keyboard::key::Named::Enter));
            let then = self.effects_of(&before, task);
            effects.push(format!("then {}", describe(&then)));
        }
        effects
    }

    /// Types `query` at the root and selects the first row `wanted` accepts.
    fn select(&mut self, query: &str, wanted: impl Fn(&LauncherApp, RootRow) -> bool) -> bool {
        self.app.page = Page::Root;
        self.app.panel = None;
        self.app.query = query.to_owned();
        self.app.search();
        let Some(position) = self
            .app
            .results
            .iter()
            .position(|row| wanted(&self.app, *row))
        else {
            return false;
        };
        self.app.selected = position;
        true
    }

    /// Selects `command` in root search by its own title.
    fn select_command(&mut self, command: &BuiltinCommand) {
        let found = self.select(
            command.title,
            |_, row| matches!(row, RootRow::Command(c) if c.entrypoint == command.entrypoint),
        );
        assert!(
            found,
            "{} is not offered for its own title: {}",
            command.title,
            self.app.state_line()
        );
    }

    /// Opens `command` from root search, as Enter does.
    fn open(&mut self, command: &BuiltinCommand) -> Vec<String> {
        self.select_command(command);
        self.run(|app| app.update(Message::LaunchSelected))
    }

    /// The action panel over what is shown, opened as Ctrl+B opens it: its
    /// actions' titles, by the panel row each is on.
    fn panel(&mut self) -> Vec<(usize, String)> {
        let task = self.app.update(Message::TogglePanel);
        drive(&mut self.app, task);
        let Some(panel) = &self.app.panel else {
            return Vec::new();
        };
        panel
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.selectable())
            .filter_map(|(at, row)| {
                let action = panel.sections.get(row.section)?.actions.get(row.action?)?;
                Some((at, action.title.clone()))
            })
            .collect()
    }
}

/// Effects as one cell of the inventory.
fn describe(effects: &[String]) -> String {
    if effects.is_empty() {
        "NOTHING".to_owned()
    } else {
        effects.join("; ")
    }
}

/// One audited item: what it is, what it did, and the test that saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    item: String,
    effect: String,
    test: &'static str,
}

/// The inventory's section for `test` in `ACTION-AUDIT.md`, between its
/// markers.
fn check_inventory(section: &str, rows: &[Row]) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/rust-engine/ACTION-AUDIT.md");
    let begin = format!("<!-- audit:{section} -->");
    let end = format!("<!-- /audit:{section} -->");
    let mut table = String::from("| Item | Effect | Test |\n|---|---|---|\n");
    for row in rows {
        table.push_str(&format!(
            "| {} | {} | `{}` |\n",
            row.item.replace('|', "\\|"),
            row.effect.replace('|', "\\|"),
            row.test
        ));
    }
    let text = fs::read_to_string(&path).unwrap_or_default();
    let (Some(start), Some(stop)) = (text.find(&begin), text.find(&end)) else {
        panic!("{} has no {begin} … {end} section", path.display());
    };
    let current = &text[start + begin.len()..stop];
    let wanted = format!("\n{table}");
    if current == wanted {
        return;
    }
    if std::env::var_os("COMPASS_UPDATE_ACTION_AUDIT").is_some() {
        let updated = format!("{}{wanted}{}", &text[..start + begin.len()], &text[stop..]);
        fs::write(&path, updated).unwrap();
        return;
    }
    let old: Vec<&str> = current.lines().collect();
    let differences: Vec<String> = wanted
        .lines()
        .filter(|line| !old.contains(line))
        .map(|line| format!("+ {line}"))
        .chain(
            old.iter()
                .filter(|line| !wanted.lines().any(|new| new == **line))
                .map(|line| format!("- {line}")),
        )
        .collect();
    panic!(
        "the {section} inventory in ACTION-AUDIT.md is out of date; check what changed, then \
         rewrite it with COMPASS_UPDATE_ACTION_AUDIT=1:\n{}",
        differences.join("\n")
    );
}

/// Fails with every dead item at once, so one run lists them all.
fn assert_none_dead(rows: &[Row]) {
    let dead: Vec<&str> = rows
        .iter()
        .filter(|row| row.effect.contains("NOTHING"))
        .map(|row| row.item.as_str())
        .collect();
    assert!(dead.is_empty(), "these items do nothing: {dead:#?}");
}

use compass_core::commands::{BUILTIN_COMMANDS, BuiltinCommand};

#[test]
fn every_builtin_command_does_something_from_root_search() {
    const TEST: &str = "every_builtin_command_does_something_from_root_search";
    let rows: Vec<Row> = BUILTIN_COMMANDS
        .iter()
        .map(|command| {
            let mut world = fixture();
            let effects = world.open(command);
            Row {
                item: command.title.to_owned(),
                effect: describe(&effects),
                test: TEST,
            }
        })
        .collect();
    assert_none_dead(&rows);
    check_inventory("commands", &rows);
}

fn command(entrypoint: &str) -> &'static BuiltinCommand {
    BUILTIN_COMMANDS
        .iter()
        .find(|command| command.entrypoint == entrypoint)
        .unwrap_or_else(|| panic!("no command {entrypoint}"))
}

/// #253: Open Config File opens the live `compass.json`, writing it first
/// when there is none yet; Show Log File and Open Default Config File reach
/// Compass's own files; and a failure is said where it can be seen.
#[test]
fn the_config_and_log_commands_open_compass_files_and_say_why_not() {
    let mut world = fixture();
    assert!(!world.config_path.exists(), "no configuration file yet");
    world.open(command("open-config-file"));
    let written = compass_core::Config::load_from(&world.config_path).unwrap();
    assert_eq!(written.schema(), Some(compass_core::config::SCHEMA_URL));
    assert_eq!(
        world.backend.opened.lock().unwrap().as_slice(),
        [(world.config_path.to_string_lossy().into_owned(), false)]
    );

    let mut world = fixture();
    world.open(command("show-logs"));
    let log = world.root.join("state/compass.log");
    assert_eq!(
        world.backend.opened.lock().unwrap().as_slice(),
        [(log.to_string_lossy().into_owned(), true)],
        "shown in the file browser"
    );
    let mut world_without_log = fixture();
    fs::remove_file(world_without_log.root.join("state/compass.log")).unwrap();
    let effects = world_without_log.open(command("show-logs"));
    assert_eq!(
        world_without_log.app.error.as_deref(),
        Some(super::super::compass_commands::NO_LOG_YET),
        "{effects:?}"
    );

    let mut world = fixture();
    world.open(command("open-default-config"));
    let default = world.root.join("cache/default-config.jsonc");
    assert!(fs::metadata(&default).unwrap().permissions().readonly());
    assert_eq!(
        world.backend.opened.lock().unwrap().as_slice(),
        [(default.to_string_lossy().into_owned(), false)]
    );

    // Nothing opens it: the launcher stays, and says why.
    let mut world = fixture();
    let refusing = Arc::new(TestBackend {
        refuse_opens: Some("No application opens this kind of file".into()),
        ..TestBackend::default()
    });
    world.app.backend = Some(refusing);
    let effects = world.open(command("open-config-file"));
    assert_eq!(
        world.app.error.as_deref(),
        Some("No application opens this kind of file")
    );
    assert!(
        !effects.iter().any(|effect| effect == "hides"),
        "{effects:?}"
    );
}

/// What a view needs after it opens before its panel has a row to act on.
fn prepare_view(world: &mut World, command: &BuiltinCommand) {
    let typed = match command.entrypoint {
        "search-files" => Some("notes"),
        "run-program" => Some("true"),
        _ => None,
    };
    if let Some(typed) = typed {
        let task = world.app.update(Message::QueryChanged(typed.to_owned()));
        drive(&mut world.app, task);
    }
    // The stores open on their intro; its one way on is Continue.
    if matches!(world.app.page, Page::StoreIntro(_)) {
        let task = world.app.continue_to_store();
        drive(&mut world.app, task);
    }
    // A glyph picked before, so Reset ranking has a ranking to reset.
    if let Page::Emoji(page) = &mut world.app.page
        && let Some(glyph) = page.selected_glyph()
    {
        page.register_visit(glyph);
    }
}

#[test]
fn every_action_in_a_builtin_views_panel_does_something() {
    const TEST: &str = "every_action_in_a_builtin_views_panel_does_something";
    let mut rows = Vec::new();
    for command in BUILTIN_COMMANDS {
        let mut world = fixture();
        world.open(command);
        if world.app.closing || matches!(world.app.page, Page::Root) {
            continue;
        }
        prepare_view(&mut world, command);
        let actions = world.panel();
        for (row, title) in actions {
            let mut world = fixture();
            world.open(command);
            prepare_view(&mut world, command);
            let offered = world.panel();
            assert!(
                offered.iter().any(|(at, t)| *at == row && *t == title),
                "{} › {title} moved between runs",
                command.title
            );
            let effects = world.run(|app| app.update(Message::PanelClicked(row)));
            rows.push(Row {
                item: format!("{} › {title}", command.title),
                effect: describe(&effects),
                test: TEST,
            });
        }
    }
    assert_none_dead(&rows);
    check_inventory("view-panels", &rows);
}

/// Each kind of root row, as a query and the row it selects.
type RowPick = (
    &'static str,
    &'static str,
    fn(&LauncherApp, RootRow) -> bool,
);

const ROOT_ROWS: &[RowPick] = &[
    ("Application", "browser", |_, row| {
        matches!(row, RootRow::App(_))
    }),
    (
        "Builtin command",
        "clipboard history",
        |_, row| matches!(row, RootRow::Command(c) if c.entrypoint == "clipboard-history"),
    ),
    ("Extension command", "write greeting", |_, row| {
        matches!(row, RootRow::Extension(_))
    }),
    ("Quicklink", "crate docs", |_, row| {
        matches!(row, RootRow::Shortcut(_))
    }),
    ("Script command", "count things", |_, row| {
        matches!(row, RootRow::Script(_))
    }),
    ("Rhai script", "tidy downloads", |_, row| {
        matches!(row, RootRow::RhaiScript(_))
    }),
    ("Calculator answer", "1+1", |_, row| {
        matches!(row, RootRow::Calculator)
    }),
    ("Fallback", "zebra crossing", |_, row| {
        matches!(row, RootRow::Fallback(_))
    }),
    ("Update", "", |_, row| matches!(row, RootRow::Update)),
];

#[test]
fn every_action_in_a_root_rows_panel_does_something() {
    const TEST: &str = "every_action_in_a_root_rows_panel_does_something";
    let mut rows = Vec::new();
    for (kind, query, wanted) in ROOT_ROWS {
        let mut world = fixture();
        assert!(
            world.select(query, wanted),
            "no {kind} row for {query:?}: {}",
            world.app.state_line()
        );
        // Every kind of row offers a panel: Ctrl+B over one that opens
        // nothing is as dead as an action that does nothing.
        let actions = world.panel();
        assert!(
            !actions.is_empty(),
            "Ctrl+B over a {kind} row opens nothing"
        );
        for (row, title) in actions {
            let mut world = fixture();
            world.select(query, wanted);
            world.panel();
            let effects = world.run(|app| app.update(Message::PanelClicked(row)));
            rows.push(Row {
                item: format!("{kind} › {title}"),
                effect: describe(&effects),
                test: TEST,
            });
        }
    }
    assert_none_dead(&rows);
    check_inventory("root-panels", &rows);
}

/// Where the settings view is clicked: a grid over the card (from, to,
/// step), one pass per scroll position of its body, each pass scrolled
/// [`SCROLL_STEP`] further.
const GRID_X: (u16, u16, u16) = (140, 890, 16);
const GRID_Y: (u16, u16, u16) = (0, 600, 9);
const SCROLL_PASSES: usize = 8;
const SCROLL_STEP: f32 = 250.0;

/// How far below a click the probe click lands: on a list's first option,
/// when the click opened one.
const OPTION_PROBE: f32 = 34.0;

/// Every message a click anywhere on the view produces, in the order found,
/// each once. The grid finds what a hand-written list would miss: a button
/// added tomorrow is clicked without anyone naming it. Each click is followed
/// by one just below it, which picks a list's first option when the click
/// opened a list and is one more grid click when it did not.
fn clickables(app: &LauncherApp) -> Vec<Message> {
    use iced::mouse::{Event as Mouse, ScrollDelta};
    let mut messages = Vec::new();
    for pass in 0..SCROLL_PASSES {
        let mut ui = iced_test::simulator(app.view());
        if pass > 0 {
            ui.point_at(iced::Point::new(600.0, 300.0));
            let _ = ui.simulate([iced::Event::Mouse(Mouse::WheelScrolled {
                delta: ScrollDelta::Pixels {
                    x: 0.0,
                    y: -SCROLL_STEP * pass as f32,
                },
            })]);
        }
        for y in (GRID_Y.0..GRID_Y.1).step_by(usize::from(GRID_Y.2)) {
            // On the unscrolled pass, which holds every short page whole, each
            // grid line gets a fresh simulator: state an earlier click left in
            // a widget (a focused field, a list it opened) otherwise hid
            // controls further down from the grid, so what the audit found
            // shifted with the layout. Later passes only sample the long
            // pages, and rebuilding there costs minutes for little.
            if pass == 0 {
                messages.extend(ui.into_messages());
                ui = iced_test::simulator(app.view());
            }
            for x in (GRID_X.0..GRID_X.1).step_by(usize::from(GRID_X.2)) {
                let (x, y) = (f32::from(x), f32::from(y));
                ui.point_at(iced::Point::new(x, y));
                let _ = ui.simulate(iced_test::simulator::click());
                ui.point_at(iced::Point::new(x, y + OPTION_PROBE));
                let _ = ui.simulate(iced_test::simulator::click());
            }
            // Close any list a click opened before the next line.
            ui.point_at(EMPTY_CORNER);
            let _ = ui.simulate(iced_test::simulator::click());
        }
        messages.extend(ui.into_messages());
    }
    let mut seen = std::collections::BTreeSet::new();
    messages
        .into_iter()
        .filter(|message| seen.insert(format!("{message:?}")))
        .collect()
}

/// A point inside the window but on no control: the card's top-left padding.
const EMPTY_CORNER: iced::Point = iced::Point::new(4.0, 4.0);

/// A fresh launcher with the settings open at sidebar `row`.
fn settings_at(row: usize) -> World {
    let mut world = fixture();
    let open = BUILTIN_COMMANDS
        .iter()
        .find(|command| command.entrypoint == "settings")
        .unwrap();
    world.open(open);
    let task = world.app.update(Message::Settings(
        crate::settings_page::SettingsMessage::SidebarSelected(row),
    ));
    drive(&mut world.app, task);
    world
}

#[test]
fn every_control_in_the_settings_view_does_something() {
    const TEST: &str = "every_control_in_the_settings_view_does_something";
    let world = settings_at(0);
    let Page::Settings(page) = &world.app.page else {
        panic!(
            "Open Settings did not open them: {}",
            world.app.state_line()
        );
    };
    let tabs: Vec<(usize, String)> = page
        .sidebar
        .rows()
        .iter()
        .enumerate()
        .filter(|(_, row)| row.selectable())
        .map(|(at, row)| (at, row.label.clone()))
        .collect();
    let mut found: Vec<(usize, String, Message)> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (row, label) in &tabs {
        let world = settings_at(*row);
        for message in clickables(&world.app) {
            if seen.insert(format!("{message:?}")) {
                found.push((*row, label.clone(), message));
            }
        }
    }
    assert!(
        found.len() > tabs.len(),
        "the grid found too little: {found:?}"
    );
    let rows: Vec<Row> = found
        .into_iter()
        .map(|(row, label, message)| {
            let item = format!("{message:?}");
            let item = item
                .strip_prefix("Settings(")
                .and_then(|inner| inner.strip_suffix(')'))
                .unwrap_or(&item)
                .to_owned();
            // A page's own sidebar row is clicked from another page: on its
            // own page it is already selected.
            let from = match &message {
                Message::Settings(crate::settings_page::SettingsMessage::SidebarSelected(at))
                    if *at == row =>
                {
                    tabs.iter()
                        .map(|(other, _)| *other)
                        .find(|other| *other != row)
                        .unwrap_or(row)
                }
                _ => row,
            };
            let mut world = settings_at(from);
            let effects = world.run(|app| app.update(message));
            Row {
                item: format!("{label} › {item}"),
                effect: describe(&effects),
                test: TEST,
            }
        })
        .collect();
    assert_none_dead(&rows);
    check_inventory("settings", &rows);
}
