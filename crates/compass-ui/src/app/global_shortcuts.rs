//! The launcher's side of the global shortcuts: hiding on focus loss
//! (`launcher.close_on_focus_loss`, `NavigationController::setWindowActivated`),
//! and telling the engine when the shortcut recorder captures, so the
//! desktop's bindings step aside (`GlobalShortcutBridge::setCapturing`).

use iced::Task;

use super::{LauncherApp, Page};
use crate::message::Message;

impl LauncherApp {
    /// The window gained or lost the focus. Losing it after having had it
    /// hides the window when `close_on_focus_loss` is on, as the C++ closes
    /// on the activated-to-deactivated edge; losing it to the file chooser
    /// the window opened is not the user's doing and is ignored.
    pub(super) fn window_focus_changed(&mut self, focused: bool) -> Task<Message> {
        if focused {
            self.window_focused = true;
            self.focus_seen = true;
            return self.ask_monitor_size();
        }
        let had = std::mem::take(&mut self.window_focused);
        if !had
            || !self.close_on_focus_loss
            || self.choosing_files
            || !self.is_visible()
            || self.closing
        {
            return Task::none();
        }
        self.conceal()
    }

    /// Asks once per toplevel for the monitor it is on, so the window can be
    /// fitted to it (`monitor_sized`). Asked on the focus rather than on
    /// opening because the toolkit learns the monitor from the compositor's
    /// `enter`, which comes once the window is mapped. A layer surface
    /// measures its output itself.
    fn ask_monitor_size(&mut self) -> Task<Message> {
        let Some(id) = self.window else {
            return Task::none();
        };
        if crate::surface::opens_to_measure() || self.monitor_asked == Some(id) {
            return Task::none();
        }
        self.monitor_asked = Some(id);
        iced::window::monitor_size(id).map(move |size| Message::MonitorSized(id, size))
    }

    /// Whether the window is hiding on focus loss, for tests.
    #[must_use]
    pub fn closes_on_focus_loss(&self) -> bool {
        self.close_on_focus_loss
    }

    /// Whether a shortcut recorder is capturing: the action panel's, or the
    /// settings view's.
    pub(super) fn recording_shortcut(&self) -> bool {
        self.panel
            .as_ref()
            .is_some_and(|panel| panel.recorder.is_some())
            || matches!(&self.page, Page::Settings(page) if page.recorder.is_some())
    }

    /// Tells the engine when capturing starts or stops, once per change;
    /// `None` when there is nothing to tell.
    pub(super) fn report_shortcut_capture(&mut self) -> Option<Task<Message>> {
        let capturing = self.recording_shortcut();
        if capturing == self.capture_reported {
            return None;
        }
        self.capture_reported = capturing;
        let backend = self.backend.clone()?;
        Some(Task::perform(
            async move { backend.set_shortcut_capture(capturing).await },
            Message::ShortcutCaptureSet,
        ))
    }

    /// Whether the engine was last told the recorder is capturing, for
    /// tests.
    #[must_use]
    pub fn capture_reported(&self) -> bool {
        self.capture_reported
    }

