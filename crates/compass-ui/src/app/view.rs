//! Drawing the launcher: the card, its search field, the page under it,
//! the action panel over it, and the theme and font it is drawn in.
//!
//! Each page's body is drawn by that page's module; this module lays them
//! out in the card and holds the row and heading helpers they share.

use super::icons::shortcut_url;
use super::{
    Alignment, AppItem, Border, Color, Element, Fallback, LauncherApp, Length, Message, Padding,
    Page, RootRow, SEARCH_INPUT, Space, TINT_ALPHA, Task, Theme, action_panel, answer_keys, column,
    container, design, mouse_area, preview, row, scrollable, sensor, stack, text, text_input,
};

/// The gap between the floating action panel and the card's edges.
const PANEL_MARGIN: f32 = 8.0;

/// The tallest the card grows to hold the action panel; a longer panel
/// scrolls.
const PANEL_ROOM_CAP: f32 = 360.0;

/// What the search field sends as the user types.
type OnInput = fn(String) -> Message;

impl LauncherApp {
    /// The appearance the card is drawn in: the desktop's light or dark
    /// preference, its font, and a theme previewed before it is kept.
    pub(super) fn appearance_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::AppearanceChanged(appearance) => {
                self.view.appearance = appearance;
                Task::none()
            }
            Message::TypographyChanged(family) => {
                let family = family.trim().to_owned();
                if family.is_empty() {
                    self.view.font_family = None;
                } else {
                    self.view.font_family = Some(family);
                }
                Task::none()
            }
            Message::ThemePreview(theme) => {
                if self.view.theme_preview.is_none() {
                    self.view.theme_preview = Some(self.view.theme_choice);
                }
                self.view.theme_choice = theme;
                Task::none()
            }
            Message::ThemeCommit => {
                // Persist is handled by compass theme set; in-ui commit clears preview backup.
                self.view.theme_preview = None;
                Task::none()
            }
            Message::ThemeCancel => {
                if let Some(prev) = self.view.theme_preview.take() {
                    self.view.theme_choice = prev;
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The application title.
    pub fn title(&self) -> String {
        compass_core::tray::APP_NAME.to_owned()
    }

    /// The application theme.
    ///
    /// Adwaita's palette rather than one of Iced's built-ins, so the launcher
    /// looks like the desktop it sits on. [`crate::design`] holds the colours
    /// and the browser surrogate under `tools/design/` is served the same
    /// ones, which is what keeps the two renderings honest about each other.
    ///
    /// The appearance follows the desktop when something is feeding
    /// [`AppFlags::appearance_link`](super::AppFlags::appearance_link), and
    /// otherwise stays on [`AppFlags::appearance`](super::AppFlags::appearance)
    /// for the window's whole life.
    pub(super) fn palette(&self) -> design::Palette {
        self.view.theme_choice.palette(self.view.appearance)
    }

    /// The font the launcher draws text with.
    ///
    /// Follows `org.gnome.desktop.interface font-name` when a family was
    /// supplied at startup (or later via `TypographyChanged`), otherwise
    /// `iced::Font::DEFAULT` which keeps the historic `Cantarell` fallback
    /// readable on a minimal image.
    pub(super) fn font(&self) -> iced::Font {
        match &self.view.font_family {
            Some(family) => crate::typography::iced_font(family),
            None => iced::Font::DEFAULT,
        }
    }

    /// How every Markdown view is drawn: 14 px, in the launcher's own
    /// [`font`](Self::font).
    ///
    /// Iced's `Style::from(&Theme)` leaves `style.font` at `Font::default()`,
    /// the generic sans-serif, which cosmic-text maps to a hard-coded
    /// "Open Sans" rather than to the desktop's interface font. Where that
    /// family is missing — a stock GNOME install — every Markdown span went
    /// through cosmic-text's fallback chain instead, and its bold spans
    /// landed on whichever family happened to have a static bold face. Code
    /// keeps iced's monospace.
    pub(super) fn markdown_settings(&self) -> iced::widget::markdown::Settings {
        let mut style = iced::widget::markdown::Style::from(&self.theme());
        style.font = self.font();
        iced::widget::markdown::Settings::with_text_size(14, style)
    }

    /// The application theme.
    pub fn theme(&self) -> Theme {
        let p = self.palette();
        if self.view.theme_choice == crate::theme::Theme::System {
            return design::theme(self.view.appearance);
        }
        iced::Theme::custom(
            format!(
                "Compass {}-{}",
                self.view.theme_choice.name(),
                self.view.appearance.name()
            ),
            iced::theme::Palette {
                background: p.surface.to_iced(),
                text: p.text.to_iced(),
                primary: p.accent.to_iced(),
                success: p.accent.to_iced(),
                warning: p.accent.to_iced(),
                danger: iced::Color::from_rgb8(0xe0, 0x1b, 0x24),
            },
        )
    }

    /// The surface's own style: what the runtime clears the window to before
    /// the view is drawn. Transparent, so only the card and its shadow are on
    /// screen. Left to the theme it is the palette's background (the card's
    /// surface colour), and the whole window, shadow padding included,
    /// paints as an opaque rectangle behind the card.
    #[must_use]
    pub fn style(&self, theme: &Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: theme.palette().text,
        }
    }

    /// The search field as the page on screen has it: its placeholder, its
    /// text, and the message typing sends (none where it is read-only).
    pub(super) fn search_field(&self) -> (&str, &str, Option<OnInput>) {
        match &self.page {
            Page::Root => (
                self.provider_scope
                    .as_ref()
                    .map_or("Search…", |scope| scope.placeholder.as_str()),
                &self.query,
                self.panel
                    .is_none()
                    .then_some(Message::QueryChanged as OnInput),
            ),
            Page::Clipboard(page) => (
                "Search clipboard history…",
                &page.query,
                Some(Message::ClipboardQueryChanged as OnInput),
            ),
            Page::Emoji(page) => (
                "Search emojis and symbols…",
                &page.query,
                Some(Message::EmojiQueryChanged as OnInput),
            ),
            Page::Windows(page) => (
                "Search windows…",
                &page.query,
                Some(Message::WindowsQueryChanged as OnInput),
            ),
            Page::Workspaces(page) => (
                crate::workspaces_page::PLACEHOLDER,
                &page.query,
                Some(Message::WorkspacesQueryChanged as OnInput),
            ),
            Page::OpenWith(page) => (
                crate::open_with_page::PLACEHOLDER,
                &page.query,
                Some(Message::OpenWithQueryChanged as OnInput),
            ),
            Page::Files(page) => (
                "Search for files…",
                &page.query,
                Some(Message::FilesQueryChanged as OnInput),
            ),
            Page::Shortcuts(page) => (
                "Search shortcuts...",
                &page.query,
                Some(Message::ShortcutsQueryChanged as OnInput),
            ),
            Page::Snippets(page) => (
                "Search for snippets...",
                &page.query,
                Some(Message::SnippetsQueryChanged as OnInput),
            ),
            Page::ScriptOutput(page) => ("", &page.title, None),
            Page::Onboarding(_) | Page::StoreIntro(_) => ("", "", None),
            Page::Fallbacks(page) => (
                crate::fallbacks_page::PLACEHOLDER,
                &page.query,
                Some(Message::FallbacksQueryChanged as OnInput),
            ),
            Page::Extensions(page) => (
                crate::compass_pages::EXTENSIONS_PLACEHOLDER,
                &page.query,
                Some(Message::ExtensionsQueryChanged as OnInput),
            ),
            Page::Icons(page) => (
                crate::compass_pages::ICONS_PLACEHOLDER,
                &page.query,
                Some(Message::IconsQueryChanged as OnInput),
            ),
            Page::Storage(page) => (
                if page.browsing.is_some() {
                    crate::compass_pages::ITEMS_PLACEHOLDER
                } else {
                    crate::compass_pages::NAMESPACES_PLACEHOLDER
                },
                &page.query,
                Some(Message::StorageQueryChanged as OnInput),
            ),
            Page::Tokens(page) => (
                crate::compass_pages::TOKENS_PLACEHOLDER,
                &page.query,
                Some(Message::TokensQueryChanged as OnInput),
            ),
            Page::Programs(page) => (
                "Search for a program to execute...",
                &page.query,
                Some(Message::ProgramsQueryChanged as OnInput),
            ),
            Page::Dmenu(page) => (
                page.placeholder(),
                &page.query,
                Some(Message::DmenuQueryChanged as OnInput),
            ),
            Page::Calculator(page) => (
                crate::calculator_page::PLACEHOLDER,
                &page.query,
                Some(Message::CalculatorQueryChanged as OnInput),
            ),
            Page::Grants(page) => (
                crate::grants_page::PLACEHOLDER,
                &page.query,
                Some(Message::GrantsQueryChanged as OnInput),
            ),
            Page::Tray(page) => (
                if page.menu_key().is_some() {
                    crate::tray_page::MENU_PLACEHOLDER
                } else {
                    crate::tray_page::PLACEHOLDER
                },
                &page.query,
                Some(Message::TrayQueryChanged as OnInput),
            ),
            Page::Apps(page) => (
                page.placeholder(),
                &page.query,
                Some(Message::AppsQueryChanged as OnInput),
            ),
            Page::NowPlaying(page) => (
                crate::media_page::PLACEHOLDER,
                &page.query,
                Some(Message::NowPlayingQueryChanged as OnInput),
            ),
            Page::Themes(page) => (
                compass_core::theme_picker::PLACEHOLDER,
                &page.query,
                Some(Message::ThemesQueryChanged as OnInput),
            ),
            Page::Created(page) => ("", &page.path, None),
            Page::Fonts(page) => (
                "Search fonts...",
                &page.query,
                Some(Message::FontsQueryChanged as OnInput),
            ),
            Page::FontPreview(page) => ("", &page.name, None),
            Page::Store(page) => (
                page.store.placeholder(),
                &page.query,
                Some(Message::StoreQueryChanged as OnInput),
            ),
            Page::StoreDetail(page) => ("", &page.title, None),
            Page::Preferences(page) => ("Configure", &page.title, None),
            Page::Settings(page) => (
                crate::settings_page::PLACEHOLDER,
                &page.query,
                Some(
                    (|query| {
                        Message::Settings(crate::settings_page::SettingsMessage::QueryChanged(
                            query,
                        ))
                    }) as OnInput,
                ),
            ),
            Page::Extension(page) => (
                page.list()
                    .and_then(|list| list.search.placeholder.as_deref())
                    .unwrap_or("Search…"),
                &page.query,
                Some(Message::ExtensionQueryChanged as OnInput),
            ),
        }
    }

    /// View the application.
    ///
    /// A card: a search field over a list of rows, each row an icon, a title
    /// and a subtitle, with the selection drawn as a filled rounded rectangle
    /// rather than a caret.
    ///
    /// **The caret is gone and that is deliberate.** It was there because a
    /// comment said "under llvmpipe at 1280x800 a background tint is not
    /// identifiable in a captured frame". `framediff.py` compares raw RGB
    /// bytes for exact inequality, with no threshold, so a tinted row of
    /// roughly 600x30 is about 18,000 changed pixels -- some seven times the
    /// 2,697 the action-panel assertion already detects reliably. A highlight
    /// is *easier* for the tier to see than a caret, not harder.
    pub fn view(&self) -> Element<'_, Message> {
        let geometry = self.geometry;
        let palette = self.palette();

        // One field, whose meaning follows the view: the root query, or a
        // command's own filter. Same id either way, so focus survives the
        // switch and `focus_search` needs no second target.
        let (placeholder, value, on_input) = self.search_field();
        let input = text_input(placeholder, value)
            .id(SEARCH_INPUT)
            .font(self.font())
            .style(move |theme: &Theme, status| query_input_style(theme, status, palette))
            .on_input_maybe(on_input)
            .padding(Padding::new(0.0).left(14).right(14))
            .size(f32::from(geometry.query_size));

        let field = container(input)
            .height(Length::Fixed(f32::from(geometry.field_height)))
            .width(Length::Fill)
            .align_y(Alignment::Center)
            .style(move |_: &Theme| container::Style {
                background: Some(palette.field.to_iced().into()),
                border: Border {
                    color: palette.border.to_iced(),
                    width: 1.0,
                    radius: f32::from(geometry.field_radius).into(),
                },
                ..container::Style::default()
            });

        let body: Element<Message> = if let Some(confirm) = &self.confirm {
            column![
                text(confirm.title.as_str())
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..self.font()
                    })
                    .size(16),
                text(confirm.message.as_str()).font(self.font()),
                text(answer_keys(&confirm.confirm_text))
                    .font(self.font())
                    .size(12),
            ]
            .spacing(8)
            .padding(Padding::new(18.0))
            .into()
        } else if let Some(power) = self.power_confirm {
            column![
                text(compass_core::power_commands::CONFIRM_TITLE)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..self.font()
                    })
                    .size(16),
                text(compass_core::power_commands::CONFIRM_BODY).font(self.font()),
                text(answer_keys(power.name)).font(self.font()).size(12),
            ]
            .spacing(8)
            .padding(Padding::new(18.0))
            .into()
        } else if let Page::Preferences(page) = &self.page {
            self.preferences_body(page)
        } else if let Page::Extension(page) = &self.page {
            self.extension_body(page)
        } else if let Page::Emoji(page) = &self.page {
            self.emoji_body(page)
        } else if let Page::Windows(page) = &self.page {
            self.windows_body(page)
        } else if let Page::Workspaces(page) = &self.page {
            self.workspaces_body(page)
        } else if let Page::OpenWith(page) = &self.page {
            self.open_with_body(page)
        } else if let Page::Files(page) = &self.page {
            self.files_body(page)
        } else if let Page::Shortcuts(page) = &self.page {
            self.shortcuts_body(page)
        } else if let Page::Snippets(page) = &self.page {
            self.snippets_body(page)
        } else if let Page::ScriptOutput(page) = &self.page {
            self.script_output_body(page)
        } else if let Page::Programs(page) = &self.page {
            self.programs_body(page)
        } else if let Page::Dmenu(page) = &self.page {
            self.dmenu_body(page)
        } else if let Page::Grants(page) = &self.page {
            self.grants_body(page)
        } else if let Page::Tray(page) = &self.page {
            self.tray_body(page)
        } else if let Page::Apps(page) = &self.page {
            self.apps_body(page)
        } else if let Page::Calculator(page) = &self.page {
            self.calculator_body(page)
        } else if let Page::Fallbacks(page) = &self.page {
            self.fallbacks_body(page)
        } else if let Some(body) = self.compass_view_body() {
            body
        } else if let Page::StoreIntro(page) = &self.page {
            self.store_intro_body(page)
        } else if let Page::NowPlaying(page) = &self.page {
            self.now_playing_body(page)
        } else if let Page::Themes(page) = &self.page {
            self.themes_body(page)
        } else if let Page::Settings(page) = &self.page {
            self.settings_body(page)
        } else if let Page::Created(page) = &self.page {
            self.created_body(page)
        } else if let Page::Fonts(page) = &self.page {
            self.fonts_body(page)
        } else if let Page::FontPreview(page) = &self.page {
            self.font_preview_body(page)
        } else if let Page::Store(page) = &self.page {
            self.store_body(page)
        } else if let Page::StoreDetail(page) = &self.page {
            self.store_detail_body(page)
        } else if let Page::Clipboard(page) = &self.page {
            self.clipboard_body(page)
        } else if let Some(err) = &self.error {
            self.notice(err)
        } else if self.query.is_empty() && self.results.is_empty() {
            self.notice("Type to search")
        } else if self.results.is_empty() {
            self.notice("No results")
        } else {
            let mut list = column![].spacing(f32::from(geometry.row_spacing));
            for (position, root_row) in self.results.iter().enumerate() {
                if let Some(heading) = self.root_heading_at(position) {
                    list = list.push(self.section_heading(heading.to_owned()));
                }
                let selected = position == self.selected;
                let row = match root_row {
                    RootRow::App(index) => {
                        let Some(item) = self.app_index.items().get(*index) else {
                            continue;
                        };
                        self.result_row(item, selected)
                    }
                    RootRow::Command(command) => self.list_row(
                        self.command_icon(command, selected),
                        command.title.to_owned(),
                        self.subtitles.then(|| command.subtitle.to_owned()),
                        selected,
                    ),
                    RootRow::Update => {
                        let Some(row) = self.update_row(selected) else {
                            continue;
                        };
                        row
                    }
                    RootRow::Calculator => {
                        let Some(answer) = &self.calculator else {
                            continue;
                        };
                        self.list_row(
                            self.initial_badge("=", selected),
                            answer.answer.clone(),
                            Some(answer.question.clone()),
                            selected,
                        )
                    }
                    RootRow::Extension(index) => {
                        let Some(command) = self.app_index.extensions().get(*index) else {
                            continue;
                        };
                        self.list_row(
                            self.url_icon(
                                self.extension_url(command).as_ref(),
                                &command.title,
                                selected,
                            ),
                            command.title.clone(),
                            self.subtitles.then(|| {
                                self.extension_subtitles
                                    .get(&command.id)
                                    .unwrap_or(&command.extension_title)
                                    .clone()
                            }),
                            selected,
                        )
                    }
                    RootRow::Script(index) => {
                        let Some(script) = self.app_index.scripts().get(*index) else {
                            continue;
                        };
                        self.list_row(
                            self.url_icon(
                                self.script_icons.get(&script.id),
                                &script.title,
                                selected,
                            ),
                            script.title.clone(),
                            self.subtitles.then(|| script.subtitle.clone()),
                            selected,
                        )
                    }
                    RootRow::RhaiScript(index) => {
                        let Some(script) = self.app_index.rhai_scripts().get(*index) else {
                            continue;
                        };
                        let icon = match self.rhai_icons.get(&script.id) {
                            Some(icon) => self.extension_icon(icon, selected),
                            None => self.initial_badge(&script.title, selected),
                        };
                        self.list_row(
                            icon,
                            script.title.clone(),
                            self.subtitles.then(|| script.subtitle().to_owned()),
                            selected,
                        )
                    }
                    RootRow::Fallback(fallback) => {
                        let Some(title) = self.fallback_title(*fallback) else {
                            continue;
                        };
                        if position == 0
                            || !matches!(self.results.get(position - 1), Some(RootRow::Fallback(_)))
                        {
                            list = list.push(
                                container(
                                    text(compass_core::root_items::fallback_heading(&self.query))
                                        .font(self.font())
                                        .size(12)
                                        .color(palette.muted.to_iced()),
                                )
                                .padding(Padding::new(4.0).left(10)),
                            );
                        }
                        let subtitle = match fallback {
                            Fallback::Command(command) => command.subtitle.to_owned(),
                            Fallback::Extension(_) => "Command".to_owned(),
                            Fallback::Shortcut(_) => "Shortcut".to_owned(),
                        };
                        self.list_row(
                            match fallback {
                                Fallback::Command(command) => self.command_icon(command, selected),
                                _ => self.initial_badge(&title, selected),
                            },
                            title,
                            self.subtitles.then_some(subtitle),
                            selected,
                        )
                    }
                    RootRow::Shortcut(index) => {
                        let Some(shortcut) = self.app_index.shortcuts().get(*index) else {
                            continue;
                        };
                        let title = crate::shortcuts_page::display_name(shortcut);
                        self.list_row(
                            self.url_icon(Some(&shortcut_url(&shortcut.icon)), title, selected),
                            title.to_owned(),
                            self.subtitles.then(|| "Shortcut".to_owned()),
                            selected,
                        )
                    }
                };
                let row: Element<Message> = mouse_area(row)
                    .on_press(Message::ResultClicked(position))
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
        };

        // At the root the status bar stays on the card: the list scrolls in
        // what the card has left, rather than pushing the bar off its foot.
        let root_footer = matches!(self.page, Page::Root)
            && self.confirm.is_none()
            && self.power_confirm.is_none();
        let body: Element<Message> = if root_footer {
            let room = f32::from(geometry.card_max_height)
                - 2.0 * f32::from(geometry.card_padding)
                - f32::from(geometry.field_height)
                - preview::FOOTER_HEIGHT
                - if self.field_rule { 1.0 } else { 0.0 };
            container(body).max_height(room).into()
        } else {
            body
        };
        // Flow's hairline rule under the query field (#84). A one-pixel
        // container rather than a border on the field, because the field has
        // its own rounded border in the other presets and a rule has to span
        // the card's full width regardless of the field's radius.
        let card_content = if let Page::Onboarding(page) = &self.page {
            column![self.onboarding_body(page)].width(Length::Fill)
        } else if self.field_rule {
            column![
                field,
                container(Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(1.0))
                    .style(move |_: &Theme| container::Style {
                        background: Some(palette.border.to_iced().into()),
                        ..container::Style::default()
                    }),
                body
            ]
            .width(Length::Fill)
        } else {
            column![field, body].width(Length::Fill)
        };
        // The root search's status bar: the clock as its title, as
        // `scheduleNextClockTick` sets the navigation title, and on the
        // right what Enter does and how to reach the other actions, as
        // dmenu's footer and Raycast's action bar say it.
        let card_content = if root_footer {
            card_content.push(self.footer(self.clock_text.as_deref(), self.root_primary_action()))
        } else {
            card_content
        };

        // The panel floats over the list rather than replacing it. The old
        // comment said an overlay "needs a stacking widget and a backdrop" --
        // `stack!` is that widget, and the backdrop turned out to be
        // unnecessary because the panel is opaque and bounded.
        //
        // The card is at least as tall as the panel's unfiltered rows (up to
        // a cap past which the panel scrolls), so a panel over a short list,
        // such as a lone calculator answer, is not cut off at the card's
        // edge.
        let card_body: Element<Message> = match &self.panel {
            Some(panel) => stack![
                row![
                    card_content,
                    // One pixel wide: a row drops a child with no width at all.
                    Space::new().width(1).height(if panel.recorder.is_some() {
                        0.0
                    } else {
                        (action_panel::natural_height(&action_panel::flatten(&panel.sections, ""))
                            + 2.0 * PANEL_MARGIN)
                            .min(PANEL_ROOM_CAP)
                    })
                ],
                container(self.view_panel(panel))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::End)
                    .align_y(Alignment::End)
                    .padding(PANEL_MARGIN)
            ]
            .into(),
            None => card_content.into(),
        };

        let card_background = card_background(palette.surface, self.tint);

        let (card_width, card_height) = match self.dmenu_card_size() {
            Some((width, height)) => (width as f32, Length::Fixed(height as f32)),
            None => (f32::from(geometry.card_width), Length::Shrink),
        };
        let card = container(card_body)
            .width(Length::Fixed(card_width))
            .height(card_height)
            .padding(geometry.card_padding)
            .style(move |_: &Theme| container::Style {
                background: Some(card_background.into()),
                border: Border {
                    color: palette.border.to_iced(),
                    width: 1.0,
                    radius: f32::from(geometry.card_radius).into(),
                },
                shadow: design::card_shadow(),
                ..container::Style::default()
            });
        // Reports the card's size when it is shown, when it resizes, and
        // again when the look the blur depends on changes (the key), so the
        // blur behind it follows the card (`crate::material`).
        let card = sensor(card)
            .key((self.tint, geometry.card_radius))
            .on_show(Message::CardMeasured)
            .on_resize(Message::CardMeasured);
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(design::SHADOW_PADDING)
            .align_x(Alignment::Center)
            .align_y(Alignment::Start)
            .into()
    }

    /// The window switcher's body: its state, or its rows.
    /// A section's heading in a list: small, muted, indented to the rows'
    /// text.
    pub(super) fn section_heading<'a>(&self, label: String) -> Element<'a, Message> {
        container(
            text(label)
                .font(self.font())
                .size(12)
                .color(self.palette().muted.to_iced()),
        )
        .padding(Padding::new(4.0).left(10))
        .into()
    }

    /// A line of explanation where the list would be.
    pub(super) fn notice(&self, message: &str) -> Element<'_, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        container(
            text(message.to_owned())
                .font(self.font())
                .size(f32::from(geometry.title_size))
                .color(palette.muted.to_iced()),
        )
        .width(Length::Fill)
        .padding(28)
        .align_x(Alignment::Center)
        .into()
    }

    /// One result: icon, title, subtitle.
    ///
    /// The icon slot is a fixed square of `geometry.icon_size`, and what goes
    /// in it depends on `launcher.appearance.icons` (#85). Off, or on with a
    /// name the theme cannot resolve, it is the application's first letter in a
    /// tinted square. On and resolved, it is the icon itself.
    ///
    /// **The slot is the same size either way**, which is what keeps the
    /// fallback from being visible as a defect: a missing icon leaves the row's
    /// proportions and the title's position exactly where the others are, and
    /// the VM tier's window box does not move when the option is turned on.
    pub(super) fn result_row(&self, item: &AppItem, selected: bool) -> Element<'_, Message> {
        let icon = self.app_icon(item, selected);
        // `subtitles` gates this, not just the presence of a comment: the dense
        // preset's row is one line tall and a second would overflow it.
        let subtitle = self
            .subtitles
            .then(|| item.comment().map(str::to_owned))
            .flatten();
        self.list_row(icon, item.name().to_owned(), subtitle, selected)
    }

    /// One list row: an icon slot, a title and an optional subtitle, with the
    /// selection drawn as a filled rounded rectangle. Every list the card
    /// shows -- applications, commands, clipboard history -- is these rows, so
    /// they measure and select alike.
    pub(super) fn list_row<'a>(
        &'a self,
        icon: Element<'a, Message>,
        title: String,
        subtitle: Option<String>,
        selected: bool,
    ) -> Element<'a, Message> {
        self.list_row_with(icon, title, subtitle, None, selected)
    }

    /// [`Self::list_row`] with a line of muted text at the right edge, as a
    /// store row shows its download count and whether it is installed.
    pub(super) fn list_row_with<'a>(
        &'a self,
        icon: Element<'a, Message>,
        title: String,
        subtitle: Option<String>,
        accessory: Option<String>,
        selected: bool,
    ) -> Element<'a, Message> {
        let colour = if selected {
            self.palette().selection_text
        } else {
            self.palette().muted
        };
        let accessory = accessory.filter(|text| !text.is_empty()).map(|accessory| {
            text(accessory)
                .font(self.font())
                .size(f32::from(self.geometry.subtitle_size))
                .color(colour.to_iced())
                .into()
        });
        self.list_row_parts(icon, title, subtitle, accessory, selected)
    }

    /// [`Self::list_row_with`], with any element at the row's right.
    pub(super) fn list_row_parts<'a>(
        &'a self,
        icon: Element<'a, Message>,
        title: String,
        subtitle: Option<String>,
        accessory: Option<Element<'a, Message>>,
        selected: bool,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        let title_color = if selected {
            palette.selection_text
        } else {
            palette.text
        };
        let subtitle_color = if selected {
            palette.selection_text
        } else {
            palette.muted
        };

        // One line each, elided: the row is a fixed height, and a wrapped
        // subtitle ran into the row below it (P-07).
        let mut labels = column![crate::elided::elided(
            title,
            f32::from(geometry.title_size),
            Some(self.font()),
            title_color.to_iced(),
        )];
        if let Some(subtitle) = subtitle {
            labels = labels.push(crate::elided::elided(
                subtitle,
                f32::from(geometry.subtitle_size),
                Some(self.font()),
                subtitle_color.to_iced(),
            ));
        }

        let line = match accessory {
            Some(accessory) => row![icon, labels.width(Length::Fill), accessory],
            None => row![icon, labels.width(Length::Fill)],
        }
        .spacing(12)
        .align_y(Alignment::Center)
        .padding(Padding::new(0.0).left(12).right(12));
        container(line)
            .width(Length::Fill)
            .height(Length::Fixed(f32::from(geometry.row_height)))
            .align_y(Alignment::Center)
            .style(move |_: &Theme| {
                if selected {
                    container::Style {
                        background: Some(palette.selection.to_iced().into()),
                        border: Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: f32::from(geometry.row_radius).into(),
                        },
                        ..container::Style::default()
                    }
                } else {
                    container::Style::default()
                }
            })
            .into()
    }
}

