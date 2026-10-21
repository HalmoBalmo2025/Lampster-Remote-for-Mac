//! Messages to and from the panel (`helpers/ui/`): Lampster Remote sends
//! the whole [`View`] whenever it changes, the panel a [`Command`] for every
//! change the user makes.

use serde::{Deserialize, Serialize};

use crate::lamp::Temperature;
use crate::preferences::{Options, Switch};
use crate::setting::{Brightness, ColorSetting, Mode, Power, Settings, WhiteSetting};
use crate::timer::{ClockTime, Minutes, TimerView};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    Power {
        power: Power,
    },
    /// A lamp that is off stays off.
    Mode {
        mode: Mode,
    },
    /// Switches the white light on.
    White {
        white: WhiteSetting,
        brightness: Brightness,
    },
    /// Switches the colour light on.
    Color {
        color: ColorSetting,
        brightness: Brightness,
    },
    /// Switches the lamp off and ends Lampster Remote.
    Quit,
    /// The Mac shuts down, restarts or logs out.
    Shutdown,
    /// The Mac locks its screen or goes to sleep.
    Away,
    OpenAtLogin {
        open_at_login: Switch,
    },
    /// Switch the lamp on at the next `at`, also without the Mac.
    Wake {
        at: ClockTime,
    },
    WakeOff,
    /// Switch the lamp off in `minutes`, also without the Mac.
    Sleep {
        minutes: Minutes,
    },
    SleepOff,
    OffWithMac {
        off_with_mac: Switch,
    },
    /// A problem of the panel for the log.
    Log {
        message: String,
    },
}

/// Who a command comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sender {
    /// The user, in the panel.
    User,
    /// The panel by itself: the Mac locking, sleeping, shutting down,
    /// restarting or logging out, or a problem of the panel for the log.
    Panel,
}

