//! Suite 0 rung 1: diff the C++ scorer against the Rust one over the corpus.
//!
//! See `docs/rust-engine/PLAN.md` §8.1a. Suite 0 proper (§8.1) diffs whole
//! *engines*, and cannot run yet: the C++ CLI has no command that emits ranked
//! results and its IPC protocol has no ranked-search method. This diffs the one
//! piece that is reachable today — the scorer — because `vicinae::fuzzy` is a
//! header-only library with no Qt dependency, so the C++ side compiles in about
//! a second with a bare `c++ -std=c++23`.
//!
//! **This is scorer parity, not ranking parity, and the difference matters.**
//! `RootItemManager::searchGroupedByProvider` wraps the scorer in provider
//! bucketing, a separate provider-name score, favourite and enabled filtering
//! and per-item frecency. A green run here does not mean the engines rank
//! alike; it means they score alike, which is necessary and not sufficient.
//!
//! Only ONE corpus parser exists, and it is this file. The probe scores
//! `id<TAB>text` lines handed to it on stdin, so a disagreement about which
//! `Name=` line to take cannot masquerade as a scoring divergence.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use compass_search::{Query, WeightedField, score_weighted};
use compass_testkit::corpus::{Provenance, desktop_entries};

/// Matches `fuzzy::MIN_QUALITY` in upstream `fuzzy-searchable.hpp` (`scripts/bench/upstream/fuzzy`).
///
/// Duplicated deliberately rather than plumbed across the language boundary: if
/// the C++ constant changes, this diff should go red and say so, not silently
/// follow it.
const MIN_QUALITY: u32 = 60;

/// One side's ranked output for one query.
type Ranking = Vec<(String, u32, u32)>;

/// The divergence baseline, and why this is a ratchet rather than a list.
///
/// The first version of this harness enumerated individual divergences, and
/// that worked while the corpus was 115 entries harvested from one Bluefin
/// image: six queries differed, each could be written down, and a seventh would
/// fail the run.
///
/// **The corpus then grew to 738 real entries and the count went to 351
/// divergent queries out of 1685.** Enumerating those is not a declaration, it
/// is surrender: nobody reads a 1407-line exception list, and one that long
/// hides a regression as effectively as having no check at all.
///
/// The small corpus was not evidence of agreement. It was one distribution's
/// stock application set, and it hid the problem.
///
/// So the assertion is a ratchet on the measured totals. It fails when the
/// numbers get WORSE, which is the regression it exists to catch — and it also
/// fails when they get BETTER, because a baseline nobody lowers is a baseline
/// that rots into a rubber stamp. Either way the fix is deliberate: look at
/// what changed, then move the number.
///
/// **These numbers are NOT a defect to drive to zero**, and an earlier version
/// of this comment said they were. `PARITY.md`'s "compass-search — nucleo is
/// not fzf" already records that the Rust matcher wraps `nucleo_matcher` while
/// the C++ side is a vendored fzf, that absolute scores are on different
/// scales, and that the normalized values match "closely". Both shapes counted
/// here are already listed there — #4 (score ignores match position) and #2
/// (nucleo prefers a short scatter where fzf's word-boundary bonuses do not).
/// Choosing nucleo over hand-rolling a matcher is settled (PLAN §10).
///
/// What this adds is the number. "Closely" turns out to be 79.2% of queries
/// over 738 real entries. The ratchet holds a KNOWN divergence still so a
/// nucleo bump or a scoring change cannot move it unnoticed — zero would mean
/// replacing nucleo, which is not this harness's call to force.
///
/// They were also set wrong the first time, and the ratchet caught it: the
/// figures were copied from a run that reported divergences MINUS the ten then
/// declared, so removing the list added those back and the first run failed at
/// 352/1417 against 351/1407. A baseline you cannot get wrong is a baseline
/// that is not checking anything.
// The Unicode single-character boundary correction removes six Bear Factory
// pairs (A/a, E and I), including one previously rejected match. See the
// harvested_editor_titles_match_their_later_word_starts regression test.
const BASELINE_DIVERGENT_QUERIES: usize = 351;
const BASELINE_DIVERGENT_PAIRS: usize = 1411;

