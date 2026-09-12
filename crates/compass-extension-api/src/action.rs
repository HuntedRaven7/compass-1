//! Actions, action panels and keyboard shortcuts.
//!
//! An action is a leaf of the view tree that the user can invoke. It carries two
//! identities and they are deliberately different:
//!
//! * [`NodeId`] — assigned by the host when the tree is built, used for diffing and for
//!   telling the UI which row to highlight.
//! * [`HandlerId`] — an opaque token minted by the *extension*. The host never
//!   interprets it; it only hands it back when the action is invoked. That is what keeps
//!   this crate free of any assumption about how the other side stores its callbacks.

use serde::{Deserialize, Serialize};

use crate::id::NodeId;
use crate::view::Image;

/// An opaque callback token minted by an extension and echoed back on invocation.
///
/// The host must treat it as a bag of bytes: no parsing, no structure, no meaning.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HandlerId(pub String);

impl HandlerId {
    /// Wraps a token.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The token as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HandlerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A keyboard modifier.
///
/// Closed set: the extension surface has exactly these, and a modifier the host does not
/// understand is not something it can usefully render, so a new one is a breaking change
/// on both sides anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyModifier {
    /// The platform command/super key.
    Cmd,
    /// Control.
    Ctrl,
    /// Alt / option.
    Alt,
    /// Shift.
    Shift,
    /// Meta, where the platform distinguishes it from `Cmd`.
    Meta,
}

/// The non-modifier half of a shortcut.
///
/// An open newtype rather than a seventy-variant enum: the key vocabulary grows (media
/// keys, locale-specific punctuation) and an older host that receives a key it cannot
/// bind should ignore the shortcut, not fail to parse the whole view.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyEquivalent(pub String);

impl KeyEquivalent {
    /// Wraps a key name.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A keyboard shortcut bound to an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    /// Modifiers, order-insensitive but stored as given.
    #[serde(default)]
    pub modifiers: Vec<KeyModifier>,
    /// The key itself.
    pub key: KeyEquivalent,
}

impl Shortcut {
    /// A shortcut with no modifiers.
    pub fn bare(key: impl Into<String>) -> Self {
        Self {
            modifiers: Vec::new(),
            key: KeyEquivalent::new(key),
        }
    }

    /// A shortcut with modifiers.
    pub fn new(modifiers: impl IntoIterator<Item = KeyModifier>, key: impl Into<String>) -> Self {
        Self {
            modifiers: modifiers.into_iter().collect(),
            key: KeyEquivalent::new(key),
        }
    }
}

/// How prominently the UI should present an action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStyle {
    /// Ordinary.
    #[default]
    Regular,
    /// Destructive; the UI is expected to mark it.
    Destructive,
}

/// A single invocable action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    /// Derived identity. Assigned by [`crate::ViewTree`]; ignore whatever is here before that.
    #[serde(default)]
    pub id: NodeId,
    /// Author-supplied stable key, if any. Feeds id derivation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Title shown in the panel.
    pub title: String,
    /// Optional icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Image>,
    /// Optional keyboard shortcut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<Shortcut>,
    /// Presentation hint.
    #[serde(default)]
    pub style: ActionStyle,
    /// The token handed back to the extension when this action fires.
    pub handler: HandlerId,
}

impl Action {
    /// A minimal action.
    pub fn new(title: impl Into<String>, handler: impl Into<String>) -> Self {
        Self {
            id: NodeId::ROOT,
            key: None,
            title: title.into(),
            icon: None,
            shortcut: None,
            style: ActionStyle::Regular,
            handler: HandlerId::new(handler),
        }
    }

    /// Builder: attach a shortcut.
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Builder: attach a stable key.
    #[must_use]
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
}

/// A nested panel of actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSubmenu {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Author-supplied stable key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Title of the submenu entry.
    pub title: String,
    /// Optional icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Image>,
    /// Optional shortcut that opens the submenu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<Shortcut>,
    /// Fired when the submenu is opened, so the extension can populate it lazily.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_open: Option<HandlerId>,
    /// Children.
    #[serde(default)]
    pub items: Vec<ActionItem>,
}

/// An entry in an action panel section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionItem {
    /// A leaf action.
    Action(Action),
    /// A nested panel.
    Submenu(ActionSubmenu),
}

/// A titled group within an action panel.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActionSection {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Optional section title. `None` is the implicit, untitled section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Entries.
    #[serde(default)]
    pub items: Vec<ActionItem>,
}

/// The set of actions attached to a view, an item or an empty state.
///
/// Loose actions are normalised into a single untitled section, so a host renders exactly
/// one shape.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActionPanel {
    /// Derived identity.
    #[serde(default)]
    pub id: NodeId,
    /// Optional panel title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Sections, in order.
    #[serde(default)]
    pub sections: Vec<ActionSection>,
}

impl ActionPanel {
    /// A panel holding one untitled section of actions.
    pub fn of(actions: impl IntoIterator<Item = Action>) -> Self {
        Self {
            id: NodeId::ROOT,
            title: None,
            sections: vec![ActionSection {
                id: NodeId::ROOT,
                title: None,
                items: actions.into_iter().map(ActionItem::Action).collect(),
            }],
        }
    }

    /// Every action reachable from this panel, submenus included, in render order.
    pub fn actions(&self) -> Vec<&Action> {
        let mut out = Vec::new();
        for section in &self.sections {
            collect_actions(&section.items, &mut out);
        }
        out
    }
}

fn collect_actions<'a>(items: &'a [ActionItem], out: &mut Vec<&'a Action>) {
    for item in items {
        match item {
            ActionItem::Action(a) => out.push(a),
            ActionItem::Submenu(s) => collect_actions(&s.items, out),
        }
    }
}
