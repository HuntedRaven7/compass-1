//! The first-run flow: whether it is due, its steps, and the record that it
//! was finished.
//!
//! Ports `OnboardingWindow` (`ui/windows/onboarding-window.*`) and the step
//! logic of `OnboardingWindow.qml`. The C++ shows the flow at server start
//! when `onboarding.json` in its state directory records a version older than
//! [`VERSION`] (or nothing), and only in a build with `ENABLE_ONBOARDING`,
//! which is on by default. Compass reads and writes the same file, so a person
//! who finished it under either engine is not asked again; its switch is
//! [`DISABLE_ENV`], set by the harnesses that must see the bare launcher.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store_listing::Store;

/// `ONBOARDING_VERSION`: bumping it shows the flow again to everyone.
pub const VERSION: u32 = 1;

/// The state file's name, in `$XDG_STATE_HOME/compass`.
pub const FILE_NAME: &str = "onboarding.json";

/// Set (to anything but `0` or empty) to leave the flow out, as a build
/// without `ENABLE_ONBOARDING` does.
pub const DISABLE_ENV: &str = "COMPASS_NO_ONBOARDING";

/// The window's title and the first step's heading.
pub const TITLE: &str = "Welcome to Compass";

/// What `onboarding.json` holds (`OnboardingState`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    /// The version of the flow last finished; 0 for never.
    pub version: u32,
    /// When, as an ISO 8601 UTC time.
    pub completed_at: String,
}

/// `$XDG_STATE_HOME/compass/onboarding.json`, falling back to
/// `~/.local/state`, as `Omnicast::stateDir()`.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    Some(crate::xdg_dirs::state_dir()?.join(FILE_NAME))
}

/// The version recorded at `path`; 0 when there is no file or it cannot be
/// read, as `completedVersion`.
#[must_use]
pub fn completed_version(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<State>(&text).ok())
        .map_or(0, |state| state.version)
}

/// `OnboardingWindow::shouldShow`: the flow is due when what was finished is
/// older than [`VERSION`], unless `disabled`.
#[must_use]
pub fn should_show(path: &Path, disabled: bool) -> bool {
    !disabled && completed_version(path) < VERSION
}

/// Whether `config_home` holds a Vicinae setup that Compass carried over:
/// the `vicinae` directory (or the symlink the move leaves behind), or
/// Vicinae's `settings.json` in Compass's directory.
///
/// Someone coming from Vicinae already has a launcher set up the way they
/// like it, so the first-run flow is not shown to them.
#[must_use]
pub fn came_from_vicinae(config_home: &Path) -> bool {
    config_home
        .join(compass_xdg::brand::LEGACY_DIR_NAME)
        .symlink_metadata()
        .is_ok()
        || config_home
            .join(compass_xdg::brand::DIR_NAME)
            .join("settings.json")
            .is_file()
}

/// Whether [`DISABLE_ENV`]'s value turns the flow off.
#[must_use]
pub fn disabled_by(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty() && value != "0")
}

/// `markCompleted`: records [`VERSION`] as finished at `completed_at`,
/// creating the state directory if it is not there.
///
/// # Errors
///
/// When the directory cannot be created or the file written.
pub fn mark_completed(path: &Path, completed_at: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let state = State {
        version: VERSION,
        completed_at: completed_at.to_owned(),
    };
    let text = serde_json::to_string(&state).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// One step of the flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// "Welcome to Compass".
    Welcome,
    /// macOS's permissions; never offered on Linux.
    Permissions,
    /// "Make it your own": the theme and the global hotkey.
    Personalize,
    /// "Add extensions": [`RECOMMENDED_EXTENSIONS`], each with Install.
    Extensions,
    /// "Setup complete".
    Complete,
}

impl Step {
    /// The step's heading.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Welcome => TITLE,
            Self::Permissions => "Permissions",
            Self::Personalize => "Make it your own",
            Self::Extensions => "Add extensions",
            Self::Complete => "Setup complete",
        }
    }

    /// The line under the heading. `shortcuts` is whether the platform binds
    /// global shortcuts itself, which changes the last step's sentence.
    #[must_use]
    pub const fn subtitle(self, shortcuts: bool) -> &'static str {
        match self {
            Self::Welcome => "Let's set it up. It only takes a minute.",
            Self::Permissions => {
                "Compass needs additional permissions in order to make the best of your Mac."
            }
            Self::Personalize => "You will be able to change these settings later.",
            Self::Extensions => {
                "A few to start with. Find more in the Extension Store at any time."
            }
            Self::Complete if shortcuts => "Compass is ready. Open the launcher with:",
            Self::Complete => "Compass is ready. Finish opens the launcher.",
        }
    }
}

