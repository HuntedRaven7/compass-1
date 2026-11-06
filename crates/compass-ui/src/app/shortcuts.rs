//! Shortcuts (quicklinks) in the launcher: their root rows, Manage Shortcuts,
//! and the forms that create, edit and open them.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the decisions that do not need that state are in
//! [`crate::shortcuts_page`].

use compass_core::shortcut_form::Mode;
use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    RootRow, Space, Task, chord_direction, column, container, focus_search, mouse_area,
    next_selection, row, scrollable, text,
};
use crate::action_panel::Action;
use crate::shortcuts_page::{self, ShortcutsPage};

const OPEN: &str = "shortcut.open";
const OPEN_WITH: &str = "shortcut.open-with";
const COPY: &str = "shortcut.copy";
const EDIT: &str = "shortcut.edit";
const DUPLICATE: &str = "shortcut.duplicate";
const REMOVE: &str = "shortcut.remove";

/// The actions a shortcut row offers, as `RootShortcutItem::newActionPanel`
/// arranges them.
pub(super) fn panel_sections(in_manage: bool) -> Vec<PanelSection> {
    vec![
        PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("Open shortcut")
                    .with_id(OPEN)
                    .with_shortcut("enter"),
                Action::new("Open with...")
                    .with_id(OPEN_WITH)
                    .with_shortcut("ctrl+o"),
                Action::new("Copy shortcut").with_id(COPY),
            ],
        },
        PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("Edit shortcut")
                    .with_id(EDIT)
                    .with_shortcut("Ctrl+E"),
                Action::new("Duplicate link")
                    .with_id(DUPLICATE)
                    .with_shortcut("Ctrl+D"),
            ],
        },
        PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("Remove link")
                    .with_id(REMOVE)
                    .with_shortcut(if in_manage { "Ctrl+X" } else { "Ctrl+Shift+X" }),
            ],
        },
    ]
}

