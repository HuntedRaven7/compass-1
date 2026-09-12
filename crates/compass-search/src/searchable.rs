//! Weighted multi-field scoring and the [`FuzzySearchable`] trait.
//!
//! Port of `src/lib/fuzzy/include/fuzzy/fuzzy-searchable.hpp`.

use crate::matcher::Matcher;
use crate::query::Query;

/// Minimum [`Match::quality`] for an item to be considered a hit.
pub const MIN_QUALITY: u32 = 60;

/// One searchable field of an item, and how much it counts towards ranking.
///
/// A weight of `1.0` means "as good as a perfect match"; a keyword or
/// description field typically gets `0.5` so it ranks below a name match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeightedField<'a> {
    /// The text to match against.
    pub text: &'a str,
    /// Multiplier applied to this field's raw score.
    pub weight: f32,
}

impl<'a> WeightedField<'a> {
    /// Convenience constructor.
    pub fn new(text: &'a str, weight: f32) -> Self {
        Self { text, weight }
    }
}

/// The outcome of scoring an item against a query.
///
/// `score` is field-weighted and used for ranking; `quality` is the unweighted
/// score of the *worst-matched* query word, so it can be thresholded
/// independently of query length and is used for filtering. Both are 0-100.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    /// Field-weighted, query-normalized score in `0..=100`.
    pub score: u32,
    /// Unweighted quality of the worst-matched query word, in `0..=100`.
    pub quality: u32,
    /// The raw field-weighted matcher score averaged over query words, before
    /// normalization against a perfect match. Unbounded and only meaningful
    /// relative to other items scored with the *same* query, but it keeps the
    /// resolution that `score` loses to its 0-100 cap, so ranking uses it as a
    /// tiebreak. The C++ ordering tests rank on this value
    /// (`QueryScore::weighted` in `order-helpers.hpp`).
    pub weighted: u32,
}

impl Match {
    /// Whether the match clears the [`MIN_QUALITY`] filtering gate.
    pub fn accepted(self) -> bool {
        self.quality >= MIN_QUALITY
    }
}

/// A type that can be fuzzy-searched over one or more weighted fields.
///
/// This is the Rust stand-in for the C++ `template <typename T> struct
/// FuzzySearchable` specialisation point. C++ needs an external template
/// because it cannot add members to foreign types; Rust's coherence rules make
/// a plain trait the natural equivalent, and unlike the C++ concept it is
/// checked at the definition site.
///
/// The method pushes into a caller-owned buffer rather than returning a
/// `Vec`/iterator: ranking calls it once per item, and a reused buffer keeps
/// that allocation-free, which is the same reason the C++ side passes
/// `std::span`/`views` around instead of materializing vectors. Returning
/// `impl Iterator` would need a GAT to borrow from `&self`, and returning a
/// `Vec` would allocate per item.
///
/// ```
/// use compass_search::{FuzzySearchable, WeightedField};
///
/// struct App { name: String, description: String }
///
/// impl FuzzySearchable for App {
///     fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
///         out.push(WeightedField::new(&self.name, 1.0));
///         out.push(WeightedField::new(&self.description, 0.5));
///     }
/// }
/// ```
pub trait FuzzySearchable {
    /// Appends this item's searchable fields, with their weights, to `out`.
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>);
}

impl FuzzySearchable for str {
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
        out.push(WeightedField::new(self, 1.0));
    }
}

impl FuzzySearchable for String {
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
        out.push(WeightedField::new(self, 1.0));
    }
}

impl<T: FuzzySearchable + ?Sized> FuzzySearchable for &T {
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
        (**self).fuzzy_fields(out);
    }
}

