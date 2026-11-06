//! Clipboard history, and the engine's own clipboard: the store the
//! clipboard service records into, its switch and preferences, and the
//! requests that read, change and paste them.
//!
//! The store arrives late (the service opens it once the keyring answers)
//! and may never arrive, so every request here answers without one.

use std::sync::Arc;

use compass_core::Config;
use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

/// Clipboard history and its service's controls.
pub(super) struct Clipboard {
    /// Clipboard history, once [`crate::clipboard_service::run`] has opened
    /// it. `None` until then, and for good when there is no keyring.
    pub(super) store: Option<Arc<crate::clipboard_service::ClipboardStore>>,
    /// The clipboard service's switch and preferences, shared with its
    /// recording loop and eviction timer.
    pub(super) control: Arc<crate::clipboard_service::Control>,
}

impl Clipboard {
    /// No history yet, recording as `settings` say.
    pub(super) fn new(settings: &crate::clipboard_service::Settings) -> Self {
        Self {
            store: None,
            control: Arc::new(crate::clipboard_service::Control::new(settings)),
        }
    }
}

/// The clipboard requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::ClipboardHistory { query, limit } => {
            clipboard_history(state, query, limit, None).await
        }

        Request::ClipboardHistoryOfKind { query, limit, kind } => {
            clipboard_history(state, query, limit, kind).await
        }

        Request::ClipboardDetail { id } => {
            let Some(store) = state.read().await.clipboard.store.clone() else {
                return clipboard_unavailable();
            };
            match tokio::task::spawn_blocking(move || store.detail(&id)).await {
                Ok(Ok(Some(detail))) => Response::ClipboardDetail { detail },
                Ok(Ok(None)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard detail task failed: {err}"),
                )),
            }
        }

        Request::ClipboardSetKeywords { id, keywords } => {
            let Some(store) = state.read().await.clipboard.store.clone() else {
                return clipboard_unavailable();
            };
            match tokio::task::spawn_blocking(move || store.set_keywords(&id, &keywords)).await {
                Ok(Ok(true)) => Response::Ack,
                Ok(Ok(false)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard keywords task failed: {err}"),
                )),
            }
        }

        Request::ClipboardRemoveAll => {
            let (store, control) = {
                let state = state.read().await;
                (state.clipboard.store.clone(), state.clipboard_control())
            };
            let Some(store) = store else {
                return clipboard_unavailable();
            };
            let preserve = control.preserve_tagged();
            match tokio::task::spawn_blocking(move || store.remove_all(preserve)).await {
                Ok(Ok(_)) => Response::Ack,
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard remove-all task failed: {err}"),
                )),
            }
        }

        Request::ClipboardMonitoring { enabled } => {
            let control = state.read().await.clipboard_control();
            if let Some(enabled) = enabled {
                control.set_monitoring(enabled);
                // Kept as the preference, as `toggleMonitoring` patches it,
                // so the choice outlives the engine.
                let saved = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    // A file that does not parse is left alone rather than
                    // replaced by one holding only this choice.
                    let mut config = Config::load()?;
                    config.set_provider_preference(
                        crate::clipboard_service::PROVIDER_ID,
                        "monitoring",
                        serde_json::Value::Bool(enabled),
                    );
                    config.save_to(compass_core::config::default_config_path()?)?;
                    Ok(())
                })
                .await;
                match saved {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => tracing::warn!(error = %err, "monitoring choice not saved"),
                    Err(err) => tracing::warn!(error = %err, "the monitoring save task failed"),
                }
            }
            Response::ClipboardMonitoring {
                supported: control.supported(),
                enabled: control.monitoring(),
            }
        }

        Request::ClipboardContent { id } => {
            let Some(store) = state.read().await.clipboard.store.clone() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    CLIPBOARD_UNAVAILABLE,
                ));
            };
            match tokio::task::spawn_blocking(move || store.content(&id)).await {
                Ok(Ok(Some((mime_type, data)))) => Response::ClipboardContent { mime_type, data },
                Ok(Ok(None)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard content task failed: {err}"),
                )),
            }
        }

        Request::PasteText { text } => paste_text(state, text).await,
        Request::CopyText { text } => copy_text(state, text).await,
        Request::ClipboardStatus => {
            Response::ClipboardStatus(state.read().await.clipboard_control().status())
        }

        Request::ClipboardSetPinned { .. } | Request::ClipboardRemove { .. } => {
            let Some(store) = state.read().await.clipboard.store.clone() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    CLIPBOARD_UNAVAILABLE,
                ));
            };
            let changed = tokio::task::spawn_blocking(move || match request {
                Request::ClipboardSetPinned { id, pinned } => store.set_pinned(&id, pinned),
                Request::ClipboardRemove { id } => store.remove(&id),
                _ => unreachable!("matched above"),
            })
            .await;
            match changed {
                Ok(Ok(true)) => Response::Ack,
                Ok(Ok(false)) => Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no clipboard history entry has that id",
                )),
                Ok(Err(err)) => {
                    Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string()))
                }
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("clipboard update task failed: {err}"),
                )),
            }
        }

        Request::ClipboardPaste { id } => {
            const WHAT: &str = "Pasting";
            if let Some(refused) = crate::paste::no_clipboard(state, WHAT).await {
                return refused;
            }
            let store = state.read().await.clipboard.store.clone();
            let Some(store) = store else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Unsupported,
                    CLIPBOARD_UNAVAILABLE,
                ));
            };
            let (mime_type, data) =
                match tokio::task::spawn_blocking(move || store.content(&id)).await {
                    Ok(Ok(Some(content))) => content,
                    Ok(Ok(None)) => {
                        return Response::Error(ProtocolError::new(
                            ErrorKind::BadRequest,
                            "no clipboard history entry has that id",
                        ));
                    }
                    Ok(Err(err)) => {
                        return Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            err.to_string(),
                        ));
                    }
                    Err(err) => {
                        return Response::Error(ProtocolError::new(
                            ErrorKind::Internal,
                            format!("clipboard content task failed: {err}"),
                        ));
                    }
                };
            let content = compass_shell::ClipboardContent::binary(data, mime_type);
            crate::paste::paste(state, content, WHAT).await
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a clipboard request",
        )),
    }
}

