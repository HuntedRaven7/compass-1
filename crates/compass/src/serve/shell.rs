//! The desktop shell as the engine drives it: the resident launcher window,
//! the GNOME Shell extension's client, the dmenu lists waiting on the window,
//! and the window listing and switching requests.

use std::sync::Arc;

use compass_ipc::{
    ErrorKind, ProtocolError, Request, Response, WindowCommand, WindowLink, WindowOutcome,
};
use tokio::sync::{Mutex, RwLock};

use super::EngineState;

/// The launcher window and the Shell extension.
#[derive(Default)]
pub(super) struct Shell {
    /// The attached launcher window, if one is.
    pub(super) window: WindowSlot,
    /// The GNOME Shell extension's client, once the session bus answered.
    /// `None` until then, and for good without a session bus.
    pub(super) client: Option<Arc<compass_shell::ShellClient>>,
    /// `compass dmenu` lists waiting on the launcher.
    pub(super) dmenus: Arc<crate::dmenu::Pending>,
}

/// The window and dmenu requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::ListWindows => list_windows(state).await,

        Request::ActivateWindow { id } | Request::CloseWindow { id } => {
            act_on_window(state, id, matches!(request, Request::CloseWindow { .. })).await
        }

        Request::Dmenu { spec } => {
            let (slot, dmenus) = {
                let state = state.read().await;
                (state.window_slot(), Arc::clone(&state.shell.dmenus))
            };
            let (token, chosen) = dmenus.open(spec);
            match forward(&slot, WindowCommand::Dmenu(token), "show a dmenu list").await {
                Response::Ack => Response::DmenuOutput {
                    output: chosen.await.unwrap_or_default(),
                },
                refused => {
                    dmenus.choose(token, None);
                    refused
                }
            }
        }
        Request::DmenuFetch { token } => match state.read().await.shell.dmenus.spec(token) {
            Some(spec) => Response::DmenuList { spec },
            None => Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no dmenu list has that token",
            )),
        },
        Request::DmenuChoose { token, output } => {
            if state.read().await.shell.dmenus.choose(token, output) {
                Response::Ack
            } else {
                Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    "no dmenu list has that token",
                ))
            }
        }

        Request::Toggle | Request::Show | Request::Hide => {
            let (slot, command, what) = {
                let state = state.read().await;
                match request {
                    Request::Toggle => (
                        state.window_slot(),
                        WindowCommand::Toggle,
                        "toggle a window",
                    ),
                    Request::Show => (state.window_slot(), WindowCommand::Show, "show a window"),
                    _ => (state.window_slot(), WindowCommand::Hide, "hide a window"),
                }
            };
            // The read lock is released before the push: a window that takes a
            // moment to answer must not block queries from other clients.
            forward(&slot, command, what).await
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a shell request",
        )),
    }
}

/// The at-most-one launcher window this engine drives.
///
/// # Last window wins
///
/// A second `compass ui` **replaces** the first rather than being refused.
/// Dropping the old [`WindowLink`] closes its socket, so that window's
/// `next_command` returns `None` and it exits its loop.
///
/// The alternative — refuse the newcomer — reads better until you ask what
/// happens when a window wedges without closing its socket: the engine would
/// hold a link that answers nothing and refuse every replacement, and the only
/// way out would be restarting the daemon. Replacing has no state that can get
/// stuck. It also means the handler needs no reservation between answering
/// `WindowAttached` and the link actually arriving, which is a window (however
/// narrow) in which an attach that died mid-handshake could strand the slot.
///
/// # Death is noticed on the next push, not before
///
/// Nothing polls the link. A window that dies is discovered when the engine
/// next tries to push to it, at which point the slot is cleared and the request
/// is refused with [`no_window`] — correctly, since by then there is none.
/// Proactively watching would mean a second reader on a socket whose only
/// reader is [`WindowLink::push`]'s reply, so it would have to be built into
/// the link rather than bolted beside it.
pub(super) type WindowSlot = Arc<Mutex<Option<WindowLink>>>;

/// The three requests that need a launcher window, when none is connected.
///
/// [ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)
/// makes the window a resident process that connects to this daemon, so the
/// honest answer is about a *connection* rather than about the build. That is
/// the more useful thing to be told: "start the window" is actionable, "this
/// engine is headless" was not.
///
/// The property this preserves is the one that matters. A client can still
/// tell "no window" from "the window was shown", which is the only thing
/// standing between an honest gap and a `toggle` that silently does nothing.
fn no_window(what: &str) -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        format!(
            "cannot {what}: no launcher window is connected to this engine. \
             Start one with `compass ui`. Query and doctor work without it."
        ),
    ))
}

