//! The Lampster's Bluetooth protocol and what Lampster Remote believes
//! about the lamp. What is known about the protocol, and what experiments
//! with a real lamp showed, is written down in the README, section "The
//! lamp's Bluetooth protocol"; the comments here give only what the code
//! relies on.
//!
//! The lamp needs no pairing. It advertises [`ADVERTISED_SERVICE`]; every
//! [`Characteristic`] Lampster Remote uses belongs to [`LIGHT_SERVICE`].

#![deny(clippy::arithmetic_side_effects)]

pub(crate) mod plan;

use std::time::Duration;

pub(crate) const ADVERTISED_SERVICE: &str = "01FF5550-BA5E-F4EE-5CA1-EB1E5E4B1CE0";
pub(crate) const LIGHT_SERVICE: &str = "01FF5553-BA5E-F4EE-5CA1-EB1E5E4B1CE0";

/// The measured 1.8 s with some margin.
pub(crate) const MODE_LOCKOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[cfg_attr(test, derive(strum::EnumIter))]
#[serde(try_from = "String")]
pub(crate) enum Characteristic {
    Mode,
    White,
    Color,
    WakeTimer,
    SleepTimer,
    Temperature,
}

impl TryFrom<String> for Characteristic {
    type Error = UnknownCharacteristic;

    fn try_from(uuid: String) -> Result<Self, Self::Error> {
        Self::ALL
            .into_iter()
            .find(|characteristic| characteristic.uuid().eq_ignore_ascii_case(&uuid))
            .ok_or(UnknownCharacteristic(uuid))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown characteristic {0}")]
pub(crate) struct UnknownCharacteristic(String);

impl Characteristic {
    pub(crate) const ALL: [Self; 6] = [
        Self::Mode,
        Self::White,
        Self::Color,
        Self::WakeTimer,
        Self::SleepTimer,
        Self::Temperature,
    ];

    #[must_use]
    pub(crate) const fn uuid(self) -> &'static str {
        match self {
            Self::Mode => "01FF5554-BA5E-F4EE-5CA1-EB1E5E4B1CE0",
            Self::White => "01FF5556-BA5E-F4EE-5CA1-EB1E5E4B1CE0",
            Self::Color => "01FF5559-BA5E-F4EE-5CA1-EB1E5E4B1CE0",
            Self::WakeTimer => "01FF5561-BA5E-F4EE-5CA1-EB1E5E4B1CE0",
            Self::SleepTimer => "01FF5562-BA5E-F4EE-5CA1-EB1E5E4B1CE0",
            // The Bluetooth standard's temperature characteristic.
            Self::Temperature => "2A6E",
        }
    }
}

/// The intensity of one group of LEDs on the lamp's own scale, 0 to 100,
/// not the 0 to 255 of RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Level(u8);

impl Level {
    pub(crate) const MAX: Self = Self(100);

    #[must_use]
    pub(crate) const fn new(value: u8) -> Option<Self> {
        match value {
            0..=100 => Some(Self(value)),
            _ => None,
        }
    }

    /// `value` comes from percent and is at most 100.
    #[must_use]
    pub(crate) fn capped(value: u32) -> Self {
        let level = u8::try_from(value).ok().and_then(Self::new);
        debug_assert!(level.is_some(), "invariant: {value} is a percentage");
        // Should a computation ever overshoot, full light is its limit.
        level.unwrap_or(Self::MAX)
    }

    #[must_use]
    pub(crate) const fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct WhiteLevels {
    pub(crate) warm: Level,
    pub(crate) cold: Level,
}

impl WhiteLevels {
    #[must_use]
    pub(crate) const fn bytes(self) -> [u8; 2] {
        [self.warm.get(), self.cold.get()]
    }

