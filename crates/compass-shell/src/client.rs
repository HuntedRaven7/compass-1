//! The client: capability probing, graceful degradation, reconnection.

use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::Context;
use std::time::Duration;

use zbus::export::futures_core::Stream;
use zbus::{Connection, proxy::CacheProperties};

use crate::capability::{Availability, ShellCapabilities};
use crate::contract;
use crate::error::{Result, ShellError};
use crate::model::{ClipboardChange, ClipboardContent, Window, WindowId};
use crate::proxy::{ClipboardProxy, WindowsProxy};

/// Retry policy used when re-probing after `gnome-shell` reappears.
///
/// Reloading an extension restarts `gnome-shell`, and the extension's D-Bus
/// objects are exported some milliseconds after the shell takes its bus name
/// again. The C++ implementation handled this with a fixed 5 x 500 ms retry;
/// this is the same idea with exponential spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// Delay before the second attempt.
    pub initial: Duration,
    /// Ceiling for the delay.
    pub max: Duration,
    /// Total attempts per reconnection, including the first.
    pub attempts: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(250),
            max: Duration::from_secs(5),
            attempts: 6,
        }
    }
}

impl Backoff {
    fn delay_for(self, attempt: u32) -> Duration {
        let scaled = self
            .initial
            .checked_mul(1u32 << attempt.min(16))
            .unwrap_or(self.max);
        scaled.min(self.max)
    }
}

/// Where to find the extension, and how hard to try.
#[derive(Debug, Clone)]
pub struct ShellConfig {
    /// Explicit bus address. `None` means the ambient session bus.
    ///
    /// Tests use this to run against a private `dbus-daemon` without touching
    /// `DBUS_SESSION_BUS_ADDRESS`.
    pub address: Option<String>,
    /// Well-known name to talk to.
    pub service: String,
    /// Object path of the windows interface.
    pub windows_path: String,
    /// Object path of the clipboard interface.
    pub clipboard_path: String,
    /// Reconnection retry policy.
    pub backoff: Backoff,
    /// Deadline for any single D-Bus call, including capability probes.
    ///
    /// D-Bus method calls have no inherent timeout. A `gnome-shell` that owns
    /// the bus name but has stopped servicing its main loop would otherwise
    /// hang every launcher interaction, so every call is bounded.
    pub call_timeout: Duration,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            address: None,
            service: contract::SHELL_SERVICE.to_owned(),
            windows_path: contract::WINDOWS_PATH.to_owned(),
            clipboard_path: contract::CLIPBOARD_PATH.to_owned(),
            backoff: Backoff::default(),
            call_timeout: Duration::from_secs(5),
        }
    }
}

impl ShellConfig {
    /// Point the client at a specific bus address.
    pub fn with_address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    /// Override the reconnection policy.
    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// Override the per-call deadline.
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }
}

struct Shared {
    config: ShellConfig,
    conn: Connection,
    caps: RwLock<ShellCapabilities>,
    caps_tx: tokio::sync::broadcast::Sender<ShellCapabilities>,
}

