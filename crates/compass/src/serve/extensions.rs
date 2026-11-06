//! Extension commands: running them, the views the launcher follows, their
//! preferences and arguments, and the launches handed to the window.

use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, Request, Response, WindowCommand};
use tokio::sync::RwLock;

use super::EngineState;
use super::forward;
use super::rhai::{open_rhai_script, rhai_outcomes};

/// Running extension commands.
#[derive(Default)]
pub(super) struct Extensions {
    /// Running extension view commands, which the launcher follows.
    pub(super) views: Arc<crate::extension_runner::Views>,
    /// Launches extensions asked for, and their commands' subtitle overrides.
    pub(super) launches: Arc<crate::extension_commands::Launches>,
}

/// The extension command requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::RunExtensionCommand { id, arguments_json } => {
            run_extension_command(state, id, arguments_json).await
        }
        Request::ExtensionView { session, after } => extension_view(state, session, after).await,
        Request::ExtensionEvent {
            session,
            handler,
            args_json,
        } => {
            let args: Vec<serde_json::Value> = match serde_json::from_str(&args_json) {
                Ok(args) => args,
                Err(err) => {
                    return Response::Error(ProtocolError::new(
                        ErrorKind::BadRequest,
                        format!("args_json is not a JSON array: {err}"),
                    ));
                }
            };
            let rhai = Arc::clone(&state.read().await.rhai.scripts);
            if rhai.has(session) {
                return match rhai.event(session, &handler, &args).await {
                    Ok(outcomes) => {
                        rhai_outcomes(state, outcomes).await;
                        Response::Ack
                    }
                    Err(reason) => {
                        Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                    }
                };
            }
            let views = Arc::clone(&state.read().await.extensions.views);
            match tokio::task::spawn_blocking(move || views.activate(session, &handler, &args))
                .await
            {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the extension event task failed: {err}"),
                )),
            }
        }
        Request::ExtensionAlertAnswer { session, confirmed } => {
            let rhai = Arc::clone(&state.read().await.rhai.scripts);
            if rhai.has(session) {
                return match rhai.answer(session, confirmed).await {
                    Ok(()) => Response::Ack,
                    Err(reason) => {
                        Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                    }
                };
            }
            let views = Arc::clone(&state.read().await.extensions.views);
            match tokio::task::spawn_blocking(move || views.answer_alert(session, confirmed)).await
            {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the alert answer task failed: {err}"),
                )),
            }
        }
        Request::ExtensionAlertRemember { session } => {
            let views = Arc::clone(&state.read().await.extensions.views);
            match tokio::task::spawn_blocking(move || views.answer_alert_always(session)).await {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the alert answer task failed: {err}"),
                )),
            }
        }
        Request::SetExtensionPreferences { id, values_json } => {
            set_extension_preferences(state, id, values_json).await
        }
        Request::ExtensionLaunchFetch { token } => {
            match state.read().await.extensions.launches.take(token) {
                Some(launch) => match launch.fallback_text {
                    Some(fallback_text) => Response::CommandLaunch {
                        id: launch.id,
                        arguments_json: launch.arguments_json,
                        fallback_text: Some(fallback_text),
                    },
                    None => Response::ExtensionLaunch {
                        id: launch.id,
                        arguments_json: launch.arguments_json,
                        preferences: launch.preferences,
                    },
                },
                None => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no launch is waiting under that token",
                )),
            }
        }
        Request::ExtensionSubtitles => Response::ExtensionSubtitles {
            subtitles: state.read().await.extensions.launches.subtitles(),
        },
        Request::ExtensionPreferences { id } => extension_preferences(state, &id).await,
        Request::ExtensionPop { session } => {
            // A script shows one view; there is nothing above it to pop.
            if state.read().await.rhai.scripts.has(session) {
                return Response::Ack;
            }
            let views = Arc::clone(&state.read().await.extensions.views);
            match tokio::task::spawn_blocking(move || views.pop(session)).await {
                Ok(Ok(())) => Response::Ack,
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the extension pop task failed: {err}"),
                )),
            }
        }
        Request::CloseExtension { session } => {
            let state = state.read().await;
            if !state.rhai.scripts.close(session) {
                state.extensions.views.close(session);
            }
            Response::Ack
        }
        Request::OAuthRedirect { url } => match crate::extension_runner::oauth_redirect(&url) {
            Ok(()) => Response::Ack,
            Err(reason) => Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason)),
        },
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a extensions request",
        )),
    }
}

