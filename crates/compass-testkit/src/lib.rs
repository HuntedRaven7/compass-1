//! Shared test fixtures and corpora for the Rust engine.
//!
//! This crate is a test dependency only: it is never linked into the shipped binary. It exists so
//! that corpora live in one place and every crate asserts against the same fixtures.
//!
//! See `docs/rust-engine/PLAN.md` §8.1 for what the corpora are for.

pub mod corpus;
