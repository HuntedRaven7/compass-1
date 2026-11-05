//! What the launcher window means, for assistive technology (ADR-0022 §1).
//!
//! Iced 0.14 has no accessibility tree, so the launcher describes itself:
//! [`Semantics`] is the window as a screen reader should understand it —
//! the search field and its text, the results and which one is selected,
//! the action panel, a form's fields — built from application state by
//! `LauncherApp::semantics` and turned into an [`accesskit::TreeUpdate`]
//! here. Nothing in this module draws or needs a display, so every
//! assertion about what Orca would be told is an ordinary unit test.
//!
//! The tree is described whole on every change; AccessKit compares it with
//! the last one and tells the screen reader only what differs, so a list
//! that did not change is not announced again.
//!
//! # Where the keyboard focus is said to be
//!
//! The launcher's keyboard focus never leaves its search field: the arrow
//! keys move the selection while the field keeps the text. A screen reader
//! only follows the focus, so [`FocusTracker`] reports it where a person
//! would look: on the field while they type, on the selected result once
//! they move through the results, on the action panel's selected action
//! while it is open, on a dialog while one asks. Typing puts it back on the
//! field, which is what keeps the typed text echoed rather than the top
//! result read out at every keystroke.

use accesskit::{
    Action, Invalid, Live, Node, NodeId, Role, TextPosition, TextSelection, Toggled, TreeId,
    TreeInfo, TreeUpdate,
};
use unicode_segmentation::UnicodeSegmentation;

/// The launcher window, as assistive technology sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Semantics {
    /// The window's name.
    pub window: String,
    /// The field at the top of the card.
    pub search: Option<SearchField>,
    /// What is under it.
    pub body: Body,
    /// A short state of the view a screen reader should announce when it
    /// changes: no results, loading, an error.
    pub status: Option<String>,
    /// The action panel, while it is open.
    pub panel: Option<Menu>,
}

/// The search field, or the title shown in its place.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchField {
    /// What it says while it is empty.
    pub placeholder: String,
    /// What is in it.
    pub value: String,
    /// Whether typing changes it; a view that shows its title there is not.
    pub editable: bool,
}

/// What the card shows under the search field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Body {
    /// Nothing a screen reader is told about beyond the field and status.
    #[default]
    Empty,
    /// A list of results.
    List(List),
    /// A question waiting on Enter or Escape.
    Dialog(Dialog),
    /// A form.
    Form(Form),
}

/// A list of results.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct List {
    /// What the list is.
    pub label: String,
    /// Its rows, in order.
    pub items: Vec<Item>,
    /// The selected row, by position.
    pub selected: Option<usize>,
}

/// One result.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Item {
    /// Its title.
    pub name: String,
    /// Its subtitle.
    pub description: Option<String>,
}

/// A question asked before something runs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dialog {
    /// The question.
    pub title: String,
    /// What it means; may be empty.
    pub message: String,
    /// The keys that answer it, as the card shows them.
    pub keys: String,
}

/// The action panel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Menu {
    /// Its actions, in order; headings and dividers are not among them.
    pub items: Vec<MenuItem>,
    /// The selected action, by position in `items`.
    pub selected: Option<usize>,
}

/// One action in the panel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuItem {
    /// What it is called.
    pub name: String,
    /// The section it is under, if that has a heading.
    pub section: Option<String>,
    /// The key that runs it without the panel.
    pub shortcut: Option<String>,
    /// Its row in the panel, which a click activates.
    pub row: usize,
}

/// A form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Form {
    /// What the form is for.
    pub label: String,
    /// Its fields and notes, in order.
    pub fields: Vec<Field>,
    /// The field the keyboard is on, by position, when the form knows.
    pub focused: Option<usize>,
}

/// One entry of a form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Its label.
    pub label: String,
    /// What it takes and what it holds.
    pub kind: FieldKind,
    /// Help text, or why its value is refused.
    pub description: Option<String>,
    /// Whether the form cannot be submitted without it.
    pub required: bool,
    /// Whether its value is refused.
    pub invalid: bool,
    /// Which field it is, for an assistive technology's request to change it.
    pub target: FieldTarget,
}

