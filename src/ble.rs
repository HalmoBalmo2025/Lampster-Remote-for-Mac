//! Messages to and from the Bluetooth helper (`helpers/ble/`). The
//! helper knows nothing about the lamp; it is told what to look for, read
//! and write. Every answer names its [`RequestId`], so answers to requests
//! that no longer matter can be dropped.

use serde::{Deserialize, Serialize};

use crate::lamp::plan::{Delivery, Step};
use crate::lamp::{ADVERTISED_SERVICE, Characteristic, LIGHT_SERVICE};

/// Counts up: a later request has a larger id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RequestId(u64);

impl RequestId {
    pub(crate) const FIRST: Self = Self(1);

    /// This id, and the next one in its place.
    pub(crate) const fn take(&mut self) -> Self {
        let id = *self;
        *self = id.next();
        id
    }

    #[must_use]
    pub(crate) const fn next(self) -> Self {
        // Ids do not reach the end of a u64.
        Self(self.0.saturating_add(1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub(crate) enum Request {
    /// Search for a device that advertises `advertised` until one shows up
    /// (also while Bluetooth is off), connect, discover `services` and
    /// subscribe to the changes of `subscribe`. Replaces a connect under
    /// way.
    Connect {
        id: RequestId,
        advertised: &'static str,
        services: [&'static str; 1],
        subscribe: [&'static str; Characteristic::ALL.len()],
    },
    Read {
        id: RequestId,
        characteristic: &'static str,
    },
    Write {
        id: RequestId,
        characteristic: &'static str,
        value: Vec<u8>,
        delivery: Delivery,
    },
    /// Drop the connection or the search; no answer.
    Disconnect,
}

impl Request {
    #[must_use]
    pub(crate) fn connect(id: RequestId) -> Self {
        Self::Connect {
            id,
            advertised: ADVERTISED_SERVICE,
            services: [LIGHT_SERVICE],
            subscribe: Characteristic::ALL.map(Characteristic::uuid),
        }
    }

    #[must_use]
    pub(crate) const fn read(id: RequestId, characteristic: Characteristic) -> Self {
        Self::Read {
            id,
            characteristic: characteristic.uuid(),
        }
    }

    #[must_use]
    pub(crate) fn write(id: RequestId, step: Step) -> Self {
        Self::Write {
            id,
            characteristic: step.characteristic().uuid(),
            value: step.bytes(),
            delivery: step.delivery(),
        }
    }
}

/// The state of the Mac's Bluetooth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Radio {
    On,
    Off,
    /// No access in System Settings → Privacy & Security → Bluetooth.
    Unauthorized,
    Unsupported,
    /// Starting up or resetting.
    Unknown,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Event {
    Bluetooth {
        state: Radio,
    },
    /// The characteristics can be used.
    Connected {
        id: RequestId,
    },
    Value {
        id: RequestId,
        value: Vec<u8>,
    },
    /// The device notified a new value: its own change, or the echo of a
    /// write.
    Changed {
        characteristic: Characteristic,
        value: Vec<u8>,
    },
    /// Sent, and confirmed if confirmation was asked for.
    Written {
        id: RequestId,
    },
    /// The device refused a confirmed write ("Writing is not permitted").
    Rejected {
        id: RequestId,
    },
    Failed {
        /// `None` when the failure answers no request: one the helper could
        /// not read, or a change of the lamp that came with an error.
        id: Option<RequestId>,
        reason: String,
    },
    /// Changes of `characteristic` will not come: subscribing failed
    /// after the connect.
    Unsubscribed {
        characteristic: Characteristic,
        reason: String,
    },
    Disconnected {
        reason: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lamp::ModeCommand;

    use crate::lamp::{Level, WhiteLevels};

    /// The fixture's lines; the Swift tests read the same file.
    fn lines(fixture: &str) -> Vec<&str> {
        fixture.lines().collect()
    }

    #[test]
    fn requests_are_written_as_the_fixture_has_them() {
        let white = WhiteLevels {
            warm: Level::new(50).unwrap(),
            cold: Level::new(0).unwrap(),
        };
        let requests = [
            Request::connect(RequestId(1)),
            Request::read(RequestId(2), Characteristic::Mode),
            Request::write(RequestId(3), Step::Mode(ModeCommand::WhiteOn)),
            Request::write(RequestId(4), Step::White(white)),
            Request::Disconnect,
        ];
        let written: Vec<String> = requests
            .iter()
            .map(|request| serde_json::to_string(request).unwrap())
            .collect();
        assert_eq!(
            written,
            lines(include_str!("../fixtures/ble/requests.jsonl"))
        );
    }

    #[test]
    fn events_are_read_as_the_fixture_has_them() {
        let read: Vec<Event> = lines(include_str!("../fixtures/ble/events.jsonl"))
            .into_iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            read,
            [
                Event::Bluetooth { state: Radio::On },
                Event::Connected { id: RequestId(1) },
                Event::Value {
                    id: RequestId(2),
                    value: vec![0x48]
                },
                Event::Changed {
                    characteristic: Characteristic::White,
                    value: vec![10, 20]
                },
                Event::Written { id: RequestId(3) },
                Event::Rejected { id: RequestId(3) },
                Event::Failed {
                    id: Some(RequestId(4)),
                    reason: "not connected or unknown characteristic".to_owned()
                },
                Event::Failed {
                    id: None,
                    reason: "unreadable request".to_owned()
                },
                Event::Unsubscribed {
                    characteristic: Characteristic::Temperature,
                    reason: "denied".to_owned()
                },
                Event::Disconnected {
                    reason: Some("Bluetooth is off".to_owned())
                },
                Event::Disconnected { reason: None },
            ]
        );
    }

    #[test]
    fn events_that_make_no_sense_are_errors() {
        // Lower case, as CoreBluetooth may spell it.
        assert_eq!(
            serde_json::from_str::<Event>(
                r#"{"characteristic":"01ff5556-ba5e-f4ee-5ca1-eb1e5e4b1ce0","value":[10,20],"event":"changed"}"#
            )
            .unwrap(),
            Event::Changed {
                characteristic: Characteristic::White,
                value: vec![10, 20]
            }
        );
        assert!(
            serde_json::from_str::<Event>(
                r#"{"characteristic":"2A26","value":[1,2],"event":"changed"}"#
            )
            .is_err(),
            "a characteristic Lampster Remote does not know"
        );
        assert!(
            serde_json::from_str::<Event>(r#"{"event":"exploded"}"#).is_err(),
            "unknown events are errors"
        );
    }

    #[test]
    fn ids_count_up() {
        assert_eq!(RequestId::FIRST.next(), RequestId(2));
    }
}
