//! Browse Fonts in the launcher: the list, its category filter, and the
//! specimen.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the rest is in [`crate::fonts_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, row, scrollable, text,
};
use crate::action_panel::Action;
use crate::fonts_page::{self, FontPreviewPage, FontsPage, GridMove, SpecimenLine, Status};
use compass_core::font_browser::COLUMNS;

const PREVIEW: &str = "font.preview";
const COPY_FAMILY: &str = "font.copy-family";
const SET_APP_FONT: &str = "font.set-app-font";

/// A tile's height; the C++ grid's cells are square (`ASPECT_RATIO`), and a
/// sixth of the card's width is about this.
const TILE_HEIGHT: f32 = 104.0;

impl LauncherApp {
    /// Opens Browse Fonts and asks for the families.
    pub(super) fn open_browse_fonts(&mut self) -> Task<Message> {
        let page = self.parked_fonts.take().unwrap_or_default();
        let loaded = page.status == Status::Ready;
        self.page = Page::Fonts(page);
        if loaded {
            return focus_search();
        }
        let Some(backend) = self.backend.clone() else {
            if let Page::Fonts(page) = &mut self.page {
                page.apply(Err("Browse Fonts needs the Compass engine".to_owned()));
            }
            return focus_search();
        };
        Task::batch([
            Task::perform(
                async move { backend.list_fonts().await },
                Message::FontsLoaded,
            ),
            focus_search(),
        ])
    }

    /// Opens the selected family's specimen.
    fn preview_selected_font(&mut self) -> Task<Message> {
        let Page::Fonts(page) = &self.page else {
            return Task::none();
        };
        let (Some(family), Some(backend)) = (page.selected_family(), self.backend.clone()) else {
            return Task::none();
        };
        let (name, family) = (family.name.clone(), family.family.clone());
        self.panel = None;
        Task::perform(
            {
                let name = name.clone();
                async move { backend.font_specimen(name).await }
            },
            move |result| Message::FontSpecimenLoaded {
                name: name.clone(),
                family: family.clone(),
                result,
            },
        )
    }

