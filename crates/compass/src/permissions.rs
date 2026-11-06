//! The launcher hotkey's and login start's states on the wire, and back.
//!
//! `compass-ipc` does not depend on `compass-core`, so the protocol carries
//! its own copies of `compass_core::launcher_shortcut::LauncherShortcut` and
//! `compass_core::login_start::Status`; the engine and the window's backend
//! translate here, in one place, so the two never drift apart.

use compass_core::launcher_shortcut::LauncherShortcut;
use compass_core::login_start::{Mechanism, State, Status, Unit};
use compass_ipc::{LauncherShortcutState, LoginStartState, LoginStartUnit, Response};

/// The engine's answer to a [`compass_ipc::Request::LauncherShortcut`].
#[must_use]
pub fn launcher_response(state: LauncherShortcut) -> Response {
    let state = match state {
        LauncherShortcut::NotRequested => LauncherShortcutState::NotRequested,
        LauncherShortcut::Pending => LauncherShortcutState::Pending,
        LauncherShortcut::Granted { trigger } => LauncherShortcutState::Granted { trigger },
        LauncherShortcut::Conflict { reason } => LauncherShortcutState::Conflict { reason },
        LauncherShortcut::Refused => LauncherShortcutState::Refused,
        LauncherShortcut::Unavailable => LauncherShortcutState::Unavailable,
        LauncherShortcut::Failed { reason } => LauncherShortcutState::Failed { reason },
        LauncherShortcut::Unset => LauncherShortcutState::Unset,
        LauncherShortcut::Off => LauncherShortcutState::Off,
    };
    Response::LauncherShortcut { state }
}

/// The window's reading of a [`Response::LauncherShortcut`].
#[must_use]
pub fn launcher_from_wire(state: LauncherShortcutState) -> LauncherShortcut {
    match state {
        LauncherShortcutState::NotRequested => LauncherShortcut::NotRequested,
        LauncherShortcutState::Pending => LauncherShortcut::Pending,
        LauncherShortcutState::Granted { trigger } => LauncherShortcut::Granted { trigger },
        LauncherShortcutState::Conflict { reason } => LauncherShortcut::Conflict { reason },
        LauncherShortcutState::Refused => LauncherShortcut::Refused,
        LauncherShortcutState::Unavailable => LauncherShortcut::Unavailable,
        LauncherShortcutState::Failed { reason } => LauncherShortcut::Failed { reason },
        LauncherShortcutState::Unset => LauncherShortcut::Unset,
        LauncherShortcutState::Off => LauncherShortcut::Off,
    }
}

/// The engine's answer to a [`compass_ipc::Request::LoginStart`].
#[must_use]
pub fn login_start_response(status: Status) -> Response {
    let state = match status.state {
        State::Off => LoginStartState::Off,
        State::On {
            mechanism: Mechanism::Portal,
        } => LoginStartState::Portal,
        State::On {
            mechanism: Mechanism::AutostartFile,
        } => LoginStartState::AutostartFile,
        State::Refused => LoginStartState::Refused,
        State::Unavailable => LoginStartState::Unavailable,
    };
    let unit = status.unit.map(|unit| match unit {
        Unit::Installed => LoginStartUnit::Installed,
        Unit::Enabled => LoginStartUnit::Enabled,
    });
    Response::LoginStart { state, unit }
}

/// The window's reading of a [`Response::LoginStart`].
#[must_use]
pub fn login_start_from_wire(state: LoginStartState, unit: Option<LoginStartUnit>) -> Status {
    let state = match state {
        LoginStartState::Off => State::Off,
        LoginStartState::Portal => State::On {
            mechanism: Mechanism::Portal,
        },
        LoginStartState::AutostartFile => State::On {
            mechanism: Mechanism::AutostartFile,
        },
        LoginStartState::Refused => State::Refused,
        LoginStartState::Unavailable => State::Unavailable,
    };
    let unit = unit.map(|unit| match unit {
        LoginStartUnit::Installed => Unit::Installed,
        LoginStartUnit::Enabled => Unit::Enabled,
    });
    Status { state, unit }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_survives_the_wire() {
        for state in [
            LauncherShortcut::NotRequested,
            LauncherShortcut::Pending,
            LauncherShortcut::Granted {
                trigger: Some("Super+Space".to_owned()),
            },
            LauncherShortcut::Granted { trigger: None },
            LauncherShortcut::Conflict {
                reason: "Taken.".to_owned(),
            },
            LauncherShortcut::Refused,
            LauncherShortcut::Unavailable,
            LauncherShortcut::Failed {
                reason: "No reply.".to_owned(),
            },
            LauncherShortcut::Unset,
            LauncherShortcut::Off,
        ] {
            let Response::LauncherShortcut { state: wire } = launcher_response(state.clone())
            else {
                unreachable!()
            };
            assert_eq!(launcher_from_wire(wire), state);
        }
        for state in [
            State::Off,
            State::On {
                mechanism: Mechanism::Portal,
            },
            State::On {
                mechanism: Mechanism::AutostartFile,
            },
            State::Refused,
            State::Unavailable,
        ] {
            for unit in [None, Some(Unit::Installed), Some(Unit::Enabled)] {
                let status = Status {
                    state: state.clone(),
                    unit,
                };
                let Response::LoginStart { state, unit } = login_start_response(status.clone())
                else {
                    unreachable!()
                };
                assert_eq!(login_start_from_wire(state, unit), status);
            }
        }
    }
}
