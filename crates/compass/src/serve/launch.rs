//! The C++ CLI's requests beyond the window: `cmd ls`, `cmd launch`,
//! `app launch`, `state open` and `fs query` (IPC v17).
//!
//! Each is the C++ `IpcService` method of the same purpose: `listCommands`,
//! `launchCommand`, `launchApp`, `describe` and `fsQuery`.

use std::sync::Arc;

use compass_core::app_windows::AppIdentity;
use compass_ipc::{
    CommandInfo, ErrorKind, ProtocolError, Response, WindowCommand, WindowInfo, WindowOutcome,
};
use compass_platform::AppLauncher;
use tokio::sync::RwLock;

use super::EngineState;

fn bad_request(message: impl Into<String>) -> Response {
    Response::Error(ProtocolError::new(ErrorKind::BadRequest, message))
}

/// Every root item's id and title, sorted by id, as `listCommands`.
#[must_use]
pub fn commands(index: &compass_core::AppIndex) -> Vec<CommandInfo> {
    let mut commands: Vec<CommandInfo> = index
        .roots()
        .iter()
        .map(|root| CommandInfo {
            id: root.id.clone(),
            name: root.title.clone(),
        })
        .collect();
    commands.sort_by(|a, b| a.id.cmp(&b.id));
    commands
}

/// `launchCommand`: an application is launched here; any other root item
/// goes to the window as a launch, with its arguments checked first.
pub async fn launch_command(
    state: &Arc<RwLock<EngineState>>,
    id: String,
    args: &[String],
    cwd: Option<String>,
    query: Option<String>,
) -> Response {
    // `EntrypointId::fromSerialized`: a provider and an entrypoint.
    if !id
        .split_once(':')
        .is_some_and(|(provider, entrypoint)| !provider.is_empty() && !entrypoint.is_empty())
    {
        return bad_request(format!("Ill-formed command entrypoint: {id}"));
    }
    let (app_key, declared, is_extension) = {
        let state = state.read().await;
        if !state.search.index.roots().iter().any(|root| root.id == id) {
            return bad_request(format!("Unknown command entrypoint: {id}"));
        }
        let app_key = state
            .search
            .index
            .position_by_entrypoint(&id)
            .map(|position| state.search.index.items()[position].key().to_owned());
        let extension = state.search.index.extension(&id);
        (
            app_key,
            extension
                .map(|command| command.arguments.clone())
                .unwrap_or_default(),
            extension.is_some(),
        )
    };
    let arguments = match compass_core::extension_commands::launch_arguments(&id, &declared, args) {
        Ok(arguments) => arguments,
        Err(message) => return bad_request(message),
    };
    // The C++ hands the working directory to the command's launch props,
    // which no Linux builtin reads; kept for the log.
    tracing::debug!(%id, ?cwd, "launching a command from the command line");

    if let Some(key) = app_key {
        return match launch_app(state, &key, &[], true).await {
            Response::AppLaunched { .. } => {
                if let Err(error) = state.write().await.search.frecency.record_launch(&key) {
                    tracing::warn!(%error, "could not record a launch from the command line");
                }
                Response::Ack
            }
            other => other,
        };
    }

    let (launches, slot) = {
        let state = state.read().await;
        (Arc::clone(&state.extensions.launches), state.window_slot())
    };
    let context = (is_extension && query.is_some()).then(|| crate::extension_commands::Context {
        launch_context: serde_json::Value::Null,
        fallback_text: query.clone(),
    });
    let token = launches.open(
        crate::extension_commands::Launch {
            id,
            arguments_json: is_extension.then(|| serde_json::Value::Object(arguments).to_string()),
            preferences: false,
            fallback_text: query,
        },
        context,
    );
    let response = super::forward(&slot, WindowCommand::Launch(token), "launch a command").await;
    if matches!(response, Response::Error(_)) {
        let _ = launches.take(token);
    }
    response
}

