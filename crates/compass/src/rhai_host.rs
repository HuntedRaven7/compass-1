//! The services a Rhai script's granted capabilities reach, from the engine.
//!
//! The same ones an extension reaches, so a script and an extension doing the
//! same thing do it the same way:
//!
//! - the clipboard: the GNOME Shell extension's, or data-control on a wlroots
//!   compositor ([`crate::extension_runner`]'s `ShellClipboard`);
//! - opening: the default application for the target, resolved from the
//!   engine's application index and the MIME associations
//!   ([`crate::extension_apps::EngineApps`]);
//! - storage: Compass's encrypted extension database, in a namespace of the
//!   script's own ([`storage_namespace`]); the same keyring key the extensions'
//!   storage opens with, fetched on first use;
//! - notifications, over `notify-rust`;
//! - programs, on the host (through `flatpak-spawn --host` inside a Flatpak,
//!   [`compass_platform_linux::host_command`]), killed at the script's
//!   timeout, with their output read up to [`OUTPUT_LIMIT`] bytes.
//!
//! Every method is called on the script's blocking thread, so blocking into
//! the runtime is allowed here.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use compass_extension_api::CapabilityGrant;
use compass_script::{HostError, ProcessOutcome, ScriptHost};
use compass_sqlcipher_sys::rusqlite::Connection;
use compass_worker_host::application_service::Apps as _;
use compass_worker_host::clipboard_service::{Clipboard as _, Content, CopyOptions};
use tokio::io::{AsyncRead, AsyncReadExt as _};

use crate::extension_runner::{ShellClipboard, Storage};

/// Where the GNOME Shell extension's client lands once the session bus has
/// answered; shared with the engine state, which fills it.
pub type ShellSlot = Arc<RwLock<Option<Arc<compass_shell::ShellClient>>>>;

/// The local-storage namespace a script's values live in: out of reach of
/// any extension's own `<id>:data`, whatever its directory is called.
#[must_use]
pub fn storage_namespace(script: &str) -> String {
    format!("compass.rhai:{script}")
}

/// How much of each of a program's output streams a script receives. Kept
/// well under the script's own string limit; the rest is read and dropped so
/// a chatty program still finishes rather than blocking on a full pipe.
pub const OUTPUT_LIMIT: u64 = 256 * 1024;

/// The engine's [`ScriptHost`].
pub struct EngineHost {
    shell: ShellSlot,
    handle: Option<tokio::runtime::Handle>,
    apps: Option<crate::extension_apps::EngineApps>,
    /// Where the storage database is, once asked; `None` inside means there
    /// is no keyring to open it with.
    storage: OnceLock<Option<Storage>>,
    /// Compass's data directory, whose keyring-keyed database scripts share
    /// with extensions.
    data_dir: Option<PathBuf>,
    connection: Mutex<Option<Connection>>,
}

impl std::fmt::Debug for EngineHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineHost")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl EngineHost {
    /// A host reaching the clipboard through `shell` (once it is filled),
    /// opening with `apps`, keeping storage in `data_dir`'s database.
    #[must_use]
    pub fn new(
        shell: ShellSlot,
        apps: Option<crate::extension_apps::EngineApps>,
        data_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            shell,
            handle: tokio::runtime::Handle::try_current().ok(),
            apps,
            storage: OnceLock::new(),
            data_dir,
            connection: Mutex::new(None),
        }
    }

    /// A host whose storage is `storage`, without asking the keyring; for
    /// tests.
    #[must_use]
    pub fn with_storage(storage: Storage) -> Self {
        let host = Self::new(ShellSlot::default(), None, None);
        let _ = host.storage.set(Some(storage));
        host
    }

    fn clipboard(&self) -> Result<ShellClipboard, HostError> {
        let shell = self
            .shell
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        ShellClipboard::available(shell, self.handle.clone())
            .ok_or_else(|| HostError::new("Compass cannot reach the clipboard in this session"))
    }

    fn storage(&self) -> Option<&Storage> {
        self.storage
            .get_or_init(|| {
                let (Some(handle), Some(dir)) = (&self.handle, &self.data_dir) else {
                    return None;
                };
                handle.block_on(crate::serve::extension_storage(dir))
            })
            .as_ref()
    }

    /// Runs `f` against the script's own namespace.
    fn with_store<T>(
        &self,
        grant: &CapabilityGrant,
        f: impl FnOnce(&compass_local_storage::Scoped<'_>) -> Result<T, compass_local_storage::Error>,
    ) -> Result<T, HostError> {
        let storage = self.storage().ok_or_else(|| {
            HostError::new("Storage needs the login keyring, and Compass cannot reach it")
        })?;
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if connection.is_none() {
            *connection = crate::extension_runner::open_storage(storage);
        }
        let db = connection
            .as_ref()
            .ok_or_else(|| HostError::new("Compass could not open its storage"))?;
        let local = compass_local_storage::LocalStorage::new(db);
        let scoped = local.scoped(&storage_namespace(grant.extension().as_str()));
        f(&scoped).map_err(|err| HostError::new(format!("Storage failed: {err}")))
    }
}

