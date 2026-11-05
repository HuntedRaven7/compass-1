//! Rhai scripts in the engine (PLAN §2.2, Track C).
//!
//! `compass-script` is the tier: the sandbox, discovery, hot reload, and
//! `search`/`invoke` producing the same view the TypeScript tier produces.
//! This is where the engine uses it:
//!
//! - **Loading.** [`RhaiScripts::reload`] scans the search paths, applies the
//!   [`ScriptSet`] diff, declares and grants capabilities, and builds one
//!   [`ScriptInstance`] per script. The engine lists every script it found in
//!   root search, including one whose source does not compile: opening it
//!   shows why, which is what an author editing it wants to see.
//! - **Granting**, by where a script is installed (see [`Origin`] and
//!   `docs/rust-engine/RHAI-SCRIPTS.md`, "Permissions").
//! - **The view.** A script is opened with `RunExtensionCommand` and its
//!   `rhai:` id and then followed and driven with the same requests an
//!   extension's view uses, so the launcher's extension page draws it
//!   unchanged. Each session keeps a [`ViewState`] the launcher long-polls;
//!   the search bar's text arrives as the [`SEARCH_HANDLER`] event, and an
//!   action's token is dispatched through the seam's [`ActionIndex`] and
//!   [`Pending`].
//! - **Hot reload.** [`watch`] rebuilds what changed on disk; a view that is
//!   open on a changed script re-renders with the new code.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use compass_core::rhai_scripts::RhaiScriptItem;
use compass_extension_api::{
    ActionEffect, ActionIndex, ActionPayload, ActionResponse, Capability, CapabilityRegistry,
    DispatchError, ExtensionId, HandlerId, InvocationSource, Pending, ToastStyle, View, ViewTree,
};
use compass_script::{DiscoveredScript, Limits, ScriptHost, ScriptInstance, ScriptSet, discovery};
use tokio::sync::RwLock;

use crate::extension_runner::ViewState;
use crate::serve::EngineState;

/// The handler a script's list carries for its search bar: the launcher sends
/// the text with it on every keystroke, and the engine runs `search(text)`.
pub const SEARCH_HANDLER: &str = "rhai.search";

/// The file user consent is recorded in, under `$XDG_CONFIG_HOME/compass/`.
pub const CONSENT_FILE: &str = "script-grants.json";

/// Where a script is installed, which decides what it is granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Under an `$XDG_DATA_DIRS` entry: installed with Compass (the Flatpak's
    /// `/app/share`, a distribution package's `/usr/share`). Granted what its
    /// manifest declares, as installing it was the decision.
    Packaged,
    /// In the user's own directory, `$XDG_DATA_HOME/compass/scripts`. Granted
    /// what it declares only once the user has allowed it in the launcher.
    User,
}

/// Where the engine looks for scripts and keeps consent.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Search paths, highest precedence first.
    pub search_paths: Vec<PathBuf>,
    /// The user's own directory, created at start and watched; scripts in it
    /// are [`Origin::User`].
    pub user_dir: Option<PathBuf>,
    /// The consent file; `None` keeps consent for this run only.
    pub consent_file: Option<PathBuf>,
}

impl Config {
    /// The XDG locations: `compass_script::discovery::search_paths`, and the
    /// consent file under `$XDG_CONFIG_HOME/compass`.
    #[must_use]
    pub fn from_environment() -> Self {
        let mut search_paths = discovery::search_paths();
        // An install's own scripts, whether or not its prefix is on
        // `$XDG_DATA_DIRS` (an AppImage's, a Nix store path's).
        if let Some(installed) = std::env::current_exe().ok().and_then(|exe| {
            Some(
                exe.parent()?
                    .parent()?
                    .join("share")
                    .join(discovery::DATA_DIR_NAME)
                    .join(discovery::SCRIPTS_SUBDIR),
            )
        }) && !search_paths.contains(&installed)
        {
            search_paths.push(installed);
        }
        Self {
            search_paths,
            user_dir: discovery::user_scripts_dir(),
            consent_file: compass_xdg::xdg_dirs::config_home()
                .map(|home| home.join(discovery::DATA_DIR_NAME).join(CONSENT_FILE)),
        }
    }
}

