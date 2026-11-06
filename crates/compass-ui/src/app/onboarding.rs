//! The first-run flow in the launcher: `OnboardingWindow.qml`'s Linux steps
//! (welcome, "Make it your own", "Setup complete") with Back, Continue and the
//! step dots, Enter to continue and Escape to go back (closing from the
//! first step), Tab through each step's controls, and Compass's own "Add
//! extensions" step before the last, which installs through the same backend
//! call as the store's detail page.
//!
//! "Make it your own" also sets up the launcher hotkey and starting at login.
//! The hotkey row shows where the hotkey stands with the desktop
//! (`compass_core::launcher_shortcut`) and, where asking can help, offers Set
//! Up Shortcut, which has the engine ask the GlobalShortcuts portal: the
//! desktop's permission dialog appears because the person asked for it.
//! Finishing or closing the flow asks if nobody did, since the engine waits
//! for the flow. On Sway, Hyprland and niri the row keeps the compositor's own
//! configuration line. The Start at login switch is off until it is turned on
//! (`compass_core::login_start`).
//!
//! Finishing records the flow in `onboarding.json` and leaves the launcher
//! open at its search. Closing it, with Escape or the hotkey, records it too:
//! unlike Vicinae, which asks again at the next start, Compass does not show
//! it at every login to someone who has already dismissed it. Started with
//! `--hidden`, the flow waits for the first summon.

use iced::keyboard::{Key, Modifiers, key::Named};
use iced::widget::{Space, button, column, container, pick_list, row, text};
use iced::{Alignment, Border, Element, Length, Padding, Theme};

use super::{LauncherApp, Message, Page, Task};
use crate::adwaita;
use crate::onboarding_page::{Control, OnboardingPage, ThemeOption};
use compass_core::launcher_shortcut::{self, LauncherShortcut};
use compass_core::onboarding::{self, Advance, Step};

const COMPASS_LOGO: &[u8] = include_bytes!("../../../../extra/compass.svg");

/// A recommendation's button: shorter than Adwaita's 34 px, so the whole
/// list fits the card with room for a failure's notice under it.
const INSTALL_PADDING: [f32; 2] = [5.0, 12.0];

/// How tall the theme list opened from the keyboard grows before it
/// scrolls: five themes.
const MENU_HEIGHT: f32 = 160.0;

/// What the Start at login row says before the engine has.
pub(super) const LOGIN_START_OFFER: &str =
    "Compass can start, hidden, when you log in, so the hotkey works straight away.";

/// Why Install did nothing, as the store pages say it.
const NEEDS_ENGINE: &str = "Installing extensions needs the Compass engine";

/// Why a link did not open, as `open_link` says it.
const OPEN_NEEDS_ENGINE: &str = "Opening a link needs the Compass engine";

/// Records the flow as seen, now. A file that cannot be written is logged:
/// the cost is seeing the flow again, not losing anything.
pub(super) fn record_completed(path: &std::path::Path) {
    let completed_at = jiff::Timestamp::now()
        .round(jiff::Unit::Second)
        .unwrap_or_else(|_| jiff::Timestamp::now())
        .to_string();
    if let Err(error) = onboarding::mark_completed(path, &completed_at) {
        tracing::warn!(%error, path = %path.display(), "could not write the onboarding state file");
    }
}

impl LauncherApp {
    /// How to open the launcher from anywhere on this desktop: the
    /// compositor's own line on Sway, Hyprland and niri, the hotkey
    /// elsewhere, and in the Flatpak the command the Flatpak needs.
    pub(super) fn open_hint(&self) -> String {
        let command = compass_core::hotkey_guide::toggle_command(self.flatpak);
        if let Some(compositor) = self.hotkey_compositor {
            return compositor.instruction(&command);
        }
        let key = compass_core::key_combo::KeyCombo::parse(&self.launcher_hotkey).map_or_else(
            || self.launcher_hotkey.clone(),
            |combo| combo.display_tokens().join("+"),
        );
        format!(
            "Press {key} to open Compass from anywhere. If it does nothing, bind a key to `{command}` in your desktop's keyboard settings."
        )
    }

    /// Opens the flow at its first step, recording to `state_path`.
    pub fn open_onboarding(&mut self, state_path: std::path::PathBuf) {
        let files = crate::theme::load_user_themes(&self.view.theme_dirs);
        let extensions = self.app_index.extensions();
        let installed = |id: &str| extensions.iter().any(|command| command.extension_id == id);
        let mut page = OnboardingPage::new(state_path, files, installed);
        page.compositor = self.hotkey_compositor;
        self.page = Page::Onboarding(Box::new(page));
    }

