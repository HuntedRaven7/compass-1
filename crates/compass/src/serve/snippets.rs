//! Snippets: the store, keyword expansion and its input server, and the
//! requests that list, save, expand and paste them.

use std::sync::Arc;

use compass_core::Config;
use compass_ipc::{ErrorKind, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

/// The snippet store and its keyword expansion.
#[derive(Default)]
pub(super) struct Snippets {
    /// `None` on the same terms as [`super::shortcuts::Shortcuts::store`].
    pub(super) store: Option<compass_core::snippet_store::SnippetStore>,
    /// Snippet keyword expansion and its input server, once started.
    pub(super) expander: Option<Arc<crate::snippet_expansion::Expander>>,
}

impl Snippets {
    /// The user's snippets, from the data directory when there is one.
    pub(super) fn open() -> Self {
        Self {
            store: crate::shortcuts::data_dir().map(|dir| crate::snippets::open(&dir)),
            expander: None,
        }
    }
}

/// The snippet requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::ListSnippets => {
            let state = state.read().await;
            match &state.snippets.store {
                Some(snippets) => snippets_response(snippets),
                None => snippets_unavailable(),
            }
        }
        Request::SaveSnippet {
            id,
            name,
            text,
            keyword,
            word,
            apps,
        } => {
            let response = save_snippet(state, id, name, text, keyword, word, apps).await;
            sync_keywords(state).await;
            response
        }
        Request::RemoveSnippet { id } => {
            let response = {
                let mut state = state.write().await;
                let Some(snippets) = state.snippets.store.as_mut() else {
                    return snippets_unavailable();
                };
                match snippets.remove(&id) {
                    Ok(_) => snippets_response(snippets),
                    Err(error @ compass_core::snippet_store::Error::NoSuchSnippet) => {
                        Response::Error(ProtocolError::new(
                            ErrorKind::BadRequest,
                            error.to_string(),
                        ))
                    }
                    Err(error) => {
                        Response::Error(ProtocolError::new(ErrorKind::Internal, error.to_string()))
                    }
                }
            };
            sync_keywords(state).await;
            response
        }
        Request::InputServerStatus => input_server(state, None).await,
        Request::SetInputServerEnabled { enabled } => input_server(state, Some(enabled)).await,
        Request::PreviewSnippet { id, arguments } => {
            match expand_snippet(state, &id, &arguments, false).await {
                Ok(text) => Response::Text { text },
                Err(response) => response,
            }
        }
        Request::ExpandSnippet { id, arguments } => {
            match expand_snippet(state, &id, &arguments, true).await {
                Ok(text) => Response::Text { text },
                Err(response) => response,
            }
        }
        Request::PasteSnippet { id, arguments } => {
            const WHAT: &str = "Pasting";
            let text = match expand_snippet(state, &id, &arguments, true).await {
                Ok(text) => text,
                Err(response) => return response,
            };
            crate::paste::paste(state, compass_shell::ClipboardContent::text(text), WHAT).await
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a snippets request",
        )),
    }
}

/// The snippet list as the wire carries it.
fn snippets_response(snippets: &compass_core::snippet_store::SnippetStore) -> Response {
    Response::Snippets {
        snippets: snippets
            .snippets()
            .iter()
            .map(crate::snippets::entry)
            .collect(),
    }
}

fn snippets_unavailable() -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        "snippets are unavailable: there is no data directory to keep them in",
    ))
}