impl Shared {
    async fn windows_proxy(&self) -> Result<WindowsProxy<'static>> {
        WindowsProxy::builder(&self.conn)
            .destination(self.config.service.clone())
            .and_then(|b| b.path(self.config.windows_path.clone()))
            .map_err(ShellError::Call)?
            .cache_properties(CacheProperties::No)
            .build()
            .await
            .map_err(ShellError::Call)
    }

    async fn clipboard_proxy(&self) -> Result<ClipboardProxy<'static>> {
        ClipboardProxy::builder(&self.conn)
            .destination(self.config.service.clone())
            .and_then(|b| b.path(self.config.clipboard_path.clone()))
            .map_err(ShellError::Call)?
            .cache_properties(CacheProperties::No)
            .build()
            .await
            .map_err(ShellError::Call)
    }

    /// Bound any single D-Bus round trip.
    async fn bounded<T>(
        &self,
        method: &'static str,
        fut: impl std::future::Future<Output = zbus::Result<T>>,
    ) -> Result<T> {
        let timeout = self.config.call_timeout;
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(err)) => Err(ShellError::Call(err)),
            Err(_) => Err(ShellError::Timeout { method, timeout }),
        }
    }

    async fn probe_one(
        &self,
        what: &'static str,
        fut: impl std::future::Future<Output = zbus::Result<u32>>,
    ) -> Availability {
        match self.bounded(what, fut).await {
            Ok(version) => Availability::from_probe(Ok(version)),
            Err(ShellError::Call(err)) => Availability::from_probe(Err(err)),
            Err(err) => {
                tracing::warn!(%err, "shell extension probe timed out; treating as absent");
                Availability::Absent
            }
        }
    }

    /// Read both `Version` properties and classify them.
    async fn probe(&self) -> ShellCapabilities {
        let windows = match self.windows_proxy().await {
            Ok(p) => self.probe_one("Windows.Version", p.version()).await,
            Err(_) => Availability::Absent,
        };
        let clipboard = match self.clipboard_proxy().await {
            Ok(p) => self.probe_one("Clipboard.Version", p.version()).await,
            Err(_) => Availability::Absent,
        };
        ShellCapabilities { windows, clipboard }
    }

    fn publish(&self, next: ShellCapabilities) -> ShellCapabilities {
        let changed = {
            let mut guard = self.caps.write().unwrap_or_else(|e| e.into_inner());
            if *guard == next {
                false
            } else {
                *guard = next;
                true
            }
        };
        if changed {
            tracing::info!(
                windows = %next.windows,
                clipboard = %next.clipboard,
                "shell extension capabilities changed"
            );
            // A send error only means nobody is listening.
            let _ = self.caps_tx.send(next);
        }
        next
    }

    fn snapshot(&self) -> ShellCapabilities {
        *self.caps.read().unwrap_or_else(|e| e.into_inner())
    }

    async fn probe_and_publish(&self) -> ShellCapabilities {
        let next = self.probe().await;
        self.publish(next)
    }

    /// Probe repeatedly until something is available or attempts run out.
    async fn probe_with_backoff(&self) -> ShellCapabilities {
        let backoff = self.config.backoff;
        let mut last = self.probe_and_publish().await;
        for attempt in 0..backoff.attempts.saturating_sub(1) {
            if last.any_available() {
                break;
            }
            tokio::time::sleep(backoff.delay_for(attempt)).await;
            last = self.probe_and_publish().await;
        }
        last
    }
}

/// Async client for the GNOME Shell helper extension.
///
/// Cloning is cheap and shares one connection, one capability cache and one
/// reconnection supervisor.
#[derive(Clone)]
pub struct ShellClient {
    shared: Arc<Shared>,
    _supervisor: Arc<SupervisorHandle>,
}

struct SupervisorHandle(tokio::task::JoinHandle<()>);

impl Drop for SupervisorHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl std::fmt::Debug for ShellClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellClient")
            .field("service", &self.shared.config.service)
            .field("capabilities", &self.shared.snapshot())
            .finish()
    }
}

impl ShellClient {
    /// Connect to the bus and probe for the extension.
    ///
    /// This fails only when the session bus itself is unreachable. A missing
    /// or wrong-version extension is reported through [`Self::capabilities`],
    /// not as an error.
    pub async fn connect(config: ShellConfig) -> Result<Self> {
        let conn = match &config.address {
            Some(address) => zbus::connection::Builder::address(address.as_str())
                .map_err(ShellError::Bus)?
                .build()
                .await
                .map_err(ShellError::Bus)?,
            None => Connection::session().await.map_err(ShellError::Bus)?,
        };

        let (caps_tx, _) = tokio::sync::broadcast::channel(16);
        let shared = Arc::new(Shared {
            config,
            conn,
            caps: RwLock::new(ShellCapabilities::absent()),
            caps_tx,
        });

        // Subscribe to NameOwnerChanged *before* the first probe, and before
        // returning: a subscription installed later by the spawned task could
        // miss a `gnome-shell` restart that happens in between, and a missed
        // edge means a client that never reconnects.
        let owner_changes = subscribe_owner_changes(&shared).await;

        shared.probe_and_publish().await;

        let supervisor = tokio::spawn(supervise(Arc::clone(&shared), owner_changes));

        Ok(Self {
            shared,
            _supervisor: Arc::new(SupervisorHandle(supervisor)),
        })
    }

    /// Connect with the default configuration (ambient session bus).
    pub async fn connect_session() -> Result<Self> {
        Self::connect(ShellConfig::default()).await
    }

    /// The underlying D-Bus connection, for callers that need it.
    pub fn connection(&self) -> &Connection {
        &self.shared.conn
    }

    /// Last known capabilities. Cheap; never blocks on the bus.
    pub fn capabilities(&self) -> ShellCapabilities {
        self.shared.snapshot()
    }

