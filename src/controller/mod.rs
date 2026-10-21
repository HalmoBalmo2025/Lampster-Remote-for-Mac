//! The one owner of the connection, the lamp's state and the settings.
//! Everything arrives as an [`Input`]; the answers go out through
//! [`Effects`], which the tests fake. Only one write to the lamp is under
//! way at a time, and the next one heads for whatever is set by then, so a
//! moving slider never queues up writes.
//!
//! The parts: the connection ([`link`]), the writes and their answers while
//! connected ([`session`]), changes the lamp reports ([`notifications`]),
//! heat protection ([`heat`]) and what the panel shows ([`view`]).

mod heat;
mod link;
mod notifications;
mod session;
mod view;

use std::fmt;
use std::io;
use std::time::{Duration, Instant};

use jiff::{Timestamp, Zoned};

use crate::ble::{self, Radio, Request, RequestId};
use crate::helper::Helper;
use crate::preferences::{Options, Preferences, PreferencesError, Switch};
use crate::setting::{Power, Settings};
use crate::timer::{self, TimeError, Timer, TimerKind, Timers};
use crate::ui::{self, View};

use heat::COOL_BRIGHTNESS;
use link::{Link, Phase};
use session::Activity;

/// One lockout and a write. At shutdown the panel holds macOS back for
/// `quitWait` (helpers/ui/AppDelegate.swift), which must be longer: if the
/// panel ends first, the lamp may stay on.
const QUIT_GRACE: Duration = Duration::from_secs(4);

#[derive(Debug)]
pub(crate) enum Input {
    Ble(ble::Event),
    Ui(ui::Command),
    /// A helper wrote something that is not a message.
    Garbage {
        helper: Helper,
        detail: String,
    },
    /// A helper wrote `line` to stderr.
    Said {
        helper: Helper,
        line: String,
    },
    Gone(Helper),
    /// The supervisor ended (logout, or it was stopped).
    ParentGone,
    /// [`Controller::deadline`] has passed.
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    End(Exit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    Quit,
    /// The supervisor starts Lampster Remote again.
    Fail,
}

pub(crate) trait Effects {
    /// Fails when the helper no longer reads; [`Input::Gone`] follows.
    fn request(&mut self, request: &Request) -> io::Result<()>;
    fn show(&mut self, view: &View);
    fn log(&mut self, message: &str);
    fn set_open_at_login(&mut self, wanted: Switch) -> Result<(), PreferencesError>;
    fn save(&mut self, preferences: Preferences) -> Result<(), PreferencesError>;
    fn clock(&mut self) -> Zoned;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    Running,
    /// The lamp is being switched off; ends when it is dark or at `until`.
    Quitting {
        until: Instant,
    },
}

/// Whose light the lamp is to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Aim {
    /// What it shows: the settings follow the lamp (at connecting, its
    /// button), and its levels do not always lead back to the same
    /// setting, so nothing is written.
    Lamp,
    /// The settings: the user, heat protection or quitting changed them.
    Settings,
}

/// A sentence of the panel. The panel shows every one that holds, in the
/// order declared here (`Ord`), so none hides another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Problem {
    /// The answer to the user's last command, until the next one.
    Reply(Reply),
    /// What heat protection did, until the light is changed.
    Heat(HeatAction),
    /// The lamp refused writes of one kind too often; read from the
    /// session, so it ends with the refusals.
    Refused,
    /// The controller gave up on the connection several times in a row;
    /// read from the give-ups.
    Unstable,
    /// There is no log file to write to, for the whole run.
    LogLost,
}

/// A command of the panel that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Reply {
    /// A setting could not be changed; the log has the error.
    SettingFailed(Setting),
    /// Opening at login was asked for outside of the app bundle.
    NotBundled,
    /// The time of a timer could not be worked out; the log has why.
    TimeUnknown,
}

/// What heat protection did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum HeatAction {
    /// The lamp got warm and was dimmed to [`COOL_BRIGHTNESS`].
    Dimmed,
    /// The lamp got too hot and was switched off.
    SwitchedOff,
}

/// Where the log goes, for the whole run.
#[derive(Debug, Clone, Copy)]
enum LogFile {
    /// Into its file, or nowhere while it is switched off: nothing to tell.
    Written,
    /// To stderr, which nobody sees.
    Lost,
}

