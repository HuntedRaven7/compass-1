//! `compass clipboard reset`: the way out when clipboard history cannot be
//! opened.
//!
//! The engine never deletes a history it cannot read, and never replaces a
//! keyring entry it did not write ([`crate::clipboard_service`]), so a lost key
//! or a foreign keyring entry leaves clipboard history off until someone acts.
//! This is that action. [`decide`] works out what is safe from what is on
//! disk and in the keyring, the plan is listed, and nothing changes without
//! `--yes`, which is refused while an engine has the files open.
//!
//! A history the key opens is deleted, since the user asked for a new one. One
//! it does not open is moved into a backup directory instead: its key may be in
//! a keyring that comes back. A locked or unreachable keyring is refused rather
//! than guessed at, because then it cannot be told which of the two applies.

use std::fmt::Write as _;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use compass_clipboard::files::{self, Disposal, Part, PartKind, ReadError, ResetPlan, Survey};
use compass_ipc::SocketPath;

use crate::cli::ClipboardCommand;
use crate::clipboard_service::{self, KEYRING_LABEL, KeyState};
use crate::{EXIT_OK, data_cmd};

/// Runs a `compass clipboard` subcommand.
///
/// # Errors
///
/// When the history cannot be inspected, the keyring cannot tell whether it
/// is readable, Compass is running and `--yes` was passed, or the reset fails.
pub async fn run(socket: &SocketPath, command: ClipboardCommand) -> Result<ExitCode> {
    match command {
        ClipboardCommand::Reset { yes } => reset(socket, yes).await,
    }
}

async fn reset(socket: &SocketPath, yes: bool) -> Result<ExitCode> {
    if yes && data_cmd::engine_answers(socket).await {
        bail!(
            "Compass is running and using the clipboard history. Quit it with `compass shutdown`, \
             or `systemctl --user stop compass` if it runs as a service, then run this again."
        );
    }
    let inspection = clipboard_service::inspect()
        .await
        .context("finding the clipboard history")?;
    let survey = inspection.survey.as_ref().map_err(|err| {
        anyhow!(
            "could not list the clipboard history in {}: {err}",
            inspection.dir.display()
        )
    })?;
    let (disposal, remove_key) = match decide(survey, &inspection.key, inspection.read.as_ref()) {
        Decision::Nothing => {
            println!("There is no clipboard history to reset.");
            return Ok(ExitCode::from(EXIT_OK));
        }
        Decision::Refuse(reason) => bail!(reason),
        Decision::Reset {
            disposal,
            remove_key,
        } => (disposal, remove_key),
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let plan = files::plan_reset(survey, disposal, stamp);
    let entries = inspection.read.as_ref().and_then(|read| read.as_ref().ok());
    print!("{}", render_plan(&plan, entries.copied(), remove_key, yes));
    if !yes {
        return Ok(ExitCode::from(EXIT_OK));
    }
    tokio::task::spawn_blocking(move || files::apply_reset(&plan))
        .await
        .context("the reset task failed")??;
    if remove_key {
        clipboard_service::Oo7Store::connect_quietly()
            .await?
            .delete_key()
            .await
            .context("removing the keyring entry")?;
        println!("Removed the keyring entry \"{KEYRING_LABEL}\".");
    }
    println!("Done. Compass starts a new clipboard history the next time it starts.");
    Ok(ExitCode::from(EXIT_OK))
}

/// What a reset may do, given what is on disk and in the keyring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// There is nothing to reset.
    Nothing,
    /// It cannot be told what is safe; why, as the user should read it.
    Refuse(String),
    /// Reset the files this way, and remove the keyring entry when it is not
    /// a key.
    Reset {
        /// Delete the files, or move them into a backup.
        disposal: Disposal,
        /// Whether the keyring entry is to go.
        remove_key: bool,
    },
}

