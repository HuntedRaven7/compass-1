//! What a script can reach outside itself, and the one type that performs it.
//!
//! Every method takes a [`CapabilityGrant`] by value. A grant can only be
//! obtained from [`CapabilityRegistry::check`](compass_extension_api::CapabilityRegistry::check),
//! so "was this checked?" is answered by the type system: this crate cannot
//! call the host for a capability it was not granted, because it cannot make
//! the argument. The grant also names the extension, which is how a host
//! scopes storage per script.
//!
//! Methods are synchronous and run on the script's blocking thread. A host
//! whose implementation is async blocks into its runtime (for example with
//! `tokio::runtime::Handle::block_on`); the call is still covered by the
//! script's wall-clock timeout as far as the caller is concerned.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use compass_extension_api::{CapabilityGrant, ExtensionId};

/// A host function failed. The message reaches the script as a catchable
/// error, so it should say what went wrong, not how.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct HostError(pub String);

impl HostError {
    /// Wraps a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// What became of a program a script ran with `process.execute`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutcome {
    /// It finished on its own.
    Exited {
        /// The exit code; `None` when a signal ended it.
        code: Option<i32>,
        /// Standard output, as text, cut off at the host's limit.
        stdout: String,
        /// Standard error, as text, cut off at the host's limit.
        stderr: String,
    },
    /// It was still running when its time ran out, and was killed.
    TimedOut,
}

/// The services a script can be granted, one method per capability-backed
/// operation. See the module documentation for the calling convention.
pub trait ScriptHost: Send + Sync + std::fmt::Debug {
    /// `clipboard.write`: replace the clipboard's text.
    ///
    /// # Errors
    ///
    /// When the host cannot write the clipboard.
    fn clipboard_write(&self, grant: CapabilityGrant, text: &str) -> Result<(), HostError>;

    /// `clipboard.read`: the clipboard's current text, if it holds text.
    ///
    /// # Errors
    ///
    /// When the host cannot read the clipboard.
    fn clipboard_read(&self, grant: CapabilityGrant) -> Result<Option<String>, HostError>;

    /// `clipboard.paste`: paste text into the focused surface.
    ///
    /// # Errors
    ///
    /// When the host cannot paste.
    fn clipboard_paste(&self, grant: CapabilityGrant, text: &str) -> Result<(), HostError>;

    /// `application.open`: open a URL or path with its default handler.
    ///
    /// # Errors
    ///
    /// When nothing can open `target`.
    fn open(&self, grant: CapabilityGrant, target: &str) -> Result<(), HostError>;

    /// `storage.read`: a value from the script's own store, as JSON text.
    ///
    /// # Errors
    ///
    /// When the store cannot be read.
    fn storage_get(&self, grant: CapabilityGrant, key: &str) -> Result<Option<String>, HostError>;

    /// `storage.read`: every key in the script's own store, sorted.
    ///
    /// # Errors
    ///
    /// When the store cannot be read.
    fn storage_keys(&self, grant: CapabilityGrant) -> Result<Vec<String>, HostError>;

    /// `storage.write`: set a value, as JSON text, in the script's own store.
    ///
    /// # Errors
    ///
    /// When the store cannot be written.
    fn storage_set(&self, grant: CapabilityGrant, key: &str, value: &str) -> Result<(), HostError>;

    /// `storage.write`: remove a key from the script's own store.
    ///
    /// # Errors
    ///
    /// When the store cannot be written.
    fn storage_remove(&self, grant: CapabilityGrant, key: &str) -> Result<(), HostError>;

    /// `notification.send`: post a desktop notification.
    ///
    /// # Errors
    ///
    /// When the notification cannot be posted.
    fn notify(&self, grant: CapabilityGrant, title: &str, body: &str) -> Result<(), HostError>;

    /// `process.execute`: run `program` with `args`, without a shell and with
    /// standard input closed, and kill it if it has not finished within
    /// `timeout`. `Ok(None)` when there is no such program, so a script can
    /// tell "not installed" from "failed".
    ///
    /// # Errors
    ///
    /// When the program exists but cannot be started.
    fn run(
        &self,
        grant: CapabilityGrant,
        program: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<Option<ProcessOutcome>, HostError>;
}

/// One thing a [`MemoryHost`] was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCall {
    /// `clipboard_write`.
    Copy(String),
    /// `clipboard_paste`.
    Paste(String),
    /// `open`.
    Open(String),
    /// `notify`.
    Notify {
        /// Title.
        title: String,
        /// Body.
        body: String,
    },
    /// `run`.
    Run {
        /// The program, then its arguments.
        argv: Vec<String>,
        /// How long the program was given.
        timeout: Duration,
    },
}