    #[must_use]
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let [warm, cold] = bytes else { return None };
        Some(Self {
            warm: Level::new(*warm)?,
            cold: Level::new(*cold)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ColorLevels {
    pub(crate) red: Level,
    pub(crate) green: Level,
    pub(crate) blue: Level,
}

impl ColorLevels {
    #[must_use]
    pub(crate) const fn bytes(self) -> [u8; 3] {
        [self.red.get(), self.green.get(), self.blue.get()]
    }

    #[must_use]
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let [red, green, blue] = bytes else {
            return None;
        };
        Some(Self {
            red: Level::new(*red)?,
            green: Level::new(*green)?,
            blue: Level::new(*blue)?,
        })
    }
}

/// The byte is a bit field: `0x80` on, `0x40` white, `0x20` colour. `0x08`
/// is set by the lamp's button and in the sources' commands; without it
/// they work too, its meaning is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModeCommand {
    WhiteOn,
    WhiteOff,
    ColorOn,
    ColorOff,
}

impl ModeCommand {
    #[must_use]
    pub(crate) const fn byte(self) -> u8 {
        match self {
            Self::WhiteOn => 0xC8,
            Self::WhiteOff => 0x40,
            Self::ColorOn => 0xA8,
            Self::ColorOff => 0x20,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Channel {
    White,
    Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Output {
    Dark,
    Shining(Channel),
}

/// The mode byte read from the lamp is the last command it accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModeReading {
    pub(crate) output: Output,
    /// The light to switch on next while the lamp is dark.
    pub(crate) last: Channel,
}

impl ModeReading {
    /// What a byte no known command writes is taken for when connecting:
    /// dark, and the next "on" puts it right.
    pub(crate) const UNKNOWN: Self = Self {
        output: Output::Dark,
        last: Channel::White,
    };

    /// `None` unless it is one byte, with the bits of one light.
    #[must_use]
    pub(crate) const fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let [byte] = bytes else { return None };
        Self::from_byte(*byte)
    }

    /// `None` for a byte with neither or both lights' bits.
    #[must_use]
    const fn from_byte(byte: u8) -> Option<Self> {
        let channel = match byte & 0x60 {
            0x40 => Channel::White,
            0x20 => Channel::Color,
            _ => return None,
        };
        let output = match byte & 0x80 {
            0 => Output::Dark,
            _ => Output::Shining(channel),
        };
        Some(Self {
            output,
            last: channel,
        })
    }
}

/// Hundredths of a degree Celsius, as the lamp reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct Temperature(i16);

impl Temperature {
    #[must_use]
    pub(crate) const fn celsius(degrees: i16) -> Self {
        // For constants far below the limit of an i16.
        Self(degrees.saturating_mul(100))
    }

    #[must_use]
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes: [u8; 2] = bytes.try_into().ok()?;
        Some(Self(i16::from_le_bytes(bytes)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lamp {
    pub(crate) output: Output,
    pub(crate) white: WhiteLevels,
    pub(crate) color: ColorLevels,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_round_trip_through_bytes() {
        let white = WhiteLevels {
            warm: Level::new(50).unwrap(),
            cold: Level::new(0).unwrap(),
        };
        assert_eq!(white.bytes(), [50, 0]);
        assert_eq!(WhiteLevels::from_bytes(&[50, 0]), Some(white));
        let color = ColorLevels {
            red: Level::MAX,
            green: Level::new(1).unwrap(),
            blue: Level::new(30).unwrap(),
        };
        assert_eq!(color.bytes(), [100, 1, 30]);
        assert_eq!(ColorLevels::from_bytes(&[100, 1, 30]), Some(color));
    }

    #[test]
    fn rejects_bytes_that_make_no_sense() {
        assert_eq!(WhiteLevels::from_bytes(&[50]), None);
        assert_eq!(WhiteLevels::from_bytes(&[50, 101]), None);
        assert_eq!(ColorLevels::from_bytes(&[1, 2, 3, 4]), None);
        assert_eq!(ColorLevels::from_bytes(&[255, 0, 0]), None);
    }

    #[test]
    fn caps_levels() {
        assert_eq!(Level::capped(0), Level::new(0).unwrap());
        assert_eq!(Level::capped(42), Level::new(42).unwrap());
        assert_eq!(Level::capped(100), Level::MAX);
    }

    #[test]
    fn reads_the_mode_byte() {
        let white = ModeReading::from_byte(0xC8).unwrap();
        assert_eq!(white.output, Output::Shining(Channel::White));
        let color = ModeReading::from_byte(0xA0).unwrap();
        assert_eq!(color.output, Output::Shining(Channel::Color));
        let color_off = ModeReading::from_byte(0x20).unwrap();
        assert_eq!(
            (color_off.output, color_off.last),
            (Output::Dark, Channel::Color)
        );
        for white_off in [0x40, 0x48] {
            let reading = ModeReading::from_byte(white_off).unwrap();
            assert_eq!(
                (reading.output, reading.last),
                (Output::Dark, Channel::White)
            );
        }
        assert_eq!(ModeReading::from_byte(0x28).unwrap().last, Channel::Color);
        assert_eq!(ModeReading::from_byte(0x60), None, "both lights");
        assert_eq!(ModeReading::from_byte(0x08), None, "no light");
        assert_eq!(ModeReading::from_bytes(&[0xC8, 0]), None, "two bytes");
        assert_eq!(
            ModeReading::from_bytes(&[0xC8]),
            ModeReading::from_byte(0xC8)
        );
    }

    #[test]
    fn reads_the_temperature() {
        assert_eq!(
            Temperature::from_bytes(&[0xd1, 0x07]),
            Some(Temperature(2001))
        );
        assert_eq!(
            Temperature::from_bytes(&[0x18, 0xfc]),
            Some(Temperature(-1000))
        );
        assert_eq!(Temperature::from_bytes(&[1]), None);
        assert!(
            Temperature::celsius(60) > Temperature(5999),
            "60 °C in hundredths"
        );
    }

    #[test]
    fn all_lists_every_characteristic_once() {
        use strum::IntoEnumIterator;
        let every: Vec<Characteristic> = Characteristic::iter().collect();
        assert_eq!(every, Characteristic::ALL, "every variant, in order");
        for characteristic in Characteristic::ALL {
            assert_eq!(
                Characteristic::try_from(characteristic.uuid().to_owned()).unwrap(),
                characteristic
            );
        }
    }
}
