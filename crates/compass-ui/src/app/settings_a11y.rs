//! The settings and the first-run flow, for assistive technology: what
//! [`super::settings_view`] and [`super::onboarding`] draw, as a sidebar and
//! a form, and what a request to one of their controls does.
//!
//! Each control is a field whose target is its position in the order Tab
//! reaches it ([`crate::settings_page::SettingsPage::controls`],
//! [`crate::onboarding_page::OnboardingPage::controls`]); the text between
//! them is notes. The dropdowns are the launcher's own list, opened from the
//! keyboard, so a request can open and close them as well as choose.

use accesskit::Action;
use iced::Task;

use super::a11y::{PageSemantics, item};
use super::settings_view::{field_placeholder, shortcut_label};
use super::{LauncherApp, Message, Page};
use crate::onboarding_page::{Control as Step, OnboardingPage, ThemeOption};
use crate::semantics::{Body, Field, FieldKind, FieldTarget, Form, List};
use crate::settings::SidebarKind;
use crate::settings_page::{
    Control, DOCS_URL, ProviderEntry, SettingsMessage, SettingsPage, Shown, alias_key,
};
use compass_core::onboarding::{self, Install, RECOMMENDED_EXTENSIONS, Step as FlowStep};
use compass_core::settings_catalog::{self, CorePage, Kind, Setting};

/// A field that takes nothing: text between the controls.
fn note(label: impl Into<String>, text: impl Into<String>) -> Field {
    Field {
        label: label.into(),
        kind: FieldKind::Note { text: text.into() },
        description: None,
        required: false,
        invalid: false,
        target: FieldTarget::None,
    }
}

/// A field for the control at `position` in the order Tab reaches them.
fn control(label: impl Into<String>, kind: FieldKind, position: Option<usize>) -> Field {
    Field {
        label: label.into(),
        kind,
        description: None,
        required: false,
        invalid: false,
        target: position.map_or(FieldTarget::None, FieldTarget::Control),
    }
}

/// `field` with `description`, when it says something.
fn described(mut field: Field, description: impl Into<String>) -> Field {
    let description = description.into();
    field.description = (!description.is_empty()).then_some(description);
    field
}

/// The settings' sidebar rows a person can choose, by their row in the
/// sidebar: the dividers between them are not rows to a screen reader.
fn sidebar_rows(page: &SettingsPage) -> Vec<usize> {
    page.sidebar
        .rows()
        .iter()
        .enumerate()
        .filter(|(_, row)| row.kind != SidebarKind::Divider)
        .map(|(index, _)| index)
        .collect()
}

impl LauncherApp {
    /// The settings: the sidebar's pages, and the page shown as a form.
    pub(super) fn settings_semantics(&self, page: &SettingsPage) -> PageSemantics {
        let rows = sidebar_rows(page);
        let items = rows
            .iter()
            .filter_map(|&row| page.sidebar.rows().get(row))
            .map(|row| {
                item(
                    row.label.clone(),
                    (!row.enabled).then(|| "Disabled".to_owned()),
                )
            })
            .collect();
        let selected = rows
            .iter()
            .position(|&row| isize::try_from(row).ok() == Some(page.selected));
        let list = List {
            label: "Settings".to_owned(),
            items,
            selected,
            grid: None,
        };
        let (form, status) = self.settings_form(page);
        PageSemantics {
            body: Body::ListAndForm(list, form),
            status: page.notice.clone().or(status),
            detail: None,
        }
    }

