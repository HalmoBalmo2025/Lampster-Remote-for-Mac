//! Lampster Remote: a menu bar app that controls The Lampster, a Bluetooth
//! lamp. The program runs twice: as the supervisor ([`watchdog`]) and as
//! its child, which starts the two Swift helpers and feeds what they report
//! to the [`controller`].

mod ble;
mod clock;
mod controller;
mod file;
mod helper;
mod lamp;
mod log;
mod paths;
mod preferences;
mod process;
mod setting;
mod timer;
mod ui;
mod watchdog;

use std::io;
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Instant;

use serde::de::DeserializeOwned;

use clock::{Clock, SystemClock};
use controller::{Controller, Effects, Exit, Flow, Input};
use helper::{Helper, HelperError, HelperOutput, Running};
use log::Log;
use paths::Paths;
use preferences::{Options, Preferences, PreferencesError, Saving, Switch};
use watchdog::Ending;

/// With `panic = "abort"` and no terminal, where a panic happened would
/// be lost; the supervisor's log only shows that the app ended.
fn log_panics(path: Option<&Path>) {
    let log = Log::open(path);
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log.record(&log::panic_line(
            std::thread::current().name(),
            info.location(),
        ));
        default(info);
    }));
}

fn main() -> ExitCode {
    let paths = Paths::of_user();
    let log_path = paths.as_ref().map(|paths| paths.log.as_path());
    let child = std::env::args_os().any(|argument| argument == watchdog::CHILD_ARGUMENT);
    // Before anything opens the log.
    let rotated = match (child, log_path) {
        (false, Some(path)) => log::rotate(path),
        (true, _) | (false, None) => Ok(()),
    };
    log_panics(log_path);
    let log = Log::open(log_path);
    if let Err(error) = rotated {
        log.record(&format!("cannot move the large log aside: {error}"));
    }
    if child {
        return match run(&log, paths) {
            Ok(Exit::Quit) => ExitCode::SUCCESS,
            Ok(Exit::Fail) => ExitCode::FAILURE,
            Err(error) => {
                log.record(&format!("cannot start: {error}"));
                ExitCode::FAILURE
            }
        };
    }
    match supervise(&log) {
        Ok(Ending::Quit) => ExitCode::SUCCESS,
        Ok(Ending::GaveUp) => ExitCode::FAILURE,
        Err(error) => {
            log.record(&format!("supervisor: {error}"));
            ExitCode::FAILURE
        }
    }
}

fn supervise(log: &Log) -> io::Result<Ending> {
    let executable = std::env::current_exe()?;
    watchdog::supervise(&executable, |message| log.record(message), &SystemClock)
}

#[derive(Debug, thiserror::Error)]
enum StartError {
    #[error(transparent)]
    Helper(#[from] HelperError),
    #[error("cannot watch the supervisor: {0}")]
    Watch(#[source] io::Error),
}

fn run(log: &Log, paths: Option<Paths>) -> Result<Exit, StartError> {
    log.record("started");
    let (inputs, received) = mpsc::channel();
    let parent_gone = mpsc::Sender::clone(&inputs);
    let _watch = watchdog::watch_parent(move || {
        // An app that has ended needs no news.
        match parent_gone.send(Input::ParentGone) {
            Ok(()) | Err(_) => {}
        }
    })
    .map_err(StartError::Watch)?;
    let ble = Running::spawn(Helper::Ble, mpsc::Sender::clone(&inputs), ble_input)?;
    let ui = Running::spawn(Helper::Ui, inputs, ui_input)?;
    let Loaded { options, saving } = load_options(log, paths.as_ref());
    let mut live = Live {
        ble,
        ui,
        log,
        paths,
        saving,
    };
    let clock = SystemClock;
    let mut controller = Controller::new(clock.now(), options);
    if log.is_lost() {
        controller.report_lost_log();
    }
    controller.start(&mut live, clock.now());
    loop {
        let Some(input) = next_input(&received, controller.deadline(), clock.now()) else {
            // Every sender is gone, so are both helpers.
            return Ok(Exit::Fail);
        };
        match controller.handle(input, &mut live, clock.now()) {
            Flow::Continue => {}
            Flow::End(exit) => {
                log.record(&format!("ending: {exit:?}"));
                return Ok(exit);
            }
        }
    }
}

/// The settings at start, and whether the preferences file may be written.
struct Loaded {
    options: Options,
    saving: Saving,
}

fn load_options(log: &Log, paths: Option<&Paths>) -> Loaded {
    let Some(paths) = paths else {
        // Without a home folder there is nothing to read, nor to save.
        return Loaded {
            options: Options::default(),
            saving: Saving::Allowed,
        };
    };
    let loaded = preferences::load(&paths.preferences);
    let saving = Saving::after(&loaded);
    let preferences = loaded.unwrap_or_else(|error| {
        log.record(&format!("{error}; the defaults apply and are not saved"));
        Preferences::default()
    });
    Loaded {
        options: Options {
            open_at_login: preferences::open_at_login(&paths.agent),
            off_with_mac: preferences.off_with_mac,
        },
        saving,
    }
}

/// A [`Input::Tick`] when `deadline` passes first; `None` when no input
/// can come any more.
fn next_input(
    received: &Receiver<Input>,
    deadline: Option<Instant>,
    now: Instant,
) -> Option<Input> {
    let Some(deadline) = deadline else {
        return received.recv().ok();
    };
    // A deadline that has passed waits no time.
    match received.recv_timeout(deadline.saturating_duration_since(now)) {
        Ok(input) => Some(input),
        Err(RecvTimeoutError::Timeout) => Some(Input::Tick),
        Err(RecvTimeoutError::Disconnected) => None,
    }
}

fn ble_input(helper: Helper, output: HelperOutput) -> Input {
    input_from(helper, output, Input::Ble)
}

fn ui_input(helper: Helper, output: HelperOutput) -> Input {
    input_from(helper, output, Input::Ui)
}

fn input_from<M: DeserializeOwned>(
    helper: Helper,
    output: HelperOutput,
    wrap: fn(M) -> Input,
) -> Input {
    match output {
        // The log gets the length and the kind of error, not the line.
        HelperOutput::Line(line) => match serde_json::from_str::<M>(&line) {
            Ok(message) => wrap(message),
            Err(error) => Input::Garbage {
                helper,
                detail: garbage(&line, &error),
            },
        },
        HelperOutput::Unreadable(detail) => Input::Garbage { helper, detail },
        HelperOutput::Gone => Input::Gone(helper),
        HelperOutput::Said(line) => Input::Said { helper, line },
    }
}

/// serde's `Display` would repeat the values it could not read.
fn garbage(line: &str, error: &serde_json::Error) -> String {
    let length = line.chars().count();
    let kind = error.classify();
    // serde knows no position for some errors, e.g. in a tagged enum.
    match error.line() {
        0 => format!("{length} characters, {kind:?} error"),
        number => format!(
            "{length} characters, {kind:?} error at line {number} column {}",
            error.column()
        ),
    }
}

struct Live<'log> {
    ble: Running,
    ui: Running,
    log: &'log Log,
    paths: Option<Paths>,
    saving: Saving,
}

impl Effects for Live<'_> {
    fn request(&mut self, request: &ble::Request) -> io::Result<()> {
        let line = serde_json::to_string(request).map_err(io::Error::other)?;
        self.ble.send(&line)
    }

