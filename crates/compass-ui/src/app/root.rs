//! The root search's own behaviour beyond ranking (`RootViewHost`,
//! `RootSearchModel`, `RootSearchActionGenerator`): the favourites above the
//! suggestions, the row's action panel, the alias form, the space-bar alias
//! shortcut, the up arrow's search history and the status bar's clock.

use compass_core::root_items::RootEdit;
use compass_core::root_view;

use super::{
    Confirm, ConfirmAction, Direction, LauncherApp, Message, Page, PanelSection, PanelState,
    ProviderScope, RootRow, Task, focus_search,
};
use crate::action_panel::Action;
use crate::backend::{PreferenceInput, PreferenceInputKind};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};
use crate::shortcut_recorder::{Outcome as RecorderOutcome, ShortcutRecorder};

const OPEN: &str = "root.open";
const COPY_DEEPLINK: &str = "root.copy-deeplink";
const RESET_RANKING: &str = "root.reset-ranking";
const FAVORITE: &str = "root.favorite";
const UNFAVORITE: &str = "root.unfavorite";
const FAVORITE_DOWN: &str = "root.favorite-down";
const FAVORITE_UP: &str = "root.favorite-up";
const ALIAS: &str = "root.alias";
const COPY_ID: &str = "root.copy-id";
const SHORTCUT: &str = "root.shortcut";
const DISABLE: &str = "root.disable";
const PREFERENCES: &str = "root.preferences";

/// The heading over the favourites.
pub(super) const FAVORITES_HEADING: &str = "Favorites";

/// The heading over the rest of the empty query's rows.
pub(super) const SUGGESTIONS_HEADING: &str = "Suggestions";

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

impl LauncherApp {
    /// The `provider:entrypoint` id of a root row, for the rows that are
    /// root items.
    pub(super) fn root_id(&self, row: RootRow) -> Option<String> {
        use compass_core::root_items::entrypoint_id;
        match row {
            RootRow::App(index) => {
                let item = self.app_index.items().get(index)?;
                Some(entrypoint_id(
                    compass_core::root_items::APPS_PROVIDER_ID,
                    &compass_core::root_items::app_entrypoint_id(item.key()),
                ))
            }
            RootRow::Command(command) => Some(command.id()),
            RootRow::Extension(index) => Some(self.app_index.extensions().get(index)?.id.clone()),
            RootRow::Shortcut(index) => Some(entrypoint_id(
                compass_core::shortcut::SHORTCUTS_PROVIDER_ID,
                &self.app_index.shortcuts().get(index)?.id,
            )),
            RootRow::Script(index) => Some(entrypoint_id(
                compass_core::script_scan::SCRIPTS_PROVIDER_ID,
                &self.app_index.scripts().get(index)?.id,
            )),
            RootRow::RhaiScript(index) => {
                Some(self.app_index.rhai_scripts().get(index)?.entrypoint_id())
            }
            RootRow::Calculator | RootRow::Fallback(_) | RootRow::Update => None,
        }
    }

    /// The root row an entrypoint id names, if this window knows it.
    fn row_for_id(&self, id: &str) -> Option<RootRow> {
        if let Some(command) = compass_core::commands::by_id(id) {
            return Some(RootRow::Command(command));
        }
        if let Some(index) = self
            .app_index
            .extensions()
            .iter()
            .position(|command| command.id == id)
        {
            return Some(RootRow::Extension(index));
        }
        if let Some(shortcut) = self.app_index.shortcut_by_entrypoint(id) {
            return self
                .app_index
                .shortcuts()
                .iter()
                .position(|known| known.id == shortcut.id)
                .map(RootRow::Shortcut);
        }
        if let Some(script) = self.app_index.script_by_entrypoint(id) {
            return self
                .app_index
                .scripts()
                .iter()
                .position(|known| known.id == script.id)
                .map(RootRow::Script);
        }
        if let Some(script) = self.app_index.rhai_script_by_entrypoint(id) {
            return self
                .app_index
                .rhai_scripts()
                .iter()
                .position(|known| known.id == script.id)
                .map(RootRow::RhaiScript);
        }
        let position = self.app_index.position_by_entrypoint(id)?;
        (!self.app_index.items()[position].is_action()).then_some(RootRow::App(position))
    }

