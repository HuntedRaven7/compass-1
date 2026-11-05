//! How big the file-search index is, and what the size is made of.
//!
//! Part of [`super`]; see that module for the purity rule. The measurements
//! are `compass_db::index_stats::inspect`'s, read by the caller, which opens
//! the database read-only and never creates it.

use compass_db::index_stats::{IndexStats, Measurement};
use compass_ipc::{DoctorCheck, DoctorStatus};
use compass_ui::clipboard_page::format_size;

use super::check;

fn size(bytes: u64) -> String {
    format_size(i64::try_from(bytes).unwrap_or(i64::MAX))
}

/// `files.index`: the indexed entries, then the database split into what
/// holds data (and how much of that is the full-text index), what free pages
/// a `VACUUM` would return, and the write-ahead log. A database that cannot
/// be measured, or a cache directory that cannot be found (`None`), warns
/// with the reason; one not built yet is fine.
#[must_use]
pub fn file_index(stats: Option<&IndexStats>) -> DoctorCheck {
    const NAME: &str = "files.index";
    let Some(stats) = stats else {
        return check(
            NAME,
            DoctorStatus::Warn,
            "The cache directory cannot be found: neither XDG_CACHE_HOME nor HOME is set.",
        );
    };
    if stats.database_bytes == Measurement::Missing {
        return check(
            NAME,
            DoctorStatus::Ok,
            "The file index has not been built yet.",
        );
    }
    let (Measurement::Value(rows), Some(live), Some(reclaimable)) = (
        &stats.indexed_rows,
        stats.live_bytes(),
        stats.reclaimable_bytes(),
    ) else {
        let reason = [
            &stats.database_bytes,
            &stats.page_size,
            &stats.page_count,
            &stats.freelist_count,
            &stats.indexed_rows,
        ]
        .into_iter()
        .find_map(|metric| match metric {
            Measurement::Unavailable(reason) => Some(reason.as_str()),
            _ => None,
        })
        .unwrap_or("a measurement is missing");
        return check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "The file index could not be measured: {reason}. Run compass fs stats for each measurement."
            ),
        );
    };
    let entries = if *rows == 1 {
        "The file index holds 1 entry.".to_owned()
    } else {
        format!("The file index holds {rows} entries.")
    };
    let database = match &stats.fts_bytes {
        Measurement::Value(fts) => format!(
            "Its data takes {}, {} of it for full-text search, and {} more is free pages that only compacting gives back.",
            size(live),
            size(*fts),
            size(reclaimable)
        ),
        _ => format!(
            "Its data takes {}, and {} more is free pages that only compacting gives back.",
            size(live),
            size(reclaimable)
        ),
    };
    let wal = match &stats.wal_bytes {
        Measurement::Value(wal) => format!("Its write-ahead log takes {}.", size(*wal)),
        _ => "It has no write-ahead log.".to_owned(),
    };
    check(
        NAME,
        DoctorStatus::Ok,
        format!("{entries} {database} {wal}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured() -> IndexStats {
        IndexStats {
            database_bytes: Measurement::Value(40 * 4096),
            wal_bytes: Measurement::Value(8 * 1024 * 1024),
            shm_bytes: Measurement::Value(32 * 1024),
            page_size: Measurement::Value(4096),
            page_count: Measurement::Value(40),
            freelist_count: Measurement::Value(10),
            indexed_rows: Measurement::Value(1234),
            fts_bytes: Measurement::Value(12 * 4096),
        }
    }

    #[test]
    fn a_measured_index_is_split_into_data_free_pages_and_log() {
        let report = file_index(Some(&measured()));
        assert_eq!(report.status, DoctorStatus::Ok);
        assert_eq!(
            report.detail.as_deref(),
            Some(
                "The file index holds 1234 entries. Its data takes 120 KB, 48.0 KB of it for full-text search, and 40.0 KB more is free pages that only compacting gives back. Its write-ahead log takes 8.00 MB."
            )
        );
    }

    #[test]
    fn what_was_not_measured_is_left_out_rather_than_called_zero() {
        let stats = IndexStats {
            wal_bytes: Measurement::Missing,
            shm_bytes: Measurement::Missing,
            indexed_rows: Measurement::Value(1),
            fts_bytes: Measurement::Unavailable(
                "dbstat is unavailable in this SQLite build".into(),
            ),
            ..measured()
        };
        assert_eq!(
            file_index(Some(&stats)).detail.as_deref(),
            Some(
                "The file index holds 1 entry. Its data takes 120 KB, and 40.0 KB more is free pages that only compacting gives back. It has no write-ahead log."
            )
        );
    }

    #[test]
    fn a_missing_index_is_fine_and_a_busy_one_warns_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let missing = compass_db::index_stats::inspect(&dir.path().join("absent.db"));
        let report = file_index(Some(&missing));
        assert_eq!(report.status, DoctorStatus::Ok);
        assert!(!dir.path().join("absent.db").exists());

        let busy = IndexStats {
            page_count: Measurement::Unavailable(
                "database is busy; retry when the indexer is idle".into(),
            ),
            ..measured()
        };
        let report = file_index(Some(&busy));
        assert_eq!(report.status, DoctorStatus::Warn);
        assert!(
            report
                .detail
                .unwrap()
                .contains("database is busy; retry when the indexer is idle")
        );
        assert_eq!(file_index(None).status, DoctorStatus::Warn);
    }
}
