//! The view an extension command draws, as the launcher shows it.
//!
//! The engine runs the command and publishes each render as a
//! [`compass_extension_api::View`]; this is what the launcher keeps of it,
//! and every decision about it that is not drawing: which rows the search
//! leaves, which one is selected, and which callback Enter runs.
//!
//! # Who filters
//!
//! A `List` either lets the host filter it (the default) or takes the search
//! text itself (`onSearchTextChange`). The first is filtered here, fuzzily
//! over title, subtitle and keywords; the second is left alone and the text
//! goes to the extension, which renders a new list.

use compass_extension_api::View;
use compass_extension_api::action::{ActionPanel, HandlerId};
use compass_extension_api::view::ListItem;

/// What a row's icon is drawn from, resolved once per render.
#[derive(Debug, Clone, PartialEq)]
pub enum RowIcon {
    /// A file. A builtin is monochrome and drawn in `tint`, else the text
    /// colour; other art keeps its own colours.
    Art {
        /// The file.
        art: crate::icons::IconArt,
        /// Whether it is one of the builtin, single-colour icons.
        monochrome: bool,
        /// The colour the extension asked for.
        tint: Option<iced::Color>,
    },
    /// A flat colour: a grid cell whose content is one.
    Swatch(iced::Color),
    /// An emoji or another glyph, drawn as text.
    Text(String),
}

/// What [`compass_core::image_url::ImageUrl::from_source`] asks of an
/// extension's view: its builtin icons, its assets and the icon theme.
struct PageLookup<'a> {
    assets: Option<&'a std::path::Path>,
    icon_lookup: Option<&'a crate::app::IconLookup>,
}

impl compass_core::image_url::SourceLookup for PageLookup<'_> {
    fn builtin(&self, name: &str) -> bool {
        compass_core::builtin_icon::path(name).is_some()
    }
    fn file(&self, path: &str) -> bool {
        let path = std::path::Path::new(path);
        path.is_absolute() && path.is_file()
    }
    fn asset(&self, relative: &str) -> Option<String> {
        let path = self.assets?.join(relative);
        path.is_file().then(|| path.to_string_lossy().into_owned())
    }
    fn themed(&self, name: &str) -> bool {
        self.icon_lookup
            .is_some_and(|lookup| lookup.find(name).is_some())
    }
}

/// Where the command is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Started; nothing rendered yet.
    Loading,
    /// Showing its view.
    Ready,
    /// It cannot be shown, or it ended: why.
    Stopped(String),
}

/// One extension command's view.
#[derive(Debug, Clone)]
pub struct ExtensionPage {
    /// The engine's session for it.
    pub session: u64,
    /// The command's title, until the view names itself.
    pub title: String,
    /// The last version the engine sent.
    pub version: u64,
    /// The latest view.
    pub view: Option<View>,
    /// Where it is.
    pub status: Status,
    /// The search text.
    pub query: String,
    /// How many times the search text changed, for the extension's echo
    /// counter (ADR-0009).
    pub query_events: u64,
    /// The search text when the shown view arrived: a view for a different
    /// text is a new search, which starts at its first row.
    rendered_query: String,
    /// Rows the search leaves, as `(section, item)` in the view.
    pub shown: Vec<(usize, usize)>,
    /// Position in `shown`.
    pub selected: usize,
    /// What went wrong with the last action, if anything.
    pub notice: Option<String>,
    /// A confirmation the extension waits on; Enter and Escape answer it.
    pub alert: Option<crate::backend::ExtensionPrompt>,
    /// The toast the extension shows, drawn under its view.
    pub toast: Option<crate::backend::ExtensionToast>,
    /// How many views the extension has pushed; Escape pops above one.
    pub depth: u32,
    /// A detail's Markdown, parsed once per render rather than per frame.
    pub markdown: Vec<iced::widget::markdown::Item>,
    /// A form's values as the person has them, by field name.
    pub form_values: serde_json::Map<String, serde_json::Value>,
    /// How many times the person has edited each field, for `onChange`'s
    /// echo count (ADR-0009).
    pub form_edits: std::collections::BTreeMap<String, u64>,
    /// Each text area's editor, by field name: a multi-line field keeps its
    /// cursor and selection here, and its text in `form_values`.
    pub editors: std::collections::BTreeMap<String, iced::widget::text_editor::Content>,
    /// The extension's `assets` directory, which `Image` paths are relative to.
    pub assets: Option<std::path::PathBuf>,
    /// Whether the launcher is dark, for themed images.
    pub prefers_dark: bool,
    /// How a file icon's theme name becomes a file; `None` draws the
    /// builtin fallback without looking in a theme.
    pub icon_lookup: Option<crate::app::IconLookup>,
    /// Each row's icon, by `(section, item)` like the list.
    pub icons: Vec<Vec<Option<RowIcon>>>,
    /// Rows whose icon is a remote image, and its URL.
    pub remote_rows: std::collections::BTreeMap<(usize, usize), String>,
    /// Rows whose image is clipped (`Image.mask`), and to what.
    pub masks: std::collections::BTreeMap<(usize, usize), compass_core::image_url::ImageMask>,
    /// The images a detail's Markdown shows that have been fetched, by URL.
    pub markdown_art: std::collections::HashMap<String, RowIcon>,
    /// For a grid, each section's column count (the section's, else the
    /// grid's, else `SectionGridModel`'s eight); `None` for any other view.
    pub grid_columns: Option<Vec<usize>>,
    /// Remote images fetched so far, by URL.
    pub remote_art: std::collections::HashMap<String, RowIcon>,
    /// Remote images already asked for, fetched or not, so a re-render does
    /// not ask again.
    pub requested: std::collections::BTreeSet<String>,
    /// Each date field's text as typed, by field name, while it is not yet
    /// a date; a field with no draft shows its value.
    pub date_drafts: std::collections::BTreeMap<String, String>,
    /// Whether the view ending without a problem goes back to the root
    /// search: a Rhai script's does, as popping its only view.
    pub leaves_on_end: bool,
}

impl ExtensionPage {
    /// A page for a session that has just started.
    #[must_use]
    pub fn new(session: u64, title: impl Into<String>) -> Self {
        Self {
            session,
            title: title.into(),
            version: 0,
            view: None,
            status: Status::Loading,
            query: String::new(),
            query_events: 0,
            rendered_query: String::new(),
            shown: Vec::new(),
            selected: 0,
            notice: None,
            alert: None,
            toast: None,
            depth: 1,
            markdown: Vec::new(),
            form_values: serde_json::Map::new(),
            form_edits: std::collections::BTreeMap::new(),
            editors: std::collections::BTreeMap::new(),
            assets: None,
            prefers_dark: false,
            icon_lookup: None,
            masks: std::collections::BTreeMap::new(),
            markdown_art: std::collections::HashMap::new(),
            grid_columns: None,
            icons: Vec::new(),
            remote_rows: std::collections::BTreeMap::new(),
            remote_art: std::collections::HashMap::new(),
            requested: std::collections::BTreeSet::new(),
            date_drafts: std::collections::BTreeMap::new(),
            leaves_on_end: false,
        }
    }

