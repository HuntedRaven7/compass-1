//! Two of Compass's own inspection views: Show Installed
//! Extensions (`InstalledExtensionsViewHost`) and Search Builtin Icons
//! (`BuiltinIconsViewHost`). Each is a filtered list; the launcher draws it
//! and runs its panel ([`crate::app`]).

use compass_core::manifest::{ExtensionManifest, Provenance};
use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

/// Show Installed Extensions' placeholder.
pub const EXTENSIONS_PLACEHOLDER: &str = "Search extensions...";
/// Search Builtin Icons' placeholder.
pub const ICONS_PLACEHOLDER: &str = "Search icons...";

/// The positions in `0..len` whose fields match `query`, best first; every
/// position, in order, for an empty query (`FuzzySection`'s filter).
fn filter<'a>(
    len: usize,
    query: &str,
    fields: impl Fn(usize) -> Vec<(&'a str, f32)>,
) -> Vec<usize> {
    let query = Query::new(query);
    if query.is_empty() {
        return (0..len).collect();
    }
    let mut scored: Vec<(u32, usize)> = (0..len)
        .filter_map(|index| {
            let weighted: Vec<WeightedField<'_>> = fields(index)
                .into_iter()
                .map(|(text, weight)| WeightedField::new(text, weight))
                .collect();
            let found = score_weighted(&weighted, &query);
            (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, index))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, index)| index).collect()
}

/// The badge beside an installed extension (`displayAccessories`): where it
/// came from, with its colour's name.
#[must_use]
pub const fn provenance_badge(provenance: Provenance) -> &'static str {
    match provenance {
        Provenance::Raycast => "Raycast",
        Provenance::Vicinae => "Vicinae",
        Provenance::Local => "Local",
    }
}

/// Show Installed Extensions' state.
#[derive(Debug, Clone, Default)]
pub struct ExtensionsPage {
    /// The filter text.
    pub query: String,
    /// Every installed extension, as its manifest says.
    pub all: Vec<ExtensionManifest>,
    /// Positions in `all` that match, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the last action said, or why it did not happen.
    pub notice: Option<String>,
}

impl ExtensionsPage {
    /// The view over `all`.
    #[must_use]
    pub fn new(all: Vec<ExtensionManifest>) -> Self {
        let mut page = Self {
            all,
            ..Self::default()
        };
        page.refilter();
        page
    }

    /// Recomputes `shown` for the query, over the title and description.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let all = &self.all;
        self.shown = filter(all.len(), &self.query, |index| {
            vec![
                (all[index].title.as_str(), 1.0),
                (all[index].description.as_str(), 0.5),
            ]
        });
    }

    /// The selected extension.
    #[must_use]
    pub fn selected_extension(&self) -> Option<&ExtensionManifest> {
        self.all.get(*self.shown.get(self.selected)?)
    }

    /// What the view shows in place of the list when it has nothing to list.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&'static str> {
        if self.all.is_empty() {
            Some("No extensions are installed")
        } else if self.shown.is_empty() {
            Some("No extensions match")
        } else {
            None
        }
    }

    /// Forgets an uninstalled extension.
    pub fn remove(&mut self, id: &str) {
        self.all.retain(|extension| extension.id != id);
        let selected = self.selected;
        self.refilter();
        self.selected = selected.min(self.shown.len().saturating_sub(1));
    }
}

/// Search Builtin Icons' state.
#[derive(Debug, Clone, Default)]
pub struct IconsPage {
    /// The filter text.
    pub query: String,
    /// Positions in [`compass_core::builtin_icon::names`] that match.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
}

impl IconsPage {
    /// Every builtin icon.
    #[must_use]
    pub fn new() -> Self {
        let mut page = Self::default();
        page.refilter();
        page
    }

    /// Recomputes `shown` for the query, over the icon's name.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let names = compass_core::builtin_icon::names();
        self.shown = filter(names.len(), &self.query, |index| vec![(names[index], 1.0)]);
    }

    /// The selected icon's name.
    #[must_use]
    pub fn selected_name(&self) -> Option<&'static str> {
        compass_core::builtin_icon::names()
            .get(*self.shown.get(self.selected)?)
            .copied()
    }

    /// What the view shows in place of the list when it has nothing to list.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&'static str> {
        self.shown.is_empty().then_some("No icons match")
    }
}

