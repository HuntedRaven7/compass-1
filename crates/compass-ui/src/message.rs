//! Messages for the launcher application.

use iced::Event;

/// Messages that the launcher application can handle.
#[derive(Debug, Clone)]
pub enum Message {
    /// Initialize the application (connect portals, bind shortcuts).
    Initialize,
    /// The search query changed.
    QueryChanged(String),
    /// A result was selected (by keyboard navigation).
    ResultSelected(usize),
    /// Launch the selected result.
    LaunchSelected,
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
}
