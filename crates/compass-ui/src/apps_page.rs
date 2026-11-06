//! Browse Apps, Set Default Browser and Set Default Terminal: three lists of
//! applications from the system extension (`src/server/src/builtins/system`).
//!
//! Browse Apps is drawn from the window's own index, through
//! [`compass_core::browse_apps`]; the two pickers ask the engine, which reads
//! the MIME associations and writes the choice. This keeps the rows, their
//! filter and the selection.

use compass_core::AppItem;
use compass_core::browse_apps::{self, BrowseApp, Options};
use compass_search::Query;

use crate::backend::{DefaultApp, DefaultAppRow};

/// Which list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppsKind {
    /// Browse Apps.
    Browse,
    /// Set Default Browser or Set Default Terminal.
    Default(DefaultApp),
}

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The list is on its way.
    Loading,
    /// It arrived.
    Ready,
    /// It cannot be listed, and why.
    Failed(String),
}

/// One application.
#[derive(Debug, Clone)]
pub struct AppRow {
    /// What the model knows about it.
    pub app: BrowseApp,
    /// The indexed entry, to launch and to draw the icon; `None` for a
    /// picker row the window's index does not have.
    pub item: Option<AppItem>,
    /// Whether it is the current default (the pickers only).
    pub is_default: bool,
}

/// The view's state.
#[derive(Debug, Clone)]
pub struct AppsPage {
    /// Which list.
    pub kind: AppsKind,
    /// The filter text.
    pub query: String,
    /// Every application, in the order listed.
    pub rows: Vec<AppRow>,
    /// Positions in `rows` that match `query`, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// What the last action said.
    pub notice: Option<String>,
    /// The selected application's open windows, by its id, as the engine
    /// last answered (Browse Apps' Focus Window).
    pub running: Option<(String, Vec<u32>)>,
}

impl AppsPage {
    /// Browse Apps over `index`, as `BrowseAppsViewHost::reload` lists it.
    #[must_use]
    pub fn browse(index: &compass_core::AppIndex, options: Options) -> Self {
        let rows = browse_apps::listed(index, options)
            .into_iter()
            .map(|(item, displayable)| AppRow {
                app: browse_apps::from_item(item, displayable),
                item: Some(item.clone()),
                is_default: false,
            })
            .collect();
        let mut page = Self::new(AppsKind::Browse, Status::Ready);
        page.rows = rows;
        page.refilter();
        page
    }

    /// A picker waiting for the engine's list.
    #[must_use]
    pub fn picker(kind: DefaultApp) -> Self {
        Self::new(AppsKind::Default(kind), Status::Loading)
    }

    fn new(kind: AppsKind, status: Status) -> Self {
        Self {
            kind,
            query: String::new(),
            rows: Vec::new(),
            shown: Vec::new(),
            selected: 0,
            status,
            notice: None,
            running: None,
        }
    }

    /// Takes the engine's list for a picker, finding each application in
    /// `index` for its icon and keywords.
    pub fn apply(
        &mut self,
        result: Result<Vec<DefaultAppRow>, String>,
        index: &compass_core::AppIndex,
    ) {
        match result {
            Ok(apps) => {
                let service = compass_core::app_service::AppService::new(index);
                self.rows = apps
                    .into_iter()
                    .map(|row| {
                        let item = service.find_by_id(&row.id);
                        AppRow {
                            app: BrowseApp {
                                keywords: item.map(|i| i.keywords().to_vec()).unwrap_or_default(),
                                path: item
                                    .and_then(AppItem::path)
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                                id: row.id,
                                display_name: row.name,
                                description: row.description,
                                displayable: true,
                                actions: Vec::new(),
                            },
                            item: item.cloned(),
                            is_default: row.is_default,
                        }
                    })
                    .collect();
                self.status = Status::Ready;
            }
            Err(reason) => {
                self.rows.clear();
                self.status = Status::Failed(reason);
            }
        }
        self.refilter();
    }

    /// Recomputes `shown` for the current query, back at the top: every row
    /// in order for an empty one, else those that match, best first, with
    /// Browse Apps' field weights (name, description, keywords).
    pub fn refilter(&mut self) {
        self.selected = 0;
        let query = Query::new(&self.query);
        if query.is_empty() {
            self.shown = (0..self.rows.len()).collect();
            return;
        }
        let mut scored: Vec<(u32, usize)> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let found = browse_apps::score(&row.app, &query);
                found.accepted().then_some((found.score, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.shown = scored.into_iter().map(|(_, index)| index).collect();
    }

    /// The selected row, if any.
    #[must_use]
    pub fn selected_row(&self) -> Option<&AppRow> {
        self.rows.get(*self.shown.get(self.selected)?)
    }

    /// What the view says in place of the list: loading, why it cannot be
    /// listed, or that nothing matches; `None` while it shows applications.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Looking for applications…"),
            Status::Failed(reason) => Some(reason),
            Status::Ready if self.shown.is_empty() => Some("No applications match"),
            Status::Ready => None,
        }
    }

