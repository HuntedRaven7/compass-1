//! The clipboard an extension reaches: the GNOME Shell extension's, or
//! data-control on a wlroots compositor.
//!
//! Separate because it is not only a run's: the Rhai host and snippet
//! expansion reach the clipboard through the same [`ShellClipboard`].

use std::sync::Arc;

use compass_worker_host::clipboard_service::{Clipboard, Content, CopyOptions, ReadContent};

/// The clipboard an extension reaches: the GNOME Shell extension's, or
/// data-control on a wlroots compositor.
pub(crate) struct ShellClipboard {
    pub(super) shell: Option<Arc<compass_shell::ShellClient>>,
    pub(super) handle: Option<tokio::runtime::Handle>,
}

impl ShellClipboard {
    /// The clipboard through `shell`, or data-control on wlroots; `None`
    /// when this session has neither.
    pub(crate) fn available(
        shell: Option<Arc<compass_shell::ShellClient>>,
        handle: Option<tokio::runtime::Handle>,
    ) -> Option<Self> {
        (data_control() || (shell.is_some() && handle.is_some())).then_some(Self { shell, handle })
    }
}

impl ShellClipboard {
    fn run<T>(
        &self,
        what: &str,
        call: impl FnOnce(
            Arc<compass_shell::ShellClient>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = compass_shell::Result<T>> + Send>,
        >,
    ) -> Option<T> {
        let (Some(shell), Some(handle)) = (&self.shell, &self.handle) else {
            tracing::info!(
                what,
                "no GNOME Shell extension; the clipboard call did nothing"
            );
            return None;
        };
        match handle.block_on(call(Arc::clone(shell))) {
            Ok(value) => Some(value),
            Err(err) => {
                tracing::info!(what, error = %err, "clipboard call failed");
                None
            }
        }
    }

    fn selection(content: Content) -> Option<compass_shell::ClipboardContent> {
        match content {
            Content::NoData => None,
            Content::Text(text) => Some(compass_shell::ClipboardContent::text(text)),
            Content::Html {
                text: Some(text), ..
            } => Some(compass_shell::ClipboardContent::text(text)),
            Content::Html { html, text: None } => Some(compass_shell::ClipboardContent::binary(
                html.into_bytes(),
                "text/html",
            )),
            Content::Urls(urls) => Some(compass_shell::ClipboardContent::binary(
                urls.join("\r\n").into_bytes(),
                "text/uri-list",
            )),
        }
    }
}

/// The selection as data-control offers, for a wlroots compositor.
///
/// Richer than [`ShellClipboard::selection`]: data-control can offer several
/// types at once, so HTML keeps its plain-text alternative and a concealed
/// copy carries the marker the history watcher skips.
fn data_control_offers(
    content: Content,
    concealed: bool,
) -> Option<Vec<compass_wayland::data_control::Offer>> {
    use compass_wayland::data_control::{CONCEALED_MIME_TYPE, Offer};
    let offer = |mime: &str, data: Vec<u8>| Offer {
        mime_type: mime.to_owned(),
        data,
    };
    let mut offers = match content {
        Content::NoData => return None,
        Content::Text(text) => vec![offer("text/plain;charset=utf-8", text.into_bytes())],
        Content::Html { html, text } => {
            let mut offers = vec![offer("text/html", html.into_bytes())];
            if let Some(text) = text {
                offers.push(offer("text/plain;charset=utf-8", text.into_bytes()));
            }
            offers
        }
        Content::Urls(urls) => vec![offer("text/uri-list", urls.join("\r\n").into_bytes())],
    };
    if concealed {
        offers.push(offer(CONCEALED_MIME_TYPE, Vec::new()));
    }
    Some(offers)
}

/// Whether this session's clipboard is reached over data-control.
pub(super) fn data_control() -> bool {
    crate::wlroots::session().is_some_and(|wlroots| wlroots.capabilities.data_control)
}

fn data_control_set(what: &str, offers: Vec<compass_wayland::data_control::Offer>) {
    if let Err(err) = compass_wayland::clipboard::set(offers) {
        tracing::info!(what, error = %err, "clipboard call failed");
    }
}

fn data_control_read() -> ReadContent {
    let text = |mime: &str| {
        compass_wayland::clipboard::read(mime)
            .ok()
            .flatten()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    };
    ReadContent {
        text: text("text/plain").unwrap_or_default(),
        html: text("text/html"),
        urls: text("text/uri-list")
            .map(|list| {
                list.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

impl Clipboard for ShellClipboard {
    fn copy(&self, content: Content, options: CopyOptions) {
        if data_control() {
            if let Some(offers) = data_control_offers(content, options.concealed) {
                data_control_set("copy", offers);
            }
            return;
        }
        let Some(selection) = Self::selection(content) else {
            return;
        };
        self.run("copy", move |shell| {
            Box::pin(async move { shell.set_clipboard(&selection).await })
        });
    }

    fn paste(&self, content: Content) {
        if data_control() {
            if let Some(offers) = data_control_offers(content, false) {
                match &self.handle {
                    Some(handle) => crate::paste::paste_blocking(handle, offers),
                    None => data_control_set("paste", offers),
                }
            }
            return;
        }
        let Some(selection) = Self::selection(content) else {
            return;
        };
        self.run("paste", move |shell| {
            Box::pin(async move {
                shell.set_clipboard(&selection).await?;
                shell.paste(&[]).await
            })
        });
    }

    fn clear(&self) {
        if data_control() {
            if let Err(err) = compass_wayland::clipboard::clear() {
                tracing::info!(error = %err, "clipboard clear failed");
            }
            return;
        }
        self.run("clear", |shell| {
            Box::pin(async move {
                shell
                    .set_clipboard(&compass_shell::ClipboardContent::text(""))
                    .await
            })
        });
    }

    fn read(&self) -> ReadContent {
        if data_control() {
            return data_control_read();
        }
        self.run("read", |shell| {
            Box::pin(async move { shell.clipboard().await })
        })
        .map(|content| ReadContent {
            text: content.as_text().unwrap_or_default().to_owned(),
            ..ReadContent::default()
        })
        .unwrap_or_default()
    }
}