/// Inspect Local Storage's placeholder over the namespaces.
pub const NAMESPACES_PLACEHOLDER: &str = "Search namespaces...";
/// Inspect Local Storage's placeholder over one namespace.
pub const ITEMS_PLACEHOLDER: &str = "Search items...";
/// Manage OAuth Token Sets' placeholder.
pub const TOKENS_PLACEHOLDER: &str = "Search token sets...";

/// Inspect Local Storage (`LocalStorageViewHost`, then
/// `LocalStorageItemViewHost` over one namespace).
#[derive(Debug, Clone, Default)]
pub struct StoragePage {
    /// The filter text.
    pub query: String,
    /// Every namespace.
    pub namespaces: Vec<String>,
    /// The namespace being browsed, and its items, once opened.
    pub browsing: Option<(String, Vec<crate::backend::StorageItemRow>)>,
    /// The namespaces' filter while a namespace is browsed, to go back to.
    pub parked_query: String,
    /// Positions that match, in the list on screen.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// A value shown ("Show value") or why something could not be read.
    pub notice: Option<String>,
}

impl StoragePage {
    /// Takes the engine's namespaces.
    pub fn set_namespaces(&mut self, namespaces: Vec<String>) {
        self.namespaces = namespaces;
        self.refilter();
    }

    /// Opens a namespace's items.
    pub fn browse(&mut self, namespace: String, items: Vec<crate::backend::StorageItemRow>) {
        self.parked_query = std::mem::take(&mut self.query);
        self.browsing = Some((namespace, items));
        self.notice = None;
        self.refilter();
    }

    /// Back from a namespace to the list of them; `false` when already there.
    pub fn back(&mut self) -> bool {
        if self.browsing.take().is_none() {
            return false;
        }
        self.query = std::mem::take(&mut self.parked_query);
        self.notice = None;
        self.refilter();
        true
    }

    /// Recomputes `shown` over the names or the keys.
    pub fn refilter(&mut self) {
        self.selected = 0;
        self.shown = match &self.browsing {
            Some((_, items)) => filter(items.len(), &self.query, |index| {
                vec![(items[index].key.as_str(), 1.0)]
            }),
            None => {
                let namespaces = &self.namespaces;
                filter(namespaces.len(), &self.query, |index| {
                    vec![(namespaces[index].as_str(), 1.0)]
                })
            }
        };
    }

    /// The selected namespace, on the first level.
    #[must_use]
    pub fn selected_namespace(&self) -> Option<&str> {
        if self.browsing.is_some() {
            return None;
        }
        self.namespaces
            .get(*self.shown.get(self.selected)?)
            .map(String::as_str)
    }

    /// The selected item, inside a namespace.
    #[must_use]
    pub fn selected_item(&self) -> Option<&crate::backend::StorageItemRow> {
        let (_, items) = self.browsing.as_ref()?;
        items.get(*self.shown.get(self.selected)?)
    }

    /// The rows' titles, in order: names or keys.
    #[must_use]
    pub fn titles(&self) -> Vec<&str> {
        self.shown
            .iter()
            .filter_map(|&index| match &self.browsing {
                Some((_, items)) => items.get(index).map(|item| item.key.as_str()),
                None => self.namespaces.get(index).map(String::as_str),
            })
            .collect()
    }

    /// What the view shows in place of the list when it has nothing to list:
    /// why nothing could be read, or that there is nothing.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        if !self.shown.is_empty() {
            return None;
        }
        Some(
            self.notice
                .as_deref()
                .unwrap_or(if self.browsing.is_some() {
                    "No items"
                } else {
                    "No namespaces"
                }),
        )
    }
}

/// Manage OAuth Token Sets (`OAuthTokenStoreViewHost`).
#[derive(Debug, Clone, Default)]
pub struct TokensPage {
    /// The filter text.
    pub query: String,
    /// Every token set.
    pub sets: Vec<crate::backend::TokenSetRow>,
    /// Positions in `sets` that match.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the last action said, or why it did not happen.
    pub notice: Option<String>,
}

