//! Clipboard history in the launcher: the list and its search, copy, paste,
//! pin and remove, the kind filter, the detail pane, keyword editing,
//! remove-all and the monitoring switch (`ClipboardHistoryViewHost`,
//! `ClipboardHistorySection::actionPanel`).

use iced::keyboard::{Key, Modifiers};

use super::icons::clipboard_url;
use super::{
    Alignment, Confirm, ConfirmAction, Element, LauncherApp, Length, Message, Padding, Page,
    PanelSection, PanelState, Space, Task, column, container, focus_search, hud, image, mouse_area,
    preview, row, scrollable, text,
};
use crate::action_panel::Action;
use crate::backend::{PreferenceInput, PreferenceInputKind};
use crate::clipboard_page::{self, ClipboardPage, DetailContent};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};

const PASTE: &str = "clipboard.paste";
const COPY: &str = "clipboard.copy";
const PIN: &str = "clipboard.pin";
const EDIT_KEYWORDS: &str = "clipboard.edit-keywords";
const REMOVE: &str = "clipboard.remove";
const REMOVE_ALL: &str = "clipboard.remove-all";
const MONITORING: &str = "clipboard.monitoring";
const OPEN: &str = "clipboard.open";
const OPEN_WITH: &str = "clipboard.open-with";

/// `EditClipboardKeywordsAction`'s description, under the field.
const KEYWORDS_DESCRIPTION: &str = "Additional keywords that will be used to index this selection.";

/// A change to one clipboard history entry, from its keyboard shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClipboardChange {
    TogglePin,
    Remove,
}

impl LauncherApp {
    /// Opens Clipboard History with the filter it was left on, and asks
    /// whether copies are being recorded.
    pub(super) fn open_clipboard_history(&mut self) -> Task<Message> {
        let kind = clipboard_page::kind_for_stored(
            self.view.view_memory.get(clipboard_page::FILTER_MEMORY_KEY),
        );
        self.page = Page::Clipboard(ClipboardPage {
            kind,
            ..ClipboardPage::default()
        });
        let monitoring = self.clipboard.clone().map(|clipboard| {
            Task::perform(
                async move { clipboard.clipboard_monitoring(None).await },
                Message::ClipboardMonitoringLoaded,
            )
        });
        Task::batch([
            self.clipboard_search_task(),
            monitoring.unwrap_or_else(Task::none),
            focus_search(),
        ])
    }

    /// Fetches the detail pane for the selected entry when it is not already
    /// showing it.
    pub(super) fn clipboard_detail_task(&mut self) -> Task<Message> {
        let Page::Clipboard(page) = &mut self.page else {
            return Task::none();
        };
        let Some(id) = page.detail_wanted() else {
            return Task::none();
        };
        let Some(clipboard) = self.clipboard.clone() else {
            return Task::none();
        };
        let info = {
            let (clipboard, id) = (clipboard.clone(), id.clone());
            Task::perform(
                {
                    let id = id.clone();
                    async move { clipboard.clipboard_detail(id).await }
                },
                move |result| Message::ClipboardDetailLoaded {
                    id: id.clone(),
                    result,
                },
            )
        };
        let content = Task::perform(
            {
                let id = id.clone();
                async move { clipboard.clipboard_content(id).await }
            },
            move |result| Message::ClipboardDetailContent {
                id: id.clone(),
                result,
            },
        );
        Task::batch([info, content])
    }

