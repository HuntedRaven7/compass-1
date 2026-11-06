//! Quit and Force Quit, and whether an application is running: the C++
//! `LinuxAppRuntime` over this engine's window providers.
//!
//! An application is running when it has a window (`isRunning` is
//! `!findAppWindows(app).empty()`), and frontmost when one of them has focus.
//! Quit closes every window it has; Force Quit sends `SIGKILL` to each
//! process that owns one of them, once per process, and closes the windows
//! that name no process. Either is a failure only when it did nothing.
//! Windows come from wherever `ListWindows` gets them: the GNOME Shell
//! extension (which reports pids), a wlroots compositor's toplevels
//! (enriched with pids from Hyprland's or niri's IPC), and nowhere else.

use std::sync::Arc;

use compass_core::app_windows::AppIdentity;
use compass_ipc::{ErrorKind, ProtocolError, Response, WindowInfo};
use tokio::sync::RwLock;

use super::EngineState;

/// What Force Quit does with a set of windows: the processes to kill, each
/// once, and the windows that name no process, to close instead.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForceQuitPlan {
    /// Owning processes, deduplicated, in window order; never 0.
    pub kill: Vec<u32>,
    /// Windows without a process.
    pub close: Vec<u32>,
}

/// The C++ `forceQuit`'s split of `windows`.
#[must_use]
pub fn force_quit_plan(windows: &[WindowInfo]) -> ForceQuitPlan {
    let mut plan = ForceQuitPlan::default();
    for window in windows {
        match window.pid.filter(|pid| *pid > 0) {
            Some(pid) => {
                if !plan.kill.contains(&pid) {
                    plan.kill.push(pid);
                }
            }
            None => plan.close.push(window.id),
        }
    }
    plan
}

/// Sends `SIGKILL` to `pid`; whether it was delivered.
fn kill(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    )
    .is_ok()
}

/// The application `id` names and what identifies its windows.
async fn application(state: &Arc<RwLock<EngineState>>, id: &str) -> Option<(AppIdentity, String)> {
    let state = state.read().await;
    let item = state
        .search
        .index
        .get(id)
        .or_else(|| {
            state
                .search
                .index
                .position_by_entrypoint(id)
                .map(|position| &state.search.index.items()[position])
        })
        .filter(|item| !item.is_action())?;
    Some((
        compass_core::app_service::identity(item),
        item.display_name(),
    ))
}

/// `AppRuntime`: running, frontmost, and the windows.
pub async fn describe(state: &Arc<RwLock<EngineState>>, id: &str) -> Response {
    let Some((identity, _)) = application(state, id).await else {
        return Response::Error(ProtocolError::new(ErrorKind::BadRequest, "No app with id"));
    };
    let windows = super::launch::app_windows(state, &identity).await;
    Response::AppRuntime {
        running: !windows.is_empty(),
        frontmost: windows.iter().any(|window| window.focused),
        windows,
    }
}

/// `QuitApp`.
pub async fn quit(state: &Arc<RwLock<EngineState>>, id: &str, force: bool) -> Response {
    let Some((identity, name)) = application(state, id).await else {
        return Response::Error(ProtocolError::new(ErrorKind::BadRequest, "No app with id"));
    };
    let windows = super::launch::app_windows(state, &identity).await;
    quit_windows(state, &windows, &name, force).await
}

/// `QuitWindowApp`: the application whose class the window has, as the
/// window switcher recognised it.
pub async fn quit_window_app(
    state: &Arc<RwLock<EngineState>>,
    window: u32,
    force: bool,
) -> Response {
    let windows = match super::shell::list_windows(state).await {
        Response::Windows { windows } => windows,
        other => return other,
    };
    let Some(target) = windows.iter().find(|candidate| candidate.id == window) else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "No window with that id",
        ));
    };
    let found = {
        let state = state.read().await;
        compass_core::app_service::AppService::new(&state.search.index)
            .find_by_class(&target.wm_class)
            .map(|item| {
                (
                    compass_core::app_service::identity(item),
                    item.display_name(),
                )
            })
    };
    let Some((identity, name)) = found else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            format!("{} belongs to no known application", target.title),
        ));
    };
    let owned: Vec<WindowInfo> = windows
        .into_iter()
        .filter(|candidate| identity.matches_window(&candidate.wm_class, &candidate.title))
        .collect();
    quit_windows(state, &owned, &name, force).await
}

async fn quit_windows(
    state: &Arc<RwLock<EngineState>>,
    windows: &[WindowInfo],
    name: &str,
    force: bool,
) -> Response {
    let (kill_pids, close) = if force {
        let plan = force_quit_plan(windows);
        (plan.kill, plan.close)
    } else {
        (Vec::new(), windows.iter().map(|window| window.id).collect())
    };
    let mut acted = false;
    for pid in kill_pids {
        acted |= kill(pid);
    }
    for id in close {
        acted |= matches!(
            super::shell::act_on_window(state, id, true).await,
            Response::Ack
        );
    }
    if acted {
        return Response::Ack;
    }
    Response::Error(ProtocolError::new(
        ErrorKind::Unsupported,
        if force {
            format!("Failed to force quit {name}")
        } else {
            format!("Failed to quit {name}")
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u32, pid: Option<u32>) -> WindowInfo {
        WindowInfo {
            id,
            title: format!("window {id}"),
            wm_class: "app".into(),
            app_name: None,
            app_icon: None,
            pid,
            workspace: None,
            focused: false,
            can_close: true,
        }
    }

    #[test]
    fn force_quit_kills_each_process_once_and_closes_the_windows_that_name_none() {
        let plan = force_quit_plan(&[
            window(1, Some(40)),
            window(2, Some(40)),
            window(3, None),
            window(4, Some(0)),
            window(5, Some(41)),
        ]);
        assert_eq!(
            plan,
            ForceQuitPlan {
                kill: vec![40, 41],
                close: vec![3, 4],
            }
        );
        assert_eq!(force_quit_plan(&[]), ForceQuitPlan::default());
    }

    #[test]
    fn killing_a_process_this_test_started_ends_it_with_sigkill() {
        use std::os::unix::process::ExitStatusExt;
        let mut child = std::process::Command::new("sleep")
            .arg("600")
            .spawn()
            .expect("spawn sleep");
        assert!(kill(child.id()));
        let status = child.wait().expect("reaped");
        assert_eq!(status.signal(), Some(9), "{status:?}");
        assert!(!kill(u32::MAX), "a pid that does not fit is not signalled");
    }
}
