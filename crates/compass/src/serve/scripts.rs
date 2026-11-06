//! Script commands and Run Terminal Program: what the last scan found, the
//! runs the launcher follows, and the requests that list and run them.

use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;
use super::engine_apps;
use super::media::show_hud;

/// Script commands and programs.
pub(super) struct Scripts {
    /// Script commands, as the last scan found them.
    pub(super) commands: crate::scripts::Scripts,
    /// Script runs the launcher follows.
    pub(super) runs: Arc<crate::scripts::Runs>,
    /// Run Terminal Program's `default-action` preference.
    pub(super) run_program_default: String,
}

impl Scripts {
    /// `commands` with no runs yet.
    pub(super) fn new(commands: crate::scripts::Scripts, run_program_default: String) -> Self {
        Self {
            commands,
            runs: Arc::default(),
            run_program_default,
        }
    }
}

/// The script commands in the directories `config` names and the default
/// ones, scanned.
pub(super) fn scan_commands(config: &compass_core::Config) -> crate::scripts::Scripts {
    let mut scripts = crate::scripts::Scripts::new(
        compass_core::script_scan::scan_directories(
            &crate::scripts::custom_directories(
                config.provider_preferences(crate::scripts::PREFERENCES_PROVIDER_ID),
            ),
            &crate::scripts::default_directories(),
        ),
        crate::shortcuts::data_dir().map(|dir| dir.join(crate::scripts::METADATA_FILE)),
    );
    scripts.rescan();
    scripts
}

/// The script command and program requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::ListScripts => {
            let mut state = state.write().await;
            let state = &mut *state;
            state.scripts.commands.rescan();
            let items = state.scripts.commands.items();
            let scripts = items.iter().map(crate::scripts::entry).collect();
            state.search.index.set_scripts(items);
            Response::Scripts { scripts }
        }
        Request::ScriptIcons => {
            let icons = state.read().await.scripts.commands.icons();
            Response::ScriptIcons { icons }
        }
        Request::RunScript { id, arguments } => run_script(state, &id, &arguments).await,
        Request::ScriptOutput { session } => {
            let runs = Arc::clone(&state.read().await.scripts.runs);
            let Some(run) = runs.get(session) else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no script run has that session",
                ));
            };
            let run = run.lock().map_err(|_| ()).ok();
            match run {
                Some(run) => Response::ScriptOutput {
                    output: String::from_utf8_lossy(&run.output).into_owned(),
                    finished: run.finished,
                    exit_code: run.exit_code,
                    elapsed_ms: u64::try_from(run.elapsed().as_millis()).unwrap_or(u64::MAX),
                },
                None => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "the script run's state is poisoned",
                )),
            }
        }
        Request::StopScript { session } => {
            state.read().await.scripts.runs.stop(session);
            Response::Ack
        }
        Request::ListPrograms => {
            let default_action = state.read().await.scripts.run_program_default.clone();
            let apps = engine_apps(state).await;
            let programs = tokio::task::spawn_blocking(crate::programs::scan)
                .await
                .unwrap_or_default();
            Response::Programs {
                programs,
                terminal: apps.terminal_name(),
                default_action,
            }
        }
        Request::RunProgram {
            argv,
            terminal,
            hold,
        } => run_program(state, argv, terminal, hold).await,
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a scripts request",
        )),
    }
}

/// Runs a command line for Run Terminal Program, as `OpenInTerminalAction`
/// and `OpenRawProgramAction` do.
async fn run_program(
    state: &Arc<RwLock<EngineState>>,
    argv: Vec<String>,
    terminal: bool,
    hold: bool,
) -> Response {
    use compass_worker_host::application_service::{Apps, TerminalOptions};
    if argv.is_empty() || crate::programs::program_path(&argv[0]).is_none() {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "Not a valid executable",
        ));
    }
    if terminal {
        let apps = engine_apps(state).await;
        let options = TerminalOptions {
            hold,
            ..TerminalOptions::default()
        };
        return if apps.run_in_terminal(&argv, &options) {
            Response::Ack
        } else {
            Response::Error(ProtocolError::new(
                ErrorKind::Unsupported,
                "No terminal emulator is installed",
            ))
        };
    }
    match compass_platform_linux::run_command(&argv).await {
        Ok(_) => Response::Ack,
        Err(error) => Response::Error(ProtocolError::new(ErrorKind::Internal, error.to_string())),
    }
}

