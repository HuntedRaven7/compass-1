//! Script commands in the launcher: their root rows, the form their
//! arguments are entered in, and the full-output view.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the decisions that do not need that state are in
//! [`crate::script_page`].

use compass_core::script_command::OutputMode;
use compass_core::script_output::Color as OutputColor;
use iced::keyboard::{Key, Modifiers, key::Named};

use super::a11y::PageSemantics;
use super::{
    Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState, RootRow, Task,
    column, container, scrollable, text,
};
use crate::action_panel::Action;
use crate::preferences_page::Purpose;
use crate::script_page::{self, ScriptOutputPage};
use crate::semantics::{Body, Document, text_blocks};

const RUN: &str = "script.run";
const COPY_PATH: &str = "script.copy-path";
const KILL: &str = "script.kill";
const RERUN: &str = "script.rerun";
const COPY_OUTPUT: &str = "script.copy-output";

/// How far one arrow press scrolls the output, and one Page Up or Down.
const LINE_STEP: f32 = 40.0;
const PAGE_STEP: f32 = 320.0;

/// How often a followed run is asked for its output.
const POLL: std::time::Duration = std::time::Duration::from_millis(250);

const NEEDS_ENGINE: &str =
    "Script commands need the Compass engine, and this window is running without one";

/// A one-line run the root list is waiting on.
#[derive(Debug, Clone)]
pub(super) struct FollowedScript {
    /// The run.
    pub session: u64,
    /// `compact` or `inline`.
    pub mode: OutputMode,
}

