//! Browse Apps, Set Default Browser and Set Default Terminal in the launcher.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the list's state is in [`crate::apps_page`].

use std::sync::Arc;

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, launch_task, mouse_area,
    next_selection, scrollable, text,
};
use crate::action_panel::Action;
use crate::apps_page::{self, AppRow, AppsKind, AppsPage};
use crate::backend::DefaultApp;
use compass_core::browse_apps::{self, ActionKind, Shortcut};

/// The panel's action ids: this prefix, then the position in the row's
/// actions.
const ACTION_PREFIX: &str = "apps.action.";

const NEEDS_ENGINE: &str = "Setting a default application needs the Compass engine, and this window is running without one";

impl LauncherApp {
    /// Opens Browse Apps over this window's index, with the command's
    /// preferences as the configuration holds them now (`showHidden`,
    /// `sortAlphabetically`, read on each opening as the C++ reads them).
    pub(super) fn open_browse_apps(&mut self) -> Task<Message> {
        let options = self
            .config_path
            .as_deref()
            .and_then(|path| match compass_core::Config::load_from(path) {
                Ok(config) => Some(browse_apps::Options::from_preferences(
                    config.entrypoint_preferences(
                        compass_core::commands::COMMANDS_PROVIDER_ID,
                        browse_apps::ENTRYPOINT,
                    ),
                )),
                Err(error) => {
                    tracing::debug!(%error, "Browse Apps keeps the preferences it started with");
                    None
                }
            })
            .unwrap_or(self.browse_apps);
        self.browse_apps = options;
        self.page = Page::Apps(AppsPage::browse(&self.app_index, options));
        self.warm_app_icons();
        Task::batch([focus_search(), self.apps_runtime_task()])
    }

    /// Asks the engine whether Browse Apps' selected application has a
    /// window open, for Focus Window.
    fn apps_runtime_task(&self) -> Task<Message> {
        let Page::Apps(page) = &self.page else {
            return Task::none();
        };
        let (AppsKind::Browse, Some(row), Some(windows)) =
            (page.kind, page.selected_row(), self.windows.clone())
        else {
            return Task::none();
        };
        let id = row.app.id.clone();
        let asked = id.clone();
        Task::perform(
            async move { windows.app_runtime(asked).await },
            move |result| Message::BrowseAppRuntime {
                id: id.clone(),
                result,
            },
        )
    }

    /// Opens Set Default Browser or Set Default Terminal and asks the engine
    /// for the candidates.
    pub(super) fn open_default_picker(&mut self, kind: DefaultApp) -> Task<Message> {
        self.page = Page::Apps(AppsPage::picker(kind));
        let Some(backend) = self.backend.clone() else {
            let index = &self.app_index;
            if let Page::Apps(page) = &mut self.page {
                page.apply(Err(NEEDS_ENGINE.to_owned()), index);
            }
            return focus_search();
        };
        Task::batch([
            Task::perform(
                async move { backend.list_default_apps(kind).await },
                Message::DefaultAppsLoaded,
            ),
            focus_search(),
        ])
    }

    /// The actions a row offers, in panel order: Browse Apps' panel as
    /// [`browse_apps::action_panel`] builds it, Focus Window first when the
    /// application has a window open, or the picker's one action.
    fn app_actions(
        &self,
        kind: AppsKind,
        row: &AppRow,
        windows: &[String],
    ) -> Vec<(String, Option<String>, Act)> {
        match kind {
            AppsKind::Browse => {
                let panel = browse_apps::action_panel(&row.app, windows, self.backend.is_some());
                panel
                    .actions
                    .into_iter()
                    .filter_map(|action| {
                        let shortcut = action.shortcut.map(|shortcut| match shortcut {
                            Shortcut::Literal(chord) => chord,
                            Shortcut::Keybind(_) => browse_apps::OPEN_KEYBIND_DEFAULT.to_owned(),
                        });
                        let act = match action.kind {
                            ActionKind::OpenApp => Act::Open(None),
                            ActionKind::OpenDesktopAction { id } => Act::Open(Some(id)),
                            ActionKind::OpenLocation => Act::OpenLocation,
                            ActionKind::CopyAppId => Act::Copy(row.app.id.clone()),
                            ActionKind::CopyAppLocation => Act::Copy(row.app.path.clone()),
                            ActionKind::FocusWindow { window } => Act::Focus(window.parse().ok()?),
                        };
                        Some((action.title, shortcut, act))
                    })
                    .collect()
            }
            AppsKind::Default(kind) => {
                let title = match kind {
                    DefaultApp::Browser => compass_core::default_app::BROWSER_ACTION,
                    DefaultApp::Terminal => compass_core::default_app::TERMINAL_ACTION,
                };
                vec![(title.to_owned(), None, Act::SetDefault(kind))]
            }
        }
    }

