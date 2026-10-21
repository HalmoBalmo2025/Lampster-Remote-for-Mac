//! The log ([`Paths::log`](crate::paths::Paths::log)): events with the
//! local time, one per line. It is off unless its file exists, which the
//! user makes to switch it on (README, "Switching the log on"). Without
//! the file, and
//! when it cannot be written, the lines go to stderr; only the latter does
//! the panel tell.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use crate::{clock, file};

/// Larger logs are moved aside to `LampsterRemote.old.log` when the
/// supervisor starts ([`rotate`]).
const MAX_SIZE: u64 = 1_000_000;

pub(crate) struct Log {
    sink: Sink,
}

enum Sink {
    File(File),
    /// The log is off: its file does not exist.
    Off,
    /// No home folder, or a file that cannot be opened for writing.
    Unwritable,
}

impl Log {
    /// `path` is `None` without a home folder.
    #[must_use]
    pub(crate) fn open(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self {
                sink: Sink::Unwritable,
            };
        };
        let sink = match file::open_append_existing(path) {
            Ok(file) => Sink::File(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Sink::Off,
            Err(error) => {
                eprintln!("cannot open the log: {error}");
                Sink::Unwritable
            }
        };
        Self { sink }
    }

    /// Whether the log cannot be written: switched on but not to be
    /// opened, or no home folder to look for it in. Its lines go to
    /// stderr, which an app opened from Finder does not show anywhere.
    pub(crate) const fn is_lost(&self) -> bool {
        match self.sink {
            Sink::Unwritable => true,
            Sink::File(_) | Sink::Off => false,
        }
    }

    pub(crate) fn record(&self, message: &str) {
        let time = clock::wall().strftime("%Y-%m-%d %H:%M:%S");
        let line = format!("{time} [{}] {message}\n", std::process::id());
        let written = match &self.sink {
            Sink::File(file) => {
                // `&File` writes, so the log needs no `&mut self`.
                let mut file: &File = file;
                file.write_all(line.as_bytes())
            }
            Sink::Off | Sink::Unwritable => Err(io::Error::other("no log file")),
        };
        if written.is_err() {
            eprint!("{line}");
        }
    }
}

/// Moves a log larger than [`MAX_SIZE`] aside, replacing the one moved
/// before, and starts an empty one, so the log stays switched on. Only the
/// supervisor does this, before it starts the app: the supervisor and the
/// app each keep the log open, and a log moved under one of them would
/// take that one's lines with it.
pub(crate) fn rotate(path: &Path) -> io::Result<()> {
    // A log that is not there needs no room: it is off.
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_SIZE) {
        file::move_over(path, &path.with_extension("old.log"))?;
        file::open_append(path)?;
    }
    Ok(())
}

/// The log line of a panic: where, and on which thread. Not the message,
/// which may hold the values it was formatted with.
pub(crate) fn panic_line(
    thread: Option<&str>,
    location: Option<&std::panic::Location<'_>>,
) -> String {
    // A thread started without a name, or a panic the runtime cannot place.
    let thread = thread.unwrap_or("without a name");
    match location {
        Some(location) => format!("panic on the thread {thread} at {location}"),
        None => format!("panic on the thread {thread}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_moves_a_large_log_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("LampsterRemote.log");
        File::create_new(&path)
            .unwrap()
            .write_all(&vec![b'x'; 1_000_001])
            .unwrap();
        rotate(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            0,
            "an empty log, still on"
        );
        let log = Log::open(Some(&path));
        log.record("first");
        log.record("second");
        let lines = fs::read_to_string(&path).unwrap();
        assert_eq!(lines.lines().count(), 2, "{lines}");
        assert!(lines.ends_with("second\n"), "{lines}");
        assert!(
            dir.path().join("LampsterRemote.old.log").is_file(),
            "the large log was moved aside"
        );
    }

    #[test]
    fn a_log_of_the_largest_size_stays() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("LampsterRemote.log");
        let size = usize::try_from(MAX_SIZE).unwrap();
        File::create_new(&path)
            .unwrap()
            .write_all(&vec![b'x'; size])
            .unwrap();
        rotate(&path).unwrap();
        assert!(!dir.path().join("LampsterRemote.old.log").exists());
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_SIZE);
    }

    #[test]
    fn the_app_moving_its_log_never_takes_the_supervisors_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("LampsterRemote.log");
        File::create_new(&path).unwrap();
        let supervisor = Log::open(Some(&path));
        supervisor.record(&"x".repeat(1_000_001));
        let app = Log::open(Some(&path));
        app.record("app started");
        supervisor.record("starting again");
        let current = fs::read_to_string(&path).unwrap();
        assert!(current.contains("starting again"), "the log the user reads");
    }

    #[test]
    fn a_panic_is_logged_by_place_and_thread() {
        let location = std::panic::Location::caller();
        let line = panic_line(Some("lampster-ble-reader"), Some(location));
        assert!(
            line.starts_with("panic on the thread lampster-ble-reader at src/log.rs:"),
            "{line}"
        );
        assert_eq!(panic_line(None, None), "panic on the thread without a name");
    }

    #[test]
    fn a_log_without_its_file_is_off_and_makes_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("LampsterRemote.log");
        let log = Log::open(Some(&path));
        assert!(!log.is_lost(), "off is not told");
        log.record("started");
        assert!(!path.exists(), "the log stays off");
        rotate(&path).unwrap();
        assert!(!path.exists(), "rotating does not switch it on");
    }

    #[test]
    fn a_log_switched_on_takes_the_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("LampsterRemote.log");
        File::create_new(&path).unwrap();
        let log = Log::open(Some(&path));
        assert!(!log.is_lost());
        log.record("started");
        assert!(
            fs::read_to_string(&path).unwrap().ends_with("started\n"),
            "the line is in the file"
        );
    }

    #[test]
    fn a_log_that_cannot_be_written_is_lost() {
        assert!(Log::open(None).is_lost(), "no home folder");
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Log::open(Some(dir.path())).is_lost(),
            "a folder is no log file"
        );
    }
}
