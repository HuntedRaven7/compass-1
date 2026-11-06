//! The clipboard history view: its state, and the decisions it makes.
//!
//! Kept out of `app.rs` so that what the view *decides* — which row is
//! selected after a reload, whether content can be copied back as text,
//! what a row's subtitle says — is testable without a window.

use crate::backend::{
    ClipboardContent, ClipboardDetail, ClipboardMonitoring, ClipboardRow, ClipboardRowKind,
};

/// How many entries one request asks for.
pub const PAGE_SIZE: u32 = 100;

/// The kind filter's options, as `(label, stored value, kind)`, in the
/// dropdown's order: `compass_clipboard::history_view`'s
/// `KIND_FILTER_OPTIONS` and `FILTER_STORED_VALUES`, which this crate cannot
/// name. The stored vocabulary is the enum's (`image`), the label the
/// interface's (`Images`).
pub const KIND_FILTERS: [(&str, &str, Option<ClipboardRowKind>); 5] = [
    ("All", "all", None),
    ("Text", "text", Some(ClipboardRowKind::Text)),
    ("Images", "image", Some(ClipboardRowKind::Image)),
    ("Links", "link", Some(ClipboardRowKind::Link)),
    ("Files", "file", Some(ClipboardRowKind::File)),
];

/// Where the chosen filter is remembered between openings (the C++ keeps it
/// in the command's storage under `filter`).
pub const FILTER_MEMORY_KEY: &str = "clipboard.filter";

/// The kind a stored filter value names; anything unknown is every kind.
#[must_use]
pub fn kind_for_stored(stored: Option<&str>) -> Option<ClipboardRowKind> {
    KIND_FILTERS
        .iter()
        .find(|(_, value, _)| Some(*value) == stored)
        .and_then(|(_, _, kind)| *kind)
}

/// The label and stored value for `kind`.
#[must_use]
pub fn filter_for_kind(kind: Option<ClipboardRowKind>) -> (&'static str, &'static str) {
    KIND_FILTERS
        .iter()
        .find(|(_, _, candidate)| *candidate == kind)
        .map_or(("All", "all"), |(label, value, _)| (*label, *value))
}

/// The kind a dropdown label names.
#[must_use]
pub fn kind_for_label(label: &str) -> Option<ClipboardRowKind> {
    KIND_FILTERS
        .iter()
        .find(|(candidate, _, _)| *candidate == label)
        .and_then(|(_, _, kind)| *kind)
}

/// The detail pane's state for one entry.
#[derive(Debug, Clone)]
pub struct Detail {
    /// Which entry.
    pub id: String,
    /// Its metadata, once it has arrived.
    pub info: Option<Result<ClipboardDetail, String>>,
    /// Its content, once it has arrived.
    pub content: Option<Result<ClipboardContent, String>>,
    /// What the pane draws, decided once when the content arrives.
    pub pane: Option<DetailContent>,
    /// The image, when the content is one, made once so it is decoded once.
    pub image: Option<iced::widget::image::Handle>,
}

/// What the pane draws above the metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailContent {
    /// The start of text.
    Text(String),
    /// An image, as it was copied.
    Image(Vec<u8>),
    /// A single file that still exists, previewed as Search Files would.
    File(crate::file_preview::FilePreview),
    /// Why there is nothing: the title and the sentence under it.
    Error(String, String),
    /// Nothing to draw.
    None,
}

/// How much text the pane shows (`MAX_DISPLAY`, 10 KiB).
pub const MAX_DISPLAY: usize = 10 * 1024;

