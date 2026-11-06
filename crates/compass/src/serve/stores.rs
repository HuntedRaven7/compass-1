//! Installing and removing extensions: the extension stores, the installer
//! of the extensions `compass.json` declares, `vici develop`'s links, the
//! extension template, and Compass's own update check.

use std::sync::Arc;

use compass_core::Config;
use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;
use super::{engine_apps, extension_storage};

/// The extension stores and the update check.
pub(super) struct Stores {
    /// The extension stores.
    pub(super) stores: Arc<crate::stores::Stores>,
    /// Wakes the installer of the extensions `compass.json` declares.
    pub(super) sync: Arc<crate::extension_sync::Trigger>,
    /// The update check.
    pub(super) updates: Arc<crate::updates::Updates>,
}

impl Stores {
    /// The stores, checking for updates through `updates`.
    pub(super) fn new(updates: crate::updates::Updates) -> Self {
        Self {
            stores: Arc::default(),
            sync: Arc::default(),
            updates: Arc::new(updates),
        }
    }
}

/// The store, update and extension template requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::UpdateStatus => {
            let updates = Arc::clone(&state.read().await.stores.updates);
            // Read when asked, so turning the setting off stops the next check.
            let enabled = Config::load()
                .map(|config| config.launcher().check_for_updates())
                .unwrap_or(compass_core::config::DEFAULT_CHECK_FOR_UPDATES);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| {
                    i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
                });
            Response::UpdateStatus {
                current: updates.current().to_owned(),
                available: updates.status(enabled, now).await,
            }
        }
        Request::SkipUpdate { tag } => {
            let updates = Arc::clone(&state.read().await.stores.updates);
            match updates.skip(&tag) {
                Ok(()) => Response::Ack,
                Err(message) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            }
        }
        Request::StoreBrowse { store, query } => {
            let stores = Arc::clone(&state.read().await.stores.stores);
            match stores.browse(store, &query).await {
                Ok(listed) => Response::StoreListing {
                    heading: listed.heading,
                    entries: listed.entries,
                },
                Err(message) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            }
        }
        Request::StoreExtension { store, owner, name } => {
            let stores = Arc::clone(&state.read().await.stores.stores);
            match stores.detail(store, &owner, &name).await {
                Ok(detail) => Response::StoreExtension { detail },
                Err(message) => Response::Error(ProtocolError::new(ErrorKind::BadRequest, message)),
            }
        }
        Request::StoreInstall { store, owner, name } => {
            let stores = Arc::clone(&state.read().await.stores.stores);
            // A Raycast extension the overrides manifest replaces on Linux
            // installs its replacement instead.
            let (store, owner, name) = match (
                store,
                compass_core::raycast_overrides::Manifest::shipped().raycast_redirect(&name),
            ) {
                (compass_ipc::StoreKind::Raycast, Some(redirect))
                    if redirect.store == "vicinae" =>
                {
                    tracing::info!(
                        from = %name, to = %redirect.name, why = %redirect.why,
                        "installing the Linux-capable replacement of a Raycast extension"
                    );
                    (
                        compass_ipc::StoreKind::Vicinae,
                        redirect.author.clone(),
                        redirect.name.clone(),
                    )
                }
                _ => (store, owner, name),
            };
            match stores.install(store, &owner, &name).await {
                Ok((id, title)) => {
                    state.write().await.search.index.rescan_extensions();
                    Response::StoreInstalled { id, title }
                }
                Err(message) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            }
        }
        Request::StoreUninstall { id } => uninstall_extension(state, id).await,
        Request::OpenUrl { url } => {
            use compass_worker_host::application_service::Apps;
            if !crate::stores::is_openable_url(&url) {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "only http and https URLs are opened",
                ));
            }
            let apps = engine_apps(state).await;
            match crate::shortcuts::resolve_app(&apps, compass_core::shortcut::DEFAULT_APP_ID, &url)
            {
                Some(app) => {
                    apps.launch(&app, &url);
                    Response::Ack
                }
                None => Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    format!("No default app to open {url}"),
                )),
            }
        }
        Request::CreateExtension {
            author,
            title,
            description,
            location,
            command_title,
            command_description,
            template,
        } => {
            let form = compass_core::create_extension::Form {
                author,
                title,
                description,
                location,
                command_title,
                command_description,
                template_id: template,
            };
            match tokio::task::spawn_blocking(move || crate::developer::create_extension(&form))
                .await
            {
                Ok(response) => response,
                Err(error) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("creating the extension failed: {error}"),
                )),
            }
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a stores request",
        )),
    }
}

/// Installed extensions' titles, by extension id.
pub(super) async fn extension_titles(
    state: &Arc<RwLock<EngineState>>,
) -> std::collections::BTreeMap<String, String> {
    state
        .read()
        .await
        .search
        .index
        .extensions()
        .iter()
        .map(|command| {
            (
                command.extension_id.clone(),
                command.extension_title.clone(),
            )
        })
        .collect()
}

/// Uninstalls an extension, as `ExtensionRegistry::uninstall` does: its
/// directory, its support directory and its stored data, then root search
/// forgets its commands.
async fn uninstall_extension(state: &Arc<RwLock<EngineState>>, id: String) -> Response {
    let lookup = id.clone();
    let Ok(Some(directory)) =
        tokio::task::spawn_blocking(move || crate::stores::installed_directory(&lookup)).await
    else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            format!("No extension is installed with the id {id}"),
        ));
    };
    let Some(support) = compass_core::manifest::registry::support_directory(&id) else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "Uninstalling an extension needs a data directory",
        ));
    };
    let removed = tokio::task::spawn_blocking(move || {
        compass_core::store_bundle::uninstall(&directory, &support)
    })
    .await
    .unwrap_or_else(|err| Err(format!("the uninstall task failed: {err}")));
    if let Err(reason) = removed {
        return Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("Failed to uninstall extension: {reason}"),
        ));
    }
    clear_extension_storage(&id).await;
    state.write().await.search.index.rescan_extensions();
    tracing::info!(%id, "extension uninstalled");
    Response::Ack
}

/// Clears what an extension kept in local storage, and its preference
/// values, as `m_storage.clearNamespace(id)` does. Only when the storage
/// database exists: an extension that never ran has nothing there, and
/// opening the keyring to find that out would be a prompt for nothing.
async fn clear_extension_storage(id: &str) {
    let Some(data_dir) = compass_core::xdg_dirs::data_home().map(|home| home.join("compass"))
    else {
        return;
    };
    if !data_dir
        .join(crate::extension_runner::STORAGE_DATABASE)
        .is_file()
    {
        return;
    }
    let Some(storage) = extension_storage(&data_dir).await else {
        return;
    };
    let id = id.to_owned();
    let _ = tokio::task::spawn_blocking(move || {
        crate::extension_runner::clear_extension_data(&storage, &id);
    })
    .await;
}

/// `vici develop`'s deeplinks ([`compass_core::develop_link`]): a new or
/// rebuilt extension is rescanned, so it is in root search and its next run
/// uses the new bundle.
pub(super) async fn develop(
    state: &Arc<RwLock<EngineState>>,
    link: &compass_core::develop_link::DevelopLink,
) -> Response {
    use compass_core::develop_link::DevelopAction;
    match link.action {
        DevelopAction::Start | DevelopAction::Refresh => {
            tracing::info!(id = %link.id, action = ?link.action, "an extension under development was rebuilt");
            state.write().await.rescan_extensions();
        }
        DevelopAction::Stop => {
            tracing::info!(id = %link.id, "an extension's development session ended");
        }
    }
    Response::Ack
}
