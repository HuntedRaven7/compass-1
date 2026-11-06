//! Root search: the application and root item index, its launch history,
//! and Search Files, with the requests that read and change them.
//!
//! The index and the launch history are one service because every query
//! ranks against both, and a launch or a rescan changes them under the same
//! write lock. Search Files sits beside them as the launcher's other search;
//! its indexer keeps its own state, so only the handle lives here.

use std::sync::Arc;

use compass_core::{AppIndex, Config, FrecencyStore, JsonFrecencyStore, SystemClock};
use compass_ipc::{ErrorKind, ProtocolError, QueryHit, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

/// The index root search ranks, and what orders it.
pub(super) struct Search {
    /// The applications and root items.
    pub(super) index: AppIndex,
    /// Launch history.
    pub(super) frecency: Box<dyn FrecencyStore + Send + Sync>,
    /// How many hits a query answers with. `launcher.max_results` from the
    /// user's config, so the wire honours the same limit the UI would.
    pub(super) max_results: usize,
    /// How many times a directory watch has rescanned the catalog; see
    /// [`Request::CatalogGeneration`].
    pub(super) generation: u64,
    /// Search Files, and the file indexer it supervises.
    pub(super) files: Arc<crate::file_search::FileSearch>,
}

impl Search {
    /// Search over `index`, ranked with `frecency`, before any rescan.
    pub(super) fn new(
        index: AppIndex,
        frecency: Box<dyn FrecencyStore + Send + Sync>,
        max_results: usize,
        files: crate::file_search::FileSearch,
    ) -> Self {
        Self {
            index,
            frecency,
            max_results,
            generation: 0,
            files: Arc::new(files),
        }
    }

    /// Ranks `text` against the index; see [`EngineState::query`].
    #[must_use]
    pub(super) fn query(
        &self,
        text: &str,
        launches: &crate::extension_commands::Launches,
    ) -> Vec<QueryHit> {
        self.index
            .search_root_all(text, Some(self.frecency.as_ref()))
            .into_iter()
            .take(self.max_results)
            .map(|hit| match hit {
                compass_core::RootHit::App(ranked) => app_hit(&ranked),
                compass_core::RootHit::Command {
                    command,
                    match_score,
                } => QueryHit {
                    id: command.id(),
                    title: command.title.to_owned(),
                    subtitle: Some(command.subtitle.to_owned()),
                    score: match_score,
                },
                compass_core::RootHit::Extension {
                    command,
                    match_score,
                } => QueryHit {
                    id: command.id.clone(),
                    title: command.title.clone(),
                    subtitle: Some(
                        launches
                            .subtitle(&command.id)
                            .unwrap_or_else(|| command.extension_title.clone()),
                    ),
                    score: match_score,
                },
                compass_core::RootHit::Script {
                    script,
                    match_score,
                } => QueryHit {
                    id: compass_core::root_items::entrypoint_id(
                        compass_core::script_scan::SCRIPTS_PROVIDER_ID,
                        &script.id,
                    ),
                    title: script.title.clone(),
                    subtitle: Some(script.subtitle.clone()),
                    score: match_score,
                },
                compass_core::RootHit::RhaiScript {
                    script,
                    match_score,
                } => QueryHit {
                    id: script.entrypoint_id(),
                    title: script.title.clone(),
                    subtitle: Some(script.subtitle().to_owned()),
                    score: match_score,
                },
                compass_core::RootHit::Shortcut {
                    shortcut,
                    match_score,
                } => QueryHit {
                    id: compass_core::root_items::entrypoint_id(
                        compass_core::shortcut::SHORTCUTS_PROVIDER_ID,
                        &shortcut.id,
                    ),
                    title: shortcut.name.clone(),
                    subtitle: Some("Shortcut".to_owned()),
                    score: match_score,
                },
            })
            .collect()
    }
}

/// Search Files, started with the engine, as `FileExtension::initialized`
/// starts it, so the index is warm by the time anyone searches it.
pub(super) fn start_file_search(
    config: &Config,
    config_readable: bool,
) -> crate::file_search::FileSearch {
    let file_settings = if config_readable {
        super::settings::file_indexing_settings(config)
    } else {
        Err("the configuration could not be read".to_owned())
    };
    match file_settings {
        Ok(settings) => crate::file_search::FileSearch::start(settings),
        Err(reason) => {
            tracing::warn!(%reason, "file indexing is disabled until its configuration is valid");
            crate::file_search::FileSearch::without_indexer()
        }
    }
}

/// The root search and Search Files requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::Query { text } => {
            let state = state.read().await;
            Response::QueryResults {
                hits: state.query(&text),
            }
        }

        Request::RecordLaunch { key } => {
            let state = Arc::clone(state);
            // JSON persistence is blocking disk work. Keep it off the async
            // executor, and serialize updates through the daemon's store.
            tokio::task::spawn_blocking(move || {
                let mut state = state.blocking_write();
                if state.search.index.get(&key).is_none()
                    && compass_core::commands::by_id(&key).is_none()
                {
                    return Response::Error(ProtocolError::new(
                        ErrorKind::BadRequest,
                        "launch key is neither an application nor a builtin command",
                    ));
                }
                match state.search.frecency.record_launch(&key) {
                    Ok(()) => Response::Ack,
                    Err(error) => {
                        tracing::warn!(%error, "could not persist launch history");
                        Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            "could not persist launch history",
                        ))
                    }
                }
            })
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "launch history task failed");
                Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "launch history task failed",
                ))
            })
        }

        Request::RootItemEdit { id, edit } => {
            let state = Arc::clone(state);
            tokio::task::spawn_blocking(move || edit_root_item(&state, &id, edit))
                .await
                .unwrap_or_else(|err| {
                    Response::Error(ProtocolError::new(
                        ErrorKind::Internal,
                        format!("the root item task failed: {err}"),
                    ))
                })
        }

        Request::SearchFiles { query, category } => {
            let files = Arc::clone(&state.read().await.search.files);
            let category = category
                .as_deref()
                .and_then(compass_core::file_search::category_for_key);
            let search = tokio::task::spawn_blocking(move || files.search(&query, category));
            match tokio::time::timeout(FILE_SEARCH_TIMEOUT, search).await {
                Ok(Ok(Ok(found))) => Response::Files {
                    heading: found.heading.to_owned(),
                    files: found.files,
                },
                Ok(Ok(Err(message))) => {
                    Response::Error(ProtocolError::new(ErrorKind::Unsupported, message))
                }
                Ok(Err(err)) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("file search task failed: {err}"),
                )),
                Err(_) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    "the file indexer did not answer in time",
                )),
            }
        }

        Request::OpenFile { path, reveal } => open_file(state, path, reveal).await,
        Request::CatalogGeneration => Response::CatalogGeneration {
            generation: state.read().await.search.generation,
        },
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a search request",
        )),
    }
}