#[derive(Debug, Default)]
struct MemoryState {
    clipboard: Option<String>,
    storage: BTreeMap<ExtensionId, BTreeMap<String, String>>,
    calls: Vec<HostCall>,
    programs: BTreeMap<Vec<String>, ProcessOutcome>,
}

/// A host that keeps everything in memory and records what it was asked to
/// do. For tests, for trying a script from the command line, and as the
/// reference for what each method means.
#[derive(Debug, Default)]
pub struct MemoryHost {
    state: Mutex<MemoryState>,
}

impl MemoryHost {
    /// An empty host.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn with<T>(&self, f: impl FnOnce(&mut MemoryState) -> T) -> T {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut state)
    }

    /// Everything the host was asked to do, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<HostCall> {
        self.with(|s| s.calls.clone())
    }

    /// The clipboard's text.
    #[must_use]
    pub fn clipboard(&self) -> Option<String> {
        self.with(|s| s.clipboard.clone())
    }

    /// Sets the clipboard's text, as if the user had copied something.
    pub fn set_clipboard(&self, text: impl Into<String>) {
        let text = text.into();
        self.with(|s| s.clipboard = Some(text));
    }

    /// One script's store.
    #[must_use]
    pub fn storage(&self, extension: &ExtensionId) -> BTreeMap<String, String> {
        self.with(|s| s.storage.get(extension).cloned().unwrap_or_default())
    }

    /// Makes `argv` (the program, then its arguments) answer with `outcome`.
    /// Any command line not set this way runs a program that is not
    /// installed.
    pub fn set_program(&self, argv: &[&str], outcome: ProcessOutcome) {
        let argv = argv.iter().map(|&arg| arg.to_owned()).collect();
        self.with(|s| s.programs.insert(argv, outcome));
    }
}

impl ScriptHost for MemoryHost {
    fn clipboard_write(&self, _grant: CapabilityGrant, text: &str) -> Result<(), HostError> {
        self.with(|s| {
            s.clipboard = Some(text.to_owned());
            s.calls.push(HostCall::Copy(text.to_owned()));
        });
        Ok(())
    }

    fn clipboard_read(&self, _grant: CapabilityGrant) -> Result<Option<String>, HostError> {
        Ok(self.clipboard())
    }

    fn clipboard_paste(&self, _grant: CapabilityGrant, text: &str) -> Result<(), HostError> {
        self.with(|s| s.calls.push(HostCall::Paste(text.to_owned())));
        Ok(())
    }

    fn open(&self, _grant: CapabilityGrant, target: &str) -> Result<(), HostError> {
        self.with(|s| s.calls.push(HostCall::Open(target.to_owned())));
        Ok(())
    }

    fn storage_get(&self, grant: CapabilityGrant, key: &str) -> Result<Option<String>, HostError> {
        Ok(self.with(|s| {
            s.storage
                .get(grant.extension())
                .and_then(|store| store.get(key).cloned())
        }))
    }

    fn storage_keys(&self, grant: CapabilityGrant) -> Result<Vec<String>, HostError> {
        Ok(self.with(|s| {
            s.storage
                .get(grant.extension())
                .map(|store| store.keys().cloned().collect())
                .unwrap_or_default()
        }))
    }

    fn storage_set(&self, grant: CapabilityGrant, key: &str, value: &str) -> Result<(), HostError> {
        self.with(|s| {
            s.storage
                .entry(grant.extension().clone())
                .or_default()
                .insert(key.to_owned(), value.to_owned());
        });
        Ok(())
    }

    fn storage_remove(&self, grant: CapabilityGrant, key: &str) -> Result<(), HostError> {
        self.with(|s| {
            if let Some(store) = s.storage.get_mut(grant.extension()) {
                store.remove(key);
            }
        });
        Ok(())
    }

    fn notify(&self, _grant: CapabilityGrant, title: &str, body: &str) -> Result<(), HostError> {
        self.with(|s| {
            s.calls.push(HostCall::Notify {
                title: title.to_owned(),
                body: body.to_owned(),
            });
        });
        Ok(())
    }

    fn run(
        &self,
        _grant: CapabilityGrant,
        program: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<Option<ProcessOutcome>, HostError> {
        let argv: Vec<String> = std::iter::once(program.to_owned())
            .chain(args.iter().cloned())
            .collect();
        Ok(self.with(|s| {
            s.calls.push(HostCall::Run {
                argv: argv.clone(),
                timeout,
            });
            s.programs.get(&argv).cloned()
        }))
    }
}
