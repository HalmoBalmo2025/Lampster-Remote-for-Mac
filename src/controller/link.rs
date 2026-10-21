//! The connection: searching for the lamp, the reads after connecting, and
//! losing it again.

#![deny(clippy::arithmetic_side_effects)]

use std::time::{Duration, Instant};

use crate::ble::{self, Request, RequestId};
use crate::lamp::{Characteristic, ColorLevels, Lamp, ModeReading, WhiteLevels};
use crate::setting::Settings;
use crate::timer::Countdown;

use super::session::{Session, drop_overdue};
use super::{Aim, Controller, Effects, Run, later};

/// An unanswered request after this long counts as a lost connection.
pub(super) const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);
/// The pause before connecting again; it doubles with every give-up in a
/// row, up to [`LONGEST_RETRY_PAUSE`].
const RETRY_PAUSE: Duration = Duration::from_secs(2);
const LONGEST_RETRY_PAUSE: Duration = Duration::from_secs(30);
/// Give-ups in a row after which the panel tells the user; the controller
/// keeps trying, as the lamp may come back any time.
pub(super) const GIVE_UPS_TOLD: u8 = 3;
/// A connection that lasted this long worked: the give-ups before it are
/// forgotten.
const STABLE: Duration = Duration::from_secs(60);

/// How a connection ended.
#[derive(Debug, Clone, Copy)]
pub(super) enum Loss {
    /// The controller gave up: a request failed or was not answered.
    GaveUp,
    /// It dropped: the lamp went away, or Bluetooth.
    Dropped,
}

/// The pause before connecting again after `give_ups` in a row.
fn retry_pause(give_ups: u8) -> Duration {
    // No give-up and the first one wait the same.
    let doublings = u32::from(give_ups.saturating_sub(1));
    // Past the longest pause, more doublings change nothing.
    RETRY_PAUSE
        .checked_mul(2_u32.saturating_pow(doublings))
        .map_or(LONGEST_RETRY_PAUSE, |pause| pause.min(LONGEST_RETRY_PAUSE))
}

/// macOS sometimes stops reporting the lamp after the connection was lost;
/// a fresh search finds it, like switching Bluetooth off and on does.
const SEARCH_RESTART: Duration = Duration::from_secs(30);

/// Every loss goes back to searching, and a new attempt follows.
#[derive(Debug)]
pub(super) enum Link {
    /// Not connected yet; `wish` says whose state counts once it is.
    Searching {
        wish: Wish,
        phase: Phase,
    },
    Ready(Session),
}

/// The steps before a connection is ready.
#[derive(Debug, Clone, Copy)]
pub(super) enum Phase {
    /// Until the next attempt.
    Waiting { until: Instant },
    /// The helper searches until the lamp shows up.
    Connecting {
        request: RequestId,
        since: Instant,
        /// For the log.
        restarts: u32,
    },
    /// Connected; reading what the lamp does.
    Reading {
        request: RequestId,
        since: Instant,
        progress: Progress,
    },
}

/// Whose state counts once the reads after connecting are done.
#[derive(Debug, Clone, Copy)]
pub(super) enum Wish {
    /// The lamp's; it may have been switched at the plug meanwhile.
    Adopt,
    /// The settings'; the user changed something.
    Apply,
}

/// The reads after connecting, in this order.
#[derive(Debug, Clone, Copy)]
pub(super) enum Progress {
    Mode,
    White {
        mode: ModeReading,
    },
    Color {
        mode: ModeReading,
        white: WhiteLevels,
    },
    WakeTimer {
        mode: ModeReading,
        lamp: Lamp,
    },
    SleepTimer {
        mode: ModeReading,
        lamp: Lamp,
        wake_left: Countdown,
    },
}

/// Everything the reads after connecting found.
#[derive(Debug, Clone, Copy)]
struct Readings {
    mode: ModeReading,
    lamp: Lamp,
    wake_left: Countdown,
    sleep_left: Countdown,
}

/// Where a read after connecting leads.
enum Read {
    Next(Progress),
    Done(Readings),
}

