//! The configuration's global shortcuts, bound on the desktop, and what
//! happens when one fires: `GlobalShortcutService`.
//!
//! What is bound is `compass_core::global_shortcuts`' to decide: the
//! launcher hotkey (`launcher.hotkey`) and every command's
//! `providers.<p>.entrypoints.<e>.shortcut`. This is the part that knows the
//! desktop. A [`Backend`] binds one shortcut at a time and reports each
//! press as the id it was bound under; a [`Service`] keeps the backend in
//! step with the configuration (`reconcile`), suspends every binding while
//! the shortcut recorder captures (`setCapturing`), and turns a press into
//! an [`Action`]: the launcher's toggle, pushed to the attached window, or a
//! command launched as `cmd launch` and root search launch it
//! (`activateEntrypoint`).
//!
//! # Which backend
//!
//! The C++ factory's order, then the portal the C++ does not have:
//!
//! 1. `xx-hotkey-v1`, then `vicinae-hotkey-v1`, over Wayland
//!    ([`compass_wayland::hotkey`]), on any compositor but GNOME's.
//! 2. `org.freedesktop.portal.GlobalShortcuts` ([`ShortcutBinder`]): on
//!    GNOME 50/51, the first target, the only path an unprivileged
//!    application has. The portal binds a set at a time, so the backend
//!    collects the binds of one reconcile and hands the whole set over at
//!    the end of it.
//!
//! No backend is a warning, never a reason to stop: the engine still answers
//! `compass toggle`, and on a wlroots compositor the log says how to bind
//! that by hand.
//!
//! # The first run
//!
//! The portal's permission dialog is the desktop's own, and appearing beside
//! a first-run flow that has not yet explained it is what made it look like
//! something the launcher did by itself. So while the flow is due, the portal
//! is not asked at start ([`Service::defer`]): the flow's Set Up Shortcut
//! asks ([`Control::request_launcher`]), and finishing or closing the flow
//! asks if the person did not. Every outcome is kept as a
//! [`LauncherShortcut`] for the flow and Settings to show.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use compass_core::Config;
use compass_core::global_shortcuts::{self, Action, Binding, LAUNCHER_ID, Reconciler};
use compass_core::key_combo::KeyCombo;
use compass_core::launcher_shortcut::LauncherShortcut;
use compass_ipc::{Response, WindowCommand};
use compass_portals::{
    PortalConfig, PortalError, Portals, ShortcutBinder, ShortcutDescriptor, ShortcutEvent,
    ShortcutsOutcome, Trigger,
};
use tokio::sync::{RwLock, mpsc, oneshot, watch};

use crate::serve::{EngineState, forward};

/// What the rest of the engine uses to steer the service: a new
/// configuration to bind, the recorder capturing, and the first-run flow
/// asking for the launcher hotkey.
#[derive(Debug)]
pub struct Control {
    generation: watch::Sender<u64>,
    capturing: watch::Sender<bool>,
    frontmost: watch::Sender<Option<String>>,
    probes: std::sync::Mutex<Option<mpsc::UnboundedSender<Probe>>>,
    launcher: watch::Sender<LauncherShortcut>,
    requests: std::sync::Mutex<Option<mpsc::UnboundedSender<LauncherRequest>>>,
}

/// A combination the recorder captured, and where the backend's answer
/// goes.
type Probe = (KeyCombo, oneshot::Sender<Option<String>>);

/// The first-run flow asking for the launcher hotkey, and where the answer
/// goes.
type LauncherRequest = oneshot::Sender<LauncherShortcut>;

/// How long the recorder waits for the backend's answer to a probe before
/// taking the combination.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// How long the first-run flow's request for the launcher hotkey waits for
/// the desktop: its dialog, then the bind. Past it the answer is "still
/// waiting", and the outcome is kept for the next time someone asks.
pub const LAUNCHER_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

impl Default for Control {
    fn default() -> Self {
        Self {
            generation: watch::Sender::new(0),
            capturing: watch::Sender::new(false),
            frontmost: watch::Sender::new(None),
            probes: std::sync::Mutex::new(None),
            launcher: watch::Sender::new(LauncherShortcut::Pending),
            requests: std::sync::Mutex::new(None),
        }
    }
}

impl Control {
    /// The focused application changed to `app` (its desktop id, when it
    /// was recognised): `AppRuntime::frontmostAppChanged`.
    pub fn set_frontmost(&self, app: Option<String>) {
        self.frontmost.send_if_modified(|current| {
            let changed = *current != app;
            *current = app;
            changed
        });
    }

    /// Whether the running backend would bind `combo`
    /// (`GlobalShortcutService::probeBind`): the desktop's refusal, or
    /// `None` when it would, when no backend runs, or when it does not
    /// answer in time.
    pub async fn probe(&self, combo: KeyCombo) -> Option<String> {
        let sender = self
            .probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()?;
        let (reply, answer) = oneshot::channel();
        sender.send((combo, reply)).ok()?;
        tokio::time::timeout(PROBE_TIMEOUT, answer)
            .await
            .ok()?
            .ok()
            .flatten()
    }

    fn serve_probes(&self, sender: Option<mpsc::UnboundedSender<Probe>>) {
        *self
            .probes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = sender;
    }

    /// The configuration changed: bind what it now says (`configChanged`).
    pub fn reload(&self) {
        self.generation.send_modify(|generation| *generation += 1);
    }

    /// The shortcut recorder started or stopped capturing.
    pub fn set_capturing(&self, capturing: bool) {
        self.capturing.send_if_modified(|current| {
            let changed = *current != capturing;
            *current = capturing;
            changed
        });
    }

    /// Where the launcher hotkey stands, as the service last learned it.
    #[must_use]
    pub fn launcher(&self) -> LauncherShortcut {
        self.launcher.borrow().clone()
    }

    /// Records where the launcher hotkey stands.
    pub fn set_launcher(&self, state: LauncherShortcut) {
        self.launcher.send_if_modified(|current| {
            let changed = *current != state;
            *current = state;
            changed
        });
    }

