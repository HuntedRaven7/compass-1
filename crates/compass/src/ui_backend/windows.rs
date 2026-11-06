//! The [`WindowBackend`] impl: the window switcher's list of open windows
//! and what it does to them.
//!
//! Its own trait, apart from the launcher's requests, with the conversion of
//! a window from the wire.

use compass_ipc::Request;
use compass_ui::backend::{BackendFuture, WindowBackend, WindowRow};

use super::DaemonBackend;

impl WindowBackend for DaemonBackend {
    fn list_windows(&self) -> BackendFuture<'_, Vec<WindowRow>> {
        Box::pin(async move {
            match self.ask(Request::ListWindows, "Listing windows").await? {
                compass_ipc::Response::Windows { windows } => {
                    Ok(windows.into_iter().map(window_row).collect())
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn activate_window(&self, id: u32) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::ActivateWindow { id }, "Switching windows")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn close_window(&self, id: u32) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::CloseWindow { id }, "Closing a window")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn app_runtime(&self, id: String) -> BackendFuture<'_, compass_ui::backend::AppRuntimeInfo> {
        Box::pin(async move {
            match self
                .ask(Request::AppRuntime { id }, "Asking whether it runs")
                .await?
            {
                compass_ipc::Response::AppRuntime {
                    running,
                    frontmost,
                    windows,
                } => Ok(compass_ui::backend::AppRuntimeInfo {
                    running,
                    frontmost,
                    windows: windows.into_iter().map(window_row).collect(),
                }),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn quit_app(&self, id: String, force: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self.ask(Request::QuitApp { id, force }, "Quitting").await? {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn quit_window_app(&self, window: u32, force: bool) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::QuitWindowApp { window, force }, "Quitting")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn window_manager_capabilities(
        &self,
    ) -> BackendFuture<'_, compass_core::window_switcher::Capabilities> {
        Box::pin(async move {
            match self
                .ask(
                    Request::WindowManagerCapabilities,
                    "Asking what the window manager can do",
                )
                .await?
            {
                compass_ipc::Response::WindowManagerCapabilities(caps) => {
                    Ok(compass_core::window_switcher::Capabilities {
                        workspaces: caps.workspaces,
                        fullscreen: caps.fullscreen,
                        toggle_floating: caps.floating,
                        toggle_overview: caps.overview,
                        ..compass_core::window_switcher::Capabilities::default()
                    })
                }
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn list_workspaces(&self) -> BackendFuture<'_, Vec<compass_ui::backend::WorkspaceRow>> {
        Box::pin(async move {
            match self
                .ask(Request::ListWorkspaces, "Listing workspaces")
                .await?
            {
                compass_ipc::Response::Workspaces { workspaces } => Ok(workspaces
                    .into_iter()
                    .map(|workspace| compass_ui::backend::WorkspaceRow {
                        id: workspace.id,
                        name: workspace.name,
                        monitor: workspace.monitor,
                        window_count: workspace.window_count as usize,
                        apps: workspace
                            .apps
                            .into_iter()
                            .map(|app| (app.name, app.icon))
                            .collect(),
                        active: workspace.active,
                    })
                    .collect()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn focus_workspace(&self, id: String) -> BackendFuture<'_, ()> {
        Box::pin(async move {
            match self
                .ask(Request::FocusWorkspace { id }, "Switching workspaces")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }

    fn toggle_window_state(
        &self,
        toggle: compass_ui::backend::WindowToggle,
    ) -> BackendFuture<'_, ()> {
        use compass_ui::backend::WindowToggle;
        Box::pin(async move {
            let toggle = match toggle {
                WindowToggle::Fullscreen => compass_ipc::WindowToggle::Fullscreen,
                WindowToggle::Floating => compass_ipc::WindowToggle::Floating,
                WindowToggle::Overview => compass_ipc::WindowToggle::Overview,
            };
            match self
                .ask(Request::ToggleWindowState { toggle }, "Toggling")
                .await?
            {
                compass_ipc::Response::Ack => Ok(()),
                other => Err(format!("Unexpected answer from the engine: {other:?}")),
            }
        })
    }
}

fn window_row(window: compass_ipc::WindowInfo) -> WindowRow {
    WindowRow {
        id: window.id,
        app_known: window.app_name.is_some(),
        app: window.app_name.unwrap_or_else(|| window.wm_class.clone()),
        title: window.title,
        wm_class: window.wm_class,
        pid: window.pid,
        can_close: window.can_close,
    }
}
