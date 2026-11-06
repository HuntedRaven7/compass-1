//! Versioned request/response types carried over the socket.
//!
//! Every frame on the wire is one [`RequestEnvelope`] (client to server) or one
//! [`ResponseEnvelope`] (server to client). Both carry the [`PROTOCOL_VERSION`]
//! they were produced with, so an old `compass` CLI meeting a new engine — or
//! the reverse — gets a clear [`ErrorKind::VersionMismatch`] instead of a
//! postcard decode failure or, worse, a silent misinterpretation.
//!
//! Postcard is **not** self-describing: field order and variant order *are* the
//! schema. Adding a variant in the middle of [`Request`] or [`Response`], or
//! reordering fields, is a breaking change. Append new variants at the end and
//! bump [`PROTOCOL_VERSION`] whenever the meaning of existing bytes changes.

use serde::{Deserialize, Serialize};

/// Version of the request/response schema understood by this build.
///
/// Bumped whenever the postcard encoding of [`Request`] or [`Response`]
/// changes in a way that an older peer would misread.
///
/// # Why v2 bumped for appended variants
///
/// Appending a variant does not change the meaning of any byte a v1 peer can
/// produce, so by the rule above it looks like it should not need a bump. It
/// does, because of the direction the new variants travel: a v2 window sends
/// [`Request::AttachWindow`], and a **v1 engine** decoding it would not get a
/// clean refusal — the variant index is past the end of its `Request` enum, so
/// it gets a postcard decode error and drops the connection with no
/// explanation. The version field exists precisely to turn that into a sentence
/// a human can act on, and it only does so if the number moves.
///
/// Version 3 adds successful-launch reporting to the daemon-owned history.
/// Version 4 adds clipboard history; version 5, fetching an entry's content;
/// version 6, window switching; version 7, pasting, pinning and removing a
/// clipboard entry, and running an installed extension's command; version 8,
/// following and driving an extension's view; version 9, an extension view's
/// toast; version 10, the power and media commands; version 11, file search;
/// version 12, an OAuth provider's redirect back to the launcher; version 13,
/// shortcuts, snippets, script commands, Run Terminal Program, dmenu, themes,
/// create-extension and fonts; version 14, Rhai scripts and the extension stores;
/// version 15, the input server and the last extension host routes: the
/// snippet keyword expander's input server ([`Request::InputServerStatus`],
/// [`Request::SetInputServerEnabled`]), an extension launching another command
/// or opening its preferences in the launcher ([`WindowCommand::Launch`],
/// [`Request::ExtensionLaunchFetch`]), its subtitle override in root search
/// ([`Request::ExtensionSubtitles`]), and a command's preferences form without
/// running it ([`Request::ExtensionPreferences`]); version 16, media arguments,
/// Now Playing, the launcher's font, store avatars, extension deeplinks and
/// reviewing Rhai script permissions; version 17, the catalog generation a
/// window compares to know that applications or extensions were installed or
/// removed while it ran ([`Request::CatalogGeneration`]), the default
/// browser and terminal pickers ([`Request::ListDefaultApps`],
/// [`Request::SetDefaultApp`]), and clipboard history's kind filter, detail
/// pane, keywords, remove-all and monitoring switch
/// ([`Request::ClipboardHistoryOfKind`], [`Request::ClipboardDetail`],
/// [`Request::ClipboardSetKeywords`], [`Request::ClipboardRemoveAll`],
/// [`Request::ClipboardMonitoring`]), the root row's favourite, alias, disable
/// and reset-ranking actions ([`Request::RootItemEdit`]), and the rest of
/// the C++ CLI's requests: listing and launching root commands
/// ([`Request::ListCommands`], [`Request::LaunchCommand`]), launching or
/// focusing an application ([`Request::LaunchApp`]), whether the window is
/// open ([`Request::DescribeWindow`], [`WindowCommand::Describe`]) and the
/// file index's own query ([`Request::FsQuery`]); and Quit and Force Quit
/// for a running application ([`Request::AppRuntime`], [`Request::QuitApp`],
/// [`Request::QuitWindowApp`]); and the calculator's history
/// ([`Request::CalculatorHistory`], [`Request::AddCalculatorRecord`],
/// [`Request::EditCalculatorHistory`]); version 18, other applications'
/// tray icons ([`Request::TrayItems`], [`Request::TrayActivate`],
/// [`Request::TrayMenu`], [`Request::TrayTriggerMenu`]), a root item's
/// keyboard shortcut ([`RootItemEdit::Shortcut`]), pasting text the engine
/// did not store ([`Request::PasteText`]), the window-management commands'
/// capabilities, workspaces and toggles
/// ([`Request::WindowManagerCapabilities`], [`Request::ListWorkspaces`],
/// [`Request::FocusWorkspace`], [`Request::ToggleWindowState`]), "Open
/// with…" ([`Request::ListOpeners`], [`Request::OpenWith`]) and a file's
/// action panel ([`Request::FileActions`], [`Request::CopyFile`],
/// [`Request::RunExecutable`], [`Request::SetWallpaper`]); version 19,
/// Manage Snippets' detail pane and script commands' icons in root search
/// ([`Request::PreviewSnippet`], [`Request::ScriptIcons`]), and the settings
/// view's writes: one setting of `compass.json` ([`Request::SetSetting`]),
/// a provider's switch ([`Request::SetProviderEnabled`]) and turning a root
/// item back on ([`RootItemEdit::Enabled`]), and the HUD the engine asks the
/// window to show ([`WindowCommand::Hud`]), and the fallback manager's switch
/// ([`RootItemEdit::Fallback`]), and Inspect Local Storage's and Manage OAuth
/// Token Sets' reads ([`Request::LocalStorageNamespaces`],
/// [`Request::LocalStorageItems`], [`Request::OAuthTokenSets`],
/// [`Request::RemoveOAuthTokenSet`]); version 20, the shortcut recorder
/// suspending the global shortcuts while it captures
/// ([`Request::ShortcutCapture`]); version 21, the update check: whether a
/// newer Compass release is out ([`Request::UpdateStatus`],
/// [`Response::UpdateStatus`]) and skipping it ([`Request::SkipUpdate`]), and currency conversion's exchange rates
/// ([`Request::ExchangeRates`], [`Request::RefreshExchangeRates`], both
/// answered with [`Response::ExchangeRates`]), and the recorder asking whether the
/// desktop would bind a combination ([`Request::ProbeShortcut`],
/// [`Response::ShortcutProbe`]); version 22, an extension view's alert with a
/// third, remembered answer ([`ExtensionAlert::remember_text`],
/// [`Request::ExtensionAlertRemember`]), which the engine's consent to run a
/// host program for an extension is asked with, and those grants listed and
/// revoked beside the Rhai scripts' ([`Request::ListScriptGrants`],
/// [`Request::RevokeScriptGrant`]); version 23, a store extension's owner
/// beside its author ([`StoreEntry::owner`]), which the Raycast store keys an
/// organisation's extension by, and which [`Request::StoreExtension`] and
/// [`Request::StoreInstall`] now carry, and a no-view command's failure
/// said in the launcher ([`WindowCommand::Failure`]); version 24, copying text
/// on the engine's long-lived clipboard rather than the window's, which goes
/// when the window hides ([`Request::CopyText`]); version 25, the first-run
/// flow's launcher hotkey and login start ([`Request::LauncherShortcut`],
/// [`Request::LoginStart`]); version 26, how clipboard history captures and
/// stores copies, and why it does not ([`Request::ClipboardStatus`],
/// [`Response::ClipboardStatus`]).
pub const PROTOCOL_VERSION: u16 = 26;

/// How long the engine holds an [`Request::ExtensionView`] open waiting for a
/// change before it answers with the version it has.
///
/// A client must allow longer than this for the answer, or a view that is
/// simply idle reads as a failure: [`long_poll_deadline`] says how much
/// longer.
pub const EXTENSION_VIEW_HOLD: std::time::Duration = std::time::Duration::from_secs(20);

/// What a client adds to a request the engine may hold, for the engine's own
/// work and the round trip.
pub const LONG_POLL_MARGIN: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a client waits for a request the engine holds for up to `hold`.
#[must_use]
pub const fn long_poll_deadline(hold: std::time::Duration) -> std::time::Duration {
    hold.saturating_add(LONG_POLL_MARGIN)
}

/// A client-to-server frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    /// Protocol version the sender speaks. See [`PROTOCOL_VERSION`].
    pub version: u16,
    /// Correlation id, echoed in the matching [`ResponseEnvelope`].
    ///
    /// The current transport is strictly request/response per connection, but
    /// the id is on the wire from day one so pipelining does not need a
    /// protocol bump.
    pub id: u64,
    /// The request itself.
    pub request: Request,
}

impl RequestEnvelope {
    /// Wraps `request` at the current [`PROTOCOL_VERSION`].
    #[must_use]
    pub fn new(id: u64, request: Request) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            request,
        }
    }
}

/// A server-to-client frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    /// Protocol version the sender speaks. See [`PROTOCOL_VERSION`].
    pub version: u16,
    /// Correlation id copied from the request this answers.
    ///
    /// Zero when the server could not decode a request far enough to know it.
    pub id: u64,
    /// The response itself.
    pub response: Response,
}

impl ResponseEnvelope {
    /// Wraps `response` at the current [`PROTOCOL_VERSION`].
    #[must_use]
    pub fn new(id: u64, response: Response) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            response,
        }
    }
}

