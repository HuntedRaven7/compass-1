//! The host-command broker: a program an extension asks the engine to run on
//! the host, outside the extension's sandbox, once the person allows it.
//!
//! Raycast's Brew extension runs `brew`. Behind its Landlock policy an
//! extension cannot execute anything under `/home/linuxbrew`, and inside the
//! Flatpak it cannot even see it — and widening the policy for every
//! extension to give one of them Homebrew is not a trade worth making. So the
//! runtime's shim sends `brew` here (`HostCommand/run`), and the engine:
//!
//! 1. refuses a program no extension is known to need: only what
//!    `extensions/raycast-linux-overrides.json` lists
//!    ([`compass_core::raycast_overrides`]), `brew` for every extension;
//! 2. asks the person the first time, per extension and per program, in the
//!    extension's own view: **Allow Once** (until the command closes),
//!    **Always Allow** (kept in [`GRANTS_FILE`], listed and revocable in
//!    Script Permissions), or **Deny**;
//! 3. runs it on the host — through `flatpak-spawn --host` inside the
//!    Flatpak, directly outside it — as `env PATH=… program args…`, so the
//!    program is found on the host's `PATH` and then in the Linuxbrew prefix
//!    ([`LINUXBREW_PREFIX`]) when it is not on it;
//! 4. answers with its exit status and output, or with a sentence that says
//!    why it did not run, which the shim hands the extension as a named error
//!    rather than `ENOENT`.
//!
//! The extension's Landlock policy is never widened: the program runs as the
//! engine's child, not the extension's.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use compass_core::raycast_overrides::Manifest;
use compass_worker_host::host_command_service::{self as service, Outcome, Request};
use compass_worker_host::session::SessionEvents;
use compass_worker_host::tsapi::Deferral;

use crate::extension_runner::Answer;

/// Where "Always Allow" is kept, under `$XDG_CONFIG_HOME/compass/`.
pub const GRANTS_FILE: &str = "host-command-grants.json";

/// Where the commands a person always allowed to run from a link are kept.
/// Separate from [`GRANTS_FILE`] so revoking one set cannot touch the other.
pub const LINK_GRANTS_FILE: &str = "link-command-grants.json";

/// The id Script Permissions uses for the link grants' single, revocable row.
pub const LINK_GRANTS_ID: &str = "deeplink";

/// Homebrew's prefix on Linux.
pub const LINUXBREW_PREFIX: &str = "/home/linuxbrew/.linuxbrew";

/// The host's own programs, for a search path built inside the Flatpak, where
/// the engine's `PATH` is the sandbox's.
const HOST_SYSTEM_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// The capability a grant shows as in Script Permissions.
#[must_use]
pub fn capability(program: &str) -> String {
    format!("host.run:{program}")
}

/// A grant in the consent prompt's words.
#[must_use]
pub fn describe(program: &str) -> String {
    format!("run {program} on your computer, outside the extension sandbox")
}

/// What the person has always allowed, by extension id.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct GrantFile {
    #[serde(default)]
    extensions: BTreeMap<String, BTreeSet<String>>,
}

/// The "Always Allow" grants, in [`GRANTS_FILE`].
#[derive(Debug, Clone, Default)]
pub struct Grants {
    path: Option<PathBuf>,
}

impl Grants {
    /// Grants kept in `path`; `None` keeps nothing.
    #[must_use]
    pub const fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// `$XDG_CONFIG_HOME/compass/host-command-grants.json`, beside the Rhai
    /// scripts' consent file.
    #[must_use]
    pub fn from_environment() -> Self {
        Self::new(
            compass_xdg::xdg_dirs::config_home().map(|home| home.join("compass").join(GRANTS_FILE)),
        )
    }

    /// The grants for commands the person always allowed from a link.
    #[must_use]
    pub fn link_grants() -> Self {
        Self::new(
            compass_xdg::xdg_dirs::config_home()
                .map(|home| home.join("compass").join(LINK_GRANTS_FILE)),
        )
    }