impl LauncherApp {
    /// Asks the engine to scan the script directories, so root search shows
    /// what is on disk now.
    pub(super) fn refresh_scripts_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.list_scripts().await },
            Message::ScriptsLoaded,
        )
    }

    /// Runs the script at `index`: through its form when it takes arguments
    /// or asks for confirmation, at once otherwise.
    pub(super) fn run_script_at(&mut self, index: usize) -> Task<Message> {
        let Some(script) = self.app_index.scripts().get(index) else {
            return Task::none();
        };
        self.panel = None;
        if let Some(form) = script_page::arguments_form(script) {
            self.page = Page::Preferences(Box::new(form));
            return Task::none();
        }
        let id = script.id.clone();
        self.send_run_script(id, Vec::new())
    }

    /// Asks the engine to run a script.
    fn send_run_script(&mut self, id: String, arguments: Vec<String>) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            self.script_notice(NEEDS_ENGINE.to_owned());
            return Task::none();
        };
        let key = compass_core::root_items::entrypoint_id(
            compass_core::script_scan::SCRIPTS_PROVIDER_ID,
            &id,
        );
        Task::perform(
            {
                let (id, arguments) = (id.clone(), arguments.clone());
                async move {
                    let started = backend.run_script(id, arguments).await?;
                    if let Err(error) = backend.record_launch(key).await {
                        tracing::warn!(%error, "could not record running a script");
                    }
                    Ok(started)
                }
            },
            move |result| Message::ScriptStarted {
                id: id.clone(),
                arguments: arguments.clone(),
                result,
            },
        )
    }

    /// Asks for run `session`'s output, after `delay`.
    fn poll_script(&self, session: u64, delay: std::time::Duration) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                backend.script_output(session).await
            },
            move |result| Message::ScriptPolled { session, result },
        )
    }

    /// Shows why a script did not run, where the person is.
    fn script_notice(&mut self, reason: String) {
        match &mut self.page {
            Page::ScriptOutput(page) => page.notice = Some(reason),
            Page::Preferences(page) => page.notice = Some(reason),
            _ => self.error = Some(reason),
        }
    }

    /// Submits a script's form. `None` when the form showing is not one.
    pub(super) fn submit_script_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(page) = &mut self.page else {
            return None;
        };
        if page.purpose != Purpose::ScriptArguments {
            return None;
        }
        if let Err(missing) = page.submission() {
            page.notice = Some(format!("Fill in {}", missing.join(", ")));
            return Some(Task::none());
        }
        let id = page.command_id.clone();
        let arguments = script_page::argument_values(page);
        Some(self.send_run_script(id, arguments))
    }

    /// Stops the followed run, if it is still going.
    pub(super) fn stop_followed_script(&mut self) -> Task<Message> {
        let Page::ScriptOutput(page) = &self.page else {
            return Task::none();
        };
        if page.state.finished {
            return Task::none();
        }
        let (session, Some(backend)) = (page.session, self.backend.clone()) else {
            return Task::none();
        };
        Task::perform(
            async move { backend.stop_script(session).await },
            move |result| match result {
                // The next report says it ended.
                Ok(()) => Message::ScriptPolled {
                    session,
                    result: Err(String::new()),
                },
                Err(reason) => Message::ScriptPolled {
                    session,
                    result: Err(reason),
                },
            },
        )
    }

    /// The panel over a script row, or over the full-output view.
    pub(super) fn open_script_panel(&mut self) -> Option<Task<Message>> {
        let actions = match &self.page {
            Page::ScriptOutput(page) => {
                let mut actions = if page.state.finished {
                    vec![
                        Action::new("Run script again")
                            .with_id(RERUN)
                            .with_shortcut("ctrl+r"),
                    ]
                } else {
                    vec![Action::new("Kill process").with_id(KILL)]
                };
                actions.push(
                    Action::new("Copy output")
                        .with_id(COPY_OUTPUT)
                        .with_shortcut("ctrl+shift+c"),
                );
                actions
            }
            Page::Root => match self.selected_row()? {
                RootRow::Script(_) => vec![
                    Action::new("Run script")
                        .with_id(RUN)
                        .with_shortcut("enter"),
                    Action::new("Copy path to script").with_id(COPY_PATH),
                ],
                _ => return None,
            },
            _ => return None,
        };
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions,
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a script action from the panel, if `id` is one.
    pub(super) fn script_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let task = match id {
            RUN => match self.selected_row()? {
                RootRow::Script(index) => self.run_script_at(index),
                _ => return None,
            },
            COPY_PATH => match self.selected_row()? {
                RootRow::Script(index) => {
                    let path = self.app_index.scripts().get(index)?.path.clone();
                    super::hud::copy_text(self.backend.clone(), path)
                }
                _ => return None,
            },
            KILL => self.stop_followed_script(),
            RERUN => self.rerun_script(),
            COPY_OUTPUT => {
                self.panel = None;
                return Some(self.copy_script_output());
            }
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// Runs the full-output view's script again, with the same arguments.
    fn rerun_script(&mut self) -> Task<Message> {
        let Page::ScriptOutput(page) = &self.page else {
            return Task::none();
        };
        let (id, arguments) = (page.script_id.clone(), page.arguments.clone());
        self.send_run_script(id, arguments)
    }

    /// Copies the output as it reads, without its colour codes.
    fn copy_script_output(&mut self) -> Task<Message> {
        let Page::ScriptOutput(page) = &self.page else {
            return Task::none();
        };
        let text = script_page::plain_text(&page.state.output);
        self.copy_with_hud(text)
    }

    /// The full-output view's keys: Escape stops a running script and goes
    /// back, Ctrl+R runs it again once it has ended, Ctrl+Shift+C copies the
    /// output, and the arrows, Page Up and Down, Home and End scroll it.
    pub(super) fn script_output_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        use iced::widget::operation::{self, AbsoluteOffset, RelativeOffset};
        let scroll = |y: f32| {
            operation::scroll_by(crate::scroll::ROOT_RESULTS, AbsoluteOffset { x: 0.0, y })
        };
        match key.as_ref() {
            Key::Named(Named::ArrowDown) => scroll(LINE_STEP),
            Key::Named(Named::ArrowUp) => scroll(-LINE_STEP),
            Key::Named(Named::PageDown) => scroll(PAGE_STEP),
            Key::Named(Named::PageUp) => scroll(-PAGE_STEP),
            Key::Named(Named::Home) => {
                operation::snap_to(crate::scroll::ROOT_RESULTS, RelativeOffset::START)
            }
            Key::Named(Named::End) => operation::snap_to_end(crate::scroll::ROOT_RESULTS),
            Key::Character(c)
                if modifiers.control() && modifiers.shift() && c.eq_ignore_ascii_case("c") =>
            {
                self.copy_script_output()
            }
            Key::Named(Named::Escape) => {
                let stop = self.stop_followed_script();
                let back = self.update(Message::Back);
                Task::batch([stop, back])
            }
            Key::Character(c) if modifiers.control() && c.eq_ignore_ascii_case("r") => {
                match &self.page {
                    Page::ScriptOutput(page) if page.state.finished => self.rerun_script(),
                    _ => Task::none(),
                }
            }
            _ => Task::none(),
        }
    }

    /// Asks the engine for each script command's icon, when icons are drawn.
    fn script_icons_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone().filter(|_| self.icons) else {
            return Task::none();
        };
        Task::perform(
            async move { backend.script_icons().await },
            Message::ScriptIconsLoaded,
        )
    }

    /// Handles the script messages.
    pub(super) fn script_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ScriptsLoaded(Ok(scripts)) => {
                let changed = self.app_index.scripts() != scripts.as_slice();
                self.app_index.set_scripts(scripts);
                let icons = self.script_icons_task();
                if changed && matches!(self.page, Page::Root) && !self.query.trim().is_empty() {
                    return Task::batch([self.search_task(), icons]);
                }
                icons
            }
            Message::ScriptIconsLoaded(Ok(icons)) => {
                self.script_icons = icons
                    .into_iter()
                    .map(|(id, url)| (id, compass_core::image_url::ImageUrl::parse(&url)))
                    .collect();
                self.warm_icons();
                Task::none()
            }
            Message::ScriptIconsLoaded(Err(reason)) => {
                tracing::debug!(%reason, "could not list script icons");
                Task::none()
            }
            Message::ScriptsLoaded(Err(reason)) => {
                tracing::debug!(%reason, "could not list script commands");
                Task::none()
            }
            Message::ScriptStarted {
                id,
                arguments,
                result,
            } => {
                let session = match result {
                    Ok(Some(session)) => session,
                    Ok(None) => {
                        if matches!(self.page, Page::Preferences(_)) {
                            self.page = Page::Root;
                        }
                        return self.conceal();
                    }
                    Err(reason) => {
                        self.script_notice(reason);
                        return Task::none();
                    }
                };
                let script = self
                    .app_index
                    .scripts()
                    .iter()
                    .find(|script| script.id == id);
                let mode = script.map_or(OutputMode::Full, |script| script.mode);
                let title = script
                    .map(|script| script.title.clone())
                    .unwrap_or(id.clone());
                self.panel = None;
                if mode == OutputMode::Full {
                    self.following_script = None;
                    self.page =
                        Page::ScriptOutput(ScriptOutputPage::new(id, arguments, title, session));
                } else {
                    if matches!(self.page, Page::Preferences(_)) {
                        self.page = Page::Root;
                    }
                    self.following_script = Some(FollowedScript { session, mode });
                    // Its line shows in place of the results; the search
                    // field keeps the keyboard, so typing replaces it.
                    return Task::batch([
                        self.poll_script(session, std::time::Duration::ZERO),
                        super::focus_search(),
                    ]);
                }
                self.poll_script(session, std::time::Duration::ZERO)
            }
            Message::ScriptPolled { session, result } => {
                let state = match result {
                    Ok(state) => state,
                    // An empty reason is a stop that went through: ask again
                    // for the final report.
                    Err(reason) if reason.is_empty() => return self.poll_script(session, POLL),
                    Err(reason) => {
                        self.script_notice(reason);
                        return Task::none();
                    }
                };
                if let Page::ScriptOutput(page) = &mut self.page
                    && page.session == session
                {
                    return if page.apply(session, state) {
                        self.poll_script(session, POLL)
                    } else {
                        Task::none()
                    };
                }
                let Some(followed) = self.following_script.clone() else {
                    return Task::none();
                };
                if followed.session != session {
                    return Task::none();
                }
                if !state.finished {
                    return self.poll_script(session, POLL);
                }
                self.following_script = None;
                let ok = state.exit_code == Some(0);
                let line =
                    script_page::plain_text(state.output.split('\n').next().unwrap_or_default());
                if followed.mode == OutputMode::Inline && ok {
                    // Its line is its subtitle now; the engine has it.
                    return self.refresh_scripts_task();
                }
                self.error = Some(one_line_message(followed.mode, ok, &line));
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The full-output view's body: how the run stands, then its output in
    /// the monospace font, coloured as the script asked.
    pub(super) fn script_output_body<'a>(
        &'a self,
        page: &'a ScriptOutputPage,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let colour = |colour: OutputColor| -> iced::Color {
            match colour {
                OutputColor::Red => iced::Color::from_rgb8(0xe0, 0x1b, 0x24),
                OutputColor::Green => iced::Color::from_rgb8(0x26, 0xa2, 0x69),
                OutputColor::Yellow => iced::Color::from_rgb8(0xe5, 0xa5, 0x0a),
                OutputColor::Blue => iced::Color::from_rgb8(0x35, 0x84, 0xe4),
                OutputColor::Magenta => iced::Color::from_rgb8(0x91, 0x41, 0xac),
                OutputColor::Cyan => iced::Color::from_rgb8(0x2a, 0xa1, 0xb3),
                OutputColor::Black => iced::Color::BLACK,
                OutputColor::TextPrimary => palette.text.to_iced(),
            }
        };
        let spans: Vec<iced::widget::text::Span<'a, String, iced::Font>> = page
            .runs
            .iter()
            .map(|run| {
                let mut span = iced::widget::span(run.text.as_str())
                    .font(iced::Font::MONOSPACE)
                    .color(run.foreground.map_or(palette.text.to_iced(), colour));
                if let Some(background) = run.background {
                    span = span.background(colour(background));
                }
                if run.link {
                    span = span.underline(true).link(run.text.clone());
                }
                span
            })
            .collect();
        let heading =
            text(page.heading_line())
                .font(self.font())
                .size(12)
                .color(if page.failed() {
                    colour(OutputColor::Red)
                } else {
                    palette.muted.to_iced()
                });
        let output = iced::widget::rich_text(spans)
            .size(13)
            .width(Length::Fill)
            .on_link_click(Message::ExtensionLinkClicked);
        let mut body = column![heading, output].spacing(8).width(Length::Fill);
        if let Some(notice) = &page.notice {
            body = body.push(text(notice.clone()).font(self.font()).size(12));
        }
        scrollable(
            container(body)
                .width(Length::Fill)
                .padding(Padding::new(12.0)),
        )
        .id(crate::scroll::ROOT_RESULTS)
        .width(Length::Fill)
        .height(Length::Shrink)
        .into()
    }

    /// The full-output view for assistive technology: the output a line at
    /// a time, under the line that says how the run stands.
    pub(super) fn script_output_semantics(&self, page: &ScriptOutputPage) -> PageSemantics {
        PageSemantics {
            body: Body::Document(Document {
                label: page.heading_line(),
                blocks: text_blocks(&page.text()),
                metadata: Vec::new(),
            }),
            status: page.notice.clone(),
            detail: None,
        }
    }
}

