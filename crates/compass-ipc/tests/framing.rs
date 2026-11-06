//! Frame-level tests: exhaustive variant round-trips, partial frames, and the
//! oversized-frame guard.

use compass_ipc::codec::{FrameCodec, LENGTH_PREFIX_LEN, MAX_FRAME_LEN};
use compass_ipc::{
    ClipboardEntry, ClipboardKind, DoctorCheck, DoctorStatus, Error, ErrorKind, PROTOCOL_VERSION,
    ProtocolError, QueryHit, Request, RequestEnvelope, Response, ResponseEnvelope, WindowCommand,
    WindowOutcome,
};
use tokio_util::bytes::{BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

/// Every `Request` variant. Kept exhaustive by the `match` in
/// [`request_variants_are_exhaustive`].
fn all_requests() -> Vec<Request> {
    vec![
        Request::Ping,
        Request::Toggle,
        Request::Show,
        Request::Hide,
        Request::Query {
            text: String::new(),
        },
        Request::Query {
            text: "firefox --private".into(),
        },
        Request::Query {
            text: "unicode: é 漢字 🚀".into(),
        },
        Request::Doctor,
        Request::Shutdown,
        Request::AttachWindow,
        Request::WindowOutcome(WindowOutcome::Shown),
        Request::WindowOutcome(WindowOutcome::Hidden),
        Request::WindowOutcome(WindowOutcome::Failed(String::new())),
        Request::WindowOutcome(WindowOutcome::Failed("no compositor: é 🚀".into())),
        Request::RecordLaunch {
            key: "app.desktop".into(),
        },
        Request::RecordLaunch { key: String::new() },
        Request::ClipboardHistory {
            query: String::new(),
            limit: 50,
        },
        Request::ClipboardHistory {
            query: "https://é.example 🚀".into(),
            limit: u32::MAX,
        },
        Request::ClipboardContent { id: "abc".into() },
        Request::ClipboardContent { id: String::new() },
        Request::ListWindows,
        Request::ActivateWindow { id: 0 },
        Request::ActivateWindow { id: u32::MAX },
        Request::CloseWindow { id: 7 },
        Request::ClipboardPaste { id: "abc".into() },
        Request::ClipboardSetPinned {
            id: "abc".into(),
            pinned: true,
        },
        Request::ClipboardSetPinned {
            id: String::new(),
            pinned: false,
        },
        Request::ClipboardRemove { id: "abc".into() },
        Request::RunPowerCommand {
            id: "reboot".into(),
        },
        Request::RunMediaCommand {
            id: "play-pause".into(),
        },
        Request::RunExtensionCommand {
            id: "@raycast/github:search-repositories".into(),
            arguments_json: Some(r#"{"query":"compass"}"#.into()),
        },
        Request::ExtensionView {
            session: 1,
            after: 0,
        },
        Request::ExtensionView {
            session: u64::MAX,
            after: u64::MAX,
        },
        Request::ExtensionEvent {
            session: 1,
            handler: "cb-7".into(),
            args_json: "[\"é 🚀\", 3]".into(),
        },
        Request::ExtensionPop { session: 1 },
        Request::SetExtensionPreferences {
            id: "@raycast/github:search".into(),
            values_json: "{\"token\":\"é 🚀\"}".into(),
        },
        Request::ExtensionAlertAnswer {
            session: 1,
            confirmed: true,
        },
        Request::CloseExtension { session: 1 },
        Request::SearchFiles {
            query: "rapport é 🚀".into(),
            category: Some("Documents".into()),
        },
        Request::SearchFiles {
            query: String::new(),
            category: None,
        },
        Request::OpenFile {
            path: "/home/me/Documents/rapport é.pdf".into(),
            reveal: true,
        },
        Request::OAuthRedirect {
            url: "raycast://oauth?package_name=Extension&code=é&state=s".into(),
        },
        Request::ListShortcuts,
        Request::SaveShortcut {
            id: None,
            name: "Recherche 🚀".into(),
            icon: "default".into(),
            url: "https://x.test/?q={query}".into(),
            app: "default".into(),
        },
        Request::SaveShortcut {
            id: Some("sct-0123456789ab".into()),
            name: String::new(),
            icon: "icon://builtin/link".into(),
            url: "{clipboard}".into(),
            app: "firefox.desktop".into(),
        },
        Request::RemoveShortcut {
            id: "sct-0123456789ab".into(),
        },
        Request::OpenShortcut {
            id: "sct-0123456789ab".into(),
            arguments: vec!["é".into(), String::new()],
        },
        Request::ExpandShortcut {
            id: "sct-0123456789ab".into(),
            arguments: vec![],
        },
        Request::ListSnippets,
        Request::SaveSnippet {
            id: None,
            name: "Signature ✍".into(),
            text: "Best,\n{cursor}".into(),
            keyword: Some(";sig".into()),
            word: true,
            apps: vec!["org.gnome.TextEditor.desktop".into()],
        },
        Request::SaveSnippet {
            id: Some("snp-0123456789ab".into()),
            name: "Address".into(),
            text: "1 Rue de l'Église".into(),
            keyword: None,
            word: false,
            apps: vec![],
        },
        Request::RemoveSnippet {
            id: "snp-0123456789ab".into(),
        },
        Request::ExpandSnippet {
            id: "snp-0123456789ab".into(),
            arguments: vec![("name".into(), "Zoë".into())],
        },
        Request::PasteSnippet {
            id: "snp-0123456789ab".into(),
            arguments: vec![],
        },
        Request::ListScripts,
        Request::RunScript {
            id: "tools.uptime.sh".into(),
            arguments: vec!["é".into(), String::new()],
        },
        Request::ScriptOutput { session: 3 },
        Request::StopScript { session: 3 },
        Request::ListPrograms,
        Request::Dmenu {
            spec: compass_ipc::DmenuSpec {
                content: "alpha\n/home/me/β.txt\n\ngamma 🚀".into(),
                navigation_title: Some("Pick".into()),
                section_title: Some("Items ({count})".into()),
                output_index: true,
                placeholder: Some("Filter…".into()),
                query: Some("al".into()),
                width: Some(480),
                height: None,
                no_section: false,
                no_quick_look: true,
                no_metadata: false,
                no_footer: true,
            },
        },
        Request::Dmenu {
            spec: compass_ipc::DmenuSpec::default(),
        },
        Request::DmenuFetch { token: 9 },
        Request::DmenuChoose {
            token: 9,
            output: Some("β".into()),
        },
        Request::DmenuChoose {
            token: 9,
            output: None,
        },
        Request::SetTheme {
            theme: "tokyo-night".into(),
        },
        Request::StoreBrowse {
            store: compass_ipc::StoreKind::Raycast,
            query: "spotify & co".into(),
        },
        Request::StoreExtension {
            store: compass_ipc::StoreKind::Vicinae,
            owner: "zoë".into(),
            name: "clock".into(),
        },
        Request::StoreInstall {
            store: compass_ipc::StoreKind::Vicinae,
            owner: "zoë".into(),
            name: "clock".into(),
        },
        Request::StoreUninstall {
            id: "store.vicinae.clock".into(),
        },
        Request::OpenUrl {
            url: "https://example.com/?q=é".into(),
        },
        Request::InputServerStatus,
        Request::SetInputServerEnabled { enabled: false },
        Request::ExtensionLaunchFetch { token: u64::MAX },
        Request::ExtensionSubtitles,
        Request::ExtensionPreferences {
            id: "@zoë/notes:list".into(),
        },
        Request::RunMediaCommandWith {
            id: "play-pause".into(),
            argument: Some("spötify".into()),
        },
        Request::ListMediaPlayers,
        Request::SetFont {
            family: "Noto Sans ไทย".into(),
        },
        Request::OpenDeeplink {
            url: "vicinae://extensions/zoë/clock".into(),
        },
        Request::ListScriptGrants,
        Request::RevokeScriptGrant {
            id: "script.quick-notes".into(),
        },
        Request::CatalogGeneration,
        Request::ListDefaultApps {
            kind: compass_ipc::DefaultAppKind::Browser,
        },
        Request::SetDefaultApp {
            kind: compass_ipc::DefaultAppKind::Terminal,
            id: "org.gnome.Ptyxis.desktop".into(),
        },
        Request::ClipboardHistoryOfKind {
            query: "café".into(),
            limit: 100,
            kind: Some(compass_ipc::ClipboardKind::Image),
        },
        Request::ClipboardDetail {
            id: "c0ffee".into(),
        },
        Request::ClipboardSetKeywords {
            id: "c0ffee".into(),
            keywords: "reçu facture".into(),
        },
        Request::ClipboardRemoveAll,
        Request::ClipboardMonitoring {
            enabled: Some(false),
        },
        Request::RootItemEdit {
            id: "applications:org.gnome.TextEditor".into(),
            edit: compass_ipc::RootItemEdit::Favorite(true),
        },
        Request::RootItemEdit {
            id: "commands:clipboard-history".into(),
            edit: compass_ipc::RootItemEdit::MoveFavorite { down: true },
        },
        Request::RootItemEdit {
            id: "@zoë/notes:list".into(),
            edit: compass_ipc::RootItemEdit::Alias("nö".into()),
        },
        Request::RootItemEdit {
            id: "scripts:hello".into(),
            edit: compass_ipc::RootItemEdit::Disable,
        },
        Request::RootItemEdit {
            id: "commands:clipboard-history".into(),
            edit: compass_ipc::RootItemEdit::ResetRanking,
        },
        Request::RootItemEdit {
            id: "@zoë/notes:list".into(),
            edit: compass_ipc::RootItemEdit::Shortcut("super+control+alt+shift+Ö".into()),
        },
        Request::RootItemEdit {
            id: "scripts:hello".into(),
            edit: compass_ipc::RootItemEdit::Shortcut(String::new()),
        },
        Request::RootItemEdit {
            id: "scripts:hello".into(),
            edit: compass_ipc::RootItemEdit::Enabled(true),
        },
        Request::SetSetting {
            key: "launcher.appearance.theme".into(),
            value_json: "\"tokyo-night\"".into(),
        },
        Request::SetSetting {
            key: "providers.files.preferences.indexingPaths".into(),
            value_json: "[\"/home/ä/Docs\"]".into(),
        },
        Request::SetProviderEnabled {
            provider: "@zoë/notes".into(),
            enabled: false,
        },
        Request::RootItemEdit {
            id: "files:search".into(),
            edit: compass_ipc::RootItemEdit::Fallback(true),
        },
        Request::RootItemEdit {
            id: "@zoë/notes:new".into(),
            edit: compass_ipc::RootItemEdit::Fallback(false),
        },
        Request::ListCommands,
        Request::LaunchCommand {
            id: "@zoë/notes:new".into(),
            args: vec!["first ✓".into(), String::new()],
            cwd: Some("/home/zoë".into()),
            query: Some("groceries 🛒".into()),
        },
        Request::LaunchCommand {
            id: "commands:clipboard-history".into(),
            args: vec![],
            cwd: None,
            query: None,
        },
        Request::LaunchApp {
            id: "org.gnome.Nautilus.desktop".into(),
            args: vec!["/home/zoë/Téléchargements".into()],
            new_instance: true,
        },
        Request::DescribeWindow,
        Request::AppRuntime {
            id: "org.gnome.Nautilus.desktop".into(),
        },
        Request::QuitApp {
            id: "firefox.desktop".into(),
            force: true,
        },
        Request::QuitWindowApp {
            window: u32::MAX,
            force: false,
        },
        Request::CalculatorHistory {
            query: "√2 ≈".into(),
        },
        Request::AddCalculatorRecord {
            question: "5 ft to m".into(),
            answer: "1.524 m".into(),
            conversion: true,
        },
        Request::EditCalculatorHistory {
            edit: compass_ipc::CalculatorEdit::Pin("ä-1".into()),
        },
        Request::EditCalculatorHistory {
            edit: compass_ipc::CalculatorEdit::Unpin("ä-1".into()),
        },
        Request::EditCalculatorHistory {
            edit: compass_ipc::CalculatorEdit::Remove("ä-1".into()),
        },
        Request::EditCalculatorHistory {
            edit: compass_ipc::CalculatorEdit::RemoveAll,
        },
        Request::TrayItems,
        Request::TrayActivate {
            key: ":1.42/StatusNotifierItem".into(),
            secondary: true,
        },
        Request::TrayMenu {
            key: "org.kde.StatusNotifierItem-7-1/StatusNotifierItem".into(),
        },
        Request::TrayTriggerMenu {
            key: ":1.42/StatusNotifierItem".into(),
            id: i32::MIN,
        },
        Request::PasteText {
            text: "👍🏽 zoë".into(),
        },
        Request::WindowManagerCapabilities,
        Request::ListWorkspaces,
        Request::FocusWorkspace { id: "ä-3".into() },
        Request::ToggleWindowState {
            toggle: compass_ipc::WindowToggle::Fullscreen,
        },
        Request::ToggleWindowState {
            toggle: compass_ipc::WindowToggle::Floating,
        },
        Request::ToggleWindowState {
            toggle: compass_ipc::WindowToggle::Overview,
        },
        Request::ListOpeners {
            target: "https://example.com/?q={query}".into(),
        },
        Request::OpenWith {
            app: "org.gnome.Loupe.desktop".into(),
            target: "/home/ä/a b.png".into(),
        },
        Request::FileActions {
            path: "/home/ä/a b.png".into(),
        },
        Request::CopyFile {
            path: "/home/ä/a b.png".into(),
            paste: true,
        },
        Request::RunExecutable {
            path: "/home/ä/Tool.AppImage".into(),
            make_executable: true,
        },
        Request::SetWallpaper {
            path: "/home/ä/a b.png".into(),
        },
        Request::PreviewSnippet {
            id: "snp-0123456789ab".into(),
            arguments: vec![("name".into(), "Zoë".into())],
        },
        Request::ScriptIcons,
        Request::LocalStorageNamespaces,
        Request::LocalStorageItems {
            namespace: "@zoë/notes".into(),
        },
        Request::OAuthTokenSets,
        Request::RemoveOAuthTokenSet {
            extension_id: "github".into(),
            provider_id: Some("GitHub ✓".into()),
        },
        Request::RemoveOAuthTokenSet {
            extension_id: "linear".into(),
            provider_id: None,
        },
        Request::ShortcutCapture { capturing: true },
        Request::ShortcutCapture { capturing: false },
        Request::UpdateStatus,
        Request::SkipUpdate {
            tag: "v1.2.0-ü".into(),
        },
        Request::ExchangeRates,
        Request::RefreshExchangeRates,
        Request::ProbeShortcut {
            trigger: "super+shift+K".into(),
        },
        Request::ProbeShortcut {
            trigger: "ctrl+alt+é".into(),
        },
        Request::ExtensionAlertRemember { session: u64::MAX },
        Request::CopyText {
            text: "88.07 👍🏽 zoë".into(),
        },
        Request::LauncherShortcut { request: true },
        Request::LauncherShortcut { request: false },
        Request::LoginStart { enable: None },
        Request::LoginStart { enable: Some(true) },
        Request::LoginStart {
            enable: Some(false),
        },
        Request::FsQuery {
            query: "résumé".into(),
            limit: 10_000,
            category: Some("Documents".into()),
        },
        Request::ControlMediaPlayer {
            player: "org.mpris.MediaPlayer2.spotify".into(),
            action: compass_ipc::MediaPlayerAction::Next,
        },
        Request::ListFonts,
        Request::FontSpecimen {
            name: "Noto Sans ไทย".into(),
        },
        Request::ListRhaiScripts,
        Request::CreateExtension {
            author: "zoë".into(),
            title: "My Extension".into(),
            description: "Does something useful, promise".into(),
            location: "~/code".into(),
            command_title: "Search".into(),
            command_description: "Search things".into(),
            template: ":boilerplate/tmpl-list".into(),
        },
        Request::RunProgram {
            argv: vec!["htop".into(), "-d".into(), "é 5".into()],
            terminal: true,
            hold: false,
        },
    ]
}

/// Every `Response` variant.
fn store_entry() -> compass_ipc::StoreEntry {
    compass_ipc::StoreEntry {
        id: "store.vicinae.clock".into(),
        name: "clock".into(),
        author: "zoe".into(),
        author_name: "Zoë".into(),
        title: "Clock".into(),
        description: "Shows the time".into(),
        icon_light: Some("https://example.com/light.png".into()),
        icon_dark: None,
        downloads: "1.1K".into(),
        installed: true,
        update_available: true,
        compat: Some(1),
        author_avatar: Some("https://example.com/zoë.png".into()),
        owner: "zoe".into(),
    }
}

fn all_responses() -> Vec<Response> {
    vec![
        Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: 1,
        },
        Response::Pong {
            protocol_version: u16::MAX,
            pid: u32::MAX,
        },
        Response::Ack,
        Response::QueryResults { hits: vec![] },
        Response::QueryResults {
            hits: vec![
                QueryHit {
                    id: "app:firefox.desktop".into(),
                    title: "Firefox".into(),
                    subtitle: Some("Web Browser".into()),
                    score: 100,
                },
                QueryHit {
                    id: "x".into(),
                    title: "y".into(),
                    subtitle: None,
                    score: 0,
                },
            ],
        },
        Response::DoctorReport { checks: vec![] },
        Response::DoctorReport {
            checks: vec![
                DoctorCheck {
                    name: "portal.global-shortcuts".into(),
                    status: DoctorStatus::Ok,
                    detail: None,
                },
                DoctorCheck {
                    name: "shell-extension".into(),
                    status: DoctorStatus::Warn,
                    detail: Some("not installed; window management degraded".into()),
                },
                DoctorCheck {
                    name: "ipc.socket".into(),
                    status: DoctorStatus::Fail,
                    detail: Some("permission denied".into()),
                },
            ],
        },
        Response::ShuttingDown,
        Response::Error(ProtocolError::new(ErrorKind::VersionMismatch, "v2 vs v1")),
        Response::Error(ProtocolError::new(ErrorKind::Unsupported, "")),
        Response::Error(ProtocolError::new(ErrorKind::BadRequest, "empty query")),
        Response::Error(ProtocolError::new(ErrorKind::Internal, "handler panicked")),
        Response::WindowAttached,
        Response::Window(WindowCommand::Show),
        Response::Window(WindowCommand::Hide),
        Response::Window(WindowCommand::Toggle),
        Response::Window(WindowCommand::Dmenu(u64::MAX)),
        Response::Window(WindowCommand::Launch(u64::MAX)),
        Response::Window(WindowCommand::Deeplink(
            "vicinae://extensions/zoë/clock".into(),
        )),
        Response::Window(WindowCommand::Describe),
        Response::Window(WindowCommand::Hud {
            text: "Quit Fichiers ✓".into(),
            icon: Some("copy-clipboard".into()),
        }),
        Response::Window(WindowCommand::Hud {
            text: "Clipboard cleared".into(),
            icon: None,
        }),
        Response::Commands {
            commands: vec![compass_ipc::CommandInfo {
                id: "@zoë/notes:new".into(),
                name: "New Note ✍".into(),
            }],
        },
        Response::AppLaunched {
            focused_window_title: Some("Téléchargements — Files".into()),
        },
        Response::AppLaunched {
            focused_window_title: None,
        },
        Response::WindowState { open: true },
        Response::CalculatorHistory {
            groups: vec![compass_ipc::CalculatorGroup {
                name: "Pinned".into(),
                records: vec![compass_ipc::CalculatorRecord {
                    id: "ä-1".into(),
                    question: "2π".into(),
                    answer: "approx. 6.2831853072".into(),
                    conversion: false,
                    pinned: true,
                }],
            }],
        },
        Response::WindowManagerCapabilities(compass_ipc::WindowManagerCapabilities {
            workspaces: true,
            fullscreen: true,
            floating: false,
            overview: true,
        }),
        Response::Workspaces {
            workspaces: vec![compass_ipc::WorkspaceEntry {
                id: "3".into(),
                name: "müsic".into(),
                monitor: Some("HDMI-A-1".into()),
                window_count: 2,
                apps: vec![compass_ipc::WorkspaceApp {
                    name: "Spotify".into(),
                    icon: None,
                }],
                active: false,
            }],
        },
        Response::Openers {
            apps: vec![compass_ipc::OpenerEntry {
                id: "org.gnome.Loupe.desktop".into(),
                name: "Image Viewer".into(),
                icon: Some("org.gnome.Loupe".into()),
                default: true,
            }],
        },
        Response::LocalStorageNamespaces {
            namespaces: vec!["@zoë/notes".into(), "core".into()],
        },
        Response::LocalStorageItems {
            items: vec![compass_ipc::LocalStorageEntry {
                key: "draft ✍".into(),
                value: "{\"n\":1}".into(),
            }],
        },
        Response::ExchangeRates { rates: None },
        Response::ExchangeRates {
            rates: Some(compass_ipc::ExchangeRateTable {
                date: "2026-09-24".into(),
                fetched_at: 1_790_000_000,
                rates: vec![("EUR".into(), "1".into()), ("USD".into(), "1.1367".into())],
            }),
        },
        Response::OAuthTokenSets {
            sets: vec![compass_ipc::OAuthTokenSetEntry {
                extension_id: "github".into(),
                provider_id: Some("GitHub".into()),
                access_token: "gho_ä".into(),
                refresh_token: None,
                id_token: Some("eyJ".into()),
                scope: Some("repo read:user".into()),
                expires_at: Some(i64::MAX),
                expired: false,
            }],
        },
        Response::UpdateStatus {
            current: "v0.1.0".into(),
            available: Some(compass_ipc::UpdateOffer {
                tag: "v1.2.0".into(),
                version: "1.2.0".into(),
                release_url: "https://github.com/tuna-os/compass/releases/tag/v1.2.0".into(),
            }),
        },
        Response::UpdateStatus {
            current: String::new(),
            available: None,
        },
        Response::ShortcutProbe { refusal: None },
        Response::ShortcutProbe {
            refusal: Some("Already bound by the desktop ✗".into()),
        },
        Response::LauncherShortcut {
            state: compass_ipc::LauncherShortcutState::Granted {
                trigger: Some("Super+Space ✓".into()),
            },
        },
        Response::LauncherShortcut {
            state: compass_ipc::LauncherShortcutState::Conflict {
                reason: "Already taken by the desktop.".into(),
            },
        },
        Response::LauncherShortcut {
            state: compass_ipc::LauncherShortcutState::NotRequested,
        },
        Response::LoginStart {
            state: compass_ipc::LoginStartState::Portal,
            unit: Some(compass_ipc::LoginStartUnit::Enabled),
        },
        Response::LoginStart {
            state: compass_ipc::LoginStartState::Unavailable,
            unit: None,
        },
        Response::FileActions(compass_ipc::FileActionInfo {
            mime: Some("image/png".into()),
            has_opener: true,
            can_set_wallpaper: false,
            can_paste: true,
        }),
        Response::ScriptIcons {
            icons: vec![("hello.sh".into(), "icon://emoji/🎉".into())],
        },
        Response::AppRuntime {
            running: true,
            frontmost: false,
            windows: vec![compass_ipc::WindowInfo {
                id: 7,
                title: "Téléchargements".into(),
                wm_class: "org.gnome.Nautilus".into(),
                app_name: Some("Files".into()),
                app_icon: None,
                pid: Some(4242),
                workspace: Some(1),
                focused: false,
                can_close: true,
            }],
        },
        Response::TrayItems {
            items: vec![compass_ipc::TrayItemInfo {
                key: ":1.42/StatusNotifierItem".into(),
                title: "Réseau".into(),
                subtitle: "Connecté".into(),
                attention: true,
                has_menu: true,
                item_is_menu: false,
                icon_path: Some("/tmp/ä.png".into()),
                icon_name: Some("nm-signal-100".into()),
                icon_png: Some(vec![0x89, b'P', b'N', b'G']),
            }],
        },
        Response::TrayMenu {
            entries: vec![compass_ipc::TrayMenuEntry {
                id: -1,
                label: "Vitesse › Rapide".into(),
                toggled: Some(false),
                icon_name: None,
            }],
        },
        Response::CommandLaunch {
            id: "commands:search-files".into(),
            arguments_json: Some(r#"{"a":"ü"}"#.into()),
            fallback_text: Some("résumé".into()),
        },
        Response::ClipboardHistory { entries: vec![] },
        Response::ClipboardHistory {
            entries: vec![
                ClipboardEntry {
                    id: "a".into(),
                    preview: "hello é 🚀".into(),
                    mime_type: "text/plain;charset=utf-8".into(),
                    kind: ClipboardKind::Text,
                    pinned: true,
                    updated_at: i64::MAX,
                    url_host: None,
                },
                ClipboardEntry {
                    id: String::new(),
                    preview: "Image".into(),
                    mime_type: "image/png".into(),
                    kind: ClipboardKind::Image,
                    pinned: false,
                    updated_at: 0,
                    url_host: Some("example.org".into()),
                },
            ],
        },
        Response::ClipboardContent {
            mime_type: "text/plain".into(),
            data: "é 🚀".into(),
        },
        Response::ClipboardContent {
            mime_type: "image/png".into(),
            data: vec![0, 255, 0x89, b'P', b'N', b'G'],
        },
        Response::Windows { windows: vec![] },
        Response::Windows {
            windows: vec![compass_ipc::WindowInfo {
                id: u32::MAX,
                title: "Title é 🚀".into(),
                wm_class: "org.gnome.Nautilus".into(),
                app_name: Some("Files".into()),
                app_icon: None,
                pid: Some(1),
                workspace: Some(-1),
                focused: true,
                can_close: false,
            }],
        },
        Response::ExtensionStarted { session: 7 },
        Response::ExtensionNeedsArguments {
            title: "Search Repositories".into(),
            fields: vec![compass_ipc::PreferenceField {
                name: "query".into(),
                title: "Query".into(),
                description: String::new(),
                placeholder: "Query".into(),
                required: true,
                kind: compass_ipc::PreferenceFieldKind::Text,
                value_json: None,
            }],
        },
        Response::ExtensionNeedsPreferences {
            title: "Search Repositories".into(),
            fields: vec![
                compass_ipc::PreferenceField {
                    name: "token".into(),
                    title: "Token".into(),
                    description: String::new(),
                    placeholder: "ghp_…".into(),
                    required: true,
                    kind: compass_ipc::PreferenceFieldKind::Password,
                    value_json: None,
                },
                compass_ipc::PreferenceField {
                    name: "sort".into(),
                    title: "Sort".into(),
                    description: "Order".into(),
                    placeholder: String::new(),
                    required: false,
                    kind: compass_ipc::PreferenceFieldKind::Dropdown {
                        options: vec![("Stars".into(), "stars".into())],
                    },
                    value_json: Some("\"stars\"".into()),
                },
            ],
        },
        Response::ExtensionView {
            version: 3,
            view_json: Some("{\"kind\":\"list\"}".into()),
            problem: None,
            ended: false,
            depth: 2,
            alert: Some(compass_ipc::ExtensionAlert {
                title: "Delete é 🚀?".into(),
                message: String::new(),
                confirm_text: "Delete".into(),
                cancel_text: "Cancel".into(),
                remember_text: Some("Always".into()),
            }),
            toast: Some(compass_ipc::ExtensionToast {
                title: "Copied".into(),
                message: "to the clipboard".into(),
                style: compass_ipc::ExtensionToastStyle::Animated,
            }),
        },
        Response::ExtensionView {
            version: u64::MAX,
            view_json: None,
            problem: Some("Compass cannot draw the extension component <grid> yet".into()),
            ended: true,
            depth: 0,
            alert: None,
            toast: None,
        },
        Response::Files {
            heading: "Results".into(),
            files: vec![compass_ipc::FileHit {
                path: "/home/me/Documents/rapport é.pdf".into(),
                name: "rapport é.pdf".into(),
                category: "Documents".into(),
            }],
        },
        Response::Files {
            heading: "Recently Accessed".into(),
            files: vec![],
        },
        Response::Shortcuts {
            shortcuts: vec![compass_ipc::ShortcutEntry {
                id: "sct-0123456789ab".into(),
                name: "Recherche 🚀".into(),
                icon: "icon://favicon/x.test".into(),
                url: "https://x.test/?q={query}".into(),
                app: "default".into(),
                open_count: 3,
                created_at: 1_700_000_000,
                updated_at: 1_700_000_100,
                last_used_at: Some(1_700_000_200),
            }],
        },
        Response::Shortcuts { shortcuts: vec![] },
        Response::Text {
            text: "https://x.test/?q=é".into(),
        },
        Response::Snippets {
            snippets: vec![
                compass_ipc::SnippetEntry {
                    id: "snp-0123456789ab".into(),
                    name: "Signature ✍".into(),
                    text: Some("Best,\n{cursor}".into()),
                    file: None,
                    created_at: 1_700_000_000,
                    updated_at: Some(1_700_000_001),
                    keyword: Some(";sig".into()),
                    word: true,
                    apps: vec![],
                },
                compass_ipc::SnippetEntry {
                    id: "snp-ba9876543210".into(),
                    name: "Logo".into(),
                    text: None,
                    file: Some("/home/me/logo.png".into()),
                    created_at: 1_700_000_000,
                    updated_at: None,
                    keyword: None,
                    word: false,
                    apps: vec!["gimp.desktop".into()],
                },
            ],
        },
        Response::Snippets { snippets: vec![] },
        Response::Scripts {
            scripts: vec![compass_ipc::ScriptEntry {
                id: "tools.uptime.sh".into(),
                title: "Uptime ⏱".into(),
                subtitle: "tools".into(),
                keywords: vec!["load".into()],
                mode: "fullOutput".into(),
                needs_confirmation: true,
                path: "/home/me/.local/share/vicinae/scripts/tools/uptime.sh".into(),
                arguments: vec![compass_ipc::ScriptArgumentEntry {
                    kind: "dropdown".into(),
                    placeholder: Some("Unit".into()),
                    optional: false,
                    options: vec![("Seconds".into(), "s".into())],
                }],
            }],
        },
        Response::Scripts { scripts: vec![] },
        Response::ScriptStarted { session: Some(3) },
        Response::ScriptStarted { session: None },
        Response::ScriptOutput {
            output: "\u{1b}[31mred\u{1b}[0m https://x.test é".into(),
            finished: true,
            exit_code: Some(0),
            elapsed_ms: 1500,
        },
        Response::RhaiScripts {
            scripts: vec![compass_ipc::RhaiScriptEntry {
                id: "script.unit-converter".into(),
                title: "Unit Converter".into(),
                description: Some("Convert °C, km and kg".into()),
                icon: Some("calculator".into()),
                keywords: vec!["convert".into(), "单位".into()],
            }],
        },
        Response::RhaiScripts { scripts: vec![] },
        Response::Programs {
            programs: vec!["/usr/bin/htop".into(), "/opt/bin/ünï".into()],
            terminal: Some("Ptyxis".into()),
            default_action: "run-in-terminal".into(),
        },
        Response::ExtensionCreated {
            path: "/home/me/code/my-extension".into(),
        },
        Response::StoreListing {
            heading: "Extensions".into(),
            entries: vec![store_entry()],
        },
        Response::StoreExtension {
            detail: compass_ipc::StoreDetail {
                entry: store_entry(),
                markdown: "# Clock\n\nShows the time".into(),
                screenshots: vec!["https://example.com/1.png".into()],
                readme_url: Some("https://example.com/README.md".into()),
                source_url: None,
                store_url: Some("https://www.raycast.com/zoe/clock".into()),
            },
        },
        Response::StoreInstalled {
            id: "store.vicinae.clock".into(),
            title: "Clock".into(),
        },
        Response::InputServerStatus(compass_ipc::InputServerStatus {
            enabled: true,
            running: false,
            injection: false,
            keywords: 3,
            helper: Some("/usr/libexec/compass/compass-input-server".into()),
            problem: Some("inside a Flatpak: /dev/input is unreachable".into()),
        }),
        Response::InputServerStatus(compass_ipc::InputServerStatus::default()),
        Response::ExtensionLaunch {
            id: "@zoë/notes:create".into(),
            arguments_json: Some(r#"{"title":"é"}"#.into()),
            preferences: false,
        },
        Response::ExtensionLaunch {
            id: "@zoë/notes:list".into(),
            arguments_json: None,
            preferences: true,
        },
        Response::ExtensionSubtitles {
            subtitles: vec![("@zoë/notes:list".into(), "3 unread 🚀".into())],
        },
        Response::MediaPlayers {
            players: vec![compass_ipc::MediaPlayerEntry {
                id: "org.mpris.MediaPlayer2.spotify".into(),
                identity: "Spotify".into(),
                title: "Blue Monday".into(),
                artist: "New Order".into(),
                playing: true,
                can_go_next: true,
                ..Default::default()
            }],
        },
        Response::CatalogGeneration { generation: 3 },
        Response::DefaultApps {
            apps: vec![compass_ipc::DefaultAppEntry {
                id: "firefox.desktop".into(),
                name: "Firefox".into(),
                description: "Browse the Wörld Wide Web".into(),
                is_default: true,
            }],
        },
        Response::ScriptGrants {
            grants: vec![compass_ipc::ScriptGrantEntry {
                id: "script.quick-notes".into(),
                title: "Quick Notes".into(),
                capabilities: vec!["clipboard.write".into()],
                descriptions: vec!["copy to the clipboard".into()],
            }],
        },
        Response::ClipboardDetail {
            detail: compass_ipc::ClipboardDetail {
                id: "c0ffee".into(),
                mime_type: "text/plain;charset=utf-8".into(),
                kind: ClipboardKind::Text,
                size: 12,
                md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
                updated_at: 1_700_000_000_000,
                encrypted: true,
                keywords: "reçu".into(),
                pinned: false,
            },
        },
        Response::ClipboardMonitoring {
            supported: true,
            enabled: false,
        },
        Response::Fonts {
            fonts: vec![compass_ipc::FontEntry {
                name: "Noto Sans Thai".into(),
                family: "Noto Sans Thai".into(),
                glyph: Some("กข".into()),
                color: false,
                primary: "Thai".into(),
                categories: vec!["Thai".into(), "Latin".into()],
            }],
            categories: vec!["Latin".into(), "Thai".into()],
        },
        Response::DmenuOutput {
            output: "gamma 🚀".into(),
        },
        Response::DmenuOutput {
            output: String::new(),
        },
        Response::DmenuList {
            spec: compass_ipc::DmenuSpec {
                content: "a\nb".into(),
                ..compass_ipc::DmenuSpec::default()
            },
        },
        Response::Programs {
            programs: vec![],
            terminal: None,
            default_action: "run".into(),
        },
    ]
}

#[test]
fn request_variants_are_exhaustive() {
    // If a variant is added to `Request`, this stops compiling and whoever adds
    // it has to extend `all_requests`.
    for request in all_requests() {
        match request {
            Request::Ping
            | Request::Toggle
            | Request::Show
            | Request::Hide
            | Request::Query { .. }
            | Request::Doctor
            | Request::Shutdown
            | Request::AttachWindow
            | Request::RecordLaunch { .. }
            | Request::ClipboardHistory { .. }
            | Request::ClipboardContent { .. }
            | Request::ListWindows
            | Request::ActivateWindow { .. }
            | Request::CloseWindow { .. }
            | Request::ClipboardPaste { .. }
            | Request::ClipboardSetPinned { .. }
            | Request::ClipboardRemove { .. }
            | Request::RunExtensionCommand { .. }
            | Request::RunPowerCommand { .. }
            | Request::RunMediaCommand { .. }
            | Request::ExtensionView { .. }
            | Request::ExtensionEvent { .. }
            | Request::ExtensionPop { .. }
            | Request::SetExtensionPreferences { .. }
            | Request::ExtensionAlertAnswer { .. }
            | Request::CloseExtension { .. }
            | Request::SearchFiles { .. }
            | Request::OpenFile { .. }
            | Request::OAuthRedirect { .. }
            | Request::ListShortcuts
            | Request::SaveShortcut { .. }
            | Request::RemoveShortcut { .. }
            | Request::OpenShortcut { .. }
            | Request::ExpandShortcut { .. }
            | Request::ListSnippets
            | Request::SaveSnippet { .. }
            | Request::RemoveSnippet { .. }
            | Request::ExpandSnippet { .. }
            | Request::PasteSnippet { .. }
            | Request::ListScripts
            | Request::RunScript { .. }
            | Request::ScriptOutput { .. }
            | Request::StopScript { .. }
            | Request::ListPrograms
            | Request::RunProgram { .. }
            | Request::Dmenu { .. }
            | Request::DmenuFetch { .. }
            | Request::DmenuChoose { .. }
            | Request::SetTheme { .. }
            | Request::CreateExtension { .. }
            | Request::ListFonts
            | Request::FontSpecimen { .. }
            | Request::ListRhaiScripts
            | Request::StoreBrowse { .. }
            | Request::StoreExtension { .. }
            | Request::StoreInstall { .. }
            | Request::StoreUninstall { .. }
            | Request::OpenUrl { .. }
            | Request::InputServerStatus
            | Request::SetInputServerEnabled { .. }
            | Request::ExtensionLaunchFetch { .. }
            | Request::ExtensionSubtitles
            | Request::ExtensionPreferences { .. }
            | Request::RunMediaCommandWith { .. }
            | Request::ListMediaPlayers
            | Request::SetFont { .. }
            | Request::OpenDeeplink { .. }
            | Request::ListScriptGrants
            | Request::RevokeScriptGrant { .. }
            | Request::CatalogGeneration
            | Request::ListDefaultApps { .. }
            | Request::SetDefaultApp { .. }
            | Request::ClipboardHistoryOfKind { .. }
            | Request::ClipboardDetail { .. }
            | Request::ClipboardSetKeywords { .. }
            | Request::ClipboardRemoveAll
            | Request::ClipboardMonitoring { .. }
            | Request::RootItemEdit { .. }
            | Request::ControlMediaPlayer { .. }
            | Request::ListCommands
            | Request::LaunchCommand { .. }
            | Request::LaunchApp { .. }
            | Request::DescribeWindow
            | Request::FsQuery { .. }
            | Request::AppRuntime { .. }
            | Request::QuitApp { .. }
            | Request::QuitWindowApp { .. }
            | Request::CalculatorHistory { .. }
            | Request::AddCalculatorRecord { .. }
            | Request::EditCalculatorHistory { .. }
            | Request::TrayItems
            | Request::TrayActivate { .. }
            | Request::TrayMenu { .. }
            | Request::TrayTriggerMenu { .. }
            | Request::PasteText { .. }
            | Request::WindowManagerCapabilities
            | Request::ListWorkspaces
            | Request::FocusWorkspace { .. }
            | Request::ToggleWindowState { .. }
            | Request::ListOpeners { .. }
            | Request::OpenWith { .. }
            | Request::FileActions { .. }
            | Request::CopyFile { .. }
            | Request::RunExecutable { .. }
            | Request::SetWallpaper { .. }
            | Request::PreviewSnippet { .. }
            | Request::ScriptIcons
            | Request::SetSetting { .. }
            | Request::SetProviderEnabled { .. }
            | Request::LocalStorageNamespaces
            | Request::LocalStorageItems { .. }
            | Request::OAuthTokenSets
            | Request::RemoveOAuthTokenSet { .. }
            | Request::ShortcutCapture { .. }
            | Request::UpdateStatus
            | Request::SkipUpdate { .. }
            | Request::ExchangeRates
            | Request::RefreshExchangeRates
            | Request::ProbeShortcut { .. }
            | Request::ExtensionAlertRemember { .. }
            | Request::CopyText { .. }
            | Request::LauncherShortcut { .. }
            | Request::LoginStart { .. }
            | Request::WindowOutcome(_) => {}
        }
    }
}

#[test]
fn response_variants_are_exhaustive() {
    for response in all_responses() {
        match response {
            Response::Pong { .. }
            | Response::Ack
            | Response::QueryResults { .. }
            | Response::DoctorReport { .. }
            | Response::ShuttingDown
            | Response::Error(_)
            | Response::WindowAttached
            | Response::ClipboardHistory { .. }
            | Response::ClipboardContent { .. }
            | Response::Windows { .. }
            | Response::ExtensionStarted { .. }
            | Response::ExtensionNeedsPreferences { .. }
            | Response::ExtensionNeedsArguments { .. }
            | Response::ExtensionView { .. }
            | Response::Files { .. }
            | Response::Shortcuts { .. }
            | Response::Text { .. }
            | Response::Snippets { .. }
            | Response::Scripts { .. }
            | Response::ScriptStarted { .. }
            | Response::ScriptOutput { .. }
            | Response::Programs { .. }
            | Response::ExtensionCreated { .. }
            | Response::Fonts { .. }
            | Response::StoreListing { .. }
            | Response::StoreExtension { .. }
            | Response::StoreInstalled { .. }
            | Response::MediaPlayers { .. }
            | Response::ScriptGrants { .. }
            | Response::CatalogGeneration { .. }
            | Response::DefaultApps { .. }
            | Response::ClipboardDetail { .. }
            | Response::ClipboardMonitoring { .. }
            | Response::DmenuOutput { .. }
            | Response::DmenuList { .. }
            | Response::RhaiScripts { .. }
            | Response::InputServerStatus(_)
            | Response::ExtensionLaunch { .. }
            | Response::ExtensionSubtitles { .. }
            | Response::Commands { .. }
            | Response::AppLaunched { .. }
            | Response::WindowState { .. }
            | Response::CommandLaunch { .. }
            | Response::AppRuntime { .. }
            | Response::CalculatorHistory { .. }
            | Response::TrayItems { .. }
            | Response::TrayMenu { .. }
            | Response::WindowManagerCapabilities(_)
            | Response::Workspaces { .. }
            | Response::Openers { .. }
            | Response::FileActions(_)
            | Response::ScriptIcons { .. }
            | Response::LocalStorageNamespaces { .. }
            | Response::LocalStorageItems { .. }
            | Response::OAuthTokenSets { .. }
            | Response::UpdateStatus { .. }
            | Response::ExchangeRates { .. }
            | Response::ShortcutProbe { .. }
            | Response::LauncherShortcut { .. }
            | Response::LoginStart { .. }
            | Response::Window(_) => {}
        }
    }
}

#[test]
fn every_request_variant_round_trips() {
    let mut codec = FrameCodec::<RequestEnvelope>::new();

    for (id, request) in all_requests().into_iter().enumerate() {
        let envelope = RequestEnvelope::new(id as u64, request);
        let mut buf = BytesMut::new();
        codec.encode(&envelope, &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap();
        assert_eq!(decoded, Some(envelope));
        assert!(buf.is_empty(), "codec left trailing bytes");
    }
}

#[test]
fn every_response_variant_round_trips() {
    let mut codec = FrameCodec::<ResponseEnvelope>::new();

    for (id, response) in all_responses().into_iter().enumerate() {
        let envelope = ResponseEnvelope::new(id as u64, response);
        let mut buf = BytesMut::new();
        codec.encode(&envelope, &mut buf).unwrap();

        assert_eq!(codec.decode(&mut buf).unwrap(), Some(envelope));
        assert!(buf.is_empty());
    }
}

#[test]
fn an_extension_view_of_several_mebibytes_is_carried() {
    // Suite 1's dashboard-icons draws 4,473 grid items: more than four
    // mebibytes of view, which the old one-mebibyte limit refused.
    let view_json = format!("[{}]", "{\"title\":\"an icon\"},".repeat(300_000));
    assert!(view_json.len() > 4 * 1024 * 1024);
    let envelope = ResponseEnvelope::new(
        1,
        Response::ExtensionView {
            version: 2,
            view_json: Some(view_json),
            problem: None,
            ended: false,
            depth: 1,
            alert: None,
            toast: None,
        },
    );
    let mut codec = FrameCodec::<ResponseEnvelope>::new();
    let mut buf = BytesMut::new();
    codec.encode(&envelope, &mut buf).expect("encoded");
    assert_eq!(codec.decode(&mut buf).expect("decoded"), Some(envelope));
}

#[test]
fn all_variants_round_trip_back_to_back_in_one_stream() {
    let mut codec = FrameCodec::<RequestEnvelope>::new();
    let mut buf = BytesMut::new();

    let envelopes: Vec<_> = all_requests()
        .into_iter()
        .enumerate()
        .map(|(i, r)| RequestEnvelope::new(i as u64, r))
        .collect();

    for envelope in &envelopes {
        codec.encode(envelope, &mut buf).unwrap();
    }

    for expected in &envelopes {
        assert_eq!(codec.decode(&mut buf).unwrap().as_ref(), Some(expected));
    }
    assert_eq!(codec.decode(&mut buf).unwrap(), None);
}

#[test]
fn a_frame_fed_one_byte_at_a_time_yields_exactly_one_message_at_the_end() {
    let envelope = RequestEnvelope::new(
        42,
        Request::Query {
            text: "a moderately long query".into(),
        },
    );
    let mut wire = BytesMut::new();
    FrameCodec::<RequestEnvelope>::new()
        .encode(&envelope, &mut wire)
        .unwrap();
    let wire = wire.freeze();
    assert!(wire.len() > LENGTH_PREFIX_LEN);

    let mut codec = FrameCodec::<RequestEnvelope>::new();
    let mut buf = BytesMut::new();
    let mut yielded = Vec::new();

    for (index, byte) in wire.iter().enumerate() {
        buf.put_u8(*byte);
        let decoded = codec.decode(&mut buf).unwrap();

        if index + 1 < wire.len() {
            assert!(
                decoded.is_none(),
                "decoder produced a message after only {} bytes",
                index + 1
            );
        }
        if let Some(item) = decoded {
            yielded.push(item);
        }
    }

    assert_eq!(yielded, vec![envelope]);
    assert!(buf.is_empty());
    assert_eq!(codec.decode(&mut buf).unwrap(), None);
}

#[test]
fn two_frames_fed_one_byte_at_a_time_yield_two_messages_in_order() {
    let a = RequestEnvelope::new(1, Request::Toggle);
    let b = RequestEnvelope::new(
        2,
        Request::Query {
            text: "second".into(),
        },
    );

    let mut wire = BytesMut::new();
    let mut codec = FrameCodec::<RequestEnvelope>::new();
    codec.encode(&a, &mut wire).unwrap();
    let boundary = wire.len();
    codec.encode(&b, &mut wire).unwrap();
    let wire = wire.freeze();

    let mut buf = BytesMut::new();
    let mut yielded = Vec::new();
    for (index, byte) in wire.iter().enumerate() {
        buf.put_u8(*byte);
        if let Some(item) = codec.decode(&mut buf).unwrap() {
            // The only two byte offsets that may complete a message are the
            // last byte of each frame.
            assert!(index + 1 == boundary || index + 1 == wire.len());
            yielded.push(item);
        }
    }

    assert_eq!(yielded, vec![a, b]);
}

#[test]
fn a_truncated_frame_never_yields_a_message() {
    let envelope = RequestEnvelope::new(
        7,
        Request::Query {
            text: "truncated".into(),
        },
    );
    let mut wire = BytesMut::new();
    FrameCodec::<RequestEnvelope>::new()
        .encode(&envelope, &mut wire)
        .unwrap();

    // Drop the final byte: the decoder must wait forever rather than guess.
    let truncated = &wire[..wire.len() - 1];
    let mut buf = BytesMut::from(truncated);
    let mut codec = FrameCodec::<RequestEnvelope>::new();

    assert_eq!(codec.decode(&mut buf).unwrap(), None);
    assert_eq!(codec.decode(&mut buf).unwrap(), None);
    // Nothing was lost while waiting: the missing byte completes the frame.
    buf.put_u8(wire[wire.len() - 1]);
    assert_eq!(codec.decode(&mut buf).unwrap(), Some(envelope));
}

#[test]
fn an_oversized_length_prefix_is_rejected_without_allocating() {
    for announced in [MAX_FRAME_LEN as u32 + 1, u32::MAX, u32::MAX - 1, 1 << 30] {
        let mut buf = BytesMut::with_capacity(LENGTH_PREFIX_LEN);
        buf.put_u32_le(announced);
        let capacity_before = buf.capacity();

        let err = FrameCodec::<RequestEnvelope>::new()
            .decode(&mut buf)
            .unwrap_err();

        match err {
            Error::FrameTooLarge { len, max } => {
                assert_eq!(len, announced as usize);
                assert_eq!(max, MAX_FRAME_LEN);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }

        // The whole point: four bytes from a peer must not turn into a
        // multi-gigabyte reservation.
        assert_eq!(buf.capacity(), capacity_before);
        assert!(buf.capacity() <= LENGTH_PREFIX_LEN * 2);
    }
}

#[test]
fn the_size_limit_is_configurable_and_enforced_on_both_sides() {
    let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(32);
    assert_eq!(codec.max_frame_len(), 32);

    let big = RequestEnvelope::new(
        1,
        Request::Query {
            text: "z".repeat(1000),
        },
    );
    let mut buf = BytesMut::new();
    assert!(matches!(
        codec.encode(&big, &mut buf).unwrap_err(),
        Error::FrameTooLarge { max: 32, .. }
    ));

    let mut wire = BytesMut::new();
    wire.put_u32_le(33);
    assert!(matches!(
        codec.decode(&mut wire).unwrap_err(),
        Error::FrameTooLarge { len: 33, max: 32 }
    ));
}

#[test]
fn a_well_framed_but_nonsense_body_is_an_error_not_a_panic() {
    for body in [vec![0xff, 0xff, 0xff, 0xff], vec![0x00], vec![0x7f; 12]] {
        let mut buf = BytesMut::new();
        buf.put_u32_le(body.len() as u32);
        buf.put_slice(&body);

        // Either it decodes to something (postcard is not self-describing, so
        // some byte strings are valid) or it errors. It must never panic and it
        // must always consume the frame.
        let result = FrameCodec::<RequestEnvelope>::new().decode(&mut buf);
        assert!(buf.is_empty(), "the frame body should have been consumed");
        let _ = result;
    }
}

#[test]
fn error_messages_are_legible() {
    let err = Error::FrameTooLarge {
        len: 50_000_000,
        max: MAX_FRAME_LEN,
    };
    assert_eq!(
        err.to_string(),
        "frame of 50000000 bytes exceeds the 33554432 byte limit"
    );

    let err = Error::VersionMismatch {
        expected: 1,
        actual: 9,
    };
    assert!(err.to_string().contains("peer speaks v9"));
    assert!(err.to_string().contains("this build speaks v1"));
}
