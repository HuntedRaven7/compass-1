//! Drawing the settings view: the sidebar, each page, and the rows and
//! controls they are built from.
//!
//! What a control does when it is used is [`super::settings_view`]'s; this
//! module only turns the [`crate::settings_page::SettingsPage`] model into
//! widgets.

use iced::widget::{button, pick_list, toggler};

use super::settings_view::{
    BODY_HEIGHT, LOGIN_START_DEFAULT, SIDEBAR_WIDTH, field_placeholder, settings, shortcut_label,
};
use super::{
    Element, LauncherApp, Length, Message, Padding, column, container, mouse_area, row, scrollable,
    text, text_input,
};
use crate::adwaita;
use crate::settings::SidebarKind;
use crate::settings_page::{
    Control, DOCS_URL, HINT, ItemEntry, ProviderEntry, RecordTarget, SettingsMessage, SettingsPage,
    Shown, alias_key,
};
use crate::shortcut_recorder::ShortcutRecorder;
use compass_core::settings_catalog::{self, CorePage, Kind, Setting};

impl LauncherApp {
    /// The view's body: the sidebar, the page, and a line for what went
    /// wrong and what the keys do.
    pub(super) fn settings_body<'a>(&'a self, page: &'a SettingsPage) -> Element<'a, Message> {
        let content: Element<'a, Message> = match (&page.recorder, page.shown()) {
            (Some((_, recorder)), _) => self.settings_recorder(recorder),
            (None, Shown::Core(core)) => self.settings_core_page(page, core),
            (None, Shown::Provider(provider)) => self.settings_provider_page(page, provider),
            (None, Shown::Nothing) => self.notice("No settings match"),
        };
        let palette = self.palette();
        let footer = text(page.notice.clone().unwrap_or_else(|| HINT.to_owned()))
            .font(self.font())
            .size(12)
            .color(if page.notice.is_some() {
                palette.error().to_iced()
            } else {
                palette.muted.to_iced()
            });
        column![
            row![
                self.settings_sidebar(page),
                container(
                    scrollable(container(content).padding(Padding::new(12.0)))
                        .id(crate::scroll::SETTINGS_BODY)
                )
                .width(Length::Fill)
                .height(Length::Fill),
            ]
            .height(Length::Fixed(BODY_HEIGHT)),
            container(footer).padding(Padding::new(6.0).left(14)),
        ]
        .into()
    }

    fn settings_sidebar<'a>(&'a self, page: &'a SettingsPage) -> Element<'a, Message> {
        let palette = self.palette();
        let mut list = column![].spacing(2);
        for (position, entry) in page.sidebar.rows().iter().enumerate() {
            if entry.kind == SidebarKind::Divider {
                list = list.push(
                    container(
                        iced::widget::rule::horizontal(1)
                            .style(move |_: &iced::Theme| adwaita::separator(palette)),
                    )
                    .padding(Padding::new(6.0)),
                );
                continue;
            }
            let selected = position as isize == page.selected;
            let colour = if entry.enabled {
                palette.text
            } else {
                palette.muted
            }
            .to_iced();
            let label = container(
                text(entry.label.clone())
                    .font(self.font())
                    .size(13)
                    .color(colour),
            )
            .width(Length::Fill)
            .padding(Padding::new(8.0).left(12))
            .style(move |_: &iced::Theme| adwaita::sidebar_row(palette, selected));
            let row =
                mouse_area(label).on_press(settings(SettingsMessage::SidebarSelected(position)));
            list = list.push(if selected {
                Element::from(container(row).id(crate::scroll::SETTINGS_SIDEBAR_SELECTION))
            } else {
                row.into()
            });
        }
        container(
            scrollable(container(list).padding(Padding::new(8.0)))
                .id(crate::scroll::SETTINGS_SIDEBAR),
        )
        .width(Length::Fixed(SIDEBAR_WIDTH))
        .height(Length::Fill)
        .into()
    }

    fn settings_heading(&self, label: String) -> Element<'_, Message> {
        text(label)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..self.font()
            })
            .size(16)
            .into()
    }

    /// A preferences group's title (`AdwPreferencesGroup:title`): bold, in
    /// the text colour, over its boxed list.
    fn settings_group_title(&self, label: String) -> Element<'_, Message> {
        container(
            text(label)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..self.font()
                })
                .size(13),
        )
        .padding(Padding::new(0.0).left(2))
        .into()
    }

    /// Rows in a boxed list (`.boxed-list`): one card, 12 px corners, a
    /// hairline between rows.
    fn settings_boxed<'a>(&self, rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
        let palette = self.palette();
        let mut list = column![];
        for (index, entry) in rows.into_iter().enumerate() {
            if index > 0 {
                list = list.push(
                    iced::widget::rule::horizontal(1)
                        .style(move |_: &iced::Theme| adwaita::separator(palette)),
                );
            }
            list = list.push(entry);
        }
        container(list)
            .width(Length::Fill)
            .style(move |_: &iced::Theme| adwaita::boxed_list(palette))
            .into()
    }

    /// A group: its title, if it has one, over its rows in a boxed list.
    fn settings_group<'a>(
        &'a self,
        title: Option<String>,
        rows: Vec<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let mut group = column![].spacing(adwaita::SPACING / 2.0);
        if let Some(title) = title {
            group = group.push(self.settings_group_title(title));
        }
        group.push(self.settings_boxed(rows)).into()
    }

    /// One labelled row (`AdwActionRow`): the label and its description,
    /// the control on the right, at least 50 px tall.
    fn settings_row<'a>(
        &'a self,
        label: String,
        description: String,
        control: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let mut words = column![text(label).font(self.font()).size(13)].spacing(2);
        if !description.is_empty() {
            words = words.push(
                text(description)
                    .font(self.font())
                    .size(11)
                    .color(self.palette().muted.to_iced()),
            );
        }
        container(
            row![
                iced::widget::Space::new().height(adwaita::ROW_MIN_CONTENT),
                row![container(words).width(Length::Fill), control]
                    .spacing(adwaita::SPACING)
                    .align_y(iced::Alignment::Center),
            ]
            .align_y(iced::Alignment::Center),
        )
        .padding(adwaita::ROW_PADDING)
        .into()
    }

    /// `control` with its focus ring, drawn while the page's keyboard is on
    /// `target`. A field draws its own ring, so it only takes the room; the
    /// control with the keyboard is also what the page scrolls to, unless an
    /// open dropdown's highlighted option is.
    fn settings_ring<'a>(
        &self,
        page: &SettingsPage,
        target: &Control,
        control: impl Into<Element<'a, Message>>,
        radius: f32,
    ) -> Element<'a, Message> {
        let focused = page.has_focus(target);
        let ring = adwaita::ringed(
            control,
            self.palette(),
            focused && target.field_id().is_none(),
            radius,
        );
        if focused && page.menu.is_none() {
            ring.id(crate::scroll::SETTINGS_FOCUS).into()
        } else {
            ring.into()
        }
    }

    /// An open dropdown's options, under its row: the highlighted one in
    /// the selection's colours and the current one ticked.
    fn settings_menu<'a>(
        &self,
        key: &str,
        options: Vec<(String, String)>,
        current: &str,
        highlighted: usize,
    ) -> Element<'a, Message> {
        let options = options
            .into_iter()
            .map(|(value, label)| {
                let ticked = value.eq_ignore_ascii_case(current);
                let pick = settings(SettingsMessage::Changed(
                    key.to_owned(),
                    serde_json::Value::String(value),
                ));
                (label, ticked, pick)
            })
            .collect();
        container(self.choice_menu(options, highlighted))
            .padding(Padding::new(4.0).left(12).right(12).bottom(8))
            .into()
    }

    /// A dropdown's options opened from the keyboard, as `(label, whether it
    /// is the current value, what clicking it sends)`: the highlighted one
    /// in the selection's colours, and what the page scrolls to.
    pub(super) fn choice_menu<'a>(
        &self,
        options: Vec<(String, bool, Message)>,
        highlighted: usize,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let mut list = column![].spacing(2);
        for (index, (label, ticked, pick)) in options.into_iter().enumerate() {
            let selected = index == highlighted;
            let colour = if selected {
                palette.selection_text
            } else {
                palette.text
            }
            .to_iced();
            let line = row![
                text(label)
                    .font(self.font())
                    .size(13)
                    .color(colour)
                    .width(Length::Fill),
                text(if ticked { "✓" } else { "" })
                    .font(self.font())
                    .size(13)
                    .color(colour),
            ]
            .align_y(iced::Alignment::Center);
            let option = container(line)
                .width(Length::Fill)
                .padding(Padding::new(6.0).left(12).right(12))
                .style(move |_: &iced::Theme| iced::widget::container::Style {
                    background: selected.then(|| palette.selection.to_iced().into()),
                    border: iced::Border {
                        radius: adwaita::CONTROL_RADIUS.into(),
                        ..iced::Border::default()
                    },
                    ..iced::widget::container::Style::default()
                });
            let option = mouse_area(option).on_press(pick);
            list = list.push(if selected {
                Element::from(container(option).id(crate::scroll::SETTINGS_FOCUS))
            } else {
                option.into()
            });
        }
        list.into()
    }

    /// A row the search matched: tinted with the accent, and the best one is
    /// what the page scrolls to while the search field has the keyboard.
    fn settings_match<'a>(
        &self,
        page: &SettingsPage,
        key: &str,
        row: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let tinted = container(row)
            .width(Length::Fill)
            .style(move |_: &iced::Theme| iced::widget::container::Style {
                background: Some(
                    iced::Color {
                        a: 0.10,
                        ..palette.accent.to_iced()
                    }
                    .into(),
                ),
                border: iced::Border {
                    color: palette.accent.to_iced(),
                    width: 1.0,
                    radius: adwaita::CONTROL_RADIUS.into(),
                },
                ..iced::widget::container::Style::default()
            });
        if page.focused.is_none() && page.first_match_shown().as_deref() == Some(key) {
            tinted.id(crate::scroll::SETTINGS_FOCUS).into()
        } else {
            tinted.into()
        }
    }

    /// A flat, Adwaita-style button.
    fn settings_button<'a>(&self, label: String, size: f32) -> iced::widget::Button<'a, Message> {
        let palette = self.palette();
        button(text(label).font(self.font()).size(size))
            .padding(adwaita::CONTROL_PADDING)
            .style(move |_: &iced::Theme, status| adwaita::button(palette, status))
    }

    /// A setting's control, by its kind.
    fn settings_control<'a>(
        &'a self,
        page: &'a SettingsPage,
        setting: &Setting,
    ) -> Element<'a, Message> {
        let target = Control::Setting(setting.key.clone());
        let radius = if setting.kind == Kind::Toggle {
            adwaita::SWITCH_SIZE / 2.0
        } else {
            adwaita::CONTROL_RADIUS
        };
        let control = self.settings_bare_control(page, setting, &target);
        self.settings_ring(page, &target, control, radius)
    }

    fn settings_bare_control<'a>(
        &'a self,
        page: &'a SettingsPage,
        setting: &Setting,
        target: &Control,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let key = setting.key.clone();
        let value = page.value(setting);
        match &setting.kind {
            Kind::Toggle => {
                let on = value.as_bool().unwrap_or(false);
                toggler(on)
                    .size(adwaita::SWITCH_SIZE)
                    .style(move |_: &iced::Theme, status| adwaita::switch(palette, status))
                    .on_toggle(move |on| {
                        settings(SettingsMessage::Changed(
                            key.clone(),
                            serde_json::Value::Bool(on),
                        ))
                    })
                    .into()
            }
            Kind::Choice(options) => {
                let labels: Vec<String> = options.iter().map(|(_, l)| (*l).to_owned()).collect();
                let current = value.as_str().and_then(|value| {
                    options
                        .iter()
                        .find(|(option, _)| *option == value)
                        .map(|(_, label)| (*label).to_owned())
                });
                let options = options.clone();
                pick_list(labels, current, move |label: String| {
                    let chosen = options
                        .iter()
                        .find(|(_, l)| *l == label)
                        .map_or("", |(option, _)| *option);
                    settings(SettingsMessage::Changed(
                        key.clone(),
                        serde_json::Value::String(chosen.to_owned()),
                    ))
                })
                .text_size(13)
                .font(self.font())
                .padding(adwaita::CONTROL_PADDING)
                .style(move |_: &iced::Theme, status| adwaita::dropdown(palette, status))
                .menu_style(move |_: &iced::Theme| crate::design::dropdown_menu(palette))
                .into()
            }
            Kind::Theme => {
                let titles: Vec<String> = page.themes.iter().map(|(_, t)| t.clone()).collect();
                let current = value.as_str().and_then(|value| {
                    page.themes
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(value))
                        .map(|(_, title)| title.clone())
                });
                let themes = page.themes.clone();
                pick_list(titles, current, move |title: String| {
                    let name = themes
                        .iter()
                        .find(|(_, t)| *t == title)
                        .map_or_else(String::new, |(name, _)| name.clone());
                    settings(SettingsMessage::Changed(
                        key.clone(),
                        serde_json::Value::String(name),
                    ))
                })
                .text_size(13)
                .font(self.font())
                .padding(adwaita::CONTROL_PADDING)
                .style(move |_: &iced::Theme, status| adwaita::dropdown(palette, status))
                .menu_style(move |_: &iced::Theme| crate::design::dropdown_menu(palette))
                .into()
            }
            Kind::Shortcut => self
                .settings_button(shortcut_label(value.as_str()), 13.0)
                .on_press(settings(SettingsMessage::Record(RecordTarget::Setting(
                    key,
                ))))
                .into(),
            Kind::Number { .. } | Kind::Text | Kind::Paths | Kind::Names | Kind::Font => {
                let submit = key.clone();
                text_input(field_placeholder(setting), &page.text_of(setting))
                    .id(target.field_id().unwrap_or_default())
                    .font(self.font())
                    .size(13)
                    .padding(adwaita::ENTRY_PADDING)
                    .width(Length::Fixed(220.0))
                    .style(move |_: &iced::Theme, status| adwaita::entry(palette, status))
                    .on_input(move |draft| {
                        settings(SettingsMessage::DraftEdited(key.clone(), draft))
                    })
                    .on_submit(settings(SettingsMessage::DraftSubmitted(submit)))
                    .into()
            }
        }
    }

    /// Each setting's row, in order, with the section it is listed under.
    pub(super) fn settings_rows<'a>(
        &'a self,
        page: &'a SettingsPage,
        settings_shown: &[Setting],
    ) -> Vec<(&'static str, Element<'a, Message>)> {
        settings_shown
            .iter()
            .map(|setting| {
                // On Sway, Hyprland and niri the compositor owns the key, so
                // the row says where to bind it rather than offering a
                // recorder whose shortcut nothing would bind.
                if setting.key == "launcher.hotkey"
                    && let Some(compositor) = self.hotkey_compositor
                {
                    let control = text(format!("Set in {}", compositor.name()))
                        .font(self.font())
                        .size(13)
                        .color(self.palette().muted.to_iced())
                        .into();
                    return (
                        setting.section,
                        self.settings_row(setting.label.to_owned(), self.open_hint(), control),
                    );
                }
                let row = self.settings_row(
                    setting.label.to_owned(),
                    setting.description.to_owned(),
                    self.settings_control(page, setting),
                );
                let open = page
                    .menu
                    .filter(|_| page.has_focus(&Control::Setting(setting.key.clone())));
                let row = if page.is_match(&setting.key) {
                    self.settings_match(page, &setting.key, row)
                } else {
                    row
                };
                let row = match open {
                    Some(highlighted) => {
                        let current = page.value(setting);
                        column![
                            row,
                            self.settings_menu(
                                &setting.key,
                                page.options(setting),
                                current.as_str().unwrap_or_default(),
                                highlighted,
                            )
                        ]
                        .into()
                    }
                    None => row,
                };
                (setting.section, row)
            })
            .collect()
    }

    /// Settings grouped under their section titles, one boxed list each.
    fn settings_list<'a>(
        &'a self,
        page: &'a SettingsPage,
        settings_shown: &[Setting],
    ) -> Element<'a, Message> {
        let mut groups: Vec<(&'static str, Vec<Element<'a, Message>>)> = Vec::new();
        for (section, entry) in self.settings_rows(page, settings_shown) {
            match groups.last_mut() {
                Some((current, rows)) if *current == section => rows.push(entry),
                _ => groups.push((section, vec![entry])),
            }
        }
        let mut list = column![].spacing(adwaita::SPACING * 1.5);
        for (section, rows) in groups {
            let title = (!section.is_empty()).then(|| section.to_owned());
            list = list.push(self.settings_group(title, rows));
        }
        list.into()
    }

    fn settings_core_page<'a>(
        &'a self,
        page: &'a SettingsPage,
        core: CorePage,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let mut body =
            column![self.settings_heading(core.title().to_owned())].spacing(adwaita::SPACING);
        match core {
            CorePage::About => {
                let muted = palette.muted.to_iced();
                body = body
                    .push(
                        text(format!("Compass {}", env!("CARGO_PKG_VERSION")))
                            .font(self.font())
                            .size(14),
                    )
                    .push(
                        text("A launcher for the Linux desktop that runs Vicinae and Raycast extensions.")
                            .font(self.font())
                            .size(12)
                            .color(muted),
                    );
                if let Some(offer) = &self.update {
                    body = body.push(
                        row![
                            text(super::release_check::title(offer))
                                .font(self.font())
                                .size(13),
                            self.settings_ring(
                                page,
                                &Control::Link(offer.release_url.clone()),
                                self.settings_button("View release notes".to_owned(), 13.0)
                                    .style(move |_: &iced::Theme, status| {
                                        adwaita::suggested_button(palette, status)
                                    })
                                    .on_press(settings(SettingsMessage::OpenUrl(
                                        offer.release_url.clone()
                                    ))),
                                adwaita::CONTROL_RADIUS,
                            ),
                        ]
                        .spacing(adwaita::SPACING)
                        .align_y(iced::Alignment::Center),
                    );
                }
                body = body.push(
                    row![
                        self.settings_ring(
                            page,
                            &Control::Link(DOCS_URL.to_owned()),
                            self.settings_button("Documentation".to_owned(), 13.0)
                                .on_press(settings(SettingsMessage::OpenUrl(DOCS_URL.to_owned()))),
                            adwaita::CONTROL_RADIUS,
                        ),
                        self.settings_ring(
                            page,
                            &Control::Link(compass_core::bug_report::CREATE_ISSUE_URL.to_owned()),
                            self.settings_button("Report a Bug".to_owned(), 13.0)
                                .on_press(settings(SettingsMessage::OpenUrl(
                                    compass_core::bug_report::CREATE_ISSUE_URL.to_owned()
                                ))),
                            adwaita::CONTROL_RADIUS,
                        ),
                    ]
                    .spacing(adwaita::SPACING),
                );
            }
            CorePage::Keybindings => {
                let rows = settings_catalog::KEYBINDINGS
                    .iter()
                    .map(|(name, description, keys)| {
                        self.settings_row(
                            (*name).to_owned(),
                            (*description).to_owned(),
                            text(*keys).font(self.font()).size(13).into(),
                        )
                    })
                    .collect();
                body = body.push(self.settings_boxed(rows));
            }
            CorePage::General => {
                body = body
                    .push(self.settings_list(page, &SettingsPage::core_settings(core)))
                    .push(self.settings_group(
                        Some("Startup".to_owned()),
                        vec![self.settings_login_start(page)],
                    ));
            }
            CorePage::Appearance | CorePage::Advanced => {
                body = body.push(self.settings_list(page, &SettingsPage::core_settings(core)));
            }
        }
        let missing = SettingsPage::not_in_compass(core);
        if !missing.is_empty() {
            let muted = palette.muted.to_iced();
            let mut notes =
                column![self.settings_group_title("Not in Compass".to_owned())].spacing(4);
            for item in missing {
                notes = notes.push(
                    text(format!("{}: {}", item.label, item.reason))
                        .font(self.font())
                        .size(11)
                        .color(muted),
                );
            }
            body = body.push(notes);
        }
        body.into()
    }

    /// General's Start at login row: the switch, and what the engine says
    /// starts Compass at login, including a packaged systemd unit it leaves
    /// alone.
    fn settings_login_start<'a>(&'a self, page: &'a SettingsPage) -> Element<'a, Message> {
        let palette = self.palette();
        let on = self.login_start_on();
        let description = self.login_start_line(LOGIN_START_DEFAULT);
        let switch = toggler(on)
            .size(adwaita::SWITCH_SIZE)
            .style(move |_: &iced::Theme, status| adwaita::switch(palette, status))
            .on_toggle_maybe((!self.login_start_busy).then_some(Message::LoginStartToggled));
        self.settings_row(
            "Start at login".to_owned(),
            description,
            self.settings_ring(
                page,
                &Control::LoginStart,
                switch,
                adwaita::SWITCH_SIZE / 2.0,
            ),
        )
    }

    fn settings_provider_page<'a>(
        &'a self,
        page: &'a SettingsPage,
        provider: &'a ProviderEntry,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let id = provider.id.clone();
        let header = self.settings_row(
            provider.title.clone(),
            format!("{} · {} items", provider.provenance, provider.items.len()),
            self.settings_ring(
                page,
                &Control::ProviderSwitch(provider.id.clone()),
                toggler(provider.enabled)
                    .size(adwaita::SWITCH_SIZE)
                    .style(move |_: &iced::Theme, status| adwaita::switch(palette, status))
                    .on_toggle(move |on| {
                        settings(SettingsMessage::ProviderToggled(id.clone(), on))
                    }),
                adwaita::SWITCH_SIZE / 2.0,
            ),
        );
        let mut body = column![self.settings_boxed(vec![header])].spacing(adwaita::SPACING * 1.5);
        let own = SettingsPage::provider_settings(&provider.id);
        if !own.is_empty() {
            body = body.push(self.settings_list(page, &own));
        }
        if !provider.items.is_empty() {
            let rows = provider
                .items
                .iter()
                .flat_map(|item| self.settings_item(page, item))
                .collect();
            body = body.push(self.settings_boxed(rows));
        }
        body.into()
    }

    /// One root item's rows: its switch, alias, shortcut and preferences
    /// button, then the settings that belong to it, indented under it.
    fn settings_item<'a>(
        &'a self,
        page: &'a SettingsPage,
        item: &'a ItemEntry,
    ) -> Vec<Element<'a, Message>> {
        let palette = self.palette();
        let id = item.id.clone();
        let toggle_id = id.clone();
        let alias = alias_key(&id);
        let submit = alias.clone();
        let alias_text = page
            .drafts
            .get(&alias)
            .cloned()
            .unwrap_or_else(|| item.alias.clone().unwrap_or_default());
        let shortcut = shortcut_label(item.shortcut.as_deref());
        let alias_target = Control::ItemAlias(id.clone());
        let mut controls = row![
            self.settings_ring(
                page,
                &alias_target,
                text_input("Alias", &alias_text)
                    .id(alias_target.field_id().unwrap_or_default())
                    .font(self.font())
                    .size(12)
                    .padding(adwaita::ENTRY_PADDING)
                    .width(Length::Fixed(90.0))
                    .style(move |_: &iced::Theme, status| adwaita::entry(palette, status))
                    .on_input(move |draft| {
                        settings(SettingsMessage::DraftEdited(alias.clone(), draft))
                    })
                    .on_submit(settings(SettingsMessage::DraftSubmitted(submit))),
                adwaita::CONTROL_RADIUS,
            ),
            self.settings_ring(
                page,
                &Control::ItemShortcut(id.clone()),
                self.settings_button(shortcut, 12.0)
                    .on_press(settings(SettingsMessage::Record(RecordTarget::Item(
                        id.clone()
                    )))),
                adwaita::CONTROL_RADIUS,
            ),
        ]
        .spacing(adwaita::SPACING / 2.0)
        .align_y(iced::Alignment::Center);
        if item.has_preferences {
            controls = controls.push(
                self.settings_ring(
                    page,
                    &Control::ItemPreferences(id.clone()),
                    self.settings_button("Preferences".to_owned(), 12.0)
                        .on_press(settings(SettingsMessage::OpenPreferences(id.clone()))),
                    adwaita::CONTROL_RADIUS,
                ),
            );
        }
        controls = controls.push(
            self.settings_ring(
                page,
                &Control::ItemSwitch(id.clone()),
                toggler(item.enabled)
                    .size(adwaita::SWITCH_SIZE)
                    .style(move |_: &iced::Theme, status| adwaita::switch(palette, status))
                    .on_toggle(move |on| {
                        settings(SettingsMessage::ItemToggled(toggle_id.clone(), on))
                    }),
                adwaita::SWITCH_SIZE / 2.0,
            ),
        );
        let mut rows = vec![self.settings_row(item.title.clone(), String::new(), controls.into())];
        let own = SettingsPage::item_settings(&item.id);
        rows.extend(
            self.settings_rows(page, &own)
                .into_iter()
                .map(|(_, entry)| container(entry).padding(Padding::new(0.0).left(16)).into()),
        );
        rows
    }

    fn settings_recorder<'a>(&'a self, recorder: &'a ShortcutRecorder) -> Element<'a, Message> {
        let palette = self.palette();
        let badge = if recorder.tokens.is_empty() {
            "…".to_owned()
        } else {
            recorder.tokens.join(" ")
        };
        let mut body = column![
            self.settings_heading(recorder.title.clone()),
            text(badge).font(self.font()).size(20),
            text(recorder.status.clone())
                .font(self.font())
                .size(12)
                .color(if recorder.error {
                    palette.accent.to_iced()
                } else {
                    palette.muted.to_iced()
                }),
        ]
        .spacing(12)
        .align_x(iced::Alignment::Center);
        if recorder.current.is_some() {
            body = body.push(
                text(crate::shortcut_recorder::REMOVE_HINT)
                    .font(self.font())
                    .size(11)
                    .color(palette.muted.to_iced()),
            );
        }
        container(body).width(Length::Fill).padding(24).into()
    }
}
