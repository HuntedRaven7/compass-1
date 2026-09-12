//! `org.freedesktop.portal.FileChooser`.
//!
//! Ported from the intent of `src/server/src/services/file-chooser/`: the C++
//! `AbstractFileChooser` models a request as
//! `{canChooseFiles, canChooseDirectories, allowMultipleSelection, currentFolder}`
//! and reports either `filesChosen(paths)` or `rejected()`. That two-outcome
//! shape is kept, as [`FileChooserOutcome`], because "the user pressed Cancel"
//! is not an error.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ashpd::desktop::ResponseError;
use ashpd::desktop::file_chooser::SelectedFiles;

use crate::error::{PortalError, Result};
use crate::shortcuts::bounded;

/// What to ask the user for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileChooserRequest {
    /// Dialog title.
    pub title: String,
    /// Label for the accept button, if the default is not right.
    pub accept_label: Option<String>,
    /// Allow selecting more than one entry.
    pub multiple: bool,
    /// Select directories instead of files.
    pub directory: bool,
    /// Where the dialog should start.
    pub current_folder: Option<PathBuf>,
    /// Whether the dialog should be modal to the parent window. We never pass
    /// a parent window identifier, so this is advisory.
    pub modal: bool,
}

impl FileChooserRequest {
    /// A request for a single file.
    pub fn file(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    /// A request for a single directory.
    pub fn directory(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            directory: true,
            ..Self::default()
        }
    }

    /// Allow multiple selection.
    #[must_use]
    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// Start the dialog in `folder`.
    #[must_use]
    pub fn starting_in(mut self, folder: impl Into<PathBuf>) -> Self {
        self.current_folder = Some(folder.into());
        self
    }
}

/// The user's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileChooserOutcome {
    /// Paths the user chose. Never empty.
    Selected {
        /// Chosen paths, decoded from the portal's `file:` URIs.
        paths: Vec<PathBuf>,
    },
    /// The user cancelled the dialog (portal response code 1).
    Cancelled,
    /// The portal ended the request unsuccessfully without saying why
    /// (response code 2).
    Refused,
}

impl FileChooserOutcome {
    /// Chosen paths, or an empty slice.
    pub fn paths(&self) -> &[PathBuf] {
        match self {
            Self::Selected { paths } => paths,
            _ => &[],
        }
    }
}

/// Client for `org.freedesktop.portal.FileChooser`.
#[derive(Debug, Clone)]
pub struct FileChooserPortal {
    conn: zbus::Connection,
    timeout: Duration,
}

impl FileChooserPortal {
    pub(crate) fn new(conn: zbus::Connection, timeout: Duration) -> Self {
        Self { conn, timeout }
    }

    /// Show an open dialog.
    ///
    /// Note the deadline: unlike every other call here the user is genuinely
    /// expected to take their time, so callers should pass a
    /// [`PortalConfig::dialog_timeout`](crate::PortalConfig::dialog_timeout)
    /// large enough to be a hang detector rather than an impatience detector.
    pub async fn open(&self, request: FileChooserRequest) -> Result<FileChooserOutcome> {
        let mut builder = SelectedFiles::open_file()
            .connection(Some(self.conn.clone()))
            .title(request.title.as_str())
            .multiple(request.multiple)
            .directory(request.directory)
            .modal(request.modal);
        if let Some(label) = &request.accept_label {
            builder = builder.accept_label(label.as_str());
        }
        if let Some(folder) = &request.current_folder {
            builder = builder
                .current_folder::<&Path>(Some(folder.as_path()))
                .map_err(|source| PortalError::call("OpenFile", source))?;
        }

        let reply = bounded(self.timeout, "OpenFile", builder.send()).await?;
        match reply.response() {
            Ok(selected) => {
                let mut paths = Vec::with_capacity(selected.uris().len());
                for uri in selected.uris() {
                    paths.push(file_uri_to_path(uri.as_str()).ok_or_else(|| {
                        PortalError::Protocol(format!(
                            "FileChooser returned a non-file URI: {}",
                            uri.as_str()
                        ))
                    })?);
                }
                if paths.is_empty() {
                    // A successful reply with nothing in it violates the
                    // interface; treat it as a refusal rather than pretending
                    // the user picked something.
                    return Ok(FileChooserOutcome::Refused);
                }
                Ok(FileChooserOutcome::Selected { paths })
            }
            Err(ashpd::Error::Response(ResponseError::Cancelled)) => {
                Ok(FileChooserOutcome::Cancelled)
            }
            Err(ashpd::Error::Response(ResponseError::Other)) => Ok(FileChooserOutcome::Refused),
            Err(source) => Err(PortalError::call("OpenFile", source)),
        }
    }
}

/// Decode a `file://` URI into a path, undoing percent-encoding.
///
/// The portal always answers with `file:` URIs and we always want paths; the
/// C++ side did the same conversion with `QUrl::toLocalFile`. Returns `None`
/// for any other scheme, or for a percent escape that is not valid.
pub(crate) fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // An authority component other than the (empty) localhost is not a local
    // file we can open.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(_) => return None,
        None => return None,
    };
    let decoded = percent_decode(path)?;
    if decoded.is_empty() {
        return None;
    }
    Some(PathBuf::from(decoded))
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = input.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A path that came back from the portal, for use by callers that hold a raw
/// URI rather than a [`FileChooserOutcome`].
pub fn path_from_file_uri(uri: &str) -> Option<&Path> {
    // Only valid when nothing needed decoding; otherwise the caller needs the
    // owned form.
    if uri.contains('%') {
        return None;
    }
    uri.strip_prefix("file://").map(Path::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_file_uris_decode() {
        assert_eq!(
            file_uri_to_path("file:///home/user/a.txt"),
            Some(PathBuf::from("/home/user/a.txt"))
        );
    }

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(
            file_uri_to_path("file:///home/user/My%20Documents/r%C3%A9sum%C3%A9.pdf"),
            Some(PathBuf::from("/home/user/My Documents/résumé.pdf"))
        );
    }

    #[test]
    fn non_file_and_malformed_uris_are_rejected_not_panicked_on() {
        for uri in [
            "http://example.com/a",
            "file://remotehost/share/a",
            "file://",
            "file:///%zz",
            "file:///%2",
            "",
            "/home/user/a.txt",
        ] {
            assert_eq!(file_uri_to_path(uri), None, "{uri}");
        }
    }

    #[test]
    fn outcomes_expose_paths_only_when_selected() {
        assert!(FileChooserOutcome::Cancelled.paths().is_empty());
        let selected = FileChooserOutcome::Selected {
            paths: vec![PathBuf::from("/tmp/x")],
        };
        assert_eq!(selected.paths().len(), 1);
    }

    #[test]
    fn borrowed_helper_declines_encoded_uris() {
        assert_eq!(path_from_file_uri("file:///a/b"), Some(Path::new("/a/b")));
        assert_eq!(path_from_file_uri("file:///a%20b"), None);
    }
}
