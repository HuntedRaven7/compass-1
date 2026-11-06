//! The two extension stores in the launcher: the list, the detail page, and
//! what an install or an uninstall changes on each.
//!
//! State only; the messages and drawing are in `app::stores`. Both stores
//! share one page, as the C++ shares `StoreListingView` and
//! `StoreDetailView`: what differs is where the rows come from (the Vicinae
//! store's whole list filtered as you type, the Raycast store searched on the
//! server after a 200 ms pause) and the Raycast rows' compatibility.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::backend::{Store, StoreDetail, StoreList, StoreRow};
use crate::extension_page::RowIcon;

/// Where the list is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The first fetch has not answered.
    Loading,
    /// Rows are showing.
    Ready,
    /// The first fetch failed, and why.
    Failed(String),
}

/// What Enter would confirm, while the uninstall question is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    /// The installed id.
    pub id: String,
    /// Its title, for the question.
    pub title: String,
}

/// The question `UninstallExtensionAction` asks.
pub const CONFIRM_TITLE: &str = "Are you sure?";

/// The rest of it.
pub const CONFIRM_MESSAGE: &str = "All this extension data will be permanently lost. If you just \
     want the extension to not appear in the root search anymore, consider disabling it instead.";

/// The button that answers [`CONFIRM_TITLE`] yes, and what Enter does.
#[must_use]
pub fn confirm_button(title: &str) -> String {
    format!("Uninstall {title}")
}

/// What a successful uninstall says.
pub const UNINSTALLED: &str = "Extension uninstalled";

/// What a successful install says.
pub const INSTALLED: &str = "Extension installed";

/// What an install in progress says.
pub const DOWNLOADING: &str = "Downloading extension...";

/// The colour a compatibility tier's dot is drawn in, as RGB: green,
/// orange, red, and grey for unknown.
#[must_use]
pub const fn compat_colour(tier: u8) -> (u8, u8, u8) {
    match tier {
        0 => (0x3a, 0x9c, 0x61),
        1 => (0xf0, 0x88, 0x3e),
        2 => (0xb9, 0x54, 0x3b),
        _ => (0x8a, 0x8a, 0x8a),
    }
}

/// The text of a row's accessory before the compatibility dot: installed or
/// out of date, and the download count.
#[must_use]
pub fn status_text(row: &StoreRow) -> String {
    let mut parts = Vec::new();
    if row.update_available {
        parts.push("Update available".to_owned());
    } else if row.installed {
        parts.push("Installed".to_owned());
    }
    if !row.downloads.is_empty() {
        parts.push(format!("↓ {}", row.downloads));
    }
    parts.join(" · ")
}

/// Every image URL a Markdown document shows, in order.
#[must_use]
pub fn markdown_images(items: &[iced::widget::markdown::Item]) -> Vec<String> {
    use iced::widget::markdown::Item;
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Image { url, .. } => out.push(url.clone()),
            Item::Quote(inner) => out.extend(markdown_images(inner)),
            _ => {}
        }
    }
    out
}

/// The words a compatibility tier is shown as.
#[must_use]
pub const fn compat_label(tier: u8) -> &'static str {
    match tier {
        0 => "Compatible",
        1 => "Partial",
        2 => "Incompatible",
        _ => "Unknown",
    }
}

/// The text at a row's right: installed or out of date, the download
/// count, and the compatibility tier where there is one.
#[must_use]
pub fn accessory(row: &StoreRow) -> String {
    let mut parts = Vec::new();
    if row.update_available {
        parts.push("Update available".to_owned());
    } else if row.installed {
        parts.push("Installed".to_owned());
    }
    if !row.downloads.is_empty() {
        parts.push(format!("↓ {}", row.downloads));
    }
    if let Some(tier) = row.compat {
        parts.push(compat_label(tier).to_owned());
    }
    parts.join(" · ")
}

/// The line under a detail page's title, and what it is: the row's
/// [`accessory`], or its author when that says nothing.
#[must_use]
pub fn detail_status(row: &StoreRow) -> (&'static str, String) {
    let status = accessory(row);
    if status.is_empty() {
        ("Author", row.author_name.clone())
    } else {
        ("Status", status)
    }
}

