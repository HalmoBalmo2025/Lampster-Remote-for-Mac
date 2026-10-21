//! The connected lamp: one write at a time towards the settings, and the
//! lamp's answers to it.

use std::time::Instant;

use jiff::Zoned;

use crate::ble::{Request, RequestId};
use crate::lamp::plan::{self, Step};
use crate::lamp::{Lamp, MODE_LOCKOUT, Temperature};
use crate::setting::Settings;
use crate::timer::{Timer, TimerKind, Timers};

use super::link::{ANSWER_TIMEOUT, Link, overdue};
use super::notifications::Echoes;
use super::{Aim, Controller, Effects, later};

/// Refusals of the same kind of write in a row before the controller
/// waits for the user.
pub(super) const MAX_REFUSALS: u8 = 3;

#[derive(Debug)]
pub(super) struct Session {
    /// When the reads after connecting were done.
    pub(super) since: Instant,
    pub(super) lamp: Lamp,
    /// Of mode changes, which the lamp refuses during its lockout.
    pub(super) mode: Refusals,
    /// Of timer writes: tried again after the same pause.
    pub(super) timers: Refusals,
    pub(super) activity: Activity,
    /// The last reading of the lamp's sensor.
    pub(super) temperature: Option<Temperature>,
    pub(super) echoes: Echoes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Activity {
    Idle,
    Writing(InFlight),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InFlight {
    request: RequestId,
    step: Step,
    since: Instant,
}

/// Refusals of one kind of write in a row, and when the next one may go
/// out.
#[derive(Debug)]
pub(super) struct Refusals {
    in_a_row: u8,
    free_at: Instant,
}

impl Refusals {
    const fn new(now: Instant) -> Self {
        Self {
            in_a_row: 0,
            free_at: now,
        }
    }

    /// The next write of this kind waits for the lamp's lockout.
    pub(super) fn lock(&mut self, now: Instant) {
        self.free_at = later(now, MODE_LOCKOUT);
    }

    fn refused(&mut self, now: Instant) {
        self.lock(now);
        // Past the limit the count no longer matters.
        self.in_a_row = self.in_a_row.saturating_add(1);
    }

    /// The user changed something, the lamp's button was used, or the lamp
    /// took a write; for the light also heat protection switching it off:
    /// try again.
    pub(super) const fn forget(&mut self) {
        self.in_a_row = 0;
    }

    /// Refused too often; waits until something changes, or the
    /// connection is made again with a new session.
    pub(super) const fn gave_up(&self) -> bool {
        self.in_a_row >= MAX_REFUSALS
    }

    /// When the next write of this kind may go out; `None` after giving up.
    fn next_at(&self) -> Option<Instant> {
        (!self.gave_up()).then_some(self.free_at)
    }

    fn allows(&self, now: Instant) -> bool {
        self.next_at().is_some_and(|at| now >= at)
    }
}

impl Session {
    pub(super) fn new(lamp: Lamp, now: Instant) -> Self {
        Self {
            since: now,
            lamp,
            // The lamp may have just changed its mode; a refusal would
            // tell.
            mode: Refusals::new(now),
            timers: Refusals::new(now),
            activity: Activity::Idle,
            temperature: None,
            echoes: Echoes::default(),
        }
    }

    /// Something changed: what the lamp refused is tried again.
    pub(super) const fn forget_refusals(&mut self) {
        self.mode.forget();
        self.timers.forget();
    }

    /// The write under way, if `id` answers it.
    pub(super) fn writing(&self, id: RequestId) -> Option<InFlight> {
        match self.activity {
            Activity::Writing(in_flight) if in_flight.request == id => Some(in_flight),
            Activity::Writing(_) | Activity::Idle => None,
        }
    }

    /// Whether a write of the light is under way; a timer's does not count.
    pub(super) const fn writes_light(&self) -> bool {
        match self.activity {
            Activity::Idle => false,
            Activity::Writing(in_flight) => match in_flight.step {
                Step::Timer(..) => false,
                Step::Mode(_) | Step::White(_) | Step::Color(_) => true,
            },
        }
    }

    /// The next write to bring the light to what is aimed at.
    pub(super) fn next_step(&self, settings: Settings, aim: Aim) -> Option<Step> {
        match aim {
            Aim::Lamp => None,
            Aim::Settings => plan::next_step(self.lamp, settings.light()),
        }
    }

    /// A write that is not answered in time, or the end of the pause a
    /// mode change or the `timer` that is needed waits for.
    pub(super) fn deadline(
        &self,
        settings: Settings,
        aim: Aim,
        timer: Option<TimerKind>,
    ) -> Option<Instant> {
        match self.activity {
            Activity::Writing(in_flight) => in_flight.since.checked_add(ANSWER_TIMEOUT),
            Activity::Idle => {
                let mode = self
                    .next_step(settings, aim)
                    .filter(|step| step.is_mode_change())
                    .and_then(|_| self.mode.next_at());
                let timer = timer.and_then(|_| self.timers.next_at());
                mode.into_iter().chain(timer).min()
            }
        }
    }
}

impl Controller {
    pub(super) fn advance_session(&mut self, effects: &mut impl Effects, now: Instant) {
        let Link::Ready(session) = &mut self.link else {
            return;
        };
        match session.activity {
            Activity::Writing(in_flight) => {
                if overdue(in_flight.since, now) {
                    self.lose(effects, now, "the lamp did not answer a write");
                }
                return;
            }
            Activity::Idle => {}
        }
        // The light first; timers while a mode change waits for the
        // lockout or for the user.
        let light = session
            .next_step(self.settings, self.aim)
            .filter(|step| !step.is_mode_change() || session.mode.allows(now));
        let step = match light {
            Some(step) => step,
            None if session.timers.allows(now) => {
                let Some(step) = next_timer(&mut self.timers, effects) else {
                    return;
                };
                step
            }
            None => return,
        };
        match step {
            Step::Timer(kind, _) => self.timers.get_mut(kind).sent(),
            Step::White(_) | Step::Color(_) | Step::Mode(_) => {}
        }
        let request = self.next_id.take();
        session.activity = Activity::Writing(InFlight {
            request,
            step,
            since: now,
        });
        session.echoes.record(step, now);
        Self::send(effects, &Request::write(request, step));
    }

    pub(super) fn on_written(&mut self, id: RequestId, now: Instant) {
        let Link::Ready(session) = &mut self.link else {
            return;
        };
        let Some(in_flight) = session.writing(id) else {
            return;
        };
        session.activity = Activity::Idle;
        session.lamp.apply(in_flight.step);
        match in_flight.step {
            Step::Mode(_) => {
                session.mode.lock(now);
                session.mode.forget();
            }
            Step::Timer(kind, _) => {
                session.timers.forget();
                self.timers.get_mut(kind).confirmed();
            }
            Step::White(_) | Step::Color(_) => {}
        }
    }

    /// Tries again after the lockout, or gives up.
    pub(super) fn on_rejected(&mut self, id: RequestId, effects: &mut impl Effects, now: Instant) {
        let Link::Ready(session) = &mut self.link else {
            return;
        };
        let Some(in_flight) = session.writing(id) else {
            return;
        };
        session.activity = Activity::Idle;
        // A refused write changed nothing, so the lamp sends no echo of it.
        session.echoes.forget(in_flight.step);
        let refusals = match in_flight.step {
            Step::Timer(kind, _) => {
                self.timers.get_mut(kind).write_again();
                &mut session.timers
            }
            Step::Mode(_) | Step::White(_) | Step::Color(_) => &mut session.mode,
        };
        refusals.refused(now);
        effects.log(&format!(
            "the lamp refused {:?} ({} in a row)",
            in_flight.step, refusals.in_a_row
        ));
    }
}

fn next_timer(timers: &mut Timers, effects: &mut impl Effects) -> Option<Step> {
    let clock = effects.clock();
    drop_overdue(timers, effects, &clock);
    let kind = timers.needed()?;
    Some(Step::Timer(kind, timers.get(kind).countdown(&clock)))
}

/// A timer the lamp would fire at once is not written; the user set it
/// for a time that has passed.
pub(super) fn drop_overdue(timers: &mut Timers, effects: &mut impl Effects, clock: &Zoned) {
    for kind in TimerKind::ALL {
        let timer = timers.get_mut(kind);
        if timer.is_overdue(clock) {
            *timer = Timer::UNSET;
            effects.log(&format!("the {kind:?} timer was overdue and dropped"));
        }
    }
}
