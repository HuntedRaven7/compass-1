//! Clipboard history in the launcher, beyond search, copy, paste, pin and
//! remove: the kind filter, the detail pane, keyword editing, remove-all and
//! the monitoring switch (`ClipboardHistoryViewHost`,
//! `ClipboardHistorySection::actionPanel`).

use iced::keyboard::{Key, Modifiers};

use super::{
    Alignment, ClipboardChange, Confirm, ConfirmAction, Element, LauncherApp, Length, Message,
    Padding, Page, PanelSection, PanelState, Space, Task, column, container, focus_search, image,
    row, scrollable, text,
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
    use crate::backend::{ClipboardDetail, ClipboardMonitoring, ClipboardRow, ClipboardRowKind};
    use crate::clipboard_page::Status;
    use crate::semantics::Body;

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
}
