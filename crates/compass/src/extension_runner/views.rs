//! Running view commands as the launcher follows them: each session's
//! [`ViewState`], which the launcher long-polls, and the way back to the
//! extension while its serving thread blocks reading it.
//!
//! Separate because two sides share it: the engine's IPC handlers (through
//! [`Views`]) and the serving thread (through its `ViewHandle`), and the
//! alerts a view shows are answered from either.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use compass_worker_host::session::SessionEvents;
use compass_worker_host::tsapi::Deferral;

/// Running view commands, as the launcher follows them.
///
/// Each session has a [`ViewState`] the launcher long-polls
/// (`ExtensionView`), and a way to reach the extension (`ExtensionEvent`)
/// while the serving thread blocks reading it.
#[derive(Debug, Default)]
pub struct Views {
    next: std::sync::atomic::AtomicU64,
    sessions: std::sync::Mutex<std::collections::HashMap<u64, ViewEntry>>,
}

#[derive(Debug)]
struct ViewEntry {
    state: tokio::sync::watch::Sender<ViewState>,
    events: Arc<std::sync::Mutex<Option<SessionEvents>>>,
    pid: u32,
    /// What a shown alert is waiting to answer.
    asking: Arc<std::sync::Mutex<Option<Asking>>>,
}

/// What a view session shows now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewState {
    /// Bumped on every change.
    pub version: u64,
    /// The view, as `compass_extension_api::View` JSON; `None` before the
    /// first render.
    pub view: Option<String>,
    /// Why it cannot be drawn (a component Compass does not support), or why
    /// it ended.
    pub problem: Option<String>,
    /// Whether the command has ended.
    pub ended: bool,
    /// How many views the extension has pushed, the root one included.
    pub depth: u32,
    /// A confirmation the extension waits on.
    pub alert: Option<compass_ipc::ExtensionAlert>,
    /// The toast the extension shows over its view.
    pub toast: Option<compass_ipc::ExtensionToast>,
}

impl Views {
    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<u64, ViewEntry>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn open(self: &Arc<Self>, pid: u32) -> ViewHandle {
        let session = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (state, _) = tokio::sync::watch::channel(ViewState::default());
        let events = Arc::new(std::sync::Mutex::new(None));
        let asking = Arc::new(std::sync::Mutex::new(None));
        self.lock().insert(
            session,
            ViewEntry {
                state: state.clone(),
                events: Arc::clone(&events),
                pid,
                asking: Arc::clone(&asking),
            },
        );
        ViewHandle {
            session,
            state,
            events,
            asking,
            views: Arc::clone(self),
        }
    }

    /// A session number no view has, for a view this module does not run
    /// (a Rhai script's), so the launcher's session numbers stay unique.
    #[must_use]
    pub fn reserve(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Follows `session`'s state; `None` for a session that is not running.
    #[must_use]
    pub fn watch(&self, session: u64) -> Option<tokio::sync::watch::Receiver<ViewState>> {
        self.lock()
            .get(&session)
            .map(|entry| entry.state.subscribe())
    }

    /// Sends `handler` with `args` to `session`'s extension.
    ///
    /// # Errors
    ///
    /// A sentence: the session is gone, or its worker is.
    pub fn activate(
        &self,
        session: u64,
        handler: &str,
        args: &[serde_json::Value],
    ) -> Result<(), String> {
        let events = self
            .lock()
            .get(&session)
            .map(|entry| Arc::clone(&entry.events))
            .ok_or_else(|| "That extension view has closed".to_owned())?;
        let events = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| "That extension view has not started yet".to_owned())?;
        events
            .handler_activated(
                &compass_extension_api::action::HandlerId::new(handler),
                args,
            )
            .map_err(|err| format!("The extension did not take it: {err}"))
    }

    /// Answers the alert `session` is showing: confirmed or not.
    ///
    /// # Errors
    ///
    /// A sentence: no such session, no alert waiting, or the worker is gone.
    pub fn answer_alert(&self, session: u64, confirmed: bool) -> Result<(), String> {
        self.answer_with(session, if confirmed { Answer::Yes } else { Answer::No })
    }

    /// Answers the alert `session` is showing with its third choice, the one
    /// that is remembered (a host program's "Always Allow"). An alert with no
    /// third choice takes it as confirming.
    ///
    /// # Errors
    ///
    /// A sentence: no such session, no alert waiting, or the worker is gone.
    pub fn answer_alert_always(&self, session: u64) -> Result<(), String> {
        self.answer_with(session, Answer::Always)
    }

    fn answer_with(&self, session: u64, answer: Answer) -> Result<(), String> {
        let (events, asking, state) = {
            let sessions = self.lock();
            let entry = sessions
                .get(&session)
                .ok_or_else(|| "That extension view has closed".to_owned())?;
            (
                Arc::clone(&entry.events),
                Arc::clone(&entry.asking),
                entry.state.clone(),
            )
        };
        let asking = asking
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| "That extension is not asking anything".to_owned())?;
        state.send_modify(|state| {
            state.version += 1;
            state.alert = None;
        });
        let events = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| "That extension view has not started yet".to_owned())?;
        asking.settle(answer, &events)
    }

    /// Pops `session`'s top view, as Escape on a pushed view does. An alert
    /// the view was showing is answered "no" first: leaving a view is one of
    /// the ways out of a dialog that is not its confirm button.
    ///
    /// # Errors
    ///
    /// A sentence: the session is gone, or its worker is.
    pub fn pop(&self, session: u64) -> Result<(), String> {
        let (events, asking, state) = {
            let sessions = self.lock();
            let entry = sessions
                .get(&session)
                .ok_or_else(|| "That extension view has closed".to_owned())?;
            (
                Arc::clone(&entry.events),
                Arc::clone(&entry.asking),
                entry.state.clone(),
            )
        };
        let events = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| "That extension view has not started yet".to_owned())?;
        settle(&asking, &state, Some(&events));
        events
            .view_popped()
            .map_err(|err| format!("The extension did not take it: {err}"))
    }

    /// Ends `session`: its runtime is stopped. `false` when it was not running.
    pub fn close(&self, session: u64) -> bool {
        let Some(entry) = self.lock().remove(&session) else {
            return false;
        };
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(entry.pid.to_string())
            .status();
        true
    }
}

