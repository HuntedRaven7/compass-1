//! Where the launcher's own hotkey stands with the desktop, and what the
//! first-run flow says about it.
//!
//! On GNOME and KDE the hotkey is a grant: the engine asks the
//! GlobalShortcuts portal, the desktop shows its own permission dialog, and
//! the person may accept, decline, or leave the key unassigned because
//! something else has it. Each of those needs a different next step, so the
//! engine reports one [`LauncherShortcut`] and the onboarding and the settings
//! say it in whole sentences from here.
//!
//! While the first-run flow is due, the engine does not ask the portal on its
//! own: the dialog would appear beside the flow before anything has explained
//! it. The flow's Set Up Shortcut asks, and finishing or closing the flow asks
//! if the person did not ([`LauncherShortcut::NotRequested`]). Compass only
//! ever binds its own shortcuts through its own portal session, so nothing
//! here touches another application's or the desktop's bindings.

use crate::hotkey_guide::Compositor;

/// The launcher hotkey's state, as the engine last learned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherShortcut {
    /// The engine is waiting for the first-run flow to ask: the portal's
    /// dialog is shown only when the person chooses to set the key up.
    NotRequested,
    /// The desktop has been asked and has not answered yet; its dialog is
    /// probably on screen.
    Pending,
    /// Bound. `trigger` is the desktop's own description of the key, which
    /// may differ from the one asked for, when the desktop gave one.
    Granted {
        /// The key, in the desktop's words.
        trigger: Option<String>,
    },
    /// The desktop answered but did not bind the launcher's key, usually
    /// because something else already has it. `reason` is a whole sentence.
    Conflict {
        /// Why, as a sentence.
        reason: String,
    },
    /// The person declined the desktop's permission dialog.
    Refused,
    /// There is nothing to ask: no GlobalShortcuts portal and no compositor
    /// hotkey protocol in this session.
    Unavailable,
    /// Asking failed. `reason` is a whole sentence.
    Failed {
        /// Why, as a sentence.
        reason: String,
    },
    /// The configuration has no launcher hotkey (`launcher.hotkey` is empty).
    Unset,
    /// The engine was started with `--no-hotkey` and binds nothing.
    Off,
}

impl LauncherShortcut {
    /// Whether the key opens the launcher.
    #[must_use]
    pub const fn is_granted(&self) -> bool {
        matches!(self, Self::Granted { .. })
    }

    /// Whether asking the desktop (again) is a step that can help: before
    /// the first ask, and after a conflict, a refusal or a failure. Never
    /// while an answer is awaited, and never when there is nothing to ask.
    #[must_use]
    pub const fn can_request(&self) -> bool {
        matches!(
            self,
            Self::NotRequested | Self::Conflict { .. } | Self::Refused | Self::Failed { .. }
        )
    }

    /// The label of the button that asks: Set Up Shortcut the first time,
    /// Try Again after an answer that did not bind the key.
    #[must_use]
    pub const fn request_label(&self) -> &'static str {
        match self {
            Self::NotRequested => "Set Up Shortcut",
            _ => "Try Again",
        }
    }

    /// What the state means and what to do next, as whole sentences. `key`
    /// is the configured hotkey as it is shown, and `command` the toggle
    /// command a person binds by hand
    /// ([`crate::hotkey_guide::toggle_command`]).
    #[must_use]
    pub fn describe(&self, key: &str, command: &str) -> String {
        match self {
            Self::NotRequested => format!(
                "Compass asks your desktop for permission to open the launcher with {key}. Your desktop shows its own dialog, where you can also choose another key."
            ),
            Self::Pending => format!(
                "Waiting for your desktop to answer the request for {key}. Look for its permission dialog."
            ),
            Self::Granted {
                trigger: Some(trigger),
            } if !trigger.is_empty() => {
                format!("Press {trigger} to open Compass from anywhere.")
            }
            Self::Granted { .. } => format!("Press {key} to open Compass from anywhere."),
            Self::Conflict { reason } => format!(
                "Your desktop did not bind {key}. {reason} Choose another key in Settings, or bind a key to `{command}` in your desktop's keyboard settings."
            ),
            Self::Refused => format!(
                "You declined your desktop's permission dialog, so {key} does nothing yet. Choose Try Again to be asked again, or bind a key to `{command}` in your desktop's keyboard settings."
            ),
            Self::Unavailable => format!(
                "Your desktop has no GlobalShortcuts portal, so Compass cannot bind a key itself. Bind a key to `{command}` in your desktop's keyboard settings."
            ),
            Self::Failed { reason } => format!(
                "Asking your desktop for {key} failed: {reason} Choose Try Again, or bind a key to `{command}` in your desktop's keyboard settings."
            ),
            Self::Unset => format!(
                "No launcher hotkey is set. Record one in Settings, or bind a key to `{command}` in your desktop's keyboard settings."
            ),
            Self::Off => format!(
                "The engine was started with --no-hotkey, so it binds no keys. Bind a key to `{command}` in your desktop's keyboard settings."
            ),
        }
    }
}

