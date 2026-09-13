//! Suite 0 Parity Harness — Cross-implementation parity tests.
//!
//! This binary runs the same operations against both the C++ and Rust engines
//! and compares their JSON output to ensure they produce identical results.
//!
//! See `docs/rust-engine/PLAN.md` §8.1 for the full specification.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use compass_testkit::corpus::desktop_entries;

#[derive(Debug, Serialize, Deserialize)]
struct ParityConfig {
    /// Path to the C++ vicinae binary.
    cpp_engine: PathBuf,
    /// Path to the Rust vicinae binary.
    rust_engine: PathBuf,
    /// Corpus directory.
    corpus_dir: PathBuf,
    /// Output directory for diffs.
    output_dir: Option<PathBuf>,
}

/// One ranked hit, as an engine actually emits it.
///
/// This mirrors `compass_ipc::protocol::QueryHit` field for field, and did not
/// before: it was `{key, name, score, quality}` against an engine that emits
/// `{id, title, subtitle, score}`. Every name differed but `score`, and
/// `quality` does not exist at all, so `serde_json::from_str` would have failed
/// on every query — the fourth thing wrong with a harness written to §8.1's
/// prose and never run against a binary.
///
/// Deliberately NOT `compass_ipc::QueryHit` itself. Suite 0's whole job is to
/// notice when the two engines disagree, and sharing the Rust engine's own type
/// would make the harness track it silently through a rename. A separate
/// definition means a shape change shows up here as a parse failure, which is
/// the outcome that gets looked at.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SearchResultItem {
    id: String,
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    score: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ParityReport {
    total: usize,
    identical: usize,
    known_divergence: usize,
    regression: usize,
    details: Vec<ParityDetail>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ParityDetail {
    query: String,
    status: ParityStatus,
    cpp_results: Vec<SearchResultItem>,
    rust_results: Vec<SearchResultItem>,
    divergence_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
enum ParityStatus {
    Identical,
    KnownDivergence,
    Regression,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut config = ParityConfig {
        cpp_engine: PathBuf::from("./build/bin/vicinae"),
        rust_engine: PathBuf::from("./target/release/vicinae"),
        corpus_dir: PathBuf::from("crates/compass-testkit/corpus/desktop-entries"),
        output_dir: None,
    };

    // Parse command line args
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--cpp" => {
                i += 1;
                config.cpp_engine = PathBuf::from(&args[i]);
            }
            "--rust" => {
                i += 1;
                config.rust_engine = PathBuf::from(&args[i]);
            }
            "--corpus" => {
                i += 1;
                config.corpus_dir = PathBuf::from(&args[i]);
            }
            "--output" => {
                i += 1;
                config.output_dir = Some(PathBuf::from(&args[i]));
            }
            _ => {}
        }
        i += 1;
    }

    println!("Suite 0 Parity Harness");
    println!("  C++ engine:  {}", config.cpp_engine.display());
    println!("  Rust engine: {}", config.rust_engine.display());
    println!("  Corpus:      {}", config.corpus_dir.display());

    let report = run_parity(&config)?;

    println!("\nResults:");
    println!("  Total queries:     {}", report.total);
    println!("  Identical:         {}", report.identical);
    println!("  Known divergence:  {}", report.known_divergence);
    println!("  Regressions:       {}", report.regression);

    if report.regression > 0 {
        eprintln!("\n❌ REGRESSIONS DETECTED!");
        for detail in &report.details {
            if detail.status == ParityStatus::Regression {
                eprintln!("  Query: {}", detail.query);
                eprintln!("    C++:  {:?}", detail.cpp_results);
                eprintln!("    Rust: {:?}", detail.rust_results);
            }
        }
        std::process::exit(1);
    } else {
        println!("\n✅ All checks passed");
    }

    // Write report if output dir specified
    if let Some(output_dir) = config.output_dir {
        std::fs::create_dir_all(&output_dir)?;
        let report_path = output_dir.join("parity-report.json");
        std::fs::write(report_path, serde_json::to_string_pretty(&report)?)?;
        println!("Report written to: {}", output_dir.display());
    }

    Ok(())
}

fn run_parity(config: &ParityConfig) -> Result<ParityReport> {
    // Generate test queries from corpus
    let queries = generate_test_queries(&config.corpus_dir)?;

    let mut report = ParityReport {
        total: 0,
        identical: 0,
        known_divergence: 0,
        regression: 0,
        details: Vec::new(),
    };

    // One engine each, started once and reused for every query. Starting them
    // per query would make the run time dominated by index builds and would say
    // nothing extra: Suite 0 compares ranking, not startup.
    let cpp = RunningEngine::start(&config.cpp_engine, "cpp")?;
    let rust = RunningEngine::start(&config.rust_engine, "rust")?;

    for query in &queries {
        report.total += 1;

        let cpp_results = run_search(&cpp, "cpp", query)?;
        let rust_results = run_search(&rust, "rust", query)?;

        let (status, reason) = compare_results(&cpp_results, &rust_results);

        match status {
            ParityStatus::Identical => report.identical += 1,
            ParityStatus::KnownDivergence => report.known_divergence += 1,
            ParityStatus::Regression => report.regression += 1,
        }

        report.details.push(ParityDetail {
            query: query.clone(),
            status,
            cpp_results,
            rust_results,
            divergence_reason: reason,
        });
    }

    Ok(report)
}