/// Creates or updates a text snippet, as `SnippetFormViewHost::submit` does:
/// the form's rules first, then the store's (a keyword belongs to one
/// snippet), each refusal carrying the sentence the form would show.
async fn save_snippet(
    state: &Arc<RwLock<EngineState>>,
    id: Option<String>,
    name: String,
    text: String,
    keyword: Option<String>,
    word: bool,
    apps: Vec<String>,
) -> Response {
    use compass_core::snippet_form::{Submission, submit};
    use compass_core::snippet_store::{Error, SnippetData, SnippetPayload, StoredExpansion};
    let cursors = compass_core::placeholder::parse_snippet_text(&text)
        .placeholders
        .iter()
        .filter(|placeholder| placeholder.id == compass_core::snippet_expander::CURSOR_ID)
        .count();
    let keyword = keyword.unwrap_or_default();
    let (name, content, expansion) =
        match submit(id.as_deref(), &name, &text, cursors, &keyword, word, &apps) {
            Submission::Rejected { errors, toast } => {
                let reason = [errors.name, errors.content, errors.keyword]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("; ");
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("{toast}: {reason}"),
                ));
            }
            Submission::Save {
                name,
                content,
                expansion,
                ..
            } => (name, content, expansion),
        };
    let payload = SnippetPayload {
        name,
        data: SnippetData::Text { text: content },
        expansion: expansion.map(|expansion| StoredExpansion {
            keyword: expansion.keyword,
            apps: expansion.apps,
            word: expansion.word,
        }),
    };
    let mut state = state.write().await;
    let Some(snippets) = state.snippets.store.as_mut() else {
        return snippets_unavailable();
    };
    let now = crate::shortcuts::now();
    let saved = match &id {
        Some(id) => snippets.update(id, payload, now),
        None => snippets.add(payload, now).map(|_| ()),
    };
    match saved {
        Ok(()) => snippets_response(snippets),
        Err(error @ (Error::KeywordTaken(_) | Error::NoSuchId | Error::LimitReached)) => {
            Response::Error(ProtocolError::new(ErrorKind::BadRequest, error.to_string()))
        }
        Err(error) => Response::Error(ProtocolError::new(ErrorKind::Internal, error.to_string())),
    }
}

/// Tells the input server about keywords added, changed or removed, as
/// `SnippetService::createSnippet`/`updateSnippet`/`removeSnippet` do.
async fn sync_keywords(state: &Arc<RwLock<EngineState>>) {
    let (expander, wanted) = {
        let state = state.read().await;
        let Some(expander) = state.snippets.expander.clone() else {
            return;
        };
        let wanted = state
            .snippets
            .store
            .as_ref()
            .map(|store| crate::snippet_expansion::keywords(store.snippets()))
            .unwrap_or_default();
        (expander, wanted)
    };
    tokio::spawn(async move { expander.sync(wanted).await });
}

/// `InputServerStatus`, and `SetInputServerEnabled` after saving and applying
/// the setting.
async fn input_server(state: &Arc<RwLock<EngineState>>, enable: Option<bool>) -> Response {
    if let Some(enabled) = enable {
        let saved = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let mut config = Config::load()?;
            config.input_server_mut().set_enabled(Some(enabled));
            config.save_to(compass_core::config::default_config_path()?)?;
            Ok(())
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("could not save input_server.enabled: {error}"),
                ));
            }
            Err(error) => {
                return Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("saving input_server.enabled failed: {error}"),
                ));
            }
        }
    }
    let Some(expander) = state.read().await.snippets.expander.clone() else {
        return Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            "this engine was started without the input server",
        ));
    };
    if let Some(enabled) = enable {
        expander.server().set_enabled(enabled);
        // Give the helper a moment to come up (or go), so the answer says
        // how it went rather than how it was.
        for _ in 0..20 {
            let status = expander.status();
            if status.running == enabled || status.problem.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    Response::InputServerStatus(expander.status())
}

/// A snippet's text expanded with `arguments` (a file snippet's path),
/// reading the clipboard first when the text asks for it.
async fn expand_snippet(
    state: &Arc<RwLock<EngineState>>,
    id: &str,
    arguments: &[(String, String)],
    run_shell: bool,
) -> Result<String, Response> {
    let (snippet, shell) = {
        let state = state.read().await;
        let Some(snippets) = &state.snippets.store else {
            return Err(snippets_unavailable());
        };
        let Some(snippet) = snippets.find_by_id(id) else {
            return Err(Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no snippet has that id",
            )));
        };
        (snippet.clone(), state.shell.client.clone())
    };
    let text = match &snippet.data {
        compass_core::snippet_store::SnippetData::Text { text } => text,
        compass_core::snippet_store::SnippetData::File { file } => return Ok(file.clone()),
    };
    let mut clipboard = None;
    if crate::snippets::needs_clipboard(text) {
        match shell {
            Some(shell) => match shell.clipboard().await {
                Ok(content) => clipboard = content.as_text().map(str::to_owned),
                Err(error) => tracing::info!(%error, "could not read the clipboard for a snippet"),
            },
            None => tracing::info!("no GNOME Shell extension; {{clipboard}} expands to nothing"),
        }
    }
    if !run_shell {
        return Ok(crate::snippets::preview(text, arguments, clipboard).to_text());
    }
    Ok(crate::snippets::expand(text, arguments, clipboard)
        .await
        .to_text())
}
