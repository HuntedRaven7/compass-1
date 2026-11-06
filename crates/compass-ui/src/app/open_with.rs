//! "Open with…": the app-selector view over a target, opened from a
//! shortcut, a file or a clipboard entry, which Escape leaves for the view
//! it came from.

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, Task, chord_direction, column,
    container, focus_search, mouse_area, next_selection, scrollable,
};
use crate::backend::OPEN_WITH_NEEDS_ENGINE;
use crate::open_with_page::OpenWithPage;

impl LauncherApp {
    /// Opens the selector for `target`, its applications looked up by
    /// `lookup`, keeping the view showing to come back to.
    pub(super) fn open_with(&mut self, target: String, lookup: String) -> Task<Message> {
        self.panel = None;
        let page = OpenWithPage::new(target, lookup.clone());
        let previous = std::mem::replace(&mut self.page, Page::OpenWith(page));
        if !matches!(previous, Page::OpenWith(_)) {
            self.open_with_return = Some(Box::new(previous));
        }
        let Some(backend) = self.backend.clone() else {
            if let Page::OpenWith(page) = &mut self.page {
                page.apply(Err(OPEN_WITH_NEEDS_ENGINE.to_owned()));
            }
            return focus_search();
        };
        Task::batch([
            Task::perform(
                async move { backend.list_openers(lookup).await },
                Message::OpenersLoaded,
            ),
            focus_search(),
        ])
    }

    /// Shows why an action did not happen, in the view it was taken from.
    pub(super) fn say(&mut self, reason: String) {
        match &mut self.page {
            Page::Shortcuts(page) => page.notice = Some(reason),
            Page::Files(page) => page.notice = Some(reason),
            Page::Clipboard(page) => page.notice = Some(reason),
            Page::OpenWith(page) => page.notice = Some(reason),
            Page::Windows(page) => page.notice = Some(reason),
            _ => self.error = Some(reason),
        }
    }

    /// Escape from the selector: back to the view it was opened over.
    pub(super) fn back_from_open_with(&mut self) -> Option<Task<Message>> {
        if !matches!(self.page, Page::OpenWith(_)) {
            return None;
        }
        self.page = self
            .open_with_return
            .take()
            .map_or(Page::Root, |previous| *previous);
        Some(focus_search())
    }

    fn open_with_selected(&mut self) -> Task<Message> {
        let Page::OpenWith(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(backend)) = (page.selected_row(), self.backend.clone()) else {
            return Task::none();
        };
        let (app, target) = (row.id.clone(), page.target.clone());
        Task::perform(
            async move { backend.open_with(app, target).await },
            Message::OpenedWith,
        )
    }

    /// The selector's keys.
    pub(super) fn open_with_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::OpenWith(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.open_with_selected(),
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

    /// Handles the selector's messages.
    pub(super) fn open_with_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenWithTarget(Ok((target, lookup))) => self.open_with(target, lookup),
            Message::OpenWithTarget(Err(reason)) => {
                self.say(reason);
                Task::none()
            }
            Message::OpenersLoaded(result) => {
                if let Page::OpenWith(page) = &mut self.page {
                    page.apply(result);
                }
                Task::none()
            }
            Message::OpenWithQueryChanged(query) => {
                if let Page::OpenWith(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::OpenWithSelected(position) => {
                if let Page::OpenWith(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.open_with_selected()
            }
            Message::OpenedWith(Ok(())) => {
                self.open_with_return = None;
                self.conceal()
            }
            Message::OpenedWith(Err(reason)) => {
                if let Page::OpenWith(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The selector's body.
    pub(super) fn open_with_body<'a>(&'a self, page: &'a OpenWithPage) -> Element<'a, Message> {
        if let Some(empty) = page.empty_notice() {
            return self.notice(empty);
        }
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(app) = page.all.get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let row = self.list_row_with(
                self.initial_badge(&app.name, selected),
                app.name.clone(),
                None,
                default_label(app),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::OpenWithSelected(position))
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

    /// The selector's applications, for assistive technology.
    pub(super) fn open_with_semantics(&self, page: &OpenWithPage) -> super::a11y::PageSemantics {
        let items = page
            .shown
            .iter()
            .map(|&index| {
                page.all
                    .get(index)
                    .map_or_else(crate::semantics::Item::default, |app| {
                        super::a11y::item(app.name.clone(), default_label(app))
                    })
            })
            .collect();
        super::a11y::PageSemantics::list("Applications", items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }
}

/// What an application's row says beside its name: that it is the default.
fn default_label(app: &crate::backend::OpenerRow) -> Option<String> {
    app.default.then(|| "Default".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::OpenerRow;
    use crate::semantics::Body;

    fn opener(id: &str, name: &str, default: bool) -> OpenerRow {
        OpenerRow {
            id: id.into(),
            name: name.into(),
            icon: None,
            default,
        }
    }

    #[test]
    fn the_selector_names_each_application_and_the_default() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = OpenWithPage::new("/tmp/a.png".into(), "/tmp/a.png".into());
        assert_eq!(
            app.open_with_semantics(&page).status.as_deref(),
            Some("Finding applications…")
        );
        page.apply(Ok(vec![
            opener("org.gnome.Loupe.desktop", "Image Viewer", true),
            opener("gimp.desktop", "GIMP", false),
        ]));
        page.selected = 1;
        page.notice = Some("GIMP could not be opened.".into());
        let semantics = app.open_with_semantics(&page);
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.items[0].name, "Image Viewer");
        assert_eq!(list.items[0].description.as_deref(), Some("Default"));
        assert_eq!(list.items[1].name, "GIMP");
        assert_eq!(list.items[1].description, None);
        assert_eq!(list.selected, Some(1));
        assert_eq!(
            semantics.status.as_deref(),
            Some("GIMP could not be opened.")
        );

        page.apply(Ok(Vec::new()));
        app.page = Page::OpenWith(page);
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(
            semantics.status.as_deref(),
            Some("No application opens this")
        );
    }
}