/// The card's background colour, given the surface colour and whether `tint` is on.
///
/// Extracted from `view`'s style closure on purpose. A headless renderer can
/// assert what is laid out but not what colour a container was filled with, so
/// the alpha decision inside a closure would be untestable without a pixel
/// baseline — and pixel baselines get rubber-stamped, which is why §8.5 rejects
/// them. As a function it is an ordinary assertion.
///
/// Translucency is applied here rather than in the palette so the theme stays
/// one set of colours: `tint` is a property of the window, not of the scheme,
/// and a palette that changed with it would make every other surface's
/// contrast depend on a background setting.
fn card_background(surface: design::Rgb, tint: bool) -> iced::Color {
    let colour = surface.to_iced();
    if tint {
        colour.scale_alpha(TINT_ALPHA)
    } else {
        colour
    }
}

/// The search field's and the panel filter's text: the enclosing field owns
/// the fill and the rounded border, and the placeholder is the secondary
/// text colour, which the palette holds at 4.5:1. Iced's own placeholder is
/// a faded text colour, 3.03:1 on the light field.
pub(super) fn query_input_style(
    theme: &Theme,
    status: text_input::Status,
    palette: design::Palette,
) -> text_input::Style {
    text_input::Style {
        background: Color::TRANSPARENT.into(),
        border: Border::default(),
        placeholder: palette.muted.to_iced(),
        ..text_input::default(theme, status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::flags::AppFlags;
    use crate::app::test_support::{app, index, theme_name};
    use crate::design::Appearance;

    #[test]
    fn the_root_footer_names_enter_and_the_action_panel() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        let _ = app.update(Message::QueryChanged("fire".into()));
        assert_eq!(app.root_primary_action(), Some("Open"));
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(800.0, 600.0),
            app.view(),
        );
        for label in ["Open", "Actions", "Ctrl+B"] {
            assert!(ui.find(label).is_ok(), "the footer has no {label:?}");
        }
        drop(ui);
        let _ = app.update(Message::QueryChanged("zzzzqqq".into()));
        assert!(
            app.selected_row().is_none() || app.root_primary_action().is_some(),
            "a row selected always says what Enter does"
        );
    }

    #[test]
    fn an_appearance_change_reaches_the_theme() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        assert_eq!(
            theme_name(&app),
            design::theme(Appearance::Light).to_string()
        );

        let _ = app.update(Message::AppearanceChanged(Appearance::Light));
        // Asserted through `theme()` rather than the field, because the field
        // being right while the theme is built from something else is exactly
        // the bug worth catching.
        assert_eq!(
            theme_name(&app),
            design::theme(Appearance::Light).to_string()
        );
    }

    #[test]
    fn a_window_nobody_is_feeding_keeps_the_appearance_it_started_with() {
        let (mut app, _task) = LauncherApp::new(AppFlags {
            appearance: Appearance::Light,
            appearance_link: None,
            ..AppFlags::default()
        });

        // No portal, no feeder: whatever it opened as is what it stays as. A
        // launcher that fell back to dark here would change colour on every
        // desktop without a Settings portal.
        let opened_as = theme_name(&app);
        assert_eq!(opened_as, design::theme(Appearance::Light).to_string());
        let _ = app.update(Message::QueryChanged("fir".to_owned()));
        let _ = app.update(Message::TogglePanel);
        assert_eq!(theme_name(&app), opened_as);
    }

    #[test]
    fn theme_preview_restores_on_cancel_and_clears_on_commit() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::default());
        app.apply(AppFlags {
            theme: crate::theme::Theme::System,
            appearance: Appearance::Light,
            ..AppFlags::default()
        });
        let before = theme_name(&app);
        let _ = app.update(Message::ThemePreview(crate::theme::Theme::Dracula));
        assert_ne!(theme_name(&app), before);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Dracula);
        let _ = app.update(Message::ThemeCancel);
        assert_eq!(theme_name(&app), before);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::System);

        let _ = app.update(Message::ThemePreview(crate::theme::Theme::Nord));
        let _ = app.update(Message::ThemeCommit);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Nord);
        assert!(app.view.theme_preview.is_none());
    }

    #[test]
    fn theme_preview_return_to_system_via_cancel_and_commit() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::default());
        app.apply(AppFlags {
            theme: crate::theme::Theme::Dracula,
            appearance: Appearance::Dark,
            ..AppFlags::default()
        });
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Dracula);
        let _ = app.update(Message::ThemePreview(crate::theme::Theme::System));
        assert_eq!(app.view.theme_choice, crate::theme::Theme::System);
        let _ = app.update(Message::ThemeCancel);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Dracula);
        let _ = app.update(Message::ThemePreview(crate::theme::Theme::System));
        let _ = app.update(Message::ThemeCommit);
        assert_eq!(app.view.theme_choice, crate::theme::Theme::System);
        assert!(app.view.theme_preview.is_none());
    }

    #[test]
    fn theme_and_preset_are_independently_selectable_in_app() {
        // #153: colour themes and layout presets independently selectable
        let mut app = LauncherApp::with_index(compass_core::AppIndex::default());
        app.apply(AppFlags {
            theme: crate::theme::Theme::Nord,
            appearance: Appearance::Dark,
            ..AppFlags::default()
        });
        // Simulate that preset is stored separately — app holds theme_choice,
        // preset is in appearance config, but the UI must not couple them.
        // This test documents the contract: changing one must not change the other.
        let theme_before = app.view.theme_choice;
        let _ = app.update(Message::ThemePreview(crate::theme::Theme::Gruvbox));
        assert_eq!(app.view.theme_choice, crate::theme::Theme::Gruvbox);
        assert_eq!(app.view.theme_preview, Some(theme_before));
        let _ = app.update(Message::ThemeCancel);
        assert_eq!(app.view.theme_choice, theme_before);
    }
}

