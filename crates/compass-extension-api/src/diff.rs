//! Diffing two rendered trees.
//!
//! The point of derived identity is that a front end can patch rather than rebuild. This
//! module turns two [`ViewTree`]s into the minimum a UI needs to act on: which nodes are
//! new, which are gone, which kept their id but changed, and which were re-parented.
//!
//! Comparison is by [`NodeId`] and per-node fingerprint. A node whose id and fingerprint
//! both match is untouched even if its children changed, which is what lets a UI stop
//! descending.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::id::NodeId;
use crate::tree::{NodeSummary, ViewTree};

/// A single change between two renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum NodeChange {
    /// The node did not exist in the previous tree.
    Added {
        /// The new node.
        id: NodeId,
        /// Its parent, or `None` for a new root.
        parent: Option<NodeId>,
        /// Its shape.
        kind: Cow<'static, str>,
    },
    /// The node is gone.
    Removed {
        /// The departed node.
        id: NodeId,
        /// Its shape in the previous tree.
        kind: Cow<'static, str>,
    },
    /// Same id, different own fields.
    Updated {
        /// The node.
        id: NodeId,
        /// Its shape.
        kind: Cow<'static, str>,
    },
    /// Same id, same fields, different parent.
    Moved {
        /// The node.
        id: NodeId,
        /// Previous parent.
        from: Option<NodeId>,
        /// New parent.
        to: Option<NodeId>,
    },
}

impl NodeChange {
    /// The node this change concerns.
    #[must_use]
    pub fn id(&self) -> NodeId {
        match self {
            NodeChange::Added { id, .. }
            | NodeChange::Removed { id, .. }
            | NodeChange::Updated { id, .. }
            | NodeChange::Moved { id, .. } => *id,
        }
    }
}

/// The result of comparing two trees.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ViewDiff {
    /// Changes, in the render order of the new tree, removals last.
    pub changes: Vec<NodeChange>,
}

impl ViewDiff {
    /// Compares `previous` with `next`.
    #[must_use]
    pub fn between(previous: &ViewTree, next: &ViewTree) -> Self {
        let old: BTreeMap<NodeId, NodeSummary> =
            previous.nodes().into_iter().map(|n| (n.id, n)).collect();
        let mut seen: BTreeMap<NodeId, ()> = BTreeMap::new();
        let mut changes = Vec::new();

        for node in next.nodes() {
            seen.insert(node.id, ());
            match old.get(&node.id) {
                None => changes.push(NodeChange::Added {
                    id: node.id,
                    parent: node.parent,
                    kind: node.kind.clone(),
                }),
                Some(before) => {
                    if before.fingerprint != node.fingerprint || before.kind != node.kind {
                        changes.push(NodeChange::Updated {
                            id: node.id,
                            kind: node.kind.clone(),
                        });
                    } else if before.parent != node.parent {
                        changes.push(NodeChange::Moved {
                            id: node.id,
                            from: before.parent,
                            to: node.parent,
                        });
                    }
                }
            }
        }

        for (id, node) in &old {
            if !seen.contains_key(id) {
                changes.push(NodeChange::Removed {
                    id: *id,
                    kind: node.kind.clone(),
                });
            }
        }

        Self { changes }
    }

    /// Whether the two trees were identical.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Number of changes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.changes.len()
    }
}
