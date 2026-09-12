//! The seam every Compass extension tier sits behind.
//!
//! Compass will run extensions written in more than one language, executed in more than
//! one way. This crate is the part they share, and it is defined as much by what it
//! refuses to contain as by what it holds:
//!
//! * **no transport** — nothing here sends, receives, frames, encodes or connects;
//! * **no execution** — nothing spawns, schedules, sandboxes or awaits;
//! * **no host** — this crate depends on `serde`, `serde_json`, `thiserror` and
//!   `tracing`, and on no crate belonging to Compass. A test enforces exactly that, and
//!   it is the acceptance criterion for the seam, not a formality.
//!
//! What is left is pure data and pure decisions, in three parts.
//!
//! ## The view tree
//!
//! [`ViewTree`] is the *rendered result* an extension produces — a list, grid, detail or
//! form, already reconciled and normalised — not a component model. Every node carries a
//! [`NodeId`] derived from its parent, its slot and either its author-supplied key or its
//! ordinal position, so the same logical tree always yields the same ids and a front end
//! can diff ([`ViewDiff`]) instead of rebuilding. See [`id`] for exactly how identity is
//! computed and what it costs.
//!
//! ## Capabilities
//!
//! [`CapabilityRegistry`] answers one question — may this extension do this thing? —
//! from declarations and grants, never from ambient authority. A refusal is a [`Denial`]
//! value naming the exact capability, not a panic and not a string. See [`capability`].
//!
//! ## Controlled inputs
//!
//! An extension that owns an input's value must be able to tell its own stale answer from
//! a current one, or a slow render silently overwrites what the user has typed since. The
//! [`input`] module holds that rule — an edit counter per node and the decision that uses
//! it — as pure data, the same way capabilities are.
//!
//! ## Action dispatch
//!
//! An extension mints opaque [`HandlerId`] tokens for its actions; the UI reports one
//! back; [`ActionIndex`] resolves it and [`Pending`] tracks it until an
//! [`ActionResponse`] arrives — or until the extension disappears and every outstanding
//! invocation is turned into a reportable failure. See [`dispatch`].

#![deny(missing_docs)]
#![deny(missing_debug_implementations)]

pub mod action;
pub mod capability;
pub mod diff;
pub mod dispatch;
pub mod id;
pub mod input;
pub mod tree;
pub mod view;

pub use action::{
    Action, ActionItem, ActionPanel, ActionSection, ActionStyle, ActionSubmenu, HandlerId,
    KeyEquivalent, KeyModifier, Shortcut,
};
pub use capability::{
    Capability, CapabilityGrant, CapabilityRegistry, CapabilityState, Denial, DenialReason,
    ExtensionId,
};
pub use diff::{NodeChange, ViewDiff};
pub use dispatch::{
    ActionEffect, ActionIndex, ActionPayload, ActionRequest, ActionResponse, DispatchError,
    InvocationId, InvocationSource, Pending, Stage, ToastStyle,
};
pub use id::{NodeId, NodeKey};
pub use input::{Echo, EchoTracker, EventCounted, Seq};
pub use tree::{ActionRef, NodeSummary, ViewTree};
pub use view::{
    Accessory, AspectRatio, Color, DatePrecision, Detail, Dropdown, DropdownOption,
    DropdownSection, EmptyState, FieldKind, FieldValue, FormField, FormItem, FormView, GridContent,
    GridFit, GridInset, GridItem, GridSection, GridView, Image, ImageMask, ImageSource, ListItem,
    ListSection, ListView, MetadataItem, MetadataTag, Pagination, SearchBar, View,
};
