//! The view tree: the *rendered result* an extension produces, in a form any front end
//! can draw.
//!
//! This is not a component model and it is not a description of React. There are no
//! fragments, no keys-as-props, no lifecycle, no children-of-unknown-type. What arrives
//! here is the tree after reconciliation: four view shapes (list, grid, detail, form),
//! each already normalised — loose items folded into an implicit untitled section, loose
//! actions folded into an implicit untitled panel section — so a front end has exactly
//! one layout to implement per shape.

use serde::{Deserialize, Serialize};

use crate::action::{ActionPanel, HandlerId};
use crate::id::NodeId;

/// A colour, either a theme role the front end resolves or a literal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Color {
    /// A named theme role, e.g. `primary`, `red`. Unknown names fall back to the default
    /// text colour rather than failing.
    Named(String),
    /// A literal `#rrggbb` or `#rrggbbaa` value.
    Literal(String),
}

/// Where an image comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ImageSource {
    /// A built-in icon name from the host's icon set.
    Builtin(String),
    /// A path relative to the extension's own asset directory.
    Asset(String),
    /// An absolute URL.
    Url(String),
    /// The system icon for a file, by path.
    FileIcon(String),
    /// A pair chosen by the active theme.
    Themed {
        /// Source used under a light theme.
        light: Box<ImageSource>,
        /// Source used under a dark theme.
        dark: Box<ImageSource>,
    },
}

/// How an image is masked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMask {
    /// Clipped to a circle.
    Circle,
    /// Clipped to a rounded rectangle.
    RoundedRectangle,
}

/// An image reference. Never image *data* — resolution and caching belong to the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    /// Primary source.
    pub source: ImageSource,
    /// Used when the primary source cannot be resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<ImageSource>,
    /// Tint applied to a monochrome source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<Color>,
    /// Mask applied when drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<ImageMask>,
}

impl Image {
    /// A built-in icon by name.
    pub fn builtin(name: impl Into<String>) -> Self {
        Self {
            source: ImageSource::Builtin(name.into()),
            fallback: None,
            tint: None,
            mask: None,
        }
    }
}

/// Trailing decoration on a list row.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Accessory {
    /// Plain trailing text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Text rendered as a pill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Colour applied to whichever of `text`/`tag` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Trailing icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Image>,
    /// Hover text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
}

/// A tag inside a metadata tag list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataTag {
    /// Tag text.
    pub text: String,
    /// Optional icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Image>,
    /// Optional colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Fired when the tag is activated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<HandlerId>,
}

/// A row in a detail's metadata panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetadataItem {
    /// Title plus value.
    Label {
        /// Row title.
        title: String,
        /// Row value.
        text: String,
        /// Optional icon.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<Image>,
        /// Optional colour for the value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
    },
    /// Title plus a hyperlink.
    Link {
        /// Row title.
        title: String,
        /// Link text.
        text: String,
        /// Link target.
        target: String,
    },
    /// A list of tags under a title.
    TagList {
        /// Row title.
        title: String,
        /// The tags.
        tags: Vec<MetadataTag>,
    },
    /// A horizontal rule.
    Separator,
}

/// Markdown plus an optional metadata panel. Used both as a standalone view and as the
/// right-hand pane of a list row.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Detail {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Markdown body. `None` renders as blank, not as an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    /// Metadata rows.
    #[serde(default)]
    pub metadata: Vec<MetadataItem>,
    /// Set while the body is still being produced.
    #[serde(default)]
    pub is_loading: bool,
    /// Title shown in the navigation bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_title: Option<String>,
    /// Actions for this detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<ActionPanel>,
}

/// What a view shows when it has nothing to show.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EmptyState {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Headline.
    #[serde(default)]
    pub title: String,
    /// Supporting text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Illustration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Image>,
    /// Actions offered in the empty state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<ActionPanel>,
}

/// Incremental loading state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pagination {
    /// Whether another page exists.
    #[serde(default)]
    pub has_more: bool,
    /// Number of items loaded so far, for the UI's progress affordance.
    #[serde(default)]
    pub loaded: u32,
    /// Fired when the UI wants the next page.
    pub on_load_more: HandlerId,
}

/// A search-bar dropdown option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropdownOption {
    /// Display title.
    pub title: String,
    /// Value reported on change.
    pub value: String,
    /// Optional icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Image>,
    /// Extra terms the filter should match.
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// A titled group of dropdown options.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DropdownSection {
    /// Optional title; `None` is the implicit untitled group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Options.
    #[serde(default)]
    pub options: Vec<DropdownOption>,
}

/// A dropdown, used both as a search-bar accessory and as a form field.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Dropdown {
    /// Placeholder shown when nothing is selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Currently selected value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The edit this value answers, when it answers one.
    ///
    /// `None` means the extension is *setting* the value, not echoing the user's; see
    /// [`crate::input`] for why the host must treat those differently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<crate::input::Seq>,
    /// Option groups.
    #[serde(default)]
    pub sections: Vec<DropdownSection>,
    /// Whether the host filters options as the user types.
    #[serde(default)]
    pub filtering: bool,
    /// Fired when the selection changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_change: Option<HandlerId>,
}

