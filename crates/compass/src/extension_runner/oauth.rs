//! OAuth sign-ins waiting on a browser: an extension's `OAuth/authorize`
//! opens the provider's page, and the redirect comes back as a deeplink over
//! IPC, maybe long after.
//!
//! Separate because the waiting outlives any one call into a run: the
//! pending authorizations are process-wide, keyed by the URL's `state`, and
//! [`oauth_redirect`] is reached from the engine, not from the session.

use compass_worker_host::oauth_service::{AuthorizeRequest, Authorizer, Redirect};
use compass_worker_host::session::SessionEvents;
use compass_worker_host::tsapi::Deferral;

use super::views::ViewState;

/// How the toast a view shows while an OAuth sign-in waits on the browser
/// begins; the provider's name follows.
pub const SIGN_IN_TOAST: &str = "Continue in your browser to connect";

/// Authorizations waiting on a browser, by the `state` their URL carries.
///
/// Process-wide, like the C++ `OAuthService`'s request map: the redirect
/// arrives as a deeplink over IPC, with nothing but the `state` to say which
/// extension's call it answers.
pub(super) static OAUTH: std::sync::LazyLock<OAuthRequests> =
    std::sync::LazyLock::new(OAuthRequests::default);

#[derive(Default)]
pub(super) struct OAuthRequests(
    std::sync::Mutex<std::collections::HashMap<String, PendingAuthorization>>,
);

struct PendingAuthorization {
    session_id: String,
    provider: String,
    events: SessionEvents,
    deferral: Deferral,
    view: Option<tokio::sync::watch::Sender<ViewState>>,
}

impl OAuthRequests {
    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, PendingAuthorization>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Forgets `session_id`'s authorizations: its worker is gone, so there is
    /// no promise left to settle.
    pub(super) fn abandon(&self, session_id: &str) {
        self.lock()
            .retain(|_, pending| pending.session_id != session_id);
    }
}

/// Takes the `OAuth/authorize` call the service defers, for the serving
/// loop to begin.
#[derive(Debug, Default)]
pub(super) struct EngineAuthorizer(pub(super) std::sync::Mutex<Option<AuthorizeRequest>>);

impl Authorizer for EngineAuthorizer {
    fn authorize(&self, request: AuthorizeRequest, _deferral: &Deferral) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(request);
    }
}

/// Opens `request`'s URL in the browser and waits for [`oauth_redirect`].
/// While it waits, the view says where the person has to go.
pub(super) fn begin_authorization(
    session_id: &str,
    request: AuthorizeRequest,
    deferral: Deferral,
    events: SessionEvents,
    view: Option<tokio::sync::watch::Sender<ViewState>>,
    apps: Option<&crate::extension_apps::EngineApps>,
) -> Result<(), String> {
    use compass_worker_host::application_service::Apps as _;
    let state = request
        .state
        .clone()
        .ok_or("the authorization URL has no state parameter")?;
    let apps = apps.ok_or("Compass cannot open a browser for this command")?;
    let browser = apps
        .default_opener(&request.url)
        .ok_or("No web browser is installed to sign in with")?;
    let provider = if request.provider.is_empty() {
        "the provider".to_owned()
    } else {
        request.provider.clone()
    };
    if let Some(view) = &view {
        view.send_modify(|state| {
            state.version += 1;
            state.toast = Some(compass_ipc::ExtensionToast {
                title: format!("{SIGN_IN_TOAST} {provider}"),
                message: request.description.clone(),
                style: compass_ipc::ExtensionToastStyle::Animated,
            });
        });
    }
    OAUTH.lock().insert(
        state,
        PendingAuthorization {
            session_id: session_id.to_owned(),
            provider,
            events,
            deferral,
            view,
        },
    );
    apps.launch(&browser, &request.url);
    Ok(())
}

/// Answers the authorization a provider's redirect names: `url` is the
/// `raycast://oauth?code=…&state=…` deeplink the desktop handed `compass`.
///
/// # Errors
///
/// A sentence: not an OAuth redirect, no authorization waiting on its state,
/// or the extension could not be told.
pub fn oauth_redirect(url: &str) -> Result<(), String> {
    let redirect = Redirect::parse(url)?;
    let pending = OAUTH
        .lock()
        .remove(redirect.state())
        .ok_or("No extension is waiting on that authorization; it may have been closed")?;
    let (answered, toast) = match &redirect {
        Redirect::Code { code, .. } => (
            pending
                .events
                .answer(&pending.deferral, serde_json::json!({ "code": code })),
            compass_ipc::ExtensionToast {
                title: format!("Connected to {}", pending.provider),
                message: String::new(),
                style: compass_ipc::ExtensionToastStyle::Success,
            },
        ),
        Redirect::Refused { reason, .. } => (
            pending.events.fail(&pending.deferral, reason),
            compass_ipc::ExtensionToast {
                title: format!("{} did not connect", pending.provider),
                message: reason.clone(),
                style: compass_ipc::ExtensionToastStyle::Failure,
            },
        ),
    };
    if let Some(view) = &pending.view {
        view.send_modify(|state| {
            state.version += 1;
            state.toast = Some(toast);
        });
    }
    answered.map_err(|err| format!("The extension did not take the authorization: {err}"))
}