impl Progress {
    /// Takes the value this step read. A value that makes no sense is
    /// logged and taken as no light and no timer.
    fn take(self, value: &[u8], effects: &mut impl Effects) -> Read {
        match self {
            Self::Mode => {
                let mode = ModeReading::from_bytes(value).unwrap_or_else(|| {
                    effects.log(&format!("strange mode value {value:02x?}"));
                    ModeReading::UNKNOWN
                });
                Read::Next(Self::White { mode })
            }
            Self::White { mode } => {
                let white = WhiteLevels::from_bytes(value).unwrap_or_else(|| {
                    effects.log(&format!("strange white value {value:02x?}"));
                    WhiteLevels::default()
                });
                Read::Next(Self::Color { mode, white })
            }
            Self::Color { mode, white } => {
                let color = ColorLevels::from_bytes(value).unwrap_or_else(|| {
                    effects.log(&format!("strange colour value {value:02x?}"));
                    ColorLevels::default()
                });
                let lamp = Lamp {
                    output: mode.output,
                    white,
                    color,
                };
                Read::Next(Self::WakeTimer { mode, lamp })
            }
            Self::WakeTimer { mode, lamp } => Read::Next(Self::SleepTimer {
                mode,
                lamp,
                wake_left: timer_left(value, effects),
            }),
            Self::SleepTimer {
                mode,
                lamp,
                wake_left,
            } => Read::Done(Readings {
                mode,
                lamp,
                wake_left,
                sleep_left: timer_left(value, effects),
            }),
        }
    }

    /// What this step reads.
    const fn characteristic(self) -> Characteristic {
        match self {
            Self::Mode => Characteristic::Mode,
            Self::White { mode: _ } => Characteristic::White,
            Self::Color { mode: _, white: _ } => Characteristic::Color,
            Self::WakeTimer { mode: _, lamp: _ } => Characteristic::WakeTimer,
            Self::SleepTimer {
                mode: _,
                lamp: _,
                wake_left: _,
            } => Characteristic::SleepTimer,
        }
    }
}

impl Link {
    /// At start the lamp's state counts.
    pub(super) const fn new(now: Instant) -> Self {
        Self::Searching {
            wish: Wish::Adopt,
            phase: Phase::Waiting { until: now },
        }
    }

    pub(super) const fn session(&self) -> Option<&Session> {
        match self {
            Self::Ready(session) => Some(session),
            Self::Searching { wish: _, phase: _ } => None,
        }
    }

    /// The user changed something: the settings count after the next
    /// connect, and a refused write, of the light or a timer, is tried
    /// again.
    pub(super) const fn user_changed(&mut self) {
        match self {
            Self::Searching { wish, phase: _ } => *wish = Wish::Apply,
            Self::Ready(session) => session.forget_refusals(),
        }
    }

    /// Whether it is a connection that has lasted `long`.
    fn lasted(&self, now: Instant, long: Duration) -> bool {
        match self {
            Self::Searching { wish: _, phase: _ } => false,
            // A `now` before it began counts as no time.
            Self::Ready(session) => now.saturating_duration_since(session.since) >= long,
        }
    }

    /// The user set a timer: one the lamp refused is tried again.
    pub(super) const fn user_set_timer(&mut self) {
        match self {
            Self::Searching { wish: _, phase: _ } => {}
            Self::Ready(session) => session.timers.forget(),
        }
    }

    /// A lamp that had reached the settings may be switched at the plug
    /// before the next connect; one that had not still gets them.
    fn wish_after_loss(&self, settings: Settings, aim: Aim) -> Wish {
        match self {
            Self::Searching { wish, phase: _ } => *wish,
            Self::Ready(session) => {
                if !session.writes_light() && session.next_step(settings, aim).is_none() {
                    Wish::Adopt
                } else {
                    Wish::Apply
                }
            }
        }
    }
}

impl Controller {
    pub(super) fn link_deadline(&self) -> Option<Instant> {
        match &self.link {
            Link::Searching { wish: _, phase } => match *phase {
                Phase::Waiting { until } => Some(until),
                Phase::Connecting {
                    since,
                    request: _,
                    restarts: _,
                } => since.checked_add(SEARCH_RESTART),
                Phase::Reading {
                    since,
                    request: _,
                    progress: _,
                } => since.checked_add(ANSWER_TIMEOUT),
            },
            Link::Ready(session) => {
                let write = session.deadline(self.settings, self.aim, self.timers.needed());
                // When the give-ups before it are forgotten.
                let stable = match self.give_ups {
                    0 => None,
                    _ => session.since.checked_add(STABLE),
                };
                write.into_iter().chain(stable).min()
            }
        }
    }

