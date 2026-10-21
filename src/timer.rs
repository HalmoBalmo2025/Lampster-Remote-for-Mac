//! The lamp's two timers, which run without the Mac: one switches it on
//! (the wake-up light), one off (the sleep timer). The lamp counts seconds
//! and has no clock, so a time of day becomes the seconds from now.

#![deny(clippy::arithmetic_side_effects)]

use std::num::{NonZeroI64, NonZeroU32};

use jiff::{SignedDuration, Timestamp, ToSpan, Zoned};
use serde::{Deserialize, Serialize};

use crate::setting::OutOfRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(strum::EnumIter))]
pub(crate) enum TimerKind {
    Wake,
    Sleep,
}

impl TimerKind {
    pub(crate) const ALL: [Self; 2] = [Self::Wake, Self::Sleep];
}

/// A time of day from the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClockTime {
    hour: Hour,
    minute: Minute,
}

/// Held as jiff takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "u8")]
struct Hour(i8);

impl TryFrom<u8> for Hour {
    type Error = OutOfRange;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        i8::try_from(value)
            .ok()
            .filter(|hour| (0..=23).contains(hour))
            .map(Self)
            .ok_or_else(|| OutOfRange::new(value.into(), 0, 23))
    }
}

/// Held as jiff takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "u8")]
struct Minute(i8);

impl TryFrom<u8> for Minute {
    type Error = OutOfRange;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        i8::try_from(value)
            .ok()
            .filter(|minute| (0..=59).contains(minute))
            .map(Self)
            .ok_or_else(|| OutOfRange::new(value.into(), 0, 59))
    }
}

/// jiff could not work out a time: only for a clock at the end of what
/// jiff can hold.
#[derive(Debug, thiserror::Error)]
#[error("cannot work out the time of the timer: {0}")]
pub(crate) struct TimeError(#[from] jiff::Error);

/// Minutes until the sleep timer switches the lamp off: what the panel
/// takes, 1 minute to 2 hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "u16")]
pub(crate) struct Minutes(u16);

impl Minutes {
    const SHORTEST: u16 = 1;
    const LONGEST: u16 = 120;
}

impl TryFrom<u16> for Minutes {
    type Error = OutOfRange;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            Self::SHORTEST..=Self::LONGEST => Ok(Self(value)),
            _ => Err(OutOfRange::new(value, Self::SHORTEST, Self::LONGEST)),
        }
    }
}

/// The next moment the clock shows `time`: today, or tomorrow if that has
/// passed. Daylight saving time is taken into account.
pub(crate) fn next(time: ClockTime, now: &Zoned) -> Result<Timestamp, TimeError> {
    let today = now
        .with()
        .hour(time.hour.0)
        .minute(time.minute.0)
        .second(0)
        .subsec_nanosecond(0)
        .build()?;
    let at = if today.timestamp() > now.timestamp() {
        today
    } else {
        today.checked_add(1.day())?
    };
    Ok(at.timestamp())
}

pub(crate) fn after(minutes: Minutes, now: &Zoned) -> Result<Timestamp, TimeError> {
    Ok(now
        .timestamp()
        .checked_add(i64::from(minutes.0).minutes())?)
}

/// The seconds the lamp counts until `at`; at least 1, as 0 cancels, so a
/// time that has come or passed fires at once.
#[must_use]
pub(crate) fn seconds_until(at: Timestamp, now: &Zoned) -> NonZeroU32 {
    let seconds = at.duration_since(now.timestamp()).as_secs();
    if seconds <= 0 {
        return NonZeroU32::MIN;
    }
    let counted = u32::try_from(seconds).ok().and_then(NonZeroU32::new);
    debug_assert!(
        counted.is_some(),
        "invariant: timers are at most a day away"
    );
    // A count the lamp cannot hold is as far away as it can count.
    counted.unwrap_or(NonZeroU32::MAX)
}

/// What the lamp's timer characteristic holds: seconds, little endian.
const fn bytes(seconds: u32) -> [u8; 4] {
    seconds.to_le_bytes()
}

/// What a timer characteristic holds: written, when the lamp is to fire;
/// read or notified, the seconds left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Countdown {
    Seconds(NonZeroU32),
    /// 0: written, it cancels; read, no timer runs (any more).
    Stopped,
}

impl Countdown {
    #[must_use]
    pub(crate) const fn bytes(self) -> [u8; 4] {
        bytes(match self {
            Self::Seconds(seconds) => seconds.get(),
            Self::Stopped => 0,
        })
    }

    #[must_use]
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes: [u8; 4] = bytes.try_into().ok()?;
        Some(NonZeroU32::new(u32::from_le_bytes(bytes)).map_or(Self::Stopped, Self::Seconds))
    }
}