    fn show(&mut self, view: &ui::View) {
        let sent = serde_json::to_string(view)
            .map_err(io::Error::other)
            .and_then(|line| self.ui.send(&line));
        if let Err(error) = sent {
            self.log.record(&format!("cannot reach the panel: {error}"));
        }
    }

    fn log(&mut self, message: &str) {
        self.log.record(message);
    }

    fn set_open_at_login(&mut self, wanted: Switch) -> Result<(), PreferencesError> {
        let paths = self.paths.as_ref().ok_or(PreferencesError::NoHome)?;
        let app = match wanted {
            Switch::On => preferences::running_app()?,
            Switch::Off => None,
        };
        preferences::set_open_at_login(&paths.agent, wanted, app.as_deref())
    }

    fn clock(&mut self) -> jiff::Zoned {
        clock::wall()
    }

    fn save(&mut self, preferences: Preferences) -> Result<(), PreferencesError> {
        let paths = self.paths.as_ref().ok_or(PreferencesError::NoHome)?;
        preferences::save(&paths.preferences, preferences, self.saving)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn waits_for_input_until_the_deadline() {
        let (inputs, received) = mpsc::channel();
        let now = SystemClock.now();
        assert!(
            matches!(next_input(&received, Some(now), now), Some(Input::Tick)),
            "the deadline has come"
        );
        inputs.send(Input::ParentGone).unwrap();
        let later = now.checked_add(Duration::from_secs(60)).unwrap();
        assert!(
            matches!(
                next_input(&received, Some(later), now),
                Some(Input::ParentGone)
            ),
            "an input before the deadline"
        );
        inputs.send(Input::ParentGone).unwrap();
        assert!(
            matches!(next_input(&received, None, now), Some(Input::ParentGone)),
            "an input without a deadline"
        );
        drop(inputs);
        assert!(next_input(&received, Some(later), now).is_none());
        assert!(next_input(&received, None, now).is_none());
    }

    #[test]
    fn garbage_logs_the_length_not_the_line() {
        let line = r#"{"command":"power","power":"private words"}"#;
        let Input::Garbage { helper: _, detail } =
            ui_input(Helper::Ui, HelperOutput::Line(line.to_owned()))
        else {
            panic!("garbage expected");
        };
        assert_eq!(detail, "43 characters, Data error");
        let Input::Garbage {
            helper: _,
            detail: cut_off,
        } = ui_input(Helper::Ui, HelperOutput::Line("{".to_owned()))
        else {
            panic!("garbage expected");
        };
        assert_eq!(cut_off, "1 characters, Eof error at line 1 column 1");
    }

    #[test]
    fn a_preferences_file_that_failed_to_load_is_not_saved_over() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_library(dir.path());
        file::replace(&paths.preferences, b"{broken").unwrap();
        let Loaded { options: _, saving } = load_options(&Log::open(None), Some(&paths));
        assert_eq!(saving, Saving::Blocked);
    }
}
