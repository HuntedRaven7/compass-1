//! The settings view: the C++ settings window's pages, drawn inside the
//! launcher.
//!
//! The C++ opens the settings as a second, independent window
//! (`SettingsController::openWindow`). Compass shows the same pages as a
//! view of the launcher instead — a declared difference: a second Iced
//! window would need the resident daemon's per-window views and a second
//! surface on every compositor path, for pages that are a sidebar and a
//! form. The sidebar is [`crate::settings::SidebarModel`], the port of
//! `SettingsSidebarModel`; the settings and their keys are
//! [`compass_core::settings_catalog`]'s; an extension page is
//! `ExtensionSettingsModel`'s provider row and its commands, with their
//! switches, aliases and shortcuts, over the launcher's root items.

use std::collections::BTreeMap;

use compass_core::settings_catalog::{self, CorePage, Scope, Setting};

use crate::settings::{ProviderInfo, SidebarKind, SidebarModel};

/// The search field's placeholder: the sidebar's filter.
pub const PLACEHOLDER: &str = "Search settings...";

/// The line under the pages saying what the keys do.
pub const HINT: &str = "↑↓: pages    Tab: controls    Space: change    Esc: back";

/// Why a change could not be kept: no engine to write it, and no file to
/// write it to.
pub const NEEDS_ENGINE: &str = "Changing settings needs the Compass engine";

/// Where the project's documentation is (`Omnicast::DOC_URL`).
pub const DOCS_URL: &str = "https://tunaos.org/compass";

/// What a recorded shortcut is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordTarget {
    /// A setting of kind `Shortcut` (the launcher hotkey), by its key.
    Setting(String),
    /// A root item's shortcut, by its `provider:entrypoint` id.
    Item(String),
}

/// A control on a settings page that the keyboard can reach, in the order
/// [`SettingsPage::controls`] lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    /// A setting's switch, dropdown, shortcut button or field, by its key.
    Setting(String),
    /// A provider's switch, by its id.
    ProviderSwitch(String),
    /// A root item's alias field, by the item's id.
    ItemAlias(String),
    /// A root item's shortcut button.
    ItemShortcut(String),
    /// A root item's Preferences button.
    ItemPreferences(String),
    /// A root item's switch.
    ItemSwitch(String),
    /// A link button (About), by its URL.
    Link(String),
    /// General's Start at login switch, which is the engine's to change
    /// rather than a setting of `compass.json`.
    LoginStart,
}

impl Control {
    /// The id of its text field, when it is one: typing goes there, so it
    /// takes Iced's own focus as well as the page's.
    #[must_use]
    pub fn field_id(&self) -> Option<String> {
        use settings_catalog::Kind;
        match self {
            Self::Setting(key) => settings_catalog::find(key)
                .filter(|setting| {
                    matches!(
                        setting.kind,
                        Kind::Number { .. } | Kind::Text | Kind::Paths | Kind::Names | Kind::Font
                    )
                })
                .map(|_| format!("settings-field:{key}")),
            Self::ItemAlias(id) => Some(format!("settings-field:{}", alias_key(id))),
            _ => None,
        }
    }
}

/// The settings view's messages.
#[derive(Debug, Clone)]
pub enum SettingsMessage {
    /// The sidebar's filter changed.
    QueryChanged(String),
    /// A sidebar row was clicked.
    SidebarSelected(usize),
    /// A switch or a list chose a value: written at once.
    Changed(String, serde_json::Value),
    /// A text field was typed in: kept as typed until submitted. The key is
    /// a setting's, or [`alias_key`]'s for an item's alias.
    DraftEdited(String, String),
    /// A text field was submitted.
    DraftSubmitted(String),
    /// The engine kept a setting, or said why not.
    Saved {
        /// The setting's key.
        key: String,
        /// The answer.
        result: Result<(), String>,
        /// The view's copy of the file before the change, put back when the
        /// change is refused.
        previous: Box<compass_core::Config>,
    },
    /// Start recording a shortcut.
    Record(RecordTarget),
    /// A provider's switch.
    ProviderToggled(String, bool),
    /// A root item's switch.
    ItemToggled(String, bool),
    /// Open an extension command's preferences form.
    OpenPreferences(String),
    /// Open a link (About).
    OpenUrl(String),
    /// An edit the engine answered: nothing to do unless it failed.
    Done(Result<(), String>),
}