/// What the pane draws for `content`, as `loadDetail` decides: a single
/// local file that exists is previewed, an image is drawn, text (and a URI
/// list) is quoted up to [`MAX_DISPLAY`] bytes, anything else is nothing.
#[must_use]
pub fn detail_content(content: &Result<ClipboardContent, String>) -> DetailContent {
    let content = match content {
        Ok(content) => content,
        Err(reason) => {
            return DetailContent::Error("Data unavailable".to_owned(), reason.clone());
        }
    };
    let mime = content.mime_type.as_str();
    if mime == "text/uri-list" {
        let text = String::from_utf8_lossy(&content.data);
        let paths: Vec<&str> = text.split("\r\n").filter(|line| !line.is_empty()).collect();
        if let [single] = paths.as_slice()
            && let Some(path) = single.strip_prefix("file://")
        {
            let path = std::path::Path::new(path);
            if path.is_file()
                && let Some(preview) = crate::file_preview::load(path, None, false)
            {
                return DetailContent::File(preview);
            }
        }
    }
    if mime.starts_with("image/") {
        return DetailContent::Image(content.data.clone());
    }
    if mime.starts_with("text/") || mime == "text/uri-list" || mime == "application/x-sh" {
        let end = content.data.len().min(MAX_DISPLAY);
        return DetailContent::Text(String::from_utf8_lossy(&content.data[..end]).into_owned());
    }
    DetailContent::None
}

/// What clipboard history's Open and Open with… act on
/// (`ClipboardHistorySection::actionPanel`): a link's URL, or a copied
/// single file that still exists, as its path. `None` for anything else,
/// which offers neither.
#[must_use]
pub fn open_target(kind: ClipboardRowKind, content: &ClipboardContent) -> Option<OpenTarget> {
    let text = String::from_utf8_lossy(&content.data);
    match kind {
        ClipboardRowKind::Link => {
            let url = text.trim();
            (!url.is_empty()).then(|| OpenTarget::Url(url.to_owned()))
        }
        ClipboardRowKind::File => {
            let urls: Vec<&str> = text
                .split("\r\n")
                .flat_map(|line| line.split('\n'))
                .filter(|line| !line.trim().is_empty())
                .collect();
            let [single] = urls.as_slice() else {
                return None;
            };
            let path = url::Url::parse(single.trim()).ok()?.to_file_path().ok()?;
            path.exists()
                .then(|| OpenTarget::File(path.to_string_lossy().into_owned()))
        }
        _ => None,
    }
}

/// What an entry opens as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTarget {
    /// A link, opened with the default browser or a chosen application.
    Url(String),
    /// A file's path.
    File(String),
}

impl OpenTarget {
    /// The URL or path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Url(target) | Self::File(target) => target,
        }
    }
}

/// A size as the pane shows it (`utils.cpp`'s `formatSize`): whole bytes,
/// then the largest 1024-based unit it reaches, with two decimals under 10,
/// one under 100 and none above.
#[must_use]
pub fn format_size(bytes: i64) -> String {
    const UNITS: [&str; 6] = ["bytes", "KB", "MB", "GB", "TB", "PB"];
    if bytes <= 0 {
        return "0 bytes".to_owned();
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    let number = if unit == 0 {
        bytes.to_string()
    } else if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    };
    format!("{number} {}", UNITS[unit])
}

/// A kind's name in the pane (`kindLabel`).
#[must_use]
pub const fn kind_label(kind: ClipboardRowKind) -> &'static str {
    match kind {
        ClipboardRowKind::Text => "Text",
        ClipboardRowKind::Link => "Link",
        ClipboardRowKind::Image => "Image",
        ClipboardRowKind::File => "File",
        ClipboardRowKind::Unknown => "Unknown",
    }
}

/// The pane's metadata rows, as `(label, value)`.
#[must_use]
pub fn detail_fields(info: &ClipboardDetail) -> Vec<(&'static str, String)> {
    let copied_at = jiff::Timestamp::from_millisecond(info.updated_at)
        .map(|at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%a %b %-d %H:%M:%S %Y")
                .to_string()
        })
        .unwrap_or_default();
    let mut fields = vec![
        ("Type", kind_label(info.kind).to_owned()),
        ("MIME type", info.mime_type.clone()),
        ("Size", format_size(info.size)),
        ("Copied at", copied_at),
        ("MD5", info.md5.clone()),
    ];
    if info.encrypted {
        fields.push(("Encrypted", "Yes".to_owned()));
    }
    if !info.keywords.is_empty() {
        fields.push(("Keywords", info.keywords.clone()));
    }
    fields
}

/// What the view says about monitoring (`handleMonitoringChanged`): the
/// panel action's title, or why there is none.
#[must_use]
pub const fn monitoring_action(monitoring: ClipboardMonitoring) -> Option<&'static str> {
    match (monitoring.supported, monitoring.enabled) {
        (false, _) => None,
        (true, true) => Some("Pause clipboard"),
        (true, false) => Some("Resume clipboard"),
    }
}