    /// For the empty query, puts the favourites first, in the order they
    /// were arranged, and takes them out of the suggestions below
    /// (`queryFavorites`, and the search that leaves favourites out). A
    /// favourite that is disabled or unknown here is not shown.
    pub(super) fn apply_favorites(&mut self) {
        self.favorites_len = 0;
        if !self.query.is_empty() || !matches!(self.page, Page::Root) {
            return;
        }
        let favorites: Vec<RootRow> = self
            .root_config
            .favorites
            .iter()
            .filter(|id| self.app_index.root(id).is_none_or(|root| root.meta.enabled))
            .filter_map(|id| self.row_for_id(id))
            .collect();
        self.results.retain(|row| !favorites.contains(row));
        self.favorites_len = favorites.len();
        self.results.splice(0..0, favorites);
    }

    /// The heading drawn above the root row at `position`, if a section
    /// starts there: only the empty query has sections, and only when it
    /// has favourites or an update.
    pub(super) fn root_heading_at(&self, position: usize) -> Option<&'static str> {
        if !self.query.is_empty() {
            return None;
        }
        // The Update section, when it leads, heads its row and moves the
        // others down one (`RootUpdateSection` before the favourites).
        let position = if self.update_shown() {
            if position == 0 {
                return Some(super::release_check::UPDATE_HEADING);
            }
            if position == 1 && self.favorites_len == 0 {
                return Some(SUGGESTIONS_HEADING);
            }
            position - 1
        } else {
            position
        };
        if self.favorites_len == 0 {
            return None;
        }
        if position == 0 {
            return Some(FAVORITES_HEADING);
        }
        (position == self.favorites_len).then_some(SUGGESTIONS_HEADING)
    }

    /// The panel over the selected root row: its own actions (an
    /// application's, or opening), then the item actions every root item has
    /// (`RootSearchActionGenerator::generateActions`).
    pub(super) fn open_root_panel(&mut self) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root) {
            return None;
        }
        let row = self.selected_row()?;
        let id = self.root_id(row)?;
        let mut sections = match (row, self.selected_item()) {
            (RootRow::App(_), Some(item)) => super::actions_for_app(item),
            _ => vec![PanelSection {
                name: String::new(),
                actions: vec![
                    Action::new(match row {
                        RootRow::Command(_) | RootRow::Extension(_) => "Open command",
                        _ => "Open",
                    })
                    .with_id(OPEN)
                    .with_shortcut("enter"),
                ],
            }],
        };
        let favorite = self.root_config.favorites.iter().position(|fav| *fav == id);
        let mut item = vec![
            Action::new("Copy deeplink")
                .with_id(COPY_DEEPLINK)
                .with_shortcut("ctrl+shift+c"),
            Action::new("Reset ranking").with_id(RESET_RANKING),
            match favorite {
                Some(_) => Action::new("Remove from favorites").with_id(UNFAVORITE),
                None => Action::new("Add to favorites").with_id(FAVORITE),
            },
        ];
        if let Some(at) = favorite {
            if at + 1 < self.root_config.favorites.len() {
                item.push(
                    Action::new("Move down in favorites")
                        .with_id(FAVORITE_DOWN)
                        .with_shortcut("ctrl+shift+down"),
                );
            }
            if at != 0 {
                item.push(
                    Action::new("Move up in favorites")
                        .with_id(FAVORITE_UP)
                        .with_shortcut("ctrl+shift+up"),
                );
            }
        }
        item.push(Action::new("Set alias").with_id(ALIAS));
        item.push(Action::new("Set global shortcut").with_id(SHORTCUT));
        item.push(Action::new("Open preferences").with_id(PREFERENCES));
        item.push(Action::new("Copy ID").with_id(COPY_ID));
        item.push(
            Action::new("Disable item")
                .with_id(DISABLE)
                .with_shortcut("ctrl+x"),
        );
        sections.push(PanelSection {
            name: String::new(),
            actions: item,
        });
        self.panel = Some(PanelState::new(sections));
        let focus = iced::widget::operation::focus(super::PANEL_INPUT);
        // An application's panel grows its running-only actions once the
        // engine says it runs (`AppRootItem::newActionPanel`).
        if let (RootRow::App(_), Some(item)) = (row, self.selected_item()) {
            let (key, desktop_id) = (item.key().to_owned(), item.desktop_id().to_owned());
            self.app_runtime = None;
            return Some(Task::batch([focus, self.app_runtime_task(key, desktop_id)]));
        }
        Some(focus)
    }

    /// Runs a root item action, if `action` is one.
    pub(super) fn root_panel_action(&mut self, action: &str) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root) {
            return None;
        }
        let row = self.selected_row()?;
        let id = self.root_id(row)?;
        let task = match action {
            OPEN => {
                self.panel = None;
                return Some(self.update(Message::LaunchSelected));
            }
            COPY_DEEPLINK => {
                let link = compass_core::root_items::deeplink(&id)?;
                self.panel = None;
                return Some(self.copy_with_hud(link));
            }
            COPY_ID => {
                self.panel = None;
                return Some(self.copy_with_hud(id));
            }
            RESET_RANKING => {
                self.confirm = Some(Confirm {
                    title: "Are you sure?".to_owned(),
                    message: "You will have to rebuild search history for this item in order \
                              for it to reappear on top of the root search results."
                        .to_owned(),
                    confirm_text: "Reset".to_owned(),
                    action: ConfirmAction::RootEdit(id, RootEdit::ResetRanking),
                });
                Task::none()
            }
            DISABLE => {
                self.confirm = Some(Confirm {
                    title: "Are you sure?".to_owned(),
                    message: "You will need to go in the settings to manually re-enable it."
                        .to_owned(),
                    confirm_text: "Disable".to_owned(),
                    action: ConfirmAction::RootEdit(id, RootEdit::Disable),
                });
                Task::none()
            }
            FAVORITE => self.edit_root_item(id, RootEdit::Favorite(true)),
            UNFAVORITE => self.edit_root_item(id, RootEdit::Favorite(false)),
            FAVORITE_DOWN => self.edit_root_item(id, RootEdit::MoveFavorite { down: true }),
            FAVORITE_UP => self.edit_root_item(id, RootEdit::MoveFavorite { down: false }),
            ALIAS => {
                self.panel = None;
                return Some(self.open_alias_form(row, id));
            }
            // `OpenItemPreferencesAction`: the settings, at the item's
            // provider.
            PREFERENCES => {
                self.panel = None;
                return Some(self.open_settings(Some(&id)));
            }
            SHORTCUT => {
                let title = self.root_title(row).unwrap_or_default();
                let current = self
                    .app_index
                    .root(&id)
                    .and_then(|root| root.meta.shortcut.clone());
                if let Some(panel) = self.panel.as_mut() {
                    panel.recorder = Some(ShortcutRecorder::new(id, title, current));
                }
                return Some(Task::none());
            }
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// A key event while the shortcut recorder shows: an accepted chord is
    /// kept for the item and closes the panel, Escape goes back to the
    /// actions (`SetRootItemShortcutAction`'s accept handler, `setShortcut`).
    pub(super) fn recorder_event(&mut self, event: &iced::keyboard::Event) -> Task<Message> {
        let bound = self.recorder_bound();
        let launcher_hotkey = self.launcher_hotkey.clone();
        let Some(recorder) = self
            .panel
            .as_mut()
            .and_then(|panel| panel.recorder.as_mut())
        else {
            return Task::none();
        };
        let outcome = recorder.key(
            event,
            Some(&launcher_hotkey),
            bound
                .iter()
                .map(|(id, title, shortcut)| (id.as_str(), title.as_str(), shortcut.as_str())),
        );
        self.panel_recorder_outcome(outcome)
    }

    /// What the action panel's recorder said to do.
    pub(super) fn panel_recorder_outcome(&mut self, outcome: RecorderOutcome) -> Task<Message> {
        match outcome {
            RecorderOutcome::Recording => Task::none(),
            RecorderOutcome::Probe(trigger) => self.probe_shortcut(trigger),
            RecorderOutcome::Back => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.recorder = None;
                }
                iced::widget::operation::focus(super::PANEL_INPUT)
            }
            RecorderOutcome::Save(shortcut) => {
                let Some(id) = self
                    .panel
                    .as_ref()
                    .and_then(|panel| panel.recorder.as_ref())
                    .map(|recorder| recorder.id.clone())
                else {
                    return Task::none();
                };
                self.panel = None;
                self.edit_root_item(id, RootEdit::Shortcut(shortcut))
            }
        }
    }

    /// Applies `edit` to this window's root settings at once, and asks the
    /// engine to keep it; the list is searched again when it has.
    pub(super) fn edit_root_item(&mut self, id: String, edit: RootEdit) -> Task<Message> {
        if compass_core::root_items::apply_edit(&mut self.root_config, &id, &edit) {
            self.app_index.apply_root_config(&self.root_config);
        }
        match self.backend.clone() {
            Some(backend) => Task::batch([
                Task::perform(
                    async move { backend.edit_root_item(id, edit).await },
                    Message::RootItemEdited,
                ),
                focus_search(),
            ]),
            None => Task::batch([self.search_task(), focus_search()]),
        }
    }

    /// The engine's answer to a root edit: search again, or say why not.
    pub(super) fn root_item_edited(&mut self, result: Result<(), String>) -> Task<Message> {
        if let Err(reason) = result {
            match &mut self.page {
                Page::Fallbacks(page) => page.notice = Some(reason),
                _ => self.error = Some(reason),
            }
            return Task::none();
        }
        if matches!(self.page, Page::Root) {
            let selected = self.selected;
            let task = self.search_task();
            self.selected = selected;
            return task;
        }
        Task::none()
    }

    /// The alias form (`AliasFormViewHost`): one field, holding the alias
    /// the item has.
    fn open_alias_form(&mut self, row: RootRow, id: String) -> Task<Message> {
        let existing = self
            .app_index
            .root(&id)
            .and_then(|root| root.meta.alias.clone());
        let title = self.root_title(row).unwrap_or_default();
        let field = PreferenceInput {
            name: "alias".to_owned(),
            title: "Alias".to_owned(),
            description: String::new(),
            placeholder: String::new(),
            required: false,
            kind: PreferenceInputKind::Text,
            value: Some(serde_json::Value::String(
                root_view::alias_form_initial_value(existing.as_deref()),
            )),
        };
        self.page = Page::Preferences(Box::new(PreferencesPage::new(
            Purpose::Alias,
            id,
            root_view::alias_form_title(&title),
            vec![field],
        )));
        Task::none()
    }

    /// A root row's title, for the alias form's.
    fn root_title(&self, row: RootRow) -> Option<String> {
        Some(match row {
            RootRow::App(index) => self.app_index.items().get(index)?.name().to_owned(),
            RootRow::Command(command) => command.title.to_owned(),
            RootRow::Fallback(fallback) => return self.fallback_title(fallback),
            RootRow::Extension(index) => self.app_index.extensions().get(index)?.title.clone(),
            RootRow::Shortcut(index) => {
                crate::shortcuts_page::display_name(self.app_index.shortcuts().get(index)?)
                    .to_owned()
            }
            RootRow::Script(index) => self.app_index.scripts().get(index)?.title.clone(),
            RootRow::RhaiScript(index) => self.app_index.rhai_scripts().get(index)?.title.clone(),
            RootRow::Calculator | RootRow::Update => return None,
        })
    }

    /// Saves the alias form and goes back to the root. `None` when the form
    /// showing is not the alias form.
    pub(super) fn submit_alias_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::Alias {
            return None;
        }
        let id = form.command_id.clone();
        let alias = match form.values.first() {
            Some(FieldValue::Text(text)) => text.clone(),
            _ => String::new(),
        };
        self.page = Page::Root;
        Some(self.edit_root_item(id, RootEdit::Alias(alias)))
    }

    /// Escape from the alias form: back to the root, unchanged.
    pub(super) fn back_from_alias_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::Alias {
            return None;
        }
        self.page = Page::Root;
        Some(focus_search())
    }

    /// Whether the row can be opened by its alias and a space
    /// (`supportsAliasSpaceShortcut`): a command that opens a view.
    fn supports_alias_space(&self, row: RootRow) -> bool {
        match row {
            RootRow::Command(command) => compass_core::commands::opens_a_view(command.kind),
            RootRow::Extension(index) => self
                .app_index
                .extensions()
                .get(index)
                .is_some_and(|command| command.mode == compass_core::manifest::CommandMode::View),
            _ => false,
        }
    }

    /// Whether the row takes arguments, which the C++ asks for in the search
    /// bar's completer and this launcher in an arguments form.
    fn has_completer(&self, row: RootRow) -> bool {
        let index = &self.app_index;
        match row {
            RootRow::Shortcut(at) => index
                .shortcuts()
                .get(at)
                .is_some_and(|shortcut| !shortcut.link.arguments.is_empty()),
            RootRow::Extension(at) => index
                .extensions()
                .get(at)
                .is_some_and(|command| !command.arguments.is_empty()),
            RootRow::Script(at) => index
                .scripts()
                .get(at)
                .is_some_and(|script| !script.arguments.is_empty()),
            _ => false,
        }
    }

    /// The space-bar alias shortcut (`tryAliasFastTrack`): a space typed
    /// after exactly the selected item's alias opens it rather than being
    /// typed. An item that takes arguments opens its arguments form, where
    /// the C++ focuses the completer's first field (nothing has been typed
    /// into it yet, as the form is not open). `None` lets the text through.
    pub(super) fn alias_space(&mut self, typed: &str) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root) || self.panel.is_some() {
            return None;
        }
        if typed.strip_suffix(' ') != Some(self.query.as_str()) {
            return None;
        }
        let row = self.selected_row()?;
        let id = self.root_id(row)?;
        let alias = self.app_index.root(&id)?.meta.alias.clone();
        let outcome = root_view::space_outcome(
            &self.query,
            alias.as_deref(),
            self.has_completer(row),
            true,
            self.supports_alias_space(row),
        );
        matches!(
            outcome,
            root_view::SpaceOutcome::Activate | root_view::SpaceOutcome::FocusCompleter
        )
        .then(|| self.update(Message::LaunchSelected))
    }

    /// Remembers the search an action ran from (`beforeActionExecuted`).
    pub(super) fn record_search(&mut self) {
        let now = u64::try_from(now_secs()).unwrap_or_default();
        if !self.search_history.add(&self.query, now) {
            return;
        }
        if let Some(path) = &self.search_history_path
            && let Err(err) = self.search_history.save_file(path)
        {
            tracing::warn!(%err, "could not save the search history");
        }
    }

    /// The up arrow at the top of the list reaches back through past
    /// searches (`inputFilter`), unless navigation wraps. `None` when the
    /// arrow should move the selection instead.
    pub(super) fn history_up(&mut self, direction: Option<Direction>) -> Option<Task<Message>> {
        if direction != Some(Direction::Up)
            || !matches!(self.page, Page::Root)
            || self.panel.is_some()
        {
            return None;
        }
        let selected = i32::try_from(self.selected).unwrap_or(i32::MAX);
        if !root_view::up_cycles_history(self.wrap_navigation, selected, 0) {
            return None;
        }
        let offset = root_view::next_history_offset(self.history_offset);
        let queries = self.search_history.queries();
        match root_view::history_entry_at(&queries, offset, &self.query) {
            Some((reached, query)) => {
                if self.history_offset.is_none() {
                    self.history_draft = Some(self.query.clone());
                }
                self.history_offset = Some(reached);
                self.query = query;
                self.error = None;
                Some(self.search_task())
            }
            None => {
                self.history_offset = Some(offset);
                Some(Task::none())
            }
        }
    }

    /// The down arrow while the up arrow has reached into history: back
    /// towards the newest search, then to what was being typed before.
    pub(super) fn history_down(&mut self, direction: Option<Direction>) -> Option<Task<Message>> {
        if direction != Some(Direction::Down)
            || !matches!(self.page, Page::Root)
            || self.panel.is_some()
        {
            return None;
        }
        let offset = self.history_offset?;
        let queries = self.search_history.queries();
        match root_view::newer_history_entry(&queries, offset, &self.query) {
            Some((reached, query)) => {
                self.history_offset = Some(reached);
                self.query = query;
            }
            None => {
                self.history_offset = None;
                self.query = self.history_draft.take().unwrap_or_default();
            }
        }
        self.error = None;
        Some(self.search_task())
    }

    /// A `compass://launch/...` (or `vicinae://`) deeplink the engine handed over: a
    /// provider's search view, or an item launched with the link's text.
    pub(super) fn open_launch_link(
        &mut self,
        link: compass_core::root_items::LaunchLink,
    ) -> Task<Message> {
        use compass_core::root_items::LaunchTarget;
        let index = &self.app_index;
        match link.target(|id| index.has_provider(id)) {
            Ok(LaunchTarget::Provider(id)) => self.open_provider_search(&id, link.fallback_text),
            Ok(LaunchTarget::Entrypoint(id)) => {
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                let query = link.fallback_text;
                Task::perform(
                    async move { backend.launch_command(id, query).await },
                    Message::BuiltinCommandDone,
                )
            }
            Err(reason) => {
                self.error = Some(reason);
                Task::none()
            }
        }
    }

    /// Opens the provider search view (`ProviderSearchViewHost`): root search
    /// over the items of `id` alone, every one of them for the empty query,
    /// titled `Search <provider>`, with `text` typed in. Leaving it closes
    /// the window, as the deeplink's `setInstantDismiss` does.
    pub(super) fn open_provider_search(&mut self, id: &str, text: Option<String>) -> Task<Message> {
        let Some(title) = self.app_index.provider_title(id) else {
            self.error = Some(format!("No provider has the id {id}"));
            return Task::none();
        };
        let closing = self.close_extension_view();
        self.panel = None;
        self.page = Page::Root;
        self.history_offset = None;
        self.history_draft = None;
        self.provider_scope = Some(ProviderScope {
            id: id.to_owned(),
            placeholder: format!("Search {title}"),
            title,
        });
        self.query = text.unwrap_or_default();
        Task::batch([closing, self.search_task(), focus_search()])
    }

    /// A second passed: redraws the clock when it is due
    /// (`scheduleNextClockTick`), on a multiple of its interval.
    pub(super) fn clock_tick(&mut self) {
        let Some(clock) = &self.clock else {
            self.clock_text = None;
            return;
        };
        let now = now_secs();
        if now < self.clock_next_at {
            return;
        }
        let interval = i64::try_from(clock.interval).unwrap_or(60);
        self.clock_text = Some(clock_text(&clock.format, now));
        self.clock_next_at = now + root_view::next_clock_tick_secs(now, interval);
    }
}

