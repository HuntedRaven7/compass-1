//! Search Tray: other applications' tray icons, and their menus.
//!
//! Ports `SearchTrayViewHost` and `TrayMenuViewHost`
//! (`src/server/src/builtins/vicinae/search-tray-view-host.hpp`). The engine's
//! StatusNotifierItem host answers the lists; this keeps them, their filter
//! and the selection, and which of the two is showing.

use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

use crate::backend::{TrayItemRow, TrayMenuRow};

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The list is on its way.
    Loading,
    /// It arrived (possibly empty).
    Ready,
    /// It cannot be read, and why.
    Failed(String),
}

/// Which list is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Every tray item.
    Items,
    /// One item's menu, pushed from the items.
    Menu {
        /// The item's key.
        key: String,
        /// Its title, the view's heading.
        title: String,
    },
}

/// The view's state.
#[derive(Debug, Clone)]
pub struct TrayPage {
    /// Which list.
    pub mode: Mode,
    /// The filter text.
    pub query: String,
    /// The items, when `mode` is [`Mode::Items`].
    pub items: Vec<TrayItemRow>,
    /// The menu entries, when `mode` is [`Mode::Menu`].
    pub entries: Vec<TrayMenuRow>,
    /// Positions in the current list that match `query`, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// What the last action said.
    pub notice: Option<String>,
}

impl Default for TrayPage {
    fn default() -> Self {
        Self {
            mode: Mode::Items,
            query: String::new(),
            items: Vec::new(),
            entries: Vec::new(),
            shown: Vec::new(),
            selected: 0,
            status: Status::Loading,
            notice: None,
        }
    }
}

/// The items' search placeholder.
pub const PLACEHOLDER: &str = "Search tray items...";

/// The menu's search placeholder.
pub const MENU_PLACEHOLDER: &str = "Search menu...";

/// The accessory on an item asking for attention.
pub const ATTENTION: &str = "Attention";

impl TrayPage {
    /// Takes the engine's items, keeping the selection's position.
    pub fn apply_items(&mut self, result: Result<Vec<TrayItemRow>, String>) {
        if self.mode != Mode::Items {
            return;
        }
        let selected = self.selected;
        match result {
            Ok(items) => {
                self.items = items;
                self.status = Status::Ready;
            }
            Err(reason) => {
                self.items.clear();
                self.status = Status::Failed(reason);
            }
        }
        self.refilter();
        self.selected = selected.min(self.shown.len().saturating_sub(1));
    }

    /// Shows the selected item's menu, empty until it arrives; its key.
    pub fn push_menu(&mut self) -> Option<String> {
        let item = self.selected_item()?.clone();
        self.mode = Mode::Menu {
            key: item.key.clone(),
            title: item.title,
        };
        self.query.clear();
        self.entries.clear();
        self.shown.clear();
        self.selected = 0;
        self.status = Status::Loading;
        self.notice = None;
        Some(item.key)
    }

    /// Takes a menu for `key`, if that menu is still showing.
    pub fn apply_menu(&mut self, key: &str, result: Result<Vec<TrayMenuRow>, String>) {
        if !matches!(&self.mode, Mode::Menu { key: showing, .. } if showing == key) {
            return;
        }
        let selected = self.selected;
        match result {
            Ok(entries) => {
                self.entries = entries;
                self.status = Status::Ready;
            }
            Err(reason) => {
                self.entries.clear();
                self.status = Status::Failed(reason);
            }
        }
        self.refilter();
        self.selected = selected.min(self.shown.len().saturating_sub(1));
    }

    /// Back from a menu to the items; `false` when already there.
    pub fn pop_menu(&mut self) -> bool {
        if self.mode == Mode::Items {
            return false;
        }
        self.mode = Mode::Items;
        self.query.clear();
        self.notice = None;
        self.status = Status::Ready;
        self.refilter();
        true
    }

