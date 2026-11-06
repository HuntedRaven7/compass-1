//! The launcher's description of itself for assistive technology
//! ([`crate::semantics`]), and what it does when assistive technology asks
//! something of it.
//!
//! Every view is described: the root search (its field, results, selection
//! and status), the action panel, the questions asked before an action, an
//! extension's list, grid, detail, alert and form, the preference and
//! argument forms, the settings, and each builtin view's list or text, with
//! the preview or detail beside a list. Each builtin view's description is
//! next to its drawing, in its own module (`*_semantics`), so the two are
//! changed together.

use accesskit::{Action, ActionData, ActionRequest};
use iced::Task;

use super::{
    Fallback, LauncherApp, Page, RootRow, alert_keys, answer_keys, focus_search, form_focus,
};
use crate::message::Message;
use crate::semantics::{
    Body, Dialog, Document, Field, FieldKind, FieldTarget, Form, Item, List, Menu, MenuItem,
    Metadata, SearchField, Semantics, Target, markdown_blocks, result_count,
};

/// What a view shows under the search field, for assistive technology.
#[derive(Debug, Clone, Default)]
pub(super) struct PageSemantics {
    /// The list, document or form.
    pub body: Body,
    /// What the view says about its state: loading, empty, failed, a count.
    pub status: Option<String>,
    /// The pane beside the list.
    pub detail: Option<Document>,
}

impl PageSemantics {
    /// A list of `items` under `label`, with `selected` selected, or the
    /// `empty` notice in its place when the view shows one.
    pub(super) fn list(
        label: &str,
        items: Vec<Item>,
        selected: usize,
        empty: Option<&str>,
    ) -> Self {
        if let Some(empty) = empty {
            return Self {
                status: Some(empty.to_owned()),
                ..Self::default()
            };
        }
        let selected = (selected < items.len()).then_some(selected);
        Self {
            body: Body::List(List {
                label: label.to_owned(),
                items,
                selected,
                grid: None,
            }),
            ..Self::default()
        }
    }

    /// The same, with the view's `notice` as its status, or the count when
    /// `query` is not empty and there is no notice.
    #[must_use]
    pub(super) fn with_status(mut self, notice: Option<&str>, query: &str) -> Self {
        if self.status.is_some() {
            return self;
        }
        self.status = match (notice, &self.body) {
            (Some(notice), _) => Some(notice.to_owned()),
            (None, Body::List(list)) if !query.is_empty() => Some(result_count(list.items.len())),
            _ => None,
        };
        self
    }

    /// The same, with `detail` beside the list.
    #[must_use]
    pub(super) fn with_detail(mut self, detail: Option<Document>) -> Self {
        if matches!(self.body, Body::List(_)) {
            self.detail = detail;
        }
        self
    }
}

/// An item named `name` and described by `description`, when it says
/// something.
pub(super) fn item(name: impl Into<String>, description: Option<String>) -> Item {
    Item {
        name: name.into(),
        description: description.filter(|d| !d.is_empty()),
    }
}

/// Metadata from `(label, value)` pairs, leaving out the empty ones.
pub(super) fn metadata<L: Into<String>, V: Into<String>>(
    pairs: impl IntoIterator<Item = (L, V)>,
) -> Vec<Metadata> {
    pairs
        .into_iter()
        .map(|(label, value)| Metadata {
            label: label.into(),
            value: value.into(),
        })
        .filter(|entry| !entry.value.is_empty())
        .collect()
}

