//! The one place that starts child processes: the Swift helpers
//! ([`helper`](crate::helper)) and the app itself under its supervisor
//! ([`watchdog`](crate::watchdog)).

use std::ffi::OsStr;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

use rustix::process::{Pid, Signal, kill_process_group};

/// A command for `program`, which starts in a process group of its own,
/// so [`stop`] ends what it started too. The environment is inherited:
/// every program started here is named by its full path, so a short
/// `PATH` (an app opened from Finder or by launchd) does not matter.
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "the process module is the one place that starts children"
)]
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.process_group(0);
    command
}

/// Kills `child` and the processes it started, then reaps it. A process
/// it started that kept its stdout would otherwise keep the reader of
/// that pipe waiting.
pub(crate) fn stop(child: &mut Child) {
    let group = Pid::from_child(child);
    // macOS sometimes refuses to signal the group (EPERM); in a stress run
    // of the tests a shell's child then lived on. The child is killed on
    // its own as well, and after it is reaped the group is signalled once
    // more. Each fails only when what it signals has ended, which is what
    // this is for.
    match kill_process_group(group, Signal::KILL) {
        Ok(()) | Err(_) => {}
    }
    match child.kill() {
        Ok(()) | Err(_) => {}
    }
    match child.wait() {
        Ok(_) | Err(_) => {}
    }
    match kill_process_group(group, Signal::KILL) {
        Ok(()) | Err(_) => {}
    }
}
