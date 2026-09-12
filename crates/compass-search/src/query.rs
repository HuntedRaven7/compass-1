//! A parsed query: words, each with its matchable variants and score ceiling.
//!
//! Port of `fzf::Query` from `src/lib/fuzzy/include/fuzzy/fzf.hpp`.

use crate::matcher::Matcher;
use crate::translit::{TranslitScheme, needs_transliteration, transliterate};

/// One matchable spelling of a query word, with the best score it can reach.
///
/// Transliterations are shorter than their source script, so each variant needs
/// its own ceiling (`self_score`) or the normalized score would be nonsense.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// The needle text handed to the matcher.
    pub text: String,
    /// Score of `text` matched against itself: a perfect match for this variant.
    pub self_score: u32,
}

/// One whitespace-separated word of the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// The word as the user typed it.
    pub text: String,
    /// `text` plus any transliterations of it.
    pub variants: Vec<Variant>,
}

/// A query prepared once and scored against many items.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    text: String,
    words: Vec<Word>,
}

impl Query {
    /// Parses `text` using this thread's matcher for the score ceilings.
    pub fn new(text: &str) -> Self {
        Matcher::with_thread_local(|matcher| Self::with_matcher(text, matcher))
    }

    /// Parses `text`, computing score ceilings with `matcher`.
    pub fn with_matcher(text: &str, matcher: &mut Matcher) -> Self {
        let mut words = Vec::new();

        for word_text in text.split_whitespace() {
            let mut variants = vec![Variant {
                text: word_text.to_owned(),
                self_score: matcher.score_folded(word_text, word_text).unwrap_or(0),
            }];

            if needs_transliteration(word_text) {
                for scheme in TranslitScheme::ALL {
                    let Some(translit) = transliterate(word_text, scheme) else {
                        continue;
                    };
                    if variants.iter().any(|v| v.text == translit) {
                        continue;
                    }
                    let self_score = matcher.score_folded(&translit, &translit).unwrap_or(0);
                    variants.push(Variant {
                        text: translit,
                        self_score,
                    });
                }
            }

            words.push(Word {
                text: word_text.to_owned(),
                variants,
            });
        }

        Self {
            text: text.to_owned(),
            words,
        }
    }

    /// The raw query text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The parsed words.
    pub fn words(&self) -> &[Word] {
        &self.words
    }

    /// Whether the query contains no words (empty or whitespace-only).
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}
