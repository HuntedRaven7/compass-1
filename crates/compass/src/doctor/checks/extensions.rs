//! Whether the extensions `compass.json` declares are installed.
//!
//! Part of [`super`]; see that module for the purity rule. The plan is
//! `compass_core::declared_extensions::plan` against what is installed,
//! worked out by the caller without the network, so this says what is
//! missing but not why an install failed; `compass ext sync` says that.

use compass_core::declared_extensions::Step;
use compass_ipc::{DoctorCheck, DoctorStatus};

use super::check;

/// `extensions.declared`: fine when everything declared is installed (or
/// nothing is), a warning naming each entry that is not installed yet and
/// each one that can never be.
#[must_use]
pub fn declared_extensions(plan: &Result<Vec<Step>, String>) -> DoctorCheck {
    const NAME: &str = "extensions.declared";
    let steps = match plan {
        Ok(steps) => steps,
        Err(error) => {
            return check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "compass.json cannot be read, so the extensions it declares are not known: {error}"
                ),
            );
        }
    };
    if steps.is_empty() {
        return check(
            NAME,
            DoctorStatus::Ok,
            "compass.json declares no extensions to install.",
        );
    }
    let mut missing = Vec::new();
    let mut unresolvable = Vec::new();
    for step in steps {
        match step {
            Step::Present { .. } => {}
            Step::Install { entry, .. } => missing.push(entry.as_str()),
            Step::Unresolvable { entry, reason } => {
                unresolvable.push(format!("{entry} cannot be installed: {reason}"));
            }
        }
    }
    if missing.is_empty() && unresolvable.is_empty() {
        return check(
            NAME,
            DoctorStatus::Ok,
            if steps.len() == 1 {
                "The extension compass.json declares is installed.".to_owned()
            } else {
                format!(
                    "All {} extensions compass.json declares are installed.",
                    steps.len()
                )
            },
        );
    }
    let mut sentences = Vec::new();
    match missing.as_slice() {
        [] => {}
        [one] => sentences.push(format!(
            "{one} is not installed yet. The engine installs it when it starts and whenever compass.json changes, and tries again while the store cannot be reached; run compass ext sync to install it now and see any error."
        )),
        many => sentences.push(format!(
            "{} declared extensions are not installed yet: {}. The engine installs them when it starts and whenever compass.json changes, and tries again while the store cannot be reached; run compass ext sync to install them now and see any error.",
            many.len(),
            many.join(", ")
        )),
    }
    sentences.extend(unresolvable);
    check(NAME, DoctorStatus::Warn, sentences.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_core::declared_extensions::StoreExtension;
    use compass_core::store_listing::Store;

    fn present(entry: &str) -> Step {
        Step::Present {
            entry: entry.into(),
            id: entry.into(),
        }
    }

    fn install(entry: &str) -> Step {
        Step::Install {
            entry: entry.into(),
            extension: StoreExtension {
                store: Store::Raycast,
                owner: None,
                name: entry.into(),
            },
        }
    }

    #[test]
    fn everything_installed_or_nothing_declared_is_fine() {
        let none = declared_extensions(&Ok(Vec::new()));
        assert_eq!(none.status, DoctorStatus::Ok);
        let one = declared_extensions(&Ok(vec![present("a")]));
        assert_eq!(
            one.detail.as_deref(),
            Some("The extension compass.json declares is installed.")
        );
        let two = declared_extensions(&Ok(vec![present("a"), present("b")]));
        assert_eq!(two.status, DoctorStatus::Ok);
        assert_eq!(
            two.detail.as_deref(),
            Some("All 2 extensions compass.json declares are installed.")
        );
    }

    #[test]
    fn what_is_missing_or_impossible_is_named() {
        let report = declared_extensions(&Ok(vec![
            present("vicinae:zoe/clock"),
            install("raycast:raycast/github"),
            install("store.raycast.hn"),
            Step::Unresolvable {
                entry: "brew:x/y".into(),
                reason: "\"brew\" is not a store Compass installs from; use raycast or vicinae."
                    .into(),
            },
        ]));
        assert_eq!(report.status, DoctorStatus::Warn);
        let detail = report.detail.unwrap();
        assert!(
            detail.starts_with(
                "2 declared extensions are not installed yet: raycast:raycast/github, store.raycast.hn."
            ),
            "{detail}"
        );
        assert!(
            detail.ends_with(
                "brew:x/y cannot be installed: \"brew\" is not a store Compass installs from; use raycast or vicinae."
            ),
            "{detail}"
        );
        assert!(!detail.contains("vicinae:zoe/clock"));
    }

    #[test]
    fn an_unreadable_file_warns() {
        let report = declared_extensions(&Err("expected value at line 1 column 1".into()));
        assert_eq!(report.status, DoctorStatus::Warn);
    }
}
