//! The launcher window: search, move, launch, dismiss.
//!
//! The state machine is deliberately separable from Iced. `update` takes a
//! [`Message`] and mutates fields; the only Iced-shaped things it returns are
//! [`Task`]s, which can be constructed without a running runtime. That is what
//! makes this crate testable in a container with no display server — the
//! selection and launch-target logic, which is where the bugs live, is exercised
//! by ordinary unit tests, and only the drawing needs a compositor.

use iced::{
    Element, Length, Task, Theme,
    widget::{Space, column, container, row, text, text_input},
    window,
};

use compass_core::{AppIndex, AppItem};
use compass_search::rank_indices;

use crate::message::{Direction, Message};

/// Flags for configuring the launcher app.
#[derive(Debug, Clone)]
pub struct AppFlags {
    /// Window configuration.
    pub window_config: window::Settings,
}

impl Default for AppFlags {
    fn default() -> Self {
        Self {
            window_config: window::Settings {
                size: iced::Size::new(640.0, 480.0),
                position: window::Position::Centered,
                resizable: false,
                decorations: false,
                transparent: true,
                ..Default::default()
            },
        }
    }
}

/// The launcher application state.
pub struct LauncherApp {
    /// The application index.
    app_index: AppIndex,
    /// Current query text.
    query: String,
    /// Ranked results, as indices into `app_index.items()`.
    ///
    /// Indices rather than cloned items: the ranking already works in index
    /// space, an `AppItem` carries its whole parsed desktop entry, and a
    /// launcher re-ranks on every keystroke.
    results: Vec<usize>,
    /// Which row is selected, as a position in `results`.
    selected: usize,
    /// The last launch failure, shown until the query changes.
    error: Option<String>,
}

/// Where the selection lands after moving one row in `direction`.
///
/// Wraps at both ends, which is what every launcher does and what makes the
/// first Up press useful. Returns 0 for an empty list so callers never index
/// into nothing.
#[must_use]
pub fn next_selection(len: usize, current: usize, direction: Direction) -> usize {
    if len == 0 {
        return 0;
    }
    match direction {
        Direction::Down => (current + 1) % len,
        // `current` can exceed `len` only if the list shrank without the
        // selection being reset; saturating keeps that from underflowing.
        Direction::Up => current.checked_sub(1).unwrap_or(len - 1).min(len - 1),
    }
}

impl LauncherApp {
    /// Create a new launcher application, indexing the environment.
    pub fn new(_flags: AppFlags) -> (Self, Task<Message>) {
        (Self::with_index(AppIndex::from_environment()), Task::none())
    }

    /// Create one over a supplied index.
    ///
    /// Exists so tests can drive the state machine over a known corpus instead
    /// of whatever applications the machine running them happens to have.
    #[must_use]
    pub fn with_index(app_index: AppIndex) -> Self {
        Self {
            app_index,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            error: None,
        }
    }

    /// The item the selection currently points at, if any.
    #[must_use]
    pub fn selected_item(&self) -> Option<&AppItem> {
        let index = *self.results.get(self.selected)?;
        self.app_index.items().get(index)
    }

    /// The application title.
    pub fn title(&self) -> String {
        "Vicinae".to_owned()
    }

