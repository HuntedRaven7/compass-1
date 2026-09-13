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

/// Matches `fuzzy::MIN_QUALITY` in `src/lib/fuzzy/include/fuzzy/fuzzy-searchable.hpp`.
///
/// Duplicated deliberately rather than plumbed across the language boundary: if
/// the C++ constant changes, this diff should go red and say so, not silently
/// follow it.
const MIN_QUALITY: u32 = 60;

/// One side's ranked output for one query.
type Ranking = Vec<(String, u32, u32)>;

/// A scoring difference we have seen, looked at, and not yet resolved.
///
/// Recorded exactly, so the list cannot rot into a blanket exemption: a
/// divergence only counts as declared if BOTH sides still produce the values
/// written here. If either moves — or a seventh appears — the run fails.
///
/// `None` means that side rejected the entry (quality below `MIN_QUALITY`).
struct Declared {
    query: &'static str,
    id: &'static str,
    cpp: Option<u32>,
    rust: Option<u32>,
    why: &'static str,
}

/// The divergences found the first time these two scorers were ever compared.
///
/// Every one is the same shape: the query matches the entry NON-CONTIGUOUSLY,
/// and the Rust port is more permissive than the C++ engine — it accepts
/// matches C++ rejects outright, and scores lower the ones both accept. The
/// control is that contiguous queries agree exactly: `Sy` ranks the System
/// entries at 100 on both sides, `Se` (S…e) drops them on C++ alone.
///
/// WHICH BEHAVIOUR IS CORRECT IS NOT DECIDED HERE. Declaring them keeps CI
/// honest about the current state and makes any NEW divergence fail loudly;
/// it is not a judgement that the port is right. These are separate from the
/// two divergences PARITY.md already declares (Latin Extended-A folding, and an
/// ordering case from upstream #946) — those are unrelated, and these are all
/// ASCII.
const DECLARED: &[Declared] = &[
    Declared {
        query: "Ac",
        id: "host--gnome-background-panel",
        cpp: None,
        rust: Some(69),
        why: "non-contiguous: A…c in Appearance",
    },
    Declared {
        query: "B",
        id: "host--ibus-setup-libbopomofo",
        cpp: Some(83),
        rust: Some(72),
        why: "non-contiguous: B inside LibBopomofo",
    },
    Declared {
        query: "O",
        id: "host--libreoffice-startcenter",
        cpp: Some(83),
        rust: Some(72),
        why: "non-contiguous: O inside LibreOffice",
    },
    Declared {
        query: "O",
        id: "host--libreoffice-xsltfilter",
        cpp: Some(83),
        rust: Some(72),
        why: "non-contiguous: O inside LibreOffice",
    },
    Declared {
        query: "P",
        id: "host--ibus-setup-libpinyin",
        cpp: Some(83),
        rust: Some(72),
        why: "non-contiguous: P inside LibPinyin",
    },
    Declared {
        query: "Py",
        id: "host--ibus-setup-libpinyin",
        cpp: Some(67),
        rust: Some(61),
        why: "non-contiguous: P…y inside LibPinyin",
    },
    Declared {
        query: "Se",
        id: "host--gnome-system-monitor-kde",
        cpp: None,
        rust: Some(75),
        why: "non-contiguous: S…e in System",
    },
    Declared {
        query: "Se",
        id: "host--org.gnome.SystemMonitor",
        cpp: None,
        rust: Some(75),
        why: "non-contiguous: S…e in System Monitor",
    },
    Declared {
        query: "Se",
        id: "host--gnome-system-panel",
        cpp: None,
        rust: Some(75),
        why: "non-contiguous: S…e in System",
    },
    Declared {
        query: "Se",
        id: "host--system-update",
        cpp: None,
        rust: Some(75),
        why: "non-contiguous: S…e in System Update",
    },
];

fn declared_for(query: &str, id: &str) -> Option<&'static Declared> {
    DECLARED.iter().find(|d| d.query == query && d.id == id)
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
         `c++ -std=c++23 -O2 -Isrc/lib/fuzzy/include -o probe src/lib/fuzzy/probe/main.cpp`",
    )?;

    let items = corpus_items();
    let queries = queries(&items);
    println!("Suite 0 rung 1 — scorer parity");
    println!("  corpus:  {} real entries", items.len());
    println!("  queries: {}", queries.len());
    println!("  probe:   {}", probe.display());

    let mut identical = 0usize;
    let mut declared_hits = 0usize;
    let mut undeclared: Vec<String> = Vec::new();
    // Which declarations actually fired, so a stale one can be reported.
    let mut seen: BTreeMap<(&str, &str), bool> =
        DECLARED.iter().map(|d| ((d.query, d.id), false)).collect();

    for query in &queries {
        let cpp = cpp_ranking(&probe, &items, query)?;
        let rust = rust_ranking(&items, query);
        if cpp == rust {
            identical += 1;
            continue;
        }

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
            let c = cpp_by.get(id).copied();
            let r = rust_by.get(id).copied();
            if c == r {
                continue;
            }
            match declared_for(query, id) {
                Some(d) if d.cpp == c.map(|v| v.1) && d.rust == r.map(|v| v.1) => {
                    declared_hits += 1;
                    seen.insert((d.query, d.id), true);
                }
                Some(d) => undeclared.push(format!(
                    "{query:?} / {id}: declared cpp={:?} rust={:?} ({}) but observed cpp={:?} rust={:?}",
                    d.cpp,
                    d.rust,
                    d.why,
                    c.map(|v| v.1),
                    r.map(|v| v.1)
                )),
                None => undeclared.push(format!(
                    "{query:?} / {id}: cpp={:?} rust={:?}",
                    c.map(|v| v.1),
                    r.map(|v| v.1)
                )),
            }
        }
    }

    println!("\nResults:");
    println!("  queries identical:    {identical}");
    println!("  declared divergences: {declared_hits}");
    println!("  undeclared:           {}", undeclared.len());

    let stale: Vec<String> = seen
        .iter()
        .filter(|(_, fired)| !**fired)
        .map(|((q, id), _)| format!("{q:?} / {id}"))
        .collect();

    if !stale.is_empty() {
        // A declaration that no longer fires is not harmless: it means the
        // behaviour changed and nobody noticed, which is the thing this exists
        // to prevent.
        eprintln!("\n❌ declared divergences that no longer occur — the list is stale:");
        for s in &stale {
            eprintln!("  {s}");
        }
    }
    if !undeclared.is_empty() {
        eprintln!("\n❌ undeclared scorer divergences:");
        for u in &undeclared {
            eprintln!("  {u}");
        }
    }
    if !stale.is_empty() || !undeclared.is_empty() {
        std::process::exit(1);
    }

    println!("\n✅ no undeclared scorer divergences");
    Ok(())
}
