//! The launcher window: search, move, launch, dismiss.
//!
//! The state machine is deliberately separable from Iced. `update` takes a
//! [`Message`] and mutates fields; the only Iced-shaped things it returns are
//! [`Task`]s, which can be constructed without a running runtime. That is what
//! makes this crate testable in a container with no display server — the
//! selection and launch-target logic, which is where the bugs live, is exercised
//! by ordinary unit tests, and only the drawing needs a compositor.
//!
//! # Where things are
//!
//! This file is the coordinator: [`LauncherApp`]'s state, building it, and
//! `update`, which routes each [`Message`] to the module that owns it. The
//! behaviour lives in the `app/` modules, each an `impl LauncherApp` block so
//! it reaches the state without widening it:
//!
//! - the window: `lifecycle` (opening, hiding, the engine's commands),
//!   `keyboard` (who gets a key press), `view` (drawing the card, its theme)
//!   and `icons`;
//! - the root search: `search` (ranking, selection, launching), `root` (its
//!   favourites, aliases and history), `panel` (the action panel) and
//!   `commands` (opening a builtin command);
//! - one module per view a command opens: `clipboard`, `files`, `windows`,
//!   `emoji`, `extension`, `preferences`, `settings_view` and the rest;
//! - `state_line`, the log line every message leaves, and `flags`, what the
//!   launcher starts with.
//!
//! Tests sit beside the module they exercise; fixtures more than one module
//! uses are in `test_support`. The imports below double as the prelude the
//! `app/` modules take through `super::`.

use std::sync::Arc;

use iced::{
    Alignment, Border, Color, Element, Length, Padding, Task, Theme,
    widget::{
        Space, column, container, image, mouse_area, row, sensor, stack, svg, text, text_input,
    },
    window,
};

use compass_core::{AppIndex, AppItem};
use compass_platform::{AppLauncher, NullLauncher};

use crate::action_panel::{self, Action, PanelSection, Row, RowKind, Step};
use crate::design::{self, Appearance, GEOMETRY, TINT_ALPHA};
use crate::message::{Direction, Message};
use crate::resident::{EngineLink, UiCommand, UiOutcome};
use crate::scroll::scrollable;

mod a11y;
mod apps;
mod calculator;
mod clipboard;
mod commands;
mod compass_commands;
mod developer;
mod dmenu;
mod emoji;
mod extension;
mod file_actions;
mod files;
mod flags;
mod fonts;
mod global_shortcuts;
mod grants;
mod hud;
mod icons;
mod keyboard;
mod launch;
mod lifecycle;
mod media;
mod onboarding;
mod open_with;
mod panel;
mod preferences;
mod preview;
mod programs;
mod release_check;
mod rhai;
mod root;
mod runtime;
mod scripts;
mod search;
mod settings_a11y;
mod settings_draw;
mod settings_view;
mod shortcuts;
mod snippets;
mod state_line;
mod stores;
mod themes;
mod tray;
mod view;
mod windows;
mod workspaces;

pub use flags::AppFlags;
pub use icons::IconLookup;
pub use keyboard::quick_launch_position;
pub use lifecycle::Dismissal;
pub use panel::{PanelState, actions_for_app};

use lifecycle::exit;
pub(crate) use lifecycle::exit_requested;

/// The search field's widget id.
///
/// It exists so the field can be FOCUSED, and that is not a detail. An Iced
/// `text_input` receives typed characters only while it holds widget focus, and
/// nothing focuses it on its own: a launcher whose field is never focused opens,
/// draws a search box, and silently ignores every keystroke until the user
/// thinks to click it. That was #91, and it survived this long because a person
/// trying the launcher clicks the box without noticing they did, while the VM
/// tier -- which only types -- recorded a frame byte-identical to the one before
/// the keystroke and had no way to say why.
///
/// One constant rather than a literal at each site, because the id in `view` and
/// the id passed to `focus` have to be the same string and nothing would report
/// a typo: focus would simply find no widget, and the launcher would go back to
/// ignoring the keyboard. Kept as a `&'static str` rather than an
/// `iced::advanced::widget::Id` so this crate does not have to turn on Iced's
/// `advanced` feature for one constant; both call sites take `impl Into<Id>`.
const SEARCH_INPUT: &str = "compass-search-input";
const PANEL_INPUT: &str = "compass-action-filter";
/// How long the root's answer stands before assistive technology hears it:
/// longer than Orca's 0.1 s between two announcements, shorter than a pause
/// a person would notice.
const RESULTS_SETTLE: std::time::Duration = std::time::Duration::from_millis(250);

/// Focus the search field.
///
/// Every path that puts the window on screen ends in one of these, because
/// focus does not survive the window being closed and reopened -- `conceal`
/// closes it, so a summon is a brand new window with a brand new, unfocused
/// field.
fn focus_search() -> Task<Message> {
    iced::widget::operation::focus(SEARCH_INPUT)
}

/// The line under a question saying which keys answer it, as the card shows
/// it and the accessibility tree reads it.
fn answer_keys(confirm: &str) -> String {
    format!("Enter: {confirm}    Esc: cancel")
}

/// [`answer_keys`] for an extension's alert, which names its own cancel and
/// may offer a remembered answer.
fn alert_keys(alert: &crate::backend::ExtensionPrompt) -> String {
    match &alert.remember_text {
        Some(remember) => format!(
            "Enter: {}    Ctrl+Enter: {remember}    Esc: {}",
            alert.confirm_text, alert.cancel_text
        ),
        None => format!(
            "Enter: {}    Esc: {}",
            alert.confirm_text, alert.cancel_text
        ),
    }
}

/// Gives Iced's focus to the form field the keyboard is on: the text field
/// itself, or nothing for a checkbox or a dropdown, which the form draws as
/// focused and drives from the key handler.
fn form_focus(page: &crate::preferences_page::PreferencesPage) -> Task<Message> {
    match page.focus {
        Some(index) if page.is_text(index) => {
            iced::widget::operation::focus(crate::preferences_page::field_id(index))
        }
        _ => iced::widget::operation::focus(crate::preferences_page::NO_FIELD),
    }
}