fn generate_test_queries(_corpus_dir: &Path) -> Result<Vec<String>> {
    let mut queries = Vec::new();

    // Add queries from the corpus
    for entry in desktop_entries() {
        if let Some(text) = entry.as_str() {
            // Extract potential search terms from the desktop entry
            for line in text.lines() {
                if let Some(name) = line.strip_prefix("Name=") {
                    let name = name.trim();
                    if !name.is_empty() && name.len() > 2 {
                        queries.push(name.to_owned());
                        // Add partial queries
                        for i in 1..=name.len().min(4) {
                            queries.push(name[..i].to_owned());
                        }
                    }
                }
            }
        }
    }

    // Add some common search terms
    queries.extend([
        "firefox".to_owned(),
        "chrome".to_owned(),
        "terminal".to_owned(),
        "editor".to_owned(),
        "browser".to_owned(),
        "calc".to_owned(),
        "file".to_owned(),
        "settings".to_owned(),
        "system".to_owned(),
        "app".to_owned(),
    ]);

    // Deduplicate and sort
    queries.sort();
    queries.dedup();

    Ok(queries)
}

/// An engine process started for the duration of a parity run.
///
/// `query` asks a RUNNING engine over its IPC socket — it is not a one-shot
/// ranking command. The harness used to exec the binary once per query with
/// nothing listening, so every call returned
///
///   error: no Compass engine is listening on /tmp/vicinae-default/ipc.sock
///
/// and Suite 0 could never have produced a single comparison. Each engine now
/// gets its own `serve` on its own socket, so the two cannot reach each other's
/// index and a stale engine from a previous run cannot be mistaken for this
/// one's.
struct RunningEngine {
    child: Child,
    binary: PathBuf,
    socket: PathBuf,
}

