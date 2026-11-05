//! The launcher's description of itself for assistive technology
//! ([`crate::semantics`]), and what it does when assistive technology asks
//! something of it.
//!
//! What is described: the root search (its field, results, selection and
//! status), the action panel, the questions asked before an action, an
//! extension's list, alert and form, and the preference and argument forms.
//! Every other view describes its search field and nothing under it yet.

use accesskit::{Action, ActionData, ActionRequest};
use iced::Task;

use super::{
    Fallback, LauncherApp, Page, RootRow, alert_keys, answer_keys, focus_search, form_focus,
};
use crate::message::Message;
use crate::semantics::{
    Body, Dialog, Field, FieldKind, FieldTarget, Form, Item, List, Menu, MenuItem, SearchField,
    Semantics, Target, result_count,
};

impl LauncherApp {
    /// Keeps the platform's accessibility tree in step with the window
    /// ([`crate::accessibility`]); nothing when the binary installed none.
    pub(super) fn follow_accessibility(&mut self) {
        let open = self.window.is_some() && !self.closing;
        let focused = self.window_focused;
        let mut tree = std::mem::take(&mut self.a11y);
        tree.follow(open, focused, || self.semantics());
        self.a11y = tree;
    }

    /// The window as assistive technology should understand it.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        let (placeholder, value, on_input) = self.search_field();
        let search = match &self.page {
            Page::Onboarding(_) | Page::StoreIntro(_) => None,
            // The root field stays the root field while the action panel
            // over it takes the typing.
            page => Some(SearchField {
                placeholder: placeholder.to_owned(),
                value: value.to_owned(),
                editable: on_input.is_some() || matches!(page, Page::Root),
            }),
        };
        let (body, status) = self.body_semantics();
        Semantics {
            window: self.title(),
            search,
            body,
            status,
            panel: self.panel_semantics(),
        }
    }

    fn body_semantics(&self) -> (Body, Option<String>) {
        if let Some(confirm) = &self.confirm {
            let dialog = Dialog {
                title: confirm.title.clone(),
                message: confirm.message.clone(),
                keys: answer_keys(&confirm.confirm_text),
            };
            return (Body::Dialog(dialog), None);
        }
        if let Some(power) = self.power_confirm {
            let dialog = Dialog {
                title: compass_core::power_commands::CONFIRM_TITLE.to_owned(),
                message: compass_core::power_commands::CONFIRM_BODY.to_owned(),
                keys: answer_keys(power.name),
            };
            return (Body::Dialog(dialog), None);
        }
        match &self.page {
            Page::Root => self.root_semantics(),
            Page::Extension(page) => extension_semantics(page),
            Page::Preferences(page) => (Body::Form(preferences_form(page)), page.notice.clone()),
            _ => (Body::Empty, None),
        }
    }

    fn root_semantics(&self) -> (Body, Option<String>) {
        if let Some(error) = &self.error {
            return (Body::Empty, Some(error.clone()));
        }
        if self.results.is_empty() {
            let status = if self.query.is_empty() {
                "Type to search"
            } else {
                "No results"
            };
            return (Body::Empty, Some(status.to_owned()));
        }
        let items: Vec<Item> = self
            .results
            .iter()
            .map(|row| self.root_item(*row).unwrap_or_default())
            .collect();
        // The count only once something is typed: it answers the query, and
        // an empty query's favourites are not an answer to anything.
        let status = (!self.query.is_empty()).then(|| result_count(items.len()));
        let list = List {
            label: "Results".to_owned(),
            items,
            selected: Some(self.selected),
        };
        (Body::List(list), status)
    }

    /// A root row's title and subtitle, as the row shows them; `None` for a
    /// row whose item has gone.
    fn root_item(&self, row: RootRow) -> Option<Item> {
        let (name, description) = match row {
            RootRow::App(index) => {
                let item = self.app_index.items().get(index)?;
                (item.name().to_owned(), item.comment().map(str::to_owned))
            }
            RootRow::Command(command) => {
                (command.title.to_owned(), Some(command.subtitle.to_owned()))
            }
            RootRow::Update => {
                let offer = self.update.as_ref()?;
                (
                    super::release_check::title(offer),
                    Some(super::release_check::subtitle(offer)),
                )
            }
            RootRow::Calculator => {
                let answer = self.calculator.as_ref()?;
                (answer.answer.clone(), Some(answer.question.clone()))
            }
            RootRow::Extension(index) => {
                let command = self.app_index.extensions().get(index)?;
                let subtitle = self
                    .extension_subtitles
                    .get(&command.id)
                    .unwrap_or(&command.extension_title);
                (command.title.clone(), Some(subtitle.clone()))
            }
            RootRow::Script(index) => {
                let script = self.app_index.scripts().get(index)?;
                (script.title.clone(), Some(script.subtitle.clone()))
            }
            RootRow::RhaiScript(index) => {
                let script = self.app_index.rhai_scripts().get(index)?;
                (script.title.clone(), Some(script.subtitle().to_owned()))
            }
            RootRow::Fallback(fallback) => {
                let subtitle = match fallback {
                    Fallback::Command(command) => command.subtitle.to_owned(),
                    Fallback::Extension(_) => "Command".to_owned(),
                    Fallback::Shortcut(_) => "Shortcut".to_owned(),
                };
                (self.fallback_title(fallback)?, Some(subtitle))
            }
            RootRow::Shortcut(index) => {
                let shortcut = self.app_index.shortcuts().get(index)?;
                (
                    crate::shortcuts_page::display_name(shortcut).to_owned(),
                    Some("Shortcut".to_owned()),
                )
            }
        };
        Some(Item {
            name,
            description: description.filter(|d| !d.is_empty()),
        })
    }

    fn panel_semantics(&self) -> Option<Menu> {
        let panel = self
            .panel
            .as_ref()
            .filter(|panel| panel.recorder.is_none())?;
        let mut menu = Menu::default();
        for (row_index, row) in panel.rows.iter().enumerate() {
            let Some(section) = panel.sections.get(row.section) else {
                continue;
            };
            let Some(action) = row.action.and_then(|a| section.actions.get(a)) else {
                continue;
            };
            if usize::try_from(panel.selected).ok() == Some(row_index) {
                menu.selected = Some(menu.items.len());
            }
            menu.items.push(MenuItem {
                name: action.title.clone(),
                section: (!section.name.is_empty()).then(|| section.name.clone()),
                shortcut: action.shortcut.clone(),
                row: row_index,
            });
        }
        Some(menu)
    }

    /// Does what assistive technology asked of the tree, as the keyboard or
    /// a click would have done it.
    ///
    /// Node ids are positions, and the tree is brought up to date after every
    /// update, so the window described now is the one the request was about.
    pub fn accessibility_request(&mut self, request: ActionRequest) -> Task<Message> {
        let semantics = self.semantics();
        let Some(target) = semantics.target(request.target_node) else {
            return Task::none();
        };
        let field = match target {
            Target::Field(index) => match &semantics.body {
                Body::Form(form) => form.fields.get(index).cloned(),
                _ => None,
            },
            _ => None,
        };
        let menu_row = match target {
            Target::MenuItem(index) => semantics
                .panel
                .as_ref()
                .and_then(|menu| menu.items.get(index))
                .map(|item| item.row),
            _ => None,
        };
        let value = match request.data {
            Some(ActionData::Value(value)) => Some(value.into_string()),
            _ => None,
        };
        match (target, request.action) {
            (Target::Search, Action::Focus) => focus_search(),
            (Target::Search, Action::SetValue | Action::ReplaceSelectedText) => {
                let (_, current, on_input) = self.search_field();
                let (Some(on_input), Some(value)) = (on_input, value) else {
                    return Task::none();
                };
                // The caret is said to be at the end, so a replacement of
                // the (empty) selection appends.
                let text = if request.action == Action::ReplaceSelectedText {
                    format!("{current}{value}")
                } else {
                    value
                };
                self.update(on_input(text))
            }
            (Target::Item(position), Action::Click) => match &self.page {
                Page::Root => self.update(Message::ResultClicked(position)),
                Page::Extension(_) => self.update(Message::ExtensionItemSelected(position)),
                _ => Task::none(),
            },
            (Target::Item(position), Action::Focus) => match &mut self.page {
                Page::Root => {
                    self.selected = position;
                    crate::scroll::reveal_root_selection()
                }
                Page::Extension(page) => {
                    page.selected = position;
                    crate::scroll::reveal_root_selection()
                }
                _ => Task::none(),
            },
            (Target::MenuItem(_), Action::Click) => match menu_row {
                Some(row) => self.update(Message::PanelClicked(row)),
                None => Task::none(),
            },
            (Target::MenuItem(_), Action::Focus) => {
                if let (Some(panel), Some(Ok(row))) =
                    (self.panel.as_mut(), menu_row.map(isize::try_from))
                {
                    panel.selected = row;
                }
                crate::scroll::reveal_panel_selection()
            }
            (Target::Field(_), action) => match field {
                Some(field) => self.field_request(field, action, value),
                None => Task::none(),
            },
            _ => Task::none(),
        }
    }

    fn field_request(
        &mut self,
        field: Field,
        action: Action,
        value: Option<String>,
    ) -> Task<Message> {
        use crate::preferences_page::FieldValue;
        match (field.target, field.kind, action) {
            (FieldTarget::Preference(index), _, Action::Focus) => {
                let Page::Preferences(page) = &mut self.page else {
                    return Task::none();
                };
                page.focus = Some(index);
                form_focus(page)
            }
            (FieldTarget::Preference(index), FieldKind::Checkbox { checked }, Action::Click) => {
                self.update(Message::PreferenceEdited(
                    index,
                    FieldValue::Checked(!checked),
                ))
            }
            (
                FieldTarget::Preference(index),
                FieldKind::Text {
                    multiline: false, ..
                },
                Action::SetValue,
            ) => match value {
                Some(value) => {
                    self.update(Message::PreferenceEdited(index, FieldValue::Text(value)))
                }
                None => Task::none(),
            },
            (FieldTarget::Extension(name), FieldKind::Date { .. }, Action::SetValue) => match value
            {
                Some(value) => self.update(Message::ExtensionDateEdited(name, value)),
                None => Task::none(),
            },
            (FieldTarget::Extension(name), FieldKind::Checkbox { checked }, Action::Click) => {
                self.update(Message::ExtensionFieldEdited(name, (!checked).into()))
            }
            (
                FieldTarget::Extension(name),
                FieldKind::Text {
                    multiline: false, ..
                },
                Action::SetValue,
            ) => match value {
                Some(value) => self.update(Message::ExtensionFieldEdited(name, value.into())),
                None => Task::none(),
            },
            _ => Task::none(),
        }
    }
}