/// Decides what a reset of `survey` may do. `read` is what reading it with
/// the key found gave, when there was a database and a key to try.
#[must_use]
pub fn decide(survey: &Survey, key: &KeyState, read: Option<&Result<u64, ReadError>>) -> Decision {
    // An entry that is not a key cannot have encrypted anything Compass wrote,
    // so removing it loses nothing; keeping it keeps clipboard history off.
    let remove_key = matches!(key, KeyState::Malformed(_));
    if survey.part(PartKind::Database).is_none() {
        // Without a database the payloads are unreachable, whatever the key.
        return if survey.is_empty() && !remove_key {
            Decision::Nothing
        } else {
            Decision::Reset {
                disposal: Disposal::Delete,
                remove_key,
            }
        };
    }
    let backup = Decision::Reset {
        disposal: Disposal::Backup,
        remove_key,
    };
    match key {
        KeyState::Found(_) => match read {
            Some(Ok(_)) => Decision::Reset {
                disposal: Disposal::Delete,
                remove_key,
            },
            Some(Err(ReadError::Unreadable | ReadError::NotInitialised)) => backup,
            Some(Err(err)) => Decision::Refuse(format!(
                "The clipboard history could not be read ({err}), so Compass cannot tell \
                 whether it is safe to delete. Make sure Compass is not running and try again."
            )),
            None => Decision::Refuse(
                "The clipboard history was not read, so Compass cannot tell whether it is safe \
                 to delete. Try again."
                    .to_owned(),
            ),
        },
        KeyState::Missing | KeyState::Malformed(_) => backup,
        KeyState::Locked => Decision::Refuse(
            "The keyring is locked, so Compass cannot tell whether the clipboard history can \
             still be read. Unlock the keyring and run this again."
                .to_owned(),
        ),
        KeyState::Unavailable(err) => Decision::Refuse(format!(
            "No keyring answered ({err}), so Compass cannot tell whether the clipboard history \
             can still be read. Run this again in your desktop session, with the keyring \
             running."
        )),
    }
}

/// The listing: why, then each file and what happens to it, then, for a dry
/// run, how to go ahead.
#[must_use]
pub fn render_plan(plan: &ResetPlan, entries: Option<u64>, remove_key: bool, yes: bool) -> String {
    let mut out = String::new();
    let heading = match (&plan.backup_dir, entries) {
        (Some(backup), _) => format!(
            "The clipboard history cannot be read with the key in the keyring, so it will not \
             be deleted. These files will be moved into {}:",
            backup.display()
        ),
        (None, Some(1)) => "The clipboard history opens with the key in the keyring and holds 1 \
                            entry. These files will be deleted, pinned entries included:"
            .to_owned(),
        (None, Some(entries)) => format!(
            "The clipboard history opens with the key in the keyring and holds {entries} \
             entries. These files will be deleted, pinned entries included:"
        ),
        (None, None) => "There is no clipboard history database, only files left from one. \
                         These will be deleted:"
            .to_owned(),
    };
    if !plan.parts.is_empty() {
        let _ = writeln!(out, "{heading}");
        for part in &plan.parts {
            let _ = writeln!(out, "  {}", describe(part, plan.backup_dir.is_some()));
        }
    }
    if remove_key {
        let _ = writeln!(
            out,
            "The keyring entry \"{KEYRING_LABEL}\" is not a key Compass made. It will be \
             removed, and Compass will make a new key when it next starts."
        );
    }
    if !yes {
        let _ = writeln!(
            out,
            "Nothing has been changed yet. Run `compass clipboard reset --yes` to go ahead."
        );
    }
    out
}