/// Where launch history lives, under `$XDG_DATA_HOME`.
///
/// Returns `None` when there is no data home to put it in, which is the
/// read-only-session case the caller degrades into an in-memory store for.
fn frecency_path() -> Option<std::path::PathBuf> {
    Some(compass_core::xdg_dirs::data_home()?.join("compass/frecency.json"))
}

/// Opens the launch-history store, degrading to in-memory.
///
/// Not fatal on failure: the launcher works without launch history, just
/// less well ordered, and refusing to start over a corrupt ranking cache
/// would be a worse failure than the one it is reporting.
pub(super) fn open_frecency() -> Box<dyn FrecencyStore + Send + Sync> {
    let clock = std::sync::Arc::new(SystemClock);
    match frecency_path() {
        Some(path) => match JsonFrecencyStore::open(&path) {
            Ok(store) => Box::new(store),
            Err(err) => {
                tracing::warn!(
                    path = %path.display(), error = %err,
                    "launch history unreadable; ranking by match score alone this session"
                );
                Box::new(JsonFrecencyStore::in_memory(clock))
            }
        },
        None => {
            tracing::warn!("no XDG data home; launch history will not persist");
            Box::new(JsonFrecencyStore::in_memory(clock))
        }
    }
}

/// How long a Search Files query may wait on the indexer: under the
/// launcher's own request timeout, so it hears why rather than a timeout.
pub(super) const FILE_SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Opens a file with its default application, or shows it in the file
/// browser, through the same application database extensions open with.
/// Answers an error the window shows when nothing opened it, rather than an
/// acknowledgement for a launch that failed out of sight (#253).
async fn open_file(state: &Arc<RwLock<EngineState>>, path: String, reveal: bool) -> Response {
    let target = std::path::PathBuf::from(&path);
    if !target.is_absolute() || !target.exists() {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no file exists at that path",
        ));
    }
    let apps = crate::extension_apps::EngineApps::new(
        &state.read().await.search.index,
        compass_xdg::mimeapps::Lists::from_environment(),
        tokio::runtime::Handle::current(),
    );
    let opened = if reveal {
        apps.reveal_path(&target).await
    } else {
        apps.open_path(&target).await
    };
    if let Err(reason) = opened {
        tracing::warn!(%reason, path, reveal, "the file was not opened");
        return Response::Error(ProtocolError::new(ErrorKind::Unsupported, reason));
    }
    if !reveal {
        // `OpenFileAction` records the open, so the file tops the empty
        // query next time.
        if let Some(xbel) = compass_xdg::bookmarks::recently_used_path() {
            let recorded = tokio::task::spawn_blocking(move || {
                let mime = compass_xdg::mimeapps::file_mime(&target);
                let now = jiff::Timestamp::now()
                    .strftime("%Y-%m-%dT%H:%M:%S%.6fZ")
                    .to_string();
                compass_xdg::bookmarks::record_access(&xbel, &target, &mime, &now)
            })
            .await;
            if let Ok(Err(error)) = recorded {
                tracing::warn!(%error, "not recording a recent file access");
            }
        }
    }
    Response::Ack
}

