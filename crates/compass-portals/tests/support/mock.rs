//! A mock `org.freedesktop.portal.Desktop` served with `zbus`.
//!
//! `zbus` serves as well as it consumes, so the mock implements the very
//! interfaces `ashpd` was generated against. Everything below goes over a real
//! unix socket on a real `dbus-daemon`; nothing is stubbed at the Rust level.
//!
//! The mock is configurable along the axes that actually matter in the field:
//! which interfaces exist at all (the wlroots case is "frontend yes,
//! GlobalShortcuts no"), what version they claim, and how the backend answers
//! a request — success, the user denying the dialog, an unexplained refusal,
//! silence, or a reply of the wrong shape.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use zbus::message::Header;
use zbus::names::UniqueName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Type, as_value};
use zbus::{Connection, interface};

use compass_portals::{DESKTOP_DESTINATION, DESKTOP_PATH};

const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";

/// How the mock backend answers a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Behaviour {
    /// Emit `Response(0, results)`.
    #[default]
    Succeed,
    /// Emit `Response(1, {})` — the portal's code for "the user said no".
    Deny,
    /// Emit `Response(2, {})` — ended unsuccessfully, no reason given.
    Other,
    /// Return the request handle and never emit `Response` at all. This is
    /// what a wedged backend, or a dialog nobody ever answers, looks like.
    Silent,
    /// Emit `Response(0, results)` where `results` has the wrong shape.
    Malformed,
}

/// Everything a test wants to inspect about the mock.
#[derive(Debug, Default)]
pub struct MockState {
    /// Session object paths handed out by `CreateSession`.
    pub sessions: Vec<OwnedObjectPath>,
    /// `(id, description, preferred_trigger)` for every shortcut ever bound.
    pub bound: Vec<(String, String, Option<String>)>,
    /// Method names invoked, in order.
    pub calls: Vec<&'static str>,
    /// URIs passed to `OpenURI`.
    pub opened_uris: Vec<String>,
    /// Bytes read back from the descriptors passed to `OpenURI.OpenFile`.
    pub opened_fd_contents: Vec<String>,
    /// Titles passed to `FileChooser.OpenFile`.
    pub chooser_titles: Vec<String>,
    /// Whatever the last `FileChooser.OpenFile` options dict contained.
    pub chooser_options: HashMap<String, OwnedValue>,
    /// URIs the mock will answer the next `FileChooser.OpenFile` with.
    pub chooser_reply: Vec<String>,
}

/// Shared handle to [`MockState`].
pub type SharedState = Arc<Mutex<MockState>>;

fn lock(state: &SharedState) -> std::sync::MutexGuard<'_, MockState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// Portal request/response plumbing
// ---------------------------------------------------------------------------

fn escape(sender: &UniqueName<'_>) -> String {
    sender.as_str().trim_start_matches(':').replace('.', "_")
}

fn token(options: &HashMap<String, OwnedValue>, key: &str) -> String {
    options
        .get(key)
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or("missing_token")
        .to_owned()
}

fn handle_path(kind: &str, sender: &UniqueName<'_>, token: &str) -> String {
    format!(
        "/org/freedesktop/portal/desktop/{kind}/{}/{token}",
        escape(sender)
    )
}

fn sender_of<'a>(header: &'a Header<'_>) -> UniqueName<'a> {
    header
        .sender()
        .expect("bus messages always carry a sender")
        .to_owned()
}

/// Emit `org.freedesktop.portal.Request.Response` on `path`.
async fn respond<B>(conn: Connection, path: String, code: u32, results: B)
where
    B: Serialize + Type + zbus::zvariant::DynamicType + Send + Sync + 'static,
{
    tokio::spawn(async move {
        if let Err(err) = conn
            .emit_signal(
                None::<&str>,
                path.as_str(),
                REQUEST_INTERFACE,
                "Response",
                &(code, results),
            )
            .await
        {
            eprintln!("mock portal failed to emit Response: {err}");
        }
    });
}

