//! Capability detection: is the helper extension there, and does it speak a
//! contract version we understand?
//!
//! Nothing here ever hard-fails. App search, launch, calculator, emoji,
//! snippets and file search must all work with zero extension installed
//! (PLAN.md 3.5.1), so a missing extension is modelled as data, not as an
//! error.

use crate::contract::CONTRACT_VERSION;

/// Whether one capability of the shell extension is usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Present and speaking a version we support.
    Available {
        /// Contract version the extension reported.
        version: u32,
    },
    /// Nothing is exported at the contract's object path. Either the extension
    /// is not installed, or it is installed but disabled, or `gnome-shell` is
    /// not running.
    Absent,
    /// The interface is there, but at a contract version we do not speak.
    VersionMismatch {
        /// Version the extension reported.
        found: u32,
        /// Version this build of Compass speaks.
        expected: u32,
    },
}

impl Availability {
    /// Classify a `Version` property read.
    pub fn from_probe(result: std::result::Result<u32, zbus::Error>) -> Self {
        match result {
            Ok(version) if version == CONTRACT_VERSION => Self::Available { version },
            Ok(found) => Self::VersionMismatch {
                found,
                expected: CONTRACT_VERSION,
            },
            Err(err) => {
                tracing::debug!(%err, "shell extension probe failed; treating as absent");
                Self::Absent
            }
        }
    }

    /// True only when calls against this capability are worth attempting.
    pub fn is_available(self) -> bool {
        matches!(self, Self::Available { .. })
    }

    /// The contract version in use, when there is one.
    pub fn version(self) -> Option<u32> {
        match self {
            Self::Available { version } => Some(version),
            _ => None,
        }
    }
}

impl std::fmt::Display for Availability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Available { version } => write!(f, "available (contract v{version})"),
            Self::Absent => write!(f, "absent"),
            Self::VersionMismatch { found, expected } => write!(
                f,
                "version mismatch (extension speaks v{found}, this build speaks v{expected})"
            ),
        }
    }
}

/// What the shell extension can currently do for us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellCapabilities {
    /// The windows interface.
    pub windows: Availability,
    /// The clipboard interface.
    pub clipboard: Availability,
}

impl ShellCapabilities {
    /// The "no extension at all" state, which is also the pre-probe state.
    pub fn absent() -> Self {
        Self {
            windows: Availability::Absent,
            clipboard: Availability::Absent,
        }
    }

    /// True when at least one capability is usable.
    pub fn any_available(self) -> bool {
        self.windows.is_available() || self.clipboard.is_available()
    }

    /// Product features that are unusable in this state.
    ///
    /// This is the list `vicinae doctor` reports (PLAN.md 3.5.4).
    pub fn degraded(self) -> Vec<DegradedFeature> {
        let mut out = Vec::new();
        if !self.windows.is_available() {
            out.push(DegradedFeature::WindowSwitching);
        }
        if !self.clipboard.is_available() {
            out.push(DegradedFeature::ClipboardHistory);
            out.push(DegradedFeature::Paste);
        }
        out
    }
}

impl Default for ShellCapabilities {
    fn default() -> Self {
        Self::absent()
    }
}

/// A product feature lost when part of the contract is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DegradedFeature {
    /// Listing, switching to and closing other applications' windows.
    WindowSwitching,
    /// Recording a history of copied items.
    ClipboardHistory,
    /// Pasting into the previously focused window.
    Paste,
}

impl DegradedFeature {
    /// Short label.
    pub fn title(self) -> &'static str {
        match self {
            Self::WindowSwitching => "Window switching",
            Self::ClipboardHistory => "Clipboard history",
            Self::Paste => "Paste into the focused window",
        }
    }

    /// One-line explanation of what the user actually loses, phrased for
    /// `vicinae doctor`.
    pub fn explanation(self) -> &'static str {
        match self {
            Self::WindowSwitching => {
                "Compass cannot list, focus or close other windows. Mutter implements neither \
                 ext-foreign-toplevel-list-v1 nor wlr-foreign-toplevel-management, and \
                 org.gnome.Shell.Introspect.GetWindows is allowlisted to the portal backends, so \
                 the helper extension is the only source of a window list on GNOME."
            }
            Self::ClipboardHistory => {
                "Compass cannot observe copies, so clipboard history records nothing while the \
                 extension is unavailable. Mutter implements neither wlr-data-control nor \
                 ext-data-control, so there is no protocol-level fallback."
            }
            Self::Paste => {
                "Compass can put an item back on the clipboard for pasting only through the \
                 extension. Note that contract v1 has no keystroke-synthesis method, so the \
                 final Ctrl+V still has to come from the user; on Bluefin /dev/uinput is \
                 unavailable, so wtype/dotool cannot supply it either."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_version_is_available() {
        let availability = Availability::from_probe(Ok(CONTRACT_VERSION));
        assert_eq!(
            availability,
            Availability::Available {
                version: CONTRACT_VERSION
            }
        );
        assert!(availability.is_available());
        assert_eq!(availability.version(), Some(CONTRACT_VERSION));
    }

    #[test]
    fn other_versions_are_a_mismatch_not_an_absence() {
        let availability = Availability::from_probe(Ok(CONTRACT_VERSION + 1));
        assert!(matches!(availability, Availability::VersionMismatch { .. }));
        assert_ne!(availability, Availability::Absent);
        assert!(!availability.is_available());
        assert_eq!(availability.version(), None);
    }

    #[test]
    fn probe_errors_are_an_absence() {
        let availability = Availability::from_probe(Err(zbus::Error::Unsupported));
        assert_eq!(availability, Availability::Absent);
    }

    #[test]
    fn degradation_is_reported_per_capability() {
        let both = ShellCapabilities::absent();
        assert_eq!(
            both.degraded(),
            vec![
                DegradedFeature::WindowSwitching,
                DegradedFeature::ClipboardHistory,
                DegradedFeature::Paste
            ]
        );
        assert!(!both.any_available());

        let clipboard_only = ShellCapabilities {
            windows: Availability::Absent,
            clipboard: Availability::Available {
                version: CONTRACT_VERSION,
            },
        };
        assert_eq!(
            clipboard_only.degraded(),
            vec![DegradedFeature::WindowSwitching]
        );
        assert!(clipboard_only.any_available());

        let healthy = ShellCapabilities {
            windows: Availability::Available {
                version: CONTRACT_VERSION,
            },
            clipboard: Availability::Available {
                version: CONTRACT_VERSION,
            },
        };
        assert!(healthy.degraded().is_empty());
    }

    #[test]
    fn every_degraded_feature_explains_itself() {
        for feature in ShellCapabilities::absent().degraded() {
            assert!(!feature.title().is_empty());
            assert!(feature.explanation().len() > 40, "{feature:?}");
        }
    }
}
