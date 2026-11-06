//! The shell a command talks to (`UiShellService`'s side in the engine):
//! toasts, HUDs, notifications, alerts and the selected text.
//!
//! Separate because it is where a run meets the desktop: a view's toasts go
//! to its state, while a `no-view` command's HUD and failures go to the
//! launcher window and to desktop notifications.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use compass_core::alert::Alert;
use compass_worker_host::tsapi::Deferral;
use compass_worker_host::ui_shell_service::{
    CloseWindow, CommandInfo, Notification, Shell, ToastStyle,
};

use super::Launcher;
use super::clipboard::data_control;
use super::views::ViewState;

/// The shell around a command with no view: what it would show on screen
/// becomes a desktop notification.
pub(super) struct HeadlessShell {
    pub(super) title: String,
    pub(super) handle: Option<tokio::runtime::Handle>,
    /// The alert `show_alert` was last given, for the serving loop to hand
    /// the launcher when the call defers.
    pub(super) alert: std::sync::Mutex<Option<compass_ipc::ExtensionAlert>>,
    /// A view's state, where its toasts are shown; `None` for a no-view run,
    /// whose toasts become notifications.
    pub(super) view: Option<tokio::sync::watch::Sender<ViewState>>,
    /// Where `getSelectedText` reads from.
    pub(super) selection: Selection,
    /// The extension's `assets` directory, which a notification's icon may
    /// name a file in.
    pub(super) assets: Option<PathBuf>,
    /// The launcher window, for a no-view run.
    pub(super) launcher: Option<Launcher>,
}

/// `getSelectedText`'s answer, verbatim from the C++, when nothing is
/// selected or there is nowhere to read a selection from.
pub const NO_SELECTED_TEXT: &str = "Unable to get selected text";

/// The primary selection, as `LinuxSelectionService` reads it: over
/// data-control on a wlroots compositor, and through the Shell extension on
/// GNOME, where Mutter has no data-control and only a focused client may read
/// the primary selection.
pub(super) struct Selection {
    pub(super) shell: Option<Arc<compass_shell::ShellClient>>,
    pub(super) handle: Option<tokio::runtime::Handle>,
}

impl Selection {
    fn text(&self) -> Result<String, String> {
        let text = if data_control() {
            compass_wayland::clipboard::read_primary_text().unwrap_or_else(|err| {
                tracing::info!(error = %err, "could not read the primary selection");
                None
            })
        } else if let (Some(shell), Some(handle)) = (&self.shell, &self.handle) {
            let shell = Arc::clone(shell);
            handle
                .block_on(async move { shell.primary_selection().await })
                .unwrap_or_else(|err| {
                    tracing::info!(error = %err, "could not read the primary selection");
                    None
                })
        } else {
            None
        };
        text.ok_or_else(|| NO_SELECTED_TEXT.to_owned())
    }
}

/// The notification `FreedesktopNotificationClient::send` posts: from
/// `Compass`, with the extension's urgency as the `urgency` hint when it gave
/// one, and its icon as the file [`crate::notification_icon`] made of it.
fn desktop_notification(
    title: &str,
    body: &str,
    urgency: Option<compass_worker_host::ui_shell_service::Urgency>,
    icon: Option<&Path>,
) -> notify_rust::Notification {
    use compass_worker_host::ui_shell_service::Urgency;
    let mut notification = notify_rust::Notification::new();
    notification.appname("Compass").summary(title).body(body);
    if let Some(urgency) = urgency {
        notification.urgency(match urgency {
            Urgency::Low => notify_rust::Urgency::Low,
            Urgency::Normal => notify_rust::Urgency::Normal,
            Urgency::High => notify_rust::Urgency::Critical,
        });
    }
    if let Some(path) = icon {
        notification.icon(&path.to_string_lossy());
    }
    notification
}

impl HeadlessShell {
    fn notify(&self, title: &str, body: &str) {
        self.post(desktop_notification(title, body, None, None));
    }

    /// Hands the launcher `command`; answers whether it took it.
    fn tell_launcher(&self, command: compass_ipc::WindowCommand) -> bool {
        self.launcher
            .as_ref()
            .is_some_and(|launcher| (launcher.0)(command))
    }

    /// A no-view run's failure: in the launcher, as its error line and in its
    /// HUD, and as a desktop notification too, which outlasts the HUD.
    pub(super) fn fail(&self, title: &str, message: &str) {
        let told = self.tell_launcher(compass_ipc::WindowCommand::Failure {
            title: title.to_owned(),
            message: message.to_owned(),
        });
        if !told {
            tracing::info!(
                command = self.title,
                title,
                message,
                "no launcher to tell of a failure"
            );
        }
        self.notify(title, message);
    }

