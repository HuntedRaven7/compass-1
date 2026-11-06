//! The dmenu view in the launcher: shown when the engine pushes a
//! `compass dmenu` list, answered when the person chooses or dismisses it.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the decisions that do not need that state are in
//! [`crate::dmenu_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, mouse_area, next_selection, row, scrollable, text,
};
use crate::action_panel::Action;
use crate::design::GEOMETRY;
use crate::dmenu_page::{self, DmenuPage, Status};

const SELECT: &str = "dmenu.select";
const PASS_TEXT: &str = "dmenu.pass-text";
const SELECT_AND_COPY: &str = "dmenu.select-copy";

impl LauncherApp {
    /// The card size the dmenu list showing asks for with `--width` and
    /// `--height`, the side not given keeping the launcher's own.
    pub(super) fn dmenu_card_size(&self) -> Option<(u32, u32)> {
        let Page::Dmenu(page) = &self.page else {
            return None;
        };
        page.window_size((
            u32::from(GEOMETRY.card_width),
            u32::from(GEOMETRY.card_max_height),
        ))
    }

    /// Resizes the open window to what the dmenu list asks for, or back to
    /// the launcher's own size after a list that asked (`requestWindowSize`).
    pub(super) fn apply_dmenu_size(&mut self) -> Task<Message> {
        let Some(id) = self.window else {
            return Task::none();
        };
        let wanted = self.dmenu_card_size();
        if wanted == self.resized_to {
            return Task::none();
        }
        self.resized_to = wanted;
        // A layer surface still measuring its output gets this size when it
        // has measured (`window_sized`).
        if self.measuring == Some(id) {
            return Task::none();
        }
        crate::surface::resize(id, self.wanted_window_size())
    }

    /// Puts up the dmenu view for `token` and asks the engine for its list.
    pub(super) fn start_dmenu(&mut self, token: u64) -> Task<Message> {
        self.panel = None;
        let _ = self.close_extension_view();
        self.page = Page::Dmenu(DmenuPage::loading(token));
        let Some(backend) = self.backend.clone() else {
            if let Page::Dmenu(page) = &mut self.page {
                page.apply(Err("dmenu needs the Compass engine".to_owned()));
            }
            return Task::none();
        };
        Task::perform(
            async move { backend.fetch_dmenu(token).await },
            move |result| Message::DmenuLoaded { token, result },
        )
    }

