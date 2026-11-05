//! The privacy-preserving `compass fs stats` report.

use std::fmt::Write as _;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use compass_db::index_stats::{IndexStats, Measurement};
use serde_json::{Value, json};

use crate::{EXIT_FAILURE, EXIT_OK};

const NOTES: &str = "File sizes are logical bytes observed separately and may change during a \
scan. SQLite counts share a read snapshot including committed WAL pages. In-use bytes are every \
page but the free ones and include the FTS bytes; reclaimable bytes are free pages SQLite reuses \
before growing the file, which only VACUUM returns. FTS bytes exclude indexed_file content and \
WAL overhead. Missing files and unavailable metrics are not zero.";

/// Reads the cache without starting the indexer and prints the measurements.
///
/// # Errors
///
/// Returns an error if the cache directory cannot be resolved, the worker
/// cannot finish, or the report cannot be serialized. Missing/unreadable
/// databases instead produce a structured report and exit code 1.
pub async fn run(as_json: bool) -> Result<ExitCode> {
    let cache =
        compass_core::xdg_dirs::cache_home().context("cannot resolve the cache directory")?;
    let path = crate::indexer_service::database_path(&cache);
    let stats = tokio::task::spawn_blocking(move || compass_db::index_stats::inspect(&path))
        .await
        .context("reading file-index measurements")?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&json_report(&stats))?);
    } else {
        print!("{}", human_report(&stats));
    }
    Ok(ExitCode::from(if stats.complete() {
        EXIT_OK
    } else {
        EXIT_FAILURE
    }))
}

fn status(stats: &IndexStats) -> &'static str {
    if stats.database_bytes == Measurement::Missing {
        "missing"
    } else if stats.complete() {
        "ok"
    } else {
        "partial"
    }
}

fn metric_json(metric: &Measurement) -> Value {
    match metric {
        Measurement::Value(value) => json!({ "status": "ok", "value": value }),
        Measurement::Missing => json!({ "status": "missing", "value": null }),
        Measurement::Unavailable(reason) => {
            json!({ "status": "unavailable", "value": null, "reason": reason })
        }
    }
}

/// [`IndexStats::live_bytes`] or [`IndexStats::reclaimable_bytes`] as a
/// measurement, unavailable when the pages were not all measured.
fn derived(bytes: Option<u64>) -> Measurement {
    bytes.map_or_else(
        || Measurement::Unavailable("page measurements unavailable".into()),
        Measurement::Value,
    )
}

fn json_report(stats: &IndexStats) -> Value {
    json!({
        "schema": 1,
        "status": status(stats),
        "database_bytes": metric_json(&stats.database_bytes),
        "wal_bytes": metric_json(&stats.wal_bytes),
        "shm_bytes": metric_json(&stats.shm_bytes),
        "page_size": metric_json(&stats.page_size),
        "page_count": metric_json(&stats.page_count),
        "freelist_count": metric_json(&stats.freelist_count),
        "indexed_rows": metric_json(&stats.indexed_rows),
        "fts_bytes": metric_json(&stats.fts_bytes),
        "live_bytes": metric_json(&derived(stats.live_bytes())),
        "reclaimable_bytes": metric_json(&derived(stats.reclaimable_bytes())),
        "notes": NOTES,
    })
}

fn human_report(stats: &IndexStats) -> String {
    let mut out = format!("File index size: {}\n", status(stats));
    let live = derived(stats.live_bytes());
    let reclaimable = derived(stats.reclaimable_bytes());
    for (label, metric) in [
        ("Database bytes", &stats.database_bytes),
        ("WAL bytes", &stats.wal_bytes),
        ("SHM bytes", &stats.shm_bytes),
        ("Page size (bytes)", &stats.page_size),
        ("Page count", &stats.page_count),
        ("Freelist pages", &stats.freelist_count),
        ("In-use bytes", &live),
        ("Reclaimable bytes", &reclaimable),
        ("Indexed rows", &stats.indexed_rows),
        ("FTS bytes (dbstat)", &stats.fts_bytes),
    ] {
        let value = match metric {
            Measurement::Value(value) => value.to_string(),
            Measurement::Missing => "missing".to_owned(),
            Measurement::Unavailable(reason) => format!("unavailable: {reason}"),
        };
        writeln!(&mut out, "  {label:<20} {value}").expect("writing to String cannot fail");
    }
    writeln!(&mut out, "\n{NOTES}").expect("writing to String cannot fail");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_preserves_unknown_values_instead_of_claiming_zero() {
        let dir = tempfile::tempdir().unwrap();
        let stats = compass_db::index_stats::inspect(&dir.path().join("absent.db"));
        let report = json_report(&stats);
        assert_eq!(report["schema"], 1);
        assert_eq!(report["status"], "missing");
        assert_eq!(report["database_bytes"]["status"], "missing");
        assert!(report["database_bytes"]["value"].is_null());
        assert_eq!(report["indexed_rows"]["status"], "unavailable");
        assert!(
            !report
                .to_string()
                .contains(&dir.path().display().to_string())
        );
    }

    #[test]
    fn human_report_names_units_and_never_prints_a_database_path() {
        let dir = tempfile::tempdir().unwrap();
        let stats = compass_db::index_stats::inspect(&dir.path().join("absent.db"));
        let report = human_report(&stats);
        assert!(report.contains("Database bytes"));
        assert!(report.contains("WAL bytes"));
        assert!(report.contains("SHM bytes"));
        assert!(report.contains("unavailable"));
        assert!(!report.contains(&dir.path().display().to_string()));
    }
}
