//! Set Theme in the launcher: moving the selection previews a theme, Enter
//! keeps it, leaving goes back to the one configured.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the sections and filter are in [`crate::themes_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, row,
    scrollable, text,
};
use crate::action_panel::Action;
use crate::themes_page::ThemesPage;
use compass_core::theme_picker;

const SET: &str = "theme.set";
const OPEN_FILE: &str = "theme.open-file";
const COPY_ID: &str = "theme.copy-id";
const COPY_PATH: &str = "theme.copy-path";

impl LauncherApp {
    /// Opens Set Theme over the theme in use, reading the theme files anew
    /// so one added since the last opening is offered.
    pub(super) fn open_set_theme(&mut self) -> Task<Message> {
        let files = crate::theme::load_user_themes(&self.view.theme_dirs);
        self.page = Page::Themes(ThemesPage::new(self.view.theme_choice, files));
        focus_search()
    }

    /// The panel over the selected theme: `ThemeViewHost`'s actions.
    pub(super) fn open_theme_panel(&mut self) -> Option<Task<Message>> {
        let Page::Themes(page) = &self.page else {
            return None;
        };
        let theme = page.selected_theme()?;
        let picked = theme_picker::Theme {
            id: theme.name().to_owned(),
            name: theme.title().to_owned(),
            description: theme.description().to_owned(),
            icon: None,
            path: theme.path().map(str::to_owned),
        };
        let actions = theme_picker::action_panel(&picked, self.backend.is_some())
            .into_iter()
            .map(|action| {
                let id = match action.title {
                    theme_picker::SET_THEME_TITLE => SET,
                    theme_picker::OPEN_FILE_TITLE => OPEN_FILE,
                    theme_picker::COPY_ID_TITLE => COPY_ID,
                    _ => COPY_PATH,
                };
                let item = Action::new(action.title).with_id(id);
                if id == SET {
                    item.with_shortcut("enter")
                } else {
                    item
                }
            })
            .collect();
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions,
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a Set Theme panel action, if `id` is one.
    pub(super) fn theme_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if ![SET, OPEN_FILE, COPY_ID, COPY_PATH].contains(&id) {
            return None;
        }
        let Page::Themes(page) = &self.page else {
            return None;
        };
        let theme = page.selected_theme()?;
        self.panel = None;
        Some(match id {
            SET => self.keep_selected_theme(),
            COPY_ID => super::hud::copy_text(self.backend.clone(), theme.name().to_owned()),
            COPY_PATH => super::hud::copy_text(self.backend.clone(), theme.path()?.to_owned()),
            _ => {
                let path = theme.path()?.to_owned();
                let backend = self.backend.clone()?;
                let opened = Task::perform(
                    async move { backend.open_file(path, false).await },
                    Message::BuiltinCommandDone,
                );
                Task::batch([opened, self.conceal()])
            }
        })
    }

    /// Previews the selected theme, as selecting a row applies it in the
    /// C++ view.
    fn preview_selected_theme(&mut self) -> Task<Message> {
        let Page::Themes(page) = &self.page else {
            return Task::none();
        };
        match page.selected_theme() {
            Some(theme) if theme != self.view.theme_choice => {
                self.update(Message::ThemePreview(theme))
            }
            _ => Task::none(),
        }
    }

    /// Keeps the selected theme: in the configuration through the engine,
    /// and in this window.
    fn keep_selected_theme(&mut self) -> Task<Message> {
        let Page::Themes(page) = &self.page else {
            return Task::none();
        };
        let Some(theme) = page.selected_theme() else {
            return Task::none();
        };
        let preview = self.update(Message::ThemePreview(theme));
        let Some(backend) = self.backend.clone() else {
            if let Page::Themes(page) = &mut self.page {
                page.notice =
                    Some("Set Theme needs the Compass engine to keep the theme".to_owned());
            }
            return preview;
        };
        let name = theme.name().to_owned();
        Task::batch([
            preview,
            Task::perform(
                async move { backend.set_theme(name).await },
                Message::ThemeSaved,
            ),
        ])
    }

    /// Leaves Set Theme, putting back the theme it opened with.
    pub(super) fn leave_set_theme(&mut self) -> Task<Message> {
        let cancel = self.update(Message::ThemeCancel);
        self.page = Page::Root;
        Task::batch([cancel, focus_search()])
    }

    /// The view's keys.
    pub(super) fn themes_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Themes(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.leave_set_theme(),
            Key::Named(Named::Enter) => return self.keep_selected_theme(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.rows.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            let reveal = crate::scroll::reveal_root_selection();
            return Task::batch([self.preview_selected_theme(), reveal]);
        }
        Task::none()
    }