/// Which field of which form an entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldTarget {
    /// A preference or argument form's field, by position.
    Preference(usize),
    /// An extension form's field, by name.
    Extension(String),
    /// A note, which takes nothing.
    None,
}

/// What a form field takes, and what it holds now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    /// A line of text, or several.
    Text {
        /// What is in it.
        value: String,
        /// What it says while it is empty.
        placeholder: Option<String>,
        /// Whether what is typed is hidden.
        secret: bool,
        /// Whether it takes several lines.
        multiline: bool,
    },
    /// A tick box.
    Checkbox {
        /// Whether it is ticked.
        checked: bool,
    },
    /// One of a list.
    Dropdown {
        /// The chosen option's title.
        value: Option<String>,
    },
    /// A date, typed in the format its placeholder gives.
    Date {
        /// What is typed.
        value: String,
        /// The format, `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`.
        format: String,
    },
    /// Several of a list.
    Tags {
        /// Every option's title, and whether it is chosen.
        options: Vec<(String, bool)>,
    },
    /// Files or directories, chosen in the desktop's file chooser.
    Files {
        /// What is chosen.
        chosen: Vec<String>,
    },
    /// Text that explains the form, or a field it cannot edit.
    Note {
        /// The text.
        text: String,
    },
}

/// What an assistive technology's request is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The window itself.
    Window,
    /// The search field.
    Search,
    /// A result, by position.
    Item(usize),
    /// An action in the panel, by position in [`Menu::items`].
    MenuItem(usize),
    /// A form field, by position in [`Form::fields`].
    Field(usize),
}

// Node ids are positions, not identities: a result's node is "the third row",
// whatever the third row is now. A new query then reads as rows renamed
// rather than rows replaced, which is what the screen shows.
const WINDOW: NodeId = NodeId(1);
const SEARCH: NodeId = NodeId(2);
const SEARCH_TEXT: NodeId = NodeId(3);
const LIST: NodeId = NodeId(4);
const STATUS: NodeId = NodeId(5);
const DIALOG: NodeId = NodeId(6);
const FORM: NodeId = NodeId(7);
const MENU: NodeId = NodeId(8);
const DIALOG_KEYS: NodeId = NodeId(9);
const ITEM_BASE: u64 = 1 << 32;
const MENU_BASE: u64 = 2 << 32;
const FIELD_BASE: u64 = 3 << 32;
/// A field's own node and its parts (its text, its options) are numbered
/// within a block this wide.
const FIELD_STRIDE: u64 = 1 << 16;

/// The toolkit, as the tree reports it to assistive technology.
const TOOLKIT: &str = "iced";
const TOOLKIT_VERSION: &str = "0.14";

fn item_id(position: usize) -> NodeId {
    NodeId(ITEM_BASE + position as u64)
}

fn menu_item_id(position: usize) -> NodeId {
    NodeId(MENU_BASE + position as u64)
}

fn field_id(position: usize) -> NodeId {
    NodeId(FIELD_BASE + position as u64 * FIELD_STRIDE)
}

fn field_part_id(position: usize, part: usize) -> NodeId {
    NodeId(field_id(position).0 + 1 + part as u64)
}

