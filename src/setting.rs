//! The panel's brightness, tone and colour, and how they become the lamp's
//! levels. Integer arithmetic on percent, so nothing is rounded twice.
//! Every value is at most 100 (or 359 for a hue), so the saturating
//! operations below never saturate; they keep each step free of a panic.

#![deny(clippy::arithmetic_side_effects)]

use serde::{Deserialize, Serialize};

use crate::lamp::plan::Light;
use crate::lamp::{Channel, ColorLevels, Lamp, Level, Output, WhiteLevels};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub(crate) struct Percent(u8);

impl TryFrom<u8> for Percent {
    type Error = OutOfRange;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0..=100 => Ok(Self(value)),
            _ => Err(OutOfRange::new(value.into(), 0, 100)),
        }
    }
}

impl From<Percent> for u8 {
    fn from(percent: Percent) -> Self {
        percent.0
    }
}

/// 1 to 100: "off" is the power switch, not a brightness of zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub(crate) struct Brightness(u8);

impl Brightness {
    /// Half the light: where the panel begins, and what heat protection
    /// dims to.
    pub(crate) const HALF: Self = Self(50);
}

impl Default for Brightness {
    fn default() -> Self {
        Self::HALF
    }
}

impl TryFrom<u8> for Brightness {
    type Error = OutOfRange;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1..=100 => Ok(Self(value)),
            _ => Err(OutOfRange::new(value.into(), 1, 100)),
        }
    }
}

impl From<Brightness> for u8 {
    fn from(brightness: Brightness) -> Self {
        brightness.0
    }
}

/// Degrees, 0 (red) to 359.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub(crate) struct Hue(u16);

impl TryFrom<u16> for Hue {
    type Error = OutOfRange;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0..=359 => Ok(Self(value)),
            _ => Err(OutOfRange::new(value, 0, 359)),
        }
    }
}