    /// Re-read both `Version` properties now and update the cache.
    pub async fn refresh_capabilities(&self) -> ShellCapabilities {
        self.shared.probe_and_publish().await
    }

    /// Subscribe to capability transitions (extension appearing, disappearing,
    /// or changing version). Only actual changes are published.
    pub fn capability_changes(&self) -> CapabilityStream {
        CapabilityStream(self.shared.caps_tx.subscribe())
    }

    /// Check a capability, re-probing once if the cache says it is unusable.
    ///
    /// The re-probe matters because enabling an extension does not change the
    /// owner of `org.gnome.Shell`, so there is no `NameOwnerChanged` to react
    /// to: the object simply starts existing.
    async fn require(&self, windows: bool) -> Result<()> {
        let pick = |c: ShellCapabilities| if windows { c.windows } else { c.clipboard };
        if pick(self.shared.snapshot()).is_available() {
            return Ok(());
        }
        let refreshed = pick(self.shared.probe_and_publish().await);
        if refreshed.is_available() {
            Ok(())
        } else {
            Err(ShellError::Unavailable(refreshed))
        }
    }

    /// List all windows the extension exposes.
    pub async fn list_windows(&self) -> Result<Vec<Window>> {
        self.require(true).await?;
        let proxy = self.shared.windows_proxy().await?;
        let raw = self
            .shared
            .bounded("ListWindows", proxy.list_windows())
            .await?;
        raw.iter().map(Window::from_dict).collect()
    }

    /// Focus and raise a window.
    pub async fn activate_window(&self, id: WindowId) -> Result<()> {
        self.require(true).await?;
        let proxy = self.shared.windows_proxy().await?;
        self.shared
            .bounded("ActivateWindow", proxy.activate_window(id.0))
            .await
    }

    /// Ask a window to close.
    pub async fn close_window(&self, id: WindowId) -> Result<()> {
        self.require(true).await?;
        let proxy = self.shared.windows_proxy().await?;
        self.shared
            .bounded("CloseWindow", proxy.close_window(id.0))
            .await
    }

    /// Subscribe to `WindowsChanged`.
    ///
    /// The match rule is installed before this returns, so a caller that
    /// awaits this and then triggers a change cannot miss the signal.
    pub async fn windows_changed(&self) -> Result<WindowsChangedStream> {
        self.require(true).await?;
        let proxy = self.shared.windows_proxy().await?;
        let inner = proxy
            .receive_windows_changed()
            .await
            .map_err(ShellError::Call)?;
        Ok(WindowsChangedStream(inner))
    }

    /// Read the current selection.
    pub async fn clipboard(&self) -> Result<ClipboardContent> {
        self.require(false).await?;
        let proxy = self.shared.clipboard_proxy().await?;
        let (data, mime_type) = self
            .shared
            .bounded("GetClipboard", proxy.get_clipboard())
            .await?;
        Ok(ClipboardContent { data, mime_type })
    }

    /// Replace the current selection.
    pub async fn set_clipboard(&self, content: &ClipboardContent) -> Result<()> {
        self.require(false).await?;
        let proxy = self.shared.clipboard_proxy().await?;
        self.shared
            .bounded(
                "SetClipboard",
                proxy.set_clipboard(&content.data, &content.mime_type),
            )
            .await
    }

    /// Subscribe to `ClipboardChanged`.
    pub async fn clipboard_changes(&self) -> Result<ClipboardStream> {
        self.require(false).await?;
        let proxy = self.shared.clipboard_proxy().await?;
        let inner = proxy
            .receive_clipboard_changed()
            .await
            .map_err(ShellError::Call)?;
        Ok(ClipboardStream(inner))
    }
}

/// Install the `NameOwnerChanged` match rule for our service.
///
/// The `DBusProxy` is returned alongside the stream because it owns the match
/// rule: dropping it would tear the subscription down.
async fn subscribe_owner_changes(
    shared: &Shared,
) -> Option<(
    zbus::fdo::DBusProxy<'static>,
    zbus::fdo::NameOwnerChangedStream,
)> {
    let dbus = match zbus::fdo::DBusProxy::new(&shared.conn).await {
        Ok(p) => p,
        Err(err) => {
            tracing::warn!(%err, "cannot watch the bus for gnome-shell restarts");
            return None;
        }
    };
    let service = shared.config.service.clone();
    match dbus
        .receive_name_owner_changed_with_args(&[(0, service.as_str())])
        .await
    {
        Ok(stream) => Some((dbus, stream)),
        Err(err) => {
            tracing::warn!(%err, "cannot subscribe to NameOwnerChanged");
            None
        }
    }
}

