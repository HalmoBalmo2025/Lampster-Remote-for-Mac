//! The settings of the app itself: opening at login, and switching the
//! lamp off when the Mac locks, sleeps or shuts down.
//!
//! Opening at login is a `LaunchAgent` that runs `open -a` on the app, the
//! same way as a double click, so the app's Bluetooth access applies; it is
//! on while its file exists. The rest is stored in a JSON file.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::file;
use crate::paths::AGENT_LABEL;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Switch {
    On,
    #[default]
    Off,
}

/// What the preferences file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Preferences {
    /// When the Mac locks, sleeps, shuts down, restarts or logs out.
    pub(crate) off_with_mac: Switch,
}

/// The format of the preferences file this build reads and writes.
/// `fixtures/preferences/` holds a file of every format, which a test
/// reads.
const VERSION: u32 = 1;

/// The preferences file as it lies on disk. A file without a version is
/// from before versions were written, in the format of version 1.
#[derive(Serialize, Deserialize)]
struct Stored {
    version: Option<u32>,
    #[serde(default)]
    off_with_mac: Switch,
}

/// The settings as the panel shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub(crate) struct Options {
    pub(crate) open_at_login: Switch,
    pub(crate) off_with_mac: Switch,
}

/// Which of Lampster Remote's files an error is about. Errors name the
/// file, not its path, which holds the user's name and goes to the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppFile {
    Preferences,
    LoginAgent,
}

impl fmt::Display for AppFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Preferences => "the preferences file",
            Self::LoginAgent => "the login agent",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PreferencesError {
    #[error("no home folder")]
    NoHome,
    #[error("opening at login only works when Lampster Remote runs as Lampster Remote.app")]
    NotBundled,
    #[error("cannot find the running program: {0}")]
    NoExecutable(#[source] io::Error),
    #[error("cannot read {file}: {source}")]
    Read {
        file: AppFile,
        #[source]
        source: io::Error,
    },
    #[error("cannot write {file}: {source}")]
    Write {
        file: AppFile,
        #[source]
        source: io::Error,
    },
    #[error("the preferences file is of version {0}, newer than this build reads")]
    Newer(u32),
    #[error("the preferences file could not be read at start, so it is not written over")]
    NotLoaded,
    #[error("cannot remove {file}: {source}")]
    Remove {
        file: AppFile,
        #[source]
        source: io::Error,
    },
}

/// A missing file holds the defaults. One that cannot be read, or that a
/// newer build wrote, is an error; the caller must then not save over it.
pub(crate) fn load(path: &Path) -> Result<Preferences, PreferencesError> {
    let read = |source| PreferencesError::Read {
        file: AppFile::Preferences,
        source,
    };
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Preferences::default());
        }
        Err(source) => return Err(read(source)),
    };
    let stored: Stored = serde_json::from_str(&text).map_err(|error| read(error.into()))?;
    if let Some(newer) = stored.version.filter(|version| *version > VERSION) {
        return Err(PreferencesError::Newer(newer));
    }
    Ok(Preferences {
        off_with_mac: stored.off_with_mac,
    })
}

/// Whether the preferences file may be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Saving {
    Allowed,
    /// It could not be read at start; writing would replace what it holds.
    Blocked,
}

impl Saving {
    /// A file that could not be read is never saved over.
    pub(crate) const fn after<T>(loaded: &Result<T, PreferencesError>) -> Self {
        match loaded {
            Ok(_) => Self::Allowed,
            Err(_) => Self::Blocked,
        }
    }
}

pub(crate) fn save(
    path: &Path,
    preferences: Preferences,
    saving: Saving,
) -> Result<(), PreferencesError> {
    match saving {
        Saving::Allowed => {}
        Saving::Blocked => return Err(PreferencesError::NotLoaded),
    }
    let write = |source| PreferencesError::Write {
        file: AppFile::Preferences,
        source,
    };
    let stored = Stored {
        version: Some(VERSION),
        off_with_mac: preferences.off_with_mac,
    };
    let text = serde_json::to_string_pretty(&stored).map_err(|error| write(error.into()))?;
    file::replace(path, text.as_bytes()).map_err(write)
}

#[must_use]
pub(crate) fn open_at_login(agent: &Path) -> Switch {
    if agent.is_file() {
        Switch::On
    } else {
        Switch::Off
    }
}

