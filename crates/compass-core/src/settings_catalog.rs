//! Every setting the settings view edits: where it lives in `compass.json`,
//! what it takes, what it is when unset, and which C++ setting it stands for.
//!
//! The C++ settings window reads and writes its values through
//! `GeneralSettingsModel`'s properties, the root item manager and each
//! builtin provider's `preferences()`. Compass keeps one table instead, so the
//! view, the engine that writes the file ([`apply`]) and the tests agree on
//! the keys. A key is always one the Rust engine reads; a C++ setting nothing
//! in Compass reads is in [`NOT_IN_COMPASS`] with the reason, rather than
//! written to a file that would then look as though it were honoured (the
//! same rule `config_migration` follows).

use serde_json::Value;

use crate::config::Config;

/// The settings window's own pages, in the C++ sidebar's order
/// (`SettingsSidebarModel::rebuildRows`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CorePage {
    /// General: the hotkey, closing, the results and the clock.
    General,
    /// Appearance: the theme, the font and the layout.
    Appearance,
    /// Keybindings: the launcher's keys.
    Keybindings,
    /// Advanced: navigation and the input server.
    Advanced,
    /// About: the version and the project's links.
    About,
}

impl CorePage {
    /// All five, in display order.
    pub const ALL: [Self; 5] = [
        Self::General,
        Self::Appearance,
        Self::Keybindings,
        Self::Advanced,
        Self::About,
    ];

    /// The page's id, as `openTab` and the sidebar key it.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Keybindings => "keybindings",
            Self::Advanced => "advanced",
            Self::About => "about",
        }
    }

    /// The page's title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Keybindings => "Keybindings",
            Self::Advanced => "Advanced",
            Self::About => "About",
        }
    }

    /// The page a tab id opens, as `SettingsWindow::openTab` reads it:
    /// `keybinds` and `shortcuts` are the keybindings page, `extensions` the
    /// general one. `None` for anything else, which may be a provider id.
    #[must_use]
    pub fn from_tab(tab: &str) -> Option<Self> {
        match tab {
            "keybinds" | "shortcuts" => Some(Self::Keybindings),
            "extensions" => Some(Self::General),
            _ => Self::ALL.into_iter().find(|page| page.id() == tab),
        }
    }
}

/// Which page a setting is drawn on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// One of the window's own pages.
    Core(CorePage),
    /// A provider's page, by provider id (`scripts`).
    Provider(&'static str),
    /// Under one root item on its provider's page, by its
    /// `provider:entrypoint` id (`commands:clipboard-history`).
    Command(String),
}

/// What a setting takes, and so how it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A switch; a JSON boolean.
    Toggle,
    /// One of a list, as `(value, label)`; a JSON string.
    Choice(Vec<(&'static str, &'static str)>),
    /// A whole number in `min..=max`; a JSON number.
    Number {
        /// The smallest accepted.
        min: u64,
        /// The largest accepted.
        max: u64,
    },
    /// A line of text; a JSON string.
    Text,
    /// A list of paths, separated by colons in the view; a JSON array of strings.
    Paths,
    /// A list of names (application ids), comma-separated in the view; a
    /// JSON array of strings.
    Names,
    /// A key combination, recorded; a JSON string in `KeyCombo`'s spelling.
    Shortcut,
    /// A theme's name, from the themes the view lists; a JSON string.
    Theme,
    /// A font family, from the installed ones; a JSON string.
    Font,
}

/// One setting.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    /// Its dotted path in `compass.json`.
    pub key: String,
    /// The page it is on.
    pub scope: Scope,
    /// The heading it is grouped under.
    pub section: &'static str,
    /// Its label.
    pub label: &'static str,
    /// The line under the label; may be empty.
    pub description: &'static str,
    /// What it takes.
    pub kind: Kind,
    /// What it is when the file does not set it.
    pub default: Value,
    /// What the field shows when the file does not set it and the default is
    /// not something to show (the file indexer's home directory); may be
    /// empty.
    pub placeholder: &'static str,
    /// The C++ setting it ports: a `GeneralSettingsModel` property, or
    /// `<provider>.<preference>`. `None` for a setting only Compass has.
    pub cpp: Option<&'static str>,
}

impl Setting {
    fn new(key: impl Into<String>, scope: Scope, section: &'static str) -> Self {
        Self {
            key: key.into(),
            scope,
            section,
            label: "",
            description: "",
            kind: Kind::Toggle,
            default: Value::Null,
            placeholder: "",
            cpp: None,
        }
    }

    fn label(mut self, label: &'static str, description: &'static str) -> Self {
        self.label = label;
        self.description = description;
        self
    }

    fn kind(mut self, kind: Kind, default: Value) -> Self {
        self.kind = kind;
        self.default = default;
        self
    }