/// Search bar configuration, shared by list and grid.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SearchBar {
    /// Placeholder text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Current text, when the extension controls it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The edit this value answers, when it answers one.
    ///
    /// `None` means the extension is *setting* the value, not echoing the user's; see
    /// [`crate::input`] for why the host must treat those differently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<crate::input::Seq>,
    /// Whether the host filters items itself.
    #[serde(default)]
    pub host_filtering: bool,
    /// Fired as the text changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_change: Option<HandlerId>,
    /// Optional accessory dropdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessory: Option<Dropdown>,
}

/// A row in a list.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ListItem {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Author-supplied stable key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Primary text.
    #[serde(default)]
    pub title: String,
    /// Secondary text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Leading icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Image>,
    /// Trailing decorations.
    #[serde(default)]
    pub accessories: Vec<Accessory>,
    /// Extra terms the filter should match.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Detail pane shown when the row is selected and the list shows details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Detail>,
    /// Actions for this row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<ActionPanel>,
}

impl ListItem {
    /// A row with a title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    /// Builder: attach a stable key.
    #[must_use]
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Builder: attach actions.
    #[must_use]
    pub fn with_actions(mut self, actions: ActionPanel) -> Self {
        self.actions = Some(actions);
        self
    }
}

/// A titled group of list rows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ListSection {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Author-supplied stable key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Optional title; `None` is the implicit untitled section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional subtitle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Rows.
    #[serde(default)]
    pub items: Vec<ListItem>,
}

impl ListSection {
    /// An untitled section wrapping loose items.
    pub fn untitled(items: impl IntoIterator<Item = ListItem>) -> Self {
        Self {
            items: items.into_iter().collect(),
            ..Self::default()
        }
    }
}

/// A filterable list of rows, optionally with a detail pane.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ListView {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Title shown in the navigation bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_title: Option<String>,
    /// Set while items are being produced. Independent of emptiness: an empty *and*
    /// loading list shows a spinner, an empty and settled one shows `empty_state`.
    #[serde(default)]
    pub is_loading: bool,
    /// Whether the detail pane is shown.
    #[serde(default)]
    pub show_detail: bool,
    /// Currently selected row, by node id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<NodeId>,
    /// Fired when the selection changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_selection_change: Option<HandlerId>,
    /// Search bar.
    #[serde(default)]
    pub search: SearchBar,
    /// Sections, in order.
    #[serde(default)]
    pub sections: Vec<ListSection>,
    /// Actions that apply to the view rather than a row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<ActionPanel>,
    /// Shown when there are no rows and the view is not loading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_state: Option<EmptyState>,
    /// Incremental loading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
}

/// How a grid cell fits its content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridFit {
    /// Whole image visible.
    #[default]
    Contain,
    /// Cell filled, image cropped.
    Fill,
}

/// Padding inside a grid cell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridInset {
    /// No inset.
    #[default]
    None,
    /// Small inset.
    Small,
    /// Medium inset.
    Medium,
    /// Large inset.
    Large,
}

/// What fills a grid cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GridContent {
    /// An image.
    Image(Image),
    /// A flat colour.
    Color(Color),
}

/// Cell aspect ratio, as an exact `width:height` pair.
///
/// A pair rather than a float so the type stays `Eq` and round-trips through JSON without
/// the usual decimal drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AspectRatio {
    /// Width term.
    pub width: u16,
    /// Height term.
    pub height: u16,
}

/// A cell in a grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridItem {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Author-supplied stable key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Caption.
    #[serde(default)]
    pub title: String,
    /// Sub-caption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Cell content.
    pub content: GridContent,
    /// Hover text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// Extra terms the filter should match.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Actions for this cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<ActionPanel>,
}

/// A titled group of grid cells.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GridSection {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Author-supplied stable key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Optional title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional subtitle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Column count override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u16>,
    /// Aspect ratio override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<AspectRatio>,
    /// Inset override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inset: Option<GridInset>,
    /// Fit override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<GridFit>,
    /// Cells.
    #[serde(default)]
    pub items: Vec<GridItem>,
}

/// A grid of cells.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GridView {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Title shown in the navigation bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_title: Option<String>,
    /// Set while cells are being produced.
    #[serde(default)]
    pub is_loading: bool,
    /// Default column count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u16>,
    /// Default aspect ratio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<AspectRatio>,
    /// Default inset.
    #[serde(default)]
    pub inset: GridInset,
    /// Default fit.
    #[serde(default)]
    pub fit: GridFit,
    /// Currently selected cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<NodeId>,
    /// Fired when the selection changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_selection_change: Option<HandlerId>,
    /// Search bar.
    #[serde(default)]
    pub search: SearchBar,
    /// Sections, in order.
    #[serde(default)]
    pub sections: Vec<GridSection>,
    /// View-level actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<ActionPanel>,
    /// Shown when there are no cells and the view is not loading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_state: Option<EmptyState>,
    /// Incremental loading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
}

