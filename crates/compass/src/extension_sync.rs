//! Installing the store extensions `compass.json` declares
//! (`extensions.installed`), when the engine starts and each time the file
//! changes.
//!
//! What an entry means, and why nothing is ever uninstalled, is
//! `compass_core::declared_extensions`; the install itself is the store's
//! own ([`crate::stores::Stores::reconcile`]). One task does every pass, so
//! two changes in quick succession never install the same extension twice
//! at once, and each pass reads the file afresh, so a pass that starts late
//! still installs what the file says now.
//!
//! A login usually starts the engine before the network is up, so an install
//! that failed is tried again after [`RETRY_DELAYS`], the last of them
//! repeating, until it succeeds or the file changes. An entry that cannot be
//! installed at all is reported once per pass and not retried. Failures go to
//! the log; `compass doctor` and `compass ext sync` report them too.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use compass_core::Config;
use serde_json::{Value, json};
use tokio::sync::{Notify, RwLock};

use crate::serve::EngineState;
use crate::stores::{Outcome, Reconciled, Stores};
use crate::{EXIT_FAILURE, EXIT_OK};

/// How long to wait before trying failed installs again, one delay per
/// consecutive failed pass; the last repeats.
pub const RETRY_DELAYS: [Duration; 4] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
    Duration::from_secs(60 * 60),
];

/// Asks the installer for another pass.
#[derive(Debug, Default)]
pub struct Trigger {
    wake: Notify,
}

impl Trigger {
    /// Runs a pass soon. Requests made while a pass runs fold into one more.
    pub fn request(&self) {
        self.wake.notify_one();
    }
}

/// The engine's installer: a pass now, then one per [`Trigger::request`] or
/// retry. Runs until the engine stops.
pub async fn run(state: Arc<RwLock<EngineState>>) {
    let (stores, trigger) = state.read().await.extension_sync();
    let mut failed_passes = 0;
    loop {
        let retry = match declared().await {
            Some(entries) if !entries.is_empty() => {
                let reconciled = stores.reconcile(&entries, false).await;
                log(&reconciled);
                if reconciled
                    .iter()
                    .any(|item| matches!(item.outcome, Outcome::Installed { .. }))
                {
                    state.write().await.rescan_extensions();
                }
                reconciled
                    .iter()
                    .any(|item| matches!(item.outcome, Outcome::Failed(_)))
            }
            _ => false,
        };
        let delay = if retry {
            let delay = RETRY_DELAYS[failed_passes.min(RETRY_DELAYS.len() - 1)];
            failed_passes += 1;
            tracing::info!(
                seconds = delay.as_secs(),
                "trying the declared extensions that failed to install again later"
            );
            Some(delay)
        } else {
            failed_passes = 0;
            None
        };
        tokio::select! {
            () = trigger.wake.notified() => failed_passes = 0,
            () = tokio::time::sleep(delay.unwrap_or_default()), if delay.is_some() => {}
        }
    }
}

/// `extensions.installed` as the file says now; `None` when it cannot be
/// read, which installs nothing rather than guessing.
async fn declared() -> Option<Vec<String>> {
    match tokio::task::spawn_blocking(Config::load).await {
        Ok(Ok(config)) => Some(config.extensions().installed().to_vec()),
        Ok(Err(error)) => {
            tracing::warn!(%error,
                "compass.json cannot be read, so the extensions it declares are not installed");
            None
        }
        Err(_) => None,
    }
}

fn log(reconciled: &[Reconciled]) {
    for Reconciled { entry, outcome } in reconciled {
        match outcome {
            Outcome::Present { .. } | Outcome::WouldInstall { .. } => {}
            Outcome::Installed { id, title } => {
                tracing::info!(%entry, %id, %title, "installed an extension compass.json declares");
            }
            Outcome::Unresolvable(reason) => {
                tracing::warn!(%entry, %reason,
                    "an extension compass.json declares cannot be installed");
            }
            Outcome::Failed(reason) => {
                tracing::warn!(%entry, %reason,
                    "an extension compass.json declares failed to install");
            }
        }
    }
}

