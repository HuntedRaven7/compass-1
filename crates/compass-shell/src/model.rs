//! Typed models for what the extension hands back.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

use crate::contract::window_key;
use crate::error::{Result, ShellError};

/// Opaque handle for a window, as minted by the shell extension.
///
/// Stable for the lifetime of the window; not stable across shell restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
pub struct WindowId(pub u32);

impl std::fmt::Display for WindowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for WindowId {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

/// One toplevel window as reported by `ListWindows`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct Window {
    /// Handle to pass back to activate/close.
    pub id: WindowId,
    /// Window title. May legitimately be empty.
    pub title: String,
    /// `WM_CLASS` (the "app" side of the pair). May legitimately be empty.
    pub wm_class: String,
    /// `WM_CLASS` instance name. Empty when the extension omits it.
    pub wm_class_instance: String,
    /// Owning process, when the extension could determine one.
    pub pid: Option<u32>,
    /// Whether this window currently has focus.
    pub focused: bool,
    /// Workspace index, when known.
    pub workspace: Option<i32>,
    /// Whether the window advertises that it can be closed.
    pub can_close: bool,
}

/// A clipboard selection: one blob plus the mime type describing it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClipboardContent {
    /// Raw bytes. For `text/*` this is UTF-8.
    pub data: Vec<u8>,
    /// Mime type. Empty means "the selection is empty / not offerable".
    pub mime_type: String,
}

impl ClipboardContent {
    /// Build a `text/plain;charset=utf-8` selection.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            data: text.into().into_bytes(),
            mime_type: "text/plain;charset=utf-8".to_owned(),
        }
    }

    /// Build a selection from raw bytes and an explicit mime type.
    pub fn binary(data: impl Into<Vec<u8>>, mime_type: impl Into<String>) -> Self {
        Self {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }

    /// True when the extension reported no offerable selection.
    pub fn is_empty(&self) -> bool {
        self.mime_type.is_empty() && self.data.is_empty()
    }

    /// Interpret the payload as UTF-8 text, if it plausibly is any.
    pub fn as_text(&self) -> Option<&str> {
        if self.mime_type.starts_with("text/") || self.mime_type.is_empty() {
            std::str::from_utf8(&self.data).ok()
        } else {
            None
        }
    }
}

/// A clipboard change as observed on the `ClipboardChanged` signal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClipboardChange {
    /// The new selection.
    pub content: ClipboardContent,
    /// Best-effort application id of whoever copied; `None` when unknown.
    pub source_app: Option<String>,
}

/// A single `a{sv}` entry from `ListWindows`.
pub(crate) type WindowDict = HashMap<String, OwnedValue>;

fn missing(key: &str) -> ShellError {
    ShellError::Protocol(format!(
        "window entry is missing the required `{key}` field"
    ))
}

fn wrong_type(key: &str, want: &str, got: &Value<'_>) -> ShellError {
    ShellError::Protocol(format!(
        "window field `{key}` should be {want} but the extension sent signature `{}`",
        got.value_signature()
    ))
}

fn as_u32(dict: &WindowDict, key: &str) -> Result<Option<u32>> {
    match dict.get(key) {
        None => Ok(None),
        Some(v) => match &**v {
            Value::U32(n) => Ok(Some(*n)),
            other => Err(wrong_type(key, "u", other)),
        },
    }
}

fn as_i32(dict: &WindowDict, key: &str) -> Result<Option<i32>> {
    match dict.get(key) {
        None => Ok(None),
        Some(v) => match &**v {
            Value::I32(n) => Ok(Some(*n)),
            other => Err(wrong_type(key, "i", other)),
        },
    }
}

fn as_bool(dict: &WindowDict, key: &str) -> Result<Option<bool>> {
    match dict.get(key) {
        None => Ok(None),
        Some(v) => match &**v {
            Value::Bool(b) => Ok(Some(*b)),
            other => Err(wrong_type(key, "b", other)),
        },
    }
}

fn as_string(dict: &WindowDict, key: &str) -> Result<Option<String>> {
    match dict.get(key) {
        None => Ok(None),
        Some(v) => match &**v {
            Value::Str(s) => Ok(Some(s.as_str().to_owned())),
            other => Err(wrong_type(key, "s", other)),
        },
    }
}

impl Window {
    /// Decode one `a{sv}` entry.
    ///
    /// Unknown keys are ignored on purpose, so the extension can add fields
    /// within a contract version. Missing or mistyped *required* keys are a
    /// [`ShellError::Protocol`] rather than a panic or a silent default.
    pub(crate) fn from_dict(dict: &WindowDict) -> Result<Self> {
        Ok(Self {
            id: WindowId(as_u32(dict, window_key::ID)?.ok_or_else(|| missing(window_key::ID))?),
            title: as_string(dict, window_key::TITLE)?.ok_or_else(|| missing(window_key::TITLE))?,
            wm_class: as_string(dict, window_key::WM_CLASS)?
                .ok_or_else(|| missing(window_key::WM_CLASS))?,
            wm_class_instance: as_string(dict, window_key::WM_CLASS_INSTANCE)?.unwrap_or_default(),
            pid: as_u32(dict, window_key::PID)?,
            focused: as_bool(dict, window_key::FOCUSED)?.unwrap_or(false),
            workspace: as_i32(dict, window_key::WORKSPACE)?.filter(|w| *w >= 0),
            can_close: as_bool(dict, window_key::CAN_CLOSE)?.unwrap_or(true),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> WindowDict {
        HashMap::from([
            ("id".to_owned(), OwnedValue::from(4u32)),
            (
                "title".to_owned(),
                OwnedValue::try_from(Value::from("Inbox")).expect("string"),
            ),
            (
                "wm_class".to_owned(),
                OwnedValue::try_from(Value::from("Geary")).expect("string"),
            ),
        ])
    }

    #[test]
    fn optional_fields_take_documented_defaults() {
        let window = Window::from_dict(&base()).expect("decodes");
        assert_eq!(window.id, WindowId(4));
        assert_eq!(window.wm_class_instance, "");
        assert_eq!(window.pid, None);
        assert!(!window.focused);
        assert_eq!(window.workspace, None);
        assert!(window.can_close, "can_close defaults to true");
    }

    #[test]
    fn negative_workspace_means_unknown() {
        let mut dict = base();
        dict.insert("workspace".to_owned(), OwnedValue::from(-1i32));
        assert_eq!(Window::from_dict(&dict).expect("decodes").workspace, None);
    }

    #[test]
    fn missing_required_field_is_an_error() {
        for key in ["id", "title", "wm_class"] {
            let mut dict = base();
            dict.remove(key);
            let err = Window::from_dict(&dict).expect_err("must not decode");
            assert!(matches!(err, ShellError::Protocol(_)), "{err:?}");
            assert!(err.to_string().contains(key), "{err}");
        }
    }

    #[test]
    fn mistyped_field_is_an_error() {
        let mut dict = base();
        dict.insert(
            "id".to_owned(),
            OwnedValue::try_from(Value::from("four")).expect("string"),
        );
        let err = Window::from_dict(&dict).expect_err("must not decode");
        assert!(err.to_string().contains("should be u"), "{err}");
    }

    #[test]
    fn clipboard_text_helpers() {
        let text = ClipboardContent::text("hi");
        assert_eq!(text.as_text(), Some("hi"));
        assert!(!text.is_empty());

        let png = ClipboardContent::binary(vec![0x89, 0x50], "image/png");
        assert_eq!(png.as_text(), None);

        assert!(ClipboardContent::default().is_empty());
    }
}
