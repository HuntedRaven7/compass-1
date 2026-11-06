//! Calculator History in the launcher, and remembering a copied answer.
//!
//! Copying the calculator's answer — from the root list or from this view's
//! live result — remembers it, as the C++ `CopyCalculatorAnswerAction`
//! does; the history's rows offer the C++ panel: pin or unpin, the three
//! copies with the answer primary, then remove and remove all.

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
    text,
};
use crate::action_panel::Action;
use crate::backend::CalculatorChange;
use crate::calculator_page::{CalcRow, CalculatorPage};

const PIN: &str = "calc.pin";
const UNPIN: &str = "calc.unpin";
const COPY_ANSWER: &str = "calc.copy-answer";
const COPY_QUESTION: &str = "calc.copy-question";
const COPY_EXPRESSION: &str = "calc.copy-expression";
const REMOVE: &str = "calc.remove";
const REMOVE_ALL: &str = "calc.remove-all";

/// The prefix of the root answer's panel actions, before
/// `compass_core::calculator_history::live_action_panel`'s ids.
const ROOT: &str = "calc-root.";

const NEEDS_ENGINE: &str =
    "Calculator History needs the Compass engine, and this window is running without one";

/// The calculator's own HUD, as `CopyCalculatorAnswerAction`.
#[must_use]
pub fn answer_copied() -> crate::hud::Hud {
    crate::hud::Hud::new("Answer copied to clipboard").with_icon(crate::hud::COPY_ICON)
}

/// Refresh Exchange Rates' HUD while it fetches, as the C++'s dynamic
/// "Refreshing rates..." toast.
#[must_use]
pub fn refreshing_rates() -> crate::hud::Hud {
    crate::hud::Hud::new("Refreshing rates...").with_icon(RATES_ICON)
}

/// Refresh Exchange Rates' HUD on success, as the C++'s success toast.
#[must_use]
pub fn rates_refreshed() -> crate::hud::Hud {
    crate::hud::Hud::new("Rates successfully refreshed").with_icon(RATES_ICON)
}

/// Refresh Exchange Rates' HUD on failure: the C++'s failure toast carries
/// the error alone.
#[must_use]
pub fn rates_not_refreshed(reason: &str) -> crate::hud::Hud {
    crate::hud::Hud::new(reason.to_owned()).with_icon("warning")
}

const RATES_ICON: &str = "globe-01";

/// How long after asking the engine for rates this window waits before
/// asking again, when what came back was none or stale (offline, say).
const RATES_ASK_AGAIN: std::time::Duration = std::time::Duration::from_secs(60);

/// Makes `rates` the ones the calculator converts currencies with, and
/// says whether that changed anything. An engine never gives rates up once
/// it has some, so "none" changes nothing.
fn install_rates(rates: compass_core::exchange_rates::ExchangeRates) -> bool {
    let changed =
        compass_core::calculator::exchange_rates().is_none_or(|current| *current != rates);
    if changed {
        compass_core::calculator::set_exchange_rates(Some(std::sync::Arc::new(rates)));
    }
    changed
}

/// Seconds since the epoch, as the rates' `fetched_at`.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

impl LauncherApp {
    /// Asks the engine for exchange rates when an evaluation since the last
    /// update wanted one and the rates in hand are missing or stale. The
    /// engine fetches them then, so rates cost nothing until a currency
    /// expression is typed; their answer re-evaluates what is shown.
    pub(super) fn rates_on_demand(&mut self) -> Option<Task<Message>> {
        if !compass_core::calculator::take_rates_wanted() {
            return None;
        }
        let current = compass_core::calculator::exchange_rates()
            .is_some_and(|rates| !rates.is_stale(now_secs()));
        let asked_lately = self
            .rates_asked
            .is_some_and(|at| at.elapsed() < RATES_ASK_AGAIN);
        if current || asked_lately || self.backend.is_none() {
            return None;
        }
        self.rates_asked = Some(std::time::Instant::now());
        Some(self.exchange_rates_task())
    }

