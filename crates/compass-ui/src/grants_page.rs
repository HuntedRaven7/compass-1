//! Script Permissions: what the user has allowed their own Rhai scripts, and
//! revoking it.
//!
//! The engine reads and writes `script-grants.json`; this keeps the list, its
//! filter and the selection.

use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

use crate::backend::ScriptGrant;

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The list is on its way.
    Loading,
    /// It arrived (possibly empty).
    Ready,
    /// It cannot be read, and why.
    Failed(String),
}

/// The view's state.
#[derive(Debug, Clone)]
pub struct GrantsPage {
    /// The filter text.
    pub query: String,
    /// Every script with something allowed, in id order.
    pub all: Vec<ScriptGrant>,
    /// Positions in `all` that match `query`, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// What the last action said.
    pub notice: Option<String>,
}

impl Default for GrantsPage {
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

/// The search field's placeholder.
pub const PLACEHOLDER: &str = "Search scripts...";

/// What the view says when nothing is allowed.
pub const EMPTY: &str = "No script has been allowed anything";

/// What a revoke says.
pub const REVOKED: &str = "Permissions revoked";

impl GrantsPage {
    /// Takes the engine's answer, keeping the selection's position.
    pub fn apply(&mut self, result: Result<Vec<ScriptGrant>, String>) {
        let selected = self.selected;
        match result {
            Ok(grants) => {
                self.all = grants;
                self.status = Status::Ready;
            }
            Err(reason) => {
                self.all.clear();
                self.status = Status::Failed(reason);
            }
        }
        self.refilter();
        self.selected = selected.min(self.shown.len().saturating_sub(1));
    }

    /// Recomputes `shown` for the current query, back at the top.
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
            .filter_map(|(index, grant)| {
                let fields = [
                    WeightedField::new(&grant.title, 1.0),
                    WeightedField::new(&grant.id, 0.5),
                ];
                let found = score_weighted(&fields, &query);
                (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.shown = scored.into_iter().map(|(_, index)| index).collect();
    }

    /// The selected script, if any.
    #[must_use]
    pub fn selected_grant(&self) -> Option<&ScriptGrant> {
        self.all.get(*self.shown.get(self.selected)?)
    }

    /// What the view says in place of the list: loading, why the list
    /// cannot be read, that nothing is allowed, or that nothing matches;
    /// `None` while it shows scripts.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Reading script permissions…"),
            Status::Failed(reason) => Some(reason),
            Status::Ready if self.all.is_empty() => Some(EMPTY),
            Status::Ready if self.shown.is_empty() => Some("No scripts match"),
            Status::Ready => None,
        }
    }
}

/// A row's second line: what the script may do.
#[must_use]
pub fn subtitle(grant: &ScriptGrant) -> String {
    let mut line = grant.descriptions.join(", ");
    if let Some(first) = line.get(..1) {
        let upper = first.to_uppercase();
        line.replace_range(..1, &upper);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(id: &str, title: &str, descriptions: &[&str]) -> ScriptGrant {
        ScriptGrant {
            id: id.into(),
            title: title.into(),
            capabilities: Vec::new(),
            descriptions: descriptions.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    #[test]
    fn scripts_are_found_by_title_and_say_what_they_may_do() {
        let mut page = GrantsPage::default();
        page.apply(Ok(vec![
            grant(
                "script.notes",
                "Quick Notes",
                &["read its saved data", "save data"],
            ),
            grant("script.clip", "Clip Tool", &["copy to the clipboard"]),
        ]));
        assert_eq!(page.shown, [0, 1]);
        page.query = "clip".into();
        page.refilter();
        assert_eq!(
            page.selected_grant().map(|g| g.id.as_str()),
            Some("script.clip")
        );
        assert_eq!(subtitle(&page.all[0]), "Read its saved data, save data");
        page.query.clear();
        page.refilter();
        page.selected = 1;
        page.apply(Ok(vec![grant("script.notes", "Quick Notes", &[])]));
        assert_eq!(page.selected, 0, "the selection stays inside the list");
    }
}