/// The line shown while monitoring is not happening, if it is not.
#[must_use]
pub const fn monitoring_notice(monitoring: ClipboardMonitoring) -> Option<&'static str> {
    match (monitoring.supported, monitoring.enabled) {
        (false, _) => Some("Clipboard monitoring unavailable"),
        (true, false) => Some("Clipboard monitoring is paused"),
        (true, true) => None,
    }
}

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// A request is in flight and nothing has arrived yet.
    Loading,
    /// Rows have arrived (possibly none).
    Ready,
    /// History cannot be shown, and why.
    Failed(String),
}

/// The clipboard history view's state.
#[derive(Debug, Clone)]
pub struct ClipboardPage {
    /// The filter text.
    pub query: String,
    /// Rows in presentation order.
    pub rows: Vec<ClipboardRow>,
    /// Position in `rows`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Bumped per request so a late answer to an old query is dropped.
    pub generation: u64,
    /// Why the last Enter did not copy, until the next keystroke.
    pub notice: Option<String>,
    /// The kind filter; `None` is every kind.
    pub kind: Option<ClipboardRowKind>,
    /// The detail pane, for the selected entry.
    pub detail: Option<Detail>,
    /// Whether copies are being recorded, once the engine has said.
    pub monitoring: Option<ClipboardMonitoring>,
    /// The filter text and kind the rows were last listed for. An answer for
    /// the same pair is a reload (after a delete, a pin, a new copy), which
    /// keeps the selection on its entry; a new filter starts at the top.
    pub listed_for: Option<(String, Option<ClipboardRowKind>)>,
}

impl Default for ClipboardPage {
    fn default() -> Self {
        Self {
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
            status: Status::Loading,
            generation: 0,
            notice: None,
            kind: None,
            detail: None,
            monitoring: None,
            listed_for: None,
        }
    }
}

impl ClipboardPage {
    /// The selected row, if any.
    #[must_use]
    pub fn selected_row(&self) -> Option<&ClipboardRow> {
        self.rows.get(self.selected)
    }

    /// What the view says in place of the list: loading, why it failed, or
    /// that there is nothing to show.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Loading clipboard history…"),
            Status::Failed(reason) => Some(reason.as_str()),
            Status::Ready
                if self.rows.is_empty() && self.query.is_empty() && self.kind.is_none() =>
            {
                Some("Nothing copied yet")
            }
            Status::Ready if self.rows.is_empty() => Some("No matching entries"),
            Status::Ready => None,
        }
    }

    /// The id whose detail should be fetched, when the pane is not already
    /// showing (or fetching) the selected entry's; `None` clears nothing and
    /// asks for nothing.
    pub fn detail_wanted(&mut self) -> Option<String> {
        let Some(row) = self.selected_row() else {
            self.detail = None;
            return None;
        };
        if self
            .detail
            .as_ref()
            .is_some_and(|detail| detail.id == row.id)
        {
            return None;
        }
        let id = row.id.clone();
        self.detail = Some(Detail {
            id: id.clone(),
            info: None,
            content: None,
            pane: None,
            image: None,
        });
        Some(id)
    }

    /// Takes in an answer for the pane; one for an entry no longer selected
    /// is dropped.
    pub fn apply_detail(
        &mut self,
        id: &str,
        info: Option<Result<ClipboardDetail, String>>,
        content: Option<Result<ClipboardContent, String>>,
    ) {
        let Some(detail) = self.detail.as_mut().filter(|detail| detail.id == id) else {
            return;
        };
        if info.is_some() {
            detail.info = info;
        }
        if let Some(content) = content {
            let pane = detail_content(&content);
            detail.image = match &pane {
                DetailContent::Image(bytes) => {
                    Some(iced::widget::image::Handle::from_bytes(bytes.clone()))
                }
                _ => None,
            };
            detail.pane = Some(pane);
            detail.content = Some(content);
        }
    }

    /// Applies an answer. Returns `false` (and changes nothing) for a stale one.
    pub fn apply(&mut self, generation: u64, result: Result<Vec<ClipboardRow>, String>) -> bool {
        if generation != self.generation {
            return false;
        }
        match result {
            Ok(rows) => {
                let listing = (self.query.clone(), self.kind);
                let reload = self.listed_for.as_ref() == Some(&listing);
                let was = self.selected_row().map(|row| row.id.clone());
                let at = self.selected;
                self.rows = rows;
                // By id, not by position: a reload that lands between seeing
                // a row and pressing Ctrl+X must not put the key on another.
                self.selected = match (reload, was) {
                    (true, Some(id)) => self
                        .rows
                        .iter()
                        .position(|row| row.id == id)
                        .unwrap_or_else(|| at.min(self.rows.len().saturating_sub(1))),
                    _ => 0,
                };
                self.listed_for = Some(listing);
                self.status = Status::Ready;
            }
            Err(error) => {
                self.rows.clear();
                self.selected = 0;
                self.status = Status::Failed(error);
            }
        }
        true
    }
}

