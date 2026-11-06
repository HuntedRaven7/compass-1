//! Configure Fallback Commands (`ManageFallbackViewHost`): the items that can
//! answer a search nothing else claimed, in two sections, "Enabled" in the
//! configured order and "Available", each with the one action that moves it
//! to the other.
//!
//! `compass_core::bug_report` decides the sections and the enabled order
//! (`fallback_sections`, `order_enabled`); this is the view's state around it:
//! the candidates, the filter, and the selection.

use compass_core::bug_report::{FallbackSection, fallback_action_label, order_enabled};
use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

/// The search field's placeholder.
pub const PLACEHOLDER: &str = "Search commands...";

/// An item that can be a fallback (`isSuitableForFallback`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The id a `fallbacks` entry names it by when it is enabled here.
    pub id: String,
    /// The row's title.
    pub title: String,
    /// The row's subtitle.
    pub subtitle: String,
    /// Extra search terms, weighed at 0.3 as `FuzzySearchable<RootItemPtr>`.
    pub keywords: Vec<String>,
    /// A builtin icon, where the item has one.
    pub icon: Option<&'static str>,
    /// The `fallbacks` entry that names it now, if one does: the id it is
    /// disabled by, which for Search Files may be either of its two ids.
    pub enabled_as: Option<String>,
}

/// One row on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Position in [`FallbacksPage::candidates`].
    pub candidate: usize,
    /// Its section.
    pub section: FallbackSection,
}

/// The view's state.
#[derive(Debug, Clone, Default)]
pub struct FallbacksPage {
    /// The filter text.
    pub query: String,
    /// Every item that can be a fallback.
    pub candidates: Vec<Candidate>,
    /// The configured order, the `fallbacks` entries.
    pub order: Vec<String>,
    /// The rows shown: the enabled ones, then the available ones.
    pub rows: Vec<Row>,
    /// Position in `rows`.
    pub selected: usize,
    /// Why the last change was not kept.
    pub notice: Option<String>,
}

impl FallbacksPage {
    /// The view over `candidates`, with `order` the configured `fallbacks`.
    #[must_use]
    pub fn new(candidates: Vec<Candidate>, order: Vec<String>) -> Self {
        let mut page = Self {
            candidates,
            order,
            ..Self::default()
        };
        page.refilter();
        page
    }

    /// Takes new candidates and order, keeping the filter and, as far as it
    /// can, the selection, as the C++ reloads on `fallbackEnabled`.
    pub fn reload(&mut self, candidates: Vec<Candidate>, order: Vec<String>) {
        let selected = self.selected;
        self.candidates = candidates;
        self.order = order;
        self.refilter();
        self.selected = selected.min(self.rows.len().saturating_sub(1));
    }

