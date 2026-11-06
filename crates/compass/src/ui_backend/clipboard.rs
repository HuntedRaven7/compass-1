//! The [`ClipboardBackend`] impl: clipboard history, read, pinned, pasted
//! and removed through the engine.
//!
//! Its own trait, apart from the launcher's requests, with the conversions
//! between the window's rows and the wire.

use compass_ipc::Request;
use compass_ui::backend::{
    BackendFuture, ClipboardBackend, ClipboardContent, ClipboardRow, ClipboardRowKind,
};

use super::DaemonBackend;

impl ClipboardBackend for DaemonBackend {
    fn clipboard_history(&self, query: String, limit: u32) -> BackendFuture<'_, Vec<ClipboardRow>> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardHistory { query, limit },
                    "Clipboard history",
                )
                .await?
            {
                compass_ipc::Response::ClipboardHistory { entries } => {
                    Ok(entries.into_iter().map(row).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_content(&self, id: String) -> BackendFuture<'_, ClipboardContent> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardContent { id },
                    "Reading the clipboard entry",
                )
                .await?
            {
                compass_ipc::Response::ClipboardContent { mime_type, data } => {
                    Ok(ClipboardContent { mime_type, data })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_paste(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self.ask(Request::ClipboardPaste { id }, "Pasting").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_set_pinned(&self, id: String, pinned: bool) -> BackendFuture<'_, ()> {
        let what = if pinned { "Pinning" } else { "Unpinning" };
        Box::pin(async move {
            match self
                .ask(Request::ClipboardSetPinned { id, pinned }, what)
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_remove(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ClipboardRemove { id }, "Removing the entry")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_history_of_kind(
        &self,
        query: String,
        limit: u32,
        kind: Option<ClipboardRowKind>,
    ) -> BackendFuture<'_, Vec<ClipboardRow>> {
        Box::pin(async move {
            let request = Request::ClipboardHistoryOfKind {
                query,
                limit,
                kind: kind.map(wire_kind),
            };
            match self.ask(request, "Clipboard history").await? {
                compass_ipc::Response::ClipboardHistory { entries } => {
                    Ok(entries.into_iter().map(row).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_detail(
        &self,
        id: String,
    ) -> BackendFuture<'_, compass_ui::backend::ClipboardDetail> {
        Box::pin(async move {
            match self
                .ask(Request::ClipboardDetail { id }, "Reading the entry")
                .await?
            {
                compass_ipc::Response::ClipboardDetail { detail } => {
                    Ok(compass_ui::backend::ClipboardDetail {
                        id: detail.id,
                        mime_type: detail.mime_type,
                        kind: row_kind(detail.kind),
                        size: detail.size,
                        md5: detail.md5,
                        updated_at: detail.updated_at,
                        encrypted: detail.encrypted,
                        keywords: detail.keywords,
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_set_keywords(&self, id: String, keywords: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardSetKeywords { id, keywords },
                    "Saving the keywords",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_remove_all(&self) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ClipboardRemoveAll, "Removing every entry")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn clipboard_monitoring(
        &self,
        enabled: Option<bool>,
    ) -> BackendFuture<'_, compass_ui::backend::ClipboardMonitoring> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ClipboardMonitoring { enabled },
                    "Clipboard monitoring",
                )
                .await?
            {
                compass_ipc::Response::ClipboardMonitoring { supported, enabled } => {
                    Ok(compass_ui::backend::ClipboardMonitoring { supported, enabled })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

const fn row_kind(kind: compass_ipc::ClipboardKind) -> ClipboardRowKind {
    match kind {
        compass_ipc::ClipboardKind::Text => ClipboardRowKind::Text,
        compass_ipc::ClipboardKind::Link => ClipboardRowKind::Link,
        compass_ipc::ClipboardKind::Image => ClipboardRowKind::Image,
        compass_ipc::ClipboardKind::File => ClipboardRowKind::File,
        compass_ipc::ClipboardKind::Unknown => ClipboardRowKind::Unknown,
    }
}

const fn wire_kind(kind: ClipboardRowKind) -> compass_ipc::ClipboardKind {
    match kind {
        ClipboardRowKind::Text => compass_ipc::ClipboardKind::Text,
        ClipboardRowKind::Link => compass_ipc::ClipboardKind::Link,
        ClipboardRowKind::Image => compass_ipc::ClipboardKind::Image,
        ClipboardRowKind::File => compass_ipc::ClipboardKind::File,
        ClipboardRowKind::Unknown => compass_ipc::ClipboardKind::Unknown,
    }
}

fn row(entry: compass_ipc::ClipboardEntry) -> ClipboardRow {
    ClipboardRow {
        id: entry.id,
        preview: entry.preview,
        kind: row_kind(entry.kind),
        pinned: entry.pinned,
        url_host: entry.url_host,
    }
}