/// The text to put back on the clipboard, or why there is none.
///
/// Text and links go back as text. Images and files need a clipboard write
/// that carries their MIME type, which this view does not do yet, so they are
/// refused with a reason rather than copied as a lossy text rendering.
///
/// # Errors
///
/// A sentence for the user when the content is not text.
pub fn copyable_text(content: &ClipboardContent) -> Result<String, String> {
    let is_text = content.mime_type.starts_with("text/")
        || content.mime_type == "application/x-sh"
        || content.mime_type.contains("uri-list");
    if !is_text {
        return Err(format!(
            "Copying {} back is not supported yet",
            describe_mime(&content.mime_type)
        ));
    }
    String::from_utf8(content.data.clone())
        .map_err(|_| "This entry is not valid text and cannot be copied back".to_owned())
}

fn describe_mime(mime: &str) -> &'static str {
    if mime.starts_with("image/") {
        "images"
    } else {
        "this kind of entry"
    }
}

/// The detail text cut where the search's words occur, each piece with
/// whether it is one (`MatchHighlighter` over `searchTerms`: the query's
/// words, found literally, ignoring case and accents).
#[must_use]
pub fn highlighted<'a>(body: &'a str, query: &str) -> Vec<(&'a str, bool)> {
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut pieces = Vec::new();
    let mut at = 0;
    for range in compass_search::term_ranges(body, &terms) {
        if range.start > at {
            pieces.push((&body[at..range.start], false));
        }
        pieces.push((&body[range.clone()], true));
        at = range.end;
    }
    if at < body.len() || pieces.is_empty() {
        pieces.push((&body[at..], false));
    }
    pieces
}

/// A row's second line.
#[must_use]
pub fn subtitle(row: &ClipboardRow) -> String {
    let kind = match row.kind {
        ClipboardRowKind::Text => "Text",
        ClipboardRowKind::Link => "Link",
        ClipboardRowKind::Image => "Image",
        ClipboardRowKind::File => "File",
        ClipboardRowKind::Unknown => "Other",
    };
    let mut line = match (&row.kind, &row.url_host) {
        (ClipboardRowKind::Link, Some(host)) => format!("{kind} · {host}"),
        _ => kind.to_owned(),
    };
    if row.pinned {
        line.push_str(" · Pinned");
    }
    line
}

#[cfg(test)]
mod highlight_tests {
    use super::*;

