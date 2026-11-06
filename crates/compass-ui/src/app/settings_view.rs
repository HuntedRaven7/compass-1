//! The settings view in the launcher: the C++ settings window's sidebar and
//! pages, each control writing its setting through the engine.
//!
//! A child module of `app` so it can reach the launcher's state; the model is
//! [`crate::settings_page`].

use iced::keyboard::{Key, Modifiers, key::Named};
use iced::widget::{button, pick_list, toggler};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, Task, chord_direction, column,
    container, focus_search, mouse_area, row, scrollable, text, text_input,
};
use crate::adwaita;
use crate::settings::SidebarKind;
use crate::settings_page::{
    Control, DOCS_URL, HINT, ItemEntry, ProviderEntry, RecordTarget, SettingsMessage, SettingsPage,
    Shown, alias_key, providers_of,
};
use crate::shortcut_recorder::{Outcome as RecorderOutcome, ShortcutRecorder};
use compass_core::root_items::RootEdit;
use compass_core::settings_catalog::{self, CorePage, Kind, Setting};

/// The settings body's height: the card less its search field.
const BODY_HEIGHT: f32 = 430.0;

/// The sidebar's width.
const SIDEBAR_WIDTH: f32 = 190.0;

/// An id no field has: focusing it takes Iced's focus off every field, for
/// when the keyboard is on a switch or a button.
const NO_FIELD: &str = "settings-no-field";

/// How far Page Up and Page Down scroll: the body less a row, so the row at
/// the edge stays in view.
const PAGE_SCROLL: f32 = BODY_HEIGHT - 50.0;

/// Another page is on show: it opens at its top, not at the scroll offset
/// the last one was left at, and the sidebar scrolls to its row.
fn shown_page_changed() -> Task<Message> {
    use iced::widget::operation::{RelativeOffset, snap_to};
    Task::batch([
        snap_to(crate::scroll::SETTINGS_BODY, RelativeOffset::START),
        crate::scroll::reveal_settings_page(),
    ])
}

fn settings(message: SettingsMessage) -> Message {
    Message::Settings(message)
}

impl LauncherApp {
    /// Opens the settings at `tab` (`SettingsController::openTab`,
    /// `openExtensionPreferences`), reading the configuration file afresh.
    pub(super) fn open_settings(&mut self, tab: Option<&str>) -> Task<Message> {
        let (config, notice) = match &self.config_path {
            Some(path) => match compass_core::Config::load_checked(path) {
                Ok((config, issues)) => {
                    let notice = match issues.as_slice() {
                        [] => None,
                        [issue] => Some(format!("compass.json: {issue}.")),
                        [issue, rest @ ..] => Some(format!(
                            "compass.json: {issue}. {} more problems are listed by compass doctor.",
                            rest.len()
                        )),
                    };
                    (config, notice)
                }
                Err(err) => (
                    compass_core::Config::default(),
                    Some(format!("{err}. Fix it before changing settings here.")),
                ),
            },
            None => (compass_core::Config::default(), None),
        };
        let providers = providers_of(&self.app_index, &self.root_config);
        let themes = crate::theme::Theme::ALL
            .iter()
            .copied()
            .chain(crate::theme::load_user_themes(&self.view.theme_dirs))
            .map(|theme| (theme.name().to_owned(), theme.title().to_owned()))
            .collect();
        let mut page = SettingsPage::new(config, providers, themes, tab);
        page.notice = notice;
        page.hotkey_elsewhere = self.hotkey_compositor.is_some();
        self.panel = None;
        let _ = self.close_extension_view();
        self.page = Page::Settings(Box::new(page));
        let login_start = match self.backend.clone() {
            Some(backend) => Task::perform(
                async move { backend.login_start(None).await },
                Message::LoginStartAnswered,
            ),
            None => Task::none(),
        };
        Task::batch([focus_search(), shown_page_changed(), login_start])
    }

    /// Moves the settings page aside while an extension command's
    /// preferences form is open over it.
    pub(super) fn park_settings(&mut self) {
        if matches!(self.page, Page::Settings(_))
            && let Page::Settings(page) = std::mem::replace(&mut self.page, Page::Root)
        {
            self.parked_settings = Some(page);
        }
    }

    /// Back to the settings from a command's preferences form, when the form
    /// was opened from them.
    pub(super) fn back_to_settings(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != crate::preferences_page::Purpose::CommandPreferences {
            return None;
        }
        let page = self.parked_settings.take()?;
        self.page = Page::Settings(page);
        Some(focus_search())
    }