    /// Asks the desktop for the launcher hotkey now, and answers where it
    /// stands after: the first-run flow's Set Up Shortcut and Try Again.
    /// With no backend running it answers what is known, changing nothing.
    /// It waits for up to [`LAUNCHER_REQUEST_TIMEOUT`], then answers what is
    /// known while the request carries on.
    pub async fn request_launcher(&self) -> LauncherShortcut {
        let sender = self
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(sender) = sender else {
            return self.launcher();
        };
        let (reply, answer) = oneshot::channel();
        if sender.send(reply).is_err() {
            return self.launcher();
        }
        match tokio::time::timeout(LAUNCHER_REQUEST_TIMEOUT, answer).await {
            Ok(Ok(state)) => state,
            Ok(Err(_)) | Err(_) => self.launcher(),
        }
    }

    fn serve_requests(&self, sender: Option<mpsc::UnboundedSender<LauncherRequest>>) {
        *self
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = sender;
    }
}

/// Binds shortcuts on the desktop, one at a time
/// (`AbstractGlobalShortcutBackend`).
pub trait Backend: Send {
    /// Its name, for logs.
    fn name(&self) -> &'static str;

    /// Binds `binding`, or says why the desktop would not.
    fn bind(&mut self, binding: &Binding) -> impl Future<Output = Result<(), String>> + Send;

    /// Releases `id`.
    fn unbind(&mut self, id: &str) -> impl Future<Output = ()> + Send;

    /// Releases everything.
    fn unbind_all(&mut self) -> impl Future<Output = ()> + Send;

    /// Carries out the binds and unbinds since the last flush where the
    /// backend collects them, and names each id that then failed, with why.
    fn flush(&mut self) -> impl Future<Output = Vec<(String, String)>> + Send {
        async { Vec::new() }
    }

    /// Whether the desktop would bind `binding`: binds it and releases it at
    /// once (`probeBind`).
    fn probe(&mut self, binding: &Binding) -> impl Future<Output = Result<(), String>> + Send {
        async move {
            let bound = self.bind(binding).await;
            self.unbind(&binding.id).await;
            let failed = self.flush().await;
            bound?;
            match failed.into_iter().find(|(id, _)| *id == binding.id) {
                Some((_, reason)) => Err(reason),
                None => Ok(()),
            }
        }
    }

    /// What the desktop said about the launcher hotkey at the last flush,
    /// when the backend learns more than "bound" or "refused because".
    fn launcher_state(&self) -> Option<LauncherShortcut> {
        None
    }
}

/// A backend kept in step with the configuration.
#[derive(Debug)]
pub struct Service<B> {
    backend: B,
    reconciler: Reconciler,
    capturing: bool,
    inhibited: bool,
    deferred: bool,
    failures: BTreeMap<String, String>,
}

impl<B: Backend> Service<B> {
    /// Nothing bound yet.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            reconciler: Reconciler::default(),
            capturing: false,
            inhibited: false,
            deferred: false,
            failures: BTreeMap::new(),
        }
    }

    /// Binds nothing until [`Self::request`]: the first-run flow asks for
    /// the permission dialog rather than the engine at start.
    pub fn defer(&mut self) {
        self.deferred = true;
    }

    /// Whether binding waits for [`Self::request`].
    #[must_use]
    pub fn deferred(&self) -> bool {
        self.deferred
    }

    /// Asks the desktop for `desired` now: the end of a deferral, or a
    /// second ask after the launcher hotkey was refused or failed. Asking
    /// again for a hotkey that is bound changes nothing.
    pub async fn request(&mut self, desired: &BTreeMap<String, Binding>) {
        if std::mem::take(&mut self.deferred) {
            self.reconcile(desired).await;
            return;
        }
        if self.launcher(desired).is_granted() || self.capturing || self.inhibited {
            return;
        }
        self.backend.unbind_all().await;
        self.reconciler.clear();
        self.failures.clear();
        self.reconcile(desired).await;
    }

    /// Where the launcher hotkey stands after the last reconcile.
    #[must_use]
    pub fn launcher(&self, desired: &BTreeMap<String, Binding>) -> LauncherShortcut {
        if !desired.contains_key(LAUNCHER_ID) {
            return LauncherShortcut::Unset;
        }
        if self.deferred {
            return LauncherShortcut::NotRequested;
        }
        if let Some(state) = self.backend.launcher_state() {
            return state;
        }
        if self.reconciler.action(LAUNCHER_ID).is_some() {
            return LauncherShortcut::Granted { trigger: None };
        }
        match self.failures.get(LAUNCHER_ID) {
            Some(reason) => LauncherShortcut::Conflict {
                reason: format!("The desktop said: {reason}."),
            },
            None => LauncherShortcut::Pending,
        }
    }

    /// Whether the recorder is capturing.
    #[must_use]
    pub fn capturing(&self) -> bool {
        self.capturing
    }

    /// `updateInhibition`: every binding is released while the focused
    /// application is one `global_shortcuts.inhibit_apps` lists, so its keys
    /// reach it, and `desired` is bound again once it is not. While the
    /// recorder captures nothing is bound anyway, and its end binds or not
    /// as this says.
    pub async fn set_inhibited(&mut self, inhibited: bool, desired: &BTreeMap<String, Binding>) {
        if self.inhibited == inhibited {
            return;
        }
        self.inhibited = inhibited;
        tracing::info!(
            inhibited,
            "global shortcuts paused for the focused application"
        );
        if self.capturing {
            return;
        }
        if inhibited {
            self.backend.unbind_all().await;
            self.backend.flush().await;
            self.reconciler.clear();
        } else {
            self.reconcile(desired).await;
        }
    }

    /// `probeBind`: the desktop's refusal of `combo`, or `None` when it
    /// would bind it. Only while the recorder captures, when nothing else is
    /// bound for the probe to collide with.
    pub async fn probe(&mut self, combo: &KeyCombo) -> Option<String> {
        if !self.capturing {
            return None;
        }
        self.backend
            .probe(&global_shortcuts::probe(combo))
            .await
            .err()
    }

    /// The backend, for tests.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// `reconcile`: releases what is no longer wanted and binds what is new.
    /// Nothing while capturing, paused or deferred.
    pub async fn reconcile(&mut self, desired: &BTreeMap<String, Binding>) {
        if self.capturing || self.inhibited || self.deferred {
            return;
        }
        let plan = self.reconciler.plan(desired);
        if plan.is_empty() {
            return;
        }
        for id in &plan.unbind {
            self.backend.unbind(id).await;
        }
        for binding in &plan.bind {
            let result = self.backend.bind(binding).await;
            match &result {
                Ok(()) => {
                    self.failures.remove(&binding.id);
                }
                Err(reason) => {
                    tracing::warn!(
                        id = %binding.id, trigger = %binding.trigger, %reason,
                        "failed to bind a global shortcut"
                    );
                    self.failures.insert(binding.id.clone(), reason.clone());
                }
            }
            self.reconciler.bound(binding, &result);
        }
        for (id, reason) in self.backend.flush().await {
            if let Some(binding) = desired.get(&id) {
                tracing::warn!(
                    %id, trigger = %binding.trigger, %reason,
                    "failed to bind a global shortcut"
                );
                self.failures.insert(id.clone(), reason.clone());
                self.reconciler.bound(binding, &Err(reason));
            }
        }
        tracing::info!(
            backend = self.backend.name(),
            bound = ?self.reconciler.live(),
            "global shortcuts bound"
        );
    }

    /// `setCapturing`: every binding is released while the recorder
    /// captures, so it sees the combination rather than the desktop, and
    /// `desired` is bound again after.
    pub async fn set_capturing(&mut self, capturing: bool, desired: &BTreeMap<String, Binding>) {
        if self.capturing == capturing {
            return;
        }
        self.capturing = capturing;
        if capturing {
            self.backend.unbind_all().await;
            self.backend.flush().await;
            self.reconciler.clear();
        } else {
            self.reconcile(desired).await;
        }
    }

    /// What pressing `id` does (`onActivated`); `None` for an id that is
    /// not bound.
    #[must_use]
    pub fn action(&self, id: &str) -> Option<Action> {
        self.reconciler.action(id).cloned()
    }
}