async fn run_extension_command(
    state: &Arc<RwLock<EngineState>>,
    id: String,
    arguments_json: Option<String>,
) -> Response {
    if let Some(script) = compass_core::rhai_scripts::script_id(&id) {
        return open_rhai_script(state, &id, script).await;
    }
    let Some(command) = state.read().await.search.index.extension(&id).cloned() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no installed extension command has that id",
        ));
    };
    let given = match arguments_json.as_deref().map(serde_json::from_str) {
        None => None,
        Some(Ok(serde_json::Value::Object(given))) => Some(given),
        Some(_) => {
            return Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "the arguments are not a JSON object",
            ));
        }
    };
    let runtime = match crate::extension_runner::Runtime::locate() {
        Ok(runtime) => runtime,
        Err(reason) => return Response::Error(ProtocolError::new(ErrorKind::Unsupported, reason)),
    };
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("compass"))
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "Running extensions needs a data directory, and $XDG_DATA_HOME and $HOME are unset",
        ));
    };
    let storage = extension_storage(&data_dir).await;
    let stored = match storage.clone() {
        Some(storage) => {
            let extension = command.extension_id.clone();
            tokio::task::spawn_blocking(move || {
                crate::extension_runner::load_preferences(&storage, &extension)
            })
            .await
            .unwrap_or_default()
        }
        None => serde_json::Map::new(),
    };
    let preferences = match command.preferences_with(&stored) {
        Ok(preferences) => preferences,
        Err(missing) if storage.is_none() => {
            let names: Vec<&str> = missing.iter().map(|p| p.title.as_str()).collect();
            return Response::Error(ProtocolError::new(
                ErrorKind::Unsupported,
                format!(
                    "{} needs {} set, and without a keyring Compass has nowhere safe to keep it",
                    command.title,
                    names.join(", ")
                ),
            ));
        }
        Err(_) => {
            return Response::ExtensionNeedsPreferences {
                title: command.title.clone(),
                fields: preference_fields(&command, &stored),
            };
        }
    };
    let arguments = match command.arguments_with(given.as_ref()) {
        Ok(arguments) => arguments,
        Err(_) => {
            return Response::ExtensionNeedsArguments {
                title: command.title.clone(),
                fields: argument_fields(&command, given.as_ref()),
            };
        }
    };
    let (files, launches, known) = {
        let state = state.read().await;
        (
            Arc::clone(&state.search.files),
            Arc::clone(&state.extensions.launches),
            crate::extension_commands::Known::all(&state.search.index),
        )
    };
    let deliver: crate::extension_commands::Deliver = {
        let (state, handle) = (Arc::clone(state), tokio::runtime::Handle::current());
        Arc::new(move |token| {
            let state = Arc::clone(&state);
            handle.spawn(async move { deliver_launch(&state, token).await });
        })
    };
    let host = crate::extension_runner::Host {
        storage,
        shell: state.read().await.shell.client.clone(),
        views: Arc::clone(&state.read().await.extensions.views),
        preferences,
        arguments,
        apps: Some(crate::extension_apps::EngineApps::new(
            &state.read().await.search.index,
            compass_xdg::mimeapps::Lists::from_environment(),
            tokio::runtime::Handle::current(),
        )),
        files: Some(files),
        context: launches.take_context(&id),
        commands: Some(crate::extension_commands::EngineCommands::new(
            id.clone(),
            known,
            launches,
            deliver,
        )),
        launcher: Some(launcher_window(state)),
    };
    let started = tokio::task::spawn_blocking(move || {
        crate::extension_runner::start(&runtime, &command, &data_dir, host)
    })
    .await;
    match started {
        Ok(Ok(started)) => {
            let recorded = tokio::task::spawn_blocking({
                let state = Arc::clone(state);
                move || state.blocking_write().search.frecency.record_launch(&id)
            })
            .await;
            if !matches!(recorded, Ok(Ok(()))) {
                tracing::warn!("could not record running an extension command");
            }
            match started {
                crate::extension_runner::Started::Ran => Response::Ack,
                crate::extension_runner::Started::View(session) => {
                    Response::ExtensionStarted { session }
                }
            }
        }
        Ok(Err(reason)) => Response::Error(ProtocolError::new(ErrorKind::Internal, reason)),
        Err(err) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("the extension task failed: {err}"),
        )),
    }
}

/// The launcher window, for a command's serving thread: it blocks that
/// thread, never the engine's, until the window answers.
fn launcher_window(state: &Arc<RwLock<EngineState>>) -> crate::extension_runner::Launcher {
    let (state, handle) = (Arc::clone(state), tokio::runtime::Handle::current());
    crate::extension_runner::Launcher(Arc::new(move |command| {
        let state = Arc::clone(&state);
        handle.block_on(async move {
            let slot = state.read().await.window_slot();
            matches!(
                forward(&slot, command, "tell the launcher").await,
                Response::Ack
            )
        })
    }))
}

