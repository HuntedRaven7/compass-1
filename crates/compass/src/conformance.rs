//! Suite 1's real-extension harness (PLAN §8.2): installed extensions, run
//! headlessly through the engine, one command each, judged on their first
//! frame.
//!
//! `compass conformance` starts an engine of its own on a private socket —
//! with whatever environment it was given, so the caller decides which
//! extensions are installed and where the data lives — and then, for every
//! command in the plan (or the first command of every installed extension),
//! does what the launcher does: fills what the command needs, starts it, and
//! follows its view until it draws something.
//!
//! # What counts as a pass
//!
//! A `view` command passes when a frame with something in it arrives — a
//! list or grid item, a form field, a detail's text, or the extension's own
//! empty view on a list or grid that has stopped loading — within the timeout,
//! and nothing on the way was a crash, a view Compass cannot draw, or a
//! failure toast. A `no-view` command passes when the engine starts it; the
//! runtime never says when one finished, so there is no frame to judge.
//!
//! Every verdict carries the reason, because the question the suite answers
//! is not only "how many" but "what is missing": a command that needs a real
//! API token and shows "401" in a toast failed for a reason that is not the
//! host's, and the report has to say so for anyone to act on it.
//!
//! # Placeholders
//!
//! A required preference or argument that the plan does not supply is filled
//! with a placeholder, and the report lists which. Without that, most of the
//! corpus would stop at "needs an API token" and the harness would measure
//! the store's login walls rather than the host.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use compass_core::extension_commands::ExtensionCommand;
use compass_core::manifest::{ArgumentType, CommandMode, PreferenceKind};
use compass_ipc::{Request, Response, SocketPath};
use serde::{Deserialize, Serialize};

/// What a placeholder text preference or argument is set to.
pub const PLACEHOLDER: &str = "compass-conformance";

/// How long a view waits after its first full frame for a failure toast.
const SETTLE: Duration = Duration::from_secs(2);

/// How long the private engine has to start.
const ENGINE_STARTUP: Duration = Duration::from_secs(30);

/// Which commands to run and with what. Every field is optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// The commands, in order. Empty means the first command of every
    /// installed extension.
    #[serde(default)]
    pub commands: Vec<PlanEntry>,
}

/// One command to run.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEntry {
    /// The extension's directory name, e.g. `store.raycast.translate`.
    pub extension: String,
    /// The command's name; `None` for the extension's first `view` command,
    /// else its first command.
    #[serde(default)]
    pub command: Option<String>,
    /// Preference values to store before the run.
    #[serde(default)]
    pub preferences: serde_json::Map<String, serde_json::Value>,
    /// Argument values to launch with.
    #[serde(default)]
    pub arguments: serde_json::Map<String, serde_json::Value>,
    /// Text to type into a list or grid that searches for itself, once its
    /// first frame arrives: a command whose first frame is empty until
    /// something is typed (a translator, a web search) is judged on what it
    /// draws for this.
    #[serde(default)]
    pub search: Option<String>,
    /// Why this command, for whoever reads the plan; not used.
    #[serde(default)]
    pub note: Option<String>,
}

/// How one run went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// A view drew a frame with something in it.
    Rendered,
    /// A no-view command was started.
    Ran,
    /// The view drew, but only ever an empty frame.
    RenderedEmpty,
    /// A failure toast appeared.
    ErrorToast,
    /// The view has a component Compass cannot draw.
    Unsupported,
    /// The command crashed or ended before drawing.
    Crashed,
    /// The engine refused to run it.
    Refused,
    /// Nothing arrived in time.
    Timeout,
    /// The command is waiting on an OAuth sign-in in the browser, which a
    /// headless run cannot give it.
    NeedsSignIn,
    /// The command is waiting for the person to allow a program on the host
    /// (the host-command broker's Allow Once / Always Allow / Deny), which a
    /// headless run cannot answer.
    NeedsConsent,
    /// The plan names something that is not installed.
    NotInstalled,
}

impl Verdict {
    /// Whether this verdict passes.
    #[must_use]
    pub const fn passes(self) -> bool {
        matches!(self, Self::Rendered | Self::Ran)
    }
}

/// One command's result.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    /// The extension's directory name.
    pub extension: String,
    /// The command's name.
    pub command: String,
    /// The command's title.
    pub title: String,
    /// `view` or `no-view`.
    pub mode: String,
    /// How it went.
    pub verdict: Verdict,
    /// Whether that is a pass.
    pub pass: bool,
    /// Why, in a sentence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The last frame's shape: `list`, `grid`, `detail` or `form`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// How many items, fields or characters the last frame had.
    pub size: usize,
    /// Preferences and arguments given a placeholder.
    pub placeholders: Vec<String>,
    /// From start to verdict.
    pub elapsed_ms: u64,
}