/// What a client can ask the engine to do.
///
/// Deliberately small: this is the Phase 2 surface (socket, CLI, doctor) and
/// nothing more. New variants go at the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Liveness probe. Answered with [`Response::Pong`].
    Ping,
    /// Toggle the launcher window between shown and hidden.
    Toggle,
    /// Show the launcher window.
    Show,
    /// Hide the launcher window.
    Hide,
    /// Run a root search and return the ranked hits.
    Query {
        /// Raw search text, exactly as typed.
        text: String,
    },
    /// Run the self-diagnostic checks behind `compass doctor`.
    Doctor,
    /// Ask the engine to shut down cleanly.
    Shutdown,
    /// Offer this connection as *the* launcher window.
    ///
    /// Answered with [`Response::WindowAttached`], after which the connection
    /// reverses: the engine pushes [`Response::Window`] frames and the window
    /// answers each with [`Request::WindowOutcome`]. See
    /// [`crate::transport::WindowLink`].
    AttachWindow,
    /// A window's answer to one pushed [`WindowCommand`].
    ///
    /// Only legal on an attached connection, where it is a *reply* rather than
    /// a request; the engine never sends a [`Response`] back to it.
    WindowOutcome(WindowOutcome),
    /// Record a successfully completed launch of an indexed item.
    ///
    /// Reports an outcome; it does not launch an application. The daemon
    /// validates the key and updates its history before acknowledging it.
    RecordLaunch {
        /// Stable application/action key, as returned by the index.
        key: String,
    },
    /// Search clipboard history, newest first with pinned entries on top.
    ///
    /// Answered with [`Response::ClipboardHistory`], or with an
    /// [`ErrorKind::Unsupported`] error while the engine has no store — no
    /// keyring, or the store would not open. An empty `query` lists.
    ClipboardHistory {
        /// Search text; empty for the whole history.
        query: String,
        /// Most entries to return. Zero is a bad request.
        limit: u32,
    },
    /// The full content of one clipboard history entry, decrypted.
    ///
    /// Answered with [`Response::ClipboardContent`]; an id that names no entry
    /// is a bad request. Separate from the list because a list row needs only
    /// the preview, and content can be a whole image.
    ClipboardContent {
        /// [`ClipboardEntry::id`].
        id: String,
    },
    /// The open windows, for the window switcher.
    ///
    /// Answered with [`Response::Windows`], or refused as
    /// [`ErrorKind::Unsupported`] without the GNOME Shell extension, which is
    /// the only way to list windows on GNOME.
    ListWindows,
    /// Focus and raise one window. Answered with [`Response::Ack`].
    ActivateWindow {
        /// [`WindowInfo::id`].
        id: u32,
    },
    /// Ask one window to close. Answered with [`Response::Ack`].
    CloseWindow {
        /// [`WindowInfo::id`].
        id: u32,
    },
    /// Put one clipboard history entry on the clipboard and paste it into the
    /// window focus moves to next.
    ///
    /// Send it while the launcher still has focus and hide the launcher once
    /// it is answered with [`Response::Ack`]: the paste lands after the focus
    /// change. Refused as [`ErrorKind::Unsupported`] without the GNOME Shell
    /// extension, which is the only thing on GNOME that can press a key in
    /// another window; the caller then copies instead.
    ClipboardPaste {
        /// [`ClipboardEntry::id`].
        id: String,
    },
    /// Pin or unpin one clipboard history entry. Pinned entries list first and
    /// survive eviction. Answered with [`Response::Ack`]; an id that names no
    /// entry is a bad request.
    ClipboardSetPinned {
        /// [`ClipboardEntry::id`].
        id: String,
        /// Pin when true, unpin when false.
        pinned: bool,
    },
    /// Remove one clipboard history entry and its stored content. Answered
    /// with [`Response::Ack`]; an id that names no entry is a bad request.
    ClipboardRemove {
        /// [`ClipboardEntry::id`].
        id: String,
    },
    /// Run a Power Management command. Answered with [`Response::Ack`] once
    /// logind (or the desktop's session manager) accepted it; refused as
    /// [`ErrorKind::Unsupported`] with the command's own "cannot" sentence
    /// when the system reports it cannot, and as [`ErrorKind::Internal`] with
    /// its "failed" sentence when the attempt fails.
    RunPowerCommand {
        /// The command's id in `compass_core::power_commands`, e.g. `reboot`.
        id: String,
    },
    /// Run a media command (`play-pause`, `next-track`, `previous-track`) on
    /// the default player. Answered with [`Response::Ack`] once the player
    /// took it; refused as [`ErrorKind::Unsupported`] with the sentence to
    /// show when no player is running or it cannot skip, and as
    /// [`ErrorKind::Internal`] when the player fails.
    RunMediaCommand {
        /// The command's id in `compass_core::media_commands`.
        id: String,
    },
    /// Run an installed extension's command, by the entrypoint id a
    /// [`QueryHit`] carries. Answered with [`Response::Ack`] once it has
    /// started; refused as [`ErrorKind::Unsupported`], with the reason, when
    /// this engine cannot run it (a view command, a preference it cannot
    /// fill, no extension runtime).
    RunExtensionCommand {
        /// [`QueryHit::id`].
        id: String,
        /// The command's argument values as a JSON object, or `None` when the
        /// launcher has none to give: a command that declares arguments is
        /// then answered with [`Response::ExtensionNeedsArguments`].
        arguments_json: Option<String>,
    },
    /// What a view command's session shows, once it differs from `after`.
    ///
    /// Held open until the session's version passes `after` or
    /// [`EXTENSION_VIEW_HOLD`] passes, then answered with [`Response::ExtensionView`] either way; the
    /// launcher asks again with the version it got. A session that is not
    /// running is a bad request.
    ExtensionView {
        /// From [`Response::ExtensionStarted`].
        session: u64,
        /// The last version the launcher has; zero for none.
        after: u64,
    },
    /// Run one of the view's callbacks: an action's handler, the search bar's
    /// change handler, the selection handler. Answered with [`Response::Ack`].
    ExtensionEvent {
        /// From [`Response::ExtensionStarted`].
        session: u64,
        /// The handler id the view carries.
        handler: String,
        /// Its arguments, as a JSON array.
        args_json: String,
    },
    /// Keeps an extension's preference values, then answers [`Response::Ack`].
    /// Sent after [`Response::ExtensionNeedsPreferences`], before running the
    /// command again.
    SetExtensionPreferences {
        /// The command's [`QueryHit::id`]; the values are its extension's.
        id: String,
        /// The values, as a JSON object by preference name.
        values_json: String,
    },
    /// The person's answer to the view's [`ExtensionAlert`]. Answered with
    /// [`Response::Ack`]; a session with no alert waiting is a bad request.
    ExtensionAlertAnswer {
        /// From [`Response::ExtensionStarted`].
        session: u64,
        /// Whether they confirmed.
        confirmed: bool,
    },
    /// Escape on a pushed view: pop it, and the extension renders the view
    /// beneath. Answered with [`Response::Ack`].
    ExtensionPop {
        /// From [`Response::ExtensionStarted`].
        session: u64,
    },
    /// The person left the view: stop the command. Answered with
    /// [`Response::Ack`].
    CloseExtension {
        /// From [`Response::ExtensionStarted`].
        session: u64,
    },
    /// Search Files: recently accessed files for an empty query, the path
    /// itself for a query naming one that exists, and the file index
    /// otherwise. Answered with [`Response::Files`]; an index search while
    /// the file indexer is not running is refused as
    /// [`ErrorKind::Unsupported`].
    SearchFiles {
        /// Search text, exactly as typed.
        query: String,
        /// Only files of this category: a filter key such as `Images`, as
        /// `compass_core::file_search::CATEGORY_FILTER_KEYS` spells it.
        /// `None` (or `All`) filters nothing.
        category: Option<String>,
    },
    /// Open one file with its default application, or show it in the file
    /// browser. Answered with [`Response::Ack`] once the launch started; a
    /// file with no application to open it is refused as
    /// [`ErrorKind::Unsupported`], and a path that does not exist as
    /// [`ErrorKind::BadRequest`].
    OpenFile {
        /// Absolute path, as [`FileHit::path`] carries it.
        path: String,
        /// Show the file in the file browser instead of opening it.
        reveal: bool,
    },
    /// An OAuth provider redirected back to the launcher: the
    /// `raycast://oauth?code=…&state=…` deeplink (or its `com.raycast:` and
    /// `vicinae:` spellings) the desktop handed `compass`. The engine answers
    /// the extension's `OAuth/authorize` whose URL carried that `state`.
    /// [`Response::Ack`] once it has; [`ErrorKind::BadRequest`] when the URL
    /// is not an OAuth redirect or no authorization is waiting on its state.
    OAuthRedirect {
        /// The deeplink, verbatim.
        url: String,
    },
    /// Every stored shortcut (quicklink), answered with
    /// [`Response::Shortcuts`].
    ListShortcuts,
    /// Create a shortcut (`id` is `None`) or update one in place. Answered
    /// with [`Response::Shortcuts`], the list after the change; a store that
    /// could not be written, or an `id` that names nothing, is refused as
    /// [`ErrorKind::Internal`] and [`ErrorKind::BadRequest`].
    SaveShortcut {
        /// The shortcut to update, or `None` for a new one.
        id: Option<String>,
        /// What it is called; may be empty.
        name: String,
        /// Its icon as an image URL, or `default` for whatever the link's
        /// opener or site offers, which the engine resolves when saving.
        icon: String,
        /// The link, `{placeholders}` and all.
        url: String,
        /// The application id that opens it, or `default`.
        app: String,
    },
    /// Remove a shortcut. Answered with [`Response::Shortcuts`].
    RemoveShortcut {
        /// Which one.
        id: String,
    },
    /// Expand a shortcut's link with `arguments` and open it with its
    /// application, counting the visit. Answered with [`Response::Ack`] once
    /// the launch started; a shortcut with no application to open it is
    /// refused as [`ErrorKind::Unsupported`].
    OpenShortcut {
        /// Which one.
        id: String,
        /// Values for its argument placeholders, in order.
        arguments: Vec<String>,
    },
    /// Expand a shortcut's link without opening it. Answered with
    /// [`Response::Text`].
    ExpandShortcut {
        /// Which one.
        id: String,
        /// Values for its argument placeholders, in order.
        arguments: Vec<String>,
    },
    /// Every stored snippet, answered with [`Response::Snippets`].
    ListSnippets,
    /// Create a text snippet (`id` is `None`) or update one. Answered with
    /// [`Response::Snippets`], the list after the change; a form that does not
    /// validate, or a keyword another snippet has, is refused as
    /// [`ErrorKind::BadRequest`] with the reason.
    SaveSnippet {
        /// The snippet to update, or `None` for a new one.
        id: Option<String>,
        /// Its name, two characters at least.
        name: String,
        /// Its text, `{placeholders}` and all.
        text: String,
        /// The keyword that expands it as it is typed, if any.
        keyword: Option<String>,
        /// Whether the keyword waits for a word boundary.
        word: bool,
        /// The applications the keyword is limited to; empty for everywhere.
        apps: Vec<String>,
    },
    /// Remove a snippet. Answered with [`Response::Snippets`].
    RemoveSnippet {
        /// Which one.
        id: String,
    },
    /// Expand a snippet with its arguments, running its `{shell}`
    /// placeholders. Answered with [`Response::Text`]; a file snippet
    /// answers with its path.
    ExpandSnippet {
        /// Which one.
        id: String,
        /// `(name, value)` for its arguments.
        arguments: Vec<(String, String)>,
    },
    /// Expand a snippet and paste it into the focused window. Answered with
    /// [`Response::Ack`] once pasted.
    PasteSnippet {
        /// Which one.
        id: String,
        /// `(name, value)` for its arguments.
        arguments: Vec<(String, String)>,
    },
    /// Every script command in the script directories, scanned afresh.
    /// Answered with [`Response::Scripts`].
    ListScripts,
    /// Run a script command with its arguments, in the mode its header
    /// declares. Answered with [`Response::ScriptStarted`]: a session to
    /// follow with [`Request::ScriptOutput`] for `fullOutput`, `compact` and
    /// `inline`, none for `silent` (the engine shows the result itself) and
    /// `terminal`. A script that no longer parses, or has no interpreter, is
    /// refused as [`ErrorKind::BadRequest`].
    RunScript {
        /// The script's id, as [`ScriptEntry::id`] carries it.
        id: String,
        /// Values for its arguments, in order.
        arguments: Vec<String>,
    },
    /// What a running script has printed so far. Answered with
    /// [`Response::ScriptOutput`].
    ScriptOutput {
        /// From [`Response::ScriptStarted`].
        session: u64,
    },
    /// Stop a running script. Answered with [`Response::Ack`].
    StopScript {
        /// From [`Response::ScriptStarted`].
        session: u64,
    },
    /// Every executable in the `PATH` directories, and how Run Terminal
    /// Program runs them. Answered with [`Response::Programs`].
    ListPrograms,
    /// Run a command line: in the terminal emulator (kept open when `hold`),
    /// or directly. Answered with [`Response::Ack`] once started; a program
    /// that is not found is refused as [`ErrorKind::BadRequest`] ("Not a
    /// valid executable"), and a terminal run with no terminal installed as
    /// [`ErrorKind::Unsupported`].
    RunProgram {
        /// The program and its arguments.
        argv: Vec<String>,
        /// Run it in a terminal.
        terminal: bool,
        /// Keep the terminal open once it exits.
        hold: bool,
    },
    /// `compass dmenu`: show a list in the launcher and wait for the choice.
    /// Answered with [`Response::DmenuOutput`] once the person chose (or
    /// dismissed the list); refused as [`ErrorKind::Unsupported`] when no
    /// launcher window is attached.
    Dmenu {
        /// What to show.
        spec: DmenuSpec,
    },
    /// The window asks for the list behind a [`WindowCommand::Dmenu`].
    /// Answered with [`Response::DmenuList`].
    DmenuFetch {
        /// From the pushed command.
        token: u64,
    },
    /// The window's answer to a dmenu list: what to print, or `None` when
    /// the list was dismissed. Answered with [`Response::Ack`].
    DmenuChoose {
        /// From the pushed command.
        token: u64,
        /// The chosen entry, its index, or the search text.
        output: Option<String>,
    },
    /// Keep a theme in the configuration (`launcher.appearance.theme`), as
    /// `compass theme set` does. Answered with [`Response::Ack`]; an unknown
    /// name is refused as [`ErrorKind::BadRequest`], and a configuration that
    /// cannot be written as [`ErrorKind::Internal`].
    SetTheme {
        /// The theme's persisted name, e.g. `tokyo-night`.
        theme: String,
    },
    /// Generate a new extension's boilerplate, as the developer extension's
    /// Create Extension form does. Answered with
    /// [`Response::ExtensionCreated`]; a form that does not validate is
    /// refused as [`ErrorKind::BadRequest`] naming the fields, and a failed
    /// generation as [`ErrorKind::Internal`].
    CreateExtension {
        /// Who is writing it; three characters at least.
        author: String,
        /// The extension's title; three characters at least.
        title: String,
        /// What it does; sixteen characters at least.
        description: String,
        /// The directory to create it in, which must exist; `~` is expanded.
        location: String,
        /// The first command's title.
        command_title: String,
        /// The first command's description.
        command_description: String,
        /// The command template, e.g. `:boilerplate/tmpl-list`.
        template: String,
    },
    /// The installed font families, grouped and classified as Browse Fonts
    /// lists them. Answered with [`Response::Fonts`].
    ListFonts,
    /// A family's specimen, as Markdown. Answered with [`Response::Text`];
    /// an unknown family is refused as [`ErrorKind::BadRequest`].
    FontSpecimen {
        /// The family's name, as [`FontEntry::name`] carries it.
        name: String,
    },
    /// Every Rhai script the engine has loaded, as root search lists them.
    /// Answered with [`Response::RhaiScripts`]. A script is opened with
    /// [`Request::RunExtensionCommand`] and its `rhai:` id, and then followed
    /// and driven exactly as an extension's view is.
    ListRhaiScripts,
    /// A store's extensions: the Vicinae store's whole list filtered by
    /// `query`, or the Raycast store's first page (empty `query`) or its
    /// search results. Answered with [`Response::StoreListing`].
    StoreBrowse {
        /// Which store.
        store: StoreKind,
        /// What was typed; empty for the list itself.
        query: String,
    },
    /// One store extension's detail page. Answered with
    /// [`Response::StoreExtension`]; one the store does not have is refused
    /// as [`ErrorKind::BadRequest`].
    StoreExtension {
        /// Which store.
        store: StoreKind,
        /// [`StoreEntry::owner`].
        owner: String,
        /// Its name in the store.
        name: String,
    },
    /// Download a store extension's bundle and install it, replacing an
    /// installed copy (which is how an update is applied). Answered with
    /// [`Response::StoreInstalled`] once it is in root search.
    StoreInstall {
        /// Which store.
        store: StoreKind,
        /// [`StoreEntry::owner`].
        owner: String,
        /// Its name in the store.
        name: String,
    },
    /// Remove an installed extension, its support files and its stored
    /// data. Answered with [`Response::Ack`]; an id nothing is installed
    /// under is a bad request.
    StoreUninstall {
        /// The installed id, e.g. `store.vicinae.bluetooth`.
        id: String,
    },
    /// Open an `http(s)` URL with the default browser. Answered with
    /// [`Response::Ack`]; any other scheme is a bad request.
    OpenUrl {
        /// The URL.
        url: String,
    },
    /// The snippet keyword expander's keyboard helper: whether it is wanted,
    /// running, and able to type. Answered with
    /// [`Response::InputServerStatus`].
    InputServerStatus,
    /// Turn the keyboard helper on or off, as `input_server.enabled` in
    /// `compass.json` (which is written), and answer
    /// [`Response::InputServerStatus`] once applied.
    SetInputServerEnabled {
        /// Whether it should run.
        enabled: bool,
    },
    /// The launch an extension asked for, which the engine pushed to the
    /// window as [`WindowCommand::Launch`]. Answered once with
    /// [`Response::ExtensionLaunch`]; a token already taken, or never
    /// given, is a bad request.
    ExtensionLaunchFetch {
        /// From [`WindowCommand::Launch`].
        token: u64,
    },
    /// The subtitles extensions set for their commands
    /// (`updateCommandMetadata`), which root search shows in place of the
    /// extension's title. Answered with [`Response::ExtensionSubtitles`].
    ExtensionSubtitles,
    /// A command's preferences form, without running it. Answered with
    /// [`Response::ExtensionNeedsPreferences`]; save it with
    /// [`Request::SetExtensionPreferences`].
    ExtensionPreferences {
        /// The command's [`QueryHit::id`].
        id: String,
    },
    /// [`Request::RunMediaCommand`] with the command's optional argument:
    /// the `player` to fuzzy-match over the running players, or the volume
    /// `step` in percent. `None` or empty is the command's default.
    RunMediaCommandWith {
        /// The command's id in `compass_core::media_commands`.
        id: String,
        /// What was entered for its argument.
        argument: Option<String>,
    },
    /// The running media players, for Now Playing. Answered with
    /// [`Response::MediaPlayers`].
    ListMediaPlayers,
    /// Play/pause, skip or go back on one player, by its bus name, without a
    /// HUD. Answered with [`Response::Ack`].
    ControlMediaPlayer {
        /// The player's bus name, as [`MediaPlayerEntry::id`] carries it.
        player: String,
        /// What to do.
        action: MediaPlayerAction,
    },
    /// "Set as Compass font": write `font.normal.family` to `compass.json`.
    /// Answered with [`Response::Ack`]; an empty family is a bad request.
    SetFont {
        /// The family's name.
        family: String,
    },
    /// A deeplink the launcher handles (`vicinae://extensions/<author>/<name>`
    /// and its `raycast://` spellings), pushed to the window as
    /// [`WindowCommand::Deeplink`]. Answered with [`Response::Ack`] once the
    /// window shows it; one it does not handle is a bad request.
    OpenDeeplink {
        /// The URL.
        url: String,
    },
    /// What the user has allowed their own Rhai scripts to do, and, since
    /// v22, which programs they have always allowed an extension to run on
    /// the host. Answered with [`Response::ScriptGrants`].
    ListScriptGrants,
    /// Withdraw everything the user allowed a Rhai script, or an extension.
    /// Answered with [`Response::ScriptGrants`], the list after the change;
    /// an id with nothing recorded is a bad request.
    RevokeScriptGrant {
        /// The script's id, `script.<folder name>`, or the extension's,
        /// `store.raycast.brew`.
        id: String,
    },
    /// How many times the engine has rescanned its catalog (the applications
    /// or the installed extensions) because their directories changed.
    /// Answered with [`Response::CatalogGeneration`]; a window whose last
    /// answer differs scans its own copy again.
    CatalogGeneration,
    /// The applications Set Default Browser or Set Default Terminal offers,
    /// the current default first. Answered with [`Response::DefaultApps`].
    ListDefaultApps {
        /// Which choice.
        kind: DefaultAppKind,
    },
    /// Make `id` the default browser (`mimeapps.list`) or terminal
    /// (`xdg-terminals.list`). Answered with [`Response::Ack`], or an error
    /// carrying the sentence to show.
    SetDefaultApp {
        /// Which choice.
        kind: DefaultAppKind,
        /// The desktop file id.
        id: String,
    },
    /// [`Request::ClipboardHistory`] restricted to one kind of entry, the
    /// history view's filter; `None` is every kind. Answered with
    /// [`Response::ClipboardHistory`].
    ClipboardHistoryOfKind {
        /// Text to filter by; empty lists everything.
        query: String,
        /// At most this many entries; zero is a bad request.
        limit: u32,
        /// The kind to keep.
        kind: Option<ClipboardKind>,
    },
    /// What the detail pane shows about one entry besides its content.
    /// Answered with [`Response::ClipboardDetail`]; an unknown id is not
    /// found.
    ClipboardDetail {
        /// [`ClipboardEntry::id`].
        id: String,
    },
    /// Set the words an entry is also found by; empty clears them.
    /// Answered with [`Response::Ack`]; an unknown id is not found.
    ClipboardSetKeywords {
        /// [`ClipboardEntry::id`].
        id: String,
        /// Space-separated keywords.
        keywords: String,
    },
    /// Remove every entry, sparing pinned and keyworded ones when the
    /// `preserveTagged` preference says so. Answered with [`Response::Ack`].
    ClipboardRemoveAll,
    /// Whether copies are being recorded, turning it on or off first when
    /// `enabled` is given (and keeping the choice in the configuration, as
    /// the `monitoring` preference). Answered with
    /// [`Response::ClipboardMonitoring`].
    ClipboardMonitoring {
        /// The new state, or `None` to ask.
        enabled: Option<bool>,
    },
    /// What the root row's action panel changes about one root item: its
    /// favourite, its place among the favourites, its alias, its switch
    /// (all kept in the configuration) or its ranking (the launch history).
    /// Answered with [`Response::Ack`] once kept and applied to root search;
    /// an id no root item has is a bad request.
    RootItemEdit {
        /// The item's `provider:entrypoint` id, as [`QueryHit::id`].
        id: String,
        /// What to change.
        edit: RootItemEdit,
    },
    /// Every root item's id and title, sorted by id, as the C++
    /// `listCommands` answers `compass cmd ls`. Answered with
    /// [`Response::Commands`]. (v17.)
    ListCommands,
    /// Run a root item as if it had been picked in root search: an
    /// application is launched by the engine, anything else is pushed to the
    /// window as [`WindowCommand::Launch`]. `args` fill the command's
    /// arguments in order, checked as the C++ `buildLaunchArguments` checks
    /// them; `query` is its fallback text. Answered with [`Response::Ack`];
    /// an unknown id or ill-fitting arguments are a bad request. (v17.)
    LaunchCommand {
        /// The item's [`QueryHit::id`], e.g. `commands:clipboard-history`.
        id: String,
        /// The command's arguments, positionally.
        args: Vec<String>,
        /// The caller's working directory, for the command's context.
        cwd: Option<String>,
        /// Fallback text: what the command's search starts with.
        query: Option<String>,
    },
    /// Launch an application, or focus its first open window unless
    /// `new_instance`. Answered with [`Response::AppLaunched`]; an unknown
    /// id is a bad request. (v17.)
    LaunchApp {
        /// The application's desktop id, e.g. `firefox.desktop`, or its root
        /// id, `applications:firefox`.
        id: String,
        /// Passed to it as `%U`/`%F` arguments.
        args: Vec<String>,
        /// Always start a new instance.
        new_instance: bool,
    },
    /// Whether the launcher window is open. Answered with
    /// [`Response::WindowState`]; with no window attached it is closed. (v17.)
    DescribeWindow,
    /// The file index, queried directly as `compass fs query` does: no
    /// recent files, no direct paths. Answered with [`Response::Files`];
    /// refused as [`ErrorKind::Unsupported`] while the indexer is not
    /// running. (v17.)
    FsQuery {
        /// Search text.
        query: String,
        /// At most this many files.
        limit: u32,
        /// Only this category, as `compass_core::file_search::CATEGORY_FILTER_KEYS`
        /// spells it.
        category: Option<String>,
    },
    /// Whether an application is running, which is whether it has a
    /// window, and whether one of them has focus. Answered with
    /// [`Response::AppRuntime`]; an unknown id is a bad request. (v17.)
    AppRuntime {
        /// The application's desktop id, e.g. `firefox.desktop`.
        id: String,
    },
    /// Quit an application, as the C++ `LinuxAppRuntime` does: close every
    /// window it has, or with `force`, `SIGKILL` every process that owns one
    /// and close the windows that name none. Answered with
    /// [`Response::Ack`] when something was done; refused otherwise. (v17.)
    QuitApp {
        /// The application's desktop id.
        id: String,
        /// Force Quit rather than Quit.
        force: bool,
    },
    /// [`Request::QuitApp`] for the application a window belongs to, as the
    /// window switcher offers it. (v17.)
    QuitWindowApp {
        /// The window's [`WindowInfo::id`].
        window: u32,
        /// Force Quit rather than Quit.
        force: bool,
    },
    /// The calculator's history matching `query`, grouped by when each
    /// answer was copied. Answered with [`Response::CalculatorHistory`];
    /// refused as [`ErrorKind::Unsupported`] with no keyring to open it
    /// with. (v17.)
    CalculatorHistory {
        /// Filters the rows by question and answer; empty keeps them all.
        query: String,
    },
    /// Remember a calculation whose answer was copied, as the C++
    /// `addRecord`. Answered with [`Response::Ack`]. (v17.)
    AddCalculatorRecord {
        /// What was asked.
        question: String,
        /// What came back.
        answer: String,
        /// A unit conversion rather than arithmetic.
        conversion: bool,
    },
    /// Pin, unpin or remove one remembered calculation, or remove them all.
    /// Answered with [`Response::Ack`]. (v17.)
    EditCalculatorHistory {
        /// What to do.
        edit: CalculatorEdit,
    },
    /// Other applications' tray icons, as the engine's StatusNotifierItem
    /// host has them. Answered with [`Response::TrayItems`]; refused as
    /// [`ErrorKind::Unsupported`] when the host could not start (no session
    /// bus). (v18.)
    TrayItems,
    /// Activate a tray item, as clicking its icon would, or its secondary
    /// activation (a middle click). Answered with [`Response::Ack`]. (v18.)
    TrayActivate {
        /// The item's [`TrayItemInfo::key`].
        key: String,
        /// `SecondaryActivate` rather than `Activate`.
        secondary: bool,
    },
    /// A tray item's menu, flattened as the C++ tray view lists it.
    /// Answered with [`Response::TrayMenu`]. (v18.)
    TrayMenu {
        /// The item's [`TrayItemInfo::key`].
        key: String,
    },
    /// Click one entry of a tray item's menu. Answered with
    /// [`Response::Ack`]. (v18.)
    TrayTriggerMenu {
        /// The item's [`TrayItemInfo::key`].
        key: String,
        /// The entry's [`TrayMenuEntry::id`].
        id: i32,
    },
    /// Put `text` on the clipboard and paste it into the window that has
    /// focus once the launcher hides (`PasteToFocusedWindowAction`).
    /// Answered with [`Response::Ack`]; refused where the engine cannot
    /// paste, and the window copies instead. (v18.)
    PasteText {
        /// What to paste.
        text: String,
    },
    /// What the compositor's window manager can do, which decides the
    /// window-management commands root search offers. Answered with
    /// [`Response::WindowManagerCapabilities`]; all `false` off a compositor
    /// the engine drives. (v18.)
    WindowManagerCapabilities,
    /// The workspaces, for Switch Workspaces. Answered with
    /// [`Response::Workspaces`], or refused as [`ErrorKind::Unsupported`]
    /// where the engine has no workspaces to list. (v18.)
    ListWorkspaces,
    /// Switch to the workspace `id` (a [`WorkspaceEntry::id`]). Answered
    /// with [`Response::Ack`]. (v18.)
    FocusWorkspace {
        /// The compositor's own id.
        id: String,
    },
    /// Toggle fullscreen or floating on the window the person was in, or the
    /// overview. Answered with [`Response::Ack`], or refused with the
    /// sentence to show. (v18.)
    ToggleWindowState {
        /// What to toggle.
        toggle: WindowToggle,
    },
    /// The applications that open `target` (a path or a URL), for "Open
    /// with…". Answered with [`Response::Openers`]. (v18.)
    ListOpeners {
        /// What would be opened.
        target: String,
    },
    /// Open `target` with the application `app` (an [`OpenerEntry::id`]).
    /// Answered with [`Response::Ack`]; an unknown id is a bad request.
    /// (v18.)
    OpenWith {
        /// The application's desktop id.
        app: String,
        /// What to open.
        target: String,
    },
    /// What a file's action panel depends on (`FileActions::actionPanel`):
    /// its MIME type and what this session can do with it. Answered with
    /// [`Response::FileActions`]. (v18.)
    FileActions {
        /// The file's absolute path.
        path: String,
    },
    /// Put a file on the clipboard as a file (a `text/uri-list`), and with
    /// `paste`, paste it into the focused window. Answered with
    /// [`Response::Ack`]. (v18.)
    CopyFile {
        /// The file's absolute path.
        path: String,
        /// Paste it too.
        paste: bool,
    },
    /// Run a file as a program, first making it executable when asked (an
    /// AppImage). Answered with [`Response::Ack`], or refused with the
    /// sentence to show. (v18.)
    RunExecutable {
        /// The file's absolute path.
        path: String,
        /// Give it the owner's execute permission first.
        make_executable: bool,
    },
    /// Make an image the wallpaper. Answered with [`Response::Ack`], or
    /// refused with the backend's reason. (v18.)
    SetWallpaper {
        /// The image's absolute path.
        path: String,
    },
    /// Expand a text snippet for Manage Snippets' detail pane, as
    /// `updateExpandedText` does: its `{shell}` placeholders are shown, not
    /// run. Answered with [`Response::Text`]; a file snippet answers with its
    /// path. (v19.)
    PreviewSnippet {
        /// Which one.
        id: String,
        /// `(name, value)` for its arguments.
        arguments: Vec<(String, String)>,
    },
    /// The icon of each script command, as `ScriptCommandFile::icon` resolves
    /// its `@raycast.icon`. Answered with [`Response::ScriptIcons`]. (v19.)
    ScriptIcons,
    /// Write one setting the settings view edits into `compass.json` and
    /// apply it: `key` is its dotted path, as `compass_core::settings_catalog`
    /// lists it, and `value_json` its new value, `null` to reset it. Answered
    /// with [`Response::Ack`]; a key that is not a setting, or a value it
    /// does not take, is a bad request with the sentence to show. (v19.)
    SetSetting {
        /// The setting's dotted path (`launcher.wrap_navigation`).
        key: String,
        /// The value, as JSON.
        value_json: String,
    },
    /// Turn a whole provider's items on or off in root search
    /// (`setProviderEnabled`). Answered with [`Response::Ack`]. (v19.)
    SetProviderEnabled {
        /// The provider's id (`applications`).
        provider: String,
        /// Whether its items are offered.
        enabled: bool,
    },
    /// Inspect Local Storage's list: every namespace that holds an item.
    /// Answered with [`Response::LocalStorageNamespaces`]. (v19.)
    LocalStorageNamespaces,
    /// One namespace's items. Answered with [`Response::LocalStorageItems`].
    /// (v19.)
    LocalStorageItems {
        /// The namespace.
        namespace: String,
    },
    /// Manage OAuth Token Sets' list. Answered with
    /// [`Response::OAuthTokenSets`]. (v19.)
    OAuthTokenSets,
    /// Remove one extension's token set for one provider (`None` for its
    /// unnamed one). Answered with [`Response::Ack`]. (v19.)
    RemoveOAuthTokenSet {
        /// The extension.
        extension_id: String,
        /// The provider.
        provider_id: Option<String>,
    },
    /// The shortcut recorder started (`true`) or stopped capturing: the
    /// engine releases every global shortcut while it captures, so the
    /// combination reaches the recorder rather than the desktop, and binds
    /// them again after (`GlobalShortcutService::setCapturing`). Answered
    /// with [`Response::Ack`]. (v20.)
    ShortcutCapture {
        /// Whether the recorder is capturing.
        capturing: bool,
    },
    /// Whether a newer Compass release is out, as the root search's Update
    /// section shows it (`UpdateService::available`). The engine asks the
    /// release feed when its last answer is older than six hours and
    /// `launcher.check_for_updates` is on. Answered with
    /// [`Response::UpdateStatus`]. (v21.)
    UpdateStatus,
    /// Never offer the release `tag` again (`skipAvailableVersion`).
    /// Answered with [`Response::Ack`]. (v21.)
    SkipUpdate {
        /// The release's tag, as [`UpdateOffer::tag`] gave it.
        tag: String,
    },
    /// The exchange rates the engine holds, for the calculator's currency
    /// conversions. Answered with [`Response::ExchangeRates`]. (v21.)
    ExchangeRates,
    /// Refresh Exchange Rates: fetch the rates now, whatever their age.
    /// Answered with [`Response::ExchangeRates`] holding the fresh rates, or
    /// an error saying why the fetch failed (the rates held are kept).
    /// (v21.)
    RefreshExchangeRates,
    /// The shortcut recorder captured `trigger`: would the desktop bind it
    /// (`GlobalShortcutService::probeBind`)? Asked while capturing, so the
    /// probe binds and releases it at once. Answered with
    /// [`Response::ShortcutProbe`]. (v21.)
    ProbeShortcut {
        /// The combination, as the configuration spells it.
        trigger: String,
    },
    /// The person chose the view's alert's third answer
    /// ([`ExtensionAlert::remember_text`], Ctrl+Enter): allow, and remember
    /// it. Answered with [`Response::Ack`]; a session with no alert waiting is
    /// a bad request. (v22.)
    ExtensionAlertRemember {
        /// From [`Response::ExtensionStarted`].
        session: u64,
    },
    /// Put `text` on the clipboard, and keep it there after the window hides
    /// (`ClipboardService::copyText`). The window's own clipboard source goes
    /// with its last surface, so a copy that hides the launcher has to be
    /// owned by the engine: over data-control on wlroots, through the GNOME
    /// Shell extension elsewhere. Answered with [`Response::Ack`]; refused
    /// where the engine reaches no clipboard, and the window copies itself.
    /// (v23.)
    CopyText {
        /// What to copy.
        text: String,
    },
    /// The launcher hotkey's state with the desktop. With `request`, the
    /// desktop is asked for it now (the first-run flow's Set Up Shortcut and
    /// Try Again), and the answer waits for its dialog. Answered with
    /// [`Response::LauncherShortcut`]. (v25.)
    LauncherShortcut {
        /// Ask the desktop now, rather than only report.
        request: bool,
    },
    /// Starting Compass at login: `enable` turns it on or off, and `None`
    /// only reads it. Answered with [`Response::LoginStart`], or an error
    /// sentence when the change failed and nothing changed. (v25.)
    LoginStart {
        /// On, off, or only report.
        enable: Option<bool>,
    },
    /// How clipboard history is capturing and storing copies, and why not
    /// when it is not. Answered with [`Response::ClipboardStatus`]. (v26.)
    ClipboardStatus,
}