    /// Recomputes `shown` for the current query, back at the top: an item by
    /// its title and, at half weight, its tooltip; an entry by its label.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let query = Query::new(&self.query);
        let len = match self.mode {
            Mode::Items => self.items.len(),
            Mode::Menu { .. } => self.entries.len(),
        };
        if query.is_empty() {
            self.shown = (0..len).collect();
            return;
        }
        let mut scored: Vec<(u32, usize)> = (0..len)
            .filter_map(|index| {
                let found = match self.mode {
                    Mode::Items => {
                        let item = &self.items[index];
                        score_weighted(
                            &[
                                WeightedField::new(&item.title, 1.0),
                                WeightedField::new(&item.subtitle, 0.5),
                            ],
                            &query,
                        )
                    }
                    Mode::Menu { .. } => score_weighted(
                        &[WeightedField::new(&self.entries[index].label, 1.0)],
                        &query,
                    ),
                };
                (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.shown = scored.into_iter().map(|(_, index)| index).collect();
    }

    /// The selected item, in the items list.
    #[must_use]
    pub fn selected_item(&self) -> Option<&TrayItemRow> {
        if self.mode != Mode::Items {
            return None;
        }
        self.items.get(*self.shown.get(self.selected)?)
    }

    /// The selected entry, in a menu.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&TrayMenuRow> {
        if self.mode == Mode::Items {
            return None;
        }
        self.entries.get(*self.shown.get(self.selected)?)
    }

    /// The key of the menu showing.
    #[must_use]
    pub fn menu_key(&self) -> Option<&str> {
        match &self.mode {
            Mode::Menu { key, .. } => Some(key),
            Mode::Items => None,
        }
    }

    /// What the view says in place of the list: loading, why it cannot be
    /// read, that it is empty, or that nothing matches; `None` while it
    /// shows rows.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        let in_menu = matches!(self.mode, Mode::Menu { .. });
        let unfiltered = self.query.is_empty();
        match &self.status {
            Status::Loading if in_menu => Some("Loading the menu…"),
            Status::Loading => Some("Looking for tray items…"),
            Status::Failed(reason) => Some(reason),
            Status::Ready if self.shown.is_empty() && unfiltered && in_menu => {
                Some("This menu is empty")
            }
            Status::Ready if self.shown.is_empty() && unfiltered => Some("No tray items"),
            Status::Ready if self.shown.is_empty() => Some("No matches"),
            Status::Ready => None,
        }
    }
}

/// The actions an item offers, in the C++'s order: a menu-only item can only
/// be browsed; any other is activated first, then browsed if it has a menu,
/// then secondarily activated.
#[must_use]
pub fn item_actions(item: &TrayItemRow) -> Vec<ItemAction> {
    if item.item_is_menu {
        return if item.has_menu {
            vec![ItemAction::BrowseMenu]
        } else {
            Vec::new()
        };
    }
    let mut actions = vec![ItemAction::Activate];
    if item.has_menu {
        actions.push(ItemAction::BrowseMenu);
    }
    actions.push(ItemAction::SecondaryActivate);
    actions
}

/// One thing an item's panel does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemAction {
    /// `Activate`, then the launcher closes.
    Activate,
    /// The item's menu, as a list.
    BrowseMenu,
    /// `SecondaryActivate`, then the launcher closes.
    SecondaryActivate,
}

impl ItemAction {
    /// What the panel calls it.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Activate => "Activate",
            Self::BrowseMenu => "Browse menu",
            Self::SecondaryActivate => "Secondary activate",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, title: &str) -> TrayItemRow {
        TrayItemRow {
            key: key.into(),
            title: title.into(),
            has_menu: true,
            ..TrayItemRow::default()
        }
    }

    #[test]
    fn items_filter_by_title_and_tooltip_and_a_menu_is_pushed_and_popped() {
        let mut page = TrayPage::default();
        let mut network = item(":1.1", "Network");
        network.subtitle = "Wired connection".into();
        page.apply_items(Ok(vec![network, item(":1.2", "Steam")]));
        assert_eq!(page.shown, [0, 1]);
        page.query = "wired".into();
        page.refilter();
        assert_eq!(page.shown, [0], "the tooltip is searched");
        assert_eq!(page.push_menu().as_deref(), Some(":1.1"));
        assert_eq!(page.status, Status::Loading);
        assert!(page.query.is_empty());
        page.apply_menu(":1.2", Ok(vec![TrayMenuRow::default()]));
        assert!(page.entries.is_empty(), "another item's menu is dropped");
        page.apply_menu(
            ":1.1",
            Ok(vec![
                TrayMenuRow {
                    id: 3,
                    label: "Disconnect".into(),
                    ..TrayMenuRow::default()
                },
                TrayMenuRow {
                    id: 4,
                    label: "Settings".into(),
                    ..TrayMenuRow::default()
                },
            ]),
        );
        page.query = "sett".into();
        page.refilter();
        assert_eq!(page.selected_entry().map(|e| e.id), Some(4));
        assert!(page.pop_menu());
        assert_eq!(page.mode, Mode::Items);
        assert!(!page.pop_menu());
    }

    #[test]
    fn a_menu_only_item_can_only_be_browsed() {
        let mut only_menu = item(":1.1", "Menu");
        only_menu.item_is_menu = true;
        assert_eq!(item_actions(&only_menu), [ItemAction::BrowseMenu]);
        let mut plain = item(":1.2", "Plain");
        plain.has_menu = false;
        assert_eq!(
            item_actions(&plain),
            [ItemAction::Activate, ItemAction::SecondaryActivate]
        );
        assert_eq!(
            item_actions(&item(":1.3", "Both")),
            [
                ItemAction::Activate,
                ItemAction::BrowseMenu,
                ItemAction::SecondaryActivate
            ]
        );
    }
}
