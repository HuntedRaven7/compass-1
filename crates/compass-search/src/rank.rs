//! Ranking a list of [`FuzzySearchable`] items against a query.
//!
//! Port of `fuzzy::fuzzyFilter` / `fuzzy::FuzzyScorer` (`fuzzy-searchable.hpp`,
//! `weighted-fuzzy-scorer.hpp`).

use crate::matcher::Matcher;
use crate::query::Query;
use crate::searchable::{FuzzySearchable, Match, WeightedField, score_weighted_with};

/// An item paired with its score and its position in the input slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scored<T> {
    /// The scored value.
    pub item: T,
    /// Field-weighted score in `0..=100`.
    pub score: u32,
    /// Quality of the worst-matched query word, in `0..=100`.
    pub quality: u32,
    /// Raw weighted score; the tiebreak between items whose `score` is equal
    /// (notably both capped at 100). See [`Match::weighted`].
    pub weighted: u32,
    /// Index of the item in the input slice; the ranking tiebreak.
    pub index: usize,
}

/// Ranks `items` against `query`, best first.
///
/// Non-matching items (those failing the [`MIN_QUALITY`](crate::MIN_QUALITY)
/// gate) are dropped. An empty query keeps every item, in input order, with
/// score 0 — matching the C++ `fuzzyFilter`.
///
/// The order is **total and deterministic**: descending score, then ascending
/// input index. Equal-scoring items therefore keep their input order and two
/// runs over the same input always produce the same sequence.
pub fn rank<'a, T: FuzzySearchable>(query: &str, items: &'a [T]) -> Vec<Scored<&'a T>> {
    let parsed = Query::new(query);
    rank_with_query(&parsed, items)
}

/// [`rank`], but with a query parsed once and reused across calls.
pub fn rank_with_query<'a, T: FuzzySearchable>(
    query: &Query,
    items: &'a [T],
) -> Vec<Scored<&'a T>> {
    let scored = rank_indices_with_query(query, items);
    scored
        .into_iter()
        .map(|s| Scored {
            item: &items[s.index],
            score: s.score,
            quality: s.quality,
            weighted: s.weighted,
            index: s.index,
        })
        .collect()
}

/// [`rank`], returning indices into `items` instead of borrows.
pub fn rank_indices<T: FuzzySearchable>(query: &str, items: &[T]) -> Vec<Scored<usize>> {
    let parsed = Query::new(query);
    rank_indices_with_query(&parsed, items)
}

/// [`rank_indices`], with a pre-parsed query.
pub fn rank_indices_with_query<T: FuzzySearchable>(
    query: &Query,
    items: &[T],
) -> Vec<Scored<usize>> {
    let mut out: Vec<Scored<usize>> = Vec::with_capacity(items.len());

    if query.is_empty() {
        out.extend(items.iter().enumerate().map(|(index, _)| Scored {
            item: index,
            score: 0,
            quality: 0,
            weighted: 0,
            index,
        }));
        return out;
    }

    Matcher::with_thread_local(|matcher| {
        let mut fields: Vec<WeightedField<'_>> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            fields.clear();
            item.fuzzy_fields(&mut fields);
            let Match {
                score,
                quality,
                weighted,
            } = score_weighted_with(matcher, &fields, query);
            if quality >= crate::MIN_QUALITY {
                out.push(Scored {
                    item: index,
                    score,
                    quality,
                    weighted,
                    index,
                });
            }
        }
    });

    // Total order: normalized score descending, then raw weighted score
    // descending (the normalized score saturates at 100, so it alone leaves
    // many ties), then input index ascending. No two entries compare equal, so
    // the order is deterministic even with an unstable sort.
    out.sort_unstable_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.weighted.cmp(&a.weighted))
            .then(a.index.cmp(&b.index))
    });
    out
}