impl LauncherApp {
    /// Asks the engine for the shortcut list, so root search and Manage
    /// Shortcuts show what the store holds now.
    pub(super) fn refresh_shortcuts_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.list_shortcuts().await },
            Message::ShortcutsLoaded,
        )
    }

    /// The shortcut the selection is on: a root row, or a Manage Shortcuts
    /// row. By position in the index's list.
    pub(super) fn selected_shortcut(&self) -> Option<usize> {
        match &self.page {
            Page::Shortcuts(page) => page.selected_index(),
            Page::Root => match self.selected_row()? {
                RootRow::Shortcut(index) => Some(index),
                _ => None,
            },
            _ => None,
        }
    }

    /// Shows why a shortcut action did not happen, where the person is.
    fn shortcut_notice(&mut self, reason: String) {
        match &mut self.page {
            Page::Shortcuts(page) => page.notice = Some(reason),
            Page::Preferences(page) => page.notice = Some(reason),
            _ => self.error = Some(reason),
        }
    }

    /// Takes a new list: root search and Manage Shortcuts both follow it.
    fn apply_shortcuts(&mut self, shortcuts: Vec<crate::backend::Shortcut>) {
        self.app_index.set_shortcuts(
            shortcuts
                .iter()
                .map(compass_core::shortcut_service::from_serialized)
                .collect(),
        );
        if let Page::Shortcuts(page) = &mut self.page {
            page.refilter(self.app_index.shortcuts());
            page.detail = None;
        }
    }

    /// Opens Manage Shortcuts, as it was left when a form was opened over it.
    pub(super) fn open_manage_shortcuts(&mut self) -> Task<Message> {
        let mut page = self.parked_shortcuts.take().unwrap_or_default();
        page.refilter(self.app_index.shortcuts());
        self.page = Page::Shortcuts(page);
        Task::batch([
            self.refresh_shortcuts_task(),
            self.shortcut_detail_task(),
            focus_search(),
        ])
    }

    /// Opens the shortcut form: creating, or editing or duplicating the
    /// shortcut at `index`.
    pub(super) fn open_shortcut_form(
        &mut self,
        mode: Mode,
        index: Option<usize>,
        from_manage: bool,
    ) -> Task<Message> {
        let mut applications: Vec<(String, String)> = self
            .app_index
            .applications()
            .map(|item| (item.display_name(), item.desktop_id().to_owned()))
            .collect();
        applications.sort_by_key(|(name, _)| name.to_lowercase());
        let existing = index.and_then(|index| self.app_index.shortcuts().get(index));
        self.panel = None;
        self.parked_shortcuts = match &self.page {
            Page::Shortcuts(page) if from_manage => Some(page.clone()),
            _ => None,
        };
        self.page = Page::Preferences(Box::new(shortcuts_page::form(
            mode,
            existing,
            &applications,
            from_manage,
        )));
        Task::none()
    }

    /// Opens the shortcut at `index`: through its arguments form when its
    /// link takes arguments, at once otherwise.
    pub(super) fn open_shortcut_at(&mut self, index: usize) -> Task<Message> {
        let Some(shortcut) = self.app_index.shortcuts().get(index) else {
            return Task::none();
        };
        self.panel = None;
        if let Some(form) = shortcuts_page::arguments_form(shortcut) {
            self.page = Page::Preferences(Box::new(form));
            return Task::none();
        }
        let id = shortcut.id.clone();
        self.send_open_shortcut(id, Vec::new())
    }

    /// Asks the engine to open a shortcut, and counts the use in root search.
    pub(super) fn send_open_shortcut(
        &mut self,
        id: String,
        arguments: Vec<String>,
    ) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            self.shortcut_notice(
                "Shortcuts need the Compass engine, and this window is running without one"
                    .to_owned(),
            );
            return Task::none();
        };
        let key = compass_core::root_items::entrypoint_id(
            compass_core::shortcut::SHORTCUTS_PROVIDER_ID,
            &id,
        );
        Task::perform(
            async move {
                backend.open_shortcut(id, arguments).await?;
                if let Err(error) = backend.record_launch(key).await {
                    tracing::warn!(%error, "could not record opening a shortcut");
                }
                Ok(())
            },
            Message::ShortcutOpened,
        )
    }

    /// Copies the shortcut at `index`, expanded, as `CopyShortcutAction` does.
    fn copy_shortcut_at(&mut self, index: usize) -> Task<Message> {
        let (Some(shortcut), Some(backend)) =
            (self.app_index.shortcuts().get(index), self.backend.clone())
        else {
            return Task::none();
        };
        let id = shortcut.id.clone();
        self.panel = None;
        Task::perform(
            async move { backend.expand_shortcut(id, Vec::new()).await },
            Message::ShortcutExpanded,
        )
    }

    /// "Open with…" over the shortcut at `index`
    /// (`OpenCompletedShortcutWithAction`): its link expanded, the
    /// applications looked up by the link as stored.
    fn open_shortcut_with(&mut self, index: usize) -> Task<Message> {
        let (Some(shortcut), Some(backend)) =
            (self.app_index.shortcuts().get(index), self.backend.clone())
        else {
            return Task::none();
        };
        let (id, raw) = (shortcut.id.clone(), shortcut.link.raw.clone());
        self.panel = None;
        Task::perform(
            async move {
                let expanded = backend.expand_shortcut(id, Vec::new()).await?;
                Ok((expanded, raw))
            },
            Message::OpenWithTarget,
        )
    }

    /// Asks for the detail pane of the selected shortcut in Manage
    /// Shortcuts, unless it is already showing: the link expanded, and the
    /// application that opens it (`loadDetail`).
    pub(super) fn shortcut_detail_task(&mut self) -> Task<Message> {
        let Page::Shortcuts(page) = &self.page else {
            return Task::none();
        };
        let Some(shortcut) = page
            .selected_index()
            .and_then(|index| self.app_index.shortcuts().get(index))
        else {
            return Task::none();
        };
        if page.detail.as_ref().is_some_and(|d| d.id == shortcut.id) {
            return Task::none();
        }
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        let chosen = (shortcut.app != compass_core::shortcut_form::DEFAULT_APP)
            .then(|| {
                self.app_index
                    .applications()
                    .find(|item| {
                        item.desktop_id() == shortcut.app
                            || item.desktop_id().strip_suffix(".desktop") == Some(&shortcut.app)
                    })
                    .map(|item| shortcuts_page::app_label(&item.display_name(), false))
            })
            .flatten();
        let (id, raw) = (shortcut.id.clone(), shortcut.link.raw.clone());
        let is_default = shortcut.app == compass_core::shortcut_form::DEFAULT_APP;
        Task::perform(
            async move {
                let expanded = backend.expand_shortcut(id.clone(), Vec::new()).await;
                let app = if is_default {
                    backend.list_openers(raw).await.ok().and_then(|apps| {
                        apps.into_iter()
                            .find(|app| app.default)
                            .map(|app| shortcuts_page::app_label(&app.name, true))
                    })
                } else {
                    chosen
                };
                shortcuts_page::Detail { id, expanded, app }
            },
            Message::ShortcutDetailLoaded,
        )
    }

    /// Removes the shortcut at `index`.
    fn remove_shortcut_at(&mut self, index: usize) -> Task<Message> {
        let (Some(shortcut), Some(backend)) =
            (self.app_index.shortcuts().get(index), self.backend.clone())
        else {
            return Task::none();
        };
        let id = shortcut.id.clone();
        self.panel = None;
        Task::perform(
            async move { backend.remove_shortcut(id).await },
            Message::ShortcutRemoved,
        )
    }

    /// Runs a shortcut action from the panel, if `id` is one.
    pub(super) fn shortcut_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let index = self.selected_shortcut()?;
        let from_manage = matches!(self.page, Page::Shortcuts(_));
        let task = match id {
            OPEN => self.open_shortcut_at(index),
            OPEN_WITH => self.open_shortcut_with(index),
            COPY => self.copy_shortcut_at(index),
            EDIT => self.open_shortcut_form(Mode::Edit, Some(index), from_manage),
            DUPLICATE => self.open_shortcut_form(Mode::Duplicate, Some(index), from_manage),
            REMOVE => self.remove_shortcut_at(index),
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// Opens the action panel over the selected shortcut, if one is.
    pub(super) fn open_shortcut_panel(&mut self) -> Option<Task<Message>> {
        self.selected_shortcut()?;
        let in_manage = matches!(self.page, Page::Shortcuts(_));
        self.panel = Some(PanelState::new(panel_sections(in_manage)));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// The keys a shortcut row takes without opening the panel: Ctrl+E edit,
    /// Ctrl+D duplicate, and remove — Ctrl+X in Manage Shortcuts
    /// (`action.remove`), Ctrl+Shift+X in root search
    /// (`action.dangerous-remove`), as the C++ binds them.
    pub(super) fn shortcut_chord(
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
        let index = self.selected_shortcut()?;
        let in_manage = matches!(self.page, Page::Shortcuts(_));
        let c = c.to_lowercase();
        match (c.as_str(), modifiers.shift()) {
            ("e", false) => Some(self.open_shortcut_form(Mode::Edit, Some(index), in_manage)),
            ("d", false) => Some(self.open_shortcut_form(Mode::Duplicate, Some(index), in_manage)),
            ("x", shift) if shift != in_manage => Some(self.remove_shortcut_at(index)),
            _ => None,
        }
    }

    /// Manage Shortcuts' keys: the list's navigation, Enter to open, Escape
    /// to go back.
    pub(super) fn shortcuts_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Shortcuts(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => {
                return match page.selected_index() {
                    Some(index) => self.open_shortcut_at(index),
                    None => Task::none(),
                };
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
            return Task::batch([
                crate::scroll::reveal_root_selection(),
                self.shortcut_detail_task(),
            ]);
        }
        Task::none()
    }

    /// Submits a shortcut form: its arguments, to open it, or its fields, to
    /// save it. `None` when the form showing is not a shortcut form.
    pub(super) fn submit_shortcut_form(&mut self) -> Option<Task<Message>> {
        use crate::preferences_page::Purpose;
        let Page::Preferences(page) = &mut self.page else {
            return None;
        };
        match page.purpose {
            Purpose::ShortcutArguments => {
                if let Err(missing) = page.submission() {
                    page.notice = Some(format!("Fill in {}", missing.join(", ")));
                    return Some(Task::none());
                }
                let id = page.command_id.clone();
                let arguments = shortcuts_page::argument_values(page);
                Some(self.send_open_shortcut(id, arguments))
            }
            Purpose::ShortcutForm { mode, .. } => {
                if let Err(missing) = page.submission() {
                    page.notice = Some(format!(
                        "{}: fill in {}",
                        compass_core::shortcut_form::VALIDATION_FAILED,
                        missing.join(", ")
                    ));
                    return Some(Task::none());
                }
                let (name, url, app, icon) = shortcuts_page::form_values(page);
                let draft = crate::backend::ShortcutDraft {
                    id: (mode == Mode::Edit).then(|| page.command_id.clone()),
                    name,
                    icon,
                    url,
                    app,
                };
                let Some(backend) = self.backend.clone() else {
                    page.notice = Some(
                        "Shortcuts need the Compass engine, and this window is running without one"
                            .to_owned(),
                    );
                    return Some(Task::none());
                };
                Some(Task::perform(
                    async move { backend.save_shortcut(draft).await },
                    Message::ShortcutSaved,
                ))
            }
            _ => None,
        }
    }

    /// Where Escape goes from a shortcut form: back to Manage Shortcuts when
    /// it was opened from there.
    pub(super) fn back_from_shortcut_form(&mut self) -> Option<Task<Message>> {
        use crate::preferences_page::Purpose;
        let Page::Preferences(page) = &self.page else {
            return None;
        };
        let Purpose::ShortcutForm {
            from_manage: true, ..
        } = page.purpose
        else {
            return None;
        };
        Some(self.open_manage_shortcuts())
    }

    /// Handles the shortcut messages.
    pub(super) fn shortcut_message(&mut self, message: Message) -> Task<Message> {
        use crate::preferences_page::Purpose;
        match message {
            Message::ShortcutsLoaded(Ok(shortcuts)) => {
                self.apply_shortcuts(shortcuts);
                self.shortcut_detail_task()
            }
            Message::ShortcutDetailLoaded(detail) => {
                if let Page::Shortcuts(page) = &mut self.page
                    && page
                        .selected_index()
                        .and_then(|index| self.app_index.shortcuts().get(index))
                        .is_some_and(|shortcut| shortcut.id == detail.id)
                {
                    page.detail = Some(detail);
                }
                Task::none()
            }
            Message::ShortcutsLoaded(Err(reason)) => {
                tracing::debug!(%reason, "could not list shortcuts");
                Task::none()
            }
            Message::ShortcutSaved(Ok(shortcuts)) => {
                self.apply_shortcuts(shortcuts);
                let from_manage = matches!(
                    &self.page,
                    Page::Preferences(page)
                        if matches!(page.purpose, Purpose::ShortcutForm { from_manage: true, .. })
                );
                if from_manage {
                    return self.open_manage_shortcuts();
                }
                self.page = Page::Root;
                self.error = None;
                Task::batch([self.search_task(), focus_search()])
            }
            Message::ShortcutSaved(Err(reason)) | Message::ShortcutRemoved(Err(reason)) => {
                self.shortcut_notice(reason);
                Task::none()
            }
            Message::ShortcutRemoved(Ok(shortcuts)) => {
                self.apply_shortcuts(shortcuts);
                if matches!(self.page, Page::Root) {
                    return self.search_task();
                }
                Task::none()
            }
            Message::ShortcutOpened(Ok(())) => {
                if matches!(self.page, Page::Preferences(_)) {
                    self.page = Page::Root;
                }
                // `OpenShortcutAction::setClearSearch(true)` from root search.
                self.query.clear();
                self.results.clear();
                Task::batch([self.refresh_shortcuts_task(), self.conceal()])
            }
            Message::ShortcutOpened(Err(reason)) => {
                self.shortcut_notice(reason);
                Task::none()
            }
            Message::ShortcutExpanded(Ok(text)) => {
                let hud = crate::hud::Hud::new("Copied to clipboard");
                Task::batch([
                    super::hud::copy_text(self.backend.clone(), text),
                    self.show_hud(hud),
                ])
            }
            Message::ShortcutExpanded(Err(reason)) => {
                self.shortcut_notice(reason);
                Task::none()
            }
            Message::ShortcutsQueryChanged(query) => {
                if let Page::Shortcuts(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.selected = 0;
                    page.refilter(self.app_index.shortcuts());
                }
                Task::batch([
                    crate::scroll::reveal_root_selection(),
                    self.shortcut_detail_task(),
                ])
            }
            Message::ShortcutSelected(position) => {
                let Page::Shortcuts(page) = &mut self.page else {
                    return Task::none();
                };
                if position >= page.shown.len() {
                    return Task::none();
                }
                page.selected = position;
                match page.selected_index() {
                    Some(index) => self.open_shortcut_at(index),
                    None => Task::none(),
                }
            }
            _ => Task::none(),
        }
    }

    /// The detail pane beside Manage Shortcuts' list: the link expanded,
    /// then the metadata `loadDetail` lists.
    fn shortcut_detail_pane<'a>(
        &'a self,
        shortcut: &'a compass_core::shortcut_service::CachedShortcut,
        detail: &'a shortcuts_page::Detail,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let muted = |value: String| {
            text(value)
                .font(self.font())
                .size(12)
                .color(palette.muted.to_iced())
        };
        let content: Element<'a, Message> = match &detail.expanded {
            Ok(link) => text(link.as_str())
                .font(iced::Font::MONOSPACE)
                .size(12)
                .color(palette.text.to_iced())
                .into(),
            Err(reason) => muted(reason.clone()).into(),
        };
        let mut fields = column![].spacing(4);
        for (label, value) in shortcut_detail_fields(shortcut, detail) {
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

    /// Manage Shortcuts' body.
    pub(super) fn shortcuts_body<'a>(&'a self, page: &'a ShortcutsPage) -> Element<'a, Message> {
        if let Some(message) = self.shortcuts_empty_notice(page) {
            return match &page.notice {
                Some(notice) => column![self.notice(message), self.notice(notice)].into(),
                None => self.notice(message),
            };
        }
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(shortcut) = self.app_index.shortcuts().get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let title = shortcuts_page::display_name(shortcut);
            let row = self.list_row(
                self.initial_badge(title, selected),
                title.to_owned(),
                self.subtitles.then(|| shortcut.link.raw.clone()),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::ShortcutSelected(position))
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
        let rows: Element<Message> = match self.shortcut_detail_shown(page) {
            Some((shortcut, detail)) => row![
                container(rows).width(Length::FillPortion(super::preview::LIST_PORTION)),
                self.shortcut_detail_pane(shortcut, detail),
            ]
            .into(),
            None => rows.into(),
        };
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows,
        }
    }

    /// What Manage Shortcuts says in place of the list: that there are none
    /// yet, or that none match.
    fn shortcuts_empty_notice(&self, page: &ShortcutsPage) -> Option<&'static str> {
        if !page.shown.is_empty() {
            return None;
        }
        Some(if self.app_index.shortcuts().is_empty() {
            "No shortcuts yet. Create one with Create Shortcut."
        } else {
            "No shortcuts match"
        })
    }

    /// The selected shortcut and its detail, when the detail that arrived is
    /// still the selected one's and so is drawn beside the list.
    fn shortcut_detail_shown<'a>(
        &'a self,
        page: &'a ShortcutsPage,
    ) -> Option<(
        &'a compass_core::shortcut_service::CachedShortcut,
        &'a shortcuts_page::Detail,
    )> {
        let shortcut = self.app_index.shortcuts().get(page.selected_index()?)?;
        let detail = page
            .detail
            .as_ref()
            .filter(|detail| detail.id == shortcut.id)?;
        Some((shortcut, detail))
    }

    /// Manage Shortcuts' rows and the selected one's detail, for assistive
    /// technology.
    pub(super) fn shortcuts_semantics(&self, page: &ShortcutsPage) -> super::a11y::PageSemantics {
        let items = page
            .shown
            .iter()
            .map(|&index| {
                self.app_index.shortcuts().get(index).map_or_else(
                    crate::semantics::Item::default,
                    |shortcut| {
                        super::a11y::item(
                            shortcuts_page::display_name(shortcut),
                            Some(shortcut.link.raw.clone()),
                        )
                    },
                )
            })
            .collect();
        let detail = self.shortcut_detail_shown(page).map(|(shortcut, detail)| {
            let text = match &detail.expanded {
                Ok(link) => link,
                Err(reason) => reason,
            };
            crate::semantics::Document {
                label: shortcuts_page::display_name(shortcut).to_owned(),
                blocks: crate::semantics::text_blocks(text),
                metadata: super::a11y::metadata(shortcut_detail_fields(shortcut, detail)),
            }
        });
        super::a11y::PageSemantics::list(
            "Shortcuts",
            items,
            page.selected,
            self.shortcuts_empty_notice(page),
        )
        .with_status(page.notice.as_deref(), &page.query)
        .with_detail(detail)
    }
}

