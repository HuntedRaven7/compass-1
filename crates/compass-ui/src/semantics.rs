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
//!
//! # While a search is in flight
//!
//! Typing clears the results and the engine answers a moment later. Said
//! as it happens, that is the list and the status line removed and added
//! again at every keystroke, a "No results" before each count, and a new
//! first row selected each time, which Orca follows out of the field and
//! then loses when the row goes. So the root describes that moment as
//! [`Body::Pending`], and [`Semantics::settle`] keeps the last description
//! under the field until the answer arrives: the rows are relabelled in
//! place, and the count is said once, when it changes. Found by the Orca
//! gate (`scripts/tier2/orca.sh`, #305).

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
    /// The pane beside the list: the selected entry's preview or detail.
    pub detail: Option<Document>,
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
    /// Text to read: a detail, a preview, a command's output.
    Document(Document),
    /// A list beside the form for its selected entry, as the settings show
    /// their pages and the page chosen.
    ListAndForm(List, Form),
    /// Being recomputed: typing cleared the results and the engine has not
    /// answered yet. Described as whatever was there before
    /// ([`Semantics::settle`]).
    Pending,
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
    /// The rows of the grid the items are laid out in, each the items'
    /// positions left to right; `None` for a list, one item to a row.
    pub grid: Option<Vec<Vec<usize>>>,
}

impl List {
    /// Rows of `columns` items each, the last one short, for `count` items.
    #[must_use]
    pub fn grid_of(count: usize, columns: usize) -> Vec<Vec<usize>> {
        (0..count)
            .collect::<Vec<_>>()
            .chunks(columns.max(1))
            .map(<[usize]>::to_vec)
            .collect()
    }
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

/// Text to read, and the facts listed beside it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    /// What it is.
    pub label: String,
    /// Its text, a paragraph or heading at a time.
    pub blocks: Vec<Block>,
    /// Facts about it, as label and value.
    pub metadata: Vec<Metadata>,
}

/// A paragraph, a heading, a list item or a line of code.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Block {
    /// Its text, without markup.
    pub text: String,
    /// Its level, when it is a heading.
    pub heading: Option<usize>,
}

