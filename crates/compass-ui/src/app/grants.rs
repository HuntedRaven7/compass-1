//! Script Permissions in the launcher: the Rhai scripts the user has allowed
//! something, and the extensions they have always allowed to run a program on
//! the host (`compass::host_commands`), and revoking either.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the list's state is in [`crate::grants_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
};
use crate::action_panel::Action;
use crate::grants_page::{self, GrantsPage};

const REVOKE: &str = "grants.revoke";

const NEEDS_ENGINE: &str =
    "Script Permissions needs the Compass engine, and this window is running without one";

impl LauncherApp {
    /// Opens Script Permissions and asks for the list.
    pub(super) fn open_script_grants(&mut self) -> Task<Message> {
        self.page = Page::Grants(GrantsPage::default());
        let Some(backend) = self.backend.clone() else {
            if let Page::Grants(page) = &mut self.page {
                page.apply(Err(NEEDS_ENGINE.to_owned()));
            }
            return focus_search();
        };
        Task::batch([
            Task::perform(
                async move { backend.list_script_grants().await },
                Message::GrantsLoaded,
            ),
            focus_search(),
        ])
    }

    /// Revokes everything the selected script was allowed.
    fn revoke_selected_grant(&mut self) -> Task<Message> {
        self.panel = None;
        let Page::Grants(page) = &self.page else {
            return Task::none();
        };
        let (Some(grant), Some(backend)) = (page.selected_grant(), self.backend.clone()) else {
            return Task::none();
        };
        let id = grant.id.clone();
        Task::batch([
            Task::perform(
                async move { backend.revoke_script_grant(id).await },
                Message::GrantRevoked,
            ),
            focus_search(),
        ])
    }

    /// The panel over the selected script.
    pub(super) fn open_grants_panel(&mut self) -> Option<Task<Message>> {
        let Page::Grants(page) = &self.page else {
            return None;
        };
        page.selected_grant()?;
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("Revoke permissions")
                    .with_id(REVOKE)
                    .with_shortcut("enter"),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a Script Permissions panel action, if `id` is one.
    pub(super) fn grants_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        (id == REVOKE && matches!(self.page, Page::Grants(_))).then(|| self.revoke_selected_grant())
    }

    /// The view's keys.
    pub(super) fn grants_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Grants(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.revoke_selected_grant(),
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

    /// Handles the view's messages.
    pub(super) fn grants_message(&mut self, message: Message) -> Task<Message> {
        let Page::Grants(page) = &mut self.page else {
            return Task::none();
        };
        match message {
            Message::GrantsLoaded(result) => page.apply(result),
            Message::GrantsQueryChanged(query) => {
                page.query = query;
                page.notice = None;
                page.refilter();
            }
            Message::GrantSelected(position) => {
                if position < page.shown.len() {
                    page.selected = position;
                }
            }
            Message::GrantRevoked(Ok(grants)) => {
                page.apply(Ok(grants));
                page.notice = Some(grants_page::REVOKED.to_owned());
            }
            Message::GrantRevoked(Err(reason)) => page.notice = Some(reason),
            _ => {}
        }
        crate::scroll::reveal_root_selection()
    }

    /// The view's body.
    pub(super) fn grants_body<'a>(&'a self, page: &'a GrantsPage) -> Element<'a, Message> {
        if let Some(empty) = page.empty_notice() {
            return match &page.notice {
                Some(notice) => column![self.notice(empty), self.notice(notice)].into(),
                None => self.notice(empty),
            };
        }
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let grant = &page.all[index];
            let selected = position == page.selected;
            let item = self.list_row(
                self.initial_badge(&grant.title, selected),
                grant.title.clone(),
                self.subtitles.then(|| grants_page::subtitle(grant)),
                selected,
            );
            let item: Element<Message> = mouse_area(item)
                .on_press(Message::GrantSelected(position))
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
    pub(super) fn grants_semantics(&self, page: &GrantsPage) -> super::a11y::PageSemantics {
        let items = page
            .shown
            .iter()
            .filter_map(|&index| page.all.get(index))
            .map(|grant| super::a11y::item(grant.title.clone(), Some(grants_page::subtitle(grant))))
            .collect();
        super::a11y::PageSemantics::list("Scripts", items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ScriptGrant;
    use crate::semantics::Body;

    #[test]
    fn the_scripts_are_a_list_saying_what_each_may_do() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = GrantsPage::default();
        page.apply(Ok(Vec::new()));
        assert_eq!(
            app.grants_semantics(&page).status.as_deref(),
            Some(grants_page::EMPTY)
        );
        page.apply(Ok(vec![ScriptGrant {
            id: "script.notes".into(),
            title: "Quick Notes".into(),
            capabilities: Vec::new(),
            descriptions: vec!["read its saved data".into(), "save data".into()],
        }]));
        page.notice = Some(grants_page::REVOKED.to_owned());
        app.page = Page::Grants(page);
        let semantics = app.semantics();
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Scripts");
        assert_eq!(list.items[0].name, "Quick Notes");
        assert_eq!(
            list.items[0].description.as_deref(),
            Some("Read its saved data, save data")
        );
        assert_eq!(list.selected, Some(0));
        assert_eq!(semantics.status.as_deref(), Some(grants_page::REVOKED));
    }
}
