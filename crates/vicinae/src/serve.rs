//! The engine: the thing that actually runs.
//!
//! Everything else in this workspace is a library, and every `vicinae`
//! subcommand until now has been a *client* looking for a server that did not
//! exist. This module is that server.
//!
//! # Headless on purpose
//!
//! There is no `compass-ui` crate yet, so this engine has no window. It serves
//! the requests that do not need one — [`Request::Ping`], [`Request::Query`],
//! [`Request::Doctor`], [`Request::Shutdown`] — and **refuses** the three that
//! do, with [`ErrorKind::Unsupported`] and a message saying why.
//!
//! Refusing matters more than it looks. `Response::Ack` means "the side effect
//! was performed"; answering `Ack` to `Show` when nothing can be shown would
//! make a client that later grows a window unable to tell a working engine from
//! this one, and would make the first UI bring-up debug a lie instead of a gap.
//!
//! What it *does* serve is a genuine vertical slice — index the machine's
//! applications, rank a query against them with frecency, answer over the same
//! socket the C++ engine uses — and all of it is exercisable with no display,
//! which is why it is the part built first.

use std::sync::Arc;

use anyhow::{Context, Result};
use compass_core::{
    AppIndex, Config, FrecencyStore, JsonFrecencyStore, SystemClock, rank_with_frecency,
};
use compass_ipc::{
    ErrorKind, Listener, ProtocolError, QueryHit, Request, Response, SocketPath,
    protocol::PROTOCOL_VERSION,
};
use tokio::sync::RwLock;

use crate::doctor;
use crate::engine::Engine;

/// Where launch history lives, under `$XDG_DATA_HOME`.
///
/// Returns `None` when there is no data home to put it in, which is the
/// read-only-session case the caller degrades into an in-memory store for.
fn frecency_path() -> Option<std::path::PathBuf> {
    Some(compass_core::xdg_dirs::data_home()?.join("vicinae/frecency.json"))
}

/// The engine's state, shared across connections.
///
/// `RwLock` rather than `Mutex` because queries are the hot path and they only
/// read; the write side is frecency updates and, later, reindexing.
pub struct EngineState {
    index: AppIndex,
    frecency: Box<dyn FrecencyStore + Send + Sync>,
    socket: SocketPath,
    /// How many hits a query answers with. `launcher.max_results` from the
    /// user's config, so the wire honours the same limit the UI would.
    max_results: usize,
}

// Hand-written because `dyn FrecencyStore` is not `Debug`, and widening that
// trait to require it would push a formatting concern onto every future store
// implementation for the sake of one log line.
impl std::fmt::Debug for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineState")
            .field("applications", &self.index.len())
            .field("socket", &self.socket)
            .field("max_results", &self.max_results)
            .finish_non_exhaustive()
    }
}

impl EngineState {
    /// Builds the state by indexing this machine.
    ///
    /// A frecency store that cannot be loaded is **not** fatal: the launcher
    /// works without launch history, just less well ordered, and refusing to
    /// start over a corrupt ranking cache would be a worse failure than the one
    /// it is reporting.
    pub fn from_environment(socket: SocketPath) -> Self {
        let index = AppIndex::from_environment();
        tracing::info!(applications = index.len(), "indexed applications");

        // A bad config is reported and then ignored rather than fatal. Refusing
        // to start because `max_results` is misspelled would be a worse outcome
        // than starting with the default and saying so.
        let max_results = match Config::load() {
            Ok(config) => config.launcher().max_results(),
            Err(err) => {
                let fallback = Config::default();
                tracing::warn!(error = %err, "using default configuration");
                fallback.launcher().max_results()
            }
        };

        let frecency = Self::open_frecency();

        Self {
            index,
            frecency,
            socket,
            max_results,
        }
    }

