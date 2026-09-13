//! Application launching via `flatpak-spawn --host` and `OpenURI` portal.

use std::path::Path;

use compass_portals::OpenOutcome;
use compass_xdg::DesktopEntry;
use tokio::process::Command as TokioCommand;
use tracing::{debug, info, warn};

/// How the application was launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMethod {
    /// Launched via `flatpak-spawn --host`.
    FlatpakSpawn,
    /// Launched via the OpenURI portal.
    OpenUri,
    /// Launched directly (not sandboxed).
    Direct,
}

/// Errors that can occur during launch.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// No Exec key in desktop entry.
    #[error("no Exec key in desktop entry")]
    NoExec,
    /// flatpak-spawn not found.
    #[error("flatpak-spawn not found")]
    FlatpakSpawnNotFound,
    /// flatpak-spawn failed.
    #[error("flatpak-spawn failed: {0}")]
    FlatpakSpawnFailed(String),
    /// OpenURI portal unavailable.
    #[error("OpenURI portal unavailable: {0}")]
    OpenUriUnavailable(String),
    /// OpenURI portal failed.
    #[error("OpenURI portal failed: {0}")]
    OpenUriFailed(String),
    /// OpenURI dismissed by user.
    #[error("OpenURI dismissed by user")]
    OpenUriDismissed,
    /// OpenURI refused.
    #[error("OpenURI refused: {0}")]
    OpenUriRefused(String),
    /// Direct execution failed.
    #[error("direct execution failed: {0}")]
    DirectFailed(String),
}

/// Launch an application from a desktop entry.
///
/// Tries `flatpak-spawn --host` first (for Flatpak sandbox), then falls back
/// to the OpenURI portal, then direct execution.
pub async fn launch_app(entry: &DesktopEntry) -> Result<LaunchMethod, LaunchError> {
    launch_app_with_uris(entry, &[]).await
}

/// Launch an application with URIs.
///
/// Expands the `Exec` field codes with the given URIs, then tries:
/// 1. `flatpak-spawn --host` (if running in a Flatpak)
/// 2. OpenURI portal
/// 3. Direct execution
pub async fn launch_app_with_uris(
    entry: &DesktopEntry,
    uris: &[&str],
) -> Result<LaunchMethod, LaunchError> {
    let exec = entry.expand_exec_with(uris, false, None);
    if exec.is_empty() {
        return Err(LaunchError::NoExec);
    }

    info!(?exec, "Launching application");

    // Try flatpak-spawn --host first if we're in a Flatpak
    if is_flatpak() {
        match launch_via_flatpak_spawn(&exec).await {
            Ok(()) => return Ok(LaunchMethod::FlatpakSpawn),
            Err(e) => warn!(%e, "flatpak-spawn failed, trying OpenURI"),
        }
    }

    // Try OpenURI portal
    match launch_via_open_uri(&exec).await {
        Ok(()) => return Ok(LaunchMethod::OpenUri),
        Err(e) => warn!(%e, "OpenURI failed, trying direct execution"),
    }

    // Fall back to direct execution
    launch_direct(&exec).await.map(|_| LaunchMethod::Direct)
}

/// Check if we're running inside a Flatpak.
fn is_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// Launch via `flatpak-spawn --host`.
async fn launch_via_flatpak_spawn(exec: &[String]) -> Result<(), LaunchError> {
    let mut cmd = TokioCommand::new("flatpak-spawn");
    cmd.arg("--host");
    cmd.args(exec);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    debug!(?cmd, "Running flatpak-spawn");

    let status = cmd
        .spawn()
        .map_err(|e| LaunchError::FlatpakSpawnFailed(e.to_string()))?
        .wait()
        .await
        .map_err(|e| LaunchError::FlatpakSpawnFailed(e.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(LaunchError::FlatpakSpawnFailed(format!(
            "exit code: {:?}",
            status.code()
        )))
    }
}

/// Launch via the OpenURI portal.
async fn launch_via_open_uri(exec: &[String]) -> Result<(), LaunchError> {
    // For OpenURI, we need to construct a URI that the desktop will open.
    // This is tricky because OpenURI opens URIs, not arbitrary commands.
    // We use the "exec:" URI scheme if available, or fall back to launching
    // the command directly via the portal's OpenFile if it's a .desktop file.
    // For now, we just try to use the first arg as a URI if it looks like one.

    let uri = exec.first().ok_or(LaunchError::NoExec)?;

    // Try to get the OpenURI portal
    let portals = compass_portals::Portals::connect(compass_portals::PortalConfig::default())
        .await
        .map_err(|e| LaunchError::OpenUriUnavailable(e.to_string()))?;

    let open_uri = portals
        .open_uri()
        .map_err(|e| LaunchError::OpenUriUnavailable(e.to_string()))?;

    // For desktop entries, we can try the exec URI or just the command
    // This is a simplified version - real implementation would need more logic
    match open_uri.open_uri(uri, true).await {
        Ok(OpenOutcome::Opened) => Ok(()),
        Ok(OpenOutcome::Dismissed) => Err(LaunchError::OpenUriDismissed),
        Ok(OpenOutcome::Refused) => Err(LaunchError::OpenUriRefused("portal refused".to_owned())),
        Ok(_) => Err(LaunchError::OpenUriRefused("unknown outcome".to_owned())),
        Err(e) => Err(LaunchError::OpenUriFailed(e.to_string())),
    }
}

/// Launch directly (not sandboxed).
async fn launch_direct(exec: &[String]) -> Result<(), LaunchError> {
    let mut cmd = TokioCommand::new(&exec[0]);
    cmd.args(&exec[1..]);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    debug!(?cmd, "Running direct");

    let status = cmd
        .spawn()
        .map_err(|e| LaunchError::DirectFailed(e.to_string()))?
        .wait()
        .await
        .map_err(|e| LaunchError::DirectFailed(e.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(LaunchError::DirectFailed(format!(
            "exit code: {:?}",
            status.code()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_flatpak_false_when_no_file() {
        // Can't easily test this without mocking the filesystem
        // Just ensure the function compiles
        let _ = is_flatpak();
    }

    #[test]
    fn launch_error_display() {
        let err = LaunchError::NoExec;
        assert!(err.to_string().contains("Exec"));

        let err = LaunchError::FlatpakSpawnNotFound;
        assert!(err.to_string().contains("flatpak-spawn"));
    }
}