/// One fact in a document's metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Metadata {
    /// What it is.
    pub label: String,
    /// What it says.
    pub value: String,
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
    /// A settings page's or the first-run flow's control, by position in
    /// the order Tab reaches them.
    Control(usize),
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
    /// A switch, on or off.
    Switch {
        /// Whether it is on.
        on: bool,
    },
    /// One of a list.
    Dropdown {
        /// The chosen option's title.
        value: Option<String>,
        /// Every option's title, in order.
        options: Vec<String>,
        /// Whether its list is open, where the launcher draws that list
        /// itself; `None` where the toolkit does, which a request cannot
        /// open.
        expanded: Option<bool>,
        /// The option the keyboard is on while the list is open.
        highlighted: Option<usize>,
    },
    /// A button: it records a shortcut, opens a page or a link.
    Button {
        /// What it shows beside its label, such as the shortcut recorded.
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
    /// One option of a form field: of a dropdown's list or a tag picker's
    /// tags, by position.
    Option {
        /// The field, by position in [`Form::fields`].
        field: usize,
        /// The option, by position.
        option: usize,
    },
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
const DOCUMENT: NodeId = NodeId(10);
const DETAIL: NodeId = NodeId(11);
const ITEM_BASE: u64 = 1 << 32;
const MENU_BASE: u64 = 2 << 32;
const FIELD_BASE: u64 = 3 << 32;
/// A field's own node and its parts (its text, its options) are numbered
/// within a block this wide.
const FIELD_STRIDE: u64 = 1 << 16;
const DOCUMENT_BASE: u64 = 4 << 32;
const DETAIL_BASE: u64 = 5 << 32;
/// A document's blocks are numbered from its base, its metadata from this
/// far above it.
const METADATA_OFFSET: u64 = 1 << 31;
const GRID_ROW_BASE: u64 = 6 << 32;

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

fn grid_row_id(row: usize) -> NodeId {
    NodeId(GRID_ROW_BASE + row as u64)
}

impl Semantics {
    /// The list under the field, alone or beside a form.
    #[must_use]
    pub const fn list(&self) -> Option<&List> {
        match &self.body {
            Body::List(list) | Body::ListAndForm(list, _) => Some(list),
            _ => None,
        }
    }

    /// The form under the field, alone or beside a list.
    #[must_use]
    pub const fn form(&self) -> Option<&Form> {
        match &self.body {
            Body::Form(form) | Body::ListAndForm(_, form) => Some(form),
            _ => None,
        }
    }

    /// What a node of this tree is, for an assistive technology's request
    /// about it. `None` for a node that takes no requests (a text run, the
    /// status line, a document) or one that is no longer there.
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
                self.list()
                    .filter(|list| position < list.items.len())
                    .map(|_| Target::Item(position))
            }
            NodeId(id) if (MENU_BASE..FIELD_BASE).contains(&id) => {
                let position = usize::try_from(id - MENU_BASE).ok()?;
                self.panel
                    .as_ref()
                    .filter(|menu| position < menu.items.len())
                    .map(|_| Target::MenuItem(position))
            }
            NodeId(id) if (FIELD_BASE..DOCUMENT_BASE).contains(&id) => {
                let position = usize::try_from((id - FIELD_BASE) / FIELD_STRIDE).ok()?;
                let field = self.form()?.fields.get(position)?;
                let offset = (id - FIELD_BASE) % FIELD_STRIDE;
                let part = offset
                    .checked_sub(1)
                    .and_then(|part| usize::try_from(part).ok());
                // Which part of a field is an option follows `form_nodes`:
                // a dropdown's first part is its list, a tag's are its tags.
                let option = match (&field.kind, part) {
                    (FieldKind::Dropdown { options, .. }, Some(part)) => {
                        part.checked_sub(1).filter(|&option| option < options.len())
                    }
                    (FieldKind::Tags { options }, Some(part)) => {
                        Some(part).filter(|&option| option < options.len())
                    }
                    _ => None,
                };
                Some(match option {
                    Some(option) => Target::Option {
                        field: position,
                        option,
                    },
                    None => Target::Field(position),
                })
            }
            _ => None,
        }
    }

    /// This description, with a [`Body::Pending`] replaced by what `last`
    /// said under the field (its body, status and detail), or by nothing when there
    /// was no last description. See the module documentation.
    #[must_use]
    pub fn settle(mut self, last: Option<&Semantics>) -> Self {
        if self.body == Body::Pending {
            match last {
                Some(last) => {
                    self.body = last.body.clone();
                    self.status = last.status.clone();
                    self.detail = last.detail.clone();
                }
                None => self.body = Body::Empty,
            }
        }
        self
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
            Body::Empty | Body::Pending => {}
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
            Body::Document(document) => {
                children.push(DOCUMENT);
                document_nodes(document, DOCUMENT, DOCUMENT_BASE, &mut nodes);
            }
            Body::ListAndForm(list, form) => {
                children.push(LIST);
                list_nodes(list, &mut nodes);
                children.push(FORM);
                form_nodes(form, &mut nodes);
            }
        }

        if let Some(detail) = &self.detail {
            children.push(DETAIL);
            document_nodes(detail, DETAIL, DETAIL_BASE, &mut nodes);
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
    let count = list.items.len();
    // Where each item sits in the grid, as (row, column); every item is in
    // a list, while a grid has only the items its rows place.
    let mut places: Vec<Option<(usize, usize)>> = vec![None; count];
    let mut node = match &list.grid {
        // A grid is rows of cells, as a table is, so a screen reader can say
        // where in it a cell is and move up and down as the arrows do.
        Some(rows) => {
            let mut node = Node::new(Role::Grid);
            node.set_row_count(rows.len());
            node.set_column_count(rows.iter().map(Vec::len).max().unwrap_or(0));
            node.set_children((0..rows.len()).map(grid_row_id).collect::<Vec<_>>());
            for (row, cells) in rows.iter().enumerate() {
                let cells: Vec<usize> = cells.iter().copied().filter(|&c| c < count).collect();
                for (column, &cell) in cells.iter().enumerate() {
                    places[cell] = Some((row, column));
                }
                let mut row_node = Node::new(Role::Row);
                row_node.set_row_index(row);
                row_node.set_children(cells.into_iter().map(item_id).collect::<Vec<_>>());
                nodes.push((grid_row_id(row), row_node));
            }
            node
        }
        None => {
            let mut node = Node::new(Role::ListBox);
            node.set_children((0..count).map(item_id).collect::<Vec<_>>());
            node.set_size_of_set(count);
            node
        }
    };
    node.set_label(list.label.as_str());
    nodes.push((LIST, node));
    for (position, item) in list.items.iter().enumerate() {
        let mut node = match (&list.grid, places[position]) {
            (Some(_), Some((row, column))) => {
                let mut node = Node::new(Role::GridCell);
                node.set_row_index(row);
                node.set_column_index(column);
                node
            }
            (Some(_), None) => continue,
            (None, _) => {
                let mut node = Node::new(Role::ListBoxOption);
                node.set_position_in_set(position);
                node.set_size_of_set(count);
                node
            }
        };
        node.set_label(item.name.as_str());
        if let Some(description) = &item.description {
            node.set_description(description.as_str());
        }
        node.set_selected(list.selected == Some(position));
        node.add_action(Action::Click);
        node.add_action(Action::Focus);
        nodes.push((item_id(position), node));
    }
}

