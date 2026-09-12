//! Launch history: recording, decay, persistence, and its effect on ranking.
//!
//! Every test drives time through [`ManualClock`]. Nothing here sleeps.

mod support;

use std::sync::Arc;

use compass_core::frecency::{FrecencyRecord, JsonFrecencyStore, STORE_VERSION};
use compass_core::{Clock, FrecencyStore, ManualClock};
use support::{app, builder, write};

const DAY: i64 = 86_400;
/// An arbitrary fixed "now": 2024-01-01T00:00:00Z.
const T0: i64 = 1_704_067_200;

fn store(clock: &Arc<ManualClock>) -> JsonFrecencyStore {
    JsonFrecencyStore::in_memory(clock.clone())
}

fn clock() -> Arc<ManualClock> {
    Arc::new(ManualClock::new(T0))
}

#[test]
fn an_unknown_key_scores_zero() {
    let store = store(&clock());
    assert_eq!(store.score("never.desktop"), 0.0);
    assert_eq!(store.record("never.desktop"), None);
}

#[test]
fn recording_a_launch_counts_it_and_stamps_the_time() {
    let clock = clock();
    let mut store = store(&clock);

    store.record_launch("firefox.desktop").unwrap();
    clock.advance(60);
    store.record_launch("firefox.desktop").unwrap();

    let record = store.record("firefox.desktop").unwrap();
    assert_eq!(record.launch_count, 2);
    assert_eq!(record.last_launched_at, Some(T0 + 60));
}

#[test]
fn more_launches_score_higher_at_the_same_instant() {
    let clock = clock();
    let mut store = store(&clock);

    for _ in 0..10 {
        store.record_launch("often.desktop").unwrap();
    }
    store.record_launch("once.desktop").unwrap();

    assert!(store.score("often.desktop") > store.score("once.desktop"));
}

#[test]
fn a_stale_item_scores_below_a_fresh_one_with_the_same_count() {
    let clock = clock();
    let mut store = store(&clock);

    store.record_launch("stale.desktop").unwrap();
    clock.advance_days(120);
    store.record_launch("fresh.desktop").unwrap();

    assert!(store.score("fresh.desktop") > store.score("stale.desktop"));
}

/// Decay must be monotone: as the clock advances and nothing is launched, a score only ever
/// falls, and it never falls below the frequency floor that the launch count alone earns.
#[test]
fn decay_is_monotonically_non_increasing() {
    let record = FrecencyRecord {
        launch_count: 12,
        last_launched_at: Some(T0),
    };

    let mut previous = f64::INFINITY;
    let mut floor_reached = false;
    for day in 0..=365 {
        let score = record.score_at(T0 + day * DAY);
        assert!(
            score <= previous + f64::EPSILON,
            "score rose on day {day}: {previous} -> {score}"
        );
        assert!((0.0..=1.0).contains(&score), "day {day} scored {score}");
        if day > 300 {
            floor_reached = true;
        }
        previous = score;
    }

    assert!(floor_reached);
    let never = FrecencyRecord {
        launch_count: 12,
        last_launched_at: None,
    };
    assert!(
        previous > never.score_at(T0),
        "a year-old launch still beats no launch at all, because the count survives"
    );
}

#[test]
fn a_clock_running_backwards_does_not_inflate_the_score() {
    let record = FrecencyRecord {
        launch_count: 3,
        last_launched_at: Some(T0),
    };

    // A launch "in the future" (NTP step, dual boot) must be treated as just-now, not as a
    // negative age that would push recency above its peak.
    assert_eq!(record.score_at(T0 - 10 * DAY), record.score_at(T0));
}

#[test]
fn forgetting_removes_the_record() {
    let clock = clock();
    let mut store = store(&clock);
    store.record_launch("gone.desktop").unwrap();

    assert!(store.forget("gone.desktop").unwrap());
    assert!(!store.forget("gone.desktop").unwrap());
    assert_eq!(store.score("gone.desktop"), 0.0);
}

// --- Persistence -------------------------------------------------------------------------------

#[test]
fn history_survives_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state").join("frecency.json");
    let clock = clock();

    {
        let mut store = JsonFrecencyStore::open_with_clock(&path, clock.clone()).unwrap();
        store.record_launch("firefox.desktop").unwrap();
        store.record_launch("firefox.desktop").unwrap();
        store.flush().unwrap();
    }

    let reopened = JsonFrecencyStore::open_with_clock(&path, clock.clone()).unwrap();
    let record = reopened.record("firefox.desktop").unwrap();
    assert_eq!(record.launch_count, 2);
    assert_eq!(record.last_launched_at, Some(T0));

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["version"], STORE_VERSION);
}

#[test]
fn a_missing_store_file_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = JsonFrecencyStore::open_with_clock(dir.path().join("nope.json"), clock()).unwrap();
    assert!(store.is_empty());
}