/// A newer Compass release, in a [`Response::UpdateStatus`]. (v21.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOffer {
    /// The release's tag, such as `v1.2.0`.
    pub tag: String,
    /// The tag without its leading `v`, for showing.
    pub version: String,
    /// The release page.
    pub release_url: String,
}

/// Currency exchange rates: one day of the ECB's reference rates. (v21.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeRateTable {
    /// The ECB's reference date, `YYYY-MM-DD`.
    pub date: String,
    /// When the engine fetched them, in seconds since the epoch.
    pub fetched_at: i64,
    /// `(ISO 4217 code, units per euro)`, the rate as decimal text that
    /// reads back to the same `f64`, so the table stays `Eq`.
    pub rates: Vec<(String, String)>,
}

/// Answer to [`Request::FileActions`]. (v18.)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FileActionInfo {
    /// Its MIME type.
    pub mime: Option<String>,
    /// Whether an application opens it.
    pub has_opener: bool,
    /// Whether this desktop's wallpaper can be set.
    pub can_set_wallpaper: bool,
    /// Whether the engine can paste into the focused window.
    pub can_paste: bool,
}

/// An application in a [`Response::Openers`]. (v18.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenerEntry {
    /// Its desktop id, for [`Request::OpenWith`].
    pub id: String,
    /// Its display name.
    pub name: String,
    /// Its icon name.
    pub icon: Option<String>,
    /// Whether it is the default for the target's type.
    pub default: bool,
}

