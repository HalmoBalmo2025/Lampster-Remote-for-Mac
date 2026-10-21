//! What the panel shows, sent only when it changes.

use crate::ble::Radio;
use crate::lamp::{Channel, Output};
use crate::ui::{Connection, Lit, TimersView, View};

use super::link::{GIVE_UPS_TOLD, Link, Phase};
use super::{Controller, Effects, LogFile, Problem};

impl Controller {
    fn view(&self) -> View {
        let connection = match self.radio {
            Radio::Off => Connection::BluetoothOff,
            Radio::Unauthorized => Connection::Unauthorized,
            Radio::Unsupported => Connection::Unsupported,
            Radio::On | Radio::Unknown => match &self.link {
                Link::Searching { wish: _, phase } => match phase {
                    Phase::Waiting { until: _ }
                    | Phase::Connecting {
                        request: _,
                        since: _,
                        restarts: _,
                    } => Connection::Searching,
                    Phase::Reading {
                        request: _,
                        since: _,
                        progress: _,
                    } => Connection::Connecting,
                },
                Link::Ready(_) => Connection::Connected,
            },
        };
        let session = self.link.session();
        let lit = session.map_or(Lit::Dark, |session| match session.lamp.output {
            Output::Dark => Lit::Dark,
            Output::Shining(Channel::White) => Lit::White,
            Output::Shining(Channel::Color) => Lit::Color,
        });
        View {
            connection,
            lit,
            temperature: session.and_then(|session| session.temperature),
            settings: self.settings,
            options: self.options,
            timers: TimersView {
                wake: self.timers.wake.view(),
                sleep: self.timers.sleep.view(),
            },
            problem: self.problems(),
            commands: self.commands,
        }
    }

    /// Every sentence that holds, in the order of [`Problem`]; `None` when
    /// all is well.
    fn problems(&self) -> Option<String> {
        let refused = self
            .link
            .session()
            .is_some_and(|session| session.mode.gave_up() || session.timers.gave_up());
        // In no particular order: the sort below gives it.
        let mut holding: Vec<Problem> = [
            match self.log {
                LogFile::Written => None,
                LogFile::Lost => Some(Problem::LogLost),
            },
            (self.give_ups >= GIVE_UPS_TOLD).then_some(Problem::Unstable),
            refused.then_some(Problem::Refused),
            self.heat.map(Problem::Heat),
            self.reply.map(Problem::Reply),
        ]
        .into_iter()
        .flatten()
        .collect();
        holding.sort_unstable();
        let shown: Vec<String> = holding.iter().map(ToString::to_string).collect();
        (!shown.is_empty()).then(|| shown.join(" "))
    }

    pub(super) fn show(&mut self, effects: &mut impl Effects) {
        let view = self.view();
        if self.shown.as_ref() != Some(&view) {
            effects.show(&view);
            self.shown = Some(view);
        }
    }
}
