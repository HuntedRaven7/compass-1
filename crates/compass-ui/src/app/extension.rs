//! An extension command in the launcher: starting it, the view it pushes,
//! the engine's polling for that view's changes, and drawing its list,
//! grid, detail and form.
//!
//! The command itself runs in the engine's extension host; this side holds
//! the view the host last rendered, sends the person's input back as
//! events, and pops the view when the host does.

use super::{
    Alignment, Border, Element, LauncherApp, Length, Message, Padding, Page, Space, Task, Theme,
    alert_keys, column, container, focus_search, mouse_area, row, scrollable, stores, text,
    text_input,
};

impl LauncherApp {
    /// An extension command's messages: its start, its view, and the
    /// person's input to that view.
    pub(super) fn extension_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ExtensionCommandStarted { id, title, result } => match result {
                Ok(crate::backend::ExtensionStart::Ran) => self.conceal(),
                Ok(crate::backend::ExtensionStart::NeedsPreferences { title, fields }) => {
                    self.page =
                        Page::Preferences(Box::new(crate::preferences_page::PreferencesPage::new(
                            crate::preferences_page::Purpose::Preferences,
                            id,
                            title,
                            fields,
                        )));
                    Task::none()
                }
                Ok(crate::backend::ExtensionStart::NeedsArguments { title, fields }) => {
                    self.page =
                        Page::Preferences(Box::new(crate::preferences_page::PreferencesPage::new(
                            crate::preferences_page::Purpose::Arguments,
                            id,
                            title,
                            fields,
                        )));
                    Task::none()
                }
                Ok(crate::backend::ExtensionStart::View(session)) => {
                    let mut page = crate::extension_page::ExtensionPage::new(session, title);
                    page.assets = self
                        .app_index
                        .extension(&id)
                        .map(|command| command.extension_dir.join("assets"));
                    page.leaves_on_end = compass_core::rhai_scripts::script_id(&id).is_some();
                    page.icon_lookup = Some(self.icon_lookup.clone());
                    let surface = self.palette().surface.to_iced();
                    page.prefers_dark =
                        0.299 * surface.r + 0.587 * surface.g + 0.114 * surface.b < 0.5;
                    self.page = Page::Extension(Box::new(page));
                    Task::batch([self.extension_poll(session, 0), focus_search()])
                }
                Err(reason) => {
                    self.error = Some(reason);
                    Task::none()
                }
            },
            Message::ExtensionViewLoaded { session, result } => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                if page.session != session {
                    return Task::none();
                }
                match result {
                    // The engine held the request until its deadline and
                    // nothing changed: ask again, keeping what is on screen.
                    Ok(state) if page.is_unchanged(&state) => {
                        let after = page.version;
                        self.extension_poll(session, after)
                    }
                    Ok(state) => {
                        let ended = state.ended;
                        // A script that popped itself: back to the root
                        // search, as leaving any view does.
                        if ended && state.problem.is_none() && page.leaves_on_end {
                            let close = self.close_extension_view();
                            self.page = Page::Root;
                            return Task::batch([close, focus_search(), self.search_task()]);
                        }
                        page.apply(state);
                        let after = page.version;
                        let images = crate::remote_image::fetch_tasks(page.wanted_images());
                        if ended {
                            images
                        } else {
                            Task::batch([
                                self.extension_poll(session, after),
                                crate::scroll::reveal_root_selection(),
                                images,
                            ])
                        }
                    }
                    Err(reason) => {
                        page.status = crate::extension_page::Status::Stopped(reason);
                        Task::none()
                    }
                }
            }
            Message::ExtensionQueryChanged(query) => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                page.query = query;
                let handler = page
                    .extension_filters()
                    .then(|| page.list().and_then(|list| list.search.on_change.clone()))
                    .flatten();
                match handler {
                    // The extension filters: it gets the text and the echo
                    // count, and renders a new list.
                    Some(handler) => {
                        page.query_events += 1;
                        let args = vec![
                            serde_json::Value::String(page.query.clone()),
                            serde_json::Value::from(page.query_events),
                        ];
                        let session = page.session;
                        self.extension_event(session, handler.0, args)
                    }
                    None => {
                        page.refilter();
                        crate::scroll::reveal_root_selection()
                    }
                }
            }
            Message::ExtensionItemSelected(position) => {
                if let Page::Extension(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.activate_extension_action()
            }
            Message::ExtensionFieldEdited(name, value) => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                let session = page.session;
                match page.edit_field(&name, value) {
                    Some((handler, args)) => self.extension_event(session, handler.0, args),
                    None => Task::none(),
                }
            }
            Message::ExtensionTextAreaEdited(name, action) => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                let session = page.session;
                match page.edit_text_area(&name, action) {
                    Some((handler, args)) => self.extension_event(session, handler.0, args),
                    None => Task::none(),
                }
            }
            Message::ExtensionImageFetched { url, result } => {
                if self.remote_requested.contains(&url) {
                    self.root_icon_arrived(&url, &result);
                }
                if self.store_image_arrived(&url, &result) {
                    return Task::none();
                }
                if let Page::Extension(page) = &mut self.page {
                    page.image_arrived(url, result);
                }
                Task::none()
            }
            Message::ExtensionDateEdited(name, typed) => {
                let Page::Extension(page) = &mut self.page else {
                    return Task::none();
                };
                let session = page.session;
                match page.edit_date(&name, typed) {
                    Some((handler, args)) => self.extension_event(session, handler.0, args),
                    None => Task::none(),
                }
            }
            Message::ExtensionChooseFiles { name, choice } => {
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                self.choosing_files = true;
                Task::perform(
                    async move {
                        let result = backend.choose_files(choice).await;
                        (name, result)
                    },
                    |(name, result)| Message::ExtensionFilesChosen { name, result },
                )
            }
            Message::ExtensionFilesChosen { name, result } => match result {
                Ok(paths) if paths.is_empty() => Task::none(),
                Ok(paths) => self.update(Message::ExtensionFieldEdited(
                    name,
                    serde_json::Value::from(paths),
                )),
                Err(reason) => {
                    if let Page::Extension(page) = &mut self.page {
                        page.notice = Some(reason);
                    }
                    Task::none()
                }
            },
            Message::ExtensionLinkClicked(url) => {
                // Web links open in the browser, through the engine; anything
                // else is said, not lost.
                match self.backend.clone() {
                    Some(backend) if crate::remote_image::is_remote(&url) => Task::perform(
                        async move { backend.open_url(url).await },
                        Message::StoreUrlOpened,
                    ),
                    _ => {
                        tracing::info!(%url, "a link in a view was clicked");
                        Task::none()
                    }
                }
            }
            Message::ExtensionEventSent(result) => {
                if let (Err(reason), Page::Extension(page)) = (result, &mut self.page) {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// Opens a builtin command's view, and counts the use like a launch.
    /// Hands an extension command to the engine, which runs it; the launcher
    /// hides once it has started, and says why when it could not.
    pub(super) fn run_extension_command(&mut self, index: usize) -> Task<Message> {
        self.run_extension_command_with(index, None)
    }

    /// [`Self::run_extension_command`], with the arguments entered for it.
    pub(super) fn run_extension_command_with(
        &mut self,
        index: usize,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Task<Message> {
        self.panel = None;
        let Some(command) = self.app_index.extensions().get(index) else {
            return Task::none();
        };
        let Some(backend) = self.backend.clone() else {
            self.error = Some(format!(
                "{} needs the Compass engine to run, and this window is running without one",
                command.title
            ));
            return Task::none();
        };
        let id = command.id.clone();
        let title = command.title.clone();
        let started_id = id.clone();
        Task::perform(
            async move { backend.run_extension_command(id, arguments).await },
            move |result| Message::ExtensionCommandStarted {
                id: started_id.clone(),
                title: title.clone(),
                result,
            },
        )
    }

    /// An extension command offered as a fallback: launched through the
    /// engine with the query as its fallback text, which comes back as a
    /// launch like any other (`OpenBuiltinCommandAction` forwarding the
    /// search text).
    pub(super) fn run_extension_fallback(&mut self, index: usize, query: String) -> Task<Message> {
        let Some(command) = self.app_index.extensions().get(index) else {
            return Task::none();
        };
        let Some(backend) = self.backend.clone() else {
            return self.run_extension_command(index);
        };
        let id = command.id.clone();
        Task::perform(
            async move { backend.launch_command(id, Some(query)).await },
            Message::BuiltinCommandDone,
        )
    }

    /// A command the launcher let go of failed. The launcher may have hidden
    /// before it did, so a refusal said only inside it would wait unseen for
    /// the next summon: it is kept as the error line, and the HUD says it now.
    /// The HUD has room for one short line, so it gets the reason alone.
    pub(super) fn command_failed(&mut self, title: &str, message: &str) -> Task<Message> {
        let hidden = self.closing || self.window.is_none();
        self.error = Some(if message.is_empty() {
            title.to_owned()
        } else {
            format!("{title}: {message}")
        });
        if hidden {
            self.error_for_summon.clone_from(&self.error);
            let line = if message.is_empty() { title } else { message };
            return self
                .put_up_hud_for_engine(crate::hud::Hud::new(line.to_owned()))
                .1;
        }
        Task::none()
    }

    /// Asks the engine for `session`'s next state after `after`.
    fn extension_poll(&self, session: u64, after: u64) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.extension_view(session, after).await },
            move |result| Message::ExtensionViewLoaded { session, result },
        )
    }

    pub(super) fn extension_event(
        &self,
        session: u64,
        handler: String,
        args: Vec<serde_json::Value>,
    ) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.extension_event(session, handler, args).await },
            Message::ExtensionEventSent,
        )
    }

    /// Escape on a pushed extension view: the extension pops it and renders
    /// the view beneath, which arrives like any other render.
    pub(super) fn extension_pop(&self, session: u64) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.extension_pop(session).await },
            Message::ExtensionEventSent,
        )
    }

    /// Enter in an extension's view: the first action on offer.
    pub(super) fn activate_extension_action(&mut self) -> Task<Message> {
        let Page::Extension(page) = &self.page else {
            return Task::none();
        };
        let Some(handler) = page.primary_action().cloned() else {
            return Task::none();
        };
        self.extension_event(page.session, handler.0, page.action_args())
    }

    /// Stops the extension whose view is open, if one is.
    pub(super) fn close_extension_view(&self) -> Task<Message> {
        let (Page::Extension(page), Some(backend)) = (&self.page, self.backend.clone()) else {
            return Task::none();
        };
        let session = page.session;
        Task::future(async move {
            if let Err(error) = backend.close_extension(session).await {
                tracing::warn!(%error, "could not close an extension view");
            }
        })
        .discard()
    }

    /// An extension's form: its fields as the person has them, and how to
    /// submit it.
    pub(super) fn extension_form<'a>(
        &'a self,
        page: &'a crate::extension_page::ExtensionPage,
        form: &'a compass_extension_api::view::FormView,
    ) -> Element<'a, Message> {
        use compass_extension_api::view::{FieldKind, FormItem};
        let mut body = column![].spacing(12).padding(Padding::new(16.0));
        for item in &form.items {
            let field = match item {
                FormItem::Separator { .. } => {
                    body = body.push(iced::widget::rule::horizontal(1));
                    continue;
                }
                FormItem::Description { title, text, .. } => {
                    let mut entry = column![].spacing(4);
                    if let Some(title) = title {
                        entry = entry
                            .push(iced::widget::text(title.clone()).font(self.font()).size(13));
                    }
                    body = body.push(
                        entry.push(iced::widget::text(text.clone()).font(self.font()).size(12)),
                    );
                    continue;
                }
                FormItem::Field(field) => field,
            };
            let name = field.name.clone();
            let value = page.form_values.get(&field.name);
            let text_value = value
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let mut entry = column![].spacing(4);
            if let Some(title) = &field.title {
                entry = entry.push(iced::widget::text(title.clone()).font(self.font()).size(13));
            }
            let input: Element<Message> = match &field.kind {
                FieldKind::TextArea { placeholder, .. } => match page.editors.get(&field.name) {
                    Some(editor) => iced::widget::text_editor(editor)
                        .placeholder(placeholder.as_deref().unwrap_or_default())
                        .font(self.font())
                        .height(Length::Fixed(96.0))
                        .padding(8)
                        .on_action(move |action| {
                            Message::ExtensionTextAreaEdited(name.clone(), action)
                        })
                        .into(),
                    None => iced::widget::text("").into(),
                },
                FieldKind::Text { placeholder } | FieldKind::Password { placeholder } => {
                    text_input(placeholder.as_deref().unwrap_or_default(), text_value)
                        .secure(matches!(field.kind, FieldKind::Password { .. }))
                        .font(self.font())
                        .style(crate::adwaita::form_entry)
                        .on_input(move |text| {
                            Message::ExtensionFieldEdited(name.clone(), text.into())
                        })
                        .padding(8)
                        .into()
                }
                FieldKind::Checkbox { label } => iced::widget::checkbox(
                    value.and_then(serde_json::Value::as_bool).unwrap_or(false),
                )
                .label(label.clone().unwrap_or_default())
                .on_toggle(move |on| Message::ExtensionFieldEdited(name.clone(), on.into()))
                .into(),
                FieldKind::Dropdown(dropdown) => {
                    let options: Vec<(String, String)> = dropdown
                        .sections
                        .iter()
                        .flat_map(|section| &section.options)
                        .map(|option| (option.title.clone(), option.value.clone()))
                        .collect();
                    let titles: Vec<String> = options.iter().map(|(t, _)| t.clone()).collect();
                    let selected = options
                        .iter()
                        .find(|(_, v)| {
                            Some(v.as_str()) == value.and_then(serde_json::Value::as_str)
                        })
                        .map(|(title, _)| title.clone());
                    iced::widget::pick_list(titles, selected, move |title: String| {
                        let chosen = options
                            .iter()
                            .find(|(t, _)| *t == title)
                            .map(|(_, value)| value.clone())
                            .unwrap_or_default();
                        Message::ExtensionFieldEdited(name.clone(), chosen.into())
                    })
                    .font(self.font())
                    .style(crate::adwaita::form_dropdown)
                    .menu_style({
                        let palette = self.palette();
                        move |_: &Theme| crate::design::dropdown_menu(palette)
                    })
                    .into()
                }
                FieldKind::DatePicker { precision, .. } => crate::extension_fields::date_field(
                    &field.name,
                    page.date_shown(&field.name, *precision),
                    *precision,
                    self.font(),
                ),
                FieldKind::TagPicker { options, .. } => crate::extension_fields::tag_field(
                    &field.name,
                    options,
                    &crate::extension_fields::strings(value),
                    self.font(),
                ),
                FieldKind::FilePicker {
                    allow_multiple,
                    allow_directories,
                    allow_files,
                } => crate::extension_fields::file_field(
                    &field.name,
                    &crate::extension_fields::strings(value),
                    crate::extension_fields::FileChoice {
                        multiple: *allow_multiple,
                        directories: *allow_directories,
                        files: *allow_files,
                    },
                    self.font(),
                ),
            };
            entry = entry.push(input);
            if let Some(error) = &field.error {
                entry = entry.push(
                    iced::widget::text(error.clone())
                        .font(self.font())
                        .size(11)
                        .color(self.theme().palette().danger),
                );
            } else if let Some(info) = &field.info {
                entry = entry.push(iced::widget::text(info.clone()).font(self.font()).size(11));
            }
            body = body.push(entry);
        }
        let submit = page
            .actions()
            .and_then(|panel| panel.actions().into_iter().next())
            .map_or_else(
                || "Esc: back".to_owned(),
                |action| {
                    let submit = if page.has_text_area() {
                        "Ctrl+Enter"
                    } else {
                        "Enter"
                    };
                    format!("{submit}: {}    Esc: back", action.title)
                },
            );
        body = body.push(iced::widget::text(submit).font(self.font()).size(12));
        if let Some(notice) = &page.notice {
            body = body.push(iced::widget::text(notice.clone()).font(self.font()));
        }
        scrollable(body)
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }

    /// An extension's page: its view, with its toast underneath.
    pub(super) fn extension_body<'a>(
        &'a self,
        page: &'a crate::extension_page::ExtensionPage,
    ) -> Element<'a, Message> {
        let body = self.extension_view_body(page);
        let Some(toast) = &page.toast else {
            return body;
        };
        let mut line = toast.title.clone();
        if !toast.message.is_empty() {
            line.push_str(" — ");
            line.push_str(&toast.message);
        }
        if toast.animated {
            line.push('…');
        }
        let mut footer = iced::widget::text(line).font(self.font()).size(12);
        if toast.failure {
            footer = footer.color(self.theme().palette().danger);
        }
        column![body, container(footer).padding(Padding::new(6.0).left(14))].into()
    }

    fn extension_view_body<'a>(
        &'a self,
        page: &'a crate::extension_page::ExtensionPage,
    ) -> Element<'a, Message> {
        use crate::extension_page::Status;
        use compass_extension_api::View;
        let geometry = self.geometry;
        if let Some(alert) = &page.alert {
            let mut prompt = column![
                iced::widget::text(alert.title.clone())
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..self.font()
                    })
                    .size(16)
            ]
            .spacing(8)
            .padding(Padding::new(18.0));
            if !alert.message.is_empty() {
                prompt = prompt.push(iced::widget::text(alert.message.clone()).font(self.font()));
            }
            let keys = alert_keys(alert);
            prompt = prompt.push(iced::widget::text(keys).font(self.font()).size(12));
            return prompt.into();
        }
        match (&page.status, &page.view) {
            (Status::Loading, _) => return self.notice("Loading…"),
            (Status::Stopped(why), _) => return self.notice(why),
            (Status::Ready, Some(View::Detail(_))) => {
                // Its images drawn once fetched, as the store page draws a
                // README's.
                let markdown = iced::widget::markdown::view_with(
                    &page.markdown,
                    self.markdown_settings(),
                    &stores::StoreMarkdown {
                        images: &page.markdown_art,
                    },
                );
                let body = scrollable(container(markdown).padding(Padding::new(14.0)))
                    .id(crate::scroll::ROOT_RESULTS)
                    .height(Length::Shrink);
                return match &page.notice {
                    Some(notice) => column![body, self.notice(notice)].into(),
                    None => body.into(),
                };
            }
            (Status::Ready, Some(View::Form(form))) => return self.extension_form(page, form),
            (Status::Ready, Some(View::List(_))) => {}
            (Status::Ready, _) => {
                return self.notice("Compass cannot draw this extension view yet");
            }
        }
        if page.shown.is_empty() {
            let empty = page
                .list()
                .and_then(|list| list.empty_state.as_ref())
                .map_or("No results", |empty| empty.title.as_str());
            return self.notice(empty);
        }
        if page.grid_columns.is_some() {
            return self.extension_grid(page);
        }
        let mut list = column![].spacing(f32::from(geometry.row_spacing));
        let sections = page.list().map(|list| &list.sections[..]).unwrap_or(&[]);
        let mut last_section = None;
        for (position, &(s, i)) in page.shown.iter().enumerate() {
            // Each section under its title, as `List.Section` draws it.
            if last_section != Some(s) {
                last_section = Some(s);
                if let Some(title) = sections[s].title.clone().filter(|t| !t.is_empty()) {
                    list = list.push(self.section_heading(title));
                }
            }
            let item = &sections[s].items[i];
            let selected = position == page.selected;
            let subtitle = crate::extension_page::row_subtitle(item);
            let icon = match page.icon(s, i) {
                Some(icon) => self.masked_icon(icon, page.mask(s, i), selected),
                None => self.initial_badge(&item.title, selected),
            };
            let row = self.list_row(
                icon,
                item.title.clone(),
                subtitle.filter(|_| self.subtitles),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::ExtensionItemSelected(position))
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

    /// An extension's grid: each section under its title, its cells in rows
    /// of the section's columns, each cell its content (an image, a colour)
    /// over its title, as `SectionGridModel` lays them out.
    pub(super) fn extension_grid<'a>(
        &'a self,
        page: &'a crate::extension_page::ExtensionPage,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let sections = page.list().map(|list| &list.sections[..]).unwrap_or(&[]);
        let width = f32::from(self.geometry.card_width) - 24.0;
        let mut grid = column![].spacing(8);
        for (section, positions) in page.grid_groups() {
            if let Some(title) = sections.get(section).and_then(|s| s.title.clone()) {
                grid = grid.push(self.section_heading(title));
            }
            let columns = page.section_columns(section);
            #[allow(clippy::cast_precision_loss)]
            let side = (width / columns as f32 - 8.0).max(16.0);
            for chunk in positions.chunks(columns) {
                let mut cells = row![].spacing(8);
                for &position in chunk {
                    let (s, i) = page.shown[position];
                    let Some(item) = sections.get(s).and_then(|sec| sec.items.get(i)) else {
                        continue;
                    };
                    let selected = position == page.selected;
                    let art: Element<Message> = match page.icon(s, i) {
                        Some(icon) => self.icon_art(icon, page.mask(s, i), selected, side * 0.7),
                        None => Space::new().into(),
                    };
                    let tile = container(art)
                        .width(Length::Fixed(side))
                        .height(Length::Fixed(side))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center)
                        .style(move |_: &Theme| container::Style {
                            background: Some(palette.surface.to_iced().into()),
                            border: Border {
                                color: if selected {
                                    palette.accent.to_iced()
                                } else {
                                    palette.border.to_iced()
                                },
                                width: if selected { 2.0 } else { 1.0 },
                                radius: 8.0.into(),
                            },
                            ..container::Style::default()
                        });
                    let cell = column![
                        tile,
                        text(item.title.clone())
                            .font(self.font())
                            .size(12)
                            .color(palette.text.to_iced())
                            .width(Length::Fixed(side)),
                    ]
                    .spacing(4);
                    let cell: Element<Message> = mouse_area(cell)
                        .on_press(Message::ExtensionItemSelected(position))
                        .into();
                    let cell: Element<Message> = if selected {
                        container(cell).id(crate::scroll::ROOT_SELECTION).into()
                    } else {
                        cell
                    };
                    cells = cells.push(cell);
                }
                grid = grid.push(cells);
            }
        }
        let body = scrollable(container(grid).padding(Padding::new(12.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![body, self.notice(notice)].into(),
            None => body.into(),
        }
    }
}

#[cfg(test)]
mod tests;
