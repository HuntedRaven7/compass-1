//! An injectable view of the D-Bus session bus.
//!
//! Three questions cover every bus-dependent check the doctor makes:
//!
//! * can we reach a session bus at all ([`BusProbe::connect`]),
//! * does someone own this bus name ([`BusProbe::name_has_owner`]),
//! * what does this property read back as ([`BusProbe::property`]).
//!
//! # `Ok(None)` versus `Err`
//!
//! The distinction is the whole point of the trait and the doctor leans on it
//! everywhere: `Ok(None)` means *we asked and the answer is "that is not
//! there"*, while `Err` means *we could not ask*. A doctor that collapses the
//! two reports missing portals on a machine whose bus is merely unreachable,
//! which is exactly the kind of confident wrongness `PLAN.md` §8.6 warns about.

use std::fmt;
use std::future::Future;

/// A bus interaction that could not be completed.
///
/// This is "we could not ask", never "the answer was no".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusError(String);

impl BusError {
    /// Builds an error with `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for BusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BusError {}

/// Result alias for bus probes.
pub type BusResult<T> = Result<T, BusError>;

/// Read-only session-bus queries used by the doctor's checks.
pub trait BusProbe {
    /// Establishes (or reuses) a session-bus connection.
    fn connect(&self) -> impl Future<Output = BusResult<()>>;

    /// Whether `name` currently has an owner on the session bus.
    ///
    /// `Ok(false)` for a D-Bus-activatable service that simply has not been
    /// started yet, which is why the portal check follows a `false` up with a
    /// property read rather than declaring the portal missing.
    fn name_has_owner(&self, name: &str) -> impl Future<Output = BusResult<bool>>;

    /// Reads a property, rendered as a string.
    ///
    /// `Ok(None)` when the destination, object, interface or property does not
    /// exist — the four ways D-Bus says "no such capability". Rendering as a
    /// string keeps the trait free of `zvariant` types, so a fake is a map.
    fn property(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        property: &str,
    ) -> impl Future<Output = BusResult<Option<String>>>;
}

/// Address of a property, as [`FakeBus`] keys them.
pub type PropertyKey = (String, String, String, String);

/// [`BusProbe`] built from literal facts, for tests.
///
/// Default state: the bus is reachable, nobody owns any name and no property
/// exists. Every interesting doctor case is one or two builder calls from
/// there.
#[derive(Debug, Clone, Default)]
pub struct FakeBus {
    connect_error: Option<String>,
    query_error: Option<String>,
    owned_names: std::collections::BTreeSet<String>,
    properties: std::collections::BTreeMap<PropertyKey, String>,
}

impl FakeBus {
    /// A reachable but completely empty session bus.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A bus that cannot be connected to at all.
    #[must_use]
    pub fn unreachable(reason: impl Into<String>) -> Self {
        Self {
            connect_error: Some(reason.into()),
            ..Self::default()
        }
    }

    /// A bus that connects but fails every query — the "we could not ask" case.
    #[must_use]
    pub fn failing_queries(reason: impl Into<String>) -> Self {
        Self {
            query_error: Some(reason.into()),
            ..Self::default()
        }
    }

    /// Declares `name` as owned.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.owned_names.insert(name.into());
        self
    }

    /// Declares a readable property.
    #[must_use]
    pub fn with_property(
        mut self,
        destination: &str,
        path: &str,
        interface: &str,
        property: &str,
        value: impl Into<String>,
    ) -> Self {
        self.properties.insert(
            (
                destination.to_string(),
                path.to_string(),
                interface.to_string(),
                property.to_string(),
            ),
            value.into(),
        );
        self
    }

    fn guard(&self) -> BusResult<()> {
        if let Some(reason) = &self.connect_error {
            return Err(BusError::new(reason.clone()));
        }
        if let Some(reason) = &self.query_error {
            return Err(BusError::new(reason.clone()));
        }
        Ok(())
    }
}

impl BusProbe for FakeBus {
    async fn connect(&self) -> BusResult<()> {
        match &self.connect_error {
            Some(reason) => Err(BusError::new(reason.clone())),
            None => Ok(()),
        }
    }

