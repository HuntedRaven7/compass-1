//! The dmenu view: `compass dmenu`'s lines, filtered, and the choice.
//!
//! Ports `DMenuSection` (`src/server/src/ui/views/dmenu-model.cpp`): the
//! entries are stdin's non-empty lines, the filter is the same weighted fuzzy
//! score as everywhere else and keeps the input order among equals, a path
//! shows its last component, and the output is the entry or its index.

use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

use crate::backend::DmenuList;

/// The section heading when the command gives none: `m_sectionTemplate`.
pub const DEFAULT_SECTION: &str = "Entries ({count})";

/// The search field's placeholder when the command gives none.
pub const DEFAULT_PLACEHOLDER: &str = "Search entries...";

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The list is on its way from the engine.
    Loading,
    /// The list is shown.
    Ready,
    /// The list could not be fetched, and why.
    Failed(String),
}

/// The view's state.
#[derive(Debug, Clone)]
pub struct DmenuPage {
    /// The engine's token for the waiting command.
    pub token: u64,
    /// The command's options, once fetched.
    pub list: DmenuList,
    /// The entries, in input order.
    pub entries: Vec<String>,
    /// The search text.
    pub query: String,
    /// Positions in `entries` that match, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Whether the choice has been sent, so dismissing does not send another.
    pub answered: bool,
    /// Quick look: the selected entry's preview, when it is a file that
    /// exists and quick look is on.
    pub preview: Option<crate::file_preview::FilePreview>,
}

impl DmenuPage {
    /// A view waiting for the list behind `token`.
    #[must_use]
    pub fn loading(token: u64) -> Self {
        Self {
            token,
            list: DmenuList::default(),
            entries: Vec::new(),
            query: String::new(),
            shown: Vec::new(),
            selected: 0,
            status: Status::Loading,
            answered: false,
            preview: None,
        }
    }

    /// Takes the fetched list: its entries are the non-empty lines, and its
    /// `--query` is the initial search text.
    pub fn apply(&mut self, result: Result<DmenuList, String>) {
        match result {
            Ok(list) => {
                self.entries = list
                    .content
                    .split('\n')
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect();
                self.query = list.query.clone().unwrap_or_default();
                self.list = list;
                self.status = Status::Ready;
            }
            Err(reason) => self.status = Status::Failed(reason),
        }
        self.refilter();
    }

    /// Recomputes `shown` for the current text, back at the top.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let query = Query::new(&self.query);
        let mut scored: Vec<(u32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                if query.is_empty() {
                    return Some((0, index));
                }
                let found = score_weighted(&[WeightedField::new(entry, 1.0)], &query);
                (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.shown = scored.into_iter().map(|(_, index)| index).collect();
        self.refresh_preview();
    }

    /// Reads the selected entry's preview when it is a path that exists and
    /// quick look is on (`setOnFileHighlighted`), and clears it otherwise.
    pub fn refresh_preview(&mut self) {
        let path = self
            .selected_entry()
            .filter(|entry| !self.list.no_quick_look && entry.starts_with('/'))
            .map(std::path::PathBuf::from);
        let Some(path) = path else {
            self.preview = None;
            return;
        };
        if self
            .preview
            .as_ref()
            .is_some_and(|preview| std::path::Path::new(&preview.path) == path)
        {
            return;
        }
        self.preview = crate::file_preview::load(&path, None, false);
    }

    /// The window size `--width`/`--height` ask for, when either is given:
    /// the other side keeps the launcher's own.
    #[must_use]
    pub fn window_size(&self, default: (u32, u32)) -> Option<(u32, u32)> {
        if self.list.width.is_none() && self.list.height.is_none() {
            return None;
        }
        Some((
            self.list.width.unwrap_or(default.0),
            self.list.height.unwrap_or(default.1),
        ))
    }

    /// What choosing the selected entry prints: the entry, or its index in
    /// the input with `--format index`; the search text when nothing matches,
    /// as the empty list's "Pass search text" does.
    #[must_use]
    pub fn selected_output(&self) -> String {
        match self.shown.get(self.selected) {
            Some(&index) if self.list.output_index => index.to_string(),
            Some(&index) => self.entries[index].clone(),
            None => self.query.clone(),
        }
    }

    /// The selected entry's own text, for "Select and copy entry".
    #[must_use]
    pub fn selected_entry(&self) -> Option<&str> {
        self.shown
            .get(self.selected)
            .map(|&index| self.entries[index].as_str())
    }

    /// The section heading, with `{count}` expanded; `None` with
    /// `--no-section`.
    #[must_use]
    pub fn heading(&self) -> Option<String> {
        if self.list.no_section {
            return None;
        }
        let template = self
            .list
            .section_title
            .as_deref()
            .unwrap_or(DEFAULT_SECTION);
        Some(template.replace("{count}", &self.shown.len().to_string()))
    }

    /// The search field's placeholder.
    #[must_use]
    pub fn placeholder(&self) -> &str {
        self.list
            .placeholder
            .as_deref()
            .unwrap_or(DEFAULT_PLACEHOLDER)
    }

    /// What the view says in place of the list: loading, why it failed, or
    /// that nothing matches; `None` while it shows entries.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Loading entries…"),
            Status::Failed(reason) => Some(reason),
            Status::Ready if self.shown.is_empty() => {
                Some("No entries match. Enter passes the search text.")
            }
            Status::Ready => None,
        }
    }
}

