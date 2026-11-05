//! The extension stores in the launcher: the list, the detail page, install,
//! uninstall and the links.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the page state is in [`crate::store_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, image, mouse_area, next_selection, row,
    scrollable, text,
};
use crate::action_panel::Action;
use crate::backend::Store;
use crate::store_page::{self, Confirm, Status, StoreDetailPage, StorePage, actions};

const NEEDS_ENGINE: &str = "The extension stores need the Compass engine";

/// The detail page's Markdown drawn with its images: those fetched are drawn,
/// the rest keep the renderer's placeholder. Headings and paragraphs carry
/// their words for a find by text, which Iced's `rich_text` does not.
pub(super) struct StoreMarkdown<'b> {
    pub(super) images: &'b std::collections::HashMap<String, crate::extension_page::RowIcon>,
}

impl<'a> iced::widget::markdown::Viewer<'a, Message> for StoreMarkdown<'a> {
    fn on_link_click(url: iced::widget::markdown::Uri) -> Message {
        Message::ExtensionLinkClicked(url)
    }

    fn image(
        &self,
        settings: iced::widget::markdown::Settings,
        url: &'a iced::widget::markdown::Uri,
        title: &'a str,
        alt: &iced::widget::markdown::Text,
    ) -> Element<'a, Message> {
        match self.images.get(url) {
            Some(crate::extension_page::RowIcon::Art { art, .. }) => match art {
                crate::icons::IconArt::Raster(path) => {
                    image(path.clone()).width(Length::Shrink).into()
                }
                crate::icons::IconArt::Vector(path) => {
                    iced::widget::svg(path.clone()).width(Length::Fill).into()
                }
            },
            _ => {
                let _ = alt;
                container(
                    text(if title.is_empty() { "Image" } else { title })
                        .font(settings.style.font)
                        .size(12),
                )
                .padding(4)
                .into()
            }
        }
    }

    fn heading(
        &self,
        settings: iced::widget::markdown::Settings,
        level: &'a iced::widget::markdown::HeadingLevel,
        text: &'a iced::widget::markdown::Text,
        index: usize,
    ) -> Element<'a, Message> {
        crate::labelled::labelled(
            iced::widget::markdown::heading(settings, level, text, index, Self::on_link_click),
            plain(text, settings),
        )
    }

    fn paragraph(
        &self,
        settings: iced::widget::markdown::Settings,
        text: &iced::widget::markdown::Text,
    ) -> Element<'a, Message> {
        crate::labelled::labelled(
            iced::widget::markdown::paragraph(settings, text, Self::on_link_click),
            plain(text, settings),
        )
    }
}

/// What a piece of Markdown says, without its styling.
fn plain(
    text: &iced::widget::markdown::Text,
    settings: iced::widget::markdown::Settings,
) -> String {
    text.spans(settings.style)
        .iter()
        .map(|span| span.text.as_ref())
        .collect()
}

impl LauncherApp {
    /// Opens the detail page a deeplink names (`compass://extensions/<owner>/<name>`,
    /// or the Raycast store's for `raycast://`), as the C++ pushes a detail
    /// host over the root. A link with a command
    /// (`raycast://extensions/<owner>/<name>/<command>`) runs that command
    /// when it is installed, and opens its store page when it is not.
    pub(super) fn open_deeplink(&mut self, url: &str) -> Task<Message> {
        if let Some(link) = compass_core::root_items::parse_launch_link(url) {
            return self.open_launch_link(link);
        }
        if let Some(tab) = compass_core::settings_catalog::parse_settings_link(url) {
            return self.open_settings(tab.as_deref());
        }
        let Some(Ok(link)) = compass_core::store_listing::parse_extension_link(url) else {
            tracing::warn!(%url, "a deeplink the launcher does not handle");
            return Task::none();
        };
        if let Some(command) = &link.command
            && let Some(installed) =
                self.app_index
                    .extension_by_link(&link.owner, &link.name, command)
        {
            return self.open_launch_link(compass_core::root_items::LaunchLink {
                path: format!("{}/{}", installed.provider_id, installed.name),
                fallback_text: None,
                toggle: false,
            });
        }
        let store = if link.raycast {
            Store::Raycast
        } else {
            Store::Vicinae
        };
        self.panel = None;
        let _ = self.close_extension_view();
        self.parked_store = None;
        let mut page = StorePage::new(store);
        let Some(backend) = self.backend.clone() else {
            page.status = Status::Failed(NEEDS_ENGINE.to_owned());
            self.page = Page::Store(page);
            return Task::none();
        };
        self.page = Page::Store(page);
        Task::perform(
            async move { backend.store_extension(store, link.owner, link.name).await },
            Message::StoreDetailLoaded,
        )
    }