/// Carries out a pressed shortcut's action.
pub async fn activate(state: &Arc<RwLock<EngineState>>, action: Action) -> Response {
    match action {
        Action::ToggleLauncher => {
            let slot = state.read().await.window_slot();
            forward(&slot, WindowCommand::Toggle, "toggle a window").await
        }
        Action::RunCommand(id) => crate::serve::launch_entrypoint(state, id).await,
    }
}

/// What the configuration asks to be bound, each item named by its title.
pub async fn desired(
    state: &Arc<RwLock<EngineState>>,
    config: &Config,
) -> BTreeMap<String, Binding> {
    let state = state.read().await;
    global_shortcuts::desired(config, |id| {
        state.index().root(id).map(|root| root.title.clone())
    })
}

/// Reads `compass.json`, or the defaults when it cannot be read.
async fn load_config() -> Config {
    match tokio::task::spawn_blocking(Config::load).await {
        Ok(Ok(config)) => config,
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "global shortcuts: using the default configuration");
            Config::default()
        }
        Err(_) => Config::default(),
    }
}

/// Records where the launcher hotkey stands, unless every binding is
/// released for the recorder or a listed application, when the last answer
/// is still the one that will hold once they are bound again.
fn report<B: Backend>(
    control: &Control,
    service: &Service<B>,
    desired: &BTreeMap<String, Binding>,
) {
    if !service.capturing && !service.inhibited {
        control.set_launcher(service.launcher(desired));
    }
}

/// Runs `service` until its presses stop: binds what `config` reads, again
/// on each reload, suspends while capturing, acts on each press, and asks
/// the desktop for the launcher hotkey when the first-run flow does.
pub async fn serve<B, F, Fut>(
    state: Arc<RwLock<EngineState>>,
    mut service: Service<B>,
    mut presses: mpsc::UnboundedReceiver<String>,
    config: F,
) where
    B: Backend,
    F: Fn() -> Fut,
    Fut: Future<Output = Config>,
{
    let control = state.read().await.global_shortcuts();
    let mut generation = control.generation.subscribe();
    let mut capturing = control.capturing.subscribe();
    let mut frontmost = control.frontmost.subscribe();
    let (probes_tx, mut probes) = mpsc::unbounded_channel::<Probe>();
    control.serve_probes(Some(probes_tx));
    let (requests_tx, mut requests) = mpsc::unbounded_channel::<LauncherRequest>();
    control.serve_requests(Some(requests_tx));
    let paused = |config: &Config, frontmost: &watch::Receiver<Option<String>>| {
        global_shortcuts::inhibited(
            config.global_shortcuts().inhibit_apps(),
            frontmost.borrow().as_deref(),
        )
    };
    {
        let config = config().await;
        let desired = desired(&state, &config).await;
        if !service.deferred() {
            control.set_launcher(LauncherShortcut::Pending);
        }
        service
            .set_inhibited(paused(&config, &frontmost), &desired)
            .await;
        service.reconcile(&desired).await;
        report(&control, &service, &desired);
    }
    loop {
        tokio::select! {
            press = presses.recv() => {
                let Some(id) = press else { break };
                if let Some(action) = service.action(&id) {
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        if let Response::Error(err) = activate(&state, action).await {
                            tracing::warn!(%id, error = %err.message, "a global shortcut did nothing");
                        }
                    });
                }
            }
            changed = generation.changed() => {
                if changed.is_err() {
                    break;
                }
                let config = config().await;
                let desired = desired(&state, &config).await;
                service.set_inhibited(paused(&config, &frontmost), &desired).await;
                service.reconcile(&desired).await;
                report(&control, &service, &desired);
            }
            changed = capturing.changed() => {
                if changed.is_err() {
                    break;
                }
                let on = *capturing.borrow_and_update();
                let desired = desired(&state, &config().await).await;
                service.set_capturing(on, &desired).await;
                report(&control, &service, &desired);
            }
            changed = frontmost.changed() => {
                if changed.is_err() {
                    break;
                }
                frontmost.borrow_and_update();
                let config = config().await;
                let desired = desired(&state, &config).await;
                service.set_inhibited(paused(&config, &frontmost), &desired).await;
                report(&control, &service, &desired);
            }
            Some((combo, reply)) = probes.recv() => {
                // The recorder says it captures before it probes; take that
                // first if this loop has not yet.
                let on = *capturing.borrow_and_update();
                if on != service.capturing() {
                    let desired = desired(&state, &config().await).await;
                    service.set_capturing(on, &desired).await;
                }
                let _ = reply.send(service.probe(&combo).await);
            }
            Some(reply) = requests.recv() => {
                let desired = desired(&state, &config().await).await;
                control.set_launcher(LauncherShortcut::Pending);
                service.request(&desired).await;
                let answer = service.launcher(&desired);
                tracing::info!(state = ?answer, "the launcher hotkey, as the first-run flow asked");
                control.set_launcher(answer.clone());
                let _ = reply.send(answer);
            }
        }
    }
    control.serve_probes(None);
    control.serve_requests(None);
    tracing::info!("global shortcuts: the backend went away; they are unbound");
}

