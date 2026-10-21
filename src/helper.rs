//! The Swift helpers next to the program in the app bundle. CoreBluetooth
//! and SwiftUI are only comfortable from Swift, so Lampster Remote itself
//! needs no `unsafe`. They talk in JSON lines over stdin and stdout and end
//! when their stdin closes. `scripts/bundle.sh` builds them; with a plain
//! `cargo run` they are missing.

use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};

use crate::process;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Helper {
    Ble,
    Ui,
}

impl Helper {
    #[must_use]
    pub(crate) const fn file_name(self) -> &'static str {
        match self {
            Self::Ble => "lampster-ble",
            Self::Ui => "lampster-ui",
        }
    }

    /// Next to the running program.
    fn path(self) -> Result<PathBuf, HelperError> {
        let executable = std::env::current_exe().map_err(|source| HelperError::Locate {
            helper: self,
            source,
        })?;
        executable
            .parent()
            .map(|dir| dir.join(self.file_name()))
            .filter(|path| path.is_file())
            .ok_or(HelperError::Missing(self))
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HelperError {
    #[error("helper {} is missing", .0.file_name())]
    Missing(Helper),
    #[error("cannot find helper {}: {source}", .helper.file_name())]
    Locate {
        helper: Helper,
        #[source]
        source: io::Error,
    },
    #[error("cannot start helper {}: {source}", .helper.file_name())]
    Spawn {
        helper: Helper,
        #[source]
        source: io::Error,
    },
    #[error("helper {} started without pipes", .0.file_name())]
    NoPipes(Helper),
    #[error("cannot start the reader for helper {}: {source}", .helper.file_name())]
    Thread {
        helper: Helper,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HelperOutput {
    Line(String),
    /// The helper counts as gone afterwards.
    Unreadable(String),
    Gone,
    /// A line the helper wrote to stderr, such as a crash message, for the
    /// log; an app opened from Finder has no terminal to show it.
    Said(String),
}

/// Dropping it stops the helper and joins its reader threads.
pub(crate) struct Running {
    child: Child,
    stdin: ChildStdin,
    reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
}

impl Running {
    /// Passes every line the helper writes, and its end, to `outputs` as
    /// `wrap(helper, output)`.
    pub(crate) fn spawn<Message: Send + 'static>(
        helper: Helper,
        outputs: Sender<Message>,
        wrap: fn(Helper, HelperOutput) -> Message,
    ) -> Result<Self, HelperError> {
        let path = helper.path()?;
        Self::spawn_program(&mut process::command(path), helper, outputs, wrap)
    }

    fn spawn_program<Message: Send + 'static>(
        command: &mut Command,
        helper: Helper,
        outputs: Sender<Message>,
        wrap: fn(Helper, HelperOutput) -> Message,
    ) -> Result<Self, HelperError> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| HelperError::Spawn { helper, source })?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            process::stop(&mut child);
            return Err(HelperError::NoPipes(helper));
        };
        let said = Sender::clone(&outputs);
        let stderr_reader = thread::Builder::new()
            .name(format!("{}-stderr", helper.file_name()))
            .spawn(move || pass_stderr(stderr, helper, &said, wrap));
        let stderr_reader = match stderr_reader {
            Ok(stderr_reader) => stderr_reader,
            Err(source) => {
                process::stop(&mut child);
                return Err(HelperError::Thread { helper, source });
            }
        };
        let reader = thread::Builder::new()
            .name(format!("{}-reader", helper.file_name()))
            .spawn(move || pass_lines(stdout, helper, &outputs, wrap));
        // From here on dropping it stops the helper and joins what runs.
        let mut running = Self {
            child,
            stdin,
            reader: None,
            stderr_reader: Some(stderr_reader),
        };
        match reader {
            Ok(reader) => {
                running.reader = Some(reader);
                Ok(running)
            }
            Err(source) => Err(HelperError::Thread { helper, source }),
        }
    }

    pub(crate) fn send(&mut self, line: &str) -> io::Result<()> {
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()
    }
}

/// Runs on the reader thread until the helper's stdout ends or cannot be
/// read, or nobody receives any more; then reports the helper as gone.
fn pass_lines<Message>(
    stdout: ChildStdout,
    helper: Helper,
    outputs: &Sender<Message>,
    wrap: fn(Helper, HelperOutput) -> Message,
) {
    for line in BufReader::new(stdout).lines() {
        let output = match line {
            Ok(line) => HelperOutput::Line(line),
            Err(error) => HelperOutput::Unreadable(error.to_string()),
        };
        // A line that cannot be read ends the reading: the helper counts
        // as gone afterwards.
        let last = match output {
            HelperOutput::Unreadable(_) => true,
            HelperOutput::Line(_) | HelperOutput::Gone | HelperOutput::Said(_) => false,
        };
        if outputs.send(wrap(helper, output)).is_err() || last {
            break;
        }
    }
    // A receiver that is gone needs no news.
    match outputs.send(wrap(helper, HelperOutput::Gone)) {
        Ok(()) | Err(_) => {}
    }
}