/// What the user has allowed, by script id.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Consents {
    #[serde(default)]
    scripts: BTreeMap<String, BTreeSet<String>>,
}

impl Consents {
    fn load(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                tracing::warn!(path = %path.display(), %err, "unreadable script consent; asking again");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|err| err.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let staging = path.with_extension("json.tmp");
        std::fs::write(&staging, text).map_err(|err| err.to_string())?;
        std::fs::rename(&staging, path).map_err(|err| err.to_string())
    }

    fn allowed(&self, id: &ExtensionId) -> BTreeSet<Capability> {
        self.scripts
            .get(id.as_str())
            .into_iter()
            .flatten()
            .map(|name| Capability::new(name.clone()))
            .collect()
    }
}

/// What the engine must do after a script's action, beyond the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Hide the launcher.
    Close,
    /// Show a HUD, then hide the launcher.
    Hud(String),
}

/// One script as loaded.
#[derive(Debug)]
struct Loaded {
    origin: Origin,
    /// The capabilities it holds.
    granted: BTreeSet<Capability>,
    instance: Result<ScriptInstance, String>,
}

#[derive(Debug, Default)]
struct Scripts {
    set: ScriptSet,
    registry: CapabilityRegistry,
    consents: Consents,
    loaded: BTreeMap<ExtensionId, Loaded>,
    /// Directories that would not load, and why, as last reported.
    failed: BTreeMap<PathBuf, String>,
}

/// Every loaded script and every open script view.
#[derive(Debug)]
pub struct RhaiScripts {
    config: Config,
    host: Arc<dyn ScriptHost>,
    limits: Limits,
    scripts: Mutex<Scripts>,
    sessions: Mutex<HashMap<u64, Arc<Session>>>,
}

impl Default for RhaiScripts {
    fn default() -> Self {
        Self::new(Config::default(), Arc::new(RefusingHost))
    }
}