    /// The update's release notes, which About offers as a link.
    fn release_url(&self) -> Option<String> {
        self.update.as_ref().map(|offer| offer.release_url.clone())
    }

    /// The view's keys.
    ///
    /// From the search field the arrows move through the sidebar. Tab and
    /// Shift+Tab walk the page's controls in the order they are drawn, the
    /// arrows too once one has the keyboard; Space or Enter flips a switch,
    /// presses a button or opens a dropdown, whose options the arrows then
    /// move through. Page Up, Page Down, Home and End scroll the page (Home
    /// and End stay the field's while text is being edited). Escape closes a
    /// dropdown, then gives the keyboard back to the search field, then
    /// leaves.
    pub(super) fn settings_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        use iced::widget::operation::{AbsoluteOffset, RelativeOffset, scroll_by, snap_to};
        let release = self.release_url();
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        let editing = match &page.focused {
            None => !page.query.is_empty(),
            Some(control) => control.field_id().is_some(),
        };
        match key.as_ref() {
            Key::Named(Named::Escape) => {
                if page.menu.take().is_some() {
                    return Task::none();
                }
                if page.focused.take().is_some() {
                    return focus_search();
                }
                self.page = Page::Root;
                return focus_search();
            }
            Key::Named(Named::Tab) => {
                page.move_focus(!modifiers.shift(), release.as_deref());
                return self.settings_focus_task();
            }
            Key::Named(Named::PageDown | Named::PageUp) => {
                let down = key.as_ref() == Key::Named(Named::PageDown);
                let y = if down { PAGE_SCROLL } else { -PAGE_SCROLL };
                return scroll_by(crate::scroll::SETTINGS_BODY, AbsoluteOffset { x: 0.0, y });
            }
            Key::Named(Named::Home) if !editing => {
                return snap_to(crate::scroll::SETTINGS_BODY, RelativeOffset::START);
            }
            Key::Named(Named::End) if !editing => {
                return snap_to(crate::scroll::SETTINGS_BODY, RelativeOffset::END);
            }
            _ => {}
        }
        let activate = matches!(key.as_ref(), Key::Named(Named::Enter | Named::Space));
        if page.menu.is_some() {
            match key.as_ref() {
                Key::Named(Named::ArrowDown) => page.move_menu(1),
                Key::Named(Named::ArrowUp) => page.move_menu(-1),
                _ if activate => {
                    let choice = page.menu_choice();
                    page.menu = None;
                    return match choice {
                        Some((key, value)) => {
                            self.change_setting(key, serde_json::Value::String(value))
                        }
                        None => Task::none(),
                    };
                }
                _ => return Task::none(),
            }
            return crate::scroll::reveal_settings_focus();
        }
        if let Some(control) = page.focused.clone() {
            let direction = match key.as_ref() {
                Key::Named(Named::ArrowDown) => Some(Direction::Down),
                Key::Named(Named::ArrowUp) => Some(Direction::Up),
                _ => None,
            };
            if let Some(direction) = direction {
                page.move_focus(direction == Direction::Down, release.as_deref());
                return self.settings_focus_task();
            }
            if activate && control.field_id().is_none() {
                return self.activate_settings_control(control);
            }
            return Task::none();
        }
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        // Enter in the search field takes the keyboard to the best match.
        if key.as_ref() == Key::Named(Named::Enter)
            && let Some(matched) = page.first_match_shown()
        {
            page.focused = Some(Control::Setting(matched));
            return self.settings_focus_task();
        }
        if let Some(direction) = direction {
            let before = page.selected;
            page.step(direction == Direction::Down);
            page.notice = None;
            if page.selected != before {
                return shown_page_changed();
            }
        }
        Task::none()
    }

    /// Puts Iced's focus where the page's is: in the control's field, or in
    /// no field for a switch or a button, or in the search field; and
    /// scrolls the control into view.
    fn settings_focus_task(&self) -> Task<Message> {
        let Page::Settings(page) = &self.page else {
            return Task::none();
        };
        let Some(control) = &page.focused else {
            return focus_search();
        };
        let keyboard = match control.field_id() {
            Some(id) => iced::widget::operation::focus(id),
            None => iced::widget::operation::focus(NO_FIELD),
        };
        Task::batch([keyboard, crate::scroll::reveal_settings_focus()])
    }

    /// Space or Enter on a control that is not a field.
    fn activate_settings_control(&mut self, control: Control) -> Task<Message> {
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        let item_enabled = |page: &SettingsPage, id: &str| {
            page.providers
                .iter()
                .flat_map(|provider| &provider.items)
                .find(|item| item.id == id)
                .is_some_and(|item| item.enabled)
        };
        match control {
            Control::Setting(key) => {
                let Some(setting) = settings_catalog::find(&key) else {
                    return Task::none();
                };
                match setting.kind {
                    Kind::Toggle => {
                        let on = page.value(&setting).as_bool().unwrap_or(false);
                        self.change_setting(key, serde_json::Value::Bool(!on))
                    }
                    Kind::Choice(_) | Kind::Theme => {
                        page.open_menu();
                        crate::scroll::reveal_settings_focus()
                    }
                    Kind::Shortcut => {
                        self.settings_message(SettingsMessage::Record(RecordTarget::Setting(key)))
                    }
                    _ => Task::none(),
                }
            }
            Control::ProviderSwitch(id) => {
                let enabled = page
                    .providers
                    .iter()
                    .find(|provider| provider.id == id)
                    .is_some_and(|provider| provider.enabled);
                self.settings_message(SettingsMessage::ProviderToggled(id, !enabled))
            }
            Control::ItemSwitch(id) => {
                let enabled = item_enabled(page, &id);
                self.settings_message(SettingsMessage::ItemToggled(id, !enabled))
            }
            Control::ItemShortcut(id) => {
                self.settings_message(SettingsMessage::Record(RecordTarget::Item(id)))
            }
            Control::ItemPreferences(id) => {
                self.settings_message(SettingsMessage::OpenPreferences(id))
            }
            Control::Link(url) => self.settings_message(SettingsMessage::OpenUrl(url)),
            Control::LoginStart => {
                let on = self.login_start_on();
                self.update(Message::LoginStartToggled(!on))
            }
            Control::ItemAlias(_) => Task::none(),
        }
    }

    /// A key while a shortcut is recorded: the recorder takes it.
    pub(super) fn settings_recorder_event(
        &mut self,
        event: &iced::keyboard::Event,
    ) -> Task<Message> {
        let bound = self.recorder_bound();
        let launcher_hotkey = self.launcher_hotkey.clone();
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        let Some((target, recorder)) = page.recorder.as_mut() else {
            return Task::none();
        };
        let outcome = recorder.key(
            event,
            Some(&launcher_hotkey),
            bound
                .iter()
                .map(|(id, title, shortcut)| (id.as_str(), title.as_str(), shortcut.as_str())),
        );
        let target = target.clone();
        self.settings_recorder_outcome(target, outcome)
    }

    /// What the settings view's recorder, recording for `target`, said to do.
    pub(super) fn settings_recorder_outcome(
        &mut self,
        target: RecordTarget,
        outcome: RecorderOutcome,
    ) -> Task<Message> {
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        match outcome {
            RecorderOutcome::Recording => Task::none(),
            RecorderOutcome::Probe(trigger) => self.probe_shortcut(trigger),
            RecorderOutcome::Back => {
                page.recorder = None;
                self.settings_focus_task()
            }
            RecorderOutcome::Save(shortcut) => {
                page.recorder = None;
                let focus = self.settings_focus_task();
                let saved = match target {
                    RecordTarget::Setting(key) => {
                        self.change_setting(key, serde_json::Value::String(shortcut))
                    }
                    RecordTarget::Item(id) => {
                        self.edit_settings_item(id, RootEdit::Shortcut(shortcut))
                    }
                };
                Task::batch([saved, focus])
            }
        }
    }

    /// Handles the view's messages.
    pub(super) fn settings_message(&mut self, message: SettingsMessage) -> Task<Message> {
        match message {
            SettingsMessage::QueryChanged(query) => {
                if let Page::Settings(page) = &mut self.page {
                    let before = page.selected;
                    page.set_query(query);
                    // The best match is highlighted on its page and
                    // scrolled to.
                    let reveal = crate::scroll::reveal_settings_focus();
                    if page.selected != before {
                        return shown_page_changed().chain(reveal);
                    }
                    return reveal;
                }
                Task::none()
            }
            SettingsMessage::SidebarSelected(row) => {
                if let Page::Settings(page) = &mut self.page {
                    page.select(row);
                    page.notice = None;
                }
                Task::batch([focus_search(), shown_page_changed()])
            }
            SettingsMessage::Changed(key, value) => self.change_setting(key, value),
            SettingsMessage::DraftEdited(key, draft) => {
                if let Page::Settings(page) = &mut self.page {
                    // A field clicked into has the keyboard: Tab goes on
                    // from it.
                    page.focused = Some(match key.strip_prefix("alias:") {
                        Some(id) => Control::ItemAlias(id.to_owned()),
                        None => Control::Setting(key.clone()),
                    });
                    page.menu = None;
                    page.drafts.insert(key, draft);
                }
                Task::none()
            }
            SettingsMessage::DraftSubmitted(key) => {
                let Page::Settings(page) = &mut self.page else {
                    return Task::none();
                };
                let Some(draft) = page.drafts.get(&key).cloned() else {
                    return Task::none();
                };
                if let Some(id) = key.strip_prefix("alias:") {
                    return self.edit_settings_item(id.to_owned(), RootEdit::Alias(draft));
                }
                let Some(setting) = settings_catalog::find(&key) else {
                    return Task::none();
                };
                match SettingsPage::parse_draft(&setting, &draft) {
                    Ok(value) => self.change_setting(key, value),
                    Err(reason) => {
                        page.notice = Some(reason);
                        Task::none()
                    }
                }
            }
            SettingsMessage::Saved {
                key,
                result,
                previous,
            } => match result {
                Ok(()) => {
                    self.apply_setting_live(&key);
                    Task::none()
                }
                Err(reason) => {
                    if settings_catalog::find(&key).is_some_and(|s| s.kind == Kind::Theme) {
                        let _ = self.update(Message::ThemeCancel);
                    }
                    if let Page::Settings(page) = &mut self.page {
                        // The write was refused, so the switch goes back to
                        // what the file still says.
                        page.config = *previous;
                        page.drafts.remove(&key);
                        page.notice = Some(reason);
                    }
                    Task::none()
                }
            },
            SettingsMessage::Record(target) => {
                let Page::Settings(page) = &mut self.page else {
                    return Task::none();
                };
                let (id, title, current) = match &target {
                    RecordTarget::Setting(key) => {
                        let Some(setting) = settings_catalog::find(key) else {
                            return Task::none();
                        };
                        let current = page.value(&setting).as_str().map(str::to_owned);
                        (key.clone(), setting.label.to_owned(), current)
                    }
                    RecordTarget::Item(id) => {
                        let Some(item) = page
                            .providers
                            .iter()
                            .flat_map(|provider| &provider.items)
                            .find(|item| &item.id == id)
                        else {
                            return Task::none();
                        };
                        (id.clone(), item.title.clone(), item.shortcut.clone())
                    }
                };
                page.recorder = Some((target, ShortcutRecorder::new(id, title, current)));
                Task::none()
            }
            SettingsMessage::ProviderToggled(provider, enabled) => {
                if let Page::Settings(page) = &mut self.page {
                    page.set_provider_enabled(&provider, enabled);
                }
                compass_core::root_items::set_provider_enabled(
                    &mut self.root_config,
                    &provider,
                    enabled,
                );
                self.app_index.apply_root_config(&self.root_config);
                match self.backend.clone() {
                    Some(backend) => Task::perform(
                        async move { backend.set_provider_enabled(provider, enabled).await },
                        |result| settings(SettingsMessage::Done(result)),
                    ),
                    None => {
                        if let Err(reason) = self.write_settings_file()
                            && let Page::Settings(page) = &mut self.page
                        {
                            page.notice = Some(reason);
                        }
                        Task::none()
                    }
                }
            }
            SettingsMessage::ItemToggled(id, enabled) => {
                self.edit_settings_item(id, RootEdit::Enabled(enabled))
            }
            SettingsMessage::OpenPreferences(id) => {
                let Some(backend) = self.backend.clone() else {
                    if let Page::Settings(page) = &mut self.page {
                        page.notice = Some(crate::settings_page::NEEDS_ENGINE.to_owned());
                    }
                    return Task::none();
                };
                self.park_settings();
                let opened = id.clone();
                Task::perform(
                    async move { backend.extension_preferences(id).await },
                    move |result| Message::PreferencesOpened {
                        id: opened.clone(),
                        result,
                    },
                )
            }
            SettingsMessage::OpenUrl(url) => {
                let Some(backend) = self.backend.clone() else {
                    if let Page::Settings(page) = &mut self.page {
                        page.notice = Some(crate::settings_page::NEEDS_ENGINE.to_owned());
                    }
                    return Task::none();
                };
                let opened = Task::perform(async move { backend.open_url(url).await }, |result| {
                    settings(SettingsMessage::Done(result))
                });
                Task::batch([opened, self.conceal()])
            }
            SettingsMessage::Done(Ok(())) => Task::none(),
            SettingsMessage::Done(Err(reason)) => {
                match &mut self.page {
                    Page::Settings(page) => page.notice = Some(reason),
                    _ => self.error = Some(reason),
                }
                Task::none()
            }
        }
    }

    /// Writes a setting: into the view's copy at once, then through the
    /// engine (or, with none, straight into the file).
    fn change_setting(&mut self, key: String, value: serde_json::Value) -> Task<Message> {
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        let Some(setting) = settings_catalog::find(&key) else {
            return Task::none();
        };
        page.menu = None;
        let previous = Box::new(page.config.clone());
        if let Err(reason) = page.apply(&key, value.clone()) {
            page.notice = Some(reason);
            return Task::none();
        }
        // A theme shows at once, as Set Theme previews it; the engine's
        // answer keeps it or puts the old one back.
        let preview = match (&setting.kind, value.as_str()) {
            (Kind::Theme, Some(name)) => match crate::theme::Theme::from_name(name) {
                Some(theme) => self.update(Message::ThemePreview(theme)),
                None => Task::none(),
            },
            _ => Task::none(),
        };
        let saved = match self.backend.clone() {
            Some(backend) => {
                let answer = key.clone();
                Task::perform(
                    async move { backend.set_setting(key, value).await },
                    move |result| {
                        settings(SettingsMessage::Saved {
                            key: answer.clone(),
                            result,
                            previous: previous.clone(),
                        })
                    },
                )
            }
            None => {
                let result = self.write_settings_file();
                self.settings_message(SettingsMessage::Saved {
                    key,
                    result,
                    previous,
                })
            }
        };
        Task::batch([preview, saved])
    }

    /// Without an engine, the view's copy of the file is written where the
    /// launcher reads it.
    fn write_settings_file(&self) -> Result<(), String> {
        let (Page::Settings(page), Some(path)) = (&self.page, &self.config_path) else {
            return Err(crate::settings_page::NEEDS_ENGINE.to_owned());
        };
        page.config
            .save_to(path)
            .map_err(|err| format!("could not save the configuration: {err}"))
    }

    /// An item's switch, alias or shortcut: in the view, in this window's
    /// root search, and kept by the engine.
    fn edit_settings_item(&mut self, id: String, edit: RootEdit) -> Task<Message> {
        if let Page::Settings(page) = &mut self.page {
            page.edit_item(&id, &edit);
        }
        if self.backend.is_none()
            && let Err(reason) = self.write_settings_file()
            && let Page::Settings(page) = &mut self.page
        {
            page.notice = Some(reason);
        }
        self.edit_root_item(id, edit)
    }

    /// What this window holds of a setting it has just kept: the navigation
    /// keys, the layout, the clock, the font and the theme apply at once;
    /// the rest is the engine's.
    fn apply_setting_live(&mut self, key: &str) {
        let Page::Settings(page) = &self.page else {
            return;
        };
        let config = page.config.clone();
        self.apply_config_key(&config, key);
    }

    /// `compass.json` changed on disk: everything this window holds of it is
    /// brought in line, as a change in the settings view would, and an open
    /// settings view shows the file as it now is.
    pub(super) fn apply_reloaded_config(&mut self, config: &compass_core::Config) -> Task<Message> {
        for key in [
            "launcher.wrap_navigation",
            "launcher.keybinding",
            "launcher.quick_launch",
            "launcher.close_on_focus_loss",
            "launcher.pop_to_root_on_close",
            "launcher.hotkey",
            "launcher.clock.enabled",
            "launcher.appearance.preset",
            "launcher.appearance.color_scheme",
            "font.normal.family",
            "providers.power.entrypoints.",
            "providers.core.entrypoints.search-emojis.",
        ] {
            self.apply_config_key(config, key);
        }
        let theme = crate::theme::Theme::from_name(config.launcher().appearance().theme())
            .unwrap_or_default();
        if self.view.theme_preview.is_none() {
            self.view.theme_choice = theme;
        }
        self.root_config = config.root_config();
        self.app_index.apply_root_config(&self.root_config);
        self.fallbacks = config.fallback_ids();
        if let Page::Settings(page) = &mut self.page
            && page.recorder.is_none()
        {
            page.config = config.clone();
            page.drafts.clear();
        }
        Task::none()
    }

    /// Applies what `config` says about `key` to this window.
    fn apply_config_key(&mut self, config: &compass_core::Config, key: &str) {
        let launcher = config.launcher();
        match key {
            "launcher.wrap_navigation" => self.wrap_navigation = launcher.wrap_navigation(),
            "launcher.keybinding" => self.keybinding = launcher.keybinding_scheme(),
            "launcher.quick_launch" => self.quick_launch = launcher.quick_launch(),
            "launcher.close_on_focus_loss" => {
                self.close_on_focus_loss = launcher.close_on_focus_loss();
            }
            "launcher.hotkey" => self.launcher_hotkey = launcher.hotkey().to_owned(),
            "launcher.clock.enabled" | "launcher.clock.format" | "launcher.clock.interval" => {
                let clock = launcher.clock();
                self.clock = clock.enabled().then(|| super::ClockSettings {
                    format: clock.format().to_owned(),
                    interval: clock.interval(),
                });
            }
            "launcher.appearance.preset"
            | "launcher.appearance.icons"
            | "launcher.appearance.tint" => {
                let appearance = launcher.appearance();
                let resolved = crate::preset::resolve(
                    Some(appearance.preset()),
                    appearance.icons_override(),
                    appearance.tint_override(),
                );
                self.geometry = resolved.geometry;
                self.field_rule = resolved.field_rule;
                self.tint = resolved.tint;
                self.subtitles = resolved.subtitles;
                self.icons = resolved.icons;
            }
            "launcher.appearance.color_scheme" => match launcher.appearance().color_scheme() {
                "light" => self.view.appearance = crate::design::Appearance::Light,
                "dark" => self.view.appearance = crate::design::Appearance::Dark,
                _ => {}
            },
            "launcher.appearance.theme" => {
                let _ = self.update(Message::ThemeCommit);
            }
            "launcher.pop_to_root_on_close" => {
                self.pop_to_root_on_close = launcher.pop_to_root_on_close();
            }
            "font.normal.family" => self.view.font_family = config.font_family().map(str::to_owned),
            _ if key.starts_with("providers.power.entrypoints.") => {
                for command in compass_core::power_commands::COMMANDS {
                    let preferences = config.entrypoint_preferences(
                        compass_core::power_commands::EXTENSION_ID,
                        command.id,
                    );
                    self.power_asks.insert(
                        command.id.to_owned(),
                        compass_core::power_commands::should_confirm(command, preferences),
                    );
                }
            }
            _ if key.starts_with("providers.core.entrypoints.search-emojis.") => {
                let preferences = config.entrypoint_preferences("core", "search-emojis");
                self.emoji_skin_tone = preferences
                    .and_then(|p| p.get("skinTone")?.as_str())
                    .map(str::to_owned);
                self.emoji_default_action = preferences
                    .and_then(|p| p.get("defaultAction")?.as_str())
                    .unwrap_or(compass_core::emoji_grid::DEFAULT_ACTION_PASTE)
                    .to_owned();
            }
            _ => {}
        }
    }

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
            Kind::Shortcut => {
                let current = value
                    .as_str()
                    .and_then(compass_core::key_combo::KeyCombo::parse)
                    .map_or_else(
                        || "Record Shortcut".to_owned(),
                        |combo| combo.display_tokens().join(" "),
                    );
                self.settings_button(current, 13.0)
                    .on_press(settings(SettingsMessage::Record(RecordTarget::Setting(
                        key,
                    ))))
                    .into()
            }
            Kind::Number { .. } | Kind::Text | Kind::Paths | Kind::Names | Kind::Font => {
                let submit = key.clone();
                let placeholder = if setting.placeholder.is_empty() {
                    match setting.kind {
                        Kind::Paths => "Folders, separated by colons",
                        Kind::Names => "Application IDs, separated by commas",
                        _ => "",
                    }
                } else {
                    setting.placeholder
                };
                text_input(placeholder, &page.text_of(setting))
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
    fn settings_rows<'a>(
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
        let description = self.login_start.as_ref().map_or_else(
            || "Start Compass, hidden, when you log in.".to_owned(),
            compass_core::login_start::Status::describe,
        );
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
        let shortcut = item
            .shortcut
            .as_deref()
            .and_then(compass_core::key_combo::KeyCombo::parse)
            .map_or_else(
                || "Record Shortcut".to_owned(),
                |combo| combo.display_tokens().join(" "),
            );
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

#[cfg(test)]
mod tests;
