//! The application selector "Open with…" opens: the applications that
//! handle a target, filtered as typed, the chosen one opening it.
//!
//! The C++ offers these as a submenu of the action panel (`OpenWithAction`,
//! `OpenCompletedShortcutWithAction`); here it is a list view of its own,
//! the app-selector, so the same fuzzy search and keys apply as everywhere
//! else. Shortcuts, Search Files and clipboard history all open it.

use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

use crate::backend::OpenerRow;

/// The search field's placeholder.
pub const PLACEHOLDER: &str = "Open with...";

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The applications are on their way.
    Loading,
    /// They arrived (possibly none).
    Ready,
    /// They cannot be listed, and why.
    Failed(String),
}

/// The app-selector's state.
#[derive(Debug, Clone)]
pub struct OpenWithPage {
    /// What is opened: a path or a URL.
    pub target: String,
    /// What the list is looked up by, when not `target` itself: a shortcut's
    /// link before its placeholders are filled.
    pub lookup: String,
    /// The filter text.
    pub query: String,
    /// Every application the engine offered, the default first.
    pub all: Vec<OpenerRow>,
    /// Positions in `all` that match `query`, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Why the last open did not happen.
    pub notice: Option<String>,
}

impl OpenWithPage {
    /// A selector for `target`, its applications looked up by `lookup`.
    #[must_use]
    pub fn new(target: String, lookup: String) -> Self {
        Self {
            target,
            lookup,
            query: String::new(),
            all: Vec::new(),
            shown: Vec::new(),
            selected: 0,
            status: Status::Loading,
            notice: None,
        }
    }

    /// Takes the engine's answer.
    pub fn apply(&mut self, result: Result<Vec<OpenerRow>, String>) {
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

    /// Recomputes `shown` for the current query.
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
                let found = score_weighted(&[WeightedField::new(&row.name, 1.0)], &query);
                (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.shown = scored.into_iter().map(|(_, index)| index).collect();
    }

    /// The selected application, if any.
    #[must_use]
    pub fn selected_row(&self) -> Option<&OpenerRow> {
        self.all.get(*self.shown.get(self.selected)?)
    }

    /// What the view says in place of the list: loading, why it failed, or
    /// that there is nothing to show.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Finding applications…"),
            Status::Failed(reason) => Some(reason.as_str()),
            Status::Ready if self.all.is_empty() => Some("No application opens this"),
            Status::Ready if self.shown.is_empty() => Some("No applications match"),
            Status::Ready => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str, default: bool) -> OpenerRow {
        OpenerRow {
            id: id.into(),
            name: name.into(),
            icon: None,
            default,
        }
    }

    #[test]
    fn the_default_leads_and_typing_narrows_the_applications() {
        let mut page = OpenWithPage::new("/tmp/a.png".into(), "/tmp/a.png".into());
        page.apply(Ok(vec![
            row("org.gnome.Loupe.desktop", "Image Viewer", true),
            row("gimp.desktop", "GNU Image Manipulation Program", false),
            row("firefox.desktop", "Firefox", false),
        ]));
        assert_eq!(page.selected_row().map(|r| r.default), Some(true));
        page.query = "fire".into();
        page.refilter();
        assert_eq!(
            page.selected_row().map(|r| r.id.as_str()),
            Some("firefox.desktop")
        );
        page.query = "zzzz".into();
        page.refilter();
        assert!(page.selected_row().is_none());
    }

    #[test]
    fn a_failure_says_why() {
        let mut page = OpenWithPage::new("x".into(), "x".into());
        page.apply(Err("no engine".into()));
        assert_eq!(page.status, Status::Failed("no engine".into()));
        assert!(page.shown.is_empty());
    }
}