/// The icon URL a row shows under the theme.
#[must_use]
pub fn icon_url(row: &StoreRow, dark: bool) -> Option<&str> {
    let (preferred, other) = if dark {
        (&row.icon_dark, &row.icon_light)
    } else {
        (&row.icon_light, &row.icon_dark)
    };
    preferred
        .as_deref()
        .or(other.as_deref())
        .filter(|url| crate::remote_image::is_remote(url))
}

/// Remote images a page shows, fetched once each.
#[derive(Debug, Clone, Default)]
pub struct Images {
    /// What arrived, by URL.
    pub art: HashMap<String, RowIcon>,
    requested: HashSet<String>,
}

impl Images {
    /// Those of `urls` nobody has asked for yet; each is returned once.
    pub fn wanted<'a>(&mut self, urls: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut wanted = Vec::new();
        for url in urls {
            if self.requested.insert(url.to_owned()) {
                wanted.push(url.to_owned());
            }
        }
        wanted
    }

    /// An image arrived in the cache, or could not be fetched.
    pub fn arrived(&mut self, url: String, fetched: Result<std::path::PathBuf, String>) {
        match fetched.map(|path| crate::icons::classify(&path)) {
            Ok(Some(art)) => {
                self.art.insert(
                    url,
                    RowIcon::Art {
                        art,
                        monochrome: false,
                        tint: None,
                    },
                );
            }
            Ok(None) => tracing::debug!(%url, "a store image the launcher cannot draw"),
            Err(reason) => tracing::debug!(%url, %reason, "a store image was not fetched"),
        }
    }
}

/// A store's list.
#[derive(Debug, Clone)]
pub struct StorePage {
    /// Which store.
    pub store: Store,
    /// The search text.
    pub query: String,
    /// Where the first fetch is.
    pub status: Status,
    /// The heading over the rows.
    pub heading: String,
    /// The rows.
    pub rows: Vec<StoreRow>,
    /// The selected row.
    pub selected: usize,
    /// A failure or a confirmation, shown under the list.
    pub notice: Option<String>,
    /// Bumped by every change of the search text, so a late answer to an
    /// older one is dropped (`query_result_is_current`).
    pub generation: u64,
    /// Whether a fetch is in flight.
    pub loading: bool,
    /// The uninstall question, while it is showing.
    pub confirm: Option<Confirm>,
    /// The rows' icons.
    pub images: Images,
}

impl StorePage {
    /// An empty list for `store`, waiting on its first fetch.
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self {
            store,
            query: String::new(),
            status: Status::Loading,
            heading: compass_core::raycast_store_view::LIST_HEADING.to_owned(),
            rows: Vec::new(),
            selected: 0,
            notice: None,
            generation: 0,
            loading: true,
            confirm: None,
            images: Images::default(),
        }
    }

    /// Takes new search text, answering the generation it starts.
    pub fn set_query(&mut self, query: String) -> u64 {
        self.query = query;
        self.notice = None;
        self.confirm = None;
        self.generation += 1;
        self.generation
    }

    /// How long to wait before asking for the current text.
    ///
    /// The Raycast store searches on the server, so typing settles for
    /// [`compass_core::raycast_store_view::SEARCH_DEBOUNCE_MS`] first; emptying the
    /// box reloads the list at once. The Vicinae store filters the list it
    /// already has, so it never waits.
    #[must_use]
    pub fn debounce(&self) -> Option<Duration> {
        (self.store == Store::Raycast
            && compass_core::raycast_store_view::text_starts_search(&self.query))
        .then(|| Duration::from_millis(compass_core::raycast_store_view::SEARCH_DEBOUNCE_MS))
    }

    /// Takes a fetch's answer, if it is for the text now in the box.
    ///
    /// A failure after rows have shown keeps them and says why beneath, as
    /// the C++ keeps its model and raises a toast; a first fetch that fails
    /// has nothing to keep.
    pub fn apply(&mut self, generation: u64, result: Result<StoreList, String>) {
        if generation != self.generation {
            return;
        }
        self.loading = false;
        match result {
            Ok(list) => {
                let selected_id = self.selected_row().map(|row| row.id.clone());
                self.heading = list.heading;
                self.rows = list.rows;
                self.selected = selected_id
                    .and_then(|id| self.rows.iter().position(|row| row.id == id))
                    .unwrap_or(0);
                self.status = Status::Ready;
            }
            Err(reason) if self.status == Status::Ready => self.notice = Some(reason),
            Err(reason) => self.status = Status::Failed(reason),
        }
    }

    /// The selected row.
    #[must_use]
    pub fn selected_row(&self) -> Option<&StoreRow> {
        self.rows.get(self.selected)
    }

    /// What the view says in place of the rows: loading, why the first
    /// fetch failed, or that nothing matches; `None` while rows show.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Loading extensions…"),
            Status::Failed(reason) => Some(reason),
            Status::Ready if self.rows.is_empty() => Some("No extensions match"),
            Status::Ready => None,
        }
    }

    /// Records that `id` was installed (at the store's current build) or
    /// removed.
    pub fn mark(&mut self, id: &str, installed: bool) {
        for row in self.rows.iter_mut().filter(|row| row.id == id) {
            row.installed = installed;
            row.update_available = false;
        }
    }

    /// The rows' icon and avatar URLs not yet asked for.
    pub fn wanted_images(&mut self, dark: bool) -> Vec<String> {
        let urls: Vec<String> = self
            .rows
            .iter()
            .flat_map(|row| {
                [
                    icon_url(row, dark).map(str::to_owned),
                    row.author_avatar
                        .clone()
                        .filter(|url| crate::remote_image::is_remote(url)),
                ]
            })
            .flatten()
            .collect();
        self.images.wanted(urls.iter().map(String::as_str))
    }
}