    /// Whether a link may run `id` without asking.
    #[must_use]
    pub fn link_allows(&self, id: &str) -> bool {
        self.allows(LINK_GRANTS_ID, id)
    }

    /// Remember that a link may always run `id`.
    ///
    /// # Errors
    ///
    /// A sentence when the file cannot be written.
    pub fn allow_link(&self, id: &str) -> Result<(), String> {
        self.allow(LINK_GRANTS_ID, id)
    }

    fn load(&self) -> GrantFile {
        let Some(path) = &self.path else {
            return GrantFile::default();
        };
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                tracing::warn!(path = %path.display(), %err, "unreadable host-command grants; asking again");
                GrantFile::default()
            }),
            Err(_) => GrantFile::default(),
        }
    }

    fn save(&self, grants: &GrantFile) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let text = serde_json::to_string_pretty(grants).map_err(|err| err.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let staging = path.with_extension("json.tmp");
        std::fs::write(&staging, text).map_err(|err| err.to_string())?;
        std::fs::rename(&staging, path).map_err(|err| err.to_string())
    }

    /// Whether `extension` may always run `program`.
    #[must_use]
    pub fn allows(&self, extension: &str, program: &str) -> bool {
        self.load()
            .extensions
            .get(extension)
            .is_some_and(|programs| programs.contains(program))
    }

    /// Records that `extension` may always run `program`.
    ///
    /// # Errors
    ///
    /// A sentence when the file cannot be written.
    pub fn allow(&self, extension: &str, program: &str) -> Result<(), String> {
        let mut grants = self.load();
        grants
            .extensions
            .entry(extension.to_owned())
            .or_default()
            .insert(program.to_owned());
        self.save(&grants)
    }

    /// Withdraws everything `extension` was always allowed. `false` when
    /// nothing was recorded for it.
    ///
    /// # Errors
    ///
    /// A sentence when the file cannot be written.
    pub fn revoke(&self, extension: &str) -> Result<bool, String> {
        let mut grants = self.load();
        if grants.extensions.remove(extension).is_none() {
            return Ok(false);
        }
        self.save(&grants)?;
        Ok(true)
    }

    /// The grants as Script Permissions lists them, titled by `title` (the
    /// id when it gives none). In id order.
    #[must_use]
    pub fn entries(
        &self,
        title: impl Fn(&str) -> Option<String>,
    ) -> Vec<compass_ipc::ScriptGrantEntry> {
        self.load()
            .extensions
            .into_iter()
            .filter(|(_, programs)| !programs.is_empty())
            .map(|(id, programs)| compass_ipc::ScriptGrantEntry {
                title: title(&id).unwrap_or_else(|| id.clone()),
                capabilities: programs.iter().map(|p| capability(p)).collect(),
                descriptions: programs.iter().map(|p| describe(p)).collect(),
                id,
            })
            .collect()
    }

    /// The link grants as one row Script Permissions can revoke, with a
    /// capability and description that say what the grant is for.
    #[must_use]
    pub fn link_entries(&self) -> Vec<compass_ipc::ScriptGrantEntry> {
        let file = self.load();
        let Some(commands) = file.extensions.get(LINK_GRANTS_ID) else {
            return Vec::new();
        };
        if commands.is_empty() {
            return Vec::new();
        }
        vec![compass_ipc::ScriptGrantEntry {
            title: "Commands allowed from links".to_owned(),
            capabilities: commands.iter().map(|id| format!("link.run:{id}")).collect(),
            descriptions: commands
                .iter()
                .map(|id| format!("run {id} from a link without asking"))
                .collect(),
            id: LINK_GRANTS_ID.to_owned(),
        }]
    }
}

/// Runs a brokered program on the host.
#[derive(Debug, Clone)]
pub struct Runner {
    /// Through `flatpak-spawn --host`.
    flatpak: bool,
    /// The `PATH` the program is looked up in.
    search_path: String,
}

impl Runner {
    /// For this process: inside the Flatpak or not, and its `PATH` and home.
    #[must_use]
    pub fn detect() -> Self {
        let flatpak = Path::new("/.flatpak-info").exists();
        let path = std::env::var("PATH").ok();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self {
            flatpak,
            search_path: search_path(flatpak, path.as_deref(), home.as_deref()),
        }
    }

