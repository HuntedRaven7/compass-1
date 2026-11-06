//! The window-management extension's other commands: Switch Workspaces and
//! the fullscreen, floating and overview toggles (`src/builtins/wm/`), over
//! the compositor's own IPC (Hyprland, niri, KWin), and on GNOME Switch
//! Workspaces through the Shell extension's contract 4.
//!
//! Workspaces are described as `SwitchWorkspacesViewHost::refreshWindows`
//! builds them: each with its window count and the applications with a
//! window on it, once each. A toggle acts on the window the person was in
//! before the launcher took focus, refused when it is not on the active
//! workspace, as `ToggleFullscreenWindowCommand` refuses it.

use compass_core::AppIndex;
use compass_core::app_service::AppService;
use compass_ipc::{
    ErrorKind, ProtocolError, Response, WindowManagerCapabilities, WindowToggle, WorkspaceApp,
    WorkspaceEntry,
};
use compass_platform_linux::compositor::{OwnWindows, Provider, WmWindow, WmWorkspace};

/// What `provider` can do; nothing without one.
#[must_use]
pub fn capabilities(provider: Option<&Provider>) -> WindowManagerCapabilities {
    provider.map_or_else(WindowManagerCapabilities::default, |provider| {
        let caps = provider.capabilities();
        WindowManagerCapabilities {
            workspaces: caps.workspaces,
            fullscreen: caps.fullscreen,
            floating: caps.floating,
            overview: caps.overview,
        }
    })
}

/// The rows of Switch Workspaces for `workspaces` and `windows`, each
/// window's application recognised in `index`.
#[must_use]
pub fn entries(
    workspaces: Vec<WmWorkspace>,
    windows: &[WmWindow],
    active: Option<&str>,
    index: &AppIndex,
) -> Vec<WorkspaceEntry> {
    let apps = AppService::new(index);
    workspaces
        .into_iter()
        .map(|workspace| {
            let on_it: Vec<&WmWindow> = windows
                .iter()
                .filter(|window| window.workspace.as_deref() == Some(workspace.id.as_str()))
                .collect();
            let mut found: Vec<WorkspaceApp> = Vec::new();
            for window in &on_it {
                let Some(app) = (!window.wm_class.is_empty())
                    .then(|| apps.find_by_class(&window.wm_class))
                    .flatten()
                else {
                    continue;
                };
                let name = app.display_name();
                if !found.iter().any(|known| known.name == name) {
                    found.push(WorkspaceApp {
                        name,
                        icon: app.icon().map(str::to_owned),
                    });
                }
            }
            let name = if workspace.name.is_empty() {
                workspace
                    .number
                    .map_or_else(|| workspace.id.clone(), |n| n.to_string())
            } else {
                workspace.name
            };
            WorkspaceEntry {
                active: active == Some(workspace.id.as_str()),
                id: workspace.id,
                name,
                monitor: workspace.monitor,
                window_count: u32::try_from(on_it.len()).unwrap_or(u32::MAX),
                apps: found,
            }
        })
        .collect()
}

/// `WindowManager::isOnActiveWorkspace`: a window on no workspace, or with
/// no active workspace to compare, counts as on it.
#[must_use]
pub fn on_active_workspace(window: Option<&str>, active: Option<&str>) -> bool {
    match (window, active) {
        (Some(on), Some(active)) if !on.is_empty() => on == active,
        _ => true,
    }
}

fn unsupported() -> ProtocolError {
    ProtocolError::new(
        ErrorKind::Unsupported,
        "Window management needs Hyprland, niri or KDE Plasma, and this session is none of them",
    )
}

fn failed(what: &str, err: &compass_platform_linux::compositor::IpcError) -> ProtocolError {
    ProtocolError::new(ErrorKind::Internal, format!("{what} failed: {err}"))
}

/// What Switch Workspaces is built from: the workspaces, the windows and the
/// active workspace's id.
type Listing = (Vec<WmWorkspace>, Vec<WmWindow>, Option<String>);

/// Asks `provider` for what [`entries`] needs.
fn fetch(provider: Option<&Provider>) -> Result<Listing, ProtocolError> {
    let provider = provider.ok_or_else(unsupported)?;
    let workspaces = provider
        .workspaces()
        .map_err(|err| failed("Listing workspaces", &err))?;
    let windows = provider.windows().unwrap_or_default();
    let active = provider.active_workspace().ok().flatten().map(|w| w.id);
    Ok((workspaces, windows, active))
}

