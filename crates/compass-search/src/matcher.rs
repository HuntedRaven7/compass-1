//! The low-level matcher: a reusable wrapper around [`nucleo_matcher::Matcher`].
//!
//! Ports the role of `fzf::Matcher` from `src/lib/fuzzy/include/fuzzy/fzf.hpp`.
//! The *algorithm* is nucleo's, not fzf-v2-as-ported-to-C++, so absolute scores
//! differ; the semantics ported here are the surrounding ones: case-insensitive
//! and diacritic-insensitive matching, cross-script transliteration of the
//! needle, and a thread-local reusable instance (nucleo, like the C++ matcher,
//! keeps scratch buffers and so needs `&mut self`).

use std::cell::RefCell;
use std::ops::Range;

use nucleo_matcher::{Config, Utf32Str, chars};

use crate::coherence::is_coherent_with;
use crate::translit::{TranslitScheme, needs_transliteration, transliterate};

/// A successful match of a needle against a haystack.
///
/// Indices are **character** indices into the haystack, not byte offsets (the
/// C++ `fzf::Result` reports byte offsets; nucleo works in `char`s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchResult {
    /// The raw matcher score. Only comparable against other scores produced by
    /// the same needle; not comparable with the C++ fzf scores.
    pub score: u32,
    /// Char indices of the matched haystack characters, ascending.
    pub indices: Vec<u32>,
    /// Whether this alignment reads as a match a human would recognize, in the
    /// sense of the C++ `fzf::Result::coherent`: it is `false` when the match
    /// spreads over several words *and* some run of consecutive matched
    /// characters starts mid-word ("time" in "S[t]art [I]nput [Me]thod").
    ///
    /// Incoherent matches are excluded from [`Match::quality`](crate::Match::quality)
    /// and so cannot clear the [`MIN_QUALITY`](crate::MIN_QUALITY) gate. See
    /// [`crate::is_coherent`].
    pub coherent: bool,
}

impl MatchResult {
    /// Char index of the first matched character, if any.
    pub fn start(&self) -> Option<u32> {
        self.indices.first().copied()
    }

    /// Char index one past the last matched character, if any.
    pub fn end(&self) -> Option<u32> {
        self.indices.last().map(|last| last + 1)
    }

    /// The half-open char range spanned by the match (`0..0` when the needle
    /// was empty).
    pub fn range(&self) -> Range<u32> {
        match (self.start(), self.end()) {
            (Some(start), Some(end)) => start..end,
            _ => 0..0,
        }
    }

    /// Whether the matched characters form one uninterrupted run.
    pub fn is_contiguous(&self) -> bool {
        self.indices.windows(2).all(|w| w[1] == w[0] + 1)
    }
}

/// A fuzzy matcher with reusable scratch buffers.
///
/// Not `Sync`-friendly by design: like the C++ matcher it reuses internal
/// allocations, so share it per-thread via [`Matcher::with_thread_local`].
pub struct Matcher {
    inner: nucleo_matcher::Matcher,
    haystack_buf: Vec<char>,
    needle_buf: Vec<char>,
    needle_str: String,
    indices_buf: Vec<u32>,
    boundary_buf: Vec<bool>,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

thread_local! {
    static THREAD_LOCAL_MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new());
}

impl Matcher {
    /// Creates a matcher with the default configuration (case-insensitive,
    /// diacritic-folding).
    pub fn new() -> Self {
        Self {
            inner: nucleo_matcher::Matcher::new(Config::DEFAULT),
            haystack_buf: Vec::new(),
            needle_buf: Vec::new(),
            needle_str: String::new(),
            indices_buf: Vec::new(),
            boundary_buf: Vec::new(),
        }
    }

    /// Runs `f` with this thread's shared matcher.
    ///
    /// The C++ side hands out a `const Matcher&` from a `thread_local`; nucleo
    /// needs `&mut`, so the Rust equivalent is a scoped closure over a
    /// `RefCell`. Calls nest safely: a re-entrant call (for instance a
    /// [`FuzzySearchable`](crate::FuzzySearchable) implementation that scores
    /// something itself) gets a temporary matcher rather than a panic. Only the
    /// buffer reuse is lost, not correctness.
    pub fn with_thread_local<R>(f: impl FnOnce(&mut Matcher) -> R) -> R {
        THREAD_LOCAL_MATCHER.with(|cell| match cell.try_borrow_mut() {
            Ok(mut matcher) => f(&mut matcher),
            Err(_) => f(&mut Matcher::new()),
        })
    }

    /// Normalizes `needle` into `out`: diacritics folded and case lowered, as
    /// nucleo requires of needles when those config options are on.
    fn prepare_needle(out: &mut String, needle: &str) {
        out.clear();
        out.extend(
            needle
                .chars()
                .map(|c| chars::to_lower_case(chars::normalize(c))),
        );
    }

