//! The Search Files view: its state, and what it decides.
//!
//! What a query answers is the engine's (`compass::file_search`); this keeps
//! what the view decides on its own — when to wait out the indexer's
//! debounce, which answer is stale, and what a row's second line says.

use compass_core::file_search::{QUERY_DEBOUNCE_MS, is_explicit_path_query, should_debounce};

use crate::backend::{FileResults, FileRow};

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The first answer has not arrived.
    Loading,
    /// An answer arrived (possibly empty).
    Ready,
    /// Files cannot be shown, and why.
    Failed(String),
}

/// The Search Files view's state.
#[derive(Debug, Clone)]
pub struct FilesPage {
    /// The search text.
    pub query: String,
    /// What the list is, as the engine named it.
    pub heading: String,
    /// Rows in presentation order.
    pub rows: Vec<FileRow>,
    /// Position in `rows`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Bumped per keystroke, so a late or superseded answer is dropped.
    pub generation: u64,
    /// Why the last open did not happen, until the next keystroke.
    pub notice: Option<String>,
    /// The category filter's key (`compass_core::file_search::CATEGORY_FILTER_KEYS`);
    /// `None` is "All".
    pub category: Option<String>,
    /// The selected file's preview, for the detail pane.
    pub preview: Option<crate::file_preview::FilePreview>,
    /// The file the preview was read from.
    preview_path: Option<String>,
    /// A query is out and its answer has not come: the view's loading
    /// indicator (`setLoading`).
    pub searching: bool,
}

impl Default for FilesPage {
    fn default() -> Self {
        Self {
            query: String::new(),
            heading: String::new(),
            rows: Vec::new(),
            selected: 0,
            status: Status::Loading,
            generation: 0,
            notice: None,
            category: None,
            preview: None,
            preview_path: None,
            searching: true,
        }
    }
}

/// Whether Run executable is offered for `path`: an AppImage, which the
/// C++ makes executable on the fly (`AUTO_EXECUTABLE_EXTENSIONS`).
#[must_use]
pub fn runs_as_executable(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|ext| ext == "AppImage")
}

impl FilesPage {
    /// The selected row, if any.
    #[must_use]
    pub fn selected_row(&self) -> Option<&FileRow> {
        self.rows.get(self.selected)
    }

    /// What the view says in place of the list: searching, why it failed,
    /// or that nothing was found.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Searching files…"),
            Status::Failed(reason) => Some(reason.as_str()),
            Status::Ready if self.rows.is_empty() => Some("No files found"),
            Status::Ready => None,
        }
    }

    /// The loading indicator beside the filter (`setLoading`), while a
    /// query is out.
    #[must_use]
    pub fn indicator(&self) -> Option<&'static str> {
        self.searching.then_some("Searching…")
    }

    /// Takes new search text, returning the generation its answer must carry.
    pub fn set_query(&mut self, query: String) -> u64 {
        self.query = query;
        self.notice = None;
        self.generation = self.generation.wrapping_add(1);
        self.searching = true;
        self.generation
    }

    /// Applies an answer. Returns `false` (and changes nothing) for a stale
    /// one — the query moved on while it was being answered.
    pub fn apply(&mut self, generation: u64, result: Result<FileResults, String>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.searching = false;
        self.selected = 0;
        match result {
            Ok(results) => {
                self.heading = results.heading;
                self.rows = results.files;
                self.status = Status::Ready;
            }
            Err(reason) => {
                self.heading.clear();
                self.rows.clear();
                self.status = Status::Failed(reason);
            }
        }
        self.refresh_preview();
        true
    }

    /// Filters by the option `key`; `All` clears the filter. Returns the
    /// generation the next answer must carry, since the list is asked again.
    pub fn set_category(&mut self, key: &str) -> u64 {
        self.category = (key != compass_core::file_search::CATEGORY_FILTER_KEYS[0]
            && compass_core::file_search::category_for_key(key).is_some())
        .then(|| key.to_owned());
        self.generation = self.generation.wrapping_add(1);
        self.searching = true;
        self.generation
    }

    /// Reads the selected file's preview (`loadDetail`): name, the path with
    /// home folded, type, modified time, and the image or text; nothing
    /// when there is no selection.
    pub fn refresh_preview(&mut self) {
        let Some(path) = self.selected_row().map(|row| row.path.clone()) else {
            self.preview = None;
            self.preview_path = None;
            return;
        };
        if self.preview_path.as_deref() == Some(path.as_str()) {
            return;
        }
        let home = compass_core::xdg_dirs::home_dir();
        self.preview =
            crate::file_preview::load(std::path::Path::new(&path), home.as_deref(), true);
        self.preview_path = Some(path);
    }
}