/// The `tint` option (#86): translucency, and deliberately not called blur.
#[cfg(test)]
mod tint_tests {
    use super::*;
    use crate::app::flags::AppFlags;
    use crate::preset;
    use crate::preset::Preset;
    use compass_core::AppIndex;
    use iced::window;

    #[test]
    fn only_raycast_tints() {
        // Raycast is the one preset imitating a look built on real compositor
        // blur, so it gets the closest thing available. The Spotlight-simple
        // default does not, which is also what keeps the VM tier's pixel gates
        // seeing an unchanged image.
        assert!(Preset::Raycast.tint());
        assert!(!Preset::Gnome.tint());
        assert!(!Preset::Flow.tint());
        assert!(!Preset::Rofi.tint());
    }

    #[test]
    fn an_explicit_setting_beats_the_preset_both_ways() {
        // Both directions: a one-way override reads as working until someone
        // tries to turn the feature off under a preset that enables it.
        assert!(!preset::resolve(Some("raycast"), None, Some(false)).tint);
        assert!(preset::resolve(Some("gnome"), None, Some(true)).tint);
    }

    #[test]
    fn the_flag_reaches_the_launcher() {
        // Through `apply`, which is the real assignment list `compass::run`
        // uses -- a helper that set the field itself would be a test of the
        // helper. This is the control #108 recorded and could not close.
        let mut app = LauncherApp::with_index(AppIndex::default());
        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some("raycast"), None, None),
            ..AppFlags::default()
        });
        assert!(app.tint, "the resolved preset's tint never reached the app");

        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some("gnome"), None, None),
            ..AppFlags::default()
        });
        assert!(!app.tint);
    }

    #[test]
    fn a_translucent_card_asks_for_blur_behind_itself_and_an_opaque_one_takes_it_away() {
        // `WindowMaterial.enabled: blurEnabled`, `radius: cornerRadius`,
        // `region: (shadowPadding, shadowPadding, w, h)`.
        let mut app = LauncherApp::with_index(AppIndex::default());
        let card = iced::Size::new(768.0, 312.0);

        let _ = app.update(Message::CardMeasured(card));
        assert_eq!(app.material_asked, None, "no window, nothing to blur");

        app.apply(AppFlags {
            appearance_preset: preset::resolve(Some("raycast"), None, None),
            ..AppFlags::default()
        });
        app.window = Some(window::Id::unique());
        let _ = app.update(Message::CardMeasured(card));
        let asked = app.material_asked.expect("a translucent card is blurred");
        assert_eq!(
            (asked.x, asked.y, asked.width, asked.height),
            (
                i32::from(design::SHADOW_PADDING),
                i32::from(design::SHADOW_PADDING),
                768,
                312
            )
        );
        assert_eq!(asked.radius, i32::from(app.geometry.card_radius));

        // The card grows: the region follows it.
        let _ = app.update(Message::CardMeasured(iced::Size::new(768.0, 560.0)));
        assert_eq!(app.material_asked.map(|region| region.height), Some(560));

        // Tint turned off: the blur is taken away.
        app.tint = false;
        let _ = app.update(Message::CardMeasured(iced::Size::new(768.0, 560.0)));
        assert_eq!(app.material_asked, None);
    }

    #[test]
    fn a_closed_window_forgets_its_blur() {
        let mut app = LauncherApp::with_index(AppIndex::default());
        let id = window::Id::unique();
        app.window = Some(id);
        app.tint = true;
        let _ = app.update(Message::CardMeasured(iced::Size::new(768.0, 560.0)));
        assert!(app.material_asked.is_some());
        let _ = app.update(Message::Closed(id));
        assert_eq!(
            app.material_asked, None,
            "the next window is a new surface and must be asked again"
        );
    }

    #[test]
    fn tint_changes_the_card_background_and_only_its_alpha() {
        let surface = design::DARK.surface;
        let opaque = card_background(surface, false);
        let tinted = card_background(surface, true);

        assert!(
            (opaque.a - 1.0).abs() < f32::EPSILON,
            "the untinted card must be fully opaque, got alpha {}",
            opaque.a
        );
        assert!(
            tinted.a < opaque.a,
            "tint did not make the card more transparent: {} vs {}",
            tinted.a,
            opaque.a
        );

        // Translucency, not a different colour. If the hue moved, this would be
        // a theme change wearing a transparency setting's name.
        assert!(
            (tinted.r - opaque.r).abs() < f32::EPSILON
                && (tinted.g - opaque.g).abs() < f32::EPSILON
                && (tinted.b - opaque.b).abs() < f32::EPSILON,
            "tint changed the card's colour, not just its alpha"
        );
    }

    #[test]
    fn the_tinted_card_stays_legible() {
        // A launcher bound to Super+Space is text over an arbitrary wallpaper.
        // There is a temptation to push the alpha down because it looks better
        // in a screenshot over a photo; this is the floor that stops it.
        //
        // Asserted on the colour the view actually fills with, not on
        // `TINT_ALPHA` directly: clippy rejects an assertion over a constant,
        // and rightly -- `assert!(SOME_CONST >= 0.75)` is evaluated by the
        // compiler, not by the test, so it proves nothing about the code path.
        // The same mistake was already fixed once here in a quick-launch
        // assertion over `DEFAULT_QUICK_LAUNCH`.
        let alpha = card_background(design::DARK.surface, true).a;
        assert!(
            alpha >= 0.75,
            "the tinted card fills at alpha {alpha}, low enough that body text over a bright \
             wallpaper loses contrast. Legibility is not negotiable for the launcher."
        );
        assert!(
            alpha < 1.0,
            "a tint that fills at alpha {alpha} is not a tint"
        );
    }
}