/// A document under `root`: its blocks, then its metadata as a list of
/// terms and their values, numbered from `base`.
fn document_nodes(document: &Document, root: NodeId, base: u64, nodes: &mut Vec<(NodeId, Node)>) {
    let block_id = |block: usize| NodeId(base + block as u64);
    let metadata_list = NodeId(base + METADATA_OFFSET);
    let metadata_id =
        |entry: usize, part: u64| NodeId(base + METADATA_OFFSET + 1 + 2 * entry as u64 + part);
    let mut children: Vec<NodeId> = (0..document.blocks.len()).map(block_id).collect();
    if !document.metadata.is_empty() {
        children.push(metadata_list);
    }
    let mut node = Node::new(Role::Document);
    if !document.label.is_empty() {
        node.set_label(document.label.as_str());
    }
    node.set_read_only();
    node.set_children(children);
    nodes.push((root, node));
    for (index, block) in document.blocks.iter().enumerate() {
        // A heading is named by its text; a paragraph is a label whose
        // value is its text, which is what a screen reader reads of one.
        let node = match block.heading {
            Some(level) => {
                let mut node = Node::new(Role::Heading);
                node.set_label(block.text.as_str());
                node.set_level(level);
                node
            }
            None => {
                let mut node = Node::new(Role::Label);
                node.set_value(block.text.as_str());
                node
            }
        };
        nodes.push((block_id(index), node));
    }
    if document.metadata.is_empty() {
        return;
    }
    let mut list = Node::new(Role::DescriptionList);
    list.set_children(
        (0..document.metadata.len())
            .flat_map(|entry| [metadata_id(entry, 0), metadata_id(entry, 1)])
            .collect::<Vec<_>>(),
    );
    nodes.push((metadata_list, list));
    for (entry, metadata) in document.metadata.iter().enumerate() {
        let mut term = Node::new(Role::Term);
        term.set_label(metadata.label.as_str());
        nodes.push((metadata_id(entry, 0), term));
        let mut value = Node::new(Role::Definition);
        value.set_label(metadata.value.as_str());
        nodes.push((metadata_id(entry, 1), value));
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
            FieldKind::Switch { on } => {
                let mut node = Node::new(Role::Switch);
                node.set_toggled(Toggled::from(*on));
                node.add_action(Action::Click);
                node
            }
            FieldKind::Dropdown {
                value,
                options,
                expanded,
                highlighted,
            } => {
                let mut node = Node::new(Role::ComboBox);
                if let Some(value) = value {
                    node.set_value(value.as_str());
                }
                // Only a list the launcher draws itself can be opened and
                // closed on request; the toolkit's own keeps that to itself.
                if let Some(expanded) = expanded {
                    node.set_expanded(*expanded);
                    node.add_action(Action::Click);
                    node.add_action(if *expanded {
                        Action::Collapse
                    } else {
                        Action::Expand
                    });
                }
                let popup = field_part_id(position, 0);
                node.set_children(vec![popup]);
                // The options are there to read and choose whether or not
                // the list is open, so a screen reader can pick one without
                // opening a list it may not be able to open.
                let parts: Vec<NodeId> = (0..options.len())
                    .map(|option| field_part_id(position, option + 1))
                    .collect();
                let mut list = Node::new(Role::MenuListPopup);
                list.set_children(parts.clone());
                if let (Some(true), Some(at)) = (expanded, highlighted)
                    && let Some(&active) = parts.get(*at)
                {
                    list.set_active_descendant(active);
                }
                nodes.push((popup, list));
                for (option, (title, part_id)) in options.iter().zip(parts).enumerate() {
                    let mut node = Node::new(Role::MenuListOption);
                    node.set_label(title.as_str());
                    node.set_selected(value.as_deref() == Some(title.as_str()));
                    node.set_position_in_set(option);
                    node.set_size_of_set(options.len());
                    node.add_action(Action::Click);
                    nodes.push((part_id, node));
                }
                node
            }
            FieldKind::Button { value } => {
                let mut node = Node::new(Role::Button);
                if let Some(value) = value {
                    node.set_value(value.as_str());
                }
                node.add_action(Action::Click);
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
                    option.add_action(Action::Click);
                    nodes.push((part_id, option));
                }
                node
            }
            FieldKind::Files { chosen } => {
                let mut node = Node::new(Role::Button);
                if !chosen.is_empty() {
                    node.set_value(chosen.join(", "));
                }
                node.add_action(Action::Click);
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
        // A preference, argument or settings form follows the keyboard
        // itself; an extension form's focus is Iced's own, which a request
        // cannot move.
        if matches!(
            field.target,
            FieldTarget::Preference(_) | FieldTarget::Control(_)
        ) && !matches!(field.kind, FieldKind::Note { .. })
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
        let selected = semantics
            .list()
            .and_then(|list| list.selected.filter(|&s| s < list.items.len()));
        match &self.last {
            Some((last_query, _)) if *last_query != query => self.on_results = false,
            // A move is from one selected row to another. A list arriving
            // where there was none (the search's answer, after typing
            // cleared the old one) selects its first row without anyone
            // having moved, and the focus stays on the field.
            Some((_, Some(last_selected))) if Some(*last_selected) != selected => {
                self.on_results = selected.is_some();
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
        if let Body::Dialog(_) = &semantics.body {
            return DIALOG;
        }
        if let Some(form) = semantics.form() {
            let focused = form.focused.filter(|&f| f < form.fields.len());
            match (focused, &semantics.body) {
                (Some(focused), _) => return focused_field(form, focused),
                (None, Body::Form(_)) => return FORM,
                _ => {}
            }
        }
        match selected {
            Some(selected) if self.on_results => item_id(selected),
            _ if semantics.search.as_ref().is_some_and(|s| s.editable) => SEARCH,
            _ if matches!(semantics.body, Body::Document(_)) => DOCUMENT,
            _ => WINDOW,
        }
    }
}

/// The node of `form`'s field `position` that has the keyboard: the field,
/// or the option it is on while its list is open.
fn focused_field(form: &Form, position: usize) -> NodeId {
    match form.fields.get(position).map(|field| &field.kind) {
        Some(FieldKind::Dropdown {
            options,
            expanded: Some(true),
            highlighted: Some(at),
            ..
        }) if *at < options.len() => field_part_id(position, at + 1),
        _ => field_id(position),
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

/// Markdown as a screen reader should read it: a block for each heading,
/// paragraph, list item, table row and line of code, without the markup.
#[must_use]
pub fn markdown_blocks(markdown: &str) -> Vec<Block> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut heading = None;
    let mut code = false;
    let mut metadata = false;
    let mut flush = |text: &mut String, heading: &mut Option<usize>| {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            blocks.push(Block {
                text: trimmed.to_owned(),
                heading: *heading,
            });
        }
        text.clear();
        *heading = None;
    };
    // The extensions iced's markdown widget parses with, so what is read is
    // what is drawn: a front-matter block is neither.
    let options = Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
        | Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::MetadataBlock(_)) => metadata = true,
            Event::End(TagEnd::MetadataBlock(_)) => metadata = false,
            _ if metadata => {}
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut text, &mut heading);
                heading = Some(level as usize);
            }
            Event::Start(Tag::Item | Tag::Paragraph | Tag::TableRow | Tag::TableHead) => {
                flush(&mut text, &mut heading);
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut text, &mut heading);
                code = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                flush(&mut text, &mut heading);
                code = false;
            }
            Event::End(
                TagEnd::Heading(_)
                | TagEnd::Paragraph
                | TagEnd::Item
                | TagEnd::TableRow
                | TagEnd::TableHead,
            ) => flush(&mut text, &mut heading),
            Event::End(TagEnd::TableCell) => text.push(' '),
            // Code is read a line at a time, as it is laid out.
            Event::Text(chunk) if code => {
                for line in chunk.split_inclusive('\n') {
                    text.push_str(line);
                    if line.ends_with('\n') {
                        flush(&mut text, &mut heading);
                    }
                }
            }
            Event::Text(chunk) | Event::Code(chunk) | Event::InlineMath(chunk) => {
                text.push_str(&chunk);
            }
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            _ => {}
        }
    }
    flush(&mut text, &mut heading);
    blocks
}