    /// Asks the engine whether the desktop would bind `trigger`, which a
    /// recorder captured (`probeBind`). Without an engine nothing is bound,
    /// so there is nobody to refuse it and the answer is given at once.
    pub(super) fn probe_shortcut(&mut self, trigger: String) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return self.shortcut_probed(&trigger, Ok(None));
        };
        Task::perform(
            async move {
                let result = backend.probe_shortcut(trigger.clone()).await;
                (trigger, result)
            },
            |(trigger, result)| Message::ShortcutProbed(trigger, result),
        )
    }

    /// The engine's answer about `trigger`, handed to whichever recorder
    /// asked. An engine that could not be asked stands in nobody's way.
    pub(super) fn shortcut_probed(
        &mut self,
        trigger: &str,
        result: Result<Option<String>, String>,
    ) -> Task<Message> {
        let refusal = result.unwrap_or_else(|error| {
            tracing::warn!(%error, "could not ask whether the desktop would bind the shortcut");
            None
        });
        if let Some(recorder) = self
            .panel
            .as_mut()
            .and_then(|panel| panel.recorder.as_mut())
        {
            let outcome = recorder.probed(trigger, refusal);
            return self.panel_recorder_outcome(outcome);
        }
        if let Page::Settings(page) = &mut self.page
            && let Some((target, recorder)) = page.recorder.as_mut()
        {
            let outcome = recorder.probed(trigger, refusal);
            let target = target.clone();
            return self.settings_recorder_outcome(target, outcome);
        }
        Task::none()
    }

    /// Every root item's id, title and stored shortcut, for the recorder's
    /// conflict check.
    pub(super) fn recorder_bound(&self) -> Vec<(String, String, String)> {
        self.app_index
            .roots()
            .iter()
            .filter_map(|root| {
                let shortcut = root.meta.shortcut.clone()?;
                Some((root.id.clone(), root.title.clone(), shortcut))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::{TestBackend, app, closes, key_event, settle, shown};
    use std::sync::Arc;

    #[test]
    fn losing_the_focus_hides_the_launcher_when_close_on_focus_loss_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        app.close_on_focus_loss = true;
        let _ = app.update(Message::WindowFocusChanged(true));
        assert!(closes(app.update(Message::WindowFocusChanged(false)), id));
        let _ = app.update(Message::Closed(id));
        assert!(!app.is_visible());
    }

    #[test]
    fn losing_the_focus_keeps_the_launcher_when_close_on_focus_loss_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        assert!(!app.closes_on_focus_loss(), "off by default, as in the C++");
        let _ = app.update(Message::WindowFocusChanged(true));
        assert!(!closes(app.update(Message::WindowFocusChanged(false)), id));
        assert!(app.is_visible());
    }

    #[test]
    fn only_losing_a_focus_the_launcher_had_hides_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, id, _engine) = shown(dir.path());
        app.close_on_focus_loss = true;
        assert!(
            !closes(app.update(Message::WindowFocusChanged(false)), id),
            "never focused: the compositor did not hand it over"
        );
        let _ = app.update(Message::WindowFocusChanged(true));
        app.choosing_files = true;
        assert!(
            !closes(app.update(Message::WindowFocusChanged(false)), id),
            "the file chooser the launcher opened took it"
        );
        assert!(app.is_visible());
    }

    #[test]
    fn the_recorder_suspends_the_global_shortcuts_while_it_captures() {
        use iced::keyboard::{Key, Modifiers, key::Named};
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(TestBackend::default());
        let mut app = app(dir.path());
        let _ = app.update(Message::QueryChanged("firefox".into()));
        let _ = app.update(Message::TogglePanel);
        let row = app
            .panel
            .as_ref()
            .and_then(|panel| panel.row_titled("Set global shortcut"))
            .expect("the panel offers a shortcut");
        // Attached from here: the fake engine searches nothing.
        app.backend = Some(backend.clone());
        let task = app.update(Message::PanelClicked(row));
        settle(&mut app, task);
        assert!(app.capture_reported());
        assert_eq!(backend.captures.lock().unwrap().as_slice(), [true]);

        // The launcher hotkey and the launcher's own keys are taken.
        let task = app.update(key_event(true, Key::Named(Named::Space), Modifiers::LOGO));
        settle(&mut app, task);
        let status = |app: &LauncherApp| {
            app.panel
                .as_ref()
                .and_then(|p| p.recorder.as_ref())
                .map(|r| r.status.clone())
        };
        assert_eq!(
            status(&app).as_deref(),
            Some("Already bound to \"the launcher hotkey\"")
        );
        let task = app.update(key_event(true, Key::Character("b".into()), Modifiers::CTRL));
        settle(&mut app, task);
        assert_eq!(
            status(&app).as_deref(),
            Some("Already bound to \"Toggle action panel\"")
        );

        let task = app.update(key_event(
            true,
            Key::Named(Named::Escape),
            Modifiers::empty(),
        ));
        settle(&mut app, task);
        assert!(!app.capture_reported());
        assert_eq!(backend.captures.lock().unwrap().as_slice(), [true, false]);
    }

    #[test]
    fn the_recorder_shows_the_desktops_refusal_and_keeps_what_it_takes() {
        use iced::keyboard::{Key, Modifiers};
        let dir = tempfile::tempdir().expect("tempdir");
        let open = |backend: Arc<TestBackend>| {
            let mut app = app(dir.path());
            let _ = app.update(Message::QueryChanged("firefox".into()));
            let id = app.root_id(app.selected_row().unwrap()).unwrap();
            let _ = app.update(Message::TogglePanel);
            let row = app
                .panel
                .as_ref()
                .and_then(|panel| panel.row_titled("Set global shortcut"))
                .expect("the panel offers a shortcut");
            app.backend = Some(backend);
            let task = app.update(Message::PanelClicked(row));
            settle(&mut app, task);
            (app, id)
        };

        let refusing = Arc::new(TestBackend {
            probe_refusal: Some("The compositor has already bound super+Q".into()),
            ..TestBackend::default()
        });
        let (mut app, _) = open(refusing.clone());
        let task = app.update(key_event(true, Key::Character("q".into()), Modifiers::LOGO));
        settle(&mut app, task);
        assert_eq!(refusing.probes.lock().unwrap().as_slice(), ["super+Q"]);
        let recorder = app
            .panel
            .as_ref()
            .and_then(|p| p.recorder.as_ref())
            .expect("the recorder stays open");
        assert_eq!(recorder.status, "The compositor has already bound super+Q");
        assert!(recorder.error);
        assert!(
            refusing.root_edits.lock().unwrap().is_empty(),
            "nothing kept"
        );

        let taking = Arc::new(TestBackend::default());
        let (mut app, id) = open(taking.clone());
        let task = app.update(key_event(true, Key::Character("q".into()), Modifiers::LOGO));
        settle(&mut app, task);
        assert_eq!(taking.probes.lock().unwrap().as_slice(), ["super+Q"]);
        assert!(
            app.panel.is_none(),
            "a combination the desktop takes is kept"
        );
        assert_eq!(
            app.app_index
                .root(&id)
                .and_then(|root| root.meta.shortcut.clone())
                .as_deref(),
            Some("super+Q")
        );
    }
}