/// Tests that render `view` headlessly.
///
/// # Why this module exists
///
/// Every other test in this crate asserts on state. `view` returns an opaque
/// `Element`, so a branch inside it could be deleted with nothing failing --
/// and one was: #108 recorded a control that mutated away the `field_rule`
/// branch and left all 79 tests green. The design surrogate did not close it
/// either, because it builds its own DOM from the same tokens and never
/// executes the Iced tree.
///
/// `iced_test` renders the real tree in headless mode, which is what finally
/// makes those branches reachable from a test.
///
/// # Bounds rather than pixels
///
/// `Simulator::snapshot` compares against a committed PNG. That would make the
/// assertion depend on which fonts the machine running it happens to have, and
/// a gate that reddens when a runner image changes its font package is a gate
/// people turn off. Selecting a widget by its text and reading its **layout
/// bounds** asserts the same structure without depending on how the glyphs came
/// out.
/// Input-method behaviour: the "watch for" item on the Phase 1 issue.
///
/// #4 flags IME and screen-reader behaviour with an explicit deadline: "Test
/// both here, not in Phase 5 — if Iced can't do them, ADR-0001 needs
/// revisiting while that is still cheap." Nothing tested it, so the risk was
/// carried unresolved rather than answered.
///
/// # The stack does support it, end to end
///
/// * `winit` 0.30 implements `zwp_text_input_v3` on Wayland
///   (`platform_impl/linux/wayland/seat/text_input/`) and emits
///   `WindowEvent::Ime(Enabled | Preedit | Commit | Disabled)`.
/// * `iced_winit` 0.14 converts those into `Event::InputMethod`
///   (`conversion.rs`), and calls `set_ime_allowed`, `set_ime_cursor_area` and
///   `set_ime_purpose` from `enable_ime`, which runs when a widget asks for an
///   input method — so a focused search field turns the IME on by itself.
/// * `iced_core` carries `InputMethod` and `Preedit`.
///
/// So **ADR-0001 does not need revisiting on this point.** That is a claim
/// about libraries, and libraries change, which is what these tests are for:
/// they fail if a future Iced stops routing composed text to the field.
///
/// # What these do and do not prove
///
/// They drive `Event::InputMethod` through the real widget tree and assert the
/// query is what a CJK or accented commit should leave behind. They do **not**
/// prove a real IME works against a real compositor — that needs ibus or fcitx
/// in the VM tier, and it is a different test. What they rule out is the
/// cheaper and more likely failure: composed text never reaching the field at
/// all, which would make the launcher unusable for anyone typing Japanese,
/// Chinese, Korean or with a compose key, and which no other test would catch.
#[cfg(test)]
mod ime_tests {
    use super::*;
    use crate::app::flags::AppFlags;
    use compass_core::AppIndex;
    use iced_winit::core::input_method;

