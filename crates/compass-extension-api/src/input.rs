//! Keeping a controlled input from being overwritten by a stale echo.
//!
//! A controlled input is one whose value the extension owns: the user types, the host
//! reports the change, the extension re-renders with a value, and the host displays it.
//! That round trip is not instant, and the user does not stop typing during it.
//!
//! ```text
//! user types "a"     -> host sends change("a")
//! user types "ab"    -> host sends change("ab")
//! user types "abc"   -> host sends change("abc")
//!                    <- extension renders value "a"      (answering the first change)
//! ```
//!
//! Applying that render puts "a" back in a box the user has typed "abc" into, and their
//! next keystroke lands after the truncation. Every controlled-input surface hits this;
//! it is not a rare race, it is what happens whenever the extension is slower than a
//! typist.
//!
//! The fix is to count. Each local edit to a node mints a [`Seq`]; the host sends it with
//! the change; a render echoes back the [`Seq`] whose change it answers. The host then
//! knows whether an incoming value is current or was computed before the user's latest
//! keystroke, and can drop the stale one *without* dropping the rest of the render.
//!
//! # An absent echo means something different from a stale one
//!
//! A rendered value carrying no [`Seq`] at all is not an old answer — it is the extension
//! *setting* the value rather than echoing it: a "Clear" action, a programmatic reset, the
//! initial render. Those must win over local state, or a Clear button typed over would
//! never clear. So:
//!
//! | Rendered value | Meaning | Host applies it? |
//! |---|---|---|
//! | no `Seq` | the extension is setting the value | always |
//! | `Seq` == the latest local edit | a current answer | yes |
//! | `Seq` < the latest local edit | computed before the user's latest keystroke | no |
//! | `Seq` > the latest local edit | not reachable from this host | no, and it is logged |
//!
//! # What this module is and is not
//!
//! It is the *rule*, as a pure decision, which is all this crate is allowed to hold: no
//! transport, no clock, no task. The host owns an [`EchoTracker`], tells it when the user
//! edits, and asks it what to do with each rendered value. Nothing here sends anything.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::id::NodeId;

/// How many times a controlled input has been edited locally.
///
/// Minted by the host, never by an extension: an extension only ever echoes one back. It
/// is meaningful solely in comparison with another `Seq` for the *same* node, so there is
/// no arithmetic on it beyond ordering and no interpretation of the number itself.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Seq(u64);

impl Seq {
    /// The value before any edit.
    pub const ZERO: Seq = Seq(0);

    /// Builds a `Seq` from its raw value. Only useful when reading one back off the wire.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// The next `Seq` after this one.
    ///
    /// Saturating rather than wrapping. A wrap would make an ancient echo compare as
    /// current, which is the exact bug this type exists to prevent; saturating instead
    /// freezes the counter, which makes every subsequent echo compare equal and merely
    /// restores the un-counted behaviour. Reaching `u64::MAX` needs about 585 years of
    /// keystrokes at one per nanosecond, so this is a statement about which failure is
    /// acceptable, not a case anyone will hit.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "seq:{}", self.0)
    }
}

/// A value together with the input event it answers.
///
/// Used where a host wants to carry the two together; the view types keep them in
/// separate fields so that an absent echo stays representable on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EventCounted<T> {
    /// The value.
    pub value: T,
    /// The edit it answers.
    pub seq: Seq,
}

impl<T> EventCounted<T> {
    /// Pairs a value with the edit it answers.
    pub const fn new(value: T, seq: Seq) -> Self {
        Self { value, seq }
    }

    /// Applies `f` to the value, keeping the sequence.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> EventCounted<U> {
        EventCounted {
            value: f(self.value),
            seq: self.seq,
        }
    }
}

/// What the host should do with a rendered value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Echo {
    /// The extension is setting the value rather than answering an edit. Apply it.
    Authoritative,
    /// The value answers the user's most recent edit. Apply it.
    Current,
    /// The value was computed before the user's latest edit. Keep what is on screen.
    Stale {
        /// The most recent local edit.
        local: Seq,
        /// The edit the render answered.
        echoed: Seq,
    },
    /// The echo names an edit this host never made.
    ///
    /// Not reachable from a correct extension: a `Seq` originates here and is only ever
    /// echoed back. Reaching it means an extension invented one, or a render arrived for
    /// a node whose history the host has forgotten. Treated as stale, because applying a
    /// value the host cannot place is the worse of the two mistakes, and surfaced
    /// separately so it can be logged rather than silently absorbed.
    FromTheFuture {
        /// The most recent local edit.
        local: Seq,
        /// The edit the render claimed to answer.
        echoed: Seq,
    },
}

impl Echo {
    /// Whether the host should apply the rendered value.
    #[must_use]
    pub fn applies(self) -> bool {
        matches!(self, Echo::Authoritative | Echo::Current)
    }
}

/// Per-node edit counters, and the decision that uses them.
///
/// One of these belongs to the host, alongside whatever holds the on-screen values. It
/// stores a `u64` per controlled node that the user has actually edited, and nothing else.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EchoTracker {
    latest: BTreeMap<NodeId, Seq>,
}

impl EchoTracker {
    /// An empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that the user edited `node`, and returns the [`Seq`] to send with the
    /// change so the extension can echo it.
    pub fn edited(&mut self, node: NodeId) -> Seq {
        let seq = self.latest.entry(node).or_insert(Seq::ZERO);
        *seq = seq.next();
        *seq
    }

    /// The most recent edit to `node`, or [`Seq::ZERO`] if the user has never edited it.
    #[must_use]
    pub fn latest(&self, node: NodeId) -> Seq {
        self.latest.get(&node).copied().unwrap_or(Seq::ZERO)
    }

    /// What to do with a value rendered for `node`.
    ///
    /// `echoed` is `None` when the extension set the value rather than answering an edit.
    #[must_use]
    pub fn verdict(&self, node: NodeId, echoed: Option<Seq>) -> Echo {
        let Some(echoed) = echoed else {
            return Echo::Authoritative;
        };
        let local = self.latest(node);
        match echoed.cmp(&local) {
            std::cmp::Ordering::Equal => Echo::Current,
            std::cmp::Ordering::Less => Echo::Stale { local, echoed },
            std::cmp::Ordering::Greater => Echo::FromTheFuture { local, echoed },
        }
    }

    /// Drops `node`'s history.
    ///
    /// Call when a node leaves the tree. Keeping it would let a node that is removed and
    /// later re-created under the same id inherit a counter from its previous life, so
    /// that its first genuine echo would compare as stale.
    pub fn forget(&mut self, node: NodeId) {
        self.latest.remove(&node);
    }

    /// Drops the history of every node not in `live`.
    ///
    /// The bulk form of [`EchoTracker::forget`], for calling with the ids of a freshly
    /// rendered tree. Without it the map grows for the lifetime of the extension.
    pub fn retain_only(&mut self, live: &std::collections::BTreeSet<NodeId>) {
        self.latest.retain(|node, _| live.contains(node));
    }

    /// How many nodes are being tracked. Only the user has edited a node appear here.
    #[must_use]
    pub fn len(&self) -> usize {
        self.latest.len()
    }

    /// Whether nothing is being tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.latest.is_empty()
    }
}