/// Dispatch one request according to `behaviour`.
///
/// `results` is only used by [`Behaviour::Succeed`].
async fn dispatch<B>(conn: Connection, path: String, behaviour: Behaviour, results: B)
where
    B: Serialize + Type + zbus::zvariant::DynamicType + Send + Sync + 'static,
{
    match behaviour {
        Behaviour::Succeed => respond(conn, path, 0, results).await,
        Behaviour::Deny => respond(conn, path, 1, EmptyResults::default()).await,
        Behaviour::Other => respond(conn, path, 2, EmptyResults::default()).await,
        Behaviour::Silent => {}
        Behaviour::Malformed => respond(conn, path, 0, MalformedResults::default()).await,
    }
}

/// An `a{sv}` with nothing in it.
#[derive(Serialize, Type, Default)]
#[zvariant(signature = "dict")]
pub struct EmptyResults {}

/// An `a{sv}` whose keys are right but whose value types are not.
#[derive(Serialize, Type)]
#[zvariant(signature = "dict")]
pub struct MalformedResults {
    #[serde(with = "as_value")]
    shortcuts: String,
    #[serde(with = "as_value")]
    session_handle: u32,
    #[serde(with = "as_value")]
    uris: u32,
}

impl Default for MalformedResults {
    fn default() -> Self {
        Self {
            shortcuts: "this should have been an array of structs".to_owned(),
            session_handle: 42,
            uris: 7,
        }
    }
}

/// `{"session_handle": o}`.
#[derive(Serialize, Type)]
#[zvariant(signature = "dict")]
struct SessionResults {
    #[serde(with = "as_value")]
    session_handle: OwnedObjectPath,
}

/// One entry of the `shortcuts` array, matching the interface's `a(sa{sv})`.
#[derive(Serialize, Type, Clone)]
struct ShortcutEntry(String, ShortcutEntryInfo);

#[derive(Serialize, Type, Clone)]
#[zvariant(signature = "dict")]
struct ShortcutEntryInfo {
    #[serde(with = "as_value")]
    description: String,
    #[serde(with = "as_value")]
    trigger_description: String,
}

/// `{"shortcuts": a(sa{sv})}`.
#[derive(Serialize, Type)]
#[zvariant(signature = "dict")]
struct ShortcutResults {
    #[serde(with = "as_value")]
    shortcuts: Vec<ShortcutEntry>,
}

/// `{"uris": as}`.
#[derive(Serialize, Type)]
#[zvariant(signature = "dict")]
struct FileResults {
    #[serde(with = "as_value")]
    uris: Vec<String>,
}

// ---------------------------------------------------------------------------
// GlobalShortcuts
// ---------------------------------------------------------------------------

/// Mock `org.freedesktop.portal.GlobalShortcuts`.
pub struct GlobalShortcutsIface {
    pub version: u32,
    /// How `CreateSession` answers. Kept separate so a test can deny a *bind*
    /// without also denying the session it needs to bind through.
    pub session_behaviour: Behaviour,
    /// How `BindShortcuts` and `ListShortcuts` answer.
    pub behaviour: Behaviour,
    pub state: SharedState,
}