/// Runs on its own thread until the helper's stderr ends; a line that is
/// not UTF-8 is passed with the broken bytes replaced.
fn pass_stderr<Message>(
    stderr: ChildStderr,
    helper: Helper,
    outputs: &Sender<Message>,
    wrap: fn(Helper, HelperOutput) -> Message,
) {
    for line in BufReader::new(stderr).split(b'\n') {
        let Ok(line) = line else {
            // A pipe that cannot be read any more has nothing more to say.
            break;
        };
        let said = HelperOutput::Said(String::from_utf8_lossy(&line).into_owned());
        if outputs.send(wrap(helper, said)).is_err() {
            break;
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        process::stop(&mut self.child);
        // Both readers end with the helper's pipes; one that panicked has
        // nothing left to report.
        for reader in [self.reader.take(), self.stderr_reader.take()]
            .into_iter()
            .flatten()
        {
            match reader.join() {
                Ok(()) | Err(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, SystemClock};
    use std::sync::mpsc;
    use std::time::Duration;

    #[derive(Debug, PartialEq, Eq)]
    struct Received(Helper, HelperOutput);

    fn received(helper: Helper, output: HelperOutput) -> Received {
        Received(helper, output)
    }

    fn shell(script: &str) -> (Running, mpsc::Receiver<Received>) {
        let (sender, receiver) = mpsc::channel();
        let running = Running::spawn_program(
            process::command("/bin/sh").args(["-c", script]),
            Helper::Ble,
            sender,
            received,
        )
        .unwrap();
        (running, receiver)
    }

    fn next(receiver: &mpsc::Receiver<Received>) -> HelperOutput {
        let Received(helper, output) = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(helper, Helper::Ble);
        output
    }

    #[test]
    fn passes_lines_both_ways_and_reports_the_end() {
        let (mut running, receiver) = shell("read line; echo \"got $line\"; echo bye");
        running.send("hello").unwrap();
        assert_eq!(next(&receiver), HelperOutput::Line("got hello".to_owned()));
        assert_eq!(next(&receiver), HelperOutput::Line("bye".to_owned()));
        assert_eq!(next(&receiver), HelperOutput::Gone);
    }

    #[test]
    fn an_unreadable_line_ends_the_reading() {
        let (_running, receiver) = shell(r"printf '\377\nafter\n'");
        assert!(
            matches!(next(&receiver), HelperOutput::Unreadable(_)),
            "not UTF-8"
        );
        assert_eq!(next(&receiver), HelperOutput::Gone);
    }

    #[test]
    fn dropping_stops_a_helper_that_runs_on() {
        let (running, receiver) = shell("echo started; sleep 30");
        assert_eq!(next(&receiver), HelperOutput::Line("started".to_owned()));
        let started = SystemClock.now();
        drop(running);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "drop kills the helper instead of waiting for it"
        );
        assert_eq!(next(&receiver), HelperOutput::Gone);
    }

    #[test]
    fn a_missing_helper_is_reported() {
        let (sender, _receiver) = mpsc::channel();
        let Err(error) = Running::spawn(Helper::Ui, sender, received) else {
            panic!("no helper lies next to the test binary");
        };
        assert!(
            matches!(error, HelperError::Missing(Helper::Ui)),
            "{error:?}"
        );
    }

    #[test]
    fn the_bundle_script_builds_every_helper_under_its_name() {
        let script = include_str!("../scripts/bundle.sh");
        for helper in [Helper::Ble, Helper::Ui] {
            let built = format!("-o \"$APP/Contents/MacOS/{}\"", helper.file_name());
            assert!(script.contains(&built), "{helper:?}");
        }
    }

    #[test]
    fn dropping_stops_a_helper_with_a_child_of_its_own() {
        // The grandchild keeps the helper's stdout open. The shell starts
        // it before "started", so it is there when the group is killed.
        let (running, receiver) = shell("sleep 8 & echo started; wait");
        assert_eq!(next(&receiver), HelperOutput::Line("started".to_owned()));
        let started = SystemClock.now();
        drop(running);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the reader waited for the grandchild"
        );
    }

    #[test]
    fn what_a_helper_says_on_stderr_is_passed_on() {
        let (running, receiver) = shell("echo oops >&2; printf 'bad \\377\\n' >&2; echo bye");
        let mut outputs = Vec::new();
        while outputs.last() != Some(&HelperOutput::Gone) {
            outputs.push(next(&receiver));
        }
        // The stderr reader may still be at it; dropping joins it.
        drop(running);
        outputs.extend(receiver.try_iter().map(|Received(_, output)| output));
        let (said, rest): (Vec<_>, Vec<_>) = outputs
            .into_iter()
            .partition(|output| matches!(output, HelperOutput::Said(_)));
        assert_eq!(
            rest,
            [HelperOutput::Line("bye".to_owned()), HelperOutput::Gone]
        );
        assert_eq!(
            said,
            [
                HelperOutput::Said("oops".to_owned()),
                HelperOutput::Said("bad \u{FFFD}".to_owned())
            ]
        );
    }
}
