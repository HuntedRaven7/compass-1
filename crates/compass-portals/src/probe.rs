//! Capability probing.
//!
//! The rule this module exists to enforce: **availability is decided by
//! reading the interface's `version` property, never by asking whether
//! `org.freedesktop.portal.Desktop` is owned.** A portal frontend with no
//! backend for the interface owns the name and answers the bus, but exports
//! nothing — which is precisely the `xdg-desktop-portal-wlr` situation for
//! GlobalShortcuts. Checking the bus name would report that as working.
//!
//! Probing is also bounded: a frontend that owns the name and never answers
//! must produce [`NotQueryable::ProbeTimedOut`], not a hang.

use std::time::Duration;

use zbus::names::InterfaceName;

use crate::availability::{
    Availability, DESKTOP_DESTINATION, DESKTOP_PATH, NotQueryable, PortalInterface, Unavailable,
};

/// Read `interface.version` and classify the outcome.
pub(crate) async fn probe(
    conn: &zbus::Connection,
    interface: PortalInterface,
    timeout: Duration,
) -> Availability {
    let name = match InterfaceName::try_from(interface.name) {
        Ok(name) => name,
        Err(err) => {
            // Only reachable if a constant in `availability` is malformed.
            return Availability::NotQueryable {
                reason: NotQueryable::ProbeFailed {
                    message: err.to_string(),
                },
            };
        }
    };

    let proxy = match zbus::fdo::PropertiesProxy::builder(conn)
        .destination(DESKTOP_DESTINATION)
        .and_then(|b| b.path(DESKTOP_PATH))
    {
        Ok(builder) => match builder.build().await {
            Ok(proxy) => proxy,
            Err(err) => return probe_failed(err),
        },
        Err(err) => return probe_failed(err),
    };

    let raw = match tokio::time::timeout(timeout, proxy.get(name, "version")).await {
        Ok(result) => result,
        Err(_) => {
            tracing::debug!(
                interface = interface.name,
                ?timeout,
                "portal version probe timed out"
            );
            return Availability::NotQueryable {
                reason: NotQueryable::ProbeTimedOut { timeout },
            };
        }
    };

    let availability = match raw {
        Ok(value) => match u32::try_from(&value) {
            Ok(version) => classify_version(interface, version),
            Err(err) => Availability::NotQueryable {
                reason: NotQueryable::MalformedVersion {
                    message: format!("expected `u`, got `{}`: {err}", value.value_signature()),
                },
            },
        },
        Err(err) => classify_error(err),
    };

    tracing::debug!(
        interface = interface.name,
        availability = %availability,
        "probed portal interface"
    );
    availability
}

/// Classify a version we successfully read.
pub(crate) fn classify_version(interface: PortalInterface, version: u32) -> Availability {
    if version < interface.minimum_version {
        Availability::Unavailable {
            reason: Unavailable::VersionTooOld {
                found: version,
                required: interface.minimum_version,
            },
        }
    } else {
        if version > interface.newest_known_version {
            tracing::debug!(
                interface = interface.name,
                version,
                known = interface.newest_known_version,
                "portal interface is newer than this build knows about; using it anyway"
            );
        }
        Availability::Available { version }
    }
}

/// Classify a failed `Properties.Get`.
///
/// The interesting distinction is between "nobody is home" and "somebody is
/// home but does not implement this". Real `xdg-desktop-portal` is GDBus-based
/// and answers a `Get` for an unexported interface with `InvalidArgs`; a
/// `zbus` server answers with `UnknownInterface`. Both mean the same thing.
pub(crate) fn classify_error(err: zbus::fdo::Error) -> Availability {
    use zbus::fdo::Error as E;
    match err {
        E::ServiceUnknown(_) | E::NameHasNoOwner(_) => Availability::Unavailable {
            reason: Unavailable::NoPortalFrontend,
        },
        E::UnknownInterface(_)
        | E::InvalidArgs(_)
        | E::UnknownProperty(_)
        | E::UnknownObject(_)
        | E::UnknownMethod(_) => Availability::Unavailable {
            reason: Unavailable::InterfaceMissing,
        },
        E::NoReply(_) | E::Timeout(_) | E::TimedOut(_) => Availability::NotQueryable {
            reason: NotQueryable::ProbeTimedOut {
                timeout: Duration::ZERO,
            },
        },
        other => Availability::NotQueryable {
            reason: NotQueryable::ProbeFailed {
                message: other.to_string(),
            },
        },
    }
}

fn probe_failed(err: zbus::Error) -> Availability {
    Availability::NotQueryable {
        reason: NotQueryable::ProbeFailed {
            message: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::availability::GLOBAL_SHORTCUTS;

    #[test]
    fn a_supported_version_is_available() {
        assert_eq!(
            classify_version(GLOBAL_SHORTCUTS, 1),
            Availability::Available { version: 1 }
        );
    }

    #[test]
    fn a_newer_version_than_we_know_is_still_available() {
        let availability = classify_version(GLOBAL_SHORTCUTS, 99);
        assert_eq!(availability, Availability::Available { version: 99 });
        assert!(!GLOBAL_SHORTCUTS.is_known_version(99));
    }

    #[test]
    fn a_version_below_the_floor_is_unavailable_not_absent() {
        assert_eq!(
            classify_version(GLOBAL_SHORTCUTS, 0),
            Availability::Unavailable {
                reason: Unavailable::VersionTooOld {
                    found: 0,
                    required: 1
                }
            }
        );
    }

    #[test]
    fn an_unowned_bus_name_means_no_frontend() {
        assert_eq!(
            classify_error(zbus::fdo::Error::ServiceUnknown("nope".into())),
            Availability::Unavailable {
                reason: Unavailable::NoPortalFrontend
            }
        );
    }

    #[test]
    fn an_unexported_interface_is_distinguishable_from_no_frontend() {
        for err in [
            zbus::fdo::Error::UnknownInterface("nope".into()),
            zbus::fdo::Error::InvalidArgs("No such interface".into()),
        ] {
            assert_eq!(
                classify_error(err),
                Availability::Unavailable {
                    reason: Unavailable::InterfaceMissing
                }
            );
        }
    }

    #[test]
    fn an_uninterpretable_failure_is_not_an_absence() {
        let availability = classify_error(zbus::fdo::Error::AccessDenied("nope".into()));
        assert!(availability.is_indeterminate());
        assert!(!availability.is_available());
    }
}