    fn cpp(mut self, cpp: &'static str) -> Self {
        self.cpp = Some(cpp);
        self
    }

    fn placeholder(mut self, placeholder: &'static str) -> Self {
        self.placeholder = placeholder;
        self
    }
}

/// A C++ setting Compass has no reader for, and so does not offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotPorted {
    /// The `GeneralSettingsModel` property (or window feature).
    pub cpp: &'static str,
    /// The page the C++ draws it on.
    pub page: CorePage,
    /// Its C++ label.
    pub label: &'static str,
    /// Why Compass leaves it out.
    pub reason: &'static str,
}

const LAUNCHER_DRAWS: &str = "Compass does not offer this yet";

/// The C++ settings with no counterpart in Compass, each with why. The view
/// lists them at the foot of their page; PARITY.md declares them.
pub const NOT_IN_COMPASS: &[NotPorted] = &[
    NotPorted {
        cpp: "closeOnEscape",
        page: CorePage::General,
        label: "Close on Escape",
        reason: "Escape always goes back one view, clears the search, then hides the launcher",
    },
    NotPorted {
        cpp: "language",
        page: CorePage::General,
        label: "Language",
        reason: "Compass is not translated yet",
    },
    NotPorted {
        cpp: "telemetrySystemInfo",
        page: CorePage::General,
        label: "Basic usage statistics",
        reason: "Compass sends no usage statistics",
    },
    NotPorted {
        cpp: "fontSize",
        page: CorePage::Appearance,
        label: "Font size",
        reason: "the layout preset sets the text size",
    },
    NotPorted {
        cpp: "iconTheme",
        page: CorePage::Appearance,
        label: "Icon theme",
        reason: "icons follow the desktop's icon theme",
    },
    NotPorted {
        cpp: "windowMaterial",
        page: CorePage::Appearance,
        label: "Window material",
        reason: "Compass cannot blur the desktop behind it yet; see Translucent background",
    },
    NotPorted {
        cpp: "windowOpacity",
        page: CorePage::Appearance,
        label: "Window opacity",
        reason: "see Translucent background",
    },
    NotPorted {
        cpp: "compactMode",
        page: CorePage::Appearance,
        label: "Compact mode",
        reason: LAUNCHER_DRAWS,
    },
    NotPorted {
        cpp: "floatingStatusBar",
        page: CorePage::Appearance,
        label: "Floating status bar",
        reason: LAUNCHER_DRAWS,
    },
    NotPorted {
        cpp: "layerShellEnabled",
        page: CorePage::Appearance,
        label: "Use layer shell",
        reason: "Compass picks the kind of window that suits your desktop",
    },
    NotPorted {
        cpp: "clientSideDecorations",
        page: CorePage::Appearance,
        label: "Client-side decorations",
        reason: "the layout preset draws the window's border and corners",
    },
    NotPorted {
        cpp: "rounding",
        page: CorePage::Appearance,
        label: "Corner rounding",
        reason: "the layout preset sets the corner radius",
    },
    NotPorted {
        cpp: "csdBorderWidth",
        page: CorePage::Appearance,
        label: "Border width",
        reason: "the layout preset sets the border",
    },
    NotPorted {
        cpp: "csdShadowSize",
        page: CorePage::Appearance,
        label: "Shadow size",
        reason: "the compositor draws the shadow",
    },
    NotPorted {
        cpp: "nativeTextRendering",
        page: CorePage::Appearance,
        label: "Native font rendering",
        reason: "Compass draws text the same way on every desktop",
    },
    NotPorted {
        cpp: "keybinds",
        page: CorePage::Keybindings,
        label: "Custom keybindings",
        reason: "the launcher's action keys are fixed; the navigation scheme is under Advanced",
    },
    NotPorted {
        cpp: "popOnBackspace",
        page: CorePage::Advanced,
        label: "Pop on backspace",
        reason: LAUNCHER_DRAWS,
    },
    NotPorted {
        cpp: "activateOnSingleClick",
        page: CorePage::Advanced,
        label: "Activate on single click",
        reason: "a click always activates",
    },
    NotPorted {
        cpp: "considerPreedit",
        page: CorePage::Advanced,
        label: "IME handling",
        reason: "Compass cannot search text an input method is still composing",
    },
    NotPorted {
        cpp: "searchFilesInRoot",
        page: CorePage::Advanced,
        label: "Root file search",
        reason: "files are searched through the Search Files fallback",
    },
    NotPorted {
        cpp: "faviconService",
        page: CorePage::Advanced,
        label: "Favicon fetching",
        reason: "Compass fetches no favicons yet",
    },
    NotPorted {
        cpp: "encryptSensitiveData",
        page: CorePage::Advanced,
        label: "Encrypt sensitive data",
        reason: "Compass always encrypts what it keeps, with a key from the keyring",
    },
];