    /// The page shown, as a form, and what it says in place of one.
    fn settings_form(&self, page: &SettingsPage) -> (Form, Option<String>) {
        if let Some((_, recorder)) = &page.recorder {
            let shortcut = if recorder.tokens.is_empty() {
                "…".to_owned()
            } else {
                recorder.tokens.join(" ")
            };
            let mut fields = vec![note("Shortcut", shortcut)];
            if recorder.current.is_some() {
                fields.push(note("", crate::shortcut_recorder::REMOVE_HINT));
            }
            let form = Form {
                label: recorder.title.clone(),
                fields,
                focused: None,
            };
            return (form, Some(recorder.status.clone()));
        }
        let release = self.release_url();
        let controls = page.controls(release.as_deref());
        let at = |wanted: &Control| controls.iter().position(|c| c == wanted);
        let (label, fields) = match page.shown() {
            Shown::Nothing => {
                let form = Form::default();
                return (form, Some("No settings match".to_owned()));
            }
            Shown::Core(core) => (core.title().to_owned(), self.core_fields(page, core, &at)),
            Shown::Provider(provider) => (
                provider.title.clone(),
                self.provider_fields(page, provider, &at),
            ),
        };
        let focused = page.focused.as_ref().and_then(|focused| {
            let position = at(focused)?;
            fields
                .iter()
                .position(|field| field.target == FieldTarget::Control(position))
        });
        let form = Form {
            label,
            fields,
            focused,
        };
        (form, None)
    }

    fn core_fields(
        &self,
        page: &SettingsPage,
        core: CorePage,
        at: &dyn Fn(&Control) -> Option<usize>,
    ) -> Vec<Field> {
        let link = |label: &str, url: &str| {
            control(
                label,
                FieldKind::Button { value: None },
                at(&Control::Link(url.to_owned())),
            )
        };
        let mut fields = match core {
            CorePage::About => {
                let mut fields = vec![
                    note("", format!("Compass {}", env!("CARGO_PKG_VERSION"))),
                    note(
                        "",
                        "A launcher for the Linux desktop that runs Vicinae and Raycast extensions.",
                    ),
                ];
                if let Some(offer) = &self.update {
                    fields.push(note("", super::release_check::title(offer)));
                    fields.push(link("View release notes", &offer.release_url));
                }
                fields.push(link("Documentation", DOCS_URL));
                fields.push(link(
                    "Report a Bug",
                    compass_core::bug_report::CREATE_ISSUE_URL,
                ));
                fields
            }
            CorePage::Keybindings => settings_catalog::KEYBINDINGS
                .iter()
                .map(|(name, description, keys)| described(note(*name, *keys), *description))
                .collect(),
            CorePage::General => {
                let mut fields = self.setting_fields(page, &SettingsPage::core_settings(core), at);
                fields.push(described(
                    control(
                        "Start at login",
                        FieldKind::Switch {
                            on: self.login_start_on(),
                        },
                        at(&Control::LoginStart),
                    ),
                    self.login_start_line(super::settings_view::LOGIN_START_DEFAULT),
                ));
                fields
            }
            CorePage::Appearance | CorePage::Advanced => {
                self.setting_fields(page, &SettingsPage::core_settings(core), at)
            }
        };
        fields.extend(
            SettingsPage::not_in_compass(core)
                .into_iter()
                .map(|missing| {
                    note(
                        "Not in Compass",
                        format!("{}: {}", missing.label, missing.reason),
                    )
                }),
        );
        fields
    }