    async fn name_has_owner(&self, name: &str) -> BusResult<bool> {
        self.guard()?;
        Ok(self.owned_names.contains(name))
    }

    async fn property(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        property: &str,
    ) -> BusResult<Option<String>> {
        self.guard()?;
        let key = (
            destination.to_string(),
            path.to_string(),
            interface.to_string(),
            property.to_string(),
        );
        Ok(self.properties.get(&key).cloned())
    }
}

/// [`BusProbe`] backed by a real session bus, via `zbus`.
#[derive(Debug, Default)]
pub struct ZbusProbe {
    connection: tokio::sync::OnceCell<zbus::Connection>,
}

impl ZbusProbe {
    /// Builds a probe that connects lazily, on first use.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    async fn connection(&self) -> BusResult<&zbus::Connection> {
        self.connection
            .get_or_try_init(|| async {
                zbus::Connection::session()
                    .await
                    .map_err(|e| BusError::new(e.to_string()))
            })
            .await
    }
}

/// Renders a D-Bus value as the string the doctor reports.
///
/// `zvariant`'s own `Display` is type-annotated (`uint32 2`, `'51.0'`), which
/// is right for a debugging dump and wrong inside a sentence, so the scalars
/// the doctor actually reads back are unwrapped by hand. Anything else keeps
/// the annotated form: a surprising type is better shown than flattened.
fn render(value: &zbus::zvariant::Value<'_>) -> String {
    use zbus::zvariant::Value;
    match value {
        Value::Str(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        Value::U8(v) => v.to_string(),
        Value::I16(v) => v.to_string(),
        Value::U16(v) => v.to_string(),
        Value::I32(v) => v.to_string(),
        Value::U32(v) => v.to_string(),
        Value::I64(v) => v.to_string(),
        Value::U64(v) => v.to_string(),
        Value::F64(v) => v.to_string(),
        Value::Value(inner) => render(inner),
        other => other.to_string(),
    }
}

/// Whether a D-Bus error means "that capability is not there", as opposed to
/// "the request failed".
fn is_absence(err: &zbus::fdo::Error) -> bool {
    matches!(
        err,
        zbus::fdo::Error::ServiceUnknown(_)
            | zbus::fdo::Error::NameHasNoOwner(_)
            | zbus::fdo::Error::UnknownObject(_)
            | zbus::fdo::Error::UnknownInterface(_)
            | zbus::fdo::Error::UnknownMethod(_)
            | zbus::fdo::Error::UnknownProperty(_)
            | zbus::fdo::Error::InvalidArgs(_)
            // When no portal frontend is running, D-Bus may try to auto-activate
            // the service. If activation fails (e.g. the .service file is missing
            // or the binary crashes), we get Spawn.* errors. Treat these the same
            // as "that capability is not there" because from the launcher's
            // perspective there is no usable portal frontend.
            | zbus::fdo::Error::SpawnExecFailed(_)
            | zbus::fdo::Error::SpawnForkFailed(_)
            | zbus::fdo::Error::SpawnChildExited(_)
            | zbus::fdo::Error::SpawnChildSignaled(_)
            | zbus::fdo::Error::SpawnFailed(_)
            | zbus::fdo::Error::SpawnFailedToSetup(_)
            | zbus::fdo::Error::SpawnConfigInvalid(_)
            | zbus::fdo::Error::SpawnServiceNotValid(_)
    )
}

impl BusProbe for ZbusProbe {
    async fn connect(&self) -> BusResult<()> {
        self.connection().await.map(|_| ())
    }

    async fn name_has_owner(&self, name: &str) -> BusResult<bool> {
        let connection = self.connection().await?;
        let dbus = zbus::fdo::DBusProxy::new(connection)
            .await
            .map_err(|e| BusError::new(e.to_string()))?;
        let name = zbus::names::BusName::try_from(name.to_string())
            .map_err(|e| BusError::new(format!("invalid bus name: {e}")))?;
        dbus.name_has_owner(name)
            .await
            .map_err(|e| BusError::new(e.to_string()))
    }

    async fn property(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        property: &str,
    ) -> BusResult<Option<String>> {
        let connection = self.connection().await?;
        let destination = zbus::names::BusName::try_from(destination.to_string())
            .map_err(|e| BusError::new(format!("invalid destination: {e}")))?;
        let path = zbus::zvariant::ObjectPath::try_from(path.to_string())
            .map_err(|e| BusError::new(format!("invalid object path: {e}")))?;
        let interface = zbus::names::InterfaceName::try_from(interface.to_string())
            .map_err(|e| BusError::new(format!("invalid interface name: {e}")))?;

        let proxy = zbus::fdo::PropertiesProxy::builder(connection)
            .destination(destination)
            .map_err(|e| BusError::new(e.to_string()))?
            .path(path)
            .map_err(|e| BusError::new(e.to_string()))?
            .build()
            .await
            .map_err(|e| BusError::new(e.to_string()))?;

        match proxy.get(interface, property).await {
            Ok(value) => Ok(Some(render(&value))),
            Err(err) if is_absence(&err) => Ok(None),
            Err(err) => Err(BusError::new(err.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_fake_bus_connects_and_knows_nothing() {
        let bus = FakeBus::new();
        assert_eq!(bus.connect().await, Ok(()));
        assert_eq!(bus.name_has_owner("org.example.Thing").await, Ok(false));
        assert_eq!(bus.property("a", "/b", "c", "d").await, Ok(None));
    }

    #[tokio::test]
    async fn unreachable_fake_bus_errors_everywhere() {
        let bus = FakeBus::unreachable("No such file or directory");
        assert!(bus.connect().await.is_err());
        assert!(bus.name_has_owner("org.example.Thing").await.is_err());
        assert!(bus.property("a", "/b", "c", "d").await.is_err());
    }

    #[tokio::test]
    async fn failing_queries_still_connect() {
        let bus = FakeBus::failing_queries("Connection reset");
        assert_eq!(bus.connect().await, Ok(()));
        assert!(bus.name_has_owner("org.example.Thing").await.is_err());
    }

    #[tokio::test]
    async fn declared_facts_read_back() {
        let bus = FakeBus::new().with_name("org.gnome.Shell").with_property(
            "org.gnome.Shell",
            "/org/gnome/Shell",
            "org.gnome.Shell",
            "ShellVersion",
            "51.0",
        );

        assert_eq!(bus.name_has_owner("org.gnome.Shell").await, Ok(true));
        assert_eq!(bus.name_has_owner("org.kde.Whatever").await, Ok(false));
        assert_eq!(
            bus.property(
                "org.gnome.Shell",
                "/org/gnome/Shell",
                "org.gnome.Shell",
                "ShellVersion"
            )
            .await,
            Ok(Some("51.0".to_string()))
        );
        assert_eq!(
            bus.property(
                "org.gnome.Shell",
                "/org/gnome/Shell",
                "org.gnome.Shell",
                "Nope"
            )
            .await,
            Ok(None)
        );
    }

    #[test]
    fn scalar_values_render_without_type_annotations() {
        use zbus::zvariant::Value;
        assert_eq!(render(&Value::from("51.0")), "51.0");
        assert_eq!(render(&Value::from(2u32)), "2");
        assert_eq!(render(&Value::from(1i32)), "1");
        assert_eq!(render(&Value::from(true)), "true");
        // A variant-wrapped scalar is what a property read commonly yields.
        assert_eq!(render(&Value::Value(Box::new(Value::from(7u32)))), "7");
    }

    #[test]
    fn non_scalar_values_keep_their_annotated_form_rather_than_being_flattened() {
        let rendered = render(&zbus::zvariant::Value::from(vec!["a", "b"]));
        assert!(rendered.contains('a'), "unexpected rendering: {rendered}");
    }

    #[test]
    fn bus_error_displays_its_message() {
        assert_eq!(BusError::new("boom").to_string(), "boom");
    }
}
