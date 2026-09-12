//! Which engine implementation a `vicinae` invocation is aimed at.
//!
//! `docs/rust-engine/PLAN.md` §5 makes the two engines selectable side by side
//! for the whole migration:
//!
//! ```text
//! vicinae --engine=cpp     # the shipping C++ engine
//! vicinae --engine=rust    # this one
//! COMPASS_ENGINE=rust      # env override for CI and dogfooding
//! ```
//!
//! # Why the default here is `rust`, not `cpp`
//!
//! PLAN §5 says the *shipping* default stays `cpp` until Phase 7. That default
//! belongs to the front-end binary that can actually exec either engine. This
//! binary is the Rust engine's own CLI: it has no C++ engine to hand off to,
//! and dispatching to one is explicitly out of scope for Phase 2. Defaulting to
//! `cpp` here would make every invocation fail by default, which teaches
//! nobody anything.
//!
//! So: the flag parses, `COMPASS_ENGINE` overrides it, `vicinae doctor` reports
//! it, and asking for [`Engine::Cpp`] on a command that would need to be
//! dispatched fails with an explanation rather than silently doing the Rust
//! thing. When the dispatching front-end lands, it owns the `cpp` default.

use std::fmt;

/// Engine implementation selected for this invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Engine {
    /// The legacy C++ engine in `src/`. This binary cannot dispatch to it.
    Cpp,
    /// The Rust engine: this binary and the `compass-*` crates.
    #[default]
    Rust,
}

impl Engine {
    /// The value as it is spelled on the command line and in `COMPASS_ENGINE`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpp => "cpp",
            Self::Rust => "rust",
        }
    }

    /// Whether this binary can serve requests for the selected engine itself.
    ///
    /// Always false for [`Engine::Cpp`]: see the [module docs](self).
    #[must_use]
    pub fn is_served_by_this_binary(self) -> bool {
        matches!(self, Self::Rust)
    }
}

impl fmt::Display for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;

    #[test]
    fn default_is_rust() {
        assert_eq!(Engine::default(), Engine::Rust);
    }

    #[test]
    fn spellings_round_trip() {
        for engine in [Engine::Cpp, Engine::Rust] {
            let parsed = Engine::from_str(engine.as_str(), true).expect("known spelling");
            assert_eq!(parsed, engine);
            assert_eq!(engine.to_string(), engine.as_str());
        }
    }

    #[test]
    fn only_rust_is_served_here() {
        assert!(Engine::Rust.is_served_by_this_binary());
        assert!(!Engine::Cpp.is_served_by_this_binary());
    }

    #[test]
    fn unknown_spelling_is_rejected() {
        assert!(Engine::from_str("go", true).is_err());
    }
}