fn extension_semantics(page: &crate::extension_page::ExtensionPage) -> (Body, Option<String>) {
    use crate::extension_page::Status;
    use compass_extension_api::View;
    if let Some(alert) = &page.alert {
        let dialog = Dialog {
            title: alert.title.clone(),
            message: alert.message.clone(),
            keys: alert_keys(alert),
        };
        return (Body::Dialog(dialog), None);
    }
    match (&page.status, &page.view) {
        (Status::Loading, _) => (Body::Empty, Some("Loading…".to_owned())),
        (Status::Stopped(why), _) => (Body::Empty, Some(why.clone())),
        (Status::Ready, Some(View::Form(form))) => {
            (Body::Form(extension_form(page, form)), page.notice.clone())
        }
        (Status::Ready, Some(View::List(list))) => {
            let items: Vec<Item> = page
                .shown
                .iter()
                .filter_map(|&(s, i)| list.sections.get(s)?.items.get(i))
                .map(|item| Item {
                    name: item.title.clone(),
                    description: crate::extension_page::row_subtitle(item),
                })
                .collect();
            let status = if let Some(notice) = &page.notice {
                Some(notice.clone())
            } else if items.is_empty() && list.is_loading {
                Some("Loading…".to_owned())
            } else if items.is_empty() {
                Some(
                    list.empty_state
                        .as_ref()
                        .map_or("No results", |empty| empty.title.as_str())
                        .to_owned(),
                )
            } else {
                (!page.query.is_empty()).then(|| result_count(items.len()))
            };
            let label = list
                .navigation_title
                .clone()
                .unwrap_or_else(|| page.title.clone());
            let selected = (!items.is_empty()).then_some(page.selected);
            (
                Body::List(List {
                    label,
                    items,
                    selected,
                }),
                status,
            )
        }
        (Status::Ready, _) => (Body::Empty, page.notice.clone()),
    }
}