/// Applies what the root row's panel changed: forgets the launch history,
/// or writes the configuration and applies it to root search, as the C++
/// root item manager merges it into the user's file and its metadata.
fn edit_root_item(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    edit: compass_ipc::RootItemEdit,
) -> Response {
    use compass_core::root_items::RootEdit;
    let edit = match edit {
        compass_ipc::RootItemEdit::Favorite(favorite) => RootEdit::Favorite(favorite),
        compass_ipc::RootItemEdit::MoveFavorite { down } => RootEdit::MoveFavorite { down },
        compass_ipc::RootItemEdit::Alias(alias) => RootEdit::Alias(alias),
        compass_ipc::RootItemEdit::Disable => RootEdit::Disable,
        compass_ipc::RootItemEdit::ResetRanking => RootEdit::ResetRanking,
        compass_ipc::RootItemEdit::Shortcut(shortcut) => RootEdit::Shortcut(shortcut),
        compass_ipc::RootItemEdit::Enabled(enabled) => RootEdit::Enabled(enabled),
        compass_ipc::RootItemEdit::Fallback(enabled) => RootEdit::Fallback(enabled),
    };
    let mut state = state.blocking_write();
    // Search Files is a fallback by the C++'s id (`files:search`), which
    // names no root item here.
    let known = state.search.index.root(id).is_some()
        || (matches!(edit, RootEdit::Fallback(_))
            && compass_core::commands::fallback(id).is_some());
    if !known {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no root item has that id",
        ));
    }
    if edit == RootEdit::ResetRanking {
        let key = state.search.index.history_key(id);
        return match state.search.frecency.forget(&key) {
            Ok(_) => Response::Ack,
            Err(err) => Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("could not reset the ranking: {err}"),
            )),
        };
    }
    // A file that does not parse is left alone rather than replaced by one
    // holding only this change.
    let mut config = match Config::load() {
        Ok(config) => config,
        Err(err) => {
            return Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("could not read the configuration: {err}"),
            ));
        }
    };
    if config.apply_root_edit(id, &edit) {
        let saved =
            compass_core::config::default_config_path().and_then(|path| config.save_to(path));
        if let Err(err) = saved {
            return Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("could not save the configuration: {err}"),
            ));
        }
    }
    state.search.index.apply_root_config(&config.root_config());
    state.integration.global_shortcuts.reload();
    Response::Ack
}

fn app_hit(ranked: &compass_core::apps::ApplicationRootHit<'_>) -> QueryHit {
    QueryHit {
        // The ENTRYPOINT id, not the desktop key. The protocol
        // documents this field as the "stable identifier of the
        // underlying root item", and `AppIndex` already builds one
        // (`app_root_item` -> `entrypoint_id(APPS_PROVIDER_ID, ...)`);
        // this put `AppItem::key` there instead, so the wire carried
        // `host--byobu.desktop` for the item every other part of the
        // system — and the C++ engine — calls
        // `applications:host--byobu`.
        id: ranked.entrypoint_id.to_owned(),
        title: ranked.item.display_name(),
        subtitle: None,
        // `match_score`, not `score`. `Ranked::score` is the combined
        // value that includes the frecency boost and is not bounded,
        // whereas the wire documents `0..=100` on compass-search's
        // scale. See the ordering note on `query`.
        score: ranked.match_score,
    }
}
