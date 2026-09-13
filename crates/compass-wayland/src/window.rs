//! The launcher window: a plain `xdg_toplevel` with activation and focus handling.

use std::sync::Arc;
use std::time::Duration;

use iced::window::Id as WindowId;
use iced_winit::conversion::window_id;
use tokio::sync::mpsc;
use tracing::{debug, warn};
use wayland_client::{
    protocol::wl_surface::WlSurface,
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::XdgSurface,
    xdg_toplevel::{self, XdgToplevel},
    xdg_wm_base::{self, XdgWmBase},
};
use winit::application::ApplicationHandler;
use winit::event::{WindowEvent as WinitWindowEvent, StartCause};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowAttributes;

use crate::activation::ActivationManager;
use crate::keyboard_inhibit::KeyboardInhibitManager;

/// Configuration for the launcher window.
#[derive(Debug, Clone)]
pub struct WindowConfig {
    /// Initial window width.
    pub width: f32,
    /// Initial window height.
    pub height: f32,
    /// Whether the window should be centered on the output.
    pub center: bool,
    /// Application ID for Wayland.
    pub app_id: String,
    /// Title shown in the window decoration (if any).
    pub title: String,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 640.0,
            height: 480.0,
            center: true,
            app_id: "com.vicinae.Vicinae".to_owned(),
            title: "Vicinae".to_owned(),
        }
    }
}

/// Events emitted by the Wayland window.
#[derive(Debug, Clone)]
pub enum WindowEvent {
    /// The window was shown and is ready.
    Opened,
    /// The window was closed.
    Closed,
    /// The window gained focus.
    Focused,
    /// The window lost focus.
    Blurred,
    /// The window was activated (via xdg-activation-v1).
    Activated,
}

/// The Wayland window state.
pub struct WaylandWindow {
    config: WindowConfig,
    event_sender: mpsc::UnboundedSender<WindowEvent>,
    activation: Option<ActivationManager>,
    keyboard_inhibit: Option<KeyboardInhibitManager>,
    xdg_wm_base: Option<XdgWmBase>,
    xdg_surface: Option<XdgSurface>,
    xdg_toplevel: Option<XdgToplevel>,
    wl_surface: Option<WlSurface>,
    event_loop: Option<EventLoop<()>>,
}

impl WaylandWindow {
    /// Create a new launcher window.
    pub fn new(config: WindowConfig) -> (Self, mpsc::UnboundedReceiver<WindowEvent>) {
        let (sender, receiver) = mpsc::unbounded_channel();

        let window = Self {
            config,
            event_sender: sender,
            activation: None,
            keyboard_inhibit: None,
            xdg_wm_base: None,
            xdg_surface: None,
            xdg_toplevel: None,
            wl_surface: None,
            event_loop: None,
        };

        (window, receiver)
    }

    /// Run the window event loop.
    ///
    /// This blocks until the window is closed.
    pub fn run(mut self) -> Result<(), WaylandError> {
        let event_loop = EventLoop::new().map_err(WaylandError::EventLoop)?;
        self.event_loop = Some(event_loop);

        // The actual window creation and Wayland protocol handling
        // happens when the event loop runs. For now, we set up the
        // connection and let Iced/winit handle the rest.
        // TODO: Implement full Wayland protocol handling

        // Placeholder - in reality this would run the Iced application
        // which internally manages the Wayland connection
        Ok(())
    }

    /// Get the window ID for Iced integration.
    pub fn window_id(&self) -> Option<WindowId> {
        // Would be set after window creation
        None
    }

    /// Request activation via xdg-activation-v1.
    pub async fn activate(&self) -> Result<(), WaylandError> {
        if let Some(activation) = &self.activation {
            activation.request_activation().await
        } else {
            Err(WaylandError::ActivationUnavailable)
        }
    }

    /// Enable keyboard shortcuts inhibit.
    pub async fn inhibit_shortcuts(&self) -> Result<(), WaylandError> {
        if let Some(inhibit) = &self.keyboard_inhibit {
            inhibit.inhibit().await
        } else {
            Err(WaylandError::KeyboardInhibitUnavailable)
        }
    }