/// A question asked before an action that cannot be undone, as the C++
/// `CallbackAlertWidget`s ask it: Enter confirms, Escape cancels.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Confirm {
    title: String,
    message: String,
    confirm_text: String,
    action: ConfirmAction,
}

/// What a [`Confirm`] runs when it is confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfirmAction {
    /// Remove every clipboard history entry.
    ClipboardRemoveAll,
    /// Change a root item in a way that asks first: reset its ranking or
    /// disable it.
    RootEdit(String, compass_core::root_items::RootEdit),
    /// Uninstall an extension, from Show Installed Extensions.
    UninstallExtension(String),
    /// Remove an extension's token set for a provider, from Manage OAuth
    /// Token Sets.
    RemoveTokenSet(String, Option<String>),
}

/// The root search's clock (`launcher.clock`), when it is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockSettings {
    /// The Qt date-time format it is drawn in.
    pub format: String,
    /// Seconds between redraws.
    pub interval: u64,
}

/// One row of the root list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootRow {
    /// An application, as its index in `AppIndex::items`.
    App(usize),
    /// A builtin command.
    Command(&'static compass_core::commands::BuiltinCommand),
    /// An installed extension's command, as its index in
    /// `AppIndex::extensions`.
    Extension(usize),
    /// The calculator's answer to the query, held in `LauncherApp::calculator`.
    Calculator,
    /// A shortcut (quicklink), as its index in `AppIndex::shortcuts`.
    Shortcut(usize),
    /// A script command, as its index in `AppIndex::scripts`.
    Script(usize),
    /// A Rhai script, as its index in `AppIndex::rhai_scripts`.
    RhaiScript(usize),
    /// A fallback offered for the query, under "Use "…" with...".
    Fallback(Fallback),
    /// A newer Compass release, held in `LauncherApp::update`.
    Update,
}

/// A root item offered as a fallback for the query (`RootFallbackSection`):
/// an entry of `fallbacks` that `isSuitableForFallback`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// A builtin command that can take the query: Search Files.
    Command(&'static compass_core::commands::BuiltinCommand),
    /// An extension's command, opened with the query as its fallback text,
    /// as its index in `AppIndex::extensions`.
    Extension(usize),
    /// A quicklink with exactly one argument, opened with the query as it,
    /// as its index in `AppIndex::shortcuts`.
    Shortcut(usize),
}

/// The provider search view (`ProviderSearchViewHost`): root search over one
/// provider's items, opened by a `compass://launch/<provider>` deeplink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderScope {
    /// The provider's id.
    pub id: String,
    /// Its display name, the view's title.
    pub title: String,
    /// `Search <title>`.
    pub placeholder: String,
}

/// Which view the card shows.
#[derive(Debug, Clone)]
enum Page {
    /// The root list: applications and commands.
    Root,
    /// Clipboard history.
    Clipboard(crate::clipboard_page::ClipboardPage),
    /// The window switcher.
    Windows(crate::windows_page::WindowsPage),
    /// Switch Workspaces.
    Workspaces(crate::workspaces_page::WorkspacesPage),
    /// "Open with…": the applications that open a target.
    OpenWith(crate::open_with_page::OpenWithPage),
    /// The emoji and symbol picker.
    Emoji(crate::emoji_page::EmojiPage),
    /// Search Files.
    Files(crate::files_page::FilesPage),
    /// Manage Shortcuts.
    Shortcuts(crate::shortcuts_page::ShortcutsPage),
    /// Manage Snippets.
    Snippets(crate::snippets_page::SnippetsPage),
    /// A script command's full output.
    ScriptOutput(crate::script_page::ScriptOutputPage),
    /// Run Terminal Program.
    Programs(crate::programs_page::ProgramsPage),
    /// A `compass dmenu` list.
    Dmenu(crate::dmenu_page::DmenuPage),
    /// Set Theme.
    Themes(crate::themes_page::ThemesPage),
    /// The page after Create Extension.
    Created(crate::developer_page::CreatedPage),
    /// Browse Fonts.
    Fonts(crate::fonts_page::FontsPage),
    /// One font's specimen.
    FontPreview(crate::fonts_page::FontPreviewPage),
    /// An extension store's list.
    Store(crate::store_page::StorePage),
    /// One store extension's detail page.
    StoreDetail(Box<crate::store_page::StoreDetailPage>),
    /// Now Playing.
    NowPlaying(crate::media_page::NowPlayingPage),
    /// Script Permissions.
    Grants(crate::grants_page::GrantsPage),
    /// Search Tray.
    Tray(crate::tray_page::TrayPage),
    /// Browse Apps, Set Default Browser or Set Default Terminal.
    Apps(crate::apps_page::AppsPage),
    /// Calculator History.
    Calculator(crate::calculator_page::CalculatorPage),
    /// An extension command's view.
    Extension(Box<crate::extension_page::ExtensionPage>),
    /// The form an extension command's preferences are set in.
    Preferences(Box<crate::preferences_page::PreferencesPage>),
    /// The settings: the C++ settings window's pages.
    Settings(Box<crate::settings_page::SettingsPage>),
    /// The first-run flow.
    Onboarding(Box<crate::onboarding_page::OnboardingPage>),
    /// Configure Fallback Commands.
    Fallbacks(crate::fallbacks_page::FallbacksPage),
    /// Show Installed Extensions.
    Extensions(crate::compass_pages::ExtensionsPage),
    /// Search Builtin Icons.
    Icons(crate::compass_pages::IconsPage),
    /// Inspect Local Storage.
    Storage(crate::compass_pages::StoragePage),
    /// Manage OAuth Token Sets.
    Tokens(crate::compass_pages::TokensPage),
    /// A store's intro, before its list.
    StoreIntro(crate::compass_pages::StoreIntroPage),
}