    /// Scores `needle` against `haystack` without computing indices.
    ///
    /// This is the hot path used by ranking. Transliteration variants of the
    /// needle are tried and the best score wins, as in the C++ `Matcher::match`.
    pub fn score(&mut self, haystack: &str, needle: &str) -> Option<u32> {
        let mut best = self.score_folded(haystack, needle);

        if needs_transliteration(needle) {
            for scheme in TranslitScheme::ALL {
                let Some(variant) = transliterate(needle, scheme) else {
                    continue;
                };
                if variant == needle {
                    continue;
                }
                let candidate = self.score_folded(haystack, &variant);
                if candidate > best {
                    best = candidate;
                }
            }
        }

        best
    }

    /// Matches `needle` against `haystack`, returning the score and the matched
    /// char indices, or `None` when the needle is not a subsequence.
    ///
    /// An empty needle matches everything with score 0 and no indices (the C++
    /// `match` likewise reports a zero-length match rather than a non-match).
    pub fn match_(&mut self, haystack: &str, needle: &str) -> Option<MatchResult> {
        let mut best = self.match_folded(haystack, needle);

        if needs_transliteration(needle) {
            for scheme in TranslitScheme::ALL {
                let Some(variant) = transliterate(needle, scheme) else {
                    continue;
                };
                if variant == needle {
                    continue;
                }
                let candidate = self.match_folded(haystack, &variant);
                let better = match (&best, &candidate) {
                    (Some(best), Some(candidate)) => candidate.score > best.score,
                    (None, Some(_)) => true,
                    _ => false,
                };
                if better {
                    best = candidate;
                }
            }
        }

        best
    }

    /// Like [`Matcher::score`] but without transliteration: the direct
    /// equivalent of the C++ `match_folded`.
    pub fn score_folded(&mut self, haystack: &str, needle: &str) -> Option<u32> {
        if needle.is_empty() {
            return Some(0);
        }
        let Self {
            inner,
            haystack_buf,
            needle_buf,
            needle_str,
            ..
        } = self;
        Self::prepare_needle(needle_str, needle);
        let needle = Utf32Str::new(needle_str, needle_buf);
        let haystack = Utf32Str::new(haystack, haystack_buf);
        inner.fuzzy_match(haystack, needle).map(u32::from)
    }

    /// Like [`Matcher::match_`] but without transliteration.
    pub fn match_folded(&mut self, haystack: &str, needle: &str) -> Option<MatchResult> {
        if needle.is_empty() {
            return Some(MatchResult {
                score: 0,
                indices: Vec::new(),
                coherent: true,
            });
        }
        let Self {
            inner,
            haystack_buf,
            needle_buf,
            needle_str,
            boundary_buf,
            ..
        } = self;
        Self::prepare_needle(needle_str, needle);
        let needle_utf = Utf32Str::new(needle_str, needle_buf);
        let haystack_utf = Utf32Str::new(haystack, haystack_buf);
        let mut indices = Vec::new();
        let score = inner.fuzzy_indices(haystack_utf, needle_utf, &mut indices)?;
        indices.sort_unstable();
        let coherent = is_coherent_with(haystack, &indices, boundary_buf);
        Some(MatchResult {
            score: u32::from(score),
            indices,
            coherent,
        })
    }

    /// Score `needle` against `haystack` together with the coherence of the
    /// alignment nucleo picked, without allocating a fresh index vector.
    ///
    /// The scoring path needs the coherence flag (it gates
    /// [`Match::quality`](crate::Match::quality)), and coherence is a property
    /// of an alignment, so unlike [`Matcher::score_folded`] this has to ask
    /// nucleo for indices. The C++ pays the same price: its backtracking pass
    /// runs on every match precisely so `fzf::Result::coherent` is always
    /// populated, whether or not positions were requested.
    pub(crate) fn score_folded_coherent(
        &mut self,
        haystack: &str,
        needle: &str,
    ) -> Option<(u32, bool)> {
        if needle.is_empty() {
            return Some((0, true));
        }
        let Self {
            inner,
            haystack_buf,
            needle_buf,
            needle_str,
            indices_buf,
            boundary_buf,
        } = self;
        Self::prepare_needle(needle_str, needle);
        let needle_utf = Utf32Str::new(needle_str, needle_buf);
        let haystack_utf = Utf32Str::new(haystack, haystack_buf);
        indices_buf.clear();
        let score = inner.fuzzy_indices(haystack_utf, needle_utf, indices_buf)?;
        indices_buf.sort_unstable();
        let coherent = is_coherent_with(haystack, indices_buf, boundary_buf);
        Some((u32::from(score), coherent))
    }
}
