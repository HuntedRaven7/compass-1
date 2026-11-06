//! `compass login-start`: the Settings switch "Start at login", from a
//! terminal.
//!
//! The engine owns the change, as it does for the switch: it asks the
//! Background portal, or writes its own autostart entry where there is no
//! portal (`compass_platform_linux::LinuxLoginStart`). This module sends the
//! request and reports the answer, so a script or a person without the window
//! sees the same state and the same sentences Settings shows.

use std::process::ExitCode;

use anyhow::{Result, bail};
use compass_core::login_start::{Mechanism, State, Status, Unit};
use compass_ipc::{Request, Response, SocketPath};

use crate::cli::LoginStartCommand;
use crate::{EXIT_FAILURE, EXIT_OK, ipc, permissions};

/// Runs a `compass login-start` subcommand.
///
/// `status` exits 0 whatever the state; `enable` and `disable` exit 0 only
/// when the state afterwards is the one asked for, so a refusal or a session
/// without a mechanism is a failure a script can see.
///
/// # Errors
///
/// When no engine answers, or the engine could not carry out the change.
pub async fn run(socket: &SocketPath, command: LoginStartCommand) -> Result<ExitCode> {
    let (enable, json) = match command {
        LoginStartCommand::Status { json } => (None, json),
        LoginStartCommand::Enable { json } => (Some(true), json),
        LoginStartCommand::Disable { json } => (Some(false), json),
    };
    let status = match ipc::send(socket, Request::LoginStart { enable }).await? {
        Response::LoginStart { state, unit } => permissions::login_start_from_wire(state, unit),
        other => bail!("unexpected answer from the engine: {other:?}"),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&to_json(&status))?);
    } else {
        println!("{}", status.describe());
    }
    let settled = enable.is_none_or(|wanted| status.state.is_on() == wanted);
    Ok(ExitCode::from(if settled { EXIT_OK } else { EXIT_FAILURE }))
}

/// The status as JSON: `state` (`off`, `on`, `refused`, `unavailable`),
/// `mechanism` when on (`portal`, `autostart-file`), `unit` (`installed`,
/// `enabled`, or null) and the sentence Settings shows as `description`.
#[must_use]
pub fn to_json(status: &Status) -> serde_json::Value {
    let (state, mechanism) = match status.state {
        State::Off => ("off", None),
        State::On {
            mechanism: Mechanism::Portal,
        } => ("on", Some("portal")),
        State::On {
            mechanism: Mechanism::AutostartFile,
        } => ("on", Some("autostart-file")),
        State::Refused => ("refused", None),
        State::Unavailable => ("unavailable", None),
    };
    let unit = status.unit.map(|unit| match unit {
        Unit::Installed => "installed",
        Unit::Enabled => "enabled",
    });
    let mut value = serde_json::json!({
        "state": state,
        "unit": unit,
        "description": status.describe(),
    });
    if let Some(mechanism) = mechanism {
        value["mechanism"] = serde_json::Value::from(mechanism);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_names_the_state_mechanism_and_unit() {
        let on = Status {
            state: State::On {
                mechanism: Mechanism::Portal,
            },
            unit: Some(Unit::Enabled),
        };
        let json = to_json(&on);
        assert_eq!(json["state"], "on");
        assert_eq!(json["mechanism"], "portal");
        assert_eq!(json["unit"], "enabled");
        assert_eq!(json["description"], on.describe());

        let off = to_json(&Status::default());
        assert_eq!(off["state"], "off");
        assert!(off.get("mechanism").is_none());
        assert!(off["unit"].is_null());

        let file = to_json(&Status {
            state: State::On {
                mechanism: Mechanism::AutostartFile,
            },
            unit: None,
        });
        assert_eq!(file["mechanism"], "autostart-file");
        assert_eq!(
            to_json(&Status {
                state: State::Refused,
                unit: None
            })["state"],
            "refused"
        );
    }
}