/// The launcher application state.
pub struct LauncherApp {
    /// The application index.
    app_index: AppIndex,
    /// The engine's catalog generation this index was last brought up to;
    /// see [`crate::backend::ApplicationBackend::catalog_generation`].
    catalog_generation: u64,
    /// Current query text.
    query: String,
    /// Ranked results: applications as indices into `app_index.items()`,
    /// and builtin commands.
    ///
    /// Indices rather than cloned items: the ranking already works in index
    /// space, an `AppItem` carries its whole parsed desktop entry, and a
    /// launcher re-ranks on every keystroke.
    results: Vec<RootRow>,
    /// The calculator's answer to the query, shown first when there is one.
    calculator: Option<compass_core::calculator::Answer>,
    /// When this window last asked the engine for exchange rates, so a
    /// currency expression typed while they are missing or stale asks once,
    /// not on every keystroke.
    rates_asked: Option<std::time::Instant>,
    /// A power command waiting on the person's yes.
    power_confirm: Option<&'static compass_core::power_commands::PowerCommand>,
    /// A question waiting on Enter or Escape before an action runs.
    confirm: Option<Confirm>,
    /// Clipboard History while its keyword form is open.
    parked_clipboard: Option<crate::clipboard_page::ClipboardPage>,
    /// The view "Open with…" was opened over, to go back to.
    open_with_return: Option<Box<Page>>,
    /// The MIME type of the file the open panel is over, for Copy mime type.
    file_mime: Option<String>,
    /// Which view is showing. See [`Page`].
    page: Page,
    /// Clipboard history. See [`AppFlags::clipboard`].
    clipboard: Option<Arc<dyn crate::backend::ClipboardBackend>>,
    /// Window switching. See [`AppFlags::windows`].
    windows: Option<Arc<dyn crate::backend::WindowBackend>>,
    /// Which row is selected, as a position in `results`.
    selected: usize,
    /// The last launch failure, shown until the query changes.
    error: Option<String>,
    /// A failure that came while the launcher was hidden, for the error line
    /// when it is next shown: a summon searches again, which clears `error`.
    error_for_summon: Option<String>,
    /// How to launch. See [`AppFlags::launcher`].
    launcher: Arc<dyn AppLauncher>,
    backend: Option<Arc<dyn crate::backend::ApplicationBackend>>,
    search_generation: u64,
    search_task: Option<iced::task::Handle>,
    /// The engine answered the root search a moment ago, and assistive
    /// technology is not told until it has stood for [`RESULTS_SETTLE`]:
    /// answers that arrive close together would each change the announced
    /// count, and Orca drops an announcement within 0.1 s of the last, so the
    /// final count was the one it lost (the Orca gate, #305).
    results_settling: bool,
    /// The engine driving this window. See [`AppFlags::link`].
    link: Option<EngineLink>,
    exit_on_engine_disconnect: bool,
    /// The open window, if one is.
    ///
    /// `None` is the hidden state: on Wayland a hidden window is a closed one.
    window: Option<window::Id>,
    pending_window: Option<window::Id>,
    pending_hide: bool,
    closing: bool,
    reopen_after_close: bool,
    /// Settings to open a window with, kept for every summon after the first.
    window_config: window::Settings,
    /// Theme, appearance, typography and what views remember: the
    /// presentation state ([`crate::view_state`]).
    view: crate::view_state::ViewState,
    /// The `fallbacks` entries a non-empty query offers, as ids.
    fallbacks: Vec<String>,
    /// The provider search view, when root search is showing one provider.
    provider_scope: Option<ProviderScope>,
    /// Each Rhai script's manifest icon, resolved, by script id.
    rhai_icons: std::collections::HashMap<String, crate::extension_page::RowIcon>,
    /// Each script command's icon, as the engine resolved it, by script id.
    script_icons: std::collections::HashMap<String, compass_core::image_url::ImageUrl>,
    /// Root rows' `ImageURL`s resolved to what is drawn, by URL, warmed
    /// from `update`; `None` is a miss (or a remote image not yet here).
    url_glyphs: std::collections::HashMap<String, Option<crate::icons::Glyph>>,
    /// Remote icons fetched into the cache, by URL.
    remote_files: std::collections::HashMap<String, std::path::PathBuf>,
    /// Remote icons asked for, so each is fetched once.
    remote_requested: std::collections::BTreeSet<String>,
    /// Remote icons to fetch after this update.
    remote_pending: Vec<String>,
    /// Whether the compositor's shortcuts were last asked to go to the
    /// launcher, because a shortcut recorder records
    /// ([`crate::shortcut_inhibit`]).
    shortcuts_inhibited: bool,
    /// See [`AppFlags::remote_icons`].
    remote_icons: bool,
    /// See [`AppFlags::favicon_service`].
    favicon_service: compass_core::favicon::Service,
    /// The first-run flow, when it is due but the launcher started hidden:
    /// it opens with the first summon rather than at login.
    pending_onboarding: Option<std::path::PathBuf>,
    /// Where `compass.json` arrives after a change. See [`AppFlags::config_link`].
    config_link: Option<crate::config_link::ConfigLink>,
    /// Whether the selection wraps at the ends. See
    /// [`compass_core::list_navigation`].
    wrap_navigation: bool,
    /// Whether Ctrl+1..9 launches the Nth result. See [`AppFlags::quick_launch`].
    quick_launch: bool,
    /// See [`AppFlags::close_on_focus_loss`].
    close_on_focus_loss: bool,
    /// See [`AppFlags::pop_to_root_on_close`].
    pop_to_root_on_close: bool,
    /// Whether the window has had the focus since it was last shown: losing
    /// it hides the window only after it had it (`setWindowActivated`).
    window_focused: bool,
    /// See [`AppFlags::launcher_hotkey`].
    launcher_hotkey: String,
    /// See [`AppFlags::hotkey_compositor`].
    hotkey_compositor: Option<compass_core::hotkey_guide::Compositor>,
    /// See [`AppFlags::flatpak`].
    flatpak: bool,
    /// Whether Compass starts at login, as the engine last answered; the
    /// first-run flow's switch and Settings' show it.
    login_start: Option<compass_core::login_start::Status>,
    /// Whether a change to login start is on its way to the engine: the
    /// switch waits for its answer, and a failure is said where it was
    /// flipped.
    login_start_busy: bool,
    /// Whether the engine was last told the recorder is capturing.
    capture_reported: bool,
    /// Whether an extension's file chooser is open, which takes the focus
    /// without the user leaving the launcher.
    choosing_files: bool,
    /// See [`AppFlags::power_asks`].
    power_asks: std::collections::BTreeMap<String, bool>,
    /// See [`AppFlags::browse_apps`].
    browse_apps: compass_core::browse_apps::Options,
    /// See [`AppFlags::config_path`].
    config_path: Option<std::path::PathBuf>,
    /// See [`AppFlags::glyph_path`].
    glyph_path: Option<std::path::PathBuf>,
    /// See [`AppFlags::log_path`].
    log_path: Option<std::path::PathBuf>,
    /// See [`AppFlags::default_config_dir`].
    default_config_dir: Option<std::path::PathBuf>,
    /// See [`AppFlags::emoji_skin_tone`].
    emoji_skin_tone: Option<String>,
    /// See [`AppFlags::emoji_default_action`].
    emoji_default_action: String,
    /// The emoji picker while its keyword form is open.
    parked_emoji: Option<crate::emoji_page::EmojiPage>,
    /// The settings while an extension command's preferences form is open
    /// over them.
    parked_settings: Option<Box<crate::settings_page::SettingsPage>>,
    /// See [`AppFlags::search_history_path`].
    search_history_path: Option<std::path::PathBuf>,
    /// The root search's history, newest first.
    search_history: compass_core::root_view::SearchHistory,
    /// Where the up arrow has reached in the history; `None` until it is
    /// pressed, and again once something is typed.
    history_offset: Option<usize>,
    /// What was typed when the up arrow first reached into history, for the
    /// down arrow to put back.
    history_draft: Option<String>,
    /// See [`AppFlags::clock`].
    clock: Option<ClockSettings>,
    /// The time the root search's status bar shows, once the clock ticked.
    clock_text: Option<String>,
    /// When, in seconds since the epoch, the clock is next redrawn.
    clock_next_at: i64,
    /// The root settings this window applies locally: the startup
    /// configuration, and what its own panel has changed since.
    root_config: compass_core::root_items::RootConfig,
    /// How many of `results`' first rows are the favourites (empty query).
    favorites_len: usize,
    /// Sizes and spacing, from the resolved appearance preset (#84).
    ///
    /// Held rather than read from [`design::GEOMETRY`] at each draw: a preset
    /// varies it, and `view` must not have to know which one is in force.
    geometry: design::Geometry,
    /// Whether a rule separates the field from the results. See
    /// [`crate::preset::Preset::field_rule`].
    field_rule: bool,

