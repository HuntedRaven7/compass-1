//! Snippets in the launcher: Manage Snippets, and the forms that create,
//! edit and fill them in.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the decisions that do not need that state are in
//! [`crate::snippets_page`].

use compass_core::shortcut_form::Mode;
use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Space, Task, chord_direction, column, container, focus_search, mouse_area, next_selection, row,
    scrollable, text,
};
use crate::action_panel::Action;
use crate::preferences_page::Purpose;
use crate::snippets_page::{self, SnippetsPage};

const COPY: &str = "snippet.copy";
const PASTE: &str = "snippet.paste";
const EDIT: &str = "snippet.edit";
const DUPLICATE: &str = "snippet.duplicate";
const REMOVE: &str = "snippet.remove";

const NEEDS_ENGINE: &str =
    "Snippets need the Compass engine, and this window is running without one";

/// The actions a snippet row offers, as `ManageSnippetsSection` arranges
/// them, with Paste after Copy.
fn panel_sections() -> Vec<PanelSection> {
    vec![PanelSection {
        name: String::new(),
        actions: vec![
            Action::new("Copy to clipboard")
                .with_id(COPY)
                .with_shortcut("enter"),
            Action::new("Paste").with_id(PASTE),
            Action::new("Edit snippet")
                .with_id(EDIT)
                .with_shortcut("Ctrl+E"),
            Action::new("Duplicate snippet")
                .with_id(DUPLICATE)
                .with_shortcut("Ctrl+D"),
            Action::new("Remove snippet")
                .with_id(REMOVE)
                .with_shortcut("Ctrl+X"),
        ],
    }]
}

impl LauncherApp {
    /// Opens Manage Snippets, as it was left when a form was opened over it,
    /// and asks for the list.
    pub(super) fn open_manage_snippets(&mut self) -> Task<Message> {
        let page = self.parked_snippets.take().unwrap_or_default();
        self.page = Page::Snippets(page);
        let Some(backend) = self.backend.clone() else {
            if let Page::Snippets(page) = &mut self.page {
                page.apply(Err(NEEDS_ENGINE.to_owned()));
            }
            return focus_search();
        };
        Task::batch([
            Task::perform(
                async move { backend.list_snippets().await },
                Message::SnippetsLoaded,
            ),
            focus_search(),
        ])
    }

    /// Opens the snippet form: creating, or editing or duplicating the
    /// selected snippet of Manage Snippets.
    pub(super) fn open_snippet_form(&mut self, mode: Mode) -> Task<Message> {
        let (existing, from_manage) = match &self.page {
            Page::Snippets(page) if mode != Mode::Create => {
                let Some(snippet) = page.selected_snippet() else {
                    return Task::none();
                };
                (Some(snippet.clone()), true)
            }
            Page::Snippets(_) => (None, true),
            _ => (None, false),
        };
        self.parked_snippets = match &self.page {
            Page::Snippets(page) if from_manage => Some(page.clone()),
            _ => None,
        };
        self.panel = None;
        self.page = Page::Preferences(Box::new(snippets_page::form(
            mode,
            existing.as_ref(),
            from_manage,
        )));
        iced::widget::operation::focus_next()
    }

    /// Copies or pastes the selected snippet: through its arguments form
    /// when its text takes arguments, at once otherwise.
    fn use_selected_snippet(&mut self, paste: bool) -> Task<Message> {
        let Page::Snippets(page) = &self.page else {
            return Task::none();
        };
        let Some(snippet) = page.selected_snippet().cloned() else {
            return Task::none();
        };
        self.panel = None;
        if let Some(form) = snippets_page::arguments_form(&snippet, paste) {
            self.parked_snippets = match &self.page {
                Page::Snippets(page) => Some(page.clone()),
                _ => None,
            };
            self.page = Page::Preferences(Box::new(form));
            return iced::widget::operation::focus_next();
        }
        self.send_snippet(snippet.id, Vec::new(), paste)
    }