/// A file's preview as a document: the start of a text file, line by line,
/// or what the pane shows in place of an image or anything else, then its
/// metadata unless `metadata` is off (dmenu's `--no-metadata`).
pub(super) fn file_preview_document(
    preview: &crate::file_preview::FilePreview,
    metadata: bool,
) -> Document {
    use crate::file_preview::Content;
    let blocks = match &preview.content {
        Content::Text(text) => crate::semantics::text_blocks(text),
        // An image is described by its name, which the document is labelled
        // with; the pane shows nothing but the type for anything else.
        Content::Image(_) => Vec::new(),
        Content::None => crate::semantics::text_blocks(&preview.mime),
    };
    Document {
        label: preview.name.clone(),
        blocks,
        metadata: if metadata {
            self::metadata(preview.fields())
        } else {
            Vec::new()
        },
    }
}

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
        let PageSemantics {
            body,
            status,
            detail,
        } = self.body_semantics();
        Semantics {
            window: self.title(),
            search,
            body,
            status,
            panel: self.panel_semantics(),
            detail,
        }
    }

    fn body_semantics(&self) -> PageSemantics {
        let dialog = |dialog: Dialog| PageSemantics {
            body: Body::Dialog(dialog),
            ..PageSemantics::default()
        };
        if let Some(confirm) = &self.confirm {
            return dialog(Dialog {
                title: confirm.title.clone(),
                message: confirm.message.clone(),
                keys: answer_keys(&confirm.confirm_text),
            });
        }
        if let Some(power) = self.power_confirm {
            return dialog(Dialog {
                title: compass_core::power_commands::CONFIRM_TITLE.to_owned(),
                message: compass_core::power_commands::CONFIRM_BODY.to_owned(),
                keys: answer_keys(power.name),
            });
        }
        match &self.page {
            Page::Root => self.root_semantics(),
            Page::Extension(page) => extension_semantics(page),
            Page::Preferences(page) => PageSemantics {
                body: Body::Form(preferences_form(page)),
                status: page.notice.clone(),
                detail: None,
            },
            Page::Settings(page) => self.settings_semantics(page),
            Page::Onboarding(page) => self.onboarding_semantics(page),
            Page::Clipboard(page) => self.clipboard_semantics(page),
            Page::Windows(page) => self.windows_semantics(page),
            Page::Workspaces(page) => self.workspaces_semantics(page),
            Page::OpenWith(page) => self.open_with_semantics(page),
            Page::Emoji(page) => self.emoji_semantics(page),
            Page::Files(page) => self.files_semantics(page),
            Page::Shortcuts(page) => self.shortcuts_semantics(page),
            Page::Snippets(page) => self.snippets_semantics(page),
            Page::ScriptOutput(page) => self.script_output_semantics(page),
            Page::Programs(page) => self.programs_semantics(page),
            Page::Dmenu(page) => self.dmenu_semantics(page),
            Page::Themes(page) => self.themes_semantics(page),
            Page::Created(page) => self.created_semantics(page),
            Page::Fonts(page) => self.fonts_semantics(page),
            Page::FontPreview(page) => self.font_preview_semantics(page),
            Page::Store(page) => self.store_semantics(page),
            Page::StoreDetail(page) => self.store_detail_semantics(page),
            Page::NowPlaying(page) => self.now_playing_semantics(page),
            Page::Grants(page) => self.grants_semantics(page),
            Page::Tray(page) => self.tray_semantics(page),
            Page::Apps(page) => self.apps_semantics(page),
            Page::Calculator(page) => self.calculator_semantics(page),
            Page::Fallbacks(page) => self.fallbacks_semantics(page),
            Page::Extensions(page) => self.extensions_semantics(page),
            Page::Icons(page) => self.icons_semantics(page),
            Page::Storage(page) => self.storage_semantics(page),
            Page::Tokens(page) => self.tokens_semantics(page),
            Page::StoreIntro(page) => self.store_intro_semantics(page),
        }
    }

    fn root_semantics(&self) -> PageSemantics {
        if let Some(error) = &self.error {
            return PageSemantics {
                status: Some(error.clone()),
                ..PageSemantics::default()
            };
        }
        // Typing clears the list and the engine answers later, and its answer
        // is described once it has stood a moment (`results_settling`);
        // meanwhile what is described is what was there (`Semantics::settle`).
        if (self.search_task.is_some() && self.results.is_empty()) || self.results_settling {
            return PageSemantics {
                body: Body::Pending,
                ..PageSemantics::default()
            };
        }
        if self.results.is_empty() {
            let status = if self.query.is_empty() {
                "Type to search"
            } else {
                "No results"
            };
            return PageSemantics {
                status: Some(status.to_owned()),
                ..PageSemantics::default()
            };
        }
        let items: Vec<Item> = self
            .results
            .iter()
            .map(|row| self.root_item(*row).unwrap_or_default())
            .collect();
        // The count only once something is typed: it answers the query, and
        // an empty query's favourites are not an answer to anything.
        PageSemantics::list("Results", items, self.selected, None).with_status(None, &self.query)
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
        let (field, option) = match target {
            Target::Field(index) => (index, None),
            Target::Option { field, option } => (field, Some(option)),
            _ => (usize::MAX, None),
        };
        let field = semantics
            .form()
            .and_then(|form| form.fields.get(field))
            .cloned();
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
            (Target::Item(position), action @ (Action::Click | Action::Focus))
                if matches!(self.page, Page::Settings(_)) =>
            {
                self.settings_sidebar_request(position, action)
            }
            (Target::Item(position), Action::Click) => match self.item_click(position) {
                Some(message) => self.update(message),
                None => Task::none(),
            },
            (Target::Item(position), Action::Focus) => {
                if let Some(selected) = self.selection_mut() {
                    *selected = position;
                }
                crate::scroll::reveal_root_selection()
            }
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
            (Target::Field(_) | Target::Option { .. }, action) => match field {
                Some(field) => self.field_request(field, option, action, value),
                None => Task::none(),
            },
            _ => Task::none(),
        }
    }

    /// What clicking the `position`th item of the view's list sends, as a
    /// click on its row does; `None` for a view without one.
    fn item_click(&self, position: usize) -> Option<Message> {
        Some(match &self.page {
            Page::Root => Message::ResultClicked(position),
            Page::Extension(_) => Message::ExtensionItemSelected(position),
            Page::Clipboard(_) => Message::ClipboardSelected(position),
            Page::Windows(_) => Message::WindowSelected(position),
            Page::Workspaces(_) => Message::WorkspaceSelected(position),
            Page::OpenWith(_) => Message::OpenWithSelected(position),
            Page::Emoji(_) => Message::EmojiSelected(position),
            Page::Files(_) => Message::FilesSelected(position),
            Page::Shortcuts(_) => Message::ShortcutSelected(position),
            Page::Snippets(_) => Message::SnippetSelected(position),
            Page::Programs(_) => Message::ProgramSelected(position),
            Page::Dmenu(_) => Message::DmenuSelected(position),
            Page::Themes(_) => Message::ThemeSelected(position),
            Page::Fonts(_) => Message::FontSelected(position),
            Page::Store(_) => Message::StoreSelected(position),
            Page::NowPlaying(_) => Message::NowPlayingSelected(position),
            Page::Grants(_) => Message::GrantSelected(position),
            Page::Tray(_) => Message::TraySelected(position),
            Page::Apps(_) => Message::AppsSelected(position),
            Page::Calculator(_) => Message::CalculatorSelected(position),
            Page::Fallbacks(_) => Message::FallbackSelected(position),
            Page::Extensions(_) | Page::Icons(_) | Page::Storage(_) | Page::Tokens(_) => {
                Message::CompassRowSelected(position)
            }
            Page::ScriptOutput(_)
            | Page::Created(_)
            | Page::FontPreview(_)
            | Page::StoreDetail(_)
            | Page::StoreIntro(_)
            | Page::Preferences(_)
            | Page::Settings(_)
            | Page::Onboarding(_) => return None,
        })
    }

    /// The view's selected row, which moving the selection moves.
    fn selection_mut(&mut self) -> Option<&mut usize> {
        Some(match &mut self.page {
            Page::Root => &mut self.selected,
            Page::Extension(page) => &mut page.selected,
            Page::Clipboard(page) => &mut page.selected,
            Page::Windows(page) => &mut page.selected,
            Page::Workspaces(page) => &mut page.selected,
            Page::OpenWith(page) => &mut page.selected,
            Page::Emoji(page) => &mut page.selected,
            Page::Files(page) => &mut page.selected,
            Page::Shortcuts(page) => &mut page.selected,
            Page::Snippets(page) => &mut page.selected,
            Page::Programs(page) => &mut page.selected,
            Page::Dmenu(page) => &mut page.selected,
            Page::Themes(page) => &mut page.selected,
            Page::Fonts(page) => &mut page.selected,
            Page::Store(page) => &mut page.selected,
            Page::NowPlaying(page) => &mut page.selected,
            Page::Grants(page) => &mut page.selected,
            Page::Tray(page) => &mut page.selected,
            Page::Apps(page) => &mut page.selected,
            Page::Calculator(page) => &mut page.selected,
            Page::Fallbacks(page) => &mut page.selected,
            Page::Extensions(page) => &mut page.selected,
            Page::Icons(page) => &mut page.selected,
            Page::Storage(page) => &mut page.selected,
            Page::Tokens(page) => &mut page.selected,
            Page::ScriptOutput(_)
            | Page::Created(_)
            | Page::FontPreview(_)
            | Page::StoreDetail(_)
            | Page::StoreIntro(_)
            | Page::Preferences(_)
            | Page::Settings(_)
            | Page::Onboarding(_) => return None,
        })
    }

    fn field_request(
        &mut self,
        field: Field,
        option: Option<usize>,
        action: Action,
        value: Option<String>,
    ) -> Task<Message> {
        use crate::preferences_page::FieldValue;
        match (field.target, field.kind, action) {
            (FieldTarget::Control(position), _, action) => match &self.page {
                Page::Settings(_) => self.settings_request(position, option, action, value),
                Page::Onboarding(_) => self.onboarding_request(position, option, action),
                _ => Task::none(),
            },
            (FieldTarget::Preference(index), FieldKind::Dropdown { .. }, Action::Click) => {
                let Page::Preferences(page) = &self.page else {
                    return Task::none();
                };
                let chosen = match (page.fields.get(index).map(|f| &f.kind), option) {
                    (
                        Some(crate::backend::PreferenceInputKind::Dropdown { options }),
                        Some(option),
                    ) => options.get(option).map(|(_, value)| value.clone()),
                    _ => None,
                };
                match chosen {
                    Some(chosen) => self.update(Message::PreferenceEdited(
                        index,
                        FieldValue::Choice(Some(chosen)),
                    )),
                    None => Task::none(),
                }
            }
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
            (
                FieldTarget::Extension(name),
                FieldKind::Dropdown { .. } | FieldKind::Tags { .. } | FieldKind::Files { .. },
                Action::Click,
            ) => match self.extension_choice(&name, option) {
                Some(message) => self.update(message),
                None => Task::none(),
            },
            _ => Task::none(),
        }
    }

    /// What choosing `option` of the extension form's field `name` sends, as
    /// clicking it does: a dropdown's value, a tag toggled, or the file
    /// chooser for a file picker (which has no options).
    fn extension_choice(&self, name: &str, option: Option<usize>) -> Option<Message> {
        use compass_extension_api::View;
        use compass_extension_api::view::{FieldKind as Kind, FormItem};
        let Page::Extension(page) = &self.page else {
            return None;
        };
        let Some(View::Form(form)) = &page.view else {
            return None;
        };
        let field = form.items.iter().find_map(|item| match item {
            FormItem::Field(field) if field.name == name => Some(field),
            _ => None,
        })?;
        let value = page.form_values.get(name);
        match (&field.kind, option) {
            (Kind::Dropdown(dropdown), Some(option)) => {
                let chosen = dropdown
                    .sections
                    .iter()
                    .flat_map(|section| &section.options)
                    .nth(option)?;
                Some(Message::ExtensionFieldEdited(
                    name.to_owned(),
                    chosen.value.clone().into(),
                ))
            }
            (Kind::TagPicker { options, .. }, Some(option)) => {
                let tag = options.get(option)?;
                let chosen = crate::extension_fields::strings(value);
                let next = crate::extension_fields::toggle_tag(options, &chosen, &tag.value);
                Some(Message::ExtensionFieldEdited(
                    name.to_owned(),
                    serde_json::Value::from(next),
                ))
            }
            (
                Kind::FilePicker {
                    allow_multiple,
                    allow_directories,
                    allow_files,
                },
                None,
            ) => Some(Message::ExtensionChooseFiles {
                name: name.to_owned(),
                choice: crate::extension_fields::FileChoice {
                    multiple: *allow_multiple,
                    directories: *allow_directories,
                    files: *allow_files,
                },
            }),
            _ => None,
        }
    }
}