    /// Whether the card background is translucent (#86).
    ///
    /// Translucency, not blur — see [`crate::preset::Preset::tint`]. The window
    /// surface is already transparent (`AppFlags::default`), so this only
    /// changes the card's own background alpha.
    tint: bool,
    /// Whether rows show their subtitle. See [`crate::preset::Preset::subtitles`].
    subtitles: bool,
    /// When the first frame was drawn, for the cold-start figure (#13).
    ///
    /// `None` until it happens, which is also what keeps the per-frame
    /// subscription from running for the life of the process.
    first_frame_at: Option<std::time::Instant>,
    /// When a `Show` that has to open a window arrived, until that window's
    /// first frame is drawn. The summon figure (§8.5); see `FrameDrawn`.
    summoned_at: Option<std::time::Instant>,
    /// The last summon figure, kept so it can be asserted on.
    last_summon_draw: Option<std::time::Duration>,
    /// When the process started. See [`AppFlags::started_at`].
    started_at: Option<std::time::Instant>,
    /// Whether result rows show the application's icon. See [`AppFlags::icons`].
    icons: bool,
    /// How an `Icon=` name becomes a file. See [`AppFlags::icon_lookup`].
    icon_lookup: IconLookup,
    /// Icons resolved for rows that have been on screen (#85).
    ///
    /// Warmed from `update`, never from `view`: resolving a name walks the
    /// theme directories, which is disk work that does not belong in a draw.
    icon_cache: crate::icons::IconCache,
    /// Where the builtin icon set is installed, read once at startup.
    builtin_icons: Option<std::path::PathBuf>,
    /// The icon theme's symbolic SVG for each builtin that has one
    /// ([`compass_core::builtin_icon::symbolic_name`]), resolved once at
    /// startup so `view` never walks the theme.
    symbolic_icons: std::collections::HashMap<&'static str, std::path::PathBuf>,
    /// File-type icons for file rows, by path, warmed as `icon_cache` is.
    file_glyphs: crate::icons::FileGlyphCache,
    /// Which navigation chords are in force. See [`compass_core::keybinding`].
    ///
    /// Held rather than read per keystroke because it comes from the user's
    /// configuration, which is read once.
    keybinding: compass_core::keybinding::Scheme,
    /// The action panel, when it is open.
    ///
    /// `None` is closed. Holding the whole state rather than a bare flag is
    /// what lets the panel keep its own filter and selection while the list
    /// underneath keeps its own -- they are two lists on screen at once, and
    /// sharing either would make one of them jump when the other moved.
    panel: Option<PanelState>,
    /// Whether the engine is waiting for an outcome right now.
    ///
    /// # Every report must answer a command, or the stream goes out of step
    ///
    /// The link is strictly one command, one outcome: the bridge sends a
    /// command and then blocks reading exactly one reply. So an outcome sent
    /// when nothing was asked does not go nowhere -- it sits in the channel and
    /// becomes the answer to the *next* command, and every answer after that is
    /// one behind, permanently.
    ///
    /// Two things used to do exactly that. Opening the window at boot reported
    /// `Shown`, and a user pressing Escape reported `Hidden`; neither answers
    /// anything. The engine would then report "shown" for a toggle that hid the
    /// window -- the precise lie this whole design exists to prevent, arriving
    /// through the mechanism built to prevent it.
    awaiting: bool,
    /// Manage Shortcuts as it was when a form was opened over it, so going
    /// back returns to the same filter and selection.
    parked_shortcuts: Option<crate::shortcuts_page::ShortcutsPage>,
    /// Manage Snippets as it was when a form was opened over it.
    parked_snippets: Option<crate::snippets_page::SnippetsPage>,
    /// Browse Fonts as it was when a specimen was opened over it.
    parked_fonts: Option<crate::fonts_page::FontsPage>,
    /// A store's list as it was when a detail page was opened over it.
    parked_store: Option<crate::store_page::StorePage>,
    /// The card size a dmenu list resized the open window to, while it is.
    resized_to: Option<(u32, u32)>,
    /// The blur region last asked for behind the open window's card, so a
    /// layout that changes nothing asks nothing. `None` is none asked for.
    material_asked: Option<compass_platform::MaterialRegion>,
    /// A compact or inline script run the root list is waiting on.
    following_script: Option<scripts::FollowedScript>,
    /// Subtitles extensions set for their commands (`updateCommandMetadata`),
    /// by command id, shown in place of the extension's title.
    extension_subtitles: std::collections::HashMap<String, String>,
    /// A newer Compass release, as the engine last said.
    update: Option<crate::backend::UpdateOffer>,
    /// Whether the application under the root panel runs, by its key.
    app_runtime: Option<(String, crate::backend::AppRuntimeInfo)>,
    /// The HUD shown after an action hides the launcher (`crate::hud`).
    hud: crate::hud::HudState,
    /// The room the launcher window last had on its output, logical pixels,
    /// once the compositor or the toolkit has said; its size is fitted to it
    /// (`crate::surface::fit`).
    room: Option<iced::Size>,
    /// A layer surface opened at the output's full size to learn the room,
    /// not yet given its own size. Nothing is drawn in it meanwhile.
    measuring: Option<window::Id>,
    /// The launcher window's size, as last laid out.
    window_size: Option<iced::Size>,
    /// The toplevel whose monitor was asked for, so it is asked once.
    monitor_asked: Option<window::Id>,
    /// Whether the open window has reported gaining the focus at least once
    /// (see `behind_another_window`).
    focus_seen: bool,
    /// The transparent layer surface under the launcher that takes a click
    /// outside it, with `close_on_focus_loss` on (`crate::surface::open_backdrop`).
    backdrop: Option<window::Id>,
    /// The backdrop's size, which is the room the launcher is centred in.
    backdrop_size: Option<iced::Size>,
    /// The accessibility tree's view of this window ([`crate::accessibility`]).
    a11y: crate::accessibility::TreeSync,
}

