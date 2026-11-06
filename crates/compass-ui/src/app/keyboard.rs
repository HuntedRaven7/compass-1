//! The launcher's key handling: who gets a key press, in order.
//!
//! A question on screen takes Enter and Escape first, then the shortcut
//! recorders, the form a view is showing, the action panel while it is
//! open, and each builtin page's own keys; whatever is left moves the root
//! selection, launches, or is a quick-launch chord. Text typed into a field
//! is not handled here: it arrives through the field's `on_input`.

use super::clipboard::ClipboardChange;
use super::{
    Direction, LauncherApp, Message, Page, Task, chord_direction, focus_search, form_focus,
    next_selection,
};

impl LauncherApp {
    /// A key event from the keyboard subscription.
    pub(super) fn keyboard_message(&mut self, message: Message) -> Task<Message> {
        match message {
            // The settings' shortcut recorder takes every key too.
            Message::Keyboard(event) if matches!(&self.page, Page::Settings(page) if page.recorder.is_some()) => {
                self.settings_recorder_event(&event)
            }
            // The shortcut recorder takes every key, releases included.
            Message::Keyboard(event)
                if self
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.recorder.is_some()) =>
            {
                self.recorder_event(&event)
            }
            Message::Keyboard(iced::keyboard::Event::KeyPressed {
                ref key, modifiers, ..
            }) => {
                use iced::keyboard::{Key, key::Named};

                // A command's view takes every key the root list would, and
                // Escape goes back to the root rather than hiding the window:
                // one key undoes opening the wrong command.
                let panel_key = self.panel.is_some()
                    || (modifiers.control() && key.as_ref() == Key::Character("b"));
                if self.confirm.is_some() {
                    return match key.as_ref() {
                        Key::Named(Named::Enter) => self.run_confirmed(),
                        Key::Named(Named::Escape) => {
                            self.confirm = None;
                            focus_search()
                        }
                        _ => Task::none(),
                    };
                }
                if let Some(power) = self.power_confirm {
                    return match key.as_ref() {
                        Key::Named(Named::Enter) => self.run_power_command(power),
                        // Back to the search, as the generic confirmation
                        // above: the field lost the focus to the question.
                        Key::Named(Named::Escape) => {
                            self.power_confirm = None;
                            focus_search()
                        }
                        _ => Task::none(),
                    };
                }
                if matches!(self.page, Page::Onboarding(_)) {
                    return self.onboarding_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::StoreIntro(_)) {
                    return match key.as_ref() {
                        Key::Named(Named::Enter) => self.continue_to_store(),
                        Key::Named(Named::Escape) => self.update(Message::Back),
                        _ => crate::scroll::reading_key(key.as_ref()).unwrap_or_else(Task::none),
                    };
                }
                if let Page::Preferences(page) = &self.page {
                    return match key.as_ref() {
                        // A text area's Enter is a newline; the form submits
                        // with Ctrl+Enter.
                        Key::Named(Named::Enter)
                            if page.has_text_area() && !modifiers.control() =>
                        {
                            Task::none()
                        }
                        Key::Named(Named::Enter) => self.update(Message::PreferencesSubmit),
                        Key::Named(Named::Escape) => self.update(Message::Back),
                        Key::Named(Named::Tab) => {
                            let Page::Preferences(page) = &mut self.page else {
                                return Task::none();
                            };
                            page.step_focus(!modifiers.shift());
                            form_focus(page)
                        }
                        // A checkbox or a dropdown with the keyboard on it:
                        // Space ticks or steps, the arrows step.
                        _ if page.focus.is_some_and(|index| !page.is_text(index)) => {
                            use crate::preferences_page::FieldKey;
                            let field_key = match key.as_ref() {
                                Key::Named(Named::Space) => FieldKey::Toggle,
                                Key::Named(Named::ArrowDown) => FieldKey::Next,
                                Key::Named(Named::ArrowUp) => FieldKey::Previous,
                                _ => return Task::none(),
                            };
                            if let Page::Preferences(page) = &mut self.page {
                                page.field_key(field_key);
                            }
                            Task::none()
                        }
                        _ => Task::none(),
                    };
                }
                // An alert takes Enter and Escape and nothing else: the
                // extension is waiting on the answer.
                if let Page::Extension(page) = &mut self.page
                    && page.alert.is_some()
                {
                    let remembers = page
                        .alert
                        .as_ref()
                        .is_some_and(|alert| alert.remember_text.is_some());
                    let answer = match key.as_ref() {
                        Key::Named(Named::Enter) if remembers && modifiers.control() => None,
                        Key::Named(Named::Enter) => Some(true),
                        Key::Named(Named::Escape) => Some(false),
                        _ => return Task::none(),
                    };
                    page.alert = None;
                    let (session, backend) = (page.session, self.backend.clone());
                    let Some(backend) = backend else {
                        return Task::none();
                    };
                    return Task::perform(
                        async move {
                            match answer {
                                Some(confirmed) => {
                                    backend.extension_alert_answer(session, confirmed).await
                                }
                                None => backend.extension_alert_remember(session).await,
                            }
                        },
                        Message::ExtensionEventSent,
                    );
                }
                // A panel action's shortcut works whether or not the panel is
                // open: the panel says what the chord does, and the chord
                // should not need the panel to do it.
                if !panel_key && let Some(task) = self.advertised_chord(key, modifiers) {
                    return task;
                }
                if !panel_key
                    && let Page::Extension(page) = &self.page
                    && let Some(handler) = extension_chord(key, modifiers)
                        .and_then(|(mods, name)| page.action_for(&mods, &name))
                        .cloned()
                {
                    return self.extension_event(page.session, handler.0, page.action_args());
                }
                if !panel_key && let Page::Extension(page) = &mut self.page {
                    if page.form().is_some() && key.as_ref() == Key::Named(Named::Tab) {
                        return if modifiers.shift() {
                            iced::widget::operation::focus_previous()
                        } else {
                            iced::widget::operation::focus_next()
                        };
                    }
                    // A Markdown detail is read, not picked from: the
                    // arrows and the page keys scroll it.
                    if matches!(page.view, Some(compass_extension_api::View::Detail(_)))
                        && let Some(scroll) = crate::scroll::reading_key(key.as_ref())
                    {
                        return scroll;
                    }
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) if page.depth > 1 => {
                            let session = page.session;
                            return self.extension_pop(session);
                        }
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        // A text area's Enter is a newline; its form submits
                        // with Ctrl+Enter.
                        Key::Named(Named::Enter)
                            if page.has_text_area() && !modifiers.control() =>
                        {
                            return Task::none();
                        }
                        Key::Named(Named::Enter) => return self.activate_extension_action(),
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    // A grid moves by cell and by row, as `SectionGridModel`.
                    if page.grid_columns.is_some() {
                        use crate::fonts_page::GridMove;
                        let step = match (key.as_ref(), direction) {
                            (Key::Named(Named::ArrowLeft), _) => Some(GridMove::Left),
                            (Key::Named(Named::ArrowRight), _) => Some(GridMove::Right),
                            (_, Some(Direction::Up)) => Some(GridMove::Up),
                            (_, Some(Direction::Down)) => Some(GridMove::Down),
                            _ => None,
                        };
                        if let Some(step) = step {
                            page.selected = page.grid_step(step, self.wrap_navigation);
                            return crate::scroll::reveal_root_selection();
                        }
                        return Task::none();
                    }
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.shown.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        return crate::scroll::reveal_root_selection();
                    }
                    return Task::none();
                }
                if !panel_key && matches!(self.page, Page::Emoji(_)) {
                    return self.emoji_page_key(key, modifiers);
                }
                if !panel_key && let Some(task) = self.shortcut_chord(key, modifiers) {
                    return task;
                }
                if !panel_key && matches!(self.page, Page::Shortcuts(_)) {
                    return self.shortcuts_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Snippets(_)) {
                    return self.snippets_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::ScriptOutput(_)) {
                    return self.script_output_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Programs(_)) {
                    return self.programs_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Dmenu(_)) {
                    return self.dmenu_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Grants(_)) {
                    return self.grants_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Tray(_)) {
                    return self.tray_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Apps(_)) {
                    return self.apps_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Calculator(_)) {
                    return self.calculator_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Fallbacks(_)) {
                    return self.fallbacks_page_key(key, modifiers);
                }
                if !panel_key
                    && matches!(
                        self.page,
                        Page::Extensions(_) | Page::Icons(_) | Page::Storage(_) | Page::Tokens(_)
                    )
                {
                    return self.compass_view_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Workspaces(_)) {
                    return self.workspaces_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::OpenWith(_)) {
                    return self.open_with_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::NowPlaying(_)) {
                    return self.now_playing_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Themes(_)) {
                    return self.themes_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Settings(_)) {
                    return self.settings_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Created(_)) {
                    return self.created_page_key(key);
                }
                if !panel_key && matches!(self.page, Page::Fonts(_)) {
                    return self.fonts_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::FontPreview(_)) {
                    return self.font_preview_key(key);
                }
                if !panel_key && matches!(self.page, Page::Store(_)) {
                    return self.store_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::StoreDetail(_)) {
                    return self.store_detail_key(key);
                }
                if !panel_key && let Page::Files(page) = &mut self.page {
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        // Ctrl+Enter is `Keyboard::Shortcut::submit()`, which
                        // the C++ binds to "Show in file browser".
                        Key::Named(Named::Enter) => {
                            return self.open_selected_file(modifiers.control());
                        }
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.rows.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        page.refresh_preview();
                        return crate::scroll::reveal_root_selection();
                    }
                    return Task::none();
                }
                if !panel_key && let Page::Windows(page) = &mut self.page {
                    if modifiers.control() && key.as_ref() == Key::Character("q") {
                        return self.close_selected_window();
                    }
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        Key::Named(Named::Enter) => return self.activate_selected_window(),
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
                    return Task::none();
                }
                if !panel_key && matches!(self.page, Page::Clipboard(_)) {
                    // The C++ defaults: `action.pin` is Ctrl+Shift+P and
                    // `action.remove` is Ctrl+X.
                    if let Key::Character(c) = key.as_ref()
                        && modifiers.control()
                        && !modifiers.alt()
                        && !modifiers.logo()
                    {
                        if modifiers.shift() && c.eq_ignore_ascii_case("p") {
                            return self
                                .change_selected_clipboard_entry(ClipboardChange::TogglePin);
                        }
                        if !modifiers.shift() && c.eq_ignore_ascii_case("x") {
                            return self.change_selected_clipboard_entry(ClipboardChange::Remove);
                        }
                    }
                    if let Some(task) = self.clipboard_chord(key, modifiers) {
                        return task;
                    }
                    let Page::Clipboard(page) = &mut self.page else {
                        return Task::none();
                    };
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        Key::Named(Named::Enter) => return self.paste_selected_clipboard_entry(),
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.rows.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        return Task::batch([
                            crate::scroll::reveal_root_selection(),
                            self.clipboard_detail_task(),
                        ]);
                    }
                    return Task::none();
                }

                // Ctrl+B opens and closes the panel, over the list either way.
                //
                // Ctrl+B and not Ctrl+K, which is what the C++ binds on macOS
                // only: on Linux Ctrl+K is the vim chord for "move up", and
                // taking it here would have broken navigation for every user
                // of the default scheme. `keybind-manager.cpp` has the same
                // `#ifdef`, for the same reason.
                if modifiers.control() && key.as_ref() == Key::Character("b") {
                    return self.update(Message::TogglePanel);
                }

                // Ctrl+, opens the settings (`Keybind::OpenSettings`).
                if modifiers.control()
                    && self.panel.is_none()
                    && matches!(self.page, Page::Root)
                    && key.as_ref() == Key::Character(",")
                {
                    return self.open_settings(None);
                }

                // While the panel is open it takes the keys the list would
                // otherwise take. Escape closes the panel rather than the
                // launcher, because a panel opened by mistake should cost one
                // key and not the whole window.
                if self.panel.is_some() {
                    if let Some(task) = self.advertised_chord(key, modifiers) {
                        return task;
                    }
                    match key.as_ref() {
                        Key::Named(Named::ArrowDown) => {
                            return self.update(Message::PanelMove(Direction::Down));
                        }
                        Key::Named(Named::ArrowUp) => {
                            return self.update(Message::PanelMove(Direction::Up));
                        }
                        Key::Named(Named::Enter) => return self.update(Message::PanelActivate),
                        Key::Named(Named::Escape) => {
                            self.panel = None;
                            return focus_search();
                        }
                        _ => {}
                    }
                    return match chord_direction(self.keybinding, key.as_ref(), modifiers) {
                        Some(direction) => self.update(Message::PanelMove(direction)),
                        None => Task::none(),
                    };
                }

                // Ctrl+1..9 launches the Nth row outright (#87). Above the
                // arrows because it is a complete action rather than a
                // movement, and below the panel branch because while the panel
                // is open the list is not what a keystroke is aimed at.
                //
                // OUT OF RANGE DOES NOTHING, deliberately. `max_results` can be
                // below nine, and a query can match two things; Ctrl+7 then
                // refers to no row. Launching the last one instead would be a
                // guess at what the user meant, and the thing it launches is
                // whatever happens to be at the bottom of an unrelated list.
                //
                // An out-of-range position falls through rather than returning
                // early. The early return was there to stop the chord reaching
                // the navigation matcher, and a control could not be made to
                // fire on it: no scheme maps a digit to a direction, so the two
                // paths are indistinguishable. Rather than keep a branch no
                // test can tell apart from its absence, it is gone -- and if a
                // scheme ever does bind digits, the test that catches it is the
                // one that notices the selection moving.
                if self.quick_launch
                    && let Some(position) = quick_launch_position(key.as_ref(), modifiers)
                    && position < self.results.len()
                {
                    self.selected = position;
                    return self.update(Message::LaunchSelected);
                }

                // `ctrl+q` closes a window from the switcher (`CLOSE_WINDOW_SHORTCUT`).
                if modifiers.control()
                    && key.as_ref() == Key::Character("q")
                    && let Some(item) = self.selected_item()
                    && let Some(window_id) =
                        compass_core::window_switcher::window_launch_target_for_app(item.key())
                {
                    return self.update(Message::CloseWindow(window_id));
                }

                // Up at the top of the list reaches back through past searches.
                let up = match key.as_ref() {
                    Key::Named(Named::ArrowUp) => Some(Direction::Up),
                    _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                };
                if let Some(task) = self.history_up(up) {
                    return task;
                }
                let down = match key.as_ref() {
                    Key::Named(Named::ArrowDown) => Some(Direction::Down),
                    _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                };
                if let Some(task) = self.history_down(down) {
                    return task;
                }

                match key.as_ref() {
                    Key::Named(Named::ArrowDown) => {
                        return self.update(Message::MoveSelection(Direction::Down));
                    }
                    Key::Named(Named::ArrowUp) => {
                        return self.update(Message::MoveSelection(Direction::Up));
                    }
                    // Text in the field is cleared first, and only an empty
                    // field hides: `NavigationController::goBack` at the
                    // root search, as Raycast does. The field gives up its
                    // focus on Escape, so it is handed back for the next key.
                    Key::Named(Named::Escape) if !self.query.is_empty() => {
                        let cleared = self.update(Message::QueryChanged(String::new()));
                        return Task::batch([cleared, focus_search()]);
                    }
                    Key::Named(Named::Escape) => return self.update(Message::Dismiss),
                    // A message (a script's line, an error) stands in place
                    // of the results: Enter puts them back rather than
                    // launching a row nobody can see.
                    Key::Named(Named::Enter)
                        if self.error.is_some() && matches!(self.page, Page::Root) =>
                    {
                        self.error = None;
                        return focus_search();
                    }
                    Key::Named(Named::Enter) => return self.update(Message::LaunchSelected),
                    _ => {}
                }

                // A chord, if this one is. The scheme decides; `compass-core`
                // owns which chords each scheme has, so this front end does
                // not have a second opinion about it.
                match chord_direction(self.keybinding, key.as_ref(), modifiers) {
                    Some(direction) => self.update(Message::MoveSelection(direction)),
                    None => Task::none(),
                }
            }
            _ => Task::none(),
        }
    }
}

