//! The emoji picker in the launcher: its list, its keys, its action panel
//! and the keyword form, over [`crate::emoji_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Alignment, Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelState, Task,
    chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable, text,
};
use crate::backend::{PreferenceInput, PreferenceInputKind};
use crate::emoji_page::{EmojiPage, actions};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};

/// `EditEmojiKeywordsAction`'s description, under the field.
const KEYWORDS_DESCRIPTION: &str = "Additional keywords that will be used to index this glyph";

impl LauncherApp {
    /// The picker's query and its selection.
    pub(super) fn emoji_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::EmojiQueryChanged(query) => {
                if let Page::Emoji(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::EmojiSelected(position) => {
                if let Page::Emoji(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.copy_selected_emoji()
            }
            _ => Task::none(),
        }
    }

    /// Opens Search Emojis & Symbols over what the person has done with it.
    pub(super) fn open_emoji_picker(&mut self) -> Task<Message> {
        let mut page = EmojiPage::open(self.glyph_path.clone(), self.emoji_skin_tone.as_deref());
        page.supports_paste = self.backend.is_some();
        page.default_action = self.emoji_default_action.clone();
        self.page = Page::Emoji(page);
        focus_search()
    }

    /// The picker's keys, below the panel's.
    pub(super) fn emoji_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        if modifiers.control() && key.as_ref() == Key::Character("e") {
            return self.edit_emoji_keywords();
        }
        let Page::Emoji(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => {
                return if page.supports_paste && page.default_action == "paste" {
                    self.paste_selected_emoji()
                } else {
                    self.copy_selected_emoji()
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
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Copies the selected glyph in its tone, counts the pick, and gets out
    /// of the way so it can be pasted where the person was.
    pub(super) fn copy_selected_emoji(&mut self) -> Task<Message> {
        let Page::Emoji(page) = &mut self.page else {
            return Task::none();
        };
        let Some(glyph) = page.selected_glyph() else {
            return Task::none();
        };
        let text = page.display(glyph);
        page.register_visit(glyph);
        self.panel = None;
        self.copy_with_hud(text)
    }

    /// Pastes the selected glyph into the window the launcher hides back to
    /// (`PasteToFocusedWindowAction`), counting the pick; where the engine
    /// cannot paste, it is copied instead.
    pub(super) fn paste_selected_emoji(&mut self) -> Task<Message> {
        let Page::Emoji(page) = &mut self.page else {
            return Task::none();
        };
        let Some(glyph) = page.selected_glyph() else {
            return Task::none();
        };
        let text = page.display(glyph);
        page.register_visit(glyph);
        self.panel = None;
        let Some(backend) = self.backend.clone() else {
            return self.copy_with_hud(text);
        };
        let pasted = text.clone();
        Task::perform(
            async move { backend.paste_text(pasted).await },
            move |result| Message::EmojiPasted {
                text: text.clone(),
                result,
            },
        )
    }

    /// The engine's answer to a paste: hide, or copy where it could not.
    pub(super) fn emoji_pasted(
        &mut self,
        text: String,
        result: Result<(), String>,
    ) -> Task<Message> {
        match result {
            Ok(()) => self.conceal(),
            Err(reason) => {
                tracing::debug!(%reason, "paste refused; copying the glyph instead");
                self.copy_with_hud(text)
            }
        }
    }

    /// The panel over the selected glyph.
    pub(super) fn open_emoji_panel(&mut self) -> Option<Task<Message>> {
        let Page::Emoji(page) = &self.page else {
            return None;
        };
        let sections = page.panel_sections();
        if sections.is_empty() {
            return Some(Task::none());
        }
        self.panel = Some(PanelState::new(sections));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs an emoji panel action, if `id` is one.
    pub(super) fn emoji_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let Page::Emoji(page) = &mut self.page else {
            return None;
        };
        let glyph = page.selected_glyph()?;
        let backend = self.backend.clone();
        let copy = |text: String| {
            Task::batch([super::hud::copy_text(backend.clone(), text), focus_search()])
        };
        let task = match id {
            actions::COPY => return Some(self.copy_selected_emoji()),
            actions::PASTE => return Some(self.paste_selected_emoji()),
            actions::COPY_NAME => copy(glyph.name.to_owned()),
            actions::COPY_CODEPOINT => copy(compass_core::emoji_grid::formatted_codepoint(
                glyph.character,
            )),
            actions::COPY_CATEGORY => copy(glyph.category.label().to_owned()),
            actions::EDIT_KEYWORD => return Some(self.edit_emoji_keywords()),
            actions::RESET_RANKING => {
                page.reset_ranking(glyph);
                focus_search()
            }
            actions::PIN | actions::UNPIN => {
                page.toggle_pin(glyph);
                Task::batch([focus_search(), crate::scroll::reveal_root_selection()])
            }
            actions::TONE_RESET => {
                page.set_tone(glyph, None);
                focus_search()
            }
            other => {
                let tone = other
                    .strip_prefix(actions::TONE)
                    .and_then(compass_core::emoji_grid::skin_tone_by_id)?;
                page.set_tone(glyph, Some(tone));
                focus_search()
            }
        };
        self.panel = None;
        Some(task)
    }

    /// Opens the keyword form for the selected glyph, keeping the picker to
    /// come back to.
    fn edit_emoji_keywords(&mut self) -> Task<Message> {
        let Page::Emoji(page) = &self.page else {
            return Task::none();
        };
        let Some(glyph) = page.selected_glyph() else {
            return Task::none();
        };
        let field = PreferenceInput {
            name: "keywords".to_owned(),
            title: "Keywords".to_owned(),
            description: KEYWORDS_DESCRIPTION.to_owned(),
            placeholder: String::new(),
            required: false,
            kind: PreferenceInputKind::Text,
            value: Some(serde_json::Value::String(page.keywords(glyph))),
        };
        let form = PreferencesPage::new(
            Purpose::GlyphKeywords,
            glyph.character.to_owned(),
            "Edit keyword".to_owned(),
            vec![field],
        );
        self.panel = None;
        let Page::Emoji(page) = std::mem::replace(&mut self.page, Page::Root) else {
            return Task::none();
        };
        self.parked_emoji = Some(page);
        self.page = Page::Preferences(Box::new(form));
        Task::none()
    }

    /// Saves the keyword form and goes back to the picker. `None` when the
    /// form showing is not the keyword form.
    pub(super) fn submit_emoji_keywords(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::GlyphKeywords {
            return None;
        }
        let character = form.command_id.clone();
        let keywords = match form.values.first() {
            Some(FieldValue::Text(text)) => text.clone(),
            _ => String::new(),
        };
        let mut page = self.parked_emoji.take().unwrap_or_default();
        page.set_keywords(&character, &keywords);
        self.page = Page::Emoji(page);
        Some(focus_search())
    }

    /// Escape from the keyword form: back to the picker, as it was.
    pub(super) fn back_from_emoji_keywords(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::GlyphKeywords {
            return None;
        }
        self.page = Page::Emoji(self.parked_emoji.take().unwrap_or_default());
        Some(focus_search())
    }

    /// The picker's rows, for assistive technology: each glyph's name, with
    /// its category as the row's second line says.
    pub(super) fn emoji_semantics(&self, page: &EmojiPage) -> super::a11y::PageSemantics {
        let glyphs = compass_core::glyph::glyphs();
        let items = page
            .shown
            .iter()
            .map(|&index| {
                glyphs
                    .get(index)
                    .map_or_else(crate::semantics::Item::default, |glyph| {
                        super::a11y::item(glyph.name, Some(glyph.category.label().to_owned()))
                    })
            })
            .collect();
        // The notice is the heading over the rows, so it is the view's status.
        super::a11y::PageSemantics::list(
            "Emojis and symbols",
            items,
            page.selected,
            page.empty_notice(),
        )
        .with_status(page.notice.as_deref(), &page.query)
    }

    /// The emoji picker's rows: the character in the icon slot, its name.
    pub(super) fn emoji_body<'a>(
        &'a self,
        page: &'a crate::emoji_page::EmojiPage,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        if let Some(empty) = page.empty_notice() {
            return self.notice(empty);
        }
        let glyphs = compass_core::glyph::glyphs();
        let mut list = column![].spacing(f32::from(geometry.row_spacing));
        if let Some(notice) = &page.notice {
            list = list.push(self.section_heading(notice.clone()));
        }
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(glyph) = glyphs.get(index) else {
                continue;
            };
            if let Some(heading) = page.heading_at(position) {
                list = list.push(self.section_heading(heading.to_owned()));
            }
            let selected = position == page.selected;
            let icon =
                container(text(page.display(glyph)).size(f32::from(geometry.icon_size) * 0.75))
                    .width(Length::Fixed(f32::from(geometry.icon_size)))
                    .height(Length::Fixed(f32::from(geometry.icon_size)))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .into();
            let row = self.list_row(
                icon,
                glyph.name.to_owned(),
                self.subtitles.then(|| glyph.category.label().to_owned()),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::EmojiSelected(position))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RootRow;
    use crate::app::test_support::{TestBackend, app, pressed, settle};
    use crate::semantics::Body;
    use std::sync::Arc;

    #[test]
    fn the_picker_names_each_glyph_with_its_category_and_says_when_none_match() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = EmojiPage::new();
        page.query = "thumbs up".into();
        page.refilter();
        page.notice = Some("The keywords could not be saved.".into());
        let thumbs = page.selected_glyph().expect("a match");
        let semantics = app.emoji_semantics(&page);
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.items.len(), page.shown.len());
        assert_eq!(list.items[0].name, thumbs.name);
        assert_eq!(
            list.items[0].description.as_deref(),
            Some(thumbs.category.label())
        );
        assert_eq!(list.selected, Some(0));
        assert_eq!(
            semantics.status.as_deref(),
            Some("The keywords could not be saved.")
        );

        page.query = "zzzqqqxx".into();
        page.refilter();
        page.notice = None;
        app.page = Page::Emoji(page);
        let semantics = app.semantics();
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(
            semantics.status.as_deref(),
            Some("No matching emojis or symbols")
        );
    }

    #[test]
    fn the_emoji_picker_opens_from_root_search_and_filters() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("emoji".to_owned()));
        let position = app
            .results
            .iter()
            .position(|row| matches!(row, RootRow::Command(c) if c.entrypoint == "search-emojis"))
            .expect("the command is in root search");
        app.selected = position;
        let _ = app.update(Message::LaunchSelected);
        let _ = app.update(Message::EmojiQueryChanged("thumbs up".into()));
        let Page::Emoji(page) = &app.page else {
            panic!("not the picker: {}", app.state_line());
        };
        assert_eq!(page.selected_glyph().map(|g| g.character), Some("👍"));
        let mut ui = iced_test::simulator(app.view());
        assert!(ui.find("thumbs up").is_ok());
    }

    #[test]
    fn the_picker_remembers_a_pick_a_pin_and_a_keyword_in_its_file() {
        use iced::keyboard::key::Named;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("emojis").join("emojis.json");
        let mut app = app(dir.path());
        app.glyph_path = Some(path.clone());
        app.emoji_skin_tone = Some("dark".to_owned());
        let command = compass_core::commands::by_id("commands:search-emojis").expect("command");
        let _ = app.open_command(command);

        // A pick is copied in the picker's tone and counted.
        let _ = app.update(Message::EmojiQueryChanged("waving hand".into()));
        let copied: Vec<String> =
            iced_winit::runtime::task::into_stream(app.update(pressed(Named::Enter)))
                .map(|stream| {
                    iced::futures::executor::block_on(iced::futures::StreamExt::collect::<Vec<_>>(
                        stream,
                    ))
                })
                .unwrap_or_default()
                .into_iter()
                .filter_map(|action| match action {
                    iced_winit::runtime::Action::Clipboard(
                        iced_winit::runtime::clipboard::Action::Write { contents, .. },
                    ) => Some(contents),
                    _ => None,
                })
                .collect();
        assert_eq!(copied, ["👋\u{1F3FF}"]);
        let _ = app.open_command(command);
        let Page::Emoji(page) = &app.page else {
            panic!("not the picker: {}", app.state_line());
        };
        assert_eq!(page.recent, 1, "the pick is recently used");

        // Pin from the panel.
        let _ = app.update(Message::EmojiQueryChanged("pizza".into()));
        let _ = app.update(Message::TogglePanel);
        let pin = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Pin emoji"))
            .expect("the panel offers to pin");
        let _ = app.update(Message::PanelClicked(pin));

        // A keyword of one's own, through the form and back to the picker.
        let _ = app.update(Message::TogglePanel);
        let edit = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Edit keyword"))
            .expect("the panel offers the keyword form");
        let _ = app.update(Message::PanelClicked(edit));
        assert!(
            matches!(&app.page, Page::Preferences(form)
                if form.purpose == crate::preferences_page::Purpose::GlyphKeywords),
            "{}",
            app.state_line()
        );
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("qqsupper".into()),
        ));
        let _ = app.update(Message::PreferencesSubmit);
        let Page::Emoji(page) = &app.page else {
            panic!("back in the picker: {}", app.state_line());
        };
        assert_eq!(page.query, "pizza", "the picker comes back as it was");

        let stored = compass_core::glyph_service::GlyphService::load_file(&path);
        let pizza = stored.find("🍕").expect("pizza is remembered");
        assert!(pizza.pinned_at.is_some());
        assert_eq!(pizza.keyword.as_deref(), Some("qqsupper"));
        assert_eq!(stored.find("👋").map(|wave| wave.visit_count), Some(1));
    }

    #[test]
    fn the_picker_pastes_the_glyph_and_copies_where_the_engine_cannot() {
        use iced::keyboard::key::Named;
        let command = compass_core::commands::by_id("commands:search-emojis").expect("command");
        let picker = |backend: Arc<TestBackend>, default_action: &str| {
            let dir = tempfile::tempdir().expect("tempdir");
            let mut app = app(dir.path());
            app.glyph_path = Some(dir.path().join("emojis.json"));
            app.backend = Some(backend);
            app.emoji_default_action = default_action.to_owned();
            let _ = app.open_command(command);
            let _ = app.update(Message::EmojiQueryChanged("waving hand".into()));
            (app, dir)
        };

        // Paste is the default: first in the panel, on Enter, and nothing is
        // copied by the window.
        let backend = Arc::new(TestBackend::default());
        let (mut app, _dir) = picker(backend.clone(), "paste");
        let _ = app.update(Message::TogglePanel);
        let first = app
            .panel
            .as_ref()
            .and_then(|panel| panel.sections.first()?.actions.first().cloned())
            .expect("an action");
        assert_eq!(first.title, "Paste to active window");
        assert_eq!(first.shortcut.as_deref(), Some("enter"));
        let _ = app.update(Message::TogglePanel);
        let task = app.update(pressed(Named::Enter));
        let copied = settle(&mut app, task);
        assert_eq!(backend.pasted.lock().unwrap().as_slice(), ["👋"]);
        assert!(copied.is_empty(), "{copied:?}");
        let Page::Root = app.page else {
            panic!("the picker hides once pasted: {}", app.state_line());
        };

        // Refused (no Shell extension): the glyph is copied instead.
        let backend = Arc::new(TestBackend {
            refuse_paste: true,
            ..TestBackend::default()
        });
        let (mut app, _dir) = picker(backend.clone(), "paste");
        let task = app.update(pressed(Named::Enter));
        assert_eq!(settle(&mut app, task), ["👋"]);
        assert!(backend.pasted.lock().unwrap().is_empty());

        // `defaultAction: copy` puts copy first and on Enter.
        let backend = Arc::new(TestBackend::default());
        let (mut app, _dir) = picker(backend.clone(), "copy");
        let task = app.update(pressed(Named::Enter));
        assert_eq!(settle(&mut app, task), ["👋"]);
        assert!(backend.pasted.lock().unwrap().is_empty());
        let _ = app.open_command(command);
        let _ = app.update(Message::EmojiQueryChanged("waving hand".into()));
        let _ = app.update(Message::TogglePanel);
        let titles: Vec<String> = app.panel.as_ref().unwrap().sections[0]
            .actions
            .iter()
            .map(|action| action.title.clone())
            .collect();
        assert_eq!(titles[..2], ["Copy", "Paste to active window"]);
    }
}
