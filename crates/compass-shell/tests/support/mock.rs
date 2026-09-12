//! A mock GNOME Shell serving the versioned Compass contract.
//!
//! This is Suite 3a from `PLAN.md` §8.4: a fake
//! `org.gnome.Shell.Extensions.Vicinae.{Windows,Clipboard}` on a private bus,
//! with no display server involved. `zbus` serves as well as it consumes, so
//! the mock implements the very interfaces the client's proxies were generated
//! from — if the two drift, the tests stop compiling or stop passing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use compass_shell::contract;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, interface};

/// A window as the mock will report it.
#[derive(Debug, Clone)]
pub struct MockWindow {
    pub id: u32,
    pub title: String,
    pub wm_class: String,
    pub pid: Option<u32>,
    pub focused: bool,
    pub workspace: Option<i32>,
    /// Extra or overriding `a{sv}` entries, used to build malformed replies.
    pub overrides: Vec<(String, OwnedValue)>,
    /// Keys to omit entirely, used to build malformed replies.
    pub removed: Vec<String>,
}

impl MockWindow {
    pub fn new(id: u32, wm_class: &str, title: &str) -> Self {
        Self {
            id,
            title: title.to_owned(),
            wm_class: wm_class.to_owned(),
            pid: Some(1000 + id),
            focused: false,
            workspace: Some(0),
            overrides: Vec::new(),
            removed: Vec::new(),
        }
    }

    pub fn focused(mut self) -> Self {
        self.focused = true;
        self
    }

    /// Add or replace a raw dictionary entry, bypassing the typed fields.
    pub fn with_raw(mut self, key: &str, value: Value<'static>) -> Self {
        let value = OwnedValue::try_from(value).expect("value is convertible");
        self.overrides.push((key.to_owned(), value));
        self
    }

    /// Remove a key entirely from the emitted dictionary.
    pub fn without(mut self, key: &str) -> Self {
        self.removed.push(key.to_owned());
        self
    }

    fn to_dict(&self) -> HashMap<String, OwnedValue> {
        let mut dict: HashMap<String, OwnedValue> = HashMap::new();
        dict.insert(contract::window_key::ID.into(), OwnedValue::from(self.id));
        dict.insert(
            contract::window_key::TITLE.into(),
            OwnedValue::try_from(Value::from(self.title.clone())).expect("string"),
        );
        dict.insert(
            contract::window_key::WM_CLASS.into(),
            OwnedValue::try_from(Value::from(self.wm_class.clone())).expect("string"),
        );
        dict.insert(
            contract::window_key::WM_CLASS_INSTANCE.into(),
            OwnedValue::try_from(Value::from(self.wm_class.to_lowercase())).expect("string"),
        );
        if let Some(pid) = self.pid {
            dict.insert(contract::window_key::PID.into(), OwnedValue::from(pid));
        }
        dict.insert(
            contract::window_key::FOCUSED.into(),
            OwnedValue::from(self.focused),
        );
        if let Some(ws) = self.workspace {
            dict.insert(contract::window_key::WORKSPACE.into(), OwnedValue::from(ws));
        }
        for key in &self.removed {
            dict.remove(key);
        }
        for (key, value) in &self.overrides {
            dict.insert(key.clone(), value.try_clone().expect("clonable"));
        }
        dict
    }
}

/// Everything a test wants to inspect or steer about the mock.
#[derive(Debug, Default)]
pub struct MockState {
    pub windows: Vec<MockWindow>,
    pub clipboard: (Vec<u8>, String),
    /// Every `(method, argument)` the client invoked, in order.
    pub calls: Vec<(&'static str, u32)>,
}

pub type SharedState = Arc<Mutex<MockState>>;

pub struct WindowsService {
    pub version: u32,
    pub state: SharedState,
}

#[interface(name = "org.gnome.Shell.Extensions.Vicinae.Windows")]
impl WindowsService {
    #[zbus(property)]
    fn version(&self) -> u32 {
        self.version
    }

    fn list_windows(&self) -> Vec<HashMap<String, OwnedValue>> {
        let state = self.state.lock().expect("mock state");
        state.windows.iter().map(MockWindow::to_dict).collect()
    }

    fn activate_window(&self, id: u32) {
        let mut state = self.state.lock().expect("mock state");
        state.calls.push(("ActivateWindow", id));
        for window in &mut state.windows {
            window.focused = window.id == id;
        }
    }

    fn close_window(&self, id: u32) {
        let mut state = self.state.lock().expect("mock state");
        state.calls.push(("CloseWindow", id));
        state.windows.retain(|w| w.id != id);
    }

    #[zbus(signal)]
    async fn windows_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// A placeholder object so that a mock with neither contract interface still
/// runs an object server, and therefore answers unknown paths with a proper
/// `UnknownObject` error instead of never answering at all. A real
/// `gnome-shell` always exports plenty of other objects.
pub struct OtherShellObject;

#[interface(name = "org.gnome.Shell")]
impl OtherShellObject {
    fn eval(&self, _script: String) -> (bool, String) {
        (false, "Eval is disabled".to_owned())
    }
}

pub struct ClipboardService {
    pub version: u32,
    pub state: SharedState,
}

#[interface(name = "org.gnome.Shell.Extensions.Vicinae.Clipboard")]
impl ClipboardService {
    #[zbus(property)]
    fn version(&self) -> u32 {
        self.version
    }

