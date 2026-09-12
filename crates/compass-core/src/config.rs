//! The `vicinae.json` user configuration.
//!
//! The schema is the one in `docs/rust-engine/PLAN.md` §5.2:
//!
//! ```json
//! {
//!   "launcher": {
//!     "hotkey": "super+space",
//!     "close_on_focus_loss": false,
//!     "max_results": 50
//!   },
//!   "extensions": {
//!     "auto_update": true,
//!     "installed": ["com.example.clock"]
//!   }
//! }
//! ```
//!
//! Two properties drive the design:
//!
//! * **Every field is optional and has a documented default** ([`DEFAULT_HOTKEY`],
//!   [`DEFAULT_CLOSE_ON_FOCUS_LOSS`], [`DEFAULT_MAX_RESULTS`], [`DEFAULT_AUTO_UPDATE`]). An empty
//!   file, an empty object, or a missing file all yield a working [`Config`].
//! * **Unknown fields survive a round trip.** A config written by a newer build must not be
//!   silently truncated when an older build reads and rewrites it, so every struct captures the
//!   keys it does not know in a `serde(flatten)` map and writes them back out. This is why the
//!   getters take the "absent means default" decision at read time rather than at parse time:
//!   materialising defaults into the file would rewrite a user's config with values they never
//!   chose.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default global toggle hotkey. Matches the Phase 1 gate in the plan (Super+Space on GNOME).
pub const DEFAULT_HOTKEY: &str = "super+space";

/// Default for `launcher.close_on_focus_loss`; mirrors the C++ `Config::closeOnFocusLoss`.
pub const DEFAULT_CLOSE_ON_FOCUS_LOSS: bool = false;

/// Default for `launcher.max_results`.
pub const DEFAULT_MAX_RESULTS: usize = 50;

/// Default for `extensions.auto_update`.
pub const DEFAULT_AUTO_UPDATE: bool = true;

/// Path of the config file relative to `$XDG_CONFIG_HOME`.
pub const CONFIG_RELATIVE_PATH: &str = "vicinae/vicinae.json";

/// Everything that can go wrong loading or saving a [`Config`].
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists but could not be read.
    #[error("could not read the configuration at {path}")]
    Read {
        /// The offending path.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The file could not be written.
    #[error("could not write the configuration to {path}")]
    Write {
        /// The offending path.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The file is not valid JSON, or is JSON of the wrong shape.
    ///
    /// The message names the problem and where it is, e.g.
    /// `invalid configuration at /home/u/.config/vicinae/vicinae.json: line 3 column 18: expected
    /// value`.
    #[error("invalid configuration at {path}: line {line} column {column}: {message}")]
    Parse {
        /// The offending path.
        path: PathBuf,
        /// 1-based line of the problem, `0` when the parser could not place it.
        line: usize,
        /// 1-based column of the problem, `0` when the parser could not place it.
        column: usize,
        /// What the JSON parser objected to.
        message: String,
        /// The underlying serde error.
        #[source]
        source: serde_json::Error,
    },

    /// `$XDG_CONFIG_HOME` (and its `$HOME` fallback) could not be resolved.
    #[error("could not determine the user configuration directory")]
    NoConfigDir,
}

/// The `launcher` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LauncherConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hotkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    close_on_focus_loss: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_results: Option<usize>,

    /// Keys this build does not know about, preserved verbatim.
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

impl LauncherConfig {
    /// The configured hotkey, or [`DEFAULT_HOTKEY`].
    #[must_use]
    pub fn hotkey(&self) -> &str {
        self.hotkey.as_deref().unwrap_or(DEFAULT_HOTKEY)
    }

    /// Whether the launcher closes when it loses focus. Defaults to
    /// [`DEFAULT_CLOSE_ON_FOCUS_LOSS`].
    #[must_use]
    pub fn close_on_focus_loss(&self) -> bool {
        self.close_on_focus_loss
            .unwrap_or(DEFAULT_CLOSE_ON_FOCUS_LOSS)
    }

    /// How many results the launcher shows. Defaults to [`DEFAULT_MAX_RESULTS`].
    ///
    /// A configured `0` is honoured as "show nothing"; only an absent key falls back.
    #[must_use]
    pub fn max_results(&self) -> usize {
        self.max_results.unwrap_or(DEFAULT_MAX_RESULTS)
    }

    /// Sets `launcher.hotkey`. `None` removes the key, restoring the default.
    pub fn set_hotkey(&mut self, hotkey: Option<String>) -> &mut Self {
        self.hotkey = hotkey;
        self
    }

    /// Sets `launcher.close_on_focus_loss`. `None` removes the key.
    pub fn set_close_on_focus_loss(&mut self, value: Option<bool>) -> &mut Self {
        self.close_on_focus_loss = value;
        self
    }

    /// Sets `launcher.max_results`. `None` removes the key.
    pub fn set_max_results(&mut self, value: Option<usize>) -> &mut Self {
        self.max_results = value;
        self
    }

    /// Keys present in the file that this build does not understand.
    #[must_use]
    pub fn unknown_fields(&self) -> &BTreeMap<String, Value> {
        &self.unknown
    }

    /// Whether the section carries nothing at all, known or unknown.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hotkey.is_none()
            && self.close_on_focus_loss.is_none()
            && self.max_results.is_none()
            && self.unknown.is_empty()
    }
}

