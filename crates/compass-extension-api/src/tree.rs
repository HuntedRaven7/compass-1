//! The rendered tree, its id assignment pass and its flattening.

use std::borrow::Cow;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionItem, ActionPanel, HandlerId, Shortcut};
use crate::id::{Fnv, NodeId};
use crate::view::{Detail, EmptyState, FormItem, FormView, GridView, ListView, SearchBar, View};

/// Hashes a small leaf value into `h`. Leaf values only — never a node with children,
/// which would make the walk quadratic.
fn leaf<T: Serialize>(value: &T, h: &mut Fnv) {
    match serde_json::to_string(value) {
        Ok(s) => h.write_framed(s.as_bytes()),
        // Unreachable for the types in this crate, but a fingerprint must never panic.
        Err(_) => h.write(b"\0serialize-failed"),
    }
}

macro_rules! fingerprint {
    ($($field:expr),* $(,)?) => {{
        let mut h = Fnv::new();
        $( leaf(&$field, &mut h); )*
        h.finish()
    }};
}

/// One node, flattened out of the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSummary {
    /// The node's derived identity.
    pub id: NodeId,
    /// Its parent, or `None` for the root.
    pub parent: Option<NodeId>,
    /// A short tag naming the node's shape. Owned on the wire so that a tree can be
    /// read back from storage; borrowed in practice.
    pub kind: Cow<'static, str>,
    /// Hash of the node's *own* fields — children excluded, id excluded. Two nodes with
    /// the same id and the same fingerprint need no repaint.
    pub fingerprint: u64,
}

/// An action found in the tree, with everything dispatch needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRef {
    /// The action node's identity.
    pub node: NodeId,
    /// The token to hand back to the extension.
    pub handler: HandlerId,
    /// Title, for denial and log messages the user reads.
    pub title: String,
    /// Shortcut, if the action has one.
    pub shortcut: Option<Shortcut>,
}

#[derive(Debug, Default)]
pub(crate) struct Visit {
    pub(crate) nodes: Vec<NodeSummary>,
    pub(crate) actions: Vec<ActionRef>,
}

impl Visit {
    fn push(&mut self, id: NodeId, parent: Option<NodeId>, kind: &'static str, fingerprint: u64) {
        self.nodes.push(NodeSummary {
            id,
            parent,
            kind: Cow::Borrowed(kind),
            fingerprint,
        });
    }
}

/// A rendered view tree with identity assigned.
///
/// Construct with [`ViewTree::new`]; that is the only way ids get stamped, and it is a
/// pure function of the tree's shape and keys.
///
/// Deserialising re-runs that assignment rather than trusting the ids in the payload.
/// The derivation is idempotent, so a tree built by [`ViewTree::new`] round-trips
/// unchanged; what the pass buys is that ids in *every* `ViewTree` that exists are ones
/// this crate derived. A stale, hand-written or hostile payload cannot smuggle in a
/// duplicate id and make the diff misattribute one node's fields to another.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ViewTree {
    root: View,
}

impl<'de> Deserialize<'de> for ViewTree {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Same shape on the wire as the derive would produce -- a one-field struct -- so
        // this is a validation pass, not a format change.
        #[derive(Deserialize)]
        #[serde(rename = "ViewTree")]
        struct Wire {
            root: View,
        }

        Wire::deserialize(deserializer).map(|w| ViewTree::new(w.root))
    }
}

impl ViewTree {
    /// Assigns identity to every node and returns the tree.
    #[must_use]
    pub fn new(mut root: View) -> Self {
        let id = NodeId::ROOT.child("root", crate::id::NodeKey::Stable(root.tag()));
        match &mut root {
            View::List(v) => assign_list(v, id),
            View::Grid(v) => assign_grid(v, id),
            View::Detail(v) => assign_detail(v, id),
            View::Form(v) => assign_form(v, id),
        }
        Self { root }
    }

    /// The root view.
    #[must_use]
    pub fn root(&self) -> &View {
        &self.root
    }

    /// Consumes the tree, yielding the root view.
    #[must_use]
    pub fn into_root(self) -> View {
        self.root
    }

    /// Every node, in render order.
    #[must_use]
    pub fn nodes(&self) -> Vec<NodeSummary> {
        self.visit().nodes
    }