    fn provider_fields(
        &self,
        page: &SettingsPage,
        provider: &ProviderEntry,
        at: &dyn Fn(&Control) -> Option<usize>,
    ) -> Vec<Field> {
        let mut fields = vec![described(
            control(
                provider.title.clone(),
                FieldKind::Switch {
                    on: provider.enabled,
                },
                at(&Control::ProviderSwitch(provider.id.clone())),
            ),
            format!("{} · {} items", provider.provenance, provider.items.len()),
        )];
        fields.extend(self.setting_fields(
            page,
            &SettingsPage::provider_settings(&provider.id),
            at,
        ));
        for entry in &provider.items {
            let alias = page
                .drafts
                .get(&alias_key(&entry.id))
                .cloned()
                .unwrap_or_else(|| entry.alias.clone().unwrap_or_default());
            fields.push(described(
                control(
                    "Alias",
                    FieldKind::Text {
                        value: alias,
                        placeholder: Some("Alias".to_owned()),
                        secret: false,
                        multiline: false,
                    },
                    at(&Control::ItemAlias(entry.id.clone())),
                ),
                entry.title.clone(),
            ));
            fields.push(described(
                control(
                    "Shortcut",
                    FieldKind::Button {
                        value: Some(shortcut_label(entry.shortcut.as_deref())),
                    },
                    at(&Control::ItemShortcut(entry.id.clone())),
                ),
                entry.title.clone(),
            ));
            if entry.has_preferences {
                fields.push(described(
                    control(
                        "Preferences",
                        FieldKind::Button { value: None },
                        at(&Control::ItemPreferences(entry.id.clone())),
                    ),
                    entry.title.clone(),
                ));
            }
            fields.push(control(
                entry.title.clone(),
                FieldKind::Switch { on: entry.enabled },
                at(&Control::ItemSwitch(entry.id.clone())),
            ));
            fields.extend(self.setting_fields(page, &SettingsPage::item_settings(&entry.id), at));
        }
        fields
    }

    /// Each setting's row, as `settings_rows` draws it.
    fn setting_fields(
        &self,
        page: &SettingsPage,
        settings: &[Setting],
        at: &dyn Fn(&Control) -> Option<usize>,
    ) -> Vec<Field> {
        settings
            .iter()
            .map(|setting| {
                if setting.key == "launcher.hotkey"
                    && let Some(compositor) = self.hotkey_compositor
                {
                    return described(
                        note(setting.label, format!("Set in {}", compositor.name())),
                        self.open_hint(),
                    );
                }
                let target = Control::Setting(setting.key.clone());
                let value = page.value(setting);
                let kind = match &setting.kind {
                    Kind::Toggle => FieldKind::Switch {
                        on: value.as_bool().unwrap_or(false),
                    },
                    Kind::Choice(_) | Kind::Theme => {
                        let options = page.options(setting);
                        let current = value.as_str().unwrap_or_default();
                        let open = page.menu.filter(|_| page.has_focus(&target));
                        FieldKind::Dropdown {
                            value: options
                                .iter()
                                .find(|(option, _)| option.eq_ignore_ascii_case(current))
                                .map(|(_, label)| label.clone()),
                            options: options.into_iter().map(|(_, label)| label).collect(),
                            expanded: Some(open.is_some()),
                            highlighted: open,
                        }
                    }
                    Kind::Shortcut => FieldKind::Button {
                        value: Some(shortcut_label(value.as_str())),
                    },
                    Kind::Number { .. } | Kind::Text | Kind::Paths | Kind::Names | Kind::Font => {
                        let placeholder = field_placeholder(setting);
                        FieldKind::Text {
                            value: page.text_of(setting),
                            placeholder: (!placeholder.is_empty()).then(|| placeholder.to_owned()),
                            secret: false,
                            multiline: false,
                        }
                    }
                };
                described(
                    control(setting.label, kind, at(&target)),
                    setting.description,
                )
            })
            .collect()
    }