/// `PasteService::pasteContent` for text the engine did not store (the
/// emoji picker's glyph): on the clipboard, then pasted into the window that
/// takes focus ([`crate::paste`]). Where nothing can press the paste, the
/// window copies instead.
async fn paste_text(state: &Arc<RwLock<EngineState>>, text: String) -> Response {
    crate::paste::paste(
        state,
        compass_shell::ClipboardContent::text(text),
        "Pasting",
    )
    .await
}

/// `ClipboardService::copyText` for the window: the text goes on a clipboard
/// the engine owns, so it outlives the window that asked. Over data-control
/// on wlroots, through the Shell extension elsewhere.
async fn copy_text(state: &Arc<RwLock<EngineState>>, text: String) -> Response {
    const WHAT: &str = "Copying";
    if crate::wlroots::detect()
        .await
        .is_some_and(|wlroots| wlroots.capabilities.data_control)
    {
        let offers = text_offers(&text);
        return match tokio::task::spawn_blocking(move || compass_wayland::clipboard::set(offers))
            .await
        {
            Ok(Ok(())) => Response::Ack,
            Ok(Err(err)) => Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("{WHAT} failed: {err}"),
            )),
            Err(err) => Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("{WHAT} failed: {err}"),
            )),
        };
    }
    let Some(shell) = state.read().await.shell_client() else {
        return Response::Error(crate::window_service::no_bus(WHAT));
    };
    match shell
        .set_clipboard(&compass_shell::ClipboardContent::text(text))
        .await
    {
        Ok(()) => Response::Ack,
        Err(err) => Response::Error(crate::window_service::refusal(&err, WHAT)),
    }
}

/// Text as data-control offers: UTF-8 under the types toolkits ask for, so
/// GTK, Qt, terminals and X clients through Xwayland all find one.
fn text_offers(text: &str) -> Vec<compass_wayland::data_control::Offer> {
    ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
        .into_iter()
        .map(|mime_type| compass_wayland::data_control::Offer {
            mime_type: mime_type.to_owned(),
            data: text.as_bytes().to_vec(),
        })
        .collect()
}

/// Why clipboard history answers nothing: almost always a keyring that is
/// missing or locked, which `compass doctor` explains.
const CLIPBOARD_UNAVAILABLE: &str = "clipboard history is unavailable: Compass needs an unlocked \
     keyring to keep it encrypted. Run compass doctor to see what is missing";

fn clipboard_unavailable() -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        CLIPBOARD_UNAVAILABLE,
    ))
}

/// Clipboard history for `query`, of one kind or of every kind.
async fn clipboard_history(
    state: &Arc<RwLock<EngineState>>,
    query: String,
    limit: u32,
    kind: Option<compass_ipc::ClipboardKind>,
) -> Response {
    if limit == 0 {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "a clipboard history request must ask for at least one entry",
        ));
    }
    let Some(store) = state.read().await.clipboard.store.clone() else {
        return clipboard_unavailable();
    };
    // SQLite is blocking I/O; keep it off the executor.
    match tokio::task::spawn_blocking(move || store.history_of_kind(&query, limit, kind)).await {
        Ok(Ok(entries)) => Response::ClipboardHistory { entries },
        Ok(Err(err)) => Response::Error(ProtocolError::new(ErrorKind::Internal, err.to_string())),
        Err(err) => Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!("clipboard history task failed: {err}"),
        )),
    }
}

/// The selected text, read where `getSelectedText` reads it: the primary
/// selection over data-control on a wlroots compositor, else through the
/// Shell extension on GNOME. `None` when nothing is selected or there is
/// nowhere to read from, which a link expands to nothing, as the C++'s does.
pub(super) async fn primary_selection(
    shell: Option<Arc<compass_shell::ShellClient>>,
) -> Option<String> {
    if crate::wlroots::session().is_some_and(|wlroots| wlroots.capabilities.data_control) {
        return tokio::task::spawn_blocking(compass_wayland::clipboard::read_primary_text)
            .await
            .ok()?
            .unwrap_or_else(|error| {
                tracing::info!(%error, "could not read the primary selection");
                None
            });
    }
    match shell?.primary_selection().await {
        Ok(text) => text,
        Err(error) => {
            tracing::info!(%error, "could not read the primary selection");
            None
        }
    }
}
