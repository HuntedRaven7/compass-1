//! Ranking that combines fuzzy match quality with launch history.
//!
//! [`compass_search::rank`] answers "how well does this item match what was typed". A launcher
//! also has to answer "and is this the one you always pick", which is what [`crate::frecency`]
//! records. This module is the join: match score plus up to [`compass_search::FRECENCY_WEIGHT`]
//! points of frecency, on the same 0-100 scale the matcher uses.

use compass_search::{FRECENCY_WEIGHT, FuzzySearchable};

use crate::frecency::FrecencyStore;

/// An item together with why it ranked where it did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ranked<T> {
    /// The ranked value.
    pub item: T,
    /// Combined score: `match_score + FRECENCY_WEIGHT * frecency`.
    pub score: f64,
    /// The field-weighted fuzzy score in `0..=100`, before the frecency boost.
    pub match_score: u32,
    /// Quality of the worst-matched query word, in `0..=100`.
    pub quality: u32,
    /// The raw weighted matcher score; the tiebreak between equal `score`s.
    pub weighted: u32,
    /// The item's frecency in `[0, 1]` at the store's current time.
    pub frecency: f64,
    /// Index of the item in the input slice.
    pub index: usize,
}

/// Ranks `items` against `query`, boosting each by its launch history.
///
/// Items are keyed into the store by `key`, so the same function serves applications
/// ([`crate::AppItem::key`]) and anything else that grows a history later.
///
/// An empty query keeps every item and orders it purely by frecency, which is the "most used
/// first" list a launcher shows before anything is typed. A non-empty query drops non-matching
/// items exactly as [`compass_search::rank`] does — frecency reorders results, it never
/// resurrects one that does not match.
///
/// The order is total and deterministic: combined score descending, raw weighted score
/// descending, then input index ascending.
pub fn rank_with_frecency<'a, T, K, S>(
    query: &str,
    items: &'a [T],
    key: K,
    store: &S,
) -> Vec<Ranked<&'a T>>
where
    T: FuzzySearchable,
    K: Fn(&T) -> &str,
    S: FrecencyStore + ?Sized,
{
    let now = store.now();
    let scored = compass_search::rank_indices(query, items);

    let mut out: Vec<Ranked<&'a T>> = scored
        .into_iter()
        .map(|s| {
            let item = &items[s.index];
            let frecency = store
                .record(key(item))
                .map_or(0.0, |record| record.score_at(now));
            Ranked {
                item,
                score: f64::from(s.score) + FRECENCY_WEIGHT * frecency,
                match_score: s.score,
                quality: s.quality,
                weighted: s.weighted,
                frecency,
                index: s.index,
            }
        })
        .collect();

    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.weighted.cmp(&a.weighted))
            .then(a.index.cmp(&b.index))
    });
    out
}

impl crate::AppIndex {
    /// Ranks the index against `query`, boosting each item by its launch history.
    ///
    /// See [`rank_with_frecency`].
    #[must_use]
    pub fn search_with_frecency<'a, S: FrecencyStore + ?Sized>(
        &'a self,
        query: &str,
        store: &S,
    ) -> Vec<Ranked<&'a crate::AppItem>> {
        rank_with_frecency(query, self.items(), |item| item.key(), store)
    }
}