impl ScriptHost for EngineHost {
    fn clipboard_write(&self, _grant: CapabilityGrant, text: &str) -> Result<(), HostError> {
        self.clipboard()?
            .copy(Content::Text(text.to_owned()), CopyOptions::default());
        Ok(())
    }

    fn clipboard_read(&self, _grant: CapabilityGrant) -> Result<Option<String>, HostError> {
        let content = self.clipboard()?.read();
        Ok((!content.text.is_empty()).then_some(content.text))
    }

    fn clipboard_paste(&self, _grant: CapabilityGrant, text: &str) -> Result<(), HostError> {
        self.clipboard()?.paste(Content::Text(text.to_owned()));
        Ok(())
    }

    fn open(&self, _grant: CapabilityGrant, target: &str) -> Result<(), HostError> {
        let apps = self
            .apps
            .as_ref()
            .ok_or_else(|| HostError::new("Compass cannot open anything from here"))?;
        let opener = apps
            .default_opener(target)
            .ok_or_else(|| HostError::new(format!("No application opens {target}")))?;
        apps.launch(&opener, target);
        Ok(())
    }

    fn storage_get(&self, grant: CapabilityGrant, key: &str) -> Result<Option<String>, HostError> {
        self.with_store(&grant, |store| {
            store.get(key).map(|value| value.map(|value| value.text))
        })
    }

    fn storage_keys(&self, grant: CapabilityGrant) -> Result<Vec<String>, HostError> {
        let mut keys = self.with_store(&grant, |store| {
            store
                .list()
                .map(|values| values.into_iter().map(|(key, _)| key).collect::<Vec<_>>())
        })?;
        keys.sort();
        Ok(keys)
    }

    fn storage_set(&self, grant: CapabilityGrant, key: &str, value: &str) -> Result<(), HostError> {
        // The JSON text itself, as a string: `Value::from_json` keeps only
        // scalars, and a script stores arrays and maps.
        let value = compass_local_storage::Value {
            text: value.to_owned(),
            kind: compass_local_storage::ValueType::String,
        };
        self.with_store(&grant, |store| store.set(key, &value))
    }

    fn storage_remove(&self, grant: CapabilityGrant, key: &str) -> Result<(), HostError> {
        self.with_store(&grant, |store| store.remove(key).map(drop))
    }

    fn notify(&self, grant: CapabilityGrant, title: &str, body: &str) -> Result<(), HostError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| HostError::new("Compass cannot show notifications from here"))?;
        handle
            .block_on(
                notify_rust::Notification::new()
                    .appname("Compass")
                    .summary(title)
                    .body(body)
                    .show_async(),
            )
            .map(drop)
            .map_err(|err| {
                tracing::info!(script = %grant.extension(), %err, "notification not shown");
                HostError::new("The notification could not be shown")
            })
    }

    fn run(
        &self,
        grant: CapabilityGrant,
        program: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<Option<ProcessOutcome>, HostError> {
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| HostError::new("Compass cannot run programs from here"))?;
        tracing::debug!(script = %grant.extension(), program, ?args, ?timeout, "running a program for a script");
        let mut command =
            tokio::process::Command::from(compass_platform_linux::host_command(program));
        command.args(args);
        handle.block_on(run_bounded(command, timeout))
    }
}

/// Runs `command` to completion or until `timeout`, whichever is first.
/// `None` when the program does not exist.
///
/// Inside a Flatpak the program is `flatpak-spawn`, which exists, so a
/// missing host program shows as a failed run rather than as `None`; and
/// killing `flatpak-spawn` does not reach the host process, which is left to
/// finish on its own while the script carries on.
async fn run_bounded(
    mut command: tokio::process::Command,
    timeout: Duration,
) -> Result<Option<ProcessOutcome>, HostError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(HostError::new(format!(
                "The program could not be started: {err}"
            )));
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let finished = tokio::time::timeout(timeout, async {
        tokio::join!(capture(stdout), capture(stderr), child.wait())
    })
    .await;
    match finished {
        Ok((stdout, stderr, status)) => {
            let status = status.map_err(|err| {
                HostError::new(format!("The program could not be waited for: {err}"))
            })?;
            Ok(Some(ProcessOutcome::Exited {
                code: status.code(),
                stdout,
                stderr,
            }))
        }
        Err(_) => {
            // Reaped here rather than left to `kill_on_drop`, so no zombie
            // outlives the call.
            if let Err(err) = child.kill().await {
                tracing::debug!(%err, "a timed-out program could not be killed");
            }
            Ok(Some(ProcessOutcome::TimedOut))
        }
    }
}