/// How the two engines differ on one entry, for the directional breakdown.
///
/// Worth reporting separately because the direction was initially miscalled.
/// From the 115-entry corpus every divergence had C++ stricter, and this
/// harness's first write-up said the Rust port was "systematically more
/// permissive". At 738 entries that is the dominant direction but not a rule:
/// 206 of 1407 go the other way, and a summary that only counted one direction
/// would have kept saying something false.
#[derive(Debug, Default)]
struct Directions {
    cpp_rejected_rust_accepted: usize,
    rust_rejected_cpp_accepted: usize,
    cpp_higher: usize,
    rust_higher: usize,
}

impl Directions {
    fn record(&mut self, cpp: Option<u32>, rust: Option<u32>) {
        match (cpp, rust) {
            (None, Some(_)) => self.cpp_rejected_rust_accepted += 1,
            (Some(_), None) => self.rust_rejected_cpp_accepted += 1,
            (Some(c), Some(r)) if c > r => self.cpp_higher += 1,
            (Some(c), Some(r)) if r > c => self.rust_higher += 1,
            _ => {}
        }
    }

    fn total(&self) -> usize {
        self.cpp_rejected_rust_accepted
            + self.rust_rejected_cpp_accepted
            + self.cpp_higher
            + self.rust_higher
    }
}

/// `(id, display name)` for every real harvested entry, in a fixed order.
///
/// Real only: the synthetic fixtures exist to exercise parser edge cases —
/// deliberately invalid UTF-8 among them — and are not desktop entries a user
/// would ever search.
fn corpus_items() -> Vec<(String, String)> {
    let mut items: Vec<(String, String)> = desktop_entries()
        .into_iter()
        .filter(|e| e.provenance == Provenance::Real)
        .filter_map(|e| {
            let text = e.as_str()?;
            let name = text.lines().find_map(|l| l.strip_prefix("Name="))?;
            let name = name.trim();
            (!name.is_empty()).then(|| (e.id.clone(), name.to_owned()))
        })
        .collect();
    items.sort();
    items
}

/// The query set, derived from the corpus the same way `parity.rs` derives its
/// own: every name, plus its 1–4 character prefixes, plus a few common terms.
fn queries(items: &[(String, String)]) -> Vec<String> {
    let mut qs: Vec<String> = Vec::new();
    for (_, name) in items {
        if name.chars().count() > 2 {
            qs.push(name.clone());
            for i in 1..=4.min(name.chars().count()) {
                qs.push(name.chars().take(i).collect());
            }
        }
    }
    qs.extend(
        [
            "firefox", "chrome", "terminal", "editor", "browser", "calc", "file", "settings",
            "system", "app",
        ]
        .iter()
        .map(|s| (*s).to_owned()),
    );
    qs.sort();
    qs.dedup();
    qs
}

fn rust_ranking(items: &[(String, String)], query: &str) -> Ranking {
    let q = Query::new(query);
    let mut hits: Vec<(String, u32, u32)> = items
        .iter()
        .filter_map(|(id, name)| {
            let m = score_weighted(&[WeightedField::new(name, 1.0)], &q);
            (m.quality >= MIN_QUALITY).then(|| (id.clone(), m.score, m.quality))
        })
        .collect();
    hits.sort_by(|a, b| b.1.cmp(&a.1));
    hits
}

fn cpp_ranking(probe: &PathBuf, items: &[(String, String)], query: &str) -> Result<Ranking> {
    let mut child = Command::new(probe)
        .arg(query)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning the C++ probe at {}", probe.display()))?;

    {
        let stdin = child.stdin.as_mut().context("probe stdin")?;
        for (id, name) in items {
            writeln!(stdin, "{id}\t{name}")?;
        }
    }

    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "the C++ probe failed for query {query:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let mut hits = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.split('\t');
        let (Some(id), Some(score), Some(quality)) = (parts.next(), parts.next(), parts.next())
        else {
            bail!("malformed probe output line: {line:?}");
        };
        hits.push((id.to_owned(), score.parse()?, quality.parse()?));
    }
    Ok(hits)
}