/// `launchApp`: focus the application's first window unless a new instance
/// is asked for, else launch it with `args`.
pub async fn launch_app(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    args: &[String],
    new_instance: bool,
) -> Response {
    let found = {
        let state = state.read().await;
        let index = &state.search.index;
        index
            .get(id)
            .or_else(|| {
                index
                    .position_by_entrypoint(id)
                    .map(|position| &index.items()[position])
            })
            .filter(|item| !item.is_action())
            .map(|item| {
                (
                    item.entry().clone(),
                    compass_core::app_service::identity(item),
                )
            })
    };
    let Some((entry, identity)) = found else {
        return bad_request("No app with id");
    };

    if !new_instance && let Some(window) = app_windows(state, &identity).await.into_iter().next() {
        return match super::shell::act_on_window(state, window.id, false).await {
            Response::Error(error) => Response::Error(error),
            _ => Response::AppLaunched {
                focused_window_title: Some(window.title),
            },
        };
    }

    let uris: Vec<&str> = args.iter().map(String::as_str).collect();
    match compass_platform_linux::LinuxLauncher
        .launch(&entry, &uris)
        .await
    {
        Ok(_) => Response::AppLaunched {
            focused_window_title: None,
        },
        Err(error) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("Failed to launch app with id {id}: {error}"),
        )),
    }
}

/// The open windows that belong to `app`, as `findAppWindows` matches them:
/// by class, or by a title that is its name. None when windows cannot be
/// listed here, which makes a launch launch, as the C++'s dummy provider does.
pub async fn app_windows(state: &Arc<RwLock<EngineState>>, app: &AppIdentity) -> Vec<WindowInfo> {
    match super::shell::list_windows(state).await {
        Response::Windows { windows } => windows
            .into_iter()
            .filter(|window| app.matches_window(&window.wm_class, &window.title))
            .collect(),
        _ => Vec::new(),
    }
}

/// `describe`: whether the window is on screen, asked of the window itself.
pub async fn describe_window(state: &Arc<RwLock<EngineState>>) -> Response {
    let slot = state.read().await.window_slot();
    let mut guard = slot.lock().await;
    let Some(link) = guard.as_mut() else {
        return Response::WindowState { open: false };
    };
    match link.push(WindowCommand::Describe).await {
        Ok(outcome) => Response::WindowState {
            open: outcome == WindowOutcome::Shown,
        },
        Err(error) => {
            tracing::info!(%error, "the launcher window went away");
            *guard = None;
            Response::WindowState { open: false }
        }
    }
}

/// The `launch` deeplink (`IpcCommandHandler`'s `launch` command):
/// `toggle=true` hides an open window instead; a provider's id opens the
/// window's provider search view; `<provider>/<entrypoint>` launches that
/// item as `cmd launch` does, with `fallbackText` as its query.
pub async fn open_launch_link(
    state: &Arc<RwLock<EngineState>>,
    url: String,
    link: compass_core::root_items::LaunchLink,
) -> Response {
    if link.toggle
        && matches!(
            describe_window(state).await,
            Response::WindowState { open: true }
        )
    {
        let slot = state.read().await.window_slot();
        return super::forward(&slot, WindowCommand::Hide, "hide a window").await;
    }
    let target = {
        let state = state.read().await;
        link.target(|id| state.search.index.has_provider(id))
    };
    match target {
        Ok(compass_core::root_items::LaunchTarget::Provider(_)) => {
            let slot = state.read().await.window_slot();
            super::forward(&slot, WindowCommand::Deeplink(url), "open a deeplink").await
        }
        Ok(compass_core::root_items::LaunchTarget::Entrypoint(id)) => {
            let known = state.read().await.search.index.root(&id).is_some();
            if !known {
                return bad_request(format!("{id} does not refer to a valid entrypoint"));
            }
            launch_command(state, id, &[], None, link.fallback_text).await
        }
        Err(reason) => bad_request(reason),
    }
}

/// `fsQuery`: the index alone, at most `limit` rows.
pub async fn fs_query(
    state: &Arc<RwLock<EngineState>>,
    query: String,
    limit: u32,
    category: Option<String>,
) -> Response {
    let category = match category.as_deref() {
        None => None,
        Some(key) => match compass_core::file_search::category_for_key(key) {
            Some(category) => Some(category),
            None => return bad_request(format!("Unknown file category: {key}")),
        },
    };
    let files = Arc::clone(&state.read().await.search.files);
    if !files.index_available() {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            crate::file_search::INDEXER_UNAVAILABLE,
        ));
    }
    let limit = i32::try_from(limit).unwrap_or(i32::MAX);
    let search = tokio::task::spawn_blocking(move || files.index_query(&query, limit, category));
    match tokio::time::timeout(super::search::FILE_SEARCH_TIMEOUT, search).await {
        Ok(Ok(rows)) => Response::Files {
            heading: String::new(),
            files: rows
                .into_iter()
                .map(|(path, _, category, _)| crate::file_search::hit(&path, Some(category)))
                .collect(),
        },
        Ok(Err(error)) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("file search task failed: {error}"),
        )),
        Err(_) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            "the file indexer did not answer in time",
        )),
    }
}