/// The launcher's keys, as the Keybindings page lists them: `(name,
/// description, keys)`. The C++ makes these rebindable (`keybinds`);
/// Compass's are fixed (see [`NOT_IN_COMPASS`]).
pub const KEYBINDINGS: &[(&str, &str, &str)] = &[
    (
        "Toggle action panel",
        "Open the actions for the selected item, and filter them",
        "Ctrl+B",
    ),
    (
        "Run the default action",
        "Open, launch or paste the selected item",
        "Enter",
    ),
    (
        "Go back",
        "Leave the view; at the root, clear the search, then hide the launcher",
        "Escape",
    ),
    (
        "Quick launch",
        "Open one of the first nine results",
        "Ctrl+1 … Ctrl+9",
    ),
    (
        "Open settings",
        "Open these settings from the launcher",
        "Ctrl+,",
    ),
];

fn core(page: CorePage) -> Scope {
    Scope::Core(page)
}

fn choice(options: &[(&'static str, &'static str)]) -> Kind {
    Kind::Choice(options.to_vec())
}

/// Every setting, pages in order and each page in the order it is drawn.
#[must_use]
// Over the 150-line limit when it was added (#379): split it, don't grow it.
#[allow(clippy::too_many_lines)]
pub fn catalog() -> Vec<Setting> {
    use CorePage::{Advanced, Appearance, General};
    use serde_json::json;

    let mut settings = vec![
        Setting::new("launcher.hotkey", core(General), "Behavior")
            .label(
                "Launcher hotkey",
                "Global shortcut to toggle the launcher.",
            )
            .kind(Kind::Shortcut, json!(crate::config::DEFAULT_HOTKEY))
            .cpp("toggleShortcut"),
        Setting::new("global_shortcuts.inhibit_apps", core(General), "Behavior")
            .label(
                "Pause shortcuts in",
                "While one of these applications is focused, global shortcuts are released so its keys reach it.",
            )
            .kind(Kind::Names, json!([]))
            .placeholder("org.gnome.Boxes.desktop"),
        Setting::new("launcher.close_on_focus_loss", core(General), "Behavior")
            .label("Close on focus loss", "")
            .kind(
                Kind::Toggle,
                json!(crate::config::DEFAULT_CLOSE_ON_FOCUS_LOSS),
            )
            .cpp("closeOnFocusLoss"),
        Setting::new("launcher.pop_to_root_on_close", core(General), "Behavior")
            .label(
                "Clear the search on close",
                "Open the launcher with an empty search every time.",
            )
            .kind(
                Kind::Toggle,
                json!(crate::config::DEFAULT_POP_TO_ROOT_ON_CLOSE),
            )
            .cpp("popToRootOnClose"),
        Setting::new("launcher.quick_launch", core(General), "Behavior")
            .label(
                "Quick launch",
                "Ctrl+1 to Ctrl+9 open the first nine results.",
            )
            .kind(Kind::Toggle, json!(crate::config::DEFAULT_QUICK_LAUNCH)),
        Setting::new("launcher.max_results", core(General), "Behavior")
            .label("Results shown", "How many results the root search lists.")
            .kind(
                Kind::Number { min: 0, max: 500 },
                json!(crate::config::DEFAULT_MAX_RESULTS),
            ),
        Setting::new("launcher.check_for_updates", core(General), "Updates")
            .label(
                "Check for updates",
                "Ask Compass's GitHub releases, at most every six hours, whether a newer version is out.",
            )
            .kind(Kind::Toggle, json!(crate::config::DEFAULT_CHECK_FOR_UPDATES)),
        Setting::new("launcher.clock.enabled", core(General), "Clock")
            .label("Show the clock", "The time, in the root search's status bar.")
            .kind(Kind::Toggle, json!(crate::config::DEFAULT_CLOCK_ENABLED)),
        Setting::new("launcher.clock.format", core(General), "Clock")
            .label(
                "Clock format",
                "Such as hh:mm for 14:05, or h:mm ap for 2:05 pm.",
            )
            .kind(Kind::Text, json!(crate::config::DEFAULT_CLOCK_FORMAT)),
        Setting::new("launcher.clock.interval", core(General), "Clock")
            .label("Clock refresh", "How often the clock is redrawn, in seconds.")
            .kind(
                Kind::Number { min: 1, max: 3600 },
                json!(crate::config::DEFAULT_CLOCK_INTERVAL),
            ),
        Setting::new("launcher.appearance.theme", core(Appearance), "Theme")
            .label("Theme", "")
            .kind(Kind::Theme, json!(crate::config::DEFAULT_THEME))
            .cpp("theme"),
        Setting::new("launcher.appearance.color_scheme", core(Appearance), "Theme")
            .label(
                "Color scheme",
                "Light or dark; System follows the desktop.",
            )
            .kind(
                choice(&[("system", "System"), ("light", "Light"), ("dark", "Dark")]),
                json!(crate::config::DEFAULT_COLOR_SCHEME),
            ),
        Setting::new("font.normal.family", core(Appearance), "Theme")
            .label("Font", "")
            .kind(Kind::Font, json!("auto"))
            .placeholder("The desktop's interface font")
            .cpp("font"),
        Setting::new("launcher.appearance.preset", core(Appearance), "Layout")
            .label(
                "Layout",
                "The launcher's shape; the switches below override it.",
            )
            .kind(
                choice(&[
                    ("gnome", "GNOME"),
                    ("raycast", "Raycast"),
                    ("flow", "Flow"),
                    ("rofi", "Rofi"),
                ]),
                json!(crate::config::DEFAULT_PRESET),
            ),
        Setting::new("launcher.appearance.icons", core(Appearance), "Layout")
            .label("Application icons", "Show each result's icon.")
            .kind(Kind::Toggle, json!(crate::config::DEFAULT_ICONS)),
        Setting::new("launcher.appearance.tint", core(Appearance), "Layout")
            .label(
                "Translucent background",
                "Let the desktop show through the launcher.",
            )
            .kind(Kind::Toggle, json!(crate::config::DEFAULT_TINT)),
        Setting::new("launcher.wrap_navigation", core(Advanced), "Input & Navigation")
            .label(
                "Wrap navigation",
                "Wrap around to the opposite end when moving past the first or last item.",
            )
            .kind(Kind::Toggle, json!(crate::config::DEFAULT_WRAP_NAVIGATION))
            .cpp("wrapNavigation"),
        Setting::new("launcher.keybinding", core(Advanced), "Input & Navigation")
            .label(
                "Keybinding Scheme",
                "Default and Vim use Ctrl+J/K and Ctrl+H/L; Emacs uses Ctrl+N/P and Ctrl+Alt+B/F for navigation.",
            )
            .kind(
                choice(&[("default", "Default"), ("vim", "Vim"), ("emacs", "Emacs")]),
                json!(crate::config::DEFAULT_KEYBINDING),
            )
            .cpp("keybindingScheme"),
        Setting::new("input_server.enabled", core(Advanced), "System")
            .label(
                "Input server",
                "Run the input server, which snippets and pasting into the active window need.",
            )
            .kind(
                Kind::Toggle,
                json!(crate::config::DEFAULT_INPUT_SERVER_ENABLED),
            )
            .cpp("inputServerEnabled"),
        Setting::new("tray.enabled", core(Advanced), "System")
            .label(
                "Tray icon",
                "Show Compass in the tray, with a menu to toggle the launcher, open settings and quit.",
            )
            .kind(Kind::Toggle, json!(crate::config::DEFAULT_TRAY_ENABLED))
            .cpp("trayEnabled"),
    ];

    let clipboard = || Scope::Command("commands:clipboard-history".to_owned());
    let clipboard_key = |name: &str| format!("providers.clipboard.preferences.{name}");
    settings.extend([
        Setting::new(clipboard_key("monitoring"), clipboard(), "Clipboard")
            .label(
                "Clipboard monitoring",
                "Whether new clipboard selections are appended to the history",
            )
            .kind(Kind::Toggle, json!(true))
            .cpp("clipboard.monitoring"),
        Setting::new(clipboard_key("evictionThreshold"), clipboard(), "Clipboard")
            .label(
                "Eviction threshold",
                "Automatically delete selections older than this threshold",
            )
            .kind(
                choice(&[
                    ("never", "Never"),
                    ("900", "15 minutes"),
                    ("3600", "1 hour"),
                    ("86400", "1 day"),
                    ("604800", "1 week"),
                    ("2592000", "1 month"),
                    ("31536000", "1 year"),
                ]),
                json!("never"),
            )
            .cpp("clipboard.evictionThreshold"),
        Setting::new(clipboard_key("preserveTagged"), clipboard(), "Clipboard")
            .label(
                "Preserve tagged",
                "Never evict or mass delete selections that have been explicitly tagged (pinned, custom keyword)",
            )
            .kind(Kind::Toggle, json!(true))
            .cpp("clipboard.preserveTagged"),
        Setting::new(clipboard_key("eraseOnStartup"), clipboard(), "Clipboard")
            .label(
                "Erase on startup",
                "Erase clipboard history every time the engine is started",
            )
            .kind(Kind::Toggle, json!(false))
            .cpp("clipboard.eraseOnStartup"),
        Setting::new(clipboard_key("ignorePasswords"), clipboard(), "Clipboard")
            .label(
                "Ignore Passwords",
                "Ignore selections that can be identified as a password. May not work with all apps.",
            )
            .kind(Kind::Toggle, json!(true))
            .cpp("clipboard.ignorePasswords"),
    ]);

    let files = || Scope::Command("commands:search-files".to_owned());
    let files_key = |name: &str| {
        format!(
            "providers.{}.preferences.{name}",
            crate::file_search::PREFERENCES_PROVIDER_ID
        )
    };
    settings.extend([
        Setting::new(files_key("autoIndexing"), files(), "File index")
            .label(
                "Automatic indexing",
                "Keep an index of your files for Search Files.",
            )
            .kind(Kind::Toggle, json!(true))
            .cpp("files.autoIndexing"),
        Setting::new(files_key("indexingPaths"), files(), "File index")
            .label("Indexed folders", "Separate folders with a colon (:).")
            .kind(Kind::Paths, json!([]))
            .placeholder("Your home folder")
            .cpp("files.indexingPaths"),
        Setting::new(files_key("excludedIndexingPaths"), files(), "File index")
            .label(
                "Excluded folders",
                "Never indexed. Separate folders with a colon (:).",
            )
            .kind(Kind::Paths, json!([]))
            .cpp("files.excludedIndexingPaths"),
    ]);

    let snippets = || Scope::Command("commands:manage-snippets".to_owned());
    let snippets_key = |name: &str| {
        format!(
            "providers.{}.preferences.{name}",
            crate::input_server::expansion::PROVIDER_ID
        )
    };
    settings.extend([
        Setting::new(snippets_key("enabled"), snippets(), "Snippet expansion")
            .label(
                "Expand keywords",
                "Replace a snippet's keyword with the snippet as you type it anywhere.",
            )
            .kind(Kind::Toggle, json!(true))
            .cpp("snippets.enabled"),
        Setting::new(snippets_key("undo"), snippets(), "Snippet expansion")
            .label(
                "Undo with Backspace",
                "Backspace right after an expansion puts the keyword back.",
            )
            .kind(Kind::Toggle, json!(true))
            .cpp("snippets.undo"),
        Setting::new(
            snippets_key("prePasteDelay"),
            snippets(),
            "Snippet expansion",
        )
        .label("Delay before pasting", "In milliseconds, from 0 to 5000.")
        .kind(Kind::Text, json!("0"))
        .cpp("snippets.prePasteDelay"),
        Setting::new(snippets_key("keyDelay"), snippets(), "Snippet expansion")
            .label("Delay between keys", "In milliseconds, from 0 to 50.")
            .kind(Kind::Text, json!("2"))
            .cpp("snippets.keyDelay"),
        Setting::new(snippets_key("layout"), snippets(), "Snippet expansion")
            .label(
                "Keyboard layout",
                "The XKB layout keywords are typed in; empty for the system's.",
            )
            .kind(Kind::Text, json!(""))
            .cpp("snippets.layout"),
    ]);

    settings.push(
        Setting::new(
            "providers.scripts.preferences.customDirs",
            Scope::Provider(crate::script_scan::SCRIPTS_PROVIDER_ID),
            "Script Commands",
        )
        .label(
            "Script directories",
            "Searched before the default ones. Separate folders with a colon (:).",
        )
        .kind(Kind::Paths, json!([]))
        .cpp("scripts.customDirs"),
    );

    let browse_apps = || {
        Scope::Command(crate::root_items::entrypoint_id(
            crate::commands::COMMANDS_PROVIDER_ID,
            crate::browse_apps::ENTRYPOINT,
        ))
    };
    let browse_key = |name: &str| {
        format!(
            "providers.{}.entrypoints.{}.preferences.{name}",
            crate::commands::COMMANDS_PROVIDER_ID,
            crate::browse_apps::ENTRYPOINT
        )
    };
    settings.extend([
        Setting::new(
            browse_key(crate::browse_apps::SHOW_HIDDEN),
            browse_apps(),
            "Browse Apps",
        )
        .label(
            "Show hidden apps",
            "List the applications menus leave out too.",
        )
        .kind(Kind::Toggle, json!(false))
        .cpp("browse-apps.showHidden"),
        Setting::new(
            browse_key(crate::browse_apps::SORT_ALPHABETICALLY),
            browse_apps(),
            "Browse Apps",
        )
        .label("Sort alphabetically", "")
        .kind(Kind::Toggle, json!(true))
        .cpp("browse-apps.sortAlphabetically"),
        Setting::new(
            "providers.commands.entrypoints.run-program.preferences.default-action",
            Scope::Command("commands:run-program".to_owned()),
            "Run Terminal Program",
        )
        .label("Default action", "What Enter does with a program.")
        .kind(
            choice(&[("run-in-terminal", "Run in terminal"), ("run", "Run")]),
            json!("run-in-terminal"),
        )
        .cpp("run-program.default-action"),
    ]);

    let emoji = || Scope::Command("commands:search-emojis".to_owned());
    let emoji_key =
        |name: &str| format!("providers.core.entrypoints.search-emojis.preferences.{name}");
    settings.extend([
        Setting::new(emoji_key("skinTone"), emoji(), "Search Emojis")
            .label("Skin tone", "Applied to the emojis that have one.")
            .kind(
                Kind::Choice(
                    crate::emoji_grid::SKIN_TONES
                        .iter()
                        .map(|tone| (tone.id, tone.display_name))
                        .collect(),
                ),
                json!("default"),
            )
            .cpp("search-emojis.skinTone"),
        Setting::new(emoji_key("defaultAction"), emoji(), "Search Emojis")
            .label("Default action", "What Enter does with an emoji.")
            .kind(
                choice(&[("paste", "Paste"), ("copy", "Copy")]),
                json!(crate::emoji_grid::DEFAULT_ACTION_PASTE),
            )
            .cpp("search-emojis.defaultAction"),
    ]);

    for command in crate::power_commands::COMMANDS {
        let scope = || {
            Scope::Command(crate::root_items::entrypoint_id(
                crate::commands::COMMANDS_PROVIDER_ID,
                command.id,
            ))
        };
        let key = |name: &str| {
            format!(
                "providers.{}.entrypoints.{}.preferences.{name}",
                crate::power_commands::EXTENSION_ID,
                command.id
            )
        };
        settings.push(
            Setting::new(
                key(crate::power_commands::CONFIRM_PREFERENCE),
                scope(),
                command.name,
            )
            .label("Confirm", "Ask before running it.")
            .kind(Kind::Toggle, json!(command.confirm_by_default))
            .cpp("power.confirm"),
        );
        settings.push(
            Setting::new(
                key(crate::power_commands::CUSTOM_PROGRAM_PREFERENCE),
                scope(),
                command.name,
            )
            .label(
                crate::power_commands::CUSTOM_PROGRAM_TITLE,
                crate::power_commands::CUSTOM_PROGRAM_DESCRIPTION,
            )
            .kind(Kind::Text, json!(""))
            .cpp("power.customProgram"),
        );
    }
    settings
}