    /// A runner that looks programs up in `search_path` and runs them
    /// directly, for tests.
    #[must_use]
    pub fn direct(search_path: impl Into<String>) -> Self {
        Self {
            flatpak: false,
            search_path: search_path.into(),
        }
    }

    /// The command that runs `request`: `env PATH=… [NAME=VALUE…] program
    /// args…`, on the host.
    #[must_use]
    pub fn command(&self, request: &Request) -> Command {
        let mut command = if self.flatpak {
            compass_platform_linux::host_command("env")
        } else {
            Command::new("env")
        };
        command.arg(format!("PATH={}", self.search_path));
        for (name, value) in &request.env {
            if service::is_forwarded_env(name) {
                command.arg(format!("{name}={value}"));
            }
        }
        command.arg(&request.program).args(&request.args);
        command
    }

    /// Runs `request` to the end.
    ///
    /// # Errors
    ///
    /// A sentence when it could not be started, or is not installed.
    pub fn run(&self, request: &Request) -> Result<Outcome, String> {
        if let Some(why) = service::refusal(request) {
            return Err(why);
        }
        let mut child = self
            .command(request)
            .stdin(if request.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("Compass could not start {}: {err}", request.program))?;
        if let (Some(input), Some(mut stdin)) = (&request.input, child.stdin.take()) {
            // A program that exits without reading its input is not an error.
            let _ = stdin.write_all(input.as_bytes());
        }
        let output = child
            .wait_with_output()
            .map_err(|err| format!("{} did not finish: {err}", request.program))?;
        let outcome = Outcome {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        };
        // `env` could not find it: say so, rather than pass on env's words.
        if outcome.exit_code == 127
            && outcome.stderr.starts_with("env:")
            && outcome.stderr.contains(&request.program)
        {
            return Err(format!(
                "{} is not installed on this computer: Compass looked for it on PATH and in the \
                 Linuxbrew prefix, {LINUXBREW_PREFIX}",
                request.program
            ));
        }
        Ok(outcome)
    }
}

/// Where a brokered program is looked for: the engine's own `PATH` outside
/// the Flatpak (the host's system directories inside it, where the engine's
/// is the sandbox's), then Linuxbrew's `bin` and `sbin`, and the per-user
/// prefix under the home directory.
#[must_use]
pub fn search_path(flatpak: bool, path: Option<&str>, home: Option<&Path>) -> String {
    let mut dirs: Vec<String> = if flatpak {
        vec![HOST_SYSTEM_PATH.to_owned()]
    } else {
        path.filter(|path| !path.is_empty()).map_or_else(
            || vec![HOST_SYSTEM_PATH.to_owned()],
            |path| vec![path.to_owned()],
        )
    };
    dirs.push(format!("{LINUXBREW_PREFIX}/bin"));
    dirs.push(format!("{LINUXBREW_PREFIX}/sbin"));
    if let Some(home) = home {
        dirs.push(home.join(".linuxbrew/bin").to_string_lossy().into_owned());
    }
    dirs.join(":")
}

/// The question the person is asked.
#[must_use]
pub fn consent_prompt(title: &str, program: &str) -> compass_ipc::ExtensionAlert {
    compass_ipc::ExtensionAlert {
        title: format!("Allow {title} to run {program}?"),
        message: format!(
            "{title} wants to run {program} on your computer, outside its sandbox and with your \
             permissions.\n\nAllow Once lasts until you close {title}. Always Allow is remembered, \
             and Script Permissions can take it back."
        ),
        confirm_text: "Allow Once".to_owned(),
        cancel_text: "Deny".to_owned(),
        remember_text: Some("Always Allow".to_owned()),
    }
}

/// What is called with the person's answer.
pub type Answered = Box<dyn FnOnce(Answer) + Send>;

/// What asks the person, in the command's view.
pub trait Asker {
    /// Shows `alert`; `answered` gets the person's answer.
    fn ask(&self, alert: compass_ipc::ExtensionAlert, answered: Answered);
}