fn describe(part: &Part, moved: bool) -> String {
    let path = part.path.display();
    let bytes = part.bytes;
    match (part.directory, part.files, moved) {
        (true, 1, true) => format!("{path} will be moved, with the 1 file in it ({bytes} bytes)."),
        (true, files, true) => {
            format!("{path} will be moved, with the {files} files in it ({bytes} bytes).")
        }
        (true, 1, false) => {
            format!("{path} will be deleted, with the 1 file in it ({bytes} bytes).")
        }
        (true, files, false) => {
            format!("{path} will be deleted, with the {files} files in it ({bytes} bytes).")
        }
        (false, _, true) => format!("{path} ({bytes} bytes) will be moved."),
        (false, _, false) => format!("{path} ({bytes} bytes) will be deleted."),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    const DIR: &str = "/h/.local/share/compass";

    fn part(kind: PartKind) -> Part {
        Part {
            kind,
            path: Path::new(DIR).join(kind.file_name()),
            directory: kind == PartKind::Payloads,
            bytes: 100,
            files: if kind == PartKind::Payloads { 2 } else { 1 },
        }
    }

    fn survey(kinds: &[PartKind]) -> Survey {
        Survey {
            dir: PathBuf::from(DIR),
            parts: kinds.iter().copied().map(part).collect(),
        }
    }

    const HISTORY: &[PartKind] = &[PartKind::Database, PartKind::Payloads];

    #[test]
    fn a_readable_history_is_deleted() {
        assert_eq!(
            decide(&survey(HISTORY), &KeyState::Found([1; 32]), Some(&Ok(5))),
            Decision::Reset {
                disposal: Disposal::Delete,
                remove_key: false
            }
        );
    }

    #[test]
    fn a_history_the_key_does_not_open_is_kept_as_a_backup() {
        let backup = Decision::Reset {
            disposal: Disposal::Backup,
            remove_key: false,
        };
        assert_eq!(
            decide(
                &survey(HISTORY),
                &KeyState::Found([1; 32]),
                Some(&Err(ReadError::Unreadable))
            ),
            backup
        );
        assert_eq!(
            decide(&survey(HISTORY), &KeyState::Missing, None),
            backup,
            "a lost key is the same case"
        );
        assert_eq!(
            decide(&survey(HISTORY), &KeyState::Malformed(3), None),
            Decision::Reset {
                disposal: Disposal::Backup,
                remove_key: true
            }
        );
    }

    #[test]
    fn a_keyring_that_cannot_say_is_refused_not_guessed_at() {
        for key in [KeyState::Locked, KeyState::Unavailable("no bus".into())] {
            assert!(matches!(
                decide(&survey(HISTORY), &key, None),
                Decision::Refuse(_)
            ));
        }
        assert!(matches!(
            decide(
                &survey(HISTORY),
                &KeyState::Found([1; 32]),
                Some(&Err(ReadError::Busy))
            ),
            Decision::Refuse(_)
        ));
    }

    #[test]
    fn nothing_is_nothing_unless_the_keyring_entry_is_broken() {
        assert_eq!(
            decide(&survey(&[]), &KeyState::Missing, None),
            Decision::Nothing
        );
        assert_eq!(
            decide(&survey(&[]), &KeyState::Malformed(1), None),
            Decision::Reset {
                disposal: Disposal::Delete,
                remove_key: true
            }
        );
        assert_eq!(
            decide(&survey(&[PartKind::Payloads]), &KeyState::Locked, None),
            Decision::Reset {
                disposal: Disposal::Delete,
                remove_key: false
            },
            "orphaned payloads need no key to judge"
        );
    }

    #[test]
    fn the_listing_says_what_goes_where_and_how_to_go_ahead() {
        let deleted = files::plan_reset(&survey(HISTORY), Disposal::Delete, 1);
        let text = render_plan(&deleted, Some(5), false, false);
        assert!(text.starts_with("The clipboard history opens"), "{text}");
        assert!(text.contains("holds 5 entries"), "{text}");
        assert!(text.contains(&format!(
            "{DIR}/compass-clipboard.db (100 bytes) will be deleted."
        )));
        assert!(text.contains(&format!(
            "{DIR}/compass-clipboard will be deleted, with the 2 files in it (100 bytes)."
        )));
        assert!(text.ends_with("Run `compass clipboard reset --yes` to go ahead.\n"));
        assert!(!render_plan(&deleted, Some(5), false, true).contains("Nothing has been changed"));

        let moved = files::plan_reset(&survey(HISTORY), Disposal::Backup, 1_700_000_000);
        let text = render_plan(&moved, None, true, false);
        assert!(text.contains(&format!(
            "will be moved into {DIR}/compass-clipboard-backup-1700000000:"
        )));
        assert!(text.contains("is not a key Compass made"));
    }
}
