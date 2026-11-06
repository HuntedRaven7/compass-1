//! The window switcher view: its state, and what it decides.
//!
//! The engine sends every window once; filtering happens here, on each
//! keystroke, with the same weighted fuzzy scoring the root list uses and the
//! weights `compass_core::window_switcher` chose (title over application over
//! `WM_CLASS`).

use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

use crate::backend::WindowRow;

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The list is on its way.
    Loading,
    /// Windows have arrived (possibly none).
    Ready,
    /// Windows cannot be listed, and why.
    Failed(String),
}

/// The window switcher's state.
#[derive(Debug, Clone)]
pub struct WindowsPage {
    /// The filter text.
    pub query: String,
    /// Every window the engine reported, less the launcher's own.
    pub all: Vec<WindowRow>,
    /// Positions in `all` that match `query`, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Why the last action did not happen.
    pub notice: Option<String>,
}

impl Default for WindowsPage {
    fn default() -> Self {
        Self {
            query: String::new(),
            all: Vec::new(),
            shown: Vec::new(),
            selected: 0,
            status: Status::Loading,
            notice: None,
        }
    }
}

impl WindowsPage {
    /// Takes the engine's answer, leaving out the launcher's own window: owned
    /// by `own_pid`, or carrying [`crate::APP_ID`] as its class.
    pub fn apply(&mut self, result: Result<Vec<WindowRow>, String>, own_pid: u32) {
        match result {
            Ok(rows) => {
                self.all = rows
                    .into_iter()
                    .filter(|row| {
                        row.pid != Some(own_pid)
                            && !row.wm_class.eq_ignore_ascii_case(crate::APP_ID)
                    })
                    .collect();
                self.status = Status::Ready;
            }
            Err(reason) => {
                self.all.clear();
                self.status = Status::Failed(reason);
            }
        }
        self.refilter();
    }

    /// Recomputes `shown` for the current query. An empty query shows every
    /// window in the engine's order.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let query = Query::new(&self.query);
        if query.is_empty() {
            self.shown = (0..self.all.len()).collect();
            return;
        }
        let mut scored: Vec<(u32, usize)> = self
            .all
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let fields = [
                    WeightedField::new(&row.title, 1.0),
                    WeightedField::new(&row.app, 0.5),
                    WeightedField::new(&row.wm_class, 0.3),
                ];
                let found = score_weighted(&fields, &query);
                (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, index))
            })
            .collect();
        // Stable on ties, so equal matches keep the engine's order.
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.shown = scored.into_iter().map(|(_, index)| index).collect();
    }

    /// The selected window, if any.
    #[must_use]
    pub fn selected_row(&self) -> Option<&WindowRow> {
        self.all.get(*self.shown.get(self.selected)?)
    }

    /// What the view says in place of the list: loading, why it failed, or
    /// that there is nothing to show.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Loading windows…"),
            Status::Failed(reason) => Some(reason.as_str()),
            Status::Ready if self.all.is_empty() => Some("No other windows are open"),
            Status::Ready if self.shown.is_empty() => Some("No matching windows"),
            Status::Ready => None,
        }
    }
}

/// What a window's row is titled: its title, or its application's name when
/// it has none.
#[must_use]
pub fn title(row: &WindowRow) -> &str {
    if row.title.is_empty() {
        &row.app
    } else {
        &row.title
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u32, title: &str, app: &str, pid: u32) -> WindowRow {
        WindowRow {
            id,
            title: title.into(),
            app: app.into(),
            wm_class: app.to_lowercase(),
            pid: Some(pid),
            can_close: true,
            app_known: true,
        }
    }

    fn page() -> WindowsPage {
        let mut page = WindowsPage::default();
        page.apply(
            Ok(vec![
                row(1, "Downloads", "Files", 10),
                row(2, "Inbox — Mail", "Evolution", 11),
                row(3, "Compass", "Compass", 99),
                row(4, "notes.txt", "Text Editor", 12),
            ]),
            99,
        );
        page
    }

    #[test]
    fn the_launcher_leaves_out_its_own_window() {
        let page = page();
        assert_eq!(page.all.len(), 3);
        assert!(page.all.iter().all(|row| row.pid != Some(99)));
        assert_eq!(page.selected_row().map(|r| r.id), Some(1));
    }

    #[test]
    fn typing_finds_a_window_by_title_or_application() {
        let mut page = page();
        page.query = "inbox".into();
        page.refilter();
        assert_eq!(page.selected_row().map(|r| r.id), Some(2));

        page.query = "editor".into();
        page.refilter();
        assert_eq!(
            page.selected_row().map(|r| r.id),
            Some(4),
            "by application name"
        );

        page.query = "zzzz".into();
        page.refilter();
        assert!(page.shown.is_empty());
        assert!(page.selected_row().is_none());
    }

    #[test]
    fn a_failure_says_why_and_shows_nothing() {
        let mut page = page();
        page.apply(Err("needs the extension".into()), 99);
        assert!(page.shown.is_empty());
        assert_eq!(page.status, Status::Failed("needs the extension".into()));
    }
}
