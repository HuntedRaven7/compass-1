//! The window manager an extension reaches through `WindowManagement/*`.
//!
//! The backends the window switcher uses, chosen in the C++ order: a
//! compositor with its own IPC first (Hyprland's socket, niri's — the C++
//! Hyprland and niri providers, `compass_platform_linux::compositor`), then
//! the foreign-toplevel protocols on any other wlroots compositor, then the
//! GNOME Shell extension's window list. Monitors come from `wl_output`, which
//! every compositor carries. Where no backend is there (a headless run, a
//! TTY, a compositor with no toplevel protocol) there are no windows, and
//! `getActiveWindow` fails with the C++'s own "No active window".
//!
//! # Which window is "active"
//!
//! An extension asks from inside the launcher, so the focused window is
//! usually the launcher itself. The C++ answers with the window focused
//! *before* it (`WindowManager::getFocusedWindow` remembers the last foreign
//! one); this takes the first window in the backend's most-recently-used
//! order that is not the launcher, which is the same window without a
//! focus-tracking loop to keep it. On Hyprland it is the C++'s
//! `getFrontmostWindowSync`: the lowest focus history on the active
//! workspace.
//!
//! # What only the compositor IPC reports
//!
//! Workspaces (and a window's workspace, pid and — on Hyprland — geometry).
//! The Shell contract gives a window's workspace index but no workspace
//! list, and the toplevel protocols give nothing: there `getActiveWorkspace`
//! fails with "No active workspace" and `getWorkspaces` is empty. No backend
//! moves or resizes a window, so `setWindowBounds` is refused, as the C++
//! Hyprland and niri providers refuse it. PARITY, "The extension host API".

use std::sync::Arc;

use compass_core::app_windows::AppIdentity;
use compass_platform_linux::compositor::{OwnWindows, Provider, WmWindow, WmWorkspace};
use compass_worker_host::application_service::Application;
use compass_worker_host::window_service::{Rect, Screen, Size, Window, Windows, Workspace};

/// Where the windows come from.
enum Backend {
    /// The GNOME Shell extension, reached on a runtime.
    Shell {
        client: Arc<compass_shell::ShellClient>,
        handle: tokio::runtime::Handle,
    },
    /// A wlroots compositor's toplevel list.
    Toplevels(Arc<compass_wayland::Toplevels>),
    /// A compositor with its own IPC (Hyprland, niri).
    Compositor(Provider),
    /// Nothing to ask.
    None,
}

/// [`Windows`] over this session's window manager.
pub struct EngineWindows {
    backend: Backend,
    /// Every installed application, as `findByClass` matches them.
    classes: Vec<(AppIdentity, Application)>,
    /// Whether to ask the compositor for its outputs: off in a session with
    /// no Wayland display.
    outputs: bool,
}

impl std::fmt::Debug for EngineWindows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match &self.backend {
            Backend::Shell { .. } => "shell",
            Backend::Toplevels(_) => "toplevels",
            Backend::Compositor(provider) => provider.id(),
            Backend::None => "none",
        };
        f.debug_struct("EngineWindows")
            .field("backend", &backend)
            .field("classes", &self.classes.len())
            .finish_non_exhaustive()
    }
}

impl EngineWindows {
    /// This session's window manager: the wlroots toplevel list when the
    /// compositor is one, else the Shell extension when there is a client.
    ///
    /// Blocking (wlroots detection is a registry round trip); called from
    /// the thread that serves the command.
    #[must_use]
    pub fn detect(
        shell: Option<Arc<compass_shell::ShellClient>>,
        handle: Option<tokio::runtime::Handle>,
        classes: Vec<(AppIdentity, Application)>,
    ) -> Self {
        let backend = if let Some(provider) = crate::wlroots::compositor() {
            Backend::Compositor(provider.clone())
        } else if let Some(wlroots) = crate::wlroots::session() {
            wlroots
                .toplevels
                .clone()
                .map_or(Backend::None, Backend::Toplevels)
        } else if let (Some(client), Some(handle)) = (shell, handle) {
            Backend::Shell { client, handle }
        } else {
            Backend::None
        };
        Self {
            backend,
            classes,
            outputs: crate::doctor::has_wayland_in(&crate::doctor::Env::from_process()),
        }
    }