#[interface(name = "org.freedesktop.portal.GlobalShortcuts")]
impl GlobalShortcutsIface {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        self.version
    }

    async fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let sender = sender_of(&header);
        let request = handle_path("request", &sender, &token(&options, "handle_token"));
        let session = OwnedObjectPath::try_from(handle_path(
            "session",
            &sender,
            &token(&options, "session_handle_token"),
        ))
        .expect("valid object path");

        {
            let mut state = lock(&self.state);
            state.calls.push("CreateSession");
            state.sessions.push(session.clone());
        }

        let conn = emitter.connection().clone();
        dispatch(
            conn,
            request.clone(),
            self.session_behaviour,
            SessionResults {
                session_handle: session,
            },
        )
        .await;
        Ok(OwnedObjectPath::try_from(request).expect("valid object path"))
    }

    #[allow(clippy::too_many_arguments)]
    async fn bind_shortcuts(
        &self,
        _session: OwnedObjectPath,
        shortcuts: Vec<(String, HashMap<String, OwnedValue>)>,
        _parent_window: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let sender = sender_of(&header);
        let request = handle_path("request", &sender, &token(&options, "handle_token"));

        let mut entries = Vec::new();
        {
            let mut state = lock(&self.state);
            state.calls.push("BindShortcuts");
            for (id, info) in &shortcuts {
                let description = info
                    .get("description")
                    .and_then(|v| <&str>::try_from(v).ok())
                    .unwrap_or_default()
                    .to_owned();
                let preferred = info
                    .get("preferred_trigger")
                    .and_then(|v| <&str>::try_from(v).ok())
                    .map(ToOwned::to_owned);
                state
                    .bound
                    .push((id.clone(), description.clone(), preferred.clone()));
                entries.push(ShortcutEntry(
                    id.clone(),
                    ShortcutEntryInfo {
                        description,
                        // The compositor, not the app, decides the final
                        // trigger; echo the preference back in the desktop's
                        // own prose so tests can tell the two apart.
                        trigger_description: preferred
                            .as_deref()
                            .map(describe_trigger)
                            .unwrap_or_else(|| "unassigned".to_owned()),
                    },
                ));
            }
        }

        let conn = emitter.connection().clone();
        dispatch(
            conn,
            request.clone(),
            self.behaviour,
            ShortcutResults { shortcuts: entries },
        )
        .await;
        Ok(OwnedObjectPath::try_from(request).expect("valid object path"))
    }

    async fn list_shortcuts(
        &self,
        _session: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let sender = sender_of(&header);
        let request = handle_path("request", &sender, &token(&options, "handle_token"));

        let entries: Vec<ShortcutEntry> = {
            let mut state = lock(&self.state);
            state.calls.push("ListShortcuts");
            state
                .bound
                .iter()
                .map(|(id, description, preferred)| {
                    ShortcutEntry(
                        id.clone(),
                        ShortcutEntryInfo {
                            description: description.clone(),
                            trigger_description: preferred
                                .as_deref()
                                .map(describe_trigger)
                                .unwrap_or_else(|| "unassigned".to_owned()),
                        },
                    )
                })
                .collect()
        };

        let conn = emitter.connection().clone();
        dispatch(
            conn,
            request.clone(),
            self.behaviour,
            ShortcutResults { shortcuts: entries },
        )
        .await;
        Ok(OwnedObjectPath::try_from(request).expect("valid object path"))
    }

    #[zbus(signal)]
    async fn activated(
        emitter: &SignalEmitter<'_>,
        session: OwnedObjectPath,
        shortcut_id: String,
        timestamp: u64,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn deactivated(
        emitter: &SignalEmitter<'_>,
        session: OwnedObjectPath,
        shortcut_id: String,
        timestamp: u64,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;
}

/// `xdg-desktop-portal-gnome` renders a trigger for display; mimic that so
/// tests can prove the client uses the *portal's* string and not its own.
fn describe_trigger(trigger: &str) -> String {
    trigger.replace("LOGO", "Super")
}

/// A GlobalShortcuts interface whose `version` property is a string.
pub struct MalformedVersionIface;

#[interface(name = "org.freedesktop.portal.GlobalShortcuts")]
impl MalformedVersionIface {
    #[zbus(property, name = "version")]
    fn version(&self) -> String {
        "one".to_owned()
    }
}

// ---------------------------------------------------------------------------
// OpenURI
// ---------------------------------------------------------------------------

/// Mock `org.freedesktop.portal.OpenURI`.
pub struct OpenUriIface {
    pub version: u32,
    pub behaviour: Behaviour,
    pub state: SharedState,
}

#[interface(name = "org.freedesktop.portal.OpenURI")]
impl OpenUriIface {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        self.version
    }

    #[zbus(name = "OpenURI")]
    async fn open_uri(
        &self,
        _parent_window: String,
        uri: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let sender = sender_of(&header);
        let request = handle_path("request", &sender, &token(&options, "handle_token"));
        {
            let mut state = lock(&self.state);
            state.calls.push("OpenURI");
            state.opened_uris.push(uri);
        }
        let conn = emitter.connection().clone();
        dispatch(
            conn,
            request.clone(),
            self.behaviour,
            EmptyResults::default(),
        )
        .await;
        Ok(OwnedObjectPath::try_from(request).expect("valid object path"))
    }

    /// `OpenFile(parent_window s, fd h, options a{sv})`.
    ///
    /// The descriptor really does cross the bus, so the mock reads it back and
    /// records the bytes: that is the only way to prove the sandboxed path
    /// works, since inside a Flatpak the portal never sees a filename.
    async fn open_file(
        &self,
        _parent_window: String,
        fd: zbus::zvariant::OwnedFd,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        use std::io::Read;

        let sender = sender_of(&header);
        let request = handle_path("request", &sender, &token(&options, "handle_token"));
        let mut contents = String::new();
        let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
        file.read_to_string(&mut contents).ok();
        {
            let mut state = lock(&self.state);
            state.calls.push("OpenFile");
            state.opened_fd_contents.push(contents);
        }
        let conn = emitter.connection().clone();
        dispatch(
            conn,
            request.clone(),
            self.behaviour,
            EmptyResults::default(),
        )
        .await;
        Ok(OwnedObjectPath::try_from(request).expect("valid object path"))
    }

    async fn scheme_supported(
        &self,
        scheme: String,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<bool> {
        lock(&self.state).calls.push("SchemeSupported");
        Ok(matches!(scheme.as_str(), "http" | "https" | "mailto"))
    }
}

// ---------------------------------------------------------------------------
// FileChooser
// ---------------------------------------------------------------------------

/// Mock `org.freedesktop.portal.FileChooser`.
pub struct FileChooserIface {
    pub version: u32,
    pub behaviour: Behaviour,
    pub state: SharedState,
}

#[interface(name = "org.freedesktop.portal.FileChooser")]
impl FileChooserIface {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        self.version
    }

    async fn open_file(
        &self,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let sender = sender_of(&header);
        let request = handle_path("request", &sender, &token(&options, "handle_token"));
        let uris = {
            let mut state = lock(&self.state);
            state.calls.push("OpenFile");
            state.chooser_titles.push(title);
            state.chooser_options = options
                .iter()
                .map(|(k, v)| (k.clone(), v.try_clone().expect("clonable")))
                .collect();
            state.chooser_reply.clone()
        };
        let conn = emitter.connection().clone();
        dispatch(conn, request.clone(), self.behaviour, FileResults { uris }).await;
        Ok(OwnedObjectPath::try_from(request).expect("valid object path"))
    }
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

/// Which interfaces to export, and how they behave.
#[derive(Debug, Clone, Copy)]
pub struct MockOptions {
    /// Export GlobalShortcuts.
    pub global_shortcuts: bool,
    /// Export OpenURI.
    pub open_uri: bool,
    /// Export FileChooser.
    pub file_chooser: bool,
    /// Version every exported interface reports.
    pub version: u32,
    /// How requests are answered.
    pub behaviour: Behaviour,
    /// How `GlobalShortcuts.CreateSession` is answered.
    pub session_behaviour: Behaviour,
    /// Take the `org.freedesktop.portal.Desktop` name.
    pub own_name: bool,
    /// Export GlobalShortcuts with a `version` property of the wrong type.
    pub malformed_version: bool,
}

impl Default for MockOptions {
    fn default() -> Self {
        Self {
            global_shortcuts: true,
            open_uri: true,
            file_chooser: true,
            version: 2,
            behaviour: Behaviour::Succeed,
            session_behaviour: Behaviour::Succeed,
            own_name: true,
            malformed_version: false,
        }
    }
}

impl MockOptions {
    /// The `xdg-desktop-portal-wlr` shape: a healthy frontend that implements
    /// everything except GlobalShortcuts.
    pub fn without_global_shortcuts() -> Self {
        Self {
            global_shortcuts: false,
            ..Self::default()
        }
    }

    /// Answer every request with `behaviour`.
    #[must_use]
    pub fn behaving(mut self, behaviour: Behaviour) -> Self {
        self.behaviour = behaviour;
        self
    }

    /// Report `version` on every interface.
    #[must_use]
    pub fn at_version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }
}

