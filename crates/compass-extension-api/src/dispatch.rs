//! Action dispatch: the UI says an action fired, the host routes it, the extension
//! answers.
//!
//! The round trip is modelled as three values — [`ActionRequest`], [`ActionResponse`] and
//! [`DispatchError`] — plus two pieces of state a host needs and would otherwise
//! reinvent per tier: [`ActionIndex`], which is what makes "unknown action id" an
//! answerable question, and [`Pending`], which is what makes "the extension went away
//! mid-action" an answerable one.
//!
//! Nothing here moves bytes. Producing an [`ActionRequest`] and consuming an
//! [`ActionResponse`] is the whole contract; how the two halves reach each other is the
//! caller's problem, and deliberately invisible from this crate.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::action::{HandlerId, Shortcut};
use crate::capability::{Capability, Denial, ExtensionId};
use crate::id::NodeId;
use crate::input::Seq;
use crate::tree::{ActionRef, ViewTree};
use crate::view::FieldValue;

/// Identifies one in-flight invocation. Monotonic per [`Pending`]; never reused while the
/// invocation is outstanding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InvocationId(pub u64);

impl fmt::Display for InvocationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// How the user reached the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationSource {
    /// The primary action of the selected row, fired by confirming it.
    #[default]
    Primary,
    /// Chosen from the action panel.
    Panel,
    /// Triggered by its keyboard shortcut.
    Shortcut,
}

/// Extra data the UI collected before firing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "payload", content = "value", rename_all = "snake_case")]
pub enum ActionPayload {
    /// Nothing to send.
    #[default]
    None,
    /// A form was submitted; values keyed by field name.
    FormValues(BTreeMap<String, FieldValue>),
    /// The current search text at the moment of firing.
    SearchText(String),
    /// The selected item, when the action is view-level but acts on a selection.
    Selection(NodeId),
    /// A controlled input changed.
    ///
    /// Distinct from [`ActionPayload::SearchText`] and [`ActionPayload::FormValues`],
    /// which report state at the moment an action fired. This reports an *edit*, and
    /// carries the [`Seq`] the extension must echo back in the value it renders — without
    /// it the host cannot tell a current answer from one computed two keystrokes ago. See
    /// [`crate::input`].
    InputChanged {
        /// The input that changed.
        node: NodeId,
        /// Its new value.
        value: FieldValue,
        /// The edit being reported.
        seq: Seq,
    },
}

/// One action, on its way to the extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRequest {
    /// Correlates this request with its response.
    pub invocation: InvocationId,
    /// Whose action it is.
    pub extension: ExtensionId,
    /// The token the extension minted, echoed back verbatim.
    pub handler: HandlerId,
    /// The action node in the tree, for logging and for highlighting.
    pub node: NodeId,
    /// How the user got here.
    pub source: InvocationSource,
    /// Collected data.
    #[serde(default)]
    pub payload: ActionPayload,
}

/// What the host should do once an action completes.
///
/// Every variant corresponds to something the host already offers extensions; there is no
/// effect here that a front end cannot carry out on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum ActionEffect {
    /// A new tree is coming; repaint when it arrives.
    Rerender,
    /// Push a view onto the navigation stack.
    PushView,
    /// Pop one view.
    PopView,
    /// Return to the root view.
    PopToRoot,
    /// Close the launcher window.
    CloseWindow,
    /// Show a transient message attached to the current view.
    Toast {
        /// Presentation.
        style: ToastStyle,
        /// Headline.
        title: String,
        /// Optional body.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Show a heads-up display message and close the window.
    Hud {
        /// The text.
        text: String,
    },
}

/// How a toast is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToastStyle {
    /// Neutral.
    #[default]
    Info,
    /// In progress.
    Animated,
    /// Succeeded.
    Success,
    /// Failed.
    Failure,
}