/// `ListWorkspaces` against `provider`, in one step (the engine fetches off
/// the executor and reads the index after, in [`handle`]).
#[cfg(test)]
pub fn list(provider: Option<&Provider>, index: &AppIndex) -> Response {
    match fetch(provider) {
        Ok((workspaces, windows, active)) => Response::Workspaces {
            workspaces: entries(workspaces, &windows, active.as_deref(), index),
        },
        Err(err) => Response::Error(err),
    }
}

/// Answers the window-management requests with this session's compositor,
/// asking it off the executor: each request is a blocking socket exchange.
pub(super) async fn handle(
    state: &std::sync::Arc<tokio::sync::RwLock<super::EngineState>>,
    request: compass_ipc::Request,
) -> Response {
    use compass_ipc::Request;
    let provider = crate::wlroots::compositor();
    if provider.is_none()
        && !matches!(request, Request::ToggleWindowState { .. })
        && let Some(shell) = shell(state).await
    {
        return gnome(state, &shell, request).await;
    }
    let blocking = |work: Box<dyn FnOnce() -> Response + Send>| async move {
        tokio::task::spawn_blocking(work)
            .await
            .unwrap_or_else(|err| {
                Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the window manager task failed: {err}"),
                ))
            })
    };
    match request {
        Request::WindowManagerCapabilities => {
            Response::WindowManagerCapabilities(capabilities(provider))
        }
        Request::ListWorkspaces => {
            let fetched = tokio::task::spawn_blocking(move || fetch(provider)).await;
            match fetched {
                Ok(Ok((workspaces, windows, active))) => Response::Workspaces {
                    workspaces: entries(
                        workspaces,
                        &windows,
                        active.as_deref(),
                        &state.read().await.search.index,
                    ),
                },
                Ok(Err(err)) => Response::Error(err),
                Err(err) => Response::Error(ProtocolError::new(
                    ErrorKind::Internal,
                    format!("the window manager task failed: {err}"),
                )),
            }
        }
        Request::FocusWorkspace { id } => blocking(Box::new(move || focus(provider, &id))).await,
        Request::ToggleWindowState { toggle: which } => {
            blocking(Box::new(move || toggle(provider, which))).await
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a window-management request",
        )),
    }
}

/// The Shell extension, when there is no compositor IPC and it answers: GNOME,
/// where the workspaces are Mutter's and only the extension reaches them.
async fn shell(
    state: &std::sync::Arc<tokio::sync::RwLock<super::EngineState>>,
) -> Option<std::sync::Arc<compass_shell::ShellClient>> {
    let shell = state.read().await.shell_client()?;
    let answers = shell.capabilities().windows.is_available()
        || shell.refresh_capabilities().await.windows.is_available();
    answers.then_some(shell)
}

/// A GNOME workspace as the listing's: its index is its id, and a person
/// counts from one (`Gnome::Workspace`).
#[must_use]
pub fn gnome_workspace(workspace: &compass_shell::Workspace) -> WmWorkspace {
    WmWorkspace {
        id: workspace.index.to_string(),
        name: workspace.name.clone(),
        number: Some(workspace.index.saturating_add(1)),
        monitor: None,
        has_fullscreen: workspace.has_fullscreen,
    }
}

/// A Shell window as the listing's, on the workspace it names.
#[must_use]
pub fn gnome_window(window: &compass_shell::Window) -> WmWindow {
    WmWindow {
        id: window.id.to_string(),
        title: window.title.clone(),
        wm_class: window.wm_class.clone(),
        pid: window.pid,
        workspace: window.workspace.map(|index| index.to_string()),
        bounds: None,
        focused: window.focused,
        fullscreen: window.fullscreen,
    }
}