    /// The panel over the selected family: `FontBrowserViewHost`'s actions.
    pub(super) fn open_font_panel(&mut self) -> Option<Task<Message>> {
        let Page::Fonts(page) = &self.page else {
            return None;
        };
        page.selected_family()?;
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new(compass_core::font_browser::PREVIEW_TITLE)
                    .with_id(PREVIEW)
                    .with_shortcut("enter"),
                Action::new(compass_core::font_browser::COPY_FAMILY_TITLE).with_id(COPY_FAMILY),
                Action::new(compass_core::font_browser::SET_APP_FONT_TITLE).with_id(SET_APP_FONT),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a font panel action, if `id` is one.
    pub(super) fn font_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let Page::Fonts(page) = &self.page else {
            return None;
        };
        let task = match id {
            PREVIEW => self.preview_selected_font(),
            COPY_FAMILY => {
                let family = page.selected_family()?.family.clone();
                self.panel = None;
                return Some(Task::batch([
                    super::hud::copy_text(self.backend.clone(), family),
                    self.conceal(),
                ]));
            }
            SET_APP_FONT => {
                let family = page.selected_family()?.family.clone();
                self.panel = None;
                let Some(backend) = self.backend.clone() else {
                    if let Page::Fonts(page) = &mut self.page {
                        page.notice = Some("Setting the font needs the Compass engine".to_owned());
                    }
                    return Some(Task::none());
                };
                return Some(Task::batch([
                    Task::perform(
                        {
                            let family = family.clone();
                            async move { backend.set_font(family).await }
                        },
                        move |result| Message::FontSet(result.map(|()| family.clone())),
                    ),
                    focus_search(),
                ]));
            }
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// The list's keys.
    pub(super) fn fonts_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Fonts(page) = &mut self.page else {
            return Task::none();
        };
        let step = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(GridMove::Down),
            Key::Named(Named::ArrowUp) => Some(GridMove::Up),
            Key::Named(Named::ArrowLeft) => Some(GridMove::Left),
            Key::Named(Named::ArrowRight) => Some(GridMove::Right),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.preview_selected_font(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers).map(|direction| {
                match direction {
                    Direction::Up => GridMove::Up,
                    Direction::Down => GridMove::Down,
                }
            }),
        };
        if let Some(step) = step {
            page.selected = fonts_page::grid_step(
                page.shown().1.len(),
                page.selected,
                COLUMNS,
                step,
                self.wrap_navigation,
            );
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// The specimen's keys: Escape goes back to the list, as it was.
    pub(super) fn font_preview_key(&mut self, key: &Key) -> Task<Message> {
        if key.as_ref() == Key::Named(Named::Escape) {
            return self.open_browse_fonts();
        }
        Task::none()
    }

    /// Handles the Browse Fonts messages.
    pub(super) fn font_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FontsLoaded(result) => {
                let saved = self
                    .view
                    .view_memory
                    .get(crate::view_memory::FONT_CATEGORY)
                    .map(str::to_owned);
                if let Page::Fonts(page) = &mut self.page {
                    page.apply(result);
                    // `restoreCategoryFilter`: a remembered category some
                    // font still has.
                    if let Some(index) =
                        compass_core::font_browser::index_for_saved(&page.options, saved.as_deref())
                    {
                        let option = page.options[index].clone();
                        page.set_category(&option);
                    }
                }
                crate::scroll::reveal_root_selection()
            }
            Message::FontSet(result) => {
                match result {
                    Ok(family) => {
                        // The configured family replaces the desktop's.
                        self.view.typography_link = None;
                        self.view.font_family = Some(family);
                    }
                    Err(reason) => {
                        if let Page::Fonts(page) = &mut self.page {
                            page.notice = Some(reason);
                        }
                    }
                }
                Task::none()
            }
            Message::FontsQueryChanged(query) => {
                if let Page::Fonts(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::FontsCategoryChanged(option) => {
                if let Page::Fonts(page) = &mut self.page {
                    page.set_category(&option);
                }
                self.view
                    .view_memory
                    .set(crate::view_memory::FONT_CATEGORY, &option);
                Task::batch([focus_search(), crate::scroll::reveal_root_selection()])
            }
            Message::FontSelected(position) => {
                if let Page::Fonts(page) = &mut self.page
                    && position < page.shown().1.len()
                {
                    page.selected = position;
                    return self.preview_selected_font();
                }
                Task::none()
            }
            Message::FontSpecimenLoaded {
                name,
                family,
                result: Ok(markdown),
            } => {
                if let Page::Fonts(page) = &self.page {
                    self.parked_fonts = Some(page.clone());
                }
                self.page = Page::FontPreview(FontPreviewPage {
                    name,
                    family,
                    lines: fonts_page::specimen_lines(&markdown),
                });
                Task::none()
            }
            Message::FontSpecimenLoaded {
                result: Err(reason),
                ..
            } => {
                if let Page::Fonts(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The list's body: the category filter, the heading, and each family
    /// named in its own font beside its glyph.
    pub(super) fn fonts_body<'a>(&'a self, page: &'a FontsPage) -> Element<'a, Message> {
        let empty = page.empty_notice();
        if let Some(notice) = empty
            && page.status != Status::Ready
        {
            return self.notice(notice);
        }
        let palette = self.palette();
        let (title, families) = page.shown();
        let selected_option = page
            .category
            .clone()
            .unwrap_or_else(|| compass_core::font_browser::ALL_OPTION.to_owned());
        let filter = iced::widget::pick_list(
            page.options.clone(),
            Some(selected_option),
            Message::FontsCategoryChanged,
        )
        .text_size(12)
        .font(self.font())
        .style(move |_, status| crate::design::dropdown(palette, status))
        .menu_style(move |_| crate::design::dropdown_menu(palette));
        let heading = text(title.to_owned())
            .font(self.font())
            .size(12)
            .color(palette.muted.to_iced());
        let mut list = column![
            container(row![
                heading,
                iced::widget::Space::new().width(Length::Fill),
                filter
            ])
            .padding(Padding::new(4.0).left(10).right(10))
        ]
        .spacing(f32::from(self.geometry.row_spacing));
        if let Some(notice) = empty {
            return column![list, self.notice(notice)].into();
        }
        for (row_index, chunk) in families.chunks(COLUMNS).enumerate() {
            let mut line = row![].spacing(6);
            for (column, family) in chunk.iter().enumerate() {
                let position = row_index * COLUMNS + column;
                line = line.push(self.font_tile(family, position == page.selected, position));
            }
            for _ in chunk.len()..COLUMNS {
                line = line.push(iced::widget::Space::new().width(Length::Fill));
            }
            list = list.push(line);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// The tiles, for assistive technology: a grid of [`COLUMNS`] to a row,
    /// under the heading the view shows.
    pub(super) fn fonts_semantics(&self, page: &FontsPage) -> super::a11y::PageSemantics {
        let (title, families) = page.shown();
        let items = families
            .iter()
            .map(|family| super::a11y::item(family.name.clone(), None))
            .collect();
        let mut semantics =
            super::a11y::PageSemantics::list(title, items, page.selected, page.empty_notice())
                .with_status(page.notice.as_deref(), &page.query);
        if let crate::semantics::Body::List(list) = &mut semantics.body {
            list.grid = Some(crate::semantics::List::grid_of(list.items.len(), COLUMNS));
        }
        semantics
    }

    /// One tile of the grid: the family's glyph drawn in it, its name under.
    fn font_tile<'a>(
        &'a self,
        family: &'a compass_core::font_browser::FontFamily,
        selected: bool,
        position: usize,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let preview = compass_core::font_browser::preview(family);
        let glyph_font = if preview.family.is_empty() {
            self.font()
        } else {
            iced::Font::with_name(fonts_page::static_family(&family.family))
        };
        let colour = if selected {
            palette.selection_text
        } else {
            palette.text
        };
        let tile = column![
            container(text(preview.glyph).font(glyph_font).size(34).color(
                if preview.fill_foreground {
                    colour.to_iced()
                } else {
                    palette.text.to_iced()
                }
            ))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::Alignment::Center)
            .align_y(iced::Alignment::Center),
            text(family.name.clone())
                .font(self.font())
                .size(11)
                .color(if selected {
                    palette.selection_text.to_iced()
                } else {
                    palette.muted.to_iced()
                })
                .wrapping(iced::widget::text::Wrapping::None)
                .width(Length::Fill)
                .align_x(iced::Alignment::Center),
        ]
        .spacing(4);
        let tile = container(tile)
            .width(Length::Fill)
            .height(Length::Fixed(TILE_HEIGHT))
            .padding(6)
            .clip(true)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(
                    if selected {
                        palette.selection
                    } else {
                        palette.field
                    }
                    .to_iced()
                    .into(),
                ),
                // The field fill is one step from the card (1.04:1 in
                // light), so a tile is outlined in the control colour to be
                // seen as one at all.
                border: iced::Border {
                    radius: 8.0.into(),
                    width: if selected { 0.0 } else { 1.0 },
                    color: palette.control().to_iced(),
                },
                ..container::Style::default()
            });
        let tile: Element<'a, Message> = mouse_area(tile)
            .on_press(Message::FontSelected(position))
            .into();
        if selected {
            container(tile)
                .id(crate::scroll::ROOT_SELECTION)
                .width(Length::Fill)
                .into()
        } else {
            tile
        }
    }

    /// The specimen's body, drawn in the font.
    pub(super) fn font_preview_body<'a>(
        &'a self,
        page: &'a FontPreviewPage,
    ) -> Element<'a, Message> {
        let family = fonts_page::static_family(&page.family);
        let font = iced::Font::with_name(family);
        let palette = self.palette();
        let mut body = column![
            text(page.title())
                .font(self.font())
                .size(12)
                .color(palette.muted.to_iced())
        ]
        .spacing(8);
        for line in &page.lines {
            let element: Element<Message> = match line {
                SpecimenLine::Heading(sample) => text(sample.clone()).font(font).size(28).into(),
                SpecimenLine::Regular(sample) => text(sample.clone()).font(font).size(16).into(),
                SpecimenLine::Bold(sample) => text(sample.clone())
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..font
                    })
                    .size(16)
                    .into(),
                SpecimenLine::Italic(sample) => text(sample.clone())
                    .font(iced::Font {
                        style: iced::font::Style::Italic,
                        ..font
                    })
                    .size(16)
                    .into(),
                SpecimenLine::Rule => iced::widget::rule::horizontal(1).into(),
            };
            body = body.push(element);
        }
        scrollable(container(body).padding(Padding::new(14.0)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }

    /// The specimen, for assistive technology: its samples as text, under
    /// the title the view shows above them.
    pub(super) fn font_preview_semantics(
        &self,
        page: &FontPreviewPage,
    ) -> super::a11y::PageSemantics {
        let blocks = page
            .lines
            .iter()
            .filter_map(|line| {
                // The view's title is the document's label, so a sample's
                // heading is one level under it.
                let (sample, heading) = match line {
                    SpecimenLine::Heading(sample) => (sample, Some(2)),
                    SpecimenLine::Regular(sample)
                    | SpecimenLine::Bold(sample)
                    | SpecimenLine::Italic(sample) => (sample, None),
                    SpecimenLine::Rule => return None,
                };
                Some(crate::semantics::Block {
                    text: sample.clone(),
                    heading,
                })
            })
            .collect();
        super::a11y::PageSemantics {
            body: crate::semantics::Body::Document(crate::semantics::Document {
                label: page.title(),
                blocks,
                metadata: Vec::new(),
            }),
            status: None,
            detail: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{TestBackend, index, open_builtin, pressed, settle};
    use crate::backend::{FontList, FontListEntry};
    use crate::semantics::{Block, Body};
    use std::sync::Arc;

    fn fonts(names: &[&str]) -> FontList {
        let font = |name: &&str| FontListEntry {
            name: (*name).to_owned(),
            family: (*name).to_owned(),
            glyph: Some("Aa".into()),
            color: false,
            primary: "Latin".into(),
            categories: vec!["Latin".into()],
        };
        FontList {
            fonts: names.iter().map(font).collect(),
            categories: vec!["Latin".into()],
        }
    }

    #[test]
    fn the_fonts_are_a_grid_of_rows_as_the_tiles_are_laid_out() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = FontsPage::default();
        assert_eq!(
            app.fonts_semantics(&page).status.as_deref(),
            Some("Reading the installed fonts…")
        );
        page.apply(Ok(fonts(&[
            "Font A", "Font B", "Font C", "Font D", "Font E", "Font F", "Font G", "Font H",
        ])));
        page.selected = 7;
        app.page = Page::Fonts(page);
        let semantics = app.semantics();
        let Body::List(list) = &semantics.body else {
            panic!("expected a grid, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "All Fonts (8)");
        assert_eq!(list.items.len(), 8);
        assert_eq!(list.items[0].name, "Font A");
        assert_eq!(list.selected, Some(7));
        assert_eq!(
            list.grid,
            Some(vec![vec![0, 1, 2, 3, 4, 5], vec![6, 7]]),
            "six tiles to a row, the last one short"
        );

        let Page::Fonts(page) = &mut app.page else {
            unreachable!("the page was just set");
        };
        page.query = "%%%%".into();
        page.refilter();
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(semantics.status.as_deref(), Some("No fonts match"));
    }

    #[test]
    fn the_specimen_is_a_document_of_its_samples_without_the_rules() {
        let app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let page = FontPreviewPage {
            name: "Inter".into(),
            family: "Inter".into(),
            lines: fonts_page::specimen_lines(
                "# Latin\nThe quick brown fox\n**Bold sample**\n---\n*Italic sample*",
            ),
        };
        let semantics = app.font_preview_semantics(&page);
        let Body::Document(document) = &semantics.body else {
            panic!("expected a document, got {:?}", semantics.body);
        };
        assert_eq!(document.label, "Browse Fonts - Inter");
        let block = |text: &str, heading| Block {
            text: text.into(),
            heading,
        };
        assert_eq!(
            document.blocks,
            [
                block("Latin", Some(2)),
                block("The quick brown fox", None),
                block("Bold sample", None),
                block("Italic sample", None),
            ]
        );
    }

    #[test]
    fn browse_fonts_filters_previews_and_goes_back_to_the_same_list() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend);
        open_builtin(&mut app, "browse fonts", "commands:browse-fonts");
        let Page::Fonts(page) = &app.page else {
            panic!("not Browse Fonts: {}", app.state_line());
        };
        assert_eq!(page.shown().0, "All Fonts (2)");
        assert_eq!(page.options, ["All", "Latin", "Monospace"]);

        let task = app.update(Message::FontsCategoryChanged("Monospace".into()));
        settle(&mut app, task);
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        settle(&mut app, task);
        let Page::FontPreview(preview) = &app.page else {
            panic!("no specimen: {}", app.state_line());
        };
        assert_eq!(preview.name, "JetBrains Mono");
        assert_eq!(
            preview.lines.first(),
            Some(&crate::fonts_page::SpecimenLine::Heading(
                "JetBrains Mono".into()
            ))
        );

        let task = app.update(pressed(iced::keyboard::key::Named::Escape));
        settle(&mut app, task);
        let Page::Fonts(page) = &app.page else {
            panic!("not back at the list: {}", app.state_line());
        };
        assert_eq!(page.category.as_deref(), Some("Monospace"), "filter kept");

        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let _ = app.update(Message::PanelMove(Direction::Down));
        let task = app.update(Message::PanelActivate);
        assert_eq!(
            settle(&mut app, task),
            ["JetBrains Mono"],
            "Copy font family"
        );
    }

    #[test]
    fn browse_fonts_is_a_grid_that_remembers_its_category_and_sets_the_font() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.view.view_memory =
            crate::view_memory::ViewMemory::load(Some(dir.path().join("view-state.json")));
        open_builtin(&mut app, "browse fonts", "commands:browse-fonts");
        let task = app.update(pressed(iced::keyboard::key::Named::ArrowRight));
        settle(&mut app, task);
        let Page::Fonts(page) = &app.page else {
            panic!("not Browse Fonts: {}", app.state_line());
        };
        assert_eq!(page.selected, 1, "Right moves along the row");
        let _ = app.view();

        let task = app.update(Message::FontsCategoryChanged("Monospace".into()));
        settle(&mut app, task);
        let task = app.update(Message::TogglePanel);
        settle(&mut app, task);
        let _ = app.update(Message::PanelMove(Direction::Down));
        let _ = app.update(Message::PanelMove(Direction::Down));
        let task = app.update(Message::PanelActivate);
        settle(&mut app, task);
        assert_eq!(
            backend.fonts_set.lock().unwrap().as_slice(),
            ["JetBrains Mono"]
        );
        assert_eq!(app.view.font_family.as_deref(), Some("JetBrains Mono"));

        let task = app.update(Message::Dismiss);
        settle(&mut app, task);
        let mut reopened = LauncherApp::with_index(index(dir.path()));
        reopened.backend = Some(backend);
        reopened.view.view_memory =
            crate::view_memory::ViewMemory::load(Some(dir.path().join("view-state.json")));
        open_builtin(&mut reopened, "browse fonts", "commands:browse-fonts");
        let Page::Fonts(page) = &reopened.page else {
            panic!("not Browse Fonts: {}", reopened.state_line());
        };
        assert_eq!(
            page.category.as_deref(),
            Some("Monospace"),
            "the category is remembered across processes"
        );
    }
}
