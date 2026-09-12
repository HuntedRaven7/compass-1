//! Fuzzy search for compass: a matcher, a weighted-field searchable trait, and
//! a deterministic ranker.
//!
//! This is a Rust port of the C++ `src/lib/fuzzy` library. The *semantics* are
//! ported — case- and diacritic-insensitive matching, cross-script
//! transliteration, per-field weights, a quality gate for filtering, and a
//! stable ranking order. The scoring *algorithm* is [`nucleo_matcher`]'s rather
//! than the C++ side's fzf-v2 port, so absolute score values differ between the
//! two implementations; only relative order is a contract.

#![deny(missing_docs)]

mod coherence;
mod matcher;
mod query;
mod rank;
mod searchable;
mod translit;

pub use coherence::is_coherent;
pub use matcher::{MatchResult, Matcher};
pub use query::{Query, Variant, Word};
pub use rank::{Scored, rank, rank_indices, rank_indices_with_query, rank_with_query};
pub use searchable::{
    FRECENCY_WEIGHT, FuzzySearchable, MIN_QUALITY, Match, WeightedField, frecency, score_item,
    score_weighted, score_weighted_with,
};
pub use translit::{TranslitScheme, needs_transliteration, transliterate, transliterate_char};
