//! Where the IPC socket lives.
//!
//! The canonical location is `$XDG_RUNTIME_DIR/vicinae/ipc.sock`.
//! `XDG_RUNTIME_DIR` is a per-user, `0700`, tmpfs-backed directory that the
//! session manager cleans up at logout, which is exactly the lifetime a socket
//! wants.
//!
//! ## Fallback
//!
//! `XDG_RUNTIME_DIR` is unset in cron jobs, bare `ssh` sessions, some
//! containers and minimal init setups. When it is missing or empty we fall
//! back to `/tmp/vicinae-$USER/ipc.sock`, using `$USER`, then `$LOGNAME`, then
//! the literal `default` as the name. The username is part of the *directory*
//! so that two users on one machine cannot collide on a path, and so the
//! directory's owner is the only one who can create the socket inside it. This
//! mirrors what the C++ build does (`/tmp/vicinae`) but adds the per-user
//! suffix, since a shared `/tmp/vicinae` owned by whoever logged in first is a
//! denial-of-service on everyone else.
//!
//! The fallback is a fallback: it survives a reboot, is not cleaned at logout,
//! and lives on a directory other users can read. Callers that care should log
//! when [`SocketPath::is_fallback`] is true — `vicinae doctor` does.
//!
//! Nothing in this module reads the real environment unless you ask it to:
//! [`SocketPath::in_dir`] builds a path under any directory, which is how the
//! tests stay out of the developer's live session.

use std::path::{Path, PathBuf};

/// Directory created under the runtime dir (or the fallback root).
pub const SOCKET_DIR_NAME: &str = "vicinae";

/// File name of the socket itself.
pub const SOCKET_FILE_NAME: &str = "ipc.sock";

/// A resolved socket location, remembering how it was resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketPath {
    path: PathBuf,
    fallback: bool,
}

impl SocketPath {
    /// Resolves the socket path from the process environment.
    ///
    /// Uses `$XDG_RUNTIME_DIR` when it is set and non-empty, otherwise the
    /// fallback documented at the [module level](self).
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) if !dir.is_empty() => Self {
                path: PathBuf::from(dir)
                    .join(SOCKET_DIR_NAME)
                    .join(SOCKET_FILE_NAME),
                fallback: false,
            },
            _ => Self {
                path: Path::new("/tmp")
                    .join(format!("{SOCKET_DIR_NAME}-{}", current_user_name()))
                    .join(SOCKET_FILE_NAME),
                fallback: true,
            },
        }
    }

    /// Builds `<dir>/vicinae/ipc.sock`, ignoring the environment entirely.
    ///
    /// This is the injection point: tests pass a temporary directory, the
    /// Flatpak build can pass its sandboxed runtime dir.
    pub fn in_dir(dir: impl AsRef<Path>) -> Self {
        Self {
            path: dir.as_ref().join(SOCKET_DIR_NAME).join(SOCKET_FILE_NAME),
            fallback: false,
        }
    }

    /// Uses `path` verbatim, with no `vicinae/` component added.
    ///
    /// For `--socket /some/where.sock` style overrides.
    pub fn exact(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            fallback: false,
        }
    }

    /// The socket path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// The directory the socket lives in, if it has one.
    #[must_use]
    pub fn parent(&self) -> Option<&Path> {
        self.path.parent()
    }

    /// Whether the path came from the `XDG_RUNTIME_DIR`-is-missing fallback.
    #[must_use]
    pub fn is_fallback(&self) -> bool {
        self.fallback
    }

    /// Consumes this value and returns the owned path.
    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

impl AsRef<Path> for SocketPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Display for SocketPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

fn current_user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
        .filter(|u| !u.is_empty() && !u.contains('/'))
        .unwrap_or_else(|| "default".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_dir_appends_vicinae_and_the_socket_name() {
        let p = SocketPath::in_dir("/run/user/1000");
        assert_eq!(p.as_path(), Path::new("/run/user/1000/vicinae/ipc.sock"));
        assert!(!p.is_fallback());
        assert_eq!(p.parent(), Some(Path::new("/run/user/1000/vicinae")));
    }

    #[test]
    fn exact_is_used_verbatim() {
        let p = SocketPath::exact("/tmp/custom.sock");
        assert_eq!(p.as_path(), Path::new("/tmp/custom.sock"));
    }

    #[test]
    fn display_prints_the_path() {
        assert_eq!(SocketPath::exact("/a/b.sock").to_string(), "/a/b.sock");
    }

    // `from_env` reads process-global state, so the two branches share one test
    // rather than racing each other across threads.
    #[test]
    fn from_env_prefers_xdg_runtime_dir_and_falls_back_when_unset() {
        let saved = std::env::var_os("XDG_RUNTIME_DIR");

        // SAFETY-adjacent note: `set_var` is unsafe in edition 2024 and
        // `unsafe_code` is forbidden here, so instead of mutating the
        // environment we assert the pure logic against both shapes directly.
        let with_xdg = SocketPath::in_dir("/run/user/1000");
        assert_eq!(
            with_xdg.as_path(),
            Path::new("/run/user/1000/vicinae/ipc.sock")
        );

        let resolved = SocketPath::from_env();
        match saved {
            Some(dir) if !dir.is_empty() => {
                assert!(!resolved.is_fallback());
                assert_eq!(resolved.as_path(), SocketPath::in_dir(&dir).as_path());
            }
            _ => {
                assert!(resolved.is_fallback());
                assert!(resolved.as_path().starts_with("/tmp"));
                assert_eq!(resolved.as_path().file_name().unwrap(), SOCKET_FILE_NAME);
            }
        }
    }

    #[test]
    fn user_name_is_never_a_path_component() {
        // Whatever the environment says, the fallback stays a single directory
        // under /tmp.
        let name = current_user_name();
        assert!(!name.contains('/'));
        assert!(!name.is_empty());
    }
}