/// One extension's detail page.
#[derive(Debug, Clone)]
pub struct StoreDetailPage {
    /// Which store.
    pub store: Store,
    /// Everything the engine sent.
    pub detail: StoreDetail,
    /// The navigation title: the store's name, then the extension's.
    pub title: String,
    /// Its Markdown, parsed.
    pub markdown: Vec<iced::widget::markdown::Item>,
    /// An install or an uninstall in progress, and what it says.
    pub busy: Option<String>,
    /// What the last action said.
    pub notice: Option<String>,
    /// Whether the uninstall question is showing.
    pub confirm: bool,
    /// The icon and screenshots.
    pub images: Images,
}

impl StoreDetailPage {
    /// The page for `detail`.
    #[must_use]
    pub fn new(store: Store, detail: StoreDetail) -> Self {
        let markdown = iced::widget::markdown::parse(&detail.markdown).collect();
        let title = match store {
            Store::Raycast => {
                compass_core::raycast_store_view::detail_navigation_title(&detail.row.title)
            }
            Store::Vicinae => format!("Vicinae Store - {}", detail.row.title),
        };
        Self {
            store,
            detail,
            title,
            markdown,
            busy: None,
            notice: None,
            confirm: false,
            images: Images::default(),
        }
    }

    /// The icon, avatar, screenshot and README image URLs not yet asked
    /// for.
    pub fn wanted_images(&mut self, dark: bool) -> Vec<String> {
        let mut urls: Vec<String> = icon_url(&self.detail.row, dark)
            .map(str::to_owned)
            .into_iter()
            .collect();
        urls.extend(self.detail.row.author_avatar.clone());
        urls.extend(self.detail.screenshots.iter().cloned());
        urls.extend(markdown_images(&self.markdown));
        urls.retain(|url| crate::remote_image::is_remote(url));
        self.images.wanted(urls.iter().map(String::as_str))
    }
}

/// The id of each action the store pages offer.
pub mod actions {
    /// Show the selected row's detail page.
    pub const DETAILS: &str = "store.details";
    /// Install (or update) the extension.
    pub const INSTALL: &str = "store.install";
    /// Ask to uninstall it.
    pub const UNINSTALL: &str = "store.uninstall";
    /// Open its README in the browser.
    pub const README: &str = "store.readme";
    /// Open its source in the browser.
    pub const SOURCE: &str = "store.source";
    /// Open its page on the store's website.
    pub const WEBSITE: &str = "store.website";
    /// Open the extensions repository's new-issue page.
    pub const REPORT: &str = "store.report";
}