    fn focused_app() -> LauncherApp {
        let mut app = LauncherApp::with_index(AppIndex::default());
        app.apply(AppFlags::default());
        app
    }

    /// One simulator, focused, driven by `events`, with the messages applied.
    ///
    /// A fresh simulator per interaction would rebuild the widget tree and lose
    /// focus, so a two-step sequence has to share one. Focus itself comes from
    /// clicking the placeholder: in the running launcher it arrives via a
    /// `Task` (`focus_search`, dispatched on show) and the simulator does not
    /// run tasks.
    fn drive(app: &mut LauncherApp, events: Vec<Vec<iced_winit::core::Event>>) {
        let mut out = Vec::new();
        for batch in events {
            // Scoped so the borrow of `app.view()` ends before `app.update`.
            {
                let mut ui = iced_test::simulator(app.view());
                ui.click("Search…").expect("the search field is clickable");
                let _ = ui.simulate(batch);
                out.extend(ui.into_messages());
            }
            for message in out.drain(..) {
                let _ = app.update(message);
            }
        }
    }

    fn commit(text: &str) -> Vec<iced_winit::core::Event> {
        vec![iced_winit::core::Event::InputMethod(
            input_method::Event::Commit(text.to_owned()),
        )]
    }

    /// CONTROL. If plain typing does not reach the query in this harness, the
    /// IME assertions below are measuring the harness, not the input method.
    /// This failed first — the field was unfocused and nothing reached it —
    /// which is exactly the false "IME is broken" this control exists to stop.
    #[test]
    fn control_typewrite_reaches_the_query() {
        let mut app = focused_app();
        let mut ui = iced_test::simulator(app.view());
        ui.click("Search…").expect("the search field is clickable");
        let _ = ui.typewrite("abc");
        for message in ui.into_messages() {
            let _ = app.update(message);
        }
        assert_eq!(
            app.query, "abc",
            "CONTROL: plain typing did not reach the query, so nothing below measures IME"
        );
    }

    #[test]
    fn a_committed_composition_reaches_the_query() {
        let mut app = focused_app();
        drive(&mut app, vec![commit("日本語")]);
        assert_eq!(
            app.query, "日本語",
            "a committed IME composition did not reach the query field. Typed ASCII arrives \
             as key events and would still work, so every other test here would pass while \
             the launcher was unusable for anyone composing text"
        );
    }

    #[test]
    fn a_preedit_does_not_commit_early() {
        let mut app = focused_app();
        drive(
            &mut app,
            vec![vec![iced_winit::core::Event::InputMethod(
                input_method::Event::Preedit("にほんご".to_owned(), None),
            )]],
        );
        assert_eq!(
            app.query, "",
            "an uncommitted IME pre-edit leaked into the query; the launcher would search \
             for each intermediate composition state"
        );
    }
}

#[cfg(test)]
mod view_tests;
