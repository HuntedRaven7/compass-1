//! Calculator History: past calculations grouped by when their answer was
//! copied, with a live result for what is typed, as the C++
//! `CalcHistoryViewHost`.
//!
//! The engine keeps, filters and groups the rows; this holds them, the live
//! result, and the selection over both.

use crate::backend::{CalculatorGroupRow, CalculatorRow};

/// The search field's placeholder.
pub const PLACEHOLDER: &str = "Search past calculations...";

/// What the view says with nothing remembered.
pub const EMPTY: &str = "No calculations yet: copy an answer to keep it here";

/// One selectable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalcRow<'a> {
    /// The live result for what is typed.
    Live(&'a compass_core::calculator::Answer),
    /// A remembered calculation.
    Record(&'a CalculatorRow),
}

impl CalcRow<'_> {
    /// The row's title and second line, as the view draws them.
    #[must_use]
    pub fn text(&self) -> (String, String) {
        match self {
            Self::Live(live) => (
                compass_core::calculator_history::live_result_title(&live.question, &live.answer),
                "Copy to keep it".to_owned(),
            ),
            Self::Record(record) => (record.question.clone(), record.answer.clone()),
        }
    }
}

/// The view's state.
#[derive(Debug, Clone, Default)]
pub struct CalculatorPage {
    /// The filter text.
    pub query: String,
    /// The groups the engine answered with.
    pub groups: Vec<CalculatorGroupRow>,
    /// The live result for `query`, if it computes.
    pub live: Option<compass_core::calculator::Answer>,
    /// Position over every row, the live one first.
    pub selected: usize,
    /// Why there are no rows, when the engine said.
    pub failure: Option<String>,
    /// Whether the first answer has arrived.
    pub loaded: bool,
    /// What the last action said.
    pub notice: Option<String>,
    /// The request whose answer is current.
    pub generation: u64,
}

impl CalculatorPage {
    /// Sets the query and its live result, by the C++ rule
    /// (`live_calc`: three characters, or a leading `=`).
    pub fn set_query(&mut self, query: String) {
        self.live = match compass_core::calculator_history::live_calc(&query) {
            compass_core::calculator_history::LiveCalc::Compute(question) => {
                compass_core::calculator::compute(&question)
            }
            compass_core::calculator_history::LiveCalc::None => None,
        };
        self.query = query;
        self.selected = 0;
        self.generation += 1;
    }

    /// Takes the engine's answer for `generation`, keeping the selection's
    /// position; a stale one is dropped.
    pub fn apply(&mut self, generation: u64, result: Result<Vec<CalculatorGroupRow>, String>) {
        if generation != self.generation {
            return;
        }
        self.loaded = true;
        match result {
            Ok(groups) => {
                self.groups = groups;
                self.failure = None;
            }
            Err(reason) => {
                self.groups.clear();
                self.failure = Some(reason);
            }
        }
        self.selected = self.selected.min(self.len().saturating_sub(1));
    }

    /// Every row, in order.
    #[must_use]
    pub fn rows(&self) -> Vec<CalcRow<'_>> {
        self.live
            .iter()
            .map(CalcRow::Live)
            .chain(
                self.groups
                    .iter()
                    .flat_map(|group| group.records.iter().map(CalcRow::Record)),
            )
            .collect()
    }

    /// How many rows there are.
    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.live.is_some())
            + self
                .groups
                .iter()
                .map(|group| group.records.len())
                .sum::<usize>()
    }

    /// Whether there is nothing to select.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The selected row.
    #[must_use]
    pub fn selected_row(&self) -> Option<CalcRow<'_>> {
        self.rows().into_iter().nth(self.selected)
    }

    /// What the view says in place of the rows: why there are none, that
    /// the history is being read, that nothing is remembered, or that
    /// nothing matches; `None` while there are rows.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        if !self.is_empty() {
            return None;
        }
        Some(match (&self.failure, self.loaded) {
            (Some(reason), _) => reason,
            (None, false) => "Reading the calculator history…",
            (None, true) if self.query.is_empty() => EMPTY,
            (None, true) => "No calculations match",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> CalculatorRow {
        CalculatorRow {
            id: id.into(),
            question: format!("q{id}"),
            answer: format!("a{id}"),
            conversion: false,
            pinned: false,
        }
    }

    #[test]
    fn the_live_result_leads_and_a_stale_answer_is_dropped() {
        let mut page = CalculatorPage::default();
        page.set_query("2+2".into());
        assert_eq!(page.live.as_ref().map(|a| a.answer.as_str()), Some("4"));
        let first = page.generation;
        page.set_query("=1+1".into());
        page.apply(
            first,
            Ok(vec![CalculatorGroupRow {
                name: "Today".into(),
                records: vec![record("x")],
            }]),
        );
        assert!(page.groups.is_empty(), "stale");
        page.apply(
            page.generation,
            Ok(vec![CalculatorGroupRow {
                name: "Today".into(),
                records: vec![record("a"), record("b")],
            }]),
        );
        assert_eq!(page.len(), 3);
        assert!(matches!(page.selected_row(), Some(CalcRow::Live(_))));
        page.selected = 2;
        assert!(matches!(page.selected_row(), Some(CalcRow::Record(r)) if r.id == "b"));

        page.set_query("ab".into());
        assert!(page.live.is_none(), "under three characters");
    }
}
