//! The launcher window: search, move, launch, dismiss.
//!
//! The state machine is deliberately separable from Iced. `update` takes a
//! [`Message`] and mutates fields; the only Iced-shaped things it returns are
//! [`Task`]s, which can be constructed without a running runtime. That is what
//! makes this crate testable in a container with no display server — the
//! selection and launch-target logic, which is where the bugs live, is exercised
//! by ordinary unit tests, and only the drawing needs a compositor.

use iced::{
    Alignment, Border, Color, Element, Length, Padding, Task, Theme,
    widget::{
        Space, column, container, image, mouse_area, row, sensor, stack, svg, text, text_input,
    },
    window,
};

use std::sync::Arc;

use compass_core::{AppIndex, AppItem};
use compass_platform::{AppLauncher, NullLauncher};

use crate::action_panel::{self, Action, PanelSection, Row, RowKind, Step};
use crate::design::{self, Appearance, GEOMETRY, TINT_ALPHA};
use crate::message::{Direction, Message};
use crate::resident::{EngineLink, UiCommand, UiOutcome};
use crate::scroll::scrollable;

mod apps;
mod calculator;
mod clipboard;
mod compass_commands;
mod developer;
mod dmenu;
mod emoji;
mod file_actions;
mod fonts;
mod global_shortcuts;
mod grants;
mod hud;
mod launch;
mod media;
mod onboarding;
mod open_with;
mod preview;
mod programs;
mod release_check;
mod rhai;
mod root;
mod runtime;
mod scripts;
mod settings_view;
mod shortcuts;
mod snippets;
mod stores;
mod themes;
mod tray;
mod workspaces;

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
const APP_OPEN: &str = "app.open";

/// The gap between the floating action panel and the card's edges.
const PANEL_MARGIN: f32 = 8.0;

/// The tallest the card grows to hold the action panel; a longer panel
/// scrolls.
const PANEL_ROOM_CAP: f32 = 360.0;
const APP_COPY_NAME: &str = "app.copy-name";
const APP_COPY_PATH: &str = "app.copy-path";
const APP_DESKTOP_ACTION: &str = "app.desktop:";

/// The nominal pixel size asked of the icon theme.
///
/// [`design::GEOMETRY`]'s `icon_size`, which is the square the row reserves.
/// Asking the theme for that size rather than a fixed 32 keeps the two from
/// drifting: a design change that grows the slot asks for a larger icon
/// instead of scaling a small one up.
const ICON_PIXELS: u32 = GEOMETRY.icon_size as u32;

/// Focus the search field.
///
/// Every path that puts the window on screen ends in one of these, because
/// focus does not survive the window being closed and reopened -- `conceal`
/// closes it, so a summon is a brand new window with a brand new, unfocused
/// field.
fn focus_search() -> Task<Message> {
    iced::widget::operation::focus(SEARCH_INPUT)
}

/// Set when the app asks its event loop to end.
static EXIT_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ends the event loop, noting that the app asked for it. `iced_winit` returns
/// `Ok` whether the loop ended on request or because the compositor
/// connection failed, and only this tells the two apart ([`exit_requested`]).
fn exit() -> Task<Message> {
    EXIT_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
    iced::exit()
}

/// Whether the app asked its event loop to end, rather than the loop ending
/// under it.
pub(crate) fn exit_requested() -> bool {
    EXIT_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
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

/// Lifts keyboard and window events out of the runtime's event stream.
///
/// A free function rather than a closure because [`iced::event::listen_with`]
/// takes a plain `fn` pointer. See [`LauncherApp::subscription`] for why the
/// stream is read at this level rather than through `iced::keyboard::listen`.
fn keyboard_events(
    event: iced::Event,
    _status: iced::event::Status,
    window: window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Keyboard(event) => Some(Message::Keyboard(event)),
        iced::Event::Window(window::Event::Focused) => Some(Message::WindowFocusChanged(true)),
        iced::Event::Window(window::Event::Unfocused) => Some(Message::WindowFocusChanged(false)),
        iced::Event::Window(window::Event::Opened { size, .. }) => Some(Message::WindowSized {
            id: window,
            size,
            opened: true,
        }),
        iced::Event::Window(window::Event::Resized(size)) => Some(Message::WindowSized {
            id: window,
            size,
            opened: false,
        }),
        _ => None,
    }
}

/// Which result `Ctrl+1`..`Ctrl+9` asks for, as a **position in the visible
/// list** counting from zero.
///
/// # The two things this is easy to get wrong
///
/// It returns a POSITION, not an item index. `LauncherApp::results` holds
/// indices into the application index, and the selection is a position within
/// `results` -- so `Ctrl+3` means the third row on screen, not item 3. Mixing
/// those up launches something plausible and wrong, which is the worst kind of
/// bug to have here because nothing looks broken.
///
/// And there is no zero. `Ctrl+1` is the first row, so the digit is one-based
/// and `Ctrl+0` is not a quick-launch chord at all.
///
/// Bounds are the caller's: this says which row was asked for, not whether it
/// exists.
#[must_use]
pub fn quick_launch_position(
    key: iced::keyboard::Key<&str>,
    modifiers: iced::keyboard::Modifiers,
) -> Option<usize> {
    use iced::keyboard::Key;

    // Control alone. A chord carrying Alt or Logo as well is somebody else's,
    // and claiming it would take a binding the desktop may already have.
    // Shift is excluded too: Ctrl+Shift+1 is a different chord, and on many
    // layouts it is not a digit at all.
    if !modifiers.control() || modifiers.alt() || modifiers.logo() || modifiers.shift() {
        return None;
    }

    let Key::Character(text) = key else {
        return None;
    };

    // `"1".."9"`, and nothing else: not "10", not a digit with anything
    // attached. `chars().next()` on a longer string would accept "1x".
    let mut chars = text.chars();
    let digit = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let value = digit.to_digit(10)?;
    if value == 0 {
        return None;
    }
    Some(value as usize - 1)
}

type IconFinder = dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync;

/// Resolves an `Icon=` name to a file. See [`AppFlags::icon_lookup`].
///
/// A named type rather than a bare `Arc<dyn Fn…>` so it can carry a `Debug`
/// impl: [`AppFlags`] derives `Debug`, and a closure does not.
#[derive(Clone)]
pub struct IconLookup(Arc<IconFinder>);

impl IconLookup {
    /// Wraps a lookup.
    #[must_use]
    pub fn new(find: impl Fn(&str) -> Option<std::path::PathBuf> + Send + Sync + 'static) -> Self {
        Self(Arc::new(find))
    }

    /// The file for `name`, if the lookup can find one.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<std::path::PathBuf> {
        (self.0)(name)
    }
}

impl std::fmt::Debug for IconLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IconLookup(..)")
    }
}

impl Default for IconLookup {
    /// The XDG icon themes, at the size the row reserves.
    fn default() -> Self {
        Self::new(|name: &str| {
            compass_xdg::find_icon(
                name,
                Some(&compass_xdg::default_theme()),
                Some(ICON_PIXELS),
                1.0,
            )
        })
    }
}

/// A shortcut's stored icon as its root row draws it (`RootShortcutItem::iconUrl`):
/// the `ImageURL` it names, a builtin on a purple tile.
fn shortcut_url(icon: &str) -> compass_core::image_url::ImageUrl {
    let url = compass_core::image_url::ImageUrl::parse(icon);
    if url.is_builtin() {
        url.with_background_tint(compass_core::image_url::ColorLike::Semantic(
            "Purple".into(),
        ))
    } else {
        url
    }
}

/// A clipboard link row's favicon, with the link builtin as its fallback
/// (`ClipboardHistoryModel`'s icon for a link).
fn clipboard_url(
    entry: &crate::backend::ClipboardRow,
) -> Option<compass_core::image_url::ImageUrl> {
    use compass_core::image_url::{ImageUrl, ImageUrlType};
    let host = entry.url_host.as_deref().filter(|host| !host.is_empty())?;
    (entry.kind == crate::backend::ClipboardRowKind::Link).then(|| {
        ImageUrl::new(ImageUrlType::Favicon, host).with_fallback(&ImageUrl::builtin("link"))
    })
}

/// Flags for configuring the launcher app.
#[derive(Debug, Clone)]
pub struct AppFlags {
    /// Curated color theme (#153), or System for Adwaita.
    pub theme: crate::theme::Theme,
    /// Where Set Theme looks for theme files; none in tests.
    pub theme_dirs: Vec<std::path::PathBuf>,
    /// The file views' remembered filters are kept in; `None` keeps them in
    /// memory, as tests do.
    pub view_state_path: Option<std::path::PathBuf>,
    /// The `fallbacks` entries a non-empty query offers, as ids.
    pub fallbacks: Vec<String>,
    /// Window configuration.
    pub window_config: window::Settings,
    /// How to launch the selected application.
    ///
    /// Injected rather than reached for: this crate must not know whether it
    /// is on Linux. `compass` supplies `compass-platform-linux`'s launcher;
    /// tests supply their own. See ADR-0013.
    pub launcher: Arc<dyn AppLauncher>,
    /// Shared ranking/history service when attached to an engine.
    pub backend: Option<Arc<dyn crate::backend::ApplicationBackend>>,
    /// Clipboard history, which only an attached engine has.
    pub clipboard: Option<Arc<dyn crate::backend::ClipboardBackend>>,
    /// Window switching, which only an attached engine has.
    pub windows: Option<Arc<dyn crate::backend::WindowBackend>>,
    /// Startup root settings for local searches; attached searches use the engine's settings.
    pub root_config: compass_core::root_items::RootConfig,
    /// The navigation chord scheme, from `launcher.keybinding`.
    pub keybinding: compass_core::keybinding::Scheme,
    /// Whether the selection wraps, from `launcher.wrap_navigation`.
    pub wrap_navigation: bool,
    /// Whether Ctrl+1..9 launches the Nth result, from `launcher.quick_launch`.
    pub quick_launch: bool,
    /// Whether the window hides when it loses focus, from
    /// `launcher.close_on_focus_loss`.
    pub close_on_focus_loss: bool,
    /// `launcher.pop_to_root_on_close`: hiding clears the search.
    pub pop_to_root_on_close: bool,
    /// The launcher hotkey as stored (`launcher.hotkey`), for the shortcut
    /// recorder's conflict check.
    pub launcher_hotkey: String,
    /// The compositor that binds the launcher's key in its own configuration
    /// (Sway, Hyprland, niri), when this is one; there the hotkey setting
    /// does nothing and the guidance names the compositor's line instead.
    pub hotkey_compositor: Option<compass_core::hotkey_guide::Compositor>,
    /// Whether this is the Flatpak, whose toggle command is longer.
    pub flatpak: bool,
    /// Whether each power command asks first, by id, as its `confirm`
    /// preference resolves (`compass_core::power_commands::should_confirm`).
    /// A command missing here asks by its own default.
    pub power_asks: std::collections::BTreeMap<String, bool>,
    /// Browse Apps' `showHidden` and `sortAlphabetically` preferences.
    pub browse_apps: compass_core::browse_apps::Options,
    /// The configuration file commands read their preferences from when
    /// they open (Browse Apps); `None` keeps the ones given at start.
    pub config_path: Option<std::path::PathBuf>,
    /// Where the emoji picker's visits, pins, tones and keywords are kept
    /// (`compass_core::glyph_service::default_path`); `None` keeps them in
    /// memory, as tests do.
    pub glyph_path: Option<std::path::PathBuf>,
    /// The engine's log file, which Show Log File shows
    /// ([`compass_core::xdg_dirs::log_file`]); `None` has none to show.
    pub log_path: Option<std::path::PathBuf>,
    /// Where Open Default Config File writes the defaults for the editor
    /// (the cache directory's `compass`); `None` has nowhere to write them.
    pub default_config_dir: Option<std::path::PathBuf>,
    /// Where the builtin icon set is installed
    /// ([`compass_core::builtin_icon::directory`]); `None` draws initials
    /// where a builtin icon would go.
    pub builtin_icons: Option<std::path::PathBuf>,
    /// The emoji picker's `skinTone` preference, as a tone id.
    pub emoji_skin_tone: Option<String>,
    /// The picker's `defaultAction` preference, `paste` unless set: what
    /// Enter does over a glyph.
    pub emoji_default_action: String,
    /// Where the root search's history is kept
    /// (`compass_core::root_view::default_history_path`); `None` keeps it in
    /// memory, as tests do.
    pub search_history_path: Option<std::path::PathBuf>,
    /// The root search's clock (`launcher.clock`); `None` shows none.
    pub clock: Option<ClockSettings>,
    /// Where favicons come from (`favicon_service`).
    pub favicon_service: compass_core::favicon::Service,
    /// Whether root rows fetch their remote icons (an `https` script icon,
    /// a shortcut's or a clipboard link's favicon) into
    /// [`crate::remote_image`]'s cache. Off by default, so a test never
    /// reaches the network or the cache under the real home; `compass` turns
    /// it on.
    pub remote_icons: bool,
    /// When the process started, for the cold-start figure (#13).
    ///
    /// `None` in a test or anywhere nobody is timing, which simply means no
    /// figure is logged. Taken by the binary on entry rather than here, so it
    /// covers as much of the startup as a process can see of itself.
    pub started_at: Option<std::time::Instant>,
    /// The resolved appearance preset (#84): geometry and structural flags.
    pub appearance_preset: crate::preset::Resolved,
    /// Whether result rows show the application's icon, from
    /// `launcher.appearance.icons` (#85).
    ///
    /// Resolved from the preset and any explicit key, so this is the answer
    /// rather than the configured value. See [`crate::preset::resolve`].
    pub icons: bool,
    /// How an `Icon=` name becomes a file on disk.
    ///
    /// Injected for the reason [`AppFlags::launcher`] is: the default walks the
    /// XDG icon themes, so a test that did not replace it would assert against
    /// whatever the machine running it happens to have installed. See
    /// ADR-0013.
    pub icon_lookup: IconLookup,
    /// What the window draws before the desktop says otherwise.
    ///
    /// Separate from `appearance_link` because a window has to draw before any
    /// change can arrive: this is the first frame's colour, and the link only
    /// carries what comes after.
    pub appearance: Appearance,
    /// Changes to the desktop's light/dark preference, when something is
    /// feeding them.
    ///
    /// `None` is a desktop with no Settings portal, or a test. The window then
    /// stays on `appearance` for its whole life, which is the honest outcome:
    /// nothing is telling it otherwise.
    pub appearance_link: Option<crate::appearance::AppearanceLink>,
    /// The engine driving this window, when there is one.
    ///
    /// `None` is the standalone case -- `compass ui` run by hand with no daemon
    /// -- and it changes what dismissing means: with nothing able to summon the
    /// window back, hiding it would strand the process invisible, so it exits.
    pub link: Option<EngineLink>,
    /// End an app-grid session when its engine goes away. Explicit UI mode opts out.
    pub exit_on_engine_disconnect: bool,
    /// Start without a window when an engine link can summon it later.
    pub start_hidden: bool,
    /// Where the first-run flow records that it was finished, when it is
    /// due (`compass_core::onboarding::should_show`): the window then opens
    /// on it, even when started hidden, as the C++ shows its onboarding
    /// window at server start. `None` when it is not due.
    pub onboarding: Option<std::path::PathBuf>,
    /// The desktop's interface font family, as `org.gnome.desktop.interface font-name`.
    ///
    /// `None` is the historic hard-coded `Cantarell` path; `Some` means the
    /// launcher follows whatever the desktop reports, with `FONT_STACK` as
    /// fallbacks for families not installed on this image. See
    /// `crate::typography`.
    pub font_family: Option<String>,
    /// Live updates to the font family, when something is feeding them.
    ///
    /// Mirrors `appearance_link`: `None` is a test or a desktop without a
    /// Settings portal.
    pub typography_link: Option<crate::typography::TypographyLink>,
    /// `compass.json` as it is read again after each change on disk, when
    /// something is watching it. `None` in tests.
    pub config_link: Option<crate::config_link::ConfigLink>,
}

impl Default for AppFlags {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        let platform_specific = window::settings::PlatformSpecific {
            application_id: crate::APP_ID.to_owned(),
            ..Default::default()
        };
        #[cfg(not(target_os = "linux"))]
        let platform_specific = window::settings::PlatformSpecific::default();
        Self {
            theme: crate::theme::Theme::System,
            theme_dirs: Vec::new(),
            view_state_path: None,
            fallbacks: Vec::new(),
            window_config: window::Settings {
                size: iced::Size::new(
                    f32::from(GEOMETRY.card_width + 2 * design::SHADOW_PADDING),
                    f32::from(GEOMETRY.card_max_height + 2 * design::SHADOW_PADDING),
                ),
                position: window::Position::Centered,
                resizable: false,
                decorations: false,
                transparent: true,
                platform_specific,
                ..Default::default()
            },
            keybinding: compass_core::keybinding::Scheme::default(),
            wrap_navigation: compass_core::config::DEFAULT_WRAP_NAVIGATION,
            quick_launch: compass_core::config::DEFAULT_QUICK_LAUNCH,
            close_on_focus_loss: compass_core::config::DEFAULT_CLOSE_ON_FOCUS_LOSS,
            pop_to_root_on_close: compass_core::config::DEFAULT_POP_TO_ROOT_ON_CLOSE,
            launcher_hotkey: compass_core::config::DEFAULT_HOTKEY.to_owned(),
            hotkey_compositor: None,
            flatpak: false,
            power_asks: std::collections::BTreeMap::new(),
            browse_apps: compass_core::browse_apps::Options::default(),
            config_path: None,
            glyph_path: None,
            log_path: None,
            default_config_dir: None,
            builtin_icons: None,
            emoji_skin_tone: None,
            emoji_default_action: compass_core::emoji_grid::DEFAULT_ACTION_PASTE.to_owned(),
            search_history_path: None,
            clock: None,
            favicon_service: compass_core::favicon::Service::default(),
            remote_icons: false,
            icons: compass_core::config::DEFAULT_ICONS,
            icon_lookup: IconLookup::default(),
            appearance_preset: crate::preset::resolve(None, None, None),
            started_at: None,
            // Deliberately the launcher that launches nothing. A default that
            // silently picked a real backend would make the platform choice
            // invisible at the call site, which is the arrangement ADR-0013
            // exists to end. `compass` sets this explicitly.
            launcher: Arc::new(NullLauncher),
            backend: None,
            clipboard: None,
            windows: None,
            root_config: compass_core::root_items::RootConfig::default(),
            link: None,
            exit_on_engine_disconnect: false,
            start_hidden: false,
            onboarding: None,
            // Light, until a desktop says otherwise. This is Adwaita's
            // documented no-preference fallback; `compass` replaces it with
            // the portal's native choice before the first frame when possible.
            appearance: Appearance::Light,
            appearance_link: None,
            font_family: None,
            typography_link: None,
            config_link: None,
        }
    }
}

/// The action panel's own state while it is open.
///
/// Its selection is an `isize` because -1 means "nothing selectable", which is
/// a real state for a panel filtered down to nothing and is not the same as
/// row 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelState {
    /// The sections, as the command supplied them.
    pub sections: Vec<PanelSection>,
    /// What has been typed into the panel's own filter.
    pub filter: String,
    /// The flattened rows under that filter.
    pub rows: Vec<Row>,
    /// The selected row, or -1.
    pub selected: isize,
    /// The shortcut recorder, when it has taken the panel's place.
    pub recorder: Option<crate::shortcut_recorder::ShortcutRecorder>,
}

impl PanelState {
    /// Open a panel over `sections`, selecting its first action.
    #[must_use]
    pub fn new(sections: Vec<PanelSection>) -> Self {
        let rows = action_panel::flatten(&sections, "");
        let selected = action_panel::selection_after_filter(&rows);
        Self {
            sections,
            filter: String::new(),
            rows,
            selected,
            recorder: None,
        }
    }

    /// Re-filter, and put the selection back on the first row of what is left.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.rows = action_panel::flatten(&self.sections, &self.filter);
        self.selected = action_panel::selection_after_filter(&self.rows);
    }

    /// The action the selection is on, if any.
    #[must_use]
    pub fn selected_action(&self) -> Option<&Action> {
        let row = self.rows.get(usize::try_from(self.selected).ok()?)?;
        self.sections.get(row.section)?.actions.get(row.action?)
    }

    /// The row of the action whose shortcut is the key `pressed`.
    #[must_use]
    pub fn row_answering(&self, pressed: &action_panel::Pressed<'_>) -> Option<usize> {
        self.rows.iter().position(|row| {
            row.action
                .and_then(|action| self.sections.get(row.section)?.actions.get(action))
                .is_some_and(|action| action.answers_to(pressed))
        })
    }

    /// The row of the action called `title`, for a test to click.
    #[cfg(test)]
    fn row_titled(&self, title: &str) -> Option<usize> {
        self.rows.iter().position(|row| {
            row.action
                .and_then(|action| self.sections.get(row.section)?.actions.get(action))
                .is_some_and(|action| action.title == title)
        })
    }
}

/// The actions offered for an application.
///
/// Launching is first because it is what the return key does, and the panel's
/// first row is the one the return key runs -- so the panel opening does not
/// change what enter means.
#[must_use]
pub fn actions_for_app(item: &AppItem) -> Vec<PanelSection> {
    let mut primary = vec![Action::new("Open").with_id(APP_OPEN).with_shortcut("enter")];
    if !item.is_action() && item.entry().is_application() {
        for action in item.entry().actions() {
            if let Some(name) = action.name().filter(|name| !name.is_empty())
                && action.exec().is_some()
            {
                primary.push(
                    Action::new(name).with_id(format!("{APP_DESKTOP_ACTION}{}", action.id())),
                );
            }
        }
    }
    let mut copy = vec![Action::new("Copy name").with_id(APP_COPY_NAME)];
    if item.path().is_some() {
        copy.push(Action::new("Copy path").with_id(APP_COPY_PATH));
    }
    vec![
        PanelSection {
            name: String::new(),
            actions: primary,
        },
        PanelSection {
            name: "Copy".to_owned(),
            actions: copy,
        },
    ]
}

fn launch_task(
    launcher: Arc<dyn AppLauncher>,
    entry: compass_xdg::DesktopEntry,
    action_id: Option<String>,
    backend: Option<Arc<dyn crate::backend::ApplicationBackend>>,
    key: String,
) -> Task<Message> {
    Task::perform(
        async move {
            match action_id {
                Some(id) => launcher.launch_action(&entry, &id, &[]).await,
                None => launcher.launch(&entry, &[]).await,
            }
            .map(|_| ())
            .map_err(|error| error.to_string())?;
            if let Some(backend) = backend
                && let Err(error) = backend.record_launch(key).await
            {
                // The application already opened. A history failure must not
                // invite the user to retry that successful launch.
                tracing::warn!(%error, "could not record successful launch");
            }
            Ok(())
        },
        Message::Launched,
    )
}

/// What the search field sends as the user types.
type OnInput = fn(String) -> Message;

/// A change to one clipboard history entry, from its keyboard shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardChange {
    TogglePin,
    Remove,
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

/// A key press as an extension shortcut: its modifiers and the key's name
/// as `jsx.d.ts` spells it. `None` for a press with no Control, Alt or Super,
/// which is typing or navigation, never a shortcut.
/// A key's name as panel shortcuts spell it: a character lower-cased, or
/// the named key (`enter`, `up`, `delete`, ...). `None` for a key no
/// shortcut names.
fn pressed_key_name(key: &iced::keyboard::Key) -> Option<String> {
    use iced::keyboard::{Key, key::Named};
    Some(match key.as_ref() {
        Key::Character(c) => c.to_lowercase(),
        Key::Named(named) => match named {
            Named::Enter => "enter",
            Named::Delete => "delete",
            Named::Backspace => "backspace",
            Named::Tab => "tab",
            Named::Space => "space",
            Named::ArrowUp => "up",
            Named::ArrowDown => "down",
            Named::ArrowLeft => "left",
            Named::ArrowRight => "right",
            Named::Home => "home",
            Named::End => "end",
            Named::PageUp => "pageup",
            Named::PageDown => "pagedown",
            _ => return None,
        }
        .to_owned(),
        Key::Unidentified => return None,
    })
}

fn extension_chord(
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<(Vec<compass_extension_api::action::KeyModifier>, String)> {
    use compass_extension_api::action::KeyModifier;
    use iced::keyboard::{Key, key::Named};
    if !(modifiers.control() || modifiers.alt() || modifiers.logo()) {
        return None;
    }
    let name = match key {
        Key::Character(c) => c.to_lowercase(),
        Key::Named(named) => match named {
            Named::Enter => "return",
            Named::Delete => "delete",
            Named::Backspace => "backspace",
            Named::Tab => "tab",
            Named::Space => "space",
            Named::ArrowUp => "arrowUp",
            Named::ArrowDown => "arrowDown",
            Named::ArrowLeft => "arrowLeft",
            Named::ArrowRight => "arrowRight",
            Named::Home => "home",
            Named::End => "end",
            Named::PageUp => "pageUp",
            Named::PageDown => "pageDown",
            _ => return None,
        }
        .to_owned(),
        Key::Unidentified => return None,
    };
    let mut mods = Vec::new();
    if modifiers.control() {
        mods.push(KeyModifier::Ctrl);
    }
    if modifiers.alt() {
        mods.push(KeyModifier::Alt);
    }
    if modifiers.shift() {
        mods.push(KeyModifier::Shift);
    }
    if modifiers.logo() {
        mods.push(KeyModifier::Meta);
    }
    Some((mods, name))
}

/// How a shortcut reads in the action panel: `Ctrl+Shift+C`.
fn shortcut_label(shortcut: &compass_extension_api::action::Shortcut) -> String {
    use compass_extension_api::action::KeyModifier;
    let mut parts: Vec<String> = shortcut
        .modifiers
        .iter()
        .map(|modifier| {
            match modifier {
                KeyModifier::Ctrl | KeyModifier::Cmd => "Ctrl",
                KeyModifier::Alt => "Alt",
                KeyModifier::Shift => "Shift",
                KeyModifier::Meta => "Super",
            }
            .to_owned()
        })
        .collect();
    let key = shortcut.key.as_str();
    parts.push(if key.chars().count() == 1 {
        key.to_uppercase()
    } else {
        let mut chars = key.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    });
    parts.join("+")
}

/// Panel ids for an extension's actions: this prefix, then the handler.
const EXTENSION_ACTION: &str = "ext:";

/// An extension view's actions on offer, as panel sections. Submenus are
/// flattened into their section for now.
fn extension_panel_sections(page: &crate::extension_page::ExtensionPage) -> Vec<PanelSection> {
    use compass_extension_api::action::ActionItem;
    fn actions(items: &[ActionItem], out: &mut Vec<action_panel::Action>) {
        for item in items {
            match item {
                ActionItem::Action(action) => {
                    let row = action_panel::Action::new(action.title.clone())
                        .with_id(format!("{EXTENSION_ACTION}{}", action.handler.0));
                    out.push(match &action.shortcut {
                        Some(shortcut) => row.with_shortcut(shortcut_label(shortcut)),
                        None => row,
                    });
                }
                ActionItem::Submenu(submenu) => actions(&submenu.items, out),
            }
        }
    }
    page.actions()
        .map(|panel| {
            panel
                .sections
                .iter()
                .map(|section| {
                    let mut list = Vec::new();
                    actions(&section.items, &mut list);
                    PanelSection {
                        name: section.title.clone().unwrap_or_default(),
                        actions: list,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
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
}

/// What a dismissal does. See [`LauncherApp::on_dismiss`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dismissal {
    /// Close the window and wait to be summoned again.
    Hide,
    /// End the process, because nothing could summon it back.
    Exit,
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

/// The card's background colour, given the surface colour and whether `tint` is on.
///
/// Extracted from `view`'s style closure on purpose. A headless renderer can
/// assert what is laid out but not what colour a container was filled with, so
/// the alpha decision inside a closure would be untestable without a pixel
/// baseline — and pixel baselines get rubber-stamped, which is why §8.5 rejects
/// them. As a function it is an ordinary assertion.
///
/// Translucency is applied here rather than in the palette so the theme stays
/// one set of colours: `tint` is a property of the window, not of the scheme,
/// and a palette that changed with it would make every other surface's
/// contrast depend on a background setting.
fn card_background(surface: design::Rgb, tint: bool) -> iced::Color {
    let colour = surface.to_iced();
    if tint {
        colour.scale_alpha(TINT_ALPHA)
    } else {
        colour
    }
}

/// The search field's and the panel filter's text: the enclosing field owns
/// the fill and the rounded border, and the placeholder is the secondary
/// text colour, which the palette holds at 4.5:1. Iced's own placeholder is
/// a faded text colour, 3.03:1 on the light field.
fn query_input_style(
    theme: &Theme,
    status: text_input::Status,
    palette: design::Palette,
) -> text_input::Style {
    text_input::Style {
        background: Color::TRANSPARENT.into(),
        border: Border::default(),
        placeholder: palette.muted.to_iced(),
        ..text_input::default(theme, status)
    }
}

impl LauncherApp {
    /// Create a new launcher application, indexing the environment.
    pub fn new(flags: AppFlags) -> (Self, Task<Message>) {
        let mut app = Self::with_index(AppIndex::from_environment());
        app.apply(flags);
        (app, Task::none())
    }

    /// Copy the flags onto an already-built state.
    ///
    /// Split out of [`LauncherApp::new`] so the copying is reachable without
    /// `AppIndex::from_environment`, which reads the invoking user's real
    /// application directories. A test that reproduced this assignment list
    /// instead of calling it would pass while the real one stopped copying a
    /// field -- which is exactly the mutation it is meant to catch.
    fn apply(&mut self, flags: AppFlags) {
        let app = self;
        app.launcher = flags.launcher;
        app.backend = flags.backend;
        app.clipboard = flags.clipboard;
        app.windows = flags.windows;
        app.app_index.apply_root_config(&flags.root_config);
        app.root_config = flags.root_config;
        app.search_history = flags
            .search_history_path
            .as_deref()
            .map(compass_core::root_view::SearchHistory::load_file)
            .unwrap_or_default();
        app.search_history_path = flags.search_history_path;
        app.clock = flags.clock;
        app.fallbacks = flags.fallbacks;
        app.window_config = flags.window_config;
        app.keybinding = flags.keybinding;
        app.wrap_navigation = flags.wrap_navigation;
        app.quick_launch = flags.quick_launch;
        app.close_on_focus_loss = flags.close_on_focus_loss;
        app.pop_to_root_on_close = flags.pop_to_root_on_close;
        app.launcher_hotkey = flags.launcher_hotkey;
        app.hotkey_compositor = flags.hotkey_compositor;
        app.flatpak = flags.flatpak;
        app.power_asks = flags.power_asks;
        app.browse_apps = flags.browse_apps;
        app.config_path = flags.config_path;
        app.glyph_path = flags.glyph_path;
        app.log_path = flags.log_path;
        app.default_config_dir = flags.default_config_dir;
        app.builtin_icons = flags.builtin_icons;
        app.emoji_skin_tone = flags.emoji_skin_tone;
        app.emoji_default_action = flags.emoji_default_action;
        app.icons = flags.icons;
        app.remote_icons = flags.remote_icons;
        app.favicon_service = flags.favicon_service;
        app.geometry = flags.appearance_preset.geometry;
        app.field_rule = flags.appearance_preset.field_rule;
        app.tint = flags.appearance_preset.tint;
        app.subtitles = flags.appearance_preset.subtitles;
        app.started_at = flags.started_at;
        app.icon_lookup = flags.icon_lookup;
        app.link = flags.link;
        app.exit_on_engine_disconnect = flags.exit_on_engine_disconnect;
        app.view.theme_choice = flags.theme;
        app.view.theme_dirs = flags.theme_dirs;
        app.view.view_memory = crate::view_memory::ViewMemory::load(flags.view_state_path);
        app.view.appearance = flags.appearance;
        app.view.appearance_link = flags.appearance_link;
        app.view.font_family = flags.font_family;
        app.view.typography_link = flags.typography_link;
        app.config_link = flags.config_link;
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
        }
    }

    /// Whether a window is currently on screen.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.window.is_some()
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

    /// Answers the engine, if it is waiting for one.
    ///
    /// Does nothing when no command is outstanding. See [`Self::awaiting`] for
    /// why that matters more than it looks.
    fn answer(&mut self, outcome: UiOutcome) {
        if !self.awaiting {
            return;
        }
        self.awaiting = false;
        if let Some(link) = &self.link {
            link.report(outcome);
        }
    }

    /// Whether the engine is waiting for an outcome. For tests.
    #[must_use]
    pub fn is_awaiting(&self) -> bool {
        self.awaiting
    }

    /// What dismissing does, given whether anything could bring the window back.
    ///
    /// Split out from `LauncherApp::conceal` because the two outcomes it
    /// chooses between are both opaque `Task`s: a test can see that the window
    /// closed, but not that the process was told to exit. This is the decision
    /// itself, and it is what the tests assert on.
    #[must_use]
    pub fn on_dismiss(&self) -> Dismissal {
        if self.link.is_some() {
            Dismissal::Hide
        } else {
            Dismissal::Exit
        }
    }

    /// Hides the window, or ends the process when nothing could summon it back.
    ///
    /// This is what dismissing and a successful launch both do. A launcher that
    /// stayed on screen after launching is a bug report waiting to happen; a
    /// launcher that vanished with no way back is a worse one.
    fn conceal(&mut self) -> Task<Message> {
        // Dismissing the first-run flow, by Escape or the hotkey, counts as
        // having seen it: it is not shown again at every login.
        if let Page::Onboarding(page) = &self.page {
            onboarding::record_completed(&page.state_path);
        }
        self.cancel_search();
        self.panel = None;
        // Leaving Set Theme without choosing puts the theme back, as
        // `beforePop` does.
        if matches!(self.page, Page::Themes(_)) {
            let _ = self.update(Message::ThemeCancel);
        }
        self.parked_fonts = None;
        self.parked_store = None;
        self.provider_scope = None;
        self.open_with_return = None;
        let dismissed = self.cancel_dmenu();
        let closing = Task::batch([dismissed, self.close_extension_view()]);
        // A summon starts at the root, whatever view was open when it hid.
        self.page = Page::Root;
        let cleared = if self.pop_to_root_on_close && !self.query.is_empty() {
            self.update(Message::QueryChanged(String::new()))
        } else {
            Task::none()
        };
        let hidden = self.hide_window();
        Task::batch([closing, cleared, hidden])
    }

    /// Hides or exits after [`Self::conceal`] has reset the view.
    fn hide_window(&mut self) -> Task<Message> {
        self.reopen_after_close = false;
        self.window_focused = false;
        if self.on_dismiss() == Dismissal::Exit {
            return exit();
        }
        if self.pending_window.is_some() {
            self.pending_hide = true;
            return Task::none();
        }
        if self.link.is_none() {
            // Unreachable: `on_dismiss` returns `Hide` only when there is a
            // link. Written as a return rather than an unwrap so a future
            // change to `on_dismiss` degrades into exiting rather than
            // panicking in the middle of a keystroke.
            return exit();
        }
        match self.window {
            // The answer waits for `Message::Closed`, which arrives when the
            // window is actually gone.
            //
            // REPORTING HERE WOULD BE OPTIMISTIC, AND IT WAS. `window::close`
            // returns a Task; answering before it runs tells the engine
            // "hidden" while the window is still on screen. A VM run caught it:
            // `compass toggle` reported success and the screenshot taken
            // straight afterwards still had the launcher in it.
            //
            // `self.window` is deliberately NOT cleared yet. Until the close
            // lands the window really is still visible, and `is_visible` should
            // say so -- the engine cannot send another command in the meantime
            // because it is blocked reading this one's reply.
            Some(id) => {
                self.closing = true;
                Task::batch([window::close(id), self.close_backdrop()])
            }
            // Nothing to close, so nothing to wait for. Still answers, because
            // the engine is blocked until it hears something.
            None => {
                self.answer(UiOutcome::Hidden);
                self.close_backdrop()
            }
        }
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

    /// One line saying what the launcher is showing, for a log.
    ///
    /// # Why this exists, and why it is a function rather than a `tracing!`
    ///
    /// The VM tier asserts what the launcher did by counting changed pixels.
    /// That is the only way to prove something reached the screen, and it is
    /// terrible at everything else: "5.44% of pixels in a box changed" cannot
    /// say *which* row is selected, what the query matched, or what the panel
    /// contains, and it moves with the font, the theme and the card geometry.
    /// Two of this tier's runs were spent on a containment box that had gone
    /// stale, asserting nothing about the launcher at all.
    ///
    /// So the state machine says what it did, in a line a grep can check
    /// exactly, and the pixels go back to proving only the thing they are
    /// uniquely good for: that a window was drawn.
    ///
    /// It is a pure function returning a `String` rather than a `tracing`
    /// macro at the call site so that its content is unit-testable. A log line
    /// asserted only inside a 25-minute VM run is a log line nobody can
    /// control.
    ///
    /// The shape is `key=value`, space separated, with titles quoted, because
    /// that is what survives being grepped out of a log interleaved with
    /// wgpu's debug output.
    #[must_use]
    pub fn state_line(&self) -> String {
        let mut line = format!(
            "query={:?} results={} selected={}",
            self.query,
            self.results.len(),
            self.selected
        );
        // The title, not just the index: an index is only meaningful against a
        // corpus the reader cannot see, and "selected=0" is equally true of a
        // right and a wrong first row.
        match self.selected_row() {
            Some(RootRow::App(_)) => {
                let name = self.selected_item().map_or("", AppItem::name);
                line.push_str(&format!(" selected_title={name:?}"));
            }
            Some(RootRow::Command(command)) => {
                line.push_str(&format!(" selected_title={:?}", command.title));
            }
            Some(RootRow::Extension(index)) => {
                let title = self
                    .app_index
                    .extensions()
                    .get(index)
                    .map_or("", |command| command.title.as_str());
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Calculator) => {
                let answer = self.calculator.as_ref().map_or("", |a| a.answer.as_str());
                line.push_str(&format!(" selected_title={answer:?}"));
            }
            Some(RootRow::Update) => {
                let title = self
                    .update
                    .as_ref()
                    .map(release_check::title)
                    .unwrap_or_default();
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Shortcut(index)) => {
                let title = self
                    .app_index
                    .shortcuts()
                    .get(index)
                    .map_or("", crate::shortcuts_page::display_name);
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Script(index)) => {
                let title = self
                    .app_index
                    .scripts()
                    .get(index)
                    .map_or("", |script| script.title.as_str());
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::RhaiScript(index)) => {
                let title = self
                    .app_index
                    .rhai_scripts()
                    .get(index)
                    .map_or("", |script| script.title.as_str());
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Fallback(fallback)) => {
                let title = self.fallback_title(fallback).unwrap_or_default();
                line.push_str(&format!(" selected_title={title:?} fallback"));
            }
            None => line.push_str(" selected_title=none"),
        }
        if let Page::Themes(page) = &self.page {
            line.push_str(&format!(
                " page=themes themes_query={:?} themes_rows={} themes_selected={} theme={}",
                page.query,
                page.rows.len(),
                page.selected,
                self.view.theme_choice.name()
            ));
        }
        if let Page::Dmenu(page) = &self.page {
            line.push_str(&format!(
                " page=dmenu dmenu_token={} dmenu_query={:?} dmenu_shown={} dmenu_selected={}",
                page.token,
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Programs(page) = &self.page {
            line.push_str(&format!(
                " page=programs programs_query={:?} programs_rows={} programs_selected={}",
                page.query,
                page.rows.len(),
                page.selected
            ));
        }
        if let Page::ScriptOutput(page) = &self.page {
            line.push_str(&format!(
                " page=script_output script_session={} script_finished={} script_exit={:?}",
                page.session, page.state.finished, page.state.exit_code
            ));
        }
        if let Page::Snippets(page) = &self.page {
            line.push_str(&format!(
                " page=snippets snippets_query={:?} snippets_shown={} snippets_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Shortcuts(page) = &self.page {
            line.push_str(&format!(
                " page=shortcuts shortcuts_query={:?} shortcuts_shown={} shortcuts_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Windows(page) = &self.page {
            line.push_str(&format!(
                " page=windows windows_query={:?} windows_shown={} windows_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Workspaces(page) = &self.page {
            line.push_str(&format!(
                " page=workspaces workspaces_query={:?} workspaces_shown={} workspaces_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Clipboard(page) = &self.page {
            line.push_str(&format!(
                " page=clipboard clipboard_query={:?} clipboard_rows={} clipboard_selected={}",
                page.query,
                page.rows.len(),
                page.selected
            ));
        }
        if let Page::Files(page) = &self.page {
            line.push_str(&format!(
                " page=files files_query={:?} files_heading={:?} files_rows={} files_selected={}",
                page.query,
                page.heading,
                page.rows.len(),
                page.selected
            ));
        }
        match &self.panel {
            None => line.push_str(" panel=closed"),
            Some(panel) => {
                line.push_str(&format!(
                    " panel=open panel_filter={:?} panel_rows={} panel_selected={}",
                    panel.filter,
                    panel.rows.len(),
                    panel.selected
                ));
                let title = usize::try_from(panel.selected)
                    .ok()
                    .and_then(|index| panel.rows.get(index))
                    .and_then(|row| {
                        let action = row.action?;
                        Some(
                            panel
                                .sections
                                .get(row.section)?
                                .actions
                                .get(action)?
                                .title
                                .as_str(),
                        )
                    });
                match title {
                    Some(title) => line.push_str(&format!(" panel_title={title:?}")),
                    None => line.push_str(" panel_title=none"),
                }
            }
        }
        if let Some(error) = &self.error {
            line.push_str(&format!(" error={error:?}"));
        }
        line.push_str(if self.window.is_some() {
            " window=open"
        } else {
            " window=hidden"
        });
        line
    }

    /// The application title.
    pub fn title(&self) -> String {
        compass_core::tray::APP_NAME.to_owned()
    }

    /// The application theme.
    ///
    /// Adwaita's palette rather than one of Iced's built-ins, so the launcher
    /// looks like the desktop it sits on. [`crate::design`] holds the colours
    /// and the browser surrogate under `tools/design/` is served the same
    /// ones, which is what keeps the two renderings honest about each other.
    ///
    /// The appearance follows the desktop when something is feeding
    /// [`AppFlags::appearance_link`], and otherwise stays on
    /// [`AppFlags::appearance`] for the window's whole life.
    fn palette(&self) -> design::Palette {
        self.view.theme_choice.palette(self.view.appearance)
    }

    /// The font the launcher draws text with.
    ///
    /// Follows `org.gnome.desktop.interface font-name` when a family was
    /// supplied at startup (or later via `TypographyChanged`), otherwise
    /// `iced::Font::DEFAULT` which keeps the historic `Cantarell` fallback
    /// readable on a minimal image.
    fn font(&self) -> iced::Font {
        match &self.view.font_family {
            Some(family) => crate::typography::iced_font(family),
            None => iced::Font::DEFAULT,
        }
    }

    /// How every Markdown view is drawn: 14 px, in the launcher's own
    /// [`font`](Self::font).
    ///
    /// Iced's `Style::from(&Theme)` leaves `style.font` at `Font::default()`,
    /// the generic sans-serif, which cosmic-text maps to a hard-coded
    /// "Open Sans" rather than to the desktop's interface font. Where that
    /// family is missing — a stock GNOME install — every Markdown span went
    /// through cosmic-text's fallback chain instead, and its bold spans
    /// landed on whichever family happened to have a static bold face. Code
    /// keeps iced's monospace.
    fn markdown_settings(&self) -> iced::widget::markdown::Settings {
        let mut style = iced::widget::markdown::Style::from(&self.theme());
        style.font = self.font();
        iced::widget::markdown::Settings::with_text_size(14, style)
    }

    /// The application theme.
    pub fn theme(&self) -> Theme {
        let p = self.palette();
        if self.view.theme_choice == crate::theme::Theme::System {
            return design::theme(self.view.appearance);
        }
        iced::Theme::custom(
            format!(
                "Compass {}-{}",
                self.view.theme_choice.name(),
                self.view.appearance.name()
            ),
            iced::theme::Palette {
                background: p.surface.to_iced(),
                text: p.text.to_iced(),
                primary: p.accent.to_iced(),
                success: p.accent.to_iced(),
                warning: p.accent.to_iced(),
                danger: iced::Color::from_rgb8(0xe0, 0x1b, 0x24),
            },
        )
    }

    /// The surface's own style: what the runtime clears the window to before
    /// the view is drawn. Transparent, so only the card and its shadow are on
    /// screen. Left to the theme it is the palette's background (the card's
    /// surface colour), and the whole window, shadow padding included,
    /// paints as an opaque rectangle behind the card.
    #[must_use]
    pub fn style(&self, theme: &Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: theme.palette().text,
        }
    }

    /// Every keyboard event, consumed by a widget or not.
    ///
    /// THIS DELIBERATELY DOES NOT USE `iced::keyboard::listen`, and the reason
    /// is a regression that focusing the search field would otherwise have
    /// introduced.
    ///
    /// `listen` yields only events with `Status::Ignored`. That was fine while
    /// nothing focused the search field, because an unfocused `text_input`
    /// consumes nothing -- which is also why the launcher ignored the keyboard
    /// entirely (#91). Focusing it fixes the typing and changes this: a focused
    /// `text_input` handles Escape by unfocusing itself and CAPTURING the
    /// event, so under `listen` the first Escape would silently stop the user
    /// typing instead of dismissing the launcher, and only a second one would
    /// reach here. Arrows and printable keys are unaffected either way -- the
    /// field captures neither.
    ///
    /// `listen_with` sees events whatever their status, so Escape means dismiss
    /// on the first press. Nothing is handled twice as a result: `update` acts
    /// on the arrows and Escape and drops every other key, while the text the
    /// field consumes arrives separately through `on_input`.
    ///
    /// Note that this is not covered by a test, and cannot easily be: what
    /// broke would be event DELIVERY, inside the Iced runtime, not the handler
    /// in `update`. The VM tier types but never presses Escape, so it would not
    /// catch it either.
    pub fn subscription(&self) -> iced::Subscription<Message> {
        let mut streams = vec![
            iced::event::listen_with(keyboard_events),
            window::close_events().map(Message::Closed),
            crate::surface::opened_events(),
        ];
        // Only while the first frame is still owed. Once it has been reported
        // this stream is dropped, so the per-frame message stops entirely
        // rather than being produced and discarded at the refresh rate.
        if self.first_frame_at.is_none() || self.summoned_at.is_some() {
            streams.push(window::frames().map(|_| Message::FrameDrawn));
        }
        if let Some(link) = &self.link {
            streams.push(
                link.subscription()
                    .map(|command| command.map_or(Message::EngineDisconnected, Message::Command)),
            );
        }
        if let Some(link) = &self.view.appearance_link {
            streams.push(link.subscription().map(Message::AppearanceChanged));
        }
        if let Some(link) = &self.view.typography_link {
            streams.push(link.subscription().map(Message::TypographyChanged));
        }
        if let Some(link) = &self.config_link {
            streams.push(link.subscription().map(Message::ConfigReloaded));
        }
        // Each second while the clock shows; `clock_tick` redraws it only
        // when its interval comes round.
        if self.clock.is_some() && matches!(self.page, Page::Root) {
            streams.push(
                iced::time::every(std::time::Duration::from_secs(1)).map(|_| Message::ClockTick),
            );
        }
        streams.extend(self.hud_subscription());
        iced::Subscription::batch(streams)
    }

    /// Acts on a command from the engine and reports what happened.
    ///
    /// `Show` on an already-visible window reports `Shown` without opening a
    /// second one: the outcome names the state the window ended in, so
    /// "already there" and "just opened" are the same answer.
    fn obey(&mut self, command: UiCommand) -> Task<Message> {
        // From here until the outcome is sent, the engine is blocked reading
        // one reply. Set before any branch so every path answers exactly once.
        self.awaiting = true;

        if command == UiCommand::Describe {
            let open = (self.is_visible() && !self.closing)
                || (self.pending_window.is_some() && !self.pending_hide);
            self.answer(if open {
                UiOutcome::Shown
            } else {
                UiOutcome::Hidden
            });
            return Task::none();
        }

        if let UiCommand::Failure { title, message } = &command {
            let task = self.command_failed(title, message);
            let open = self.is_visible() && !self.closing;
            self.answer(if open {
                UiOutcome::Shown
            } else {
                UiOutcome::Hidden
            });
            return task;
        }

        if let UiCommand::Hud { text, icon } = &command {
            let hud = crate::hud::Hud {
                text: text.clone(),
                icon: icon.clone(),
            };
            let (shown, task) = self.put_up_hud_for_engine(hud);
            // The outcome names the launcher's state, which a HUD leaves alone.
            let open = self.is_visible() && !self.closing;
            self.answer(if shown && open {
                UiOutcome::Shown
            } else if shown {
                UiOutcome::Hidden
            } else {
                UiOutcome::Failed("this session has no HUD".to_owned())
            });
            return task;
        }

        let opened = match &command {
            UiCommand::Dmenu(token) => self.start_dmenu(*token),
            UiCommand::Launch(token) => self.start_launch(*token),
            UiCommand::Deeplink(url) => self.open_deeplink(url),
            _ => Task::none(),
        };
        let shown = self.obey_visibility(&command);
        Task::batch([opened, shown])
    }

    /// The visibility half of [`Self::obey`]: every command but `Hide`
    /// shows the window, `Toggle` depending on where it is.
    fn obey_visibility(&mut self, command: &UiCommand) -> Task<Message> {
        let show = match command {
            UiCommand::Show
            | UiCommand::Dmenu(_)
            | UiCommand::Launch(_)
            | UiCommand::Deeplink(_) => true,
            UiCommand::Hide => false,
            // Answered in `obey` without touching the window.
            UiCommand::Describe | UiCommand::Hud { .. } | UiCommand::Failure { .. } => {
                return Task::none();
            }
            UiCommand::Toggle => {
                let open = (self.is_visible() && !self.closing)
                    || (self.pending_window.is_some() && !self.pending_hide)
                    || self.reopen_after_close;
                !open || self.behind_another_window()
            }
        };

        if !show {
            self.summoned_at = None;
            return self.conceal();
        }

        if matches!(command, UiCommand::Show | UiCommand::Toggle)
            && let Some(path) = self.pending_onboarding.take()
        {
            self.open_onboarding(path);
        }

        // A toplevel left behind another window comes back as a new one.
        // Asking the compositor to raise and focus the old one is refused
        // without an activation token (Sway kept the keys with the other
        // window), while a new window is focused like every summon.
        if let (true, Some(id)) = (self.behind_another_window(), self.window) {
            self.summoned_at.get_or_insert_with(std::time::Instant::now);
            self.closing = true;
            self.reopen_after_close = true;
            return window::close(id);
        }

        if self.window.is_none() || self.closing {
            // A summon that has to put a window on screen: the case the
            // resident window (ADR-0015) exists to make fast. A second Show
            // while the first is still opening keeps the earlier start.
            self.summoned_at.get_or_insert_with(std::time::Instant::now);
        }

        if self.pending_window.is_some() {
            self.pending_hide = false;
            return Task::none();
        }
        if self.closing {
            self.reopen_after_close = true;
            return Task::none();
        }

        if let Some(id) = self.window {
            self.answer(UiOutcome::Shown);
            // Both, and they are not the same thing: `gain_focus` raises the
            // WINDOW, `focus_search` focuses the FIELD inside it. A window
            // summoned with only the first is on top and still deaf.
            return Task::batch([window::gain_focus(id), focus_search()]);
        }

        self.open_window()
    }

    fn open_window(&mut self) -> Task<Message> {
        let mut settings = self.window_config.clone();
        settings.size = self.wanted_window_size();
        let backdrop = self.open_backdrop();
        let (id, opened) = crate::surface::open(settings);
        self.pending_window = Some(id);
        self.pending_hide = false;
        self.window_size = None;
        self.monitor_asked = None;
        self.focus_seen = false;
        self.measuring = crate::surface::opens_to_measure().then_some(id);
        // Which of two surfaces on one layer is on top is the compositor's
        // call (Sway 1.9 put the one asked for first on top), so the
        // launcher is asked for first, and the
        // backdrop's input region leaves the launcher's rectangle out
        // (`cut_backdrop`) so a click there reaches the launcher either way.
        opened.chain(backdrop)
    }

    /// Leaves the launcher's rectangle out of the backdrop's input region,
    /// once both have their sizes and the launcher is not still measuring.
    fn cut_backdrop(&self) -> Task<Message> {
        match (self.backdrop, self.backdrop_size, self.window_size) {
            (Some(backdrop), Some(room), Some(launcher)) if self.measuring.is_none() => {
                crate::surface::cut_backdrop(backdrop, room, launcher)
            }
            _ => Task::none(),
        }
    }

    /// Puts the backdrop under a layer-shell launcher that hides on focus
    /// loss. A layer surface holding the keyboard exclusively never loses
    /// the focus, and a click on another window went through to it and left
    /// the launcher up (TIL-03); the backdrop takes that click instead. A
    /// toplevel loses the focus like any window and needs none.
    fn open_backdrop(&mut self) -> Task<Message> {
        if !self.close_on_focus_loss || self.backdrop.is_some() {
            return Task::none();
        }
        let id = window::Id::unique();
        let task = crate::surface::open_backdrop(id);
        if task.units() > 0 {
            self.backdrop = Some(id);
        }
        task
    }

    /// Takes the backdrop down with the launcher.
    fn close_backdrop(&mut self) -> Task<Message> {
        self.backdrop_size = None;
        self.backdrop.take().map_or_else(Task::none, window::close)
    }

    /// Whether the launcher is open but another window has the focus: a
    /// toplevel left behind one with `close_on_focus_loss` off. `Toggle`
    /// brings it forward rather than hiding what the user cannot see.
    ///
    /// Only once this window has reported its focus at all: a seat with no
    /// keyboard reports none, and the launcher must still toggle closed
    /// there. A layer surface on top with the keyboard is never behind.
    fn behind_another_window(&self) -> bool {
        crate::surface::presentation() == crate::surface::Presentation::Toplevel
            && self.is_visible()
            && !self.closing
            && self.focus_seen
            && !self.window_focused
    }

    /// The window size the current view asks for (the launcher's own, or a
    /// dmenu list's), fitted to the room last seen on the output.
    fn wanted_window_size(&self) -> iced::Size {
        let pad = 2 * u32::from(design::SHADOW_PADDING);
        let (width, height) = self.dmenu_card_size().unwrap_or((
            u32::from(GEOMETRY.card_width),
            u32::from(GEOMETRY.card_max_height),
        ));
        crate::surface::fit(
            iced::Size::new((width + pad) as f32, (height + pad) as f32),
            self.room,
        )
    }

    /// The launcher window was opened or resized. A layer surface opened to
    /// measure the output is given its fitted size; one being fitted is
    /// drawn, and focused, once it has it.
    fn window_sized(&mut self, id: window::Id, size: iced::Size, opened: bool) -> Task<Message> {
        if self.backdrop == Some(id) {
            self.backdrop_size = Some(size);
            return self.cut_backdrop();
        }
        // Under `iced::application` (`crate::run`) the one window is not
        // tracked by id, and its size is still the launcher's.
        let launcher = match (self.window, self.pending_window) {
            (None, None) => !self.hud.owns(id),
            (window, pending) => window == Some(id) || pending == Some(id),
        };
        if !launcher {
            return Task::none();
        }
        self.window_size = Some(size);
        if self.measuring != Some(id) {
            return self.cut_backdrop();
        }
        if opened {
            self.room = Some(size);
            let fitted = self.wanted_window_size();
            self.resized_to = self.dmenu_card_size();
            if fitted != size {
                return crate::surface::fit_opened(id, fitted);
            }
        }
        self.measuring = None;
        // The field was not in the tree while the window measured.
        Task::batch([focus_search(), self.cut_backdrop()])
    }

    /// The monitor a toplevel is on: its size is the room, and a window
    /// larger than that is shrunk to fit.
    fn monitor_sized(&mut self, id: window::Id, monitor: Option<iced::Size>) -> Task<Message> {
        let Some(monitor) = monitor else {
            return Task::none();
        };
        if self.window != Some(id) {
            return Task::none();
        }
        self.room = Some(monitor);
        let fitted = self.wanted_window_size();
        if self.window_size == Some(fitted) {
            return Task::none();
        }
        crate::surface::resize(id, fitted)
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

            // COLD START, RECORDED -- AND IT IS A LOWER BOUND, NOT THE SLA.
            //
            // §8.5 names "cold start to first frame < 120 ms" and nothing has
            // ever measured it. This is the first number.
            //
            // Iced yields this on `RedrawRequested`, which is the compositor
            // asking for a frame, NOT the frame reaching the screen. The
            // rendering that follows is unmeasured here and under llvmpipe it
            // is not small, so the figure is a floor on what a user waits for.
            // It is logged as `first_draw_ms` rather than `first_frame_ms` so
            // nobody reads it as the thing the SLA names.
            //
            // The elapsed time is from `AppFlags::started_at`, which `compass`
            // takes on entry -- so it excludes dynamic linking, and a wgpu
            // binary's is not free either. The VM tier measures from spawn to
            // this line and catches both.
            //
            // Recorded, not gated: ADR-0010. A threshold comes from the
            // numbers, once there are some.
            Message::FrameDrawn => {
                // SUMMON, RECORDED ON THE SAME TERMS AS COLD START BELOW: from
                // the engine's `Show` reaching this process to the new window's
                // first `RedrawRequested`. Only once that window has opened --
                // a frame from a window still closing is not the summoned one.
                // Logged as `summon_draw_ms`, a floor for the same reason
                // `first_draw_ms` is: painting what was requested is not in it.
                if self.window.is_some()
                    && self.pending_window.is_none()
                    && !self.closing
                    && let Some(summoned) = self.summoned_at.take()
                {
                    let drawn = summoned.elapsed();
                    self.last_summon_draw = Some(drawn);
                    tracing::info!(
                        target: "compass_ui::startup",
                        summon_draw_ms = drawn.as_millis() as u64,
                        "summoned window's first frame requested (a floor on summon latency)"
                    );
                }
                if self.first_frame_at.is_none() {
                    let now = std::time::Instant::now();
                    self.first_frame_at = Some(now);
                    if let Some(started) = self.started_at {
                        tracing::info!(
                            target: "compass_ui::startup",
                            first_draw_ms = now.duration_since(started).as_millis() as u64,
                            "first frame requested (a floor on cold start, not the §8.5 figure)"
                        );
                    }
                }
                Task::none()
            }
            Message::AppearanceChanged(appearance) => {
                self.view.appearance = appearance;
                Task::none()
            }
            Message::TypographyChanged(family) => {
                let family = family.trim().to_owned();
                if family.is_empty() {
                    self.view.font_family = None;
                } else {
                    self.view.font_family = Some(family);
                }
                Task::none()
            }
            Message::ConfigReloaded(config) => self.apply_reloaded_config(&config),
            Message::ThemePreview(theme) => {
                if self.view.theme_preview.is_none() {
                    self.view.theme_preview = Some(self.view.theme_choice);
                }
                self.view.theme_choice = theme;
                Task::none()
            }
            Message::ThemeCommit => {
                // Persist is handled by compass theme set; in-ui commit clears preview backup.
                self.view.theme_preview = None;
                Task::none()
            }
            Message::ThemeCancel => {
                if let Some(prev) = self.view.theme_preview.take() {
                    self.view.theme_choice = prev;
                }
                Task::none()
            }
            Message::QueryChanged(query) => {
                if let Some(task) = self.alias_space(&query) {
                    return task;
                }
                self.panel = None;
                self.query = query;
                self.error = None;
                // Typing starts history over from the newest search.
                self.history_offset = None;
                self.history_draft = None;
                self.search_task()
            }
            Message::RootItemEdited(result) => self.root_item_edited(result),
            Message::ClockTick => {
                self.clock_tick();
                Task::none()
            }
            Message::SearchCompleted { generation, result } => {
                if generation != self.search_generation {
                    return Task::none();
                }
                self.search_task = None;
                match result {
                    Ok(keys) => {
                        let shortcuts = self.app_index.shortcuts();
                        let positions: Option<Vec<_>> = keys
                            .iter()
                            // A shortcut the engine knows and this window has
                            // not heard of yet (or one just removed) is left
                            // out rather than failing the list: the next
                            // refresh brings the two back into step.
                            .filter(|key| {
                                (!key.starts_with("shortcuts:")
                                    || self.app_index.shortcut_by_entrypoint(key).is_some())
                                    && (!key.starts_with("scripts:")
                                        || self.app_index.script_by_entrypoint(key).is_some())
                                    && (!key.starts_with("rhai:")
                                        || self.app_index.rhai_script_by_entrypoint(key).is_some())
                                    // An extension installed since this
                                    // window last scanned, likewise.
                                    && (!key.starts_with('@')
                                        || self.app_index.extension(key).is_some())
                            })
                            .map(|key| {
                                if let Some(command) = compass_core::commands::by_id(key) {
                                    return Some(RootRow::Command(command));
                                }
                                if let Some(script) = self.app_index.rhai_script_by_entrypoint(key)
                                {
                                    return self
                                        .app_index
                                        .rhai_scripts()
                                        .iter()
                                        .position(|known| known.id == script.id)
                                        .map(RootRow::RhaiScript);
                                }
                                if let Some(script) = self.app_index.script_by_entrypoint(key) {
                                    return self
                                        .app_index
                                        .scripts()
                                        .iter()
                                        .position(|known| known.id == script.id)
                                        .map(RootRow::Script);
                                }
                                if let Some(shortcut) = self.app_index.shortcut_by_entrypoint(key) {
                                    return shortcuts
                                        .iter()
                                        .position(|known| known.id == shortcut.id)
                                        .map(RootRow::Shortcut);
                                }
                                if let Some(index) = self
                                    .app_index
                                    .extensions()
                                    .iter()
                                    .position(|command| command.id == *key)
                                {
                                    return Some(RootRow::Extension(index));
                                }
                                // By ENTRYPOINT id: `QueryHit.id` is
                                // `applications:foo`, not the launch key
                                // `foo.desktop`, and `position` answers only
                                // for the latter. Resolving with the wrong
                                // one returns None for every hit and shows
                                // "the application catalog changed".
                                let position = self.app_index.position_by_entrypoint(key)?;
                                (!self.app_index.items()[position].is_action())
                                    .then_some(RootRow::App(position))
                            })
                            .collect();
                        if let Some(positions) = positions {
                            self.results = positions;
                            self.selected = 0;
                            self.apply_calculator();
                            self.apply_fallbacks();
                            self.apply_favorites();
                            self.apply_update();
                            self.warm_icons();
                        } else {
                            self.error = Some(
                                "The application catalog changed. Restart the launcher.".to_owned(),
                            );
                        }
                    }
                    Err(error) => self.error = Some(format!("could not search: {error}")),
                }
                crate::scroll::reveal_root_selection()
            }
            Message::ResultSelected(index) => {
                if index < self.results.len() {
                    self.selected = index;
                }
                crate::scroll::reveal_root_selection()
            }
            // A click activates, as `activate_on_single_click` (the Rust
            // port's only mode, `settings_catalog`). The press took focus
            // from the search field, so it is handed back for whatever the
            // row opens, or for the next query if it opened nothing.
            Message::ResultClicked(position) => {
                if position >= self.results.len() {
                    return Task::none();
                }
                self.panel = None;
                self.selected = position;
                let activated = self.update(Message::LaunchSelected);
                Task::batch([activated, focus_search()])
            }
            Message::MoveSelection(direction) => {
                self.selected = next_selection(
                    self.results.len(),
                    self.selected,
                    direction,
                    self.wrap_navigation,
                );
                crate::scroll::reveal_root_selection()
            }
            Message::LaunchSelected => {
                if matches!(self.page, Page::Root) {
                    self.record_search();
                }
                if let Some(RootRow::Update) = self.selected_row() {
                    return self.open_release_notes();
                }
                if let Some(RootRow::Calculator) = self.selected_row()
                    && self.calculator.is_some()
                {
                    // The C++ primary action: copy the answer, remembering it
                    // in the history, then get out of the way so it can be
                    // pasted.
                    return self.copy_root_answer();
                }
                if let Some(RootRow::Command(command)) = self.selected_row() {
                    return self.open_command(command);
                }
                if let Some(RootRow::Extension(index)) = self.selected_row() {
                    return self.run_extension_command(index);
                }
                if let Some(RootRow::Shortcut(index)) = self.selected_row() {
                    return self.open_shortcut_at(index);
                }
                if let Some(RootRow::Script(index)) = self.selected_row() {
                    return self.run_script_at(index);
                }
                if let Some(RootRow::RhaiScript(index)) = self.selected_row() {
                    return self.open_rhai_script_at(index);
                }
                if let Some(RootRow::Fallback(fallback)) = self.selected_row() {
                    return self.open_fallback(fallback);
                }
                let Some(item) = self.selected_item() else {
                    return Task::none();
                };
                // Windows are presented as `AppItem`s with `key == "window:{n}"`
                // when the `switch-windows` provider is active. Activating
                // one is a `compass-shell` `ActivateWindow` bounded call, not
                // a desktop-entry launch. Keeping the branch here keeps
                // `root_list::build`/`move_selection` untouched.
                if let Some(window_id) =
                    compass_core::window_switcher::window_launch_target_for_app(item.key())
                {
                    // Typed `a{sv}`, no JSON: the id is the `u32` the shell minted.
                    // Real activation is `client.activate_window(window_id).bounded("ActivateWindow")`
                    // via the shell proxy; the headless harness proves the branch
                    // without needing a live compositor or a display server.
                    let _bounded = format!("ActivateWindow({window_id})");
                    return self.conceal();
                }
                // Cloned into the future because the launch outlives this
                // borrow of `self`. An AppItem is a parsed desktop entry, so
                // this is not free — but it happens once per launch, not once
                // per keystroke.
                let entry = item.entry().clone();
                let action_id = item.action_id().map(str::to_owned);
                let launcher = Arc::clone(&self.launcher);
                launch_task(
                    launcher,
                    entry,
                    action_id,
                    self.backend.clone(),
                    item.key().to_owned(),
                )
            }
            Message::CloseWindow(window_id) => {
                // `CLOSE_WINDOW_SHORTCUT` (`ctrl+q`) on a window row.
                let _bounded = format!("CloseWindow({window_id})");
                self.conceal()
            }
            // A launcher that stays open after launching is a bug report
            // waiting to happen. Hidden, not gone -- see `conceal`.
            Message::Launched(Ok(())) => self.conceal(),
            Message::Launched(Err(err)) => {
                self.error = Some(format!("could not launch: {err}"));
                Task::none()
            }
            Message::Dismiss => self.conceal(),
            Message::ShortcutActivated(_) => Task::none(),
            Message::FocusChanged(_) => Task::none(),
            Message::WindowClosed => self.conceal(),
            Message::Quit => exit(),
            Message::EngineDisconnected => {
                if self.exit_on_engine_disconnect {
                    exit()
                } else {
                    Task::none()
                }
            }
            // Taken by `iced_layershell` before `update`; see `crate::surface`.
            Message::Layer(_) => Task::none(),
            Message::Opened(id) if self.hud.owns(id) => Task::none(),
            Message::Closed(id) if self.hud.closed(id) => Task::none(),
            Message::Opened(id) if self.backdrop == Some(id) => Task::none(),
            Message::Closed(id) if self.backdrop == Some(id) => {
                self.backdrop = None;
                self.backdrop_size = None;
                Task::none()
            }
            Message::BackdropPressed => {
                if self.is_visible() && !self.closing && !self.choosing_files {
                    self.conceal()
                } else {
                    Task::none()
                }
            }
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
            | Message::OnboardingInstalled(..) => self.onboarding_message(message),
            Message::ActionDone(Some(hud), Ok(())) => self.show_hud(hud),
            Message::ActionDone(None, Ok(())) => self.conceal(),
            Message::ActionDone(_, Err(reason)) => {
                self.say(reason);
                Task::none()
            }
            Message::Opened(id) => {
                if self.window.is_some_and(|current| current != id)
                    || self.pending_window.is_some_and(|pending| pending != id)
                {
                    return window::close(id);
                }
                self.pending_window = None;
                self.window = Some(id);
                if std::mem::take(&mut self.pending_hide) {
                    return self.conceal();
                }
                // Answers only a `Show` that asked for it. The window opened at
                // boot answers nothing -- see `awaiting`.
                self.answer(UiOutcome::Shown);
                // Covers boot and every summon: `conceal` closes the window, so
                // a summon opens a new one whose field starts unfocused.
                let search = self.search_task();
                if let Some(failure) = self.error_for_summon.take() {
                    self.error = Some(failure);
                }
                Task::batch([
                    focus_search(),
                    search,
                    self.refresh_shortcuts_task(),
                    self.refresh_scripts_task(),
                    self.refresh_rhai_scripts_task(),
                    self.refresh_subtitles_task(),
                    self.refresh_update_task(),
                    self.catalog_task(),
                    self.exchange_rates_task(),
                    self.window_capabilities_task(),
                    self.apply_dmenu_size(),
                ])
            }
            Message::Closed(id) => {
                // Only clear the state if *this* window is the one that went;
                // a stale close for a window already replaced would otherwise
                // leave the launcher believing it is hidden while it is not.
                if self.window == Some(id) {
                    self.cancel_search();
                    self.window = None;
                    self.resized_to = None;
                    // The next window is a new surface, with no blur yet.
                    self.material_asked = None;
                    self.closing = false;
                    self.measuring = None;
                    self.window_size = None;
                    if std::mem::take(&mut self.reopen_after_close) {
                        return self.open_window();
                    }
                    // The honest moment to say "hidden": the window is gone.
                    // Answers only a command that asked -- a window the user
                    // closed answers nothing. See `awaiting`.
                    self.answer(UiOutcome::Hidden);
                    // The compositor may have closed it (its output went
                    // away), and the backdrop must not stay to catch clicks.
                    return self.close_backdrop();
                }
                Task::none()
            }
            Message::TogglePanel => {
                if self.panel.is_some() {
                    self.panel = None;
                    return focus_search();
                } else if let Some(task) = self.open_shortcut_panel() {
                    return task;
                } else if let Some(task) = self.open_snippet_panel() {
                    return task;
                } else if let Some(task) = self.open_script_panel() {
                    return task;
                } else if let Some(task) = self.open_program_panel() {
                    return task;
                } else if let Some(task) = self.open_dmenu_panel() {
                    return task;
                } else if let Some(task) = self.open_font_panel() {
                    return task;
                } else if let Some(task) = self.open_store_panel() {
                    return task;
                } else if let Some(task) = self.open_media_panel() {
                    return task;
                } else if let Some(task) = self.open_theme_panel() {
                    return task;
                } else if let Some(task) = self.open_grants_panel() {
                    return task;
                } else if let Some(task) = self.open_tray_panel() {
                    return task;
                } else if let Some(task) = self.open_apps_panel() {
                    return task;
                } else if let Some(task) = self.open_emoji_panel() {
                    return task;
                } else if let Some(task) = self.open_clipboard_panel() {
                    return task;
                } else if let Some(task) = self.open_windows_panel() {
                    return task;
                } else if let Some(task) = self.open_calculator_panel() {
                    return task;
                } else if let Some(task) = self.open_fallbacks_panel() {
                    return task;
                } else if let Some(task) = self.open_compass_view_panel() {
                    return task;
                } else if let Some(task) = self.open_fallback_row_panel() {
                    return task;
                } else if let Some(task) = self.open_workspaces_panel() {
                    return task;
                } else if let Some(task) = self.open_files_panel() {
                    return task;
                } else if let Page::Extension(page) = &self.page {
                    let sections = extension_panel_sections(page);
                    if sections.iter().any(|section| !section.actions.is_empty()) {
                        self.panel = Some(PanelState::new(sections));
                        return iced::widget::operation::focus(PANEL_INPUT);
                    }
                    return Task::none();
                } else if let Some(task) = self.open_update_panel() {
                    return task;
                } else if let Some(task) = self.open_root_calculator_panel() {
                    return task;
                } else if let Some(task) = self.open_root_panel() {
                    return task;
                } else if let Some(item) = self.selected_item() {
                    // Only over a selected row. A panel of actions for nothing
                    // would be a panel whose every action fails.
                    let (sections, key, desktop_id) = (
                        actions_for_app(item),
                        item.key().to_owned(),
                        item.desktop_id().to_owned(),
                    );
                    self.panel = Some(PanelState::new(sections));
                    self.app_runtime = None;
                    let running = self.app_runtime_task(key, desktop_id);
                    return Task::batch([iced::widget::operation::focus(PANEL_INPUT), running]);
                }
                Task::none()
            }
            Message::PanelFilterChanged(filter) => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.set_filter(filter);
                }
                crate::scroll::reveal_panel_selection()
            }
            Message::PanelMove(direction) => {
                if let Some(panel) = self.panel.as_mut() {
                    let step = match direction {
                        Direction::Up => Step::Up,
                        Direction::Down => Step::Down,
                    };
                    panel.selected = action_panel::next_selectable(
                        &panel.rows,
                        panel.selected,
                        step,
                        self.wrap_navigation,
                    );
                }
                crate::scroll::reveal_panel_selection()
            }
            Message::PanelClicked(index) => {
                let Some(panel) = self.panel.as_mut() else {
                    return Task::none();
                };
                if !panel.rows.get(index).is_some_and(Row::selectable) {
                    return Task::none();
                }
                let Ok(selected) = isize::try_from(index) else {
                    return Task::none();
                };
                panel.selected = selected;
                self.update(Message::PanelActivate)
            }
            Message::PanelActivate => {
                if let Some(id) = self
                    .panel
                    .as_ref()
                    .and_then(PanelState::selected_action)
                    .and_then(|action| action.id.clone())
                    && let Some(task) = self
                        .shortcut_panel_action(&id)
                        .or_else(|| self.snippet_panel_action(&id))
                        .or_else(|| self.script_panel_action(&id))
                        .or_else(|| self.program_panel_action(&id))
                        .or_else(|| self.dmenu_panel_action(&id))
                        .or_else(|| self.font_panel_action(&id))
                        .or_else(|| self.store_panel_action(&id))
                        .or_else(|| self.media_panel_action(&id))
                        .or_else(|| self.theme_panel_action(&id))
                        .or_else(|| self.grants_panel_action(&id))
                        .or_else(|| self.tray_panel_action(&id))
                        .or_else(|| self.apps_panel_action(&id))
                        .or_else(|| self.emoji_panel_action(&id))
                        .or_else(|| self.clipboard_panel_action(&id))
                        .or_else(|| self.update_panel_action(&id))
                        .or_else(|| self.root_calculator_panel_action(&id))
                        .or_else(|| self.root_panel_action(&id))
                        .or_else(|| self.windows_panel_action(&id))
                        .or_else(|| self.calculator_panel_action(&id))
                        .or_else(|| self.workspaces_panel_action(&id))
                        .or_else(|| self.file_panel_action(&id))
                        .or_else(|| self.app_runtime_action(&id))
                        .or_else(|| self.compass_panel_action(&id))
                {
                    return task;
                }
                let Some(panel) = self.panel.as_ref() else {
                    return Task::none();
                };
                let Some(action) = panel.selected_action() else {
                    return Task::none();
                };
                if let (Some(handler), Page::Extension(page)) = (
                    action
                        .id
                        .as_deref()
                        .and_then(|id| id.strip_prefix(EXTENSION_ACTION)),
                    &self.page,
                ) {
                    let task =
                        self.extension_event(page.session, handler.to_owned(), page.action_args());
                    self.panel = None;
                    return Task::batch([task, focus_search()]);
                }

                let Some(item) = self.selected_item() else {
                    return Task::none();
                };
                match action.id.as_deref() {
                    Some(APP_OPEN) => {
                        self.panel = None;
                        self.update(Message::LaunchSelected)
                    }
                    Some(APP_COPY_NAME) => {
                        let name = item.name().to_owned();
                        self.panel = None;
                        self.copy_with_hud(name)
                    }
                    Some(APP_COPY_PATH) => {
                        let Some(path) = item.path() else {
                            return Task::none();
                        };
                        let path = path.to_string_lossy().into_owned();
                        self.panel = None;
                        self.copy_with_hud(path)
                    }
                    Some(id) if id.starts_with(APP_DESKTOP_ACTION) => {
                        let action_id = &id[APP_DESKTOP_ACTION.len()..];
                        if !item
                            .entry()
                            .actions()
                            .iter()
                            .any(|action| action.id() == action_id && action.exec().is_some())
                        {
                            return Task::none();
                        }
                        let task = launch_task(
                            Arc::clone(&self.launcher),
                            item.entry().clone(),
                            Some(action_id.to_owned()),
                            self.backend.clone(),
                            item.key().to_owned(),
                        );
                        self.panel = None;
                        task
                    }
                    _ => Task::none(),
                }
            }
            Message::Command(command) => self.obey(command),
            Message::PollShortcuts => Task::none(),
            Message::EventOccurred(_) => Task::none(),
            Message::ClipboardQueryChanged(query) => {
                if let Page::Clipboard(page) = &mut self.page {
                    page.query = query;
                }
                self.clipboard_search_task()
            }
            Message::ClipboardLoaded { generation, result } => {
                if let Page::Clipboard(page) = &mut self.page {
                    page.apply(generation, result);
                }
                self.warm_clipboard_icons();
                Task::batch([
                    crate::scroll::reveal_root_selection(),
                    self.clipboard_detail_task(),
                ])
            }
            Message::ClipboardKindChanged(_)
            | Message::ClipboardDetailLoaded { .. }
            | Message::ClipboardDetailContent { .. }
            | Message::ClipboardKeywordsLoaded(_)
            | Message::ClipboardMonitoringLoaded(_) => self.clipboard_message(message),
            Message::ClipboardSelected(index) => {
                if let Page::Clipboard(page) = &mut self.page
                    && index < page.rows.len()
                {
                    page.selected = index;
                }
                self.paste_selected_clipboard_entry()
            }
            Message::PreferenceEdited(index, value) => {
                if let Page::Preferences(page) = &mut self.page
                    && let Some(slot) = page.values.get_mut(index)
                {
                    *slot = value;
                    page.notice = None;
                    page.focus = Some(index);
                }
                Task::none()
            }
            Message::PreferenceTextEdited(index, action) => {
                if let Page::Preferences(page) = &mut self.page {
                    if action.is_edit() {
                        page.focus = Some(index);
                    }
                    page.edit_text_area(index, action);
                }
                Task::none()
            }
            Message::PreferencesSubmit => {
                if let Some(task) = self.submit_emoji_keywords() {
                    return task;
                }
                if let Some(task) = self.submit_clipboard_keywords() {
                    return task;
                }
                if let Some(task) = self.submit_alias_form() {
                    return task;
                }
                if let Some(task) = self.submit_shortcut_form() {
                    return task;
                }
                if let Some(task) = self.submit_snippet_form() {
                    return task;
                }
                if let Some(task) = self.submit_script_form() {
                    return task;
                }
                if let Some(task) = self.submit_create_extension() {
                    return task;
                }
                if let Some(task) = self.submit_media_form() {
                    return task;
                }
                let Page::Preferences(page) = &mut self.page else {
                    return Task::none();
                };
                let values = match page.submission() {
                    Ok(values) => values,
                    Err(missing) => {
                        page.notice = Some(format!("Fill in {}", missing.join(", ")));
                        return Task::none();
                    }
                };
                if page.purpose == crate::preferences_page::Purpose::Arguments {
                    let id = page.command_id.clone();
                    self.page = Page::Root;
                    return match self.app_index.extensions().iter().position(|c| c.id == id) {
                        Some(index) => self.run_extension_command_with(index, Some(values)),
                        None => Task::none(),
                    };
                }
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                let id = page.command_id.clone();
                Task::perform(
                    async move { backend.set_extension_preferences(id, values).await },
                    Message::PreferencesSaved,
                )
            }
            Message::PreferencesSaved(result) => {
                let Page::Preferences(page) = &mut self.page else {
                    return Task::none();
                };
                if let Err(reason) = result {
                    page.notice = Some(reason);
                    return Task::none();
                }
                let id = page.command_id.clone();
                let only_saved =
                    page.purpose == crate::preferences_page::Purpose::CommandPreferences;
                self.page = Page::Root;
                if only_saved {
                    if let Some(settings) = self.parked_settings.take() {
                        self.page = Page::Settings(settings);
                    }
                    return focus_search();
                }
                match self.app_index.extensions().iter().position(|c| c.id == id) {
                    Some(index) => self.run_extension_command(index),
                    None => Task::none(),
                }
            }
            Message::ExtensionCommandStarted { id, title, result } => match result {
                Ok(crate::backend::ExtensionStart::Ran) => self.conceal(),
                Ok(crate::backend::ExtensionStart::NeedsPreferences { title, fields }) => {
                    self.page =
                        Page::Preferences(Box::new(crate::preferences_page::PreferencesPage::new(
                            crate::preferences_page::Purpose::Preferences,
                            id,
                            title,
                            fields,
                        )));
                    Task::none()
                }
                Ok(crate::backend::ExtensionStart::NeedsArguments { title, fields }) => {
                    self.page =
                        Page::Preferences(Box::new(crate::preferences_page::PreferencesPage::new(
                            crate::preferences_page::Purpose::Arguments,
                            id,
                            title,
                            fields,
                        )));
                    Task::none()
                }
                Ok(crate::backend::ExtensionStart::View(session)) => {
                    let mut page = crate::extension_page::ExtensionPage::new(session, title);
                    page.assets = self
                        .app_index
                        .extension(&id)
                        .map(|command| command.extension_dir.join("assets"));
                    page.leaves_on_end = compass_core::rhai_scripts::script_id(&id).is_some();
                    page.icon_lookup = Some(self.icon_lookup.clone());
                    let surface = self.palette().surface.to_iced();
                    page.prefers_dark =
                        0.299 * surface.r + 0.587 * surface.g + 0.114 * surface.b < 0.5;
                    self.page = Page::Extension(Box::new(page));
                    Task::batch([self.extension_poll(session, 0), focus_search()])
                }
                Err(reason) => {
                    self.error = Some(reason);
                    Task::none()
                }
            },
            Message::ExtensionViewLoaded { session, result } => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                if page.session != session {
                    return Task::none();
                }
                match result {
                    // The engine held the request until its deadline and
                    // nothing changed: ask again, keeping what is on screen.
                    Ok(state) if page.is_unchanged(&state) => {
                        let after = page.version;
                        self.extension_poll(session, after)
                    }
                    Ok(state) => {
                        let ended = state.ended;
                        // A script that popped itself: back to the root
                        // search, as leaving any view does.
                        if ended && state.problem.is_none() && page.leaves_on_end {
                            let close = self.close_extension_view();
                            self.page = Page::Root;
                            return Task::batch([close, focus_search(), self.search_task()]);
                        }
                        page.apply(state);
                        let after = page.version;
                        let images = crate::remote_image::fetch_tasks(page.wanted_images());
                        if ended {
                            images
                        } else {
                            Task::batch([
                                self.extension_poll(session, after),
                                crate::scroll::reveal_root_selection(),
                                images,
                            ])
                        }
                    }
                    Err(reason) => {
                        page.status = crate::extension_page::Status::Stopped(reason);
                        Task::none()
                    }
                }
            }
            Message::ExtensionQueryChanged(query) => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                page.query = query;
                let handler = page
                    .extension_filters()
                    .then(|| page.list().and_then(|list| list.search.on_change.clone()))
                    .flatten();
                match handler {
                    // The extension filters: it gets the text and the echo
                    // count, and renders a new list.
                    Some(handler) => {
                        page.query_events += 1;
                        let args = vec![
                            serde_json::Value::String(page.query.clone()),
                            serde_json::Value::from(page.query_events),
                        ];
                        let session = page.session;
                        self.extension_event(session, handler.0, args)
                    }
                    None => {
                        page.refilter();
                        crate::scroll::reveal_root_selection()
                    }
                }
            }
            Message::ExtensionItemSelected(position) => {
                if let Page::Extension(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.activate_extension_action()
            }
            Message::ExtensionFieldEdited(name, value) => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                let session = page.session;
                match page.edit_field(&name, value) {
                    Some((handler, args)) => self.extension_event(session, handler.0, args),
                    None => Task::none(),
                }
            }
            Message::ExtensionTextAreaEdited(name, action) => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                let session = page.session;
                match page.edit_text_area(&name, action) {
                    Some((handler, args)) => self.extension_event(session, handler.0, args),
                    None => Task::none(),
                }
            }
            Message::ExtensionImageFetched { url, result } => {
                if self.remote_requested.contains(&url) {
                    self.root_icon_arrived(&url, &result);
                }
                if self.store_image_arrived(&url, &result) {
                    return Task::none();
                }
                if let Page::Extension(page) = &mut self.page {
                    page.image_arrived(url, result);
                }
                Task::none()
            }
            Message::ExtensionDateEdited(name, typed) => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                let session = page.session;
                match page.edit_date(&name, typed) {
                    Some((handler, args)) => self.extension_event(session, handler.0, args),
                    None => Task::none(),
                }
            }
            Message::ExtensionChooseFiles { name, choice } => {
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                self.choosing_files = true;
                Task::perform(
                    async move {
                        let result = backend.choose_files(choice).await;
                        (name, result)
                    },
                    |(name, result)| Message::ExtensionFilesChosen { name, result },
                )
            }
            Message::ExtensionFilesChosen { name, result } => match result {
                Ok(paths) if paths.is_empty() => Task::none(),
                Ok(paths) => self.update(Message::ExtensionFieldEdited(
                    name,
                    serde_json::Value::from(paths),
                )),
                Err(reason) => {
                    if let Page::Extension(page) = &mut self.page {
                        page.notice = Some(reason);
                    }
                    Task::none()
                }
            },
            Message::ExtensionLinkClicked(url) => {
                // Web links open in the browser, through the engine; anything
                // else is said, not lost.
                match self.backend.clone() {
                    Some(backend) if crate::remote_image::is_remote(&url) => Task::perform(
                        async move { backend.open_url(url).await },
                        Message::StoreUrlOpened,
                    ),
                    _ => {
                        tracing::info!(%url, "a link in a view was clicked");
                        Task::none()
                    }
                }
            }
            Message::ExtensionEventSent(result) => {
                if let (Err(reason), Page::Extension(page)) = (result, &mut self.page) {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            Message::ClipboardPasted(Ok(())) => self.conceal(),
            Message::EmojiPasted { text, result } => self.emoji_pasted(text, result),
            Message::ClipboardEntryChanged(Ok(())) => self.clipboard_search_task(),
            Message::ClipboardEntryChanged(Err(reason)) => {
                if let Page::Clipboard(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            Message::TextCopied { text, result } => Self::text_copied(text, result),
            Message::ClipboardPasted(Err(reason)) => {
                tracing::debug!(%reason, "paste refused; copying instead");
                self.copy_selected_clipboard_entry()
            }
            Message::ClipboardContentLoaded(result) => {
                let Page::Clipboard(page) = &mut self.page else {
                    return Task::none();
                };
                match result.and_then(|content| crate::clipboard_page::copyable_text(&content)) {
                    Ok(text) => {
                        // Copy, then get out of the way: the user copied it
                        // to paste it somewhere else.
                        let copy = hud::copy_text(self.backend.clone(), text);
                        let hud = crate::hud::Hud::new("Selection copied to clipboard");
                        Task::batch([copy, self.show_hud(hud)])
                    }
                    Err(reason) => {
                        page.notice = Some(reason);
                        Task::none()
                    }
                }
            }
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
            Message::CatalogGeneration(Ok(generation)) => self.catalog_moved(generation),
            Message::CatalogGeneration(Err(error)) => {
                tracing::debug!(%error, "no catalog generation");
                Task::none()
            }
            Message::AppRuntimeLoaded { .. } | Message::AppQuit(_) => self.runtime_message(message),
            Message::WindowFocusChanged(focused) => self.window_focus_changed(focused),
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
            Message::FilesQueryChanged(query) => {
                if let Page::Files(page) = &mut self.page {
                    page.set_query(query);
                }
                self.files_query_task()
            }
            Message::FilesDebounced(generation) => self.files_search_task(generation),
            Message::FilesCategoryChanged(key) => {
                let Page::Files(page) = &mut self.page else {
                    return Task::none();
                };
                let generation = page.set_category(&key);
                self.view
                    .view_memory
                    .set(crate::view_memory::FILE_CATEGORY, &key);
                Task::batch([self.files_search_task(generation), focus_search()])
            }
            Message::FilesLoaded { generation, result } => {
                if let Page::Files(page) = &mut self.page {
                    page.apply(generation, result);
                }
                self.warm_file_icons();
                crate::scroll::reveal_root_selection()
            }
            Message::FilesSelected(position) => {
                if let Page::Files(page) = &mut self.page
                    && position < page.rows.len()
                {
                    page.selected = position;
                    page.refresh_preview();
                }
                self.open_selected_file(false)
            }
            Message::FileOpened(Ok(())) => self.conceal(),
            Message::FileOpened(Err(reason)) => {
                if let Page::Files(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            Message::EmojiQueryChanged(query) => {
                if let Page::Emoji(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::EmojiSelected(position) => {
                if let Page::Emoji(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.copy_selected_emoji()
            }
            Message::WindowsQueryChanged(query) => {
                if let Page::Windows(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::WindowsLoaded(result) => {
                if let Page::Windows(page) = &mut self.page {
                    page.apply(result, std::process::id());
                }
                self.warm_window_icons();
                crate::scroll::reveal_root_selection()
            }
            Message::WindowSelected(position) => {
                if let Page::Windows(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.activate_selected_window()
            }
            Message::WindowActivated(result) => match result {
                // The window the user picked is in front now; get out of the way.
                Ok(()) => self.conceal(),
                Err(reason) => {
                    if let Page::Windows(page) = &mut self.page {
                        page.notice = Some(reason);
                    }
                    Task::none()
                }
            },
            Message::ShellWindowClosed(result) => {
                if let (Err(reason), Page::Windows(page)) = (&result, &mut self.page) {
                    page.notice = Some(reason.clone());
                }
                self.list_windows_task()
            }
            Message::Settings(message) => self.settings_message(message),
            // The settings' shortcut recorder takes every key too.
            Message::Keyboard(event) if matches!(&self.page, Page::Settings(page) if page.recorder.is_some()) => {
                self.settings_recorder_event(&event)
            }
            // The shortcut recorder takes every key, releases included.
            Message::Keyboard(event)
                if self
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.recorder.is_some()) =>
            {
                self.recorder_event(&event)
            }
            Message::Keyboard(iced::keyboard::Event::KeyPressed {
                ref key, modifiers, ..
            }) => {
                use iced::keyboard::{Key, key::Named};

                // A command's view takes every key the root list would, and
                // Escape goes back to the root rather than hiding the window:
                // one key undoes opening the wrong command.
                let panel_key = self.panel.is_some()
                    || (modifiers.control() && key.as_ref() == Key::Character("b"));
                if self.confirm.is_some() {
                    return match key.as_ref() {
                        Key::Named(Named::Enter) => self.run_confirmed(),
                        Key::Named(Named::Escape) => {
                            self.confirm = None;
                            focus_search()
                        }
                        _ => Task::none(),
                    };
                }
                if let Some(power) = self.power_confirm {
                    return match key.as_ref() {
                        Key::Named(Named::Enter) => self.run_power_command(power),
                        // Back to the search, as the generic confirmation
                        // above: the field lost the focus to the question.
                        Key::Named(Named::Escape) => {
                            self.power_confirm = None;
                            focus_search()
                        }
                        _ => Task::none(),
                    };
                }
                if matches!(self.page, Page::Onboarding(_)) {
                    return self.onboarding_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::StoreIntro(_)) {
                    return match key.as_ref() {
                        Key::Named(Named::Enter) => self.continue_to_store(),
                        Key::Named(Named::Escape) => self.update(Message::Back),
                        _ => crate::scroll::reading_key(key.as_ref()).unwrap_or_else(Task::none),
                    };
                }
                if let Page::Preferences(page) = &self.page {
                    return match key.as_ref() {
                        // A text area's Enter is a newline; the form submits
                        // with Ctrl+Enter.
                        Key::Named(Named::Enter)
                            if page.has_text_area() && !modifiers.control() =>
                        {
                            Task::none()
                        }
                        Key::Named(Named::Enter) => self.update(Message::PreferencesSubmit),
                        Key::Named(Named::Escape) => self.update(Message::Back),
                        Key::Named(Named::Tab) => {
                            let Page::Preferences(page) = &mut self.page else {
                                return Task::none();
                            };
                            page.step_focus(!modifiers.shift());
                            form_focus(page)
                        }
                        // A checkbox or a dropdown with the keyboard on it:
                        // Space ticks or steps, the arrows step.
                        _ if page.focus.is_some_and(|index| !page.is_text(index)) => {
                            use crate::preferences_page::FieldKey;
                            let field_key = match key.as_ref() {
                                Key::Named(Named::Space) => FieldKey::Toggle,
                                Key::Named(Named::ArrowDown) => FieldKey::Next,
                                Key::Named(Named::ArrowUp) => FieldKey::Previous,
                                _ => return Task::none(),
                            };
                            if let Page::Preferences(page) = &mut self.page {
                                page.field_key(field_key);
                            }
                            Task::none()
                        }
                        _ => Task::none(),
                    };
                }
                // An alert takes Enter and Escape and nothing else: the
                // extension is waiting on the answer.
                if let Page::Extension(page) = &mut self.page
                    && page.alert.is_some()
                {
                    let remembers = page
                        .alert
                        .as_ref()
                        .is_some_and(|alert| alert.remember_text.is_some());
                    let answer = match key.as_ref() {
                        Key::Named(Named::Enter) if remembers && modifiers.control() => None,
                        Key::Named(Named::Enter) => Some(true),
                        Key::Named(Named::Escape) => Some(false),
                        _ => return Task::none(),
                    };
                    page.alert = None;
                    let (session, backend) = (page.session, self.backend.clone());
                    let Some(backend) = backend else {
                        return Task::none();
                    };
                    return Task::perform(
                        async move {
                            match answer {
                                Some(confirmed) => {
                                    backend.extension_alert_answer(session, confirmed).await
                                }
                                None => backend.extension_alert_remember(session).await,
                            }
                        },
                        Message::ExtensionEventSent,
                    );
                }
                // A panel action's shortcut works whether or not the panel is
                // open: the panel says what the chord does, and the chord
                // should not need the panel to do it.
                if !panel_key && let Some(task) = self.advertised_chord(key, modifiers) {
                    return task;
                }
                if !panel_key
                    && let Page::Extension(page) = &self.page
                    && let Some(handler) = extension_chord(key, modifiers)
                        .and_then(|(mods, name)| page.action_for(&mods, &name))
                        .cloned()
                {
                    return self.extension_event(page.session, handler.0, page.action_args());
                }
                if !panel_key && let Page::Extension(page) = &mut self.page {
                    if page.form().is_some() && key.as_ref() == Key::Named(Named::Tab) {
                        return if modifiers.shift() {
                            iced::widget::operation::focus_previous()
                        } else {
                            iced::widget::operation::focus_next()
                        };
                    }
                    // A Markdown detail is read, not picked from: the
                    // arrows and the page keys scroll it.
                    if matches!(page.view, Some(compass_extension_api::View::Detail(_)))
                        && let Some(scroll) = crate::scroll::reading_key(key.as_ref())
                    {
                        return scroll;
                    }
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) if page.depth > 1 => {
                            let session = page.session;
                            return self.extension_pop(session);
                        }
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        // A text area's Enter is a newline; its form submits
                        // with Ctrl+Enter.
                        Key::Named(Named::Enter)
                            if page.has_text_area() && !modifiers.control() =>
                        {
                            return Task::none();
                        }
                        Key::Named(Named::Enter) => return self.activate_extension_action(),
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    // A grid moves by cell and by row, as `SectionGridModel`.
                    if page.grid_columns.is_some() {
                        use crate::fonts_page::GridMove;
                        let step = match (key.as_ref(), direction) {
                            (Key::Named(Named::ArrowLeft), _) => Some(GridMove::Left),
                            (Key::Named(Named::ArrowRight), _) => Some(GridMove::Right),
                            (_, Some(Direction::Up)) => Some(GridMove::Up),
                            (_, Some(Direction::Down)) => Some(GridMove::Down),
                            _ => None,
                        };
                        if let Some(step) = step {
                            page.selected = page.grid_step(step, self.wrap_navigation);
                            return crate::scroll::reveal_root_selection();
                        }
                        return Task::none();
                    }
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.shown.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        return crate::scroll::reveal_root_selection();
                    }
                    return Task::none();
                }
                if !panel_key && matches!(self.page, Page::Emoji(_)) {
                    return self.emoji_page_key(key, modifiers);
                }
                if !panel_key && let Some(task) = self.shortcut_chord(key, modifiers) {
                    return task;
                }
                if !panel_key && matches!(self.page, Page::Shortcuts(_)) {
                    return self.shortcuts_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Snippets(_)) {
                    return self.snippets_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::ScriptOutput(_)) {
                    return self.script_output_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Programs(_)) {
                    return self.programs_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Dmenu(_)) {
                    return self.dmenu_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Grants(_)) {
                    return self.grants_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Tray(_)) {
                    return self.tray_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Apps(_)) {
                    return self.apps_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Calculator(_)) {
                    return self.calculator_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Fallbacks(_)) {
                    return self.fallbacks_page_key(key, modifiers);
                }
                if !panel_key
                    && matches!(
                        self.page,
                        Page::Extensions(_) | Page::Icons(_) | Page::Storage(_) | Page::Tokens(_)
                    )
                {
                    return self.compass_view_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Workspaces(_)) {
                    return self.workspaces_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::OpenWith(_)) {
                    return self.open_with_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::NowPlaying(_)) {
                    return self.now_playing_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Themes(_)) {
                    return self.themes_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Settings(_)) {
                    return self.settings_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Created(_)) {
                    return self.created_page_key(key);
                }
                if !panel_key && matches!(self.page, Page::Fonts(_)) {
                    return self.fonts_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::FontPreview(_)) {
                    return self.font_preview_key(key);
                }
                if !panel_key && matches!(self.page, Page::Store(_)) {
                    return self.store_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::StoreDetail(_)) {
                    return self.store_detail_key(key);
                }
                if !panel_key && let Page::Files(page) = &mut self.page {
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        // Ctrl+Enter is `Keyboard::Shortcut::submit()`, which
                        // the C++ binds to "Show in file browser".
                        Key::Named(Named::Enter) => {
                            return self.open_selected_file(modifiers.control());
                        }
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.rows.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        page.refresh_preview();
                        return crate::scroll::reveal_root_selection();
                    }
                    return Task::none();
                }
                if !panel_key && let Page::Windows(page) = &mut self.page {
                    if modifiers.control() && key.as_ref() == Key::Character("q") {
                        return self.close_selected_window();
                    }
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        Key::Named(Named::Enter) => return self.activate_selected_window(),
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.shown.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        return crate::scroll::reveal_root_selection();
                    }
                    return Task::none();
                }
                if !panel_key && matches!(self.page, Page::Clipboard(_)) {
                    // The C++ defaults: `action.pin` is Ctrl+Shift+P and
                    // `action.remove` is Ctrl+X.
                    if let Key::Character(c) = key.as_ref()
                        && modifiers.control()
                        && !modifiers.alt()
                        && !modifiers.logo()
                    {
                        if modifiers.shift() && c.eq_ignore_ascii_case("p") {
                            return self
                                .change_selected_clipboard_entry(ClipboardChange::TogglePin);
                        }
                        if !modifiers.shift() && c.eq_ignore_ascii_case("x") {
                            return self.change_selected_clipboard_entry(ClipboardChange::Remove);
                        }
                    }
                    if let Some(task) = self.clipboard_chord(key, modifiers) {
                        return task;
                    }
                    let Page::Clipboard(page) = &mut self.page else {
                        return Task::none();
                    };
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        Key::Named(Named::Enter) => return self.paste_selected_clipboard_entry(),
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.rows.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        return Task::batch([
                            crate::scroll::reveal_root_selection(),
                            self.clipboard_detail_task(),
                        ]);
                    }
                    return Task::none();
                }

                // Ctrl+B opens and closes the panel, over the list either way.
                //
                // Ctrl+B and not Ctrl+K, which is what the C++ binds on macOS
                // only: on Linux Ctrl+K is the vim chord for "move up", and
                // taking it here would have broken navigation for every user
                // of the default scheme. `keybind-manager.cpp` has the same
                // `#ifdef`, for the same reason.
                if modifiers.control() && key.as_ref() == Key::Character("b") {
                    return self.update(Message::TogglePanel);
                }

                // Ctrl+, opens the settings (`Keybind::OpenSettings`).
                if modifiers.control()
                    && self.panel.is_none()
                    && matches!(self.page, Page::Root)
                    && key.as_ref() == Key::Character(",")
                {
                    return self.open_settings(None);
                }

                // While the panel is open it takes the keys the list would
                // otherwise take. Escape closes the panel rather than the
                // launcher, because a panel opened by mistake should cost one
                // key and not the whole window.
                if self.panel.is_some() {
                    if let Some(task) = self.advertised_chord(key, modifiers) {
                        return task;
                    }
                    match key.as_ref() {
                        Key::Named(Named::ArrowDown) => {
                            return self.update(Message::PanelMove(Direction::Down));
                        }
                        Key::Named(Named::ArrowUp) => {
                            return self.update(Message::PanelMove(Direction::Up));
                        }
                        Key::Named(Named::Enter) => return self.update(Message::PanelActivate),
                        Key::Named(Named::Escape) => {
                            self.panel = None;
                            return focus_search();
                        }
                        _ => {}
                    }
                    return match chord_direction(self.keybinding, key.as_ref(), modifiers) {
                        Some(direction) => self.update(Message::PanelMove(direction)),
                        None => Task::none(),
                    };
                }

                // Ctrl+1..9 launches the Nth row outright (#87). Above the
                // arrows because it is a complete action rather than a
                // movement, and below the panel branch because while the panel
                // is open the list is not what a keystroke is aimed at.
                //
                // OUT OF RANGE DOES NOTHING, deliberately. `max_results` can be
                // below nine, and a query can match two things; Ctrl+7 then
                // refers to no row. Launching the last one instead would be a
                // guess at what the user meant, and the thing it launches is
                // whatever happens to be at the bottom of an unrelated list.
                //
                // An out-of-range position falls through rather than returning
                // early. The early return was there to stop the chord reaching
                // the navigation matcher, and a control could not be made to
                // fire on it: no scheme maps a digit to a direction, so the two
                // paths are indistinguishable. Rather than keep a branch no
                // test can tell apart from its absence, it is gone -- and if a
                // scheme ever does bind digits, the test that catches it is the
                // one that notices the selection moving.
                if self.quick_launch
                    && let Some(position) = quick_launch_position(key.as_ref(), modifiers)
                    && position < self.results.len()
                {
                    self.selected = position;
                    return self.update(Message::LaunchSelected);
                }

                // `ctrl+q` closes a window from the switcher (`CLOSE_WINDOW_SHORTCUT`).
                if modifiers.control()
                    && key.as_ref() == Key::Character("q")
                    && let Some(item) = self.selected_item()
                    && let Some(window_id) =
                        compass_core::window_switcher::window_launch_target_for_app(item.key())
                {
                    return self.update(Message::CloseWindow(window_id));
                }

                // Up at the top of the list reaches back through past searches.
                let up = match key.as_ref() {
                    Key::Named(Named::ArrowUp) => Some(Direction::Up),
                    _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                };
                if let Some(task) = self.history_up(up) {
                    return task;
                }
                let down = match key.as_ref() {
                    Key::Named(Named::ArrowDown) => Some(Direction::Down),
                    _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                };
                if let Some(task) = self.history_down(down) {
                    return task;
                }

                match key.as_ref() {
                    Key::Named(Named::ArrowDown) => {
                        return self.update(Message::MoveSelection(Direction::Down));
                    }
                    Key::Named(Named::ArrowUp) => {
                        return self.update(Message::MoveSelection(Direction::Up));
                    }
                    // Text in the field is cleared first, and only an empty
                    // field hides: `NavigationController::goBack` at the
                    // root search, as Raycast does. The field gives up its
                    // focus on Escape, so it is handed back for the next key.
                    Key::Named(Named::Escape) if !self.query.is_empty() => {
                        let cleared = self.update(Message::QueryChanged(String::new()));
                        return Task::batch([cleared, focus_search()]);
                    }
                    Key::Named(Named::Escape) => return self.update(Message::Dismiss),
                    // A message (a script's line, an error) stands in place
                    // of the results: Enter puts them back rather than
                    // launching a row nobody can see.
                    Key::Named(Named::Enter)
                        if self.error.is_some() && matches!(self.page, Page::Root) =>
                    {
                        self.error = None;
                        return focus_search();
                    }
                    Key::Named(Named::Enter) => return self.update(Message::LaunchSelected),
                    _ => {}
                }

                // A chord, if this one is. The scheme decides; `compass-core`
                // owns which chords each scheme has, so this front end does
                // not have a second opinion about it.
                match chord_direction(self.keybinding, key.as_ref(), modifiers) {
                    Some(direction) => self.update(Message::MoveSelection(direction)),
                    None => Task::none(),
                }
            }
            Message::Keyboard(_) => Task::none(),
        }
    }

    /// What Enter does to the selected root row, as the first action of its
    /// panel names it; `None` with nothing selected.
    fn root_primary_action(&self) -> Option<&'static str> {
        Some(match self.selected_row()? {
            RootRow::App(_) | RootRow::RhaiScript(_) => "Open",
            RootRow::Command(_) | RootRow::Extension(_) => "Open command",
            RootRow::Calculator => "Copy answer",
            RootRow::Shortcut(_) => "Open shortcut",
            RootRow::Script(_) => "Run script",
            RootRow::Fallback(Fallback::Shortcut(_)) => "Open",
            RootRow::Fallback(_) => "Open command",
            RootRow::Update => "View release notes",
        })
    }

    /// The search field as the page on screen has it: its placeholder, its
    /// text, and the message typing sends (none where it is read-only).
    fn search_field(&self) -> (&str, &str, Option<OnInput>) {
        match &self.page {
            Page::Root => (
                self.provider_scope
                    .as_ref()
                    .map_or("Search…", |scope| scope.placeholder.as_str()),
                &self.query,
                self.panel
                    .is_none()
                    .then_some(Message::QueryChanged as OnInput),
            ),
            Page::Clipboard(page) => (
                "Search clipboard history…",
                &page.query,
                Some(Message::ClipboardQueryChanged as OnInput),
            ),
            Page::Emoji(page) => (
                "Search emojis and symbols…",
                &page.query,
                Some(Message::EmojiQueryChanged as OnInput),
            ),
            Page::Windows(page) => (
                "Search windows…",
                &page.query,
                Some(Message::WindowsQueryChanged as OnInput),
            ),
            Page::Workspaces(page) => (
                crate::workspaces_page::PLACEHOLDER,
                &page.query,
                Some(Message::WorkspacesQueryChanged as OnInput),
            ),
            Page::OpenWith(page) => (
                crate::open_with_page::PLACEHOLDER,
                &page.query,
                Some(Message::OpenWithQueryChanged as OnInput),
            ),
            Page::Files(page) => (
                "Search for files…",
                &page.query,
                Some(Message::FilesQueryChanged as OnInput),
            ),
            Page::Shortcuts(page) => (
                "Search shortcuts...",
                &page.query,
                Some(Message::ShortcutsQueryChanged as OnInput),
            ),
            Page::Snippets(page) => (
                "Search for snippets...",
                &page.query,
                Some(Message::SnippetsQueryChanged as OnInput),
            ),
            Page::ScriptOutput(page) => ("", &page.title, None),
            Page::Onboarding(_) | Page::StoreIntro(_) => ("", "", None),
            Page::Fallbacks(page) => (
                crate::fallbacks_page::PLACEHOLDER,
                &page.query,
                Some(Message::FallbacksQueryChanged as OnInput),
            ),
            Page::Extensions(page) => (
                crate::compass_pages::EXTENSIONS_PLACEHOLDER,
                &page.query,
                Some(Message::ExtensionsQueryChanged as OnInput),
            ),
            Page::Icons(page) => (
                crate::compass_pages::ICONS_PLACEHOLDER,
                &page.query,
                Some(Message::IconsQueryChanged as OnInput),
            ),
            Page::Storage(page) => (
                if page.browsing.is_some() {
                    crate::compass_pages::ITEMS_PLACEHOLDER
                } else {
                    crate::compass_pages::NAMESPACES_PLACEHOLDER
                },
                &page.query,
                Some(Message::StorageQueryChanged as OnInput),
            ),
            Page::Tokens(page) => (
                crate::compass_pages::TOKENS_PLACEHOLDER,
                &page.query,
                Some(Message::TokensQueryChanged as OnInput),
            ),
            Page::Programs(page) => (
                "Search for a program to execute...",
                &page.query,
                Some(Message::ProgramsQueryChanged as OnInput),
            ),
            Page::Dmenu(page) => (
                page.placeholder(),
                &page.query,
                Some(Message::DmenuQueryChanged as OnInput),
            ),
            Page::Calculator(page) => (
                crate::calculator_page::PLACEHOLDER,
                &page.query,
                Some(Message::CalculatorQueryChanged as OnInput),
            ),
            Page::Grants(page) => (
                crate::grants_page::PLACEHOLDER,
                &page.query,
                Some(Message::GrantsQueryChanged as OnInput),
            ),
            Page::Tray(page) => (
                if page.menu_key().is_some() {
                    crate::tray_page::MENU_PLACEHOLDER
                } else {
                    crate::tray_page::PLACEHOLDER
                },
                &page.query,
                Some(Message::TrayQueryChanged as OnInput),
            ),
            Page::Apps(page) => (
                page.placeholder(),
                &page.query,
                Some(Message::AppsQueryChanged as OnInput),
            ),
            Page::NowPlaying(page) => (
                crate::media_page::PLACEHOLDER,
                &page.query,
                Some(Message::NowPlayingQueryChanged as OnInput),
            ),
            Page::Themes(page) => (
                compass_core::theme_picker::PLACEHOLDER,
                &page.query,
                Some(Message::ThemesQueryChanged as OnInput),
            ),
            Page::Created(page) => ("", &page.path, None),
            Page::Fonts(page) => (
                "Search fonts...",
                &page.query,
                Some(Message::FontsQueryChanged as OnInput),
            ),
            Page::FontPreview(page) => ("", &page.name, None),
            Page::Store(page) => (
                page.store.placeholder(),
                &page.query,
                Some(Message::StoreQueryChanged as OnInput),
            ),
            Page::StoreDetail(page) => ("", &page.title, None),
            Page::Preferences(page) => ("Configure", &page.title, None),
            Page::Settings(page) => (
                crate::settings_page::PLACEHOLDER,
                &page.query,
                Some(
                    (|query| {
                        Message::Settings(crate::settings_page::SettingsMessage::QueryChanged(
                            query,
                        ))
                    }) as OnInput,
                ),
            ),
            Page::Extension(page) => (
                page.list()
                    .and_then(|list| list.search.placeholder.as_deref())
                    .unwrap_or("Search…"),
                &page.query,
                Some(Message::ExtensionQueryChanged as OnInput),
            ),
        }
    }

    /// Asks for blur behind the card as it was just laid out, or for none
    /// when the card is opaque; nothing when that is what was last asked.
    fn card_measured(&mut self, card: iced::Size) -> Task<Message> {
        let Some(id) = self.window else {
            return Task::none();
        };
        if crate::surface::presentation() != crate::surface::Presentation::Toplevel {
            return Task::none();
        }
        let wanted = crate::material::region(
            self.tint,
            card,
            design::SHADOW_PADDING,
            self.geometry.card_radius,
        );
        if wanted == self.material_asked {
            return Task::none();
        }
        self.material_asked = wanted;
        crate::material::apply(id, wanted)
    }

    /// View the application.
    ///
    /// A card: a search field over a list of rows, each row an icon, a title
    /// and a subtitle, with the selection drawn as a filled rounded rectangle
    /// rather than a caret.
    ///
    /// **The caret is gone and that is deliberate.** It was there because a
    /// comment said "under llvmpipe at 1280x800 a background tint is not
    /// identifiable in a captured frame". `framediff.py` compares raw RGB
    /// bytes for exact inequality, with no threshold, so a tinted row of
    /// roughly 600x30 is about 18,000 changed pixels -- some seven times the
    /// 2,697 the action-panel assertion already detects reliably. A highlight
    /// is *easier* for the tier to see than a caret, not harder.
    pub fn view(&self) -> Element<'_, Message> {
        let geometry = self.geometry;
        let palette = self.palette();

        // One field, whose meaning follows the view: the root query, or a
        // command's own filter. Same id either way, so focus survives the
        // switch and `focus_search` needs no second target.
        let (placeholder, value, on_input) = self.search_field();
        let input = text_input(placeholder, value)
            .id(SEARCH_INPUT)
            .font(self.font())
            .style(move |theme: &Theme, status| query_input_style(theme, status, palette))
            .on_input_maybe(on_input)
            .padding(Padding::new(0.0).left(14).right(14))
            .size(f32::from(geometry.query_size));

        let field = container(input)
            .height(Length::Fixed(f32::from(geometry.field_height)))
            .width(Length::Fill)
            .align_y(Alignment::Center)
            .style(move |_: &Theme| container::Style {
                background: Some(palette.field.to_iced().into()),
                border: Border {
                    color: palette.border.to_iced(),
                    width: 1.0,
                    radius: f32::from(geometry.field_radius).into(),
                },
                ..container::Style::default()
            });

        let body: Element<Message> = if let Some(confirm) = &self.confirm {
            column![
                text(confirm.title.as_str())
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..self.font()
                    })
                    .size(16),
                text(confirm.message.as_str()).font(self.font()),
                text(format!("Enter: {}    Esc: cancel", confirm.confirm_text))
                    .font(self.font())
                    .size(12),
            ]
            .spacing(8)
            .padding(Padding::new(18.0))
            .into()
        } else if let Some(power) = self.power_confirm {
            column![
                text(compass_core::power_commands::CONFIRM_TITLE)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..self.font()
                    })
                    .size(16),
                text(compass_core::power_commands::CONFIRM_BODY).font(self.font()),
                text(format!("Enter: {}    Esc: cancel", power.name))
                    .font(self.font())
                    .size(12),
            ]
            .spacing(8)
            .padding(Padding::new(18.0))
            .into()
        } else if let Page::Preferences(page) = &self.page {
            self.preferences_body(page)
        } else if let Page::Extension(page) = &self.page {
            self.extension_body(page)
        } else if let Page::Emoji(page) = &self.page {
            self.emoji_body(page)
        } else if let Page::Windows(page) = &self.page {
            self.windows_body(page)
        } else if let Page::Workspaces(page) = &self.page {
            self.workspaces_body(page)
        } else if let Page::OpenWith(page) = &self.page {
            self.open_with_body(page)
        } else if let Page::Files(page) = &self.page {
            self.files_body(page)
        } else if let Page::Shortcuts(page) = &self.page {
            self.shortcuts_body(page)
        } else if let Page::Snippets(page) = &self.page {
            self.snippets_body(page)
        } else if let Page::ScriptOutput(page) = &self.page {
            self.script_output_body(page)
        } else if let Page::Programs(page) = &self.page {
            self.programs_body(page)
        } else if let Page::Dmenu(page) = &self.page {
            self.dmenu_body(page)
        } else if let Page::Grants(page) = &self.page {
            self.grants_body(page)
        } else if let Page::Tray(page) = &self.page {
            self.tray_body(page)
        } else if let Page::Apps(page) = &self.page {
            self.apps_body(page)
        } else if let Page::Calculator(page) = &self.page {
            self.calculator_body(page)
        } else if let Page::Fallbacks(page) = &self.page {
            self.fallbacks_body(page)
        } else if let Some(body) = self.compass_view_body() {
            body
        } else if let Page::StoreIntro(page) = &self.page {
            self.store_intro_body(page)
        } else if let Page::NowPlaying(page) = &self.page {
            self.now_playing_body(page)
        } else if let Page::Themes(page) = &self.page {
            self.themes_body(page)
        } else if let Page::Settings(page) = &self.page {
            self.settings_body(page)
        } else if let Page::Created(page) = &self.page {
            self.created_body(page)
        } else if let Page::Fonts(page) = &self.page {
            self.fonts_body(page)
        } else if let Page::FontPreview(page) = &self.page {
            self.font_preview_body(page)
        } else if let Page::Store(page) = &self.page {
            self.store_body(page)
        } else if let Page::StoreDetail(page) = &self.page {
            self.store_detail_body(page)
        } else if let Page::Clipboard(page) = &self.page {
            self.clipboard_body(page)
        } else if let Some(err) = &self.error {
            self.notice(err)
        } else if self.query.is_empty() && self.results.is_empty() {
            self.notice("Type to search")
        } else if self.results.is_empty() {
            self.notice("No results")
        } else {
            let mut list = column![].spacing(f32::from(geometry.row_spacing));
            for (position, root_row) in self.results.iter().enumerate() {
                if let Some(heading) = self.root_heading_at(position) {
                    list = list.push(self.section_heading(heading.to_owned()));
                }
                let selected = position == self.selected;
                let row = match root_row {
                    RootRow::App(index) => {
                        let Some(item) = self.app_index.items().get(*index) else {
                            continue;
                        };
                        self.result_row(item, selected)
                    }
                    RootRow::Command(command) => self.list_row(
                        self.command_icon(command, selected),
                        command.title.to_owned(),
                        self.subtitles.then(|| command.subtitle.to_owned()),
                        selected,
                    ),
                    RootRow::Update => {
                        let Some(row) = self.update_row(selected) else {
                            continue;
                        };
                        row
                    }
                    RootRow::Calculator => {
                        let Some(answer) = &self.calculator else {
                            continue;
                        };
                        self.list_row(
                            self.initial_badge("=", selected),
                            answer.answer.clone(),
                            Some(answer.question.clone()),
                            selected,
                        )
                    }
                    RootRow::Extension(index) => {
                        let Some(command) = self.app_index.extensions().get(*index) else {
                            continue;
                        };
                        self.list_row(
                            self.url_icon(
                                self.extension_url(command).as_ref(),
                                &command.title,
                                selected,
                            ),
                            command.title.clone(),
                            self.subtitles.then(|| {
                                self.extension_subtitles
                                    .get(&command.id)
                                    .unwrap_or(&command.extension_title)
                                    .clone()
                            }),
                            selected,
                        )
                    }
                    RootRow::Script(index) => {
                        let Some(script) = self.app_index.scripts().get(*index) else {
                            continue;
                        };
                        self.list_row(
                            self.url_icon(
                                self.script_icons.get(&script.id),
                                &script.title,
                                selected,
                            ),
                            script.title.clone(),
                            self.subtitles.then(|| script.subtitle.clone()),
                            selected,
                        )
                    }
                    RootRow::RhaiScript(index) => {
                        let Some(script) = self.app_index.rhai_scripts().get(*index) else {
                            continue;
                        };
                        let icon = match self.rhai_icons.get(&script.id) {
                            Some(icon) => self.extension_icon(icon, selected),
                            None => self.initial_badge(&script.title, selected),
                        };
                        self.list_row(
                            icon,
                            script.title.clone(),
                            self.subtitles.then(|| script.subtitle().to_owned()),
                            selected,
                        )
                    }
                    RootRow::Fallback(fallback) => {
                        let Some(title) = self.fallback_title(*fallback) else {
                            continue;
                        };
                        if position == 0
                            || !matches!(self.results.get(position - 1), Some(RootRow::Fallback(_)))
                        {
                            list = list.push(
                                container(
                                    text(compass_core::root_items::fallback_heading(&self.query))
                                        .font(self.font())
                                        .size(12)
                                        .color(palette.muted.to_iced()),
                                )
                                .padding(Padding::new(4.0).left(10)),
                            );
                        }
                        let subtitle = match fallback {
                            Fallback::Command(command) => command.subtitle.to_owned(),
                            Fallback::Extension(_) => "Command".to_owned(),
                            Fallback::Shortcut(_) => "Shortcut".to_owned(),
                        };
                        self.list_row(
                            match fallback {
                                Fallback::Command(command) => self.command_icon(command, selected),
                                _ => self.initial_badge(&title, selected),
                            },
                            title,
                            self.subtitles.then_some(subtitle),
                            selected,
                        )
                    }
                    RootRow::Shortcut(index) => {
                        let Some(shortcut) = self.app_index.shortcuts().get(*index) else {
                            continue;
                        };
                        let title = crate::shortcuts_page::display_name(shortcut);
                        self.list_row(
                            self.url_icon(Some(&shortcut_url(&shortcut.icon)), title, selected),
                            title.to_owned(),
                            self.subtitles.then(|| "Shortcut".to_owned()),
                            selected,
                        )
                    }
                };
                let row: Element<Message> = mouse_area(row)
                    .on_press(Message::ResultClicked(position))
                    .into();
                let row: Element<Message> = if selected {
                    container(row).id(crate::scroll::ROOT_SELECTION).into()
                } else {
                    row
                };
                list = list.push(row);
            }
            scrollable(container(list).padding(Padding::new(6.0).top(8)))
                .id(crate::scroll::ROOT_RESULTS)
                .height(Length::Shrink)
                .into()
        };

        // At the root the status bar stays on the card: the list scrolls in
        // what the card has left, rather than pushing the bar off its foot.
        let root_footer = matches!(self.page, Page::Root)
            && self.confirm.is_none()
            && self.power_confirm.is_none();
        let body: Element<Message> = if root_footer {
            let room = f32::from(geometry.card_max_height)
                - 2.0 * f32::from(geometry.card_padding)
                - f32::from(geometry.field_height)
                - preview::FOOTER_HEIGHT
                - if self.field_rule { 1.0 } else { 0.0 };
            container(body).max_height(room).into()
        } else {
            body
        };
        // Flow's hairline rule under the query field (#84). A one-pixel
        // container rather than a border on the field, because the field has
        // its own rounded border in the other presets and a rule has to span
        // the card's full width regardless of the field's radius.
        let card_content = if let Page::Onboarding(page) = &self.page {
            column![self.onboarding_body(page)].width(Length::Fill)
        } else if self.field_rule {
            column![
                field,
                container(Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(1.0))
                    .style(move |_: &Theme| container::Style {
                        background: Some(palette.border.to_iced().into()),
                        ..container::Style::default()
                    }),
                body
            ]
            .width(Length::Fill)
        } else {
            column![field, body].width(Length::Fill)
        };
        // The root search's status bar: the clock as its title, as
        // `scheduleNextClockTick` sets the navigation title, and on the
        // right what Enter does and how to reach the other actions, as
        // dmenu's footer and Raycast's action bar say it.
        let card_content = if root_footer {
            card_content.push(self.footer(self.clock_text.as_deref(), self.root_primary_action()))
        } else {
            card_content
        };

        // The panel floats over the list rather than replacing it. The old
        // comment said an overlay "needs a stacking widget and a backdrop" --
        // `stack!` is that widget, and the backdrop turned out to be
        // unnecessary because the panel is opaque and bounded.
        //
        // The card is at least as tall as the panel's unfiltered rows (up to
        // a cap past which the panel scrolls), so a panel over a short list,
        // such as a lone calculator answer, is not cut off at the card's
        // edge.
        let card_body: Element<Message> = match &self.panel {
            Some(panel) => stack![
                row![
                    card_content,
                    // One pixel wide: a row drops a child with no width at all.
                    Space::new().width(1).height(if panel.recorder.is_some() {
                        0.0
                    } else {
                        (action_panel::natural_height(&action_panel::flatten(&panel.sections, ""))
                            + 2.0 * PANEL_MARGIN)
                            .min(PANEL_ROOM_CAP)
                    })
                ],
                container(self.view_panel(panel))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::End)
                    .align_y(Alignment::End)
                    .padding(PANEL_MARGIN)
            ]
            .into(),
            None => card_content.into(),
        };

        let card_background = card_background(palette.surface, self.tint);

        let (card_width, card_height) = match self.dmenu_card_size() {
            Some((width, height)) => (width as f32, Length::Fixed(height as f32)),
            None => (f32::from(geometry.card_width), Length::Shrink),
        };
        let card = container(card_body)
            .width(Length::Fixed(card_width))
            .height(card_height)
            .padding(geometry.card_padding)
            .style(move |_: &Theme| container::Style {
                background: Some(card_background.into()),
                border: Border {
                    color: palette.border.to_iced(),
                    width: 1.0,
                    radius: f32::from(geometry.card_radius).into(),
                },
                shadow: design::card_shadow(),
                ..container::Style::default()
            });
        // Reports the card's size when it is shown, when it resizes, and
        // again when the look the blur depends on changes (the key), so the
        // blur behind it follows the card (`crate::material`).
        let card = sensor(card)
            .key((self.tint, geometry.card_radius))
            .on_show(Message::CardMeasured)
            .on_resize(Message::CardMeasured);
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(design::SHADOW_PADDING)
            .align_x(Alignment::Center)
            .align_y(Alignment::Start)
            .into()
    }

    /// The window switcher's body: its state, or its rows.
    /// A section's heading in a list: small, muted, indented to the rows'
    /// text.
    fn section_heading<'a>(&self, label: String) -> Element<'a, Message> {
        container(
            text(label)
                .font(self.font())
                .size(12)
                .color(self.palette().muted.to_iced()),
        )
        .padding(Padding::new(4.0).left(10))
        .into()
    }

    /// The emoji picker's rows: the character in the icon slot, its name.
    fn emoji_body<'a>(&'a self, page: &'a crate::emoji_page::EmojiPage) -> Element<'a, Message> {
        let geometry = self.geometry;
        if page.shown.is_empty() {
            return self.notice("No matching emojis or symbols");
        }
        let glyphs = compass_core::glyph::glyphs();
        let mut list = column![].spacing(f32::from(geometry.row_spacing));
        if let Some(notice) = &page.notice {
            list = list.push(self.section_heading(notice.clone()));
        }
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(glyph) = glyphs.get(index) else {
                continue;
            };
            if let Some(heading) = page.heading_at(position) {
                list = list.push(self.section_heading(heading.to_owned()));
            }
            let selected = position == page.selected;
            let icon =
                container(text(page.display(glyph)).size(f32::from(geometry.icon_size) * 0.75))
                    .width(Length::Fixed(f32::from(geometry.icon_size)))
                    .height(Length::Fixed(f32::from(geometry.icon_size)))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .into();
            let row = self.list_row(
                icon,
                glyph.name.to_owned(),
                self.subtitles.then(|| glyph.category.label().to_owned()),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::EmojiSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
        }
        scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }

    /// Hides the launcher, then runs `power` in the engine, as the C++ plan
    /// does (the window closes first). A refusal is shown when it comes back.
    fn run_power_command(
        &mut self,
        power: &'static compass_core::power_commands::PowerCommand,
    ) -> Task<Message> {
        self.power_confirm = None;
        let Some(backend) = self.backend.clone() else {
            self.error = Some(format!(
                "{} needs the Compass engine, and this window is running without one",
                power.name
            ));
            return Task::none();
        };
        let id = power.id.to_owned();
        let run = Task::perform(
            async move { backend.run_power_command(id).await },
            Message::BuiltinCommandDone,
        );
        Task::batch([self.conceal(), run])
    }

    fn windows_body<'a>(
        &'a self,
        page: &'a crate::windows_page::WindowsPage,
    ) -> Element<'a, Message> {
        use crate::windows_page::Status;
        let geometry = self.geometry;
        match &page.status {
            Status::Loading => return self.notice("Loading windows…"),
            Status::Failed(reason) => return self.notice(reason),
            Status::Ready if page.all.is_empty() => {
                return self.notice("No other windows are open");
            }
            Status::Ready if page.shown.is_empty() => return self.notice("No matching windows"),
            Status::Ready => {}
        }
        let mut list = column![].spacing(f32::from(geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(window) = page.all.get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let title = if window.title.is_empty() {
                window.app.clone()
            } else {
                window.title.clone()
            };
            let row = self.list_row(
                self.window_icon(window, selected),
                title,
                self.subtitles.then(|| window.app.clone()),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::WindowSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// Search Files' body: its state, or its heading and rows.
    fn files_body<'a>(&'a self, page: &'a crate::files_page::FilesPage) -> Element<'a, Message> {
        use crate::files_page::Status;
        let geometry = self.geometry;
        let key = page
            .category
            .clone()
            .unwrap_or_else(|| compass_core::file_search::CATEGORY_FILTER_KEYS[0].to_owned());
        let filter = container(
            iced::widget::pick_list(
                compass_core::file_search::CATEGORY_FILTER_KEYS
                    .iter()
                    .map(|key| (*key).to_owned())
                    .collect::<Vec<_>>(),
                Some(key),
                Message::FilesCategoryChanged,
            )
            .text_size(12)
            .font(self.font())
            .style({
                let palette = self.palette();
                move |_, status| crate::design::dropdown(palette, status)
            })
            .menu_style({
                let palette = self.palette();
                move |_| crate::design::dropdown_menu(palette)
            }),
        )
        .width(Length::Fill)
        .align_x(Alignment::End)
        .padding(Padding::new(4.0).right(10));
        // The loading indicator (`setLoading`): a query is out.
        let indicator = container(
            text(if page.searching { "Searching…" } else { "" })
                .font(self.font())
                .size(12)
                .color(self.palette().muted.to_iced()),
        )
        .padding(Padding::new(8.0).left(12));
        let filter = row![indicator, filter];
        let empty = match &page.status {
            Status::Loading => Some("Searching files…"),
            Status::Failed(reason) => Some(reason.as_str()),
            Status::Ready if page.rows.is_empty() => Some("No files found"),
            Status::Ready => None,
        };
        if let Some(empty) = empty {
            return column![filter, self.notice(empty)].into();
        }
        let home =
            compass_core::xdg_dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
        let heading = text(page.heading.clone())
            .font(self.font())
            .size(12)
            .color(self.palette().muted.to_iced());
        let mut list = column![container(heading).padding(Padding::new(4.0).left(10))]
            .spacing(f32::from(geometry.row_spacing));
        for (position, file) in page.rows.iter().enumerate() {
            let selected = position == page.selected;
            let subtitle = self
                .subtitles
                .then(|| crate::files_page::subtitle(file, home.as_deref()));
            let row = self.list_row(
                self.glyph_or_initial(
                    self.icons
                        .then(|| self.file_glyphs.cached(&file.path))
                        .flatten(),
                    &file.name,
                    selected,
                ),
                file.name.clone(),
                subtitle,
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::FilesSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        let rows: Element<Message> = match &page.preview {
            Some(preview) => row![
                container(rows).width(Length::FillPortion(preview::LIST_PORTION)),
                self.file_preview_pane(preview, true),
            ]
            .into(),
            None => rows.into(),
        };
        match &page.notice {
            Some(notice) => column![filter, rows, self.notice(notice)].into(),
            None => column![filter, rows].into(),
        }
    }

    /// Clipboard history's body: its state, or its rows.
    fn clipboard_body<'a>(
        &'a self,
        page: &'a crate::clipboard_page::ClipboardPage,
    ) -> Element<'a, Message> {
        use crate::clipboard_page::Status;
        let geometry = self.geometry;
        let filter = self.clipboard_filter(page);
        let status = page
            .monitoring
            .and_then(crate::clipboard_page::monitoring_notice)
            .map(|line| self.section_heading(line.to_owned()));
        let empty = match &page.status {
            Status::Loading => Some("Loading clipboard history…"),
            Status::Failed(reason) => Some(reason.as_str()),
            Status::Ready
                if page.rows.is_empty() && page.query.is_empty() && page.kind.is_none() =>
            {
                Some("Nothing copied yet")
            }
            Status::Ready if page.rows.is_empty() => Some("No matching entries"),
            Status::Ready => None,
        };
        if let Some(empty) = empty {
            return column![filter].push(status).push(self.notice(empty)).into();
        }
        let mut list = column![].spacing(f32::from(geometry.row_spacing));
        for (position, entry) in page.rows.iter().enumerate() {
            let selected = position == page.selected;
            let subtitle = self
                .subtitles
                .then(|| crate::clipboard_page::subtitle(entry));
            let favicon =
                clipboard_url(entry).and_then(|url| self.url_glyphs.get(&url.to_url())?.clone());
            let row = self.list_row(
                self.glyph_or_initial(
                    self.icons
                        .then(|| {
                            favicon.unwrap_or_else(|| crate::icons::clipboard_glyph(entry.kind))
                        })
                        .as_ref(),
                    crate::clipboard_page::subtitle(entry).as_str(),
                    selected,
                ),
                entry.preview.clone(),
                subtitle,
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::ClipboardSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        let rows: Element<Message> = match &page.detail {
            Some(detail) => row![
                container(rows).width(Length::FillPortion(preview::LIST_PORTION)),
                self.clipboard_detail_pane(detail, &page.query),
            ]
            .into(),
            None => rows.into(),
        };
        let body = column![filter].push(status).push(rows);
        match &page.notice {
            Some(notice) => body.push(self.notice(notice)).into(),
            None => body.into(),
        }
    }

    /// A line of explanation where the list would be.
    fn notice(&self, message: &str) -> Element<'_, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        container(
            text(message.to_owned())
                .font(self.font())
                .size(f32::from(geometry.title_size))
                .color(palette.muted.to_iced()),
        )
        .width(Length::Fill)
        .padding(28)
        .align_x(Alignment::Center)
        .into()
    }

    /// One result: icon, title, subtitle.
    ///
    /// The icon slot is a fixed square of `geometry.icon_size`, and what goes
    /// in it depends on `launcher.appearance.icons` (#85). Off, or on with a
    /// name the theme cannot resolve, it is the application's first letter in a
    /// tinted square. On and resolved, it is the icon itself.
    ///
    /// **The slot is the same size either way**, which is what keeps the
    /// fallback from being visible as a defect: a missing icon leaves the row's
    /// proportions and the title's position exactly where the others are, and
    /// the VM tier's window box does not move when the option is turned on.
    fn result_row(&self, item: &AppItem, selected: bool) -> Element<'_, Message> {
        let icon = self.app_icon(item, selected);
        // `subtitles` gates this, not just the presence of a comment: the dense
        // preset's row is one line tall and a second would overflow it.
        let subtitle = self
            .subtitles
            .then(|| item.comment().map(str::to_owned))
            .flatten();
        self.list_row(icon, item.name().to_owned(), subtitle, selected)
    }

    /// An application's icon slot: its themed icon when icons are on and it
    /// resolved, its initial otherwise. See [`Self::result_row`].
    fn app_icon(&self, item: &AppItem, selected: bool) -> Element<'_, Message> {
        let geometry = self.geometry;
        match self.row_art(item) {
            Some(crate::icons::IconArt::Raster(path)) => {
                container(image(path).width(Length::Fill).height(Length::Fill))
                    .width(Length::Fixed(f32::from(geometry.icon_size)))
                    .height(Length::Fixed(f32::from(geometry.icon_size)))
                    .into()
            }
            Some(crate::icons::IconArt::Vector(path)) => {
                container(svg(path).width(Length::Fill).height(Length::Fill))
                    .width(Length::Fixed(f32::from(geometry.icon_size)))
                    .height(Length::Fixed(f32::from(geometry.icon_size)))
                    .into()
            }
            None => self.initial_badge(item.name(), selected),
        }
    }

    /// An extension row's icon: its art, a builtin tinted to read on the
    /// theme, or a grid cell's colour.
    fn extension_icon(
        &self,
        icon: &crate::extension_page::RowIcon,
        selected: bool,
    ) -> Element<'_, Message> {
        self.masked_icon(icon, compass_core::image_url::ImageMask::None, selected)
    }

    /// [`Self::extension_icon`], clipped to `mask` (`Image.mask`): the
    /// image drawn into pixels and clipped as `applyCircleMask` and
    /// `applyRoundedRectMask` clip it.
    fn masked_icon(
        &self,
        icon: &crate::extension_page::RowIcon,
        mask: compass_core::image_url::ImageMask,
        selected: bool,
    ) -> Element<'_, Message> {
        self.icon_art(icon, mask, selected, f32::from(self.geometry.icon_size))
    }

    /// [`Self::masked_icon`] in a square of `side`.
    fn icon_art(
        &self,
        icon: &crate::extension_page::RowIcon,
        mask: compass_core::image_url::ImageMask,
        selected: bool,
        side: f32,
    ) -> Element<'_, Message> {
        use crate::extension_page::RowIcon;
        let size = Length::Fixed(side);
        let palette = self.palette();
        let text_color = if selected {
            palette.selection_text
        } else {
            palette.text
        }
        .to_iced();
        let masked = match icon {
            RowIcon::Art {
                art,
                monochrome,
                tint,
            } if mask != compass_core::image_url::ImageMask::None => {
                let color = tint.or(monochrome.then_some(text_color)).map(|c| {
                    let [r, g, b, _] = c.into_rgba8();
                    [r, g, b]
                });
                self.view.masked.get(art, mask, color)
            }
            _ => None,
        };
        if let Some(handle) = masked {
            return container(image(handle).width(Length::Fill).height(Length::Fill))
                .width(size)
                .height(size)
                .into();
        }
        let art: Element<Message> = match icon {
            RowIcon::Text(glyph) => container(text(glyph.clone()).size(side * 0.75))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .into(),
            RowIcon::Swatch(color) => {
                let color = *color;
                container(text(""))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(move |_: &Theme| container::Style {
                        background: Some(color.into()),
                        border: Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: 6.0.into(),
                        },
                        ..container::Style::default()
                    })
                    .into()
            }
            RowIcon::Art {
                art: crate::icons::IconArt::Raster(path),
                ..
            } => image(path).width(Length::Fill).height(Length::Fill).into(),
            RowIcon::Art {
                art: crate::icons::IconArt::Vector(path),
                monochrome,
                tint,
            } => {
                let color = tint.or(monochrome.then_some(text_color));
                svg(path)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(move |_: &Theme, _| iced::widget::svg::Style { color })
                    .into()
            }
        };
        container(art).width(size).height(size).into()
    }

    /// A builtin command's icon slot: its icon on its C++ tile when icons are
    /// on and the builtin set is installed, its initial otherwise.
    fn command_icon(
        &self,
        command: &compass_core::commands::BuiltinCommand,
        selected: bool,
    ) -> Element<'_, Message> {
        let glyph = self
            .icons
            .then(|| crate::icons::command_glyph(command, self.palette().accent));
        self.glyph_or_initial(glyph.as_ref(), command.title, selected)
    }

    /// A window's icon slot: its application's icon, else `AppWindow`
    /// (`SwitchWindowsSection::displayIcon`).
    fn window_icon(
        &self,
        window: &crate::backend::WindowRow,
        selected: bool,
    ) -> Element<'_, Message> {
        let glyph = self.icons.then(|| {
            self.window_app(window)
                .and_then(|item| self.row_art(item))
                .cloned()
                .map_or_else(
                    || crate::icons::Glyph::builtin("app-window"),
                    crate::icons::Glyph::Art,
                )
        });
        self.glyph_or_initial(glyph.as_ref(), &window.app, selected)
    }

    /// The application a window belongs to, when the engine recognised it.
    fn window_app(&self, window: &crate::backend::WindowRow) -> Option<&AppItem> {
        if !window.app_known {
            return None;
        }
        self.app_index
            .items()
            .iter()
            .find(|item| item.name() == window.app)
    }

    /// `glyph` drawn in the row's icon slot, or `title`'s initial when there
    /// is none or it cannot be drawn (a builtin with no installed icon set).
    fn glyph_or_initial(
        &self,
        glyph: Option<&crate::icons::Glyph>,
        title: &str,
        selected: bool,
    ) -> Element<'_, Message> {
        glyph
            .and_then(|glyph| self.glyph(glyph, selected, f32::from(self.geometry.icon_size)))
            .unwrap_or_else(|| self.initial_badge(title, selected))
    }

    /// The builtin icon `name` as an SVG of `size`, drawn in `color`.
    fn builtin_svg(
        &self,
        name: &str,
        color: Color,
        size: f32,
    ) -> Option<Element<'static, Message>> {
        let file = self
            .builtin_icons
            .as_ref()?
            .join(compass_core::builtin_icon::file_name(name)?);
        Some(
            svg(file)
                .width(Length::Fixed(size))
                .height(Length::Fixed(size))
                .style(move |_: &Theme, _| iced::widget::svg::Style { color: Some(color) })
                .into(),
        )
    }

    /// A [`crate::icons::Glyph`] drawn in a square of `size`.
    ///
    /// A tile is `applyBackdrop`'s rounded square (a quarter of the side),
    /// with the glyph inset by 19% of it as `backdropContentSize`; a badge is
    /// `applyBadge`'s black disc, 44% of the side, 4% in from the corner,
    /// carrying the badge glyph in white.
    fn glyph(
        &self,
        glyph: &crate::icons::Glyph,
        selected: bool,
        size: f32,
    ) -> Option<Element<'static, Message>> {
        use crate::icons::Glyph;
        let square = Length::Fixed(size);
        match glyph {
            Glyph::Text(glyph) => Some(
                container(text(glyph.clone()).size(size * 0.75))
                    .width(square)
                    .height(square)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .into(),
            ),
            Glyph::Art(art) => {
                let drawn: Element<'static, Message> = match art {
                    crate::icons::IconArt::Raster(path) => image(path.clone())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into(),
                    crate::icons::IconArt::Vector(path) => svg(path.clone())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into(),
                };
                Some(container(drawn).width(square).height(square).into())
            }
            Glyph::Builtin {
                name,
                fill,
                tile,
                badge,
            } => {
                let palette = self.palette();
                let text_color = if selected {
                    palette.selection_text
                } else {
                    palette.text
                };
                let base: Element<'static, Message> = match tile {
                    None => container(self.builtin_svg(
                        name,
                        fill.unwrap_or(text_color).to_iced(),
                        size * 0.8,
                    )?)
                    .width(square)
                    .height(square)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .into(),
                    Some(tile) => {
                        let (top, bottom) = crate::icons::tile_gradient(*tile);
                        let gradient = iced::gradient::Linear::new(std::f32::consts::PI)
                            .add_stop(0.0, top.to_iced())
                            .add_stop(1.0, bottom.to_iced());
                        let inner = size * (1.0 - 2.0 * 0.19);
                        let centred = |element: Element<'static, Message>, drop: f32| {
                            container(element)
                                .width(square)
                                .height(square)
                                .align_x(Alignment::Center)
                                .align_y(Alignment::Center)
                                .padding(Padding::ZERO.top(drop * 2.0))
                        };
                        // `applyBackdrop`'s shadow: the glyph's silhouette in
                        // translucent black, a little lower, under it.
                        let shadow = self.builtin_svg(
                            name,
                            Color::from_rgba8(
                                0,
                                0,
                                0,
                                f32::from(crate::icons::TILE_SHADOW_ALPHA) / 255.0,
                            ),
                            inner,
                        )?;
                        let glyph = self.builtin_svg(
                            name,
                            fill.unwrap_or(crate::design::Rgb::new(255, 255, 255))
                                .to_iced(),
                            inner,
                        )?;
                        container(iced::widget::stack![
                            centred(shadow, size * crate::icons::TILE_SHADOW_OFFSET),
                            centred(glyph, 0.0),
                        ])
                        .width(square)
                        .height(square)
                        .style(move |_: &Theme| container::Style {
                            background: Some(iced::Background::Gradient(gradient.into())),
                            border: Border {
                                color: Color::from_rgba8(255, 255, 255, 30.0 / 255.0),
                                width: (size / 32.0).max(1.0),
                                radius: (size * 0.25).into(),
                            },
                            ..container::Style::default()
                        })
                        .into()
                    }
                };
                let Some(badge) = badge else {
                    return Some(base);
                };
                let diameter = size * 0.44;
                let disc = container(self.builtin_svg(badge, Color::WHITE, diameter * 0.56)?)
                    .width(Length::Fixed(diameter))
                    .height(Length::Fixed(diameter))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .style(move |_: &Theme| container::Style {
                        background: Some(Color::BLACK.into()),
                        border: Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: (diameter / 2.0).into(),
                        },
                        ..container::Style::default()
                    });
                let corner = container(disc)
                    .width(square)
                    .height(square)
                    .align_x(Alignment::End)
                    .align_y(Alignment::End)
                    .padding(size * 0.04);
                Some(iced::widget::stack![base, corner].into())
            }
        }
    }

    /// The first letter of `title` in a tinted square, for a row with no art.
    fn initial_badge(&self, title: &str, selected: bool) -> Element<'_, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        let title_color = if selected {
            palette.selection_text
        } else {
            palette.text
        };
        let initial = title
            .chars()
            .next()
            .map_or_else(String::new, |c| c.to_uppercase().to_string());
        container(
            text(initial)
                .font(self.font())
                .size(f32::from(geometry.icon_size) / 2.0)
                .color(title_color.to_iced()),
        )
        .width(Length::Fixed(f32::from(geometry.icon_size)))
        .height(Length::Fixed(f32::from(geometry.icon_size)))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_: &Theme| container::Style {
            background: Some(
                Color {
                    a: 0.18,
                    ..palette.accent.to_iced()
                }
                .into(),
            ),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        })
        .into()
    }

    /// One list row: an icon slot, a title and an optional subtitle, with the
    /// selection drawn as a filled rounded rectangle. Every list the card
    /// shows -- applications, commands, clipboard history -- is these rows, so
    /// they measure and select alike.
    fn list_row<'a>(
        &'a self,
        icon: Element<'a, Message>,
        title: String,
        subtitle: Option<String>,
        selected: bool,
    ) -> Element<'a, Message> {
        self.list_row_with(icon, title, subtitle, None, selected)
    }

    /// [`Self::list_row`] with a line of muted text at the right edge, as a
    /// store row shows its download count and whether it is installed.
    fn list_row_with<'a>(
        &'a self,
        icon: Element<'a, Message>,
        title: String,
        subtitle: Option<String>,
        accessory: Option<String>,
        selected: bool,
    ) -> Element<'a, Message> {
        let colour = if selected {
            self.palette().selection_text
        } else {
            self.palette().muted
        };
        let accessory = accessory.filter(|text| !text.is_empty()).map(|accessory| {
            text(accessory)
                .font(self.font())
                .size(f32::from(self.geometry.subtitle_size))
                .color(colour.to_iced())
                .into()
        });
        self.list_row_parts(icon, title, subtitle, accessory, selected)
    }

    /// [`Self::list_row_with`], with any element at the row's right.
    fn list_row_parts<'a>(
        &'a self,
        icon: Element<'a, Message>,
        title: String,
        subtitle: Option<String>,
        accessory: Option<Element<'a, Message>>,
        selected: bool,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        let title_color = if selected {
            palette.selection_text
        } else {
            palette.text
        };
        let subtitle_color = if selected {
            palette.selection_text
        } else {
            palette.muted
        };

        // One line each, elided: the row is a fixed height, and a wrapped
        // subtitle ran into the row below it (P-07).
        let mut labels = column![crate::elided::elided(
            title,
            f32::from(geometry.title_size),
            Some(self.font()),
            title_color.to_iced(),
        )];
        if let Some(subtitle) = subtitle {
            labels = labels.push(crate::elided::elided(
                subtitle,
                f32::from(geometry.subtitle_size),
                Some(self.font()),
                subtitle_color.to_iced(),
            ));
        }

        let line = match accessory {
            Some(accessory) => row![icon, labels.width(Length::Fill), accessory],
            None => row![icon, labels.width(Length::Fill)],
        }
        .spacing(12)
        .align_y(Alignment::Center)
        .padding(Padding::new(0.0).left(12).right(12));
        container(line)
            .width(Length::Fill)
            .height(Length::Fixed(f32::from(geometry.row_height)))
            .align_y(Alignment::Center)
            .style(move |_: &Theme| {
                if selected {
                    container::Style {
                        background: Some(palette.selection.to_iced().into()),
                        border: Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: f32::from(geometry.row_radius).into(),
                        },
                        ..container::Style::default()
                    }
                } else {
                    container::Style::default()
                }
            })
            .into()
    }

    /// Draw the action panel.
    ///
    /// A floating card at the bottom right, the way Raycast's sits. Headers
    /// and dividers are drawn as themselves rather than as indented text, and
    /// the selection is the same filled rectangle the result list uses.
    fn view_panel<'a>(&'a self, panel: &'a PanelState) -> Element<'a, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        if let Some(recorder) = &panel.recorder {
            return self.view_recorder(recorder);
        }
        let filter = text_input("Search…", &panel.filter)
            .id(PANEL_INPUT)
            .font(self.font())
            .on_input(Message::PanelFilterChanged)
            .padding(
                Padding::new(0.0)
                    .left(design::panel_metric("inset"))
                    .right(design::panel_metric("inset")),
            )
            .style(move |theme: &Theme, status| query_input_style(theme, status, palette))
            .size(f32::from(geometry.title_size));
        let filter = container(filter)
            .height(design::panel_metric("filter-height"))
            .align_y(Alignment::Center);
        let mut col = column![].spacing(design::panel_metric("gap"));

        for (index, panel_row) in panel.rows.iter().enumerate() {
            let element: Element<Message> = match panel_row.kind {
                RowKind::Divider => container(
                    container(Space::new().height(Length::Fixed(1.0)))
                        .width(Length::Fill)
                        .id("panel-divider-line")
                        .style(move |_: &Theme| container::Style {
                            background: Some(palette.border.to_iced().into()),
                            ..container::Style::default()
                        }),
                )
                .padding(
                    Padding::new(design::panel_metric("divider-gap"))
                        .left(design::panel_metric("inset"))
                        .right(design::panel_metric("inset")),
                )
                .into(),
                RowKind::Header => {
                    let name = panel
                        .sections
                        .get(panel_row.section)
                        .map_or("", |section| section.name.as_str());
                    container(
                        text(name.to_uppercase())
                            .font(self.font())
                            .size(f32::from(geometry.heading_size))
                            .color(palette.muted.to_iced()),
                    )
                    .height(design::panel_metric("header-height"))
                    .align_y(Alignment::Center)
                    .padding(Padding::new(0.0).left(design::panel_metric("inset")))
                    .into()
                }
                RowKind::Item => {
                    let action = panel_row.action.and_then(|position| {
                        panel.sections.get(panel_row.section)?.actions.get(position)
                    });
                    let title = action.map_or("", |action| action.title.as_str());
                    let shortcut = action.and_then(|action| action.shortcut.clone());
                    let selected = isize::try_from(index).unwrap_or(isize::MAX) == panel.selected;
                    let item = self.panel_item(title, shortcut.as_deref(), selected);
                    let item: Element<Message> = if selected {
                        container(item).id(crate::scroll::PANEL_SELECTION).into()
                    } else {
                        item
                    };
                    mouse_area(item)
                        .on_press(Message::PanelClicked(index))
                        .into()
                }
            };
            col = col.push(element);
        }

        if panel.rows.is_empty() {
            col = col.push(self.notice("No actions"));
        }

        container(
            column![
                filter,
                scrollable(col)
                    .id(crate::scroll::PANEL_RESULTS)
                    .height(Length::Shrink)
            ]
            .spacing(design::panel_metric("gap")),
        )
        .width(Length::Fixed(design::panel_metric("width")))
        .padding(design::panel_metric("padding"))
        .style(move |_: &Theme| container::Style {
            background: Some(palette.surface.to_iced().into()),
            border: Border {
                color: palette.border.to_iced(),
                width: 1.0,
                radius: 12.0.into(),
            },
            ..container::Style::default()
        })
        .into()
    }

    /// The shortcut recorder in the panel's place (`ShortcutRecorderPanel`):
    /// the item's title, then the chord or the current shortcut, the status
    /// line and, while the item has a shortcut, how to remove it.
    fn view_recorder<'a>(
        &'a self,
        recorder: &'a crate::shortcut_recorder::ShortcutRecorder,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        let small = f32::from(geometry.subtitle_size);
        let title = container(
            text(recorder.title.clone())
                .font(self.font())
                .size(f32::from(geometry.title_size))
                .color(palette.text.to_iced()),
        )
        .height(design::panel_metric("filter-height"))
        .align_y(Alignment::Center)
        .padding(Padding::new(0.0).left(design::panel_metric("inset")));
        let divider = container(Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fill)
            .style(move |_: &Theme| container::Style {
                background: Some(palette.border.to_iced().into()),
                ..container::Style::default()
            });
        let mut capture = column![].spacing(5).align_x(Alignment::Center);
        if !recorder.tokens.is_empty() {
            capture = capture.push(
                text(recorder.tokens.join(" "))
                    .font(self.font())
                    .size(f32::from(geometry.title_size))
                    .color(palette.text.to_iced()),
            );
        }
        let status = if recorder.error {
            iced::Color::from_rgb8(0xe5, 0x48, 0x4d)
        } else {
            palette.text.to_iced()
        };
        capture = capture.push(
            text(recorder.status.clone())
                .font(self.font())
                .size(small)
                .color(status),
        );
        if recorder.current.is_some() {
            capture = capture.push(
                text(crate::shortcut_recorder::REMOVE_HINT)
                    .font(self.font())
                    .size(small)
                    .color(palette.muted.to_iced()),
            );
        }
        let capture = container(capture)
            .width(Length::Fill)
            .height(Length::Fixed(130.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center);
        container(column![title, divider, capture])
            .width(Length::Fixed(design::panel_metric("width")))
            .padding(design::panel_metric("padding"))
            .style(move |_: &Theme| container::Style {
                background: Some(palette.surface.to_iced().into()),
                border: Border {
                    color: palette.border.to_iced(),
                    width: 1.0,
                    radius: 12.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    /// One action row, with its shortcut right-aligned.
    fn panel_item(
        &self,
        title: &str,
        shortcut: Option<&str>,
        selected: bool,
    ) -> Element<'_, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        let colour = if selected {
            palette.selection_text
        } else {
            palette.text
        };

        let mut line = row![
            text(title.to_owned())
                .font(self.font())
                .size(f32::from(geometry.title_size))
                .color(colour.to_iced())
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        if let Some(shortcut) = shortcut {
            line = line.push(Space::new().width(Length::Fill));
            line = line.push(
                text(compass_core::key_combo::badge(shortcut))
                    .font(self.font())
                    .size(f32::from(geometry.subtitle_size))
                    .color(
                        if selected {
                            palette.selection_text
                        } else {
                            palette.muted
                        }
                        .to_iced(),
                    ),
            );
        }

        container(
            line.padding(
                Padding::new(0.0)
                    .left(design::panel_metric("inset"))
                    .right(design::panel_metric("inset")),
            ),
        )
        .width(Length::Fill)
        .height(Length::Fixed(design::panel_metric("row-height")))
        .align_y(Alignment::Center)
        .style(move |_: &Theme| {
            if selected {
                container::Style {
                    background: Some(palette.selection.to_iced().into()),
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: design::panel_metric("row-radius").into(),
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        })
        .into()
    }

    /// Re-rank against the current query.
    fn search_task(&mut self) -> Task<Message> {
        self.cancel_search();
        self.error = None;
        // The provider search view ranks here: the engine's search is the
        // whole root list.
        if let Some(backend) = self.backend.clone()
            && self.provider_scope.is_none()
        {
            self.results.clear();
            self.selected = 0;
            let query = self.query.clone();
            let generation = self.search_generation;
            let (task, handle) =
                Task::perform(async move { backend.search(query).await }, move |result| {
                    Message::SearchCompleted { generation, result }
                })
                .abortable();
            self.search_task = Some(handle.abort_on_drop());
            return task;
        }
        self.search();
        crate::scroll::reveal_root_selection()
    }

    fn cancel_search(&mut self) {
        self.search_generation = self.search_generation.wrapping_add(1);
        if let Some(handle) = self.search_task.take() {
            handle.abort();
        }
    }

    /// Opens a builtin command's view, and counts the use like a launch.
    /// Hands an extension command to the engine, which runs it; the launcher
    /// hides once it has started, and says why when it could not.
    fn run_extension_command(&mut self, index: usize) -> Task<Message> {
        self.run_extension_command_with(index, None)
    }

    /// [`Self::run_extension_command`], with the arguments entered for it.
    fn run_extension_command_with(
        &mut self,
        index: usize,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Task<Message> {
        self.panel = None;
        let Some(command) = self.app_index.extensions().get(index) else {
            return Task::none();
        };
        let Some(backend) = self.backend.clone() else {
            self.error = Some(format!(
                "{} needs the Compass engine to run, and this window is running without one",
                command.title
            ));
            return Task::none();
        };
        let id = command.id.clone();
        let title = command.title.clone();
        let started_id = id.clone();
        Task::perform(
            async move { backend.run_extension_command(id, arguments).await },
            move |result| Message::ExtensionCommandStarted {
                id: started_id.clone(),
                title: title.clone(),
                result,
            },
        )
    }

    /// An extension command offered as a fallback: launched through the
    /// engine with the query as its fallback text, which comes back as a
    /// launch like any other (`OpenBuiltinCommandAction` forwarding the
    /// search text).
    fn run_extension_fallback(&mut self, index: usize, query: String) -> Task<Message> {
        let Some(command) = self.app_index.extensions().get(index) else {
            return Task::none();
        };
        let Some(backend) = self.backend.clone() else {
            return self.run_extension_command(index);
        };
        let id = command.id.clone();
        Task::perform(
            async move { backend.launch_command(id, Some(query)).await },
            Message::BuiltinCommandDone,
        )
    }

    /// A command the launcher let go of failed. The launcher may have hidden
    /// before it did, so a refusal said only inside it would wait unseen for
    /// the next summon: it is kept as the error line, and the HUD says it now.
    /// The HUD has room for one short line, so it gets the reason alone.
    fn command_failed(&mut self, title: &str, message: &str) -> Task<Message> {
        let hidden = self.closing || self.window.is_none();
        self.error = Some(if message.is_empty() {
            title.to_owned()
        } else {
            format!("{title}: {message}")
        });
        if hidden {
            self.error_for_summon.clone_from(&self.error);
            let line = if message.is_empty() { title } else { message };
            return self
                .put_up_hud_for_engine(crate::hud::Hud::new(line.to_owned()))
                .1;
        }
        Task::none()
    }

    /// Asks the engine for `session`'s next state after `after`.
    fn extension_poll(&self, session: u64, after: u64) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.extension_view(session, after).await },
            move |result| Message::ExtensionViewLoaded { session, result },
        )
    }

    fn extension_event(
        &self,
        session: u64,
        handler: String,
        args: Vec<serde_json::Value>,
    ) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.extension_event(session, handler, args).await },
            Message::ExtensionEventSent,
        )
    }

    /// Escape on a pushed extension view: the extension pops it and renders
    /// the view beneath, which arrives like any other render.
    fn extension_pop(&self, session: u64) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.extension_pop(session).await },
            Message::ExtensionEventSent,
        )
    }

    /// Runs the panel action whose advertised shortcut was pressed, if one
    /// was: over the open panel, or over the panel Ctrl+B would open. A
    /// chord that only moves (the keybinding scheme's) or that no action
    /// claims changes nothing and is left to the rest of the key handler.
    fn advertised_chord(
        &mut self,
        key: &iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Option<Task<Message>> {
        if !(modifiers.control() || modifiers.alt() || modifiers.logo())
            || chord_direction(self.keybinding, key.as_ref(), modifiers).is_some()
            || matches!(
                self.page,
                Page::Extension(_) | Page::Settings(_) | Page::Preferences(_)
            )
        {
            return None;
        }
        let name = pressed_key_name(key)?;
        let pressed = action_panel::Pressed {
            key: &name,
            ctrl: modifiers.control(),
            shift: modifiers.shift(),
            alt: modifiers.alt(),
            logo: modifiers.logo(),
        };
        let opened_here = self.panel.is_none();
        if opened_here {
            // Opened as Ctrl+B opens it, but not shown: what it would focus
            // or fetch for showing is dropped.
            let _ = self.update(Message::TogglePanel);
            if self.panel.is_none() {
                self.panel = self.files_panel_now();
            }
        }
        let Some(row) = self
            .panel
            .as_ref()
            .and_then(|panel| panel.row_answering(&pressed))
        else {
            if opened_here {
                self.panel = None;
            }
            return None;
        };
        let panel = self.panel.as_mut()?;
        panel.selected = isize::try_from(row).ok()?;
        let sections = panel.sections.clone();
        let task = self.update(Message::PanelActivate);
        // An action that left the panel as it was (one that copies and
        // stays, say) leaves no panel the person never opened; one that
        // turned it into something else (the shortcut recorder) keeps it.
        let untouched = self
            .panel
            .as_ref()
            .is_some_and(|panel| panel.recorder.is_none() && panel.sections == sections);
        if opened_here && untouched {
            self.panel = None;
            return Some(Task::batch([task, focus_search()]));
        }
        Some(task)
    }

    /// Search Files' panel for the selected file, without the engine's
    /// answer about it: the panel itself waits for that answer before it
    /// opens, but a shortcut only needs to know which action it names.
    fn files_panel_now(&self) -> Option<PanelState> {
        let Page::Files(page) = &self.page else {
            return None;
        };
        let path = &page.selected_row()?.path;
        let assumed = crate::backend::FileActions {
            mime: self.file_mime.clone(),
            has_opener: true,
            can_set_wallpaper: true,
            can_paste: true,
        };
        Some(PanelState::new(file_actions::file_panel_sections(
            path, &assumed,
        )))
    }

    /// Enter in an extension's view: the first action on offer.
    fn activate_extension_action(&mut self) -> Task<Message> {
        let Page::Extension(page) = &self.page else {
            return Task::none();
        };
        let Some(handler) = page.primary_action().cloned() else {
            return Task::none();
        };
        self.extension_event(page.session, handler.0, page.action_args())
    }

    /// Stops the extension whose view is open, if one is.
    fn close_extension_view(&self) -> Task<Message> {
        let (Page::Extension(page), Some(backend)) = (&self.page, self.backend.clone()) else {
            return Task::none();
        };
        let session = page.session;
        Task::future(async move {
            if let Err(error) = backend.close_extension(session).await {
                tracing::warn!(%error, "could not close an extension view");
            }
        })
        .discard()
    }

    /// A checkbox in a form, ringed in the accent colour while the keyboard
    /// is on it: it cannot take Iced's focus, so it draws none of its own.
    fn form_focus_ring<'a>(
        &self,
        focused: bool,
        field: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let accent = self.palette().accent.to_iced();
        container(field)
            .padding(4)
            .style(move |_: &Theme| container::Style {
                border: Border {
                    color: if focused { accent } else { Color::TRANSPARENT },
                    width: 2.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    fn preferences_body<'a>(
        &'a self,
        page: &'a crate::preferences_page::PreferencesPage,
    ) -> Element<'a, Message> {
        use crate::backend::PreferenceInputKind;
        use crate::preferences_page::FieldValue;
        let mut form = column![
            iced::widget::text(match page.purpose {
                crate::preferences_page::Purpose::Preferences => {
                    format!("{} needs a few settings", page.title)
                }
                crate::preferences_page::Purpose::CommandPreferences => {
                    format!("{} settings", page.title)
                }
                crate::preferences_page::Purpose::Arguments
                | crate::preferences_page::Purpose::ShortcutArguments
                | crate::preferences_page::Purpose::ShortcutForm { .. }
                | crate::preferences_page::Purpose::SnippetArguments { .. }
                | crate::preferences_page::Purpose::SnippetForm { .. }
                | crate::preferences_page::Purpose::ScriptArguments
                | crate::preferences_page::Purpose::MediaArguments
                | crate::preferences_page::Purpose::GlyphKeywords
                | crate::preferences_page::Purpose::ClipboardKeywords
                | crate::preferences_page::Purpose::Alias
                | crate::preferences_page::Purpose::CreateExtension => page.title.clone(),
            })
            .font(self.font())
            .size(14)
        ]
        .spacing(12)
        .padding(Padding::new(16.0));
        for (index, (field, value)) in page.fields.iter().zip(&page.values).enumerate() {
            let label = if field.required {
                format!("{} *", field.title)
            } else {
                field.title.clone()
            };
            let mut entry =
                column![iced::widget::text(label).font(self.font()).size(13)].spacing(4);
            let input: Element<Message> = match (&field.kind, value) {
                (
                    PreferenceInputKind::Text | PreferenceInputKind::Password,
                    FieldValue::Text(text),
                ) => text_input(&field.placeholder, text)
                    .id(crate::preferences_page::field_id(index))
                    .secure(matches!(field.kind, PreferenceInputKind::Password))
                    .font(self.font())
                    .style(crate::adwaita::form_entry)
                    .on_input(move |text| Message::PreferenceEdited(index, FieldValue::Text(text)))
                    // No `on_submit`: the key handler submits the form on
                    // Enter, and `listen_with` delivers it whether or not the
                    // field captured it, so a field that also submitted made
                    // every Enter submit twice.
                    .padding(8)
                    .into(),
                (PreferenceInputKind::Checkbox { label }, FieldValue::Checked(checked)) => self
                    .form_focus_ring(
                        page.focus == Some(index),
                        iced::widget::checkbox(*checked)
                            .label(label.clone())
                            .on_toggle(move |checked| {
                                Message::PreferenceEdited(index, FieldValue::Checked(checked))
                            })
                            .into(),
                    ),
                (PreferenceInputKind::Dropdown { options }, FieldValue::Choice(choice)) => {
                    let titles: Vec<String> =
                        options.iter().map(|(title, _)| title.clone()).collect();
                    let selected = choice.as_ref().and_then(|value| {
                        options
                            .iter()
                            .find(|(_, v)| v == value)
                            .map(|(title, _)| title.clone())
                    });
                    let options = options.clone();
                    // Ringed through its own border, so it lines up with the
                    // text fields above it.
                    let ring = (page.focus == Some(index)).then(|| self.palette().accent.to_iced());
                    iced::widget::pick_list(titles, selected, move |title: String| {
                        let value = options
                            .iter()
                            .find(|(t, _)| *t == title)
                            .map(|(_, value)| value.clone());
                        Message::PreferenceEdited(index, FieldValue::Choice(value))
                    })
                    .font(self.font())
                    .menu_style({
                        let palette = self.palette();
                        move |_: &Theme| crate::design::dropdown_menu(palette)
                    })
                    .style(move |theme: &Theme, status| {
                        let mut style = crate::adwaita::form_dropdown(theme, status);
                        if let Some(accent) = ring {
                            style.border.color = accent;
                            style.border.width = 2.0;
                        }
                        style
                    })
                    .into()
                }
                (PreferenceInputKind::TextArea, _) => match page.editors.get(&index) {
                    Some(editor) => iced::widget::text_editor(editor)
                        .id(crate::preferences_page::field_id(index))
                        .placeholder(field.placeholder.as_str())
                        .font(self.font())
                        .height(Length::Fixed(120.0))
                        .padding(8)
                        .on_action(move |action| Message::PreferenceTextEdited(index, action))
                        .into(),
                    None => iced::widget::text("").into(),
                },
                (PreferenceInputKind::Unsupported { declared }, _) => {
                    iced::widget::text(format!("Compass cannot edit {declared} preferences yet"))
                        .font(self.font())
                        .size(12)
                        .into()
                }
                _ => iced::widget::text("").into(),
            };
            entry = entry.push(input);
            if !field.description.is_empty() {
                entry = entry.push(
                    iced::widget::text(field.description.clone())
                        .font(self.font())
                        .size(11),
                );
            }
            form = form.push(entry);
        }
        form = form.push(
            iced::widget::text(page.purpose.hint())
                .font(self.font())
                .size(12),
        );
        if let Some(notice) = &page.notice {
            form = form.push(iced::widget::text(notice.clone()).font(self.font()));
        }
        scrollable(form)
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }

    /// An extension's form: its fields as the person has them, and how to
    /// submit it.
    fn extension_form<'a>(
        &'a self,
        page: &'a crate::extension_page::ExtensionPage,
        form: &'a compass_extension_api::view::FormView,
    ) -> Element<'a, Message> {
        use compass_extension_api::view::{FieldKind, FormItem};
        let mut body = column![].spacing(12).padding(Padding::new(16.0));
        for item in &form.items {
            let field = match item {
                FormItem::Separator { .. } => {
                    body = body.push(iced::widget::rule::horizontal(1));
                    continue;
                }
                FormItem::Description { title, text, .. } => {
                    let mut entry = column![].spacing(4);
                    if let Some(title) = title {
                        entry = entry
                            .push(iced::widget::text(title.clone()).font(self.font()).size(13));
                    }
                    body = body.push(
                        entry.push(iced::widget::text(text.clone()).font(self.font()).size(12)),
                    );
                    continue;
                }
                FormItem::Field(field) => field,
            };
            let name = field.name.clone();
            let value = page.form_values.get(&field.name);
            let text_value = value
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let mut entry = column![].spacing(4);
            if let Some(title) = &field.title {
                entry = entry.push(iced::widget::text(title.clone()).font(self.font()).size(13));
            }
            let input: Element<Message> = match &field.kind {
                FieldKind::TextArea { placeholder, .. } => match page.editors.get(&field.name) {
                    Some(editor) => iced::widget::text_editor(editor)
                        .placeholder(placeholder.as_deref().unwrap_or_default())
                        .font(self.font())
                        .height(Length::Fixed(96.0))
                        .padding(8)
                        .on_action(move |action| {
                            Message::ExtensionTextAreaEdited(name.clone(), action)
                        })
                        .into(),
                    None => iced::widget::text("").into(),
                },
                FieldKind::Text { placeholder } | FieldKind::Password { placeholder } => {
                    text_input(placeholder.as_deref().unwrap_or_default(), text_value)
                        .secure(matches!(field.kind, FieldKind::Password { .. }))
                        .font(self.font())
                        .style(crate::adwaita::form_entry)
                        .on_input(move |text| {
                            Message::ExtensionFieldEdited(name.clone(), text.into())
                        })
                        .padding(8)
                        .into()
                }
                FieldKind::Checkbox { label } => iced::widget::checkbox(
                    value.and_then(serde_json::Value::as_bool).unwrap_or(false),
                )
                .label(label.clone().unwrap_or_default())
                .on_toggle(move |on| Message::ExtensionFieldEdited(name.clone(), on.into()))
                .into(),
                FieldKind::Dropdown(dropdown) => {
                    let options: Vec<(String, String)> = dropdown
                        .sections
                        .iter()
                        .flat_map(|section| &section.options)
                        .map(|option| (option.title.clone(), option.value.clone()))
                        .collect();
                    let titles: Vec<String> = options.iter().map(|(t, _)| t.clone()).collect();
                    let selected = options
                        .iter()
                        .find(|(_, v)| {
                            Some(v.as_str()) == value.and_then(serde_json::Value::as_str)
                        })
                        .map(|(title, _)| title.clone());
                    iced::widget::pick_list(titles, selected, move |title: String| {
                        let chosen = options
                            .iter()
                            .find(|(t, _)| *t == title)
                            .map(|(_, value)| value.clone())
                            .unwrap_or_default();
                        Message::ExtensionFieldEdited(name.clone(), chosen.into())
                    })
                    .font(self.font())
                    .style(crate::adwaita::form_dropdown)
                    .menu_style({
                        let palette = self.palette();
                        move |_: &Theme| crate::design::dropdown_menu(palette)
                    })
                    .into()
                }
                FieldKind::DatePicker { precision, .. } => crate::extension_fields::date_field(
                    &field.name,
                    page.date_shown(&field.name, *precision),
                    *precision,
                    self.font(),
                ),
                FieldKind::TagPicker { options, .. } => crate::extension_fields::tag_field(
                    &field.name,
                    options,
                    &crate::extension_fields::strings(value),
                    self.font(),
                ),
                FieldKind::FilePicker {
                    allow_multiple,
                    allow_directories,
                    allow_files,
                } => crate::extension_fields::file_field(
                    &field.name,
                    &crate::extension_fields::strings(value),
                    crate::extension_fields::FileChoice {
                        multiple: *allow_multiple,
                        directories: *allow_directories,
                        files: *allow_files,
                    },
                    self.font(),
                ),
            };
            entry = entry.push(input);
            if let Some(error) = &field.error {
                entry = entry.push(
                    iced::widget::text(error.clone())
                        .font(self.font())
                        .size(11)
                        .color(self.theme().palette().danger),
                );
            } else if let Some(info) = &field.info {
                entry = entry.push(iced::widget::text(info.clone()).font(self.font()).size(11));
            }
            body = body.push(entry);
        }
        let submit = page
            .actions()
            .and_then(|panel| panel.actions().into_iter().next())
            .map_or_else(
                || "Esc: back".to_owned(),
                |action| {
                    let submit = if page.has_text_area() {
                        "Ctrl+Enter"
                    } else {
                        "Enter"
                    };
                    format!("{submit}: {}    Esc: back", action.title)
                },
            );
        body = body.push(iced::widget::text(submit).font(self.font()).size(12));
        if let Some(notice) = &page.notice {
            body = body.push(iced::widget::text(notice.clone()).font(self.font()));
        }
        scrollable(body)
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }

    /// An extension's page: its view, with its toast underneath.
    fn extension_body<'a>(
        &'a self,
        page: &'a crate::extension_page::ExtensionPage,
    ) -> Element<'a, Message> {
        let body = self.extension_view_body(page);
        let Some(toast) = &page.toast else {
            return body;
        };
        let mut line = toast.title.clone();
        if !toast.message.is_empty() {
            line.push_str(" — ");
            line.push_str(&toast.message);
        }
        if toast.animated {
            line.push('…');
        }
        let mut footer = iced::widget::text(line).font(self.font()).size(12);
        if toast.failure {
            footer = footer.color(self.theme().palette().danger);
        }
        column![body, container(footer).padding(Padding::new(6.0).left(14))].into()
    }

    fn extension_view_body<'a>(
        &'a self,
        page: &'a crate::extension_page::ExtensionPage,
    ) -> Element<'a, Message> {
        use crate::extension_page::Status;
        use compass_extension_api::View;
        let geometry = self.geometry;
        if let Some(alert) = &page.alert {
            let mut prompt = column![
                iced::widget::text(alert.title.clone())
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..self.font()
                    })
                    .size(16)
            ]
            .spacing(8)
            .padding(Padding::new(18.0));
            if !alert.message.is_empty() {
                prompt = prompt.push(iced::widget::text(alert.message.clone()).font(self.font()));
            }
            let keys = match &alert.remember_text {
                Some(remember) => format!(
                    "Enter: {}    Ctrl+Enter: {remember}    Esc: {}",
                    alert.confirm_text, alert.cancel_text
                ),
                None => format!(
                    "Enter: {}    Esc: {}",
                    alert.confirm_text, alert.cancel_text
                ),
            };
            prompt = prompt.push(iced::widget::text(keys).font(self.font()).size(12));
            return prompt.into();
        }
        match (&page.status, &page.view) {
            (Status::Loading, _) => return self.notice("Loading…"),
            (Status::Stopped(why), _) => return self.notice(why),
            (Status::Ready, Some(View::Detail(_))) => {
                // Its images drawn once fetched, as the store page draws a
                // README's.
                let markdown = iced::widget::markdown::view_with(
                    &page.markdown,
                    self.markdown_settings(),
                    &stores::StoreMarkdown {
                        images: &page.markdown_art,
                    },
                );
                let body = scrollable(container(markdown).padding(Padding::new(14.0)))
                    .id(crate::scroll::ROOT_RESULTS)
                    .height(Length::Shrink);
                return match &page.notice {
                    Some(notice) => column![body, self.notice(notice)].into(),
                    None => body.into(),
                };
            }
            (Status::Ready, Some(View::Form(form))) => return self.extension_form(page, form),
            (Status::Ready, Some(View::List(_))) => {}
            (Status::Ready, _) => {
                return self.notice("Compass cannot draw this extension view yet");
            }
        }
        if page.shown.is_empty() {
            let empty = page
                .list()
                .and_then(|list| list.empty_state.as_ref())
                .map_or("No results", |empty| empty.title.as_str());
            return self.notice(empty);
        }
        if page.grid_columns.is_some() {
            return self.extension_grid(page);
        }
        let mut list = column![].spacing(f32::from(geometry.row_spacing));
        let sections = page.list().map(|list| &list.sections[..]).unwrap_or(&[]);
        let mut last_section = None;
        for (position, &(s, i)) in page.shown.iter().enumerate() {
            // Each section under its title, as `List.Section` draws it.
            if last_section != Some(s) {
                last_section = Some(s);
                if let Some(title) = sections[s].title.clone().filter(|t| !t.is_empty()) {
                    list = list.push(self.section_heading(title));
                }
            }
            let item = &sections[s].items[i];
            let selected = position == page.selected;
            let accessories: Vec<&str> = item
                .accessories
                .iter()
                .filter_map(|a| a.text.as_deref().or(a.tag.as_deref()))
                .collect();
            let subtitle = match (&item.subtitle, accessories.is_empty()) {
                (Some(subtitle), true) => Some(subtitle.clone()),
                (Some(subtitle), false) => {
                    Some(format!("{subtitle}  ·  {}", accessories.join("  ")))
                }
                (None, false) => Some(accessories.join("  ")),
                (None, true) => None,
            };
            let icon = match page.icon(s, i) {
                Some(icon) => self.masked_icon(icon, page.mask(s, i), selected),
                None => self.initial_badge(&item.title, selected),
            };
            let row = self.list_row(
                icon,
                item.title.clone(),
                subtitle.filter(|_| self.subtitles),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::ExtensionItemSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// An extension's grid: each section under its title, its cells in rows
    /// of the section's columns, each cell its content (an image, a colour)
    /// over its title, as `SectionGridModel` lays them out.
    fn extension_grid<'a>(
        &'a self,
        page: &'a crate::extension_page::ExtensionPage,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let sections = page.list().map(|list| &list.sections[..]).unwrap_or(&[]);
        let width = f32::from(self.geometry.card_width) - 24.0;
        let mut grid = column![].spacing(8);
        for (section, positions) in page.grid_groups() {
            if let Some(title) = sections.get(section).and_then(|s| s.title.clone()) {
                grid = grid.push(self.section_heading(title));
            }
            let columns = page.section_columns(section);
            #[allow(clippy::cast_precision_loss)]
            let side = (width / columns as f32 - 8.0).max(16.0);
            for chunk in positions.chunks(columns) {
                let mut cells = row![].spacing(8);
                for &position in chunk {
                    let (s, i) = page.shown[position];
                    let Some(item) = sections.get(s).and_then(|sec| sec.items.get(i)) else {
                        continue;
                    };
                    let selected = position == page.selected;
                    let art: Element<Message> = match page.icon(s, i) {
                        Some(icon) => self.icon_art(icon, page.mask(s, i), selected, side * 0.7),
                        None => Space::new().into(),
                    };
                    let tile = container(art)
                        .width(Length::Fixed(side))
                        .height(Length::Fixed(side))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center)
                        .style(move |_: &Theme| container::Style {
                            background: Some(palette.surface.to_iced().into()),
                            border: Border {
                                color: if selected {
                                    palette.accent.to_iced()
                                } else {
                                    palette.border.to_iced()
                                },
                                width: if selected { 2.0 } else { 1.0 },
                                radius: 8.0.into(),
                            },
                            ..container::Style::default()
                        });
                    let cell = column![
                        tile,
                        text(item.title.clone())
                            .font(self.font())
                            .size(12)
                            .color(palette.text.to_iced())
                            .width(Length::Fixed(side)),
                    ]
                    .spacing(4);
                    let cell: Element<Message> = mouse_area(cell)
                        .on_press(Message::ExtensionItemSelected(position))
                        .into();
                    let cell: Element<Message> = if selected {
                        container(cell).id(crate::scroll::ROOT_SELECTION).into()
                    } else {
                        cell
                    };
                    cells = cells.push(cell);
                }
                grid = grid.push(cells);
            }
        }
        let body = scrollable(container(grid).padding(Padding::new(12.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![body, self.notice(notice)].into(),
            None => body.into(),
        }
    }

    /// Asks the engine whether its catalog moved since this window last
    /// looked.
    fn catalog_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.catalog_generation().await },
            Message::CatalogGeneration,
        )
    }

    /// The engine rescanned applications or extensions: this window scans
    /// its own copy again, as the C++'s one process reloads its root items on
    /// `appsChanged`, and searches again so no row points at a moved entry.
    fn catalog_moved(&mut self, generation: u64) -> Task<Message> {
        if generation == self.catalog_generation {
            return Task::none();
        }
        self.catalog_generation = generation;
        self.app_index.rescan_applications();
        self.app_index.rescan_extensions();
        if matches!(self.page, Page::Root) {
            self.search_task()
        } else {
            self.results.clear();
            Task::none()
        }
    }

    fn open_command(
        &mut self,
        command: &'static compass_core::commands::BuiltinCommand,
    ) -> Task<Message> {
        use compass_core::commands::CommandKind;
        self.panel = None;
        let record = match self.backend.clone() {
            Some(backend) => {
                let key = command.id();
                Task::future(async move {
                    if let Err(error) = backend.record_launch(key).await {
                        tracing::warn!(%error, "could not record opening a command");
                    }
                })
                .discard()
            }
            None => Task::none(),
        };
        match command.kind {
            CommandKind::ClipboardHistory => Task::batch([record, self.open_clipboard_history()]),
            CommandKind::Power(id) => {
                let Some(power) = compass_core::power_commands::command(id) else {
                    return record;
                };
                let asks = self
                    .power_asks
                    .get(power.id)
                    .copied()
                    .unwrap_or(power.confirm_by_default);
                if asks {
                    self.power_confirm = Some(power);
                    return record;
                }
                Task::batch([record, self.run_power_command(power)])
            }
            CommandKind::Media(id) => Task::batch([record, self.run_media(command, id, None)]),
            CommandKind::NowPlaying => Task::batch([record, self.open_now_playing()]),
            CommandKind::ScriptPermissions => Task::batch([record, self.open_script_grants()]),
            CommandKind::SearchTray => Task::batch([record, self.open_search_tray()]),
            CommandKind::OpenSettings => Task::batch([record, self.open_settings(None)]),
            CommandKind::Compass(id) => Task::batch([record, self.open_compass_command(id)]),
            CommandKind::CalculatorHistory => Task::batch([record, self.open_calculator_history()]),
            CommandKind::RefreshExchangeRates => {
                Task::batch([record, self.refresh_exchange_rates(command)])
            }
            CommandKind::BrowseApps => Task::batch([record, self.open_browse_apps()]),
            CommandKind::SetDefaultBrowser => Task::batch([
                record,
                self.open_default_picker(crate::backend::DefaultApp::Browser),
            ]),
            CommandKind::SetDefaultTerminal => Task::batch([
                record,
                self.open_default_picker(crate::backend::DefaultApp::Terminal),
            ]),
            CommandKind::SearchEmojis => Task::batch([record, self.open_emoji_picker()]),
            CommandKind::SearchFiles => {
                Task::batch([record, self.open_search_files(String::new())])
            }
            CommandKind::CreateShortcut => Task::batch([
                record,
                self.open_shortcut_form(compass_core::shortcut_form::Mode::Create, None, false),
            ]),
            CommandKind::ManageShortcuts => Task::batch([record, self.open_manage_shortcuts()]),
            CommandKind::CreateSnippet => Task::batch([
                record,
                self.open_snippet_form(compass_core::shortcut_form::Mode::Create),
            ]),
            CommandKind::ManageSnippets => Task::batch([record, self.open_manage_snippets()]),
            CommandKind::RunProgram => Task::batch([record, self.open_run_program()]),
            CommandKind::SetTheme => Task::batch([record, self.open_set_theme()]),
            CommandKind::CreateExtension => Task::batch([record, self.open_create_extension()]),
            CommandKind::ExtensionStore => {
                self.parked_store = None;
                Task::batch([
                    record,
                    self.open_store_or_intro(crate::backend::Store::Vicinae),
                ])
            }
            CommandKind::RaycastStore => {
                self.parked_store = None;
                Task::batch([
                    record,
                    self.open_store_or_intro(crate::backend::Store::Raycast),
                ])
            }
            CommandKind::BrowseFonts => {
                self.parked_fonts = None;
                Task::batch([record, self.open_browse_fonts()])
            }
            CommandKind::SwitchWindows => {
                self.page = Page::Windows(crate::windows_page::WindowsPage::default());
                Task::batch([record, self.list_windows_task(), focus_search()])
            }
            CommandKind::SwitchWorkspaces => Task::batch([record, self.open_switch_workspaces()]),
            CommandKind::ToggleFullscreen => Task::batch([
                record,
                self.run_window_toggle(crate::backend::WindowToggle::Fullscreen),
            ]),
            CommandKind::ToggleFloating => Task::batch([
                record,
                self.run_window_toggle(crate::backend::WindowToggle::Floating),
            ]),
            CommandKind::ToggleOverview => Task::batch([
                record,
                self.run_window_toggle(crate::backend::WindowToggle::Overview),
            ]),
        }
    }

    /// Opens Search Files with `query` typed, and the remembered category
    /// (`restoreCategoryFilter`: anything but "All").
    fn open_search_files(&mut self, query: String) -> Task<Message> {
        let mut page = crate::files_page::FilesPage::default();
        if let Some(key) = self.view.view_memory.get(crate::view_memory::FILE_CATEGORY) {
            page.set_category(key);
        }
        page.set_query(query);
        self.page = Page::Files(page);
        Task::batch([self.files_query_task(), focus_search()])
    }

    /// Asks Search Files' query: at once, or once its debounce runs out.
    fn files_query_task(&mut self) -> Task<Message> {
        let Page::Files(page) = &self.page else {
            return Task::none();
        };
        let generation = page.generation;
        match crate::files_page::debounce_for(&page.query) {
            Some(delay) => Task::perform(
                async move {
                    tokio::time::sleep(delay).await;
                    generation
                },
                Message::FilesDebounced,
            ),
            None => self.files_search_task(generation),
        }
    }

    /// Sends Search Files' query `generation`, unless the text moved on.
    fn files_search_task(&mut self, generation: u64) -> Task<Message> {
        let Page::Files(page) = &mut self.page else {
            return Task::none();
        };
        if page.generation != generation {
            return Task::none();
        }
        let Some(backend) = self.backend.clone() else {
            page.apply(
                generation,
                Err(
                    "Search Files needs the Compass engine, and this window is running \
                     without one"
                        .to_owned(),
                ),
            );
            return Task::none();
        };
        let query = page.query.clone();
        let category = page.category.clone();
        Task::perform(
            async move { backend.search_files(query, category).await },
            move |result| Message::FilesLoaded { generation, result },
        )
    }

    /// Opens the selected file with its default application, or shows it in
    /// the file browser when `reveal`.
    fn open_selected_file(&mut self, reveal: bool) -> Task<Message> {
        let Page::Files(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(backend)) = (page.selected_row(), self.backend.clone()) else {
            return Task::none();
        };
        let path = row.path.clone();
        Task::perform(
            async move { backend.open_file(path, reveal).await },
            Message::FileOpened,
        )
    }

    /// Asks for clipboard history matching the view's filter.
    fn clipboard_search_task(&mut self) -> Task<Message> {
        let Page::Clipboard(page) = &mut self.page else {
            return Task::none();
        };
        page.generation = page.generation.wrapping_add(1);
        page.notice = None;
        let generation = page.generation;
        let Some(clipboard) = self.clipboard.clone() else {
            page.status = crate::clipboard_page::Status::Failed(
                "Clipboard history needs the Compass engine, and this window is running \
                 without one"
                    .to_owned(),
            );
            return Task::none();
        };
        let (query, kind) = (page.query.clone(), page.kind);
        Task::perform(
            async move {
                clipboard
                    .clipboard_history_of_kind(query, crate::clipboard_page::PAGE_SIZE, kind)
                    .await
            },
            move |result| Message::ClipboardLoaded { generation, result },
        )
    }

    /// Runs what the open question asked about, once Enter confirms it.
    fn run_confirmed(&mut self) -> Task<Message> {
        let Some(confirm) = self.confirm.take() else {
            return Task::none();
        };
        let task = match confirm.action {
            ConfirmAction::ClipboardRemoveAll => self.remove_all_clipboard_entries(),
            ConfirmAction::RootEdit(id, edit) => self.edit_root_item(id, edit),
            ConfirmAction::UninstallExtension(id) => self.uninstall_extension(id),
            ConfirmAction::RemoveTokenSet(extension, provider) => {
                self.remove_token_set(extension, provider)
            }
        };
        Task::batch([task, focus_search()])
    }

    /// Asks the engine for the open windows.
    fn list_windows_task(&mut self) -> Task<Message> {
        let Page::Windows(page) = &mut self.page else {
            return Task::none();
        };
        page.notice = None;
        let Some(windows) = self.windows.clone() else {
            page.apply(
                Err(
                    "Window switching needs the Compass engine, and this window is running \
                     without one"
                        .to_owned(),
                ),
                std::process::id(),
            );
            return Task::none();
        };
        Task::perform(
            async move { windows.list_windows().await },
            Message::WindowsLoaded,
        )
    }

    /// Switches to the selected window.
    fn activate_selected_window(&mut self) -> Task<Message> {
        let Page::Windows(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(windows)) = (page.selected_row(), self.windows.clone()) else {
            return Task::none();
        };
        let id = row.id;
        Task::perform(
            async move { windows.activate_window(id).await },
            Message::WindowActivated,
        )
    }

    /// Closes the selected window, when it can be closed.
    fn close_selected_window(&mut self) -> Task<Message> {
        let Page::Windows(page) = &mut self.page else {
            return Task::none();
        };
        let Some(row) = page.selected_row() else {
            return Task::none();
        };
        if !row.can_close {
            page.notice = Some(format!("{} cannot be closed", row.title));
            return Task::none();
        }
        let Some(windows) = self.windows.clone() else {
            return Task::none();
        };
        let id = row.id;
        Task::perform(
            async move { windows.close_window(id).await },
            Message::ShellWindowClosed,
        )
    }

    /// Fetches the selected entry's content, to copy it.
    fn change_selected_clipboard_entry(&mut self, change: ClipboardChange) -> Task<Message> {
        let Page::Clipboard(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(clipboard)) = (page.selected_row(), self.clipboard.clone()) else {
            return Task::none();
        };
        let (id, pinned) = (row.id.clone(), row.pinned);
        Task::perform(
            async move {
                match change {
                    ClipboardChange::TogglePin => clipboard.clipboard_set_pinned(id, !pinned).await,
                    ClipboardChange::Remove => clipboard.clipboard_remove(id).await,
                }
            },
            Message::ClipboardEntryChanged,
        )
    }

    /// Paste where the engine can, and copy where it cannot: the paste needs
    /// the GNOME Shell extension, and without it the entry still goes on the
    /// clipboard for the user's own Ctrl+V.
    fn paste_selected_clipboard_entry(&mut self) -> Task<Message> {
        let Page::Clipboard(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(clipboard)) = (page.selected_row(), self.clipboard.clone()) else {
            return Task::none();
        };
        let id = row.id.clone();
        Task::perform(
            async move { clipboard.clipboard_paste(id).await },
            Message::ClipboardPasted,
        )
    }

    fn copy_selected_clipboard_entry(&mut self) -> Task<Message> {
        let Page::Clipboard(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(clipboard)) = (page.selected_row(), self.clipboard.clone()) else {
            return Task::none();
        };
        let id = row.id.clone();
        Task::perform(
            async move { clipboard.clipboard_content(id).await },
            Message::ClipboardContentLoaded,
        )
    }

    /// Re-rank locally when no daemon backend is attached.
    fn search(&mut self) {
        let options = compass_core::root_items::SearchOptions {
            provider_id: self.provider_scope.as_ref().map(|scope| scope.id.clone()),
            ..compass_core::root_items::SearchOptions::default()
        };
        self.favorites_len = 0;
        if self.query.trim().is_empty() && options.provider_id.is_none() {
            self.results.clear();
            self.selected = 0;
            self.apply_favorites();
            self.apply_update();
            return;
        }

        self.results = self
            .app_index
            .search_root_with(&self.query, None, &options)
            .into_iter()
            .map(|hit| match hit {
                compass_core::RootHit::App(app) => RootRow::App(app.index),
                compass_core::RootHit::Command { command, .. } => RootRow::Command(command),
                compass_core::RootHit::Extension { command, .. } => RootRow::Extension(
                    self.app_index
                        .extensions()
                        .iter()
                        .position(|known| known.id == command.id)
                        .unwrap_or_default(),
                ),
                compass_core::RootHit::Script { script, .. } => RootRow::Script(
                    self.app_index
                        .scripts()
                        .iter()
                        .position(|known| known.id == script.id)
                        .unwrap_or_default(),
                ),
                compass_core::RootHit::RhaiScript { script, .. } => RootRow::RhaiScript(
                    self.app_index
                        .rhai_scripts()
                        .iter()
                        .position(|known| known.id == script.id)
                        .unwrap_or_default(),
                ),
                compass_core::RootHit::Shortcut { shortcut, .. } => RootRow::Shortcut(
                    self.app_index
                        .shortcuts()
                        .iter()
                        .position(|known| known.id == shortcut.id)
                        .unwrap_or_default(),
                ),
            })
            .collect();
        // Back to the top on every new query: the old selection pointed into a
        // different list, and keeping its position would silently select an
        // unrelated application.
        self.selected = 0;
        if self.provider_scope.is_none() {
            self.apply_calculator();
            self.apply_fallbacks();
        }
        self.warm_icons();
    }

    /// Offers the fallbacks under the results for a non-empty query, as
    /// `RootFallbackSection` does.
    fn apply_fallbacks(&mut self) {
        self.results
            .retain(|row| !matches!(row, RootRow::Fallback(_)));
        if self.query.trim().is_empty() || self.provider_scope.is_some() {
            return;
        }
        // An item the search already lists is not offered a second time
        // under the fallbacks ("docs" finds Search Files by its keywords).
        let listed = |fallback: &Fallback| {
            self.results.iter().any(|row| match (row, fallback) {
                (RootRow::Command(listed), Fallback::Command(offered)) => {
                    listed.entrypoint == offered.entrypoint
                }
                (RootRow::Extension(listed), Fallback::Extension(offered))
                | (RootRow::Shortcut(listed), Fallback::Shortcut(offered)) => listed == offered,
                _ => false,
            })
        };
        let fallbacks: Vec<RootRow> = self
            .fallbacks
            .iter()
            .filter_map(|id| self.resolve_fallback(id))
            .filter(|fallback| !listed(fallback))
            .map(RootRow::Fallback)
            .collect();
        self.results.extend(fallbacks);
    }

    /// The fallback a `fallbacks` entry names here, when it names an item
    /// that can be one (`isSuitableForFallback`): Search Files, any
    /// extension command, or a quicklink with exactly one argument.
    fn resolve_fallback(&self, id: &str) -> Option<Fallback> {
        if let Some(command) = compass_core::commands::fallback(id) {
            return Some(Fallback::Command(command));
        }
        if let Some(index) = self
            .app_index
            .extensions()
            .iter()
            .position(|command| command.id == id)
        {
            return Some(Fallback::Extension(index));
        }
        let shortcut = self.app_index.shortcut_by_entrypoint(id)?;
        if shortcut.link.arguments.len() != 1 {
            return None;
        }
        self.app_index
            .shortcuts()
            .iter()
            .position(|known| known.id == shortcut.id)
            .map(Fallback::Shortcut)
    }

    /// What a fallback row says.
    fn fallback_title(&self, fallback: Fallback) -> Option<String> {
        Some(match fallback {
            Fallback::Command(command) => command.title.to_owned(),
            Fallback::Extension(index) => self.app_index.extensions().get(index)?.title.clone(),
            Fallback::Shortcut(index) => {
                crate::shortcuts_page::display_name(self.app_index.shortcuts().get(index)?)
                    .to_owned()
            }
        })
    }

    /// Runs a fallback with the query: Search Files opens searching for it,
    /// an extension command opens with it as its fallback text
    /// (`OpenBuiltinCommandAction::setForwardSearchText`), and a quicklink
    /// opens with it as its argument (`OpenShortcutFromSearchText`).
    fn open_fallback(&mut self, fallback: Fallback) -> Task<Message> {
        use compass_core::commands::CommandKind;
        self.panel = None;
        let query = self.query.clone();
        match fallback {
            Fallback::Command(command) => match command.kind {
                CommandKind::SearchFiles => self.open_search_files(query),
                _ => self.open_command(command),
            },
            Fallback::Extension(index) => self.run_extension_fallback(index, query),
            Fallback::Shortcut(index) => {
                let Some(shortcut) = self.app_index.shortcuts().get(index) else {
                    return Task::none();
                };
                let id = shortcut.id.clone();
                self.send_open_shortcut(id, vec![query])
            }
        }
    }

    /// Puts the calculator's answer to the query first, when there is one.
    fn apply_calculator(&mut self) {
        self.results.retain(|row| *row != RootRow::Calculator);
        self.calculator = compass_core::calculator::evaluate(&self.query, !self.results.is_empty());
        if self.calculator.is_some() {
            self.results.insert(0, RootRow::Calculator);
        }
    }

    /// What goes in a row's icon slot: resolved art, or nothing for the initial.
    ///
    /// The three ways to get nothing -- icons off, an entry with no `Icon=`,
    /// and a name the theme could not resolve -- are deliberately one answer.
    /// They all mean the same thing to the row, and collapsing them here is
    /// what keeps `view` from having to know the difference.
    fn row_art(&self, item: &AppItem) -> Option<&crate::icons::IconArt> {
        if !self.icons {
            return None;
        }
        item.icon().and_then(|name| self.icon_cache.cached(name))
    }

    /// A root row's `ImageURL` drawn in its icon slot, once warmed, else
    /// `title`'s initial.
    fn url_icon(
        &self,
        url: Option<&compass_core::image_url::ImageUrl>,
        title: &str,
        selected: bool,
    ) -> Element<'_, Message> {
        let glyph = url
            .filter(|_| self.icons)
            .and_then(|url| self.url_glyphs.get(&url.to_url())?.as_ref());
        self.glyph_or_initial(glyph, title, selected)
    }

    /// An extension command's icon URL, once warmed.
    fn extension_url(
        &self,
        command: &compass_core::extension_commands::ExtensionCommand,
    ) -> Option<compass_core::image_url::ImageUrl> {
        self.icons
            .then(|| command.icon_url(|path| self.view.known_files.contains(path)))
    }

    /// Resolves `urls` into [`Self::url_glyphs`], asking for each remote
    /// image not yet fetched (when [`AppFlags::remote_icons`] allows).
    fn warm_urls(&mut self, urls: Vec<compass_core::image_url::ImageUrl>) {
        let find = self.icon_lookup.clone();
        let find = |name: &str| find.find(name);
        for url in urls {
            let key = url.to_url();
            if self.url_glyphs.contains_key(&key) {
                continue;
            }
            let remote_files = &self.remote_files;
            let remote = |remote: &str| remote_files.get(remote).cloned();
            let lookup = crate::icons::UrlLookup {
                find: &find,
                remote: &remote,
                favicon: self.favicon_service,
                accent: self.palette().accent,
            };
            let glyph = crate::icons::url_glyph(&url, &lookup);
            let waiting = crate::icons::remote_source(&url, self.favicon_service)
                .filter(|remote| !self.remote_files.contains_key(remote));
            if let Some(remote) = waiting {
                if self.remote_icons && self.remote_requested.insert(remote.clone()) {
                    self.remote_pending.push(remote);
                }
                if glyph.is_none() {
                    // Resolved again when it arrives.
                    continue;
                }
            }
            self.url_glyphs.insert(key, glyph);
        }
    }

    /// A remote root icon arrived in the cache, or could not be fetched.
    fn root_icon_arrived(&mut self, url: &str, result: &Result<std::path::PathBuf, String>) {
        match result {
            Ok(path) => {
                self.remote_files.insert(url.to_owned(), path.clone());
                // Whatever waited on it (and its fallbacks) is resolved again.
                self.url_glyphs.clear();
                self.warm_icons();
                self.warm_clipboard_icons();
            }
            Err(reason) => tracing::debug!(%url, %reason, "a root icon was not fetched"),
        }
    }

    /// Resolve the favicons of clipboard history's link rows.
    fn warm_clipboard_icons(&mut self) {
        if !self.icons {
            return;
        }
        let Page::Clipboard(page) = &self.page else {
            return;
        };
        let urls: Vec<_> = page.rows.iter().filter_map(clipboard_url).collect();
        self.warm_urls(urls);
    }

    /// Resolve the icons of the rows now on screen.
    ///
    /// Here rather than in `view` because resolution walks the theme
    /// directories, and a launcher re-ranks on every keystroke: doing disk work
    /// per draw is the cost #85 warns about. Bounded to the current results, so
    /// it is the size of the visible list rather than of the index, and the
    /// cache makes each name at most one walk for the life of the process.
    ///
    /// A no-op when icons are off, so the default configuration does no lookup
    /// at all.
    pub(super) fn warm_icons(&mut self) {
        if !self.icons {
            return;
        }

        let names: Vec<&str> = self
            .results
            .iter()
            .filter_map(|row| match row {
                RootRow::App(index) => self.app_index.items().get(*index),
                RootRow::Command(_)
                | RootRow::Extension(_)
                | RootRow::Shortcut(_)
                | RootRow::Script(_)
                | RootRow::RhaiScript(_)
                | RootRow::Fallback(_)
                | RootRow::Calculator
                | RootRow::Update => None,
            })
            .filter_map(AppItem::icon)
            .collect();
        let find = self.icon_lookup.clone();
        self.icon_cache.warm(names, &|name| find.find(name));

        // Extension, script and shortcut rows: their `ImageURL`s.
        let mut urls = Vec::new();
        for row in &self.results {
            match row {
                RootRow::Extension(index) => {
                    if let Some(command) = self.app_index.extensions().get(*index) {
                        for icon in [
                            command.icon.as_deref(),
                            Some(command.extension_icon.as_str()),
                        ]
                        .into_iter()
                        .flatten()
                        .filter(|icon| !icon.is_empty())
                        {
                            let path = command.extension_dir.join("assets").join(icon);
                            if path.is_file() {
                                self.view.known_files.insert(path);
                            }
                        }
                        urls.push(command.icon_url(|path| self.view.known_files.contains(path)));
                    }
                }
                RootRow::Script(index) => {
                    if let Some(url) = self
                        .app_index
                        .scripts()
                        .get(*index)
                        .and_then(|script| self.script_icons.get(&script.id))
                    {
                        urls.push(url.clone());
                    }
                }
                RootRow::Shortcut(index) => {
                    if let Some(shortcut) = self.app_index.shortcuts().get(*index) {
                        urls.push(shortcut_url(&shortcut.icon));
                    }
                }
                _ => {}
            }
        }
        self.warm_urls(urls);
    }

    /// Resolve the file-type icons of Search Files' rows.
    fn warm_file_icons(&mut self) {
        if !self.icons {
            return;
        }
        let Page::Files(page) = &self.page else {
            return;
        };
        let find = self.icon_lookup.clone();
        self.file_glyphs
            .warm(page.rows.iter().map(|file| file.path.as_str()), &|name| {
                find.find(name)
            });
    }

    /// Resolve the icons of the windows' applications.
    fn warm_window_icons(&mut self) {
        if !self.icons {
            return;
        }
        let Page::Windows(page) = &self.page else {
            return;
        };
        let names: Vec<String> = page
            .all
            .iter()
            .filter_map(|window| self.window_app(window)?.icon().map(str::to_owned))
            .collect();
        let find = self.icon_lookup.clone();
        self.icon_cache
            .warm(names.iter().map(String::as_str), &|name| find.find(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// An index over three entries written to a tempdir, so the tests do not
    /// depend on what the machine running them has installed.
    fn index(dir: &std::path::Path) -> AppIndex {
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

    fn app(dir: &std::path::Path) -> LauncherApp {
        LauncherApp::with_index(index(dir))
    }

    #[test]
    fn a_moved_catalog_generation_rescans_and_the_same_one_does_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        fs::write(
            dir.path().join("zephyr.desktop"),
            "[Desktop Entry]\nType=Application\nName=Zephyr\nExec=/bin/true\n",
        )
        .expect("write entry");
        let _ = app.update(Message::CatalogGeneration(Ok(0)));
        assert!(app.app_index.get("zephyr.desktop").is_none(), "unchanged");

        let _ = app.update(Message::CatalogGeneration(Ok(1)));
        assert!(app.app_index.get("zephyr.desktop").is_some());
        let _ = app.update(Message::QueryChanged("zeph".into()));
        assert_eq!(
            app.selected_item().map(AppItem::key),
            Some("zephyr.desktop")
        );

        fs::remove_file(dir.path().join("zephyr.desktop")).expect("remove");
        let _ = app.update(Message::CatalogGeneration(Ok(1)));
        assert!(
            app.app_index.get("zephyr.desktop").is_some(),
            "same generation"
        );
        let _ = app.update(Message::CatalogGeneration(Err("no engine".into())));
        let _ = app.update(Message::CatalogGeneration(Ok(2)));
        assert!(app.app_index.get("zephyr.desktop").is_none());
    }

    #[test]
    fn pending_window_is_reused_and_escape_dismisses_after_it_opens() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
        app.link = Some(EngineLink::new(receiver, sender));
        let _ = app.open_window();
        let id = app.pending_window.unwrap();
        let _ = app.update(Message::Command(UiCommand::Show));
        assert_eq!(app.pending_window, Some(id));
        assert!(outcomes.try_recv().is_err(), "opening is not yet shown");
        let _ = app.update(Message::Opened(id));
        assert_eq!(outcomes.try_recv().unwrap(), UiOutcome::Shown);
        let close = app.update(pressed(iced::keyboard::key::Named::Escape));
        {
            use iced::futures::{StreamExt, executor::block_on};
            use iced_winit::runtime::{Action, task, window as runtime_window};
            let mut stream = task::into_stream(close).expect("Escape must request a close");
            assert!(matches!(
                block_on(stream.next()),
                Some(Action::Window(runtime_window::Action::Close(closed))) if closed == id
            ));
        }
        let _ = app.update(Message::Closed(id));
        assert!(!app.is_visible());
        assert!(app.pending_window.is_none());
    }

    #[test]
    fn only_app_grid_sessions_exit_when_the_engine_disconnects() {
        use iced::futures::{StreamExt, executor::block_on};
        use iced_winit::runtime::{Action, task};
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        assert!(task::into_stream(app.update(Message::EngineDisconnected)).is_none());
        app.apply(AppFlags {
            exit_on_engine_disconnect: true,
            ..AppFlags::default()
        });
        let mut exit = task::into_stream(app.update(Message::EngineDisconnected)).unwrap();
        assert!(matches!(block_on(exit.next()), Some(Action::Exit)));
    }

    #[test]
    fn a_due_onboarding_started_hidden_waits_for_the_first_summon() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compass/onboarding.json");
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, _outcomes) = tokio::sync::mpsc::unbounded_channel();
        let (mut app, _) = LauncherApp::boot(AppFlags {
            start_hidden: true,
            link: Some(EngineLink::new(receiver, sender)),
            onboarding: Some(path.clone()),
            ..AppFlags::default()
        });
        assert!(app.pending_window.is_none(), "login stays quiet");
        assert!(!app.showing_onboarding());

        let _ = app.update(Message::Command(UiCommand::Toggle));
        assert!(app.pending_window.is_some(), "the first summon opens it");
        assert_eq!(
            app.onboarding_step(),
            Some(compass_core::onboarding::Step::Welcome)
        );

        // Dismissed with the hotkey: seen, and not shown at the next login.
        let _ = app.update(Message::Command(UiCommand::Toggle));
        assert!(!app.showing_onboarding());
        assert!(!compass_core::onboarding::should_show(&path, false));
        let _ = app.update(Message::Command(UiCommand::Toggle));
        assert!(!app.showing_onboarding(), "once is enough");

        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, _outcomes) = tokio::sync::mpsc::unbounded_channel();
        let (app, _) = LauncherApp::boot(AppFlags {
            start_hidden: false,
            link: Some(EngineLink::new(receiver, sender)),
            onboarding: Some(dir.path().join("other/onboarding.json")),
            ..AppFlags::default()
        });
        assert!(
            app.pending_window.is_some(),
            "started shown, it opens at once"
        );
        assert_eq!(
            app.onboarding_step(),
            Some(compass_core::onboarding::Step::Welcome)
        );

        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, _outcomes) = tokio::sync::mpsc::unbounded_channel();
        let (app, _) = LauncherApp::boot(AppFlags {
            start_hidden: true,
            link: Some(EngineLink::new(receiver, sender)),
            onboarding: None,
            ..AppFlags::default()
        });
        assert!(app.pending_window.is_none(), "not due: hidden as asked");
        assert!(!app.showing_onboarding());
    }

    #[test]
    fn finishing_the_onboarding_records_it_and_hides() {
        use compass_core::onboarding::{self, Step};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compass").join(onboarding::FILE_NAME);
        let backend = Arc::new(TestBackend::default());
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(backend.clone());
        app.open_onboarding(path.clone());

        let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert_eq!(app.onboarding_step(), Some(Step::Personalize));
        let task = app.update(Message::OnboardingTheme(
            crate::onboarding_page::ThemeOption(crate::theme::Theme::Dracula),
        ));
        settle(&mut app, task);
        assert_eq!(
            app.view.theme_choice,
            crate::theme::Theme::Dracula,
            "previewed"
        );
        assert_eq!(
            backend.themes_kept.lock().unwrap().as_slice(),
            ["dracula"],
            "and kept"
        );
        let task = app.update(Message::OnboardingOpen(onboarding::HOTKEY_DOCS_URL));
        settle(&mut app, task);
        assert_eq!(
            backend.opened_urls.lock().unwrap().as_slice(),
            [onboarding::HOTKEY_DOCS_URL]
        );

        let _ = app.update(Message::OnboardingContinue);
        assert_eq!(app.onboarding_step(), Some(Step::Extensions));
        let _ = app.update(Message::OnboardingContinue);
        assert_eq!(app.onboarding_step(), Some(Step::Complete));
        assert!(onboarding::should_show(&path, false), "not before Finish");
        let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(!app.showing_onboarding(), "Finish closes the flow");
        assert!(
            matches!(app.page, Page::Root),
            "and leaves the launcher open at its search"
        );
        assert!(!onboarding::should_show(&path, false), "and records it");
    }

    #[test]
    fn escape_clears_the_search_text_before_it_hides() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let _ = app.update(Message::QueryChanged("fire".into()));
        app.window_focused = true;
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert_eq!(app.query, "", "the first Escape clears the text");
        assert!(app.window_focused, "and does not hide");
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(!app.window_focused, "the second hides");
    }

    #[test]
    fn escape_closes_the_onboarding_and_records_it_as_seen() {
        use compass_core::onboarding;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(onboarding::FILE_NAME);
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.open_onboarding(path.clone());
        let _ = app.update(Message::OnboardingJump(3));
        assert_eq!(
            app.onboarding_step(),
            Some(onboarding::Step::Complete),
            "a dot goes straight to its step"
        );
        let _ = app.update(Message::OnboardingBack);
        assert_eq!(app.onboarding_step(), Some(onboarding::Step::Extensions));
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert_eq!(
            app.onboarding_step(),
            Some(onboarding::Step::Personalize),
            "Escape goes back a step"
        );
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert_eq!(app.onboarding_step(), Some(onboarding::Step::Welcome));
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(!app.showing_onboarding(), "and closes from the first");
        assert!(
            !onboarding::should_show(&path, false),
            "not asked again at every login"
        );
    }

    #[test]
    fn the_onboarding_is_worked_from_the_keyboard() {
        use crate::onboarding_page::Control;
        use compass_core::onboarding::Step;
        use iced::keyboard::key::Named;
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.open_onboarding(dir.path().join("onboarding.json"));
        let focused = |app: &LauncherApp| match &app.page {
            Page::Onboarding(page) => page.focused,
            _ => panic!("not the onboarding"),
        };
        let shift_tab = || {
            Message::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(Named::Tab),
                modified_key: iced::keyboard::Key::Named(Named::Tab),
                physical_key: iced::keyboard::key::Physical::Unidentified(
                    iced::keyboard::key::NativeCode::Unidentified,
                ),
                location: iced::keyboard::Location::Standard,
                modifiers: iced::keyboard::Modifiers::SHIFT,
                text: None,
                repeat: false,
            })
        };
        let _ = app.update(pressed(Named::Tab));
        assert_eq!(focused(&app), Some(Control::Continue));
        let _ = app.update(pressed(Named::Space));
        assert_eq!(app.onboarding_step(), Some(Step::Personalize));
        assert_eq!(focused(&app), None, "a new step starts unfocused");

        // The theme dropdown, from the keyboard.
        let _ = app.update(pressed(Named::Tab));
        assert_eq!(focused(&app), Some(Control::Theme));
        let _ = app.update(pressed(Named::Enter));
        let Page::Onboarding(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.menu, Some(0), "open at the current theme");
        let _ = app.update(pressed(Named::ArrowDown));
        let _ = app.update(pressed(Named::Enter));
        assert_eq!(app.view.theme_choice, crate::theme::Theme::ALL[1]);
        let _ = app.update(shift_tab());
        assert_eq!(focused(&app), None, "Shift+Tab from the first control");

        // Install is reached and pressed; Back and Continue close the walk.
        let _ = app.update(pressed(Named::Enter));
        assert_eq!(app.onboarding_step(), Some(Step::Extensions));
        let _ = app.update(pressed(Named::Tab));
        assert_eq!(focused(&app), Some(Control::Install(0)));
        let _ = app.update(pressed(Named::Space));
        let Page::Onboarding(page) = &app.page else {
            unreachable!()
        };
        assert_ne!(
            page.extensions.state(0),
            Some(compass_core::onboarding::Install::Available),
            "Install was pressed"
        );
        for _ in 0..10 {
            if focused(&app) == Some(Control::Back) {
                break;
            }
            let _ = app.update(pressed(Named::Tab));
        }
        assert_eq!(focused(&app), Some(Control::Back));
        let _ = app.update(pressed(Named::Enter));
        assert_eq!(app.onboarding_step(), Some(Step::Personalize));
    }

    #[test]
    fn the_root_footer_names_enter_and_the_action_panel() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        let _ = app.update(Message::QueryChanged("fire".into()));
        assert_eq!(app.root_primary_action(), Some("Open"));
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(800.0, 600.0),
            app.view(),
        );
        for label in ["Open", "Actions", "Ctrl+B"] {
            assert!(ui.find(label).is_ok(), "the footer has no {label:?}");
        }
        drop(ui);
        let _ = app.update(Message::QueryChanged("zzzzqqq".into()));
        assert!(
            app.selected_row().is_none() || app.root_primary_action().is_some(),
            "a row selected always says what Enter does"
        );
    }

    #[test]
    fn every_onboarding_step_draws_its_heading_and_buttons() {
        use compass_core::onboarding::Step;
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.open_onboarding(dir.path().join("onboarding.json"));
        for (step, expected) in [
            (Step::Welcome, ["Welcome to Compass", "Continue"]),
            (Step::Personalize, ["Make it your own", "Open Docs"]),
            (Step::Extensions, ["Add extensions", "Install"]),
            (Step::Complete, ["Setup complete", "Finish"]),
        ] {
            assert_eq!(app.onboarding_step(), Some(step));
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(800.0, 600.0),
                app.view(),
            );
            for label in expected {
                assert!(ui.find(label).is_ok(), "{step:?} has no {label:?}");
            }
            drop(ui);
            let _ = app.update(Message::OnboardingContinue);
        }
    }

    /// Clicks `label` on the onboarding card, as a pointer would, and runs
    /// what the click sends to the end.
    fn click_onboarding(app: &mut LauncherApp, label: &str) {
        let messages: Vec<_> = {
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(800.0, 600.0),
                app.view(),
            );
            ui.click(label)
                .unwrap_or_else(|_| panic!("{label:?} is not on the card"));
            ui.into_messages().collect()
        };
        for message in messages {
            let task = app.update(message);
            settle(app, task);
        }
    }

    fn onboarding_page(app: &LauncherApp) -> &crate::onboarding_page::OnboardingPage {
        match &app.page {
            Page::Onboarding(page) => page,
            _ => panic!("the flow is not on screen"),
        }
    }

    /// Every button of the flow, clicked: each opens its URL or does its
    /// action, and none leads to upstream Vicinae.
    #[test]
    fn every_onboarding_button_does_what_it_says() {
        use compass_core::onboarding::{self, Install, RECOMMENDED_EXTENSIONS, Step};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compass").join(onboarding::FILE_NAME);
        let first = &RECOMMENDED_EXTENSIONS[0];
        let backend = Arc::new(TestBackend {
            store_rows: std::sync::Mutex::new(vec![crate::backend::StoreRow {
                id: first.id(),
                name: first.name.into(),
                author: first.owner.into(),
                owner: first.owner.into(),
                title: first.title.into(),
                ..crate::backend::StoreRow::default()
            }]),
            ..TestBackend::default()
        });
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(backend.clone());
        app.open_onboarding(path.clone());

        click_onboarding(&mut app, "Continue");
        assert_eq!(app.onboarding_step(), Some(Step::Personalize));
        click_onboarding(&mut app, "Open Docs");
        assert_eq!(
            backend.opened_urls.lock().unwrap().as_slice(),
            ["https://tunaos.org/docs/compass/getting-started#set-a-keyboard-shortcut"]
        );
        assert_eq!(
            onboarding_page(&app).notice.as_deref(),
            Some("Opened in browser"),
            "the click is answered on the card"
        );
        assert!(app.showing_onboarding(), "and the flow stays up");
        click_onboarding(&mut app, "Back");
        assert_eq!(
            onboarding_page(&app).notice,
            None,
            "a step's notice stays on it"
        );
        assert_eq!(app.onboarding_step(), Some(Step::Welcome));
        click_onboarding(&mut app, "Continue");
        click_onboarding(&mut app, "Continue");
        assert_eq!(app.onboarding_step(), Some(Step::Extensions));

        click_onboarding(&mut app, "Install");
        assert_eq!(
            onboarding_page(&app).extensions.state(0),
            Some(Install::Installed),
            "the first Install installs the first recommendation"
        );
        assert_eq!(
            backend.installs.lock().unwrap().first(),
            Some(&(
                crate::backend::Store::Raycast,
                "gebeto".to_owned(),
                "translate".to_owned()
            )),
            "from the Raycast store, by its author and name"
        );
        assert!(
            backend.store_rows.lock().unwrap()[0].installed,
            "through the store's own install call"
        );
        click_onboarding(&mut app, "Install");
        let page = onboarding_page(&app);
        assert_eq!(page.extensions.state(1), Some(Install::Failed));
        let notice = page.extensions.notice().expect("the failure is said");
        assert!(
            notice.starts_with(&format!(
                "Could not install {}: ",
                RECOMMENDED_EXTENSIONS[1].title
            )) && notice.ends_with("install it later from the Extension Store."),
            "{notice}"
        );
        {
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(800.0, 600.0),
                app.view(),
            );
            assert!(ui.find(notice).is_ok(), "and shown on the card");
            assert!(ui.find("Try Again").is_ok());
            assert!(ui.find("Installed").is_ok());
        }
        click_onboarding(&mut app, "Continue");
        assert_eq!(
            app.onboarding_step(),
            Some(Step::Complete),
            "a failure never blocks"
        );

        {
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(800.0, 600.0),
                app.view(),
            );
            assert!(ui.find("Compass is open source software.").is_ok());
            assert!(ui.find("Sponsor").is_err(), "Compass has no sponsor page");
        }
        click_onboarding(&mut app, "GitHub");
        assert_eq!(
            backend
                .opened_urls
                .lock()
                .unwrap()
                .last()
                .map(String::as_str),
            Some("https://github.com/tuna-os/compass")
        );
        assert_eq!(
            onboarding_page(&app).notice.as_deref(),
            Some("Opened in browser")
        );
        assert!(
            backend
                .opened_urls
                .lock()
                .unwrap()
                .iter()
                .all(|url| !url.contains("vicinae")),
            "no button leads upstream"
        );
        click_onboarding(&mut app, "Finish");
        assert!(!app.showing_onboarding());
        assert!(!onboarding::should_show(&path, false), "Finish records it");
    }

    /// A link that cannot open says why on the card: without the engine, and
    /// when the engine could not start a browser.
    #[test]
    fn an_onboarding_link_that_cannot_open_says_why() {
        use compass_core::onboarding::Step;
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.open_onboarding(dir.path().join("onboarding.json"));
        let _ = app.update(Message::OnboardingJump(3));
        assert_eq!(app.onboarding_step(), Some(Step::Complete));
        click_onboarding(&mut app, "GitHub");
        let notice = onboarding_page(&app).notice.clone();
        assert_eq!(
            notice.as_deref(),
            Some("Opening a link needs the Compass engine")
        );
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(800.0, 600.0),
            app.view(),
        );
        assert!(
            ui.find(notice.unwrap().as_str()).is_ok(),
            "shown on the card"
        );
        drop(ui);

        let refusing = Arc::new(TestBackend {
            refuse_opens: Some("No application opens https links".into()),
            ..TestBackend::default()
        });
        app.backend = Some(refusing.clone());
        click_onboarding(&mut app, "GitHub");
        assert_eq!(
            onboarding_page(&app).notice.as_deref(),
            Some("No application opens https links")
        );
        assert_eq!(refusing.opened_urls.lock().unwrap().len(), 1);
        assert!(app.showing_onboarding());
    }

    /// The theme picked on "Make it your own" is the one the page is drawn
    /// in, kept by the engine or not.
    #[test]
    fn the_onboarding_theme_picker_shows_the_choice() {
        use crate::theme::Theme;
        let dir = tempfile::tempdir().unwrap();
        for backend in [None, Some(Arc::new(TestBackend::default()))] {
            let mut app = LauncherApp::with_index(index(dir.path()));
            app.backend = backend
                .clone()
                .map(|b| b as Arc<dyn crate::backend::ApplicationBackend>);
            app.open_onboarding(dir.path().join("onboarding.json"));
            let _ = app.update(Message::OnboardingJump(1));
            let task = app.update(Message::OnboardingTheme(
                crate::onboarding_page::ThemeOption(Theme::Nord),
            ));
            settle(&mut app, task);
            assert_eq!(app.view.theme_choice, Theme::Nord);
            // The pick list draws its value without a text widget a selector
            // finds; the paint tier checks the card is drawn in the theme.
            assert_eq!(app.palette(), Theme::Nord.palette(app.view.appearance));
            if let Some(backend) = backend {
                assert_eq!(backend.themes_kept.lock().unwrap().as_slice(), ["nord"]);
                assert_eq!(app.view.theme_preview, None, "kept, so nothing to put back");
            }
            assert_eq!(onboarding_page(&app).notice, None);
        }
    }

    /// Without the engine, Install says why and the flow goes on.
    #[test]
    fn installing_without_the_engine_says_why_and_goes_on() {
        use compass_core::onboarding::{Install, Step};
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.open_onboarding(dir.path().join("onboarding.json"));
        let _ = app.update(Message::OnboardingJump(2));
        assert_eq!(app.onboarding_step(), Some(Step::Extensions));
        click_onboarding(&mut app, "Install");
        let page = onboarding_page(&app);
        assert_eq!(page.extensions.state(0), Some(Install::Failed));
        assert!(
            page.extensions
                .notice()
                .is_some_and(|notice| notice.contains("needs the Compass engine"))
        );
        click_onboarding(&mut app, "Continue");
        assert_eq!(app.onboarding_step(), Some(Step::Complete));
    }

    #[test]
    fn hidden_boot_requires_an_engine_and_waits_for_activation() {
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, _outcomes) = tokio::sync::mpsc::unbounded_channel();
        let (mut hidden, _) = LauncherApp::boot(AppFlags {
            start_hidden: true,
            link: Some(EngineLink::new(receiver, sender)),
            ..AppFlags::default()
        });
        assert!(hidden.window.is_none());
        assert!(
            hidden.pending_window.is_none(),
            "hidden boot must never allocate a surface"
        );
        let _ = hidden.update(Message::Command(UiCommand::Show));
        assert!(
            hidden.pending_window.is_some(),
            "the engine can summon the hidden session"
        );

        let (standalone, _) = LauncherApp::boot(AppFlags {
            start_hidden: true,
            ..AppFlags::default()
        });
        assert!(
            standalone.pending_window.is_some(),
            "never strand an undriven UI invisibly"
        );
    }

    #[test]
    fn a_copied_answer_is_remembered_and_calculator_history_lists_pins_and_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());

        // Copying the root list's answer keeps the calculation.
        app.query = "5 ft to m".into();
        app.search();
        assert_eq!(app.selected_row(), Some(RootRow::Calculator));
        let task = app.update(Message::LaunchSelected);
        let writes = settle(&mut app, task);
        assert_eq!(
            writes,
            ["1.524"],
            "the value, as the C++ copies answer.text"
        );
        let kept = backend.calculations.lock().unwrap().clone();
        assert_eq!(kept.len(), 1);
        assert_eq!(
            (kept[0].question.as_str(), kept[0].conversion),
            ("5 ft to m", true)
        );

        let _ = app.update(Message::Command(UiCommand::Show));
        app.query = "calculator history".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let Page::Calculator(page) = &app.page else {
            panic!("not on Calculator History: {}", app.state_line());
        };
        assert_eq!(page.groups.len(), 1);
        assert_eq!(page.groups[0].records[0].answer, "1.524 m");

        // The live result for what is typed leads, and copying it keeps it.
        let task = app.update(Message::CalculatorQueryChanged("=6*7".into()));
        settle(&mut app, task);
        let Page::Calculator(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.live.as_ref().map(|a| a.answer.as_str()), Some("42"));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        let writes = settle(&mut app, task);
        assert_eq!(writes, ["42"]);
        assert_eq!(backend.calculations.lock().unwrap().len(), 2);

        // The panel pins and removes a remembered row.
        let _ = app.update(Message::Command(UiCommand::Show));
        let task = app.open_calculator_history();
        settle(&mut app, task);
        let _ = app.update(Message::TogglePanel);
        let task = app.update(Message::PanelFilterChanged("Pin entry".into()));
        settle(&mut app, task);
        let task = app.update(Message::PanelActivate);
        settle(&mut app, task);
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("Delete entry".into()));
        let task = app.update(Message::PanelActivate);
        settle(&mut app, task);
        assert_eq!(
            backend.calculator_edits.lock().unwrap().as_slice(),
            [
                crate::backend::CalculatorChange::Pin("r1".into()),
                crate::backend::CalculatorChange::Remove("r1".into()),
            ]
        );
        let Page::Calculator(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.notice.as_deref(), Some("Entry removed"));
        assert_eq!(page.len(), 1, "the list reloaded without it");
    }

    #[test]
    fn describe_answers_whether_the_window_is_open_and_changes_nothing() {
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
        let (mut app, _) = LauncherApp::boot(AppFlags {
            start_hidden: true,
            link: Some(EngineLink::new(receiver, sender)),
            ..AppFlags::default()
        });
        let _ = app.update(Message::Command(UiCommand::Describe));
        assert_eq!(outcomes.try_recv(), Ok(UiOutcome::Hidden));
        assert!(app.pending_window.is_none(), "asking opened nothing");
        assert!(!app.is_awaiting());

        let _ = app.update(Message::Command(UiCommand::Show));
        let _ = app.update(Message::Command(UiCommand::Describe));
        assert_eq!(
            outcomes.try_recv(),
            Ok(UiOutcome::Shown),
            "a window on its way counts as open"
        );
    }

    #[test]
    fn a_command_line_launch_opens_the_builtin_and_types_its_fallback_text() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend);
        let launch = crate::backend::ExtensionLaunch {
            id: "commands:search-files".into(),
            arguments: None,
            preferences: false,
            fallback_text: Some("report".into()),
        };
        // Not settled: the search waits out its debounce on a timer.
        let _ = app.update(Message::LaunchFetched(Ok(launch)));
        assert_eq!(files_page(&app).query, "report", "{}", app.state_line());

        let launch = crate::backend::ExtensionLaunch {
            id: "commands:manage-snippets".into(),
            arguments: None,
            preferences: false,
            fallback_text: None,
        };
        let _ = app.update(Message::LaunchFetched(Ok(launch)));
        assert!(
            matches!(app.page, Page::Snippets(_)),
            "{}",
            app.state_line()
        );
    }

    #[test]
    fn hide_during_initial_open_waits_until_the_window_is_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
        app.link = Some(EngineLink::new(receiver, sender));
        let _ = app.open_window();
        let id = app.pending_window.unwrap();
        let _ = app.update(Message::Command(UiCommand::Hide));
        assert!(app.pending_hide);
        assert!(outcomes.try_recv().is_err());
        let _ = app.update(Message::Opened(id));
        assert!(outcomes.try_recv().is_err(), "must not acknowledge shown");
        let _ = app.update(Message::Closed(id));
        assert_eq!(outcomes.try_recv().unwrap(), UiOutcome::Hidden);
        assert!(!app.is_visible());
    }

    #[test]
    fn show_during_dismissal_waits_for_close_before_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
        app.link = Some(EngineLink::new(receiver, sender));
        let old = window::Id::unique();
        let _ = app.update(Message::Opened(old));
        let _ = app.update(Message::Dismiss);
        let _ = app.update(Message::Command(UiCommand::Show));
        assert!(app.pending_window.is_none());
        assert!(outcomes.try_recv().is_err());
        let _ = app.update(Message::Closed(old));
        let next = app.pending_window.unwrap();
        assert_ne!(old, next);
        assert!(outcomes.try_recv().is_err());
        let _ = app.update(Message::Opened(next));
        assert_eq!(outcomes.try_recv().unwrap(), UiOutcome::Shown);
        let _ = app.update(Message::Opened(old));
        assert_eq!(
            app.window,
            Some(next),
            "stale open cannot replace the current window"
        );
    }

    #[test]
    fn standalone_search_respects_startup_root_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        for provider_enabled in [true, false] {
            let config = compass_core::Config::parse(
                &format!(
                    r#"{{"providers":{{"applications":{{"enabled":{provider_enabled},"entrypoints":{{
                        "firefox":{{"enabled":false}},
                        "terminal":{{"enabled":true,"alias":"shellwork"}}
                    }}}}}}}}"#
                ),
                &dir.path().join("config.json"),
            )
            .unwrap();
            app.apply(AppFlags {
                root_config: config.root_config(),
                ..AppFlags::default()
            });
            app.query = "Firefox".into();
            app.search();
            assert!(app.results.is_empty());
            let _ = app.update(Message::QueryChanged("shellwork".into()));
            assert_eq!(app.results.len(), usize::from(provider_enabled));
            if provider_enabled {
                assert_eq!(app.selected_item().map(AppItem::name), Some("Terminal"));
            }
            if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
                for appearance in Appearance::ALL {
                    app.view.appearance = appearance;
                    let mut ui = iced_test::Simulator::with_size(
                        iced::Settings::default(),
                        iced::Size::new(800.0, 320.0),
                        app.view(),
                    );
                    assert!(
                        ui.snapshot(&app.theme())
                            .unwrap()
                            .matches_image(std::path::PathBuf::from(&directory).join(format!(
                                "{}-standalone-provider-{provider_enabled}.png",
                                appearance.name()
                            )))
                            .unwrap()
                    );
                }
            }
        }
        app.apply(AppFlags::default());
        let _ = app.update(Message::QueryChanged("shellwork".into()));
        assert!(
            app.results.is_empty(),
            "clearing settings removes the alias"
        );
        app.query = "Firefox".into();
        app.search();
        assert_eq!(app.results.len(), 1);
    }

    fn clipboard_writes(task: Task<Message>) -> Vec<String> {
        use iced::futures::{StreamExt, executor::block_on};
        use iced_winit::runtime::{Action, clipboard, task};

        let Some(stream) = task::into_stream(task) else {
            return Vec::new();
        };
        block_on(
            stream
                .filter_map(|action| async move {
                    match action {
                        Action::Clipboard(clipboard::Action::Write { contents, .. }) => {
                            Some(contents)
                        }
                        // A copy that hides the launcher exits one with no engine.
                        Action::Widget(_) | Action::Exit => None,
                        other => panic!("copy issued an unexpected action: {other:?}"),
                    }
                })
                .collect(),
        )
    }

    #[derive(Debug, Default)]
    struct RecordingLaunchTarget(std::sync::Mutex<Vec<Option<String>>>);

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

    fn recorded_launch(action: bool) -> Vec<Option<String>> {
        use iced::futures::{StreamExt, executor::block_on};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("browser.desktop"), "[Desktop Entry]\nType=Application\nName=Browser\nExec=parent\nActions=private;\n[Desktop Action private]\nName=Private Window\nExec=private-app\n").unwrap();
        let index = AppIndex::builder().dir(dir.path()).build();
        let selected = index
            .items()
            .iter()
            .position(|item| item.is_action() == action)
            .unwrap();
        let launcher = Arc::new(RecordingLaunchTarget::default());
        let mut app = LauncherApp::with_index(index).with_launcher(launcher.clone());
        app.results = vec![RootRow::App(selected)];
        let task = app.update(Message::LaunchSelected);
        let stream = iced_winit::runtime::task::into_stream(task).expect("launch task");
        let _outputs = block_on(stream.collect::<Vec<_>>());
        launcher.0.lock().unwrap().clone()
    }

    #[test]
    fn selecting_a_desktop_action_dispatches_its_id_not_the_parent() {
        assert_eq!(recorded_launch(true), [Some("private".to_owned())]);
    }

    #[test]
    fn selecting_an_application_still_dispatches_the_parent() {
        assert_eq!(recorded_launch(false), [None]);
    }

    // ---- The pointer ----

    /// What clicking the text `label` in the current view produces.
    fn click(app: &LauncherApp, label: &str) -> Vec<Message> {
        let mut ui = iced_test::simulator(app.view());
        ui.click(label)
            .unwrap_or_else(|_| panic!("{label:?} is not on screen: {}", app.state_line()));
        ui.into_messages().collect()
    }

    fn deliver(app: &mut LauncherApp, messages: Vec<Message>) {
        for message in messages {
            let task = app.update(message);
            settle(app, task);
        }
    }

    /// The desktop entries launched, by name.
    /// Four applications, all matching "Application", the first selected.
    fn four_applications(dir: &std::path::Path) -> LauncherApp {
        for i in 0..4 {
            fs::write(
                dir.join(format!("app-{i:02}.desktop")),
                format!(
                    "[Desktop Entry]\nType=Application\nName=Application {i:02}\nExec=/bin/true\n"
                ),
            )
            .expect("write entry");
        }
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir).build());
        app.query = "Application".into();
        app.search();
        assert!(app.results.len() >= 4, "{}", app.state_line());
        assert_eq!(app.selected, 0);
        app
    }

    #[derive(Debug, Default)]
    struct LaunchedEntries(std::sync::Mutex<Vec<String>>);

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

    #[test]
    fn clicking_a_root_row_launches_that_row_not_the_selected_one() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = Arc::new(LaunchedEntries::default());
        let mut app = four_applications(dir.path()).with_launcher(launcher.clone());
        let (position, name) = app
            .results
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(position, row)| match row {
                RootRow::App(index) => {
                    Some((position, app.app_index.items()[*index].name().to_owned()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no application below the first row: {}", app.state_line()));

        let messages = click(&app, &name);
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::ResultClicked(p) if *p == position)),
            "{messages:?}"
        );
        deliver(&mut app, messages);
        assert_eq!(launcher.0.lock().unwrap().as_slice(), [name]);
    }

    #[test]
    fn clicking_a_root_command_opens_it_and_a_store_row_opens_that_extension() {
        let dir = tempfile::tempdir().unwrap();
        let backend = store_backend(dir.path());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend);
        app.view.view_memory.set(
            crate::compass_pages::intro_key(crate::backend::Store::Vicinae),
            "true",
        );
        app.query = "extension store".into();
        app.search();
        let title = app
            .results
            .iter()
            .find_map(|row| match row {
                RootRow::Command(command) if command.id() == "commands:store" => {
                    Some(command.title)
                }
                _ => None,
            })
            .expect("the store is a root result");

        let messages = click(&app, title);
        deliver(&mut app, messages);
        let Page::Store(page) = &app.page else {
            panic!("the click did not open the store: {}", app.state_line());
        };
        assert_eq!(page.selected, 0);
        assert_eq!(page.rows[1].title, "Timer");

        let messages = click(&app, "Timer");
        deliver(&mut app, messages);
        let Page::StoreDetail(detail) = &app.page else {
            panic!("the click did not open the row: {}", app.state_line());
        };
        assert_eq!(detail.title, "Vicinae Store - Timer");
    }

    /// Hovering sends nothing and selects nothing: the C++ list moves its
    /// selection only on a click or a key (`SelectableDelegate.qml`).
    #[test]
    fn hovering_a_root_row_does_not_move_the_selection() {
        let dir = tempfile::tempdir().unwrap();
        let app = four_applications(dir.path());
        let RootRow::App(index) = app.results[1] else {
            panic!("the second row is not an application: {}", app.state_line())
        };
        let name = app.app_index.items()[index].name().to_owned();
        let mut ui = iced_test::simulator(app.view());
        let centre = ui
            .find(name.as_str())
            .expect("the row is drawn")
            .bounds()
            .center();
        ui.point_at(centre);
        ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
            position: centre,
        })]);
        let messages = ui.into_messages().collect::<Vec<_>>();
        assert!(messages.is_empty(), "{messages:?}");
        assert_eq!(app.selected, 0);
    }

    /// A snippet the fake expanded or pasted: `(id, arguments, pasted)`.
    type SnippetUse = (String, Vec<(String, String)>, bool);

    #[derive(Debug, Default)]
    struct TestBackend {
        /// Local storage, by namespace.
        storage: Vec<(String, Vec<crate::backend::StorageItemRow>)>,
        /// Stored token sets.
        token_sets: std::sync::Mutex<Vec<crate::backend::TokenSetRow>>,
        /// The root edits the engine was asked for.
        root_edits: std::sync::Mutex<Vec<(String, compass_core::root_items::RootEdit)>>,
        /// Other applications' tray icons.
        tray: Vec<crate::backend::TrayItemRow>,
        /// What the tray was asked to do.
        tray_calls: std::sync::Mutex<Vec<String>>,
        /// What the default pickers offer.
        default_apps: Vec<crate::backend::DefaultAppRow>,
        /// The recorder's capture, as the engine was told it.
        captures: std::sync::Mutex<Vec<bool>>,
        /// What the desktop says of every probed combination, and the
        /// combinations probed.
        probe_refusal: Option<String>,
        probes: std::sync::Mutex<Vec<String>>,
        /// The defaults set: `(kind, id)`.
        defaults_set: std::sync::Mutex<Vec<(crate::backend::DefaultApp, String)>>,
        keys: Vec<String>,
        recorded: std::sync::Mutex<Vec<String>>,
        fail_history: bool,
        ran: std::sync::Mutex<Vec<String>>,
        refuse_runs: Option<String>,
        /// Starts a view session instead of running to completion.
        view: Option<compass_extension_api::View>,
        events: std::sync::Mutex<Vec<(String, Vec<serde_json::Value>)>>,
        closed: std::sync::Mutex<Vec<u64>>,
        /// The view stack depth the fake reports.
        depth: std::sync::Mutex<u32>,
        /// An alert the fake's first view carries.
        alert: Option<crate::backend::ExtensionPrompt>,
        /// A toast the fake's views carry.
        toast: Option<crate::backend::ExtensionToast>,
        /// The power commands asked for.
        powered: std::sync::Mutex<Vec<String>>,
        /// The media commands asked for, each with its argument after a
        /// space.
        played: std::sync::Mutex<Vec<String>>,
        /// The players Now Playing lists.
        players: std::sync::Mutex<Vec<crate::backend::MediaPlayerRow>>,
        /// The families "Set as Compass font" saved.
        fonts_set: std::sync::Mutex<Vec<String>>,
        /// What the user allowed their Rhai scripts.
        grants: std::sync::Mutex<Vec<crate::backend::ScriptGrant>>,
        /// The calculator's history, one group, and what changed it.
        calculations: std::sync::Mutex<Vec<crate::backend::CalculatorRow>>,
        calculator_edits: std::sync::Mutex<Vec<crate::backend::CalculatorChange>>,
        /// What "Open with…" offers, and what it was asked to open.
        openers: Vec<crate::backend::OpenerRow>,
        opener_lookups: std::sync::Mutex<Vec<String>>,
        opened_with: std::sync::Mutex<Vec<(String, String)>>,
        /// What a file's panel depends on, and the file actions asked for.
        file_info: crate::backend::FileActions,
        file_calls: std::sync::Mutex<Vec<(&'static str, String)>>,
        /// What Now Playing asked the players to do.
        controlled: std::sync::Mutex<Vec<(String, crate::backend::MediaAction)>>,
        answers: std::sync::Mutex<Vec<bool>>,
        /// Whether the engine copies text; what it copied.
        copies: bool,
        copied: std::sync::Mutex<Vec<String>>,
        remembered: std::sync::Mutex<Vec<()>>,
        /// Preferences the fake asks for until some are saved.
        needs: Vec<crate::backend::PreferenceInput>,
        saved: std::sync::Mutex<Vec<serde_json::Map<String, serde_json::Value>>>,
        /// Arguments the fake asks for until a run carries some.
        wants: Vec<crate::backend::PreferenceInput>,
        given: std::sync::Mutex<Vec<Option<serde_json::Map<String, serde_json::Value>>>>,
        /// The files Search Files finds, by name.
        files: Vec<crate::backend::FileRow>,
        /// The Search Files queries asked, in order.
        file_queries: std::sync::Mutex<Vec<String>>,
        /// The files opened, and whether each was only revealed.
        opened: std::sync::Mutex<Vec<(String, bool)>>,
        /// The shortcut store.
        shortcuts: std::sync::Mutex<Vec<crate::backend::Shortcut>>,
        /// The shortcuts opened, with their arguments.
        opened_shortcuts: std::sync::Mutex<Vec<(String, Vec<String>)>>,
        /// The root items launched through the engine, with their text.
        launched: std::sync::Mutex<Vec<(String, Option<String>)>>,
        /// The text pasted.
        pasted: std::sync::Mutex<Vec<String>>,
        /// Whether a paste is refused, as where the engine cannot paste.
        refuse_paste: bool,
        /// The shortcuts saved, as the form sent them.
        drafts: std::sync::Mutex<Vec<crate::backend::ShortcutDraft>>,
        /// The snippet store.
        snippets: std::sync::Mutex<Vec<crate::backend::Snippet>>,
        /// The snippets saved, as the form sent them.
        snippet_drafts: std::sync::Mutex<Vec<crate::backend::SnippetDraft>>,
        /// The snippets expanded or pasted: `(id, arguments, pasted)`.
        snippet_uses: std::sync::Mutex<Vec<SnippetUse>>,
        /// The script commands the fake lists.
        scripts: Vec<compass_core::script_scan::ScriptItem>,
        /// Each script's icon URL, by id.
        script_icons: Vec<(String, String)>,
        /// The scripts run, with their arguments.
        script_runs: std::sync::Mutex<Vec<(String, Vec<String>)>>,
        /// The programs Run Terminal Program ran: `(argv, terminal, hold)`.
        programs_ran: std::sync::Mutex<Vec<(Vec<String>, bool, bool)>>,
        /// The dmenu answers sent: `(token, output)`.
        dmenu_answers: std::sync::Mutex<Vec<(u64, Option<String>)>>,
        /// The themes kept.
        themes_kept: std::sync::Mutex<Vec<String>>,
        /// The extensions created.
        created: std::sync::Mutex<Vec<crate::backend::ExtensionDraft>>,
        /// The store's rows; an install marks one installed and writes its
        /// manifest into `store_dir`, as the engine would.
        store_rows: std::sync::Mutex<Vec<crate::backend::StoreRow>>,
        /// Each install asked for: the store, the owner and the name.
        installs: std::sync::Mutex<Vec<(crate::backend::Store, String, String)>>,
        /// The handles detail pages and installs were asked by, in order.
        store_owners: std::sync::Mutex<Vec<String>>,
        /// Where installs land.
        store_dir: Option<std::path::PathBuf>,
        /// The store browses asked, in order.
        store_queries: std::sync::Mutex<Vec<(crate::backend::Store, String)>>,
        /// The URLs opened.
        opened_urls: std::sync::Mutex<Vec<String>>,
        /// The newer release the engine reports.
        update: std::sync::Mutex<Option<crate::backend::UpdateOffer>>,
        /// The releases skipped.
        skipped: std::sync::Mutex<Vec<String>>,
        /// The exchange rates the fake engine holds and refreshes to, and
        /// why a refresh fails when it does.
        rates: Option<compass_core::exchange_rates::ExchangeRates>,
        refuse_rates: Option<String>,
        /// The Rhai scripts the fake lists; `None` answers as the trait's
        /// default does.
        rhai_scripts: Option<Vec<compass_core::rhai_scripts::RhaiScriptItem>>,
        /// Whether settings writes are taken and recorded rather than refused
        /// as the trait's default refuses them.
        records_settings: bool,
        /// The settings written: `(key, value)`.
        settings_set: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
        /// The providers switched: `(provider, enabled)`.
        providers_set: std::sync::Mutex<Vec<(String, bool)>>,
        /// The commands whose preferences were asked for.
        preferences_asked: std::sync::Mutex<Vec<String>>,
        /// Why opening a file or a link fails, when it does.
        refuse_opens: Option<String>,
    }

    impl crate::backend::ApplicationBackend for TestBackend {
        fn search(&self, _query: String) -> crate::backend::BackendFuture<'_, Vec<String>> {
            Box::pin(async { Ok(self.keys.clone()) })
        }

        fn set_shortcut_capture(&self, capturing: bool) -> crate::backend::BackendFuture<'_, ()> {
            self.captures.lock().unwrap().push(capturing);
            Box::pin(async { Ok(()) })
        }

        fn probe_shortcut(
            &self,
            trigger: String,
        ) -> crate::backend::BackendFuture<'_, Option<String>> {
            self.probes.lock().unwrap().push(trigger);
            let refusal = self.probe_refusal.clone();
            Box::pin(async move { Ok(refusal) })
        }

        fn edit_root_item(
            &self,
            id: String,
            edit: compass_core::root_items::RootEdit,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.root_edits.lock().unwrap().push((id, edit));
                Ok(())
            })
        }

        fn local_storage_namespaces(&self) -> crate::backend::BackendFuture<'_, Vec<String>> {
            Box::pin(async move { Ok(self.storage.iter().map(|(ns, _)| ns.clone()).collect()) })
        }

        fn local_storage_items(
            &self,
            namespace: String,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::StorageItemRow>> {
            Box::pin(async move {
                Ok(self
                    .storage
                    .iter()
                    .find(|(ns, _)| *ns == namespace)
                    .map(|(_, items)| items.clone())
                    .unwrap_or_default())
            })
        }

        fn oauth_token_sets(
            &self,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::TokenSetRow>> {
            Box::pin(async move { Ok(self.token_sets.lock().unwrap().clone()) })
        }

        fn remove_oauth_token_set(
            &self,
            extension_id: String,
            provider_id: Option<String>,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.token_sets.lock().unwrap().retain(|set| {
                    set.extension_id != extension_id || set.provider_id != provider_id
                });
                Ok(())
            })
        }

        fn list_default_apps(
            &self,
            _kind: crate::backend::DefaultApp,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::DefaultAppRow>> {
            Box::pin(async move { Ok(self.default_apps.clone()) })
        }

        fn set_default_app(
            &self,
            kind: crate::backend::DefaultApp,
            id: String,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                if id == "broken.desktop" {
                    return Err(compass_core::default_app::BROWSER_FAILURE.to_owned());
                }
                self.defaults_set.lock().unwrap().push((kind, id));
                Ok(())
            })
        }

        fn record_launch(&self, key: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.recorded.lock().unwrap().push(key);
                if self.fail_history {
                    Err("disk full".to_owned())
                } else {
                    Ok(())
                }
            })
        }

        fn run_power_command(&self, id: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.powered.lock().unwrap().push(id);
                Ok(())
            })
        }

        fn run_media_command(
            &self,
            id: String,
            argument: Option<String>,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.played.lock().unwrap().push(match argument {
                    Some(argument) => format!("{id} {argument}"),
                    None => id,
                });
                Err("No media player is running".to_owned())
            })
        }

        fn list_media_players(
            &self,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::MediaPlayerRow>> {
            Box::pin(async move { Ok(self.players.lock().unwrap().clone()) })
        }

        fn control_media_player(
            &self,
            player: String,
            action: crate::backend::MediaAction,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                let mut players = self.players.lock().unwrap();
                if action == crate::backend::MediaAction::PlayPause
                    && let Some(row) = players.iter_mut().find(|row| row.id == player)
                {
                    row.playing = !row.playing;
                    row.paused = !row.playing;
                }
                self.controlled.lock().unwrap().push((player, action));
                Ok(())
            })
        }

        fn file_actions(
            &self,
            _path: String,
        ) -> crate::backend::BackendFuture<'_, crate::backend::FileActions> {
            Box::pin(async move { Ok(self.file_info.clone()) })
        }

        fn copy_file(&self, path: String, paste: bool) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                let what = if paste { "paste" } else { "copy" };
                self.file_calls.lock().unwrap().push((what, path));
                Ok(())
            })
        }

        fn run_executable(
            &self,
            path: String,
            make_executable: bool,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                assert!(make_executable);
                self.file_calls.lock().unwrap().push(("run", path));
                Ok(())
            })
        }

        fn set_wallpaper(&self, path: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.file_calls.lock().unwrap().push(("wallpaper", path));
                Err("Failed to set wallpaper: no backend".to_owned())
            })
        }

        fn search_files(
            &self,
            query: String,
            category: Option<String>,
        ) -> crate::backend::BackendFuture<'_, crate::backend::FileResults> {
            Box::pin(async move {
                self.file_queries.lock().unwrap().push(match &category {
                    Some(category) => format!("{query} [{category}]"),
                    None => query.clone(),
                });
                Ok(crate::backend::FileResults {
                    heading: if query.is_empty() {
                        "Recently Accessed".to_owned()
                    } else {
                        "Results".to_owned()
                    },
                    files: self
                        .files
                        .iter()
                        .filter(|file| file.name.contains(&query))
                        .filter(|file| category.as_ref().is_none_or(|c| &file.category == c))
                        .cloned()
                        .collect(),
                })
            })
        }

        fn open_file(&self, path: String, reveal: bool) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.opened.lock().unwrap().push((path, reveal));
                self.refuse_opens.clone().map_or(Ok(()), Err)
            })
        }

        fn fetch_dmenu(
            &self,
            token: u64,
        ) -> crate::backend::BackendFuture<'_, crate::backend::DmenuList> {
            Box::pin(async move {
                assert_eq!(token, 5);
                Ok(crate::backend::DmenuList {
                    content: "alpha\nbeta\n\ngamma\n".into(),
                    section_title: Some("Pick ({count})".into()),
                    ..crate::backend::DmenuList::default()
                })
            })
        }

        fn create_extension(
            &self,
            draft: crate::backend::ExtensionDraft,
        ) -> crate::backend::BackendFuture<'_, String> {
            Box::pin(async move {
                let path = format!("{}/{}", draft.location, draft.title.to_lowercase());
                self.created.lock().unwrap().push(draft);
                Ok(path)
            })
        }

        fn set_theme(&self, theme: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.themes_kept.lock().unwrap().push(theme);
                Ok(())
            })
        }

        fn list_fonts(&self) -> crate::backend::BackendFuture<'_, crate::backend::FontList> {
            Box::pin(async {
                let font = |name: &str, primary: &str, categories: &[&str]| {
                    crate::backend::FontListEntry {
                        name: name.into(),
                        family: name.into(),
                        glyph: Some("Aa".into()),
                        color: false,
                        primary: primary.into(),
                        categories: categories.iter().map(|c| (*c).to_owned()).collect(),
                    }
                };
                Ok(crate::backend::FontList {
                    fonts: vec![
                        font("Inter", "Latin", &["Latin"]),
                        font("JetBrains Mono", "Monospace", &["Latin", "Monospace"]),
                    ],
                    categories: vec!["Latin".into(), "Monospace".into()],
                })
            })
        }

        fn list_script_grants(
            &self,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::ScriptGrant>> {
            Box::pin(async move { Ok(self.grants.lock().unwrap().clone()) })
        }

        fn tray_items(
            &self,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::TrayItemRow>> {
            Box::pin(async move { Ok(self.tray.clone()) })
        }

        fn tray_activate(
            &self,
            key: String,
            secondary: bool,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.tray_calls
                    .lock()
                    .unwrap()
                    .push(format!("activate {key} {secondary}"));
                Ok(())
            })
        }

        fn tray_menu(
            &self,
            key: String,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::TrayMenuRow>> {
            Box::pin(async move {
                self.tray_calls.lock().unwrap().push(format!("menu {key}"));
                Ok(vec![
                    crate::backend::TrayMenuRow {
                        id: 1,
                        label: "Open Chat".into(),
                        ..crate::backend::TrayMenuRow::default()
                    },
                    crate::backend::TrayMenuRow {
                        id: 3,
                        label: "Status › Away".into(),
                        toggled: Some(true),
                        ..crate::backend::TrayMenuRow::default()
                    },
                ])
            })
        }

        fn tray_trigger(&self, key: String, id: i32) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.tray_calls
                    .lock()
                    .unwrap()
                    .push(format!("trigger {key} {id}"));
                Ok(())
            })
        }

        fn exchange_rates(
            &self,
        ) -> crate::backend::BackendFuture<'_, Option<compass_core::exchange_rates::ExchangeRates>>
        {
            Box::pin(async move { Ok(self.rates.clone()) })
        }

        fn refresh_exchange_rates(
            &self,
        ) -> crate::backend::BackendFuture<'_, compass_core::exchange_rates::ExchangeRates>
        {
            Box::pin(async move {
                match (&self.refuse_rates, &self.rates) {
                    (Some(reason), _) => Err(reason.clone()),
                    (None, Some(rates)) => Ok(rates.clone()),
                    (None, None) => Err("there is no exchange rate source".to_owned()),
                }
            })
        }

        fn calculator_history(
            &self,
            query: String,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::CalculatorGroupRow>> {
            Box::pin(async move {
                let records: Vec<_> = self
                    .calculations
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|r| r.question.contains(&query) || r.answer.contains(&query))
                    .cloned()
                    .collect();
                Ok(if records.is_empty() {
                    Vec::new()
                } else {
                    vec![crate::backend::CalculatorGroupRow {
                        name: "Today".into(),
                        records,
                    }]
                })
            })
        }

        fn add_calculator_record(
            &self,
            question: String,
            answer: String,
            conversion: bool,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                let mut rows = self.calculations.lock().unwrap();
                let id = format!("r{}", rows.len());
                rows.insert(
                    0,
                    crate::backend::CalculatorRow {
                        id,
                        question,
                        answer,
                        conversion,
                        pinned: false,
                    },
                );
                Ok(())
            })
        }

        fn edit_calculator_history(
            &self,
            change: crate::backend::CalculatorChange,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                if let crate::backend::CalculatorChange::Remove(id) = &change {
                    self.calculations.lock().unwrap().retain(|r| r.id != *id);
                }
                self.calculator_edits.lock().unwrap().push(change);
                Ok(())
            })
        }

        fn revoke_script_grant(
            &self,
            id: String,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::ScriptGrant>> {
            Box::pin(async move {
                let mut grants = self.grants.lock().unwrap();
                grants.retain(|grant| grant.id != id);
                Ok(grants.clone())
            })
        }

        fn set_font(&self, family: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.fonts_set.lock().unwrap().push(family);
                Ok(())
            })
        }

        fn font_specimen(&self, name: String) -> crate::backend::BackendFuture<'_, String> {
            Box::pin(async move { Ok(format!("# {name}\n\nThe quick brown fox\n\n---\n")) })
        }

        fn store_browse(
            &self,
            store: crate::backend::Store,
            query: String,
        ) -> crate::backend::BackendFuture<'_, crate::backend::StoreList> {
            Box::pin(async move {
                self.store_queries
                    .lock()
                    .unwrap()
                    .push((store, query.clone()));
                let rows = self
                    .store_rows
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|row| row.title.to_lowercase().contains(&query.to_lowercase()))
                    .cloned()
                    .collect();
                Ok(crate::backend::StoreList {
                    heading: if query.is_empty() {
                        "Extensions"
                    } else {
                        "Results"
                    }
                    .into(),
                    rows,
                })
            })
        }

        fn store_extension(
            &self,
            _store: crate::backend::Store,
            owner: String,
            name: String,
        ) -> crate::backend::BackendFuture<'_, crate::backend::StoreDetail> {
            Box::pin(async move {
                // Only a named owner: the action audit watches this
                // backend's fields, and its rows name none.
                if !owner.is_empty() {
                    self.store_owners.lock().unwrap().push(owner);
                }
                let row = self
                    .store_rows
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|row| row.name == name)
                    .cloned()
                    .ok_or("Extension not found")?;
                Ok(crate::backend::StoreDetail {
                    markdown: format!("# {}\n\n{}", row.title, row.description),
                    row,
                    readme_url: Some("https://example.com/README.md".into()),
                    ..crate::backend::StoreDetail::default()
                })
            })
        }

        fn store_install(
            &self,
            store: crate::backend::Store,
            owner: String,
            name: String,
        ) -> crate::backend::BackendFuture<'_, (String, String)> {
            Box::pin(async move {
                self.installs
                    .lock()
                    .unwrap()
                    .push((store, owner.clone(), name.clone()));
                // Only a named owner: the action audit watches this
                // backend's fields, and its rows name none.
                if !owner.is_empty() {
                    self.store_owners.lock().unwrap().push(owner);
                }
                let mut rows = self.store_rows.lock().unwrap();
                let row = rows
                    .iter_mut()
                    .find(|row| row.name == name)
                    .ok_or("Extension not found")?;
                row.installed = true;
                if let Some(dir) = &self.store_dir {
                    let target = dir.join(&row.id);
                    fs::create_dir_all(&target).unwrap();
                    fs::write(
                        target.join("package.json"),
                        format!(
                            r#"{{"name": "{name}", "title": "{}", "author": "zoe",
                                "commands": [{{"name": "show", "title": "Show The Clock", "mode": "view"}}]}}"#,
                            row.title
                        ),
                    )
                    .unwrap();
                }
                Ok((row.id.clone(), row.title.clone()))
            })
        }

        fn store_uninstall(&self, id: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                for row in self.store_rows.lock().unwrap().iter_mut() {
                    if row.id == id {
                        row.installed = false;
                    }
                }
                if let Some(dir) = &self.store_dir {
                    let _ = fs::remove_dir_all(dir.join(&id));
                }
                Ok(())
            })
        }

        fn open_url(&self, url: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.opened_urls.lock().unwrap().push(url);
                self.refuse_opens.clone().map_or(Ok(()), Err)
            })
        }

        fn update_status(
            &self,
        ) -> crate::backend::BackendFuture<'_, Option<crate::backend::UpdateOffer>> {
            Box::pin(async move { Ok(self.update.lock().unwrap().clone()) })
        }

        fn skip_update(&self, tag: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                let mut update = self.update.lock().unwrap();
                if update.as_ref().is_some_and(|offer| offer.tag == tag) {
                    *update = None;
                }
                self.skipped.lock().unwrap().push(tag);
                Ok(())
            })
        }

        fn choose_dmenu(
            &self,
            token: u64,
            output: Option<String>,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.dmenu_answers.lock().unwrap().push((token, output));
                Ok(())
            })
        }

        fn list_programs(&self) -> crate::backend::BackendFuture<'_, crate::backend::ProgramList> {
            Box::pin(async {
                Ok(crate::backend::ProgramList {
                    programs: vec!["/usr/bin/htop".into(), "/usr/bin/top".into()],
                    terminal: Some("Ptyxis".into()),
                    default_action: "run-in-terminal".into(),
                })
            })
        }

        fn run_program(
            &self,
            argv: Vec<String>,
            terminal: bool,
            hold: bool,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.programs_ran
                    .lock()
                    .unwrap()
                    .push((argv, terminal, hold));
                Ok(())
            })
        }

        fn list_scripts(
            &self,
        ) -> crate::backend::BackendFuture<'_, Vec<compass_core::script_scan::ScriptItem>> {
            Box::pin(async move { Ok(self.scripts.clone()) })
        }

        fn run_script(
            &self,
            id: String,
            arguments: Vec<String>,
        ) -> crate::backend::BackendFuture<'_, Option<u64>> {
            Box::pin(async move {
                let mode = self
                    .scripts
                    .iter()
                    .find(|script| script.id == id)
                    .map(|script| script.mode)
                    .ok_or("No script command has that id")?;
                self.script_runs.lock().unwrap().push((id, arguments));
                Ok(match mode {
                    compass_core::script_command::OutputMode::Silent
                    | compass_core::script_command::OutputMode::Terminal => None,
                    _ => Some(7),
                })
            })
        }

        fn script_output(
            &self,
            session: u64,
        ) -> crate::backend::BackendFuture<'_, crate::backend::ScriptOutputState> {
            Box::pin(async move {
                let (id, arguments) = self.script_runs.lock().unwrap().last().cloned().unwrap();
                assert_eq!(session, 7);
                Ok(crate::backend::ScriptOutputState {
                    output: format!(
                        "\u{1b}[31m{id}\u{1b}[0m {}\nsecond line",
                        arguments.join(" ")
                    ),
                    finished: true,
                    exit_code: Some(0),
                    elapsed_ms: 1200,
                })
            })
        }

        fn list_snippets(&self) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Snippet>> {
            Box::pin(async move { Ok(self.snippets.lock().unwrap().clone()) })
        }

        fn save_snippet(
            &self,
            draft: crate::backend::SnippetDraft,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Snippet>> {
            Box::pin(async move {
                self.snippet_drafts.lock().unwrap().push(draft.clone());
                let mut snippets = self.snippets.lock().unwrap();
                let id = draft
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("snp-{}", snippets.len()));
                let stored = crate::backend::Snippet {
                    id: id.clone(),
                    name: draft.name,
                    data: compass_core::snippet_store::SnippetData::Text { text: draft.text },
                    expansion: draft.keyword.map(|keyword| {
                        compass_core::snippet_store::StoredExpansion {
                            keyword,
                            apps: draft.apps,
                            word: draft.word,
                        }
                    }),
                    ..crate::backend::Snippet::default()
                };
                match snippets.iter_mut().find(|s| s.id == id) {
                    Some(existing) => *existing = stored,
                    None => snippets.push(stored),
                }
                Ok(snippets.clone())
            })
        }

        fn remove_snippet(
            &self,
            id: String,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Snippet>> {
            Box::pin(async move {
                let mut snippets = self.snippets.lock().unwrap();
                snippets.retain(|s| s.id != id);
                Ok(snippets.clone())
            })
        }

        fn preview_snippet(
            &self,
            id: String,
            arguments: Vec<(String, String)>,
        ) -> crate::backend::BackendFuture<'_, String> {
            Box::pin(async move {
                let values: Vec<String> = arguments.into_iter().map(|(_, v)| v).collect();
                Ok(format!("preview {id}:{}", values.join(",")))
            })
        }

        fn script_icons(&self) -> crate::backend::BackendFuture<'_, Vec<(String, String)>> {
            Box::pin(async move { Ok(self.script_icons.clone()) })
        }

        fn expand_snippet(
            &self,
            id: String,
            arguments: Vec<(String, String)>,
        ) -> crate::backend::BackendFuture<'_, String> {
            Box::pin(async move {
                self.snippet_uses
                    .lock()
                    .unwrap()
                    .push((id.clone(), arguments.clone(), false));
                let values: Vec<String> = arguments.into_iter().map(|(_, v)| v).collect();
                Ok(format!("{id}:{}", values.join(",")))
            })
        }

        fn paste_snippet(
            &self,
            id: String,
            arguments: Vec<(String, String)>,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.snippet_uses
                    .lock()
                    .unwrap()
                    .push((id, arguments, true));
                Ok(())
            })
        }

        fn list_shortcuts(
            &self,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Shortcut>> {
            Box::pin(async move { Ok(self.shortcuts.lock().unwrap().clone()) })
        }

        fn save_shortcut(
            &self,
            draft: crate::backend::ShortcutDraft,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Shortcut>> {
            Box::pin(async move {
                self.drafts.lock().unwrap().push(draft.clone());
                let mut shortcuts = self.shortcuts.lock().unwrap();
                let id = draft
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("sct-{}", shortcuts.len()));
                let stored = crate::backend::Shortcut {
                    id: id.clone(),
                    name: draft.name,
                    icon: draft.icon,
                    url: draft.url,
                    app: draft.app,
                    ..crate::backend::Shortcut::default()
                };
                match shortcuts.iter_mut().find(|s| s.id == id) {
                    Some(existing) => *existing = stored,
                    None => shortcuts.push(stored),
                }
                Ok(shortcuts.clone())
            })
        }

        fn remove_shortcut(
            &self,
            id: String,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::Shortcut>> {
            Box::pin(async move {
                let mut shortcuts = self.shortcuts.lock().unwrap();
                shortcuts.retain(|s| s.id != id);
                Ok(shortcuts.clone())
            })
        }

        fn list_openers(
            &self,
            target: String,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::OpenerRow>> {
            Box::pin(async move {
                self.opener_lookups.lock().unwrap().push(target);
                Ok(self.openers.clone())
            })
        }

        fn open_with(&self, app: String, target: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.opened_with.lock().unwrap().push((app, target));
                Ok(())
            })
        }

        fn open_shortcut(
            &self,
            id: String,
            arguments: Vec<String>,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.opened_shortcuts.lock().unwrap().push((id, arguments));
                Ok(())
            })
        }

        fn copy_text(&self, text: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                if !self.copies {
                    return Err("Copying needs a clipboard".to_owned());
                }
                self.copied.lock().unwrap().push(text);
                Ok(())
            })
        }

        fn paste_text(&self, text: String) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                if self.refuse_paste {
                    return Err("Pasting needs the GNOME Shell extension".to_owned());
                }
                self.pasted.lock().unwrap().push(text);
                Ok(())
            })
        }

        fn launch_command(
            &self,
            id: String,
            query: Option<String>,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.launched.lock().unwrap().push((id, query));
                Ok(())
            })
        }

        fn expand_shortcut(
            &self,
            id: String,
            arguments: Vec<String>,
        ) -> crate::backend::BackendFuture<'_, String> {
            Box::pin(async move {
                let shortcuts = self.shortcuts.lock().unwrap();
                let shortcut = shortcuts.iter().find(|s| s.id == id).ok_or("gone")?;
                Ok(format!("{}|{}", shortcut.url, arguments.join(",")))
            })
        }

        fn run_extension_command(
            &self,
            id: String,
            arguments: Option<serde_json::Map<String, serde_json::Value>>,
        ) -> crate::backend::BackendFuture<'_, crate::backend::ExtensionStart> {
            Box::pin(async move {
                self.ran.lock().unwrap().push(id);
                let asked = !self.wants.is_empty() && arguments.is_none();
                self.given.lock().unwrap().push(arguments);
                if let Some(reason) = self.refuse_runs.clone() {
                    return Err(reason);
                }
                if !self.needs.is_empty() && self.saved.lock().unwrap().is_empty() {
                    return Ok(crate::backend::ExtensionStart::NeedsPreferences {
                        title: "Write Greeting".into(),
                        fields: self.needs.clone(),
                    });
                }
                if asked {
                    return Ok(crate::backend::ExtensionStart::NeedsArguments {
                        title: "Write Greeting".into(),
                        fields: self.wants.clone(),
                    });
                }
                Ok(if self.view.is_some() {
                    crate::backend::ExtensionStart::View(7)
                } else {
                    crate::backend::ExtensionStart::Ran
                })
            })
        }

        // The first poll gets the view; the next says the command ended, so a
        // test's task loop stops rather than polling for ever.
        fn extension_view(
            &self,
            _session: u64,
            after: u64,
        ) -> crate::backend::BackendFuture<'_, crate::backend::ExtensionViewState> {
            Box::pin(async move {
                Ok(crate::backend::ExtensionViewState {
                    version: after + 1,
                    view: (after == 0)
                        .then(|| self.view.clone().map(Box::new))
                        .flatten(),
                    problem: None,
                    ended: after > 0,
                    depth: *self.depth.lock().unwrap(),
                    alert: self.alert.clone(),
                    toast: self.toast.clone(),
                })
            })
        }

        fn extension_event(
            &self,
            _session: u64,
            handler: String,
            args: Vec<serde_json::Value>,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.events.lock().unwrap().push((handler, args));
                Ok(())
            })
        }

        fn set_extension_preferences(
            &self,
            _id: String,
            values: serde_json::Map<String, serde_json::Value>,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.saved.lock().unwrap().push(values);
                Ok(())
            })
        }

        fn extension_alert_answer(
            &self,
            _session: u64,
            confirmed: bool,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.answers.lock().unwrap().push(confirmed);
                Ok(())
            })
        }

        fn extension_alert_remember(&self, _session: u64) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.remembered.lock().unwrap().push(());
                Ok(())
            })
        }

        fn extension_pop(&self, _session: u64) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.events.lock().unwrap().push(("pop".to_owned(), vec![]));
                Ok(())
            })
        }

        fn close_extension(&self, session: u64) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.closed.lock().unwrap().push(session);
                Ok(())
            })
        }

        fn list_rhai_scripts(
            &self,
        ) -> crate::backend::BackendFuture<'_, Vec<compass_core::rhai_scripts::RhaiScriptItem>>
        {
            let scripts = self.rhai_scripts.clone();
            Box::pin(async move { scripts.ok_or_else(|| "no Rhai scripts here".to_owned()) })
        }

        fn set_setting(
            &self,
            key: String,
            value: serde_json::Value,
        ) -> crate::backend::BackendFuture<'_, ()> {
            if !self.records_settings {
                return Box::pin(async { Err(crate::settings_page::NEEDS_ENGINE.to_owned()) });
            }
            self.settings_set.lock().unwrap().push((key, value));
            Box::pin(async { Ok(()) })
        }

        fn set_provider_enabled(
            &self,
            provider: String,
            enabled: bool,
        ) -> crate::backend::BackendFuture<'_, ()> {
            if !self.records_settings {
                return Box::pin(async { Err(crate::settings_page::NEEDS_ENGINE.to_owned()) });
            }
            self.providers_set.lock().unwrap().push((provider, enabled));
            Box::pin(async { Ok(()) })
        }

        fn extension_preferences(
            &self,
            id: String,
        ) -> crate::backend::BackendFuture<'_, crate::backend::ExtensionStart> {
            if !self.records_settings {
                return Box::pin(async {
                    Err("Running extension commands needs the Compass engine".to_owned())
                });
            }
            self.preferences_asked.lock().unwrap().push(id.clone());
            Box::pin(async move {
                Ok(crate::backend::ExtensionStart::NeedsPreferences {
                    title: id,
                    fields: Vec::new(),
                })
            })
        }
    }

    mod action_audit;

    fn extension_app(dir: &std::path::Path, backend: Arc<TestBackend>) -> LauncherApp {
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

    #[test]
    fn configure_fallback_commands_moves_items_between_its_sections() {
        use compass_core::root_items::RootEdit;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = extension_app(dir.path(), backend.clone());
        app.fallbacks = vec!["files:search".into()];
        open_builtin(&mut app, "configure fallback", "commands:manage-fallback");
        let Page::Fallbacks(page) = &app.page else {
            panic!("not the fallback manager: {}", app.state_line());
        };
        let titles: Vec<(&str, Option<&str>)> = page
            .rows
            .iter()
            .enumerate()
            .map(|(at, row)| {
                (
                    page.candidates[row.candidate].title.as_str(),
                    page.heading_at(at),
                )
            })
            .collect();
        assert_eq!(
            titles,
            [
                ("Search Files", Some("Enabled")),
                ("Write Greeting", Some("Available"))
            ]
        );

        // Enter on the available command enables it, first.
        let _ = app.update(Message::FallbacksQueryChanged("greet".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            app.fallbacks,
            ["@someone/hello:write".to_owned(), "files:search".to_owned()]
        );
        assert_eq!(
            backend.root_edits.lock().unwrap().last(),
            Some(&("@someone/hello:write".to_owned(), RootEdit::Fallback(true)))
        );

        // The panel's action disables Search Files by the id it is stored as.
        let _ = app.update(Message::FallbacksQueryChanged("search files".into()));
        let _ = app.update(Message::TogglePanel);
        let task = choose(&mut app, "Disable fallback");
        settle(&mut app, task);
        assert_eq!(app.fallbacks, ["@someone/hello:write".to_owned()]);
        assert_eq!(
            backend.root_edits.lock().unwrap().last(),
            Some(&("files:search".to_owned(), RootEdit::Fallback(false)))
        );

        // A root fallback row's panel opens the manager.
        let _ = app.update(Message::Back);
        app.query = "zzzz".into();
        app.search();
        app.selected = app
            .results
            .iter()
            .position(|row| matches!(row, RootRow::Fallback(_)))
            .expect("the extension is a fallback now");
        let _ = app.update(Message::TogglePanel);
        assert_eq!(
            panel_titles(&app),
            ["Open command", "Manage fallback actions"]
        );
        let task = choose(&mut app, "Manage fallback actions");
        settle(&mut app, task);
        assert!(matches!(app.page, Page::Fallbacks(_)));
    }

    #[test]
    fn installed_extensions_are_listed_copied_and_uninstalled_after_asking() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = with_resident_hud(extension_app(dir.path(), backend.clone()));
        open_builtin(&mut app, "installed extensions", "commands:list-extensions");
        let Page::Extensions(page) = &app.page else {
            panic!("not the installed extensions: {}", app.state_line());
        };
        assert_eq!(page.all.len(), 1);
        assert_eq!(page.all[0].title, "Hello");
        let _ = app.update(Message::TogglePanel);
        assert_eq!(
            panel_titles(&app),
            [
                "Uninstall",
                "Copy name",
                "Copy ID",
                "Copy path",
                "Copy author"
            ]
        );
        let task = choose(&mut app, "Copy author");
        assert_eq!(settle(&mut app, task), ["someone"]);
        assert_eq!(app.hud_content(), Some(&crate::hud::Hud::copied()));

        let _ = app.update(Message::Command(UiCommand::Show));
        open_builtin(&mut app, "installed extensions", "commands:list-extensions");
        let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(app.confirm.is_some(), "asks first");
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::Extensions(page) = &app.page else {
            panic!("left the view: {}", app.state_line());
        };
        assert!(page.all.is_empty(), "gone from the list");
        assert_eq!(page.notice.as_deref(), Some("Extension uninstalled"));
    }

    #[test]
    fn search_builtin_icons_copies_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let config = compass_core::Config::parse(
            r#"{"providers":{"commands":{"entrypoints":{"search-builtin-icons":{"enabled":true}}}}}"#,
            std::path::Path::new("config.json"),
        )
        .unwrap();
        app.root_config = config.root_config();
        app.app_index.apply_root_config(&app.root_config);
        open_builtin(&mut app, "builtin icons", "commands:search-builtin-icons");
        let _ = app.update(Message::IconsQueryChanged("copy-clipboard".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert_eq!(settle(&mut app, task), ["copy-clipboard"]);
        assert_eq!(app.hud_content(), Some(&crate::hud::Hud::copied()));
    }

    /// Turns on the default-disabled Compass commands named.
    fn enable_commands(app: &mut LauncherApp, entrypoints: &[&str]) {
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

    #[test]
    fn inspect_local_storage_browses_a_namespace_and_shows_a_value() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            storage: vec![
                (
                    "@a/notes".to_owned(),
                    vec![crate::backend::StorageItemRow {
                        key: "draft".into(),
                        value: "hello".into(),
                    }],
                ),
                ("core".to_owned(), Vec::new()),
            ],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend);
        enable_commands(&mut app, &["inspect-local-storage"]);
        open_builtin(
            &mut app,
            "inspect local storage",
            "commands:inspect-local-storage",
        );
        let Page::Storage(page) = &app.page else {
            panic!("not local storage: {}", app.state_line());
        };
        assert_eq!(page.titles(), ["@a/notes", "core"]);
        let _ = app.update(Message::TogglePanel);
        assert_eq!(panel_titles(&app), ["Browse namespace"]);
        let task = choose(&mut app, "Browse namespace");
        settle(&mut app, task);
        let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
        let Page::Storage(page) = &app.page else {
            panic!("left local storage");
        };
        assert_eq!(page.titles(), ["draft"]);
        assert_eq!(page.notice.as_deref(), Some("hello"), "Show value");
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        let Page::Storage(page) = &app.page else {
            panic!("Escape left the view instead of the namespace");
        };
        assert!(page.browsing.is_none());
    }

    #[test]
    fn manage_oauth_token_sets_copies_and_removes_after_asking() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            token_sets: std::sync::Mutex::new(vec![crate::backend::TokenSetRow {
                extension_id: "github".into(),
                provider_id: Some("GitHub".into()),
                access_token: "gho_token".into(),
                scope: Some("repo".into()),
                expires_at: Some(0),
                expired: true,
                ..crate::backend::TokenSetRow::default()
            }]),
            ..TestBackend::default()
        });
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(backend.clone());
        enable_commands(&mut app, &["oauth-token-store"]);
        open_builtin(
            &mut app,
            "manage oauth token sets",
            "commands:oauth-token-store",
        );
        let _ = app.update(Message::TogglePanel);
        assert_eq!(
            panel_titles(&app),
            [
                "Remove token set",
                "Copy access token",
                "Copy scopes",
                "Copy expiration date"
            ]
        );
        let task = choose(&mut app, "Copy access token");
        assert_eq!(settle(&mut app, task), ["gho_token"]);

        let _ = app.update(Message::Command(UiCommand::Show));
        open_builtin(
            &mut app,
            "manage oauth token sets",
            "commands:oauth-token-store",
        );
        let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(app.confirm.is_some(), "asks first");
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert!(backend.token_sets.lock().unwrap().is_empty());
        let Page::Tokens(page) = &app.page else {
            panic!("left the view");
        };
        assert!(page.sets.is_empty(), "listed again");
        assert_eq!(page.notice.as_deref(), Some("Token set removed"));
    }

    #[test]
    fn the_link_and_refresh_commands_do_what_their_cpp_ones_do() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "donate", "commands:sponsor");
        assert_eq!(
            backend.opened_urls.lock().unwrap().as_slice(),
            [compass_core::commands::SPONSOR_URL]
        );
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Opened in browser")
        );

        let _ = app.update(Message::Command(UiCommand::Show));
        open_builtin(&mut app, "report a compass bug", "commands:report-bug");
        let reported = backend.opened_urls.lock().unwrap().last().cloned().unwrap();
        assert!(
            reported.starts_with(compass_core::bug_report::CREATE_ISSUE_URL),
            "{reported}"
        );
        assert!(reported.contains("type=bug"));

        fs::write(
            dir.path().join("zephyr.desktop"),
            "[Desktop Entry]\nType=Application\nName=Zephyr\nExec=/bin/true\n",
        )
        .unwrap();
        let _ = app.update(Message::Command(UiCommand::Show));
        open_builtin(&mut app, "refresh apps", "commands:refresh-apps");
        assert!(app.app_index.get("zephyr.desktop").is_some(), "rescanned");
        assert_eq!(app.error.as_deref(), Some("Apps successfully refreshed"));
    }

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

    #[test]
    fn an_extension_s_launch_runs_its_command_and_its_subtitle_override_shows() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["@someone/hello:write".to_owned()],
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        let _ = app.update(Message::ExtensionSubtitlesLoaded(Ok(vec![(
            "@someone/hello:write".into(),
            "3 unread".into(),
        )])));
        for message in task_messages(app.update(Message::QueryChanged("greeting".into()))) {
            let _ = app.update(message);
        }
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("3 unread").is_ok(), "the override is the subtitle");
            assert!(
                ui.find("Hello").is_err(),
                "in place of the extension's title"
            );
        }

        let mut arguments = serde_json::Map::new();
        arguments.insert("who".into(), "ada".into());
        let launch = crate::backend::ExtensionLaunch {
            id: "@someone/hello:write".into(),
            arguments: Some(arguments.clone()),
            preferences: false,
            fallback_text: None,
        };
        for message in task_messages(app.update(Message::LaunchFetched(Ok(launch)))) {
            let _ = app.update(message);
        }
        assert_eq!(
            backend.ran.lock().unwrap().as_slice(),
            ["@someone/hello:write"]
        );
        assert_eq!(backend.given.lock().unwrap().last(), Some(&Some(arguments)));
    }

    #[test]
    fn preferences_an_extension_opens_are_saved_without_running_it() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = extension_app(dir.path(), backend.clone());
        let _ = app.update(Message::PreferencesOpened {
            id: "@someone/hello:write".into(),
            result: Ok(crate::backend::ExtensionStart::NeedsPreferences {
                title: "Write Greeting".into(),
                fields: Vec::new(),
            }),
        });
        let Page::Preferences(page) = &app.page else {
            panic!("no preferences form: {}", app.state_line());
        };
        assert_eq!(
            page.purpose,
            crate::preferences_page::Purpose::CommandPreferences
        );
        for message in task_messages(app.update(Message::PreferencesSubmit)) {
            let _ = app.update(message);
        }
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
        assert_eq!(backend.saved.lock().unwrap().len(), 1);
        assert!(
            backend.ran.lock().unwrap().is_empty(),
            "saving does not run it"
        );
    }

    #[test]
    fn a_rhai_script_is_a_row_opens_as_an_extension_view_and_leaves_when_it_pops() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["rhai:script.hello".to_owned()],
            view: Some(greeting_list(false)),
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        let _ = app.update(Message::RhaiScriptsLoaded(Ok(vec![
            compass_core::rhai_scripts::RhaiScriptItem {
                id: "script.hello".into(),
                title: "Hello Script".into(),
                description: Some("Says hello".into()),
                icon: Some("globe".into()),
                keywords: Vec::new(),
            },
        ])));
        for message in task_messages(app.update(Message::QueryChanged("hello script".into()))) {
            let _ = app.update(message);
        }
        assert_eq!(
            app.selected_row(),
            Some(RootRow::RhaiScript(0)),
            "{}",
            app.state_line()
        );
        assert!(app.state_line().contains("selected_title=\"Hello Script\""));
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(
                ui.find("Says hello").is_ok(),
                "the description is the subtitle"
            );
        }

        // The fake draws the list on the first poll and ends on the second,
        // as a script that popped itself does.
        let mut drawn = false;
        let mut pending = task_messages(app.update(Message::LaunchSelected));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
            if let Page::Extension(page) = &app.page {
                assert!(page.leaves_on_end);
                drawn |= page.shown.len() == 2;
            }
        }
        assert_eq!(
            backend.ran.lock().unwrap().as_slice(),
            ["rhai:script.hello"]
        );
        assert!(drawn, "the script's list was drawn");
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
        assert_eq!(backend.closed.lock().unwrap().as_slice(), [7]);
    }

    fn greeting_list(host_filtering: bool) -> compass_extension_api::View {
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
            list.search.on_change =
                Some(compass_extension_api::action::HandlerId::new("cb-search"));
        }
        compass_extension_api::View::List(list)
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

    fn task_messages(task: Task<Message>) -> Vec<Message> {
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

    fn backend_app(dir: &std::path::Path, backend: Arc<TestBackend>) -> LauncherApp {
        for (id, name) in [("alpha", "Alpha Editor"), ("beta", "Beta Editor")] {
            std::fs::write(
                dir.join(format!("{id}.desktop")),
                format!("[Desktop Entry]\nType=Application\nName={name}\nExec=unused\n"),
            )
            .unwrap();
        }
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir).build());
        app.apply(AppFlags {
            backend: Some(backend),
            ..AppFlags::default()
        });
        app
    }

    #[test]
    fn backend_order_is_used_and_old_queries_cannot_replace_new_results() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec![
                "applications:beta".to_owned(),
                "applications:alpha".to_owned(),
            ],
            ..TestBackend::default()
        });
        let mut app = backend_app(dir.path(), backend);
        let old = app.update(Message::QueryChanged("Ed".to_owned()));
        let old_generation = app.search_generation;
        let current = app.update(Message::QueryChanged("Editor".to_owned()));
        assert!(
            task_messages(old).is_empty(),
            "superseded request is aborted"
        );
        assert!(
            app.selected_item().is_none(),
            "old results cannot be launched while pending"
        );
        for message in task_messages(current) {
            let _ = app.update(message);
        }
        assert_eq!(app.selected_item().unwrap().key(), "beta.desktop");
        let _ = app.update(Message::SearchCompleted {
            generation: old_generation,
            result: Ok(vec!["applications:alpha".to_owned()]),
        });
        assert_eq!(app.selected_item().unwrap().key(), "beta.desktop");
        let generation = app.search_generation;
        let _ = app.update(Message::QueryChanged(String::new()));
        let _ = app.update(Message::SearchCompleted {
            generation,
            result: Ok(vec!["applications:alpha".to_owned()]),
        });
        assert!(app.results.is_empty());
    }

    #[test]
    fn backend_errors_or_unknown_keys_do_not_restore_stale_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = backend_app(dir.path(), Arc::new(TestBackend::default()));
        for result in [
            Err("offline".to_owned()),
            Ok(vec!["applications:missing".to_owned()]),
        ] {
            let _pending = app.update(Message::QueryChanged("Editor".to_owned()));
            let _ = app.update(Message::SearchCompleted {
                generation: app.search_generation,
                result,
            });
            assert!(app.results.is_empty());
            assert!(app.error.is_some());
        }
    }

    #[test]
    fn opening_or_clearing_an_attached_window_requests_initial_suggestions() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec![
                "applications:beta".to_owned(),
                "applications:alpha".to_owned(),
            ],
            ..TestBackend::default()
        });
        let mut app = backend_app(dir.path(), backend);
        let opened = app.update(Message::Opened(window::Id::unique()));
        for message in task_messages(opened) {
            let _ = app.update(message);
        }
        assert!(app.query.is_empty());
        assert_eq!(app.selected_item().unwrap().key(), "beta.desktop");
        let pending = app.update(Message::QueryChanged("Ed".to_owned()));
        let cleared = app.update(Message::QueryChanged(String::new()));
        assert!(task_messages(pending).is_empty());
        for message in task_messages(cleared) {
            let _ = app.update(message);
        }
        assert_eq!(app.results.len(), 2);
        assert_eq!(app.selected_item().unwrap().key(), "beta.desktop");
    }

    #[test]
    fn only_successful_launches_report_the_captured_key() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["applications:beta".to_owned()],
            fail_history: true,
            ..TestBackend::default()
        });
        let mut app = backend_app(dir.path(), backend.clone());
        let query = app.update(Message::QueryChanged("Editor".to_owned()));
        for message in task_messages(query) {
            let _ = app.update(message);
        }
        let failed = app.update(Message::LaunchSelected);
        assert!(matches!(
            &task_messages(failed)[0],
            Message::Launched(Err(_))
        ));
        assert!(backend.recorded.lock().unwrap().is_empty());
        app.launcher = Arc::new(RecordingLaunchTarget::default());
        let launched = app.update(Message::LaunchSelected);
        let _ = app.update(Message::QueryChanged(String::new()));
        assert!(
            matches!(&task_messages(launched)[0], Message::Launched(Ok(()))),
            "history failure is not a launch failure"
        );
        assert_eq!(*backend.recorded.lock().unwrap(), ["beta.desktop"]);
    }

    #[test]
    fn iced_executor_drives_tokio_tasks() {
        let executor = <iced::executor::Default as iced::Executor>::new().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        iced::Executor::spawn(&executor, async move {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            sender.send(()).unwrap();
        });
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }

    #[test]
    fn a_desktop_link_is_searchable_without_application_actions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manual.desktop"), "[Desktop Entry]\nType=Link\nName=Manual\nURL=file:///manual.pdf\nActions=invalid;\n[Desktop Action invalid]\nName=Invalid\nExec=wrong\n").unwrap();
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        let _ = app.update(Message::QueryChanged("Manual".to_owned()));
        assert_eq!(app.results.len(), 1);
        let item = app.selected_item().unwrap();
        assert_eq!(item.entry().url(), Some("file:///manual.pdf"));
        let sections = actions_for_app(item);
        assert_eq!(sections[0].actions.len(), 1);
        assert_eq!(sections[0].actions[0].id.as_deref(), Some(APP_OPEN));
    }

    #[test]
    fn a_root_application_exposes_and_dispatches_its_desktop_action_in_the_panel() {
        use iced::futures::{StreamExt, executor::block_on};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("browser.desktop"), "[Desktop Entry]\nType=Application\nName=Webbrowser\nExec=parent\nActions=private;\n[Desktop Action private]\nName=Private Window\nExec=private-app\n").unwrap();
        let index = AppIndex::builder().dir(dir.path()).build();
        let launcher = Arc::new(RecordingLaunchTarget::default());
        let mut app = LauncherApp::with_index(index).with_launcher(launcher.clone());
        let _ = app.update(Message::QueryChanged("Webbrowser".to_owned()));
        // Set Default Browser's subtitle matches too; only the application
        // rows are in question here.
        assert_eq!(
            app.results
                .iter()
                .filter(|row| matches!(row, RootRow::App(_)))
                .count(),
            1,
            "actions are not duplicate root rows"
        );
        assert!(matches!(app.results[0], RootRow::App(_)));
        let backend = Arc::new(TestBackend::default());
        app.backend = Some(backend.clone());
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("Private".to_owned()));
        assert_eq!(
            app.panel
                .as_ref()
                .unwrap()
                .selected_action()
                .unwrap()
                .id
                .as_deref(),
            Some("app.desktop:private")
        );
        let task = app.update(Message::PanelActivate);
        assert!(app.panel.is_none());
        let stream = iced_winit::runtime::task::into_stream(task).unwrap();
        let _ = block_on(stream.collect::<Vec<_>>());
        assert_eq!(*launcher.0.lock().unwrap(), [Some("private".to_owned())]);
        assert_eq!(*backend.recorded.lock().unwrap(), ["browser.desktop"]);
    }

    #[test]
    fn filtered_copy_actions_write_the_selected_apps_name_and_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let expected_name = app.selected_item().expect("selected").name().to_owned();
        let expected_path = app
            .selected_item()
            .expect("selected")
            .path()
            .expect("desktop path")
            .to_string_lossy()
            .into_owned();
        for (filter, expected) in [("name", expected_name), ("path", expected_path)] {
            let _ = app.update(Message::TogglePanel);
            let _ = app.update(Message::PanelFilterChanged(filter.to_owned()));
            let task = app.update(pressed(iced::keyboard::key::Named::Enter));
            assert_eq!(clipboard_writes(task), vec![expected]);
            assert!(app.panel.is_none());
            assert_eq!(app.query, "fi");
        }
    }

    #[test]
    fn dispatch_uses_identity_even_when_the_label_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let expected = app.selected_item().expect("selected").name().to_owned();
        let _ = app.update(Message::TogglePanel);
        let panel = app.panel.as_mut().expect("panel");
        panel.sections[1].actions[0].title = "Copier le nom".to_owned();
        panel.set_filter("Copier le nom".to_owned());
        let task = app.update(Message::PanelActivate);
        assert_eq!(clipboard_writes(task), vec![expected]);
    }

    #[test]
    fn an_empty_filter_result_cannot_launch_or_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("zzzznotanaction".to_owned()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(iced_winit::runtime::task::into_stream(task).is_none());
        assert!(app.panel.is_some());
    }

    #[test]
    fn changing_the_root_query_invalidates_its_panel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::QueryChanged("terminal".to_owned()));
        assert!(app.panel.is_none());
        assert_eq!(app.selected_item().expect("selected").name(), "Terminal");
        assert!(clipboard_writes(app.update(Message::PanelActivate)).is_empty());
    }

    #[test]
    fn clicking_an_action_copies_but_clicking_a_heading_does_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let expected = app.selected_item().expect("selected").name().to_owned();
        let _ = app.update(Message::TogglePanel);
        assert!(clipboard_writes(app.update(Message::PanelClicked(2))).is_empty());
        assert!(app.panel.is_some());
        let messages = {
            let mut ui = iced_test::simulator(app.view());
            ui.click("Copy name").expect("click the action");
            ui.into_messages().collect::<Vec<_>>()
        };
        assert_eq!(messages.len(), 1);
        let writes: Vec<_> = messages
            .into_iter()
            .flat_map(|message| clipboard_writes(app.update(message)))
            .collect();
        assert_eq!(writes, vec![expected]);
    }

    #[test]
    fn typing_in_the_panel_emits_only_panel_edits_and_no_submit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        let mut ui = iced_test::simulator(app.view());
        ui.click(iced_test::selector::id(PANEL_INPUT))
            .expect("panel input");
        ui.typewrite("n");
        ui.tap_key(iced::keyboard::key::Named::Enter);
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            matches!(messages.as_slice(), [Message::PanelFilterChanged(filter)] if filter == "n")
        );
    }

    #[test]
    fn the_selection_clamps_by_default_because_the_cpp_does() {
        // This used to assert the opposite. `Config::wrapNavigation` is
        // `false` in the engine being ported, so Up at the top stays at the
        // top -- and `compass_core::list_navigation` holds the rule.
        assert_eq!(next_selection(3, 0, Direction::Up, false), 0);
        assert_eq!(next_selection(3, 2, Direction::Down, false), 2);
        assert_eq!(next_selection(3, 0, Direction::Down, false), 1);
        assert_eq!(next_selection(3, 2, Direction::Up, false), 1);
    }

    /// Themes compare by value, and a failed comparison prints two whole
    /// palettes. The name is what distinguishes ours, so assert on that.
    fn theme_name(app: &LauncherApp) -> String {
        app.theme().to_string()
    }

    /// The log every test in this binary writes into, installed once.
    ///
    /// A thread-local `with_default` subscriber was the obvious way to do this
    /// and does not work here: `tracing` keeps a PROCESS-GLOBAL max-level
    /// hint, so with tests running in parallel a `debug!` can be filtered out
    /// while a thread-local DEBUG subscriber is active. The test passed alone
    /// and failed in the suite -- 38/38 pass under `--test-threads=1`, which
    /// is how that was pinned down rather than guessed.
    ///
    /// Installing one global subscriber at DEBUG raises that hint for the
    /// whole run and keeps it raised, so there is no race to lose.
    fn log() -> &'static std::sync::Mutex<Vec<u8>> {
        use std::sync::{Mutex, OnceLock};
        use tracing_subscriber::fmt::MakeWriter;

        static LOG: OnceLock<&'static Mutex<Vec<u8>>> = OnceLock::new();

        #[derive(Clone, Copy)]
        struct Writer(&'static Mutex<Vec<u8>>);

        impl std::io::Write for Writer {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> MakeWriter<'a> for Writer {
            type Writer = Self;
            fn make_writer(&'a self) -> Self::Writer {
                *self
            }
        }

        LOG.get_or_init(|| {
            let buffer: &'static Mutex<Vec<u8>> = Box::leak(Box::new(Mutex::new(Vec::new())));
            let subscriber = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(Writer(buffer))
                .without_time()
                .finish();
            // Another test binary in the same process may have got here first;
            // either way a DEBUG subscriber is installed, which is what this
            // needs.
            let _ = tracing::subscriber::set_global_default(subscriber);
            buffer
        })
    }

    fn logged() -> String {
        let bytes = log()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn every_message_leaves_its_state_in_the_log() {
        // A query no other test uses, so the lines below are this test's even
        // though every test in the binary shares one log. Counting lines that
        // merely say `compass_ui::state` would race with whatever else is
        // running.
        const QUERY: &str = "compass-log-state-sentinel";

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("log-state.desktop"),
            format!("[Desktop Entry]\nType=Application\nName={QUERY}\nExec=true\n"),
        )
        .expect("log-state fixture");
        let _ = log();
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged(QUERY.to_owned()));
        let _ = app.update(Message::TogglePanel);

        let log = logged();
        let mine: Vec<&str> = log
            .lines()
            .filter(|line| line.contains(&format!(r#"query="{QUERY}""#)))
            .collect();

        // If `update` stops emitting, this fails here in milliseconds instead
        // of in a 25-minute VM run against a line the launcher no longer
        // writes. A control confirmed the other state-line tests do NOT catch
        // that: deleting the `tracing::debug!` left all of them green.
        assert_eq!(
            mine.len(),
            2,
            "one line per message, no more and no fewer:\n{log}"
        );
        assert!(mine[0].contains("compass_ui::state"), "{:?}", mine[0]);
        assert!(mine[0].contains("panel=closed"), "{:?}", mine[0]);
        assert!(mine[1].contains("panel=open"), "{:?}", mine[1]);
    }

    #[test]
    fn the_state_line_names_what_is_on_screen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());

        let _ = app.update(Message::QueryChanged("fir".to_owned()));
        let line = app.state_line();
        // Everything the VM tier wants to assert, in one greppable line.
        assert!(line.contains(r#"query="fir""#), "{line}");
        assert!(line.contains("results=1"), "{line}");
        assert!(line.contains("selected=0"), "{line}");
        assert!(line.contains(r#"selected_title="Firefox""#), "{line}");
        assert!(line.contains("panel=closed"), "{line}");
    }

    #[test]
    fn the_state_line_follows_the_selection_rather_than_the_index_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());

        // A query matching more than one row, so moving down has somewhere to
        // go. Not the empty query: the launcher shows nothing for that, which
        // this test discovered. The title is what makes "selected=1" mean
        // anything.
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let first = app.state_line();
        let _ = app.update(Message::MoveSelection(Direction::Down));
        let second = app.state_line();

        assert!(first.contains("selected=0"), "{first}");
        assert!(second.contains("selected=1"), "{second}");
        assert_ne!(
            first
                .split_once("selected_title=")
                .map(|(_, rest)| rest.to_owned()),
            second
                .split_once("selected_title=")
                .map(|(_, rest)| rest.to_owned()),
            "the title must move with the selection, not just the number"
        );
    }

    #[test]
    fn the_state_line_describes_the_panel_when_it_is_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fir".to_owned()));

        assert!(app.state_line().contains("panel=closed"));

        let _ = app.update(Message::TogglePanel);
        let line = app.state_line();
        assert!(line.contains("panel=open"), "{line}");
        assert!(line.contains("panel_selected=0"), "{line}");
        // The first SELECTABLE row, which is the thing the VM frame had to be
        // read by eye to confirm. Now it is a string.
        assert!(line.contains(r#"panel_title="Open""#), "{line}");

        let _ = app.update(Message::PanelFilterChanged("copy".to_owned()));
        let filtered = app.state_line();
        assert!(filtered.contains(r#"panel_filter="copy""#), "{filtered}");
        assert!(filtered.contains(r#"panel_title="Copy"#), "{filtered}");

        let _ = app.update(Message::TogglePanel);
        assert!(app.state_line().contains("panel=closed"));
    }

    #[test]
    fn the_state_line_says_when_nothing_matched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzz".to_owned()));
        let line = app.state_line();
        // "results=0" and "no window" are different failures and the tier has
        // to be able to tell them apart from a log.
        assert!(line.contains("results=0"), "{line}");
        assert!(line.contains("selected_title=none"), "{line}");
    }

    #[test]
    fn an_appearance_change_reaches_the_theme() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        assert_eq!(
            theme_name(&app),
            design::theme(Appearance::Light).to_string()
        );

        let _ = app.update(Message::AppearanceChanged(Appearance::Light));
        // Asserted through `theme()` rather than the field, because the field
        // being right while the theme is built from something else is exactly
        // the bug worth catching.
        assert_eq!(
            theme_name(&app),
            design::theme(Appearance::Light).to_string()
        );
    }

    #[test]
    fn a_window_nobody_is_feeding_keeps_the_appearance_it_started_with() {
        let (mut app, _task) = LauncherApp::new(AppFlags {
            appearance: Appearance::Light,
            appearance_link: None,
            ..AppFlags::default()
        });

        // No portal, no feeder: whatever it opened as is what it stays as. A
        // launcher that fell back to dark here would change colour on every
        // desktop without a Settings portal.
        let opened_as = theme_name(&app);
        assert_eq!(opened_as, design::theme(Appearance::Light).to_string());
        let _ = app.update(Message::QueryChanged("fir".to_owned()));
        let _ = app.update(Message::TogglePanel);
        assert_eq!(theme_name(&app), opened_as);
    }

    #[test]
    fn the_starting_appearance_is_what_the_flags_say() {
        for appearance in Appearance::ALL {
            let (app, _task) = LauncherApp::new(AppFlags {
                appearance,
                ..AppFlags::default()
            });
            assert_eq!(theme_name(&app), design::theme(appearance).to_string());
        }
    }

    #[test]
    fn theme_preview_restores_on_cancel_and_clears_on_commit() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::default());
        app.apply(AppFlags {
            theme: crate::theme::Theme::System,
            appearance: Appearance::Light,
            ..AppFlags::default()
        });
        let before = theme_name(&app);
        let _ = app.update(Message::ThemePreview(crate::theme::Theme::Dracula));
        assert_ne!(theme_name(&app), before);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Dracula);
        let _ = app.update(Message::ThemeCancel);
        assert_eq!(theme_name(&app), before);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::System);

        let _ = app.update(Message::ThemePreview(crate::theme::Theme::Nord));
        let _ = app.update(Message::ThemeCommit);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Nord);
        assert!(app.view.theme_preview.is_none());
    }

    #[test]
    fn theme_preview_return_to_system_via_cancel_and_commit() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::default());
        app.apply(AppFlags {
            theme: crate::theme::Theme::Dracula,
            appearance: Appearance::Dark,
            ..AppFlags::default()
        });
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Dracula);
        let _ = app.update(Message::ThemePreview(crate::theme::Theme::System));
        assert_eq!(app.view.theme_choice, crate::theme::Theme::System);
        let _ = app.update(Message::ThemeCancel);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Dracula);
        let _ = app.update(Message::ThemePreview(crate::theme::Theme::System));
        let _ = app.update(Message::ThemeCommit);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::System);
        assert!(app.view.theme_preview.is_none());
    }

    #[test]
    fn theme_and_preset_are_independently_selectable_in_app() {
        // #153: colour themes and layout presets independently selectable
        let mut app = LauncherApp::with_index(compass_core::AppIndex::default());
        app.apply(AppFlags {
            theme: crate::theme::Theme::Nord,
            appearance: Appearance::Dark,
            ..AppFlags::default()
        });
        // Simulate that preset is stored separately — app holds theme_choice,
        // preset is in appearance config, but the UI must not couple them.
        // This test documents the contract: changing one must not change the other.
        let theme_before = app.view.theme_choice;
        let _ = app.update(Message::ThemePreview(crate::theme::Theme::Gruvbox));
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Gruvbox);
        assert_eq!(app.view.theme_preview, Some(theme_before));
        let _ = app.update(Message::ThemeCancel);
        assert_eq!(app.view.theme_choice, theme_before);
    }

    #[test]
    fn the_selection_wraps_when_the_setting_asks_for_it() {
        assert_eq!(next_selection(3, 0, Direction::Up, true), 2);
        assert_eq!(next_selection(3, 2, Direction::Down, true), 0);
    }

    #[test]
    fn selection_on_an_empty_list_is_not_an_index_into_nothing() {
        assert_eq!(next_selection(0, 0, Direction::Up, false), 0);
        assert_eq!(next_selection(0, 0, Direction::Down, true), 0);
    }

    #[test]
    fn a_selection_left_past_the_end_is_clamped_rather_than_underflowing() {
        // Reachable if a list shrinks without the selection being reset. The
        // arithmetic here is unsigned, so getting this wrong is a panic.
        assert_eq!(next_selection(2, 9, Direction::Up, false), 0);
        assert_eq!(next_selection(1, 5, Direction::Down, false), 0);
    }

    #[test]
    fn typing_ranks_and_selects_the_first_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        assert_eq!(app.selected, 0);
        assert_eq!(
            app.selected_item().map(compass_core::AppItem::name),
            Some("Firefox")
        );
    }

    #[test]
    fn a_new_query_resets_the_selection() {
        // Otherwise the old position points into a different list and the user
        // launches something they never looked at.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        // "fi" matches Files and Firefox. A single "e" matches nothing at all:
        // the coherence rule (ADR-0006) drops a one-character query that is not
        // a word prefix, which is correct and cost this test a first draft.
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let _ = app.update(Message::MoveSelection(Direction::Down));
        assert_ne!(app.selected, 0, "precondition: the selection moved");
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn an_empty_query_shows_nothing_and_selects_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        let _ = app.update(Message::QueryChanged("   ".to_owned()));
        assert!(app.results.is_empty());
        assert!(app.selected_item().is_none());
    }

    #[test]
    fn launching_with_no_results_does_nothing_rather_than_panicking() {
        // Enter on an empty list is the most ordinary way to reach this.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzznotathing".to_owned()));
        assert!(app.results.is_empty());
        let _ = app.update(Message::LaunchSelected);
    }

    #[test]
    fn a_failed_launch_is_shown_and_cleared_by_the_next_keystroke() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::Launched(Err("no Exec key".to_owned())));
        assert_eq!(app.error.as_deref(), Some("could not launch: no Exec key"));
        let _ = app.update(Message::QueryChanged("f".to_owned()));
        assert!(app.error.is_none(), "a new query should clear the error");
    }

    /// Builds a `KeyPressed` for a named key, as the subscription delivers it.
    /// A `Ctrl`+`digit` press. `chord` below builds it; this names the intent.
    pub(super) fn ctrl_digit(digit: &str) -> Message {
        chord(digit, iced::keyboard::Modifiers::CTRL)
    }

    fn pressed(key: iced::keyboard::key::Named) -> Message {
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
    fn chord(character: &str, modifiers: iced::keyboard::Modifiers) -> Message {
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
    pub(super) fn app_with_rows(dir: &std::path::Path) -> LauncherApp {
        let mut app = app(dir);
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        assert!(app.results.len() > 1, "need several rows to move between");
        app
    }

    #[test]
    fn a_power_command_asks_first_and_only_a_yes_runs_it() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["commands:reboot".to_owned()],
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        let open = |app: &mut LauncherApp| {
            for message in task_messages(app.update(Message::QueryChanged("reboot".into()))) {
                let _ = app.update(message);
            }
            app.selected = app
                .results
                .iter()
                .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == "reboot"))
                .expect("Reboot System is in root search");
            let _ = app.update(Message::LaunchSelected);
        };
        open(&mut app);
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Enter: Reboot System    Esc: cancel").is_ok());
        }
        let cancelled = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(app.power_confirm.is_none());
        assert!(
            cancelled.units() > 0,
            "Escape gives the search field its focus back (P-03)"
        );
        assert!(
            backend.powered.lock().unwrap().is_empty(),
            "Escape runs nothing"
        );

        open(&mut app);
        let mut pending = task_messages(app.update(pressed(iced::keyboard::key::Named::Enter)));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        assert_eq!(backend.powered.lock().unwrap().as_slice(), ["reboot"]);
    }

    #[test]
    fn the_confirm_preference_decides_whether_a_power_command_asks() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["commands:reboot".to_owned(), "commands:lock".to_owned()],
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        app.power_asks = [("reboot".to_owned(), false), ("lock".to_owned(), true)].into();
        let launch = |app: &mut LauncherApp, query: &str, entrypoint: &str| {
            for message in task_messages(app.update(Message::QueryChanged(query.into()))) {
                let _ = app.update(message);
            }
            app.selected = app
                .results
                .iter()
                .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == entrypoint))
                .expect("the command is in root search");
            let mut pending = task_messages(app.update(Message::LaunchSelected));
            while let Some(message) = pending.pop() {
                pending.extend(task_messages(app.update(message)));
            }
        };

        launch(&mut app, "reboot", "reboot");
        assert!(app.power_confirm.is_none(), "reboot was told not to ask");
        assert_eq!(backend.powered.lock().unwrap().as_slice(), ["reboot"]);

        launch(&mut app, "lock", "lock");
        assert_eq!(
            app.power_confirm.map(|power| power.id),
            Some("lock"),
            "lock was told to ask, though by default it does not"
        );
        assert_eq!(backend.powered.lock().unwrap().as_slice(), ["reboot"]);
    }

    #[test]
    fn a_media_command_runs_at_once_and_shows_why_it_did_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["commands:next-track".to_owned()],
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        for message in task_messages(app.update(Message::QueryChanged("next track".into()))) {
            let _ = app.update(message);
        }
        app.selected = app
            .results
            .iter()
            .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == "next-track"))
            .expect("Next Track is in root search");
        let mut pending = task_messages(app.update(Message::LaunchSelected));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        assert!(app.power_confirm.is_none(), "media commands do not ask");
        assert_eq!(backend.played.lock().unwrap().as_slice(), ["next-track"]);
        assert_eq!(app.error.as_deref(), Some("No media player is running"));
    }

    #[test]
    fn a_media_command_runs_with_the_player_chosen_in_its_form() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["commands:play-pause".to_owned()],
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        for message in task_messages(app.update(Message::QueryChanged("play pause".into()))) {
            let _ = app.update(message);
        }
        app.selected = app
            .results
            .iter()
            .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == "play-pause"))
            .expect("Play / Pause is in root search");
        let _ = app.update(Message::TogglePanel);
        let panel = app.panel.as_ref().expect("a media command has a panel");
        let titles: Vec<&str> = panel.sections[0]
            .actions
            .iter()
            .map(|a| a.title.as_str())
            .collect();
        assert_eq!(titles, ["Play / Pause", "Choose player…"]);
        let _ = app.update(Message::PanelMove(Direction::Down));
        let _ = app.update(Message::PanelActivate);
        assert!(
            matches!(&app.page, Page::Preferences(page)
                if page.purpose == crate::preferences_page::Purpose::MediaArguments),
            "{}",
            app.state_line()
        );
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text(" spotify ".into()),
        ));
        let mut pending = task_messages(app.update(pressed(iced::keyboard::key::Named::Enter)));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        assert_eq!(
            backend.played.lock().unwrap().as_slice(),
            ["play-pause spotify"]
        );
    }

    #[test]
    fn now_playing_lists_the_players_and_controls_the_selected_one() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _entered = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["commands:now-playing".to_owned()],
            ..TestBackend::default()
        });
        *backend.players.lock().unwrap() = vec![
            crate::backend::MediaPlayerRow {
                id: "org.mpris.MediaPlayer2.firefox".into(),
                identity: "Firefox".into(),
                ..Default::default()
            },
            crate::backend::MediaPlayerRow {
                id: "org.mpris.MediaPlayer2.spotify".into(),
                identity: "Spotify".into(),
                title: "Blue Monday".into(),
                artist: "New Order".into(),
                playing: true,
                can_go_next: true,
                ..Default::default()
            },
        ];
        let mut app = extension_app(dir.path(), backend.clone());
        for message in task_messages(app.update(Message::QueryChanged("now playing".into()))) {
            let _ = app.update(message);
        }
        app.selected = app
            .results
            .iter()
            .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == "now-playing"))
            .expect("Now Playing is in root search");
        let mut pending = task_messages(app.update(Message::LaunchSelected));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        let Page::NowPlaying(page) = &app.page else {
            panic!("Now Playing did not open: {}", app.state_line());
        };
        assert_eq!(page.shown.len(), 2);

        for message in task_messages(app.update(Message::NowPlayingQueryChanged("order".into()))) {
            let _ = app.update(message);
        }
        let _ = app.update(Message::TogglePanel);
        let titles: Vec<String> = app.panel.as_ref().expect("a player has a panel").sections[0]
            .actions
            .iter()
            .map(|a| a.title.clone())
            .collect();
        assert_eq!(titles, ["Pause", "Next Track"]);
        let _ = app.update(Message::TogglePanel);

        let mut pending = task_messages(app.update(pressed(iced::keyboard::key::Named::Enter)));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        assert_eq!(
            backend.controlled.lock().unwrap().as_slice(),
            [(
                "org.mpris.MediaPlayer2.spotify".to_owned(),
                crate::backend::MediaAction::PlayPause
            )]
        );
        let Page::NowPlaying(page) = &app.page else {
            panic!("Now Playing closed: {}", app.state_line());
        };
        let row = page.selected_row().expect("still selected");
        assert!(!row.playing && row.paused, "the list was reloaded");
    }

    #[test]
    fn the_emoji_picker_opens_from_root_search_and_filters() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("emoji".to_owned()));
        let position = app
            .results
            .iter()
            .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == "search-emojis"))
            .expect("the command is in root search");
        app.selected = position;
        let _ = app.update(Message::LaunchSelected);
        let _ = app.update(Message::EmojiQueryChanged("thumbs up".into()));
        let Page::Emoji(page) = &app.page else {
            panic!("not the picker: {}", app.state_line());
        };
        assert_eq!(page.selected_glyph().map(|g| g.character), Some("👍"));
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("thumbs up").is_ok());
    }

    #[test]
    fn the_picker_remembers_a_pick_a_pin_and_a_keyword_in_its_file() {
        use iced::keyboard::key::Named;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("emojis").join("emojis.json");
        let mut app = app(dir.path());
        app.glyph_path = Some(path.clone());
        app.emoji_skin_tone = Some("dark".to_owned());
        let command = compass_core::commands::by_id("commands:search-emojis").expect("command");
        let _ = app.open_command(command);

        // A pick is copied in the picker's tone and counted.
        let _ = app.update(Message::EmojiQueryChanged("waving hand".into()));
        let copied: Vec<String> =
            iced_winit::runtime::task::into_stream(app.update(pressed(Named::Enter)))
                .map(|stream| {
                    iced::futures::executor::block_on(iced::futures::StreamExt::collect::<Vec<_>>(
                        stream,
                    ))
                })
                .unwrap_or_default()
                .into_iter()
                .filter_map(|action| match action {
                    iced_winit::runtime::Action::Clipboard(
                        iced_winit::runtime::clipboard::Action::Write { contents, .. },
                    ) => Some(contents),
                    _ => None,
                })
                .collect();
        assert_eq!(copied, ["👋\u{1F3FF}"]);
        let _ = app.open_command(command);
        let Page::Emoji(page) = &app.page else {
            panic!("not the picker: {}", app.state_line());
        };
        assert_eq!(page.recent, 1, "the pick is recently used");

        // Pin from the panel.
        let _ = app.update(Message::EmojiQueryChanged("pizza".into()));
        let _ = app.update(Message::TogglePanel);
        let pin = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Pin emoji"))
            .expect("the panel offers to pin");
        let _ = app.update(Message::PanelClicked(pin));

        // A keyword of one's own, through the form and back to the picker.
        let _ = app.update(Message::TogglePanel);
        let edit = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Edit keyword"))
            .expect("the panel offers the keyword form");
        let _ = app.update(Message::PanelClicked(edit));
        assert!(
            matches!(&app.page, Page::Preferences(form)
                if form.purpose == crate::preferences_page::Purpose::GlyphKeywords),
            "{}",
            app.state_line()
        );
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("qqsupper".into()),
        ));
        let _ = app.update(Message::PreferencesSubmit);
        let Page::Emoji(page) = &app.page else {
            panic!("back in the picker: {}", app.state_line());
        };
        assert_eq!(page.query, "pizza", "the picker comes back as it was");

        let stored = compass_core::glyph_service::GlyphService::load_file(&path);
        let pizza = stored.find("🍕").expect("pizza is remembered");
        assert!(pizza.pinned_at.is_some());
        assert_eq!(pizza.keyword.as_deref(), Some("qqsupper"));
        assert_eq!(stored.find("👋").map(|wave| wave.visit_count), Some(1));
    }

    #[test]
    fn a_newer_release_leads_the_empty_query_and_its_panel_opens_or_skips_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(TestBackend::default());
        let url = "https://github.com/tuna-os/compass/releases/tag/v0.2.0";
        *backend.update.lock().unwrap() = Some(crate::backend::UpdateOffer {
            tag: "v0.2.0".into(),
            version: "0.2.0".into(),
            release_url: url.into(),
            current: "v0.1.0".into(),
        });
        let mut app = app(dir.path());
        app.backend = Some(backend.clone());

        // What the window asks each time it opens.
        let task = app.refresh_update_task();
        settle(&mut app, task);
        assert_eq!(app.results.first(), Some(&RootRow::Update));
        assert_eq!(app.root_heading_at(0), Some("Update"));
        assert!(
            app.state_line()
                .contains("selected_title=\"Compass v0.2.0 is available\""),
            "{}",
            app.state_line()
        );
        let offer = app.update.as_ref().expect("held");
        assert_eq!(release_check::subtitle(offer), "You are running v0.1.0");

        // Only for the empty query.
        let task = app.update(Message::QueryChanged("fire".into()));
        settle(&mut app, task);
        assert!(!app.results.contains(&RootRow::Update));
        let task = app.update(Message::QueryChanged(String::new()));
        settle(&mut app, task);
        assert_eq!(app.results.first(), Some(&RootRow::Update));

        // Enter reads the release notes.
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(backend.opened_urls.lock().unwrap().as_slice(), [url]);

        // The panel skips it, and the row goes.
        let _ = app.update(Message::TogglePanel);
        let skip = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Skip this version"))
            .expect("the panel offers to skip");
        assert!(
            app.panel
                .as_ref()
                .and_then(|panel| panel.row_titled("View release notes"))
                .is_some()
        );
        let task = app.update(Message::PanelClicked(skip));
        settle(&mut app, task);
        assert_eq!(backend.skipped.lock().unwrap().as_slice(), ["v0.2.0"]);
        assert!(app.update.is_none());
        assert!(!app.results.contains(&RootRow::Update));
        assert_eq!(app.root_heading_at(0), None);
    }

    #[test]
    fn the_root_panel_favourites_aliases_and_the_up_arrow_recalls_searches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let history = dir.path().join("history").join("search-history.json");
        app.search_history_path = Some(history.clone());

        // Favouriting Firefox from its panel puts it first, under a heading.
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let firefox = app.selected_row().expect("a row");
        let id = app.root_id(firefox).expect("a root item");
        let _ = app.update(Message::TogglePanel);
        let favorite = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Add to favorites"))
            .expect("the panel offers to favourite");
        let _ = app.update(Message::PanelClicked(favorite));
        assert_eq!(app.root_config.favorites, std::slice::from_ref(&id));
        let _ = app.update(Message::QueryChanged(String::new()));
        assert_eq!(app.results.first(), Some(&firefox));
        assert_eq!(app.root_heading_at(0), Some(FAVORITES_HEADING_FOR_TESTS));
        assert_eq!(
            app.results.iter().filter(|row| **row == firefox).count(),
            1,
            "a favourite is not suggested again"
        );

        // An alias through the form.
        let _ = app.update(Message::TogglePanel);
        let alias = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Set alias"))
            .expect("the panel offers an alias");
        let _ = app.update(Message::PanelClicked(alias));
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("ff".into()),
        ));
        let _ = app.update(Message::PreferencesSubmit);
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
        assert_eq!(
            app.app_index
                .root(&id)
                .and_then(|root| root.meta.alias.as_deref()),
            Some("ff")
        );

        // Launching records the search; the up arrow at the top brings it back.
        let _ = app.update(Message::QueryChanged("term".into()));
        let _ = app.update(Message::LaunchSelected);
        let _ = app.update(Message::QueryChanged(String::new()));
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowUp));
        assert_eq!(app.query, "term");
        let stored = compass_core::root_view::SearchHistory::load_file(&history);
        assert_eq!(stored.queries(), ["term"]);

        // A query being typed is not lost to the up arrow: down brings it back.
        let _ = app.update(Message::QueryChanged("zzz".into()));
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowUp));
        assert_eq!(app.query, "term");
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(app.query, "zzz", "{}", app.state_line());
    }

    const FAVORITES_HEADING_FOR_TESTS: &str = "Favorites";

    #[test]
    fn the_picker_pastes_the_glyph_and_copies_where_the_engine_cannot() {
        use iced::keyboard::key::Named;
        let command = compass_core::commands::by_id("commands:search-emojis").expect("command");
        let picker = |backend: Arc<TestBackend>, default_action: &str| {
            let dir = tempfile::tempdir().expect("tempdir");
            let mut app = app(dir.path());
            app.glyph_path = Some(dir.path().join("emojis.json"));
            app.backend = Some(backend);
            app.emoji_default_action = default_action.to_owned();
            let _ = app.open_command(command);
            let _ = app.update(Message::EmojiQueryChanged("waving hand".into()));
            (app, dir)
        };

        // Paste is the default: first in the panel, on Enter, and nothing is
        // copied by the window.
        let backend = Arc::new(TestBackend::default());
        let (mut app, _dir) = picker(backend.clone(), "paste");
        let _ = app.update(Message::TogglePanel);
        let first = app
            .panel
            .as_ref()
            .and_then(|panel| panel.sections.first()?.actions.first().cloned())
            .expect("an action");
        assert_eq!(first.title, "Paste to active window");
        assert_eq!(first.shortcut.as_deref(), Some("enter"));
        let _ = app.update(Message::TogglePanel);
        let task = app.update(pressed(Named::Enter));
        let copied = settle(&mut app, task);
        assert_eq!(backend.pasted.lock().unwrap().as_slice(), ["👋"]);
        assert!(copied.is_empty(), "{copied:?}");
        let Page::Root = app.page else {
            panic!("the picker hides once pasted: {}", app.state_line());
        };

        // Refused (no Shell extension): the glyph is copied instead.
        let backend = Arc::new(TestBackend {
            refuse_paste: true,
            ..TestBackend::default()
        });
        let (mut app, _dir) = picker(backend.clone(), "paste");
        let task = app.update(pressed(Named::Enter));
        assert_eq!(settle(&mut app, task), ["👋"]);
        assert!(backend.pasted.lock().unwrap().is_empty());

        // `defaultAction: copy` puts copy first and on Enter.
        let backend = Arc::new(TestBackend::default());
        let (mut app, _dir) = picker(backend.clone(), "copy");
        let task = app.update(pressed(Named::Enter));
        assert_eq!(settle(&mut app, task), ["👋"]);
        assert!(backend.pasted.lock().unwrap().is_empty());
        let _ = app.open_command(command);
        let _ = app.update(Message::EmojiQueryChanged("waving hand".into()));
        let _ = app.update(Message::TogglePanel);
        let titles: Vec<String> = app.panel.as_ref().unwrap().sections[0]
            .actions
            .iter()
            .map(|action| action.title.clone())
            .collect();
        assert_eq!(titles[..2], ["Copy", "Paste to active window"]);
    }

    fn key_event(
        pressed: bool,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Message {
        let physical = iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        );
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
    fn closes(task: Task<Message>, id: window::Id) -> bool {
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
    fn shown(dir: &std::path::Path) -> (LauncherApp, window::Id, Box<dyn std::any::Any>) {
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

    #[test]
    fn losing_the_focus_hides_the_launcher_when_close_on_focus_loss_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        app.close_on_focus_loss = true;
        let _ = app.update(Message::WindowFocusChanged(true));
        assert!(closes(app.update(Message::WindowFocusChanged(false)), id));
        let _ = app.update(Message::Closed(id));
        assert!(!app.is_visible());
    }

    #[test]
    fn losing_the_focus_keeps_the_launcher_when_close_on_focus_loss_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        assert!(!app.closes_on_focus_loss(), "off by default, as in the C++");
        let _ = app.update(Message::WindowFocusChanged(true));
        assert!(!closes(app.update(Message::WindowFocusChanged(false)), id));
        assert!(app.is_visible());
    }

    #[test]
    fn only_losing_a_focus_the_launcher_had_hides_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        app.close_on_focus_loss = true;
        assert!(
            !closes(app.update(Message::WindowFocusChanged(false)), id),
            "never focused: the compositor did not hand it over"
        );
        let _ = app.update(Message::WindowFocusChanged(true));
        app.choosing_files = true;
        assert!(
            !closes(app.update(Message::WindowFocusChanged(false)), id),
            "the file chooser the launcher opened took it"
        );
        assert!(app.is_visible());
    }

    #[test]
    fn toggling_a_launcher_left_behind_another_window_brings_it_back_instead_of_hiding_it() {
        // TIL-03: with `close_on_focus_loss` off, a click on another window
        // leaves the launcher open behind it, and the next toggle hid what the
        // user could not see. It comes back as a new window, which the
        // compositor focuses as it does every summon.
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        let _ = app.update(Message::WindowFocusChanged(true));
        let _ = app.update(Message::WindowFocusChanged(false));
        assert!(app.is_visible(), "close_on_focus_loss is off");

        assert!(
            closes(app.obey(UiCommand::Toggle), id),
            "the old window goes"
        );
        assert!(app.reopen_after_close, "and a new one is on its way");
        let _ = app.update(Message::Closed(id));
        let reopened = app.pending_window.expect("a new window is opening");
        assert_ne!(reopened, id);
    }

    #[test]
    fn toggling_a_focused_launcher_hides_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        let _ = app.update(Message::WindowFocusChanged(true));
        assert!(closes(app.obey(UiCommand::Toggle), id));
        assert!(!app.reopen_after_close);
    }

    #[test]
    fn toggling_a_launcher_that_never_reported_the_focus_hides_it() {
        // A seat with no keyboard reports no focus at all; the launcher must
        // still toggle closed there.
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        assert!(closes(app.obey(UiCommand::Toggle), id));
        assert!(!app.reopen_after_close);
    }

    #[test]
    fn a_click_on_the_backdrop_hides_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        assert!(closes(app.update(Message::BackdropPressed), id));
    }

    #[test]
    fn a_toplevel_larger_than_its_monitor_is_shrunk_to_fit_and_opens_fitted_next_time() {
        // TIL-04 on the toplevel path: 1280×800 at 200 % is 640×400.
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        let wanted = app.window_config.size;
        let _ = app.update(Message::WindowSized {
            id,
            size: wanted,
            opened: true,
        });
        let resize = app.update(Message::MonitorSized(
            id,
            Some(iced::Size::new(640.0, 400.0)),
        ));
        assert!(resize.units() > 0, "the window is resized to fit");
        assert_eq!(app.wanted_window_size(), iced::Size::new(624.0, 384.0));
        // A monitor with room changes nothing.
        let _ = app.update(Message::WindowSized {
            id,
            size: iced::Size::new(624.0, 384.0),
            opened: false,
        });
        let _ = app.update(Message::MonitorSized(
            id,
            Some(iced::Size::new(1920.0, 1080.0)),
        ));
        assert_eq!(app.wanted_window_size(), wanted);
    }

    #[test]
    fn the_recorder_suspends_the_global_shortcuts_while_it_captures() {
        use iced::keyboard::{Key, Modifiers, key::Named};
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(TestBackend::default());
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let _ = app.update(Message::TogglePanel);
        let row = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Set global shortcut"))
            .expect("the panel offers a shortcut");
        // Attached from here: the fake engine searches nothing.
        app.backend = Some(backend.clone());
        let task = app.update(Message::PanelClicked(row));
        settle(&mut app, task);
        assert!(app.capture_reported());
        assert_eq!(backend.captures.lock().unwrap().as_slice(), [true]);

        // The launcher hotkey and the launcher's own keys are taken.
        let task = app.update(key_event(true, Key::Named(Named::Space), Modifiers::LOGO));
        settle(&mut app, task);
        let status = |app: &LauncherApp| {
            app.panel
                .as_ref()
                .and_then(|p| p.recorder.as_ref())
                .map(|r| r.status.clone())
        };
        assert_eq!(
            status(&app).as_deref(),
            Some("Already bound to \"the launcher hotkey\"")
        );
        let task = app.update(key_event(true, Key::Character("b".into()), Modifiers::CTRL));
        settle(&mut app, task);
        assert_eq!(
            status(&app).as_deref(),
            Some("Already bound to \"Toggle action panel\"")
        );

        let task = app.update(key_event(
            true,
            Key::Named(Named::Escape),
            Modifiers::empty(),
        ));
        settle(&mut app, task);
        assert!(!app.capture_reported());
        assert_eq!(backend.captures.lock().unwrap().as_slice(), [true, false]);
    }

    #[test]
    fn the_recorder_shows_the_desktops_refusal_and_keeps_what_it_takes() {
        use iced::keyboard::{Key, Modifiers};
        let dir = tempfile::tempdir().expect("tempdir");
        let open = |backend: Arc<TestBackend>| {
            let mut app = app(dir.path());
            let _ = app.update(Message::QueryChanged("firefox".into()));
            let id = app.root_id(app.selected_row().unwrap()).unwrap();
            let _ = app.update(Message::TogglePanel);
            let row = app
                .panel
                .as_ref()
                .and_then(|panel| panel.row_titled("Set global shortcut"))
                .expect("the panel offers a shortcut");
            app.backend = Some(backend);
            let task = app.update(Message::PanelClicked(row));
            settle(&mut app, task);
            (app, id)
        };

        let refusing = Arc::new(TestBackend {
            probe_refusal: Some("The compositor has already bound super+Q".into()),
            ..TestBackend::default()
        });
        let (mut app, _) = open(refusing.clone());
        let task = app.update(key_event(true, Key::Character("q".into()), Modifiers::LOGO));
        settle(&mut app, task);
        assert_eq!(refusing.probes.lock().unwrap().as_slice(), ["super+Q"]);
        let recorder = app
            .panel
            .as_ref()
            .and_then(|p| p.recorder.as_ref())
            .expect("the recorder stays open");
        assert_eq!(recorder.status, "The compositor has already bound super+Q");
        assert!(recorder.error);
        assert!(
            refusing.root_edits.lock().unwrap().is_empty(),
            "nothing kept"
        );

        let taking = Arc::new(TestBackend::default());
        let (mut app, id) = open(taking.clone());
        let task = app.update(key_event(true, Key::Character("q".into()), Modifiers::LOGO));
        settle(&mut app, task);
        assert_eq!(taking.probes.lock().unwrap().as_slice(), ["super+Q"]);
        assert!(
            app.panel.is_none(),
            "a combination the desktop takes is kept"
        );
        assert_eq!(
            app.app_index
                .root(&id)
                .and_then(|root| root.meta.shortcut.clone())
                .as_deref(),
            Some("super+Q")
        );
    }

    #[test]
    fn the_root_panel_records_an_items_shortcut_and_backspace_removes_it() {
        use iced::keyboard::{Key, Modifiers, key::Named};
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let id = app.root_id(app.selected_row().unwrap()).unwrap();
        let open_recorder = |app: &mut LauncherApp| {
            let _ = app.update(Message::TogglePanel);
            assert!(!app.shortcuts_inhibited(), "the panel alone takes nothing");
            let row = app
                .panel
                .as_ref()
                .and_then(|panel| panel.row_titled("Set global shortcut"))
                .expect("the panel offers a shortcut");
            let _ = app.update(Message::PanelClicked(row));
            assert!(
                app.panel.as_ref().is_some_and(|p| p.recorder.is_some()),
                "the recorder takes the panel's place"
            );
            assert!(
                app.shortcuts_inhibited(),
                "the compositor's shortcuts reach the recorder"
            );
        };
        open_recorder(&mut app);

        // A bare letter is refused and typed nowhere; Control, then K.
        let _ = app.update(key_event(
            true,
            Key::Character("k".into()),
            Modifiers::empty(),
        ));
        let recorder = app
            .panel
            .as_ref()
            .and_then(|p| p.recorder.as_ref())
            .unwrap();
        assert_eq!(recorder.status, compass_core::key_combo::MODIFIER_REQUIRED);
        assert_eq!(app.query, "firefox");
        let _ = app.update(key_event(true, Key::Named(Named::Control), Modifiers::CTRL));
        let _ = app.update(key_event(true, Key::Character("k".into()), Modifiers::CTRL));
        assert!(app.panel.is_none(), "an accepted shortcut closes the panel");
        assert!(!app.shortcuts_inhibited(), "and gives the shortcuts back");
        let shortcut = |app: &LauncherApp| {
            app.app_index
                .root(&id)
                .and_then(|root| root.meta.shortcut.clone())
        };
        assert_eq!(shortcut(&app).as_deref(), Some("control+K"));
        let (provider, entrypoint) = compass_core::root_items::split_entrypoint_id(&id).unwrap();
        assert_eq!(
            app.root_config.providers[provider].entrypoints[entrypoint]
                .shortcut
                .as_deref(),
            Some("control+K")
        );

        // Escape goes back to the actions; Backspace removes the shortcut.
        let _ = app.update(key_event(
            false,
            Key::Named(Named::Control),
            Modifiers::empty(),
        ));
        open_recorder(&mut app);
        let _ = app.update(key_event(
            true,
            Key::Named(Named::Escape),
            Modifiers::empty(),
        ));
        assert!(app.panel.as_ref().is_some_and(|p| p.recorder.is_none()));
        assert!(!app.shortcuts_inhibited(), "Escape gives them back");
        let _ = app.update(Message::TogglePanel);
        open_recorder(&mut app);
        let _ = app.update(key_event(
            true,
            Key::Named(Named::Backspace),
            Modifiers::empty(),
        ));
        assert!(app.panel.is_none());
        assert_eq!(shortcut(&app), None);
    }

    #[test]
    fn a_launch_deeplink_to_a_provider_searches_its_items_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = extension_app(dir.path(), Arc::new(TestBackend::default()));

        let task = app.open_deeplink("vicinae://launch/@someone/hello?fallbackText=wri");
        settle(&mut app, task);
        assert_eq!(app.search_field().0, "Search Hello");
        assert_eq!(app.query, "wri");
        assert_eq!(app.results, [RootRow::Extension(0)], "{}", app.state_line());

        // The empty query lists every item of the provider, and nothing else:
        // no favourites, no calculator, no fallbacks.
        let task = app.open_deeplink("vicinae://launch/applications/");
        settle(&mut app, task);
        assert_eq!(app.search_field().0, "Search Applications");
        assert_eq!(app.results.len(), 3, "{}", app.state_line());
        assert!(app.results.iter().all(|row| matches!(row, RootRow::App(_))));
        let _ = app.update(Message::QueryChanged("fire".into()));
        assert!(
            app.results.iter().all(|row| matches!(row, RootRow::App(_))),
            "{}",
            app.state_line()
        );
        assert_eq!(app.results.len(), 1);

        // Leaving closes it: the next summon is the root again.
        let _ = app.update(Message::Dismiss);
        assert_eq!(app.provider_scope, None);
        assert_eq!(app.search_field().0, "Search…");

        let _ = app.open_deeplink("compass://launch/nothing");
        assert_eq!(
            app.error.as_deref(),
            Some(compass_core::root_items::INVALID_LAUNCH_LINK)
        );
    }

    #[test]
    fn a_launch_deeplink_to_an_item_launches_it_with_its_text() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = extension_app(dir.path(), backend.clone());
        let task = app.open_deeplink("vicinae://launch/@someone/hello/write?fallbackText=hi+there");
        settle(&mut app, task);
        assert_eq!(
            backend.launched.lock().unwrap().as_slice(),
            [(
                "@someone/hello:write".to_owned(),
                Some("hi there".to_owned())
            )]
        );
        assert_eq!(app.provider_scope, None);
    }

    #[test]
    fn fallbacks_open_a_one_argument_shortcut_and_an_extension_with_the_query() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        app.fallbacks = vec![
            "shortcuts:sct-news".into(),
            "shortcuts:sct-docs".into(),
            "files:search".into(),
        ];
        app.query = "serde".into();
        app.search();
        let fallbacks: Vec<RootRow> = app
            .results
            .iter()
            .copied()
            .filter(|row| matches!(row, RootRow::Fallback(_)))
            .collect();
        let docs = app
            .app_index
            .shortcuts()
            .iter()
            .position(|shortcut| shortcut.id == "sct-docs")
            .unwrap();
        assert!(
            matches!(
                fallbacks.as_slice(),
                [
                    RootRow::Fallback(Fallback::Shortcut(at)),
                    RootRow::Fallback(Fallback::Command(_)),
                ] if *at == docs
            ),
            "a shortcut without exactly one argument is no fallback: {fallbacks:?}"
        );
        app.selected = app
            .results
            .iter()
            .position(|row| *row == fallbacks[0])
            .unwrap();
        assert!(
            app.state_line()
                .contains("selected_title=\"Crate Docs\" fallback")
        );
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            backend.opened_shortcuts.lock().unwrap().last(),
            Some(&("sct-docs".to_owned(), vec!["serde".to_owned()]))
        );

        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = extension_app(dir.path(), backend.clone());
        app.fallbacks = vec!["@someone/hello:write".into()];
        app.query = "zzzz".into();
        app.search();
        assert_eq!(
            app.results.last(),
            Some(&RootRow::Fallback(Fallback::Extension(0)))
        );
        app.selected = app.results.len() - 1;
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            backend.launched.lock().unwrap().as_slice(),
            [("@someone/hello:write".to_owned(), Some("zzzz".to_owned()))]
        );
    }

    #[test]
    fn an_alias_and_a_space_open_an_items_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        compass_core::root_items::apply_edit(
            &mut app.root_config,
            "shortcuts:sct-docs",
            &compass_core::root_items::RootEdit::Alias("cd".into()),
        );
        app.app_index.apply_root_config(&app.root_config);
        app.query = "cd".into();
        app.search();
        assert!(
            matches!(app.selected_row(), Some(RootRow::Shortcut(_))),
            "{}",
            app.state_line()
        );
        let task = app.update(Message::QueryChanged("cd ".into()));
        settle(&mut app, task);
        assert!(
            matches!(&app.page, Page::Preferences(form)
                if form.purpose == crate::preferences_page::Purpose::ShortcutArguments),
            "the space opens the arguments rather than being typed: {}",
            app.state_line()
        );
        assert_eq!(app.query, "cd");
        assert!(backend.opened_shortcuts.lock().unwrap().is_empty());
    }

    #[test]
    fn a_calculation_that_matches_nothing_is_answered_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("12*3+6".to_owned()));
        assert_eq!(app.results, [RootRow::Calculator], "{}", app.state_line());
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("42").is_ok() && ui.find("12*3+6").is_ok());
        }
        assert!(app.state_line().contains("selected_title=\"42\""));

        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        assert!(
            !app.results.contains(&RootRow::Calculator),
            "a query that matches applications is not a calculation"
        );
        let _ = app.update(Message::QueryChanged("=2^10".to_owned()));
        assert_eq!(app.results.first(), Some(&RootRow::Calculator));
    }

    #[test]
    fn the_action_panel_is_not_bound_to_the_vim_chord() {
        // Ctrl+K is "move up" in the default Linux scheme. The C++ binds the
        // panel to Ctrl+K on macOS only and Ctrl+B everywhere else, and this
        // is why -- taking Ctrl+K here would break navigation for every user
        // who has configured nothing.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 1, "precondition: Ctrl+J moved down");

        let _ = app.update(chord("k", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_none(), "Ctrl+K must not open the panel");
        assert_eq!(app.selected, 0, "Ctrl+K still moves up");
    }

    #[test]
    fn ctrl_b_opens_and_closes_the_panel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_some(), "Ctrl+B opened it");

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_none(), "and Ctrl+B closed it again");
    }

    #[test]
    fn the_panel_does_not_open_over_nothing() {
        // A panel of actions for no selected row is a panel whose every action
        // fails.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzzzzzz".to_owned()));
        assert!(
            app.selected_item().is_none(),
            "precondition: nothing selected"
        );

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_none());
    }

    #[test]
    fn the_panel_takes_the_arrow_keys_while_it_is_open() {
        // Otherwise the list underneath moves out from under a panel whose
        // actions are for the row that was selected when it opened.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let before = app.selected;

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(app.selected, before, "the list did not move");
        // Row 1 is the divider and row 2 the heading, so the next selectable
        // row is 3. Expecting 1 would have been expecting the selection to
        // land on a divider.
        assert_eq!(
            app.panel.as_ref().map(|panel| panel.selected),
            Some(3),
            "the panel did"
        );
    }

    #[test]
    fn typing_the_panels_letter_without_ctrl_does_not_open_it() {
        // The search field takes ordinary characters, and a launcher whose
        // search box opens a panel when someone types `b` is unusable.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());

        let _ = app.update(chord("b", iced::keyboard::Modifiers::default()));
        assert!(app.panel.is_none());
    }

    #[test]
    fn escape_closes_the_panel_rather_than_the_launcher() {
        // A panel opened by mistake should cost one key, not the whole window.
        // The window has to be open for this to mean anything -- with no
        // window, "the window did not close" is true however Escape is routed.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::Opened(window::Id::unique()));
        assert!(app.window.is_some(), "precondition: a window is open");

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(app.panel.is_none(), "the panel closed");
        assert!(app.window.is_some(), "and the window did not");

        // HALF OF THIS IS NOT CONTROL-BACKED, and it is worth saying which.
        //
        // "The panel closed" fires under a mutation. "The window did not"
        // does not: `conceal` closes the window through a Task and clears
        // `self.window` only when `Message::Closed` comes back, and with no
        // engine link `on_dismiss` returns `Exit`, so a mutation that made
        // Escape dismiss as well as close the panel changes nothing this test
        // can see. Proving it would need an app built around a live
        // `EngineLink`, which is a harness this crate does not have yet.
        //
        // Recorded rather than left looking covered.
    }

    #[test]
    fn the_panel_stops_intercepting_once_it_is_closed() {
        // The follow-on from the test above: with the panel gone, Escape is
        // the launcher's again. What it *does* then is `conceal`, which closes
        // the window through a Task and so is not observable from here — so
        // this asserts the routing rather than the closing, and the test above
        // asserts that the routing was different while the panel was open.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::Opened(window::Id::unique()));
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(app.panel.is_none(), "the panel closed");

        // Arrows reach the list again, which the panel was taking.
        let before = app.selected;
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_ne!(app.selected, before, "the list moved again");
    }

    #[test]
    fn the_vim_chords_move_the_panel_while_it_is_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let before = app.selected;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, before, "the list did not move");
        assert_eq!(app.panel.as_ref().map(|panel| panel.selected), Some(3));
    }

    #[test]
    fn running_an_action_closes_the_panel() {
        // An action that ran and one that is not wired up both leave the panel
        // with nothing more to say, and leaving it open would look like the key
        // had not registered.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));

        let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(app.panel.is_none());
    }

    #[test]
    fn the_vim_chords_move_the_selection_because_they_are_the_linux_default() {
        // `KeyBindingService::getMode` falls back to vim off macOS, so a user
        // who has configured nothing still expects Ctrl+J and Ctrl+K to work.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;
        assert_eq!(app.selected, 0);

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 1, "Ctrl+J moved down");

        let _ = app.update(chord("k", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 0, "and Ctrl+K moved back");
    }

    #[test]
    fn a_bare_letter_is_not_a_chord() {
        // The negative that matters most: a handler that ignored modifiers
        // would make the search field unusable, because every `j` typed would
        // also move the selection.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::empty()));
        assert_eq!(app.selected, 0, "a bare `j` moved the selection");
    }

    #[test]
    fn a_chord_from_another_scheme_does_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Emacs;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 0, "Ctrl+J is vim's, not emacs'");

        let _ = app.update(chord("n", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 1, "Ctrl+N is emacs' down");
    }

    #[test]
    fn the_horizontal_chords_do_nothing_in_a_one_column_list() {
        // Recognised by `compass-core`, dropped here on purpose rather than
        // bound to something they do not mean.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;

        let _ = app.update(chord("l", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(chord("h", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn the_arrows_move_the_selection_through_the_keyboard_subscription() {
        // Goes through `Message::Keyboard` rather than `Message::MoveSelection`
        // on purpose: the direct path is covered above, and what this covers is
        // the match arm that routes a key to it. That arm is what a change to
        // the subscription puts at risk.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        assert!(app.results.len() > 1, "need several rows to move between");
        assert_eq!(app.selected, 0);

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(app.selected, 1, "ArrowDown moved down");

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowUp));
        assert_eq!(app.selected, 0, "and ArrowUp moved back");
    }

    #[test]
    fn a_printable_key_from_the_subscription_does_not_also_type() {
        // THE CONTROL FOR HANDLING A KEY TWICE.
        //
        // `subscription` reads every keyboard event, including ones the search
        // field consumed, so that Escape survives the field being focused. The
        // risk that buys is double handling: if `update` ever grew an arm that
        // appended printable keys to the query, every character would arrive
        // twice -- once here and once through the field's `on_input` -- and
        // "fi" would be typed as "ffii".
        //
        // Nothing here can observe `on_input`, which Iced calls; what it can
        // observe is that this path contributes nothing on its own.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let before = app.query.clone();

        let _ = app.update(Message::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character("x".into()),
            modified_key: iced::keyboard::Key::Character("x".into()),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::default(),
            text: Some("x".into()),
            repeat: false,
        }));

        assert_eq!(app.query, before, "the query is the field's to change");
    }

    #[test]
    fn moving_the_selection_lands_on_the_item_that_gets_launched() {
        // The property that matters: what the caret points at in the view is
        // what LaunchSelected resolves. A off-by-one here launches the wrong
        // application, silently and every time.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let expected: Vec<String> = app
            .results
            .iter()
            .filter_map(|row| match row {
                RootRow::App(i) => app.app_index.items().get(*i),
                RootRow::Command(_)
                | RootRow::Extension(_)
                | RootRow::Shortcut(_)
                | RootRow::Script(_)
                | RootRow::RhaiScript(_)
                | RootRow::Fallback(_)
                | RootRow::Calculator
                | RootRow::Update => None,
            })
            .map(|item| item.name().to_owned())
            .collect();
        assert!(expected.len() > 1, "need several rows: {expected:?}");

        for (position, name) in expected.iter().enumerate() {
            assert_eq!(app.selected, position);
            assert_eq!(
                app.selected_item().map(compass_core::AppItem::name),
                Some(name.as_str())
            );
            let _ = app.update(Message::MoveSelection(Direction::Down));
        }
        // Commands rank after the applications here; walk past them too.
        for _ in expected.len()..app.results.len() {
            let _ = app.update(Message::MoveSelection(Direction::Down));
        }
        assert_eq!(
            app.selected,
            app.results.len() - 1,
            "and it stayed on the last row, because wrap_navigation is off by default"
        );
    }

    // ---- Search Files ----

    fn file_row(path: &str, category: &str) -> crate::backend::FileRow {
        crate::backend::FileRow {
            path: path.into(),
            name: path.rsplit('/').next().unwrap_or(path).into(),
            category: category.into(),
        }
    }

    fn files_app(dir: &std::path::Path) -> (LauncherApp, Arc<TestBackend>) {
        let backend = Arc::new(TestBackend {
            files: vec![
                file_row("/home/me/Documents/quarterly-report.pdf", "Documents"),
                file_row("/home/me/notes.md", "Documents"),
            ],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir));
        app.backend = Some(backend.clone());
        app.query = "search files".into();
        app.search();
        assert_eq!(
            app.selected_row(),
            Some(RootRow::Command(
                compass_core::commands::by_id("commands:search-files").unwrap()
            )),
            "{}",
            app.state_line()
        );
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        (app, backend)
    }

    fn files_page(app: &LauncherApp) -> &crate::files_page::FilesPage {
        let Page::Files(page) = &app.page else {
            panic!("not on Search Files: {}", app.state_line())
        };
        page
    }

    #[test]
    fn search_files_lists_recent_files_then_searches_after_the_debounce() {
        // The debounce sleeps on tokio's timer, which the iced executor
        // provides in the launcher and this runtime provides here.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let _entered = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = files_app(dir.path());

        let page = files_page(&app);
        assert_eq!(page.heading, "Recently Accessed");
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            backend.recorded.lock().unwrap().as_slice(),
            ["commands:search-files"]
        );
        assert!(
            app.state_line().contains("page=files"),
            "{}",
            app.state_line()
        );

        // Two keystrokes inside one debounce: only the second is asked.
        let first = app.update(Message::FilesQueryChanged("quart".into()));
        let second = app.update(Message::FilesQueryChanged("quarterly".into()));
        settle(&mut app, first);
        settle(&mut app, second);
        assert_eq!(
            backend.file_queries.lock().unwrap().as_slice(),
            ["", "quarterly"],
            "the superseded keystroke never reached the engine"
        );
        let page = files_page(&app);
        assert_eq!(page.heading, "Results");
        assert_eq!(
            page.selected_row().map(|row| row.name.as_str()),
            Some("quarterly-report.pdf")
        );

        let back = app.update(pressed(iced::keyboard::key::Named::Escape));
        drop(back);
        assert!(matches!(app.page, Page::Root), "Escape goes back");
    }

    #[test]
    fn enter_opens_the_file_and_ctrl_enter_shows_it_in_the_file_browser() {
        let enter = |modifiers| {
            Message::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
                modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
                physical_key: iced::keyboard::key::Physical::Unidentified(
                    iced::keyboard::key::NativeCode::Unidentified,
                ),
                location: iced::keyboard::Location::Standard,
                modifiers,
                text: None,
                repeat: false,
            })
        };
        let dir = tempfile::tempdir().unwrap();

        let (mut app, backend) = files_app(dir.path());
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(files_page(&app).selected, 1);
        let task = app.update(enter(iced::keyboard::Modifiers::default()));
        settle(&mut app, task);
        assert_eq!(
            backend.opened.lock().unwrap().as_slice(),
            [("/home/me/notes.md".to_owned(), false)]
        );
        assert!(matches!(app.page, Page::Root), "opening hides the launcher");

        let (mut app, backend) = files_app(dir.path());
        let task = app.update(enter(iced::keyboard::Modifiers::CTRL));
        settle(&mut app, task);
        assert_eq!(
            backend.opened.lock().unwrap().as_slice(),
            [("/home/me/Documents/quarterly-report.pdf".to_owned(), true)]
        );
    }

    #[test]
    fn search_files_filters_by_a_remembered_category_and_previews_the_selection() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, "remember the milk").unwrap();
        let picture = dir.path().join("cat.png");
        std::fs::write(&picture, b"png").unwrap();
        let backend = Arc::new(TestBackend {
            files: vec![
                file_row(&notes.to_string_lossy(), "Documents"),
                file_row(&picture.to_string_lossy(), "Images"),
            ],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.view.view_memory =
            crate::view_memory::ViewMemory::load(Some(dir.path().join("view-state.json")));
        open_builtin(&mut app, "search files", "commands:search-files");
        let page = files_page(&app);
        let preview = page.preview.as_ref().expect("the selection is previewed");
        assert_eq!(preview.name, "notes.txt");
        assert_eq!(
            preview.content,
            crate::file_preview::Content::Text("remember the milk".into())
        );
        assert!(preview.modified.is_some());
        let _ = app.view();

        let task = app.update(Message::FilesCategoryChanged("Images".into()));
        settle(&mut app, task);
        let page = files_page(&app);
        assert_eq!(page.rows.len(), 1);
        assert_eq!(
            page.preview.as_ref().map(|p| p.content.clone()),
            Some(crate::file_preview::Content::Image(picture.clone()))
        );
        assert_eq!(
            backend
                .file_queries
                .lock()
                .unwrap()
                .last()
                .map(String::as_str),
            Some(" [Images]")
        );

        // A new opening, even in a new process, starts filtered.
        let mut again = LauncherApp::with_index(index(dir.path()));
        again.backend = Some(backend.clone());
        again.view.view_memory =
            crate::view_memory::ViewMemory::load(Some(dir.path().join("view-state.json")));
        open_builtin(&mut again, "search files", "commands:search-files");
        assert_eq!(files_page(&again).category.as_deref(), Some("Images"));
    }

    #[test]
    fn a_query_offers_search_files_as_a_fallback_that_searches_for_it() {
        // The typed query waits out the indexer's debounce on a timer.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _entered = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            files: vec![file_row("/home/me/zebra-notes.md", "Documents")],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.apply(AppFlags {
            backend: Some(backend.clone()),
            fallbacks: vec!["files:search".into()],
            ..AppFlags::default()
        });
        app.query = "zebra".into();
        app.search();
        assert!(
            matches!(app.results.last(), Some(RootRow::Fallback(Fallback::Command(c)))
                if c.kind == compass_core::commands::CommandKind::SearchFiles),
            "{}",
            app.state_line()
        );
        let _ = app.view();
        app.selected = app.results.len() - 1;
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let page = files_page(&app);
        assert_eq!(page.query, "zebra");
        assert_eq!(page.rows.len(), 1);
        app.query.clear();
        app.search();
        assert!(app.results.is_empty(), "an empty query offers no fallback");
    }

    #[test]
    fn ctrl_b_opens_the_file_panel_and_its_shortcuts_work_with_it_closed() {
        use iced::keyboard::Modifiers;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            files: vec![file_row("/home/me/sunset.png", "Images")],
            file_info: crate::backend::FileActions {
                mime: Some("image/png".into()),
                has_opener: true,
                can_set_wallpaper: true,
                can_paste: true,
            },
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "search files", "commands:search-files");

        let task = app.update(chord("b", Modifiers::CTRL));
        settle(&mut app, task);
        assert!(
            panel_titles(&app).contains(&"Copy file path".to_owned()),
            "{}",
            app.state_line()
        );
        let task = app.update(chord("b", Modifiers::CTRL));
        settle(&mut app, task);
        assert!(app.panel.is_none(), "and Ctrl+B closes it again");

        // Copy file is Ctrl+Shift+C; it needs no panel.
        let task = app.update(chord("C", Modifiers::CTRL | Modifiers::SHIFT));
        settle(&mut app, task);
        assert_eq!(
            backend.file_calls.lock().unwrap().as_slice(),
            [("copy", "/home/me/sunset.png".to_owned())]
        );
        assert!(app.panel.is_none());
    }

    #[test]
    fn ctrl_shift_c_copies_a_root_rows_deeplink_without_opening_the_panel() {
        use iced::keyboard::Modifiers;
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        open_builtin_row(&mut app, "clipboard history");
        assert!(app.panel.is_none());
        let task = app.update(chord("C", Modifiers::CTRL | Modifiers::SHIFT));
        assert_eq!(
            settle(&mut app, task),
            ["compass://launch/commands/clipboard-history"]
        );
        assert!(app.panel.is_none(), "no panel the person never opened");
        // A chord no action claims leaves the panel shut and the list alone.
        let selected = app.selected;
        let task = app.update(chord("y", Modifiers::CTRL | Modifiers::SHIFT));
        settle(&mut app, task);
        assert!(app.panel.is_none());
        assert_eq!(app.selected, selected);
    }

    /// Searches root for `query`, leaving its first row selected.
    fn open_builtin_row(app: &mut LauncherApp, query: &str) {
        app.query = query.into();
        app.search();
        assert!(app.selected_row().is_some(), "{}", app.state_line());
    }

    #[test]
    fn search_files_panel_is_the_cpps_file_actions() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            files: vec![
                file_row("/home/me/sunset.png", "Images"),
                file_row("/home/me/Tool.AppImage", "Documents"),
            ],
            file_info: crate::backend::FileActions {
                mime: Some("image/png".into()),
                has_opener: true,
                can_set_wallpaper: true,
                can_paste: true,
            },
            openers: vec![opener("gimp.desktop", "GIMP", false)],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "search files", "commands:search-files");
        assert_eq!(files_page(&app).rows.len(), 2, "{}", app.state_line());

        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert_eq!(
            panel_titles(&app),
            [
                "Open",
                "Show in file browser",
                "Open with...",
                "Set as wallpaper",
                "Create shortcut",
                "Paste to active window",
                "Copy file",
                "Copy file path",
                "Copy file name",
                "Copy mime type",
            ]
        );
        let task = choose(&mut app, "Copy mime type");
        assert_eq!(settle(&mut app, task), ["image/png"]);

        let reopen = |app: &mut LauncherApp| {
            let _ = app.update(Message::Command(UiCommand::Show));
            open_builtin(app, "search files", "commands:search-files");
            let task = app.update(Message::TogglePanel);
            settle(app, task);
        };
        reopen(&mut app);
        let task = choose(&mut app, "Copy file name");
        assert_eq!(settle(&mut app, task), ["sunset.png"]);
        reopen(&mut app);
        let task = choose(&mut app, "Copy file");
        settle(&mut app, task);
        reopen(&mut app);
        let task = choose(&mut app, "Paste to active window");
        settle(&mut app, task);
        reopen(&mut app);
        let task = choose(&mut app, "Set as wallpaper");
        settle(&mut app, task);
        let Page::Files(page) = &app.page else {
            panic!("a failure stays: {}", app.state_line());
        };
        assert_eq!(
            page.notice.as_deref(),
            Some("Failed to set wallpaper: no backend")
        );
        assert_eq!(
            backend.file_calls.lock().unwrap().as_slice(),
            [
                ("copy", "/home/me/sunset.png".to_owned()),
                ("paste", "/home/me/sunset.png".to_owned()),
                ("wallpaper", "/home/me/sunset.png".to_owned()),
            ]
        );

        // Create shortcut opens the form with the file's name and path.
        let _ = app.update(Message::TogglePanel);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Create shortcut");
        settle(&mut app, task);
        let Page::Preferences(form) = &app.page else {
            panic!("no shortcut form: {}", app.state_line());
        };
        let (name, link, _, _) = crate::shortcuts_page::form_values(form);
        assert_eq!(
            (name.as_str(), link.as_str()),
            ("sunset.png", "/home/me/sunset.png")
        );

        // Open with… lists the file's openers.
        reopen(&mut app);
        let task = choose(&mut app, "Open with...");
        settle(&mut app, task);
        assert!(
            matches!(app.page, Page::OpenWith(_)),
            "{}",
            app.state_line()
        );
        assert_eq!(
            backend
                .opener_lookups
                .lock()
                .unwrap()
                .last()
                .map(String::as_str),
            Some("/home/me/sunset.png")
        );

        // An AppImage runs, and it is primary only when nothing opens it.
        let _ = app.update(Message::Command(UiCommand::Show));
        open_builtin(&mut app, "search files", "commands:search-files");
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert!(panel_titles(&app).contains(&"Run executable".to_owned()));
        let task = choose(&mut app, "Run executable");
        settle(&mut app, task);
        assert_eq!(
            backend.file_calls.lock().unwrap().last(),
            Some(&("run", "/home/me/Tool.AppImage".to_owned()))
        );
        let sections = super::file_actions::file_panel_sections(
            "/x/Tool.AppImage",
            &crate::backend::FileActions::default(),
        );
        assert_eq!(sections[0].actions[0].title, "Run executable");
        assert_eq!(sections[0].actions[0].shortcut.as_deref(), Some("enter"));
        assert!(
            !sections
                .iter()
                .flat_map(|s| &s.actions)
                .any(|a| a.title == "Set as wallpaper" || a.title == "Copy mime type")
        );
    }

    #[test]
    fn search_files_without_an_engine_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.query = "search files".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let page = files_page(&app);
        assert!(
            matches!(&page.status, crate::files_page::Status::Failed(reason)
                if reason.contains("needs the Compass engine")),
            "{:?}",
            page.status
        );
    }

    // ---- Shortcuts ----

    fn stored_shortcut(id: &str, name: &str, url: &str) -> crate::backend::Shortcut {
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
    fn shortcuts_app(dir: &std::path::Path) -> (LauncherApp, Arc<TestBackend>) {
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

    fn open_builtin(app: &mut LauncherApp, query: &str, id: &str) {
        app.query = query.into();
        app.search();
        let Some(RootRow::Command(command)) = app.selected_row() else {
            panic!("{query:?} did not select a command: {}", app.state_line());
        };
        assert_eq!(command.id(), id);
        let task = app.update(Message::LaunchSelected);
        settle(app, task);
    }

    /// One Enter in the form's text field holding `field_text`, delivered as
    /// the runtime delivers it: whatever the field publishes for the press,
    /// then the `KeyPressed` that `listen_with` hands the key handler whether
    /// or not the field captured it.
    fn enter_once_in_form(app: &mut LauncherApp, field_text: &str) {
        let mut messages: Vec<Message> = {
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(800.0, 600.0),
                app.view(),
            );
            ui.click(field_text)
                .unwrap_or_else(|_| panic!("no field holds {field_text:?}: {}", app.state_line()));
            ui.tap_key(iced::keyboard::Key::Named(
                iced::keyboard::key::Named::Enter,
            ));
            ui.into_messages().collect()
        };
        messages.push(pressed(iced::keyboard::key::Named::Enter));
        // Both reach `update` before the engine answers the first: the
        // runtime queues them from the same event, and a save or a run is a
        // round trip to the engine.
        let tasks: Vec<_> = messages
            .into_iter()
            .map(|message| app.update(message))
            .collect();
        for task in tasks {
            settle(app, task);
        }
    }

    #[test]
    fn a_new_form_puts_the_keyboard_on_its_first_field_and_tab_reaches_the_checkbox() {
        use crate::preferences_page::FieldValue;
        use iced::keyboard::key::Named;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.query = "create snippet".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        let focusing = task_actions(task);
        let Page::Preferences(page) = &app.page else {
            panic!("no form: {}", app.state_line());
        };
        assert_eq!(page.focus, Some(0), "the name field, as the form opens");
        assert!(!page.focus_pending);
        assert!(focusing > 0, "and a focus operation went out with it");

        let word = page
            .fields
            .iter()
            .position(|f| f.name == crate::snippets_page::WORD_FIELD)
            .unwrap();
        let before = page.values[word].clone();
        for _ in 0..word {
            let _ = app.update(pressed(Named::Tab));
        }
        let Page::Preferences(page) = &app.page else {
            panic!("left the form");
        };
        assert_eq!(page.focus, Some(word), "Tab reaches the checkbox");
        let _ = app.update(pressed(Named::Space));
        let Page::Preferences(page) = &app.page else {
            panic!("left the form");
        };
        assert_ne!(page.values[word], before, "Space ticks it");
        assert!(matches!(page.values[word], FieldValue::Checked(_)));
        let _ = app.update(pressed(Named::Tab));
        let Page::Preferences(page) = &app.page else {
            panic!("left the form");
        };
        assert_eq!(page.focus, Some(0), "and Tab wraps round to the top");
    }

    #[test]
    fn the_alias_and_arguments_forms_focus_their_field_and_say_what_enter_does() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let _ = app.update(Message::TogglePanel);
        let alias = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Set alias"))
            .expect("the panel offers an alias");
        let _ = app.update(Message::PanelClicked(alias));
        let Page::Preferences(page) = &app.page else {
            panic!("no alias form: {}", app.state_line());
        };
        assert_eq!(page.focus, Some(0));
        assert_eq!(
            crate::preferences_page::Purpose::Arguments.hint(),
            "Enter: run    Esc: back",
            "arguments are this run's and are not saved"
        );
    }

    /// How many actions other than messages a task carries: the widget
    /// operations, such as focusing a field.
    fn task_actions(task: Task<Message>) -> usize {
        use iced::futures::{StreamExt, executor::block_on};
        let Some(stream) = iced_winit::runtime::task::into_stream(task) else {
            return 0;
        };
        block_on(stream.collect::<Vec<_>>())
            .into_iter()
            .filter(|action| matches!(action, iced_winit::runtime::Action::Widget(_)))
            .count()
    }

    #[test]
    fn one_enter_saves_a_new_shortcut_once() {
        use crate::preferences_page::FieldValue;
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        open_builtin(&mut app, "create shortcut", "commands:create-shortcut");
        let Page::Preferences(page) = &app.page else {
            panic!("no form: {}", app.state_line());
        };
        let position = |name: &str| page.fields.iter().position(|f| f.name == name).unwrap();
        let (name, link) = (position("name"), position("link"));
        let _ = app.update(Message::PreferenceEdited(
            name,
            FieldValue::Text("Wiki".into()),
        ));
        let _ = app.update(Message::PreferenceEdited(
            link,
            FieldValue::Text("https://en.wikipedia.org/wiki/{page}".into()),
        ));
        enter_once_in_form(&mut app, "Wiki");
        assert_eq!(
            backend.drafts.lock().unwrap().len(),
            1,
            "{}",
            app.state_line()
        );
    }

    #[test]
    fn one_enter_opens_a_quicklink_with_arguments_once() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        app.query = "crate docs".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("serde".into()),
        ));
        enter_once_in_form(&mut app, "serde");
        assert_eq!(
            backend.opened_shortcuts.lock().unwrap().as_slice(),
            [("sct-docs".to_owned(), vec!["serde".to_owned()])]
        );
    }

    #[test]
    fn one_enter_runs_a_script_with_arguments_once() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = scripts_app(dir.path());
        app.query = "disk report".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("/home".into()),
        ));
        enter_once_in_form(&mut app, "/home");
        assert_eq!(
            backend.script_runs.lock().unwrap().as_slice(),
            [("report.sh".to_owned(), vec!["/home".to_owned()])]
        );
    }

    #[test]
    fn one_enter_runs_an_extension_command_with_its_arguments_once() {
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
        let task = app.update(Message::QueryChanged("greeting".into()));
        settle(&mut app, task);
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("Ada".into()),
        ));
        enter_once_in_form(&mut app, "Ada");
        assert_eq!(
            backend.given.lock().unwrap().len(),
            2,
            "the first try, which asked for the argument, and one run with it"
        );
    }

    #[test]
    fn one_enter_in_the_alias_form_saves_it_and_launches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = Arc::new(LaunchedEntries::default());
        let mut app = app(dir.path()).with_launcher(launcher.clone());
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let firefox = app.selected_row().expect("a row");
        let id = app.root_id(firefox).expect("a root item");
        let _ = app.update(Message::TogglePanel);
        let alias = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Set alias"))
            .expect("the panel offers an alias");
        let task = app.update(Message::PanelClicked(alias));
        settle(&mut app, task);
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("ff".into()),
        ));
        enter_once_in_form(&mut app, "ff");
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
        assert_eq!(
            app.app_index
                .root(&id)
                .and_then(|root| root.meta.alias.as_deref()),
            Some("ff")
        );
        assert!(
            launcher.0.lock().unwrap().is_empty(),
            "saving the alias launched the selected row too"
        );
    }

    #[test]
    fn one_ctrl_enter_saves_a_snippet_once_and_a_plain_enter_none() {
        use crate::preferences_page::FieldValue;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "create snippet", "commands:create-snippet");
        let Page::Preferences(page) = &app.page else {
            panic!("no snippet form: {}", app.state_line());
        };
        let position = |name: &str| page.fields.iter().position(|f| f.name == name).unwrap();
        let (name, content) = (position("name"), position("content"));
        let _ = app.update(Message::PreferenceEdited(
            name,
            FieldValue::Text("Thanks".into()),
        ));
        let _ = app.update(Message::PreferenceTextEdited(
            content,
            iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Paste(
                Arc::new("Thank you!".into()),
            )),
        ));
        let before = backend.snippet_drafts.lock().unwrap().len();
        enter_once_in_form(&mut app, "Thanks");
        assert_eq!(
            backend.snippet_drafts.lock().unwrap().len(),
            before,
            "Enter is a newline in a form with a text area"
        );
        let task = app.update(ctrl_enter());
        settle(&mut app, task);
        assert_eq!(backend.snippet_drafts.lock().unwrap().len(), before + 1);
    }

    fn opener(id: &str, name: &str, default: bool) -> crate::backend::OpenerRow {
        crate::backend::OpenerRow {
            id: id.into(),
            name: name.into(),
            icon: None,
            default,
        }
    }

    #[test]
    fn manage_shortcuts_shows_the_detail_pane_and_opens_with_a_chosen_application() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            shortcuts: std::sync::Mutex::new(vec![
                stored_shortcut("sct-docs", "Crate Docs", "https://docs.rs/{crate}"),
                stored_shortcut("sct-news", "Hacker News", "https://news.ycombinator.com"),
            ]),
            openers: vec![
                opener("firefox.desktop", "Firefox", true),
                opener("chromium.desktop", "Chromium", false),
            ],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        let task = app.refresh_shortcuts_task();
        settle(&mut app, task);
        open_builtin(&mut app, "manage shortcuts", "commands:manage-shortcuts");

        // The pane follows the selection: the link expanded, the default
        // application named as such.
        let Page::Shortcuts(page) = &app.page else {
            panic!("not on Manage Shortcuts: {}", app.state_line());
        };
        let detail = page.detail.clone().expect("a pane for the first row");
        assert_eq!(detail.id, "sct-docs");
        assert_eq!(detail.expanded.as_deref(), Ok("https://docs.rs/{crate}|"));
        assert_eq!(detail.app.as_deref(), Some("Firefox (Default)"));
        let task = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        settle(&mut app, task);
        let Page::Shortcuts(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(
            page.detail.as_ref().map(|d| d.id.as_str()),
            Some("sct-news")
        );

        // Open with… lists the applications for the stored link, opens the
        // expanded one with the chosen application and hides.
        let _ = app.update(Message::TogglePanel);
        let task = choose(&mut app, "Open with...");
        settle(&mut app, task);
        let Page::OpenWith(page) = &app.page else {
            panic!("no app selector: {}", app.state_line());
        };
        assert_eq!(page.all.len(), 2);
        assert_eq!(
            backend
                .opener_lookups
                .lock()
                .unwrap()
                .last()
                .map(String::as_str),
            Some("https://news.ycombinator.com")
        );
        // Escape goes back to the list, and the selector opens again.
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        assert!(
            matches!(app.page, Page::Shortcuts(_)),
            "{}",
            app.state_line()
        );
        let _ = app.update(Message::TogglePanel);
        let task = choose(&mut app, "Open with...");
        settle(&mut app, task);
        let _ = app.update(Message::OpenWithQueryChanged("chrom".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            backend.opened_with.lock().unwrap().as_slice(),
            [(
                "chromium.desktop".to_owned(),
                "https://news.ycombinator.com|".to_owned()
            )]
        );
        assert!(
            !matches!(app.page, Page::OpenWith(_)),
            "hidden after opening"
        );
    }

    #[test]
    fn a_one_argument_shortcut_named_as_a_fallback_opens_with_the_query() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            shortcuts: std::sync::Mutex::new(vec![
                stored_shortcut("sct-docs", "Crate Docs", "https://docs.rs/{crate}"),
                stored_shortcut("sct-news", "Hacker News", "https://news.ycombinator.com"),
            ]),
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.apply(AppFlags {
            fallbacks: vec![
                "shortcuts:sct-news".into(),
                "files:search".into(),
                "shortcuts:sct-docs".into(),
            ],
            ..AppFlags::default()
        });
        app.backend = Some(backend.clone());
        let task = app.refresh_shortcuts_task();
        settle(&mut app, task);
        app.query = "tokio".into();
        app.search();
        let fallbacks: Vec<RootRow> = app
            .results
            .iter()
            .copied()
            .filter(|row| matches!(row, RootRow::Fallback(_)))
            .collect();
        assert_eq!(
            fallbacks.len(),
            2,
            "Search Files and the one-argument shortcut, not the one with none: {}",
            app.state_line()
        );
        assert!(
            matches!(fallbacks[0], RootRow::Fallback(_)),
            "in configured order"
        );
        assert_eq!(fallbacks[1], RootRow::Fallback(Fallback::Shortcut(0)));
        app.selected = app
            .results
            .iter()
            .position(|row| *row == RootRow::Fallback(Fallback::Shortcut(0)))
            .unwrap();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            backend.opened_shortcuts.lock().unwrap().as_slice(),
            [("sct-docs".to_owned(), vec!["tokio".to_owned()])]
        );
    }

    #[test]
    fn a_shortcut_in_root_search_asks_for_its_argument_then_opens() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        app.query = "crate docs".into();
        app.search();
        assert_eq!(
            app.selected_row(),
            Some(RootRow::Shortcut(0)),
            "{}",
            app.state_line()
        );
        assert!(app.state_line().contains("selected_title=\"Crate Docs\""));

        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let Page::Preferences(page) = &app.page else {
            panic!("no arguments form: {}", app.state_line());
        };
        assert_eq!(page.fields[0].title, "crate");

        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        assert!(
            backend.opened_shortcuts.lock().unwrap().is_empty(),
            "a required argument left empty does not open it"
        );

        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("serde".into()),
        ));
        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        assert_eq!(
            backend.opened_shortcuts.lock().unwrap().as_slice(),
            [("sct-docs".to_owned(), vec!["serde".to_owned()])]
        );
        assert_eq!(
            backend.recorded.lock().unwrap().as_slice(),
            ["shortcuts:sct-docs"],
            "the visit counts in root search's ranking"
        );
        assert!(matches!(app.page, Page::Root));

        // One with no arguments opens at once.
        app.query = "hacker news".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            backend.opened_shortcuts.lock().unwrap().last(),
            Some(&("sct-news".to_owned(), Vec::new()))
        );
    }

    #[test]
    fn create_shortcut_saves_the_form_and_the_new_one_is_searchable() {
        use crate::preferences_page::FieldValue;
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        open_builtin(&mut app, "create shortcut", "commands:create-shortcut");
        let Page::Preferences(page) = &app.page else {
            panic!("no form: {}", app.state_line());
        };
        assert_eq!(page.title, "Create Shortcut");
        let position = |name: &str| page.fields.iter().position(|f| f.name == name).unwrap();
        let (name, link, icon) = (position("name"), position("link"), position("icon"));

        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        let Page::Preferences(page) = &app.page else {
            panic!("the form went away without a link");
        };
        assert!(
            page.notice.as_deref().is_some_and(|n| n.contains("Link")),
            "{:?}",
            page.notice
        );

        let _ = app.update(Message::PreferenceEdited(
            name,
            FieldValue::Text("Wiki".into()),
        ));
        let _ = app.update(Message::PreferenceEdited(
            link,
            FieldValue::Text("https://en.wikipedia.org/wiki/{page}".into()),
        ));
        let _ = app.update(Message::PreferenceEdited(
            icon,
            FieldValue::Choice(Some("icon://omnicast/book".into())),
        ));
        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        assert_eq!(
            backend.drafts.lock().unwrap().as_slice(),
            [crate::backend::ShortcutDraft {
                id: None,
                name: "Wiki".into(),
                icon: "icon://omnicast/book".into(),
                url: "https://en.wikipedia.org/wiki/{page}".into(),
                app: "default".into(),
            }]
        );
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
        app.query = "wiki".into();
        app.search();
        assert_eq!(app.selected_row(), Some(RootRow::Shortcut(2)));
    }

    #[test]
    fn manage_shortcuts_filters_edits_and_removes() {
        use iced::keyboard::Modifiers;
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        open_builtin(&mut app, "manage shortcuts", "commands:manage-shortcuts");
        assert!(
            app.state_line().contains("page=shortcuts"),
            "{}",
            app.state_line()
        );

        let _ = app.update(Message::ShortcutsQueryChanged("hackr".into()));
        let Page::Shortcuts(page) = &app.page else {
            panic!("not on Manage Shortcuts");
        };
        assert_eq!(page.shown, [1], "the filter is fuzzy");

        // Ctrl+E edits it in the form, prefilled; Escape comes back here.
        let task = app.update(chord("e", Modifiers::CTRL));
        settle(&mut app, task);
        let Page::Preferences(page) = &app.page else {
            panic!("no edit form: {}", app.state_line());
        };
        assert_eq!(page.title, "Edit \"Hacker News\"");
        assert_eq!(page.command_id, "sct-news");
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        assert!(
            matches!(app.page, Page::Shortcuts(_)),
            "{}",
            app.state_line()
        );

        // Saving an edit sends the id, and returns to the list.
        let task = app.update(chord("e", Modifiers::CTRL));
        settle(&mut app, task);
        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        assert_eq!(
            backend
                .drafts
                .lock()
                .unwrap()
                .last()
                .and_then(|d| d.id.clone()),
            Some("sct-news".to_owned())
        );
        assert!(
            matches!(app.page, Page::Shortcuts(_)),
            "{}",
            app.state_line()
        );

        // Ctrl+X removes the selected one, here; Ctrl+Shift+X does not.
        let _ = app.update(Message::ShortcutsQueryChanged(String::new()));
        let task = app.update(chord("x", Modifiers::CTRL | Modifiers::SHIFT));
        settle(&mut app, task);
        assert_eq!(app.app_index.shortcuts().len(), 2);
        let task = app.update(chord("x", Modifiers::CTRL));
        settle(&mut app, task);
        assert_eq!(
            app.app_index
                .shortcuts()
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["sct-news"]
        );

        // The panel offers the rest; Copy writes the expanded link.
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("copy".into()));
        let task = app.update(Message::PanelActivate);
        let writes = settle(&mut app, task);
        assert_eq!(writes, ["https://news.ycombinator.com|"]);
    }

    #[test]
    fn a_root_shortcut_is_removed_with_ctrl_shift_x() {
        use iced::keyboard::Modifiers;
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _backend) = shortcuts_app(dir.path());
        app.query = "hacker news".into();
        app.search();
        let task = app.update(chord("x", Modifiers::CTRL));
        settle(&mut app, task);
        assert_eq!(
            app.app_index.shortcuts().len(),
            2,
            "Ctrl+X is not remove here"
        );
        let task = app.update(chord("x", Modifiers::CTRL | Modifiers::SHIFT));
        settle(&mut app, task);
        assert_eq!(app.app_index.shortcuts().len(), 1);
        assert!(
            app.results
                .iter()
                .all(|row| !matches!(row, RootRow::Shortcut(_))),
            "the removed shortcut left the results"
        );
    }

    #[test]
    fn shortcuts_without_an_engine_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        open_builtin(&mut app, "create shortcut", "commands:create-shortcut");
        let _ = app.update(Message::PreferenceEdited(
            1,
            crate::preferences_page::FieldValue::Text("https://x.test".into()),
        ));
        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        let Page::Preferences(page) = &app.page else {
            panic!("the form went away");
        };
        assert!(
            page.notice
                .as_deref()
                .is_some_and(|n| n.contains("need the Compass engine")),
            "{:?}",
            page.notice
        );
    }

    // ---- Create Extension ----

    #[test]
    fn create_extension_sends_the_form_and_shows_where_it_went() {
        use crate::preferences_page::FieldValue;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "create extension", "commands:create-extension");
        for (position, value) in [
            "zoe",
            "Hello",
            "Says hello to the whole world",
            "/home/me/code",
            "Say Hello",
            "Says hello",
        ]
        .into_iter()
        .enumerate()
        {
            let _ = app.update(Message::PreferenceEdited(
                position,
                FieldValue::Text(value.into()),
            ));
        }
        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        let created = backend.created.lock().unwrap().clone();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].template, ":boilerplate/tmpl-list");
        let Page::Created(page) = &app.page else {
            panic!("no success page: {}", app.state_line());
        };
        assert_eq!(page.path, "/home/me/code/hello");
    }

    // ---- Browse Fonts ----

    #[test]
    fn browse_fonts_filters_previews_and_goes_back_to_the_same_list() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend);
        open_builtin(&mut app, "browse fonts", "commands:browse-fonts");
        let Page::Fonts(page) = &app.page else {
            panic!("not Browse Fonts: {}", app.state_line());
        };
        assert_eq!(page.shown().0, "All Fonts (2)");
        assert_eq!(page.options, ["All", "Latin", "Monospace"]);

        let task = app.update(Message::FontsCategoryChanged("Monospace".into()));
        settle(&mut app, task);
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::FontPreview(preview) = &app.page else {
            panic!("no specimen: {}", app.state_line());
        };
        assert_eq!(preview.name, "JetBrains Mono");
        assert_eq!(
            preview.lines.first(),
            Some(&crate::fonts_page::SpecimenLine::Heading(
                "JetBrains Mono".into()
            ))
        );

        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        let Page::Fonts(page) = &app.page else {
            panic!("not back at the list: {}", app.state_line());
        };
        assert_eq!(page.category.as_deref(), Some("Monospace"), "filter kept");

        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let _ = app.update(Message::PanelMove(Direction::Down));
        let task = app.update(Message::PanelActivate);
        assert_eq!(
            settle(&mut app, task),
            ["JetBrains Mono"],
            "Copy font family"
        );
    }

    #[test]
    fn browse_fonts_is_a_grid_that_remembers_its_category_and_sets_the_font() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.view.view_memory =
            crate::view_memory::ViewMemory::load(Some(dir.path().join("view-state.json")));
        open_builtin(&mut app, "browse fonts", "commands:browse-fonts");
        let task = app.update(pressed(iced::keyboard::key::Named::ArrowRight));
        settle(&mut app, task);
        let Page::Fonts(page) = &app.page else {
            panic!("not Browse Fonts: {}", app.state_line());
        };
        assert_eq!(page.selected, 1, "Right moves along the row");
        let _ = app.view();

        let task = app.update(Message::FontsCategoryChanged("Monospace".into()));
        settle(&mut app, task);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let _ = app.update(Message::PanelMove(Direction::Down));
        let _ = app.update(Message::PanelMove(Direction::Down));
        let task = app.update(Message::PanelActivate);
        settle(&mut app, task);
        assert_eq!(
            backend.fonts_set.lock().unwrap().as_slice(),
            ["JetBrains Mono"]
        );
        assert_eq!(app.view.font_family.as_deref(), Some("JetBrains Mono"));

        let task = app.update(Message::Dismiss);
        settle(&mut app, task);
        let mut reopened = LauncherApp::with_index(index(dir.path()));
        reopened.backend = Some(backend);
        reopened.view.view_memory =
            crate::view_memory::ViewMemory::load(Some(dir.path().join("view-state.json")));
        open_builtin(&mut reopened, "browse fonts", "commands:browse-fonts");
        let Page::Fonts(page) = &reopened.page else {
            panic!("not Browse Fonts: {}", reopened.state_line());
        };
        assert_eq!(
            page.category.as_deref(),
            Some("Monospace"),
            "the category is remembered across processes"
        );
    }

    /// Browse Apps, once enabled (`isDefaultDisabled`): every application,
    /// the hidden ones with `showHidden`, the name, comment and keyword
    /// filter, Enter to open, `control+shift+1` for the first desktop action
    /// and the copy actions from the panel.
    #[test]
    fn browse_apps_lists_filters_opens_and_copies() {
        let dir = tempfile::tempdir().unwrap();
        drop(index(dir.path()));
        fs::write(
            dir.path().join("editor.desktop"),
            "[Desktop Entry]\nType=Application\nName=Editor\nComment=Write prose\n\
             Exec=/bin/true\nActions=new;\n[Desktop Action new]\nName=New Window\nExec=/bin/true -n\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("probe.desktop"),
            "[Desktop Entry]\nType=Application\nName=Probe\nExec=/bin/true\nNoDisplay=true\n",
        )
        .unwrap();
        let launcher = Arc::new(RecordingLaunchTarget::default());
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build())
            .with_launcher(launcher.clone());
        app.query = "browse apps".into();
        app.search();
        assert!(
            !matches!(app.selected_row(), Some(RootRow::Command(c)) if c.entrypoint == "browse-apps"),
            "disabled until the configuration enables it"
        );
        let config = compass_core::Config::parse(
            r#"{"providers":{"commands":{"entrypoints":{"browse-apps":{"enabled":true}}}}}"#,
            std::path::Path::new("config.json"),
        )
        .unwrap();
        app.app_index.apply_root_config(&config.root_config());
        app.browse_apps.show_hidden = true;
        open_builtin(&mut app, "browse apps", "commands:browse-apps");
        let Page::Apps(page) = &app.page else {
            panic!("not Browse Apps: {}", app.state_line());
        };
        assert_eq!(page.heading(), "Applications (5)");
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Probe").is_ok());
            assert!(ui.find("Hidden").is_ok(), "the NoDisplay entry says so");
        }

        let _ = app.update(Message::AppsQueryChanged("prose".into()));
        let task = app.update(chord(
            "!",
            iced::keyboard::Modifiers::CTRL | iced::keyboard::Modifiers::SHIFT,
        ));
        settle(&mut app, task);
        assert_eq!(*launcher.0.lock().unwrap(), [Some("new".to_owned())]);

        open_builtin(&mut app, "browse apps", "commands:browse-apps");
        let _ = app.update(Message::AppsQueryChanged("prose".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            launcher.0.lock().unwrap().last(),
            Some(&None),
            "Open application"
        );

        open_builtin(&mut app, "browse apps", "commands:browse-apps");
        let _ = app.update(Message::AppsQueryChanged("editor".into()));
        let _ = app.open_apps_panel().expect("a panel");
        let titles: Vec<String> = app.panel.as_ref().unwrap().sections[0]
            .actions
            .iter()
            .map(|action| action.title.clone())
            .collect();
        assert_eq!(
            titles,
            [
                "Open application",
                "New Window",
                "Copy app ID",
                "Copy app location"
            ],
            "no Open Location without an engine to open it"
        );
        let writes = {
            let task = app.apps_panel_action("apps.action.2").expect("copy id");
            settle(&mut app, task)
        };
        assert_eq!(writes, ["editor.desktop"]);
    }

    /// Set Default Browser and Set Default Terminal: the engine's list with
    /// the default marked, Enter to choose, back to the root on success and
    /// the picker's sentence on failure.
    #[test]
    fn a_default_picker_lists_the_engines_candidates_and_sets_the_chosen_one() {
        use crate::backend::{DefaultApp, DefaultAppRow};
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            default_apps: vec![
                DefaultAppRow {
                    id: "firefox.desktop".into(),
                    name: "Firefox".into(),
                    description: "Browse the web".into(),
                    is_default: true,
                },
                DefaultAppRow {
                    id: "broken.desktop".into(),
                    name: "Broken".into(),
                    ..DefaultAppRow::default()
                },
                DefaultAppRow {
                    id: "files.desktop".into(),
                    name: "Files".into(),
                    ..DefaultAppRow::default()
                },
            ],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(
            &mut app,
            "set default browser",
            "commands:set-default-browser",
        );
        let Page::Apps(page) = &app.page else {
            panic!("not the picker: {}", app.state_line());
        };
        assert_eq!(page.placeholder(), "Select a web browser...");
        assert_eq!(page.shown.len(), 3);
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Available web browsers").is_ok());
            assert!(ui.find("✓ Default").is_ok());
        }

        let _ = app.update(Message::AppsQueryChanged("broken".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::Apps(page) = &app.page else {
            panic!("a failure stays on the picker");
        };
        assert_eq!(
            page.notice.as_deref(),
            Some(compass_core::default_app::BROWSER_FAILURE)
        );

        let _ = app.update(Message::AppsQueryChanged("files".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert!(matches!(app.page, Page::Root), "popToRoot");
        assert_eq!(
            *backend.defaults_set.lock().unwrap(),
            [(DefaultApp::Browser, "files.desktop".to_owned())]
        );

        open_builtin(
            &mut app,
            "set default terminal",
            "commands:set-default-terminal",
        );
        let Page::Apps(page) = &app.page else {
            panic!("not the terminal picker");
        };
        assert_eq!(page.heading(), "Available terminal emulators");
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            backend.defaults_set.lock().unwrap().last(),
            Some(&(DefaultApp::Terminal, "firefox.desktop".to_owned()))
        );
    }

    #[test]
    fn script_permissions_lists_what_was_allowed_and_revokes_it() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        *backend.grants.lock().unwrap() = vec![
            crate::backend::ScriptGrant {
                id: "script.clip".into(),
                title: "Clip Tool".into(),
                capabilities: vec!["clipboard.write".into()],
                descriptions: vec!["copy to the clipboard".into()],
            },
            crate::backend::ScriptGrant {
                id: "script.notes".into(),
                title: "Quick Notes".into(),
                capabilities: vec!["storage.read".into()],
                descriptions: vec!["read its saved data".into()],
            },
        ];
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(
            &mut app,
            "script permissions",
            "commands:script-permissions",
        );
        let Page::Grants(page) = &app.page else {
            panic!("not Script Permissions: {}", app.state_line());
        };
        assert_eq!(page.shown.len(), 2);
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Copy to the clipboard").is_ok());
        }
        let _ = app.update(Message::GrantsQueryChanged("notes".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::Grants(page) = &app.page else {
            panic!("left Script Permissions");
        };
        assert_eq!(
            page.all.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            ["script.clip"]
        );
        assert_eq!(page.notice.as_deref(), Some(crate::grants_page::REVOKED));
    }

    #[test]
    fn search_tray_lists_activates_and_browses_an_items_menu() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            tray: vec![
                crate::backend::TrayItemRow {
                    key: ":1.7".into(),
                    title: "Fake Chat".into(),
                    subtitle: "3 unread messages".into(),
                    attention: true,
                    has_menu: true,
                    ..crate::backend::TrayItemRow::default()
                },
                crate::backend::TrayItemRow {
                    key: ":1.8".into(),
                    title: "Network".into(),
                    ..crate::backend::TrayItemRow::default()
                },
            ],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "search tray", "commands:search-tray");
        let Page::Tray(page) = &app.page else {
            panic!("not Search Tray: {}", app.state_line());
        };
        assert_eq!(page.shown.len(), 2);
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Fake Chat").is_ok());
            assert!(ui.find("3 unread messages").is_ok());
            assert!(ui.find(crate::tray_page::ATTENTION).is_ok());
        }
        let _ = app.update(Message::TogglePanel);
        let titles: Vec<String> = app
            .panel
            .as_ref()
            .unwrap()
            .sections
            .iter()
            .flat_map(|section| section.actions.iter().map(|a| a.title.clone()))
            .collect();
        assert_eq!(titles, ["Activate", "Browse menu", "Secondary activate"]);
        let _ = app.update(Message::TogglePanel);

        let _ = app.update(Message::TrayQueryChanged("network".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            backend.tray_calls.lock().unwrap().as_slice(),
            ["activate :1.8 false"],
            "Enter activates"
        );

        let _ = app.update(Message::Command(UiCommand::Show));
        open_builtin(&mut app, "search tray", "commands:search-tray");
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let _ = app.update(Message::PanelFilterChanged("Browse menu".into()));
        let task = app.update(Message::PanelActivate);
        settle(&mut app, task);
        let Page::Tray(page) = &app.page else {
            panic!("left Search Tray");
        };
        assert_eq!(page.menu_key(), Some(":1.7"));
        assert_eq!(page.entries.len(), 2);
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Status › Away").is_ok());
        }
        let _ = app.update(Message::TrayQueryChanged("away".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert!(
            backend
                .tray_calls
                .lock()
                .unwrap()
                .contains(&"trigger :1.7 3".to_owned())
        );
        assert!(
            matches!(app.page, Page::Tray(_)),
            "a toggle keeps the menu open"
        );
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        let Page::Tray(page) = &app.page else {
            panic!("Escape in a menu left Search Tray");
        };
        assert_eq!(page.menu_key(), None, "Escape goes back to the items");
    }

    #[test]
    fn a_rhai_script_row_draws_its_manifest_icon_when_installed() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        let _ = app.update(Message::RhaiScriptsLoaded(Ok(vec![
            compass_core::rhai_scripts::RhaiScriptItem {
                id: "script.no-icon".into(),
                title: "No Icon".into(),
                description: None,
                icon: Some("not-a-builtin".into()),
                keywords: Vec::new(),
            },
        ])));
        assert!(
            app.rhai_icons.is_empty(),
            "an unknown name draws the initial"
        );
    }

    // ---- The extension stores ----

    fn store_backend(dir: &std::path::Path) -> Arc<TestBackend> {
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

    /// Every span's font, through quotes and lists, as `rich_text` gets it.
    fn markdown_span_fonts(
        items: &[iced::widget::markdown::Item],
        style: iced::widget::markdown::Style,
        out: &mut Vec<(String, iced::Font)>,
    ) {
        use iced::widget::markdown::{Bullet, Item};
        for item in items {
            match item {
                Item::Heading(_, text) | Item::Paragraph(text) => {
                    for span in text.spans(style).iter() {
                        out.push((
                            span.text.to_string(),
                            span.font.expect("markdown sets every span's font"),
                        ));
                    }
                }
                Item::Quote(inner) => markdown_span_fonts(inner, style, out),
                Item::List { bullets, .. } => {
                    for bullet in bullets {
                        let (Bullet::Point { items } | Bullet::Task { items, .. }) = bullet;
                        markdown_span_fonts(items, style, out);
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn markdown_is_drawn_in_the_launchers_font() {
        // The store detail page's Markdown as `detail_markdown` writes it:
        // heading, description, compatibility quote, bold-label facts and
        // the command list, plus inline code.
        let markdown = "# Brew\n\nSearch and install Homebrew formulae.\n\n\
            > **No compatibility data for this extension**\n\n\
            **Author** nhojb · **Downloads** 12.3K\n\n\
            ## Commands (2)\n\n\
            - **Search** — Search formulae and casks\n\
            - **Installed** — List `brew list`\n";
        let items: Vec<_> = iced::widget::markdown::parse(markdown).collect();
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));

        for family in [None, Some("Cantarell")] {
            app.view.font_family = family.map(str::to_owned);
            let launcher = app.font();
            let settings = app.markdown_settings();
            let mut spans = Vec::new();
            markdown_span_fonts(&items, settings.style, &mut spans);
            assert!(spans.len() > 10, "the walk found the spans: {spans:?}");
            for (text, font) in &spans {
                if text == "brew list" {
                    assert_eq!(
                        font.family,
                        iced::font::Family::Monospace,
                        "inline code stays monospace"
                    );
                    continue;
                }
                assert_eq!(
                    font.family, launcher.family,
                    "{text:?} is drawn in {font:?}, not the launcher's {launcher:?}"
                );
                assert_eq!(
                    font.style,
                    iced::font::Style::Normal,
                    "{text:?} has no emphasis and must not be italic"
                );
            }
            let bold: Vec<_> = spans
                .iter()
                .filter(|(_, font)| font.weight == iced::font::Weight::Bold)
                .map(|(text, _)| text.as_str())
                .collect();
            assert!(
                bold.contains(&"Author") && bold.contains(&"Search"),
                "the labels stay bold: {bold:?}"
            );
        }
    }

    #[test]
    fn a_deeplink_opens_the_detail_page_and_uninstalling_asks_in_a_dialog() {
        let dir = tempfile::tempdir().unwrap();
        let extensions = dir.path().join("extensions");
        fs::create_dir_all(&extensions).unwrap();
        let backend = store_backend(&extensions);
        {
            let mut rows = backend.store_rows.lock().unwrap();
            rows[1].installed = true;
            rows[1].compat = Some(1);
        }
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.window = Some(window::Id::unique());
        let task = app.update(Message::Command(UiCommand::Deeplink(
            "raycast://extensions/zoe/timer".into(),
        )));
        settle(&mut app, task);
        let Page::StoreDetail(detail) = &app.page else {
            panic!(
                "the deeplink did not open a detail page: {}",
                app.state_line()
            );
        };
        assert_eq!(detail.store, crate::backend::Store::Raycast);
        assert_eq!(detail.detail.row.name, "timer");

        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let _ = app.update(Message::PanelActivate);
        let Page::StoreDetail(detail) = &app.page else {
            panic!("left the detail page");
        };
        assert!(detail.confirm, "the panel's uninstall asks first");
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Are you sure?").is_ok());
            assert!(ui.find("Uninstall Timer").is_ok(), "a dialog with buttons");
        }
        let task = app.update(Message::StoreConfirmAnswered(false));
        settle(&mut app, task);
        let Page::StoreDetail(detail) = &app.page else {
            panic!("left the detail page");
        };
        assert!(
            !detail.confirm && detail.detail.row.installed,
            "Cancel keeps it"
        );

        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        let Page::Store(_) = &app.page else {
            panic!("Escape did not go to the list: {}", app.state_line());
        };
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Partial").is_ok(), "the tier beside its dot");
        }

        let bad = app.update(Message::Command(UiCommand::Deeplink(
            "vicinae://extensions/only-one".into(),
        )));
        settle(&mut app, bad);
    }

    #[test]
    fn the_extension_store_installs_into_root_search_and_uninstalls_after_asking() {
        let dir = tempfile::tempdir().unwrap();
        let extensions = dir.path().join("extensions");
        fs::create_dir_all(&extensions).unwrap();
        let backend = store_backend(&extensions);
        let mut app = LauncherApp::with_index(
            AppIndex::builder()
                .dir(dir.path())
                .extension_dirs([extensions.clone()])
                .build(),
        );
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "extension store", "commands:store");
        // The first time, the intro (`StoreIntroViewHost`); Enter goes on,
        // and it is not shown again.
        assert!(
            matches!(app.page, Page::StoreIntro(_)),
            "{}",
            app.state_line()
        );
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Enter: Continue to store    Esc: back").is_ok());
        }
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            app.view.view_memory.get(crate::compass_pages::intro_key(
                crate::backend::Store::Vicinae
            )),
            Some("true")
        );
        let _ = app.update(Message::Back);
        open_builtin(&mut app, "extension store", "commands:store");
        let Page::Store(page) = &app.page else {
            panic!("not the store: {}", app.state_line());
        };
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.heading, "Extensions");
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Clock").is_ok(), "{}", app.state_line());
            assert!(ui.find("↓ 12").is_ok(), "the accessory is drawn");
        }

        let task = app.update(Message::StoreQueryChanged("clo".into()));
        settle(&mut app, task);
        let Page::Store(page) = &app.page else {
            panic!("left the store");
        };
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].title, "Clock");

        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::StoreDetail(detail) = &app.page else {
            panic!("no detail page: {}", app.state_line());
        };
        assert_eq!(detail.title, "Vicinae Store - Clock");
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Vicinae Store - Clock").is_ok());
        }

        assert!(app.app_index.extensions().is_empty());
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::StoreDetail(detail) = &app.page else {
            panic!("left the detail page: {}", app.state_line());
        };
        assert!(detail.detail.row.installed, "Enter installs");
        assert_eq!(detail.notice.as_deref(), Some("Extension installed"));
        assert_eq!(
            app.app_index
                .extensions()
                .iter()
                .map(|command| command.title.as_str())
                .collect::<Vec<_>>(),
            ["Show The Clock"],
            "root search has the new command"
        );

        // Enter on an installed extension asks before uninstalling.
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::StoreDetail(detail) = &app.page else {
            panic!("left the detail page");
        };
        assert!(detail.confirm, "asked first");
        assert!(backend.store_rows.lock().unwrap()[0].installed);
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::StoreDetail(detail) = &app.page else {
            panic!("left the detail page");
        };
        assert!(!detail.detail.row.installed);
        assert_eq!(detail.notice.as_deref(), Some("Extension uninstalled"));
        assert!(
            app.app_index.extensions().is_empty(),
            "gone from root search"
        );

        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let titles: Vec<_> = app
            .panel
            .as_ref()
            .expect("a panel")
            .sections
            .iter()
            .flat_map(|section| section.actions.iter().map(|action| action.title.clone()))
            .collect();
        assert_eq!(titles, ["Install extension", "Open README", "Report issue"]);
        let _ = app.update(Message::PanelMove(Direction::Down));
        let task = app.update(Message::PanelActivate);
        settle(&mut app, task);
        assert_eq!(
            *backend.opened_urls.lock().unwrap(),
            ["https://example.com/README.md"]
        );
    }

    #[test]
    fn escape_from_a_store_detail_returns_to_the_same_list() {
        let dir = tempfile::tempdir().unwrap();
        let backend = store_backend(dir.path());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.view.view_memory.set(
            crate::compass_pages::intro_key(crate::backend::Store::Raycast),
            "true",
        );
        open_builtin(&mut app, "raycast store", "commands:raycast-store");
        // The Raycast store waits for typing to settle; the wait itself
        // needs a runtime, so the test delivers its end.
        let _debounce = app.update(Message::StoreQueryChanged("tim".into()));
        let Page::Store(page) = &app.page else {
            panic!("not the store");
        };
        let generation = page.generation;
        let stale = app.update(Message::StoreSearchDue(generation - 1));
        assert!(settle(&mut app, stale).is_empty());
        let task = app.update(Message::StoreSearchDue(generation));
        settle(&mut app, task);
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert!(matches!(app.page, Page::StoreDetail(_)));
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        let Page::Store(page) = &app.page else {
            panic!("not back at the list: {}", app.state_line());
        };
        assert_eq!(page.query, "tim", "the search is kept");
        assert_eq!(page.rows[0].title, "Timer");
        assert_eq!(
            backend.store_owners.lock().unwrap().as_slice(),
            ["clockworks"],
            "the detail page is asked for by the owner the store files it under, not the author"
        );
        let asked = backend.store_queries.lock().unwrap().clone();
        assert!(
            asked
                .iter()
                .all(|(store, _)| *store == crate::backend::Store::Raycast)
        );
        assert_eq!(asked.last().map(|(_, q)| q.as_str()), Some("tim"));
    }

    // ---- Set Theme ----

    #[test]
    fn set_theme_previews_as_the_selection_moves_and_escape_puts_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.view.theme_choice = crate::theme::Theme::Nord;
        open_builtin(&mut app, "set theme", "commands:set-theme");
        assert!(
            app.state_line().contains("page=themes"),
            "{}",
            app.state_line()
        );

        let task = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        settle(&mut app, task);
        assert_ne!(
            app.view.theme_choice,
            crate::theme::Theme::Nord,
            "previewed"
        );
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Nord, "put back");
        assert!(matches!(app.page, Page::Root));
        assert!(backend.themes_kept.lock().unwrap().is_empty());
    }

    #[test]
    fn set_theme_keeps_the_chosen_theme() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.view.theme_choice = crate::theme::Theme::System;
        open_builtin(&mut app, "set theme", "commands:set-theme");
        let _ = app.update(Message::ThemesQueryChanged("dracula".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(backend.themes_kept.lock().unwrap().as_slice(), ["dracula"]);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Dracula);
        let task = app.update(Message::Dismiss);
        settle(&mut app, task);
        assert_eq!(
            app.view.theme_choice,
            crate::theme::Theme::Dracula,
            "a kept theme stays when the launcher hides"
        );
    }

    // ---- dmenu ----

    fn dmenu_app(dir: &std::path::Path) -> (LauncherApp, Arc<TestBackend>) {
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir));
        app.backend = Some(backend.clone());
        // Already open, so the summon focuses it rather than opening one,
        // which only a running event loop could answer.
        app.window = Some(window::Id::unique());
        let task = app.update(Message::Command(UiCommand::Dmenu(5)));
        settle(&mut app, task);
        (app, backend)
    }

    #[test]
    fn a_pushed_dmenu_list_is_shown_filtered_and_answered_with_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = dmenu_app(dir.path());
        let Page::Dmenu(page) = &app.page else {
            panic!("no dmenu view: {}", app.state_line());
        };
        assert_eq!(page.entries, ["alpha", "beta", "gamma"]);
        assert_eq!(page.heading().as_deref(), Some("Pick (3)"));

        let _ = app.update(Message::DmenuQueryChanged("gama".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            backend.dmenu_answers.lock().unwrap().as_slice(),
            [(5, Some("gamma".to_owned()))]
        );
        assert!(
            matches!(app.page, Page::Root),
            "choosing hides the launcher"
        );
    }

    #[test]
    fn escape_dismisses_a_dmenu_list_once() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = dmenu_app(dir.path());
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        let task = app.update(Message::Dismiss);
        settle(&mut app, task);
        assert_eq!(
            backend.dmenu_answers.lock().unwrap().as_slice(),
            [(5, None)],
            "one dismissal, however the view went away"
        );
    }

    #[test]
    fn a_dmenu_size_resizes_the_window_until_a_list_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) = dmenu_app(dir.path());
        assert_eq!(app.dmenu_card_size(), None);
        assert_eq!(app.resized_to, None);
        if let Page::Dmenu(page) = &mut app.page {
            page.list.width = Some(400);
            page.list.navigation_title = Some("Pick one".into());
        }
        let _ = app.apply_dmenu_size();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "hello").unwrap();
        if let Page::Dmenu(page) = &mut app.page {
            page.entries.push(file.to_string_lossy().into_owned());
            page.query = "notes".into();
            page.refilter();
            assert!(page.preview.is_some(), "quick look reads the file");
        }
        let _ = app.view();
        let size = Some((400, u32::from(GEOMETRY.card_max_height)));
        assert_eq!(app.dmenu_card_size(), size);
        assert_eq!(app.resized_to, size);
        let task = app.update(Message::Command(UiCommand::Dmenu(5)));
        settle(&mut app, task);
        assert_eq!(app.resized_to, None, "the next list puts the size back");
    }

    // ---- Run Terminal Program ----

    #[test]
    fn run_terminal_program_runs_the_typed_command_line_in_a_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "run terminal program", "commands:run-program");
        let Page::Programs(page) = &app.page else {
            panic!("not on Run Terminal Program: {}", app.state_line());
        };
        assert_eq!(page.rows.len(), 2, "every program for the empty query");

        let _ = app.update(Message::ProgramsQueryChanged("htop -d 5".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            backend.programs_ran.lock().unwrap().as_slice(),
            [(
                vec!["htop".to_owned(), "-d".to_owned(), "5".to_owned()],
                true,
                false
            )],
            "run-in-terminal is the default: a terminal that closes"
        );
        assert!(matches!(app.page, Page::Root), "running hides the launcher");
    }

    // ---- Script commands ----

    fn script_item(
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

    fn scripts_app(dir: &std::path::Path) -> (LauncherApp, Arc<TestBackend>) {
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

    #[test]
    fn a_full_output_script_asks_for_its_argument_and_shows_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = scripts_app(dir.path());
        app.query = "disk report".into();
        app.search();
        assert_eq!(
            app.selected_row(),
            Some(RootRow::Script(0)),
            "{}",
            app.state_line()
        );
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let Page::Preferences(page) = &app.page else {
            panic!("no arguments form: {}", app.state_line());
        };
        assert_eq!(page.fields[0].title, "arg0");
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("/home".into()),
        ));
        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        assert_eq!(
            backend.script_runs.lock().unwrap().as_slice(),
            [("report.sh".to_owned(), vec!["/home".to_owned()])]
        );
        assert_eq!(
            backend.recorded.lock().unwrap().as_slice(),
            ["scripts:report.sh"]
        );
        let Page::ScriptOutput(page) = &app.page else {
            panic!("no output view: {}", app.state_line());
        };
        assert_eq!(page.heading(), "Done in 1.2s (exit=0)");
        assert_eq!(page.runs[0].text, "report.sh");
        assert_eq!(
            page.runs[0].foreground,
            Some(compass_core::script_output::Color::Red)
        );
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
    }

    #[test]
    fn full_output_scrolls_from_the_keyboard_copies_and_shows_a_failure() {
        use iced::keyboard::key::Named;
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _backend) = scripts_app(dir.path());
        app.page = Page::ScriptOutput(crate::script_page::ScriptOutputPage::new(
            "report.sh".into(),
            Vec::new(),
            "Disk Report".into(),
            7,
        ));
        let Page::ScriptOutput(page) = &mut app.page else {
            unreachable!()
        };
        page.apply(
            7,
            crate::backend::ScriptOutputState {
                output: "\u{1b}[31mdisk full\u{1b}[0m\nline 2\n".into(),
                finished: true,
                exit_code: Some(3),
                elapsed_ms: 100,
            },
        );
        assert!(page.failed());
        assert_eq!(page.heading(), "Failed after 0.1s (exit code 3)");

        for key in [
            Named::ArrowDown,
            Named::ArrowUp,
            Named::PageDown,
            Named::PageUp,
            Named::Home,
            Named::End,
        ] {
            assert_eq!(
                task_actions(app.update(pressed(key))),
                1,
                "{key:?} scrolls the output"
            );
        }

        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let copy = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Copy output"))
            .unwrap_or_else(|| panic!("no Copy output: {}", app.state_line()));
        let task = app.update(Message::PanelClicked(copy));
        assert_eq!(settle(&mut app, task), ["disk full\nline 2\n"]);
    }

    #[test]
    fn after_a_compact_script_enter_does_not_run_it_again_and_typing_searches() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = scripts_app(dir.path());
        app.query = "count things".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert!(app.error.is_some(), "{}", app.state_line());
        let runs = backend.script_runs.lock().unwrap().len();
        let started = app.update(Message::ScriptStarted {
            id: "count.sh".into(),
            arguments: Vec::new(),
            result: Ok(Some(7)),
        });
        assert!(
            task_actions(started) > 0,
            "the search field gets the keyboard back"
        );

        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            backend.script_runs.lock().unwrap().len(),
            runs,
            "not run again"
        );
        assert!(app.error.is_none(), "the results are back");

        app.error = Some("count.sh".into());
        let task = app.update(Message::QueryChanged("touch".into()));
        settle(&mut app, task);
        assert!(app.error.is_none(), "typing replaces the line with results");
        assert_eq!(app.query, "touch");
    }

    #[test]
    fn a_compact_script_says_its_first_line_and_a_silent_one_hides_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = scripts_app(dir.path());
        app.query = "count things".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            app.error.as_deref(),
            Some("count.sh "),
            "{}",
            app.state_line()
        );

        app.query = "touch marker".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(backend.script_runs.lock().unwrap().len(), 2);
    }

    // ---- Snippets ----

    fn stored_snippet(
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

    /// An app whose engine holds two snippets, with Manage Snippets open.
    fn snippets_app(dir: &std::path::Path) -> (LauncherApp, Arc<TestBackend>) {
        let backend = Arc::new(TestBackend {
            snippets: std::sync::Mutex::new(vec![
                stored_snippet("snp-sig", "Signature", "Best,\nMe", Some(";sig")),
                stored_snippet("snp-hi", "Greeting", "Hello {name}!", None),
            ]),
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "manage snippets", "commands:manage-snippets");
        assert!(
            app.state_line().contains("page=snippets"),
            "{}",
            app.state_line()
        );
        (app, backend)
    }

    fn ctrl_enter() -> Message {
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

    #[test]
    fn manage_snippets_shows_the_selected_snippets_detail_pane() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _backend) = snippets_app(dir.path());
        let task = app.update(Message::SnippetsQueryChanged(String::new()));
        settle(&mut app, task);
        let Page::Snippets(page) = &app.page else {
            panic!("not on Manage Snippets: {}", app.state_line());
        };
        assert_eq!(
            page.detail,
            Some(crate::snippets_page::Detail {
                id: "snp-sig".into(),
                expanded: Ok("preview snp-sig:".into()),
            }),
            "the first row's text, previewed by the engine"
        );
        let task = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        settle(&mut app, task);
        let Page::Snippets(page) = &app.page else {
            panic!("left Manage Snippets: {}", app.state_line());
        };
        assert_eq!(
            page.detail.as_ref().map(|d| d.id.as_str()),
            Some("snp-hi"),
            "the pane follows the selection"
        );
        // A late answer for a row no longer selected is dropped.
        let _ = app.update(Message::SnippetDetailLoaded(crate::snippets_page::Detail {
            id: "snp-sig".into(),
            expanded: Ok("late".into()),
        }));
        let Page::Snippets(page) = &app.page else {
            panic!("left Manage Snippets");
        };
        assert_eq!(page.detail.as_ref().map(|d| d.id.as_str()), Some("snp-hi"));
    }

    #[test]
    fn manage_snippets_copies_asking_for_arguments_first() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = snippets_app(dir.path());
        let Page::Snippets(page) = &app.page else {
            panic!("not on Manage Snippets");
        };
        assert_eq!(page.shown, [0, 1]);

        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        let writes = settle(&mut app, task);
        assert_eq!(
            writes,
            ["snp-sig:"],
            "Enter copies, as the C++'s primary action"
        );
        assert!(
            !matches!(app.page, Page::Snippets(_)),
            "copying hides the launcher: {}",
            app.state_line()
        );
        open_builtin(&mut app, "manage snippets", "commands:manage-snippets");

        let _ = app.update(Message::SnippetsQueryChanged("greting".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::Preferences(page) = &app.page else {
            panic!("no arguments form: {}", app.state_line());
        };
        assert_eq!(page.fields[0].title, "name");
        // Escape goes back to the list, as it was.
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        let Page::Snippets(page) = &app.page else {
            panic!(
                "Escape did not return to Manage Snippets: {}",
                app.state_line()
            );
        };
        assert_eq!(page.query, "greting");

        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("paste".into()));
        let task = app.update(Message::PanelActivate);
        settle(&mut app, task);
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("Zoë".into()),
        ));
        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        assert_eq!(
            backend.snippet_uses.lock().unwrap().last(),
            Some(&(
                "snp-hi".to_owned(),
                vec![("name".to_owned(), "Zoë".to_owned())],
                true
            ))
        );
        assert!(matches!(app.page, Page::Root), "pasting hides the launcher");
    }

    #[test]
    fn create_snippet_takes_several_lines_and_saves_with_ctrl_enter() {
        use crate::preferences_page::FieldValue;
        use iced::widget::text_editor::{Action, Edit};
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "create snippet", "commands:create-snippet");
        let Page::Preferences(page) = &app.page else {
            panic!("no form: {}", app.state_line());
        };
        assert_eq!(page.title, "Create Snippet");
        let content = page
            .fields
            .iter()
            .position(|f| f.name == "content")
            .unwrap();

        let _ = app.update(Message::PreferenceEdited(0, FieldValue::Text("Sig".into())));
        let _ = app.update(Message::PreferenceTextEdited(
            content,
            Action::Edit(Edit::Paste(Arc::new("Best,\n{cursor}".to_owned()))),
        ));
        let _ = app.update(Message::PreferenceEdited(
            2,
            FieldValue::Text(" ;sig ".into()),
        ));

        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert!(
            backend.snippet_drafts.lock().unwrap().is_empty(),
            "Enter is a newline in the text area, not a submit"
        );
        let task = app.update(ctrl_enter());
        settle(&mut app, task);
        assert_eq!(
            backend.snippet_drafts.lock().unwrap().as_slice(),
            [crate::backend::SnippetDraft {
                id: None,
                name: "Sig".into(),
                text: "Best,\n{cursor}".into(),
                keyword: Some(";sig".into()),
                word: true,
                apps: vec![],
            }]
        );
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
    }

    #[test]
    fn editing_a_snippet_keeps_its_apps_and_returns_to_the_list() {
        use iced::keyboard::Modifiers;
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = snippets_app(dir.path());
        let task = app.update(chord("e", Modifiers::CTRL));
        settle(&mut app, task);
        let Page::Preferences(page) = &app.page else {
            panic!("no edit form: {}", app.state_line());
        };
        assert_eq!(page.title, "Edit \"Signature\"");
        let task = app.update(ctrl_enter());
        settle(&mut app, task);
        let draft = backend
            .snippet_drafts
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap();
        assert_eq!(draft.id.as_deref(), Some("snp-sig"));
        assert_eq!(draft.apps, ["org.gnome.TextEditor.desktop"]);
        assert!(
            matches!(app.page, Page::Snippets(_)),
            "{}",
            app.state_line()
        );

        let task = app.update(chord("x", Modifiers::CTRL));
        settle(&mut app, task);
        let Page::Snippets(page) = &app.page else {
            panic!("left Manage Snippets");
        };
        assert_eq!(
            page.all.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["snp-hi"]
        );
    }

    #[test]
    fn snippets_without_an_engine_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        open_builtin(&mut app, "manage snippets", "commands:manage-snippets");
        let Page::Snippets(page) = &app.page else {
            panic!("not on Manage Snippets");
        };
        assert!(
            matches!(&page.status, crate::snippets_page::Status::Failed(reason)
                if reason.contains("need the Compass engine")),
            "{:?}",
            page.status
        );
    }

    // ---- builtin commands and the clipboard history view ----

    #[derive(Debug, Default)]
    struct FakeClipboard {
        rows: Vec<crate::backend::ClipboardRow>,
        content: Option<crate::backend::ClipboardContent>,
        queries: std::sync::Mutex<Vec<String>>,
        /// Whether the engine can paste (it has the Shell extension).
        can_paste: bool,
        pasted: std::sync::Mutex<Vec<String>>,
        changes: std::sync::Mutex<Vec<String>>,
        fail_changes: bool,
        /// Each entry's keywords, as set.
        keywords: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
        /// Whether copies are being recorded; `None` is never asked.
        monitoring: std::sync::Mutex<Option<bool>>,
        /// The kinds history was asked for.
        kinds: std::sync::Mutex<Vec<Option<crate::backend::ClipboardRowKind>>>,
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

    fn clip_row(id: &str, preview: &str) -> crate::backend::ClipboardRow {
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
    fn settle(app: &mut LauncherApp, task: Task<Message>) -> Vec<String> {
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

    fn clipboard_app(
        dir: &std::path::Path,
        clipboard: Option<Arc<FakeClipboard>>,
    ) -> (LauncherApp, Arc<TestBackend>) {
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir));
        app.apply(AppFlags {
            clipboard: clipboard.map(|c| c as Arc<dyn crate::backend::ClipboardBackend>),
            ..AppFlags::default()
        });
        // The ranking backend only for `record_launch`; search stays local so
        // the command row comes from the real index.
        app.backend = Some(backend.clone());
        (app, backend)
    }

    fn open_clipboard(app: &mut LauncherApp) -> Vec<String> {
        app.query = "clipboard".into();
        app.search();
        assert_eq!(
            app.selected_row(),
            Some(RootRow::Command(
                compass_core::commands::by_id("commands:clipboard-history").unwrap()
            )),
            "{}",
            app.state_line()
        );
        let task = app.update(Message::LaunchSelected);
        settle(app, task)
    }

    #[test]
    fn enter_on_the_command_opens_clipboard_history_and_records_the_use() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "newest"), clip_row("2", "older")],
            ..FakeClipboard::default()
        });
        let (mut app, backend) = clipboard_app(dir.path(), Some(clipboard.clone()));
        open_clipboard(&mut app);

        assert!(app.showing_clipboard());
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.status, crate::clipboard_page::Status::Ready);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            backend.recorded.lock().unwrap().as_slice(),
            ["commands:clipboard-history"]
        );
        assert!(
            app.state_line().contains("page=clipboard"),
            "{}",
            app.state_line()
        );
    }

    #[test]
    fn a_copied_link_opens_and_opens_with_a_chosen_application() {
        let dir = tempfile::tempdir().unwrap();
        let mut link = clip_row("1", "https://docs.rs/serde");
        link.kind = crate::backend::ClipboardRowKind::Link;
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![link],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"https://docs.rs/serde".to_vec(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, backend) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let titles = panel_titles(&app);
        assert_eq!(
            titles[..4],
            [
                "Paste to active window",
                "Copy to clipboard",
                "Open",
                "Open with..."
            ]
        );
        let task = choose(&mut app, "Open with...");
        settle(&mut app, task);
        assert!(
            matches!(app.page, Page::OpenWith(_)),
            "{}",
            app.state_line()
        );
        assert_eq!(
            backend.opener_lookups.lock().unwrap().as_slice(),
            ["https://docs.rs/serde"]
        );
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        assert!(
            matches!(app.page, Page::Clipboard(_)),
            "Escape returns to the history: {}",
            app.state_line()
        );
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Open");
        settle(&mut app, task);
        assert_eq!(
            backend.opened_urls.lock().unwrap().as_slice(),
            ["https://docs.rs/serde"]
        );
    }

    #[test]
    fn plain_text_offers_no_open() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "some text")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"some text".to_vec(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert!(!panel_titles(&app).iter().any(|t| t.starts_with("Open")));
    }

    #[test]
    fn the_kind_filter_the_pane_keywords_remove_all_and_monitoring() {
        let dir = tempfile::tempdir().unwrap();
        let mut image = clip_row("2", "Image");
        image.kind = crate::backend::ClipboardRowKind::Image;
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "some text"), image],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"some text".to_vec(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard.clone()));
        app.view.view_memory = crate::view_memory::ViewMemory::load(None);
        open_clipboard(&mut app);

        // The pane shows the selected entry, its text and its metadata.
        {
            let Page::Clipboard(page) = &app.page else {
                unreachable!()
            };
            let detail = page.detail.as_ref().expect("the pane is loaded");
            assert_eq!(detail.id, "1");
            assert_eq!(
                detail.pane,
                Some(crate::clipboard_page::DetailContent::Text(
                    "some text".into()
                ))
            );
            assert!(matches!(&detail.info, Some(Ok(info)) if info.size == 1536));
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("1.50 KB").is_ok(), "the size is in the pane");
        }

        // The filter asks the engine for one kind, and is remembered.
        let task = app.update(Message::ClipboardKindChanged("Images".into()));
        settle(&mut app, task);
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.rows.len(), 1);
        assert_eq!(
            clipboard.kinds.lock().unwrap().last(),
            Some(&Some(crate::backend::ClipboardRowKind::Image))
        );
        assert_eq!(
            app.view
                .view_memory
                .get(crate::clipboard_page::FILTER_MEMORY_KEY),
            Some("image")
        );
        let task = app.update(Message::ClipboardKindChanged("All".into()));
        settle(&mut app, task);

        // Ctrl+E opens the keyword form with what is stored; saving it goes
        // back to the history.
        let task = app.update(chord("e", iced::keyboard::Modifiers::CTRL));
        settle(&mut app, task);
        assert!(
            matches!(&app.page, Page::Preferences(form)
                if form.purpose == crate::preferences_page::Purpose::ClipboardKeywords
                    && form.command_id == "1"),
            "{}",
            app.state_line()
        );
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("invoice".into()),
        ));
        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        assert!(app.showing_clipboard());
        assert!(
            clipboard
                .changes
                .lock()
                .unwrap()
                .contains(&"keywords 1 invoice".to_owned())
        );

        // Remove-all asks first; Escape leaves everything, Enter removes.
        let _ = app.update(Message::TogglePanel);
        let remove_all = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Remove all"))
            .expect("the panel offers remove-all");
        let _ = app.update(Message::PanelClicked(remove_all));
        assert!(app.confirm.is_some());
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(app.confirm.is_none());
        assert!(
            !clipboard
                .changes
                .lock()
                .unwrap()
                .contains(&"remove-all".to_owned())
        );
        let _ = app.update(chord(
            "x",
            iced::keyboard::Modifiers::CTRL | iced::keyboard::Modifiers::SHIFT,
        ));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert!(
            clipboard
                .changes
                .lock()
                .unwrap()
                .contains(&"remove-all".to_owned())
        );

        // The panel pauses recording, and then offers to resume it.
        let _ = app.update(Message::TogglePanel);
        let pause = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Pause clipboard"))
            .expect("recording can be paused");
        let task = app.update(Message::PanelClicked(pause));
        settle(&mut app, task);
        assert_eq!(*clipboard.monitoring.lock().unwrap(), Some(false));
        let _ = app.update(Message::TogglePanel);
        assert!(
            app.panel
                .as_ref()
                .and_then(|panel| panel.row_titled("Resume clipboard"))
                .is_some()
        );
    }

    #[test]
    fn typing_filters_arrows_move_and_escape_goes_back_not_away() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![
                clip_row("1", "alpha one"),
                clip_row("2", "alpha two"),
                clip_row("3", "beta"),
            ],
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard.clone()));
        open_clipboard(&mut app);

        let task = app.update(Message::ClipboardQueryChanged("alpha".into()));
        settle(&mut app, task);
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            clipboard.queries.lock().unwrap().last().map(String::as_str),
            Some("alpha")
        );

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.selected, 1);

        let back = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(!app.showing_clipboard(), "Escape leaves the view");
        assert_eq!(app.query, "clipboard", "and the root query is where it was");
        // Leaving the view by hiding the window would also reset it, so the
        // state alone cannot tell "back" from "away": the task can.
        let actions: Vec<_> = iced_winit::runtime::task::into_stream(back)
            .map(|stream| {
                iced::futures::executor::block_on(iced::futures::StreamExt::collect::<Vec<_>>(
                    stream,
                ))
            })
            .unwrap_or_default();
        assert!(
            !actions.iter().any(|action| matches!(
                action,
                iced_winit::runtime::Action::Exit | iced_winit::runtime::Action::Window(_)
            )),
            "Escape in a command view must not hide or quit the launcher"
        );
    }

    #[test]
    fn enter_pastes_through_the_engine_and_hides_without_copying() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "newest"), clip_row("2", "older")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"older".to_vec(),
            }),
            can_paste: true,
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard.clone()));
        open_clipboard(&mut app);

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        let writes = settle(&mut app, task);
        assert_eq!(clipboard.pasted.lock().unwrap().as_slice(), ["2"]);
        assert!(
            writes.is_empty(),
            "the engine put it on the clipboard; the launcher must not race it"
        );
        assert!(
            !app.showing_clipboard(),
            "and the launcher got out of the way"
        );
    }

    #[test]
    fn ctrl_shift_p_toggles_the_pin_and_ctrl_x_removes_then_the_list_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let pinned = crate::backend::ClipboardRow {
            pinned: true,
            ..clip_row("2", "pinned one")
        };
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "loose one"), pinned],
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard.clone()));
        open_clipboard(&mut app);
        let loads = clipboard.queries.lock().unwrap().len();

        let ctrl = iced::keyboard::Modifiers::CTRL;
        let ctrl_shift = iced::keyboard::Modifiers::CTRL | iced::keyboard::Modifiers::SHIFT;
        let task = app.update(chord("P", ctrl_shift));
        settle(&mut app, task);
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        let task = app.update(chord("P", ctrl_shift));
        settle(&mut app, task);
        let task = app.update(chord("x", ctrl));
        settle(&mut app, task);

        assert_eq!(
            clipboard.changes.lock().unwrap().as_slice(),
            ["pin 1 true", "pin 2 false", "remove 2"],
            "the pin flips each entry's own state, and the selection stays on its entry \
             through each reload, so Ctrl+X removes the entry the person was on"
        );
        assert_eq!(
            clipboard.queries.lock().unwrap().len(),
            loads + 3,
            "every change reloads the list"
        );
        assert!(app.showing_clipboard(), "and the view stays open");
    }

    #[test]
    fn a_refused_change_says_why_and_keeps_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "only one")],
            fail_changes: true,
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);

        let task = app.update(chord("x", iced::keyboard::Modifiers::CTRL));
        settle(&mut app, task);
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.notice.as_deref(), Some("That entry is already gone"));
        assert_eq!(page.rows.len(), 1);
    }

    #[test]
    fn enter_copies_the_whole_entry_and_hides_when_the_engine_cannot_paste() {
        let dir = tempfile::tempdir().unwrap();
        let whole = "the whole entry, not its preview ".repeat(10);
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "the whole entry…")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: whole.clone().into_bytes(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);

        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        let writes = settle(&mut app, task);
        assert_eq!(writes, [whole]);
        assert!(
            !app.showing_clipboard(),
            "hiding resets the view for the next summon"
        );
    }

    #[test]
    fn an_image_is_not_copied_as_text_and_the_view_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "Image")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "image/png".into(),
                data: vec![0x89, b'P', b'N', b'G'],
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);

        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(settle(&mut app, task).is_empty(), "nothing written");
        let Page::Clipboard(page) = &app.page else {
            panic!("the view stays open to show the reason")
        };
        assert!(page.notice.as_deref().is_some_and(|n| n.contains("images")));
    }

    #[test]
    fn without_an_engine_the_view_says_so_instead_of_showing_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) = clipboard_app(dir.path(), None);
        open_clipboard(&mut app);
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert!(
            matches!(&page.status, crate::clipboard_page::Status::Failed(reason) if reason.contains("engine")),
            "{:?}",
            page.status
        );
    }

    #[test]
    fn clicking_a_clipboard_row_copies_it() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "first"), clip_row("2", "second entry")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"second entry".to_vec(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);

        let mut ui = iced_test::simulator(app.view());
        ui.click("second entry").expect("the row is drawn");
        let messages = ui.into_messages().collect::<Vec<_>>();
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::ClipboardSelected(1))),
            "{messages:?}"
        );
        let mut writes = Vec::new();
        for message in messages {
            let task = app.update(message);
            writes.extend(settle(&mut app, task));
        }
        assert_eq!(writes, ["second entry"]);
    }

    #[derive(Debug, Default)]
    struct FakeWindows {
        rows: Vec<crate::backend::WindowRow>,
        fail_activate: bool,
        activated: std::sync::Mutex<Vec<u32>>,
        closed: std::sync::Mutex<Vec<u32>>,
        listed: std::sync::atomic::AtomicUsize,
        /// The applications running, by desktop id.
        running: Vec<(String, crate::backend::AppRuntimeInfo)>,
        quits: std::sync::Mutex<Vec<(String, bool)>>,
        window_quits: std::sync::Mutex<Vec<(u32, bool)>>,
        fail_quit: bool,
        /// What the window manager can do.
        caps: compass_core::window_switcher::Capabilities,
        workspaces: Vec<crate::backend::WorkspaceRow>,
        focused_workspaces: std::sync::Mutex<Vec<String>>,
        toggles: std::sync::Mutex<Vec<crate::backend::WindowToggle>>,
        /// What a toggle answers when it fails.
        toggle_refusal: Option<&'static str>,
    }

    impl crate::backend::WindowBackend for FakeWindows {
        fn window_manager_capabilities(
            &self,
        ) -> crate::backend::BackendFuture<'_, compass_core::window_switcher::Capabilities>
        {
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
        fn list_windows(
            &self,
        ) -> crate::backend::BackendFuture<'_, Vec<crate::backend::WindowRow>> {
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
        fn quit_window_app(
            &self,
            window: u32,
            force: bool,
        ) -> crate::backend::BackendFuture<'_, ()> {
            Box::pin(async move {
                self.window_quits.lock().unwrap().push((window, force));
                Ok(())
            })
        }
    }

    fn panel_titles(app: &LauncherApp) -> Vec<String> {
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

    fn choose(app: &mut LauncherApp, title: &str) -> Task<Message> {
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

    #[test]
    fn a_running_applications_panel_offers_quit_and_force_quit_and_they_reach_the_engine() {
        let dir = tempfile::tempdir().unwrap();
        let running = crate::backend::AppRuntimeInfo {
            running: true,
            frontmost: false,
            windows: vec![window_row(31, "Mozilla Firefox", "Firefox", 5)],
        };
        let windows = Arc::new(FakeWindows {
            running: vec![("firefox.desktop".into(), running)],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows.clone()));

        // Not running: the panel is the plain one, and stays so.
        app.query = "Terminal".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert!(!panel_titles(&app).iter().any(|t| t.contains("Quit")));
        let _ = app.update(Message::TogglePanel);

        app.query = "Firefox".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert_eq!(
            panel_titles(&app),
            [
                "Open",
                "Focus window",
                "Close window",
                "Copy name",
                "Copy path",
                "Quit application",
                "Force quit application",
                "Copy deeplink",
                "Reset ranking",
                "Add to favorites",
                "Set alias",
                "Set global shortcut",
                "Open preferences",
                "Copy ID",
                "Disable item"
            ]
        );
        let quit_row = app
            .panel
            .as_ref()
            .unwrap()
            .sections
            .iter()
            .find(|section| {
                section
                    .actions
                    .iter()
                    .any(|a| a.id.as_deref() == Some(super::runtime::APP_QUIT))
            })
            .unwrap();
        assert_eq!(quit_row.actions[0].shortcut.as_deref(), Some("ctrl+q"));

        let task = choose(&mut app, "Force quit application");
        settle(&mut app, task);
        assert_eq!(
            windows.quits.lock().unwrap().as_slice(),
            [("firefox.desktop".to_owned(), true)]
        );

        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Quit application");
        settle(&mut app, task);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Focus window");
        settle(&mut app, task);
        assert_eq!(
            windows.quits.lock().unwrap().last(),
            Some(&("firefox.desktop".to_owned(), false))
        );
        assert_eq!(windows.activated.lock().unwrap().as_slice(), [31]);
    }

    #[test]
    fn a_quit_that_does_nothing_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let running = crate::backend::AppRuntimeInfo {
            running: true,
            frontmost: true,
            windows: vec![window_row(31, "Files", "Files", 5)],
        };
        let windows = Arc::new(FakeWindows {
            running: vec![("files.desktop".into(), running)],
            fail_quit: true,
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows));
        app.query = "Files".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Quit application");
        settle(&mut app, task);
        assert_eq!(app.error.as_deref(), Some("Failed to quit files.desktop"));
    }

    /// A resident launcher (dismissal hides) whose presentation has a HUD.
    fn with_resident_hud(mut app: LauncherApp) -> LauncherApp {
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, outcomes) = tokio::sync::mpsc::unbounded_channel();
        // Kept alive for the test's length: a dropped end is a disconnect.
        std::mem::forget((commands, outcomes));
        app.link = Some(EngineLink::new(receiver, sender));
        app.with_hud(true)
    }

    #[test]
    fn refresh_exchange_rates_installs_the_fresh_rates_and_says_so_or_why_not() {
        let dir = tempfile::tempdir().unwrap();
        let rates = compass_core::exchange_rates::ExchangeRates {
            date: "2026-09-24".into(),
            fetched_at: 7,
            rates: [("EUR".to_owned(), 1.0), ("USD".to_owned(), 1.25)].into(),
        };
        let backend = Arc::new(TestBackend {
            rates: Some(rates.clone()),
            ..TestBackend::default()
        });
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(backend);
        open_builtin(&mut app, "refresh exchange rates", "commands:refresh-rates");
        let installed = compass_core::calculator::exchange_rates();
        let answer = compass_core::calculator::compute("10 usd to eur").map(|a| a.answer);
        compass_core::calculator::set_exchange_rates(None);
        assert_eq!(installed.as_deref(), Some(&rates));
        assert_eq!(answer.as_deref(), Some("8 EUR"));
        assert_eq!(
            app.hud_content(),
            Some(&crate::app::calculator::rates_refreshed())
        );

        let failing = Arc::new(TestBackend {
            refuse_rates: Some("Could not refresh the exchange rates: offline".into()),
            ..TestBackend::default()
        });
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(failing);
        open_builtin(&mut app, "refresh exchange rates", "commands:refresh-rates");
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Could not refresh the exchange rates: offline")
        );
        assert_eq!(
            app.error.as_deref(),
            Some("Could not refresh the exchange rates: offline")
        );
    }

    #[test]
    fn quit_and_force_quit_hide_with_the_cpps_hud() {
        let dir = tempfile::tempdir().unwrap();
        let running = crate::backend::AppRuntimeInfo {
            running: true,
            frontmost: true,
            windows: vec![window_row(31, "Files", "Files", 5)],
        };
        let windows = Arc::new(FakeWindows {
            running: vec![("files.desktop".into(), running)],
            ..FakeWindows::default()
        });
        let (app, _) = windows_app(dir.path(), Some(windows));
        let mut app = with_resident_hud(app);
        app.query = "Files".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Quit application");
        settle(&mut app, task);
        assert_eq!(app.hud_content(), Some(&crate::hud::Hud::new("Quit Files")));
        assert!(matches!(app.page, Page::Root), "the launcher hid");

        app.query = "Files".into();
        app.search();
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Force quit application");
        settle(&mut app, task);
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Force quit Files")
        );
    }

    #[test]
    fn a_copy_goes_through_the_engine_and_through_the_window_only_where_it_cannot() {
        let dir = tempfile::tempdir().unwrap();
        for copies in [true, false] {
            let backend = Arc::new(TestBackend {
                copies,
                ..TestBackend::default()
            });
            let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
            app.query = "5 ft to m".into();
            app.search();
            assert_eq!(app.selected_row(), Some(RootRow::Calculator));
            app.backend = Some(backend.clone());
            let task = app.update(Message::LaunchSelected);
            let window_writes = settle(&mut app, task);
            if copies {
                assert_eq!(
                    backend.copied.lock().unwrap().as_slice(),
                    ["1.524"],
                    "the engine's clipboard outlives the window; the value, not `1.524 m`"
                );
                assert!(window_writes.is_empty(), "{window_writes:?}");
            } else {
                assert_eq!(window_writes, ["1.524"], "the window's, as a fallback");
            }
        }
    }

    #[test]
    fn a_copied_answer_shows_the_calculators_hud_until_its_time_is_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.query = "6*7".into();
        app.search();
        assert_eq!(app.selected_row(), Some(RootRow::Calculator));
        let shown_at = std::time::Instant::now();
        let task = app.update(Message::LaunchSelected);
        assert_eq!(settle(&mut app, task), ["42"]);
        let hud = app.hud_content().expect("a HUD").clone();
        assert_eq!(hud.text, "Answer copied to clipboard");
        assert_eq!(hud.icon.as_deref(), Some(crate::hud::COPY_ICON));

        let _ = app.update(Message::HudTick(shown_at));
        assert!(app.hud_content().is_some(), "not yet");
        let _ = app.update(Message::HudTick(
            shown_at + crate::hud::DURATION + std::time::Duration::from_millis(100),
        ));
        assert!(app.hud_content().is_none(), "gone after 1.5 s");
    }

    /// Ctrl+B over the answer offers upstream's panel, and each action does
    /// what it says: the copies copy and remember, the search bar takes the
    /// answer, and the history opens.
    #[test]
    fn the_calculator_answers_panel_copies_types_and_opens_the_history() {
        let dir = tempfile::tempdir().unwrap();
        let answer = |app: &mut LauncherApp| {
            app.page = Page::Root;
            app.query = "6*7".into();
            app.search();
            assert_eq!(app.selected_row(), Some(RootRow::Calculator));
            let _ = app.update(Message::TogglePanel);
        };
        let backend = Arc::new(TestBackend::default());
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(backend.clone());
        answer(&mut app);
        let titles: Vec<String> = app
            .panel
            .as_ref()
            .expect("a panel over the answer")
            .sections
            .iter()
            .flat_map(|section| section.actions.iter().map(|a| a.title.clone()))
            .collect();
        assert_eq!(
            titles,
            [
                "Copy Result",
                "Copy Question And Answer",
                "Put answer in search bar",
                "Open Calculator History",
            ]
        );

        let task = choose(&mut app, "Copy Result");
        assert_eq!(settle(&mut app, task), ["42"]);
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Answer copied to clipboard")
        );

        answer(&mut app);
        let task = choose(&mut app, "Copy Question And Answer");
        assert_eq!(settle(&mut app, task), ["6*7 = 42"]);
        let remembered: Vec<(String, String)> = backend
            .calculations
            .lock()
            .unwrap()
            .iter()
            .map(|row| (row.question.clone(), row.answer.clone()))
            .collect();
        assert_eq!(
            remembered,
            [("6*7".into(), "42".into()), ("6*7".into(), "42".into())],
            "both copies are remembered, as CopyCalculatorAnswerAction's are"
        );

        answer(&mut app);
        let task = choose(&mut app, "Put answer in search bar");
        settle(&mut app, task);
        assert_eq!(app.query, "42");
        assert!(app.panel.is_none());
        assert!(matches!(app.page, Page::Root));

        answer(&mut app);
        let task = choose(&mut app, "Open Calculator History");
        settle(&mut app, task);
        assert!(matches!(app.page, Page::Calculator(_)));
    }

    #[test]
    fn without_a_hud_a_copy_only_hides_and_an_exiting_launcher_shows_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path()))).with_hud(false);
        app.query = "6*7".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        assert_eq!(settle(&mut app, task), ["42"]);
        assert!(app.hud_content().is_none(), "GNOME's toplevel has no HUD");

        let mut exiting = LauncherApp::with_index(index(dir.path())).with_hud(true);
        exiting.query = "6*7".into();
        exiting.search();
        let task = exiting.update(Message::LaunchSelected);
        settle(&mut exiting, task);
        assert!(
            exiting.hud_content().is_none(),
            "no process left to show it"
        );
    }

    #[test]
    fn the_hud_surface_is_not_taken_for_the_launcher_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let launcher = window::Id::unique();
        let _ = app.update(Message::Opened(launcher));
        assert_eq!(app.window, Some(launcher));
        let _ = app.update(Message::Command(UiCommand::Hud {
            text: "Paused".into(),
            icon: Some("pause".into()),
        }));
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Paused")
        );
        assert_eq!(app.window, Some(launcher), "the launcher stays up");
        let hud = app.hud.window_id().expect("the HUD surface");
        let _ = app.update(Message::Opened(hud));
        assert_eq!(app.window, Some(launcher), "the HUD is not the launcher");
        let _ = app.update(Message::Closed(hud));
        assert_eq!(app.window, Some(launcher));
        assert!(app.hud_content().is_none());
    }

    #[test]
    fn the_engines_hud_is_refused_where_the_presentation_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path())).with_hud(false);
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
        app.link = Some(EngineLink::new(receiver, sender));
        let _ = app.update(Message::Command(UiCommand::Hud {
            text: "Next Track".into(),
            icon: None,
        }));
        assert!(matches!(outcomes.try_recv(), Ok(UiOutcome::Failed(_))));

        app.hud.set_supported(true);
        let _ = app.update(Message::Command(UiCommand::Hud {
            text: "Next Track".into(),
            icon: Some("forward".into()),
        }));
        assert_eq!(outcomes.try_recv().ok(), Some(UiOutcome::Hidden));
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Next Track")
        );
    }

    /// A no-view command that failed after the launcher hid: the reason is in
    /// the HUD now, and on the launcher's error line when it is next shown,
    /// not only in a desktop notification nobody may be there to post.
    #[test]
    fn a_no_view_commands_failure_is_said_in_the_hud_and_kept_for_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path())).with_hud(true);
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
        app.link = Some(EngineLink::new(receiver, sender));
        let _ = app.update(Message::Command(UiCommand::Failure {
            title: "Could not complete New Window".into(),
            message: "AppleScript is only supported on macOS".into(),
        }));
        assert_eq!(outcomes.try_recv().ok(), Some(UiOutcome::Hidden));
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("AppleScript is only supported on macOS"),
            "the HUD has room for the reason"
        );
        assert_eq!(
            app.error.as_deref(),
            Some("Could not complete New Window: AppleScript is only supported on macOS")
        );
        // The next summon opens a window and searches again.
        let _ = app.update(Message::Opened(window::Id::unique()));
        let mut ui = iced_test::simulator(app.view());
        assert!(
            ui.find("Could not complete New Window: AppleScript is only supported on macOS")
                .is_ok(),
            "the launcher shows it: {}",
            app.state_line()
        );
    }

    #[test]
    fn a_long_hud_message_is_elided_inside_the_surface_and_a_short_one_hugs_its_text() {
        // The pill's width in a frame of the HUD surface: the columns whose
        // middle row is not transparent.
        fn pill_width(app: &LauncherApp) -> usize {
            let id = app.hud.window_id().expect("the HUD surface");
            let (width, height) = crate::hud::SURFACE_SIZE;
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(width as f32, height as f32),
                app.view_for(id),
            );
            let snapshot = ui.snapshot(&app.theme()).expect("the HUD renders");
            if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
                let name = format!("hud-{}", app.hud_content().map_or(0, |hud| hud.text.len()));
                let _ = snapshot.matches_image(std::path::PathBuf::from(&directory).join(name));
            }
            let dir = tempfile::tempdir().unwrap();
            let _ = snapshot.matches_image(dir.path().join("hud"));
            let png = std::fs::read_dir(dir.path())
                .unwrap()
                .find_map(|entry| {
                    let path = entry.ok()?.path();
                    path.extension().is_some_and(|x| x == "png").then_some(path)
                })
                .expect("a PNG");
            let image = ::image::open(png).unwrap().to_rgba8();
            // In surface pixels: the snapshot may be at a higher scale.
            let scale = image.width() as usize / crate::hud::SURFACE_SIZE.0 as usize;
            let row = image.height() / 2;
            let backdrop = *image.get_pixel(0, row);
            (0..image.width())
                .filter(|&x| *image.get_pixel(x, row) != backdrop)
                .count()
                / scale.max(1)
        }

        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let _ = app.show_hud(crate::hud::Hud::new("Copied"));
        let short = pill_width(&app);
        let _ = app.show_hud(crate::hud::Hud::new(
            "Could not start the extension: the worker exited before it answered, \
             with status 1 and nothing on its standard error",
        ));
        let long = pill_width(&app);
        let surface = crate::hud::SURFACE_SIZE.0 as usize;
        assert!(
            short < 120,
            "a short message keeps a short pill: {short} px"
        );
        assert!(
            long > short && long < surface,
            "a long one is cut inside the {surface} px surface: {long} px"
        );
    }

    #[test]
    fn a_refused_paste_copies_the_glyph_with_the_copy_hud() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let task = app.emoji_pasted("🙂".into(), Err("no paste here".into()));
        assert_eq!(settle(&mut app, task), ["🙂"]);
        assert_eq!(app.hud_content(), Some(&crate::hud::Hud::copied()));
    }

    #[test]
    fn a_set_wallpaper_and_a_copied_file_say_so_and_running_does_not() {
        assert_eq!(
            file_actions::file_action_hud("file.wallpaper"),
            Some(crate::hud::Hud::new("Wallpaper set").with_icon("image"))
        );
        assert_eq!(
            file_actions::file_action_hud("file.copy"),
            Some(crate::hud::Hud::copied())
        );
        assert_eq!(file_actions::file_action_hud("file.run"), None);
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let task = app.update(Message::ActionDone(
            file_actions::file_action_hud("file.wallpaper"),
            Ok(()),
        ));
        settle(&mut app, task);
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Wallpaper set")
        );
        let task = app.update(Message::ActionDone(None, Err("no backend".into())));
        settle(&mut app, task);
        assert_eq!(app.error.as_deref(), Some("no backend"));
    }

    #[test]
    fn the_window_switchers_panel_quits_a_known_windows_application() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            rows: vec![
                window_row(7, "Downloads", "Files", 1),
                crate::backend::WindowRow {
                    app_known: false,
                    ..window_row(9, "xterm", "XTerm", 2)
                },
            ],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows.clone()));
        open_windows(&mut app);
        let _ = app.update(Message::WindowsQueryChanged("Downloads".into()));
        let _ = app.update(Message::TogglePanel);
        assert_eq!(
            panel_titles(&app),
            [
                "Focus window",
                "Close window",
                "Quit application",
                "Force quit application"
            ]
        );
        let task = choose(&mut app, "Force quit application");
        settle(&mut app, task);
        assert_eq!(windows.window_quits.lock().unwrap().as_slice(), [(7, true)]);

        let _ = app.update(Message::Command(UiCommand::Show));
        open_windows(&mut app);
        let _ = app.update(Message::WindowsQueryChanged("xterm".into()));
        let _ = app.update(Message::TogglePanel);
        assert_eq!(
            panel_titles(&app),
            ["Focus window", "Close window"],
            "no application to quit"
        );
        let task = choose(&mut app, "Close window");
        settle(&mut app, task);
        assert_eq!(windows.closed.lock().unwrap().as_slice(), [9]);
    }

    fn window_row(id: u32, title: &str, app: &str, pid: u32) -> crate::backend::WindowRow {
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

    #[test]
    fn browse_apps_offers_focus_window_first_and_reads_its_preferences_on_opening() {
        let dir = tempfile::tempdir().unwrap();
        drop(index(dir.path()));
        fs::write(
            dir.path().join("editor.desktop"),
            "[Desktop Entry]\nType=Application\nName=Editor\nExec=/bin/true\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("probe.desktop"),
            "[Desktop Entry]\nType=Application\nName=Probe\nExec=/bin/true\nNoDisplay=true\n",
        )
        .unwrap();
        let config_path = dir.path().join("compass.json");
        let write_config = |show_hidden: bool| {
            fs::write(
                &config_path,
                format!(
                    r#"{{"providers":{{"commands":{{"entrypoints":{{"browse-apps":
                        {{"enabled":true,"preferences":{{"showHidden":{show_hidden}}}}}}}}}}}}}"#
                ),
            )
            .unwrap();
        };
        write_config(false);
        let running = crate::backend::AppRuntimeInfo {
            running: true,
            frontmost: false,
            windows: vec![window_row(42, "Draft", "Editor", 7)],
        };
        let windows = Arc::new(FakeWindows {
            running: vec![("editor.desktop".into(), running)],
            ..FakeWindows::default()
        });
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        app.apply(AppFlags {
            windows: Some(windows.clone() as Arc<dyn crate::backend::WindowBackend>),
            config_path: Some(config_path.clone()),
            ..AppFlags::default()
        });
        let config = compass_core::Config::load_from(&config_path).unwrap();
        app.app_index.apply_root_config(&config.root_config());

        open_builtin(&mut app, "browse apps", "commands:browse-apps");
        let Page::Apps(page) = &app.page else {
            panic!("not Browse Apps: {}", app.state_line());
        };
        assert_eq!(page.heading(), "Applications (4)");

        // A change to the preference applies the next time the view opens,
        // without a restart.
        write_config(true);
        open_builtin(&mut app, "browse apps", "commands:browse-apps");
        let Page::Apps(page) = &app.page else {
            panic!("not Browse Apps: {}", app.state_line());
        };
        assert_eq!(
            page.heading(),
            "Applications (5)",
            "the hidden entry now shows"
        );

        // A running application's panel starts with Focus Window, which Enter
        // runs.
        let task = app.update(Message::AppsQueryChanged("editor".into()));
        settle(&mut app, task);
        let _ = app.open_apps_panel().expect("a panel");
        let titles: Vec<String> = app.panel.as_ref().unwrap().sections[0]
            .actions
            .iter()
            .map(|action| action.title.clone())
            .collect();
        assert_eq!(titles[..2], ["Focus window", "Open application"]);
        let _ = app.update(Message::TogglePanel);
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(windows.activated.lock().unwrap().as_slice(), [42]);

        // One that does not run opens.
        open_builtin(&mut app, "browse apps", "commands:browse-apps");
        let task = app.update(Message::AppsQueryChanged("terminal".into()));
        settle(&mut app, task);
        let _ = app.open_apps_panel().expect("a panel");
        assert_eq!(
            app.panel.as_ref().unwrap().sections[0].actions[0].title,
            "Open application"
        );
    }

    fn windows_app(
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

    fn open_windows(app: &mut LauncherApp) {
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

    #[test]
    fn workspaces_and_the_toggles_are_offered_where_the_compositor_has_them() {
        use compass_core::window_switcher::Capabilities;
        let dir = tempfile::tempdir().unwrap();
        let workspace = |id: &str, name: &str, apps: &[&str]| crate::backend::WorkspaceRow {
            id: id.into(),
            name: name.into(),
            monitor: Some("DP-1".into()),
            window_count: apps.len(),
            apps: apps.iter().map(|a| ((*a).to_owned(), None)).collect(),
            active: false,
        };
        let windows = Arc::new(FakeWindows {
            caps: Capabilities {
                workspaces: true,
                fullscreen: true,
                toggle_floating: true,
                ..Capabilities::default()
            },
            workspaces: vec![
                workspace("1", "1", &["Firefox"]),
                workspace("3", "music", &["Spotify"]),
            ],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows.clone()));
        let is_command = |app: &LauncherApp, entrypoint: &str| matches!(app.selected_row(), Some(RootRow::Command(c)) if c.entrypoint == entrypoint);

        // Until the engine says what the compositor can do, none is offered.
        app.query = "switch workspaces".into();
        app.search();
        assert!(
            !is_command(&app, "switch-workspaces"),
            "{}",
            app.state_line()
        );
        let task = app.window_capabilities_task();
        settle(&mut app, task);
        app.search();
        assert!(
            is_command(&app, "switch-workspaces"),
            "{}",
            app.state_line()
        );
        app.query = "toggle overview".into();
        app.search();
        assert!(
            !is_command(&app, "toggle-overview"),
            "no overview on this compositor"
        );

        // Switch Workspaces lists them, filters by an application on one,
        // and Enter switches to it, then hides.
        app.query = "switch workspaces".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let Page::Workspaces(page) = &app.page else {
            panic!("not on Switch Workspaces: {}", app.state_line());
        };
        assert_eq!(page.shown.len(), 2);
        let _ = app.update(Message::WorkspacesQueryChanged("spotify".into()));
        let _ = app.update(Message::TogglePanel);
        assert_eq!(panel_titles(&app), ["Switch to workspace"]);
        let task = choose(&mut app, "Switch to workspace");
        settle(&mut app, task);
        assert_eq!(windows.focused_workspaces.lock().unwrap().as_slice(), ["3"]);
        assert!(
            !matches!(app.page, Page::Workspaces(_)),
            "hidden, and back at the root next time"
        );

        // A toggle runs from root search.
        let _ = app.update(Message::Command(UiCommand::Show));
        app.query = "toggle floating".into();
        app.search();
        assert!(is_command(&app, "toggle-floating"));
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            windows.toggles.lock().unwrap().as_slice(),
            [crate::backend::WindowToggle::Floating]
        );
    }

    #[test]
    fn a_refused_toggle_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            caps: compass_core::window_switcher::Capabilities {
                fullscreen: true,
                ..Default::default()
            },
            toggle_refusal: Some("Active window is not on the current workspace"),
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows));
        let task = app.window_capabilities_task();
        settle(&mut app, task);
        app.query = "toggle fullscreen".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        assert_eq!(
            app.error.as_deref(),
            Some("Active window is not on the current workspace")
        );
    }

    #[test]
    fn switching_windows_lists_others_and_enter_focuses_one_then_hides() {
        let dir = tempfile::tempdir().unwrap();
        let own = std::process::id();
        let windows = Arc::new(FakeWindows {
            rows: vec![
                window_row(7, "Downloads", "Files", 1),
                window_row(8, "Compass", "Compass", own),
                window_row(9, "notes.txt", "Text Editor", 2),
                // Inside the Flatpak the Shell reports a host pid the
                // launcher cannot see, so its window is known by app id.
                crate::backend::WindowRow {
                    wm_class: crate::APP_ID.to_owned(),
                    ..window_row(10, "Compass", "Compass", 4_000_000)
                },
            ],
            ..FakeWindows::default()
        });
        let (mut app, backend) = windows_app(dir.path(), Some(windows.clone()));
        open_windows(&mut app);
        assert!(app.showing_windows());
        let Page::Windows(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.all.len(), 2, "the launcher's own window is left out");
        assert_eq!(
            backend.recorded.lock().unwrap().as_slice(),
            ["commands:switch-windows"]
        );

        let task = app.update(Message::WindowsQueryChanged("notes".into()));
        settle(&mut app, task);
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(windows.activated.lock().unwrap().as_slice(), [9]);
        assert!(
            !app.showing_windows(),
            "hidden, and back at the root next time"
        );
    }

    #[test]
    fn control_q_closes_the_selected_window_and_reloads_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            rows: vec![window_row(3, "Old tab", "Browser", 1)],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows.clone()));
        open_windows(&mut app);
        let before = windows.listed.load(std::sync::atomic::Ordering::SeqCst);
        let task = app.update(chord("q", iced::keyboard::Modifiers::CTRL));
        settle(&mut app, task);
        assert_eq!(windows.closed.lock().unwrap().as_slice(), [3]);
        assert_eq!(
            windows.listed.load(std::sync::atomic::Ordering::SeqCst),
            before + 1,
            "the list is asked for again"
        );
        assert!(
            app.showing_windows(),
            "closing a window keeps the switcher open"
        );
    }

    #[test]
    fn a_failed_switch_says_why_and_stays_open() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            rows: vec![window_row(3, "Gone", "Browser", 1)],
            fail_activate: true,
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows));
        open_windows(&mut app);
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::Windows(page) = &app.page else {
            panic!("a failed switch must not hide the launcher")
        };
        assert_eq!(page.notice.as_deref(), Some("that window has gone"));
    }

    #[test]
    fn switching_windows_without_an_engine_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) = windows_app(dir.path(), None);
        open_windows(&mut app);
        let Page::Windows(page) = &app.page else {
            unreachable!()
        };
        assert!(
            matches!(&page.status, crate::windows_page::Status::Failed(r) if r.contains("engine")),
            "{:?}",
            page.status
        );
    }

    #[test]
    fn a_backend_answer_naming_a_command_becomes_a_command_row() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec![
                "commands:clipboard-history".into(),
                "applications:alpha".into(),
            ],
            ..TestBackend::default()
        });
        let mut app = backend_app(dir.path(), backend);
        let task = app.update(Message::QueryChanged("a".into()));
        settle(&mut app, task);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(matches!(app.results.first(), Some(RootRow::Command(_))));
        assert!(matches!(app.results.get(1), Some(RootRow::App(_))));
    }
    use super::icon_tests::{app_with_icons, asked, recording};
    use crate::icons::IconArt;
    use std::path::{Path, PathBuf};

    fn repo_builtin_icons() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extra/builtin-icons")
            .canonicalize()
            .expect("the builtin icon set is in the repository")
    }

    #[test]
    fn a_builtin_command_draws_its_tiled_icon_and_without_the_set_its_initial() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.icons = true;
        app.query = "clipboard history".into();
        app.search();
        assert!(matches!(app.selected_row(), Some(RootRow::Command(_))));
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(
                ui.find("C").is_ok(),
                "with no builtin icon set installed the row keeps its initial"
            );
        }
        app.builtin_icons = Some(repo_builtin_icons());
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Clipboard History").is_ok());
        assert!(
            ui.find("C").is_err(),
            "the builtin icon replaces the initial"
        );
    }

    #[test]
    fn search_files_rows_draw_their_file_type_icons() {
        let dir = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let photo = files.path().join("photo.png");
        fs::write(&photo, b"x").unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.icons = true;
        app.builtin_icons = Some(repo_builtin_icons());
        let (log, lookup) = recording(&["image-png"]);
        app.icon_lookup = lookup;
        let page = crate::files_page::FilesPage::default();
        let generation = page.generation;
        app.page = Page::Files(page);
        let row = |path: &Path, category: &str| crate::backend::FileRow {
            path: path.to_string_lossy().into_owned(),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            category: category.into(),
        };
        let _ = app.update(Message::FilesLoaded {
            generation,
            result: Ok(crate::backend::FileResults {
                heading: "Files".into(),
                files: vec![row(&photo, "Images"), row(files.path(), "Folders")],
            }),
        });
        assert_eq!(
            app.file_glyphs.cached(&photo.to_string_lossy()),
            Some(&crate::icons::Glyph::Art(IconArt::Vector(PathBuf::from(
                "/i/image-png.svg"
            ))))
        );
        assert_eq!(
            app.file_glyphs.cached(&files.path().to_string_lossy()),
            Some(&crate::icons::Glyph::builtin("folder")),
            "a folder no theme has an icon for draws the builtin folder"
        );
        assert!(asked(&log).contains(&"inode-directory".to_owned()));
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("photo.png").is_ok());
        assert!(ui.find("P").is_err(), "no initial on a file row");
    }

    #[test]
    fn a_window_row_draws_its_applications_icon_or_the_app_window_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let windows = Arc::new(FakeWindows {
            rows: vec![
                window_row(7, "Downloads", "Firefox", 1),
                crate::backend::WindowRow {
                    app_known: false,
                    ..window_row(9, "xterm", "XTerm", 2)
                },
            ],
            ..FakeWindows::default()
        });
        let (mut app, _) = windows_app(dir.path(), Some(windows));
        let fresh = app_with_icons(dir.path());
        app.app_index = fresh.app_index;
        app.icons = true;
        app.builtin_icons = Some(repo_builtin_icons());
        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        open_windows(&mut app);
        assert!(asked(&log).contains(&"firefox".to_owned()));
        let Page::Windows(page) = &app.page else {
            panic!("not the window switcher: {}", app.state_line());
        };
        let firefox = page.all.iter().find(|w| w.id == 7).unwrap();
        let xterm = page.all.iter().find(|w| w.id == 9).unwrap();
        assert_eq!(
            app.window_app(firefox).and_then(|item| app.row_art(item)),
            Some(&IconArt::Vector(PathBuf::from("/i/firefox.svg")))
        );
        assert!(app.window_app(xterm).is_none());
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("X").is_err(), "the unknown window draws AppWindow");
        assert!(ui.find("F").is_err());
    }

    #[test]
    fn the_default_pickers_mark_is_the_green_check_icon() {
        use crate::backend::DefaultAppRow;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            default_apps: vec![DefaultAppRow {
                id: "firefox.desktop".into(),
                name: "Firefox".into(),
                description: "Browse the web".into(),
                is_default: true,
            }],
            ..TestBackend::default()
        });
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend);
        app.builtin_icons = Some(repo_builtin_icons());
        open_builtin(
            &mut app,
            "set default browser",
            "commands:set-default-browser",
        );
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("Firefox").is_ok());
        assert!(
            ui.find("✓ Default").is_err(),
            "the check icon replaces the text mark"
        );
    }

    #[test]
    fn clipboard_rows_draw_the_builtin_for_their_kind() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.builtin_icons = Some(repo_builtin_icons());
        let page = crate::clipboard_page::ClipboardPage {
            rows: vec![crate::backend::ClipboardRow {
                id: "1".into(),
                preview: "hello".into(),
                kind: crate::backend::ClipboardRowKind::Text,
                pinned: false,
                url_host: None,
            }],
            status: crate::clipboard_page::Status::Ready,
            ..crate::clipboard_page::ClipboardPage::default()
        };
        app.page = Page::Clipboard(page);
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("hello").is_ok());
        assert!(ui.find("T").is_err(), "the text builtin, not an initial");
    }

    #[test]
    fn extension_script_and_shortcut_rows_draw_their_icons_in_root_search() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let ext = dir.path().join("extensions/hello");
        fs::create_dir_all(ext.join("assets")).unwrap();
        fs::write(ext.join("assets/hello.png"), b"png").unwrap();
        fs::write(
            ext.join("package.json"),
            r#"{"name": "hello", "title": "Hello", "author": "someone", "icon": "hello.png",
                "commands": [{"name": "write", "title": "Write Greeting", "mode": "no-view"}]}"#,
        )
        .unwrap();
        index(dir.path());
        let mut app = LauncherApp::with_index(
            AppIndex::builder()
                .dir(dir.path())
                .extension_dirs([dir.path().join("extensions")])
                .build(),
        );
        app.backend = Some(backend);
        app.icons = true;
        app.builtin_icons = Some(repo_builtin_icons());
        app.query = "greeting".into();
        app.search();
        assert_eq!(app.selected_row(), Some(RootRow::Extension(0)));
        let extension_icon = ext.join("assets/hello.png");
        let key =
            compass_core::image_url::ImageUrl::local(extension_icon.to_string_lossy()).to_url();
        assert_eq!(
            app.url_glyphs.get(&key),
            Some(&Some(crate::icons::Glyph::Art(IconArt::Raster(
                extension_icon
            )))),
            "the extension's own icon, from its assets"
        );
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("Write Greeting").is_ok());
            assert!(ui.find("W").is_err(), "the icon replaces the initial");
        }

        // A script whose header names an emoji.
        app.app_index.set_scripts(vec![script_item(
            "party.sh",
            "Party Time",
            compass_core::script_command::OutputMode::Silent,
            0,
        )]);
        let _ = app.update(Message::ScriptIconsLoaded(Ok(vec![(
            "party.sh".into(),
            "icon://emoji/🎉".into(),
        )])));
        app.query = "party time".into();
        app.search();
        assert!(matches!(app.selected_row(), Some(RootRow::Script(_))));
        {
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("🎉").is_ok(), "the emoji is drawn as text");
            assert!(ui.find("P").is_err());
        }

        // A shortcut with a builtin icon: on a purple tile.
        app.app_index
            .set_shortcuts(vec![compass_core::shortcut_service::CachedShortcut {
                id: "s1".into(),
                name: "Docs Search".into(),
                icon: "icon://omnicast/link".into(),
                link: compass_core::shortcut::parse_link("https://docs.rs/{query}"),
                app: "default".into(),
                open_count: 0,
                created_at: 0,
                updated_at: 0,
                last_opened_at: None,
            }]);
        app.query = "docs search".into();
        app.search();
        assert!(matches!(app.selected_row(), Some(RootRow::Shortcut(_))));
        let key = "icon://omnicast/link?bg_tint=purple";
        let Some(Some(crate::icons::Glyph::Builtin { name, tile, .. })) = app.url_glyphs.get(key)
        else {
            panic!("no glyph for {key}: {:?}", app.url_glyphs.keys());
        };
        assert_eq!(name, "link");
        assert!(tile.is_some(), "a shortcut's builtin sits on a purple tile");
    }

    #[test]
    fn a_favicon_is_fetched_once_into_the_cache_and_then_drawn() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.icons = true;
        app.builtin_icons = Some(repo_builtin_icons());
        app.favicon_service = compass_core::favicon::Service::Twenty;
        let row = crate::backend::ClipboardRow {
            id: "1".into(),
            preview: "https://example.com/page".into(),
            kind: crate::backend::ClipboardRowKind::Link,
            pinned: false,
            url_host: Some("example.com".into()),
        };
        let page = crate::clipboard_page::ClipboardPage::default();
        let generation = page.generation;
        app.page = Page::Clipboard(page);
        let load = |app: &mut LauncherApp| {
            let _ = app.update(Message::ClipboardLoaded {
                generation,
                result: Ok(vec![row.clone()]),
            });
        };
        let favicon = clipboard_url(&row).expect("a link has a favicon").to_url();
        let favicon = favicon.as_str();
        assert!(favicon.starts_with("icon://favicon/example.com?fallback="));

        // Without remote icons (every test's default) nothing is fetched and
        // the link builtin stands in.
        load(&mut app);
        assert!(app.remote_requested.is_empty());
        assert_eq!(
            app.url_glyphs.get(favicon),
            Some(&Some(crate::icons::Glyph::builtin("link")))
        );

        app.remote_icons = true;
        app.url_glyphs.clear();
        load(&mut app);
        let wanted = "https://twenty-icons.com/example.com/128";
        assert_eq!(
            app.remote_requested.iter().collect::<Vec<_>>(),
            [wanted],
            "the favicon is asked of the configured service"
        );
        load(&mut app);
        assert_eq!(app.remote_requested.len(), 1, "and asked for once");

        let cached = dir.path().join("favicon.png");
        fs::write(&cached, b"png").unwrap();
        let _ = app.update(Message::ExtensionImageFetched {
            url: wanted.into(),
            result: Ok(cached.clone()),
        });
        assert_eq!(
            app.url_glyphs.get(favicon),
            Some(&Some(crate::icons::Glyph::Art(IconArt::Raster(cached)))),
            "once in the cache, the favicon is drawn"
        );
    }
}

#[cfg(test)]
mod quick_launch_tests {
    use super::*;
    use iced::keyboard::{Key, Modifiers};

    #[test]
    fn ctrl_one_through_nine_are_positions_counting_from_zero() {
        for (digit, want) in [("1", 0), ("2", 1), ("5", 4), ("9", 8)] {
            assert_eq!(
                quick_launch_position(Key::Character(digit), Modifiers::CTRL),
                Some(want),
                "Ctrl+{digit}"
            );
        }
    }

    #[test]
    fn there_is_no_ctrl_zero() {
        // The digits are one-based because the rows are. A zero would have to
        // mean either "the first" or "the tenth" and neither is what anyone
        // presses it for.
        assert_eq!(
            quick_launch_position(Key::Character("0"), Modifiers::CTRL),
            None
        );
    }

    #[test]
    fn a_digit_without_control_is_somebody_elses() {
        // Typing "3" into the search field must reach the field.
        assert_eq!(
            quick_launch_position(Key::Character("3"), Modifiers::default()),
            None
        );
    }

    #[test]
    fn control_plus_another_modifier_is_a_different_chord() {
        // Not ours to claim: the desktop may already bind these, and on many
        // layouts Ctrl+Shift+1 is not a digit at all.
        for extra in [Modifiers::SHIFT, Modifiers::ALT, Modifiers::LOGO] {
            assert_eq!(
                quick_launch_position(Key::Character("1"), Modifiers::CTRL | extra),
                None,
                "{extra:?}"
            );
        }
    }

    #[test]
    fn a_non_digit_character_is_not_a_quick_launch() {
        for text in ["b", "-", "", "1x", "12"] {
            assert_eq!(
                quick_launch_position(Key::Character(text), Modifiers::CTRL),
                None,
                "{text:?}"
            );
        }
    }

    #[test]
    fn ctrl_n_selects_and_launches_the_nth_visible_row() {
        // THE BUG THIS IS HERE FOR: `results` holds indices into the
        // application index, and the selection is a position within `results`.
        // Ctrl+3 means the third ROW, not item 3. Confusing the two launches
        // something plausible and wrong, which is the worst kind to have here
        // because nothing looks broken.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = super::tests::app_with_rows(dir.path());

        let rows: Vec<String> = app
            .results
            .iter()
            .filter_map(|row| match row {
                RootRow::App(i) => app.app_index.items().get(*i),
                RootRow::Command(_)
                | RootRow::Extension(_)
                | RootRow::Shortcut(_)
                | RootRow::Script(_)
                | RootRow::RhaiScript(_)
                | RootRow::Fallback(_)
                | RootRow::Calculator
                | RootRow::Update => None,
            })
            .map(|item| item.name().to_owned())
            .collect();
        assert!(rows.len() >= 2, "need several rows: {rows:?}");

        for (position, name) in rows.iter().enumerate().take(9) {
            let digit = (position + 1).to_string();
            let _ = app.update(super::tests::ctrl_digit(&digit));
            assert_eq!(
                app.selected_item().map(compass_core::AppItem::name),
                Some(name.as_str()),
                "Ctrl+{digit} must resolve to row {position}"
            );
        }
    }

    #[test]
    fn a_row_that_is_not_there_does_nothing_at_all() {
        // THE BOUNDS ARE THE ROWS', NOT THE INDEX'S. A query filters the list,
        // so `results` is shorter than `app_index.items()` -- and a check
        // against the wrong one accepts a position that has no row, then
        // selects it. The fixture is queried precisely so the two lengths
        // differ: a control that swapped them was SILENT until this test did.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = super::tests::app_with_rows(dir.path());
        let _ = app.update(Message::QueryChanged("fir".to_owned()));

        let rows = app.results.len();
        let items = app.app_index.items().len();
        assert!(
            rows < items,
            "the query must narrow the list: {rows} of {items}"
        );
        assert!(rows < 9, "and leave fewer than nine rows: {rows}");

        let before = app.selected;
        // A position with no row, but well inside the item index.
        let _ = app.update(super::tests::ctrl_digit(&(rows + 1).to_string()));
        assert_eq!(app.selected, before, "the selection must not move either");
    }

    #[test]
    fn the_setting_turns_it_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = super::tests::app_with_rows(dir.path());
        app.quick_launch = false;
        app.selected = 1;

        let _ = app.update(super::tests::ctrl_digit("1"));
        assert_eq!(
            app.selected, 1,
            "with quick_launch off, Ctrl+1 must not move the selection"
        );
    }

    #[test]
    fn it_is_on_by_default() {
        // #87 asks for on by default: it costs nothing when unused, because
        // the chords are otherwise unbound.
        assert!(compass_core::config::LauncherConfig::default().quick_launch());
        assert!(AppFlags::default().quick_launch);
    }

    #[test]
    fn the_panel_takes_the_chord_while_it_is_open() {
        // While the panel is open the list is not what a keystroke is aimed
        // at, and launching a row out from under an open action panel would be
        // a surprise.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = super::tests::app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        assert!(app.panel.is_some(), "precondition");
        app.selected = 1;

        let _ = app.update(super::tests::ctrl_digit("1"));
        assert_eq!(app.selected, 1, "the row under the panel did not move");
        assert!(app.panel.is_some(), "and the panel is still open");
    }

    #[test]
    fn a_named_key_is_not_a_quick_launch() {
        assert_eq!(
            quick_launch_position(
                Key::Named(iced::keyboard::key::Named::Enter),
                Modifiers::CTRL
            ),
            None
        );
    }
}

#[cfg(test)]
mod icon_tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::icons::IconArt;

    /// Three entries, two of which name an icon.
    pub(super) fn app_with_icons(dir: &std::path::Path) -> LauncherApp {
        for (file, name, icon) in [
            ("firefox.desktop", "Firefox", Some("firefox")),
            ("files.desktop", "Files", Some("system-file-manager")),
            ("terminal.desktop", "Terminal", None),
        ] {
            let icon_line = icon.map_or_else(String::new, |value| format!("Icon={value}\n"));
            fs::write(
                dir.join(file),
                format!(
                    "[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n{icon_line}"
                ),
            )
            .expect("write entry");
        }
        LauncherApp::with_index(AppIndex::builder().dir(dir).build())
    }

    /// A lookup that answers for `hits` and records every name it is asked.
    pub(super) fn recording(hits: &[&str]) -> (Arc<Mutex<Vec<String>>>, IconLookup) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&asked);
        let hits: Vec<String> = hits.iter().map(|hit| (*hit).to_owned()).collect();
        let lookup = IconLookup::new(move |name: &str| {
            log.lock().expect("lock").push(name.to_owned());
            hits.iter()
                .any(|hit| hit == name)
                .then(|| PathBuf::from(format!("/i/{name}.svg")))
        });
        (asked, lookup)
    }

    pub(super) fn asked(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().expect("lock").clone()
    }

    fn row<'a>(app: &'a LauncherApp, name: &str) -> &'a AppItem {
        app.results
            .iter()
            .filter_map(|row| match row {
                RootRow::App(index) => app.app_index.items().get(*index),
                RootRow::Command(_)
                | RootRow::Extension(_)
                | RootRow::Shortcut(_)
                | RootRow::Script(_)
                | RootRow::RhaiScript(_)
                | RootRow::Fallback(_)
                | RootRow::Calculator
                | RootRow::Update => None,
            })
            .find(|item| item.name() == name)
            .unwrap_or_else(|| panic!("{name} is not a row"))
    }

    #[test]
    fn icons_off_looks_nothing_up_at_all() {
        // Explicitly disabling icons must cost nothing: not a directory walk,
        // not even a cached miss.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = false;

        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("fi".to_owned()));

        assert!(!app.results.is_empty(), "precondition: there are rows");
        assert!(asked(&log).is_empty(), "{:?}", asked(&log));
        assert!(app.icon_cache.is_empty());
    }

    #[test]
    fn icons_on_warms_only_the_rows_on_screen() {
        // Bounded by the visible list rather than by the index: that is the
        // whole reason resolution happens in `update` and not in `view`.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("firefox".to_owned()));

        assert_eq!(asked(&log), ["firefox"], "{:?}", asked(&log));
    }

    #[test]
    fn an_entry_with_no_icon_key_is_never_looked_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (log, lookup) = recording(&[]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("terminal".to_owned()));

        assert!(!app.results.is_empty(), "precondition: Terminal is a row");
        assert!(asked(&log).is_empty(), "{:?}", asked(&log));
    }

    #[test]
    fn the_same_name_is_resolved_once_however_many_keystrokes() {
        // A launcher re-ranks on every keystroke. Without the cache this is
        // one theme walk per character typed.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        for query in ["f", "fi", "fir", "fire", "firef"] {
            let _ = app.update(Message::QueryChanged(query.to_owned()));
        }

        let names = asked(&log);
        assert_eq!(
            names.iter().filter(|name| *name == "firefox").count(),
            1,
            "{names:?}"
        );
    }

    #[test]
    fn a_resolved_row_draws_its_icon_and_an_unresolved_one_falls_back() {
        // The ordinary state of a mixed desktop: the theme has one of them.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (_log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("f".to_owned()));

        assert_eq!(
            app.row_art(row(&app, "Firefox")),
            Some(&IconArt::Vector(PathBuf::from("/i/firefox.svg")))
        );
        assert_eq!(
            app.row_art(row(&app, "Files")),
            None,
            "an icon the theme does not have must fall back to the initial"
        );
    }

    #[test]
    fn a_resolved_icon_is_still_not_drawn_once_icons_are_turned_off() {
        // The cache outlives the toggle, so the gate has to be at the draw and
        // not only at the warm.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_icons(dir.path());
        app.icons = true;

        let (_log, lookup) = recording(&["firefox"]);
        app.icon_lookup = lookup;
        let _ = app.update(Message::QueryChanged("firefox".to_owned()));

        assert!(
            app.row_art(row(&app, "Firefox")).is_some(),
            "precondition: resolved"
        );

        app.icons = false;
        assert_eq!(app.row_art(row(&app, "Firefox")), None);
    }

    #[test]
    fn native_app_icons_are_on_by_default() {
        assert!(
            compass_core::config::LauncherConfig::default()
                .appearance()
                .icons()
        );
        assert!(AppFlags::default().icons);
    }
}

/// The `tint` option (#86): translucency, and deliberately not called blur.
#[cfg(test)]
mod tint_tests {
    use super::*;
    use crate::preset::{self, Preset};

    #[test]
    fn only_raycast_tints() {
        // Raycast is the one preset imitating a look built on real compositor
        // blur, so it gets the closest thing available. The Spotlight-simple
        // default does not, which is also what keeps the VM tier's pixel gates
        // seeing an unchanged image.
        assert!(Preset::Raycast.tint());
        assert!(!Preset::Gnome.tint());
        assert!(!Preset::Flow.tint());
        assert!(!Preset::Rofi.tint());
    }

    #[test]
    fn an_explicit_setting_beats_the_preset_both_ways() {
        // Both directions: a one-way override reads as working until someone
        // tries to turn the feature off under a preset that enables it.
        assert!(!preset::resolve(Some("raycast"), None, Some(false)).tint);
        assert!(preset::resolve(Some("gnome"), None, Some(true)).tint);
    }

    #[test]
    fn the_flag_reaches_the_launcher() {
        // Through `apply`, which is the real assignment list `compass::run`
        // uses -- a helper that set the field itself would be a test of the
        // helper. This is the control #108 recorded and could not close.
        let mut app = LauncherApp::with_index(AppIndex::default());
        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some("raycast"), None, None),
            ..AppFlags::default()
        });
        assert!(app.tint, "the resolved preset's tint never reached the app");

        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some("gnome"), None, None),
            ..AppFlags::default()
        });
        assert!(!app.tint);
    }

    #[test]
    fn a_translucent_card_asks_for_blur_behind_itself_and_an_opaque_one_takes_it_away() {
        // `WindowMaterial.enabled: blurEnabled`, `radius: cornerRadius`,
        // `region: (shadowPadding, shadowPadding, w, h)`.
        let mut app = LauncherApp::with_index(AppIndex::default());
        let card = iced::Size::new(768.0, 312.0);

        let _ = app.update(Message::CardMeasured(card));
        assert_eq!(app.material_asked, None, "no window, nothing to blur");

        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some("raycast"), None, None),
            ..AppFlags::default()
        });
        app.window = Some(window::Id::unique());
        let _ = app.update(Message::CardMeasured(card));
        let asked = app.material_asked.expect("a translucent card is blurred");
        assert_eq!(
            (asked.x, asked.y, asked.width, asked.height),
            (
                i32::from(design::SHADOW_PADDING),
                i32::from(design::SHADOW_PADDING),
                768,
                312
            )
        );
        assert_eq!(asked.radius, i32::from(app.geometry.card_radius));

        // The card grows: the region follows it.
        let _ = app.update(Message::CardMeasured(iced::Size::new(768.0, 560.0)));
        assert_eq!(app.material_asked.map(|region| region.height), Some(560));

        // Tint turned off: the blur is taken away.
        app.tint = false;
        let _ = app.update(Message::CardMeasured(iced::Size::new(768.0, 560.0)));
        assert_eq!(app.material_asked, None);
    }

    #[test]
    fn a_closed_window_forgets_its_blur() {
        let mut app = LauncherApp::with_index(AppIndex::default());
        let id = window::Id::unique();
        app.window = Some(id);
        app.tint = true;
        let _ = app.update(Message::CardMeasured(iced::Size::new(768.0, 560.0)));
        assert!(app.material_asked.is_some());
        let _ = app.update(Message::Closed(id));
        assert_eq!(
            app.material_asked, None,
            "the next window is a new surface and must be asked again"
        );
    }

    #[test]
    fn tint_changes_the_card_background_and_only_its_alpha() {
        let surface = design::DARK.surface;
        let opaque = card_background(surface, false);
        let tinted = card_background(surface, true);

        assert!(
            (opaque.a - 1.0).abs() < f32::EPSILON,
            "the untinted card must be fully opaque, got alpha {}",
            opaque.a
        );
        assert!(
            tinted.a < opaque.a,
            "tint did not make the card more transparent: {} vs {}",
            tinted.a,
            opaque.a
        );

        // Translucency, not a different colour. If the hue moved, this would be
        // a theme change wearing a transparency setting's name.
        assert!(
            (tinted.r - opaque.r).abs() < f32::EPSILON
                && (tinted.g - opaque.g).abs() < f32::EPSILON
                && (tinted.b - opaque.b).abs() < f32::EPSILON,
            "tint changed the card's colour, not just its alpha"
        );
    }

    #[test]
    fn the_tinted_card_stays_legible() {
        // A launcher bound to Super+Space is text over an arbitrary wallpaper.
        // There is a temptation to push the alpha down because it looks better
        // in a screenshot over a photo; this is the floor that stops it.
        //
        // Asserted on the colour the view actually fills with, not on
        // `TINT_ALPHA` directly: clippy rejects an assertion over a constant,
        // and rightly -- `assert!(SOME_CONST >= 0.75)` is evaluated by the
        // compiler, not by the test, so it proves nothing about the code path.
        // The same mistake was already fixed once here in a quick-launch
        // assertion over `DEFAULT_QUICK_LAUNCH`.
        let alpha = card_background(design::DARK.surface, true).a;
        assert!(
            alpha >= 0.75,
            "the tinted card fills at alpha {alpha}, low enough that body text over a bright \
             wallpaper loses contrast. Legibility is not negotiable for the launcher."
        );
        assert!(
            alpha < 1.0,
            "a tint that fills at alpha {alpha} is not a tint"
        );
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;
    use crate::preset::{self, Preset};

    /// A launcher built from flags, through the code `compass` uses.
    ///
    /// `apply` rather than a hand-written assignment list: a helper that set
    /// the fields itself would pass while the real copying silently stopped,
    /// which is the whole mutation these tests exist to catch. It was written
    /// that way first and the control caught it.
    fn app_with(preset_name: &str, icons: Option<bool>) -> LauncherApp {
        let resolved = preset::resolve(Some(preset_name), icons, None);
        let mut app = LauncherApp::with_index(AppIndex::builder().build());
        app.apply(AppFlags {
            icons: resolved.icons,
            appearance_preset: resolved,
            ..AppFlags::default()
        });
        app
    }

    #[test]
    fn the_preset_reaches_the_state_that_draws() {
        // The wiring, not the resolver: `preset::resolve` is tested on its
        // own, and this is the half that would silently do nothing if the
        // flags stopped being copied across.
        let rofi = app_with("rofi", None);
        assert_eq!(rofi.geometry.row_height, Preset::Rofi.geometry().row_height);
        assert_eq!(rofi.geometry.card_radius, 0);
        assert!(!rofi.icons);
        assert!(!rofi.field_rule);
        assert!(!rofi.subtitles, "the dense preset draws one line per row");

        let flow = app_with("flow", None);
        assert!(flow.field_rule, "flow draws the rule under the field");
        assert!(flow.icons);
    }

    #[test]
    fn the_default_launcher_draws_the_shipped_geometry() {
        // `with_index` is what every other test builds, so a preset layer that
        // changed the default would change all of them -- and move the VM
        // tier's containment box with no commit saying so.
        let app = LauncherApp::with_index(AppIndex::builder().build());
        assert_eq!(
            format!("{:?}", app.geometry),
            format!("{:?}", design::GEOMETRY)
        );
        assert!(!app.field_rule);
    }

    #[test]
    fn a_preset_and_an_explicit_key_both_reach_the_app() {
        // raycast turns icons on; the explicit key turns them back off.
        assert!(app_with("raycast", None).icons);
        assert!(!app_with("raycast", Some(false)).icons);
    }
}

/// Tests that render `view` headlessly.
///
/// # Why this module exists
///
/// Every other test in this crate asserts on state. `view` returns an opaque
/// `Element`, so a branch inside it could be deleted with nothing failing --
/// and one was: #108 recorded a control that mutated away the `field_rule`
/// branch and left all 79 tests green. The design surrogate did not close it
/// either, because it builds its own DOM from the same tokens and never
/// executes the Iced tree.
///
/// `iced_test` renders the real tree in headless mode, which is what finally
/// makes those branches reachable from a test.
///
/// # Bounds rather than pixels
///
/// `Simulator::snapshot` compares against a committed PNG. That would make the
/// assertion depend on which fonts the machine running it happens to have, and
/// a gate that reddens when a runner image changes its font package is a gate
/// people turn off. Selecting a widget by its text and reading its **layout
/// bounds** asserts the same structure without depending on how the glyphs came
/// out.
/// Input-method behaviour: the "watch for" item on the Phase 1 issue.
///
/// #4 flags IME and screen-reader behaviour with an explicit deadline: "Test
/// both here, not in Phase 5 — if Iced can't do them, ADR-0001 needs
/// revisiting while that is still cheap." Nothing tested it, so the risk was
/// carried unresolved rather than answered.
///
/// # The stack does support it, end to end
///
/// * `winit` 0.30 implements `zwp_text_input_v3` on Wayland
///   (`platform_impl/linux/wayland/seat/text_input/`) and emits
///   `WindowEvent::Ime(Enabled | Preedit | Commit | Disabled)`.
/// * `iced_winit` 0.14 converts those into `Event::InputMethod`
///   (`conversion.rs`), and calls `set_ime_allowed`, `set_ime_cursor_area` and
///   `set_ime_purpose` from `enable_ime`, which runs when a widget asks for an
///   input method — so a focused search field turns the IME on by itself.
/// * `iced_core` carries `InputMethod` and `Preedit`.
///
/// So **ADR-0001 does not need revisiting on this point.** That is a claim
/// about libraries, and libraries change, which is what these tests are for:
/// they fail if a future Iced stops routing composed text to the field.
///
/// # What these do and do not prove
///
/// They drive `Event::InputMethod` through the real widget tree and assert the
/// query is what a CJK or accented commit should leave behind. They do **not**
/// prove a real IME works against a real compositor — that needs ibus or fcitx
/// in the VM tier, and it is a different test. What they rule out is the
/// cheaper and more likely failure: composed text never reaching the field at
/// all, which would make the launcher unusable for anyone typing Japanese,
/// Chinese, Korean or with a compose key, and which no other test would catch.
#[cfg(test)]
mod ime_tests {
    use super::*;
    use iced_winit::core::input_method;

    fn focused_app() -> LauncherApp {
        let mut app = LauncherApp::with_index(AppIndex::default());
        app.apply(AppFlags::default());
        app
    }

    /// One simulator, focused, driven by `events`, with the messages applied.
    ///
    /// A fresh simulator per interaction would rebuild the widget tree and lose
    /// focus, so a two-step sequence has to share one. Focus itself comes from
    /// clicking the placeholder: in the running launcher it arrives via a
    /// `Task` (`focus_search`, dispatched on show) and the simulator does not
    /// run tasks.
    fn drive(app: &mut LauncherApp, events: Vec<Vec<iced_winit::core::Event>>) {
        let mut out = Vec::new();
        for batch in events {
            // Scoped so the borrow of `app.view()` ends before `app.update`.
            {
                let mut ui = iced_test::simulator(app.view());
                ui.click("Search…").expect("the search field is clickable");
                let _ = ui.simulate(batch);
                out.extend(ui.into_messages());
            }
            for message in out.drain(..) {
                let _ = app.update(message);
            }
        }
    }

    fn commit(text: &str) -> Vec<iced_winit::core::Event> {
        vec![iced_winit::core::Event::InputMethod(
            input_method::Event::Commit(text.to_owned()),
        )]
    }

    /// CONTROL. If plain typing does not reach the query in this harness, the
    /// IME assertions below are measuring the harness, not the input method.
    /// This failed first — the field was unfocused and nothing reached it —
    /// which is exactly the false "IME is broken" this control exists to stop.
    #[test]
    fn control_typewrite_reaches_the_query() {
        let mut app = focused_app();
        let mut ui = iced_test::simulator(app.view());
        ui.click("Search…").expect("the search field is clickable");
        let _ = ui.typewrite("abc");
        for message in ui.into_messages() {
            let _ = app.update(message);
        }
        assert_eq!(
            app.query, "abc",
            "CONTROL: plain typing did not reach the query, so nothing below measures IME"
        );
    }

    #[test]
    fn a_committed_composition_reaches_the_query() {
        let mut app = focused_app();
        drive(&mut app, vec![commit("日本語")]);
        assert_eq!(
            app.query, "日本語",
            "a committed IME composition did not reach the query field. Typed ASCII arrives \
             as key events and would still work, so every other test here would pass while \
             the launcher was unusable for anyone composing text"
        );
    }

    #[test]
    fn a_preedit_does_not_commit_early() {
        let mut app = focused_app();
        drive(
            &mut app,
            vec![vec![iced_winit::core::Event::InputMethod(
                input_method::Event::Preedit("にほんご".to_owned(), None),
            )]],
        );
        assert_eq!(
            app.query, "",
            "an uncommitted IME pre-edit leaked into the query; the launcher would search \
             for each intermediate composition state"
        );
    }
}
#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::preset::{self, Preset};

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
                format!(
                    "[Desktop Entry]\nType=Application\nName=Application {i:02}\nExec=/bin/true\n"
                ),
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
}

#[cfg(test)]
mod cold_start_tests {
    use super::*;

    #[test]
    fn the_first_frame_is_recorded_once_and_only_once() {
        // The latch is the whole mechanism. Without it the figure would be
        // rewritten at the refresh rate and report the time between the last
        // two frames rather than the time to the first.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        app.started_at = Some(std::time::Instant::now());

        assert!(app.first_frame_at.is_none(), "nothing drawn yet");

        let _ = app.update(Message::FrameDrawn);
        let first = app.first_frame_at.expect("the first frame was recorded");

        for _ in 0..5 {
            let _ = app.update(Message::FrameDrawn);
        }
        assert_eq!(
            app.first_frame_at,
            Some(first),
            "a later frame must not overwrite the first"
        );
    }

    // NOT TESTED, AND RECORDED RATHER THAN FAKED: that `subscription` stops
    // asking for frames once the first has arrived.
    //
    // `window::frames()` yields at the refresh rate, so leaving it subscribed
    // would push a message sixty times a second for the life of the process.
    // The guard in `subscription` is what prevents that, and removing it is a
    // mutation no test here catches: `iced::Subscription` is opaque -- its
    // `Debug` is the bare string "Subscription" -- so a test cannot see which
    // streams a batch contains. Asserting on a predicate extracted from the
    // guard would test the predicate, not the guard, and would pass with the
    // guard deleted.
    //
    // The cost of the miss is wasted work rather than wrong behaviour: the
    // latch below keeps a late frame from overwriting the figure either way.

    fn hidden_app() -> (tempfile::TempDir, LauncherApp) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        app.first_frame_at = Some(std::time::Instant::now());
        app.window = None;
        app.pending_window = None;
        (dir, app)
    }

    #[test]
    fn a_summon_is_timed_from_show_to_the_new_windows_first_frame() {
        let (_dir, mut app) = hidden_app();
        let _ = app.update(Message::Command(UiCommand::Show));
        let pending = app.pending_window.expect("Show opened a window");

        // Nothing drawn yet: the window has not opened.
        let _ = app.update(Message::FrameDrawn);
        assert!(
            app.last_summon_draw.is_none(),
            "a frame before Opened is not the summoned one"
        );

        let _ = app.update(Message::Opened(pending));
        let _ = app.update(Message::FrameDrawn);
        let first = app.last_summon_draw.expect("the summon was timed");
        assert!(app.summoned_at.is_none(), "the latch closed");

        let _ = app.update(Message::FrameDrawn);
        assert_eq!(
            app.last_summon_draw,
            Some(first),
            "a later frame does not re-time it"
        );
    }

    #[test]
    fn show_on_a_visible_window_and_hide_time_nothing() {
        let (_dir, mut app) = hidden_app();
        let _ = app.update(Message::Command(UiCommand::Show));
        let pending = app.pending_window.expect("opened");
        let _ = app.update(Message::Opened(pending));
        let _ = app.update(Message::FrameDrawn);
        let first = app.last_summon_draw.expect("timed");

        // Already on screen: raising it is not a summon.
        let _ = app.update(Message::Command(UiCommand::Show));
        assert!(
            app.summoned_at.is_none(),
            "a visible window is not summoned"
        );

        // Hide cancels a pending measurement rather than leaving it to be
        // closed by whatever frame comes next.
        app.window = None;
        let _ = app.update(Message::Command(UiCommand::Show));
        assert!(app.summoned_at.is_some());
        let _ = app.update(Message::Command(UiCommand::Hide));
        assert!(app.summoned_at.is_none(), "hide drops the pending summon");
        assert_eq!(app.last_summon_draw, Some(first));
    }

    #[test]
    fn without_a_start_time_nothing_is_claimed() {
        // A test, or any caller that is not timing, sets no start. The frame
        // is still latched -- that is what stops the subscription -- but no
        // figure is logged, because there would be nothing to measure from.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        assert!(app.started_at.is_none(), "precondition");

        let _ = app.update(Message::FrameDrawn);
        assert!(
            app.first_frame_at.is_some(),
            "the latch still closes, or the subscription would never stop"
        );
    }
}