    /// Recomputes the rows for the query: the matches of each section, the
    /// enabled ones in the configured order and the available ones by score.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let query = Query::new(&self.query);
        let score = |candidate: &Candidate| -> Option<u32> {
            if query.is_empty() {
                return Some(0);
            }
            let mut fields = vec![WeightedField::new(&candidate.title, 1.0)];
            fields.extend(
                candidate
                    .keywords
                    .iter()
                    .map(|keyword| WeightedField::new(keyword, 0.3)),
            );
            let found = score_weighted(&fields, &query);
            (found.quality >= MIN_QUALITY && found.score > 0).then_some(found.score)
        };
        let mut enabled: Vec<(String, usize)> = Vec::new();
        let mut available: Vec<(u32, usize)> = Vec::new();
        for (index, candidate) in self.candidates.iter().enumerate() {
            let Some(found) = score(candidate) else {
                continue;
            };
            match &candidate.enabled_as {
                Some(id) => enabled.push((id.clone(), index)),
                None => available.push((found, index)),
            }
        }
        let ids: Vec<String> = enabled.iter().map(|(id, _)| id.clone()).collect();
        let ordered = order_enabled(&ids, &self.order);
        self.rows = ordered
            .iter()
            .filter_map(|id| enabled.iter().find(|(known, _)| known == id))
            .map(|&(_, candidate)| Row {
                candidate,
                section: FallbackSection::Enabled,
            })
            .collect();
        available.sort_by(|a, b| b.0.cmp(&a.0));
        self.rows
            .extend(available.into_iter().map(|(_, candidate)| Row {
                candidate,
                section: FallbackSection::Available,
            }));
    }

    /// The selected row and its item.
    #[must_use]
    pub fn selected_row(&self) -> Option<(Row, &Candidate)> {
        let row = *self.rows.get(self.selected)?;
        Some((row, self.candidates.get(row.candidate)?))
    }

    /// What the view shows in place of the list when it has nothing to list.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&'static str> {
        if !self.rows.is_empty() {
            None
        } else if self.candidates.is_empty() {
            Some("Nothing can be a fallback")
        } else {
            Some("No commands match")
        }
    }

    /// The heading above the row at `position`, when it starts its section.
    #[must_use]
    pub fn heading_at(&self, position: usize) -> Option<&'static str> {
        let row = self.rows.get(position)?;
        let starts = position == 0
            || self
                .rows
                .get(position - 1)
                .is_some_and(|before| before.section != row.section);
        starts.then_some(section_name(row.section))
    }

    /// The selected row's action: its label, the id to change, and whether
    /// it enables.
    #[must_use]
    pub fn selected_action(&self) -> Option<(&'static str, String, bool)> {
        let (row, candidate) = self.selected_row()?;
        let label = fallback_action_label(row.section);
        Some(match row.section {
            FallbackSection::Enabled => (
                label,
                candidate
                    .enabled_as
                    .clone()
                    .unwrap_or_else(|| candidate.id.clone()),
                false,
            ),
            FallbackSection::Available => (label, candidate.id.clone(), true),
        })
    }
}

/// A section's heading, as `sectionName`.
#[must_use]
pub const fn section_name(section: FallbackSection) -> &'static str {
    match section {
        FallbackSection::Enabled => "Enabled",
        FallbackSection::Available => "Available",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, title: &str, enabled_as: Option<&str>) -> Candidate {
        Candidate {
            id: id.into(),
            title: title.into(),
            subtitle: String::new(),
            keywords: Vec::new(),
            icon: None,
            enabled_as: enabled_as.map(str::to_owned),
        }
    }

    #[test]
    fn enabled_come_first_in_the_configured_order_then_the_available() {
        let page = FallbacksPage::new(
            vec![
                candidate("files:search", "Search Files", Some("files:search")),
                candidate("@a/notes:new", "New Note", Some("@a/notes:new")),
                candidate("shortcuts:google", "Google", None),
            ],
            vec!["@a/notes:new".into(), "files:search".into()],
        );
        let titles: Vec<&str> = page
            .rows
            .iter()
            .map(|row| page.candidates[row.candidate].title.as_str())
            .collect();
        assert_eq!(titles, ["New Note", "Search Files", "Google"]);
        assert_eq!(page.heading_at(0), Some("Enabled"));
        assert_eq!(page.heading_at(1), None);
        assert_eq!(page.heading_at(2), Some("Available"));
        assert_eq!(
            page.selected_action(),
            Some(("Disable fallback", "@a/notes:new".to_owned(), false))
        );
    }

    #[test]
    fn the_filter_narrows_both_sections_and_an_available_row_enables() {
        let mut page = FallbacksPage::new(
            vec![
                candidate(
                    "files:search",
                    "Search Files",
                    Some("commands:search-files"),
                ),
                candidate("shortcuts:google", "Google", None),
            ],
            vec!["commands:search-files".into()],
        );
        page.query = "goo".into();
        page.refilter();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(
            page.selected_action(),
            Some(("Enable fallback", "shortcuts:google".to_owned(), true))
        );
        page.query = "search".into();
        page.refilter();
        assert_eq!(
            page.selected_action(),
            Some((
                "Disable fallback",
                "commands:search-files".to_owned(),
                false
            )),
            "disabled by the entry that names it"
        );
    }
}