/// The setting `key` names, if it is one.
#[must_use]
pub fn find(key: &str) -> Option<Setting> {
    catalog().into_iter().find(|setting| setting.key == key)
}

/// What the setting is now: the file's value, or its default.
#[must_use]
pub fn value(config: &Config, setting: &Setting) -> Value {
    config
        .get_path(&setting.key)
        .filter(|value| !value.is_null())
        .unwrap_or_else(|| setting.default.clone())
}

/// Why a value was refused.
fn refuse(setting: &Setting, wanted: &str) -> String {
    format!("{} takes {wanted}", setting.label)
}

/// Checks `value` against the setting and returns what to write: `None` to
/// take the key out of the file (a JSON `null`, or an empty shortcut).
///
/// # Errors
///
/// The sentence to show when the value is not one the setting takes.
pub fn validate(setting: &Setting, value: Value) -> Result<Option<Value>, String> {
    if value.is_null() {
        return Ok(None);
    }
    match &setting.kind {
        Kind::Toggle => value
            .is_boolean()
            .then_some(Some(value))
            .ok_or_else(|| refuse(setting, "on or off")),
        Kind::Choice(options) => match value.as_str() {
            Some(chosen) if options.iter().any(|(option, _)| *option == chosen) => Ok(Some(value)),
            _ => Err(refuse(
                setting,
                &format!(
                    "one of {}",
                    options
                        .iter()
                        .map(|(option, _)| *option)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
        },
        Kind::Number { min, max } => match value.as_u64() {
            Some(number) if (*min..=*max).contains(&number) => Ok(Some(value)),
            _ => Err(refuse(
                setting,
                &format!("a whole number from {min} to {max}"),
            )),
        },
        Kind::Text => value
            .is_string()
            .then_some(Some(value))
            .ok_or_else(|| refuse(setting, "text")),
        Kind::Paths => match value.as_array() {
            Some(items) if items.iter().all(Value::is_string) => {
                let kept: Vec<Value> = items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(|path| Value::String(path.to_owned()))
                    .collect();
                Ok(Some(Value::Array(kept)))
            }
            _ => Err(refuse(setting, "a list of folders")),
        },
        Kind::Names => match value.as_array() {
            Some(items) if items.iter().all(Value::is_string) => {
                let kept: Vec<Value> = items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| Value::String(name.to_owned()))
                    .collect();
                Ok(Some(Value::Array(kept)))
            }
            _ => Err(refuse(setting, "a list of application ids")),
        },
        Kind::Shortcut => match value.as_str().map(str::trim) {
            Some("") => Ok(None),
            Some(shortcut) if crate::key_combo::KeyCombo::parse(shortcut).is_some() => {
                Ok(Some(Value::String(shortcut.to_owned())))
            }
            _ => Err(refuse(setting, "a key combination such as super+space")),
        },
        Kind::Theme | Kind::Font => match value.as_str().map(str::trim) {
            Some(name) if !name.is_empty() => Ok(Some(Value::String(name.to_owned()))),
            _ => Err(refuse(setting, "a name")),
        },
    }
}

/// Writes `value` for the setting `key` into `config`, as the settings
/// window's controls do; a JSON `null` resets it to its default by taking
/// the key out.
///
/// # Errors
///
/// The sentence to show when `key` is not a setting or `value` is not one it
/// takes.
pub fn apply(config: &mut Config, key: &str, value: Value) -> Result<(), String> {
    let setting = find(key).ok_or_else(|| format!("{key:?} is not a setting"))?;
    let value = validate(&setting, value)?;
    config.set_path(&setting.key, value)
}

/// The tab a `compass://settings/open` deeplink (or `vicinae://`) names (`?tab=`), as
/// `IpcCommandHandler` reads the `settings` command: `Some(None)` opens the
/// window where it was, `None` is not a settings link.
#[must_use]
pub fn parse_settings_link(link: &str) -> Option<Option<String>> {
    let url = url::Url::parse(link).ok()?;
    if !matches!(url.scheme(), "compass" | "vicinae")
        || url.host_str() != Some("settings")
        || !matches!(url.path(), "" | "/" | "/open")
    {
        return None;
    }
    let tab = url
        .query_pairs()
        .find(|(name, _)| name == "tab")
        .map(|(_, tab)| tab.into_owned())
        .filter(|tab| !tab.is_empty());
    Some(tab)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(text: &str) -> Config {
        Config::parse(text, std::path::Path::new("compass.json")).expect("valid")
    }

    #[test]
    fn keys_are_unique() {
        let settings = catalog();
        let mut keys: Vec<&str> = settings.iter().map(|s| s.key.as_str()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "a key is listed twice");
    }

    #[test]
    fn every_default_is_a_value_its_setting_takes() {
        for setting in catalog() {
            assert!(
                validate(&setting, setting.default.clone()).is_ok(),
                "{}'s default is refused",
                setting.key
            );
        }
    }

    #[test]
    fn a_default_the_schema_documents_is_the_same_here() {
        let document = crate::config::default_document();
        for setting in catalog() {
            let documented = setting
                .key
                .split('.')
                .try_fold(&document, |node, segment| node.get(segment));
            if let Some(documented) = documented {
                assert_eq!(
                    documented, &setting.default,
                    "{} disagrees with the schema",
                    setting.key
                );
            }
        }
    }

    #[test]
    fn a_setting_is_read_from_the_file_or_else_its_default() {
        let config = config(r#"{"launcher": {"wrap_navigation": true}}"#);
        let wrap = find("launcher.wrap_navigation").unwrap();
        assert_eq!(value(&config, &wrap), json!(true));
        let keys = find("launcher.keybinding").unwrap();
        assert_eq!(value(&config, &keys), json!("default"));
    }

    #[test]
    fn applying_writes_the_key_the_engine_reads_and_keeps_the_rest() {
        let mut config = config(
            r#"{"launcher": {"hotkey": "alt+space"}, "font": {"normal": {"size": 11}}, "mystery": 1}"#,
        );
        apply(&mut config, "launcher.wrap_navigation", json!(true)).unwrap();
        apply(&mut config, "launcher.keybinding", json!("emacs")).unwrap();
        apply(&mut config, "input_server.enabled", json!(false)).unwrap();
        apply(&mut config, "tray.enabled", json!(false)).unwrap();
        apply(&mut config, "launcher.appearance.tint", json!(true)).unwrap();
        apply(&mut config, "font.normal.family", json!("Inter")).unwrap();
        apply(
            &mut config,
            "providers.clipboard.preferences.evictionThreshold",
            json!("3600"),
        )
        .unwrap();
        assert!(config.launcher().wrap_navigation());
        assert_eq!(
            config.launcher().keybinding_scheme(),
            crate::keybinding::Scheme::Emacs
        );
        assert!(!config.input_server().enabled());
        assert!(!config.tray().enabled());
        assert!(config.launcher().appearance().tint());
        assert_eq!(config.font_family(), Some("Inter"));
        assert_eq!(config.launcher().hotkey(), "alt+space");
        assert_eq!(config.get_path("font.normal.size"), Some(json!(11)));
        assert_eq!(config.get_path("mystery"), Some(json!(1)));
        assert_eq!(
            config
                .provider_preferences("clipboard")
                .and_then(|p| p.get("evictionThreshold")),
            Some(&json!("3600"))
        );
    }

    #[test]
    fn a_value_the_setting_does_not_take_is_refused_and_nothing_changes() {
        let mut config = Config::default();
        assert!(apply(&mut config, "launcher.keybinding", json!("qwerty")).is_err());
        assert!(apply(&mut config, "launcher.wrap_navigation", json!("yes")).is_err());
        assert!(apply(&mut config, "launcher.max_results", json!(9000)).is_err());
        assert!(apply(&mut config, "launcher.hotkey", json!("not a key")).is_err());
        assert!(apply(&mut config, "launcher.nonsense", json!(true)).is_err());
        assert_eq!(config, Config::default());
    }

    #[test]
    fn null_resets_and_an_empty_shortcut_clears_the_hotkey() {
        let mut config =
            config(r#"{"launcher": {"hotkey": "alt+space", "wrap_navigation": true}}"#);
        apply(&mut config, "launcher.wrap_navigation", Value::Null).unwrap();
        apply(&mut config, "launcher.hotkey", json!("")).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn a_power_commands_confirm_default_is_its_own() {
        let power_off = find("providers.power.entrypoints.power-off.preferences.confirm").unwrap();
        let lock = find("providers.power.entrypoints.lock.preferences.confirm").unwrap();
        assert_eq!(power_off.default, json!(true));
        assert_eq!(lock.default, json!(false));
        let mut config = Config::default();
        apply(&mut config, &lock.key, json!(true)).unwrap();
        let command = crate::power_commands::command("lock").unwrap();
        assert!(crate::power_commands::should_confirm(
            command,
            config.entrypoint_preferences("power", "lock")
        ));
    }

    #[test]
    fn paths_are_trimmed_and_blank_lines_dropped() {
        let mut config = Config::default();
        apply(
            &mut config,
            "providers.files.preferences.indexingPaths",
            json!([" /home/u/docs ", "", "/srv"]),
        )
        .unwrap();
        let settings = crate::file_search::IndexingSettings::from_preferences(
            config.provider_preferences("files"),
            None,
        );
        assert_eq!(settings.paths, ["/home/u/docs", "/srv"]);
    }

    #[test]
    fn the_apps_that_pause_the_shortcuts_are_names_the_engine_reads() {
        let mut config = Config::default();
        apply(
            &mut config,
            "global_shortcuts.inhibit_apps",
            json!([" org.gnome.Boxes.desktop ", "", "steam.desktop"]),
        )
        .unwrap();
        assert_eq!(
            config.global_shortcuts().inhibit_apps(),
            ["org.gnome.Boxes.desktop", "steam.desktop"]
        );
        assert!(apply(&mut config, "global_shortcuts.inhibit_apps", json!("steam")).is_err());
        apply(&mut config, "global_shortcuts.inhibit_apps", Value::Null).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn every_cpp_general_settings_property_is_ported_or_declared() {
        // `GeneralSettingsModel`'s Q_PROPERTYs and selectors, and the
        // keybindings page's `keybinds`.
        const CPP: &[&str] = &[
            "searchFilesInRoot",
            "inputServerEnabled",
            "trayEnabled",
            "closeOnFocusLoss",
            "closeOnEscape",
            "considerPreedit",
            "popToRootOnClose",
            "popOnBackspace",
            "activateOnSingleClick",
            "wrapNavigation",
            "encryptSensitiveData",
            "telemetrySystemInfo",
            "layerShellEnabled",
            "clientSideDecorations",
            "rounding",
            "csdBorderWidth",
            "csdShadowSize",
            "compactMode",
            "floatingStatusBar",
            "windowOpacity",
            "nativeTextRendering",
            "fontSize",
            "windowMaterial",
            "theme",
            "font",
            "iconTheme",
            "faviconService",
            "keybindingScheme",
            "language",
            "toggleShortcut",
            "keybinds",
        ];
        let settings = catalog();
        for property in CPP {
            let ported = settings.iter().any(|s| s.cpp == Some(*property));
            let declared = NOT_IN_COMPASS.iter().any(|n| n.cpp == *property);
            assert!(
                ported ^ declared,
                "{property} must be exactly one of ported or declared"
            );
        }
    }

    #[test]
    fn a_settings_deeplink_names_its_tab() {
        assert_eq!(parse_settings_link("vicinae://settings"), Some(None));
        assert_eq!(parse_settings_link("compass://settings/open"), Some(None));
        assert_eq!(parse_settings_link("raycast://settings/open"), None);
        assert_eq!(parse_settings_link("vicinae://settings/open"), Some(None));
        assert_eq!(
            parse_settings_link("vicinae://settings/open?tab=about"),
            Some(Some("about".to_owned()))
        );
        assert_eq!(parse_settings_link("vicinae://settings/close"), None);
        assert_eq!(parse_settings_link("vicinae://launch/x"), None);
        assert_eq!(CorePage::from_tab("shortcuts"), Some(CorePage::Keybindings));
        assert_eq!(CorePage::from_tab("extensions"), Some(CorePage::General));
        assert_eq!(CorePage::from_tab("clipboard"), None);
    }
}