/// The window-management requests on GNOME, through the Shell extension's
/// contract 4 (`GnomeWindowManager::listWorkspaces`). The C++ lists GNOME's
/// workspaces but cannot switch to one; the extension's `ActivateWorkspace`
/// does.
async fn gnome(
    state: &std::sync::Arc<tokio::sync::RwLock<super::EngineState>>,
    shell: &compass_shell::ShellClient,
    request: compass_ipc::Request,
) -> Response {
    use compass_ipc::Request;
    const WHAT: &str = "Switch Workspaces";
    match request {
        Request::WindowManagerCapabilities => {
            Response::WindowManagerCapabilities(WindowManagerCapabilities {
                workspaces: shell
                    .capabilities()
                    .windows
                    .supports(compass_shell::WORKSPACES_SINCE),
                ..WindowManagerCapabilities::default()
            })
        }
        Request::ListWorkspaces => {
            let workspaces = match shell.list_workspaces().await {
                Ok(workspaces) => workspaces,
                Err(err) => {
                    return Response::Error(crate::window_service::refusal(&err, WHAT));
                }
            };
            let windows = shell.list_windows().await.unwrap_or_default();
            let active = workspaces
                .iter()
                .find(|workspace| workspace.active)
                .map(|workspace| workspace.index.to_string());
            let windows: Vec<WmWindow> = windows.iter().map(gnome_window).collect();
            Response::Workspaces {
                workspaces: entries(
                    workspaces.iter().map(gnome_workspace).collect(),
                    &windows,
                    active.as_deref(),
                    &state.read().await.search.index,
                ),
            }
        }
        Request::FocusWorkspace { id } => {
            let Ok(index) = id.parse::<i32>() else {
                return Response::Error(ProtocolError::new(
                    ErrorKind::BadRequest,
                    format!("{id:?} is not a GNOME workspace"),
                ));
            };
            match shell.activate_workspace(index).await {
                Ok(()) => Response::Ack,
                Err(err) => Response::Error(crate::window_service::refusal(&err, WHAT)),
            }
        }
        _ => Response::Error(unsupported()),
    }
}

/// `FocusWorkspace` against `provider`.
pub fn focus(provider: Option<&Provider>, id: &str) -> Response {
    let Some(provider) = provider else {
        return Response::Error(unsupported());
    };
    match provider.focus_workspace(id) {
        Ok(()) => Response::Ack,
        Err(err) => Response::Error(failed("Switching workspaces", &err)),
    }
}