/// One application's tray icon (`TrayItem`), as the tray view lists it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrayItemInfo {
    /// The bus name it registered from: what the other tray requests name
    /// it by.
    pub key: String,
    /// Its title, else its id.
    pub title: String,
    /// Its tooltip, as the row's second line.
    pub subtitle: String,
    /// It is asking for attention.
    pub attention: bool,
    /// It has a menu to browse.
    pub has_menu: bool,
    /// The whole item is a menu: it cannot be activated.
    pub item_is_menu: bool,
    /// Its icon: a file (resolved against its own theme path), when it named
    /// one there.
    pub icon_path: Option<String>,
    /// Its icon: a theme name, when it gave one.
    pub icon_name: Option<String>,
    /// Its icon: its largest pixmap encoded as a PNG, when it sent pixels.
    pub icon_png: Option<Vec<u8>>,
}

/// One clickable entry of a tray item's menu.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrayMenuEntry {
    /// Its id on the bus, for [`Request::TrayTriggerMenu`].
    pub id: i32,
    /// Its label, after its submenus' (`Speed › Fast`).
    pub label: String,
    /// For a checkbox or radio entry, whether it is on.
    pub toggled: Option<bool>,
    /// Its icon's theme name.
    pub icon_name: Option<String>,
}

/// What [`Request::ToggleWindowState`] toggles. (v18.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowToggle {
    /// The window in and out of fullscreen.
    Fullscreen,
    /// The window between floating and tiled.
    Floating,
    /// The compositor's overview.
    Overview,
}