/// Which result `Ctrl+1`..`Ctrl+9` asks for, as a **position in the visible
/// list** counting from zero.
///
/// # The two things this is easy to get wrong
///
/// It returns a POSITION, not an item index. `LauncherApp::results` holds
/// indices into the application index, and the selection is a position within
/// `results` -- so `Ctrl+3` means the third row on screen, not item 3. Mixing
/// those up launches something plausible and wrong, which is the worst kind of
/// bug to have here because nothing looks broken.
///
/// And there is no zero. `Ctrl+1` is the first row, so the digit is one-based
/// and `Ctrl+0` is not a quick-launch chord at all.
///
/// Bounds are the caller's: this says which row was asked for, not whether it
/// exists.
#[must_use]
pub fn quick_launch_position(
    key: iced::keyboard::Key<&str>,
    modifiers: iced::keyboard::Modifiers,
) -> Option<usize> {
    use iced::keyboard::Key;

    // Control alone. A chord carrying Alt or Logo as well is somebody else's,
    // and claiming it would take a binding the desktop may already have.
    // Shift is excluded too: Ctrl+Shift+1 is a different chord, and on many
    // layouts it is not a digit at all.
    if !modifiers.control() || modifiers.alt() || modifiers.logo() || modifiers.shift() {
        return None;
    }

    let Key::Character(text) = key else {
        return None;
    };

    // `"1".."9"`, and nothing else: not "10", not a digit with anything
    // attached. `chars().next()` on a longer string would accept "1x".
    let mut chars = text.chars();
    let digit = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let value = digit.to_digit(10)?;
    if value == 0 {
        return None;
    }
    Some(value as usize - 1)
}