fn extension_semantics(page: &crate::extension_page::ExtensionPage) -> PageSemantics {
    use crate::extension_page::Status;
    use compass_extension_api::View;
    if let Some(alert) = &page.alert {
        return PageSemantics {
            body: Body::Dialog(Dialog {
                title: alert.title.clone(),
                message: alert.message.clone(),
                keys: alert_keys(alert),
            }),
            ..PageSemantics::default()
        };
    }
    let status = |status: &str| PageSemantics {
        status: Some(status.to_owned()),
        ..PageSemantics::default()
    };
    match (&page.status, &page.view) {
        (Status::Loading, _) => status("Loading…"),
        (Status::Stopped(why), _) => status(why),
        (Status::Ready, Some(View::Form(form))) => PageSemantics {
            body: Body::Form(extension_form(page, form)),
            status: page.notice.clone(),
            detail: None,
        },
        (Status::Ready, Some(View::Detail(detail))) => PageSemantics {
            body: Body::Document(extension_detail(page, detail)),
            status: page.notice.clone(),
            detail: None,
        },
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
            // A grid's rows are its sections' cells in rows of each
            // section's columns, as `extension_grid` lays them out.
            let grid = page.grid_columns.as_ref().map(|_| {
                page.grid_groups()
                    .into_iter()
                    .flat_map(|(section, positions)| {
                        positions
                            .chunks(page.section_columns(section).max(1))
                            .map(<[usize]>::to_vec)
                            .collect::<Vec<_>>()
                    })
                    .collect()
            });
            PageSemantics {
                body: Body::List(List {
                    label,
                    items,
                    selected,
                    grid,
                }),
                status,
                detail: None,
            }
        }
        (Status::Ready, _) => PageSemantics {
            status: page.notice.clone(),
            ..PageSemantics::default()
        },
    }
}