/// Hands the launch under `token` to the launcher window. Without a window,
/// a no-view command runs here as it would from the window; a view command
/// has nowhere to be shown, and the launch is dropped with a warning.
async fn deliver_launch(state: &Arc<RwLock<EngineState>>, token: u64) {
    let slot = state.read().await.window_slot();
    let Response::Error(refused) = forward(
        &slot,
        WindowCommand::Launch(token),
        "take an extension's launch",
    )
    .await
    else {
        return;
    };
    let Some(launch) = state.read().await.extensions.launches.take(token) else {
        return;
    };
    let no_view = state
        .read()
        .await
        .search
        .index
        .extension(&launch.id)
        .is_some_and(|command| command.mode == compass_core::manifest::CommandMode::NoView);
    if launch.preferences || !no_view {
        tracing::warn!(
            id = %launch.id, reason = %refused.message,
            "an extension's launch had no launcher window to show it"
        );
        return;
    }
    // Asked over the engine's own socket, as the window would ask: this runs
    // inside a command's launch, and calling `run_extension_command` from
    // here would make that function's future contain itself.
    let socket = state.read().await.socket.clone();
    let asked = async {
        compass_ipc::Client::connect(socket.as_path())
            .await?
            .request(Request::RunExtensionCommand {
                id: launch.id.clone(),
                arguments_json: launch.arguments_json,
            })
            .await
    };
    match asked.await {
        Ok(Response::Error(err)) => {
            tracing::warn!(id = %launch.id, error = %err.message, "an extension's launch failed");
        }
        Err(err) => {
            tracing::warn!(id = %launch.id, error = %err, "an extension's launch failed");
        }
        Ok(_) => {}
    }
}

/// The preferences form for the extension command `id`, without running it.
async fn extension_preferences(state: &Arc<RwLock<EngineState>>, id: &str) -> Response {
    let Some(command) = state.read().await.search.index.extension(id).cloned() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no installed extension command has that id",
        ));
    };
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("compass"))
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "Extension preferences need a data directory, and $XDG_DATA_HOME and $HOME are unset",
        ));
    };
    let Some(storage) = extension_storage(&data_dir).await else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            format!(
                "{}'s preferences need a keyring: without one Compass has nowhere safe to keep them",
                command.title
            ),
        ));
    };
    let extension = command.extension_id.clone();
    let stored = tokio::task::spawn_blocking(move || {
        crate::extension_runner::load_preferences(&storage, &extension)
    })
    .await
    .unwrap_or_default();
    Response::ExtensionNeedsPreferences {
        title: command.title.clone(),
        fields: preference_fields(&command, &stored),
    }
}

/// Every argument `command` takes, as the launcher's form draws it, with what
/// was already entered.
fn argument_fields(
    command: &compass_core::extension_commands::ExtensionCommand,
    given: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<compass_ipc::PreferenceField> {
    use compass_core::manifest::ArgumentType;
    use compass_ipc::PreferenceFieldKind;
    command
        .arguments
        .iter()
        .map(|argument| compass_ipc::PreferenceField {
            name: argument.name.clone(),
            // Raycast labels an argument by its placeholder; it has no title.
            title: if argument.placeholder.is_empty() {
                argument.name.clone()
            } else {
                argument.placeholder.clone()
            },
            description: String::new(),
            placeholder: argument.placeholder.clone(),
            required: argument.required,
            kind: match argument.argument_type {
                ArgumentType::Text => PreferenceFieldKind::Text,
                ArgumentType::Password => PreferenceFieldKind::Password,
                ArgumentType::Dropdown => PreferenceFieldKind::Dropdown {
                    options: argument
                        .data
                        .iter()
                        .flatten()
                        .map(|option| (option.title.clone(), option.value.clone()))
                        .collect(),
                },
            },
            value_json: given
                .and_then(|given| given.get(&argument.name))
                .map(ToString::to_string),
        })
        .collect()
}

/// Every preference `command` reads, as the launcher's form draws it.
fn preference_fields(
    command: &compass_core::extension_commands::ExtensionCommand,
    stored: &serde_json::Map<String, serde_json::Value>,
) -> Vec<compass_ipc::PreferenceField> {
    use compass_core::manifest::PreferenceKind;
    use compass_ipc::PreferenceFieldKind;
    command
        .preferences
        .iter()
        .map(|preference| compass_ipc::PreferenceField {
            name: preference.name.clone(),
            title: preference.title.clone(),
            description: preference.description.clone(),
            placeholder: preference.placeholder.clone(),
            required: preference.required,
            kind: match &preference.kind {
                PreferenceKind::TextField => PreferenceFieldKind::Text,
                PreferenceKind::Password => PreferenceFieldKind::Password,
                PreferenceKind::Checkbox { label } => PreferenceFieldKind::Checkbox {
                    label: label.clone(),
                },
                PreferenceKind::Dropdown { options } => PreferenceFieldKind::Dropdown {
                    options: options
                        .iter()
                        .map(|option| (option.title.clone(), option.value.clone()))
                        .collect(),
                },
                PreferenceKind::AppPicker => PreferenceFieldKind::Unsupported {
                    declared: "appPicker".to_owned(),
                },
                PreferenceKind::FilePicker { .. } => PreferenceFieldKind::Unsupported {
                    declared: "file".to_owned(),
                },
                PreferenceKind::DirectoryPicker { .. } => PreferenceFieldKind::Unsupported {
                    declared: "directory".to_owned(),
                },
                PreferenceKind::Unknown { declared } => PreferenceFieldKind::Unsupported {
                    declared: declared.clone(),
                },
            },
            value_json: stored
                .get(&preference.name)
                .or(preference.default.as_ref())
                .map(ToString::to_string),
        })
        .collect()
}

async fn set_extension_preferences(
    state: &Arc<RwLock<EngineState>>,
    id: String,
    values_json: String,
) -> Response {
    let Some(command) = state.read().await.search.index.extension(&id).cloned() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no installed extension command has that id",
        ));
    };
    let values: serde_json::Map<String, serde_json::Value> =
        match serde_json::from_str(&values_json) {
            Ok(values) => values,
            Err(err) => {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("values_json is not a JSON object: {err}"),
                ));
            }
        };
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("compass"))
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "no data directory",
        ));
    };
    let Some(storage) = extension_storage(&data_dir).await else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "without a keyring Compass has nowhere safe to keep extension preferences",
        ));
    };
    let saved = tokio::task::spawn_blocking(move || {
        crate::extension_runner::save_preferences(&storage, &command.extension_id, &values)
    })
    .await;
    match saved {
        Ok(Ok(())) => Response::Ack,
        Ok(Err(reason)) => Response::Error(ProtocolError::new(ErrorKind::Internal, reason)),
        Err(err) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("the preferences task failed: {err}"),
        )),
    }
}