/// Answer to [`Request::WindowManagerCapabilities`]. (v18.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WindowManagerCapabilities {
    /// It has workspaces to switch between.
    pub workspaces: bool,
    /// A window can be made fullscreen.
    pub fullscreen: bool,
    /// A window can be floated.
    pub floating: bool,
    /// It has an overview.
    pub overview: bool,
}

/// One workspace in a [`Response::Workspaces`]. (v18.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    /// The compositor's own id, for [`Request::FocusWorkspace`].
    pub id: String,
    /// What it is called: its name, else its number.
    pub name: String,
    /// The monitor it is on, when known.
    pub monitor: Option<String>,
    /// How many windows are on it.
    pub window_count: u32,
    /// The applications with a window on it, once each, in window order.
    pub apps: Vec<WorkspaceApp>,
    /// Whether it is the active one.
    pub active: bool,
}

/// An application on a [`WorkspaceEntry`]. (v18.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceApp {
    /// Its display name.
    pub name: String,
    /// Its icon name.
    pub icon: Option<String>,
}

/// One change [`Request::RootItemEdit`] makes (`RootSearchActionGenerator`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RootItemEdit {
    /// Add it to the favourites (first) or take it out.
    Favorite(bool),
    /// Swap it with its neighbour among the favourites, below when `down`.
    MoveFavorite {
        /// Towards the end of the list.
        down: bool,
    },
    /// Set its alias.
    Alias(String),
    /// Take it out of root search.
    Disable,
    /// Forget its launch history.
    ResetRanking,
    /// Give it a keyboard shortcut (`control+shift+A`), or clear it with an
    /// empty one. Version 18.
    Shortcut(String),
    /// Put it in root search or take it out, as the settings view's switch
    /// does. Version 19.
    Enabled(bool),
    /// Make it a fallback, first in the list (`enableFallback`), or stop it
    /// being one (`disableFallback`). Version 19.
    Fallback(bool),
}

