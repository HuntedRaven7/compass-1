//! The Power Management commands, through logind or the desktop's session
//! manager.

use compass_core::Config;
use compass_ipc::{ErrorKind, ProtocolError, Response};

/// Runs a Power Management command through logind, or the desktop's session
/// manager for logout, answering with the command's own sentences.
pub(super) async fn run_power_command(id: &str) -> Response {
    use compass_power::{Action, PowerManager};
    use std::os::unix::fs::MetadataExt;
    let Some(command) = compass_core::power_commands::command(id) else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "no power command has that id",
        ));
    };
    let cannot = || {
        Response::Error(ProtocolError::new(
            ErrorKind::Unsupported,
            command.cannot_message,
        ))
    };
    let failed = |err: &dyn std::fmt::Display| {
        tracing::warn!(%err, command = id, "power command failed");
        Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            command.failed_message,
        ))
    };
    // Read when run, as the C++ reads `preferenceValues()` in `execute`.
    let config = Config::load().unwrap_or_default();
    let preferences = config.entrypoint_preferences(compass_core::power_commands::EXTENSION_ID, id);
    if let Some(program) = compass_core::power_commands::custom_program(preferences) {
        return run_custom_power_program(program.to_owned()).await;
    }
    let system = match zbus::Connection::system().await {
        Ok(connection) => connection,
        Err(err) => return failed(&err),
    };
    let manager = PowerManager::new(system);
    // The owner of /proc/self is this process's uid, without `unsafe`.
    let uid = std::fs::metadata("/proc/self").map_or(u32::MAX, |m| m.uid());
    let checked = match id {
        "power-off" => Some(Action::PowerOff),
        "reboot" | "soft-reboot" => Some(Action::Reboot),
        "suspend" | "sleep" => Some(Action::Suspend),
        "hibernate" => Some(Action::Hibernate),
        _ => None,
    };
    if let Some(action) = checked {
        match manager.can(action).await {
            Ok(capability) if capability.is_offerable() => {}
            Ok(_) => return cannot(),
            Err(err) => return failed(&err),
        }
    }
    let result = match id {
        "sleep" => manager.sleep().await,
        "soft-reboot" => manager.soft_reboot().await,
        "lock" => manager.lock(uid).await,
        "logout" => {
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            match zbus::Connection::session().await {
                Ok(session) => {
                    manager
                        .logout(compass_power::logout_target(&desktop), &session, uid)
                        .await
                }
                Err(err) => return failed(&err),
            }
        }
        _ => match checked {
            Some(action) => manager.perform(action, true).await,
            None => return cannot(),
        },
    };
    match result {
        Ok(()) => Response::Ack,
        Err(err) => failed(&err),
    }
}

/// Runs a power command's `customProgram` in its place, as
/// `AppService::shellProcess` does: `$SHELL -c program` (else `/bin/sh`), on
/// the host, waited for. Like `QProcess::waitForFinished`, only a program
/// that could not start or was killed by a signal is a failure; its exit code
/// is its own business.
async fn run_custom_power_program(program: String) -> Response {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    let run = {
        let program = program.clone();
        tokio::task::spawn_blocking(move || {
            compass_platform_linux::host_command(&shell)
                .args(["-c", &program])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        })
    };
    match run.await {
        Ok(Ok(status)) if status.code().is_some() => Response::Ack,
        outcome => {
            tracing::warn!(?outcome, program, "custom power program failed");
            Response::Error(ProtocolError::new(
                ErrorKind::Internal,
                compass_core::power_commands::custom_program_failure(&program),
            ))
        }
    }
}
