//! Clipboard history, the keyring it keeps its key in, and its database.
//!
//! Part of [`super`]; see that module for the purity rule. Clipboard history
//! needs two things a bare compositor session may lack: a way to see what
//! other applications copy (the data-control protocol, or on GNOME the Shell
//! extension), and a keyring answering the Secret Service on the session bus,
//! where the key that encrypts the history is kept. Without the second, the
//! engine only logs a keyring error; this says what to install.
//!
//! When the engine runs it is asked what it actually does
//! ([`ClipboardStatus`]); otherwise `clipboard.history` says what it would
//! do. `clipboard.storage` looks at the files and the key itself, read-only, so
//! a history encrypted under a key the keyring no longer holds is named as
//! such, with `compass clipboard reset` as the way out.

use compass_clipboard::files::{PartKind, ReadError, Survey};
use compass_ipc::{ClipboardBackend, ClipboardStatus, ClipboardStorage, DoctorCheck, DoctorStatus};

use super::check;
use super::wlroots::WaylandFindings;
use crate::clipboard_service::{Inspection, KEYRING_LABEL, KeyState};
use crate::doctor::bus::BusProbe;
use crate::doctor::env::Env;

/// The Secret Service's well-known name.
pub const SECRET_SERVICE_BUS_NAME: &str = "org.freedesktop.secrets";
/// The default collection, through its alias.
pub const DEFAULT_COLLECTION_PATH: &str = "/org/freedesktop/secrets/aliases/default";
/// The collection interface, whose `Locked` says whether it is open.
pub const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";

/// What `clipboard.storage` and `clipboard.history` read.
#[derive(Debug)]
pub struct ClipboardFacts {
    /// The history on disk and its key, or why there is nowhere to look.
    pub inspection: Result<Inspection, String>,
    /// What the running engine reports; `None` when none answered.
    pub engine: Option<ClipboardStatus>,
}

/// `clipboard.history`: whether copies made in other applications reach the
/// history, and through what.
#[must_use]
pub fn clipboard_history(
    env: &Env,
    wayland: Option<&WaylandFindings>,
    engine: Option<&ClipboardStatus>,
) -> DoctorCheck {
    const NAME: &str = "clipboard.history";
    if let Some(status) = engine {
        let (status, detail) = running_history(status);
        return check(NAME, status, detail);
    }
    if super::desktop::is_gnome(env) {
        return check(
            NAME,
            DoctorStatus::Ok,
            "On GNOME, clipboard history records copies through the Compass GNOME Shell \
             extension (see gnome.shell-extension) and keeps its key in the keyring (see \
             keyring).",
        );
    }
    match wayland {
        Some(found) if found.capabilities.data_control => check(
            NAME,
            DoctorStatus::Ok,
            "The compositor offers the data-control protocol, so clipboard history can record \
             what other applications copy. It also needs the keyring (see keyring).",
        ),
        Some(_) => check(
            NAME,
            DoctorStatus::Warn,
            "The compositor does not offer the data-control protocol, so clipboard history \
             only records what is copied while the launcher is open.",
        ),
        None if env.get("XDG_SESSION_TYPE") == Some("x11") => check(
            NAME,
            DoctorStatus::Warn,
            "This is an X11 session. Compass supports Wayland sessions only, so clipboard \
             history cannot follow the clipboard.",
        ),
        None => check(
            NAME,
            DoctorStatus::Warn,
            "No Wayland display was found, so clipboard history cannot follow the clipboard.",
        ),
    }
}