impl TokensPage {
    /// Takes the engine's token sets.
    pub fn set_sets(&mut self, sets: Vec<crate::backend::TokenSetRow>) {
        let selected = self.selected;
        self.sets = sets;
        self.refilter();
        self.selected = selected.min(self.shown.len().saturating_sub(1));
    }

    /// Recomputes `shown` over the extension and the provider.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let sets = &self.sets;
        self.shown = filter(sets.len(), &self.query, |index| {
            let mut fields = vec![(sets[index].extension_id.as_str(), 1.0)];
            if let Some(provider) = &sets[index].provider_id {
                fields.push((provider.as_str(), 0.8));
            }
            fields
        });
    }

    /// The selected token set.
    #[must_use]
    pub fn selected_set(&self) -> Option<&crate::backend::TokenSetRow> {
        self.sets.get(*self.shown.get(self.selected)?)
    }

    /// What the view shows in place of the list when it has nothing to list:
    /// why nothing could be read, or that there is nothing.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        if !self.shown.is_empty() {
            return None;
        }
        Some(self.notice.as_deref().unwrap_or("No token sets"))
    }
}

/// The Vicinae store's intro (`VicinaeStoreCommand`'s `INTRO`).
pub const VICINAE_STORE_INTRO: &str = "# Welcome to the Vicinae Store

The Vicinae Store is a third-party store run by the Vicinae project, not by Compass. It lists community-built extensions that the Vicinae maintainers have reviewed, and Compass runs them unchanged.