    #[test]
    fn the_searched_words_are_marked_in_the_detail_text() {
        assert_eq!(
            highlighted("Déjà vu, deja VU again", "deja vu"),
            [
                ("Déjà", true),
                (" ", false),
                ("vu", true),
                (", ", false),
                ("deja", true),
                (" ", false),
                ("VU", true),
                (" again", false),
            ]
        );
        assert_eq!(highlighted("plain", "  "), [("plain", false)]);
        assert_eq!(highlighted("", "x"), [("", false)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_or_one_existing_file_is_what_open_acts_on() {
        let content = |data: &str| ClipboardContent {
            mime_type: "text/uri-list".into(),
            data: data.as_bytes().to_vec(),
        };
        assert_eq!(
            open_target(ClipboardRowKind::Link, &content(" https://docs.rs \n")),
            Some(OpenTarget::Url("https://docs.rs".into()))
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a b.txt");
        std::fs::write(&file, "x").unwrap();
        let uri = url::Url::from_file_path(&file).unwrap().to_string();
        assert!(uri.contains("%20"), "{uri}");
        assert_eq!(
            open_target(ClipboardRowKind::File, &content(&format!("{uri}\r\n"))),
            Some(OpenTarget::File(file.to_string_lossy().into_owned())),
            "the path is decoded, as QUrl::path"
        );
        assert_eq!(
            open_target(
                ClipboardRowKind::File,
                &content(&format!("{uri}\r\n{uri}\r\n"))
            ),
            None,
            "two files open neither"
        );
        assert_eq!(
            open_target(ClipboardRowKind::File, &content("file:///nowhere/at/all")),
            None,
            "a file that is gone"
        );
        assert_eq!(
            open_target(ClipboardRowKind::Text, &content("https://docs.rs")),
            None
        );
    }

    fn row(id: &str, kind: ClipboardRowKind) -> ClipboardRow {
        ClipboardRow {
            id: id.into(),
            preview: id.into(),
            kind,
            pinned: false,
            url_host: None,
        }
    }

    #[test]
    fn a_stale_answer_is_dropped() {
        let mut page = ClipboardPage {
            generation: 2,
            ..ClipboardPage::default()
        };
        assert!(!page.apply(1, Ok(vec![row("old", ClipboardRowKind::Text)])));
        assert!(page.rows.is_empty());
        assert_eq!(page.status, Status::Loading);
        assert!(page.apply(2, Ok(vec![row("new", ClipboardRowKind::Text)])));
        assert_eq!(page.selected_row().map(|r| r.id.as_str()), Some("new"));
        assert_eq!(page.status, Status::Ready);
    }

    #[test]
    fn a_reload_keeps_the_selection_on_its_entry_and_a_new_filter_starts_at_the_top() {
        let text = ClipboardRowKind::Text;
        let mut page = ClipboardPage::default();
        page.apply(
            0,
            Ok(vec![row("pinned", text), row("a", text), row("b", text)]),
        );
        page.selected = 2;
        // A new copy arrives at the top while "b" is selected.
        page.apply(
            0,
            Ok(vec![
                row("pinned", text),
                row("new", text),
                row("a", text),
                row("b", text),
            ]),
        );
        assert_eq!(page.selected_row().map(|r| r.id.as_str()), Some("b"));
        // "b" is deleted: the selection stays where it was, on the next one
        // up the list rather than jumping to the pinned entry at the top.
        page.apply(
            0,
            Ok(vec![row("pinned", text), row("new", text), row("a", text)]),
        );
        assert_eq!(page.selected_row().map(|r| r.id.as_str()), Some("a"));
        page.query = "n".into();
        page.apply(0, Ok(vec![row("pinned", text), row("new", text)]));
        assert_eq!(page.selected, 0);
    }

    #[test]
    fn a_failure_clears_the_rows_and_says_why() {
        let mut page = ClipboardPage::default();
        page.apply(0, Ok(vec![row("a", ClipboardRowKind::Text)]));
        page.apply(0, Err("no keyring".into()));
        assert!(page.rows.is_empty());
        assert_eq!(page.status, Status::Failed("no keyring".into()));
    }

    #[test]
    fn text_goes_back_whole_and_images_are_refused_by_name() {
        let text = ClipboardContent {
            mime_type: "text/plain;charset=utf-8".into(),
            data: "héllo 🚀".as_bytes().to_vec(),
        };
        assert_eq!(copyable_text(&text).as_deref(), Ok("héllo 🚀"));

        let image = ClipboardContent {
            mime_type: "image/png".into(),
            data: vec![0x89, b'P'],
        };
        let refusal = copyable_text(&image).expect_err("an image is not text");
        assert!(refusal.contains("images"), "{refusal}");

        let broken = ClipboardContent {
            mime_type: "text/plain".into(),
            data: vec![0xff, 0xfe],
        };
        assert!(copyable_text(&broken).is_err());
    }

    #[test]
    fn the_filter_stores_the_enums_word_and_shows_the_interfaces() {
        assert_eq!(
            kind_for_stored(Some("image")),
            Some(ClipboardRowKind::Image)
        );
        assert_eq!(
            kind_for_stored(Some("Images")),
            None,
            "labels are not stored"
        );
        assert_eq!(kind_for_stored(Some("bogus")), None);
        assert_eq!(kind_for_stored(None), None);
        assert_eq!(
            filter_for_kind(Some(ClipboardRowKind::Link)),
            ("Links", "link")
        );
        assert_eq!(filter_for_kind(None), ("All", "all"));
        assert_eq!(kind_for_label("Files"), Some(ClipboardRowKind::File));
        assert_eq!(
            filter_for_kind(Some(ClipboardRowKind::Unknown)),
            ("All", "all"),
            "a kind with no option falls back to the first"
        );
    }

    #[test]
    fn sizes_read_as_the_cpp_writes_them() {
        assert_eq!(format_size(0), "0 bytes");
        assert_eq!(format_size(1023), "1023 bytes");
        assert_eq!(format_size(1536), "1.50 KB");
        assert_eq!(format_size(20 * 1024), "20.0 KB");
        assert_eq!(format_size(300 * 1024 * 1024), "300 MB");
    }

    #[test]
    fn the_pane_quotes_text_draws_images_and_previews_one_existing_file() {
        let text = Ok(ClipboardContent {
            mime_type: "text/plain".into(),
            data: vec![b'a'; MAX_DISPLAY + 10],
        });
        assert!(matches!(detail_content(&text), DetailContent::Text(t) if t.len() == MAX_DISPLAY));

        let png = Ok(ClipboardContent {
            mime_type: "image/png".into(),
            data: vec![0x89, b'P'],
        });
        assert_eq!(detail_content(&png), DetailContent::Image(vec![0x89, b'P']));

        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "hello").expect("written");
        let one = Ok(ClipboardContent {
            mime_type: "text/uri-list".into(),
            data: format!("file://{}\r\n", file.display()).into_bytes(),
        });
        assert!(
            matches!(detail_content(&one), DetailContent::File(preview) if preview.name == "notes.txt")
        );

        let gone = Ok(ClipboardContent {
            mime_type: "text/uri-list".into(),
            data: b"file:///nowhere/at/all\r\n".to_vec(),
        });
        assert!(matches!(detail_content(&gone), DetailContent::Text(_)));

        let failed = Err("decryption failed".to_owned());
        assert!(
            matches!(detail_content(&failed), DetailContent::Error(_, reason) if reason == "decryption failed")
        );
    }

    #[test]
    fn the_pane_follows_the_selection_and_drops_a_late_answer() {
        let mut page = ClipboardPage::default();
        page.apply(
            0,
            Ok(vec![
                row("a", ClipboardRowKind::Text),
                row("b", ClipboardRowKind::Text),
            ]),
        );
        assert_eq!(page.detail_wanted().as_deref(), Some("a"));
        assert_eq!(page.detail_wanted(), None, "already fetching it");
        page.selected = 1;
        assert_eq!(page.detail_wanted().as_deref(), Some("b"));
        page.apply_detail("a", None, Some(Err("late".into())));
        assert_eq!(page.detail.as_ref().and_then(|d| d.content.clone()), None);
        page.apply_detail("b", None, Some(Err("now".into())));
        assert!(page.detail.as_ref().is_some_and(|d| d.content.is_some()));
    }

    #[test]
    fn monitoring_offers_the_opposite_or_says_it_cannot() {
        let on = ClipboardMonitoring {
            supported: true,
            enabled: true,
        };
        let off = ClipboardMonitoring {
            supported: true,
            enabled: false,
        };
        let none = ClipboardMonitoring {
            supported: false,
            enabled: true,
        };
        assert_eq!(monitoring_action(on), Some("Pause clipboard"));
        assert_eq!(monitoring_action(off), Some("Resume clipboard"));
        assert_eq!(monitoring_action(none), None);
        assert_eq!(monitoring_notice(on), None);
        assert_eq!(
            monitoring_notice(none),
            Some("Clipboard monitoring unavailable")
        );
    }

    #[test]
    fn a_link_names_its_host_and_a_pin_is_shown() {
        let mut link = row("l", ClipboardRowKind::Link);
        link.url_host = Some("example.org".into());
        link.pinned = true;
        assert_eq!(subtitle(&link), "Link · example.org · Pinned");
        assert_eq!(subtitle(&row("t", ClipboardRowKind::Text)), "Text");
    }
}
