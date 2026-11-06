//! Starting Compass at login: which mechanism, what state it is in, and the
//! files Compass owns for it (ADR-0022 §2).
//!
//! Every package asks the desktop's Background portal for autostart with
//! [`COMMAND`]. On GNOME and KDE the portal serves unsandboxed applications
//! too, so the Flatpak and the native packages take the same path, and the
//! desktop shows the person what it granted. Only an unsandboxed install in a
//! session with no Background portal (some wlroots sessions) writes
//! `$XDG_CONFIG_HOME/autostart/compass.desktop` itself, marked with
//! [`OWNER_KEY`], and only a file with that mark is ever replaced or removed.
//!
//! The packaged systemd user unit (`packaging/systemd/compass.service`) is a
//! packager's option. Compass never enables or disables it; it is reported
//! ([`Unit`]) so that the person is not left wondering why Compass starts
//! with the switch off. A second `compass start --hidden` while one runs exits
//! at once (the resident instance's lease), so the two never make two
//! launchers.
//!
//! The portal cannot be asked what it granted earlier, so the answer it gave
//! is recorded in `$XDG_STATE_HOME/compass/login-start.json` and checked
//! against what can be seen ([`current`]). The default is off: nothing here
//! runs until the person chooses, in the first-run flow or in Settings.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The command line the desktop runs at login: a resident launcher that
/// waits, hidden, for its hotkey.
pub const COMMAND: &[&str] = &["compass", "start", "--hidden"];

/// Why Compass asks, as the Background portal shows it to the person.
pub const REASON: &str =
    "Compass starts hidden when you log in, so its hotkey opens the launcher straight away.";

/// The autostart entry Compass writes where there is no portal, in
/// `$XDG_CONFIG_HOME/autostart`.
pub const AUTOSTART_FILE_NAME: &str = "compass.desktop";

/// The key that marks an autostart entry as Compass's own.
pub const OWNER_KEY: &str = "X-Compass-Login-Start";

/// Where the last answer is recorded, in `$XDG_STATE_HOME/compass`.
pub const STATE_FILE_NAME: &str = "login-start.json";

/// The packaged systemd user unit.
pub const SYSTEMD_UNIT: &str = "compass.service";

/// Where `systemctl --user enable` links the unit, under a unit directory.
const WANTS_DIR: &str = "graphical-session.target.wants";

/// The system's user-unit directories a package installs into.
pub const SYSTEM_UNIT_DIRS: &[&str] = &[
    "/etc/systemd/user",
    "/usr/local/lib/systemd/user",
    "/usr/local/share/systemd/user",
    "/usr/lib/systemd/user",
    "/usr/share/systemd/user",
    "/lib/systemd/user",
];

/// How Compass starts at login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mechanism {
    /// `org.freedesktop.portal.Background`'s autostart.
    Portal,
    /// Compass's own `$XDG_CONFIG_HOME/autostart/compass.desktop`.
    AutostartFile,
}

impl Mechanism {
    /// The mechanism this session offers: the portal wherever there is one;
    /// the autostart file only outside a sandbox, which cannot write the
    /// host's autostart directory; otherwise none.
    #[must_use]
    pub const fn choose(portal: bool, sandboxed: bool) -> Option<Self> {
        if portal {
            Some(Self::Portal)
        } else if sandboxed {
            None
        } else {
            Some(Self::AutostartFile)
        }
    }
}

/// Whether Compass starts at login.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum State {
    /// It does not: never chosen, or turned off.
    #[default]
    Off,
    /// It does, through `mechanism`.
    On {
        /// How.
        mechanism: Mechanism,
    },
    /// The desktop declined the last request.
    Refused,
    /// This session has no way to start Compass at login.
    Unavailable,
}

impl State {
    /// Whether Compass starts at login.
    #[must_use]
    pub const fn is_on(&self) -> bool {
        matches!(self, Self::On { .. })
    }

    /// The state, as a sentence for the switch's row.
    #[must_use]
    pub const fn describe(&self) -> &'static str {
        match self {
            Self::Off => "Compass does not start when you log in.",
            Self::On {
                mechanism: Mechanism::Portal,
            } => "Your desktop starts Compass, hidden, when you log in.",
            Self::On {
                mechanism: Mechanism::AutostartFile,
            } => {
                "Compass starts, hidden, when you log in, from its entry in your autostart folder."
            }
            Self::Refused => {
                "Your desktop did not allow Compass to start when you log in. Allow it in your desktop's settings for background apps, then turn this on again."
            }
            Self::Unavailable => {
                "This session has no Background portal, and the Flatpak cannot add an autostart entry itself, so Compass cannot start when you log in."
            }
        }
    }
}