    /// Every action reachable in the tree, in render order.
    #[must_use]
    pub fn actions(&self) -> Vec<ActionRef> {
        self.visit().actions
    }

    pub(crate) fn visit(&self) -> Visit {
        let mut v = Visit::default();
        match &self.root {
            View::List(x) => visit_list(x, None, &mut v),
            View::Grid(x) => visit_grid(x, None, &mut v),
            View::Detail(x) => visit_detail(x, None, "detail", &mut v),
            View::Form(x) => visit_form(x, None, &mut v),
        }
        v
    }
}

// ---------------------------------------------------------------------------
// identity assignment
// ---------------------------------------------------------------------------

/// Hands out sibling ids within one slot of one parent, guaranteeing they are distinct.
///
/// Nothing stops an extension emitting `key: "row"` on two items in the same list, and
/// two siblings with the same key derive the same [`NodeId`]. Left alone that corrupts
/// the diff: [`ViewDiff::between`](crate::ViewDiff::between) matches by id, so one
/// sibling's fingerprint is compared against the other's and the UI patches the wrong
/// row -- a tree even reports changes against *itself*.
///
/// So the first claimant of a key keeps the derived id, and any later sibling claiming
/// the same key falls back to its ordinal. The fallback cannot itself collide:
/// `NodeKey::Index(i)` is only reachable by the sibling at position `i`, and a slot has
/// exactly one of those.
///
/// This is a repair, not a feature. A duplicate key still costs the author what keys buy
/// -- the demoted sibling's id now shifts when the list is reordered, exactly as a keyless
/// item's would. Correct output for an incorrect input, which is the most a pure derivation
/// can offer without a channel to complain on.
struct Slot<'a> {
    parent: NodeId,
    name: &'a str,
    taken: BTreeSet<NodeId>,
}

impl<'a> Slot<'a> {
    fn new(parent: NodeId, name: &'a str) -> Self {
        Self {
            parent,
            name,
            taken: BTreeSet::new(),
        }
    }

    /// The id for the sibling at `index` carrying `key`.
    fn id_for(&mut self, key: Option<&str>, index: usize) -> NodeId {
        let derived = self.parent.child_keyed(self.name, key, index);
        if self.taken.insert(derived) {
            return derived;
        }

        let ordinal = self
            .parent
            .child(self.name, crate::id::NodeKey::Index(index));
        let fresh = self.taken.insert(ordinal);
        debug_assert!(
            fresh,
            "ordinal fallback collided, which position uniqueness should make impossible"
        );
        ordinal
    }
}

fn assign_panel(panel: &mut ActionPanel, parent: NodeId) {
    panel.id = parent.child("actions", crate::id::NodeKey::Index(0));
    let mut slot = Slot::new(panel.id, "sections");
    for (si, section) in panel.sections.iter_mut().enumerate() {
        section.id = slot.id_for(section.title.as_deref(), si);
        assign_items(&mut section.items, section.id);
    }
}

fn assign_items(items: &mut [ActionItem], parent: NodeId) {
    let mut slot = Slot::new(parent, "items");
    for (i, item) in items.iter_mut().enumerate() {
        match item {
            ActionItem::Action(a) => {
                a.id = slot.id_for(a.key.as_deref(), i);
            }
            ActionItem::Submenu(s) => {
                s.id = slot.id_for(s.key.as_deref(), i);
                let sid = s.id;
                assign_items(&mut s.items, sid);
            }
        }
    }
}

fn assign_empty(empty: &mut EmptyState, parent: NodeId) {
    empty.id = parent.child("empty", crate::id::NodeKey::Index(0));
    if let Some(p) = &mut empty.actions {
        assign_panel(p, empty.id);
    }
}

fn assign_detail(detail: &mut Detail, id: NodeId) {
    detail.id = id;
    if let Some(p) = &mut detail.actions {
        assign_panel(p, id);
    }
}