/// Seconds between the time a timer was set for and the time the lamp
/// counts towards within which both are the same.
const SAME_TIME: SignedDuration = SignedDuration::from_secs(60);

/// Whether the lamp knows a timer as it is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sync {
    /// The lamp needs it (again).
    Needed,
    /// Written, waiting for the confirmation.
    Writing,
    Done,
}

/// One timer as Lampster Remote keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timer {
    /// `None`: not set.
    pub(crate) at: Option<Timestamp>,
    pub(crate) sync: Sync,
}

impl Timer {
    pub(crate) const UNSET: Self = Self {
        at: None,
        sync: Sync::Done,
    };

    pub(crate) const fn set(at: Option<Timestamp>) -> Self {
        Self {
            at,
            sync: Sync::Needed,
        }
    }

    /// What the lamp is told to make it run as set.
    #[must_use]
    pub(crate) fn countdown(self, now: &Zoned) -> Countdown {
        self.at.map_or(Countdown::Stopped, |at| {
            Countdown::Seconds(seconds_until(at, now))
        })
    }

    /// The write went out.
    pub(crate) const fn sent(&mut self) {
        self.sync = Sync::Writing;
    }

    /// The lamp has the write; a change made meanwhile still needs its
    /// own.
    pub(crate) const fn confirmed(&mut self) {
        self.write_ended(Sync::Done);
    }

    /// The write did not reach the lamp; it is made again.
    pub(crate) const fn write_again(&mut self) {
        self.write_ended(Sync::Needed);
    }

    /// A write under way ends in `then`; a change made since stays needed.
    const fn write_ended(&mut self, then: Sync) {
        match self.sync {
            Sync::Writing => self.sync = then,
            Sync::Needed | Sync::Done => {}
        }
    }

    /// A change the lamp does not have yet whose time has passed: the lamp
    /// would fire it at once.
    #[must_use]
    pub(crate) fn is_overdue(self, now: &Zoned) -> bool {
        match self.sync {
            Sync::Done => false,
            Sync::Needed | Sync::Writing => self.at.is_some_and(|at| at <= now.timestamp()),
        }
    }

    /// Takes what the lamp reported on connecting: the seconds left. A
    /// change it may not have got is written (again), unless it is
    /// overdue; a timer the lamp lost (it had no power) is written again;
    /// one it fired is gone.
    pub(crate) fn reconcile(&mut self, left: Countdown, now: &Zoned) {
        match self.sync {
            Sync::Needed | Sync::Writing if !self.is_overdue(now) => self.sync = Sync::Needed,
            Sync::Needed | Sync::Writing | Sync::Done => self.take(left, now),
        }
    }

    /// The lamp's count is what holds.
    fn take(&mut self, left: Countdown, now: &Zoned) {
        match left {
            Countdown::Stopped => match self.at {
                Some(at) if at > now.timestamp() => self.sync = Sync::Needed,
                Some(_) | None => *self = Self::UNSET,
            },
            Countdown::Seconds(seconds) => {
                let counted = now
                    .timestamp()
                    .checked_add(NonZeroI64::from(seconds).get().seconds())
                    .ok();
                let at = match (self.at, counted) {
                    // The lamp counts whole seconds and drifts a little; the
                    // time the user set stays.
                    (Some(at), Some(counted))
                        if at > now.timestamp()
                            && at.duration_since(counted).abs() <= SAME_TIME =>
                    {
                        Some(at)
                    }
                    (Some(_) | None, _) => counted,
                };
                *self = Self {
                    at,
                    sync: Sync::Done,
                };
            }
        }
    }
}

/// Both timers as Lampster Remote keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timers {
    pub(crate) wake: Timer,
    pub(crate) sleep: Timer,
}

impl Timers {
    pub(crate) const UNSET: Self = Self {
        wake: Timer::UNSET,
        sleep: Timer::UNSET,
    };

    #[must_use]
    pub(crate) const fn get(self, kind: TimerKind) -> Timer {
        match kind {
            TimerKind::Wake => self.wake,
            TimerKind::Sleep => self.sleep,
        }
    }

    pub(crate) const fn get_mut(&mut self, kind: TimerKind) -> &mut Timer {
        match kind {
            TimerKind::Wake => &mut self.wake,
            TimerKind::Sleep => &mut self.sleep,
        }
    }

    /// A write cut off by a lost connection is made again.
    pub(crate) fn interrupted(&mut self) {
        for kind in TimerKind::ALL {
            self.get_mut(kind).write_again();
        }
    }

    /// The first timer the lamp has to learn about.
    #[must_use]
    pub(crate) fn needed(self) -> Option<TimerKind> {
        TimerKind::ALL
            .into_iter()
            .find(|kind| match self.get(*kind).sync {
                Sync::Needed => true,
                Sync::Writing | Sync::Done => false,
            })
    }
}