    /// A row's right side: installed and downloads, the compatibility tier
    /// as a coloured dot and its name, and the author's avatar.
    fn store_accessory<'a>(
        &'a self,
        entry: &'a crate::backend::StoreRow,
        images: &'a store_page::Images,
        selected: bool,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let colour = if selected {
            palette.selection_text
        } else {
            palette.muted
        }
        .to_iced();
        let size = f32::from(self.geometry.subtitle_size);
        let mut line = row![].spacing(8).align_y(iced::Alignment::Center);
        let status = store_page::status_text(entry);
        if !status.is_empty() {
            line = line.push(text(status).font(self.font()).size(size).color(colour));
        }
        if let Some(tier) = entry.compat {
            let (r, g, b) = store_page::compat_colour(tier);
            let dot = container(iced::widget::Space::new())
                .width(Length::Fixed(8.0))
                .height(Length::Fixed(8.0))
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(iced::Color::from_rgb8(r, g, b).into()),
                    border: iced::Border {
                        radius: 4.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                });
            line = line.push(
                row![
                    dot,
                    text(store_page::compat_label(tier))
                        .font(self.font())
                        .size(size)
                        .color(colour)
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
            );
        }
        if let Some(crate::extension_page::RowIcon::Art { art, .. }) = entry
            .author_avatar
            .as_ref()
            .and_then(|url| images.art.get(url))
        {
            let avatar: Element<'a, Message> = match art {
                crate::icons::IconArt::Raster(path) => image(path.clone())
                    .width(Length::Fixed(18.0))
                    .height(Length::Fixed(18.0))
                    .into(),
                crate::icons::IconArt::Vector(path) => iced::widget::svg(path.clone())
                    .width(Length::Fixed(18.0))
                    .height(Length::Fixed(18.0))
                    .into(),
            };
            line = line.push(avatar);
        }
        line.into()
    }

    /// The uninstall question as a dialog over the page, with its two
    /// buttons, as `UninstallExtensionAction`'s alert.
    fn store_dialog<'a>(
        &'a self,
        content: Element<'a, Message>,
        title: &'a str,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let button = |label: String, confirm: bool| {
            iced::widget::button(text(label).font(self.font()).size(13))
                .on_press(Message::StoreConfirmAnswered(confirm))
                .padding(Padding::new(6.0).left(14).right(14))
        };
        let dialog = container(
            column![
                text(store_page::CONFIRM_TITLE)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..self.font()
                    })
                    .size(15)
                    .color(palette.text.to_iced()),
                text(store_page::CONFIRM_MESSAGE)
                    .font(self.font())
                    .size(13)
                    .color(palette.text.to_iced()),
                row![
                    iced::widget::Space::new().width(Length::Fill),
                    button("Cancel".to_owned(), false),
                    button(format!("Uninstall {title}"), true),
                ]
                .spacing(8),
            ]
            .spacing(10),
        )
        .max_width(420)
        .padding(Padding::new(18.0))
        .style(move |_: &iced::Theme| container::Style {
            background: Some(palette.surface.to_iced().into()),
            border: iced::Border {
                color: palette.border.to_iced(),
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::Style::default()
        });
        let scrim = container(dialog)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::Alignment::Center)
            .align_y(iced::Alignment::Center)
            .style(|_: &iced::Theme| container::Style {
                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35).into()),
                ..container::Style::default()
            });
        iced::widget::stack![content, iced::widget::opaque(scrim)].into()
    }

    /// Whether the theme is dark, which decides an extension's icon.
    fn store_prefers_dark(&self) -> bool {
        let surface = self.palette().surface.to_iced();
        0.299 * surface.r + 0.587 * surface.g + 0.114 * surface.b < 0.5
    }

    /// Opens a store's list, as it was left when a detail page was opened
    /// over it, and asks for its rows again so what is installed is current.
    pub(super) fn open_store(&mut self, store: Store) -> Task<Message> {
        let page = self
            .parked_store
            .take()
            .filter(|page| page.store == store)
            .unwrap_or_else(|| StorePage::new(store));
        let generation = page.generation;
        self.page = Page::Store(page);
        Task::batch([self.store_fetch(generation), focus_search()])
    }

    /// Asks for the rows for search `generation`, unless the text moved on.
    fn store_fetch(&mut self, generation: u64) -> Task<Message> {
        let Page::Store(page) = &mut self.page else {
            return Task::none();
        };
        if page.generation != generation {
            return Task::none();
        }
        let Some(backend) = self.backend.clone() else {
            page.apply(generation, Err(NEEDS_ENGINE.to_owned()));
            return Task::none();
        };
        page.loading = true;
        let (store, query) = (page.store, page.query.clone());
        Task::perform(
            async move { backend.store_browse(store, query).await },
            move |result| Message::StoreLoaded { generation, result },
        )
    }

    /// The fetch for the text now in the box, after the Raycast store's pause.
    fn store_query_task(&mut self) -> Task<Message> {
        let Page::Store(page) = &self.page else {
            return Task::none();
        };
        let generation = page.generation;
        match page.debounce() {
            Some(delay) => Task::perform(
                async move {
                    tokio::time::sleep(delay).await;
                    generation
                },
                Message::StoreSearchDue,
            ),
            None => self.store_fetch(generation),
        }
    }

    /// Fetches the images the store page on screen shows and has not asked
    /// for.
    fn store_images(&mut self) -> Task<Message> {
        let dark = self.store_prefers_dark();
        let wanted = match &mut self.page {
            Page::Store(page) => page.wanted_images(dark),
            Page::StoreDetail(page) => page.wanted_images(dark),
            _ => Vec::new(),
        };
        if wanted.is_empty() {
            Task::none()
        } else {
            crate::remote_image::fetch_tasks(wanted)
        }
    }

    /// Asks for the selected row's detail page.
    fn open_selected_store_detail(&mut self) -> Task<Message> {
        let Page::Store(page) = &mut self.page else {
            return Task::none();
        };
        let Some(row) = page.selected_row() else {
            return Task::none();
        };
        let Some(backend) = self.backend.clone() else {
            page.notice = Some(NEEDS_ENGINE.to_owned());
            return Task::none();
        };
        let (store, owner, name) = (page.store, row.owner.clone(), row.name.clone());
        page.loading = true;
        page.notice = None;
        self.panel = None;
        Task::perform(
            async move { backend.store_extension(store, owner, name).await },
            Message::StoreDetailLoaded,
        )
    }

    /// Installs, or updates, the extension on the detail page.
    fn store_install(&mut self) -> Task<Message> {
        let Page::StoreDetail(page) = &mut self.page else {
            return Task::none();
        };
        if page.busy.is_some() {
            return Task::none();
        }
        let Some(backend) = self.backend.clone() else {
            page.notice = Some(NEEDS_ENGINE.to_owned());
            return Task::none();
        };
        let row = &page.detail.row;
        let (store, owner, name) = (page.store, row.owner.clone(), row.name.clone());
        page.busy = Some(store_page::DOWNLOADING.to_owned());
        page.notice = None;
        Task::perform(
            async move { backend.store_install(store, owner, name).await },
            Message::StoreInstalled,
        )
    }

    /// Uninstalls what the question on screen asked about.
    fn store_uninstall(&mut self, id: String) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            self.store_notice(NEEDS_ENGINE.to_owned());
            return Task::none();
        };
        match &mut self.page {
            Page::Store(page) => page.confirm = None,
            Page::StoreDetail(page) => {
                page.confirm = false;
                page.busy = Some("Uninstalling extension...".to_owned());
            }
            _ => {}
        }
        Task::perform(
            {
                let id = id.clone();
                async move { backend.store_uninstall(id).await }
            },
            move |result| Message::StoreUninstalled {
                id: id.clone(),
                result,
            },
        )
    }

    /// Opens a link from the detail page.
    fn store_open(&mut self, url: String) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            self.store_notice(NEEDS_ENGINE.to_owned());
            return Task::none();
        };
        Task::perform(
            async move { backend.open_url(url).await },
            Message::StoreUrlOpened,
        )
    }

    /// Says `message` under whichever store page is showing.
    fn store_notice(&mut self, message: String) {
        match &mut self.page {
            Page::Store(page) => page.notice = Some(message),
            Page::StoreDetail(page) => page.notice = Some(message),
            _ => self.error = Some(message),
        }
    }

    /// The action panel over a store row or a detail page.
    pub(super) fn open_store_panel(&mut self) -> Option<Task<Message>> {
        let actions = match &self.page {
            Page::Store(page) => {
                let row = page.selected_row()?;
                // The C++ offers "Uninstall Extension" on every row, where it
                // fails for one that is not installed; offered only where it
                // can work.
                let mut sections = vec![PanelSection {
                    name: String::new(),
                    actions: vec![
                        Action::new("Show details")
                            .with_id(actions::DETAILS)
                            .with_shortcut("enter"),
                    ],
                }];
                if row.installed {
                    sections.push(PanelSection {
                        name: String::new(),
                        actions: vec![
                            Action::new("Uninstall Extension").with_id(actions::UNINSTALL),
                        ],
                    });
                }
                sections
            }
            Page::StoreDetail(page) => vec![PanelSection {
                name: String::new(),
                actions: store_page::detail_actions(page)
                    .into_iter()
                    .map(|(id, title, shortcut)| {
                        let action = Action::new(title).with_id(id);
                        match shortcut {
                            Some(shortcut) => action.with_shortcut(shortcut),
                            None => action,
                        }
                    })
                    .collect(),
            }],
            _ => return None,
        };
        self.panel = Some(PanelState::new(actions));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a store panel action, if `id` is one.
    pub(super) fn store_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Store(_) | Page::StoreDetail(_)) {
            return None;
        }
        self.panel = None;
        let task = match (id, &mut self.page) {
            (actions::DETAILS, Page::Store(_)) => self.open_selected_store_detail(),
            (actions::UNINSTALL, Page::Store(page)) => {
                if let Some(row) = page.selected_row() {
                    page.confirm = Some(Confirm {
                        id: row.id.clone(),
                        title: row.title.clone(),
                    });
                }
                Task::none()
            }
            (actions::INSTALL, Page::StoreDetail(_)) => self.store_install(),
            (actions::UNINSTALL, Page::StoreDetail(page)) => {
                page.confirm = true;
                Task::none()
            }
            (actions::README, Page::StoreDetail(page)) => {
                let url = page.detail.readme_url.clone()?;
                self.store_open(url)
            }
            (actions::SOURCE, Page::StoreDetail(page)) => {
                let url = page.detail.source_url.clone()?;
                self.store_open(url)
            }
            (actions::WEBSITE, Page::StoreDetail(page)) => {
                let url = page.detail.store_url.clone()?;
                self.store_open(url)
            }
            (actions::REPORT, Page::StoreDetail(_)) => {
                self.store_open(compass_core::store_listing::REPORT_ISSUE_URL.to_owned())
            }
            _ => return None,
        };
        Some(task)
    }

    /// The list's keys.
    pub(super) fn store_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Store(page) = &mut self.page else {
            return Task::none();
        };
        if let Some(confirm) = page.confirm.clone() {
            return match key.as_ref() {
                Key::Named(Named::Enter) => self.store_uninstall(confirm.id),
                Key::Named(Named::Escape) => {
                    page.confirm = None;
                    Task::none()
                }
                _ => Task::none(),
            };
        }
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.open_selected_store_detail(),
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

    /// The detail page's keys: Enter runs the first action, Escape goes back
    /// to the list as it was.
    pub(super) fn store_detail_key(&mut self, key: &Key) -> Task<Message> {
        let Page::StoreDetail(page) = &mut self.page else {
            return Task::none();
        };
        if page.confirm {
            return match key.as_ref() {
                Key::Named(Named::Enter) => {
                    let id = page.detail.row.id.clone();
                    self.store_uninstall(id)
                }
                Key::Named(Named::Escape) => {
                    page.confirm = false;
                    Task::none()
                }
                _ => Task::none(),
            };
        }
        if let Some(scroll) = crate::scroll::reading_key(key.as_ref()) {
            return scroll;
        }
        match key.as_ref() {
            Key::Named(Named::Escape) => {
                let store = page.store;
                self.open_store(store)
            }
            Key::Named(Named::Enter) => {
                let Some((id, _, _)) = store_page::detail_actions(page).into_iter().next() else {
                    return Task::none();
                };
                self.store_panel_action(id).unwrap_or_else(Task::none)
            }
            _ => Task::none(),
        }
    }

    /// Records an install or an uninstall everywhere it shows: the page on
    /// screen, the list parked under a detail page, and root search.
    pub(super) fn store_changed(&mut self, id: &str, installed: bool) {
        if let Page::Store(page) = &mut self.page {
            page.mark(id, installed);
        }
        if let Page::StoreDetail(page) = &mut self.page
            && page.detail.row.id == id
        {
            page.detail.row.installed = installed;
            page.detail.row.update_available = false;
        }
        if let Some(parked) = &mut self.parked_store {
            parked.mark(id, installed);
        }
        self.app_index.rescan_extensions();
    }

    /// Handles the store messages.
    pub(super) fn store_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::StoreLoaded { generation, result } => {
                if let Page::Store(page) = &mut self.page {
                    page.apply(generation, result);
                }
                Task::batch([self.store_images(), crate::scroll::reveal_root_selection()])
            }
            Message::StoreQueryChanged(query) => {
                if let Page::Store(page) = &mut self.page {
                    page.set_query(query);
                }
                self.store_query_task()
            }
            Message::StoreSearchDue(generation) => self.store_fetch(generation),
            Message::StoreSelected(position) => {
                if let Page::Store(page) = &mut self.page
                    && position < page.rows.len()
                {
                    page.selected = position;
                    return self.open_selected_store_detail();
                }
                Task::none()
            }
            Message::StoreDetailLoaded(Ok(detail)) => {
                let Page::Store(page) = &mut self.page else {
                    return Task::none();
                };
                page.loading = false;
                let store = page.store;
                if let Page::Store(page) = std::mem::replace(
                    &mut self.page,
                    Page::StoreDetail(Box::new(StoreDetailPage::new(store, detail))),
                ) {
                    self.parked_store = Some(page);
                }
                Task::batch([self.store_images(), focus_search()])
            }
            Message::StoreDetailLoaded(Err(reason)) => {
                if let Page::Store(page) = &mut self.page {
                    page.loading = false;
                    // A deeplink's page has no list under it to keep.
                    if page.status == Status::Loading {
                        page.status = Status::Failed(reason);
                    } else {
                        page.notice = Some(reason);
                    }
                }
                Task::none()
            }
            Message::StoreConfirmAnswered(confirmed) => {
                let id = match &mut self.page {
                    Page::Store(page) => {
                        let confirm = page.confirm.take();
                        confirm.map(|confirm| confirm.id)
                    }
                    Page::StoreDetail(page) if page.confirm => {
                        page.confirm = false;
                        Some(page.detail.row.id.clone())
                    }
                    _ => None,
                };
                match id {
                    Some(id) if confirmed => self.store_uninstall(id),
                    _ => focus_search(),
                }
            }
            Message::StoreInstalled(result) => {
                if let Page::StoreDetail(page) = &mut self.page {
                    page.busy = None;
                }
                match result {
                    Ok((id, _title)) => {
                        self.store_changed(&id, true);
                        self.store_notice(store_page::INSTALLED.to_owned());
                    }
                    Err(reason) => self.store_notice(reason),
                }
                Task::none()
            }
            Message::StoreUninstalled { id, result } => {
                if let Page::StoreDetail(page) = &mut self.page {
                    page.busy = None;
                }
                match result {
                    Ok(()) => {
                        self.store_changed(&id, false);
                        self.store_notice(store_page::UNINSTALLED.to_owned());
                    }
                    Err(reason) => {
                        self.store_notice(format!("Failed to uninstall extension: {reason}"));
                    }
                }
                Task::none()
            }
            Message::StoreUrlOpened(Ok(())) => self.conceal(),
            Message::StoreUrlOpened(Err(reason)) => {
                self.store_notice(reason);
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// A store image arrived; answers whether a store page took it.
    pub(super) fn store_image_arrived(
        &mut self,
        url: &str,
        result: &Result<std::path::PathBuf, String>,
    ) -> bool {
        let images = match &mut self.page {
            Page::Store(page) => &mut page.images,
            Page::StoreDetail(page) => &mut page.images,
            _ => return false,
        };
        images.arrived(url.to_owned(), result.clone());
        true
    }

    /// A banner over a page: work in progress, or what the last action
    /// said. Over, not under: the rows shrink to fit, so a full list pushed
    /// a banner below them out of sight, and with it an install's failure.
    fn store_banner(
        &self,
        busy: Option<&str>,
        notice: Option<&str>,
    ) -> Option<Element<'_, Message>> {
        let palette = self.palette();
        let line = busy.or(notice)?;
        Some(
            container(
                text(line.to_owned())
                    .font(self.font())
                    .size(13)
                    .color(palette.muted.to_iced()),
            )
            .padding(Padding::new(8.0).left(14).right(14))
            .into(),
        )
    }

    /// The list's body: the heading, then each extension with its icon,
    /// description and accessory.
    pub(super) fn store_body<'a>(&'a self, page: &'a StorePage) -> Element<'a, Message> {
        match &page.status {
            Status::Loading => return self.notice("Loading extensions…"),
            Status::Failed(reason) => return self.notice(reason),
            Status::Ready => {}
        }
        let palette = self.palette();
        let dark = self.store_prefers_dark();
        let heading = if page.loading {
            format!("{} …", page.heading)
        } else {
            page.heading.clone()
        };
        let mut list = column![
            container(
                text(heading)
                    .font(self.font())
                    .size(12)
                    .color(palette.muted.to_iced())
            )
            .padding(Padding::new(4.0).left(10).right(10))
        ]
        .spacing(f32::from(self.geometry.row_spacing));
        if page.rows.is_empty() {
            list = list.push(self.notice("No extensions match"));
        }
        for (position, entry) in page.rows.iter().enumerate() {
            let selected = position == page.selected;
            let icon = store_page::icon_url(entry, dark)
                .and_then(|url| page.images.art.get(url))
                .map_or_else(
                    || self.initial_badge(&entry.title, selected),
                    |art| self.extension_icon(art, selected),
                );
            let item = self.list_row_parts(
                icon,
                entry.title.clone(),
                self.subtitles.then(|| entry.description.clone()),
                Some(self.store_accessory(entry, &page.images, selected)),
                selected,
            );
            let item: Element<Message> = mouse_area(item)
                .on_press(Message::StoreSelected(position))
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
        let content: Element<'a, Message> = match self.store_banner(None, page.notice.as_deref()) {
            Some(banner) => column![banner, rows].into(),
            None => rows.into(),
        };
        match &page.confirm {
            Some(confirm) => self.store_dialog(content, &confirm.title),
            None => content,
        }
    }

    /// The detail page's body: the icon and navigation title, the Markdown,
    /// then the screenshots that have arrived.
    pub(super) fn store_detail_body<'a>(
        &'a self,
        page: &'a StoreDetailPage,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let dark = self.store_prefers_dark();
        let icon = store_page::icon_url(&page.detail.row, dark)
            .and_then(|url| page.images.art.get(url))
            .map_or_else(
                || self.initial_badge(&page.detail.row.title, false),
                |art| self.extension_icon(art, false),
            );
        let mut status = store_page::accessory(&page.detail.row);
        if status.is_empty() {
            status = page.detail.row.author_name.clone();
        }
        let header = row![
            icon,
            column![
                text(page.title.clone())
                    .font(self.font())
                    .size(12)
                    .color(palette.muted.to_iced()),
                text(status)
                    .font(self.font())
                    .size(12)
                    .color(palette.muted.to_iced()),
            ]
            .spacing(2)
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        let markdown = iced::widget::markdown::view_with(
            &page.markdown,
            self.markdown_settings(),
            &StoreMarkdown {
                images: &page.images.art,
            },
        );
        let mut body = column![header, markdown].spacing(12);
        for url in &page.detail.screenshots {
            if let Some(crate::extension_page::RowIcon::Art { art, .. }) = page.images.art.get(url)
            {
                let shot: Element<Message> = match art {
                    crate::icons::IconArt::Raster(path) => {
                        image(path.clone()).width(Length::Fill).into()
                    }
                    crate::icons::IconArt::Vector(path) => {
                        iced::widget::svg(path.clone()).width(Length::Fill).into()
                    }
                };
                body = body.push(shot);
            }
        }
        let content = scrollable(container(body).padding(Padding::new(14.0)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        let content: Element<'a, Message> =
            match self.store_banner(page.busy.as_deref(), page.notice.as_deref()) {
                Some(banner) => column![banner, content].into(),
                None => content.into(),
            };
        if page.confirm {
            self.store_dialog(content, &page.detail.row.title)
        } else {
            content
        }
    }
}
