//! Launching an application on Linux.
//!
//! Three strategies, tried in order, unchanged from when this code lived in
//! `compass-platform`:
//!
//! 1. on the host, when running inside a Flatpak: the session helper's
//!    `HostCommand`, the call `flatpak-spawn --host` makes
//! 2. the XDG `OpenURI` portal
//! 3. a direct spawn
//!
//! The order matters and is not arbitrary. Inside the sandbox a direct spawn
//! reaches only what the sandbox contains, so it is the last resort rather
//! than the obvious first move.
//!
//! Desktop links instead use `xdg-open` with their URL, on the host when
//! sandboxed. They have no application Exec to expand or fall back to.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use ashpd::flatpak::Development;
use compass_platform::{AppLauncher, LaunchError, LaunchFuture, LaunchMethod};
use compass_portals::OpenOutcome;
use compass_xdg::DesktopEntry;
use tokio::process::Command as TokioCommand;
use tracing::{debug, info, warn};

/// Launches applications the way a Linux desktop expects.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinuxLauncher;

impl AppLauncher for LinuxLauncher {
    fn launch<'a>(&'a self, entry: &'a DesktopEntry, uris: &'a [&'a str]) -> LaunchFuture<'a> {
        Box::pin(launch_app_with_uris(entry, uris))
    }

    fn launch_action<'a>(
        &'a self,
        entry: &'a DesktopEntry,
        action_id: &'a str,
        uris: &'a [&'a str],
    ) -> LaunchFuture<'a> {
        Box::pin(async move {
            let exec = action_exec(entry, action_id, uris)?;
            launch_exec(exec).await
        })
    }
}

fn action_exec(
    entry: &DesktopEntry,
    action_id: &str,
    uris: &[&str],
) -> Result<Vec<String>, LaunchError> {
    let action = entry
        .actions()
        .iter()
        .find(|action| action.id() == action_id)
        .ok_or_else(|| LaunchError::UnknownAction(action_id.to_owned()))?;
    let exec = action.expand_exec_with(uris, false, None);
    if exec.is_empty() {
        return Err(LaunchError::NoExec);
    }
    Ok(exec)
}

#[cfg(test)]
mod action_tests {
    use super::*;

    fn entry() -> DesktopEntry {
        DesktopEntry::parse("[Desktop Entry]\nType=Application\nName=Browser\nExec=parent-app\nActions=private;broken;\n[Desktop Action private]\nName=Private Window\nExec=action-app --private %U\n[Desktop Action broken]\nName=Broken\n").unwrap()
    }

    #[test]
    fn action_uses_its_own_exec_and_preserves_uri_arguments() {
        assert_eq!(
            action_exec(&entry(), "private", &["https://example.test/a b"]).unwrap(),
            ["action-app", "--private", "https://example.test/a b"]
        );
    }

    #[test]
    fn unknown_action_never_falls_back_to_parent_exec() {
        assert!(
            matches!(action_exec(&entry(), "other", &[]), Err(LaunchError::UnknownAction(id)) if id == "other")
        );
    }

    #[test]
    fn missing_action_exec_never_falls_back_to_parent_exec() {
        assert!(matches!(
            action_exec(&entry(), "broken", &[]),
            Err(LaunchError::NoExec)
        ));
    }
}

/// Launch an application with URIs.
async fn launch_app_with_uris(
    entry: &DesktopEntry,
    uris: &[&str],
) -> Result<LaunchMethod, LaunchError> {
    if matches!(entry.entry_type(), compass_xdg::EntryType::Link) {
        let exec = desktop_link_exec(entry)?;
        // Resolve file: links on the host too: the sandbox may not contain
        // the target file even though its desktop entry is visible here.
        return if is_flatpak() {
            launch_on_host(&exec)
                .await
                .map(|()| LaunchMethod::FlatpakHost)
        } else {
            launch_direct(&exec).await.map(|()| LaunchMethod::Direct)
        };
    }
    let exec = entry.expand_exec_with(uris, false, None);
    launch_exec(exec).await
}

fn desktop_link_exec(entry: &DesktopEntry) -> Result<Vec<String>, LaunchError> {
    let uri = entry.url().ok_or(LaunchError::InvalidLinkUrl)?;
    let (scheme, _) = uri.split_once(':').ok_or(LaunchError::InvalidLinkUrl)?;
    if !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        || uri.chars().any(char::is_control)
    {
        return Err(LaunchError::InvalidLinkUrl);
    }
    Ok(vec!["xdg-open".to_owned(), uri.to_owned()])
}

async fn launch_exec(exec: Vec<String>) -> Result<LaunchMethod, LaunchError> {
    if exec.is_empty() {
        return Err(LaunchError::NoExec);
    }

    info!(?exec, "Launching application");

    // Inside a Flatpak, start it on the host first
    if is_flatpak() {
        match launch_on_host(&exec).await {
            Ok(()) => return Ok(LaunchMethod::FlatpakHost),
            Err(e) => warn!(%e, "starting it on the host failed, trying OpenURI"),
        }
    }

    // Try OpenURI portal
    match launch_via_open_uri(&exec).await {
        Ok(()) => return Ok(LaunchMethod::OpenUri),
        Err(e) => warn!(%e, "OpenURI failed, trying direct execution"),
    }

    // Fall back to direct execution
    launch_direct(&exec).await.map(|()| LaunchMethod::Direct)
}