/// The whole run.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Each command, in the order run.
    pub outcomes: Vec<Outcome>,
    /// How many passed.
    pub passed: usize,
    /// How many ran.
    pub total: usize,
    /// Manifests that would not load, and why.
    pub broken_manifests: Vec<(String, String)>,
    /// The private engine's log.
    pub engine_log: PathBuf,
}

impl Report {
    /// A table for a person.
    #[must_use]
    pub fn render_human(&self) -> String {
        let mut out = String::new();
        for o in &self.outcomes {
            let mark = if o.pass { "PASS" } else { "FAIL" };
            out.push_str(&format!(
                "{mark}  {:<14} {}:{}",
                format!("{:?}", o.verdict).to_lowercase(),
                o.extension,
                o.command
            ));
            if let Some(detail) = &o.detail {
                out.push_str(&format!("  — {detail}"));
            }
            out.push('\n');
        }
        for (dir, why) in &self.broken_manifests {
            out.push_str(&format!("BROKEN {dir}: {why}\n"));
        }
        out.push_str(&format!(
            "{} of {} passed; engine log at {}\n",
            self.passed,
            self.total,
            self.engine_log.display()
        ));
        out
    }
}

/// Runs the plan (or every installed extension) and reports.
///
/// # Errors
///
/// The plan will not read, or the private engine will not start.
pub async fn run(plan: Option<&Path>, timeout: Duration) -> Result<Report> {
    let plan: Plan = match plan {
        Some(path) => serde_json::from_str(
            &std::fs::read_to_string(path)
                .with_context(|| format!("reading the plan {}", path.display()))?,
        )
        .with_context(|| format!("parsing the plan {}", path.display()))?,
        None => Plan::default(),
    };
    let scan =
        compass_core::manifest::registry::scan(&compass_core::manifest::registry::search_paths());
    let broken_manifests = scan
        .failed
        .iter()
        .map(|(dir, err)| (dir.display().to_string(), err.to_string()))
        .collect();
    let commands = ExtensionCommand::from_manifests(&scan.extensions);
    let entries = if plan.commands.is_empty() {
        scan.extensions
            .iter()
            .map(|manifest| PlanEntry {
                extension: manifest.id.clone(),
                command: None,
                preferences: serde_json::Map::new(),
                arguments: serde_json::Map::new(),
                search: None,
                note: None,
            })
            .collect()
    } else {
        plan.commands
    };

    let engine = Engine::start().await?;
    let mut outcomes = Vec::with_capacity(entries.len());
    for entry in &entries {
        let outcome = match pick(&commands, entry) {
            Some(command) => run_one(&engine.socket, command, entry, timeout).await,
            None => Outcome {
                extension: entry.extension.clone(),
                command: entry.command.clone().unwrap_or_default(),
                title: String::new(),
                mode: String::new(),
                verdict: Verdict::NotInstalled,
                pass: false,
                detail: Some("no such extension command is installed".to_owned()),
                view: None,
                size: 0,
                placeholders: Vec::new(),
                elapsed_ms: 0,
            },
        };
        tracing::info!(
            extension = outcome.extension,
            command = outcome.command,
            verdict = ?outcome.verdict,
            "conformance"
        );
        outcomes.push(outcome);
    }
    let passed = outcomes.iter().filter(|o| o.pass).count();
    Ok(Report {
        total: outcomes.len(),
        passed,
        outcomes,
        broken_manifests,
        engine_log: engine.log.clone(),
    })
}

/// The command `entry` names: by name, else the extension's first view
/// command, else its first.
fn pick<'a>(commands: &'a [ExtensionCommand], entry: &PlanEntry) -> Option<&'a ExtensionCommand> {
    let mut own = commands
        .iter()
        .filter(|command| command.extension_id == entry.extension);
    match &entry.command {
        Some(name) => own.find(|command| &command.name == name),
        None => {
            let own: Vec<&ExtensionCommand> = own.collect();
            own.iter()
                .find(|command| command.mode == CommandMode::View)
                .or_else(|| own.first())
                .copied()
        }
    }
}