Every extension listed here has its source code in the [vicinaehq/extensions](https://github.com/vicinaehq/extensions) repository.

To build your own extension, see the [Vicinae SDK documentation](https://docs.vicinae.com/extensions/introduction), which Compass extensions use too. Extensions are submitted to the store through that repository.
";

/// The Raycast store's intro (`RaycastStoreCommand`'s `INTRO`, with its
/// compatibility sheet, which this store shows).
pub const RAYCAST_STORE_INTRO: &str = "# Welcome to the Raycast Extension Store

Compass provides direct integration with the official [Raycast Store](https://www.raycast.com/store), so you can search for Raycast extensions and install them from Compass.

Raycast extensions are written for macOS, so not all of them work on Linux. Each extension has a colored indicator showing how well it works here.

Compass can also install from the [Vicinae Store](compass://launch/core/store), a third-party store whose extensions are all made for Linux.
";

/// The intro's one action (`StoreIntroViewHost`'s primary).
pub const CONTINUE_TO_STORE: &str = "Continue to store";

/// The keys under the intro, as the card shows them.
#[must_use]
pub fn intro_hint() -> String {
    format!("Enter: {CONTINUE_TO_STORE}    Esc: back")
}

/// A store's intro, as Markdown.
#[must_use]
pub const fn intro_text(store: crate::backend::Store) -> &'static str {
    match store {
        crate::backend::Store::Vicinae => VICINAE_STORE_INTRO,
        crate::backend::Store::Raycast => RAYCAST_STORE_INTRO,
    }
}

/// A store's name, as its brand has it.
#[must_use]
pub const fn store_name(store: crate::backend::Store) -> &'static str {
    match store {
        crate::backend::Store::Vicinae => "Vicinae Store",
        crate::backend::Store::Raycast => "Raycast Store",
    }
}

/// The view memory key that records a store's intro was seen (the C++'s
/// `introCompleted` in the command's storage).
#[must_use]
pub const fn intro_key(store: crate::backend::Store) -> &'static str {
    match store {
        crate::backend::Store::Vicinae => "store.introCompleted",
        crate::backend::Store::Raycast => "raycast-store.introCompleted",
    }
}

/// Whether a store opens on its intro: when `alwaysShowIntro` is set, or the
/// intro has not been continued past.
#[must_use]
pub fn shows_intro(always: bool, completed: Option<&str>) -> bool {
    always || completed != Some("true")
}

/// A store's intro (`StoreIntroViewHost`): its Markdown and "Continue to
/// store".
#[derive(Debug, Clone)]
pub struct StoreIntroPage {
    /// Which store it introduces.
    pub store: crate::backend::Store,
    /// The Markdown, parsed once.
    pub markdown: Vec<iced::widget::markdown::Item>,
}

impl StoreIntroPage {
    /// The intro to `store`.
    #[must_use]
    pub fn new(store: crate::backend::Store) -> Self {
        Self {
            store,
            markdown: iced::widget::markdown::parse(intro_text(store)).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, title: &str, description: &str) -> ExtensionManifest {
        ExtensionManifest {
            path: std::path::PathBuf::from("/extensions").join(id),
            id: id.into(),
            name: id.into(),
            title: title.into(),
            description: description.into(),
            icon: String::new(),
            author: "someone".into(),
            owner: None,
            categories: Vec::new(),
            preferences: Vec::new(),
            commands: Vec::new(),
            needs_raycast_api: false,
            provenance: Provenance::from_id(id),
        }
    }

    #[test]
    fn extensions_filter_on_title_and_description_and_forget_an_uninstalled_one() {
        let mut page = ExtensionsPage::new(vec![
            manifest("store.raycast.github", "GitHub", "Work with issues"),
            manifest("notes", "Notes", "Write things down"),
        ]);
        assert_eq!(page.shown, [0, 1]);
        page.query = "write".into();
        page.refilter();
        assert_eq!(
            page.selected_extension().map(|e| e.id.as_str()),
            Some("notes")
        );
        page.remove("notes");
        assert!(page.selected_extension().is_none());
        assert_eq!(
            provenance_badge(page.all[0].provenance),
            "Raycast",
            "the store an id names"
        );
    }

    #[test]
    fn storage_browses_a_namespace_and_comes_back_to_its_filter() {
        use crate::backend::StorageItemRow;
        let mut page = StoragePage::default();
        page.set_namespaces(vec!["@a/notes".into(), "core".into()]);
        page.query = "notes".into();
        page.refilter();
        assert_eq!(page.selected_namespace(), Some("@a/notes"));
        page.browse(
            "@a/notes".into(),
            vec![StorageItemRow {
                key: "draft".into(),
                value: "hello".into(),
            }],
        );
        assert!(page.query.is_empty(), "the items start unfiltered");
        assert_eq!(page.titles(), ["draft"]);
        assert_eq!(
            page.selected_item().map(|i| i.value.as_str()),
            Some("hello")
        );
        assert!(page.back());
        assert_eq!(page.query, "notes");
        assert_eq!(page.titles(), ["@a/notes"]);
        assert!(!page.back(), "already at the namespaces");
    }

    #[test]
    fn token_sets_filter_on_extension_and_provider() {
        use crate::backend::TokenSetRow;
        let mut page = TokensPage::default();
        page.set_sets(vec![
            TokenSetRow {
                extension_id: "github".into(),
                provider_id: Some("GitHub".into()),
                ..TokenSetRow::default()
            },
            TokenSetRow {
                extension_id: "linear".into(),
                ..TokenSetRow::default()
            },
        ]);
        page.query = "linear".into();
        page.refilter();
        assert_eq!(
            page.selected_set().map(|s| s.extension_id.as_str()),
            Some("linear")
        );
    }

    #[test]
    fn a_store_shows_its_intro_until_continued_or_when_always_asked() {
        assert!(shows_intro(false, None));
        assert!(!shows_intro(false, Some("true")));
        assert!(shows_intro(true, Some("true")));
        assert_ne!(
            intro_key(crate::backend::Store::Vicinae),
            intro_key(crate::backend::Store::Raycast)
        );
        assert!(
            !StoreIntroPage::new(crate::backend::Store::Raycast)
                .markdown
                .is_empty()
        );
    }

    #[test]
    fn every_icon_is_listed_and_typing_finds_one() {
        let mut page = IconsPage::new();
        assert_eq!(page.shown.len(), compass_core::builtin_icon::names().len());
        page.query = "copy-clipboard".into();
        page.refilter();
        assert_eq!(page.selected_name(), Some("copy-clipboard"));
    }
}