/// Binds the configuration's global shortcuts for as long as the engine
/// runs.
pub async fn run(state: Arc<RwLock<EngineState>>) {
    tokio::spawn(follow_frontmost(Arc::clone(&state)));
    let (presses_tx, presses) = mpsc::unbounded_channel();
    if let Some(backend) = connect_wayland(presses_tx.clone()).await {
        serve(state, Service::new(backend), presses, load_config).await;
        return;
    }
    if let Some(backend) = connect_portal(presses_tx).await {
        let mut service = Service::new(backend);
        if crate::onboarding_due().is_some() {
            tracing::info!(
                "the first-run flow is due; the launcher hotkey waits for it to ask the desktop"
            );
            service.defer();
        }
        serve(state, service, presses, load_config).await;
        return;
    }
    state
        .read()
        .await
        .global_shortcuts()
        .set_launcher(LauncherShortcut::Unavailable);
    let hint = match crate::wlroots::detect().await {
        Some(wlroots) => compass_wayland::hotkey::manual_binding_hint(wlroots.desktop.as_deref()),
        None => "bind `compass toggle` to a key in your desktop's settings".to_owned(),
    };
    tracing::warn!("no global shortcut backend; {hint}. `compass doctor` explains the options");
}

/// Tells the service which application is focused, now and each time focus
/// moves, from the window-manager providers ([`crate::frontmost`]).
async fn follow_frontmost(state: Arc<RwLock<EngineState>>) {
    let report = |state: Arc<RwLock<EngineState>>| async move {
        let app = crate::frontmost::frontmost(&state).await.app_id;
        state.read().await.global_shortcuts().set_frontmost(app);
    };
    report(Arc::clone(&state)).await;
    let watched = Arc::clone(&state);
    crate::frontmost::watch(watched, || report(Arc::clone(&state))).await;
}

/// `xx-hotkey-v1` or `vicinae-hotkey-v1`, on any Wayland compositor but
/// GNOME's (whose path is the portal, whatever Mutter grows).
async fn connect_wayland(presses: mpsc::UnboundedSender<String>) -> Option<WaylandBackend> {
    use compass_wayland::compositor::{current_desktop, desktop_is_gnome};
    use compass_wayland::hotkey::{HotkeyClient, HotkeyError, HotkeyEvent};

    if std::env::var_os("WAYLAND_DISPLAY").is_none()
        || desktop_is_gnome(current_desktop().as_deref())
    {
        return None;
    }
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let client = match tokio::task::spawn_blocking(move || {
        HotkeyClient::connect(compass_ui::APP_ID, events_tx)
    })
    .await
    {
        Ok(Ok(client)) => client,
        Ok(Err(HotkeyError::Unsupported)) => {
            tracing::info!("the compositor has neither hotkey protocol; trying the portal");
            return None;
        }
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "the compositor's hotkey protocol failed");
            return None;
        }
        Err(err) => {
            tracing::warn!(error = %err, "the hotkey task failed");
            return None;
        }
    };
    tracing::info!(
        protocol = client.protocol().name(),
        "global shortcuts over the compositor's hotkey protocol"
    );
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                HotkeyEvent::Triggered { id, .. } => {
                    if presses.send(id).is_err() {
                        break;
                    }
                }
                HotkeyEvent::Revoked { id, message } => tracing::warn!(
                    %id, message,
                    "the compositor revoked a global shortcut"
                ),
            }
        }
    });
    Some(WaylandBackend {
        client,
        bound: BTreeMap::new(),
    })
}

/// The GlobalShortcuts portal, where the session bus has one.
async fn connect_portal(presses: mpsc::UnboundedSender<String>) -> Option<PortalBackend> {
    let portals = match Portals::connect(PortalConfig::default()).await {
        Ok(portals) => portals,
        Err(err) => {
            tracing::warn!(error = %err, "no portal connection for global shortcuts");
            return None;
        }
    };
    let availability = portals.capabilities().global_shortcuts;
    if !availability.is_available() {
        tracing::warn!(%availability, "no GlobalShortcuts portal");
        return None;
    }
    let (events_tx, mut events) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let Some(id) = portal_press(event)
                && presses.send(id).is_err()
            {
                break;
            }
        }
    });
    Some(PortalBackend::new(ShortcutBinder::new(portals, events_tx)))
}

/// The id a portal event presses, if it is a press.
///
/// A release is not one: toggling on it too would open the launcher on the
/// press and close it on the release, which looks exactly like a hotkey
/// that does nothing.
pub fn portal_press(event: ShortcutEvent) -> Option<String> {
    match event {
        ShortcutEvent::Activated { id, .. } => Some(id),
        ShortcutEvent::Changed { shortcuts } => {
            for bound in shortcuts {
                tracing::info!(
                    id = %bound.id,
                    trigger = bound.trigger_description,
                    "the desktop changed a shortcut binding"
                );
            }
            None
        }
        _ => None,
    }
}

/// The Wayland hotkey protocols: one hotkey object per shortcut.
#[derive(Debug)]
pub struct WaylandBackend {
    client: compass_wayland::hotkey::HotkeyClient,
    bound: BTreeMap<String, compass_wayland::hotkey::Hotkey>,
}