    /// The selected row's actions.
    fn selected_app_actions(&self) -> Vec<(String, Option<String>, Act)> {
        let Page::Apps(page) = &self.page else {
            return Vec::new();
        };
        let Some(row) = page.selected_row() else {
            return Vec::new();
        };
        let windows: Vec<String> = page
            .running
            .as_ref()
            .filter(|(id, _)| *id == row.app.id)
            .map(|(_, windows)| windows.iter().map(u32::to_string).collect())
            .unwrap_or_default();
        self.app_actions(page.kind, row, &windows)
    }

    /// Carries out one of the selected row's actions.
    fn act_on_app(&mut self, act: Act) -> Task<Message> {
        self.panel = None;
        let Page::Apps(page) = &self.page else {
            return Task::none();
        };
        let Some(row) = page.selected_row() else {
            return Task::none();
        };
        match act {
            Act::Open(action) => {
                let Some(item) = &row.item else {
                    return Task::none();
                };
                // `open->setClearSearch(true)`.
                self.query.clear();
                launch_task(
                    Arc::clone(&self.launcher),
                    item.entry().clone(),
                    action,
                    self.backend.clone(),
                    item.key().to_owned(),
                )
            }
            Act::OpenLocation => {
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                let path = row.app.path.clone();
                Task::perform(
                    async move { backend.open_file(path, false).await },
                    Message::FileOpened,
                )
            }
            Act::Copy(text) => self.copy_with_hud(text),
            Act::Focus(window) => {
                let Some(windows) = self.windows.clone() else {
                    return Task::none();
                };
                Task::perform(
                    async move { windows.activate_window(window).await },
                    Message::AppQuit,
                )
            }
            Act::SetDefault(kind) => {
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                let id = row.app.id.clone();
                Task::perform(
                    async move { backend.set_default_app(kind, id).await },
                    Message::DefaultAppSet,
                )
            }
        }
    }

