//! Property tests: the matcher and the ranker must survive arbitrary input.

use compass_search::{
    FuzzySearchable, Matcher, Query, WeightedField, rank_indices, score_weighted,
};
use proptest::prelude::*;

/// Where a counterexample gets written so it can be replayed.
///
/// proptest's default `SourceParallel` persistence looks for `lib.rs` or `main.rs` beside
/// the test file. An integration test under `tests/` has neither, so proptest prints
/// "failed to find lib.rs or main.rs" and *discards the seed*. A property that fails on
/// one seed in a few hundred is then unreplayable -- and that is precisely the failure
/// worth keeping, since it will not reproduce on the next run. Writing the seed to a
/// checked-in file turns each such find into a permanent regression case.
fn regressions(cases: u32, file: &'static str) -> ProptestConfig {
    ProptestConfig {
        cases,
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct(file),
        )),
        ..ProptestConfig::default()
    }
}

/// Text drawn from ASCII, Latin-1 accents, Cyrillic, Greek, CJK and emoji, so
/// the normalization, transliteration and multi-byte paths all get exercised.
fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        ".{0,32}",
        "[a-zA-Z0-9 ._/:;|-]{0,32}",
        "[\\u00c0-\\u024f ]{0,16}",
        "[\\u0400-\\u045f ]{0,16}",
        "[\\u0386-\\u03ce ]{0,16}",
        "[\\u4e00-\\u9fff\\u3040-\\u30ff]{0,12}",
        "[\\u{1f300}-\\u{1f5ff}]{0,8}",
    ]
}

struct Item(String, String);

impl FuzzySearchable for Item {
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
        out.push(WeightedField::new(&self.0, 1.0));
        out.push(WeightedField::new(&self.1, 0.5));
    }
}

proptest! {
    #![proptest_config(regressions(512, "tests/regressions/properties.txt"))]

    #[test]
    fn matching_never_panics(haystack in text(), needle in text()) {
        Matcher::with_thread_local(|m| {
            let by_score = m.score(&haystack, &needle);
            let by_match = m.match_(&haystack, &needle);

            // The two entry points must agree on whether there is a match, and
            // any indices reported must be in range and ascending.
            prop_assert_eq!(by_score.is_some(), by_match.is_some());
            if let Some(result) = by_match {
                let chars = haystack.chars().count() as u32;
                prop_assert!(result.indices.windows(2).all(|w| w[0] < w[1]));
                prop_assert!(result.indices.iter().all(|i| *i < chars));
                if !result.indices.is_empty() {
                    prop_assert!(result.range().end <= chars);
                }
            }
            Ok(())
        })?;
    }

    #[test]
    fn scoring_never_panics_and_stays_in_range(
        haystack in text(),
        other in text(),
        needle in text(),
    ) {
        let query = Query::new(&needle);
        let m = score_weighted(
            &[WeightedField::new(&haystack, 1.0), WeightedField::new(&other, 0.5)],
            &query,
        );
        prop_assert!(m.score <= 100);
        prop_assert!(m.quality <= 100);
        prop_assert!(!(m.accepted() && m.weighted == 0));
    }

    #[test]
    fn ranking_never_panics_and_is_deterministic(
        items in prop::collection::vec((text(), text()), 0..12),
        needle in text(),
    ) {
        let items: Vec<Item> = items.into_iter().map(|(a, b)| Item(a, b)).collect();
        let first = rank_indices(&needle, &items);
        let second = rank_indices(&needle, &items);
        prop_assert_eq!(&first, &second);

        prop_assert!(first.len() <= items.len());
        // Indices are unique, in range, and the order is a total one.
        let mut seen = vec![false; items.len()];
        for scored in &first {
            prop_assert!(!seen[scored.index]);
            seen[scored.index] = true;
        }
        for pair in first.windows(2) {
            prop_assert!(
                (pair[0].score, pair[0].weighted) > (pair[1].score, pair[1].weighted)
                    || ((pair[0].score, pair[0].weighted) == (pair[1].score, pair[1].weighted)
                        && pair[0].index < pair[1].index)
            );
        }
    }

    #[test]
    fn a_string_always_matches_itself(haystack in "[a-zA-Z0-9 ]{1,24}") {
        prop_assume!(!haystack.trim().is_empty());
        let m = score_weighted(&[WeightedField::new(&haystack, 1.0)], &Query::new(&haystack));
        prop_assert_eq!(m.score, 100);
        prop_assert_eq!(m.quality, 100);
        prop_assert!(m.accepted());
    }
}