/// The draft key an item's alias field is kept under.
#[must_use]
pub fn alias_key(id: &str) -> String {
    format!("alias:{id}")
}

/// One root item on its provider's page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemEntry {
    /// Its `provider:entrypoint` id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Whether it is in root search.
    pub enabled: bool,
    /// Its alias.
    pub alias: Option<String>,
    /// Its shortcut, as stored.
    pub shortcut: Option<String>,
    /// Whether it is an extension command with preferences of its own.
    pub has_preferences: bool,
}

/// One provider: a sidebar row and its page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEntry {
    /// Its id.
    pub id: String,
    /// Its name.
    pub title: String,
    /// Where it comes from (`ExtensionSettingsModel::provenanceForProvider`).
    pub provenance: &'static str,
    /// Whether it is on.
    pub enabled: bool,
    /// Whether it is one of the launcher's own (a group) rather than an
    /// extension.
    pub is_group: bool,
    /// Its items, in index order.
    pub items: Vec<ItemEntry>,
}

/// What the content pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown<'a> {
    /// One of the window's own pages.
    Core(CorePage),
    /// A provider's page.
    Provider(&'a ProviderEntry),
    /// Nothing selected (the filter matches nothing).
    Nothing,
}

/// The settings view's state.
#[derive(Debug, Clone)]
pub struct SettingsPage {
    /// The sidebar's filter, which the search field edits.
    pub query: String,
    /// The sidebar.
    pub sidebar: SidebarModel,
    /// The selected sidebar row.
    pub selected: isize,
    /// The providers, for the extension pages.
    pub providers: Vec<ProviderEntry>,
    /// The file as last read, with the view's own writes applied.
    pub config: compass_core::Config,
    /// Text fields as typed, by setting key or [`alias_key`].
    pub drafts: BTreeMap<String, String>,
    /// The themes the Theme list offers, as `(name, title)`.
    pub themes: Vec<(String, String)>,
    /// A shortcut being recorded, and what for.
    pub recorder: Option<(RecordTarget, crate::shortcut_recorder::ShortcutRecorder)>,
    /// Why the last write failed, or what it needs.
    pub notice: Option<String>,
    /// The control with the keyboard; `None` while the search field has it.
    pub focused: Option<Control>,
    /// The option highlighted in the focused dropdown, while it is open.
    pub menu: Option<usize>,
    /// The settings the search matched by label or description, best first.
    pub matches: Vec<String>,
    /// Whether the compositor binds the launcher's hotkey, so its row is
    /// guidance with nothing for the keyboard to land on.
    pub hotkey_elsewhere: bool,
}

impl SettingsPage {
    /// The view over `config` and `providers`, open at `tab` (a page id as
    /// `openTab` takes it, a provider id, or a root item's id as
    /// `openExtensionPreferences` takes it), else at General.
    #[must_use]
    pub fn new(
        config: compass_core::Config,
        providers: Vec<ProviderEntry>,
        themes: Vec<(String, String)>,
        tab: Option<&str>,
    ) -> Self {
        let sidebar = SidebarModel::new(&provider_infos(&providers));
        let mut page = Self {
            query: String::new(),
            selected: sidebar.first_selectable_row(),
            sidebar,
            providers,
            config,
            drafts: BTreeMap::new(),
            themes,
            recorder: None,
            notice: None,
            focused: None,
            menu: None,
            matches: Vec::new(),
            hotkey_elsewhere: false,
        };
        if let Some(tab) = tab {
            page.open_tab(tab);
        }
        page
    }

    /// Selects the page `tab` names, as `openTab` and
    /// `openExtensionPreferences` do. Returns whether one did.
    pub fn open_tab(&mut self, tab: &str) -> bool {
        let key = match CorePage::from_tab(tab) {
            Some(page) => page.id().to_owned(),
            None => compass_core::root_items::split_entrypoint_id(tab)
                .map_or(tab, |(provider, _)| provider)
                .to_owned(),
        };
        let row = self.sidebar.index_of_key(&key);
        if row < 0 {
            return false;
        }
        self.select_row(row);
        true
    }

    /// Shows the sidebar's `row`, with the keyboard back in the search field.
    fn select_row(&mut self, row: isize) {
        if row != self.selected {
            self.focused = None;
            self.menu = None;
        }
        self.selected = row;
    }

