//! Keeping the platform's accessibility tree in step with the launcher
//! window (ADR-0022 §1).
//!
//! The binary hands the window a [`compass_platform::AccessibilityTree`] (on
//! Linux, AT-SPI through `compass-a11y-linux`) with [`install`]. After every
//! update the window describes itself ([`crate::semantics`]) and, when that
//! description changed, hands the new tree over; when the window opens or
//! closes, the tree does too, so a hidden launcher leaves nothing behind on
//! the bus. What assistive technology asks of the tree — press this, focus
//! that, set this text — comes back as [`Message::Accessibility`].
//!
//! Without an installed tree (tests, and the non-resident `run`) every
//! function here does nothing, and nothing is described.

use std::sync::{Mutex, OnceLock, PoisonError};

use accesskit::ActionRequest;
use compass_platform::AccessibilityTree;
use tokio::sync::mpsc;

use crate::message::Message;
use crate::semantics::{FocusTracker, Semantics};

struct Installed {
    tree: Box<dyn AccessibilityTree>,
    requests: mpsc::UnboundedSender<ActionRequest>,
}

static INSTALLED: OnceLock<Mutex<Installed>> = OnceLock::new();
static REQUESTS: Mutex<Option<mpsc::UnboundedReceiver<ActionRequest>>> = Mutex::new(None);

/// Keeps the tree the binary chose. Call it before the launcher runs; a
/// second call is ignored, as the window has one tree for its life.
pub fn install(tree: Box<dyn AccessibilityTree>) {
    let (requests, receiver) = mpsc::unbounded_channel();
    if INSTALLED
        .set(Mutex::new(Installed { tree, requests }))
        .is_ok()
    {
        *REQUESTS.lock().unwrap_or_else(PoisonError::into_inner) = Some(receiver);
    }
}

/// Whether there is a tree to keep in step.
pub(crate) fn installed() -> bool {
    INSTALLED.get().is_some()
}

fn with_tree(f: impl FnOnce(&mut Installed)) {
    if let Some(installed) = INSTALLED.get() {
        f(&mut installed.lock().unwrap_or_else(PoisonError::into_inner));
    }
}

/// Assistive technology's requests, as messages for the window.
pub(crate) fn subscription() -> iced::Subscription<Message> {
    fn requests() -> impl iced::futures::Stream<Item = ActionRequest> {
        let receiver = REQUESTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        iced::futures::stream::unfold(receiver, |receiver| async move {
            let mut receiver = receiver?;
            let request = receiver.recv().await?;
            Some((request, Some(receiver)))
        })
    }
    iced::Subscription::run(requests).map(Message::Accessibility)
}

/// The window's side of the tree: whether it is open, what it last said,
/// and where the focus was said to be.
#[derive(Debug, Default)]
pub(crate) struct TreeSync {
    open: bool,
    last: Option<Semantics>,
    focus: FocusTracker,
    window_focused: bool,
}

impl TreeSync {
    /// Brings the tree in step with the window: open with it, closed with
    /// it, and `describe`d again when it is open. `describe` is only called
    /// when there is a tree to give its answer to.
    pub(crate) fn follow(
        &mut self,
        open: bool,
        window_focused: bool,
        describe: impl FnOnce() -> Semantics,
    ) {
        if !installed() {
            return;
        }
        if !open {
            if self.open {
                self.open = false;
                self.last = None;
                with_tree(|installed| installed.tree.close());
            }
            return;
        }
        let semantics = describe();
        if !self.open {
            self.open = true;
            self.focus = FocusTracker::new();
            let update = semantics.tree_update(self.focus.focus(&semantics));
            self.last = Some(semantics);
            with_tree(|installed| {
                let requests = installed.requests.clone();
                installed.tree.open(
                    update,
                    Box::new(move |request| {
                        // The window is gone or going; its tree goes with it.
                        let _ = requests.send(request);
                    }),
                );
                installed.tree.set_focused(window_focused);
            });
            self.window_focused = window_focused;
            return;
        }
        if self.last.as_ref() != Some(&semantics) {
            let update = semantics.tree_update(self.focus.focus(&semantics));
            self.last = Some(semantics);
            with_tree(|installed| installed.tree.update(update));
        }
        if self.window_focused != window_focused {
            self.window_focused = window_focused;
            with_tree(|installed| installed.tree.set_focused(window_focused));
        }
    }
}