/// The packaged systemd user unit, when it is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Installed and not enabled.
    Installed,
    /// Enabled: it starts Compass whatever the switch says.
    Enabled,
}

impl Unit {
    /// What it means for the switch, as a sentence.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Installed => {
                "The compass.service systemd user unit is installed but not enabled. Compass leaves it to you."
            }
            Self::Enabled => {
                "The compass.service systemd user unit is enabled, so it also starts Compass when you log in. Compass leaves it to you: run `systemctl --user disable compass.service` to stop it."
            }
        }
    }
}

/// What Settings and the first-run flow show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Compass's own login start.
    pub state: State,
    /// The packaged systemd unit, when there is one.
    pub unit: Option<Unit>,
}

impl Status {
    /// The row's description: the state, then the unit when it matters.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.unit {
            Some(unit) => format!("{} {}", self.state.describe(), unit.describe()),
            None => self.state.describe().to_owned(),
        }
    }
}

/// What the Background portal answered a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalAnswer {
    /// The request succeeded; `autostart` is what the desktop granted.
    Granted {
        /// Whether the desktop starts Compass at login now.
        autostart: bool,
    },
    /// The person dismissed the desktop's dialog (response code 1).
    Dismissed,
    /// The desktop ended the request without saying why (response code 2).
    Refused,
    /// The call failed, as a sentence.
    Failed(String),
}

/// The state after the portal answered a request to turn login start on
/// (`enable`) or off, or why the request did not change it.
///
/// # Errors
///
/// A failed call, or a request to turn it off that the desktop did not
/// carry out: the state is then what it was, and the sentence says why.
pub fn after_portal(enable: bool, answer: PortalAnswer) -> Result<State, String> {
    match (enable, answer) {
        (_, PortalAnswer::Granted { autostart: true }) => Ok(State::On {
            mechanism: Mechanism::Portal,
        }),
        (true, PortalAnswer::Granted { autostart: false })
        | (true, PortalAnswer::Dismissed | PortalAnswer::Refused) => Ok(State::Refused),
        (false, PortalAnswer::Granted { autostart: false }) => Ok(State::Off),
        (false, PortalAnswer::Dismissed | PortalAnswer::Refused) => Err(
            "Your desktop did not confirm that Compass no longer starts when you log in. Turn it off in your desktop's settings for background apps."
                .to_owned(),
        ),
        (_, PortalAnswer::Failed(reason)) => Err(format!(
            "Asking your desktop's Background portal failed: {reason}"
        )),
    }
}

/// What turning login start on or off does in a session offering
/// `mechanism`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Ask the Background portal for `autostart`.
    Portal {
        /// On or off.
        autostart: bool,
    },
    /// Write Compass's autostart entry.
    WriteFile,
    /// Remove Compass's autostart entry.
    RemoveFile,
    /// Nothing can be done here; the state is this.
    Nothing(State),
}

/// The step that turns login start on (`enable`) or off.
#[must_use]
pub const fn plan(enable: bool, mechanism: Option<Mechanism>) -> Plan {
    match (mechanism, enable) {
        (Some(Mechanism::Portal), autostart) => Plan::Portal { autostart },
        (Some(Mechanism::AutostartFile), true) => Plan::WriteFile,
        (Some(Mechanism::AutostartFile), false) => Plan::RemoveFile,
        (None, _) => Plan::Nothing(State::Unavailable),
    }
}

/// `$XDG_STATE_HOME/compass/login-start.json`.
#[must_use]
pub fn default_record_path() -> Option<PathBuf> {
    Some(crate::xdg_dirs::state_dir()?.join(STATE_FILE_NAME))
}

/// The state recorded at `path`; off when there is none or it cannot be
/// read, which is the default the person has not changed.
#[must_use]
pub fn load_record(path: &Path) -> State {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Records `state` at `path`, creating its directory.
///
/// # Errors
///
/// When the directory cannot be created or the file written.
pub fn save_record(path: &Path, state: &State) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string(state).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// `config_home/autostart/compass.desktop`.
#[must_use]
pub fn autostart_path(config_home: &Path) -> PathBuf {
    config_home.join("autostart").join(AUTOSTART_FILE_NAME)
}

/// The autostart entry Compass writes. `TryExec` makes it inert once Compass
/// is uninstalled, and the command is looked up on `PATH`, so a package
/// upgrade that moves the binary needs no rewrite.
#[must_use]
pub fn autostart_entry() -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Compass\nComment=Start the Compass launcher, hidden, when you log in.\nExec={}\nTryExec={}\nIcon={}\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n{OWNER_KEY}=true\n",
        COMMAND.join(" "),
        COMMAND[0],
        compass_xdg::brand::APP_ID,
    )
}

