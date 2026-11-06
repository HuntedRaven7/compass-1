//! Shortcuts (quicklinks): the store, and the requests that list, save,
//! remove, expand and open them.

use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;
use super::clipboard::primary_selection;
use super::engine_apps;

/// The shortcut store.
#[derive(Default)]
pub(super) struct Shortcuts {
    /// `None` without a data directory to keep them in, and in tests that
    /// build the state around an index.
    pub(super) store: Option<compass_core::shortcut_service::ShortcutService>,
}

impl Shortcuts {
    /// The user's shortcuts, from the data directory when there is one.
    pub(super) fn open() -> Self {
        Self {
            store: crate::shortcuts::data_dir().map(|dir| crate::shortcuts::open(&dir)),
        }
    }
}

/// The shortcut requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::ListShortcuts => {
            let state = state.read().await;
            match &state.shortcuts.store {
                Some(shortcuts) => shortcuts_response(shortcuts),
                None => shortcuts_unavailable(),
            }
        }
        Request::SaveShortcut {
            id,
            name,
            icon,
            url,
            app,
        } => save_shortcut(state, id, name, icon, url, app).await,
        Request::RemoveShortcut { id } => {
            let mut state = state.write().await;
            let state = &mut *state;
            let Some(shortcuts) = state.shortcuts.store.as_mut() else {
                return shortcuts_unavailable();
            };
            if shortcuts.find_by_id(&id).is_none() {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no shortcut has that id",
                ));
            }
            if !shortcuts.remove(&id) {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "Failed to remove link",
                ));
            }
            state
                .search
                .index
                .set_shortcuts(shortcuts.shortcuts().to_vec());
            shortcuts_response(shortcuts)
        }
        Request::OpenShortcut { id, arguments } => open_shortcut(state, &id, &arguments).await,
        Request::ExpandShortcut { id, arguments } => {
            match expand_shortcut(state, &id, &arguments).await {
                Ok((_, expanded)) => Response::Text { text: expanded },
                Err(response) => response,
            }
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a shortcuts request",
        )),
    }
}

/// The shortcut list as the wire carries it.
fn shortcuts_response(shortcuts: &compass_core::shortcut_service::ShortcutService) -> Response {
    Response::Shortcuts {
        shortcuts: shortcuts
            .shortcuts()
            .iter()
            .map(crate::shortcuts::entry)
            .collect(),
    }
}

fn shortcuts_unavailable() -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        "shortcuts are unavailable: there is no data directory to keep them in",
    ))
}

/// Creates or updates a shortcut, as `ShortcutFormViewHost::submit` does
/// once the form validates: the `default` icon is resolved to what the link
/// currently offers, and stored as that.
async fn save_shortcut(
    state: &Arc<RwLock<EngineState>>,
    id: Option<String>,
    name: String,
    icon: String,
    url: String,
    app: String,
) -> Response {
    use compass_core::shortcut_form::{Existing, Mode, Submission, submit};
    let icon = if icon == compass_core::shortcut_form::DEFAULT_ICON {
        let apps = engine_apps(state).await;
        crate::shortcuts::resolve_default_icon(&apps, &url)
    } else {
        icon
    };
    let mut state = state.write().await;
    let state = &mut *state;
    let Some(shortcuts) = state.shortcuts.store.as_mut() else {
        return shortcuts_unavailable();
    };
    let existing = match &id {
        Some(id) => {
            let Some(found) = shortcuts.find_by_id(id) else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no shortcut has that id",
                ));
            };
            Some(Existing {
                id: found.id.clone(),
                name: found.name.clone(),
                url: found.link.raw.clone(),
                app: found.app.clone(),
                icon: found.icon.clone(),
            })
        }
        None => None,
    };
    let mode = if existing.is_some() {
        Mode::Edit
    } else {
        Mode::Create
    };
    let now = crate::shortcuts::now();
    let saved = match submit(mode, existing.as_ref(), &name, &url, &app, &icon, &icon) {
        Submission::Rejected { errors, toast } => {
            let field = if errors.link.is_some() {
                "link"
            } else if errors.app.is_some() {
                "application"
            } else {
                "icon"
            };
            return Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                format!("{toast}: the {field} is required"),
            ));
        }
        Submission::Update {
            id,
            name,
            icon,
            link,
            app,
            failure,
            ..
        } => shortcuts
            .update(&id, &name, &icon, &link, &app, now)
            .then_some(())
            .ok_or(failure),
        Submission::Create {
            name,
            icon,
            link,
            app,
            failure,
            ..
        } => shortcuts
            .create(&name, &icon, &link, &app, now)
            .then_some(())
            .ok_or(failure),
    };
    if let Err(failure) = saved {
        return Response::Error(ProtocolError::new(ErrorKind::Internal, failure));
    }
    state
        .search
        .index
        .set_shortcuts(shortcuts.shortcuts().to_vec());
    shortcuts_response(shortcuts)
}

/// The shortcut `id` names, expanded with `arguments`, reading the clipboard
/// first when the link asks for it.
async fn expand_shortcut(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    arguments: &[String],
) -> Result<(compass_core::shortcut_service::CachedShortcut, String), Response> {
    let (shortcut, shell) = {
        let state = state.read().await;
        let Some(shortcuts) = &state.shortcuts.store else {
            return Err(shortcuts_unavailable());
        };
        let Some(shortcut) = shortcuts.find_by_id(id) else {
            return Err(Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no shortcut has that id",
            )));
        };
        (shortcut.clone(), state.shell.client.clone())
    };
    let mut reserved = crate::shortcuts::Reserved::default();
    if crate::shortcuts::needs_selection(&shortcut) {
        reserved.selection = primary_selection(shell.clone()).await;
    }
    if crate::shortcuts::needs_clipboard(&shortcut) {
        match shell {
            Some(shell) => match shell.clipboard().await {
                Ok(content) => reserved.clipboard = content.as_text().map(str::to_owned),
                Err(error) => tracing::info!(%error, "could not read the clipboard for a shortcut"),
            },
            None => tracing::info!("no GNOME Shell extension; {{clipboard}} expands to nothing"),
        }
    }
    let expanded = compass_core::shortcut::expand(&shortcut.link, arguments, &reserved);
    Ok((shortcut, expanded))
}

/// Opens a shortcut, as `OpenShortcutAction::execute` does: expand, find the
/// application, launch, and count the visit.
async fn open_shortcut(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    arguments: &[String],
) -> Response {
    use compass_worker_host::application_service::Apps;
    let (shortcut, expanded) = match expand_shortcut(state, id, arguments).await {
        Ok(expanded) => expanded,
        Err(response) => return response,
    };
    let apps = engine_apps(state).await;
    let Some(app) = crate::shortcuts::resolve_app(&apps, &shortcut.app, &expanded) else {
        let message = if shortcut.app == compass_core::shortcut::DEFAULT_APP_ID {
            format!("No default app to open {expanded}")
        } else {
            format!("No app with id {}", shortcut.app)
        };
        return Response::Error(ProtocolError::new(ErrorKind::Unsupported, message));
    };
    apps.launch(&app, &expanded);
    let mut state = state.write().await;
    let state = &mut *state;
    if let Some(shortcuts) = state.shortcuts.store.as_mut()
        && shortcuts.register_visit(id, crate::shortcuts::now())
    {
        state
            .search
            .index
            .set_shortcuts(shortcuts.shortcuts().to_vec());
    }
    Response::Ack
}