    /// Asks the engine to expand a snippet for copying, or to paste it.
    fn send_snippet(
        &mut self,
        id: String,
        arguments: Vec<(String, String)>,
        paste: bool,
    ) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            self.snippet_notice(NEEDS_ENGINE.to_owned());
            return Task::none();
        };
        if paste {
            Task::perform(
                async move { backend.paste_snippet(id, arguments).await },
                Message::SnippetPasted,
            )
        } else {
            Task::perform(
                async move { backend.expand_snippet(id, arguments).await },
                Message::SnippetExpanded,
            )
        }
    }

    /// Removes the selected snippet.
    fn remove_selected_snippet(&mut self) -> Task<Message> {
        let Page::Snippets(page) = &self.page else {
            return Task::none();
        };
        let (Some(snippet), Some(backend)) = (page.selected_snippet(), self.backend.clone()) else {
            return Task::none();
        };
        let id = snippet.id.clone();
        self.panel = None;
        Task::perform(
            async move { backend.remove_snippet(id).await },
            Message::SnippetsLoaded,
        )
    }

    /// Shows why a snippet action did not happen, where the person is.
    fn snippet_notice(&mut self, reason: String) {
        match &mut self.page {
            Page::Snippets(page) => page.notice = Some(reason),
            Page::Preferences(page) => page.notice = Some(reason),
            _ => self.error = Some(reason),
        }
    }

    /// Opens the action panel over the selected snippet, if one is.
    pub(super) fn open_snippet_panel(&mut self) -> Option<Task<Message>> {
        let Page::Snippets(page) = &self.page else {
            return None;
        };
        page.selected_snippet()?;
        self.panel = Some(PanelState::new(panel_sections()));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a snippet action from the panel, if `id` is one.
    pub(super) fn snippet_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Snippets(_)) {
            return None;
        }
        let task = match id {
            COPY => self.use_selected_snippet(false),
            PASTE => self.use_selected_snippet(true),
            EDIT => self.open_snippet_form(Mode::Edit),
            DUPLICATE => self.open_snippet_form(Mode::Duplicate),
            REMOVE => self.remove_selected_snippet(),
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// Manage Snippets' keys: the list's navigation, Enter to copy, Ctrl+E
    /// edit, Ctrl+D duplicate, Ctrl+X remove, Escape to go back.
    pub(super) fn snippets_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        if let Key::Character(c) = key.as_ref()
            && modifiers.control()
            && !modifiers.alt()
            && !modifiers.logo()
            && !modifiers.shift()
        {
            match c.to_lowercase().as_str() {
                "e" => return self.open_snippet_form(Mode::Edit),
                "d" => return self.open_snippet_form(Mode::Duplicate),
                "x" => return self.remove_selected_snippet(),
                _ => {}
            }
        }
        let Page::Snippets(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.use_selected_snippet(false),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.shown.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            return Task::batch([
                crate::scroll::reveal_root_selection(),
                self.snippet_detail_task(),
            ]);
        }
        Task::none()
    }

    /// Asks for the detail pane of the selected snippet, unless it is
    /// already showing: a text snippet expanded without running its shell
    /// placeholders (`loadDetail`, `updateExpandedText`).
    pub(super) fn snippet_detail_task(&mut self) -> Task<Message> {
        let Page::Snippets(page) = &mut self.page else {
            return Task::none();
        };
        let Some(snippet) = page.selected_snippet() else {
            page.detail = None;
            return Task::none();
        };
        if page.detail.as_ref().is_some_and(|d| d.id == snippet.id) {
            return Task::none();
        }
        let id = snippet.id.clone();
        if snippet.text().is_none() {
            page.detail = Some(snippets_page::Detail {
                id,
                expanded: Ok(String::new()),
            });
            return Task::none();
        }
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                let expanded = backend.preview_snippet(id.clone(), Vec::new()).await;
                snippets_page::Detail { id, expanded }
            },
            Message::SnippetDetailLoaded,
        )
    }

    /// The detail pane beside Manage Snippets' list: the expanded text, then
    /// the metadata `loadDetail` lists.
    fn snippet_detail_pane<'a>(
        &'a self,
        snippet: &'a crate::backend::Snippet,
        detail: &'a snippets_page::Detail,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let muted = |value: String| {
            text(value)
                .font(self.font())
                .size(12)
                .color(palette.muted.to_iced())
        };
        let content: Element<'a, Message> = match &detail.expanded {
            Ok(expanded) => text(expanded.as_str())
                .font(self.font())
                .size(13)
                .color(palette.text.to_iced())
                .into(),
            Err(reason) => muted(reason.clone()).into(),
        };
        let mut fields = column![].spacing(4);
        for (label, value) in self.snippet_detail_fields(snippet) {
            fields = fields.push(
                row![
                    muted(label.to_owned()),
                    Space::new().width(Length::Fill),
                    text(value)
                        .font(self.font())
                        .size(12)
                        .color(palette.text.to_iced()),
                ]
                .spacing(12),
            );
        }
        let pane = column![
            scrollable(container(content).padding(8)).height(Length::Fill),
            container(fields)
                .padding(Padding::new(8.0))
                .style(move |_: &iced::Theme| container::Style {
                    border: iced::Border {
                        color: palette.border.to_iced(),
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                }),
        ]
        .spacing(6);
        container(pane)
            .width(Length::FillPortion(super::preview::PANE_PORTION))
            .height(Length::Fixed(320.0))
            .padding(Padding::new(6.0).top(8))
            .into()
    }

    /// The detail pane's metadata, the applications by name and the dates
    /// written as the C++ writes them.
    fn snippet_detail_fields(
        &self,
        snippet: &crate::backend::Snippet,
    ) -> Vec<(&'static str, String)> {
        let date = |at: u64| {
            crate::file_preview::qt_text_date(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(at),
            )
            .unwrap_or_default()
        };
        let app_name = |id: &str| {
            self.app_index
                .applications()
                .find(|item| {
                    item.desktop_id() == id
                        || item.desktop_id().strip_suffix(".desktop") == Some(id)
                })
                .map(|item| item.display_name())
        };
        snippets_page::detail_fields(snippet, app_name, date)
    }

    /// Submits a snippet form: its arguments, to copy or paste it, or its
    /// fields, to save it. `None` when the form showing is not a snippet form.
    pub(super) fn submit_snippet_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(page) = &mut self.page else {
            return None;
        };
        match page.purpose {
            Purpose::SnippetArguments { paste } => {
                if let Err(missing) = page.submission() {
                    page.notice = Some(format!("Fill in {}", missing.join(", ")));
                    return Some(Task::none());
                }
                let id = page.command_id.clone();
                let arguments = snippets_page::argument_values(page);
                Some(self.send_snippet(id, arguments, paste))
            }
            Purpose::SnippetForm { mode, .. } => {
                let (name, text, keyword, word) = snippets_page::form_values(page);
                // The keyword's applications are not edited here; they stay
                // what the snippet edited or duplicated had.
                let apps = self
                    .parked_snippets
                    .as_ref()
                    .and_then(|parked| parked.all.iter().find(|s| s.id == page.command_id))
                    .and_then(|snippet| snippet.expansion.as_ref())
                    .map(|expansion| expansion.apps.clone())
                    .unwrap_or_default();
                let draft = crate::backend::SnippetDraft {
                    id: (mode == Mode::Edit).then(|| page.command_id.clone()),
                    name,
                    text,
                    keyword: (!keyword.is_empty()).then_some(keyword),
                    word,
                    apps,
                };
                let Some(backend) = self.backend.clone() else {
                    page.notice = Some(NEEDS_ENGINE.to_owned());
                    return Some(Task::none());
                };
                Some(Task::perform(
                    async move { backend.save_snippet(draft).await },
                    Message::SnippetSaved,
                ))
            }
            _ => None,
        }
    }

    /// Where Escape goes from a snippet form: back to Manage Snippets when it
    /// was opened from there.
    pub(super) fn back_from_snippet_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(page) = &self.page else {
            return None;
        };
        match page.purpose {
            Purpose::SnippetForm {
                from_manage: true, ..
            }
            | Purpose::SnippetArguments { .. } => Some(self.open_manage_snippets()),
            _ => None,
        }
    }

    /// Handles the snippet messages.
    pub(super) fn snippet_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SnippetsLoaded(result) => {
                if let Page::Snippets(page) = &mut self.page {
                    page.apply(result);
                } else if let Err(reason) = result {
                    self.snippet_notice(reason);
                }
                Task::batch([
                    crate::scroll::reveal_root_selection(),
                    self.snippet_detail_task(),
                ])
            }
            Message::SnippetDetailLoaded(detail) => {
                if let Page::Snippets(page) = &mut self.page
                    && page
                        .selected_snippet()
                        .is_some_and(|snippet| snippet.id == detail.id)
                {
                    page.detail = Some(detail);
                }
                Task::none()
            }
            Message::SnippetSaved(Ok(snippets)) => {
                let from_manage = matches!(
                    &self.page,
                    Page::Preferences(page)
                        if matches!(page.purpose, Purpose::SnippetForm { from_manage: true, .. })
                );
                if from_manage {
                    let task = self.open_manage_snippets();
                    if let Page::Snippets(page) = &mut self.page {
                        page.apply(Ok(snippets));
                    }
                    return task;
                }
                self.parked_snippets = None;
                self.page = Page::Root;
                Task::batch([self.search_task(), focus_search()])
            }
            Message::SnippetSaved(Err(reason))
            | Message::SnippetExpanded(Err(reason))
            | Message::SnippetPasted(Err(reason)) => {
                self.snippet_notice(reason);
                Task::none()
            }
            Message::SnippetExpanded(Ok(text)) => {
                self.parked_snippets = None;
                if matches!(self.page, Page::Preferences(_)) {
                    self.page = Page::Root;
                }
                let hud = crate::hud::Hud::new("Copied to clipboard");
                Task::batch([
                    super::hud::copy_text(self.backend.clone(), text),
                    self.show_hud(hud),
                ])
            }
            Message::SnippetPasted(Ok(())) => {
                self.parked_snippets = None;
                if matches!(self.page, Page::Preferences(_)) {
                    self.page = Page::Root;
                }
                self.conceal()
            }
            Message::SnippetsQueryChanged(query) => {
                if let Page::Snippets(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.selected = 0;
                    page.refilter();
                }
                Task::batch([
                    crate::scroll::reveal_root_selection(),
                    self.snippet_detail_task(),
                ])
            }
            Message::SnippetSelected(position) => {
                let Page::Snippets(page) = &mut self.page else {
                    return Task::none();
                };
                if position >= page.shown.len() {
                    return Task::none();
                }
                page.selected = position;
                self.use_selected_snippet(false)
            }
            _ => Task::none(),
        }
    }

    /// Manage Snippets' body.
    pub(super) fn snippets_body<'a>(&'a self, page: &'a SnippetsPage) -> Element<'a, Message> {
        if let Some(empty) = page.empty_notice() {
            return self.notice(empty);
        }
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(snippet) = page.all.get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let row = self.list_row(
                self.initial_badge(&snippet.name, selected),
                snippet.name.clone(),
                self.subtitles.then(|| snippets_page::subtitle(snippet)),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::SnippetSelected(position))
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
        let rows: Element<Message> = match snippet_detail_shown(page) {
            Some((snippet, detail)) => row![
                container(rows).width(Length::FillPortion(super::preview::LIST_PORTION)),
                self.snippet_detail_pane(snippet, detail),
            ]
            .into(),
            None => rows.into(),
        };
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows,
        }
    }

    /// Manage Snippets' rows and the selected one's detail, for assistive
    /// technology.
    pub(super) fn snippets_semantics(&self, page: &SnippetsPage) -> super::a11y::PageSemantics {
        let items = page
            .shown
            .iter()
            .map(|&index| {
                page.all
                    .get(index)
                    .map_or_else(crate::semantics::Item::default, |snippet| {
                        super::a11y::item(
                            snippet.name.clone(),
                            Some(snippets_page::subtitle(snippet)),
                        )
                    })
            })
            .collect();
        let detail = snippet_detail_shown(page).map(|(snippet, detail)| {
            let text = match &detail.expanded {
                Ok(expanded) => expanded,
                Err(reason) => reason,
            };
            crate::semantics::Document {
                label: snippet.name.clone(),
                blocks: crate::semantics::text_blocks(text),
                metadata: super::a11y::metadata(self.snippet_detail_fields(snippet)),
            }
        });
        super::a11y::PageSemantics::list("Snippets", items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
            .with_detail(detail)
    }
}

