//! `compass-script` — Rhai scripts as a third extension tier, in-process.
//!
//! A forty-line `.rhai` file in a folder gets a filterable list view with
//! actions, at the cost of parsing an AST rather than spawning Node. The tier
//! is a second front end onto `compass-extension-api`, not a parallel stack:
//! scripts produce the same [`View`](compass_extension_api::View) the
//! TypeScript tier produces, their actions are dispatched through the same
//! [`ActionRequest`](compass_extension_api::ActionRequest) /
//! [`ActionResponse`](compass_extension_api::ActionResponse) pair, and what
//! they may reach is decided by the same
//! [`CapabilityRegistry`](compass_extension_api::CapabilityRegistry).
//!
//! # The sandbox, by construction
//!
//! Rhai's language has no I/O. The engine here starts from
//! `Engine::new_raw()` — no functions at all — and adds pure packages, a few
//! pure helpers (JSON, time, randomness), and then one function per
//! *granted* capability. A capability that is not granted has no function, so
//! a script cannot call it, try it, or probe for it. On top of that:
//!
//! * `import` resolves nothing: a dummy resolver and a module limit of zero;
//! * `eval` is disabled as a symbol, and `sleep` is not registered;
//! * every `set_max_*` limit is set from [`Limits`];
//! * an operation budget is enforced from `on_progress`, which also checks the
//!   wall-clock deadline, so a runaway script is *terminated*, not merely
//!   abandoned;
//! * every call runs on Tokio's blocking pool with a timeout, so no script can
//!   stall the caller.
//!
//! The negative tests in `tests/sandbox.rs` are the specification of all of
//! the above (PLAN §8.2).
//!
//! # Layout
//!
//! * [`manifest`] — `script.toml`: title, icon, capabilities, entry file.
//! * [`discovery`] — XDG search paths and shadowing, like Node extensions.
//! * [`ScriptInstance`] — compile, `search(query)` → [`ViewTree`](compass_extension_api::ViewTree), invoke actions.
//! * [`ScriptHost`] — the services a granted capability reaches.
//! * [`watch`] — hot reload.
//!
//! Authoring documentation lives in `docs/rust-engine/RHAI-SCRIPTS.md`, and
//! first-party examples in `extensions/rhai-examples/`.

pub mod discovery;
mod engine;
pub mod error;
pub mod host;
mod instance;
pub mod limits;
pub mod manifest;
mod render;
pub mod watch;

pub use discovery::{DiscoveredScript, Scan};
pub use error::{LimitKind, ManifestError, ScriptError};
pub use host::{HostCall, HostError, MemoryHost, ProcessOutcome, ScriptHost};
pub use instance::{SEARCH_FN, ScriptInstance};
pub use limits::Limits;
pub use manifest::ScriptManifest;
pub use watch::{Reload, ScriptSet, ScriptWatcher};