impl Semantics {
    /// What a node of this tree is, for an assistive technology's request
    /// about it. `None` for a node that takes no requests (a text run, the
    /// status line) or one that is no longer there.
    #[must_use]
    pub fn target(&self, node: NodeId) -> Option<Target> {
        match node {
            WINDOW => Some(Target::Window),
            SEARCH | SEARCH_TEXT => self
                .search
                .as_ref()
                .filter(|search| search.editable)
                .map(|_| Target::Search),
            NodeId(id) if (ITEM_BASE..MENU_BASE).contains(&id) => {
                let position = usize::try_from(id - ITEM_BASE).ok()?;
                match &self.body {
                    Body::List(list) if position < list.items.len() => Some(Target::Item(position)),
                    _ => None,
                }
            }
            NodeId(id) if (MENU_BASE..FIELD_BASE).contains(&id) => {
                let position = usize::try_from(id - MENU_BASE).ok()?;
                self.panel
                    .as_ref()
                    .filter(|menu| position < menu.items.len())
                    .map(|_| Target::MenuItem(position))
            }
            NodeId(id) if id >= FIELD_BASE => {
                let position = usize::try_from((id - FIELD_BASE) / FIELD_STRIDE).ok()?;
                match &self.body {
                    Body::Form(form) if position < form.fields.len() => {
                        Some(Target::Field(position))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The whole tree, with the keyboard focus on `focus`.
    #[must_use]
    pub fn tree_update(&self, focus: NodeId) -> TreeUpdate {
        let mut nodes = Vec::new();
        let mut children = Vec::new();

        if let Some(search) = &self.search {
            if search.editable {
                children.push(SEARCH);
                nodes.push((SEARCH, search_node(search)));
                nodes.push((SEARCH_TEXT, text_run(&search.value)));
            } else if !search.value.is_empty() {
                // A view's title shown where the field would be.
                children.push(SEARCH);
                let mut heading = Node::new(Role::Heading);
                heading.set_label(search.value.as_str());
                nodes.push((SEARCH, heading));
            }
        }

        match &self.body {
            Body::Empty => {}
            Body::List(list) => {
                children.push(LIST);
                list_nodes(list, &mut nodes);
            }
            Body::Dialog(dialog) => {
                children.push(DIALOG);
                let mut node = Node::new(Role::AlertDialog);
                node.set_label(dialog.title.as_str());
                if !dialog.message.is_empty() {
                    node.set_description(dialog.message.as_str());
                }
                node.set_modal();
                node.set_children(vec![DIALOG_KEYS]);
                nodes.push((DIALOG, node));
                let mut keys = Node::new(Role::Label);
                keys.set_value(dialog.keys.as_str());
                nodes.push((DIALOG_KEYS, keys));
            }
            Body::Form(form) => {
                children.push(FORM);
                form_nodes(form, &mut nodes);
            }
        }

        if let Some(status) = &self.status {
            children.push(STATUS);
            let mut node = Node::new(Role::Status);
            node.set_label(status.as_str());
            node.set_live(Live::Polite);
            nodes.push((STATUS, node));
        }

        if let Some(menu) = &self.panel {
            children.push(MENU);
            menu_nodes(menu, &mut nodes);
        }

        let mut window = Node::new(Role::Window);
        window.set_label(self.window.as_str());
        window.set_children(children);
        nodes.insert(0, (WINDOW, window));

        TreeUpdate {
            nodes,
            tree: Some(TreeInfo {
                root: WINDOW,
                toolkit_name: Some(TOOLKIT.to_owned()),
                toolkit_version: Some(TOOLKIT_VERSION.to_owned()),
            }),
            tree_id: TreeId::ROOT,
            focus,
        }
    }
}

fn search_node(search: &SearchField) -> Node {
    let mut node = Node::new(Role::SearchInput);
    if !search.placeholder.is_empty() {
        // Named by its placeholder ("Search clipboard history…") as well as
        // showing it: once something is typed the placeholder is gone, and
        // the field would otherwise be read as an unnamed entry.
        node.set_label(search.placeholder.as_str());
        node.set_placeholder(search.placeholder.as_str());
    }
    node.set_value(search.value.as_str());
    node.set_children(vec![SEARCH_TEXT]);
    // The caret is at the end: typing appends, and the toolkit does not say
    // where a moved caret is.
    let end = search.value.graphemes(true).count();
    node.set_text_selection(TextSelection {
        anchor: TextPosition {
            node: SEARCH_TEXT,
            character_index: end,
        },
        focus: TextPosition {
            node: SEARCH_TEXT,
            character_index: end,
        },
    });
    node.add_action(Action::Focus);
    node.add_action(Action::SetValue);
    node.add_action(Action::ReplaceSelectedText);
    node
}

/// The text of a field, as AccessKit wants it: one run, with each
/// character's length, so a screen reader can read and move through it.
fn text_run(value: &str) -> Node {
    let mut node = Node::new(Role::TextRun);
    node.set_value(value);
    let lengths: Vec<u8> = value
        .graphemes(true)
        .map(|grapheme| u8::try_from(grapheme.len()).unwrap_or(u8::MAX))
        .collect();
    node.set_character_lengths(lengths);
    node
}

fn list_nodes(list: &List, nodes: &mut Vec<(NodeId, Node)>) {
    let mut node = Node::new(Role::ListBox);
    node.set_label(list.label.as_str());
    node.set_children((0..list.items.len()).map(item_id).collect::<Vec<_>>());
    node.set_size_of_set(list.items.len());
    nodes.push((LIST, node));
    for (position, item) in list.items.iter().enumerate() {
        let mut node = Node::new(Role::ListBoxOption);
        node.set_label(item.name.as_str());
        if let Some(description) = &item.description {
            node.set_description(description.as_str());
        }
        node.set_selected(list.selected == Some(position));
        node.set_position_in_set(position);
        node.set_size_of_set(list.items.len());
        node.add_action(Action::Click);
        node.add_action(Action::Focus);
        nodes.push((item_id(position), node));
    }
}

fn menu_nodes(menu: &Menu, nodes: &mut Vec<(NodeId, Node)>) {
    let mut node = Node::new(Role::Menu);
    node.set_label("Actions");
    node.set_children((0..menu.items.len()).map(menu_item_id).collect::<Vec<_>>());
    nodes.push((MENU, node));
    for (position, item) in menu.items.iter().enumerate() {
        let mut node = Node::new(Role::MenuItem);
        node.set_label(item.name.as_str());
        if let Some(section) = &item.section {
            node.set_description(section.as_str());
        }
        if let Some(shortcut) = &item.shortcut {
            node.set_keyboard_shortcut(shortcut.as_str());
        }
        node.set_selected(menu.selected == Some(position));
        node.set_position_in_set(position);
        node.set_size_of_set(menu.items.len());
        node.add_action(Action::Click);
        node.add_action(Action::Focus);
        nodes.push((menu_item_id(position), node));
    }
}

fn form_nodes(form: &Form, nodes: &mut Vec<(NodeId, Node)>) {
    let mut node = Node::new(Role::Form);
    node.set_label(form.label.as_str());
    node.set_children((0..form.fields.len()).map(field_id).collect::<Vec<_>>());
    nodes.push((FORM, node));
    for (position, field) in form.fields.iter().enumerate() {
        let id = field_id(position);
        let mut node = match &field.kind {
            FieldKind::Text {
                value,
                placeholder,
                secret,
                multiline,
            } => {
                let role = match (secret, multiline) {
                    (true, _) => Role::PasswordInput,
                    (false, true) => Role::MultilineTextInput,
                    (false, false) => Role::TextInput,
                };
                let mut node = Node::new(role);
                if let Some(placeholder) = placeholder.as_deref().filter(|p| !p.is_empty()) {
                    node.set_placeholder(placeholder);
                }
                // A secret is read as its length in bullets, as GTK's
                // password entries are, never as itself.
                let shown = if *secret {
                    "•".repeat(value.chars().count())
                } else {
                    value.clone()
                };
                let run = field_part_id(position, 0);
                node.set_value(shown.as_str());
                node.set_children(vec![run]);
                // A text area keeps its text in an editor of its own, which
                // a request cannot reach.
                if !multiline {
                    node.add_action(Action::SetValue);
                }
                nodes.push((run, text_run(&shown)));
                node
            }
            FieldKind::Date { value, format } => {
                let mut node = Node::new(Role::TextInput);
                node.set_placeholder(format.as_str());
                let run = field_part_id(position, 0);
                node.set_value(value.as_str());
                node.set_children(vec![run]);
                node.add_action(Action::SetValue);
                nodes.push((run, text_run(value)));
                node
            }
            FieldKind::Checkbox { checked } => {
                let mut node = Node::new(Role::CheckBox);
                node.set_toggled(Toggled::from(*checked));
                node.add_action(Action::Click);
                node
            }
            FieldKind::Dropdown { value } => {
                let mut node = Node::new(Role::ComboBox);
                if let Some(value) = value {
                    node.set_value(value.as_str());
                }
                node
            }
            FieldKind::Tags { options } => {
                let mut node = Node::new(Role::ListBox);
                node.set_multiselectable();
                let parts: Vec<NodeId> = (0..options.len())
                    .map(|part| field_part_id(position, part))
                    .collect();
                node.set_children(parts.clone());
                for (part, ((title, chosen), part_id)) in options.iter().zip(parts).enumerate() {
                    let mut option = Node::new(Role::ListBoxOption);
                    option.set_label(title.as_str());
                    option.set_selected(*chosen);
                    option.set_position_in_set(part);
                    option.set_size_of_set(options.len());
                    nodes.push((part_id, option));
                }
                node
            }
            FieldKind::Files { chosen } => {
                let mut node = Node::new(Role::Button);
                if !chosen.is_empty() {
                    node.set_value(chosen.join(", "));
                }
                node
            }
            FieldKind::Note { text } => {
                let mut node = Node::new(Role::Label);
                node.set_value(text.as_str());
                node
            }
        };
        if !field.label.is_empty() {
            node.set_label(field.label.as_str());
        }
        if let Some(description) = &field.description {
            node.set_description(description.as_str());
        }
        if field.required {
            node.set_required();
        }
        if field.invalid {
            node.set_invalid(Invalid::True);
        }
        // Only a preference or argument form follows the keyboard itself;
        // an extension form's focus is Iced's own, which a request cannot move.
        if matches!(field.target, FieldTarget::Preference(_))
            && !matches!(field.kind, FieldKind::Note { .. })
        {
            node.add_action(Action::Focus);
        }
        nodes.push((id, node));
    }
}

/// Where the keyboard focus is said to be, from one description of the
/// window to the next. See the module documentation for why it moves.
#[derive(Debug, Clone, Default)]
pub struct FocusTracker {
    /// The search text and selection last seen, to tell typing from moving.
    last: Option<(String, Option<usize>)>,
    /// Whether the person has moved into the results since they last typed.
    on_results: bool,
}

impl FocusTracker {
    /// A tracker for a window that has just opened: focus on the field.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The focused node for `semantics`, which follows the previous ones
    /// this tracker saw.
    pub fn focus(&mut self, semantics: &Semantics) -> NodeId {
        let query = semantics
            .search
            .as_ref()
            .map(|search| search.value.clone())
            .unwrap_or_default();
        let selected = match &semantics.body {
            Body::List(list) => list.selected.filter(|&s| s < list.items.len()),
            _ => None,
        };
        match &self.last {
            Some((last_query, _)) if *last_query != query => self.on_results = false,
            Some((_, last_selected)) if *last_selected != selected && selected.is_some() => {
                self.on_results = true;
            }
            _ => {}
        }
        if selected.is_none() {
            self.on_results = false;
        }
        self.last = Some((query, selected));

        if let Some(menu) = &semantics.panel {
            return menu.selected.map_or(MENU, menu_item_id);
        }
        match &semantics.body {
            Body::Dialog(_) => return DIALOG,
            Body::Form(form) => {
                return form
                    .focused
                    .filter(|&f| f < form.fields.len())
                    .map_or(FORM, field_id);
            }
            Body::List(_) | Body::Empty => {}
        }
        match selected {
            Some(selected) if self.on_results => item_id(selected),
            _ if semantics.search.as_ref().is_some_and(|s| s.editable) => SEARCH,
            _ => WINDOW,
        }
    }
}

/// How many results there are, as a sentence for the status line.
#[must_use]
pub fn result_count(count: usize) -> String {
    if count == 1 {
        "1 result".to_owned()
    } else {
        format!("{count} results")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(query: &str, items: &[(&str, Option<&str>)], selected: Option<usize>) -> Semantics {
        Semantics {
            window: "Compass".to_owned(),
            search: Some(SearchField {
                placeholder: "Search…".to_owned(),
                value: query.to_owned(),
                editable: true,
            }),
            body: Body::List(List {
                label: "Results".to_owned(),
                items: items
                    .iter()
                    .map(|(name, description)| Item {
                        name: (*name).to_owned(),
                        description: description.map(str::to_owned),
                    })
                    .collect(),
                selected,
            }),
            status: None,
            panel: None,
        }
    }

    fn node(update: &TreeUpdate, id: NodeId) -> &Node {
        &update
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .unwrap_or_else(|| panic!("no node {id:?}"))
            .1
    }

    #[test]
    fn the_window_is_the_root_and_names_the_launcher() {
        let update = root("", &[], None).tree_update(SEARCH);
        assert_eq!(update.tree.as_ref().map(|tree| tree.root), Some(WINDOW));
        let window = node(&update, WINDOW);
        assert_eq!(window.role(), Role::Window);
        assert_eq!(window.label(), Some("Compass"));
        assert_eq!(update.nodes[0].0, WINDOW);
    }

    #[test]
    fn the_query_is_an_editable_search_field_with_its_text() {
        let update = root("fire", &[], None).tree_update(SEARCH);
        let field = node(&update, SEARCH);
        assert_eq!(field.role(), Role::SearchInput);
        assert_eq!(field.value(), Some("fire"));
        assert_eq!(field.placeholder(), Some("Search…"));
        assert!(field.supports_action(Action::SetValue));
        let run = node(&update, SEARCH_TEXT);
        assert_eq!(run.role(), Role::TextRun);
        assert_eq!(run.value(), Some("fire"));
        assert_eq!(run.character_lengths(), &[1, 1, 1, 1]);
        assert_eq!(
            field.text_selection().map(|s| s.focus.character_index),
            Some(4)
        );
    }

    #[test]
    fn a_character_is_a_grapheme_not_a_byte() {
        let update = root("é👍🏽", &[], None).tree_update(SEARCH);
        assert_eq!(
            node(&update, SEARCH_TEXT).character_lengths(),
            &["é".len() as u8, "👍🏽".len() as u8]
        );
    }

    #[test]
    fn results_are_a_list_of_named_described_options_with_their_count() {
        let update = root(
            "f",
            &[("Firefox", Some("Web Browser")), ("Files", None)],
            Some(1),
        )
        .tree_update(SEARCH);
        let list = node(&update, LIST);
        assert_eq!(list.role(), Role::ListBox);
        assert_eq!(list.children(), &[item_id(0), item_id(1)]);
        assert_eq!(list.size_of_set(), Some(2));
        let first = node(&update, item_id(0));
        assert_eq!(first.role(), Role::ListBoxOption);
        assert_eq!(first.label(), Some("Firefox"));
        assert_eq!(first.description(), Some("Web Browser"));
        assert_eq!(first.is_selected(), Some(false));
        assert_eq!(first.position_in_set(), Some(0));
        assert!(first.supports_action(Action::Click));
        let second = node(&update, item_id(1));
        assert_eq!(second.is_selected(), Some(true));
        assert_eq!(second.description(), None);
    }

    #[test]
    fn a_status_is_a_polite_live_region_and_absent_without_one() {
        let mut semantics = root("zzz", &[], None);
        assert!(
            !semantics
                .tree_update(SEARCH)
                .nodes
                .iter()
                .any(|(id, _)| *id == STATUS)
        );
        semantics.status = Some("No results".to_owned());
        let update = semantics.tree_update(SEARCH);
        let status = node(&update, STATUS);
        assert_eq!(status.role(), Role::Status);
        assert_eq!(status.label(), Some("No results"));
        assert_eq!(status.live(), Some(Live::Polite));
    }

    #[test]
    fn every_child_named_by_a_node_is_in_the_tree() {
        let mut semantics = root("a", &[("A", None)], Some(0));
        semantics.panel = Some(Menu {
            items: vec![MenuItem {
                name: "Open".to_owned(),
                section: None,
                shortcut: Some("enter".to_owned()),
                row: 0,
            }],
            selected: Some(0),
        });
        semantics.status = Some("1 result".to_owned());
        let update = semantics.tree_update(SEARCH);
        let ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
        for (_, node) in &update.nodes {
            for child in node.children() {
                assert!(ids.contains(child), "{child:?} is named but missing");
            }
        }
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "a node id is used twice");
    }

    #[test]
    fn the_action_panel_is_a_menu_with_its_current_item() {
        let mut semantics = root("", &[], None);
        semantics.panel = Some(Menu {
            items: vec![
                MenuItem {
                    name: "Open".to_owned(),
                    section: None,
                    shortcut: Some("enter".to_owned()),
                    row: 0,
                },
                MenuItem {
                    name: "Copy name".to_owned(),
                    section: Some("Copy".to_owned()),
                    shortcut: None,
                    row: 2,
                },
            ],
            selected: Some(1),
        });
        let update = semantics.tree_update(menu_item_id(1));
        assert_eq!(node(&update, MENU).role(), Role::Menu);
        let open = node(&update, menu_item_id(0));
        assert_eq!(open.role(), Role::MenuItem);
        assert_eq!(open.keyboard_shortcut(), Some("enter"));
        assert!(open.supports_action(Action::Click));
        let copy = node(&update, menu_item_id(1));
        assert_eq!(copy.description(), Some("Copy"));
        assert_eq!(copy.is_selected(), Some(true));
        assert_eq!(semantics.target(menu_item_id(1)), Some(Target::MenuItem(1)));
        assert_eq!(semantics.target(menu_item_id(2)), None);
    }

    #[test]
    fn form_controls_have_their_roles_values_and_states() {
        let semantics = Semantics {
            window: "Compass".to_owned(),
            search: None,
            body: Body::Form(Form {
                label: "Settings".to_owned(),
                fields: vec![
                    Field {
                        label: "Token".to_owned(),
                        kind: FieldKind::Text {
                            value: "abc".to_owned(),
                            placeholder: None,
                            secret: true,
                            multiline: false,
                        },
                        description: Some("From your account page".to_owned()),
                        required: true,
                        invalid: false,
                        target: FieldTarget::Preference(0),
                    },
                    Field {
                        label: "Notify".to_owned(),
                        kind: FieldKind::Checkbox { checked: true },
                        description: None,
                        required: false,
                        invalid: true,
                        target: FieldTarget::Preference(1),
                    },
                    Field {
                        label: "Theme".to_owned(),
                        kind: FieldKind::Dropdown {
                            value: Some("Dark".to_owned()),
                        },
                        description: None,
                        required: false,
                        invalid: false,
                        target: FieldTarget::Preference(2),
                    },
                    Field {
                        label: "Files".to_owned(),
                        kind: FieldKind::Files {
                            chosen: vec!["/a".to_owned(), "/b".to_owned()],
                        },
                        description: None,
                        required: false,
                        invalid: false,
                        target: FieldTarget::Extension("files".to_owned()),
                    },
                    Field {
                        label: "Tags".to_owned(),
                        kind: FieldKind::Tags {
                            options: vec![("One".to_owned(), true), ("Two".to_owned(), false)],
                        },
                        description: None,
                        required: false,
                        invalid: false,
                        target: FieldTarget::Extension("tags".to_owned()),
                    },
                ],
                focused: Some(1),
            }),
            status: None,
            panel: None,
        };
        let update = semantics.tree_update(FocusTracker::new().focus(&semantics));
        assert_eq!(update.focus, field_id(1));
        let token = node(&update, field_id(0));
        assert_eq!(token.role(), Role::PasswordInput);
        assert_eq!(token.label(), Some("Token"));
        assert_eq!(token.value(), Some("•••"), "a secret is never exposed");
        assert!(token.is_required());
        assert_eq!(token.description(), Some("From your account page"));
        let notify = node(&update, field_id(1));
        assert_eq!(notify.role(), Role::CheckBox);
        assert_eq!(notify.toggled(), Some(Toggled::True));
        assert_eq!(notify.invalid(), Some(Invalid::True));
        assert!(notify.supports_action(Action::Click));
        let theme = node(&update, field_id(2));
        assert_eq!(theme.role(), Role::ComboBox);
        assert_eq!(theme.value(), Some("Dark"));
        let files = node(&update, field_id(3));
        assert_eq!(files.role(), Role::Button);
        assert_eq!(files.value(), Some("/a, /b"));
        let tags = node(&update, field_id(4));
        assert!(tags.is_multiselectable());
        assert_eq!(node(&update, field_part_id(4, 0)).is_selected(), Some(true));
        assert_eq!(semantics.target(field_id(3)), Some(Target::Field(3)));
        assert_eq!(
            semantics.target(field_part_id(0, 0)),
            Some(Target::Field(0))
        );
    }

    #[test]
    fn focus_is_on_the_field_while_typing_and_follows_the_selection_once_moved() {
        let mut focus = FocusTracker::new();
        let items = [("Firefox", None), ("Files", None), ("Fonts", None)];
        assert_eq!(focus.focus(&root("", &items, Some(0))), SEARCH);
        assert_eq!(focus.focus(&root("f", &items, Some(0))), SEARCH);
        // Results arriving for the same text do not move it.
        assert_eq!(focus.focus(&root("f", &items[..2], Some(0))), SEARCH);
        // An arrow key does.
        assert_eq!(focus.focus(&root("f", &items, Some(1))), item_id(1));
        assert_eq!(focus.focus(&root("f", &items, Some(0))), item_id(0));
        // A list that changes under it keeps the focus on the selection.
        assert_eq!(focus.focus(&root("f", &items[..1], Some(0))), item_id(0));
        // Typing takes it back to the field.
        assert_eq!(focus.focus(&root("fi", &items, Some(0))), SEARCH);
    }

    #[test]
    fn an_open_panel_or_dialog_takes_the_focus_and_gives_it_back() {
        let mut focus = FocusTracker::new();
        let items = [("Firefox", None), ("Files", None)];
        assert_eq!(focus.focus(&root("f", &items, Some(0))), SEARCH);
        assert_eq!(focus.focus(&root("f", &items, Some(1))), item_id(1));
        let mut with_panel = root("f", &items, Some(1));
        with_panel.panel = Some(Menu {
            items: vec![MenuItem::default()],
            selected: Some(0),
        });
        assert_eq!(focus.focus(&with_panel), menu_item_id(0));
        assert_eq!(focus.focus(&root("f", &items, Some(1))), item_id(1));
        let dialog = Semantics {
            body: Body::Dialog(Dialog::default()),
            ..root("f", &[], None)
        };
        assert_eq!(focus.focus(&dialog), DIALOG);
        assert_eq!(focus.focus(&root("", &[], None)), SEARCH);
    }

    #[test]
    fn requests_name_what_is_there_and_nothing_else() {
        let semantics = root("f", &[("Firefox", None)], Some(0));
        assert_eq!(semantics.target(WINDOW), Some(Target::Window));
        assert_eq!(semantics.target(SEARCH), Some(Target::Search));
        assert_eq!(semantics.target(item_id(0)), Some(Target::Item(0)));
        assert_eq!(semantics.target(item_id(1)), None);
        assert_eq!(semantics.target(STATUS), None);
        assert_eq!(semantics.target(menu_item_id(0)), None);
        assert_eq!(semantics.target(field_id(0)), None);
    }

    #[test]
    fn a_read_only_field_is_a_heading_and_no_target() {
        let semantics = Semantics {
            search: Some(SearchField {
                placeholder: String::new(),
                value: "Script output".to_owned(),
                editable: false,
            }),
            ..Semantics::default()
        };
        let update = semantics.tree_update(WINDOW);
        assert_eq!(node(&update, SEARCH).role(), Role::Heading);
        assert_eq!(node(&update, SEARCH).label(), Some("Script output"));
        assert_eq!(semantics.target(SEARCH), None);
        assert_eq!(FocusTracker::new().focus(&semantics), WINDOW);
    }

    #[test]
    fn the_count_is_a_whole_sentence_in_both_numbers() {
        assert_eq!(result_count(1), "1 result");
        assert_eq!(result_count(3), "3 results");
        assert_eq!(result_count(0), "0 results");
    }
}