    /// No window manager at all, for a session that has none.
    #[must_use]
    pub const fn none(classes: Vec<(AppIdentity, Application)>) -> Self {
        Self {
            backend: Backend::None,
            classes,
            outputs: false,
        }
    }

    /// The windows, most recently used first, with whether each is focused.
    fn windows(&self) -> Vec<(Window, bool)> {
        match &self.backend {
            Backend::Shell { client, handle } => {
                let client = Arc::clone(client);
                match handle.block_on(async move { client.list_windows().await }) {
                    Ok(windows) => windows.into_iter().map(from_shell).collect(),
                    Err(err) => {
                        tracing::info!(error = %err, "no window list for the extension");
                        Vec::new()
                    }
                }
            }
            Backend::Toplevels(toplevels) => toplevels
                .list()
                .into_iter()
                .map(|toplevel| {
                    let focused = toplevel.activated;
                    (
                        Window {
                            id: toplevel.id.to_string(),
                            title: toplevel.title,
                            wm_class: toplevel.app_id,
                            ..Window::default()
                        },
                        focused,
                    )
                })
                .collect(),
            Backend::Compositor(provider) => match provider.windows() {
                Ok(windows) => windows
                    .into_iter()
                    .map(|window| {
                        let focused = window.focused;
                        (from_compositor(window), focused)
                    })
                    .collect(),
                Err(err) => {
                    tracing::info!(error = %err, "no window list from the compositor");
                    Vec::new()
                }
            },
            Backend::None => Vec::new(),
        }
    }

    /// A compositor, for a caller that already has one.
    #[must_use]
    pub const fn compositor(provider: Provider, classes: Vec<(AppIdentity, Application)>) -> Self {
        Self {
            backend: Backend::Compositor(provider),
            classes,
            outputs: false,
        }
    }
}

fn from_compositor(window: WmWindow) -> Window {
    Window {
        id: window.id,
        title: window.title,
        workspace_id: window.workspace,
        fullscreen: window.fullscreen,
        bounds: window.bounds.map(|bounds| Rect {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
        }),
        wm_class: window.wm_class,
    }
}

fn workspace(workspace: WmWorkspace) -> Workspace {
    Workspace {
        id: workspace.id,
        name: workspace.name,
        fullscreen: workspace.has_fullscreen,
        monitor: workspace.monitor,
    }
}

fn from_shell(window: compass_shell::Window) -> (Window, bool) {
    let focused = window.focused;
    (
        Window {
            id: window.id.0.to_string(),
            title: window.title,
            workspace_id: window.workspace.map(|index| index.to_string()),
            fullscreen: window.fullscreen,
            bounds: window.frame.map(|frame| Rect {
                x: frame.x.into(),
                y: frame.y.into(),
                width: frame.width.into(),
                height: frame.height.into(),
            }),
            wm_class: window.wm_class,
        },
        focused,
    )
}

/// Whether `window` is the launcher's own.
fn own(window: &Window) -> bool {
    window.wm_class.eq_ignore_ascii_case(compass_ui::APP_ID)
}

/// The active window among `windows` (most recently used first): the
/// focused one, or the one focused before the launcher took focus.
fn active(windows: Vec<(Window, bool)>) -> Option<Window> {
    let launcher_focused = windows
        .iter()
        .any(|(window, focused)| *focused && own(window));
    windows
        .into_iter()
        .find(|(window, focused)| !own(window) && (*focused || launcher_focused))
        .map(|(window, _)| window)
}

/// `outputs` as screens, the one holding `focus`'s centre active; with one
/// monitor, that one.
fn screens(outputs: Vec<compass_wayland::output::Output>, focus: Option<Rect>) -> Vec<Screen> {
    let only = outputs.len() == 1;
    outputs
        .into_iter()
        .map(|output| {
            let bounds = Rect {
                x: output.x.into(),
                y: output.y.into(),
                width: output.width.into(),
                height: output.height.into(),
            };
            let active = only
                || focus.is_some_and(|focus| {
                    let (cx, cy) = (focus.x + focus.width / 2, focus.y + focus.height / 2);
                    (bounds.x..bounds.x + bounds.width).contains(&cx)
                        && (bounds.y..bounds.y + bounds.height).contains(&cy)
                });
            Screen {
                name: output.name,
                model: output.model,
                make: output.make,
                serial: None,
                bounds,
                physical_resolution: Size {
                    width: output.pixel_width.into(),
                    height: output.pixel_height.into(),
                },
                active,
            }
        })
        .collect()
}