    /// Handles the messages of this module.
    pub(super) fn clipboard_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ClipboardKindChanged(label) => {
                let kind = clipboard_page::kind_for_label(&label);
                if let Page::Clipboard(page) = &mut self.page {
                    page.kind = kind;
                    // `setKindFilter` clears the search text as it changes
                    // the filter.
                    page.query.clear();
                }
                self.view.view_memory.set(
                    clipboard_page::FILTER_MEMORY_KEY,
                    clipboard_page::filter_for_kind(kind).1,
                );
                Task::batch([self.clipboard_search_task(), focus_search()])
            }
            Message::ClipboardDetailLoaded { id, result } => {
                if let Page::Clipboard(page) = &mut self.page {
                    page.apply_detail(&id, Some(result), None);
                }
                Task::none()
            }
            Message::ClipboardDetailContent { id, result } => {
                if let Page::Clipboard(page) = &mut self.page {
                    page.apply_detail(&id, None, Some(result));
                }
                Task::none()
            }
            Message::ClipboardKeywordsLoaded(result) => match result {
                Ok(detail) => self.open_clipboard_keywords(&detail.id, &detail.keywords),
                Err(reason) => {
                    if let Page::Clipboard(page) = &mut self.page {
                        page.notice = Some(reason);
                    }
                    Task::none()
                }
            },
            Message::ClipboardMonitoringLoaded(result) => {
                if let Page::Clipboard(page) = &mut self.page {
                    match result {
                        Ok(monitoring) => page.monitoring = Some(monitoring),
                        Err(reason) => tracing::debug!(%reason, "monitoring state unknown"),
                    }
                }
                Task::none()
            }
            Message::ClipboardQueryChanged(query) => {
                if let Page::Clipboard(page) = &mut self.page {
                    page.query = query;
                }
                self.clipboard_search_task()
            }
            Message::ClipboardLoaded { generation, result } => {
                if let Page::Clipboard(page) = &mut self.page {
                    page.apply(generation, result);
                }
                self.warm_clipboard_icons();
                Task::batch([
                    crate::scroll::reveal_root_selection(),
                    self.clipboard_detail_task(),
                ])
            }
            Message::ClipboardSelected(index) => {
                if let Page::Clipboard(page) = &mut self.page
                    && index < page.rows.len()
                {
                    page.selected = index;
                }
                self.paste_selected_clipboard_entry()
            }
            Message::ClipboardPasted(Ok(())) => self.conceal(),
            Message::ClipboardEntryChanged(Ok(())) => self.clipboard_search_task(),
            Message::ClipboardEntryChanged(Err(reason)) => {
                if let Page::Clipboard(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            Message::ClipboardPasted(Err(reason)) => {
                tracing::debug!(%reason, "paste refused; copying instead");
                self.copy_selected_clipboard_entry()
            }
            Message::ClipboardContentLoaded(result) => {
                let Page::Clipboard(page) = &mut self.page else {
                    return Task::none();
                };
                match result.and_then(|content| crate::clipboard_page::copyable_text(&content)) {
                    Ok(text) => {
                        // Copy, then get out of the way: the user copied it
                        // to paste it somewhere else.
                        let copy = hud::copy_text(self.backend.clone(), text);
                        let hud = crate::hud::Hud::new("Selection copied to clipboard");
                        Task::batch([copy, self.show_hud(hud)])
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

    /// The clipboard view's own chords, besides pin and remove: the keyword
    /// form (`action.edit`) and remove-all (`action.dangerous-remove`).
    pub(super) fn clipboard_chord(
        &mut self,
        key: &Key,
        modifiers: Modifiers,
    ) -> Option<Task<Message>> {
        let Key::Character(c) = key.as_ref() else {
            return None;
        };
        if !modifiers.control() || modifiers.alt() || modifiers.logo() {
            return None;
        }
        if !modifiers.shift() && c.eq_ignore_ascii_case("e") {
            return Some(self.edit_clipboard_keywords());
        }
        if modifiers.shift() && c.eq_ignore_ascii_case("x") {
            return Some(self.confirm_clipboard_remove_all());
        }
        None
    }

    /// The panel over the selected entry.
    pub(super) fn open_clipboard_panel(&mut self) -> Option<Task<Message>> {
        let Page::Clipboard(page) = &self.page else {
            return None;
        };
        let entry = page.selected_row()?;
        let mut main = vec![
            Action::new("Paste to active window")
                .with_id(PASTE)
                .with_shortcut("enter"),
            Action::new("Copy to clipboard")
                .with_id(COPY)
                .with_shortcut("ctrl+shift+c"),
        ];
        if self.clipboard_open_target().is_some() {
            main.push(Action::new("Open").with_id(OPEN).with_shortcut("ctrl+o"));
            main.push(
                Action::new("Open with...")
                    .with_id(OPEN_WITH)
                    .with_shortcut("ctrl+shift+o"),
            );
        }
        let mut sections = vec![
            PanelSection {
                name: String::new(),
                actions: main,
            },
            PanelSection {
                name: String::new(),
                actions: vec![
                    Action::new(if entry.pinned { "Unpin" } else { "Pin" })
                        .with_id(PIN)
                        .with_shortcut("ctrl+shift+p"),
                    Action::new("Edit keywords")
                        .with_id(EDIT_KEYWORDS)
                        .with_shortcut("ctrl+e"),
                ],
            },
            PanelSection {
                name: String::new(),
                actions: vec![
                    Action::new("Remove entry")
                        .with_id(REMOVE)
                        .with_shortcut("ctrl+x"),
                    Action::new("Remove all")
                        .with_id(REMOVE_ALL)
                        .with_shortcut("ctrl+shift+x"),
                ],
            },
        ];
        if let Some(title) = page.monitoring.and_then(clipboard_page::monitoring_action) {
            sections.push(PanelSection {
                name: String::new(),
                actions: vec![Action::new(title).with_id(MONITORING)],
            });
        }
        self.panel = Some(PanelState::new(sections));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a clipboard panel action, if `id` is one.
    pub(super) fn clipboard_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Clipboard(_)) {
            return None;
        }
        if id == OPEN || id == OPEN_WITH {
            let target = self.clipboard_open_target()?;
            self.panel = None;
            if id == OPEN_WITH {
                let target = target.as_str().to_owned();
                return Some(self.open_with(target.clone(), target));
            }
            let backend = self.backend.clone()?;
            return Some(Task::perform(
                async move {
                    match target {
                        clipboard_page::OpenTarget::Url(url) => backend.open_url(url).await,
                        clipboard_page::OpenTarget::File(path) => {
                            backend.open_file(path, false).await
                        }
                    }
                },
                Message::FileActionDone,
            ));
        }
        let task = match id {
            PASTE => self.paste_selected_clipboard_entry(),
            COPY => self.copy_selected_clipboard_entry(),
            PIN => self.change_selected_clipboard_entry(ClipboardChange::TogglePin),
            EDIT_KEYWORDS => self.edit_clipboard_keywords(),
            REMOVE => self.change_selected_clipboard_entry(ClipboardChange::Remove),
            REMOVE_ALL => self.confirm_clipboard_remove_all(),
            MONITORING => self.toggle_clipboard_monitoring(),
            _ => return None,
        };
        self.panel = None;
        Some(Task::batch([task, focus_search()]))
    }

    /// What the selected entry opens as, once its content is in the pane: a
    /// link, or a single copied file that still exists.
    fn clipboard_open_target(&self) -> Option<clipboard_page::OpenTarget> {
        let Page::Clipboard(page) = &self.page else {
            return None;
        };
        let entry = page.selected_row()?;
        let detail = page
            .detail
            .as_ref()
            .filter(|detail| detail.id == entry.id)?;
        let Some(Ok(content)) = &detail.content else {
            return None;
        };
        clipboard_page::open_target(entry.kind, content)
    }

    /// Asks before removing everything, as `RemoveAllSelectionsAction` does.
    fn confirm_clipboard_remove_all(&mut self) -> Task<Message> {
        self.panel = None;
        self.confirm = Some(Confirm {
            title: "Are you sure?".to_owned(),
            message: "All your clipboard history will be lost forever".to_owned(),
            confirm_text: "Delete all".to_owned(),
            action: ConfirmAction::ClipboardRemoveAll,
        });
        Task::none()
    }

    /// Removes every entry, once confirmed.
    pub(super) fn remove_all_clipboard_entries(&mut self) -> Task<Message> {
        let Some(clipboard) = self.clipboard.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { clipboard.clipboard_remove_all().await },
            Message::ClipboardEntryChanged,
        )
    }

    /// Pauses or resumes recording, as the status button does.
    fn toggle_clipboard_monitoring(&mut self) -> Task<Message> {
        let Page::Clipboard(page) = &self.page else {
            return Task::none();
        };
        let (Some(monitoring), Some(clipboard)) = (page.monitoring, self.clipboard.clone()) else {
            return Task::none();
        };
        let enabled = !monitoring.enabled;
        Task::perform(
            async move { clipboard.clipboard_monitoring(Some(enabled)).await },
            Message::ClipboardMonitoringLoaded,
        )
    }

    /// Fetches the selected entry's keywords, then opens the form.
    fn edit_clipboard_keywords(&mut self) -> Task<Message> {
        let Page::Clipboard(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(clipboard)) = (page.selected_row(), self.clipboard.clone()) else {
            return Task::none();
        };
        self.panel = None;
        let id = row.id.clone();
        Task::perform(
            async move { clipboard.clipboard_detail(id).await },
            Message::ClipboardKeywordsLoaded,
        )
    }

    /// The keyword form over entry `id`, keeping the history to come back to.
    fn open_clipboard_keywords(&mut self, id: &str, keywords: &str) -> Task<Message> {
        if !matches!(self.page, Page::Clipboard(_)) {
            return Task::none();
        }
        let field = PreferenceInput {
            name: "keywords".to_owned(),
            title: "Keywords".to_owned(),
            description: KEYWORDS_DESCRIPTION.to_owned(),
            placeholder: String::new(),
            required: false,
            kind: PreferenceInputKind::Text,
            value: Some(serde_json::Value::String(keywords.to_owned())),
        };
        let form = PreferencesPage::new(
            Purpose::ClipboardKeywords,
            id.to_owned(),
            "Edit keywords".to_owned(),
            vec![field],
        );
        if let Page::Clipboard(page) = std::mem::replace(&mut self.page, Page::Root) {
            self.parked_clipboard = Some(page);
        }
        self.page = Page::Preferences(Box::new(form));
        Task::none()
    }

    /// Saves the keyword form and goes back to the history, which reloads.
    /// `None` when the form showing is not the clipboard's keyword form.
    pub(super) fn submit_clipboard_keywords(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::ClipboardKeywords {
            return None;
        }
        let id = form.command_id.clone();
        let keywords = match form.values.first() {
            Some(FieldValue::Text(text)) => text.clone(),
            _ => String::new(),
        };
        let mut page = self.parked_clipboard.take().unwrap_or_default();
        // The pane shows keywords, so it is fetched again.
        page.detail = None;
        self.page = Page::Clipboard(page);
        let Some(clipboard) = self.clipboard.clone() else {
            return Some(focus_search());
        };
        Some(Task::batch([
            Task::perform(
                async move { clipboard.clipboard_set_keywords(id, keywords).await },
                Message::ClipboardEntryChanged,
            ),
            focus_search(),
        ]))
    }

    /// Escape from the keyword form: back to the history, as it was.
    pub(super) fn back_from_clipboard_keywords(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::ClipboardKeywords {
            return None;
        }
        self.page = Page::Clipboard(self.parked_clipboard.take().unwrap_or_default());
        Some(focus_search())
    }

    /// The kind filter, right-aligned above the list: the card's own dropdown
    /// look (#239), in the launcher's font, rather than Iced's default
    /// pick-list chrome.
    pub(super) fn clipboard_filter<'a>(&self, page: &ClipboardPage) -> Element<'a, Message> {
        let palette = self.palette();
        container(
            iced::widget::pick_list(
                clipboard_page::KIND_FILTERS
                    .iter()
                    .map(|(label, _, _)| (*label).to_owned())
                    .collect::<Vec<_>>(),
                Some(clipboard_page::filter_for_kind(page.kind).0.to_owned()),
                Message::ClipboardKindChanged,
            )
            .text_size(12)
            .font(self.font())
            .style(move |_, status| crate::design::dropdown(palette, status))
            .menu_style(move |_| crate::design::dropdown_menu(palette)),
        )
        .width(Length::Fill)
        .align_x(Alignment::End)
        .padding(Padding::new(4.0).right(10))
        .into()
    }

    /// The detail pane beside the list: the content, then its metadata.
    pub(super) fn clipboard_detail_pane<'a>(
        &'a self,
        detail: &'a clipboard_page::Detail,
        query: &'a str,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let muted = |value: String| {
            text(value)
                .font(self.font())
                .size(12)
                .color(palette.muted.to_iced())
        };
        let drawn: Element<'a, Message> = match (&detail.pane, &detail.image) {
            (Some(DetailContent::File(preview)), _) => {
                return self.file_preview_pane(preview, true);
            }
            (Some(DetailContent::Text(body)), _) => scrollable(
                container({
                    // The searched words behind the accent at 35%, as
                    // `TextViewer`'s `highlightColor`.
                    let mark = iced::Color {
                        a: 0.35,
                        ..palette.accent.to_iced()
                    };
                    let spans: Vec<iced::widget::text::Span<'a, ()>> =
                        clipboard_page::highlighted(body, query)
                            .into_iter()
                            .map(|(piece, hit)| {
                                let span = iced::widget::span(piece);
                                if hit { span.background(mark) } else { span }
                            })
                            .collect();
                    iced::widget::rich_text(spans)
                        .font(iced::Font::MONOSPACE)
                        .size(12)
                        .color(palette.text.to_iced())
                })
                .padding(8),
            )
            .height(Length::Fill)
            .into(),
            (Some(DetailContent::Image(_)), Some(handle)) => container(
                image(handle.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Contain),
            )
            .height(Length::Fill)
            .padding(8)
            .into(),
            (Some(DetailContent::Error(title, reason)), _) => column![
                text(title.as_str()).font(self.font()).size(13),
                muted(reason.clone()),
            ]
            .spacing(6)
            .padding(8)
            .height(Length::Fill)
            .into(),
            _ => Space::new().height(Length::Fill).into(),
        };
        let mut pane = column![drawn].spacing(6);
        if let Some(Ok(info)) = &detail.info {
            let mut fields = column![].spacing(4);
            for (label, value) in clipboard_page::detail_fields(info) {
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
            pane = pane.push(container(fields).padding(Padding::new(8.0)).style(
                move |_: &iced::Theme| container::Style {
                    border: iced::Border {
                        color: palette.border.to_iced(),
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                },
            ));
        }
        container(pane)
            .width(Length::FillPortion(super::preview::PANE_PORTION))
            .height(Length::Fixed(320.0))
            .padding(Padding::new(6.0).top(8))
            .into()
    }

    /// Clipboard history's rows and the selected entry's detail, for
    /// assistive technology.
    pub(super) fn clipboard_semantics(&self, page: &ClipboardPage) -> super::a11y::PageSemantics {
        let items = page
            .rows
            .iter()
            .map(|entry| {
                super::a11y::item(entry.preview.clone(), Some(clipboard_page::subtitle(entry)))
            })
            .collect();
        // The line about monitoring sits above the list for as long as copies
        // are not recorded, so it is the status whenever nothing else is.
        let notice = page
            .notice
            .as_deref()
            .or_else(|| page.monitoring.and_then(clipboard_page::monitoring_notice));
        super::a11y::PageSemantics::list(
            "Clipboard history",
            items,
            page.selected,
            page.empty_notice(),
        )
        .with_status(notice, &page.query)
        .with_detail(
            page.detail
                .as_ref()
                .map(|detail| clipboard_detail_document(page, detail)),
        )
    }

    /// Clipboard history's body: its state, or its rows.
    pub(super) fn clipboard_body<'a>(
        &'a self,
        page: &'a crate::clipboard_page::ClipboardPage,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        let filter = self.clipboard_filter(page);
        let status = page
            .monitoring
            .and_then(crate::clipboard_page::monitoring_notice)
            .map(|line| self.section_heading(line.to_owned()));
        if let Some(empty) = page.empty_notice() {
            return column![filter].push(status).push(self.notice(empty)).into();
        }
        let mut list = column![].spacing(f32::from(geometry.row_spacing));
        for (position, entry) in page.rows.iter().enumerate() {
            let selected = position == page.selected;
            let subtitle = self
                .subtitles
                .then(|| crate::clipboard_page::subtitle(entry));
            let favicon =
                clipboard_url(entry).and_then(|url| self.url_glyphs.get(&url.to_url())?.clone());
            let row = self.list_row(
                self.glyph_or_initial(
                    self.icons
                        .then(|| {
                            favicon.unwrap_or_else(|| crate::icons::clipboard_glyph(entry.kind))
                        })
                        .as_ref(),
                    crate::clipboard_page::subtitle(entry).as_str(),
                    selected,
                ),
                entry.preview.clone(),
                subtitle,
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::ClipboardSelected(position))
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
        let rows: Element<Message> = match &page.detail {
            Some(detail) => row![
                container(rows).width(Length::FillPortion(preview::LIST_PORTION)),
                self.clipboard_detail_pane(detail, &page.query),
            ]
            .into(),
            None => rows.into(),
        };
        let body = column![filter].push(status).push(rows);
        match &page.notice {
            Some(notice) => body.push(self.notice(notice)).into(),
            None => body.into(),
        }
    }

    /// Asks for clipboard history matching the view's filter.
    fn clipboard_search_task(&mut self) -> Task<Message> {
        let Page::Clipboard(page) = &mut self.page else {
            return Task::none();
        };
        page.generation = page.generation.wrapping_add(1);
        page.notice = None;
        let generation = page.generation;
        let Some(clipboard) = self.clipboard.clone() else {
            page.status = crate::clipboard_page::Status::Failed(
                "Clipboard history needs the Compass engine, and this window is running \
                 without one"
                    .to_owned(),
            );
            return Task::none();
        };
        let (query, kind) = (page.query.clone(), page.kind);
        Task::perform(
            async move {
                clipboard
                    .clipboard_history_of_kind(query, crate::clipboard_page::PAGE_SIZE, kind)
                    .await
            },
            move |result| Message::ClipboardLoaded { generation, result },
        )
    }

    /// Fetches the selected entry's content, to copy it.
    pub(super) fn change_selected_clipboard_entry(
        &mut self,
        change: ClipboardChange,
    ) -> Task<Message> {
        let Page::Clipboard(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(clipboard)) = (page.selected_row(), self.clipboard.clone()) else {
            return Task::none();
        };
        let (id, pinned) = (row.id.clone(), row.pinned);
        Task::perform(
            async move {
                match change {
                    ClipboardChange::TogglePin => clipboard.clipboard_set_pinned(id, !pinned).await,
                    ClipboardChange::Remove => clipboard.clipboard_remove(id).await,
                }
            },
            Message::ClipboardEntryChanged,
        )
    }

    /// Paste where the engine can, and copy where it cannot: the paste needs
    /// the GNOME Shell extension, and without it the entry still goes on the
    /// clipboard for the user's own Ctrl+V.
    pub(super) fn paste_selected_clipboard_entry(&mut self) -> Task<Message> {
        let Page::Clipboard(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(clipboard)) = (page.selected_row(), self.clipboard.clone()) else {
            return Task::none();
        };
        let id = row.id.clone();
        Task::perform(
            async move { clipboard.clipboard_paste(id).await },
            Message::ClipboardPasted,
        )
    }

    fn copy_selected_clipboard_entry(&mut self) -> Task<Message> {
        let Page::Clipboard(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(clipboard)) = (page.selected_row(), self.clipboard.clone()) else {
            return Task::none();
        };
        let id = row.id.clone();
        Task::perform(
            async move { clipboard.clipboard_content(id).await },
            Message::ClipboardContentLoaded,
        )
    }
}

/// The detail pane as a document: what it draws above the metadata, then the
/// metadata, or a copied file's preview as Search Files describes it.
fn clipboard_detail_document(
    page: &ClipboardPage,
    detail: &clipboard_page::Detail,
) -> crate::semantics::Document {
    use crate::semantics::text_blocks;
    let blocks = match &detail.pane {
        Some(DetailContent::File(preview)) => {
            return super::a11y::file_preview_document(preview, true);
        }
        Some(DetailContent::Text(body)) => text_blocks(body),
        Some(DetailContent::Error(title, reason)) => {
            let mut blocks = text_blocks(title);
            blocks.extend(text_blocks(reason));
            blocks
        }
        // An image says nothing beyond the entry it is, which labels the
        // document.
        Some(DetailContent::Image(_) | DetailContent::None) | None => Vec::new(),
    };
    let metadata = match &detail.info {
        Some(Ok(info)) => super::a11y::metadata(clipboard_page::detail_fields(info)),
        _ => Vec::new(),
    };
    let label = page
        .rows
        .iter()
        .find(|row| row.id == detail.id)
        .map_or_else(|| "Clipboard entry".to_owned(), |row| row.preview.clone());
    crate::semantics::Document {
        label,
        blocks,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RootRow;
    use crate::app::flags::AppFlags;
    use crate::app::test_support::{
        FakeClipboard, TestBackend, choose, chord, clip_row, index, panel_titles, pressed, settle,
    };
    use crate::backend::{ClipboardDetail, ClipboardMonitoring, ClipboardRow, ClipboardRowKind};
    use crate::clipboard_page::Status;
    use crate::semantics::Body;
    use std::sync::Arc;

    fn entry(id: &str, preview: &str, pinned: bool) -> ClipboardRow {
        ClipboardRow {
            id: id.into(),
            preview: preview.into(),
            kind: ClipboardRowKind::Text,
            pinned,
            url_host: None,
        }
    }

    #[test]
    fn history_describes_its_entries_and_the_selected_ones_detail() {
        let app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = ClipboardPage {
            rows: vec![entry("1", "hello world", true), entry("2", "second", false)],
            selected: 1,
            status: Status::Ready,
            monitoring: Some(ClipboardMonitoring {
                supported: true,
                enabled: false,
            }),
            ..ClipboardPage::default()
        };
        page.detail = Some(clipboard_page::Detail {
            id: "2".into(),
            info: Some(Ok(ClipboardDetail {
                id: "2".into(),
                mime_type: "text/plain".into(),
                kind: ClipboardRowKind::Text,
                size: 6,
                md5: String::new(),
                updated_at: 0,
                encrypted: false,
                keywords: String::new(),
            })),
            content: None,
            pane: Some(DetailContent::Text("second\n\nline".into())),
            image: None,
        });
        let semantics = app.clipboard_semantics(&page);
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Clipboard history");
        assert_eq!(list.items[0].name, "hello world");
        assert_eq!(list.items[0].description.as_deref(), Some("Text · Pinned"));
        assert_eq!(list.items[1].description.as_deref(), Some("Text"));
        assert_eq!(list.selected, Some(1));
        assert_eq!(
            semantics.status.as_deref(),
            Some("Clipboard monitoring is paused")
        );
        let detail = semantics.detail.expect("the detail pane");
        assert_eq!(detail.label, "second");
        let blocks: Vec<&str> = detail.blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(blocks, ["second", "line"]);
        let labels: Vec<&str> = detail.metadata.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels.first(), Some(&"Type"));
        assert!(
            detail
                .metadata
                .iter()
                .any(|m| m.label == "MIME type" && m.value == "text/plain"),
            "{labels:?}"
        );
        assert!(!labels.contains(&"MD5"), "an empty value is left out");

        page.notice = Some("Images cannot be copied back yet.".into());
        assert_eq!(
            app.clipboard_semantics(&page).status.as_deref(),
            Some("Images cannot be copied back yet.")
        );
    }

    #[test]
    fn history_says_what_the_view_shows_in_place_of_the_list() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = ClipboardPage::default();
        assert_eq!(
            app.clipboard_semantics(&page).status.as_deref(),
            Some("Loading clipboard history…")
        );
        page.status = Status::Ready;
        app.page = Page::Clipboard(page);
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(semantics.status.as_deref(), Some("Nothing copied yet"));
        assert_eq!(semantics.detail, None);
    }

    fn clipboard_app(
        dir: &std::path::Path,
        clipboard: Option<Arc<FakeClipboard>>,
    ) -> (LauncherApp, Arc<TestBackend>) {
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir));
        app.apply(AppFlags {
            clipboard: clipboard.map(|c| c as Arc<dyn crate::backend::ClipboardBackend>),
            ..AppFlags::default()
        });
        // The ranking backend only for `record_launch`; search stays local so
        // the command row comes from the real index.
        app.backend = Some(backend.clone());
        (app, backend)
    }

    fn open_clipboard(app: &mut LauncherApp) -> Vec<String> {
        app.query = "clipboard".into();
        app.search();
        assert_eq!(
            app.selected_row(),
            Some(RootRow::Command(
                compass_core::commands::by_id("commands:clipboard-history").unwrap()
            )),
            "{}",
            app.state_line()
        );
        let task = app.update(Message::LaunchSelected);
        settle(app, task)
    }

    #[test]
    fn enter_on_the_command_opens_clipboard_history_and_records_the_use() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "newest"), clip_row("2", "older")],
            ..FakeClipboard::default()
        });
        let (mut app, backend) = clipboard_app(dir.path(), Some(clipboard.clone()));
        open_clipboard(&mut app);

        assert!(app.showing_clipboard());
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.status, crate::clipboard_page::Status::Ready);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            backend.recorded.lock().unwrap().as_slice(),
            ["commands:clipboard-history"]
        );
        assert!(
            app.state_line().contains("page=clipboard"),
            "{}",
            app.state_line()
        );
    }

    #[test]
    fn a_copied_link_opens_and_opens_with_a_chosen_application() {
        let dir = tempfile::tempdir().unwrap();
        let mut link = clip_row("1", "https://docs.rs/serde");
        link.kind = crate::backend::ClipboardRowKind::Link;
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![link],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"https://docs.rs/serde".to_vec(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, backend) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let titles = panel_titles(&app);
        assert_eq!(
            titles[..4],
            [
                "Paste to active window",
                "Copy to clipboard",
                "Open",
                "Open with..."
            ]
        );
        let task = choose(&mut app, "Open with...");
        settle(&mut app, task);
        assert!(
            matches!(app.page, Page::OpenWith(_)),
            "{}",
            app.state_line()
        );
        assert_eq!(
            backend.opener_lookups.lock().unwrap().as_slice(),
            ["https://docs.rs/serde"]
        );
        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        assert!(
            matches!(app.page, Page::Clipboard(_)),
            "Escape returns to the history: {}",
            app.state_line()
        );
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let task = choose(&mut app, "Open");
        settle(&mut app, task);
        assert_eq!(
            backend.opened_urls.lock().unwrap().as_slice(),
            ["https://docs.rs/serde"]
        );
    }

    #[test]
    fn plain_text_offers_no_open() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "some text")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"some text".to_vec(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        assert!(!panel_titles(&app).iter().any(|t| t.starts_with("Open")));
    }

    #[test]
    fn the_kind_filter_the_pane_keywords_remove_all_and_monitoring() {
        let dir = tempfile::tempdir().unwrap();
        let mut image = clip_row("2", "Image");
        image.kind = crate::backend::ClipboardRowKind::Image;
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "some text"), image],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"some text".to_vec(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard.clone()));
        app.view.view_memory = crate::view_memory::ViewMemory::load(None);
        open_clipboard(&mut app);

        // The pane shows the selected entry, its text and its metadata.
        {
            let Page::Clipboard(page) = &app.page else {
                unreachable!()
            };
            let detail = page.detail.as_ref().expect("the pane is loaded");
            assert_eq!(detail.id, "1");
            assert_eq!(
                detail.pane,
                Some(crate::clipboard_page::DetailContent::Text(
                    "some text".into()
                ))
            );
            assert!(matches!(&detail.info, Some(Ok(info)) if info.size == 1536));
            let mut ui = iced_test::simulator(app.view());
            assert!(ui.find("1.50 KB").is_ok(), "the size is in the pane");
        }

        // The filter asks the engine for one kind, and is remembered.
        let task = app.update(Message::ClipboardKindChanged("Images".into()));
        settle(&mut app, task);
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.rows.len(), 1);
        assert_eq!(
            clipboard.kinds.lock().unwrap().last(),
            Some(&Some(crate::backend::ClipboardRowKind::Image))
        );
        assert_eq!(
            app.view
                .view_memory
                .get(crate::clipboard_page::FILTER_MEMORY_KEY),
            Some("image")
        );
        let task = app.update(Message::ClipboardKindChanged("All".into()));
        settle(&mut app, task);

        // Ctrl+E opens the keyword form with what is stored; saving it goes
        // back to the history.
        let task = app.update(chord("e", iced::keyboard::Modifiers::CTRL));
        settle(&mut app, task);
        assert!(
            matches!(&app.page, Page::Preferences(form)
                if form.purpose == crate::preferences_page::Purpose::ClipboardKeywords
                    && form.command_id == "1"),
            "{}",
            app.state_line()
        );
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("invoice".into()),
        ));
        let task = app.update(Message::PreferencesSubmit);
        settle(&mut app, task);
        assert!(app.showing_clipboard());
        assert!(
            clipboard
                .changes
                .lock()
                .unwrap()
                .contains(&"keywords 1 invoice".to_owned())
        );

        // Remove-all asks first; Escape leaves everything, Enter removes.
        let _ = app.update(Message::TogglePanel);
        let remove_all = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Remove all"))
            .expect("the panel offers remove-all");
        let _ = app.update(Message::PanelClicked(remove_all));
        assert!(app.confirm.is_some());
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(app.confirm.is_none());
        assert!(
            !clipboard
                .changes
                .lock()
                .unwrap()
                .contains(&"remove-all".to_owned())
        );
        let _ = app.update(chord(
            "x",
            iced::keyboard::Modifiers::CTRL | iced::keyboard::Modifiers::SHIFT,
        ));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        assert!(
            clipboard
                .changes
                .lock()
                .unwrap()
                .contains(&"remove-all".to_owned())
        );

        // The panel pauses recording, and then offers to resume it.
        let _ = app.update(Message::TogglePanel);
        let pause = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Pause clipboard"))
            .expect("recording can be paused");
        let task = app.update(Message::PanelClicked(pause));
        settle(&mut app, task);
        assert_eq!(*clipboard.monitoring.lock().unwrap(), Some(false));
        let _ = app.update(Message::TogglePanel);
        assert!(
            app.panel
                .as_ref()
                .and_then(|panel| panel.row_titled("Resume clipboard"))
                .is_some()
        );
    }

    #[test]
    fn typing_filters_arrows_move_and_escape_goes_back_not_away() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![
                clip_row("1", "alpha one"),
                clip_row("2", "alpha two"),
                clip_row("3", "beta"),
            ],
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard.clone()));
        open_clipboard(&mut app);

        let task = app.update(Message::ClipboardQueryChanged("alpha".into()));
        settle(&mut app, task);
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            clipboard.queries.lock().unwrap().last().map(String::as_str),
            Some("alpha")
        );

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.selected, 1);

        let back = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(!app.showing_clipboard(), "Escape leaves the view");
        assert_eq!(app.query, "clipboard", "and the root query is where it was");
        // Leaving the view by hiding the window would also reset it, so the
        // state alone cannot tell "back" from "away": the task can.
        let actions: Vec<_> = iced_winit::runtime::task::into_stream(back)
            .map(|stream| {
                iced::futures::executor::block_on(iced::futures::StreamExt::collect::<Vec<_>>(
                    stream,
                ))
            })
            .unwrap_or_default();
        assert!(
            !actions.iter().any(|action| matches!(
                action,
                iced_winit::runtime::Action::Exit | iced_winit::runtime::Action::Window(_)
            )),
            "Escape in a command view must not hide or quit the launcher"
        );
    }

    #[test]
    fn enter_pastes_through_the_engine_and_hides_without_copying() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "newest"), clip_row("2", "older")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"older".to_vec(),
            }),
            can_paste: true,
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard.clone()));
        open_clipboard(&mut app);

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        let writes = settle(&mut app, task);
        assert_eq!(clipboard.pasted.lock().unwrap().as_slice(), ["2"]);
        assert!(
            writes.is_empty(),
            "the engine put it on the clipboard; the launcher must not race it"
        );
        assert!(
            !app.showing_clipboard(),
            "and the launcher got out of the way"
        );
    }

    #[test]
    fn ctrl_shift_p_toggles_the_pin_and_ctrl_x_removes_then_the_list_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let pinned = crate::backend::ClipboardRow {
            pinned: true,
            ..clip_row("2", "pinned one")
        };
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "loose one"), pinned],
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard.clone()));
        open_clipboard(&mut app);
        let loads = clipboard.queries.lock().unwrap().len();

        let ctrl = iced::keyboard::Modifiers::CTRL;
        let ctrl_shift = iced::keyboard::Modifiers::CTRL | iced::keyboard::Modifiers::SHIFT;
        let task = app.update(chord("P", ctrl_shift));
        settle(&mut app, task);
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        let task = app.update(chord("P", ctrl_shift));
        settle(&mut app, task);
        let task = app.update(chord("x", ctrl));
        settle(&mut app, task);

        assert_eq!(
            clipboard.changes.lock().unwrap().as_slice(),
            ["pin 1 true", "pin 2 false", "remove 2"],
            "the pin flips each entry's own state, and the selection stays on its entry \
             through each reload, so Ctrl+X removes the entry the person was on"
        );
        assert_eq!(
            clipboard.queries.lock().unwrap().len(),
            loads + 3,
            "every change reloads the list"
        );
        assert!(app.showing_clipboard(), "and the view stays open");
    }

    #[test]
    fn a_refused_change_says_why_and_keeps_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "only one")],
            fail_changes: true,
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);

        let task = app.update(chord("x", iced::keyboard::Modifiers::CTRL));
        settle(&mut app, task);
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.notice.as_deref(), Some("That entry is already gone"));
        assert_eq!(page.rows.len(), 1);
    }

    #[test]
    fn enter_copies_the_whole_entry_and_hides_when_the_engine_cannot_paste() {
        let dir = tempfile::tempdir().unwrap();
        let whole = "the whole entry, not its preview ".repeat(10);
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "the whole entry…")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: whole.clone().into_bytes(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);

        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        let writes = settle(&mut app, task);
        assert_eq!(writes, [whole]);
        assert!(
            !app.showing_clipboard(),
            "hiding resets the view for the next summon"
        );
    }

    #[test]
    fn an_image_is_not_copied_as_text_and_the_view_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "Image")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "image/png".into(),
                data: vec![0x89, b'P', b'N', b'G'],
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);

        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(settle(&mut app, task).is_empty(), "nothing written");
        let Page::Clipboard(page) = &app.page else {
            panic!("the view stays open to show the reason")
        };
        assert!(page.notice.as_deref().is_some_and(|n| n.contains("images")));
    }

    #[test]
    fn without_an_engine_the_view_says_so_instead_of_showing_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) = clipboard_app(dir.path(), None);
        open_clipboard(&mut app);
        let Page::Clipboard(page) = &app.page else {
            unreachable!()
        };
        assert!(
            matches!(&page.status, crate::clipboard_page::Status::Failed(reason) if reason.contains("engine")),
            "{:?}",
            page.status
        );
    }

    #[test]
    fn clicking_a_clipboard_row_copies_it() {
        let dir = tempfile::tempdir().unwrap();
        let clipboard = Arc::new(FakeClipboard {
            rows: vec![clip_row("1", "first"), clip_row("2", "second entry")],
            content: Some(crate::backend::ClipboardContent {
                mime_type: "text/plain".into(),
                data: b"second entry".to_vec(),
            }),
            ..FakeClipboard::default()
        });
        let (mut app, _) = clipboard_app(dir.path(), Some(clipboard));
        open_clipboard(&mut app);

        let mut ui = iced_test::simulator(app.view());
        ui.click("second entry").expect("the row is drawn");
        let messages = ui.into_messages().collect::<Vec<_>>();
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::ClipboardSelected(1))),
            "{messages:?}"
        );
        let mut writes = Vec::new();
        for message in messages {
            let task = app.update(message);
            writes.extend(settle(&mut app, task));
        }
        assert_eq!(writes, ["second entry"]);
    }
}