impl Backend for WaylandBackend {
    fn name(&self) -> &'static str {
        self.client.protocol().name()
    }

    async fn bind(&mut self, binding: &Binding) -> Result<(), String> {
        let Some(keysym) = global_shortcuts::keysym(&binding.combo.key) else {
            return Err("Unsupported trigger key".to_owned());
        };
        let request = compass_wayland::hotkey::HotkeyRequest {
            id: binding.id.clone(),
            description: binding.description.clone(),
            keysym,
            modifiers: global_shortcuts::modifier_mask(binding.combo.modifiers),
        };
        let hotkey = self
            .client
            .bind(&request)
            .await
            .map_err(|err| err.to_string())?;
        self.bound.insert(binding.id.clone(), hotkey);
        Ok(())
    }

    async fn unbind(&mut self, id: &str) {
        self.bound.remove(id);
    }

    async fn unbind_all(&mut self) {
        self.bound.clear();
    }
}

/// The portal: the binds of one reconcile, bound as one set on flush.
#[derive(Debug)]
pub struct PortalBackend {
    binder: ShortcutBinder,
    set: BTreeMap<String, ShortcutDescriptor>,
    dirty: bool,
    launcher: Option<LauncherShortcut>,
}

impl PortalBackend {
    /// Nothing bound yet.
    #[must_use]
    pub fn new(binder: ShortcutBinder) -> Self {
        Self {
            binder,
            set: BTreeMap::new(),
            dirty: false,
            launcher: None,
        }
    }
}

/// What the portal's answer to binding a set that holds the launcher hotkey
/// means for it: bound with the key the desktop describes, left without a
/// key (something else has it), declined, unavailable, or failed.
#[must_use]
pub fn launcher_outcome(result: &Result<ShortcutsOutcome, PortalError>) -> LauncherShortcut {
    match result {
        Ok(ShortcutsOutcome::Granted { shortcuts }) => {
            match shortcuts.iter().find(|bound| bound.id == LAUNCHER_ID) {
                Some(bound) if !bound.trigger_description.trim().is_empty() => {
                    LauncherShortcut::Granted {
                        trigger: Some(bound.trigger_description.clone()),
                    }
                }
                Some(_) => LauncherShortcut::Conflict {
                    reason: "Your desktop did not assign it a key, which usually means another shortcut already uses it.".to_owned(),
                },
                None => LauncherShortcut::Conflict {
                    reason: "Your desktop left it out of the shortcuts it bound.".to_owned(),
                },
            }
        }
        Ok(ShortcutsOutcome::Denied) => LauncherShortcut::Refused,
        Ok(_) => LauncherShortcut::Failed {
            reason: "the desktop ended the request without saying why.".to_owned(),
        },
        Err(error) if error.is_unavailable() => LauncherShortcut::Unavailable,
        Err(error) => LauncherShortcut::Failed {
            reason: format!("{error}."),
        },
    }
}

/// The portal's descriptor for `binding`: its id, its title, and the
/// trigger it would like, which the desktop may override.
#[must_use]
pub fn portal_descriptor(binding: &Binding) -> ShortcutDescriptor {
    let descriptor = ShortcutDescriptor::new(binding.id.clone(), binding.description.clone());
    match global_shortcuts::portal_trigger(&binding.combo)
        .and_then(|trigger| Trigger::parse(&trigger).ok())
    {
        Some(trigger) => descriptor.with_trigger(trigger),
        None => descriptor,
    }
}