    pub(super) fn on_ble(&mut self, event: ble::Event, effects: &mut impl Effects, now: Instant) {
        match event {
            ble::Event::Bluetooth { state } => {
                if state != self.radio {
                    effects.log(&format!("Bluetooth: {state:?}"));
                }
                self.radio = state;
            }
            ble::Event::Connected { id } => self.on_connected(id, effects, now),
            ble::Event::Value { id, value } => self.on_value(id, &value, effects, now),
            ble::Event::Changed {
                characteristic,
                value,
            } => self.on_changed(characteristic, &value, effects, now),
            ble::Event::Written { id } => self.on_written(id, now),
            ble::Event::Rejected { id } => self.on_rejected(id, effects, now),
            ble::Event::Failed {
                id: Some(id),
                reason,
            } => {
                if self.awaits(id) {
                    self.lose(effects, now, &format!("request failed: {reason}"));
                } else {
                    effects.log(&format!("an old request failed: {reason}"));
                }
            }
            ble::Event::Failed { id: None, reason } => {
                effects.log(&format!("Bluetooth helper: {reason}"));
            }
            ble::Event::Unsubscribed {
                characteristic,
                reason,
            } => self.on_unsubscribed(characteristic, &reason, effects, now),
            ble::Event::Disconnected { reason } => {
                let reason = reason.unwrap_or_else(|| "no reason given".to_owned());
                effects.log(&format!("disconnected: {reason}"));
                self.wait_to_retry(now, Loss::Dropped);
            }
        }
    }

    fn on_connected(&mut self, id: RequestId, effects: &mut impl Effects, now: Instant) {
        let Link::Searching {
            wish,
            phase:
                Phase::Connecting {
                    request,
                    restarts,
                    since: _,
                },
        } = self.link
        else {
            return;
        };
        if request != id {
            return;
        }
        if restarts > 0 {
            effects.log(&format!("found after {restarts} new searches"));
        }
        self.read(effects, now, Progress::Mode, wish);
    }

    /// Without the lamp's changes the controller cannot follow its button,
    /// nor protect it from heat: a connection that lacks them is given up
    /// and made again.
    fn on_unsubscribed(
        &mut self,
        characteristic: Characteristic,
        reason: &str,
        effects: &mut impl Effects,
        now: Instant,
    ) {
        let connected = match self.link {
            Link::Searching { wish: _, phase } => match phase {
                Phase::Waiting { until: _ }
                | Phase::Connecting {
                    request: _,
                    since: _,
                    restarts: _,
                } => false,
                Phase::Reading {
                    request: _,
                    since: _,
                    progress: _,
                } => true,
            },
            Link::Ready(_) => true,
        };
        if connected {
            self.lose(
                effects,
                now,
                &format!("cannot follow {characteristic:?}: {reason}"),
            );
        } else {
            effects.log(&format!("an old subscription to {characteristic:?} failed"));
        }
    }

    fn awaits(&self, id: RequestId) -> bool {
        match &self.link {
            Link::Searching { wish: _, phase } => match *phase {
                Phase::Waiting { until: _ } => false,
                Phase::Connecting {
                    request,
                    since: _,
                    restarts: _,
                }
                | Phase::Reading {
                    request,
                    since: _,
                    progress: _,
                } => request == id,
            },
            Link::Ready(session) => session.writing(id).is_some(),
        }
    }

    fn on_value(&mut self, id: RequestId, value: &[u8], effects: &mut impl Effects, now: Instant) {
        let Link::Searching {
            wish,
            phase:
                Phase::Reading {
                    request,
                    progress,
                    since: _,
                },
        } = self.link
        else {
            return;
        };
        if request != id {
            return;
        }
        match progress.take(value, effects) {
            Read::Next(progress) => self.read(effects, now, progress, wish),
            Read::Done(readings) => self.ready(readings, wish, effects, now),
        }
    }

    fn ready(&mut self, readings: Readings, wish: Wish, effects: &mut impl Effects, now: Instant) {
        let Readings {
            mode,
            lamp,
            wake_left,
            sleep_left,
        } = readings;
        let clock = effects.clock();
        drop_overdue(&mut self.timers, effects, &clock);
        self.timers.wake.reconcile(wake_left, &clock);
        self.timers.sleep.reconcile(sleep_left, &clock);
        effects.log(&format!("connected: {lamp:?}"));
        match wish {
            Wish::Adopt => {
                debug_assert!(
                    self.run == Run::Running,
                    "invariant: quitting counts as a change by the user, so the wish is to apply"
                );
                self.settings.follow(lamp, mode.last);
                self.aim = Aim::Lamp;
            }
            Wish::Apply => {}
        }
        self.link = Link::Ready(Session::new(lamp, now));
    }

