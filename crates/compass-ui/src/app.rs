//! The main launcher application - minimal stub for Phase 1.

use iced::{
    Element, Length, Task, Theme,
    widget::{Space, column, container, text, text_input},
    window,
};

use compass_core::AppIndex;
use compass_search::rank;

use crate::message::Message;

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
    /// Ranked search results (just names for now).
    results: Vec<String>,
    /// Selected result index.
    selected: usize,
}

impl LauncherApp {
    /// Create a new launcher application.
    pub fn new(_flags: AppFlags) -> (Self, Task<Message>) {
        let app = Self {
            app_index: AppIndex::from_environment(),
            query: String::new(),
            results: Vec::new(),
            selected: 0,
        };

        (app, Task::none())
    }

    /// The application title.
    pub fn title(&self) -> String {
        "Vicinae".to_owned()
    }

    /// The application theme.
    pub fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }

    /// Update the application state.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Initialize => Task::none(),
            Message::QueryChanged(query) => {
                self.query = query;
                self.search();
                Task::none()
            }
            Message::ResultSelected(index) => {
                if index < self.results.len() {
                    self.selected = index;
                }
                Task::none()
            }
            Message::LaunchSelected => Task::none(),
            Message::ShortcutActivated(_) => Task::none(),
            Message::FocusChanged(_) => Task::none(),
            Message::WindowClosed => iced::exit(),
            Message::PollShortcuts => Task::none(),
            Message::EventOccurred(_) => Task::none(),
        }
    }

    /// View the application.
    pub fn view(&self) -> Element<'_, Message> {
        let input = text_input("Search...", &self.query)
            .on_input(Message::QueryChanged)
            .padding(12)
            .size(24)
            .on_submit(Message::LaunchSelected);

        let results_content: Element<Message> = if self.query.is_empty() {
            text("Type to search...").size(16).into()
        } else if self.results.is_empty() {
            text("No results").size(16).into()
        } else {
            let mut col = column![].spacing(4);
            for (index, name) in self.results.iter().enumerate() {
                let _is_selected = index == self.selected;
                let item_text = text(name).size(16);
                col = col.push(item_text);
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

    /// Perform search.
    fn search(&mut self) {
        if self.query.trim().is_empty() {
            self.results.clear();
            self.selected = 0;
            return;
        }

        let scored = rank(&self.query, self.app_index.items());
        self.results = scored
            .into_iter()
            .map(|s| s.item.name().to_owned())
            .collect();
        self.selected = 0;
    }
}