impl Windows for EngineWindows {
    fn find_window(&self, id: &str) -> Option<Window> {
        self.windows()
            .into_iter()
            .map(|(window, _)| window)
            .find(|window| window.id == id)
    }

    fn focus(&self, window: &Window) {
        if let Backend::Compositor(provider) = &self.backend {
            if let Err(err) = provider.focus_window(&window.id) {
                tracing::info!(error = %err, "could not focus the extension's window");
            }
            return;
        }
        let Ok(id) = window.id.parse::<u32>() else {
            return;
        };
        match &self.backend {
            Backend::Shell { client, handle } => {
                let client = Arc::clone(client);
                let done = handle.block_on(async move {
                    client.activate_window(compass_shell::WindowId(id)).await
                });
                if let Err(err) = done {
                    tracing::info!(error = %err, "could not focus the extension's window");
                }
            }
            Backend::Toplevels(toplevels) => {
                if let Err(err) = toplevels.activate(id) {
                    tracing::info!(error = %err, "could not focus the extension's window");
                }
            }
            Backend::Compositor(_) | Backend::None => {}
        }
    }

    fn focused_window(&self) -> Option<Window> {
        if let Backend::Compositor(provider) = &self.backend {
            let own = OwnWindows {
                pids: Vec::new(),
                classes: vec![compass_ui::APP_ID.to_owned()],
            };
            return match provider.frontmost_window(&own) {
                Ok(window) => window.map(from_compositor),
                Err(err) => {
                    tracing::info!(error = %err, "no active window from the compositor");
                    None
                }
            };
        }
        active(self.windows())
    }

    fn list_windows(&self) -> Vec<Window> {
        self.windows()
            .into_iter()
            .map(|(window, _)| window)
            .filter(|window| !own(window))
            .collect()
    }

    fn active_workspace(&self) -> Option<Workspace> {
        let Backend::Compositor(provider) = &self.backend else {
            return None;
        };
        match provider.active_workspace() {
            Ok(active) => active.map(workspace),
            Err(err) => {
                tracing::info!(error = %err, "no active workspace from the compositor");
                None
            }
        }
    }

    fn list_workspaces(&self) -> Vec<Workspace> {
        let Backend::Compositor(provider) = &self.backend else {
            return Vec::new();
        };
        match provider.workspaces() {
            Ok(workspaces) => workspaces.into_iter().map(workspace).collect(),
            Err(err) => {
                tracing::info!(error = %err, "no workspaces from the compositor");
                Vec::new()
            }
        }
    }

    fn list_screens(&self) -> Vec<Screen> {
        if !self.outputs {
            return Vec::new();
        }
        match compass_wayland::output::list() {
            Ok(outputs) => screens(
                outputs,
                self.focused_window().and_then(|window| window.bounds),
            ),
            Err(err) => {
                tracing::info!(error = %err, "no monitors for the extension");
                Vec::new()
            }
        }
    }

    fn app_for_class(&self, wm_class: &str) -> Option<Application> {
        if wm_class.is_empty() {
            return None;
        }
        self.classes
            .iter()
            .find(|(identity, _)| identity.matches_window_class(wm_class))
            .map(|(_, app)| app.clone())
    }

