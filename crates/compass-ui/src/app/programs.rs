//! Run Terminal Program in the launcher.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the decisions that do not need that state are in
//! [`crate::programs_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
};
use crate::action_panel::Action;
use crate::programs_page::{self, Act, ProgramsPage};

/// The panel's action ids: `program.action.<n>`, by position in the row's
/// actions.
const ACTION_PREFIX: &str = "program.action.";

const NEEDS_ENGINE: &str =
    "Run Terminal Program needs the Compass engine, and this window is running without one";

impl LauncherApp {
    /// Opens Run Terminal Program and asks for the programs.
    pub(super) fn open_run_program(&mut self) -> Task<Message> {
        self.page = Page::Programs(ProgramsPage::default());
        let Some(backend) = self.backend.clone() else {
            if let Page::Programs(page) = &mut self.page {
                page.apply(Err(NEEDS_ENGINE.to_owned()));
            }
            return focus_search();
        };
        Task::batch([
            Task::perform(
                async move { backend.list_programs().await },
                Message::ProgramsLoaded,
            ),
            focus_search(),
        ])
    }

    /// Carries out one of a row's actions.
    fn act(&mut self, act: Act) -> Task<Message> {
        self.panel = None;
        match act {
            Act::CopyPath(path) => self.copy_with_hud(path),
            Act::Run {
                argv,
                terminal,
                hold,
            } => {
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                Task::perform(
                    async move { backend.run_program(argv, terminal, hold).await },
                    Message::ProgramRan,
                )
            }
        }
    }

    /// Runs the selected row's first action, as Enter does.
    fn run_selected_program(&mut self) -> Task<Message> {
        let Page::Programs(page) = &self.page else {
            return Task::none();
        };
        let Some(first) = page
            .selected_row()
            .and_then(|row| page.actions(row).into_iter().next())
        else {
            return Task::none();
        };
        self.act(first.1)
    }

    /// Opens the action panel over the selected row.
    pub(super) fn open_program_panel(&mut self) -> Option<Task<Message>> {
        let Page::Programs(page) = &self.page else {
            return None;
        };
        let row = page.selected_row()?;
        let actions = page
            .actions(row)
            .into_iter()
            .enumerate()
            .map(|(position, (title, _))| {
                let action = Action::new(title).with_id(format!("{ACTION_PREFIX}{position}"));
                if position == 0 {
                    action.with_shortcut("enter")
                } else {
                    action
                }
            })
            .collect();
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions,
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a panel action, if `id` is one of this view's.
    pub(super) fn program_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let position: usize = id.strip_prefix(ACTION_PREFIX)?.parse().ok()?;
        let Page::Programs(page) = &self.page else {
            return None;
        };
        let (_, act) = page
            .selected_row()
            .and_then(|row| page.actions(row).into_iter().nth(position))?;
        Some(self.act(act))
    }

    /// The view's keys.
    pub(super) fn programs_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Programs(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.run_selected_program(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.rows.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Handles the view's messages.
    pub(super) fn program_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ProgramsLoaded(result) => {
                if let Page::Programs(page) = &mut self.page {
                    page.apply(result.map(|list| {
                        (
                            list.programs,
                            list.terminal,
                            compass_core::system_run::DefaultAction::parse(&list.default_action),
                        )
                    }));
                }
                crate::scroll::reveal_root_selection()
            }
            Message::ProgramsQueryChanged(query) => {
                if let Page::Programs(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::ProgramSelected(position) => {
                if let Page::Programs(page) = &mut self.page
                    && position < page.rows.len()
                {
                    page.selected = position;
                    return self.run_selected_program();
                }
                Task::none()
            }
            // `closeWindow({.clearRootSearch = true})`.
            Message::ProgramRan(Ok(())) => {
                self.query.clear();
                self.results.clear();
                self.conceal()
            }
            Message::ProgramRan(Err(reason)) => {
                if let Page::Programs(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The view's body.
    pub(super) fn programs_body<'a>(&'a self, page: &'a ProgramsPage) -> Element<'a, Message> {
        if let Some(notice) = page.empty_notice() {
            return self.notice(notice);
        }
        let home =
            compass_core::xdg_dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, row) in page.rows.iter().enumerate() {
            let selected = position == page.selected;
            let (title, subtitle) = programs_page::row_text(row, home.as_deref().unwrap_or(""));
            let badge = if matches!(row, programs_page::RunRow::CommandLine(_)) {
                self.initial_badge(">", selected)
            } else {
                self.initial_badge(&title, selected)
            };
            let row = self.list_row(badge, title, subtitle.filter(|_| self.subtitles), selected);
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::ProgramSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
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
    pub(super) fn programs_semantics(&self, page: &ProgramsPage) -> super::a11y::PageSemantics {
        let home =
            compass_core::xdg_dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
        let home = home.as_deref().unwrap_or("");
        let items = page
            .rows
            .iter()
            .map(|row| {
                let (title, subtitle) = programs_page::row_text(row, home);
                super::a11y::item(title, subtitle)
            })
            .collect();
        super::a11y::PageSemantics::list("Programs", items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{TestBackend, index, open_builtin, pressed, settle};
    use crate::semantics::Body;
    use std::sync::Arc;

    #[test]
    fn the_programs_are_a_list_with_their_paths_and_the_loading_notice_before_them() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = ProgramsPage::default();
        assert_eq!(
            app.programs_semantics(&page).status.as_deref(),
            Some("Looking for programs…")
        );
        page.apply(Ok((
            vec!["/opt/tools/htop".into(), "/opt/tools/top".into()],
            None,
            compass_core::system_run::DefaultAction::default(),
        )));
        page.selected = 1;
        app.page = Page::Programs(page);
        let semantics = app.semantics();
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Programs");
        let names: Vec<&str> = list.items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["htop", "top"]);
        assert!(
            list.items.iter().all(|item| item
                .description
                .as_deref()
                .is_some_and(|d| d.contains("tools/"))),
            "each program is described by its path"
        );
        assert_eq!(list.selected, Some(1));
        assert_eq!(semantics.status, None);
    }

    #[test]
    fn run_terminal_program_runs_the_typed_command_line_in_a_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "run terminal program", "commands:run-program");
        let Page::Programs(page) = &app.page else {
            panic!("not on Run Terminal Program: {}", app.state_line());
        };
        assert_eq!(page.rows.len(), 2, "every program for the empty query");

        let _ = app.update(Message::ProgramsQueryChanged("htop -d 5".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            backend.programs_ran.lock().unwrap().as_slice(),
            [(
                vec!["htop".to_owned(), "-d".to_owned(), "5".to_owned()],
                true,
                false
            )],
            "run-in-terminal is the default: a terminal that closes"
        );
        assert!(matches!(app.page, Page::Root), "running hides the launcher");
    }
}