    /// Filters the sidebar by page name and by each setting's label and
    /// description, fuzzily, and shows the page of the best match; with
    /// nothing typed, the page shown stays.
    pub fn set_query(&mut self, query: String) {
        let key = self.selected_key().to_owned();
        let infos = provider_infos(&self.providers);
        let matched = self.setting_matches(&query);
        let hits = matched
            .iter()
            .map(|(row, _, score)| (row.clone(), *score))
            .collect();
        self.matches = matched.into_iter().map(|(_, key, _)| key).collect();
        self.sidebar
            .set_query_with_hits(query.clone(), &infos, hits);
        self.query = query;
        let row = self.sidebar.index_of_key(&key);
        self.select_row(if self.query.is_empty() && row >= 0 && !key.is_empty() {
            row
        } else {
            self.sidebar.first_selectable_row()
        });
    }

    /// Every setting whose label or description `query` matches, as
    /// `(sidebar row key, setting key, score)`, best first. A setting under
    /// a provider that is not listed is left out.
    fn setting_matches(&self, query: &str) -> Vec<(String, String, u32)> {
        use compass_search::{Query, WeightedField, score_weighted};
        if query.trim().is_empty() {
            return Vec::new();
        }
        let query = Query::new(query);
        let mut matched: Vec<(String, String, u32)> = settings_catalog::catalog()
            .into_iter()
            .filter_map(|setting| {
                let row = match &setting.scope {
                    Scope::Core(page) => page.id().to_owned(),
                    Scope::Provider(id) => (*id).to_owned(),
                    Scope::Command(item) => compass_core::root_items::split_entrypoint_id(item)
                        .map_or_else(|| item.clone(), |(provider, _)| provider.to_owned()),
                };
                if !self.sidebar_lists(&row) {
                    return None;
                }
                let found = score_weighted(
                    &[
                        WeightedField::new(setting.label, 1.0),
                        WeightedField::new(setting.description, 0.5),
                    ],
                    &query,
                );
                found.accepted().then_some((row, setting.key, found.score))
            })
            .collect();
        matched.sort_by(|left, right| right.2.cmp(&left.2));
        matched
    }

    /// Whether `key` is a page the sidebar can list: a core page or a
    /// provider.
    fn sidebar_lists(&self, key: &str) -> bool {
        CorePage::ALL.iter().any(|page| page.id() == key)
            || self.providers.iter().any(|provider| provider.id == key)
    }

    /// Whether the search matched `key`, a setting on the page on show.
    #[must_use]
    pub fn is_match(&self, key: &str) -> bool {
        !self.query.is_empty() && self.matches.iter().any(|matched| matched == key)
    }

    /// The best match on the page on show, which the page scrolls to and
    /// Enter in the search field moves the keyboard to.
    #[must_use]
    pub fn first_match_shown(&self) -> Option<String> {
        let controls = self.controls(None);
        self.matches
            .iter()
            .find(|key| controls.contains(&Control::Setting((*key).clone())))
            .cloned()
    }

    /// Moves the sidebar selection one selectable row.
    pub fn step(&mut self, down: bool) {
        self.select_row(
            self.sidebar
                .step_row(self.selected, if down { 1 } else { -1 }),
        );
    }

    /// Selects a clicked row, when it can be.
    pub fn select(&mut self, row: usize) {
        if self
            .sidebar
            .rows()
            .get(row)
            .is_some_and(crate::settings::SidebarRow::selectable)
        {
            self.select_row(row as isize);
        }
    }

