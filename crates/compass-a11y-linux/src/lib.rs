//! The launcher's accessibility tree on AT-SPI, through `accesskit_unix`
//! (ADR-0022 §1).
//!
//! `compass-ui` describes the launcher window as an [`accesskit::TreeUpdate`];
//! this crate puts it on the accessibility bus, where Orca and the
//! accessibility inspectors read it. The adapter runs on a thread of its own
//! with its own runtime, and is idle until assistive technology turns
//! accessibility on (`org.a11y.Status.IsEnabled`): until then nothing is
//! exported and no tree is walked.
//!
//! One `accesskit_unix::Adapter` is one window under the application's AT-SPI
//! root. It is created when the launcher window opens and dropped when it
//! closes, which is what removes the window and every node under it from the
//! bus; a hidden launcher leaves nothing for a screen reader to find.
//!
//! The adapter asks for the initial tree from its own thread when it is
//! activated, which can be long after the window opened (Orca started while
//! the launcher is up). So the latest tree is kept here, not rebuilt on
//! request: the UI's state lives on the UI thread and cannot be read from
//! the adapter's.

#![deny(missing_docs)]

use std::sync::{Arc, Mutex, PoisonError};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use compass_platform::{AccessibilityTree, ActionSink};

/// The launcher window's tree on AT-SPI.
#[derive(Default)]
pub struct AtspiTree {
    /// The window's adapter while it is open.
    adapter: Option<accesskit_unix::Adapter>,
    /// The latest whole tree, for the adapter to start from when it is
    /// activated.
    latest: Arc<Mutex<Option<TreeUpdate>>>,
    /// Whether the window has the keyboard focus, for an adapter created
    /// after the focus arrived.
    focused: bool,
}

impl AtspiTree {
    /// A tree with no window yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for AtspiTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtspiTree")
            .field("open", &self.adapter.is_some())
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

impl AccessibilityTree for AtspiTree {
    fn open(&mut self, initial: TreeUpdate, actions: ActionSink) {
        // The old window's adapter goes first, so the bus never shows two.
        self.adapter = None;
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(initial);
        let mut adapter = accesskit_unix::Adapter::new(
            Latest(Arc::clone(&self.latest)),
            Actions(actions),
            Deactivated,
        );
        adapter.update_window_focus_state(self.focused);
        self.adapter = Some(adapter);
    }

    fn update(&mut self, update: TreeUpdate) {
        let Some(adapter) = &mut self.adapter else {
            return;
        };
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(update.clone());
        adapter.update_if_active(|| update);
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if let Some(adapter) = &mut self.adapter {
            adapter.update_window_focus_state(focused);
        }
    }

    fn close(&mut self) {
        self.adapter = None;
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// Answers the adapter's request for the whole tree with the latest one.
struct Latest(Arc<Mutex<Option<TreeUpdate>>>);

impl ActivationHandler for Latest {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Passes assistive technology's requests to the launcher.
struct Actions(ActionSink);

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        (self.0)(request);
    }
}

/// Nothing to release when assistive technology goes away: the tree is kept
/// up to date regardless, for the next activation.
struct Deactivated;

impl DeactivationHandler for Deactivated {
    fn deactivate_accessibility(&mut self) {
        tracing::debug!("assistive technology stopped reading the launcher");
    }
}