/// What the running engine's own report means.
fn running_history(status: &ClipboardStatus) -> (DoctorStatus, String) {
    let Some(backend) = &status.backend else {
        return if status.storage == ClipboardStorage::Open {
            (
                DoctorStatus::Warn,
                "Clipboard history is still starting.".to_owned(),
            )
        } else {
            (
                DoctorStatus::Warn,
                "Clipboard history does not record copies yet, because its database is not \
                 open (see clipboard.storage)."
                    .to_owned(),
            )
        };
    };
    if status.capturing && !status.monitoring {
        return (
            DoctorStatus::Warn,
            "Clipboard monitoring is paused, so nothing copied is recorded. Choose Resume \
             clipboard in Clipboard History to record again."
                .to_owned(),
        );
    }
    let problem = status.capture_problem.as_deref();
    match (backend, status.capturing, problem) {
        (
            ClipboardBackend::DataControl {
                protocol: Some(protocol),
            },
            true,
            _,
        ) => (
            DoctorStatus::Ok,
            format!(
                "Clipboard history is recording what other applications copy, over the \
                 compositor's data-control protocol ({protocol})."
            ),
        ),
        (ClipboardBackend::DataControl { protocol: None }, true, _) => (
            DoctorStatus::Ok,
            "Clipboard history is recording what other applications copy, over the \
             compositor's data-control protocol."
                .to_owned(),
        ),
        (ClipboardBackend::ShellExtension, true, _) => (
            DoctorStatus::Ok,
            "Clipboard history is recording what other applications copy, through the \
             Compass GNOME Shell extension."
                .to_owned(),
        ),
        (ClipboardBackend::DataControl { .. }, false, Some(problem)) => (
            DoctorStatus::Warn,
            format!(
                "Clipboard history is not receiving copies over the compositor's data-control \
                 protocol ({problem}). Compass reconnects by itself; if this persists, check \
                 that the compositor offers ext-data-control-v1 or wlr-data-control."
            ),
        ),
        (ClipboardBackend::DataControl { .. }, false, None) => (
            DoctorStatus::Warn,
            "Clipboard history has not connected to the compositor's data-control protocol \
             yet."
                .to_owned(),
        ),
        (ClipboardBackend::ShellExtension, false, Some(problem)) => (
            DoctorStatus::Warn,
            format!(
                "Clipboard history is not receiving copies from the Compass GNOME Shell \
                 extension ({problem}). Make sure the extension is installed and enabled (see \
                 gnome.shell-extension); Compass reconnects by itself."
            ),
        ),
        (ClipboardBackend::ShellExtension, false, None) => (
            DoctorStatus::Warn,
            "Clipboard history has not connected to the Compass GNOME Shell extension yet."
                .to_owned(),
        ),
    }
}

/// `keyring`: whether a keyring answers, and is unlocked.
pub async fn keyring<B: BusProbe>(env: &Env, bus: &B) -> DoctorCheck {
    const NAME: &str = "keyring";
    let advice = match super::desktop::wlroots_compositor(env) {
        Some(compositor) => format!(
            "Install GNOME Keyring (or KeePassXC with its Secret Service integration on) and \
             start it with your session, for example with \
             `exec gnome-keyring-daemon --start --components=secrets` in {}",
            compositor.config_file()
        ),
        None => "Install GNOME Keyring, KWallet or KeePassXC with its Secret Service \
                 integration on, and make sure it starts with your session"
            .to_owned(),
    };
    let lost = "clipboard history, extension preferences and extension storage are off";
    let locked = bus
        .property(
            SECRET_SERVICE_BUS_NAME,
            DEFAULT_COLLECTION_PATH,
            COLLECTION_INTERFACE,
            "Locked",
        )
        .await;
    match locked {
        Ok(Some(value)) if value == "false" => check(
            NAME,
            DoctorStatus::Ok,
            "a keyring answers on the session bus and is unlocked; Compass keeps the key \
             that encrypts clipboard history and extension data there",
        ),
        Ok(Some(_)) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "the keyring is locked, so {lost} until it is unlocked. Logging in with a \
                 password normally unlocks it; otherwise unlock it in your keyring manager"
            ),
        ),
        Ok(None) => match bus.name_has_owner(SECRET_SERVICE_BUS_NAME).await {
            Ok(true) => check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "a keyring answers on the session bus but has no default collection, so \
                     {lost} until one is created. Open your keyring manager and create a \
                     keyring named Login, or set one as the default"
                ),
            ),
            _ => check(
                NAME,
                DoctorStatus::Warn,
                format!("no keyring answers on the session bus, so {lost}. {advice}"),
            ),
        },
        Err(err) => check(
            NAME,
            DoctorStatus::Warn,
            format!("could not ask the session bus for a keyring ({err}), so {lost}. {advice}"),
        ),
    }
}

/// `clipboard.storage`: whether the history opens with the key in the
/// keyring, how large it is, and what to do when it does not open.
#[must_use]
pub fn clipboard_storage(facts: &ClipboardFacts) -> DoctorCheck {
    const NAME: &str = "clipboard.storage";
    let (status, detail) = match &facts.inspection {
        Ok(inspection) => on_disk(inspection),
        Err(reason) => (
            DoctorStatus::Warn,
            format!(
                "Compass has nowhere to keep clipboard history ({reason}). Set XDG_DATA_HOME or \
                 HOME."
            ),
        ),
    };
    let (status, detail) = match facts.engine.as_ref().map(|engine| &engine.storage) {
        Some(ClipboardStorage::Waiting { reason }) => (
            worse(status, DoctorStatus::Warn),
            format!(
                "{detail} The running engine is waiting for the keyring ({reason}) and tries \
                 again by itself."
            ),
        ),
        Some(ClipboardStorage::Failed { reason, .. }) if status != DoctorStatus::Fail => (
            DoctorStatus::Fail,
            format!(
                "{detail} The running engine could not open the history ({reason}). Restart \
                 Compass once that is fixed."
            ),
        ),
        _ => (status, detail),
    };
    check(NAME, status, detail)
}

