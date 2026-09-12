//! Stable identity for view-tree nodes.
//!
//! Identity is *derived*, never allocated from a counter and never random. A node's
//! [`NodeId`] is a pure function of
//!
//! 1. its parent's id,
//! 2. the *slot* it occupies in that parent (`"items"`, `"actions"`, `"detail"`, …), and
//! 3. its [`NodeKey`]: the author-supplied stable key when the node carries one, otherwise
//!    its ordinal position within that slot.
//!
//! Two consequences matter, and they are what the UI relies on:
//!
//! * The same logical tree rendered twice yields byte-identical ids, on any host, in any
//!   process, with no state carried between renders.
//! * Ids do not depend on node *content*. Re-rendering a list whose titles changed keeps
//!   every id, so the UI diffs and patches instead of rebuilding (and keeps selection).
//!
//! The cost is the usual index-key cost: a keyless item inserted at the head of a list
//! shifts every following id. Authors avoid that by setting a key, which is exactly what
//! the extension surface already encourages via item ids.

use std::fmt;

use serde::{Deserialize, Serialize};

/// FNV-1a, 64-bit. Chosen because it is four lines, allocation-free and, unlike
/// [`std::hash::DefaultHasher`], guaranteed stable across releases and processes —
/// which is the whole point of a derived id.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fnv(u64);

impl Fnv {
    pub(crate) const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    /// Writes a length prefix before the payload so that `("ab", "c")` and `("a", "bc")`
    /// do not collide.
    pub(crate) fn write_framed(&mut self, bytes: &[u8]) {
        self.write(&(bytes.len() as u64).to_le_bytes());
        self.write(bytes);
    }

    pub(crate) const fn finish(self) -> u64 {
        self.0
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Self::new()
    }
}

/// What distinguishes a node from its siblings in the same slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKey<'a> {
    /// A key supplied by the extension author. Survives reordering and insertion.
    Stable(&'a str),
    /// Ordinal position within the slot, used when no key was supplied.
    Index(usize),
}

impl NodeKey<'_> {
    fn hash_into(&self, h: &mut Fnv) {
        match self {
            NodeKey::Stable(s) => {
                h.write(b"k");
                h.write_framed(s.as_bytes());
            }
            NodeKey::Index(i) => {
                h.write(b"i");
                h.write(&(*i as u64).to_le_bytes());
            }
        }
    }
}

/// Stable identity of a node in a rendered view tree.
///
/// Serialises as a plain integer so that hosts in other languages can carry it around
/// without a parser. [`fmt::Display`] renders the conventional `n:` + 16 hex digits form
/// used in logs and denial messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(u64);

impl NodeId {
    /// The id every tree walk starts from.
    pub const ROOT: NodeId = NodeId(0xcbf2_9ce4_8422_2325);

    /// Builds an id from its raw value. Only useful when reading an id back from storage;
    /// prefer [`NodeId::child`].
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Derives the id of a child of `self` occupying `slot`, distinguished by `key`.
    #[must_use]
    pub fn child(self, slot: &str, key: NodeKey<'_>) -> Self {
        let mut h = Fnv::new();
        h.write(&self.0.to_le_bytes());
        h.write_framed(slot.as_bytes());
        key.hash_into(&mut h);
        Self(h.finish())
    }

    /// Derives a child id, preferring the author-supplied `key` and falling back to
    /// `index`. This is the form nearly every node uses.
    #[must_use]
    pub fn child_keyed(self, slot: &str, key: Option<&str>, index: usize) -> Self {
        match key {
            Some(k) => self.child(slot, NodeKey::Stable(k)),
            None => self.child(slot, NodeKey::Index(index)),
        }
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::ROOT
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n:{:016x}", self.0)
    }
}
