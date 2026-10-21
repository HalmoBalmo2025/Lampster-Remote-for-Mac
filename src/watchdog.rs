//! The supervisor. A panic aborts the release build, and a helper that ends
//! makes the app end as failed; so the app runs as a child
//! ([`CHILD_ARGUMENT`]) that the supervisor starts again after a failure,
//! until it fails too often. The child ends when the supervisor's pipe on
//! its stdin closes ([`watch_parent`]), e.g. at logout.

use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};

use crate::clock::Clock;
use crate::process;

pub(crate) const CHILD_ARGUMENT: &str = "--child";

/// Failures within [`FAILURE_WINDOW`] before the supervisor gives up.
const MAX_FAILURES: usize = 3;
/// On the wall clock: failures hours apart, with the Mac asleep between
/// them, are not in a row.
const FAILURE_WINDOW: SignedDuration = SignedDuration::from_secs(120);
const RESTART_PAUSE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    Quit,
    GaveUp,
}

pub(crate) fn supervise(
    executable: &Path,
    log: impl Fn(&str),
    clock: &impl Clock,
) -> io::Result<Ending> {
    let mut failures: VecDeque<Timestamp> = VecDeque::new();
    loop {
        let status = run_child(executable)?;
        if status.success() {
            return Ok(Ending::Quit);
        }
        let now = clock.timestamp();
        // A failure "after" now (the clock was set back) counts as recent.
        failures.retain(|failure| now.duration_since(*failure) < FAILURE_WINDOW);
        failures.push_back(now);
        if failures.len() >= MAX_FAILURES {
            log(&format!(
                "Lampster Remote failed {MAX_FAILURES} times in a row ({status}) and stays off"
            ));
            return Ok(Ending::GaveUp);
        }
        log(&format!(
            "Lampster Remote failed ({status}); starting again"
        ));
        clock.sleep(RESTART_PAUSE);
    }
}

fn run_child(executable: &Path) -> io::Result<ExitStatus> {
    let mut child = process::command(executable)
        .arg(CHILD_ARGUMENT)
        .stdin(Stdio::piped())
        .spawn()?;
    // The pipe stays open while the child runs; its end tells the child.
    let pipe = child.stdin.take();
    let status = child.wait();
    drop(pipe);
    status
}

/// For the child: calls `on_parent_gone` when stdin closes. The thread
/// blocks on stdin, which nothing can interrupt, so it is not joined but
/// ends with the process.
pub(crate) fn watch_parent(
    on_parent_gone: impl FnOnce() + Send + 'static,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("watch-parent".to_owned())
        .spawn(move || {
            // A read error means the same as the end.
            match io::copy(&mut io::stdin(), &mut io::sink()) {
                Ok(_) | Err(_) => on_parent_gone(),
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SystemClock;
    use std::cell::Cell;
    use std::fs::{self, File};
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::Instant;

    /// Time passes only while the supervisor waits, and on the wall clock
    /// also while the Mac `naps` during each wait.
    struct FakeClock {
        instant: Cell<Instant>,
        wall: Cell<Timestamp>,
        naps: Duration,
    }

    impl FakeClock {
        fn new() -> Self {
            Self::napping(Duration::ZERO)
        }

        fn napping(naps: Duration) -> Self {
            Self {
                instant: Cell::new(SystemClock.now()),
                wall: Cell::new(SystemClock.timestamp()),
                naps,
            }
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.instant.get()
        }

        fn timestamp(&self) -> Timestamp {
            self.wall.get()
        }

        fn sleep(&self, pause: Duration) {
            self.instant
                .set(self.instant.get().checked_add(pause).unwrap());
            let slept = SignedDuration::try_from(pause.checked_add(self.naps).unwrap()).unwrap();
            self.wall.set(self.wall.get().checked_add(slept).unwrap());
        }
    }

    /// A script as the child, in a folder of its own, that counts its runs
    /// and ends with the code given for each run.
    fn child(codes: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("runs");
        let script = dir.path().join("child.sh");
        File::create_new(&script)
            .unwrap()
            .write_all(
                format!(
                    "#!/bin/sh\necho x >> '{counter}'\nruns=$(wc -l < '{counter}')\n\
                     set -- {codes}\nshift $((runs - 1))\nexit $1\n",
                    counter = counter.display()
                )
                .as_bytes(),
            )
            .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        (dir, script)
    }

    fn runs(dir: &tempfile::TempDir) -> usize {
        fs::read_to_string(dir.path().join("runs"))
            .unwrap()
            .lines()
            .count()
    }

    #[test]
    fn restarts_after_a_failure_and_stops_after_quit() {
        let (dir, script) = child("1 0");
        let clock = FakeClock::new();
        let start = clock.now();
        assert_eq!(supervise(&script, |_| {}, &clock).unwrap(), Ending::Quit);
        assert_eq!(runs(&dir), 2);
        assert_eq!(clock.now().duration_since(start), RESTART_PAUSE);
    }

    #[test]
    fn gives_up_after_failing_too_often() {
        let (dir, script) = child("1 1 1 0");
        let logged = Cell::new(0);
        let log = |_: &str| logged.set(logged.get() + 1);
        assert_eq!(
            supervise(&script, log, &FakeClock::new()).unwrap(),
            Ending::GaveUp
        );
        assert_eq!(runs(&dir), 3);
        assert_eq!(logged.get(), 3, "two restarts and giving up");
    }

    #[test]
    fn failures_far_apart_do_not_add_up() {
        let (dir, script) = child("1 1 1 1 0");
        // Half the window between failures, nearly all of it with the Mac
        // asleep: never three within it.
        let clock = FakeClock::napping(Duration::from_secs(59));
        assert_eq!(supervise(&script, |_| {}, &clock).unwrap(), Ending::Quit);
        assert_eq!(runs(&dir), 5);
    }
}