/// Who an autostart entry belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// There is none.
    Absent,
    /// Compass wrote it ([`OWNER_KEY`]).
    Ours,
    /// Someone else did: never replaced or removed.
    Theirs,
}

/// Who the entry at `path` belongs to.
#[must_use]
pub fn autostart_owner(path: &Path) -> Entry {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let ours = text
                .lines()
                .any(|line| line.trim() == format!("{OWNER_KEY}=true"));
            if ours { Entry::Ours } else { Entry::Theirs }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Entry::Absent,
        Err(_) => Entry::Theirs,
    }
}

/// Writes Compass's autostart entry at `path`. Writing it again changes
/// nothing.
///
/// # Errors
///
/// An entry someone else wrote, which is left alone, or a file that cannot
/// be written, as a sentence.
pub fn write_autostart(path: &Path) -> Result<(), String> {
    if autostart_owner(path) == Entry::Theirs {
        return Err(format!(
            "{} was not written by Compass, so Compass leaves it alone. Remove it, or start Compass from it yourself.",
            path.display()
        ));
    }
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(path, autostart_entry()));
    written.map_err(|error| format!("Could not write {}: {error}.", path.display()))
}

/// Removes Compass's autostart entry at `path`, if there is one. An entry
/// someone else wrote is left alone.
///
/// # Errors
///
/// A file that cannot be removed, as a sentence.
pub fn remove_autostart(path: &Path) -> Result<(), String> {
    if autostart_owner(path) != Entry::Ours {
        return Ok(());
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not remove {}: {error}.", path.display())),
    }
}

/// The state to show: the `recorded` answer, checked against what this
/// session can see. An autostart entry that is gone is off; a session with
/// no mechanism is unavailable unless the portal already started Compass at
/// login, which stays true whatever the session offers now.
#[must_use]
pub fn current(recorded: State, mechanism: Option<Mechanism>, entry: Entry) -> State {
    match recorded {
        State::On {
            mechanism: Mechanism::AutostartFile,
        } if entry != Entry::Ours => State::Off,
        State::Off | State::Unavailable if mechanism.is_none() => State::Unavailable,
        State::Unavailable => State::Off,
        state => state,
    }
}

/// The user-unit directories the packaged unit can be installed or enabled
/// in: the person's own, then the system's.
#[must_use]
pub fn unit_dirs(config_home: Option<&Path>, data_home: Option<&Path>) -> Vec<PathBuf> {
    config_home
        .into_iter()
        .chain(data_home)
        .map(|home| home.join("systemd/user"))
        .chain(SYSTEM_UNIT_DIRS.iter().map(PathBuf::from))
        .collect()
}