/// The detail page's actions, in order, as `(id, title, shortcut)`: install
/// or update when it applies, uninstall when installed, then the links.
#[must_use]
pub fn detail_actions(
    page: &StoreDetailPage,
) -> Vec<(&'static str, &'static str, Option<&'static str>)> {
    let row = &page.detail.row;
    let mut out = Vec::new();
    if !row.installed {
        out.push((actions::INSTALL, "Install extension", Some("enter")));
    } else if row.update_available {
        out.push((actions::INSTALL, "Update extension", Some("enter")));
    }
    if row.installed {
        let shortcut = (!row.update_available).then_some("enter");
        out.push((actions::UNINSTALL, "Uninstall Extension", shortcut));
    }
    if page.detail.readme_url.is_some() {
        out.push((actions::README, "Open README", None));
    }
    if page.detail.source_url.is_some() {
        out.push((actions::SOURCE, "View source code", None));
    }
    if page.detail.store_url.is_some() {
        out.push((actions::WEBSITE, "Open in Raycast Store", None));
    }
    out.push((actions::REPORT, "Report issue", None));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> StoreRow {
        StoreRow {
            id: id.into(),
            title: id.into(),
            downloads: "1.1K".into(),
            ..StoreRow::default()
        }
    }

    #[test]
    fn a_late_answer_for_older_text_is_dropped() {
        let mut page = StorePage::new(Store::Raycast);
        let first = page.set_query("spo".into());
        let second = page.set_query("spotify".into());
        page.apply(
            first,
            Ok(StoreList {
                heading: "Results".into(),
                rows: vec![row("old")],
            }),
        );
        assert!(page.rows.is_empty());
        page.apply(
            second,
            Ok(StoreList {
                heading: "Results".into(),
                rows: vec![row("new")],
            }),
        );
        assert_eq!(page.rows[0].id, "new");
        assert_eq!(page.status, Status::Ready);
    }

    #[test]
    fn only_the_raycast_store_waits_and_only_for_text() {
        let mut raycast = StorePage::new(Store::Raycast);
        assert_eq!(raycast.debounce(), None, "the list reloads at once");
        raycast.set_query("x".into());
        assert_eq!(raycast.debounce(), Some(Duration::from_millis(200)));
        let mut vicinae = StorePage::new(Store::Vicinae);
        vicinae.set_query("x".into());
        assert_eq!(vicinae.debounce(), None);
    }

    #[test]
    fn a_failure_after_rows_keeps_them() {
        let mut page = StorePage::new(Store::Vicinae);
        page.apply(
            0,
            Ok(StoreList {
                heading: "Extensions".into(),
                rows: vec![row("a")],
            }),
        );
        page.apply(0, Err("offline".into()));
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.notice.as_deref(), Some("offline"));
        let mut fresh = StorePage::new(Store::Vicinae);
        fresh.apply(0, Err("offline".into()));
        assert_eq!(fresh.status, Status::Failed("offline".into()));
    }

    #[test]
    fn the_accessory_says_installed_or_out_of_date_and_the_tier() {
        let mut r = row("a");
        assert_eq!(accessory(&r), "↓ 1.1K");
        r.installed = true;
        r.compat = Some(1);
        assert_eq!(accessory(&r), "Installed · ↓ 1.1K · Partial");
        r.update_available = true;
        assert!(accessory(&r).starts_with("Update available"));
    }

    #[test]
    fn the_detail_offers_install_update_or_uninstall_first() {
        let mut page = StoreDetailPage::new(
            Store::Raycast,
            StoreDetail {
                row: row("a"),
                store_url: Some("https://www.raycast.com/a/a".into()),
                ..StoreDetail::default()
            },
        );
        let ids = |page: &StoreDetailPage| {
            detail_actions(page)
                .into_iter()
                .map(|(id, _, _)| id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&page),
            [actions::INSTALL, actions::WEBSITE, actions::REPORT]
        );
        page.detail.row.installed = true;
        assert_eq!(
            ids(&page),
            [actions::UNINSTALL, actions::WEBSITE, actions::REPORT]
        );
        page.detail.row.update_available = true;
        assert_eq!(
            ids(&page)[..2],
            [actions::INSTALL, actions::UNINSTALL],
            "an update is the primary action"
        );
    }

    #[test]
    fn icons_prefer_the_themes_side_and_are_asked_for_once() {
        let mut page = StorePage::new(Store::Vicinae);
        let mut r = row("a");
        r.icon_light = Some("https://example.com/l.png".into());
        r.icon_dark = Some("https://example.com/d.png".into());
        page.rows = vec![r.clone()];
        assert_eq!(icon_url(&r, true), Some("https://example.com/d.png"));
        assert_eq!(page.wanted_images(false), ["https://example.com/l.png"]);
        assert!(page.wanted_images(false).is_empty());
    }
}
