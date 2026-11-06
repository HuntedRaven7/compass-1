//! The settings view: fonts and theme, settings and providers, the
//! launcher hotkey and other shortcuts, start at login, default
//! applications and updates.
//!
//! The bodies of these [`ApplicationBackend`](compass_ui::backend::ApplicationBackend)
//! requests, apart from the rest because several of them wait on the desktop
//! (a shortcut probe, the hotkey and login start dialogs) or the release feed,
//! each with its own deadline.

use compass_ipc::Request;
use compass_ui::backend::BackendFuture;

use super::{
    DaemonBackend, LAUNCHER_TIMEOUT, LOGIN_START_TIMEOUT, PROBE_TIMEOUT, REQUEST_TIMEOUT,
    UPDATE_TIMEOUT,
};

impl DaemonBackend {
    pub(super) fn list_default_apps(
        &self,
        kind: compass_ui::backend::DefaultApp,
    ) -> BackendFuture<'_, Vec<compass_ui::backend::DefaultAppRow>> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ListDefaultApps {
                        kind: default_app_kind(kind),
                    },
                    "Listing the applications",
                )
                .await?
            {
                compass_ipc::Response::DefaultApps { apps } => Ok(apps
                    .into_iter()
                    .map(|app| compass_ui::backend::DefaultAppRow {
                        id: app.id,
                        name: app.name,
                        description: app.description,
                        is_default: app.is_default,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn set_default_app(
        &self,
        kind: compass_ui::backend::DefaultApp,
        id: String,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::SetDefaultApp {
                        kind: default_app_kind(kind),
                        id,
                    },
                    "Setting the default",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn set_font(&self, family: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::SetFont { family }, "Setting the font")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn set_setting(
        &self,
        key: String,
        value: serde_json::Value,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let value_json = value.to_string();
            match self
                .ask(
                    Request::SetSetting { key, value_json },
                    "Saving the setting",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn probe_shortcut(&self, trigger: String) -> BackendFuture<'_, Option<String>> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::ProbeShortcut { trigger },
                    "Asking the desktop about the shortcut",
                    PROBE_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::ShortcutProbe { refusal } => Ok(refusal),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn launcher_shortcut(
        &self,
        request: bool,
    ) -> BackendFuture<'_, compass_core::launcher_shortcut::LauncherShortcut> {
        Box::pin(async move {
            let deadline = if request {
                LAUNCHER_TIMEOUT
            } else {
                REQUEST_TIMEOUT
            };
            match self
                .ask_within(
                    Request::LauncherShortcut { request },
                    "Asking the desktop for the launcher hotkey",
                    deadline,
                )
                .await?
            {
                compass_ipc::Response::LauncherShortcut { state } => {
                    Ok(crate::permissions::launcher_from_wire(state))
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn login_start(
        &self,
        enable: Option<bool>,
    ) -> BackendFuture<'_, compass_core::login_start::Status> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::LoginStart { enable },
                    "Changing whether Compass starts at login",
                    LOGIN_START_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::LoginStart { state, unit } => {
                    Ok(crate::permissions::login_start_from_wire(state, unit))
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn set_shortcut_capture(&self, capturing: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ShortcutCapture { capturing },
                    "Suspending the global shortcuts",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn update_status(
        &self,
    ) -> BackendFuture<'_, Option<compass_ui::backend::UpdateOffer>> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::UpdateStatus,
                    "Checking for updates",
                    UPDATE_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::UpdateStatus { current, available } => {
                    Ok(available.map(|offer| compass_ui::backend::UpdateOffer {
                        tag: offer.tag,
                        version: offer.version,
                        release_url: offer.release_url,
                        current,
                    }))
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn skip_update(&self, tag: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::SkipUpdate { tag }, "Skipping the update")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn set_provider_enabled(
        &self,
        provider: String,
        enabled: bool,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::SetProviderEnabled { provider, enabled },
                    "Changing the provider",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn set_theme(&self, theme: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::SetTheme { theme }, "Saving the theme")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn list_fonts(&self) -> BackendFuture<'_, compass_ui::backend::FontList> {
        Box::pin(async move {
            match self.ask(Request::ListFonts, "Listing the fonts").await? {
                compass_ipc::Response::Fonts { fonts, categories } => {
                    Ok(compass_ui::backend::FontList {
                        fonts: fonts
                            .into_iter()
                            .map(|font| compass_ui::backend::FontListEntry {
                                name: font.name,
                                family: font.family,
                                glyph: font.glyph,
                                color: font.color,
                                primary: font.primary,
                                categories: font.categories,
                            })
                            .collect(),
                        categories,
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn font_specimen(&self, name: String) -> BackendFuture<'_, String> {
        Box::pin(async move {
            match self
                .ask(Request::FontSpecimen { name }, "Loading the specimen")
                .await?
            {
                compass_ipc::Response::Text { text } => Ok(text),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

fn default_app_kind(kind: compass_ui::backend::DefaultApp) -> compass_ipc::DefaultAppKind {
    match kind {
        compass_ui::backend::DefaultApp::Browser => compass_ipc::DefaultAppKind::Browser,
        compass_ui::backend::DefaultApp::Terminal => compass_ipc::DefaultAppKind::Terminal,
    }
}