impl RhaiScripts {
    /// Nothing loaded yet; call [`Self::reload`].
    #[must_use]
    pub fn new(config: Config, host: Arc<dyn ScriptHost>) -> Self {
        Self {
            config,
            host,
            limits: Limits::default(),
            scripts: Mutex::new(Scripts::default()),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The paths scripts are found in.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Scripts> {
        self.scripts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_sessions(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn origin(&self, script: &DiscoveredScript) -> Origin {
        let parent = script.manifest.directory.parent();
        match (&self.config.user_dir, parent) {
            (Some(user), Some(parent)) if user == parent => Origin::User,
            _ => Origin::Packaged,
        }
    }

    /// Creates the user's script directory, so it can be watched from the
    /// start and a script dropped into it is found.
    pub fn create_user_dir(&self) {
        if let Some(dir) = &self.config.user_dir
            && let Err(err) = std::fs::create_dir_all(dir)
        {
            tracing::info!(dir = %dir.display(), %err, "cannot create the user's script directory");
        }
    }

    /// Rescans the search paths and the consent file, rebuilds every script
    /// whose files or grants changed, and returns what root search lists.
    /// A view open on a rebuilt script renders again with the new code.
    pub fn reload(&self) -> Vec<RhaiScriptItem> {
        let scan = discovery::scan(&self.config.search_paths);
        for (shadowed, winner) in &scan.shadowed {
            tracing::debug!(shadowed = %shadowed.display(), winner = %winner.display(), "a Rhai script is shadowed");
        }
        let origins: BTreeMap<ExtensionId, Origin> = scan
            .scripts
            .iter()
            .map(|script| (script.manifest.id.clone(), self.origin(script)))
            .collect();

        let (rebuilt, removed, items) = {
            let mut scripts = self.lock();
            // Said once per breakage, not on every rescan (every summon).
            let failed: BTreeMap<PathBuf, String> = scan
                .failed
                .iter()
                .map(|(path, error)| (path.clone(), error.to_string()))
                .collect();
            for (path, error) in &failed {
                if scripts.failed.get(path) != Some(error) {
                    tracing::warn!(path = %path.display(), %error, "a Rhai script was not loaded");
                }
            }
            scripts.failed = failed;
            scripts.consents = Consents::load(self.config.consent_file.as_deref());
            let diff = scripts.set.apply(scan);
            for id in &diff.removed {
                scripts.registry.forget(id);
                scripts.loaded.remove(id);
            }
            let changed: BTreeSet<ExtensionId> =
                diff.added.iter().chain(&diff.changed).cloned().collect();
            let ids: Vec<ExtensionId> = scripts.set.iter().map(|s| s.manifest.id.clone()).collect();
            let mut rebuilt = Vec::new();
            for id in ids {
                let origin = origins.get(&id).copied().unwrap_or(Origin::User);
                if self.refresh(&mut scripts, &id, origin, changed.contains(&id)) {
                    rebuilt.push(id);
                }
            }
            (rebuilt, diff.removed, items(&scripts.set))
        };
        if !rebuilt.is_empty() || !removed.is_empty() {
            tracing::info!(
                scripts = items.len(),
                rebuilt = rebuilt.len(),
                removed = removed.len(),
                "Rhai scripts reloaded"
            );
        }
        for id in &removed {
            for session in self.sessions_of(id) {
                session.end(Some(format!("{} was removed", session.title)));
            }
        }
        for id in &rebuilt {
            self.renew_sessions(id, None);
        }
        items
    }

    /// Recomputes `id`'s grant and rebuilds its instance when its files
    /// (`changed`) or its grant did. Returns whether it was rebuilt.
    fn refresh(
        &self,
        scripts: &mut Scripts,
        id: &ExtensionId,
        origin: Origin,
        changed: bool,
    ) -> bool {
        let Some(script) = scripts.set.get(id).cloned() else {
            return false;
        };
        let declared: BTreeSet<Capability> = script.manifest.capabilities.iter().cloned().collect();
        let granted: BTreeSet<Capability> = match origin {
            Origin::Packaged => declared.iter().filter(|c| c.is_known()).cloned().collect(),
            Origin::User => scripts
                .consents
                .allowed(id)
                .intersection(&declared)
                .filter(|c| c.is_known())
                .cloned()
                .collect(),
        };
        let unchanged = scripts
            .loaded
            .get(id)
            .is_some_and(|loaded| loaded.origin == origin && loaded.granted == granted);
        if unchanged && !changed {
            return false;
        }
        scripts.registry.forget(id);
        script.manifest.declare_into(&mut scripts.registry);
        for capability in &granted {
            if let Err(denial) = scripts.registry.grant(id, capability) {
                tracing::warn!(%denial, "a Rhai script's capability was not granted");
            }
        }
        let instance = ScriptInstance::load(
            &script,
            &scripts.registry,
            Arc::clone(&self.host),
            self.limits,
        )
        .map_err(|error| format!("{} cannot run: {error}", script.manifest.title));
        if let Err(problem) = &instance {
            tracing::warn!(script = %id, %problem, "a Rhai script does not load");
        }
        scripts.loaded.insert(
            id.clone(),
            Loaded {
                origin,
                granted,
                instance,
            },
        );
        true
    }

    /// Every script root search lists, in id order.
    #[must_use]
    pub fn items(&self) -> Vec<RhaiScriptItem> {
        items(&self.lock().set)
    }

    /// The capabilities `id` declares, knows about and has not been allowed:
    /// what a consent prompt asks for. Empty for a packaged script.
    fn needs_consent(&self, id: &ExtensionId) -> Vec<Capability> {
        let scripts = self.lock();
        let (Some(script), Some(loaded)) = (scripts.set.get(id), scripts.loaded.get(id)) else {
            return Vec::new();
        };
        if loaded.origin == Origin::Packaged {
            return Vec::new();
        }
        let allowed = scripts.consents.allowed(id);
        let mut missing: Vec<Capability> = script
            .manifest
            .capabilities
            .iter()
            .filter(|c| c.is_known() && !allowed.contains(*c))
            .cloned()
            .collect();
        missing.sort();
        missing.dedup();
        missing
    }

    fn instance(&self, id: &ExtensionId) -> Option<Result<ScriptInstance, String>> {
        self.lock()
            .loaded
            .get(id)
            .map(|loaded| loaded.instance.clone())
    }

    fn sessions_of(&self, id: &ExtensionId) -> Vec<Arc<Session>> {
        self.lock_sessions()
            .values()
            .filter(|session| &session.script == id)
            .cloned()
            .collect()
    }

    /// Hands the views open on `id` its new instance, and renders them again,
    /// all but `except`, which renders itself: a second render racing it would
    /// retire the action tokens of the first.
    fn renew_sessions(&self, id: &ExtensionId, except: Option<&Arc<Session>>) {
        let Some(instance) = self.instance(id) else {
            return;
        };
        for session in self.sessions_of(id) {
            if session.awaiting_consent() || except.is_some_and(|e| Arc::ptr_eq(e, &session)) {
                continue;
            }
            session.set_instance(instance.clone());
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let session = Arc::clone(&session);
                handle.spawn(async move { session.render().await });
            }
        }
    }

    /// Opens the script `id` as view session `session`, and renders it —
    /// unless it needs the user's consent first, which the view asks for.
    ///
    /// # Errors
    ///
    /// A sentence when no script has that id.
    pub async fn open(&self, id: &str, session: u64) -> Result<(), String> {
        // Consent may have been given or withdrawn by hand since the last
        // scan; a reload is cheap when nothing changed.
        self.reload();
        let id = ExtensionId::new(id);
        let title = self
            .lock()
            .set
            .get(&id)
            .map(|script| script.manifest.title.clone())
            .ok_or_else(|| "No Rhai script has that id".to_owned())?;
        let missing = self.needs_consent(&id);
        let instance = if missing.is_empty() {
            self.instance(&id)
        } else {
            None
        };
        let opened = Arc::new(Session::new(id, title, instance));
        self.lock_sessions().insert(session, Arc::clone(&opened));
        if missing.is_empty() {
            opened.render().await;
        } else {
            opened.ask(consent_prompt(&opened.title, &missing));
        }
        Ok(())
    }

    fn session(&self, session: u64) -> Option<Arc<Session>> {
        self.lock_sessions().get(&session).cloned()
    }

    /// Whether `session` is a script's view.
    #[must_use]
    pub fn has(&self, session: u64) -> bool {
        self.lock_sessions().contains_key(&session)
    }

    /// Follows `session`'s state; `None` for one that is not a script's.
    #[must_use]
    pub fn watch(&self, session: u64) -> Option<tokio::sync::watch::Receiver<ViewState>> {
        self.session(session).map(|s| s.state.subscribe())
    }

    /// Runs `handler` in `session`: the search text, or an action. Returns
    /// what the engine must do beyond the view.
    ///
    /// # Errors
    ///
    /// A sentence: the session is gone, still waiting on consent, or the
    /// action is no longer on screen.
    pub async fn event(
        &self,
        session: u64,
        handler: &str,
        args: &[serde_json::Value],
    ) -> Result<Vec<Outcome>, String> {
        let session = self
            .session(session)
            .filter(|session| !session.ended())
            .ok_or_else(|| "That script's view has closed".to_owned())?;
        if session.awaiting_consent() {
            return Err(format!("{} is waiting to be allowed", session.title));
        }
        if handler == SEARCH_HANDLER {
            let text = args
                .first()
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            // The launcher sends its keystroke count after the text.
            let sequence = args.get(1).and_then(serde_json::Value::as_u64);
            if !session.set_query(text, sequence) {
                return Ok(Vec::new());
            }
            let search = Arc::clone(&session);
            tokio::spawn(async move { search.render().await });
            return Ok(Vec::new());
        }
        session.act(handler).await
    }

    /// The user's answer to the consent prompt `session` shows. Allowing is
    /// recorded, the script rebuilt with its grant, and the view rendered;
    /// refusing ends the view and records nothing, so it asks again.
    ///
    /// # Errors
    ///
    /// A sentence: no such session, nothing asked, or the answer could not
    /// be kept.
    pub async fn answer(&self, session: u64, allowed: bool) -> Result<(), String> {
        let session = self
            .session(session)
            .filter(|session| !session.ended())
            .ok_or_else(|| "That script's view has closed".to_owned())?;
        if !session.awaiting_consent() {
            return Err(format!("{} is not asking anything", session.title));
        }
        if !allowed {
            session.end(Some(format!(
                "{} was not allowed what it asks for. Open it again to be asked again.",
                session.title
            )));
            return Ok(());
        }
        let missing = self.needs_consent(&session.script);
        {
            let mut scripts = self.lock();
            let mut consents = Consents::load(self.config.consent_file.as_deref());
            consents
                .scripts
                .entry(session.script.as_str().to_owned())
                .or_default()
                .extend(missing.iter().map(|c| c.as_str().to_owned()));
            if let Some(path) = &self.config.consent_file {
                consents
                    .save(path)
                    .map_err(|err| format!("Compass could not keep the permission: {err}"))?;
            }
            scripts.consents = consents;
            let origin = scripts
                .loaded
                .get(&session.script)
                .map_or(Origin::User, |l| l.origin);
            self.refresh(&mut scripts, &session.script, origin, false);
        }
        tracing::info!(script = %session.script, capabilities = ?missing, "Rhai script allowed");
        let instance = self.instance(&session.script);
        session.consented(instance);
        // Other views of the same script were built without the grant.
        self.renew_sessions(&session.script, Some(&session));
        session.render().await;
        Ok(())
    }

    /// What the user has allowed, script by script, from the consent file:
    /// each recorded script's id, its title when it is still installed (its
    /// id otherwise), and what it may do, by capability and in the consent
    /// prompt's words. In id order.
    #[must_use]
    pub fn grants(&self) -> Vec<compass_ipc::ScriptGrantEntry> {
        let consents = Consents::load(self.config.consent_file.as_deref());
        let scripts = self.lock();
        consents
            .scripts
            .iter()
            .filter(|(_, capabilities)| !capabilities.is_empty())
            .map(|(id, capabilities)| {
                let title = scripts
                    .set
                    .get(&ExtensionId::new(id.clone()))
                    .map_or_else(|| id.clone(), |s| s.manifest.title.clone());
                compass_ipc::ScriptGrantEntry {
                    id: id.clone(),
                    title,
                    capabilities: capabilities.iter().cloned().collect(),
                    descriptions: capabilities
                        .iter()
                        .map(|name| describe(&Capability::new(name.clone())))
                        .collect(),
                }
            })
            .collect()
    }

    /// Withdraws everything the user allowed `id`: its entry leaves the
    /// consent file, the script is rebuilt without the grant, and a view open
    /// on it renders again (and asks again when next opened).
    ///
    /// # Errors
    ///
    /// A sentence: nothing is recorded for `id`, or the file could not be
    /// written.
    pub fn revoke(&self, id: &str) -> Result<(), String> {
        let mut consents = Consents::load(self.config.consent_file.as_deref());
        if consents.scripts.remove(id).is_none() {
            return Err(format!("No permissions are recorded for {id}"));
        }
        if let Some(path) = &self.config.consent_file {
            consents
                .save(path)
                .map_err(|err| format!("Compass could not change the permissions: {err}"))?;
        }
        let script = ExtensionId::new(id.to_owned());
        {
            let mut scripts = self.lock();
            scripts.consents = consents;
            let origin = scripts
                .loaded
                .get(&script)
                .map_or(Origin::User, |loaded| loaded.origin);
            self.refresh(&mut scripts, &script, origin, false);
        }
        tracing::info!(%script, "Rhai script permissions revoked");
        for session in self.sessions_of(&script) {
            session.end(Some(format!(
                "{}'s permissions were revoked. Open it again to be asked.",
                session.title
            )));
        }
        Ok(())
    }

    /// Closes `session`. `false` when it is not a script's view.
    pub fn close(&self, session: u64) -> bool {
        self.lock_sessions().remove(&session).is_some()
    }
}

fn items(set: &ScriptSet) -> Vec<RhaiScriptItem> {
    set.iter()
        .map(|script| RhaiScriptItem {
            id: script.manifest.id.as_str().to_owned(),
            title: script.manifest.title.clone(),
            description: script.manifest.description.clone(),
            icon: script.manifest.icon.clone(),
            keywords: script.manifest.keywords.clone(),
        })
        .collect()
}

/// What a capability lets a script do, in the words of a consent prompt.
fn describe(capability: &Capability) -> String {
    match capability.as_str() {
        "clipboard.write" => "copy to the clipboard".to_owned(),
        "clipboard.read" => "read the clipboard".to_owned(),
        "clipboard.paste" => "paste into other windows".to_owned(),
        "application.open" => "open links, files and applications".to_owned(),
        "storage.read" => "read its saved data".to_owned(),
        "storage.write" => "save data".to_owned(),
        "notification.send" => "show notifications".to_owned(),
        "process.execute" => "run programs on your computer".to_owned(),
        other => other.to_owned(),
    }
}

fn consent_prompt(title: &str, missing: &[Capability]) -> compass_ipc::ExtensionAlert {
    let lines: Vec<String> = missing
        .iter()
        .map(|capability| format!("• {}", describe(capability)))
        .collect();
    compass_ipc::ExtensionAlert {
        title: format!("Allow {title} to:"),
        message: format!(
            "{}\n\nThis script is in your own scripts folder. Compass remembers your answer.",
            lines.join("\n")
        ),
        confirm_text: "Allow".to_owned(),
        cancel_text: "Don't Allow".to_owned(),
        remember_text: None,
    }
}

#[derive(Debug)]
struct SessionInner {
    /// `None` while consent is asked for.
    instance: Option<Result<ScriptInstance, String>>,
    awaiting_consent: bool,
    query: String,
    /// The launcher's count for the text in `query`. Each keystroke is its
    /// own request, so they can arrive out of order; an older text that
    /// lands after a newer one is dropped rather than searched.
    query_sequence: Option<u64>,
    /// Bumped per search; only the latest publishes.
    generation: u64,
    index: ActionIndex,
    pending: Pending,
}

/// One open script view.
#[derive(Debug)]
struct Session {
    script: ExtensionId,
    title: String,
    state: tokio::sync::watch::Sender<ViewState>,
    inner: Mutex<SessionInner>,
}

impl Session {
    fn new(
        script: ExtensionId,
        title: String,
        instance: Option<Result<ScriptInstance, String>>,
    ) -> Self {
        let (state, _) = tokio::sync::watch::channel(ViewState {
            depth: 1,
            ..ViewState::default()
        });
        Self {
            script,
            title,
            state,
            inner: Mutex::new(SessionInner {
                awaiting_consent: instance.is_none(),
                instance,
                query: String::new(),
                query_sequence: None,
                generation: 0,
                index: ActionIndex::default(),
                pending: Pending::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SessionInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn awaiting_consent(&self) -> bool {
        self.lock().awaiting_consent
    }

    /// Whether the view has ended: it stays, showing why, until the
    /// launcher closes it.
    fn ended(&self) -> bool {
        self.state.borrow().ended
    }

    fn set_instance(&self, instance: Result<ScriptInstance, String>) {
        self.lock().instance = Some(instance);
    }

    /// Takes `query` as the search text unless the launcher's `sequence`
    /// says a newer one is already in. Returns whether it was taken.
    fn set_query(&self, query: String, sequence: Option<u64>) -> bool {
        let mut inner = self.lock();
        if let (Some(new), Some(current)) = (sequence, inner.query_sequence)
            && new < current
        {
            return false;
        }
        inner.query = query;
        inner.query_sequence = sequence.or(inner.query_sequence);
        true
    }

    fn ask(&self, alert: compass_ipc::ExtensionAlert) {
        self.state.send_modify(|state| {
            state.version += 1;
            state.alert = Some(alert);
        });
    }

    fn consented(&self, instance: Option<Result<ScriptInstance, String>>) {
        {
            let mut inner = self.lock();
            inner.awaiting_consent = false;
            inner.instance = instance;
        }
        self.state.send_modify(|state| {
            state.version += 1;
            state.alert = None;
        });
    }

    fn end(&self, problem: Option<String>) {
        self.state.send_modify(|state| {
            state.version += 1;
            state.ended = true;
            state.alert = None;
            if problem.is_some() {
                state.problem = problem;
            }
        });
    }

    fn toast(&self, style: ToastStyle, title: String, message: Option<String>) {
        let toast = compass_ipc::ExtensionToast {
            title,
            message: message.unwrap_or_default(),
            style: match style {
                ToastStyle::Info => compass_ipc::ExtensionToastStyle::Info,
                ToastStyle::Animated => compass_ipc::ExtensionToastStyle::Animated,
                ToastStyle::Success => compass_ipc::ExtensionToastStyle::Success,
                ToastStyle::Failure => compass_ipc::ExtensionToastStyle::Failure,
            },
        };
        self.state.send_modify(|state| {
            state.version += 1;
            state.toast = Some(toast);
        });
    }

    /// Runs `search` with the current query and publishes the view, unless a
    /// later search overtook it.
    async fn render(&self) {
        let (instance, query, generation) = {
            let mut inner = self.lock();
            inner.generation += 1;
            (
                inner.instance.clone(),
                inner.query.clone(),
                inner.generation,
            )
        };
        let instance = match instance {
            None => return,
            Some(Err(problem)) => {
                self.publish_problem(problem);
                return;
            }
            Some(Ok(instance)) => instance,
        };
        let result = instance.search(&query).await;
        let mut inner = self.lock();
        if inner.generation != generation {
            return;
        }
        match result {
            Ok(tree) => {
                inner.index = ActionIndex::from_tree(&tree);
                drop(inner);
                self.publish(tree);
            }
            Err(error) => {
                drop(inner);
                self.publish_problem(format!("{}: {error}", self.title));
            }
        }
    }

    fn publish(&self, tree: ViewTree) {
        let mut view = tree.into_root();
        if let View::List(list) = &mut view
            && !list.search.host_filtering
        {
            list.search.on_change = Some(HandlerId::new(SEARCH_HANDLER));
        }
        let json = serde_json::to_string(&view).ok();
        self.state.send_modify(|state| {
            state.version += 1;
            state.depth = 1;
            state.view = json;
            state.problem = None;
        });
    }

    fn publish_problem(&self, problem: String) {
        self.state.send_modify(|state| {
            state.version += 1;
            state.problem = Some(problem);
        });
    }

    /// Runs the action `handler` names and carries out what it asks for
    /// that the view can: toasts, re-rendering, leaving.
    async fn act(&self, handler: &str) -> Result<Vec<Outcome>, String> {
        let (instance, request) = {
            let mut inner = self.lock();
            let Some(Ok(instance)) = inner.instance.clone() else {
                return Err(format!("{} is not running", self.title));
            };
            let SessionInner { index, pending, .. } = &mut *inner;
            let request = pending
                .begin(
                    index,
                    &self.script,
                    &HandlerId::new(handler),
                    InvocationSource::Primary,
                    ActionPayload::None,
                )
                .map_err(|error| match error {
                    DispatchError::UnknownAction { .. } => {
                        "That action is no longer on screen".to_owned()
                    }
                    other => other.to_string(),
                })?;
            (instance, request)
        };
        let response = instance.invoke(&request).await;
        if let Err(error) = self.lock().pending.complete(&response) {
            tracing::debug!(%error, "a script action's answer matched nothing in flight");
        }
        let mut outcomes = Vec::new();
        let effects = match response {
            ActionResponse::Completed { effects, .. } => effects,
            ActionResponse::Failed { error, .. } => {
                let message = match &error {
                    DispatchError::CapabilityDenied { denial } => {
                        format!("{} is not allowed to use {}", self.title, denial.capability)
                    }
                    other => other.to_string(),
                };
                self.toast(
                    ToastStyle::Failure,
                    "Action failed".to_owned(),
                    Some(message),
                );
                return Ok(outcomes);
            }
        };
        let (mut rerender, mut ended) = (false, false);
        for effect in effects {
            match effect {
                ActionEffect::Toast {
                    style,
                    title,
                    message,
                } => self.toast(style, title, message),
                ActionEffect::Rerender => rerender = true,
                ActionEffect::PopView | ActionEffect::PopToRoot => ended = true,
                ActionEffect::CloseWindow => outcomes.push(Outcome::Close),
                ActionEffect::Hud { text } => outcomes.push(Outcome::Hud(text)),
                ActionEffect::PushView => {}
            }
        }
        if rerender && !ended {
            self.render().await;
        }
        if ended {
            self.end(None);
        }
        Ok(outcomes)
    }
}

/// A host that refuses everything, for an engine built without one.
#[derive(Debug)]
struct RefusingHost;

impl ScriptHost for RefusingHost {
    fn clipboard_write(
        &self,
        _: compass_extension_api::CapabilityGrant,
        _: &str,
    ) -> Result<(), compass_script::HostError> {
        Err(refused())
    }
    fn clipboard_read(
        &self,
        _: compass_extension_api::CapabilityGrant,
    ) -> Result<Option<String>, compass_script::HostError> {
        Err(refused())
    }
    fn clipboard_paste(
        &self,
        _: compass_extension_api::CapabilityGrant,
        _: &str,
    ) -> Result<(), compass_script::HostError> {
        Err(refused())
    }
    fn open(
        &self,
        _: compass_extension_api::CapabilityGrant,
        _: &str,
    ) -> Result<(), compass_script::HostError> {
        Err(refused())
    }
    fn storage_get(
        &self,
        _: compass_extension_api::CapabilityGrant,
        _: &str,
    ) -> Result<Option<String>, compass_script::HostError> {
        Err(refused())
    }
    fn storage_keys(
        &self,
        _: compass_extension_api::CapabilityGrant,
    ) -> Result<Vec<String>, compass_script::HostError> {
        Err(refused())
    }
    fn storage_set(
        &self,
        _: compass_extension_api::CapabilityGrant,
        _: &str,
        _: &str,
    ) -> Result<(), compass_script::HostError> {
        Err(refused())
    }
    fn storage_remove(
        &self,
        _: compass_extension_api::CapabilityGrant,
        _: &str,
    ) -> Result<(), compass_script::HostError> {
        Err(refused())
    }
    fn notify(
        &self,
        _: compass_extension_api::CapabilityGrant,
        _: &str,
        _: &str,
    ) -> Result<(), compass_script::HostError> {
        Err(refused())
    }
    fn run(
        &self,
        _: compass_extension_api::CapabilityGrant,
        _: &str,
        _: &[String],
        _: std::time::Duration,
    ) -> Result<Option<compass_script::ProcessOutcome>, compass_script::HostError> {
        Err(refused())
    }
}

fn refused() -> compass_script::HostError {
    compass_script::HostError::new("This engine has no services for scripts")
}

/// Hot reload: watches the search paths and applies what changed to the
/// engine's scripts and root search, until the watcher stops.
pub async fn watch(state: Arc<RwLock<EngineState>>) {
    let scripts = state.read().await.rhai_scripts();
    let roots = scripts.config().search_paths.clone();
    let mut watcher = match compass_script::ScriptWatcher::new(&roots) {
        Ok(watcher) => watcher,
        Err(err) => {
            tracing::warn!(%err, "cannot watch Rhai scripts; they reload when the launcher opens");
            return;
        }
    };
    tracing::info!(watched = ?watcher.watched(), "watching Rhai scripts");
    while watcher.changed().await {
        let reload = Arc::clone(&scripts);
        let Ok(items) = tokio::task::spawn_blocking(move || reload.reload()).await else {
            continue;
        };
        state.write().await.set_rhai_items(items);
    }
}