    /// Does what assistive technology asked of a settings control: `field`,
    /// or its `option`, as the keyboard or a click would have.
    pub(super) fn settings_request(
        &mut self,
        position: usize,
        option: Option<usize>,
        action: Action,
        value: Option<String>,
    ) -> Task<Message> {
        let release = self.release_url();
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        let Some(target) = page.controls(release.as_deref()).get(position).cloned() else {
            return Task::none();
        };
        let setting = match &target {
            Control::Setting(key) => settings_catalog::find(key),
            _ => None,
        };
        if let (Some(option), Some(setting)) = (option, &setting) {
            let Some((chosen, _)) = page.options(setting).into_iter().nth(option) else {
                return Task::none();
            };
            page.focused = Some(target);
            page.menu = None;
            return self.update(Message::Settings(SettingsMessage::Changed(
                setting.key.clone(),
                serde_json::Value::String(chosen),
            )));
        }
        let dropdown = setting
            .as_ref()
            .is_some_and(|setting| matches!(setting.kind, Kind::Choice(_) | Kind::Theme));
        let open = page.menu.is_some() && page.has_focus(&target);
        match action {
            Action::Focus => {
                page.focused = Some(target);
                page.menu = None;
                self.settings_focus_task()
            }
            Action::Collapse if dropdown => {
                page.menu = None;
                Task::none()
            }
            Action::Click if dropdown && open => {
                page.menu = None;
                Task::none()
            }
            Action::Click | Action::Expand if dropdown => {
                page.focused = Some(target);
                page.open_menu();
                crate::scroll::reveal_settings_focus()
            }
            Action::Click => {
                page.focused = Some(target.clone());
                page.menu = None;
                self.activate_settings_control(target)
            }
            Action::SetValue => {
                let key = match &target {
                    Control::Setting(key) => key.clone(),
                    Control::ItemAlias(id) => alias_key(id),
                    _ => return Task::none(),
                };
                let Some(value) = value else {
                    return Task::none();
                };
                let edited = self.update(Message::Settings(SettingsMessage::DraftEdited(
                    key.clone(),
                    value,
                )));
                let submitted =
                    self.update(Message::Settings(SettingsMessage::DraftSubmitted(key)));
                Task::batch([edited, submitted])
            }
            _ => Task::none(),
        }
    }

    /// Chooses the sidebar's `position`th page, as a click on its row.
    pub(super) fn settings_sidebar_request(
        &mut self,
        position: usize,
        action: Action,
    ) -> Task<Message> {
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        let Some(&row) = sidebar_rows(page).get(position) else {
            return Task::none();
        };
        match action {
            Action::Click => self.update(Message::Settings(SettingsMessage::SidebarSelected(row))),
            Action::Focus => {
                page.select(row);
                crate::scroll::reveal_settings_page()
            }
            _ => Task::none(),
        }
    }

    /// The first-run flow's step, as a form: its heading, what it says and
    /// its controls, then Back and Continue.
    pub(super) fn onboarding_semantics(&self, page: &OnboardingPage) -> PageSemantics {
        let step = page.flow.step();
        let controls = page.controls();
        let at = |wanted: Step| controls.iter().position(|c| *c == wanted);
        let granted = page
            .shortcut
            .as_ref()
            .is_some_and(compass_core::launcher_shortcut::LauncherShortcut::is_granted);
        let mut fields = vec![note("", step.subtitle(granted))];
        match step {
            FlowStep::Personalize => {
                let current = ThemeOption(self.view.theme_choice);
                let open = page.menu.filter(|_| page.focused == Some(Step::Theme));
                fields.push(described(
                    control(
                        "Theme",
                        FieldKind::Dropdown {
                            value: Some(current.to_string()),
                            options: page.themes.iter().map(ToString::to_string).collect(),
                            expanded: Some(open.is_some()),
                            highlighted: open,
                        },
                        at(Step::Theme),
                    ),
                    "Shared across the entire app.",
                ));
                // The button the hotkey row draws: asking the desktop where
                // that can help, the documentation otherwise.
                let hotkey = match page.shortcut.as_ref().filter(|_| page.offers_request()) {
                    Some(state) => control(
                        state.request_label(),
                        FieldKind::Button { value: None },
                        at(Step::RequestShortcut),
                    ),
                    None => control(
                        "Open Docs",
                        FieldKind::Button { value: None },
                        at(Step::OpenDocs),
                    ),
                };
                fields.push(described(
                    hotkey,
                    format!("Global hotkey: {}", self.hotkey_guidance(page)),
                ));
                fields.push(described(
                    control(
                        "Start at login",
                        FieldKind::Switch {
                            on: self.login_start_on(),
                        },
                        at(Step::LoginStart),
                    ),
                    self.login_start_line(super::onboarding::LOGIN_START_OFFER),
                ));
            }
            FlowStep::Extensions => {
                for (index, recommendation) in RECOMMENDED_EXTENSIONS.iter().enumerate() {
                    let state = page.extensions.state(index).unwrap_or(Install::Available);
                    // Installing and Installed are said, not pressed.
                    let field = match at(Step::Install(index)) {
                        Some(position) => control(
                            recommendation.title,
                            FieldKind::Button {
                                value: Some(state.label().to_owned()),
                            },
                            Some(position),
                        ),
                        None => note(recommendation.title, state.label()),
                    };
                    fields.push(described(
                        field,
                        format!(
                            "{} · {}",
                            recommendation.store.name(),
                            recommendation.description
                        ),
                    ));
                }
                if let Some(notice) = page.extensions.notice() {
                    fields.push(note("", notice));
                }
            }
            FlowStep::Complete => {
                fields.push(note("", self.hotkey_guidance(page)));
                fields.push(note("", "Compass is open source software."));
                fields.push(control(
                    "GitHub",
                    FieldKind::Button { value: None },
                    at(Step::GitHub),
                ));
            }
            FlowStep::Welcome | FlowStep::Permissions => {}
        }
        if page.flow.can_go_back() {
            fields.push(control(
                "Back",
                FieldKind::Button { value: None },
                at(Step::Back),
            ));
        }
        fields.push(control(
            page.flow.primary_label(),
            FieldKind::Button { value: None },
            at(Step::Continue),
        ));
        let focused = page.focused.and_then(|focused| {
            let position = at(focused)?;
            fields
                .iter()
                .position(|field| field.target == FieldTarget::Control(position))
        });
        let form = Form {
            label: step.heading().to_owned(),
            fields,
            focused,
        };
        PageSemantics {
            body: Body::Form(form),
            status: page.notice.clone(),
            detail: None,
        }
    }

