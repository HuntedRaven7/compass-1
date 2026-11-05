//! Whether a screen reader is running, and what it can read here.
//!
//! Part of [`super`]; see that module for the purity rule every function here
//! keeps to.
//!
//! The launcher describes itself on AT-SPI (ADR-0022 §1): the search field,
//! the results and the selection, the action panel and forms. Its other views
//! describe only their search field, and Orca has not yet been checked
//! against it in a real session (#305). So a running screen reader is still a
//! warning, and only then does it matter.

use compass_ipc::{DoctorCheck, DoctorStatus};

use super::check;
use crate::doctor::bus::BusProbe;

/// Bus name of the accessibility bus launcher, which owns the switch.
pub const A11Y_BUS_NAME: &str = "org.a11y.Bus";
/// Object path the switch is exported on.
pub const A11Y_OBJECT_PATH: &str = "/org/a11y/bus";
/// Interface holding the screen-reader switch GNOME's settings flip.
pub const A11Y_STATUS_INTERFACE: &str = "org.a11y.Status";

/// Whether a screen reader is enabled, and so whether what the tree leaves
/// out matters.
pub async fn screen_reader<B: BusProbe>(bus: &B) -> DoctorCheck {
    const NAME: &str = "a11y.screen-reader";
    let enabled = bus
        .property(
            A11Y_BUS_NAME,
            A11Y_OBJECT_PATH,
            A11Y_STATUS_INTERFACE,
            "ScreenReaderEnabled",
        )
        .await;
    match enabled {
        Ok(Some(value)) if value == "true" => check(
            NAME,
            DoctorStatus::Warn,
            "a screen reader is enabled. Compass describes its search, results, action panel \
             and forms to it, but its other views show only their search field, and it has not \
             been checked with Orca yet",
        ),
        Ok(Some(_)) => check(NAME, DoctorStatus::Ok, "no screen reader enabled"),
        Ok(None) => check(
            NAME,
            DoctorStatus::Ok,
            format!("{A11Y_BUS_NAME} does not report a screen-reader switch, so none is running"),
        ),
        Err(err) => check(
            NAME,
            DoctorStatus::Warn,
            format!("could not ask {A11Y_BUS_NAME} whether a screen reader is enabled: {err}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::bus::FakeBus;

    fn reader(value: &str) -> FakeBus {
        FakeBus::new().with_property(
            A11Y_BUS_NAME,
            A11Y_OBJECT_PATH,
            A11Y_STATUS_INTERFACE,
            "ScreenReaderEnabled",
            value,
        )
    }

    #[tokio::test]
    async fn an_enabled_screen_reader_is_told_what_it_can_and_cannot_read() {
        let result = screen_reader(&reader("true")).await;
        assert_eq!(result.status, DoctorStatus::Warn);
        let detail = result.detail.expect("detail");
        assert!(!detail.contains("#305"), "{detail}");
        assert!(detail.contains("describes its search"), "{detail}");
        assert!(detail.contains("other views"), "{detail}");
    }

    #[tokio::test]
    async fn a_disabled_screen_reader_is_fine() {
        assert_eq!(
            screen_reader(&reader("false")).await.status,
            DoctorStatus::Ok
        );
    }

    #[tokio::test]
    async fn no_accessibility_bus_means_no_screen_reader() {
        assert_eq!(
            screen_reader(&FakeBus::new()).await.status,
            DoctorStatus::Ok
        );
    }

    #[tokio::test]
    async fn a_failed_query_is_not_reported_as_all_clear() {
        let result = screen_reader(&FakeBus::failing_queries("timeout")).await;
        assert_eq!(result.status, DoctorStatus::Warn);
    }
}