/// Forwards `command` to the attached window, if there is one.
///
/// Clears the slot when the push fails. That is **hygiene, not behaviour**: a
/// push to a dead socket fails anyway, so the refusal a client sees is the same
/// either way — a control confirmed the end-to-end tests pass with the clearing
/// removed. What it buys is releasing the file descriptor and not paying a
/// doomed write on every subsequent request. See [`WindowSlot`].
pub(crate) async fn forward(slot: &WindowSlot, command: WindowCommand, what: &str) -> Response {
    forward_within(slot, command, what, WINDOW_ANSWER_TIMEOUT).await
}

/// How long the engine waits for the window to answer a command.
///
/// The slot is locked while it waits, so every request that needs the window
/// waits too. Every command is answered as soon as the window has acted on it
/// (a dmenu's choice comes later, outside the push), but the first `Show` waits
/// on the new window's renderer, and a machine without a GPU spends seconds
/// probing Vulkan before it settles on GL. So this bounds a stuck window rather
/// than a slow one, and is longer than the CLI's own 10 seconds: a slow first
/// show must not be refused while it is still coming up.
const WINDOW_ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// [`forward`] with the wait for the window's answer bounded by `timeout`.
///
/// A window that does not answer in time keeps its link. It may only be busy,
/// and dropping it would leave the launcher undriven until it restarts; its
/// late answer is skipped by the next push (see [`WindowLink::push`]).
pub(crate) async fn forward_within(
    slot: &WindowSlot,
    command: WindowCommand,
    what: &str,
    timeout: std::time::Duration,
) -> Response {
    let mut guard = slot.lock().await;

    let Some(link) = guard.as_mut() else {
        return no_window(what);
    };

    let Ok(answer) = tokio::time::timeout(timeout, link.push(command)).await else {
        tracing::warn!(
            seconds = timeout.as_secs_f32(),
            "the launcher window did not answer"
        );
        return Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            format!(
                "the launcher window did not answer within {} seconds, so it could not {what}",
                timeout.as_secs()
            ),
        ));
    };

    match answer {
        Ok(WindowOutcome::Shown | WindowOutcome::Hidden) => Response::Ack,
        Ok(WindowOutcome::Failed(reason)) => {
            // The window is alive and said no. Keeping the link is the point:
            // a compositor that refused one activation will very likely accept
            // the next, and dropping the window over it would turn a recoverable
            // refusal into a dead launcher.
            tracing::warn!(reason = %reason, "the launcher window refused a command");
            Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                format!("the launcher window could not {what}: {reason}"),
            ))
        }
        Err(err) => {
            tracing::info!(error = %err, "the launcher window went away");
            *guard = None;
            no_window(what)
        }
    }
}

/// `ListWindows`: from KWin's tracker on Plasma, over Wayland on a wlroots
/// compositor, through the Shell extension everywhere else.
pub(crate) async fn list_windows(state: &Arc<RwLock<EngineState>>) -> Response {
    {
        let state = state.read().await;
        if let Some(response) = crate::window_service::kwin_list(&state.search.index).await {
            return response;
        }
    }
    // wlroots compositors list windows over Wayland; never on GNOME.
    if crate::wlroots::detect().await.is_some() {
        let state = state.read().await;
        if let Some(response) = crate::window_service::wlroots_list(&state.search.index).await {
            return response;
        }
    }
    let (shell, index_state) = (state.read().await.shell.client.clone(), Arc::clone(state));
    let Some(shell) = shell else {
        return Response::Error(crate::window_service::no_bus("Window switching"));
    };
    match shell.list_windows().await {
        Ok(windows) => {
            let state = index_state.read().await;
            Response::Windows {
                windows: crate::window_service::rows(
                    windows.into_iter().map(Into::into).collect(),
                    &state.search.index,
                ),
            }
        }
        Err(err) => Response::Error(crate::window_service::refusal(&err, "Window switching")),
    }
}

/// `ActivateWindow` or, with `close`, `CloseWindow`.
pub(crate) async fn act_on_window(
    state: &Arc<RwLock<EngineState>>,
    id: u32,
    close: bool,
) -> Response {
    let what = if close {
        "Closing a window"
    } else {
        "Switching to a window"
    };
    if let Some(response) = crate::window_service::kwin_act(id, close, what).await {
        return response;
    }
    if let Some(response) = crate::window_service::wlroots_act(id, close, what).await {
        return response;
    }
    let Some(shell) = state.read().await.shell.client.clone() else {
        return Response::Error(crate::window_service::no_bus(what));
    };
    let id = compass_shell::WindowId(id);
    let done = if close {
        shell.close_window(id).await
    } else {
        shell.activate_window(id).await
    };
    match done {
        Ok(()) => Response::Ack,
        Err(err) => Response::Error(crate::window_service::refusal(&err, what)),
    }
}
