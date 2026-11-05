//! The launcher window's accessibility tree, as the platform's assistive
//! technology sees it (ADR-0022 §1).
//!
//! The launcher describes itself as an [`accesskit::TreeUpdate`], built from
//! its own state rather than from toolkit widgets, and hands it here; how the
//! tree reaches a screen reader is the platform's (on Linux, AT-SPI through
//! `accesskit_unix`, in `compass-a11y-linux`), handed to the window by the
//! `compass` binary. AccessKit's macOS and Windows adapters take the same
//! tree, so the description is shared and only the adapter is per platform.

pub use accesskit;

use accesskit::{ActionRequest, TreeUpdate};

/// Where an assistive technology's requests go: "press this", "focus that",
/// "set this field's text". Called from the adapter's own thread.
pub type ActionSink = Box<dyn FnMut(ActionRequest) + Send>;

/// Exposes the launcher window to the platform's assistive technology.
///
/// One window at a time: the launcher's. [`open`](Self::open) and
/// [`close`](Self::close) follow the window, so the tree is never there
/// without it.
pub trait AccessibilityTree: Send {
    /// The window appeared: expose `initial`, which is a whole tree, and send
    /// what assistive technology asks of it to `actions`.
    ///
    /// Opening an already open tree replaces it.
    fn open(&mut self, initial: TreeUpdate, actions: ActionSink);

    /// The window's content changed. `update` is a whole tree; the adapter
    /// tells assistive technology only what differs from the last one.
    ///
    /// Nothing while the tree is closed.
    fn update(&mut self, update: TreeUpdate);

    /// The window gained (`true`) or lost the keyboard focus.
    fn set_focused(&mut self, focused: bool);

    /// The window went away: take its tree away with it.
    fn close(&mut self);
}
