//! One write at a time from what the lamp does to what it should do. The
//! caller asks [`next_step`], writes, applies the step and asks again, so a
//! new target takes over with the next write.

use serde::Serialize;

use crate::timer::{Countdown, TimerKind};

use super::{Channel, Characteristic, ColorLevels, Lamp, ModeCommand, Output, WhiteLevels};

/// What the lamp should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Light {
    Off,
    White(WhiteLevels),
    Color(ColorLevels),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    White(WhiteLevels),
    Color(ColorLevels),
    Mode(ModeCommand),
    Timer(TimerKind, Countdown),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Delivery {
    Confirmed,
    Unconfirmed,
}

impl Step {
    #[must_use]
    pub(crate) const fn characteristic(self) -> Characteristic {
        match self {
            Self::White(_) => Characteristic::White,
            Self::Color(_) => Characteristic::Color,
            Self::Mode(_) => Characteristic::Mode,
            Self::Timer(TimerKind::Wake, _) => Characteristic::WakeTimer,
            Self::Timer(TimerKind::Sleep, _) => Characteristic::SleepTimer,
        }
    }

    #[must_use]
    pub(crate) fn bytes(self) -> Vec<u8> {
        match self {
            Self::White(levels) => levels.bytes().to_vec(),
            Self::Color(levels) => levels.bytes().to_vec(),
            Self::Mode(command) => vec![command.byte()],
            Self::Timer(_, countdown) => countdown.bytes().to_vec(),
        }
    }

    /// Mode changes and timers need the lamp's answer: it refuses mode
    /// changes during the lockout, and a timer counts as set only once the
    /// lamp has it. Light values go out unconfirmed, quick enough for a
    /// slider.
    #[must_use]
    pub(crate) const fn delivery(self) -> Delivery {
        match self {
            Self::White(_) | Self::Color(_) => Delivery::Unconfirmed,
            Self::Mode(_) | Self::Timer(..) => Delivery::Confirmed,
        }
    }

    #[must_use]
    pub(crate) const fn is_mode_change(self) -> bool {
        match self {
            Self::Mode(_) => true,
            Self::White(_) | Self::Color(_) | Self::Timer(..) => false,
        }
    }
}

/// Values come before the "on", so the lamp shows the right light as soon
/// as it is switched; "off" goes only to the light that shines.
#[must_use]
pub(crate) fn next_step(lamp: Lamp, target: Light) -> Option<Step> {
    match target {
        Light::Off => match lamp.output {
            Output::Dark => None,
            Output::Shining(Channel::White) => Some(Step::Mode(ModeCommand::WhiteOff)),
            Output::Shining(Channel::Color) => Some(Step::Mode(ModeCommand::ColorOff)),
        },
        Light::White(levels) => {
            if lamp.white != levels {
                return Some(Step::White(levels));
            }
            (lamp.output != Output::Shining(Channel::White))
                .then_some(Step::Mode(ModeCommand::WhiteOn))
        }
        Light::Color(levels) => {
            if lamp.color != levels {
                return Some(Step::Color(levels));
            }
            (lamp.output != Output::Shining(Channel::Color))
                .then_some(Step::Mode(ModeCommand::ColorOn))
        }
    }
}

impl Lamp {
    pub(crate) const fn apply(&mut self, step: Step) {
        match step {
            Step::White(levels) => self.white = levels,
            Step::Color(levels) => self.color = levels,
            Step::Mode(ModeCommand::WhiteOn) => self.output = Output::Shining(Channel::White),
            Step::Mode(ModeCommand::ColorOn) => self.output = Output::Shining(Channel::Color),
            Step::Mode(ModeCommand::WhiteOff | ModeCommand::ColorOff) => {
                self.output = Output::Dark;
            }
            Step::Timer(..) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Level;
    use super::*;

    fn warm(percent: u8) -> WhiteLevels {
        WhiteLevels {
            warm: Level::new(percent).unwrap(),
            cold: Level::default(),
        }
    }

    fn red(percent: u8) -> ColorLevels {
        ColorLevels {
            red: Level::new(percent).unwrap(),
            ..ColorLevels::default()
        }
    }

    fn lamp(output: Output) -> Lamp {
        Lamp {
            output,
            white: warm(50),
            color: red(100),
        }
    }

    const WHITE: Output = Output::Shining(Channel::White);
    const COLOR: Output = Output::Shining(Channel::Color);

    /// All steps from `lamp` to `target`, applied one after the other.
    fn steps(mut lamp: Lamp, target: Light) -> Vec<Step> {
        let mut steps = Vec::new();
        while let Some(step) = next_step(lamp, target) {
            assert!(steps.len() < 10, "no end in sight: {steps:?}");
            lamp.apply(step);
            steps.push(step);
        }
        steps
    }

    #[test]
    fn switches_on_with_the_values_first() {
        assert_eq!(
            steps(lamp(Output::Dark), Light::White(warm(30))),
            [Step::White(warm(30)), Step::Mode(ModeCommand::WhiteOn)]
        );
        assert_eq!(
            steps(lamp(Output::Dark), Light::Color(red(100))),
            [Step::Mode(ModeCommand::ColorOn)]
        );
    }

    #[test]
    fn changes_between_the_lights_with_one_command() {
        assert_eq!(
            steps(lamp(WHITE), Light::Color(red(40))),
            [Step::Color(red(40)), Step::Mode(ModeCommand::ColorOn)]
        );
        assert_eq!(
            steps(lamp(COLOR), Light::White(warm(50))),
            [Step::Mode(ModeCommand::WhiteOn)]
        );
    }

    #[test]
    fn a_slider_only_writes_values() {
        assert_eq!(
            steps(lamp(WHITE), Light::White(warm(80))),
            [Step::White(warm(80))]
        );
        assert_eq!(
            steps(lamp(COLOR), Light::Color(red(20))),
            [Step::Color(red(20))]
        );
    }

    #[test]
    fn switches_off_only_the_light_that_shines() {
        assert_eq!(
            steps(lamp(WHITE), Light::Off),
            [Step::Mode(ModeCommand::WhiteOff)]
        );
        assert_eq!(
            steps(lamp(COLOR), Light::Off),
            [Step::Mode(ModeCommand::ColorOff)]
        );
        assert_eq!(steps(lamp(Output::Dark), Light::Off), []);
    }

    #[test]
    fn mode_changes_and_timers_wait_for_the_lamp() {
        assert_eq!(Step::White(warm(1)).delivery(), Delivery::Unconfirmed);
        assert_eq!(Step::Color(red(100)).delivery(), Delivery::Unconfirmed);
        assert_eq!(
            Step::Mode(ModeCommand::WhiteOn).delivery(),
            Delivery::Confirmed
        );
        assert_eq!(
            Step::Timer(TimerKind::Sleep, Countdown::Stopped).delivery(),
            Delivery::Confirmed
        );
        assert_eq!(Step::Mode(ModeCommand::ColorOn).bytes(), [0xA8]);
        assert_eq!(Step::Color(red(100)).bytes(), [100, 0, 0]);
    }
}