/// How long to wait before asking, or `None` to ask at once.
///
/// An empty query and a typed path are answered at once, the way the C++
/// stops its timer for both; an index search waits out the indexer's
/// debounce, so a word typed quickly is one query rather than one per letter.
#[must_use]
pub fn debounce_for(query: &str) -> Option<std::time::Duration> {
    if query.is_empty() || is_explicit_path_query(query.trim()) {
        return None;
    }
    should_debounce(QUERY_DEBOUNCE_MS).then(|| std::time::Duration::from_millis(QUERY_DEBOUNCE_MS))
}

/// A row's second line: the folder it is in, with the home directory as `~`.
#[must_use]
pub fn subtitle(row: &FileRow, home: Option<&str>) -> String {
    let path = std::path::Path::new(&row.path);
    let folder = path
        .parent()
        .map_or_else(String::new, |parent| parent.to_string_lossy().into_owned());
    match home {
        Some(home) if !home.is_empty() => {
            if folder == home {
                "~".to_owned()
            } else if let Some(rest) = folder.strip_prefix(home)
                && rest.starts_with('/')
            {
                format!("~{rest}")
            } else {
                folder
            }
        }
        _ => folder,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str) -> FileRow {
        FileRow {
            path: path.into(),
            name: path.rsplit('/').next().unwrap_or(path).into(),
            category: "Other".into(),
        }
    }

    #[test]
    fn a_superseded_answer_is_dropped() {
        let mut page = FilesPage::default();
        let first = page.set_query("rep".into());
        let second = page.set_query("report".into());
        assert!(!page.apply(
            first,
            Ok(FileResults {
                heading: "Results".into(),
                files: vec![row("/old")],
            })
        ));
        assert!(page.rows.is_empty());
        assert_eq!(page.status, Status::Loading);
        assert!(page.searching, "the indicator stays until the last answer");
        assert!(page.apply(
            second,
            Ok(FileResults {
                heading: "Results".into(),
                files: vec![row("/home/me/report.pdf")],
            })
        ));
        assert_eq!(
            page.selected_row().map(|r| r.name.as_str()),
            Some("report.pdf")
        );
        assert_eq!(page.heading, "Results");
        assert!(!page.searching);
        page.set_category("Images");
        assert!(page.searching, "a new filter asks again");
        assert!(runs_as_executable("/opt/Tool.AppImage"));
        assert!(!runs_as_executable("/opt/tool.sh"));
    }

    #[test]
    fn a_failure_clears_the_list_and_says_why() {
        let mut page = FilesPage::default();
        let generation = page.set_query("x".into());
        page.apply(generation, Err("no indexer".into()));
        assert!(page.rows.is_empty());
        assert_eq!(page.status, Status::Failed("no indexer".into()));
    }

    #[test]
    fn only_index_searches_wait_out_the_debounce() {
        assert_eq!(debounce_for(""), None);
        assert_eq!(debounce_for("~/Documents"), None);
        assert_eq!(debounce_for("/etc"), None);
        assert_eq!(
            debounce_for("report"),
            Some(std::time::Duration::from_millis(100))
        );
    }

    #[test]
    fn the_subtitle_is_the_folder_with_home_folded() {
        let home = Some("/home/me");
        assert_eq!(
            subtitle(&row("/home/me/Documents/a.pdf"), home),
            "~/Documents"
        );
        assert_eq!(subtitle(&row("/home/me/a.pdf"), home), "~");
        assert_eq!(subtitle(&row("/home/meow/a.pdf"), home), "/home/meow");
        assert_eq!(subtitle(&row("/srv/a.pdf"), None), "/srv");
    }
}
