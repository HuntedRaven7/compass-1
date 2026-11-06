//! Extensions and the extension store: starting a command, reading and
//! driving its view, its preferences and file choices, and browsing,
//! installing and removing extensions.
//!
//! The bodies of these [`ApplicationBackend`](compass_ui::backend::ApplicationBackend)
//! requests, apart from the rest because they are the largest domain the
//! launcher asks about and carry their own conversions from the wire, and
//! because the requests the engine may hold (a view, a start, a store fetch)
//! are all here.

use compass_ipc::Request;
use compass_ui::backend::{BackendFuture, ExtensionDraft, ExtensionStart, ExtensionViewState};

use super::{DaemonBackend, INSTALL_TIMEOUT, START_TIMEOUT, STORE_TIMEOUT, VIEW_TIMEOUT};

impl DaemonBackend {
    pub(super) fn create_extension(&self, draft: ExtensionDraft) -> BackendFuture<'_, String> {
        Box::pin(async move {
            let request = Request::CreateExtension {
                author: draft.author,
                title: draft.title,
                description: draft.description,
                location: draft.location,
                command_title: draft.command_title,
                command_description: draft.command_description,
                template: draft.template,
            };
            match self.ask(request, "Creating the extension").await? {
                compass_ipc::Response::ExtensionCreated { path } => Ok(path),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn store_browse(
        &self,
        store: compass_ui::backend::Store,
        query: String,
    ) -> BackendFuture<'_, compass_ui::backend::StoreList> {
        Box::pin(async move {
            let request = Request::StoreBrowse {
                store: store_kind(store),
                query,
            };
            match self
                .ask_within(request, "Loading the store", STORE_TIMEOUT)
                .await?
            {
                compass_ipc::Response::StoreListing { heading, entries } => {
                    Ok(compass_ui::backend::StoreList {
                        heading,
                        rows: entries.into_iter().map(store_row).collect(),
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn store_extension(
        &self,
        store: compass_ui::backend::Store,
        owner: String,
        name: String,
    ) -> BackendFuture<'_, compass_ui::backend::StoreDetail> {
        Box::pin(async move {
            let request = Request::StoreExtension {
                store: store_kind(store),
                owner,
                name,
            };
            match self
                .ask_within(request, "Loading the extension", STORE_TIMEOUT)
                .await?
            {
                compass_ipc::Response::StoreExtension { detail } => {
                    Ok(compass_ui::backend::StoreDetail {
                        row: store_row(detail.entry),
                        markdown: detail.markdown,
                        screenshots: detail.screenshots,
                        readme_url: detail.readme_url,
                        source_url: detail.source_url,
                        store_url: detail.store_url,
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn store_install(
        &self,
        store: compass_ui::backend::Store,
        owner: String,
        name: String,
    ) -> BackendFuture<'_, (String, String)> {
        Box::pin(async move {
            let request = Request::StoreInstall {
                store: store_kind(store),
                owner,
                name,
            };
            match self
                .ask_within(request, "Installing the extension", INSTALL_TIMEOUT)
                .await?
            {
                compass_ipc::Response::StoreInstalled { id, title } => Ok((id, title)),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn store_uninstall(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::StoreUninstall { id },
                    "Uninstalling the extension",
                    STORE_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn fetch_launch(
        &self,
        token: u64,
    ) -> BackendFuture<'_, compass_ui::backend::ExtensionLaunch> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionLaunchFetch { token },
                    "Taking the extension's launch",
                )
                .await?
            {
                compass_ipc::Response::ExtensionLaunch {
                    id,
                    arguments_json,
                    preferences,
                } => Ok(compass_ui::backend::ExtensionLaunch {
                    id,
                    arguments: launch_arguments(arguments_json)?,
                    preferences,
                    fallback_text: None,
                }),
                compass_ipc::Response::CommandLaunch {
                    id,
                    arguments_json,
                    fallback_text,
                } => Ok(compass_ui::backend::ExtensionLaunch {
                    id,
                    arguments: launch_arguments(arguments_json)?,
                    preferences: false,
                    fallback_text,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn extension_subtitles(&self) -> BackendFuture<'_, Vec<(String, String)>> {
        Box::pin(async move {
            match self
                .ask(Request::ExtensionSubtitles, "Reading command subtitles")
                .await?
            {
                compass_ipc::Response::ExtensionSubtitles { subtitles } => Ok(subtitles),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn extension_preferences(&self, id: String) -> BackendFuture<'_, ExtensionStart> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionPreferences { id },
                    "Reading the preferences",
                )
                .await?
            {
                compass_ipc::Response::ExtensionNeedsPreferences { title, fields } => {
                    Ok(ExtensionStart::NeedsPreferences {
                        title,
                        fields: fields.into_iter().map(preference_input).collect(),
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn run_extension_command(
        &self,
        id: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> BackendFuture<'_, ExtensionStart> {
        Box::pin(async move {
            let arguments_json =
                arguments.map(|arguments| serde_json::Value::Object(arguments).to_string());
            match self
                .ask_within(
                    Request::RunExtensionCommand { id, arguments_json },
                    "Running the command",
                    START_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(ExtensionStart::Ran),
                compass_ipc::Response::ExtensionStarted { session } => {
                    Ok(ExtensionStart::View(session))
                }
                compass_ipc::Response::ExtensionNeedsPreferences { title, fields } => {
                    Ok(ExtensionStart::NeedsPreferences {
                        title,
                        fields: fields.into_iter().map(preference_input).collect(),
                    })
                }
                compass_ipc::Response::ExtensionNeedsArguments { title, fields } => {
                    Ok(ExtensionStart::NeedsArguments {
                        title,
                        fields: fields.into_iter().map(preference_input).collect(),
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn extension_view(
        &self,
        session: u64,
        after: u64,
    ) -> BackendFuture<'_, ExtensionViewState> {
        Box::pin(async move {
            match self
                .ask_within(
                    Request::ExtensionView { session, after },
                    "Reading the view",
                    VIEW_TIMEOUT,
                )
                .await?
            {
                compass_ipc::Response::ExtensionView {
                    version,
                    view_json,
                    problem,
                    ended,
                    depth,
                    alert,
                    toast,
                } => Ok(ExtensionViewState {
                    depth,
                    toast: toast.map(|toast| compass_ui::backend::ExtensionToast {
                        failure: toast.style == compass_ipc::ExtensionToastStyle::Failure,
                        animated: toast.style == compass_ipc::ExtensionToastStyle::Animated,
                        title: toast.title,
                        message: toast.message,
                    }),
                    alert: alert.map(|alert| compass_ui::backend::ExtensionPrompt {
                        title: alert.title,
                        message: alert.message,
                        confirm_text: alert.confirm_text,
                        cancel_text: alert.cancel_text,
                        remember_text: alert.remember_text,
                    }),
                    version,
                    view: view_json
                        .map(|json| serde_json::from_str(&json).map(Box::new))
                        .transpose()
                        .map_err(|err| {
                            format!("The engine sent a view this launcher cannot read: {err}")
                        })?,
                    problem,
                    ended,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn extension_event(
        &self,
        session: u64,
        handler: String,
        args: Vec<serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let args_json = serde_json::Value::Array(args).to_string();
            match self
                .ask(
                    Request::ExtensionEvent {
                        session,
                        handler,
                        args_json,
                    },
                    "Running the action",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn set_extension_preferences(
        &self,
        id: String,
        values: serde_json::Map<String, serde_json::Value>,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            let values_json = serde_json::Value::Object(values).to_string();
            match self
                .ask(
                    Request::SetExtensionPreferences { id, values_json },
                    "Saving the preferences",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn extension_alert_answer(
        &self,
        session: u64,
        confirmed: bool,
    ) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionAlertAnswer { session, confirmed },
                    "Answering the extension",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn extension_alert_remember(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ExtensionAlertRemember { session },
                    "Answering the extension",
                )
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn extension_pop(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ExtensionPop { session }, "Going back")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn close_extension(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::CloseExtension { session }, "Closing the view")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn choose_files(
        &self,
        choice: compass_ui::extension_fields::FileChoice,
    ) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async move {
            // The portal, not a dialog of our own: inside a Flatpak it is also
            // what grants the extension the file it names.
            let portals =
                compass_portals::Portals::connect(compass_portals::PortalConfig::default())
                    .await
                    .map_err(|err| format!("The file chooser is not available: {err}"))?;
            let chooser = portals
                .file_chooser()
                .map_err(|err| format!("The file chooser is not available: {err}"))?;
            // A picker that takes directories and not files asks for a
            // directory; the portal cannot offer both in one dialog.
            let request = if choice.directories && !choice.files {
                compass_portals::FileChooserRequest::directory("Choose a folder")
            } else {
                compass_portals::FileChooserRequest::file("Choose a file")
            }
            .multiple(choice.multiple);
            let outcome = chooser
                .open(request)
                .await
                .map_err(|err| format!("The file chooser failed: {err}"))?;
            Ok(outcome
                .paths()
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect())
        })
    }
}

const fn store_kind(store: compass_ui::backend::Store) -> compass_ipc::StoreKind {
    match store {
        compass_ui::backend::Store::Vicinae => compass_ipc::StoreKind::Vicinae,
        compass_ui::backend::Store::Raycast => compass_ipc::StoreKind::Raycast,
    }
}

fn store_row(entry: compass_ipc::StoreEntry) -> compass_ui::backend::StoreRow {
    compass_ui::backend::StoreRow {
        id: entry.id,
        name: entry.name,
        author: entry.author,
        author_name: entry.author_name,
        title: entry.title,
        description: entry.description,
        icon_light: entry.icon_light,
        icon_dark: entry.icon_dark,
        downloads: entry.downloads,
        installed: entry.installed,
        update_available: entry.update_available,
        compat: entry.compat,
        author_avatar: entry.author_avatar,
        owner: entry.owner,
    }
}

fn preference_input(field: compass_ipc::PreferenceField) -> compass_ui::backend::PreferenceInput {
    use compass_ipc::PreferenceFieldKind;
    use compass_ui::backend::PreferenceInputKind;
    compass_ui::backend::PreferenceInput {
        kind: match field.kind {
            PreferenceFieldKind::Text => PreferenceInputKind::Text,
            PreferenceFieldKind::Password => PreferenceInputKind::Password,
            PreferenceFieldKind::Checkbox { label } => PreferenceInputKind::Checkbox { label },
            PreferenceFieldKind::Dropdown { options } => PreferenceInputKind::Dropdown { options },
            PreferenceFieldKind::Unsupported { declared } => {
                PreferenceInputKind::Unsupported { declared }
            }
        },
        value: field
            .value_json
            .and_then(|json| serde_json::from_str(&json).ok()),
        name: field.name,
        title: field.title,
        description: field.description,
        placeholder: field.placeholder,
        required: field.required,
    }
}

/// A launch's arguments, from the JSON object the engine carries them as.
fn launch_arguments(
    json: Option<String>,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    json.map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|err| format!("The launch's arguments are unreadable: {err}"))
}
