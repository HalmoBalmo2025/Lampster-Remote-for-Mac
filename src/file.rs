//! The one place that replaces, moves, removes or appends to files. Only
//! Lampster Remote's own files pass through here (preferences, the login
//! agent, the log); a file is never truncated in place.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Writes `contents` to `path`, creating its folder: first to a draft
/// beside it, then renamed over it, so a crash never leaves half a file.
/// A draft left by a crashed run is overwritten.
pub(crate) fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let draft = path.with_added_extension("new");
    let mut file = open_draft(&draft)?;
    file.write_all(contents)?;
    file.sync_all()?;
    move_over(&draft, path)
}

#[expect(
    clippy::disallowed_methods,
    reason = "the draft is this module's own file, and a stale one must go"
)]
fn open_draft(draft: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(draft)
}

/// Renames `from` to `to`, replacing a file at `to`.
#[expect(
    clippy::disallowed_methods,
    reason = "the file module is the one place that renames over a file"
)]
pub(crate) fn move_over(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// Removes the file at `path`; a file that is not there is fine.
#[expect(
    clippy::disallowed_methods,
    reason = "the file module is the one place that removes files"
)]
pub(crate) fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Opens `path` for appending, creating it and its folder if needed.
#[expect(
    clippy::disallowed_methods,
    reason = "appending never overwrites what the file holds"
)]
pub(crate) fn open_append(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// Opens `path` for appending only if it exists; a missing file is
/// `NotFound` and stays missing.
pub(crate) fn open_append_existing(path: &Path) -> io::Result<File> {
    OpenOptions::new().append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_a_file_and_leaves_no_draft() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("preferences.json");
        replace(&path, b"first").unwrap();
        replace(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert!(
            !path.with_added_extension("new").exists(),
            "the draft is renamed"
        );
    }

    #[test]
    fn a_stale_draft_does_not_block_replacing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.plist");
        File::create_new(path.with_added_extension("new"))
            .unwrap()
            .write_all(b"left by a crash, and longer")
            .unwrap();
        replace(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn removing_a_missing_file_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.plist");
        replace(&path, b"x").unwrap();
        remove_if_present(&path).unwrap();
        assert!(!path.exists());
        remove_if_present(&path).unwrap();
        assert!(
            remove_if_present(dir.path()).is_err(),
            "a folder is not removed"
        );
    }

    #[test]
    fn appending_keeps_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Logs").join("app.log");
        open_append(&path).unwrap().write_all(b"one\n").unwrap();
        open_append(&path).unwrap().write_all(b"two\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        open_append_existing(&path)
            .unwrap()
            .write_all(b"three\n")
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\nthree\n");
    }

    #[test]
    fn appending_to_what_is_not_there_makes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        assert_eq!(
            open_append_existing(&path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(!path.exists());
    }
}
