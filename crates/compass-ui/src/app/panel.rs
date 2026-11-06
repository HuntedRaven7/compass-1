//! The action panel: `Ctrl+B`'s list of what can be done to the selected
//! row, its filter, its keys, and running the chosen action.
//!
//! Each view supplies its own sections (`open_*_panel`) and runs its own
//! actions (`*_panel_action`); this module owns what is common to all of
//! them -- opening and closing, filtering, moving the selection, the
//! shortcuts an action advertises, and drawing the panel over the card.

use super::search::launch_task;
use super::view::query_input_style;
use super::{
    Action, Alignment, AppItem, Arc, Border, Color, Direction, Element, LauncherApp, Length,
    Message, PANEL_INPUT, Padding, Page, PanelSection, Row, RowKind, Space, Step, Task, Theme,
    action_panel, chord_direction, column, container, design, focus_search, mouse_area, row,
    scrollable, text, text_input,
};

const APP_OPEN: &str = "app.open";
const APP_COPY_NAME: &str = "app.copy-name";
const APP_COPY_PATH: &str = "app.copy-path";
const APP_DESKTOP_ACTION: &str = "app.desktop:";

/// Panel ids for an extension's actions: this prefix, then the handler.
const EXTENSION_ACTION: &str = "ext:";

/// The action panel's own state while it is open.
///
/// Its selection is an `isize` because -1 means "nothing selectable", which is
/// a real state for a panel filtered down to nothing and is not the same as
/// row 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelState {
    /// The sections, as the command supplied them.
    pub sections: Vec<PanelSection>,
    /// What has been typed into the panel's own filter.
    pub filter: String,
    /// The flattened rows under that filter.
    pub rows: Vec<Row>,
    /// The selected row, or -1.
    pub selected: isize,
    /// The shortcut recorder, when it has taken the panel's place.
    pub recorder: Option<crate::shortcut_recorder::ShortcutRecorder>,
}

impl PanelState {
    /// Open a panel over `sections`, selecting its first action.
    #[must_use]
    pub fn new(sections: Vec<PanelSection>) -> Self {
        let rows = action_panel::flatten(&sections, "");
        let selected = action_panel::selection_after_filter(&rows);
        Self {
            sections,
            filter: String::new(),
            rows,
            selected,
            recorder: None,
        }
    }

    /// Re-filter, and put the selection back on the first row of what is left.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.rows = action_panel::flatten(&self.sections, &self.filter);
        self.selected = action_panel::selection_after_filter(&self.rows);
    }

    /// The action the selection is on, if any.
    #[must_use]
    pub fn selected_action(&self) -> Option<&Action> {
        let row = self.rows.get(usize::try_from(self.selected).ok()?)?;
        self.sections.get(row.section)?.actions.get(row.action?)
    }

    /// The row of the action whose shortcut is the key `pressed`.
    #[must_use]
    pub fn row_answering(&self, pressed: &action_panel::Pressed<'_>) -> Option<usize> {
        self.rows.iter().position(|row| {
            row.action
                .and_then(|action| self.sections.get(row.section)?.actions.get(action))
                .is_some_and(|action| action.answers_to(pressed))
        })
    }

    /// The row of the action called `title`, for a test to click.
    #[cfg(test)]
    pub(super) fn row_titled(&self, title: &str) -> Option<usize> {
        self.rows.iter().position(|row| {
            row.action
                .and_then(|action| self.sections.get(row.section)?.actions.get(action))
                .is_some_and(|action| action.title == title)
        })
    }
}