fn extension_chord(
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<(Vec<compass_extension_api::action::KeyModifier>, String)> {
    use compass_extension_api::action::KeyModifier;
    use iced::keyboard::{Key, key::Named};
    if !(modifiers.control() || modifiers.alt() || modifiers.logo()) {
        return None;
    }
    let name = match key {
        Key::Character(c) => c.to_lowercase(),
        Key::Named(named) => match named {
            Named::Enter => "return",
            Named::Delete => "delete",
            Named::Backspace => "backspace",
            Named::Tab => "tab",
            Named::Space => "space",
            Named::ArrowUp => "arrowUp",
            Named::ArrowDown => "arrowDown",
            Named::ArrowLeft => "arrowLeft",
            Named::ArrowRight => "arrowRight",
            Named::Home => "home",
            Named::End => "end",
            Named::PageUp => "pageUp",
            Named::PageDown => "pageDown",
            _ => return None,
        }
        .to_owned(),
        Key::Unidentified => return None,
    };
    let mut mods = Vec::new();
    if modifiers.control() {
        mods.push(KeyModifier::Ctrl);
    }
    if modifiers.alt() {
        mods.push(KeyModifier::Alt);
    }
    if modifiers.shift() {
        mods.push(KeyModifier::Shift);
    }
    if modifiers.logo() {
        mods.push(KeyModifier::Meta);
    }
    Some((mods, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{app, app_with_rows, chord, index, pressed, with_resident_hud};

    #[test]
    fn escape_clears_the_search_text_before_it_hides() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let _ = app.update(Message::QueryChanged("fire".into()));
        app.window_focused = true;
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert_eq!(app.query, "", "the first Escape clears the text");
        assert!(app.window_focused, "and does not hide");
        let _ = app.update(pressed(iced::keyboard::key::Named::Escape));
        assert!(!app.window_focused, "the second hides");
    }

    /// Builds a `KeyPressed` for a named key, as the subscription delivers it.
    /// A `Ctrl`+`digit` press. `chord` below builds it; this names the intent.
    pub(super) fn ctrl_digit(digit: &str) -> Message {
        chord(digit, iced::keyboard::Modifiers::CTRL)
    }

    #[test]
    fn the_vim_chords_move_the_selection_because_they_are_the_linux_default() {
        // `KeyBindingService::getMode` falls back to vim off macOS, so a user
        // who has configured nothing still expects Ctrl+J and Ctrl+K to work.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;
        assert_eq!(app.selected, 0);

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 1, "Ctrl+J moved down");

        let _ = app.update(chord("k", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 0, "and Ctrl+K moved back");
    }

    #[test]
    fn a_bare_letter_is_not_a_chord() {
        // The negative that matters most: a handler that ignored modifiers
        // would make the search field unusable, because every `j` typed would
        // also move the selection.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::empty()));
        assert_eq!(app.selected, 0, "a bare `j` moved the selection");
    }

    #[test]
    fn a_chord_from_another_scheme_does_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Emacs;

        let _ = app.update(chord("j", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 0, "Ctrl+J is vim's, not emacs'");

        let _ = app.update(chord("n", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 1, "Ctrl+N is emacs' down");
    }

    #[test]
    fn the_horizontal_chords_do_nothing_in_a_one_column_list() {
        // Recognised by `compass-core`, dropped here on purpose rather than
        // bound to something they do not mean.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_rows(dir.path());
        app.keybinding = compass_core::keybinding::Scheme::Vim;

        let _ = app.update(chord("l", iced::keyboard::Modifiers::CTRL));
        let _ = app.update(chord("h", iced::keyboard::Modifiers::CTRL));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn the_arrows_move_the_selection_through_the_keyboard_subscription() {
        // Goes through `Message::Keyboard` rather than `Message::MoveSelection`
        // on purpose: the direct path is covered above, and what this covers is
        // the match arm that routes a key to it. That arm is what a change to
        // the subscription puts at risk.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        assert!(app.results.len() > 1, "need several rows to move between");
        assert_eq!(app.selected, 0);

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowDown));
        assert_eq!(app.selected, 1, "ArrowDown moved down");

        let _ = app.update(pressed(iced::keyboard::key::Named::ArrowUp));
        assert_eq!(app.selected, 0, "and ArrowUp moved back");
    }

    #[test]
    fn a_printable_key_from_the_subscription_does_not_also_type() {
        // THE CONTROL FOR HANDLING A KEY TWICE.
        //
        // `subscription` reads every keyboard event, including ones the search
        // field consumed, so that Escape survives the field being focused. The
        // risk that buys is double handling: if `update` ever grew an arm that
        // appended printable keys to the query, every character would arrive
        // twice -- once here and once through the field's `on_input` -- and
        // "fi" would be typed as "ffii".
        //
        // Nothing here can observe `on_input`, which Iced calls; what it can
        // observe is that this path contributes nothing on its own.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("fi".to_owned()));
        let before = app.query.clone();

        let _ = app.update(Message::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character("x".into()),
            modified_key: iced::keyboard::Key::Character("x".into()),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::default(),
            text: Some("x".into()),
            repeat: false,
        }));

        assert_eq!(app.query, before, "the query is the field's to change");
    }
}