    /// The controls the page on show has, in the order they are drawn;
    /// `release_url` is the update's, when About offers one.
    #[must_use]
    pub fn controls(&self, release_url: Option<&str>) -> Vec<Control> {
        if self.recorder.is_some() {
            return Vec::new();
        }
        let settings = |list: Vec<Setting>| {
            list.into_iter()
                .filter(|s| !(self.hotkey_elsewhere && s.key == "launcher.hotkey"))
                .map(|s| Control::Setting(s.key))
        };
        match self.shown() {
            Shown::Nothing => Vec::new(),
            Shown::Core(CorePage::About) => release_url
                .map(str::to_owned)
                .into_iter()
                .chain([
                    DOCS_URL.to_owned(),
                    compass_core::bug_report::CREATE_ISSUE_URL.to_owned(),
                ])
                .map(Control::Link)
                .collect(),
            Shown::Core(CorePage::Keybindings) => Vec::new(),
            Shown::Core(CorePage::General) => settings(Self::core_settings(CorePage::General))
                .chain(std::iter::once(Control::LoginStart))
                .collect(),
            Shown::Core(core) => settings(Self::core_settings(core)).collect(),
            Shown::Provider(provider) => {
                let mut controls = vec![Control::ProviderSwitch(provider.id.clone())];
                controls.extend(settings(Self::provider_settings(&provider.id)));
                for item in &provider.items {
                    controls.push(Control::ItemAlias(item.id.clone()));
                    controls.push(Control::ItemShortcut(item.id.clone()));
                    if item.has_preferences {
                        controls.push(Control::ItemPreferences(item.id.clone()));
                    }
                    controls.push(Control::ItemSwitch(item.id.clone()));
                    controls.extend(settings(Self::item_settings(&item.id)));
                }
                controls
            }
        }
    }

    /// Tab (`forward`) or Shift+Tab: the next control, or back to the
    /// search field past either end. Closes an open dropdown.
    pub fn move_focus(&mut self, forward: bool, release_url: Option<&str>) {
        let controls = self.controls(release_url);
        self.focused = crate::focus::step(&controls, self.focused.as_ref(), forward);
        self.menu = None;
    }

    /// Whether `control` has the keyboard.
    #[must_use]
    pub fn has_focus(&self, control: &Control) -> bool {
        self.focused.as_ref() == Some(control)
    }

    /// A dropdown setting's options, as `(value, label)`.
    #[must_use]
    pub fn options(&self, setting: &Setting) -> Vec<(String, String)> {
        match &setting.kind {
            settings_catalog::Kind::Choice(options) => options
                .iter()
                .map(|(value, label)| ((*value).to_owned(), (*label).to_owned()))
                .collect(),
            settings_catalog::Kind::Theme => self.themes.clone(),
            _ => Vec::new(),
        }
    }

    /// The focused dropdown's setting and its options, when one is focused.
    #[must_use]
    pub fn focused_choice(&self) -> Option<(Setting, Vec<(String, String)>)> {
        let Some(Control::Setting(key)) = &self.focused else {
            return None;
        };
        let setting = settings_catalog::find(key)?;
        let options = self.options(&setting);
        (!options.is_empty()).then_some((setting, options))
    }

    /// Opens the focused dropdown with its current value highlighted.
    /// Returns whether one opened.
    pub fn open_menu(&mut self) -> bool {
        let Some((setting, options)) = self.focused_choice() else {
            return false;
        };
        let value = self.value(&setting);
        let current = value.as_str().unwrap_or_default();
        self.menu = Some(
            options
                .iter()
                .position(|(option, _)| option.eq_ignore_ascii_case(current))
                .unwrap_or(0),
        );
        true
    }

    /// Moves the open dropdown's highlight by `delta`.
    pub fn move_menu(&mut self, delta: isize) {
        let len = self
            .focused_choice()
            .map_or(0, |(_, options)| options.len());
        if let Some(at) = self.menu {
            self.menu = Some(crate::focus::move_highlight(len, at, delta));
        }
    }

    /// The open dropdown's highlighted choice, as `(setting key, value)`.
    #[must_use]
    pub fn menu_choice(&self) -> Option<(String, String)> {
        let (setting, options) = self.focused_choice()?;
        let (value, _) = options.into_iter().nth(self.menu?)?;
        Some((setting.key, value))
    }

    /// The selected row's key.
    #[must_use]
    pub fn selected_key(&self) -> &str {
        usize::try_from(self.selected).map_or("", |row| self.sidebar.key_at(row))
    }