fn assign_list(view: &mut ListView, id: NodeId) {
    view.id = id;
    let mut sections = Slot::new(id, "sections");
    for (si, section) in view.sections.iter_mut().enumerate() {
        section.id = sections.id_for(section.key.as_deref().or(section.title.as_deref()), si);
        let mut items = Slot::new(section.id, "items");
        for (ii, item) in section.items.iter_mut().enumerate() {
            item.id = items.id_for(item.key.as_deref(), ii);
            let iid = item.id;
            if let Some(d) = &mut item.detail {
                assign_detail(d, iid.child("detail", crate::id::NodeKey::Index(0)));
            }
            if let Some(p) = &mut item.actions {
                assign_panel(p, iid);
            }
        }
    }
    if let Some(p) = &mut view.actions {
        assign_panel(p, id);
    }
    if let Some(e) = &mut view.empty_state {
        assign_empty(e, id);
    }
}

fn assign_grid(view: &mut GridView, id: NodeId) {
    view.id = id;
    let mut sections = Slot::new(id, "sections");
    for (si, section) in view.sections.iter_mut().enumerate() {
        section.id = sections.id_for(section.key.as_deref().or(section.title.as_deref()), si);
        let mut items = Slot::new(section.id, "items");
        for (ii, item) in section.items.iter_mut().enumerate() {
            item.id = items.id_for(item.key.as_deref(), ii);
            let iid = item.id;
            if let Some(p) = &mut item.actions {
                assign_panel(p, iid);
            }
        }
    }
    if let Some(p) = &mut view.actions {
        assign_panel(p, id);
    }
    if let Some(e) = &mut view.empty_state {
        assign_empty(e, id);
    }
}

fn assign_form(view: &mut FormView, id: NodeId) {
    view.id = id;
    let mut slot = Slot::new(id, "items");
    for (i, item) in view.items.iter_mut().enumerate() {
        match item {
            // A field's `name` is its key: it is what the submitted value map is keyed by,
            // so two fields sharing one is already an author error the form layer reports.
            // Identity still has to stay distinct so the UI can render both.
            FormItem::Field(f) => f.id = slot.id_for(Some(&f.name), i),
            FormItem::Description { id: did, .. } => *did = slot.id_for(None, i),
            FormItem::Separator { id: sid } => *sid = slot.id_for(None, i),
        }
    }
    if let Some(p) = &mut view.actions {
        assign_panel(p, id);
    }
}

// ---------------------------------------------------------------------------
// flattening
// ---------------------------------------------------------------------------

/// A search bar's *painted* content, with the echo counters left out.
///
/// `echo` on a [`SearchBar`] or its accessory [`Dropdown`] is protocol bookkeeping (see
/// [`crate::input`]), not something the user can see. Hashing it would make a render that
/// only re-stamps the counter report as an update, so the two surfaces that nest a search
/// bar project it through here first. Form fields get the same treatment for free, since
/// their fingerprint enumerates fields rather than hashing the struct.
fn search_content(s: &SearchBar) -> impl Serialize + '_ {
    (
        &s.placeholder,
        &s.text,
        s.host_filtering,
        &s.on_change,
        s.accessory.as_ref().map(|d| {
            (
                &d.value,
                &d.placeholder,
                &d.on_change,
                &d.sections,
                d.filtering,
            )
        }),
    )
}

fn visit_panel(panel: &ActionPanel, parent: NodeId, v: &mut Visit) {
    v.push(
        panel.id,
        Some(parent),
        "action_panel",
        fingerprint!(panel.title),
    );
    for section in &panel.sections {
        v.push(
            section.id,
            Some(panel.id),
            "action_section",
            fingerprint!(section.title),
        );
        visit_action_items(&section.items, section.id, v);
    }
}

fn visit_action_items(items: &[ActionItem], parent: NodeId, v: &mut Visit) {
    for item in items {
        match item {
            ActionItem::Action(a) => visit_action(a, parent, v),
            ActionItem::Submenu(s) => {
                v.push(
                    s.id,
                    Some(parent),
                    "action_submenu",
                    fingerprint!(s.key, s.title, s.icon, s.shortcut, s.on_open),
                );
                visit_action_items(&s.items, s.id, v);
            }
        }
    }
}

fn visit_action(a: &Action, parent: NodeId, v: &mut Visit) {
    v.push(
        a.id,
        Some(parent),
        "action",
        fingerprint!(a.key, a.title, a.icon, a.shortcut, a.style, a.handler),
    );
    v.actions.push(ActionRef {
        node: a.id,
        handler: a.handler.clone(),
        title: a.title.clone(),
        shortcut: a.shortcut.clone(),
    });
}