/// The verdict from the files and the key alone.
fn on_disk(inspection: &Inspection) -> (DoctorStatus, String) {
    let dir = inspection.dir.display();
    let survey = match &inspection.survey {
        Ok(survey) => survey,
        Err(err) => {
            return (
                DoctorStatus::Warn,
                format!("Could not look at the clipboard history in {dir} ({err})."),
            );
        }
    };
    if survey.part(PartKind::Database).is_none() {
        return match &inspection.key {
            KeyState::Malformed(len) => (DoctorStatus::Fail, malformed(*len)),
            KeyState::Locked | KeyState::Unavailable(_) => (
                DoctorStatus::Warn,
                "There is no clipboard history yet, and Compass cannot start one until a \
                 keyring is available and unlocked (see keyring)."
                    .to_owned(),
            ),
            KeyState::Found(_) | KeyState::Missing => (
                DoctorStatus::Ok,
                format!(
                    "There is no clipboard history yet. Compass starts one in {dir} the first \
                     time it runs with a keyring."
                ),
            ),
        };
    }
    let sizes = sizes(survey);
    match (&inspection.key, &inspection.read) {
        (KeyState::Found(_), Some(Ok(1))) => (
            DoctorStatus::Ok,
            format!(
                "The clipboard history in {dir} opens with the key in the keyring and holds 1 \
                 entry. {sizes}"
            ),
        ),
        (KeyState::Found(_), Some(Ok(entries))) => (
            DoctorStatus::Ok,
            format!(
                "The clipboard history in {dir} opens with the key in the keyring and holds \
                 {entries} entries. {sizes}"
            ),
        ),
        (KeyState::Found(_), Some(Err(ReadError::Unreadable))) => (
            DoctorStatus::Fail,
            format!(
                "The clipboard history in {dir} cannot be read with the key in the keyring. It \
                 was encrypted with another key, most likely one lost when the keyring was \
                 reset or replaced, or the file is damaged. Run `compass clipboard reset` to \
                 move it into a backup and start a new history. {sizes}"
            ),
        ),
        (KeyState::Found(_), Some(Err(ReadError::Busy))) => (
            DoctorStatus::Warn,
            format!(
                "The clipboard history in {dir} is locked by another process, so its entries \
                 were not counted. {sizes}"
            ),
        ),
        (KeyState::Found(_), Some(Err(ReadError::NotInitialised))) => (
            DoctorStatus::Warn,
            format!(
                "The clipboard history in {dir} opens with the key in the keyring but has no \
                 history table yet. Compass creates it when it starts. {sizes}"
            ),
        ),
        (KeyState::Found(_), Some(Err(err))) => (
            DoctorStatus::Fail,
            format!("The clipboard history in {dir} could not be read ({err}). {sizes}"),
        ),
        (KeyState::Found(_), None) => (
            DoctorStatus::Warn,
            format!("The clipboard history in {dir} was not read. {sizes}"),
        ),
        (KeyState::Missing, _) => (
            DoctorStatus::Fail,
            format!(
                "The keyring has no key for the clipboard history in {dir}, so it cannot be \
                 read. If the key is in another keyring, make that keyring the default again. \
                 Otherwise the key is lost: run `compass clipboard reset` to move the history \
                 into a backup and start a new one. {sizes}"
            ),
        ),
        (KeyState::Malformed(len), _) => (DoctorStatus::Fail, malformed(*len)),
        (KeyState::Locked, _) => (
            DoctorStatus::Warn,
            format!(
                "The keyring is locked, so Compass could not check that it opens the clipboard \
                 history in {dir}. Unlock the keyring and run `compass doctor` again. {sizes}"
            ),
        ),
        (KeyState::Unavailable(err), _) => (
            DoctorStatus::Warn,
            format!(
                "No keyring answered ({err}), so Compass could not check that it opens the \
                 clipboard history in {dir} (see keyring). {sizes}"
            ),
        ),
    }
}