/// A value held by a form field or submitted with a form.
///
/// Deliberately not an arbitrary JSON value: a closed, `Eq`-able set with no floating
/// point, so submitted values compare and round-trip exactly. Dates travel as RFC 3339
/// strings because the seam owns no date library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum FieldValue {
    /// Free text.
    Text(String),
    /// A boolean.
    Bool(bool),
    /// A whole number.
    Integer(i64),
    /// An RFC 3339 timestamp.
    Date(String),
    /// A set of filesystem paths.
    Paths(Vec<String>),
    /// A set of selected values.
    Values(Vec<String>),
    /// Explicitly empty.
    Empty,
}

/// Granularity a date picker accepts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatePrecision {
    /// Date only.
    #[default]
    Day,
    /// Date and time.
    Minute,
}

/// The typed part of a form field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldKind {
    /// Single-line text.
    Text {
        /// Placeholder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    /// Single-line concealed text.
    Password {
        /// Placeholder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    /// Multi-line text.
    TextArea {
        /// Placeholder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
        /// Whether the host highlights markdown.
        #[serde(default)]
        markdown: bool,
    },
    /// A checkbox.
    Checkbox {
        /// Label beside the box.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A dropdown.
    Dropdown(Dropdown),
    /// A multi-select tag picker.
    TagPicker {
        /// Available options.
        #[serde(default)]
        options: Vec<DropdownOption>,
        /// Placeholder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    /// A date picker.
    DatePicker {
        /// Earliest accepted value, RFC 3339.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<String>,
        /// Latest accepted value, RFC 3339.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<String>,
        /// Granularity.
        #[serde(default)]
        precision: DatePrecision,
    },
    /// A file picker.
    FilePicker {
        /// Whether more than one path may be chosen.
        #[serde(default)]
        allow_multiple: bool,
        /// Whether directories may be chosen.
        #[serde(default)]
        allow_directories: bool,
        /// Whether files may be chosen.
        #[serde(default = "crate::view::default_true")]
        allow_files: bool,
    },
}

pub(crate) fn default_true() -> bool {
    true
}

/// A named, typed input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormField {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// The field name. Also the author-supplied stable key, and the key this field's
    /// value appears under on submission — a form field without a name is meaningless,
    /// so unlike other nodes this key is mandatory.
    pub name: String,
    /// Label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Validation message currently shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
    /// Whether the field takes focus on render.
    #[serde(default)]
    pub autofocus: bool,
    /// Current value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<FieldValue>,
    /// The edit this value answers, when it answers one.
    ///
    /// `None` means the extension is *setting* the value, not echoing the user's; see
    /// [`crate::input`] for why the host must treat those differently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<crate::input::Seq>,
    /// Fired when the value changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_change: Option<HandlerId>,
    /// The typed part.
    pub kind: FieldKind,
}

/// An entry in a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// Tagged `item`, not `kind`: a form field has a `kind` of its own and the two would
// collide in the flattened representation.
#[serde(tag = "item", rename_all = "snake_case")]
pub enum FormItem {
    /// An input. Boxed: a field is an order of magnitude larger than the other two
    /// variants and there are usually few of them.
    Field(Box<FormField>),
    /// Static explanatory text.
    Description {
        /// Derived identity.
        #[serde(default)]
        id: NodeId,
        /// Optional heading.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Body text.
        text: String,
    },
    /// A horizontal rule.
    Separator {
        /// Derived identity.
        #[serde(default)]
        id: NodeId,
    },
}

/// A form.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FormView {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Title shown in the navigation bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_title: Option<String>,
    /// Set while the form is busy.
    #[serde(default)]
    pub is_loading: bool,
    /// Whether the host keeps a draft of unsubmitted values.
    #[serde(default)]
    pub enable_drafts: bool,
    /// Entries, in order.
    #[serde(default)]
    pub items: Vec<FormItem>,
    /// Actions, normally including the submit action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<ActionPanel>,
}

/// The four view shapes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum View {
    /// A list.
    List(ListView),
    /// A grid.
    Grid(GridView),
    /// A detail.
    Detail(Detail),
    /// A form.
    Form(FormView),
}

impl Default for View {
    fn default() -> Self {
        View::List(ListView::default())
    }
}

impl View {
    /// The root node's id.
    #[must_use]
    pub fn id(&self) -> NodeId {
        match self {
            View::List(v) => v.id,
            View::Grid(v) => v.id,
            View::Detail(v) => v.id,
            View::Form(v) => v.id,
        }
    }

    /// A short tag naming the shape, used in ids, diffs and logs.
    #[must_use]
    pub fn tag(&self) -> &'static str {
        match self {
            View::List(_) => "list",
            View::Grid(_) => "grid",
            View::Detail(_) => "detail",
            View::Form(_) => "form",
        }
    }
}