    /// What the content pane shows.
    #[must_use]
    pub fn shown(&self) -> Shown<'_> {
        let Ok(row) = usize::try_from(self.selected) else {
            return Shown::Nothing;
        };
        let Some(entry) = self.sidebar.rows().get(row) else {
            return Shown::Nothing;
        };
        match entry.kind {
            SidebarKind::Core => CorePage::ALL
                .into_iter()
                .find(|page| page.id() == entry.key)
                .map_or(Shown::Nothing, Shown::Core),
            SidebarKind::Group | SidebarKind::Extension | SidebarKind::Command => self
                .providers
                .iter()
                .find(|provider| provider.id == entry.key)
                .map_or(Shown::Nothing, Shown::Provider),
            SidebarKind::Divider => Shown::Nothing,
        }
    }

    /// The settings a core page shows, in order.
    #[must_use]
    pub fn core_settings(page: CorePage) -> Vec<Setting> {
        settings_catalog::catalog()
            .into_iter()
            .filter(|setting| setting.scope == Scope::Core(page))
            .collect()
    }

    /// The settings a provider's page shows for the provider itself.
    #[must_use]
    pub fn provider_settings(provider: &str) -> Vec<Setting> {
        settings_catalog::catalog()
            .into_iter()
            .filter(|setting| matches!(&setting.scope, Scope::Provider(id) if *id == provider))
            .collect()
    }

    /// The settings shown under one root item.
    #[must_use]
    pub fn item_settings(id: &str) -> Vec<Setting> {
        settings_catalog::catalog()
            .into_iter()
            .filter(|setting| matches!(&setting.scope, Scope::Command(item) if item == id))
            .collect()
    }

    /// What a setting holds now.
    #[must_use]
    pub fn value(&self, setting: &Setting) -> serde_json::Value {
        settings_catalog::value(&self.config, setting)
    }

    /// What a text field shows: the draft as typed, else the value as text.
    #[must_use]
    pub fn text_of(&self, setting: &Setting) -> String {
        if let Some(draft) = self.drafts.get(&setting.key) {
            return draft.clone();
        }
        match self.value(setting) {
            serde_json::Value::String(text) if setting.kind == settings_catalog::Kind::Font => {
                if text == "auto" {
                    String::new()
                } else {
                    text
                }
            }
            serde_json::Value::String(text) => text,
            serde_json::Value::Array(items) => items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(if setting.kind == settings_catalog::Kind::Names {
                    ", "
                } else {
                    ":"
                }),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        }
    }

    /// The value a submitted text field means, for the setting it edits: a
    /// number for a number, the folders of a `:`-separated list, the text
    /// otherwise; `null` for an emptied font, which resets it.
    ///
    /// # Errors
    ///
    /// The sentence to show when the text is not a number where one is due.
    pub fn parse_draft(setting: &Setting, draft: &str) -> Result<serde_json::Value, String> {
        use settings_catalog::Kind;
        Ok(match &setting.kind {
            Kind::Number { min, max } => draft
                .trim()
                .parse::<u64>()
                .map(serde_json::Value::from)
                .map_err(|_| {
                format!("{} takes a whole number from {min} to {max}", setting.label)
            })?,
            Kind::Paths => serde_json::Value::Array(
                draft
                    .split(':')
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(|path| serde_json::Value::String(path.to_owned()))
                    .collect(),
            ),
            Kind::Names => serde_json::Value::Array(
                draft
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| serde_json::Value::String(name.to_owned()))
                    .collect(),
            ),
            Kind::Font if draft.trim().is_empty() => serde_json::Value::Null,
            _ => serde_json::Value::String(draft.to_owned()),
        })
    }

    /// Applies a setting's new value to the view's copy of the file, as the
    /// engine will write it. Returns the sentence to show when it is not one
    /// the setting takes.
    ///
    /// # Errors
    ///
    /// As [`settings_catalog::apply`].
    pub fn apply(&mut self, key: &str, value: serde_json::Value) -> Result<(), String> {
        let applied = settings_catalog::apply(&mut self.config, key, value);
        if applied.is_ok() {
            self.drafts.remove(key);
            self.notice = None;
        }
        applied
    }

    /// A provider's switch, in the view.
    pub fn set_provider_enabled(&mut self, provider: &str, enabled: bool) {
        self.config.set_provider_enabled(provider, enabled);
        let infos = {
            if let Some(entry) = self.providers.iter_mut().find(|p| p.id == provider) {
                entry.enabled = enabled;
            }
            provider_infos(&self.providers)
        };
        let key = self.selected_key().to_owned();
        let query = self.query.clone();
        self.sidebar = SidebarModel::new(&infos);
        self.sidebar.set_query(query, &infos);
        let row = self.sidebar.index_of_key(&key);
        if row >= 0 {
            self.selected = row;
        }
    }

    /// An item's switch, alias or shortcut, in the view.
    pub fn edit_item(&mut self, id: &str, edit: &compass_core::root_items::RootEdit) {
        use compass_core::root_items::RootEdit;
        self.config.apply_root_edit(id, edit);
        for item in self
            .providers
            .iter_mut()
            .flat_map(|provider| provider.items.iter_mut())
            .filter(|item| item.id == id)
        {
            match edit {
                RootEdit::Enabled(enabled) => item.enabled = *enabled,
                RootEdit::Disable => item.enabled = false,
                RootEdit::Alias(alias) => {
                    item.alias = (!alias.is_empty()).then(|| alias.clone());
                }
                RootEdit::Shortcut(shortcut) => {
                    item.shortcut = (!shortcut.is_empty()).then(|| shortcut.clone());
                }
                _ => {}
            }
        }
        self.drafts.remove(&alias_key(id));
    }

    /// The C++ settings a core page would show that Compass does not have.
    #[must_use]
    pub fn not_in_compass(page: CorePage) -> Vec<&'static settings_catalog::NotPorted> {
        settings_catalog::NOT_IN_COMPASS
            .iter()
            .filter(|missing| missing.page == page)
            .collect()
    }
}