    /// The flow's hotkey sentence: where the hotkey stands with the desktop,
    /// or the compositor's own configuration line on Sway, Hyprland and niri.
    pub(super) fn hotkey_guidance(&self, page: &OnboardingPage) -> String {
        let key = compass_core::key_combo::KeyCombo::parse(&self.launcher_hotkey).map_or_else(
            || self.launcher_hotkey.clone(),
            |combo| combo.display_tokens().join("+"),
        );
        let command = compass_core::hotkey_guide::toggle_command(self.flatpak);
        launcher_shortcut::guidance(page.shortcut.as_ref(), page.compositor, &key, &command)
    }

    /// Whether Compass starts at login, as last answered.
    pub(super) fn login_start_on(&self) -> bool {
        self.login_start
            .as_ref()
            .is_some_and(|status| status.state.is_on())
    }

    /// What the Start at login row says under its label: what the engine
    /// answered starts Compass at login, or `offer` before it has.
    pub(super) fn login_start_line(&self, offer: &str) -> String {
        self.login_start.as_ref().map_or_else(
            || offer.to_owned(),
            compass_core::login_start::Status::describe,
        )
    }

    /// Asks the engine where the hotkey and login start stand, for the step
    /// now on show when it shows them.
    fn onboarding_step_shown(&self) -> Task<Message> {
        let Page::Onboarding(page) = &self.page else {
            return Task::none();
        };
        if !matches!(page.flow.step(), Step::Personalize | Step::Complete) {
            return Task::none();
        }
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        let login = backend.clone();
        Task::batch([
            Task::perform(
                async move { backend.launcher_shortcut(false).await },
                Message::LauncherShortcutAnswered,
            ),
            Task::perform(
                async move { login.login_start(None).await },
                Message::LoginStartAnswered,
            ),
        ])
    }

    /// Finishing or closing the flow: asks the desktop for the hotkey when
    /// `ask` ([`OnboardingPage::asks_on_finish`]), since the engine waits for
    /// the flow to.
    pub(super) fn ask_for_hotkey_on_finish(&self, ask: bool) -> Task<Message> {
        match self.backend.clone() {
            Some(backend) if ask => Task::perform(
                async move { backend.launcher_shortcut(true).await },
                Message::LauncherShortcutAnswered,
            ),
            _ => Task::none(),
        }
    }

    /// Says `notice` where the person is: on the flow's card, or in
    /// Settings.
    fn first_run_notice(&mut self, notice: String) {
        match &mut self.page {
            Page::Onboarding(page) => page.notice = Some(notice),
            Page::Settings(page) => page.notice = Some(notice),
            _ => self.say(notice),
        }
    }