fn malformed(len: usize) -> String {
    if len == 1 {
        format!(
            "The keyring entry \"{KEYRING_LABEL}\" is 1 byte long, which is not a key Compass \
             made, so clipboard history is off. Run `compass clipboard reset` to remove that \
             entry and start a new history; any existing history is kept as a backup."
        )
    } else {
        format!(
            "The keyring entry \"{KEYRING_LABEL}\" is {len} bytes long, which is not a key \
             Compass made, so clipboard history is off. Run `compass clipboard reset` to \
             remove that entry and start a new history; any existing history is kept as a \
             backup."
        )
    }
}

/// How much space the history takes, as one sentence.
fn sizes(survey: &Survey) -> String {
    let bytes = |kind| survey.part(kind).map_or(0, |part| part.bytes);
    let database = bytes(PartKind::Database);
    let wal = bytes(PartKind::WriteAheadLog) + bytes(PartKind::SharedMemory);
    let (files, payload) = survey
        .part(PartKind::Payloads)
        .map_or((0, 0), |part| (part.files, part.bytes));
    if files == 1 {
        format!(
            "Its database takes {database} bytes, its WAL files {wal} bytes, and 1 payload \
             file takes {payload} bytes."
        )
    } else {
        format!(
            "Its database takes {database} bytes, its WAL files {wal} bytes, and {files} \
             payload files take {payload} bytes."
        )
    }
}

