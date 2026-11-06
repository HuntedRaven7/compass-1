//! The launcher's forms: one `PreferencesPage` drawn for every purpose it
//! serves -- an extension command's preferences or arguments, a shortcut's,
//! a snippet's or a script's arguments, the shortcut and snippet editors,
//! Create Extension, an alias -- and saving or submitting what was entered.
//!
//! The purpose decides what Enter does with the values; see
//! `crate::preferences_page::Purpose`.

use super::{
    Border, Color, Element, LauncherApp, Length, Message, Padding, Page, Task, Theme, column,
    container, focus_search, scrollable, text_input,
};

impl LauncherApp {
    /// Editing and submitting the form on screen.
    pub(super) fn preferences_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PreferenceEdited(index, value) => {
                if let Page::Preferences(page) = &mut self.page
                    && let Some(slot) = page.values.get_mut(index)
                {
                    *slot = value;
                    page.notice = None;
                    page.focus = Some(index);
                }
                Task::none()
            }
            Message::PreferenceTextEdited(index, action) => {
                if let Page::Preferences(page) = &mut self.page {
                    if action.is_edit() {
                        page.focus = Some(index);
                    }
                    page.edit_text_area(index, action);
                }
                Task::none()
            }
            Message::PreferencesSubmit => {
                if let Some(task) = self.submit_emoji_keywords() {
                    return task;
                }
                if let Some(task) = self.submit_clipboard_keywords() {
                    return task;
                }
                if let Some(task) = self.submit_alias_form() {
                    return task;
                }
                if let Some(task) = self.submit_shortcut_form() {
                    return task;
                }
                if let Some(task) = self.submit_snippet_form() {
                    return task;
                }
                if let Some(task) = self.submit_script_form() {
                    return task;
                }
                if let Some(task) = self.submit_create_extension() {
                    return task;
                }
                if let Some(task) = self.submit_media_form() {
                    return task;
                }
                let Page::Preferences(page) = &mut self.page else {
                    return Task::none();
                };
                let values = match page.submission() {
                    Ok(values) => values,
                    Err(missing) => {
                        page.notice = Some(format!("Fill in {}", missing.join(", ")));
                        return Task::none();
                    }
                };
                if page.purpose == crate::preferences_page::Purpose::Arguments {
                    let id = page.command_id.clone();
                    self.page = Page::Root;
                    return match self.app_index.extensions().iter().position(|c| c.id == id) {
                        Some(index) => self.run_extension_command_with(index, Some(values)),
                        None => Task::none(),
                    };
                }
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                let id = page.command_id.clone();
                Task::perform(
                    async move { backend.set_extension_preferences(id, values).await },
                    Message::PreferencesSaved,
                )
            }
            Message::PreferencesSaved(result) => {
                let Page::Preferences(page) = &mut self.page else {
                    return Task::none();
                };
                if let Err(reason) = result {
                    page.notice = Some(reason);
                    return Task::none();
                }
                let id = page.command_id.clone();
                let only_saved =
                    page.purpose == crate::preferences_page::Purpose::CommandPreferences;
                self.page = Page::Root;
                if only_saved {
                    if let Some(settings) = self.parked_settings.take() {
                        self.page = Page::Settings(settings);
                    }
                    return focus_search();
                }
                match self.app_index.extensions().iter().position(|c| c.id == id) {
                    Some(index) => self.run_extension_command(index),
                    None => Task::none(),
                }
            }
            _ => Task::none(),
        }
    }

    /// A checkbox in a form, ringed in the accent colour while the keyboard
    /// is on it: it cannot take Iced's focus, so it draws none of its own.
    fn form_focus_ring<'a>(
        &self,
        focused: bool,
        field: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let accent = self.palette().accent.to_iced();
        container(field)
            .padding(4)
            .style(move |_: &Theme| container::Style {
                border: Border {
                    color: if focused { accent } else { Color::TRANSPARENT },
                    width: 2.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    pub(super) fn preferences_body<'a>(
        &'a self,
        page: &'a crate::preferences_page::PreferencesPage,
    ) -> Element<'a, Message> {
        use crate::backend::PreferenceInputKind;
        use crate::preferences_page::FieldValue;
        let mut form = column![
            iced::widget::text(match page.purpose {
                crate::preferences_page::Purpose::Preferences => {
                    format!("{} needs a few settings", page.title)
                }
                crate::preferences_page::Purpose::CommandPreferences => {
                    format!("{} settings", page.title)
                }
                crate::preferences_page::Purpose::Arguments
                | crate::preferences_page::Purpose::ShortcutArguments
                | crate::preferences_page::Purpose::ShortcutForm { .. }
                | crate::preferences_page::Purpose::SnippetArguments { .. }
                | crate::preferences_page::Purpose::SnippetForm { .. }
                | crate::preferences_page::Purpose::ScriptArguments
                | crate::preferences_page::Purpose::MediaArguments
                | crate::preferences_page::Purpose::GlyphKeywords
                | crate::preferences_page::Purpose::ClipboardKeywords
                | crate::preferences_page::Purpose::Alias
                | crate::preferences_page::Purpose::CreateExtension => page.title.clone(),
            })
            .font(self.font())
            .size(14)
        ]
        .spacing(12)
        .padding(Padding::new(16.0));
        for (index, (field, value)) in page.fields.iter().zip(&page.values).enumerate() {
            let label = if field.required {
                format!("{} *", field.title)
            } else {
                field.title.clone()
            };
            let mut entry =
                column![iced::widget::text(label).font(self.font()).size(13)].spacing(4);
            let input: Element<Message> = match (&field.kind, value) {
                (
                    PreferenceInputKind::Text | PreferenceInputKind::Password,
                    FieldValue::Text(text),
                ) => text_input(&field.placeholder, text)
                    .id(crate::preferences_page::field_id(index))
                    .secure(matches!(field.kind, PreferenceInputKind::Password))
                    .font(self.font())
                    .style(crate::adwaita::form_entry)
                    .on_input(move |text| Message::PreferenceEdited(index, FieldValue::Text(text)))
                    // No `on_submit`: the key handler submits the form on
                    // Enter, and `listen_with` delivers it whether or not the
                    // field captured it, so a field that also submitted made
                    // every Enter submit twice.
                    .padding(8)
                    .into(),
                (PreferenceInputKind::Checkbox { label }, FieldValue::Checked(checked)) => self
                    .form_focus_ring(
                        page.focus == Some(index),
                        iced::widget::checkbox(*checked)
                            .label(label.clone())
                            .on_toggle(move |checked| {
                                Message::PreferenceEdited(index, FieldValue::Checked(checked))
                            })
                            .into(),
                    ),
                (PreferenceInputKind::Dropdown { options }, FieldValue::Choice(choice)) => {
                    let titles: Vec<String> =
                        options.iter().map(|(title, _)| title.clone()).collect();
                    let selected = choice.as_ref().and_then(|value| {
                        options
                            .iter()
                            .find(|(_, v)| v == value)
                            .map(|(title, _)| title.clone())
                    });
                    let options = options.clone();
                    // Ringed through its own border, so it lines up with the
                    // text fields above it.
                    let ring = (page.focus == Some(index)).then(|| self.palette().accent.to_iced());
                    iced::widget::pick_list(titles, selected, move |title: String| {
                        let value = options
                            .iter()
                            .find(|(t, _)| *t == title)
                            .map(|(_, value)| value.clone());
                        Message::PreferenceEdited(index, FieldValue::Choice(value))
                    })
                    .font(self.font())
                    .menu_style({
                        let palette = self.palette();
                        move |_: &Theme| crate::design::dropdown_menu(palette)
                    })
                    .style(move |theme: &Theme, status| {
                        let mut style = crate::adwaita::form_dropdown(theme, status);
                        if let Some(accent) = ring {
                            style.border.color = accent;
                            style.border.width = 2.0;
                        }
                        style
                    })
                    .into()
                }
                (PreferenceInputKind::TextArea, _) => match page.editors.get(&index) {
                    Some(editor) => iced::widget::text_editor(editor)
                        .id(crate::preferences_page::field_id(index))
                        .placeholder(field.placeholder.as_str())
                        .font(self.font())
                        .height(Length::Fixed(120.0))
                        .padding(8)
                        .on_action(move |action| Message::PreferenceTextEdited(index, action))
                        .into(),
                    None => iced::widget::text("").into(),
                },
                (PreferenceInputKind::Unsupported { declared }, _) => {
                    iced::widget::text(crate::preferences_page::unsupported_note(declared))
                        .font(self.font())
                        .size(12)
                        .into()
                }
                _ => iced::widget::text("").into(),
            };
            entry = entry.push(input);
            if !field.description.is_empty() {
                entry = entry.push(
                    iced::widget::text(field.description.clone())
                        .font(self.font())
                        .size(11),
                );
            }
            form = form.push(entry);
        }
        form = form.push(
            iced::widget::text(page.purpose.hint())
                .font(self.font())
                .size(12),
        );
        if let Some(notice) = &page.notice {
            form = form.push(iced::widget::text(notice.clone()).font(self.font()));
        }
        scrollable(form)
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{
        LaunchedEntries, TestBackend, app, ctrl_enter, extension_app, index, open_builtin, pressed,
        scripts_app, settle, shortcuts_app, task_actions,
    };
    use std::sync::Arc;

    /// One Enter in the form's text field holding `field_text`, delivered as
    /// the runtime delivers it: whatever the field publishes for the press,
    /// then the `KeyPressed` that `listen_with` hands the key handler whether
    /// or not the field captured it.
    fn enter_once_in_form(app: &mut LauncherApp, field_text: &str) {
        let mut messages: Vec<Message> = {
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(800.0, 600.0),
                app.view(),
            );
            ui.click(field_text)
                .unwrap_or_else(|_| panic!("no field holds {field_text:?}: {}", app.state_line()));
            ui.tap_key(iced::keyboard::Key::Named(
                iced::keyboard::key::Named::Enter,
            ));
            ui.into_messages().collect()
        };
        messages.push(pressed(iced::keyboard::key::Named::Enter));
        // Both reach `update` before the engine answers the first: the
        // runtime queues them from the same event, and a save or a run is a
        // round trip to the engine.
        let tasks: Vec<_> = messages
            .into_iter()
            .map(|message| app.update(message))
            .collect();
        for task in tasks {
            settle(app, task);
        }
    }

    #[test]
    fn a_new_form_puts_the_keyboard_on_its_first_field_and_tab_reaches_the_checkbox() {
        use crate::preferences_page::FieldValue;
        use iced::keyboard::key::Named;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        app.query = "create snippet".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        let focusing = task_actions(task);
        let Page::Preferences(page) = &app.page else {
            panic!("no form: {}", app.state_line());
        };
        assert_eq!(page.focus, Some(0), "the name field, as the form opens");
        assert!(!page.focus_pending);
        assert!(focusing > 0, "and a focus operation went out with it");

        let word = page
            .fields
            .iter()
            .position(|f| f.name == crate::snippets_page::WORD_FIELD)
            .unwrap();
        let before = page.values[word].clone();
        for _ in 0..word {
            let _ = app.update(pressed(Named::Tab));
        }
        let Page::Preferences(page) = &app.page else {
            panic!("left the form");
        };
        assert_eq!(page.focus, Some(word), "Tab reaches the checkbox");
        let _ = app.update(pressed(Named::Space));
        let Page::Preferences(page) = &app.page else {
            panic!("left the form");
        };
        assert_ne!(page.values[word], before, "Space ticks it");
        assert!(matches!(page.values[word], FieldValue::Checked(_)));
        let _ = app.update(pressed(Named::Tab));
        let Page::Preferences(page) = &app.page else {
            panic!("left the form");
        };
        assert_eq!(page.focus, Some(0), "and Tab wraps round to the top");
    }

    #[test]
    fn the_alias_and_arguments_forms_focus_their_field_and_say_what_enter_does() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let _ = app.update(Message::TogglePanel);
        let alias = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Set alias"))
            .expect("the panel offers an alias");
        let _ = app.update(Message::PanelClicked(alias));
        let Page::Preferences(page) = &app.page else {
            panic!("no alias form: {}", app.state_line());
        };
        assert_eq!(page.focus, Some(0));
        assert_eq!(
            crate::preferences_page::Purpose::Arguments.hint(),
            "Enter: run    Esc: back",
            "arguments are this run's and are not saved"
        );
    }

    #[test]
    fn one_enter_saves_a_new_shortcut_once() {
        use crate::preferences_page::FieldValue;
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        open_builtin(&mut app, "create shortcut", "commands:create-shortcut");
        let Page::Preferences(page) = &app.page else {
            panic!("no form: {}", app.state_line());
        };
        let position = |name: &str| page.fields.iter().position(|f| f.name == name).unwrap();
        let (name, link) = (position("name"), position("link"));
        let _ = app.update(Message::PreferenceEdited(
            name,
            FieldValue::Text("Wiki".into()),
        ));
        let _ = app.update(Message::PreferenceEdited(
            link,
            FieldValue::Text("https://en.wikipedia.org/wiki/{page}".into()),
        ));
        enter_once_in_form(&mut app, "Wiki");
        assert_eq!(
            backend.drafts.lock().unwrap().len(),
            1,
            "{}",
            app.state_line()
        );
    }

    #[test]
    fn one_enter_opens_a_quicklink_with_arguments_once() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        app.query = "crate docs".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("serde".into()),
        ));
        enter_once_in_form(&mut app, "serde");
        assert_eq!(
            backend.opened_shortcuts.lock().unwrap().as_slice(),
            [("sct-docs".to_owned(), vec!["serde".to_owned()])]
        );
    }

    #[test]
    fn one_enter_runs_a_script_with_arguments_once() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = scripts_app(dir.path());
        app.query = "disk report".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("/home".into()),
        ));
        enter_once_in_form(&mut app, "/home");
        assert_eq!(
            backend.script_runs.lock().unwrap().as_slice(),
            [("report.sh".to_owned(), vec!["/home".to_owned()])]
        );
    }

    #[test]
    fn one_enter_runs_an_extension_command_with_its_arguments_once() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend {
            keys: vec!["@someone/hello:write".to_owned()],
            wants: vec![crate::backend::PreferenceInput {
                name: "name".into(),
                title: "Name".into(),
                description: String::new(),
                placeholder: "Name".into(),
                required: true,
                kind: crate::backend::PreferenceInputKind::Text,
                value: None,
            }],
            ..TestBackend::default()
        });
        let mut app = extension_app(dir.path(), backend.clone());
        let task = app.update(Message::QueryChanged("greeting".into()));
        settle(&mut app, task);
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("Ada".into()),
        ));
        enter_once_in_form(&mut app, "Ada");
        assert_eq!(
            backend.given.lock().unwrap().len(),
            2,
            "the first try, which asked for the argument, and one run with it"
        );
    }

    #[test]
    fn one_enter_in_the_alias_form_saves_it_and_launches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = Arc::new(LaunchedEntries::default());
        let mut app = app(dir.path()).with_launcher(launcher.clone());
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let firefox = app.selected_row().expect("a row");
        let id = app.root_id(firefox).expect("a root item");
        let _ = app.update(Message::TogglePanel);
        let alias = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Set alias"))
            .expect("the panel offers an alias");
        let task = app.update(Message::PanelClicked(alias));
        settle(&mut app, task);
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("ff".into()),
        ));
        enter_once_in_form(&mut app, "ff");
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
        assert_eq!(
            app.app_index
                .root(&id)
                .and_then(|root| root.meta.alias.as_deref()),
            Some("ff")
        );
        assert!(
            launcher.0.lock().unwrap().is_empty(),
            "saving the alias launched the selected row too"
        );
    }

    #[test]
    fn one_ctrl_enter_saves_a_snippet_once_and_a_plain_enter_none() {
        use crate::preferences_page::FieldValue;
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());
        open_builtin(&mut app, "create snippet", "commands:create-snippet");
        let Page::Preferences(page) = &app.page else {
            panic!("no snippet form: {}", app.state_line());
        };
        let position = |name: &str| page.fields.iter().position(|f| f.name == name).unwrap();
        let (name, content) = (position("name"), position("content"));
        let _ = app.update(Message::PreferenceEdited(
            name,
            FieldValue::Text("Thanks".into()),
        ));
        let _ = app.update(Message::PreferenceTextEdited(
            content,
            iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Paste(
                Arc::new("Thank you!".into()),
            )),
        ));
        let before = backend.snippet_drafts.lock().unwrap().len();
        enter_once_in_form(&mut app, "Thanks");
        assert_eq!(
            backend.snippet_drafts.lock().unwrap().len(),
            before,
            "Enter is a newline in a form with a text area"
        );
        let task = app.update(ctrl_enter());
        settle(&mut app, task);
        assert_eq!(backend.snippet_drafts.lock().unwrap().len(), before + 1);
    }
}