    /// Sends the choice (or the dismissal, as `None`) for the dmenu view,
    /// once.
    fn answer_dmenu(&mut self, output: Option<String>) -> Task<Message> {
        let Page::Dmenu(page) = &mut self.page else {
            return Task::none();
        };
        if page.answered {
            return Task::none();
        }
        page.answered = true;
        let token = page.token;
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.choose_dmenu(token, output).await },
            Message::DmenuChosen,
        )
    }

    /// Dismisses a dmenu list that was not answered, when the view goes away
    /// for any reason: Escape, the window hiding, another command.
    pub(super) fn cancel_dmenu(&mut self) -> Task<Message> {
        self.answer_dmenu(None)
    }

    /// Chooses: the selected entry (or its index), else the search text; and
    /// hides the launcher, as `selectEntry` closes the window.
    fn choose_dmenu(&mut self, output: String, copy: Option<String>) -> Task<Message> {
        let answer = self.answer_dmenu(Some(output));
        let backend = self.backend.clone();
        let copied = copy.map_or_else(Task::none, |text| super::hud::copy_text(backend, text));
        Task::batch([answer, copied, self.conceal()])
    }

    /// The view's keys: navigation, Enter to choose, Escape to dismiss.
    pub(super) fn dmenu_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Dmenu(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => {
                let output = page.selected_output();
                return self.choose_dmenu(output, None);
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
            page.refresh_preview();
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// The panel over the dmenu view: `DMenuSection::actionPanel`, or the
    /// empty list's "Pass search text".
    pub(super) fn open_dmenu_panel(&mut self) -> Option<Task<Message>> {
        let Page::Dmenu(page) = &self.page else {
            return None;
        };
        let actions = if page.selected_entry().is_some() {
            vec![
                Action::new(if page.list.output_index {
                    "Select entry (index)"
                } else {
                    "Select entry"
                })
                .with_id(SELECT)
                .with_shortcut("enter"),
                Action::new("Pass search text").with_id(PASS_TEXT),
                Action::new("Select and copy entry").with_id(SELECT_AND_COPY),
            ]
        } else {
            vec![
                Action::new("Pass search text")
                    .with_id(PASS_TEXT)
                    .with_shortcut("enter"),
            ]
        };
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions,
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a dmenu panel action, if `id` is one.
    pub(super) fn dmenu_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let Page::Dmenu(page) = &self.page else {
            return None;
        };
        let (output, copy) = match id {
            SELECT => (page.selected_output(), None),
            PASS_TEXT => (page.query.clone(), None),
            SELECT_AND_COPY => {
                let entry = page.selected_entry()?.to_owned();
                (entry.clone(), Some(entry))
            }
            _ => return None,
        };
        self.panel = None;
        Some(self.choose_dmenu(output, copy))
    }

    /// Handles the dmenu messages.
    pub(super) fn dmenu_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::DmenuLoaded { token, result } => {
                if let Page::Dmenu(page) = &mut self.page
                    && page.token == token
                {
                    page.apply(result);
                }
                Task::batch([
                    self.apply_dmenu_size(),
                    crate::scroll::reveal_root_selection(),
                ])
            }
            Message::DmenuQueryChanged(query) => {
                if let Page::Dmenu(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::DmenuSelected(position) => {
                let Page::Dmenu(page) = &mut self.page else {
                    return Task::none();
                };
                if position >= page.shown.len() {
                    return Task::none();
                }
                page.selected = position;
                let output = page.selected_output();
                self.choose_dmenu(output, None)
            }
            Message::DmenuChosen(Err(reason)) => {
                tracing::warn!(%reason, "could not answer a dmenu list");
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The view's body: the list, quick look beside it when the selected
    /// entry is a file, and the footer unless `--no-footer`.
    pub(super) fn dmenu_body<'a>(&'a self, page: &'a DmenuPage) -> Element<'a, Message> {
        let list = self.dmenu_list(page);
        let body: Element<'a, Message> = match &page.preview {
            Some(preview) => row![
                container(list).width(Length::FillPortion(super::preview::LIST_PORTION)),
                self.file_preview_pane(preview, !page.list.no_metadata),
            ]
            .into(),
            None => list,
        };
        if page.list.no_footer || page.status != Status::Ready {
            return body;
        }
        let primary = match (page.selected_entry(), page.list.output_index) {
            (None, _) => "Pass search text",
            (Some(_), false) => "Select entry",
            (Some(_), true) => "Select entry (index)",
        };
        column![
            body,
            self.footer(page.list.navigation_title.as_deref(), Some(primary))
        ]
        .into()
    }

    /// The list itself.
    fn dmenu_list<'a>(&'a self, page: &'a DmenuPage) -> Element<'a, Message> {
        let empty = page.empty_notice();
        if let Some(notice) = empty
            && page.status != Status::Ready
        {
            return self.notice(notice);
        }
        let quick_look = !page.list.no_quick_look;
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        if let Some(heading) = page.heading() {
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
        if let Some(notice) = empty {
            return column![list, self.notice(notice)].into();
        }
        for (position, &index) in page.shown.iter().enumerate() {
            let selected = position == page.selected;
            let (title, subtitle) = dmenu_page::entry_text(&page.entries[index], quick_look);
            let row = self.list_row(
                self.initial_badge(&title, selected),
                title,
                subtitle.filter(|_| self.subtitles),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::DmenuSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
        }
        scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }

    /// The view, for assistive technology: the entries under their heading,
    /// and quick look's preview beside them.
    pub(super) fn dmenu_semantics(&self, page: &DmenuPage) -> super::a11y::PageSemantics {
        let quick_look = !page.list.no_quick_look;
        let items = page
            .shown
            .iter()
            .map(|&index| {
                let (title, subtitle) = dmenu_page::entry_text(&page.entries[index], quick_look);
                super::a11y::item(title, subtitle)
            })
            .collect();
        let label = page.heading().unwrap_or_else(|| "Entries".to_owned());
        let detail = page
            .preview
            .as_ref()
            .map(|preview| super::a11y::file_preview_document(preview, !page.list.no_metadata));
        super::a11y::PageSemantics::list(&label, items, page.selected, page.empty_notice())
            .with_status(None, &page.query)
            .with_detail(detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{TestBackend, index, pressed, settle};
    use crate::resident::UiCommand;
    use crate::semantics::Body;
    use iced::window;
    use std::sync::Arc;

    #[test]
    fn the_entries_are_a_list_under_their_heading_with_the_file_preview_beside_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "first line\nsecond line\n").expect("write file");
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = DmenuPage::loading(1);
        assert_eq!(
            app.dmenu_semantics(&page).status.as_deref(),
            Some("Loading entries…")
        );
        page.apply(Ok(crate::backend::DmenuList {
            content: format!("{}\nplain entry\n", file.display()),
            ..crate::backend::DmenuList::default()
        }));
        app.page = Page::Dmenu(page);
        let semantics = app.semantics();
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Entries (2)");
        let names: Vec<&str> = list.items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["notes.txt", "plain entry"]);
        assert_eq!(list.selected, Some(0));
        let detail = semantics.detail.as_ref().expect("the preview is described");
        assert_eq!(detail.label, "notes.txt");
        assert!(!detail.metadata.is_empty(), "metadata is on by default");

        let Page::Dmenu(page) = &mut app.page else {
            unreachable!("the page was just set");
        };
        page.query = "%%%%".into();
        page.refilter();
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(
            semantics.status.as_deref(),
            Some("No entries match. Enter passes the search text.")
        );
    }

    fn dmenu_app(dir: &std::path::Path) -> (LauncherApp, Arc<TestBackend>) {
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir));
        app.backend = Some(backend.clone());
        // Already open, so the summon focuses it rather than opening one,
        // which only a running event loop could answer.
        app.window = Some(window::Id::unique());
        let task = app.update(Message::Command(UiCommand::Dmenu(5)));
        settle(&mut app, task);
        (app, backend)
    }

    #[test]
    fn a_pushed_dmenu_list_is_shown_filtered_and_answered_with_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = dmenu_app(dir.path());
        let Page::Dmenu(page) = &app.page else {
            panic!("no dmenu view: {}", app.state_line());
        };
        assert_eq!(page.entries, ["alpha", "beta", "gamma"]);
        assert_eq!(page.heading().as_deref(), Some("Pick (3)"));

        let _ = app.update(Message::DmenuQueryChanged("gama".into()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert_eq!(
            backend.dmenu_answers.lock().unwrap().as_slice(),
            [(5, Some("gamma".to_owned()))]
        );
        assert!(
            matches!(app.page, Page::Root),
            "choosing hides the launcher"
        );
    }

    #[test]
    fn escape_dismisses_a_dmenu_list_once() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = dmenu_app(dir.path());
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        let task = app.update(Message::Dismiss);
        settle(&mut app, task);
        assert_eq!(
            backend.dmenu_answers.lock().unwrap().as_slice(),
            [(5, None)],
            "one dismissal, however the view went away"
        );
    }

    #[test]
    fn a_dmenu_size_resizes_the_window_until_a_list_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) = dmenu_app(dir.path());
        assert_eq!(app.dmenu_card_size(), None);
        assert_eq!(app.resized_to, None);
        if let Page::Dmenu(page) = &mut app.page {
            page.list.width = Some(400);
            page.list.navigation_title = Some("Pick one".into());
        }
        let _ = app.apply_dmenu_size();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "hello").unwrap();
        if let Page::Dmenu(page) = &mut app.page {
            page.entries.push(file.to_string_lossy().into_owned());
            page.query = "notes".into();
            page.refilter();
            assert!(page.preview.is_some(), "quick look reads the file");
        }
        let _ = app.view();
        let size = Some((400, u32::from(GEOMETRY.card_max_height)));
        assert_eq!(app.dmenu_card_size(), size);
        assert_eq!(app.resized_to, size);
        let task = app.update(Message::Command(UiCommand::Dmenu(5)));
        settle(&mut app, task);
        assert_eq!(app.resized_to, None, "the next list puts the size back");
    }
}