    /// Opens the action panel over the selected row.
    pub(super) fn open_apps_panel(&mut self) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Apps(_)) {
            return None;
        }
        let actions = self.selected_app_actions();
        if actions.is_empty() {
            return None;
        }
        let actions = actions
            .into_iter()
            .enumerate()
            .map(|(position, (title, shortcut, _))| {
                let action = Action::new(title).with_id(format!("{ACTION_PREFIX}{position}"));
                match (position, shortcut) {
                    (0, _) => action.with_shortcut("enter"),
                    (_, Some(chord)) => action.with_shortcut(chord),
                    (_, None) => action,
                }
            })
            .collect();
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions,
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a panel action, if `id` is one of this view's.
    pub(super) fn apps_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let position: usize = id.strip_prefix(ACTION_PREFIX)?.parse().ok()?;
        let (_, _, act) = self.selected_app_actions().into_iter().nth(position)?;
        Some(self.act_on_app(act))
    }

    /// The view's keys: the list's navigation, Enter for the first action,
    /// and Browse Apps' chords (`control+shift+1..9` for the desktop
    /// actions, `action.open` for the location).
    pub(super) fn apps_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        if modifiers.control()
            && let Key::Character(c) = key.as_ref()
        {
            let chord = if modifiers.shift() {
                // Shift turns the digit row into symbols on most layouts; the
                // physical digit is what the C++ shortcut names.
                digit_of(c).map(|digit| format!("control+shift+{digit}"))
            } else {
                Some(format!("control+{}", c.to_lowercase()))
            };
            if let Some(chord) = chord
                && let Some((_, _, act)) = self
                    .selected_app_actions()
                    .into_iter()
                    .find(|(_, shortcut, _)| shortcut.as_deref() == Some(chord.as_str()))
            {
                return self.act_on_app(act);
            }
        }
        let Page::Apps(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => {
                return match self.selected_app_actions().into_iter().next() {
                    Some((_, _, act)) => self.act_on_app(act),
                    None => Task::none(),
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
            return Task::batch([
                crate::scroll::reveal_root_selection(),
                self.apps_runtime_task(),
            ]);
        }
        Task::none()
    }

    /// Handles the view's messages.
    pub(super) fn apps_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::DefaultAppsLoaded(result) => {
                let index = &self.app_index;
                if let Page::Apps(page) = &mut self.page {
                    page.apply(result, index);
                }
                self.warm_app_icons();
            }
            Message::AppsQueryChanged(query) => {
                if let Page::Apps(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                self.warm_app_icons();
                return Task::batch([
                    crate::scroll::reveal_root_selection(),
                    self.apps_runtime_task(),
                ]);
            }
            Message::BrowseAppRuntime { id, result } => {
                let Page::Apps(page) = &mut self.page else {
                    return Task::none();
                };
                if page.selected_row().is_none_or(|row| row.app.id != id) {
                    return Task::none();
                }
                let windows = match result {
                    Ok(info) if info.running => info.windows.iter().map(|w| w.id).collect(),
                    Ok(_) => Vec::new(),
                    Err(reason) => {
                        tracing::debug!(%reason, "no answer on whether the application runs");
                        Vec::new()
                    }
                };
                page.running = Some((id, windows));
                // An open panel takes Focus Window in, keeping its filter.
                if let Some(filter) = self.panel.as_ref().map(|panel| panel.filter.clone()) {
                    let _ = self.open_apps_panel();
                    if let Some(panel) = self.panel.as_mut() {
                        panel.set_filter(filter);
                    }
                }
                return Task::none();
            }
            Message::AppsSelected(position) => {
                if let Page::Apps(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                    return match self.selected_app_actions().into_iter().next() {
                        Some((_, _, act)) => self.act_on_app(act),
                        None => Task::none(),
                    };
                }
            }
            // `showHud(...)` (the engine's notification) and `popToRoot()`.
            Message::DefaultAppSet(Ok(())) => {
                self.page = Page::Root;
                self.query.clear();
                self.results.clear();
                return self.conceal();
            }
            Message::DefaultAppSet(Err(reason)) => {
                if let Page::Apps(page) = &mut self.page {
                    page.notice = Some(reason);
                }
            }
            _ => {}
        }
        crate::scroll::reveal_root_selection()
    }

    /// Looks up the icons of the rows on screen, as the root list does.
    fn warm_app_icons(&mut self) {
        if !self.icons {
            return;
        }
        let Page::Apps(page) = &self.page else {
            return;
        };
        let names: Vec<&str> = page
            .shown
            .iter()
            .filter_map(|&index| page.rows[index].item.as_ref()?.icon())
            .collect();
        let find = self.icon_lookup.clone();
        self.icon_cache.warm(names, &|name| find.find(name));
    }

    /// The view's body.
    pub(super) fn apps_body<'a>(&'a self, page: &'a AppsPage) -> Element<'a, Message> {
        if let Some(empty) = page.empty_notice() {
            return match &page.notice {
                Some(notice) => column![self.notice(empty), self.notice(notice)].into(),
                None => self.notice(empty),
            };
        }
        let palette = self.palette();
        let mut list = column![
            container(
                text(page.heading())
                    .font(self.font())
                    .size(12)
                    .color(palette.muted.to_iced())
            )
            .padding(Padding::new(4.0).left(10).right(10))
        ]
        .spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let row = &page.rows[index];
            let selected = position == page.selected;
            let icon = match &row.item {
                Some(item) => self.app_icon(item, selected),
                None => self.initial_badge(&row.app.display_name, selected),
            };
            let subtitle = self
                .subtitles
                .then(|| row.app.description.clone())
                .filter(|description| !description.is_empty());
            let mark = row
                .is_default
                .then(|| {
                    self.glyph(
                        &crate::icons::default_mark(),
                        selected,
                        f32::from(self.geometry.subtitle_size) + 4.0,
                    )
                })
                .flatten();
            let item = match mark {
                Some(mark) => self.list_row_parts(
                    icon,
                    row.app.display_name.clone(),
                    subtitle,
                    Some(mark),
                    selected,
                ),
                None => self.list_row_with(
                    icon,
                    row.app.display_name.clone(),
                    subtitle,
                    apps_page::accessory(row),
                    selected,
                ),
            };
            let item: Element<Message> = mouse_area(item)
                .on_press(Message::AppsSelected(position))
                .into();
            let item: Element<Message> = if selected {
                container(item).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                item
            };
            list = list.push(item);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// The view, for assistive technology: the current default is said in
    /// words where the row shows a check.
    pub(super) fn apps_semantics(&self, page: &AppsPage) -> super::a11y::PageSemantics {
        let items = page
            .shown
            .iter()
            .filter_map(|&index| page.rows.get(index))
            .map(|row| {
                let accessory = if row.is_default {
                    Some("Default".to_owned())
                } else {
                    apps_page::accessory(row)
                };
                let description = [Some(row.app.description.clone()), accessory]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                super::a11y::item(row.app.display_name.clone(), Some(description))
            })
            .collect();
        super::a11y::PageSemantics::list(&page.heading(), items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }
}

/// What an action does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Act {
    /// Launch the application, or one of its desktop actions.
    Open(Option<String>),
    /// Open the desktop file with its default application.
    OpenLocation,
    /// Copy text.
    Copy(String),
    /// Focus and raise one of its windows.
    Focus(u32),
    /// Make it the default.
    SetDefault(DefaultApp),
}