impl LauncherApp {
    /// Opening, filtering, moving through and running the action panel.
    pub(super) fn panel_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TogglePanel => {
                if self.panel.is_some() {
                    self.panel = None;
                    return focus_search();
                } else if let Some(task) = self.open_shortcut_panel() {
                    return task;
                } else if let Some(task) = self.open_snippet_panel() {
                    return task;
                } else if let Some(task) = self.open_script_panel() {
                    return task;
                } else if let Some(task) = self.open_program_panel() {
                    return task;
                } else if let Some(task) = self.open_dmenu_panel() {
                    return task;
                } else if let Some(task) = self.open_font_panel() {
                    return task;
                } else if let Some(task) = self.open_store_panel() {
                    return task;
                } else if let Some(task) = self.open_media_panel() {
                    return task;
                } else if let Some(task) = self.open_theme_panel() {
                    return task;
                } else if let Some(task) = self.open_grants_panel() {
                    return task;
                } else if let Some(task) = self.open_tray_panel() {
                    return task;
                } else if let Some(task) = self.open_apps_panel() {
                    return task;
                } else if let Some(task) = self.open_emoji_panel() {
                    return task;
                } else if let Some(task) = self.open_clipboard_panel() {
                    return task;
                } else if let Some(task) = self.open_windows_panel() {
                    return task;
                } else if let Some(task) = self.open_calculator_panel() {
                    return task;
                } else if let Some(task) = self.open_fallbacks_panel() {
                    return task;
                } else if let Some(task) = self.open_compass_view_panel() {
                    return task;
                } else if let Some(task) = self.open_fallback_row_panel() {
                    return task;
                } else if let Some(task) = self.open_workspaces_panel() {
                    return task;
                } else if let Some(task) = self.open_files_panel() {
                    return task;
                } else if let Page::Extension(page) = &self.page {
                    let sections = extension_panel_sections(page);
                    if sections.iter().any(|section| !section.actions.is_empty()) {
                        self.panel = Some(PanelState::new(sections));
                        return iced::widget::operation::focus(PANEL_INPUT);
                    }
                    return Task::none();
                } else if let Some(task) = self.open_update_panel() {
                    return task;
                } else if let Some(task) = self.open_root_calculator_panel() {
                    return task;
                } else if let Some(task) = self.open_root_panel() {
                    return task;
                } else if let Some(item) = self.selected_item() {
                    // Only over a selected row. A panel of actions for nothing
                    // would be a panel whose every action fails.
                    let (sections, key, desktop_id) = (
                        actions_for_app(item),
                        item.key().to_owned(),
                        item.desktop_id().to_owned(),
                    );
                    self.panel = Some(PanelState::new(sections));
                    self.app_runtime = None;
                    let running = self.app_runtime_task(key, desktop_id);
                    return Task::batch([iced::widget::operation::focus(PANEL_INPUT), running]);
                }
                Task::none()
            }
            Message::PanelFilterChanged(filter) => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.set_filter(filter);
                }
                crate::scroll::reveal_panel_selection()
            }
            Message::PanelMove(direction) => {
                if let Some(panel) = self.panel.as_mut() {
                    let step = match direction {
                        Direction::Up => Step::Up,
                        Direction::Down => Step::Down,
                    };
                    panel.selected = action_panel::next_selectable(
                        &panel.rows,
                        panel.selected,
                        step,
                        self.wrap_navigation,
                    );
                }
                crate::scroll::reveal_panel_selection()
            }
            Message::PanelClicked(index) => {
                let Some(panel) = self.panel.as_mut() else {
                    return Task::none();
                };
                if !panel.rows.get(index).is_some_and(Row::selectable) {
                    return Task::none();
                }
                let Ok(selected) = isize::try_from(index) else {
                    return Task::none();
                };
                panel.selected = selected;
                self.update(Message::PanelActivate)
            }
            Message::PanelActivate => {
                if let Some(id) = self
                    .panel
                    .as_ref()
                    .and_then(PanelState::selected_action)
                    .and_then(|action| action.id.clone())
                    && let Some(task) = self
                        .shortcut_panel_action(&id)
                        .or_else(|| self.snippet_panel_action(&id))
                        .or_else(|| self.script_panel_action(&id))
                        .or_else(|| self.program_panel_action(&id))
                        .or_else(|| self.dmenu_panel_action(&id))
                        .or_else(|| self.font_panel_action(&id))
                        .or_else(|| self.store_panel_action(&id))
                        .or_else(|| self.media_panel_action(&id))
                        .or_else(|| self.theme_panel_action(&id))
                        .or_else(|| self.grants_panel_action(&id))
                        .or_else(|| self.tray_panel_action(&id))
                        .or_else(|| self.apps_panel_action(&id))
                        .or_else(|| self.emoji_panel_action(&id))
                        .or_else(|| self.clipboard_panel_action(&id))
                        .or_else(|| self.update_panel_action(&id))
                        .or_else(|| self.root_calculator_panel_action(&id))
                        .or_else(|| self.root_panel_action(&id))
                        .or_else(|| self.windows_panel_action(&id))
                        .or_else(|| self.calculator_panel_action(&id))
                        .or_else(|| self.workspaces_panel_action(&id))
                        .or_else(|| self.file_panel_action(&id))
                        .or_else(|| self.app_runtime_action(&id))
                        .or_else(|| self.compass_panel_action(&id))
                {
                    return task;
                }
                let Some(panel) = self.panel.as_ref() else {
                    return Task::none();
                };
                let Some(action) = panel.selected_action() else {
                    return Task::none();
                };
                if let (Some(handler), Page::Extension(page)) = (
                    action
                        .id
                        .as_deref()
                        .and_then(|id| id.strip_prefix(EXTENSION_ACTION)),
                    &self.page,
                ) {
                    let task =
                        self.extension_event(page.session, handler.to_owned(), page.action_args());
                    self.panel = None;
                    return Task::batch([task, focus_search()]);
                }

                let Some(item) = self.selected_item() else {
                    return Task::none();
                };
                match action.id.as_deref() {
                    Some(APP_OPEN) => {
                        self.panel = None;
                        self.update(Message::LaunchSelected)
                    }
                    Some(APP_COPY_NAME) => {
                        let name = item.name().to_owned();
                        self.panel = None;
                        self.copy_with_hud(name)
                    }
                    Some(APP_COPY_PATH) => {
                        let Some(path) = item.path() else {
                            return Task::none();
                        };
                        let path = path.to_string_lossy().into_owned();
                        self.panel = None;
                        self.copy_with_hud(path)
                    }
                    Some(id) if id.starts_with(APP_DESKTOP_ACTION) => {
                        let action_id = &id[APP_DESKTOP_ACTION.len()..];
                        if !item
                            .entry()
                            .actions()
                            .iter()
                            .any(|action| action.id() == action_id && action.exec().is_some())
                        {
                            return Task::none();
                        }
                        let task = launch_task(
                            Arc::clone(&self.launcher),
                            item.entry().clone(),
                            Some(action_id.to_owned()),
                            self.backend.clone(),
                            item.key().to_owned(),
                        );
                        self.panel = None;
                        task
                    }
                    _ => Task::none(),
                }
            }
            _ => Task::none(),
        }
    }

    /// Runs the panel action whose advertised shortcut was pressed, if one
    /// was: over the open panel, or over the panel Ctrl+B would open. A
    /// chord that only moves (the keybinding scheme's) or that no action
    /// claims changes nothing and is left to the rest of the key handler.
    pub(super) fn advertised_chord(
        &mut self,
        key: &iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Option<Task<Message>> {
        if !(modifiers.control() || modifiers.alt() || modifiers.logo())
            || chord_direction(self.keybinding, key.as_ref(), modifiers).is_some()
            || matches!(
                self.page,
                Page::Extension(_) | Page::Settings(_) | Page::Preferences(_)
            )
        {
            return None;
        }
        let name = pressed_key_name(key)?;
        let pressed = action_panel::Pressed {
            key: &name,
            ctrl: modifiers.control(),
            shift: modifiers.shift(),
            alt: modifiers.alt(),
            logo: modifiers.logo(),
        };
        let opened_here = self.panel.is_none();
        if opened_here {
            // Opened as Ctrl+B opens it, but not shown: what it would focus
            // or fetch for showing is dropped.
            let _ = self.update(Message::TogglePanel);
            if self.panel.is_none() {
                self.panel = self.files_panel_now();
            }
        }
        let Some(row) = self
            .panel
            .as_ref()
            .and_then(|panel| panel.row_answering(&pressed))
        else {
            if opened_here {
                self.panel = None;
            }
            return None;
        };
        let panel = self.panel.as_mut()?;
        panel.selected = isize::try_from(row).ok()?;
        let sections = panel.sections.clone();
        let task = self.update(Message::PanelActivate);
        // An action that left the panel as it was (one that copies and
        // stays, say) leaves no panel the person never opened; one that
        // turned it into something else (the shortcut recorder) keeps it.
        let untouched = self
            .panel
            .as_ref()
            .is_some_and(|panel| panel.recorder.is_none() && panel.sections == sections);
        if opened_here && untouched {
            self.panel = None;
            return Some(Task::batch([task, focus_search()]));
        }
        Some(task)
    }

    /// Draw the action panel.
    ///
    /// A floating card at the bottom right, the way Raycast's sits. Headers
    /// and dividers are drawn as themselves rather than as indented text, and
    /// the selection is the same filled rectangle the result list uses.
    pub(super) fn view_panel<'a>(&'a self, panel: &'a PanelState) -> Element<'a, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        if let Some(recorder) = &panel.recorder {
            return self.view_recorder(recorder);
        }
        let filter = text_input("Search…", &panel.filter)
            .id(PANEL_INPUT)
            .font(self.font())
            .on_input(Message::PanelFilterChanged)
            .padding(
                Padding::new(0.0)
                    .left(design::panel_metric("inset"))
                    .right(design::panel_metric("inset")),
            )
            .style(move |theme: &Theme, status| query_input_style(theme, status, palette))
            .size(f32::from(geometry.title_size));
        let filter = container(filter)
            .height(design::panel_metric("filter-height"))
            .align_y(Alignment::Center);
        let mut col = column![].spacing(design::panel_metric("gap"));

        for (index, panel_row) in panel.rows.iter().enumerate() {
            let element: Element<Message> = match panel_row.kind {
                RowKind::Divider => container(
                    container(Space::new().height(Length::Fixed(1.0)))
                        .width(Length::Fill)
                        .id("panel-divider-line")
                        .style(move |_: &Theme| container::Style {
                            background: Some(palette.border.to_iced().into()),
                            ..container::Style::default()
                        }),
                )
                .padding(
                    Padding::new(design::panel_metric("divider-gap"))
                        .left(design::panel_metric("inset"))
                        .right(design::panel_metric("inset")),
                )
                .into(),
                RowKind::Header => {
                    let name = panel
                        .sections
                        .get(panel_row.section)
                        .map_or("", |section| section.name.as_str());
                    container(
                        text(name.to_uppercase())
                            .font(self.font())
                            .size(f32::from(geometry.heading_size))
                            .color(palette.muted.to_iced()),
                    )
                    .height(design::panel_metric("header-height"))
                    .align_y(Alignment::Center)
                    .padding(Padding::new(0.0).left(design::panel_metric("inset")))
                    .into()
                }
                RowKind::Item => {
                    let action = panel_row.action.and_then(|position| {
                        panel.sections.get(panel_row.section)?.actions.get(position)
                    });
                    let title = action.map_or("", |action| action.title.as_str());
                    let shortcut = action.and_then(|action| action.shortcut.clone());
                    let selected = isize::try_from(index).unwrap_or(isize::MAX) == panel.selected;
                    let item = self.panel_item(title, shortcut.as_deref(), selected);
                    let item: Element<Message> = if selected {
                        container(item).id(crate::scroll::PANEL_SELECTION).into()
                    } else {
                        item
                    };
                    mouse_area(item)
                        .on_press(Message::PanelClicked(index))
                        .into()
                }
            };
            col = col.push(element);
        }

        if panel.rows.is_empty() {
            col = col.push(self.notice("No actions"));
        }

        container(
            column![
                filter,
                scrollable(col)
                    .id(crate::scroll::PANEL_RESULTS)
                    .height(Length::Shrink)
            ]
            .spacing(design::panel_metric("gap")),
        )
        .width(Length::Fixed(design::panel_metric("width")))
        .padding(design::panel_metric("padding"))
        .style(move |_: &Theme| container::Style {
            background: Some(palette.surface.to_iced().into()),
            border: Border {
                color: palette.border.to_iced(),
                width: 1.0,
                radius: 12.0.into(),
            },
            ..container::Style::default()
        })
        .into()
    }

    /// The shortcut recorder in the panel's place (`ShortcutRecorderPanel`):
    /// the item's title, then the chord or the current shortcut, the status
    /// line and, while the item has a shortcut, how to remove it.
    fn view_recorder<'a>(
        &'a self,
        recorder: &'a crate::shortcut_recorder::ShortcutRecorder,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        let small = f32::from(geometry.subtitle_size);
        let title = container(
            text(recorder.title.clone())
                .font(self.font())
                .size(f32::from(geometry.title_size))
                .color(palette.text.to_iced()),
        )
        .height(design::panel_metric("filter-height"))
        .align_y(Alignment::Center)
        .padding(Padding::new(0.0).left(design::panel_metric("inset")));
        let divider = container(Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fill)
            .style(move |_: &Theme| container::Style {
                background: Some(palette.border.to_iced().into()),
                ..container::Style::default()
            });
        let mut capture = column![].spacing(5).align_x(Alignment::Center);
        if !recorder.tokens.is_empty() {
            capture = capture.push(
                text(recorder.tokens.join(" "))
                    .font(self.font())
                    .size(f32::from(geometry.title_size))
                    .color(palette.text.to_iced()),
            );
        }
        let status = if recorder.error {
            iced::Color::from_rgb8(0xe5, 0x48, 0x4d)
        } else {
            palette.text.to_iced()
        };
        capture = capture.push(
            text(recorder.status.clone())
                .font(self.font())
                .size(small)
                .color(status),
        );
        if recorder.current.is_some() {
            capture = capture.push(
                text(crate::shortcut_recorder::REMOVE_HINT)
                    .font(self.font())
                    .size(small)
                    .color(palette.muted.to_iced()),
            );
        }
        let capture = container(capture)
            .width(Length::Fill)
            .height(Length::Fixed(130.0))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center);
        container(column![title, divider, capture])
            .width(Length::Fixed(design::panel_metric("width")))
            .padding(design::panel_metric("padding"))
            .style(move |_: &Theme| container::Style {
                background: Some(palette.surface.to_iced().into()),
                border: Border {
                    color: palette.border.to_iced(),
                    width: 1.0,
                    radius: 12.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    /// One action row, with its shortcut right-aligned.
    pub(super) fn panel_item(
        &self,
        title: &str,
        shortcut: Option<&str>,
        selected: bool,
    ) -> Element<'_, Message> {
        let geometry = self.geometry;
        let palette = self.palette();
        let colour = if selected {
            palette.selection_text
        } else {
            palette.text
        };

        let mut line = row![
            text(title.to_owned())
                .font(self.font())
                .size(f32::from(geometry.title_size))
                .color(colour.to_iced())
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        if let Some(shortcut) = shortcut {
            line = line.push(Space::new().width(Length::Fill));
            line = line.push(
                text(compass_core::key_combo::badge(shortcut))
                    .font(self.font())
                    .size(f32::from(geometry.subtitle_size))
                    .color(
                        if selected {
                            palette.selection_text
                        } else {
                            palette.muted
                        }
                        .to_iced(),
                    ),
            );
        }

        container(
            line.padding(
                Padding::new(0.0)
                    .left(design::panel_metric("inset"))
                    .right(design::panel_metric("inset")),
            ),
        )
        .width(Length::Fill)
        .height(Length::Fixed(design::panel_metric("row-height")))
        .align_y(Alignment::Center)
        .style(move |_: &Theme| {
            if selected {
                container::Style {
                    background: Some(palette.selection.to_iced().into()),
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: design::panel_metric("row-radius").into(),
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        })
        .into()
    }
}

/// The actions offered for an application.
///
/// Launching is first because it is what the return key does, and the panel's
/// first row is the one the return key runs -- so the panel opening does not
/// change what enter means.
#[must_use]
pub fn actions_for_app(item: &AppItem) -> Vec<PanelSection> {
    let mut primary = vec![Action::new("Open").with_id(APP_OPEN).with_shortcut("enter")];
    if !item.is_action() && item.entry().is_application() {
        for action in item.entry().actions() {
            if let Some(name) = action.name().filter(|name| !name.is_empty())
                && action.exec().is_some()
            {
                primary.push(
                    Action::new(name).with_id(format!("{APP_DESKTOP_ACTION}{}", action.id())),
                );
            }
        }
    }
    let mut copy = vec![Action::new("Copy name").with_id(APP_COPY_NAME)];
    if item.path().is_some() {
        copy.push(Action::new("Copy path").with_id(APP_COPY_PATH));
    }
    vec![
        PanelSection {
            name: String::new(),
            actions: primary,
        },
        PanelSection {
            name: "Copy".to_owned(),
            actions: copy,
        },
    ]
}

/// A key press as an extension shortcut: its modifiers and the key's name
/// as `jsx.d.ts` spells it. `None` for a press with no Control, Alt or Super,
/// which is typing or navigation, never a shortcut.
/// A key's name as panel shortcuts spell it: a character lower-cased, or
/// the named key (`enter`, `up`, `delete`, ...). `None` for a key no
/// shortcut names.
fn pressed_key_name(key: &iced::keyboard::Key) -> Option<String> {
    use iced::keyboard::{Key, key::Named};
    Some(match key.as_ref() {
        Key::Character(c) => c.to_lowercase(),
        Key::Named(named) => match named {
            Named::Enter => "enter",
            Named::Delete => "delete",
            Named::Backspace => "backspace",
            Named::Tab => "tab",
            Named::Space => "space",
            Named::ArrowUp => "up",
            Named::ArrowDown => "down",
            Named::ArrowLeft => "left",
            Named::ArrowRight => "right",
            Named::Home => "home",
            Named::End => "end",
            Named::PageUp => "pageup",
            Named::PageDown => "pagedown",
            _ => return None,
        }
        .to_owned(),
        Key::Unidentified => return None,
    })
}

/// How a shortcut reads in the action panel: `Ctrl+Shift+C`.
pub(super) fn shortcut_label(shortcut: &compass_extension_api::action::Shortcut) -> String {
    use compass_extension_api::action::KeyModifier;
    let mut parts: Vec<String> = shortcut
        .modifiers
        .iter()
        .map(|modifier| {
            match modifier {
                KeyModifier::Ctrl | KeyModifier::Cmd => "Ctrl",
                KeyModifier::Alt => "Alt",
                KeyModifier::Shift => "Shift",
                KeyModifier::Meta => "Super",
            }
            .to_owned()
        })
        .collect();
    let key = shortcut.key.as_str();
    parts.push(if key.chars().count() == 1 {
        key.to_uppercase()
    } else {
        let mut chars = key.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    });
    parts.join("+")
}

/// An extension view's actions on offer, as panel sections. Submenus are
/// flattened into their section for now.
fn extension_panel_sections(page: &crate::extension_page::ExtensionPage) -> Vec<PanelSection> {
    use compass_extension_api::action::ActionItem;
    fn actions(items: &[ActionItem], out: &mut Vec<action_panel::Action>) {
        for item in items {
            match item {
                ActionItem::Action(action) => {
                    let row = action_panel::Action::new(action.title.clone())
                        .with_id(format!("{EXTENSION_ACTION}{}", action.handler.0));
                    out.push(match &action.shortcut {
                        Some(shortcut) => row.with_shortcut(shortcut_label(shortcut)),
                        None => row,
                    });
                }
                ActionItem::Submenu(submenu) => actions(&submenu.items, out),
            }
        }
    }
    page.actions()
        .map(|panel| {
            panel
                .sections
                .iter()
                .map(|section| {
                    let mut list = Vec::new();
                    actions(&section.items, &mut list);
                    PanelSection {
                        name: section.title.clone().unwrap_or_default(),
                        actions: list,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RootRow;
    use crate::app::test_support::{
        RecordingLaunchTarget, TestBackend, app, app_with_rows, chord, pressed,
    };
    use compass_core::AppIndex;
    use iced::window;

    fn clipboard_writes(task: Task<Message>) -> Vec<String> {
        use iced::futures::{StreamExt, executor::block_on};
        use iced_winit::runtime::{Action, clipboard, task};

        let Some(stream) = task::into_stream(task) else {
            return Vec::new();
        };
        block_on(
            stream
                .filter_map(|action| async move {
                    match action {
                        Action::Clipboard(clipboard::Action::Write { contents, .. }) => {
                            Some(contents)
                        }
                        // A copy that hides the launcher exits one with no engine.
                        Action::Widget(_) | Action::Exit => None,
                        other => panic!("copy issued an unexpected action: {other:?}"),
                    }
                })
                .collect(),
        )
    }

    fn recorded_launch(action: bool) -> Vec<Option<String>> {
        use iced::futures::{StreamExt, executor::block_on};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("browser.desktop"), "[Desktop Entry]\nType=Application\nName=Browser\nExec=parent\nActions=private;\n[Desktop Action private]\nName=Private Window\nExec=private-app\n").unwrap();
        let index = AppIndex::builder().dir(dir.path()).build();
        let selected = index
            .items()
            .iter()
            .position(|item| item.is_action() == action)
            .unwrap();
        let launcher = Arc::new(RecordingLaunchTarget::default());
        let mut app = LauncherApp::with_index(index).with_launcher(launcher.clone());
        app.results = vec![RootRow::App(selected)];
        let task = app.update(Message::LaunchSelected);
        let stream = iced_winit::runtime::task::into_stream(task).expect("launch task");
        let _outputs = block_on(stream.collect::<Vec<_>>());
        launcher.0.lock().unwrap().clone()
    }

    #[test]
    fn selecting_a_desktop_action_dispatches_its_id_not_the_parent() {
        assert_eq!(recorded_launch(true), [Some("private".to_owned())]);
    }

    #[test]
    fn selecting_an_application_still_dispatches_the_parent() {
        assert_eq!(recorded_launch(false), [None]);
    }

    #[test]
    fn a_desktop_link_is_searchable_without_application_actions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manual.desktop"), "[Desktop Entry]\nType=Link\nName=Manual\nURL=file:///manual.pdf\nActions=invalid;\n[Desktop Action invalid]\nName=Invalid\nExec=wrong\n").unwrap();
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        let _ = app.update(Message::QueryChanged("Manual".to_owned()));
        assert_eq!(app.results.len(), 1);
        let item = app.selected_item().unwrap();
        assert_eq!(item.entry().url(), Some("file:///manual.pdf"));
        let sections = actions_for_app(item);
        assert_eq!(sections[0].actions.len(), 1);
        assert_eq!(sections[0].actions[0].id.as_deref(), Some(APP_OPEN));
    }

    #[test]
    fn a_root_application_exposes_and_dispatches_its_desktop_action_in_the_panel() {
        use iced::futures::{StreamExt, executor::block_on};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("browser.desktop"), "[Desktop Entry]\nType=Application\nName=Webbrowser\nExec=parent\nActions=private;\n[Desktop Action private]\nName=Private Window\nExec=private-app\n").unwrap();
        let index = AppIndex::builder().dir(dir.path()).build();
        let launcher = Arc::new(RecordingLaunchTarget::default());
        let mut app = LauncherApp::with_index(index).with_launcher(launcher.clone());
        let _ = app.update(Message::QueryChanged("Webbrowser".to_owned()));
        // Set Default Browser's subtitle matches too; only the application
        // rows are in question here.
        assert_eq!(
            app.results
                .iter()
                .filter(|row| matches!(row, RootRow::App(_)))
                .count(),
            1,
            "actions are not duplicate root rows"
        );
        assert!(matches!(app.results[0], RootRow::App(_)));
        let backend = Arc::new(TestBackend::default());
        app.backend = Some(backend.clone());
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("Private".to_owned()));
        assert_eq!(
            app.panel
                .as_ref()
                .unwrap()
                .selected_action()
                .unwrap()
                .id
                .as_deref(),
            Some("app.desktop:private")
        );
        let task = app.update(Message::PanelActivate);
        assert!(app.panel.is_none());
        let stream = iced_winit::runtime::task::into_stream(task).unwrap();
        let _ = block_on(stream.collect::<Vec<_>>());
        assert_eq!(*launcher.0.lock().unwrap(), [Some("private".to_owned())]);
        assert_eq!(*backend.recorded.lock().unwrap(), ["browser.desktop"]);
    }

    #[test]
    fn filtered_copy_actions_write_the_selected_apps_name_and_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let expected_name = app.selected_item().expect("selected").name().to_owned();
        let expected_path = app
            .selected_item()
            .expect("selected")
            .path()
            .expect("desktop path")
            .to_string_lossy()
            .into_owned();
        for (filter, expected) in [("name", expected_name), ("path", expected_path)] {
            let _ = app.update(Message::TogglePanel);
            let _ = app.update(Message::PanelFilterChanged(filter.to_owned()));
            let task = app.update(pressed(iced::keyboard::key::Named::Enter));
            assert_eq!(clipboard_writes(task), vec![expected]);
            assert!(app.panel.is_none());
            assert_eq!(app.query, "fi");
        }
    }

    #[test]
    fn dispatch_uses_identity_even_when_the_label_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let expected = app.selected_item().expect("selected").name().to_owned();
        let _ = app.update(Message::TogglePanel);
        let panel = app.panel.as_mut().expect("panel");
        panel.sections[1].actions[0].title = "Copier le nom".to_owned();
        panel.set_filter("Copier le nom".to_owned());
        let task = app.update(Message::PanelActivate);
        assert_eq!(clipboard_writes(task), vec![expected]);
    }

    #[test]
    fn an_empty_filter_result_cannot_launch_or_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("zzzznotanaction".to_owned()));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(iced_winit::runtime::task::into_stream(task).is_none());
        assert!(app.panel.is_some());
    }

    #[test]
    fn changing_the_root_query_invalidates_its_panel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::QueryChanged("terminal".to_owned()));
        assert!(app.panel.is_none());
        assert_eq!(app.selected_item().expect("selected").name(), "Terminal");
        assert!(clipboard_writes(app.update(Message::PanelActivate)).is_empty());
    }

    #[test]
    fn clicking_an_action_copies_but_clicking_a_heading_does_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let expected = app.selected_item().expect("selected").name().to_owned();
        let _ = app.update(Message::TogglePanel);
        assert!(clipboard_writes(app.update(Message::PanelClicked(2))).is_empty());
        assert!(app.panel.is_some());
        let messages = {
            let mut ui = iced_test::simulator(app.view());
            ui.click("Copy name").expect("click the action");
            ui.into_messages().collect::<Vec<_>>()
        };
        assert_eq!(messages.len(), 1);
        let writes: Vec<_> = messages
            .into_iter()
            .flat_map(|message| clipboard_writes(app.update(message)))
            .collect();
        assert_eq!(writes, vec![expected]);
    }

    #[test]
    fn typing_in_the_panel_emits_only_panel_edits_and_no_submit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        let mut ui = iced_test::simulator(app.view());
        ui.click(iced_test::selector::id(PANEL_INPUT))
            .expect("panel input");
        ui.typewrite("n");
        ui.tap_key(iced::keyboard::key::Named::Enter);
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            matches!(messages.as_slice(), [Message::PanelFilterChanged(filter)] if filter == "n")
        );
    }

    #[test]
    fn the_action_panel_is_not_bound_to_the_vim_chord() {
        // Ctrl+K is "move up" in the default Linux scheme. The C++ binds the
        // panel to Ctrl+K on macOS only and Ctrl+B everywhere else, and this
        // is why -- taking Ctrl+K here would break navigation for every user
        // who has configured nothing.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 1, "precondition: Ctrl+J moved down");

        let _ = app.update(chord("k", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_none(), "Ctrl+K must not open the panel");
        assert_eq!(app.selected, 0, "Ctrl+K still moves up");
    }

    #[test]
    fn ctrl_b_opens_and_closes_the_panel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_some(), "Ctrl+B opened it");

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_none(), "and Ctrl+B closed it again");
    }

    #[test]
    fn the_panel_does_not_open_over_nothing() {
        // A panel of actions for no selected row is a panel whose every action
        // fails.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("zzzzzzzz".to_owned()));
        assert!(
            app.selected_item().is_none(),
            "precondition: nothing selected"
        );

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        assert!(app.panel.is_none());
    }

    #[test]
    fn the_panel_takes_the_arrow_keys_while_it_is_open() {
        // Otherwise the list underneath moves out from under a panel whose
        // actions are for the row that was selected when it opened.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let before = app.selected;

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(app.selected, before, "the list did not move");
        // Row 1 is the divider and row 2 the heading, so the next selectable
        // row is 3. Expecting 1 would have been expecting the selection to
        // land on a divider.
        assert_eq!(
            app.panel.as_ref().map(|panel| panel.selected),
            Some(3),
            "the panel did"
        );
    }

    #[test]
    fn typing_the_panels_letter_without_ctrl_does_not_open_it() {
        // The search field takes ordinary characters, and a launcher whose
        // search box opens a panel when someone types `b` is unusable.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());

        let _ = app.update(chord("b", iced::keyboard::Modifiers::default()));
        assert!(app.panel.is_none());
    }

    #[test]
    fn escape_closes_the_panel_rather_than_the_launcher() {
        // A panel opened by mistake should cost one key, not the whole window.
        // The window has to be open for this to mean anything -- with no
        // window, "the window did not close" is true however Escape is routed.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::Opened(window::Id::unique()));
        assert!(app.window.is_some(), "precondition: a window is open");

        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(app.panel.is_none(), "the panel closed");
        assert!(app.window.is_some(), "and the window did not");

        // HALF OF THIS IS NOT CONTROL-BACKED, and it is worth saying which.
        //
        // "The panel closed" fires under a mutation. "The window did not"
        // does not: `conceal` closes the window through a Task and clears
        // `self.window` only when `Message::Closed` comes back, and with no
        // engine link `on_dismiss` returns `Exit`, so a mutation that made
        // Escape dismiss as well as close the panel changes nothing this test
        // can see. Proving it would need an app built around a live
        // `EngineLink`, which is a harness this crate does not have yet.
        //
        // Recorded rather than left looking covered.
    }

    #[test]
    fn the_panel_stops_intercepting_once_it_is_closed() {
        // The follow-on from the test above: with the panel gone, Escape is
        // the launcher's again. What it *does* then is `conceal`, which closes
        // the window through a Task and so is not observable from here — so
        // this asserts the routing rather than the closing, and the test above
        // asserts that the routing was different while the panel was open.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(Message::Opened(window::Id::unique()));
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(app.panel.is_none(), "the panel closed");

        // Arrows reach the list again, which the panel was taking.
        let before = app.selected;
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_ne!(app.selected, before, "the list moved again");
    }

    #[test]
    fn the_vim_chords_move_the_panel_while_it_is_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let before = app.selected;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, before, "the list did not move");
        assert_eq!(app.panel.as_ref().map(|panel| panel.selected), Some(3));
    }

    #[test]
    fn running_an_action_closes_the_panel() {
        // An action that ran and one that is not wired up both leave the panel
        // with nothing more to say, and leaving it open would look like the key
        // had not registered.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        let _ = app.update(chord("b", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));

        let _ = app.update(pressed(iced::keyboard::key::Named::Enter));
        assert!(app.panel.is_none());
    }
}