fn provider_infos(providers: &[ProviderEntry]) -> Vec<ProviderInfo> {
    providers
        .iter()
        .map(|provider| ProviderInfo {
            id: provider.id.clone(),
            display_name: provider.title.clone(),
            is_group: provider.is_group,
            is_transient: false,
            enabled: provider.enabled,
        })
        .collect()
}

/// The providers of `index`'s root items, in the order their first item
/// comes, each with its items; `config` says which are on. A provider with
/// settings of its own (script directories) is listed even with no items.
#[must_use]
pub fn providers_of(
    index: &compass_core::AppIndex,
    config: &compass_core::root_items::RootConfig,
) -> Vec<ProviderEntry> {
    let mut providers: Vec<ProviderEntry> = Vec::new();
    for root in index.roots() {
        let provider_id = &root.meta.provider_id;
        let position = match providers.iter().position(|p| &p.id == provider_id) {
            Some(position) => position,
            None => {
                let extension = index
                    .extensions()
                    .iter()
                    .find(|command| &command.provider_id == provider_id);
                providers.push(ProviderEntry {
                    id: provider_id.clone(),
                    title: index
                        .provider_title(provider_id)
                        .unwrap_or_else(|| provider_id.clone()),
                    provenance: match extension {
                        Some(command) if command.is_raycast => "Raycast",
                        Some(_) => "Extension",
                        None => "Built-in",
                    },
                    enabled: config
                        .providers
                        .get(provider_id)
                        .and_then(|provider| provider.enabled)
                        != Some(false),
                    is_group: extension.is_none(),
                    items: Vec::new(),
                });
                providers.len() - 1
            }
        };
        providers[position].items.push(ItemEntry {
            id: root.id.clone(),
            title: root.title.clone(),
            enabled: root.meta.enabled,
            alias: root.meta.alias.clone(),
            shortcut: root.meta.shortcut.clone(),
            has_preferences: index
                .extension(&root.id)
                .is_some_and(|command| !command.preferences.is_empty()),
        });
    }
    for setting in settings_catalog::catalog() {
        if let Scope::Provider(id) = setting.scope
            && !providers.iter().any(|provider| provider.id == id)
        {
            providers.push(ProviderEntry {
                id: id.to_owned(),
                title: index
                    .provider_title(id)
                    .unwrap_or_else(|| provider_title_fallback(id)),
                provenance: "Built-in",
                enabled: config.providers.get(id).and_then(|p| p.enabled) != Some(false),
                is_group: true,
                items: Vec::new(),
            });
        }
    }
    providers
}