/// Scores `fields` against `query` using this thread's matcher.
pub fn score_weighted(fields: &[WeightedField<'_>], query: &Query) -> Match {
    Matcher::with_thread_local(|matcher| score_weighted_with(matcher, fields, query))
}

/// Scores `fields` against `query`.
///
/// Every query word must match at least one field (AND semantics); a word that
/// matches nothing zeroes the whole match, exactly as in the C++
/// `Matcher::score_query`.
pub fn score_weighted_with(
    matcher: &mut Matcher,
    fields: &[WeightedField<'_>],
    query: &Query,
) -> Match {
    if query.is_empty() || fields.is_empty() {
        return Match::default();
    }

    let mut weighted_sum: u64 = 0;
    let mut self_sum: u64 = 0;
    let mut min_quality = 100;

    for word in query.words() {
        let mut best_weighted: u64 = 0;
        let mut best_weighted_ratio: u64 = 0;
        let mut best_self: u64 = 0;
        let mut quality: u64 = 0;

        for variant in &word.variants {
            if variant.self_score == 0 {
                continue;
            }

            let mut max_weighted: u64 = 0;
            let mut max_raw: u64 = 0;
            for field in fields {
                let Some((raw, coherent)) =
                    matcher.score_folded_coherent(field.text, &variant.text)
                else {
                    continue;
                };
                // Only coherent alignments count towards `quality`, exactly as
                // in the C++ `score_query`: an incoherent match still ranks
                // (it contributes to `weighted`/`score`) but cannot clear the
                // `MIN_QUALITY` gate on its own.
                if coherent {
                    max_raw = max_raw.max(u64::from(raw));
                }
                let weighted = (raw as f32 * field.weight) as u64;
                max_weighted = max_weighted.max(weighted);
            }

            let self_score = u64::from(variant.self_score);
            let weighted_ratio = max_weighted * 100 / self_score;
            if max_weighted > 0 && (best_weighted == 0 || weighted_ratio > best_weighted_ratio) {
                best_weighted = max_weighted;
                best_weighted_ratio = weighted_ratio;
                best_self = self_score;
            }
            quality = quality.max(max_raw * 100 / self_score);
        }

        if best_weighted == 0 {
            return Match::default();
        }

        min_quality = min_quality.min(quality);
        weighted_sum += best_weighted;
        self_sum += best_self;
    }

    if self_sum == 0 {
        return Match::default();
    }

    Match {
        score: (weighted_sum * 100 / self_sum).min(100) as u32,
        quality: min_quality as u32,
        weighted: (weighted_sum / query.words().len() as u64) as u32,
    }
}

/// Scores an item against a query, using this thread's matcher.
pub fn score_item<T: FuzzySearchable + ?Sized>(item: &T, query: &Query) -> Match {
    Matcher::with_thread_local(|matcher| {
        let mut fields = Vec::new();
        item.fuzzy_fields(&mut fields);
        score_weighted_with(matcher, &fields, query)
    })
}

/// Recency-and-frequency score in `[0, 1]`; timestamps are unix seconds.
///
/// Direct port of `fuzzy::frecency`.
pub fn frecency(visit_count: u32, last_visited_at: Option<u64>, now: i64) -> f64 {
    const FREQUENCY_SCALE: f64 = 5.0;
    const RECENCY_PEAK: f64 = 10.0;
    const RECENCY_HALF_LIFE_DAYS: f64 = 30.0;
    const CAP: f64 = 25.0;
    const SECONDS_PER_DAY: f64 = 86400.0;

    let frequency = FREQUENCY_SCALE * (1.0 + f64::from(visit_count) * 0.1).ln();
    let recency = match last_visited_at {
        Some(last) => {
            let days_since = (now - last as i64) as f64 / SECONDS_PER_DAY;
            RECENCY_PEAK * (-days_since.max(0.0) / RECENCY_HALF_LIFE_DAYS).exp()
        }
        None => 0.0,
    };

    ((frequency + recency) / CAP).min(1.0)
}

/// Score points (out of 100) a maximal [`frecency`] is worth.
pub const FRECENCY_WEIGHT: f64 = 6.0;
