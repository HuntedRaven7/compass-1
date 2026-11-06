//! `compass-clipboard` — the clipboard history store, ported from
//! `src/server/src/services/clipboard/clipboard-db.cpp`.
//!
//! Landing in slices rather than at once. Present so far:
//!
//! * [`kind`] — what an entry is and whether it is encrypted, as the integers
//!   SQLite actually stores.
//! * [`schema`] — the migrations, and applying them to a database.
//! * [`search`] — how a query in the search box becomes the FTS match phrases
//!   and substring terms the SQL layer binds.
//! * [`store`] — reading history back: the paginated query.
//! * [`crate::write`] — inserts, indexing, deletion and eviction.
//! * [`ingest`] — recording one observed copy, as delivered by the GNOME
//!   helper extension's `ClipboardChanged` signal.
//! * [`files`] — the history's files on disk: measuring them, reading them
//!   without writing, and resetting them.
//!
//! Still on the C++ side: the monitoring loop and multi-offer sanitising in
//! `clipboard-service.cpp`, which the daemon wiring will need. See
//! `docs/rust-engine/PARITY.md`.
//!
//! [`classify`]: ingest::classify
//! [`preview`]: ingest::preview

#![deny(missing_docs)]

pub mod files;
pub mod history_view;
pub mod ingest;
pub mod kind;
pub mod retention;
pub mod schema;
pub mod search;
pub mod store;
pub mod write;
