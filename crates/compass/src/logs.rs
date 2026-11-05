//! The engine's log file, and `compass logs`, which prints it.
//!
//! The C++ server writes every log line to `$XDG_STATE_HOME/vicinae/vicinae.log`
//! as well as to standard error, rotating it to `vicinae.log.1` past five
//! mebibytes, and `compass logs` tails it (`FileTailer`). The engine here does
//! the same into its own file, `compass.log` beside it (ADR-0017: Compass owns
//! its files, and two engines appending to one log would interleave), and
//! `compass logs` prints the last lines and, with `--follow`, what is written
//! after them, reopening the file when it is rotated or truncated.
//!
//! The file is opened only once the engine owns its socket
//! ([`LogFile::activate`]): an engine refused because another is running has
//! nothing to add to that one's log, and lines before then go to standard
//! error alone.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// Past this size the log is rotated, as the C++ `MAX_LOG_SIZE`.
pub const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024;

/// How often `--follow` looks for new lines, as the C++ `POLL_INTERVAL`.
pub const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The log's file name under the state directory.
pub const FILE_NAME: &str = compass_core::xdg_dirs::LOG_FILE_NAME;

/// `$XDG_STATE_HOME/compass/compass.log`, falling back to
/// `~/.local/state/compass/compass.log`, as the C++ `stateDir`: the file
/// [`compass_core::xdg_dirs::log_file`] names, which Show Log File shows.
#[must_use]
pub fn log_path() -> Option<PathBuf> {
    compass_core::xdg_dirs::log_file()
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    rotated: PathBuf,
    file: Option<File>,
    size: u64,
    max: u64,
}

/// The log file the engine appends to, rotated past a size: a
/// `tracing_subscriber` writer. Cloning shares the file.
#[derive(Debug, Clone)]
pub struct LogFile {
    inner: Arc<Mutex<Inner>>,
}

/// The engine's log, once `main` has made one: what `serve` activates.
static ENGINE_LOG: std::sync::OnceLock<LogFile> = std::sync::OnceLock::new();

impl LogFile {
    /// The log at `path`, rotated past [`MAX_LOG_SIZE`], written nowhere
    /// until [`Self::activate`].
    #[must_use]
    pub fn pending(path: &Path) -> Self {
        Self::pending_with_limit(path, MAX_LOG_SIZE)
    }

    /// As [`Self::pending`], rotating past `max` bytes.
    #[must_use]
    pub fn pending_with_limit(path: &Path, max: u64) -> Self {
        let mut rotated = path.as_os_str().to_owned();
        rotated.push(".1");
        Self {
            inner: Arc::new(Mutex::new(Inner {
                path: path.to_owned(),
                rotated: PathBuf::from(rotated),
                file: None,
                size: 0,
                max,
            })),
        }
    }

    /// Opens the file for appending, creating its directory.
    pub fn activate(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.file.is_some() {
            return;
        }
        if let Some(parent) = inner.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        inner.file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&inner.path)
            .ok();
        inner.size = inner
            .file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map_or(0, |meta| meta.len());
    }

    /// Keeps this as the engine's log, for [`activate_engine_log`].
    pub fn register(&self) {
        let _ = ENGINE_LOG.set(self.clone());
    }
}