impl From<Hue> for u16 {
    fn from(hue: Hue) -> Self {
        hue.0
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{value} is not between {min} and {max}")]
pub(crate) struct OutOfRange {
    value: u16,
    min: u16,
    max: u16,
}

impl OutOfRange {
    pub(crate) const fn new(value: u16, min: u16, max: u16) -> Self {
        Self { value, min, max }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Power {
    On,
    #[default]
    Off,
}

/// The light the lamp uses when it shines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    White,
    Color,
}

/// A setting read back from the lamp's levels, with the brightness they
/// shine in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WithBrightness<Setting> {
    pub(crate) setting: Setting,
    pub(crate) brightness: Brightness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WhiteSetting {
    /// 0 only the warm LEDs, 100 only the cold ones.
    pub(crate) tone: Percent,
}

impl WhiteSetting {
    /// Towards one end of the tone, that group shines at full brightness
    /// and the other fades out, so the middle has both at full brightness
    /// instead of both at half.
    #[must_use]
    pub(crate) fn levels(self, brightness: Brightness) -> WhiteLevels {
        let tone = u32::from(self.tone.0);
        let brightness = u32::from(brightness.0);
        let share = |part: u32| scale(brightness, part.saturating_mul(2).min(100), 100);
        WhiteLevels {
            warm: Level::capped(share(100_u32.saturating_sub(tone))),
            cold: Level::capped(share(tone)),
        }
    }

    /// `None` when the levels hold no light.
    #[must_use]
    pub(crate) fn from_levels(levels: WhiteLevels) -> Option<WithBrightness<Self>> {
        let warm = u32::from(levels.warm.get());
        let cold = u32::from(levels.cold.get());
        let brightness = warm.max(cold);
        let tone = match warm.cmp(&cold) {
            std::cmp::Ordering::Less => 100_u32.saturating_sub(scale(warm, 50, brightness)),
            std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => scale(cold, 50, brightness),
        };
        Some(WithBrightness {
            setting: Self {
                tone: Percent::try_from(u8::try_from(tone).ok()?).ok()?,
            },
            brightness: Brightness::try_from(u8::try_from(brightness).ok()?).ok()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ColorSetting {
    pub(crate) hue: Hue,
    pub(crate) saturation: Percent,
}

impl Default for ColorSetting {
    fn default() -> Self {
        Self {
            hue: Hue(0),
            saturation: Percent(100),
        }
    }
}

impl ColorSetting {
    /// Hue, saturation and value to red, green and blue.
    #[must_use]
    pub(crate) fn levels(self, brightness: Brightness) -> ColorLevels {
        let hue = u32::from(self.hue.0);
        let span = u32::from(self.saturation.0);
        let low = 100_u32.saturating_sub(span);
        let offset = hue % 60;
        let rising = low.saturating_add(scale(span, offset, 60));
        let falling = 100_u32.saturating_sub(scale(span, offset, 60));
        let (red, green, blue) = match hue / 60 {
            0 => (100, rising, low),
            1 => (falling, 100, low),
            2 => (low, 100, rising),
            3 => (low, falling, 100),
            4 => (rising, low, 100),
            _ => (100, low, falling),
        };
        let brightness = u32::from(brightness.0);
        let level = |part: u32| Level::capped(scale(part, brightness, 100));
        ColorLevels {
            red: level(red),
            green: level(green),
            blue: level(blue),
        }
    }

    /// `None` when the levels hold no light.
    #[must_use]
    pub(crate) fn from_levels(levels: ColorLevels) -> Option<WithBrightness<Self>> {
        let [red, green, blue] = levels.bytes().map(i32::from);
        let max = red.max(green).max(blue);
        let min = red.min(green).min(blue);
        let span = max.checked_sub(min)?;
        let sixths = |from: i32, to: i32, base: i32| {
            let part = to.checked_sub(from)?.checked_mul(60)?;
            base.checked_add(rounded_div(part, span)?)
        };
        let hue = match (span, max) {
            (0, _) => Some(0),
            (_, top) if top == red => sixths(blue, green, 0),
            (_, top) if top == green => sixths(red, blue, 120),
            _ => sixths(green, red, 240),
        }?
        .rem_euclid(360);
        let saturation = rounded_div(span.checked_mul(100)?, max)?;
        Some(WithBrightness {
            setting: Self {
                hue: Hue::try_from(u16::try_from(hue).ok()?).ok()?,
                saturation: Percent::try_from(u8::try_from(saturation).ok()?).ok()?,
            },
            brightness: Brightness::try_from(u8::try_from(max).ok()?).ok()?,
        })
    }
}

/// `value * times / per`, rounded; 0 when `per` is 0.
fn scale(value: u32, times: u32, per: u32) -> u32 {
    let half = per.checked_div(2).unwrap_or(0);
    value
        .saturating_mul(times)
        .saturating_add(half)
        .checked_div(per)
        .unwrap_or(0)
}

/// `value / per` for a positive `per`, rounded half away from zero.
fn rounded_div(value: i32, per: i32) -> Option<i32> {
    let half = per.checked_div(2)?;
    let nudged = match value.signum() {
        -1 => value.checked_sub(half)?,
        _ => value.checked_add(half)?,
    };
    nudged.checked_div(per)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub(crate) struct Settings {
    pub(crate) power: Power,
    pub(crate) mode: Mode,
    /// One brightness for the white and the colour light.
    pub(crate) brightness: Brightness,
    pub(crate) white: WhiteSetting,
    pub(crate) color: ColorSetting,
}

impl Settings {
    #[must_use]
    pub(crate) fn light(self) -> Light {
        match (self.power, self.mode) {
            (Power::Off, Mode::White | Mode::Color) => Light::Off,
            (Power::On, Mode::White) => Light::White(self.white.levels(self.brightness)),
            (Power::On, Mode::Color) => Light::Color(self.color.levels(self.brightness)),
        }
    }

    /// The white light, switched on.
    pub(crate) const fn white_on(&mut self, white: WhiteSetting, brightness: Brightness) {
        self.white = white;
        self.brightness = brightness;
        self.mode = Mode::White;
        self.power = Power::On;
    }

    /// The colour light, switched on.
    pub(crate) const fn color_on(&mut self, color: ColorSetting, brightness: Brightness) {
        self.color = color;
        self.brightness = brightness;
        self.mode = Mode::Color;
        self.power = Power::On;
    }

    /// Follows what the lamp shows; `last` is the light it switches on
    /// next while it is dark. The one brightness is that light's. Levels
    /// without light leave their setting, and the brightness, as they
    /// were: the panel keeps showing the light as it last shone.
    pub(crate) fn follow(&mut self, lamp: Lamp, last: Channel) {
        let (power, channel) = match lamp.output {
            Output::Dark => (Power::Off, last),
            Output::Shining(channel) => (Power::On, channel),
        };
        let white = WhiteSetting::from_levels(lamp.white);
        let color = ColorSetting::from_levels(lamp.color);
        let brightness = match channel {
            Channel::White => white.map(|white| white.brightness),
            Channel::Color => color.map(|color| color.brightness),
        };
        self.power = power;
        self.mode = Mode::from(channel);
        if let Some(white) = white {
            self.white = white.setting;
        }
        if let Some(color) = color {
            self.color = color.setting;
        }
        if let Some(brightness) = brightness {
            self.brightness = brightness;
        }
    }
}

impl From<Channel> for Mode {
    fn from(channel: Channel) -> Self {
        match channel {
            Channel::White => Self::White,
            Channel::Color => Self::Color,
        }
    }
}

impl From<Mode> for Channel {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::White => Self::White,
            Mode::Color => Self::Color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brightness(value: u8) -> Brightness {
        Brightness::try_from(value).unwrap()
    }

    fn white(tone: u8, light: u8) -> WithBrightness<WhiteSetting> {
        let setting = WhiteSetting {
            tone: Percent::try_from(tone).unwrap(),
        };
        WithBrightness {
            setting,
            brightness: brightness(light),
        }
    }

    fn color(hue: u16, saturation: u8, light: u8) -> WithBrightness<ColorSetting> {
        let setting = ColorSetting {
            hue: Hue::try_from(hue).unwrap(),
            saturation: Percent::try_from(saturation).unwrap(),
        };
        WithBrightness {
            setting,
            brightness: brightness(light),
        }
    }

    fn white_levels(white: WithBrightness<WhiteSetting>) -> WhiteLevels {
        white.setting.levels(white.brightness)
    }

    fn color_levels(color: WithBrightness<ColorSetting>) -> ColorLevels {
        color.setting.levels(color.brightness)
    }

    fn levels(warm: u8, cold: u8) -> WhiteLevels {
        WhiteLevels {
            warm: Level::new(warm).unwrap(),
            cold: Level::new(cold).unwrap(),
        }
    }

    fn rgb(red: u8, green: u8, blue: u8) -> ColorLevels {
        ColorLevels {
            red: Level::new(red).unwrap(),
            green: Level::new(green).unwrap(),
            blue: Level::new(blue).unwrap(),
        }
    }

    #[test]
    fn rejects_numbers_out_of_range() {
        assert!(Percent::try_from(101).is_err(), "101 is no percentage");
        assert!(Brightness::try_from(0).is_err(), "0 is off, not dim");
        assert!(Hue::try_from(360).is_err(), "360 degrees is 0 degrees");
        assert!(
            serde_json::from_str::<Brightness>("0").is_err(),
            "the panel cannot send a brightness of 0"
        );
    }

    #[test]
    fn white_tone_mixes_the_two_groups() {
        assert_eq!(white_levels(white(0, 50)), levels(50, 0));
        assert_eq!(white_levels(white(100, 80)), levels(0, 80));
        assert_eq!(white_levels(white(50, 60)), levels(60, 60));
        assert_eq!(white_levels(white(25, 100)), levels(100, 50));
        assert_eq!(white_levels(white(75, 100)), levels(50, 100));
    }

    #[test]
    fn white_levels_lead_back_to_the_setting() {
        for setting in [white(0, 50), white(30, 80), white(50, 100), white(90, 40)] {
            assert_eq!(
                WhiteSetting::from_levels(white_levels(setting)),
                Some(setting),
                "{setting:?}"
            );
        }
        assert_eq!(WhiteSetting::from_levels(levels(0, 0)), None);
    }

    #[test]
    fn colors_become_levels() {
        assert_eq!(color_levels(color(0, 100, 100)), rgb(100, 0, 0));
        assert_eq!(color_levels(color(120, 100, 100)), rgb(0, 100, 0));
        assert_eq!(color_levels(color(240, 100, 50)), rgb(0, 0, 50));
        assert_eq!(color_levels(color(60, 100, 100)), rgb(100, 100, 0));
        assert_eq!(color_levels(color(30, 100, 100)), rgb(100, 50, 0));
        assert_eq!(color_levels(color(300, 50, 100)), rgb(100, 50, 100));
        assert_eq!(color_levels(color(200, 0, 40)), rgb(40, 40, 40));
    }

    #[test]
    fn color_levels_lead_back_to_the_setting() {
        for setting in [
            color(0, 100, 100),
            color(30, 100, 100),
            color(200, 80, 60),
            color(330, 40, 100),
        ] {
            assert_eq!(
                ColorSetting::from_levels(color_levels(setting)),
                Some(setting),
                "{setting:?}"
            );
        }
        assert_eq!(ColorSetting::from_levels(rgb(0, 0, 0)), None);
        // Grey has no hue.
        assert_eq!(
            ColorSetting::from_levels(rgb(30, 30, 30)),
            Some(color(0, 0, 30))
        );
    }

    fn followed(lamp: Lamp, last: Channel) -> Settings {
        let mut settings = Settings::default();
        settings.follow(lamp, last);
        settings
    }

    #[test]
    fn settings_read_from_the_lamp() {
        let lamp = Lamp {
            output: Output::Dark,
            white: levels(50, 0),
            color: rgb(0, 0, 30),
        };
        // `0x20` was the last command: the colour light was switched off,
        // so its brightness is the one brightness.
        let settings = followed(lamp, Channel::Color);
        assert_eq!(settings.power, Power::Off);
        assert_eq!(settings.mode, Mode::Color);
        assert_eq!(settings.white, white(0, 50).setting);
        assert_eq!(settings.color, color(240, 100, 30).setting);
        assert_eq!(settings.brightness, brightness(30));
        let shining = Lamp {
            output: Output::Shining(Channel::White),
            ..lamp
        };
        let on = followed(shining, Channel::White);
        assert_eq!((on.power, on.mode), (Power::On, Mode::White));
        assert_eq!(on.brightness, brightness(50));
    }

    #[test]
    fn only_the_shining_light_sets_the_brightness() {
        let mut settings = followed(
            Lamp {
                output: Output::Shining(Channel::Color),
                white: levels(100, 0),
                color: rgb(30, 0, 0),
            },
            Channel::Color,
        );
        assert_eq!(settings.brightness, brightness(30));
        // The button switches to white: its own brightness counts now.
        settings.follow(
            Lamp {
                output: Output::Shining(Channel::White),
                white: levels(100, 0),
                color: rgb(30, 0, 0),
            },
            Channel::White,
        );
        assert_eq!(
            (settings.mode, settings.brightness),
            (Mode::White, brightness(100))
        );
        // A light without levels keeps what the panel showed.
        settings.follow(
            Lamp {
                output: Output::Shining(Channel::White),
                white: levels(0, 0),
                color: rgb(30, 0, 0),
            },
            Channel::White,
        );
        assert_eq!(settings.brightness, brightness(100));
        assert_eq!(settings.white, white(0, 100).setting);
    }

    #[test]
    fn one_brightness_for_both_lights() {
        let settings = Settings {
            power: Power::On,
            mode: Mode::Color,
            brightness: brightness(40),
            white: white(0, 1).setting,
            color: color(120, 100, 1).setting,
        };
        assert_eq!(settings.light(), Light::Color(rgb(0, 40, 0)));
        let white_light = Settings {
            mode: Mode::White,
            ..settings
        };
        assert_eq!(white_light.light(), Light::White(levels(40, 0)));
        let off = Settings {
            power: Power::Off,
            ..settings
        };
        assert_eq!(off.light(), Light::Off);
        assert_eq!(Settings::default().brightness, brightness(50));
    }
}
