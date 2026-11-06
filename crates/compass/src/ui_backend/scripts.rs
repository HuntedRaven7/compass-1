//! Script commands: listing them and their icons, running one and reading
//! its output, and the permissions a Rhai script has been granted.
//!
//! The bodies of these [`ApplicationBackend`](compass_ui::backend::ApplicationBackend)
//! requests, apart from the rest with the conversions from the wire only they
//! use.

use compass_ipc::Request;
use compass_ui::backend::{BackendFuture, ScriptOutputState};

use super::DaemonBackend;

impl DaemonBackend {
    pub(super) fn list_script_grants(
        &self,
    ) -> BackendFuture<'_, Vec<compass_ui::backend::ScriptGrant>> {
        Box::pin(async move {
            match self
                .ask(Request::ListScriptGrants, "Reading script permissions")
                .await?
            {
                compass_ipc::Response::ScriptGrants { grants } => {
                    Ok(grants.into_iter().map(script_grant).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn revoke_script_grant(
        &self,
        id: String,
    ) -> BackendFuture<'_, Vec<compass_ui::backend::ScriptGrant>> {
        Box::pin(async move {
            match self
                .ask(Request::RevokeScriptGrant { id }, "Revoking permissions")
                .await?
            {
                compass_ipc::Response::ScriptGrants { grants } => {
                    Ok(grants.into_iter().map(script_grant).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn list_scripts(
        &self,
    ) -> BackendFuture<'_, Vec<compass_core::script_scan::ScriptItem>> {
        Box::pin(async move {
            match self
                .ask(Request::ListScripts, "Listing script commands")
                .await?
            {
                compass_ipc::Response::Scripts { scripts } => {
                    Ok(scripts.into_iter().map(script_item).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn list_rhai_scripts(
        &self,
    ) -> BackendFuture<'_, Vec<compass_core::rhai_scripts::RhaiScriptItem>> {
        Box::pin(async move {
            match self
                .ask(Request::ListRhaiScripts, "Listing Rhai scripts")
                .await?
            {
                compass_ipc::Response::RhaiScripts { scripts } => Ok(scripts
                    .into_iter()
                    .map(|script| compass_core::rhai_scripts::RhaiScriptItem {
                        id: script.id,
                        title: script.title,
                        description: script.description,
                        icon: script.icon,
                        keywords: script.keywords,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn run_script(
        &self,
        id: String,
        arguments: Vec<String>,
    ) -> BackendFuture<'_, Option<u64>> {
        Box::pin(async move {
            match self
                .ask(Request::RunScript { id, arguments }, "Running the script")
                .await?
            {
                compass_ipc::Response::ScriptStarted { session } => Ok(session),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn script_output(&self, session: u64) -> BackendFuture<'_, ScriptOutputState> {
        Box::pin(async move {
            match self
                .ask(
                    Request::ScriptOutput { session },
                    "Reading the script's output",
                )
                .await?
            {
                compass_ipc::Response::ScriptOutput {
                    output,
                    finished,
                    exit_code,
                    elapsed_ms,
                } => Ok(ScriptOutputState {
                    output,
                    finished,
                    exit_code,
                    elapsed_ms,
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn stop_script(&self, session: u64) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::StopScript { session }, "Stopping the script")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    pub(super) fn script_icons(&self) -> BackendFuture<'_, Vec<(String, String)>> {
        Box::pin(async move {
            match self
                .ask(Request::ScriptIcons, "Listing script icons")
                .await?
            {
                compass_ipc::Response::ScriptIcons { icons } => Ok(icons),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

fn script_grant(entry: compass_ipc::ScriptGrantEntry) -> compass_ui::backend::ScriptGrant {
    compass_ui::backend::ScriptGrant {
        id: entry.id,
        title: entry.title,
        capabilities: entry.capabilities,
        descriptions: entry.descriptions,
    }
}

/// A script as the launcher holds it, from the wire.
fn script_item(entry: compass_ipc::ScriptEntry) -> compass_core::script_scan::ScriptItem {
    use compass_core::script_command::{
        ArgumentDataOption, ArgumentType, OutputMode, ScriptArgument,
    };
    compass_core::script_scan::ScriptItem {
        id: entry.id,
        title: entry.title,
        subtitle: entry.subtitle,
        keywords: entry.keywords,
        mode: OutputMode::parse(&entry.mode).unwrap_or_default(),
        needs_confirmation: entry.needs_confirmation,
        path: entry.path,
        arguments: entry
            .arguments
            .into_iter()
            .map(|argument| ScriptArgument {
                argument_type: match argument.kind.as_str() {
                    "password" => ArgumentType::Password,
                    "dropdown" => ArgumentType::Dropdown,
                    _ => ArgumentType::Text,
                },
                placeholder: argument.placeholder,
                optional: argument.optional,
                // The engine encodes; the launcher only asks.
                percent_encoded: false,
                data: argument
                    .options
                    .into_iter()
                    .next()
                    .map(|(title, value)| ArgumentDataOption { title, value }),
            })
            .collect(),
    }
}