impl Backend for PortalBackend {
    fn name(&self) -> &'static str {
        "GlobalShortcuts portal"
    }

    async fn bind(&mut self, binding: &Binding) -> Result<(), String> {
        self.set
            .insert(binding.id.clone(), portal_descriptor(binding));
        self.dirty = true;
        Ok(())
    }

    async fn unbind(&mut self, id: &str) {
        self.dirty |= self.set.remove(id).is_some();
    }

    async fn unbind_all(&mut self) {
        self.set.clear();
        self.dirty = true;
    }

    /// The portal binds a whole set behind the desktop's own dialog, where
    /// the person picks the trigger, so there is nothing to ask ahead of it:
    /// a probe would open that dialog for a throwaway shortcut.
    async fn probe(&mut self, _binding: &Binding) -> Result<(), String> {
        Ok(())
    }

    fn launcher_state(&self) -> Option<LauncherShortcut> {
        self.launcher.clone()
    }

    async fn flush(&mut self) -> Vec<(String, String)> {
        if !std::mem::take(&mut self.dirty) {
            return Vec::new();
        }
        let set: Vec<ShortcutDescriptor> = self.set.values().cloned().collect();
        let failed = |reason: &str| {
            set.iter()
                .map(|descriptor| (descriptor.id.clone(), reason.to_owned()))
                .collect()
        };
        let result = self.binder.apply(&set).await;
        self.launcher = self
            .set
            .contains_key(LAUNCHER_ID)
            .then(|| launcher_outcome(&result));
        match result {
            Ok(ShortcutsOutcome::Granted { shortcuts }) => {
                for bound in &shortcuts {
                    tracing::info!(
                        id = %bound.id,
                        trigger = bound.trigger_description,
                        "the desktop bound a global shortcut"
                    );
                }
                set.iter()
                    .filter(|descriptor| !shortcuts.iter().any(|bound| bound.id == descriptor.id))
                    .map(|descriptor| {
                        (
                            descriptor.id.clone(),
                            "the desktop did not bind it".to_owned(),
                        )
                    })
                    .collect()
            }
            Ok(ShortcutsOutcome::Denied) => failed("the desktop's permission dialog was dismissed"),
            Ok(_) => failed("the desktop refused the shortcuts"),
            Err(err) => failed(&err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;
    use std::time::Duration;

    use compass_core::key_combo::KeyCombo;
    use compass_core::{AppIndex, JsonFrecencyStore, SystemClock};
    use compass_ipc::{ErrorKind, SocketPath};

    /// An engine with no window attached.
    fn engine() -> Arc<RwLock<EngineState>> {
        let clock = Arc::new(SystemClock);
        Arc::new(RwLock::new(EngineState::with_index(
            AppIndex::builder().build(),
            Box::new(JsonFrecencyStore::in_memory(clock)),
            SocketPath::exact("/nonexistent/ipc.sock"),
            10,
        )))
    }

    /// Records what it is asked, and refuses what `refuse` names.
    #[derive(Debug, Default, Clone)]
    struct Fake {
        log: Arc<Mutex<Vec<String>>>,
        refuse: Vec<String>,
    }

    impl Fake {
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Backend for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }

        async fn bind(&mut self, binding: &Binding) -> Result<(), String> {
            self.log
                .lock()
                .unwrap()
                .push(format!("bind {} {}", binding.id, binding.trigger));
            if self.refuse.contains(&binding.id) {
                Err("taken".to_owned())
            } else {
                Ok(())
            }
        }

        async fn unbind(&mut self, id: &str) {
            self.log.lock().unwrap().push(format!("unbind {id}"));
        }

        async fn unbind_all(&mut self) {
            self.log.lock().unwrap().push("unbind all".to_owned());
        }
    }

    fn config(json: &str) -> Config {
        serde_json::from_str(json).expect("a configuration")
    }

    fn wanted(json: &str) -> BTreeMap<String, Binding> {
        global_shortcuts::desired(&config(json), |_| None)
    }

    #[tokio::test]
    async fn the_launcher_hotkey_and_a_commands_shortcut_are_bound_and_rebound_when_changed() {
        let fake = Fake::default();
        let mut service = Service::new(fake.clone());
        service
            .reconcile(&wanted(
                r#"{"providers": {"clipboard": {"entrypoints": {"history": {"shortcut": "super+shift+V"}}}}}"#,
            ))
            .await;
        assert_eq!(
            fake.log(),
            [
                "bind clipboard:history super+shift+V",
                "bind toggle super+space"
            ]
        );
        assert_eq!(service.action("toggle"), Some(Action::ToggleLauncher));
        assert_eq!(
            service.action("clipboard:history"),
            Some(Action::RunCommand("clipboard:history".to_owned()))
        );

        // The settings view changed the launcher hotkey.
        service
            .reconcile(&wanted(
                r#"{"launcher": {"hotkey": "alt+SPACE"},
                    "providers": {"clipboard": {"entrypoints": {"history": {"shortcut": "super+shift+V"}}}}}"#,
            ))
            .await;
        assert_eq!(
            fake.log()[2..],
            ["unbind toggle", "bind toggle alt+SPACE"],
            "only what changed is touched"
        );
    }

    #[tokio::test]
    async fn a_refused_shortcut_does_nothing_when_pressed() {
        let fake = Fake {
            refuse: vec!["a:b".to_owned()],
            ..Fake::default()
        };
        let mut service = Service::new(fake);
        service
            .reconcile(&wanted(
                r#"{"providers": {"a": {"entrypoints": {"b": {"shortcut": "super+B"}}}}}"#,
            ))
            .await;
        assert_eq!(service.action("a:b"), None);
        assert_eq!(service.action("toggle"), Some(Action::ToggleLauncher));
    }

    #[tokio::test]
    async fn capturing_releases_everything_and_binds_it_again_after() {
        let fake = Fake::default();
        let mut service = Service::new(fake.clone());
        let desired = wanted("{}");
        service.reconcile(&desired).await;
        service.set_capturing(true, &desired).await;
        assert_eq!(service.action("toggle"), None, "the recorder gets the keys");
        service.reconcile(&desired).await;
        service.set_capturing(false, &desired).await;
        assert_eq!(
            fake.log(),
            [
                "bind toggle super+space",
                "unbind all",
                "bind toggle super+space"
            ]
        );
        assert_eq!(service.action("toggle"), Some(Action::ToggleLauncher));
    }

    #[tokio::test]
    async fn pressing_the_launcher_hotkey_reaches_the_window() {
        // With no window attached the engine refuses by name, which is the
        // signature of "this press went to the window".
        let state = engine();
        let response = activate(&state, Action::ToggleLauncher).await;
        assert!(
            matches!(response, Response::Error(ref err) if err.kind == ErrorKind::Unsupported),
            "{response:?}"
        );
    }

    #[tokio::test]
    async fn pressing_a_commands_shortcut_launches_it_as_cmd_launch_does() {
        let state = engine();
        let response = activate(&state, Action::RunCommand("nope:missing".to_owned())).await;
        assert!(
            matches!(response, Response::Error(ref err)
                if err.kind == ErrorKind::BadRequest && err.message.contains("nope:missing")),
            "{response:?}"
        );
    }

    #[tokio::test]
    async fn a_reload_binds_what_the_configuration_now_says() {
        let state = engine();
        let fake = Fake::default();
        let (presses_tx, presses) = mpsc::unbounded_channel();
        let current = Arc::new(Mutex::new(config("{}")));
        let read = {
            let current = Arc::clone(&current);
            move || {
                let config = current.lock().unwrap().clone();
                async move { config }
            }
        };
        let task = tokio::spawn(serve(
            Arc::clone(&state),
            Service::new(fake.clone()),
            presses,
            read,
        ));
        let until = |want: usize| {
            let fake = fake.clone();
            async move {
                for _ in 0..200 {
                    if fake.log().len() >= want {
                        return fake.log();
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                panic!("the service did not bind: {:?}", fake.log());
            }
        };
        assert_eq!(until(1).await, ["bind toggle super+space"]);

        *current.lock().unwrap() = config(r#"{"launcher": {"hotkey": "alt+F1"}}"#);
        state.read().await.global_shortcuts().reload();
        assert_eq!(until(3).await[1..], ["unbind toggle", "bind toggle alt+F1"]);

        state.read().await.global_shortcuts().set_capturing(true);
        assert_eq!(until(4).await[3], "unbind all");

        drop(presses_tx);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the service ends with its presses")
            .expect("no panic");
    }

    #[tokio::test]
    async fn a_listed_application_in_front_releases_every_shortcut_until_it_leaves() {
        let fake = Fake::default();
        let mut service = Service::new(fake.clone());
        let desired = wanted("{}");
        service.reconcile(&desired).await;
        service.set_inhibited(true, &desired).await;
        assert_eq!(
            service.action("toggle"),
            None,
            "the keys reach the application"
        );
        service.reconcile(&desired).await;
        service.set_inhibited(true, &desired).await;
        service.set_inhibited(false, &desired).await;
        assert_eq!(
            fake.log(),
            [
                "bind toggle super+space",
                "unbind all",
                "bind toggle super+space"
            ],
            "nothing is bound while paused, and it is bound once after"
        );
        assert_eq!(service.action("toggle"), Some(Action::ToggleLauncher));

        // Paused while the recorder captures: its end binds nothing.
        service.set_capturing(true, &desired).await;
        service.set_inhibited(true, &desired).await;
        service.set_capturing(false, &desired).await;
        assert_eq!(fake.log()[3..], ["unbind all"]);
        assert_eq!(service.action("toggle"), None);
    }

    #[tokio::test]
    async fn a_probe_binds_and_releases_the_combination_and_says_why_it_was_refused() {
        let fake = Fake {
            refuse: vec![global_shortcuts::PROBE_ID.to_owned()],
            ..Fake::default()
        };
        let mut service = Service::new(fake.clone());
        let combo = KeyCombo::parse("super+Q").unwrap();
        assert_eq!(
            service.probe(&combo).await,
            None,
            "not capturing: nothing is asked"
        );
        assert!(fake.log().is_empty());

        service.set_capturing(true, &wanted("{}")).await;
        assert_eq!(service.probe(&combo).await.as_deref(), Some("taken"));
        assert_eq!(
            fake.log()[1..],
            ["bind @probe super+Q", "unbind @probe"],
            "bound and released at once"
        );

        let mut taking = Service::new(Fake::default());
        taking.set_capturing(true, &wanted("{}")).await;
        assert_eq!(taking.probe(&combo).await, None);
    }

    /// Starts `serve` over `fake` with `json` as the configuration, and
    /// waits for its first bind.
    async fn serving(
        state: &Arc<RwLock<EngineState>>,
        fake: &Fake,
        json: &str,
    ) -> (mpsc::UnboundedSender<String>, tokio::task::JoinHandle<()>) {
        let (presses_tx, presses) = mpsc::unbounded_channel();
        let json = json.to_owned();
        let task = tokio::spawn(serve(
            Arc::clone(state),
            Service::new(fake.clone()),
            presses,
            move || {
                let config = config(&json);
                async move { config }
            },
        ));
        until(fake, |log| !log.is_empty()).await;
        (presses_tx, task)
    }

    async fn until(fake: &Fake, done: impl Fn(&[String]) -> bool) -> Vec<String> {
        for _ in 0..400 {
            let log = fake.log();
            if done(&log) {
                return log;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the service never got there: {:?}", fake.log());
    }

    #[tokio::test]
    async fn the_frontmost_application_pauses_the_shortcuts_the_configuration_names() {
        let state = engine();
        let fake = Fake::default();
        let control = state.read().await.global_shortcuts();
        let (presses_tx, task) = serving(
            &state,
            &fake,
            r#"{"global_shortcuts": {"inhibit_apps": ["org.gnome.Boxes.desktop"]}}"#,
        )
        .await;

        control.set_frontmost(Some("firefox.desktop".into()));
        control.set_frontmost(Some("org.gnome.Boxes.desktop".into()));
        until(&fake, |log| log.last().is_some_and(|l| l == "unbind all")).await;
        control.set_frontmost(Some("firefox.desktop".into()));
        let log = until(&fake, |log| log.len() == 3).await;
        assert_eq!(
            log,
            [
                "bind toggle super+space",
                "unbind all",
                "bind toggle super+space"
            ]
        );

        drop(presses_tx);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the service ends with its presses")
            .expect("no panic");
    }

    #[tokio::test]
    async fn the_recorders_probe_reaches_the_backend_over_ipc() {
        let state = engine();
        let probe = |trigger: &str| {
            crate::serve::handle(
                &state,
                compass_ipc::Request::ProbeShortcut {
                    trigger: trigger.to_owned(),
                },
            )
        };
        assert_eq!(
            probe("super+Q").await,
            Response::ShortcutProbe { refusal: None },
            "no backend: nothing to refuse it"
        );
        assert!(matches!(
            probe("not a key").await,
            Response::Error(ref err) if err.kind == ErrorKind::BadRequest
        ));

        let fake = Fake {
            refuse: vec![global_shortcuts::PROBE_ID.to_owned()],
            ..Fake::default()
        };
        let (presses_tx, task) = serving(&state, &fake, "{}").await;
        let _ = crate::serve::handle(
            &state,
            compass_ipc::Request::ShortcutCapture { capturing: true },
        )
        .await;
        assert_eq!(
            probe("super+Q").await,
            Response::ShortcutProbe {
                refusal: Some("taken".to_owned())
            }
        );
        assert!(fake.log().contains(&"bind @probe super+Q".to_owned()));

        drop(presses_tx);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the service ends with its presses")
            .expect("no panic");
        assert_eq!(
            probe("super+Q").await,
            Response::ShortcutProbe { refusal: None },
            "the service is gone"
        );
    }

    #[tokio::test]
    async fn the_recorders_capture_reaches_the_service_over_ipc() {
        let state = engine();
        let mut capturing = state.read().await.global_shortcuts().capturing.subscribe();
        let answer = crate::serve::handle(
            &state,
            compass_ipc::Request::ShortcutCapture { capturing: true },
        )
        .await;
        assert_eq!(answer, Response::Ack);
        assert!(capturing.has_changed().unwrap());
        assert!(*capturing.borrow_and_update());
        let _ = crate::serve::handle(
            &state,
            compass_ipc::Request::ShortcutCapture { capturing: true },
        )
        .await;
        assert!(!capturing.has_changed().unwrap(), "said once per change");
    }

    #[test]
    fn a_release_is_not_a_press() {
        let released = ShortcutEvent::Deactivated {
            id: "toggle".to_owned(),
            timestamp: Duration::from_secs(1),
        };
        assert_eq!(portal_press(released), None);
        assert_eq!(
            portal_press(ShortcutEvent::Changed { shortcuts: vec![] }),
            None
        );
        let pressed = ShortcutEvent::Activated {
            id: "clipboard:history".to_owned(),
            timestamp: Duration::from_secs(1),
            activation_token: None,
        };
        assert_eq!(portal_press(pressed).as_deref(), Some("clipboard:history"));
    }

    #[test]
    fn the_portal_is_asked_for_the_configured_trigger() {
        let binding = Binding {
            id: "toggle".to_owned(),
            trigger: "super+SPACE".to_owned(),
            combo: KeyCombo::parse("super+SPACE").unwrap(),
            description: global_shortcuts::LAUNCHER_DESCRIPTION.to_owned(),
            action: Action::ToggleLauncher,
        };
        let descriptor = portal_descriptor(&binding);
        assert_eq!(descriptor.id, "toggle");
        assert_eq!(
            descriptor
                .preferred_trigger
                .map(|t| t.to_string())
                .as_deref(),
            Some("LOGO+space")
        );
    }

    #[tokio::test]
    async fn a_deferred_service_binds_nothing_until_the_flow_asks() {
        let fake = Fake::default();
        let mut service = Service::new(fake.clone());
        service.defer();
        let desired = wanted("{}");
        service.reconcile(&desired).await;
        assert!(fake.log().is_empty(), "no dialog at start");
        assert_eq!(service.launcher(&desired), LauncherShortcut::NotRequested);
        assert_eq!(service.action("toggle"), None);

        service.request(&desired).await;
        assert_eq!(fake.log(), ["bind toggle super+space"]);
        assert_eq!(
            service.launcher(&desired),
            LauncherShortcut::Granted { trigger: None }
        );
        service.request(&desired).await;
        assert_eq!(fake.log().len(), 1, "a bound hotkey is not asked for again");
        assert_eq!(
            service.launcher(&wanted(r#"{"launcher": {"hotkey": ""}}"#)),
            LauncherShortcut::Unset
        );
    }

    #[tokio::test]
    async fn a_refused_launcher_hotkey_is_asked_for_again_when_the_flow_asks() {
        let fake = Fake {
            refuse: vec!["toggle".to_owned()],
            ..Fake::default()
        };
        let mut service = Service::new(fake.clone());
        let desired = wanted("{}");
        service.reconcile(&desired).await;
        assert!(matches!(
            service.launcher(&desired),
            LauncherShortcut::Conflict { ref reason } if reason.contains("taken")
        ));
        service.reconcile(&desired).await;
        assert_eq!(fake.log().len(), 1, "a reload does not ask again");
        service.request(&desired).await;
        assert_eq!(
            fake.log(),
            [
                "bind toggle super+space",
                "unbind all",
                "bind toggle super+space"
            ],
            "Try Again does"
        );
    }

    #[test]
    fn the_portals_answers_become_the_launcher_hotkeys_state() {
        use compass_portals::{Availability, BoundShortcut, Unavailable};
        let bound = |trigger: &str| {
            Ok(ShortcutsOutcome::Granted {
                shortcuts: vec![BoundShortcut {
                    id: "toggle".to_owned(),
                    description: global_shortcuts::LAUNCHER_DESCRIPTION.to_owned(),
                    trigger_description: trigger.to_owned(),
                }],
            })
        };
        assert_eq!(
            launcher_outcome(&bound("Super+Space")),
            LauncherShortcut::Granted {
                trigger: Some("Super+Space".to_owned())
            }
        );
        assert!(matches!(
            launcher_outcome(&bound("")),
            LauncherShortcut::Conflict { .. }
        ));
        assert!(matches!(
            launcher_outcome(&Ok(ShortcutsOutcome::Granted { shortcuts: vec![] })),
            LauncherShortcut::Conflict { .. }
        ));
        assert_eq!(
            launcher_outcome(&Ok(ShortcutsOutcome::Denied)),
            LauncherShortcut::Refused
        );
        assert!(matches!(
            launcher_outcome(&Ok(ShortcutsOutcome::Refused)),
            LauncherShortcut::Failed { .. }
        ));
        assert_eq!(
            launcher_outcome(&Err(PortalError::Unavailable {
                interface: "org.freedesktop.portal.GlobalShortcuts",
                availability: Availability::Unavailable {
                    reason: Unavailable::InterfaceMissing,
                },
            })),
            LauncherShortcut::Unavailable
        );
        assert!(matches!(
            launcher_outcome(&Err(PortalError::Timeout {
                method: "BindShortcuts",
                timeout: Duration::from_secs(30),
            })),
            LauncherShortcut::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn the_first_run_flow_reads_and_asks_for_the_hotkey_over_ipc() {
        let state = engine();
        let ask = |request: bool| {
            crate::serve::handle(&state, compass_ipc::Request::LauncherShortcut { request })
        };
        let launcher = |state: LauncherShortcut| crate::permissions::launcher_response(state);
        assert_eq!(
            ask(true).await,
            launcher(LauncherShortcut::Pending),
            "no service yet: what is known"
        );

        let fake = Fake::default();
        let (presses_tx, presses) = mpsc::unbounded_channel();
        let mut service = Service::new(fake.clone());
        service.defer();
        let task = tokio::spawn(serve(Arc::clone(&state), service, presses, || async {
            config("{}")
        }));
        let control = state.read().await.global_shortcuts();
        for _ in 0..400 {
            if control.launcher() == LauncherShortcut::NotRequested {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(ask(false).await, launcher(LauncherShortcut::NotRequested));
        assert!(fake.log().is_empty());
        assert_eq!(
            ask(true).await,
            launcher(LauncherShortcut::Granted { trigger: None })
        );
        assert_eq!(fake.log(), ["bind toggle super+space"]);
        assert_eq!(
            ask(false).await,
            launcher(LauncherShortcut::Granted { trigger: None })
        );

        drop(presses_tx);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the service ends with its presses")
            .expect("no panic");
    }
}