/// Where Continue took the flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// To the next step.
    Next,
    /// Past the last one: the flow is finished.
    Finish,
}

/// The flow's position, as `root.step` over `stepCount` steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    steps: Vec<Step>,
    current: usize,
}

impl Flow {
    /// The steps, with the permissions page only where the platform has
    /// permissions to ask for (`permissionsAvailable`, macOS).
    #[must_use]
    pub fn new(permissions: bool) -> Self {
        let steps = if permissions {
            vec![
                Step::Welcome,
                Step::Permissions,
                Step::Personalize,
                Step::Extensions,
                Step::Complete,
            ]
        } else {
            vec![
                Step::Welcome,
                Step::Personalize,
                Step::Extensions,
                Step::Complete,
            ]
        };
        Self { steps, current: 0 }
    }

    /// The step on screen.
    #[must_use]
    pub fn step(&self) -> Step {
        self.steps[self.current]
    }

    /// Its position.
    #[must_use]
    pub fn position(&self) -> usize {
        self.current
    }

    /// How many steps there are (the dots).
    #[must_use]
    pub fn count(&self) -> usize {
        self.steps.len()
    }

    /// Whether Back is shown.
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.current > 0
    }

    /// The primary button's label: Finish on the last step, else Continue.
    #[must_use]
    pub fn primary_label(&self) -> &'static str {
        if self.current + 1 == self.steps.len() {
            "Finish"
        } else {
            "Continue"
        }
    }

    /// `advance()`: the next step, or the end.
    pub fn advance(&mut self) -> Advance {
        if self.current + 1 == self.steps.len() {
            return Advance::Finish;
        }
        self.current += 1;
        Advance::Next
    }

    /// `goBack()`.
    pub fn back(&mut self) {
        self.current = self.current.saturating_sub(1);
    }

    /// A dot was clicked: straight to that step.
    pub fn jump(&mut self, position: usize) {
        if position < self.steps.len() {
            self.current = position;
        }
    }
}

/// The documentation the hotkey row links to: the getting-started guide's
/// "Set a keyboard shortcut" section, which `docs/getting-started.md` in this
/// repository becomes on tunaos.org.
pub const HOTKEY_DOCS_URL: &str =
    "https://tunaos.org/docs/compass/getting-started#set-a-keyboard-shortcut";
/// The last step's GitHub button.
pub const GITHUB_URL: &str = crate::tray::PROJECT_URL;

/// A store extension the extensions step recommends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recommendation {
    /// The store it comes from.
    pub store: Store,
    /// The handle that store files it under (its owner's).
    pub owner: &'static str,
    /// Its name in that store.
    pub name: &'static str,
    /// Its title, as the store lists it.
    pub title: &'static str,
    /// One line on what it does.
    pub description: &'static str,
}

impl Recommendation {
    /// The id it installs under, which is how an installed copy is found.
    #[must_use]
    pub fn id(&self) -> String {
        self.store.extension_id(self.name)
    }
}

/// The extensions the extensions step offers.
///
/// Each one is general enough for a new user on any Linux desktop, needs no
/// account or API key, and Suite 1 (`scripts/suite1/expected.json`) shows its
/// first command rendering. The Raycast ones come first, because running
/// Raycast store extensions on Linux is what Compass adds; none of them runs
/// AppleScript or needs Homebrew, so they work without the runtime's shim
/// having to stand in for macOS. None talks to a local daemon over a unix
/// socket either (the system bus, Docker), which the sandbox refuses.
pub const RECOMMENDED_EXTENSIONS: &[Recommendation] = &[
    Recommendation {
        store: Store::Raycast,
        owner: "gebeto",
        name: "translate",
        title: "Google Translate",
        description: "Translate text between languages.",
    },
    Recommendation {
        store: Store::Raycast,
        owner: "mblode",
        name: "google-search",
        title: "Google Search",
        description: "Search Google with suggestions as you type.",
    },
    Recommendation {
        store: Store::Raycast,
        owner: "josephschmitt",
        name: "gif-search",
        title: "GIF Search",
        description: "Find animated GIFs and copy them.",
    },
    Recommendation {
        store: Store::Raycast,
        owner: "vimtor",
        name: "tailwindcss",
        title: "Tailwind CSS",
        description: "Search the Tailwind CSS documentation.",
    },
    Recommendation {
        store: Store::Vicinae,
        owner: "dagimg-dot",
        name: "wifi-commander",
        title: "Wifi Commander",
        description: "Connect to Wi-Fi networks and manage saved ones.",
    },
    Recommendation {
        store: Store::Vicinae,
        owner: "costeer",
        name: "color-converter",
        title: "Color Converter",
        description: "Convert colors between hex, RGB and HSL.",
    },
    Recommendation {
        store: Store::Vicinae,
        owner: "fbosch",
        name: "flathub-search",
        title: "Flathub",
        description: "Search Flathub for applications.",
    },
];