/// Opens the engine's log, if `main` made one: called once the engine has
/// bound its socket.
pub fn activate_engine_log() {
    if let Some(log) = ENGINE_LOG.get() {
        log.activate();
    }
}

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(file) = inner.file.as_mut() else {
            return Ok(buf.len());
        };
        file.write_all(buf)?;
        inner.size += buf.len() as u64;
        if inner.size >= inner.max {
            let (path, rotated) = (inner.path.clone(), inner.rotated.clone());
            inner.file = None;
            let _ = std::fs::rename(&path, &rotated);
            inner.file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .ok();
            inner.size = 0;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match inner.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogFile {
    type Writer = LogFile;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Where the last `lines` lines of the first `size` bytes of `file` start, as
/// the C++ `findLastLines`: a trailing newline does not count as an empty last
/// line, and asking for more lines than there are starts at the beginning.
///
/// # Errors
///
/// When the file cannot be read.
pub fn last_lines_offset(file: &mut File, size: u64, lines: usize) -> std::io::Result<u64> {
    if size == 0 {
        return Ok(0);
    }
    if lines == 0 {
        return Ok(size);
    }
    let mut buf = vec![0_u8; 8192];
    let mut pos = size;
    let mut found = 0;
    let mut at_end = true;
    while pos > 0 {
        let chunk = pos.min(buf.len() as u64);
        pos -= chunk;
        let chunk = usize::try_from(chunk).unwrap_or(buf.len());
        file.seek(SeekFrom::Start(pos))?;
        file.read_exact(&mut buf[..chunk])?;
        let mut end = chunk;
        if at_end && buf[chunk - 1] == b'\n' {
            end -= 1;
        }
        at_end = false;
        for i in (0..end).rev() {
            if buf[i] == b'\n' {
                found += 1;
                if found == lines {
                    return Ok(pos + i as u64 + 1);
                }
            }
        }
    }
    Ok(0)
}

/// Copies `file` from `offset` to its end into `out`, returning the new
/// offset.
fn drain(file: &mut File, offset: u64, out: &mut impl Write) -> std::io::Result<u64> {
    file.seek(SeekFrom::Start(offset))?;
    let copied = std::io::copy(file, out)?;
    out.flush()?;
    Ok(offset + copied)
}

/// Prints the last `lines` lines of the log at `path` to `out`. `None` when
/// there is no log to read.
///
/// # Errors
///
/// When the file exists but cannot be read, or `out` cannot be written.
pub fn tail(path: &Path, lines: usize, out: &mut impl Write) -> std::io::Result<Option<u64>> {
    let Ok(mut file) = File::open(path) else {
        return Ok(None);
    };
    // One size for both the search and the copy: the engine keeps appending,
    // and copying to whatever the end is by then would print more lines than
    // were asked for. `follow` picks up from `size`.
    let size = file.metadata()?.len();
    let offset = last_lines_offset(&mut file, size, lines)?;
    file.seek(SeekFrom::Start(offset))?;
    std::io::copy(&mut (&mut file).take(size - offset), out)?;
    out.flush()?;
    Ok(Some(size))
}

/// After [`tail`], prints what is appended to `path` as it is written,
/// until `keep_going` says to stop. A file that shrinks (rotated, or
/// truncated) or appears is read again from its start, as `FileTailer::follow`.
///
/// # Errors
///
/// When `out` cannot be written.
pub fn follow(
    path: &Path,
    mut offset: Option<u64>,
    out: &mut impl Write,
    mut keep_going: impl FnMut() -> bool,
) -> std::io::Result<()> {
    while keep_going() {
        std::thread::sleep(POLL_INTERVAL);
        let Ok(size) = std::fs::metadata(path).map(|meta| meta.len()) else {
            continue;
        };
        let Ok(mut file) = File::open(path) else {
            continue;
        };
        let from = match offset {
            Some(offset) if size >= offset => offset,
            _ => 0,
        };
        if Some(size) == offset {
            continue;
        }
        offset = Some(drain(&mut file, from, out)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail_of(text: &str, lines: usize) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        std::fs::write(&path, text).unwrap();
        let mut out = Vec::new();
        tail(&path, lines, &mut out).unwrap().unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_last_lines_are_found_as_the_cpp_finds_them() {
        assert_eq!(tail_of("a\nb\nc\n", 2), "b\nc\n");
        assert_eq!(tail_of("a\nb\nc", 2), "b\nc", "no trailing newline");
        assert_eq!(tail_of("a\nb\nc\n", 10), "a\nb\nc\n", "more than there are");
        assert_eq!(tail_of("a\nb\nc\n", 0), "", "none");
        assert_eq!(tail_of("", 5), "");
        let long: String = (0..5000).map(|i| format!("line {i}\n")).collect();
        assert_eq!(tail_of(&long, 3), "line 4997\nline 4998\nline 4999\n");
    }

    #[test]
    fn lines_appended_after_the_size_was_taken_are_not_counted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        std::fs::write(&path, "a\nb\nc\nnew\n").unwrap();
        let mut file = File::open(&path).unwrap();
        assert_eq!(
            last_lines_offset(&mut file, 6, 1).unwrap(),
            4,
            "the line before the cut"
        );
    }

    #[test]
    fn no_log_is_none_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        assert_eq!(tail(&dir.path().join("absent"), 5, &mut out).unwrap(), None);
    }

    #[test]
    fn a_pending_log_creates_nothing_until_it_is_activated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/compass/compass.log");
        let mut log = LogFile::pending(&path);
        log.write_all(b"early\n").unwrap();
        assert!(!dir.path().join("state").exists());
        log.activate();
        log.write_all(b"late\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "late\n");
    }

    #[test]
    fn the_log_appends_and_rotates_past_its_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/compass/compass.log");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "kept\n").unwrap();
        let mut log = LogFile::pending_with_limit(&path, 20);
        log.write_all(b"dropped\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "kept\n",
            "not yet active"
        );
        log.activate();
        log.write_all(b"0123456789\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "kept\n0123456789\n"
        );
        log.write_all(b"abcdefghij\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("state/compass/compass.log.1")).unwrap(),
            "kept\n0123456789\nabcdefghij\n"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        log.write_all(b"after\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after\n");
    }

    #[test]
    fn following_prints_what_is_appended_and_starts_over_after_a_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        std::fs::write(&path, "one\n").unwrap();
        let mut out = Vec::new();
        let offset = tail(&path, 1, &mut out).unwrap();
        let mut rounds = 0;
        let writer = path.clone();
        follow(&path, offset, &mut out, || {
            rounds += 1;
            match rounds {
                1 => {
                    let mut file = std::fs::OpenOptions::new()
                        .append(true)
                        .open(&writer)
                        .unwrap();
                    file.write_all(b"two\n").unwrap();
                }
                2 => std::fs::write(&writer, "new\n").unwrap(),
                _ => {}
            }
            rounds <= 3
        })
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "one\ntwo\nnew\n");
    }
}
