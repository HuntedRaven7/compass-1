//! Snippets and URL shortcuts: the texts and links a person saves, expands
//! and pastes or opens.
//!
//! The bodies of these [`ApplicationBackend`](compass_ui::backend::ApplicationBackend)
//! requests, together because an edit to either is answered with the whole
//! list again, which the readers at the end of this module take from the wire.

use compass_ipc::Request;
use compass_ui::backend::{BackendFuture, Shortcut, ShortcutDraft, Snippet, SnippetDraft};

use super::DaemonBackend;

impl DaemonBackend {
    pub(super) fn list_snippets(&self) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(
            async move { snippets(self.ask(Request::ListSnippets, "Listing snippets").await?) },
        )
    }

    pub(super) fn save_snippet(&self, snippet: SnippetDraft) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(async move {
            let request = Request::SaveSnippet {
                id: snippet.id,
                name: snippet.name,
                text: snippet.text,
                keyword: snippet.keyword,
                word: snippet.word,
                apps: snippet.apps,
            };
            snippets(self.ask(request, "Saving the snippet").await?)
        })
    }

    pub(super) fn remove_snippet(&self, id: String) -> BackendFuture<'_, Vec<Snippet>> {
        Box::pin(async move {
            snippets(
                self.ask(Request::RemoveSnippet { id }, "Removing the snippet")
                    .await?,
            )
        })
    }

    pub(super) fn expand_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, String> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExpandSnippet { id, arguments },
                    "Expanding the snippet",
                )
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn preview_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, String> {
        Box::pin(async move {
            match self
                .ask(
                    Request::PreviewSnippet { id, arguments },
                    "Previewing the snippet",
                )
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn paste_snippet(
        &self,
        id: String,
        arguments: Vec<(String, String)>,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::PasteSnippet { id, arguments },
                    "Pasting the snippet",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn list_shortcuts(&self) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async move {
            shortcuts(
                self.ask(Request::ListShortcuts, "Listing shortcuts")
                    .await?,
            )
        })
    }

    pub(super) fn save_shortcut(
        &self,
        shortcut: ShortcutDraft,
    ) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async move {
            let request = Request::SaveShortcut {
                id: shortcut.id,
                name: shortcut.name,
                icon: shortcut.icon,
                url: shortcut.url,
                app: shortcut.app,
            };
            shortcuts(self.ask(request, "Saving the shortcut").await?)
        })
    }

    pub(super) fn remove_shortcut(&self, id: String) -> BackendFuture<'_, Vec<Shortcut>> {
        Box::pin(async move {
            shortcuts(
                self.ask(Request::RemoveShortcut { id }, "Removing the shortcut")
                    .await?,
            )
        })
    }

    pub(super) fn open_shortcut(
        &self,
        id: String,
        arguments: Vec<String>,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::OpenShortcut { id, arguments },
                    "Opening the shortcut",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn expand_shortcut(
        &self,
        id: String,
        arguments: Vec<String>,
    ) -> BackendFuture<'_, String> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExpandShortcut { id, arguments },
                    "Expanding the shortcut",
                )
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

/// The snippet list in an engine answer.
fn snippets(response: compass_ipc::Response) -> Result<Vec<Snippet>, String> {
    use compass_core::snippet_store::{SnippetData, StoredExpansion};
    match response {
        compass_ipc::Response::Snippets { snippets } => Ok(snippets
            .into_iter()
            .map(|entry| Snippet {
                id: entry.id,
                name: entry.name,
                data: match (entry.text, entry.file) {
                    (Some(text), _) => SnippetData::Text { text },
                    (None, Some(file)) => SnippetData::File { file },
                    (None, None) => SnippetData::default(),
                },
                created_at: entry.created_at,
                updated_at: entry.updated_at,
                expansion: entry.keyword.map(|keyword| StoredExpansion {
                    keyword,
                    apps: entry.apps,
                    word: entry.word,
                }),
            })
            .collect()),
        other => Err(format!("Unexpected answer from the engine: {other:?}")),
    }
}

/// The shortcut list in an engine answer.
fn shortcuts(response: compass_ipc::Response) -> Result<Vec<Shortcut>, String> {
    match response {
        compass_ipc::Response::Shortcuts { shortcuts } => Ok(shortcuts
            .into_iter()
            .map(|entry| Shortcut {
                id: entry.id,
                name: entry.name,
                icon: entry.icon,
                url: entry.url,
                app: entry.app,
                open_count: entry.open_count,
                created_at: entry.created_at,
                updated_at: entry.updated_at,
                last_used_at: entry.last_used_at,
            })
            .collect()),
        other => Err(format!("Unexpected answer from the engine: {other:?}")),
    }
}