#[test]
fn an_empty_store_file_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frecency.json");
    std::fs::write(&path, "  \n").unwrap();

    let store = JsonFrecencyStore::open_with_clock(&path, clock()).unwrap();
    assert!(store.is_empty());
}

#[test]
fn a_corrupt_store_file_is_an_error_that_names_the_position() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frecency.json");
    std::fs::write(&path, "{\n  \"entries\": [\n").unwrap();

    let err = JsonFrecencyStore::open_with_clock(&path, clock()).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("invalid frecency store"), "{message}");
    assert!(message.contains("line 2"), "{message}");
}

#[test]
fn an_in_memory_store_writes_nothing() {
    let clock = clock();
    let mut store = JsonFrecencyStore::in_memory(clock);
    store.record_launch("x").unwrap();
    store.flush().unwrap();
    assert_eq!(store.path(), None);
    assert_eq!(store.len(), 1);
}

// --- Ranking -----------------------------------------------------------------------------------

#[test]
fn launch_history_breaks_a_tie_between_equally_good_matches() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "firefox.desktop", &app("Firefox", "firefox"));
    write(dir.path(), "firewall.desktop", &app("Firewall", "firewall"));
    let index = builder().dir(dir.path()).build();

    let clock = clock();
    let mut history = store(&clock);
    for _ in 0..5 {
        history.record_launch("firefox.desktop").unwrap();
    }

    let ranked = index.search_with_frecency("fir", &history);
    assert_eq!(ranked[0].item.name(), "Firefox");
    assert!(ranked[0].frecency > 0.0);
    assert_eq!(ranked[1].frecency, 0.0);
}

#[test]
fn a_recently_and_often_launched_item_outranks_a_stale_one() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "editor-a.desktop", &app("Editor Alpha", "a"));
    write(dir.path(), "editor-b.desktop", &app("Editor Beta", "b"));
    let index = builder().dir(dir.path()).build();

    let clock = clock();
    let mut history = store(&clock);

    // Beta was used heavily a year ago and never since.
    for _ in 0..40 {
        history.record_launch("editor-b.desktop").unwrap();
    }
    clock.advance_days(365);

    // Alpha is used every day this week.
    for _ in 0..7 {
        history.record_launch("editor-a.desktop").unwrap();
        clock.advance_days(1);
    }

    let ranked = index.search_with_frecency("editor", &history);
    assert_eq!(
        ranked[0].item.name(),
        "Editor Alpha",
        "ranked: {:?}",
        ranked
            .iter()
            .map(|r| (r.item.name(), r.score, r.frecency))
            .collect::<Vec<_>>()
    );
}

#[test]
fn frecency_never_resurrects_a_non_matching_item() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "firefox.desktop", &app("Firefox", "firefox"));
    write(dir.path(), "gimp.desktop", &app("GIMP", "gimp"));
    let index = builder().dir(dir.path()).build();

    let clock = clock();
    let mut history = store(&clock);
    for _ in 0..500 {
        history.record_launch("gimp.desktop").unwrap();
    }

    let ranked = index.search_with_frecency("firefox", &history);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].item.name(), "Firefox");
}

#[test]
fn an_empty_query_orders_purely_by_history() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.desktop", &app("Alpha", "a"));
    write(dir.path(), "b.desktop", &app("Beta", "b"));
    write(dir.path(), "c.desktop", &app("Gamma", "c"));
    let index = builder().dir(dir.path()).build();

    let clock = clock();
    let mut history = store(&clock);
    for _ in 0..3 {
        history.record_launch("c.desktop").unwrap();
    }
    history.record_launch("b.desktop").unwrap();

    let ranked = index.search_with_frecency("", &history);
    let names: Vec<&str> = ranked.iter().map(|r| r.item.name()).collect();
    assert_eq!(names, ["Gamma", "Beta", "Alpha"]);
}

#[test]
fn ranking_with_frecency_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..20 {
        write(
            dir.path(),
            &format!("app{i}.desktop"),
            &app(&format!("Application {i}"), "run"),
        );
    }
    let index = builder().dir(dir.path()).build();
    let history = store(&clock());

    let once: Vec<&str> = index
        .search_with_frecency("app", &history)
        .iter()
        .map(|r| r.item.key())
        .collect();
    let twice: Vec<&str> = index
        .search_with_frecency("app", &history)
        .iter()
        .map(|r| r.item.key())
        .collect();
    assert_eq!(once, twice);
}

#[test]
fn the_manual_clock_is_the_only_source_of_time_in_these_tests() {
    let clock = ManualClock::new(0);
    assert_eq!(clock.now_unix(), 0);
    clock.advance_days(2);
    assert_eq!(clock.now_unix(), 2 * DAY);
    clock.set(T0);
    assert_eq!(clock.now_unix(), T0);
}