/// The first [`OUTPUT_LIMIT`] bytes of `pipe`, as text; the rest is drained.
async fn capture(pipe: Option<impl AsyncRead + Unpin>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut bytes = Vec::new();
    // A read error leaves what was read so far, which is all a script could
    // use anyway.
    let _ = (&mut pipe).take(OUTPUT_LIMIT).read_to_end(&mut bytes).await;
    let _ = tokio::io::copy(&mut pipe, &mut tokio::io::sink()).await;
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_extension_api::{Capability, CapabilityRegistry, ExtensionId};

    fn grant(
        registry: &mut CapabilityRegistry,
        id: &ExtensionId,
        cap: &Capability,
    ) -> CapabilityGrant {
        registry.declare(id, [cap.clone()]);
        registry.grant(id, cap).unwrap();
        registry.check(id, cap).unwrap()
    }

    /// A host with a runtime to block into, called from outside it as the
    /// script's blocking thread is.
    fn process_host() -> (tokio::runtime::Runtime, EngineHost, CapabilityGrant) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let host = {
            let _entered = runtime.enter();
            EngineHost::new(ShellSlot::default(), None, None)
        };
        let mut registry = CapabilityRegistry::new();
        let id = ExtensionId::new("script.health");
        let grant = grant(&mut registry, &id, &Capability::PROCESS_EXECUTE);
        (runtime, host, grant)
    }

    fn args(args: &[&str]) -> Vec<String> {
        args.iter().map(|&arg| arg.to_owned()).collect()
    }

    #[test]
    fn a_programs_exit_code_and_output_come_back() {
        let (_runtime, host, grant) = process_host();
        let outcome = host
            .run(
                grant,
                "sh",
                &args(&["-c", "printf out; printf err >&2; exit 3"]),
                Duration::from_secs(5),
            )
            .unwrap();
        assert_eq!(
            outcome,
            Some(ProcessOutcome::Exited {
                code: Some(3),
                stdout: "out".to_owned(),
                stderr: "err".to_owned(),
            })
        );
    }

    #[test]
    fn a_program_that_is_not_installed_is_absent_rather_than_an_error() {
        let (_runtime, host, grant) = process_host();
        let outcome = host
            .run(
                grant,
                "compass-test-no-such-program",
                &[],
                Duration::from_secs(5),
            )
            .unwrap();
        assert_eq!(outcome, None);
    }

    #[test]
    fn a_program_still_running_at_its_timeout_is_killed_and_says_so() {
        let (_runtime, host, grant) = process_host();
        let started = std::time::Instant::now();
        let outcome = host
            .run(grant, "sleep", &args(&["30"]), Duration::from_millis(200))
            .unwrap();
        assert_eq!(outcome, Some(ProcessOutcome::TimedOut));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn output_past_the_limit_is_dropped_and_the_program_still_finishes() {
        let (_runtime, host, grant) = process_host();
        let outcome = host
            .run(
                grant,
                "sh",
                &args(&["-c", "head -c 1000000 /dev/zero | tr '\\0' x"]),
                Duration::from_secs(10),
            )
            .unwrap();
        let Some(ProcessOutcome::Exited { code, stdout, .. }) = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(code, Some(0));
        assert_eq!(u64::try_from(stdout.len()).unwrap(), OUTPUT_LIMIT);
    }

    #[test]
    fn storage_keeps_json_per_script_in_the_encrypted_database() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage {
            path: dir.path().join("storage.db"),
            key: [7; compass_crypto::KEY_SIZE],
        };
        let host = EngineHost::with_storage(storage.clone());
        let mut registry = CapabilityRegistry::new();
        let notes = ExtensionId::new("script.notes");
        let other = ExtensionId::new("script.other");
        let write = |r: &mut CapabilityRegistry, id| grant(r, id, &Capability::STORAGE_WRITE);
        let read = |r: &mut CapabilityRegistry, id| grant(r, id, &Capability::STORAGE_READ);

        host.storage_set(write(&mut registry, &notes), "notes", r#"["a","b"]"#)
            .unwrap();
        host.storage_set(write(&mut registry, &notes), "count", "2")
            .unwrap();
        assert_eq!(
            host.storage_get(read(&mut registry, &notes), "notes")
                .unwrap(),
            Some(r#"["a","b"]"#.to_owned())
        );
        assert_eq!(
            host.storage_keys(read(&mut registry, &notes)).unwrap(),
            ["count", "notes"]
        );
        assert_eq!(
            host.storage_get(read(&mut registry, &other), "notes")
                .unwrap(),
            None,
            "another script's store is its own"
        );
        host.storage_remove(write(&mut registry, &notes), "count")
            .unwrap();

        // A second host over the same database: it survives a restart.
        let again = EngineHost::with_storage(storage);
        assert_eq!(
            again.storage_keys(read(&mut registry, &notes)).unwrap(),
            ["notes"]
        );
    }

    #[test]
    fn without_services_a_call_fails_with_a_sentence_rather_than_doing_nothing() {
        let host = EngineHost::new(ShellSlot::default(), None, None);
        let mut registry = CapabilityRegistry::new();
        let id = ExtensionId::new("script.x");
        let error = host
            .open(
                grant(&mut registry, &id, &Capability::APPLICATION_OPEN),
                "https://example.org",
            )
            .unwrap_err();
        assert!(error.0.contains("cannot open"), "{error}");
        let error = host
            .storage_get(grant(&mut registry, &id, &Capability::STORAGE_READ), "k")
            .unwrap_err();
        assert!(error.0.contains("keyring"), "{error}");
    }
}