/// What the onboarding's hotkey row shows: on Sway, Hyprland and niri the
/// compositor's own configuration line, whatever the engine reports, since
/// there the compositor owns the key; elsewhere the engine's state, or, before
/// the engine has said anything, the plain instruction to bind the toggle
/// command.
#[must_use]
pub fn guidance(
    state: Option<&LauncherShortcut>,
    compositor: Option<Compositor>,
    key: &str,
    command: &str,
) -> String {
    if let Some(compositor) = compositor {
        return compositor.instruction(command);
    }
    match state {
        Some(state) => state.describe(key, command),
        None => format!(
            "Press {key} to open Compass from anywhere. If it does nothing, bind a key to `{command}` in your desktop's keyboard settings."
        ),
    }
}

/// Whether the onboarding offers to ask the desktop: not where the
/// compositor binds keys in its own configuration, and only when asking can
/// help ([`LauncherShortcut::can_request`]).
#[must_use]
pub fn offers_request(state: Option<&LauncherShortcut>, compositor: Option<Compositor>) -> bool {
    compositor.is_none() && state.is_some_and(LauncherShortcut::can_request)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "Super+Space";
    const COMMAND: &str = "compass toggle";

    #[test]
    fn each_state_says_what_happened_and_what_to_do() {
        let granted = LauncherShortcut::Granted {
            trigger: Some("Super+Space".to_owned()),
        };
        assert_eq!(
            granted.describe("Alt+Space", COMMAND),
            "Press Super+Space to open Compass from anywhere.",
            "the desktop's own description wins"
        );
        let conflict = LauncherShortcut::Conflict {
            reason: "The desktop did not assign a key to it.".to_owned(),
        };
        assert!(
            conflict
                .describe(KEY, COMMAND)
                .contains("Choose another key in Settings")
        );
        assert!(
            LauncherShortcut::Refused
                .describe(KEY, COMMAND)
                .contains("declined")
        );
        assert!(
            LauncherShortcut::Unavailable
                .describe(KEY, COMMAND)
                .contains("no GlobalShortcuts portal")
        );
        let failed = LauncherShortcut::Failed {
            reason: "The portal did not answer in time.".to_owned(),
        };
        assert!(failed.describe(KEY, COMMAND).contains("did not answer"));
        for state in [
            conflict,
            LauncherShortcut::Refused,
            LauncherShortcut::Unavailable,
            failed,
            LauncherShortcut::Unset,
            LauncherShortcut::Off,
        ] {
            assert!(
                state.describe(KEY, COMMAND).contains("`compass toggle`"),
                "{state:?} offers the manual binding"
            );
            assert!(!state.is_granted());
        }
    }

    #[test]
    fn asking_is_offered_only_where_it_can_help() {
        assert!(LauncherShortcut::NotRequested.can_request());
        assert_eq!(
            LauncherShortcut::NotRequested.request_label(),
            "Set Up Shortcut"
        );
        assert_eq!(LauncherShortcut::Refused.request_label(), "Try Again");
        for state in [
            LauncherShortcut::Pending,
            LauncherShortcut::Granted { trigger: None },
            LauncherShortcut::Unavailable,
            LauncherShortcut::Unset,
            LauncherShortcut::Off,
        ] {
            assert!(!state.can_request(), "{state:?}");
        }
        assert!(
            !offers_request(Some(&LauncherShortcut::Refused), Some(Compositor::Sway)),
            "the compositor owns the key on Sway"
        );
        assert!(
            !offers_request(None, None),
            "nothing known, nothing offered"
        );
    }

    #[test]
    fn wlroots_compositors_keep_their_configuration_line() {
        let line = guidance(
            Some(&LauncherShortcut::Unavailable),
            Some(Compositor::Hyprland),
            KEY,
            COMMAND,
        );
        assert_eq!(line, Compositor::Hyprland.instruction(COMMAND));
        assert!(guidance(None, None, KEY, COMMAND).starts_with("Press Super+Space"));
    }
}
