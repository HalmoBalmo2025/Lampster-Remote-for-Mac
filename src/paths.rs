//! Where Lampster Remote keeps its files in the user's Library.

use std::path::{Path, PathBuf};

/// The name of the `LaunchAgent` that opens the app at login, and of its
/// file.
pub(crate) const AGENT_LABEL: &str = "local.lampster-remote";

const APP_NAME: &str = "LampsterRemote";

#[derive(Debug)]
pub(crate) struct Paths {
    pub(crate) log: PathBuf,
    pub(crate) preferences: PathBuf,
    /// Exists while the app opens at login.
    pub(crate) agent: PathBuf,
}

impl Paths {
    /// `None` without a home folder.
    #[must_use]
    pub(crate) fn of_user() -> Option<Self> {
        Some(Self::in_library(&std::env::home_dir()?.join("Library")))
    }

    /// `library` is the user's Library folder.
    #[must_use]
    pub(crate) fn in_library(library: &Path) -> Self {
        Self {
            log: library.join(format!("Logs/{APP_NAME}.log")),
            preferences: library.join(format!("Application Support/{APP_NAME}/preferences.json")),
            agent: library.join(format!("LaunchAgents/{AGENT_LABEL}.plist")),
        }
    }
}