/// An extension's detail: its markdown as text, and its metadata as label
/// and value, a tag list's tags together.
fn extension_detail(
    page: &crate::extension_page::ExtensionPage,
    detail: &compass_extension_api::view::Detail,
) -> Document {
    use compass_extension_api::view::MetadataItem;
    let entries = detail.metadata.iter().filter_map(|entry| match entry {
        MetadataItem::Label { title, text, .. } | MetadataItem::Link { title, text, .. } => {
            Some((title.clone(), text.clone()))
        }
        MetadataItem::TagList { title, tags } => Some((
            title.clone(),
            tags.iter()
                .map(|tag| tag.text.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        )),
        MetadataItem::Separator => None,
    });
    Document {
        label: detail
            .navigation_title
            .clone()
            .unwrap_or_else(|| page.title.clone()),
        blocks: markdown_blocks(detail.markdown.as_deref().unwrap_or_default()),
        metadata: metadata(entries),
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
            Kind::Dropdown(dropdown) => {
                let options: Vec<_> = dropdown
                    .sections
                    .iter()
                    .flat_map(|section| &section.options)
                    .collect();
                FieldKind::Dropdown {
                    value: options
                        .iter()
                        .find(|option| {
                            Some(option.value.as_str()) == value.and_then(serde_json::Value::as_str)
                        })
                        .map(|option| option.title.clone()),
                    options: options.iter().map(|option| option.title.clone()).collect(),
                    expanded: None,
                    highlighted: None,
                }
            }
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
                        options: options.iter().map(|(title, _)| title.clone()).collect(),
                        expanded: None,
                        highlighted: None,
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
        assert_eq!(
            form.fields[2].kind,
            FieldKind::Dropdown {
                value: None,
                options: vec!["Stars".to_owned()],
                expanded: None,
                highlighted: None,
            }
        );
        // The field the keyboard is on is the one said to have the focus.
        let focused = FocusTracker::new().focus(&semantics);
        assert_eq!(focused, node_of(&semantics, Role::CheckBox, "PRIVATE"));

        // Pressing the checkbox ticks it.
        let _ = app.update(request(focused, Action::Click, None));
        let Body::Form(form) = app.semantics().body else {
            panic!("still a form");
        };
        assert_eq!(form.fields[1].kind, FieldKind::Checkbox { checked: true });

        // Choosing a dropdown's option sets it, though its list stays shut.
        let stars = node_of(&app.semantics(), Role::MenuListOption, "Stars");
        let _ = app.update(request(stars, Action::Click, None));
        let Body::Form(form) = app.semantics().body else {
            panic!("still a form");
        };
        assert!(matches!(
            &form.fields[2].kind,
            FieldKind::Dropdown { value: Some(value), .. } if value == "Stars"
        ));
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

        // Pressing a chosen tag unchooses it, as clicking it does.
        let work = node_of(&app.semantics(), Role::ListBoxOption, "Work");
        let _ = app.update(request(work, Action::Click, None));
        let Page::Extension(page) = &app.page else {
            panic!("still the extension");
        };
        assert_eq!(page.form_values.get("tags"), Some(&serde_json::json!([])));
    }

    fn extension_page(view: compass_extension_api::View) -> LauncherApp {
        let mut page = crate::extension_page::ExtensionPage::new(1, "Weather");
        page.apply(crate::backend::ExtensionViewState {
            version: 1,
            view: Some(Box::new(view)),
            depth: 1,
            ..Default::default()
        });
        let mut app = LauncherApp::with_index(AppIndex::builder().build());
        app.page = Page::Extension(Box::new(page));
        app
    }

    #[test]
    fn an_extension_detail_is_read_as_its_markdown_and_metadata() {
        use compass_extension_api::View;
        use compass_extension_api::view::{Detail, MetadataItem, MetadataTag};
        let app = extension_page(View::Detail(Detail {
            markdown: Some("# Berlin\n\nSunny, **21°C**.".to_owned()),
            metadata: vec![
                MetadataItem::Label {
                    title: "Wind".to_owned(),
                    text: "12 km/h".to_owned(),
                    icon: None,
                    color: None,
                },
                MetadataItem::Separator,
                MetadataItem::TagList {
                    title: "Alerts".to_owned(),
                    tags: ["Heat", "UV"]
                        .into_iter()
                        .map(|text| MetadataTag {
                            text: text.to_owned(),
                            icon: None,
                            color: None,
                            handler: None,
                        })
                        .collect(),
                },
            ],
            ..Detail::default()
        }));
        let semantics = app.semantics();
        let Body::Document(document) = &semantics.body else {
            panic!("expected a document, got {:?}", semantics.body);
        };
        assert_eq!(document.label, "Weather");
        let text: Vec<&str> = document.blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(text, ["Berlin", "Sunny, 21°C."]);
        assert_eq!(document.blocks[0].heading, Some(1));
        let facts: Vec<(&str, &str)> = document
            .metadata
            .iter()
            .map(|m| (m.label.as_str(), m.value.as_str()))
            .collect();
        assert_eq!(facts, [("Wind", "12 km/h"), ("Alerts", "Heat, UV")]);
    }

    #[test]
    fn an_extension_grid_is_rows_of_each_sections_columns() {
        use compass_extension_api::View;
        use compass_extension_api::view::{GridContent, GridItem, GridSection, GridView};
        let cell = |title: &str| GridItem {
            id: Default::default(),
            key: None,
            title: title.to_owned(),
            subtitle: None,
            content: GridContent::Image(compass_extension_api::view::Image::builtin("circle")),
            tooltip: None,
            keywords: Vec::new(),
            actions: None,
        };
        let mut app = extension_page(View::Grid(GridView {
            columns: Some(2),
            sections: vec![GridSection {
                items: ["Red", "Green", "Blue"].into_iter().map(cell).collect(),
                ..GridSection::default()
            }],
            ..GridView::default()
        }));
        let semantics = app.semantics();
        let list = list(&semantics);
        let names: Vec<&str> = list.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Red", "Green", "Blue"]);
        assert_eq!(list.grid, Some(vec![vec![0, 1], vec![2]]));
        let blue = node_of(&semantics, Role::GridCell, "Blue");
        let _ = app.update(request(blue, Action::Focus, None));
        let Page::Extension(page) = &app.page else {
            panic!("still the extension");
        };
        assert_eq!(page.selected, 2);
    }

    #[test]
    fn the_settings_are_their_pages_beside_the_page_shown_as_a_form() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.open_settings(Some("appearance"));
        let semantics = app.semantics();
        let Body::ListAndForm(pages, form) = &semantics.body else {
            panic!("expected the settings, got {:?}", semantics.body);
        };
        let names: Vec<&str> = pages.items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"General"), "{names:?}");
        assert_eq!(
            pages.selected.and_then(|s| names.get(s).copied()),
            Some("Appearance")
        );
        assert_eq!(form.label, "Appearance");
        let (position, theme) = form
            .fields
            .iter()
            .enumerate()
            .find(|(_, field)| matches!(field.kind, FieldKind::Dropdown { .. }))
            .expect("a dropdown on Appearance");
        let FieldKind::Dropdown {
            options, expanded, ..
        } = &theme.kind
        else {
            unreachable!()
        };
        assert!(!options.is_empty());
        assert_eq!(*expanded, Some(false));

        // Pressing it opens its list on the option the keyboard is on.
        let combo = node_of(&semantics, Role::ComboBox, &theme.label);
        let _ = app.update(request(combo, Action::Click, None));
        let semantics = app.semantics();
        let form = semantics.form().expect("still a form");
        assert_eq!(form.focused, Some(position));
        assert!(matches!(
            form.fields[position].kind,
            FieldKind::Dropdown {
                expanded: Some(true),
                highlighted: Some(_),
                ..
            }
        ));
        let _ = app.update(request(combo, Action::Collapse, None));
        let Page::Settings(page) = &app.page else {
            panic!("still the settings");
        };
        assert_eq!(page.menu, None);

        // Choosing a page from the sidebar shows it.
        let general = node_of(&app.semantics(), Role::ListBoxOption, "General");
        let _ = app.update(request(general, Action::Click, None));
        let semantics = app.semantics();
        let general = semantics.form().expect("General");
        assert_eq!(general.label, "General");
        // Startup's switch, which the engine rather than a setting keeps.
        assert!(
            general
                .fields
                .iter()
                .any(|field| field.label == "Start at login"
                    && matches!(field.kind, FieldKind::Switch { on: false })
                    && matches!(field.target, FieldTarget::Control(_)))
        );
        let login = node_of(&semantics, Role::Switch, "Start at login");
        let _ = app.update(request(login, Action::Focus, None));
        let Page::Settings(page) = &app.page else {
            panic!("still the settings");
        };
        assert_eq!(
            page.focused,
            Some(crate::settings_page::Control::LoginStart)
        );
    }

    #[test]
    fn the_first_run_flow_is_a_form_whose_buttons_press() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        app.open_onboarding(dir.path().join("onboarding.json"));
        let semantics = app.semantics();
        assert_eq!(semantics.search, None);
        let form = semantics.form().expect("a form");
        let first = form.label.clone();
        let continue_label = form.fields.last().expect("Continue").label.clone();
        let next = node_of(&semantics, Role::Button, &continue_label);
        let _ = app.update(request(next, Action::Click, None));
        let semantics = app.semantics();
        let form = semantics.form().expect("a form");
        assert_ne!(form.label, first, "the next step");
        assert!(form.fields.iter().any(|field| field.label == "Theme"
            && matches!(
                field.kind,
                FieldKind::Dropdown {
                    expanded: Some(false),
                    ..
                }
            )));
        assert!(form.fields.iter().any(|field| field.label == "Back"));
        // The hotkey row's button, and the Start at login switch.
        assert!(form.fields.iter().any(|field| {
            field.label == "Open Docs"
                && field
                    .description
                    .as_deref()
                    .is_some_and(|d| d.starts_with("Global hotkey: "))
        }));
        let login = node_of(&semantics, Role::Switch, "Start at login");
        let _ = app.update(request(login, Action::Focus, None));
        let Page::Onboarding(page) = &app.page else {
            panic!("still the flow");
        };
        assert_eq!(
            page.focused,
            Some(crate::onboarding_page::Control::LoginStart)
        );
    }

    #[test]
    fn a_builtin_lists_rows_are_clicked_and_focused_as_its_rows() {
        let mut app = LauncherApp::with_index(AppIndex::builder().build());
        let mut page = crate::windows_page::WindowsPage::default();
        let window = |id: u32, title: &str, app: &str| crate::backend::WindowRow {
            id,
            title: title.into(),
            app: app.into(),
            wm_class: app.to_lowercase(),
            pid: None,
            can_close: true,
            app_known: true,
        };
        page.apply(
            Ok(vec![window(1, "Inbox", "Mail"), window(2, "", "Terminal")]),
            0,
        );
        app.page = Page::Windows(page);
        let semantics = app.semantics();
        let terminal = node_of(&semantics, Role::ListBoxOption, "Terminal");
        let _ = app.update(request(terminal, Action::Focus, None));
        let Page::Windows(page) = &app.page else {
            panic!("still the window switcher");
        };
        assert_eq!(page.selected, 1);
        assert_eq!(list(&app.semantics()).selected, Some(1));
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