#[cfg(test)]
mod quick_launch_tests {
    use super::*;
    use crate::app::RootRow;
    use crate::app::flags::AppFlags;
    use iced::keyboard::{Key, Modifiers};

    #[test]
    fn ctrl_one_through_nine_are_positions_counting_from_zero() {
        for (digit, want) in [("1", 0), ("2", 1), ("5", 4), ("9", 8)] {
            assert_eq!(
                quick_launch_position(Key::Character(digit), Modifiers::CTRL),
                Some(want),
                "Ctrl+{digit}"
            );
        }
    }

    #[test]
    fn there_is_no_ctrl_zero() {
        // The digits are one-based because the rows are. A zero would have to
        // mean either "the first" or "the tenth" and neither is what anyone
        // presses it for.
        assert_eq!(
            quick_launch_position(Key::Character("0"), Modifiers::CTRL),
            None
        );
    }

    #[test]
    fn a_digit_without_control_is_somebody_elses() {
        // Typing "3" into the search field must reach the field.
        assert_eq!(
            quick_launch_position(Key::Character("3"), Modifiers::default()),
            None
        );
    }

    #[test]
    fn control_plus_another_modifier_is_a_different_chord() {
        // Not ours to claim: the desktop may already bind these, and on many
        // layouts Ctrl+Shift+1 is not a digit at all.
        for extra in [Modifiers::SHIFT, Modifiers::ALT, Modifiers::LOGO] {
            assert_eq!(
                quick_launch_position(Key::Character("1"), Modifiers::CTRL | extra),
                None,
                "{extra:?}"
            );
        }
    }