    /// Does what assistive technology asked of a control of the flow, or of
    /// the theme dropdown's `option`.
    pub(super) fn onboarding_request(
        &mut self,
        position: usize,
        option: Option<usize>,
        action: Action,
    ) -> Task<Message> {
        let Page::Onboarding(page) = &mut self.page else {
            return Task::none();
        };
        let Some(target) = page.controls().get(position).copied() else {
            return Task::none();
        };
        if let Some(option) = option {
            return match page.themes.get(option).copied() {
                Some(theme) => {
                    page.focused = Some(target);
                    self.update(Message::OnboardingTheme(theme))
                }
                None => Task::none(),
            };
        }
        let open = page.menu.is_some();
        match (target, action) {
            (_, Action::Focus) => {
                page.focused = Some(target);
                page.menu = None;
                Task::none()
            }
            (Step::Theme, Action::Collapse) | (Step::Theme, Action::Click) if open => {
                page.menu = None;
                Task::none()
            }
            (Step::Theme, Action::Click | Action::Expand) => {
                page.focused = Some(target);
                let current = ThemeOption(self.view.theme_choice);
                page.menu = Some(
                    page.themes
                        .iter()
                        .position(|theme| *theme == current)
                        .unwrap_or(0),
                );
                crate::scroll::reveal_onboarding_option()
            }
            (_, Action::Click) => {
                page.focused = Some(target);
                page.menu = None;
                let message = match target {
                    Step::Theme => return Task::none(),
                    Step::OpenDocs => Message::OnboardingOpen(onboarding::HOTKEY_DOCS_URL),
                    Step::RequestShortcut => Message::OnboardingRequestShortcut,
                    Step::LoginStart => Message::LoginStartToggled(!self.login_start_on()),
                    Step::GitHub => Message::OnboardingOpen(onboarding::GITHUB_URL),
                    Step::Install(index) => Message::OnboardingInstall(index),
                    Step::Back => Message::OnboardingBack,
                    Step::Continue => Message::OnboardingContinue,
                };
                self.update(message)
            }
            _ => Task::none(),
        }
    }
}