/// The value a required preference gets when the plan has none, or `None`
/// when it has a default or is not required.
fn preference_placeholder(
    preference: &compass_core::manifest::Preference,
) -> Option<serde_json::Value> {
    if !preference.required || preference.default.is_some() {
        return None;
    }
    Some(match &preference.kind {
        PreferenceKind::Checkbox { .. } => serde_json::Value::Bool(false),
        PreferenceKind::Dropdown { options } => options.first().map_or_else(
            || serde_json::Value::from(PLACEHOLDER),
            |option| serde_json::Value::from(option.value.clone()),
        ),
        PreferenceKind::FilePicker { .. } | PreferenceKind::DirectoryPicker { .. } => {
            serde_json::Value::from(std::env::temp_dir().to_string_lossy().into_owned())
        }
        PreferenceKind::TextField
        | PreferenceKind::Password
        | PreferenceKind::AppPicker
        | PreferenceKind::Unknown { .. } => serde_json::Value::from(PLACEHOLDER),
    })
}

/// The value a required argument gets when the plan has none.
fn argument_placeholder(argument: &compass_core::manifest::CommandArgument) -> serde_json::Value {
    match argument.argument_type {
        ArgumentType::Dropdown => argument.data.iter().flatten().next().map_or_else(
            || serde_json::Value::from(PLACEHOLDER),
            |option| serde_json::Value::from(option.value.clone()),
        ),
        ArgumentType::Text | ArgumentType::Password => serde_json::Value::from(PLACEHOLDER),
    }
}

// Over the 150-line limit when it was added (#379): split it, don't grow it.
#[allow(clippy::too_many_lines)]
async fn run_one(
    socket: &SocketPath,
    command: &ExtensionCommand,
    entry: &PlanEntry,
    timeout: Duration,
) -> Outcome {
    let started = Instant::now();
    let mut outcome = Outcome {
        extension: command.extension_id.clone(),
        command: command.name.clone(),
        title: command.title.clone(),
        mode: match command.mode {
            CommandMode::View => "view",
            CommandMode::NoView => "no-view",
        }
        .to_owned(),
        verdict: Verdict::Timeout,
        pass: false,
        detail: None,
        view: None,
        size: 0,
        placeholders: Vec::new(),
        elapsed_ms: 0,
    };
    let finish = |mut outcome: Outcome, verdict: Verdict, detail: Option<String>| {
        outcome.verdict = verdict;
        outcome.pass = verdict.passes();
        outcome.detail = detail;
        outcome.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        outcome
    };

    let mut preferences = entry.preferences.clone();
    for preference in &command.preferences {
        if preferences.contains_key(&preference.name) {
            continue;
        }
        if let Some(value) = preference_placeholder(preference) {
            outcome
                .placeholders
                .push(format!("preference {}", preference.name));
            preferences.insert(preference.name.clone(), value);
        }
    }
    if !preferences.is_empty() {
        let stored = send(
            socket,
            Request::SetExtensionPreferences {
                id: command.id.clone(),
                values_json: serde_json::Value::Object(preferences).to_string(),
            },
        )
        .await;
        if let Err(err) = stored {
            return finish(
                outcome,
                Verdict::Refused,
                Some(format!("could not store its preferences: {err}")),
            );
        }
    }

    let mut arguments = entry.arguments.clone();
    for argument in &command.arguments {
        if argument.required && !arguments.contains_key(&argument.name) {
            outcome
                .placeholders
                .push(format!("argument {}", argument.name));
            arguments.insert(argument.name.clone(), argument_placeholder(argument));
        }
    }
    let arguments_json =
        (!command.arguments.is_empty()).then(|| serde_json::Value::Object(arguments).to_string());

    let answer = send(
        socket,
        Request::RunExtensionCommand {
            id: command.id.clone(),
            arguments_json,
        },
    )
    .await;
    let session = match answer {
        Ok(Response::Ack) => return finish(outcome, Verdict::Ran, None),
        Ok(Response::ExtensionStarted { session }) => session,
        Ok(Response::ExtensionNeedsPreferences { fields, .. }) => {
            let names: Vec<String> = fields
                .iter()
                .filter(|field| field.required)
                .map(|field| field.name.clone())
                .collect();
            return finish(
                outcome,
                Verdict::Refused,
                Some(format!("still needs preferences: {}", names.join(", "))),
            );
        }
        Ok(Response::ExtensionNeedsArguments { .. }) => {
            return finish(
                outcome,
                Verdict::Refused,
                Some("still needs arguments".to_owned()),
            );
        }
        Ok(other) => {
            return finish(
                outcome,
                Verdict::Refused,
                Some(format!("unexpected answer: {other:?}")),
            );
        }
        Err(err) => return finish(outcome, Verdict::Refused, Some(err.to_string())),
    };

    let deadline = started + timeout;
    let mut after = 0;
    let mut full_since: Option<Instant> = None;
    let mut typed = false;
    let mut signing_in = false;
    let mut consent: Option<String> = None;
    let verdict = loop {
        let now = Instant::now();
        if now >= deadline || full_since.is_some_and(|since| now >= since + SETTLE) {
            break if full_since.is_some() {
                (Verdict::Rendered, None)
            } else if let Some(title) = consent.take() {
                (Verdict::NeedsConsent, Some(title))
            } else if signing_in {
                (
                    Verdict::NeedsSignIn,
                    Some("waiting on an OAuth sign-in in the browser".to_owned()),
                )
            } else if outcome.view.is_some() {
                (
                    Verdict::RenderedEmpty,
                    Some("only ever drew an empty frame".to_owned()),
                )
            } else {
                (Verdict::Timeout, Some("drew nothing".to_owned()))
            };
        }
        let wait = deadline
            .min(full_since.map_or(deadline, |since| since + SETTLE))
            .saturating_duration_since(now);
        let answer = tokio::time::timeout(
            wait,
            send(socket, Request::ExtensionView { session, after }),
        )
        .await;
        let Ok(answer) = answer else {
            continue;
        };
        let (version, view_json, problem, ended, toast, alert) = match answer {
            Ok(Response::ExtensionView {
                version,
                view_json,
                problem,
                ended,
                toast,
                alert,
                ..
            }) => (version, view_json, problem, ended, toast, alert),
            Ok(other) => break (Verdict::Crashed, Some(format!("unexpected: {other:?}"))),
            Err(err) => break (Verdict::Crashed, Some(err.to_string())),
        };
        after = version;
        consent = alert
            .filter(|alert| alert.remember_text.is_some())
            .map(|alert| alert.title);
        signing_in = toast.as_ref().is_some_and(|toast| {
            toast
                .title
                .starts_with(crate::extension_runner::SIGN_IN_TOAST)
        });
        if let Some(toast) = toast
            && toast.style == compass_ipc::ExtensionToastStyle::Failure
        {
            break (
                Verdict::ErrorToast,
                Some(format!("{}: {}", toast.title, toast.message)),
            );
        }
        if let Some(problem) = problem {
            break if ended {
                (Verdict::Crashed, Some(problem))
            } else {
                (Verdict::Unsupported, Some(problem))
            };
        }
        if ended {
            break (
                Verdict::Crashed,
                Some("ended before drawing anything".to_owned()),
            );
        }
        if let Some(view) = view_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<compass_extension_api::View>(json).ok())
        {
            if !typed
                && let Some(text) = &entry.search
                && let Some(handler) = search_handler(&view)
            {
                typed = true;
                let args = serde_json::json!([text, 1]).to_string();
                if let Err(err) = send(
                    socket,
                    Request::ExtensionEvent {
                        session,
                        handler: handler.0.clone(),
                        args_json: args,
                    },
                )
                .await
                {
                    break (Verdict::Crashed, Some(format!("typing failed: {err}")));
                }
            }
            let (kind, size) = measure(&view);
            outcome.view = Some(kind.to_owned());
            outcome.size = size;
            if size > 0 && full_since.is_none() {
                full_since = Some(Instant::now());
            }
        }
    };
    let _ = send(socket, Request::CloseExtension { session }).await;
    finish(outcome, verdict.0, verdict.1)
}