/// Where the answer to a waiting call goes.
pub trait Settle: Send + Sync {
    /// Answers `deferral` with `value`.
    fn answer(&self, deferral: &Deferral, value: serde_json::Value);
    /// Fails `deferral` with `message`.
    fn fail(&self, deferral: &Deferral, message: &str);
}

impl Settle for SessionEvents {
    fn answer(&self, deferral: &Deferral, value: serde_json::Value) {
        if let Err(err) = Self::answer(self, deferral, value) {
            tracing::warn!(%err, "could not answer a host command; the extension has gone");
        }
    }

    fn fail(&self, deferral: &Deferral, message: &str) {
        if let Err(err) = Self::fail(self, deferral, message) {
            tracing::warn!(%err, "could not refuse a host command; the extension has gone");
        }
    }
}

type Waiting = Vec<(Request, Deferral)>;

/// One command run's broker: its grants, what it was allowed once, and the
/// calls waiting on an answer.
pub struct SessionBroker {
    extension_id: String,
    title: String,
    grants: Grants,
    runner: Runner,
    manifest: &'static Manifest,
    once: Mutex<BTreeSet<String>>,
    waiting: Mutex<BTreeMap<String, Waiting>>,
    mailbox: Mutex<Vec<Request>>,
}

impl std::fmt::Debug for SessionBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionBroker")
            .field("extension_id", &self.extension_id)
            .finish_non_exhaustive()
    }
}

impl service::Broker for &SessionBroker {
    fn request(&self, request: Request, _deferral: &Deferral) {
        self.mailbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }
}

impl SessionBroker {
    /// A broker for one run of `extension_id`, called `title` in prompts.
    #[must_use]
    pub fn new(
        extension_id: impl Into<String>,
        title: impl Into<String>,
        grants: Grants,
        runner: Runner,
        manifest: &'static Manifest,
    ) -> Arc<Self> {
        Arc::new(Self {
            extension_id: extension_id.into(),
            title: title.into(),
            grants,
            runner,
            manifest,
            once: Mutex::default(),
            waiting: Mutex::default(),
            mailbox: Mutex::default(),
        })
    }

    /// The oldest call the service took and nobody dispatched yet.
    pub fn take(&self) -> Option<Request> {
        let mut mailbox = self.mailbox.lock().unwrap_or_else(PoisonError::into_inner);
        (!mailbox.is_empty()).then(|| mailbox.remove(0))
    }