/// The serving thread's side of one view session.
pub(super) struct ViewHandle {
    pub(super) session: u64,
    pub(super) state: tokio::sync::watch::Sender<ViewState>,
    events: Arc<std::sync::Mutex<Option<SessionEvents>>>,
    asking: Arc<std::sync::Mutex<Option<Asking>>>,
    views: Arc<Views>,
}

impl ViewHandle {
    pub(super) fn attach(&self, events: SessionEvents) {
        *self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(events);
    }

    pub(super) fn publish(
        &self,
        view: Result<compass_extension_api::View, compass_worker_host::view_model::Unsupported>,
        depth: u32,
    ) {
        // The extension navigated (pushed or popped a view) while a dialog
        // was open: that is a way out of the dialog, and it answers "no".
        // Its first render is not navigating: a question asked while the
        // command loads (a host program's consent) stays open across it.
        let previous = self.state.borrow().depth;
        if previous != 0 && previous != depth {
            self.settle();
        }
        self.state.send_modify(|state| {
            state.version += 1;
            state.depth = depth;
            match view {
                Ok(view) => {
                    state.view = serde_json::to_string(&view).ok();
                    state.problem = None;
                }
                Err(unsupported) => state.problem = Some(unsupported.to_string()),
            }
        });
    }

    /// Shows `alert` and holds `deferral` until the launcher answers. An
    /// alert already showing is answered "no" first, as `AlertModel` cancels
    /// the one a second replaces: it is some promise the extension is still
    /// waiting on.
    pub(super) fn ask(&self, alert: compass_ipc::ExtensionAlert, deferral: Deferral) {
        self.show(alert, Asking::Alert(deferral));
    }

    fn show(&self, alert: compass_ipc::ExtensionAlert, asking: Asking) {
        self.settle();
        *self
            .asking
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(asking);
        self.state.send_modify(|state| {
            state.version += 1;
            state.alert = Some(alert);
        });
    }

    /// Answers "no" to the alert this view is showing, if it is showing one.
    fn settle(&self) {
        let events = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        settle(&self.asking, &self.state, events.as_ref());
    }

    pub(super) fn end(self, why: Option<String>) {
        self.state.send_modify(|state| {
            state.version += 1;
            state.ended = true;
            if why.is_some() {
                state.problem = why;
            }
        });
        self.views.lock().remove(&self.session);
    }
}

impl crate::host_commands::Asker for ViewHandle {
    /// Shows `alert`; `answered` gets the person's answer, or [`Answer::No`]
    /// if the view navigates or another alert replaces it.
    fn ask(&self, alert: compass_ipc::ExtensionAlert, answered: crate::host_commands::Answered) {
        self.show(alert, Asking::Consent(answered));
    }
}

/// What a person answered an alert with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Escape, or any way out of the alert that is not a button.
    No,
    /// Enter.
    Yes,
    /// The third, remembered choice.
    Always,
}

/// What a shown alert answers.
pub(crate) enum Asking {
    /// An extension's `confirmAlert`, which gets `true` or `false`.
    Alert(Deferral),
    /// The engine's own question, asked for an extension (a host program's
    /// consent); the engine acts on the answer.
    Consent(Box<dyn FnOnce(Answer) + Send>),
}

impl std::fmt::Debug for Asking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alert(deferral) => f.debug_tuple("Alert").field(deferral).finish(),
            Self::Consent(_) => f.write_str("Consent"),
        }
    }
}

impl Asking {
    fn settle(self, answer: Answer, events: &SessionEvents) -> Result<(), String> {
        match self {
            Self::Alert(deferral) => events
                .answer(&deferral, serde_json::json!(answer != Answer::No))
                .map_err(|err| format!("The extension did not take the answer: {err}")),
            Self::Consent(answered) => {
                answered(answer);
                Ok(())
            }
        }
    }
}

/// Takes what the shown alert is waiting on, if anything, clears it from
/// `state` and answers it "no": through `events` for an extension's alert,
/// to the engine for its own question.
fn settle(
    asking: &std::sync::Mutex<Option<Asking>>,
    state: &tokio::sync::watch::Sender<ViewState>,
    events: Option<&SessionEvents>,
) {
    let Some(asking) = asking
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return;
    };
    state.send_modify(|state| {
        state.version += 1;
        state.alert = None;
    });
    let answered = match (asking, events) {
        (Asking::Consent(answered), _) => {
            answered(Answer::No);
            Ok(())
        }
        (asking, Some(events)) => asking.settle(Answer::No, events),
        (Asking::Alert(_), None) => Err("no session".to_owned()),
    };
    if answered.is_err() {
        tracing::warn!("could not answer a dismissed dialog; the extension may wait on it");
    }
}