/// The detail pane's metadata, its dates written as the C++ writes them.
fn shortcut_detail_fields(
    shortcut: &compass_core::shortcut_service::CachedShortcut,
    detail: &shortcuts_page::Detail,
) -> Vec<(&'static str, String)> {
    let date = |at: u64| {
        crate::file_preview::qt_text_date(
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(at),
        )
        .unwrap_or_default()
    };
    shortcuts_page::detail_fields(shortcut, detail.app.as_deref(), date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantics::Body;
    use compass_core::shortcut_service::from_serialized;
    use compass_core::shortcut_store::SerializedShortcut;

    #[test]
    fn manage_shortcuts_describes_its_rows_and_the_selected_detail() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = ShortcutsPage::default();
        page.refilter(app.app_index.shortcuts());
        assert_eq!(
            app.shortcuts_semantics(&page).status.as_deref(),
            Some("No shortcuts yet. Create one with Create Shortcut.")
        );

        app.app_index.set_shortcuts(vec![
            from_serialized(&SerializedShortcut {
                id: "a".into(),
                name: "GitHub Search".into(),
                icon: "icon://omnicast/link".into(),
                url: "https://github.com/search?q={q}".into(),
                app: "default".into(),
                ..SerializedShortcut::default()
            }),
            from_serialized(&SerializedShortcut {
                id: "b".into(),
                name: "Docs".into(),
                icon: "icon://omnicast/link".into(),
                url: "https://docs.rs/{crate}".into(),
                app: "default".into(),
                ..SerializedShortcut::default()
            }),
        ]);
        page.refilter(app.app_index.shortcuts());
        page.selected = 1;
        page.detail = Some(shortcuts_page::Detail {
            id: "b".into(),
            expanded: Ok("https://docs.rs/serde".into()),
            app: Some("Firefox (Default)".into()),
        });
        let semantics = app.shortcuts_semantics(&page);
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.items[1].name, "Docs");
        assert_eq!(
            list.items[1].description.as_deref(),
            Some(app.app_index.shortcuts()[1].link.raw.as_str())
        );
        assert_eq!(list.selected, Some(1));
        let detail = semantics.detail.expect("the detail pane");
        assert_eq!(detail.label, "Docs");
        assert_eq!(detail.blocks[0].text, "https://docs.rs/serde");
        let fields: Vec<(&str, &str)> = detail
            .metadata
            .iter()
            .map(|m| (m.label.as_str(), m.value.as_str()))
            .collect();
        assert_eq!(fields[0], ("Name", "Docs"));
        assert_eq!(fields[1], ("Application", "Firefox (Default)"));
        assert!(fields.contains(&("Last Opened", "Never")), "{fields:?}");

        // A detail for a row no longer selected is not drawn, so not described.
        page.selected = 0;
        assert_eq!(app.shortcuts_semantics(&page).detail, None);

        page.query = "zzzz".into();
        page.refilter(app.app_index.shortcuts());
        app.page = Page::Shortcuts(page);
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(semantics.status.as_deref(), Some("No shortcuts match"));
    }
}