/// Watch `org.gnome.Shell` coming and going, re-probing on each transition.
///
/// Reloading a GNOME Shell extension restarts `gnome-shell`, so this is the
/// common case, not an exceptional one.
async fn supervise(
    shared: Arc<Shared>,
    subscription: Option<(
        zbus::fdo::DBusProxy<'static>,
        zbus::fdo::NameOwnerChangedStream,
    )>,
) {
    let Some((_dbus, mut owner_changes)) = subscription else {
        return;
    };

    while let Some(signal) = next_signal(&mut owner_changes).await {
        let Ok(args) = signal.args() else {
            continue;
        };
        let acquired = args.new_owner().is_some();
        tracing::info!(
            service = %shared.config.service,
            acquired,
            "gnome-shell bus name transition"
        );
        if acquired {
            // The shell is back, but its extensions are exported a moment
            // after it takes the name again: retry with backoff.
            shared.probe_with_backoff().await;
        } else {
            shared.publish(ShellCapabilities::absent());
        }
    }
}

async fn next_signal<S: Stream + Unpin>(stream: &mut S) -> Option<S::Item> {
    std::future::poll_fn(|cx: &mut Context<'_>| Pin::new(&mut *stream).poll_next(cx)).await
}

/// Stream of capability transitions.
///
/// Backed by a broadcast channel, so a slow consumer lags rather than blocking
/// the supervisor; [`Self::next`] transparently resynchronises after a lag.
pub struct CapabilityStream(tokio::sync::broadcast::Receiver<ShellCapabilities>);

impl CapabilityStream {
    /// Await the next capability change. `None` once the client is dropped.
    pub async fn next(&mut self) -> Option<ShellCapabilities> {
        loop {
            match self.0.recv().await {
                Ok(caps) => return Some(caps),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "capability subscriber lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// Stream of `WindowsChanged` notifications.
pub struct WindowsChangedStream(crate::proxy::WindowsChangedStream);

impl WindowsChangedStream {
    /// Await the next change notification.
    pub async fn next(&mut self) -> Option<()> {
        next_signal(&mut self.0).await.map(|_| ())
    }
}

/// Stream of clipboard changes.
pub struct ClipboardStream(crate::proxy::ClipboardChangedStream);

impl ClipboardStream {
    /// Await the next clipboard change.
    ///
    /// A signal whose body does not match the contract yields
    /// `Some(Err(ShellError::Protocol))` rather than panicking or silently
    /// ending the stream.
    pub async fn next(&mut self) -> Option<Result<ClipboardChange>> {
        let signal = next_signal(&mut self.0).await?;
        Some(match signal.args() {
            Ok(args) => Ok(ClipboardChange {
                content: ClipboardContent {
                    data: args.content().clone(),
                    mime_type: args.mime_type().clone(),
                },
                source_app: Some(args.source_app().clone()).filter(|s| !s.is_empty()),
            }),
            Err(err) => Err(ShellError::Protocol(format!(
                "ClipboardChanged signal body does not match the contract: {err}"
            ))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_is_capped() {
        let backoff = Backoff {
            initial: Duration::from_millis(100),
            max: Duration::from_millis(700),
            attempts: 6,
        };
        let delays: Vec<_> = (0..6).map(|i| backoff.delay_for(i)).collect();
        assert_eq!(delays[0], Duration::from_millis(100));
        assert_eq!(delays[1], Duration::from_millis(200));
        assert_eq!(delays[2], Duration::from_millis(400));
        for pair in delays.windows(2) {
            assert!(pair[1] >= pair[0], "backoff must not shrink: {delays:?}");
        }
        assert!(
            delays.iter().all(|d| *d <= backoff.max),
            "backoff must be capped: {delays:?}"
        );
    }

    #[test]
    fn backoff_does_not_overflow_on_absurd_attempt_counts() {
        let backoff = Backoff {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(30),
            attempts: u32::MAX,
        };
        assert_eq!(backoff.delay_for(u32::MAX), backoff.max);
    }

    #[test]
    fn default_config_targets_the_contract() {
        let config = ShellConfig::default();
        assert_eq!(config.service, contract::SHELL_SERVICE);
        assert_eq!(config.windows_path, contract::WINDOWS_PATH);
        assert_eq!(config.clipboard_path, contract::CLIPBOARD_PATH);
        assert!(config.address.is_none());
    }
}