/// A running mock portal. Dropping it releases the bus name.
pub struct MockPortal {
    conn: Connection,
    /// Shared, inspectable state.
    pub state: SharedState,
}

impl MockPortal {
    /// Start a mock portal on `address`.
    pub async fn start(address: &str, options: MockOptions) -> zbus::Result<Self> {
        let state: SharedState = Arc::new(Mutex::new(MockState::default()));
        let mut builder = zbus::connection::Builder::address(address)?;

        if options.malformed_version {
            builder = builder.serve_at(DESKTOP_PATH, MalformedVersionIface)?;
        } else if options.global_shortcuts {
            builder = builder.serve_at(
                DESKTOP_PATH,
                GlobalShortcutsIface {
                    version: options.version,
                    session_behaviour: options.session_behaviour,
                    behaviour: options.behaviour,
                    state: Arc::clone(&state),
                },
            )?;
        }
        if options.open_uri {
            builder = builder.serve_at(
                DESKTOP_PATH,
                OpenUriIface {
                    version: options.version,
                    behaviour: options.behaviour,
                    state: Arc::clone(&state),
                },
            )?;
        }
        if options.file_chooser {
            builder = builder.serve_at(
                DESKTOP_PATH,
                FileChooserIface {
                    version: options.version,
                    behaviour: options.behaviour,
                    state: Arc::clone(&state),
                },
            )?;
        }
        if options.own_name {
            builder = builder.name(DESKTOP_DESTINATION)?;
        }

        Ok(Self {
            conn: builder.build().await?,
            state,
        })
    }