/// The clock's text at `now` (seconds since the epoch), in `format`, local
/// time.
#[must_use]
pub(super) fn clock_text(format: &str, now: i64) -> String {
    let Ok(at) = jiff::Timestamp::from_second(now) else {
        return String::new();
    };
    let at = at.to_zoned(jiff::tz::TimeZone::system());
    let small = |value: i8| u8::try_from(value).unwrap_or_default();
    let broken = compass_core::qt_date::DateTime {
        year: i32::from(at.year()),
        month: small(at.month()),
        day: small(at.day()),
        hour: small(at.hour()),
        minute: small(at.minute()),
        second: small(at.second()),
        millisecond: u16::try_from(at.millisecond()).unwrap_or_default(),
        weekday: small(at.weekday().to_monday_one_offset()),
        zone: at.strftime("%Z").to_string(),
    };
    compass_core::qt_date::format(format, &broken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{
        TestBackend, app, chord, extension_app, index, key_event, pressed, settle, shortcuts_app,
    };
    use std::sync::Arc;

    #[test]
    fn the_root_panel_favourites_aliases_and_the_up_arrow_recalls_searches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let history = dir.path().join("history").join("search-history.json");
        app.search_history_path = Some(history.clone());

        // Favouriting Firefox from its panel puts it first, under a heading.
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let firefox = app.selected_row().expect("a row");
        let id = app.root_id(firefox).expect("a root item");
        let _ = app.update(Message::TogglePanel);
        let favorite = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Add to favorites"))
            .expect("the panel offers to favourite");
        let _ = app.update(Message::PanelClicked(favorite));
        assert_eq!(app.root_config.favorites, std::slice::from_ref(&id));
        let _ = app.update(Message::QueryChanged(String::new()));
        assert_eq!(app.results.first(), Some(&firefox));
        assert_eq!(app.root_heading_at(0), Some(FAVORITES_HEADING_FOR_TESTS));
        assert_eq!(
            app.results.iter().filter(|row| **row == firefox).count(),
            1,
            "a favourite is not suggested again"
        );

        // An alias through the form.
        let _ = app.update(Message::TogglePanel);
        let alias = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Set alias"))
            .expect("the panel offers an alias");
        let _ = app.update(Message::PanelClicked(alias));
        let _ = app.update(Message::PreferenceEdited(
            0,
            crate::preferences_page::FieldValue::Text("ff".into()),
        ));
        let _ = app.update(Message::PreferencesSubmit);
        assert!(matches!(app.page, Page::Root), "{}", app.state_line());
        assert_eq!(
            app.app_index
                .root(&id)
                .and_then(|root| root.meta.alias.as_deref()),
            Some("ff")
        );

        // Launching records the search; the up arrow at the top brings it back.
        let _ = app.update(Message::QueryChanged("term".into()));
        let _ = app.update(Message::LaunchSelected);
        let _ = app.update(Message::QueryChanged(String::new()));
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowUp));
        assert_eq!(app.query, "term");
        let stored = compass_core::root_view::SearchHistory::load_file(&history);
        assert_eq!(stored.queries(), ["term"]);

        // A query being typed is not lost to the up arrow: down brings it back.
        let _ = app.update(Message::QueryChanged("zzz".into()));
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowUp));
        assert_eq!(app.query, "term");
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(app.query, "zzz", "{}", app.state_line());
    }

    const FAVORITES_HEADING_FOR_TESTS: &str = "Favorites";

    #[test]
    fn the_root_panel_records_an_items_shortcut_and_backspace_removes_it() {
        use iced::keyboard::{Key, Modifiers, key::Named};
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let id = app.root_id(app.selected_row().unwrap()).unwrap();
        let open_recorder = |app: &mut LauncherApp| {
            let _ = app.update(Message::TogglePanel);
            assert!(!app.shortcuts_inhibited(), "the panel alone takes nothing");
            let row = app
                .panel
                .as_ref()
                .and_then(|panel| panel.row_titled("Set global shortcut"))
                .expect("the panel offers a shortcut");
            let _ = app.update(Message::PanelClicked(row));
            assert!(
                app.panel.as_ref().is_some_and(|p| p.recorder.is_some()),
                "the recorder takes the panel's place"
            );
            assert!(
                app.shortcuts_inhibited(),
                "the compositor's shortcuts reach the recorder"
            );
        };
        open_recorder(&mut app);

        // A bare letter is refused and typed nowhere; Control, then K.
        let _ = app.update(key_event(
            true,
            Key::Character("k".into()),
            Modifiers::empty(),
        ));
        let recorder = app
            .panel
            .as_ref()
            .and_then(|p| p.recorder.as_ref())
            .unwrap();
        assert_eq!(recorder.status, compass_core::key_combo::MODIFIER_REQUIRED);
        assert_eq!(app.query, "firefox");
        let _ = app.update(key_event(true, Key::Named(Named::Control), Modifiers::CTRL));
        let _ = app.update(key_event(true, Key::Character("k".into()), Modifiers::CTRL));
        assert!(app.panel.is_none(), "an accepted shortcut closes the panel");
        assert!(!app.shortcuts_inhibited(), "and gives the shortcuts back");
        let shortcut = |app: &LauncherApp| {
            app.app_index
                .root(&id)
                .and_then(|root| root.meta.shortcut.clone())
        };
        assert_eq!(shortcut(&app).as_deref(), Some("control+K"));
        let (provider, entrypoint) = compass_core::root_items::split_entrypoint_id(&id).unwrap();
        assert_eq!(
            app.root_config.providers[provider].entrypoints[entrypoint]
                .shortcut
                .as_deref(),
            Some("control+K")
        );

        // Escape goes back to the actions; Backspace removes the shortcut.
        let _ = app.update(key_event(
            false,
            Key::Named(Named::Control),
            Modifiers::empty(),
        ));
        open_recorder(&mut app);
        let _ = app.update(key_event(
            true,
            Key::Named(Named::Escape),
            Modifiers::empty(),
        ));
        assert!(app.panel.as_ref().is_some_and(|p| p.recorder.is_none()));
        assert!(!app.shortcuts_inhibited(), "Escape gives them back");
        let _ = app.update(Message::TogglePanel);
        open_recorder(&mut app);
        let _ = app.update(key_event(
            true,
            Key::Named(Named::Backspace),
            Modifiers::empty(),
        ));
        assert!(app.panel.is_none());
        assert_eq!(shortcut(&app), None);
    }

    #[test]
    fn a_launch_deeplink_to_a_provider_searches_its_items_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = extension_app(dir.path(), Arc::new(TestBackend::default()));

        let task = app.open_deeplink("vicinae://launch/@someone/hello?fallbackText=wri");
        settle(&mut app, task);
        assert_eq!(app.search_field().0, "Search Hello");
        assert_eq!(app.query, "wri");
        assert_eq!(app.results, [RootRow::Extension(0)], "{}", app.state_line());

        // The empty query lists every item of the provider, and nothing else:
        // no favourites, no calculator, no fallbacks.
        let task = app.open_deeplink("vicinae://launch/applications/");
        settle(&mut app, task);
        assert_eq!(app.search_field().0, "Search Applications");
        assert_eq!(app.results.len(), 3, "{}", app.state_line());
        assert!(app.results.iter().all(|row| matches!(row, RootRow::App(_))));
        let _ = app.update(Message::QueryChanged("fire".into()));
        assert!(
            app.results.iter().all(|row| matches!(row, RootRow::App(_))),
            "{}",
            app.state_line()
        );
        assert_eq!(app.results.len(), 1);

        // Leaving closes it: the next summon is the root again.
        let _ = app.update(Message::Dismiss);
        assert_eq!(app.provider_scope, None);
        assert_eq!(app.search_field().0, "Search…");

        let _ = app.open_deeplink("compass://launch/nothing");
        assert_eq!(
            app.error.as_deref(),
            Some(compass_core::root_items::INVALID_LAUNCH_LINK)
        );
    }

    #[test]
    fn a_launch_deeplink_to_an_item_launches_it_with_its_text() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = extension_app(dir.path(), backend.clone());
        let task = app.open_deeplink("vicinae://launch/@someone/hello/write?fallbackText=hi+there");
        settle(&mut app, task);
        assert_eq!(
            backend.launched.lock().unwrap().as_slice(),
            [(
                "@someone/hello:write".to_owned(),
                Some("hi there".to_owned())
            )]
        );
        assert_eq!(app.provider_scope, None);
    }

    #[test]
    fn an_alias_and_a_space_open_an_items_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, backend) = shortcuts_app(dir.path());
        compass_core::root_items::apply_edit(
            &mut app.root_config,
            "shortcuts:sct-docs",
            &compass_core::root_items::RootEdit::Alias("cd".into()),
        );
        app.app_index.apply_root_config(&app.root_config);
        app.query = "cd".into();
        app.search();
        assert!(
            matches!(app.selected_row(), Some(RootRow::Shortcut(_))),
            "{}",
            app.state_line()
        );
        let task = app.update(Message::QueryChanged("cd ".into()));
        settle(&mut app, task);
        assert!(
            matches!(&app.page, Page::Preferences(form)
                if form.purpose == crate::preferences_page::Purpose::ShortcutArguments),
            "the space opens the arguments rather than being typed: {}",
            app.state_line()
        );
        assert_eq!(app.query, "cd");
        assert!(backend.opened_shortcuts.lock().unwrap().is_empty());
    }

    #[test]
    fn ctrl_shift_c_copies_a_root_rows_deeplink_without_opening_the_panel() {
        use iced::keyboard::Modifiers;
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        open_builtin_row(&mut app, "clipboard history");
        assert!(app.panel.is_none());
        let task = app.update(chord("C", Modifiers::CTRL | Modifiers::SHIFT));
        assert_eq!(
            settle(&mut app, task),
            ["compass://launch/commands/clipboard-history"]
        );
        assert!(app.panel.is_none(), "no panel the person never opened");
        // A chord no action claims leaves the panel shut and the list alone.
        let selected = app.selected;
        let task = app.update(chord("y", Modifiers::CTRL | Modifiers::SHIFT));
        settle(&mut app, task);
        assert!(app.panel.is_none());
        assert_eq!(app.selected, selected);
    }

    /// Searches root for `query`, leaving its first row selected.
    fn open_builtin_row(app: &mut LauncherApp, query: &str) {
        app.query = query.into();
        app.search();
        assert!(app.selected_row().is_some(), "{}", app.state_line());
    }
}