    /// Settles `request` now or once the person has answered: runs it,
    /// asks about it, or refuses it by name. `asker` is `None` for a command
    /// with no view, which has nowhere to ask.
    pub fn dispatch(
        self: &Arc<Self>,
        request: Request,
        deferral: Deferral,
        settle: Arc<dyn Settle>,
        asker: Option<&dyn Asker>,
    ) {
        let program = request.program.clone();
        if !self
            .manifest
            .allows_host_program(&self.extension_id, &program)
        {
            settle.fail(
                &deferral,
                &format!(
                    "{} asked to run {program} on your computer, which Compass does not do for \
                     extensions",
                    self.title
                ),
            );
            return;
        }
        let allowed_once = self
            .once
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&program);
        if allowed_once || self.grants.allows(&self.extension_id, &program) {
            self.run(request, deferral, settle);
            return;
        }
        let Some(asker) = asker else {
            settle.fail(
                &deferral,
                &format!(
                    "{} needs your permission to run {program} on your computer, and a command \
                     with no view cannot ask. Open one of its views to allow it.",
                    self.title
                ),
            );
            return;
        };
        let first = {
            let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            let queue = waiting.entry(program.clone()).or_default();
            queue.push((request, deferral));
            queue.len() == 1
        };
        if !first {
            return;
        }
        let broker = Arc::clone(self);
        asker.ask(
            consent_prompt(&self.title, &program),
            Box::new(move |answer| broker.answered(&program, answer, &settle)),
        );
    }

    fn answered(&self, program: &str, answer: Answer, settle: &Arc<dyn Settle>) {
        let waiting = self
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(program)
            .unwrap_or_default();
        match answer {
            Answer::No => {
                tracing::info!(extension = %self.extension_id, program, "host program denied");
                for (_, deferral) in waiting {
                    settle.fail(
                        &deferral,
                        &format!(
                            "You did not allow {} to run {program} on your computer",
                            self.title
                        ),
                    );
                }
                return;
            }
            Answer::Yes => {
                self.once
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(program.to_owned());
            }
            Answer::Always => {
                if let Err(err) = self.grants.allow(&self.extension_id, program) {
                    tracing::warn!(%err, "could not remember a host-program grant; allowed once");
                }
                self.once
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(program.to_owned());
            }
        }
        tracing::info!(extension = %self.extension_id, program, ?answer, "host program allowed");
        for (request, deferral) in waiting {
            self.run(request, deferral, Arc::clone(settle));
        }
    }

    fn run(&self, request: Request, deferral: Deferral, settle: Arc<dyn Settle>) {
        let runner = self.runner.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("host {}", request.program))
            .spawn(move || match runner.run(&request) {
                Ok(outcome) => settle.answer(&deferral, outcome.to_json()),
                Err(why) => settle.fail(&deferral, &why),
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "could not start a host-command thread");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::mpsc;

    /// A temporary prefix with a fake `brew` in `bin`.
    fn fake_prefix() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).expect("bin");
        let brew = bin.join("brew");
        std::fs::write(
            &brew,
            "#!/bin/sh\ncase \"$1\" in\n  --prefix) echo /fake/prefix ;;\n  \
             env) echo \"$HOMEBREW_NO_AUTO_UPDATE|$LD_PRELOAD|$SUDO_ASKPASS\" ;;\n  \
             cat) cat ;;\n  *) echo \"Error: Unknown command: $1\" >&2; exit 1 ;;\nesac\n",
        )
        .expect("write brew");
        std::fs::set_permissions(&brew, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let path = format!("{}:/usr/bin:/bin", bin.display());
        (dir, path)
    }

    fn request(args: &[&str]) -> Request {
        Request {
            program: "brew".into(),
            args: args.iter().map(ToString::to_string).collect(),
            input: None,
            env: vec![],
        }
    }

    struct Recorded(
        Mutex<Vec<(u64, Result<serde_json::Value, String>)>>,
        mpsc::Sender<()>,
    );

    impl Settle for Recorded {
        fn answer(&self, deferral: &Deferral, value: serde_json::Value) {
            self.0.lock().unwrap().push((deferral.id, Ok(value)));
            let _ = self.1.send(());
        }
        fn fail(&self, deferral: &Deferral, message: &str) {
            self.0
                .lock()
                .unwrap()
                .push((deferral.id, Err(message.to_owned())));
            let _ = self.1.send(());
        }
    }

    fn recorder() -> (Arc<Recorded>, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        (Arc::new(Recorded(Mutex::default(), tx)), rx)
    }

    /// Answers every question with `answer`, and counts them.
    struct Answering(Answer, Mutex<Vec<String>>);

    impl Asker for Answering {
        fn ask(&self, alert: compass_ipc::ExtensionAlert, answered: Answered) {
            self.1.lock().unwrap().push(alert.title);
            answered(self.0);
        }
    }

    fn deferral(id: u64) -> Deferral {
        Deferral {
            id,
            method: "HostCommand/run".into(),
        }
    }

    fn broker(grants: Grants, path: &str) -> Arc<SessionBroker> {
        SessionBroker::new(
            "store.raycast.brew",
            "Brew",
            grants,
            Runner::direct(path),
            Manifest::shipped(),
        )
    }

    #[test]
    fn the_runner_finds_brew_on_the_path_it_is_given_and_passes_only_homebrew_switches() {
        let (_dir, path) = fake_prefix();
        let runner = Runner::direct(path);
        let outcome = runner.run(&request(&["--prefix"])).expect("ran");
        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.stdout, "/fake/prefix\n");

        let mut env = request(&["env"]);
        env.env = vec![
            ("HOMEBREW_NO_AUTO_UPDATE".into(), "1".into()),
            ("LD_PRELOAD".into(), "/evil.so".into()),
            ("SUDO_ASKPASS".into(), "/evil.sh".into()),
        ];
        assert_eq!(runner.run(&env).expect("ran").stdout, "1||\n");

        let mut input = request(&["cat"]);
        input.input = Some("piped".into());
        assert_eq!(runner.run(&input).expect("ran").stdout, "piped");

        let failed = runner.run(&request(&["frobnicate"])).expect("ran");
        assert_eq!(failed.exit_code, 1);
        assert!(failed.stderr.contains("Unknown command"));
    }

    #[test]
    fn a_program_that_is_not_there_is_named() {
        let runner = Runner::direct("/nonexistent");
        let why = runner.run(&request(&["list"])).expect_err("not installed");
        assert!(why.starts_with("brew is not installed"), "{why}");
    }

    #[test]
    fn the_search_path_ends_in_linuxbrew() {
        assert_eq!(
            search_path(false, Some("/usr/bin"), Some(Path::new("/home/me"))),
            "/usr/bin:/home/linuxbrew/.linuxbrew/bin:/home/linuxbrew/.linuxbrew/sbin:/home/me/.linuxbrew/bin"
        );
        assert!(
            search_path(true, Some("/app/bin"), None).starts_with("/usr/local/bin:/usr/bin:/bin:"),
            "inside the Flatpak the sandbox's PATH means nothing on the host"
        );
    }

    #[test]
    fn inside_the_flatpak_it_goes_through_flatpak_spawn() {
        let runner = Runner {
            flatpak: true,
            search_path: "/usr/bin".into(),
        };
        let command = runner.command(&request(&["list"]));
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        if Path::new("/.flatpak-info").exists() {
            assert_eq!(command.get_program(), "flatpak-spawn");
        }
        assert!(args.ends_with(&["PATH=/usr/bin".into(), "brew".into(), "list".into()]));
    }

    #[test]
    fn grants_are_kept_listed_and_revoked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let grants = Grants::new(Some(dir.path().join("compass").join(GRANTS_FILE)));
        assert!(!grants.allows("store.raycast.brew", "brew"));
        grants.allow("store.raycast.brew", "brew").expect("allow");
        assert!(grants.allows("store.raycast.brew", "brew"));
        assert!(!grants.allows("store.raycast.other", "brew"));
        let entries = grants.entries(|id| (id == "store.raycast.brew").then(|| "Brew".to_owned()));
        assert_eq!(
            entries,
            [compass_ipc::ScriptGrantEntry {
                id: "store.raycast.brew".into(),
                title: "Brew".into(),
                capabilities: vec!["host.run:brew".into()],
                descriptions: vec![
                    "run brew on your computer, outside the extension sandbox".into()
                ],
            }]
        );
        let text =
            std::fs::read_to_string(dir.path().join("compass").join(GRANTS_FILE)).expect("file");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).expect("json"),
            serde_json::json!({ "extensions": { "store.raycast.brew": ["brew"] } })
        );
        assert!(grants.revoke("store.raycast.brew").expect("revoke"));
        assert!(
            !grants.revoke("store.raycast.brew").expect("revoke"),
            "nothing left"
        );
        assert!(!grants.allows("store.raycast.brew", "brew"));
    }

    #[test]
    fn always_allow_runs_it_remembers_it_and_does_not_ask_again() {
        let (_dir, path) = fake_prefix();
        let config = tempfile::tempdir().expect("tempdir");
        let grants = Grants::new(Some(config.path().join(GRANTS_FILE)));
        let broker = broker(grants.clone(), &path);
        let (settle, done) = recorder();
        let asker = Answering(Answer::Always, Mutex::default());
        broker.dispatch(
            request(&["--prefix"]),
            deferral(1),
            settle.clone(),
            Some(&asker),
        );
        done.recv().expect("settled");
        assert_eq!(
            settle.0.lock().unwrap()[0],
            (
                1,
                Ok(serde_json::json!({ "exitCode": 0, "stdout": "/fake/prefix\n", "stderr": "" }))
            )
        );
        assert_eq!(*asker.1.lock().unwrap(), ["Allow Brew to run brew?"]);
        assert!(grants.allows("store.raycast.brew", "brew"));

        // A new run of the command: remembered, so nobody is asked.
        let again = self::broker(grants, &path);
        let never = Answering(Answer::No, Mutex::default());
        again.dispatch(
            request(&["--prefix"]),
            deferral(2),
            settle.clone(),
            Some(&never),
        );
        done.recv().expect("settled");
        assert!(never.1.lock().unwrap().is_empty());
        assert!(settle.0.lock().unwrap()[1].1.is_ok());
    }

    #[test]
    fn allow_once_lasts_the_run_and_is_not_kept() {
        let (_dir, path) = fake_prefix();
        let config = tempfile::tempdir().expect("tempdir");
        let grants = Grants::new(Some(config.path().join(GRANTS_FILE)));
        let broker = broker(grants.clone(), &path);
        let (settle, done) = recorder();
        let asker = Answering(Answer::Yes, Mutex::default());
        for id in 1..=2 {
            broker.dispatch(
                request(&["--prefix"]),
                deferral(id),
                settle.clone(),
                Some(&asker),
            );
            done.recv().expect("settled");
        }
        assert_eq!(asker.1.lock().unwrap().len(), 1, "asked once for the run");
        assert!(!grants.allows("store.raycast.brew", "brew"));
        assert!(!config.path().join(GRANTS_FILE).exists());
    }

    #[test]
    fn a_denial_fails_every_waiting_call_by_name() {
        let (_dir, path) = fake_prefix();
        let broker = broker(Grants::new(None), &path);
        let (settle, done) = recorder();
        let asker = Answering(Answer::No, Mutex::default());
        broker.dispatch(
            request(&["list"]),
            deferral(7),
            settle.clone(),
            Some(&asker),
        );
        done.recv().expect("settled");
        assert_eq!(
            settle.0.lock().unwrap()[0],
            (
                7,
                Err("You did not allow Brew to run brew on your computer".into())
            )
        );
    }

    #[test]
    fn calls_made_while_the_person_decides_wait_for_the_one_answer() {
        let (_dir, path) = fake_prefix();
        let broker = broker(Grants::new(None), &path);
        let (settle, done) = recorder();
        /// Keeps the question open until the test answers it.
        struct Later(Mutex<Option<Answered>>);
        impl Asker for Later {
            fn ask(&self, _: compass_ipc::ExtensionAlert, answered: Answered) {
                assert!(
                    self.0.lock().unwrap().replace(answered).is_none(),
                    "asked once"
                );
            }
        }
        let later = Later(Mutex::default());
        broker.dispatch(
            request(&["--prefix"]),
            deferral(1),
            settle.clone(),
            Some(&later),
        );
        broker.dispatch(
            request(&["--prefix"]),
            deferral(2),
            settle.clone(),
            Some(&later),
        );
        assert!(
            settle.0.lock().unwrap().is_empty(),
            "nothing runs before the answer"
        );
        (later.0.lock().unwrap().take().expect("asked"))(Answer::Yes);
        done.recv().expect("first");
        done.recv().expect("second");
        let mut ids: Vec<u64> = settle.0.lock().unwrap().iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2]);
    }

    #[test]
    fn a_program_nobody_listed_or_a_command_without_a_view_is_refused_by_name() {
        let broker = broker(Grants::new(None), "/nonexistent");
        let (settle, _done) = recorder();
        let mut sh = request(&["-c", "true"]);
        sh.program = "sh".into();
        broker.dispatch(sh, deferral(1), settle.clone(), None);
        broker.dispatch(request(&["list"]), deferral(2), settle.clone(), None);
        let settled = settle.0.lock().unwrap();
        assert!(matches!(&settled[0].1, Err(why) if why.contains("does not do for extensions")));
        assert!(matches!(&settled[1].1, Err(why) if why.contains("cannot ask")));
    }
}
