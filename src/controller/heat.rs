//! Protection from heat: the lamp's own limits are unknown, so
//! Lampster Remote watches its temperature.

#![deny(clippy::arithmetic_side_effects)]

use crate::lamp::{Output, Temperature};
use crate::setting::{Brightness, Power};

use super::link::Link;
use super::{Aim, Controller, Effects, HeatAction};

/// From here on the lamp is dimmed to [`COOL_BRIGHTNESS`], from
/// [`HOT`] on switched off. At full white the lamp levels off at about
/// 57 °C after a quarter of an hour, so normal use stays below; its own
/// limits are unknown.
const WARM: Temperature = Temperature::celsius(60);
const HOT: Temperature = Temperature::celsius(65);
pub(super) const COOL_BRIGHTNESS: Brightness = Brightness::HALF;

/// How warm the lamp is, by [`WARM`] and [`HOT`].
#[derive(Debug, Clone, Copy)]
enum Heat {
    Normal,
    Warm,
    Hot,
}

impl Heat {
    fn of(temperature: Temperature) -> Self {
        if temperature >= HOT {
            Self::Hot
        } else if temperature >= WARM {
            Self::Warm
        } else {
            Self::Normal
        }
    }
}

impl Controller {
    /// Dims a warm lamp and switches a hot one off.
    pub(super) fn on_temperature(&mut self, value: &[u8], effects: &mut impl Effects) {
        let Some(temperature) = Temperature::from_bytes(value) else {
            return effects.log(&format!("strange temperature {value:02x?}"));
        };
        let Link::Ready(session) = &mut self.link else {
            return;
        };
        session.temperature = Some(temperature);
        if session.lamp.output == Output::Dark {
            return;
        }
        let settings = &mut self.settings;
        match Heat::of(temperature) {
            Heat::Hot => {
                settings.power = Power::Off;
                self.aim = Aim::Settings;
                // Heat protection does not give up.
                session.mode.forget();
                effects.log(&format!("too hot ({temperature:?}): switched off"));
                self.heat = Some(HeatAction::SwitchedOff);
            }
            // Already dimmed far enough, perhaps by the user.
            Heat::Warm if settings.brightness <= COOL_BRIGHTNESS => {}
            Heat::Warm => {
                settings.brightness = COOL_BRIGHTNESS;
                self.aim = Aim::Settings;
                effects.log(&format!("warm ({temperature:?}): dimmed"));
                self.heat = Some(HeatAction::Dimmed);
            }
            Heat::Normal => {}
        }
    }
}
