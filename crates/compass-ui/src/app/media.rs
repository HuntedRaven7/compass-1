//! The media commands in the launcher: running one at once or with its
//! argument, and the Now Playing view.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the decisions that do not need that state are in
//! [`crate::media_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    RootRow, Task, chord_direction, column, container, focus_search, mouse_area, next_selection,
    scrollable,
};
use crate::action_panel::Action;
use crate::backend::{MediaAction, PreferenceInput, PreferenceInputKind};
use crate::media_page::{self, NowPlayingPage};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};
use compass_core::commands::{BuiltinCommand, CommandKind};

const RUN: &str = "media.run";
const RUN_WITH: &str = "media.run-with";
/// Now Playing's panel actions: `media.player.<n>`, by position in the
/// row's actions.
const PLAYER_ACTION: &str = "media.player.";

/// How long after an action the players are asked again, so the list shows
/// the state the player moved to rather than the one it was leaving.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

const NEEDS_ENGINE: &str =
    "Now Playing needs the Compass engine, and this window is running without one";

impl LauncherApp {
    /// Runs a media command, with what was entered for its argument; the
    /// launcher hides at once, as a callback command does.
    pub(super) fn run_media(
        &mut self,
        command: &'static BuiltinCommand,
        id: &'static str,
        argument: Option<String>,
    ) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            self.error = Some(format!(
                "{} needs the Compass engine, and this window is running without one",
                command.title
            ));
            return Task::none();
        };
        let run = Task::perform(
            async move { backend.run_media_command(id.to_owned(), argument).await },
            Message::BuiltinCommandDone,
        );
        Task::batch([self.conceal(), run])
    }

    /// The form a media command's optional argument is entered in: the
    /// player to act on, or the volume step.
    fn open_media_arguments(&mut self, command: &'static BuiltinCommand) -> Task<Message> {
        let CommandKind::Media(id) = command.kind else {
            return Task::none();
        };
        let Some((name, placeholder)) = compass_core::media_commands::command_argument(id) else {
            return Task::none();
        };
        self.panel = None;
        let field = PreferenceInput {
            name: name.to_owned(),
            title: if name == "player" {
                "Player".to_owned()
            } else {
                "Step".to_owned()
            },
            description: String::new(),
            placeholder: placeholder.to_owned(),
            required: false,
            kind: PreferenceInputKind::Text,
            value: None,
        };
        self.page = Page::Preferences(Box::new(PreferencesPage::new(
            Purpose::MediaArguments,
            command.entrypoint.to_owned(),
            command.title.to_owned(),
            vec![field],
        )));
        Task::none()
    }

    /// Submits a media command's form. `None` when the form showing is not
    /// one.
    pub(super) fn submit_media_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(page) = &self.page else {
            return None;
        };
        if page.purpose != Purpose::MediaArguments {
            return None;
        }
        let command = compass_core::commands::BUILTIN_COMMANDS
            .iter()
            .find(|command| command.entrypoint == page.command_id)?;
        let CommandKind::Media(id) = command.kind else {
            return None;
        };
        let argument = match page.values.first() {
            Some(FieldValue::Text(text)) if !text.trim().is_empty() => Some(text.trim().to_owned()),
            _ => None,
        };
        self.page = Page::Root;
        Some(self.run_media(command, id, argument))
    }

    /// Opens Now Playing and asks for the players.
    pub(super) fn open_now_playing(&mut self) -> Task<Message> {
        self.page = Page::NowPlaying(NowPlayingPage::default());
        Task::batch([self.list_players(std::time::Duration::ZERO), focus_search()])
    }

    /// Asks for the players, after `delay`.
    fn list_players(&mut self, delay: std::time::Duration) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            if let Page::NowPlaying(page) = &mut self.page {
                page.apply(Err(NEEDS_ENGINE.to_owned()));
            }
            return Task::none();
        };
        Task::perform(
            async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                backend.list_media_players().await
            },
            Message::NowPlayingLoaded,
        )
    }

    /// Asks the selected player to do one of its actions.
    fn control_selected_player(&mut self, action: MediaAction) -> Task<Message> {
        self.panel = None;
        let Page::NowPlaying(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(backend)) = (page.selected_row(), self.backend.clone()) else {
            return Task::none();
        };
        let player = row.id.clone();
        Task::batch([
            Task::perform(
                async move { backend.control_media_player(player, action).await },
                Message::NowPlayingActed,
            ),
            focus_search(),
        ])
    }

    /// The panel over a media command's root row, or over a player.
    pub(super) fn open_media_panel(&mut self) -> Option<Task<Message>> {
        let actions = match &self.page {
            Page::NowPlaying(page) => {
                let row = page.selected_row()?;
                media_page::actions(row)
                    .into_iter()
                    .enumerate()
                    .map(|(position, (title, _))| {
                        let action =
                            Action::new(title).with_id(format!("{PLAYER_ACTION}{position}"));
                        if position == 0 {
                            action.with_shortcut("enter")
                        } else {
                            action
                        }
                    })
                    .collect()
            }
            Page::Root => {
                let RootRow::Command(command) = self.selected_row()? else {
                    return None;
                };
                let CommandKind::Media(id) = command.kind else {
                    return None;
                };
                let (name, _) = compass_core::media_commands::command_argument(id)?;
                vec![
                    Action::new(command.title)
                        .with_id(RUN)
                        .with_shortcut("enter"),
                    Action::new(if name == "player" {
                        "Choose player…"
                    } else {
                        "Choose step…"
                    })
                    .with_id(RUN_WITH),
                ]
            }
            _ => return None,
        };
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions,
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a media panel action, if `id` is one.
    pub(super) fn media_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if let Some(position) = id.strip_prefix(PLAYER_ACTION) {
            let position: usize = position.parse().ok()?;
            let Page::NowPlaying(page) = &self.page else {
                return None;
            };
            let (_, action) = media_page::actions(page.selected_row()?)
                .into_iter()
                .nth(position)?;
            return Some(self.control_selected_player(action));
        }
        if id != RUN && id != RUN_WITH {
            return None;
        }
        let RootRow::Command(command) = self.selected_row()? else {
            return None;
        };
        let CommandKind::Media(media) = command.kind else {
            return None;
        };
        self.panel = None;
        if id == RUN_WITH {
            return Some(self.open_media_arguments(command));
        }
        Some(self.run_media(command, media, None))
    }

    /// Now Playing's keys.
    pub(super) fn now_playing_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::NowPlaying(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => {
                return self.control_selected_player(MediaAction::PlayPause);
            }
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.shown.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Handles Now Playing's messages.
    pub(super) fn media_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::NowPlayingLoaded(result) => {
                if let Page::NowPlaying(page) = &mut self.page {
                    page.apply(result);
                }
                Task::none()
            }
            Message::NowPlayingQueryChanged(query) => {
                if let Page::NowPlaying(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::NowPlayingSelected(position) => {
                if let Page::NowPlaying(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                    return self.control_selected_player(MediaAction::PlayPause);
                }
                Task::none()
            }
            Message::NowPlayingActed(result) => {
                let Page::NowPlaying(page) = &mut self.page else {
                    return Task::none();
                };
                match result {
                    Ok(()) => {
                        page.notice = None;
                        self.list_players(SETTLE)
                    }
                    Err(reason) => {
                        page.notice = Some(reason);
                        Task::none()
                    }
                }
            }
            _ => Task::none(),
        }
    }

    /// Now Playing's body.
    pub(super) fn now_playing_body<'a>(&'a self, page: &'a NowPlayingPage) -> Element<'a, Message> {
        if let Some(notice) = page.empty_notice() {
            return self.notice(notice);
        }
        let mut list = column![
            container(
                iced::widget::text(media_page::SECTION)
                    .font(self.font())
                    .size(12)
                    .color(self.palette().muted.to_iced()),
            )
            .padding(Padding::new(4.0).left(10))
        ]
        .spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let row = &page.all[index];
            let selected = position == page.selected;
            let title = media_page::title(row).to_owned();
            let subtitle = media_page::subtitle(row);
            let badge = self.initial_badge(&row.identity, selected);
            let item = self.list_row(badge, title, subtitle.filter(|_| self.subtitles), selected);
            let item: Element<Message> = mouse_area(item)
                .on_press(Message::NowPlayingSelected(position))
                .into();
            let item: Element<Message> = if selected {
                container(item).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                item
            };
            list = list.push(item);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// Now Playing, for assistive technology.
    pub(super) fn now_playing_semantics(
        &self,
        page: &NowPlayingPage,
    ) -> super::a11y::PageSemantics {
        let items = page
            .shown
            .iter()
            .filter_map(|&index| page.all.get(index))
            .map(|row| super::a11y::item(media_page::title(row), media_page::subtitle(row)))
            .collect();
        super::a11y::PageSemantics::list(
            media_page::SECTION,
            items,
            page.selected,
            page.empty_notice(),
        )
        .with_status(page.notice.as_deref(), &page.query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{TestBackend, extension_app, pressed, task_messages};
    use crate::backend::MediaPlayerRow;
    use crate::semantics::Body;
    use std::sync::Arc;

    #[test]
    fn the_players_are_a_list_with_artist_and_state() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = NowPlayingPage::default();
        page.apply(Ok(Vec::new()));
        assert_eq!(
            app.now_playing_semantics(&page).status.as_deref(),
            Some("No media player is running")
        );
        page.apply(Ok(vec![
            MediaPlayerRow {
                id: "a".into(),
                identity: "Firefox".into(),
                ..MediaPlayerRow::default()
            },
            MediaPlayerRow {
                id: "b".into(),
                identity: "Spotify".into(),
                title: "Blue Monday".into(),
                artist: "New Order".into(),
                playing: true,
                ..MediaPlayerRow::default()
            },
        ]));
        app.page = Page::NowPlaying(page);
        let semantics = app.semantics();
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Players");
        assert_eq!(list.items[0].name, "Firefox");
        assert_eq!(list.items[0].description, None);
        assert_eq!(list.items[1].name, "Blue Monday");
        assert_eq!(
            list.items[1].description.as_deref(),
            Some("New Order  ·  Playing")
        );
        assert_eq!(list.selected, Some(0));
    }

    #[test]
    fn a_media_command_runs_at_once_and_shows_why_it_did_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["commands:next-track".to_owned()],
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        for message in task_messages(app.update(Message::QueryChanged("next track".into()))) {
            let _ = app.update(message);
        }
        app.selected = app
            .results
            .iter()
            .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == "next-track"))
            .expect("Next Track is in root search");
        let mut pending = task_messages(app.update(Message::LaunchSelected));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        assert!(app.power_confirm.is_none(), "media commands do not ask");
        assert_eq!(backend.played.lock().unwrap().as_slice(), ["next-track"]);
        assert_eq!(app.error.as_deref(), Some("No media player is running"));
    }

    #[test]
    fn a_media_command_runs_with_the_player_chosen_in_its_form() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["commands:play-pause".to_owned()],
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        for message in task_messages(app.update(Message::QueryChanged("play pause".into()))) {
            let _ = app.update(message);
        }
        app.selected = app
            .results
            .iter()
            .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == "play-pause"))
            .expect("Play / Pause is in root search");
        let _ = app.update(Message::TogglePanel);
        let panel = app.panel.as_ref().expect("a media command has a panel");
        let titles: Vec<&str> = panel.sections[0]
            .actions
            .iter()
            .map(|a| a.title.as_str())
            .collect();
        assert_eq!(titles, ["Play / Pause", "Choose player…"]);
        let _ = app.update(Message::PanelMove(Direction::Down));
        let _ = app.update(Message::PanelActivate);
        assert!(
            matches!(&app.page, Page::Preferences(page)
                if page.purpose == crate::preferences_page::Purpose::MediaArguments),
            "{}",
            app.state_line()
        );
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text(" spotify ".into()),
        ));
        let mut pending = task_messages(app.update(pressed(iced::keyboard::key::Named::Enter)));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        assert_eq!(
            backend.played.lock().unwrap().as_slice(),
            ["play-pause spotify"]
        );
    }

    #[test]
    fn now_playing_lists_the_players_and_controls_the_selected_one() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _entered = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["commands:now-playing".to_owned()],
            ..TestBackend::default()
        });
        *backend.players.lock().unwrap() = vec![
            crate::backend::MediaPlayerRow {
                id: "org.mpris.MediaPlayer2.firefox".into(),
                identity: "Firefox".into(),
                ..Default::default()
            },
            crate::backend::MediaPlayerRow {
                id: "org.mpris.MediaPlayer2.spotify".into(),
                identity: "Spotify".into(),
                title: "Blue Monday".into(),
                artist: "New Order".into(),
                playing: true,
                can_go_next: true,
                ..Default::default()
            },
        ];
        let mut app = extension_app(dir.path(), backend.clone());
        for message in task_messages(app.update(Message::QueryChanged("now playing".into()))) {
            let _ = app.update(message);
        }
        app.selected = app
            .results
            .iter()
            .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == "now-playing"))
            .expect("Now Playing is in root search");
        let mut pending = task_messages(app.update(Message::LaunchSelected));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        let Page::NowPlaying(page) = &app.page else {
            panic!("Now Playing did not open: {}", app.state_line());
        };
        assert_eq!(page.shown.len(), 2);

        for message in task_messages(app.update(Message::NowPlayingQueryChanged("order".into()))) {
            let _ = app.update(message);
        }
        let _ = app.update(Message::TogglePanel);
        let titles: Vec<String> = app.panel.as_ref().expect("a player has a panel").sections[0]
            .actions
            .iter()
            .map(|a| a.title.clone())
            .collect();
        assert_eq!(titles, ["Pause", "Next Track"]);
        let _ = app.update(Message::TogglePanel);

        let mut pending = task_messages(app.update(pressed(iced::keyboard::key::Named::Enter)));
        while let Some(message) = pending.pop() {
            pending.extend(task_messages(app.update(message)));
        }
        assert_eq!(
            backend.controlled.lock().unwrap().as_slice(),
            [(
                "org.mpris.MediaPlayer2.spotify".to_owned(),
                crate::backend::MediaAction::PlayPause
            )]
        );
        let Page::NowPlaying(page) = &app.page else {
            panic!("Now Playing closed: {}", app.state_line());
        };
        let row = page.selected_row().expect("still selected");
        assert!(!row.playing && row.paused, "the list was reloaded");
    }
}