/// `compass ext sync`: one pass in this process, reported.
///
/// # Errors
///
/// When `compass.json` cannot be read, or the report cannot be written.
pub async fn sync_now(dry_run: bool, as_json: bool) -> Result<ExitCode> {
    let config = tokio::task::spawn_blocking(Config::load)
        .await
        .context("reading compass.json")?
        .context("compass.json cannot be read, so the extensions it declares are not known")?;
    let entries = config.extensions().installed().to_vec();
    let reconciled = Stores::from_environment()
        .reconcile(&entries, dry_run)
        .await;
    if as_json {
        let report: Vec<Value> = reconciled.iter().map(json_entry).collect();
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", human_report(&reconciled));
    }
    let failed = reconciled.iter().any(|item| item.outcome.is_problem());
    Ok(ExitCode::from(if failed { EXIT_FAILURE } else { EXIT_OK }))
}

fn json_entry(Reconciled { entry, outcome }: &Reconciled) -> Value {
    match outcome {
        Outcome::Present { id } => json!({"entry": entry, "status": "present", "id": id}),
        Outcome::Installed { id, title } => {
            json!({"entry": entry, "status": "installed", "id": id, "title": title})
        }
        Outcome::WouldInstall { id } => {
            json!({"entry": entry, "status": "would_install", "id": id})
        }
        Outcome::Unresolvable(reason) => {
            json!({"entry": entry, "status": "unresolvable", "reason": reason})
        }
        Outcome::Failed(reason) => json!({"entry": entry, "status": "failed", "reason": reason}),
    }
}

fn human_report(reconciled: &[Reconciled]) -> String {
    if reconciled.is_empty() {
        return "compass.json declares no extensions to install.\n".to_owned();
    }
    let mut out = String::new();
    for Reconciled { entry, outcome } in reconciled {
        let line = match outcome {
            Outcome::Present { id } => format!("{entry} is installed as {id}."),
            Outcome::Installed { id, title } => format!("{entry} was installed as {id} ({title})."),
            Outcome::WouldInstall { id } => format!("{entry} would be installed as {id}."),
            Outcome::Unresolvable(reason) => format!("{entry} cannot be installed: {reason}"),
            Outcome::Failed(reason) => format!("{entry} failed to install: {reason}"),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Reconciled> {
        let item = |entry: &str, outcome| Reconciled {
            entry: entry.to_owned(),
            outcome,
        };
        vec![
            item(
                "vicinae:zoe/clock",
                Outcome::Present {
                    id: "store.vicinae.clock".into(),
                },
            ),
            item(
                "raycast:raycast/github",
                Outcome::Installed {
                    id: "store.raycast.github".into(),
                    title: "GitHub".into(),
                },
            ),
            item(
                "store.raycast.hn",
                Outcome::Failed("Failed to fetch extensions: offline".into()),
            ),
            item("brew:x/y", Outcome::Unresolvable("Not a store.".into())),
        ]
    }

    #[test]
    fn the_report_says_what_became_of_each_entry() {
        assert_eq!(
            human_report(&sample()),
            "vicinae:zoe/clock is installed as store.vicinae.clock.\n\
             raycast:raycast/github was installed as store.raycast.github (GitHub).\n\
             store.raycast.hn failed to install: Failed to fetch extensions: offline\n\
             brew:x/y cannot be installed: Not a store.\n"
        );
        assert_eq!(
            human_report(&[]),
            "compass.json declares no extensions to install.\n"
        );
        let statuses: Vec<Value> = sample()
            .iter()
            .map(|item| json_entry(item)["status"].clone())
            .collect();
        assert_eq!(statuses, ["present", "installed", "failed", "unresolvable"]);
    }
}