    /// The launcher hotkey's and login start's answers, and the switch,
    /// which the flow and Settings share.
    pub(super) fn first_run_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LauncherShortcutAnswered(answer) => {
                if let Page::Onboarding(page) = &mut self.page {
                    match answer {
                        Ok(state) => page.shortcut = Some(state),
                        // Only a request the person made says why it
                        // failed; a status that could not be read leaves
                        // the row as it was.
                        Err(reason) if page.shortcut == Some(LauncherShortcut::Pending) => {
                            page.shortcut = None;
                            page.notice = Some(reason);
                        }
                        Err(_) => {}
                    }
                }
                Task::none()
            }
            Message::LoginStartToggled(enable) => {
                if self.login_start_busy {
                    return Task::none();
                }
                let Some(backend) = self.backend.clone() else {
                    self.first_run_notice(crate::backend::LOGIN_START_NEEDS_ENGINE.to_owned());
                    return Task::none();
                };
                self.login_start_busy = true;
                Task::perform(
                    async move { backend.login_start(Some(enable)).await },
                    Message::LoginStartAnswered,
                )
            }
            Message::LoginStartAnswered(answer) => {
                let asked = std::mem::take(&mut self.login_start_busy);
                match answer {
                    Ok(status) => self.login_start = Some(status),
                    Err(reason) if asked => self.first_run_notice(reason),
                    Err(_) => {}
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// Whether the flow is on screen. For tests.
    #[must_use]
    pub fn showing_onboarding(&self) -> bool {
        matches!(self.page, Page::Onboarding(_))
    }

    /// The step on screen, if the flow is. For tests.
    #[must_use]
    pub fn onboarding_step(&self) -> Option<Step> {
        match &self.page {
            Page::Onboarding(page) => Some(page.flow.step()),
            _ => None,
        }
    }

    /// What the extensions step's last failed install said, while the flow
    /// is on screen. For tests.
    #[must_use]
    pub fn onboarding_install_notice(&self) -> Option<&str> {
        match &self.page {
            Page::Onboarding(page) => page.extensions.notice(),
            _ => None,
        }
    }

    /// The flow's keys, as Settings has them: Tab and Shift+Tab walk the
    /// step's controls, then Back and Continue; Space or Enter presses the
    /// one with the keyboard or opens the theme dropdown, whose themes the
    /// arrows then move through. With no control focused Enter continues.
    /// Escape closes the dropdown, else goes back a step, and closes the
    /// flow only from its first step.
    pub(super) fn onboarding_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Onboarding(page) = &mut self.page else {
            return Task::none();
        };
        let activate = matches!(key.as_ref(), Key::Named(Named::Enter | Named::Space));
        if let Some(at) = page.menu {
            return match key.as_ref() {
                Key::Named(Named::ArrowDown) => {
                    page.menu = Some(crate::focus::move_highlight(page.themes.len(), at, 1));
                    crate::scroll::reveal_onboarding_option()
                }
                Key::Named(Named::ArrowUp) => {
                    page.menu = Some(crate::focus::move_highlight(page.themes.len(), at, -1));
                    crate::scroll::reveal_onboarding_option()
                }
                Key::Named(Named::Escape) => {
                    page.menu = None;
                    Task::none()
                }
                _ if activate => match page.themes.get(at).copied() {
                    Some(theme) => self.update(Message::OnboardingTheme(theme)),
                    None => Task::none(),
                },
                _ => Task::none(),
            };
        }
        match key.as_ref() {
            Key::Named(Named::Tab) => {
                page.move_focus(!modifiers.shift());
                Task::none()
            }
            Key::Named(Named::Escape) if page.flow.can_go_back() => {
                self.update(Message::OnboardingBack)
            }
            Key::Named(Named::Escape) => self.conceal(),
            _ if activate => match page.focused {
                Some(Control::Theme) => {
                    let current = ThemeOption(self.view.theme_choice);
                    page.menu = Some(
                        page.themes
                            .iter()
                            .position(|theme| *theme == current)
                            .unwrap_or(0),
                    );
                    Task::none()
                }
                Some(Control::OpenDocs) => {
                    self.update(Message::OnboardingOpen(onboarding::HOTKEY_DOCS_URL))
                }
                Some(Control::RequestShortcut) => self.update(Message::OnboardingRequestShortcut),
                Some(Control::LoginStart) => {
                    let on = self.login_start_on();
                    self.update(Message::LoginStartToggled(!on))
                }
                Some(Control::GitHub) => {
                    self.update(Message::OnboardingOpen(onboarding::GITHUB_URL))
                }
                Some(Control::Install(index)) => self.update(Message::OnboardingInstall(index)),
                Some(Control::Back) => self.update(Message::OnboardingBack),
                Some(Control::Continue) => self.update(Message::OnboardingContinue),
                None if key.as_ref() == Key::Named(Named::Enter) => {
                    self.update(Message::OnboardingContinue)
                }
                None => Task::none(),
            },
            _ => Task::none(),
        }
    }

    /// Handles the flow's messages.
    pub(super) fn onboarding_message(&mut self, message: Message) -> Task<Message> {
        let Page::Onboarding(page) = &mut self.page else {
            return Task::none();
        };
        if matches!(
            message,
            Message::OnboardingContinue | Message::OnboardingBack | Message::OnboardingJump(_)
        ) {
            page.step_changed();
        }
        match message {
            Message::OnboardingContinue => match page.flow.advance() {
                Advance::Next => {
                    // What a step said belongs to that step.
                    page.notice = None;
                    self.onboarding_step_shown()
                }
                Advance::Finish => {
                    // Finish leaves the launcher open at its search, so the
                    // first thing after setting it up is using it.
                    record_completed(&page.state_path);
                    let ask = page.asks_on_finish();
                    self.page = Page::Root;
                    Task::batch([super::focus_search(), self.ask_for_hotkey_on_finish(ask)])
                }
            },
            Message::OnboardingBack => {
                page.notice = None;
                page.flow.back();
                self.onboarding_step_shown()
            }
            Message::OnboardingJump(position) => {
                page.notice = None;
                page.flow.jump(position);
                self.onboarding_step_shown()
            }
            Message::OnboardingRequestShortcut => {
                let Some(backend) = self.backend.clone() else {
                    page.notice = Some(crate::backend::SHORTCUT_NEEDS_ENGINE.to_owned());
                    return Task::none();
                };
                page.notice = None;
                page.shortcut = Some(LauncherShortcut::Pending);
                Task::perform(
                    async move { backend.launcher_shortcut(true).await },
                    Message::LauncherShortcutAnswered,
                )
            }
            Message::OnboardingTheme(ThemeOption(theme)) => {
                page.notice = None;
                page.menu = None;
                let preview = self.update(Message::ThemePreview(theme));
                let Some(backend) = self.backend.clone() else {
                    return preview;
                };
                let name = theme.name().to_owned();
                Task::batch([
                    preview,
                    Task::perform(
                        async move { backend.set_theme(name).await },
                        Message::ThemeSaved,
                    ),
                ])
            }
            Message::OnboardingOpen(url) => {
                // As `open_link` everywhere else: through the engine, which
                // starts the default browser, or a reason why not. The flow
                // stays up, so what happened is said on the card.
                let Some(backend) = self.backend.clone() else {
                    page.notice = Some(OPEN_NEEDS_ENGINE.to_owned());
                    return Task::none();
                };
                page.notice = None;
                Task::perform(
                    async move { backend.open_url(url.to_owned()).await },
                    Message::OnboardingLinkOpened,
                )
            }
            Message::OnboardingLinkOpened(result) => {
                page.notice = Some(match result {
                    Ok(()) => super::compass_commands::OPENED_IN_BROWSER.to_owned(),
                    Err(reason) => reason,
                });
                Task::none()
            }
            Message::OnboardingInstall(index) => {
                let Some(recommendation) = page.extensions.start(index) else {
                    return Task::none();
                };
                let Some(backend) = self.backend.clone() else {
                    page.extensions
                        .finished(index, Err(NEEDS_ENGINE.to_owned()));
                    return Task::none();
                };
                let (store, owner, name) = (
                    recommendation.store,
                    recommendation.owner.to_owned(),
                    recommendation.name.to_owned(),
                );
                Task::perform(
                    async move { backend.store_install(store, owner, name).await },
                    move |result| Message::OnboardingInstalled(index, result),
                )
            }
            Message::OnboardingInstalled(index, result) => {
                let installed = result.as_ref().ok().map(|(id, _)| id.clone());
                page.extensions.finished(index, result.map(|_| ()));
                // What the store's detail page does with an install: the
                // stores and root search learn of it.
                if let Some(id) = installed {
                    self.store_changed(&id, true);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// A step's box of rows: an Adwaita boxed list.
    fn onboarding_box<'a>(&self, rows: Element<'a, Message>) -> Element<'a, Message> {
        let palette = self.palette();
        container(rows)
            .padding(Padding::new(10.0).left(14).right(14))
            .width(Length::Fixed(480.0))
            .style(move |_: &Theme| adwaita::boxed_list(palette))
            .into()
    }

    /// Where a recommendation comes from, as a small pill after its title.
    fn store_badge<'a>(&self, store: compass_core::store_listing::Store) -> Element<'a, Message> {
        let palette = self.palette();
        container(
            text(store.name())
                .font(self.font())
                .size(11)
                .color(palette.muted.to_iced()),
        )
        .padding(Padding::new(1.0).left(6).right(6))
        .style(move |_: &Theme| adwaita::badge(palette))
        .into()
    }

    /// The flow's card: the step, then Back, the dots and Continue.
    pub(super) fn onboarding_body<'a>(&'a self, page: &'a OnboardingPage) -> Element<'a, Message> {
        let palette = self.palette();
        let step = page.flow.step();
        let heading = text(step.heading())
            .font(iced::Font {
                weight: iced::font::Weight::Semibold,
                ..self.font()
            })
            .size(20)
            .color(palette.text.to_iced());
        let granted = page
            .shortcut
            .as_ref()
            .is_some_and(LauncherShortcut::is_granted);
        let subtitle = text(step.subtitle(granted))
            .font(self.font())
            .size(14)
            .color(palette.muted.to_iced())
            .align_x(Alignment::Center);
        let small = |line: &'a str| {
            text(line)
                .font(self.font())
                .size(12)
                .color(palette.muted.to_iced())
        };
        // Adwaita's buttons: the one the step asks for in the accent, the
        // rest flat and neutral.
        let action = |label: &'a str, message: Message| {
            button(text(label).font(self.font()).size(13))
                .on_press(message)
                .padding(adwaita::CONTROL_PADDING)
                .style(move |_: &Theme, status| adwaita::button(palette, status))
        };
        let suggested = |label: &'a str, message: Message| {
            button(text(label).font(self.font()).size(13))
                .on_press(message)
                .padding(adwaita::CONTROL_PADDING)
                .style(move |_: &Theme, status| adwaita::suggested_button(palette, status))
        };
        let ring = |control: Control, element: Element<'a, Message>| -> Element<'a, Message> {
            adwaita::ringed(
                element,
                palette,
                page.focused == Some(control),
                adwaita::CONTROL_RADIUS,
            )
            .into()
        };

        let mut content = column![].spacing(8).align_x(Alignment::Center);
        if step == Step::Welcome {
            let logo = iced::widget::svg(iced::widget::svg::Handle::from_memory(COMPASS_LOGO))
                .width(72)
                .height(72);
            content = content.push(container(logo).padding(Padding::new(0.0).bottom(12)));
        }
        content = content.push(heading).push(subtitle);
        match step {
            Step::Personalize => {
                let current = ThemeOption(self.view.theme_choice);
                let theme_row = row![
                    column![
                        text("Theme").font(self.font()).size(14),
                        small("Shared across the entire app."),
                    ]
                    .width(Length::Fill),
                    ring(
                        Control::Theme,
                        pick_list(
                            page.themes.as_slice(),
                            Some(current),
                            Message::OnboardingTheme
                        )
                        .text_size(13)
                        .font(self.font())
                        .padding(adwaita::CONTROL_PADDING)
                        .style(move |_: &Theme, status| adwaita::dropdown(palette, status))
                        .menu_style(move |_: &Theme| crate::design::dropdown_menu(palette))
                        .width(Length::Fixed(200.0))
                        .into()
                    ),
                ]
                .align_y(Alignment::Center);
                let theme_row: Element<'a, Message> = match page.menu {
                    Some(highlighted) => {
                        let options = page
                            .themes
                            .iter()
                            .map(|theme| {
                                (
                                    theme.to_string(),
                                    *theme == current,
                                    Message::OnboardingTheme(*theme),
                                )
                            })
                            .collect();
                        column![
                            theme_row,
                            // A list of every theme file would push the
                            // step off the card: it scrolls past a few.
                            container(
                                crate::scroll::scrollable(self.choice_menu(options, highlighted))
                                    .id(crate::scroll::ONBOARDING_MENU)
                            )
                            .max_height(MENU_HEIGHT)
                            .padding(Padding::new(4.0).left(220))
                        ]
                        .into()
                    }
                    None => theme_row.into(),
                };
                // Where asking can help, the button asks the desktop: the
                // step's own action, so it takes the accent. Otherwise the
                // documentation says how to bind the key by hand.
                let hotkey_control = match page.shortcut.as_ref().filter(|_| page.offers_request())
                {
                    Some(state) => ring(
                        Control::RequestShortcut,
                        suggested(state.request_label(), Message::OnboardingRequestShortcut).into(),
                    ),
                    None => ring(
                        Control::OpenDocs,
                        action(
                            "Open Docs",
                            Message::OnboardingOpen(onboarding::HOTKEY_DOCS_URL),
                        )
                        .into(),
                    ),
                };
                let hotkey_row = row![
                    column![
                        text("Global hotkey").font(self.font()).size(14),
                        text(self.hotkey_guidance(page))
                            .font(self.font())
                            .size(12)
                            .color(palette.muted.to_iced()),
                    ]
                    .width(Length::Fill),
                    hotkey_control,
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                let login_row = row![
                    column![
                        text("Start at login").font(self.font()).size(14),
                        text(self.login_start_line(LOGIN_START_OFFER))
                            .font(self.font())
                            .size(12)
                            .color(palette.muted.to_iced()),
                    ]
                    .width(Length::Fill),
                    adwaita::ringed(
                        iced::widget::toggler(self.login_start_on())
                            .size(adwaita::SWITCH_SIZE)
                            .style(move |_: &Theme, status| adwaita::switch(palette, status))
                            .on_toggle_maybe(
                                (!self.login_start_busy).then_some(Message::LoginStartToggled)
                            ),
                        palette,
                        page.focused == Some(Control::LoginStart),
                        adwaita::SWITCH_SIZE / 2.0,
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                content =
                    content.push(self.onboarding_box(
                        column![theme_row, hotkey_row, login_row].spacing(14).into(),
                    ));
            }
            Step::Extensions => {
                let mut rows = column![].spacing(4);
                for (index, recommendation) in onboarding::RECOMMENDED_EXTENSIONS.iter().enumerate()
                {
                    let state = page
                        .extensions
                        .state(index)
                        .unwrap_or(onboarding::Install::Available);
                    // Install and Try Again are neutral, as every row has one;
                    // Installing and Installed are flat, with nothing to press.
                    let install = button(text(state.label()).font(self.font()).size(13))
                        .on_press_maybe(
                            state
                                .can_install()
                                .then_some(Message::OnboardingInstall(index)),
                        )
                        .padding(INSTALL_PADDING)
                        .style(move |_: &Theme, status| {
                            if state.can_install() {
                                adwaita::button(palette, status)
                            } else {
                                adwaita::flat_button(palette, status)
                            }
                        });
                    rows = rows.push(
                        row![
                            column![
                                row![
                                    text(recommendation.title).font(self.font()).size(14),
                                    self.store_badge(recommendation.store),
                                ]
                                .spacing(6)
                                .align_y(Alignment::Center),
                                small(recommendation.description),
                            ]
                            .width(Length::Fill),
                            ring(Control::Install(index), install.into()),
                        ]
                        .align_y(Alignment::Center),
                    );
                }
                // Inside the box and small: a store error can run to three
                // lines, and the card's height is fixed.
                if let Some(notice) = page.extensions.notice() {
                    rows = rows.push(
                        text(notice)
                            .font(self.font())
                            .size(12)
                            .color(palette.text.to_iced()),
                    );
                }
                content = content.push(self.onboarding_box(rows.into()));
            }
            Step::Complete => {
                content = content
                    .push(
                        container(
                            text(self.hotkey_guidance(page))
                                .font(self.font())
                                .size(13)
                                .color(palette.text.to_iced())
                                .align_x(Alignment::Center),
                        )
                        .max_width(480.0),
                    )
                    .push(Space::new().height(16))
                    .push(small("Compass is open source software."))
                    .push(
                        row![ring(
                            Control::GitHub,
                            action("GitHub", Message::OnboardingOpen(onboarding::GITHUB_URL))
                                .into()
                        )]
                        .spacing(8),
                    );
            }
            Step::Welcome | Step::Permissions => {}
        }
        if let Some(notice) = &page.notice {
            content = content.push(self.notice(notice));
        }

        // The step that is not this one in the control colour, 3:1 on the
        // card, rather than a faint text colour nobody can count.
        let accent = palette.accent.to_iced();
        let dim = palette.control().to_iced();
        let mut dots = row![].spacing(7);
        for position in 0..page.flow.count() {
            let color = if position == page.flow.position() {
                accent
            } else {
                dim
            };
            dots = dots.push(
                iced::widget::mouse_area(container(Space::new().width(7).height(7)).style(
                    move |_: &Theme| container::Style {
                        background: Some(color.into()),
                        border: Border {
                            radius: 3.5.into(),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    },
                ))
                .on_press(Message::OnboardingJump(position)),
            );
        }
        let back: Element<'a, Message> = if page.flow.can_go_back() {
            ring(
                Control::Back,
                action("Back", Message::OnboardingBack).into(),
            )
        } else {
            Space::new().into()
        };
        let footer = row![
            container(back).width(Length::Fill),
            dots,
            container(ring(
                Control::Continue,
                suggested(page.flow.primary_label(), Message::OnboardingContinue).into()
            ))
            .width(Length::Fill)
            .align_x(Alignment::End),
        ]
        .align_y(Alignment::Center)
        .padding(16);

        column![
            container(content)
                .width(Length::Fill)
                .height(Length::Fixed(400.0))
                .center_x(Length::Fill)
                .center_y(Length::Fixed(400.0)),
            footer,
        ]
        .width(Length::Fill)
        .into()
    }
}