/// What the engine answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    /// Answer to [`Request::Ping`].
    Pong {
        /// Protocol version the server speaks.
        protocol_version: u16,
        /// Process id of the engine, for `doctor` and stale-socket reporting.
        pid: u32,
    },
    /// The requested side effect was performed; there is nothing to report.
    Ack,
    /// Ranked results for a [`Request::Query`].
    QueryResults {
        /// Hits in presentation order: best first.
        hits: Vec<QueryHit>,
    },
    /// Diagnostic report for a [`Request::Doctor`].
    DoctorReport {
        /// One entry per check, in the order they were run.
        checks: Vec<DoctorCheck>,
    },
    /// The engine accepted [`Request::Shutdown`] and is on its way down.
    ShuttingDown,
    /// The request could not be served.
    Error(ProtocolError),
    /// [`Request::AttachWindow`] was accepted; this connection is now the
    /// launcher window's push channel.
    WindowAttached,
    /// A command pushed from the engine to an attached window.
    ///
    /// This is the one frame the engine sends unsolicited. Its envelope id is
    /// allocated by the engine and echoed by the window in the matching
    /// [`Request::WindowOutcome`].
    Window(WindowCommand),
    /// Entries for a [`Request::ClipboardHistory`], in presentation order.
    ClipboardHistory {
        /// Matching entries: pinned first, then most recently copied.
        entries: Vec<ClipboardEntry>,
    },
    /// Content for a [`Request::ClipboardContent`].
    ClipboardContent {
        /// MIME type of the bytes.
        mime_type: String,
        /// The content, exactly as it was copied.
        data: Vec<u8>,
    },
    /// Answer to [`Request::ListWindows`], most recently used first.
    Windows {
        /// Every window the extension reports.
        windows: Vec<WindowInfo>,
    },
    /// A view command started: follow it with [`Request::ExtensionView`].
    ExtensionStarted {
        /// The session to follow.
        session: u64,
    },
    /// Answer to [`Request::ExtensionView`].
    ExtensionView {
        /// The session's version now; equal to `after` on a timeout.
        version: u64,
        /// The view, as `compass_extension_api::View` JSON, once rendered.
        view_json: Option<String>,
        /// Why the view cannot be drawn, or why the command ended.
        problem: Option<String>,
        /// Whether the command has ended.
        ended: bool,
        /// How many views the extension has pushed, the root one included.
        depth: u32,
        /// A confirmation the extension is waiting on, if any. Answer it with
        /// [`Request::ExtensionAlertAnswer`].
        alert: Option<ExtensionAlert>,
        /// The toast the extension is showing, if any.
        toast: Option<ExtensionToast>,
    },
    /// Answer to [`Request::RunExtensionCommand`] when a required preference
    /// has no value: the form to show. Answer with
    /// [`Request::SetExtensionPreferences`], then run the command again.
    ExtensionNeedsPreferences {
        /// The command's title, for the form's heading.
        title: String,
        /// Every preference the command reads, required ones included.
        fields: Vec<PreferenceField>,
    },
    /// Answer to [`Request::RunExtensionCommand`] when the command declares
    /// arguments and was given none, or left a required one empty: the form
    /// to show. Run the command again with what was entered.
    ExtensionNeedsArguments {
        /// The command's title, for the form's heading.
        title: String,
        /// Every argument, in the manifest's order.
        fields: Vec<PreferenceField>,
    },
    /// Answer to [`Request::SearchFiles`].
    Files {
        /// What the list is: "Recently Accessed", "Direct file path",
        /// "Recently Modified" or "Results".
        heading: String,
        /// The files, in presentation order.
        files: Vec<FileHit>,
    },
    /// Every stored shortcut, in the store's order: the answer to
    /// [`Request::ListShortcuts`], and to a change to the list.
    Shortcuts {
        /// The shortcuts.
        shortcuts: Vec<ShortcutEntry>,
    },
    /// A piece of text the engine produced, such as an expanded shortcut.
    Text {
        /// The text.
        text: String,
    },
    /// Every stored snippet, in the store's order: the answer to
    /// [`Request::ListSnippets`], and to a change to the list.
    Snippets {
        /// The snippets.
        snippets: Vec<SnippetEntry>,
    },
    /// Answer to [`Request::ListScripts`], in scan order.
    Scripts {
        /// The script commands.
        scripts: Vec<ScriptEntry>,
    },
    /// Answer to [`Request::RunScript`].
    ScriptStarted {
        /// The run to follow, when the launcher shows its output.
        session: Option<u64>,
    },
    /// Answer to [`Request::ListFonts`].
    Fonts {
        /// The families, in the browser's order.
        fonts: Vec<FontEntry>,
        /// The category names, in the order the filter offers them.
        categories: Vec<String>,
    },
    /// Answer to [`Request::CreateExtension`].
    ExtensionCreated {
        /// Where the extension was written.
        path: String,
    },
    /// Answer to [`Request::Dmenu`]: what to print; empty when the list was
    /// dismissed.
    DmenuOutput {
        /// The chosen entry, its index, or the search text.
        output: String,
    },
    /// Answer to [`Request::DmenuFetch`].
    DmenuList {
        /// What to show.
        spec: DmenuSpec,
    },
    /// Answer to [`Request::ListPrograms`].
    Programs {
        /// Every executable found, as absolute paths, in `PATH` order.
        programs: Vec<String>,
        /// The terminal emulator's name, for the actions' titles; `None`
        /// when none is installed.
        terminal: Option<String>,
        /// The command's `default-action` preference: `run-in-terminal`,
        /// `run-in-terminal-hold` or `run`.
        default_action: String,
    },
    /// Answer to [`Request::ScriptOutput`].
    ScriptOutput {
        /// Everything read so far: stdout and stderr interleaved for
        /// `fullOutput`, stdout alone otherwise.
        output: String,
        /// Whether the script has exited (or was stopped, or timed out).
        finished: bool,
        /// Its exit code, once it exited normally.
        exit_code: Option<i32>,
        /// Milliseconds since it started, or how long it ran once finished.
        elapsed_ms: u64,
    },
    /// Answer to [`Request::ListRhaiScripts`], in id order.
    RhaiScripts {
        /// The scripts.
        scripts: Vec<RhaiScriptEntry>,
    },
    /// Answer to [`Request::StoreBrowse`].
    StoreListing {
        /// The heading over the rows: `Extensions` or `Results`.
        heading: String,
        /// The extensions, in the order to show them.
        entries: Vec<StoreEntry>,
    },
    /// Answer to [`Request::StoreExtension`].
    StoreExtension {
        /// The extension.
        detail: StoreDetail,
    },
    /// Answer to [`Request::StoreInstall`].
    StoreInstalled {
        /// The id it was installed under.
        id: String,
        /// Its title, for the confirmation.
        title: String,
    },
    /// Answer to [`Request::InputServerStatus`] and
    /// [`Request::SetInputServerEnabled`].
    InputServerStatus(InputServerStatus),
    /// Answer to [`Request::ExtensionLaunchFetch`]: run the command as if it
    /// had been picked in root search, or open its preferences.
    ExtensionLaunch {
        /// The command's [`QueryHit::id`].
        id: String,
        /// Its arguments, as a JSON object, when the extension passed any.
        arguments_json: Option<String>,
        /// Open its preferences form instead of running it.
        preferences: bool,
    },
    /// Answer to [`Request::ExtensionSubtitles`]: `(command id, subtitle)`.
    ExtensionSubtitles {
        /// Every override, by command id.
        subtitles: Vec<(String, String)>,
    },
    /// Answer to [`Request::ListMediaPlayers`], in the bus's order.
    MediaPlayers {
        /// The players.
        players: Vec<MediaPlayerEntry>,
    },
    /// Answer to [`Request::ListScriptGrants`] and
    /// [`Request::RevokeScriptGrant`], in id order.
    ScriptGrants {
        /// One per script with something allowed.
        grants: Vec<ScriptGrantEntry>,
    },
    /// Answer to [`Request::CatalogGeneration`].
    CatalogGeneration {
        /// Starts at zero and goes up by one per rescan.
        generation: u64,
    },
    /// Answer to [`Request::ListDefaultApps`], in the order offered.
    DefaultApps {
        /// The candidates.
        apps: Vec<DefaultAppEntry>,
    },
    /// Answer to [`Request::ClipboardDetail`].
    ClipboardDetail {
        /// What the pane shows.
        detail: ClipboardDetail,
    },
    /// Answer to [`Request::ClipboardMonitoring`].
    ClipboardMonitoring {
        /// Whether the engine can record copies at all on this desktop.
        supported: bool,
        /// Whether it is recording them.
        enabled: bool,
    },
    /// Answer to [`Request::ListCommands`]. (v17.)
    Commands {
        /// Sorted by id.
        commands: Vec<CommandInfo>,
    },
    /// Answer to [`Request::LaunchApp`]. (v17.)
    AppLaunched {
        /// The title of the window focused instead of launching, if one was.
        focused_window_title: Option<String>,
    },
    /// Answer to [`Request::DescribeWindow`]. (v17.)
    WindowState {
        /// Whether the launcher window is on screen.
        open: bool,
    },
    /// Answer to [`Request::CalculatorHistory`]: the non-empty groups, in
    /// order. (v17.)
    CalculatorHistory {
        /// `Pinned`, `Today`, `This week`, `This month`, `This year`,
        /// `A few years ago`, each only when it has a row.
        groups: Vec<CalculatorGroup>,
    },
    /// Answer to [`Request::AppRuntime`]. (v17.)
    AppRuntime {
        /// It has at least one window.
        running: bool,
        /// One of its windows has focus.
        frontmost: bool,
        /// Its windows, the first the one to focus.
        windows: Vec<WindowInfo>,
    },
    /// Answer to [`Request::ExtensionLaunchFetch`] for a launch that carries
    /// fallback text (`compass cmd launch --query`). (v17.)
    CommandLaunch {
        /// The item's [`QueryHit::id`].
        id: String,
        /// Its arguments, as a JSON object, when any were given.
        arguments_json: Option<String>,
        /// What its search starts with.
        fallback_text: Option<String>,
    },
    /// Answer to [`Request::TrayItems`]. (v18.)
    TrayItems {
        /// In the order the host learned of them.
        items: Vec<TrayItemInfo>,
    },
    /// Answer to [`Request::TrayMenu`]. (v18.)
    TrayMenu {
        /// The clickable entries, flattened.
        entries: Vec<TrayMenuEntry>,
    },
    /// Answer to [`Request::WindowManagerCapabilities`]. (v18.)
    WindowManagerCapabilities(WindowManagerCapabilities),
    /// Answer to [`Request::ListWorkspaces`], in the compositor's order.
    /// (v18.)
    Workspaces {
        /// Every workspace.
        workspaces: Vec<WorkspaceEntry>,
    },
    /// Answer to [`Request::ListOpeners`]: the default first. (v18.)
    Openers {
        /// The applications.
        apps: Vec<OpenerEntry>,
    },
    /// Answer to [`Request::FileActions`]. (v18.)
    FileActions(FileActionInfo),
    /// Answer to [`Request::ScriptIcons`]: `(script id, icon URL)`, the URL
    /// in `ImageURL`'s `icon://` form. (v19.)
    ScriptIcons {
        /// One per script command.
        icons: Vec<(String, String)>,
    },
    /// Answer to [`Request::LocalStorageNamespaces`], sorted. (v19.)
    LocalStorageNamespaces {
        /// The namespaces.
        namespaces: Vec<String>,
    },
    /// Answer to [`Request::LocalStorageItems`], by key. (v19.)
    LocalStorageItems {
        /// The items.
        items: Vec<LocalStorageEntry>,
    },
    /// Answer to [`Request::OAuthTokenSets`]. (v19.)
    OAuthTokenSets {
        /// The token sets.
        sets: Vec<OAuthTokenSetEntry>,
    },
    /// Answer to [`Request::UpdateStatus`]. (v21.)
    UpdateStatus {
        /// The version the engine is, such as `v0.1.0`.
        current: String,
        /// The newer release, or `None`: up to date, checking off, or the
        /// feed not reached.
        available: Option<UpdateOffer>,
    },
    /// Answer to [`Request::ExchangeRates`] and
    /// [`Request::RefreshExchangeRates`]: `None` when the engine has none,
    /// neither cached nor fetched. (v21.)
    ExchangeRates {
        /// The rates.
        rates: Option<ExchangeRateTable>,
    },
    /// Answer to [`Request::ProbeShortcut`]: why the desktop refused the
    /// combination, or `None` when it would bind it or cannot say. (v21.)
    ShortcutProbe {
        /// The desktop's reason.
        refusal: Option<String>,
    },
    /// Answer to [`Request::LauncherShortcut`]. (v25.)
    LauncherShortcut {
        /// Where the launcher hotkey stands.
        state: LauncherShortcutState,
    },
    /// Answer to [`Request::LoginStart`]: the state after the change, if
    /// there was one. (v25.)
    LoginStart {
        /// Whether Compass starts at login, and how.
        state: LoginStartState,
        /// The packaged systemd user unit, when it is installed.
        unit: Option<LoginStartUnit>,
    },
    /// Answer to [`Request::ClipboardStatus`]. (v26.)
    ClipboardStatus(ClipboardStatus),
}

/// The launcher hotkey's state with the desktop, in a
/// [`Response::LauncherShortcut`] (`compass_core::launcher_shortcut`). (v25.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LauncherShortcutState {
    /// Waiting for the first-run flow to ask.
    NotRequested,
    /// Asked; the desktop has not answered.
    Pending,
    /// Bound, with the desktop's description of the key when it gave one.
    Granted {
        /// The key, in the desktop's words.
        trigger: Option<String>,
    },
    /// Answered without binding the launcher's key.
    Conflict {
        /// Why, as a sentence.
        reason: String,
    },
    /// The person declined the desktop's dialog.
    Refused,
    /// No GlobalShortcuts portal and no compositor hotkey protocol.
    Unavailable,
    /// Asking failed.
    Failed {
        /// Why, as a sentence.
        reason: String,
    },
    /// No launcher hotkey is configured.
    Unset,
    /// The engine runs with `--no-hotkey`.
    Off,
}