/// Where the selection lands after moving one row in `direction`.
///
/// `wrap` is `launcher.wrap_navigation`, which is **false** by default -- the
/// C++ `Config::wrapNavigation` is, and the selection clamps at the first and
/// last row. An earlier version of this function wrapped unconditionally, with
/// a comment saying that is "what every launcher does"; the launcher being
/// ported does not.
///
/// Returns 0 for an empty list so callers never index into nothing.
#[must_use]
pub fn next_selection(len: usize, current: usize, direction: Direction, wrap: bool) -> usize {
    use compass_core::list_navigation::{Step, next};

    if len == 0 {
        return 0;
    }
    // Clamp the incoming index first: the list may have shrunk since it was
    // set, and `next` would otherwise step from somewhere that is not there.
    let current = current.min(len - 1);
    match direction {
        Direction::Down => next(current, Step::Forward, len, wrap),
        Direction::Up => next(current, Step::Backward, len, wrap),
    }
}

/// The direction an iced key press moves the selection, under `scheme`.
///
/// Returns `None` for `Left` and `Right` as well as for a key that is not a
/// chord: the results list is one column, so `Ctrl+H` and `Ctrl+L` have
/// nowhere to go here. They are recognised by `compass-core` and dropped here
/// deliberately, rather than being quietly bound to something they do not
/// mean.
fn chord_direction(
    scheme: compass_core::keybinding::Scheme,
    key: iced::keyboard::Key<&str>,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Direction> {
    use compass_core::keybinding::{Chord, Modifiers as CoreModifiers, navigation};

    let iced::keyboard::Key::Character(text) = key else {
        return None;
    };
    let mut chars = text.chars();
    let (character, None) = (chars.next()?, chars.next()) else {
        return None;
    };

    let chord = Chord::new(
        character,
        CoreModifiers {
            // iced reports the physical Control key as `control` on every
            // platform, which is what these chords want.
            ctrl: modifiers.control(),
            alt: modifiers.alt(),
            shift: modifiers.shift(),
            logo: modifiers.logo(),
        },
    );

    match navigation(scheme, chord)? {
        compass_core::keybinding::Direction::Up => Some(Direction::Up),
        compass_core::keybinding::Direction::Down => Some(Direction::Down),
        compass_core::keybinding::Direction::Left | compass_core::keybinding::Direction::Right => {
            None
        }
    }
}

impl LauncherApp {
    /// Create a new launcher application, indexing the environment.
    pub fn new(flags: AppFlags) -> (Self, Task<Message>) {
        let mut app = Self::with_index(AppIndex::from_environment());
        app.apply(flags);
        (app, Task::none())
    }

    /// Builds the state and opens the first window, for [`crate::run_resident`].
    ///
    /// Distinct from [`LauncherApp::new`] because `iced::daemon` starts with no
    /// windows at all: without this, `compass ui` with no engine attached would
    /// be an invisible process with no way to summon it.
    pub fn boot(flags: AppFlags) -> (Self, Task<Message>) {
        let onboarding = flags.onboarding.clone();
        let hidden = flags.start_hidden && flags.link.is_some();
        let (mut app, task) = Self::new(flags);
        if hidden {
            // `compass start --hidden` runs at login: the flow waits for the
            // first time someone opens the launcher.
            app.pending_onboarding = onboarding;
            return (app, task);
        }
        if let Some(path) = onboarding {
            app.open_onboarding(path);
        }
        let opened = app.open_window();
        (app, Task::batch([task, opened]))
    }

    /// Create one over a supplied index.
    ///
    /// Exists so tests can drive the state machine over a known corpus instead
    /// of whatever applications the machine running them happens to have.
    #[must_use]
    pub fn with_index(app_index: AppIndex) -> Self {
        Self {
            app_index,
            catalog_generation: 0,
            query: String::new(),
            results: Vec::new(),
            calculator: None,
            rates_asked: None,
            power_confirm: None,
            confirm: None,
            parked_clipboard: None,
            open_with_return: None,
            file_mime: None,
            page: Page::Root,
            clipboard: None,
            windows: None,
            selected: 0,
            panel: None,
            error: None,
            error_for_summon: None,
            launcher: Arc::new(NullLauncher),
            backend: None,
            search_generation: 0,
            search_task: None,
            results_settling: false,
            link: None,
            exit_on_engine_disconnect: false,
            window: None,
            pending_window: None,
            pending_hide: false,
            closing: false,
            reopen_after_close: false,
            window_config: AppFlags::default().window_config,
            view: crate::view_state::ViewState::default(),
            fallbacks: Vec::new(),
            provider_scope: None,
            rhai_icons: std::collections::HashMap::new(),
            script_icons: std::collections::HashMap::new(),
            url_glyphs: std::collections::HashMap::new(),
            remote_files: std::collections::HashMap::new(),
            remote_requested: std::collections::BTreeSet::new(),
            remote_pending: Vec::new(),
            shortcuts_inhibited: false,
            remote_icons: false,
            favicon_service: compass_core::favicon::Service::default(),
            pending_onboarding: None,
            config_link: None,
            keybinding: compass_core::keybinding::Scheme::default(),
            wrap_navigation: compass_core::config::DEFAULT_WRAP_NAVIGATION,
            quick_launch: compass_core::config::DEFAULT_QUICK_LAUNCH,
            close_on_focus_loss: compass_core::config::DEFAULT_CLOSE_ON_FOCUS_LOSS,
            pop_to_root_on_close: compass_core::config::DEFAULT_POP_TO_ROOT_ON_CLOSE,
            window_focused: false,
            launcher_hotkey: compass_core::config::DEFAULT_HOTKEY.to_owned(),
            hotkey_compositor: None,
            flatpak: false,
            login_start: None,
            login_start_busy: false,
            capture_reported: false,
            choosing_files: false,
            power_asks: std::collections::BTreeMap::new(),
            config_path: None,
            browse_apps: compass_core::browse_apps::Options::default(),
            glyph_path: None,
            log_path: None,
            default_config_dir: None,
            emoji_default_action: compass_core::emoji_grid::DEFAULT_ACTION_PASTE.to_owned(),
            emoji_skin_tone: None,
            search_history_path: None,
            search_history: compass_core::root_view::SearchHistory::default(),
            history_offset: None,
            history_draft: None,
            clock: None,
            clock_text: None,
            clock_next_at: 0,
            root_config: compass_core::root_items::RootConfig::default(),
            favorites_len: 0,
            parked_emoji: None,
            parked_settings: None,
            icons: compass_core::config::DEFAULT_ICONS,
            icon_lookup: IconLookup::default(),
            icon_cache: crate::icons::IconCache::new(),
            builtin_icons: None,
            symbolic_icons: std::collections::HashMap::new(),
            file_glyphs: crate::icons::FileGlyphCache::default(),
            geometry: design::GEOMETRY,
            field_rule: false,
            tint: false,
            subtitles: true,
            first_frame_at: None,
            summoned_at: None,
            last_summon_draw: None,
            started_at: None,
            awaiting: false,
            parked_shortcuts: None,
            parked_snippets: None,
            parked_fonts: None,
            parked_store: None,
            resized_to: None,
            material_asked: None,
            following_script: None,
            extension_subtitles: std::collections::HashMap::new(),
            update: None,
            app_runtime: None,
            hud: crate::hud::HudState::new(
                crate::surface::presentation() == crate::surface::Presentation::LayerShell,
            ),
            room: None,
            measuring: None,
            window_size: None,
            monitor_asked: None,
            focus_seen: false,
            backdrop: None,
            backdrop_size: None,
            a11y: crate::accessibility::TreeSync::default(),
        }
    }

    /// Attaches an engine link. For tests that drive the resident path.
    #[must_use]
    pub fn with_link(mut self, link: EngineLink) -> Self {
        self.link = Some(link);
        self
    }

    /// Use a shared application ranking and history service.
    #[must_use]
    pub fn with_backend(mut self, backend: Arc<dyn crate::backend::ApplicationBackend>) -> Self {
        self.backend = Some(backend);
        self
    }

    /// Replace the launcher. For tests that assert what the UI asked for.
    pub fn with_launcher(mut self, launcher: Arc<dyn AppLauncher>) -> Self {
        self.launcher = launcher;
        self
    }

    /// The item the selection currently points at, if any.
    #[must_use]
    pub fn selected_item(&self) -> Option<&AppItem> {
        match *self.results.get(self.selected)? {
            RootRow::App(index) => self.app_index.items().get(index),
            RootRow::Command(_)
            | RootRow::Extension(_)
            | RootRow::Shortcut(_)
            | RootRow::Script(_)
            | RootRow::RhaiScript(_)
            | RootRow::Fallback(_)
            | RootRow::Calculator
            | RootRow::Update => None,
        }
    }

    /// The row the selection points at, application or command.
    #[must_use]
    pub fn selected_row(&self) -> Option<RootRow> {
        self.results.get(self.selected).copied()
    }

    /// Whether the window switcher is the view showing.
    #[must_use]
    pub fn showing_windows(&self) -> bool {
        matches!(self.page, Page::Windows(_))
    }

    /// Whether clipboard history is the view showing.
    #[must_use]
    pub fn showing_clipboard(&self) -> bool {
        matches!(self.page, Page::Clipboard(_))
    }

    /// Update the application state.
    ///
    /// Every message leaves a line in the log saying what the launcher now
    /// shows -- see [`LauncherApp::state_line`] for why. `debug`, not `info`:
    /// a line per keystroke is what makes a failure readable afterwards and is
    /// not what someone running the launcher wants in their terminal.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        if matches!(message, Message::ExtensionFilesChosen { .. }) {
            self.choosing_files = false;
        }
        let mut task = self.update_inner(message);
        if let Some(rates) = self.rates_on_demand() {
            task = Task::batch([task, rates]);
        }
        // A form that has just opened puts the keyboard on its first field,
        // whichever of the many paths opened it.
        if let Page::Preferences(page) = &mut self.page
            && page.focus_pending
        {
            page.focus_first();
            task = Task::batch([task, form_focus(page)]);
        }
        let recording = self.recording_shortcut();
        if recording != self.shortcuts_inhibited {
            self.shortcuts_inhibited = recording;
            crate::shortcut_inhibit::set_recording(recording);
        } else {
            // Every update, not only while recording: the inhibitor's keyboard
            // is sent each key the launcher is, and they would pile up unread.
            crate::shortcut_inhibit::follow_focus();
        }
        let task = match self.report_shortcut_capture() {
            Some(report) => Task::batch([task, report]),
            None => task,
        };
        self.follow_accessibility();
        tracing::debug!(target: "compass_ui::state", "{}", self.state_line());
        if self.remote_pending.is_empty() {
            return task;
        }
        let wanted = std::mem::take(&mut self.remote_pending);
        Task::batch([task, crate::remote_image::fetch_tasks(wanted)])
    }

    /// Whether the compositor's shortcuts were last asked to go to the
    /// launcher.
    #[must_use]
    pub fn shortcuts_inhibited(&self) -> bool {
        self.shortcuts_inhibited
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Initialize => Task::none(),

            Message::FrameDrawn
            | Message::CloseWindow(_)
            | Message::EngineDisconnected
            | Message::Opened(_)
            | Message::Closed(_)
            | Message::BackdropPressed => self.lifecycle_message(message),
            Message::AppearanceChanged(_)
            | Message::TypographyChanged(_)
            | Message::ThemePreview(_)
            | Message::ThemeCommit
            | Message::ThemeCancel => self.appearance_message(message),
            Message::ConfigReloaded(config) => self.apply_reloaded_config(&config),
            Message::QueryChanged(_)
            | Message::ClockTick
            | Message::SearchCompleted { .. }
            | Message::ResultsSettled(_)
            | Message::ResultSelected(_)
            | Message::ResultClicked(_)
            | Message::MoveSelection(_)
            | Message::LaunchSelected
            | Message::Launched(_)
            | Message::CatalogGeneration(_) => self.search_message(message),
            Message::RootItemEdited(result) => self.root_item_edited(result),
            Message::Dismiss => self.conceal(),
            Message::ShortcutActivated(_) => Task::none(),
            Message::FocusChanged(_) => Task::none(),
            Message::WindowClosed => self.conceal(),
            Message::Quit => exit(),
            // Taken by `iced_layershell` before `update`; see `crate::surface`.
            Message::Layer(_) => Task::none(),
            Message::HudTick(now) => self.hud_tick(now),
            Message::CardMeasured(card) => self.card_measured(card),
            Message::WindowSized { id, size, opened } => self.window_sized(id, size, opened),
            Message::MonitorSized(id, size) => self.monitor_sized(id, size),
            Message::FallbacksQueryChanged(_) | Message::FallbackSelected(_) => {
                self.fallbacks_message(message)
            }
            Message::ExtensionsQueryChanged(_)
            | Message::IconsQueryChanged(_)
            | Message::CompassRowSelected(_)
            | Message::ExtensionUninstalled { .. }
            | Message::StorageQueryChanged(_)
            | Message::TokensQueryChanged(_)
            | Message::StorageNamespacesLoaded(_)
            | Message::StorageItemsLoaded { .. }
            | Message::TokenSetsLoaded(_)
            | Message::TokenSetRemoved(_) => self.compass_view_message(message),
            Message::OnboardingContinue
            | Message::OnboardingBack
            | Message::OnboardingJump(_)
            | Message::OnboardingTheme(_)
            | Message::OnboardingOpen(_)
            | Message::OnboardingLinkOpened(_)
            | Message::OnboardingInstall(_)
            | Message::OnboardingInstalled(..)
            | Message::OnboardingRequestShortcut => self.onboarding_message(message),
            Message::LauncherShortcutAnswered(_)
            | Message::LoginStartToggled(_)
            | Message::LoginStartAnswered(_) => self.first_run_message(message),
            Message::ActionDone(Some(hud), Ok(())) => self.show_hud(hud),
            Message::ActionDone(None, Ok(())) => self.conceal(),
            Message::ActionDone(_, Err(reason)) => {
                self.say(reason);
                Task::none()
            }
            Message::TogglePanel
            | Message::PanelFilterChanged(_)
            | Message::PanelMove(_)
            | Message::PanelClicked(_)
            | Message::PanelActivate => self.panel_message(message),
            Message::Command(command) => self.obey(command),
            Message::PollShortcuts => Task::none(),
            Message::EventOccurred(_) => Task::none(),
            Message::ClipboardKindChanged(_)
            | Message::ClipboardDetailLoaded { .. }
            | Message::ClipboardDetailContent { .. }
            | Message::ClipboardKeywordsLoaded(_)
            | Message::ClipboardMonitoringLoaded(_)
            | Message::ClipboardQueryChanged(_)
            | Message::ClipboardLoaded { .. }
            | Message::ClipboardSelected(_)
            | Message::ClipboardPasted(_)
            | Message::ClipboardEntryChanged(_)
            | Message::ClipboardContentLoaded(_) => self.clipboard_message(message),
            Message::PreferenceEdited(..)
            | Message::PreferenceTextEdited(..)
            | Message::PreferencesSubmit
            | Message::PreferencesSaved(_) => self.preferences_message(message),
            Message::ExtensionCommandStarted { .. }
            | Message::ExtensionViewLoaded { .. }
            | Message::ExtensionQueryChanged(_)
            | Message::ExtensionItemSelected(_)
            | Message::ExtensionFieldEdited(..)
            | Message::ExtensionTextAreaEdited(..)
            | Message::ExtensionImageFetched { .. }
            | Message::ExtensionDateEdited(..)
            | Message::ExtensionChooseFiles { .. }
            | Message::ExtensionFilesChosen { .. }
            | Message::ExtensionLinkClicked(_)
            | Message::ExtensionEventSent(_) => self.extension_message(message),
            Message::EmojiPasted { text, result } => self.emoji_pasted(text, result),
            Message::TextCopied { text, result } => Self::text_copied(text, result),
            Message::ShortcutsLoaded(_)
            | Message::ShortcutSaved(_)
            | Message::ShortcutRemoved(_)
            | Message::ShortcutOpened(_)
            | Message::ShortcutExpanded(_)
            | Message::ShortcutsQueryChanged(_)
            | Message::ShortcutSelected(_)
            | Message::ShortcutDetailLoaded(_) => self.shortcut_message(message),
            Message::SnippetsLoaded(_)
            | Message::SnippetSaved(_)
            | Message::SnippetExpanded(_)
            | Message::SnippetPasted(_)
            | Message::SnippetsQueryChanged(_)
            | Message::SnippetSelected(_)
            | Message::SnippetDetailLoaded(_) => self.snippet_message(message),
            Message::ScriptsLoaded(_)
            | Message::ScriptIconsLoaded(_)
            | Message::ScriptStarted { .. }
            | Message::ScriptPolled { .. } => self.script_message(message),
            Message::RhaiScriptsLoaded(_) => self.rhai_message(message),
            Message::UpdateStatusLoaded(_) | Message::UpdateSkipped(..) => {
                self.release_check_message(message)
            }
            Message::LaunchFetched(_)
            | Message::ExtensionSubtitlesLoaded(_)
            | Message::PreferencesOpened { .. } => self.launch_message(message),
            Message::AppsQueryChanged(_)
            | Message::AppsSelected(_)
            | Message::DefaultAppsLoaded(_)
            | Message::DefaultAppSet(_)
            | Message::BrowseAppRuntime { .. } => self.apps_message(message),
            Message::AppRuntimeLoaded { .. } | Message::AppQuit(_) => self.runtime_message(message),
            Message::WindowFocusChanged(focused) => self.window_focus_changed(focused),
            Message::Accessibility(request) => self.accessibility_request(request),
            Message::ShortcutCaptureSet(result) => {
                if let Err(error) = result {
                    tracing::warn!(%error, "the engine did not suspend the global shortcuts");
                }
                Task::none()
            }
            Message::ShortcutProbed(trigger, result) => self.shortcut_probed(&trigger, result),
            Message::WindowCapabilities(_)
            | Message::WorkspacesQueryChanged(_)
            | Message::WorkspacesLoaded(_)
            | Message::WorkspaceSelected(_)
            | Message::WorkspaceFocused(_)
            | Message::WindowToggled(_) => self.workspaces_message(message),
            Message::FileActionsLoaded { .. } | Message::FileActionDone(_) => {
                self.file_actions_message(message)
            }
            Message::OpenWithTarget(_)
            | Message::OpenersLoaded(_)
            | Message::OpenWithQueryChanged(_)
            | Message::OpenWithSelected(_)
            | Message::OpenedWith(_) => self.open_with_message(message),
            Message::CalculatorQueryChanged(_)
            | Message::CalculatorLoaded { .. }
            | Message::CalculatorSelected(_)
            | Message::CalculatorEdited(_)
            | Message::ExchangeRatesLoaded(_)
            | Message::ExchangeRatesRefreshed(_) => self.calculator_message(message),
            Message::GrantsLoaded(_)
            | Message::GrantsQueryChanged(_)
            | Message::GrantSelected(_)
            | Message::GrantRevoked(_) => self.grants_message(message),
            Message::TrayItemsLoaded(_)
            | Message::TrayMenuLoaded { .. }
            | Message::TrayQueryChanged(_)
            | Message::TraySelected(_)
            | Message::TrayActed(_) => self.tray_message(message),
            Message::NowPlayingLoaded(_)
            | Message::NowPlayingQueryChanged(_)
            | Message::NowPlayingSelected(_)
            | Message::NowPlayingActed(_) => self.media_message(message),
            Message::ProgramsLoaded(_)
            | Message::ProgramsQueryChanged(_)
            | Message::ProgramSelected(_)
            | Message::ProgramRan(_) => self.program_message(message),
            Message::DmenuLoaded { .. }
            | Message::DmenuQueryChanged(_)
            | Message::DmenuSelected(_)
            | Message::DmenuChosen(_) => self.dmenu_message(message),
            Message::ThemesQueryChanged(_) | Message::ThemeSelected(_) | Message::ThemeSaved(_) => {
                self.theme_message(message)
            }
            Message::ExtensionCreated { .. } | Message::CreatedFolderOpened(_) => {
                self.developer_message(message)
            }
            Message::FontsLoaded(_)
            | Message::FontsQueryChanged(_)
            | Message::FontsCategoryChanged(_)
            | Message::FontSet(_)
            | Message::FontSelected(_)
            | Message::FontSpecimenLoaded { .. } => self.font_message(message),
            Message::StoreLoaded { .. }
            | Message::StoreQueryChanged(_)
            | Message::StoreSearchDue(_)
            | Message::StoreSelected(_)
            | Message::StoreDetailLoaded(_)
            | Message::StoreInstalled(_)
            | Message::StoreUninstalled { .. }
            | Message::StoreUrlOpened(_)
            | Message::StoreConfirmAnswered(_) => self.store_message(message),
            Message::Back => {
                // Escape on a dmenu list dismisses it and the launcher, as the
                // C++'s instant dismiss does.
                if matches!(self.page, Page::Dmenu(_)) {
                    return self.conceal();
                }
                if let Some(task) = self.back_from_emoji_keywords() {
                    return task;
                }
                if let Some(task) = self.back_from_clipboard_keywords() {
                    return task;
                }
                if let Some(task) = self.back_from_open_with() {
                    return task;
                }
                if let Some(task) = self.back_from_alias_form() {
                    return task;
                }
                if let Some(task) = self.back_to_settings() {
                    return task;
                }
                if let Some(task) = self.back_from_shortcut_form() {
                    return task;
                }
                if let Some(task) = self.back_from_snippet_form() {
                    return task;
                }
                let closing = self.close_extension_view();
                self.page = Page::Root;
                Task::batch([closing, focus_search()])
            }
            Message::BuiltinCommandDone(result) => {
                let Err(reason) = result else {
                    return Task::none();
                };
                self.command_failed(&reason, "")
            }
            Message::FilesQueryChanged(_)
            | Message::FilesDebounced(_)
            | Message::FilesCategoryChanged(_)
            | Message::FilesLoaded { .. }
            | Message::FilesSelected(_)
            | Message::FileOpened(_) => self.files_message(message),
            Message::EmojiQueryChanged(_) | Message::EmojiSelected(_) => {
                self.emoji_message(message)
            }
            Message::WindowsQueryChanged(_)
            | Message::WindowsLoaded(_)
            | Message::WindowSelected(_)
            | Message::WindowActivated(_)
            | Message::ShellWindowClosed(_) => self.windows_message(message),
            Message::Settings(message) => self.settings_message(message),
            Message::Keyboard(_) => self.keyboard_message(message),
        }
    }
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;
