//! Suite 0 Parity Harness — Cross-implementation parity tests.
//!
//! This binary runs the same operations against both the C++ and Rust engines
//! and compares their JSON output to ensure they produce identical results.
//!
//! See `docs/rust-engine/PLAN.md` §8.1 for the full specification.

use std::path::{Path, PathBuf};
use std::process::Command;

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

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SearchResultItem {
    key: String,
    name: String,
    score: u32,
    quality: u32,
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

    for query in &queries {
        report.total += 1;

        let cpp_results = run_search(&config.cpp_engine, "cpp", query)?;
        let rust_results = run_search(&config.rust_engine, "rust", query)?;

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

fn run_search(engine_path: &Path, engine_name: &str, query: &str) -> Result<Vec<SearchResultItem>> {
    let output = Command::new(engine_path)
        .args(["--engine", engine_name, "--json", "query", query])
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