/// Whether Compass starts at login, in a [`Response::LoginStart`]
/// (`compass_core::login_start`). (v25.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoginStartState {
    /// It does not.
    Off,
    /// The desktop starts it, through the Background portal.
    Portal,
    /// Compass's own XDG autostart entry starts it.
    AutostartFile,
    /// The desktop declined the last request.
    Refused,
    /// This session has no way to start it at login.
    Unavailable,
}

/// The packaged systemd user unit, in a [`Response::LoginStart`]. (v25.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoginStartUnit {
    /// Installed and not enabled.
    Installed,
    /// Enabled.
    Enabled,
}

/// Clipboard history as the running engine sees it, for `compass doctor`.
/// (v26.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardStatus {
    /// How copies reach the history; `None` until a backend has been chosen,
    /// which waits for the history to open.
    pub backend: Option<ClipboardBackend>,
    /// Whether that backend is delivering copies now.
    pub capturing: bool,
    /// Whether recording is switched on (the clipboard's `monitoring`).
    pub monitoring: bool,
    /// Why the backend is not delivering copies, when it is not.
    pub capture_problem: Option<String>,
    /// Whether the history database is open.
    pub storage: ClipboardStorage,
}

/// Where clipboard history's copies come from. (v26.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardBackend {
    /// The compositor's data-control protocol, by interface name
    /// (`ext_data_control_manager_v1`, `zwlr_data_control_manager_v1`).
    DataControl {
        /// The interface bound; `None` until a connection has bound one.
        protocol: Option<String>,
    },
    /// The Compass GNOME Shell extension's `ClipboardChanged` signal.
    ShellExtension,
}

/// The state of the history database in the running engine. (v26.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardStorage {
    /// Still asking the keyring for its key.
    Opening,
    /// Open, and recording.
    Open,
    /// The keyring is locked or absent; the engine tries again by itself.
    Waiting {
        /// What the keyring said.
        reason: String,
    },
    /// The engine has stopped trying.
    Failed {
        /// Why.
        reason: String,
        /// Whether `compass clipboard reset` is the way out: the key in the
        /// keyring does not open the history, or the keyring entry is not a
        /// key.
        reset: bool,
    },
}

/// One item of a [`Response::LocalStorageItems`]. (v19.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStorageEntry {
    /// Its key.
    pub key: String,
    /// Its value as text: a string as it is, anything else as JSON.
    pub value: String,
}

/// One token set of a [`Response::OAuthTokenSets`]. (v19.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthTokenSetEntry {
    /// The extension it belongs to.
    pub extension_id: String,
    /// The provider, or `None` for the unnamed one.
    pub provider_id: Option<String>,
    /// The bearer token.
    pub access_token: String,
    /// The refresh token.
    pub refresh_token: Option<String>,
    /// The id token.
    pub id_token: Option<String>,
    /// The granted scope.
    pub scope: Option<String>,
    /// When it expires, in seconds since the epoch, when it does.
    pub expires_at: Option<i64>,
    /// Whether it has expired.
    pub expired: bool,
}

/// Which system default a picker sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefaultAppKind {
    /// The web browser.
    Browser,
    /// The terminal emulator.
    Terminal,
}

/// One application a default picker offers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultAppEntry {
    /// The desktop file id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its comment.
    pub description: String,
    /// Whether it is the current default.
    pub is_default: bool,
}

/// What the clipboard detail pane shows about one entry besides its content
/// (`ClipboardHistoryViewHost::loadDetail`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardDetail {
    /// [`ClipboardEntry::id`].
    pub id: String,
    /// The preferred offer's MIME type.
    pub mime_type: String,
    /// What kind of thing it is.
    pub kind: ClipboardKind,
    /// The payload's size in bytes.
    pub size: i64,
    /// Its MD5, as the store keeps it.
    pub md5: String,
    /// When it was last copied, in milliseconds since the Unix epoch.
    pub updated_at: i64,
    /// Whether the payload is encrypted at rest.
    pub encrypted: bool,
    /// The words it is also found by; empty for none.
    pub keywords: String,
    /// Whether it is pinned.
    pub pinned: bool,
}

/// One group of [`Response::CalculatorHistory`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalculatorGroup {
    /// The section's name.
    pub name: String,
    /// Its rows, pinned first and newest first.
    pub records: Vec<CalculatorRecord>,
}

/// One remembered calculation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalculatorRecord {
    /// Its id, for [`Request::EditCalculatorHistory`].
    pub id: String,
    /// What was asked.
    pub question: String,
    /// What came back.
    pub answer: String,
    /// A unit conversion rather than arithmetic.
    pub conversion: bool,
    /// Whether it is pinned.
    pub pinned: bool,
}

/// A change to the calculator's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalculatorEdit {
    /// Pin a row, by id.
    Pin(String),
    /// Unpin a row, by id.
    Unpin(String),
    /// Remove a row, by id.
    Remove(String),
    /// Remove every row.
    RemoveAll,
}

/// One root item, as `compass cmd ls` lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandInfo {
    /// Its [`QueryHit::id`].
    pub id: String,
    /// Its title.
    pub name: String,
}

/// What the user has allowed one Rhai script.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptGrantEntry {
    /// The script's id.
    pub id: String,
    /// Its title, or its id when it is no longer installed.
    pub title: String,
    /// The capabilities allowed, e.g. `clipboard.write`.
    pub capabilities: Vec<String>,
    /// The same, in the consent prompt's words.
    pub descriptions: Vec<String>,
}

/// The keyboard helper behind snippet keyword expansion, as the engine sees
/// it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputServerStatus {
    /// `input_server.enabled`.
    pub enabled: bool,
    /// Whether the helper process is up and answered.
    pub running: bool,
    /// Whether it can type (its virtual keyboard was created). Keywords are
    /// still detected without it, but nothing is erased or pasted.
    pub injection: bool,
    /// How many keywords it watches for.
    pub keywords: u32,
    /// The helper binary found, if one was.
    pub helper: Option<String>,
    /// Why it is not working, when it is not: not installed, inside a
    /// Flatpak, no permission, gave up after crashing.
    pub problem: Option<String>,
}

/// One running media player, as Now Playing lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaPlayerEntry {
    /// Its bus name.
    pub id: String,
    /// What it calls itself.
    pub identity: String,
    /// Its `DesktopEntry`, when it offers one.
    pub app_id: String,
    /// The current track's title.
    pub title: String,
    /// The current track's artists.
    pub artist: String,
    /// Whether it is playing.
    pub playing: bool,
    /// Whether it is paused (neither this nor `playing` is stopped).
    pub paused: bool,
    /// Whether it has a next track.
    pub can_go_next: bool,
    /// Whether it has a previous track.
    pub can_go_previous: bool,
}

/// What [`Request::ControlMediaPlayer`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaPlayerAction {
    /// Toggle playback.
    PlayPause,
    /// Skip to the next track.
    Next,
    /// Go back to the previous track.
    Previous,
}

/// One Rhai script, as root search and the launcher need it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RhaiScriptEntry {
    /// `script.<folder name>`; the root entry is `rhai:<id>`.
    pub id: String,
    /// The manifest's `title`.
    pub title: String,
    /// The manifest's `description`.
    pub description: Option<String>,
    /// A builtin icon name.
    pub icon: Option<String>,
    /// Extra search terms.
    pub keywords: Vec<String>,
}

/// Which extension store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StoreKind {
    /// The Vicinae extension store.
    Vicinae,
    /// The Raycast store.
    Raycast,
}

/// One extension as a store's list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreEntry {
    /// The id it installs under, e.g. `store.raycast.spotify-player`.
    pub id: String,
    /// Its name in the store.
    pub name: String,
    /// Its author's handle.
    pub author: String,
    /// Its author's display name.
    pub author_name: String,
    /// Its title.
    pub title: String,
    /// What it does.
    pub description: String,
    /// Its icon's URL for a light theme.
    pub icon_light: Option<String>,
    /// Its icon's URL for a dark theme.
    pub icon_dark: Option<String>,
    /// Its download count, formatted (`1.1K`).
    pub downloads: String,
    /// Whether it is installed.
    pub installed: bool,
    /// Whether it is installed and the store serves a newer build.
    pub update_available: bool,
    /// Its Raycast compatibility tier (0 compatible, 1 partial,
    /// 2 incompatible, 3 unknown); `None` where there is no sheet.
    pub compat: Option<u8>,
    /// Its author's avatar URL, when the store has one. (v16.)
    pub author_avatar: Option<String>,
    /// The handle the store files it under, which its detail page, its
    /// install and its deeplinks are addressed by: an organisation's for a
    /// Raycast extension an organisation owns (`raycast/github`, whose
    /// author is a person), else the author's. (v23.)
    pub owner: String,
}

/// One extension's detail page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreDetail {
    /// The row it was opened from.
    pub entry: StoreEntry,
    /// The page's text: header, facts, compatibility, commands and README.
    pub markdown: String,
    /// Screenshot URLs.
    pub screenshots: Vec<String>,
    /// Where its README is.
    pub readme_url: Option<String>,
    /// Where its source is.
    pub source_url: Option<String>,
    /// Its page on the store's website.
    pub store_url: Option<String>,
}

/// One script command, as root search and the launcher need it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptEntry {
    /// The dotted id the scan gave it, from its path below its directory.
    pub id: String,
    /// `@raycast.title`.
    pub title: String,
    /// Its package name, or an inline script's last line of output.
    pub subtitle: String,
    /// Extra search terms.
    pub keywords: Vec<String>,
    /// `fullOutput`, `compact`, `inline`, `silent` or `terminal`.
    pub mode: String,
    /// Whether to ask before running it.
    pub needs_confirmation: bool,
    /// Where the file is.
    pub path: String,
    /// What it asks for, in order.
    pub arguments: Vec<ScriptArgumentEntry>,
}

/// One argument a script command declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptArgumentEntry {
    /// `text`, `password` or `dropdown`.
    pub kind: String,
    /// The field's placeholder, if it declares one.
    pub placeholder: Option<String>,
    /// Whether it may be left empty.
    pub optional: bool,
    /// A dropdown's options, as `(title, value)`.
    pub options: Vec<(String, String)>,
}