/// Where one recommendation stands on the extensions step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    /// Not installed: Install is offered.
    Available,
    /// Downloading and installing.
    Installing,
    /// Installed, before the flow opened or since.
    Installed,
    /// The last attempt failed: Install is offered again.
    Failed,
}

impl Install {
    /// The row's button label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Available => "Install",
            Self::Installing => "Installing...",
            Self::Installed => "Installed",
            Self::Failed => "Try Again",
        }
    }

    /// Whether the button does anything.
    #[must_use]
    pub const fn can_install(self) -> bool {
        matches!(self, Self::Available | Self::Failed)
    }
}

/// What the extensions step says when an install fails. The store's own
/// reason says whether it could be reached at all; either way the flow goes
/// on, and the store is there later.
#[must_use]
pub fn install_failed(title: &str, reason: &str) -> String {
    let reason = reason.trim().trim_end_matches('.');
    format!(
        "Could not install {title}: {reason}. You can continue and install it later from the \
         Extension Store."
    )
}

/// The extensions step's state: one [`Install`] per entry of
/// [`RECOMMENDED_EXTENSIONS`], and what the last failure said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extensions {
    states: Vec<Install>,
    notice: Option<String>,
}

impl Extensions {
    /// The step as the flow opens: each recommendation installed or not,
    /// as `installed` answers for its id.
    #[must_use]
    pub fn new(installed: impl Fn(&str) -> bool) -> Self {
        let states = RECOMMENDED_EXTENSIONS
            .iter()
            .map(|recommendation| {
                if installed(&recommendation.id()) {
                    Install::Installed
                } else {
                    Install::Available
                }
            })
            .collect();
        Self {
            states,
            notice: None,
        }
    }

    /// Where recommendation `index` stands.
    #[must_use]
    pub fn state(&self, index: usize) -> Option<Install> {
        self.states.get(index).copied()
    }