fn provider_title_fallback(id: &str) -> String {
    match id {
        compass_core::script_scan::SCRIPTS_PROVIDER_ID => "Script Commands".to_owned(),
        _ => id.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn providers() -> Vec<ProviderEntry> {
        vec![
            ProviderEntry {
                id: "applications".into(),
                title: "Applications".into(),
                provenance: "Built-in",
                enabled: true,
                is_group: true,
                items: vec![ItemEntry {
                    id: "applications:firefox".into(),
                    title: "Firefox".into(),
                    enabled: true,
                    alias: None,
                    shortcut: None,
                    has_preferences: false,
                }],
            },
            ProviderEntry {
                id: "@me/notes".into(),
                title: "Notes".into(),
                provenance: "Extension",
                enabled: true,
                is_group: false,
                items: vec![],
            },
        ]
    }

    fn page(tab: Option<&str>) -> SettingsPage {
        SettingsPage::new(compass_core::Config::default(), providers(), vec![], tab)
    }

    #[test]
    fn it_opens_at_general_or_at_the_tab_named() {
        assert_eq!(page(None).shown(), Shown::Core(CorePage::General));
        assert_eq!(
            page(Some("shortcuts")).shown(),
            Shown::Core(CorePage::Keybindings)
        );
        assert_eq!(page(Some("about")).shown(), Shown::Core(CorePage::About));
        let opened = page(Some("applications:firefox"));
        assert!(matches!(opened.shown(), Shown::Provider(p) if p.id == "applications"));
    }

    #[test]
    fn the_filter_shows_the_best_match_and_nothing_when_nothing_does() {
        let mut page = page(Some("@me/notes"));
        page.set_query("notes".into());
        assert_eq!(page.selected_key(), "@me/notes");
        page.set_query("appear".into());
        assert_eq!(page.shown(), Shown::Core(CorePage::Appearance));
        page.set_query("zzzz".into());
        assert_eq!(page.shown(), Shown::Nothing);
    }

    #[test]
    fn a_setting_is_found_by_its_label_and_its_page_shown() {
        let mut page = page(Some("about"));
        page.set_query("quick launch".into());
        assert_eq!(page.shown(), Shown::Core(CorePage::General));
        assert_eq!(
            page.matches.first().map(String::as_str),
            Some("launcher.quick_launch")
        );
        assert_eq!(
            page.first_match_shown().as_deref(),
            Some("launcher.quick_launch")
        );
        assert!(page.is_match("launcher.quick_launch"));

        page.set_query("translucnt".into());
        assert_eq!(page.shown(), Shown::Core(CorePage::Appearance), "fuzzily");
        assert!(page.is_match("launcher.appearance.tint"));

        page.set_query(String::new());
        assert!(!page.is_match("launcher.appearance.tint"));
        assert_eq!(
            page.shown(),
            Shown::Core(CorePage::Appearance),
            "the page stays"
        );
    }

    #[test]
    fn a_setting_is_found_by_its_description() {
        let mut page = page(None);
        page.set_query("ctrl+1".into());
        assert!(page.is_match("launcher.quick_launch"), "{:?}", page.matches);
    }

    #[test]
    fn the_arrows_skip_the_dividers() {
        let mut page = page(Some("about"));
        page.step(true);
        assert_eq!(page.selected_key(), "applications");
        page.step(false);
        assert_eq!(page.selected_key(), "about");
    }

    #[test]
    fn a_draft_is_read_as_its_setting_takes_it() {
        let results = compass_core::settings_catalog::find("launcher.max_results").unwrap();
        assert_eq!(SettingsPage::parse_draft(&results, " 12 "), Ok(json!(12)));
        assert!(SettingsPage::parse_draft(&results, "twelve").is_err());
        let paths =
            compass_core::settings_catalog::find("providers.files.preferences.indexingPaths")
                .unwrap();
        assert_eq!(
            SettingsPage::parse_draft(&paths, "/a: /b ::"),
            Ok(json!(["/a", "/b"]))
        );
        let font = compass_core::settings_catalog::find("font.normal.family").unwrap();
        assert_eq!(SettingsPage::parse_draft(&font, " "), Ok(json!(null)));
    }

    #[test]
    fn a_provider_switch_is_shown_in_the_sidebar() {
        let mut page = page(Some("applications"));
        page.set_provider_enabled("applications", false);
        let row = page
            .sidebar
            .rows()
            .iter()
            .find(|row| row.key == "applications")
            .unwrap();
        assert!(!row.enabled);
        assert_eq!(page.selected_key(), "applications");
        assert_eq!(
            page.config.root_config().providers["applications"].enabled,
            Some(false)
        );
    }
}