/// Shortens how long an `ExtensionView` is held, in milliseconds, so a test
/// can hold a view idle past a client's deadline without waiting the whole
/// [`compass_ipc::EXTENSION_VIEW_HOLD`]. Never lengthens it: a client allows
/// [`compass_ipc::long_poll_deadline`] of the constant, not of this.
pub const VIEW_HOLD_ENV: &str = "COMPASS_EXTENSION_VIEW_HOLD_MS";

/// How long an `ExtensionView` is held open waiting for a change.
fn view_hold() -> std::time::Duration {
    static HOLD: std::sync::LazyLock<std::time::Duration> = std::sync::LazyLock::new(|| {
        std::env::var(VIEW_HOLD_ENV)
            .ok()
            .and_then(|ms| ms.trim().parse().ok())
            .map(std::time::Duration::from_millis)
            .map_or(compass_ipc::EXTENSION_VIEW_HOLD, |hold| {
                hold.min(compass_ipc::EXTENSION_VIEW_HOLD)
            })
    });
    *HOLD
}

async fn extension_view(state: &Arc<RwLock<EngineState>>, session: u64, after: u64) -> Response {
    let watched = {
        let state = state.read().await;
        state
            .extensions
            .views
            .watch(session)
            .or_else(|| state.rhai.scripts.watch(session))
    };
    let Some(mut watch) = watched else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no extension view is running with that session",
        ));
    };
    // A timeout is an answer too: the same version, so the launcher asks again.
    let _ = tokio::time::timeout(view_hold(), watch.wait_for(|view| view.version > after)).await;
    let view = watch.borrow().clone();
    Response::ExtensionView {
        version: view.version,
        view_json: view.view,
        problem: view.problem,
        ended: view.ended,
        depth: view.depth,
        alert: view.alert,
        toast: view.toast,
    }
}

/// Extensions' local storage, keyed from Compass's own master key; `None`
/// (and local storage answered "not implemented") without a keyring.
pub(crate) async fn extension_storage(
    data_dir: &std::path::Path,
) -> Option<crate::extension_runner::Storage> {
    let keyring = match crate::clipboard_service::Oo7Store::connect().await {
        Ok(keyring) => keyring,
        Err(err) => {
            tracing::info!(error = %err, "no keyring; extension storage unavailable");
            return None;
        }
    };
    match crate::clipboard_service::master_key(&keyring).await {
        Ok(master) => Some(crate::extension_runner::Storage {
            path: data_dir.join(crate::extension_runner::STORAGE_DATABASE),
            key: compass_crypto::keys::derive_all(&master).database,
        }),
        Err(err) => {
            tracing::info!(error = %err, "no master key; extension storage unavailable");
            None
        }
    }
}