/// An entry's title and second line: a path shows its last component, and
/// the folder it is in when it exists and quick look is off; with quick look
/// on, the preview pane names the folder instead (`itemSubtitle`).
#[must_use]
pub fn entry_text(entry: &str, quick_look: bool) -> (String, Option<String>) {
    if !entry.starts_with('/') {
        return (entry.to_owned(), None);
    }
    let path = std::path::Path::new(entry);
    let title = path.file_name().map_or_else(
        || entry.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let subtitle = (!quick_look && path.exists())
        .then(|| path.parent().map(|p| p.to_string_lossy().into_owned()))
        .flatten();
    (title, subtitle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(content: &str, output_index: bool) -> DmenuPage {
        let mut page = DmenuPage::loading(1);
        page.apply(Ok(DmenuList {
            content: content.into(),
            output_index,
            ..DmenuList::default()
        }));
        page
    }

    #[test]
    fn lines_are_entries_and_the_filter_keeps_input_order_among_equals() {
        let mut page = page("firefox\n\nfiles\nterminal\n", false);
        assert_eq!(page.entries, ["firefox", "files", "terminal"]);
        assert_eq!(page.heading().as_deref(), Some("Entries (3)"));
        page.query = "fi".into();
        page.refilter();
        assert_eq!(page.shown.len(), 2);
        assert_eq!(page.heading().as_deref(), Some("Entries (2)"));
        page.query = "termnal".into();
        page.refilter();
        assert_eq!(page.selected_output(), "terminal", "a typo still matches");
    }

    #[test]
    fn the_output_is_the_entry_its_index_or_the_search_text() {
        let mut page = page("alpha\nbeta\ngamma", true);
        page.query = "gamma".into();
        page.refilter();
        assert_eq!(page.selected_output(), "2");
        page.query = "nothing like it".into();
        page.refilter();
        assert!(page.shown.is_empty());
        assert_eq!(page.selected_output(), "nothing like it");
    }

    #[test]
    fn a_path_shows_its_name_and_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, "x").unwrap();
        let (title, subtitle) = entry_text(&file.to_string_lossy(), false);
        assert_eq!(title, "notes.md");
        assert_eq!(subtitle.as_deref(), Some(&*dir.path().to_string_lossy()));
        assert_eq!(entry_text(&file.to_string_lossy(), true).1, None);
        assert_eq!(entry_text("plain", false), ("plain".to_owned(), None));
    }

    #[test]
    fn quick_look_previews_a_selected_file_and_the_size_is_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, "hello").unwrap();
        let mut list = page(&format!("plain\n{}", file.display()), false);
        assert!(list.preview.is_none(), "a plain entry has none");
        list.selected = 1;
        list.refresh_preview();
        let preview = list.preview.clone().expect("the file is previewed");
        assert_eq!(preview.name, "notes.md");
        assert_eq!(
            preview.content,
            crate::file_preview::Content::Text("hello".into())
        );
        list.list.no_quick_look = true;
        list.refresh_preview();
        assert!(list.preview.is_none(), "--no-quick-look has none");

        assert_eq!(list.window_size((720, 560)), None);
        list.list.width = Some(400);
        assert_eq!(list.window_size((720, 560)), Some((400, 560)));
    }
}