/// The `extensions` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtensionsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_update: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installed: Option<Vec<String>>,

    /// Keys this build does not know about, preserved verbatim.
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

impl ExtensionsConfig {
    /// Whether installed extensions update themselves. Defaults to [`DEFAULT_AUTO_UPDATE`].
    #[must_use]
    pub fn auto_update(&self) -> bool {
        self.auto_update.unwrap_or(DEFAULT_AUTO_UPDATE)
    }

    /// The installed extension ids. Defaults to empty.
    #[must_use]
    pub fn installed(&self) -> &[String] {
        self.installed.as_deref().unwrap_or(&[])
    }

    /// Sets `extensions.auto_update`. `None` removes the key.
    pub fn set_auto_update(&mut self, value: Option<bool>) -> &mut Self {
        self.auto_update = value;
        self
    }

    /// Sets `extensions.installed`. `None` removes the key.
    pub fn set_installed(&mut self, value: Option<Vec<String>>) -> &mut Self {
        self.installed = value;
        self
    }

    /// Keys present in the file that this build does not understand.
    #[must_use]
    pub fn unknown_fields(&self) -> &BTreeMap<String, Value> {
        &self.unknown
    }

    /// Whether the section carries nothing at all, known or unknown.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.auto_update.is_none() && self.installed.is_none() && self.unknown.is_empty()
    }
}

/// A parsed `vicinae.json`.
///
/// [`Config::default`] is the fully-defaulted configuration and is what an empty file produces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "LauncherConfig::is_empty")]
    launcher: LauncherConfig,
    #[serde(default, skip_serializing_if = "ExtensionsConfig::is_empty")]
    extensions: ExtensionsConfig,

    /// Top level keys this build does not know about, preserved verbatim.
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

impl Config {
    /// The `launcher` section.
    #[must_use]
    pub fn launcher(&self) -> &LauncherConfig {
        &self.launcher
    }

    /// The `launcher` section, mutably.
    pub fn launcher_mut(&mut self) -> &mut LauncherConfig {
        &mut self.launcher
    }

    /// The `extensions` section.
    #[must_use]
    pub fn extensions(&self) -> &ExtensionsConfig {
        &self.extensions
    }

    /// The `extensions` section, mutably.
    pub fn extensions_mut(&mut self) -> &mut ExtensionsConfig {
        &mut self.extensions
    }

    /// Top level keys present in the file that this build does not understand.
    #[must_use]
    pub fn unknown_fields(&self) -> &BTreeMap<String, Value> {
        &self.unknown
    }

    /// Parses `data`. An empty or whitespace-only input yields the default configuration.
    ///
    /// `path` is used only to build error messages.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] when `data` is not a JSON object of the expected shape.
    pub fn parse(data: &str, path: &Path) -> Result<Config, ConfigError> {
        if data.trim().is_empty() {
            return Ok(Config::default());
        }

        serde_json::from_str(data).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            line: source.line(),
            column: source.column(),
            message: parse_message(&source),
            source,
        })
    }

    /// Loads the configuration from `path`.
    ///
    /// A missing file is not an error: it yields the default configuration, which is what a fresh
    /// install has.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] if the file exists but cannot be read, [`ConfigError::Parse`] if it
    /// is not valid.
    pub fn load_from(path: impl AsRef<Path>) -> Result<Config, ConfigError> {
        let path = path.as_ref();
        let data = match std::fs::read_to_string(path) {
            Ok(data) => data,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %path.display(), "no configuration file, using defaults");
                return Ok(Config::default());
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        Config::parse(&data, path)
    }

    /// Loads the configuration from [`default_config_path`].
    ///
    /// Prefer [`Config::load_from`] anywhere the path should be injectable — notably in tests,
    /// which must never touch the invoking user's real configuration.
    ///
    /// # Errors
    ///
    /// See [`Config::load_from`], plus [`ConfigError::NoConfigDir`].
    pub fn load() -> Result<Config, ConfigError> {
        Config::load_from(default_config_path()?)
    }

    /// Serialises the configuration, unknown fields included.
    ///
    /// # Errors
    ///
    /// Only if a preserved unknown value cannot be serialised, which cannot happen for values
    /// that came from [`Config::parse`].
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        let mut out = serde_json::to_string_pretty(self)?;
        out.push('\n');
        Ok(out)
    }

    /// Writes the configuration to `path`, creating parent directories.
    ///
    /// The write goes to a sibling temporary file and is renamed into place, so a crash cannot
    /// leave a half-written config behind.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Write`] on any I/O failure.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let data = self.to_json_pretty().map_err(|err| ConfigError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other(err),
        })?;
        crate::atomic_write(path, data.as_bytes()).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Turns a [`serde_json::Error`] into a message that names the problem without repeating the
/// position, which the [`ConfigError::Parse`] display already carries.
fn parse_message(err: &serde_json::Error) -> String {
    let text = err.to_string();
    match text.find(" at line ") {
        Some(at) => text[..at].to_owned(),
        None => text,
    }
}

/// `$XDG_CONFIG_HOME/vicinae/vicinae.json`, falling back to `~/.config`.
///
/// # Errors
///
/// [`ConfigError::NoConfigDir`] when neither `$XDG_CONFIG_HOME` nor `$HOME` is usable.
pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    let dir = dirs::config_dir().ok_or(ConfigError::NoConfigDir)?;
    Ok(dir.join(CONFIG_RELATIVE_PATH))
}