/// A timer as the panel shows it. A timer that is off, and that the lamp
/// knows is off, is not shown at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum TimerView {
    /// In the lamp, which fires it at `at`.
    InLamp { at: UnixSeconds },
    /// Set for `at`; the lamp gets it once it is connected.
    Pending { at: UnixSeconds },
    /// Switched off; until it is connected, the lamp still fires it.
    Cancelling,
}

/// Seconds since 1970, for the panel to show the time and what is left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct UnixSeconds(i64);

impl From<Timestamp> for UnixSeconds {
    fn from(at: Timestamp) -> Self {
        Self(at.as_second())
    }
}

impl Timer {
    #[must_use]
    pub(crate) fn view(self) -> Option<TimerView> {
        match self.sync {
            Sync::Done => self.at.map(|at| TimerView::InLamp { at: at.into() }),
            Sync::Needed | Sync::Writing => Some(match self.at {
                Some(at) => TimerView::Pending { at: at.into() },
                None => TimerView::Cancelling,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zoned(text: &str) -> Zoned {
        text.parse().unwrap()
    }

    fn left(seconds: u32) -> Countdown {
        NonZeroU32::new(seconds).map_or(Countdown::Stopped, Countdown::Seconds)
    }

    fn clock(hour: u8, minute: u8) -> ClockTime {
        ClockTime {
            hour: Hour::try_from(hour).unwrap(),
            minute: Minute::try_from(minute).unwrap(),
        }
    }

    #[test]
    fn a_time_in_the_repeated_hour_keeps_the_offset_of_now() {
        // 2:15 in the second pass of the hour repeated in autumn (CET): the
        // panel's fieldDay (helpers/ui/TimerTexts.swift) names the same day.
        let now = zoned("2026-10-25T02:15:00+01:00[Europe/Berlin]");
        let at = next(clock(2, 30), &now).unwrap();
        assert_eq!(
            at,
            zoned("2026-10-25T02:30:00+01:00[Europe/Berlin]").timestamp()
        );
    }

    #[test]
    fn a_time_of_day_is_today_or_tomorrow() {
        let now = zoned("2026-10-19T22:30:15+02:00[Europe/Berlin]");
        let tomorrow = next(clock(7, 0), &now).unwrap();
        assert_eq!(
            tomorrow,
            zoned("2026-10-20T07:00:00+02:00[Europe/Berlin]").timestamp()
        );
        assert_eq!(seconds_until(tomorrow, &now).get(), 30_585);
        let tonight = next(clock(23, 0), &now).unwrap();
        assert_eq!(seconds_until(tonight, &now).get(), 1785);
        let now_exactly = next(
            clock(22, 30),
            &zoned("2026-10-19T22:30:00+02:00[Europe/Berlin]"),
        );
        assert_eq!(
            now_exactly.unwrap(),
            zoned("2026-10-20T22:30:00+02:00[Europe/Berlin]").timestamp(),
            "a time that is now is tomorrow"
        );
    }

    #[test]
    fn the_night_the_clocks_change_has_an_hour_more() {
        // 25 October 2026: 3:00 summer time becomes 2:00 winter time.
        let now = zoned("2026-10-24T23:00:00+02:00[Europe/Berlin]");
        let at = next(clock(7, 0), &now).unwrap();
        assert_eq!(seconds_until(at, &now).get(), 9 * 3600);
    }

    #[test]
    fn timers_are_little_endian_seconds() {
        assert_eq!(bytes(261), [5, 1, 0, 0]);
        assert_eq!(Countdown::from_bytes(&[0xa8, 0, 0, 0]), Some(left(168)));
        assert_eq!(Countdown::from_bytes(&[1, 2]), None);
        let now = zoned("2026-10-19T22:30:00+02:00[Europe/Berlin]");
        assert_eq!(
            seconds_until(now.timestamp(), &now).get(),
            1,
            "0 would cancel"
        );
    }

    #[test]
    fn a_timer_is_spelled_as_the_panel_reads_it() {
        let at = zoned("2026-10-20T07:00:00+02:00[Europe/Berlin]").timestamp();
        let line = |timer: Timer| serde_json::to_string(&timer.view()).unwrap();
        assert_eq!(
            line(Timer::set(Some(at))),
            r#"{"state":"pending","at":1792472400}"#
        );
        let in_lamp = Timer {
            at: Some(at),
            sync: Sync::Done,
        };
        assert_eq!(line(in_lamp), r#"{"state":"in_lamp","at":1792472400}"#);
        assert_eq!(line(Timer::set(None)), r#"{"state":"cancelling"}"#);
        assert_eq!(line(Timer::UNSET), "null");
    }

    #[test]
    fn the_panel_cannot_send_impossible_times() {
        assert!(
            serde_json::from_str::<ClockTime>(r#"{"hour":24,"minute":0}"#).is_err(),
            "there is no hour 24"
        );
        assert!(
            serde_json::from_str::<Minutes>("0").is_err(),
            "a sleep timer of 0 minutes"
        );
        assert!(
            serde_json::from_str::<Minutes>("121").is_err(),
            "longer than the panel offers"
        );
        assert_eq!(serde_json::from_str::<Minutes>("1").unwrap(), Minutes(1));
        assert_eq!(
            serde_json::from_str::<Minutes>("120").unwrap(),
            Minutes(120)
        );
    }

    #[test]
    fn what_the_lamp_reports_on_connecting_wins() {
        let now = zoned("2026-10-19T22:00:00+02:00[Europe/Berlin]");
        let later = now.timestamp().checked_add(1.hour()).unwrap();
        let earlier = now.timestamp().checked_sub(1.hour()).unwrap();

        let mut fired = Timer {
            at: Some(earlier),
            sync: Sync::Done,
        };
        fired.reconcile(left(0), &now);
        assert_eq!(fired, Timer::UNSET);

        let mut lost = Timer {
            at: Some(later),
            sync: Sync::Done,
        };
        lost.reconcile(left(0), &now);
        assert_eq!(lost.sync, Sync::Needed, "the lamp had no power");

        let mut running = Timer::UNSET;
        running.reconcile(left(120), &now);
        assert_eq!(
            running.at,
            Some(now.timestamp().checked_add(2.minutes()).unwrap())
        );

        let mut changed = Timer::set(None);
        changed.reconcile(left(120), &now);
        assert_eq!(
            changed,
            Timer::set(None),
            "the user's change still goes out"
        );

        let mut cut_off = Timer {
            at: Some(later),
            sync: Sync::Writing,
        };
        cut_off.reconcile(left(7200), &now);
        assert_eq!(cut_off, Timer::set(Some(later)), "written again");

        let mut overdue = Timer::set(Some(earlier));
        overdue.reconcile(left(0), &now);
        assert_eq!(overdue, Timer::UNSET);
    }

    #[test]
    fn a_running_timer_keeps_the_time_it_was_set_for() {
        let now = zoned("2026-10-19T22:00:00+02:00[Europe/Berlin]");
        let later = now.timestamp().checked_add(1.hour()).unwrap();
        let mut running = Timer {
            at: Some(later),
            sync: Sync::Done,
        };
        running.reconcile(left(3599), &now);
        assert_eq!(running.at, Some(later));
        running.reconcile(left(3540), &now);
        assert_eq!(running.at, Some(later));
        running.reconcile(left(3000), &now);
        assert_eq!(
            running.at,
            Some(now.timestamp().checked_add(50.minutes()).unwrap()),
            "another timer"
        );
    }

    /// What the panel's field takes (helpers/ui/Model.swift) and the type
    /// agree.
    #[test]
    fn the_panel_takes_the_sleep_times_the_type_takes() {
        let model = include_str!("../helpers/ui/Model.swift");
        let (shortest, longest) = model
            .split_once("static let sleepMinutes = ")
            .and_then(|(_, rest)| rest.split_once('\n'))
            .and_then(|(range, _)| range.split_once("..."))
            .unwrap();
        assert_eq!(
            (
                shortest.parse::<u16>().unwrap(),
                longest.parse::<u16>().unwrap()
            ),
            (Minutes::SHORTEST, Minutes::LONGEST)
        );
    }

    #[test]
    fn all_lists_every_timer_once() {
        use strum::IntoEnumIterator;
        let every: Vec<TimerKind> = TimerKind::iter().collect();
        assert_eq!(every, TimerKind::ALL, "every variant, in order");
    }

    #[test]
    fn a_timer_due_right_when_the_lamp_says_it_stopped_has_fired() {
        let now = zoned("2026-10-19T22:00:00+02:00[Europe/Berlin]");
        let mut due = Timer {
            at: Some(now.timestamp()),
            sync: Sync::Done,
        };
        due.reconcile(left(0), &now);
        assert_eq!(due, Timer::UNSET);
    }

    #[test]
    fn a_timer_due_now_that_the_lamp_still_counts_takes_the_lamps_count() {
        let now = zoned("2026-10-19T22:00:00+02:00[Europe/Berlin]");
        let mut due = Timer {
            at: Some(now.timestamp()),
            sync: Sync::Done,
        };
        due.reconcile(left(30), &now);
        assert_eq!(
            due.at,
            Some(now.timestamp().checked_add(30.seconds()).unwrap()),
            "the time set has passed; the lamp's count holds"
        );
    }
}