/// Everything that can go wrong routing an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum DispatchError {
    /// The UI reported a handler that is not in the current tree. Normal, not
    /// exceptional: it happens whenever a re-render lands between the keypress and the
    /// dispatch, and the correct response is to drop the invocation.
    #[error("no action with handler `{handler}` in the current view")]
    UnknownAction {
        /// The token that could not be resolved.
        handler: HandlerId,
    },
    /// The invocation id does not correspond to anything outstanding.
    #[error("invocation {invocation} is not in flight")]
    UnknownInvocation {
        /// The id.
        invocation: InvocationId,
    },
    /// The extension stopped existing between dispatch and response — it exited, was
    /// disabled, was uninstalled, or was killed for exceeding a limit. The user chose an
    /// action and will get no result, so this must be reportable, not swallowed.
    #[error("extension `{extension}` is no longer available ({stage})")]
    ExtensionGone {
        /// Which extension.
        extension: ExtensionId,
        /// How far the invocation got before it was lost.
        stage: Stage,
    },
    /// The action needed a capability the extension does not hold.
    #[error("{denial}")]
    CapabilityDenied {
        /// The denial, naming the exact missing capability.
        denial: Denial,
    },
    /// The extension ran the action and reported failure.
    #[error("extension `{extension}` failed to run the action: {message}")]
    Failed {
        /// Which extension.
        extension: ExtensionId,
        /// The extension's own message.
        message: String,
    },
    /// The payload did not match what the action expected.
    #[error("invalid payload for action: {detail}")]
    InvalidPayload {
        /// What was wrong.
        detail: String,
    },
}

impl DispatchError {
    /// The missing capability, when this error is a denial. Lets a caller render a
    /// precise "grant X to continue" prompt without matching on the variant.
    #[must_use]
    pub fn missing_capability(&self) -> Option<&Capability> {
        match self {
            DispatchError::CapabilityDenied { denial } => Some(&denial.capability),
            _ => None,
        }
    }
}

impl From<Denial> for DispatchError {
    fn from(denial: Denial) -> Self {
        DispatchError::CapabilityDenied { denial }
    }
}

/// How far an invocation got before it was lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Never handed over.
    BeforeDispatch,
    /// Handed over, no response yet. The action may or may not have had an effect, and
    /// the host cannot tell which — the honest thing is to say so.
    InFlight,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Stage::BeforeDispatch => "before dispatch",
            Stage::InFlight => "in flight",
        })
    }
}

/// The extension's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ActionResponse {
    /// The action ran.
    Completed {
        /// Which invocation.
        invocation: InvocationId,
        /// What the host should do next.
        #[serde(default)]
        effects: Vec<ActionEffect>,
    },
    /// The action did not run, or its outcome is unknown.
    Failed {
        /// Which invocation.
        invocation: InvocationId,
        /// Why.
        error: DispatchError,
    },
}

impl ActionResponse {
    /// The invocation this answers.
    #[must_use]
    pub fn invocation(&self) -> InvocationId {
        match self {
            ActionResponse::Completed { invocation, .. }
            | ActionResponse::Failed { invocation, .. } => *invocation,
        }
    }
}

/// Every action in a rendered tree, keyed by the token the UI will report.
///
/// Rebuild this whenever a new tree is rendered. Resolving against a stale index is how
/// an action fires on a row the user is no longer looking at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionIndex {
    by_handler: BTreeMap<HandlerId, ActionRef>,
    by_shortcut: BTreeMap<String, HandlerId>,
}

impl ActionIndex {
    /// Indexes a tree.
    ///
    /// Duplicate handler tokens keep the first occurrence in render order, so a panel
    /// that reuses a token cannot shadow the row-level action above it.
    #[must_use]
    pub fn from_tree(tree: &ViewTree) -> Self {
        let mut by_handler = BTreeMap::new();
        let mut by_shortcut = BTreeMap::new();
        for action in tree.actions() {
            if let Some(sc) = &action.shortcut {
                by_shortcut
                    .entry(shortcut_key(sc))
                    .or_insert_with(|| action.handler.clone());
            }
            by_handler.entry(action.handler.clone()).or_insert(action);
        }
        Self {
            by_handler,
            by_shortcut,
        }
    }