/// What a finished one-line run says in the root list: its line, or the
/// mode's sentence when it printed nothing.
fn one_line_message(mode: OutputMode, ok: bool, line: &str) -> String {
    if !line.is_empty() {
        return line.to_owned();
    }
    match (mode, ok) {
        (OutputMode::Inline, false) => "Script exited with error code",
        (_, true) => "Script executed",
        (_, false) => "Script execution failed",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantics::Block;

    #[test]
    fn the_full_output_is_read_a_line_at_a_time_under_how_the_run_ended() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page =
            ScriptOutputPage::new("report.sh".into(), Vec::new(), "Disk Report".into(), 1);
        page.apply(
            1,
            crate::backend::ScriptOutputState {
                output: "\x1b[31mDisk\x1b[0m usage\n\n/home 40%\n".into(),
                finished: true,
                exit_code: Some(0),
                elapsed_ms: 1200,
            },
        );
        page.notice = Some("Copied to clipboard".into());
        let semantics = app.script_output_semantics(&page);
        let Body::Document(document) = &semantics.body else {
            panic!("expected a document, got {:?}", semantics.body);
        };
        assert_eq!(document.label, "Disk Report — Done in 1.2s (exit=0)");
        let lines: Vec<&str> = document
            .blocks
            .iter()
            .map(|block: &Block| block.text.as_str())
            .collect();
        assert_eq!(lines, ["Disk usage", "/home 40%"]);
        assert_eq!(semantics.status.as_deref(), Some("Copied to clipboard"));
        app.page = Page::ScriptOutput(page);
        assert!(matches!(app.semantics().body, Body::Document(_)));
    }
}