    /// Contents of the files handed to `OpenURI.OpenFile` as descriptors.
    pub fn opened_fd_contents(&self) -> Vec<String> {
        lock(&self.state).opened_fd_contents.clone()
    }

    /// Methods invoked so far, in order.
    pub fn calls(&self) -> Vec<&'static str> {
        lock(&self.state).calls.clone()
    }

    /// Shortcuts bound so far as `(id, description, preferred_trigger)`.
    pub fn bound(&self) -> Vec<(String, String, Option<String>)> {
        lock(&self.state).bound.clone()
    }

    /// Set the URIs the next `FileChooser.OpenFile` will answer with.
    pub fn set_chooser_reply(&self, uris: &[&str]) {
        lock(&self.state).chooser_reply = uris.iter().map(|s| (*s).to_owned()).collect();
    }

    /// Options dict from the last `FileChooser.OpenFile`.
    pub fn chooser_option_bool(&self, key: &str) -> Option<bool> {
        lock(&self.state)
            .chooser_options
            .get(key)
            .and_then(|v| bool::try_from(v).ok())
    }

    /// The session path handed out by the first `CreateSession`.
    pub fn session_path(&self) -> Option<OwnedObjectPath> {
        lock(&self.state).sessions.first().cloned()
    }

    /// Emit `Activated` for `shortcut_id` on the first session.
    pub async fn emit_activated(
        &self,
        shortcut_id: &str,
        timestamp: u64,
        activation_token: Option<&str>,
    ) -> zbus::Result<()> {
        let session = self.session_path().expect("a session must exist first");
        let emitter = SignalEmitter::new(&self.conn, DESKTOP_PATH)?;
        let mut options: HashMap<String, OwnedValue> = HashMap::new();
        if let Some(token) = activation_token {
            options.insert(
                "activation_token".to_owned(),
                OwnedValue::try_from(zbus::zvariant::Value::from(token)).expect("string"),
            );
        }
        GlobalShortcutsIface::activated(
            &emitter,
            session,
            shortcut_id.to_owned(),
            timestamp,
            options,
        )
        .await
    }

    /// Emit `Deactivated` for `shortcut_id` on the first session.
    pub async fn emit_deactivated(&self, shortcut_id: &str, timestamp: u64) -> zbus::Result<()> {
        let session = self.session_path().expect("a session must exist first");
        let emitter = SignalEmitter::new(&self.conn, DESKTOP_PATH)?;
        GlobalShortcutsIface::deactivated(
            &emitter,
            session,
            shortcut_id.to_owned(),
            timestamp,
            HashMap::new(),
        )
        .await
    }
}

/// A portal frontend that owns the bus name but never services its main loop,
/// so nothing it is asked is ever answered.
///
/// `compass-shell` hit a real hang on exactly this shape and had to add
/// timeouts; the equivalent test here exists so we do not repeat it.
pub async fn start_unresponsive_portal(address: &str) -> zbus::Result<Connection> {
    zbus::connection::Builder::address(address)?
        .name(DESKTOP_DESTINATION)?
        .build()
        .await
}