    /// Number of distinct actions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_handler.len()
    }

    /// Whether the tree has no actions at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_handler.is_empty()
    }

    /// Looks an action up by the token the UI reported.
    pub fn resolve(&self, handler: &HandlerId) -> Result<&ActionRef, DispatchError> {
        self.by_handler
            .get(handler)
            .ok_or_else(|| DispatchError::UnknownAction {
                handler: handler.clone(),
            })
    }

    /// Looks an action up by keystroke. Actions without a shortcut are simply not here,
    /// which is the difference between "no such binding" and "no such action".
    pub fn resolve_shortcut(&self, shortcut: &Shortcut) -> Option<&ActionRef> {
        self.by_shortcut
            .get(&shortcut_key(shortcut))
            .and_then(|h| self.by_handler.get(h))
    }

    /// Every action, ordered by token.
    pub fn iter(&self) -> impl Iterator<Item = &ActionRef> {
        self.by_handler.values()
    }
}

/// Canonical form of a shortcut: modifiers sorted and de-duplicated, so `cmd+shift+k`
/// and `shift+cmd+k` are the same binding.
fn shortcut_key(shortcut: &Shortcut) -> String {
    let mut mods: Vec<String> = shortcut
        .modifiers
        .iter()
        .map(|m| format!("{m:?}").to_lowercase())
        .collect();
    mods.sort();
    mods.dedup();
    mods.push(shortcut.key.as_str().to_lowercase());
    mods.join("+")
}

/// Invocations handed to an extension and not yet answered.
///
/// This is what turns "the extension went away" from a hang into a value: call
/// [`Pending::extension_gone`] and every outstanding invocation for that extension comes
/// back as a [`ActionResponse::Failed`] the UI can show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pending {
    next: u64,
    in_flight: BTreeMap<InvocationId, ActionRequest>,
}

impl Pending {
    /// An empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves a reported token against `index` and records the resulting invocation.
    ///
    /// Returns [`DispatchError::UnknownAction`] when the token is not in the current
    /// tree — the one case a host must handle on every keypress.
    pub fn begin(
        &mut self,
        index: &ActionIndex,
        extension: &ExtensionId,
        handler: &HandlerId,
        source: InvocationSource,
        payload: ActionPayload,
    ) -> Result<ActionRequest, DispatchError> {
        let action = index.resolve(handler)?;
        self.next += 1;
        let request = ActionRequest {
            invocation: InvocationId(self.next),
            extension: extension.clone(),
            handler: action.handler.clone(),
            node: action.node,
            source,
            payload,
        };
        self.in_flight.insert(request.invocation, request.clone());
        Ok(request)
    }

    /// Accepts a response, clearing the invocation.
    ///
    /// A response for an unknown invocation is rejected rather than ignored: it means the
    /// two sides disagree about what is outstanding.
    pub fn complete(&mut self, response: &ActionResponse) -> Result<ActionRequest, DispatchError> {
        let invocation = response.invocation();
        self.in_flight
            .remove(&invocation)
            .ok_or(DispatchError::UnknownInvocation { invocation })
    }

    /// Abandons every invocation belonging to an extension that has gone away, returning
    /// one failure per invocation in the order they were begun.
    pub fn extension_gone(&mut self, extension: &ExtensionId) -> Vec<ActionResponse> {
        let lost: Vec<InvocationId> = self
            .in_flight
            .iter()
            .filter(|(_, r)| &r.extension == extension)
            .map(|(id, _)| *id)
            .collect();
        lost.into_iter()
            .map(|invocation| {
                self.in_flight.remove(&invocation);
                ActionResponse::Failed {
                    invocation,
                    error: DispatchError::ExtensionGone {
                        extension: extension.clone(),
                        stage: Stage::InFlight,
                    },
                }
            })
            .collect()
    }

    /// How many invocations are outstanding.
    #[must_use]
    pub fn len(&self) -> usize {
        self.in_flight.len()
    }

    /// Whether nothing is outstanding.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.in_flight.is_empty()
    }

    /// Looks up an outstanding invocation.
    #[must_use]
    pub fn get(&self, invocation: InvocationId) -> Option<&ActionRequest> {
        self.in_flight.get(&invocation)
    }
}