/// The packaged unit, as installed in `dirs`: enabled when one of them links
/// it into the graphical session's wants, installed when one holds it.
#[must_use]
pub fn packaged_unit(dirs: &[PathBuf]) -> Option<Unit> {
    if dirs
        .iter()
        .any(|dir| dir.join(WANTS_DIR).join(SYSTEMD_UNIT).exists())
    {
        return Some(Unit::Enabled);
    }
    dirs.iter()
        .any(|dir| dir.join(SYSTEMD_UNIT).exists())
        .then_some(Unit::Installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portal_wins_and_the_file_is_only_for_unsandboxed_sessions_without_one() {
        assert_eq!(Mechanism::choose(true, true), Some(Mechanism::Portal));
        assert_eq!(Mechanism::choose(true, false), Some(Mechanism::Portal));
        assert_eq!(
            Mechanism::choose(false, false),
            Some(Mechanism::AutostartFile)
        );
        assert_eq!(Mechanism::choose(false, true), None);
        assert_eq!(
            plan(true, None),
            Plan::Nothing(State::Unavailable),
            "never shown as on"
        );
    }

    #[test]
    fn the_portals_answers_become_states_and_a_refusal_is_never_on() {
        let on = State::On {
            mechanism: Mechanism::Portal,
        };
        assert_eq!(
            after_portal(true, PortalAnswer::Granted { autostart: true }),
            Ok(on.clone())
        );
        assert_eq!(
            after_portal(true, PortalAnswer::Granted { autostart: false }),
            Ok(State::Refused),
            "background allowed but not autostart is a refusal"
        );
        assert_eq!(
            after_portal(true, PortalAnswer::Dismissed),
            Ok(State::Refused)
        );
        assert_eq!(
            after_portal(true, PortalAnswer::Refused),
            Ok(State::Refused)
        );
        assert_eq!(
            after_portal(false, PortalAnswer::Granted { autostart: false }),
            Ok(State::Off)
        );
        assert!(after_portal(false, PortalAnswer::Dismissed).is_err());
        let failed = after_portal(true, PortalAnswer::Failed("No reply.".to_owned()));
        assert!(failed.is_err_and(|reason| reason.ends_with("No reply.")));
        assert!(!State::Refused.is_on());
        assert!(!State::Unavailable.is_on());
    }

    #[test]
    fn the_record_round_trips_and_defaults_to_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compass").join(STATE_FILE_NAME);
        assert_eq!(load_record(&path), State::Off, "off until chosen");
        let on = State::On {
            mechanism: Mechanism::AutostartFile,
        };
        save_record(&path, &on).unwrap();
        assert_eq!(load_record(&path), on);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"state":"on","mechanism":"autostart-file"}"#
        );
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_record(&path), State::Off);
    }

    #[test]
    fn enabling_twice_writes_one_entry_and_disabling_removes_only_ours() {
        let dir = tempfile::tempdir().unwrap();
        let path = autostart_path(dir.path());
        assert_eq!(autostart_owner(&path), Entry::Absent);
        write_autostart(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        write_autostart(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first, "idempotent");
        assert!(first.contains("Exec=compass start --hidden\n"));
        assert!(first.contains("TryExec=compass\n"));
        assert_eq!(autostart_owner(&path), Entry::Ours);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );

        remove_autostart(&path).unwrap();
        remove_autostart(&path).unwrap();
        assert_eq!(autostart_owner(&path), Entry::Absent, "idempotent");

        let theirs = "[Desktop Entry]\nType=Application\nExec=compass start\n";
        std::fs::write(&path, theirs).unwrap();
        assert!(write_autostart(&path).is_err(), "not overwritten");
        remove_autostart(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            theirs,
            "not removed"
        );
    }

    #[test]
    fn the_shown_state_follows_what_can_be_seen() {
        let file = State::On {
            mechanism: Mechanism::AutostartFile,
        };
        let portal = State::On {
            mechanism: Mechanism::Portal,
        };
        let session = Some(Mechanism::AutostartFile);
        assert_eq!(current(file.clone(), session, Entry::Ours), file);
        assert_eq!(
            current(file, session, Entry::Absent),
            State::Off,
            "the entry was deleted by hand"
        );
        assert_eq!(current(portal.clone(), None, Entry::Absent), portal);
        assert_eq!(current(State::Off, None, Entry::Absent), State::Unavailable);
        assert_eq!(
            current(State::Unavailable, Some(Mechanism::Portal), Entry::Absent),
            State::Off
        );
        assert_eq!(
            current(State::Refused, Some(Mechanism::Portal), Entry::Absent),
            State::Refused
        );
    }

    #[test]
    fn the_packaged_unit_is_reported_and_never_toggled() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("config/systemd/user");
        let system = dir.path().join("usr/lib/systemd/user");
        let dirs = vec![user.clone(), system.clone()];
        assert_eq!(packaged_unit(&dirs), None);
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(system.join(SYSTEMD_UNIT), "[Unit]\n").unwrap();
        assert_eq!(packaged_unit(&dirs), Some(Unit::Installed));
        std::fs::create_dir_all(user.join(WANTS_DIR)).unwrap();
        std::fs::write(user.join(WANTS_DIR).join(SYSTEMD_UNIT), "[Unit]\n").unwrap();
        assert_eq!(packaged_unit(&dirs), Some(Unit::Enabled));
        let status = Status {
            state: State::Off,
            unit: Some(Unit::Enabled),
        };
        assert!(status.describe().contains("systemctl --user disable"));
        assert_eq!(
            unit_dirs(Some(Path::new("/c")), Some(Path::new("/d")))[..2],
            [
                PathBuf::from("/c/systemd/user"),
                PathBuf::from("/d/systemd/user")
            ]
        );
    }
}