    /// Whether `state` only repeats what the page shows: the engine's answer
    /// to a poll it held for its whole hold with nothing new. The page asks
    /// again rather than redrawing.
    #[must_use]
    pub fn is_unchanged(&self, state: &crate::backend::ExtensionViewState) -> bool {
        state.version == self.version && !state.ended && state.problem.is_none()
    }

    /// Takes the engine's latest answer.
    pub fn apply(&mut self, state: crate::backend::ExtensionViewState) {
        self.version = state.version;
        self.alert = state.alert;
        self.toast = state.toast;
        if state.view.is_some() {
            if state.depth != self.depth {
                // A different screen: its search starts empty, as Raycast's does.
                self.query.clear();
                self.selected = 0;
                self.form_values.clear();
                self.form_edits.clear();
                self.editors.clear();
                self.date_drafts.clear();
            }
            self.depth = state.depth.max(1);
        }
        if let Some(view) = state.view {
            // A grid is shown as rows until the launcher draws image tiles:
            // the same search, selection and actions, one cell per row.
            let view = match *view {
                View::Grid(grid) => {
                    let default = grid.columns.map_or(DEFAULT_GRID_COLUMNS, usize::from);
                    self.grid_columns = Some(
                        grid.sections
                            .iter()
                            .map(|section| section.columns.map_or(default, usize::from).max(1))
                            .collect(),
                    );
                    self.masks = masks_of(grid.sections.iter().map(|s| {
                        s.items.iter().map(|c| match &c.content {
                            compass_extension_api::view::GridContent::Image(image) => image.mask,
                            compass_extension_api::view::GridContent::Color(_) => None,
                        })
                    }));
                    self.icons = grid
                        .sections
                        .iter()
                        .map(|s| s.items.iter().map(|c| self.cell_icon(&c.content)).collect())
                        .collect();
                    self.remote_rows = rows_with(grid.sections.iter().map(|s| {
                        s.items.iter().map(|c| match &c.content {
                            compass_extension_api::view::GridContent::Image(image) => {
                                self.remote_url(image)
                            }
                            compass_extension_api::view::GridContent::Color(_) => None,
                        })
                    }));
                    Box::new(View::List(grid_as_list(grid)))
                }
                other => {
                    self.grid_columns = None;
                    self.masks = match &other {
                        View::List(list) => masks_of(list.sections.iter().map(|s| {
                            s.items
                                .iter()
                                .map(|item| item.icon.as_ref().and_then(|i| i.mask))
                        })),
                        _ => std::collections::BTreeMap::new(),
                    };
                    self.remote_rows = match &other {
                        View::List(list) => rows_with(list.sections.iter().map(|s| {
                            s.items
                                .iter()
                                .map(|item| item.icon.as_ref().and_then(|i| self.remote_url(i)))
                        })),
                        _ => std::collections::BTreeMap::new(),
                    };
                    self.icons = match &other {
                        View::List(list) => list
                            .sections
                            .iter()
                            .map(|s| {
                                s.items
                                    .iter()
                                    .map(|item| item.icon.as_ref().and_then(|i| self.image_icon(i)))
                                    .collect()
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    Box::new(other)
                }
            };
            let key = self.selected_item().and_then(|item| item.key.clone());
            self.markdown = match view.as_ref() {
                View::Detail(detail) => detail
                    .markdown
                    .as_deref()
                    .map(|text| iced::widget::markdown::parse(text).collect())
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            if let View::Form(form) = view.as_ref() {
                self.take_form_values(form);
            }
            self.view = Some(*view);
            self.status = Status::Ready;
            self.refilter();
            // Keep the selection on the same row across a re-render of the
            // same search (a timer, a toast). A new search starts at its first
            // row, as Raycast's list does.
            if self.rendered_query == self.query {
                if let Some(key) = key
                    && let Some(position) = self.shown.iter().position(|&(s, i)| {
                        self.item(s, i).and_then(|it| it.key.as_deref()) == Some(&key)
                    })
                {
                    self.selected = position;
                }
            } else {
                self.selected = 0;
                self.rendered_query.clone_from(&self.query);
            }
        }
        if let Some(problem) = state.problem {
            self.status = Status::Stopped(problem);
        } else if state.ended && self.status != Status::Ready {
            self.status = Status::Stopped(format!("{} finished", self.title));
        }
    }

    /// The icon of the row at `(section, item)`.
    #[must_use]
    pub fn icon(&self, section: usize, item: usize) -> Option<&RowIcon> {
        self.icons
            .get(section)?
            .get(item)?
            .as_ref()
            .or_else(|| self.remote_art.get(self.remote_rows.get(&(section, item))?))
    }

    /// The shown cells grouped by section, in order: each section and the
    /// positions in `shown` of its cells.
    #[must_use]
    pub fn grid_groups(&self) -> Vec<(usize, Vec<usize>)> {
        let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
        for (position, &(section, _)) in self.shown.iter().enumerate() {
            match groups.last_mut() {
                Some((last, cells)) if *last == section => cells.push(position),
                _ => groups.push((section, vec![position])),
            }
        }
        groups
    }

    /// How many columns the grid's `section` has.
    #[must_use]
    pub fn section_columns(&self, section: usize) -> usize {
        self.grid_columns
            .as_ref()
            .and_then(|columns| columns.get(section).copied())
            .unwrap_or(DEFAULT_GRID_COLUMNS)
    }

    /// Where the selection goes in a grid, as `SectionGridModel` navigates:
    /// Left and Right through the cells in reading order across sections;
    /// Up and Down by the section's columns, into the next section's first
    /// row (or the previous one's last) keeping the column; wrapping only
    /// when `wrap`.
    #[must_use]
    pub fn grid_step(&self, step: crate::fonts_page::GridMove, wrap: bool) -> usize {
        use crate::fonts_page::GridMove;
        let total = self.shown.len();
        if total == 0 {
            return 0;
        }
        let current = self.selected.min(total - 1);
        let groups = self.grid_groups();
        let Some((group, item)) = groups
            .iter()
            .enumerate()
            .find_map(|(g, (_, cells))| cells.iter().position(|&p| p == current).map(|i| (g, i)))
        else {
            return current;
        };
        let columns = |g: usize| self.section_columns(groups[g].0);
        let last_row_cell = |g: usize, column: usize| {
            let count = groups[g].1.len();
            let cols = columns(g);
            let last_row = (count - 1) / cols;
            groups[g].1[(last_row * cols + column.min(cols - 1)).min(count - 1)]
        };
        let first_row_cell = |g: usize, column: usize| {
            let count = groups[g].1.len();
            groups[g].1[column.min(columns(g).min(count) - 1)]
        };
        let cols = columns(group);
        let column = item % cols;
        let cells = &groups[group].1;
        match step {
            GridMove::Right if current + 1 < total => current + 1,
            GridMove::Right => {
                if wrap {
                    0
                } else {
                    current
                }
            }
            GridMove::Left if current > 0 => current - 1,
            GridMove::Left => {
                if wrap {
                    total - 1
                } else {
                    current
                }
            }
            GridMove::Down => {
                let next_row = item / cols + 1;
                if next_row <= (cells.len() - 1) / cols {
                    cells[(next_row * cols + column).min(cells.len() - 1)]
                } else if group + 1 < groups.len() {
                    first_row_cell(group + 1, column)
                } else if wrap {
                    first_row_cell(0, column)
                } else {
                    current
                }
            }
            GridMove::Up => {
                let row = item / cols;
                if row > 0 {
                    cells[(row - 1) * cols + column]
                } else if group > 0 {
                    last_row_cell(group - 1, column)
                } else if wrap {
                    last_row_cell(groups.len() - 1, column)
                } else {
                    current
                }
            }
        }
    }

    /// How the row at `(section, item)` clips its image.
    #[must_use]
    pub fn mask(&self, section: usize, item: usize) -> compass_core::image_url::ImageMask {
        self.masks
            .get(&(section, item))
            .copied()
            .unwrap_or_default()
    }

    /// The remote images this view shows that nobody has asked for yet,
    /// its rows' and its Markdown's; each is returned once.
    pub fn wanted_images(&mut self) -> Vec<String> {
        let markdown = crate::store_page::markdown_images(&self.markdown);
        let wanted: Vec<String> = self
            .remote_rows
            .values()
            .chain(
                markdown
                    .iter()
                    .filter(|url| crate::remote_image::is_remote(url)),
            )
            .filter(|url| !self.requested.contains(*url))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        self.requested.extend(wanted.iter().cloned());
        wanted
    }

    /// A remote image arrived in the cache at `path`, or could not be fetched.
    pub fn image_arrived(&mut self, url: String, fetched: Result<std::path::PathBuf, String>) {
        match fetched.map(|path| crate::icons::classify(&path)) {
            Ok(Some(art)) => {
                if crate::store_page::markdown_images(&self.markdown).contains(&url) {
                    self.markdown_art.insert(
                        url.clone(),
                        RowIcon::Art {
                            art: art.clone(),
                            monochrome: false,
                            tint: None,
                        },
                    );
                }
                self.remote_art.insert(
                    url,
                    RowIcon::Art {
                        art,
                        monochrome: false,
                        tint: None,
                    },
                );
            }
            Ok(None) => tracing::debug!(%url, "a fetched image the launcher cannot draw"),
            Err(reason) => tracing::debug!(%url, %reason, "an extension image was not fetched"),
        }
    }

    /// The URL of `image` when it is a remote one, after the theme has
    /// picked a side.
    fn remote_url(&self, image: &compass_extension_api::view::Image) -> Option<String> {
        use compass_extension_api::view::ImageSource;
        let mut source = &image.source;
        while let ImageSource::Themed { light, dark } = source {
            source = if self.prefers_dark { dark } else { light };
        }
        match source {
            ImageSource::Url(url) if crate::remote_image::is_remote(url) => Some(url.clone()),
            _ => None,
        }
    }

    fn cell_icon(&self, content: &compass_extension_api::view::GridContent) -> Option<RowIcon> {
        match content {
            compass_extension_api::view::GridContent::Image(image) => self.image_icon(image),
            compass_extension_api::view::GridContent::Color(color) => {
                color_of(color).map(RowIcon::Swatch)
            }
        }
    }

    /// The file an `Image` is drawn from: a builtin icon, a file in the
    /// extension's assets, or a `file://` URL. A remote URL is resolved
    /// separately, once it has been fetched ([`Self::remote_url`]); a file
    /// icon is its file-type icon ([`crate::icons::file_glyph`]).
    fn image_icon(&self, image: &compass_extension_api::view::Image) -> Option<RowIcon> {
        use compass_extension_api::view::ImageSource;
        let tint = image.tint.as_ref().and_then(color_of);
        let mut source = &image.source;
        while let ImageSource::Themed { light, dark } = source {
            source = if self.prefers_dark { dark } else { light };
        }
        let (path, monochrome) = match source {
            ImageSource::Builtin(name) => match self.builtin_path(name) {
                Some(path) => (path, true),
                // Not a builtin: what `ImageURL(source)` makes of the string.
                None => return self.source_icon(name, tint),
            },
            ImageSource::Url(url)
                if !url.starts_with("file://") && !crate::remote_image::is_remote(url) =>
            {
                return self.source_icon(url, tint);
            }
            ImageSource::Asset(relative) => {
                let path = self.assets.as_ref()?.join(relative);
                (path.is_file().then_some(path)?, false)
            }
            ImageSource::Url(url) => {
                let path = std::path::PathBuf::from(url.strip_prefix("file://")?);
                (path.is_file().then_some(path)?, false)
            }
            ImageSource::FileIcon(file) => {
                let find = |name: &str| self.icon_lookup.as_ref()?.find(name);
                match crate::icons::file_glyph(std::path::Path::new(file), &find) {
                    crate::icons::Glyph::Art(art) => {
                        return Some(RowIcon::Art {
                            art,
                            monochrome: false,
                            tint,
                        });
                    }
                    crate::icons::Glyph::Builtin { name, .. } => (self.builtin_path(&name)?, true),
                    crate::icons::Glyph::Text(glyph) => return Some(RowIcon::Text(glyph)),
                }
            }
            ImageSource::Themed { .. } => return None,
        };
        Some(RowIcon::Art {
            art: crate::icons::classify(&path)?,
            monochrome,
            tint,
        })
    }

    /// The file builtin `name` is drawn from: the icon theme's symbolic
    /// stand-in ([`compass_core::builtin_icon::symbolic_name`]) when it has
    /// an SVG one, else the installed builtin.
    fn builtin_path(&self, name: &str) -> Option<std::path::PathBuf> {
        compass_core::builtin_icon::symbolic_name(name)
            .and_then(|symbolic| self.icon_lookup.as_ref()?.find(symbolic))
            .filter(|path| path.extension().is_some_and(|ext| ext == "svg"))
            .or_else(|| compass_core::builtin_icon::path(name))
    }

    /// A bare icon string read as `ImageURL(source)` reads it: an emoji or
    /// symbol as text, a builtin, a file or asset, a theme icon.
    fn source_icon(&self, source: &str, tint: Option<iced::Color>) -> Option<RowIcon> {
        use compass_core::image_url::{ImageUrl, ImageUrlType};
        let lookup = PageLookup {
            assets: self.assets.as_deref(),
            icon_lookup: self.icon_lookup.as_ref(),
        };
        let url = ImageUrl::from_source(source, &lookup);
        let (path, monochrome) = match url.kind {
            ImageUrlType::Emoji | ImageUrlType::Symbol => return Some(RowIcon::Text(url.name)),
            ImageUrlType::Builtin => (self.builtin_path(&url.name)?, true),
            ImageUrlType::Local => (std::path::PathBuf::from(&url.name), false),
            ImageUrlType::System => (self.icon_lookup.as_ref()?.find(&url.name)?, false),
            _ => return None,
        };
        Some(RowIcon::Art {
            art: crate::icons::classify(&path)?,
            monochrome,
            tint,
        })
    }

    /// The list, when the view is one.
    #[must_use]
    pub fn list(&self) -> Option<&compass_extension_api::view::ListView> {
        match &self.view {
            Some(View::List(list)) => Some(list),
            _ => None,
        }
    }

    fn item(&self, section: usize, item: usize) -> Option<&ListItem> {
        self.list()?.sections.get(section)?.items.get(item)
    }

    /// Whether the extension filters its own list.
    #[must_use]
    pub fn extension_filters(&self) -> bool {
        self.list().is_some_and(|list| !list.search.host_filtering)
    }

    /// Recomputes `shown` for the current query.
    pub fn refilter(&mut self) {
        let Some(list) = self.list() else {
            self.shown.clear();
            self.selected = 0;
            return;
        };
        let all = list
            .sections
            .iter()
            .enumerate()
            .flat_map(|(s, section)| (0..section.items.len()).map(move |i| (s, i)));
        let query = self.query.trim();
        self.shown = if query.is_empty() || !list.search.host_filtering {
            all.collect()
        } else {
            let query = compass_search::Query::new(query);
            let mut scored: Vec<((usize, usize), u32)> = all
                .filter_map(|(s, i)| {
                    let item = &list.sections[s].items[i];
                    let mut fields = vec![compass_search::WeightedField::new(&item.title, 1.0)];
                    if let Some(subtitle) = &item.subtitle {
                        fields.push(compass_search::WeightedField::new(subtitle, 0.5));
                    }
                    fields.extend(
                        item.keywords
                            .iter()
                            .map(|k| compass_search::WeightedField::new(k, 0.6)),
                    );
                    let found = compass_search::score_weighted(&fields, &query);
                    (found.quality >= compass_search::MIN_QUALITY && found.score > 0)
                        .then_some(((s, i), found.score))
                })
                .collect();
            // Stable on ties, so equal matches keep the extension's order.
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            scored.into_iter().map(|(row, _)| row).collect()
        };
        self.selected = self.selected.min(self.shown.len().saturating_sub(1));
    }

    /// The selected row.
    #[must_use]
    pub fn selected_item(&self) -> Option<&ListItem> {
        let &(s, i) = self.shown.get(self.selected)?;
        self.item(s, i)
    }

    /// The actions Enter and the action panel offer now: the selected row's,
    /// else the list's own, else a detail's.
    #[must_use]
    pub fn actions(&self) -> Option<&ActionPanel> {
        match &self.view {
            Some(View::List(list)) => self
                .selected_item()
                .and_then(|item| item.actions.as_ref())
                .or(list.actions.as_ref()),
            Some(View::Detail(detail)) => detail.actions.as_ref(),
            Some(View::Form(form)) => form.actions.as_ref(),
            _ => None,
        }
    }

    /// The form, when the view is one.
    #[must_use]
    pub fn form(&self) -> Option<&compass_extension_api::view::FormView> {
        match &self.view {
            Some(View::Form(form)) => Some(form),
            _ => None,
        }
    }

    /// What an action is called with: a form's values, so `SubmitForm`'s
    /// `onSubmit` gets them (a plain `onAction` ignores its arguments);
    /// nothing elsewhere.
    #[must_use]
    pub fn action_args(&self) -> Vec<serde_json::Value> {
        if self.form().is_some() {
            vec![serde_json::Value::Object(self.form_values.clone())]
        } else {
            Vec::new()
        }
    }

    /// The person set `name` to `value`: kept, counted, and the field's
    /// `onChange` with its arguments, if it has one.
    pub fn edit_field(
        &mut self,
        name: &str,
        value: serde_json::Value,
    ) -> Option<(HandlerId, Vec<serde_json::Value>)> {
        self.form_values.insert(name.to_owned(), value.clone());
        let count = self.form_edits.entry(name.to_owned()).or_default();
        *count += 1;
        let count = *count;
        let handler = self.form()?.items.iter().find_map(|item| match item {
            compass_extension_api::view::FormItem::Field(field) if field.name == name => {
                field.on_change.clone()
            }
            _ => None,
        })?;
        Some((handler, vec![value, serde_json::Value::from(count)]))
    }

    /// A render's field values, where they do not undo the person's typing:
    /// a starting value only fills an empty field, and an echo only lands
    /// when it answers the latest edit (or is newer, the extension having
    /// set the field itself).
    fn take_form_values(&mut self, form: &compass_extension_api::view::FormView) {
        use compass_extension_api::view::FormItem;
        for item in &form.items {
            let FormItem::Field(field) = item else {
                continue;
            };
            let Some(value) = field.value.as_ref().map(field_json) else {
                continue;
            };
            let edits = self.form_edits.get(&field.name).copied().unwrap_or(0);
            let take = match field.echo {
                Some(echo) => echo.raw() >= edits,
                None => !self.form_values.contains_key(&field.name),
            };
            if take {
                if matches!(
                    field.kind,
                    compass_extension_api::view::FieldKind::TextArea { .. }
                ) {
                    self.editors.insert(
                        field.name.clone(),
                        iced::widget::text_editor::Content::with_text(
                            value.as_str().unwrap_or_default(),
                        ),
                    );
                }
                self.form_values.insert(field.name.clone(), value);
            }
        }
        for item in &form.items {
            if let FormItem::Field(field) = item
                && matches!(
                    field.kind,
                    compass_extension_api::view::FieldKind::TextArea { .. }
                )
            {
                self.editors.entry(field.name.clone()).or_default();
            }
        }
    }

    /// Whether the form has a multi-line field, where Enter is a newline and
    /// submitting takes Ctrl+Enter.
    #[must_use]
    pub fn has_text_area(&self) -> bool {
        self.form().is_some_and(|form| {
            form.items.iter().any(|item| {
                matches!(item, compass_extension_api::view::FormItem::Field(field)
                    if matches!(field.kind, compass_extension_api::view::FieldKind::TextArea { .. }))
            })
        })
    }

    /// Text typed into the date field `name`: kept as typed, and sent as
    /// the field's value once it is a date (or as no date once it is empty).
    pub fn edit_date(
        &mut self,
        name: &str,
        typed: String,
    ) -> Option<(HandlerId, Vec<serde_json::Value>)> {
        use compass_extension_api::view::{FieldKind, FormItem};
        let precision = self.form()?.items.iter().find_map(|item| match item {
            FormItem::Field(field) if field.name == name => match field.kind {
                FieldKind::DatePicker { precision, .. } => Some(precision),
                _ => None,
            },
            _ => None,
        })?;
        let value = if typed.trim().is_empty() {
            Some(serde_json::Value::Null)
        } else {
            crate::extension_fields::parse_date(&typed, precision).map(serde_json::Value::from)
        };
        self.date_drafts.insert(name.to_owned(), typed);
        self.edit_field(name, value?)
    }

    /// What the date field `name` shows: the text being typed, else its
    /// value in the typed format.
    #[must_use]
    pub fn date_shown(
        &self,
        name: &str,
        precision: compass_extension_api::view::DatePrecision,
    ) -> String {
        self.date_drafts.get(name).cloned().unwrap_or_else(|| {
            self.form_values
                .get(name)
                .and_then(serde_json::Value::as_str)
                .map(|value| crate::extension_fields::show_date(value, precision))
                .unwrap_or_default()
        })
    }

    /// An edit in the text area `name`: applied to its editor, and when it
    /// changed the text, recorded like any other field's edit.
    pub fn edit_text_area(
        &mut self,
        name: &str,
        action: iced::widget::text_editor::Action,
    ) -> Option<(HandlerId, Vec<serde_json::Value>)> {
        let editor = self.editors.entry(name.to_owned()).or_default();
        let changed = action.is_edit();
        editor.perform(action);
        if !changed {
            return None;
        }
        let text = editor.text();
        self.edit_field(name, serde_json::Value::String(text))
    }

    /// The handler a chord runs: the action on offer whose shortcut is
    /// exactly `modifiers` plus `key` (key names as `jsx.d.ts` gives them,
    /// compared without case).
    #[must_use]
    pub fn action_for(
        &self,
        modifiers: &[compass_extension_api::action::KeyModifier],
        key: &str,
    ) -> Option<&HandlerId> {
        let mut wanted = modifiers.to_vec();
        wanted.sort();
        wanted.dedup();
        self.actions()?
            .actions()
            .into_iter()
            .find(|action| {
                action.shortcut.as_ref().is_some_and(|shortcut| {
                    let mut has = shortcut.modifiers.clone();
                    has.sort();
                    has.dedup();
                    has == wanted && shortcut.key.as_str().eq_ignore_ascii_case(key)
                })
            })
            .map(|action| &action.handler)
    }

    /// The handler Enter runs: the first action on offer.
    #[must_use]
    pub fn primary_action(&self) -> Option<&HandlerId> {
        self.actions()?
            .actions()
            .into_iter()
            .next()
            .map(|action| &action.handler)
    }
}

/// A list row's second line: its subtitle, then its accessories' text, as
/// the row draws it and the accessibility tree describes it.
#[must_use]
pub fn row_subtitle(item: &ListItem) -> Option<String> {
    let accessories: Vec<&str> = item
        .accessories
        .iter()
        .filter_map(|a| a.text.as_deref().or(a.tag.as_deref()))
        .collect();
    match (&item.subtitle, accessories.is_empty()) {
        (Some(subtitle), true) => Some(subtitle.clone()),
        (Some(subtitle), false) => Some(format!("{subtitle}  ·  {}", accessories.join("  "))),
        (None, false) => Some(accessories.join("  ")),
        (None, true) => None,
    }
}

/// A `Color` as drawn: `#rrggbb[aa]`, or one of Raycast's named colours.
#[must_use]
pub fn color_of(color: &compass_extension_api::view::Color) -> Option<iced::Color> {
    use compass_extension_api::view::Color;
    let rgb = |r, g, b| Some(iced::Color::from_rgb8(r, g, b));
    match color {
        Color::Literal(hex) => {
            let hex = hex.strip_prefix('#')?;
            let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
            match hex.len() {
                6 => rgb(byte(0)?, byte(2)?, byte(4)?),
                8 => Some(iced::Color::from_rgba8(
                    byte(0)?,
                    byte(2)?,
                    byte(4)?,
                    f32::from(byte(6)?) / 255.0,
                )),
                _ => None,
            }
        }
        Color::Named(name) => match name.as_str() {
            "red" => rgb(0xf4, 0x43, 0x36),
            "orange" => rgb(0xff, 0x98, 0x00),
            "yellow" => rgb(0xff, 0xc1, 0x07),
            "green" => rgb(0x4c, 0xaf, 0x50),
            "blue" => rgb(0x21, 0x96, 0xf3),
            "purple" => rgb(0x9c, 0x27, 0xb0),
            "magenta" => rgb(0xe9, 0x1e, 0x63),
            _ => None,
        },
    }
}

/// A field value as the extension reads it in `Form.Values`.
fn field_json(value: &compass_extension_api::view::FieldValue) -> serde_json::Value {
    use compass_extension_api::view::FieldValue;
    match value {
        FieldValue::Text(text) | FieldValue::Date(text) => serde_json::Value::String(text.clone()),
        FieldValue::Bool(on) => serde_json::Value::Bool(*on),
        FieldValue::Integer(n) => serde_json::Value::from(*n),
        FieldValue::Paths(all) | FieldValue::Values(all) => {
            serde_json::Value::Array(all.iter().cloned().map(serde_json::Value::String).collect())
        }
        FieldValue::Empty => serde_json::Value::Null,
    }
}

/// `grid`'s cells as list rows, sections and actions kept.
fn grid_as_list(
    grid: compass_extension_api::view::GridView,
) -> compass_extension_api::view::ListView {
    use compass_extension_api::view::{ListSection, ListView};
    ListView {
        navigation_title: grid.navigation_title,
        is_loading: grid.is_loading,
        on_selection_change: grid.on_selection_change,
        search: grid.search,
        actions: grid.actions,
        empty_state: grid.empty_state,
        sections: grid
            .sections
            .into_iter()
            .map(|section| ListSection {
                title: section.title,
                subtitle: section.subtitle,
                items: section
                    .items
                    .into_iter()
                    .map(|cell| {
                        let mut item = ListItem::new(cell.title);
                        item.key = cell.key;
                        item.subtitle = cell.subtitle.or(cell.tooltip);
                        item.keywords = cell.keywords;
                        item.actions = cell.actions;
                        item
                    })
                    .collect(),
                ..ListSection::default()
            })
            .collect(),
        ..ListView::default()
    }
}

/// A grid's columns when neither it nor its section says (`SectionGridModel`).
pub const DEFAULT_GRID_COLUMNS: usize = 8;

/// The `(section, item)` of every row whose image `masks` gives a mask.
fn masks_of<S, I>(
    masks: S,
) -> std::collections::BTreeMap<(usize, usize), compass_core::image_url::ImageMask>
where
    S: Iterator<Item = I>,
    I: Iterator<Item = Option<compass_extension_api::view::ImageMask>>,
{
    use compass_core::image_url::ImageMask;
    use compass_extension_api::view::ImageMask as Declared;
    masks
        .enumerate()
        .flat_map(|(s, items)| {
            items.enumerate().filter_map(move |(i, mask)| {
                let mask = match mask? {
                    Declared::Circle => ImageMask::Circle,
                    Declared::RoundedRectangle => ImageMask::RoundedRectangle,
                };
                Some(((s, i), mask))
            })
        })
        .collect()
}

/// The `(section, item)` of every row `urls` gives a URL for.
fn rows_with<S, I>(urls: S) -> std::collections::BTreeMap<(usize, usize), String>
where
    S: Iterator<Item = I>,
    I: Iterator<Item = Option<String>>,
{
    urls.enumerate()
        .flat_map(|(s, items)| {
            items
                .enumerate()
                .filter_map(move |(i, url)| Some(((s, i), url?)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ExtensionViewState;
    use compass_extension_api::action::{Action, ActionPanel};
    use compass_extension_api::view::{ListSection, ListView};

    fn item(title: &str, key: &str, handler: &str) -> ListItem {
        ListItem::new(title)
            .with_key(key)
            .with_actions(ActionPanel::of([Action::new("Go", handler)]))
    }

    fn list(items: Vec<ListItem>, host_filtering: bool) -> View {
        let mut list = ListView {
            sections: vec![ListSection::untitled(items)],
            ..ListView::default()
        };
        list.search.host_filtering = host_filtering;
        View::List(list)
    }

    fn state(version: u64, view: View) -> ExtensionViewState {
        ExtensionViewState {
            version,
            view: Some(Box::new(view)),
            problem: None,
            ended: false,
            depth: 1,
            alert: None,
            toast: None,
        }
    }

    #[test]
    fn the_host_filters_fuzzily_and_enter_runs_the_selected_rows_first_action() {
        let mut page = ExtensionPage::new(1, "Repos");
        page.apply(state(
            1,
            list(
                vec![item("compass", "c", "cb-1"), item("vicinae", "v", "cb-2")],
                true,
            ),
        ));
        assert_eq!(page.status, Status::Ready);
        assert_eq!(page.shown.len(), 2);
        assert_eq!(page.primary_action().map(|h| h.0.as_str()), Some("cb-1"));

        page.query = "vicin".into();
        page.refilter();
        assert_eq!(page.shown.len(), 1);
        assert_eq!(page.primary_action().map(|h| h.0.as_str()), Some("cb-2"));
    }

    #[test]
    fn a_list_that_filters_itself_is_shown_whole() {
        let mut page = ExtensionPage::new(1, "Search");
        page.apply(state(
            1,
            list(vec![item("a", "a", "x"), item("b", "b", "y")], false),
        ));
        page.query = "zzz".into();
        page.refilter();
        assert!(page.extension_filters());
        assert_eq!(page.shown.len(), 2, "the extension decides what matches");
    }

    #[test]
    fn a_rerender_keeps_the_selection_on_the_same_row() {
        let mut page = ExtensionPage::new(1, "Repos");
        page.apply(state(
            1,
            list(vec![item("a", "a", "x"), item("b", "b", "y")], true),
        ));
        page.selected = 1;
        page.apply(state(
            2,
            list(
                vec![
                    item("new", "n", "z"),
                    item("a", "a", "x"),
                    item("b", "b", "y"),
                ],
                true,
            ),
        ));
        assert_eq!(page.selected_item().map(|i| i.title.as_str()), Some("b"));
    }

    #[test]
    fn a_render_for_a_new_search_starts_at_its_first_row() {
        // "3 lb" lists grams first; "3 lb in kg" puts kilograms first and
        // grams second. The answer to the new search is its first row.
        let mut page = ExtensionPage::new(1, "Unit Converter");
        page.query = "3 lb".into();
        page.apply(state(
            1,
            list(
                vec![item("1360 g", "g", "x"), item("1.36 kg", "kg", "y")],
                false,
            ),
        ));
        assert_eq!(
            page.selected_item().map(|i| i.title.as_str()),
            Some("1360 g")
        );
        page.query = "3 lb in kg".into();
        page.apply(state(
            2,
            list(
                vec![item("1.36 kg", "kg", "y"), item("1360 g", "g", "x")],
                false,
            ),
        ));
        assert_eq!(
            page.selected_item().map(|i| i.title.as_str()),
            Some("1.36 kg")
        );
    }

    #[test]
    fn a_grid_is_searched_and_acted_on_like_a_list() {
        use compass_extension_api::view::{GridContent, GridItem, GridSection, GridView, Image};
        let cell = |title: &str, handler: &str| GridItem {
            id: compass_extension_api::id::NodeId::ROOT,
            key: Some(title.to_owned()),
            title: title.to_owned(),
            subtitle: None,
            content: GridContent::Image(Image::builtin("star")),
            tooltip: None,
            keywords: Vec::new(),
            actions: Some(ActionPanel::of([Action::new("Copy", handler)])),
        };
        let mut grid = GridView {
            sections: vec![GridSection {
                items: vec![cell("sun", "cb-1"), cell("moon", "cb-2")],
                ..GridSection::default()
            }],
            ..GridView::default()
        };
        grid.search.host_filtering = true;
        let mut page = ExtensionPage::new(1, "Emoji");
        page.apply(state(1, View::Grid(grid)));
        assert_eq!(page.status, Status::Ready);
        assert_eq!(page.shown.len(), 2);
        page.query = "moon".into();
        page.refilter();
        assert_eq!(page.primary_action().map(|h| h.0.as_str()), Some("cb-2"));
    }

    #[test]
    fn a_form_keeps_typing_over_stale_echoes_and_submits_its_values() {
        use compass_extension_api::input::Seq;
        use compass_extension_api::view::{FieldKind, FieldValue, FormField, FormItem, FormView};
        let form = |value: &str, echo: Option<u64>| {
            View::Form(FormView {
                items: vec![FormItem::Field(Box::new(FormField {
                    id: compass_extension_api::id::NodeId::ROOT,
                    name: "title".into(),
                    title: None,
                    error: None,
                    info: None,
                    autofocus: false,
                    value: Some(FieldValue::Text(value.into())),
                    echo: echo.map(Seq::from_raw),
                    on_change: Some(HandlerId::new("cb-change")),
                    kind: FieldKind::Text { placeholder: None },
                }))],
                actions: Some(ActionPanel::of([Action::new("Create", "cb-submit")])),
                ..FormView::default()
            })
        };
        let mut page = ExtensionPage::new(1, "New");
        page.apply(state(1, form("", None)));
        assert_eq!(page.form_values["title"], "");

        let first = page.edit_field("title", "a".into());
        assert_eq!(
            first.map(|(h, args)| (h.0, args)),
            Some(("cb-change".into(), vec!["a".into(), 1.into()]))
        );
        page.edit_field("title", "ab".into());
        page.apply(state(2, form("a", Some(1))));
        assert_eq!(page.form_values["title"], "ab", "a stale echo is ignored");
        page.apply(state(3, form("AB", Some(2))));
        assert_eq!(
            page.form_values["title"], "AB",
            "the answer to the latest edit lands, even when the extension changed it"
        );

        assert_eq!(
            page.primary_action().map(|h| h.0.as_str()),
            Some("cb-submit")
        );
        assert_eq!(page.action_args(), [serde_json::json!({"title": "AB"})]);
    }

    #[test]
    fn row_icons_resolve_assets_file_urls_themes_and_colour_cells() {
        use compass_extension_api::view::{
            Color, GridContent, GridItem, GridSection, GridView, Image, ImageSource,
        };
        let assets = tempfile::tempdir().unwrap();
        std::fs::write(assets.path().join("logo.png"), b"png").unwrap();
        std::fs::write(assets.path().join("moon.svg"), b"<svg/>").unwrap();
        let image = |source: ImageSource| {
            let mut image = Image::builtin(String::new());
            image.source = source;
            image
        };
        let mut logo = ListItem::new("logo");
        logo.icon = Some(image(ImageSource::Asset("logo.png".into())));
        let mut themed = ListItem::new("themed");
        themed.icon = Some(image(ImageSource::Themed {
            light: Box::new(ImageSource::Asset("missing.svg".into())),
            dark: Box::new(ImageSource::Url(format!(
                "file://{}",
                assets.path().join("moon.svg").display()
            ))),
        }));
        let mut remote = ListItem::new("remote");
        remote.icon = Some(image(ImageSource::Url("https://example.com/a.png".into())));

        let mut page = ExtensionPage::new(1, "Icons");
        page.assets = Some(assets.path().to_owned());
        page.prefers_dark = true;
        page.apply(state(1, list(vec![logo, themed, remote], true)));
        assert!(matches!(
            page.icon(0, 0),
            Some(RowIcon::Art {
                art: crate::icons::IconArt::Raster(_),
                monochrome: false,
                ..
            })
        ));
        assert!(
            matches!(
                page.icon(0, 1),
                Some(RowIcon::Art {
                    art: crate::icons::IconArt::Vector(_),
                    ..
                })
            ),
            "a dark launcher takes the themed image's dark side"
        );
        assert_eq!(
            page.icon(0, 2),
            None,
            "a remote image is not there until fetched"
        );
        assert_eq!(page.wanted_images(), ["https://example.com/a.png"]);
        assert!(page.wanted_images().is_empty(), "asked for once");
        page.image_arrived(
            "https://example.com/a.png".into(),
            Ok(assets.path().join("logo.png")),
        );
        assert!(
            matches!(
                page.icon(0, 2),
                Some(RowIcon::Art {
                    art: crate::icons::IconArt::Raster(_),
                    ..
                })
            ),
            "once fetched, the row draws it"
        );

        let cell = GridItem {
            id: compass_extension_api::id::NodeId::ROOT,
            key: None,
            title: "red".into(),
            subtitle: None,
            content: GridContent::Color(Color::Literal("#ff0000".into())),
            tooltip: None,
            keywords: Vec::new(),
            actions: None,
        };
        page.apply(state(
            2,
            View::Grid(GridView {
                sections: vec![GridSection {
                    items: vec![cell],
                    ..GridSection::default()
                }],
                ..GridView::default()
            }),
        ));
        assert_eq!(
            page.icon(0, 0),
            Some(&RowIcon::Swatch(iced::Color::from_rgb8(0xff, 0, 0)))
        );
    }

    #[test]
    fn a_bare_icon_string_is_an_emoji_a_theme_icon_or_an_asset_and_masks_are_kept() {
        use compass_extension_api::view::{Image, ImageMask, ImageSource};
        let assets = tempfile::tempdir().unwrap();
        std::fs::write(assets.path().join("avatar"), b"png").unwrap();
        let theme = tempfile::tempdir().unwrap();
        let firefox = theme.path().join("firefox.png");
        std::fs::write(&firefox, b"png").unwrap();
        let image = |source: ImageSource, mask: Option<ImageMask>| {
            let mut image = Image::builtin(String::new());
            image.source = source;
            image.mask = mask;
            image
        };
        let mut emoji = ListItem::new("emoji");
        emoji.icon = Some(image(ImageSource::Builtin("🔥".into()), None));
        let mut themed = ListItem::new("themed");
        themed.icon = Some(image(
            ImageSource::Builtin("firefox".into()),
            Some(ImageMask::Circle),
        ));
        let mut remote = ListItem::new("remote");
        remote.icon = Some(image(
            ImageSource::Url("https://example.com/me.png".into()),
            Some(ImageMask::RoundedRectangle),
        ));
        let mut icon_url = ListItem::new("icon url");
        icon_url.icon = Some(image(ImageSource::Url("icon://emoji/🎉".into()), None));

        let mut page = ExtensionPage::new(1, "Icons");
        page.assets = Some(assets.path().to_owned());
        let found = firefox.clone();
        page.icon_lookup = Some(crate::app::IconLookup::new(move |name: &str| {
            (name == "firefox").then(|| found.clone())
        }));
        page.apply(state(1, list(vec![emoji, themed, remote, icon_url], true)));
        assert_eq!(page.icon(0, 0), Some(&RowIcon::Text("🔥".into())));
        assert!(
            matches!(page.icon(0, 1), Some(RowIcon::Art { art, .. }) if art.path() == firefox),
            "a name that is no builtin is the theme's icon"
        );
        assert_eq!(page.icon(0, 3), Some(&RowIcon::Text("🎉".into())));
        use compass_core::image_url::ImageMask as Mask;
        assert_eq!(page.mask(0, 0), Mask::None);
        assert_eq!(page.mask(0, 1), Mask::Circle);
        assert_eq!(
            page.mask(0, 2),
            Mask::RoundedRectangle,
            "a remote image keeps its row's mask"
        );
    }

    #[test]
    fn a_grid_moves_by_cell_and_by_its_sections_columns() {
        use crate::fonts_page::GridMove;
        use compass_extension_api::view::{GridContent, GridItem, GridSection, GridView};
        let cell = |title: &str| GridItem {
            id: compass_extension_api::id::NodeId::ROOT,
            key: None,
            title: title.into(),
            subtitle: None,
            content: GridContent::Color(compass_extension_api::view::Color::Literal(
                "#ff0000".into(),
            )),
            tooltip: None,
            keywords: Vec::new(),
            actions: None,
        };
        let section = |columns: Option<u16>, titles: &[&str]| GridSection {
            columns,
            items: titles.iter().map(|t| cell(t)).collect(),
            ..GridSection::default()
        };
        let mut page = ExtensionPage::new(1, "Grid");
        page.apply(state(
            1,
            View::Grid(GridView {
                columns: Some(3),
                sections: vec![
                    section(None, &["a", "b", "c", "d", "e"]),
                    section(Some(2), &["f", "g", "h"]),
                ],
                ..GridView::default()
            }),
        ));
        assert_eq!(page.grid_columns, Some(vec![3, 2]));
        assert_eq!(
            page.grid_groups(),
            [(0, vec![0, 1, 2, 3, 4]), (1, vec![5, 6, 7])]
        );
        let at = |page: &mut ExtensionPage, from: usize, step: GridMove, wrap: bool| {
            page.selected = from;
            page.grid_step(step, wrap)
        };
        assert_eq!(at(&mut page, 1, GridMove::Down, false), 4, "b down to e");
        assert_eq!(
            at(&mut page, 2, GridMove::Down, false),
            4,
            "c down: the row is short"
        );
        assert_eq!(
            at(&mut page, 4, GridMove::Down, false),
            6,
            "e down: g, same column"
        );
        assert_eq!(
            at(&mut page, 5, GridMove::Up, false),
            3,
            "f up: d, the last row"
        );
        assert_eq!(
            at(&mut page, 4, GridMove::Right, false),
            5,
            "reading order runs on"
        );
        assert_eq!(at(&mut page, 7, GridMove::Down, false), 7, "no wrap");
        assert_eq!(at(&mut page, 7, GridMove::Down, true), 0, "wrap to the top");
        assert_eq!(
            at(&mut page, 1, GridMove::Up, true),
            7,
            "wrap to the last row, clamped"
        );
        assert_eq!(at(&mut page, 0, GridMove::Left, true), 7);

        page.apply(state(2, list(vec![ListItem::new("x")], true)));
        assert_eq!(page.grid_columns, None, "a list is not a grid");
    }

    #[test]
    fn a_code_block_is_highlighted_by_its_language() {
        use iced::widget::markdown::Item;
        let items: Vec<Item> =
            iced::widget::markdown::parse("```rust\nfn main() { let answer = 42; }\n```").collect();
        let Some(Item::CodeBlock {
            language, lines, ..
        }) = items.first()
        else {
            panic!("no code block: {items:?}");
        };
        assert_eq!(language.as_deref(), Some("rust"));
        let style = iced::widget::markdown::Style::from_palette(iced::Theme::Dark.palette());
        let colors: std::collections::BTreeSet<String> = lines[0]
            .spans(style)
            .iter()
            .map(|span| format!("{:?}", span.color))
            .collect();
        assert!(
            colors.len() > 2,
            "keywords, numbers and names in their own colours: {colors:?}"
        );
    }

    #[test]
    fn a_details_markdown_images_are_fetched_and_drawn() {
        let assets = tempfile::tempdir().unwrap();
        let png = assets.path().join("shot.png");
        std::fs::write(&png, b"png").unwrap();
        let mut page = ExtensionPage::new(1, "Readme");
        page.apply(state(
            1,
            View::Detail(compass_extension_api::view::Detail {
                markdown: Some(
                    "# Shots\n\n![one](https://example.com/one.png)\n\n![local](file:///nowhere.png)"
                        .into(),
                ),
                ..compass_extension_api::view::Detail::default()
            }),
        ));
        assert_eq!(
            page.wanted_images(),
            ["https://example.com/one.png"],
            "only remote images are fetched"
        );
        assert!(page.markdown_art.is_empty());
        page.image_arrived("https://example.com/one.png".into(), Ok(png));
        assert!(matches!(
            page.markdown_art.get("https://example.com/one.png"),
            Some(RowIcon::Art {
                art: crate::icons::IconArt::Raster(_),
                ..
            })
        ));
    }

    #[test]
    fn a_text_area_takes_lines_and_each_edit_is_sent_with_its_count() {
        use compass_extension_api::view::{FieldKind, FieldValue, FormField, FormItem, FormView};
        use iced::widget::text_editor::{Action as EditorAction, Edit};
        let mut page = ExtensionPage::new(1, "Note");
        page.apply(state(
            1,
            View::Form(FormView {
                items: vec![FormItem::Field(Box::new(FormField {
                    id: compass_extension_api::id::NodeId::ROOT,
                    name: "body".into(),
                    title: None,
                    error: None,
                    info: None,
                    autofocus: false,
                    value: Some(FieldValue::Text("hi".into())),
                    echo: None,
                    on_change: Some(HandlerId::new("cb-body")),
                    kind: FieldKind::TextArea {
                        placeholder: None,
                        markdown: false,
                    },
                }))],
                ..FormView::default()
            }),
        ));
        assert!(page.has_text_area());
        assert_eq!(page.editors["body"].text(), "hi");

        page.edit_text_area(
            "body",
            EditorAction::Move(iced::widget::text_editor::Motion::DocumentEnd),
        );
        page.edit_text_area("body", EditorAction::Edit(Edit::Enter));
        let sent = page.edit_text_area("body", EditorAction::Edit(Edit::Insert('x')));
        assert_eq!(page.form_values["body"], "hi\nx");
        assert_eq!(
            sent.map(|(h, args)| (h.0, args)),
            Some(("cb-body".into(), vec!["hi\nx".into(), 2.into()])),
            "each edit is counted; moving the cursor is not an edit"
        );
    }

    #[test]
    fn a_problem_or_an_end_before_any_view_is_said() {
        let mut page = ExtensionPage::new(1, "Grid thing");
        page.apply(ExtensionViewState {
            version: 1,
            view: None,
            problem: Some("Compass cannot draw the extension component <grid> yet".into()),
            ended: false,
            depth: 1,
            alert: None,
            toast: None,
        });
        assert!(matches!(&page.status, Status::Stopped(why) if why.contains("<grid>")));

        let mut quiet = ExtensionPage::new(2, "Quiet");
        quiet.apply(ExtensionViewState {
            version: 1,
            view: None,
            problem: None,
            ended: true,
            depth: 1,
            alert: None,
            toast: None,
        });
        assert_eq!(quiet.status, Status::Stopped("Quiet finished".into()));
    }
}