const fn worse(a: DoctorStatus, b: DoctorStatus) -> DoctorStatus {
    match (a, b) {
        (DoctorStatus::Fail, _) | (_, DoctorStatus::Fail) => DoctorStatus::Fail,
        (DoctorStatus::Warn, _) | (_, DoctorStatus::Warn) => DoctorStatus::Warn,
        (DoctorStatus::Ok, DoctorStatus::Ok) => DoctorStatus::Ok,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::bus::FakeBus;
    use crate::doctor::checks::tests::detail;
    use compass_clipboard::files::Part;
    use compass_wayland::compositor::{Capabilities, Family};
    use std::path::PathBuf;

    fn sway() -> Env {
        Env::from_pairs([("SWAYSOCK", "/run/user/1000/sway-ipc.sock")])
    }

    #[tokio::test]
    async fn an_unlocked_keyring_passes_and_a_locked_one_says_so() {
        let unlocked = FakeBus::new().with_property(
            SECRET_SERVICE_BUS_NAME,
            DEFAULT_COLLECTION_PATH,
            COLLECTION_INTERFACE,
            "Locked",
            "false",
        );
        assert_eq!(keyring(&sway(), &unlocked).await.status, DoctorStatus::Ok);
        let locked = FakeBus::new().with_property(
            SECRET_SERVICE_BUS_NAME,
            DEFAULT_COLLECTION_PATH,
            COLLECTION_INTERFACE,
            "Locked",
            "true",
        );
        let c = keyring(&sway(), &locked).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("is locked"));
    }

    #[tokio::test]
    async fn no_keyring_on_sway_says_what_to_install_and_where_to_start_it() {
        let c = keyring(&sway(), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.contains("no keyring answers"), "{d}");
        assert!(d.contains("gnome-keyring-daemon"), "{d}");
        assert!(d.contains("~/.config/sway/config"), "{d}");
    }

    #[test]
    fn clipboard_history_follows_data_control() {
        let found = |data_control| WaylandFindings {
            family: Family::Wlroots,
            capabilities: Capabilities {
                data_control,
                ..Capabilities::default()
            },
            compositor_ipc: None,
        };
        let c = clipboard_history(&sway(), Some(&found(true)), None);
        assert_eq!(c.status, DoctorStatus::Ok);
        let c = clipboard_history(&sway(), Some(&found(false)), None);
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("while the launcher is open"));
    }

    #[test]
    fn an_x11_session_is_named_as_the_reason() {
        let env = Env::from_pairs([("XDG_SESSION_TYPE", "x11"), ("DISPLAY", ":0")]);
        let c = clipboard_history(&env, None, None);
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("X11 session"), "{}", detail(&c));
    }

    fn engine(backend: Option<ClipboardBackend>, capturing: bool) -> ClipboardStatus {
        ClipboardStatus {
            backend,
            capturing,
            monitoring: true,
            capture_problem: None,
            storage: ClipboardStorage::Open,
        }
    }

    #[test]
    fn a_running_engine_reports_the_backend_it_actually_uses() {
        let wlr = Some(ClipboardBackend::DataControl {
            protocol: Some("zwlr_data_control_manager_v1".into()),
        });
        // The engine's word beats the guess from the session.
        let gnome = Env::from_pairs([("XDG_CURRENT_DESKTOP", "GNOME")]);
        let c = clipboard_history(&gnome, None, Some(&engine(wlr.clone(), true)));
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("zwlr_data_control_manager_v1"));

        let shell = Some(ClipboardBackend::ShellExtension);
        let c = clipboard_history(&gnome, None, Some(&engine(shell.clone(), true)));
        assert!(detail(&c).contains("GNOME Shell extension"));

        let mut gone = engine(shell, false);
        gone.capture_problem = Some("the extension is disabled".into());
        let c = clipboard_history(&gnome, None, Some(&gone));
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("the extension is disabled"));

        let mut paused = engine(wlr, true);
        paused.monitoring = false;
        let c = clipboard_history(&sway(), None, Some(&paused));
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("Resume clipboard"));

        let mut closed = engine(None, false);
        closed.storage = ClipboardStorage::Waiting {
            reason: "locked".into(),
        };
        let c = clipboard_history(&sway(), None, Some(&closed));
        assert!(detail(&c).contains("see clipboard.storage"));
    }

    fn survey(database: bool) -> Survey {
        let dir = PathBuf::from("/home/tester/.local/share/compass");
        let mut parts = Vec::new();
        if database {
            parts.push(Part {
                kind: PartKind::Database,
                path: dir.join("compass-clipboard.db"),
                directory: false,
                bytes: 4096,
                files: 1,
            });
            parts.push(Part {
                kind: PartKind::Payloads,
                path: dir.join("compass-clipboard"),
                directory: true,
                bytes: 120,
                files: 3,
            });
        }
        Survey { dir, parts }
    }

    fn facts(
        database: bool,
        key: KeyState,
        read: Option<Result<u64, ReadError>>,
    ) -> ClipboardFacts {
        ClipboardFacts {
            inspection: Ok(Inspection {
                dir: PathBuf::from("/home/tester/.local/share/compass"),
                survey: Ok(survey(database)),
                key,
                read,
            }),
            engine: None,
        }
    }

    #[test]
    fn a_readable_history_reports_its_entries_and_sizes() {
        let c = clipboard_storage(&facts(true, KeyState::Found([1; 32]), Some(Ok(42))));
        assert_eq!(c.status, DoctorStatus::Ok);
        let d = detail(&c);
        assert!(d.contains("holds 42 entries"), "{d}");
        assert!(d.contains("4096 bytes"), "{d}");
        assert!(d.contains("3 payload files take 120 bytes"), "{d}");
    }

    #[test]
    fn a_key_mismatch_fails_and_names_the_reset() {
        let c = clipboard_storage(&facts(
            true,
            KeyState::Found([1; 32]),
            Some(Err(ReadError::Unreadable)),
        ));
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("compass clipboard reset"));

        let lost = clipboard_storage(&facts(true, KeyState::Missing, None));
        assert_eq!(lost.status, DoctorStatus::Fail);
        assert!(detail(&lost).contains("compass clipboard reset"));

        let malformed = clipboard_storage(&facts(false, KeyState::Malformed(3), None));
        assert_eq!(malformed.status, DoctorStatus::Fail);
        assert!(detail(&malformed).contains("3 bytes long"));
    }

    #[test]
    fn a_locked_keyring_is_a_warning_not_a_verdict_on_the_history() {
        let c = clipboard_storage(&facts(true, KeyState::Locked, None));
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("Unlock the keyring"));
    }

    #[test]
    fn no_history_yet_is_fine() {
        let c = clipboard_storage(&facts(false, KeyState::Missing, None));
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("no clipboard history yet"));
    }

    #[test]
    fn the_running_engine_s_trouble_is_added() {
        let mut waiting = facts(true, KeyState::Found([1; 32]), Some(Ok(1)));
        waiting.engine = Some(ClipboardStatus {
            storage: ClipboardStorage::Waiting {
                reason: "the keyring is locked".into(),
            },
            ..engine(None, false)
        });
        let c = clipboard_storage(&waiting);
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("holds 1 entry."));
        assert!(detail(&c).contains("waiting for the keyring"));

        let mut failed = facts(true, KeyState::Found([1; 32]), Some(Ok(1)));
        failed.engine = Some(ClipboardStatus {
            storage: ClipboardStorage::Failed {
                reason: "disk full".into(),
                reset: false,
            },
            ..engine(None, false)
        });
        assert_eq!(clipboard_storage(&failed).status, DoctorStatus::Fail);
    }
}