    #[test]
    fn a_non_digit_character_is_not_a_quick_launch() {
        for text in ["b", "-", "", "1x", "12"] {
            assert_eq!(
                quick_launch_position(Key::Character(text), Modifiers::CTRL),
                None,
                "{text:?}"
            );
        }
    }

    #[test]
    fn ctrl_n_selects_and_launches_the_nth_visible_row() {
        // THE BUG THIS IS HERE FOR: `results` holds indices into the
        // application index, and the selection is a position within `results`.
        // Ctrl+3 means the third ROW, not item 3. Confusing the two launches
        // something plausible and wrong, which is the worst kind to have here
        // because nothing looks broken.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = crate::app::test_support::app_with_rows(dir.path());

        let rows: Vec<String> = app
            .results
            .iter()
            .filter_map(|row| match row {
                RootRow::App(i) => app.app_index.items().get(*i),
                RootRow::Command(_)
                | RootRow::Extension(_)
                | RootRow::Shortcut(_)
                | RootRow::Script(_)
                | RootRow::RhaiScript(_)
                | RootRow::Fallback(_)
                | RootRow::Calculator
                | RootRow::Update => None,
            })
            .map(|item| item.name().to_owned())
            .collect();
        assert!(rows.len() >= 2, "need several rows: {rows:?}");

        for (position, name) in rows.iter().enumerate().take(9) {
            let digit = (position + 1).to_string();
            let _ = app.update(super::tests::ctrl_digit(&digit));
            assert_eq!(
                app.selected_item().map(compass_core::AppItem::name),
                Some(name.as_str()),
                "Ctrl+{digit} must resolve to row {position}"
            );
        }
    }