    /// Handles the view's messages.
    pub(super) fn theme_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ThemesQueryChanged(query) => {
                if let Page::Themes(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                Task::batch([
                    self.preview_selected_theme(),
                    crate::scroll::reveal_root_selection(),
                ])
            }
            Message::ThemeSelected(position) => {
                if let Page::Themes(page) = &mut self.page
                    && position < page.rows.len()
                {
                    page.selected = position;
                    return self.keep_selected_theme();
                }
                Task::none()
            }
            Message::ThemeSaved(Ok(())) => {
                let commit = self.update(Message::ThemeCommit);
                if let Page::Themes(page) = &mut self.page {
                    page.configured = self.view.theme_choice;
                    page.refilter();
                }
                commit
            }
            Message::ThemeSaved(Err(reason)) => {
                if let Page::Themes(page) = &mut self.page {
                    page.notice = Some(reason);
                } else if let Page::Onboarding(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The view's body.
    pub(super) fn themes_body<'a>(&'a self, page: &'a ThemesPage) -> Element<'a, Message> {
        if let Some(notice) = page.empty_notice() {
            return self.notice(notice);
        }
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, row) in page.rows.iter().enumerate() {
            if let Some(heading) = row.heading {
                list = list.push(
                    container(
                        text(heading)
                            .font(self.font())
                            .size(12)
                            .color(self.palette().muted.to_iced()),
                    )
                    .padding(Padding::new(4.0).left(10)),
                );
            }
            let selected = position == page.selected;
            let item = self.list_row(
                self.initial_badge(row.theme.title(), selected),
                row.theme.title().to_owned(),
                self.subtitles.then(|| row.subtitle().to_owned()),
                selected,
            );
            let item: Element<Message> = match row.theme.swatches() {
                Some(swatches) => {
                    let mut dots = row![].spacing(4).align_y(iced::Alignment::Center);
                    for colour in swatches {
                        let fill = colour.to_iced();
                        dots = dots.push(
                            container(iced::widget::Space::new())
                                .width(Length::Fixed(10.0))
                                .height(Length::Fixed(10.0))
                                .style(move |_: &iced::Theme| container::Style {
                                    background: Some(fill.into()),
                                    border: iced::Border {
                                        radius: 5.0.into(),
                                        ..iced::Border::default()
                                    },
                                    ..container::Style::default()
                                }),
                        );
                    }
                    iced::widget::stack![
                        item,
                        container(dots)
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .align_x(iced::Alignment::End)
                            .align_y(iced::Alignment::Center)
                            .padding(Padding::new(0.0).right(14)),
                    ]
                    .into()
                }
                None => item,
            };
            let item: Element<Message> = mouse_area(item)
                .on_press(Message::ThemeSelected(position))
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

    /// The view, for assistive technology.
    pub(super) fn themes_semantics(&self, page: &ThemesPage) -> super::a11y::PageSemantics {
        let items = page
            .rows
            .iter()
            .map(|row| super::a11y::item(row.theme.title(), Some(row.subtitle().to_owned())))
            .collect();
        super::a11y::PageSemantics::list("Themes", items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{TestBackend, index, open_builtin, pressed, settle};
    use crate::semantics::Body;
    use crate::theme::Theme;
    use std::sync::Arc;

    #[test]
    fn the_themes_are_a_list_with_their_descriptions_and_say_when_none_match() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        app.page = Page::Themes(ThemesPage::new(Theme::System, Vec::new()));
        let semantics = app.semantics();
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Themes");
        assert_eq!(list.items.len(), Theme::ALL.len());
        assert_eq!(list.items[0].name, "System");
        assert_eq!(
            list.items[0].description.as_deref(),
            Some("Follow OS (Adwaita)")
        );
        assert_eq!(list.selected, Some(0));

        let Page::Themes(page) = &mut app.page else {
            unreachable!("the page was just set");
        };
        page.query = "%%%%".into();
        page.refilter();
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(semantics.status.as_deref(), Some("No themes match"));
    }

    #[test]
    fn set_theme_previews_as_the_selection_moves_and_escape_puts_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.view.theme_choice = crate::theme::Theme::Nord;
        open_builtin(&mut app, "set theme", "commands:set-theme");
        assert!(
            app.state_line().contains("page=themes"),
            "{}",
            app.state_line()
        );

        let task = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        settle(&mut app, task);
        assert_ne!(
            app.view.theme_choice,
            crate::theme::Theme::Nord,
            "previewed"
        );
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Nord, "put back");
        assert!(matches!(app.page, Page::Root));
        assert!(backend.themes_kept.lock().unwrap().is_empty());
    }

    #[test]
    fn set_theme_keeps_the_chosen_theme() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.view.theme_choice = crate::theme::Theme::System;
        open_builtin(&mut app, "set theme", "commands:set-theme");
        let _ = app.update(Message::ThemesQueryChanged("dracula".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(backend.themes_kept.lock().unwrap().as_slice(), ["dracula"]);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Dracula);
        let task = app.update(Message::Dismiss);
        settle(&mut app, task);
        assert_eq!(
            app.view.theme_choice,
            crate::theme::Theme::Dracula,
            "a kept theme stays when the launcher hides"
        );
    }
}