fn visit_empty(e: &EmptyState, parent: NodeId, v: &mut Visit) {
    v.push(
        e.id,
        Some(parent),
        "empty_state",
        fingerprint!(e.title, e.description, e.icon),
    );
    if let Some(p) = &e.actions {
        visit_panel(p, e.id, v);
    }
}

fn visit_detail(d: &Detail, parent: Option<NodeId>, kind: &'static str, v: &mut Visit) {
    v.push(
        d.id,
        parent,
        kind,
        fingerprint!(d.markdown, d.metadata, d.is_loading, d.navigation_title),
    );
    if let Some(p) = &d.actions {
        visit_panel(p, d.id, v);
    }
}

fn visit_list(view: &ListView, parent: Option<NodeId>, v: &mut Visit) {
    v.push(
        view.id,
        parent,
        "list",
        fingerprint!(
            view.navigation_title,
            view.is_loading,
            view.show_detail,
            view.selected,
            view.on_selection_change,
            search_content(&view.search),
            view.pagination,
        ),
    );
    for section in &view.sections {
        v.push(
            section.id,
            Some(view.id),
            "list_section",
            fingerprint!(section.key, section.title, section.subtitle),
        );
        for item in &section.items {
            v.push(
                item.id,
                Some(section.id),
                "list_item",
                fingerprint!(
                    item.key,
                    item.title,
                    item.subtitle,
                    item.icon,
                    item.accessories,
                    item.keywords
                ),
            );
            if let Some(d) = &item.detail {
                visit_detail(d, Some(item.id), "item_detail", v);
            }
            if let Some(p) = &item.actions {
                visit_panel(p, item.id, v);
            }
        }
    }
    if let Some(p) = &view.actions {
        visit_panel(p, view.id, v);
    }
    if let Some(e) = &view.empty_state {
        visit_empty(e, view.id, v);
    }
}

fn visit_grid(view: &GridView, parent: Option<NodeId>, v: &mut Visit) {
    v.push(
        view.id,
        parent,
        "grid",
        fingerprint!(
            view.navigation_title,
            view.is_loading,
            view.columns,
            view.aspect_ratio,
            view.inset,
            view.fit,
            view.selected,
            view.on_selection_change,
            search_content(&view.search),
            view.pagination,
        ),
    );
    for section in &view.sections {
        v.push(
            section.id,
            Some(view.id),
            "grid_section",
            fingerprint!(
                section.key,
                section.title,
                section.subtitle,
                section.columns,
                section.aspect_ratio,
                section.inset,
                section.fit
            ),
        );
        for item in &section.items {
            v.push(
                item.id,
                Some(section.id),
                "grid_item",
                fingerprint!(
                    item.key,
                    item.title,
                    item.subtitle,
                    item.content,
                    item.tooltip,
                    item.keywords
                ),
            );
            if let Some(p) = &item.actions {
                visit_panel(p, item.id, v);
            }
        }
    }
    if let Some(p) = &view.actions {
        visit_panel(p, view.id, v);
    }
    if let Some(e) = &view.empty_state {
        visit_empty(e, view.id, v);
    }
}

fn visit_form(view: &FormView, parent: Option<NodeId>, v: &mut Visit) {
    v.push(
        view.id,
        parent,
        "form",
        fingerprint!(view.navigation_title, view.is_loading, view.enable_drafts),
    );
    for item in &view.items {
        match item {
            FormItem::Field(f) => v.push(
                f.id,
                Some(view.id),
                "form_field",
                fingerprint!(
                    f.name,
                    f.title,
                    f.error,
                    f.info,
                    f.autofocus,
                    f.value,
                    f.on_change,
                    f.kind
                ),
            ),
            FormItem::Description { id, title, text } => {
                v.push(
                    *id,
                    Some(view.id),
                    "form_description",
                    fingerprint!(title, text),
                );
            }
            FormItem::Separator { id } => {
                v.push(*id, Some(view.id), "form_separator", fingerprint!(()));
            }
        }
    }
    if let Some(p) = &view.actions {
        visit_panel(p, view.id, v);
    }
}