/// One stored snippet, as `snippets.json` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetEntry {
    /// `snp-` and twelve hex characters.
    pub id: String,
    /// What the user called it.
    pub name: String,
    /// Its text, for a text snippet.
    pub text: Option<String>,
    /// Its file, for a file snippet.
    pub file: Option<String>,
    /// When it was created, in Unix seconds.
    pub created_at: u64,
    /// When it was last edited, in Unix seconds, if it was.
    pub updated_at: Option<u64>,
    /// The keyword that expands it, if it has one.
    pub keyword: Option<String>,
    /// Whether the keyword waits for a word boundary.
    pub word: bool,
    /// The applications the keyword is limited to.
    pub apps: Vec<String>,
}

/// One family in Browse Fonts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontEntry {
    /// The typeface's name, its members folded together.
    pub name: String,
    /// The member to draw it with.
    pub family: String,
    /// The glyph its row shows, in its own script.
    pub glyph: Option<String>,
    /// Whether it is a colour emoji font.
    pub color: bool,
    /// The category it is listed under.
    pub primary: String,
    /// Every category it can be filtered by.
    pub categories: Vec<String>,
}

/// One stored shortcut (quicklink), as `shortcuts.json` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutEntry {
    /// `sct-` and twelve hex characters.
    pub id: String,
    /// What the user called it; may be empty.
    pub name: String,
    /// Its icon, as an image URL.
    pub icon: String,
    /// The link, `{placeholders}` and all.
    pub url: String,
    /// The application id that opens it, or `default`.
    pub app: String,
    /// How many times it has been opened.
    pub open_count: i64,
    /// When it was created, in Unix seconds.
    pub created_at: u64,
    /// When it was last edited, in Unix seconds.
    pub updated_at: u64,
    /// When it was last opened, in Unix seconds, if it ever was.
    pub last_used_at: Option<u64>,
}

/// One file in a [`Response::Files`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHit {
    /// Absolute path.
    pub path: String,
    /// The last path component, for the row's title.
    pub name: String,
    /// Its category's filter key, e.g. `Documents` or `Directories`.
    pub category: String,
}

/// What the engine asks an attached window to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowCommand {
    /// Become visible and take focus.
    Show,
    /// Become hidden.
    Hide,
    /// Hide if visible, show if not.
    Toggle,
    /// Show, with the dmenu list the engine holds under this token; the
    /// window fetches it with [`Request::DmenuFetch`] and answers the choice
    /// with [`Request::DmenuChoose`]. Answered, like `Show`, with
    /// [`WindowOutcome::Shown`] once visible.
    Dmenu(u64),
    /// Show, and take the launch an extension asked for under this token:
    /// fetched with [`Request::ExtensionLaunchFetch`]. Answered like `Show`.
    Launch(u64),
    /// Show, at what the deeplink names (a store extension's detail page).
    /// Answered, like `Show`, with [`WindowOutcome::Shown`]. (v16.)
    Deeplink(String),
    /// Change nothing; answer [`WindowOutcome::Shown`] if the window is on
    /// screen and [`WindowOutcome::Hidden`] if not. (v17.)
    Describe,
    /// Show the HUD, the pill the C++ shows for a moment after an action
    /// (`NavigationController::showHud`), leaving the launcher as it is.
    /// Answered with the launcher's state when shown, and
    /// [`WindowOutcome::Failed`] where the presentation has none (an
    /// `xdg_toplevel` cannot appear without taking the focus). (v19.)
    Hud {
        /// The line of text.
        text: String,
        /// A builtin icon's name or an emoji.
        icon: Option<String>,
    },
    /// A command the launcher let go of failed: a no-view extension command
    /// that refused or crashed after the launcher hid. The launcher keeps it
    /// as its error line, there when it is next shown, and says it in the
    /// HUD now where it has one. Answered with the launcher's state. (v23.)
    Failure {
        /// What failed, e.g. "Could not complete New Window".
        title: String,
        /// Why, e.g. "AppleScript is only supported on macOS"; may be empty.
        message: String,
    },
}

/// What `compass dmenu` asks the launcher to show: its stdin as a list, and
/// the C++ CLI's options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmenuSpec {
    /// The entries, one per line; empty lines are dropped.
    pub content: String,
    /// `--navigation-title`.
    pub navigation_title: Option<String>,
    /// `--section-title`, where `{count}` is the number shown.
    pub section_title: Option<String>,
    /// `--format`: print the entry (`false`) or its index (`true`).
    pub output_index: bool,
    /// `--placeholder`.
    pub placeholder: Option<String>,
    /// `--query`, the initial search text.
    pub query: Option<String>,
    /// `--width`.
    pub width: Option<u32>,
    /// `--height`.
    pub height: Option<u32>,
    /// `--no-section`.
    pub no_section: bool,
    /// `--no-quick-look`.
    pub no_quick_look: bool,
    /// `--no-metadata`.
    pub no_metadata: bool,
    /// `--no-footer`.
    pub no_footer: bool,
}

/// What an attached window reports back after acting on a [`WindowCommand`].
///
/// [`Self::Shown`] and [`Self::Hidden`] report the state the window ended in,
/// not the command it was given — which is the only useful answer to
/// [`WindowCommand::Toggle`], and lets a caller of `show` on an
/// already-visible window learn that nothing changed without a second round
/// trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowOutcome {
    /// The window is now visible.
    Shown,
    /// The window is now hidden.
    Hidden,
    /// The window could not carry the command out, with a reason to print.
    Failed(String),
}

/// One ranked search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryHit {
    /// Stable identifier of the underlying root item.
    pub id: String,
    /// Primary display text.
    pub title: String,
    /// Secondary display text, when the item has one.
    pub subtitle: Option<String>,
    /// Match score in `0..=100`, matching `compass-search`'s scale.
    pub score: u32,
}

/// One preference as the launcher's form draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferenceField {
    /// The name the extension reads it by; the key in `values_json`.
    pub name: String,
    /// The label.
    pub title: String,
    /// The help text; may be empty.
    pub description: String,
    /// The placeholder; may be empty.
    pub placeholder: String,
    /// Whether the command cannot run without it.
    pub required: bool,
    /// What kind of input it takes.
    pub kind: PreferenceFieldKind,
    /// Its current value (stored, else the default), as JSON.
    pub value_json: Option<String>,
}

/// What a [`PreferenceField`] takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreferenceFieldKind {
    /// A line of text.
    Text,
    /// A line of text, not echoed.
    Password,
    /// A tick box, with its label.
    Checkbox {
        /// The text beside the box.
        label: String,
    },
    /// One of a list, as `(title, value)`.
    Dropdown {
        /// The options.
        options: Vec<(String, String)>,
    },
    /// A kind the form cannot edit yet (a file or application picker); shown
    /// so the person sees why the command waits.
    Unsupported {
        /// What the manifest calls it.
        declared: String,
    },
}

/// A confirmation an extension asked for (`confirmAlert`), as the launcher
/// shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionAlert {
    /// The heading.
    pub title: String,
    /// The body; may be empty.
    pub message: String,
    /// The confirm button's text.
    pub confirm_text: String,
    /// The cancel button's text.
    pub cancel_text: String,
    /// A third answer that confirms and is remembered ("Always Allow"), or
    /// `None` for an alert with two. Answered with
    /// [`Request::ExtensionAlertRemember`]. (v22.)
    pub remember_text: Option<String>,
}

/// A toast an extension shows over its view (`showToast`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionToast {
    /// The heading.
    pub title: String,
    /// More text; may be empty.
    pub message: String,
    /// How it reads.
    pub style: ExtensionToastStyle,
}

/// A toast's style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtensionToastStyle {
    /// Done.
    Success,
    /// For information.
    Info,
    /// Something to notice.
    Warning,
    /// Something failed.
    Failure,
    /// Still working.
    Animated,
}

/// One clipboard history entry, as a list row needs it.
///
/// The payload itself is not sent: rows show [`preview`](Self::preview), and
/// copying an entry back is a separate request once there is one to make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardEntry {
    /// Stable id of the entry.
    pub id: String,
    /// Text shown in the row: the start of copied text, or a label such as
    /// "Image" for content that has none.
    pub preview: String,
    /// MIME type of the preferred representation.
    pub mime_type: String,
    /// What kind of thing was copied.
    pub kind: ClipboardKind,
    /// Whether the entry is pinned to the top.
    pub pinned: bool,
    /// When it was last copied, in milliseconds since the Unix epoch.
    pub updated_at: i64,
    /// For links, the host, so a row can say where it points.
    pub url_host: Option<String>,
}

/// One open window, as the switcher shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    /// Handle for [`Request::ActivateWindow`] and [`Request::CloseWindow`].
    pub id: u32,
    /// The window's title.
    pub title: String,
    /// Its `WM_CLASS`.
    pub wm_class: String,
    /// The application it belongs to, when the engine recognised one.
    pub app_name: Option<String>,
    /// That application's icon name.
    pub app_icon: Option<String>,
    /// The owning process, so a client can leave out its own windows.
    pub pid: Option<u32>,
    /// Workspace index, when known.
    pub workspace: Option<i32>,
    /// Whether it has focus right now.
    pub focused: bool,
    /// Whether it can be closed.
    pub can_close: bool,
}

/// What kind of thing a [`ClipboardEntry`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardKind {
    /// Plain text.
    Text,
    /// A URL.
    Link,
    /// An image.
    Image,
    /// One or more files.
    File,
    /// Anything the store kept but could not classify.
    Unknown,
}

/// One diagnostic check performed by `compass doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    /// Short machine-ish name, e.g. `"portal.global-shortcuts"`.
    pub name: String,
    /// Outcome of the check.
    pub status: DoctorStatus,
    /// Human-readable detail, when there is something to say.
    pub detail: Option<String>,
}

/// Outcome of a single [`DoctorCheck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoctorStatus {
    /// Working as intended.
    Ok,
    /// Degraded but usable.
    Warn,
    /// Broken.
    Fail,
}

/// A failure reported by the server in [`Response::Error`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    /// Machine-readable category.
    pub kind: ErrorKind,
    /// Human-readable explanation, safe to print to a terminal.
    pub message: String,
}

impl ProtocolError {
    /// Builds an error of `kind` with `message`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

/// Category of a [`ProtocolError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    /// Peer speaks a protocol version this build cannot serve.
    VersionMismatch,
    /// The request was understood but this build does not implement it.
    Unsupported,
    /// The request was malformed.
    BadRequest,
    /// The handler failed.
    Internal,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::VersionMismatch => "version mismatch",
            Self::Unsupported => "unsupported",
            Self::BadRequest => "bad request",
            Self::Internal => "internal error",
        };
        f.write_str(s)
    }
}

/// Builds the [`Response::Error`] the server sends when a peer's envelope
/// carries a version this build does not speak.
#[must_use]
pub fn version_mismatch(peer: u16) -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::VersionMismatch,
        format!(
            "peer speaks compass-ipc protocol v{peer}, this build speaks v{PROTOCOL_VERSION}; \
             restart the client and the engine from the same build"
        ),
    ))
}
