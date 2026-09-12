//! Properties that must hold for arbitrary input.
//!
//! The scanner reads files nobody in this project wrote: distribution packages, hand-edited
//! overrides, half-written files from a crashed installer. None of it may panic the launcher.

use std::collections::HashSet;

use compass_core::{AppIndex, Config, FrecencyRecord, JsonFrecencyStore};
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

fn index_of(files: &[(String, Vec<u8>)]) -> (tempfile::TempDir, AppIndex) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, bytes) in files {
        let _ = std::fs::write(dir.path().join(name), bytes);
    }
    let index = AppIndex::builder()
        .dir(dir.path())
        .desktops(["GNOME"])
        .locale(compass_xdg::Locale::parse("C"))
        .build();
    (dir, index)
}

/// Filenames that are safe to create on any filesystem, so the property under test is the
/// *contents*, not the path.
fn file_name() -> impl Strategy<Value = String> {
    "[a-z0-9._-]{1,20}".prop_map(|stem| format!("{stem}.desktop"))
}

fn file_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Arbitrary bytes, including invalid UTF-8.
        proptest::collection::vec(any::<u8>(), 0..512),
        // Text that looks vaguely like a desktop entry, to reach deeper into the parser.
        proptest::collection::vec(r"(\[[A-Za-z ]{0,20}\]|[A-Za-z\[\]%\\;=@ ]{0,40})", 0..20,)
            .prop_map(|lines: Vec<String>| lines.join("\n").into_bytes()),
    ]
}

proptest! {
    #![proptest_config(regressions(256, "tests/regressions/properties.txt"))]

    /// Indexing arbitrary file contents never panics, and every file is accounted for.
    #[test]
    fn indexing_arbitrary_files_never_panics(
        files in proptest::collection::vec((file_name(), file_bytes()), 0..8)
    ) {
        let unique: HashSet<&String> = files.iter().map(|(name, _)| name).collect();
        let (_dir, index) = index_of(&files);

        // Applications plus skips accounts for every distinct file written; actions are extra.
        let applications = index.applications().count();
        prop_assert!(applications + index.skipped().len() >= unique.len());

        // Nothing indexed may have an empty key or a key that does not resolve.
        for item in index.items() {
            prop_assert!(!item.key().is_empty());
            prop_assert!(index.get(item.key()).is_some());
        }
    }

    /// Searching an arbitrary index with an arbitrary query never panics and stays ordered.
    #[test]
    fn searching_an_arbitrary_index_never_panics(
        files in proptest::collection::vec((file_name(), file_bytes()), 0..6),
        query in "[ -~]{0,24}",
    ) {
        let (_dir, index) = index_of(&files);
        let hits = index.search(&query);

        prop_assert!(hits.len() <= index.len());
        for pair in hits.windows(2) {
            prop_assert!((pair[0].score, pair[1].index) >= (pair[1].score, pair[0].index));
        }

        let history = JsonFrecencyStore::in_memory(std::sync::Arc::new(
            compass_core::ManualClock::new(1_704_067_200),
        ));
        let ranked = index.search_with_frecency(&query, &history);
        prop_assert_eq!(ranked.len(), hits.len());
        for pair in ranked.windows(2) {
            prop_assert!(pair[0].score >= pair[1].score);
        }
    }

    /// Parsing arbitrary bytes as a config never panics: it is either a config or an error.
    #[test]
    fn parsing_arbitrary_config_text_never_panics(text in "\\PC{0,200}") {
        let _ = Config::parse(&text, std::path::Path::new("/test/vicinae.json"));
    }

    /// Any config that parses round-trips byte-for-byte through JSON.
    #[test]
    fn a_parsed_config_round_trips(
        hotkey in proptest::option::of("[a-z+]{1,12}"),
        max_results in proptest::option::of(0usize..1000),
        extra_key in "[a-z_]{1,10}",
        extra_value in 0i64..1000,
    ) {
        let mut object = serde_json::Map::new();
        let mut launcher = serde_json::Map::new();
        if let Some(hotkey) = &hotkey {
            launcher.insert("hotkey".into(), hotkey.clone().into());
        }
        if let Some(max) = max_results {
            launcher.insert("max_results".into(), max.into());
        }
        launcher.insert(extra_key.clone(), extra_value.into());
        object.insert("launcher".into(), launcher.into());

        let original = serde_json::Value::Object(object);
        let config = Config::parse(&original.to_string(), std::path::Path::new("/x")).unwrap();
        let rewritten: serde_json::Value =
            serde_json::from_str(&config.to_json_pretty().unwrap()).unwrap();

        prop_assert_eq!(&rewritten, &original);
        prop_assert_eq!(&config.launcher().unknown_fields()[&extra_key], &serde_json::json!(extra_value));
    }

    /// Frecency is bounded and monotone in time for any record.
    #[test]
    fn frecency_is_bounded_and_decays(
        launch_count in 0u32..10_000,
        age_days in 0i64..3650,
    ) {
        let now = 1_704_067_200i64;
        let record = FrecencyRecord {
            launch_count,
            last_launched_at: Some(now - age_days * 86_400),
        };

        let score = record.score_at(now);
        prop_assert!((0.0..=1.0).contains(&score), "{score}");

        let older = FrecencyRecord {
            launch_count,
            last_launched_at: Some(now - (age_days + 1) * 86_400),
        };
        prop_assert!(older.score_at(now) <= score + f64::EPSILON);
    }
}