/// `ToggleWindowState` against `provider`: the window the person was in,
/// leaving out the launcher's own.
pub fn toggle(provider: Option<&Provider>, toggle: WindowToggle) -> Response {
    let Some(provider) = provider else {
        return Response::Error(unsupported());
    };
    let refused = |message: &str| {
        Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            message.to_owned(),
        ))
    };
    if toggle == WindowToggle::Overview {
        return match provider.toggle_overview() {
            Ok(()) => Response::Ack,
            Err(err) => Response::Error(failed("Toggling the overview", &err)),
        };
    }
    let own = OwnWindows {
        pids: vec![std::process::id()],
        classes: vec![compass_ui::APP_ID.to_owned()],
    };
    let window = match provider.frontmost_window(&own) {
        Ok(Some(window)) => window,
        Ok(None) => {
            return refused(match toggle {
                WindowToggle::Fullscreen => "No window to fullscreen",
                _ => "No window to toggle",
            });
        }
        Err(err) => return Response::Error(failed("Finding the active window", &err)),
    };
    let active = provider.active_workspace().ok().flatten().map(|w| w.id);
    if !on_active_workspace(window.workspace.as_deref(), active.as_deref()) {
        return refused("Active window is not on the current workspace");
    }
    let done = match toggle {
        WindowToggle::Fullscreen => provider.toggle_fullscreen(&window.id),
        _ => provider.toggle_floating(&window.id),
    };
    match done {
        Ok(()) => Response::Ack,
        Err(err) => Response::Error(failed("Toggling the window", &err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_platform_linux::compositor::hyprland::Hyprland;
    use compass_testkit::fake_compositor::{FakeSocket, Framing};

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../compass-platform-linux/tests/fixtures/hyprland")
                .join(name),
        )
        .unwrap()
    }

    fn hyprland(dir: &std::path::Path, active: &str) -> (FakeSocket, Provider) {
        let path = FakeSocket::hyprland_path(dir, "sig");
        let fake = FakeSocket::replaying(
            &path,
            Framing::Hyprland,
            vec![
                ("-j/clients".into(), fixture("clients.json")),
                ("-j/workspaces".into(), fixture("workspaces.json")),
                ("-j/activeworkspace".into(), active.to_owned()),
                ("-j/activewindow".into(), fixture("activewindow.json")),
                ("dispatch".into(), "ok".into()),
            ],
        );
        (fake, Provider::Hyprland(Hyprland::new(path)))
    }

    fn index(dir: &std::path::Path) -> AppIndex {
        std::fs::write(
            dir.join("foot.desktop"),
            "[Desktop Entry]\nType=Application\nName=Foot\nExec=foot\nIcon=foot\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("firefox.desktop"),
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox\nIcon=firefox\n",
        )
        .unwrap();
        AppIndex::builder().dir(dir).build()
    }

    #[test]
    fn workspaces_count_their_windows_and_name_their_applications_once() {
        let dir = tempfile::tempdir().unwrap();
        let (_fake, provider) = hyprland(dir.path(), &fixture("activeworkspace.json"));
        let apps = tempfile::tempdir().unwrap();
        let Response::Workspaces { workspaces } = list(Some(&provider), &index(apps.path())) else {
            panic!("no workspaces");
        };
        assert_eq!(
            workspaces
                .iter()
                .map(|w| (
                    w.id.as_str(),
                    w.name.as_str(),
                    w.monitor.as_deref(),
                    w.window_count,
                    w.active
                ))
                .collect::<Vec<_>>(),
            [
                ("1", "1", Some("DP-1"), 2, true),
                ("3", "music", Some("HDMI-A-1"), 1, false)
            ]
        );
        assert_eq!(
            workspaces[0]
                .apps
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            ["Firefox", "Foot"]
        );
        assert!(workspaces[1].apps.is_empty(), "Spotify is not installed");
    }

    #[test]
    fn an_unnamed_workspace_is_called_by_its_number() {
        let dir = tempfile::tempdir().unwrap();
        let rows = entries(
            vec![WmWorkspace {
                id: "12".into(),
                number: Some(2),
                ..WmWorkspace::default()
            }],
            &[],
            None,
            &index(dir.path()),
        );
        assert_eq!(rows[0].name, "2");
        assert_eq!(rows[0].window_count, 0);
    }

    #[test]
    fn a_toggle_acts_on_the_active_window_and_refuses_one_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, provider) = hyprland(dir.path(), &fixture("activeworkspace.json"));
        assert_eq!(
            toggle(Some(&provider), WindowToggle::Fullscreen),
            Response::Ack
        );
        assert!(
            fake.seen()
                .last()
                .is_some_and(|l| l.contains("fullscreen") && l.contains("0x5581c8a4f310")),
            "{:?}",
            fake.seen()
        );
        assert_eq!(focus(Some(&provider), "3"), Response::Ack);
        assert!(fake.seen().last().is_some_and(|l| l.contains("workspace")));

        // With workspace 3 active, the window there is the one toggled.
        let dir = tempfile::tempdir().unwrap();
        let (fake, provider) = hyprland(dir.path(), r#"{"id":3,"name":"music"}"#);
        assert_eq!(
            toggle(Some(&provider), WindowToggle::Floating),
            Response::Ack
        );
        assert!(
            fake.seen()
                .last()
                .is_some_and(|l| l.contains("float") && l.contains("0x5581c8b07a90")),
            "{:?}",
            fake.seen()
        );
    }

    #[test]
    fn a_window_on_another_workspace_is_not_on_the_active_one() {
        assert!(on_active_workspace(Some("1"), Some("1")));
        assert!(!on_active_workspace(Some("2"), Some("1")));
        assert!(on_active_workspace(None, Some("1")), "on no workspace");
        assert!(on_active_workspace(Some(""), Some("1")));
        assert!(on_active_workspace(Some("2"), None), "no active workspace");
    }

    #[test]
    fn without_a_compositor_everything_is_refused_and_nothing_is_offered() {
        assert_eq!(capabilities(None), WindowManagerCapabilities::default());
        let dir = tempfile::tempdir().unwrap();
        for response in [
            list(None, &index(dir.path())),
            focus(None, "1"),
            toggle(None, WindowToggle::Overview),
        ] {
            assert!(
                matches!(&response, Response::Error(e) if e.kind == ErrorKind::Unsupported),
                "{response:?}"
            );
        }
    }
}