/// The selected snippet and its detail, when the detail that arrived is
/// still the selected one's and so is drawn beside the list.
fn snippet_detail_shown(
    page: &SnippetsPage,
) -> Option<(&crate::backend::Snippet, &snippets_page::Detail)> {
    let snippet = page.selected_snippet()?;
    let detail = page
        .detail
        .as_ref()
        .filter(|detail| detail.id == snippet.id)?;
    Some((snippet, detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Snippet;
    use crate::semantics::Body;
    use compass_core::snippet_store::{SnippetData, StoredExpansion};

    fn snippet(id: &str, name: &str, text: &str, keyword: Option<&str>) -> Snippet {
        Snippet {
            id: id.into(),
            name: name.into(),
            data: SnippetData::Text { text: text.into() },
            expansion: keyword.map(|keyword| StoredExpansion {
                keyword: keyword.into(),
                apps: Vec::new(),
                word: false,
            }),
            ..Snippet::default()
        }
    }

    #[test]
    fn manage_snippets_describes_its_rows_and_the_selected_detail() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = SnippetsPage::default();
        assert_eq!(
            app.snippets_semantics(&page).status.as_deref(),
            Some("Loading snippets…")
        );
        page.apply(Ok(vec![
            snippet("a", "Signature", "Best regards", Some(";sig")),
            snippet("b", "Address", "1 Main Street\nSpringfield", None),
        ]));
        page.selected = 1;
        page.detail = Some(snippets_page::Detail {
            id: "b".into(),
            expanded: Ok("1 Main Street\nSpringfield".into()),
        });
        let semantics = app.snippets_semantics(&page);
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.items[0].name, "Signature");
        assert_eq!(
            list.items[0].description,
            Some(snippets_page::subtitle(&page.all[0]))
        );
        assert_eq!(list.selected, Some(1));
        let detail = semantics.detail.expect("the detail pane");
        assert_eq!(detail.label, "Address");
        let blocks: Vec<&str> = detail.blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(blocks, ["1 Main Street", "Springfield"]);
        assert_eq!(detail.metadata[0].label, "Type");
        assert_eq!(detail.metadata[0].value, "Text");

        page.selected = 0;
        assert_eq!(app.snippets_semantics(&page).detail, None);

        page.apply(Ok(Vec::new()));
        app.page = Page::Snippets(page);
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(
            semantics.status.as_deref(),
            Some("No snippets. Create one with Create Snippet.")
        );
    }
}