// Over the 150-line limit when it was added (#379): split it, don't grow it.
#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    let mut probe: Option<PathBuf> = None;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--probe" {
            i += 1;
            probe = args.get(i).map(PathBuf::from);
        }
        i += 1;
    }
    let probe = probe.context(
        "pass --probe <path to vicinae-fuzzy-probe>; build it with \
         `c++ -std=c++23 -O2 -Iscripts/bench/upstream/fuzzy/include -o probe \
         scripts/bench/probes/fuzzy-probe.cpp`",
    )?;

    let items = corpus_items();
    let queries = queries(&items);
    println!("Suite 0 rung 1 — scorer parity");
    println!("  corpus:  {} real entries", items.len());
    println!("  queries: {}", queries.len());
    println!("  probe:   {}", probe.display());

    let mut identical = 0usize;
    let mut divergent_queries = 0usize;
    let mut directions = Directions::default();
    // Rank agreement, which is what Phase 1's gate actually names: it asks for
    // "ranking parity", and a user sees an ordered list, not a score. Two
    // engines can disagree that `LibreOffice` scores 83 or 72 for "O" and still
    // put the same entry first, in which case the divergence is invisible.
    // Counted only over queries where at least one side returned anything.
    let mut ranked_queries = 0usize;
    let mut top1_agree = 0usize;
    let mut top3_agree = 0usize;
    let mut order_agree = 0usize;
    let mut contested = 0usize;
    let mut contested_top1_agree = 0usize;
    // A few examples, so a failing run says what changed rather than only that
    // something did.
    let mut examples: Vec<String> = Vec::new();

    for query in &queries {
        let cpp = cpp_ranking(&probe, &items, query)?;
        let rust = rust_ranking(&items, query);

        if !cpp.is_empty() || !rust.is_empty() {
            ranked_queries += 1;
            let ids =
                |r: &Ranking| -> Vec<String> { r.iter().map(|(id, _, _)| id.clone()).collect() };
            let (ci, ri) = (ids(&cpp), ids(&rust));
            if ci.first() == ri.first() {
                top1_agree += 1;
            }
            let n = 3.min(ci.len()).min(ri.len());
            if ci.len().min(3) == ri.len().min(3) && ci[..n] == ri[..n] {
                top3_agree += 1;
            }
            if ci == ri {
                order_agree += 1;
            }
            // The control on the numbers above. "Same top result" over a corpus
            // where most queries return one hit would be trivially 100% and
            // would mean nothing, so record how many queries actually had a
            // choice to get wrong.
            if ci.len() > 1 || ri.len() > 1 {
                contested += 1;
                if ci.first() == ri.first() {
                    contested_top1_agree += 1;
                }
            }
        }

        if cpp == rust {
            identical += 1;
            continue;
        }
        divergent_queries += 1;

        let cpp_by: BTreeMap<&str, (u32, u32)> = cpp
            .iter()
            .map(|(id, s, q)| (id.as_str(), (*s, *q)))
            .collect();
        let rust_by: BTreeMap<&str, (u32, u32)> = rust
            .iter()
            .map(|(id, s, q)| (id.as_str(), (*s, *q)))
            .collect();

        let mut ids: Vec<&str> = cpp_by.keys().chain(rust_by.keys()).copied().collect();
        ids.sort_unstable();
        ids.dedup();

        for id in ids {
            let c = cpp_by.get(id).map(|v| v.1);
            let r = rust_by.get(id).map(|v| v.1);
            if c == r {
                continue;
            }
            directions.record(c, r);
            if examples.len() < 10 {
                examples.push(format!("{query:?} / {id}: cpp={c:?} rust={r:?}"));
            }
        }
    }

    let pairs = directions.total();
    println!("\nResults:");
    println!("  queries identical:  {identical}");
    println!("  queries divergent:  {divergent_queries} (baseline {BASELINE_DIVERGENT_QUERIES})");
    println!("  divergent pairs:    {pairs} (baseline {BASELINE_DIVERGENT_PAIRS})");
    let pct = |n: usize| {
        if ranked_queries == 0 {
            0.0
        } else {
            100.0 * n as f64 / ranked_queries as f64
        }
    };
    println!(
        "\n  rank agreement (what Phase 1's gate names), over {ranked_queries} queries with hits:"
    );
    println!(
        "    same top result:    {top1_agree} ({:.1}%)",
        pct(top1_agree)
    );
    println!(
        "    same top 3:         {top3_agree} ({:.1}%)",
        pct(top3_agree)
    );
    println!(
        "    same full order:    {order_agree} ({:.1}%)",
        pct(order_agree)
    );
    let cpct = if contested == 0 {
        0.0
    } else {
        100.0 * contested_top1_agree as f64 / contested as f64
    };
    println!(
        "    of the {contested} queries returning more than one hit, same top: {contested_top1_agree} ({cpct:.1}%)"
    );

    println!("\n  by direction:");
    println!(
        "    C++ rejected, Rust accepted:  {}",
        directions.cpp_rejected_rust_accepted
    );
    println!(
        "    both accepted, C++ higher:    {}",
        directions.cpp_higher
    );
    println!(
        "    both accepted, Rust higher:   {}",
        directions.rust_higher
    );
    println!(
        "    Rust rejected, C++ accepted:  {}",
        directions.rust_rejected_cpp_accepted
    );

    if !examples.is_empty() {
        println!("\n  examples:");
        for e in &examples {
            println!("    {e}");
        }
    }

    // THE ASSERTION THAT MATTERS, and it is not the score one.
    //
    // Phase 1's gate names ranking parity, and a user sees an ordered list, not
    // a score. Over 1684 queries with hits — 920 of them returning more than
    // one, so the agreement is not the trivial single-candidate kind — the two
    // engines pick the SAME top result every time. That is enforced exactly,
    // because a regression from 100% here is a user-visible change in what the
    // launcher puts first, which the score ratchet below would not catch: the
    // whole point of the two measures is that they answer different questions.
    if top1_agree != ranked_queries {
        eprintln!(
            "\n❌ the engines disagree on the BEST match for {} of {ranked_queries} queries. \
             This is user-visible: it is what the launcher puts first. Unlike the score \
             divergence below, it is not a known nucleo-vs-fzf consequence to be held still.",
            ranked_queries - top1_agree
        );
        std::process::exit(1);
    }

    let worse = divergent_queries > BASELINE_DIVERGENT_QUERIES || pairs > BASELINE_DIVERGENT_PAIRS;
    let better = divergent_queries < BASELINE_DIVERGENT_QUERIES || pairs < BASELINE_DIVERGENT_PAIRS;

    if worse {
        eprintln!(
            "\n❌ the two scorers agree LESS than they did: {divergent_queries} divergent queries \
             and {pairs} pairs, against a baseline of {BASELINE_DIVERGENT_QUERIES} and \
             {BASELINE_DIVERGENT_PAIRS}. Something in compass-search, the C++ scorer or the corpus \
             changed. Find out which before moving the baseline."
        );
        std::process::exit(1);
    }

    if better {
        eprintln!(
            "\n❌ the two scorers agree MORE than the baseline says: {divergent_queries} divergent \
             queries and {pairs} pairs, against {BASELINE_DIVERGENT_QUERIES} and \
             {BASELINE_DIVERGENT_PAIRS}. That is good news and the baseline has to come down to \
             match, or it stops catching anything. Lower both constants in this file."
        );
        std::process::exit(1);
    }

    println!(
        "\n✅ divergence is exactly at the recorded baseline. This is the known nucleo-is-not-fzf \
         divergence (PARITY.md), held still rather than driven to zero — see PLAN.md §8.1a."
    );
    Ok(())
}