/// The settings of the app itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Setting {
    OpenAtLogin,
    OffWithMac,
}

/// The sentence the panel shows.
impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Reply(Reply::SettingFailed(Setting::OpenAtLogin)) => {
                "Could not change opening at login. If the log is on, it has the details."
            }
            Self::Reply(Reply::SettingFailed(Setting::OffWithMac)) => {
                "Could not save the setting. If the log is on, it has the details."
            }
            Self::Reply(Reply::NotBundled) => {
                "Opening at login works only for Lampster Remote.app."
            }
            Self::Reply(Reply::TimeUnknown) => {
                "Could not work out when the timer goes off. If the log is on, it has the details."
            }
            Self::Heat(HeatAction::Dimmed) => {
                return write!(
                    f,
                    "Lampster got warm; brightness lowered to {} %.",
                    u8::from(COOL_BRIGHTNESS)
                );
            }
            Self::Heat(HeatAction::SwitchedOff) => "Lampster got too hot and was switched off.",
            Self::Refused => "Lampster did not accept the change. Try again.",
            Self::Unstable => "Lampster keeps losing the connection. Lampster Remote keeps trying.",
            Self::LogLost => "Lampster Remote cannot write its log in ~/Library/Logs.",
        })
    }
}

pub(crate) struct Controller {
    settings: Settings,
    aim: Aim,
    /// Give-ups on the connection in a row.
    give_ups: u8,
    radio: Radio,
    link: Link,
    next_id: RequestId,
    reply: Option<Reply>,
    heat: Option<HeatAction>,
    /// To send the panel only changes.
    shown: Option<View>,
    /// Lines read from the panel ([`View::commands`]).
    commands: u64,
    log: LogFile,
    run: Run,
    options: Options,
    timers: Timers,
}

impl Controller {
    #[must_use]
    pub(crate) fn new(now: Instant, options: Options) -> Self {
        Self {
            settings: Settings::default(),
            aim: Aim::Lamp,
            give_ups: 0,
            radio: Radio::Unknown,
            link: Link::new(now),
            next_id: RequestId::FIRST,
            reply: None,
            heat: None,
            shown: None,
            commands: 0,
            log: LogFile::Written,
            run: Run::Running,
            options,
            timers: Timers::UNSET,
        }
    }

    pub(crate) fn start(&mut self, effects: &mut impl Effects, now: Instant) {
        self.advance(effects, now);
        self.show(effects);
    }

    /// The log goes nowhere the user can read it.
    pub(crate) const fn report_lost_log(&mut self) {
        self.log = LogFile::Lost;
    }

    /// When the controller wants an [`Input::Tick`].
    #[must_use]
    pub(crate) fn deadline(&self) -> Option<Instant> {
        let due = self.link_deadline();
        match self.run {
            Run::Running => due,
            Run::Quitting { until } => Some(due.map_or(until, |due| due.min(until))),
        }
    }

    pub(crate) fn handle(
        &mut self,
        input: Input,
        effects: &mut impl Effects,
        now: Instant,
    ) -> Flow {
        self.count_panel_line(&input);
        let flow = match input {
            Input::Ble(event) => {
                self.on_ble(event, effects, now);
                Flow::Continue
            }
            Input::Ui(command) => self.on_ui(command, effects, now),
            Input::Garbage { helper, detail } => {
                effects.log(&format!("{} wrote garbage: {detail}", helper.file_name()));
                Flow::Continue
            }
            Input::Said { helper, line } => {
                effects.log(&format!("{} said: {line}", helper.file_name()));
                Flow::Continue
            }
            Input::Gone(helper) => {
                effects.log(&format!("{} has ended", helper.file_name()));
                match self.run {
                    Run::Running => Flow::End(Exit::Fail),
                    // The user quit: the supervisor must not start the app
                    // again.
                    Run::Quitting { until: _ } => Flow::End(Exit::Quit),
                }
            }
            // At logout the supervisor may end before the panel is asked
            // to quit; `bundle.sh` stops it to install a new build.
            Input::ParentGone => {
                effects.log("the supervisor has ended");
                match self.run {
                    Run::Running => self.shut_down(now),
                    // A quit under way switches the lamp off first.
                    Run::Quitting { until: _ } => Flow::Continue,
                }
            }
            Input::Tick => Flow::Continue,
        };
        match flow {
            Flow::End(_) => return flow,
            Flow::Continue => {}
        }
        self.advance(effects, now);
        self.show(effects);
        match self.run {
            Run::Quitting { until } if now >= until || self.is_dark_or_unreachable() => {
                Flow::End(Exit::Quit)
            }
            Run::Running | Run::Quitting { until: _ } => Flow::Continue,
        }
    }