/// The digit a key on the digit row stands for, shifted or not (US layout).
fn digit_of(key: &str) -> Option<char> {
    const SHIFTED: [char; 9] = ['!', '@', '#', '$', '%', '^', '&', '*', '('];
    let c = key.chars().next()?;
    if ('1'..='9').contains(&c) {
        return Some(c);
    }
    let position = SHIFTED.iter().position(|&s| s == c)?;
    char::from_digit(u32::try_from(position).ok()? + 1, 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::DefaultAppRow;
    use crate::semantics::Body;

    #[test]
    fn a_picker_lists_the_applications_and_says_which_is_the_default() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = AppsPage::picker(DefaultApp::Terminal);
        assert_eq!(
            app.apps_semantics(&page).status.as_deref(),
            Some("Looking for applications…")
        );
        page.apply(
            Ok(vec![
                DefaultAppRow {
                    id: "console.desktop".into(),
                    name: "Console".into(),
                    description: "Terminal emulator".into(),
                    is_default: true,
                },
                DefaultAppRow {
                    id: "xterm.desktop".into(),
                    name: "XTerm".into(),
                    ..DefaultAppRow::default()
                },
            ]),
            &app.app_index,
        );
        page.selected = 1;
        app.page = Page::Apps(page);
        let semantics = app.semantics();
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Available terminal emulators");
        assert_eq!(list.items[0].name, "Console");
        assert_eq!(
            list.items[0].description.as_deref(),
            Some("Terminal emulator · Default")
        );
        assert_eq!(list.items[1].name, "XTerm");
        assert_eq!(list.items[1].description, None);
        assert_eq!(list.selected, Some(1));
    }
}