impl Command {
    #[must_use]
    pub(crate) const fn sender(&self) -> Sender {
        match self {
            Self::Log { message: _ } | Self::Away | Self::Shutdown => Sender::Panel,
            Self::Power { power: _ }
            | Self::Mode { mode: _ }
            | Self::White {
                white: _,
                brightness: _,
            }
            | Self::Color {
                color: _,
                brightness: _,
            }
            | Self::Quit
            | Self::OpenAtLogin { open_at_login: _ }
            | Self::Wake { at: _ }
            | Self::WakeOff
            | Self::Sleep { minutes: _ }
            | Self::SleepOff
            | Self::OffWithMac { off_with_mac: _ } => Sender::User,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Connection {
    Searching,
    /// Found; reading what the lamp does.
    Connecting,
    Connected,
    BluetoothOff,
    Unauthorized,
    Unsupported,
}

/// What the lamp really shows; dark without a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Lit {
    Dark,
    White,
    Color,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct View {
    pub(crate) connection: Connection,
    pub(crate) lit: Lit,
    /// Hundredths of a degree Celsius; `None` without a connection.
    pub(crate) temperature: Option<Temperature>,
    pub(crate) settings: Settings,
    pub(crate) options: Options,
    pub(crate) timers: TimersView,
    pub(crate) problem: Option<String>,
    /// How many lines from the panel the core has read; the panel keeps
    /// showing its own changes until a view has read all it sent.
    pub(crate) commands: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub(crate) struct TimersView {
    pub(crate) wake: Option<TimerView>,
    pub(crate) sleep: Option<TimerView>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setting::{Brightness, Hue, Percent};

    use crate::lamp::Temperature;
    use crate::timer::{ClockTime, TimerView};

    /// The fixture's lines; the Swift tests read the same file.
    fn lines(fixture: &str) -> Vec<&str> {
        fixture.lines().collect()
    }

    #[test]
    fn commands_are_read_as_the_fixture_has_them() {
        let read: Vec<Command> = lines(include_str!("../fixtures/ui/commands.jsonl"))
            .into_iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let at: ClockTime = serde_json::from_str(r#"{"hour":7,"minute":30}"#).unwrap();
        assert_eq!(
            read,
            [
                Command::Power { power: Power::On },
                Command::Mode { mode: Mode::Color },
                Command::White {
                    white: WhiteSetting {
                        tone: Percent::try_from(30).unwrap()
                    },
                    brightness: Brightness::try_from(80).unwrap()
                },
                Command::Color {
                    color: ColorSetting {
                        hue: Hue::try_from(200).unwrap(),
                        saturation: Percent::try_from(50).unwrap()
                    },
                    brightness: Brightness::try_from(100).unwrap()
                },
                Command::Quit,
                Command::Shutdown,
                Command::Away,
                Command::OpenAtLogin {
                    open_at_login: Switch::On
                },
                Command::OffWithMac {
                    off_with_mac: Switch::On
                },
                Command::Wake { at },
                Command::WakeOff,
                Command::Sleep {
                    minutes: serde_json::from_str("30").unwrap()
                },
                Command::SleepOff,
                Command::Log {
                    message: "unreadable view (3 characters): not JSON".to_owned()
                },
            ]
        );
    }

    #[test]
    fn views_are_written_as_the_fixture_has_them() {
        let at = |second| jiff::Timestamp::from_second(second).unwrap().into();
        let start = View {
            connection: Connection::BluetoothOff,
            lit: Lit::Dark,
            temperature: None,
            settings: Settings::default(),
            options: Options::default(),
            timers: TimersView::default(),
            problem: None,
            commands: 0,
        };
        let shining = View {
            connection: Connection::Connected,
            lit: Lit::Color,
            temperature: Temperature::from_bytes(&4120_i16.to_le_bytes()),
            settings: Settings {
                power: Power::On,
                mode: Mode::Color,
                brightness: Brightness::try_from(70).unwrap(),
                white: WhiteSetting {
                    tone: Percent::try_from(30).unwrap(),
                },
                color: ColorSetting {
                    hue: Hue::try_from(200).unwrap(),
                    saturation: Percent::try_from(80).unwrap(),
                },
            },
            options: Options {
                open_at_login: Switch::On,
                off_with_mac: Switch::Off,
            },
            timers: TimersView {
                wake: Some(TimerView::InLamp {
                    at: at(1_792_472_400),
                }),
                sleep: Some(TimerView::Pending {
                    at: at(1_792_440_000),
                }),
            },
            problem: Some("Lampster got warm; brightness lowered to 50 %.".to_owned()),
            commands: 12,
        };
        let cancelling = View {
            connection: Connection::Searching,
            timers: TimersView {
                wake: Some(TimerView::Cancelling),
                sleep: None,
            },
            ..start.clone()
        };
        let written: Vec<String> = [start, shining, cancelling]
            .iter()
            .map(|view| serde_json::to_string(view).unwrap())
            .collect();
        assert_eq!(written, lines(include_str!("../fixtures/ui/views.jsonl")));
    }

    /// Each line differs from a valid one only in the value named.
    #[test]
    fn numbers_out_of_range_are_refused_at_the_panels_edge() {
        let color = |hue: u16, saturation: u8| {
            format!(
                r#"{{"command":"color","color":{{"hue":{hue},"saturation":{saturation}}},"brightness":100}}"#
            )
        };
        let white = |tone: u8, brightness: u8| {
            format!(r#"{{"command":"white","white":{{"tone":{tone}}},"brightness":{brightness}}}"#)
        };
        let wake = |hour: u8, minute: u8| {
            format!(r#"{{"command":"wake","at":{{"hour":{hour},"minute":{minute}}}}}"#)
        };
        let sleep = |minutes: u16| format!(r#"{{"command":"sleep","minutes":{minutes}}}"#);
        let read = |line: String| serde_json::from_str::<Command>(&line).is_ok();
        for (valid, invalid, what) in [
            (color(359, 100), color(360, 100), "hue"),
            (color(0, 100), color(0, 101), "saturation"),
            (white(100, 1), white(101, 1), "tone"),
            (white(0, 100), white(0, 0), "brightness"),
            (white(0, 100), white(0, 101), "brightness above"),
            (wake(23, 59), wake(24, 0), "hour"),
            (wake(23, 59), wake(23, 60), "minute"),
            (sleep(120), sleep(121), "sleep time"),
            (sleep(1), sleep(0), "sleep time below"),
        ] {
            assert!(read(valid), "the largest {what}");
            assert!(!read(invalid), "one {what} too many");
        }
    }
}