    /// The panel gets a sentence, the log the error.
    fn report(&mut self, effects: &mut impl Effects, setting: Setting, error: &PreferencesError) {
        effects.log(&format!("{setting:?}: {error}"));
        self.reply = Some(match error {
            PreferencesError::NotBundled => Reply::NotBundled,
            PreferencesError::NoHome
            | PreferencesError::NoExecutable(_)
            | PreferencesError::Read { file: _, source: _ }
            | PreferencesError::Write { file: _, source: _ }
            | PreferencesError::Newer(_)
            | PreferencesError::NotLoaded
            | PreferencesError::Remove { file: _, source: _ } => Reply::SettingFailed(setting),
        });
    }

    /// Whether quitting can end.
    fn is_dark_or_unreachable(&self) -> bool {
        let session = match &self.link {
            Link::Searching { wish: _, phase } => {
                return match phase {
                    Phase::Waiting { until: _ }
                    | Phase::Connecting {
                        request: _,
                        since: _,
                        restarts: _,
                    } => true,
                    // Connected: soon it can be switched off.
                    Phase::Reading {
                        request: _,
                        since: _,
                        progress: _,
                    } => false,
                };
            }
            Link::Ready(session) => session,
        };
        match session.activity {
            Activity::Writing(_) => false,
            Activity::Idle => {
                session.mode.gave_up() || session.next_step(self.settings, self.aim).is_none()
            }
        }
    }

    fn on_ui(&mut self, command: ui::Command, effects: &mut impl Effects, now: Instant) -> Flow {
        match self.run {
            Run::Running => {}
            Run::Quitting { until: _ } => return Self::while_quitting(command, effects),
        }
        // Each command of the user gets its own answer.
        match command.sender() {
            ui::Sender::User => self.reply = None,
            ui::Sender::Panel => {}
        }
        let settings = &mut self.settings;
        match command {
            ui::Command::Power { power } => settings.power = power,
            ui::Command::Away => {
                effects.log("the Mac locks or sleeps");
                if self.options.off_with_mac == Switch::Off {
                    return Flow::Continue;
                }
                settings.power = Power::Off;
            }
            ui::Command::Mode { mode } => settings.mode = mode,
            ui::Command::White { white, brightness } => settings.white_on(white, brightness),
            ui::Command::Color { color, brightness } => settings.color_on(color, brightness),
            ui::Command::Quit => self.quit(now),
            ui::Command::Shutdown => {
                effects.log("the Mac shuts down, restarts or logs out");
                return self.shut_down(now);
            }
            ui::Command::OpenAtLogin { open_at_login } => {
                self.set_open_at_login(open_at_login, effects);
                return Flow::Continue;
            }
            ui::Command::Wake { at } => {
                let at = timer::next(at, &effects.clock());
                self.set_timer_at(TimerKind::Wake, at, effects);
                return Flow::Continue;
            }
            ui::Command::Sleep { minutes } => {
                let at = timer::after(minutes, &effects.clock());
                self.set_timer_at(TimerKind::Sleep, at, effects);
                return Flow::Continue;
            }
            ui::Command::WakeOff => {
                self.set_timer(TimerKind::Wake, None);
                return Flow::Continue;
            }
            ui::Command::SleepOff => {
                self.set_timer(TimerKind::Sleep, None);
                return Flow::Continue;
            }
            ui::Command::OffWithMac { off_with_mac } => {
                self.set_off_with_mac(off_with_mac, effects);
                return Flow::Continue;
            }
            ui::Command::Log { message } => {
                effects.log(&format!("panel: {message}"));
                return Flow::Continue;
            }
        }
        self.user_changed_light();
        Flow::Continue
    }