    /// Install was pressed on `index`: the recommendation to install, or
    /// `None` when there is nothing to do (installed, already installing, no
    /// such row).
    pub fn start(&mut self, index: usize) -> Option<&'static Recommendation> {
        let state = self.states.get_mut(index)?;
        if !state.can_install() {
            return None;
        }
        *state = Install::Installing;
        self.notice = None;
        RECOMMENDED_EXTENSIONS.get(index)
    }

    /// The install of `index` finished.
    pub fn finished(&mut self, index: usize, result: Result<(), String>) {
        let (Some(state), Some(recommendation)) = (
            self.states.get_mut(index),
            RECOMMENDED_EXTENSIONS.get(index),
        ) else {
            return;
        };
        match result {
            Ok(()) => *state = Install::Installed,
            Err(reason) => {
                *state = Install::Failed;
                self.notice = Some(install_failed(recommendation.title, &reason));
            }
        }
    }

    /// What the last failure said, until the next install starts.
    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_home_carried_over_from_vicinae_is_recognised() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!came_from_vicinae(dir.path()), "a fresh home");
        std::fs::create_dir_all(dir.path().join("compass")).unwrap();
        std::fs::write(dir.path().join("compass/compass.json"), "{}").unwrap();
        assert!(
            !came_from_vicinae(dir.path()),
            "Compass's own file is not Vicinae's"
        );
        std::fs::write(dir.path().join("compass/settings.json"), "{}").unwrap();
        assert!(came_from_vicinae(dir.path()));
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(other.path().join("vicinae")).unwrap();
        assert!(came_from_vicinae(other.path()), "not moved yet");
    }

    #[test]
    fn it_is_due_until_the_current_version_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compass").join(FILE_NAME);
        assert!(should_show(&path, false), "no file is never finished");
        assert!(!should_show(&path, true), "switched off");

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();
        assert!(should_show(&path, false), "unreadable counts as never");
        std::fs::write(&path, r#"{"version":0,"completedAt":""}"#).unwrap();
        assert!(should_show(&path, false), "an older version");

        std::fs::remove_file(&path).unwrap();
        mark_completed(&path, "2026-09-25T10:00:00Z").unwrap();
        assert!(!should_show(&path, false));
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({"version": 1, "completedAt": "2026-09-25T10:00:00Z"}),
            "the C++'s file, field for field"
        );
    }

    #[test]
    fn the_cpps_own_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"version":1,"completedAt":"2026-01-02T03:04:05Z"}"#,
        )
        .unwrap();
        assert_eq!(completed_version(&path), 1);
        assert!(!should_show(&path, false));
    }

    #[test]
    fn linux_has_four_steps_and_continue_finishes_on_the_last() {
        let mut flow = Flow::new(false);
        assert_eq!(flow.count(), 4);
        assert_eq!(flow.step(), Step::Welcome);
        assert!(!flow.can_go_back());
        assert_eq!(flow.primary_label(), "Continue");
        assert_eq!(flow.advance(), Advance::Next);
        assert_eq!(flow.step(), Step::Personalize);
        assert_eq!(flow.advance(), Advance::Next);
        assert_eq!(flow.step(), Step::Extensions);
        assert_eq!(flow.primary_label(), "Continue");
        assert_eq!(flow.advance(), Advance::Next);
        assert_eq!(flow.step(), Step::Complete);
        assert_eq!(flow.primary_label(), "Finish");
        assert_eq!(flow.advance(), Advance::Finish);
        assert_eq!(flow.step(), Step::Complete, "finishing stays put");
        flow.back();
        assert_eq!(flow.step(), Step::Extensions);
        flow.jump(0);
        assert_eq!(flow.step(), Step::Welcome);
        flow.jump(9);
        assert_eq!(flow.step(), Step::Welcome, "no such dot");
        flow.back();
        assert_eq!(flow.position(), 0);
    }

    #[test]
    fn the_permissions_step_is_macos_only() {
        let mut flow = Flow::new(true);
        assert_eq!(flow.count(), 5);
        flow.advance();
        assert_eq!(flow.step(), Step::Permissions);
    }

    #[test]
    fn the_switch_reads_like_a_boolean_environment_variable() {
        use std::ffi::OsStr;
        assert!(!disabled_by(None));
        assert!(!disabled_by(Some(OsStr::new(""))));
        assert!(!disabled_by(Some(OsStr::new("0"))));
        assert!(disabled_by(Some(OsStr::new("1"))));
    }

    #[test]
    fn the_last_step_says_what_finish_does() {
        assert_eq!(
            Step::Complete.subtitle(false),
            "Compass is ready. Finish opens the launcher."
        );
        assert_eq!(
            Step::Complete.subtitle(true),
            "Compass is ready. Open the launcher with:"
        );
    }

    #[test]
    fn the_recommendations_are_installable_store_extensions() {
        assert!(
            (6..=8).contains(&RECOMMENDED_EXTENSIONS.len()),
            "a short list"
        );
        let mut ids = std::collections::HashSet::new();
        for recommendation in RECOMMENDED_EXTENSIONS {
            let id = recommendation.id();
            assert!(
                crate::store_bundle::is_safe_id(&id),
                "{id} is not an id Compass installs"
            );
            assert!(ids.insert(id.clone()), "{id} is recommended twice");
            assert!(!recommendation.owner.is_empty() && !recommendation.title.is_empty());
            assert!(
                recommendation.description.ends_with('.'),
                "{id}: a whole sentence"
            );
        }
    }

    /// Suite 1 is the record of what renders on Linux; a recommendation must
    /// be one it shows rendering, so a new user's first extension works.
    #[test]
    fn suite_1_shows_every_recommendation_rendering() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/suite1/expected.json"
        );
        let expected: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for recommendation in RECOMMENDED_EXTENSIONS {
            let id = recommendation.id();
            let verdicts: Vec<&str> = expected
                .as_object()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.split(':').next() == Some(id.as_str()))
                .filter_map(|(_, value)| value["verdict"].as_str())
                .collect();
            assert_eq!(verdicts, ["rendered"], "{id}");
        }
    }

    /// Both stores are recommended, each row by the author Suite 1 installed
    /// it from, so Install fetches the build that was seen rendering.
    #[test]
    fn suite_1_installed_every_recommendation_from_its_store_and_author() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/suite1/corpus.json"
        );
        let corpus: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for store in [Store::Raycast, Store::Vicinae] {
            let recommended: Vec<_> = RECOMMENDED_EXTENSIONS
                .iter()
                .filter(|recommendation| recommendation.store == store)
                .collect();
            assert!(
                recommended.len() >= 3,
                "{} has {} recommendations",
                store.name(),
                recommended.len()
            );
            let listed = corpus[store.key()].as_array().unwrap();
            for recommendation in recommended {
                // A redirected extension installs under another id, and the
                // step would never see it installed.
                assert!(
                    store != Store::Raycast
                        || crate::raycast_overrides::Manifest::shipped()
                            .raycast_redirect(recommendation.name)
                            .is_none(),
                    "{} is redirected on Linux",
                    recommendation.name
                );
                assert!(
                    listed.iter().any(|entry| {
                        entry["name"] == recommendation.name
                            && entry["author"] == recommendation.owner
                    }),
                    "{} {}/{} is not in Suite 1's corpus",
                    store.name(),
                    recommendation.owner,
                    recommendation.name
                );
            }
        }
    }

    #[test]
    fn an_installed_recommendation_is_not_offered_again() {
        let first = RECOMMENDED_EXTENSIONS[0].id();
        let mut step = Extensions::new(|id| id == first);
        assert_eq!(step.state(0), Some(Install::Installed));
        assert_eq!(step.state(1), Some(Install::Available));
        assert_eq!(step.start(0), None, "already installed");
        assert_eq!(step.state(RECOMMENDED_EXTENSIONS.len()), None);
        assert_eq!(step.start(RECOMMENDED_EXTENSIONS.len()), None);
        assert_eq!(Install::Installed.label(), "Installed");
        assert!(!Install::Installed.can_install());
    }

    #[test]
    fn an_install_runs_once_and_ends_installed() {
        let mut step = Extensions::new(|_| false);
        assert_eq!(step.start(1), Some(&RECOMMENDED_EXTENSIONS[1]));
        assert_eq!(step.state(1), Some(Install::Installing));
        assert_eq!(step.state(1).map(Install::label), Some("Installing..."));
        assert_eq!(step.start(1), None, "a second press while it runs");
        step.finished(1, Ok(()));
        assert_eq!(step.state(1), Some(Install::Installed));
        assert_eq!(step.notice(), None);
    }

    /// Offline, the store cannot be reached: the step says so, offers the
    /// install again, and nothing stops the flow from going on.
    #[test]
    fn an_unreachable_store_is_reported_and_can_be_retried() {
        let mut step = Extensions::new(|_| false);
        let title = RECOMMENDED_EXTENSIONS[0].title;
        step.start(0);
        step.finished(
            0,
            Err("Could not fetch extension data from the store: dns error.".to_owned()),
        );
        assert_eq!(step.state(0), Some(Install::Failed));
        assert_eq!(step.state(0).map(Install::label), Some("Try Again"));
        let expected = format!(
            "Could not install {title}: Could not fetch extension data from the store: dns \
             error. You can continue and install it later from the Extension Store."
        );
        assert_eq!(step.notice(), Some(expected.as_str()));
        assert!(step.start(0).is_some(), "Try Again installs again");
        assert_eq!(step.notice(), None, "and clears what the failure said");

        let mut flow = Flow::new(false);
        flow.jump(2);
        assert_eq!(flow.step(), Step::Extensions);
        assert_eq!(
            flow.advance(),
            Advance::Next,
            "the step never blocks Continue"
        );
    }

    #[test]
    fn the_hotkey_docs_are_compass_own() {
        assert_eq!(
            HOTKEY_DOCS_URL,
            "https://tunaos.org/docs/compass/getting-started#set-a-keyboard-shortcut"
        );
        assert_eq!(GITHUB_URL, "https://github.com/tuna-os/compass");
        let guide = include_str!("../../../docs/getting-started.md");
        assert!(
            guide
                .lines()
                .any(|line| line == "## Set a keyboard shortcut"),
            "the anchor is the heading's slug"
        );
    }
}