/// The handler a list or grid takes its search text by, when it asks for it.
fn search_handler(
    view: &compass_extension_api::View,
) -> Option<&compass_extension_api::action::HandlerId> {
    use compass_extension_api::View;
    let search = match view {
        View::List(list) => &list.search,
        View::Grid(grid) => &grid.search,
        View::Detail(_) | View::Form(_) => return None,
    };
    search.on_change.as_ref()
}

/// A frame's shape and how much is in it.
fn measure(view: &compass_extension_api::View) -> (&'static str, usize) {
    use compass_extension_api::View;
    match view {
        View::List(list) => (
            "list",
            list.sections.iter().map(|s| s.items.len()).sum::<usize>()
                + usize::from(
                    !list.is_loading
                        && list
                            .empty_state
                            .as_ref()
                            .is_some_and(|empty| !empty.title.is_empty()),
                ),
        ),
        View::Grid(grid) => (
            "grid",
            grid.sections.iter().map(|s| s.items.len()).sum::<usize>()
                + usize::from(
                    !grid.is_loading
                        && grid
                            .empty_state
                            .as_ref()
                            .is_some_and(|empty| !empty.title.is_empty()),
                ),
        ),
        View::Detail(detail) => (
            "detail",
            detail.markdown.as_deref().map_or(0, str::len) + detail.metadata.len(),
        ),
        View::Form(form) => ("form", form.items.len()),
    }
}

async fn send(socket: &SocketPath, request: Request) -> Result<Response> {
    crate::ipc::send(socket, request).await
}

