//! The first-run flow's state in the launcher: where the flow is, the themes
//! the "Make it your own" step offers, the launcher hotkey's standing with
//! the desktop, and where finishing is recorded.
//!
//! `compass_core::onboarding` decides the steps and the file; this is what the
//! launcher's page holds around it.

use std::path::PathBuf;

use compass_core::hotkey_guide::Compositor;
use compass_core::launcher_shortcut::{self, LauncherShortcut};
use compass_core::onboarding::{Extensions, Flow};

use crate::theme::Theme;

/// One entry of the theme dropdown: a theme, shown by its title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeOption(pub Theme);

impl std::fmt::Display for ThemeOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.title())
    }
}

/// A control on a step that the keyboard can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// The theme dropdown.
    Theme,
    /// "Open Docs" for the hotkey, where asking the desktop cannot help.
    OpenDocs,
    /// Set Up Shortcut or Try Again: ask the desktop for the hotkey.
    RequestShortcut,
    /// The Start at login switch.
    LoginStart,
    /// A recommended extension's Install, by its position.
    Install(usize),
    /// The last step's GitHub link.
    GitHub,
    /// Back.
    Back,
    /// Continue, or Finish on the last step.
    Continue,
}

/// The page's state.
#[derive(Debug, Clone)]
pub struct OnboardingPage {
    /// Where the flow is.
    pub flow: Flow,
    /// The themes offered: the curated ones, then the theme files.
    pub themes: Vec<ThemeOption>,
    /// Where finishing is recorded (`onboarding.json`).
    pub state_path: PathBuf,
    /// Why something did not happen (a theme not kept, a link not opened).
    pub notice: Option<String>,
    /// The extensions step's recommendations and their installs.
    pub extensions: Extensions,
    /// The control with the keyboard, if one has it.
    pub focused: Option<Control>,
    /// The theme highlighted in the open theme dropdown.
    pub menu: Option<usize>,
    /// Where the launcher hotkey stands, once the engine has said.
    pub shortcut: Option<LauncherShortcut>,
    /// The compositor that binds keys in its own configuration, if this
    /// session runs one: its line is the guidance, and nothing is asked.
    pub compositor: Option<Compositor>,
}

impl OnboardingPage {
    /// The flow from its first step, offering the curated themes and
    /// `files`, and recommending the extensions `installed` does not know
    /// by id; Linux has no permissions step.
    #[must_use]
    pub fn new(state_path: PathBuf, files: Vec<Theme>, installed: impl Fn(&str) -> bool) -> Self {
        let mut themes: Vec<ThemeOption> = Theme::ALL.into_iter().map(ThemeOption).collect();
        themes.extend(files.into_iter().map(ThemeOption));
        Self {
            flow: Flow::new(false),
            themes,
            state_path,
            notice: None,
            extensions: Extensions::new(installed),
            focused: None,
            menu: None,
            shortcut: None,
            compositor: None,
        }
    }

    /// The step's controls in the order they are drawn: its own, then Back
    /// and Continue. An Install that cannot be pressed is not one.
    #[must_use]
    pub fn controls(&self) -> Vec<Control> {
        use compass_core::onboarding::{Install, RECOMMENDED_EXTENSIONS, Step};
        let mut controls = match self.flow.step() {
            Step::Personalize => {
                let hotkey = if self.offers_request() {
                    Control::RequestShortcut
                } else {
                    Control::OpenDocs
                };
                vec![Control::Theme, hotkey, Control::LoginStart]
            }
            Step::Extensions => (0..RECOMMENDED_EXTENSIONS.len())
                .filter(|&index| {
                    self.extensions
                        .state(index)
                        .unwrap_or(Install::Available)
                        .can_install()
                })
                .map(Control::Install)
                .collect(),
            Step::Complete => vec![Control::GitHub],
            Step::Welcome | Step::Permissions => Vec::new(),
        };
        if self.flow.can_go_back() {
            controls.push(Control::Back);
        }
        controls.push(Control::Continue);
        controls
    }

    /// Whether the hotkey row offers to ask the desktop
    /// ([`launcher_shortcut::offers_request`]).
    #[must_use]
    pub fn offers_request(&self) -> bool {
        launcher_shortcut::offers_request(self.shortcut.as_ref(), self.compositor)
    }

    /// Whether finishing or closing the flow should ask the desktop for the
    /// hotkey: nobody pressed Set Up Shortcut, and the engine is, or may be,
    /// waiting for the flow to ask.
    #[must_use]
    pub fn asks_on_finish(&self) -> bool {
        self.compositor.is_none()
            && matches!(self.shortcut, None | Some(LauncherShortcut::NotRequested))
    }

    /// Tab or Shift+Tab.
    pub fn move_focus(&mut self, forward: bool) {
        self.focused = crate::focus::step(&self.controls(), self.focused.as_ref(), forward);
        self.menu = None;
    }

    /// Another step is on show: nothing on it has the keyboard yet.
    pub fn step_changed(&mut self) {
        self.focused = None;
        self.menu = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dropdown_offers_the_curated_themes_by_title() {
        let page = OnboardingPage::new(PathBuf::from("/nonexistent"), Vec::new(), |_| false);
        assert_eq!(page.themes.len(), Theme::ALL.len());
        assert_eq!(page.themes[0].to_string(), Theme::ALL[0].title());
        assert_eq!(page.flow.count(), 4, "no permissions step on Linux");
    }

    #[test]
    fn tab_reaches_each_steps_controls_then_back_and_continue() {
        let mut page = OnboardingPage::new(PathBuf::from("/nonexistent"), Vec::new(), |_| false);
        assert_eq!(page.controls(), [Control::Continue], "the welcome step");
        let _ = page.flow.advance();
        assert_eq!(
            page.controls(),
            [
                Control::Theme,
                Control::OpenDocs,
                Control::LoginStart,
                Control::Back,
                Control::Continue
            ]
        );
        page.move_focus(true);
        assert_eq!(page.focused, Some(Control::Theme));
        page.move_focus(false);
        page.move_focus(false);
        assert_eq!(page.focused, Some(Control::Continue));
        let _ = page.flow.advance();
        page.step_changed();
        let installs = page
            .controls()
            .into_iter()
            .filter(|control| matches!(control, Control::Install(_)))
            .count();
        assert_eq!(
            installs,
            compass_core::onboarding::RECOMMENDED_EXTENSIONS.len()
        );
    }

    #[test]
    fn the_hotkey_row_asks_only_where_asking_can_help() {
        let mut page = OnboardingPage::new(PathBuf::from("/nonexistent"), Vec::new(), |_| false);
        let _ = page.flow.advance();
        assert!(page.asks_on_finish(), "nothing known: finishing asks");
        for (state, offered) in [
            (LauncherShortcut::NotRequested, true),
            (LauncherShortcut::Refused, true),
            (
                LauncherShortcut::Conflict {
                    reason: "Taken.".to_owned(),
                },
                true,
            ),
            (LauncherShortcut::Granted { trigger: None }, false),
            (LauncherShortcut::Unavailable, false),
        ] {
            page.shortcut = Some(state.clone());
            let hotkey = if offered {
                Control::RequestShortcut
            } else {
                Control::OpenDocs
            };
            assert_eq!(page.controls()[1], hotkey, "{state:?}");
        }
        assert!(!page.asks_on_finish(), "an answer was had");
        page.shortcut = Some(LauncherShortcut::Refused);
        page.compositor = Some(Compositor::Niri);
        assert_eq!(page.controls()[1], Control::OpenDocs, "niri owns the key");
        assert!(!page.asks_on_finish());
    }
}