    fn set_window_bounds(&self, _window: &Window, _bounds: Rect) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, wm_class: &str) -> Window {
        Window {
            id: id.into(),
            title: id.into(),
            wm_class: wm_class.into(),
            ..Window::default()
        }
    }

    #[test]
    fn the_focused_window_is_active_unless_it_is_the_launcher() {
        let editor = window("1", "org.gnome.TextEditor");
        assert_eq!(
            active(vec![
                (editor.clone(), true),
                (window("2", "firefox"), false)
            ]),
            Some(editor)
        );
    }

    #[test]
    fn with_the_launcher_focused_the_window_before_it_is_active() {
        let launcher = window("9", compass_ui::APP_ID);
        let before = window("1", "firefox");
        assert_eq!(
            active(vec![
                (launcher, true),
                (before.clone(), false),
                (window("2", "org.gnome.Nautilus"), false),
            ]),
            Some(before)
        );
    }

    #[test]
    fn nothing_focused_is_no_active_window() {
        assert_eq!(active(vec![(window("1", "firefox"), false)]), None);
        assert_eq!(active(Vec::new()), None);
    }

    fn output(name: &str, x: i32) -> compass_wayland::output::Output {
        compass_wayland::output::Output {
            name: name.into(),
            x,
            width: 1920,
            height: 1080,
            pixel_width: 3840,
            pixel_height: 2160,
            ..compass_wayland::output::Output::default()
        }
    }

    #[test]
    fn the_screen_under_the_focused_window_is_the_active_one() {
        let focus = Rect {
            x: 2000,
            y: 100,
            width: 800,
            height: 600,
        };
        let screens = screens(vec![output("DP-1", 0), output("DP-2", 1920)], Some(focus));
        assert_eq!(
            screens
                .iter()
                .map(|s| (s.name.as_str(), s.active))
                .collect::<Vec<_>>(),
            [("DP-1", false), ("DP-2", true)]
        );
        assert_eq!(screens[1].physical_resolution.width, 3840);
        assert_eq!(screens[1].bounds.width, 1920);
    }

    #[test]
    fn a_lone_screen_is_active_even_with_no_window_to_place() {
        assert!(screens(vec![output("eDP-1", 0)], None)[0].active);
    }

    #[test]
    fn a_window_is_matched_to_its_application_by_class() {
        let windows = EngineWindows::none(vec![(
            AppIdentity {
                desktop_id: "org.gnome.Nautilus.desktop".into(),
                startup_wm_class: None,
                display_name: "Files".into(),
            },
            Application {
                id: "org.gnome.Nautilus.desktop".into(),
                name: "Files".into(),
                ..Application::default()
            },
        )]);
        assert_eq!(
            windows
                .app_for_class("org.gnome.Nautilus")
                .map(|app| app.name),
            Some("Files".to_owned())
        );
        assert_eq!(windows.app_for_class(""), None);
        assert_eq!(windows.list_windows(), Vec::new());
        assert_eq!(windows.focused_window(), None);
    }

    #[test]
    fn on_hyprland_windows_workspaces_and_focus_come_from_its_socket() {
        use compass_testkit::fake_compositor::{FakeSocket, Framing};
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../compass-platform-linux/tests/fixtures/hyprland");
        let read = |name: &str| std::fs::read_to_string(fixtures.join(name)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = FakeSocket::hyprland_path(dir.path(), "sig");
        let fake = FakeSocket::replaying(
            &path,
            Framing::Hyprland,
            vec![
                ("-j/clients".into(), read("clients.json")),
                ("-j/workspaces".into(), read("workspaces.json")),
                ("-j/activeworkspace".into(), read("activeworkspace.json")),
                ("-j/activewindow".into(), read("activewindow.json")),
                ("dispatch".into(), "ok".into()),
            ],
        );
        let windows = EngineWindows::compositor(
            Provider::Hyprland(compass_platform_linux::compositor::hyprland::Hyprland::new(
                path,
            )),
            Vec::new(),
        );
        let listed = windows.list_windows();
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[1].workspace_id.as_deref(), Some("1"));
        assert_eq!(listed[1].bounds.map(|b| b.width), Some(1260));
        assert_eq!(
            windows.focused_window().map(|w| w.wm_class),
            Some("foot".to_owned())
        );
        assert_eq!(
            windows.active_workspace().map(|w| (w.id, w.monitor)),
            Some(("1".to_owned(), Some("DP-1".to_owned())))
        );
        let workspaces = windows.list_workspaces();
        assert_eq!(workspaces.len(), 2);
        assert!(workspaces[1].fullscreen);
        windows.focus(&listed[0]);
        assert!(
            fake.seen()
                .last()
                .is_some_and(|last| last.contains("address:0x5581c8a1b2c0")),
            "{:?}",
            fake.seen()
        );
    }
}
