//! What the lamp reports by itself: its button, a timer that fired, its
//! temperature, and the echoes of Lampster Remote's own writes.

#![deny(clippy::arithmetic_side_effects)]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::lamp::plan::Step;
use crate::lamp::{Channel, Characteristic, ColorLevels, ModeReading, WhiteLevels};
use crate::setting::Power;
use crate::timer::{Countdown, Sync, Timer, TimerKind};

use super::link::{Link, Phase, Progress, Wish};
use super::{Aim, Controller, Effects, Run};

/// The lamp notifies the values Lampster Remote writes too, sometimes
/// seconds later; a notification that matches a write this recent is its
/// echo, not a change at the lamp.
const ECHO_WINDOW: Duration = Duration::from_secs(5);

/// The writes of the last [`ECHO_WINDOW`], to tell their echoes from
/// changes at the lamp.
#[derive(Debug, Default)]
pub(super) struct Echoes(VecDeque<Echo>);

#[derive(Debug)]
struct Echo {
    written: Instant,
    step: Step,
}

impl Echo {
    fn is_recent(&self, now: Instant) -> bool {
        // A `now` before the write counts as no time.
        now.saturating_duration_since(self.written) < ECHO_WINDOW
    }
}

impl Echoes {
    pub(super) fn record(&mut self, step: Step, now: Instant) {
        self.0.retain(|echo| echo.is_recent(now));
        self.0.push_back(Echo { written: now, step });
    }

    /// The lamp refused `step`: it sends no echo of it.
    pub(super) fn forget(&mut self, step: Step) {
        if let Some(index) = self.0.iter().rposition(|echo| echo.step == step) {
            self.0.remove(index);
        }
    }

    /// Whether a notification is the echo of a recent write. Each write
    /// has one echo, so a match uses it up. A notification that is no echo
    /// is a change at the lamp, after which the older writes to that
    /// characteristic have no echo left to tell apart.
    fn take(&mut self, characteristic: Characteristic, value: &[u8], now: Instant) -> bool {
        let found = self.0.iter().position(|echo| {
            echo.is_recent(now)
                && echo.step.characteristic() == characteristic
                && echo.step.bytes() == value
        });
        let Some(index) = found else {
            self.0
                .retain(|echo| echo.step.characteristic() != characteristic);
            return false;
        };
        self.0.remove(index).is_some()
    }
}

impl Controller {
    /// Someone used the lamp's button: the lamp's state becomes the
    /// settings, so the panel shows it and nothing is written back.
    /// While quitting, the lamp is switched off whatever the button did.
    pub(super) fn on_changed(
        &mut self,
        characteristic: Characteristic,
        value: &[u8],
        effects: &mut impl Effects,
        now: Instant,
    ) {
        let session = match &mut self.link {
            Link::Searching { wish, phase } => {
                let wish = *wish;
                return match phase {
                    Phase::Waiting { until: _ }
                    | Phase::Connecting {
                        request: _,
                        since: _,
                        restarts: _,
                    } => {}
                    Phase::Reading {
                        request: _,
                        since: _,
                        progress: _,
                    } => self.on_changed_while_reading(characteristic, wish, effects, now),
                };
            }
            Link::Ready(session) => session,
        };
        if session.echoes.take(characteristic, value, now) {
            return;
        }
        let last = match characteristic {
            Characteristic::Temperature => return self.on_temperature(value, effects),
            Characteristic::WakeTimer => {
                return self.on_timer_changed(TimerKind::Wake, value, effects);
            }
            Characteristic::SleepTimer => {
                return self.on_timer_changed(TimerKind::Sleep, value, effects);
            }
            Characteristic::Mode => {
                let Some(reading) = ModeReading::from_bytes(value) else {
                    return effects.log(&format!("strange mode change {value:02x?}"));
                };
                session.lamp.output = reading.output;
                session.mode.lock(now);
                reading.last
            }
            Characteristic::White => {
                let Some(levels) = WhiteLevels::from_bytes(value) else {
                    return effects.log(&format!("strange white change {value:02x?}"));
                };
                session.lamp.white = levels;
                Channel::from(self.settings.mode)
            }
            Characteristic::Color => {
                let Some(levels) = ColorLevels::from_bytes(value) else {
                    return effects.log(&format!("strange colour change {value:02x?}"));
                };
                session.lamp.color = levels;
                Channel::from(self.settings.mode)
            }
        };
        self.settings.follow(session.lamp, last);
        match self.run {
            Run::Running => self.aim = Aim::Lamp,
            // Quitting switches the lamp off, whatever the button does.
            Run::Quitting { until: _ } => self.settings.power = Power::Off,
        }
        // A change at the lamp: what was refused may be taken now.
        session.forget_refusals();
        // The lamp's light changed: what heat protection did is over.
        self.heat = None;
    }

    /// The reads so far may be out of date; they start over.
    fn on_changed_while_reading(
        &mut self,
        characteristic: Characteristic,
        wish: Wish,
        effects: &mut impl Effects,
        now: Instant,
    ) {
        match characteristic {
            Characteristic::Mode | Characteristic::White | Characteristic::Color => {
                effects.log(&format!(
                    "{characteristic:?} changed while reading: reading again"
                ));
                self.read(effects, now, Progress::Mode, wish);
            }
            Characteristic::WakeTimer
            | Characteristic::SleepTimer
            | Characteristic::Temperature => {}
        }
    }

    /// The lamp reports 0 when a timer fires; a change made meanwhile
    /// stays.
    fn on_timer_changed(&mut self, kind: TimerKind, value: &[u8], effects: &mut impl Effects) {
        let timer = self.timers.get_mut(kind);
        match timer.sync {
            Sync::Done => match Countdown::from_bytes(value) {
                Some(Countdown::Stopped) => {
                    effects.log(&format!("the {kind:?} timer fired"));
                    *timer = Timer::UNSET;
                }
                Some(Countdown::Seconds(_)) | None => {}
            },
            Sync::Needed | Sync::Writing => {}
        }
    }
}