/// Runs a script command in its mode, as `ScriptExecutorAction::execute`
/// does; see [`crate::scripts`].
async fn run_script(state: &Arc<RwLock<EngineState>>, id: &str, arguments: &[String]) -> Response {
    use compass_core::script_command::OutputMode;
    use compass_worker_host::application_service::{Apps, TerminalOptions};
    let bad = |message: String| Response::Error(ProtocolError::new(ErrorKind::BadRequest, message));
    let (path, runs) = {
        let state = state.read().await;
        let Some(script) = state.scripts.commands.find(id) else {
            return bad("no script command has that id".to_owned());
        };
        (script.path.clone(), Arc::clone(&state.scripts.runs))
    };
    // Read again, as the C++ reloads before every run: the file is the
    // user's and may have changed since the scan.
    let script = match std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|text| compass_core::script_scan::ScriptCommandFile::parse(&path, id, &text))
    {
        Ok(script) => script,
        Err(error) => return bad(format!("Failed to parse script: {error}")),
    };
    let argv = match crate::scripts::command_line(&script, arguments) {
        Ok(argv) => argv,
        Err(error) => return bad(error),
    };
    let cwd = crate::scripts::working_directory(&script);
    let mode = script.data.mode;
    if mode == OutputMode::Terminal {
        let data = &script.data;
        let options = TerminalOptions {
            hold: data.terminal.as_ref().and_then(|t| t.hold).unwrap_or(true),
            app_id: data.terminal.as_ref().and_then(|t| t.app_id.clone()),
            title: data
                .terminal
                .as_ref()
                .and_then(|t| t.title.clone())
                .or_else(|| (!data.title.is_empty()).then(|| data.title.clone())),
            working_directory: data
                .terminal
                .as_ref()
                .and_then(|t| t.working_directory.clone())
                .or_else(|| data.current_directory_path.clone()),
        };
        let apps = engine_apps(state).await;
        return if apps.run_in_terminal(&argv, &options) {
            Response::ScriptStarted { session: None }
        } else {
            Response::Error(ProtocolError::new(
                ErrorKind::Unsupported,
                "Failed to execute script",
            ))
        };
    }
    let (combined, timeout) = if mode == OutputMode::Full {
        (true, None)
    } else {
        (false, Some(crate::scripts::ONE_LINE_TIMEOUT))
    };
    let (session, run, task) = match runs.start(&argv, &cwd, combined, timeout) {
        Ok(started) => started,
        Err(error) => return Response::Error(ProtocolError::new(ErrorKind::Internal, error)),
    };
    match mode {
        OutputMode::Silent => {
            let slot = state.read().await.window_slot();
            tokio::spawn(async move {
                let _ = task.await;
                let (ok, line) = run
                    .lock()
                    .map(|run| (run.exit_code == Some(0), run.first_line()))
                    .unwrap_or_default();
                show_hud(
                    slot,
                    crate::scripts::one_line_message(mode, ok, &line),
                    None,
                )
                .await;
            });
            Response::ScriptStarted { session: None }
        }
        OutputMode::Inline => {
            let state = Arc::clone(state);
            let id = id.to_owned();
            tokio::spawn(async move {
                let _ = task.await;
                let (ok, line) = run
                    .lock()
                    .map(|run| (run.exit_code == Some(0), run.first_line()))
                    .unwrap_or_default();
                if ok {
                    let mut state = state.write().await;
                    let state = &mut *state;
                    state.scripts.commands.save_run(&id, &line);
                    state
                        .search
                        .index
                        .set_scripts(state.scripts.commands.items());
                }
            });
            Response::ScriptStarted {
                session: Some(session),
            }
        }
        _ => Response::ScriptStarted {
            session: Some(session),
        },
    }
}