/// `agent` is the login agent's file; `app` is the bundle that opens at
/// login.
pub(crate) fn set_open_at_login(
    agent: &Path,
    wanted: Switch,
    app: Option<&Path>,
) -> Result<(), PreferencesError> {
    match wanted {
        Switch::On => {
            let app = app.ok_or(PreferencesError::NotBundled)?;
            file::replace(agent, agent_plist(app).as_bytes()).map_err(|source| {
                PreferencesError::Write {
                    file: AppFile::LoginAgent,
                    source,
                }
            })
        }
        Switch::Off => file::remove_if_present(agent).map_err(|source| PreferencesError::Remove {
            file: AppFile::LoginAgent,
            source,
        }),
    }
}

/// The `.app` the running program lies in; `None` outside of one.
pub(crate) fn running_app() -> Result<Option<PathBuf>, PreferencesError> {
    let executable = std::env::current_exe().map_err(PreferencesError::NoExecutable)?;
    Ok(executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(Path::to_path_buf))
}

fn agent_plist(app: &Path) -> String {
    let app = escape_xml(&app.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/open</string>
        <string>-a</string>
        <string>{app}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#
    )
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own, removed when it is dropped, and a file in a
    /// subfolder that does not exist yet.
    fn file(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join(name);
        (dir, path)
    }

    #[test]
    fn preferences_survive_a_restart() {
        let (_dir, path) = file("preferences.json");
        assert_eq!(load(&path).unwrap(), Preferences::default());
        let wanted = Preferences {
            off_with_mac: Switch::On,
        };
        save(&path, wanted, Saving::Allowed).unwrap();
        assert_eq!(load(&path).unwrap(), wanted);
        file::replace(&path, b"garbage").unwrap();
        assert!(
            matches!(
                load(&path),
                Err(PreferencesError::Read {
                    file: AppFile::Preferences,
                    source: _
                })
            ),
            "garbage is reported; the caller keeps the defaults"
        );
    }

    #[test]
    fn opening_at_login_is_the_agent_file() {
        let (_dir, agent) = file("agent.plist");
        assert_eq!(open_at_login(&agent), Switch::Off);
        let app = Path::new("/Users/a/Applications/Lamp & Co.app");
        set_open_at_login(&agent, Switch::On, Some(app)).unwrap();
        assert_eq!(open_at_login(&agent), Switch::On);
        let plist = fs::read_to_string(&agent).unwrap();
        assert!(
            plist.contains("<string>/Users/a/Applications/Lamp &amp; Co.app</string>"),
            "{plist}"
        );
        set_open_at_login(&agent, Switch::Off, None).unwrap();
        set_open_at_login(&agent, Switch::Off, None).unwrap();
        assert_eq!(open_at_login(&agent), Switch::Off);
        assert!(
            matches!(
                set_open_at_login(&agent, Switch::On, None),
                Err(PreferencesError::NotBundled)
            ),
            "only from the app bundle"
        );
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/preferences")
            .join(name)
    }

    #[test]
    fn every_format_of_the_preferences_file_is_read() {
        let on = Preferences {
            off_with_mac: Switch::On,
        };
        assert_eq!(load(&fixture("without-version.json")).unwrap(), on);
        assert_eq!(load(&fixture("v1.json")).unwrap(), on);
    }

    #[test]
    fn the_preferences_file_is_written_as_the_fixture_of_its_version() {
        let (_dir, path) = file("preferences.json");
        save(
            &path,
            Preferences {
                off_with_mac: Switch::On,
            },
            Saving::Allowed,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            fs::read_to_string(fixture("v1.json")).unwrap()
        );
    }

    #[test]
    fn a_newer_preferences_file_is_an_error() {
        let (_dir, path) = file("preferences.json");
        file::replace(
            &path,
            br#"{"version":2,"off_with_mac":"on","colour":"teal"}"#,
        )
        .unwrap();
        assert!(
            matches!(load(&path), Err(PreferencesError::Newer(2))),
            "left alone for the newer build"
        );
    }

    #[test]
    fn a_file_that_failed_to_load_is_left_as_it_is() {
        let (_dir, path) = file("preferences.json");
        file::replace(&path, b"{broken").unwrap();
        let saving = Saving::after(&load(&path));
        assert_eq!(saving, Saving::Blocked);
        assert!(matches!(
            save(&path, Preferences::default(), saving),
            Err(PreferencesError::NotLoaded)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
        assert_eq!(
            Saving::after(&Ok::<(), PreferencesError>(())),
            Saving::Allowed
        );
    }
}
