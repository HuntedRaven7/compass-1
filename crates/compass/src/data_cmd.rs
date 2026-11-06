//! `compass data remove`: deleting a user's Compass data when they ask.
//!
//! Uninstalling a package keeps every user's configuration and data
//! (`packaging/README.md`), so this is the one way Compass deletes them.
//! [`compass_xdg::user_data`] decides what is Compass's; this module lists it,
//! refuses while an engine is using it, and deletes only with `--yes`.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use compass_ipc::{Request, Response, SocketPath};
use compass_xdg::user_data::{self, Base, Found, Owner, UserPath};

use crate::cli::DataCommand;
use crate::{EXIT_FAILURE, EXIT_OK, ipc};

/// How long a live engine has to answer before it is taken to be running but
/// stuck, which still means its files are in use.
const PING_TIMEOUT: Duration = Duration::from_secs(2);

/// Runs a `compass data` subcommand.
///
/// # Errors
///
/// When the directories cannot be inspected, or Compass is running and `--yes` was passed.
pub async fn run(socket: &SocketPath, command: DataCommand) -> Result<ExitCode> {
    match command {
        DataCommand::Remove {
            yes,
            include_vicinae,
        } => remove(socket, yes, include_vicinae).await,
    }
}

async fn remove(socket: &SocketPath, yes: bool, include_vicinae: bool) -> Result<ExitCode> {
    if yes && engine_answers(socket).await {
        bail!(
            "Compass is running and using these files. Quit it with `compass shutdown`, or \
             `systemctl --user stop compass` if it runs as a service, then run this again."
        );
    }
    let paths =
        user_data::survey_env(include_vicinae).context("listing Compass's data directories")?;
    print!("{}", render_plan(&paths, include_vicinae, yes));
    if !yes {
        return Ok(ExitCode::from(EXIT_OK));
    }
    let outcomes = user_data::remove(&paths);
    let mut failed = false;
    for (entry, outcome) in outcomes {
        match outcome {
            Ok(()) => println!("Removed {}.", entry.path.display()),
            Err(error) => {
                failed = true;
                eprintln!("error: {error}");
            }
        }
    }
    Ok(ExitCode::from(if failed { EXIT_FAILURE } else { EXIT_OK }))
}

/// Whether an engine answers on `socket`. One that accepts the connection but
/// does not answer in time counts as running.
pub(crate) async fn engine_answers(socket: &SocketPath) -> bool {
    match tokio::time::timeout(PING_TIMEOUT, ipc::send(socket, Request::Ping)).await {
        Ok(Ok(Response::Pong { .. })) | Err(_) => true,
        Ok(_) => false,
    }
}

/// The listing: each base directory's candidates and what will happen to
/// each, then, for a dry run, how to go ahead.
#[must_use]
pub fn render_plan(paths: &[UserPath], include_vicinae: bool, yes: bool) -> String {
    let mut out = String::new();
    let mut last_base = None;
    for entry in paths {
        if last_base != Some(entry.base) {
            last_base = Some(entry.base);
            let _ = writeln!(out, "{}", heading(entry.base));
        }
        let _ = writeln!(out, "  {}", describe(entry));
    }
    let any = paths.iter().any(|entry| entry.remove);
    let footer = match (any, yes, include_vicinae) {
        (false, _, _) => Some("There is nothing to remove."),
        (true, true, _) => None,
        (true, false, false) => Some(
            "Nothing has been removed yet. Run `compass data remove --yes` to remove the paths above.",
        ),
        (true, false, true) => Some(
            "Nothing has been removed yet. Run `compass data remove --yes --include-vicinae` to remove the paths above.",
        ),
    };
    if let Some(footer) = footer {
        let _ = writeln!(out, "{footer}");
    }
    out
}

fn heading(base: Base) -> String {
    let variable = base.variable();
    match base {
        Base::Config => format!("Configuration (${variable}):"),
        Base::Data => format!("Data (${variable}):"),
        Base::Cache => format!("Cache (${variable}):"),
        Base::State => format!("State (${variable}):"),
    }
}

fn describe(entry: &UserPath) -> String {
    let path = entry.path.display();
    match (&entry.found, entry.owner, entry.remove) {
        (Found::Missing, _, _) => format!("{path} does not exist."),
        (_, Owner::CompatibilityLink, _) => format!(
            "{path} is the link to Compass's directory left at Vicinae's old name. It will be removed."
        ),
        (Found::Symlink { target }, _, true) => {
            let target = link_target(&entry.path, target);
            format!(
                "{path} is a link to {}. The link will be removed; what it points to is kept.",
                target.display()
            )
        }
        (_, Owner::Vicinae, false) => format!(
            "{path} belongs to Vicinae and will be kept. Pass --include-vicinae to remove it as well."
        ),
        (_, Owner::Vicinae, true) => {
            format!(
                "{path} belongs to Vicinae. It will be removed because --include-vicinae was passed."
            )
        }
        (_, Owner::Compass, _) => format!("{path} will be removed with everything in it."),
    }
}

/// A link's target as a full path, so the listing names exactly what is kept.
fn link_target(link: &Path, target: &Path) -> std::path::PathBuf {
    match link.parent() {
        Some(dir) if target.is_relative() => dir.join(target),
        _ => target.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(path: &str, found: Found, owner: Owner, remove: bool) -> UserPath {
        UserPath {
            base: Base::Config,
            path: PathBuf::from(path),
            found,
            owner,
            remove,
        }
    }

    #[test]
    fn the_listing_says_what_goes_and_what_stays() {
        let paths = [
            entry("/h/.config/compass", Found::Directory, Owner::Compass, true),
            entry(
                "/h/.config/vicinae",
                Found::Symlink {
                    target: PathBuf::from("compass"),
                },
                Owner::CompatibilityLink,
                true,
            ),
        ];
        let plan = render_plan(&paths, false, false);
        assert!(plan.starts_with("Configuration ($XDG_CONFIG_HOME):\n"));
        assert!(plan.contains("/h/.config/compass will be removed with everything in it."));
        assert!(plan.contains(
            "/h/.config/vicinae is the link to Compass's directory left at Vicinae's old name."
        ));
        assert!(plan.ends_with("Run `compass data remove --yes` to remove the paths above.\n"));
        // With `--yes` the removals follow, so there is nothing more to say.
        assert!(!render_plan(&paths, false, true).contains("Nothing has been removed"));
    }

    #[test]
    fn vicinae_s_directory_is_kept_unless_asked() {
        let kept = entry(
            "/h/.config/vicinae",
            Found::Directory,
            Owner::Vicinae,
            false,
        );
        assert!(describe(&kept).contains("will be kept"));
        let plan = render_plan(&[kept], false, false);
        assert!(plan.ends_with("There is nothing to remove.\n"));
    }
}
