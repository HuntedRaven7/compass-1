//! Tests for the production ranking path: the [`FuzzySearchable`] trait, the
//! quality gate, determinism, and plain ranking sanity on realistic app names.

use compass_search::{FuzzySearchable, Query, WeightedField, rank, rank_indices, score_item};

/// A launcher entry: name matters most, keywords less, description least.
struct App {
    name: &'static str,
    keywords: &'static str,
    description: &'static str,
}

impl App {
    const fn new(name: &'static str, keywords: &'static str, description: &'static str) -> Self {
        Self {
            name,
            keywords,
            description,
        }
    }
}

impl FuzzySearchable for App {
    fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
        out.push(WeightedField::new(self.name, 1.0));
        out.push(WeightedField::new(self.keywords, 0.7));
        out.push(WeightedField::new(self.description, 0.4));
    }
}

const APPS: &[App] = &[
    App::new(
        "Firefox",
        "web browser internet",
        "Browse the World Wide Web",
    ),
    App::new(
        "File Manager",
        "files nautilus",
        "Access and organize files",
    ),
    App::new(
        "Visual Studio Code",
        "vscode editor",
        "Code editing. Redefined.",
    ),
    App::new("Thunderbird", "mail email", "Send and receive mail"),
    App::new("Calculator", "math", "Perform arithmetic"),
    App::new("Konsole", "terminal shell", "Terminal emulator"),
    App::new("Steam", "games", "Play this game on Steam"),
];

fn names(query: &str) -> Vec<&'static str> {
    rank(query, APPS).into_iter().map(|s| s.item.name).collect()
}

#[track_caller]
fn assert_ranks_above(query: &str, winner: &str, loser: &str) {
    let ranked = names(query);
    let win = ranked.iter().position(|n| *n == winner);
    let lose = ranked.iter().position(|n| *n == loser);
    let win =
        win.unwrap_or_else(|| panic!("{winner:?} missing from ranking for {query:?}: {ranked:?}"));
    // `lose == None` means the loser was filtered out entirely: an even
    // stronger result than ranking below the winner.
    if let Some(lose) = lose {
        assert!(
            win < lose,
            "expected {winner:?} above {loser:?} for {query:?}, got {ranked:?}"
        );
    }
}

#[test]
fn ranks_the_obvious_winner_first() {
    assert_eq!(names("fir").first(), Some(&"Firefox"));
    assert_eq!(names("firefox").first(), Some(&"Firefox"));
    assert_eq!(names("calc").first(), Some(&"Calculator"));
    assert_eq!(names("thund").first(), Some(&"Thunderbird"));
    assert_eq!(names("konsole").first(), Some(&"Konsole"));
    assert_eq!(names("code").first(), Some(&"Visual Studio Code"));
    assert_eq!(names("vsc").first(), Some(&"Visual Studio Code"));
}

#[test]
fn ranks_prefix_matches_above_scattered_ones() {
    assert_ranks_above("fir", "Firefox", "File Manager");
    assert_ranks_above("file", "File Manager", "Firefox");
    assert_ranks_above("mail", "Thunderbird", "File Manager");
    assert_ranks_above("term", "Konsole", "Visual Studio Code");
}

#[test]
fn a_name_match_outranks_a_description_match() {
    // "Steam" is in Steam's name and in nothing else; "game" is in its
    // keywords and description only, so it must still be found but score less.
    let by_name = score_item(&APPS[6], &Query::new("steam"));
    let by_keyword = score_item(&APPS[6], &Query::new("games"));
    assert!(by_name.accepted() && by_keyword.accepted());
    assert!(
        by_name.score > by_keyword.score,
        "name match {by_name:?} should beat keyword match {by_keyword:?}"
    );
}

#[test]
fn non_matches_are_filtered_out() {
    assert!(names("zzzz").is_empty());
    assert!(!names("fir").contains(&"Calculator"));
    assert!(!names("calc").contains(&"Konsole"));
}

#[test]
fn empty_query_keeps_everything_in_input_order() {
    let ranked = names("");
    assert_eq!(ranked.len(), APPS.len());
    assert_eq!(ranked, APPS.iter().map(|a| a.name).collect::<Vec<_>>());

    // Whitespace-only is the same as empty.
    assert_eq!(names("   "), ranked);
}

#[test]
fn ranking_is_deterministic_across_runs() {
    for query in ["e", "a", "s", "co", "man", "set", "fire", "term shell", ""] {
        let first = rank_indices(query, APPS);
        for _ in 0..8 {
            assert_eq!(
                rank_indices(query, APPS),
                first,
                "ranking for {query:?} is not deterministic"
            );
        }
    }
}

#[test]
fn equal_scores_keep_input_order() {
    // Two entries that are indistinguishable to the matcher must come back in
    // input order, and stay there when the input order is reversed.
    let items = ["Clipboard History", "Clear Clipboard History"];
    let forward: Vec<&str> = rank("clip", &items).into_iter().map(|s| *s.item).collect();
    assert_eq!(
        forward,
        vec!["Clipboard History", "Clear Clipboard History"]
    );

    let reversed = ["Clear Clipboard History", "Clipboard History"];
    let backward: Vec<&str> = rank("clip", &reversed)
        .into_iter()
        .map(|s| *s.item)
        .collect();
    assert_eq!(
        backward,
        vec!["Clear Clipboard History", "Clipboard History"]
    );
}

#[test]
fn ranking_is_sorted_descending_and_indices_are_valid() {
    let ranked = rank_indices("e", APPS);
    assert!(!ranked.is_empty());
    for pair in ranked.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(
            (a.score, a.weighted) >= (b.score, b.weighted)
                || (a.score == b.score && a.weighted == b.weighted && a.index < b.index),
            "not sorted: {a:?} then {b:?}"
        );
    }
    for scored in &ranked {
        assert!(scored.index < APPS.len());
        assert_eq!(scored.item, scored.index);
    }
}

#[test]
fn ranking_over_plain_strings_works() {
    let items = vec![
        "Firefox".to_string(),
        "File Manager".to_string(),
        "Thunderbird".to_string(),
    ];
    let ranked: Vec<&str> = rank("fir", &items)
        .into_iter()
        .map(|s| s.item.as_str())
        .collect();
    assert_eq!(ranked.first(), Some(&"Firefox"));
    assert!(!ranked.contains(&"Thunderbird"));
}

#[test]
fn field_weights_change_the_ranking() {
    struct Heavy(&'static str);
    struct Light(&'static str);
    impl FuzzySearchable for Heavy {
        fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
            out.push(WeightedField::new(self.0, 1.0));
        }
    }
    impl FuzzySearchable for Light {
        fn fuzzy_fields<'a>(&'a self, out: &mut Vec<WeightedField<'a>>) {
            out.push(WeightedField::new(self.0, 0.5));
        }
    }

    let query = Query::new("browser");
    let heavy = score_item(&Heavy("Web Browser"), &query);
    let light = score_item(&Light("Web Browser"), &query);
    assert_eq!(heavy.score, 100);
    assert_eq!(light.score, 50);
    assert!(heavy.weighted > light.weighted);
}