    /// The search field's placeholder.
    #[must_use]
    pub const fn placeholder(&self) -> &'static str {
        match self.kind {
            AppsKind::Browse => browse_apps::PLACEHOLDER,
            AppsKind::Default(DefaultApp::Browser) => {
                compass_core::default_app::BROWSER_PLACEHOLDER
            }
            AppsKind::Default(DefaultApp::Terminal) => {
                compass_core::default_app::TERMINAL_PLACEHOLDER
            }
        }
    }

    /// The heading over the rows: `Applications ({count})`, counting what
    /// the filter leaves, or the picker's section.
    #[must_use]
    pub fn heading(&self) -> String {
        match self.kind {
            AppsKind::Browse => {
                browse_apps::SECTION_TITLE.replace("{count}", &self.shown.len().to_string())
            }
            AppsKind::Default(DefaultApp::Browser) => {
                compass_core::default_app::BROWSER_SECTION.to_owned()
            }
            AppsKind::Default(DefaultApp::Terminal) => {
                compass_core::default_app::TERMINAL_SECTION.to_owned()
            }
        }
    }
}

/// The text at a row's right: Browse Apps' "Hidden", or the pickers' check
/// on the current default.
#[must_use]
pub fn accessory(row: &AppRow) -> Option<String> {
    if row.is_default {
        return Some("✓ Default".to_owned());
    }
    browse_apps::accessories(&row.app)
        .first()
        .map(|text| (*text).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(dir: &std::path::Path) -> compass_core::AppIndex {
        for (file, body) in [
            ("vim.desktop", "Name=Vim\nComment=Edit text\nKeywords=vi;\n"),
            ("files.desktop", "Name=Files\n"),
            ("probe.desktop", "Name=Probe\nNoDisplay=true\n"),
        ] {
            std::fs::write(
                dir.join(file),
                format!("[Desktop Entry]\nType=Application\nExec=x\n{body}"),
            )
            .unwrap();
        }
        compass_core::AppIndex::builder().dir(dir).build()
    }

    #[test]
    fn browse_apps_filters_on_the_name_the_comment_and_the_keywords() {
        let dir = tempfile::tempdir().unwrap();
        let index = index(dir.path());
        let mut page = AppsPage::browse(&index, Options::default());
        assert_eq!(page.heading(), "Applications (2)");
        assert_eq!(page.placeholder(), "Search apps...");
        page.query = "vi".into();
        page.refilter();
        assert_eq!(
            page.selected_row().map(|row| row.app.id.as_str()),
            Some("vim.desktop")
        );
        page.query = "edit text".into();
        page.refilter();
        assert_eq!(page.heading(), "Applications (1)");

        let all = AppsPage::browse(
            &index,
            Options {
                show_hidden: true,
                ..Options::default()
            },
        );
        let probe = all.rows.iter().find(|row| row.app.id == "probe.desktop");
        assert_eq!(probe.and_then(accessory).as_deref(), Some("Hidden"));
    }

    #[test]
    fn a_picker_takes_the_engines_order_and_marks_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let index = index(dir.path());
        let mut page = AppsPage::picker(DefaultApp::Terminal);
        assert_eq!(page.status, Status::Loading);
        assert_eq!(page.placeholder(), "Select a terminal emulator...");
        page.apply(
            Ok(vec![
                DefaultAppRow {
                    id: "files.desktop".into(),
                    name: "Files".into(),
                    is_default: true,
                    ..DefaultAppRow::default()
                },
                DefaultAppRow {
                    id: "gone.desktop".into(),
                    name: "Gone".into(),
                    ..DefaultAppRow::default()
                },
            ]),
            &index,
        );
        assert_eq!(page.heading(), "Available terminal emulators");
        assert_eq!(page.shown, [0, 1]);
        assert!(page.rows[0].item.is_some());
        assert!(page.rows[1].item.is_none(), "not in this window's index");
        assert_eq!(accessory(&page.rows[0]).as_deref(), Some("✓ Default"));
        assert_eq!(accessory(&page.rows[1]), None);
        page.apply(Err("The Compass engine is not running".into()), &index);
        assert!(page.rows.is_empty());
    }
}