    fn get_clipboard(&self) -> (Vec<u8>, String) {
        self.state.lock().expect("mock state").clipboard.clone()
    }

    fn set_clipboard(&self, content: Vec<u8>, mime_type: String) {
        self.state.lock().expect("mock state").clipboard = (content, mime_type);
    }

    #[zbus(signal)]
    async fn clipboard_changed(
        emitter: &SignalEmitter<'_>,
        content: Vec<u8>,
        mime_type: String,
        source_app: String,
    ) -> zbus::Result<()>;
}

/// How much of the contract the mock should serve.
#[derive(Debug, Clone, Copy)]
pub struct MockOptions {
    /// Version reported by both interfaces.
    pub version: u32,
    /// Serve the windows interface at all.
    pub windows: bool,
    /// Serve the clipboard interface at all.
    pub clipboard: bool,
    /// Request the `org.gnome.Shell` well-known name.
    pub own_name: bool,
}

impl Default for MockOptions {
    fn default() -> Self {
        Self {
            version: compass_shell::CONTRACT_VERSION,
            windows: true,
            clipboard: true,
            own_name: true,
        }
    }
}

/// A running mock shell. Dropping it releases the bus name, which is exactly
/// what a `gnome-shell` crash looks like to the client.
pub struct MockShell {
    conn: Connection,
    pub state: SharedState,
}

impl MockShell {
    pub async fn start(address: &str, options: MockOptions) -> zbus::Result<Self> {
        Self::start_with_state(address, options, Arc::new(Mutex::new(MockState::default()))).await
    }

    pub async fn start_with_state(
        address: &str,
        options: MockOptions,
        state: SharedState,
    ) -> zbus::Result<Self> {
        let mut builder = zbus::connection::Builder::address(address)?;

        if options.windows {
            builder = builder.serve_at(
                contract::WINDOWS_PATH,
                WindowsService {
                    version: options.version,
                    state: Arc::clone(&state),
                },
            )?;
        }
        if options.clipboard {
            builder = builder.serve_at(
                contract::CLIPBOARD_PATH,
                ClipboardService {
                    version: options.version,
                    state: Arc::clone(&state),
                },
            )?;
        }
        if !options.windows && !options.clipboard {
            builder = builder.serve_at("/org/gnome/Shell", OtherShellObject)?;
        }
        if options.own_name {
            builder = builder.name(contract::SHELL_SERVICE)?;
        }

        Ok(Self {
            conn: builder.build().await?,
            state,
        })
    }

    pub fn set_windows(&self, windows: Vec<MockWindow>) {
        self.state.lock().expect("mock state").windows = windows;
    }

    pub fn set_clipboard(&self, data: &[u8], mime_type: &str) {
        self.state.lock().expect("mock state").clipboard = (data.to_vec(), mime_type.to_owned());
    }

    pub fn calls(&self) -> Vec<(&'static str, u32)> {
        self.state.lock().expect("mock state").calls.clone()
    }

    pub async fn emit_windows_changed(&self) -> zbus::Result<()> {
        let emitter = SignalEmitter::new(&self.conn, contract::WINDOWS_PATH)?;
        WindowsService::windows_changed(&emitter).await
    }

    pub async fn emit_clipboard_changed(
        &self,
        content: &[u8],
        mime_type: &str,
        source_app: &str,
    ) -> zbus::Result<()> {
        let emitter = SignalEmitter::new(&self.conn, contract::CLIPBOARD_PATH)?;
        ClipboardService::clipboard_changed(
            &emitter,
            content.to_vec(),
            mime_type.to_owned(),
            source_app.to_owned(),
        )
        .await
    }

    /// Release the bus name and tear the connection down, simulating a
    /// `gnome-shell` restart.
    pub async fn shutdown(self) {
        self.conn.close().await.expect("close mock connection");
    }
}

/// A shell whose windows object answers `ListWindows` with the *old*
/// unversioned reply shape: a single JSON string instead of `aa{sv}`.
pub struct WrongSignatureWindows;

#[interface(name = "org.gnome.Shell.Extensions.Vicinae.Windows")]
impl WrongSignatureWindows {
    #[zbus(property)]
    fn version(&self) -> u32 {
        compass_shell::CONTRACT_VERSION
    }

    fn list_windows(&self) -> String {
        "[{\"id\": 1, \"title\": \"legacy JSON\"}]".to_owned()
    }
}

/// Serve a shell that passes capability probing but violates the reply
/// contract, so the client's decoding path is exercised end to end.
pub async fn start_wrong_signature_shell(address: &str) -> zbus::Result<Connection> {
    zbus::connection::Builder::address(address)?
        .serve_at(contract::WINDOWS_PATH, WrongSignatureWindows)?
        .name(contract::SHELL_SERVICE)?
        .build()
        .await
}

/// A shell that owns the bus name but never services its main loop.
///
/// The raw connection is built without an object server and without ever
/// draining incoming messages, so method calls to it are simply never
/// answered — which is what a wedged `gnome-shell` looks like.
pub async fn start_unresponsive_shell(address: &str) -> zbus::Result<Connection> {
    zbus::connection::Builder::address(address)?
        .name(contract::SHELL_SERVICE)?
        .build()
        .await
}
