//! The flags the launcher starts with, and copying them onto its state.

use super::icons::IconLookup;
use super::{
    AppLauncher, Appearance, Arc, ClockSettings, EngineLink, GEOMETRY, LauncherApp, NullLauncher,
    design, window,
};

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

impl LauncherApp {
    /// Copy the flags onto an already-built state.
    ///
    /// Split out of [`LauncherApp::new`] so the copying is reachable without
    /// `AppIndex::from_environment`, which reads the invoking user's real
    /// application directories. A test that reproduced this assignment list
    /// instead of calling it would pass while the real one stopped copying a
    /// field -- which is exactly the mutation it is meant to catch.
    pub(super) fn apply(&mut self, flags: AppFlags) {
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
        app.symbolic_icons = compass_core::builtin_icon::symbolic_names()
            .iter()
            .filter_map(|&(builtin, symbolic)| {
                let path = flags.icon_lookup.find(symbolic)?;
                path.extension()
                    .is_some_and(|ext| ext == "svg")
                    .then_some((builtin, path))
            })
            .collect();
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{app, theme_name};
    use crate::message::Message;
    use compass_core::AppItem;

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
}

#[cfg(test)]
mod preset_tests {
    use super::*;
    use crate::preset;
    use crate::preset::Preset;
    use compass_core::AppIndex;

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