/// The harness's own engine, stopped when dropped.
struct Engine {
    child: std::process::Child,
    socket: SocketPath,
    log: PathBuf,
    _dir: tempfile::TempDir,
}

impl Engine {
    async fn start() -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("compass-conformance")
            .tempdir()?;
        let socket = SocketPath::exact(dir.path().join("ipc.sock"));
        let log = std::env::temp_dir().join(format!(
            "compass-conformance-engine-{}.log",
            std::process::id()
        ));
        let output =
            std::fs::File::create(&log).with_context(|| format!("creating {}", log.display()))?;
        let child = std::process::Command::new(std::env::current_exe()?)
            .arg("--socket")
            .arg(socket.as_path())
            .args(["serve", "--no-hotkey"])
            .stdin(std::process::Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(output)
            .spawn()
            .context("starting the engine")?;
        let engine = Self {
            child,
            socket,
            log,
            _dir: dir,
        };
        let deadline = Instant::now() + ENGINE_STARTUP;
        while crate::ipc::ping(&engine.socket).await.is_err() {
            if Instant::now() > deadline {
                bail!(
                    "the engine did not start; its log is at {}",
                    engine.log.display()
                );
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(engine)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(self.child.id().to_string())
            .status();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_core::manifest::{Preference, PreferenceKind};

    fn preference(kind: PreferenceKind, required: bool, default: Option<&str>) -> Preference {
        Preference {
            name: "p".into(),
            title: "P".into(),
            description: String::new(),
            placeholder: String::new(),
            required,
            default: default.map(serde_json::Value::from),
            kind,
        }
    }

    #[test]
    fn only_a_required_preference_without_a_default_gets_a_placeholder() {
        assert_eq!(
            preference_placeholder(&preference(PreferenceKind::Password, true, None)),
            Some(serde_json::Value::from(PLACEHOLDER))
        );
        assert_eq!(
            preference_placeholder(&preference(PreferenceKind::Password, true, Some("x"))),
            None
        );
        assert_eq!(
            preference_placeholder(&preference(PreferenceKind::TextField, false, None)),
            None
        );
        assert_eq!(
            preference_placeholder(&preference(
                PreferenceKind::Checkbox {
                    label: String::new()
                },
                true,
                None
            )),
            Some(serde_json::Value::Bool(false))
        );
    }

    #[test]
    fn a_plan_names_extensions_by_directory_and_rejects_typos() {
        let plan: Plan = serde_json::from_str(
            r#"{"commands": [{"extension": "store.raycast.translate", "command": "translate",
                              "preferences": {"lang": "en"}, "note": "top 1"}]}"#,
        )
        .expect("a plan");
        assert_eq!(plan.commands[0].extension, "store.raycast.translate");
        assert!(
            serde_json::from_str::<Plan>(r#"{"commands": [{"extensoin": "x"}]}"#).is_err(),
            "a misspelt key must not silently run the wrong thing"
        );
    }

    #[test]
    fn an_empty_list_is_empty_and_a_list_with_an_item_is_not() {
        use compass_extension_api::View;
        use compass_extension_api::view::{ListItem, ListSection, ListView};
        let empty = View::List(ListView::default());
        assert_eq!(measure(&empty), ("list", 0));
        let mut list = ListView::default();
        list.sections.push(ListSection {
            items: vec![ListItem::new("a")],
            ..ListSection::default()
        });
        assert_eq!(measure(&View::List(list)), ("list", 1));
    }

    #[test]
    fn a_grid_showing_its_empty_view_has_drawn_something_unless_it_is_loading() {
        use compass_extension_api::View;
        use compass_extension_api::view::{EmptyState, GridView};
        let mut grid = GridView {
            empty_state: Some(EmptyState {
                title: "No downloaded wallpapers".into(),
                ..EmptyState::default()
            }),
            ..GridView::default()
        };
        assert_eq!(measure(&View::Grid(grid.clone())), ("grid", 1));
        grid.is_loading = true;
        assert_eq!(measure(&View::Grid(grid)), ("grid", 0));
    }

    #[test]
    fn only_a_frame_or_a_started_command_passes() {
        assert!(Verdict::Rendered.passes());
        assert!(Verdict::Ran.passes());
        for failing in [
            Verdict::RenderedEmpty,
            Verdict::ErrorToast,
            Verdict::Unsupported,
            Verdict::Crashed,
            Verdict::Refused,
            Verdict::Timeout,
            Verdict::NeedsSignIn,
            Verdict::NeedsConsent,
            Verdict::NotInstalled,
        ] {
            assert!(!failing.passes(), "{failing:?}");
        }
    }
}