/// Plain text, such as a command's output, a block for each line that has
/// something on it.
#[must_use]
pub fn text_blocks(text: &str) -> Vec<Block> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| Block {
            text: line.to_owned(),
            heading: None,
        })
        .collect()
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
                grid: None,
            }),
            status: None,
            panel: None,
            detail: None,
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
                            options: vec!["Light".to_owned(), "Dark".to_owned()],
                            expanded: None,
                            highlighted: None,
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
            detail: None,
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
        assert_eq!(
            theme.is_expanded(),
            None,
            "the toolkit's list cannot be opened"
        );
        assert!(!theme.supports_action(Action::Expand));
        let dark = node(&update, field_part_id(2, 2));
        assert_eq!(dark.role(), Role::MenuListOption);
        assert_eq!(dark.label(), Some("Dark"));
        assert_eq!(dark.is_selected(), Some(true));
        assert!(dark.supports_action(Action::Click));
        assert_eq!(
            semantics.target(field_part_id(2, 1)),
            Some(Target::Option {
                field: 2,
                option: 0
            })
        );
        assert_eq!(
            semantics.target(field_part_id(2, 0)),
            Some(Target::Field(2))
        );
        let files = node(&update, field_id(3));
        assert_eq!(files.role(), Role::Button);
        assert_eq!(files.value(), Some("/a, /b"));
        assert!(files.supports_action(Action::Click));
        let tags = node(&update, field_id(4));
        assert!(tags.is_multiselectable());
        assert_eq!(node(&update, field_part_id(4, 0)).is_selected(), Some(true));
        assert_eq!(semantics.target(field_id(3)), Some(Target::Field(3)));
        assert_eq!(
            semantics.target(field_part_id(4, 1)),
            Some(Target::Option {
                field: 4,
                option: 1
            })
        );
        assert_eq!(
            semantics.target(field_part_id(0, 0)),
            Some(Target::Field(0))
        );
    }

    /// Every node a node names as a child is in the tree, once.
    fn assert_well_formed(update: &TreeUpdate) {
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
    fn a_grid_is_rows_of_cells_that_say_where_they_are() {
        let mut semantics = root("", &[("A", None), ("B", None), ("C", None)], Some(2));
        if let Body::List(list) = &mut semantics.body {
            list.grid = Some(List::grid_of(3, 2));
        }
        let update = semantics.tree_update(SEARCH);
        assert_well_formed(&update);
        let grid = node(&update, LIST);
        assert_eq!(grid.role(), Role::Grid);
        assert_eq!(grid.row_count(), Some(2));
        assert_eq!(grid.column_count(), Some(2));
        assert_eq!(grid.children(), &[grid_row_id(0), grid_row_id(1)]);
        assert_eq!(node(&update, grid_row_id(1)).children(), &[item_id(2)]);
        let c = node(&update, item_id(2));
        assert_eq!(c.role(), Role::GridCell);
        assert_eq!((c.row_index(), c.column_index()), (Some(1), Some(0)));
        assert_eq!(c.is_selected(), Some(true));
        assert!(c.supports_action(Action::Click));
        assert_eq!(semantics.target(item_id(2)), Some(Target::Item(2)));
        assert_eq!(semantics.target(grid_row_id(0)), None);
    }

    fn document() -> Document {
        Document {
            label: "Readme".to_owned(),
            blocks: markdown_blocks("# Title\n\nSome **bold** text.\n\n- one\n- two"),
            metadata: vec![Metadata {
                label: "Author".to_owned(),
                value: "Ada".to_owned(),
            }],
        }
    }

    #[test]
    fn a_document_is_its_headings_paragraphs_and_metadata() {
        let semantics = Semantics {
            body: Body::Document(document()),
            ..Semantics::default()
        };
        let update = semantics.tree_update(DOCUMENT);
        assert_well_formed(&update);
        let doc = node(&update, DOCUMENT);
        assert_eq!(doc.role(), Role::Document);
        assert_eq!(doc.label(), Some("Readme"));
        assert!(doc.is_read_only());
        let blocks: Vec<&Node> = doc.children().iter().map(|id| node(&update, *id)).collect();
        assert_eq!(blocks[0].role(), Role::Heading);
        assert_eq!(blocks[0].label(), Some("Title"));
        assert_eq!(blocks[0].level(), Some(1));
        assert_eq!(blocks[1].value(), Some("Some bold text."));
        assert_eq!(blocks[2].value(), Some("one"));
        assert_eq!(blocks[3].value(), Some("two"));
        assert_eq!(blocks[4].role(), Role::DescriptionList);
        let entry: Vec<&Node> = blocks[4]
            .children()
            .iter()
            .map(|id| node(&update, *id))
            .collect();
        assert_eq!(entry[0].role(), Role::Term);
        assert_eq!(entry[0].label(), Some("Author"));
        assert_eq!(entry[1].role(), Role::Definition);
        assert_eq!(entry[1].label(), Some("Ada"));
        assert_eq!(semantics.target(DOCUMENT), None);
        assert_eq!(FocusTracker::new().focus(&semantics), DOCUMENT);
    }

    #[test]
    fn a_lists_detail_is_beside_it() {
        let mut semantics = root("", &[("A", None)], Some(0));
        semantics.detail = Some(document());
        let update = semantics.tree_update(SEARCH);
        assert_well_formed(&update);
        assert!(node(&update, WINDOW).children().contains(&DETAIL));
        assert_eq!(node(&update, DETAIL).label(), Some("Readme"));
    }

    #[test]
    fn markdown_is_read_without_its_markup() {
        let blocks = markdown_blocks(
            "---\ntitle: hidden\n---\n## Use\n\n```sh\nls\npwd\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nA `code` span.",
        );
        let texts: Vec<(&str, Option<usize>)> = blocks
            .iter()
            .map(|block| (block.text.as_str(), block.heading))
            .collect();
        assert_eq!(
            texts,
            [
                ("Use", Some(2)),
                ("ls", None),
                ("pwd", None),
                ("a b", None),
                ("1 2", None),
                ("A code span.", None),
            ]
        );
        assert_eq!(text_blocks("one\n\n  \ntwo  \n").len(), 2);
    }

    #[test]
    fn a_list_beside_a_form_takes_requests_for_both_and_an_open_dropdown_its_option() {
        let form = Form {
            label: "General".to_owned(),
            fields: vec![Field {
                label: "Theme".to_owned(),
                kind: FieldKind::Dropdown {
                    value: Some("Light".to_owned()),
                    options: vec!["Light".to_owned(), "Dark".to_owned()],
                    expanded: Some(true),
                    highlighted: Some(1),
                },
                description: None,
                required: false,
                invalid: false,
                target: FieldTarget::Control(0),
            }],
            focused: Some(0),
        };
        let list = List {
            label: "Settings".to_owned(),
            items: vec![Item {
                name: "General".to_owned(),
                description: None,
            }],
            selected: Some(0),
            grid: None,
        };
        let semantics = Semantics {
            body: Body::ListAndForm(list, form),
            ..Semantics::default()
        };
        let focus = FocusTracker::new().focus(&semantics);
        assert_eq!(focus, field_part_id(0, 2), "the highlighted option");
        let update = semantics.tree_update(focus);
        assert_well_formed(&update);
        let theme = node(&update, field_id(0));
        assert_eq!(theme.is_expanded(), Some(true));
        assert!(theme.supports_action(Action::Collapse));
        assert!(theme.supports_action(Action::Focus));
        assert_eq!(
            node(&update, field_part_id(0, 0)).active_descendant(),
            Some(field_part_id(0, 2))
        );
        assert_eq!(semantics.target(item_id(0)), Some(Target::Item(0)));
        assert_eq!(semantics.target(field_id(0)), Some(Target::Field(0)));
    }

    #[test]
    fn switches_and_buttons_are_pressed() {
        let field = |label: &str, kind| Field {
            label: label.to_owned(),
            kind,
            description: None,
            required: false,
            invalid: false,
            target: FieldTarget::Control(0),
        };
        let semantics = Semantics {
            body: Body::Form(Form {
                label: "Applications".to_owned(),
                fields: vec![
                    field("Enabled", FieldKind::Switch { on: false }),
                    field(
                        "Shortcut",
                        FieldKind::Button {
                            value: Some("Ctrl Space".to_owned()),
                        },
                    ),
                ],
                focused: None,
            }),
            ..Semantics::default()
        };
        let update = semantics.tree_update(FORM);
        let switch = node(&update, field_id(0));
        assert_eq!(switch.role(), Role::Switch);
        assert_eq!(switch.toggled(), Some(Toggled::False));
        assert!(switch.supports_action(Action::Click));
        let button = node(&update, field_id(1));
        assert_eq!(button.role(), Role::Button);
        assert_eq!(button.value(), Some("Ctrl Space"));
        assert!(button.supports_action(Action::Click));
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
    fn results_arriving_after_typing_cleared_the_list_leave_the_focus_on_the_field() {
        // The engine answers a search asynchronously: typing empties the
        // list, and the answer fills it with its first row selected. Found
        // by the Orca gate (#305), where the focus jumped to that row on
        // every keystroke.
        let mut focus = FocusTracker::new();
        let items = [("Firefox", None), ("Files", None)];
        assert_eq!(focus.focus(&root("", &items, Some(0))), SEARCH);
        let searching = Semantics {
            body: Body::Empty,
            ..root("f", &[], None)
        };
        assert_eq!(focus.focus(&searching), SEARCH);
        assert_eq!(focus.focus(&root("f", &items, Some(0))), SEARCH);
        assert_eq!(focus.focus(&root("f", &items, Some(1))), item_id(1));
    }

    #[test]
    fn a_pending_body_keeps_what_was_said_under_the_field() {
        let items = [("Firefox", None), ("Files", None)];
        let mut before = root("f", &items, Some(0));
        before.status = Some("2 results".to_owned());
        let pending = Semantics {
            body: Body::Pending,
            ..root("fi", &[], None)
        };
        let settled = pending.clone().settle(Some(&before));
        assert_eq!(settled.search.as_ref().unwrap().value, "fi");
        assert_eq!(settled.body, before.body);
        assert_eq!(settled.status, before.status);
        assert_eq!(pending.settle(None).body, Body::Empty);
        // An answered search is described as it is.
        let answered = root("fi", &items[..1], Some(0));
        assert_eq!(answered.clone().settle(Some(&before)), answered);
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
