//! Messages for the launcher application.

use iced::Event;

use crate::resident::UiCommand;

/// Which way the selection moves.
///
/// An enum rather than a signed delta: the only two motions a launcher list has
/// are next and previous, and a number invites callers to invent a third.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Towards the top of the list.
    Up,
    /// Towards the bottom of the list.
    Down,
}

/// Messages that the launcher application can handle.
#[derive(Debug, Clone)]
pub enum Message {
    /// Initialize the application (connect portals, bind shortcuts).
    Initialize,
    /// The window is about to draw a frame.
    ///
    /// Only the first one is acted on, to time cold start (#13 §8.5). Iced
    /// yields this on `RedrawRequested`, which is the moment the compositor
    /// asks for a frame rather than the moment one reaches the screen -- see
    /// [`crate::app::LauncherApp`]'s handling for why that distinction is
    /// recorded rather than glossed.
    FrameDrawn,
    /// The search query changed.
    QueryChanged(String),
    /// An asynchronous search completed. Only the current generation may apply.
    SearchCompleted {
        /// Generation captured when the request started.
        generation: u64,
        /// Stable application keys, in backend ranking order, or a failure.
        result: Result<Vec<String>, String>,
    },
    /// A result was selected (by keyboard navigation).
    ResultSelected(usize),
    /// A root row was clicked, by position: select it and do what Enter on
    /// it does.
    ResultClicked(usize),
    /// Move the selection one row, wrapping at both ends.
    MoveSelection(Direction),
    /// Launch the selected result.
    LaunchSelected,
    /// The clipboard history filter changed.
    ClipboardQueryChanged(String),
    /// Clipboard history rows arrived for request `generation`.
    ClipboardLoaded {
        /// The request they answer; a stale one is dropped.
        generation: u64,
        /// The rows, or why there are none.
        result: Result<Vec<crate::backend::ClipboardRow>, String>,
    },
    /// The selected clipboard entry's content arrived, to be copied.
    ClipboardContentLoaded(Result<crate::backend::ClipboardContent, String>),
    /// The engine armed a paste of the selected entry, or could not; on a
    /// refusal the entry is copied instead.
    ClipboardPasted(Result<(), String>),
    /// The engine put text on its clipboard, or could not; on a refusal the
    /// window copies it itself.
    TextCopied {
        /// What was copied.
        text: String,
        /// The engine's answer.
        result: Result<(), String>,
    },
    /// The engine pasted a glyph from the emoji picker, or could not; on a
    /// refusal the glyph is copied instead.
    EmojiPasted {
        /// The glyph, as it would be copied.
        text: String,
        /// The engine's answer.
        result: Result<(), String>,
    },
    /// The engine started an extension command, or said why it could not.
    ExtensionCommandStarted {
        /// The command's entrypoint id.
        id: String,
        /// The command's title, for its view until the view names itself.
        title: String,
        /// How it began, or why it could not.
        result: Result<crate::backend::ExtensionStart, String>,
    },
    /// An extension view's latest state, for the session it asked about.
    ExtensionViewLoaded {
        /// Which session.
        session: u64,
        /// Its state, or why it could not be read.
        result: Result<crate::backend::ExtensionViewState, String>,
    },
    /// The search text in an extension's view changed.
    ExtensionQueryChanged(String),
    /// A row in an extension's list was clicked.
    ExtensionItemSelected(usize),
    /// A preference field in the form changed.
    PreferenceEdited(usize, crate::preferences_page::FieldValue),
    /// The preference form was submitted (Enter).
    PreferencesSubmit,
    /// The preferences were kept, or not; on success the command runs.
    PreferencesSaved(Result<(), String>),
    /// A link in an extension's Markdown was clicked.
    ExtensionLinkClicked(String),
    /// The person changed a field of an extension's form: its name and value.
    ExtensionFieldEdited(String, serde_json::Value),
    /// An edit in a form's text area: its name and the editor action.
    ExtensionTextAreaEdited(String, iced::widget::text_editor::Action),
    /// An action or search event reached the extension, or did not.
    ExtensionEventSent(Result<(), String>),
    /// An entry was pinned, unpinned or removed, or could not be; the list
    /// reloads on success and says why on failure.
    ClipboardEntryChanged(Result<(), String>),
    /// A clipboard row was clicked.
    ClipboardSelected(usize),
    /// The clipboard kind filter was changed, to this label.
    ClipboardKindChanged(String),
    /// The detail pane's metadata for entry `id` arrived.
    ClipboardDetailLoaded {
        /// Which entry.
        id: String,
        /// What the engine answered.
        result: Result<crate::backend::ClipboardDetail, String>,
    },
    /// The detail pane's content for entry `id` arrived.
    ClipboardDetailContent {
        /// Which entry.
        id: String,
        /// What the engine answered.
        result: Result<crate::backend::ClipboardContent, String>,
    },
    /// An entry's keywords arrived, to open the keyword form with.
    ClipboardKeywordsLoaded(Result<crate::backend::ClipboardDetail, String>),
    /// Whether copies are being recorded, as the engine answered.
    ClipboardMonitoringLoaded(Result<crate::backend::ClipboardMonitoring, String>),
    /// The engine kept (or refused) what the root row's panel changed.
    RootItemEdited(Result<(), String>),
    /// A second passed; the root search's clock may need redrawing.
    ClockTick,
    /// Leave a command's view for the root list.
    Back,
    /// The window switcher's filter changed.
    WindowsQueryChanged(String),
    /// The engine answered a power or media command.
    BuiltinCommandDone(Result<(), String>),
    /// Now Playing's players arrived.
    NowPlayingLoaded(Result<Vec<crate::backend::MediaPlayerRow>, String>),
    /// Now Playing's filter changed.
    NowPlayingQueryChanged(String),
    /// A player row was clicked, by position in the shown list.
    NowPlayingSelected(usize),
    /// A player did what it was asked, or could not.
    NowPlayingActed(Result<(), String>),
    /// The emoji picker's filter changed.
    EmojiQueryChanged(String),
    /// An emoji row was clicked, by position in the shown list.
    EmojiSelected(usize),
    /// Search Files' text changed.
    FilesQueryChanged(String),
    /// Search Files' category filter changed, by its key.
    FilesCategoryChanged(String),
    /// The debounce for Search Files query `generation` ran out; ask, unless
    /// the text moved on meanwhile.
    FilesDebounced(u64),
    /// The root's answer to search `generation` has stood long enough to be
    /// described to assistive technology (`LauncherApp::results_settling`).
    ResultsSettled(u64),
    /// Search Files' answer to query `generation` arrived.
    FilesLoaded {
        /// The query it answers; a stale one is dropped.
        generation: u64,
        /// The heading and files, or why there are none.
        result: Result<crate::backend::FileResults, String>,
    },
    /// A file row was clicked, by position.
    FilesSelected(usize),
    /// Opening a file (or showing it in the file browser) finished.
    FileOpened(Result<(), String>),
    /// An edit in a form's text area, by field position.
    PreferenceTextEdited(usize, iced::widget::text_editor::Action),
    /// The snippet list arrived (on opening Manage Snippets, or after a
    /// change), or why it could not be read.
    SnippetsLoaded(Result<Vec<crate::backend::Snippet>, String>),
    /// A snippet was saved (the list after it), or why it was not.
    SnippetSaved(Result<Vec<crate::backend::Snippet>, String>),
    /// A snippet's expansion, to copy, or why it could not be expanded.
    SnippetExpanded(Result<String, String>),
    /// Pasting a snippet finished.
    SnippetPasted(Result<(), String>),
    /// Manage Snippets' filter changed.
    SnippetsQueryChanged(String),
    /// A Manage Snippets row was clicked, by position.
    SnippetSelected(usize),
    /// Manage Snippets' detail pane for a snippet arrived.
    SnippetDetailLoaded(crate::snippets_page::Detail),
    /// Create Extension finished, for the extension called `title`: where
    /// it was written, or why not.
    ExtensionCreated {
        /// The extension's title.
        title: String,
        /// Its path, or the reason.
        result: Result<String, String>,
    },
    /// Opening the new extension's folder finished.
    CreatedFolderOpened(Result<(), String>),
    /// A store's rows arrived for search `generation`, or why not.
    StoreLoaded {
        /// The search text's generation when it was asked for.
        generation: u64,
        /// The rows.
        result: Result<crate::backend::StoreList, String>,
    },
    /// A store's search text changed.
    StoreQueryChanged(String),
    /// A store's search text has settled for generation `u64`.
    StoreSearchDue(u64),
    /// A store row was clicked, by position.
    StoreSelected(usize),
    /// A store extension's detail page arrived, or why not.
    StoreDetailLoaded(Result<crate::backend::StoreDetail, String>),
    /// An install finished: the id and title, or why not.
    StoreInstalled(Result<(String, String), String>),
    /// An uninstall of `id` finished.
    StoreUninstalled {
        /// The id.
        id: String,
        /// Whether it worked.
        result: Result<(), String>,
    },
    /// Opening a store link finished.
    StoreUrlOpened(Result<(), String>),
    /// Browse Fonts' families arrived, or why they could not be listed.
    FontsLoaded(Result<crate::backend::FontList, String>),
    /// Browse Fonts' search text changed.
    FontsQueryChanged(String),
    /// Browse Fonts' category filter changed, to the option titled so.
    FontsCategoryChanged(String),
    /// "Set as Compass font" was saved, with the family, or could not be.
    FontSet(Result<String, String>),
    /// Search Tray's items arrived.
    TrayItemsLoaded(Result<Vec<crate::backend::TrayItemRow>, String>),
    /// A tray item's menu arrived.
    TrayMenuLoaded {
        /// The item's key.
        key: String,
        /// Its entries, or why not.
        result: Result<Vec<crate::backend::TrayMenuRow>, String>,
    },
    /// Search Tray's filter changed.
    TrayQueryChanged(String),
    /// A Search Tray row was clicked, by position in the shown list.
    TraySelected(usize),
    /// A tray action ran: `true` when the launcher should close.
    TrayActed(Result<bool, String>),
    /// Script Permissions' list arrived.
    GrantsLoaded(Result<Vec<crate::backend::ScriptGrant>, String>),
    /// Browse Apps' or a default picker's filter changed.
    AppsQueryChanged(String),
    /// A row of Browse Apps or a default picker was clicked.
    AppsSelected(usize),
    /// Whether Browse Apps' selected application has windows open.
    BrowseAppRuntime {
        /// The application's desktop id, so a late answer is dropped.
        id: String,
        /// Its windows.
        result: Result<crate::backend::AppRuntimeInfo, String>,
    },
    /// A default picker's candidates arrived.
    DefaultAppsLoaded(Result<Vec<crate::backend::DefaultAppRow>, String>),
    /// A default picker's choice was written, or could not be.
    DefaultAppSet(Result<(), String>),
    /// The engine's catalog generation, asked on every summon: when it moved,
    /// applications or extensions were installed or removed.
    CatalogGeneration(Result<u64, String>),
    /// Script Permissions' filter changed.
    GrantsQueryChanged(String),
    /// A script row was clicked, by position in the shown list.
    GrantSelected(usize),
    /// A revoke was done, with the list after it, or could not be.
    GrantRevoked(Result<Vec<crate::backend::ScriptGrant>, String>),
    /// The uninstall dialog was answered: `true` uninstalls.
    StoreConfirmAnswered(bool),
    /// A Browse Fonts row was clicked, by position.
    FontSelected(usize),
    /// A family's specimen arrived, or why not.
    FontSpecimenLoaded {
        /// The typeface's name.
        name: String,
        /// The member to draw it with.
        family: String,
        /// The specimen's Markdown.
        result: Result<String, String>,
    },
    /// Set Theme's search text changed.
    ThemesQueryChanged(String),
    /// A Set Theme row was clicked, by position.
    ThemeSelected(usize),
    /// The chosen theme was kept, or why not.
    ThemeSaved(Result<(), String>),
    /// A dmenu list arrived for `token`, or why it could not be fetched.
    DmenuLoaded {
        /// Which list.
        token: u64,
        /// The list.
        result: Result<crate::backend::DmenuList, String>,
    },
    /// The dmenu view's search text changed.
    DmenuQueryChanged(String),
    /// A dmenu entry was clicked, by position.
    DmenuSelected(usize),
    /// The dmenu choice reached the engine, or why it did not.
    DmenuChosen(Result<(), String>),
    /// Run Terminal Program's programs arrived, or why they could not.
    ProgramsLoaded(Result<crate::backend::ProgramList, String>),
    /// Run Terminal Program's text changed.
    ProgramsQueryChanged(String),
    /// A Run Terminal Program row was clicked, by position.
    ProgramSelected(usize),
    /// Running a program finished starting, or why it did not.
    ProgramRan(Result<(), String>),
    /// The script commands arrived, or why they could not be listed.
    ScriptsLoaded(Result<Vec<compass_core::script_scan::ScriptItem>, String>),
    /// The script commands' icons arrived, `(id, icon URL)`, or why not.
    ScriptIconsLoaded(Result<Vec<(String, String)>, String>),
    /// The Rhai scripts arrived, or why they could not be listed.
    RhaiScriptsLoaded(Result<Vec<compass_core::rhai_scripts::RhaiScriptItem>, String>),
    /// The launch an extension asked for arrived, or why it could not.
    LaunchFetched(Result<crate::backend::ExtensionLaunch, String>),
    /// The subtitles extensions set for their commands arrived.
    ExtensionSubtitlesLoaded(Result<Vec<(String, String)>, String>),
    /// The engine's answer to whether a newer Compass release is out.
    UpdateStatusLoaded(Result<Option<crate::backend::UpdateOffer>, String>),
    /// "Skip this version" finished for the tag.
    UpdateSkipped(String, Result<(), String>),
    /// A command's preferences form arrived, to edit without running it.
    PreferencesOpened {
        /// The command's root id.
        id: String,
        /// The form, or why it could not be had.
        result: Result<crate::backend::ExtensionStart, String>,
    },
    /// A script started: the run to follow, if any.
    ScriptStarted {
        /// Which script.
        id: String,
        /// The arguments it ran with.
        arguments: Vec<String>,
        /// The run, or why it did not start.
        result: Result<Option<u64>, String>,
    },
    /// A report on script run `session` arrived.
    ScriptPolled {
        /// Which run.
        session: u64,
        /// What it printed so far, or why it could not be read.
        result: Result<crate::backend::ScriptOutputState, String>,
    },
    /// The shortcut list arrived, or why it could not be read.
    ShortcutsLoaded(Result<Vec<crate::backend::Shortcut>, String>),
    /// A shortcut was saved (the list after it), or why it was not.
    ShortcutSaved(Result<Vec<crate::backend::Shortcut>, String>),
    /// A shortcut was removed (the list after it), or why it was not.
    ShortcutRemoved(Result<Vec<crate::backend::Shortcut>, String>),
    /// Opening a shortcut finished.
    ShortcutOpened(Result<(), String>),
    /// A shortcut's expanded link, to copy, or why it could not be expanded.
    ShortcutExpanded(Result<String, String>),
    /// Manage Shortcuts' filter changed.
    ShortcutsQueryChanged(String),
    /// A Manage Shortcuts row was clicked, by position.
    ShortcutSelected(usize),
    /// The open windows arrived, or why they could not be listed.
    WindowsLoaded(Result<Vec<crate::backend::WindowRow>, String>),
    /// A window row was clicked.
    WindowSelected(usize),
    /// Switching to a window finished.
    WindowActivated(Result<(), String>),
    /// Closing a window finished; the list is reloaded either way.
    ShellWindowClosed(Result<(), String>),
    /// What the window manager can do, which decides the window-management
    /// commands root search offers.
    WindowCapabilities(Result<compass_core::window_switcher::Capabilities, String>),
    /// Switch Workspaces' filter changed.
    WorkspacesQueryChanged(String),
    /// The workspaces arrived, or why they could not be listed.
    WorkspacesLoaded(Result<Vec<crate::backend::WorkspaceRow>, String>),
    /// A workspace row was clicked, by position.
    WorkspaceSelected(usize),
    /// Switching to a workspace finished.
    WorkspaceFocused(Result<(), String>),
    /// A fullscreen, floating or overview toggle finished.
    WindowToggled(Result<(), String>),
    /// What "Open with…" opens and what its applications are looked up by,
    /// once worked out (a shortcut's link is expanded first).
    OpenWithTarget(Result<(String, String), String>),
    /// "Open with…"'s applications arrived.
    OpenersLoaded(Result<Vec<crate::backend::OpenerRow>, String>),
    /// "Open with…"'s filter changed.
    OpenWithQueryChanged(String),
    /// An "Open with…" row was clicked, by position.
    OpenWithSelected(usize),
    /// Opening with the chosen application finished.
    OpenedWith(Result<(), String>),
    /// What the selected file's action panel depends on arrived.
    FileActionsLoaded {
        /// The file it describes, so a late answer for another is dropped.
        path: String,
        /// What the panel depends on, or why it is unknown.
        result: Result<crate::backend::FileActions, String>,
    },
    /// A file action that hides the launcher on success finished.
    FileActionDone(Result<(), String>),
    /// Manage Shortcuts' detail pane for a shortcut arrived.
    ShortcutDetailLoaded(crate::shortcuts_page::Detail),
    /// Whether the application under the root row's panel runs, by its key.
    AppRuntimeLoaded {
        /// The application's key, so a late answer for another row is dropped.
        key: String,
        /// Whether it runs, and its windows.
        result: Result<crate::backend::AppRuntimeInfo, String>,
    },
    /// Quit, Force Quit, or a root row's Focus or Close Window finished.
    AppQuit(Result<(), String>),
    /// Calculator History's filter changed.
    CalculatorQueryChanged(String),
    /// Calculator History's rows arrived for request `generation`.
    CalculatorLoaded {
        /// The request they answer; a stale one is dropped.
        generation: u64,
        /// The groups, or why there are none.
        result: Result<Vec<crate::backend::CalculatorGroupRow>, String>,
    },
    /// A Calculator History row was clicked, by position.
    CalculatorSelected(usize),
    /// A pin, unpin or removal finished, with what to say.
    CalculatorEdited(Result<&'static str, String>),
    /// The engine's exchange rates arrived, for currency conversions.
    ExchangeRatesLoaded(Result<Option<compass_core::exchange_rates::ExchangeRates>, String>),
    /// Refresh Exchange Rates finished.
    ExchangeRatesRefreshed(Result<compass_core::exchange_rates::ExchangeRates, String>),
    /// A launch finished, successfully or not.
    ///
    /// Carried as a string rather than the error type because a `Message` must
    /// be `Clone` and `LaunchError` is not — and because the only thing the UI
    /// does with a failure is show it.
    Launched(Result<(), String>),
    /// Dismiss the launcher without launching anything.
    Dismiss,
    /// Open or close the action panel over the results.
    TogglePanel,
    /// The action panel's filter changed.
    PanelFilterChanged(String),
    /// Move the action panel's selection one row.
    PanelMove(Direction),
    /// Run the action panel's selected action.
    PanelActivate,
    /// Activate the clicked action row; headings and dividers are ignored.
    PanelClicked(usize),
    /// A global shortcut was activated.
    ShortcutActivated(String),
    /// Window focus changed.
    FocusChanged(bool),
    /// Window was closed.
    WindowClosed,
    /// Poll for shortcut events.
    PollShortcuts,
    /// Raw Iced event (for advanced handling).
    EventOccurred(Event),
    /// The engine asked the window to show, hide or toggle.
    Command(UiCommand),
    /// The engine closed its command channel.
    EngineDisconnected,
    /// The desktop's light/dark preference changed.
    AppearanceChanged(crate::design::Appearance),
    /// The desktop's interface font family changed.
    TypographyChanged(String),
    /// `compass.json` changed on disk and was read again.
    ConfigReloaded(std::sync::Arc<compass_core::Config>),
    /// Preview a theme without persisting it (#153 live preview).
    ThemePreview(crate::theme::Theme),
    /// Commit the previewed theme to config.
    ThemeCommit,
    /// Cancel preview and restore the theme from config.
    ThemeCancel,
    /// Close a window from the switcher (`ctrl+q`).
    ///
    /// The id is the plain `u32` the shell minted (see
    /// `compass_core::window_switcher::window_launch_target_for_app`), not the
    /// shell's `WindowId` type: this shared crate must never name it.
    CloseWindow(u32),
    /// A window finished opening, and this is its id.
    ///
    /// Carried separately from `Command(Show)` because the honest moment to
    /// report `Shown` is when the window exists, not when opening it was
    /// requested.
    Opened(iced::window::Id),
    /// A request for `iced_layershell`'s runtime, which takes it before
    /// `update` would see it. See [`crate::surface`].
    Layer(crate::surface::LayerRequest),
    /// A window was closed by the compositor or the user.
    ///
    /// Distinct from [`Message::Dismiss`]: this is the window telling us it is
    /// gone, not a request to make it go.
    Closed(iced::window::Id),
    /// The card was laid out at this size: shown, resized, or its
    /// translucency or corner radius changed. Where the blur goes (see
    /// `crate::material`).
    CardMeasured(iced::Size),
    /// A window was opened (`opened`) or resized to this logical size.
    WindowSized {
        /// The window.
        id: iced::window::Id,
        /// Its size, logical pixels.
        size: iced::Size,
        /// Whether this is its first size, from the compositor's first
        /// configure.
        opened: bool,
    },
    /// The logical size of the monitor a toplevel is on, when the toolkit
    /// knows it.
    MonitorSized(iced::window::Id, Option<iced::Size>),
    /// A click landed on the backdrop, outside the launcher.
    BackdropPressed,
    /// Leave for good.
    ///
    /// The one thing that still ends the process, now that dismissing only
    /// hides. See ADR-0015.
    Quit,
    /// A keyboard event that no widget consumed.
    ///
    /// Carries the whole event rather than a pre-digested action because the
    /// text input takes the printable keys first; what reaches here is exactly
    /// the set the launcher itself has to interpret.
    Keyboard(iced::keyboard::Event),
    /// A remote image an extension's view shows was fetched into the cache,
    /// or could not be.
    ExtensionImageFetched {
        /// The image's URL.
        url: String,
        /// Where it is kept, or why it is not.
        result: Result<std::path::PathBuf, String>,
    },
    /// The text in an extension form's date field changed: its name and
    /// the text as typed.
    ExtensionDateEdited(String, String),
    /// A file picker's button: ask the desktop's file chooser.
    ExtensionChooseFiles {
        /// The field's name.
        name: String,
        /// What it may choose.
        choice: crate::extension_fields::FileChoice,
    },
    /// The file chooser answered: the paths chosen (empty when cancelled),
    /// or why it could not be shown.
    ExtensionFilesChosen {
        /// The field's name.
        name: String,
        /// The chosen paths.
        result: Result<Vec<String>, String>,
    },
    /// Something in the settings view.
    Settings(crate::settings_page::SettingsMessage),
    /// A tick while the HUD is up, at this time.
    HudTick(std::time::Instant),
    /// An action that hides the launcher finished: on success it hides,
    /// with this HUD where there is one (`Quit Files`, `Wallpaper set`); on
    /// failure the reason shows in the view.
    ActionDone(Option<crate::hud::Hud>, Result<(), String>),
    /// The first-run flow's Continue (or Finish on its last step).
    OnboardingContinue,
    /// The first-run flow's Back.
    OnboardingBack,
    /// A step dot was clicked.
    OnboardingJump(usize),
    /// A theme was chosen in the first-run flow.
    OnboardingTheme(crate::onboarding_page::ThemeOption),
    /// One of the first-run flow's links was clicked.
    OnboardingOpen(&'static str),
    /// The link opened, or why not.
    OnboardingLinkOpened(Result<(), String>),
    /// Install was pressed on this recommended extension.
    OnboardingInstall(usize),
    /// That install finished: the installed id and title, or why not.
    OnboardingInstalled(usize, Result<(String, String), String>),
    /// The first-run flow's Set Up Shortcut or Try Again: ask the desktop
    /// for the launcher hotkey.
    OnboardingRequestShortcut,
    /// Where the launcher hotkey stands, as the engine answered.
    LauncherShortcutAnswered(Result<compass_core::launcher_shortcut::LauncherShortcut, String>),
    /// The login start switch was flipped, in the first-run flow or Settings.
    LoginStartToggled(bool),
    /// Whether Compass starts at login, as the engine answered, or why a
    /// change did not happen.
    LoginStartAnswered(Result<compass_core::login_start::Status, String>),
    /// Configure Fallback Commands' filter changed.
    FallbacksQueryChanged(String),
    /// A row of Configure Fallback Commands was clicked: its action runs.
    FallbackSelected(usize),
    /// Show Installed Extensions' filter changed.
    ExtensionsQueryChanged(String),
    /// Search Builtin Icons' filter changed.
    IconsQueryChanged(String),
    /// A row of Show Installed Extensions or Search Builtin Icons was
    /// clicked: its first action runs.
    CompassRowSelected(usize),
    /// An extension was uninstalled from Show Installed Extensions, or why
    /// not.
    ExtensionUninstalled {
        /// The extension's id.
        id: String,
        /// Whether it went.
        result: Result<(), String>,
    },
    /// Inspect Local Storage's filter changed.
    StorageQueryChanged(String),
    /// Manage OAuth Token Sets' filter changed.
    TokensQueryChanged(String),
    /// Inspect Local Storage's namespaces arrived, or why not.
    StorageNamespacesLoaded(Result<Vec<String>, String>),
    /// A namespace's items arrived, or why not.
    StorageItemsLoaded {
        /// The namespace.
        namespace: String,
        /// Its items.
        result: Result<Vec<crate::backend::StorageItemRow>, String>,
    },
    /// Manage OAuth Token Sets' list arrived, or why not.
    TokenSetsLoaded(Result<Vec<crate::backend::TokenSetRow>, String>),
    /// A token set was removed, or why not.
    TokenSetRemoved(Result<(), String>),
    /// The launcher window gained (`true`) or lost the keyboard focus.
    WindowFocusChanged(bool),
    /// The engine's answer to the recorder suspending or resuming the
    /// global shortcuts.
    ShortcutCaptureSet(Result<(), String>),
    /// The engine's answer to the recorder asking whether the desktop would
    /// bind a combination: the combination, and the refusal if any.
    ShortcutProbed(String, Result<Option<String>, String>),
    /// Assistive technology asked something of the window's accessibility
    /// tree: press a result, focus a field, set the search text
    /// ([`crate::accessibility`]).
    Accessibility(accesskit::ActionRequest),
}