    /// Disable keyboard shortcuts inhibit.
    pub async fn uninhibit_shortcuts(&self) -> Result<(), WaylandError> {
        if let Some(inhibit) = &self.keyboard_inhibit {
            inhibit.uninhibit().await
        } else {
            Err(WaylandError::KeyboardInhibitUnavailable)
        }
    }
}

/// Errors that can occur in Wayland window management.
#[derive(Debug, thiserror::Error)]
pub enum WaylandError {
    #[error("failed to create event loop: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("Wayland connection failed: {0}")]
    Connection(String),
    #[error("xdg-shell unavailable")]
    XdgShellUnavailable,
    #[error("activation unavailable")]
    ActivationUnavailable,
    #[error("keyboard inhibit unavailable")]
    KeyboardInhibitUnavailable,
    #[error("surface not ready")]
    SurfaceNotReady,
}

/// Internal state for the Wayland event handler.
struct WaylandState {
    window: Arc<WaylandWindow>,
    connection: Connection,
    queue_handle: QueueHandle<WaylandState>,
    event_sender: mpsc::UnboundedSender<WindowEvent>,
}

impl WaylandState {
    fn new(window: Arc<WaylandWindow>, connection: Connection, queue_handle: QueueHandle<Self>, event_sender: mpsc::UnboundedSender<WindowEvent>) -> Self {
        Self {
            window,
            connection,
            queue_handle,
            event_sender,
        }
    }
}

impl Dispatch<XdgWmBase, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &XdgWmBase,
        _event: <XdgWmBase as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgSurface, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &XdgSurface,
        _event: <XdgSurface as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgToplevel, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &XdgToplevel,
        _event: <XdgToplevel as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSurface,
        _event: <WlSurface as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

/// Iced application that wraps the Wayland window.
pub struct LauncherApplication {
    config: WindowConfig,
    event_sender: mpsc::UnboundedSender<WindowEvent>,
}

impl LauncherApplication {
    pub fn new(config: WindowConfig, event_sender: mpsc::UnboundedSender<WindowEvent>) -> Self {
        Self { config, event_sender }
    }
}

impl ApplicationHandler for LauncherApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = WindowAttributes::default()
            .with_title(self.config.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(self.config.width, self.config.height))
            .with_app_id(self.config.app_id.clone());

        match event_loop.create_window(window_attrs) {
            Ok(window) => {
                let _ = self.event_sender.send(WindowEvent::Opened);
                if self.config.center {
                    if let Some(monitor) = window.current_monitor() {
                        let monitor_size = monitor.size();
                        let window_size = window.inner_size();
                        let x = (monitor_size.width.saturating_sub(window_size.width)) / 2;
                        let y = (monitor_size.height.saturating_sub(window_size.height)) / 2;
                        window.set_outer_position(winit::dpi::LogicalPosition::new(x as f64, y as f64));
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to create window: {}", e);
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WinitWindowEvent,
    ) {
        match event {
            WinitWindowEvent::CloseRequested => {
                event_loop.exit();
                let _ = self.event_sender.send(WindowEvent::Closed);
            }
            WinitWindowEvent::Focused(focused) => {
                if focused {
                    let _ = self.event_sender.send(WindowEvent::Focused);
                    // Re-enable keyboard inhibit on focus
                    // TODO: Call inhibit_shortcuts
                } else {
                    let _ = self.event_sender.send(WindowEvent::Blurred);
                    // Disable keyboard inhibit on blur
                    // TODO: Call uninhibit_shortcuts
                }
            }
            _ => {}
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::Init) {
            // Initial activation request
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_config_default() {
        let config = WindowConfig::default();
        assert_eq!(config.width, 640.0);
        assert_eq!(config.height, 480.0);
        assert!(config.center);
        assert_eq!(config.app_id, "com.vicinae.Vicinae");
    }
}