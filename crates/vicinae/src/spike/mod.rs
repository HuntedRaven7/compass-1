//! One-off experiments that answer a question the code cannot answer about
//! itself.
//!
//! A spike is not a feature and not a test. It exists because some questions
//! about a desktop can only be answered by asking a running desktop, and until
//! they are answered a design cannot honestly be finished. `docs/rust-engine`
//! issue #3 names two, and this module implements both.
//!
//! Three rules they share, each learned the hard way:
//!
//! - **Every outcome is a finding.** A spike exits zero whatever it discovers;
//!   "the portal refused" and "the kernel has no Landlock" are answers, and a
//!   non-zero exit would make the harness treat them as broken runs.
//! - **Every assertion has a control.** A boundary that denies everything looks
//!   identical to one that works until you check that the allowed case still
//!   passes, and a sandbox that confines nothing looks identical to one that
//!   does until you check that the forbidden case fails.
//! - **They are addressed to whoever is answering the question** — CI, or a
//!   person on a real machine — not to users. Hence `#[command(hide = true)]`,
//!   and hence each should be deleted or folded into a real subsystem once its
//!   question has an answer.

pub mod sandbox;
pub mod shortcut;
