//! Rhai scripts: the engine's scripts and their open views, the host they
//! reach the desktop through, and the requests that list them and manage
//! their permissions.

use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, Request, Response, WindowCommand};
use tokio::sync::RwLock;

use super::EngineState;
use super::forward;
use super::media::show_hud;
use super::stores::extension_titles;

/// The Rhai scripts and the Shell slot their host reads the clipboard
/// through.
#[derive(Default)]
pub(super) struct Rhai {
    /// Rhai scripts, and their open views.
    pub(super) scripts: Arc<crate::rhai_scripts::RhaiScripts>,
    /// The Shell extension's client for the scripts' clipboard, once
    /// connected.
    pub(super) shell: crate::rhai_host::ShellSlot,
}

impl Rhai {
    /// The user's scripts, loaded and listed in `index`, with a host that
    /// launches through `index`'s applications.
    pub(super) fn from_environment(index: &mut compass_core::AppIndex) -> Self {
        let shell = crate::rhai_host::ShellSlot::default();
        let apps = tokio::runtime::Handle::try_current().ok().map(|handle| {
            crate::extension_apps::EngineApps::new(
                index,
                compass_xdg::mimeapps::Lists::from_environment(),
                handle,
            )
        });
        let host = crate::rhai_host::EngineHost::new(
            Arc::clone(&shell),
            apps,
            compass_core::xdg_dirs::data_home().map(|home| home.join("compass")),
        );
        let rhai = crate::rhai_scripts::RhaiScripts::new(
            crate::rhai_scripts::Config::from_environment(),
            Arc::new(host),
        );
        rhai.create_user_dir();
        index.set_rhai_scripts(rhai.reload());
        Self {
            scripts: Arc::new(rhai),
            shell,
        }
    }
}

/// The Rhai script requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::ListScriptGrants => {
            let rhai = Arc::clone(&state.read().await.rhai.scripts);
            let titles = extension_titles(state).await;
            match tokio::task::spawn_blocking(move || all_grants(&rhai, &titles)).await {
                Ok(grants) => Response::ScriptGrants { grants },
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("reading script permissions failed: {err}"),
                )),
            }
        }
        Request::RevokeScriptGrant { id } => {
            let rhai = Arc::clone(&state.read().await.rhai.scripts);
            let titles = extension_titles(state).await;
            match tokio::task::spawn_blocking(move || {
                let hosts = crate::host_commands::Grants::from_environment();
                match hosts.revoke(&id) {
                    Ok(true) => {
                        tracing::info!(extension = %id, "host-program grants revoked");
                        Ok(())
                    }
                    Ok(false) => rhai.revoke(&id),
                    Err(err) => Err(format!("Compass could not change the permissions: {err}")),
                }
                .map(|()| all_grants(&rhai, &titles))
            })
            .await
            {
                Ok(Ok(grants)) => Response::ScriptGrants { grants },
                Ok(Err(reason)) => {
                    Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("revoking script permissions failed: {err}"),
                )),
            }
        }
        Request::ListRhaiScripts => {
            // A rescan, as `ListScripts` does: the watcher is the fast path,
            // this the one that cannot miss (a full inotify queue, a
            // directory created after start).
            let rhai = Arc::clone(&state.read().await.rhai.scripts);
            let scripts = match tokio::task::spawn_blocking(move || rhai.reload()).await {
                Ok(scripts) => scripts,
                Err(err) => {
                    return Response::Error(ProtocolError::new(
                        ErrorKind::Internal,
                        format!("the script scan failed: {err}"),
                    ));
                }
            };
            let entries = scripts
                .iter()
                .map(|script| compass_ipc::RhaiScriptEntry {
                    id: script.id.clone(),
                    title: script.title.clone(),
                    description: script.description.clone(),
                    icon: script.icon.clone(),
                    keywords: script.keywords.clone(),
                })
                .collect();
            state.write().await.set_rhai_items(scripts);
            Response::RhaiScripts { scripts: entries }
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a rhai request",
        )),
    }
}

/// What Script Permissions lists: the Rhai scripts' grants, then the
/// programs extensions were always allowed to run on the host, each part in
/// id order. Blocking: both read files.
fn all_grants(
    rhai: &crate::rhai_scripts::RhaiScripts,
    titles: &std::collections::BTreeMap<String, String>,
) -> Vec<compass_ipc::ScriptGrantEntry> {
    let mut grants = rhai.grants();
    grants.extend(
        crate::host_commands::Grants::from_environment()
            .entries(|id| titles.get(id).filter(|title| !title.is_empty()).cloned()),
    );
    grants
}

/// Opens the Rhai script `script` (root entry `id`) as a view session.
pub(super) async fn open_rhai_script(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    script: &str,
) -> Response {
    let (rhai, session) = {
        let state = state.read().await;
        (
            Arc::clone(&state.rhai.scripts),
            state.extensions.views.reserve(),
        )
    };
    if let Err(reason) = rhai.open(script, session).await {
        return Response::Error(ProtocolError::new(ErrorKind::BadRequest, reason));
    }
    let recorded = tokio::task::spawn_blocking({
        let (state, id) = (Arc::clone(state), id.to_owned());
        move || state.blocking_write().search.frecency.record_launch(&id)
    })
    .await;
    if !matches!(recorded, Ok(Ok(()))) {
        tracing::warn!("could not record opening a Rhai script");
    }
    Response::ExtensionStarted { session }
}

/// Carries out what a Rhai script's action asked of the engine.
pub(super) async fn rhai_outcomes(
    state: &Arc<RwLock<EngineState>>,
    outcomes: Vec<crate::rhai_scripts::Outcome>,
) {
    for outcome in outcomes {
        if let crate::rhai_scripts::Outcome::Hud(text) = &outcome {
            show_hud(state.read().await.window_slot(), text.clone(), None).await;
        }
        let slot = state.read().await.window_slot();
        if let Response::Error(err) = forward(&slot, WindowCommand::Hide, "hide the launcher").await
        {
            tracing::debug!(message = %err.message, "a script's close reached no window");
        }
    }
}