    fn post(&self, notification: notify_rust::Notification) {
        let (title, body) = (notification.summary.clone(), notification.body.clone());
        let Some(handle) = &self.handle else {
            tracing::info!(
                command = self.title,
                title,
                body,
                "no runtime to notify from"
            );
            return;
        };
        if let Err(err) = handle.block_on(notification.show_async()) {
            tracing::info!(command = self.title, title, body, error = %err, "not notified");
        }
    }
}

impl Shell for HeadlessShell {
    fn set_toast(&self, title: &str, style: ToastStyle, message: &str) {
        if let Some(view) = &self.view {
            let toast = compass_ipc::ExtensionToast {
                title: title.to_owned(),
                message: message.to_owned(),
                style: match style {
                    ToastStyle::Success => compass_ipc::ExtensionToastStyle::Success,
                    ToastStyle::Info => compass_ipc::ExtensionToastStyle::Info,
                    ToastStyle::Warning => compass_ipc::ExtensionToastStyle::Warning,
                    ToastStyle::Danger => compass_ipc::ExtensionToastStyle::Failure,
                    ToastStyle::Dynamic => compass_ipc::ExtensionToastStyle::Animated,
                },
            };
            view.send_modify(|state| {
                state.version += 1;
                state.toast = Some(toast);
            });
            return;
        }
        // A no-view command's success toast is the same news as the HUD it
        // usually shows next; only a failure is worth interrupting for.
        if style == ToastStyle::Danger {
            self.fail(title, message);
        } else {
            tracing::info!(command = self.title, title, message, "toast");
        }
    }

    fn clear_toast(&self) {
        if let Some(view) = &self.view {
            view.send_modify(|state| {
                if state.toast.take().is_some() {
                    state.version += 1;
                }
            });
        }
    }

    fn close_window(&self, _options: CloseWindow) {}

    fn show_hud(&self, text: &str) {
        let shown = self.tell_launcher(compass_ipc::WindowCommand::Hud {
            text: text.to_owned(),
            icon: None,
        });
        if !shown {
            self.notify(&self.title, text);
        }
    }

    fn pop_to_root(&self, _clear_search: bool) {}

    fn push_view(&self, _command: &CommandInfo) {}

    fn pop_view(&self) {}

    fn set_search_text(&self, _text: &str) {}

    fn selected_text(&self) -> Result<String, String> {
        self.selection.text()
    }

    fn send_notification(&self, notification: &Notification) {
        let icon = notification.icon.as_ref().and_then(|icon| {
            crate::notification_icon::icon_path(
                icon,
                &crate::notification_icon::Sources::from_environment(self.assets.as_deref()),
            )
        });
        self.post(desktop_notification(
            &notification.title,
            &notification.body,
            Some(notification.urgency),
            icon.as_deref(),
        ));
    }

    fn show_alert(&self, alert: &Alert, _deferral: &Deferral) {
        *self
            .alert
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(compass_ipc::ExtensionAlert {
                title: alert.title.clone(),
                message: alert.message.clone(),
                confirm_text: alert.confirm_text.clone(),
                cancel_text: alert.cancel_text.clone(),
                remember_text: None,
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notification_carries_the_urgency_and_an_icon_file() {
        use compass_worker_host::ui_shell_service::Urgency;
        use notify_rust::Hint;
        let dir = tempfile::tempdir().expect("tempdir");
        let png = dir.path().join("bell.png");
        std::fs::write(&png, b"\x89PNG").expect("icon");
        let png = png.to_string_lossy().into_owned();

        for (urgency, hint) in [
            (Urgency::Low, notify_rust::Urgency::Low),
            (Urgency::Normal, notify_rust::Urgency::Normal),
            (Urgency::High, notify_rust::Urgency::Critical),
        ] {
            let posted = desktop_notification("T", "B", Some(urgency), None);
            assert!(posted.hints.contains(&Hint::Urgency(hint)), "{urgency:?}");
            assert_eq!((posted.summary.as_str(), posted.body.as_str()), ("T", "B"));
            assert_eq!(posted.appname, "Compass");
        }
        let plain = desktop_notification("T", "B", None, None);
        assert!(plain.hints.is_empty(), "a HUD sends no urgency, as before");

        assert_eq!(
            desktop_notification("T", "B", None, Some(Path::new(&png))).icon,
            png
        );
        assert_eq!(plain.icon, "", "no icon, none passed");
    }
}
