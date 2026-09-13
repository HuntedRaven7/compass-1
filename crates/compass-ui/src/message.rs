//! Messages for the launcher application.

use iced::Event;

/// Which way the selection moves.
///
/// An enum rather than a signed delta: the only two motions a launcher list has
/// are next and previous, and a number invites callers to invent a third.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Towards the top of the list.
    Up,
    /// Towards the bottom of the list.
    Down,
}

/// Messages that the launcher application can handle.
#[derive(Debug, Clone)]
pub enum Message {
    /// Initialize the application (connect portals, bind shortcuts).
    Initialize,
    /// The search query changed.
    QueryChanged(String),
    /// A result was selected (by keyboard navigation).
    ResultSelected(usize),
    /// Move the selection one row, wrapping at both ends.
    MoveSelection(Direction),
    /// Launch the selected result.
    LaunchSelected,
    /// A launch finished, successfully or not.
    ///
    /// Carried as a string rather than the error type because a `Message` must
    /// be `Clone` and `LaunchError` is not — and because the only thing the UI
    /// does with a failure is show it.
    Launched(Result<(), String>),
    /// Dismiss the launcher without launching anything.
    Dismiss,
    /// A global shortcut was activated.
    ShortcutActivated(String),
    /// Window focus changed.
    FocusChanged(bool),
    /// Window was closed.
    WindowClosed,
    /// Poll for shortcut events.
    PollShortcuts,
    /// Raw Iced event (for advanced handling).
    EventOccurred(Event),
    /// A keyboard event that no widget consumed.
    ///
    /// Carries the whole event rather than a pre-digested action because the
    /// text input takes the printable keys first; what reaches here is exactly
    /// the set the launcher itself has to interpret.
    Keyboard(iced::keyboard::Event),
}
