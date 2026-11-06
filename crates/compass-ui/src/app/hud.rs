//! The HUD in the launcher: `NavigationController::showHud` closes the
//! window and, where the presentation can show one, puts up the pill
//! (`crate::hud`) for a moment.

use iced::widget::{container, row, text};
use iced::{Alignment, Border, Color, Element, Length, Theme};

use super::{Dismissal, LauncherApp, Message, Task};
use crate::hud::{Hud, Step, TEXT_MAX_WIDTH};

/// Puts `text` on the clipboard: the engine's, where there is an engine.
///
/// Not Iced's clipboard first, because its source lives on the window's
/// Wayland connection and both iced_layershell and iced_winit drop it with
/// the last surface: a copy that hides the launcher (most of them) was gone
/// before anything could paste it. The engine keeps its source for as long
/// as it runs. Where it cannot copy, [`Message::TextCopied`] falls back to
/// Iced's, which holds while the window does.
pub(super) fn copy_text(
    backend: Option<std::sync::Arc<dyn crate::backend::ApplicationBackend>>,
    text: String,
) -> Task<Message> {
    let Some(backend) = backend else {
        return iced::clipboard::write(text);
    };
    Task::perform(
        async move {
            let result = backend.copy_text(text.clone()).await;
            (text, result)
        },
        |(text, result)| Message::TextCopied { text, result },
    )
}

impl LauncherApp {
    /// The engine's answer to [`copy_text`]: nothing more to do, or copy
    /// through the window where it could not.
    pub(super) fn text_copied(text: String, result: Result<(), String>) -> Task<Message> {
        match result {
            Ok(()) => Task::none(),
            Err(reason) => {
                tracing::warn!(%reason, "the engine could not copy; copying through the window");
                iced::clipboard::write(text)
            }
        }
    }

    /// Hides the launcher and shows `hud`, as `showHud`: the window closes
    /// whether or not a HUD can be shown. A launcher that exits on dismissal
    /// has no process left to show one from.
    pub(super) fn show_hud(&mut self, hud: Hud) -> Task<Message> {
        let hidden = self.conceal();
        Task::batch([hidden, self.put_up_hud(hud)])
    }

    /// Clipboard write, then "Copied to clipboard", as `CopyToClipboardAction`.
    pub(super) fn copy_with_hud(&mut self, text: String) -> Task<Message> {
        Task::batch([
            copy_text(self.backend.clone(), text),
            self.show_hud(Hud::copied()),
        ])
    }

    /// The HUD without touching the launcher, for a message the engine sent
    /// (`WindowCommand::Hud`); `false` when this presentation has none.
    pub(super) fn put_up_hud_for_engine(&mut self, hud: Hud) -> (bool, Task<Message>) {
        if !self.hud.supported() || self.on_dismiss() == Dismissal::Exit {
            return (false, Task::none());
        }
        let task = self.put_up_hud(hud);
        (true, task)
    }

    fn put_up_hud(&mut self, hud: Hud) -> Task<Message> {
        if self.on_dismiss() == Dismissal::Exit {
            return Task::none();
        }
        match self.hud.show(hud, std::time::Instant::now()) {
            Step::Open(id) => crate::surface::open_hud(id),
            Step::Unsupported | Step::Update => Task::none(),
        }
    }

    /// A tick while the HUD's timer runs: closes it once the time is up.
    pub(super) fn hud_tick(&mut self, now: std::time::Instant) -> Task<Message> {
        self.hud
            .tick(now)
            .map_or_else(Task::none, iced::window::close)
    }

    /// Ticks while the HUD is up, so it closes on time.
    pub(super) fn hud_subscription(&self) -> Option<iced::Subscription<Message>> {
        self.hud
            .counting()
            .then(|| iced::time::every(std::time::Duration::from_millis(100)).map(Message::HudTick))
    }

    /// What the HUD shows, while it is up. For tests.
    #[must_use]
    pub fn hud_content(&self) -> Option<&Hud> {
        self.hud.content()
    }

    /// Lets the HUD show, as on a layer-shell presentation. For tests.
    #[must_use]
    pub fn with_hud(mut self, supported: bool) -> Self {
        self.hud.set_supported(supported);
        self
    }