    /// Opens the launch-history store, degrading to in-memory.
    ///
    /// Not fatal on failure: the launcher works without launch history, just
    /// less well ordered, and refusing to start over a corrupt ranking cache
    /// would be a worse failure than the one it is reporting.
    fn open_frecency() -> Box<dyn FrecencyStore + Send + Sync> {
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

    /// Builds state around an already-built index, for tests.
    #[must_use]
    pub fn with_index(
        index: AppIndex,
        frecency: Box<dyn FrecencyStore + Send + Sync>,
        socket: SocketPath,
        max_results: usize,
    ) -> Self {
        Self {
            index,
            frecency,
            socket,
            max_results,
        }
    }

    /// Number of indexed applications.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether nothing was indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Ranks `text` against the index.
    ///
    /// # Hits can be ordered "out of order" by score, and that is correct
    ///
    /// Ordering comes from the *combined* score — match quality plus the
    /// frecency boost — while the score reported on the wire is the match score
    /// alone, because that is what the protocol documents and what a client can
    /// interpret. So a frequently launched item with score 90 legitimately
    /// appears above a never-launched one with 95. A client that sorts by the
    /// reported score would undo the ranking; it should preserve the order it
    /// is given.
    #[must_use]
    pub fn query(&self, text: &str) -> Vec<QueryHit> {
        let items: Vec<_> = self.index.launchable_items().cloned().collect();
        rank_with_frecency(text, &items, |item| item.key(), self.frecency.as_ref())
            .into_iter()
            .take(self.max_results)
            .map(|ranked| QueryHit {
                id: ranked.item.key().to_owned(),
                title: ranked.item.display_name(),
                subtitle: ranked
                    .item
                    .generic_name()
                    .or_else(|| ranked.item.comment())
                    .map(ToOwned::to_owned),
                // `match_score`, not `score`. `Ranked::score` is the combined
                // value that includes the frecency boost and is not bounded,
                // whereas the wire documents `0..=100` on compass-search's
                // scale. See the ordering note on `query`.
                score: ranked.match_score,
            })
            .collect()
    }
}

/// The three requests that need a window this build does not have.
fn no_window(what: &str) -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        format!(
            "this engine is headless and cannot {what}: it has no window yet. \
             Use `--engine cpp` for the launcher UI. Query and doctor work here."
        ),
    ))
}

/// Answers one request.
///
/// Separated from the serve loop so the whole request surface is testable
/// without a socket, and so the serve loop stays about lifecycle.
pub async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::Ping => Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: std::process::id(),
        },

        Request::Query { text } => {
            let state = state.read().await;
            Response::QueryResults {
                hits: state.query(&text),
            }
        }

        Request::Doctor => {
            let (socket, engine) = {
                let state = state.read().await;
                (state.socket.clone(), Engine::Rust)
            };
            Response::DoctorReport {
                checks: doctor::run_on_this_machine(&socket, engine).await.checks,
            }
        }

        // Handled by the serve loop, which owns the shutdown signal; reaching
        // here means the loop did not intercept it.
        Request::Shutdown => Response::ShuttingDown,

        Request::Toggle => no_window("toggle a window"),
        Request::Show => no_window("show a window"),
        Request::Hide => no_window("hide a window"),
    }
}

/// Binds the socket and serves until shutdown.
///
/// Returns when a client sends [`Request::Shutdown`] or the process is asked to
/// terminate. The socket file is removed on the way out by [`Listener`]'s drop.
pub async fn run(socket: &SocketPath) -> Result<()> {
    let listener = Listener::bind(socket.as_path())
        .await
        .with_context(|| format!("binding the engine socket at {socket}"))?;

    let state = Arc::new(RwLock::new(EngineState::from_environment(socket.clone())));
    tracing::info!(socket = %socket, "engine listening");

    // A channel rather than a flag: `Shutdown` must be answered *before* the
    // loop stops, or the client sees its connection drop and reports a crash
    // instead of a clean stop.
    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::channel::<()>(1);

    let serving = {
        let state = Arc::clone(&state);
        listener.serve_with_shutdown(
            move |request| {
                let state = Arc::clone(&state);
                let stop_tx = stop_tx.clone();
                async move {
                    if matches!(request, Request::Shutdown) {
                        tracing::info!("shutdown requested by a client");
                        // Buffered, so the response is written first and this
                        // never blocks the handler.
                        let _ = stop_tx.try_send(());
                        return Response::ShuttingDown;
                    }
                    handle(&state, request).await
                }
            },
            async move {
                let _ = stop_rx.recv().await;
            },
        )
    };

    serving.await.context("serving the engine socket")?;
    tracing::info!("engine stopped");
    Ok(())
}