impl RunningEngine {
    /// Start `<binary> --socket <socket> serve` and wait until it answers.
    ///
    /// READINESS IS A STATE, NOT A MOMENT. This polls `ping` until it succeeds
    /// rather than sleeping a plausible number of seconds or waiting for the
    /// socket file to appear: the file exists from the moment the listener
    /// binds, which is before the index is built, and a sleep is only ever
    /// correct on the machine it was tuned on. ADR-0010 records the VM tier
    /// learning the same lesson the expensive way — three runs screenshotted a
    /// launcher that had not painted yet because the check waited for the
    /// process instead of the renderer.
    fn start(binary: &Path, name: &str) -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "compass-parity-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating the {name} engine's socket directory"))?;
        let socket = dir.join("ipc.sock");

        let child = Command::new(binary)
            .args(["--socket", &socket.to_string_lossy(), "serve"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning the {name} engine at {}", binary.display()))?;

        let engine = Self {
            child,
            binary: binary.to_path_buf(),
            socket,
        };
        engine.wait_until_answering(name)?;
        Ok(engine)
    }

    fn wait_until_answering(&self, name: &str) -> Result<()> {
        const LIMIT: Duration = Duration::from_secs(30);
        let started = Instant::now();

        while started.elapsed() < LIMIT {
            let ping = Command::new(&self.binary)
                .args(["--socket", &self.socket.to_string_lossy(), "ping"])
                .output();

            if matches!(&ping, Ok(out) if out.status.success()) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        anyhow::bail!(
            "the {name} engine did not answer `ping` on {} within {LIMIT:?} — it may have failed to \
             start, or it may be indexing something enormous",
            self.socket.display()
        )
    }

    fn shutdown(&mut self) {
        // Ask first: a clean shutdown lets the engine release its socket, and
        // an engine that ignores this is itself worth knowing about. Kill only
        // if asking did not work.
        let _ = Command::new(&self.binary)
            .args(["--socket", &self.socket.to_string_lossy(), "shutdown"])
            .output();

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();

        if let Some(dir) = self.socket.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl Drop for RunningEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Invoke one engine for one query.
///
/// THE ARGV HERE IS LOAD-BEARING, and it was wrong in three separate ways until
/// each was measured against a real binary rather than read off §8.1's prose.
///
/// First, `--json` came before the subcommand, which clap rejects outright:
///
/// ```text
/// error: unexpected argument '--json' found
///   tip: 'query --json' exists
/// ```
///
/// `json` is a flag on `query`, not a global.
///
/// Second, there was no running engine. `query` asks one over its IPC socket,
/// so every call returned "no Compass engine is listening". `RunningEngine`
/// above now starts one per side.
///
/// Third, it passed `--engine <name>`, and that is wrong in principle rather
/// than in spelling. Until the Phase 7 cutover (§5) THE BINARY IS THE ENGINE:
/// the Rust binary refuses `--engine cpp` by design —
///
/// ```text
/// --engine cpp was selected, but this binary is the Rust engine and
/// cannot dispatch to the C++ one
/// ```
///
/// — and the C++ binary has no such flag at all, so passing it can only turn a
/// working invocation into a failing one. Suite 0 selects an engine by choosing
/// which path to exec, which is what `--cpp` and `--rust` are for.
///
/// `crates/vicinae/src/cli.rs` pins this argv, because an argv assembled in one
/// crate and parsed in another has no compiler between the two.
fn run_search(
    engine: &RunningEngine,
    engine_name: &str,
    query: &str,
) -> Result<Vec<SearchResultItem>> {
    let output = Command::new(&engine.binary)
        .args([
            "--socket",
            &engine.socket.to_string_lossy(),
            "query",
            "--json",
            query,
        ])
        .output()
        .with_context(|| format!("Failed to run {} engine", engine_name))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{} engine failed: {}", engine_name, stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let results: Vec<SearchResultItem> = serde_json::from_str(&stdout)
        .with_context(|| format!("Failed to parse {} engine output", engine_name))?;

    Ok(results)
}

fn compare_results(
    cpp: &[SearchResultItem],
    rust: &[SearchResultItem],
) -> (ParityStatus, Option<String>) {
    if cpp == rust {
        return (ParityStatus::Identical, None);
    }

    // Check if it's a known divergence (different scoring algorithm)
    // For now, treat any difference as a regression
    // In the future, we can add known divergence patterns
    (
        ParityStatus::Regression,
        Some("Results differ - scoring algorithm or data source mismatch".to_owned()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str, score: u32) -> SearchResultItem {
        SearchResultItem {
            id: id.to_owned(),
            title: id.to_owned(),
            subtitle: None,
            score,
        }
    }

    #[test]
    fn identical_results_compare_identical() {
        let a = vec![hit("firefox", 100), hit("files", 80)];
        let b = vec![hit("firefox", 100), hit("files", 80)];
        assert_eq!(compare_results(&a, &b).0, ParityStatus::Identical);
    }

    // The control. A comparison harness that cannot report a difference is
    // worth nothing, and this one reported 378/378 identical the first time it
    // ever ran — which looks like success and is also exactly what a broken
    // comparison looks like. Each case below is a way the two engines could
    // really diverge.
    #[test]
    fn a_difference_is_a_regression() {
        let base = vec![hit("firefox", 100), hit("files", 80)];

        // Different ranking order — the thing Suite 0 exists to catch.
        let reordered = vec![hit("files", 80), hit("firefox", 100)];
        assert_eq!(
            compare_results(&base, &reordered).0,
            ParityStatus::Regression
        );

        // Same items, different score.
        let rescored = vec![hit("firefox", 99), hit("files", 80)];
        assert_eq!(
            compare_results(&base, &rescored).0,
            ParityStatus::Regression
        );

        // A hit one engine found and the other did not.
        let truncated = vec![hit("firefox", 100)];
        assert_eq!(
            compare_results(&base, &truncated).0,
            ParityStatus::Regression
        );

        // And one side empty, which is what a silently failing engine produces.
        assert_eq!(compare_results(&base, &[]).0, ParityStatus::Regression);
    }

    // The shape is the engine's, not §8.1's prose. This deserialises output in
    // `compass_ipc::protocol::QueryHit`'s spelling; the previous struct was
    // `{key, name, score, quality}` and would have failed on every query.
    #[test]
    fn the_engines_json_deserialises() {
        let json =
            r#"[{"id":"firefox.desktop","title":"Firefox","subtitle":"Web Browser","score":100}]"#;
        let parsed: Vec<SearchResultItem> = serde_json::from_str(json).expect("engine JSON parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "firefox.desktop");
        assert_eq!(parsed[0].title, "Firefox");
        assert_eq!(parsed[0].score, 100);

        // And the old shape is genuinely rejected, so this test would have
        // caught the original mismatch rather than passing either way.
        let old_shape = r#"[{"key":"firefox","name":"Firefox","score":100,"quality":1}]"#;
        assert!(
            serde_json::from_str::<Vec<SearchResultItem>>(old_shape).is_err(),
            "the pre-measurement shape must not parse, or this test proves nothing"
        );
    }
}