/// Runs a command line on the host: through the Flatpak session helper inside a
/// Flatpak, directly outside one. For an argv that is not an application
/// launch (a terminal running an extension's command), so the portal step
/// [`LinuxLauncher`] tries for applications does not apply.
///
/// # Errors
///
/// When `argv` is empty or the process cannot be started.
pub async fn run_command(argv: &[String]) -> Result<LaunchMethod, LaunchError> {
    if argv.is_empty() {
        return Err(LaunchError::NoExec);
    }
    info!(?argv, "Running a command on the host");
    if is_flatpak() {
        launch_on_host(argv).await?;
        return Ok(LaunchMethod::FlatpakHost);
    }
    launch_direct(argv).await.map(|()| LaunchMethod::Direct)
}

/// A command that runs `program` on the host: through `flatpak-spawn --host`
/// inside a Flatpak, directly outside one. For a tool whose output the engine
/// reads, such as `pactl`.
#[must_use]
pub fn host_command(program: &str) -> std::process::Command {
    if is_flatpak() {
        let mut command = std::process::Command::new("flatpak-spawn");
        command.args(["--host", program]);
        command
    } else {
        std::process::Command::new(program)
    }
}

/// Check if we're running inside a Flatpak.
fn is_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// Starts `exec` on the host and returns once it is running.
///
/// This makes the call `flatpak-spawn --host` makes, `HostCommand` on the
/// Flatpak session helper, directly, because its answer is what a launch
/// needs: the helper replies with the pid once the process has started, or
/// with an error when it could not start it. `flatpak-spawn` would instead
/// stay attached until the process exits, which for an application is when
/// the user quits it, so the launch would not complete until then and an
/// application's non-zero exit would read as a failed launch. A direct spawn
/// already means "started"; this gives the Flatpak path the same meaning.
///
/// As with `flatpak-spawn` and no `--watch-bus`, the process outlives
/// Compass, starts in Compass's working directory with the session helper's
/// environment, and has `/dev/null` for its standard streams.
async fn launch_on_host(exec: &[String]) -> Result<(), LaunchError> {
    let failed = |error: &dyn std::fmt::Display| LaunchError::FlatpakHostFailed(error.to_string());
    let cwd = std::env::current_dir().map_err(|e| failed(&e))?;
    let null = File::options()
        .read(true)
        .write(true)
        .open("/dev/null")
        .map_err(|e| failed(&e))?;
    let fds: HashMap<u32, &File> = (0..3).map(|fd| (fd, &null)).collect();
    let pid = Development::new()
        .await
        .map_err(|e| failed(&e))?
        .host_command(&cwd, exec, fds, HashMap::new(), Default::default())
        .await
        .map_err(|e| failed(&e))?;
    debug!(pid, ?exec, "started on the host");
    Ok(())
}

/// Launch via the OpenURI portal.
async fn launch_via_open_uri(exec: &[String]) -> Result<(), LaunchError> {
    // For OpenURI, we need to construct a URI that the desktop will open.
    // This is tricky because OpenURI opens URIs, not arbitrary commands.
    // We use the "exec:" URI scheme if available, or fall back to launching
    // the command directly via the portal's OpenFile if it's a .desktop file.
    // For now, we just try to use the first arg as a URI if it looks like one.
    let uri = exec.first().ok_or(LaunchError::NoExec)?;

    let portals = compass_portals::Portals::connect(compass_portals::PortalConfig::default())
        .await
        .map_err(|e| LaunchError::OpenUriUnavailable(e.to_string()))?;

    let open_uri = portals
        .open_uri()
        .map_err(|e| LaunchError::OpenUriUnavailable(e.to_string()))?;

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

    cmd.spawn()
        .map(|_child| ())
        .map_err(|e| LaunchError::DirectFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_links_use_native_uri_dispatch_not_exec_or_a_shell() {
        for uri in [
            "file:///usr/share/doc/manual%20one.html",
            "https://example.test/a b?x=$HOME&y=;",
            "mailto:user@example.test",
        ] {
            let entry = DesktopEntry::parse(&format!(
                "[Desktop Entry]\nType=Link\nName=Link\nURL={uri}\nExec=wrong-command\n"
            ))
            .unwrap();
            assert_eq!(desktop_link_exec(&entry).unwrap(), ["xdg-open", uri]);
        }
    }

    #[test]
    fn relative_paths_options_and_missing_link_urls_are_rejected() {
        for url in [
            "",
            "--help",
            "/etc/passwd",
            "relative/path",
            "1invalid:value",
            ":value",
        ] {
            let entry = DesktopEntry::parse(&format!(
                "[Desktop Entry]\nType=Link\nName=Link\nURL={url}\n"
            ))
            .unwrap();
            assert!(
                matches!(desktop_link_exec(&entry), Err(LaunchError::InvalidLinkUrl)),
                "{url}"
            );
        }
        assert!(matches!(
            DesktopEntry::parse("[Desktop Entry]\nType=Link\nName=Missing\n"),
            Err(compass_xdg::Error::MissingUrl)
        ));
    }

    #[test]
    fn the_linux_launcher_is_an_app_launcher() {
        // The composition in `compass` holds an Arc<dyn AppLauncher>; this
        // fails to compile if LinuxLauncher stops satisfying it.
        let launcher: std::sync::Arc<dyn AppLauncher> = std::sync::Arc::new(LinuxLauncher);
        assert_eq!(format!("{launcher:?}"), "LinuxLauncher");
    }

    #[test]
    fn flatpak_detection_reads_the_sandbox_marker() {
        // Asserts what it actually checks rather than the answer, which
        // differs between a developer machine and the Flatpak CI job.
        assert_eq!(is_flatpak(), Path::new("/.flatpak-info").exists());
    }
}