fn extension_form(
    page: &crate::extension_page::ExtensionPage,
    form: &compass_extension_api::view::FormView,
) -> Form {
    use compass_extension_api::view::{FieldKind as Kind, FormItem};
    let mut fields = Vec::new();
    for item in &form.items {
        let field = match item {
            FormItem::Separator { .. } => continue,
            FormItem::Description { title, text, .. } => {
                fields.push(Field {
                    label: title.clone().unwrap_or_default(),
                    kind: FieldKind::Note { text: text.clone() },
                    description: None,
                    required: false,
                    invalid: false,
                    target: FieldTarget::None,
                });
                continue;
            }
            FormItem::Field(field) => field,
        };
        let value = page.form_values.get(&field.name);
        let text = || {
            value
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let mut label = field.title.clone().unwrap_or_default();
        let kind = match &field.kind {
            Kind::Text { placeholder } | Kind::Password { placeholder } => FieldKind::Text {
                value: text(),
                placeholder: placeholder.clone(),
                secret: matches!(field.kind, Kind::Password { .. }),
                multiline: false,
            },
            Kind::TextArea { placeholder, .. } => FieldKind::Text {
                value: text(),
                placeholder: placeholder.clone(),
                secret: false,
                multiline: true,
            },
            Kind::Checkbox { label: beside } => {
                if label.is_empty() {
                    label = beside.clone().unwrap_or_default();
                }
                FieldKind::Checkbox {
                    checked: value.and_then(serde_json::Value::as_bool).unwrap_or(false),
                }
            }
            Kind::Dropdown(dropdown) => FieldKind::Dropdown {
                value: dropdown
                    .sections
                    .iter()
                    .flat_map(|section| &section.options)
                    .find(|option| {
                        Some(option.value.as_str()) == value.and_then(serde_json::Value::as_str)
                    })
                    .map(|option| option.title.clone()),
            },
            Kind::DatePicker { precision, .. } => FieldKind::Date {
                value: page.date_shown(&field.name, *precision),
                format: crate::extension_fields::date_format(*precision).to_owned(),
            },
            Kind::TagPicker { options, .. } => {
                let chosen = crate::extension_fields::strings(value);
                FieldKind::Tags {
                    options: options
                        .iter()
                        .map(|option| (option.title.clone(), chosen.contains(&option.value)))
                        .collect(),
                }
            }
            Kind::FilePicker { .. } => FieldKind::Files {
                chosen: crate::extension_fields::strings(value),
            },
        };
        fields.push(Field {
            label,
            kind,
            description: field.error.clone().or_else(|| field.info.clone()),
            required: false,
            invalid: field.error.is_some(),
            target: FieldTarget::Extension(field.name.clone()),
        });
    }
    Form {
        label: form
            .navigation_title
            .clone()
            .unwrap_or_else(|| page.title.clone()),
        fields,
        focused: None,
    }
}

fn preferences_form(page: &crate::preferences_page::PreferencesPage) -> Form {
    use crate::backend::PreferenceInputKind;
    use crate::preferences_page::FieldValue;
    let fields = page
        .fields
        .iter()
        .zip(&page.values)
        .enumerate()
        .map(|(index, (field, value))| {
            let mut label = field.title.clone();
            let placeholder = (!field.placeholder.is_empty()).then(|| field.placeholder.clone());
            let kind = match (&field.kind, value) {
                (PreferenceInputKind::Text, FieldValue::Text(text)) => FieldKind::Text {
                    value: text.clone(),
                    placeholder,
                    secret: false,
                    multiline: false,
                },
                (PreferenceInputKind::Password, FieldValue::Text(text)) => FieldKind::Text {
                    value: text.clone(),
                    placeholder,
                    secret: true,
                    multiline: false,
                },
                (PreferenceInputKind::TextArea, FieldValue::Text(text)) => FieldKind::Text {
                    value: text.clone(),
                    placeholder,
                    secret: false,
                    multiline: true,
                },
                (PreferenceInputKind::Checkbox { label: beside }, FieldValue::Checked(checked)) => {
                    if label.is_empty() {
                        label = beside.clone();
                    }
                    FieldKind::Checkbox { checked: *checked }
                }
                (PreferenceInputKind::Dropdown { options }, FieldValue::Choice(choice)) => {
                    FieldKind::Dropdown {
                        value: choice.as_ref().and_then(|choice| {
                            options
                                .iter()
                                .find(|(_, value)| value == choice)
                                .map(|(title, _)| title.clone())
                        }),
                    }
                }
                (PreferenceInputKind::Unsupported { declared }, _) => FieldKind::Note {
                    text: crate::preferences_page::unsupported_note(declared),
                },
                _ => FieldKind::Note {
                    text: String::new(),
                },
            };
            Field {
                label,
                kind,
                description: (!field.description.is_empty()).then(|| field.description.clone()),
                required: field.required,
                invalid: false,
                target: FieldTarget::Preference(index),
            }
        })
        .collect();
    Form {
        label: page.title.clone(),
        fields,
        focused: page.focus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantics::FocusTracker;
    use accesskit::{NodeId, Role, TreeId};
    use compass_core::AppIndex;

    fn app(dir: &std::path::Path) -> LauncherApp {
        for (file, name, comment) in [
            ("firefox.desktop", "Firefox", "Web Browser"),
            ("files.desktop", "Files", "Access and organize files"),
            ("terminal.desktop", "Terminal", "Use the command line"),
        ] {
            std::fs::write(
                dir.join(file),
                format!(
                    "[Desktop Entry]\nType=Application\nName={name}\nComment={comment}\nExec=/bin/true\n"
                ),
            )
            .expect("write entry");
        }
        LauncherApp::with_index(AppIndex::builder().dir(dir).build())
    }

    fn list(semantics: &Semantics) -> &List {
        match &semantics.body {
            Body::List(list) => list,
            other => panic!("expected a list, got {other:?}"),
        }
    }

    fn request(node: NodeId, action: Action, data: Option<ActionData>) -> Message {
        Message::Accessibility(ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node: node,
            data,
        })
    }

    /// The node `semantics` gives `role` and `label`, as a request names it.
    fn node_of(semantics: &Semantics, role: Role, label: &str) -> NodeId {
        semantics
            .tree_update(NodeId(1))
            .nodes
            .into_iter()
            .find(|(_, node)| node.role() == role && node.label() == Some(label))
            .unwrap_or_else(|| panic!("no {role:?} called {label}"))
            .0
    }

    #[test]
    fn an_empty_launcher_is_a_search_field_and_says_to_type() {
        let dir = tempfile::tempdir().expect("tempdir");
        let semantics = app(dir.path()).semantics();
        assert_eq!(semantics.window, "Compass");
        let search = semantics.search.as_ref().expect("a search field");
        assert!(search.editable);
        assert_eq!(search.value, "");
        assert_eq!(search.placeholder, "Search…");
        assert_eq!(semantics.status.as_deref(), Some("Type to search"));
        assert_eq!(semantics.body, Body::Empty);
        assert_eq!(semantics.panel, None);
    }

    #[test]
    fn a_query_describes_its_results_their_count_and_the_selection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".into()));
        let semantics = app.semantics();
        assert_eq!(
            semantics.search.as_ref().map(|s| s.value.as_str()),
            Some("fi")
        );
        let results = list(&semantics);
        let names: Vec<&str> = results.items.iter().map(|i| i.name.as_str()).collect();
        assert!(
            names.contains(&"Firefox") && names.contains(&"Files"),
            "{names:?}"
        );
        let firefox = results
            .items
            .iter()
            .find(|i| i.name == "Firefox")
            .expect("Firefox");
        assert_eq!(firefox.description.as_deref(), Some("Web Browser"));
        assert_eq!(results.selected, Some(0));
        assert_eq!(
            semantics.status,
            Some(crate::semantics::result_count(results.items.len()))
        );

        let _ = app.update(Message::MoveSelection(crate::Direction::Down));
        assert_eq!(list(&app.semantics()).selected, Some(1));
    }

    #[test]
    fn no_results_is_said_once_and_an_error_replaces_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("qqqqzzzz".into()));
        let semantics = app.semantics();
        assert_eq!(semantics.status.as_deref(), Some("No results"));
        assert_eq!(semantics.body, Body::Empty);
        app.error = Some("Firefox could not be opened.".into());
        assert_eq!(
            app.semantics().status.as_deref(),
            Some("Firefox could not be opened.")
        );
    }

    #[test]
    fn the_action_panel_is_described_as_a_menu_with_its_current_action() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let _ = app.update(Message::TogglePanel);
        let semantics = app.semantics();
        let menu = semantics.panel.as_ref().expect("the panel is open");
        let names: Vec<&str> = menu.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names.first(), Some(&"Open"), "{names:?}");
        assert!(names.contains(&"Copy name"), "{names:?}");
        assert_eq!(menu.selected, Some(0));
        assert_eq!(menu.items[0].shortcut.as_deref(), Some("enter"));
        let copy = menu
            .items
            .iter()
            .find(|i| i.name == "Copy name")
            .expect("copy");
        assert_eq!(copy.section.as_deref(), Some("Copy"));
        // The panel has the focus while it is open.
        let mut focus = FocusTracker::new();
        let focused = focus.focus(&semantics);
        assert_eq!(focused, node_of(&semantics, Role::MenuItem, "Open"));
        // Still the root field underneath, not a heading.
        assert!(semantics.search.as_ref().is_some_and(|s| s.editable));
    }

    #[test]
    fn requests_do_what_the_keyboard_would() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());

        // Setting the field's text is typing it.
        let field = node_of(&app.semantics(), Role::SearchInput, "Search…");
        let _ = app.update(request(
            field,
            Action::SetValue,
            Some(ActionData::Value("fi".into())),
        ));
        assert_eq!(app.query, "fi");
        let _ = app.update(request(
            field,
            Action::ReplaceSelectedText,
            Some(ActionData::Value("r".into())),
        ));
        assert_eq!(app.query, "fir");

        // Focusing a result selects it.
        let _ = app.update(Message::QueryChanged("f".into()));
        let semantics = app.semantics();
        let second = list(&semantics).items[1].name.clone();
        let _ = app.update(request(
            node_of(&semantics, Role::ListBoxOption, &second),
            Action::Focus,
            None,
        ));
        assert_eq!(app.selected, 1);

        // A press on a panel action runs it, which closes the panel.
        let _ = app.update(Message::TogglePanel);
        let open = node_of(&app.semantics(), Role::MenuItem, "Copy name");
        let _ = app.update(request(open, Action::Click, None));
        assert!(app.panel.is_none());

        // A request about a node that is not there does nothing.
        let before = app.semantics();
        let _ = app.update(request(NodeId(u64::MAX), Action::Click, None));
        assert_eq!(app.semantics(), before);
    }

    fn preference(
        name: &str,
        required: bool,
        kind: crate::backend::PreferenceInputKind,
    ) -> crate::backend::PreferenceInput {
        crate::backend::PreferenceInput {
            name: name.to_owned(),
            title: name.to_uppercase(),
            description: String::new(),
            placeholder: String::new(),
            required,
            kind,
            value: None,
        }
    }

    #[test]
    fn a_preference_form_has_its_controls_in_order_and_follows_the_keyboard() {
        use crate::backend::PreferenceInputKind;
        use crate::preferences_page::{PreferencesPage, Purpose};
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let mut page = PreferencesPage::new(
            Purpose::Preferences,
            "@a/b:c".into(),
            "GitHub".into(),
            vec![
                preference("token", true, PreferenceInputKind::Password),
                preference(
                    "private",
                    false,
                    PreferenceInputKind::Checkbox {
                        label: "Private".into(),
                    },
                ),
                preference(
                    "sort",
                    false,
                    PreferenceInputKind::Dropdown {
                        options: vec![("Stars".into(), "stars".into())],
                    },
                ),
            ],
        );
        page.focus = Some(1);
        app.page = Page::Preferences(Box::new(page));

        let semantics = app.semantics();
        let Body::Form(form) = &semantics.body else {
            panic!("expected a form, got {:?}", semantics.body);
        };
        assert_eq!(form.label, "GitHub");
        assert_eq!(form.focused, Some(1));
        assert!(matches!(
            form.fields[0].kind,
            FieldKind::Text { secret: true, .. }
        ));
        assert!(form.fields[0].required);
        assert_eq!(form.fields[1].kind, FieldKind::Checkbox { checked: false });
        assert_eq!(form.fields[2].kind, FieldKind::Dropdown { value: None });
        // The field the keyboard is on is the one said to have the focus.
        let focused = FocusTracker::new().focus(&semantics);
        assert_eq!(focused, node_of(&semantics, Role::CheckBox, "PRIVATE"));

        // Pressing the checkbox ticks it.
        let _ = app.update(request(focused, Action::Click, None));
        let Body::Form(form) = app.semantics().body else {
            panic!("still a form");
        };
        assert_eq!(form.fields[1].kind, FieldKind::Checkbox { checked: true });
    }

    #[test]
    fn an_extension_form_names_each_control_and_its_value() {
        use compass_extension_api::view::{
            DropdownOption, FieldKind as Kind, FormField, FormItem, FormView, View,
        };
        let field = |name: &str, title: &str, kind: Kind| {
            FormItem::Field(Box::new(FormField {
                id: Default::default(),
                name: name.to_owned(),
                title: Some(title.to_owned()),
                error: None,
                info: None,
                autofocus: false,
                value: None,
                echo: None,
                on_change: None,
                kind,
            }))
        };
        let form = FormView {
            items: vec![
                field("title", "Title", Kind::Text { placeholder: None }),
                FormItem::Description {
                    id: Default::default(),
                    title: None,
                    text: "Saved to your notes.".to_owned(),
                },
                field("done", "Done", Kind::Checkbox { label: None }),
                field(
                    "tags",
                    "Tags",
                    Kind::TagPicker {
                        options: vec![DropdownOption {
                            title: "Work".to_owned(),
                            value: "work".to_owned(),
                            icon: None,
                            keywords: Vec::new(),
                        }],
                        placeholder: None,
                    },
                ),
                field(
                    "files",
                    "Attachments",
                    Kind::FilePicker {
                        allow_multiple: true,
                        allow_directories: false,
                        allow_files: true,
                    },
                ),
            ],
            ..FormView::default()
        };
        let mut page = crate::extension_page::ExtensionPage::new(1, "Quick Note");
        page.apply(crate::backend::ExtensionViewState {
            version: 1,
            view: Some(Box::new(View::Form(form))),
            depth: 1,
            ..Default::default()
        });
        page.form_values
            .insert("title".to_owned(), serde_json::json!("Groceries"));
        page.form_values
            .insert("tags".to_owned(), serde_json::json!(["work"]));
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        app.page = Page::Extension(Box::new(page));

        let semantics = app.semantics();
        let Body::Form(form) = &semantics.body else {
            panic!("expected a form, got {:?}", semantics.body);
        };
        assert_eq!(form.label, "Quick Note");
        let labels: Vec<&str> = form.fields.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["Title", "", "Done", "Tags", "Attachments"]);
        assert!(matches!(
            &form.fields[0].kind,
            FieldKind::Text { value, .. } if value == "Groceries"
        ));
        assert_eq!(
            form.fields[1].kind,
            FieldKind::Note {
                text: "Saved to your notes.".to_owned()
            }
        );
        assert_eq!(
            form.fields[3].kind,
            FieldKind::Tags {
                options: vec![("Work".to_owned(), true)]
            }
        );
        assert_eq!(form.fields[4].kind, FieldKind::Files { chosen: vec![] });

        // Setting a text field's value is typing it.
        let title = node_of(&semantics, Role::TextInput, "Title");
        let _ = app.update(request(
            title,
            Action::SetValue,
            Some(ActionData::Value("Bills".into())),
        ));
        let Page::Extension(page) = &app.page else {
            panic!("still the extension");
        };
        assert_eq!(
            page.form_values.get("title"),
            Some(&serde_json::json!("Bills"))
        );
    }

    #[test]
    fn a_question_is_an_alert_with_the_keys_that_answer_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        app.confirm = Some(super::super::Confirm {
            title: "Clear clipboard history?".into(),
            message: "This cannot be undone.".into(),
            confirm_text: "Clear".into(),
            action: super::super::ConfirmAction::ClipboardRemoveAll,
        });
        let semantics = app.semantics();
        assert_eq!(
            semantics.body,
            Body::Dialog(Dialog {
                title: "Clear clipboard history?".into(),
                message: "This cannot be undone.".into(),
                keys: "Enter: Clear    Esc: cancel".into(),
            })
        );
        assert_eq!(
            FocusTracker::new().focus(&semantics),
            node_of(&semantics, Role::AlertDialog, "Clear clipboard history?")
        );
    }
}