    #[test]
    fn a_row_that_is_not_there_does_nothing_at_all() {
        // THE BOUNDS ARE THE ROWS', NOT THE INDEX'S. A query filters the list,
        // so `results` is shorter than `app_index.items()` -- and a check
        // against the wrong one accepts a position that has no row, then
        // selects it. The fixture is queried precisely so the two lengths
        // differ: a control that swapped them was SILENT until this test did.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = crate::app::test_support::app_with_rows(dir.path());
        let _ = app.update(Message::QueryChanged("fir".to_owned()));

        let rows = app.results.len();
        let items = app.app_index.items().len();
        assert!(
            rows < items,
            "the query must narrow the list: {rows} of {items}"
        );
        assert!(rows < 9, "and leave fewer than nine rows: {rows}");

        let before = app.selected;
        // A position with no row, but well inside the item index.
        let _ = app.update(super::tests::ctrl_digit(&(rows + 1).to_string()));
        assert_eq!(app.selected, before, "the selection must not move either");
    }

    #[test]
    fn the_setting_turns_it_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = crate::app::test_support::app_with_rows(dir.path());
        app.quick_launch = false;
        app.selected = 1;

        let _ = app.update(super::tests::ctrl_digit("1"));
        assert_eq!(
            app.selected, 1,
            "with quick_launch off, Ctrl+1 must not move the selection"
        );
    }

    #[test]
    fn it_is_on_by_default() {
        // #87 asks for on by default: it costs nothing when unused, because
        // the chords are otherwise unbound.
        assert!(compass_core::config::LauncherConfig::default().quick_launch());
        assert!(AppFlags::default().quick_launch);
    }

    #[test]
    fn the_panel_takes_the_chord_while_it_is_open() {
        // While the panel is open the list is not what a keystroke is aimed
        // at, and launching a row out from under an open action panel would be
        // a surprise.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = crate::app::test_support::app_with_rows(dir.path());
        let _ = app.update(Message::TogglePanel);
        assert!(app.panel.is_some(), "precondition");
        app.selected = 1;

        let _ = app.update(super::tests::ctrl_digit("1"));
        assert_eq!(app.selected, 1, "the row under the panel did not move");
        assert!(app.panel.is_some(), "and the panel is still open");
    }

    #[test]
    fn a_named_key_is_not_a_quick_launch() {
        assert_eq!(
            quick_launch_position(
                Key::Named(iced::keyboard::key::Named::Enter),
                Modifiers::CTRL
            ),
            None
        );
    }
}
