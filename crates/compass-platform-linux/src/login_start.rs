//! [`LoginStarter`] on Linux: the Background portal, or Compass's own XDG
//! autostart entry where an unsandboxed install has no portal
//! (`compass_core::login_start`, ADR-0022 §2).
//!
//! The portal is probed on every call rather than once at start: a portal
//! backend can be installed, or a `portals.conf` changed, while the engine
//! runs, and the answer decides which mechanism is used.

use std::path::PathBuf;
use std::time::Duration;

use compass_core::login_start::{
    self, Entry, Mechanism, Plan, PortalAnswer, State, Status, after_portal,
};
use compass_platform::{LoginStartFuture, LoginStarter};
use compass_portals::{BackgroundOutcome, BackgroundPortal, PortalConfig, Portals};

/// How long a call that shows no dialog may take: probing the portal.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the desktop's dialog may stay unanswered before the request is
/// reported as failed. Whoever asks the engine waits a little longer.
pub const DIALOG_TIMEOUT: Duration = Duration::from_secs(120);

/// Starts Compass at login on Linux.
#[derive(Debug, Clone)]
pub struct LinuxLoginStart {
    portal: PortalConfig,
    sandboxed: bool,
    record: Option<PathBuf>,
    config_home: Option<PathBuf>,
    unit_dirs: Vec<PathBuf>,
}

impl LinuxLoginStart {
    /// This session's: the ambient bus, whether this process runs in the
    /// Flatpak, and the XDG directories.
    #[must_use]
    pub fn from_environment() -> Self {
        let config_home = compass_core::xdg_dirs::config_home();
        let data_home = compass_core::xdg_dirs::data_home();
        Self {
            portal: PortalConfig::default()
                .with_call_timeout(CALL_TIMEOUT)
                .with_dialog_timeout(DIALOG_TIMEOUT),
            sandboxed: compass_core::hotkey_guide::in_flatpak(),
            record: login_start::default_record_path(),
            unit_dirs: login_start::unit_dirs(config_home.as_deref(), data_home.as_deref()),
            config_home,
        }
    }

    /// One with every input given, for tests: the portal's bus and
    /// deadlines, whether to behave as sandboxed, where the answer is
    /// recorded, `$XDG_CONFIG_HOME`, and the unit directories.
    #[must_use]
    pub fn new(
        portal: PortalConfig,
        sandboxed: bool,
        record: PathBuf,
        config_home: PathBuf,
        unit_dirs: Vec<PathBuf>,
    ) -> Self {
        Self {
            portal,
            sandboxed,
            record: Some(record),
            config_home: Some(config_home),
            unit_dirs,
        }
    }

    /// The Background portal, when this session has one.
    async fn background(&self) -> Option<BackgroundPortal> {
        let portals = match Portals::connect(self.portal.clone()).await {
            Ok(portals) => portals,
            Err(error) => {
                tracing::info!(%error, "no session bus for the Background portal");
                return None;
            }
        };
        match portals.background().await {
            Ok(background) => Some(background),
            Err(error) => {
                tracing::info!(%error, "no Background portal");
                None
            }
        }
    }

    fn autostart_path(&self) -> Option<PathBuf> {
        self.config_home.as_deref().map(login_start::autostart_path)
    }

    fn entry(&self) -> Entry {
        self.autostart_path()
            .map_or(Entry::Absent, |path| login_start::autostart_owner(&path))
    }

    fn recorded(&self) -> State {
        self.record
            .as_deref()
            .map(login_start::load_record)
            .unwrap_or_default()
    }

    fn status_of(&self, state: State) -> Status {
        Status {
            state,
            unit: login_start::packaged_unit(&self.unit_dirs),
        }
    }

    async fn read(&self) -> Status {
        let mechanism = Mechanism::choose(self.background().await.is_some(), self.sandboxed);
        self.status_of(login_start::current(
            self.recorded(),
            mechanism,
            self.entry(),
        ))
    }

    /// Removes Compass's own autostart entry, if it wrote one: after the
    /// portal took over, so the session does not start Compass twice, and
    /// after turning it off.
    fn remove_own_entry(&self) -> Result<(), String> {
        self.autostart_path()
            .map_or(Ok(()), |path| login_start::remove_autostart(&path))
    }

    async fn change(&self, enabled: bool) -> Result<Status, String> {
        let portal = self.background().await;
        let mechanism = Mechanism::choose(portal.is_some(), self.sandboxed);
        let state = match (login_start::plan(enabled, mechanism), portal) {
            (Plan::Portal { autostart }, Some(portal)) => {
                let answer = match portal
                    .request(autostart, login_start::COMMAND, login_start::REASON)
                    .await
                {
                    Ok(BackgroundOutcome::Granted { autostart, .. }) => {
                        PortalAnswer::Granted { autostart }
                    }
                    Ok(BackgroundOutcome::Denied) => PortalAnswer::Dismissed,
                    Ok(_) => PortalAnswer::Refused,
                    Err(error) => PortalAnswer::Failed(error.to_string()),
                };
                let state = after_portal(autostart, answer)?;
                self.remove_own_entry()?;
                state
            }
            (Plan::WriteFile, _) => {
                let path = self.autostart_path().ok_or_else(|| {
                    "Compass cannot find your configuration folder, so it cannot add an autostart entry."
                        .to_owned()
                })?;
                login_start::write_autostart(&path)?;
                State::On {
                    mechanism: Mechanism::AutostartFile,
                }
            }
            (Plan::RemoveFile, _) => {
                self.remove_own_entry()?;
                State::Off
            }
            (Plan::Nothing(state), _) => return Ok(self.status_of(state)),
            // The portal went away between the probe and the call.
            (Plan::Portal { .. }, None) => return Ok(self.status_of(State::Unavailable)),
        };
        if let Some(record) = &self.record {
            login_start::save_record(record, &state).map_err(|error| {
                format!(
                    "Could not record the change in {}: {error}.",
                    record.display()
                )
            })?;
        }
        tracing::info!(?state, "login start changed");
        Ok(self.status_of(state))
    }
}

impl LoginStarter for LinuxLoginStart {
    fn status(&self) -> LoginStartFuture<'_> {
        Box::pin(async move { Ok(self.read().await) })
    }

    fn set(&self, enabled: bool) -> LoginStartFuture<'_> {
        Box::pin(self.change(enabled))
    }
}