    /// The application theme.
    pub fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }

    /// Keyboard events the widgets did not consume.
    ///
    /// `listen` yields only events with `Status::Ignored`, so the text input
    /// still gets every printable key and this sees the arrows and Escape.
    pub fn subscription(&self) -> iced::Subscription<Message> {
        iced::keyboard::listen().map(Message::Keyboard)
    }

    /// Update the application state.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Initialize => Task::none(),
            Message::QueryChanged(query) => {
                self.query = query;
                self.error = None;
                self.search();
                Task::none()
            }
            Message::ResultSelected(index) => {
                if index < self.results.len() {
                    self.selected = index;
                }
                Task::none()
            }
            Message::MoveSelection(direction) => {
                self.selected = next_selection(self.results.len(), self.selected, direction);
                Task::none()
            }
            Message::LaunchSelected => {
                let Some(item) = self.selected_item() else {
                    return Task::none();
                };
                // Cloned into the future because the launch outlives this
                // borrow of `self`. An AppItem is a parsed desktop entry, so
                // this is not free — but it happens once per launch, not once
                // per keystroke.
                let entry = item.entry().clone();
                Task::perform(
                    async move {
                        compass_platform::launch_app(&entry)
                            .await
                            .map(|_method| ())
                            .map_err(|err| err.to_string())
                    },
                    Message::Launched,
                )
            }
            Message::Launched(Ok(())) => {
                // A launcher that stays open after launching is a bug report
                // waiting to happen.
                iced::exit()
            }
            Message::Launched(Err(err)) => {
                self.error = Some(err);
                Task::none()
            }
            Message::Dismiss => iced::exit(),
            Message::ShortcutActivated(_) => Task::none(),
            Message::FocusChanged(_) => Task::none(),
            Message::WindowClosed => iced::exit(),
            Message::PollShortcuts => Task::none(),
            Message::EventOccurred(_) => Task::none(),
            Message::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) => {
                use iced::keyboard::{Key, key::Named};
                match key.as_ref() {
                    Key::Named(Named::ArrowDown) => {
                        self.update(Message::MoveSelection(Direction::Down))
                    }
                    Key::Named(Named::ArrowUp) => {
                        self.update(Message::MoveSelection(Direction::Up))
                    }
                    Key::Named(Named::Escape) => self.update(Message::Dismiss),
                    _ => Task::none(),
                }
            }
            Message::Keyboard(_) => Task::none(),
        }
    }

    /// View the application.
    pub fn view(&self) -> Element<'_, Message> {
        let input = text_input("Search...", &self.query)
            .on_input(Message::QueryChanged)
            .padding(12)
            .size(24)
            .on_submit(Message::LaunchSelected);

        let results_content: Element<Message> = if let Some(err) = &self.error {
            text(format!("could not launch: {err}")).size(16).into()
        } else if self.query.is_empty() {
            text("Type to search...").size(16).into()
        } else if self.results.is_empty() {
            text("No results").size(16).into()
        } else {
            let mut col = column![].spacing(4);
            for (position, index) in self.results.iter().enumerate() {
                let Some(item) = self.app_index.items().get(*index) else {
                    continue;
                };
                // A caret rather than a colour: the selected row has to be
                // identifiable in a screenshot the VM tier captures, and under
                // llvmpipe at 1280x800 a background tint is not.
                let marker = if position == self.selected {
                    "> "
                } else {
                    "  "
                };
                col = col.push(row![text(marker).size(16), text(item.name()).size(16)]);
            }
            container(col).width(Length::Fill).padding(20).into()
        };

        let content = column![
            Space::new(),
            container(input).width(Length::Fill).padding(20),
            Space::new(),
            results_content,
            Space::new(),
        ]
        .align_x(iced::Alignment::Center);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(20)
            .into()
    }

    /// Re-rank against the current query.
    fn search(&mut self) {
        if self.query.trim().is_empty() {
            self.results.clear();
            self.selected = 0;
            return;
        }

        self.results = rank_indices(&self.query, self.app_index.items())
            .into_iter()
            .map(|scored| scored.item)
            .collect();
        // Back to the top on every new query: the old selection pointed into a
        // different list, and keeping its position would silently select an
        // unrelated application.
        self.selected = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// An index over three entries written to a tempdir, so the tests do not
    /// depend on what the machine running them has installed.
    fn index(dir: &std::path::Path) -> AppIndex {
        for (file, name) in [
            ("firefox.desktop", "Firefox"),
            ("files.desktop", "Files"),
            ("terminal.desktop", "Terminal"),
        ] {
            fs::write(
                dir.join(file),
                format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n"),
            )
            .expect("write entry");
        }
        AppIndex::builder().dir(dir).build()
    }

    fn app(dir: &std::path::Path) -> LauncherApp {
        LauncherApp::with_index(index(dir))
    }

    #[test]
    fn selection_wraps_at_both_ends() {
        // The first Up press is the one people actually use — it should land on
        // the last row, not sit at the top doing nothing.
        assert_eq!(next_selection(3, 0, Direction::Up), 2);
        assert_eq!(next_selection(3, 2, Direction::Down), 0);
        assert_eq!(next_selection(3, 0, Direction::Down), 1);
        assert_eq!(next_selection(3, 2, Direction::Up), 1);
    }

    #[test]
    fn selection_on_an_empty_list_is_not_an_index_into_nothing() {
        assert_eq!(next_selection(0, 0, Direction::Up), 0);
        assert_eq!(next_selection(0, 0, Direction::Down), 0);
    }

    #[test]
    fn a_selection_left_past_the_end_is_clamped_rather_than_underflowing() {
        // Reachable if a list shrinks without the selection being reset. The
        // arithmetic here is unsigned, so getting this wrong is a panic.
        assert_eq!(next_selection(2, 9, Direction::Up), 1);
        assert_eq!(next_selection(1, 5, Direction::Down), 0);
    }

    #[test]
    fn typing_ranks_and_selects_the_first_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        assert_eq!(app.selected, 0);
        assert_eq!(
            app.selected_item().map(compass_core::AppItem::name),
            Some("Firefox")
        );
    }

    #[test]
    fn a_new_query_resets_the_selection() {
        // Otherwise the old position points into a different list and the user
        // launches something they never looked at.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        // "fi" matches Files and Firefox. A single "e" matches nothing at all:
        // the coherence rule (ADR-0006) drops a one-character query that is not
        // a word prefix, which is correct and cost this test a first draft.
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let _ = app.update(Message::MoveSelection(Direction::Down));
        assert_ne!(app.selected, 0, "precondition: the selection moved");
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn an_empty_query_shows_nothing_and_selects_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fire".to_owned()));
        let _ = app.update(Message::QueryChanged("   ".to_owned()));
        assert!(app.results.is_empty());
        assert!(app.selected_item().is_none());
    }

    #[test]
    fn launching_with_no_results_does_nothing_rather_than_panicking() {
        // Enter on an empty list is the most ordinary way to reach this.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzznotathing".to_owned()));
        assert!(app.results.is_empty());
        let _ = app.update(Message::LaunchSelected);
    }

    #[test]
    fn a_failed_launch_is_shown_and_cleared_by_the_next_keystroke() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::Launched(Err("no Exec key".to_owned())));
        assert_eq!(app.error.as_deref(), Some("no Exec key"));
        let _ = app.update(Message::QueryChanged("f".to_owned()));
        assert!(app.error.is_none(), "a new query should clear the error");
    }

    #[test]
    fn moving_the_selection_lands_on_the_item_that_gets_launched() {
        // The property that matters: what the caret points at in the view is
        // what LaunchSelected resolves. A off-by-one here launches the wrong
        // application, silently and every time.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let expected: Vec<String> = app
            .results
            .iter()
            .filter_map(|i| app.app_index.items().get(*i))
            .map(|item| item.name().to_owned())
            .collect();
        assert!(expected.len() > 1, "need several rows: {expected:?}");

        for (position, name) in expected.iter().enumerate() {
            assert_eq!(app.selected, position);
            assert_eq!(
                app.selected_item().map(compass_core::AppItem::name),
                Some(name.as_str())
            );
            let _ = app.update(Message::MoveSelection(Direction::Down));
        }
        assert_eq!(app.selected, 0, "and it wrapped back to the top");
    }
}