    /// Asks the engine for its exchange rates, which it fetches first when
    /// it has none or they are a day old.
    pub(super) fn exchange_rates_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.exchange_rates().await },
            Message::ExchangeRatesLoaded,
        )
    }

    /// Evaluates again what the calculator shows, now that the rates moved.
    fn rates_changed(&mut self) {
        match &mut self.page {
            Page::Root => self.apply_calculator(),
            Page::Calculator(page) => page.recompute_live(),
            _ => {}
        }
    }

    /// Refresh Exchange Rates, as `CalculatorRefreshRatesCommand`: the
    /// engine fetches them, and the HUD says how it went.
    pub(super) fn refresh_exchange_rates(
        &mut self,
        command: &'static compass_core::commands::BuiltinCommand,
    ) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            self.error = Some(format!(
                "{} needs the Compass engine, and this window is running without one",
                command.title
            ));
            return Task::none();
        };
        let refresh = Task::perform(
            async move { backend.refresh_exchange_rates().await },
            Message::ExchangeRatesRefreshed,
        );
        Task::batch([self.show_hud(refreshing_rates()), refresh])
    }

    /// Opens Calculator History and asks for the rows.
    pub(super) fn open_calculator_history(&mut self) -> Task<Message> {
        self.page = Page::Calculator(CalculatorPage::default());
        Task::batch([self.calculator_query_task(), focus_search()])
    }

    fn calculator_query_task(&mut self) -> Task<Message> {
        let Page::Calculator(page) = &mut self.page else {
            return Task::none();
        };
        let Some(backend) = self.backend.clone() else {
            let generation = page.generation;
            page.apply(generation, Err(NEEDS_ENGINE.to_owned()));
            return Task::none();
        };
        let (query, generation) = (page.query.clone(), page.generation);
        Task::perform(
            async move { backend.calculator_history(query).await },
            move |result| Message::CalculatorLoaded { generation, result },
        )
    }

    /// Copies `answer`, remembering the calculation in the history.
    pub(super) fn copy_calculation(
        &self,
        question: String,
        answer: String,
        copied: String,
    ) -> Task<Message> {
        let copy = super::hud::copy_text(self.backend.clone(), copied);
        let Some(backend) = self.backend.clone() else {
            return copy;
        };
        let conversion = compass_core::calculator::is_conversion(&question);
        let remember = Task::future(async move {
            if let Err(error) = backend
                .add_calculator_record(question, answer, conversion)
                .await
            {
                tracing::warn!(%error, "could not remember a calculation");
            }
        })
        .discard();
        Task::batch([copy, remember])
    }

    fn edit_calculator(&mut self, change: CalculatorChange, said: &'static str) -> Task<Message> {
        self.panel = None;
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::batch([
            Task::perform(
                async move { backend.edit_calculator_history(change).await },
                move |result| Message::CalculatorEdited(result.map(|()| said)),
            ),
            focus_search(),
        ])
    }

    /// Copies the root answer, remembering it, and says so (the C++
    /// `CopyCalculatorAnswerAction`, the row's primary action).
    pub(super) fn copy_root_answer(&mut self) -> Task<Message> {
        let Some(answer) = &self.calculator else {
            return Task::none();
        };
        let (question, answer) = (answer.question.clone(), answer.answer.clone());
        let copied = compass_core::calculator::copied_value(&answer);
        let copy = self.copy_calculation(question, answer, copied);
        Task::batch([copy, self.show_hud(answer_copied())])
    }

    /// The panel over the root search's answer
    /// (`RootCalculatorSection::actionPanel`).
    pub(super) fn open_root_calculator_panel(&mut self) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root)
            || self.selected_row() != Some(super::RootRow::Calculator)
        {
            return None;
        }
        self.calculator.as_ref()?;
        // fend prints its answers unformatted, so there is no second form.
        let sections = compass_core::calculator_history::live_action_panel(false)
            .into_iter()
            .map(|actions| PanelSection {
                name: String::new(),
                actions: actions
                    .into_iter()
                    .map(|action| {
                        let item = Action::new(action.title.unwrap_or(action.id))
                            .with_id(format!("{ROOT}{}", action.id));
                        if action.primary {
                            item.with_shortcut("enter")
                        } else {
                            item
                        }
                    })
                    .collect(),
            })
            .collect();
        self.panel = Some(PanelState::new(sections));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs an action of the root answer's panel, if `id` is one.
    pub(super) fn root_calculator_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let action = id.strip_prefix(ROOT)?;
        let answer = self.calculator.clone()?;
        self.panel = None;
        Some(match action {
            "copy-answer" => self.copy_root_answer(),
            "copy-question-and-answer" => {
                let expression = compass_core::calculator_history::live_result_title(
                    &answer.question,
                    &answer.answer,
                );
                let copy = self.copy_calculation(answer.question, answer.answer, expression);
                Task::batch([copy, self.show_hud(answer_copied())])
            }
            "put-answer-in-search-bar" => {
                let typed = self.update(Message::QueryChanged(answer.answer));
                Task::batch([typed, focus_search()])
            }
            "open-history" => self.open_calculator_history(),
            _ => Task::none(),
        })
    }

    /// Runs the selected row's primary action: copy the answer.
    fn copy_selected_calculation(&mut self) -> Task<Message> {
        let Page::Calculator(page) = &self.page else {
            return Task::none();
        };
        match page.selected_row() {
            Some(CalcRow::Live(answer)) => {
                let (question, answer) = (answer.question.clone(), answer.answer.clone());
                let copied = compass_core::calculator::copied_value(&answer);
                let copy = self.copy_calculation(question, answer, copied);
                Task::batch([copy, self.show_hud(answer_copied())])
            }
            Some(CalcRow::Record(record)) => {
                let answer = compass_core::calculator::copied_value(&record.answer);
                self.copy_with_hud(answer)
            }
            None => Task::none(),
        }
    }

    /// The panel over the selected row.
    pub(super) fn open_calculator_panel(&mut self) -> Option<Task<Message>> {
        let Page::Calculator(page) = &self.page else {
            return None;
        };
        let sections = match page.selected_row()? {
            CalcRow::Live(_) => vec![PanelSection {
                name: String::new(),
                actions: vec![
                    Action::new("Copy Result")
                        .with_id(COPY_ANSWER)
                        .with_shortcut("enter"),
                    Action::new("Copy Question And Answer").with_id(COPY_EXPRESSION),
                ],
            }],
            CalcRow::Record(record) => vec![
                PanelSection {
                    name: String::new(),
                    actions: vec![if record.pinned {
                        Action::new("Unpin entry").with_id(UNPIN)
                    } else {
                        Action::new("Pin entry").with_id(PIN)
                    }],
                },
                PanelSection {
                    name: String::new(),
                    actions: vec![
                        Action::new("Copy answer")
                            .with_id(COPY_ANSWER)
                            .with_shortcut("enter"),
                        Action::new("Copy question").with_id(COPY_QUESTION),
                        Action::new("Copy question and answer").with_id(COPY_EXPRESSION),
                    ],
                },
                PanelSection {
                    name: String::new(),
                    actions: vec![
                        Action::new("Delete entry").with_id(REMOVE),
                        Action::new("Delete all entries").with_id(REMOVE_ALL),
                    ],
                },
            ],
        };
        self.panel = Some(PanelState::new(sections));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a Calculator History panel action, if `id` is one.
    pub(super) fn calculator_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let Page::Calculator(page) = &self.page else {
            return None;
        };
        if !id.starts_with("calc.") {
            return None;
        }
        let row = page.selected_row()?;
        let (question, answer, record_id) = match row {
            CalcRow::Live(answer) => (answer.question.clone(), answer.answer.clone(), None),
            CalcRow::Record(record) => (
                record.question.clone(),
                record.answer.clone(),
                Some(record.id.clone()),
            ),
        };
        let expression = compass_core::calculator_history::live_result_title(&question, &answer);
        self.panel = None;
        Some(match (id, record_id) {
            (COPY_ANSWER, _) => return Some(self.copy_selected_calculation()),
            (COPY_EXPRESSION, None) => {
                let copy = self.copy_calculation(question, answer, expression);
                Task::batch([copy, self.show_hud(answer_copied())])
            }
            (COPY_EXPRESSION, Some(_)) => self.copy_with_hud(expression),
            (COPY_QUESTION, Some(_)) => self.copy_with_hud(question),
            (PIN, Some(id)) => self.edit_calculator(CalculatorChange::Pin(id), "Entry pinned"),
            (UNPIN, Some(id)) => {
                self.edit_calculator(CalculatorChange::Unpin(id), "Entry unpinned")
            }
            (REMOVE, Some(id)) => {
                self.edit_calculator(CalculatorChange::Remove(id), "Entry removed")
            }
            (REMOVE_ALL, Some(_)) => {
                self.edit_calculator(CalculatorChange::RemoveAll, "All entries removed")
            }
            _ => Task::none(),
        })
    }

    /// The view's keys.
    pub(super) fn calculator_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Calculator(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.copy_selected_calculation(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected =
                next_selection(page.len(), page.selected, direction, self.wrap_navigation);
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Handles the view's messages.
    pub(super) fn calculator_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::CalculatorQueryChanged(query) => {
                if let Page::Calculator(page) = &mut self.page {
                    page.notice = None;
                    page.set_query(query);
                }
                self.calculator_query_task()
            }
            Message::CalculatorLoaded { generation, result } => {
                if let Page::Calculator(page) = &mut self.page {
                    page.apply(generation, result);
                }
                crate::scroll::reveal_root_selection()
            }
            Message::CalculatorSelected(position) => {
                if let Page::Calculator(page) = &mut self.page
                    && position < page.len()
                {
                    page.selected = position;
                }
                self.copy_selected_calculation()
            }
            Message::ExchangeRatesLoaded(result) => {
                match result {
                    Ok(Some(rates)) => {
                        if install_rates(rates) {
                            self.rates_changed();
                        }
                    }
                    Ok(None) => {}
                    Err(error) => tracing::debug!(%error, "no exchange rates from the engine"),
                }
                Task::none()
            }
            Message::ExchangeRatesRefreshed(result) => {
                let hud = match result {
                    Ok(rates) => {
                        if install_rates(rates) {
                            self.rates_changed();
                        }
                        rates_refreshed()
                    }
                    Err(reason) => {
                        let hud = rates_not_refreshed(&reason);
                        self.error = Some(reason);
                        hud
                    }
                };
                self.show_hud(hud)
            }
            Message::CalculatorEdited(result) => {
                if let Page::Calculator(page) = &mut self.page {
                    page.notice = Some(match result {
                        Ok(said) => said.to_owned(),
                        Err(reason) => reason,
                    });
                    page.generation += 1;
                }
                self.calculator_query_task()
            }
            _ => Task::none(),
        }
    }

    /// The view's body.
    pub(super) fn calculator_body<'a>(&'a self, page: &'a CalculatorPage) -> Element<'a, Message> {
        if let Some(empty) = page.empty_notice() {
            return self.notice(empty);
        }
        let heading = |name: String| -> Element<'a, Message> {
            container(text(name).font(self.font()).size(12))
                .padding(Padding::new(4.0).left(10))
                .into()
        };
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        let mut position = 0;
        let push_row = |list: iced::widget::Column<'a, Message>,
                        badge: &str,
                        title: String,
                        subtitle: String,
                        position: usize| {
            let selected = position == page.selected;
            let item = self.list_row(
                self.initial_badge(badge, selected),
                title,
                self.subtitles.then_some(subtitle),
                selected,
            );
            let item: Element<Message> = mouse_area(item)
                .on_press(Message::CalculatorSelected(position))
                .into();
            let item: Element<Message> = if selected {
                container(item).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                item
            };
            list.push(item)
        };
        if let Some(live) = &page.live {
            list = list.push(heading("Calculator".to_owned()));
            let (title, subtitle) = CalcRow::Live(live).text();
            list = push_row(list, "=", title, subtitle, position);
            position += 1;
        }
        for group in &page.groups {
            list = list.push(heading(group.name.clone()));
            for record in &group.records {
                let badge = if record.conversion { "⇄" } else { "=" };
                let (title, subtitle) = CalcRow::Record(record).text();
                list = push_row(list, badge, title, subtitle, position);
                position += 1;
            }
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// The history, for assistive technology: the live result first, then
    /// each group's calculations, as the selection moves over them.
    pub(super) fn calculator_semantics(&self, page: &CalculatorPage) -> super::a11y::PageSemantics {
        let items = page
            .rows()
            .iter()
            .map(|row| {
                let (title, subtitle) = row.text();
                super::a11y::item(title, Some(subtitle))
            })
            .collect();
        super::a11y::PageSemantics::list(
            "Calculator history",
            items,
            page.selected,
            page.empty_notice(),
        )
        .with_status(page.notice.as_deref(), &page.query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RootRow;
    use crate::app::test_support::{
        TestBackend, choose, index, open_builtin, pressed, settle, with_resident_hud,
    };
    use crate::backend::{CalculatorGroupRow, CalculatorRow};
    use crate::resident::UiCommand;
    use crate::semantics::Body;
    use std::sync::Arc;

    #[test]
    fn the_history_lists_the_live_result_then_each_calculation() {
        let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().build());
        let mut page = CalculatorPage::default();
        assert_eq!(
            app.calculator_semantics(&page).status.as_deref(),
            Some("Reading the calculator history…")
        );
        page.set_query("2+2".into());
        page.apply(
            page.generation,
            Ok(vec![CalculatorGroupRow {
                name: "Today".into(),
                records: vec![CalculatorRow {
                    id: "a".into(),
                    question: "3*3".into(),
                    answer: "9".into(),
                    conversion: false,
                    pinned: false,
                }],
            }]),
        );
        page.selected = 1;
        let live = page.live.clone().expect("2+2 computes");
        app.page = Page::Calculator(page);
        let semantics = app.semantics();
        let Body::List(list) = &semantics.body else {
            panic!("expected a list, got {:?}", semantics.body);
        };
        assert_eq!(list.label, "Calculator history");
        assert_eq!(list.items.len(), 2);
        assert_eq!(
            list.items[0].name,
            compass_core::calculator_history::live_result_title(&live.question, &live.answer)
        );
        assert_eq!(
            list.items[0].description.as_deref(),
            Some("Copy to keep it")
        );
        assert_eq!(list.items[1].name, "3*3");
        assert_eq!(list.items[1].description.as_deref(), Some("9"));
        assert_eq!(list.selected, Some(1));
    }

    #[test]
    fn a_copied_answer_is_remembered_and_calculator_history_lists_pins_and_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(TestBackend::default());
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(backend.clone());

        // Copying the root list's answer keeps the calculation.
        app.query = "5 ft to m".into();
        app.search();
        assert_eq!(app.selected_row(), Some(RootRow::Calculator));
        let task = app.update(Message::LaunchSelected);
        let writes = settle(&mut app, task);
        assert_eq!(
            writes,
            ["1.524"],
            "the value, as the C++ copies answer.text"
        );
        let kept = backend.calculations.lock().unwrap().clone();
        assert_eq!(kept.len(), 1);
        assert_eq!(
            (kept[0].question.as_str(), kept[0].conversion),
            ("5 ft to m", true)
        );

        let _ = app.update(Message::Command(UiCommand::Show));
        app.query = "calculator history".into();
        app.search();
        let task = app.update(Message::LaunchSelected);
        settle(&mut app, task);
        let Page::Calculator(page) = &app.page else {
            panic!("not on Calculator History: {}", app.state_line());
        };
        assert_eq!(page.groups.len(), 1);
        assert_eq!(page.groups[0].records[0].answer, "1.524 m");

        // The live result for what is typed leads, and copying it keeps it.
        let task = app.update(Message::CalculatorQueryChanged("=6*7".into()));
        settle(&mut app, task);
        let Page::Calculator(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.live.as_ref().map(|a| a.answer.as_str()), Some("42"));
        let task = app.update(pressed(iced::keyboard::key::Named::Enter));
        let writes = settle(&mut app, task);
        assert_eq!(writes, ["42"]);
        assert_eq!(backend.calculations.lock().unwrap().len(), 2);

        // The panel pins and removes a remembered row.
        let _ = app.update(Message::Command(UiCommand::Show));
        let task = app.open_calculator_history();
        settle(&mut app, task);
        let _ = app.update(Message::TogglePanel);
        let task = app.update(Message::PanelFilterChanged("Pin entry".into()));
        settle(&mut app, task);
        let task = app.update(Message::PanelActivate);
        settle(&mut app, task);
        let _ = app.update(Message::TogglePanel);
        let _ = app.update(Message::PanelFilterChanged("Delete entry".into()));
        let task = app.update(Message::PanelActivate);
        settle(&mut app, task);
        assert_eq!(
            backend.calculator_edits.lock().unwrap().as_slice(),
            [
                crate::backend::CalculatorChange::Pin("r1".into()),
                crate::backend::CalculatorChange::Remove("r1".into()),
            ]
        );
        let Page::Calculator(page) = &app.page else {
            unreachable!()
        };
        assert_eq!(page.notice.as_deref(), Some("Entry removed"));
        assert_eq!(page.len(), 1, "the list reloaded without it");
    }

    #[test]
    fn refresh_exchange_rates_installs_the_fresh_rates_and_says_so_or_why_not() {
        let dir = tempfile::tempdir().unwrap();
        let rates = compass_core::exchange_rates::ExchangeRates {
            date: "2026-09-24".into(),
            fetched_at: 7,
            rates: [("EUR".to_owned(), 1.0), ("USD".to_owned(), 1.25)].into(),
        };
        let backend = Arc::new(TestBackend {
            rates: Some(rates.clone()),
            ..TestBackend::default()
        });
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(backend);
        open_builtin(&mut app, "refresh exchange rates", "commands:refresh-rates");
        let installed = compass_core::calculator::exchange_rates();
        let answer = compass_core::calculator::compute("10 usd to eur").map(|a| a.answer);
        compass_core::calculator::set_exchange_rates(None);
        assert_eq!(installed.as_deref(), Some(&rates));
        assert_eq!(answer.as_deref(), Some("8 EUR"));
        assert_eq!(
            app.hud_content(),
            Some(&crate::app::calculator::rates_refreshed())
        );

        let failing = Arc::new(TestBackend {
            refuse_rates: Some("Could not refresh the exchange rates: offline".into()),
            ..TestBackend::default()
        });
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(failing);
        open_builtin(&mut app, "refresh exchange rates", "commands:refresh-rates");
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Could not refresh the exchange rates: offline")
        );
        assert_eq!(
            app.error.as_deref(),
            Some("Could not refresh the exchange rates: offline")
        );
    }

    #[test]
    fn rates_are_asked_for_once_only_when_a_currency_expression_wants_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = LauncherApp::with_index(index(dir.path()));
        app.backend = Some(Arc::new(TestBackend::default()));
        let _ = compass_core::calculator::take_rates_wanted();

        let _ = compass_core::calculator::compute("2 + 2");
        assert!(app.rates_on_demand().is_none(), "arithmetic wants no rates");

        // No test installs rates fresher than a day, so these are missing or stale.
        let _ = compass_core::calculator::compute("10 usd to eur");
        assert!(
            app.rates_on_demand().is_some(),
            "a currency expression asks"
        );
        let _ = compass_core::calculator::compute("20 usd to eur");
        assert!(
            app.rates_on_demand().is_none(),
            "the next keystroke does not ask again"
        );
    }

    /// Ctrl+B over the answer offers upstream's panel, and each action does
    /// what it says: the copies copy and remember, the search bar takes the
    /// answer, and the history opens.
    #[test]
    fn the_calculator_answers_panel_copies_types_and_opens_the_history() {
        let dir = tempfile::tempdir().unwrap();
        let answer = |app: &mut LauncherApp| {
            app.page = Page::Root;
            app.query = "6*7".into();
            app.search();
            assert_eq!(app.selected_row(), Some(RootRow::Calculator));
            let _ = app.update(Message::TogglePanel);
        };
        let backend = Arc::new(TestBackend::default());
        let mut app = with_resident_hud(LauncherApp::with_index(index(dir.path())));
        app.backend = Some(backend.clone());
        answer(&mut app);
        let titles: Vec<String> = app
            .panel
            .as_ref()
            .expect("a panel over the answer")
            .sections
            .iter()
            .flat_map(|section| section.actions.iter().map(|a| a.title.clone()))
            .collect();
        assert_eq!(
            titles,
            [
                "Copy Result",
                "Copy Question And Answer",
                "Put answer in search bar",
                "Open Calculator History",
            ]
        );

        let task = choose(&mut app, "Copy Result");
        assert_eq!(settle(&mut app, task), ["42"]);
        assert_eq!(
            app.hud_content().map(|hud| hud.text.as_str()),
            Some("Answer copied to clipboard")
        );

        answer(&mut app);
        let task = choose(&mut app, "Copy Question And Answer");
        assert_eq!(settle(&mut app, task), ["6*7 = 42"]);
        let remembered: Vec<(String, String)> = backend
            .calculations
            .lock()
            .unwrap()
            .iter()
            .map(|row| (row.question.clone(), row.answer.clone()))
            .collect();
        assert_eq!(
            remembered,
            [("6*7".into(), "42".into()), ("6*7".into(), "42".into())],
            "both copies are remembered, as CopyCalculatorAnswerAction's are"
        );

        answer(&mut app);
        let task = choose(&mut app, "Put answer in search bar");
        settle(&mut app, task);
        assert_eq!(app.query, "42");
        assert!(app.panel.is_none());
        assert!(matches!(app.page, Page::Root));

        answer(&mut app);
        let task = choose(&mut app, "Open Calculator History");
        settle(&mut app, task);
        assert!(matches!(app.page, Page::Calculator(_)));
    }
}
