//! The file preview beside a list of files: dmenu's quick look and Search
//! Files' detail pane.
//!
//! Ports `qml::resolveFilePreview` and the two view hosts' `loadDetail`: the
//! name, the path, the MIME type (and for Search Files the modified time),
//! over an image drawn from the file, the first 10 KiB of a text file up to
//! 2 MiB, or nothing for anything else.

use std::path::{Path, PathBuf};

/// A text file larger than this is not previewed: `MAX_PREVIEW_SIZE`.
pub const MAX_PREVIEW_SIZE: u64 = 2 * 1024 * 1024;

/// How much of a text file is shown.
pub const MAX_DISPLAY: usize = 10 * 1024;

/// What the pane draws above the metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// The image at this path.
    Image(PathBuf),
    /// The start of a text file.
    Text(String),
    /// Nothing to draw: a directory, or a file of another type.
    None,
}

/// One file's preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePreview {
    /// The file's own name.
    pub name: String,
    /// Its path, with the home directory folded to `~` when asked for.
    pub path: String,
    /// Its MIME type.
    pub mime: String,
    /// When it was last modified, as `QDateTime::toString()` writes it;
    /// `None` when not asked for or unreadable.
    pub modified: Option<String>,
    /// What to draw.
    pub content: Content,
}

impl FilePreview {
    /// The metadata listed under the preview, as `(label, value)`.
    #[must_use]
    pub fn fields(&self) -> Vec<(&'static str, &str)> {
        let mut fields = vec![
            ("Name", self.name.as_str()),
            ("Path", self.path.as_str()),
            ("Type", self.mime.as_str()),
        ];
        if let Some(modified) = &self.modified {
            fields.push(("Last modified", modified.as_str()));
        }
        fields
    }
}

/// Whether a MIME type is text, as `QMimeType::inherits("text/plain")`
/// answers for the types a file's extension gives.
#[must_use]
pub fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/x-sh"
                | "application/x-shellscript"
                | "application/toml"
                | "application/x-toml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/sql"
                | "application/x-perl"
                | "application/x-python"
                | "application/x-ruby"
                | "application/x-desktop"
                | "application/xhtml+xml"
                | "image/svg+xml"
        )
}

/// Reads `path`'s preview; `None` when it does not exist. `home` folds the
/// path shown (Search Files' `compressPath`); `modified` adds the time.
#[must_use]
pub fn load(path: &Path, home: Option<&Path>, modified: bool) -> Option<FilePreview> {
    let metadata = std::fs::metadata(path).ok()?;
    let mime = compass_xdg::mimeapps::file_mime(path);
    let content = if mime.starts_with("image/") && mime != "image/svg+xml" {
        Content::Image(path.to_path_buf())
    } else if metadata.is_file() && is_text_mime(&mime) && metadata.len() <= MAX_PREVIEW_SIZE {
        read_text(path).map_or(Content::None, Content::Text)
    } else {
        Content::None
    };
    let shown = path.to_string_lossy().into_owned();
    let shown = match home.map(|home| home.to_string_lossy().into_owned()) {
        Some(home) => compass_core::system_run::compress_path(&shown, &home),
        None => shown,
    };
    Some(FilePreview {
        name: path.file_name().map_or_else(
            || path.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        ),
        path: shown,
        mime,
        modified: modified
            .then(|| metadata.modified().ok().and_then(qt_text_date))
            .flatten(),
        content,
    })
}

/// The first [`MAX_DISPLAY`] bytes of a file, cut back to a character
/// boundary, as `QString::fromUtf8(file.read(MAX_DISPLAY))` reads them.
fn read_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut buffer = Vec::with_capacity(MAX_DISPLAY);
    std::fs::File::open(path)
        .ok()?
        .take(MAX_DISPLAY as u64)
        .read_to_end(&mut buffer)
        .ok()?;
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// A time as `QDateTime::toString()` (`Qt::TextDate`) writes it in the local
/// zone: `Wed May 20 03:40:13 1998`.
pub(crate) fn qt_text_date(time: std::time::SystemTime) -> Option<String> {
    let at = jiff::Timestamp::try_from(time)
        .ok()?
        .to_zoned(jiff::tz::TimeZone::system());
    let small = |value: i8| u8::try_from(value).unwrap_or_default();
    let broken = compass_core::qt_date::DateTime {
        year: i32::from(at.year()),
        month: small(at.month()),
        day: small(at.day()),
        hour: small(at.hour()),
        minute: small(at.minute()),
        second: small(at.second()),
        millisecond: u16::try_from(at.millisecond()).unwrap_or_default(),
        weekday: small(at.weekday().to_monday_one_offset()),
        zone: at.strftime("%Z").to_string(),
    };
    Some(compass_core::qt_date::format(
        "ddd MMM d hh:mm:ss yyyy",
        &broken,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_file_shows_its_start_and_an_image_itself() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.md");
        std::fs::write(&notes, "# Title\n\nbody").unwrap();
        let preview = load(&notes, Some(dir.path()), true).unwrap();
        assert_eq!(preview.name, "notes.md");
        assert_eq!(preview.path, "~/notes.md");
        assert_eq!(preview.mime, "text/markdown");
        assert_eq!(preview.content, Content::Text("# Title\n\nbody".into()));
        assert!(preview.modified.is_some());

        let picture = dir.path().join("a.png");
        std::fs::write(&picture, b"not really").unwrap();
        let preview = load(&picture, None, false).unwrap();
        assert_eq!(preview.content, Content::Image(picture.clone()));
        assert_eq!(preview.path, picture.to_string_lossy());
        assert_eq!(preview.modified, None);

        let folder = load(dir.path(), None, false).unwrap();
        assert_eq!(folder.mime, "inode/directory");
        assert_eq!(folder.content, Content::None);
        assert!(load(&dir.path().join("missing"), None, false).is_none());
    }

    #[test]
    fn a_long_text_file_is_cut_at_ten_kibibytes() {
        let dir = tempfile::tempdir().unwrap();
        let long = dir.path().join("long.txt");
        std::fs::write(&long, "é".repeat(MAX_DISPLAY)).unwrap();
        let Content::Text(text) = load(&long, None, false).unwrap().content else {
            panic!("no text");
        };
        assert!(text.len() <= MAX_DISPLAY + 3);
        assert!(text.starts_with("éé"));
    }
}
