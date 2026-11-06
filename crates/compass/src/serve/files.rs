//! A file's action panel beyond Open and Show in file browser
//! (`FileActions::actionPanel`, `src/server/src/utils/file-list-item.hpp`):
//! what the panel depends on, copying or pasting the file as a file, running
//! it as a program, and making it the wallpaper.

use std::path::Path;
use std::sync::Arc;

use compass_ipc::{ErrorKind, FileActionInfo, ProtocolError, Request, Response};
use tokio::sync::RwLock;

use super::EngineState;

/// The clipboard payload for a file: its `file://` URI, as a `text/uri-list`.
#[must_use]
pub fn uri_list(path: &Path) -> Vec<u8> {
    url::Url::from_file_path(path)
        .map_or_else(
            |()| format!("file://{}", path.display()),
            |url| url.to_string(),
        )
        .into_bytes()
}

/// Gives `path` the owner's execute permission, as `RunExecutableAction`
/// does with `mkExec`.
///
/// # Errors
///
/// The C++'s sentence, when the permission cannot be changed.
pub fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let failed = || "Failed to give executable permission".to_owned();
    let mut permissions = std::fs::metadata(path).map_err(|_| failed())?.permissions();
    permissions.set_mode(permissions.mode() | 0o100);
    std::fs::set_permissions(path, permissions).map_err(|_| failed())
}

fn bad_path() -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::BadRequest,
        "no file exists at that path",
    ))
}

fn refused(message: impl Into<String>) -> Response {
    Response::Error(ProtocolError::new(ErrorKind::Internal, message))
}

/// Answers the file-action requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    let path = match &request {
        Request::FileActions { path }
        | Request::CopyFile { path, .. }
        | Request::RunExecutable { path, .. }
        | Request::SetWallpaper { path } => std::path::PathBuf::from(path),
        _ => return bad_path(),
    };
    if !path.is_absolute() || !path.exists() {
        return bad_path();
    }
    match request {
        Request::FileActions { .. } => {
            use compass_worker_host::application_service::Apps as _;
            let has_opener = {
                let state = state.read().await;
                let apps = super::engine_apps_now(&state);
                apps.default_opener(&path.to_string_lossy()).is_some()
            };
            let can_paste = crate::paste::can_paste(state).await;
            let handle = tokio::runtime::Handle::current();
            let can_set_wallpaper = tokio::task::spawn_blocking(move || {
                crate::extension_wallpaper::EngineWallpaper::new(Some(handle)).can_set()
            })
            .await
            .unwrap_or(false);
            Response::FileActions(FileActionInfo {
                mime: Some(compass_xdg::mimeapps::file_mime(&path)),
                has_opener,
                can_set_wallpaper,
                can_paste,
            })
        }
        Request::CopyFile { paste, .. } => copy_file(state, &path, paste).await,
        Request::RunExecutable {
            make_executable: make,
            ..
        } => {
            if make && let Err(reason) = make_executable(&path) {
                return refused(reason);
            }
            let argv = vec![path.to_string_lossy().into_owned()];
            match compass_platform_linux::run_command(&argv).await {
                Ok(_) => Response::Ack,
                Err(error) => {
                    tracing::info!(%error, "an executable did not start");
                    refused("Failed to start executable")
                }
            }
        }
        Request::SetWallpaper { .. } => {
            let handle = tokio::runtime::Handle::current();
            let request = compass_worker_host::wallpaper_service::Request {
                path: path.to_string_lossy().into_owned(),
                screen: None,
                fit: compass_worker_host::wallpaper_service::Fit::default(),
            };
            let set = tokio::task::spawn_blocking(move || {
                use compass_worker_host::wallpaper_service::Wallpaper as _;
                crate::extension_wallpaper::EngineWallpaper::new(Some(handle)).set(&request)
            })
            .await
            .unwrap_or_else(|err| Err(err.to_string()));
            match set {
                Ok(()) => Response::Ack,
                Err(reason) => refused(format!("Failed to set wallpaper: {reason}")),
            }
        }
        _ => bad_path(),
    }
}

/// Puts the file on the clipboard as a file: over data-control on wlroots,
/// through the Shell extension elsewhere; a paste goes through
/// [`crate::paste`].
async fn copy_file(state: &Arc<RwLock<EngineState>>, path: &Path, paste: bool) -> Response {
    const WHAT: &str = "Copying the file";
    let payload = uri_list(path);
    if paste {
        let content = compass_shell::ClipboardContent::binary(payload, "text/uri-list");
        return crate::paste::paste(state, content, "Pasting the file").await;
    }
    if crate::wlroots::detect()
        .await
        .is_some_and(|wlroots| wlroots.capabilities.data_control)
    {
        let offers = vec![compass_wayland::data_control::Offer {
            mime_type: "text/uri-list".to_owned(),
            data: payload,
        }];
        return match tokio::task::spawn_blocking(move || compass_wayland::clipboard::set(offers))
            .await
        {
            Ok(Ok(())) => Response::Ack,
            Ok(Err(err)) => refused(format!("{WHAT} failed: {err}")),
            Err(err) => refused(format!("{WHAT} failed: {err}")),
        };
    }
    let Some(shell) = state.read().await.shell.client.clone() else {
        return Response::Error(crate::window_service::no_bus(WHAT));
    };
    let content = compass_shell::ClipboardContent::binary(payload, "text/uri-list");
    match shell.set_clipboard(&content).await {
        Ok(()) => Response::Ack,
        Err(err) => Response::Error(crate::window_service::refusal(&err, WHAT)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_executable_is_given_the_owners_execute_permission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("Tool.AppImage");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o644)).unwrap();
        make_executable(&tool).unwrap();
        let mode = std::fs::metadata(&tool).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o744);
        assert_eq!(
            make_executable(&dir.path().join("gone")),
            Err("Failed to give executable permission".to_owned())
        );
    }

    #[test]
    fn a_file_is_copied_as_its_escaped_file_uri() {
        assert_eq!(
            uri_list(Path::new("/home/ä/a b.png")),
            b"file:///home/%C3%A4/a%20b.png".to_vec()
        );
    }
}
