//! Set Default Browser and Set Default Terminal: listing the candidates and
//! writing the user's choice where the desktop reads it.

use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, Response};
use tokio::sync::RwLock;

use super::EngineState;
use super::engine_apps_now;
use super::media::show_hud;

/// Set Default Browser's and Set Default Terminal's list, as
/// `SetDefaultBrowserViewHost::reloadItems` and its terminal twin build it:
/// what opens [`compass_core::default_app::BROWSER_PROBE_URL`], or every
/// terminal emulator, with the current default first.
pub(super) async fn list_default_apps(
    state: &Arc<RwLock<EngineState>>,
    kind: compass_ipc::DefaultAppKind,
) -> Response {
    use compass_core::default_app::{PickerApp, browser_picker, terminal_picker};
    use compass_worker_host::application_service::Apps as _;
    let state = state.read().await;
    let apps = engine_apps_now(&state);
    let service = compass_core::app_service::AppService::new(&state.search.index);
    let picker_app = |app: &compass_worker_host::application_service::Application| {
        let item = service.find_by_id(&app.id);
        PickerApp {
            id: app.id.clone(),
            display_name: app.name.clone(),
            description: item
                .and_then(compass_core::AppItem::comment)
                .unwrap_or_default()
                .to_owned(),
            // The index holds only what the root shows, so everything in it
            // is `displayable()`.
            displayable: true,
            terminal_emulator: item
                .is_some_and(|item| item.categories().iter().any(|c| c == "TerminalEmulator")),
        }
    };
    let picker = match kind {
        compass_ipc::DefaultAppKind::Browser => {
            let default = apps.web_browser().map(|app| app.id);
            let openers: Vec<PickerApp> = apps
                .openers(compass_core::default_app::BROWSER_PROBE_URL)
                .iter()
                .map(picker_app)
                .collect();
            browser_picker(&openers, default.as_deref())
        }
        compass_ipc::DefaultAppKind::Terminal => {
            let default = apps.terminal_emulator().map(|app| app.id);
            let terminals: Vec<PickerApp> =
                apps.terminal_emulators().iter().map(picker_app).collect();
            terminal_picker(&terminals, default.as_deref())
        }
    };
    Response::DefaultApps {
        apps: picker
            .items
            .into_iter()
            .map(|item| compass_ipc::DefaultAppEntry {
                id: item.app.id,
                name: item.app.display_name,
                description: item.app.description,
                is_default: item.is_default,
            })
            .collect(),
    }
}

/// The picker's action: `appDb->setWebBrowser(app)` into the user's
/// `mimeapps.list`, or `xdgpp::setDefaultTerminal(id)` into the user's
/// `xdg-terminals.list`. A failure answers with the picker's own sentence.
pub(super) async fn set_default_app(
    state: &Arc<RwLock<EngineState>>,
    kind: compass_ipc::DefaultAppKind,
    id: &str,
) -> Response {
    use compass_core::default_app::{BROWSER_FAILURE, TERMINAL_FAILURE};
    let failure = |sentence: &str| {
        Response::Error(ProtocolError::new(ErrorKind::Internal, sentence.to_owned()))
    };
    let Some(config_home) = compass_xdg::mimeapps::config_home() else {
        return failure(match kind {
            compass_ipc::DefaultAppKind::Browser => BROWSER_FAILURE,
            compass_ipc::DefaultAppKind::Terminal => TERMINAL_FAILURE,
        });
    };
    match kind {
        compass_ipc::DefaultAppKind::Browser => {
            let mut apps = engine_apps_now(&*state.read().await);
            let id = id.to_owned();
            let set = tokio::task::spawn_blocking(move || {
                apps.set_web_browser(
                    &id,
                    &config_home.join("mimeapps.list"),
                    &compass_xdg::mimeapps::search_paths(),
                )
            })
            .await
            .unwrap_or(false);
            if set {
                tokio::spawn(show_hud(
                    state.read().await.window_slot(),
                    compass_core::default_app::BROWSER_SUCCESS.to_owned(),
                    Some("globe-01"),
                ));
                Response::Ack
            } else {
                failure(BROWSER_FAILURE)
            }
        }
        compass_ipc::DefaultAppKind::Terminal => {
            match compass_xdg::terminal::set_default_terminal(
                &config_home.join("xdg-terminals.list"),
                id,
                None,
            ) {
                Ok(()) => {
                    tokio::spawn(show_hud(
                        state.read().await.window_slot(),
                        compass_core::default_app::TERMINAL_SUCCESS.to_owned(),
                        None,
                    ));
                    Response::Ack
                }
                Err(error) => {
                    tracing::warn!(%error, "could not write xdg-terminals.list");
                    failure(TERMINAL_FAILURE)
                }
            }
        }
    }
}