    /// Connects, notices a missing answer, or writes the next step.
    pub(super) fn advance(&mut self, effects: &mut impl Effects, now: Instant) {
        match self.link {
            Link::Searching { wish, phase } => self.advance_search(wish, phase, effects, now),
            Link::Ready(_) => {
                self.forget_give_ups_if_stable(now);
                self.advance_session(effects, now);
            }
        }
    }

    fn advance_search(
        &mut self,
        wish: Wish,
        phase: Phase,
        effects: &mut impl Effects,
        now: Instant,
    ) {
        match phase {
            Phase::Waiting { until } => {
                if now >= until {
                    self.search(effects, now, 0, wish);
                }
            }
            Phase::Connecting {
                since,
                restarts,
                request: _,
            } => {
                // A `now` before `since` counts as no time.
                if now.saturating_duration_since(since) >= SEARCH_RESTART {
                    // A new connect replaces the search under way in the
                    // helper. The count is for the log; at its top it may
                    // stop.
                    self.search(effects, now, restarts.saturating_add(1), wish);
                }
            }
            Phase::Reading {
                since,
                request: _,
                progress: _,
            } => {
                if overdue(since, now) {
                    self.lose(effects, now, "the lamp did not answer a read");
                }
            }
        }
    }

    fn search(&mut self, effects: &mut impl Effects, now: Instant, restarts: u32, wish: Wish) {
        let request = self.take_id();
        self.link = Link::Searching {
            wish,
            phase: Phase::Connecting {
                request,
                since: now,
                restarts,
            },
        };
        Self::send(effects, &Request::connect(request));
    }

    pub(super) fn read(
        &mut self,
        effects: &mut impl Effects,
        now: Instant,
        progress: Progress,
        wish: Wish,
    ) {
        let request = self.take_id();
        self.link = Link::Searching {
            wish,
            phase: Phase::Reading {
                request,
                since: now,
                progress,
            },
        };
        Self::send(effects, &Request::read(request, progress.characteristic()));
    }

    pub(super) fn lose(&mut self, effects: &mut impl Effects, now: Instant, why: &str) {
        effects.log(&format!("connection given up: {why}"));
        Self::send(effects, &Request::Disconnect);
        self.wait_to_retry(now, Loss::GaveUp);
    }

    /// A connection that has lasted [`STABLE`] forgets the give-ups before
    /// it, and the panel stops saying that the lamp keeps failing.
    fn forget_give_ups_if_stable(&mut self, now: Instant) {
        if self.link.lasted(now, STABLE) {
            self.give_ups = 0;
        }
    }

    fn wait_to_retry(&mut self, now: Instant, loss: Loss) {
        self.forget_give_ups_if_stable(now);
        match loss {
            Loss::GaveUp => {
                // Past the limit the count no longer matters.
                self.give_ups = self.give_ups.saturating_add(1);
            }
            Loss::Dropped => {}
        }
        let wish = match self.run {
            Run::Running => self.link.wish_after_loss(self.settings, self.aim),
            // Quitting switches the lamp off, whatever it does meanwhile.
            Run::Quitting { until: _ } => Wish::Apply,
        };
        self.timers.interrupted();
        self.link = Link::Searching {
            wish,
            phase: Phase::Waiting {
                until: later(now, retry_pause(self.give_ups)),
            },
        };
    }

    /// A helper that does not read is reported as [`super::Input::Gone`]
    /// by its reader; here it is only logged.
    pub(super) fn send(effects: &mut impl Effects, request: &Request) {
        if let Err(error) = effects.request(request) {
            effects.log(&format!("cannot reach the Bluetooth helper: {error}"));
        }
    }

    pub(super) const fn take_id(&mut self) -> RequestId {
        self.next_id.take()
    }
}

/// The seconds left in a timer read from the lamp; stopped when
/// unreadable.
fn timer_left(value: &[u8], effects: &mut impl Effects) -> Countdown {
    Countdown::from_bytes(value).unwrap_or_else(|| {
        effects.log(&format!("strange timer value {value:02x?}"));
        Countdown::Stopped
    })
}

pub(super) fn overdue(since: Instant, now: Instant) -> bool {
    // A `now` before `since` counts as no time.
    now.saturating_duration_since(since) >= ANSWER_TIMEOUT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_retry_pause_doubles_up_to_the_longest() {
        let pauses: Vec<u64> = (0..=8)
            .map(|give_ups| retry_pause(give_ups).as_secs())
            .collect();
        assert_eq!(pauses, [2, 2, 4, 8, 16, 30, 30, 30, 30]);
        assert_eq!(retry_pause(u8::MAX), LONGEST_RETRY_PAUSE);
    }
}