    /// A time that could not be worked out leaves the timer as it was.
    fn set_timer_at(
        &mut self,
        kind: TimerKind,
        at: Result<Timestamp, TimeError>,
        effects: &mut impl Effects,
    ) {
        match at {
            Ok(at) => self.set_timer(kind, Some(at)),
            Err(error) => {
                effects.log(&format!("{kind:?} timer: {error}"));
                self.reply = Some(Reply::TimeUnknown);
            }
        }
    }

    /// `at` `None` switches the timer off. A timer the lamp refused before
    /// is tried again.
    fn set_timer(&mut self, kind: TimerKind, at: Option<Timestamp>) {
        *self.timers.get_mut(kind) = Timer::set(at);
        self.link.user_set_timer();
    }

    /// Counts the lines read from the panel ([`View::commands`]).
    fn count_panel_line(&mut self, input: &Input) {
        let from_panel = match input {
            Input::Ui(_)
            | Input::Garbage {
                helper: Helper::Ui,
                detail: _,
            } => true,
            Input::Garbage {
                helper: Helper::Ble,
                detail: _,
            }
            | Input::Ble(_)
            | Input::Said { helper: _, line: _ }
            | Input::Gone(_)
            | Input::ParentGone
            | Input::Tick => false,
        };
        if from_panel {
            // A count of lines; it does not reach the end of a u64.
            self.commands = self.commands.saturating_add(1);
        }
    }

    /// Quitting switches the lamp off; the panel must not switch it on,
    /// nor change anything else on the way out.
    fn while_quitting(command: ui::Command, effects: &mut impl Effects) -> Flow {
        match command {
            ui::Command::Log { message } => effects.log(&format!("panel: {message}")),
            ui::Command::Power { power: _ }
            | ui::Command::Mode { mode: _ }
            | ui::Command::White {
                white: _,
                brightness: _,
            }
            | ui::Command::Color {
                color: _,
                brightness: _,
            }
            | ui::Command::Quit
            | ui::Command::Shutdown
            | ui::Command::Away
            | ui::Command::OpenAtLogin { open_at_login: _ }
            | ui::Command::Wake { at: _ }
            | ui::Command::WakeOff
            | ui::Command::Sleep { minutes: _ }
            | ui::Command::SleepOff
            | ui::Command::OffWithMac { off_with_mac: _ } => {}
        }
        Flow::Continue
    }

    /// The settings count now; what heat protection did is over.
    fn user_changed_light(&mut self) {
        self.heat = None;
        self.aim = Aim::Settings;
        self.link.user_changed();
    }

    /// Switches the lamp off; Lampster Remote ends once it is dark, or
    /// after [`QUIT_GRACE`].
    /// A second quit keeps the time the first one set. Like a change by
    /// the user, it counts also after a connect still under way.
    fn quit(&mut self, now: Instant) {
        self.settings.power = Power::Off;
        self.user_changed_light();
        match self.run {
            Run::Running => {
                self.run = Run::Quitting {
                    until: later(now, QUIT_GRACE),
                };
            }
            Run::Quitting { until: _ } => {}
        }
    }

    /// The Mac shuts down, or the supervisor ended: the lamp goes off with
    /// the Mac if the user wants that.
    fn shut_down(&mut self, now: Instant) -> Flow {
        match self.options.off_with_mac {
            Switch::Off => Flow::End(Exit::Quit),
            Switch::On => {
                self.quit(now);
                Flow::Continue
            }
        }
    }

    fn set_open_at_login(&mut self, wanted: Switch, effects: &mut impl Effects) {
        match effects.set_open_at_login(wanted) {
            Ok(()) => self.options.open_at_login = wanted,
            Err(error) => self.report(effects, Setting::OpenAtLogin, &error),
        }
    }

    fn set_off_with_mac(&mut self, wanted: Switch, effects: &mut impl Effects) {
        match effects.save(Preferences {
            off_with_mac: wanted,
        }) {
            Ok(()) => self.options.off_with_mac = wanted,
            Err(error) => self.report(effects, Setting::OffWithMac, &error),
        }
    }
}

fn later(now: Instant, pause: Duration) -> Instant {
    let later = now.checked_add(pause);
    debug_assert!(
        later.is_some(),
        "invariant: pauses are seconds, far below what an Instant holds"
    );
    // Should it ever overflow, not waiting is the least harm.
    later.unwrap_or(now)
}

#[cfg(test)]
mod tests;
