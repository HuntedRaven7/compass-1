//! Switch Workspaces: its state, and what it decides.
//!
//! The engine sends every workspace once; filtering happens here with the
//! weights `SwitchWorkspacesSection` scores by (the name, then the monitor,
//! then the applications on it), and each row says what
//! `compass_core::window_switcher` has it say.

use compass_core::window_switcher::{WorkspaceEntry, workspace_subtitle};
use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

use crate::backend::WorkspaceRow;

/// The search field's placeholder.
pub const PLACEHOLDER: &str = "Search workspaces...";

/// The list's heading.
pub const SECTION: &str = "Open Workspaces";

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The list is on its way.
    Loading,
    /// Workspaces have arrived (possibly none).
    Ready,
    /// Workspaces cannot be listed, and why.
    Failed(String),
}

/// Switch Workspaces' state.
#[derive(Debug, Clone)]
pub struct WorkspacesPage {
    /// The filter text.
    pub query: String,
    /// Every workspace the engine reported.
    pub all: Vec<WorkspaceRow>,
    /// Positions in `all` that match `query`, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Why the last switch did not happen.
    pub notice: Option<String>,
}

impl Default for WorkspacesPage {
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

/// A row's subtitle: `3 windows - DP-1`, or `empty`.
#[must_use]
pub fn subtitle(row: &WorkspaceRow) -> String {
    workspace_subtitle(&WorkspaceEntry {
        name: row.name.clone(),
        screen_name: row.monitor.clone(),
        window_count: row.window_count,
        app_names: Vec::new(),
    })
}

impl WorkspacesPage {
    /// Takes the engine's answer.
    pub fn apply(&mut self, result: Result<Vec<WorkspaceRow>, String>) {
        match result {
            Ok(rows) => {
                self.all = rows;
                self.status = Status::Ready;
            }
            Err(reason) => {
                self.all.clear();
                self.status = Status::Failed(reason);
            }
        }
        self.refilter();
    }

    /// Recomputes `shown` for the current query: every workspace in the
    /// compositor's order when it is empty.
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
                let mut fields = vec![WeightedField::new(&row.name, 1.0)];
                if let Some(monitor) = &row.monitor {
                    fields.push(WeightedField::new(monitor, 0.8));
                }
                for (app, _) in &row.apps {
                    fields.push(WeightedField::new(app, 0.3));
                }
                let found = score_weighted(&fields, &query);
                (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.shown = scored.into_iter().map(|(_, index)| index).collect();
    }

    /// The selected workspace, if any.
    #[must_use]
    pub fn selected_row(&self) -> Option<&WorkspaceRow> {
        self.all.get(*self.shown.get(self.selected)?)
    }

    /// What the view says in place of the list: loading, why it failed, or
    /// that there is nothing to show.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Loading workspaces…"),
            Status::Failed(reason) => Some(reason.as_str()),
            Status::Ready if self.all.is_empty() => Some("No workspaces"),
            Status::Ready if self.shown.is_empty() => Some("No workspaces match"),
            Status::Ready => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str, monitor: &str, apps: &[&str]) -> WorkspaceRow {
        WorkspaceRow {
            id: id.into(),
            name: name.into(),
            monitor: Some(monitor.into()),
            window_count: apps.len(),
            apps: apps.iter().map(|a| ((*a).to_owned(), None)).collect(),
            active: false,
        }
    }

    fn page() -> WorkspacesPage {
        let mut page = WorkspacesPage::default();
        page.apply(Ok(vec![
            row("1", "1", "DP-1", &["Firefox", "Foot"]),
            row("3", "music", "HDMI-A-1", &["Spotify"]),
            row("4", "mail", "DP-1", &[]),
        ]));
        page
    }

    #[test]
    fn a_workspace_is_found_by_name_monitor_or_an_application_on_it() {
        let mut page = page();
        assert_eq!(page.shown, [0, 1, 2], "the compositor's order");
        page.query = "music".into();
        page.refilter();
        assert_eq!(page.selected_row().map(|r| r.id.as_str()), Some("3"));
        page.query = "spotify".into();
        page.refilter();
        assert_eq!(
            page.selected_row().map(|r| r.id.as_str()),
            Some("3"),
            "the workspace with Spotify on it"
        );
        page.query = "hdmi".into();
        page.refilter();
        assert_eq!(page.selected_row().map(|r| r.id.as_str()), Some("3"));
        page.query = "zzzz".into();
        page.refilter();
        assert!(page.selected_row().is_none());
    }

    #[test]
    fn a_row_counts_its_windows_and_names_its_monitor() {
        let page = page();
        assert_eq!(subtitle(&page.all[0]), "2 windows - DP-1");
        assert_eq!(subtitle(&page.all[2]), "empty - DP-1");
        let mut lone = page.all[1].clone();
        lone.monitor = None;
        assert_eq!(subtitle(&lone), "1 window");
    }

    #[test]
    fn a_failure_says_why_and_shows_nothing() {
        let mut page = page();
        page.apply(Err("needs Hyprland or niri".into()));
        assert!(page.shown.is_empty());
        assert_eq!(page.status, Status::Failed("needs Hyprland or niri".into()));
    }
}