    /// The view for surface `window`: the HUD's pill, the backdrop, or the
    /// launcher — nothing while a layer surface is still measuring its output
    /// (`window_sized`), which would show the card at the wrong size for a
    /// frame.
    pub fn view_for(&self, window: iced::window::Id) -> Element<'_, Message> {
        if self.hud.owns(window) {
            self.hud_view()
        } else if self.backdrop == Some(window) {
            iced::widget::mouse_area(container(text("")).width(Length::Fill).height(Length::Fill))
                .on_press(Message::BackdropPressed)
                .on_right_press(Message::BackdropPressed)
                .on_middle_press(Message::BackdropPressed)
                .into()
        } else if self.measuring == Some(window) {
            container(text("")).into()
        } else {
            self.view()
        }
    }

    /// `HudWindow.qml`: a pill in the background colour at 90%, the divider
    /// for its edge, a 16 px icon and one line of smaller text, elided at
    /// 270 px, centred in a transparent surface.
    fn hud_view(&self) -> Element<'_, Message> {
        let palette = self.palette();
        let Some(hud) = self.hud.content() else {
            return container(text("")).into();
        };
        let foreground = palette.text.to_iced();
        let size = 13.0;
        let mut line = row![].spacing(5).align_y(Alignment::Center);
        if let Some(icon) = hud.icon.as_deref() {
            if !hud.icon_is_builtin() {
                line = line.push(text(icon.to_owned()).size(size));
            } else if let Some(drawn) = self.builtin_svg(icon, foreground, 16.0) {
                line = line.push(drawn);
            }
        }
        // Elided rather than clipped, as `Layout.maximumWidth` with
        // `elide: Text.ElideRight`: a launch failure can be a long line.
        line = line.push(
            container(crate::elided::elided_shrink(
                hud.text.clone(),
                size,
                Some(self.font()),
                foreground,
            ))
            .max_width(TEXT_MAX_WIDTH),
        );
        let background = palette.surface.to_iced();
        let border = palette.border.to_iced();
        let pill = container(line)
            .padding([10, 15])
            .style(move |_: &Theme| container::Style {
                background: Some(
                    Color {
                        a: 0.9,
                        ..background
                    }
                    .into(),
                ),
                border: Border {
                    color: border,
                    width: 1.0,
                    radius: 999.0.into(),
                },
                ..container::Style::default()
            });
        container(pill)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RootRow;
    use crate::app::test_support::{TestBackend, index, settle, with_resident_hud};
    use crate::resident::{EngineLink, UiCommand, UiOutcome};
    use iced::window;
    use std::sync::Arc;

    #[test]
    fn a_copy_goes_through_the_engine_and_through_the_window_only_where_it_cannot() {
        let dir = tempfile::tempdir().unwrap();
        for copies in [true, false] {
            let backend = Arc::new(TestBackend {
                copies,
                ..TestBackend::default()
            });
            let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
            app.query = "5 ft to m".into();
            app.search();
            assert_eq!(app.selected_row(), Some(RootRow::Calculator));
            app.backend = Some(backend.clone());
            let task = app.update(Message::LaunchSelected);
            let window_writes = settle(&mut app, task);
            if copies {
                assert_eq!(
                    backend.copied.lock().unwrap().as_slice(),
                    ["1.524"],
                    "the engine's clipboard outlives the window; the value, not `1.524 m`"
                );
                assert!(window_writes.is_empty(), "{window_writes:?}");
            } else {
                assert_eq!(window_writes, ["1.524"], "the window's, as a fallback");
            }
        }
    }

    #[test]
    fn a_copied_answer_shows_the_calculators_hud_until_its_time_is_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.query = "6*7".into();
        app.search();
        assert_eq!(app.selected_row(), Some(RootRow::Calculator));
        let shown_at = std::time::Instant::now();
        let task = app.update(Message::LaunchSelected);
        assert_eq!(settle(&mut app, task), ["42"]);
        let hud = app.hud_content().expect("a HUD").clone();
        assert_eq!(hud.text, "Answer copied to clipboard");
        assert_eq!(hud.icon.as_deref(), Some(crate::hud::COPY_ICON));

        let _ = app.update(Message::HudTick(shown_at));
        assert!(app.hud_content().is_some(), "not yet");
        let _ = app.update(Message::HudTick(
            shown_at + crate::hud::DURATION + std::time::Duration::from_millis(100),
        ));
        assert!(app.hud_content().is_none(), "gone after 1.5 s");
    }

    #[test]
    fn without_a_hud_a_copy_only_hides_and_an_exiting_launcher_shows_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path()))).with_hud(false);
        app.query = "6*7".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        assert_eq!(settle(&mut app, task), ["42"]);
        assert!(app.hud_content().is_none(), "GNOME's toplevel has no HUD");

        let mut exiting = LauncherApp::with_index(index(dir.path())).with_hud(true);
        exiting.query = "6*7".into();
        exiting.search();
        let task = exiting.update(Message::LaunchSelected);
        settle(&mut exiting, task);
        assert!(
            exiting.hud_content().is_none(),
            "no process left to show it"
        );
    }

    #[test]
    fn the_hud_surface_is_not_taken_for_the_launcher_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let launcher = window::Id::unique();
        let _ = app.update(Message::Opened(launcher));
        assert_eq!(app.window, Some(launcher));
        let _ = app.update(Message::Command(UiCommand::Hud {
            text: "Paused".into(),
            icon: Some("pause".into()),
        }));
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Paused")
        );
        assert_eq!(app.window, Some(launcher), "the launcher stays up");
        let hud = app.hud.window_id().expect("the HUD surface");
        let _ = app.update(Message::Opened(hud));
        assert_eq!(app.window, Some(launcher), "the HUD is not the launcher");
        let _ = app.update(Message::Closed(hud));
        assert_eq!(app.window, Some(launcher));
        assert!(app.hud_content().is_none());
    }

    #[test]
    fn the_engines_hud_is_refused_where_the_presentation_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path())).with_hud(false);
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
        app.link = Some(EngineLink::new(receiver, sender));
        let _ = app.update(Message::Command(UiCommand::Hud {
            text: "Next Track".into(),
            icon: None,
        }));
        assert!(matches!(outcomes.try_recv(), Ok(UiOutcome::Failed(_))));

        app.hud.set_supported(true);
        let _ = app.update(Message::Command(UiCommand::Hud {
            text: "Next Track".into(),
            icon: Some("forward".into()),
        }));
        assert_eq!(outcomes.try_recv().ok(), Some(UiOutcome::Hidden));
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Next Track")
        );
    }

    /// A no-view command that failed after the launcher hid: the reason is in
    /// the HUD now, and on the launcher's error line when it is next shown,
    /// not only in a desktop notification nobody may be there to post.
    #[test]
    fn a_no_view_commands_failure_is_said_in_the_hud_and_kept_for_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path())).with_hud(true);
        let (_commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
        app.link = Some(EngineLink::new(receiver, sender));
        let _ = app.update(Message::Command(UiCommand::Failure {
            title: "Could not complete New Window".into(),
            message: "AppleScript is only supported on macOS".into(),
        }));
        assert_eq!(outcomes.try_recv().ok(), Some(UiOutcome::Hidden));
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("AppleScript is only supported on macOS"),
            "the HUD has room for the reason"
        );
        assert_eq!(
            app.error.as_deref(),
            Some("Could not complete New Window: AppleScript is only supported on macOS")
        );
        // The next summon opens a window and searches again.
        let _ = app.update(Message::Opened(window::Id::unique()));
        let mut ui = iced_test::simulator(app.view());
        assert!(
            ui.find("Could not complete New Window: AppleScript is only supported on macOS")
                .is_ok(),
            "the launcher shows it: {}",
            app.state_line()
        );
    }

    #[test]
    fn a_long_hud_message_is_elided_inside_the_surface_and_a_short_one_hugs_its_text() {
        // The pill's width in a frame of the HUD surface: the columns whose
        // middle row is not transparent.
        fn pill_width(app: &LauncherApp) -> usize {
            let id = app.hud.window_id().expect("the HUD surface");
            let (width, height) = crate::hud::SURFACE_SIZE;
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(width as f32, height as f32),
                app.view_for(id),
            );
            let snapshot = ui.snapshot(&app.theme()).expect("the HUD renders");
            if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
                let name = format!("hud-{}", app.hud_content().map_or(0, |hud| hud.text.len()));
                let _ = snapshot.matches_image(std::path::PathBuf::from(&directory).join(name));
            }
            let dir = tempfile::tempdir().unwrap();
            let _ = snapshot.matches_image(dir.path().join("hud"));
            let png = std::fs::read_dir(dir.path())
                .unwrap()
                .find_map(|entry| {
                    let path = entry.ok()?.path();
                    path.extension().is_some_and(|x| x == "png").then_some(path)
                })
                .expect("a PNG");
            let image = ::image::open(png).unwrap().to_rgba8();
            // In surface pixels: the snapshot may be at a higher scale.
            let scale = image.width() as usize / crate::hud::SURFACE_SIZE.0 as usize;
            let row = image.height() / 2;
            let backdrop = *image.get_pixel(0, row);
            (0..image.width())
                .filter(|&x| *image.get_pixel(x, row) != backdrop)
                .count()
                / scale.max(1)
        }

        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let _ = app.show_hud(crate::hud::Hud::new("Copied"));
        let short = pill_width(&app);
        let _ = app.show_hud(crate::hud::Hud::new(
            "Could not start the extension: the worker exited before it answered, \
             with status 1 and nothing on its standard error",
        ));
        let long = pill_width(&app);
        let surface = crate::hud::SURFACE_SIZE.0 as usize;
        assert!(
            short < 120,
            "a short message keeps a short pill: {short} px"
        );
        assert!(
            long > short && long < surface,
            "a long one is cut inside the {surface} px surface: {long} px"
        );
    }

    #[test]
    fn a_refused_paste_copies_the_glyph_with_the_copy_hud() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        let task = app.emoji_pasted("🙂".into(), Err("no paste here".into()));
        assert_eq!(settle(&mut app, task), ["🙂"]);
        assert_eq!(app.hud_content(), Some(&crate::hud::Hud::copied()));
    }
}
